//! Step 6 — stage (install-time; the installer IS root, so it promotes directly —
//! the daemon-time split with the root helper doesn't apply here): fetch payload
//! → `releases/<ver>.tmp-<pid>/` → verify sha256 + extracted-tree manifest +
//! Layer 5 pins (`ChannelManifest::verify_layer5_tree`) →
//! fsync → rename → flip `current` (single atomic unit: KB rides inside the
//! payload) → establish the initial floor. Prune to 2 only after probation —
//! a fresh install has no probation, so nothing is pruned here. Then, when the
//! release ships the Meshtastic presence reader, its venv (`venvs/presence`,
//! os/5-messaging/README.md §14) is built from the pinned lock in that tree
//! ([`presence_venv`]; never fails the install).

use super::context::InstallContext;
use crate::calver::Calver;
use crate::floor::Floor;
use crate::manifest::ChannelManifest;

/// Download + verify + promote the release payload, then build the presence
/// venv from it ([`presence_venv`], on both the fresh and the "already staged"
/// path). Returns the release dir.
pub fn run(
    ctx: &InstallContext,
    fetcher: &crate::util::http::Fetcher,
    manifest: &ChannelManifest,
) -> anyhow::Result<std::path::PathBuf> {
    let payload = manifest
        .payload_for(crate::daemon::updater::fetch::TARGET_TRIPLE)
        .ok_or_else(|| anyhow::anyhow!("manifest ships no payload for this target"))?;

    let release_dir = ctx
        .layout
        .root
        .join("releases")
        .join(manifest.release().to_string());
    // Content probe (never mere existence): a complete prior stage verifies —
    // against its own payload manifest AND the Layer 5 pins (§19 L1), so a release
    // dir carrying an unpinned or altered bin/meshmud, panel bundle or presence
    // lock is re-extracted from the pinned tarball, never accepted as-is.
    if release_dir.join("bin/111").exists()
        && crate::daemon::updater::stage::verify_tree(&release_dir).is_ok()
        && manifest
            .verify_layer5_tree(&release_dir, crate::daemon::updater::fetch::TARGET_TRIPLE)
            .is_ok()
    {
        eprintln!(
            "111 install: release {} already staged+verified",
            manifest.release()
        );
        presence_venv(ctx, manifest, &release_dir);
        return Ok(release_dir);
    }

    let tarball = fetcher.get(&payload.url)?;
    anyhow::ensure!(
        tarball.len() as u64 == payload.size,
        "payload size {} != manifest {}",
        tarball.len(),
        payload.size
    );
    let got_sha = crate::util::hash::sha256_bytes(&tarball);
    anyhow::ensure!(got_sha == payload.sha256, "payload sha256 mismatch");

    let releases = ctx.layout.root.join("releases");
    std::fs::create_dir_all(&releases)?;
    // F1: a prior kill mid-extract leaves `.{ver}.tmp-<pid>` debris — sweep it
    // (content-probe healing is the whole point of the tmp+rename design).
    for entry in std::fs::read_dir(&releases)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&format!(".{}.tmp-", manifest.release())) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    let tmp = releases.join(format!(
        ".{}.tmp-{}",
        manifest.release(),
        std::process::id()
    ));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir(&tmp)?;
    let result = (|| -> anyhow::Result<()> {
        crate::daemon::updater::stage::safe_unpack(&tarball, &tmp)?;
        crate::daemon::updater::stage::verify_tree(&tmp)?;
        // Layer 5 assets equal the signed manifest's pins (os/5-messaging/README.md §19 L1).
        manifest.verify_layer5_tree(&tmp, crate::daemon::updater::fetch::TARGET_TRIPLE)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    if release_dir.exists() {
        std::fs::remove_dir_all(&release_dir)?; // torn prior stage (failed verify above)
    }
    std::fs::rename(&tmp, &release_dir)?;
    crate::util::fs::fsync_dir(&releases)?;

    // Flip current (single atomic unit — binary+KB can never split-brain).
    crate::update_helper::flip_current(&ctx.layout, &manifest.release().to_string())?;

    // First-install floor: established from THIS verified manifest (the daemon
    // updater and helper only ever move it forward from here).
    let floor_path = ctx.state_dir().join("floor.json");
    if Floor::read(&floor_path)?.is_none() {
        Floor {
            schema: 1,
            highest_release: manifest.release().to_string(),
            highest_manifest_date: manifest.date().to_string(),
            highest_keychain_generation: manifest
                .keychain()
                .iter()
                .map(|k| k.generation)
                .max()
                .unwrap_or(0),
        }
        .write(&floor_path)?;
    }

    // /usr/local/bin/111 → current/bin/111
    let link = ctx.sys("/usr/local/bin/111");
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(ctx.layout.current_release().join("bin/111"), &link)?;

    presence_venv(ctx, manifest, &release_dir);
    Ok(release_dir)
}

/// The presence reader's venv for the staged release (os/5-messaging/README.md
/// §14, §19 L1): when the manifest pins `[presence-lock]`, the lock in
/// `release_dir` (already checked against that pin by `verify_layer5_tree`)
/// drives `daemon::transport::venv::presence::ensure` in install mode — the
/// build venv's pip downloads the locked closure into
/// `cache/packages/presence/` (offline rebuild assets, hash-recorded in the
/// receipt with the rest of `cache/`), then installs from there only.
/// Fail closed and never fatal: no python3, a bad hash or a failed download
/// leaves the previous venv in place and is recorded in
/// `venvs/presence.status.json`, where the meshtastic service reads it; the
/// node runs (without Nearby when no earlier venv exists). Step 7 hands
/// `venvs/` to node111 afterwards.
pub fn presence_venv(
    ctx: &InstallContext,
    manifest: &ChannelManifest,
    release_dir: &std::path::Path,
) -> crate::daemon::transport::venv::presence::Outcome {
    use crate::daemon::transport::venv::presence::{self, Mode, Outcome};
    if manifest.presence_lock().is_none() {
        return Outcome::NotShipped;
    }
    let root = &ctx.layout.root;
    let outcome = presence::ensure(
        root,
        ctx.runner,
        &presence::lockfile(release_dir),
        &presence::cache_dir(root),
        Mode::Install,
    );
    match &outcome {
        Outcome::NotShipped | Outcome::Unchanged => {}
        Outcome::Built => eprintln!("111 install: presence venv built (venvs/presence)"),
        Outcome::Unavailable {
            error,
            previous_kept: true,
        } => eprintln!("111 install: presence venv not rebuilt: {error} — the previous one stays"),
        Outcome::Unavailable { error, .. } => eprintln!(
            "111 install: presence venv not built: {error} — the node runs; Nearby stays empty"
        ),
    }
    outcome
}

/// rns-lock.txt from the signed manifest (fetched + pin-verified) — input to
/// the pip closure download and the daemon's venv build.
pub fn fetch_rns_lock(
    ctx: &InstallContext,
    fetcher: &crate::util::http::Fetcher,
    manifest: &ChannelManifest,
) -> anyhow::Result<std::path::PathBuf> {
    let pin = manifest.rns_lock();
    let bytes = fetcher.get(&pin.url)?;
    let got = crate::util::hash::sha256_bytes(&bytes);
    anyhow::ensure!(
        got == pin.sha256,
        "rns-lock sha256 {got} != manifest {}",
        pin.sha256
    );
    let dest = ctx.layout.root.join("cache/packages/python/rns-lock.txt");
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::util::fs::atomic_write(&dest, &bytes, 0o644)?;
    Ok(dest)
}

/// Installed-version gate (spec: refuses any version ≤ installed, anything
/// below min-upgrade-from). `receipt` is untrusted — a corrupt/missing receipt
/// never unlocks a downgrade; we treat missing as "fresh".
pub fn version_gate(
    receipt: Option<&super::receipt::Receipt>,
    manifest: &ChannelManifest,
) -> anyhow::Result<()> {
    let Some(r) = receipt else { return Ok(()) };
    let Ok(installed) = r.version.parse::<Calver>() else {
        return Ok(());
    };
    // Equal version proceeds: every step probes content, so a re-run of the
    // same release is an idempotent no-op (and resumes a failed install).
    anyhow::ensure!(
        manifest.release() >= installed,
        "refusing downgrade: installed {installed}, manifest {}",
        manifest.release()
    );
    anyhow::ensure!(
        installed >= manifest.min_upgrade_from(),
        "installed version {installed} is below min-upgrade-from {} — reinstall from a fresh shim",
        manifest.min_upgrade_from()
    );
    Ok(())
}
