//! Step 6 — stage (install-time; the installer IS root, so it promotes directly —
//! the daemon-time split with the root helper doesn't apply here): fetch payload
//! → `releases/<ver>.tmp-<pid>/` → verify sha256 + extracted-tree manifest →
//! fsync → rename → flip `current` (single atomic unit: KB rides inside the
//! payload) → establish the initial floor. Prune to 2 only after probation —
//! a fresh install has no probation, so nothing is pruned here.

use super::context::InstallContext;
use crate::calver::Calver;
use crate::floor::Floor;
use crate::manifest::ChannelManifest;

/// Download + verify + promote the release payload. Returns the release dir.
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
    // Content probe (never mere existence): a complete prior stage verifies.
    if release_dir.join("bin/111").exists()
        && crate::daemon::updater::stage::verify_tree(&release_dir).is_ok()
    {
        eprintln!(
            "111 install: release {} already staged+verified",
            manifest.release()
        );
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

    Ok(release_dir)
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
