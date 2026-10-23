//! Layer 6 C5 acceptance (`6-transport/README.md` §7, §2.1, §4, §15.2; the netns spike
//! `plan/layers/6-transport/research/08-spike-deltachat.md` §6–§7): the Delta Chat adapter
//! against `tests/fake_deltachat_rpc.py`, a scripted JSON-RPC-over-stdio fake of
//! `deltachat-rpc-server` run as a real supervised child with piped stdio.
//!
//! - framing: out-of-order answers matched by id, the long poll always outstanding;
//! - the handshake: configs in order (`bot=1`, `mdns_enabled=1`), `create-user` once, the
//!   Secure-Join QR on the link-out card;
//! - consent: a joined contact is not bridged until the per-contact switch flips (adapter
//!   level and through a real bridge on a real fabric, with the 409 round trip);
//! - states 20 / 26 / 28 / `MsgFailed`; receive with an untrusted name carrying markup; the
//!   relay marker on an own copy as the visible prefix only;
//! - malformed, oversize and unknown-kind lines counted as noise; a respawn re-attaches;
//! - the Madmail configuration on disk; the declarations through S4's loader; the glibc,
//!   Termux and "not in this release" refusals; the whole enable with fake binaries.
//!
//! Hermetic: temp roots, loopback only; every child is stopped on the way out (and dies
//! with the test process through the parent-death signal otherwise).

use futures_util::FutureExt;
use mesh111::store::{ForeignAuth, ForeignDir, ForeignState};
use node111::daemon::bridge::deltachat::{
    self, render_madmail_config, DcConfig, DeltaChatAdapter, OptIn, RelayAccounts, RelayBind,
    RelayPaths, RelayPorts, Setup, ACCOUNT_WARNING, EVIDENCE_DELIVERED, EVIDENCE_QUEUED,
    EVIDENCE_SENT, MAX_TEXT_BYTES,
};
use node111::daemon::bridge::settings::{contact_text, sha256_hex};
use node111::daemon::bridge::{
    Adapter, AdapterState, Auth, Bridge, BridgeConfig, BridgeError, ConvKind, ConvRef,
    ForeignMessageState, Inbound, LinkState, Reason, Retry, SendEvent, SendOpts,
};
use node111::daemon::components::manifest::{self, Component, Health, Payload};
use node111::daemon::components::registry::ComponentState;
use node111::daemon::components::runner::RunnerConfig;
use node111::daemon::components::{
    Components, ComponentsConfig, Dependencies, EnableError, Wiring,
};
use node111::daemon::dashboard::{ApiConfig, ApiState, DashboardState, SharedState};
use node111::daemon::fabric::{self, FabricConfig};
use node111::daemon::ports::PortTable;
use node111::daemon::services::ServiceId;
use node111::daemon::supervise::{
    spawn_child, BackoffPolicy, ChildControl, ChildHandle, ChildSpec, ChildStdio, PipedIo,
};
use node111::daemon::DaemonLayout;
use node111::testkit::MockRunner;
use node111::util::hash::sha256_bytes;
use node111::util::run::{Output, Runner, SystemRunner};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const LIMIT: Duration = Duration::from_secs(20);
const HUB: &str = "hub@[127.0.0.1]";
const ALICE: &str = "alice@[127.0.0.1]";
const DCLOGIN: &str = "dclogin:hub@[127.0.0.1]/?p=pw&v=1&ih=127.0.0.1&ip=11242&is=ssl&sh=127.0.0.1&sp=11241&ss=ssl&ic=3";

// ---------------------------------------------------------------- the fake core

fn fake_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fake_deltachat_rpc.py")
}

fn python3() -> PathBuf {
    std::env::var_os("PATH")
        .and_then(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join("python3"))
                .find(|c| c.is_file())
        })
        .unwrap_or_else(|| PathBuf::from("/usr/bin/python3"))
}

struct Fake {
    dir: tempfile::TempDir,
    log: PathBuf,
    child: ChildHandle,
}

impl Fake {
    fn spawn() -> Fake {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("log.jsonl");
        let spec = ChildSpec {
            name: "fake-deltachat".into(),
            program: python3(),
            args: vec![
                "-I".into(),
                fake_script().to_string_lossy().into_owned(),
                "--log".into(),
                log.to_string_lossy().into_owned(),
            ],
            env: vec![(
                "DC_ACCOUNTS_PATH".into(),
                dir.path().join("accounts").to_string_lossy().into_owned(),
            )],
            backoff: BackoffPolicy {
                initial_ms: 50,
                max_ms: 100,
                jitter_pct: 0,
                ..BackoffPolicy::default()
            },
            stdout_log: Some(dir.path().join("stderr.log")),
        };
        let child = spawn_child(spec.with_stdio(ChildStdio::Piped));
        Fake { dir, log, child }
    }

    async fn io(&self) -> PipedIo {
        let mut rx = self.child.stdio();
        let got = tokio::time::timeout(LIMIT, rx.wait_for(|io| io.is_some()))
            .await
            .expect("stdio published")
            .expect("supervisor alive");
        got.clone().unwrap()
    }

    /// One control line into the fake's stdin (no `jsonrpc` key; the adapter never
    /// writes one).
    async fn control(&self, v: Value) {
        self.io().await.write_line(&v.to_string()).await.unwrap();
    }

    fn log(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    /// `(method, params)` of every request but the polls, in order.
    fn requests(&self) -> Vec<(String, Value)> {
        self.log()
            .into_iter()
            .filter_map(|v| {
                let m = v.get("method")?.as_str()?.to_string();
                (m != "get_next_event_batch").then(|| (m, v["params"].clone()))
            })
            .collect()
    }

    fn count(&self, method: &str) -> usize {
        self.requests().iter().filter(|(m, _)| m == method).count()
    }

    async fn stop(self) {
        self.child.stop_and_wait(Duration::from_secs(5)).await;
    }
}

#[derive(Default)]
struct FakeRelay {
    calls: AtomicUsize,
}

impl RelayAccounts for FakeRelay {
    fn create_user(&self) -> Result<String, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(DCLOGIN.to_string())
    }
}

#[derive(Clone, Default)]
struct Switches(Arc<Mutex<HashSet<String>>>);

impl Switches {
    fn optin(&self) -> OptIn {
        let s = self.clone();
        Arc::new(move |account: &str, contact: &str| {
            account == HUB && s.0.lock().unwrap().contains(contact)
        })
    }
    fn on(&self, contact: &str) {
        self.0.lock().unwrap().insert(contact.to_string());
    }
}

fn cfg(dir: &Path) -> DcConfig {
    DcConfig {
        call_timeout: Duration::from_secs(5),
        configure_timeout: Duration::from_secs(5),
        settle_timeout: Duration::from_secs(2),
        relay_host: Some("127.0.0.1".into()),
        joined_file: Some(dir.join("joined.json")),
        ..DcConfig::default()
    }
}

struct Rig {
    fake: Fake,
    relay: Arc<FakeRelay>,
    switches: Switches,
    adapter: Arc<DeltaChatAdapter>,
}

async fn rig() -> Rig {
    let fake = Fake::spawn();
    let relay = Arc::new(FakeRelay::default());
    let switches = Switches::default();
    let adapter = DeltaChatAdapter::attach(
        cfg(fake.dir.path()),
        fake.child.stdio(),
        relay.clone(),
        switches.optin(),
    );
    wait_live(&adapter).await;
    Rig {
        fake,
        relay,
        switches,
        adapter,
    }
}

async fn until(what: &str, f: impl Fn() -> bool) {
    let t0 = Instant::now();
    while !f() {
        assert!(t0.elapsed() < LIMIT, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_live(a: &DeltaChatAdapter) {
    until("the adapter to be live", || a.state() == AdapterState::Live).await;
}

/// Everything the adapter has queued now.
fn drain(a: &DeltaChatAdapter) -> Vec<Inbound> {
    let mut out = Vec::new();
    while let Some(i) = a.recv().now_or_never() {
        out.push(i);
    }
    out
}

/// The next inbound item that is not link state.
async fn next_item(a: &DeltaChatAdapter) -> Inbound {
    loop {
        let i = tokio::time::timeout(LIMIT, a.recv())
            .await
            .expect("an inbound item");
        if !matches!(i, Inbound::Link(_)) {
            return i;
        }
    }
}

async fn next_event(rx: &mut tokio::sync::mpsc::Receiver<SendEvent>) -> Option<SendEvent> {
    tokio::time::timeout(LIMIT, rx.recv())
        .await
        .expect("a send event or the end")
}

fn dm(addr: &str) -> ConvRef {
    ConvRef {
        network: "app:deltachat".into(),
        account: HUB.into(),
        kind: ConvKind::Dm,
        native_id: format!("dm:{addr}"),
    }
}

/// Join `addr`, switch it on, and wait until the adapter knows it.
async fn joined_and_on(r: &Rig, addr: &str, name: &str) {
    r.fake
        .control(json!({"control": "join", "addr": addr, "name": name}))
        .await;
    until("the join", || {
        r.adapter.contacts().iter().any(|c| c.address == addr)
    })
    .await;
    r.switches.on(addr);
}

// ---------------------------------------------------------------- framing and handshake

#[tokio::test]
async fn framing_matches_out_of_order_answers_and_keeps_a_poll_outstanding() {
    let r = rig().await;
    let log = r.fake.log();
    // the first requests are the long polls, answered only after later requests
    let first_poll = log
        .iter()
        .position(|v| v["method"] == "get_next_event_batch")
        .expect("a poll");
    let info_answer = log
        .iter()
        .position(|v| v["answer"] == "get_system_info")
        .expect("get_system_info answered");
    let info_request = log
        .iter()
        .position(|v| v["method"] == "get_system_info")
        .expect("get_system_info asked");
    let first_poll_id = log[first_poll]["id"].as_u64().unwrap();
    let poll_answer = log
        .iter()
        .position(|v| v["answer"] == "get_next_event_batch" && v["id"] == first_poll_id)
        .expect("the first poll answered (ConfigureProgress)");
    assert!(
        first_poll < info_request && info_answer < poll_answer,
        "asked first, answered after a later request (matched by id): {log:?}"
    );
    // at every point after the first poll, at least one poll is outstanding
    let mut outstanding = 0i64;
    for v in &log[first_poll..] {
        if v["method"] == "get_next_event_batch" {
            outstanding += 1;
        } else if v["answer"] == "get_next_event_batch" {
            outstanding -= 1;
            assert!(outstanding >= 1, "a poll stays outstanding: {log:?}");
        }
    }
    // a delayed answer arrives after the answers to later requests and still lands
    r.fake
        .control(json!({"control": "delay", "method": "get_message", "secs": 0.3}))
        .await;
    joined_and_on(&r, ALICE, "Alice").await;
    r.fake
        .control(json!({"control": "incoming", "from": ALICE, "text": "slow"}))
        .await;
    let item = next_item(&r.adapter).await;
    assert!(matches!(item, Inbound::Conversation { .. }), "{item:?}");
    let Inbound::Text(t) = next_item(&r.adapter).await else {
        panic!("a text")
    };
    assert_eq!(t.body, "slow");
    assert_eq!(r.adapter.noise(), 0, "nothing malformed so far");
    r.fake.stop().await;
}

#[tokio::test]
async fn the_enable_sequence_sets_the_bot_flag_and_mdns_in_order() {
    let r = rig().await;
    let reqs = r.fake.requests();
    let methods: Vec<String> = reqs
        .iter()
        .map(|(m, p)| match m.as_str() {
            "set_config" | "get_config" => format!("{m} {}", p[1].as_str().unwrap_or("?")),
            _ => m.clone(),
        })
        .collect();
    assert_eq!(
        methods,
        [
            "get_system_info",
            "get_all_account_ids",
            "add_account",
            "set_config bot",
            "set_config mdns_enabled",
            "set_config displayname",
            "set_config delete_device_after",
            "is_configured",
            "add_transport_from_qr",
            "get_config configured_addr",
            "start_io",
            "wait_for_all_work_done",
            "get_chat_securejoin_qr_code",
        ],
        "{reqs:?}"
    );
    let value = |key: &str| {
        reqs.iter()
            .find(|(m, p)| m == "set_config" && p[1] == key)
            .map(|(_, p)| p[2].clone())
            .unwrap()
    };
    assert_eq!(value("bot"), "1");
    assert_eq!(
        value("mdns_enabled"),
        "1",
        "or the bot neither sends nor gets receipts"
    );
    assert_eq!(value("displayname"), "111 bridge");
    assert_eq!(
        value("delete_device_after"),
        "604800",
        "7 days, account-wide"
    );
    let qr = reqs
        .iter()
        .find(|(m, _)| m == "add_transport_from_qr")
        .unwrap();
    assert_eq!(qr.1[1], DCLOGIN, "the relay's dclogin:");
    assert_eq!(r.relay.calls.load(Ordering::SeqCst), 1, "create-user once");
    // the account is the one fact the core reads: the bot address, ids globally unique
    let f = r.adapter.facts();
    assert_eq!(f.accounts.len(), 1);
    assert_eq!(f.accounts[0].id, HUB);
    assert!(!f.accounts[0].licensed);
    assert!(f.ids_globally_unique);
    assert_eq!(r.adapter.account().as_deref(), Some(HUB));
    assert_eq!(r.adapter.core_version().as_deref(), Some("v2.63.0"));
    let caps = r.adapter.caps();
    assert_eq!(caps.kinds, [ConvKind::Dm, ConvKind::Group]);
    assert_eq!(
        caps.evidence,
        [
            ForeignMessageState::Queued,
            ForeignMessageState::Sent,
            ForeignMessageState::Delivered
        ],
        "the core has no separate hand-off: relayed never occurs"
    );
    assert_eq!(caps.auth_levels, [Auth::None]);
    assert!(matches!(
        drain(&r.adapter).as_slice(),
        [Inbound::Link(LinkState::Up)]
    ));
    r.fake.stop().await;
}

#[tokio::test]
async fn the_secure_join_qr_is_on_the_link_out_card() {
    let r = rig().await;
    let card = r.adapter.card().expect("a card once live");
    assert_eq!(card.title, "Open Delta Chat on your phone or desktop");
    assert_eq!(
        card.note,
        "this is the native app; Messages shows the same conversations as text"
    );
    assert_eq!(card.warning, ACCOUNT_WARNING);
    assert_eq!(card.account, HUB);
    assert!(
        card.qr_text.starts_with("https://i.delta.chat/#"),
        "{}",
        card.qr_text
    );
    assert!(
        card.qr_text.contains("a=hub%40%5B127.0.0.1%5D"),
        "{}",
        card.qr_text
    );
    assert!(card.qr.is_object(), "a QrMatrix the UI draws itself");
    let v = serde_json::to_value(&card).unwrap();
    assert_eq!(v["qr_text"], card.qr_text);
    r.fake.stop().await;
}

// ---------------------------------------------------------------- consent

#[tokio::test]
async fn a_joined_contact_is_not_bridged_until_its_switch_flips() {
    let r = rig().await;
    let _ = drain(&r.adapter);
    r.fake
        .control(json!({"control": "join", "addr": ALICE, "name": "Alice"}))
        .await;
    until("the join", || !r.adapter.contacts().is_empty()).await;
    let c = &r.adapter.contacts()[0];
    assert_eq!((c.address.as_str(), c.bridged), (ALICE, false));
    // joined, not switched on: received by the core, never copied
    r.fake
        .control(json!({"control": "incoming", "from": ALICE, "text": "before"}))
        .await;
    until("one unbridged text", || r.adapter.unbridged() == 1).await;
    // switched on but never joined by Secure-Join: still never copied
    r.switches.on("mallory@[127.0.0.1]");
    r.fake
        .control(json!({"control": "incoming", "from": "mallory@[127.0.0.1]", "text": "stranger"}))
        .await;
    until("two unbridged texts", || r.adapter.unbridged() == 2).await;
    assert!(
        drain(&r.adapter).is_empty(),
        "no InboundText before the switch"
    );
    // the human turns it on: texts after it are copied (history is not)
    r.switches.on(ALICE);
    assert!(r.adapter.contacts()[0].bridged);
    r.fake
        .control(json!({"control": "incoming", "from": ALICE, "text": "after"}))
        .await;
    let Inbound::Conversation { conv, title } = next_item(&r.adapter).await else {
        panic!("the conversation first")
    };
    assert_eq!(conv, dm(ALICE));
    assert_eq!(title, "Alice");
    let Inbound::Text(t) = next_item(&r.adapter).await else {
        panic!("then the text")
    };
    assert_eq!(t.body, "after");
    assert_eq!(t.sender_native, ALICE);
    assert_eq!(t.auth, Auth::None);
    // the joined contact survives a restart of the adapter (joined.json, 0600)
    let file = r.fake.dir.path().join("joined.json");
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    r.fake.stop().await;
}

struct Node {
    bridge: Arc<Bridge>,
    handle: Arc<node111::daemon::fabric::FabricHandle>,
    _api: Arc<ApiState>,
    _dir: tempfile::TempDir,
}

async fn node() -> Node {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let identity = node111::identity::generate(&layout.identity_dir()).unwrap();
    let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.10.19")));
    dash.write().unwrap().node_id = Some(identity.node_id_hex());
    let mut acfg = ApiConfig::new(layout.clone(), false);
    acfg.persist_auth = false;
    let api = ApiState::new(dash, acfg);
    let mut c = FabricConfig::production(&layout, None);
    c.listen = "127.0.0.1:0".parse().unwrap();
    c.housekeeping = Duration::from_secs(3600);
    let handle = fabric::start(c, Some(identity), &api).await;
    assert!(handle.running(), "{:?}", handle.down_reason());
    let bridge = Bridge::new(BridgeConfig::new(&layout.root), Arc::clone(&handle), &api);
    Node {
        bridge,
        handle,
        _api: api,
        _dir: dir,
    }
}

#[tokio::test]
async fn through_a_real_bridge_the_switch_gates_ingest_and_states_land_on_the_rows() {
    let n = node().await;
    let fake = Fake::spawn();
    let adapter = DeltaChatAdapter::attach(
        cfg(fake.dir.path()),
        fake.child.stdio(),
        Arc::new(FakeRelay::default()),
        deltachat::opt_in_from(&n.bridge),
    );
    wait_live(&adapter).await;
    n.bridge.enable(adapter.clone()).await.unwrap();
    let fab = n.handle.fabric().unwrap().clone();
    let convs = || {
        let fab = fab.clone();
        async move {
            fab.foreign_conversations(Some("app:deltachat"))
                .await
                .unwrap()
        }
    };
    let name = "<b>Mallory</b>\u{202e} <script>x</script>";
    fake.control(json!({"control": "join", "addr": ALICE, "name": name}))
        .await;
    until("the join", || !adapter.contacts().is_empty()).await;
    fake.control(json!({"control": "incoming", "from": ALICE, "text": "before"}))
        .await;
    until("one unbridged text", || adapter.unbridged() == 1).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        convs().await.is_empty(),
        "nothing reaches Messages before the switch"
    );

    // the per-contact switch: 409 with the exact text, then the hash
    let text = contact_text("Delta Chat", ALICE);
    let c = match n
        .bridge
        .set_contact_bridging("app:deltachat", HUB, ALICE, true, None, "loopback")
        .await
    {
        Err(BridgeError::Consent(c)) => c,
        other => panic!("consent first: {other:?}"),
    };
    assert_eq!(c.warning_text, text);
    assert_eq!(c.hash, sha256_hex(&text));
    assert!(text.contains(
        "This hub decrypts this contact's messages to bridge them. What you relay onto another network is readable there."
    ));
    n.bridge
        .set_contact_bridging("app:deltachat", HUB, ALICE, true, Some(&c.hash), "loopback")
        .await
        .unwrap();
    assert!(n.bridge.contact_bridged("app:deltachat", HUB, ALICE));
    assert!(adapter.contacts()[0].bridged);
    fake.control(json!({"control": "incoming", "from": ALICE, "text": "[Bo/LX] hi from the mesh"}))
        .await;
    let t0 = Instant::now();
    let conv = loop {
        let list = convs().await;
        if let Some(c) = list
            .into_iter()
            .find(|c| c.native_id == format!("dm:{ALICE}"))
        {
            let rows = fab.foreign_thread(c.id, None, 100).await.unwrap().messages;
            if !rows.is_empty() {
                break c;
            }
        }
        assert!(t0.elapsed() < LIMIT, "the text never reached the store");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(conv.account, HUB);
    let rows = fab
        .foreign_thread(conv.id, None, 100)
        .await
        .unwrap()
        .messages;
    assert_eq!(rows.len(), 1, "history before the switch is not copied");
    let row = &rows[0];
    assert_eq!(row.dir, ForeignDir::In);
    assert_eq!(
        row.body, "[Bo/LX] hi from the mesh",
        "the marker passes as text"
    );
    assert_eq!(row.auth, ForeignAuth::None);
    let stored = row.sender_name.clone().unwrap();
    assert!(
        !stored.contains('\u{202e}'),
        "Cf stripped by the one ingest: {stored:?}"
    );
    assert!(
        stored.starts_with("<b>Mallory</b>"),
        "markup kept as text: {stored:?}"
    );

    // a send through the core: queued → sent → delivered, the marker as the visible prefix
    n.bridge
        .send_text("app:deltachat", conv.id, "[Al/M] relayed text", None)
        .await
        .unwrap();
    let own = || {
        let fab = fab.clone();
        async move {
            fab.foreign_thread(conv.id, None, 100)
                .await
                .unwrap()
                .messages
                .into_iter()
                .find(|m| m.dir == ForeignDir::Own)
        }
    };
    let t0 = Instant::now();
    while fake.count("misc_send_text_message") == 0 {
        n.bridge.tick_once().await;
        assert!(t0.elapsed() < LIMIT, "never handed to the core");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let sent = fake
        .requests()
        .into_iter()
        .find(|(m, _)| m == "misc_send_text_message")
        .unwrap();
    assert_eq!(
        sent.1.as_array().unwrap().len(),
        3,
        "account, chat, text: no field"
    );
    assert_eq!(sent.1[2], "[Al/M] relayed text");
    for (control, state, evidence) in [
        ("deliver", ForeignState::Sent, EVIDENCE_SENT),
        ("read", ForeignState::Delivered, EVIDENCE_DELIVERED),
    ] {
        fake.control(json!({"control": control})).await;
        let t0 = Instant::now();
        loop {
            n.bridge.tick_once().await;
            let row = own().await.unwrap();
            if row.state == state {
                assert_eq!(row.evidence.as_deref(), Some(evidence));
                break;
            }
            assert!(t0.elapsed() < LIMIT, "{control}: {row:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    let v = n.bridge.networks().await.unwrap();
    let net = v["networks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "app:deltachat")
        .unwrap();
    assert_eq!(net["state"], "live");
    assert_eq!(
        net["caps"]["max_text_bytes"]["dm"]["default"],
        MAX_TEXT_BYTES
    );
    n.bridge.disable("app:deltachat").await;
    fake.stop().await;
}

// ---------------------------------------------------------------- states and sends

#[tokio::test]
async fn states_map_20_26_28_and_msg_failed_by_the_returned_msg_id() {
    let r = rig().await;
    joined_and_on(&r, ALICE, "Alice").await;
    // 20 → 26 → 28
    let mut h = r
        .adapter
        .send(&dm(ALICE), "hello", SendOpts::default())
        .await;
    assert_eq!(
        next_event(&mut h.events).await,
        Some(SendEvent::Evidence {
            state: ForeignMessageState::Queued,
            evidence: EVIDENCE_QUEUED.into()
        })
    );
    // the hidden Secure-Join messages also fire MsgDelivered: not ours, ignored
    r.fake
        .control(json!({"control": "hidden_delivered", "msg": 9999}))
        .await;
    r.fake.control(json!({"control": "deliver"})).await;
    let Some(SendEvent::Accepted { native_id }) = next_event(&mut h.events).await else {
        panic!("accepted at 26")
    };
    let sent = r
        .fake
        .log()
        .into_iter()
        .find(|v| v["answer"] == "misc_send_text_message")
        .unwrap();
    assert_eq!(
        native_id,
        sent["result"].as_u64().unwrap().to_string(),
        "keyed by msg id"
    );
    assert_eq!(
        next_event(&mut h.events).await,
        Some(SendEvent::Evidence {
            state: ForeignMessageState::Sent,
            evidence: EVIDENCE_SENT.into()
        })
    );
    r.fake.control(json!({"control": "read"})).await;
    assert_eq!(
        next_event(&mut h.events).await,
        Some(SendEvent::Evidence {
            state: ForeignMessageState::Delivered,
            evidence: EVIDENCE_DELIVERED.into()
        })
    );
    assert_eq!(
        next_event(&mut h.events).await,
        None,
        "delivered is terminal"
    );

    // MsgFailed before 26: failed, pre-air, never retried on its own
    let mut h = r
        .adapter
        .send(&dm(ALICE), "doomed", SendOpts::default())
        .await;
    assert!(matches!(
        next_event(&mut h.events).await,
        Some(SendEvent::Evidence {
            state: ForeignMessageState::Queued,
            ..
        })
    ));
    r.fake.control(json!({"control": "fail"})).await;
    assert_eq!(
        next_event(&mut h.events).await,
        Some(SendEvent::Failed {
            reason: Reason::GotNak,
            retry: Retry::Never
        })
    );

    // refusals before the core: not joined and on, another account, too large, a channel
    for (conv, text, reason) in [
        (
            dm("bob@[127.0.0.1]"),
            "x".to_string(),
            Reason::NotAuthorized,
        ),
        (
            ConvRef {
                account: "other@[127.0.0.1]".into(),
                ..dm(ALICE)
            },
            "x".to_string(),
            Reason::NotAuthorized,
        ),
        (dm(ALICE), "x".repeat(MAX_TEXT_BYTES + 1), Reason::TooLarge),
        (
            ConvRef {
                kind: ConvKind::Channel,
                native_id: "ch:x".into(),
                ..dm(ALICE)
            },
            "x".to_string(),
            Reason::NotAuthorized,
        ),
    ] {
        let mut h = r.adapter.send(&conv, &text, SendOpts::default()).await;
        assert_eq!(
            next_event(&mut h.events).await,
            Some(SendEvent::Failed {
                reason,
                retry: Retry::Never
            }),
            "{conv:?}"
        );
    }

    // a group: its conversation appears with an opted-in member's text, then takes sends
    r.fake
        .control(json!({"control": "incoming", "from": ALICE, "text": "in the group", "group": "Ops <i>team</i>"}))
        .await;
    let Inbound::Conversation { conv, title } = next_item(&r.adapter).await else {
        panic!("the group conversation")
    };
    assert_eq!(conv.kind, ConvKind::Group);
    assert!(conv.native_id.starts_with("group:"));
    assert_eq!(title, "Ops <i>team</i>", "untrusted, passed as text");
    let Inbound::Text(t) = next_item(&r.adapter).await else {
        panic!("the group text")
    };
    assert_eq!(
        (t.body.as_str(), t.sender_native.as_str()),
        ("in the group", ALICE)
    );
    let mut h = r
        .adapter
        .send(&conv, "to the group", SendOpts::default())
        .await;
    assert!(matches!(
        next_event(&mut h.events).await,
        Some(SendEvent::Evidence {
            state: ForeignMessageState::Queued,
            ..
        })
    ));
    r.fake.stop().await;
}

#[tokio::test]
async fn receive_passes_an_untrusted_name_with_markup_as_data_and_skips_info() {
    let r = rig().await;
    let name = "<img src=x onerror=alert(1)> Eve\u{200d}";
    joined_and_on(&r, "eve@[127.0.0.1]", name).await;
    let _ = drain(&r.adapter);
    // an info message and an empty one are not texts
    r.fake
        .control(json!({"control": "incoming", "from": "eve@[127.0.0.1]", "text": "Member added", "info": true}))
        .await;
    r.fake
        .control(json!({"control": "incoming", "from": "eve@[127.0.0.1]", "text": "", "file_bytes": 2048}))
        .await;
    let item = next_item(&r.adapter).await;
    let Inbound::Conversation { title, .. } = item else {
        panic!("{item:?}")
    };
    assert_eq!(title, name);
    let Inbound::Text(t) = next_item(&r.adapter).await else {
        panic!("the attachment notice")
    };
    assert_eq!(t.body, "attachment not shown (2048 bytes)");
    assert_eq!(
        t.sender_name, name,
        "passed as data; the core's ingest cleans it"
    );
    assert_eq!(t.auth, Auth::None);
    assert_eq!(
        t.claim_ts,
        Some(1_760_000_000),
        "the sender's claim, display only"
    );
    assert!(drain(&r.adapter).is_empty(), "the info message was no text");
    r.fake.stop().await;
}

#[tokio::test]
async fn an_own_relay_copy_carries_the_marker_as_its_visible_prefix_only() {
    let r = rig().await;
    joined_and_on(&r, ALICE, "Alice").await;
    let mut h = r
        .adapter
        .send(&dm(ALICE), "[W1AW7/M] (1/2) part one", SendOpts::default())
        .await;
    assert!(matches!(
        next_event(&mut h.events).await,
        Some(SendEvent::Evidence {
            state: ForeignMessageState::Queued,
            ..
        })
    ));
    let (_, params) = r
        .fake
        .requests()
        .into_iter()
        .find(|(m, _)| m == "misc_send_text_message")
        .unwrap();
    assert_eq!(params.as_array().unwrap().len(), 3);
    assert_eq!(params[2], "[W1AW7/M] (1/2) part one");
    r.fake.stop().await;
}

// ---------------------------------------------------------------- the trust boundary

#[tokio::test]
async fn malformed_oversize_and_unknown_kind_lines_are_counted_as_noise() {
    let r = rig().await;
    assert_eq!(r.adapter.noise(), 0);
    r.fake.control(json!({"control": "garbage"})).await;
    // 5 bad lines, 2 dropped by the supervisor (oversize, not UTF-8), 3 bad events
    until("ten noise items", || r.adapter.noise() >= 10).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(r.adapter.noise(), 10);
    assert_eq!(
        r.adapter.warnings(),
        1,
        "a Warning is counted, never logged"
    );
    // Info events are dropped at the reader, not noise; an overflow is shown
    r.fake.control(json!({"control": "info", "n": 150})).await;
    r.fake.control(json!({"control": "overflow", "n": 3})).await;
    let item = next_item(&r.adapter).await;
    assert_eq!(item, Inbound::Dropped { n: 3 });
    assert_eq!(r.adapter.noise(), 10);
    // still live and still bridging
    assert_eq!(r.adapter.state(), AdapterState::Live);
    joined_and_on(&r, ALICE, "Alice").await;
    r.fake
        .control(json!({"control": "incoming", "from": ALICE, "text": "still here"}))
        .await;
    let _conv = next_item(&r.adapter).await;
    let Inbound::Text(t) = next_item(&r.adapter).await else {
        panic!("a text")
    };
    assert_eq!(t.body, "still here");
    r.fake.stop().await;
}

#[tokio::test]
async fn a_respawn_re_attaches_and_a_request_in_flight_fails_adapter_down_later() {
    let r = rig().await;
    joined_and_on(&r, ALICE, "Alice").await;
    let g1 = r.adapter.generation().unwrap();
    let _ = drain(&r.adapter);
    // a send whose answer never comes: the child exits first
    r.fake
        .control(json!({"control": "delay", "method": "misc_send_text_message", "secs": 5}))
        .await;
    let mut h = r
        .adapter
        .send(&dm(ALICE), "in flight", SendOpts::default())
        .await;
    until("the send handed to the core", || {
        r.fake.count("misc_send_text_message") == 1
    })
    .await;
    r.fake.control(json!({"control": "exit"})).await;
    assert_eq!(
        next_event(&mut h.events).await,
        Some(SendEvent::Failed {
            reason: Reason::AdapterDown,
            retry: Retry::Later
        }),
        "§7: requests in flight fail adapter_down and re-queue"
    );
    until("a new generation", || {
        r.adapter.generation().is_some_and(|g| g != g1)
    })
    .await;
    wait_live(&r.adapter).await;
    let items = drain(&r.adapter);
    assert!(items.contains(&Inbound::Link(LinkState::Down)), "{items:?}");
    assert!(items.contains(&Inbound::Link(LinkState::Up)), "{items:?}");
    // the handshake ran again on the configured account: no second account or transport
    assert_eq!(r.fake.count("get_system_info"), 2);
    assert_eq!(r.fake.count("add_account"), 1);
    assert_eq!(r.fake.count("add_transport_from_qr"), 1);
    assert_eq!(
        r.fake.count("set_config"),
        8,
        "the settings again, every generation"
    );
    assert_eq!(r.relay.calls.load(Ordering::SeqCst), 1, "create-user once");
    // the joined contact is remembered; texts flow again
    r.fake
        .control(json!({"control": "incoming", "from": ALICE, "text": "back"}))
        .await;
    let _conv = next_item(&r.adapter).await;
    let Inbound::Text(t) = next_item(&r.adapter).await else {
        panic!("a text")
    };
    assert_eq!(t.body, "back");
    r.fake.stop().await;
}

// ---------------------------------------------------------------- Madmail and the declarations

#[test]
fn the_madmail_configuration_is_written_0600_in_0700_directories() {
    let dir = tempfile::tempdir().unwrap();
    let paths = RelayPaths::of(dir.path());
    deltachat::write_madmail_config(&paths, RelayBind::Loopback, RelayPorts::default()).unwrap();
    let text = std::fs::read_to_string(&paths.config).unwrap();
    assert_eq!(
        text,
        render_madmail_config(RelayBind::Loopback, RelayPorts::default(), &paths)
    );
    assert!(text.contains("imap_tls_listen = \"127.0.0.1:11242\""));
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&paths.config), 0o600);
    for d in [&paths.dir, &paths.state, &paths.runtime, &paths.accounts] {
        assert_eq!(mode(d), 0o700, "{}", d.display());
    }
    // the LAN form: every listener on the LAN address
    let lan = render_madmail_config(
        RelayBind::Lan(std::net::Ipv4Addr::new(192, 168, 1, 5)),
        RelayPorts::default(),
        &paths,
    );
    assert!(!lan.contains("127.0.0.1"), "{lan}");
    assert_eq!(lan.matches("192.168.1.5:1124").count(), 4);
}

fn embedded(id: &str) -> Component {
    manifest::load_embedded()
        .into_iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("{id} declared"))
}

#[test]
fn the_declarations_parse_through_the_component_loader() {
    let all = manifest::load_embedded();
    let ids: Vec<&str> = all.iter().map(|c| c.id.as_str()).collect();
    for id in ["deltachat", "lxmf", "madmail", "simplex"] {
        assert!(ids.contains(&id), "{id} registers: {ids:?}");
    }
    let mm = embedded("madmail");
    assert_eq!(mm.adapter, "app:deltachat");
    assert_eq!(mm.adapter_dir(), "deltachat");
    assert_eq!(mm.run.ports, [11240, 11241, 11242, 11243]);
    assert_eq!(mm.run.health, Health::None);
    assert_eq!(mm.requires.glibc, Some((2, 38)));
    assert_eq!(mm.license.spdx, "AGPL-3.0-or-later");
    assert!(
        mm.license.notice.contains("AGPL section 13"),
        "the source offer"
    );
    assert!(mm.run.args.contains(&"{bridge}/madmail.toml".to_string()));
    assert!(
        !mm.run.args.iter().any(|a| a == "install" || a == "upgrade"),
        "never install or upgrade"
    );
    if std::env::consts::ARCH == "x86_64" {
        assert!(matches!(
            &mm.payload,
            Payload::Binary {
                size: 77_331_312,
                ..
            }
        ));
        assert_eq!(mm.run.program, "madmail-linux-amd64");
    }
    let dc = embedded("deltachat");
    assert_eq!(dc.requires.component.as_deref(), Some("madmail"));
    assert_eq!(
        dc.run.env,
        [(
            "DC_ACCOUNTS_PATH".to_string(),
            "{bridge}/accounts".to_string()
        )]
    );
    assert!(dc.run.ports.is_empty(), "stdio only");
    assert_eq!(dc.license.spdx, "MPL-2.0");
    assert!(
        dc.license.notice.contains(ACCOUNT_WARNING),
        "the account-level warning is the enable's consent text"
    );
    let sx = embedded("simplex");
    assert_eq!(
        sx.payload,
        Payload::Unavailable {
            reason: "SimpleX is not enabled in this release".into()
        }
    );
    assert_eq!(sx.license.spdx, "AGPL-3.0-only");
    assert_eq!(sx.run.ports, [11232]);
    // the adapter manifest the networks view shows
    let m = node111::daemon::bridge::core::embedded_manifests()
        .into_iter()
        .find(|m| m.id == "app:deltachat")
        .unwrap();
    assert_eq!(
        m.evidence,
        [
            ForeignMessageState::Queued,
            ForeignMessageState::Sent,
            ForeignMessageState::Delivered
        ]
    );
    assert_eq!((m.limits.dm, m.limits.group), (Some(16384), Some(16384)));
    assert!(m
        .banner
        .contains("terminates Delta Chat's end-to-end encryption"));
}

struct NoDeps;

impl Dependencies for NoDeps {
    fn child(&self, _: ServiceId) -> Option<ChildControl> {
        None
    }
    fn serving(&self, _: ServiceId) -> bool {
        false
    }
}

fn components(root: &Path, list: Vec<Component>, cmd: Arc<dyn Runner>) -> Components {
    let mut cfg = ComponentsConfig::new(root);
    cfg.runner = RunnerConfig {
        retry: Duration::from_millis(50),
        retry_max: Duration::from_millis(200),
        settle: Duration::ZERO,
        grace: Duration::from_secs(2),
    };
    Components::new(
        cfg,
        list,
        Wiring {
            cmd,
            ports: Arc::new(PortTable::new()),
            deps: Arc::new(NoDeps),
            tick: Arc::new(|| 1),
        },
    )
}

fn out(status: i32, stdout: &str) -> Output {
    Output {
        status,
        stdout: stdout.into(),
        stderr: String::new(),
    }
}

#[tokio::test]
async fn the_glibc_gate_refuses_with_honest_copy_and_nothing_is_installed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("state")).unwrap();
    let list = vec![
        embedded("madmail"),
        embedded("deltachat"),
        embedded("simplex"),
    ];
    let hash = |id: &str| embedded(id).notice_hash();
    let refusal = |e: EnableError| match e {
        EnableError::RequiresUnmet(copy) => copy,
        other => panic!("requires_unmet, not {other:?}"),
    };

    // Debian 12 / Pi OS bookworm
    let cmd = Arc::new(MockRunner::new(vec![(
        "getconf GNU_LIBC_VERSION",
        out(0, "glibc 2.36\n"),
    )]));
    let c = components(root, list.clone(), cmd.clone());
    let e = c
        .enable("madmail", Some(&hash("madmail")))
        .await
        .unwrap_err();
    let body = e.problem(c.get("madmail")).body();
    assert_eq!(body["code"], "component_unavailable");
    assert_eq!(body["reason_code"], "requires_unmet");
    assert_eq!(
        refusal(e),
        "this release of Madmail needs glibc 2.38 or newer (Debian 13 / Ubuntu 24.04); this node has glibc 2.36"
    );
    cmd.assert_exhausted();

    // Termux: no glibc at all
    let cmd = Arc::new(MockRunner::new(vec![
        ("getconf GNU_LIBC_VERSION", out(1, "")),
        ("ldd --version", out(1, "")),
        ("uname -o", out(0, "Android\n")),
    ]));
    let c = components(root, list.clone(), cmd.clone());
    let copy = refusal(
        c.enable("madmail", Some(&hash("madmail")))
            .await
            .unwrap_err(),
    );
    assert_eq!(
        copy,
        "Madmail does not run on Termux: this release needs glibc 2.38 or newer, which Android does not have; enable it on a Linux hub (Debian 13 / Ubuntu 24.04)"
    );
    let cmd = Arc::new(MockRunner::new(vec![
        ("getconf GNU_LIBC_VERSION", out(1, "")),
        ("ldd --version", out(1, "")),
        ("uname -o", out(0, "Android\n")),
    ]));
    let c = components(root, list.clone(), cmd.clone());
    let copy = refusal(
        c.enable("simplex", Some(&hash("simplex")))
            .await
            .unwrap_err(),
    );
    assert!(copy.starts_with("SimpleX does not run on Termux"), "{copy}");

    // a glibc hub: SimpleX is not in this release; the core needs the relay first
    let cmd = Arc::new(MockRunner::new(vec![(
        "getconf GNU_LIBC_VERSION",
        out(0, "glibc 2.39\n"),
    )]));
    let c = components(root, list.clone(), cmd.clone());
    assert_eq!(
        refusal(
            c.enable("simplex", Some(&hash("simplex")))
                .await
                .unwrap_err()
        ),
        "SimpleX is not enabled in this release"
    );
    let c = components(
        root,
        list.clone(),
        Arc::new(MockRunner::new(Vec::<(String, Output)>::new())),
    );
    assert_eq!(
        refusal(
            c.enable("deltachat", Some(&hash("deltachat")))
                .await
                .unwrap_err()
        ),
        "it needs Madmail enabled first"
    );
    // glibc 2.39 passes; nothing is cached, so nothing is installed
    let cmd = Arc::new(MockRunner::new(vec![(
        "getconf GNU_LIBC_VERSION",
        out(0, "glibc 2.39\n"),
    )]));
    let c = components(root, list, cmd);
    let e = c
        .enable("madmail", Some(&hash("madmail")))
        .await
        .unwrap_err();
    assert_eq!(e.reason_code(), "payload_missing", "{e:?}");
    // the contract's 409: nothing downloads at runtime, the detail names the
    // command that fills the cache (`[install] prefetch = false`)
    let body = e.problem(None).body();
    assert_eq!(body["status"], 409, "{body}");
    assert_eq!(body["code"], "component_unavailable", "{body}");
    assert_eq!(body["reason_code"], "payload_missing", "{body}");
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .contains("`sudo 111 install --components madmail`"),
        "{body}"
    );
    assert!(!root.join("components").exists(), "nothing installed");
    assert!(
        !root.join("cache/packages/madmail").exists(),
        "nothing downloaded"
    );
    assert_eq!(
        c.status("madmail").await.unwrap().state,
        ComponentState::Declared
    );
}

// ---------------------------------------------------------------- the whole enable

/// The fake relay binary: records its arguments beside its config, answers the one-shot
/// CLI, prints a dclogin for `create-user`, and for `run` listens on the IMAP address of
/// the configuration 111 wrote (so the health check proves the rendered file is used).
const FAKE_MADMAIL: &str = r#"#!/bin/sh
echo "$5 $6 $7" >> "$(dirname "$2")/madmail-calls.log"
case "$5" in
  run) exec python3 -I -c '
import socket, sys, tomllib
cfg = tomllib.load(open(sys.argv[1], "rb"))
host, port = cfg["imap_tls_listen"].rsplit(":", 1)
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind((host, int(port)))
s.listen(8)
while True:
    c, _ = s.accept()
    c.close()
' "$2" ;;
  --json) printf '{"dclogin":"dclogin:hub@[127.0.0.1]/?p=pw&v=1&ih=127.0.0.1&ip=11242&is=ssl&sh=127.0.0.1&sp=11241&ss=ssl&ic=3","email":"hub@[127.0.0.1]","password":"pw","username":"hub"}\n' ;;
  *) exit 0 ;;
esac
"#;

fn free_ports() -> RelayPorts {
    let l: Vec<std::net::TcpListener> = (0..4)
        .map(|_| std::net::TcpListener::bind("127.0.0.1:0").unwrap())
        .collect();
    let p: Vec<u16> = l.iter().map(|l| l.local_addr().unwrap().port()).collect();
    RelayPorts {
        smtp: p[0],
        submission: p[1],
        imap: p[2],
        https: p[3],
    }
}

fn binary_toml(id: &str, file: &str, payload: &[u8], extra_run: &str, requires: &str) -> String {
    format!(
        r#"id = "{id}"
adapter = "app:deltachat"
title = "{title}"
[payload]
kind = "binary"
url = "https://example.invalid/dl/{file}"
sha256 = "{sha}"
size = {size}
[requires]
{requires}
[run]
program = "{file}"
{extra_run}
health = "none"
[license]
spdx = "MPL-2.0"
read_at = "https://example.invalid/LICENSE"
notice = "Test notice for {id}. You install it."
"#,
        title = if id == "madmail" {
            "Madmail"
        } else {
            "Delta Chat core"
        },
        sha = sha256_bytes(payload),
        size = payload.len(),
    )
}

#[tokio::test]
async fn the_whole_enable_renders_settles_checks_health_and_attaches() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("var/lib/111");
    std::fs::create_dir_all(root.join("state")).unwrap();
    let ports = free_ports();
    let core = format!(
        "#!/bin/sh\nexec \"{}\" -I \"{}\" --log \"$FAKE_DC_LOG\"\n",
        python3().display(),
        fake_script().display()
    );
    let mm = binary_toml(
        "madmail",
        "madmail-test",
        FAKE_MADMAIL.as_bytes(),
        &format!(
            "args = [\"--config\", \"{{bridge}}/madmail.toml\", \"--state-dir\", \"{{bridge}}/madmail\", \"run\"]\nports = {:?}",
            ports.all()
        ),
        "",
    );
    let dc = binary_toml(
        "deltachat",
        "deltachat-test",
        core.as_bytes(),
        "env = { DC_ACCOUNTS_PATH = \"{bridge}/accounts\", FAKE_DC_LOG = \"{bridge}/fake-log.jsonl\" }",
        "component = \"madmail\"",
    );
    let list = manifest::load(&[
        ("madmail.toml".into(), mm.into_bytes()),
        ("deltachat.toml".into(), dc.into_bytes()),
    ]);
    assert_eq!(list.len(), 2, "the test declarations register");
    for (id, file, bytes) in [
        ("madmail", "madmail-test", FAKE_MADMAIL.as_bytes()),
        ("deltachat", "deltachat-test", core.as_bytes()),
    ] {
        let d = root.join("cache/packages").join(id);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(file), bytes).unwrap();
    }
    let hash = |id: &str| list.iter().find(|c| c.id == id).unwrap().notice_hash();
    let comps = components(&root, list.clone(), Arc::new(SystemRunner));
    let mut s = Setup::new(
        &root,
        RelayBind::Loopback,
        Arc::new(SystemRunner),
        Arc::new(|_: &str, _: &str| false),
    );
    s.ports = ports;
    s.cfg.call_timeout = Duration::from_secs(5);
    s.cfg.settle_timeout = Duration::from_secs(2);
    s.listen_timeout = Duration::from_secs(10);

    // without the relay's acknowledgement: 409 with its notice, nothing installed
    let e = deltachat::enable(&comps, &s, None, None).await.unwrap_err();
    assert!(matches!(e, EnableError::ConsentRequired { .. }), "{e:?}");
    assert!(!root.join("components").exists());
    // the relay's, not yet the core's: the relay stays enabled, the core's 409
    let e = deltachat::enable(&comps, &s, Some(&hash("madmail")), None)
        .await
        .unwrap_err();
    let EnableError::ConsentRequired { hash: h, .. } = &e else {
        panic!("{e:?}")
    };
    assert_eq!(h, &hash("deltachat"));
    let paths = RelayPaths::of(&root);
    let calls = || std::fs::read_to_string(paths.dir.join("madmail-calls.log")).unwrap_or_default();
    let lines: Vec<String> = calls().lines().map(|l| l.trim().to_string()).collect();
    let settled: Vec<&str> = lines
        .iter()
        .map(String::as_str)
        .filter(|l| *l != "run")
        .collect();
    assert_eq!(
        settled,
        ["push off", "federation policy reject", "registration close"],
        "first start loopback-only, then stopped to settle: {lines:?}"
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some("run"),
        "then started"
    );
    // both: the core attaches, the bridge account is created on the relay once
    let adapter = deltachat::enable(&comps, &s, None, Some(&hash("deltachat")))
        .await
        .unwrap();
    wait_live(&adapter).await;
    assert_eq!(adapter.account().as_deref(), Some(HUB));
    let after: Vec<String> = calls().lines().map(|l| l.trim().to_string()).collect();
    assert_eq!(
        after.iter().filter(|l| l.as_str() == "run").count(),
        lines.iter().filter(|l| l.as_str() == "run").count(),
        "an enabled relay is not restarted: {after:?}"
    );
    assert!(
        after.contains(&"--json create-user".to_string()),
        "{after:?}"
    );
    let conf = std::fs::read_to_string(&paths.config).unwrap();
    assert!(conf.contains(&format!("imap_tls_listen = \"127.0.0.1:{}\"", ports.imap)));
    assert_eq!(
        std::fs::metadata(&paths.config)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(
        paths.accounts.join("fake-state.json").is_file(),
        "DC_ACCOUNTS_PATH"
    );
    assert_eq!(
        comps.status("madmail").await.unwrap().state,
        ComponentState::Running
    );

    // LAN sharing on: re-rendered on the LAN address, registration open, the account moved
    deltachat::relan(
        &comps,
        &s,
        RelayBind::Lan(std::net::Ipv4Addr::new(127, 0, 0, 2)),
    )
    .await
    .unwrap();
    let conf = std::fs::read_to_string(&paths.config).unwrap();
    assert!(
        conf.contains(&format!("imap_tls_listen = \"127.0.0.2:{}\"", ports.imap)),
        "{conf}"
    );
    assert!(calls().contains("registration open"));
    assert!(adapter.card().is_some());
    adapter.set_relay_host(Some("127.0.0.2".into()));
    assert_eq!(adapter.state(), AdapterState::Error("relay_moved".into()));
    assert!(
        adapter.card().is_none(),
        "a moved account's code is never offered"
    );

    // disable: nothing of either runs
    drop(adapter);
    comps.disable("deltachat").await;
    comps.disable("madmail").await;
    for id in ["deltachat", "madmail"] {
        assert_eq!(
            comps.status(id).await.unwrap().state,
            ComponentState::Disabled
        );
    }
    comps.shutdown().await;
}
