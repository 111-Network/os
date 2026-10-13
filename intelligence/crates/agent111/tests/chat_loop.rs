//! Chat loop + tools, end to end and hermetic: the real axum router on an
//! ephemeral loopback port, a scripted OpenAI-compatible mock as the model,
//! the fixture KB, and a temp state dir. No fixed ports, no real network.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent111::config::Config;
use agent111::events::Journal;
use agent111::http::{router, AppState};
use agent111::kb::Kb;
use agent111::model::Client;
use agent111::store::Store;
use agent111::tasks::TaskEngine;
use agent111::tools::{self, ToolCtx};
use common::{begin_sse, respond, MockServer};
use serde_json::{json, Value};

// ---------------------------------------------------------------- model mock

#[derive(Clone)]
enum Step {
    Text(&'static str),
    Tool {
        id: &'static str,
        name: &'static str,
        args: &'static str,
    },
    ServerError,
}

/// Scripted model: the nth `/v1/chat/completions` request gets the nth step
/// (the last step repeats forever). `/health` always answers 200.
struct ModelMock {
    server: MockServer,
    chats: Arc<Mutex<Vec<Value>>>,
}

impl ModelMock {
    fn url(&self) -> String {
        self.server.url()
    }
    /// Bodies of every chat-completions request received so far.
    fn chats(&self) -> Vec<Value> {
        self.chats.lock().unwrap().clone()
    }
}

fn model_mock(script: Vec<Step>) -> ModelMock {
    let chats: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&chats);
    let server = MockServer::start(move |req, stream| {
        if req.path == "/health" {
            respond(stream, 200, "OK", "application/json", r#"{"status":"ok"}"#);
            return;
        }
        if req.path != "/v1/chat/completions" {
            respond(stream, 404, "Not Found", "text/plain", "no such route");
            return;
        }
        let n = {
            let mut seen = seen.lock().unwrap();
            seen.push(req.body_json());
            seen.len() - 1
        };
        match script.get(n).or(script.last()).expect("non-empty script") {
            Step::ServerError => {
                respond(stream, 500, "Internal Server Error", "text/plain", "boom")
            }
            Step::Text(text) => {
                let (a, b) = text.split_at(text.len() / 2);
                let mut sse = begin_sse(stream).unwrap();
                for part in [a, b] {
                    let chunk = json!({"choices": [{"index": 0, "delta": {"content": part}}]});
                    sse.data(&chunk.to_string()).unwrap();
                }
                sse.done().unwrap();
            }
            Step::Tool { id, name, args } => {
                let chunk = json!({"choices": [{"index": 0, "delta": {"tool_calls": [{
                    "index": 0, "id": id, "type": "function",
                    "function": {"name": name, "arguments": args},
                }]}}]});
                let mut sse = begin_sse(stream).unwrap();
                sse.data(&chunk.to_string()).unwrap();
                sse.done().unwrap();
            }
        }
    });
    ModelMock { server, chats }
}

/// A URL on an ephemeral port that is guaranteed closed.
fn closed_port_url() -> String {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

fn fixture_kb() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kb")
}

// ------------------------------------------------------------ agent harness

fn build_state(
    model_base: &str,
    kb_dir: &Path,
    daemon_health_url: &str,
    state_root: &Path,
) -> Arc<AppState> {
    let cfg = Arc::new(Config {
        model_base: model_base.to_string(),
        kb_dir: kb_dir.to_path_buf(),
        state_dir: state_root.join("state"),
        category: "test-cat".to_string(),
        daemon_health_url: daemon_health_url.to_string(),
        opencode_path: PathBuf::from("/nonexistent/opencode"),
        node_id: Some("nodeid-abc".to_string()),
        ..Config::default()
    });
    let store = Arc::new(Store::init(&cfg.state_dir).unwrap());
    let kb = Arc::new(Kb::open(&cfg.kb_dir));
    let model = Arc::new(Client::new(&cfg.model_base, &cfg.model_id));
    let journal = Journal::new(store.events_path());
    let tasks = Arc::new(TaskEngine::new(
        cfg.clone(),
        store.clone(),
        journal.clone(),
        model.clone(),
    ));
    Arc::new(AppState {
        cfg,
        model,
        kb,
        store,
        journal,
        tasks,
        started: Instant::now(),
    })
}

struct Resp {
    status: u16,
    content_type: String,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("body is not JSON ({e}): {}", self.body))
    }
    /// The `data:` payloads of an SSE body, in order (comments skipped).
    fn sse_data(&self) -> Vec<String> {
        self.body
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .map(|d| d.trim_start().to_string())
            .collect()
    }
}

struct Agent {
    base: String,
    state: Arc<AppState>,
    http: ureq::Agent,
    _tmp: tempfile::TempDir,
    _rt: tokio::runtime::Runtime,
}

fn start_agent(model_base: &str, kb_dir: &Path) -> Agent {
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        model_base,
        kb_dir,
        &closed_port_url(), // daemon: never reachable unless a test says so
        tmp.path(),
    );
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state.clone());
    rt.spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        axum::serve(listener, app).await.unwrap();
    });
    let http: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    Agent {
        base: format!("http://{addr}"),
        state,
        http,
        _tmp: tmp,
        _rt: rt,
    }
}

impl Agent {
    fn read(mut resp: ureq::http::Response<ureq::Body>) -> Resp {
        Resp {
            status: resp.status().as_u16(),
            content_type: resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string(),
            body: resp.body_mut().read_to_string().unwrap(),
        }
    }

    fn post_chat(&self, body: Value) -> Resp {
        let resp = self
            .http
            .post(&format!("{}/v1/chat", self.base))
            .send_json(&body)
            .unwrap();
        Self::read(resp)
    }

    fn get(&self, path_and_query: &str) -> Resp {
        Self::read(
            self.http
                .get(&format!("{}{path_and_query}", self.base))
                .call()
                .unwrap(),
        )
    }

    /// Stored conversation lines, straight from the JSONL on disk.
    fn conversation_lines(&self, id: &str) -> Vec<Value> {
        std::fs::read_to_string(self.state.store.conversation_path(id))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

fn roles(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect()
}

const GROUNDED: [Step; 2] = [
    Step::Tool {
        id: "call_1",
        name: "kb.read",
        args: r#"{"query":"alpha-project"}"#,
    },
    Step::Text("Alpha routes the mesh."),
];

// ---------------------------------------------------------------- chat tests

#[test]
fn grounded_flow_reads_kb_then_answers() {
    let model = model_mock(GROUNDED.to_vec());
    let agent = start_agent(&model.url(), &fixture_kb());

    let r = agent.post_chat(json!({
        "conversation_id": null, "message": "what is alpha?", "stream": false
    }));
    assert_eq!(r.status, 200, "{}", r.body);
    let body = r.json();
    assert_eq!(body["schema"], 1);
    assert_eq!(body["reply"], "Alpha routes the mesh.");
    assert_eq!(body["tool_trace"], json!([{"name": "kb.read", "ok": true}]));
    let conv_id = body["conversation_id"].as_str().unwrap().to_string();
    assert!(conv_id.starts_with("conv-"), "{conv_id}");

    let chats = model.chats();
    assert_eq!(chats.len(), 2);
    // Request 1: system prompt = preamble + category + KB index; all 5 tools.
    let first = chats[0]["messages"].as_array().unwrap();
    assert_eq!(roles(first), ["system", "user"]);
    let system = first[0]["content"].as_str().unwrap();
    assert!(system.contains("never invent facts"), "{system}");
    assert!(system.contains("Node category: test-cat"), "{system}");
    assert!(
        system.contains("Alpha Project"),
        "KB index missing: {system}"
    );
    assert_eq!(chats[0]["tools"].as_array().unwrap().len(), 5);
    // Request 2: carries the tool result with the fixture entry's marker.
    let second = chats[1]["messages"].as_array().unwrap();
    let tool = second
        .iter()
        .find(|m| m["role"] == "tool")
        .expect("second request has a tool message");
    assert_eq!(tool["tool_call_id"], "call_1");
    assert!(tool["content"].as_str().unwrap().contains("ALPHA-MARKER"));

    // Stored conversation: OpenAI shapes, in order.
    let lines = agent.conversation_lines(&conv_id);
    assert_eq!(roles(&lines), ["user", "assistant", "tool", "assistant"]);
    assert_eq!(lines[0]["content"], "what is alpha?");
    let call = &lines[1]["tool_calls"][0];
    assert_eq!(call["id"], "call_1");
    assert_eq!(call["type"], "function");
    assert_eq!(call["function"]["name"], "kb.read");
    let args: Value =
        serde_json::from_str(call["function"]["arguments"].as_str().unwrap()).unwrap();
    assert_eq!(args, json!({"query": "alpha-project"}));
    assert_eq!(lines[2]["tool_call_id"], "call_1");
    assert!(lines[2]["content"]
        .as_str()
        .unwrap()
        .contains("ALPHA-MARKER"));
    assert_eq!(lines[3]["content"], "Alpha routes the mesh.");

    // One journal line per turn.
    let turns: Vec<_> = agent
        .state
        .journal
        .tail(50)
        .into_iter()
        .filter(|e| e.kind == "chat.turn")
        .collect();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].detail["conversation_id"], conv_id.as_str());
    assert_eq!(turns[0].detail["tools"], 1);
}

#[test]
fn kb_absent_is_stated_in_the_system_prompt() {
    let model = model_mock(vec![Step::Text("My project knowledge is absent here.")]);
    let agent = start_agent(&model.url(), Path::new("/nonexistent/kb-dir"));

    let r = agent.post_chat(json!({"message": "tell me about alpha", "stream": false}));
    assert_eq!(r.status, 200, "{}", r.body);

    let chats = model.chats();
    let system = chats[0]["messages"][0]["content"].as_str().unwrap();
    assert_eq!(chats[0]["messages"][0]["role"], "system");
    assert!(system.contains("ABSENT"), "{system}");
}

#[test]
fn kb_tool_with_absent_kb_reports_an_error_not_an_invention() {
    let model = model_mock(vec![
        Step::Tool {
            id: "call_a",
            name: "kb.read",
            args: r#"{"query":"alpha-project"}"#,
        },
        Step::Text("I have no knowledge base on this node."),
    ]);
    let agent = start_agent(&model.url(), Path::new("/nonexistent/kb-dir"));

    let r = agent.post_chat(json!({"message": "alpha?", "stream": false}));
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(
        r.json()["tool_trace"],
        json!([{"name": "kb.read", "ok": false}])
    );
    let second = model.chats()[1]["messages"].clone();
    let tool = second
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap()
        .clone();
    assert!(tool["content"]
        .as_str()
        .unwrap()
        .contains("knowledge base absent on this node"));
}

#[test]
fn round_cap_is_four_model_calls_and_an_honest_reply() {
    let model = model_mock(vec![Step::Tool {
        id: "call_loop",
        name: "logs.read",
        args: r#"{"n":1}"#,
    }]);
    let agent = start_agent(&model.url(), &fixture_kb());

    let r = agent.post_chat(json!({"message": "loop forever", "stream": false}));
    assert_eq!(r.status, 200, "{}", r.body);
    let body = r.json();
    assert_eq!(model.chats().len(), 4, "exactly 4 model requests");
    let reply = body["reply"].as_str().unwrap();
    assert!(reply.contains("tool budget exhausted"), "{reply}");
    // Rounds 1-3 ran their tools; the pending calls of round 4 never ran.
    assert_eq!(body["tool_trace"].as_array().unwrap().len(), 3);

    // The stored history stays valid: no tool_calls left without an answer.
    let conv_id = body["conversation_id"].as_str().unwrap();
    let lines = agent.conversation_lines(conv_id);
    assert_eq!(
        roles(&lines),
        [
            "user",
            "assistant",
            "tool",
            "assistant",
            "tool",
            "assistant",
            "tool",
            "assistant"
        ]
    );
    assert!(lines[7].get("tool_calls").is_none());
    assert!(lines[7]["content"]
        .as_str()
        .unwrap()
        .contains("tool budget exhausted"));
}

#[test]
fn model_down_is_503_before_any_stream() {
    let agent = start_agent(&closed_port_url(), &fixture_kb());

    for body in [
        json!({"message": "hi", "stream": false}),
        json!({"message": "hi", "stream": true}),
        json!({"message": "hi"}), // stream defaults to true
    ] {
        let r = agent.post_chat(body);
        assert_eq!(r.status, 503, "{}", r.body);
        assert!(
            r.content_type.starts_with("application/json"),
            "{}",
            r.content_type
        );
        assert_eq!(r.json(), json!({"schema": 1, "error": "model unavailable"}));
    }
    assert_eq!(agent.state.store.conversation_count(), 0);
}

#[test]
fn empty_message_or_bad_conversation_id_is_400_and_never_reaches_the_model() {
    let model = model_mock(vec![Step::Text("unused")]);
    let agent = start_agent(&model.url(), &fixture_kb());

    for body in [
        json!({"message": "", "stream": false}),
        json!({"message": "   \n", "stream": false}),
        json!({"conversation_id": "../../etc/passwd", "message": "hi", "stream": false}),
    ] {
        let r = agent.post_chat(body);
        assert_eq!(r.status, 400, "{}", r.body);
        assert_eq!(r.json()["schema"], 1);
        assert!(r.json()["error"].is_string());
    }
    assert!(model.chats().is_empty());
}

#[test]
fn streaming_emits_tool_call_delta_done_then_done_marker() {
    let model = model_mock(GROUNDED.to_vec());
    let agent = start_agent(&model.url(), &fixture_kb());

    // `stream` omitted: the contract default is true.
    let r = agent.post_chat(json!({"conversation_id": null, "message": "what is alpha?"}));
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(
        r.content_type.starts_with("text/event-stream"),
        "{}",
        r.content_type
    );

    let data = r.sse_data();
    assert_eq!(data.last().map(String::as_str), Some("[DONE]"), "{data:?}");
    let events: Vec<Value> = data[..data.len() - 1]
        .iter()
        .map(|d| serde_json::from_str(d).unwrap())
        .collect();
    let types: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(types.last(), Some(&"done"), "{types:?}");
    assert_eq!(types.iter().filter(|t| **t == "done").count(), 1);

    let tool_pos = types
        .iter()
        .position(|t| *t == "tool_call")
        .expect("tool_call event");
    assert_eq!(events[tool_pos]["name"], "kb.read");
    let first_delta = types
        .iter()
        .position(|t| *t == "delta")
        .expect("delta event");
    assert!(tool_pos < first_delta, "{types:?}");
    let streamed: String = events
        .iter()
        .filter(|e| e["type"] == "delta")
        .map(|e| e["text"].as_str().unwrap())
        .collect();
    assert_eq!(streamed, "Alpha routes the mesh.");

    let done = events.last().unwrap();
    assert_eq!(done["reply"], "Alpha routes the mesh.");
    assert_eq!(done["tool_trace"], json!([{"name": "kb.read", "ok": true}]));
    let conv_id = done["conversation_id"].as_str().unwrap();
    assert!(conv_id.starts_with("conv-"));
    assert_eq!(agent.conversation_lines(conv_id).len(), 4);
}

#[test]
fn history_returns_stored_messages_in_order_and_empty_for_unknown() {
    let model = model_mock(vec![
        Step::Text("first answer"),
        Step::Text("second answer"),
    ]);
    let agent = start_agent(&model.url(), &fixture_kb());

    let first = agent
        .post_chat(json!({"message": "question one", "stream": false}))
        .json();
    let conv_id = first["conversation_id"].as_str().unwrap().to_string();
    let second = agent
        .post_chat(json!({"conversation_id": conv_id, "message": "question two", "stream": false}))
        .json();
    assert_eq!(second["conversation_id"], conv_id.as_str());

    // Turn two sent turn one's messages to the model as context.
    let sent = model.chats()[1]["messages"].clone();
    let sent = sent.as_array().unwrap();
    assert_eq!(roles(sent), ["system", "user", "assistant", "user"]);
    assert_eq!(sent[1]["content"], "question one");
    assert_eq!(sent[2]["content"], "first answer");

    let h = agent.get(&format!("/v1/chat/history?conversation_id={conv_id}"));
    assert_eq!(h.status, 200, "{}", h.body);
    let h = h.json();
    assert_eq!(h["schema"], 1);
    assert_eq!(h["conversation_id"], conv_id.as_str());
    let msgs = h["messages"].as_array().unwrap();
    assert_eq!(roles(msgs), ["user", "assistant", "user", "assistant"]);
    let contents: Vec<&str> = msgs
        .iter()
        .map(|m| m["content"].as_str().unwrap())
        .collect();
    assert_eq!(
        contents,
        [
            "question one",
            "first answer",
            "question two",
            "second answer"
        ]
    );

    let unknown = agent.get("/v1/chat/history?conversation_id=conv-does-not-exist");
    assert_eq!(unknown.status, 200);
    assert_eq!(unknown.json()["messages"], json!([]));
    assert_eq!(unknown.json()["conversation_id"], "conv-does-not-exist");

    assert_eq!(agent.get("/v1/chat/history").status, 400);
    assert_eq!(agent.get("/v1/chat/history?conversation_id=").status, 400);
    assert_eq!(
        agent.get("/v1/chat/history?conversation_id=a%2Fb").status,
        400
    );
}

#[test]
fn model_failure_mid_turn_is_502_or_an_error_event_and_leaves_no_history() {
    let model = model_mock(vec![Step::ServerError]);
    let agent = start_agent(&model.url(), &fixture_kb());

    let r = agent.post_chat(json!({
        "conversation_id": "conv-fail", "message": "hi", "stream": false
    }));
    assert_eq!(r.status, 502, "{}", r.body);
    assert_eq!(r.json()["schema"], 1);
    assert!(
        r.json()["error"].as_str().unwrap().contains("500"),
        "{}",
        r.body
    );

    let s = agent.post_chat(json!({
        "conversation_id": "conv-fail", "message": "hi", "stream": true
    }));
    assert_eq!(s.status, 200);
    let data = s.sse_data();
    assert!(!data.iter().any(|d| d == "[DONE]"), "{data:?}");
    let last: Value = serde_json::from_str(data.last().unwrap()).unwrap();
    assert_eq!(last["type"], "error");
    assert!(last["error"].as_str().unwrap().contains("500"));

    // A failed turn is atomic: nothing stored, so a retry is clean.
    assert!(agent.conversation_lines("conv-fail").is_empty());
    let failed = agent
        .state
        .journal
        .tail(50)
        .into_iter()
        .filter(|e| e.kind == "chat.turn" && e.detail.get("error").is_some())
        .count();
    assert_eq!(failed, 2);
}

#[test]
fn chat_and_tools_source_is_shell_free() {
    for (name, src) in [
        ("chat.rs", include_str!("../src/chat.rs")),
        ("tools.rs", include_str!("../src/tools.rs")),
    ] {
        assert!(!src.contains("std::process"), "{name} uses std::process");
        assert!(!src.contains("Command"), "{name} mentions Command");
    }
}

// ---------------------------------------------------------------- tool tests

fn ctx_for(state: &AppState) -> ToolCtx {
    ToolCtx {
        kb: state.kb.clone(),
        store: state.store.clone(),
        journal: state.journal.clone(),
        cfg: state.cfg.clone(),
        tasks: state.tasks.clone(),
        model: state.model.clone(),
    }
}

async fn call(state: &AppState, name: &str, args: Value) -> Result<Value, String> {
    tools::dispatch(name, args, &ctx_for(state)).await
}

fn bare_state(kb_dir: &Path) -> (Arc<AppState>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(&closed_port_url(), kb_dir, &closed_port_url(), tmp.path());
    (state, tmp)
}

#[test]
fn schemas_are_exactly_the_five_tools() {
    let names: Vec<String> = tools::schemas()
        .iter()
        .map(|s| s["function"]["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        [
            "kb.read",
            "health.read",
            "logs.read",
            "proposal.write",
            "task.submit"
        ]
    );
}

#[tokio::test]
async fn tool_kb_read_reads_greps_and_admits_no_entry() {
    let (state, _tmp) = bare_state(&fixture_kb());

    let hit = call(&state, "kb.read", json!({"query": "alpha-project"}))
        .await
        .unwrap();
    assert_eq!(hit["path"], "alpha-project");
    assert!(hit["content"].as_str().unwrap().contains("ALPHA-MARKER"));

    // Ambiguous basename: no guessing, the model gets the candidates.
    let amb = call(&state, "kb.read", json!({"query": "beta-thing"}))
        .await
        .unwrap();
    assert!(amb.get("content").is_none());
    let matches: Vec<&str> = amb["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect();
    assert!(matches.contains(&"devices/beta-thing.md"), "{matches:?}");
    assert!(matches.contains(&"projects/beta-thing.md"), "{matches:?}");

    let none = call(&state, "kb.read", json!({"query": "zzz-no-such-thing"}))
        .await
        .unwrap();
    assert_eq!(none["matches"], json!([]));
    assert!(none["hint"].as_str().unwrap().contains("no entry"));

    assert!(call(&state, "kb.read", json!({})).await.is_err());
    assert!(call(&state, "kb.read", json!({"query": "  "}))
        .await
        .is_err());

    let (absent, _tmp2) = bare_state(Path::new("/nonexistent/kb-dir"));
    assert_eq!(
        call(&absent, "kb.read", json!({"query": "alpha"})).await,
        Err("knowledge base absent on this node".to_string())
    );
}

#[tokio::test]
async fn tool_health_read_reports_agent_and_daemon() {
    let model = model_mock(vec![Step::Text("x")]);
    let daemon = MockServer::start(|req, stream| {
        assert_eq!(req.path, "/health");
        respond(
            stream,
            200,
            "OK",
            "application/json",
            r#"{"status":"ok","hub":false}"#,
        );
    });
    let tmp = tempfile::tempdir().unwrap();
    let state = build_state(
        &model.url(),
        &fixture_kb(),
        &format!("{}/health", daemon.url()),
        tmp.path(),
    );
    let up = call(&state, "health.read", json!({})).await.unwrap();
    assert_eq!(
        up["agent"],
        json!({"model_reachable": true, "kb_present": true})
    );
    assert_eq!(up["daemon"], json!({"status": "ok", "hub": false}));

    let (down, _tmp2) = bare_state(Path::new("/nonexistent/kb-dir"));
    let down = call(&down, "health.read", json!({})).await.unwrap();
    assert_eq!(
        down["agent"],
        json!({"model_reachable": false, "kb_present": false})
    );
    assert!(down["daemon"]["error"]
        .as_str()
        .unwrap()
        .contains("unreachable"));
}

#[tokio::test]
async fn tool_logs_read_clamps_n() {
    let (state, _tmp) = bare_state(&fixture_kb());
    for i in 0..150 {
        state.journal.append("test.event", json!({"i": i}));
    }
    let len = |v: &Value| v["events"].as_array().unwrap().len();

    let two = call(&state, "logs.read", json!({"n": 2})).await.unwrap();
    assert_eq!(len(&two), 2);
    assert_eq!(
        two["events"][1]["detail"]["i"], 149,
        "tail is the newest events"
    );
    assert_eq!(
        len(&call(&state, "logs.read", json!({})).await.unwrap()),
        20
    );
    assert_eq!(
        len(&call(&state, "logs.read", json!({"n": 0})).await.unwrap()),
        1
    );
    assert_eq!(
        len(&call(&state, "logs.read", json!({"n": 1000})).await.unwrap()),
        100
    );
}

#[tokio::test]
async fn tool_proposal_write_persists_and_journals() {
    let (state, _tmp) = bare_state(&fixture_kb());
    let out = call(
        &state,
        "proposal.write",
        json!({"title": "Fix the thing", "body": "## Plan\nDo it."}),
    )
    .await
    .unwrap();
    let id = out["proposal_id"].as_str().unwrap();

    let path = state
        .store
        .dir()
        .join("proposals")
        .join(format!("{id}.json"));
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(saved["id"], id);
    assert_eq!(saved["node_id"], "nodeid-abc");
    assert_eq!(saved["task_id"], Value::Null);
    assert_eq!(saved["kind"], "chat");
    assert_eq!(saved["title"], "Fix the thing");
    assert_eq!(saved["body"], "## Plan\nDo it.");

    let written = state
        .journal
        .tail(10)
        .into_iter()
        .find(|e| e.kind == "proposal.written")
        .expect("proposal.written journaled");
    assert_eq!(written.detail["proposal_id"], id);

    assert!(call(&state, "proposal.write", json!({"title": "no body"}))
        .await
        .is_err());
}

#[tokio::test]
async fn tool_task_submit_validates_and_unknown_tools_are_refused() {
    let (state, _tmp) = bare_state(&fixture_kb());

    let bad = call(&state, "task.submit", json!({"kind": "rm-rf", "refs": []})).await;
    assert!(bad.unwrap_err().contains("invalid task kind"));
    assert!(call(&state, "task.submit", json!({"refs": []}))
        .await
        .is_err());

    // No opencode on this node: patch is refused as needs-hub, never faked.
    let patch = call(
        &state,
        "task.submit",
        json!({"kind": "patch", "refs": ["x"], "note": "n"}),
    )
    .await;
    assert!(patch.unwrap_err().contains("needs hub"));

    for name in ["shell.exec", "kb.write", ""] {
        assert_eq!(
            call(&state, name, json!({})).await,
            Err("unknown tool".to_string())
        );
    }
}
