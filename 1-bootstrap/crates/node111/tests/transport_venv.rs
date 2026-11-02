//! T9 acceptance: venv integrity/rebuild with fake tooling, no-python stub,
//! config schema-version seam, live rnsd child bring-up, crash-loop → degraded.

mod common;

use node111::daemon::health::TransportHealth;
use node111::daemon::supervise::ChildState;
use node111::daemon::transport::{config, start, venv, TransportArgs};
use node111::daemon::DaemonLayout;
use node111::testkit::MockRunner;
use node111::util::run::Output;
use std::path::Path;

fn out(status: i32, stdout: &str) -> Output {
    Output {
        status,
        stdout: stdout.to_string(),
        stderr: String::new(),
    }
}

fn layout_at(root: &Path) -> DaemonLayout {
    DaemonLayout {
        root: root.to_path_buf(),
        etc: root.join("etc"),
    }
}

fn lockfile_at(root: &Path) -> std::path::PathBuf {
    let p = root.join("rns-lock.txt");
    std::fs::write(&p, b"rns==1.5.6 --hash=sha256:abcdef\n").unwrap();
    p
}

/// Fake intact venv: bin/rnsd (script that sleeps) + integrity marker.
fn fake_venv(layout: &DaemonLayout, lock: &Path) {
    let venv = venv::venv_dir(layout);
    std::fs::create_dir_all(venv.join("bin")).unwrap();
    let rnsd = venv.join("bin/rnsd");
    std::fs::write(&rnsd, "#!/bin/sh\nexec sleep 300\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&rnsd, std::fs::Permissions::from_mode(0o755)).unwrap();
    let sha = node111::util::hash::sha256_file(lock).unwrap();
    std::fs::write(venv.join(".111-integrity"), sha).unwrap();
}

#[test]
fn integrity_matrix() {
    let dir = tempfile::tempdir().unwrap();
    let layout = layout_at(dir.path());
    let lock = lockfile_at(dir.path());
    // nothing there
    assert!(!venv::integrity_ok(&layout, &lock));
    // intact
    fake_venv(&layout, &lock);
    assert!(venv::integrity_ok(&layout, &lock));
    // torn (marker mismatch)
    std::fs::write(venv::venv_dir(&layout).join(".111-integrity"), "deadbeef").unwrap();
    assert!(!venv::integrity_ok(&layout, &lock));
    // rnsd gone
    fake_venv(&layout, &lock);
    std::fs::remove_file(venv::venv_dir(&layout).join("bin/rnsd")).unwrap();
    assert!(!venv::integrity_ok(&layout, &lock));
}

#[test]
fn torn_venv_rebuilds_from_cache_offline() {
    let dir = tempfile::tempdir().unwrap();
    let layout = layout_at(dir.path());
    let lock = lockfile_at(dir.path());
    let cache = dir.path().join("cache/python");
    let venv = venv::venv_dir(&layout);
    let venv_s = venv.to_string_lossy().into_owned();
    let lock_s = lock.to_string_lossy().into_owned();
    let cache_s = cache.to_string_lossy().into_owned();
    let runner = MockRunner::new(vec![
        (format!("python3 -m venv {venv_s}"), out(0, "")),
        (
            format!("{venv_s}/bin/pip install --require-hashes -r {lock_s} --no-index --find-links {cache_s}"),
            out(0, ""),
        ),
    ]);
    venv::build(&layout, &runner, &lock, &cache, false).unwrap();
    // offline flags present, index never touched
    let calls = runner.calls();
    assert!(
        calls[1].contains("--no-index"),
        "daemon rebuilds strictly offline: {calls:?}"
    );
    // integrity marker written with the lockfile's hash
    let marker = std::fs::read_to_string(venv.join(".111-integrity")).unwrap();
    assert_eq!(marker, node111::util::hash::sha256_file(&lock).unwrap());
    runner.assert_exhausted();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_python_is_deliberate_stub_not_failure() {
    let dir = tempfile::tempdir().unwrap();
    let layout = layout_at(dir.path());
    let lock = lockfile_at(dir.path());
    let runner = MockRunner::new(vec![("python3 --version".to_string(), out(127, ""))]);
    let handle = start(TransportArgs {
        layout,
        runner: &runner,
        lockfile: lock,
        cache_py: dir.path().join("cache/python"),
        allow_index: false,
    })
    .await;
    assert_eq!(handle.health(), TransportHealth::DeliberatelyAbsent);
    assert_eq!(
        handle.display_state(),
        node111::daemon::dashboard::TransportState::NoTransport
    );
    runner.assert_exhausted();
}

#[test]
fn config_render_and_schema_seam() {
    let dir = tempfile::tempdir().unwrap();
    let layout = layout_at(dir.path());
    // first write creates; second is a no-op
    assert!(config::write_if_changed(&layout).unwrap());
    let text = std::fs::read_to_string(config::config_path(&layout)).unwrap();
    assert!(text.contains("schema-version = 2"), "{text}");
    assert!(text.contains("type = AutoInterface"));
    assert!(text.contains("group_id = 111.network"));
    assert!(!config::write_if_changed(&layout).unwrap());
    assert_eq!(
        config::existing_schema_version(&layout),
        Some(config::SCHEMA_VERSION)
    );
    // a newer schema version (Layer 6) is read back correctly
    std::fs::write(config::config_path(&layout), "schema-version = 7\n").unwrap();
    assert_eq!(config::existing_schema_version(&layout), Some(7));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rnsd_child_runs_and_health_maps() {
    let dir = tempfile::tempdir().unwrap();
    let layout = layout_at(dir.path());
    let lock = lockfile_at(dir.path());
    fake_venv(&layout, &lock);
    let runner = MockRunner::new(vec![(
        "python3 --version".to_string(),
        out(0, "Python 3.12\n"),
    )]);
    let handle = start(TransportArgs {
        layout: layout.clone(),
        runner: &runner,
        lockfile: lock,
        cache_py: dir.path().join("cache/python"),
        allow_index: false,
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(handle.health(), TransportHealth::Alive);
    // config rendered for the child
    assert!(config::config_path(&layout).exists());
    assert_eq!(
        handle.child_state(),
        Some(node111::daemon::supervise::ChildState::Running)
    );
    // stop the world cleanly
    handle.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crash_loop_maps_to_degraded() {
    let dir = tempfile::tempdir().unwrap();
    let layout = layout_at(dir.path());
    let lock = lockfile_at(dir.path());
    fake_venv(&layout, &lock);
    // make rnsd a crasher
    let rnsd = venv::venv_dir(&layout).join("bin/rnsd");
    std::fs::write(&rnsd, "#!/bin/sh\nexit 1\n").unwrap();
    let runner = MockRunner::new(vec![(
        "python3 --version".to_string(),
        out(0, "Python 3.12\n"),
    )]);
    let handle = start(TransportArgs {
        layout: layout.clone(),
        runner: &runner,
        lockfile: lock,
        cache_py: dir.path().join("cache/python"),
        allow_index: false,
    })
    .await;
    // default policy: 5 crashes within 60s → Degraded (250ms→ backoff, quick)
    tokio::time::sleep(std::time::Duration::from_secs(8)).await;
    assert_eq!(handle.health(), TransportHealth::Degraded);
    assert_eq!(
        handle.display_state(),
        node111::daemon::dashboard::TransportState::Degraded
    );
    assert_eq!(handle.child_state(), Some(ChildState::Degraded));
}

// ---------------------------------------------------------------- FA1: rnsd owns its instance
//
// A stub `rnsd` (Python: a shell cannot bind an abstract unix socket) that,
// per spawn N, does what `<root>/rnsd-mode-N` says: `bind` the shared
// instance socket `\0rns/<instance_name>` from the rendered config, `wait`
// for `<root>/bind-now` and then bind, `client` (log rnsd's "connected to
// another shared local instance" line and never bind) or `silent` (never
// bind, log nothing). A fake rnstatus logs each run to
// `<root>/rnstatus-calls.log`. Each test names its own instance, so tests in
// parallel never share a socket. Skipped (with a line) without python3.

const STUB_RNSD: &str = r#"#!/usr/bin/env python3
import os, socket, sys, time
root = os.environ["HOME"]
cnt = os.path.join(root, "rnsd-spawns")
n = int(open(cnt).read() or "0") + 1 if os.path.exists(cnt) else 1
open(cnt, "w").write(str(n))
mf = os.path.join(root, "rnsd-mode-%d" % n)
mode = open(mf).read().strip() if os.path.exists(mf) else "bind"
cfg = os.path.join(sys.argv[sys.argv.index("--config") + 1], "config")
inst = "default"
for line in open(cfg):
    k, _, v = line.partition("=")
    if k.strip() == "instance_name":
        inst = v.strip()
print("stub rnsd spawn %d mode %s instance %s" % (n, mode, inst), flush=True)
if mode == "client":
    print("[Warning] Started rnsd version 1.5.7 connected to another shared local instance, this is probably NOT what you want!", flush=True)
if mode in ("client", "silent"):
    while True:
        time.sleep(1)
if mode == "wait":
    while not os.path.exists(os.path.join(root, "bind-now")):
        time.sleep(0.05)
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind("\0rns/" + inst)
s.listen(4)
print("stub rnsd serving", flush=True)
while True:
    time.sleep(1)
"#;

fn python3_present() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn unique_instance(tag: &str) -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() % 1_000_000)
        .unwrap_or(0);
    format!("fa1{tag}{}x{n}", std::process::id() % 1_000_000)
}

/// A started-able root: fake venv with the stub rnsd and a logging fake
/// rnstatus, interfaces.toml naming `instance`, the spawn modes in order.
fn fa1_root(root: &Path, instance: &str, modes: &[&str]) -> (DaemonLayout, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let layout = layout_at(root);
    let lock = lockfile_at(root);
    fake_venv(&layout, &lock);
    let bin = venv::venv_dir(&layout).join("bin");
    std::fs::write(bin.join("rnsd"), STUB_RNSD).unwrap();
    let calls = root.join("rnstatus-calls.log");
    std::fs::write(
        bin.join("rnstatus"),
        format!(
            "#!/bin/sh\necho \"$*\" >>'{}'\necho ' Shared Instance[rns/{instance}]'\necho '    Status    : Up'\necho '    Serving   : 0 programs'\n",
            calls.display()
        ),
    )
    .unwrap();
    for p in ["rnsd", "rnstatus"] {
        std::fs::set_permissions(bin.join(p), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    for (i, m) in modes.iter().enumerate() {
        std::fs::write(root.join(format!("rnsd-mode-{}", i + 1)), m).unwrap();
    }
    let mut f = node111::daemon::transport::interfaces::InterfacesFile::default();
    f.posture.instance_name = instance.to_string();
    node111::daemon::transport::interfaces::save(&layout.state_dir(), &f).unwrap();
    (layout, lock)
}

fn rnstatus_runs(root: &Path) -> usize {
    std::fs::read_to_string(root.join("rnstatus-calls.log"))
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

fn spawns(root: &Path) -> u32 {
    std::fs::read_to_string(root.join("rnsd-spawns"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

async fn until(limit: std::time::Duration, mut f: impl FnMut() -> bool) -> bool {
    let t0 = std::time::Instant::now();
    while t0.elapsed() < limit {
        if f() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    f()
}

async fn start_at(
    layout: &DaemonLayout,
    lock: std::path::PathBuf,
) -> node111::daemon::transport::TransportHandle {
    let runner = MockRunner::new(vec![(
        "python3 --version".to_string(),
        out(0, "Python 3.12\n"),
    )]);
    start(TransportArgs {
        layout: layout.clone(),
        runner: &runner,
        lockfile: lock,
        cache_py: layout.root.join("cache/python"),
        allow_index: false,
    })
    .await
}

/// FA1 (2): the components runner's serving gate and the applier's probe
/// start no rnstatus while rnsd runs but does not hold `@rns/<instance>`;
/// both go through once the supervised pid holds it. On the old logic the
/// probe and the gate each ran rnstatus at once (and that rnstatus could
/// become the instance).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fa1_no_rnstatus_until_the_supervised_rnsd_owns_the_instance() {
    use node111::daemon::components::runner::{
        rnstatus_serving, ComponentRunner, Gate, RunnerConfig, RunnerStatus,
    };
    use node111::daemon::devices::applier::{ApplyProbe, SpineProbe};
    use node111::daemon::supervise::{BackoffPolicy, ChildSpec};
    use node111::daemon::transport::posture::RnsdChild;
    use node111::daemon::transport::rnstatus::owner;
    use std::sync::Arc;
    use std::time::Duration;
    if !python3_present() {
        eprintln!("SKIP: python3 absent (the stub rnsd is Python)");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let inst = unique_instance("g");
    let (layout, lock) = fa1_root(&root, &inst, &["wait"]);
    let handle = start_at(&layout, lock).await;
    let control = handle.control().expect("rnsd is supervised");
    assert!(until(Duration::from_secs(10), || control.pid().is_some()).await);
    let pid = control.pid().unwrap();
    assert_eq!(handle.child_state(), Some(ChildState::Running));

    // the applier's probe and the runner's serving probe, rnsd up but not owning
    let probe = SpineProbe::new(
        handle.spine(),
        Some(Arc::new(control.clone()) as Arc<dyn RnsdChild>),
        None,
    );
    assert!(probe.rnsd_running());
    let r = probe.interfaces().await;
    assert!(
        matches!(&r, Err(e) if e.contains("not run")),
        "the probe answers without rnstatus: {r:?}"
    );
    assert!(!rnstatus_serving(&root));
    // a component ordered after transport waits `not_serving`
    let gate = Gate {
        dependency: {
            let c = control.clone();
            Arc::new(move || Some(c.clone()))
        },
        serving: {
            let r = root.clone();
            Arc::new(move || rnstatus_serving(&r))
        },
    };
    let spec = ChildSpec {
        name: "fa1-helper".into(),
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "exec sleep 300".into()],
        env: vec![],
        backoff: BackoffPolicy::default(),
        stdout_log: None,
    };
    let cfg = RunnerConfig {
        retry: Duration::from_millis(100),
        retry_max: Duration::from_millis(200),
        settle: Duration::from_millis(100),
        grace: Duration::from_secs(2),
    };
    let comp = ComponentRunner::start(spec, Some(gate), cfg);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        comp.status(),
        RunnerStatus::Waiting {
            reason: "not_serving"
        }
    );
    assert!(comp.child().is_none(), "nothing spawned before rnsd serves");
    assert_eq!(
        rnstatus_runs(&root),
        0,
        "no rnstatus ran while rnsd did not hold @rns/{inst}"
    );

    // rnsd takes the socket: the probe reads, the component starts
    std::fs::write(root.join("bind-now"), b"").unwrap();
    assert!(
        until(Duration::from_secs(10), || owner::owned_by(&root, pid)).await,
        "the stub rnsd holds @rns/{inst}"
    );
    assert_eq!(owner::shared_instance_owner(&root, Some(pid)), Some(pid));
    let facts = probe.interfaces().await.expect("rnstatus runs now");
    assert!(
        facts.iter().any(|f| f.kind == "Shared Instance" && f.up),
        "{facts:?}"
    );
    assert!(
        until(Duration::from_secs(10), || matches!(
            comp.status(),
            RunnerStatus::Running { .. }
        ))
        .await,
        "the component starts once rnsd serves: {:?}",
        comp.status()
    );
    assert!(
        rnstatus_runs(&root) >= 2,
        "the probe and the gate ran rnstatus"
    );
    assert_eq!(
        handle.client_restarts(),
        0,
        "an owning rnsd is never restarted"
    );
    assert_eq!(control.pid(), Some(pid));
    comp.shutdown().await;
    handle.stop_and_wait(Duration::from_secs(5)).await;
    assert!(!rnstatus_serving(&root), "unregistered at stop");
}

/// FA1 (3): an rnsd respawned as a client of another holder (its log line,
/// no bind) is terminated and respawned; the next spawn owns the instance.
/// On the old logic the supervisor kept the client rnsd Running for good.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fa1_a_respawned_rnsd_that_attached_as_a_client_is_restarted() {
    use node111::daemon::transport::rnstatus::owner;
    use std::time::Duration;
    if !python3_present() {
        eprintln!("SKIP: python3 absent (the stub rnsd is Python)");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let inst = unique_instance("c");
    let (layout, lock) = fa1_root(&root, &inst, &["bind", "client", "bind"]);
    let handle = start_at(&layout, lock).await;
    let control = handle.control().unwrap();
    assert!(
        until(Duration::from_secs(10), || control
            .pid()
            .is_some_and(|p| owner::owned_by(&root, p)))
        .await,
        "the first rnsd owns @rns/{inst}"
    );
    let p1 = control.pid().unwrap();
    // the crash: the respawn (spawn 2) attaches as a client
    unsafe { libc::kill(p1 as libc::pid_t, libc::SIGKILL) };
    assert!(
        until(Duration::from_secs(20), || spawns(&root) >= 3
            && control
                .pid()
                .is_some_and(|p| p != p1 && owner::owned_by(&root, p)))
        .await,
        "the client rnsd was restarted and spawn 3 owns the instance (spawns {}, restarts {})",
        spawns(&root),
        handle.client_restarts()
    );
    assert_eq!(handle.client_restarts(), 1);
    let log = std::fs::read_to_string(root.join("rns/rnsd.log")).unwrap();
    assert!(
        log.contains("connected to another shared local instance"),
        "{log}"
    );
    // it stays owned: nothing more is restarted
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(spawns(&root), 3);
    assert_eq!(handle.client_restarts(), 1);
    handle.stop_and_wait(Duration::from_secs(5)).await;
}

/// FA1 (3): an rnsd that never holds the instance (and logs nothing) is
/// terminated at the end of the settle; after `max_consecutive` such
/// terminations it is left alone (the gates stay closed).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fa1_an_rnsd_that_never_owns_is_restarted_at_the_settle_end_then_left() {
    use node111::daemon::supervise::{spawn_child, BackoffPolicy, ChildSpec};
    use node111::daemon::transport::rnstatus::{owner, Rnstatus};
    use node111::daemon::transport::{verify_spawns, SpawnCheck};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    if !python3_present() {
        eprintln!("SKIP: python3 absent (the stub rnsd is Python)");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let inst = unique_instance("s");
    let (layout, _lock) = fa1_root(&root, &inst, &["silent", "bind"]);
    std::fs::create_dir_all(root.join("rns")).unwrap();
    std::fs::write(
        root.join("rns/config"),
        format!("[reticulum]\n  share_instance = Yes\n  instance_name = {inst}\n"),
    )
    .unwrap();
    let spec = |log: &str| ChildSpec {
        name: "rnsd".into(),
        program: venv::venv_dir(&layout).join("bin/rnsd"),
        args: vec![
            "--config".into(),
            root.join("rns").to_string_lossy().into_owned(),
        ],
        env: vec![("HOME".into(), root.to_string_lossy().into_owned())],
        backoff: BackoffPolicy {
            initial_ms: 50,
            max_ms: 200,
            factor: 2.0,
            jitter_pct: 0,
            crash_loop_threshold: 50,
            crash_loop_window_secs: 60,
        },
        stdout_log: Some(root.join(log)),
    };
    let rn = Arc::new(Rnstatus::new(
        Arc::new(node111::daemon::transport::rnstatus::TimedRunner::new(
            &root,
        )),
        &root,
    ));
    let cfg = SpawnCheck {
        settle: Duration::from_millis(1500),
        poll: Duration::from_millis(100),
        grace: Duration::from_secs(2),
        max_consecutive: 2,
    };
    // spawn 1 silent → terminated at the settle end; spawn 2 binds
    let child = spawn_child(spec("rnsd.log"));
    let restarts = Arc::new(AtomicU64::new(0));
    let task = tokio::spawn(verify_spawns(
        child.control(),
        Arc::clone(&rn),
        root.join("rnsd.log"),
        cfg.clone(),
        Arc::clone(&restarts),
    ));
    assert!(
        until(Duration::from_secs(15), || child
            .pid()
            .is_some_and(|p| owner::owned_by(&root, p)))
        .await,
        "spawn 2 owns the instance (spawns {})",
        spawns(&root)
    );
    assert_eq!(restarts.load(Ordering::SeqCst), 1);
    assert_eq!(spawns(&root), 2);
    child.stop_and_wait(Duration::from_secs(5)).await;
    task.abort();

    // never owning: two terminations, then it is left running
    for n in 3..=8 {
        std::fs::write(root.join(format!("rnsd-mode-{n}")), "silent").unwrap();
    }
    let child = spawn_child(spec("rnsd2.log"));
    let restarts = Arc::new(AtomicU64::new(0));
    let task = tokio::spawn(verify_spawns(
        child.control(),
        rn,
        root.join("rnsd2.log"),
        cfg,
        Arc::clone(&restarts),
    ));
    assert!(
        until(Duration::from_secs(20), || restarts.load(Ordering::SeqCst)
            >= 2
            && spawns(&root) >= 5)
        .await
    );
    let p = child.pid();
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(
        restarts.load(Ordering::SeqCst),
        2,
        "capped at max_consecutive"
    );
    assert_eq!(child.pid(), p, "the last one is left running");
    assert_eq!(spawns(&root), 5);
    child.stop_and_wait(Duration::from_secs(5)).await;
    task.abort();
}
