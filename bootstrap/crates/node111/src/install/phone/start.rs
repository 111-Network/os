//! Start-now (spec phone flow step 6): on-device keygen (host NEVER sees phone
//! key material; sshd host keys generated on-device, only authorized_keys is
//! pushed) → detached start via the Termux:Boot script path (setsid/nohup via
//! runsvdir; unplug survival) → sshd key-only 8022 as the maintenance channel.

use super::adb::Adb;
use super::runas;
use crate::util::run::Runner;
use std::path::{Path, PathBuf};

/// Host maintenance keypair (~/.local/share/111/keys, 0600). Generated once,
/// reused; the private key never leaves the host.
pub fn ensure_host_keypair(runner: &dyn Runner, keys_dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(keys_dir)?;
    let key = keys_dir.join("id_ed25519");
    if key.exists() {
        return Ok(key);
    }
    let key_s = key.to_string_lossy().into_owned();
    let out = runner.run(
        "ssh-keygen",
        &[
            "-t",
            "ed25519",
            "-N",
            "",
            "-f",
            &key_s,
            "-C",
            "111-phone-maintenance",
        ],
    )?;
    anyhow::ensure!(out.ok(), "ssh-keygen failed: {}", out.stderr.trim());
    let _ = runner.run("chmod", &["0600", &key_s]);
    Ok(key)
}

/// On-device keygen: the payload binary generates the phone identity; the
/// secret NEVER leaves the phone. No-op when an identity already exists.
pub fn on_device_keygen(adb: &Adb<'_>) -> anyhow::Result<String> {
    let npub = runas::run_as_output(
        adb,
        "NODE111_ROOT=$PREFIX/var/lib/111 $PREFIX/var/lib/111/current/bin/111 identity generate 2>/dev/null || true; cat $PREFIX/var/lib/111/identity/npub 2>/dev/null",
    )?;
    let npub = npub.trim().to_string();
    anyhow::ensure!(
        npub.starts_with("npub1"),
        "on-device keygen produced no npub: {npub:?}"
    );
    Ok(npub)
}

/// sshd host keys ON-DEVICE + push only the user's authorized_keys.
pub fn setup_sshd(adb: &Adb<'_>, host_pubkey: &str) -> anyhow::Result<()> {
    // host keys (on-device, if absent)
    let out = runas::run_as_output(
        adb,
        "test -f $PREFIX/etc/ssh/ssh_host_ed25519_key || ssh-keygen -t ed25519 -N '' -f $PREFIX/etc/ssh/ssh_host_ed25519_key; echo HOSTKEY_OK",
    )?;
    anyhow::ensure!(
        out.contains("HOSTKEY_OK"),
        "on-device sshd host keygen failed"
    );
    // authorized_keys (push only the public half)
    let cmd = format!(
        "mkdir -p $HOME/.ssh && echo '{host_pubkey}' > $HOME/.ssh/authorized_keys && chmod 0600 $HOME/.ssh/authorized_keys && echo AUTHKEY_OK"
    );
    let out = runas::run_as_output(adb, &cmd)?;
    anyhow::ensure!(out.contains("AUTHKEY_OK"), "authorized_keys push failed");
    // sshd config: key-only, port 8022
    let out = runas::run_as_output(
        adb,
        "printf '%s\\n' 'Port 8022' 'PasswordAuthentication no' 'PubkeyAuthentication yes' > $PREFIX/etc/ssh/sshd_config && echo CFG_OK",
    )?;
    anyhow::ensure!(out.contains("CFG_OK"), "sshd_config write failed");
    // start sshd (idempotent)
    let _ = runas::run_as_output(
        adb,
        "pgrep -f sshd >/dev/null || $PREFIX/bin/sshd; echo SSHD_OK",
    );
    Ok(())
}

/// Detached start via the boot script (wake-lock → profile → runsvdir → sv).
pub fn start_node(adb: &Adb<'_>) -> anyhow::Result<()> {
    let out = runas::run_as_output(
        adb,
        "setsid nohup sh $HOME/.termux/boot/00-111-node.sh </dev/null >>$PREFIX/var/log/111-boot-manual.log 2>&1 & echo START_OK",
    )?;
    anyhow::ensure!(out.contains("START_OK"), "detached start failed: {out}");
    Ok(())
}

/// Verify the daemon answers on-device via adb forward (NO LAN poll, spec
/// step 7): `adb forward tcp:11180` + host-side GET on 127.0.0.1.
pub fn verify_health(
    adb: &Adb<'_>,
    fetcher: &crate::util::http::Fetcher,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    let out = adb.forward("tcp:11180 tcp:11180")?;
    anyhow::ensure!(out.ok(), "adb forward failed: {}", out.stderr.trim());
    let start = std::time::Instant::now();
    loop {
        if let Ok(body) = fetcher.get("http://127.0.0.1:11180/health") {
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
            if v["schema"] == 1 {
                return Ok(());
            }
        }
        if start.elapsed().as_secs() > timeout_secs {
            anyhow::bail!("on-device /health never answered within {timeout_secs}s");
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
}

/// sshd login verification: via forward first (scriptable), then the fresh-LAN
/// ceremony (spec step 7 — before any lockdown).
pub fn verify_sshd_forward(
    adb: &Adb<'_>,
    runner: &dyn Runner,
    host_key: &Path,
) -> anyhow::Result<()> {
    let _ = adb.forward("tcp:8022 tcp:8022")?;
    let key_s = host_key.to_string_lossy().into_owned();
    let out = runner.run(
        "ssh",
        &[
            "-p",
            "8022",
            "-i",
            &key_s,
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-o",
            "ConnectTimeout=10",
            "127.0.0.1",
            "echo SSH_OK",
        ],
    )?;
    anyhow::ensure!(
        out.ok() && out.stdout.contains("SSH_OK"),
        "sshd login via forward failed: {}",
        out.stderr.trim()
    );
    Ok(())
}

pub fn host_keys_dir() -> PathBuf {
    crate::util::paths::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local/share/111/keys")
}
