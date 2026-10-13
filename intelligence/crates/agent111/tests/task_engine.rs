//! Task engine tests: hermetic (tempfile state dir, std-TcpListener mock
//! model + daemon, fixture KB, fake-opencode script). Ephemeral loopback
//! ports only; no real network, no real opencode.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use agent111::config::Config;
use agent111::events::{Event, Journal};
use agent111::http::{self, AppState};
use agent111::kb::Kb;
use agent111::model::Client;
use agent111::store::Store;
use agent111::tasks::{SubmitError, TaskEngine, TaskKind};
use common::{begin_sse, respond, MockServer};
use serde_json::{json, Value};

const NODE_ID: &str = "abc123def4567890abc123def4567890";
const LONG: Duration = Duration::from_secs(60);

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel)
}

/// The fixture script, exec bit forced in case git lost the mode.
fn fake_opencode() -> PathBuf {
    static ONCE: Once = Once::new();
    let path = fixture("fake-opencode");
    ONCE.call_once(|| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    });
    path
}

/// A URL on an ephemeral port that is guaranteed closed.
fn closed_port_url() -> String {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

type Seen = Arc<Mutex<Vec<Value>>>;

/// Mock OpenAI endpoint: records each request body, answers with one SSE
/// content delta after `delay`.
fn model_server(reply: &'static str, delay: Duration, seen: Seen) -> MockServer {
    MockServer::start(move |req, sink| {
        if req.path.ends_with("/chat/completions") {
            seen.lock().unwrap().push(req.body_json());
        }
        std::thread::sleep(delay);
        let mut sse = begin_sse(sink).unwrap();
        let chunk = json!({"choices": [{"index": 0, "delta": {"content": reply}}]});
        sse.data(&chunk.to_string()).unwrap();
        sse.done().unwrap();
    })
}

fn failing_model_server() -> MockServer {
    MockServer::start(|_req, sink| {
        respond(sink, 500, "Internal Server Error", "text/plain", "boom");
    })
}

fn daemon_server() -> MockServer {
    MockServer::start(|req, sink| {
        assert_eq!(req.path, "/health");
        respond(
            sink,
            200,
            "OK",
            "application/json",
            r#"{"schema":1,"status":"ok","marker":"DAEMON-HEALTH-MARKER"}"#,
        );
    })
}

struct Harness {
    _tmp: tempfile::TempDir,
    state: PathBuf,
    cfg: Arc<Config>,
    store: Arc<Store>,
    journal: Journal,
    kb: Arc<Kb>,
}

fn harness(tweak: impl FnOnce(&mut Config)) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    let mut cfg = Config {
        state_dir: state.clone(),
        kb_dir: fixture("kb"),
        node_id: Some(NODE_ID.to_string()),
        daemon_health_url: format!("{}/health", closed_port_url()),
        opencode_path: PathBuf::from("/nonexistent/opencode"),
        ..Config::default()
    };
    tweak(&mut cfg);
    let store = Arc::new(Store::init(&state).unwrap());
    let journal = Journal::new(store.events_path());
    let kb = Arc::new(Kb::open(&cfg.kb_dir));
    Harness {
        _tmp: tmp,
        state,
        cfg: Arc::new(cfg),
        store,
        journal,
        kb,
    }
}

impl Harness {
    fn engine(&self, patch_timeout: Duration) -> TaskEngine {
        let model = Arc::new(Client::new(&self.cfg.model_base, &self.cfg.model_id));
        TaskEngine::new(
            self.cfg.clone(),
            self.store.clone(),
            self.journal.clone(),
            model,
        )
        .with_patch_timeout(patch_timeout)
    }

    fn task_json(&self, id: &str) -> Value {
        read_json(&self.state.join("tasks").join(format!("{id}.json")))
    }

    fn json_files(&self, sub: &str) -> Vec<Value> {
        let mut out: Vec<Value> = std::fs::read_dir(self.state.join(sub))
            .unwrap()
            .flatten()
            .map(|e| read_json(&e.path()))
            .collect();
        out.sort_by_key(|v| v["id"].as_str().unwrap_or("").to_string());
        out
    }

    fn kinds(&self) -> Vec<String> {
        self.journal.tail(500).into_iter().map(|e| e.kind).collect()
    }
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Poll the journal until `kind` appears for `task_id`.
fn wait_event(h: &Harness, kind: &str, task_id: &str) -> Event {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(ev) = h
            .journal
            .tail(500)
            .into_iter()
            .find(|e| e.kind == kind && e.detail["task_id"] == task_id)
        {
            return ev;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {kind} of {task_id}; journal kinds: {:?}",
            h.kinds()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

// (1) diagnose: the model sees health + KB context; the proposal is stamped.
#[test]
fn diagnose_builds_context_and_lands_a_proposal() {
    let seen: Seen = Arc::default();
    let model = model_server(
        "## Summary\nThe daemon reports ok.\n\n## Evidence\nhealth ok\n\n## Suggested action\nNone.",
        Duration::from_millis(300),
        seen.clone(),
    );
    let daemon = daemon_server();
    let h = harness(|c| {
        c.model_base = model.url();
        c.daemon_health_url = format!("{}/health", daemon.url());
    });
    let engine = h.engine(LONG);

    let id = engine
        .submit(
            TaskKind::Diagnose,
            vec![
                "alpha-project".into(),
                "beta-thing".into(), // ambiguous basename: must stay unresolved
                "no-such-entry".into(),
            ],
            Some("why is the mesh slow".into()),
        )
        .expect("diagnose accepted");
    assert_eq!(
        engine.open_count(),
        1,
        "running while the model is thinking"
    );
    assert_eq!(h.task_json(&id)["state"], "running");

    let done = wait_event(&h, "task.done", &id);
    assert_eq!(done.detail["kind"], "diagnose");
    assert_eq!(engine.open_count(), 0);

    // Exactly one tool-less model call; the prompt carries the evidence.
    let reqs = seen.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    let req = &reqs[0];
    assert!(req.get("tools").is_none(), "no tools for task runs");
    assert_eq!(req["messages"][0]["role"], "system");
    let system = req["messages"][0]["content"].as_str().unwrap();
    for needle in ["diagnosis", "Summary", "Evidence", "Suggested action"] {
        assert!(system.contains(needle), "system prompt lacks {needle}");
    }
    let user = req["messages"][1]["content"].as_str().unwrap();
    assert!(user.contains("why is the mesh slow"));
    let ctx: Value = serde_json::from_str(user.split("Context (JSON):\n").nth(1).unwrap()).unwrap();
    assert_eq!(ctx["daemon_health"]["marker"], "DAEMON-HEALTH-MARKER");
    assert_eq!(ctx["node_id"], NODE_ID);
    assert_eq!(ctx["refs"][0]["resolved"], true);
    assert!(ctx["refs"][0]["content"]
        .as_str()
        .unwrap()
        .contains("ALPHA-MARKER"));
    assert_eq!(ctx["refs"][1]["resolved"], false);
    assert_eq!(ctx["refs"][2]["ref"], "no-such-entry");
    assert_eq!(ctx["refs"][2]["content"], Value::Null);
    let recent: Vec<&str> = ctx["recent_events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(recent.contains(&"task.submitted"), "{recent:?}");
    drop(reqs);

    // Proposal on disk, node id stamped, linked to the task.
    let proposals = h.json_files("proposals");
    assert_eq!(proposals.len(), 1);
    let p = &proposals[0];
    assert_eq!(p["schema"], 1);
    assert_eq!(p["node_id"], NODE_ID);
    assert_eq!(p["task_id"], id.as_str());
    assert_eq!(p["kind"], "diagnose");
    assert_eq!(p["title"], "Summary");
    assert!(p["body"].as_str().unwrap().contains("Suggested action"));

    let task = h.task_json(&id);
    assert_eq!(task["state"], "done");
    assert_eq!(task["proposal_id"], p["id"]);
    assert!(task["finished_at"].is_string());
    assert_eq!(done.detail["proposal_id"], p["id"]);
    assert!(h.kinds().contains(&"task.submitted".to_string()));
}

// (2) model failure: task failed, error in the journal, no proposal.
#[test]
fn propose_with_model_500_fails_honestly() {
    let model = failing_model_server();
    let h = harness(|c| c.model_base = model.url());
    let engine = h.engine(LONG);

    let id = engine.submit(TaskKind::Propose, vec![], None).unwrap();
    let ev = wait_event(&h, "task.failed", &id);

    let err = ev.detail["error"].as_str().unwrap();
    assert!(err.contains("500"), "{err}");
    assert_eq!(ev.detail["kind"], "propose");
    let task = h.task_json(&id);
    assert_eq!(task["state"], "failed");
    assert_eq!(task["proposal_id"], Value::Null);
    assert!(task["finished_at"].is_string());
    assert!(h.json_files("proposals").is_empty());
    assert_eq!(engine.open_count(), 0);
}

// (3) patch via opencode: env is scrubbed to the local provider, JSONL text
// becomes the proposal body.
#[test]
fn patch_delegates_to_opencode_with_scrubbed_env() {
    let seen: Seen = Arc::default();
    let model = model_server("unused", Duration::ZERO, seen.clone());
    let h = harness(|c| {
        c.model_base = model.url();
        c.opencode_path = fake_opencode();
    });
    let engine = h.engine(LONG);

    let id = engine
        .submit(
            TaskKind::Patch,
            vec!["alpha-project".into()],
            Some("fix heartbeat".into()),
        )
        .expect("patch accepted when opencode exists");
    let done = wait_event(&h, "task.done", &id);

    let p = &h.json_files("proposals")[0];
    assert_eq!(p["kind"], "patch");
    assert_eq!(p["node_id"], NODE_ID);
    assert_eq!(p["task_id"], id.as_str());
    assert_eq!(p["title"], "patch: fix heartbeat");
    let body = p["body"].as_str().unwrap();
    assert!(body.contains("FAKE-OPENCODE-PATCH"), "{body}");
    assert!(body.contains("plain stdout line, not JSON"), "{body}");
    assert!(!body.contains("session.start"), "only text fields: {body}");
    let work = h.state.join("work").join(&id);
    assert!(body.contains(&work.display().to_string()), "{body}");
    assert_eq!(h.task_json(&id)["state"], "done");
    assert_eq!(done.detail["proposal_id"], p["id"]);

    // The child ran in the work dir with only the permitted environment.
    let env = std::fs::read_to_string(work.join("opencode-env.txt")).unwrap();
    let vars: Vec<(&str, &str)> = env.lines().filter_map(|l| l.split_once('=')).collect();
    let get = |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| *v);
    assert_eq!(
        get("OPENAI_BASE_URL"),
        Some(format!("{}/v1", model.url()).as_str())
    );
    assert_eq!(get("OPENAI_API_KEY"), Some("local"));
    for (name, _) in &vars {
        assert!(
            [
                "PATH",
                "HOME",
                "OPENAI_BASE_URL",
                "OPENAI_API_KEY",
                "PWD",
                "OLDPWD",
                "SHLVL",
                "_"
            ]
            .contains(name),
            "unexpected env var leaked into opencode: {name}"
        );
    }

    // Pinned headless argv, prompt last and grounded in the KB ref.
    let argv = std::fs::read_to_string(work.join("opencode-argv.txt")).unwrap();
    assert!(
        argv.starts_with("run\n--format\njson\n--pure\n-m\nlocal/"),
        "{argv}"
    );
    assert!(argv.contains("fix heartbeat") && argv.contains("ALPHA-MARKER"));
    // Per-task project config: exactly one provider, the local endpoint,
    // with the node's model declared (opencode rejects undeclared model ids).
    let oc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(work.join("opencode.json")).unwrap())
            .unwrap();
    let providers = oc["provider"].as_object().unwrap();
    assert_eq!(providers.len(), 1, "exactly one provider: {oc}");
    assert_eq!(
        oc["provider"]["local"]["options"]["baseURL"],
        format!("{}/v1", model.url())
    );
    assert!(
        oc["provider"]["local"]["models"].get("qwen3-4b").is_some()
            || oc["model"].as_str().unwrap().starts_with("local/")
    );

    // patch never touches the model.
    assert!(seen.lock().unwrap().is_empty());
}

// (4) patch without opencode: needs hub, nothing persisted, refusal journaled.
#[test]
fn patch_without_opencode_needs_hub() {
    for missing in ["/nonexistent/opencode", "definitely-not-on-path-111"] {
        let h = harness(|c| c.opencode_path = PathBuf::from(missing));
        let engine = h.engine(LONG);

        let res = engine.submit(TaskKind::Patch, vec![], Some("x".into()));
        assert!(matches!(res, Err(SubmitError::NeedsHub)), "{missing}");
        assert_eq!(res.unwrap_err().to_string(), "needs hub");

        assert!(h.json_files("tasks").is_empty(), "refusal is not a task");
        assert!(h.json_files("proposals").is_empty());
        let refused = h
            .journal
            .tail(50)
            .into_iter()
            .find(|e| e.kind == "task.refused")
            .expect("task.refused journaled");
        assert_eq!(refused.detail["kind"], "patch");
        assert_eq!(engine.open_count(), 0);
    }
}

// (5) opencode failures: non-zero exit and timeout both end `failed`.
#[test]
fn patch_nonzero_exit_and_timeout_fail() {
    let h = harness(|c| c.opencode_path = fake_opencode());

    let engine = h.engine(LONG);
    let id = engine
        .submit(TaskKind::Patch, vec![], Some("FAKE_OPENCODE_EXIT=3".into()))
        .unwrap();
    let ev = wait_event(&h, "task.failed", &id);
    let err = ev.detail["error"].as_str().unwrap();
    assert!(err.contains("exit code 3"), "{err}");
    assert!(
        err.contains("fake-opencode: finished"),
        "stderr tail: {err}"
    );
    assert!(err.len() < 2 * 1024 + 200, "stderr tail is capped");
    assert_eq!(h.task_json(&id)["state"], "failed");
    assert!(h.json_files("proposals").is_empty());

    let engine = h.engine(Duration::from_millis(600));
    let started = Instant::now();
    let id = engine
        .submit(
            TaskKind::Patch,
            vec![],
            Some("FAKE_OPENCODE_SLEEP=30".into()),
        )
        .unwrap();
    let ev = wait_event(&h, "task.failed", &id);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "child was killed"
    );
    assert!(ev.detail["error"].as_str().unwrap().contains("timed out"));
    assert_eq!(h.task_json(&id)["state"], "failed");
    assert_eq!(engine.open_count(), 0);
}

// (6) restart: a task left `running` by a dead process is orphaned.
#[test]
fn restart_marks_running_tasks_failed() {
    let h = harness(|_| {});
    let by_hand = json!({
        "schema": 1, "id": "task-old-0001", "kind": "diagnose", "refs": ["alpha-project"],
        "note": null, "state": "running", "proposal_id": null,
        "created_at": "2026-10-13T00:00:00Z", "finished_at": null
    });
    std::fs::write(
        h.state.join("tasks/task-old-0001.json"),
        serde_json::to_string_pretty(&by_hand).unwrap(),
    )
    .unwrap();
    let done = json!({
        "schema": 1, "id": "task-old-0002", "kind": "propose", "refs": [], "note": null,
        "state": "done", "proposal_id": "prop-x", "created_at": "2026-10-13T00:00:00Z",
        "finished_at": "2026-10-13T00:01:00Z"
    });
    std::fs::write(
        h.state.join("tasks/task-old-0002.json"),
        serde_json::to_string_pretty(&done).unwrap(),
    )
    .unwrap();

    let engine = h.engine(LONG);

    let task = h.task_json("task-old-0001");
    assert_eq!(task["state"], "failed");
    assert!(task["finished_at"].is_string());
    let orphaned: Vec<Event> = h
        .journal
        .tail(50)
        .into_iter()
        .filter(|e| e.kind == "task.orphaned")
        .collect();
    assert_eq!(orphaned.len(), 1, "only the running task is orphaned");
    assert_eq!(orphaned[0].detail["task_id"], "task-old-0001");
    assert_eq!(orphaned[0].detail["kind"], "diagnose");
    assert_eq!(engine.open_count(), 0);
    // Finished tasks are left exactly as they were.
    assert_eq!(h.task_json("task-old-0002"), done);
}

// (7) the HTTP contract: 409 {error:"needs hub"} for patch without opencode,
// 202 {task_id} for a diagnose that then completes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_task_endpoint_contract() {
    let seen: Seen = Arc::default();
    let model_srv = model_server("## Summary\nfine", Duration::ZERO, seen);
    let h = harness(|c| c.model_base = model_srv.url());
    let engine = Arc::new(h.engine(LONG));
    let state = Arc::new(AppState {
        cfg: h.cfg.clone(),
        model: Arc::new(Client::new(&h.cfg.model_base, &h.cfg.model_id)),
        kb: h.kb.clone(),
        store: h.store.clone(),
        journal: h.journal.clone(),
        tasks: engine,
        started: Instant::now(),
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, http::router(state)).await });

    let post = move |body: Value| {
        let url = format!("http://{addr}/v1/agent/task");
        async move {
            tokio::task::spawn_blocking(move || {
                let agent: ureq::Agent = ureq::Agent::config_builder()
                    .http_status_as_error(false)
                    .build()
                    .into();
                let mut resp = agent.post(&url).send_json(&body).unwrap();
                let status = resp.status().as_u16();
                (status, resp.body_mut().read_json::<Value>().unwrap())
            })
            .await
            .unwrap()
        }
    };

    let (status, body) = post(json!({"kind": "patch", "refs": [], "note": null})).await;
    assert_eq!(status, 409);
    assert_eq!(body, json!({"schema": 1, "error": "needs hub"}));
    assert!(h.json_files("tasks").is_empty());

    let (status, body) = post(json!({"kind": "diagnose", "refs": ["alpha-project"]})).await;
    assert_eq!(status, 202);
    assert_eq!(body["schema"], 1);
    let id = body["task_id"].as_str().unwrap().to_string();
    wait_event(&h, "task.done", &id);
    assert_eq!(h.task_json(&id)["state"], "done");
}
