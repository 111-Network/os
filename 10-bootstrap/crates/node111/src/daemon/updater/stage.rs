//! Payload staging: verify tarball pin → extract to `state/staging/<ver>.tmp` →
//! verify the extracted tree against `share/payload.manifest` → fsync → rename
//! → drop `state/update-request.json`. The daemon's staging area is the ONLY
//! place it may write; promotion to releases/ is the root helper's job (D12).

use crate::calver::Calver;
use crate::manifest::PinnedArtifact;
use flate2::read::GzDecoder;
use std::path::{Component, Path, PathBuf};

/// Extract a gzipped tarball with strict path safety: no absolute paths, no
/// `..`, no symlink/hardlink entries (payload is a plain file tree).
pub(crate) fn safe_unpack(tarball: &[u8], dest: &Path) -> anyhow::Result<()> {
    let mut archive = tar::Archive::new(GzDecoder::new(tarball));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        anyhow::ensure!(
            path.components().all(|c| matches!(c, Component::Normal(_))),
            "unsafe path in payload: {}",
            path.display()
        );
        let header = entry.header();
        let kind = header.entry_type();
        anyhow::ensure!(
            kind == tar::EntryType::Regular || kind == tar::EntryType::Directory,
            "payload carries non-regular entry ({}: {:?}) — symlinks/hardlinks/devices are refused",
            path.display(),
            kind
        );
        let out = dest.join(&path);
        if kind == tar::EntryType::Directory {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mode = header.mode().unwrap_or(0o644);
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut buf)?;
            crate::util::fs::atomic_write(
                &out,
                &buf,
                if mode & 0o111 != 0 { 0o755 } else { 0o644 },
            )?;
        }
    }
    Ok(())
}

/// Verify every regular file in the extracted tree against the manifest the
/// tree itself carries (`share/payload.manifest`: `sha256␠␠relpath` lines).
pub fn verify_tree(tree: &Path) -> anyhow::Result<()> {
    let manifest = std::fs::read_to_string(tree.join("share/payload.manifest"))
        .map_err(|e| anyhow::anyhow!("payload missing share/payload.manifest: {e}"))?;
    let mut pinned = 0usize;
    for line in manifest.lines() {
        let mut parts = line.split_whitespace();
        let (Some(sha), Some(rel)) = (parts.next(), parts.next()) else {
            continue;
        };
        pinned += 1;
        let got = crate::util::hash::sha256_file(&tree.join(rel))
            .map_err(|e| anyhow::anyhow!("payload tree file unreadable ({rel}): {e}"))?;
        anyhow::ensure!(got == sha, "payload tree hash mismatch on {rel}");
    }
    anyhow::ensure!(pinned > 0, "payload manifest pins nothing");
    Ok(())
}

/// Stage a downloaded payload: pin-check the tarball bytes, unpack safely,
/// verify the tree, atomic rename into `staging/<ver>/`.
pub fn stage_payload(
    tarball: &[u8],
    expected: &PinnedArtifact,
    staging_root: &Path,
    version: &Calver,
) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        tarball.len() as u64 == expected.size,
        "payload size {} != manifest {}",
        tarball.len(),
        expected.size
    );
    let got_sha = crate::util::hash::sha256_bytes(tarball);
    anyhow::ensure!(
        got_sha == expected.sha256,
        "payload sha256 {got_sha} != manifest {}",
        expected.sha256
    );

    std::fs::create_dir_all(staging_root)?;
    let tmp = staging_root.join(format!(".{version}.tmp-{}", std::process::id()));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir(&tmp)?;

    let result = (|| -> anyhow::Result<()> {
        // Staging bundle (D12): the raw pinned tarball + the extracted tree/ +
        // (fetch.rs adds) the signed channel manifest + minisig. The root
        // helper re-verifies everything from these bytes alone.
        crate::util::fs::atomic_write(&tmp.join("payload.tar.gz"), tarball, 0o644)?;
        let tree = tmp.join("tree");
        std::fs::create_dir(&tree)?;
        safe_unpack(tarball, &tree)?;
        verify_tree(&tree)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }

    let dest = staging_root.join(version.to_string());
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?; // re-stage of same version (content-verified identical)
    }
    std::fs::rename(&tmp, &dest)?;
    crate::util::fs::fsync_dir(staging_root)?;
    Ok(dest)
}

/// Drop the daemon→helper signal (`state/update-request.json`, schema 1).
/// The 111node-update.path unit watches this file.
pub fn write_update_request(
    state_dir: &Path,
    version: &Calver,
    manifest_sha256: &str,
    staged_path: &Path,
) -> anyhow::Result<()> {
    let body = serde_json::json!({
        "schema": 1,
        "version": version.to_string(),
        "manifest_sha256": manifest_sha256,
        "staged_path": staged_path.to_string_lossy(),
        "requested_at": time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "unknown".into()),
    });
    crate::util::fs::atomic_write(
        &state_dir.join("update-request.json"),
        body.to_string().as_bytes(),
        0o644,
    )
}

#[cfg(test)]
mod tests {
    // Integration coverage in tests/updater_stage.rs.
}
