//! Model client tests against a std-TcpListener mock OpenAI server.
//! Hermetic: ephemeral loopback ports only, no real network, no fixed
//! ports, no sleeps longer than 1s.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent111::model::{ChatMessage, Client};
use common::{begin_sse, respond, MockServer, Request};
use serde_json::json;

fn messages() -> Vec<ChatMessage> {
    vec![ChatMessage::user("hi")]
}

/// An error message chain flattened to one string for class assertions.
fn err_text(err: &anyhow::Error) -> String {
    format!("{err:#}")
}

/// A URL on an ephemeral port that is guaranteed closed (bound, then the
/// listener is dropped before the client dials it).
fn closed_port_url() -> String {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

/// Handle recording every request the mock server receives.
type Recorded = Arc<Mutex<Vec<Request>>>;

fn recorder() -> (Recorded, impl Fn(Request, &mut std::net::TcpStream)) {
    let recorded: Recorded = Arc::new(Mutex::new(Vec::new()));
    let rec = Arc::clone(&recorded);
    let record = move |req: Request, _sink: &mut std::net::TcpStream| {
        rec.lock().unwrap().push(req);
    };
    (recorded, record)
}

// (a) Happy SSE stream: several content deltas, then [DONE].
#[test]
fn happy_sse_stream() {
    let (recorded, record) = recorder();
    let server = MockServer::start(move |req, sink| {
        record(req, sink);
        let mut sse = begin_sse(sink).unwrap();
        // Role-priming chunk with empty content, as llama-server sends.
        sse.data(r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":""}}]}"#)
            .unwrap();
        sse.data(r#"{"choices":[{"index":0,"delta":{"content":"Hello"}}]}"#)
            .unwrap();
        sse.data(r#"{"choices":[{"index":0,"delta":{"content":", "}}]}"#)
            .unwrap();
        sse.data(r#"{"choices":[{"index":0,"delta":{"content":"world"}}]}"#)
            .unwrap();
        sse.data(r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#)
            .unwrap();
        sse.done().unwrap();
    });

    let client = Client::new(server.url(), "test-model");
    let mut deltas: Vec<String> = Vec::new();
    let reply = client
        .chat_stream(&messages(), &[], &mut |d| deltas.push(d.to_string()))
        .expect("chat stream should succeed");

    // Deltas arrived in order; empty role-priming delta was not delivered.
    assert_eq!(deltas, ["Hello", ", ", "world"]);
    assert_eq!(reply.text, "Hello, world");
    assert!(reply.tool_calls.is_empty());

    // The request hit the OpenAI endpoint with stream:true and no tools key.
    let reqs = recorded.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].method, "POST");
    assert_eq!(reqs[0].path, "/v1/chat/completions");
    let body = reqs[0].body_json();
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["stream"], true);
    assert_eq!(body["messages"][0]["role"], "user");
    assert!(body.get("tools").is_none(), "tools omitted when empty");
    drop(reqs);

    // Non-empty tools are serialized through.
    let tools = vec![json!({"type": "function", "function": {"name": "kb.read"}})];
    let reply2 = client
        .chat_stream(&messages(), &tools, &mut |_| {})
        .expect("second call should succeed");
    assert_eq!(reply2.text, "Hello, world");
    let reqs = recorded.lock().unwrap();
    assert_eq!(reqs.len(), 2);
    let body2 = reqs[1].body_json();
    assert_eq!(body2["tools"][0]["function"]["name"], "kb.read");
}

// (b) Tool-call reply: one call whose arguments JSON arrives in 3 fragments.
#[test]
fn tool_call_arguments_reassembled() {
    let server = MockServer::start(move |_req, sink| {
        let mut sse = begin_sse(sink).unwrap();
        // id + name + first arguments fragment for index 0.
        sse.data(r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_9f","type":"function","function":{"name":"kb.read","arguments":"{\"que"}}]}}]}"#)
            .unwrap();
        // Bare continuation fragments: no id, no name.
        sse.data(r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ry\":\"llm"}}]}}]}"#)
            .unwrap();
        sse.data(r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"s.txt\"}"}}]}}]}"#)
            .unwrap();
        sse.done().unwrap();
    });

    let client = Client::new(server.url(), "test-model");
    let reply = client
        .chat_stream(&messages(), &[], &mut |_| {})
        .expect("chat stream should succeed");

    assert_eq!(reply.text, "");
    assert_eq!(reply.tool_calls.len(), 1);
    let call = &reply.tool_calls[0];
    assert_eq!(call.id, "call_9f");
    assert_eq!(call.name, "kb.read");
    assert_eq!(call.arguments, json!({"query": "llms.txt"}));
}

// A call that never streams an arguments fragment parses as `{}`.
#[test]
fn tool_call_without_arguments_defaults_to_empty_object() {
    let server = MockServer::start(move |_req, sink| {
        let mut sse = begin_sse(sink).unwrap();
        sse.data(r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"health.read"}}]}}]}"#)
            .unwrap();
        sse.done().unwrap();
    });

    let client = Client::new(server.url(), "test-model");
    let reply = client
        .chat_stream(&messages(), &[], &mut |_| {})
        .expect("chat stream should succeed");

    assert_eq!(reply.tool_calls.len(), 1);
    assert_eq!(reply.tool_calls[0].name, "health.read");
    assert_eq!(reply.tool_calls[0].arguments, json!({}));
}

// (c) HTTP 500 with a body: the error names the code and a body snippet.
#[test]
fn http_500_names_code_and_body() {
    let server = MockServer::start(move |_req, sink| {
        respond(
            sink,
            500,
            "Internal Server Error",
            "text/plain",
            "boom: model exploded",
        );
    });

    let client = Client::new(server.url(), "test-model");
    let err = client
        .chat_stream(&messages(), &[], &mut |_| {})
        .expect_err("500 should be an error");
    let text = err_text(&err);
    assert!(text.contains("500"), "error should name the status: {text}");
    assert!(
        text.contains("boom: model exploded"),
        "error should carry the body snippet: {text}"
    );
}

// (d) Timeout: the server accepts then never responds; a ~300ms client
// timeout must turn that into an error (class/message not asserted).
#[test]
fn unresponsive_server_times_out() {
    let server = MockServer::start(move |_req, _sink| {
        // Read the request, then hold the connection forever. The handler
        // thread is detached, so test teardown never joins this park.
        std::thread::park();
    });

    let client = Client::with_timeout(server.url(), "test-model", Duration::from_millis(300));
    let started = Instant::now();
    let result = client.chat_stream(&messages(), &[], &mut |_| {});
    assert!(result.is_err(), "an unresponsive server must error");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "timeout should fire near 300ms, took {:?}",
        started.elapsed()
    );
}

// (e) Malformed SSE: `data: {not json}` is a malformed-class error.
#[test]
fn malformed_sse_is_malformed_class() {
    let server = MockServer::start(move |_req, sink| {
        let mut sse = begin_sse(sink).unwrap();
        sse.data("{not json").unwrap();
        sse.done().unwrap();
    });

    let client = Client::new(server.url(), "test-model");
    let err = client
        .chat_stream(&messages(), &[], &mut |_| {})
        .expect_err("bad SSE JSON should be an error");
    let text = err_text(&err);
    assert!(
        text.contains("malformed"),
        "error should be malformed-class: {text}"
    );
}

// (f) Connection refused (no listener): unreachable-class error.
#[test]
fn connection_refused_is_unreachable_class() {
    let client = Client::with_timeout(closed_port_url(), "test-model", Duration::from_millis(500));
    let err = client
        .chat_stream(&messages(), &[], &mut |_| {})
        .expect_err("a closed port should be an error");
    let text = err_text(&err);
    assert!(
        text.contains("unreachable"),
        "error should be unreachable-class: {text}"
    );
}

// (g) reachable(): 200 ⇒ true, 503 ⇒ false, closed port ⇒ false.
#[test]
fn reachable_probe_matrix() {
    // 200 ⇒ true (and the probe really hits GET /health).
    let (recorded, record) = recorder();
    let healthy = MockServer::start(move |req, sink| {
        record(req, sink);
        respond(sink, 200, "OK", "application/json", r#"{"status":"ok"}"#);
    });
    assert!(Client::new(healthy.url(), "test-model").reachable());
    {
        let reqs = recorded.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].method, "GET");
        assert_eq!(reqs[0].path, "/health");
    }
    drop(healthy);

    // 503 ⇒ false (model still loading; honest "not reachable").
    let loading = MockServer::start(move |_req, sink| {
        respond(
            sink,
            503,
            "Service Unavailable",
            "text/plain",
            "loading model",
        );
    });
    assert!(!Client::new(loading.url(), "test-model").reachable());
    drop(loading);

    // Closed port ⇒ false.
    assert!(
        !Client::with_timeout(closed_port_url(), "test-model", Duration::from_millis(500))
            .reachable()
    );
}
