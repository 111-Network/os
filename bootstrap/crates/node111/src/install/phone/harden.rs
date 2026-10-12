//! Hardening (spec phone flow step 5): doze whitelist for all three packages,
//! appops background grants, POST_NOTIFICATIONS (13+), phantom-killer ordered
//! sequence with READBACK verification (re-verified after the reboot test).
//! Documented: OTA/power-down/OEM can silently revert these; re-provisioning
//! is the repair.

use super::adb::Adb;

const PKGS: &[&str] = &["com.termux", "com.termux.api", "com.termux.boot"];

fn shell_ok(adb: &Adb<'_>, cmd: &str, marker: &str) -> bool {
    adb.shell(cmd)
        .map(|o| o.ok() && (marker.is_empty() || o.stdout.contains(marker)))
        .unwrap_or(false)
}

/// Doze whitelist + appops + notifications, each with readback.
pub fn run(adb: &Adb<'_>, sdk: u32) -> anyhow::Result<Vec<String>> {
    let mut notes: Vec<String> = Vec::new();

    // --- doze whitelist (all three packages) ---
    for pkg in PKGS {
        let ok = shell_ok(adb, &format!("dumpsys deviceidle whitelist +{pkg}"), "");
        let readback = shell_ok(adb, "dumpsys deviceidle whitelist | cat", pkg);
        if !(ok && readback) {
            notes.push(format!("deviceidle whitelist readback failed for {pkg} (OEM may revert; re-provisioning repairs)"));
        }
    }

    // --- appops background grants ---
    for (op, pkg) in [
        ("RUN_IN_BACKGROUND", "com.termux"),
        ("RUN_ANY_IN_BACKGROUND", "com.termux"),
        ("SYSTEM_ALERT_WINDOW", "com.termux"),
    ] {
        let ok = shell_ok(adb, &format!("appops set {pkg} {op} allow"), "")
            || shell_ok(adb, &format!("cmd appops set {pkg} {op} allow"), "");
        let readback = shell_ok(adb, &format!("appops get {pkg} {op}"), "allow");
        if !(ok && readback) {
            notes.push(format!("appops {op} readback failed for {pkg}"));
        }
    }

    // --- POST_NOTIFICATIONS (Android 13+) ---
    if sdk >= 33 {
        let ok = shell_ok(
            adb,
            "pm grant com.termux android.permission.POST_NOTIFICATIONS",
            "",
        );
        let readback = shell_ok(
            adb,
            "dumpsys package com.termux | grep android.permission.POST_NOTIFICATIONS",
            "granted=true",
        );
        if !(ok && readback) {
            notes.push("POST_NOTIFICATIONS grant readback failed".into());
        }
    }

    // --- phantom killer (ordered, spec step 5) ---
    notes.extend(phantom_killer(adb, sdk));

    Ok(notes)
}

/// The ordered phantom-killer sequence with readback verification.
/// Android 12 (SDK 31/32): device_config set_sync_disabled_for_tests FIRST,
/// then max_phantom_processes. 12L/13/14+ (SDK 32+): settings_enable_monitor_phantom_procs false.
pub fn phantom_killer(adb: &Adb<'_>, sdk: u32) -> Vec<String> {
    let mut notes = Vec::new();
    match sdk {
        31 => {
            // Android 12: sync-disable FIRST, then the process cap.
            let sync = shell_ok(
                adb,
                "device_config set_sync_disabled_for_tests persistent",
                "",
            );
            let cap = shell_ok(
                adb,
                "device_config put activity_manager max_phantom_processes 2147483647",
                "",
            );
            let readback = shell_ok(
                adb,
                "device_config get activity_manager max_phantom_processes",
                "2147483647",
            );
            if !(sync && cap && readback) {
                notes.push("Android 12 phantom-killer sequence failed readback".into());
            }
        }
        sdk if sdk >= 32 => {
            let set = shell_ok(
                adb,
                "settings put global settings_enable_monitor_phantom_procs false",
                "",
            );
            let readback = shell_ok(
                adb,
                "settings get global settings_enable_monitor_phantom_procs",
                "false",
            );
            if !(set && readback) {
                notes.push("phantom-procs monitor disable failed readback".into());
            }
        }
        _ => {} // < 12: no phantom killer
    }
    notes
}
