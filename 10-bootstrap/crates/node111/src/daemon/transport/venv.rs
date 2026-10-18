//! rnsd venv: `/var/lib/111/venvs/rns` built with `pip install --require-hashes`
//! (hash-locked closure, lockfile sha256 pinned in the signed channel manifest).
//! Integrity check at daemon start; torn venvs rebuild from
//! `cache/packages/python/` (offline path; index allowed only at install time).

use crate::daemon::DaemonLayout;
use crate::util::run::Runner;
use std::path::{Path, PathBuf};

pub fn venv_dir(layout: &DaemonLayout) -> PathBuf {
    layout.root.join("venvs/rns")
}

fn integrity_marker(venv: &Path) -> PathBuf {
    venv.join(".111-integrity")
}

/// Integrity: venv exists, rnsd present, marker sha256 == lockfile sha256.
pub fn integrity_ok(layout: &DaemonLayout, lockfile: &Path) -> bool {
    let venv = venv_dir(layout);
    if !venv.join("bin/rnsd").exists() {
        return false;
    }
    let (Ok(want), Ok(got)) = (
        crate::util::hash::sha256_file(lockfile),
        std::fs::read_to_string(integrity_marker(&venv)),
    ) else {
        return false;
    };
    got.trim() == want
}

/// Build (or rebuild) the venv. Offline-first: `--no-index --find-links` the
/// prefetched wheel cache; `allow_index` additionally permits PyPI (install
/// time only — the daemon rebuilds strictly from cache).
pub fn build(
    layout: &DaemonLayout,
    runner: &dyn Runner,
    lockfile: &Path,
    cache_py: &Path,
    allow_index: bool,
) -> anyhow::Result<()> {
    let venv = venv_dir(layout);
    if venv.exists() {
        std::fs::remove_dir_all(&venv)?;
    }
    if let Some(parent) = venv.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let venv_s = venv.to_string_lossy().into_owned();
    let out = runner.run("python3", &["-m", "venv", &venv_s])?;
    anyhow::ensure!(out.ok(), "python3 -m venv failed: {}", out.stderr.trim());

    let lock_s = lockfile.to_string_lossy().into_owned();
    let cache_s = cache_py.to_string_lossy().into_owned();
    let pip = format!("{venv_s}/bin/pip");
    let mut args: Vec<&str> = vec![
        "install",
        "--require-hashes",
        "-r",
        &lock_s,
        "--no-index",
        "--find-links",
        &cache_s,
    ];
    if allow_index {
        args = vec!["install", "--require-hashes", "-r", &lock_s];
    }
    let out = runner.run(&pip, &args)?;
    if !out.ok() && !allow_index {
        anyhow::bail!(
            "venv rebuild from cache failed (torn cache?): {}",
            out.stderr.trim()
        );
    }
    if !out.ok() && allow_index {
        anyhow::bail!("venv build failed: {}", out.stderr.trim());
    }

    let lock_sha = crate::util::hash::sha256_file(lockfile)?;
    std::fs::create_dir_all(&venv)?; // defensive (mocked venv runs create nothing)
    crate::util::fs::atomic_write(&integrity_marker(&venv), lock_sha.as_bytes(), 0o644)?;
    Ok(())
}

/// Ensure the venv is intact; rebuild when torn.
pub fn ensure(
    layout: &DaemonLayout,
    runner: &dyn Runner,
    lockfile: &Path,
    cache_py: &Path,
    allow_index: bool,
) -> anyhow::Result<PathBuf> {
    if integrity_ok(layout, lockfile) {
        return Ok(venv_dir(layout));
    }
    build(layout, runner, lockfile, cache_py, allow_index)?;
    Ok(venv_dir(layout))
}

pub fn python3_present(runner: &dyn Runner) -> bool {
    runner
        .run("python3", &["--version"])
        .map(|o| o.ok())
        .unwrap_or(false)
}
