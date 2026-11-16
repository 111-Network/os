//! Layer 7 row D6d acceptance (`os/7-relay/README.md` §13.1 with §4.6; contract `RelayStatus`,
//! the `relay` event): the `relay` frame follows a pairing scan, on two hubs with the real
//! identity handle and fabric on loopback (the scan, the offer and the unpair called as their
//! routes call them).
//!
//! **A test binary of its own, with one test.** The frame's binding is one per process
//! (`relay7::bind`, as `relay::start` makes it), every fabric of the process feeds it
//! (`relay::tap`), and every scan of the process calls `relay::changed()`. Beside other tests
//! (row D6d first put this test in `relay_pair`) their scans and fabric events publish frames on
//! this test's bus, so a frame waited for here could be theirs (the test would pass without the
//! code it tests: seven such frames came in one window there), and their runtimes would run this
//! test's frame builds and drop them when they end (a request queued meanwhile would be lost: the
//! test would fail with the code). Here the frame's only sources are the two hubs this test
//! drives, and every build runs on this test's runtime.
//!
//! - the half a scan records is on the frame at once, though no fabric event moves it (the
//!   scan's `relay::changed()`);
//! - no frame comes while the attempt dials an address nobody answers: nothing else moves;
//! - the attempt's end, long after its route answered, is followed by exactly one frame (the
//!   attempt's `relay::changed()`, from the one exit every end takes), and that frame repeats the
//!   last status apart from its ages: a timed-out end changes nothing the status shows (the
//!   record keeps its half), so the frame asks clients to re-read and reports no end.

use id111::keyset::NodeId;
use mesh111::fabric::{PairRole, PairVia};
use node111::daemon::dashboard::api_v1::{events, relay7};
use node111::daemon::dashboard::{ApiConfig, ApiState, DashboardState, SharedState};
use node111::daemon::fabric::settings::PairLink;
use node111::daemon::fabric::{self, FabricConfig, FabricHandle};
use node111::daemon::relay;
use node111::daemon::relay::pair::{self, PairAttempts, PairBundle, PairPick};
use node111::daemon::DaemonLayout;
use serde_json::Value;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::time::Instant;

// ---------------------------------------------------------------- hubs

struct Hub {
    api: Arc<ApiState>,
    fabric: Arc<FabricHandle>,
    attempts: Arc<PairAttempts>,
    layout: DaemonLayout,
    id: NodeId,
    _dir: tempfile::TempDir,
}

fn test_config(layout: &DaemonLayout) -> FabricConfig {
    let mut c = FabricConfig::production(layout, None);
    c.listen = "127.0.0.1:0".parse().unwrap();
    // both hubs are on loopback
    c.tcp.refuse_loopback = false;
    c.tcp.backoff_min = Duration::from_millis(30);
    c.tcp.backoff_max = Duration::from_millis(200);
    c.tcp.handshake_timeout = Duration::from_secs(3);
    c.hub_default = true;
    // the scan waits min(3 s, this) for the attempt's first answer
    c.enroll_timeout = Duration::from_secs(1);
    c.housekeeping = Duration::from_millis(200);
    c
}

/// A hub with the real identity handle and fabric (no dashboard socket: nothing here goes
/// through HTTP, and `relay::start` is not run, so no carrier runtime is built).
async fn hub() -> Hub {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let identity = node111::identity::generate(&layout.identity_dir()).unwrap();
    let hex = identity.node_id_hex();
    let id = NodeId::from(id111::encoding::hex_decode(&hex).unwrap());
    let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.11.16")));
    dash.write().unwrap().node_id = Some(hex);
    let api = ApiState::new(dash, ApiConfig::new(layout.clone(), false));
    let fabric = fabric::start(test_config(&layout), Some(identity), &api).await;
    assert!(fabric.running(), "{:?}", fabric.down_reason());
    api.insert_ext(Arc::clone(&fabric));
    Hub {
        api,
        fabric,
        attempts: Arc::new(PairAttempts::default()),
        layout,
        id,
        _dir: dir,
    }
}

impl Hub {
    /// `POST /relay/pair/offer {role: peer, via: tcp}`: the bundle text.
    async fn offer(&self) -> String {
        let v = pair::offer(
            &self.fabric,
            &self.layout.root,
            PairRole::Peer,
            PairVia::Tcp,
        )
        .await
        .unwrap_or_else(|p| panic!("{p:?}"));
        v["bundle"]["text"].as_str().unwrap().to_string()
    }

    /// `POST /relay/pair {bundle, role: peer, via: tcp, lan: true}` with the attempt's window:
    /// the route's `202` body.
    async fn scan(&self, bundle: String, window: Duration) -> Value {
        let pick = PairPick {
            bundle,
            role: PairRole::Peer,
            via: PairVia::Tcp,
            lan: true,
            accept_warning: None,
        };
        pair::pair_within(
            &self.fabric,
            &self.attempts,
            &self.layout.root,
            pick,
            window,
        )
        .await
        .unwrap_or_else(|r| panic!("{r:?}"))
    }

    /// The fabric holds a record of `far` with `theirs` (the far hub enrolled here).
    async fn holds_theirs(&self, far: &NodeId) -> bool {
        self.fabric
            .fabric()
            .unwrap()
            .settings()
            .await
            .unwrap()
            .paired_hubs
            .iter()
            .any(|p| p.node_id == *far && p.theirs)
    }

    /// This hub's half of `far`.
    fn link(&self, far: &NodeId) -> Option<PairLink> {
        let table = self.fabric.pair_table();
        let pairs = table.read().unwrap();
        pairs.iter().find(|l| l.node_id == *far).cloned()
    }
}

/// `bundle` with another address.
fn at(bundle: &str, address: &str) -> String {
    PairBundle {
        address: Some(address.parse().unwrap()),
        ..PairBundle::parse(bundle).unwrap()
    }
    .text()
}

/// Poll `f` until it yields, up to `secs`.
async fn until<T, F: std::future::Future<Output = Option<T>>>(
    secs: u64,
    what: &str,
    mut f: impl FnMut() -> F,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// ---------------------------------------------------------------- frames

/// The `relay` frames (§13.1) published on one hub's bus from now on.
struct Frames(broadcast::Receiver<Arc<events::Frame>>);

impl Frames {
    fn on(api: &ApiState) -> Frames {
        Frames(api.events.subscribe(Some(api.events.head())).rx)
    }

    /// The next `relay` frame before `deadline` (its `RelayStatus`).
    async fn next_before(&mut self, deadline: Instant) -> Option<Value> {
        loop {
            match tokio::time::timeout_at(deadline, self.0.recv()).await {
                Err(_) | Ok(Err(RecvError::Closed)) => return None,
                Ok(Err(RecvError::Lagged(n))) => panic!("{n} frames lost: the test reads too late"),
                Ok(Ok(f)) if f.event == "relay" => {
                    let v: Value = serde_json::from_str(&f.data).unwrap();
                    assert_eq!(v["schema"], 1, "a RelayStatus: {v}");
                    assert!(v["paired_hubs"].is_array(), "a RelayStatus: {v}");
                    return Some(v);
                }
                Ok(Ok(_)) => {}
            }
        }
    }

    /// The next `relay` frame within `ms`.
    async fn next(&mut self, ms: u64) -> Option<Value> {
        self.next_before(Instant::now() + Duration::from_millis(ms))
            .await
    }

    /// The first frame before `deadline` whose `paired_hubs` row for `far` passes `want`.
    async fn with_row(
        &mut self,
        deadline: Instant,
        far: &NodeId,
        want: impl Fn(&Value) -> bool,
    ) -> Option<Value> {
        loop {
            let f = self.next_before(deadline).await?;
            if row_of(&f, far).is_some_and(|r| want(&r)) {
                return Some(f);
            }
        }
    }

    /// Drop the frames that come until none has for `ms` (at most `max_ms` in all).
    async fn drain(&mut self, ms: u64, max_ms: u64) {
        let end = Instant::now() + Duration::from_millis(max_ms);
        while Instant::now() < end && self.next(ms).await.is_some() {}
    }
}

/// The `paired_hubs` row of `far` in a `RelayStatus`.
fn row_of(status: &Value, far: &NodeId) -> Option<Value> {
    let hex: String = far.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
    status["paired_hubs"]
        .as_array()?
        .iter()
        .find(|h| h["node_id"] == hex.as_str())
        .cloned()
}

/// A status without its ages (`gateway.checked_ago_s`, `gateway.last_flush_ago_s`): what a
/// forced frame may change when nothing else moved.
fn without_ages(status: &Value) -> Value {
    let mut v = status.clone();
    if let Some(g) = v.get_mut("gateway").and_then(Value::as_object_mut) {
        g.remove("checked_ago_s");
        g.remove("last_flush_ago_s");
    }
    v
}

// ---------------------------------------------------------------- the test

/// Row D6d (§13.1). `a` holds `b`'s record (b enrolled there, then dropped its own side, so no
/// connection is left) and scans `b`'s bundle at an address nobody answers (TEST-NET-1), marked
/// `lan`: the half emits no fabric event, yet the frame shows it on the record's row at once;
/// the attempt then dials until its window ends, long after its route answered, while nothing
/// publishes; the end is followed by one frame, the last status again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_relay_frame_follows_a_scan_and_an_attempt_that_ends_after_its_route() {
    let a = hub().await;
    let b = hub().await;
    // 1. b enrols at a (plain on loopback, marked lan): a records `theirs` for b; b then drops
    //    its side and its dial with it, so a holds the record with no half and no connection
    let v = b.scan(a.offer().await, Duration::from_secs(30)).await;
    assert_eq!(v["node_id"].as_str().map(str::len), Some(64), "{v}");
    until(30, "a holds theirs for b", || async {
        a.holds_theirs(&b.id).await.then_some(())
    })
    .await;
    pair::unpair(&b.fabric, &b.attempts, a.id).await.unwrap();
    until(15, "no connection between a and b", || async {
        (a.fabric.tcp().unwrap().connection_count() == 0).then_some(())
    })
    .await;
    assert!(a.link(&b.id).is_none(), "a holds no half of b yet");
    let shown = at(&b.offer().await, "192.0.2.7:11183");
    // 2. the frame follows a (as `relay::start` binds it); b's row before the scan, then quiet
    relay7::bind(&a.api, &a.fabric);
    let mut frames = Frames::on(&a.api);
    relay::changed();
    frames
        .with_row(Instant::now() + Duration::from_secs(5), &b.id, |r| {
            r["lan"] == false
        })
        .await
        .expect("b's row on the frame before the scan");
    frames.drain(300, 2_000).await;
    // 3. the scan records the half: the frame shows it at once, while the attempt dials
    let window = Duration::from_secs(8);
    let scanned = Instant::now();
    let v = a.scan(shown, window).await;
    assert_eq!(v["lan"], true, "{v}");
    assert!(
        a.attempts.in_flight(&b.id).await,
        "the route answered while the attempt dials"
    );
    assert!(a.link(&b.id).is_some_and(|l| l.lan));
    let half = frames
        .with_row(scanned + Duration::from_secs(5), &b.id, |r| {
            r["lan"] == true
        })
        .await
        .expect("the half on the relay frame, well before the attempt's window ends");
    let row = row_of(&half, &b.id).unwrap();
    assert_eq!(row["tls"], "none", "a lan pair runs plain: {row}");
    // 4. nothing moves while the attempt dials: no frame until just before its window ends
    //    (its deadline is taken after `scanned`)
    let quiet_until = scanned + window - Duration::from_millis(500);
    let mut stray = Vec::new();
    while let Some(f) = frames.next_before(quiet_until).await {
        stray.push((Instant::now() - scanned, row_of(&f, &b.id)));
    }
    assert!(
        stray.is_empty(),
        "frames while the attempt dialled (after the scan, b's row): {stray:?}"
    );
    assert!(
        a.attempts.in_flight(&b.id).await,
        "the window has not ended"
    );
    // 5. the window ends, long after the route answered: one frame follows the end, and it is
    //    the last status again apart from its ages (the record keeps its half)
    until(20, "the attempt's window ended", || async {
        (!a.attempts.in_flight(&b.id).await).then_some(())
    })
    .await;
    let after = frames
        .next(3_000)
        .await
        .expect("a relay frame after the attempt ended");
    assert_eq!(
        without_ages(&after),
        without_ages(&half),
        "a timed-out end changes nothing the status shows"
    );
    let more = frames.next(1_000).await;
    assert!(more.is_none(), "one frame per end: {more:?}");
    assert!(
        a.link(&b.id).is_some_and(|l| l.lan),
        "a recorded pair keeps its half"
    );
    pair::unpair(&a.fabric, &a.attempts, b.id).await.unwrap();
    assert!(a.link(&b.id).is_none());
}
