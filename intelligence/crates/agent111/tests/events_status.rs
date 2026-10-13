//! T5: events journal (append/replay/trim), /health, /v1/agent/status,
//! /v1/agent/events (SSE replay + live), Host guard, GET-only routes.
//!
//! Hermetic: tempdir state, ephemeral loopback ports, std-only raw HTTP
//! clients with hard deadlines (no test can hang).

mod common;

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use agent111::config::Config;
use agent111::events::{Event, Journal};
use agent111::http::{router, AppState};
use agent111::kb::Kb;
use agent111::model::Client;
use agent111::store::Store;
use agent111::tasks::TaskEngine;
use common::{respond, MockServer};
use serde_json::{json, Value};

const MIB: u64 = 1024 * 1024;

// ---------------------------------------------------------------- journal

fn read_events(path: &Path) -> Vec<Event> {
    std::fs::read_to_string(path)
        .expect("read events file")
        .lines()
        .map(|l| serde_json::from_str(l).expect("every line is a valid event"))
        .collect()
}

#[test]
fn journal_append_tail_order_and_monotonic_seq() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("events.jsonl");
    let j = Journal::new(path.clone());
    let mut rx = j.subscribe();

    for i in 1..=5 {
        j.append(&format!("k.{i}"), json!({"i": i}));
    }
    let all = j.tail(100);
    assert_eq!(
        all.iter().map(|e| e.seq).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5]
    );
    assert_eq!(all[0].kind, "k.1");
    assert_eq!(all[4].detail, json!({"i": 5}));
    assert!(all.iter().all(|e| e.schema == 1 && !e.ts.is_empty()));

    let last2 = j.tail(2);
    assert_eq!(last2.iter().map(|e| e.seq).collect::<Vec<_>>(), [4, 5]);
    assert!(j.tail(0).is_empty());

    // subscribers see the same events in the same order
    for want in 1..=5 {
        assert_eq!(rx.try_recv().unwrap().seq, want);
    }

    // concurrent appends: file order == seq order, no gaps
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                for _ in 0..50 {
                    j.append("c", json!(null));
                }
            });
        }
    });
    let on_disk = read_events(&path);
    assert_eq!(on_disk.len(), 205);
    for (i, e) in on_disk.iter().enumerate() {
        assert_eq!(e.seq, i as u64 + 1, "file order must equal seq order");
    }
}

#[test]
fn journal_restart_replays_history_and_continues_seq() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("events.jsonl");
    {
        let j = Journal::new(path.clone());
        for i in 1..=5 {
            j.append(&format!("k.{i}"), json!({"i": i}));
        }
    }
    let j = Journal::new(path.clone());
    let replayed = j.tail(5);
    assert_eq!(
        replayed.iter().map(|e| e.seq).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5]
    );
    assert_eq!(replayed[2].kind, "k.3");
    assert_eq!(replayed[2].detail, json!({"i": 3}));

    j.append("after", json!({}));
    assert_eq!(j.tail(1)[0].seq, 6, "seq must continue, not reset");
    assert_eq!(read_events(&path).last().unwrap().seq, 6);
}

#[test]
fn journal_replay_keeps_only_last_1000() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("events.jsonl");
    {
        let j = Journal::new(path.clone());
        for _ in 0..1200 {
            j.append("n", json!(null));
        }
    }
    let j = Journal::new(path);
    let all = j.tail(5000);
    assert_eq!(all.len(), 1000);
    assert_eq!(all.first().unwrap().seq, 201);
    assert_eq!(all.last().unwrap().seq, 1200);
}

#[test]
fn journal_replay_skips_garbage_and_heals_a_torn_last_line() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("events.jsonl");
    let good =
        |seq: u64| format!(r#"{{"schema":1,"seq":{seq},"ts":"t","kind":"g","detail":null}}"#);
    // valid, junk, valid, torn fragment without a trailing newline
    std::fs::write(
        &path,
        format!(
            "{}\nnot json at all\n{}\n{{\"schema\":1,\"seq\":9",
            good(1),
            good(7)
        ),
    )
    .unwrap();

    let j = Journal::new(path.clone());
    assert_eq!(j.tail(10).iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 7]);
    j.append("next", json!({}));
    assert_eq!(
        j.tail(1)[0].seq,
        8,
        "seq continues from the highest valid line"
    );

    // the new line must not be glued onto the torn fragment
    let last = std::fs::read_to_string(&path).unwrap();
    let last_line = last.lines().last().unwrap();
    let ev: Event = serde_json::from_str(last_line).expect("new line parses on its own");
    assert_eq!(ev.seq, 8);
}

#[test]
fn journal_trims_to_half_over_1mib_and_keeps_newest() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("events.jsonl");
    let j = Journal::new(path.clone());
    let pad = "x".repeat(1000);
    let total = 1500u64;

    let mut prev_len = 0u64;
    let mut trims = 0;
    for i in 1..=total {
        j.append("pad", json!({"i": i, "pad": pad}));
        let len = std::fs::metadata(&path).unwrap().len();
        assert!(len <= MIB, "file must never rest above 1 MiB (was {len})");
        if len < prev_len {
            trims += 1;
            assert!(len <= 600 * 1024, "trim must land near 512 KiB (was {len})");
            let newest: Event = serde_json::from_str(
                std::fs::read_to_string(&path)
                    .unwrap()
                    .lines()
                    .last()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(newest.seq, i, "the just-appended line must survive a trim");
        }
        prev_len = len;
    }
    assert!(trims >= 1, "1500 x ~1 KiB must trigger at least one trim");

    // FIFO: whole lines, contiguous, newest end intact, oldest dropped
    let on_disk = read_events(&path);
    assert_eq!(on_disk.last().unwrap().seq, total);
    assert!(on_disk.first().unwrap().seq > 1);
    for w in on_disk.windows(2) {
        assert_eq!(w[1].seq, w[0].seq + 1);
    }
    assert!(!tmp.path().join("events.jsonl.tmp").exists(), "no tmp left");

    // a restart after trimming continues the stream
    let j2 = Journal::new(path);
    j2.append("after", json!({}));
    assert_eq!(j2.tail(1)[0].seq, total + 1);
}

fn assert_memory_only(j: &Journal) {
    let mut rx = j.subscribe();
    for i in 1..=3 {
        j.append("m", json!({"i": i}));
    }
    assert_eq!(
        j.tail(10).iter().map(|e| e.seq).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(rx.try_recv().unwrap().seq, 1);
}

#[test]
fn journal_degrades_to_memory_when_it_cannot_write() {
    let tmp = tempfile::tempdir().unwrap();

    // parent directory missing
    assert_memory_only(&Journal::new(tmp.path().join("nope").join("events.jsonl")));

    // the path is a directory
    let as_dir = tmp.path().join("is-a-dir");
    std::fs::create_dir(&as_dir).unwrap();
    assert_memory_only(&Journal::new(as_dir));

    // read-only directory (a no-op for root, which can still write)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let ro = tmp.path().join("ro");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        assert_memory_only(&Journal::new(ro.join("events.jsonl")));
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

// ------------------------------------------------------------ test server

struct TestServer {
    addr: SocketAddr,
    journal: Journal,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    fn start(state_dir: &Path, model_base: &str) -> Self {
        Self::start_with_kb(state_dir, model_base, None)
    }

    fn start_with_kb(state_dir: &Path, model_base: &str, kb_dir: Option<&Path>) -> Self {
        let cfg = Arc::new(Config {
            state_dir: state_dir.to_path_buf(),
            model_base: model_base.to_string(),
            category: "2A".to_string(),
            node_id: Some("ab12cd34".to_string()),
            ..Config::default()
        });
        let store = Arc::new(Store::init(state_dir).expect("init store"));
        let kb = Arc::new(kb_dir.map(Kb::open).unwrap_or_else(Kb::absent));
        let model = Arc::new(Client::new(&cfg.model_base, &cfg.model_id));
        let journal = Journal::new(store.events_path());
        let tasks = Arc::new(TaskEngine::new(
            cfg.clone(),
            store.clone(),
            journal.clone(),
            model.clone(),
        ));
        let state = Arc::new(AppState {
            cfg,
            model,
            kb,
            store,
            journal: journal.clone(),
            tasks,
            started: Instant::now(),
        });

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test server");
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let thread = thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                tokio::select! {
                    _ = async { axum::serve(listener, router(state)).await } => {}
                    _ = stopped => {}
                }
            });
        });
        TestServer {
            addr,
            journal,
            stop: Some(stop),
            thread: Some(thread),
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A loopback base URL nothing is listening on (bind, note the port, close).
fn dead_base() -> String {
    let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    format!("http://{}", l.local_addr().unwrap())
}

/// A model stand-in whose `/health` answers 200 (llama-server's probe).
fn live_model() -> MockServer {
    MockServer::start(|_req, stream| respond(stream, 200, "OK", "application/json", "{}"))
}

struct Resp {
    status: u16,
    head: String,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("body is not JSON ({e}): {:?}", self.body))
    }
}

/// One-shot HTTP/1.1 request with an explicit Host header.
fn http(addr: SocketAddr, method: &str, path: &str, host: &str) -> Resp {
    let mut s = TcpStream::connect(addr).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).expect("read response");
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("response head");
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .expect("status code");
    Resp {
        status,
        head: head.to_string(),
        body: body.to_string(),
    }
}

fn get(srv: &TestServer, path: &str) -> Resp {
    http(srv.addr, "GET", path, "localhost:11182")
}

// ----------------------------------------------------------------- /health

#[test]
fn health_is_200_degraded_when_model_down_but_state_dir_ok() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = TestServer::start(&tmp.path().join("state"), &dead_base());
    let r = get(&srv, "/health");
    assert_eq!(r.status, 200, "a dead model must not fail supervision");
    assert_eq!(
        r.json(),
        json!({
            "schema": 1,
            "status": "degraded",
            "components": {"model": false, "kb": false, "state_dir": true}
        })
    );
}

#[test]
fn health_ok_when_everything_is_up() {
    let tmp = tempfile::tempdir().unwrap();
    let model = live_model();
    let kb = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kb");
    let srv = TestServer::start_with_kb(&tmp.path().join("state"), &model.url(), Some(&kb));
    let r = get(&srv, "/health");
    assert_eq!(r.status, 200);
    assert_eq!(
        r.json(),
        json!({
            "schema": 1,
            "status": "ok",
            "components": {"model": true, "kb": true, "state_dir": true}
        })
    );
}

#[test]
fn health_is_503_error_when_state_dir_unusable() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    let srv = TestServer::start(&state, &dead_base());
    std::fs::remove_dir_all(&state).unwrap();
    let r = get(&srv, "/health");
    assert_eq!(r.status, 503);
    assert_eq!(r.json()["status"], "error");
    assert_eq!(r.json()["components"]["state_dir"], false);
}

// -------------------------------------------------------- /v1/agent/status

#[test]
fn status_is_unavailable_without_a_model_listener() {
    let tmp = tempfile::tempdir().unwrap();
    let base = dead_base();
    let srv = TestServer::start(&tmp.path().join("state"), &base);
    let r = get(&srv, "/v1/agent/status");
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["state"], "unavailable");
    assert_eq!(v["model"]["reachable"], false);
    assert_eq!(v["model"]["base"], base);
    assert_eq!(v["model"]["id"], "qwen3-4b");
    assert_eq!(v["kb"], json!({"present": false, "entries": null}));
}

#[test]
fn status_is_local_with_a_live_loopback_model_and_full_shape() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    let model = live_model();
    let kb = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kb");
    let srv = TestServer::start_with_kb(&state, &model.url(), Some(&kb));
    std::fs::write(state.join("conversations").join("c1.jsonl"), "{}\n").unwrap();

    let r = get(&srv, "/v1/agent/status");
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["state"], "local");
    assert_eq!(v["model"]["reachable"], true);
    assert_eq!(v["model"]["base"], model.url());
    assert_eq!(v["kb"]["present"], true);
    assert_eq!(v["kb"]["entries"], 3);
    assert_eq!(v["conversations"], 1);
    assert_eq!(v["tasks_open"], 0);
    assert!(v["uptime_sec"].is_u64());
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    // additive fields
    assert_eq!(v["node_id"], "ab12cd34");
    assert_eq!(v["category"], "2A");
}

// --------------------------------------------------------------------- SSE

struct SseClient {
    stream: TcpStream,
    raw: Vec<u8>,
    scanned: usize,
    frames: std::collections::VecDeque<Value>,
}

const SSE_DEADLINE: Duration = Duration::from_secs(5);

impl SseClient {
    fn connect(addr: SocketAddr) -> Self {
        let mut stream = TcpStream::connect(addr).expect("connect sse");
        stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        write!(
            stream,
            "GET /v1/agent/events HTTP/1.1\r\nHost: localhost:11182\r\nAccept: text/event-stream\r\n\r\n"
        )
        .unwrap();
        let mut c = SseClient {
            stream,
            raw: Vec::new(),
            scanned: 0,
            frames: Default::default(),
        };
        c.fill(|c| c.head_end().is_some());
        let head_end = c.head_end().unwrap();
        let head = String::from_utf8_lossy(&c.raw[..head_end]).to_lowercase();
        assert!(head.starts_with("http/1.1 200"), "bad status: {head}");
        assert!(head.contains("text/event-stream"), "bad type: {head}");
        c.scanned = head_end;
        c.parse_frames(); // frames that arrived in the same read as the head
        c
    }

    fn head_end(&self) -> Option<usize> {
        self.raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| p + 4)
    }

    /// Read until `done` holds, failing (never hanging) at the deadline.
    fn fill(&mut self, done: impl Fn(&SseClient) -> bool) {
        let deadline = Instant::now() + SSE_DEADLINE;
        let mut buf = [0u8; 8192];
        while !done(self) {
            assert!(
                Instant::now() < deadline,
                "SSE deadline; got {:?}, raw: {:?}",
                self.frames,
                String::from_utf8_lossy(&self.raw)
            );
            match self.stream.read(&mut buf) {
                Ok(0) => panic!("SSE stream closed early"),
                Ok(n) => {
                    self.raw.extend_from_slice(&buf[..n]);
                    self.parse_frames();
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(e) => panic!("SSE read failed: {e}"),
            }
        }
    }

    /// Pull complete `data: ` lines out of the raw bytes. Chunked-transfer
    /// size lines and keep-alive comments never start with `data: `.
    fn parse_frames(&mut self) {
        if self.scanned == 0 {
            return; // head not complete yet
        }
        while let Some(rel) = self.raw[self.scanned..].iter().position(|b| *b == b'\n') {
            let line = String::from_utf8_lossy(&self.raw[self.scanned..self.scanned + rel])
                .trim_end_matches('\r')
                .to_string();
            self.scanned += rel + 1;
            if let Some(payload) = line.strip_prefix("data: ") {
                self.frames
                    .push_back(serde_json::from_str(payload).expect("frame is JSON"));
            }
        }
    }

    /// The next `n` frames, in arrival order.
    fn next(&mut self, n: usize) -> Vec<Value> {
        self.fill(|c| c.frames.len() >= n);
        self.frames.drain(..n).collect()
    }
}

fn seqs(frames: &[Value]) -> Vec<u64> {
    frames.iter().map(|f| f["seq"].as_u64().unwrap()).collect()
}

#[test]
fn events_sse_replays_history_then_streams_live() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = TestServer::start(&tmp.path().join("state"), &dead_base());
    for i in 1..=3 {
        srv.journal.append(&format!("t.{i}"), json!({"i": i}));
    }

    let mut sse = SseClient::connect(srv.addr);
    let replay = sse.next(3);
    assert_eq!(seqs(&replay), [1, 2, 3]);
    assert_eq!(replay[0]["kind"], "t.1");
    assert_eq!(replay[2]["detail"], json!({"i": 3}));
    assert!(replay.iter().all(|f| f["schema"] == 1 && f["ts"] != ""));

    srv.journal.append("t.live", json!({"live": true}));
    let live = sse.next(1);
    assert_eq!(seqs(&live), [4]);
    assert_eq!(live[0]["kind"], "t.live");
    assert_eq!(live[0]["detail"], json!({"live": true}));
}

#[test]
fn events_sse_replay_is_capped_at_the_last_50_in_order() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = TestServer::start(&tmp.path().join("state"), &dead_base());
    for i in 1..=60 {
        srv.journal.append("n", json!({"i": i}));
    }

    let mut sse = SseClient::connect(srv.addr);
    let replay = sse.next(50);
    assert_eq!(seqs(&replay), (11..=60).collect::<Vec<u64>>());

    // the very next frame is the live one: no stray 51st replay frame
    srv.journal.append("n", json!({"i": 61}));
    assert_eq!(seqs(&sse.next(1)), [61]);
}

// ------------------------------------------------------------- Host guard

#[test]
fn host_guard_refuses_foreign_hosts_and_allows_loopback_names() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = TestServer::start(&tmp.path().join("state"), &dead_base());

    for bad in ["evil.example", "evil.example:11182", "127.0.0.1.evil.com"] {
        let r = http(srv.addr, "GET", "/health", bad);
        assert_eq!(r.status, 403, "Host {bad} must be refused");
        assert_eq!(r.json(), json!({"schema": 1, "error": "host not allowed"}));
    }
    for good in [
        "localhost:11182",
        "localhost",
        "127.0.0.1:4242",
        "[::1]:11182",
    ] {
        let r = http(srv.addr, "GET", "/health", good);
        assert_eq!(r.status, 200, "Host {good} must be allowed");
    }
    // the guard covers every route, not just /health
    assert_eq!(
        http(srv.addr, "GET", "/v1/agent/status", "evil.example").status,
        403
    );
    assert_eq!(
        http(srv.addr, "GET", "/v1/agent/events", "evil.example").status,
        403
    );
}

// --------------------------------------------------------------- GET-only

#[test]
fn non_get_methods_on_get_routes_are_405() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = TestServer::start(&tmp.path().join("state"), &dead_base());

    for path in [
        "/health",
        "/v1/agent/status",
        "/v1/agent/events",
        "/v1/chat/history",
    ] {
        for method in ["POST", "PUT", "DELETE", "PATCH"] {
            let r = http(srv.addr, method, path, "localhost:11182");
            assert_eq!(r.status, 405, "{method} {path} must be 405");
            assert!(
                r.head.to_lowercase().contains("allow: get"),
                "{method} {path}: 405 should advertise GET, head was {:?}",
                r.head
            );
        }
    }
}
