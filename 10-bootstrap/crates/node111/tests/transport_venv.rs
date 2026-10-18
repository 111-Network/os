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
    std::fs::write(&rnsd, "#!/bin/sh\nsleep 300\n").unwrap();
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
    assert!(text.contains("schema-version = 1"), "{text}");
    assert!(text.contains("type = AutoInterface"));
    assert!(text.contains("group_id = 111.network"));
    assert!(!config::write_if_changed(&layout).unwrap());
    assert_eq!(config::existing_schema_version(&layout), Some(1));
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
