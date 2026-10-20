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
/// secret NEVER leaves the phone. Idempotent (an existing identity is reported,
/// never overwritten). Returns the NodeId hex — the only thing the host sees.
pub fn on_device_keygen(adb: &Adb<'_>) -> anyhow::Result<String> {
    let out = runas::run_as_output(
        adb,
        "NODE111_ROOT=$PREFIX/var/lib/111 $PREFIX/var/lib/111/current/bin/111 identity generate",
    )?;
    let node_id = out
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
        .to_string();
    anyhow::ensure!(
        crate::identity::parse_node_id_hex(&node_id).is_some(),
        "on-device keygen produced no NodeId: {out:?}"
    );
    Ok(node_id)
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

/// Phone-local dashboard URL (the phone's own browser). Loopback is NOT trusted
/// on a phone node (any app can reach 127.0.0.1), so that browser pairs once.
pub const PHONE_DASHBOARD_URL: &str = "http://127.0.0.1:11180";

/// The daemon's pair file on the phone (`<root>/state/dashboard-pair.json`,
/// created at first daemon start). Absolute: run-as shells carry no `$PREFIX`.
pub const PHONE_PAIR_FILE: &str =
    "/data/data/com.termux/files/usr/var/lib/111/state/dashboard-pair.json";

/// The pair code in a `dashboard-pair.json` body, re-rendered from the
/// validated symbols as `XXXX-XXXX` (device output never reaches a shell
/// command unvalidated).
pub fn parse_pair_code(body: &str) -> Option<String> {
    let pf: crate::daemon::dashboard::api_v1::auth::PairFile =
        serde_json::from_str(body.trim()).ok()?;
    let sym = crate::daemon::dashboard::api_v1::auth::normalize_code(&pf.code)?;
    let s = String::from_utf8(sym.to_vec()).ok()?;
    Some(format!("{}-{}", &s[..4], &s[4..]))
}

/// The phone's current pair code (run-as `cat` of [`PHONE_PAIR_FILE`]); `None`
/// when unreadable or malformed.
pub fn read_pair_code(adb: &Adb<'_>) -> Option<String> {
    let body = runas::run_as_output(adb, &format!("cat {PHONE_PAIR_FILE}")).ok()?;
    parse_pair_code(&body)
}

/// The URL the phone's browser opens: the pair link when the code is known.
pub fn phone_open_url(code: Option<&str>) -> String {
    match code {
        Some(c) => format!("{PHONE_DASHBOARD_URL}/#/pair?code={c}"),
        None => PHONE_DASHBOARD_URL.to_string(),
    }
}

/// Best-effort (spec §12, Layer 4): open the dashboard in the phone's browser
/// via an Android VIEW intent — run after on-device verify, BEFORE the adb
/// lockdown. With the pair code it opens the pair link (the browser pairs in
/// one tap); without it, the plain URL plus the `111 dashboard` hint. Never
/// fails the provisioning; returns whether adb accepted it.
pub fn open_dashboard(adb: &Adb<'_>, code: Option<&str>) -> bool {
    let url = phone_open_url(code);
    // quoted: `?` would glob in the device shell (the code is validated, no quotes)
    let cmd = format!("am start -a android.intent.action.VIEW -d '{url}'");
    if code.is_none() {
        eprintln!(
            "111 phone: could not read the pair code from the phone — the browser will ask for it; \
             `111 dashboard` on the phone prints it"
        );
    }
    match adb.shell(&cmd) {
        Ok(out) if out.ok() && !out.stdout.contains("Error") => {
            eprintln!("111 phone: opened the dashboard in the phone's browser ({url})");
            true
        }
        _ => {
            eprintln!(
                "111 phone: could not open a browser on the phone — open {url} there yourself"
            );
            false
        }
    }
}

/// The identity-backup command for a provisioned phone (TTY-only ceremony over
/// the sshd maintenance channel; `-t` allocates the terminal it requires).
pub fn backup_hint(host_key: &Path) -> String {
    format!(
        "ssh -t -p 8022 -i {} <phone-ip> '. $PREFIX/var/lib/111/profile.env && 111 identity backup'",
        host_key.display()
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::MockRunner;
    use crate::util::run::Output;

    fn out(status: i32, stdout: &str) -> Output {
        Output {
            status,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    const AM_PAIR: &str = "adb -s S1 shell am start -a android.intent.action.VIEW -d 'http://127.0.0.1:11180/#/pair?code=7KQ2-XM9D'";
    const AM_PLAIN: &str =
        "adb -s S1 shell am start -a android.intent.action.VIEW -d 'http://127.0.0.1:11180'";
    const CAT: &str = "adb -s S1 shell run-as com.termux sh -c 'export PREFIX=/data/data/com.termux/files/usr HOME=/data/data/com.termux/files/home PATH=/data/data/com.termux/files/usr/bin:$PATH; cat /data/data/com.termux/files/usr/var/lib/111/state/dashboard-pair.json'";

    #[test]
    fn open_dashboard_is_best_effort() {
        let ok = MockRunner::new(vec![(
            AM_PAIR,
            out(0, "Starting: Intent { act=android.intent.action.VIEW }"),
        )]);
        assert!(open_dashboard(
            &Adb::new(&ok, Some("S1".into()), "adb"),
            Some("7KQ2-XM9D")
        ));
        ok.assert_exhausted();
        // `am` reports some failures on stdout with exit 0
        let no_browser = MockRunner::new(vec![(
            AM_PAIR,
            out(0, "Error: Activity not started, unable to resolve Intent"),
        )]);
        assert!(!open_dashboard(
            &Adb::new(&no_browser, Some("S1".into()), "adb"),
            Some("7KQ2-XM9D")
        ));
        let adb_down = MockRunner::new(vec![(AM_PAIR, out(1, ""))]);
        assert!(!open_dashboard(
            &Adb::new(&adb_down, Some("S1".into()), "adb"),
            Some("7KQ2-XM9D")
        ));
        // no code: the plain URL (the browser shows the pair screen)
        let plain = MockRunner::new(vec![(AM_PLAIN, out(0, "Starting: Intent"))]);
        assert!(open_dashboard(
            &Adb::new(&plain, Some("S1".into()), "adb"),
            None
        ));
        plain.assert_exhausted();
    }

    #[test]
    fn pair_code_is_read_from_the_phone_and_validated() {
        let body = r#"{"schema":1,"code":"7KQ2-XM9D","created":"2026-10-14T00:00:00Z"}"#;
        let r = MockRunner::new(vec![(CAT, out(0, body))]);
        assert_eq!(
            read_pair_code(&Adb::new(&r, Some("S1".into()), "adb")).as_deref(),
            Some("7KQ2-XM9D")
        );
        r.assert_exhausted();
        // run-as failure / missing file
        let r = MockRunner::new(vec![(CAT, out(1, ""))]);
        assert_eq!(
            read_pair_code(&Adb::new(&r, Some("S1".into()), "adb")),
            None
        );
        // malformed or hostile content never reaches the am command
        for bad in [
            "",
            "garbage",
            r#"{"schema":1,"code":"7KQ2-XM9","created":"x"}"#,
            r#"{"schema":1,"code":"7KQ2-XM9D'; reboot; '","created":"x"}"#,
            r#"{"schema":1,"code":"AEIU-OOOO","created":"x"}"#,
        ] {
            assert_eq!(parse_pair_code(bad), None, "{bad}");
        }
        // read-alikes and spacing normalize to the canonical display form
        assert_eq!(
            parse_pair_code(r#"{"schema":1,"code":" 7kq2 xm9d ","created":"x"}"#).as_deref(),
            Some("7KQ2-XM9D")
        );
        assert_eq!(
            phone_open_url(Some("7KQ2-XM9D")),
            "http://127.0.0.1:11180/#/pair?code=7KQ2-XM9D"
        );
        assert_eq!(phone_open_url(None), "http://127.0.0.1:11180");
    }

    #[test]
    fn backup_hint_uses_the_sshd_channel_with_a_tty() {
        let h = backup_hint(Path::new("/home/u/.local/share/111/keys/id_ed25519"));
        assert!(h.starts_with("ssh -t -p 8022 -i /home/u/.local/share/111/keys/id_ed25519 "));
        assert!(h.ends_with("'. $PREFIX/var/lib/111/profile.env && 111 identity backup'"));
    }
}
