//! T12 acceptance: phone provisioning modules against transcript-driven fake adb
//! (MockRunner). Covers the happy path + every refusal branch + harden readbacks
//! + teardown. Real-device behavior is gated externally (V1/V2/V11 runbooks, T16).

use node111::install::phone::adb::Adb;
use node111::install::phone::{apk, harden, preflight, runas, teardown};
use node111::manifest::PinnedArtifact;
use node111::testkit::MockRunner;
use node111::util::run::Output;

fn out(status: i32, stdout: &str) -> Output {
    Output {
        status,
        stdout: stdout.to_string(),
        stderr: String::new(),
    }
}

fn adb_of(runner: &MockRunner) -> Adb<'_> {
    Adb::new(runner, Some("SER1".into()), "adb")
}

/// The Termux env every run-as command must carry (spelled out here, not
/// borrowed from the crate, so a change there fails these tests).
const TERMUX_ENV: &str = "export PREFIX=/data/data/com.termux/files/usr HOME=/data/data/com.termux/files/home PATH=/data/data/com.termux/files/usr/bin:$PATH;";

/// Expected `adb shell` key for a run-as command: env + `inner`, one
/// single-quoted word (`'` → `'\''`).
fn ra(inner: &str) -> String {
    format!(
        "adb -s SER1 shell run-as com.termux sh -c '{TERMUX_ENV} {}'",
        inner.replace('\'', "'\\''")
    )
}

// ---------- preflight ----------

fn happy_props(script: &mut Vec<(String, Output)>) {
    script.extend([
        (
            "adb -s SER1 shell getprop ro.build.version.sdk".into(),
            out(0, "34\n"),
        ),
        (
            "adb -s SER1 shell getprop ro.product.cpu.abi".into(),
            out(0, "arm64-v8a\n"),
        ),
        (
            "adb -s SER1 shell df /data/local/tmp | tail -1".into(),
            out(0, "/dev/block/dm-1 100000000 20000000 80000000 25% /data\n"),
        ),
        (
            "adb -s SER1 shell getprop ro.product.model".into(),
            out(0, "Pixel 8\n"),
        ),
        (
            "adb -s SER1 shell getprop ro.product.brand".into(),
            out(0, "google\n"),
        ),
        (
            "adb -s SER1 shell getprop ro.miui.version.code".into(),
            out(0, "\n"),
        ),
        (
            "adb -s SER1 shell getprop ro.mi.os.version.code".into(),
            out(0, "\n"),
        ),
    ]);
}

#[test]
fn preflight_happy() {
    let mut script: Vec<(String, Output)> = vec![(
        "adb devices".into(),
        out(0, "List of devices attached\nSER1\tdevice\n"),
    )];
    happy_props(&mut script);
    let runner = MockRunner::new(script);
    let adb = Adb::new(&runner, None, "adb");
    let pre = preflight::run(&adb, None, false).unwrap();
    assert_eq!(pre.serial, "SER1");
    assert_eq!(pre.sdk, 34);
    assert_eq!(pre.model, "Pixel 8");
    assert!(!pre.xiaomi);
    runner.assert_exhausted();
}

#[test]
fn preflight_two_devices_refused() {
    let runner = MockRunner::new(vec![(
        "adb devices",
        out(0, "List of devices attached\nSER1\tdevice\nSER2\tdevice\n"),
    )]);
    let adb = Adb::new(&runner, None, "adb");
    let err = preflight::run(&adb, None, false).unwrap_err();
    assert!(err.to_string().contains("exactly 1"), "{err}");
}

#[test]
fn preflight_serial_flag_selects() {
    let mut script: Vec<(String, Output)> = vec![(
        "adb devices".into(),
        out(0, "List of devices attached\nSER1\tdevice\nSER2\tdevice\n"),
    )];
    happy_props(&mut script);
    let runner = MockRunner::new(script);
    let adb = Adb::new(&runner, None, "adb");
    let pre = preflight::run(&adb, Some("SER1"), false).unwrap();
    assert_eq!(pre.serial, "SER1");
    runner.assert_exhausted();
}

#[test]
fn preflight_sdk21_refused() {
    let runner = MockRunner::new(vec![
        (
            "adb devices",
            out(0, "List of devices attached\nSER1\tdevice\n"),
        ),
        (
            "adb -s SER1 shell getprop ro.build.version.sdk",
            out(0, "21\n"),
        ),
    ]);
    let adb = Adb::new(&runner, None, "adb");
    let err = preflight::run(&adb, None, false).unwrap_err();
    assert!(err.to_string().contains("SDK 21 < 24"), "{err}");
}

#[test]
fn preflight_armeabi_refused() {
    let runner = MockRunner::new(vec![
        (
            "adb devices",
            out(0, "List of devices attached\nSER1\tdevice\n"),
        ),
        (
            "adb -s SER1 shell getprop ro.build.version.sdk",
            out(0, "30\n"),
        ),
        (
            "adb -s SER1 shell getprop ro.product.cpu.abi",
            out(0, "armeabi-v7a\n"),
        ),
    ]);
    let adb = Adb::new(&runner, None, "adb");
    let err = preflight::run(&adb, None, false).unwrap_err();
    assert!(err.to_string().contains("armeabi"), "{err}");
}

#[test]
fn preflight_bbk_old_build_refused_and_override() {
    let mk = || {
        MockRunner::new(vec![
            (
                "adb devices",
                out(0, "List of devices attached\nSER1\tdevice\n"),
            ),
            (
                "adb -s SER1 shell getprop ro.build.version.sdk",
                out(0, "33\n"),
            ),
            (
                "adb -s SER1 shell getprop ro.product.cpu.abi",
                out(0, "arm64-v8a\n"),
            ),
            (
                "adb -s SER1 shell df /data/local/tmp | tail -1",
                out(0, "/dev/block/dm-1 100000000 20000000 80000000 25% /data\n"),
            ),
            (
                "adb -s SER1 shell getprop ro.product.model",
                out(0, "OnePlus 9\n"),
            ),
            (
                "adb -s SER1 shell getprop ro.product.brand",
                out(0, "OnePlus\n"),
            ),
            (
                "adb -s SER1 shell getprop ro.build.date.utc",
                out(0, "1680000000\n"),
            ), // 2023
        ])
    };
    let runner = mk();
    let adb = Adb::new(&runner, None, "adb");
    let err = preflight::run(&adb, None, false).unwrap_err();
    assert!(err.to_string().contains("phantom-process"), "{err}");

    // --allow-degraded overrides the BBK gate
    let runner2 = MockRunner::new(vec![
        (
            "adb devices",
            out(0, "List of devices attached\nSER1\tdevice\n"),
        ),
        (
            "adb -s SER1 shell getprop ro.build.version.sdk",
            out(0, "33\n"),
        ),
        (
            "adb -s SER1 shell getprop ro.product.cpu.abi",
            out(0, "arm64-v8a\n"),
        ),
        (
            "adb -s SER1 shell df /data/local/tmp | tail -1",
            out(0, "/dev/block/dm-1 100000000 20000000 80000000 25% /data\n"),
        ),
        (
            "adb -s SER1 shell getprop ro.product.model",
            out(0, "OnePlus 9\n"),
        ),
        (
            "adb -s SER1 shell getprop ro.product.brand",
            out(0, "OnePlus\n"),
        ),
        (
            "adb -s SER1 shell getprop ro.build.date.utc",
            out(0, "1680000000\n"),
        ),
        (
            "adb -s SER1 shell getprop ro.miui.version.code",
            out(0, "\n"),
        ),
        (
            "adb -s SER1 shell getprop ro.mi.os.version.code",
            out(0, "\n"),
        ),
    ]);
    let adb2 = Adb::new(&runner2, None, "adb");
    let pre = preflight::run(&adb2, None, true).unwrap();
    runner2.assert_exhausted();
    assert_eq!(pre.oem, "OnePlus");
}

#[test]
fn preflight_miui_simless_refused_with_sim_ok() {
    let base = |sim: &str| {
        MockRunner::new(vec![
            (
                "adb devices",
                out(0, "List of devices attached\nSER1\tdevice\n"),
            ),
            (
                "adb -s SER1 shell getprop ro.build.version.sdk",
                out(0, "34\n"),
            ),
            (
                "adb -s SER1 shell getprop ro.product.cpu.abi",
                out(0, "arm64-v8a\n"),
            ),
            (
                "adb -s SER1 shell df /data/local/tmp | tail -1",
                out(0, "/dev/block/dm-1 100000000 20000000 80000000 25% /data\n"),
            ),
            (
                "adb -s SER1 shell getprop ro.product.model",
                out(0, "Redmi Note 12\n"),
            ),
            (
                "adb -s SER1 shell getprop ro.product.brand",
                out(0, "Redmi\n"),
            ),
            (
                "adb -s SER1 shell getprop ro.miui.version.code",
                out(0, "14\n"),
            ),
            (
                "adb -s SER1 shell getprop ro.mi.os.version.code",
                out(0, "\n"),
            ),
            ("adb -s SER1 shell getprop gsm.sim.state", out(0, sim)),
        ])
    };
    let runner = base("\n");
    let adb = Adb::new(&runner, None, "adb");
    let err = preflight::run(&adb, None, false).unwrap_err();
    assert!(err.to_string().contains("SIM"), "{err}");

    let runner = base("LOADED\n");
    let adb = Adb::new(&runner, None, "adb");
    let pre = preflight::run(&adb, None, false).unwrap();
    assert!(pre.xiaomi);
}

// ---------- apk ----------

fn artifact(name: &str, sha: &str) -> PinnedArtifact {
    PinnedArtifact {
        name: name.into(),
        url: "https://example.invalid/x.apk".into(),
        sha256: sha.into(),
        size: 10,
    }
}

#[test]
fn apk_resolve_hash_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let apks = vec![
        artifact(
            "termux",
            &node111::util::hash::sha256_bytes(b"termux-bytes"),
        ),
        artifact(
            "termux-boot",
            &node111::util::hash::sha256_bytes(b"boot-bytes"),
        ),
        artifact(
            "termux-api",
            &node111::util::hash::sha256_bytes(b"api-bytes"),
        ),
    ];
    std::fs::write(dir.path().join("termux.apk"), b"termux-bytes").unwrap();
    std::fs::write(dir.path().join("termux-boot.apk"), b"boot-bytes").unwrap();
    std::fs::write(dir.path().join("termux-api.apk"), b"WRONG").unwrap();
    let err = apk::resolve(dir.path(), &apks).unwrap_err();
    assert!(err.to_string().contains("sha256"), "{err}");
}

#[test]
fn apk_install_foreign_signature_bail() {
    let dir = tempfile::tempdir().unwrap();
    let apks = vec![
        artifact("termux", &node111::util::hash::sha256_bytes(b"a")),
        artifact("termux-boot", &node111::util::hash::sha256_bytes(b"b")),
        artifact("termux-api", &node111::util::hash::sha256_bytes(b"c")),
    ];
    std::fs::write(dir.path().join("termux.apk"), b"a").unwrap();
    std::fs::write(dir.path().join("termux-boot.apk"), b"b").unwrap();
    std::fs::write(dir.path().join("termux-api.apk"), b"c").unwrap();
    let set = apk::resolve(dir.path(), &apks).unwrap();

    let apk_path = dir.path().join("termux.apk").to_string_lossy().into_owned();
    let runner = MockRunner::new(vec![
        (
            "adb -s SER1 shell pm path com.termux".into(),
            out(0, "package:/data/app/com.termux/base.apk\n"),
        ),
        (
            "adb -s SER1 shell pm dump com.termux | grep -m1 -E 'signatures=|Package签名'".into(),
            out(0, "signatures=[AA:BB]\n"),
        ),
        (
            format!("adb -s SER1 install -r {apk_path}"),
            out(1, "Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE]"),
        ),
    ]);
    let adb = adb_of(&runner);
    apk::collision_check(&adb).unwrap();
    let err = apk::install_all(&adb, &set).unwrap_err();
    assert!(err.to_string().contains("DIFFERENT signature"), "{err}");
}

// ---------- runas channel ----------

#[test]
fn runas_push_verified_and_tamper_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("payload.bin");
    std::fs::write(&file, b"payload-bytes").unwrap();
    let sha = node111::util::hash::sha256_bytes(b"payload-bytes");
    let path_s = file.to_string_lossy().into_owned();

    let mk_runner = |grep_ok: bool| {
        MockRunner::new(vec![
            ("adb -s SER1 shell mkdir -p /data/local/tmp/111-stage".into(), out(0, "")),
            (format!("adb -s SER1 push {path_s} /data/local/tmp/111-stage/payload.bin"), out(0, "")),
            (
                // literal on purpose: pins the exact quoting of `cut -d' '`
                format!(
                    "adb -s SER1 shell run-as com.termux sh -c 'export PREFIX=/data/data/com.termux/files/usr HOME=/data/data/com.termux/files/home PATH=/data/data/com.termux/files/usr/bin:$PATH; sha256sum /data/local/tmp/111-stage/payload.bin | cut -d'\\'' '\\'' -f1 | grep -qx {sha} && mkdir -p $(dirname /x/y) && cp /data/local/tmp/111-stage/payload.bin /x/y && echo PUSH_OK'"
                ),
                out(if grep_ok { 0 } else { 1 }, if grep_ok { "PUSH_OK\n" } else { "" }),
            ),
            ("adb -s SER1 shell rm -f /data/local/tmp/111-stage/payload.bin".into(), out(0, "")),
        ])
    };

    let runner = mk_runner(true);
    let adb = adb_of(&runner);
    runas::push_verified(&adb, &file, "/x/y", &sha).unwrap();
    runner.assert_exhausted();

    let runner = mk_runner(false);
    let adb = adb_of(&runner);
    assert!(runas::push_verified(&adb, &file, "/x/y", &sha).is_err());
}

#[test]
fn boot_script_install_and_external_apps_violation() {
    let content = b"#!/bin/sh\necho hi\n";
    let sha = node111::util::hash::sha256_bytes(content);
    let runner = MockRunner::new(vec![
        (
            ra("mkdir -p /data/data/com.termux/files/home/.termux/boot && cat > /data/data/com.termux/files/home/.termux/boot/00-111-node.sh && chmod 0755 /data/data/com.termux/files/home/.termux/boot/00-111-node.sh && echo BOOT_OK"),
            out(0, "BOOT_OK\n"),
        ),
        (
            ra(&format!("sha256sum /data/data/com.termux/files/home/.termux/boot/00-111-node.sh | cut -d' ' -f1 | grep -qx {sha} && echo HASH_OK")),
            out(0, "HASH_OK\n"),
        ),
        (
            ra("grep -q 'allow-external-apps *= *true' /data/data/com.termux/files/home/.termux/termux.properties 2>/dev/null && echo VIOLATION || echo CLEAN"),
            out(0, "VIOLATION\n"),
        ),
    ]);
    let adb = adb_of(&runner);
    runas::install_boot_script(&adb, content).unwrap();
    let err = runas::assert_no_external_apps(&adb).unwrap_err();
    assert!(err.to_string().contains("allow-external-apps"), "{err}");
    // the stdin push was recorded
    assert!(
        runner.calls().iter().any(|c| c.contains("<stdin:")),
        "{:?}",
        runner.calls()
    );
    runner.assert_exhausted();
}

// ---------- harden ----------

#[test]
fn harden_happy_and_readback_failure_note() {
    let mut script = Vec::new();
    for pkg in ["com.termux", "com.termux.api", "com.termux.boot"] {
        script.push((
            format!("adb -s SER1 shell dumpsys deviceidle whitelist +{pkg}"),
            out(0, "Added\n"),
        ));
        script.push((
            "adb -s SER1 shell dumpsys deviceidle whitelist | cat".to_string(),
            out(0, &format!("user,{pkg}\n")),
        ));
    }
    for op in [
        "RUN_IN_BACKGROUND",
        "RUN_ANY_IN_BACKGROUND",
        "SYSTEM_ALERT_WINDOW",
    ] {
        script.push((
            format!("adb -s SER1 shell appops set com.termux {op} allow"),
            out(0, ""),
        ));
        script.push((
            format!("adb -s SER1 shell appops get com.termux {op}"),
            out(0, &format!("{op}: allow\n")),
        ));
    }
    script.extend([
        (
            "adb -s SER1 shell pm grant com.termux android.permission.POST_NOTIFICATIONS".into(),
            out(0, ""),
        ),
        (
            "adb -s SER1 shell dumpsys package com.termux | grep android.permission.POST_NOTIFICATIONS".into(),
            out(0, "android.permission.POST_NOTIFICATIONS: granted=true\n"),
        ),
        (
            "adb -s SER1 shell settings put global settings_enable_monitor_phantom_procs false".into(),
            out(0, ""),
        ),
        (
            "adb -s SER1 shell settings get global settings_enable_monitor_phantom_procs".into(),
            out(0, "false\n"),
        ),
    ]);
    let runner = MockRunner::new(script);
    let adb = adb_of(&runner);
    let notes = harden::run(&adb, 34).unwrap();
    assert!(notes.is_empty(), "unexpected notes: {notes:?}");
    runner.assert_exhausted();

    // readback failure → note, not hard error
    let runner = MockRunner::new(vec![
        ("adb -s SER1 shell dumpsys deviceidle whitelist +com.termux", out(0, "Added\n")),
        ("adb -s SER1 shell dumpsys deviceidle whitelist | cat", out(0, "nothing\n")),
        ("adb -s SER1 shell dumpsys deviceidle whitelist +com.termux.api", out(0, "Added\n")),
        (
            "adb -s SER1 shell dumpsys deviceidle whitelist | cat",
            out(0, "user,com.termux.api\n"),
        ),
        ("adb -s SER1 shell dumpsys deviceidle whitelist +com.termux.boot", out(0, "Added\n")),
        (
            "adb -s SER1 shell dumpsys deviceidle whitelist | cat",
            out(0, "user,com.termux.boot\n"),
        ),
        ("adb -s SER1 shell appops set com.termux RUN_IN_BACKGROUND allow", out(0, "")),
        (
            "adb -s SER1 shell appops get com.termux RUN_IN_BACKGROUND",
            out(0, "RUN_IN_BACKGROUND: allow\n"),
        ),
        (
            "adb -s SER1 shell appops set com.termux RUN_ANY_IN_BACKGROUND allow",
            out(0, ""),
        ),
        (
            "adb -s SER1 shell appops get com.termux RUN_ANY_IN_BACKGROUND",
            out(0, "RUN_ANY_IN_BACKGROUND: allow\n"),
        ),
        ("adb -s SER1 shell appops set com.termux SYSTEM_ALERT_WINDOW allow", out(0, "")),
        (
            "adb -s SER1 shell appops get com.termux SYSTEM_ALERT_WINDOW",
            out(0, "SYSTEM_ALERT_WINDOW: allow\n"),
        ),
        (
            "adb -s SER1 shell pm grant com.termux android.permission.POST_NOTIFICATIONS",
            out(0, ""),
        ),
        (
            "adb -s SER1 shell dumpsys package com.termux | grep android.permission.POST_NOTIFICATIONS",
            out(0, "android.permission.POST_NOTIFICATIONS: granted=true\n"),
        ),
        (
            "adb -s SER1 shell settings put global settings_enable_monitor_phantom_procs false",
            out(0, ""),
        ),
        (
            "adb -s SER1 shell settings get global settings_enable_monitor_phantom_procs",
            out(0, "false\n"),
        ),
    ]);
    let adb = adb_of(&runner);
    let notes = harden::run(&adb, 34).unwrap();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("com.termux") && !notes[0].contains("api"),
        "{notes:?}"
    );
    runner.assert_exhausted();
}

#[test]
fn phantom_order_android12() {
    let runner =
        MockRunner::new(vec![
        ("adb -s SER1 shell device_config set_sync_disabled_for_tests persistent", out(0, "")),
        (
            "adb -s SER1 shell device_config put activity_manager max_phantom_processes 2147483647",
            out(0, ""),
        ),
        (
            "adb -s SER1 shell device_config get activity_manager max_phantom_processes",
            out(0, "2147483647\n"),
        ),
    ]);
    let adb = adb_of(&runner);
    let notes = harden::phantom_killer(&adb, 31);
    assert!(notes.is_empty(), "{notes:?}");
    runner.assert_exhausted();
}

// ---------- teardown ----------

#[test]
fn teardown_via_adb_full_script() {
    let mut script = vec![
        (ra("pkill -f 111node; pkill -f '111 daemon'"), out(0, "")),
        (
            ra("rm -rf $PREFIX/var/lib/111 $PREFIX/var/service/111node $HOME/.termux/boot/00-111-node.sh"),
            out(0, ""),
        ),
    ];
    for pkg in ["com.termux", "com.termux.api", "com.termux.boot"] {
        script.push((
            format!("adb -s SER1 shell dumpsys deviceidle whitelist -{pkg}"),
            out(0, "Removed\n"),
        ));
    }
    script.extend([
        (
            "adb -s SER1 shell appops set com.termux RUN_IN_BACKGROUND default".into(),
            out(0, ""),
        ),
        (
            "adb -s SER1 shell appops set com.termux RUN_ANY_IN_BACKGROUND default".into(),
            out(0, ""),
        ),
        (
            "adb -s SER1 shell appops set com.termux SYSTEM_ALERT_WINDOW default".into(),
            out(0, ""),
        ),
        (
            "adb -s SER1 uninstall com.termux".into(),
            out(0, "Success\n"),
        ),
        (
            "adb -s SER1 uninstall com.termux.api".into(),
            out(0, "Success\n"),
        ),
        (
            "adb -s SER1 uninstall com.termux.boot".into(),
            out(0, "Success\n"),
        ),
        ("adb -s SER1 forward --remove-all".into(), out(0, "")),
        ("adb -s SER1 reverse --remove-all".into(), out(0, "")),
    ]);
    let runner = MockRunner::new(script);
    let adb = adb_of(&runner);
    teardown::via_adb(&adb).unwrap();
    runner.assert_exhausted();
}

// ---------- provision state resume ----------

#[test]
fn provision_state_resume_logic() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("state.json");
    let mut s = node111::install::phone::ProvisionState::load(&marker, "SER1");
    assert!(!s.is_done("apks"));
    s.mark("apks", &marker);
    let s2 = node111::install::phone::ProvisionState::load(&marker, "SER1");
    assert!(s2.is_done("apks"));
    // different serial = fresh state
    let s3 = node111::install::phone::ProvisionState::load(&marker, "OTHER");
    assert!(!s3.is_done("apks"));
}

// ---------- run-as env (every command, whole flow) ----------

/// A permissive fake phone: every adb call succeeds with the marker its
/// caller looks for; every invocation is recorded.
#[derive(Default)]
struct FakePhone {
    calls: std::sync::Mutex<Vec<String>>,
    /// E2: sha256 tail of the last typed chunk (echoed back by the UI dump).
    tail: std::sync::Mutex<String>,
}

impl FakePhone {
    fn answer(&self, cmd: String) -> anyhow::Result<Output> {
        let stdout = if let Some(rest) = cmd.split("printf '%s' '").nth(1) {
            let chunk = rest.split("' >>").next().unwrap();
            let h = node111::util::hash::sha256_bytes(chunk.as_bytes());
            *self.tail.lock().unwrap() = h[h.len() - 8..].to_string();
            String::new()
        } else if cmd.contains("uiautomator dump") {
            self.tail.lock().unwrap().clone()
        } else if cmd.contains("identity generate") {
            "3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29\n".into()
        } else if cmd.contains("dashboard-pair.json") {
            r#"{"schema":1,"code":"7KQ2-XM9D","created":"x"}"#.into()
        } else if cmd.contains("/health") {
            r#"{"schema":1,"status":"ok"}"#.into()
        } else {
            "RA_OK BOOTSTRAP_OK EXTRACT_OK WRITE_OK HASH_OK BOOT_OK PUSH_OK DPKG_OK CLEAN \
             HOSTKEY_OK AUTHKEY_OK CFG_OK SSHD_OK START_OK Success\n"
                .into()
        };
        self.calls.lock().unwrap().push(cmd);
        Ok(Output {
            status: 0,
            stdout,
            stderr: String::new(),
        })
    }
}

impl node111::util::run::Runner for FakePhone {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        self.answer(format!("{program} {}", args.join(" ")))
    }
    fn run_as(&self, _: &str, _: &str, _: &[&str]) -> anyhow::Result<Output> {
        anyhow::bail!("the phone flow never uses host sudo")
    }
    fn run_stdin(&self, program: &str, args: &[&str], _: &[u8]) -> anyhow::Result<Output> {
        self.answer(format!("{program} {}", args.join(" ")))
    }
}

/// Every `run-as` command the phone flow issues carries the Termux env, and
/// reaches run-as's `sh -c` as ONE intact, syntactically valid command (the
/// device shell's parse is reproduced with the host `sh`).
#[test]
fn every_run_as_command_carries_the_termux_env() {
    use node111::install::phone::{start, verify};
    let phone = FakePhone::default();
    let adb = Adb::new(&phone, Some("SER1".into()), "adb");
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("payload.tar.gz");
    std::fs::write(&payload, b"payload").unwrap();
    let deb = dir.path().join("openssh.deb");
    std::fs::write(&deb, b"deb-bytes").unwrap();
    let deb_sha = node111::util::hash::sha256_bytes(b"deb-bytes");

    // step 4: headless channel (run-as path, then the E2 fallback's exec)
    runas::wait_bootstrap(&adb, 5).unwrap();
    assert!(runas::run_as_works(&adb));
    runas::install_boot_script(&adb, b"#!/bin/sh\n").unwrap();
    runas::push_verified(
        &adb,
        &payload,
        "$PREFIX/var/lib/111/releases/payload.tar.gz",
        &node111::util::hash::sha256_bytes(b"payload"),
    )
    .unwrap();
    runas::run_as_output(
        &adb,
        "mkdir -p $PREFIX/var/lib/111/releases/v && echo EXTRACT_OK",
    )
    .unwrap();
    runas::install_file(&adb, b"X=1\n", "$PREFIX/var/lib/111/profile.env", false).unwrap();
    runas::install_file(
        &adb,
        b"#!/bin/sh\n",
        "$PREFIX/var/lib/111/sv/111node/run",
        true,
    )
    .unwrap();
    runas::install_debs(&adb, dir.path(), &[artifact("openssh", &deb_sha)]).unwrap();
    runas::assert_no_external_apps(&adb).unwrap();
    runas::e2e_inject_stub(&adb, b"#!/bin/sh\necho stub\n").unwrap();
    // steps 6-7: start + verify + pair code
    start::on_device_keygen(&adb).unwrap();
    start::setup_sshd(&adb, "ssh-ed25519 AAAAC3Nz test@host").unwrap();
    start::start_node(&adb).unwrap();
    verify::on_device_health(&adb).unwrap();
    assert_eq!(start::read_pair_code(&adb).as_deref(), Some("7KQ2-XM9D"));
    // step 9: teardown
    teardown::via_adb(&adb).unwrap();

    let calls = phone.calls.lock().unwrap().clone();
    let run_as: Vec<&String> = calls.iter().filter(|c| c.contains("run-as")).collect();
    assert!(
        run_as.len() >= 20,
        "{} run-as calls: {run_as:#?}",
        run_as.len()
    );
    let head = "adb -s SER1 shell run-as com.termux sh -c ";
    for c in run_as {
        let quoted = c
            .strip_prefix(head)
            .unwrap_or_else(|| panic!("not a wrapped run-as call: {c}"));
        // what run-as's `sh -c` receives: the device shell's parse of `quoted`
        let parsed = std::process::Command::new("sh")
            .args(["-c", &format!("printf '%s\\n' {quoted}")])
            .output()
            .unwrap();
        let text = String::from_utf8(parsed.stdout).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1, "split into several words: {c}");
        let inner = lines[0];
        assert!(
            inner.starts_with(&format!("{TERMUX_ENV} ")),
            "missing the Termux env: {inner}"
        );
        let syntax = std::process::Command::new("sh")
            .args(["-n", "-c", inner])
            .status()
            .unwrap();
        assert!(syntax.success(), "inner command is not valid sh: {inner}");
    }
}

// ---------- step 4's service files: the Termux cron seam removed (Layer 9 §3.3 rule 6) ----------

/// The cron script's path on a phone an earlier provisioner pushed it to (spelled out here,
/// not borrowed from the crate).
const OLD_CRON_SCRIPT: &str = "$PREFIX/var/lib/111/share/termux/cron-healthcheck.sh";

/// A file of `1-bootstrap/termux/` as the tree holds it: what step 4 pushes, byte for byte.
fn termux_asset(rel: &str) -> Vec<u8> {
    std::fs::read(format!("{}/../../termux/{rel}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// The two run-as calls of one `runas::install_file`: the write (on stdin), then the hash probe.
fn install_file_script(content: &[u8], remote: &str, executable: bool) -> Vec<(String, Output)> {
    let chmod = if executable {
        format!("&& chmod 0755 {remote}")
    } else {
        String::new()
    };
    let sha = node111::util::hash::sha256_bytes(content);
    vec![
        (
            ra(&format!(
                "mkdir -p $(dirname {remote}) && cat > {remote} {chmod} && echo WRITE_OK"
            )),
            out(0, "WRITE_OK\n"),
        ),
        (
            ra(&format!(
                "sha256sum {remote} | cut -d' ' -f1 | grep -qx {sha} && echo HASH_OK"
            )),
            out(0, "HASH_OK\n"),
        ),
    ]
}

/// Step 4's service files up to the delete: the profile, runit's `run` and its `finish` hook.
fn service_files_script() -> Vec<(String, Output)> {
    let mut script = install_file_script(
        &termux_asset("profile.env"),
        "$PREFIX/var/lib/111/profile.env",
        false,
    );
    script.extend(install_file_script(
        &termux_asset("sv/111node/run"),
        "$PREFIX/var/lib/111/sv/111node/run",
        true,
    ));
    script.extend(install_file_script(
        &termux_asset("sv/111node/finish"),
        "$PREFIX/var/lib/111/sv/111node/finish",
        true,
    ));
    script
}

/// The exact transcript: three files written, none of them the cron script (a push of it would
/// be an unexpected call), then the one delete (missing, the script is left unexhausted); a
/// delete that fails fails the step, so its re-run resumes there.
#[test]
fn service_files_push_no_cron_script_and_delete_an_earlier_copy() {
    let mut script = service_files_script();
    script.push((
        ra(&format!("rm -f {OLD_CRON_SCRIPT} && echo RM_OK")),
        out(0, "RM_OK\n"),
    ));
    let runner = MockRunner::new(script);
    node111::install::phone::install_service_files(&adb_of(&runner)).unwrap();
    runner.assert_exhausted();
    let calls = runner.calls();
    let writes: Vec<&String> = calls.iter().filter(|c| c.contains("<stdin:")).collect();
    assert_eq!(writes.len(), 3, "{writes:#?}");
    assert!(writes.iter().all(|c| !c.contains("cron")), "{writes:#?}");
    let cron: Vec<&String> = calls.iter().filter(|c| c.contains("cron")).collect();
    assert_eq!(cron.len(), 1, "{cron:#?}");
    assert!(
        cron[0].contains(&format!("rm -f {OLD_CRON_SCRIPT}")),
        "{cron:#?}"
    );

    let mut script = service_files_script();
    script.push((
        ra(&format!("rm -f {OLD_CRON_SCRIPT} && echo RM_OK")),
        out(1, ""),
    ));
    let runner = MockRunner::new(script);
    let err = node111::install::phone::install_service_files(&adb_of(&runner)).unwrap_err();
    assert!(err.to_string().contains("run-as command failed"), "{err}");
    runner.assert_exhausted();
}

/// A phone whose Termux tree is a temp dir: each run-as command is parsed as the device shell
/// parses it (the host `sh`, as above), its Termux env re-pointed at the temp dir, and run by
/// the host `sh` with the call's stdin. Anything but a run-as call is refused.
struct SandboxPhone {
    root: tempfile::TempDir,
    calls: std::sync::Mutex<Vec<String>>,
}

impl SandboxPhone {
    fn new() -> Self {
        SandboxPhone {
            root: tempfile::tempdir().unwrap(),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// `$PREFIX/var/lib/111` inside the sandbox.
    fn lib(&self) -> std::path::PathBuf {
        self.root.path().join("usr/var/lib/111")
    }

    fn exec(&self, program: &str, args: &[&str], input: &[u8]) -> anyhow::Result<Output> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let cmd = format!("{program} {}", args.join(" "));
        self.calls.lock().unwrap().push(cmd.clone());
        let quoted = cmd
            .strip_prefix("adb -s SER1 shell run-as com.termux sh -c ")
            .ok_or_else(|| anyhow::anyhow!("not a run-as call: {cmd}"))?;
        let parsed = Command::new("sh")
            .args(["-c", &format!("printf '%s\\n' {quoted}")])
            .output()?;
        let text = String::from_utf8(parsed.stdout)?;
        let lines: Vec<&str> = text.lines().collect();
        anyhow::ensure!(lines.len() == 1, "split into several words: {cmd}");
        let body = lines[0]
            .strip_prefix(TERMUX_ENV)
            .ok_or_else(|| anyhow::anyhow!("missing the Termux env: {}", lines[0]))?;
        let env = format!(
            "export PREFIX={} HOME={};",
            self.root.path().join("usr").display(),
            self.root.path().join("home").display()
        );
        let mut child = Command::new("sh")
            .args(["-c", &format!("{env}{body}")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().unwrap().write_all(input)?;
        let done = child.wait_with_output()?;
        Ok(Output {
            status: done.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&done.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&done.stderr).into_owned(),
        })
    }
}

impl node111::util::run::Runner for SandboxPhone {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        self.exec(program, args, b"")
    }
    fn run_as(&self, _: &str, _: &str, _: &[&str]) -> anyhow::Result<Output> {
        anyhow::bail!("the phone flow never uses host sudo")
    }
    fn run_stdin(&self, program: &str, args: &[&str], input: &[u8]) -> anyhow::Result<Output> {
        self.exec(program, args, input)
    }
}

/// End to end on a sandboxed Termux tree: the next run deletes the cron script an earlier
/// provisioner pushed and lands the service files byte for byte; a later run, and a phone that
/// never had the script, see the delete succeed with nothing to do and no script pushed.
#[test]
fn a_cron_script_pushed_earlier_is_deleted_on_the_next_run() {
    use std::os::unix::fs::PermissionsExt;
    let phone = SandboxPhone::new();
    let adb = Adb::new(&phone, Some("SER1".into()), "adb");
    let old = phone.lib().join("share/termux/cron-healthcheck.sh");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    // the copy a provisioner before Layer 9 left: the script's bytes, executable
    std::fs::write(
        &old,
        b"#!/data/data/com.termux/files/usr/bin/sh\ncurl -fsS -m 10 http://127.0.0.1:11180/health\n",
    )
    .unwrap();
    std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();

    node111::install::phone::install_service_files(&adb).unwrap();
    assert!(!old.exists(), "the copy pushed earlier survived the run");
    for (rel, executable) in [
        ("profile.env", false),
        ("sv/111node/run", true),
        ("sv/111node/finish", true),
    ] {
        let got = phone.lib().join(rel);
        assert_eq!(std::fs::read(&got).unwrap(), termux_asset(rel), "{rel}");
        if executable {
            let mode = std::fs::metadata(&got).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755, "{rel}");
        }
    }

    // a later run: nothing to delete, still a success, nothing pushed back
    node111::install::phone::install_service_files(&adb).unwrap();
    assert!(!old.exists());
    let calls = phone.calls.lock().unwrap().clone();
    // per run: three writes, three hash probes, one delete
    assert_eq!(calls.len(), 14, "{calls:#?}");
    let cron: Vec<&String> = calls.iter().filter(|c| c.contains("cron")).collect();
    assert_eq!(cron.len(), 2, "{cron:#?}");
    assert!(cron.iter().all(|c| c.contains("rm -f ")), "{cron:#?}");

    // a phone that never had the script: the delete has nothing to do, nothing lands under share/
    let fresh = SandboxPhone::new();
    node111::install::phone::install_service_files(&Adb::new(&fresh, Some("SER1".into()), "adb"))
        .unwrap();
    assert!(fresh.lib().join("sv/111node/finish").exists());
    assert!(!fresh.lib().join("share").exists());
}

/// The four calls of one `runas::push_verified`: the stage dir, the push, the on-device hash
/// check and copy, the staged copy removed.
fn push_verified_script(local: &std::path::Path, remote: &str, sha: &str) -> Vec<(String, Output)> {
    let stage = "/data/local/tmp/111-stage";
    let staged = format!("{stage}/{}", local.file_name().unwrap().to_string_lossy());
    vec![
        (format!("adb -s SER1 shell mkdir -p {stage}"), out(0, "")),
        (
            format!("adb -s SER1 push {} {staged}", local.to_string_lossy()),
            out(0, ""),
        ),
        (
            ra(&format!(
                "sha256sum {staged} | cut -d' ' -f1 | grep -qx {sha} && mkdir -p $(dirname {remote}) && cp {staged} {remote} && echo PUSH_OK"
            )),
            out(0, "PUSH_OK\n"),
        ),
        (format!("adb -s SER1 shell rm -f {staged}"), out(0, "")),
    ]
}

/// A host cache holding what step 4 pushes, with their pins: the aarch64 payload and one `.deb`.
fn step_4_cache() -> (tempfile::TempDir, PinnedArtifact, Vec<PinnedArtifact>) {
    let cache = tempfile::tempdir().unwrap();
    std::fs::write(
        cache.path().join("payload-aarch64.tar.gz"),
        b"payload-bytes",
    )
    .unwrap();
    std::fs::write(cache.path().join("openssh.deb"), b"deb-bytes").unwrap();
    let payload_pin = artifact(
        "aarch64-unknown-linux-musl",
        &node111::util::hash::sha256_bytes(b"payload-bytes"),
    );
    let debs = vec![artifact(
        "openssh",
        &node111::util::hash::sha256_bytes(b"deb-bytes"),
    )];
    (cache, payload_pin, debs)
}

/// Step 4 whole on the run-as channel for [`step_4_cache`]'s cache and release 2026.11.16, the
/// exact transcript `phone::headless_channel` must issue: the bootstrap wait, the boot script,
/// the payload and its extract, the three service files and the one delete, then, given
/// `record` (the bytes of the APK step's record), its three calls, then the debs and the guard.
fn step_4_script(
    cache: &std::path::Path,
    payload_pin: &PinnedArtifact,
    debs: &[PinnedArtifact],
    record: Option<&[u8]>,
) -> Vec<(String, Output)> {
    let boot = "/data/data/com.termux/files/home/.termux/boot/00-111-node.sh";
    let boot_sha = node111::util::hash::sha256_bytes(&termux_asset("boot/00-111-node.sh"));
    let rel = "$PREFIX/var/lib/111/releases";
    let deb_remote = "/data/data/com.termux/files/usr/var/lib/111/debs/openssh.deb";

    let mut script = vec![
        (
            "adb -s SER1 shell am start -n com.termux/.app.TermuxActivity".to_string(),
            out(0, ""),
        ),
        (
            ra("dpkg -l >/dev/null 2>&1 && test -d /data/data/com.termux/files/usr/etc && echo BOOTSTRAP_OK"),
            out(0, "BOOTSTRAP_OK\n"),
        ),
        (ra("echo RA_OK"), out(0, "RA_OK\n")),
        (
            ra(&format!(
                "mkdir -p /data/data/com.termux/files/home/.termux/boot && cat > {boot} && chmod 0755 {boot} && echo BOOT_OK"
            )),
            out(0, "BOOT_OK\n"),
        ),
        (
            ra(&format!(
                "sha256sum {boot} | cut -d' ' -f1 | grep -qx {boot_sha} && echo HASH_OK"
            )),
            out(0, "HASH_OK\n"),
        ),
    ];
    script.extend(push_verified_script(
        &cache.join("payload-aarch64.tar.gz"),
        &format!("{rel}/payload.tar.gz"),
        &payload_pin.sha256,
    ));
    script.push((
        ra(&format!(
            "mkdir -p {rel}/2026.11.16 && tar -xzf {rel}/payload.tar.gz -C {rel}/2026.11.16 && ln -sfn {rel}/2026.11.16 $PREFIX/var/lib/111/current && rm {rel}/payload.tar.gz && echo EXTRACT_OK"
        )),
        out(0, "EXTRACT_OK\n"),
    ));
    script.extend(service_files_script());
    script.push((
        ra(&format!("rm -f {OLD_CRON_SCRIPT} && echo RM_OK")),
        out(0, "RM_OK\n"),
    ));
    if let Some(record) = record {
        script.extend(apk_record_script(record));
    }
    script.extend(push_verified_script(
        &cache.join("openssh.deb"),
        deb_remote,
        &debs[0].sha256,
    ));
    script.extend([
        (
            ra(&format!("dpkg -i {deb_remote} && echo DPKG_OK")),
            out(0, "DPKG_OK\n"),
        ),
        (
            ra("grep -q 'allow-external-apps *= *true' /data/data/com.termux/files/home/.termux/termux.properties 2>/dev/null && echo VIOLATION || echo CLEAN"),
            out(0, "CLEAN\n"),
        ),
    ]);
    script
}

/// Step 4 whole on the run-as channel, as `phone::run` calls it: the exact transcript puts the
/// three service files and the one delete after the payload's extract and before the debs, so a
/// step 4 that drops them, or moves them out of the run-as branch, fails here (`run` itself needs
/// a real adb and the network: the owner's real-device run covers it). Without the pins of an
/// APK step (a run resuming a marker an earlier provisioner wrote) it writes no record.
#[test]
fn step_4_writes_the_service_files_and_deletes_the_cron_script_before_the_debs() {
    let (cache, payload_pin, debs) = step_4_cache();
    let runner = MockRunner::new(step_4_script(cache.path(), &payload_pin, &debs, None));
    node111::install::phone::headless_channel(
        &adb_of(&runner),
        &cache.path().join("payload-aarch64.tar.gz"),
        &payload_pin,
        "2026.11.16".parse().unwrap(),
        cache.path(),
        &debs,
        None,
    )
    .unwrap();
    runner.assert_exhausted();
    let calls = runner.calls();
    let writes: Vec<&String> = calls.iter().filter(|c| c.contains("<stdin:")).collect();
    // the boot script and the three service files; no write names the cron script
    assert_eq!(writes.len(), 4, "{writes:#?}");
    assert!(writes.iter().all(|c| !c.contains("cron")), "{writes:#?}");
    assert!(calls.iter().all(|c| !c.contains("apk.json")), "{calls:#?}");
}

// ---------- the APK step's record: state/apk.json (Layer 9 §12.6) ----------

/// Where the record lands in the Termux root, and the temp file it lands through (spelled out
/// here, not borrowed from the crate).
const APK_RECORD: &str = "$PREFIX/var/lib/111/state/apk.json";
const APK_RECORD_TMP: &str = "$PREFIX/var/lib/111/state/.apk.json.tmp";

/// The three pinned APKs (`apk::APK_PACKAGES`, spelled out): package, manifest name, the
/// versionName the post-install probe reads, and the bytes of this test's fake APK.
const APKS: [(&str, &str, &str, &[u8]); 3] = [
    ("com.termux", "termux", "0.118.3", b"termux-bytes"),
    ("com.termux.boot", "termux-boot", "0.8.1", b"boot-bytes"),
    ("com.termux.api", "termux-api", "0.53.0", b"api-bytes"),
];

/// The record of [`APKS`]' pins, byte for byte; `termux/tests/dryrun.sh` replays this one.
const APK_RECORD_BYTES: &str = concat!(
    r#"{"schema":1,"apk":{"#,
    r#""com.termux":"5d994501492ad52262147b765f6b51344bc4221a5bfe5015a5054046ffcb9857","#,
    r#""com.termux.api":"1bbdd4e8285ec36bff0487be380941f7519871c4bad07083cb9c9fa9bd0d92cf","#,
    r#""com.termux.boot":"b289a36d7a2cfb5f957b5bd795b61d3a6f7d3ddb8699be01f81547c8a0cc9343""#,
    "}}\n"
);

/// [`APKS`]' pins, as the APK step returns them.
fn apk_pins() -> node111::install::phone::ApkPins {
    APKS.iter()
        .map(|(package, _, _, fake)| (package.to_string(), node111::util::hash::sha256_bytes(fake)))
        .collect()
}

/// [`APKS`] written into `cache` as the prefetch leaves them, and their manifest pins.
fn apk_cache(cache: &std::path::Path) -> Vec<PinnedArtifact> {
    APKS.iter()
        .map(|(_, name, _, fake)| {
            std::fs::write(cache.join(format!("{name}.apk")), fake).unwrap();
            artifact(name, &node111::util::hash::sha256_bytes(fake))
        })
        .collect()
}

/// The APK step's exact transcript for [`apk_cache`]'s cache: per APK, `adb install -r`, then
/// its versionName probe.
fn apk_step_script(cache: &std::path::Path) -> Vec<(String, Output)> {
    let mut script = Vec::new();
    for (package, name, version, _) in APKS {
        script.push((
            format!(
                "adb -s SER1 install -r {}",
                cache.join(format!("{name}.apk")).to_string_lossy()
            ),
            out(0, "Performing Streamed Install\nSuccess\n"),
        ));
        script.push((
            format!("adb -s SER1 shell dumpsys package {package} | grep -m1 versionName"),
            out(0, &format!("    versionName={version}\n")),
        ));
    }
    script
}

/// The three run-as calls of the record: the write to the temp file (on stdin) and its hash
/// probe (`runas::install_file`), then 0600 and the rename onto `state/apk.json`.
fn apk_record_script(record: &[u8]) -> Vec<(String, Output)> {
    let mut script = install_file_script(record, APK_RECORD_TMP, false);
    script.push((
        ra(&format!(
            "chmod 0600 {APK_RECORD_TMP} && mv -f {APK_RECORD_TMP} {APK_RECORD} && echo APK_RECORD_OK"
        )),
        out(0, "APK_RECORD_OK\n"),
    ));
    script
}

/// Step 3 then step 4 on one exact transcript, as `phone::run` sequences them: the APK step
/// installs the three APKs and returns their pins, writing nothing to the phone; step 4 then
/// pushes the record of exactly those pins over run-as, after the service files and before the
/// debs (its hash probe pins the bytes written).
#[test]
fn the_apk_record_is_pushed_after_the_apk_step() {
    let (cache, payload_pin, debs) = step_4_cache();
    let apks = apk_cache(cache.path());
    let mut script = apk_step_script(cache.path());
    script.extend(step_4_script(
        cache.path(),
        &payload_pin,
        &debs,
        Some(APK_RECORD_BYTES.as_bytes()),
    ));
    let runner = MockRunner::new(script);
    let adb = adb_of(&runner);

    let pins = node111::install::phone::install_apks(&adb, cache.path(), &apks).unwrap();
    assert_eq!(pins, apk_pins());
    let step_3 = runner.calls();
    assert_eq!(step_3.len(), 6, "{step_3:#?}");
    assert!(step_3.iter().all(|c| !c.contains("run-as")), "{step_3:#?}");

    node111::install::phone::headless_channel(
        &adb,
        &cache.path().join("payload-aarch64.tar.gz"),
        &payload_pin,
        "2026.11.16".parse().unwrap(),
        cache.path(),
        &debs,
        Some(&pins),
    )
    .unwrap();
    runner.assert_exhausted();
    let calls = runner.calls();
    let last_install = calls
        .iter()
        .rposition(|c| c.contains(" install -r "))
        .unwrap();
    let first_record = calls.iter().position(|c| c.contains("apk.json")).unwrap();
    assert!(last_install < first_record, "{calls:#?}");
    let writes: Vec<&String> = calls.iter().filter(|c| c.contains("<stdin:")).collect();
    // the boot script, the three service files, then the record: its bytes and nothing else
    assert_eq!(writes.len(), 5, "{writes:#?}");
    assert!(
        writes[4].contains(".apk.json.tmp")
            && writes[4].ends_with(&format!("<stdin:{}B>", APK_RECORD_BYTES.len())),
        "{writes:#?}"
    );
}

/// No record without a completed APK step: an install that fails ends the step with no pins and
/// nothing written to the phone (`run` stops there, before step 4).
#[test]
fn a_failed_apk_step_records_nothing() {
    let cache = tempfile::tempdir().unwrap();
    let apks = apk_cache(cache.path());
    let mut script = apk_step_script(cache.path());
    script.truncate(2);
    script.push((
        format!(
            "adb -s SER1 install -r {}",
            cache.path().join("termux-boot.apk").to_string_lossy()
        ),
        out(1, "Failure [INSTALL_FAILED_USER_RESTRICTED]\n"),
    ));
    let runner = MockRunner::new(script);
    let err =
        node111::install::phone::install_apks(&adb_of(&runner), cache.path(), &apks).unwrap_err();
    assert!(err.to_string().contains("com.termux.boot"), "{err}");
    runner.assert_exhausted();
    let calls = runner.calls();
    assert!(
        calls
            .iter()
            .all(|c| !c.contains("run-as") && !c.contains("apk.json")),
        "{calls:#?}"
    );
}

/// The record's bytes: `{"schema":1,"apk":{<package>:<sha256>}}` and a newline, the packages in
/// byte order, parsing back to the pins the APK step installed; anything but one to three pinned
/// packages, each with a 64-lowercase-hex pin, is refused before anything reaches the phone.
#[test]
fn the_apk_record_bytes_and_its_refusals() {
    use node111::install::phone::{apk_record, push_apk_record, ApkPins};
    let bytes = apk_record(&apk_pins()).unwrap();
    assert_eq!(bytes, APK_RECORD_BYTES.as_bytes());
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["schema"], 1);
    let apk = v["apk"].as_object().unwrap();
    assert_eq!(apk.len(), 3);
    for (package, _, _, fake) in APKS {
        assert_eq!(apk[package], node111::util::hash::sha256_bytes(fake));
    }

    let pin = node111::util::hash::sha256_bytes(b"x");
    let with = |package: &str, sha: &str| {
        let mut pins = apk_pins();
        pins.remove("com.termux.api");
        pins.insert(package.to_string(), sha.to_string());
        pins
    };
    for (why, pins) in [
        ("no package", ApkPins::new()),
        ("a package that is not pinned", with("com.example", &pin)),
        ("a short pin", with("com.termux.api", &pin[..63])),
        ("a long pin", with("com.termux.api", &format!("{pin}0"))),
        (
            "an uppercase pin",
            with("com.termux.api", &pin.to_uppercase()),
        ),
        (
            "a pin that is not hex",
            with("com.termux.api", &"g".repeat(64)),
        ),
    ] {
        assert!(apk_record(&pins).is_err(), "{why}");
        let runner = MockRunner::new(Vec::<(String, Output)>::new());
        assert!(push_apk_record(&adb_of(&runner), &pins).is_err(), "{why}");
        assert!(runner.calls().is_empty(), "{why}");
    }
}

/// On a sandboxed Termux tree: the record lands as `state/apk.json`, its bytes and 0600, in three
/// run-as calls, `state/` created on a phone whose node never started and no temp file left; a
/// later run replaces it whole and 0600 again, also when a killed run left a world-readable temp
/// file behind (written with no `chmod`, the record would keep that file's 0644).
#[test]
fn the_apk_record_lands_as_state_apk_json_0600() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let phone = SandboxPhone::new();
    let adb = Adb::new(&phone, Some("SER1".into()), "adb");
    let state = phone.lib().join("state");
    let record = state.join("apk.json");
    let temp = state.join(".apk.json.tmp");
    assert!(!state.exists());

    node111::install::phone::push_apk_record(&adb, &apk_pins()).unwrap();
    assert_eq!(std::fs::read(&record).unwrap(), APK_RECORD_BYTES.as_bytes());
    assert_eq!(mode(&record), 0o600);
    assert!(!temp.exists());
    assert_eq!(phone.calls.lock().unwrap().len(), 3);

    std::fs::write(&temp, b"half a rec").unwrap();
    for p in [&record, &temp] {
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    let newer_api = node111::util::hash::sha256_bytes(b"api-bytes-0.54.0");
    let mut newer = apk_pins();
    newer.insert("com.termux.api".into(), newer_api.clone());
    node111::install::phone::push_apk_record(&adb, &newer).unwrap();
    let want =
        APK_RECORD_BYTES.replace(&node111::util::hash::sha256_bytes(b"api-bytes"), &newer_api);
    assert_eq!(std::fs::read_to_string(&record).unwrap(), want);
    assert_eq!(mode(&record), 0o600);
    let left: Vec<String> = std::fs::read_dir(&state)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["apk.json"], "no temp file left");
}

/// The pins step 3 installed survive a resume: saved with its marker and read back by the run
/// that resumes it; a marker without pins (an earlier provisioner's, or one before step 3) is
/// saved without the key and read with none; a marker whose pins are not a valid record is a
/// fresh state, every step redone.
#[test]
fn the_apk_pins_survive_a_resume() {
    use node111::install::phone::ProvisionState;
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("phone-provision-SER1.json");

    let mut s = ProvisionState::load(&marker, "SER1");
    assert!(s.apk_pins().is_none());
    s.mark("collision", &marker);
    assert!(!std::fs::read_to_string(&marker)
        .unwrap()
        .contains("\"apk\""));
    s.set_apk_pins(apk_pins());
    s.mark("apks", &marker);
    let resumed = ProvisionState::load(&marker, "SER1");
    assert!(resumed.is_done("apks"));
    assert_eq!(resumed.apk_pins(), Some(&apk_pins()));
    assert!(ProvisionState::load(&marker, "SER2").apk_pins().is_none());

    std::fs::write(
        &marker,
        r#"{"serial":"SER1","done":["collision","apks","channel"]}"#,
    )
    .unwrap();
    let earlier = ProvisionState::load(&marker, "SER1");
    assert!(earlier.is_done("channel"));
    assert!(earlier.apk_pins().is_none());

    std::fs::write(
        &marker,
        r#"{"serial":"SER1","done":["collision","apks"],"apk":{"com.termux":"not-a-pin"}}"#,
    )
    .unwrap();
    let fresh = ProvisionState::load(&marker, "SER1");
    assert!(!fresh.is_done("apks"));
    assert!(fresh.apk_pins().is_none());
}

/// What run-as runs of one recorded `adb shell run-as` call: the device shell's parse of the
/// call's text (the host `sh`, as [`SandboxPhone`] does), the Termux env and its space removed.
fn run_as_inner(call: &str) -> String {
    let quoted = call
        .strip_prefix("adb -s SER1 shell run-as com.termux sh -c ")
        .unwrap_or_else(|| panic!("not a run-as call: {call}"));
    let parsed = std::process::Command::new("sh")
        .args(["-c", &format!("printf '%s\\n' {quoted}")])
        .output()
        .unwrap();
    let text = String::from_utf8(parsed.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1, "split into several words: {call}");
    lines[0]
        .strip_prefix(&format!("{TERMUX_ENV} "))
        .unwrap_or_else(|| panic!("missing the Termux env: {}", lines[0]))
        .to_string()
}

/// G19 (11)'s dry-run half: `termux/tests/dryrun.sh` replays the record on its fake Termux tree,
/// and it must be this provisioner's own: the record of [`APKS`] (`REC`), then the three commands
/// run-as runs (`REC_WRITE`, `REC_HASH`, `REC_LAND`), each quoted as the harness assigns it, in
/// the order the provisioner sends them.
#[test]
fn the_dry_run_replays_the_provisioners_apk_record() {
    let dry = std::fs::read_to_string(format!(
        "{}/../../termux/tests/dryrun.sh",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let phone = SandboxPhone::new();
    node111::install::phone::push_apk_record(
        &Adb::new(&phone, Some("SER1".into()), "adb"),
        &apk_pins(),
    )
    .unwrap();
    let calls = phone.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 3, "{calls:#?}");
    let rec = format!(
        "REC={}",
        runas::sh_quote(APK_RECORD_BYTES.trim_end_matches('\n'))
    );
    let mut at = dry
        .find(&rec)
        .unwrap_or_else(|| panic!("dryrun.sh lacks the record: {rec}"))
        + rec.len();
    for (name, call) in ["REC_WRITE", "REC_HASH", "REC_LAND"].iter().zip(&calls) {
        let line = format!("{name}={}", runas::sh_quote(&run_as_inner(call)));
        let found = dry[at..]
            .find(&line)
            .unwrap_or_else(|| panic!("dryrun.sh lacks, after the line before it: {line}"));
        at += found + line.len();
    }
}
