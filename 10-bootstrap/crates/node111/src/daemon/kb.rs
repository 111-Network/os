//! KB snapshot: rides inside the release payload at `current/share/kb/` (single
//! atomic unit with the binary). Integrity-verified against the payload tree
//! manifest at daemon start; no browsing surface in v1.

use std::path::Path;

#[derive(Debug, Clone)]
pub struct KbStatus {
    /// Deterministic digest of the KB's manifest lines (sha256, hex).
    pub sha256: String,
    /// Number of KB entry files (*.md).
    pub entries: u32,
}

/// Parse `share/payload.manifest` lines (`sha256␠␠relpath`) for the KB subset.
pub fn parse_manifest_kb_lines(manifest: &str) -> Vec<(String, String)> {
    manifest
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let sha = parts.next()?;
            let path = parts.next()?;
            if path.starts_with("share/kb/") {
                Some((sha.to_string(), path.to_string()))
            } else {
                None
            }
        })
        .collect()
}

/// Verify + summarize the KB at `<root>/current/`. Every file pinned in the
/// manifest's share/kb/ section must exist and hash-match; a mismatch or a
/// missing tree ⇒ Err (health component red).
pub fn verify_current(root: &Path) -> anyhow::Result<KbStatus> {
    let release = root.join("current");
    let manifest_path = release.join("share/payload.manifest");
    let manifest = std::fs::read_to_string(&manifest_path).map_err(|e| {
        anyhow::anyhow!(
            "payload manifest unreadable at {}: {e}",
            manifest_path.display()
        )
    })?;
    let kb_lines = parse_manifest_kb_lines(&manifest);
    anyhow::ensure!(
        !kb_lines.is_empty(),
        "payload manifest pins no share/kb/ files at {}",
        manifest_path.display()
    );

    let mut entries = 0u32;
    let mut digest = sha2::Sha256::new();
    use sha2::Digest;
    for (want_sha, rel) in &kb_lines {
        let file = release.join(rel);
        let got = crate::util::hash::sha256_file(&file)
            .map_err(|e| anyhow::anyhow!("KB file unreadable: {e}"))?;
        anyhow::ensure!(
            &got == want_sha,
            "KB integrity failure: {rel} sha256 {got} != manifest {want_sha}"
        );
        if rel.ends_with(".md") {
            entries += 1;
        }
        digest.update(format!("{want_sha}  {rel}\n"));
    }
    Ok(KbStatus {
        sha256: format!("{:x}", digest.finalize()),
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_kb(root: &Path, corrupt: bool) {
        let kb = root.join("current/share/kb");
        std::fs::create_dir_all(&kb).unwrap();
        let e1 = kb.join("llms.txt");
        let e2 = kb.join("reticulum.md");
        std::fs::write(&e1, "index\n").unwrap();
        std::fs::write(&e2, "entry\n").unwrap();
        let mut manifest = String::new();
        for f in ["share/kb/llms.txt", "share/kb/reticulum.md"] {
            let sha = crate::util::hash::sha256_file(&root.join("current").join(f)).unwrap();
            manifest.push_str(&format!("{sha}  {f}\n"));
        }
        manifest.push_str(&format!(
            "{}  bin/111\n",
            crate::util::hash::sha256_bytes(b"bin")
        ));
        if corrupt {
            std::fs::write(&e2, "TAMPERED\n").unwrap();
        }
        std::fs::write(root.join("current/share/payload.manifest"), manifest).unwrap();
    }

    #[test]
    fn valid_kb_verifies() {
        let dir = tempfile::tempdir().unwrap();
        write_kb(dir.path(), false);
        let status = verify_current(dir.path()).unwrap();
        assert_eq!(status.entries, 1); // only .md counts
        assert_eq!(status.sha256.len(), 64);
    }

    #[test]
    fn tampered_kb_refused() {
        let dir = tempfile::tempdir().unwrap();
        write_kb(dir.path(), true);
        let err = verify_current(dir.path()).unwrap_err();
        assert!(err.to_string().contains("integrity failure"), "{err}");
    }

    #[test]
    fn missing_tree_is_err() {
        let dir = tempfile::tempdir().unwrap();
        assert!(verify_current(dir.path()).is_err());
    }
}
