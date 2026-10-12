//! APK steps (spec phone flow steps 2–3): collision check (pm path + signature
//! dump — the GitHub Termux test key is PUBLIC, so this exists solely to avoid
//! Android's shared-uid signature conflict; the manifest's sha256 pins are the
//! only trust anchor) → `adb install -r` ×3 → versionCode/signature verify.

use super::adb::Adb;
use crate::manifest::PinnedArtifact;
use std::path::{Path, PathBuf};

/// The three pinned APKs (spec §Artifacts): package id + expected versionName.
pub const APK_PACKAGES: &[(&str, &str, &str)] = &[
    ("com.termux", "termux", "0.118.3"),
    ("com.termux.boot", "termux-boot", "0.8.1"),
    ("com.termux.api", "termux-api", "0.53.0"),
];

#[derive(Debug)]
pub struct ApkSet {
    /// (package, artifact, local path, sha256-verified)
    pub files: Vec<(String, PinnedArtifact, PathBuf)>,
}

/// Resolve + verify the local APK files against the manifest pins.
pub fn resolve(cache: &Path, apks: &[PinnedArtifact]) -> anyhow::Result<ApkSet> {
    let mut files = Vec::new();
    for (pkg, name, _version) in APK_PACKAGES {
        let artifact = apks
            .iter()
            .find(|a| a.name == *name)
            .ok_or_else(|| anyhow::anyhow!("manifest ships no apk.{name}"))?;
        let path = cache.join(format!("{name}.apk"));
        anyhow::ensure!(
            path.exists(),
            "APK missing at {} (prefetch step)",
            path.display()
        );
        let sha = crate::util::hash::sha256_file(&path)?;
        anyhow::ensure!(
            sha == artifact.sha256,
            "{name}.apk sha256 {sha} != manifest {} — refusing to install",
            artifact.sha256
        );
        files.push((pkg.to_string(), artifact.clone(), path));
    }
    Ok(ApkSet { files })
}

/// Pre-install collision check: existing com.termux with a signature that will
/// conflict with ours ⇒ hard stop with instructions. (Android itself enforces
/// signature continuity on -r; we surface it with a clear message, not a raw
/// INSTALL_FAILED dump.)
pub fn collision_check(adb: &Adb<'_>) -> anyhow::Result<()> {
    let out = adb.shell("pm path com.termux")?;
    let installed = out.ok() && out.stdout.contains("package:");
    if !installed {
        return Ok(());
    }
    // Signature dump (informational — continuity is enforced at install time;
    // the public test key authenticates nothing, per spec).
    let dump = adb.shell("pm dump com.termux | grep -m1 -E 'signatures=|Package签名'")?;
    eprintln!(
        "111 phone: com.termux already installed — will attempt signature-continuity install (-r).{}",
        if dump.ok() && !dump.stdout.trim().is_empty() {
            " Existing signature record present.".to_string()
        } else {
            String::new()
        }
    );
    Ok(())
}

/// `adb install -r` ×3 with versionName verification post-install.
pub fn install_all(adb: &Adb<'_>, set: &ApkSet) -> anyhow::Result<()> {
    for (pkg, artifact, path) in &set.files {
        eprintln!(
            "111 phone: installing {pkg} ({} bytes pinned)…",
            artifact.size
        );
        eprintln!(
            "111 phone: (stock Android shows one install-confirm tap per APK — watch the screen)"
        );
        let out = adb.install(&path.to_string_lossy())?;
        let body = format!("{}\n{}", out.stdout, out.stderr);
        if !out.ok() || !body.contains("Success") {
            if body.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE")
                || body.contains("INSTALL_PARSE_FAILED_INCONSISTENT_CERTIFICATES")
            {
                anyhow::bail!(
                    "signature conflict on {pkg}: an existing Termux with a DIFFERENT signature is installed.\n\
                     The GitHub Termux builds share one public test key; a foreign-signed Termux cannot coexist.\n\
                     Uninstall the existing Termux from the phone (its app data is lost) or use that Termux instead."
                );
            }
            if body.contains("INSTALL_FAILED_USER_RESTRICTED")
                || body.contains("INSTALL_CANCELED_BY_USER")
            {
                anyhow::bail!(
                    "install of {pkg} blocked on-device — on MIUI/HyperOS enable the Mi-account-gated \
                     'Install via USB' toggle and confirm the per-APK prompts, then re-run"
                );
            }
            anyhow::bail!("adb install {pkg} failed: {}", body.trim());
        }
        // post-install verify: package present with the pinned versionName
        let (_, _, want_version) = APK_PACKAGES
            .iter()
            .find(|(p, _, _)| p == pkg)
            .expect("set only carries pinned packages");
        let dump = adb.shell(&format!("dumpsys package {pkg} | grep -m1 versionName"))?;
        anyhow::ensure!(
            dump.ok() && dump.stdout.contains(want_version),
            "post-install verify failed for {pkg}: expected versionName {want_version}, dumpsys said: {}",
            dump.stdout.trim()
        );
    }
    Ok(())
}
