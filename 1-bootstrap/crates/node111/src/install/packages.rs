//! Step 5 — packages (spec §Install flow Linux, online guaranteed): INSTALL the
//! transport prerequisites per distro family AND download the resolved closure
//! into `cache/packages/` (root-owned, offline reinstall assets) + the rns pip
//! closure into `cache/packages/python/` with a throwaway venv's own pip
//! ([`pip_closure`]: the host needs no `python3 -m pip`). Every cached artifact
//! is hash-recorded in the receipt. The Meshtastic presence reader's closure (Layer 5 §14) is not
//! fetched here: its lock rides in the release payload, so the stage step
//! downloads it into `cache/packages/presence/` with the build venv's own pip
//! (`stage::presence_venv`).
//!
//! **Third-party notices and the component cache (Layer 6 §2.1 rule 3, §15;
//! D26 #1 #5).** Before any third-party download the installer prints one
//! notice line per program (name, licence, where to read it) and records it
//! in the receipt ([`announce`]): the rns closure gets its line before
//! [`pip_closure`] ([`rns_notice`]); every optional component declared in the
//! binary's embedded `assets/components/` gets its line, and then its payload
//! is pre-downloaded into `cache/packages/<id>/` ([`cache_components`]): pip
//! closures with a throwaway venv's own pip (the [`pip_closure`] shape),
//! binaries through the fetcher with size and SHA-256 checked. A failed
//! download is a degrade branch, never fatal. Nothing is installed or started
//! here: that waits for a human to enable the component. The release's one pip
//! closure is the LXMF helper's (Layer 6 §6): declaration `lxmf.toml`, lock
//! `lxmf.lock.txt` beside it (byte-identical to the helper's
//! `lxmf-sidecar-lock.txt`), written as `<tool venv>/lxmf-lock.txt` and
//! downloaded into `cache/packages/lxmf/`, where enable builds `venvs/lxmf`
//! from it offline.

use super::context::InstallContext;
use super::receipt::{InstallNotice, Receipt};
use crate::daemon::components::manifest::{url_file_name, Component, Payload};
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
/// fresh images; iw feeds AP-mode probing; ca-certificates for TLS. No pip
/// package: every pip that runs is a venv's own ([`pip_closure`]) — Debian's
/// `python3-pip` would also pull `build-essential` and `python3-dev` in
/// (Recommends), and Fedora/Arch ship pip apart from python as well.
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

/// The throwaway venv whose pip downloads the rns closure ([`pip_closure`]),
/// inside the root-owned `cache/packages/` (on the node's own filesystem: a
/// `noexec` `/tmp` cannot break it); removed again before step 9 hashes
/// `cache/`.
pub const RNS_PIP_VENV: &str = "cache/packages/.rns-pip";

/// Download the rns pip closure into `cache/packages/python/` using the
/// hash-locked lockfile (spec §Transport) with the pip of a throwaway venv
/// ([`RNS_PIP_VENV`]), never the host's `python3 -m pip`: a fresh
/// Debian/Ubuntu has `python3-venv` but no pip module ("No module named
/// pip"), and without this closure the daemon's offline rns venv build
/// (`daemon::transport::venv::build`, `--no-index`) fails — a node with no
/// transport. The pip that downloads is then the one every venv bootstraps
/// (`ensurepip`), the same that installs the closure offline later.
/// Degrades when python3 or its `venv` module is absent (no venv, no
/// transport); a download that runs and fails is an install error.
pub fn pip_closure(
    ctx: &InstallContext,
    lockfile: &Path,
    receipt: &mut Receipt,
) -> anyhow::Result<()> {
    let tool = ctx.layout.root.join(RNS_PIP_VENV);
    remove_tool_venv(&tool)?; // a run killed mid-download left it behind
    let tool_s = tool.to_string_lossy().into_owned();
    let venv = ctx.runner.run("python3", &["-m", "venv", &tool_s]);
    if !venv.as_ref().is_ok_and(|o| o.ok()) {
        let _ = remove_tool_venv(&tool);
        receipt.degrade_branches.push(
            "python3/venv absent after package step: rns closure not prefetched; transport stub"
                .into(),
        );
        return Ok(());
    }
    let dest = ctx.layout.root.join("cache/packages/python");
    let downloaded = (|| -> anyhow::Result<()> {
        std::fs::create_dir_all(&dest)?;
        let dest_s = dest.to_string_lossy().into_owned();
        let lock_s = lockfile.to_string_lossy().into_owned();
        let pip = format!("{tool_s}/bin/pip");
        let out = ctx.runner.run(
            &pip,
            &["download", "--require-hashes", "-r", &lock_s, "-d", &dest_s],
        )?;
        anyhow::ensure!(
            out.ok(),
            "pip closure download failed (rns lockfile {}): {}",
            lock_s,
            out.stderr.trim()
        );
        Ok(())
    })();
    if let Err(e) = remove_tool_venv(&tool) {
        eprintln!("111 install: {e:#}");
    }
    downloaded
}

/// Remove [`RNS_PIP_VENV`] (a link is removed itself, never followed).
fn remove_tool_venv(tool: &Path) -> anyhow::Result<()> {
    let removed = match std::fs::symlink_metadata(tool) {
        Ok(meta) if meta.file_type().is_dir() => std::fs::remove_dir_all(tool),
        Ok(_) => std::fs::remove_file(tool),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => Err(e),
    };
    removed.map_err(|e| anyhow::anyhow!("cannot remove {}: {e}", tool.display()))
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

/// Where the installer gets a pinned binary payload (the HTTPS [`Fetcher`];
/// tests pass a fake).
///
/// [`Fetcher`]: crate::util::http::Fetcher
pub trait PayloadFetcher {
    fn fetch(&self, url: &str) -> anyhow::Result<Vec<u8>>;
}

impl PayloadFetcher for crate::util::http::Fetcher {
    fn fetch(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        self.get(url)
    }
}

/// One third-party notice line: the program, its licence, where to read it,
/// and the exact text (whose SHA-256 the receipt records).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub program: String,
    pub license: String,
    pub url: String,
    pub text: String,
}

impl Notice {
    /// The notice a component's declaration carries.
    pub fn of(c: &Component) -> Self {
        Notice {
            program: c.title.clone(),
            license: c.license.spdx.clone(),
            url: c.license.read_at.clone(),
            text: c.license.notice.clone(),
        }
    }

    /// The one line the installer prints.
    pub fn line(&self) -> String {
        format!(
            "111 install: notice: {} ({}; read it at {}): {}",
            self.program, self.license, self.url, self.text
        )
    }

    pub fn record(&self) -> InstallNotice {
        InstallNotice {
            program: self.program.clone(),
            license: self.license.clone(),
            url: self.url.clone(),
            hash: crate::util::hash::sha256_bytes(self.text.as_bytes()),
        }
    }
}

/// The rns closure's notice (the Layer 1 venv gets the same one line, D26 #5).
pub fn rns_notice() -> Notice {
    Notice {
        program: "Reticulum (rns)".into(),
        license: "LicenseRef-Reticulum".into(),
        url: "https://github.com/markqvist/Reticulum/blob/master/LICENSE".into(),
        text: "Reticulum (rns) is the work of the Reticulum project (reticulum.network) and is \
               licensed under the Reticulum License; read it at \
               https://github.com/markqvist/Reticulum/blob/master/LICENSE. It is downloaded \
               from PyPI, unmodified, and runs as its own program. You install it: 111 is not \
               affiliated with the Reticulum project and is not its distributor."
            .into(),
    }
}

/// Print `n`'s one line to `out` and record it in the receipt — before the
/// download it announces. A notice already recorded is not repeated.
pub fn announce(n: &Notice, receipt: &mut Receipt, out: &mut dyn std::io::Write) {
    let rec = n.record();
    if receipt.notices.contains(&rec) {
        return;
    }
    let _ = writeln!(out, "{}", n.line());
    receipt.notices.push(rec);
}

/// The throwaway venv whose pip downloads component closures
/// ([`cache_components`]), removed again before step 9 hashes `cache/`.
pub const COMPONENTS_PIP_VENV: &str = "cache/packages/.components-pip";

/// Install-time cache intake (module doc): every declared component's notice
/// first (printed and recorded, so the record exists before any download),
/// then its payload into `cache/packages/<id>/`. A component whose payload
/// the release does not carry (no lock yet) is skipped quietly; a failed
/// download, a missing python3 or a payload failing its pin is a degrade
/// branch. Never fails the install.
pub fn cache_components(
    ctx: &InstallContext,
    components: &[Component],
    fetcher: &dyn PayloadFetcher,
    receipt: &mut Receipt,
    out: &mut dyn std::io::Write,
) {
    for c in components {
        announce(&Notice::of(c), receipt, out);
    }
    let pip: Vec<(&Component, &[u8])> = components
        .iter()
        .filter_map(|c| match &c.payload {
            Payload::Pip {
                lock_bytes: Some(lock),
                ..
            } => Some((c, &lock[..])),
            _ => None,
        })
        .collect();
    if !pip.is_empty() {
        cache_pip_closures(ctx, &pip, receipt);
    }
    for c in components {
        if let Payload::Binary { url, sha256, size } = &c.payload {
            if let Err(e) = cache_binary(ctx, c, fetcher, url, sha256, *size) {
                receipt.degrade_branches.push(format!(
                    "component {}: payload not cached ({e:#}); enabling it needs the file in cache/packages/{}/",
                    c.id, c.id
                ));
            }
        }
    }
}

fn cache_pip_closures(ctx: &InstallContext, pip: &[(&Component, &[u8])], receipt: &mut Receipt) {
    let tool = ctx.layout.root.join(COMPONENTS_PIP_VENV);
    if let Err(e) = remove_tool_venv(&tool) {
        eprintln!("111 install: {e:#}");
    }
    let tool_s = tool.to_string_lossy().into_owned();
    let venv = ctx.runner.run("python3", &["-m", "venv", &tool_s]);
    if !venv.as_ref().is_ok_and(|o| o.ok()) {
        let _ = remove_tool_venv(&tool);
        for (c, _) in pip {
            receipt.degrade_branches.push(format!(
                "component {}: python3/venv absent, closure not cached",
                c.id
            ));
        }
        return;
    }
    let pip_bin = format!("{tool_s}/bin/pip");
    for (c, lock) in pip {
        let result = (|| -> anyhow::Result<()> {
            // the lock beside the tool venv (created by `venv`; made here too)
            std::fs::create_dir_all(&tool)?;
            let lock_path = tool.join(format!("{}-lock.txt", c.id));
            crate::util::fs::atomic_write(&lock_path, lock, 0o644)?;
            let dest = ctx.layout.root.join("cache/packages").join(&c.id);
            std::fs::create_dir_all(&dest)?;
            let dest_s = dest.to_string_lossy().into_owned();
            let lock_s = lock_path.to_string_lossy().into_owned();
            let out = ctx.runner.run(
                &pip_bin,
                &["download", "--require-hashes", "-r", &lock_s, "-d", &dest_s],
            )?;
            anyhow::ensure!(out.ok(), "pip download failed: {}", out.stderr.trim());
            Ok(())
        })();
        if let Err(e) = result {
            receipt
                .degrade_branches
                .push(format!("component {}: closure not cached ({e:#})", c.id));
        }
    }
    if let Err(e) = remove_tool_venv(&tool) {
        eprintln!("111 install: {e:#}");
    }
}

/// Fetch one pinned file into `cache/packages/<id>/<name>`: the size and the
/// SHA-256 must match before it lands (atomic write); an already cached
/// matching file is kept without a download.
fn cache_binary(
    ctx: &InstallContext,
    c: &Component,
    fetcher: &dyn PayloadFetcher,
    url: &str,
    sha256: &str,
    size: u64,
) -> anyhow::Result<()> {
    let name = url_file_name(url).ok_or_else(|| anyhow::anyhow!("no file name in {url}"))?;
    let dest_dir = ctx.layout.root.join("cache/packages").join(&c.id);
    let dest = dest_dir.join(&name);
    if std::fs::metadata(&dest).is_ok_and(|m| m.is_file() && m.len() == size)
        && crate::util::hash::sha256_file(&dest).ok().as_deref() == Some(sha256)
    {
        return Ok(());
    }
    let bytes = fetcher.fetch(url)?;
    anyhow::ensure!(
        bytes.len() as u64 == size,
        "{name}: {} bytes, the pin says {size}",
        bytes.len()
    );
    anyhow::ensure!(
        crate::util::hash::sha256_bytes(&bytes) == sha256,
        "{name}: sha256 does not match its pin"
    );
    std::fs::create_dir_all(&dest_dir)?;
    crate::util::fs::atomic_write(&dest, &bytes, 0o644)
}

#[cfg(test)]
mod notice_tests {
    use super::*;

    #[test]
    fn a_notice_is_one_line_and_recorded_once() {
        let mut r = Receipt::new("2026.10.19");
        let mut out = Vec::new();
        let n = rns_notice();
        announce(&n, &mut r, &mut out);
        announce(&n, &mut r, &mut out);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.contains("Reticulum (rns)") && text.contains("LicenseRef-Reticulum"));
        assert!(text.contains("https://github.com/markqvist/Reticulum/blob/master/LICENSE"));
        assert_eq!(r.notices.len(), 1);
        assert_eq!(
            r.notices[0].hash,
            crate::util::hash::sha256_bytes(n.text.as_bytes())
        );
    }

    #[test]
    fn a_component_notice_hash_is_the_enable_hash() {
        let c = crate::daemon::components::manifest::load_embedded()
            .into_iter()
            .find(|c| c.id == "lxmf")
            .unwrap();
        assert_eq!(Notice::of(&c).record().hash, c.notice_hash());
    }

    #[test]
    fn the_lxmf_closure_is_cached_under_its_id_with_its_notice_line() {
        let c = crate::daemon::components::manifest::load_embedded()
            .into_iter()
            .find(|c| c.id == "lxmf")
            .unwrap();
        // the intake names the tool-venv lock and the cache dir after the id
        assert!(matches!(
            &c.payload,
            Payload::Pip { lock, lock_bytes: Some(b), venv }
                if lock == "lxmf.lock.txt" && venv == "lxmf" && !b.is_empty()
        ));
        let line = Notice::of(&c).line();
        assert_eq!(line.lines().count(), 1);
        for must in [
            "Reticulum / LXMF helper",
            "LicenseRef-Reticulum",
            "https://github.com/markqvist/Reticulum/blob/master/LICENSE",
            "not their distributor",
        ] {
            assert!(line.contains(must), "{must}: {line}");
        }
    }
}
