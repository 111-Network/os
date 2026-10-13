//! T5 acceptance: Linux installer state machine end-to-end against a fixture
//! HTTP server + scripted MockRunner + temp fs roots — happy path + idempotent
//! re-run, downgrade refusal, degraded container path, clock-skew refusal.

mod common;

use node111::calver::Calver;
use node111::daemon::DaemonLayout;
use node111::install::context::InstallContext;
use node111::install::receipt::Receipt;
use node111::install::{run_with, supervise};
use node111::testkit::{MapEnv, MockRunner};
use node111::util::run::Output;
use std::path::Path;
use std::time::Duration;

fn out(status: i32, stdout: &str) -> Output {
    Output {
        status,
        stdout: stdout.to_string(),
        stderr: String::new(),
    }
}

const RELEASE: &str = "2026.10.12";

/// Build a served dir: signed manifest + payload + rns-lock + health + 204.
fn served_dir(release: &str) -> (tempfile::TempDir, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let payload = common::build_payload();
    let rns_lock = b"rns==1.5.6 --hash=sha256:abcdef\n".to_vec();
    std::fs::write(dir.path().join("rns-lock.txt"), &rns_lock).unwrap();
    std::fs::write(
        dir.path().join("health"),
        r#"{"schema":1,"status":"ok","components":{"identity":true,"kb":true,"transport":"absent","updater_idle":true}}"#,
    )
    .unwrap();
    let manifest = format!(
        r#"manifest-version = 1
date = "{release}"
channel = "stable"
release = "{release}"
min-upgrade-from = "2026.10.01"

[pkg.111.target.x86_64-unknown-linux-musl]
url = "https://example.invalid/111"
sha256 = "{a64}"
size = 100

[payload.x86_64-unknown-linux-musl]
url = "PAYLOAD_URL"
sha256 = "{psha}"
size = {plen}

[rns-lock]
url = "RNS_URL"
sha256 = "{rsha}"
size = {rlen}

[kb]
url = "https://example.invalid/kb.tar.gz"
sha256 = "{a64}"
size = 100

[[keychain]]
generation = 0
pubkey = "RWRoot00000000000000000000000000000000000000000="

[[keychain]]
generation = 1
pubkey = "RWGen100000000000000000000000000000000000000000="
signature_by_previous = "untrusted comment: fixture\\nplaceholder"
"#,
        a64 = "a".repeat(64),
        psha = node111::util::hash::sha256_bytes(&payload),
        plen = payload.len(),
        rsha = node111::util::hash::sha256_bytes(&rns_lock),
        rlen = rns_lock.len()
    );
    std::fs::write(dir.path().join("payload.tar.gz"), &payload).unwrap();
    std::fs::write(dir.path().join("channel-111-stable.toml"), &manifest).unwrap();
    (dir, payload)
}

fn sign_served(dir: &Path, port: u16) {
    let p = dir.join("channel-111-stable.toml");
    let m = std::fs::read_to_string(&p)
        .unwrap()
        .replace(
            "PAYLOAD_URL",
            &format!("http://127.0.0.1:{port}/payload.tar.gz"),
        )
        .replace("RNS_URL", &format!("http://127.0.0.1:{port}/rns-lock.txt"));
    std::fs::write(&p, m).unwrap();
    common::sign_file(&common::minisign_fixture_key(), &p);
}

fn base_env(distro: &str) -> MapEnv {
    MapEnv::new()
        .dir("/run/systemd/system", &["control"])
        .file("/proc/1/comm", "systemd\n")
        .file(
            "/proc/net/route",
            "Iface\tDestination\tGateway\neth0\t00000000\t0101A8C0\n",
        )
        .cmd("uname -m", "x86_64\n")
        .file(
            "/proc/cpuinfo",
            "processor\t: 0\nprocessor\t: 1\nprocessor\t: 2\nprocessor\t: 3\n",
        )
        .file("/proc/meminfo", "MemTotal:        8167848 kB\n")
        .file("/etc/os-release", &format!("ID={distro}\n"))
        .cmd("systemd-detect-virt --container", "none\n")
        .file("/proc/version", "Linux version 6.1.0\n")
        .dir("/sys/class/power_supply", &["AC"])
        .dir("/sys/class/net", &["lo"])
        .dir("/sys/class/bluetooth", &[])
        .dir("/sys/bus/usb/devices", &[])
        .dir("/sys/bus/pci/devices", &[])
}

struct Flow {
    _served: tempfile::TempDir,
    _node: tempfile::TempDir,
    fs_root: tempfile::TempDir,
    port: u16,
    layout: DaemonLayout,
}

fn setup(date_header: Option<String>) -> Flow {
    let (served, _) = served_dir(RELEASE);
    let port = common::serve_dir(served.path().to_path_buf(), date_header);
    sign_served(served.path(), port);
    let node = tempfile::tempdir().unwrap();
    let fs_root = tempfile::tempdir().unwrap();
    let layout = DaemonLayout {
        root: node.path().join("var/lib/111"),
        etc: fs_root.path().join("etc/111"),
    };
    std::fs::create_dir_all(&layout.root).unwrap();
    Flow {
        _served: served,
        _node: node,
        fs_root,
        port,
        layout,
    }
}

/// The full MockRunner script for one install pass (paths from the layout).
fn script(
    layout: &DaemonLayout,
    user_exists: bool,
    hostname_only_loopback: bool,
) -> Vec<(String, Output)> {
    let root = layout.root.to_string_lossy().into_owned();
    let state = layout.state_dir().to_string_lossy().into_owned();
    let cache = format!("{root}/cache/packages");
    let lock = format!("{cache}/python/rns-lock.txt");
    let mut s: Vec<(String, Output)> = vec![
        ("apt-get update".into(), out(0, "")),
        ("apt-get install -y python3 python3-venv iw ca-certificates".into(), out(0, "")),
        (
            format!("apt-get install -y --download-only --reinstall -o Dir::Cache::archives={cache} python3 python3-venv iw ca-certificates"),
            out(0, ""),
        ),
        ("python3 -m pip --version".into(), out(0, "pip 24.0\n")),
        (
            format!("python3 -m pip download --require-hashes -r {lock} -d {cache}/python"),
            out(0, ""),
        ),
    ];
    if !user_exists {
        s.push(("id node111".into(), out(1, "")));
        s.push((
            "useradd --system --home-dir /var/lib/111 --shell /usr/sbin/nologin node111".into(),
            out(0, ""),
        ));
    } else {
        s.push(("id node111".into(), out(0, "")));
    }
    s.extend([
        ("getent group dialout".into(), out(0, "dialout:x:20:\n")),
        ("getent group uucp".into(), out(2, "")), // absent → stripped from unit
        ("usermod -aG dialout node111".into(), out(0, "")),
    ]);
    for dir in [
        state.clone(),
        format!("{state}/staging"),
        format!("{root}/identity"),
        format!("{root}/rns"),
        format!("{root}/venvs"),
    ] {
        s.push((format!("chown -R node111:node111 {dir}"), out(0, "")));
    }
    s.push((format!("chmod 0700 {root}/identity"), out(0, "")));
    s.extend([
        ("systemctl daemon-reload".into(), out(0, "")),
        ("udevadm control --reload".into(), out(0, "")),
        ("udevadm trigger".into(), out(0, "")),
        (
            "systemctl enable --now 111node.service 111node-update.path".into(),
            out(0, ""),
        ),
    ]);
    if hostname_only_loopback {
        s.push(("hostname -I".into(), out(0, "127.0.0.1\n")));
    }
    s
}

fn ctx_for<'a>(
    flow: &'a Flow,
    runner: &'a MockRunner,
    env: &'a MapEnv,
    allow_degraded: bool,
) -> InstallContext<'a> {
    InstallContext {
        layout: flow.layout.clone(),
        fs_root: flow.fs_root.path().to_path_buf(),
        runner,
        env,
        allow_degraded,
        min_installer_version: "2026.10.01".parse::<Calver>().unwrap(),
        manifest_base: format!("http://127.0.0.1:{}", flow.port),
        manifest_fallback: None,
        health_base: format!("http://127.0.0.1:{}", flow.port),
        probe_204: vec![format!("http://127.0.0.1:{}/204", flow.port)],
        check_root: false,
    }
}

#[test]
fn happy_path_end_to_end_and_idempotent_rerun() {
    let flow = setup(None);
    let env = base_env("debian");

    // --- first run ---
    let runner = MockRunner::new(script(&flow.layout, false, true));
    let ctx = ctx_for(&flow, &runner, &env, false);
    let fetcher = node111::util::http::Fetcher::new(Duration::from_secs(10), 64 << 20).unwrap();
    let receipt = run_with(&ctx, &fetcher)
        .unwrap_or_else(|e| panic!("install failed: {e:#}\ncalls: {:?}", runner.calls()));

    assert_eq!(receipt.version, RELEASE);
    assert_eq!(receipt.category, "Node2A");
    assert!(receipt.gateway);
    // L1 v2.2: the receipt carries the NodeId as the §7 machine string (hex).
    let node_id = node111::identity::parse_node_id_hex(&receipt.node_id)
        .unwrap_or_else(|| panic!("receipt.node_id is not 64 hex: {:?}", receipt.node_id));
    assert!(receipt
        .cache_hashes
        .contains_key("packages/python/rns-lock.txt"));

    // side effects
    let root = &flow.layout.root;
    assert!(root.join("releases/2026.10.12/bin/111").exists());
    assert_eq!(
        std::fs::read_link(root.join("current"))
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy(),
        RELEASE
    );
    assert!(root.join("state/floor.json").exists());
    assert!(root.join("receipt.json").exists());
    assert!(root.join("state/offer.json").exists());
    assert!(flow.fs_root.path().join("usr/local/bin/111").exists());
    assert!(flow
        .fs_root
        .path()
        .join("usr/local/bin/111-uninstall")
        .exists());
    let unit = std::fs::read_to_string(
        flow.fs_root
            .path()
            .join("etc/systemd/system/111node.service"),
    )
    .unwrap();
    assert!(
        unit.contains("SupplementaryGroups=dialout"),
        "groups present: {unit}"
    );
    assert!(
        !unit
            .lines()
            .any(|l| l.starts_with("SupplementaryGroups=") && l.contains("uucp")),
        "absent group stripped from directive: {unit}"
    );
    assert!(flow
        .fs_root
        .path()
        .join("etc/systemd/system/111node-update.path")
        .exists());
    assert!(flow
        .fs_root
        .path()
        .join("etc/udev/rules.d/99-111-lora.rules")
        .exists());
    // L2 §13 identity file set with perms (written by id111's atomic writer)
    use std::os::unix::fs::PermissionsExt;
    let id_dir = root.join("identity");
    let mode = |name: &str| id_dir.join(name).metadata().unwrap().permissions().mode() & 0o777;
    assert_eq!(
        id_dir.metadata().unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(mode("mnemonic"), 0o600);
    assert_eq!(mode("keyset"), 0o600);
    assert_eq!(mode("nodeid"), 0o644);
    assert_eq!(mode("descriptor"), 0o644);
    assert_eq!(mode("counters"), 0o600);
    // the nodeid record is `0xed 0x01 ‖ NodeId` and agrees with the receipt
    let record = std::fs::read(id_dir.join("nodeid")).unwrap();
    assert_eq!(&record[..2], &[0xed, 0x01]);
    assert_eq!(&record[2..], node_id.as_bytes());
    // no provisional-seam files, and the dir round-trips through the binding
    assert_eq!(
        std::fs::read_dir(&id_dir).unwrap().count(),
        5,
        "exactly the five §13 files"
    );
    assert_eq!(
        node111::identity::load(&id_dir).unwrap().node_id_hex(),
        receipt.node_id
    );
    runner.assert_exhausted();

    // --- idempotent re-run (same release): all steps probe, nothing breaks ---
    let runner2 = MockRunner::new(script(&flow.layout, true, true));
    let ctx2 = ctx_for(&flow, &runner2, &env, false);
    let receipt2 = run_with(&ctx2, &fetcher)
        .unwrap_or_else(|e| panic!("re-run failed: {e:#}\ncalls: {:?}", runner2.calls()));
    assert_eq!(receipt2.version, RELEASE);
    // identity NOT regenerated
    assert_eq!(receipt2.node_id, receipt.node_id);
    assert_eq!(
        std::fs::read(root.join("identity/nodeid")).unwrap(),
        record,
        "nodeid record unchanged by the re-run"
    );
    // offer not re-written
    let offer: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("state/offer.json")).unwrap())
            .unwrap();
    assert_eq!(offer["state"], "offered");
    runner2.assert_exhausted();
}

#[test]
fn downgrade_refused() {
    let flow = setup(None);
    let env = base_env("debian");
    // prior receipt claims a NEWER version than the manifest offers
    let r = Receipt::new("2026.11.01");
    r.write(&flow.layout.root.join("receipt.json")).unwrap();

    let runner = MockRunner::new(Vec::<(String, Output)>::new());
    let ctx = ctx_for(&flow, &runner, &env, false);
    let fetcher = node111::util::http::Fetcher::new(Duration::from_secs(10), 64 << 20).unwrap();
    let err = run_with(&ctx, &fetcher).unwrap_err();
    assert!(err.to_string().contains("downgrade"), "{err}");
}

#[test]
fn degraded_container_gets_dropin() {
    let flow = setup(None);
    let env = base_env("debian").cmd("systemd-detect-virt --container", "docker\n");
    let runner = MockRunner::new(script(&flow.layout, false, true));
    let ctx = ctx_for(&flow, &runner, &env, true); // --allow-degraded
    let fetcher = node111::util::http::Fetcher::new(Duration::from_secs(10), 64 << 20).unwrap();
    let receipt =
        run_with(&ctx, &fetcher).unwrap_or_else(|e| panic!("degraded install failed: {e:#}"));
    assert!(receipt.degrade_branches.is_empty() || receipt.degrade_branches.len() <= 1);
    assert!(flow
        .fs_root
        .path()
        .join("etc/systemd/system/111node.service.d/10-container.conf")
        .exists());
}

#[test]
fn clock_skew_refuses() {
    let flow = setup(Some("Wed, 01 Jan 2020 00:00:00 GMT".into()));
    let env = base_env("debian");
    let runner = MockRunner::new(Vec::<(String, Output)>::new());
    let ctx = ctx_for(&flow, &runner, &env, false);
    let fetcher = node111::util::http::Fetcher::new(Duration::from_secs(10), 64 << 20).unwrap();
    let err = run_with(&ctx, &fetcher).unwrap_err();
    assert!(err.to_string().contains("clock"), "{err}");
}

/// F1: installer killed mid-extract leaves a torn .tmp dir — re-run must heal
/// (content probe, never existence: the torn dir is removed and restaged).
#[test]
fn f1_torn_stage_heals() {
    let flow = setup(None);
    let env = base_env("debian");
    // simulate the kill-mid-extract debris
    let debris = flow.layout.root.join("releases/.2026.10.12.tmp-9999");
    std::fs::create_dir_all(&debris).unwrap();
    std::fs::write(debris.join("partial"), b"torn").unwrap();
    // and a torn RELEASE dir (rename happened, tree incomplete)
    let torn = flow.layout.root.join("releases/2026.10.12");
    std::fs::create_dir_all(&torn).unwrap();
    std::fs::write(torn.join("bin"), b"incomplete").unwrap();

    let runner = MockRunner::new(script(&flow.layout, false, true));
    let ctx = ctx_for(&flow, &runner, &env, false);
    let fetcher = node111::util::http::Fetcher::new(Duration::from_secs(10), 64 << 20).unwrap();
    let receipt = run_with(&ctx, &fetcher).unwrap_or_else(|e| panic!("heal install failed: {e:#}"));
    assert_eq!(receipt.version, RELEASE);
    // debris gone, real tree verified
    assert!(!flow
        .layout
        .root
        .join("releases/.2026.10.12.tmp-9999")
        .exists());
    let release = flow.layout.root.join("releases/2026.10.12");
    assert!(release.join("bin/111").exists());
    assert!(node111::daemon::updater::stage::verify_tree(&release).is_ok());
    runner.assert_exhausted();
}

#[test]
fn supervise_unit_rendering_is_embedded() {
    // The embedded unit carries the supervision contract (spot-check the spec anchors).
    let _ = supervise::run; // module compiles with embedded assets
    let unit = include_str!("../../../systemd/111node.service");
    for anchor in [
        "Type=notify",
        "WatchdogSec=30",
        "StartLimitIntervalSec=0",
        "ExecStartPre=+/usr/local/bin/111 update-guard",
        "LogNamespace=111node",
        "ProtectSystem=strict",
        "StateDirectory=111",
    ] {
        assert!(unit.contains(anchor), "unit missing {anchor}");
    }
}
