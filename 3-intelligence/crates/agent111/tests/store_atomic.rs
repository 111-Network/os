//! The agent's half of the store hand-off (Layer 8 row A1;
//! 8-storage/README.md §2.4 rule 1, §2.3 rule 2; G13 (2) and (4)), hermetic:
//! temp state dirs, and for the chat turn the real router on an ephemeral
//! loopback port with a scripted model. No fixed ports, no real network.
//!
//! `proposals/` and `tasks/` are the spool `111d` ingests: every file is
//! written atomically (a `.tmp-` file in the same directory, fsync, rename), a
//! `.tmp-` file is never read as a task, and the ones a stop mid-write left
//! are removed at the agent's start. A chat turn whose body carries `history`
//! writes no conversation file and leaves its text in no file.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent111::config::Config;
use agent111::events::Journal;
use agent111::http::{router, AppState};
use agent111::kb::Kb;
use agent111::model::Client;
use agent111::store::{Store, Task, TaskState};
use agent111::tasks::{TaskEngine, TaskKind};
use common::{begin_sse, respond, MockServer};
use serde_json::{json, Value};

fn task(id: &str, state: TaskState) -> Task {
    Task {
        schema: 1,
        id: id.to_string(),
        kind: TaskKind::Diagnose,
        refs: vec!["ref-1".to_string()],
        note: None,
        state,
        proposal_id: None,
        created_at: "2026-11-16T00:00:00Z".to_string(),
        finished_at: None,
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

fn temp_names(dir: &Path) -> Vec<String> {
    names(dir)
        .into_iter()
        .filter(|n| n.starts_with(".tmp-"))
        .collect()
}

fn fresh_store() -> (Store, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::init(&tmp.path().join("state")).unwrap();
    (store, tmp)
}

// ------------------------------------------------------------ atomic writes

/// A rewrite renames a new file over the old one (a new inode); a write in
/// place would truncate the file `111d` may be reading.
#[cfg(unix)]
#[test]
fn a_task_rewrite_is_a_new_file_renamed_over_the_old() {
    use std::os::unix::fs::MetadataExt;

    let (store, _tmp) = fresh_store();
    let tasks = store.dir().join("tasks");
    let path = tasks.join("task-a.json");

    let mut t = task("task-a", TaskState::Running);
    store.save_task(&t).unwrap();
    let before = std::fs::metadata(&path).unwrap().ino();
    t.state = TaskState::Done;
    t.finished_at = Some("2026-11-16T00:01:00Z".to_string());
    store.save_task(&t).unwrap();
    let after = std::fs::metadata(&path).unwrap().ino();
    assert_ne!(before, after, "the rewrite went into the old file");

    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["state"], "done");
    assert_eq!(saved["finished_at"], "2026-11-16T00:01:00Z");
    assert_eq!(names(&tasks), ["task-a.json"]);

    let p = store
        .save_proposal(Some("nodeid".into()), None, "chat", "A title", "A body")
        .unwrap();
    let proposals = store.dir().join("proposals");
    assert_eq!(names(&proposals), [format!("{}.json", p.id)]);
    let saved: Value = serde_json::from_str(
        &std::fs::read_to_string(proposals.join(format!("{}.json", p.id))).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["id"], p.id.as_str());
    assert_eq!(saved["body"], "A body");
}

/// A reader that lists the spool the way `111d`'s sweep does (`<id>.json`
/// only, G13 (4)) never reads a torn file while the agent writes.
#[test]
fn a_reader_of_the_spool_never_sees_a_torn_file() {
    let (store, _tmp) = fresh_store();
    let dir = store.dir().to_path_buf();
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let (mut reads, mut torn) = (0usize, Vec::new());
            let mut done: Vec<String> = Vec::new();
            // One pass more after the writer stops: a reader that started
            // late still reads every file once.
            loop {
                let last = stop.load(Ordering::Relaxed);
                for sub in ["proposals", "tasks"] {
                    let Ok(rd) = std::fs::read_dir(dir.join(sub)) else {
                        continue;
                    };
                    for e in rd.flatten() {
                        let name = e.file_name().to_string_lossy().into_owned();
                        if name.starts_with('.') || !name.ends_with(".json") {
                            continue;
                        }
                        // A proposal file is written once: one whole read of
                        // it is enough. A task file is rewritten: read it every
                        // time round.
                        if sub == "proposals" && done.contains(&name) {
                            continue;
                        }
                        let Ok(text) = std::fs::read_to_string(e.path()) else {
                            continue;
                        };
                        reads += 1;
                        if serde_json::from_str::<Value>(&text).is_ok() {
                            if sub == "proposals" {
                                done.push(name);
                            }
                        } else {
                            torn.push(format!("{sub}/{name}: {} bytes", text.len()));
                        }
                    }
                }
                if last {
                    break;
                }
            }
            (reads, torn)
        })
    };

    let body = "b".repeat(512 * 1024);
    let mut t = task("task-rewritten", TaskState::Running);
    for i in 0..40 {
        store
            .save_proposal(None, None, "chat", &format!("proposal {i}"), &body)
            .unwrap();
        t.note = Some(format!("{i} {body}"));
        store.save_task(&t).unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    let (reads, torn) = reader.join().unwrap();
    assert!(reads >= 41, "{reads}");
    assert!(
        torn.is_empty(),
        "{} torn reads, the first: {:?}",
        torn.len(),
        &torn[..torn.len().min(5)]
    );
    assert!(temp_names(&store.dir().join("proposals")).is_empty());
    assert!(temp_names(&store.dir().join("tasks")).is_empty());
}

/// A write that cannot land leaves no temp file and the target as it was.
#[test]
fn a_failed_write_leaves_no_temp_file() {
    let (store, _tmp) = fresh_store();
    let tasks = store.dir().join("tasks");
    // A directory where the task file would go: the rename cannot replace it.
    std::fs::create_dir_all(tasks.join("task-blocked.json").join("inside")).unwrap();
    assert!(store
        .save_task(&task("task-blocked", TaskState::Running))
        .is_err());
    assert_eq!(names(&tasks), ["task-blocked.json"]);
    assert!(tasks.join("task-blocked.json").is_dir());
}

/// Spool writes come from async code (the proposal tools, the task route),
/// from task threads and from the start. Each waits on the disk off the
/// runtime's workers where it runs on one, and in place wherever that is not
/// allowed (a current-thread runtime): whole on every kind of thread, and
/// never a panic.
#[test]
fn a_spool_write_works_on_every_kind_of_thread() {
    let (store, _tmp) = fresh_store();
    let write = |id: &str| {
        let (store, id) = (store.clone(), id.to_string());
        move || {
            store.save_task(&task(&id, TaskState::Running)).unwrap();
            store
                .save_proposal(None, Some(id), "chat", "title", "body")
                .unwrap()
                .task_id
                .unwrap()
        }
    };
    let multi = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let current = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let on_worker = write("task-worker");
    let on_current = write("task-current");
    let in_multi_block_on = write("task-multi-block-on");
    let in_current_block_on = write("task-current-block-on");
    let mut done = vec![
        std::thread::spawn(write("task-thread")).join().unwrap(),
        multi
            .block_on(multi.spawn(async move { on_worker() }))
            .unwrap(),
        multi
            .block_on(multi.spawn_blocking(write("task-blocking")))
            .unwrap(),
        multi.block_on(async move { in_multi_block_on() }),
        current
            .block_on(current.spawn(async move { on_current() }))
            .unwrap(),
        current.block_on(async move { in_current_block_on() }),
    ];
    done.sort();
    let mut tasks: Vec<String> = store.load_tasks().into_iter().map(|t| t.id).collect();
    tasks.sort();
    assert_eq!(tasks, done);
    assert_eq!(names(&store.dir().join("proposals")).len(), 6);
    assert!(temp_names(&store.dir().join("proposals")).is_empty());
    assert!(temp_names(&store.dir().join("tasks")).is_empty());
}

// ------------------------------------------------------- temp files at start

/// At its start (`Store::init`) the agent removes every `.tmp-*` of its two
/// spool directories, complete or not, and nothing else (G13 (4)).
#[test]
fn the_start_removes_every_temp_file_of_both_spool_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("state");
    Store::init(&dir).unwrap();
    let whole_task = serde_json::to_string(&task("task-left", TaskState::Running)).unwrap();
    let plant = |rel: &str, text: &str| std::fs::write(dir.join(rel), text).unwrap();
    plant(
        "proposals/.tmp-prop-left.json.3",
        r#"{"schema":1,"id":"prop-left"}"#,
    );
    plant("proposals/.tmp-", "");
    plant("tasks/.tmp-task-left.json.0", &whole_task);
    plant("tasks/.tmp-half", r#"{"schema":1,"id":"#);
    // Spool files and anything outside the two spool directories stay.
    plant(
        "proposals/prop-keep.json",
        r#"{"schema":1,"id":"prop-keep"}"#,
    );
    plant("proposals/tmp-not-a-temp.json", "{}");
    plant("tasks/task-keep.json", &whole_task);
    plant("conversations/.tmp-c", "kept");
    plant(".tmp-root", "kept");
    std::fs::create_dir_all(dir.join("work")).unwrap();
    plant("work/.tmp-w", "kept");

    let store = Store::init(&dir).unwrap();
    assert_eq!(
        names(&dir.join("proposals")),
        ["prop-keep.json", "tmp-not-a-temp.json"]
    );
    assert_eq!(names(&dir.join("tasks")), ["task-keep.json"]);
    assert_eq!(names(&dir.join("conversations")), [".tmp-c"]);
    assert!(dir.join(".tmp-root").exists());
    assert!(dir.join("work/.tmp-w").exists());
    assert_eq!(store.dir(), dir.as_path());
}

/// A `.tmp-` file is never read as a task, so the start's orphan pass never
/// turns one into a task file either.
#[test]
fn a_temp_file_is_never_read_as_a_task() {
    let (store, _tmp) = fresh_store();
    store
        .save_task(&task("task-real", TaskState::Running))
        .unwrap();
    let ghost = serde_json::to_string(&task("task-ghost", TaskState::Running)).unwrap();
    let tasks = store.dir().join("tasks");
    std::fs::write(tasks.join(".tmp-task-ghost.json.7"), ghost).unwrap();

    let ids: Vec<String> = store.load_tasks().into_iter().map(|t| t.id).collect();
    assert_eq!(ids, ["task-real"]);
}

// ------------------------------------------------- no conversation file

fn closed_port_url() -> String {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

/// A model that answers every chat request with `reply`.
fn text_model(reply: &'static str) -> MockServer {
    MockServer::start(move |req, stream| match req.path.as_str() {
        "/health" => respond(stream, 200, "OK", "application/json", r#"{"status":"ok"}"#),
        "/v1/chat/completions" => {
            let chunk = json!({"choices": [{"index": 0, "delta": {"content": reply}}]});
            if let Ok(mut sse) = begin_sse(stream) {
                let _ = sse.data(&chunk.to_string());
                let _ = sse.done();
            }
        }
        _ => respond(stream, 404, "Not Found", "text/plain", "no such route"),
    })
}

struct Served {
    base: String,
    state: Arc<AppState>,
    _rt: tokio::runtime::Runtime,
    _tmp: tempfile::TempDir,
}

fn serve_agent(model_base: &str) -> Served {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = Arc::new(Config {
        model_base: model_base.to_string(),
        kb_dir: PathBuf::from("/nonexistent/kb-dir"),
        state_dir: tmp.path().join("state"),
        category: "test-cat".to_string(),
        daemon_health_url: closed_port_url(),
        opencode_path: PathBuf::from("/nonexistent/opencode"),
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
    let state = Arc::new(AppState {
        cfg,
        model,
        kb,
        store,
        journal,
        tasks,
        started: Instant::now(),
    });
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
    Served {
        base: format!("http://{addr}"),
        state,
        _rt: rt,
        _tmp: tmp,
    }
}

fn post_chat(base: &str, body: &Value) -> (u16, String) {
    let http: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut resp = http
        .post(&format!("{base}/v1/chat"))
        .send_json(body)
        .unwrap();
    (
        resp.status().as_u16(),
        resp.body_mut().read_to_string().unwrap(),
    )
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let path = e.path();
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Every file under `dir` that holds `needle` (a byte search, G13 (2)).
fn files_holding(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    files_under(dir, &mut files);
    files
        .into_iter()
        .filter(|p| {
            let bytes = std::fs::read(p).unwrap_or_default();
            bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
        })
        .collect()
}

/// G13 (2), the agent's half: a turn whose body carries `history` (from a
/// `111d` that keeps the conversation) writes no conversation file, streamed
/// or not, and leaves its sentences in no file of the state dir.
#[test]
fn a_chat_turn_with_history_writes_no_conversation_file() {
    let model = text_model("Morning tea is near.");
    let agent = serve_agent(&model.url());
    let dir = agent.state.store.dir().to_path_buf();

    let history = json!([
        {"role": "user", "content": "Earlier words."},
        {"role": "assistant", "content": "Earlier reply."},
    ]);
    for stream in [false, true] {
        let (status, body) = post_chat(
            &agent.base,
            &json!({
                "conversation_id": "conv-nofile",
                "message": "The kettle sings at dawn.",
                "stream": stream,
                "history": history,
            }),
        );
        assert_eq!(status, 200, "{body}");
        assert!(body.contains("Morning tea is near."), "{body}");
    }
    assert!(names(&dir.join("conversations")).is_empty());
    for sentence in ["The kettle sings at dawn.", "Morning tea is near."] {
        assert_eq!(files_holding(&dir, sentence), Vec::<PathBuf>::new());
    }

    // The same turn from an older 111d (no `history`) still keeps its file:
    // the search above would have found it.
    let (status, body) = post_chat(
        &agent.base,
        &json!({"conversation_id": "conv-legacy", "message": "The kettle sings at dawn.", "stream": false}),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(names(&dir.join("conversations")), ["conv-legacy.jsonl"]);
    assert_eq!(
        files_holding(&dir, "The kettle sings at dawn."),
        [dir.join("conversations/conv-legacy.jsonl")]
    );
}
