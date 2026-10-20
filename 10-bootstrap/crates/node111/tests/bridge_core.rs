//! Layer 6 B2 acceptance (`os/60-transport/README.md` §3, §4, §8, §9; `60-transport/TASKS.md`
//! row B2): the bridge core on a real fabric (a fresh `id111` identity, SQLCipher store V3)
//! with scripted `StubAdapter`s — two bridges cross-wired over stub "air" for the two-node
//! relay cases.
//!
//! - ingest: Cc + Cf stripped (U+202E, U+200D), NFC, the 64 B name cap at a UTF-8 boundary,
//!   the dedupe matrix (window + body vs key alone), own echo from a second account and from
//!   a recorded native id;
//! - relay: markers for `]`-named and `W1AW/7` boards and the `(1/2) ` prefix, two nodes
//!   split both ways with reverse edges (bounded), history never relayed, `relayed_seq` after
//!   a shed, a relayed copy never relayed, DM and `src = dst` refused, the consent round trip
//!   with live add-ons and pause/resume, the uplink switch and HAM gate on relay copies,
//!   `honour_ok_to_mqtt`, oversize refuse/split;
//! - fan-out: atomic refusal, split rows, cancel of unsent parts, adapter-state reasons, a
//!   111 target through `Fabric::send`, recipient lists;
//! - state machine: the monotone matrix, post-air NAKs, a pending row after a crash, `Later`
//!   back-off on ticks and the wake on an adapter event, user rows first;
//! - inert with no adapter (no task, no allocation), a 1,000-row flood in four batches that
//!   never blocks a concurrent send, and every view against the contract digest.

use futures_util::FutureExt;
use mesh111::store::{
    Class, DiskSpace, ForeignDir, ForeignKind, ForeignMessageRow, ForeignState, Watermarks,
};
use node111::daemon::bridge::edges::{edge_warning, Addon, EdgeOpts, EdgePatch};
use node111::daemon::bridge::fanout::FanoutRequest;
use node111::daemon::bridge::settings::{sha256_hex, RecipientListIn};
use node111::daemon::bridge::{
    AccountFacts, AdapterFacts, AdapterState, AirtimeProfile, Auth, Bridge, BridgeConfig,
    BridgeError, ChannelFacts, ConvKind, ConvRef, ForeignMessageState, Inbound, InboundText,
    LinkState, PerKind, Reason, Retry, SendEvent, SizeMode, StubAdapter, StubOutcome, Syntax,
};
use node111::daemon::dashboard::{ApiConfig, ApiState, DashboardState, SharedState};
use node111::daemon::fabric::{self, json as fjson, FabricConfig, FabricHandle};
use node111::daemon::DaemonLayout;
use serde_json::{json, Map, Value};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- a counting allocator

struct Counting;

thread_local! {
    static ALLOCS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn count_alloc() {
    let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
}

// SAFETY: every call is forwarded to the system allocator unchanged; the counter is a
// const-initialised thread-local without a destructor (no allocation on access).
unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        count_alloc();
        // SAFETY: forwarded with the caller's layout.
        unsafe { std::alloc::GlobalAlloc::alloc(&std::alloc::System, layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        // SAFETY: `ptr` came from the system allocator with this layout.
        unsafe { std::alloc::GlobalAlloc::dealloc(&std::alloc::System, ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        count_alloc();
        // SAFETY: forwarded with the caller's layout.
        unsafe { std::alloc::GlobalAlloc::alloc_zeroed(&std::alloc::System, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new: usize) -> *mut u8 {
        count_alloc();
        // SAFETY: `ptr` came from the system allocator with this layout.
        unsafe { std::alloc::GlobalAlloc::realloc(&std::alloc::System, ptr, layout, new) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocs() -> u64 {
    ALLOCS.with(|c| c.get())
}

// ---------------------------------------------------------------- contract digest

fn schemas() -> &'static Map<String, Value> {
    static S: OnceLock<Map<String, Value>> = OnceLock::new();
    S.get_or_init(|| {
        let path = format!(
            "{}/../../../40-interface/ui/test/contract.gen.ts",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(path).expect("contract.gen.ts");
        let marker = "export const schemas: Record<string, JsonSchema> = ";
        let start = text.find(marker).expect("schemas marker") + marker.len();
        let mut it = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
        match it.next() {
            Some(Ok(Value::Object(m))) => m,
            other => panic!("contract digest: {other:?}"),
        }
    })
}

/// `^…$` patterns of the contract: literals, `[…]` classes, `{n}`, `{n,m}`, `+`.
fn re_match(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat
        .strip_prefix('^')
        .and_then(|p| p.strip_suffix('$'))
        .unwrap_or_else(|| panic!("unanchored pattern {pat}"))
        .chars()
        .collect();
    type Atom = (Vec<(char, char)>, usize, usize);
    let mut atoms: Vec<Atom> = Vec::new();
    let mut i = 0;
    while i < p.len() {
        let mut ranges = Vec::new();
        if p[i] == '[' {
            i += 1;
            while p[i] != ']' {
                if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
                    ranges.push((p[i], p[i + 2]));
                    i += 3;
                } else {
                    ranges.push((p[i], p[i]));
                    i += 1;
                }
            }
        } else {
            ranges.push((p[i], p[i]));
        }
        i += 1;
        let (min, max) = match p.get(i) {
            Some('+') => {
                i += 1;
                (1, usize::MAX)
            }
            Some('{') => {
                let end = p[i..].iter().position(|c| *c == '}').unwrap() + i;
                let inner: String = p[i + 1..end].iter().collect();
                i = end + 1;
                match inner.split_once(',') {
                    Some((a, b)) => (a.parse().unwrap(), b.parse().unwrap()),
                    None => (inner.parse().unwrap(), inner.parse().unwrap()),
                }
            }
            _ => (1, 1),
        };
        atoms.push((ranges, min, max));
    }
    let s: Vec<char> = s.chars().collect();
    let mut at = 0;
    for (ranges, min, max) in atoms {
        let mut n = 0;
        while n < max && at < s.len() && ranges.iter().any(|(a, b)| s[at] >= *a && s[at] <= *b) {
            n += 1;
            at += 1;
        }
        if n < min {
            return false;
        }
    }
    at == s.len()
}

fn type_ok(v: &Value, t: &str) -> bool {
    match t {
        "string" => v.is_string(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        _ => false,
    }
}

fn validate(v: &Value, schema: &Value, path: &str, errs: &mut Vec<String>) {
    if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        let name = r.strip_prefix("#/components/schemas/").expect("local ref");
        let s = schemas()
            .get(name)
            .unwrap_or_else(|| panic!("no schema {name}"));
        return validate(v, s, path, errs);
    }
    if let Some(alts) = schema.get("oneOf").and_then(Value::as_array) {
        let ok = alts
            .iter()
            .filter(|a| {
                let mut e = Vec::new();
                validate(v, a, path, &mut e);
                e.is_empty()
            })
            .count();
        if ok != 1 {
            errs.push(format!("{path}: {ok} oneOf branches match {v}"));
        }
        return;
    }
    if let Some(t) = schema.get("type") {
        let types: Vec<&str> = match t {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if !types.iter().any(|t| type_ok(v, t)) {
            errs.push(format!("{path}: expected {types:?}, got {v}"));
            return;
        }
    }
    if let Some(e) = schema.get("enum").and_then(Value::as_array) {
        if !e.contains(v) {
            errs.push(format!("{path}: {v} not in enum"));
        }
    }
    if let Some(k) = schema.get("const") {
        if k != v {
            errs.push(format!("{path}: expected const {k}"));
        }
    }
    if let (Some(n), Some(m)) = (v.as_f64(), schema.get("minimum").and_then(Value::as_f64)) {
        if n < m {
            errs.push(format!("{path}: {n} below minimum {m}"));
        }
    }
    if let Some(s) = v.as_str() {
        if let Some(p) = schema.get("pattern").and_then(Value::as_str) {
            if !re_match(p, s) {
                errs.push(format!("{path}: {s:?} does not match {p}"));
            }
        }
        if schema.get("format").and_then(Value::as_str) == Some("date-time")
            && time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
                .is_err()
        {
            errs.push(format!("{path}: {s:?} is not RFC 3339"));
        }
    }
    if let (Some(o), Some(props)) = (
        v.as_object(),
        schema.get("properties").and_then(Value::as_object),
    ) {
        for r in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !o.contains_key(r) {
                errs.push(format!("{path}: required key {r} missing"));
            }
        }
        for (k, x) in o {
            match props.get(k) {
                Some(s) => validate(x, s, &format!("{path}.{k}"), errs),
                None => errs.push(format!("{path}: key {k} is not in the schema")),
            }
        }
    }
    if let (Some(a), Some(items)) = (v.as_array(), schema.get("items")) {
        for (i, x) in a.iter().enumerate() {
            validate(x, items, &format!("{path}[{i}]"), errs);
        }
    }
}

fn assert_schema(v: &Value, name: &str) {
    let mut errs = Vec::new();
    validate(
        v,
        &json!({ "$ref": format!("#/components/schemas/{name}") }),
        name,
        &mut errs,
    );
    assert!(errs.is_empty(), "{name}: {errs:#?}\n{v:#}");
}

// ---------------------------------------------------------------- harness

struct Node {
    api: Arc<ApiState>,
    handle: Arc<FabricHandle>,
    bridge: Arc<Bridge>,
    layout: DaemonLayout,
    _dir: tempfile::TempDir,
}

async fn node_with(f: impl FnOnce(&mut BridgeConfig)) -> Node {
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
    let mut bc = BridgeConfig::new(&layout.root);
    f(&mut bc);
    let bridge = Bridge::new(bc, Arc::clone(&handle), &api);
    Node {
        api,
        handle,
        bridge,
        layout,
        _dir: dir,
    }
}

async fn node() -> Node {
    node_with(|_| {}).await
}

impl Node {
    fn fab(&self) -> mesh111::fabric::Fabric {
        self.handle.fabric().unwrap().clone()
    }

    async fn conv(&self, r: &ConvRef) -> i64 {
        let kind = match r.kind {
            ConvKind::Channel => ForeignKind::Channel,
            ConvKind::Dm => ForeignKind::Dm,
            ConvKind::Group => ForeignKind::Group,
        };
        self.fab()
            .foreign_conversation_upsert(&r.network, &r.account, kind, &r.native_id, None)
            .await
            .unwrap()
    }

    async fn rows(&self, conv: i64) -> Vec<ForeignMessageRow> {
        self.fab()
            .foreign_thread(conv, None, 10_000)
            .await
            .unwrap()
            .messages
    }

    async fn inbound_rows(&self, conv: i64) -> Vec<ForeignMessageRow> {
        self.rows(conv)
            .await
            .into_iter()
            .filter(|m| m.dir == ForeignDir::In)
            .collect()
    }

    async fn own_rows(&self, conv: i64) -> Vec<ForeignMessageRow> {
        self.rows(conv)
            .await
            .into_iter()
            .filter(|m| m.dir == ForeignDir::Own)
            .collect()
    }

    /// Wait until `conv` holds at least `n` inbound rows.
    async fn wait_inbound(&self, conv: i64, n: usize) -> Vec<ForeignMessageRow> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let got = self.inbound_rows(conv).await;
            if got.len() >= n {
                return got;
            }
            assert!(
                Instant::now() < deadline,
                "{} of {n} inbound rows: {got:?}",
                got.len()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn ticks(&self, n: usize) {
        for _ in 0..n {
            self.bridge.tick_once().await;
        }
    }

    /// The network's state in `GET /networks`.
    async fn net_state(&self, net: &str) -> String {
        let v = self.bridge.networks().await.unwrap();
        v["networks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == net)
            .map(|n| n["state"].as_str().unwrap().to_string())
            .unwrap_or_default()
    }

    async fn wait_state(&self, net: &str, want: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.net_state(net).await != want {
            assert!(Instant::now() < deadline, "{net} never became {want}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Create an edge through the consent round trip; returns its row id.
    async fn edge(&self, src: i64, dst: i64, opts: EdgeOpts) -> i64 {
        let c = match self
            .bridge
            .create_edge(src, dst, opts, None, "loopback")
            .await
        {
            Err(BridgeError::Consent(c)) => c,
            other => panic!("consent first: {other:?}"),
        };
        let v = self
            .bridge
            .create_edge(src, dst, opts, Some(&c.hash), "loopback")
            .await
            .unwrap();
        fjson::parse_be_id(v["id"].as_str().unwrap()).unwrap()
    }

    async fn edge_row(&self, id: i64) -> mesh111::store::EdgeRow {
        self.fab().foreign_edge_get(id).await.unwrap()
    }
}

fn stub(id: &'static str, label: &str, tag: &str, account: &str) -> Arc<StubAdapter> {
    Arc::new(
        StubAdapter::new(id, StubAdapter::default_caps(label, tag)).with_facts(AdapterFacts {
            accounts: vec![AccountFacts {
                id: account.into(),
                label: None,
                licensed: false,
            }],
            region_set: Some(true),
            ..AdapterFacts::default()
        }),
    )
}

fn cref(net: &str, account: &str, kind: ConvKind, native: &str) -> ConvRef {
    ConvRef {
        network: net.into(),
        account: account.into(),
        kind,
        native_id: native.into(),
    }
}

fn text(conv: &ConvRef, sender: &str, name: &str, id: &str, body: &str) -> Inbound {
    Inbound::Text(InboundText {
        conv: conv.clone(),
        sender_native: sender.into(),
        sender_name: name.into(),
        auth: Auth::ChannelKey,
        native_msg_id: id.into(),
        body: body.into(),
        claim_ts: None,
        meta: json!({}),
    })
}

fn with_meta(i: Inbound, meta: Value) -> Inbound {
    match i {
        Inbound::Text(mut t) => {
            t.meta = meta;
            Inbound::Text(t)
        }
        other => other,
    }
}

fn texts(s: &StubAdapter) -> Vec<String> {
    s.sent().into_iter().map(|(_, t)| t).collect()
}

/// Two stub networks `stub-a` (A) and `stub-b` (B) with one channel each.
struct Pair {
    n: Node,
    a: Arc<StubAdapter>,
    b: Arc<StubAdapter>,
    ca: ConvRef,
    cb: ConvRef,
    src: i64,
    dst: i64,
}

async fn pair_with(f: impl FnOnce(&mut BridgeConfig)) -> Pair {
    let n = node_with(f).await;
    let a = stub("stub-a", "Stub A", "A", "na");
    let b = stub("stub-b", "Stub B", "B", "nb");
    n.bridge.enable(a.clone()).await.unwrap();
    n.bridge.enable(b.clone()).await.unwrap();
    let ca = cref("stub-a", "na", ConvKind::Channel, "ch:alpha");
    let cb = cref("stub-b", "nb", ConvKind::Channel, "ch:beta");
    let src = n.conv(&ca).await;
    let dst = n.conv(&cb).await;
    Pair {
        n,
        a,
        b,
        ca,
        cb,
        src,
        dst,
    }
}

async fn pair() -> Pair {
    pair_with(|_| {}).await
}

// ---------------------------------------------------------------- inert core

#[tokio::test(flavor = "current_thread")]
async fn with_no_adapter_tick_once_does_nothing_and_allocates_nothing() {
    let n = node().await;
    assert_eq!(n.bridge.recv_tasks(), 0, "no recv task");
    assert!(n.bridge.enabled().is_empty());
    // warm the lock's lazy state (if any), then count
    let _ = n.bridge.tick_once().now_or_never();
    let before = allocs();
    for _ in 0..100 {
        assert!(
            n.bridge.tick_once().now_or_never().is_some(),
            "returns at once"
        );
    }
    assert_eq!(allocs() - before, 0, "no allocation beyond the struct");
    assert_eq!(n.bridge.tick(), 0, "the tick does not even advance");
    assert_eq!(n.bridge.tracked(), 0);
    // enable → one task; disable → none
    let a = stub("stub-a", "Stub A", "A", "na");
    n.bridge.enable(a.clone()).await.unwrap();
    assert_eq!(n.bridge.recv_tasks(), 1);
    assert!(
        n.bridge.enable(a.clone()).await.is_err(),
        "once per network"
    );
    assert!(n.bridge.disable("stub-a").await);
    assert_eq!(n.bridge.recv_tasks(), 0);
    assert!(!n.bridge.disable("stub-a").await);
    n.handle.shutdown().await;
}

// ---------------------------------------------------------------- ingest

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ingest_strips_cc_and_cf_nfc_normalises_and_caps_names() {
    let p = pair().await;
    p.a.feed(Inbound::Conversation {
        conv: p.ca.clone(),
        title: "Long\u{202E}Fast\u{200D}".into(),
    });
    p.a.feed(text(
        &p.ca,
        "!a1b2c3d4",
        &"\u{20AC}".repeat(40),
        "1",
        "evil\u{202E}txet\u{200D} Jose\u{301}\r\nline two\tok\u{7}",
    ));
    let rows = p.n.wait_inbound(p.src, 1).await;
    let r = &rows[0];
    assert_eq!(r.body, "eviltxet Jos\u{e9}\nline two\tok");
    let name = r.sender_name.as_deref().unwrap();
    assert_eq!(name.len(), 63, "64 B cap at a UTF-8 boundary");
    assert_eq!(name, "\u{20AC}".repeat(21));
    assert_eq!(
        r.auth,
        mesh111::store::ForeignAuth::ChannelKey,
        "auth passes through"
    );
    assert_eq!(r.state, ForeignState::Delivered);
    let c = p.n.fab().foreign_conversation(p.src).await.unwrap();
    assert_eq!(c.title.as_deref(), Some("LongFast"));
    // a body over 64 KiB is stored cut, marked with its original size, never silently
    let big = "\u{20AC}".repeat(30_000);
    p.a.feed(text(&p.ca, "!a1b2c3d4", "W1AW", "2", &big));
    let rows = p.n.wait_inbound(p.src, 2).await;
    assert!(rows[1].body.len() <= 64 * 1024 && rows[1].body.len() > 64 * 1024 - 3);
    let v = p.n.handle.foreign_message_json(&rows[1]);
    assert_eq!(v["truncated"], 90_000);
    assert_schema(&v, "ForeignMessage");
    p.n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dedupe_matrix_window_with_body_versus_key_alone() {
    let n = node().await;
    let m = stub("stub-m", "Mesh-like", "MM", "nm");
    let l = Arc::new(
        StubAdapter::new("stub-l", StubAdapter::default_caps("LXMF-like", "LL")).with_facts(
            AdapterFacts {
                accounts: vec![AccountFacts {
                    id: "nl".into(),
                    label: None,
                    licensed: false,
                }],
                ids_globally_unique: true,
                ..AdapterFacts::default()
            },
        ),
    );
    n.bridge.enable(m.clone()).await.unwrap();
    n.bridge.enable(l.clone()).await.unwrap();
    let cm = cref("stub-m", "nm", ConvKind::Channel, "ch:m");
    let cl = cref("stub-l", "nl", ConvKind::Dm, "dm:peer");
    m.feed_many([
        text(&cm, "s1", "S1", "id1", "a"),
        text(&cm, "s1", "S1", "id1", "a"), // key + body within the window: duplicate
        text(&cm, "s1", "S1", "id1", "b"), // same 32-bit id, another body: a new text
        text(&cm, "s2", "S2", "id1", "a"), // another sender: a new text
        text(&cm, "s1", "S1", "", "a"),    // no native id: never a duplicate
        text(&cm, "s1", "S1", "", "a"),
        text(&cm, "s9", "S9", "end", "end"),
    ]);
    l.feed_many([
        text(&cl, "peer", "P", "hash1", "a"),
        text(&cl, "peer", "P", "hash1", "b"), // a globally unique id: the key alone
        text(&cl, "peer", "P", "hash2", "a"),
    ]);
    let mc = n.conv(&cm).await;
    let lc = n.conv(&cl).await;
    let rows = n.wait_inbound(mc, 6).await;
    let bodies: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| (r.sender_native.as_str(), r.body.as_str()))
        .collect();
    assert_eq!(
        bodies,
        vec![
            ("s1", "a"),
            ("s1", "b"),
            ("s2", "a"),
            ("s1", "a"),
            ("s1", "a"),
            ("s9", "end")
        ]
    );
    let rows = n.wait_inbound(lc, 2).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let rows2 = n.inbound_rows(lc).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows2.len(), 2, "hash1 twice is one row");
    n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn own_echo_from_a_second_account_or_a_recorded_native_id_is_dropped() {
    let p = pair().await;
    p.a.update_facts(|f| {
        f.accounts.push(AccountFacts {
            id: "na2".into(),
            label: Some("second board".into()),
            licensed: false,
        })
    });
    // our second board heard our first: dropped
    p.a.feed(text(&p.ca, "na2", "us", "e1", "echo"));
    p.a.feed(text(&p.ca, "na", "us", "e2", "echo"));
    p.a.feed(text(&p.ca, "ext", "Ext", "x1", "sentinel"));
    let rows = p.n.wait_inbound(p.src, 1).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].body, "sentinel");
    assert_eq!(p.n.bridge.echoes("stub-a"), 2);
    // our own send is accepted with a native id; a text carrying it is our echo
    p.n.bridge
        .send_text("stub-a", p.src, "from us", None)
        .await
        .unwrap();
    p.n.ticks(1).await;
    let own = p.n.own_rows(p.src).await;
    let native = own[0]
        .native_msg_id
        .clone()
        .expect("accepted with a native id");
    p.a.feed(text(&p.ca, "someone", "S", &native, "from us"));
    p.a.feed(text(&p.ca, "ext", "Ext", "x2", "sentinel 2"));
    let rows = p.n.wait_inbound(p.src, 2).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].body, "sentinel 2");
    assert_eq!(p.n.bridge.echoes("stub-a"), 3);
    p.n.handle.shutdown().await;
}

// ---------------------------------------------------------------- relay edges

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn markers_for_bracket_and_slash_named_boards_and_a_part_prefix() {
    let p = pair().await;
    p.n.edge(p.src, p.dst, EdgeOpts::default()).await;
    p.a.feed_many([
        text(&p.ca, "s1", "]", "1", "from a bracket board"),
        text(&p.ca, "s2", "W1AW/7", "2", "from a slash board"),
        text(
            &p.ca,
            "s3",
            "x",
            "3",
            "(1/2) [someone/B] relayed by another node",
        ),
        text(&p.ca, "s4", "y", "4", "[typed/A] by a human"),
        text(&p.ca, "s5", "z", "5", "[x/Q] an unregistered tag"),
    ]);
    p.n.wait_inbound(p.src, 5).await;
    p.n.ticks(2).await;
    assert_eq!(
        texts(&p.b),
        vec![
            "[?/A] from a bracket board".to_string(),
            "[W1AW7/A] from a slash board".to_string(),
            "[z/A] [x/Q] an unregistered tag".to_string(),
        ],
        "a text that parses as relayed is never relayed again"
    );
    // the reason is on the rows a marker kept from relaying (the UI says so)
    let rows = p.n.inbound_rows(p.src).await;
    let noted: Vec<&str> = rows
        .iter()
        .filter(|r| r.meta.contains("\"not_relayed\":\"marker\""))
        .map(|r| r.sender_native.as_str())
        .collect();
    assert_eq!(noted, vec!["s3", "s4"]);
    p.n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_nodes_split_both_ways_with_reverse_edges_do_not_loop() {
    let n1 = node().await;
    let n2 = node().await;
    let mk = |id: &'static str, label: &str, tag: &str, acct: &str| {
        let s = stub(id, label, tag, acct);
        s.update_caps(|c| {
            c.max_text_bytes = PerKind {
                channel: Some(40),
                dm: Some(40),
                group: None,
            }
        });
        s
    };
    let (a1, b1) = (
        mk("stub-a", "Stub A", "A", "n1a"),
        mk("stub-b", "Stub B", "B", "n1b"),
    );
    let (a2, b2) = (
        mk("stub-a", "Stub A", "A", "n2a"),
        mk("stub-b", "Stub B", "B", "n2b"),
    );
    StubAdapter::wire(&a1, &a2);
    StubAdapter::wire(&b1, &b2);
    for (n, a, b) in [(&n1, &a1, &b1), (&n2, &a2, &b2)] {
        n.bridge.enable(a.clone()).await.unwrap();
        n.bridge.enable(b.clone()).await.unwrap();
    }
    let split = EdgeOpts {
        honour_ok_to_mqtt: false,
        oversize: SizeMode::Split,
    };
    let mut convs = Vec::new();
    for (n, acct_a, acct_b) in [(&n1, "n1a", "n1b"), (&n2, "n2a", "n2b")] {
        let ca = n
            .conv(&cref("stub-a", acct_a, ConvKind::Channel, "ch:alpha"))
            .await;
        let cb = n
            .conv(&cref("stub-b", acct_b, ConvKind::Channel, "ch:beta"))
            .await;
        n.edge(ca, cb, split).await;
        n.edge(cb, ca, split).await;
        convs.push((ca, cb));
    }
    let long1 = "one two three four five six seven eight nine ten eleven twelve";
    let long2 = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
    a1.feed(text(
        &cref("stub-a", "n1a", ConvKind::Channel, "ch:alpha"),
        "ext1",
        "Ext",
        "e1",
        long1,
    ));
    b2.feed(text(
        &cref("stub-b", "n2b", ConvKind::Channel, "ch:beta"),
        "ext2",
        "Ext",
        "e2",
        long2,
    ));
    let total = || a1.sent_count() + b1.sent_count() + a2.sent_count() + b2.sent_count();
    let mut history = Vec::new();
    for _ in 0..16 {
        n1.ticks(1).await;
        n2.ticks(1).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        history.push(total());
    }
    let parts1 = b1.sent_count();
    let parts2 = a2.sent_count();
    assert!(
        parts1 >= 2 && parts2 >= 2,
        "both directions split: {parts1} {parts2}"
    );
    assert_eq!(
        a1.sent_count(),
        0,
        "node 1 never relays the copies it hears back"
    );
    assert_eq!(
        b2.sent_count(),
        0,
        "node 2 never relays the copies it hears"
    );
    assert_eq!(total(), parts1 + parts2, "bounded: {history:?}");
    assert!(
        history[history.len() - 1] == history[history.len() - 6],
        "settled: {history:?}"
    );
    for t in texts(&b1) {
        assert!(
            t.starts_with("[Ext/A] ("),
            "marker first on every part: {t:?}"
        );
        assert!(t.len() <= 40);
    }
    // node 2 stored every part node 1 relayed (heard on its own B channel)
    let heard = n2.wait_inbound(convs[1].1, parts1).await;
    assert!(
        heard
            .iter()
            .filter(|r| r.body.starts_with("[Ext/A]"))
            .count()
            == parts1
    );
    n1.handle.shutdown().await;
    n2.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_before_enable_is_never_relayed() {
    let p = pair().await;
    p.a.feed_many([
        text(&p.ca, "s", "S", "1", "old one"),
        text(&p.ca, "s", "S", "2", "old two"),
    ]);
    p.n.wait_inbound(p.src, 2).await;
    let e = p.n.edge(p.src, p.dst, EdgeOpts::default()).await;
    p.n.ticks(3).await;
    assert_eq!(p.b.sent_count(), 0, "history is never relayed");
    p.a.feed(text(&p.ca, "s", "S", "3", "new"));
    p.n.wait_inbound(p.src, 3).await;
    p.n.ticks(2).await;
    assert_eq!(texts(&p.b), vec!["[S/A] new".to_string()]);
    let row = p.n.edge_row(e).await;
    assert_eq!(row.relayed, 1);
    p.n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relayed_seq_holds_after_a_shed_and_a_relayed_copy_is_never_relayed() {
    let p = pair().await;
    let e = p.n.edge(p.src, p.dst, EdgeOpts::default()).await;
    let back = p.n.edge(p.dst, p.src, EdgeOpts::default()).await;
    // the destination's board is away: the copy waits in the backlog
    p.b.feed(Inbound::Link(LinkState::Down));
    p.n.wait_state("stub-b", "error").await;
    p.a.feed(text(&p.ca, "s", "S", "1", "first"));
    let origin = p.n.wait_inbound(p.src, 1).await[0].clone();
    p.n.ticks(1).await;
    let copies = p.n.own_rows(p.dst).await;
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].reason_code.as_deref(), Some("adapter_down"));
    assert_eq!(copies[0].edge_id, Some(e));
    assert_eq!(p.n.edge_row(e).await.relayed_seq, origin.local_seq);
    assert!(
        p.n.bridge.cancel(copies[0].id).await.is_err(),
        "a relay copy leaves by shed, disable or delete"
    );
    // shed it: the store's prune step 2b under the foreign shaping quota (what the
    // daemon's capacity pass does at capacity); the pinned source keeps its origin row
    p.n.fab()
        .foreign_conversation_pin(p.src, true)
        .await
        .unwrap();
    p.n.fab()
        .with_store(|s| {
            s.set_quota(Class::Foreign, Some(1)).unwrap();
            let r = s
                .prune_with(DiskSpace::UNKNOWN, Watermarks::with_high(u64::MAX / 2))
                .unwrap();
            s.set_quota(Class::Foreign, None).unwrap();
            r
        })
        .await
        .unwrap();
    assert!(p.n.own_rows(p.dst).await.is_empty(), "shed");
    let shed = p.n.edge_row(e).await;
    assert_eq!(shed.shed, 1, "counted on the edge");
    assert_eq!(shed.relayed_seq, origin.local_seq, "the mark holds");
    assert_eq!(
        p.n.inbound_rows(p.src).await.len(),
        1,
        "the origin is still stored"
    );
    p.b.feed(Inbound::Link(LinkState::Up));
    p.n.wait_state("stub-b", "live").await;
    p.n.ticks(3).await;
    assert!(p.n.own_rows(p.dst).await.is_empty());
    assert_eq!(p.b.sent_count(), 0);
    assert_eq!(p.n.edge_row(e).await.relayed_seq, origin.local_seq);
    // a new text relays once; the copy on B is never an input of B→A
    p.a.feed(text(&p.ca, "s", "S", "2", "second"));
    p.n.wait_inbound(p.src, 2).await;
    p.n.ticks(4).await;
    assert_eq!(texts(&p.b), vec!["[S/A] second".to_string()]);
    assert_eq!(p.a.sent_count(), 0, "a relayed copy is never relayed");
    assert_eq!(p.n.edge_row(back).await.relayed, 0);
    p.n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dm_and_self_endpoints_are_refused() {
    let p = pair().await;
    let dm =
        p.n.conv(&cref("stub-b", "nb", ConvKind::Dm, "dm:!peer"))
            .await;
    for (s, d) in [(p.src, dm), (dm, p.src), (p.src, p.src)] {
        match p
            .n
            .bridge
            .create_edge(s, d, EdgeOpts::default(), None, "loopback")
            .await
        {
            Err(BridgeError::BadRequest(_)) => {}
            other => panic!("refused with 400: {other:?}"),
        }
    }
    match p
        .n
        .bridge
        .create_edge(p.src, 9999, EdgeOpts::default(), None, "loopback")
        .await
    {
        Err(BridgeError::NotFound(_)) => {}
        other => panic!("404: {other:?}"),
    }
    p.n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn consent_round_trip_live_pause_and_the_gates_on_relay_copies() {
    let p = pair().await;
    p.a.update_facts(|f| {
        f.channels.push(ChannelFacts {
            account: "na".into(),
            native_id: "ch:alpha".into(),
            uplink: false,
            default_key: true,
        })
    });
    p.b.update_facts(|f| {
        f.channels.push(ChannelFacts {
            account: "nb".into(),
            native_id: "ch:beta".into(),
            uplink: false,
            default_key: false,
        })
    });
    let c = match p
        .n
        .bridge
        .create_edge(p.src, p.dst, EdgeOpts::default(), None, "loopback")
        .await
    {
        Err(BridgeError::Consent(c)) => c,
        other => panic!("{other:?}"),
    };
    let want = edge_warning(
        "Stub A channel ch:alpha",
        "Stub B channel ch:beta",
        "A",
        &[Addon::DefaultKeySource],
    );
    assert_eq!(c.warning_text, want, "the exact §8 text");
    assert!(want.starts_with(
        "Everyone in Stub A channel ch:alpha will have their messages copied to Stub B channel ch:beta."
    ));
    assert!(want.contains("anyone can read it; this copies public traffic"));
    assert_eq!(c.hash, sha256_hex(&want));
    assert_eq!(c.action, "edge_enable");
    assert_eq!(c.addons, vec!["default_key_source".to_string()]);
    assert_eq!(
        c.subject,
        format!("{}>{}", fjson::fc_id(p.src), fjson::fc_id(p.dst))
    );
    assert_schema(&serde_json::to_value(&c).unwrap(), "ConsentRequired");
    assert!(matches!(
        p.n.bridge
            .create_edge(
                p.src,
                p.dst,
                EdgeOpts::default(),
                Some("0".repeat(64).as_str()),
                "loopback"
            )
            .await,
        Err(BridgeError::Consent(_))
    ));
    let v =
        p.n.bridge
            .create_edge(p.src, p.dst, EdgeOpts::default(), Some(&c.hash), "loopback")
            .await
            .unwrap();
    assert_schema(&v, "BridgeEdge");
    assert_eq!(v["consent_hash"], c.hash.as_str());
    assert_eq!(v["enabled_by"], "loopback");
    let e = fjson::parse_be_id(v["id"].as_str().unwrap()).unwrap();
    p.a.feed(text(&p.ca, "s", "S", "1", "one"));
    p.n.wait_inbound(p.src, 1).await;
    p.n.ticks(2).await;
    assert_eq!(p.b.sent_count(), 1);

    // the destination's board turns uplink on: the edge pauses, nothing relays on a flag
    p.b.update_facts(|f| f.channels[0].uplink = true);
    p.a.feed(text(&p.ca, "s", "S", "2", "two"));
    p.n.wait_inbound(p.src, 2).await;
    p.n.ticks(2).await;
    assert_eq!(p.b.sent_count(), 1);
    let v = p.n.bridge.edge(e).await.unwrap();
    assert_eq!(v["paused"], true);
    assert_eq!(v["pause_reason"], "consent_paused");
    assert_schema(&v, "BridgeEdge");
    let resume = EdgePatch {
        paused: Some(false),
        ..EdgePatch::default()
    };
    let c2 = match p.n.bridge.patch_edge(e, &resume, "loopback").await {
        Err(BridgeError::Consent(c2)) => c2,
        other => panic!("{other:?}"),
    };
    assert_eq!(c2.action, "edge_resume");
    assert_eq!(
        c2.addons,
        vec![
            "default_key_source".to_string(),
            "uplinked_destination".to_string()
        ]
    );
    assert_ne!(c2.hash, c.hash);
    assert_eq!(
        p.n.bridge.edge_consent(e).await.unwrap().unwrap().hash,
        c2.hash
    );
    let stale = EdgePatch {
        paused: Some(false),
        accept_warning: Some(c.hash.clone()),
        ..EdgePatch::default()
    };
    assert!(matches!(
        p.n.bridge.patch_edge(e, &stale, "loopback").await,
        Err(BridgeError::Consent(_))
    ));
    let ok = EdgePatch {
        paused: Some(false),
        accept_warning: Some(c2.hash.clone()),
        ..EdgePatch::default()
    };
    let v = p.n.bridge.patch_edge(e, &ok, "loopback").await.unwrap();
    assert_eq!(v["paused"], false);
    assert_eq!(v["consent_hash"], c2.hash.as_str());
    // the copy is made, but the uplink switch gates every send on that channel
    p.n.ticks(2).await;
    assert_eq!(p.b.sent_count(), 1);
    let held = p.n.own_rows(p.dst).await;
    assert_eq!(
        held.last().unwrap().reason_code.as_deref(),
        Some("uplink_off")
    );
    let c3 = match p
        .n
        .bridge
        .set_uplink_send(p.dst, true, None, "loopback")
        .await
    {
        Err(BridgeError::Consent(c3)) => c3,
        other => panic!("{other:?}"),
    };
    assert_eq!(c3.action, "uplink_send");
    assert!(c3.warning_text.contains("published to the internet"));
    p.n.bridge
        .set_uplink_send(p.dst, true, Some(&c3.hash), "loopback")
        .await
        .unwrap();
    p.n.ticks(1).await;
    assert_eq!(texts(&p.b)[1], "[S/A] two");

    // the destination board becomes licensed: paused again; resumed; held for the HAM
    // confirmation; sent once confirmed
    p.b.update_facts(|f| f.accounts[0].licensed = true);
    p.a.feed(text(&p.ca, "s", "S", "3", "three"));
    p.n.wait_inbound(p.src, 3).await;
    p.n.ticks(2).await;
    assert_eq!(
        p.n.bridge.edge(e).await.unwrap()["pause_reason"],
        "consent_paused"
    );
    let c4 = p.n.bridge.edge_consent(e).await.unwrap().unwrap();
    assert!(c4.addons.contains(&"licensed_destination".to_string()));
    let ok = EdgePatch {
        paused: Some(false),
        accept_warning: Some(c4.hash.clone()),
        ..EdgePatch::default()
    };
    p.n.bridge.patch_edge(e, &ok, "loopback").await.unwrap();
    p.n.ticks(2).await;
    assert_eq!(p.b.sent_count(), 2);
    assert_eq!(
        p.n.own_rows(p.dst)
            .await
            .last()
            .unwrap()
            .reason_code
            .as_deref(),
        Some("ham_unconfirmed")
    );
    let c5 = match p.n.bridge.confirm_ham("nb", true, None, "loopback").await {
        Err(BridgeError::Consent(c5)) => c5,
        other => panic!("{other:?}"),
    };
    assert_eq!(c5.action, "ham_confirm");
    p.n.bridge
        .confirm_ham("nb", true, Some(&c5.hash), "loopback")
        .await
        .unwrap();
    p.n.ticks(1).await;
    assert_eq!(texts(&p.b)[2], "[S/A] three");
    // a user pause resumes on the recorded text
    let pause = EdgePatch {
        paused: Some(true),
        ..EdgePatch::default()
    };
    let v = p.n.bridge.patch_edge(e, &pause, "loopback").await.unwrap();
    assert_eq!(v["pause_reason"], "user");
    let v = p.n.bridge.patch_edge(e, &resume, "loopback").await.unwrap();
    assert_eq!(v["paused"], false);
    // honour_ok_to_mqtt / oversize change without a confirmation (not in the text; C1's
    // `Store::edge_set_options`), and the consent stays as recorded
    let change = EdgePatch {
        oversize: Some(SizeMode::Split),
        honour_ok_to_mqtt: Some(true),
        ..EdgePatch::default()
    };
    let v = p.n.bridge.patch_edge(e, &change, "loopback").await.unwrap();
    assert_eq!(
        (v["oversize"].as_str(), v["honour_ok_to_mqtt"].as_bool()),
        (Some("split"), Some(true))
    );
    assert_eq!(v["paused"], false);
    // the settings mirror the confirmations
    let s = p.n.bridge.settings().await.unwrap();
    assert_schema(&s, "BridgeSettings");
    assert!(s["ham"]
        .as_array()
        .unwrap()
        .iter()
        .any(|h| h["account"] == "nb" && h["confirmed"] == true && h["licensed"] == true));
    assert!(s["channels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["conv"] == fjson::fc_id(p.dst) && c["send"] == true && c["uplink"] == true));
    // and survive a restart of the bridge (state/bridge.json, 0600)
    let again = Bridge::new(
        BridgeConfig::new(&p.n.layout.root),
        Arc::clone(&p.n.handle),
        &p.n.api,
    );
    assert_eq!(again.recipient_lists(), vec![]);
    let mode = std::os::unix::fs::PermissionsExt::mode(
        &std::fs::metadata(p.n.layout.root.join("state/bridge.json"))
            .unwrap()
            .permissions(),
    );
    assert_eq!(mode & 0o777, 0o600);
    p.n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn honour_ok_to_mqtt_on_refuses_and_counts_off_relays() {
    let p = pair().await;
    let other =
        p.n.conv(&cref("stub-b", "nb", ConvKind::Channel, "ch:gamma"))
            .await;
    let honour =
        p.n.edge(
            p.src,
            p.dst,
            EdgeOpts {
                honour_ok_to_mqtt: true,
                oversize: SizeMode::Refuse,
            },
        )
        .await;
    let plain = p.n.edge(p.src, other, EdgeOpts::default()).await;
    p.a.feed_many([
        with_meta(
            text(&p.ca, "s", "S", "1", "stay on the mesh"),
            json!({"ok_to_mqtt": false}),
        ),
        with_meta(
            text(&p.ca, "s", "S", "2", "may leave"),
            json!({"ok_to_mqtt": true}),
        ),
        text(&p.ca, "s", "S", "3", "no bit reported"),
    ]);
    p.n.wait_inbound(p.src, 3).await;
    p.n.ticks(3).await;
    let to_dst: Vec<String> =
        p.b.sent()
            .into_iter()
            .filter(|(c, _)| c.native_id == "ch:beta")
            .map(|(_, t)| t)
            .collect();
    let to_other: Vec<String> =
        p.b.sent()
            .into_iter()
            .filter(|(c, _)| c.native_id == "ch:gamma")
            .map(|(_, t)| t)
            .collect();
    assert_eq!(to_dst, vec!["[S/A] may leave", "[S/A] no bit reported"]);
    assert_eq!(to_other.len(), 3, "off by default: everything relays");
    assert_eq!(p.n.edge_row(honour).await.refused, 1);
    assert_eq!(p.n.edge_row(plain).await.refused, 0);
    p.n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversize_refuse_counts_and_split_leads_every_part_with_the_marker() {
    let p = pair().await;
    p.b.update_caps(|c| c.max_text_bytes.channel = Some(40));
    let other =
        p.n.conv(&cref("stub-b", "nb", ConvKind::Channel, "ch:gamma"))
            .await;
    let refuse = p.n.edge(p.src, p.dst, EdgeOpts::default()).await;
    let split =
        p.n.edge(
            p.src,
            other,
            EdgeOpts {
                honour_ok_to_mqtt: false,
                oversize: SizeMode::Split,
            },
        )
        .await;
    let long = "a long text that cannot fit into forty bytes per packet at all";
    p.a.feed(text(&p.ca, "s", "W1AW", "1", long));
    p.n.wait_inbound(p.src, 1).await;
    p.n.ticks(4).await;
    assert_eq!(p.n.edge_row(refuse).await.refused, 1, "N too long to relay");
    assert!(p.n.own_rows(p.dst).await.is_empty(), "never truncated");
    let parts = p.n.own_rows(other).await;
    assert!(parts.len() >= 2);
    let n = parts.len();
    let mut joined = String::new();
    for (i, r) in parts.iter().enumerate() {
        let lead = format!("[W1AW/A] ({}/{n}) ", i + 1);
        assert!(r.body.starts_with(&lead), "{:?}", r.body);
        assert!(r.body.len() <= 40);
        assert_eq!((r.part_k, r.part_n), (i as u32 + 1, n as u32));
        joined.push_str(&r.body[lead.len()..]);
    }
    assert_eq!(joined, long);
    let e = p.n.edge_row(split).await;
    assert_eq!((e.relayed, e.refused), (1, 0), "one copy, counted once");
    assert_eq!(p.b.sent_count(), n);
    p.n.handle.shutdown().await;
}

// ---------------------------------------------------------------- fan-out

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fanout_refusal_is_atomic_split_is_one_row_per_part_and_cancel_drops_unsent_parts() {
    let p = pair_with(|c| c.sends_per_tick = 1).await;
    p.b.update_caps(|c| c.max_text_bytes.channel = Some(40));
    let body = "a text of more than forty bytes, sent to two networks";
    let req = |mode: Option<SizeMode>| -> FanoutRequest {
        serde_json::from_value(json!({
            "text": body,
            "targets": [
                {"network": "stub-a", "conv": fjson::fc_id(p.src)},
                {"network": "stub-b", "conv": fjson::fc_id(p.dst)},
            ],
            "size_mode": mode,
        }))
        .unwrap()
    };
    // preview: per-target plans, nothing written
    let pv = p.n.bridge.fanout_preview(&req(None)).await.unwrap();
    assert_schema(&pv, "FanoutPreview");
    assert_eq!(pv["ok"], false);
    assert_eq!(pv["text_bytes"], body.len());
    assert!(pv["items"][0]["plan"]["refusal"].is_null());
    assert_eq!(
        pv["items"][1]["plan"]["refusal"]["detail"],
        format!("{} B > 40 B per packet", body.len())
    );
    // refuse: 409 with the per-target report, nothing queued for any target
    match p.n.bridge.fanout(&req(None)).await {
        Err(e @ BridgeError::Refused { .. }) => {
            let b = e.problem().body();
            assert_schema(&b, "Problem");
            assert_eq!(b["code"], "oversize");
            assert_eq!(b["targets"].as_array().unwrap().len(), 1);
            assert_eq!(b["targets"][0]["target"]["conv"], fjson::fc_id(p.dst));
        }
        other => panic!("{other:?}"),
    }
    assert!(p.n.own_rows(p.src).await.is_empty() && p.n.own_rows(p.dst).await.is_empty());
    // split: one row per part, the request order kept
    let v =
        p.n.bridge
            .fanout(&req(Some(SizeMode::Split)))
            .await
            .unwrap();
    assert_schema(&v, "FanoutResult");
    let parts = p.n.own_rows(p.dst).await;
    assert!(parts.len() >= 2);
    assert_eq!(v["items"][1]["ids"].as_array().unwrap().len(), parts.len());
    assert!(parts
        .iter()
        .all(|r| r.body.len() <= 40 && r.group_id.is_some()));
    let group = fjson::parse_sg_id(v["group_id"].as_str().unwrap()).unwrap();
    let view = p.n.fab().foreign_group(group).await.unwrap();
    assert_eq!(
        fjson::send_group(&view.group, &view.rows)["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // one tick hands one row per network; cancel drops only the unsent parts
    p.n.ticks(1).await;
    let cancelled = p.n.bridge.cancel_unsent(group).await.unwrap();
    assert_eq!(cancelled.len(), parts.len() - 1, "only the unsent parts");
    let left = p.n.own_rows(p.dst).await;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].part_k, 1);
    assert_eq!(left[0].state, ForeignState::Sent);
    assert_eq!(p.n.own_rows(p.src).await[0].state, ForeignState::Sent);
    assert!(
        p.n.bridge.cancel(left[0].id).await.is_err(),
        "a handed part is not cancelled"
    );
    p.n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn adapter_state_is_not_a_call_error_rows_queue_with_the_reason() {
    let p = pair().await;
    p.b.feed(Inbound::Link(LinkState::Down));
    p.n.wait_state("stub-b", "error").await;
    p.a.update_facts(|f| f.region_set = Some(false));
    let req: FanoutRequest = serde_json::from_value(json!({
        "text": "hello",
        "targets": [
            {"network": "stub-a", "conv": fjson::fc_id(p.src)},
            {"network": "stub-b", "conv": fjson::fc_id(p.dst)},
        ],
    }))
    .unwrap();
    let pv = p.n.bridge.fanout_preview(&req).await.unwrap();
    assert_eq!(pv["ok"], true);
    let codes = |i: usize| -> Vec<String> {
        pv["items"][i]["plan"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["code"].as_str().unwrap().to_string())
            .collect()
    };
    assert!(codes(0).contains(&"region_unset".to_string()));
    assert!(codes(1).contains(&"adapter_down".to_string()));
    p.n.bridge.fanout(&req).await.unwrap();
    p.n.ticks(2).await;
    assert_eq!(p.a.sent_count() + p.b.sent_count(), 0);
    assert_eq!(
        p.n.own_rows(p.src).await[0].reason_code.as_deref(),
        Some("region_unset")
    );
    assert_eq!(
        p.n.own_rows(p.dst).await[0].reason_code.as_deref(),
        Some("adapter_down")
    );
    // one target's condition never holds the other: the region is set, A goes
    p.a.update_facts(|f| f.region_set = Some(true));
    p.n.ticks(1).await;
    assert_eq!((p.a.sent_count(), p.b.sent_count()), (1, 0));
    p.b.feed(Inbound::Link(LinkState::Up));
    p.n.wait_state("stub-b", "live").await;
    p.n.ticks(1).await;
    assert_eq!(p.b.sent_count(), 1);
    p.n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_111_target_rides_fabric_send_beside_the_foreign_rows() {
    let p = pair().await;
    let other = node().await;
    let card = other.fab().card().await.unwrap();
    let contact = p.n.fab().add_contact_card(&card.text).await.unwrap();
    let node_id = fjson::hex(&contact.node_id);
    let req: FanoutRequest = serde_json::from_value(json!({
        "text": "to everyone",
        "targets": [
            {"network": "stub-a", "conv": fjson::fc_id(p.src)},
            {"node_id": node_id},
        ],
    }))
    .unwrap();
    let v = p.n.bridge.fanout(&req).await.unwrap();
    assert_schema(&v, "FanoutResult");
    let msg = v["items"][1]["id"].as_str().unwrap();
    assert!(msg.starts_with("msg-"), "{v}");
    let id = fjson::parse_msg_id(msg).unwrap();
    let m = p.n.fab().message(id).await.unwrap();
    assert_eq!(m.text.as_deref(), Some("to everyone"));
    assert_eq!(m.peer, contact.node_id, "an ordinary sealed 111 message");
    // a NodeId that is not a contact: 404, nothing queued
    let req: FanoutRequest = serde_json::from_value(json!({
        "text": "x",
        "targets": [{"network": "stub-a", "conv": fjson::fc_id(p.src)}, {"node_id": "ab".repeat(32)}],
    }))
    .unwrap();
    assert!(matches!(
        p.n.bridge.fanout(&req).await,
        Err(BridgeError::NotFound(_))
    ));
    assert_eq!(p.n.own_rows(p.src).await.len(), 1);
    p.n.handle.shutdown().await;
    other.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ham_and_uplink_gates_on_a_direct_send_need_the_text_hash() {
    let p = pair().await;
    p.a.update_facts(|f| {
        f.accounts[0].licensed = true;
        f.licensed_caps = Some(PerKind {
            channel: None,
            dm: Some(232),
            group: None,
        });
        f.channels.push(ChannelFacts {
            account: "na".into(),
            native_id: "ch:alpha".into(),
            uplink: true,
            default_key: false,
        });
    });
    let c = match p.n.bridge.send_text("stub-a", p.src, "hi", None).await {
        Err(e @ BridgeError::Consent(_)) => {
            assert_schema(&e.problem().body(), "Problem");
            match e {
                BridgeError::Consent(c) => c,
                _ => unreachable!(),
            }
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(c.action, "ham_confirm");
    p.n.bridge
        .confirm_ham("na", true, Some(&c.hash), "loopback")
        .await
        .unwrap();
    let c = match p.n.bridge.send_text("stub-a", p.src, "hi", None).await {
        Err(BridgeError::Consent(c)) => c,
        other => panic!("{other:?}"),
    };
    assert_eq!(c.action, "uplink_send");
    assert_eq!(c.subject, fjson::fc_id(p.src));
    p.n.bridge
        .set_uplink_send(p.src, true, Some(&c.hash), "loopback")
        .await
        .unwrap();
    let v =
        p.n.bridge
            .send_text("stub-a", p.src, "hi", None)
            .await
            .unwrap();
    assert_schema(&v, "ForeignSendAccepted");
    assert!(v["plan"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["code"] == "uplink_channel"));
    // a licensed board's DM cap is the licensed mode's (232 B, not 220 B)
    let dm = p.n.bridge.open_dm("stub-a", "!a1b2c3d4").await.unwrap();
    assert_schema(&dm, "ForeignConversation");
    let dmc = fjson::parse_fc_id(dm["id"].as_str().unwrap()).unwrap();
    let pv: FanoutRequest = serde_json::from_value(json!({
        "text": "x".repeat(230),
        "targets": [{"network": "stub-a", "conv": fjson::fc_id(dmc)}],
    }))
    .unwrap();
    let pv = p.n.bridge.fanout_preview(&pv).await.unwrap();
    assert_eq!(pv["items"][0]["plan"]["cap"], 232);
    assert_eq!(pv["ok"], true);
    // withdrawing the HAM confirmation closes the gate again; it needs no hash
    p.n.bridge
        .confirm_ham("na", false, None, "loopback")
        .await
        .unwrap();
    assert!(matches!(
        p.n.bridge.send_text("stub-a", p.src, "hi", None).await,
        Err(BridgeError::Consent(_))
    ));
    // the user switching a channel off gates it even when it is not uplinked
    p.a.update_facts(|f| {
        f.accounts[0].licensed = false;
        f.channels[0].uplink = false;
    });
    p.n.bridge
        .set_uplink_send(p.src, false, None, "loopback")
        .await
        .unwrap();
    assert!(matches!(
        p.n.bridge.send_text("stub-a", p.src, "hi", None).await,
        Err(BridgeError::Consent(_))
    ));
    p.n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recipient_list_is_one_dm_per_member_and_an_edge_destination_only() {
    let n = node().await;
    let a = stub("stub-a", "Stub A", "A", "na");
    let l = Arc::new(
        StubAdapter::new("stub-l", {
            let mut c = StubAdapter::default_caps("Stub L", "L");
            c.kinds = vec![ConvKind::Dm, ConvKind::Group];
            c.max_text_bytes = PerKind {
                channel: None,
                dm: Some(287),
                group: Some(287),
            };
            c.recipient_syntax = Some(Syntax {
                pattern: "^[0-9a-f]{32}$".into(),
                hint: "32 hex".into(),
            });
            c
        })
        .with_facts(AdapterFacts {
            accounts: vec![AccountFacts {
                id: "ff".repeat(16),
                label: None,
                licensed: false,
            }],
            ids_globally_unique: true,
            ..AdapterFacts::default()
        }),
    );
    n.bridge.enable(a.clone()).await.unwrap();
    n.bridge.enable(l.clone()).await.unwrap();
    let (m1, m2) = ("01".repeat(16), "02".repeat(16));
    let bad = RecipientListIn {
        id: "friends".into(),
        network: "stub-l".into(),
        name: "Friends".into(),
        members: vec!["not-an-address".into()],
    };
    assert!(matches!(
        n.bridge.set_recipient_lists(&[bad]).await,
        Err(BridgeError::BadRequest(_))
    ));
    let list = RecipientListIn {
        id: "friends".into(),
        network: "stub-l".into(),
        name: "Fri\u{202E}ends".into(),
        members: vec![m1.clone(), m2.clone(), m1.clone()],
    };
    let saved = n.bridge.set_recipient_lists(&[list]).await.unwrap();
    assert_eq!(saved[0].name, "Friends");
    assert_eq!(
        saved[0].members,
        vec![m1.clone(), m2.clone()],
        "deduplicated"
    );
    let lc = n.bridge.list_conv("stub-l", "friends").await.unwrap();
    // fan-out to the list: one DM per member
    let req: FanoutRequest = serde_json::from_value(json!({
        "text": "hello friends",
        "targets": [{"network": "stub-l", "conv": fjson::fc_id(lc)}],
    }))
    .unwrap();
    let v = n.bridge.fanout(&req).await.unwrap();
    assert_schema(&v, "FanoutResult");
    assert_eq!(v["items"].as_array().unwrap().len(), 2);
    n.ticks(1).await;
    let sent = l.sent();
    assert_eq!(sent.len(), 2);
    assert!(sent
        .iter()
        .all(|(c, t)| c.kind == ConvKind::Dm && t == "hello friends"));
    assert_eq!(sent[0].0.native_id, format!("dm:{m1}"));
    // an edge into the list: each copy is one DM per member, with the marker
    let ca = cref("stub-a", "na", ConvKind::Channel, "ch:alpha");
    let src = n.conv(&ca).await;
    assert!(matches!(
        n.bridge
            .create_edge(lc, src, EdgeOpts::default(), None, "loopback")
            .await,
        Err(BridgeError::BadRequest(_))
    ));
    n.edge(src, lc, EdgeOpts::default()).await;
    a.feed(text(&ca, "s", "S", "1", "to the list"));
    n.wait_inbound(src, 1).await;
    n.ticks(2).await;
    let sent = l.sent();
    assert_eq!(sent.len(), 4);
    assert!(sent[2..]
        .iter()
        .all(|(c, t)| c.kind == ConvKind::Dm && t == "[S/A] to the list"));
    let copy = &n.own_rows(lc).await[0];
    assert_eq!(
        copy.state,
        ForeignState::Sent,
        "the lowest state among its DMs"
    );
    assert!(copy.native_msg_id.is_some());
    n.handle.shutdown().await;
}

// ---------------------------------------------------------------- state machine

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn state_machine_is_monotone_and_post_air_naks_keep_sent() {
    let p = pair().await;
    let send = |t: &'static str| {
        let b = Arc::clone(&p.n.bridge);
        let conv = p.src;
        async move {
            let v = b.send_text("stub-a", conv, t, None).await.unwrap();
            fjson::parse_fm_id(v["id"].as_str().unwrap()).unwrap()
        }
    };
    let state = |id: i64| {
        let b = Arc::clone(&p.n.bridge);
        async move { b.row_state(id).await.unwrap() }
    };
    let one = send("one").await;
    p.n.ticks(1).await;
    assert_eq!(
        state(one).await,
        (
            ForeignMessageState::Sent,
            Some("accepted by the radio".into()),
            None
        )
    );
    p.a.script(StubOutcome::Accept {
        native_id: "n2".into(),
        evidence: vec![
            (
                ForeignMessageState::Relayed,
                "a neighbour rebroadcast it".into(),
            ),
            (ForeignMessageState::Sent, "late".into()),
        ],
    });
    let two = send("two").await;
    p.n.ticks(1).await;
    assert_eq!(
        state(two).await.0,
        ForeignMessageState::Relayed,
        "never back"
    );
    p.a.script(StubOutcome::Fail {
        reason: Reason::GotNak,
        retry: Retry::Never,
    });
    let three = send("three").await;
    p.n.ticks(1).await;
    assert_eq!(
        state(three).await,
        (ForeignMessageState::Failed, None, Some("got_nak".into()))
    );
    p.a.script(StubOutcome::AcceptThenFail {
        native_id: "n4".into(),
        evidence: vec![(ForeignMessageState::Sent, "accepted by the radio".into())],
        reason: Reason::NoRoute,
        retry: Retry::Never,
    });
    let four = send("four").await;
    p.n.ticks(1).await;
    assert_eq!(
        state(four).await,
        (
            ForeignMessageState::Sent,
            Some("transmitted, not confirmed".into()),
            Some("no_route".into())
        )
    );
    // a failure after acceptance is never re-sent automatically, even `Later`
    p.a.script(StubOutcome::AcceptThenFail {
        native_id: "n5".into(),
        evidence: vec![],
        reason: Reason::PeerUnannounced,
        retry: Retry::Later,
    });
    let five = send("five").await;
    let before = p.a.sent_count();
    p.n.ticks(6).await;
    assert_eq!(p.a.sent_count(), before + 1);
    assert_eq!(state(five).await.0, ForeignMessageState::Sent);
    // held evidence: delivered never regresses; a later delivered overrides failed
    p.a.script(StubOutcome::Hold);
    let six = send("six").await;
    p.n.ticks(1).await;
    assert!(p.a.emit_held(
        0,
        SendEvent::Accepted {
            native_id: "n6".into()
        }
    ));
    assert!(p.a.emit_held(
        0,
        SendEvent::Evidence {
            state: ForeignMessageState::Delivered,
            evidence: "the recipient confirmed".into()
        }
    ));
    p.n.ticks(1).await;
    assert!(p.a.emit_held(
        0,
        SendEvent::Evidence {
            state: ForeignMessageState::Sent,
            evidence: "late".into()
        }
    ));
    p.n.ticks(1).await;
    assert_eq!(state(six).await.0, ForeignMessageState::Delivered);
    p.a.script(StubOutcome::Hold);
    let seven = send("seven").await;
    p.n.ticks(1).await;
    assert!(p.a.emit_held(
        1,
        SendEvent::Failed {
            reason: Reason::TooLarge,
            retry: Retry::Never
        }
    ));
    p.n.ticks(1).await;
    assert_eq!(state(seven).await.0, ForeignMessageState::Failed);
    assert!(p.a.emit_held(
        1,
        SendEvent::Evidence {
            state: ForeignMessageState::Delivered,
            evidence: "the recipient confirmed".into()
        }
    ));
    p.n.ticks(1).await;
    assert_eq!(state(seven).await.0, ForeignMessageState::Delivered);
    p.n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pending_row_after_a_crash_is_never_resent_and_resend_mints_a_new_row() {
    let n = node().await;
    let a = stub("stub-a", "Stub A", "A", "na");
    n.bridge.enable(a.clone()).await.unwrap();
    let ca = cref("stub-a", "na", ConvKind::Channel, "ch:alpha");
    let conv = n.conv(&ca).await;
    a.script(StubOutcome::Hold);
    let v = n
        .bridge
        .send_text("stub-a", conv, "once", None)
        .await
        .unwrap();
    let id = fjson::parse_fm_id(v["id"].as_str().unwrap()).unwrap();
    n.ticks(1).await;
    assert_eq!(a.sent_count(), 1);
    assert!(n.fab().foreign_message(id).await.unwrap().handoff_pending);
    assert!(n.bridge.is_tracked(id));
    assert!(!n.bridge.uncertain(id).await.unwrap());
    // crash: the in-memory state is gone; a new bridge on the same store
    n.bridge.disable("stub-a").await;
    let cfg = BridgeConfig::new(&n.layout.root);
    let b2 = Bridge::new(cfg, Arc::clone(&n.handle), &n.api);
    let a2 = stub("stub-a", "Stub A", "A", "na");
    b2.enable(a2.clone()).await.unwrap();
    for _ in 0..5 {
        b2.tick_once().await;
    }
    assert_eq!(
        a2.sent_count(),
        0,
        "a pending row is never re-sent automatically"
    );
    assert!(
        b2.uncertain(id).await.unwrap(),
        "may have been sent; resend?"
    );
    assert!(
        b2.cancel(id).await.is_err(),
        "nor cancelled: it may have left"
    );
    let v = b2.resend(id).await.unwrap();
    assert_schema(&v, "ForeignSendAccepted");
    let new = fjson::parse_fm_id(v["id"].as_str().unwrap()).unwrap();
    assert_ne!(new, id, "a new row (a new packet id)");
    b2.tick_once().await;
    assert_eq!(a2.sent(), vec![(ca.clone(), "once".to_string())]);
    assert_eq!(
        b2.row_state(new).await.unwrap().0,
        ForeignMessageState::Sent
    );
    let old = n.fab().foreign_message(id).await.unwrap();
    assert_eq!(
        old.state,
        ForeignState::Queued,
        "the old row keeps its state"
    );
    assert!(
        old.meta.contains(&format!("\"resent_as\":{new}")),
        "{}",
        old.meta
    );
    assert!(
        b2.resend(new).await.is_err(),
        "a delivered-path row is not resent"
    );
    // a carrier that vanishes without an answer leaves the same uncertainty
    a2.script(StubOutcome::Silent);
    let v = b2.send_text("stub-a", conv, "silent", None).await.unwrap();
    let silent = fjson::parse_fm_id(v["id"].as_str().unwrap()).unwrap();
    for _ in 0..4 {
        b2.tick_once().await;
    }
    assert_eq!(a2.sent_count(), 2, "handed once");
    assert!(b2.uncertain(silent).await.unwrap());
    n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn later_retries_back_off_on_ticks_and_wake_on_an_adapter_event() {
    let n = node_with(|c| c.backoff_cap_ticks = 8).await;
    let a = stub("stub-a", "Stub A", "A", "na");
    n.bridge.enable(a.clone()).await.unwrap();
    let ca = cref("stub-a", "na", ConvKind::Channel, "ch:alpha");
    let conv = n.conv(&ca).await;
    for _ in 0..6 {
        a.script(StubOutcome::Fail {
            reason: Reason::PeerUnannounced,
            retry: Retry::Later,
        });
    }
    let v = n
        .bridge
        .send_text("stub-a", conv, "retry me", None)
        .await
        .unwrap();
    let id = fjson::parse_fm_id(v["id"].as_str().unwrap()).unwrap();
    let mut counts = Vec::new();
    for _ in 0..10 {
        n.ticks(1).await;
        counts.push(a.sent_count());
    }
    assert_eq!(
        counts,
        vec![1, 2, 2, 3, 3, 3, 3, 4, 4, 4],
        "1, 2, 4, 8 ticks apart"
    );
    assert_eq!(
        n.bridge.row_state(id).await.unwrap(),
        (
            ForeignMessageState::Queued,
            None,
            Some("peer_unannounced".into())
        )
    );
    // an adapter event (link up) retries at once
    a.feed(Inbound::Link(LinkState::Down));
    n.wait_state("stub-a", "error").await;
    n.ticks(1).await;
    assert_eq!(
        a.sent_count(),
        4,
        "nothing is handed while the link is down"
    );
    a.feed(Inbound::Link(LinkState::Up));
    n.wait_state("stub-a", "live").await;
    n.ticks(1).await;
    assert_eq!(a.sent_count(), 5, "woken early");
    // a heard announce (Nearby) wakes it too; the script runs out and it is accepted
    let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
    let h2 = Arc::clone(&heard);
    n.bridge
        .set_nearby_sink(Arc::new(move |net: &str, v: Value| {
            h2.lock().unwrap().push((net.to_string(), v))
        }));
    for want in [6, 7] {
        a.feed(Inbound::Nearby(json!({"id": "ab", "kind": "reticulum"})));
        let deadline = Instant::now() + Duration::from_secs(10);
        while heard.lock().unwrap().len() < want - 5 {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        n.ticks(1).await;
        assert_eq!(a.sent_count(), want);
    }
    assert_eq!(
        n.bridge.row_state(id).await.unwrap().0,
        ForeignMessageState::Sent
    );
    assert!(!n.bridge.is_tracked(id));
    assert_eq!(heard.lock().unwrap()[0].0, "stub-a");
    n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_full_queue_waits_for_the_radio_and_a_flap_wakes_it() {
    let n = node_with(|c| c.backoff_cap_ticks = 64).await;
    let a = stub("stub-a", "Stub A", "A", "na");
    n.bridge.enable(a.clone()).await.unwrap();
    let ca = cref("stub-a", "na", ConvKind::Channel, "ch:alpha");
    let conv = n.conv(&ca).await;
    a.set_queue_full(true);
    let v = n
        .bridge
        .send_text("stub-a", conv, "wait", None)
        .await
        .unwrap();
    let id = fjson::parse_fm_id(v["id"].as_str().unwrap()).unwrap();
    n.ticks(4).await;
    assert_eq!(a.sent_count(), 3, "ticks 1, 2 and 4");
    assert_eq!(
        n.bridge.row_state(id).await.unwrap(),
        (
            ForeignMessageState::Queued,
            None,
            Some("rate_limited".into())
        )
    );
    assert!(n.bridge.is_tracked(id), "waiting for its retry, not lost");
    // the queue frees; a link flap (down, up) is the adapter event that retries at once
    a.set_queue_full(false);
    // while it waits, the user switches the channel off: the retry is a send, so it is held
    n.bridge
        .set_uplink_send(conv, false, None, "loopback")
        .await
        .unwrap();
    assert_eq!(n.bridge.link("stub-a"), None);
    a.flap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while n.bridge.link("stub-a") != Some(LinkState::Up) {
        assert!(Instant::now() < deadline, "the flap is seen");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    n.ticks(1).await;
    assert_eq!(a.sent_count(), 3, "the switch gates the retry too");
    n.bridge
        .set_uplink_send(conv, true, None, "loopback")
        .await
        .unwrap();
    n.ticks(1).await;
    assert_eq!(n.bridge.tick(), 6, "before its back-off (due at tick 8)");
    assert_eq!(a.sent_count(), 4, "woken by the flap");
    assert_eq!(
        n.bridge.row_state(id).await.unwrap().0,
        ForeignMessageState::Sent
    );
    n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_rows_go_ahead_of_relay_copies() {
    let p = pair_with(|c| c.sends_per_tick = 1).await;
    p.n.edge(p.src, p.dst, EdgeOpts::default()).await;
    p.b.feed(Inbound::Link(LinkState::Down));
    p.n.wait_state("stub-b", "error").await;
    p.a.feed(text(&p.ca, "s", "S", "1", "relayed text"));
    p.n.wait_inbound(p.src, 1).await;
    p.n.ticks(1).await;
    assert_eq!(p.n.own_rows(p.dst).await.len(), 1, "the copy waits");
    p.n.bridge
        .send_text("stub-b", p.dst, "typed by a human", None)
        .await
        .unwrap();
    p.b.feed(Inbound::Link(LinkState::Up));
    p.n.wait_state("stub-b", "live").await;
    p.n.ticks(1).await;
    assert_eq!(texts(&p.b), vec!["typed by a human".to_string()]);
    p.n.ticks(1).await;
    assert_eq!(
        texts(&p.b),
        vec![
            "typed by a human".to_string(),
            "[S/A] relayed text".to_string()
        ]
    );
    let _ = &p.cb;
    p.n.handle.shutdown().await;
}

// ---------------------------------------------------------------- load

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_thousand_row_flood_ingests_in_four_batches_and_never_blocks_a_send() {
    let p = pair().await;
    let flood: Vec<Inbound> = (0..1000)
        .map(|i| {
            text(
                &p.ca,
                "flooder",
                "F",
                &format!("{i:08x}"),
                &format!("flood {i}"),
            )
        })
        .collect();
    let before = p.n.bridge.ingest_batches();
    p.a.feed_many(flood);
    let t0 = Instant::now();
    p.n.bridge
        .send_text("stub-b", p.dst, "not blocked", None)
        .await
        .unwrap();
    let send_took = t0.elapsed();
    let rows = p.n.wait_inbound(p.src, 1000).await;
    assert_eq!(rows.len(), 1000);
    assert_eq!(
        p.n.bridge.ingest_batches() - before,
        4,
        "1,000 rows in ≤ 256-row batches"
    );
    assert!(send_took < Duration::from_secs(5), "{send_took:?}");
    let net = p.n.bridge.networks().await.unwrap();
    assert_eq!(net["networks"].as_array().unwrap().len(), 6);
    p.n.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_inbound_overflow_is_counted_never_silent() {
    let n = node().await;
    let a = Arc::new(
        StubAdapter::new("stub-a", StubAdapter::default_caps("Stub A", "A")).with_inbound_cap(4),
    );
    let ca = cref("stub-a", "na", ConvKind::Channel, "ch:alpha");
    // fed before the recv task runs: 6 into a queue of 4
    a.feed_many((0..6).map(|i| text(&ca, "s", "S", &format!("{i}"), "x")));
    n.bridge.enable(a.clone()).await.unwrap();
    let conv = n.conv(&ca).await;
    n.wait_inbound(conv, 4).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while n.bridge.dropped("stub-a") < 2 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let v = n.bridge.networks().await.unwrap();
    let net = v["networks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "stub-a")
        .unwrap()
        .clone();
    assert_eq!(net["dropped"], 2);
    n.handle.shutdown().await;
}

// ---------------------------------------------------------------- views

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_view_matches_the_contract() {
    let p = pair().await;
    p.a.update_caps(|c| {
        c.airtime = Some(AirtimeProfile {
            duty_cycle_permille: 100,
            bits_per_s: 1070,
            preset: Some("LongFast".into()),
        })
    });
    p.b.set_state(AdapterState::NotSet);
    let v = p.n.bridge.networks().await.unwrap();
    let nets = v["networks"].as_array().unwrap();
    let ids: Vec<&str> = nets.iter().map(|n| n["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        [
            "meshtastic",
            "lxmf",
            "app:deltachat",
            "app:simplex",
            "stub-a",
            "stub-b"
        ]
    );
    for n in nets {
        assert_schema(n, "Network");
    }
    assert_eq!(nets[0]["state"], "not_set", "compiled in, no board bound");
    assert_eq!(nets[0]["reason_code"], "no_board");
    assert_eq!(
        nets[1]["state"], "disabled",
        "an optional component, not enabled"
    );
    assert_eq!(nets[0]["copy"]["relay_tag"], "M");
    assert_eq!(nets[0]["caps"]["max_text_bytes"]["dm"]["licensed"], 232);
    assert_eq!(
        nets[1]["caps"]["recipient_syntax"]["pattern"],
        "^[0-9a-f]{32}$"
    );
    assert_eq!(nets[4]["state"], "live");
    assert!(nets[4]["caps"]["airtime"]["rule"]
        .as_str()
        .unwrap()
        .contains("LongFast"));
    assert_eq!(nets[5]["state"], "not_set");
    // conversations, a thread, a preview with an airtime estimate
    p.a.feed(text(&p.ca, "!a1b2c3d4", "W1AW", "1", "hello"));
    p.n.wait_inbound(p.src, 1).await;
    let cs = p.n.bridge.conversations("stub-a").await.unwrap();
    for c in cs["conversations"].as_array().unwrap() {
        assert_schema(c, "ForeignConversation");
    }
    assert!(matches!(
        p.n.bridge.conversations("nope").await,
        Err(BridgeError::NotFound(_))
    ));
    let t =
        p.n.bridge
            .conversation("stub-a", p.src, None, 100)
            .await
            .unwrap();
    assert_schema(&t, "ForeignThread");
    assert_eq!(t["messages"][0]["text"], "hello");
    let req: FanoutRequest = serde_json::from_value(json!({
        "text": "x".repeat(232),
        "targets": [{"network": "stub-a", "conv": fjson::fc_id(p.src)}],
    }))
    .unwrap();
    let pv = p.n.bridge.fanout_preview(&req).await.unwrap();
    assert_schema(&pv, "FanoutPreview");
    let est = &pv["items"][0]["plan"]["airtime_estimate"];
    assert!(
        est["label"].as_str().unwrap().ends_with("(estimate)"),
        "{est}"
    );
    assert!(
        (2000..2300).contains(&est["millis"].as_u64().unwrap()),
        "{est}"
    );
    let e = p.n.edge(p.src, p.dst, EdgeOpts::default()).await;
    let edges = p.n.bridge.edges().await.unwrap();
    for x in edges["edges"].as_array().unwrap() {
        assert_schema(x, "BridgeEdge");
    }
    assert!(p.n.bridge.delete_edge(e).await.unwrap());
    assert!(matches!(
        p.n.bridge.edge(e).await,
        Err(BridgeError::NotFound(_))
    ));
    let s = p.n.bridge.settings().await.unwrap();
    assert_schema(&s, "BridgeSettings");
    assert_eq!(s["announce"]["interval_min"], 60);
    let patch: node111::daemon::bridge::settings::SettingsPatch =
        serde_json::from_value(json!({"announce": {"name": "hub", "interval_min": 0}})).unwrap();
    let s = p.n.bridge.patch_settings(&patch, "loopback").await.unwrap();
    assert_schema(&s, "BridgeSettings");
    assert_eq!(s["announce"], json!({"name": "hub", "interval_min": 0}));
    // the bridge SSE frames carry contract snapshots
    let frames: Vec<Value> =
        p.n.api
            .events
            .subscribe(Some(p.n.api.events.base()))
            .replay
            .iter()
            .filter(|f| f.event == "bridge")
            .map(|f| serde_json::from_str(&f.data).unwrap())
            .collect();
    assert!(!frames.is_empty());
    for f in &frames {
        assert_schema(f, "SseBridge");
    }
    for bad in [
        BridgeError::Oversize("312 B > 232 B per packet".into()),
        BridgeError::NotFound("conversation"),
        BridgeError::Unavailable("off".into()),
    ] {
        assert_schema(&bad.problem().body(), "Problem");
    }
    p.n.handle.shutdown().await;
}
