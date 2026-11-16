//! Layer 9 §8.1 in the crate (G24 (1)): a `patch` task runs opencode denied.
//! The `opencode.json` it writes parses equal to the deny block beside the
//! provider and the model; the child's `HOME`, its four XDG directories and
//! its `TMPDIR` are inside the task's work dir; the five `OPENCODE_DISABLE_*`
//! switches are set and npm is offline; no auto-approve switch is in its
//! argv. Hermetic: a recorder script stands in for opencode and writes down
//! the environment, argv and cwd it was started with; no network, no real
//! opencode, no model call.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent111::config::Config;
use agent111::events::{Event, Journal};
use agent111::model::Client;
use agent111::store::Store;
use agent111::tasks::{TaskEngine, TaskKind};
use serde_json::{json, Value};

const MODEL_BASE: &str = "http://127.0.0.1:11181";
const MODEL_ID: &str = "qwen3-4b";

/// Stands in for opencode. In its cwd it records the environment, the argv
/// (NUL-separated, so the multi-line prompt stays one element) and the
/// physical cwd, then prints one JSONL text event and exits 0.
const RECORDER: &str = r#"#!/bin/sh
env > opencode-env.txt
printf '%s\0' "$@" > opencode-argv.bin
pwd -P > opencode-cwd.txt
printf '%s\n' '{"type":"text","part":{"text":"RECORDED-BY-STAND-IN"}}'
"#;

/// The five switches of Layer 9 §8.1 (R07g1 F2), each set to `1`.
const FLAGS: [&str; 5] = [
    "OPENCODE_DISABLE_MODELS_FETCH",
    "OPENCODE_DISABLE_AUTOUPDATE",
    "OPENCODE_DISABLE_LSP_DOWNLOAD",
    "OPENCODE_DISABLE_CLAUDE_CODE",
    "OPENCODE_DISABLE_EXTERNAL_SKILLS",
];
const XDG: [&str; 4] = [
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
];
/// opencode 1.19.5 creates `$TMPDIR/opencode` at every start: `/tmp/opencode`,
/// outside the work dir, when the variable is unset.
const TMPDIR: &str = "TMPDIR";
/// npm offline: opencode 1.19.5 installs `@opencode-ai/plugin` from the npm
/// registry into any config dir without `node_modules`, a fresh home's too.
const NPM_OFFLINE: (&str, &str) = ("npm_config_offline", "true");
/// opencode 1.19.5's auto-approve switches: `run --auto` and its hidden
/// aliases (`--yolo`, `--dangerously-skip-permissions`, camel-cased too).
const AUTO_APPROVE: [&str; 4] = [
    "--auto",
    "--yolo",
    "--dangerously-skip-permissions",
    "--dangerouslySkipPermissions",
];
/// What a POSIX shell adds to its environment by itself.
const SHELL_NOISE: [&str; 4] = ["PWD", "OLDPWD", "SHLVL", "_"];

/// The deny block, as Layer 9 §8.1 writes it.
fn deny_block() -> Value {
    json!({
        "*": "deny",
        "read": "allow",
        "glob": "allow",
        "grep": "allow",
        "edit": "deny",
        "bash": "deny",
        "webfetch": "deny",
        "websearch": "deny",
        "external_directory": "deny",
        "task": "deny",
        "skill": "deny",
        "lsp": "deny",
        "question": "deny"
    })
}

/// A URL on an ephemeral port that is guaranteed closed: the context's
/// health read fails at once instead of reaching a live daemon.
fn closed_port_url() -> String {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

/// Poll the journal until the task is done or failed.
fn wait_finished(journal: &Journal, id: &str) -> Event {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let found = journal.tail(500).into_iter().find(|e| {
            matches!(e.kind.as_str(), "task.done" | "task.failed") && e.detail["task_id"] == id
        });
        if let Some(ev) = found {
            return ev;
        }
        assert!(Instant::now() < deadline, "task {id} never finished");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// What the stand-in was started with.
struct Recorded {
    work: PathBuf,
    opencode_json: String,
    env: BTreeMap<String, String>,
    argv: Vec<String>,
    cwd: PathBuf,
}

fn recorded(work: PathBuf) -> Recorded {
    let read = |name: &str| {
        std::fs::read(work.join(name)).unwrap_or_else(|e| panic!("{name} in the work dir: {e}"))
    };
    let text = |name: &str| String::from_utf8(read(name)).expect("utf-8");
    let env = text("opencode-env.txt")
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let argv_bytes = read("opencode-argv.bin");
    let argv_bytes = argv_bytes.strip_suffix(b"\0").expect("NUL-terminated argv");
    let argv = argv_bytes
        .split(|b| *b == 0)
        .map(|a| String::from_utf8(a.to_vec()).expect("utf-8 argv"))
        .collect();
    Recorded {
        opencode_json: text("opencode.json"),
        cwd: PathBuf::from(text("opencode-cwd.txt").trim_end()),
        env,
        argv,
        work,
    }
}

/// One `patch` task through the engine, opencode replaced by the recorder.
fn run_patch() -> (tempfile::TempDir, Recorded) {
    let tmp = tempfile::tempdir().unwrap();
    // Next to the build, not in TMPDIR: a noexec /tmp cannot run it.
    let bin = tempfile::Builder::new()
        .prefix("opencode-config")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let opencode = bin.path().join("opencode");
    std::fs::write(&opencode, RECORDER).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&opencode, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let state = tmp.path().join("state");
    let cfg = Arc::new(Config {
        state_dir: state.clone(),
        kb_dir: tmp.path().join("no-kb"),
        model_base: MODEL_BASE.to_string(),
        model_id: MODEL_ID.to_string(),
        daemon_health_url: format!("{}/health", closed_port_url()),
        opencode_path: opencode,
        ..Config::default()
    });
    let store = Arc::new(Store::init(&state).unwrap());
    let journal = Journal::new(store.events_path());
    let model = Arc::new(Client::new(&cfg.model_base, &cfg.model_id));
    let engine = TaskEngine::new(cfg, store, journal.clone(), model);

    let id = engine
        .submit(
            TaskKind::Patch,
            vec![],
            Some("tighten the heartbeat".into()),
        )
        .expect("patch accepted when opencode exists");
    let ev = wait_finished(&journal, &id);
    assert_eq!(ev.kind, "task.done", "{}", ev.detail);
    assert_eq!(engine.open_count(), 0);
    (tmp, recorded(state.join("work").join(&id)))
}

#[test]
fn patch_runs_opencode_denied() {
    let (_tmp, run) = run_patch();

    // opencode reads its project config from its cwd: the work dir.
    let work = std::fs::canonicalize(&run.work).unwrap();
    assert_eq!(run.cwd, work, "the child runs where opencode.json is");

    // (1) The written opencode.json parses equal to the deny block, beside
    // the one provider and the model it held before; nothing else.
    let doc: Value = serde_json::from_str(&run.opencode_json).expect("opencode.json is JSON");
    assert_eq!(doc["permission"], deny_block(), "the deny block, exactly");
    assert_eq!(doc["autoupdate"], json!(false));
    assert_eq!(doc["share"], json!("disabled"));
    let whole = json!({
        "$schema": "https://opencode.ai/config.json",
        "provider": {
            "local": {
                "npm": "@ai-sdk/openai-compatible",
                "name": "111 local model",
                "options": { "baseURL": format!("{MODEL_BASE}/v1"), "apiKey": "local" },
                "models": { MODEL_ID: { "name": MODEL_ID } }
            }
        },
        "model": format!("local/{MODEL_ID}"),
        "permission": deny_block(),
        "autoupdate": false,
        "share": "disabled"
    });
    assert_eq!(doc, whole);
    // opencode applies the last matching rule in file order: `*` comes
    // before the three allows, or they would be denied too.
    let at = |key: &str| {
        let quoted = format!("\"{key}\"");
        assert_eq!(
            run.opencode_json.matches(&quoted).count(),
            1,
            "{quoted} once"
        );
        run.opencode_json.find(&quoted).unwrap()
    };
    for allowed in ["read", "glob", "grep"] {
        assert!(at("*") < at(allowed), "`*` before `{allowed}`");
    }

    // (2) HOME, the four XDG variables and TMPDIR: absolute, inside the work
    // dir, created; none is the node's own.
    let node_home = std::env::var("HOME").ok();
    for key in std::iter::once("HOME").chain(XDG).chain([TMPDIR]) {
        let value = run.env.get(key).unwrap_or_else(|| panic!("{key} is set"));
        let p = Path::new(value);
        assert!(p.is_absolute(), "{key}={value}");
        assert!(
            !p.components().any(|c| c == Component::ParentDir),
            "{key}={value}"
        );
        assert!(
            p.starts_with(&run.work),
            "{key}={value} is inside the work dir"
        );
        assert!(p.is_dir(), "{key}={value} exists");
        assert_ne!(
            Some(value),
            node_home.as_ref(),
            "{key} is not the node's HOME"
        );
    }
    let home = Path::new(&run.env["HOME"]);
    assert!(
        Path::new(&run.env[TMPDIR]).starts_with(home),
        "TMPDIR is under the child's HOME: {}",
        run.env[TMPDIR]
    );

    // (3) The five switches, each `1`; npm offline.
    for flag in FLAGS {
        assert_eq!(run.env.get(flag).map(String::as_str), Some("1"), "{flag}");
    }
    let (npm_key, npm_value) = NPM_OFFLINE;
    assert_eq!(
        run.env.get(npm_key).map(String::as_str),
        Some(npm_value),
        "{npm_key}"
    );

    // Nothing else of the node's environment reaches the child.
    let mut expected: BTreeSet<&str> =
        ["HOME", TMPDIR, npm_key, "OPENAI_BASE_URL", "OPENAI_API_KEY"].into();
    expected.extend(XDG);
    expected.extend(FLAGS);
    if std::env::var_os("PATH").is_some() {
        expected.insert("PATH");
    }
    for name in run.env.keys() {
        assert!(
            expected.contains(name.as_str()) || SHELL_NOISE.contains(&name.as_str()),
            "unexpected variable in opencode's environment: {name}"
        );
    }
    for name in &expected {
        assert!(run.env.contains_key(*name), "{name} is set");
    }
    assert_eq!(run.env["OPENAI_BASE_URL"], format!("{MODEL_BASE}/v1"));
    assert_eq!(run.env["OPENAI_API_KEY"], "local");

    // (4) No auto-approve switch anywhere in the argv; the prompt is the
    // last element and cannot parse as a switch.
    for arg in &run.argv {
        let name = arg.split('=').next().unwrap_or(arg);
        assert!(!AUTO_APPROVE.contains(&name), "auto-approve in argv: {arg}");
    }
    assert_eq!(
        run.argv.first().map(String::as_str),
        Some("run"),
        "{:?}",
        run.argv
    );
    let prompt = run.argv.last().unwrap();
    assert!(prompt.contains("tighten the heartbeat"), "{prompt}");
    assert!(!prompt.starts_with('-'), "{prompt}");
}
