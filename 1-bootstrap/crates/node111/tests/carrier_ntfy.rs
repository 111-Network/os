//! Layer 7 row P1 (spec `7-relay/README.md` §6.2, §6.4, §15 G10): the ntfy carrier prototype
//! against the stdlib stub server `7-relay/verify/stub-ntfy.py`.
//!
//! Every test spawns its own stub (an ephemeral loopback port, fresh state) through [`Stub`], a
//! guard that kills it on drop, panics included. The 5B tests count the threads named
//! `111d-ntfy-5b` in this process, so they run one at a time ([`SERIAL`]). `smoke_log` runs the
//! prototype's smoke sequence and writes the §6.4 stub log to `$NTFY_SMOKE_LOG`; without that
//! variable it returns at once (`7-relay/verify/carrier.sh --smoke` sets it).

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use futures_util::{FutureExt, StreamExt};
use id111::keyset::NodeId;
use node111::daemon::carrier::ntfy::{self, NtfyCarrier, NtfyConfig, RequestRecord};
use node111::daemon::carrier::{Carrier, CarrierError, CarrierFrame, ForwardDone, HandOff, Pair};
use zeroize::Zeroizing;

/// The 5B tests count process threads by name: one at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn stub_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../7-relay/verify/stub-ntfy.py")
}

/// A running `stub-ntfy.py`; killed (and reaped) on drop.
struct Stub {
    child: Option<Child>,
    port: u16,
    args: Vec<String>,
}

impl Stub {
    fn start(args: &[&str]) -> Stub {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (child, port) = Self::spawn(&args, 0);
        Stub {
            child: Some(child),
            port,
            args,
        }
    }

    fn spawn(args: &[String], port: u16) -> (Child, u16) {
        let mut child = Command::new("python3")
            .arg("-I")
            .arg(stub_path())
            .args(["--port", &port.to_string(), "--quiet"])
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("python3 runs the stub");
        let mut line = String::new();
        BufReader::new(child.stdout.take().expect("stdout piped"))
            .read_line(&mut line)
            .expect("the stub prints its port");
        let bound: u16 = line.trim().parse().expect("a port number");
        assert!(port == 0 || bound == port, "restart rebinds the same port");
        (child, bound)
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn kill(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// A restart: the process dies, its in-memory cache with it; the new one binds the same port.
    fn restart(&mut self, args: Option<&[&str]>) {
        self.kill();
        if let Some(a) = args {
            self.args = a.iter().map(|s| s.to_string()).collect();
        }
        let (child, _) = Self::spawn(&self.args, self.port);
        self.child = Some(child);
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.kill();
    }
}

fn nid(b: u8) -> NodeId {
    NodeId::from([b; 32])
}

/// The two ends of one pairing: hub A (`own` 0xA1) and hub B (`own` 0xB2), one topic key.
fn pairs() -> (Pair, Pair) {
    let key = ntfy::topic_key(&[7; 32], &[0x11; 32], &[0x22; 32]).expect("a key");
    (
        Pair {
            own: nid(0xa1),
            peer: nid(0xb2),
            topic_key: key.clone(),
        },
        Pair {
            own: nid(0xb2),
            peer: nid(0xa1),
            topic_key: key,
        },
    )
}

fn carrier(url: &str, keepalive: Duration) -> NtfyCarrier {
    let mut cfg = NtfyConfig::new(url);
    cfg.keepalive = keepalive;
    cfg.connect_timeout = Duration::from_secs(3);
    cfg.request_timeout = Duration::from_secs(10);
    cfg.trace = true;
    NtfyCarrier::new(cfg).expect("a valid config")
}

fn put(seq: u64, len: usize) -> CarrierFrame {
    CarrierFrame {
        custody_seq: Some(seq),
        bytes: vec![(seq % 251) as u8 + 1; len],
    }
}

fn ack(len: usize) -> CarrierFrame {
    CarrierFrame {
        custody_seq: None,
        bytes: vec![0xac; len],
    }
}

fn label(d: &ForwardDone) -> &'static str {
    match d {
        ForwardDone::Taken => "taken",
        ForwardDone::Rejected { .. } => "rejected",
        ForwardDone::Retry => "retry",
    }
}

/// The fabric side of the hand-off, faked (§5.9): an outbox per hub, a log of `forward_done`, the
/// frames `carrier_inbound` got.
#[derive(Default)]
struct Fake {
    outbox: Mutex<VecDeque<CarrierFrame>>,
    done: Mutex<Vec<(u64, &'static str)>>,
    inbound: Mutex<Vec<(NodeId, Vec<u8>)>>,
    fail_inbound: AtomicBool,
}

impl HandOff for Fake {
    fn forward_take(
        &self,
        _target: NodeId,
        max: usize,
    ) -> BoxFuture<'_, Result<Vec<CarrierFrame>, CarrierError>> {
        async move {
            let mut q = self.outbox.lock().unwrap();
            let n = max.min(q.len());
            Ok(q.drain(..n).collect())
        }
        .boxed()
    }

    fn forward_done(
        &self,
        custody_seq: u64,
        done: ForwardDone,
    ) -> BoxFuture<'_, Result<(), CarrierError>> {
        async move {
            self.done.lock().unwrap().push((custody_seq, label(&done)));
            Ok(())
        }
        .boxed()
    }

    fn carrier_inbound(
        &self,
        from: NodeId,
        frame: Vec<u8>,
    ) -> BoxFuture<'_, Result<(), CarrierError>> {
        async move {
            if self.fail_inbound.load(Ordering::SeqCst) {
                return Err(CarrierError::HandOff("store busy (fake)".into()));
            }
            self.inbound.lock().unwrap().push((from, frame));
            Ok(())
        }
        .boxed()
    }
}

/// A raw request outside the carrier (the 4,097 B probe, the limiter count): status and latency.
fn raw(method: &str, url: &str, body: Option<&str>) -> (Option<u16>, Duration) {
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(10)))
        .max_idle_connections(0)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let started = Instant::now();
    let r = match (method, body) {
        ("POST", Some(b)) => agent.post(url).content_type("text/plain").send(b),
        _ => agent.get(url).call(),
    };
    let status = r.ok().map(|mut resp| {
        let _ = resp.body_mut().read_to_vec();
        resp.status().as_u16()
    });
    (status, started.elapsed())
}

/// Threads of this process named `111d-ntfy-5b` (`/proc/self/task/*/comm`).
fn threads_5b() -> usize {
    let Ok(dir) = std::fs::read_dir("/proc/self/task") else {
        return 0;
    };
    dir.filter_map(|e| e.ok())
        .filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok())
        .filter(|c| c.trim_end() == ntfy::THREAD_5B)
        .count()
}

async fn wait_for(what: &str, limit: Duration, mut cond: impl FnMut() -> bool) {
    let t = Instant::now();
    while !cond() {
        assert!(t.elapsed() < limit, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn next_item<S>(s: &mut S, limit: Duration) -> Option<Result<Vec<u8>, CarrierError>>
where
    S: futures_util::Stream<Item = Result<Vec<u8>, CarrierError>> + Unpin,
{
    tokio::time::timeout(limit, s.next())
        .await
        .expect("the held stream yields in time")
}

fn kb_rss() -> (u64, u64) {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        status
            .lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    (field("VmRSS:"), field("VmHWM:"))
}

// ---------------------------------------------------------------- 5A

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn publish_then_drain_in_custody_order_and_the_cursor_moves_on_commit() {
    let stub = Stub::start(&[]);
    let (pa, pb) = pairs();
    let a = carrier(&stub.url(), Duration::from_secs(45));
    let b = carrier(&stub.url(), Duration::from_secs(45));

    let out = vec![put(1, 313), ack(110), put(2, 200)];
    let want: Vec<Vec<u8>> = out.iter().map(|f| f.bytes.clone()).collect();
    let res = a.flush(&pa, out).await;
    assert_eq!(res.len(), 3);
    assert_eq!(res[0].0, Some(1));
    assert_eq!(res[1].0, None, "an ack keeps its None");
    assert!(res.iter().all(|(_, d)| matches!(d, ForwardDone::Taken)));
    let trace = a.take_trace();
    assert_eq!(trace.len(), 1, "three frames, one message: {trace:?}");
    assert_eq!((trace[0].method, trace[0].status), ("POST", Some(200)));

    // the wrong direction is empty: A's own inbound topic holds nothing
    assert!(a.drain(&pa).await.unwrap().is_empty());

    assert_eq!(b.drain(&pb).await.unwrap(), want);
    assert_eq!(b.cursor(&pb), None, "not committed yet");
    // without a commit the same frames are read again (C13)
    assert_eq!(b.drain(&pb).await.unwrap().len(), 3);
    let id = b.commit_drain(&pb).expect("an id");
    assert_eq!(b.cursor(&pb).as_deref(), Some(id.as_str()));
    assert!(
        b.drain(&pb).await.unwrap().is_empty(),
        "since=<id> after commit"
    );
    let polls = b.take_trace();
    assert!(polls
        .iter()
        .all(|r| r.method == "GET" && r.kind == "poll" && r.status == Some(200)));
    let s = b.stats();
    assert_eq!((s.drains, s.frames_in, s.malformed), (3, 6, 0));
    assert_eq!(a.stats().frames_out, 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_5a_flush_over_a_fake_hand_off_drains_posts_and_drains() {
    let stub = Stub::start(&[]);
    let (pa, pb) = pairs();
    let a = carrier(&stub.url(), Duration::from_secs(45));
    let b = carrier(&stub.url(), Duration::from_secs(45));
    let fa = Fake::default();
    let fb = Fake::default();

    fa.outbox
        .lock()
        .unwrap()
        .extend([put(10, 313), put(11, 1500), put(12, 1500), put(13, 3068)]);
    let r = a.flush_5a(&pa, &fa, 64).await.unwrap();
    assert_eq!(
        (r.taken_from_fabric, r.posted, r.rejected, r.retry),
        (4, 4, 0, 0)
    );
    assert_eq!(r.first.frames, 0);
    assert_eq!(
        *fa.done.lock().unwrap(),
        vec![(10, "taken"), (11, "taken"), (12, "taken"), (13, "taken")]
    );
    let posts = a
        .take_trace()
        .into_iter()
        .filter(|t| t.kind == "publish")
        .count();
    assert_eq!(
        posts, 3,
        "313 + 1500 | 1500 | 3068: three messages, never one per frame"
    );

    // B's flush: its first drain takes A's four puts; it answers with acks in the same flush
    fb.outbox.lock().unwrap().extend([ack(110), ack(110)]);
    let r = b.flush_5a(&pb, &fb, 64).await.unwrap();
    assert_eq!(r.first.frames, 4);
    assert!(r.first.all_taken && r.first.committed.is_some());
    assert_eq!((r.posted, r.acks), (2, 2));
    assert!(
        fb.done.lock().unwrap().is_empty(),
        "acks are logged, not reported"
    );
    let got = fb.inbound.lock().unwrap().clone();
    assert_eq!(got.len(), 4);
    assert!(got.iter().all(|(from, _)| *from == pa.own));
    assert_eq!(got[3].1.len(), 3068);

    // A's next flush: the acks arrive first
    let r = a.flush_5a(&pa, &fa, 64).await.unwrap();
    assert_eq!((r.first.frames, r.taken_from_fabric), (2, 0));
    assert_eq!(fa.inbound.lock().unwrap().len(), 2);

    // a fabric that cannot take a frame keeps the cursor (C13)
    fa.outbox.lock().unwrap().push_back(put(14, 50));
    a.flush_5a(&pa, &fa, 64).await.unwrap();
    fb.fail_inbound.store(true, Ordering::SeqCst);
    let before = b.cursor(&pb);
    let d = b.drain_into(&pb, &fb).await.unwrap();
    assert_eq!((d.frames, d.all_taken), (1, false));
    assert_eq!(b.cursor(&pb), before);
    fb.fail_inbound.store(false, Ordering::SeqCst);
    let d = b.drain_into(&pb, &fb).await.unwrap();
    assert_eq!((d.frames, d.all_taken), (1, true));
    assert_ne!(b.cursor(&pb), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_4096_byte_limit_and_413_is_rejected() {
    let stub = Stub::start(&[]);
    let (pa, _) = pairs();
    let topic = ntfy::outbound_topic(&pa);
    let url = format!("{}/{}", stub.url(), topic);
    assert_eq!(raw("POST", &url, Some(&"a".repeat(4096))).0, Some(200));
    assert_eq!(raw("POST", &url, Some(&"a".repeat(4097))).0, Some(413));

    let a = carrier(&stub.url(), Duration::from_secs(45));
    // the largest frame makes a 4,096 B body and is taken; one byte more is never posted
    let res = a
        .flush(
            &pa,
            vec![put(1, ntfy::MAX_FRAME), put(2, ntfy::MAX_FRAME + 1)],
        )
        .await;
    assert!(matches!(res[0].1, ForwardDone::Taken));
    assert!(matches!(res[1].1, ForwardDone::Rejected { .. }));
    let trace = a.take_trace();
    assert_eq!(trace.len(), 1, "the over-size frame made no request");
    assert_eq!(trace[0].sent, 4096);

    // a server with a lower limit answers 413: Rejected, and the next batch is still posted
    let small = Stub::start(&["--message-limit", "4000"]);
    let a = carrier(&small.url(), Duration::from_secs(45));
    let res = a
        .flush(&pa, vec![put(1, ntfy::MAX_FRAME), put(2, 100)])
        .await;
    match &res[0].1 {
        ForwardDone::Rejected { why } => assert!(why.contains("413"), "{why}"),
        other => panic!("413 must be Rejected, got {}", label(other)),
    }
    assert!(matches!(res[1].1, ForwardDone::Taken));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_default_limiter_answers_429_and_that_is_retry() {
    let stub = Stub::start(&[]);
    let (pa, pb) = pairs();
    let poll = format!("{}/{}/json?poll=1", stub.url(), ntfy::inbound_topic(&pa));
    for i in 0..60 {
        assert_eq!(
            raw("GET", &poll, None).0,
            Some(200),
            "request {i} is inside the burst"
        );
    }
    assert_eq!(raw("GET", &poll, None).0, Some(429), "the 61st");
    let a = carrier(&stub.url(), Duration::from_secs(45));
    let res = a
        .flush(&pa, vec![put(1, 10), put(2, ntfy::MAX_FRAME)])
        .await;
    assert!(res.iter().all(|(_, d)| matches!(d, ForwardDone::Retry)));
    assert_eq!(
        a.take_trace().len(),
        1,
        "after a 429 the rest of the flush is not posted"
    );
    assert!(matches!(a.drain(&pb).await, Err(CarrierError::Retry(_))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_refusals_are_rejected_and_the_token_is_taken() {
    let stub = Stub::start(&["--token", "tk_pair_ab"]);
    let (pa, pb) = pairs();
    let none = carrier(&stub.url(), Duration::from_secs(45));
    let res = none
        .flush(&pa, vec![put(1, 10), put(2, ntfy::MAX_FRAME)])
        .await;
    match &res[0].1 {
        ForwardDone::Rejected { why } => assert!(why.contains("403"), "{why}"),
        other => panic!("403 must be Rejected, got {}", label(other)),
    }
    assert!(
        matches!(res[1].1, ForwardDone::Retry),
        "not posted after a 403"
    );
    assert!(matches!(
        none.drain(&pb).await,
        Err(CarrierError::Rejected(_))
    ));

    let mut cfg = NtfyConfig::new(stub.url());
    cfg.token = Some(Zeroizing::new("tk_wrong".into()));
    let wrong = NtfyCarrier::new(cfg).unwrap();
    match &wrong.flush(&pa, vec![put(1, 10)]).await[0].1 {
        ForwardDone::Rejected { why } => assert!(why.contains("401"), "{why}"),
        other => panic!("401 must be Rejected, got {}", label(other)),
    }

    let mut cfg = NtfyConfig::new(stub.url());
    cfg.token = Some(Zeroizing::new("tk_pair_ab".into()));
    let good = NtfyCarrier::new(cfg).unwrap();
    assert!(matches!(
        good.flush(&pa, vec![put(1, 10)]).await[0].1,
        ForwardDone::Taken
    ));
    let mut cfg = NtfyConfig::new(stub.url());
    cfg.token = Some(Zeroizing::new("tk_pair_ab".into()));
    let good_b = NtfyCarrier::new(cfg).unwrap();
    assert_eq!(good_b.drain(&pb).await.unwrap(), vec![vec![2u8; 10]]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_route_is_retry() {
    let mut stub = Stub::start(&[]);
    let url = stub.url();
    stub.kill();
    let (pa, pb) = pairs();
    let a = carrier(&url, Duration::from_secs(45));
    let res = a.flush(&pa, vec![put(1, 10)]).await;
    assert!(matches!(res[0].1, ForwardDone::Retry));
    assert!(matches!(a.drain(&pb).await, Err(CarrierError::Retry(_))));
    let fa = Fake::default();
    fa.outbox.lock().unwrap().push_back(put(1, 10));
    assert!(
        a.flush_5a(&pa, &fa, 8).await.is_err(),
        "the failed check stops the flush"
    );
    assert_eq!(fa.outbox.lock().unwrap().len(), 1, "nothing was taken");
    let t = a.take_trace();
    assert!(t.iter().all(|r| r.status.is_none() && r.error.is_some()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn since_after_a_stub_restart_is_a_miss_then_since_all() {
    let mut stub = Stub::start(&[]);
    let (pa, pb) = pairs();
    let a = carrier(&stub.url(), Duration::from_secs(45));
    let b = carrier(&stub.url(), Duration::from_secs(45));
    let fb = Fake::default();
    a.flush(&pa, vec![put(1, 40)]).await;
    let d = b.drain_into(&pb, &fb).await.unwrap();
    assert_eq!((d.frames, d.since_miss), (1, false));
    assert!(d.committed.is_some());

    stub.restart(None); // the cache is gone
    a.flush(&pa, vec![put(2, 41)]).await;
    assert!(matches!(b.drain(&pb).await, Err(CarrierError::SinceMiss)));
    assert_eq!(b.cursor(&pb), None, "the lost id is dropped");
    assert_eq!(
        b.drain(&pb).await.unwrap(),
        vec![vec![3u8; 41]],
        "since=all"
    );
    assert_eq!(b.stats().since_misses, 1);
    // drain_into does both reads in one call and says so
    stub.restart(None);
    b.restore_cursor(&pb, d.committed.clone()).unwrap();
    a.flush(&pa, vec![put(3, 42)]).await;
    let d2 = b.drain_into(&pb, &fb).await.unwrap();
    assert_eq!((d2.frames, d2.since_miss, d2.all_taken), (1, true, true));

    // a server that answers an unknown id with everything (silent fallback): no miss is visible
    stub.restart(Some(&["--since-miss", "all"]));
    a.flush(&pa, vec![put(4, 43)]).await;
    let d3 = b.drain_into(&pb, &fb).await.unwrap();
    assert_eq!((d3.frames, d3.since_miss), (1, false));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_bytes_on_the_topic_are_dropped_and_counted() {
    let stub = Stub::start(&[]);
    let (pa, pb) = pairs();
    let url = format!("{}/{}", stub.url(), ntfy::outbound_topic(&pa));
    // a third client posts plain text, a bad batch, then a well-formed one
    assert_eq!(raw("POST", &url, Some("hello, world")).0, Some(200));
    assert_eq!(
        raw("POST", &url, Some(&ntfy::b64url_encode(&[2, 1, 0, 1, 9]))).0,
        Some(200)
    );
    let good = ntfy::encode_batch(&[&[5u8; 9][..]]).unwrap();
    assert_eq!(raw("POST", &url, Some(&good)).0, Some(200));
    let b = carrier(&stub.url(), Duration::from_secs(45));
    // the well-formed one reaches the fabric, which verifies it (pair_mac, D11) like any frame
    assert_eq!(b.drain(&pb).await.unwrap(), vec![vec![5u8; 9]]);
    assert_eq!(b.stats().malformed, 2);
    // the cursor moves past the garbage
    b.commit_drain(&pb);
    assert!(b.drain(&pb).await.unwrap().is_empty());
}

// ---------------------------------------------------------------- 5B

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_held_stream_runs_on_one_named_thread_with_keepalives_and_survives_a_restart() {
    let _serial = SERIAL.lock().await;
    let mut stub = Stub::start(&["--keepalive", "1"]);
    let (pa, pb) = pairs();
    let a = carrier(&stub.url(), Duration::from_secs(45));
    let b = carrier(&stub.url(), Duration::from_secs(1));
    assert_eq!(threads_5b(), 0);
    assert!(
        a.drain(&pa).await.unwrap().is_empty(),
        "5A starts no thread"
    );
    assert_eq!(threads_5b(), 0);

    let mut s = b.hold(&pb);
    wait_for("the 5B thread", Duration::from_secs(3), || {
        threads_5b() == 1
    })
    .await;
    assert!(b.holding());
    // a second hold is refused while the thread lives
    let mut again = b.hold(&pb);
    assert!(matches!(
        next_item(&mut again, Duration::from_secs(1)).await,
        Some(Err(CarrierError::Retry(_)))
    ));
    assert!(next_item(&mut again, Duration::from_secs(1))
        .await
        .is_none());
    drop(again);
    assert_eq!(threads_5b(), 1, "exactly one thread");

    a.flush(&pa, vec![put(1, 100), put(2, 200)]).await;
    assert_eq!(
        next_item(&mut s, Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap(),
        vec![2u8; 100]
    );
    assert_eq!(
        next_item(&mut s, Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap(),
        vec![3u8; 200]
    );
    let id = b.commit_drain(&pb).expect("the stream moved the cursor");

    wait_for("two keepalives", Duration::from_secs(5), || {
        b.stats().keepalives >= 2
    })
    .await;
    let gap = b.stats().keepalive_gap_ms;
    assert!(
        (700..=2000).contains(&gap),
        "keepalive gap {gap} ms for a 1 s stub"
    );

    // the stub restarts: the stream drops, reconnects with since=<id>, misses, reads since=all
    stub.restart(None);
    let mut retry = 0;
    let mut miss = 0;
    let mut posted = false;
    let t = Instant::now();
    loop {
        assert!(t.elapsed() < Duration::from_secs(15), "the stream recovers");
        if miss > 0 && !posted {
            a.flush(&pa, vec![put(3, 300)]).await;
            posted = true;
        }
        match next_item(&mut s, Duration::from_secs(10)).await {
            Some(Err(CarrierError::Retry(_))) => retry += 1,
            Some(Err(CarrierError::SinceMiss)) => miss += 1,
            Some(Ok(f)) => {
                assert_eq!(f, vec![4u8; 300]);
                break;
            }
            other => panic!("unexpected {:?}", other.map(|r| r.map(|f| f.len()))),
        }
    }
    assert!(retry >= 1 && miss == 1, "retry {retry}, miss {miss}");
    assert_ne!(b.commit_drain(&pb).as_deref(), Some(id.as_str()));
    assert!(b.stats().reconnects >= 1);
    assert_eq!(threads_5b(), 1, "still the one thread");

    drop(s);
    wait_for("the 5B thread to end", Duration::from_secs(4), || {
        threads_5b() == 0
    })
    .await;
    wait_for("the holding flag", Duration::from_secs(1), || !b.holding()).await;
    let records = b.take_trace();
    assert!(records
        .iter()
        .all(|r| r.kind == "stream" && r.method == "GET"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_held_stream_read_timeout_is_three_keepalives() {
    let _serial = SERIAL.lock().await;
    let stub = Stub::start(&["--keepalive", "0"]); // the server sends no keepalive at all
    let (_, pb) = pairs();
    let b = carrier(&stub.url(), Duration::from_secs(1));
    assert_eq!(b.read_timeout(), Duration::from_secs(3));
    let mut s = b.hold(&pb);
    let t = Instant::now();
    match next_item(&mut s, Duration::from_secs(8)).await {
        Some(Err(CarrierError::Retry(why))) => assert!(why.contains("read timeout"), "{why}"),
        other => panic!(
            "expected the read timeout, got {:?}",
            other.map(|r| r.map(|f| f.len()))
        ),
    }
    let waited = t.elapsed();
    assert!(
        waited >= Duration::from_millis(2800) && waited < Duration::from_secs(6),
        "{waited:?}"
    );
    drop(s);
    wait_for("the 5B thread to end", Duration::from_secs(5), || {
        threads_5b() == 0
    })
    .await;
}

// ---------------------------------------------------------------- the smoke log (§6.4)

fn ms(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64() * 1000.0)
}

fn record_row(step: &str, r: &RequestRecord) -> String {
    format!(
        "| {step} | {} | {} | {} | {} | {} | {} |\n",
        r.method,
        r.kind,
        r.status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "none".into()),
        r.sent,
        r.received,
        ms(r.latency)
    )
}

/// The prototype's smoke sequence against the stub (§6.4, P1). Writes `$NTFY_SMOKE_LOG`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn smoke_log() {
    let Ok(log_path) = std::env::var("NTFY_SMOKE_LOG") else {
        eprintln!(
            "NTFY_SMOKE_LOG unset: the smoke log is written by 7-relay/verify/carrier.sh --smoke"
        );
        return;
    };
    let _serial = SERIAL.lock().await;
    let calver = std::env::var("NTFY_SMOKE_CALVER").unwrap_or_else(|_| "unknown".into());
    let date = std::env::var("NTFY_SMOKE_DATE").unwrap_or_else(|_| "unknown".into());
    let ntfy_bin = std::env::var("NTFY_BIN")
        .ok()
        .filter(|p| std::path::Path::new(p).is_file());
    let mut md = String::new();
    let mut rows = String::new();
    let (pa, pb) = pairs();

    // ---- requests: publish, poll, 4,096 / 4,097, the 5A flush over the fake hand-off
    let keepalive_s = 2u64;
    let mut stub = Stub::start(&["--keepalive", &keepalive_s.to_string()]);
    let a = carrier(&stub.url(), Duration::from_secs(keepalive_s));
    let b = carrier(&stub.url(), Duration::from_secs(keepalive_s));
    let (fa, fb) = (Fake::default(), Fake::default());
    fa.outbox
        .lock()
        .unwrap()
        .extend([put(1, 313), put(2, 313), put(3, ntfy::MAX_FRAME)]);
    let ra = a.flush_5a(&pa, &fa, 64).await.unwrap();
    let a_first = a.take_trace();
    let a_messages = a_first.iter().filter(|r| r.kind == "publish").count();
    for r in &a_first {
        rows.push_str(&record_row("A: 5A flush (drain, post 3 puts, drain)", r));
    }
    fb.outbox
        .lock()
        .unwrap()
        .extend([ack(110), ack(110), ack(110)]);
    let rb = b.flush_5a(&pb, &fb, 64).await.unwrap();
    for r in b.take_trace() {
        rows.push_str(&record_row(
            "B: 5A flush (drain 3 puts, post 3 acks, drain)",
            &r,
        ));
    }
    let ra2 = a.flush_5a(&pa, &fa, 64).await.unwrap();
    for r in a.take_trace() {
        rows.push_str(&record_row("A: 5A flush (drain 3 acks, nothing owed)", &r));
    }
    let url = format!("{}/{}", stub.url(), ntfy::outbound_topic(&pa));
    let (s4096, l4096) = raw("POST", &url, Some(&"a".repeat(4096)));
    let (s4097, l4097) = raw("POST", &url, Some(&"a".repeat(4097)));
    let st = |s: Option<u16>| s.map(|v| v.to_string()).unwrap_or_else(|| "none".into());
    rows.push_str(&format!(
        "| raw: 4,096 B body | POST | publish | {} | 4096 | n/a | {} |\n",
        st(s4096),
        ms(l4096)
    ));
    rows.push_str(&format!(
        "| raw: 4,097 B body | POST | publish | {} | 4097 | n/a | {} |\n",
        st(s4097),
        ms(l4097)
    ));
    let over = a.flush(&pa, vec![put(9, ntfy::MAX_FRAME + 1)]).await;
    let over_posted = a.take_trace().len();

    // ---- keepalive and the 5B thread
    let threads_before = threads_5b();
    let mut s = b.hold(&pb);
    wait_for("the 5B thread", Duration::from_secs(3), || {
        threads_5b() == 1
    })
    .await;
    let threads_holding = threads_5b();
    a.flush(&pa, vec![put(4, 120)]).await;
    a.take_trace();
    let got5b = next_item(&mut s, Duration::from_secs(5)).await;
    wait_for("three keepalives", Duration::from_secs(10), || {
        b.stats().keepalives >= 3
    })
    .await;
    let gap = b.stats().keepalive_gap_ms;
    drop(s);
    wait_for("the 5B thread to end", Duration::from_secs(5), || {
        threads_5b() == 0
    })
    .await;
    for r in b.take_trace() {
        rows.push_str(&record_row("B: 5B held stream (to the response head)", &r));
    }

    // ---- since after a stub restart (explicit miss, then the silent fallback)
    b.commit_drain(&pb);
    let before_restart = b.cursor(&pb);
    stub.restart(None);
    a.flush(&pa, vec![put(5, 50)]).await;
    let miss = b.drain(&pb).await;
    let after_all = b.drain(&pb).await;
    for r in a.take_trace().iter().chain(b.take_trace().iter()) {
        rows.push_str(&record_row("since=<id> after a restart, then since=all", r));
    }
    b.commit_drain(&pb);
    stub.restart(Some(&[
        "--keepalive",
        &keepalive_s.to_string(),
        "--since-miss",
        "all",
    ]));
    a.flush(&pa, vec![put(6, 51)]).await;
    let fallback = b.drain(&pb).await;
    a.take_trace();
    b.take_trace();
    drop(stub);

    // ---- the default limiter (burst 60, then one per 5 s)
    let lim = Stub::start(&[]);
    let poll = format!("{}/{}/json?poll=1", lim.url(), ntfy::inbound_topic(&pa));
    let mut first_429 = None;
    for i in 1..=70 {
        if raw("GET", &poll, None).0 == Some(429) {
            first_429 = Some(i);
            break;
        }
    }
    let la = carrier(&lim.url(), Duration::from_secs(45));
    let limited_flush = la.flush(&pa, vec![put(7, 10)]).await;
    for r in la.take_trace() {
        rows.push_str(&record_row("a flush inside the limit", &r));
    }
    tokio::time::sleep(Duration::from_millis(5200)).await;
    let after_5s = raw("GET", &poll, None).0;
    let after_5s_next = raw("GET", &poll, None).0;
    drop(lim);

    // ---- RSS during a flush (this test process: the carrier, ureq, tokio and the harness)
    let rss_stub = Stub::start(&["--burst", "1000"]);
    let r = carrier(&rss_stub.url(), Duration::from_secs(45));
    let frames: Vec<CarrierFrame> = (0..40).map(|i| put(100 + i, 1500)).collect();
    let (rss0, hwm0) = kb_rss();
    let stop = Arc::new(AtomicBool::new(false));
    let peak = Arc::new(Mutex::new(rss0));
    let sampler = {
        let stop = stop.clone();
        let peak = peak.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let (now, _) = kb_rss();
                let mut p = peak.lock().unwrap();
                *p = (*p).max(now);
                drop(p);
                std::thread::sleep(Duration::from_millis(2));
            }
        })
    };
    let t = Instant::now();
    let rss_res = r.flush(&pa, frames).await;
    let flush_time = t.elapsed();
    stop.store(true, Ordering::SeqCst);
    let _ = sampler.join();
    let (rss1, hwm1) = kb_rss();
    let rss_peak = *peak.lock().unwrap();
    let rss_trace = r.take_trace();
    let rss_posts = rss_trace.len();
    let rss_body_bytes: usize = rss_trace.iter().map(|t| t.sent).sum();
    let rss_taken = rss_res
        .iter()
        .filter(|(_, d)| matches!(d, ForwardDone::Taken))
        .count();
    drop(rss_stub);

    // ---- write the log
    md.push_str(&format!(
        "# ntfy carrier smoke log — stub, {calver}\n\n\
         Row P1 (`7-relay/TASKS.md`), spec `7-relay/README.md` §6.2 and §6.4. Written by \
         `7-relay/verify/carrier.sh --smoke` (the test `smoke_log` in \
         `1-bootstrap/crates/node111/tests/carrier_ntfy.rs`) on {date} (UTC).\n\n\
         **Server: `7-relay/verify/stub-ntfy.py`, the stdlib stand-in, not ntfy.** Every figure \
         below is the prototype client (`N/carrier/ntfy.rs`) against the stub on loopback; the \
         real-server log is `logs/ntfy-smoke-<calver>.md` (G10, row V4, an owner-run server). \
         The calver names the build: `NODE111_BUILD_VERSION` when a release build is stamped, \
         else the date of the run (no signed release exists yet).\n\n"
    ));
    md.push_str(
        "## Requests (method, status, body bytes, latency)\n\n\
         Client latency from the call to the end of the answer read (a held stream: to its \
         response head). `sent` is the request body, `received` the response body bytes read.\n\n\
         | Step | Method | Kind | Status | Sent B | Received B | Latency ms |\n\
         |---|---|---|---|---|---|---|\n",
    );
    md.push_str(&rows);
    md.push_str(&format!(
        "\nThe 5A sequence over the fake hand-off: A posted {} puts in {} messages (taken {}), B's \
         first drain took {} frames and posted {} acks, A's next first drain took {} acks; \
         `forward_done` for A's puts: {:?}.\n\n",
        ra.taken_from_fabric,
        a_messages,
        ra.posted,
        rb.first.frames,
        rb.acks,
        ra2.first.frames,
        *fa.done.lock().unwrap()
    ));
    md.push_str(&format!(
        "## The 4,096 / 4,097 B boundary\n\n\
         - a 4,096 B body: **{}**; a 4,097 B body: **{}** (the stub's `--message-limit 4096`, \
         attachments off).\n\
         - the carrier's largest frame, {} B, makes a body of exactly 4,096 B (posted above); a \
         frame of {} B is `{}` by the carrier and makes **{} requests** (never posted).\n\n",
        st(s4096),
        st(s4097),
        ntfy::MAX_FRAME,
        ntfy::MAX_FRAME + 1,
        over.first().map(|(_, d)| label(d)).unwrap_or("none"),
        over_posted
    ));
    md.push_str(&format!(
        "## 429 (the default limiter: a burst of 60, then one request per 5 s)\n\n\
         - the first 429 came at request **{}** (60 inside the burst);\n\
         - a flush inside the limit: `{}` (`Retry`, the row stays; the rest of a flush is not \
         posted after a 429);\n\
         - after 5.2 s one request: **{}**, the next: **{}**.\n\n",
        first_429
            .map(|v| v.to_string())
            .unwrap_or_else(|| "none in 70".into()),
        limited_flush
            .first()
            .map(|(_, d)| label(d))
            .unwrap_or("none"),
        st(after_5s),
        st(after_5s_next)
    ));
    let res_kind = |r: &Result<Vec<Vec<u8>>, CarrierError>| match r {
        Ok(f) => format!("Ok, {} frame(s)", f.len()),
        Err(e) => format!("Err: {e}"),
    };
    md.push_str(&format!(
        "## `since` after a stub restart\n\n\
         The stub keeps its cache in memory, so a restart loses it (as an ntfy server without \
         `cache-file`). B's saved id before the restart: `{}`.\n\n\
         - `--since-miss error` (the stub's default; an explicit signal): the drain with \
         `since=<id>` gave **{}**; the next drain, `since=all`, gave **{}**.\n\
         - `--since-miss all` (a silent fallback: every cached message for an unknown id): the \
         drain gave **{}**, with no miss visible to the client, so §5.9 re-post trigger (3) \
         cannot fire on such a server. Which answer upstream gives is the §6.4 bench item \
         (*unverified from this box*).\n\n",
        before_restart.as_deref().unwrap_or("none"),
        res_kind(&miss),
        res_kind(&after_all),
        res_kind(&fallback)
    ));
    md.push_str(&format!(
        "## Keepalive and the 5B thread\n\n\
         - the stub ran with `--keepalive {keepalive_s}`; the client measured a gap of **{gap} ms** \
         between keepalive events (upstream's default `keepalive-interval` is 45 s, *unverified \
         from this box*); the client's read timeout is 3 × the configured interval ({} s here).\n\
         - threads named `{}`: {} before the hold, **{}** while holding, 0 after the stream was \
         dropped; a frame posted by A arrived on the stream: {}.\n\n",
        keepalive_s * 3,
        ntfy::THREAD_5B,
        threads_before,
        threads_holding,
        matches!(got5b, Some(Ok(_)))
    ));
    md.push_str(&format!(
        "## RSS during a flush\n\n\
         `/proc/self/status` of the test process (the carrier, ureq + rustls, the tokio runtime \
         and the test harness, not `111d`): a flush of 40 frames × 1,500 B ({} messages, {} \
         frames taken) in {} ms. VmRSS before **{} kB**, peak during **{} kB** (sampled every \
         2 ms), after **{} kB**: the flush added **{} kB** at its peak to a process that had \
         already run the steps above (allocator pages reused); VmHWM {} → {} kB. The flush's \
         own data: {} B of frames, {} B of bodies.\n\n",
        rss_posts,
        rss_taken,
        ms(flush_time),
        rss0,
        rss_peak,
        rss1,
        rss_peak.saturating_sub(rss0),
        hwm0,
        hwm1,
        40 * 1500,
        rss_body_bytes
    ));
    md.push_str(
        "## Bytes per frame (base64 4/3 + the batch's 2 B per frame)\n\n\
         A batch body is `ceil((2 + Σ(2 + frame)) × 4/3)` B; the HTTP request adds its headers.\n\n\
         | Frames in one message | Body B | Body per frame B | Overhead per frame B |\n\
         |---|---|---|---|\n",
    );
    for (n, size) in [
        (1usize, 110usize),
        (1, 313),
        (3, 313),
        (9, 313),
        (1, 1500),
        (2, 1500),
        (1, 3068),
    ] {
        let f = vec![0u8; size];
        let refs: Vec<&[u8]> = (0..n).map(|_| &f[..]).collect();
        let body = ntfy::encode_batch(&refs).map(|b| b.len()).unwrap_or(0);
        md.push_str(&format!(
            "| {n} × {size} B | {body} | {:.1} | {:.1} |\n",
            body as f64 / n as f64,
            body as f64 / n as f64 - size as f64
        ));
    }
    md.push_str(&format!(
        "\n## Owner-server items (§6.4)\n\n\
         {}\n\n\
         - the server version and its config (`7-relay/carriers/ntfy/server.yml` is the \
         recommended one): **not run from this box**\n\
         - per request method, status, body bytes and latency against a real server: **not run \
         from this box**\n\
         - the 4,096 / 4,097 B boundary on a real server: **not run from this box**\n\
         - 429 behaviour and the 30-connections-per-visitor limit on a real server: **not run \
         from this box**\n\
         - retention across a server restart with and without `cache-file`: **not run from this \
         box**\n\
         - `since=<id>` after eviction, and `since=all`: **not run from this box**\n\
         - the held stream's keepalive interval on a real server: **not run from this box**\n\
         - whether the server forwards messages to a push service (Firebase, an upstream \
         server): **not run from this box**\n\
         - the token header (`Authorization: Bearer`): **not run from this box**\n\
         - ntfy.sh's anonymous limits and retention: **not run from this box**\n\
         - end-to-end delivery between two gateway hubs in netns (G10, row V4): **not run from \
         this box**\n",
        match &ntfy_bin {
            Some(p) => format!(
                "`NTFY_BIN` is on disk (`{p}`): the real-server run is G10's \
                 (`REQUIRE_NTFY=1 7-relay/verify/carrier.sh`, row V4); this stub smoke did not \
                 drive it."
            ),
            None => "`NTFY_BIN` is not set or not on disk: no real ntfy server ran.".into(),
        }
    ));
    std::fs::write(&log_path, md).expect("write the smoke log");

    // the smoke's own assertions: the log says what happened, and what happened is right
    assert_eq!((ra.posted, a_messages), (3, 2));
    assert_eq!((rb.first.frames, rb.acks), (3, 3));
    assert_eq!(ra2.first.frames, 3);
    assert_eq!((s4096, s4097), (Some(200), Some(413)));
    assert_eq!(over_posted, 0);
    assert_eq!(first_429, Some(61));
    assert!(matches!(limited_flush[0].1, ForwardDone::Retry));
    assert_eq!((after_5s, after_5s_next), (Some(200), Some(429)));
    assert!(matches!(miss, Err(CarrierError::SinceMiss)));
    assert!(matches!(&after_all, Ok(f) if f.len() == 1));
    assert!(matches!(&fallback, Ok(f) if f.len() == 1));
    assert_eq!(threads_holding, 1);
    assert!(matches!(got5b, Some(Ok(_))));
    assert_eq!(rss_taken, 40);
}
