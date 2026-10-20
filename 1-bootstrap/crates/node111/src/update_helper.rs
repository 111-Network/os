//! `111 update-helper` — root oneshot (via 111node-update.path/service). The ONLY
//! actor that promotes staged payloads into `releases/`, flips `current`, and writes
//! the probation journal + monotonic floor. Re-verifies manifest minisig + tarball
//! pin, and promotes a tree IT extracted itself — the daemon's staged tree is only
//! a hint; node111→root trust never flows through daemon-writable content (D12).
//! The extracted tree is checked against its own `share/payload.manifest` and then
//! against the manifest's Layer 5 pins (`ChannelManifest::verify_layer5_tree`).

use crate::calver::Calver;
use crate::daemon::updater::stage::safe_unpack;
use crate::daemon::updater::MANIFEST_PUBKEY;
use crate::daemon::DaemonLayout;
use crate::floor::Floor;
use crate::manifest::ChannelManifest;
use crate::probation::{Journal, PROBATION_SECS};
use crate::util::run::Runner;
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperAction {
    Promoted { version: String },
    Reverted { to: String },
    Finalized { version: String },
    Nothing,
}

#[derive(Debug, Default)]
pub struct HelperReport {
    pub actions: Vec<HelperAction>,
}

/// Current release calver (basename of the `current` symlink), if any.
pub fn current_version(layout: &DaemonLayout) -> Option<String> {
    std::fs::read_link(layout.current_release())
        .ok()?
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

/// Atomically flip `current` → releases/<version> (tmp symlink + rename).
pub fn flip_current(layout: &DaemonLayout, version: &str) -> anyhow::Result<()> {
    let target = layout.root.join("releases").join(version);
    anyhow::ensure!(target.is_dir(), "flip target missing: {}", target.display());
    let tmp = layout
        .root
        .join(format!(".current.tmp-{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(&target, &tmp)?;
    std::fs::rename(&tmp, layout.current_release())?;
    crate::util::fs::fsync_dir(&layout.root)?;
    Ok(())
}

/// Prune `releases/` to exactly `keep` entries, newest calvers first; `current`
/// (and `protect`, e.g. an open probation's revert target) are never removed.
pub fn prune_releases(
    layout: &DaemonLayout,
    keep: usize,
    protect: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    let releases_dir = layout.root.join("releases");
    let mut versions: Vec<Calver> = std::fs::read_dir(&releases_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| Calver::from_str(&e.file_name().to_string_lossy()).ok())
                .collect()
        })
        .unwrap_or_default();
    versions.sort();
    versions.reverse(); // newest first

    let current = current_version(layout);
    let mut retained: Vec<String> = Vec::new();
    if let Some(c) = &current {
        retained.push(c.clone());
    }
    if let Some(p) = protect {
        if !retained.iter().any(|r| r == p) {
            retained.push(p.to_string());
        }
    }
    for v in &versions {
        if retained.len() >= keep.max(1) {
            break;
        }
        let vs = v.to_string();
        if !retained.contains(&vs) {
            retained.push(vs);
        }
    }

    let mut removed = Vec::new();
    for v in &versions {
        let vs = v.to_string();
        if !retained.contains(&vs) {
            std::fs::remove_dir_all(releases_dir.join(&vs))?;
            removed.push(vs);
        }
    }
    Ok(removed)
}

fn read_json(path: &Path) -> anyhow::Result<Option<serde_json::Value>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(serde_json::from_slice(&b)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn consume(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Handle one update-request: re-verify → promote → journal → floor → flip →
/// restart. Errors leave the running version untouched.
fn handle_update_request(
    layout: &DaemonLayout,
    runner: &dyn Runner,
    now_epoch: u64,
    report: &mut HelperReport,
) -> anyhow::Result<()> {
    let req_path = layout.state_dir().join("update-request.json");
    let Some(req) = read_json(&req_path)? else {
        return Ok(());
    };
    let version = req["version"].as_str().unwrap_or("").to_string();
    let staged = PathBuf::from(req["staged_path"].as_str().unwrap_or(""));

    // 1. Re-verify the signed manifest from the staging bundle.
    let manifest_bytes = std::fs::read(staged.join("channel-111-stable.toml"))
        .map_err(|e| anyhow::anyhow!("staged manifest unreadable: {e}"))?;
    let sig = std::fs::read_to_string(staged.join("channel-111-stable.toml.minisig"))
        .map_err(|e| anyhow::anyhow!("staged minisig unreadable: {e}"))?;
    let floor = Floor::read(&layout.state_dir().join("floor.json"))?.unwrap_or_else(Floor::initial);
    let manifest =
        ChannelManifest::parse_verified(&manifest_bytes, &sig, MANIFEST_PUBKEY.trim(), &floor)?;
    anyhow::ensure!(
        manifest.release().to_string() == version,
        "request version {version} != manifest release {}",
        manifest.release()
    );

    // 2. Idempotency: already running this (or newer).
    if let Some(current) = current_version(layout) {
        if let Ok(cur_cv) = Calver::from_str(&current) {
            if manifest.release() <= cur_cv {
                consume(&req_path);
                report.actions.push(HelperAction::Nothing);
                return Ok(());
            }
        }
    }

    // 3. Re-hash the raw tarball against the manifest pin, then extract OURSELVES.
    let payload_pin = manifest
        .payload_for(crate::daemon::updater::fetch::TARGET_TRIPLE)
        .ok_or_else(|| anyhow::anyhow!("manifest ships no payload for this target"))?;
    let tarball = std::fs::read(staged.join("payload.tar.gz"))
        .map_err(|e| anyhow::anyhow!("staged tarball unreadable: {e}"))?;
    anyhow::ensure!(
        tarball.len() as u64 == payload_pin.size,
        "staged tarball size mismatch"
    );
    let got_sha = crate::util::hash::sha256_bytes(&tarball);
    anyhow::ensure!(
        got_sha == payload_pin.sha256,
        "staged tarball sha256 mismatch"
    );

    let releases_dir = layout.root.join("releases");
    std::fs::create_dir_all(&releases_dir)?;
    let tmp = releases_dir.join(format!(".{version}.tmp-{}", std::process::id()));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir(&tmp)?;
    let extract_result = (|| -> anyhow::Result<()> {
        safe_unpack(&tarball, &tmp)?;
        crate::daemon::updater::stage::verify_tree(&tmp)?;
        // Layer 5 (os/5-messaging/README.md §19 L1): bin/meshmud, panels/<bundle>/ and
        // the presence lock must equal the signed manifest's pins, and nothing unpinned
        // rides along. This is the authoritative check — on the tree WE extracted.
        manifest.verify_layer5_tree(&tmp, crate::daemon::updater::fetch::TARGET_TRIPLE)?;
        Ok(())
    })();
    if let Err(e) = extract_result {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    let dest = releases_dir.join(&version);
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::rename(&tmp, &dest)?;
    crate::util::fs::fsync_dir(&releases_dir)?;

    // 4. Journal BEFORE the flip (the actor that survives the restart).
    let previous = current_version(layout).unwrap_or_default();
    Journal {
        schema: 1,
        pending: true,
        version: version.clone(),
        previous: previous.clone(),
        deadline_epoch: now_epoch + PROBATION_SECS,
        health_confirmed: false,
    }
    .write(&layout.state_dir().join("update.json"))?;

    // 5. Advance the monotonic floor (root-written).
    let max_gen = manifest
        .keychain()
        .iter()
        .map(|k| k.generation)
        .max()
        .unwrap_or(0);
    let mut new_floor = floor.clone();
    if manifest.release().to_string() > floor.highest_release {
        new_floor.highest_release = manifest.release().to_string();
    }
    if manifest.date().to_string() > floor.highest_manifest_date {
        new_floor.highest_manifest_date = manifest.date().to_string();
    }
    new_floor.highest_keychain_generation = new_floor.highest_keychain_generation.max(max_gen);
    new_floor.write(&layout.state_dir().join("floor.json"))?;

    // 6. Flip + restart.
    flip_current(layout, &version)?;
    runner.run("systemctl", &["restart", "111node.service"])?;

    consume(&req_path);
    report.actions.push(HelperAction::Promoted { version });
    Ok(())
}

/// READY-but-503 revert: the daemon asked for a rollback of its own release.
fn handle_revert_request(
    layout: &DaemonLayout,
    runner: &dyn Runner,
    report: &mut HelperReport,
) -> anyhow::Result<()> {
    let req_path = layout.state_dir().join("revert-request.json");
    if read_json(&req_path)?.is_none() {
        return Ok(());
    }
    consume(&req_path);
    let journal = Journal::read(&layout.state_dir().join("update.json"))?;
    let Some(j) = journal else {
        report.actions.push(HelperAction::Nothing);
        return Ok(());
    };
    if !j.pending || j.previous.is_empty() {
        report.actions.push(HelperAction::Nothing);
        return Ok(());
    }
    flip_current(layout, &j.previous)?;
    Journal::clear(&layout.state_dir().join("update.json"))?;
    runner.run("systemctl", &["restart", "111node.service"])?;
    report
        .actions
        .push(HelperAction::Reverted { to: j.previous });
    Ok(())
}

/// Probation finalization: the daemon's sustained-green signal confirms the
/// journal; prune to exactly 2 (never during an open probation).
fn handle_health_confirmed(
    layout: &DaemonLayout,
    now_epoch: u64,
    report: &mut HelperReport,
) -> anyhow::Result<()> {
    let signal = layout.state_dir().join("health-confirmed.json");
    if !signal.exists() {
        return Ok(());
    }
    consume(&signal);
    let Some(mut j) = Journal::read(&layout.state_dir().join("update.json"))? else {
        report.actions.push(HelperAction::Nothing);
        return Ok(());
    };
    if !j.pending || now_epoch > j.deadline_epoch {
        // Expired probations are update-guard's job at the next start.
        report.actions.push(HelperAction::Nothing);
        return Ok(());
    }
    j.health_confirmed = true;
    j.pending = false;
    j.write(&layout.state_dir().join("update.json"))?;
    prune_releases(layout, 2, None)?;
    report
        .actions
        .push(HelperAction::Finalized { version: j.version });
    Ok(())
}

/// Full pass over every pending signal (path-unit trigger).
pub fn run_with(
    layout: &DaemonLayout,
    runner: &dyn Runner,
    now_epoch: u64,
) -> anyhow::Result<HelperReport> {
    let mut report = HelperReport::default();
    handle_update_request(layout, runner, now_epoch, &mut report)?;
    handle_revert_request(layout, runner, &mut report)?;
    handle_health_confirmed(layout, now_epoch, &mut report)?;
    Ok(report)
}

/// Production entry (`111 update-helper`, root via 111node-update.service).
pub fn run() -> anyhow::Result<()> {
    let layout = DaemonLayout {
        root: crate::util::paths::root(),
        etc: PathBuf::from("/etc/111"),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let report = run_with(&layout, &crate::util::run::SystemRunner, now)?;
    for action in &report.actions {
        eprintln!("111 update-helper: {action:?}");
    }
    Ok(())
}
