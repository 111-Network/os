//! Layer 7 row D11 acceptance (`os/7-relay/README.md` §3.1, §3.2, §5.9, §6.1, §6.2, §12): the
//! carrier on the forwarder, against the stdlib stub `7-relay/verify/stub-ntfy.py`.
//!
//! Real hubs (the identity handle, the fabric, the relay settings, the carrier runtime) on
//! loopback; members enrolled at each; two hubs paired through the carrier; a message from a
//! member of hub A to a member of hub B crosses the stub. The stub's request log (its stderr)
//! is read per topic: `topic(A→B)` and `topic(B→A)` tell the two hubs' requests apart.
//!
//! - nothing is built while a switch is off (no subscription, task, thread, request); turning it
//!   on builds with no request before an event; turning it off tears down;
//! - one drain, post, drain on a `Forward` event; the row commits on the far ack only; an ack is
//!   posted once; the carrier bound set on build and cleared on teardown (the forward gate);
//! - the re-post rule: never on an ordinary flush, once on "Flush now", once on a `since` miss
//!   (with the `since=all` re-read);
//! - a 413 returns the row (refused, none left); a 429 keeps it;
//! - the metered stop and the licensed refusal (no request; the routes' 409s);
//! - a frame from anyone but the pair is dropped and counted, the cursor moves past it;
//! - 5B: one held stream on `111d-ntfy-5b`, one post per `Forward` event, the far ack arriving
//!   on the stream; the thread gone after the teardown.
//!
//! Every test spawns its own stub (killed on drop) and runs alone ([`SERIAL`]): the 5B test
//! counts threads by name and the `gateway` row is a process static.

use axum::extract::connect_info::MockConnectInfo;
use id111::keyset::NodeId;
use mesh111::fabric::{PairRole, PairVia};
use node111::daemon::carrier::{ntfy, CarrierRuntime, RuntimeOptions};
use node111::daemon::dashboard::api_v1::relay7;
use node111::daemon::dashboard::{routes, ApiConfig, ApiState, DashboardState, SharedState};
use node111::daemon::fabric::{self, FabricConfig, FabricHandle};
use node111::daemon::relay::netwatch::{self, NetRead, NetWatch, Opener};
use node111::daemon::relay::pair::{self, pair_within, PairAttempts, PairPick};
use node111::daemon::relay::settings::{
    self as rs, CarrierKind, CarrierMode, ConsentKind, MeteredLink, MeteredMatch, RelaySettings,
};
use node111::daemon::relay::{self, consent, gateway, CarrierHook};
use node111::daemon::transport::netlink::LinkEvents;
use node111::daemon::DaemonLayout;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

/// One test at a time (threads counted by name; the `gateway` row is a process static).
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---------------------------------------------------------------- the stub

fn stub_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../7-relay/verify/stub-ntfy.py")
}

/// One request as the stub logged it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Req {
    method: String,
    path: String,
    status: u16,
}

impl Req {
    fn topic(&self) -> &str {
        self.path
            .trim_start_matches('/')
            .split(['/', '?'])
            .next()
            .unwrap_or("")
    }
    fn is_poll(&self, topic: &str) -> bool {
        self.method == "GET" && self.topic() == topic && self.path.contains("poll=1")
    }
    fn is_post(&self, topic: &str) -> bool {
        self.method == "POST" && self.topic() == topic
    }
}

/// A running `stub-ntfy.py`; its stderr request log read into [`Stub::log`]; killed on drop.
struct Stub {
    child: Option<Child>,
    port: u16,
    args: Vec<String>,
    log: Arc<Mutex<Vec<Req>>>,
}

impl Stub {
    fn start(args: &[&str]) -> Stub {
        let mut s = Stub {
            child: None,
            port: 0,
            args: args.iter().map(|a| a.to_string()).collect(),
            log: Arc::new(Mutex::new(Vec::new())),
        };
        s.spawn(0);
        s
    }

    fn spawn(&mut self, port: u16) {
        let mut child = Command::new("python3")
            .arg("-I")
            .arg(stub_path())
            .args(["--port", &port.to_string()])
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("python3 runs the stub");
        let mut line = String::new();
        BufReader::new(child.stdout.take().expect("stdout piped"))
            .read_line(&mut line)
            .expect("the stub prints its port");
        let bound: u16 = line.trim().parse().expect("a port number");
        assert!(
            port == 0 || bound == port,
            "a restart rebinds the same port"
        );
        let err = child.stderr.take().expect("stderr piped");
        let log = self.log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                // stub-ntfy: 127.0.0.1 "GET /t/json?poll=1&since=all HTTP/1.1" 200 -
                let Some((_, rest)) = line.split_once('"') else {
                    continue;
                };
                let Some((req, tail)) = rest.split_once('"') else {
                    continue;
                };
                let mut r = req.split_whitespace();
                let (Some(m), Some(p)) = (r.next(), r.next()) else {
                    continue;
                };
                let status = tail
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                log.lock().unwrap().push(Req {
                    method: m.into(),
                    path: p.into(),
                    status,
                });
            }
        });
        self.port = bound;
        self.child = Some(child);
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn requests(&self) -> Vec<Req> {
        self.log.lock().unwrap().clone()
    }

    fn kill(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// The process dies and its cache with it; the new one binds the same port.
    fn restart(&mut self) {
        self.kill();
        self.spawn(self.port);
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.kill();
    }
}

// ---------------------------------------------------------------- a scripted netlink watch

/// A netlink source the test fires: each send is one forced change.
struct Fired(tokio::sync::mpsc::UnboundedReceiver<()>);

impl LinkEvents for Fired {
    fn next_change(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<()>> + Send + '_>> {
        Box::pin(self.0.recv())
    }
    fn take_forced(&mut self) -> bool {
        true
    }
}

struct FiredOpener(Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<()>>>);

impl Opener for FiredOpener {
    fn open(&self, _groups: u32) -> anyhow::Result<Box<dyn LinkEvents>> {
        match self.0.lock().unwrap().take() {
            Some(rx) => Ok(Box::new(Fired(rx))),
            None => anyhow::bail!("opened once"),
        }
    }
}

struct StillNet;

impl NetRead for StillNet {
    fn routes(&self) -> Option<u64> {
        Some(1)
    }
    fn addresses(&self) -> Option<u64> {
        Some(1)
    }
}

/// A netlink watch whose changes the test fires (`tx.send(())`).
fn fired_watch() -> (&'static NetWatch, tokio::sync::mpsc::UnboundedSender<()>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let cfg = netwatch::Config {
        quiet: Duration::from_millis(30),
        max_wait: Duration::from_millis(200),
        poll_unit: Duration::from_secs(60),
        termux: false,
    };
    let w = NetWatch::new(
        cfg,
        Box::new(FiredOpener(Mutex::new(Some(rx)))),
        Arc::new(StillNet),
    );
    (Box::leak(Box::new(w)), tx)
}

// ---------------------------------------------------------------- nodes

struct Node {
    api: Arc<ApiState>,
    fabric: Arc<FabricHandle>,
    rt: Option<Arc<CarrierRuntime>>,
    port: u16,
    layout: DaemonLayout,
    id: NodeId,
    _dir: tempfile::TempDir,
}

fn test_config(layout: &DaemonLayout, hub: bool) -> FabricConfig {
    let mut c = FabricConfig::production(layout, None);
    c.listen = "127.0.0.1:0".parse().unwrap();
    c.tcp.refuse_loopback = false;
    c.tcp.backoff_min = Duration::from_millis(30);
    c.tcp.backoff_max = Duration::from_millis(200);
    c.tcp.handshake_timeout = Duration::from_secs(3);
    c.hub_default = hub;
    c.enroll_timeout = Duration::from_secs(1);
    c.housekeeping = Duration::from_millis(200);
    c
}

/// The settings of a hub whose carrier is on: the gateway, ntfy at `server`, `mode`, `metered`,
/// and the `carrier_enable` consent owed for them.
fn carrier_on(server: &str, mode: CarrierMode, metered: Vec<MeteredLink>) -> RelaySettings {
    let mut s = RelaySettings::default();
    s.set_gateway_enabled(true);
    s.set_carrier_kind(CarrierKind::Ntfy);
    s.set_ntfy_server(Some(server)).unwrap();
    s.set_carrier_mode(mode, false).unwrap();
    s.set_metered(metered).unwrap();
    let c = consent::carrier_enable_for(&s).unwrap();
    s.set_consent_hash(ConsentKind::CarrierEnable, Some(&c.hash))
        .unwrap();
    s
}

fn options(watch: Option<&'static NetWatch>, licensed: Arc<AtomicBool>) -> RuntimeOptions {
    RuntimeOptions {
        netwatch: watch,
        termux: false,
        keepalive: Duration::from_secs(1),
        connect_timeout: Duration::from_secs(3),
        request_timeout: Duration::from_secs(10),
        licensed: Arc::new(move || licensed.load(Ordering::SeqCst)),
        metered_active: Arc::new(|_m: &MeteredMatch| true),
    }
}

/// A node: `hub` (the hub role), `settings` seeded in `state/relay.json`, and, for a hub, the
/// carrier runtime with `opts` registered before `relay::start` (as the daemon runs it).
async fn node(hub: bool, settings: Option<RelaySettings>, opts: Option<RuntimeOptions>) -> Node {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    if let Some(s) = &settings {
        rs::save(&layout.state_dir(), s).unwrap();
    }
    let identity = node111::identity::generate(&layout.identity_dir()).unwrap();
    let hex = identity.node_id_hex();
    let id = NodeId::from(id111::encoding::hex_decode(&hex).unwrap());
    let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.11.15")));
    dash.write().unwrap().node_id = Some(hex);
    let api = ApiState::new(dash, ApiConfig::new(layout.clone(), false));
    let fabric = fabric::start(test_config(&layout, hub), Some(identity), &api).await;
    assert!(fabric.running(), "{:?}", fabric.down_reason());
    api.insert_ext(Arc::clone(&fabric));
    let rt = opts.map(|o| {
        let rt = CarrierRuntime::new(&api, &fabric, o);
        api.insert_ext(rt.clone() as Arc<dyn CarrierHook>);
        rt
    });
    relay::start(&api, &fabric);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = routes::app(Arc::clone(&api)).layer(MockConnectInfo(
        "127.0.0.1:40000".parse::<SocketAddr>().unwrap(),
    ));
    tokio::spawn(async move { axum::serve(listener, app).await });
    Node {
        api,
        fabric,
        rt,
        port,
        layout,
        id,
        _dir: dir,
    }
}

impl Node {
    fn rt(&self) -> &Arc<CarrierRuntime> {
        self.rt.as_ref().expect("a hub with a runtime")
    }

    fn settings(&self) -> RelaySettings {
        relay7::cell(&self.api).settings()
    }

    /// Write `s` as `PATCH /relay/settings` would and run what follows a write (the hook).
    fn write(&self, s: &RelaySettings) {
        rs::save(&self.layout.state_dir(), s).unwrap();
        relay7::cell(&self.api).replace(s.clone());
        relay::carrier_hook(&self.api).reconfigure(s);
    }

    /// `POST /relay/carrier/{what}` on the loopback dashboard: (status, body).
    fn carrier(&self, what: &str) -> (u16, Value) {
        let mut conn = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        conn.set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        let req = format!(
            "POST /api/v1/relay/carrier/{what} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\
             Content-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}"
        );
        conn.write_all(req.as_bytes()).unwrap();
        let mut raw = Vec::new();
        conn.read_to_end(&mut raw).unwrap();
        let text = String::from_utf8_lossy(&raw).into_owned();
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
        (status, serde_json::from_str(body).unwrap_or(Value::Null))
    }

    async fn forward_rows(&self) -> u64 {
        self.fabric
            .fabric()
            .unwrap()
            .with_store(|s| s.forward_stats())
            .await
            .unwrap()
            .unwrap()
            .0
    }

    async fn counters(&self) -> mesh111::fabric::Counters {
        self.fabric
            .fabric()
            .unwrap()
            .status()
            .await
            .unwrap()
            .counters
    }

    async fn reposts(&self, far: &NodeId) -> u32 {
        let st = self.fabric.fabric().unwrap().status().await.unwrap();
        st.paired
            .iter()
            .find(|v| v.hub.node_id == *far)
            .map_or(0, |v| v.reposts)
    }

    async fn idle(&self) {
        until(30, "the carrier is idle", || async {
            self.rt().idle().then_some(())
        })
        .await;
    }
}

async fn until<T, F: std::future::Future<Output = Option<T>>>(
    secs: u64,
    what: &str,
    mut f: impl FnMut() -> F,
) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

/// Enrol a member at `hub` (before any pairing pins a key there).
async fn member_of(hub: &Node) -> Node {
    let m = node(false, None, None).await;
    let at = hub.fabric.listen_addr().await.unwrap();
    let offer = hub.fabric.fabric().unwrap().offer(Some(at)).await.unwrap();
    m.fabric
        .fabric()
        .unwrap()
        .enroll(at, &offer.code, Some(&offer.card))
        .await
        .unwrap();
    m.fabric.sync_settings().await.unwrap();
    m
}

/// `a` and `b` paired through the carrier, both records whole.
async fn pair_carrier(a: &Node, b: &Node) {
    let bundle = |n: &Node| {
        let n = (Arc::clone(&n.fabric), n.layout.root.clone());
        async move {
            pair::offer(&n.0, &n.1, PairRole::Peer, PairVia::Carrier)
                .await
                .unwrap()["bundle"]["text"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let pick = |bundle: String| PairPick {
        bundle,
        role: PairRole::Peer,
        via: PairVia::Carrier,
        lan: false,
        accept_warning: None,
    };
    let at = Arc::new(PairAttempts::default());
    let bt = Arc::new(PairAttempts::default());
    let addressless = |t: String| {
        pair::PairBundle {
            address: None,
            ..pair::PairBundle::parse(&t).unwrap()
        }
        .text()
    };
    let r = pair_within(
        &a.fabric,
        &at,
        &a.layout.root,
        pick(addressless(bundle(b).await)),
        Duration::from_secs(60),
    )
    .await;
    assert!(r.is_ok(), "{r:?}");
    let r = pair_within(
        &b.fabric,
        &bt,
        &b.layout.root,
        pick(bundle(a).await),
        Duration::from_secs(60),
    )
    .await;
    assert!(r.is_ok(), "{r:?}");
    let whole = |n: &Node, far: NodeId| {
        n.fabric
            .persisted()
            .fabric
            .paired_hubs
            .iter()
            .any(|p| p.node_id == far && p.whole())
    };
    until(30, "both carrier pairs whole", || async {
        (whole(a, b.id) && whole(b, a.id)).then_some(())
    })
    .await;
}

/// `from` knows `to`'s current card (its home-hub hint).
async fn knows(from: &Node, to: &Node) {
    let card = to.fabric.fabric().unwrap().card().await.unwrap().text;
    from.fabric
        .fabric()
        .unwrap()
        .add_contact_card(&card)
        .await
        .unwrap();
}

/// The two topics: `topic(A→B)` (A posts, B drains) and `topic(B→A)`.
fn topics(a: &Node, b: &Node) -> (String, String) {
    let (sa, sb) = (a.fabric.sealer().unwrap(), b.fabric.sealer().unwrap());
    let x = sa.carrier_topic_x25519(&sb.enc_pubkey()).unwrap();
    let k = ntfy::topic_key(&x, &sa.enc_pubkey(), &sb.enc_pubkey()).unwrap();
    (ntfy::topic(&k, &a.id, &b.id), ntfy::topic(&k, &b.id, &a.id))
}

/// Two hubs paired through the carrier at `stub` (A in `mode_a`), a member at each, each member
/// knowing the other.
struct World {
    a: Node,
    b: Node,
    ma: Node,
    mb: Node,
    t_ab: String,
    t_ba: String,
    net_a: tokio::sync::mpsc::UnboundedSender<()>,
}

async fn world(stub: &Stub, mode_a: CarrierMode) -> World {
    world_with(stub, mode_a, vec![], |_| {}).await
}

/// [`world`] with A's metered rules and a last edit of A's seeded settings.
async fn world_with(
    stub: &Stub,
    mode_a: CarrierMode,
    metered_a: Vec<MeteredLink>,
    edit_a: impl FnOnce(&mut RelaySettings),
) -> World {
    world_at(stub, &stub.url(), mode_a, metered_a, edit_a).await
}

/// [`world_with`] with A's carrier at `server_a` (B's at the stub).
async fn world_at(
    stub: &Stub,
    server_a: &str,
    mode_a: CarrierMode,
    metered_a: Vec<MeteredLink>,
    edit_a: impl FnOnce(&mut RelaySettings),
) -> World {
    let (wa, net_a) = fired_watch();
    let off = Arc::new(AtomicBool::new(false));
    let mut sa = carrier_on(server_a, mode_a, metered_a);
    edit_a(&mut sa);
    let a = node(true, Some(sa), Some(options(Some(wa), off.clone()))).await;
    let b = node(
        true,
        Some(carrier_on(&stub.url(), CarrierMode::FiveA, vec![])),
        Some(options(None, off)),
    )
    .await;
    let ma = member_of(&a).await;
    let mb = member_of(&b).await;
    pair_carrier(&a, &b).await;
    knows(&ma, &mb).await;
    knows(&mb, &ma).await;
    let (t_ab, t_ba) = topics(&a, &b);
    World {
        a,
        b,
        ma,
        mb,
        t_ab,
        t_ba,
        net_a,
    }
}

async fn send(from: &Node, to: &Node, text: &str) {
    from.fabric
        .fabric()
        .unwrap()
        .send(to.id, text, None)
        .await
        .unwrap();
}

fn count(reqs: &[Req], f: impl Fn(&Req) -> bool) -> usize {
    reqs.iter().filter(|r| f(r)).count()
}

/// Threads of this process named `name`.
fn threads_named(name: &str) -> usize {
    std::fs::read_dir("/proc/self/task")
        .map(|d| {
            d.filter_map(Result::ok)
                .filter(|e| {
                    std::fs::read_to_string(e.path().join("comm"))
                        .is_ok_and(|c| c.trim_end() == name)
                })
                .count()
        })
        .unwrap_or(0)
}

/// Established TCP connections to `port` on loopback (`/proc/net/tcp`, state 01).
fn held_connections(port: u16) -> usize {
    let hex = format!(":{port:04X}");
    std::fs::read_to_string("/proc/net/tcp")
        .unwrap_or_default()
        .lines()
        .skip(1)
        .filter(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            f.len() > 3 && f[2].ends_with(&hex) && f[3] == "01"
        })
        .count()
}

fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
        .unwrap_or(0)
}

fn state_file(n: &Node) -> String {
    std::fs::read_to_string(rs::path(&n.layout.state_dir())).unwrap_or_default()
}

// ---------------------------------------------------------------- tests

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_is_built_while_off_and_no_request_before_an_event() {
    let _one = SERIAL.lock().await;
    let stub = Stub::start(&[]);
    let (watch, net) = fired_watch();
    let off = Arc::new(AtomicBool::new(false));
    let on = carrier_on(&stub.url(), CarrierMode::FiveA, vec![]);
    // every switch off but one: the gateway off; no consent; not a hub
    let mut gw_off = on.clone();
    gw_off.set_gateway_enabled(false);
    let mut no_consent = on.clone();
    no_consent
        .set_consent_hash(ConsentKind::CarrierEnable, None)
        .unwrap();
    let mut no_carrier = on.clone();
    no_carrier.set_carrier_kind(CarrierKind::None);
    let h = node(
        true,
        Some(gw_off.clone()),
        Some(options(Some(watch), off.clone())),
    )
    .await;
    for s in [&gw_off, &no_consent, &no_carrier, &RelaySettings::default()] {
        h.write(s);
        let st = h.rt().stats();
        assert!(!st.built && st.builds == 0, "{st:?}");
        assert_eq!(watch.subscribers(), 0, "no netlink subscription");
        assert!(!h.rt().check() && !h.rt().flush(), "nothing starts");
    }
    let member = node(false, Some(on.clone()), Some(options(None, off.clone()))).await;
    member.write(&on);
    assert!(
        !member.rt().stats().built,
        "never on a node without the hub role"
    );
    assert_eq!(threads_named(ntfy::THREAD_5B), 0);
    // on: built, subscribed, and still no request before an event
    h.write(&on);
    assert!(h.rt().stats().built);
    assert_eq!(watch.subscribers(), 1);
    // a no-op write keeps it as it is (idempotent, cheap)
    h.write(&on);
    h.write(&h.settings());
    assert_eq!(h.rt().stats().builds, 1);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(stub.requests().is_empty(), "{:?}", stub.requests());
    assert_eq!(h.rt().carrier_stats().unwrap().wire_bytes, 0);
    // an address change is a check: with no pair, the server's health route, once
    net.send(()).unwrap();
    until(10, "the check", || async {
        (stub.requests().len() == 1).then_some(())
    })
    .await;
    h.idle().await;
    let r = stub.requests();
    assert_eq!(
        (r[0].method.as_str(), r[0].path.as_str()),
        ("GET", "/v1/health")
    );
    // Check now through the route: started; Flush now too
    let (s, v) = h.carrier("check");
    assert_eq!((s, v["started"].as_bool()), (202, Some(true)), "{v}");
    h.idle().await;
    // off: torn down, unsubscribed; Check now starts nothing
    h.write(&gw_off);
    let st = h.rt().stats();
    assert!(!st.built && st.teardowns == 1, "{st:?}");
    until(5, "the subscription dropped", || async {
        (watch.subscribers() == 0).then_some(())
    })
    .await;
    let n = stub.requests().len();
    let _ = net.send(());
    let (s, v) = h.carrier("check");
    assert_eq!((s, v["started"].as_bool()), (202, Some(false)), "{v}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(stub.requests().len(), n);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drain_post_drain_commits_on_the_far_ack_and_posts_an_ack_once() {
    let _one = SERIAL.lock().await;
    let stub = Stub::start(&[]);
    let w = world(&stub, CarrierMode::FiveA).await;
    assert!(stub.requests().is_empty(), "nothing before an event");
    let rss_before = rss_kb();
    send(&w.ma, &w.mb, "across the carrier").await;
    // A's Forward event: drain (B→A), post (A→B), drain once, stop
    until(30, "A posted", || async {
        (count(&stub.requests(), |r| r.is_post(&w.t_ab)) == 1).then_some(())
    })
    .await;
    let rss_during = rss_kb();
    w.a.idle().await;
    let a_reqs: Vec<Req> = stub
        .requests()
        .into_iter()
        .filter(|r| r.topic() == w.t_ab || r.topic() == w.t_ba)
        .collect();
    assert_eq!(a_reqs.len(), 3, "{a_reqs:?}");
    assert!(a_reqs[0].is_poll(&w.t_ba) && a_reqs[0].path.contains("since=all"));
    assert!(a_reqs[1].is_post(&w.t_ab) && a_reqs[1].status == 200);
    assert!(a_reqs[2].is_poll(&w.t_ba));
    assert_eq!(held_connections(stub.port), 0, "5A holds no connection");
    // posted is not committed: the row waits for the far hub's ack
    assert_eq!(w.a.forward_rows().await, 1);
    // B's Flush now: B drains the put, answers the ack, drains once
    let (s, v) = w.b.carrier("flush");
    assert_eq!((s, v["started"].as_bool()), (202, Some(true)), "{v}");
    until(30, "B posted the ack", || async {
        (count(&stub.requests(), |r| r.is_post(&w.t_ba)) == 1).then_some(())
    })
    .await;
    w.b.idle().await;
    assert_eq!(w.a.forward_rows().await, 1, "still: A has not read the ack");
    // B's drain cursor is saved after the fabric took the batch (C13)
    assert!(
        w.b.settings().drain_cursor(&w.t_ab).is_some(),
        "{}",
        state_file(&w.b)
    );
    // A's check reads the ack: the row commits
    let (s, _) = w.a.carrier("check");
    assert_eq!(s, 202);
    until(30, "A committed on the ack", || async {
        (w.a.forward_rows().await == 0).then_some(())
    })
    .await;
    w.a.idle().await;
    // the ack is posted once: B's next flush (the member's delivery may owe B more frames, but
    // never the same ack) posts nothing to A while A owes nothing
    let acks_before = count(&stub.requests(), |r| r.is_post(&w.t_ba));
    w.b.rt().check();
    w.b.idle().await;
    assert_eq!(count(&stub.requests(), |r| r.is_post(&w.t_ba)), acks_before);
    // the member at B gets it
    until(30, "MB holds the message", || async {
        let th = w.mb.fabric.fabric()?.thread(w.ma.id, 10).await.ok()?;
        (!th.messages.is_empty()).then_some(())
    })
    .await;
    eprintln!(
        "D11 record: RSS of the test process {rss_before} kB before, {rss_during} kB during a \
         5A flush (four daemons in this process); held connections after it: 0"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_carrier_bound_is_set_on_build_and_cleared_on_teardown() {
    let _one = SERIAL.lock().await;
    let stub = Stub::start(&[]);
    let w = world(&stub, CarrierMode::FiveA).await;
    let on = w.a.settings();
    let mut off = on.clone();
    off.set_gateway_enabled(false);
    // torn down: the bound is None, so B is no target and nothing is held for it
    w.a.write(&off);
    tokio::time::sleep(Duration::from_millis(300)).await;
    send(&w.ma, &w.mb, "while the carrier is off").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        w.a.forward_rows().await,
        0,
        "no carrier target without the bound"
    );
    assert!(stub.requests().is_empty());
    // built again: the bound is back, a new put is held for B and posted
    w.a.write(&on);
    tokio::time::sleep(Duration::from_millis(300)).await;
    send(&w.ma, &w.mb, "with the carrier on").await;
    until(30, "a row for B, posted", || async {
        (w.a.forward_rows().await == 1 && count(&stub.requests(), |r| r.is_post(&w.t_ab)) == 1)
            .then_some(())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_re_post_rule_never_on_a_flush_once_on_flush_now_and_on_a_since_miss() {
    let _one = SERIAL.lock().await;
    let mut stub = Stub::start(&[]);
    let w = world(&stub, CarrierMode::FiveA).await;
    send(&w.ma, &w.mb, "one").await;
    until(30, "A posted", || async {
        (count(&stub.requests(), |r| r.is_post(&w.t_ab)) == 1).then_some(())
    })
    .await;
    w.a.idle().await;
    // an ordinary flush (an address change): the drain, nothing posted again
    w.net_a.send(()).unwrap();
    until(10, "the check ran", || async {
        (count(&stub.requests(), |r| r.is_poll(&w.t_ba)) == 3).then_some(())
    })
    .await;
    w.a.idle().await;
    assert_eq!(count(&stub.requests(), |r| r.is_post(&w.t_ab)), 1);
    assert_eq!(w.a.reposts(&w.b.id).await, 0);
    // Flush now: the posted, unacknowledged put once more
    let (s, _) = w.a.carrier("flush");
    assert_eq!(s, 202);
    until(30, "the re-post", || async {
        (count(&stub.requests(), |r| r.is_post(&w.t_ab)) == 2).then_some(())
    })
    .await;
    w.a.idle().await;
    assert_eq!(w.a.reposts(&w.b.id).await, 1);
    // B reads both (the second answered DUPLICATE), acks; A commits
    w.b.rt().flush();
    w.b.idle().await;
    w.a.rt().check();
    until(30, "A committed", || async {
        (w.a.forward_rows().await == 0).then_some(())
    })
    .await;
    w.a.idle().await;
    assert!(
        w.a.settings().drain_cursor(&w.t_ba).is_some(),
        "A's cursor saved"
    );
    // a second message posted; then the server loses its cache (a restart)
    send(&w.ma, &w.mb, "two").await;
    until(30, "A posted two", || async {
        (count(&stub.requests(), |r| r.is_post(&w.t_ab)) == 3).then_some(())
    })
    .await;
    w.a.idle().await;
    let asked = w.a.rt().stats().reposts_asked;
    stub.restart();
    let n = stub.requests().len();
    // A's next check: since=<id> misses (400), since=all re-reads, and the put is posted again
    w.a.rt().check();
    until(30, "the since-miss re-post", || async {
        (count(&stub.requests()[n..], |r| r.is_post(&w.t_ab)) == 1).then_some(())
    })
    .await;
    w.a.idle().await;
    let after = &stub.requests()[n..];
    assert!(
        after[0].is_poll(&w.t_ba) && after[0].status == 400,
        "{after:?}"
    );
    assert!(after[1].is_poll(&w.t_ba) && after[1].path.contains("since=all"));
    assert_eq!(w.a.rt().stats().reposts_asked, asked + 1);
    // and B gets it from the new server
    w.b.rt().flush();
    w.b.idle().await;
    w.a.rt().check();
    until(30, "two committed", || async {
        (w.a.forward_rows().await == 0).then_some(())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_413_returns_the_row_and_a_429_keeps_it() {
    let _one = SERIAL.lock().await;
    // 413: the server takes no message this large; the row has no other target: refused
    let stub = Stub::start(&["--message-limit", "64"]);
    let w = world(&stub, CarrierMode::FiveA).await;
    let refused = w.a.counters().await.forward_refused;
    send(&w.ma, &w.mb, "too large for this server").await;
    until(30, "the 413", || async {
        stub.requests()
            .iter()
            .any(|r| r.is_post(&w.t_ab) && r.status == 413)
            .then_some(())
    })
    .await;
    until(30, "refused, nothing held", || async {
        (w.a.counters().await.forward_refused == refused + 1 && w.a.forward_rows().await == 0)
            .then_some(())
    })
    .await;
    drop(w);
    drop(stub);
    // 429: one token, taken by the first drain; the post is limited and the row stays
    let stub = Stub::start(&["--burst", "1", "--replenish", "3600"]);
    let w = world(&stub, CarrierMode::FiveA).await;
    send(&w.ma, &w.mb, "limited").await;
    until(30, "the 429", || async {
        stub.requests()
            .iter()
            .any(|r| r.is_post(&w.t_ab) && r.status == 429)
            .then_some(())
    })
    .await;
    w.a.idle().await;
    assert_eq!(w.a.forward_rows().await, 1, "Retry keeps the row");
    assert_eq!(w.a.counters().await.forward_refused, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_metered_stop_and_the_licensed_refusal() {
    let _one = SERIAL.lock().await;
    let stub = Stub::start(&[]);
    // a 1 kB cap on a marked SSID (always counted: 111 cannot read the SSID)
    let metered = vec![MeteredLink {
        matcher: MeteredMatch::Ssid("Phone".into()),
        daily_cap_bytes: 1_000,
    }];
    let licensed = Arc::new(AtomicBool::new(false));
    let h = node(
        true,
        Some(carrier_on(&stub.url(), CarrierMode::FiveA, metered)),
        Some(options(None, licensed.clone())),
    )
    .await;
    assert!(h.rt().stats().built);
    let (s, v) = h.carrier("check");
    assert_eq!((s, v["started"].as_bool()), (202, Some(true)));
    until(10, "the health check", || async {
        (stub.requests().len() == 1).then_some(())
    })
    .await;
    h.idle().await;
    let mut checks = 1;
    while !h.rt().metered_cap_reached() {
        assert!(checks < 20, "the bytes are counted");
        assert!(h.rt().check());
        h.idle().await;
        checks += 1;
    }
    // the window is persisted (CLOCK_BOOTTIME seconds and the boot id; never the wall clock)
    let file: Value = serde_json::from_str(&state_file(&h)).unwrap();
    let text = file.to_string();
    assert!(text.contains("boot_id") && text.contains("at_s"), "{text}");
    // past the cap: the routes refuse, and a flush asked by the hook makes no request
    let n = stub.requests().len();
    let (s, v) = h.carrier("flush");
    assert_eq!((s, v["code"].as_str()), (409, Some("metered_cap")), "{v}");
    let (s, v) = h.carrier("check");
    assert_eq!((s, v["code"].as_str()), (409, Some("metered_cap")), "{v}");
    let refused = h.rt().stats().refused;
    assert!(h.rt().check(), "the hook takes the request ...");
    h.idle().await;
    assert_eq!(
        h.rt().stats().refused,
        refused + 1,
        "... and refuses the flush"
    );
    assert_eq!(stub.requests().len(), n, "no request past the cap");
    // the licensed board: a hard block, no request
    let h2 = node(
        true,
        Some(carrier_on(&stub.url(), CarrierMode::FiveA, vec![])),
        Some(options(None, licensed.clone())),
    )
    .await;
    licensed.store(true, Ordering::SeqCst);
    let (s, v) = h2.carrier("check");
    assert_eq!(
        (s, v["code"].as_str()),
        (409, Some("licensed_board")),
        "{v}"
    );
    assert!(h2.rt().flush());
    h2.idle().await;
    assert_eq!(h2.rt().stats().refused, 1);
    assert_eq!(stub.requests().len(), n);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_frame_from_anyone_but_the_pair_is_dropped_and_counted() {
    let _one = SERIAL.lock().await;
    let stub = Stub::start(&[]);
    let off = Arc::new(AtomicBool::new(false));
    let a = node(
        true,
        Some(carrier_on(&stub.url(), CarrierMode::FiveA, vec![])),
        Some(options(None, off.clone())),
    )
    .await;
    let b = node(
        true,
        Some(carrier_on(&stub.url(), CarrierMode::FiveA, vec![])),
        Some(options(None, off)),
    )
    .await;
    pair_carrier(&a, &b).await;
    let (_, t_ba) = topics(&a, &b);
    // someone who learnt the topic posts a batch: a frame no hub sealed, and one too short
    let forged = vec![0x5au8; 400];
    let short = vec![1u8; 3];
    let body = ntfy::encode_batch(&[&forged, &short]).unwrap();
    let agent = ureq::Agent::new_with_defaults();
    let r = agent
        .post(&format!("{}/{t_ba}", stub.url()))
        .content_type("text/plain")
        .send(body.as_str())
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let dropped = a.counters().await.dropped;
    assert!(a.rt().check());
    a.idle().await;
    assert_eq!(
        a.counters().await.dropped,
        dropped + 1,
        "failed pair_mac: dropped"
    );
    assert_eq!(a.rt().dropped(), 1, "no packet that short: dropped here");
    // the cursor moved past them (saved: the fabric took the batch)
    assert!(a.settings().drain_cursor(&t_ba).is_some());
    let n = stub.requests().len();
    assert!(a.rt().check());
    a.idle().await;
    let last = stub.requests()[n..].to_vec();
    assert_eq!(last.len(), 1);
    assert!(!last[0].path.contains("since=all"), "{last:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_b_holds_one_stream_and_posts_once_per_forward_event() {
    let _one = SERIAL.lock().await;
    let stub = Stub::start(&["--keepalive", "1"]);
    let w = world(&stub, CarrierMode::FiveB).await;
    // A holds topic(B→A) on its one named thread
    until(10, "A holds the stream", || async {
        (w.a.rt().held() == Some(w.b.id) && threads_named(ntfy::THREAD_5B) == 1).then_some(())
    })
    .await;
    until(10, "the stream is open", || async {
        stub.requests()
            .iter()
            .any(|r| r.method == "GET" && r.topic() == w.t_ba && !r.path.contains("poll=1"))
            .then_some(())
    })
    .await;
    let held = held_connections(stub.port);
    // a Forward event: one post, no poll by A
    send(&w.ma, &w.mb, "held").await;
    until(30, "A posted", || async {
        (count(&stub.requests(), |r| r.is_post(&w.t_ab)) == 1).then_some(())
    })
    .await;
    w.a.idle().await;
    assert_eq!(count(&stub.requests(), |r| r.is_poll(&w.t_ba)), 0);
    assert_eq!(w.a.forward_rows().await, 1);
    // B (5A) reads it and acks; the ack reaches A on the held stream: committed, no poll
    w.b.rt().flush();
    w.b.idle().await;
    until(30, "A committed from the stream", || async {
        (w.a.forward_rows().await == 0).then_some(())
    })
    .await;
    assert_eq!(count(&stub.requests(), |r| r.is_poll(&w.t_ba)), 0);
    assert_eq!(threads_named(ntfy::THREAD_5B), 1, "one 5B thread");
    eprintln!(
        "D11 record: 5B threads 1 ({}), held connections at the stub {held}",
        ntfy::THREAD_5B
    );
    // off: the thread ends (within the read timeout, 3 × keepalive)
    let mut off = w.a.settings();
    off.set_gateway_enabled(false);
    w.a.write(&off);
    until(10, "the 5B thread ended", || async {
        (threads_named(ntfy::THREAD_5B) == 0).then_some(())
    })
    .await;
    assert!(!w.a.rt().holding());
}

// ---------------------------------------------------------------- review fixes (D11)

/// A loopback TCP proxy to `to` that holds each new connection [`SlowProxy::delay_ms`] before
/// it connects upstream (a slow server). Each connection's threads end with it.
struct SlowProxy {
    port: u16,
    delay_ms: Arc<AtomicU64>,
    accepted: Arc<AtomicU64>,
}

impl SlowProxy {
    fn start(to: u16) -> SlowProxy {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        let delay_ms = Arc::new(AtomicU64::new(0));
        let accepted = Arc::new(AtomicU64::new(0));
        let (d, a) = (delay_ms.clone(), accepted.clone());
        std::thread::spawn(move || {
            for c in l.incoming() {
                let Ok(c) = c else { continue };
                a.fetch_add(1, Ordering::SeqCst);
                let ms = d.load(Ordering::SeqCst);
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(ms));
                    let Ok(up) = TcpStream::connect(("127.0.0.1", to)) else {
                        return;
                    };
                    let (Ok(mut c_in), Ok(mut up_out)) = (c.try_clone(), up.try_clone()) else {
                        return;
                    };
                    let t = std::thread::spawn(move || {
                        let _ = std::io::copy(&mut c_in, &mut up_out);
                        let _ = up_out.shutdown(Shutdown::Write);
                    });
                    let (mut c_out, mut up_in) = (c, up);
                    let _ = std::io::copy(&mut up_in, &mut c_out);
                    let _ = c_out.shutdown(Shutdown::Both);
                    let _ = t.join();
                });
            }
        });
        SlowProxy {
            port,
            delay_ms,
            accepted,
        }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

fn ssid_cap(cap: u64) -> Vec<MeteredLink> {
    vec![MeteredLink {
        matcher: MeteredMatch::Ssid("Phone".into()),
        daily_cap_bytes: cap,
    }]
}

/// A held stream (5B) of A on topic(B→A) is open at the stub.
async fn a_holds(stub: &Stub, w: &World) {
    until(10, "A holds the stream", || async {
        (w.a.rt().held() == Some(w.b.id) && threads_named(ntfy::THREAD_5B) == 1).then_some(())
    })
    .await;
    until(10, "the stream is open", || async {
        stub.requests()
            .iter()
            .any(|r| r.method == "GET" && r.topic() == w.t_ba && !r.path.contains("poll=1"))
            .then_some(())
    })
    .await;
}

fn streams_on(stub: &Stub, topic: &str) -> usize {
    count(&stub.requests(), |r| {
        r.method == "GET" && r.topic() == topic && !r.path.contains("poll=1")
    })
}

/// Review finding 1: the held stream is carrier traffic inside the metered budget (§12, §14
/// hard rule 4): it stops at the cap (its bytes counted though not yet saved), opens no new
/// connection past it, and is held again once a raised cap is consented.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_b_stops_at_the_metered_cap_and_holds_again_when_it_is_raised() {
    let _one = SERIAL.lock().await;
    let stub = Stub::start(&["--keepalive", "1"]);
    let w = world_with(&stub, CarrierMode::FiveB, ssid_cap(1_500), |_| {}).await;
    until(10, "A holds the stream", || async {
        (w.a.rt().held() == Some(w.b.id)).then_some(())
    })
    .await;
    // keepalives alone move bytes: the stream stops at the cap, the thread ends
    until(30, "the stream stopped at the cap", || async {
        (w.a.rt().metered_cap_reached()
            && w.a.rt().held().is_none()
            && threads_named(ntfy::THREAD_5B) == 0)
            .then_some(())
    })
    .await;
    let n = streams_on(&stub, &w.t_ba);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(streams_on(&stub, &w.t_ba), n, "no reconnect past the cap");
    assert_eq!(threads_named(ntfy::THREAD_5B), 0);
    // the cap raised and consented: the same carrier holds again
    let mut s = w.a.settings();
    s.set_metered(ssid_cap(100_000_000)).unwrap();
    let c = consent::carrier_enable_for(&s).unwrap();
    s.set_consent_hash(ConsentKind::CarrierEnable, Some(&c.hash))
        .unwrap();
    w.a.write(&s);
    until(10, "held again", || async {
        (w.a.rt().held() == Some(w.b.id)
            && threads_named(ntfy::THREAD_5B) == 1
            && streams_on(&stub, &w.t_ba) > n)
            .then_some(())
    })
    .await;
    assert_eq!(w.a.rt().stats().builds, 1, "no rebuild");
}

/// Review finding 2: a held stream's (re)connect is a check (§3.2 (b)): the row clears its error
/// when the stream is open again, with no frame and no Check now.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_b_reports_a_reconnect_as_a_check() {
    let _one = SERIAL.lock().await;
    let mut stub = Stub::start(&["--keepalive", "1"]);
    let w = world(&stub, CarrierMode::FiveB).await;
    // the `gateway` row is a process static: the members configured it last; A's settings now
    gateway::apply(&w.a.fabric, &w.a.settings());
    gateway::refresh();
    assert!(gateway::facts().configured);
    a_holds(&stub, &w).await;
    // the server restarts: the stream drops (an error), then reconnects (a check, answered)
    stub.restart();
    until(10, "the drop is recorded", || async {
        gateway::facts().error.is_some().then_some(())
    })
    .await;
    until(20, "the reconnect is a check", || async {
        let f = gateway::facts();
        (f.answered == Some(true) && f.error.is_none()).then_some(())
    })
    .await;
    assert_eq!(threads_named(ntfy::THREAD_5B), 1);
}

/// Review finding 3: the stream dropped for another pair is held again once its thread ended
/// (one held stream per carrier), not refused and forgotten.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_b_holds_the_next_pair_after_an_unpair() {
    let _one = SERIAL.lock().await;
    let stub = Stub::start(&["--keepalive", "1"]);
    let off = Arc::new(AtomicBool::new(false));
    let a = node(
        true,
        Some(carrier_on(&stub.url(), CarrierMode::FiveB, vec![])),
        Some(options(None, off.clone())),
    )
    .await;
    let b = node(
        true,
        Some(carrier_on(&stub.url(), CarrierMode::FiveA, vec![])),
        Some(options(None, off.clone())),
    )
    .await;
    let c = node(
        true,
        Some(carrier_on(&stub.url(), CarrierMode::FiveA, vec![])),
        Some(options(None, off)),
    )
    .await;
    pair_carrier(&a, &b).await;
    until(10, "A holds B", || async {
        (a.rt().held() == Some(b.id) && threads_named(ntfy::THREAD_5B) == 1).then_some(())
    })
    .await;
    pair_carrier(&a, &c).await;
    let (_, t_ca) = topics(&a, &c);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(a.rt().held(), Some(b.id), "the first pair stays held");
    pair::unpair(&a.fabric, &Arc::new(PairAttempts::default()), b.id)
        .await
        .unwrap();
    until(15, "A holds C", || async {
        (a.rt().held() == Some(c.id)
            && threads_named(ntfy::THREAD_5B) == 1
            && streams_on(&stub, &t_ca) == 1)
            .then_some(())
    })
    .await;
}

/// Review finding 4: before every flush in a round, the bytes of the pairs flushed before it
/// count (not yet saved): the cap reached by the first pair's flush stops the second.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cap_reached_by_one_pair_stops_the_next_in_the_same_round() {
    let _one = SERIAL.lock().await;
    let stub = Stub::start(&[]);
    let off = Arc::new(AtomicBool::new(false));
    let a = node(
        true,
        Some(carrier_on(&stub.url(), CarrierMode::FiveA, ssid_cap(100))),
        Some(options(None, off.clone())),
    )
    .await;
    let b = node(
        true,
        Some(carrier_on(&stub.url(), CarrierMode::FiveA, vec![])),
        Some(options(None, off.clone())),
    )
    .await;
    let c = node(
        true,
        Some(carrier_on(&stub.url(), CarrierMode::FiveA, vec![])),
        Some(options(None, off)),
    )
    .await;
    pair_carrier(&a, &b).await;
    pair_carrier(&a, &c).await;
    let (_, t_ba) = topics(&a, &b);
    let (_, t_ca) = topics(&a, &c);
    assert!(!a.rt().metered_cap_reached());
    let refused = a.rt().stats().refused;
    assert!(a.rt().check());
    a.idle().await;
    let polls = count(&stub.requests(), |r| r.is_poll(&t_ba) || r.is_poll(&t_ca));
    assert_eq!(polls, 1, "{:?}", stub.requests());
    assert_eq!(a.rt().stats().refused, refused + 1);
    assert!(a.rt().metered_cap_reached());
}

/// Review finding 5: cursors of topics no carrier pair reads are removed, so a file full of
/// dead topics never stops the live one being saved (nor the metered count).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dead_drain_cursors_are_pruned_and_never_stop_the_live_one() {
    let _one = SERIAL.lock().await;
    let stub = Stub::start(&[]);
    let w = world_with(&stub, CarrierMode::FiveA, vec![], |s| {
        for i in 0..rs::MAX_DRAIN_TOPICS {
            let t = ntfy::topic(
                &[i as u8 + 1; 32],
                &NodeId::from([1; 32]),
                &NodeId::from([2; 32]),
            );
            s.set_drain_cursor(&t, Some("abcdefABCDEF")).unwrap();
        }
    })
    .await;
    assert_eq!(w.a.settings().drain_cursors().count(), rs::MAX_DRAIN_TOPICS);
    send(&w.ma, &w.mb, "past the dead topics").await;
    until(30, "A posted", || async {
        (count(&stub.requests(), |r| r.is_post(&w.t_ab)) == 1).then_some(())
    })
    .await;
    w.a.idle().await;
    w.b.rt().flush();
    w.b.idle().await;
    w.a.rt().check();
    until(30, "A committed on the ack", || async {
        (w.a.forward_rows().await == 0).then_some(())
    })
    .await;
    w.a.idle().await;
    let s = w.a.settings();
    assert!(s.drain_cursor(&w.t_ba).is_some(), "{}", state_file(&w.a));
    assert_eq!(s.drain_cursors().count(), 1, "only the live topic is kept");
}

/// Review finding 6: switching the gateway off stops a round in flight: the pairs after the one
/// in flight are not drained, nothing more is sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_teardown_stops_the_round_in_flight() {
    let _one = SERIAL.lock().await;
    let stub = Stub::start(&[]);
    let proxy = SlowProxy::start(stub.port);
    let off = Arc::new(AtomicBool::new(false));
    let a = node(
        true,
        Some(carrier_on(&proxy.url(), CarrierMode::FiveA, vec![])),
        Some(options(None, off.clone())),
    )
    .await;
    let b = node(
        true,
        Some(carrier_on(&stub.url(), CarrierMode::FiveA, vec![])),
        Some(options(None, off.clone())),
    )
    .await;
    let c = node(
        true,
        Some(carrier_on(&stub.url(), CarrierMode::FiveA, vec![])),
        Some(options(None, off)),
    )
    .await;
    pair_carrier(&a, &b).await;
    pair_carrier(&a, &c).await;
    assert_eq!(proxy.accepted.load(Ordering::SeqCst), 0);
    // a slow server: Check now drains B, then C; B's drain is in flight when the switch goes off
    proxy.delay_ms.store(1_500, Ordering::SeqCst);
    assert!(a.rt().check());
    until(30, "A's first drain is in flight", || async {
        (proxy.accepted.load(Ordering::SeqCst) >= 1).then_some(())
    })
    .await;
    let mut gw_off = a.settings();
    gw_off.set_gateway_enabled(false);
    a.write(&gw_off);
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(
        proxy.accepted.load(Ordering::SeqCst),
        1,
        "no request after the teardown: {:?}",
        stub.requests()
    );
}

/// Review finding 7: a failed drain is no flush: the row's `last_flush_ago_s` is not reset.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_drain_records_no_flush() {
    let _one = SERIAL.lock().await;
    let mut stub = Stub::start(&[]);
    // two hubs and no member: the `gateway` row (a process static) is configured by a hub last
    let off = Arc::new(AtomicBool::new(false));
    let a = node(
        true,
        Some(carrier_on(&stub.url(), CarrierMode::FiveA, vec![])),
        Some(options(None, off.clone())),
    )
    .await;
    let b = node(
        true,
        Some(carrier_on(&stub.url(), CarrierMode::FiveA, vec![])),
        Some(options(None, off)),
    )
    .await;
    pair_carrier(&a, &b).await;
    let w = (a, b);
    stub.kill();
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    assert!(w.0.rt().check());
    w.0.idle().await;
    let f = gateway::facts();
    assert!(f.error.is_some(), "the check failed: {f:?}");
    assert!(
        f.last_flush_ago_s.is_none_or(|s| s >= 1),
        "no flush recorded: {f:?}"
    );
}

/// Review finding 8: the carrier's write of `relay.json` and of the settings cell waits for the
/// cell's write lock (a PATCH holds it), so the cell never goes back to an older snapshot.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_carrier_writes_the_settings_under_the_cells_write_lock() {
    let _one = SERIAL.lock().await;
    let stub = Stub::start(&[]);
    let off = Arc::new(AtomicBool::new(false));
    let h = node(
        true,
        Some(carrier_on(
            &stub.url(),
            CarrierMode::FiveA,
            ssid_cap(100_000_000),
        )),
        Some(options(None, off)),
    )
    .await;
    let cell = relay7::cell(&h.api);
    let held = cell.write_lock().await;
    assert!(h.rt().check());
    until(10, "the health check", || async {
        (stub.requests().len() == 1).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!state_file(&h).contains("at_s"), "{}", state_file(&h));
    assert!(!h.rt().idle(), "the round waits for the lock");
    drop(held);
    h.idle().await;
    assert!(state_file(&h).contains("at_s"), "{}", state_file(&h));
    assert_ne!(h.settings().metered_window(), &rs::MeteredWindow::default());
}

/// Review finding 9: a held frame the fabric did not take is read again (the stream held again
/// from the last commit), and the cursor is saved afterwards; nothing stays stuck.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_held_frame_not_taken_is_read_again() {
    let _one = SERIAL.lock().await;
    let stub = Stub::start(&["--keepalive", "1"]);
    let w = world(&stub, CarrierMode::FiveB).await;
    a_holds(&stub, &w).await;
    send(&w.ma, &w.mb, "held").await;
    until(30, "A posted", || async {
        (count(&stub.requests(), |r| r.is_post(&w.t_ab)) == 1).then_some(())
    })
    .await;
    w.a.idle().await;
    assert_eq!(w.a.forward_rows().await, 1);
    // the next frame A's stream reads is refused by the hand-off once
    w.a.rt().fail_next_inbound(1);
    w.b.rt().flush();
    w.b.idle().await;
    until(30, "A committed on the re-read ack", || async {
        (w.a.forward_rows().await == 0).then_some(())
    })
    .await;
    until(10, "A's cursor saved", || async {
        w.a.settings().drain_cursor(&w.t_ba).is_some().then_some(())
    })
    .await;
    until(10, "held again", || async {
        (w.a.rt().held() == Some(w.b.id) && threads_named(ntfy::THREAD_5B) == 1).then_some(())
    })
    .await;
    assert!(
        streams_on(&stub, &w.t_ba) >= 2,
        "held again from the commit"
    );
}
