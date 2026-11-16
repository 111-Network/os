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

use crate::calver::Calver;
use crate::cli::InstallArgs;
use crate::manifest::{ChannelManifest, PinnedArtifact};
use adb::Adb;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const BOOT_SCRIPT: &str = include_str!("../../../../../termux/boot/00-111-node.sh");
const PROFILE_ENV: &str = include_str!("../../../../../termux/profile.env");
const SV_RUN: &str = include_str!("../../../../../termux/sv/111node/run");
const SV_FINISH: &str = include_str!("../../../../../termux/sv/111node/finish");
/// Where a provisioner before Layer 9 pushed the Termux cron health script. The script left the
/// tree with the cron seam (Layer 9 §1 rule 2, §3.3 rule 6); [`install_service_files`] deletes a
/// copy still on the phone.
const OLD_CRON_SCRIPT: &str = "$PREFIX/var/lib/111/share/termux/cron-healthcheck.sh";
/// `state/apk.json` in the Termux root: the record of the APK step, which `111d` compares with
/// the manifest's `apk.*` pins after each verified manifest check (Layer 9 §12.6, §4's path table).
const APK_RECORD: &str = "$PREFIX/var/lib/111/state/apk.json";
/// The record lands through this temp file and one rename, so a reader never sees half of one.
const APK_RECORD_TMP: &str = "$PREFIX/var/lib/111/state/.apk.json.tmp";

/// What the APK step installed (Layer 9 §12.6): each pinned package ([`apk::APK_PACKAGES`]) and
/// the sha256 its APK was installed from, so at most three entries.
pub type ApkPins = BTreeMap<String, String>;

/// Host-side resume marker (per device serial): which steps completed, and the pins step 3
/// installed (kept beside its marker so a resumed run still records them, Layer 9 §12.6).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProvisionState {
    serial: String,
    done: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    apk: Option<ApkPins>,
}

impl ProvisionState {
    /// The marker for `serial`; a missing, unreadable or foreign one (another serial, or pins
    /// that are not a valid APK record) is a fresh state.
    pub fn load(path: &Path, serial: &str) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .filter(|s: &ProvisionState| {
                s.serial == serial && s.apk.as_ref().is_none_or(|p| apk_record(p).is_ok())
            })
            .unwrap_or(ProvisionState {
                serial: serial.to_string(),
                done: Vec::new(),
                apk: None,
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
    /// The pins the APK step installed, in this run or in the run this one resumes; `None`
    /// when no APK step of this provisioner completed for the phone.
    pub fn apk_pins(&self) -> Option<&ApkPins> {
        self.apk.as_ref()
    }
    /// Keep the pins step 3 installed; saved with that step's [`mark`](Self::mark).
    pub fn set_apk_pins(&mut self, pins: ApkPins) {
        self.apk = Some(pins);
    }
}

/// Step 3, the APK step (Layer 1 "Install flow — phone over USB-C", step 3): the three pinned
/// APKs resolved from `cache` against `apks`, installed (`adb install -r`, each `versionName`
/// verified); returns the pins it installed. Nothing is written to the phone here: the record of
/// these pins waits for step 4's run-as channel ([`push_apk_record`]), because Termux unpacks
/// its bootstrap into `$PREFIX` at its first start, which step 4 waits for, and skips that unpack
/// when `$PREFIX` already holds files.
pub fn install_apks(
    adb: &Adb<'_>,
    cache: &Path,
    apks: &[PinnedArtifact],
) -> anyhow::Result<ApkPins> {
    let set = apk::resolve(cache, apks)?;
    apk::install_all(adb, &set)?;
    Ok(set
        .files
        .iter()
        .map(|(package, artifact, _)| (package.clone(), artifact.sha256.clone()))
        .collect())
}

/// The bytes of `state/apk.json` (Layer 9 §12.6): `{"schema":1,"apk":{<package>:<sha256>,…}}`
/// and a newline, the packages in byte order. Refused unless it names one to three packages,
/// each one of [`apk::APK_PACKAGES`], each pin 64 lowercase hex.
pub fn apk_record(pins: &ApkPins) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        (1..=apk::APK_PACKAGES.len()).contains(&pins.len()),
        "an APK record names 1 to {} packages, not {}",
        apk::APK_PACKAGES.len(),
        pins.len()
    );
    for (package, sha256) in pins {
        anyhow::ensure!(
            apk::APK_PACKAGES.iter().any(|(p, _, _)| p == package),
            "{package:?} is not a pinned APK package"
        );
        anyhow::ensure!(
            sha256.len() == 64
                && sha256
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
            "the pin of {package} is not 64 lowercase hex"
        );
    }
    #[derive(Serialize)]
    struct Record<'a> {
        schema: u32,
        apk: &'a ApkPins,
    }
    let mut bytes = serde_json::to_vec(&Record {
        schema: 1,
        apk: pins,
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Push the record of the APK step to the phone as `state/apk.json`, 0600 (Layer 9 §12.6; §4's
/// path table): [`runas::install_file`] writes it to `.apk.json.tmp` beside it and checks its
/// sha256 on the device, then one run-as command sets 0600 and renames it into place. A record
/// already there is replaced whole. Only step 4 calls this, over its run-as channel, so the
/// record is never written before the APK step nor by `111d`.
pub fn push_apk_record(adb: &Adb<'_>, pins: &ApkPins) -> anyhow::Result<()> {
    let record = apk_record(pins)?;
    runas::install_file(adb, &record, APK_RECORD_TMP, false)?;
    let out = runas::run_as_output(
        adb,
        &format!(
            "chmod 0600 {APK_RECORD_TMP} && mv -f {APK_RECORD_TMP} {APK_RECORD} && echo APK_RECORD_OK"
        ),
    )?;
    anyhow::ensure!(
        out.contains("APK_RECORD_OK"),
        "landing state/apk.json failed: {}",
        out.trim()
    );
    Ok(())
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

/// Step 4's service files over run-as: `profile.env`, then runit's `run` and its `finish` hook
/// (the hook carries Layer 9's exit witness and reaches a phone only this way, Layer 9 §3.3
/// rule 7). Then the one delete the removal of the Termux cron seam adds (Layer 9 §3.3 rule 6):
/// the health script a provisioner before Layer 9 pushed (`OLD_CRON_SCRIPT`) is deleted when
/// present; `rm -f` has nothing to do where it is absent. Nothing pushes the script any more,
/// and `111d` never deletes it (it is inert: nothing schedules it, Layer 9 §1 rule 2).
pub fn install_service_files(adb: &Adb<'_>) -> anyhow::Result<()> {
    runas::install_file(
        adb,
        PROFILE_ENV.as_bytes(),
        "$PREFIX/var/lib/111/profile.env",
        false,
    )?;
    runas::install_file(
        adb,
        SV_RUN.as_bytes(),
        "$PREFIX/var/lib/111/sv/111node/run",
        true,
    )?;
    runas::install_file(
        adb,
        SV_FINISH.as_bytes(),
        "$PREFIX/var/lib/111/sv/111node/finish",
        true,
    )?;
    let out = runas::run_as_output(adb, &format!("rm -f {OLD_CRON_SCRIPT} && echo RM_OK"))?;
    anyhow::ensure!(
        out.contains("RM_OK"),
        "deleting the old cron health script failed: {}",
        out.trim()
    );
    Ok(())
}

/// Step 4, the headless channel (Layer 1 "Install flow — phone over USB-C", step 4), whole: the
/// bootstrap wait, then over run-as the boot script, the payload (pushed, hash-checked, extracted
/// as `release`), [`install_service_files`] (with Layer 9 §3.3 rule 6's one delete), the record
/// of the APK step when `apk_pins` holds the pins step 3 installed ([`push_apk_record`], Layer 9
/// §12.6) and the `.deb` closure from `cache`; when run-as is broken, the E2 fallback instead
/// (the stub over `adb reverse`; no record); last, the `allow-external-apps` guard. One
/// function so a transcript test pins the order `run` provisions in.
pub fn headless_channel(
    adb: &Adb<'_>,
    payload_path: &Path,
    payload: &PinnedArtifact,
    release: Calver,
    cache: &Path,
    debs: &[PinnedArtifact],
    apk_pins: Option<&ApkPins>,
) -> anyhow::Result<()> {
    runas::wait_bootstrap(adb, 120)?;
    if runas::run_as_works(adb) {
        runas::install_boot_script(adb, BOOT_SCRIPT.as_bytes())?;
        runas::push_verified(
            adb,
            payload_path,
            "$PREFIX/var/lib/111/releases/payload.tar.gz",
            &payload.sha256,
        )?;
        let ver = release.to_string();
        let out = runas::run_as_output(
            adb,
            &format!(
                "mkdir -p $PREFIX/var/lib/111/releases/{ver} && tar -xzf $PREFIX/var/lib/111/releases/payload.tar.gz -C $PREFIX/var/lib/111/releases/{ver} && ln -sfn $PREFIX/var/lib/111/releases/{ver} $PREFIX/var/lib/111/current && rm $PREFIX/var/lib/111/releases/payload.tar.gz && echo EXTRACT_OK"
            ),
        )?;
        anyhow::ensure!(
            out.contains("EXTRACT_OK"),
            "on-device payload extract failed: {out}"
        );
        // profile + runit service; an earlier provisioner's cron script deleted
        install_service_files(adb)?;
        // the record of the APK step: the pins step 3 installed (Layer 9 §12.6)
        if let Some(pins) = apk_pins {
            push_apk_record(adb, pins)?;
        }
        runas::install_debs(adb, cache, debs)?;
    } else {
        eprintln!("111 phone: run-as channel broken — E2 fallback (typed stub + adb reverse)");
        if apk_pins.is_some() {
            eprintln!(
                "111 phone: state/apk.json (the APK step's record) needs run-as: not written"
            );
        }
        let port = 18111u16;
        runas::e2e_reverse(adb, port)?;
        let stub = resolve_e2e_stub(port, payload)?;
        runas::e2e_inject_stub(adb, &stub)?;
        // the stub pulls + verifies the payload via the reverse channel;
        // the remaining files go through the same host server
    }
    runas::assert_no_external_apps(adb)
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

    // 3. APKs; the pins they were installed from are kept with the step's marker
    step!(state, &marker, "apks", {
        let pins = install_apks(&adb, &cache, manifest.termux_apks())?;
        state.set_apk_pins(pins);
    });

    // 4. headless channel, which also records the APK step's pins (Layer 9 §12.6)
    step!(state, &marker, "channel", {
        headless_channel(
            &adb,
            &payload_path,
            &payload_artifact,
            manifest.release(),
            &cache,
            manifest.termux_debs(),
            state.apk_pins(),
        )?;
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

    /// `run` needs a real adb and the network, so its hand-off of the APK step's pins (Layer 9
    /// §12.6) is pinned on its source: step 3 keeps the pins it installed with its marker, step 4
    /// passes the marker's pins to the channel, and nothing else in `run` reaches the record.
    #[test]
    fn run_hands_the_apk_steps_pins_to_step_4() {
        let src = include_str!("mod.rs").split("#[cfg(test)]").next().unwrap();
        let run = src.split("pub fn run(").nth(1).unwrap();
        let run = run.split("\n}\n").next().unwrap();
        let step = |name: &str| {
            let head = format!("step!(state, &marker, {name:?}, {{");
            let body = run.split(&head).nth(1).unwrap();
            body.split("\n    });\n").next().unwrap().to_string()
        };
        let apks = step("apks");
        assert!(
            apks.contains("install_apks(&adb, &cache, manifest.termux_apks())?")
                && apks.contains("state.set_apk_pins(pins);"),
            "{apks}"
        );
        assert!(step("channel").contains("state.apk_pins(),"), "{run}");
        assert_eq!(run.matches("apk_pins").count(), 2, "{run}");
        assert!(!run.contains("push_apk_record"), "{run}");
    }
}
