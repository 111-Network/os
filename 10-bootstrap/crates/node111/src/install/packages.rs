//! Step 5 — packages (spec §Install flow Linux, online guaranteed): INSTALL the
//! transport prerequisites per distro family AND download the resolved closure
//! into `cache/packages/` (root-owned, offline reinstall assets) + the rns pip
//! closure into `cache/packages/python/`. Every cached artifact is hash-recorded
//! in the receipt. The Meshtastic presence reader's closure (Layer 5 §14) is not
//! fetched here: its lock rides in the release payload, so the stage step
//! downloads it into `cache/packages/presence/` with the build venv's own pip
//! (`stage::presence_venv`).

use super::context::InstallContext;
use super::receipt::Receipt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Debian,
    Fedora,
    Arch,
    Unknown,
}

impl Family {
    pub fn of(distro: &str) -> Self {
        match distro {
            "debian" => Family::Debian,
            "fedora" => Family::Fedora,
            "arch" => Family::Arch,
            _ => Family::Unknown,
        }
    }
}

/// The pinned package set (spec step 5). python3+venv makes Bundle B real on
/// fresh images; iw feeds AP-mode probing; ca-certificates for TLS.
pub fn packages_for(family: Family) -> &'static [&'static str] {
    match family {
        Family::Debian => &["python3", "python3-venv", "iw", "ca-certificates"],
        Family::Fedora => &["python3", "iw", "ca-certificates"],
        Family::Arch => &["python", "iw", "ca-certificates"],
        Family::Unknown => &[],
    }
}

fn must(ctx: &InstallContext, program: &str, args: &[&str]) -> anyhow::Result<()> {
    let out = ctx.runner.run(program, args)?;
    anyhow::ensure!(
        out.ok(),
        "package step failed: {program} {} exited {}: {}",
        args.join(" "),
        out.status,
        out.stderr.trim()
    );
    Ok(())
}

/// Install packages + cache the closure. Degrades (receipt branch) rather than
/// failing when the distro is unknown or the cache download fails AFTER a
/// successful install (the node still runs; offline reinstall is degraded).
pub fn run(ctx: &InstallContext, family: Family, receipt: &mut Receipt) -> anyhow::Result<()> {
    let pkgs = packages_for(family);
    if pkgs.is_empty() {
        receipt
            .degrade_branches
            .push("unknown distro family: no packages installed; transport stub only".into());
        return Ok(());
    }
    let cache = ctx.layout.root.join("cache/packages");
    std::fs::create_dir_all(&cache)?;
    let cache_s = cache.to_string_lossy().into_owned();

    match family {
        Family::Debian => {
            // best-effort update (spec: apt-get update first, failure tolerated)
            let _ = ctx.runner.run("apt-get", &["update"]);
            let mut args: Vec<&str> = vec!["install", "-y"];
            args.extend_from_slice(pkgs);
            must(ctx, "apt-get", &args)?;

            let cache_arg = format!("Dir::Cache::archives={cache_s}");
            let mut dl_args: Vec<&str> = vec![
                "install",
                "-y",
                "--download-only",
                "--reinstall",
                "-o",
                &cache_arg,
            ];
            dl_args.extend_from_slice(pkgs);
            let dl = ctx.runner.run("apt-get", &dl_args)?;
            if !dl.ok() {
                receipt
                    .degrade_branches
                    .push("apt closure download failed; offline package cache incomplete".into());
            }
        }
        Family::Fedora => {
            let mut args: Vec<&str> = vec!["install", "-y"];
            args.extend_from_slice(pkgs);
            must(ctx, "dnf5", &args)?;

            let mut dl_args: Vec<&str> = vec!["download", "--resolve", "--destdir", &cache_s];
            dl_args.extend_from_slice(pkgs);
            let dl = ctx.runner.run("dnf5", &dl_args)?;
            if !dl.ok() {
                receipt
                    .degrade_branches
                    .push("dnf5 closure download failed; offline package cache incomplete".into());
            }
            let _ = ctx.runner.run("createrepo_c", &[&cache_s]);
        }
        Family::Arch => {
            let mut args: Vec<&str> = vec!["-S", "--noconfirm", "--needed"];
            args.extend_from_slice(pkgs);
            must(ctx, "pacman", &args)?;

            let mut dl_args: Vec<&str> = vec!["-Sw", "--noconfirm", "--cachedir", &cache_s];
            dl_args.extend_from_slice(pkgs);
            let dl = ctx.runner.run("pacman", &dl_args)?;
            if !dl.ok() {
                receipt.degrade_branches.push(
                    "pacman closure download failed; offline package cache incomplete".into(),
                );
            }
        }
        Family::Unknown => unreachable!("unknown family returns early"),
    }
    Ok(())
}

/// Download the rns pip closure into `cache/packages/python/` using the
/// hash-locked lockfile (spec §Transport). Degrades when python3/pip absent.
pub fn pip_closure(
    ctx: &InstallContext,
    lockfile: &Path,
    receipt: &mut Receipt,
) -> anyhow::Result<()> {
    let py = ctx.runner.run("python3", &["-m", "pip", "--version"])?;
    if !py.ok() {
        receipt.degrade_branches.push(
            "python3/pip absent after package step: rns closure not prefetched; transport stub"
                .into(),
        );
        return Ok(());
    }
    let dest = ctx.layout.root.join("cache/packages/python");
    std::fs::create_dir_all(&dest)?;
    let dest_s = dest.to_string_lossy().into_owned();
    let lock_s = lockfile.to_string_lossy().into_owned();
    let out = ctx.runner.run(
        "python3",
        &[
            "-m",
            "pip",
            "download",
            "--require-hashes",
            "-r",
            &lock_s,
            "-d",
            &dest_s,
        ],
    )?;
    anyhow::ensure!(
        out.ok(),
        "pip closure download failed (rns lockfile {}): {}",
        lock_s,
        out.stderr.trim()
    );
    Ok(())
}

/// Hash-record every cached artifact into the receipt (relpath → sha256).
pub fn record_cache_hashes(cache_root: &Path, receipt: &mut Receipt) -> anyhow::Result<()> {
    let mut stack: Vec<PathBuf> = vec![cache_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if !dir.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                let rel = path
                    .strip_prefix(cache_root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let sha = crate::util::hash::sha256_file(&path)?;
                receipt.cache_hashes.insert(rel, sha);
            }
        }
    }
    Ok(())
}
