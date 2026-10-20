//! Phone preflight (spec phone flow step 1): adb (+ fallback pinned
//! platform-tools), exactly one authorized device or --serial, SDK ≥24,
//! arm64-v8a only, free space, BBK gate, MIUI/HyperOS + SIM/Mi-account gate.
//! Every refusal prints the exact prerequisite list.

use super::adb::{parse_adb_version, Adb, MIN_ADB_VERSION};
use crate::manifest::ChannelManifest;
use crate::util::http::Fetcher;
use crate::util::run::Runner;
use std::path::Path;

#[derive(Debug)]
pub struct PhonePreflight {
    pub serial: String,
    pub model: String,
    pub sdk: u32,
    pub oem: String,
    pub adb_path: String,
    /// MIUI/HyperOS detected (extra taps + SIM/Mi-account verified).
    pub xiaomi: bool,
}

fn refuse(what: &str, steps: &str) -> anyhow::Error {
    anyhow::anyhow!("phone preflight refused: {what}\nRequired steps:\n{steps}")
}

/// Find adb: PATH binary with min-version, else the sha256-pinned
/// platform-tools fallback (pin from the signed manifest — the only trust
/// anchor for the fallback zip).
pub fn ensure_adb(
    runner: &dyn Runner,
    fetcher: &Fetcher,
    manifest: &ChannelManifest,
    cache: &Path,
) -> anyhow::Result<String> {
    if let Ok(out) = runner.run("adb", &["version"]) {
        if out.ok() {
            if let Some(v) = parse_adb_version(&out.stdout) {
                if v >= MIN_ADB_VERSION {
                    return Ok("adb".to_string());
                }
                eprintln!("111 phone: adb {v:?} below minimum {MIN_ADB_VERSION:?} — using pinned platform-tools fallback");
            }
        }
    }
    let pin = manifest.platform_tools().ok_or_else(|| {
        refuse(
            "adb not found (or too old) and the channel manifest pins no platform-tools fallback",
            "  Debian/Ubuntu: sudo apt install adb\n  Fedora/Arch: sudo pacman -S android-tools / sudo dnf install android-tools",
        )
    })?;
    let dest_dir = cache.join("platform-tools");
    let adb = dest_dir.join("adb");
    if adb.exists() {
        return Ok(adb.to_string_lossy().into_owned());
    }
    let zip = fetcher.get(&pin.url)?;
    let got = crate::util::hash::sha256_bytes(&zip);
    anyhow::ensure!(got == pin.sha256, "platform-tools zip sha256 mismatch");
    std::fs::create_dir_all(&dest_dir)?;
    let zip_path = cache.join("platform-tools.zip");
    crate::util::fs::atomic_write(&zip_path, &zip, 0o644)?;
    let out = runner.run(
        "unzip",
        &[
            "-o",
            "-q",
            &zip_path.to_string_lossy(),
            "-d",
            &cache.to_string_lossy(),
        ],
    )?;
    anyhow::ensure!(
        out.ok(),
        "unzip platform-tools failed: {}",
        out.stderr.trim()
    );
    let out = runner.run("chmod", &["0755", &adb.to_string_lossy()])?;
    anyhow::ensure!(out.ok(), "chmod adb failed");
    Ok(adb.to_string_lossy().into_owned())
}

pub fn run(
    adb: &Adb<'_>,
    serial_flag: Option<&str>,
    allow_degraded: bool,
) -> anyhow::Result<PhonePreflight> {
    Adb::refuse_root()?;

    // --- exactly one authorized device, or --serial ---
    let devices = adb.devices()?;
    let authorized: Vec<&(String, String)> =
        devices.iter().filter(|(_, s)| s == "device").collect();
    let serial = match serial_flag {
        Some(s) => {
            anyhow::ensure!(
                authorized.iter().any(|(dev, _)| dev == s),
                refuse(
                    &format!("--serial {s} not found among authorized devices"),
                    "  check `adb devices` output and re-authorize the phone"
                )
            );
            s.to_string()
        }
        None => {
            anyhow::ensure!(
                authorized.len() == 1,
                refuse(
                    &format!("{} authorized devices (need exactly 1)", authorized.len()),
                    "  attach exactly one phone, or pass --serial SERIAL\n  unauthorized devices need the on-device RSA-authorize tap"
                )
            );
            authorized[0].0.clone()
        }
    };
    let adb = Adb::new(adb.runner, Some(serial.clone()), &adb.adb_path);

    // --- SDK ≥ 24 ---
    let sdk: u32 = adb.getprop("ro.build.version.sdk")?.parse().map_err(|_| {
        refuse(
            "unparseable ro.build.version.sdk",
            "  is this an Android phone?",
        )
    })?;
    anyhow::ensure!(
        sdk >= 24,
        refuse(&format!("SDK {sdk} < 24"), "  Android 7.0+ required")
    );

    // --- arm64-v8a only (armv7 refused) ---
    let abi = adb.getprop("ro.product.cpu.abi")?;
    anyhow::ensure!(
        abi == "arm64-v8a",
        refuse(
            &format!("cpu abi {abi} (need arm64-v8a)"),
            "  32-bit ARM phones (armv7/armeabi) are refused per spec — no aarch64, no node"
        )
    );

    // --- free space (≥1GB on /data/local/tmp) ---
    let df = adb.shell("df /data/local/tmp | tail -1")?;
    let avail_kb: u64 = df
        .stdout
        .split_whitespace()
        .nth(3)
        .and_then(|s| s.trim_end_matches('K').parse().ok())
        .unwrap_or(0);
    anyhow::ensure!(
        avail_kb >= 1_000_000,
        refuse(
            &format!("{}MB free on /data (need ≥1GB)", avail_kb / 1000),
            "  free up phone storage"
        )
    );

    let model = adb
        .getprop_opt("ro.product.model")
        .unwrap_or_else(|| "unknown".into());
    let oem = adb
        .getprop_opt("ro.product.brand")
        .unwrap_or_else(|| "unknown".into());

    // --- BBK gate (OPPO/OnePlus/vivo/iQOO/realme daemon-killing builds) ---
    let brand = oem.to_lowercase();
    let is_bbk = ["oppo", "oneplus", "vivo", "iqoo", "realme"].contains(&brand.as_str());
    if is_bbk {
        let build_utc: u64 = adb.getprop("ro.build.date.utc")?.parse().map_err(|_| {
            refuse(
                "unparseable ro.build.date.utc",
                "  BBK build gate can't run",
            )
        })?;
        // scoped to the documented ColorOS/OxygenOS builds (pre-2024-06-22)
        if build_utc < 1_719_129_600 && !allow_degraded {
            return Err(refuse(
                &format!("{oem} build from {} predates the phantom-process-fix builds", build_utc),
                "  update the phone's OS (ColorOS/OxygenOS June 2024+), or re-run with --allow-degraded\n  (the node may be killed in the background on this build)",
            ));
        }
    }

    // --- MIUI/HyperOS: extra taps + SIM/Mi-account gate ---
    let miui = adb.getprop_opt("ro.miui.version.code").is_some();
    let hyperos = adb.getprop_opt("ro.mi.os.version.code").is_some();
    let xiaomi = miui || hyperos;
    if xiaomi {
        let sim = adb.getprop_opt("gsm.sim.state").unwrap_or_default();
        anyhow::ensure!(
            sim.eq_ignore_ascii_case("LOADED") || sim.contains("LOADED"),
            refuse(
                "Xiaomi (MIUI/HyperOS) phone without a SIM",
                "  'Install via USB' on MIUI/HyperOS needs: a SIM card inserted,\n  a Mi account signed in, and the toggle enabled per-app.\n  A SIM-less Xiaomi cannot be provisioned (spec §phone flow)."
            )
        );
        eprintln!(
            "111 phone: Xiaomi device — expect extra taps: Mi-account-gated 'Install via USB' \
             toggle + one confirm per APK (3 APKs). Watch the phone screen."
        );
    }

    Ok(PhonePreflight {
        serial,
        model,
        sdk,
        oem,
        adb_path: adb.adb_path.clone(),
        xiaomi,
    })
}
