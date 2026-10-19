//! Optional components end to end (Layer 6 §2.1 rules 1–5, §6 "Start order",
//! §15; `daemon/components/`): declarations, the state from disk facts, the
//! licence notice and its recorded acknowledgement, the fail-closed cache
//! check, the offline install (a fake `python3`/pip for a pip closure, a shell
//! script for a binary), disable releasing claims, "remove files" and
//! "remove cache", ordering after a dependency, and the install-time intake
//! printing and recording every notice before any download. Hermetic: temp
//! roots; every child is stopped on the way out (and dies with the test
//! process through the parent-death signal otherwise).

mod fake_python;

use fake_python::FakePython;
use node111::daemon::components::manifest::{self, Component};
use node111::daemon::components::registry::ComponentState;
use node111::daemon::components::runner::RunnerConfig;
use node111::daemon::components::{
    consent, Components, ComponentsConfig, Dependencies, EnableError, RemoveError, Wiring,
};
use node111::daemon::ports::PortTable;
use node111::daemon::services::ServiceId;
use node111::daemon::supervise::{
    spawn_child, BackoffPolicy, ChildControl, ChildHandle, ChildSpec,
};
use node111::testkit::{MapEnv, MockRunner};
use node111::util::hash::sha256_bytes;
use node111::util::run::{Output, Runner};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(10);

/// The binary payload: says ready, then echoes stdin until stopped.
const BIN: &[u8] = b"#!/bin/sh\necho '{\"type\":\"ready\",\"dest\":\"00\"}'\nexec cat\n";
const WHEEL: &[u8] = b"not really a wheel";

fn binary_toml(id: &str, payload: &[u8], ports: &[u16], extra: &str) -> String {
    format!(
        r#"id = "{id}"
adapter = "app:{id}"
title = "Demo {id}"
[payload]
kind = "binary"
url = "https://example.invalid/dl/{id}-bin"
sha256 = "{sha}"
size = {size}
[run]
program = "{id}-bin"
ports = {ports:?}
health = "ready"
health_timeout_secs = 5
{extra}
[license]
spdx = "MPL-2.0"
read_at = "https://example.invalid/LICENSE"
notice = "This installs demo {id} (MPL-2.0); read its licence at https://example.invalid/LICENSE. You install it."
"#,
        sha = sha256_bytes(payload),
        size = payload.len(),
    )
}

fn lock_for(files: &[&[u8]]) -> String {
    let mut s = String::from("# demo lock\nrns==1.5.7");
    for f in files {
        s.push_str(&format!(" \\\n    --hash=sha256:{}", sha256_bytes(f)));
    }
    s.push('\n');
    s
}

fn pip_toml(id: &str, service: Option<&str>) -> String {
    let service = service
        .map(|s| format!("service = \"{s}\"\n"))
        .unwrap_or_default();
    format!(
        r#"id = "{id}"
adapter = "lxmf"
title = "Demo helper"
[payload]
kind = "pip"
lock = "{id}.lock.txt"
venv = "{id}"
[requires]
python = "3.7"
{service}[run]
program = "share/{id}/{id}.py"
args = ["--config-dir", "{{bridge}}/rns"]
health = "none"
[license]
spdx = "LicenseRef-Reticulum"
read_at = "https://example.invalid/LICENSE"
notice = "This installs the demo helper's closure from PyPI. You install it."
"#
    )
}

fn component(files: Vec<(String, Vec<u8>)>) -> Component {
    let mut all = manifest::load(&files);
    assert_eq!(all.len(), 1, "the test declaration registers");
    all.pop().unwrap()
}

#[derive(Default)]
struct FakeDeps {
    dep: Mutex<Option<ChildControl>>,
    serving: AtomicBool,
}

impl Dependencies for FakeDeps {
    fn child(&self, _: ServiceId) -> Option<ChildControl> {
        self.dep.lock().unwrap().clone()
    }
    fn serving(&self, _: ServiceId) -> bool {
        self.serving.load(Ordering::SeqCst)
    }
}

struct Node {
    _dir: tempfile::TempDir,
    root: PathBuf,
    ports: Arc<PortTable>,
    deps: Arc<FakeDeps>,
}

impl Node {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("var/lib/111");
        std::fs::create_dir_all(root.join("state")).unwrap();
        std::fs::create_dir_all(root.join("current")).unwrap();
        Node {
            _dir: dir,
            root,
            ports: Arc::new(PortTable::new()),
            deps: Arc::new(FakeDeps::default()),
        }
    }

    fn components(&self, list: Vec<Component>, cmd: Arc<dyn Runner>) -> Components {
        let mut cfg = ComponentsConfig::new(&self.root);
        cfg.runner = RunnerConfig {
            retry: Duration::from_millis(50),
            settle: Duration::ZERO,
            grace: Duration::from_secs(2),
        };
        Components::new(
            cfg,
            list,
            Wiring {
                cmd,
                ports: Arc::clone(&self.ports),
                deps: self.deps.clone(),
                tick: Arc::new(|| 7),
            },
        )
    }

    fn cache(&self, id: &str, name: &str, bytes: &[u8]) {
        let dir = self.root.join("cache/packages").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), bytes).unwrap();
    }

    fn exists(&self, rel: &str) -> bool {
        std::fs::symlink_metadata(self.root.join(rel)).is_ok()
    }
}

fn no_commands() -> Arc<MockRunner> {
    Arc::new(MockRunner::new(Vec::<(String, Output)>::new()))
}

fn sleeper(name: &str) -> ChildHandle {
    spawn_child(ChildSpec {
        name: name.into(),
        program: PathBuf::from("/bin/sleep"),
        args: vec!["30".into()],
        env: vec![],
        backoff: BackoffPolicy::default(),
        stdout_log: None,
    })
}

#[test]
fn declarations_with_unknown_keys_or_invalid_values_are_not_registered() {
    let good = binary_toml("demo", BIN, &[], "");
    let files = vec![
        ("demo.toml".to_string(), good.clone().into_bytes()),
        (
            "extra.toml".to_string(),
            binary_toml("extra", BIN, &[], "surprise = 1").into_bytes(),
        ),
        (
            "plain.toml".to_string(),
            good.replace("demo", "plain")
                .replace("https://", "http://")
                .into_bytes(),
        ),
        ("junk.toml".to_string(), b"\xff\xfe".to_vec()),
    ];
    let ids: Vec<String> = manifest::load(&files).into_iter().map(|c| c.id).collect();
    assert_eq!(ids, ["demo"]);
    // the release's own declarations all register (lxmf among them)
    assert!(manifest::load_embedded().iter().any(|c| c.id == "lxmf"));
}

#[tokio::test]
async fn enable_needs_the_exact_acknowledgement_and_installs_nothing_without_it() {
    let n = Node::new();
    let c = component(vec![(
        "demo.toml".into(),
        binary_toml("demo", BIN, &[], "").into_bytes(),
    )]);
    n.cache("demo", "demo-bin", BIN);
    let comps = n.components(vec![c.clone()], no_commands());

    // no acknowledgement: the exact notice and its hash, nothing else happens
    let err = comps.enable("demo", None).await.unwrap_err();
    let EnableError::ConsentRequired { text, hash } = &err else {
        panic!("{err:?}")
    };
    assert_eq!(text, &c.license.notice);
    assert_eq!(hash, &sha256_bytes(c.license.notice.as_bytes()));
    assert_eq!(hash, &c.notice_hash());
    let body = err.problem(Some(&c)).body();
    assert_eq!(body["code"], "consent_required");
    assert_eq!(body["consent"]["warning_text"], c.license.notice.as_str());
    assert!(!n.exists("components"), "nothing installed");
    assert!(!n.exists("bridge"), "nothing created");
    let s = comps.status("demo").await.unwrap();
    assert_eq!(
        s.state,
        ComponentState::Cached {
            bytes: BIN.len() as u64
        }
    );
    assert!(!s.acknowledged);

    // a wrong hash: asked again, still nothing
    let err = comps
        .enable("demo", Some(&"0".repeat(64)))
        .await
        .unwrap_err();
    assert!(
        matches!(err, EnableError::ConsentRequired { .. }),
        "{err:?}"
    );
    assert!(!n.exists("components"));
    assert!(!consent::load(&n.root).acknowledged(&c));

    // the right hash: recorded, installed, started, healthy
    let done = comps.enable("demo", Some(&c.notice_hash())).await.unwrap();
    assert_eq!(done.waiting, None);
    assert_eq!(done.status.state, ComponentState::Running);
    assert!(done.status.acknowledged);
    let rec = consent::load(&n.root).record("demo");
    assert!(rec.enabled);
    assert_eq!(rec.acknowledged.unwrap().notice_hash, c.notice_hash());
    // the adapter talks to it over piped stdio; `ready` was not consumed
    let io = comps.stdio("demo").await.unwrap().borrow().clone().unwrap();
    assert_eq!(
        io.recv_line().await.as_deref(),
        Some(r#"{"type":"ready","dest":"00"}"#)
    );
    io.write_line("ping").await.unwrap();
    assert_eq!(io.recv_line().await.as_deref(), Some("ping"));
    // the acknowledgement stays: a later enable needs no hash
    comps.disable("demo").await.unwrap();
    let again = comps.enable("demo", None).await.unwrap();
    assert_eq!(again.status.state, ComponentState::Running);
    comps.disable("demo").await.unwrap();
}

#[tokio::test]
async fn a_cache_that_fails_its_pin_installs_nothing() {
    // binary: wrong bytes under the pinned name
    let n = Node::new();
    let c = component(vec![(
        "demo.toml".into(),
        binary_toml("demo", BIN, &[], "").into_bytes(),
    )]);
    let tampered = b"#!/bin/sh\necho owned\n";
    n.cache("demo", "demo-bin", tampered);
    let comps = n.components(vec![c.clone()], no_commands());
    let err = comps
        .enable("demo", Some(&c.notice_hash()))
        .await
        .unwrap_err();
    assert!(matches!(err, EnableError::HashMismatch(_)), "{err:?}");
    assert_eq!(err.problem(Some(&c)).body()["reason_code"], "pin_mismatch");
    assert!(
        !n.exists("components/demo"),
        "fail closed: nothing installed"
    );
    let s = comps.status("demo").await.unwrap();
    assert_eq!(s.reason_code, Some("pin_mismatch"));
    // an extra file beside the pinned one is refused too
    n.cache("demo", "demo-bin", BIN);
    n.cache("demo", "README", b"hi");
    let err = comps.enable("demo", None).await.unwrap_err();
    assert!(matches!(err, EnableError::HashMismatch(_)), "{err:?}");
    // nothing cached at all
    std::fs::remove_dir_all(n.root.join("cache/packages/demo")).unwrap();
    let err = comps.enable("demo", None).await.unwrap_err();
    assert!(matches!(err, EnableError::NotCached(_)), "{err:?}");

    // pip: a cached file the lock does not pin; python3 never runs a venv
    let n = Node::new();
    let py = Arc::new(FakePython::new());
    let lock = lock_for(&[WHEEL]);
    let c = component(vec![
        ("helper.toml".into(), pip_toml("helper", None).into_bytes()),
        ("helper.lock.txt".into(), lock.into_bytes()),
    ]);
    std::fs::create_dir_all(n.root.join("current/share/helper")).unwrap();
    std::fs::write(n.root.join("current/share/helper/helper.py"), b"").unwrap();
    n.cache("helper", "rns-1.5.7-py3-none-any.whl", WHEEL);
    n.cache("helper", "evil-1.0-py3-none-any.whl", b"something else");
    let comps = n.components(vec![c.clone()], py.clone());
    let err = comps
        .enable("helper", Some(&c.notice_hash()))
        .await
        .unwrap_err();
    assert!(
        matches!(err, EnableError::HashMismatch(ref d) if d.contains("evil")),
        "{err:?}"
    );
    assert!(
        !py.calls().iter().any(|l| l.contains("-m venv")),
        "no venv was built: {:?}",
        py.calls()
    );
    assert!(!n.exists("venvs/helper") && !n.exists("venvs/helper.d"));
}

#[tokio::test]
async fn a_pip_closure_installs_offline_into_a_swapped_venv() {
    let n = Node::new();
    let py = Arc::new(FakePython::new());
    let lock = lock_for(&[WHEEL]);
    let c = component(vec![
        ("helper.toml".into(), pip_toml("helper", None).into_bytes()),
        ("helper.lock.txt".into(), lock.clone().into_bytes()),
    ]);
    std::fs::create_dir_all(n.root.join("current/share/helper")).unwrap();
    std::fs::write(n.root.join("current/share/helper/helper.py"), b"").unwrap();
    n.cache("helper", "rns-1.5.7-py3-none-any.whl", WHEEL);
    let comps = n.components(vec![c.clone()], py.clone());

    // the release must carry the script: checked with [requires]
    let done = comps
        .enable("helper", Some(&c.notice_hash()))
        .await
        .unwrap();
    assert!(matches!(
        done.status.state,
        ComponentState::Running | ComponentState::Installed
    ));
    // venvs/helper → helper.d/<build>, built from the lock, offline from the cache
    let link = n.root.join("venvs/helper");
    let target = std::fs::read_link(&link).expect("venvs/helper is a link");
    assert!(target.starts_with("helper.d"), "{target:?}");
    assert_eq!(
        std::fs::read_to_string(link.join("installed-lock.txt")).unwrap(),
        lock,
        "pip installed from the declaration's lock"
    );
    assert_eq!(
        std::fs::read_to_string(link.join(".111-integrity")).unwrap(),
        sha256_bytes(lock.as_bytes())
    );
    let calls = py.calls();
    let venv_call = calls.iter().position(|l| l.starts_with("python3 -m venv"));
    let pip_call = calls.iter().position(|l| l.contains("bin/pip install"));
    assert!(venv_call < pip_call && pip_call.is_some(), "{calls:?}");
    let pip = &calls[pip_call.unwrap()];
    let cache = n.root.join("cache/packages/helper");
    for arg in [
        "--require-hashes",
        "--no-index",
        &format!("--find-links {}", cache.display()),
    ] {
        assert!(pip.contains(arg), "{arg} in {pip}");
    }
    let s = comps.status("helper").await.unwrap();
    assert!(s.installed_bytes.is_some());
    comps.disable("helper").await.unwrap();
    assert_eq!(
        comps.status("helper").await.unwrap().state,
        ComponentState::Disabled
    );

    // re-enabling an unchanged pin installs nothing again
    let before = py.calls().iter().filter(|l| l.contains("-m venv")).count();
    comps.enable("helper", None).await.unwrap();
    let after = py.calls().iter().filter(|l| l.contains("-m venv")).count();
    assert_eq!(before, after, "offline and instant");
    comps.disable("helper").await.unwrap();

    // a failing pip leaves the venv in place untouched
    let mut changed_lock = lock_for(&[WHEEL]);
    changed_lock.push_str("# changed\n");
    let c2 = component(vec![
        ("helper.toml".into(), pip_toml("helper", None).into_bytes()),
        ("helper.lock.txt".into(), changed_lock.into_bytes()),
    ]);
    let comps2 = n.components(vec![c2], py.clone());
    py.fail_install(true);
    let err = comps2.enable("helper", None).await.unwrap_err();
    assert!(matches!(err, EnableError::InstallFailed(_)), "{err:?}");
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        target,
        "the old build stays"
    );
    assert_eq!(
        std::fs::read_dir(n.root.join("venvs/helper.d"))
            .unwrap()
            .count(),
        1,
        "the failed build was removed"
    );
}

#[tokio::test]
async fn disable_releases_claims_and_a_foreign_holder_is_caught() {
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let n = Node::new();
    let c = component(vec![(
        "demo.toml".into(),
        binary_toml("demo", BIN, &[port], "").into_bytes(),
    )]);
    n.cache("demo", "demo-bin", BIN);
    let comps = n.components(vec![c.clone()], no_commands());
    let key = format!("127.0.0.1:{port}");

    comps.enable("demo", Some(&c.notice_hash())).await.unwrap();
    let owner = n.ports.is_held(&key).expect("claimed while enabled");
    assert_eq!(owner.task, "component:demo");
    assert_eq!(owner.since_tick, 7);
    assert!(
        comps.stdio("demo").await.is_some(),
        "a runner while enabled"
    );
    let st = comps.disable("demo").await.unwrap();
    assert_eq!(st.state, ComponentState::Disabled);
    assert_eq!(n.ports.is_held(&key), None, "released");
    assert!(!consent::load(&n.root).record("demo").enabled);
    assert!(comps.stdio("demo").await.is_none(), "no runner, no process");
    // idempotent
    assert_eq!(
        comps.disable("demo").await.unwrap().state,
        ComponentState::Disabled
    );

    // a holder outside 111: only a bind sees it; nothing stays claimed
    let holder = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    let err = comps.enable("demo", None).await.unwrap_err();
    assert!(matches!(err, EnableError::PortBusy(_)), "{err:?}");
    assert_eq!(err.problem(Some(&c)).body()["code"], "port_busy");
    assert_eq!(n.ports.is_held(&key), None);
    drop(holder);
}

#[tokio::test]
async fn remove_files_leaves_nothing_and_the_cache_goes_only_on_its_own_call() {
    let n = Node::new();
    let c = component(vec![(
        "demo.toml".into(),
        binary_toml("demo", BIN, &[], "").into_bytes(),
    )]);
    n.cache("demo", "demo-bin", BIN);
    let comps = n.components(vec![c.clone()], no_commands());
    assert_eq!(
        comps.status("demo").await.unwrap().state,
        ComponentState::Cached {
            bytes: BIN.len() as u64
        }
    );
    comps.enable("demo", Some(&c.notice_hash())).await.unwrap();
    // running: removal refused
    assert_eq!(
        comps.remove_files("demo").await.unwrap_err(),
        RemoveError::Running
    );
    assert_eq!(
        comps.remove_cache("demo").await.unwrap_err(),
        RemoveError::Running
    );
    comps.disable("demo").await.unwrap();
    // positive control: it was all there
    for rel in [
        "components/demo/demo-bin",
        "bridge/demo",
        "cache/packages/demo/demo-bin",
    ] {
        assert!(n.exists(rel), "{rel} before removal");
    }
    let st = comps.remove_files("demo").await.unwrap();
    for rel in ["components", "bridge", "venvs/demo"] {
        assert!(!n.exists(rel), "{rel} removed");
    }
    assert!(n.exists("cache/packages/demo/demo-bin"), "the cache stays");
    assert_eq!(
        st.state,
        ComponentState::Cached {
            bytes: BIN.len() as u64
        }
    );
    let st = comps.remove_cache("demo").await.unwrap();
    assert!(!n.exists("cache/packages/demo"));
    assert_eq!(st.state, ComponentState::Declared);
    assert_eq!(st.cached_bytes, None);
    // and a fresh enable cannot find a payload any more
    let err = comps.enable("demo", None).await.unwrap_err();
    assert!(matches!(err, EnableError::NotCached(_)), "{err:?}");
}

#[tokio::test]
async fn a_component_starts_only_after_its_dependency_serves() {
    let n = Node::new();
    let toml = binary_toml("demo", BIN, &[], "")
        .replace("[run]", "[requires]\nservice = \"transport\"\n[run]")
        .replace("health_timeout_secs = 5", "health_timeout_secs = 2");
    let c = component(vec![("demo.toml".into(), toml.into_bytes())]);
    n.cache("demo", "demo-bin", BIN);
    let comps = n.components(vec![c.clone()], no_commands());

    // no rnsd at all: a [requires] failure, nothing installed
    let err = comps
        .enable("demo", Some(&c.notice_hash()))
        .await
        .unwrap_err();
    assert!(matches!(err, EnableError::RequiresUnmet(_)), "{err:?}");
    assert!(!n.exists("components"));

    // rnsd runs but does not serve: enabled and waiting, no child
    let rnsd = sleeper("rnsd");
    *n.deps.dep.lock().unwrap() = Some(rnsd.control());
    let done = comps.enable("demo", None).await.unwrap();
    assert_eq!(done.waiting, Some("not_serving"));
    assert_eq!(done.status.state, ComponentState::Installed);
    assert_eq!(done.status.reason_code, Some("not_serving"));
    assert!(comps.stdio("demo").await.unwrap().borrow().is_none());
    assert_eq!(comps.dependants(ServiceId::Transport).await, ["demo"]);

    // it serves: the child starts
    n.deps.serving.store(true, Ordering::SeqCst);
    let mut io = comps.stdio("demo").await.unwrap();
    tokio::time::timeout(LIMIT, io.wait_for(|i| i.is_some()))
        .await
        .expect("started once rnsd served")
        .unwrap();
    let t0 = std::time::Instant::now();
    while comps.status("demo").await.unwrap().state != ComponentState::Running {
        assert!(t0.elapsed() < LIMIT, "running once rnsd served");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // a planned rnsd restart: held before, back after
    assert!(comps.stop_before("demo").await);
    assert!(comps.stdio("demo").await.unwrap().borrow().is_none());
    comps.start_after("demo").await;
    tokio::time::timeout(LIMIT, io.wait_for(|i| i.is_some()))
        .await
        .expect("back after the restart")
        .unwrap();

    // rnsd stops: the component stops with it (never a helper without its instance)
    rnsd.stop_and_wait(LIMIT).await;
    tokio::time::timeout(LIMIT, io.wait_for(|i| i.is_none()))
        .await
        .expect("stopped with its dependency")
        .unwrap();
    comps.disable("demo").await.unwrap();
}

/// Records the order of notice lines and downloads (shared by the writer and
/// the fetcher).
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl std::io::Write for Log {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for line in String::from_utf8_lossy(buf).lines() {
            if !line.is_empty() {
                self.0.lock().unwrap().push(format!("print {line}"));
            }
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct FakeFetcher {
    log: Log,
    files: Vec<(String, Vec<u8>)>,
}

impl node111::install::packages::PayloadFetcher for FakeFetcher {
    fn fetch(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        self.log.0.lock().unwrap().push(format!("fetch {url}"));
        self.files
            .iter()
            .find(|(u, _)| u == url)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| anyhow::anyhow!("GET {url}: 404"))
    }
}

#[test]
fn install_prints_and_records_every_notice_before_any_download() {
    use node111::install::context::InstallContext;
    use node111::install::packages;
    use node111::install::receipt::Receipt;

    let n = Node::new();
    let good = component(vec![(
        "good.toml".into(),
        binary_toml("good", BIN, &[], "").into_bytes(),
    )]);
    let short = component(vec![(
        "short.toml".into(),
        binary_toml("short", BIN, &[], "").into_bytes(),
    )]);
    let gone = component(vec![(
        "gone.toml".into(),
        binary_toml("gone", BIN, &[], "").into_bytes(),
    )]);
    let lock = lock_for(&[WHEEL]);
    let helper = component(vec![
        ("helper.toml".into(), pip_toml("helper", None).into_bytes()),
        ("helper.lock.txt".into(), lock.into_bytes()),
    ]);
    let log = Log::default();
    let fetcher = FakeFetcher {
        log: log.clone(),
        files: vec![
            ("https://example.invalid/dl/good-bin".into(), BIN.to_vec()),
            // served with the wrong size: never lands in the cache
            (
                "https://example.invalid/dl/short-bin".into(),
                BIN[..10].to_vec(),
            ),
        ],
    };
    let root = n.root.to_string_lossy().into_owned();
    let tool = format!("{root}/{}", packages::COMPONENTS_PIP_VENV);
    let runner = MockRunner::new(vec![
        (format!("python3 -m venv {tool}"), Output { status: 0, stdout: String::new(), stderr: String::new() }),
        (
            format!(
                "{tool}/bin/pip download --require-hashes -r {tool}/helper-lock.txt -d {root}/cache/packages/helper"
            ),
            Output { status: 0, stdout: String::new(), stderr: String::new() },
        ),
    ]);
    let env = MapEnv::new();
    let ctx = InstallContext {
        layout: node111::daemon::DaemonLayout {
            root: n.root.clone(),
            etc: n.root.join("etc"),
        },
        fs_root: n.root.clone(),
        runner: &runner,
        env: &env,
        allow_degraded: false,
        min_installer_version: "2026.10.01".parse().unwrap(),
        manifest_base: String::new(),
        manifest_fallback: None,
        health_base: String::new(),
        probe_204: Vec::new(),
        check_root: false,
    };
    let mut receipt = Receipt::new("2026.10.19");
    let mut out = log.clone();
    let all = vec![good.clone(), short.clone(), gone.clone(), helper.clone()];
    packages::cache_components(&ctx, &all, &fetcher, &mut receipt, &mut out);
    runner.assert_exhausted();

    // every notice is printed, one line each, before the first download
    let events = log.0.lock().unwrap().clone();
    let first_fetch = events.iter().position(|e| e.starts_with("fetch ")).unwrap();
    let prints: Vec<&String> = events.iter().filter(|e| e.starts_with("print ")).collect();
    assert_eq!(prints.len(), 4, "{events:?}");
    assert!(
        events[..first_fetch]
            .iter()
            .filter(|e| e.starts_with("print "))
            .count()
            == 4,
        "all notices before any download: {events:?}"
    );
    for c in &all {
        assert!(
            prints.iter().any(|p| p.contains(&c.title)
                && p.contains(&c.license.spdx)
                && p.contains(&c.license.read_at)),
            "{} announced",
            c.id
        );
    }
    // and recorded in the receipt with the hash enable asks for
    let hashes: Vec<&str> = receipt.notices.iter().map(|n| n.hash.as_str()).collect();
    for c in &all {
        assert!(
            hashes.contains(&c.notice_hash().as_str()),
            "{} recorded",
            c.id
        );
    }
    // the good binary is cached; the short one and the missing one are degrade branches
    assert_eq!(
        std::fs::read(n.root.join("cache/packages/good/good-bin")).unwrap(),
        BIN
    );
    assert!(!n.exists("cache/packages/short/short-bin"));
    assert!(!n.exists("cache/packages/gone"));
    assert_eq!(
        receipt.degrade_branches.len(),
        2,
        "{:?}",
        receipt.degrade_branches
    );
    assert!(receipt.degrade_branches.iter().any(|b| b.contains("short")));
    assert!(receipt.degrade_branches.iter().any(|b| b.contains("gone")));
    assert!(!Path::new(&tool).exists(), "the throwaway venv is gone");
    // nothing installed, nothing running
    assert!(!n.exists("components") && !n.exists("venvs"));

    // a re-run keeps the verified cached file without downloading it again
    let log2 = Log::default();
    let fetcher2 = FakeFetcher {
        log: log2.clone(),
        files: Vec::new(),
    };
    let runner2 = MockRunner::new(Vec::<(String, Output)>::new());
    let ctx2 = InstallContext {
        runner: &runner2,
        ..ctx
    };
    let mut receipt2 = Receipt::new("2026.10.19");
    packages::cache_components(&ctx2, &[good], &fetcher2, &mut receipt2, &mut log2.clone());
    assert!(!log2
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|e| e.starts_with("fetch ")));
    assert!(receipt2.degrade_branches.is_empty());
    assert_eq!(receipt2.notices.len(), 1);
}
