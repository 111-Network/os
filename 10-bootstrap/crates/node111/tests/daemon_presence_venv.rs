//! Layer 5 §14 at daemon start (os/50-messaging/README.md §14, §19 L1):
//! `daemon::ensure_presence_venv` — what `111 daemon` runs before it builds
//! the Meshtastic module — rebuilds `venvs/presence` offline when the release
//! in `current/` carries a lock the venv in place was not built from (an
//! update whose lock changed was promoted), runs nothing for an unchanged
//! lock or a release without the reader, and keeps the earlier venv when the
//! rebuild fails. A fake `python3`/pip (`fake_python`) stands in for Python.

mod fake_python;

use node111::daemon::transport::venv::presence::{self, Mode, Outcome, PresenceVenvError};
use node111::daemon::DaemonLayout;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const LOCK_A: &[u8] = b"meshtastic==2.7.11 --hash=sha256:aaaa\n";
const LOCK_B: &[u8] = b"meshtastic==2.7.12 --hash=sha256:bbbb\n";

/// `releases/<ver>/` carrying `lock` (none: a release without the reader).
fn release(layout: &DaemonLayout, ver: &str, lock: Option<&[u8]>) -> PathBuf {
    let dir = layout.root.join("releases").join(ver);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    if let Some(lock) = lock {
        let path = presence::lockfile(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, lock).unwrap();
    }
    dir
}

/// Point `current` at `releases/<ver>` (a relative link, as the installer
/// and the root helper make it).
fn promote(layout: &DaemonLayout, ver: &str) {
    let current = layout.current_release();
    let _ = std::fs::remove_file(&current);
    std::os::unix::fs::symlink(Path::new("releases").join(ver), &current).unwrap();
}

fn installed_lock(root: &Path) -> Vec<u8> {
    std::fs::read(presence::venv_dir(root).join("installed-lock.txt")).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_start_rebuilds_the_presence_venv_only_for_a_changed_lock() {
    let dir = tempfile::tempdir().unwrap();
    let layout = DaemonLayout {
        root: dir.path().join("var/lib/111"),
        etc: dir.path().join("etc/111"),
    };
    let root = layout.root.clone();
    let py = Arc::new(fake_python::FakePython::new());

    // `111 install` built the venv from release A's lock
    let a = release(&layout, "2026.10.12", Some(LOCK_A));
    promote(&layout, "2026.10.12");
    let cache = presence::cache_dir(&root);
    assert_eq!(
        presence::ensure(&root, &*py, &presence::lockfile(&a), &cache, Mode::Install),
        Outcome::Built
    );
    let built_a = std::fs::read_link(presence::venv_dir(&root)).unwrap();

    // a daemon start on the same release: nothing runs at all
    let before = py.calls().len();
    let out = node111::daemon::ensure_presence_venv(&layout, py.clone()).await;
    assert_eq!(out, Outcome::Unchanged);
    assert_eq!(py.calls().len(), before, "{:?}", &py.calls()[before..]);
    assert_eq!(
        std::fs::read_link(presence::venv_dir(&root)).unwrap(),
        built_a
    );

    // an update with a changed lock is promoted: the next start rebuilds,
    // offline from the cache (nothing downloads at runtime)
    let b = release(&layout, "2026.10.19", Some(LOCK_B));
    promote(&layout, "2026.10.19");
    let before = py.calls().len();
    let out = node111::daemon::ensure_presence_venv(&layout, py.clone()).await;
    assert_eq!(out, Outcome::Built);
    let calls = py.calls()[before..].to_vec();
    let lock_b = presence::lockfile(&b);
    // the lock is read through `current/` (whatever release it names)
    assert!(
        calls.iter().any(|c| c.ends_with(&format!(
            "/bin/pip install --require-hashes -r {} --no-index --find-links {}",
            presence::lockfile(&layout.current_release()).display(),
            cache.display()
        ))),
        "{calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.contains(" download ")),
        "the daemon downloaded: {calls:?}"
    );
    assert_eq!(installed_lock(&root), LOCK_B);
    assert!(presence::integrity_ok(&root, &lock_b));
    assert_ne!(
        std::fs::read_link(presence::venv_dir(&root)).unwrap(),
        built_a
    );
    let status = presence::status(&root).unwrap();
    assert!(status.ok, "{status:?}");
    assert_eq!(
        status.lock_sha256,
        node111::util::hash::sha256_bytes(LOCK_B)
    );

    // … and the start after that runs nothing again
    let before = py.calls().len();
    let out = node111::daemon::ensure_presence_venv(&layout, py.clone()).await;
    assert_eq!(out, Outcome::Unchanged);
    assert_eq!(py.calls().len(), before, "{:?}", &py.calls()[before..]);

    // a release without the reader: nothing runs, nothing changes
    release(&layout, "2026.10.20", None);
    promote(&layout, "2026.10.20");
    let out = node111::daemon::ensure_presence_venv(&layout, py.clone()).await;
    assert_eq!(out, Outcome::NotShipped);
    assert_eq!(py.calls().len(), before);
    assert_eq!(installed_lock(&root), LOCK_B);

    // a changed lock whose rebuild fails (a wheel the offline cache lacks):
    // the earlier venv stays, and the status says so for the service row
    py.fail_install(true);
    release(&layout, "2026.10.21", Some(LOCK_A));
    promote(&layout, "2026.10.21");
    let out = node111::daemon::ensure_presence_venv(&layout, py.clone()).await;
    let Outcome::Unavailable {
        error: PresenceVenvError::Pip { offline: true, .. },
        previous_kept: true,
    } = out
    else {
        panic!("{out:?}");
    };
    assert_eq!(installed_lock(&root), LOCK_B, "the venv in place stays");
    let reason = presence::unavailable_reason(&root).unwrap();
    assert!(reason.contains("runs on the one built earlier"), "{reason}");
    assert!(reason.contains("offline cache"), "{reason}");
}
