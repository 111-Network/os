//! T11 acceptance: root update-helper + update-guard on temp roots with an
//! injected clock + scripted systemctl — promote happy path, bad staged hash,
//! replayed manifest, probation finalize + prune, prune-during-probation
//! refused, F2 crash-before-READY revert, F3 READY-but-503 revert, and the
//! Layer 5 cross-check on the tree the helper extracts itself (§19 L1).
//!
//! Layer 7 L3 (`7-relay/README.md` §9.5 T3; I14, G12): the helper refuses a
//! manifest whose `min-upgrade-from` is above the installed release and flips
//! nothing; a bundle staged the way `111 update --from` stages it (the daemon's
//! own staging functions, an `http://` LAN `payload.url`, a planted staged tree)
//! is checked exactly as an internet one. Every fixture is signed with the
//! minisign CLI and the throwaway test key (`.github/workflows/ci.yml` installs
//! it in CI); every root is a scratch tempdir.

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
    make_staging_with(root, version, &common::build_payload(), "");
}

/// [`make_staging`] with a chosen tarball and extra manifest sections (Layer 5).
fn make_staging_with(root: &Path, version: &str, tarball: &[u8], extra: &str) {
    make_staging_min(root, version, "2026.10.01", tarball, extra);
}

/// The signed manifest text the fixtures use: `min-upgrade-from` and the
/// payload's `url` chosen by the caller.
fn manifest_text(
    version: &str,
    min_upgrade_from: &str,
    payload_url: &str,
    tarball: &[u8],
    extra: &str,
) -> String {
    format!(
        r#"manifest-version = 1
date = "{version}"
channel = "stable"
release = "{version}"
min-upgrade-from = "{min_upgrade_from}"

[pkg.111.target.x86_64-unknown-linux-musl]
url = "https://example.invalid/111"
sha256 = "{a64}"
size = 100

[payload.x86_64-unknown-linux-musl]
url = "{payload_url}"
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
{extra}"#,
        a64 = "a".repeat(64),
        tsha = node111::util::hash::sha256_bytes(tarball),
        tlen = tarball.len()
    )
}

/// [`make_staging_with`] with a chosen `min-upgrade-from` (§9.5 T3).
fn make_staging_min(root: &Path, version: &str, min: &str, tarball: &[u8], extra: &str) {
    let staged = root.join("state/staging").join(version);
    std::fs::create_dir_all(&staged).unwrap();
    let manifest = manifest_text(
        version,
        min,
        "https://example.invalid/payload.tar.gz",
        tarball,
        extra,
    );
    std::fs::write(staged.join("payload.tar.gz"), tarball).unwrap();
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

const MESHMUD: &[u8] = b"meshmud v1.0.0 linux/amd64 static";

fn meshmud_pin(bytes: &[u8]) -> String {
    format!(
        "\n[pkg.meshmud.target.x86_64-unknown-linux-musl]\nurl = \"https://example.invalid/meshmud\"\nsha256 = \"{}\"\nsize = {}\n",
        node111::util::hash::sha256_bytes(bytes),
        bytes.len()
    )
}

/// The helper promotes a payload whose Layer 5 assets equal the signed pins.
#[test]
fn layer5_pinned_payload_promotes() {
    let node = tempfile::tempdir().unwrap();
    make_release(node.path(), "2026.10.01");
    flip_to(node.path(), "2026.10.01");
    let tarball = common::build_payload_with(&[("bin/meshmud", MESHMUD)]);
    make_staging_with(node.path(), "2026.10.12", &tarball, &meshmud_pin(MESHMUD));
    let layout = layout_at(node.path());
    let runner =
        node111::testkit::MockRunner::new(vec![("systemctl restart 111node.service", ok_out())]);
    let report = update_helper::run_with(&layout, &runner, 1_760_000_000).unwrap();
    assert!(report.actions.contains(&HelperAction::Promoted {
        version: "2026.10.12".into()
    }));
    assert_eq!(
        std::fs::read(node.path().join("releases/2026.10.12/bin/meshmud")).unwrap(),
        MESHMUD
    );
    runner.assert_exhausted();
}

/// §19 L1 at the root helper (authoritative): a correctly signed and pinned
/// tarball whose tree disagrees with the manifest's Layer 5 pins is never
/// promoted — no release dir, no extraction debris, no flip, no journal.
#[test]
fn layer5_mismatch_refused_no_promote() {
    let tarball = common::build_payload_with(&[("bin/meshmud", MESHMUD)]);
    for (extra, needle) in [
        (String::new(), "pins no meshmud"),
        (meshmud_pin(b"another meshmud build"), "bin/meshmud size"),
    ] {
        let node = tempfile::tempdir().unwrap();
        make_release(node.path(), "2026.10.01");
        flip_to(node.path(), "2026.10.01");
        make_staging_with(node.path(), "2026.10.12", &tarball, &extra);
        let layout = layout_at(node.path());
        let runner = node111::testkit::MockRunner::new(Vec::<(String, Output)>::new());
        let err = update_helper::run_with(&layout, &runner, 1_760_000_000).unwrap_err();
        assert!(
            err.to_string().contains(needle),
            "wanted {needle:?} in: {err}"
        );
        assert_eq!(current_basename(node.path()), "2026.10.01");
        assert!(!node.path().join("releases/2026.10.12").exists());
        let debris: Vec<String> = std::fs::read_dir(node.path().join("releases"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('.'))
            .collect();
        assert!(debris.is_empty(), "extraction debris left: {debris:?}");
        assert!(Journal::read(&node.path().join("state/update.json"))
            .unwrap()
            .is_none());
    }
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

// --- Layer 7 L3: §9.5 T3 at the root helper, and the `--from` bundle (I14) ---

/// Nothing moved: `current` still names `installed`, no `releases/<version>`, no
/// extraction debris, no journal, no floor written, no restart asked.
fn assert_nothing_flipped(root: &Path, installed: &str, version: &str) {
    if installed.is_empty() {
        assert!(std::fs::symlink_metadata(root.join("current")).is_err());
    } else {
        assert_eq!(current_basename(root), installed);
    }
    assert!(!root.join("releases").join(version).exists());
    if let Ok(rd) = std::fs::read_dir(root.join("releases")) {
        let debris: Vec<String> = rd
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('.'))
            .collect();
        assert!(debris.is_empty(), "extraction debris left: {debris:?}");
    }
    assert!(Journal::read(&root.join("state/update.json"))
        .unwrap()
        .is_none());
    assert!(!root.join("state/floor.json").exists(), "floor written");
}

/// T3: installed 2026.10.01, the signed 2026.10.12 needs at least 2026.10.05 ⇒
/// refused before the tarball is read; nothing flips; the request is left.
#[test]
fn t3_min_upgrade_from_above_installed_refused_flips_nothing() {
    let node = tempfile::tempdir().unwrap();
    make_release(node.path(), "2026.10.01");
    flip_to(node.path(), "2026.10.01");
    make_staging_min(
        node.path(),
        "2026.10.12",
        "2026.10.05",
        &common::build_payload(),
        "",
    );
    let layout = layout_at(node.path());
    let runner = node111::testkit::MockRunner::new(Vec::<(String, Output)>::new());
    let err = update_helper::run_with(&layout, &runner, 1_760_000_000).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("min-upgrade-from 2026.10.05"), "{msg}");
    assert!(msg.contains("2026.10.01"), "{msg}");
    assert_nothing_flipped(node.path(), "2026.10.01", "2026.10.12");
    assert!(node.path().join("state/update-request.json").exists());
    assert!(runner.calls().is_empty(), "{:?}", runner.calls());
}

/// T3 is checked before the tarball: a refused manifest is refused even when its
/// staged tarball is also corrupt (the T3 reason, not the sha256 one).
#[test]
fn t3_refusal_precedes_the_tarball() {
    let node = tempfile::tempdir().unwrap();
    make_release(node.path(), "2026.10.01");
    flip_to(node.path(), "2026.10.01");
    make_staging_min(
        node.path(),
        "2026.10.12",
        "2026.10.12",
        &common::build_payload(),
        "",
    );
    std::fs::remove_file(node.path().join("state/staging/2026.10.12/payload.tar.gz")).unwrap();
    let layout = layout_at(node.path());
    let runner = node111::testkit::MockRunner::new(Vec::<(String, Output)>::new());
    let err = update_helper::run_with(&layout, &runner, 1_760_000_000).unwrap_err();
    assert!(format!("{err:#}").contains("min-upgrade-from"), "{err:#}");
    assert_nothing_flipped(node.path(), "2026.10.01", "2026.10.12");
}

/// T3 boundary: `min-upgrade-from` equal to the installed release promotes
/// (as the installer's gate, `install/stage.rs` `version_gate`).
#[test]
fn t3_min_upgrade_from_equal_to_installed_promotes() {
    let node = tempfile::tempdir().unwrap();
    make_release(node.path(), "2026.10.05");
    flip_to(node.path(), "2026.10.05");
    make_staging_min(
        node.path(),
        "2026.10.12",
        "2026.10.05",
        &common::build_payload(),
        "",
    );
    let layout = layout_at(node.path());
    let runner =
        node111::testkit::MockRunner::new(vec![("systemctl restart 111node.service", ok_out())]);
    let report = update_helper::run_with(&layout, &runner, 1_760_000_000).unwrap();
    assert!(report.actions.contains(&HelperAction::Promoted {
        version: "2026.10.12".into()
    }));
    assert_eq!(current_basename(node.path()), "2026.10.12");
    runner.assert_exhausted();
}

/// T3 fails closed: no readable installed release (no `current`, or a basename
/// that is not a calver) ⇒ refused, nothing flipped.
#[test]
fn t3_unknown_installed_release_refused() {
    // No `current` link at all.
    let node = tempfile::tempdir().unwrap();
    make_staging(node.path(), "2026.10.12");
    let layout = layout_at(node.path());
    let runner = node111::testkit::MockRunner::new(Vec::<(String, Output)>::new());
    let err = update_helper::run_with(&layout, &runner, 1_760_000_000).unwrap_err();
    assert!(
        format!("{err:#}").contains("installed release unknown"),
        "{err:#}"
    );
    assert_nothing_flipped(node.path(), "", "2026.10.12");

    // `current` names a directory that is not a calver.
    let node = tempfile::tempdir().unwrap();
    let odd = node.path().join("releases/not-a-calver");
    std::fs::create_dir_all(&odd).unwrap();
    std::os::unix::fs::symlink(&odd, node.path().join("current")).unwrap();
    make_staging(node.path(), "2026.10.12");
    let layout = layout_at(node.path());
    let err = update_helper::run_with(&layout, &runner, 1_760_000_000).unwrap_err();
    assert!(
        format!("{err:#}").contains("installed release unknown"),
        "{err:#}"
    );
    assert_nothing_flipped(node.path(), "not-a-calver", "2026.10.12");
    assert!(runner.calls().is_empty());
}

/// A bundle staged as `111 update --from <dir>` stages it (§9.2): the daemon's own
/// staging functions, a LAN `http://` `payload.url` (§9.5 T6), the staged `tree/`
/// beside it, the request file from `write_update_request`. Returns the staged
/// directory.
fn make_from_staging(root: &Path, version: &str, min: &str, tarball: &[u8]) -> PathBuf {
    use node111::daemon::updater::{fetch, stage};
    let state = root.join("state");
    let release: node111::calver::Calver = version.parse().unwrap();
    let manifest = manifest_text(
        version,
        min,
        "http://192.168.1.10:8111/origin/v1/blob/payload",
        tarball,
        "",
    );
    let pin = node111::manifest::PinnedArtifact {
        name: "x86_64-unknown-linux-musl".into(),
        url: "http://192.168.1.10:8111/origin/v1/blob/payload".into(),
        sha256: node111::util::hash::sha256_bytes(tarball),
        size: tarball.len() as u64,
    };
    let staged = stage::stage_payload(tarball, &pin, &state.join("staging"), &release).unwrap();
    let manifest_path = staged.join(fetch::MANIFEST_NAME);
    std::fs::write(&manifest_path, &manifest).unwrap();
    common::sign_file(&common::minisign_fixture_key(), &manifest_path);
    assert!(staged.join(fetch::SIGNATURE_NAME).is_file());
    stage::write_update_request(
        &state,
        &release,
        &node111::util::hash::sha256_bytes(manifest.as_bytes()),
        &staged,
    )
    .unwrap();
    staged
}

/// I14: a `--from` bundle that passes every check is promoted and flipped by the
/// helper; the staged `tree/` (daemon-writable, a hint only) is never what gets
/// promoted — the helper extracts the tarball itself.
#[test]
fn from_staged_bundle_promotes_from_its_own_extraction() {
    let node = tempfile::tempdir().unwrap();
    make_release(node.path(), "2026.10.01");
    flip_to(node.path(), "2026.10.01");
    let staged = make_from_staging(
        node.path(),
        "2026.10.12",
        "2026.10.01",
        &common::build_payload(),
    );
    // Plant a doctored file in the staged tree: it must not reach releases/.
    std::fs::write(staged.join("tree/share/kb/reticulum.md"), "PLANTED\n").unwrap();
    let layout = layout_at(node.path());
    let runner =
        node111::testkit::MockRunner::new(vec![("systemctl restart 111node.service", ok_out())]);
    let report = update_helper::run_with(&layout, &runner, 1_760_000_000).unwrap();
    assert!(report.actions.contains(&HelperAction::Promoted {
        version: "2026.10.12".into()
    }));
    assert_eq!(current_basename(node.path()), "2026.10.12");
    let kb = std::fs::read_to_string(
        node.path()
            .join("releases/2026.10.12/share/kb/reticulum.md"),
    )
    .unwrap();
    assert_ne!(kb, "PLANTED\n");
    assert!(!node.path().join("state/update-request.json").exists());
    runner.assert_exhausted();
}

/// I14: a `--from` bundle meets the same refusals as an internet one: T3, a
/// tampered tarball, a manifest signed by another key, a replay below the floor.
#[test]
fn from_staged_bundle_checked_exactly_as_an_internet_one() {
    // T3.
    let node = tempfile::tempdir().unwrap();
    make_release(node.path(), "2026.10.01");
    flip_to(node.path(), "2026.10.01");
    make_from_staging(
        node.path(),
        "2026.10.12",
        "2026.10.05",
        &common::build_payload(),
    );
    let layout = layout_at(node.path());
    let runner = node111::testkit::MockRunner::new(Vec::<(String, Output)>::new());
    let err = update_helper::run_with(&layout, &runner, 1_760_000_000).unwrap_err();
    assert!(format!("{err:#}").contains("min-upgrade-from"), "{err:#}");
    assert_nothing_flipped(node.path(), "2026.10.01", "2026.10.12");

    // A tampered tarball.
    let node = tempfile::tempdir().unwrap();
    make_release(node.path(), "2026.10.01");
    flip_to(node.path(), "2026.10.01");
    let staged = make_from_staging(
        node.path(),
        "2026.10.12",
        "2026.10.01",
        &common::build_payload(),
    );
    let tar = staged.join("payload.tar.gz");
    let mut b = std::fs::read(&tar).unwrap();
    b[10] ^= 0xFF;
    std::fs::write(&tar, &b).unwrap();
    let layout = layout_at(node.path());
    let err = update_helper::run_with(&layout, &runner, 1_760_000_000).unwrap_err();
    assert!(format!("{err:#}").contains("sha256"), "{err:#}");
    assert_nothing_flipped(node.path(), "2026.10.01", "2026.10.12");

    // Signed by a key the keychain walk never reaches.
    let node = tempfile::tempdir().unwrap();
    make_release(node.path(), "2026.10.01");
    flip_to(node.path(), "2026.10.01");
    let staged = make_from_staging(
        node.path(),
        "2026.10.12",
        "2026.10.01",
        &common::build_payload(),
    );
    common::sign_file(
        &common::minisign_other_key(),
        &staged.join("channel-111-stable.toml"),
    );
    let layout = layout_at(node.path());
    assert!(update_helper::run_with(&layout, &runner, 1_760_000_000).is_err());
    assert_nothing_flipped(node.path(), "2026.10.01", "2026.10.12");

    // A replay at/below the floor.
    let node = tempfile::tempdir().unwrap();
    make_release(node.path(), "2026.10.01");
    flip_to(node.path(), "2026.10.01");
    make_from_staging(
        node.path(),
        "2026.10.12",
        "2026.10.01",
        &common::build_payload(),
    );
    let floor = node111::floor::Floor {
        schema: 1,
        highest_release: "2026.10.13".into(),
        highest_manifest_date: "2026.10.13".into(),
        highest_keychain_generation: 0,
    };
    floor.write(&node.path().join("state/floor.json")).unwrap();
    let layout = layout_at(node.path());
    assert!(update_helper::run_with(&layout, &runner, 1_760_000_000).is_err());
    assert_eq!(current_basename(node.path()), "2026.10.01");
    assert!(!node.path().join("releases/2026.10.12").exists());
    assert_eq!(
        node111::floor::Floor::read(&node.path().join("state/floor.json"))
            .unwrap()
            .unwrap(),
        floor
    );
    assert!(runner.calls().is_empty(), "{:?}", runner.calls());
}
