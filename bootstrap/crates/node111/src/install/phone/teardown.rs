//! Teardown — `111 uninstall --phone` (spec phone flow step 9): works over
//! sshd post-lockdown or adb pre-lockdown: `pm uninstall` ×3, deviceidle
//! whitelist removal, appops resets, node tree removal.

use super::adb::Adb;
use crate::util::run::Runner;
use std::path::Path;

const PKGS: &[&str] = &["com.termux", "com.termux.api", "com.termux.boot"];

/// Teardown over adb (pre-lockdown path).
pub fn via_adb(adb: &Adb<'_>) -> anyhow::Result<()> {
    // stop the node first
    let _ = adb.shell("run-as com.termux sh -c 'pkill -f 111node; pkill -f \"111 daemon\"'");
    // node tree + boot script + service
    let _ = adb.shell(
        "run-as com.termux sh -c 'rm -rf $PREFIX/var/lib/111 $PREFIX/var/service/111node $HOME/.termux/boot/00-111-node.sh'",
    );
    // whitelist + appops resets (best-effort)
    for pkg in PKGS {
        let _ = adb.shell(&format!("dumpsys deviceidle whitelist -{pkg}"));
    }
    let _ = adb.shell("appops set com.termux RUN_IN_BACKGROUND default");
    let _ = adb.shell("appops set com.termux RUN_ANY_IN_BACKGROUND default");
    let _ = adb.shell("appops set com.termux SYSTEM_ALERT_WINDOW default");
    // APKs
    for pkg in PKGS {
        let out = adb.cmd(&["uninstall", pkg])?;
        let body = format!("{}{}", out.stdout, out.stderr);
        anyhow::ensure!(
            out.ok()
                && (body.contains("Success") || !body.contains("DELETE_FAILED_INTERNAL_ERROR")),
            "pm uninstall {pkg} failed: {}",
            body.trim()
        );
    }
    let _ = adb.cmd(&["forward", "--remove-all"]);
    let _ = adb.cmd(&["reverse", "--remove-all"]);
    Ok(())
}

/// Teardown over sshd (post-lockdown path): removes the node tree and
/// supervision; APK removal still needs adb (documented).
pub fn via_sshd(runner: &dyn Runner, host_key: &Path, target: &str) -> anyhow::Result<()> {
    let key_s = host_key.to_string_lossy().into_owned();
    let out = runner.run(
        "ssh",
        &[
            "-p", "8022",
            "-i", &key_s,
            "-o", "StrictHostKeyChecking=no",
            "-o", "UserKnownHostsFile=/dev/null",
            "-o", "ConnectTimeout=10",
            target,
            "pkill -f '111 daemon'; rm -rf $PREFIX/var/lib/111 $PREFIX/var/service/111node $HOME/.termux/boot/00-111-node.sh && echo TEARDOWN_OK",
        ],
    )?;
    anyhow::ensure!(
        out.ok() && out.stdout.contains("TEARDOWN_OK"),
        "sshd teardown failed: {}",
        out.stderr.trim()
    );
    Ok(())
}

/// Entry point for `111 uninstall --phone [--serial S]`: adb when a device is
/// present, sshd otherwise (post-lockdown maintenance channel).
pub fn run(serial: Option<String>) -> anyhow::Result<()> {
    let runner = crate::util::run::SystemRunner;
    // adb path when a device is attached
    if runner
        .run("adb", &["devices"])
        .map(|o| o.ok())
        .unwrap_or(false)
    {
        let adb = Adb::new(&runner, serial.clone(), "adb");
        if !adb.devices()?.is_empty() {
            via_adb(&adb)?;
            println!(
                "phone teardown complete (adb): packages removed, hardening reset, node tree wiped"
            );
            return Ok(());
        }
    }
    // sshd fallback (post-lockdown)
    let key = super::start::host_keys_dir().join("id_ed25519");
    anyhow::ensure!(
        key.exists(),
        "no adb device and no maintenance key — cannot reach the phone"
    );
    let target = serial.unwrap_or_else(|| "127.0.0.1".into());
    via_sshd(&runner, &key, &target)?;
    println!(
        "phone teardown complete (sshd): node tree wiped; remove Termux manually for full teardown"
    );
    Ok(())
}
