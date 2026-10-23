//! Layer 6 C3 (Rust half) acceptance (`os/6-transport/README.md` §6, §4, §14, §15.9;
//! `6-transport/TASKS.md` row C3): the LXMF adapter `daemon/bridge/lxmf.rs` against a
//! scripted helper (`tests/fake_lxmf_sidecar.py`, the JSON Lines protocol of
//! `6-transport/sidecars/lxmf-sidecar/README.md`) spawned by the real supervisor with
//! piped stdio, so every generation, respawn and stdin write is the production path.
//!
//! - `ready` feeds the account and a fresh verifier (the frozen `0x03` vector verifies with
//!   the reported key), `msg` becomes `Inbound::Text` with its labels, `heard` a
//!   `reticulum` Nearby source (coalesced per destination) and the re-send of a post-air
//!   unconfirmed send, `state` maps onto `SendEvent`s monotonically, malformed lines are
//!   noise, a respawn re-attaches and fails sends in flight, a first line that is not
//!   `ready` is refused, `partitioned` is an error state, `send` lines are byte-exact (the
//!   relay custom field included), `quit` on disable;
//! - no foreign text reaches a log line (a child run of this binary, stderr captured);
//! - the component declaration registers through S4's loader with its byte-identical lock;
//! - the adapter registers through `Bridge::enable` and its texts, echoes, Nearby items and
//!   sends flow through the bridge core onto a real fabric;
//! - FA2 (V2's gaps): announce on request (§6 (3)) — one `announce {}` after the `ready` of
//!   every run and one when a send parks as `peer_unannounced`, bounded per run — and the
//!   parked row through the core: queued, "waiting for their announce", handed once.
//!
//! Hermetic: every helper child is stopped on the way out (and dies with the test process
//! through the parent-death signal otherwise); scratch lives in temp dirs.

use futures_util::FutureExt;
use mesh111::store::{ForeignAuth, ForeignDir, ForeignKind, ForeignState};
use node111::daemon::bridge::attest::Leg;
use node111::daemon::bridge::lxmf::{
    nearby_source, LxmfAdapter, LxmfConfig, ReadyFacts, EV_DELIVERED, EV_NO_PATH, EV_SENT,
    EV_UNANNOUNCED, EV_UNCONFIRMED,
};
use node111::daemon::bridge::{
    Adapter, AdapterState, Auth, Bridge, BridgeConfig, ConvKind, ConvRef, ForeignMessageState,
    Inbound, InboundText, LinkState, Reason, Retry, SendEvent, SendOpts,
};
use node111::daemon::components::manifest::{self, Health, Payload};
use node111::daemon::dashboard::{ApiConfig, ApiState, DashboardState, SharedState};
use node111::daemon::fabric::{self, FabricConfig};
use node111::daemon::services::ServiceId;
use node111::daemon::supervise::{spawn_child, BackoffPolicy, ChildHandle, ChildSpec, ChildStdio};
use node111::daemon::DaemonLayout;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const LIMIT: Duration = Duration::from_secs(15);
/// Our delivery destination, peers, a sender.
const D1: &str = "0123456789abcdef0123456789abcdef";
const D2: &str = "fedcba9876543210fedcba9876543210";
const D3: &str = "00112233445566778899aabbccddeeff";
const D4: &str = "ffeeddccbbaa99887766554433221100";
const SRC: &str = "5555aaaa5555aaaa5555aaaa5555aaaa";
/// A relay field's `mid`: 32 lowercase hex, the shape this adapter writes (§6, §8).
const MID: &str = "00ff00ff00ff00ff00ff00ff00ff00ff";
const VECTORS: &str =
    include_str!("../../../../2-identity/test-vectors/layer6/attest-transport-ids.json");

fn h(c: char) -> String {
    std::iter::repeat_n(c, 64).collect()
}

fn vectors() -> Value {
    serde_json::from_str(VECTORS).unwrap()
}

/// The frozen identity's 64 B public key (hex), as a helper would report it.
fn pk_hex() -> String {
    vectors()["reticulum"]["pubkey64"]
        .as_str()
        .unwrap()
        .to_string()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn ready(dest: &str, pk: &str) -> Value {
    json!({"emit": {"type": "ready", "dest": dest, "pubkey": pk, "instance": "111"}})
}

fn link(state: &str) -> Value {
    json!({"emit": {"type": "link", "state": state}})
}

fn fake() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_lxmf_sidecar.py")
}

/// One scripted helper under the real supervisor, and the adapter attached to it.
struct Helper {
    dir: tempfile::TempDir,
    handle: ChildHandle,
    adapter: Arc<LxmfAdapter>,
}

/// The helper's `announce {}` command, as the adapter writes it (§6 (3)).
const ANNOUNCE: &str = r#"{"type":"announce"}"#;

impl Helper {
    fn record(&self) -> PathBuf {
        self.dir.path().join("record")
    }

    /// Every line the helper read from its stdin, in order, announces included.
    fn all_lines(&self) -> Vec<String> {
        std::fs::read_to_string(self.record())
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Every line the helper read from its stdin but the adapter's `announce {}` (written
    /// once after each `ready`, §6 (3); the announce tests count those).
    fn lines(&self) -> Vec<String> {
        self.all_lines()
            .into_iter()
            .filter(|l| l != ANNOUNCE)
            .collect()
    }

    fn announces(&self) -> usize {
        self.all_lines().iter().filter(|l| *l == ANNOUNCE).count()
    }

    async fn wait_lines(&self, n: usize) -> Vec<String> {
        assert!(
            until(LIMIT, || self.lines().len() >= n).await,
            "{:?}",
            self.lines()
        );
        self.lines()
    }

    async fn live(&self) {
        let a = Arc::clone(&self.adapter);
        assert!(
            until(LIMIT, move || a.caps().state == AdapterState::Live).await,
            "never live: {:?}",
            self.adapter.caps().state
        );
    }

    async fn stop(self) {
        self.handle.stop_and_wait(Duration::from_secs(5)).await;
    }
}

fn spawn_with(runs: Value, log_to_file: bool, f: impl FnOnce(&mut LxmfConfig)) -> Helper {
    let dir = tempfile::tempdir().unwrap();
    let scenario = dir.path().join("scenario.json");
    std::fs::write(&scenario, json!({ "runs": runs }).to_string()).unwrap();
    let record = dir.path().join("record");
    let spec = ChildSpec {
        name: "lxmf-fake".into(),
        program: PathBuf::from("python3"),
        args: vec![
            "-I".into(),
            fake().to_string_lossy().into_owned(),
            scenario.to_string_lossy().into_owned(),
            record.to_string_lossy().into_owned(),
        ],
        env: vec![],
        backoff: BackoffPolicy {
            initial_ms: 20,
            max_ms: 50,
            jitter_pct: 0,
            crash_loop_threshold: 100,
            ..BackoffPolicy::default()
        },
        stdout_log: log_to_file.then(|| dir.path().join("stderr.log")),
    };
    let handle = spawn_child(spec.with_stdio(ChildStdio::Piped));
    let mut cfg = LxmfConfig {
        heard_interval: Duration::ZERO,
        ..LxmfConfig::default()
    };
    f(&mut cfg);
    let adapter = LxmfAdapter::start(handle.stdio(), cfg);
    Helper {
        dir,
        handle,
        adapter,
    }
}

fn spawn(runs: Value) -> Helper {
    spawn_with(runs, true, |_| {})
}

async fn until(limit: Duration, f: impl Fn() -> bool) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < limit {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    f()
}

async fn next(a: &LxmfAdapter) -> Inbound {
    tokio::time::timeout(LIMIT, a.recv())
        .await
        .expect("an inbound item within the limit")
}

async fn ev(rx: &mut mpsc::Receiver<SendEvent>) -> Option<SendEvent> {
    tokio::time::timeout(LIMIT, rx.recv())
        .await
        .expect("a send event within the limit")
}

fn evidence(state: ForeignMessageState, e: &str) -> Option<SendEvent> {
    Some(SendEvent::Evidence {
        state,
        evidence: e.into(),
    })
}

fn accepted(hash: &str) -> Option<SendEvent> {
    Some(SendEvent::Accepted {
        native_id: hash.into(),
    })
}

fn dm(account: &str, peer: &str) -> ConvRef {
    ConvRef {
        network: "lxmf".into(),
        account: account.into(),
        kind: ConvKind::Dm,
        native_id: format!("dm:{peer}"),
    }
}

async fn send(a: &LxmfAdapter, to: &str, text: &str) -> mpsc::Receiver<SendEvent> {
    a.send(&dm(D1, to), text, SendOpts::default()).await.events
}

// ---------------------------------------------------------------- ready and the verifier

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ready_feeds_the_account_and_a_fresh_verifier_and_quit_ends_the_run() {
    let seen: Arc<Mutex<Vec<ReadyFacts>>> = Arc::default();
    let hook = Arc::clone(&seen);
    let hp = spawn_with(json!([[ready(D1, &pk_hex()), link("up")]]), true, |c| {
        c.on_ready = Some(Arc::new(move |r: &ReadyFacts| {
            hook.lock().unwrap().push(r.clone())
        }));
    });
    hp.live().await;
    let a = &hp.adapter;
    assert_eq!(next(a).await, Inbound::Link(LinkState::Up));
    let facts = a.facts();
    assert_eq!(facts.accounts.len(), 1);
    assert_eq!(facts.accounts[0].id, D1);
    assert!(!facts.accounts[0].licensed);
    assert!(facts.ids_globally_unique, "LXMF hashes dedupe on the key");
    let r = a.ready().unwrap();
    assert_eq!((r.dest.as_str(), r.instance.as_str()), (D1, "111"));
    assert_eq!(r.pubkey.to_vec(), unhex(&pk_hex()));
    assert_eq!(seen.lock().unwrap().as_slice(), std::slice::from_ref(&r));
    // §14: the verifier binds the reported key: S3's frozen 0x03 record verifies both legs
    let v = vectors();
    let case = v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "reticulum both legs")
        .unwrap();
    let att =
        id111::attest::Attestation::from_bytes(&unhex(case["wire"].as_str().unwrap())).unwrap();
    let ok = a.verifier().verify_attestation(&att, None).unwrap();
    assert_eq!(ok.leg, Leg::Both);
    // caps (§4, §6)
    let caps = a.caps();
    assert_eq!(caps.kinds, vec![ConvKind::Dm, ConvKind::Group]);
    assert_eq!(caps.max_text_bytes.dm, Some(287));
    assert_eq!(caps.max_text_bytes.channel, None);
    assert_eq!(caps.auth_levels, vec![Auth::None, Auth::Signed]);
    assert!(caps.airtime.is_none() && caps.splittable);
    assert_eq!(caps.copy.tag, "LX");
    assert_eq!(caps.recipient_syntax.unwrap().pattern, "^[0-9a-f]{32}$");
    // quit on disable: the line reaches the helper, which exits; nothing is sent after
    a.quit().await;
    assert_eq!(a.caps().state, AdapterState::Disabled);
    // (a respawn after the clean exit is asked to quit at once as well)
    let lines = hp.wait_lines(1).await;
    assert!(lines.iter().all(|l| l == r#"{"type":"quit"}"#), "{lines:?}");
    let mut rx = send(a, D2, "late").await;
    assert_eq!(
        ev(&mut rx).await,
        Some(SendEvent::Failed {
            reason: Reason::AdapterDown,
            retry: Retry::Later
        })
    );
    let tkey: [u8; 32] = unhex(vectors()["reticulum"]["transport_key"].as_str().unwrap())
        .try_into()
        .unwrap();
    assert!(
        a.verifier().reticulum_key_for(&tkey).is_none(),
        "no run, no key"
    );
    hp.stop().await;
}

// ---------------------------------------------------------------- msg

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn msg_lines_become_inbound_texts_with_their_labels() {
    let signed = json!({"emit": {
        "type": "msg", "hash": h('1'), "src": SRC, "name": "Ann", "text": "[W1AW7/M] hello",
        "claim_ts": 1700000000.5, "signed": true, "why_unsigned": null,
        "fields": {
            "5": {"attachment": {"bytes": 1200}},
            "251": "111.relay",
            "252": {"v": 1, "net": "meshtastic", "id": "W1AW7", "mid": MID},
            "relay": {"v": 1, "net": "meshtastic", "id": "W1AW7", "mid": MID}
        }
    }});
    let unsigned = json!({"emit": {
        "type": "msg", "hash": h('2'), "src": SRC, "name": null, "text": "who",
        "claim_ts": null, "signed": false, "why_unsigned": "source_unknown", "fields": {},
        "truncated": true
    }});
    let hp = spawn(json!([[ready(D1, &pk_hex()), signed, unsigned]]));
    hp.live().await;
    let a = &hp.adapter;
    let conv = dm(D1, SRC);
    assert_eq!(
        next(a).await,
        Inbound::Conversation {
            conv: conv.clone(),
            title: "Ann".into()
        }
    );
    assert_eq!(
        next(a).await,
        Inbound::Text(InboundText {
            conv: conv.clone(),
            sender_native: SRC.into(),
            sender_name: "Ann".into(),
            auth: Auth::Signed,
            native_msg_id: h('1'),
            body: "[W1AW7/M] hello".into(),
            claim_ts: Some(1_700_000_000),
            meta: json!({
                "relay": {"v": 1, "net": "meshtastic", "id": "W1AW7", "mid": MID},
                "attachments": {"count": 1, "bytes": 1200}
            }),
        })
    );
    // no name: no conversation upsert; unsigned with the library's reason in meta
    assert_eq!(
        next(a).await,
        Inbound::Text(InboundText {
            conv,
            sender_native: SRC.into(),
            sender_name: String::new(),
            auth: Auth::None,
            native_msg_id: h('2'),
            body: "who".into(),
            claim_ts: None,
            meta: json!({"why_unsigned": "source_unknown", "helper_truncated": true}),
        })
    );
    assert_eq!(a.noise(), 0);
    hp.stop().await;
}

/// F2 + F3 (review of C3rs): a relay field whose `id` is not a §8 marker name is refused
/// alone; a field nested deeper than `FIELDS_DEPTH` (3) is dropped alone. Either way the
/// message — valid text, hash and signature — arrives, flagged `fields_truncated`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn f2_f3_a_bad_relay_field_or_a_too_deep_field_drops_the_field_not_the_message() {
    let deep = json!({"emit": {
        "type": "msg", "hash": h('3'), "src": SRC, "name": null, "text": "deep",
        "claim_ts": null, "signed": true, "why_unsigned": null,
        "fields": {"9": [[[1]]]}
    }});
    let bad_relay = json!({"emit": {
        "type": "msg", "hash": h('4'), "src": SRC, "name": null, "text": "[ELEVENBYTES/M] x",
        "claim_ts": null, "signed": true, "why_unsigned": null,
        "fields": {"relay": {"v": 1, "net": "meshtastic", "id": "ELEVENBYTES", "mid": MID}}
    }});
    let bad_mid = json!({"emit": {
        "type": "msg", "hash": h('5'), "src": SRC, "name": null, "text": "y",
        "claim_ts": null, "signed": true, "why_unsigned": null,
        "fields": {"relay": {"v": 1, "net": "meshtastic", "id": "W1AW7", "mid": "00ff"}}
    }});
    let hp = spawn(json!([[ready(D1, &pk_hex()), deep, bad_relay, bad_mid]]));
    hp.live().await;
    let a = &hp.adapter;
    let text = |hash: char, body: &str, meta: Value| {
        Inbound::Text(InboundText {
            conv: dm(D1, SRC),
            sender_native: SRC.into(),
            sender_name: String::new(),
            auth: Auth::Signed,
            native_msg_id: h(hash),
            body: body.into(),
            claim_ts: None,
            meta,
        })
    };
    assert_eq!(
        next(a).await,
        text('3', "deep", json!({"fields_truncated": true}))
    );
    assert_eq!(next(a).await, text('4', "[ELEVENBYTES/M] x", json!({})));
    assert_eq!(next(a).await, text('5', "y", json!({})));
    assert_eq!(a.fields_dropped(), 1);
    assert_eq!(a.relay_refused(), 2);
    assert_eq!(a.noise(), 0, "a dropped field is not a malformed line");
    hp.stop().await;
}

// ---------------------------------------------------------------- heard

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn heard_feeds_nearby_is_rate_limited_and_resends_an_unconfirmed_send() {
    let heard = |d: &str, n: &str| json!({"emit": {"type": "heard", "dest": d, "name": n}});
    let runs = json!([[
        ready(D1, &pk_hex()),
        heard(D2, "Bob\u{202e}"),
        heard(D2, "Bob again"),
        {"expect": "send", "as": "s1"},
        {"state": {"ref": "s1", "hash": h('a'), "state": "queued"}},
        {"state": {"ref": "s1", "hash": h('a'), "state": "sent"}},
        {"state": {"ref": "s1", "hash": h('a'), "state": "sent", "reason": "transmitted, not confirmed"}},
        {"heard_of": "s1", "name": "Carol"},
        {"expect": "send", "as": "s1b"},
        {"state": {"ref": "s1b", "hash": h('b'), "state": "delivered"}},
        {"expect": "send", "as": "s2"},
        {"state": {"ref": "s2", "state": "queued", "reason": "peer_unannounced"}},
        {"heard_of": "s2", "name": null},
        {"state": {"ref": "s2", "hash": h('c'), "state": "queued"}},
        {"state": {"ref": "s2", "hash": h('c'), "state": "sent"}}
    ]]);
    let hp = spawn_with(runs, true, |c| c.heard_interval = Duration::from_secs(3600));
    hp.live().await;
    let a = &hp.adapter;
    // the name is untrusted text: control and format characters stripped
    assert_eq!(next(a).await, Inbound::Nearby(nearby_source(D1, D2, "Bob")));
    let mut s1 = send(a, D3, "one").await;
    // the hash rides the `queued` line; `Accepted` waits for air (F1)
    assert_eq!(
        ev(&mut s1).await,
        evidence(ForeignMessageState::Queued, EV_NO_PATH)
    );
    assert_eq!(ev(&mut s1).await, accepted(&h('a')));
    assert_eq!(
        ev(&mut s1).await,
        evidence(ForeignMessageState::Sent, EV_SENT)
    );
    assert_eq!(
        ev(&mut s1).await,
        evidence(ForeignMessageState::Sent, EV_UNCONFIRMED)
    );
    assert_eq!(EV_UNCONFIRMED, "transmitted, not confirmed");
    // the peer's next heard surfaces and the same send line is written again (§6 (4))
    assert_eq!(
        next(a).await,
        Inbound::Nearby(nearby_source(D1, D3, "Carol"))
    );
    assert_eq!(ev(&mut s1).await, accepted(&h('b')));
    assert_eq!(
        ev(&mut s1).await,
        evidence(ForeignMessageState::Delivered, EV_DELIVERED)
    );
    assert_eq!(ev(&mut s1).await, None, "a final state closes the handle");
    let lines = hp.wait_lines(2).await;
    assert_eq!(lines[0], lines[1], "the same line, the same id");
    // a parked send waits for the announce inside the helper; its heard wakes the core
    let mut s2 = send(a, D4, "two").await;
    assert_eq!(
        ev(&mut s2).await,
        evidence(ForeignMessageState::Queued, EV_UNANNOUNCED)
    );
    assert_eq!(next(a).await, Inbound::Nearby(nearby_source(D1, D4, "")));
    assert_eq!(ev(&mut s2).await, accepted(&h('c')));
    assert_eq!(
        ev(&mut s2).await,
        evidence(ForeignMessageState::Sent, EV_SENT)
    );
    assert!(
        a.coalesced() >= 1,
        "the second heard of D2 was not surfaced"
    );
    assert!(a.recv().now_or_never().is_none());
    assert_eq!(
        hp.wait_lines(3).await.len(),
        3,
        "no re-send of a parked send"
    );
    assert_eq!(
        hp.announces(),
        1,
        "the one after ready; the park came inside the 60 s window"
    );
    hp.stop().await;
}

// ---------------------------------------------------------------- announce on request (FA2)

/// FA2 (a), §6 (3) "on request": the adapter writes `announce {}` once right after the
/// `ready` of every run, and once when a send parks as `peer_unannounced` — at most one per
/// `announce_interval` per helper run (monotonic, evaluated where the lines are handled; no
/// timer). A park outside the window announces, a second park inside it does not, a new
/// run announces again at once (the bound is per run).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fa2_announce_after_ready_and_on_a_parked_send_bounded_per_run() {
    let window = Duration::from_millis(1500);
    let runs = json!([
        [
            ready(D1, &pk_hex()),
            {"expect": "send", "as": "s1"},
            {"state": {"ref": "s1", "state": "queued", "reason": "peer_unannounced"}},
            {"expect": "send", "as": "s2"},
            {"state": {"ref": "s2", "state": "queued", "reason": "peer_unannounced"}},
            {"expect": "send", "as": "s3"},
            {"state": {"ref": "s3", "hash": h('3'), "state": "queued"}},
            {"state": {"ref": "s3", "hash": h('3'), "state": "sent"}},
            {"sleep_ms": 200},
            {"exit": 0}
        ],
        [ready(D1, &pk_hex())]
    ]);
    let hp = spawn_with(runs, true, |c| c.announce_interval = window);
    hp.live().await;
    let a = &hp.adapter;
    let g1 = a.ready().unwrap().generation;
    // (i) right after ready, before anything else is written
    assert!(until(LIMIT, || hp.announces() == 1).await);
    assert_eq!(hp.all_lines(), vec![ANNOUNCE.to_string()]);
    // outside the window: the park announces once
    tokio::time::sleep(window + Duration::from_millis(300)).await;
    let mut s1 = send(a, D2, "one").await;
    assert_eq!(
        ev(&mut s1).await,
        evidence(ForeignMessageState::Queued, EV_UNANNOUNCED)
    );
    assert!(
        until(LIMIT, || hp.announces() == 2).await,
        "{:?}",
        hp.all_lines()
    );
    // inside the window: a second park announces nothing; a send that is not parked neither
    let mut s2 = send(a, D3, "two").await;
    assert_eq!(
        ev(&mut s2).await,
        evidence(ForeignMessageState::Queued, EV_UNANNOUNCED)
    );
    let mut s3 = send(a, D4, "three").await;
    assert_eq!(
        ev(&mut s3).await,
        evidence(ForeignMessageState::Queued, EV_NO_PATH)
    );
    assert_eq!(ev(&mut s3).await, accepted(&h('3')));
    // the run ends (exit 0) and the supervisor respawns it: a new run announces at once,
    // inside the old run's window
    let a2 = Arc::clone(a);
    assert!(
        until(LIMIT, move || a2.ready().is_some_and(|r| r.generation > g1)).await,
        "no second run"
    );
    assert!(
        until(LIMIT, || hp.announces() == 3).await,
        "{:?}",
        hp.all_lines()
    );
    let all = hp.all_lines();
    let kinds: Vec<&str> = all
        .iter()
        .map(|l| {
            if l == ANNOUNCE {
                "announce"
            } else if l.contains(r#""type":"send""#) {
                "send"
            } else {
                "other"
            }
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["announce", "send", "announce", "send", "send", "announce"],
        "{all:?}"
    );
    // nothing more while the second run idles
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(hp.announces(), 3);
    hp.stop().await;
}

/// FA2 (a): with the default bound (60 s) a park right after `ready` adds no announce (the
/// one after ready counts), and a run whose first line is not `ready` gets none.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fa2_the_default_window_counts_the_ready_announce() {
    assert_eq!(
        LxmfConfig::default().announce_interval,
        Duration::from_secs(60)
    );
    let runs = json!([[
        ready(D1, &pk_hex()),
        {"expect": "send", "as": "s1"},
        {"state": {"ref": "s1", "state": "queued", "reason": "peer_unannounced"}},
        {"expect": "send", "as": "s2"},
        {"state": {"ref": "s2", "state": "queued", "reason": "peer_unannounced"}}
    ]]);
    let hp = spawn(runs);
    hp.live().await;
    let a = &hp.adapter;
    for (to, text) in [(D2, "one"), (D3, "two")] {
        let mut rx = send(a, to, text).await;
        assert_eq!(
            ev(&mut rx).await,
            evidence(ForeignMessageState::Queued, EV_UNANNOUNCED)
        );
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(hp.announces(), 1, "{:?}", hp.all_lines());
    assert_eq!(hp.all_lines()[0], ANNOUNCE);
    hp.stop().await;
}

// ---------------------------------------------------------------- state

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn state_lines_map_onto_send_events_monotonically() {
    let runs = json!([[
        ready(D1, &pk_hex()),
        {"expect": "send", "as": "s1"},
        {"expect": "send", "as": "s2"},
        {"expect": "send", "as": "s3"},
        {"expect": "send", "as": "s4"},
        {"state": {"ref": "s1", "hash": h('1'), "state": "sent"}},
        {"state": {"ref": "s1", "hash": h('1'), "state": "queued"}},
        {"state": {"ref": "s1", "hash": h('1'), "state": "delivered"}},
        {"state": {"ref": "s1", "hash": h('1'), "state": "sent"}},
        {"state": {"ref": "s2", "hash": h('2'), "state": "failed", "reason": "rejected"}},
        {"state": {"ref": "s3", "state": "failed", "reason": "helper_busy"}},
        {"state": {"ref": "s4", "state": "queued"}},
        {"state": {"ref": "s4", "state": "failed", "reason": "cancelled"}}
    ]]);
    let hp = spawn(runs);
    hp.live().await;
    let a = &hp.adapter;
    let mut s1 = send(a, D2, "1").await;
    let mut s2 = send(a, D2, "2").await;
    let mut s3 = send(a, D2, "3").await;
    let mut s4 = send(a, D2, "4").await;
    assert_eq!(ev(&mut s1).await, accepted(&h('1')));
    assert_eq!(
        ev(&mut s1).await,
        evidence(ForeignMessageState::Sent, EV_SENT)
    );
    assert_eq!(
        ev(&mut s1).await,
        evidence(ForeignMessageState::Delivered, EV_DELIVERED)
    );
    assert_eq!(
        ev(&mut s1).await,
        None,
        "the late lower states never surface"
    );
    // refused before air: its hash is never an `Accepted` (F1)
    assert_eq!(
        ev(&mut s2).await,
        Some(SendEvent::Failed {
            reason: Reason::NotAuthorized,
            retry: Retry::Never
        })
    );
    assert_eq!(
        ev(&mut s3).await,
        Some(SendEvent::Failed {
            reason: Reason::AdapterDown,
            retry: Retry::Later
        })
    );
    assert_eq!(
        ev(&mut s4).await,
        evidence(ForeignMessageState::Queued, EV_NO_PATH)
    );
    assert_eq!(
        ev(&mut s4).await,
        Some(SendEvent::Failed {
            reason: Reason::Cancelled,
            retry: Retry::Never
        })
    );
    for rx in [&mut s2, &mut s3, &mut s4] {
        assert_eq!(ev(rx).await, None);
    }
    assert_eq!(a.in_flight(), 0);
    hp.stop().await;
}

// ---------------------------------------------------------------- noise

fn malformed() -> Vec<Value> {
    vec![
        json!({"raw": "not json"}),
        json!({"long": 70 * 1024}),
        json!({"emit": {"type": "hello"}}),
        json!({"emit": {"type": "heard", "dest": format!("{D2}0"), "name": "x"}}),
        json!({"raw": "[1,2]"}),
        json!({"emit": {"type": "state", "id": "m1", "state": "queued", "reason": "why not"}}),
        json!({"emit": {"type": "msg", "hash": h('4'), "src": SRC}}),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_lines_are_noise_and_never_surface() {
    let mut steps = vec![ready(D1, &pk_hex())];
    steps.extend(malformed());
    steps.push(json!({"emit": {"type": "heard", "dest": D2, "name": "fine"}}));
    let hp = spawn(json!([steps]));
    hp.live().await;
    let a = &hp.adapter;
    assert_eq!(
        next(a).await,
        Inbound::Nearby(nearby_source(D1, D2, "fine"))
    );
    assert!(a.recv().now_or_never().is_none(), "nothing else surfaced");
    // six of ours plus the supervisor's oversize drop
    assert_eq!(a.noise(), 7);
    assert_eq!(
        a.caps().state,
        AdapterState::Live,
        "noise never stops the adapter"
    );
    hp.stop().await;
}

// ---------------------------------------------------------------- respawn, refusal, partition

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_respawn_reattaches_and_fails_sends_in_flight() {
    let other_pk = "77".repeat(64);
    let runs = json!([
        [
            ready(D1, &pk_hex()),
            link("up"),
            {"expect": "send", "as": "s1"},
            {"state": {"ref": "s1", "hash": h('1'), "state": "queued"}},
            {"sleep_ms": 100},
            {"exit": 1}
        ],
        [ready(D2, &other_pk), link("up")]
    ]);
    let hp = spawn(runs);
    hp.live().await;
    let a = &hp.adapter;
    let g1 = a.ready().unwrap().generation;
    assert_eq!(next(a).await, Inbound::Link(LinkState::Up));
    let mut s1 = send(a, D3, "in flight").await;
    assert_eq!(
        ev(&mut s1).await,
        evidence(ForeignMessageState::Queued, EV_NO_PATH)
    );
    assert_eq!(
        ev(&mut s1).await,
        Some(SendEvent::Failed {
            reason: Reason::AdapterDown,
            retry: Retry::Later
        }),
        "never on air, never accepted: the core re-queues it (F1)"
    );

    assert_eq!(next(a).await, Inbound::Link(LinkState::Down));
    assert_eq!(next(a).await, Inbound::Link(LinkState::Up));
    let r = a.ready().unwrap();
    assert!(r.generation > g1);
    assert_eq!(r.dest, D2);
    assert_eq!(a.facts().accounts[0].id, D2);
    // a fresh verifier per run: the first run's key is gone
    let tkey: [u8; 32] = unhex(vectors()["reticulum"]["transport_key"].as_str().unwrap())
        .try_into()
        .unwrap();
    assert!(a.verifier().reticulum_key_for(&tkey).is_none());
    assert_eq!(a.in_flight(), 0);
    hp.stop().await;
}

/// F1 (review of C3rs): the helper reports the hash as soon as the library packed the
/// message, while it may still be `queued` (OUTBOUND, nothing on air). `Accepted` waits
/// for `sent`, so a helper killed while a send is queued fails it pre-air (`Later`, no
/// native id: the core keeps the row `queued` and re-hands it); a send already `sent` was
/// accepted, so the same failure reads post-air. The queued hash still guards own echoes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn f1_accepted_only_on_air_a_killed_helper_fails_a_queued_send_pre_air() {
    let echo = json!({"emit": {
        "type": "msg", "hash": h('1'), "src": SRC, "name": "Ann", "text": "ours",
        "claim_ts": null, "signed": true, "why_unsigned": null, "fields": {}
    }});
    let runs = json!([
        [
            ready(D1, &pk_hex()),
            link("up"),
            {"expect": "send", "as": "s1"},
            {"expect": "send", "as": "s2"},
            {"state": {"ref": "s1", "hash": h('1'), "state": "queued"}},
            {"state": {"ref": "s2", "hash": h('2'), "state": "queued"}},
            {"state": {"ref": "s2", "hash": h('2'), "state": "sent"}},
            echo,
            {"sleep_ms": 100},
            {"kill": true}
        ],
        [ready(D1, &pk_hex()), link("up")]
    ]);
    let hp = spawn(runs);
    hp.live().await;
    let a = &hp.adapter;
    assert_eq!(next(a).await, Inbound::Link(LinkState::Up));
    let mut s1 = send(a, D2, "queued when killed").await;
    let mut s2 = send(a, D3, "sent when killed").await;
    assert_eq!(
        ev(&mut s1).await,
        evidence(ForeignMessageState::Queued, EV_NO_PATH)
    );
    assert_eq!(
        ev(&mut s1).await,
        Some(SendEvent::Failed {
            reason: Reason::AdapterDown,
            retry: Retry::Later
        }),
        "never accepted: a pre-air Later"
    );
    assert_eq!(ev(&mut s1).await, None);
    assert_eq!(
        ev(&mut s2).await,
        evidence(ForeignMessageState::Queued, EV_NO_PATH)
    );
    assert_eq!(ev(&mut s2).await, accepted(&h('2')), "accepted once on air");
    assert_eq!(
        ev(&mut s2).await,
        evidence(ForeignMessageState::Sent, EV_SENT)
    );
    assert_eq!(
        ev(&mut s2).await,
        Some(SendEvent::Failed {
            reason: Reason::AdapterDown,
            retry: Retry::Later
        })
    );
    assert_eq!(ev(&mut s2).await, None);
    // the queued send's hash was kept: its echo never surfaces as an inbound text
    assert_eq!(next(a).await, Inbound::Link(LinkState::Down));
    assert_eq!(next(a).await, Inbound::Link(LinkState::Up));
    assert!(a.recv().now_or_never().is_none(), "no echo surfaced");
    assert_eq!(a.echoes(), 1);
    assert_eq!(a.in_flight(), 0);
    hp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_whose_first_line_is_not_ready_is_refused_and_asked_to_quit() {
    let runs = json!([
        [{"emit": {"type": "heard", "dest": D2, "name": "early"}}],
        [ready(D1, &pk_hex()), link("up")]
    ]);
    let hp = spawn(runs);
    hp.live().await;
    let a = &hp.adapter;
    assert_eq!(a.refused(), 1);
    assert_eq!(
        next(a).await,
        Inbound::Link(LinkState::Up),
        "nothing of the refused run surfaced"
    );
    // the refused run got only the quit; the ready run that followed got its announce
    // (§6 (3): one per run that passed the ready check)
    assert!(until(LIMIT, || hp.all_lines().len() >= 2).await);
    assert_eq!(
        hp.all_lines(),
        vec![r#"{"type":"quit"}"#.to_string(), ANNOUNCE.to_string()]
    );
    hp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn partitioned_is_an_error_state_and_fails_sends() {
    let runs = json!([[
        ready(D1, &pk_hex()),
        link("up"),
        {"expect": "send", "as": "s1"},
        link("partitioned")
    ]]);
    let hp = spawn(runs);
    hp.live().await;
    let a = &hp.adapter;
    assert_eq!(next(a).await, Inbound::Link(LinkState::Up));
    let mut s1 = send(a, D2, "x").await;
    assert_eq!(next(a).await, Inbound::Link(LinkState::Partitioned));
    assert_eq!(
        ev(&mut s1).await,
        Some(SendEvent::Failed {
            reason: Reason::AdapterDown,
            retry: Retry::Later
        })
    );
    assert_eq!(a.caps().state, AdapterState::Error("partitioned".into()));
    let mut s2 = send(a, D2, "after").await;
    assert_eq!(
        ev(&mut s2).await,
        Some(SendEvent::Failed {
            reason: Reason::AdapterDown,
            retry: Retry::Later
        })
    );
    hp.stop().await;
}

// ---------------------------------------------------------------- send lines

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_writes_the_exact_line_with_the_relay_custom_field() {
    let runs = json!([[
        ready(D1, &pk_hex()),
        {"expect": "send", "as": "s1"},
        {"expect": "send", "as": "s2"}
    ]]);
    let hp = spawn(runs);
    hp.live().await;
    let a = &hp.adapter;
    let _s1 = send(a, D2, "plain \"quoted\"\nnext").await;
    let relayed = "[W1AW7/M] hello from the mesh";
    let _s2 = send(a, D2, relayed).await;
    // not an LXMF DM of this identity: refused before anything is written
    for conv in [
        dm(D1, "XYZ"),
        dm(D3, D2),
        ConvRef {
            kind: ConvKind::Channel,
            ..dm(D1, D2)
        },
    ] {
        let mut rx = a.send(&conv, "no", SendOpts::default()).await.events;
        assert_eq!(
            ev(&mut rx).await,
            Some(SendEvent::Failed {
                reason: Reason::NoChannel,
                retry: Retry::Never
            })
        );
    }
    let lines = hp.wait_lines(2).await;
    assert_eq!(
        lines[0],
        format!(r#"{{"type":"send","id":"m1","to":"{D2}","text":"plain \"quoted\"\nnext"}}"#)
    );
    let mid = node111::daemon::bridge::lxmf::relay_for(relayed, &LxmfConfig::default().tags)
        .unwrap()
        .mid;
    assert_eq!(
        lines[1],
        format!(
            r#"{{"type":"send","id":"m2","to":"{D2}","text":"{relayed}","fields":{{"relay":{{"v":1,"net":"meshtastic","id":"W1AW7","mid":"{mid}"}}}}}}"#
        )
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(hp.lines().len(), 2, "the refused sends were never written");
    hp.stop().await;
}

// ---------------------------------------------------------------- logs (§15.1)

const FOREIGN: &str = "ZZFOREIGN";

fn foreign_runs() -> Value {
    let f = |s: &str| format!("{FOREIGN}-{s}");
    let msg = json!({"emit": {
        "type": "msg", "hash": h('5'), "src": SRC, "name": f("name"), "text": f("text"),
        "claim_ts": 1, "signed": false, "why_unsigned": "unverified",
        "fields": {"9": f("field"), "relay": {"v": 1, "net": "lxmf", "id": f("id/x"), "mid": "1"}}
    }});
    let mut run = vec![
        ready(D1, &pk_hex()),
        link("up"),
        msg,
        json!({"emit": {"type": "heard", "dest": D2, "name": f("heard")}}),
        json!({"raw": f("noise {")}),
        json!({"emit": {"type": f("type")}}),
        json!({"emit": {"type": "state", "id": f("id"), "state": "queued", "reason": f("reason")}}),
        json!({"expect": "send", "as": "s1"}),
        json!({"state": {"ref": "s1", "hash": h('6'), "state": "failed", "reason": "rejected"}}),
        json!({"emit": {"type": "heard", "dest": format!("{D2}{FOREIGN}"), "name": f("bad")}}),
    ];
    run.extend(malformed());
    run.push(json!({"exit": 1}));
    json!([
        [{"raw": f("first line")}],
        run,
        [ready(D1, &pk_hex()), link("up")]
    ])
}

/// The child half of [`no_foreign_text_reaches_a_log_line`]: drives the adapter through
/// foreign names, bodies, fields, noise, a refused run and a crash. A no-op unless the
/// parent runs it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn log_probe_child() {
    if std::env::var_os("LXMF_LOG_PROBE").is_none() {
        return;
    }
    let hp = spawn_with(foreign_runs(), false, |_| {});
    hp.live().await;
    let a = &hp.adapter;
    let mut rx = send(a, D2, &format!("{FOREIGN}-ours")).await;
    while ev(&mut rx).await.is_some() {}
    let mut items = 0;
    while items < 6 {
        let _ = next(a).await;
        items += 1;
    }
    assert!(a.refused() >= 1);
    assert!(a.noise() >= 4);
    a.quit().await;
    hp.stop().await;
}

#[test]
fn no_foreign_text_reaches_a_log_line() {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "log_probe_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("LXMF_LOG_PROBE", "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "the probe failed (stdout: {stdout})");
    assert!(
        stderr.contains("111 bridge: lxmf: "),
        "the probe logged nothing: {stderr}"
    );
    assert!(
        stderr.contains("first line was not ready") && stderr.contains("malformed helper line"),
        "{stderr}"
    );
    assert!(
        !stderr.contains(FOREIGN),
        "foreign text in a log line: {stderr}"
    );
    assert!(
        !stdout.contains(FOREIGN),
        "foreign text on stdout: {stdout}"
    );
}

// ---------------------------------------------------------------- the component declaration

fn sidecar_lock() -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../6-transport/sidecars/lxmf-sidecar/lxmf-sidecar-lock.txt"),
    )
    .unwrap()
}

#[test]
fn the_lock_copy_is_byte_identical_to_the_helpers_lock() {
    let copy = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/components/lxmf.lock.txt"),
    )
    .unwrap();
    assert_eq!(copy, sidecar_lock());
    let text = String::from_utf8(copy).unwrap();
    assert!(text.contains("rns==1.5.7 ") && text.contains("lxmf==1.2.1 "));
    assert!(text.contains("--hash=sha256:"));
}

#[test]
fn the_component_declaration_registers_through_the_loader() {
    // the release's embedded files, parsed with deny_unknown_fields (§2.1 rule 1)
    let c = manifest::load_embedded()
        .into_iter()
        .find(|c| c.id == "lxmf")
        .expect("lxmf registers");
    assert_eq!(c.adapter, "lxmf");
    assert_eq!(c.adapter_dir(), "lxmf");
    match &c.payload {
        Payload::Pip {
            lock,
            lock_bytes,
            venv,
        } => {
            assert_eq!(lock, "lxmf.lock.txt");
            assert_eq!(venv, "lxmf");
            assert_eq!(lock_bytes.as_deref(), Some(&sidecar_lock()[..]));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(c.requires.python, Some((3, 11)));
    assert_eq!(c.requires.service, Some(ServiceId::Transport));
    assert_eq!(c.run.program, "share/lxmf-sidecar/lxmf_sidecar.py");
    assert_eq!(
        c.run.args,
        [
            "--config-dir",
            "{bridge}/rns",
            "--identity",
            "{bridge}/identity",
            "--storage",
            "{bridge}/store"
        ]
    );
    assert!(
        c.run.ports.is_empty(),
        "the helper listens on no port (§15.7)"
    );
    assert!(matches!(c.run.health, Health::Ready { .. }));
    assert!(c.weight.cached_bytes > 5_000_000 && c.weight.cached_bytes < 7_000_000);
    assert!(c.weight.installed_bytes > 20_000_000 && c.weight.installed_bytes < 30_000_000);
    assert!(c.weight.rss_bytes > 40_000_000 && c.weight.rss_bytes < 50_000_000);
    assert_eq!(c.license.spdx, "LicenseRef-Reticulum");
    let n = &c.license.notice;
    for must in [
        "Reticulum (rns)",
        "LXMF (lxmf)",
        "Reticulum project",
        "reticulum.network",
        "not affiliated",
        "not their distributor",
        "You install them",
        &c.license.read_at,
        "https://github.com/markqvist/LXMF/blob/master/LICENSE",
    ] {
        assert!(n.contains(must), "the notice names {must:?}: {n}");
    }
    // the adapter's arguments fit the runner's extra-argument rules
    let args = node111::daemon::bridge::lxmf::run_args(
        &node111::daemon::components::consent::AnnounceRecord {
            name: Some("hub".into()),
            interval_min: 60,
        },
    );
    assert!(args.len() <= 8 && args.iter().all(|a| a.len() <= 256));
}

// ---------------------------------------------------------------- through the bridge core

/// A real fabric and a `Bridge` on it (the production path of the bridge core).
struct Core {
    _dir: tempfile::TempDir,
    handle: Arc<fabric::FabricHandle>,
    bridge: Arc<Bridge>,
}

async fn core() -> Core {
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
    Core {
        _dir: dir,
        handle,
        bridge,
    }
}

/// One observation of a row: `(state, evidence, reason)` and the send lines written so far.
type Seen = ((ForeignMessageState, Option<String>, Option<String>), usize);

/// F1 (review of C3rs), end to end through the bridge core: a send still `queued` when
/// the helper is killed stays `queued` (never `sent` "transmitted, not confirmed", no
/// native id) and the core re-hands it to the next helper run; once that run reports
/// `sent` and is killed too, the row stays `sent` "transmitted, not confirmed" and is
/// never re-sent automatically (§6 (4), §4).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn f1_a_send_queued_when_the_helper_is_killed_stays_queued_and_is_handed_again() {
    let Core {
        _dir,
        handle,
        bridge,
    } = core().await;
    let runs = json!([
        [
            ready(D1, &pk_hex()),
            link("up"),
            {"expect": "send", "as": "s1"},
            {"state": {"ref": "s1", "hash": h('7'), "state": "queued"}},
            {"sleep_ms": 400},
            {"kill": true}
        ],
        [
            {"sleep_ms": 400},
            ready(D1, &pk_hex()),
            link("up"),
            {"expect": "send", "as": "s2"},
            {"state": {"ref": "s2", "hash": h('8'), "state": "queued"}},
            {"state": {"ref": "s2", "hash": h('8'), "state": "sent"}},
            {"sleep_ms": 400},
            {"kill": true}
        ],
        [ready(D1, &pk_hex()), link("up")]
    ]);
    let hp = spawn(runs);
    hp.live().await;
    bridge
        .enable(Arc::clone(&hp.adapter) as Arc<dyn Adapter>)
        .await
        .unwrap();
    let fab = handle.fabric().unwrap().clone();
    let open = bridge.open_dm("lxmf", D2).await.unwrap();
    let conv = node111::daemon::fabric::json::parse_fc_id(open["id"].as_str().unwrap()).unwrap();
    let sent = bridge
        .send_text("lxmf", conv, "hi Bob", None)
        .await
        .unwrap();
    let id = node111::daemon::fabric::json::parse_fm_id(sent["id"].as_str().unwrap()).unwrap();
    // tick until `done` holds; every observed state must pass `ok`
    let drive = |done: &'static str| {
        let (bridge, hp_lines) = (Arc::clone(&bridge), hp.record());
        async move {
            let deadline = Instant::now() + LIMIT;
            let mut seen = Vec::new();
            loop {
                bridge.tick_once().await;
                let st = bridge.row_state(id).await.unwrap();
                let lines = std::fs::read_to_string(&hp_lines)
                    .unwrap_or_default()
                    .lines()
                    .filter(|l| *l != ANNOUNCE)
                    .count();
                seen.push((st.clone(), lines));
                let hit = match done {
                    "no path" => lines == 1 && st.1.as_deref() == Some(EV_NO_PATH),
                    "down" => lines == 1 && st.2.as_deref() == Some("adapter_down"),
                    "handed again" => lines == 2,
                    "sent" => st.0 == ForeignMessageState::Sent,
                    "unconfirmed" => st.1.as_deref() == Some(EV_UNCONFIRMED),
                    _ => unreachable!(),
                };
                if hit {
                    return (st, seen);
                }
                assert!(Instant::now() < deadline, "never {done}: {seen:?}");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    };
    let queued = |seen: &[Seen]| {
        seen.iter().all(|((s, e, _), _)| {
            *s == ForeignMessageState::Queued && e.as_deref() != Some(EV_UNCONFIRMED)
        })
    };
    // run 1: handed, the helper reports `queued` with the packed message's hash
    let (_, seen) = drive("no path").await;
    assert!(queued(&seen), "{seen:?}");
    // the helper is killed: the row stays `queued` (adapter_down), never `sent`
    let (st, seen) = drive("down").await;
    assert!(queued(&seen), "{seen:?}");
    assert_eq!(st.0, ForeignMessageState::Queued);
    let row = fab.foreign_message(id).await.unwrap();
    assert_eq!(row.state, ForeignState::Queued);
    assert_eq!(row.native_msg_id, None, "nothing was accepted");
    assert!(bridge.is_tracked(id), "the core retries it");
    // run 2: the core hands the same row again (a new send id, the same text)
    let (_, seen) = drive("handed again").await;
    // (the observation that sees the second line may already carry run 2's `sent`: the
    // tick between two reads can apply it; every earlier one must be queued)
    assert!(
        queued(&seen[..seen.len() - 1]),
        "queued until handed again: {seen:?}"
    );
    let lines = hp.lines();
    assert!(lines[0].contains(r#""id":"m1""#) && lines[1].contains(r#""id":"m2""#));
    assert!(lines.iter().all(|l| l.contains(r#""text":"hi Bob""#)));
    // run 2 reports `sent`: accepted, the hash recorded on the row
    let (st, _) = drive("sent").await;
    assert_eq!(st.1.as_deref(), Some(EV_SENT));
    let (st, _) = drive("unconfirmed").await;
    assert_eq!(st.0, ForeignMessageState::Sent, "on air: stays sent");
    let row = fab.foreign_message(id).await.unwrap();
    assert_eq!(row.state, ForeignState::Sent);
    assert_eq!(row.native_msg_id.as_deref(), Some(h('8').as_str()));
    // run 3: live again, and the aired row is never handed again automatically
    hp.live().await;
    for _ in 0..30 {
        bridge.tick_once().await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(hp.lines().len(), 2, "never re-sent: {:?}", hp.lines());
    let (s, e, _) = bridge.row_state(id).await.unwrap();
    assert_eq!(
        (s, e.as_deref()),
        (ForeignMessageState::Sent, Some(EV_UNCONFIRMED))
    );
    hp.adapter.quit().await;
    assert!(bridge.disable("lxmf").await);
    hp.stop().await;
    handle.shutdown().await;
}

/// FA2 (c), end to end through the bridge core: a send the helper parks as
/// `queued`/`peer_unannounced` is a `queued` row with the evidence "waiting for their
/// announce", handed exactly once — the core never re-hands it while the helper holds it
/// (no duplicate on the heard) — and it goes on the peer's heard. The row's `reason_code`:
/// the core's `SendEvent::Evidence` carries no reason, so it is null until the core takes a
/// reason with a queued state (recorded as the core's follow-up); `peer_unannounced` is the
/// only other value allowed here.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fa2_a_parked_send_is_a_queued_row_waiting_for_the_announce_handed_once() {
    let Core {
        _dir,
        handle,
        bridge,
    } = core().await;
    let runs = json!([[
        ready(D1, &pk_hex()),
        link("up"),
        {"expect": "send", "as": "s1"},
        {"state": {"ref": "s1", "state": "queued", "reason": "peer_unannounced"}},
        {"sleep_ms": 1500},
        {"heard_of": "s1", "name": "Bob"},
        {"state": {"ref": "s1", "hash": h('5'), "state": "queued"}},
        {"state": {"ref": "s1", "hash": h('5'), "state": "sent"}},
        {"state": {"ref": "s1", "hash": h('5'), "state": "delivered"}}
    ]]);
    let hp = spawn(runs);
    hp.live().await;
    bridge
        .enable(Arc::clone(&hp.adapter) as Arc<dyn Adapter>)
        .await
        .unwrap();
    let fab = handle.fabric().unwrap().clone();
    let open = bridge.open_dm("lxmf", D2).await.unwrap();
    let conv = node111::daemon::fabric::json::parse_fc_id(open["id"].as_str().unwrap()).unwrap();
    let sent = bridge
        .send_text("lxmf", conv, "hi Bob", None)
        .await
        .unwrap();
    let id = node111::daemon::fabric::json::parse_fm_id(sent["id"].as_str().unwrap()).unwrap();
    let deadline = Instant::now() + LIMIT;
    let mut parked_seen = 0;
    loop {
        bridge.tick_once().await;
        let (state, evidence, reason) = bridge.row_state(id).await.unwrap();
        if evidence.as_deref() == Some(EV_UNANNOUNCED) {
            assert_eq!(state, ForeignMessageState::Queued);
            assert!(
                reason.is_none() || reason.as_deref() == Some("peer_unannounced"),
                "{reason:?}"
            );
            let row = fab.foreign_message(id).await.unwrap();
            assert_eq!(row.state, ForeignState::Queued);
            assert_eq!(row.native_msg_id, None);
            parked_seen += 1;
        }
        if state == ForeignMessageState::Delivered {
            assert_eq!(evidence.as_deref(), Some(EV_DELIVERED));
            break;
        }
        assert!(Instant::now() < deadline, "never delivered: {state:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        parked_seen >= 10,
        "the parked row was seen {parked_seen} times"
    );
    let row = fab.foreign_message(id).await.unwrap();
    assert_eq!(row.state, ForeignState::Delivered);
    assert_eq!(row.native_msg_id.as_deref(), Some(h('5').as_str()));
    let lines = hp.lines();
    assert_eq!(lines.len(), 1, "handed once, never re-handed: {lines:?}");
    assert!(lines[0].contains(r#""text":"hi Bob""#));
    assert_eq!(hp.announces(), 1, "the park came inside the ready's window");
    hp.adapter.quit().await;
    assert!(bridge.disable("lxmf").await);
    hp.stop().await;
    handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registered_through_bridge_enable_its_texts_echoes_nearby_and_sends_flow() {
    let Core {
        _dir,
        handle,
        bridge,
    } = core().await;
    let nearby: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
    let sink = Arc::clone(&nearby);
    bridge.set_nearby_sink(Arc::new(move |net: &str, v: Value| {
        sink.lock().unwrap().push((net.to_string(), v))
    }));

    let text = |hash: char, src: &str, body: &str| {
        json!({"emit": {
            "type": "msg", "hash": h(hash), "src": src, "name": "Ann", "text": body,
            "claim_ts": 1700000000, "signed": true, "why_unsigned": null, "fields": {}
        }})
    };
    let runs = json!([[
        ready(D1, &pk_hex()),
        link("up"),
        text('1', SRC, "hello 111"),
        text('2', D1, "our own echo"),
        {"emit": {"type": "heard", "dest": D2, "name": "Bob"}},
        {"expect": "send", "as": "s1"},
        {"state": {"ref": "s1", "hash": h('9'), "state": "sent"}},
        {"state": {"ref": "s1", "hash": h('9'), "state": "delivered"}}
    ]]);
    let hp = spawn(runs);
    hp.live().await;
    // the registration seam: the component's enable, then Bridge::enable(adapter)
    bridge
        .enable(Arc::clone(&hp.adapter) as Arc<dyn Adapter>)
        .await
        .unwrap();
    assert_eq!(bridge.enabled(), vec!["lxmf".to_string()]);
    let fab = handle.fabric().unwrap().clone();
    let conv = fab
        .foreign_conversation_upsert("lxmf", D1, ForeignKind::Dm, &format!("dm:{SRC}"), None)
        .await
        .unwrap();
    let deadline = Instant::now() + LIMIT;
    let rows = loop {
        let rows = fab.foreign_thread(conv, None, 100).await.unwrap().messages;
        if !rows.is_empty() && !nearby.lock().unwrap().is_empty() && bridge.echoes("lxmf") >= 1 {
            break rows;
        }
        assert!(Instant::now() < deadline, "nothing ingested: {rows:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.dir, ForeignDir::In);
    assert_eq!(
        (r.sender_native.as_str(), r.sender_name.as_deref(), r.auth),
        (SRC, Some("Ann"), ForeignAuth::Signed)
    );
    assert_eq!(r.native_msg_id.as_deref(), Some(h('1').as_str()));
    assert_eq!(
        (r.body.as_str(), r.claim_ts),
        ("hello 111", Some(1_700_000_000))
    );
    assert_eq!(
        bridge.echoes("lxmf"),
        1,
        "our own destination is an account (§8 guard 2)"
    );
    {
        let got = nearby.lock().unwrap();
        assert_eq!(got[0].0, "lxmf");
        assert_eq!(got[0].1, nearby_source(D1, D2, "Bob"));
    }
    // a DM out: open, send, tick until the helper's states land on the row
    let open = bridge.open_dm("lxmf", D2).await.unwrap();
    let dm_conv = node111::daemon::fabric::json::parse_fc_id(open["id"].as_str().unwrap()).unwrap();
    let sent = bridge
        .send_text("lxmf", dm_conv, "hi Bob", None)
        .await
        .unwrap();
    let id = node111::daemon::fabric::json::parse_fm_id(sent["id"].as_str().unwrap()).unwrap();
    let deadline = Instant::now() + LIMIT;
    loop {
        bridge.tick_once().await;
        let (state, evidence, _) = bridge.row_state(id).await.unwrap();
        if state == ForeignMessageState::Delivered {
            assert_eq!(evidence.as_deref(), Some(EV_DELIVERED));
            break;
        }
        assert!(Instant::now() < deadline, "never delivered: {state:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let row = fab.foreign_message(id).await.unwrap();
    assert_eq!(row.state, ForeignState::Delivered);
    assert_eq!(row.native_msg_id.as_deref(), Some(h('9').as_str()));
    let lines = hp.lines();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with(r#"{"type":"send","id":"m1","to":""#));
    assert!(lines[0].contains(r#""text":"hi Bob""#));
    // disable: quit, then the core lets go (the component's disable stops the child)
    hp.adapter.quit().await;
    assert!(bridge.disable("lxmf").await);
    assert_eq!(bridge.recv_tasks(), 0);
    hp.stop().await;
    handle.shutdown().await;
}
