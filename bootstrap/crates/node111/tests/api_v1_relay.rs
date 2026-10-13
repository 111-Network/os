//! Layer 4 R3 acceptance (os/interface/README.md §10, §10.1; api/README.md):
//! the `111d` → `111-agent` relay against a scripted stub agent (std
//! `TcpListener`, raw HTTP/1.1 — TASKS.md bars test-server crates).
//!
//! - chat stream translation byte-exact, with upstream chunk boundaries that
//!   split a JSON line and a UTF-8 sequence; error mid-stream; upstream drop;
//!   client disconnect cancels the upstream turn
//! - pre-stream failures as problem+json: 503 model down, 422/400
//!   normalization, agent absent / connection refused
//! - `stream:false`, history, status, task pass-through (+ 409 needs hub)
//! - events subscriber: replay dedupe across a forced reconnect, `agent` and
//!   `proposal` frames on the bus
//! - proposals list/get/decision (200/409/404/400)
//!
//! The relay follows `/api/status.intelligence.agent_url` (set here to the
//! stub's port) and reads its timeouts/backoff from an `Arc<RelayConfig>` in
//! `ApiState::ext`.

use node111::daemon::dashboard::api_v1::relay::{self, RelayConfig};
use node111::daemon::dashboard::{
    routes, serve_listener, ApiConfig, ApiState, DashboardState, SharedState,
};
use node111::daemon::intelligence::{ChildDisplay, IntelligenceState};
use node111::daemon::DaemonLayout;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- stub agent

#[derive(Debug, Clone)]
struct StubReq {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl StubReq {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Clone, Default)]
struct Stub {
    port: u16,
    log: Arc<Mutex<Vec<StubReq>>>,
    /// The `slow` chat stream saw its connection closed by the relay.
    cancelled: Arc<AtomicBool>,
    events_conns: Arc<AtomicUsize>,
}

impl Stub {
    fn requests(&self) -> Vec<StubReq> {
        self.log.lock().unwrap().clone()
    }
}

fn read_request(conn: &mut TcpStream) -> Option<StubReq> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if conn.read(&mut byte).ok()? == 0 {
            return None;
        }
        buf.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&buf).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let path = first.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    conn.read_exact(&mut body).ok()?;
    Some(StubReq {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn reply(conn: &mut TcpStream, status: &str, ctype: &str, body: &str) {
    let msg = format!(
        "HTTP/1.1 {status}\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = conn.write_all(msg.as_bytes());
}

fn sse_start(conn: &mut TcpStream) {
    let _ = conn.write_all(
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\ntransfer-encoding: chunked\r\n\r\n",
    );
}

/// One HTTP chunk, then a pause so the relay sees it as a separate read.
fn chunk(conn: &mut TcpStream, data: &[u8]) -> std::io::Result<()> {
    conn.write_all(format!("{:x}\r\n", data.len()).as_bytes())?;
    conn.write_all(data)?;
    conn.write_all(b"\r\n")?;
    conn.flush()?;
    std::thread::sleep(Duration::from_millis(25));
    Ok(())
}

fn sse_end(conn: &mut TcpStream) {
    let _ = conn.write_all(b"0\r\n\r\n");
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap()
}

fn agent_frame(seq: u64, ts: &str, kind: &str, detail: Value) -> Vec<u8> {
    let v = json!({"schema": 1, "seq": seq, "ts": ts, "kind": kind, "detail": detail});
    format!("data: {v}\n\n").into_bytes()
}

const OLD_TS: &str = "2026-01-01T00:00:00Z";

fn stub_chat(stub: &Stub, conn: &mut TcpStream, req: &StubReq) {
    let body: Value = serde_json::from_str(&req.body).unwrap_or(Value::Null);
    let message = body["message"].as_str().unwrap_or("");
    let stream = body["stream"].as_bool().unwrap_or(true);
    if !stream {
        return match message {
            "crash" => reply(
                conn,
                "500 Internal Server Error",
                "application/json",
                r#"{"schema":1,"error":"could not save conversation: disk full"}"#,
            ),
            _ => reply(
                conn,
                "200 OK",
                "application/json",
                r#"{"schema":1,"conversation_id":"conv-1","reply":"Hello","tool_trace":[]}"#,
            ),
        };
    }
    match message {
        "hello" => {
            sse_start(conn);
            // "lo naïve" frame split across three chunks: inside the JSON
            // line, then inside the UTF-8 sequence of "ï".
            let line2 = "data: {\"type\":\"delta\",\"text\":\"lo na\u{ef}ve\"}\n\n".as_bytes();
            let cut_json = 14;
            let cut_utf8 = line2.iter().position(|&b| b == 0xc3).unwrap() + 1;
            let mut c1 = b"data: {\"type\":\"delta\",\"text\":\"Hel\"}\n\n".to_vec();
            c1.extend_from_slice(&line2[..cut_json]);
            let _ = chunk(conn, &c1);
            let _ = chunk(conn, &line2[cut_json..cut_utf8]);
            let mut c3 = line2[cut_utf8..].to_vec();
            c3.extend_from_slice(b":\n\ndata: {\"type\":\"tool_call\",\"name\":\"kb.read\"}\n\n");
            let _ = chunk(conn, &c3);
            let _ = chunk(conn, b"data: {\"type\":\"delta\",\"text\":\"!\"}\n\n");
            let _ = chunk(
                conn,
                "data: {\"type\":\"done\",\"conversation_id\":\"conv-1\",\"reply\":\"Hello na\u{ef}ve!\",\"tool_trace\":[{\"name\":\"kb.read\",\"ok\":true}]}\n\n".as_bytes(),
            );
            let _ = chunk(conn, b"data: [DONE]\n\n");
            sse_end(conn);
        }
        "boom" => {
            sse_start(conn);
            let _ = chunk(conn, b"data: {\"type\":\"delta\",\"text\":\"partial\"}\n\n");
            let _ = chunk(
                conn,
                b"data: {\"type\":\"error\",\"error\":\"model call failed: HTTP 500\"}\n\n",
            );
            sse_end(conn);
        }
        "drop" => {
            sse_start(conn);
            let _ = chunk(conn, b"data: {\"type\":\"delta\",\"text\":\"partial\"}\n\n");
            // connection closes without a terminal frame or final chunk
        }
        "slow" => {
            sse_start(conn);
            let _ = chunk(conn, b"data: {\"type\":\"delta\",\"text\":\"tick\"}\n\n");
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(10) {
                if chunk(conn, b":\n\n").is_err() {
                    stub.cancelled.store(true, Ordering::SeqCst);
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        "model-down" => reply(
            conn,
            "503 Service Unavailable",
            "application/json",
            r#"{"schema":1,"error":"model unavailable"}"#,
        ),
        "reject-422" => reply(
            conn,
            "422 Unprocessable Entity",
            "text/plain; charset=utf-8",
            "Failed to deserialize the JSON body into the target type: missing field `message` at line 1 column 2",
        ),
        "reject-400" => reply(
            conn,
            "400 Bad Request",
            "application/json",
            r#"{"schema":1,"error":"invalid conversation_id"}"#,
        ),
        _ => reply(conn, "500 Internal Server Error", "text/plain", "unscripted"),
    }
}

fn stub_events(stub: &Stub, conn: &mut TcpStream) {
    let n = stub.events_conns.fetch_add(1, Ordering::SeqCst);
    sse_start(conn);
    // every connection replays the journal tail first
    let mut replay = agent_frame(
        1,
        OLD_TS,
        "chat.turn",
        json!({"conversation_id": "conv-0", "tools": 0}),
    );
    // split the second frame across chunks too
    let f2 = agent_frame(
        2,
        OLD_TS,
        "proposal.written",
        json!({"proposal_id": "prop-old", "kind": "chat", "title": "old"}),
    );
    replay.extend_from_slice(&f2[..20]);
    let _ = chunk(conn, &replay);
    let _ = chunk(conn, &f2[20..]);
    if n == 0 {
        // first connection: drop after the replay (forced reconnect)
        sse_end(conn);
        return;
    }
    let now = now_rfc3339();
    let _ = chunk(
        conn,
        &agent_frame(
            3,
            &now,
            "proposal.written",
            json!({"proposal_id": "prop-new", "kind": "chat", "title": "fresh"}),
        ),
    );
    let _ = chunk(
        conn,
        &agent_frame(
            4,
            &now,
            "task.done",
            json!({"task_id": "task-1", "kind": "diagnose", "proposal_id": "prop-task"}),
        ),
    );
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        if chunk(conn, b": keep-alive\n\n").is_err() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn stub_handle(stub: &Stub, mut conn: TcpStream) {
    let _ = conn.set_nodelay(true);
    let Some(req) = read_request(&mut conn) else {
        return;
    };
    stub.log.lock().unwrap().push(req.clone());
    match (req.method.as_str(), req.path.as_str()) {
        ("POST", "/v1/chat") => stub_chat(stub, &mut conn, &req),
        ("GET", "/v1/chat/history?conversation_id=conv-1") => reply(
            &mut conn,
            "200 OK",
            "application/json",
            r#"{"schema":1,"conversation_id":"conv-1","messages":[{"role":"user","content":"hi"},{"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"kb.read","arguments":"{}"}}]},{"role":"tool","content":"{}","tool_call_id":"c1"},{"role":"assistant","content":"Hello"}]}"#,
        ),
        ("GET", "/v1/agent/status") => reply(
            &mut conn,
            "200 OK",
            "application/json",
            r#"{"schema":1,"state":"local","model":{"base":"http://127.0.0.1:11181","reachable":true,"id":"qwen"},"kb":{"present":true,"entries":42},"conversations":3,"tasks_open":0,"uptime_sec":17,"version":"0.1.0","node_id":null,"category":"Node2A"}"#,
        ),
        ("POST", "/v1/agent/task") => {
            if req.body.contains("\"patch\"") {
                reply(
                    &mut conn,
                    "409 Conflict",
                    "application/json",
                    r#"{"schema":1,"error":"needs hub"}"#,
                )
            } else {
                reply(
                    &mut conn,
                    "202 Accepted",
                    "application/json",
                    r#"{"schema":1,"task_id":"task-18f2-0001"}"#,
                )
            }
        }
        ("GET", "/v1/agent/events") => stub_events(stub, &mut conn),
        _ => reply(&mut conn, "404 Not Found", "text/plain", "nope"),
    }
}

fn spawn_stub() -> Stub {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let stub = Stub {
        port: listener.local_addr().unwrap().port(),
        ..Stub::default()
    };
    let s = stub.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { break };
            let s = s.clone();
            std::thread::spawn(move || stub_handle(&s, conn));
        }
    });
    stub
}

/// A port nothing listens on.
fn dead_port() -> u16 {
    let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    l.local_addr().unwrap().port()
}

// ---------------------------------------------------------------- 111d under test

struct Node {
    api: Arc<ApiState>,
    port: u16,
    dir: tempfile::TempDir,
}

fn relay_config() -> RelayConfig {
    RelayConfig {
        backoff_min: Duration::from_millis(50),
        backoff_max: Duration::from_millis(200),
        absent_poll: Duration::from_millis(50),
        events_idle: Duration::from_secs(5),
        ..RelayConfig::default()
    }
}

async fn node(agent: ChildDisplay, agent_port: Option<u16>) -> Node {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let mut dash = DashboardState::new("2026.10.13");
    dash.intelligence = IntelligenceState {
        agent,
        model_server: ChildDisplay::Running,
        agent_url: agent_port.map(|p| format!("http://127.0.0.1:{p}")),
    };
    let dash: SharedState = Arc::new(RwLock::new(dash));
    let api = ApiState::new(dash, ApiConfig::new(layout, false));
    api.insert_ext(Arc::new(relay_config()));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve_listener(listener, routes::app(Arc::clone(&api))));
    Node { api, port, dir }
}

async fn running(stub: &Stub) -> Node {
    node(ChildDisplay::Running, Some(stub.port)).await
}

// ---------------------------------------------------------------- client

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {:?}", self.body))
    }
    /// Assert an RFC 9457 problem with this status + code; returns it.
    fn problem(&self, status: u16, code: &str) -> Value {
        assert_eq!(self.status, status, "body: {}", self.body);
        assert_eq!(
            self.header("content-type"),
            Some("application/problem+json"),
            "{}",
            self.body
        );
        let p = self.json();
        assert_eq!(p["code"], code, "{p}");
        assert_eq!(p["status"], status);
        assert!(p["title"].is_string() && p["type"].is_string(), "{p}");
        p
    }
}

fn dechunk(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = raw;
    while let Some(nl) = rest.windows(2).position(|w| w == b"\r\n") {
        let size = std::str::from_utf8(&rest[..nl]).unwrap_or("0");
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        rest = &rest[nl + 2..];
        if n == 0 || rest.len() < n {
            break;
        }
        out.extend_from_slice(&rest[..n]);
        rest = rest[n..].strip_prefix(b"\r\n").unwrap_or(&rest[n..]);
    }
    out
}

fn request(port: u16, method: &str, path: &str, body: Option<&str>) -> Resp {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    conn.set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    if let Some(b) = body {
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    req.push_str("\r\n");
    req.push_str(body.unwrap_or(""));
    conn.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    conn.read_to_end(&mut raw).unwrap();
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or(raw.len());
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let body = raw.get(split + 4..).unwrap_or(&[]);
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let chunked = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked")
    });
    let body = if chunked {
        dechunk(body)
    } else {
        body.to_vec()
    };
    Resp {
        status,
        headers,
        body: String::from_utf8(body).expect("UTF-8 body"),
    }
}

fn get(port: u16, path: &str) -> Resp {
    request(port, "GET", path, None)
}

fn post(port: u16, path: &str, body: &str) -> Resp {
    request(port, "POST", path, Some(body))
}

fn chat(port: u16, message: &str) -> Resp {
    post(
        port,
        "/api/v1/chat",
        &json!({"conversation_id": "conv-1", "message": message}).to_string(),
    )
}

fn assert_sse(r: &Resp) {
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.header("content-type"), Some("text/event-stream"));
    assert_eq!(r.header("cache-control"), Some("no-cache"));
    assert_eq!(r.header("x-accel-buffering"), Some("no"));
}

/// Frames on the bus since its base: `(event, data)`.
fn bus(api: &ApiState) -> Vec<(String, Value)> {
    api.events
        .subscribe(Some(api.events.base()))
        .replay
        .iter()
        .map(|f| (f.event.clone(), serde_json::from_str(&f.data).unwrap()))
        .collect()
}

fn write_proposal(root: &std::path::Path, id: &str, title: &str, created_at: &str) {
    let dir = root.join("agent/proposals");
    std::fs::create_dir_all(&dir).unwrap();
    let p = json!({
        "schema": 1, "id": id, "node_id": "ab".repeat(32), "task_id": null,
        "kind": "chat", "title": title, "body": "## Summary\nDo the thing.", "created_at": created_at
    });
    std::fs::write(
        dir.join(format!("{id}.json")),
        serde_json::to_string_pretty(&p).unwrap(),
    )
    .unwrap();
}

// ---------------------------------------------------------------- chat

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chat_stream_translates_byte_exact() {
    let stub = spawn_stub();
    let n = running(&stub).await;
    let r = chat(n.port, "hello");
    assert_sse(&r);
    assert_eq!(
        r.body,
        "event: token\ndata: {\"text\":\"Hel\"}\n\n\
         event: token\ndata: {\"text\":\"lo na\u{ef}ve\"}\n\n\
         : keep-alive\n\n\
         event: tool\ndata: {\"name\":\"kb.read\"}\n\n\
         event: token\ndata: {\"text\":\"!\"}\n\n\
         event: done\ndata: {\"conversation_id\":\"conv-1\",\"reply\":\"Hello na\u{ef}ve!\",\"tool_trace\":[{\"name\":\"kb.read\",\"ok\":true}]}\n\n"
    );
    // what reached the agent: loopback Host, JSON, the validated request
    let reqs = stub.requests();
    assert_eq!(reqs.len(), 1);
    let up = &reqs[0];
    assert_eq!(
        up.header("host"),
        Some(format!("127.0.0.1:{}", stub.port).as_str())
    );
    assert_eq!(up.header("content-type"), Some("application/json"));
    assert_eq!(
        serde_json::from_str::<Value>(&up.body).unwrap(),
        json!({"conversation_id": "conv-1", "message": "hello", "stream": true})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chat_stream_error_mid_stream_and_upstream_drop() {
    let stub = spawn_stub();
    let n = running(&stub).await;
    let r = chat(n.port, "boom");
    assert_sse(&r);
    assert_eq!(
        r.body,
        "event: token\ndata: {\"text\":\"partial\"}\n\n\
         event: error\ndata: {\"error\":\"model call failed: HTTP 500\",\"code\":\"upstream_error\"}\n\n"
    );
    let r = chat(n.port, "drop");
    assert_sse(&r);
    assert_eq!(
        r.body,
        "event: token\ndata: {\"text\":\"partial\"}\n\n\
         event: error\ndata: {\"error\":\"stream ended\",\"code\":\"upstream_error\"}\n\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chat_client_disconnect_cancels_upstream() {
    let stub = spawn_stub();
    let n = running(&stub).await;
    let mut conn = TcpStream::connect(("127.0.0.1", n.port)).unwrap();
    conn.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let body = json!({"conversation_id": "conv-1", "message": "slow"}).to_string();
    let req = format!(
        "POST /api/v1/chat HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    conn.write_all(req.as_bytes()).unwrap();
    let mut seen = Vec::new();
    let mut buf = [0u8; 1024];
    while !String::from_utf8_lossy(&seen).contains("event: token") {
        let k = conn.read(&mut buf).unwrap();
        assert!(k > 0, "relay closed early");
        seen.extend_from_slice(&buf[..k]);
    }
    assert!(!stub.cancelled.load(Ordering::SeqCst));
    drop(conn);
    let start = Instant::now();
    while !stub.cancelled.load(Ordering::SeqCst) {
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "upstream turn was not cancelled"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chat_pre_stream_failures_are_problems() {
    let stub = spawn_stub();
    let n = running(&stub).await;

    let p = chat(n.port, "model-down").problem(503, "model_unavailable");
    assert_eq!(p["detail"], "111-agent: model unavailable");

    let p = chat(n.port, "reject-422").problem(400, "bad_request");
    assert_eq!(
        p["detail"],
        "Failed to deserialize the JSON body into the target type: missing field `message` at line 1 column 2"
    );
    let p = chat(n.port, "reject-400").problem(400, "bad_request");
    assert_eq!(p["detail"], "invalid conversation_id");

    // stream:false: JSON pass-through, and a 5xx is upstream_error
    let r = post(
        n.port,
        "/api/v1/chat",
        r#"{"conversation_id":"conv-1","message":"hello","stream":false}"#,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.header("content-type"), Some("application/json"));
    assert_eq!(
        r.json(),
        json!({"schema":1,"conversation_id":"conv-1","reply":"Hello","tool_trace":[]})
    );
    let p = post(
        n.port,
        "/api/v1/chat",
        r#"{"conversation_id":"conv-1","message":"crash","stream":false}"#,
    )
    .problem(502, "upstream_error");
    assert!(p["detail"].as_str().unwrap().contains("disk full"), "{p}");

    // contract validation happens before the relay dials
    let before = stub.requests().len();
    for bad in [
        r#"{"message":"hi"}"#,
        r#"{"conversation_id":"../x","message":"hi"}"#,
        r#"{"conversation_id":"c","message":""}"#,
        r#"{"conversation_id":"c","message":"hi","stream":"yes"}"#,
        "not json",
    ] {
        post(n.port, "/api/v1/chat", bad).problem(400, "bad_request");
    }
    let long = json!({"conversation_id": "c", "message": "x".repeat(16_001)}).to_string();
    post(n.port, "/api/v1/chat", &long).problem(400, "bad_request");
    assert_eq!(
        stub.requests().len(),
        before,
        "invalid requests never reach the agent"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_absent_or_refusing_is_agent_unavailable() {
    let stub = spawn_stub();
    for state in [
        ChildDisplay::Absent,
        ChildDisplay::Backoff,
        ChildDisplay::Degraded,
        ChildDisplay::Stopped,
    ] {
        let n = node(state, Some(stub.port)).await;
        chat(n.port, "hello").problem(503, "agent_unavailable");
        get(n.port, "/api/v1/agent/status").problem(503, "agent_unavailable");
        get(n.port, "/api/v1/chat/history?conversation_id=conv-1")
            .problem(503, "agent_unavailable");
        post(n.port, "/api/v1/agent/task", r#"{"kind":"diagnose"}"#)
            .problem(503, "agent_unavailable");
    }
    assert!(
        stub.requests().is_empty(),
        "a non-running agent is never dialed"
    );

    // running per the supervisor, but nothing listens (start-up race, crash)
    let n = node(ChildDisplay::Running, Some(dead_port())).await;
    let p = chat(n.port, "hello").problem(503, "agent_unavailable");
    assert!(
        p["detail"].as_str().unwrap().starts_with("Cannot connect"),
        "{p}"
    );
    get(n.port, "/api/v1/agent/status").problem(503, "agent_unavailable");

    // the RelayConfig base override wins over intelligence.agent_url
    n.api.insert_ext(Arc::new(RelayConfig {
        base: Some(format!("http://127.0.0.1:{}", stub.port)),
        ..relay_config()
    }));
    assert_eq!(get(n.port, "/api/v1/agent/status").status, 200);
}

// ---------------------------------------------------------------- pass-through

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn history_status_task_pass_through() {
    let stub = spawn_stub();
    let n = running(&stub).await;

    let r = get(n.port, "/api/v1/chat/history?conversation_id=conv-1");
    assert_eq!(r.status, 200, "{}", r.body);
    let h = r.json();
    assert_eq!(h["conversation_id"], "conv-1");
    assert_eq!(h["messages"].as_array().unwrap().len(), 4);
    assert_eq!(h["messages"][1]["content"], Value::Null);
    get(n.port, "/api/v1/chat/history").problem(400, "bad_request");
    get(n.port, "/api/v1/chat/history?conversation_id=a.b").problem(400, "bad_request");

    let r = get(n.port, "/api/v1/agent/status");
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["state"], "local");
    assert_eq!(r.json()["kb"]["entries"], 42);

    let r = post(
        n.port,
        "/api/v1/agent/task",
        r#"{"kind":"diagnose","refs":["services/agent"]}"#,
    );
    assert_eq!(r.status, 202, "{}", r.body);
    assert_eq!(r.json(), json!({"schema": 1, "task_id": "task-18f2-0001"}));
    let fwd = stub
        .requests()
        .into_iter()
        .rfind(|q| q.path == "/v1/agent/task")
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&fwd.body).unwrap(),
        json!({"kind": "diagnose", "refs": ["services/agent"], "note": null})
    );
    let p = post(
        n.port,
        "/api/v1/agent/task",
        r#"{"kind":"patch","note":"fix it"}"#,
    )
    .problem(409, "needs_hub");
    assert_eq!(p["detail"], "needs hub");
    post(n.port, "/api/v1/agent/task", r#"{"kind":"shell"}"#).problem(400, "bad_request");
}

// ---------------------------------------------------------------- events

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn events_republished_deduped_across_reconnect() {
    let stub = spawn_stub();
    let n = running(&stub).await;
    let now = now_rfc3339();
    write_proposal(n.dir.path(), "prop-old", "old", OLD_TS);
    write_proposal(n.dir.path(), "prop-new", "Fresh idea", &now);
    write_proposal(n.dir.path(), "prop-task", "Diagnosis", &now);
    let handle = relay::start_events(&n.api);

    let start = Instant::now();
    let frames = loop {
        let frames = bus(&n.api);
        if frames.iter().filter(|(e, _)| e == "proposal").count() >= 2 {
            break frames;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "bus so far: {frames:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    // let a few keep-alives and any stray duplicate land
    tokio::time::sleep(Duration::from_millis(300)).await;
    let frames = {
        let later = bus(&n.api);
        assert_eq!(
            later.len(),
            frames.len(),
            "nothing more after the live frames: {later:?}"
        );
        later
    };
    handle.abort();
    assert!(
        stub.events_conns.load(Ordering::SeqCst) >= 2,
        "forced reconnect happened"
    );

    let seqs: Vec<u64> = frames
        .iter()
        .filter(|(e, _)| e == "agent")
        .map(|(_, d)| d["seq"].as_u64().unwrap())
        .collect();
    assert_eq!(seqs, [1, 2, 3, 4], "each upstream seq exactly once");
    let agent3 = &frames
        .iter()
        .find(|(e, d)| e == "agent" && d["seq"] == 3)
        .unwrap()
        .1;
    assert_eq!(agent3["kind"], "proposal.written");
    assert_eq!(agent3["detail"]["proposal_id"], "prop-new");
    assert_eq!(agent3["schema"], 1);

    let props: Vec<&Value> = frames
        .iter()
        .filter(|(e, _)| e == "proposal")
        .map(|(_, d)| d)
        .collect();
    assert_eq!(
        props,
        [
            &json!({"id": "prop-new", "title": "Fresh idea", "kind": "chat", "created_at": now, "decision": null}),
            &json!({"id": "prop-task", "title": "Diagnosis", "kind": "chat", "created_at": now, "decision": null}),
        ],
        "replayed history (prop-old) does not raise a proposal frame"
    );
    // order on the bus: agent 3, proposal new, agent 4, proposal task
    let order: Vec<String> = frames
        .iter()
        .map(|(e, d)| match e.as_str() {
            "agent" => format!("agent:{}", d["seq"]),
            _ => format!("{e}:{}", d["id"].as_str().unwrap_or("")),
        })
        .collect();
    assert_eq!(
        order,
        [
            "agent:1",
            "agent:2",
            "agent:3",
            "proposal:prop-new",
            "agent:4",
            "proposal:prop-task"
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn events_subscriber_idles_while_agent_absent() {
    let stub = spawn_stub();
    let n = node(ChildDisplay::Absent, Some(stub.port)).await;
    let handle = relay::start_events(&n.api);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        stub.events_conns.load(Ordering::SeqCst),
        0,
        "absent agent is never dialed"
    );
    // the child comes up → the subscriber connects
    n.api.dash.write().unwrap().intelligence.agent = ChildDisplay::Running;
    let start = Instant::now();
    while bus(&n.api).len() < 2 {
        assert!(start.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    handle.abort();
}

// ---------------------------------------------------------------- proposals

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn proposals_list_get_and_decide() {
    let n = node(ChildDisplay::Absent, None).await;
    let root = n.dir.path();
    // no proposals dir yet → empty list
    assert_eq!(
        get(n.port, "/api/v1/proposals").json(),
        json!({"schema": 1, "proposals": []})
    );
    write_proposal(root, "prop-a", "First", "2026-10-01T10:00:00Z");
    write_proposal(root, "prop-b", "Second", "2026-10-02T10:00:00Z");
    std::fs::write(root.join("agent/proposals/garbage.json"), "{oops").unwrap();

    let list = get(n.port, "/api/v1/proposals").json();
    let ids: Vec<&str> = list["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["prop-b", "prop-a"],
        "open, newest first, unparsable skipped"
    );
    assert_eq!(list["proposals"][0]["decision"], Value::Null);
    assert_eq!(list["proposals"][0]["body"], "## Summary\nDo the thing.");

    let one = get(n.port, "/api/v1/proposals/prop-a");
    assert_eq!(one.status, 200);
    assert_eq!(one.json()["title"], "First");
    get(n.port, "/api/v1/proposals/prop-zzz").problem(404, "not_found");
    get(n.port, "/api/v1/proposals/bad.id").problem(404, "not_found");
    get(n.port, "/api/v1/proposals/..%2F..%2Fstate%2Fdashboard-pair").problem(404, "not_found");
    get(n.port, "/api/v1/proposals?status=bogus").problem(400, "bad_request");

    // decide: 200 → file on disk + bus frame; again → 409; unknown → 404
    let head = n.api.events.head();
    let r = post(
        n.port,
        "/api/v1/proposals/prop-a/decision",
        r#"{"decision":"accepted"}"#,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let p = r.json();
    assert_eq!(p["id"], "prop-a");
    assert_eq!(p["decision"]["decision"], "accepted");
    let at = p["decision"]["at"].as_str().unwrap().to_string();
    let on_disk: Value =
        serde_json::from_slice(&std::fs::read(root.join("agent/decisions/prop-a.json")).unwrap())
            .unwrap();
    assert_eq!(
        on_disk,
        json!({"schema": 1, "decision": "accepted", "at": at})
    );
    let sub = n.api.events.subscribe(Some(head));
    assert_eq!(sub.replay.len(), 1);
    assert_eq!(sub.replay[0].event, "proposal");
    assert_eq!(
        serde_json::from_str::<Value>(&sub.replay[0].data).unwrap(),
        json!({"id": "prop-a", "title": "First", "kind": "chat", "created_at": "2026-10-01T10:00:00Z",
               "decision": {"decision": "accepted", "at": at}})
    );

    let p = post(
        n.port,
        "/api/v1/proposals/prop-a/decision",
        r#"{"decision":"dismissed"}"#,
    )
    .problem(409, "already_decided");
    assert!(p["detail"].as_str().unwrap().contains("accepted"), "{p}");
    post(
        n.port,
        "/api/v1/proposals/prop-nope/decision",
        r#"{"decision":"accepted"}"#,
    )
    .problem(404, "not_found");
    post(
        n.port,
        "/api/v1/proposals/bad.id/decision",
        r#"{"decision":"accepted"}"#,
    )
    .problem(404, "not_found");
    post(
        n.port,
        "/api/v1/proposals/prop-b/decision",
        r#"{"decision":"maybe"}"#,
    )
    .problem(400, "bad_request");
    post(n.port, "/api/v1/proposals/prop-b/decision", "{}").problem(400, "bad_request");
    assert_eq!(
        n.api.events.head(),
        head + 1,
        "only the successful decision published"
    );

    // filters
    let ids = |q: &str| -> Vec<String> {
        get(n.port, &format!("/api/v1/proposals?status={q}")).json()["proposals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(ids("open"), ["prop-b"]);
    assert_eq!(ids("decided"), ["prop-a"]);
    assert_eq!(ids("all"), ["prop-b", "prop-a"]);
}
