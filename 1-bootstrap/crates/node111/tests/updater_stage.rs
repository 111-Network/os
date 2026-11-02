//! T10 acceptance: updater fetch/verify/stage against a fixture HTTP server —
//! happy path, stale manifest, bad minisig, replayed keychain generation,
//! tampered payload, clock-skew marker, and the Layer 5 cross-check
//! (`verify_layer5_tree` after staging, os/5-messaging/README.md §19 L1). Asserts
//! the daemon never writes outside state/staging (no releases/, no `current`).
//!
//! Then the presence venv across promotions (os/5-messaging/README.md §14;
//! `daemon::transport::venv::presence::ensure_current`, what the daemon runs at
//! the start that follows a promotion), with a fake python3/pip
//! (`fake_python`): an unchanged lock runs nothing, a changed lock rebuilds
//! offline, and a failed rebuild (hash mismatch, no python3) keeps the old venv
//! and records why.
//!
//! Then Layer 7 §9.2–§9.5 (row L2): the staging lock (`state/staging.lock`: a
//! held lock answers `Held` and stages nothing, a concurrent run stages once),
//! the blob rule (a hub, directory or typed-URL origin fetches
//! `blob/<sha256 of the pin>`, never `payload.url`, and writes no
//! `last-update-check.json`), a 404 trying the next origin, T3 (`min-upgrade-from`
//! above the running release is refused before any payload fetch), T6 (plain
//! `http://` to a LAN host passes only under a typed exception) and
//! `check_internet = false` (no internet base contacted, by the check or the
//! loop).

mod common;
mod fake_python;

use node111::calver::Calver;
use node111::daemon::transport::venv::presence::{self, Outcome, PresenceVenvError};
use node111::daemon::updater::fetch::{
    check_once, check_origins, origin_records, Answer, FetchOutcome, Origin, UpdaterConfig,
};
use node111::daemon::updater::stage::{LockTake, LockWait, StagingLock};
use node111::daemon::DaemonLayout;
use node111::floor::Floor;
use node111::manifest::layer5;
use std::path::{Path, PathBuf};
use std::str::FromStr;

fn sha(b: &[u8]) -> String {
    node111::util::hash::sha256_bytes(b)
}

/// Write a channel manifest (fixture-signed) + payload into a served dir;
/// `extra` is appended to the manifest (Layer 5 sections).
fn make_served(dir: &Path, release: &str, payload_bytes: &[u8], extra: &str) {
    let payload_sha = sha(payload_bytes);
    let payload_len = payload_bytes.len();
    let manifest = format!(
        r#"manifest-version = 1
date = "{release}"
channel = "stable"
release = "{release}"
min-upgrade-from = "2026.10.01"

[pkg.111.target.x86_64-unknown-linux-musl]
url = "https://example.invalid/111"
sha256 = "{a64}"
size = 100

[payload.x86_64-unknown-linux-musl]
url = "PAYLOAD_URL"
sha256 = "{payload_sha}"
size = {payload_len}

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
        a64 = "a".repeat(64)
    );
    std::fs::write(dir.join("payload.tar.gz"), payload_bytes).unwrap();
    std::fs::write(dir.join("channel-111-stable.toml"), &manifest).unwrap();
    // URL patched by the caller once the port is known (see serve()).
}

fn layout_at(root: &Path) -> DaemonLayout {
    std::fs::create_dir_all(root.join("state")).unwrap();
    DaemonLayout {
        root: root.to_path_buf(),
        etc: root.join("etc"),
    }
}

struct Fixture {
    served: tempfile::TempDir,
    node: tempfile::TempDir,
    port: u16,
}

fn setup(release: &str, sign_with: &Path, date_header: Option<String>) -> Fixture {
    setup_with(
        release,
        sign_with,
        date_header,
        &common::build_payload(),
        "",
    )
}

/// [`setup`] with a chosen payload and extra manifest sections.
fn setup_with(
    release: &str,
    sign_with: &Path,
    date_header: Option<String>,
    payload: &[u8],
    extra: &str,
) -> Fixture {
    let served = tempfile::tempdir().unwrap();
    let node = tempfile::tempdir().unwrap();
    make_served(served.path(), release, payload, extra);
    let port = common::serve_dir(served.path().to_path_buf(), date_header);
    // Patch the payload URL now that the port is known, then sign.
    let manifest_path = served.path().join("channel-111-stable.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap().replace(
        "PAYLOAD_URL",
        &format!("http://127.0.0.1:{port}/payload.tar.gz"),
    );
    std::fs::write(&manifest_path, manifest).unwrap();
    common::sign_file(sign_with, &manifest_path);
    Fixture { served, node, port }
}

fn cfg(port: u16, current: &str) -> UpdaterConfig {
    UpdaterConfig {
        manifest_base: format!("http://127.0.0.1:{port}"),
        fallback_base: None,
        current_version: Calver::from_str(current).unwrap(),
        timeout: std::time::Duration::from_secs(5),
        check_internet: true,
    }
}

#[test]
fn happy_path_stages_and_requests() {
    let fx = setup("2026.10.12", &common::minisign_fixture_key(), None);
    let layout = layout_at(fx.node.path());
    let outcome = check_once(&cfg(fx.port, "2026.10.01"), &layout).unwrap();
    assert!(
        matches!(outcome, FetchOutcome::Staged { .. }),
        "{outcome:?}"
    );

    // update-request.json written with the right fields
    let req: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(layout.state_dir().join("update-request.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(req["schema"], 1);
    assert_eq!(req["version"], "2026.10.12");
    assert!(req["manifest_sha256"].as_str().unwrap().len() == 64);
    assert!(req["staged_path"]
        .as_str()
        .unwrap()
        .ends_with("staging/2026.10.12"));

    // staging bundle is complete (D12): tarball + tree + manifest + minisig
    let staged = layout.state_dir().join("staging/2026.10.12");
    assert!(staged.join("payload.tar.gz").exists());
    assert!(staged.join("channel-111-stable.toml").exists());
    assert!(staged.join("channel-111-stable.toml.minisig").exists());
    assert!(staged.join("tree/bin/111").exists());
    assert!(staged.join("tree/share/kb/llms.txt").exists());
    assert!(staged.join("tree/share/payload.manifest").exists());

    // last-successful-check recorded
    assert!(layout.state_dir().join("last-update-check.json").exists());
    // no clock-fault marker on a clean cycle
    assert!(!layout.state_dir().join("clock-fault.json").exists());

    // D12: the daemon never creates releases/ or `current`.
    assert!(!layout.root.join("releases").exists());
    assert!(!layout.root.join("current").exists());
}

#[test]
fn stale_manifest_is_uptodate() {
    let fx = setup("2026.10.12", &common::minisign_fixture_key(), None);
    let layout = layout_at(fx.node.path());
    let outcome = check_once(&cfg(fx.port, "2026.10.12"), &layout).unwrap();
    assert!(matches!(outcome, FetchOutcome::UpToDate), "{outcome:?}");
    assert!(!layout.state_dir().join("update-request.json").exists());
    // success still recorded
    assert!(layout.state_dir().join("last-update-check.json").exists());
}

#[test]
fn bad_minisig_refused() {
    let fx = setup("2026.10.12", &common::minisign_other_key(), None);
    let layout = layout_at(fx.node.path());
    let outcome = check_once(&cfg(fx.port, "2026.10.01"), &layout).unwrap();
    match outcome {
        FetchOutcome::Refused { reason } => assert!(reason.contains("verification"), "{reason}"),
        other => panic!("expected Refused, got {other:?}"),
    }
    assert!(!layout.state_dir().join("update-request.json").exists());
}

#[test]
fn replayed_keychain_generation_refused() {
    let fx = setup("2026.10.12", &common::minisign_fixture_key(), None);
    let layout = layout_at(fx.node.path());
    // Root-written floor already saw generation 2; fixture manifest's max is 1.
    let floor = Floor {
        schema: 1,
        highest_release: "2026.10.01".into(),
        highest_manifest_date: "2026.10.01".into(),
        highest_keychain_generation: 2,
    };
    floor.write(&layout.state_dir().join("floor.json")).unwrap();
    let outcome = check_once(&cfg(fx.port, "2026.10.01"), &layout).unwrap();
    assert!(
        matches!(outcome, FetchOutcome::Refused { .. }),
        "{outcome:?}"
    );
    assert!(!layout.state_dir().join("update-request.json").exists());
}

#[test]
fn tampered_payload_never_stages() {
    let fx = setup("2026.10.12", &common::minisign_fixture_key(), None);
    // Corrupt the served tarball (manifest pin no longer matches).
    let payload_path = fx.served.path().join("payload.tar.gz");
    let mut bytes = std::fs::read(&payload_path).unwrap();
    let n = bytes.len();
    bytes[n - 1] ^= 0xFF;
    std::fs::write(&payload_path, &bytes).unwrap();

    let layout = layout_at(fx.node.path());
    let result = check_once(&cfg(fx.port, "2026.10.01"), &layout);
    assert!(result.is_err(), "pin mismatch must hard-error");
    assert!(!layout.state_dir().join("update-request.json").exists());
    assert!(!layout.state_dir().join("staging/2026.10.12").exists());
}

#[test]
fn clock_skew_writes_fault_marker() {
    let fx = setup(
        "2026.10.12",
        &common::minisign_fixture_key(),
        Some("Wed, 01 Jan 2020 00:00:00 GMT".to_string()),
    );
    let layout = layout_at(fx.node.path());
    let _ = check_once(&cfg(fx.port, "2026.10.01"), &layout).unwrap();
    assert!(
        layout.state_dir().join("clock-fault.json").exists(),
        "skewed Date header must surface the clock-fault class"
    );
}

#[test]
fn symlink_payload_entry_refused() {
    // Build a payload with a symlink entry — the safe unpacker must refuse it.
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    {
        let mut builder = tar::Builder::new(&mut gz);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_cksum();
        builder
            .append_link(&mut header, "bin/evil", "/etc/passwd")
            .unwrap();
        builder.finish().unwrap();
    }
    let tarball = gz.finish().unwrap();
    let artifact = node111::manifest::PinnedArtifact {
        name: "x86_64-unknown-linux-musl".into(),
        url: "http://x".into(),
        sha256: sha(&tarball),
        size: tarball.len() as u64,
    };
    let staging = tempfile::tempdir().unwrap();
    let err = node111::daemon::updater::stage::stage_payload(
        &tarball,
        &artifact,
        staging.path(),
        &Calver::from_str("2026.10.12").unwrap(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("non-regular"), "{err}");
    // traversal entry: the tar builder itself refuses "..", so build a benign
    // archive and patch the name bytes to "../escape" before gzipping.
    let mut tar_bytes: Vec<u8> = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder
            .append_data(&mut header, "xx/escape", b"data".as_slice())
            .unwrap();
        builder.finish().unwrap();
    }
    let pos = tar_bytes
        .windows(9)
        .position(|w| w == b"xx/escape")
        .expect("name present in archive");
    tar_bytes[pos] = b'.';
    tar_bytes[pos + 1] = b'.';
    // fix the header checksum after the edit (bytes 148..156: space, 6 digits, NUL, space)
    let header_start = pos - (pos % 512);
    for i in 148..156 {
        tar_bytes[header_start + i] = b' ';
    }
    let chksum: u32 = tar_bytes[header_start..header_start + 512]
        .iter()
        .map(|&b| b as u32)
        .sum();
    let chk = format!("{:06o}\0 ", chksum);
    tar_bytes[header_start + 148..header_start + 156].copy_from_slice(chk.as_bytes());
    let tarball = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut gz = tarball;
    {
        use std::io::Write;
        gz.write_all(&tar_bytes).unwrap();
    }
    let tarball = gz.finish().unwrap();
    let artifact = node111::manifest::PinnedArtifact {
        sha256: sha(&tarball),
        size: tarball.len() as u64,
        ..artifact
    };
    let err = node111::daemon::updater::stage::stage_payload(
        &tarball,
        &artifact,
        staging.path(),
        &Calver::from_str("2026.10.12").unwrap(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("unsafe path"), "{err}");
}

const MESHMUD: &[u8] = b"meshmud v1.0.0 linux/amd64 static";

/// `[pkg.meshmud.target.x86_64-unknown-linux-musl]` pinning `bytes`.
fn meshmud_pin(bytes: &[u8]) -> String {
    format!(
        "\n[pkg.meshmud.target.x86_64-unknown-linux-musl]\nurl = \"https://example.invalid/meshmud\"\nsha256 = \"{}\"\nsize = {}\n",
        sha(bytes),
        bytes.len()
    )
}

/// A payload whose Layer 5 asset equals the manifest's pin stages as before.
#[test]
fn layer5_pinned_meshmud_stages() {
    let payload = common::build_payload_with(&[("bin/meshmud", MESHMUD)]);
    let fx = setup_with(
        "2026.10.12",
        &common::minisign_fixture_key(),
        None,
        &payload,
        &meshmud_pin(MESHMUD),
    );
    let layout = layout_at(fx.node.path());
    let outcome = check_once(&cfg(fx.port, "2026.10.01"), &layout).unwrap();
    assert!(
        matches!(outcome, FetchOutcome::Staged { .. }),
        "{outcome:?}"
    );
    let staged = layout.state_dir().join("staging/2026.10.12");
    assert_eq!(
        std::fs::read(staged.join("tree/bin/meshmud")).unwrap(),
        MESHMUD
    );
    assert!(layout.state_dir().join("update-request.json").exists());
}

/// §19 L1 at the updater: a signed manifest whose payload carries a meshmud it
/// does not pin — or other bytes than it pins — is refused after staging; the
/// staging bundle is removed and the root helper is never asked.
#[test]
fn layer5_mismatch_is_refused_and_unstaged() {
    let payload = common::build_payload_with(&[("bin/meshmud", MESHMUD)]);
    for (extra, needle) in [
        (String::new(), "pins no meshmud"),
        (meshmud_pin(b"another meshmud build"), "bin/meshmud size"),
    ] {
        let fx = setup_with(
            "2026.10.12",
            &common::minisign_fixture_key(),
            None,
            &payload,
            &extra,
        );
        let layout = layout_at(fx.node.path());
        let outcome = check_once(&cfg(fx.port, "2026.10.01"), &layout).unwrap();
        match outcome {
            FetchOutcome::Refused { reason } => {
                assert!(reason.contains("Layer 5"), "{reason}");
                assert!(reason.contains(needle), "wanted {needle:?} in: {reason}");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
        assert!(!layout.state_dir().join("update-request.json").exists());
        assert!(!layout.state_dir().join("staging/2026.10.12").exists());
        // D12 still holds on the refusal path.
        assert!(!layout.root.join("releases").exists());
        assert!(!layout.root.join("current").exists());
    }
}

// --- the presence venv across promotions (Layer 5 §14) ---------------------

const LOCK_1: &[u8] = b"meshtastic==2.7.11 --hash=sha256:1111\n";
const LOCK_2: &[u8] = b"meshtastic==2.7.12 --hash=sha256:2222\n";
const LOCK_3: &[u8] = b"meshtastic==2.7.13 --hash=sha256:3333\n";
const LOCK_4: &[u8] = b"meshtastic==2.7.14 --hash=sha256:4444\n";

/// `[presence-lock]` pinning `bytes`.
fn presence_pin(bytes: &[u8]) -> String {
    format!(
        "\n[presence-lock]\nurl = \"https://example.invalid/meshtastic-presence-lock.txt\"\nsha256 = \"{}\"\nsize = {}\n",
        sha(bytes),
        bytes.len()
    )
}

/// `releases/<version>/` with `bin/111` and, when given, the presence lock.
fn release_tree(layout: &DaemonLayout, version: &str, lock: Option<&[u8]>) {
    let dir = layout.root.join("releases").join(version);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(dir.join("bin/111"), b"node binary").unwrap();
    if let Some(lock) = lock {
        std::fs::create_dir_all(dir.join(layer5::PRESENCE_DIR)).unwrap();
        std::fs::write(dir.join(layer5::PRESENCE_LOCK), lock).unwrap();
    }
}

/// The root helper's flip; the daemon restart that follows runs
/// `ensure_current`.
fn promote(layout: &DaemonLayout, version: &str) {
    node111::update_helper::flip_current(layout, version).unwrap();
}

fn venv_target(layout: &DaemonLayout) -> PathBuf {
    std::fs::read_link(presence::venv_dir(&layout.root)).unwrap()
}

fn installed_lock(layout: &DaemonLayout) -> Vec<u8> {
    std::fs::read(presence::venv_dir(&layout.root).join("installed-lock.txt")).unwrap()
}

fn marker(layout: &DaemonLayout) -> String {
    std::fs::read_to_string(presence::venv_dir(&layout.root).join(".111-integrity")).unwrap()
}

fn builds(layout: &DaemonLayout) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(presence::builds_dir(&layout.root))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// An update whose lock changed is fetched, pin-checked and staged by the real
/// updater, promoted, and the next daemon start rebuilds the venv from the
/// offline cache; promotions with an unchanged lock run nothing; failed
/// rebuilds keep the venv in place and say why; reverting to a release whose
/// lock the venv matches clears the reason.
#[test]
fn presence_venv_follows_the_promoted_lock() {
    let py = fake_python::FakePython::new();
    // the release the node runs: lock 1, venv built at its first start
    let fx = setup_with(
        "2026.10.12",
        &common::minisign_fixture_key(),
        None,
        &common::build_payload_with(&[(layer5::PRESENCE_LOCK, LOCK_2)]),
        &presence_pin(LOCK_2),
    );
    let layout = layout_at(fx.node.path());
    let root = layout.root.clone();
    let cache = presence::cache_dir(&root);
    std::fs::create_dir_all(&cache).unwrap();
    release_tree(&layout, "2026.10.05", Some(LOCK_1));
    promote(&layout, "2026.10.05");
    assert_eq!(presence::ensure_current(&layout, &py), Outcome::Built);
    let lock_path = root.join("current").join(layer5::PRESENCE_LOCK);
    let first = venv_target(&layout);
    assert!(first.starts_with("presence.d"), "{first:?}");
    assert_eq!(installed_lock(&layout), LOCK_1);
    assert_eq!(marker(&layout), sha(LOCK_1));
    let calls = py.calls();
    assert_eq!(calls[0], "python3 --version");
    assert!(calls[1].starts_with(&format!(
        "python3 -m venv {}/",
        presence::builds_dir(&root).display()
    )));
    assert!(
        calls[2].ends_with(&format!(
            "/bin/pip install --require-hashes -r {} --no-index --find-links {}",
            lock_path.display(),
            cache.display()
        )),
        "the daemon builds strictly offline: {calls:?}"
    );
    assert!(!calls.iter().any(|c| c.contains(" download ")), "{calls:?}");
    assert_eq!(calls.len(), 3);
    let status = presence::status(&root).unwrap();
    assert!(status.ok && status.reason.is_none(), "{status:?}");
    assert_eq!(status.lock_sha256, sha(LOCK_1));
    // the reader's interpreter is this venv's
    let reader =
        node111::daemon::meshtastic::MeshtasticConfig::production(&layout, false).reader_python;
    assert_eq!(reader, presence::python(&root));
    assert!(reader.is_file());

    // the same lock in the next release: nothing runs, nothing moves
    release_tree(&layout, "2026.10.08", Some(LOCK_1));
    promote(&layout, "2026.10.08");
    let status_before = std::fs::read(presence::status_path(&root)).unwrap();
    assert_eq!(presence::ensure_current(&layout, &py), Outcome::Unchanged);
    assert_eq!(py.calls().len(), 3, "an unchanged lock runs nothing");
    assert_eq!(venv_target(&layout), first);
    assert_eq!(
        std::fs::read(presence::status_path(&root)).unwrap(),
        status_before
    );

    // an update with lock 2: the updater stages it (pin-checked), the helper
    // promotes it, the daemon start rebuilds
    let outcome = check_once(&cfg(fx.port, "2026.10.08"), &layout).unwrap();
    assert!(
        matches!(outcome, FetchOutcome::Staged { .. }),
        "{outcome:?}"
    );
    let staged = layout.state_dir().join("staging/2026.10.12/tree");
    assert_eq!(
        std::fs::read(staged.join(layer5::PRESENCE_LOCK)).unwrap(),
        LOCK_2
    );
    std::fs::rename(&staged, root.join("releases/2026.10.12")).unwrap();
    promote(&layout, "2026.10.12");
    assert_eq!(presence::ensure_current(&layout, &py), Outcome::Built);
    let second = venv_target(&layout);
    assert_ne!(second, first);
    assert_eq!(installed_lock(&layout), LOCK_2);
    assert_eq!(marker(&layout), sha(LOCK_2));
    assert_eq!(
        builds(&layout),
        vec![second.file_name().unwrap().to_string_lossy().into_owned()],
        "the older build is pruned once the new one is in place"
    );
    assert!(!py.calls().iter().any(|c| c.contains(" download ")));

    // lock 3 and pip refuses (hash mismatch): the lock-2 venv stays in place
    py.fail_install(true);
    release_tree(&layout, "2026.10.15", Some(LOCK_3));
    promote(&layout, "2026.10.15");
    match presence::ensure_current(&layout, &py) {
        Outcome::Unavailable {
            error: PresenceVenvError::Pip { offline: true, .. },
            previous_kept: true,
        } => {}
        other => panic!("expected a kept venv after a pip failure, got {other:?}"),
    }
    assert_eq!(venv_target(&layout), second);
    assert_eq!(installed_lock(&layout), LOCK_2);
    assert_eq!(marker(&layout), sha(LOCK_2));
    assert_eq!(builds(&layout).len(), 1, "the failed build is removed");
    let status = presence::status(&root).unwrap();
    assert!(!status.ok);
    assert_eq!(status.code, "presence_venv_failed");
    assert_eq!(status.lock_sha256, sha(LOCK_3));
    assert!(status.previous_kept);
    let reason = presence::unavailable_reason(&root).unwrap();
    assert!(reason.contains("DO NOT MATCH THE HASHES"), "{reason}");
    assert!(reason.contains("runs on the one built earlier"), "{reason}");
    assert!(reason.contains("111 install"), "{reason}");

    // lock 4 on a machine that lost python3: still the lock-2 venv
    py.fail_install(false);
    py.set_missing(true);
    release_tree(&layout, "2026.10.18", Some(LOCK_4));
    promote(&layout, "2026.10.18");
    let calls_before = py.calls().len();
    assert_eq!(
        presence::ensure_current(&layout, &py),
        Outcome::Unavailable {
            error: PresenceVenvError::NoPython3,
            previous_kept: true
        }
    );
    assert_eq!(py.calls().len(), calls_before, "python3 never ran");
    assert_eq!(venv_target(&layout), second);
    assert_eq!(presence::status(&root).unwrap().code, "presence_no_python");
    assert!(presence::unavailable_reason(&root)
        .unwrap()
        .contains("python3 is not installed"));

    // a revert to the lock-2 release: the venv matches again, the reason clears
    py.set_missing(false);
    promote(&layout, "2026.10.12");
    assert_eq!(presence::ensure_current(&layout, &py), Outcome::Unchanged);
    assert_eq!(py.calls().len(), calls_before);
    assert!(presence::status(&root).unwrap().ok);
    assert_eq!(presence::unavailable_reason(&root), None);
}

/// A release without the reader leaves everything as it is; links where the
/// build would write are refused, never followed; a first build that fails
/// leaves no venv and says the reader is off.
#[test]
fn presence_venv_refusals() {
    let py = fake_python::FakePython::new();
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let root = layout.root.clone();

    release_tree(&layout, "2026.10.05", None);
    promote(&layout, "2026.10.05");
    assert_eq!(presence::ensure_current(&layout, &py), Outcome::NotShipped);
    assert!(py.calls().is_empty());
    assert!(!root.join("venvs").exists());

    // venvs/presence.d planted as a link to elsewhere
    release_tree(&layout, "2026.10.08", Some(LOCK_1));
    promote(&layout, "2026.10.08");
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.join("venvs")).unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), presence::builds_dir(&root)).unwrap();
    match presence::ensure_current(&layout, &py) {
        Outcome::Unavailable {
            error: PresenceVenvError::Unsafe(why),
            previous_kept: false,
        } => assert!(why.contains("is a link"), "{why}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(
        std::fs::read_dir(elsewhere.path()).unwrap().count(),
        0,
        "nothing written through the link"
    );
    assert!(!presence::venv_dir(&root).exists());

    // a first build whose venv step fails: no venv, the reader is off
    std::fs::remove_file(presence::builds_dir(&root)).unwrap();
    py.fail_venv(true);
    match presence::ensure_current(&layout, &py) {
        Outcome::Unavailable {
            error: PresenceVenvError::Venv(why),
            previous_kept: false,
        } => assert!(why.contains("ensurepip"), "{why}"),
        other => panic!("expected a venv failure, got {other:?}"),
    }
    assert!(builds(&layout).is_empty(), "the failed build is removed");
    assert!(std::fs::symlink_metadata(presence::venv_dir(&root)).is_err());
    let reason = presence::unavailable_reason(&root).unwrap();
    assert!(reason.starts_with("The presence reader is off"), "{reason}");

    // and the next start, with the venv step fixed, builds it
    py.fail_venv(false);
    std::fs::create_dir_all(presence::cache_dir(&root)).unwrap();
    assert_eq!(presence::ensure_current(&layout, &py), Outcome::Built);
    assert_eq!(installed_lock(&layout), LOCK_1);
    assert_eq!(presence::unavailable_reason(&root), None);
}

/// A hand-made `venvs/presence` directory (no marker) is replaced by a
/// marked build; the old directory is moved aside and pruned.
#[test]
fn presence_venv_replaces_a_hand_made_dir() {
    let py = fake_python::FakePython::new();
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let root = layout.root.clone();
    release_tree(&layout, "2026.10.05", Some(LOCK_1));
    promote(&layout, "2026.10.05");
    let manual = presence::venv_dir(&root);
    std::fs::create_dir_all(manual.join("bin")).unwrap();
    std::fs::write(manual.join("bin/python"), b"#!/bin/sh\n").unwrap();
    assert!(!presence::integrity_ok(
        &root,
        &root.join("current").join(layer5::PRESENCE_LOCK)
    ));
    assert_eq!(presence::ensure_current(&layout, &py), Outcome::Built);
    assert!(std::fs::symlink_metadata(&manual)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(installed_lock(&layout), LOCK_1);
    assert_eq!(builds(&layout).len(), 1, "the hand-made dir is pruned");
    assert!(!builds(&layout)[0].starts_with("legacy-"));
}

// --- Layer 7 §9.2–§9.5: the staging lock, the origin list, the blob rule, T3, T6 ---

use std::sync::{Arc, Mutex};

/// A loopback file server that logs every request line's path (`/x`), and
/// answers 404 for anything it does not have. Runs until the process exits.
#[derive(Clone)]
struct LogServer {
    port: u16,
    log: Arc<Mutex<Vec<String>>>,
}

impl LogServer {
    fn serve(dir: Option<PathBuf>) -> LogServer {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&log);
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { break };
                let dir = dir.clone();
                let seen = Arc::clone(&seen);
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let Ok(n) = conn.read(&mut buf) else { return };
                    let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let mut words = req.split_whitespace();
                    let method = words.next().unwrap_or("").to_string();
                    let path = words.next().unwrap_or("/").to_string();
                    seen.lock().unwrap().push(format!("{method} {path}"));
                    let file = dir
                        .as_ref()
                        .map(|d| d.join(path.trim_start_matches('/')))
                        .and_then(|f| std::fs::read(f).ok());
                    let (status, body) = match file {
                        Some(b) => ("200 OK", b),
                        None => ("404 Not Found", b"not found".to_vec()),
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = conn.write_all(head.as_bytes());
                    if method != "HEAD" {
                        let _ = conn.write_all(&body);
                    }
                });
            }
        });
        LogServer { port, log }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn requests(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn count(&self, needle: &str) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.contains(needle))
            .count()
    }

    fn exact(&self, line: &str) -> usize {
        self.requests().iter().filter(|r| *r == line).count()
    }
}

/// An origin directory: the signed manifest pinning `payload` (its
/// `payload.url` set to `payload_url`), `blob/<sha256>` holding the payload
/// unless `blob` is false, and `patch` applied to the manifest text before it is
/// signed.
fn origin_dir(
    release: &str,
    payload: &[u8],
    payload_url: &str,
    blob: bool,
    patch: &dyn Fn(String) -> String,
) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    make_served(dir.path(), release, payload, "");
    std::fs::remove_file(dir.path().join("payload.tar.gz")).unwrap();
    let manifest_path = dir.path().join("channel-111-stable.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .unwrap()
        .replace("PAYLOAD_URL", payload_url);
    std::fs::write(&manifest_path, patch(manifest)).unwrap();
    common::sign_file(&common::minisign_fixture_key(), &manifest_path);
    if blob {
        std::fs::create_dir_all(dir.path().join("blob")).unwrap();
        std::fs::write(dir.path().join("blob").join(sha(payload)), payload).unwrap();
    }
    dir
}

fn no_patch(s: String) -> String {
    s
}

/// An internet-only config whose bases are `base` (and no fallback).
fn net_cfg(base: &str, current: &str) -> UpdaterConfig {
    UpdaterConfig {
        manifest_base: base.to_string(),
        fallback_base: None,
        current_version: Calver::from_str(current).unwrap(),
        timeout: std::time::Duration::from_secs(5),
        check_internet: true,
    }
}

fn hub(server: &LogServer) -> Origin {
    Origin::HomeHub {
        base: format!("{}/origin/v1/", server.base()),
    }
}

fn assert_bundle(layout: &DaemonLayout, version: &str) {
    let staged = layout.state_dir().join("staging").join(version);
    for f in [
        "payload.tar.gz",
        "channel-111-stable.toml",
        "channel-111-stable.toml.minisig",
        "tree/bin/111",
    ] {
        assert!(staged.join(f).exists(), "{f} staged");
    }
    let req: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(layout.state_dir().join("update-request.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(req["version"], version);
    assert!(!layout.root.join("releases").exists());
    assert!(!layout.root.join("current").exists());
}

/// S5: the home hub's payload comes from `blob/<sha256 of the pin>`;
/// `payload.url` is never fetched, `last-update-check.json` never written, and
/// the per-origin record holds the answer and the newest release seen.
#[test]
fn hub_origin_fetches_the_blob_never_the_payload_url() {
    let decoy = LogServer::serve(None);
    let payload = common::build_payload();
    let dir = origin_dir(
        "2026.10.12",
        &payload,
        &format!("{}/payload.tar.gz", decoy.base()),
        true,
        &no_patch,
    );
    // the hub serves the origin under /origin/v1/ (§9.1)
    let served = tempfile::tempdir().unwrap();
    std::fs::create_dir(served.path().join("origin")).unwrap();
    std::os::unix::fs::symlink(dir.path(), served.path().join("origin/v1")).unwrap();
    let hub_server = LogServer::serve(Some(served.path().to_path_buf()));

    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let cfg = net_cfg("http://127.0.0.1:9", "2026.10.01");
    let outcome = check_origins(&cfg, &layout, &[hub(&hub_server)], LockWait::Wait).unwrap();
    assert!(
        matches!(outcome, FetchOutcome::Staged { .. }),
        "{outcome:?}"
    );
    assert_bundle(&layout, "2026.10.12");
    assert!(decoy.requests().is_empty(), "payload.url is ignored");
    assert_eq!(
        hub_server.count(&format!("GET /origin/v1/blob/{}", sha(&payload))),
        1
    );
    assert_eq!(hub_server.count("HEAD"), 0, "no Date probe of a hub");
    assert!(
        !layout.state_dir().join("last-update-check.json").exists(),
        "record_success is an internet base's"
    );
    let recs = origin_records(&layout.state_dir());
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].kind, "home_hub");
    assert_eq!(recs[0].answer, Answer::Staged);
    assert_eq!(recs[0].newest_seen.as_deref(), Some("2026.10.12"));

    // a hub answering "up to date" writes no record_success either
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let cfg = net_cfg("http://127.0.0.1:9", "2026.10.12");
    let outcome = check_origins(&cfg, &layout, &[hub(&hub_server)], LockWait::Wait).unwrap();
    assert!(matches!(outcome, FetchOutcome::UpToDate), "{outcome:?}");
    assert!(!layout.state_dir().join("last-update-check.json").exists());
    assert_eq!(
        origin_records(&layout.state_dir())[0].answer,
        Answer::UpToDate
    );
}

/// `--from <dir>` and `--from <url>` follow the same blob rule; the directory
/// origin makes no request at all.
#[test]
fn from_dir_and_url_fetch_the_blob() {
    let decoy = LogServer::serve(None);
    let payload = common::build_payload();
    let dir = origin_dir(
        "2026.10.12",
        &payload,
        &format!("{}/payload.tar.gz", decoy.base()),
        true,
        &no_patch,
    );
    let cfg = net_cfg("http://127.0.0.1:9", "2026.10.01");

    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let from_dir = Origin::Dir {
        path: dir.path().to_path_buf(),
    };
    let outcome = check_origins(&cfg, &layout, &[from_dir], LockWait::NoWait).unwrap();
    assert!(
        matches!(outcome, FetchOutcome::Staged { .. }),
        "{outcome:?}"
    );
    assert_bundle(&layout, "2026.10.12");
    assert!(!layout.state_dir().join("last-update-check.json").exists());
    assert_eq!(origin_records(&layout.state_dir())[0].kind, "dir");

    let server = LogServer::serve(Some(dir.path().to_path_buf()));
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let from_url = Origin::Url {
        base: format!("{}/", server.base()),
    };
    let outcome = check_origins(&cfg, &layout, &[from_url], LockWait::NoWait).unwrap();
    assert!(
        matches!(outcome, FetchOutcome::Staged { .. }),
        "{outcome:?}"
    );
    assert_bundle(&layout, "2026.10.12");
    assert_eq!(server.count(&format!("GET /blob/{}", sha(&payload))), 1);
    assert!(decoy.requests().is_empty(), "payload.url is ignored");
    assert!(!layout.state_dir().join("last-update-check.json").exists());

    // a tampered blob is refused and the staging removed
    let blob = dir.path().join("blob").join(sha(&payload));
    let mut bytes = std::fs::read(&blob).unwrap();
    let n = bytes.len();
    bytes[n - 1] ^= 0xFF;
    std::fs::write(&blob, &bytes).unwrap();
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let from_dir = Origin::Dir {
        path: dir.path().to_path_buf(),
    };
    assert!(check_origins(&cfg, &layout, &[from_dir], LockWait::NoWait).is_err());
    assert!(!layout.state_dir().join("update-request.json").exists());
    assert!(!layout.state_dir().join("staging/2026.10.12").exists());
}

/// A missing blob 404s and the next origin is tried; a missing manifest too.
#[test]
fn a_404_tries_the_next_origin() {
    let payload = common::build_payload();
    let without_blob = origin_dir(
        "2026.10.12",
        &payload,
        "https://example.invalid/p",
        false,
        &no_patch,
    );
    let with_blob = origin_dir(
        "2026.10.12",
        &payload,
        "https://example.invalid/p",
        true,
        &no_patch,
    );
    let first = LogServer::serve(Some(without_blob.path().to_path_buf()));
    let empty = LogServer::serve(None);
    let origins = [
        Origin::Url { base: empty.base() },
        Origin::Url { base: first.base() },
        Origin::Dir {
            path: with_blob.path().to_path_buf(),
        },
    ];
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let cfg = net_cfg("http://127.0.0.1:9", "2026.10.01");
    let outcome = check_origins(&cfg, &layout, &origins, LockWait::Wait).unwrap();
    assert!(
        matches!(outcome, FetchOutcome::Staged { .. }),
        "{outcome:?}"
    );
    assert_bundle(&layout, "2026.10.12");
    assert_eq!(empty.count("GET /channel-111-stable.toml"), 1);
    assert_eq!(first.count(&format!("GET /blob/{}", sha(&payload))), 1);
    let recs = origin_records(&layout.state_dir());
    let answers: Vec<_> = recs.iter().map(|r| (r.kind.as_str(), r.answer)).collect();
    assert_eq!(
        answers,
        vec![
            ("url", Answer::NotFound),
            ("url", Answer::NotFound),
            ("dir", Answer::Staged)
        ]
    );
    assert_eq!(recs[0].newest_seen, None, "no manifest seen there");
    assert_eq!(recs[1].newest_seen.as_deref(), Some("2026.10.12"));

    // every origin missing: degraded, nothing staged
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let outcome = check_origins(&cfg, &layout, &origins[..2], LockWait::Wait).unwrap();
    assert!(
        matches!(outcome, FetchOutcome::Degraded { .. }),
        "{outcome:?}"
    );
    assert!(!layout.state_dir().join("update-request.json").exists());
}

/// S1: a held `state/staging.lock` answers `Held` to a non-blocking take, and
/// nothing is fetched or staged; the waiting take (the daemon's loop) runs once
/// the lock is free.
#[test]
fn a_held_lock_answers_held_and_stages_nothing() {
    let payload = common::build_payload();
    let dir = origin_dir(
        "2026.10.12",
        &payload,
        "https://example.invalid/p",
        true,
        &no_patch,
    );
    let server = LogServer::serve(Some(dir.path().to_path_buf()));
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let cfg = net_cfg("http://127.0.0.1:9", "2026.10.01");
    let from = [Origin::Url {
        base: server.base(),
    }];

    let LockTake::Taken(daemon) =
        StagingLock::acquire(&layout.state_dir(), LockWait::NoWait).unwrap()
    else {
        panic!("a fresh lock is free");
    };
    let outcome = check_origins(&cfg, &layout, &from, LockWait::NoWait).unwrap();
    assert!(matches!(outcome, FetchOutcome::Held), "{outcome:?}");
    assert!(server.requests().is_empty(), "nothing fetched while held");
    assert!(!layout.state_dir().join("staging").exists());
    assert!(!layout.state_dir().join("update-request.json").exists());
    assert!(origin_records(&layout.state_dir()).is_empty());
    drop(daemon);

    // Released. A child another test forks in this process shares the lock's
    // descriptor until its exec closes it (O_CLOEXEC), so a take right after the
    // release may still read Held for that instant: retry briefly.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let outcome = loop {
        match check_origins(&cfg, &layout, &from, LockWait::NoWait).unwrap() {
            FetchOutcome::Held if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            other => break other,
        }
    };
    assert!(
        matches!(outcome, FetchOutcome::Staged { .. }),
        "{outcome:?}"
    );
}

/// Waiters blocked on `state/staging.lock` (the `-> FLOCK` lines of
/// `/proc/locks` for its inode); `None` when `/proc/locks` is unreadable.
fn flock_waiters(lock: &Path) -> Option<usize> {
    use std::os::unix::fs::MetadataExt;
    let ino = std::fs::metadata(lock).ok()?.ino();
    let locks = std::fs::read_to_string("/proc/locks").ok()?;
    Some(
        locks
            .lines()
            .filter(|l| l.contains("->") && l.contains("FLOCK"))
            .filter(|l| {
                l.split_whitespace()
                    .any(|w| w.rsplit(':').next() == Some(&ino.to_string()) && w.contains(':'))
            })
            .count(),
    )
}

/// S1, G12: two checks that wait on the lock at once stage once; the second
/// sees the request the first wrote and fetches no payload.
#[test]
fn a_concurrent_run_stages_once() {
    let payload = common::build_payload();
    let served = tempfile::tempdir().unwrap();
    let server_dir = served.path().to_path_buf();
    let server = LogServer::serve(Some(server_dir.clone()));
    let dir = origin_dir(
        "2026.10.12",
        &payload,
        &format!("{}/payload.tar.gz", server.base()),
        false,
        &no_patch,
    );
    for f in ["channel-111-stable.toml", "channel-111-stable.toml.minisig"] {
        std::fs::copy(dir.path().join(f), server_dir.join(f)).unwrap();
    }
    std::fs::write(server_dir.join("payload.tar.gz"), &payload).unwrap();

    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let cfg = net_cfg(&server.base(), "2026.10.01");
    let LockTake::Taken(gate) =
        StagingLock::acquire(&layout.state_dir(), LockWait::NoWait).unwrap()
    else {
        panic!("a fresh lock is free");
    };
    let runs: Vec<_> = (0..2)
        .map(|_| {
            let (cfg, layout) = (cfg.clone(), layout.clone());
            std::thread::spawn(move || check_once(&cfg, &layout).unwrap())
        })
        .collect();
    // both runs block on the lock before it is released
    let lock = StagingLock::path(&layout.state_dir());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match flock_waiters(&lock) {
            Some(n) if n >= 2 => break,
            Some(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            Some(n) => panic!("only {n} waiter(s) blocked on the lock"),
            None => {
                eprintln!("/proc/locks unreadable: waiting 500 ms instead");
                std::thread::sleep(std::time::Duration::from_millis(500));
                break;
            }
        }
    }
    assert!(server.requests().is_empty(), "nothing fetched while held");
    drop(gate);
    let outcomes: Vec<_> = runs.into_iter().map(|r| r.join().unwrap()).collect();
    let staged = outcomes
        .iter()
        .filter(|o| matches!(o, FetchOutcome::Staged { .. }))
        .count();
    let already = outcomes
        .iter()
        .filter(|o| matches!(o, FetchOutcome::AlreadyStaged { .. }))
        .count();
    assert_eq!((staged, already), (1, 1), "{outcomes:?}");
    assert_eq!(server.count("GET /payload.tar.gz"), 1, "one payload fetch");
    assert_eq!(server.exact("GET /channel-111-stable.toml"), 2);
    assert_bundle(&layout, "2026.10.12");
    // an internet base: record_success, no per-origin record
    assert!(layout.state_dir().join("last-update-check.json").exists());
    assert!(origin_records(&layout.state_dir()).is_empty());

    // an uncontended run re-stages as before (the hourly retry of a request the
    // helper left in place)
    let outcome = check_once(&cfg, &layout).unwrap();
    assert!(
        matches!(outcome, FetchOutcome::Staged { .. }),
        "{outcome:?}"
    );
}

/// T3 (§9.5): the daemon's staging decision refuses a release whose
/// `min-upgrade-from` is above the running one, before its payload is fetched;
/// at the boundary it stages.
#[test]
fn t3_min_upgrade_from_above_the_installed_release_is_refused() {
    let payload = common::build_payload();
    let served = tempfile::tempdir().unwrap();
    let server = LogServer::serve(Some(served.path().to_path_buf()));
    let dir = origin_dir(
        "2026.10.20",
        &payload,
        &format!("{}/payload.tar.gz", server.base()),
        true,
        &|m| {
            m.replace(
                "min-upgrade-from = \"2026.10.01\"",
                "min-upgrade-from = \"2026.10.10\"",
            )
        },
    );
    for f in ["channel-111-stable.toml", "channel-111-stable.toml.minisig"] {
        std::fs::copy(dir.path().join(f), served.path().join(f)).unwrap();
    }
    std::fs::write(served.path().join("payload.tar.gz"), &payload).unwrap();

    for origin in [
        Origin::Internet {
            base: server.base(),
        },
        Origin::Dir {
            path: dir.path().to_path_buf(),
        },
    ] {
        let node = tempfile::tempdir().unwrap();
        let layout = layout_at(node.path());
        let cfg = net_cfg(&server.base(), "2026.10.05");
        let outcome =
            check_origins(&cfg, &layout, std::slice::from_ref(&origin), LockWait::Wait).unwrap();
        match outcome {
            FetchOutcome::Refused { reason } => {
                assert!(reason.contains("min-upgrade-from"), "{reason}");
                assert!(reason.contains("2026.10.10"), "{reason}");
            }
            other => panic!("expected Refused for {origin:?}, got {other:?}"),
        }
        assert!(!layout.state_dir().join("staging").exists());
        assert!(!layout.state_dir().join("update-request.json").exists());
        assert!(!layout.state_dir().join("last-update-check.json").exists());
    }
    assert_eq!(server.count("payload.tar.gz"), 0, "no payload fetched");

    // installed == min-upgrade-from: stages
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let cfg = net_cfg(&server.base(), "2026.10.10");
    let outcome = check_once(&cfg, &layout).unwrap();
    assert!(
        matches!(outcome, FetchOutcome::Staged { .. }),
        "{outcome:?}"
    );
    assert_bundle(&layout, "2026.10.20");
}

/// T6 (§9.5): an internet base never passes plain http:// to a LAN host (no
/// request made), and neither does its manifest's `payload.url`.
#[test]
fn t6_internet_bases_refuse_plain_http_to_a_lan_host() {
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let cfg = net_cfg("http://192.0.2.1:9/stable", "2026.10.01");
    match check_once(&cfg, &layout).unwrap() {
        FetchOutcome::Degraded { reason, .. } => {
            assert!(reason.contains("refusing plain http://"), "{reason}")
        }
        other => panic!("expected Degraded, got {other:?}"),
    }

    let payload = common::build_payload();
    let served = tempfile::tempdir().unwrap();
    let dir = origin_dir(
        "2026.10.12",
        &payload,
        "http://192.0.2.1:9/payload.tar.gz",
        true,
        &no_patch,
    );
    for f in ["channel-111-stable.toml", "channel-111-stable.toml.minisig"] {
        std::fs::copy(dir.path().join(f), served.path().join(f)).unwrap();
    }
    let server = LogServer::serve(Some(served.path().to_path_buf()));
    let cfg = net_cfg(&server.base(), "2026.10.01");
    match check_once(&cfg, &layout).unwrap() {
        FetchOutcome::Degraded { reason, .. } => {
            assert!(reason.contains("refusing plain http://"), "{reason}")
        }
        other => panic!("expected Degraded, got {other:?}"),
    }
    assert!(!layout.state_dir().join("staging").exists());
}

/// `check_internet = false`: the check contacts no internet base, and neither
/// does the hourly loop, even when the dashboard wakes it.
#[test]
fn check_internet_off_contacts_no_internet_base() {
    let server = LogServer::serve(None);
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let mut cfg = net_cfg(&server.base(), "2026.10.01");
    cfg.fallback_base = Some(format!("{}/fallback", server.base()));
    cfg.check_internet = false;
    assert!(cfg.internet_origins().is_empty());
    assert_eq!(cfg.visible_order(None).len(), 2, "listed, not contacted");
    let outcome = check_once(&cfg, &layout).unwrap();
    assert!(matches!(outcome, FetchOutcome::NoOrigin), "{outcome:?}");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let hooks = node111::daemon::updater::Hooks::default();
    let status = Arc::clone(&hooks.status);
    let wake = Arc::clone(&hooks.wake);
    let state: node111::daemon::dashboard::SharedState = Arc::new(std::sync::RwLock::new(
        node111::daemon::dashboard::DashboardState::new("2026.10.01"),
    ));
    rt.block_on(async {
        let task = tokio::spawn(node111::daemon::updater::run_with_hooks(
            layout.clone(),
            state,
            cfg.clone(),
            hooks,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        // the dashboard's POST /api/v1/update/check: begin_check, then wake
        assert!(!status.begin_check());
        wake.notify_one();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while status.checking() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            !status.checking(),
            "a woken loop with nothing to check settles"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        task.abort();
    });
    assert!(server.requests().is_empty(), "{:?}", server.requests());
    assert_eq!(status.checks(), 0);
    assert_eq!(status.last_result(), None);
    assert!(!layout.state_dir().join("last-update-check.json").exists());
    assert!(!StagingLock::path(&layout.state_dir()).exists());
}
