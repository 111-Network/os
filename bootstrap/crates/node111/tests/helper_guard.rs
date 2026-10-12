//! T11 acceptance: root update-helper + update-guard on temp roots with an
//! injected clock + scripted systemctl — promote happy path, bad staged hash,
//! replayed manifest, probation finalize + prune, prune-during-probation
//! refused, F2 crash-before-READY revert, F3 READY-but-503 revert.

mod common;

use node111::daemon::DaemonLayout;
use node111::probation::{Journal, PROBATION_SECS};
use node111::update_guard::{self, GuardReport};
use node111::update_helper::{self, HelperAction};
use node111::util::run::{Output, SystemRunner};
use std::path::{Path, PathBuf};

fn ok_out() -> Output {
    Output {
        status: 0,
        stdout: String::new(),
        stderr: String::new(),
    }
}

fn layout_at(root: &Path) -> DaemonLayout {
    std::fs::create_dir_all(root.join("state")).unwrap();
    DaemonLayout {
        root: root.to_path_buf(),
        etc: root.join("etc"),
    }
}

/// Create a release dir with a valid tree (bin/111 + share/kb + manifest).
fn make_release(root: &Path, version: &str) -> PathBuf {
    let dest = root.join("releases").join(version);
    std::fs::create_dir_all(&dest).unwrap();
    let tarball = common::build_payload();
    node111::daemon::updater::stage::stage_payload(
        &tarball,
        &node111::manifest::PinnedArtifact {
            name: "x86_64-unknown-linux-musl".into(),
            url: "http://x".into(),
            sha256: node111::util::hash::sha256_bytes(&tarball),
            size: tarball.len() as u64,
        },
        &root.join("releases"),
        &version.parse().unwrap(),
    )
    .unwrap();
    // stage_payload builds <ver>/{payload.tar.gz,tree}; promote layout = tree contents at top
    let staged = root.join("releases").join(version);
    let tree = staged.join("tree");
    for entry in std::fs::read_dir(&tree).unwrap() {
        let entry = entry.unwrap();
        std::fs::rename(entry.path(), staged.join(entry.file_name())).unwrap();
    }
    std::fs::remove_dir_all(staged.join("tree")).unwrap();
    std::fs::remove_file(staged.join("payload.tar.gz")).unwrap();
    dest
}

fn flip_to(root: &Path, version: &str) {
    let target = root.join("releases").join(version);
    let tmp = root.join(".current.tmp");
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(&target, &tmp).unwrap();
    std::fs::rename(&tmp, root.join("current")).unwrap();
}

fn current_basename(root: &Path) -> String {
    std::fs::read_link(root.join("current"))
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// Build a signed staging bundle (manifest + minisig + tarball) at staging/<ver>.
fn make_staging(root: &Path, version: &str) {
    let staged = root.join("state/staging").join(version);
    std::fs::create_dir_all(&staged).unwrap();
    let tarball = common::build_payload();
    let manifest = format!(
        r#"manifest-version = 1
date = "{version}"
channel = "stable"
release = "{version}"
min-upgrade-from = "2026.10.01"

[pkg.111.target.x86_64-unknown-linux-musl]
url = "https://example.invalid/111"
sha256 = "{a64}"
size = 100

[payload.x86_64-unknown-linux-musl]
url = "https://example.invalid/payload.tar.gz"
sha256 = "{tsha}"
size = {tlen}

[rns-lock]
url = "https://example.invalid/rns-lock.txt"
sha256 = "{a64}"
size = 100

[kb]
url = "https://example.invalid/kb.tar.gz"
sha256 = "{a64}"
size = 100

[[keychain]]
generation = 0
pubkey = "RWRoot00000000000000000000000000000000000000000="

[[keychain]]
generation = 1
pubkey = "RWGen100000000000000000000000000000000000000000="
signature_by_previous = "untrusted comment: fixture\\nplaceholder"
"#,
        a64 = "a".repeat(64),
        tsha = node111::util::hash::sha256_bytes(&tarball),
        tlen = tarball.len()
    );
    std::fs::write(staged.join("payload.tar.gz"), &tarball).unwrap();
    let manifest_path = staged.join("channel-111-stable.toml");
    std::fs::write(&manifest_path, &manifest).unwrap();
    common::sign_file(&common::minisign_fixture_key(), &manifest_path);

    // daemon-side update-request
    let req = serde_json::json!({
        "schema": 1,
        "version": version,
        "manifest_sha256": node111::util::hash::sha256_bytes(manifest.as_bytes()),
        "staged_path": staged.to_string_lossy(),
        "requested_at": "2026-10-12T00:00:00Z",
    });
    std::fs::write(root.join("state/update-request.json"), req.to_string()).unwrap();
}

#[test]
fn promote_happy_path() {
    let node = tempfile::tempdir().unwrap();
    make_release(node.path(), "2026.10.01");
    flip_to(node.path(), "2026.10.01");
    make_staging(node.path(), "2026.10.12");
    let layout = layout_at(node.path());
    let runner =
        node111::testkit::MockRunner::new(vec![("systemctl restart 111node.service", ok_out())]);

    let report = update_helper::run_with(&layout, &runner, 1_760_000_000).unwrap();
    assert!(report.actions.contains(&HelperAction::Promoted {
        version: "2026.10.12".into()
    }));

    // releases/<new> exists with the tree, current flipped, journal pending,
    // floor advanced, request consumed, restart called.
    assert!(node.path().join("releases/2026.10.12/bin/111").exists());
    assert_eq!(current_basename(node.path()), "2026.10.12");
    let j = Journal::read(&node.path().join("state/update.json"))
        .unwrap()
        .unwrap();
    assert!(j.pending);
    assert_eq!(j.previous, "2026.10.01");
    assert_eq!(j.deadline_epoch, 1_760_000_000 + PROBATION_SECS);
    let floor = node111::floor::Floor::read(&node.path().join("state/floor.json"))
        .unwrap()
        .unwrap();
    assert_eq!(floor.highest_release, "2026.10.12");
    assert!(!node.path().join("state/update-request.json").exists());
    runner.assert_exhausted();
}

#[test]
fn tampered_staged_tarball_refused_no_flip() {
    let node = tempfile::tempdir().unwrap();
    make_release(node.path(), "2026.10.01");
    flip_to(node.path(), "2026.10.01");
    make_staging(node.path(), "2026.10.12");
    // corrupt the staged tarball (node111-writable area — root must catch it)
    let tarball = node.path().join("state/staging/2026.10.12/payload.tar.gz");
    let mut b = std::fs::read(&tarball).unwrap();
    b[10] ^= 0xFF;
    std::fs::write(&tarball, &b).unwrap();

    let layout = layout_at(node.path());
    let runner = node111::testkit::MockRunner::new(Vec::<(String, Output)>::new());
    let err = update_helper::run_with(&layout, &runner, 1_760_000_000).unwrap_err();
    assert!(err.to_string().contains("sha256"), "{err}");
    assert_eq!(current_basename(node.path()), "2026.10.01");
    assert!(!node.path().join("releases/2026.10.12").exists());
    assert!(Journal::read(&node.path().join("state/update.json"))
        .unwrap()
        .is_none());
}

#[test]
fn replayed_manifest_below_floor_refused() {
    let node = tempfile::tempdir().unwrap();
    make_release(node.path(), "2026.10.01");
    flip_to(node.path(), "2026.10.01");
    make_staging(node.path(), "2026.10.12");
    node111::floor::Floor {
        schema: 1,
        highest_release: "2026.10.13".into(),
        highest_manifest_date: "2026.10.13".into(),
        highest_keychain_generation: 1,
    }
    .write(&node.path().join("state/floor.json"))
    .unwrap();
    let layout = layout_at(node.path());
    let runner = node111::testkit::MockRunner::new(Vec::<(String, Output)>::new());
    assert!(update_helper::run_with(&layout, &runner, 1_760_000_000).is_err());
    assert_eq!(current_basename(node.path()), "2026.10.01");
}

#[test]
fn finalize_confirms_and_prunes_to_two() {
    let node = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(node.path().join("state")).unwrap();
    make_release(node.path(), "2026.10.01");
    make_release(node.path(), "2026.10.05");
    make_release(node.path(), "2026.10.12");
    flip_to(node.path(), "2026.10.12");
    Journal {
        schema: 1,
        pending: true,
        version: "2026.10.12".into(),
        previous: "2026.10.05".into(),
        deadline_epoch: 1_760_000_600,
        health_confirmed: false,
    }
    .write(&node.path().join("state/update.json"))
    .unwrap();
    std::fs::write(node.path().join("state/health-confirmed.json"), "{}").unwrap();

    let layout = layout_at(node.path());
    let runner = node111::testkit::MockRunner::new(Vec::<(String, Output)>::new());
    let report = update_helper::run_with(&layout, &runner, 1_760_000_100).unwrap();
    assert!(report
        .actions
        .iter()
        .any(|a| matches!(a, HelperAction::Finalized { .. })));
    let j = Journal::read(&node.path().join("state/update.json"))
        .unwrap()
        .unwrap();
    assert!(!j.pending);
    assert!(j.health_confirmed);
    // exactly 2 kept: current + newest other
    assert!(node.path().join("releases/2026.10.12").exists());
    assert!(node.path().join("releases/2026.10.05").exists());
    assert!(!node.path().join("releases/2026.10.01").exists());
}

#[test]
fn f2_crash_before_ready_guard_reverts() {
    let node = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(node.path().join("state")).unwrap();
    make_release(node.path(), "2026.10.01");
    make_release(node.path(), "2026.10.12");
    flip_to(node.path(), "2026.10.12");
    Journal {
        schema: 1,
        pending: true,
        version: "2026.10.12".into(),
        previous: "2026.10.01".into(),
        deadline_epoch: 1_760_000_000, // in the past at guard time
        health_confirmed: false,
    }
    .write(&node.path().join("state/update.json"))
    .unwrap();

    let layout = layout_at(node.path());
    let runner = node111::testkit::MockRunner::new(vec![(
        "systemctl reset-failed 111node.service",
        ok_out(),
    )]);
    let report: GuardReport = update_guard::run_with(&layout, &runner, 1_760_000_600).unwrap();
    assert_eq!(report.reverted.as_deref(), Some("2026.10.01"));
    assert_eq!(current_basename(node.path()), "2026.10.01");
    let j = Journal::read(&node.path().join("state/update.json"))
        .unwrap()
        .unwrap();
    assert!(!j.pending);
    runner.assert_exhausted();
}

#[test]
fn guard_leaves_open_probation_alone() {
    let node = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(node.path().join("state")).unwrap();
    make_release(node.path(), "2026.10.01");
    make_release(node.path(), "2026.10.12");
    flip_to(node.path(), "2026.10.12");
    Journal {
        schema: 1,
        pending: true,
        version: "2026.10.12".into(),
        previous: "2026.10.01".into(),
        deadline_epoch: 1_760_000_600, // still open
        health_confirmed: false,
    }
    .write(&node.path().join("state/update.json"))
    .unwrap();
    let layout = layout_at(node.path());
    let runner = node111::testkit::MockRunner::new(Vec::<(String, Output)>::new());
    let report = update_guard::run_with(&layout, &runner, 1_760_000_000).unwrap();
    assert!(report.reverted.is_none());
    assert_eq!(current_basename(node.path()), "2026.10.12");
}

#[test]
fn f3_ready_but_503_helper_reverts_on_daemon_request() {
    let node = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(node.path().join("state")).unwrap();
    make_release(node.path(), "2026.10.01");
    make_release(node.path(), "2026.10.12");
    flip_to(node.path(), "2026.10.12");
    Journal {
        schema: 1,
        pending: true,
        version: "2026.10.12".into(),
        previous: "2026.10.01".into(),
        deadline_epoch: 1_760_000_600,
        health_confirmed: false,
    }
    .write(&node.path().join("state/update.json"))
    .unwrap();
    std::fs::write(
        node.path().join("state/revert-request.json"),
        r#"{"schema":1,"reason":"health-red"}"#,
    )
    .unwrap();

    let layout = layout_at(node.path());
    let runner =
        node111::testkit::MockRunner::new(vec![("systemctl restart 111node.service", ok_out())]);
    let report = update_helper::run_with(&layout, &runner, 1_760_000_000).unwrap();
    assert!(report
        .actions
        .iter()
        .any(|a| matches!(a, HelperAction::Reverted { .. })));
    assert_eq!(current_basename(node.path()), "2026.10.01");
    let j = Journal::read(&node.path().join("state/update.json"))
        .unwrap()
        .unwrap();
    assert!(!j.pending);
    runner.assert_exhausted();
}

#[test]
fn torn_current_payload_guard_flips_to_previous() {
    let node = tempfile::tempdir().unwrap();
    make_release(node.path(), "2026.10.01");
    make_release(node.path(), "2026.10.12");
    flip_to(node.path(), "2026.10.12");
    // tear the current tree (no journal — pure corruption case)
    std::fs::write(
        node.path()
            .join("releases/2026.10.12/share/kb/reticulum.md"),
        "TORN\n",
    )
    .unwrap();

    let layout = layout_at(node.path());
    let runner = node111::testkit::MockRunner::new(vec![(
        "systemctl reset-failed 111node.service",
        ok_out(),
    )]);
    let report = update_guard::run_with(&layout, &runner, 1_760_000_000).unwrap();
    assert!(report.torn_payload);
    assert_eq!(report.reverted.as_deref(), Some("2026.10.01"));
    assert_eq!(current_basename(node.path()), "2026.10.01");
}

#[test]
fn idempotent_same_version_request_is_nothing() {
    let node = tempfile::tempdir().unwrap();
    make_release(node.path(), "2026.10.12");
    flip_to(node.path(), "2026.10.12");
    make_staging(node.path(), "2026.10.12");
    let layout = layout_at(node.path());
    let runner = node111::testkit::MockRunner::new(Vec::<(String, Output)>::new());
    let report = update_helper::run_with(&layout, &runner, 1_760_000_000).unwrap();
    assert!(report.actions.contains(&HelperAction::Nothing));
    assert!(!node.path().join("state/update-request.json").exists());
    assert!(Journal::read(&node.path().join("state/update.json"))
        .unwrap()
        .is_none());
}

#[test]
fn production_entry_shape_compiles() {
    // Smoke: the production entry exists with the right type (not run — needs root).
    let _f: fn() -> anyhow::Result<()> = update_helper::run;
    let _g: fn() -> anyhow::Result<()> = update_guard::run;
    let _r = SystemRunner;
}
