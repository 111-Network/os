//! Phone provisioning over USB-C (`111 install --phone`). Runs as the invoking
//! user (adb authorization is per-user — never root).

pub mod adb;
pub mod apk;
pub mod harden;
pub mod preflight;
pub mod prompt;
pub mod runas;
pub mod start;
pub mod teardown;
pub mod verify;

use crate::cli::InstallArgs;
use crate::manifest::ChannelManifest;
use adb::Adb;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const BOOT_SCRIPT: &str = include_str!("../../../../../termux/boot/00-111-node.sh");
const PROFILE_ENV: &str = include_str!("../../../../../termux/profile.env");
const SV_RUN: &str = include_str!("../../../../../termux/sv/111node/run");
const SV_FINISH: &str = include_str!("../../../../../termux/sv/111node/finish");
const CRON_HEALTH: &str = include_str!("../../../../../termux/cron/healthcheck.sh");

/// Host-side resume marker (per device serial): which steps completed.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProvisionState {
    serial: String,
    done: Vec<String>,
}

impl ProvisionState {
    pub fn load(path: &Path, serial: &str) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .filter(|s: &ProvisionState| s.serial == serial)
            .unwrap_or(ProvisionState {
                serial: serial.to_string(),
                done: Vec::new(),
            })
    }
    fn save(&self, path: &Path) {
        let _ = crate::util::fs::atomic_write(
            path,
            serde_json::to_string_pretty(self).unwrap().as_bytes(),
            0o600,
        );
    }
    pub fn is_done(&self, step: &str) -> bool {
        self.done.iter().any(|s| s == step)
    }
    pub fn mark(&mut self, step: &str, path: &Path) {
        self.done.push(step.to_string());
        self.save(path);
    }
}

pub fn cache_dir() -> PathBuf {
    crate::util::paths::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/111")
}

/// Download + pin-verify a manifest artifact into the host cache (idempotent).
fn prefetch(
    fetcher: &crate::util::http::Fetcher,
    cache: &Path,
    artifact: &crate::manifest::PinnedArtifact,
    filename: &str,
) -> anyhow::Result<PathBuf> {
    let dest = cache.join(filename);
    if dest.exists() && crate::util::hash::sha256_file(&dest)? == artifact.sha256 {
        return Ok(dest);
    }
    let bytes = fetcher.get(&artifact.url)?;
    let got = crate::util::hash::sha256_bytes(&bytes);
    anyhow::ensure!(
        got == artifact.sha256,
        "{filename} sha256 mismatch vs manifest"
    );
    crate::util::fs::atomic_write(&dest, &bytes, 0o644)?;
    Ok(dest)
}

fn fetch_manifest(fetcher: &crate::util::http::Fetcher) -> anyhow::Result<ChannelManifest> {
    use crate::daemon::updater::fetch::{DEFAULT_MANIFEST_BASE, FALLBACK_MANIFEST_BASE};
    for base in [DEFAULT_MANIFEST_BASE, FALLBACK_MANIFEST_BASE] {
        let url = format!("{}/channel-111-stable.toml", base.trim_end_matches('/'));
        if let (Ok(toml), Ok(sig)) = (fetcher.get(&url), fetcher.get(&format!("{url}.minisig"))) {
            let sig_text =
                String::from_utf8(sig).map_err(|_| anyhow::anyhow!("minisig not UTF-8"))?;
            let manifest = ChannelManifest::parse_verified(
                &toml,
                &sig_text,
                crate::daemon::updater::MANIFEST_PUBKEY.trim(),
                &crate::floor::Floor::initial(),
            )?;
            return Ok(manifest);
        }
    }
    anyhow::bail!("channel manifest unreachable on all endpoints (needed for APK/deb pins)")
}

/// Step runner with resume markers.
macro_rules! step {
    ($state:expr, $marker:expr, $name:literal, $body:block) => {
        if $state.is_done($name) {
            eprintln!("111 phone: [skip] {} (already done)", $name);
        } else {
            eprintln!("111 phone: [step] {}", $name);
            $body
            $state.mark($name, $marker);
        }
    };
}

/// The phone flow (spec §Install flow — phone over USB-C, steps 1–8).
pub fn run(args: InstallArgs) -> anyhow::Result<()> {
    Adb::refuse_root()?;
    let runner = crate::util::run::SystemRunner;
    let fetcher =
        crate::util::http::Fetcher::new(std::time::Duration::from_secs(120), 512 * 1024 * 1024)?;
    let manifest = fetch_manifest(&fetcher)?;
    let cache = cache_dir();
    std::fs::create_dir_all(&cache)?;

    // prefetch pinned artifacts into the host cache
    let mut apk_files = Vec::new();
    for apk in manifest.termux_apks() {
        apk_files.push(prefetch(
            &fetcher,
            &cache,
            apk,
            &format!("{}.apk", apk.name),
        )?);
    }
    let mut deb_files = Vec::new();
    for deb in manifest.termux_debs() {
        deb_files.push(prefetch(
            &fetcher,
            &cache,
            deb,
            &format!("{}.deb", deb.name),
        )?);
    }
    let payload_artifact = manifest
        .payload_for("aarch64-unknown-linux-musl")
        .ok_or_else(|| anyhow::anyhow!("manifest ships no aarch64 payload (phone)"))?
        .clone();
    let payload_path = prefetch(
        &fetcher,
        &cache,
        &payload_artifact,
        "payload-aarch64.tar.gz",
    )?;

    // 1. preflight
    let adb_path = preflight::ensure_adb(&runner, &fetcher, &manifest, &cache)?;
    let adb_any = Adb::new(&runner, args.serial.clone(), &adb_path);
    let pre = preflight::run(&adb_any, args.serial.as_deref(), args.allow_degraded)?;
    let adb = Adb::new(&runner, Some(pre.serial.clone()), &adb_path);
    eprintln!(
        "111 phone: {} {} (sdk {}, serial {})",
        pre.oem, pre.model, pre.sdk, pre.serial
    );

    let marker = cache.join(format!("phone-provision-{}.json", pre.serial));
    let mut state = ProvisionState::load(&marker, &pre.serial);

    // 2. collision check
    step!(state, &marker, "collision", {
        apk::collision_check(&adb)?;
    });

    // 3. APKs
    step!(state, &marker, "apks", {
        let set = apk::resolve(&cache, manifest.termux_apks())?;
        apk::install_all(&adb, &set)?;
    });

    // 4. headless channel
    step!(state, &marker, "channel", {
        runas::wait_bootstrap(&adb, 120)?;
        if runas::run_as_works(&adb) {
            runas::install_boot_script(&adb, BOOT_SCRIPT.as_bytes())?;
            runas::push_verified(
                &adb,
                &payload_path,
                "$PREFIX/var/lib/111/releases/payload.tar.gz",
                &payload_artifact.sha256,
            )?;
            let ver = manifest.release().to_string();
            let out = runas::run_as_output(
                &adb,
                &format!(
                    "mkdir -p $PREFIX/var/lib/111/releases/{ver} && tar -xzf $PREFIX/var/lib/111/releases/payload.tar.gz -C $PREFIX/var/lib/111/releases/{ver} && ln -sfn $PREFIX/var/lib/111/releases/{ver} $PREFIX/var/lib/111/current && rm $PREFIX/var/lib/111/releases/payload.tar.gz && echo EXTRACT_OK"
                ),
            )?;
            anyhow::ensure!(
                out.contains("EXTRACT_OK"),
                "on-device payload extract failed: {out}"
            );
            // profile + runit service + cron healthcheck
            runas::install_file(
                &adb,
                PROFILE_ENV.as_bytes(),
                "$PREFIX/var/lib/111/profile.env",
                false,
            )?;
            runas::install_file(
                &adb,
                SV_RUN.as_bytes(),
                "$PREFIX/var/lib/111/sv/111node/run",
                true,
            )?;
            runas::install_file(
                &adb,
                SV_FINISH.as_bytes(),
                "$PREFIX/var/lib/111/sv/111node/finish",
                true,
            )?;
            runas::install_file(
                &adb,
                CRON_HEALTH.as_bytes(),
                "$PREFIX/var/lib/111/share/termux/cron-healthcheck.sh",
                true,
            )?;
            runas::install_debs(&adb, &cache, manifest.termux_debs())?;
        } else {
            eprintln!("111 phone: run-as channel broken — E2 fallback (typed stub + adb reverse)");
            let port = 18111u16;
            runas::e2e_reverse(&adb, port)?;
            let stub = resolve_e2e_stub(port, &payload_artifact)?;
            runas::e2e_inject_stub(&adb, &stub)?;
            // the stub pulls + verifies the payload via the reverse channel;
            // the remaining files go through the same host server
        }
        runas::assert_no_external_apps(&adb)?;
    });

    // 5. harden
    step!(state, &marker, "harden", {
        let notes = harden::run(&adb, pre.sdk)?;
        for n in notes {
            eprintln!("111 phone: hardening note: {n}");
        }
    });

    // 6. start now
    let mut node_id = String::new();
    step!(state, &marker, "start", {
        let keys = start::host_keys_dir();
        let host_key = start::ensure_host_keypair(&runner, &keys)?;
        node_id = start::on_device_keygen(&adb)?;
        let pubkey = std::fs::read_to_string(format!("{}.pub", host_key.display()))?;
        start::setup_sshd(&adb, pubkey.trim())?;
        start::start_node(&adb)?;
    });

    // 7. verify
    let mut pair_code = None;
    step!(state, &marker, "verify", {
        start::verify_health(&adb, &fetcher, 90)?;
        verify::on_device_health(&adb)?;
        let host_key = start::host_keys_dir().join("id_ed25519");
        start::verify_sshd_forward(&adb, &runner, &host_key)?;
        // Layer 4 §12: phone-local browser, before the lockdown kills adb.
        // Loopback is not trusted on a phone: open the pair link.
        pair_code = start::read_pair_code(&adb);
        start::open_dashboard(&adb, pair_code.as_deref());
    });
    // resumed past verify: read it now, while adb still works (final print)
    let pair_code = pair_code.or_else(|| start::read_pair_code(&adb));

    // 8. ordered final: optional reboot test → OEM checklist → labeled lockdown
    if args.reboot_test {
        eprintln!("111 phone: reboot test — the phone restarts now; unlock it when prompted");
        let _ = adb.cmd(&["reboot"]);
        wait_for_device(&adb, 180)?;
        // PIN lockscreen: node starts at first unlock (FBE/BFU reality)
        eprintln!("111 phone: if a lockscreen is set, unlock once — the node starts at first unlock (recorded)");
        start::verify_health(&adb, &fetcher, 120)?;
        let notes = harden::phantom_killer(&adb, pre.sdk);
        for n in notes {
            eprintln!("111 phone: post-reboot hardening note: {n}");
        }
    }

    print_oem_checklist(&pre);

    if args.keep_adb {
        eprintln!("111 phone: --keep-adb set — lockdown SKIPPED (adb stays enabled)");
    } else {
        let host_key = start::host_keys_dir().join("id_ed25519");
        verify::fresh_lan_sshd_ceremony(&adb, &host_key, &runner, args.yes)?;
        eprintln!(
            "111 phone: POINT OF NO RETURN — disabling adb (settings put global adb_enabled 0)."
        );
        eprintln!("         re-enabling needs on-device taps (Developer Options). Continue? [y/N]");
        anyhow::ensure!(
            prompt::confirm(prompt::open_tty(), args.yes, "the adb lockdown")?,
            "lockdown aborted by operator (adb stays enabled; re-run to resume, or use --keep-adb)"
        );
        if args.yes {
            eprintln!("111 phone: --yes — lockdown confirmed by flag");
        }
        let out = adb.shell("settings put global adb_enabled 0")?;
        anyhow::ensure!(out.ok(), "adb_enabled 0 failed: {}", out.stderr.trim());
        eprintln!(
            "111 phone: adb disabled. Maintenance channel: ssh -p 8022 -i {} <phone-ip>",
            host_key.display()
        );
    }

    let human = crate::identity::node_id_bech32(&node_id).unwrap_or_else(|| node_id.clone());
    let host_key = start::host_keys_dir().join("id_ed25519");
    println!(
        "\n111 phone provisioned:\n  node id:   {human}\n  hex:       {node_id}\n  serial:    {}\n  dashboard: {} (in the phone's browser){}\n  teardown:  111 uninstall --phone{}\n{}{}\nBack up the phone's identity (12 words) with:\n  {}",
        pre.serial,
        start::phone_open_url(pair_code.as_deref()),
        if args.keep_adb {
            "\n             from this computer: adb forward tcp:11180 tcp:11180 → http://127.0.0.1:11180"
        } else {
            ""
        },
        if args.keep_adb { " (adb kept)" } else { " (over sshd)" },
        pair_code_text(pair_code.as_deref(), &host_key),
        lan_sharing_text(&host_key),
        start::backup_hint(&host_key)
    );
    Ok(())
}

/// The pair-code line of the phone's final print. Loopback is not trusted on a
/// phone node, so the phone's own browser pairs too (once); LAN visitors and
/// an `adb forward` browser use the same code. Unknown code → the
/// `111 dashboard` hint over the maintenance channel.
pub fn pair_code_text(code: Option<&str>, host_key: &Path) -> String {
    match code {
        Some(c) => format!(
            "Pair code: {c}   (the phone's browser asks for it once; so does every other browser)\n"
        ),
        None => format!(
            "Pair code: not read — the phone's browser asks for it; show it with:\n  ssh -p 8022 -i {} <phone-ip> '. $PREFIX/var/lib/111/profile.env && 111 dashboard'\n",
            host_key.display()
        ),
    }
}

/// The LAN-sharing lines of the phone's final print (the host print has the
/// same line). LAN sharing is off after provisioning; it is turned on over the
/// sshd maintenance channel. Visitors use the phone's IP: `111.local` does not
/// work for a phone node (mDNS receive needs a MulticastLock no CLI can take,
/// D12).
pub fn lan_sharing_text(host_key: &Path) -> String {
    format!(
        "LAN sharing: off — to open the dashboard from other devices on the phone's Wi-Fi:
  ssh -p 8022 -i {} <phone-ip> '. $PREFIX/var/lib/111/profile.env && 111 config dashboard.lan true && {} && sleep 3 && 111 dashboard'
  then browse to http://<phone-ip>:11180 and enter the pair code it prints
  (use the IP: 111.local does not resolve for a phone node — Android needs a MulticastLock for mDNS)
",
        host_key.display(),
        crate::config::TERMUX_RESTART
    )
}

fn wait_for_device(adb: &Adb<'_>, timeout_secs: u64) -> anyhow::Result<()> {
    let start = std::time::Instant::now();
    loop {
        if adb
            .devices()
            .map(|d| d.iter().any(|(_, s)| s == "device"))
            .unwrap_or(false)
        {
            return Ok(());
        }
        if start.elapsed().as_secs() > timeout_secs {
            anyhow::bail!("phone did not come back within {timeout_secs}s");
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}

/// Resolve the E2 stub with the provisioner's tokens.
fn resolve_e2e_stub(
    port: u16,
    payload: &crate::manifest::PinnedArtifact,
) -> anyhow::Result<Vec<u8>> {
    let template = include_str!("../../../../../termux/e2e-stub.sh");
    Ok(template
        .replace("@@E2_PORT@@", &port.to_string())
        .replace("@@PAYLOAD_SHA256@@", &payload.sha256)
        .replace("@@DEST@@", "$PREFIX/tmp/111-payload.tgz")
        .into_bytes())
}

fn print_oem_checklist(pre: &preflight::PhonePreflight) {
    println!(
        "
111 phone — per-OEM checklist ({} {}):
  [ ] battery optimization: Termux set to 'unrestricted' (Settings → Apps)
  [ ] auto-start/background permission granted where the OEM asks
  [ ] lockscreen PIN note: after reboot the node starts at FIRST UNLOCK (FBE)
  [ ] true adb authorization revocation is a manual Developer-Options step
      ('Revoke USB debugging authorizations') — the scriptable lockdown is done{}",
        pre.oem,
        pre.model,
        if pre.xiaomi {
            "\n  [ ] MIUI: 'Install via USB' stays on; per-APK confirms expected on updates"
        } else {
            ""
        }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_print_lan_line_uses_the_phone_ip_not_mdns() {
        let t = lan_sharing_text(Path::new("/home/u/.local/share/111/keys/id_ed25519"));
        assert!(t.starts_with("LAN sharing: off"), "{t}");
        assert!(t.contains(
            "ssh -p 8022 -i /home/u/.local/share/111/keys/id_ed25519 <phone-ip> '. $PREFIX/var/lib/111/profile.env && 111 config dashboard.lan true && SVDIR=$PREFIX/var/service sv restart 111node"
        ));
        assert!(t.contains("http://<phone-ip>:11180"));
        assert!(t.contains("111.local does not resolve for a phone node"));
        assert!(!t.contains("http://111.local"));
    }

    #[test]
    fn final_print_shows_the_pair_code() {
        let key = Path::new("/k/id_ed25519");
        assert_eq!(
            pair_code_text(Some("7KQ2-XM9D"), key),
            "Pair code: 7KQ2-XM9D   (the phone's browser asks for it once; so does every other browser)\n"
        );
        let t = pair_code_text(None, key);
        assert!(t.starts_with("Pair code: not read"), "{t}");
        assert!(t.contains(
            "ssh -p 8022 -i /k/id_ed25519 <phone-ip> '. $PREFIX/var/lib/111/profile.env && 111 dashboard'"
        ));
    }
}
