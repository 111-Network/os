//! Verify (spec phone flow step 7): `adb forward tcp:11180` + on-device
//! `curl 127.0.0.1:11180/health` via run-as (assert loopback bind; NO LAN
//! poll). sshd verified via forward AND, before lockdown, on a fresh LAN
//! connection — automated when reachable, otherwise the labeled operator step.

use super::adb::Adb;

/// On-device health via run-as curl (the on-device check — distinct from the
/// host-side forward check in start.rs).
pub fn on_device_health(adb: &Adb<'_>) -> anyhow::Result<()> {
    let out =
        adb.shell("run-as com.termux sh -c 'curl -fsS -m 10 http://127.0.0.1:11180/health'")?;
    let ok_status = out.ok();
    let body = out.stdout;
    anyhow::ensure!(
        ok_status && body.contains("\"schema\":1"),
        "on-device /health failed: {} {}",
        out.stderr.trim(),
        body.trim()
    );
    Ok(())
}

/// The fresh-LAN sshd ceremony (spec step 7): automated attempt on the phone's
/// current Wi-Fi IP; on failure the LABELED manual step (operator moves the
/// phone to a fresh network and confirms on the terminal) — after which
/// lockdown may proceed. No terminal / end of input = NOT confirmed (error);
/// `yes` (`--yes`) confirms in advance.
pub fn fresh_lan_sshd_ceremony(
    adb: &Adb<'_>,
    host_key: &std::path::Path,
    runner: &dyn crate::util::run::Runner,
    yes: bool,
) -> anyhow::Result<()> {
    // phone's wlan0 IPv4
    let ip = adb
        .shell("ip -4 addr show wlan0 | grep -m1 inet | awk '{print $2}' | cut -d/ -f1")
        .map(|o| o.stdout.trim().to_string())
        .unwrap_or_default();
    if !ip.is_empty() {
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
                "ConnectTimeout=5",
                &ip,
                "echo SSH_OK",
            ],
        )?;
        if out.ok() && out.stdout.contains("SSH_OK") {
            eprintln!("111 phone: sshd verified on LAN ({ip}:8022) — lockdown may proceed");
            return Ok(());
        }
        eprintln!("111 phone: sshd not reachable at {ip}:8022 from this machine.");
    }
    // Labeled manual ceremony
    eprintln!(
        "
111 phone — REQUIRED before lockdown (fresh-LAN sshd check):
  1. move the phone onto a FRESH network (not the install LAN — e.g. hotspot)
  2. from a machine on that network, run:
       ssh -p 8022 -i {} <phone-ip>
  3. confirm a shell opens, then press ENTER here to apply the lockdown.
  (skipping risks locking yourself out of the maintenance channel)",
        host_key.display()
    );
    match super::prompt::answer(super::prompt::open_tty(), yes, "the fresh-LAN sshd check")? {
        Some(_) => eprintln!("111 phone: operator confirmed fresh-LAN sshd check"),
        None => eprintln!(
            "111 phone: WARNING: --yes — fresh-LAN sshd check NOT performed; confirmed by flag"
        ),
    }
    Ok(())
}
