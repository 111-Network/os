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
