//! Headless channel (spec phone flow step 4): run-as writes the boot script +
//! payload + prefetched .debs (on-device sha256 against the manifest before
//! `dpkg -i`). NEVER writes allow-external-apps=true. E2 fallback when run-as
//! fails: chunked ≤4KB `input text` stub with echoed per-chunk sha256, screen
//! kept awake, stub pulls the real payload over `adb reverse` from the host.

use super::adb::Adb;
use crate::manifest::PinnedArtifact;
use std::path::{Path, PathBuf};

/// Termux app-private staging root (on-device, via run-as).
const HOME_REL: &str = "/data/data/com.termux/files/home";
const PREFIX: &str = "/data/data/com.termux/files/usr";

/// Launch TermuxActivity and wait for bootstrap-unpack completion: working
/// `dpkg -l` + `$PREFIX/etc` present (content probe, never existence-of-files).
pub fn wait_bootstrap(adb: &Adb<'_>, timeout_secs: u64) -> anyhow::Result<()> {
    let _ = adb.shell("am start -n com.termux/.app.TermuxActivity");
    let start = std::time::Instant::now();
    loop {
        let out = adb.shell(&format!(
            "run-as com.termux sh -c 'dpkg -l >/dev/null 2>&1 && test -d {PREFIX}/etc && echo BOOTSTRAP_OK'"
        ))?;
        if out.ok() && out.stdout.contains("BOOTSTRAP_OK") {
            return Ok(());
        }
        if start.elapsed().as_secs() > timeout_secs {
            anyhow::bail!(
                "Termux bootstrap unpack did not complete within {timeout_secs}s — \
                 open Termux once on the phone and re-run (E2 fallback exists for broken run-as)"
            );
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
}

/// run-as exec helper (single-quoted shell command).
fn run_as(adb: &Adb<'_>, cmd: &str) -> anyhow::Result<crate::util::run::Output> {
    adb.shell(&format!("run-as com.termux sh -c '{cmd}'"))
}

/// run-as returning stdout on success (command text is trusted: built by this
/// crate, never from device input).
pub fn run_as_output(adb: &Adb<'_>, cmd: &str) -> anyhow::Result<String> {
    let out = run_as(adb, cmd)?;
    anyhow::ensure!(out.ok(), "run-as command failed: {}", out.stderr.trim());
    Ok(out.stdout)
}

/// Does the run-as channel work at all?
pub fn run_as_works(adb: &Adb<'_>) -> bool {
    run_as(adb, "echo RA_OK")
        .map(|o| o.ok() && o.stdout.contains("RA_OK"))
        .unwrap_or(false)
}

/// Push a host file into the app-private tree via /data/local/tmp relay +
/// run-as copy, with on-device sha256 verification.
pub fn push_verified(
    adb: &Adb<'_>,
    local: &Path,
    remote: &str,
    want_sha256: &str,
) -> anyhow::Result<()> {
    let stage = "/data/local/tmp/111-stage";
    let _ = adb.shell(&format!("mkdir -p {stage}"));
    let fname = local
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("no file name: {}", local.display()))?
        .to_string_lossy()
        .into_owned();
    let staged = format!("{stage}/{fname}");
    let out = adb.push(&local.to_string_lossy(), &staged)?;
    anyhow::ensure!(
        out.ok(),
        "adb push {} failed: {}",
        local.display(),
        out.stderr.trim()
    );
    let verify = run_as(
        adb,
        &format!(
            "sha256sum {staged} | cut -d' ' -f1 | grep -qx {want_sha256} && mkdir -p $(dirname {remote}) && cp {staged} {remote} && echo PUSH_OK"
        ),
    )?;
    anyhow::ensure!(
        verify.ok() && verify.stdout.contains("PUSH_OK"),
        "on-device sha256 verify+copy failed for {fname}: {}",
        verify.stdout.trim()
    );
    let _ = adb.shell(&format!("rm -f {staged}"));
    Ok(())
}

/// Write any small file into the app-private tree via run-as stdin, with
/// on-device hash verification (never /sdcard).
pub fn install_file(
    adb: &Adb<'_>,
    content: &[u8],
    remote: &str,
    executable: bool,
) -> anyhow::Result<()> {
    let chmod = if executable {
        format!("&& chmod 0755 {remote}")
    } else {
        String::new()
    };
    let out = adb.shell_stdin(
        &format!("run-as com.termux sh -c 'mkdir -p $(dirname {remote}) && cat > {remote} {chmod} && echo WRITE_OK'"),
        content,
    )?;
    anyhow::ensure!(
        out.ok() && out.stdout.contains("WRITE_OK"),
        "file write to {remote} failed: {}",
        out.stderr.trim()
    );
    let want = crate::util::hash::sha256_bytes(content);
    let verify = run_as(
        adb,
        &format!("sha256sum {remote} | cut -d' ' -f1 | grep -qx {want} && echo HASH_OK"),
    )?;
    anyhow::ensure!(
        verify.ok() && verify.stdout.contains("HASH_OK"),
        "on-device hash mismatch for {remote}"
    );
    Ok(())
}

/// Write the Termux:Boot script (via run-as stdin — never /sdcard).
pub fn install_boot_script(adb: &Adb<'_>, content: &[u8]) -> anyhow::Result<()> {
    let out = adb.shell_stdin(
        &format!("run-as com.termux sh -c 'mkdir -p {HOME_REL}/.termux/boot && cat > {HOME_REL}/.termux/boot/00-111-node.sh && chmod 0755 {HOME_REL}/.termux/boot/00-111-node.sh && echo BOOT_OK'"),
        content,
    )?;
    anyhow::ensure!(
        out.ok() && out.stdout.contains("BOOT_OK"),
        "boot script write failed: {}",
        out.stderr.trim()
    );
    // content probe: the script on device must hash to our bytes
    let want = crate::util::hash::sha256_bytes(content);
    let verify = run_as(
        adb,
        &format!("sha256sum {HOME_REL}/.termux/boot/00-111-node.sh | cut -d' ' -f1 | grep -qx {want} && echo HASH_OK"),
    )?;
    anyhow::ensure!(
        verify.ok() && verify.stdout.contains("HASH_OK"),
        "boot script hash mismatch on device"
    );
    Ok(())
}

/// Install the .deb closure: push each with sha256 verify, then `dpkg -i`
/// (per-deb sha256 from the signed manifest).
pub fn install_debs(adb: &Adb<'_>, cache: &Path, debs: &[PinnedArtifact]) -> anyhow::Result<()> {
    for deb in debs {
        let local = cache.join(format!("{}.deb", deb.name));
        anyhow::ensure!(
            local.exists(),
            "deb missing at {} (prefetch)",
            local.display()
        );
        let sha = crate::util::hash::sha256_file(&local)?;
        anyhow::ensure!(
            sha == deb.sha256,
            "{}.deb sha256 mismatch vs manifest",
            deb.name
        );
        let remote = format!("{PREFIX}/var/lib/111/debs/{}.deb", deb.name);
        push_verified(adb, &local, &remote, &deb.sha256)?;
        let out = run_as(adb, &format!("dpkg -i {remote} && echo DPKG_OK"))?;
        anyhow::ensure!(
            out.ok() && out.stdout.contains("DPKG_OK"),
            "dpkg -i {} failed: {}",
            deb.name,
            out.stderr.trim()
        );
    }
    Ok(())
}

/// Guard: we never set allow-external-apps (spec: no v1 consumer; grants
/// RUN_COMMAND to any on-device app). Assert absence after provisioning.
pub fn assert_no_external_apps(adb: &Adb<'_>) -> anyhow::Result<()> {
    let out = run_as(
        adb,
        &format!("grep -q 'allow-external-apps *= *true' {HOME_REL}/.termux/termux.properties 2>/dev/null && echo VIOLATION || echo CLEAN"),
    )?;
    anyhow::ensure!(
        out.ok() && out.stdout.contains("CLEAN"),
        "allow-external-apps=true found — this is never written by the provisioner"
    );
    Ok(())
}

// ---------- E2 fallback (bounded, spec step 4) ----------

/// Type the E2 stub in base64 chunks via `input text`, reading each chunk's
/// echoed sha256 back through the UI (uiautomator dump). Screen kept awake.
pub fn e2e_inject_stub(adb: &Adb<'_>, stub_resolved: &[u8]) -> anyhow::Result<()> {
    const CHUNK: usize = 3800;
    use base64ct_alike::b64;
    let b64 = b64(stub_resolved);
    let _ = adb.shell("svc power stayon true");
    let target = "/data/data/com.termux/files/usr/tmp/e2e.b64";
    let mut offset = 0;
    while offset < b64.len() {
        let end = (offset + CHUNK).min(b64.len());
        let chunk = &b64[offset..end];
        let want = crate::util::hash::sha256_bytes(chunk.as_bytes());
        let line = format!(
            "printf '%s' '{chunk}' >> {target}; sha256sum {target} | tail -c 8; echo CHUNK_END"
        );
        let out = adb.shell(&format!("input text '{line}'"))?;
        anyhow::ensure!(
            out.ok(),
            "input text failed at chunk {offset}: {}",
            out.stderr.trim()
        );
        let _ = adb.shell("input keyevent 66"); // ENTER
                                                // read back via uiautomator dump: the echoed per-chunk hash tail
        let dump =
            adb.shell("uiautomator dump /sdcard/ui.xml >/dev/null 2>&1; cat /sdcard/ui.xml")?;
        let tail8 = &want[want.len() - 8..];
        anyhow::ensure!(
            dump.ok() && dump.stdout.contains(tail8),
            "E2 chunk {offset} echoed hash mismatch (want tail {tail8}) — typed content corrupted"
        );
        offset = end;
    }
    // decode + execute: stub pulls the real payload over adb reverse
    let out = adb.shell(&format!(
        "run-as com.termux sh -c 'base64 -d {target} > {PREFIX}/tmp/e2e.sh && sh {PREFIX}/tmp/e2e.sh'"
    ))?;
    anyhow::ensure!(out.ok(), "E2 stub exec failed: {}", out.stderr.trim());
    Ok(())
}

/// Open the adb reverse channel (phone 127.0.0.1:PORT → host PORT).
pub fn e2e_reverse(adb: &Adb<'_>, port: u16) -> anyhow::Result<()> {
    let spec = format!("tcp:{port} tcp:{port}");
    let out = adb.reverse(&spec)?;
    anyhow::ensure!(out.ok(), "adb reverse {spec} failed: {}", out.stderr.trim());
    Ok(())
}

pub fn home_rel() -> &'static str {
    HOME_REL
}
pub fn prefix() -> &'static str {
    PREFIX
}

pub fn stage_paths() -> (PathBuf, PathBuf) {
    (PathBuf::from(HOME_REL), PathBuf::from(PREFIX))
}

// Minimal base64 encoder (no new deps — the stub chunker needs it).
mod base64ct_alike {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    pub fn b64(data: &[u8]) -> String {
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = *chunk.get(1).unwrap_or(&0) as u32;
            let b2 = *chunk.get(2).unwrap_or(&0) as u32;
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(T[(n >> 18) as usize & 63] as char);
            out.push(T[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 {
                T[(n >> 6) as usize & 63] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                T[n as usize & 63] as char
            } else {
                '='
            });
        }
        out
    }
}
