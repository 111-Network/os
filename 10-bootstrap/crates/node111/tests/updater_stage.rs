//! T10 acceptance: updater fetch/verify/stage against a fixture HTTP server —
//! happy path, stale manifest, bad minisig, replayed keychain generation,
//! tampered payload, clock-skew marker, and the Layer 5 cross-check
//! (`verify_layer5_tree` after staging, os/messaging/README.md §19 L1). Asserts
//! the daemon never writes outside state/staging (no releases/, no `current`).

mod common;

use node111::calver::Calver;
use node111::daemon::updater::fetch::{check_once, FetchOutcome, UpdaterConfig};
use node111::daemon::DaemonLayout;
use node111::floor::Floor;
use std::path::Path;
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
