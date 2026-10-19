//! T5 acceptance: Linux installer state machine end-to-end against a fixture
//! HTTP server + scripted MockRunner + temp fs roots — happy path + idempotent
//! re-run, downgrade refusal, degraded container path, clock-skew refusal, and
//! the stage step's Layer 5 cross-check (fresh extraction and the "already
//! staged" content probe; os/50-messaging/README.md §19 L1). Then the presence
//! venv (os/50-messaging/README.md §14) with a fake python3/pip
//! (`fake_python`): a fresh install builds it from the pinned lock before step 7
//! hands `venvs/` to node111, a re-run leaves it alone, and a failed build
//! never fails the install (a receipt degrade branch). And the rns closure on
//! a fresh Debian whose `python3` has no pip module: a throwaway venv's own
//! pip downloads it (`packages::pip_closure`).

mod common;
mod fake_python;

use node111::calver::Calver;
use node111::daemon::transport::venv::presence::{self, Outcome, PresenceVenvError};
use node111::daemon::DaemonLayout;
use node111::install::context::InstallContext;
use node111::install::receipt::Receipt;
use node111::install::{run_with, supervise};
use node111::manifest::layer5;
use node111::testkit::{MapEnv, MockRunner};
use node111::util::run::{Output, Runner};
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

/// Build a served dir: signed manifest (`extra` sections appended after the
/// keychain) + `payload` + rns-lock + health + 204.
fn served_dir_with(release: &str, payload: Vec<u8>, extra: &str) -> (tempfile::TempDir, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
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
{extra}"#,
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
    setup_with(date_header, common::build_payload(), "")
}

/// [`setup`] serving `payload` with `extra` manifest sections.
fn setup_with(date_header: Option<String>, payload: Vec<u8>, extra: &str) -> Flow {
    let (served, _) = served_dir_with(RELEASE, payload, extra);
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
    let tool = format!("{root}/{}", node111::install::packages::RNS_PIP_VENV);
    let mut s: Vec<(String, Output)> = vec![
        ("apt-get update".into(), out(0, "")),
        ("apt-get install -y python3 python3-venv iw ca-certificates".into(), out(0, "")),
        (
            format!("apt-get install -y --download-only --reinstall -o Dir::Cache::archives={cache} python3 python3-venv iw ca-certificates"),
            out(0, ""),
        ),
        (format!("python3 -m venv {tool}"), out(0, "")),
        (
            format!("{tool}/bin/pip download --require-hashes -r {lock} -d {cache}/python"),
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
    runner: &'a dyn Runner,
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

const MESHMUD: &[u8] = b"meshmud v1.0.0 linux/amd64 static";

/// Serve `payload` and return a parsed manifest pinning it (plus `extra`
/// sections) — the stage step takes an already-verified manifest.
fn stage_fixture(
    payload: &[u8],
    extra: &str,
) -> (tempfile::TempDir, node111::manifest::ChannelManifest) {
    let served = tempfile::tempdir().unwrap();
    std::fs::write(served.path().join("payload.tar.gz"), payload).unwrap();
    let port = common::serve_dir(served.path().to_path_buf(), None);
    let text = format!(
        r#"manifest-version = 1
date = "{RELEASE}"
channel = "stable"
release = "{RELEASE}"
min-upgrade-from = "2026.10.01"

[pkg.111.target.x86_64-unknown-linux-musl]
url = "https://example.invalid/111"
sha256 = "{a64}"
size = 100

[payload.x86_64-unknown-linux-musl]
url = "http://127.0.0.1:{port}/payload.tar.gz"
sha256 = "{psha}"
size = {plen}

[rns-lock]
url = "https://example.invalid/rns-lock.txt"
sha256 = "{a64}"
size = 100

[kb]
url = "https://example.invalid/kb.tar.gz"
sha256 = "{a64}"
size = 100

[[keychain]]
generation = 0
pubkey = "RWRoot00000000000000000000000000000000000000000="
{extra}"#,
        a64 = "a".repeat(64),
        psha = node111::util::hash::sha256_bytes(payload),
        plen = payload.len(),
    );
    let manifest = node111::manifest::ChannelManifest::parse(&text).unwrap();
    (served, manifest)
}

/// A context for the stage step alone (it runs no commands and probes nothing).
fn stage_ctx<'a>(
    layout: &DaemonLayout,
    fs_root: &Path,
    runner: &'a dyn Runner,
    env: &'a MapEnv,
) -> InstallContext<'a> {
    InstallContext {
        layout: layout.clone(),
        fs_root: fs_root.to_path_buf(),
        runner,
        env,
        allow_degraded: false,
        min_installer_version: "2026.10.01".parse::<Calver>().unwrap(),
        manifest_base: String::new(),
        manifest_fallback: None,
        health_base: String::new(),
        probe_204: Vec::new(),
        check_root: false,
    }
}

fn meshmud_pin(bytes: &[u8]) -> String {
    format!(
        "\n[pkg.meshmud.target.x86_64-unknown-linux-musl]\nurl = \"https://example.invalid/meshmud\"\nsha256 = \"{}\"\nsize = {}\n",
        node111::util::hash::sha256_bytes(bytes),
        bytes.len()
    )
}

/// §19 L1 at the installer's extraction: a payload that disagrees with the
/// manifest's Layer 5 pins is never promoted (no release dir, no debris, no flip).
#[test]
fn stage_refuses_payload_that_disagrees_with_layer5_pins() {
    let payload = common::build_payload_with(&[("bin/meshmud", MESHMUD)]);
    for (extra, needle) in [
        (String::new(), "pins no meshmud"),
        (meshmud_pin(b"another meshmud build"), "bin/meshmud size"),
    ] {
        let (_served, manifest) = stage_fixture(&payload, &extra);
        let node = tempfile::tempdir().unwrap();
        let fs_root = tempfile::tempdir().unwrap();
        let layout = DaemonLayout {
            root: node.path().join("var/lib/111"),
            etc: fs_root.path().join("etc/111"),
        };
        std::fs::create_dir_all(layout.state_dir()).unwrap();
        let runner = MockRunner::new(Vec::<(String, Output)>::new());
        let env = MapEnv::new();
        let ctx = stage_ctx(&layout, fs_root.path(), &runner, &env);
        let fetcher = node111::util::http::Fetcher::new(Duration::from_secs(10), 64 << 20).unwrap();
        let err = node111::install::stage::run(&ctx, &fetcher, &manifest).unwrap_err();
        assert!(
            err.to_string().contains(needle),
            "wanted {needle:?} in: {err}"
        );
        let releases = layout.root.join("releases");
        assert!(!releases.join(RELEASE).exists());
        let debris: Vec<_> = std::fs::read_dir(&releases).unwrap().collect();
        assert!(debris.is_empty(), "extraction debris left: {debris:?}");
        assert!(!layout.current_release().exists());
    }
}

/// The "already staged" content probe also checks the Layer 5 pins: a prior
/// release dir that passes its own payload manifest but carries an unpinned
/// bin/meshmud is re-extracted from the pinned tarball, not accepted as-is.
#[test]
fn stage_probe_rejects_a_release_dir_with_an_unpinned_layer5_asset() {
    let (_served, manifest) = stage_fixture(&common::build_payload(), "");
    let node = tempfile::tempdir().unwrap();
    let fs_root = tempfile::tempdir().unwrap();
    let layout = DaemonLayout {
        root: node.path().join("var/lib/111"),
        etc: fs_root.path().join("etc/111"),
    };
    std::fs::create_dir_all(layout.state_dir()).unwrap();
    let runner = MockRunner::new(Vec::<(String, Output)>::new());
    let env = MapEnv::new();
    let ctx = stage_ctx(&layout, fs_root.path(), &runner, &env);
    let fetcher = node111::util::http::Fetcher::new(Duration::from_secs(10), 64 << 20).unwrap();
    let release = node111::install::stage::run(&ctx, &fetcher, &manifest).unwrap();
    assert!(release.join("bin/111").exists());

    // A planted asset the payload manifest does not list: verify_tree alone passes.
    std::fs::write(release.join("bin/meshmud"), MESHMUD).unwrap();
    assert!(node111::daemon::updater::stage::verify_tree(&release).is_ok());

    let again = node111::install::stage::run(&ctx, &fetcher, &manifest).unwrap();
    assert_eq!(again, release);
    assert!(
        !release.join("bin/meshmud").exists(),
        "the probe must not accept a release dir carrying an unpinned meshmud"
    );
    assert!(release.join("bin/111").exists());
    manifest
        .verify_layer5_tree(&release, "x86_64-unknown-linux-musl")
        .unwrap();
}

// --- the presence venv (Layer 5 §14) ----------------------------------------

const PRESENCE_LOCK: &[u8] = b"meshtastic==2.7.11 --hash=sha256:abcd\n";

/// `[presence-lock]` pinning `bytes`.
fn presence_pin(bytes: &[u8]) -> String {
    format!(
        "\n[presence-lock]\nurl = \"https://example.invalid/meshtastic-presence-lock.txt\"\nsha256 = \"{}\"\nsize = {}\n",
        node111::util::hash::sha256_bytes(bytes),
        bytes.len()
    )
}

/// The install script without its `python3` and venv `bin/pip` steps (the
/// fake python runs them: `fake_python::Routed`).
fn without_python(script: Vec<(String, Output)>) -> Vec<(String, Output)> {
    script
        .into_iter()
        .filter(|(cmd, _)| {
            let program = cmd.split(' ').next().unwrap_or_default();
            program != "python3" && !Path::new(program).ends_with("bin/pip")
        })
        .collect()
}

/// A fresh install builds `venvs/presence` from the lock the signed manifest
/// pins (the build venv's own pip downloads the closure into the cache, then
/// installs from it only), before step 7 hands `venvs/` to node111; a re-run of
/// the same release leaves the venv alone.
#[test]
fn install_builds_the_presence_venv_from_the_pinned_lock() {
    let payload = common::build_payload_with(&[(layer5::PRESENCE_LOCK, PRESENCE_LOCK)]);
    let flow = setup_with(None, payload, &presence_pin(PRESENCE_LOCK));
    let env = base_env("debian");
    let py = fake_python::FakePython::new();
    let mock = MockRunner::new(without_python(script(&flow.layout, false, true)));
    let runner = fake_python::Routed::new(&mock, &py);
    let ctx = ctx_for(&flow, &runner, &env, false);
    let fetcher = node111::util::http::Fetcher::new(Duration::from_secs(10), 64 << 20).unwrap();
    let receipt = run_with(&ctx, &fetcher)
        .unwrap_or_else(|e| panic!("install failed: {e:#}\ncalls: {:?}", runner.order()));
    assert_eq!(receipt.version, RELEASE);
    assert!(
        receipt.degrade_branches.is_empty(),
        "{:?}",
        receipt.degrade_branches
    );
    mock.assert_exhausted();

    let root = &flow.layout.root;
    let lock = root
        .join("releases")
        .join(RELEASE)
        .join(layer5::PRESENCE_LOCK);
    let cache = presence::cache_dir(root);
    let calls = py.calls();
    let find = |want: &dyn Fn(&str) -> bool| {
        calls
            .iter()
            .position(|c| want(c))
            .unwrap_or_else(|| panic!("call missing: {calls:?}"))
    };
    let builds = presence::builds_dir(root);
    let venv = find(&|c: &str| c.starts_with(&format!("python3 -m venv {}/", builds.display())));
    let download = find(&|c: &str| {
        c.starts_with(&builds.to_string_lossy().into_owned())
            && c.ends_with(&format!(
                "/bin/pip download --require-hashes -r {} -d {}",
                lock.display(),
                cache.display()
            ))
    });
    let install = find(&|c: &str| {
        c.ends_with(&format!(
            "/bin/pip install --require-hashes -r {} --no-index --find-links {}",
            lock.display(),
            cache.display()
        ))
    });
    assert!(venv < download && download < install, "{calls:?}");
    // built while the installer still holds the tree; step 7 then chowns it
    let built = runner
        .position(&format!("python3 -m venv {}/", builds.display()))
        .unwrap();
    let chown = runner
        .position(&format!(
            "chown -R node111:node111 {}/venvs",
            root.display()
        ))
        .unwrap();
    assert!(built < chown, "{:?}", runner.order());

    let target = std::fs::read_link(presence::venv_dir(root)).unwrap();
    assert!(target.starts_with("presence.d"), "{target:?}");
    assert!(presence::python(root).is_file());
    assert_eq!(
        std::fs::read(presence::venv_dir(root).join("installed-lock.txt")).unwrap(),
        PRESENCE_LOCK
    );
    assert!(presence::integrity_ok(root, &lock));
    let status = presence::status(root).unwrap();
    assert!(status.ok, "{status:?}");
    assert_eq!(
        status.lock_sha256,
        node111::util::hash::sha256_bytes(PRESENCE_LOCK)
    );
    // the reader's interpreter, as the daemon will look for it
    assert_eq!(
        node111::daemon::meshtastic::MeshtasticConfig::production(&flow.layout, false)
            .reader_python,
        presence::python(root)
    );

    // --- re-run of the same release: the venv is intact, nothing rebuilds ---
    let before = py.calls().len();
    let mock2 = MockRunner::new(without_python(script(&flow.layout, true, true)));
    let runner2 = fake_python::Routed::new(&mock2, &py);
    let ctx2 = ctx_for(&flow, &runner2, &env, false);
    run_with(&ctx2, &fetcher)
        .unwrap_or_else(|e| panic!("re-run failed: {e:#}\ncalls: {:?}", runner2.order()));
    mock2.assert_exhausted();
    let rerun = py.calls()[before..].to_vec();
    let (builds_s, cache_s) = (
        builds.to_string_lossy().into_owned(),
        cache.to_string_lossy().into_owned(),
    );
    assert!(
        !rerun
            .iter()
            .any(|c| c.contains(&builds_s) || c.contains(&cache_s)),
        "the re-run touched the presence venv: {rerun:?}"
    );
    // (step 5 re-downloads only the rns closure, with its own tool venv)
    assert!(
        rerun
            .iter()
            .all(|c| c.contains(node111::install::packages::RNS_PIP_VENV)),
        "{rerun:?}"
    );
    assert_eq!(
        std::fs::read_link(presence::venv_dir(root)).unwrap(),
        target
    );
}

/// A presence venv that cannot be built never fails the install: the release
/// is staged and promoted, no venv appears, and the status says why (a hash
/// mismatch after a failed download; then no python3). Once fixed, the next
/// run — the "already staged" path — builds it.
#[test]
fn install_survives_a_presence_venv_failure() {
    let payload = common::build_payload_with(&[(layer5::PRESENCE_LOCK, PRESENCE_LOCK)]);
    let (_served, manifest) = stage_fixture(&payload, &presence_pin(PRESENCE_LOCK));
    let node = tempfile::tempdir().unwrap();
    let fs_root = tempfile::tempdir().unwrap();
    let layout = DaemonLayout {
        root: node.path().join("var/lib/111"),
        etc: fs_root.path().join("etc/111"),
    };
    std::fs::create_dir_all(layout.state_dir()).unwrap();
    let py = fake_python::FakePython::new();
    py.fail_download(true);
    py.fail_install(true);
    let env = MapEnv::new();
    let ctx = stage_ctx(&layout, fs_root.path(), &py, &env);
    let fetcher = node111::util::http::Fetcher::new(Duration::from_secs(10), 64 << 20).unwrap();
    let root = &layout.root;

    let release = node111::install::stage::run(&ctx, &fetcher, &manifest)
        .unwrap_or_else(|e| panic!("a presence failure failed the stage step: {e:#}"));
    assert!(release.join("bin/111").exists());
    assert!(layout.current_release().join("bin/111").exists());
    assert!(std::fs::symlink_metadata(presence::venv_dir(root)).is_err());
    assert_eq!(
        std::fs::read_dir(presence::builds_dir(root))
            .unwrap()
            .count(),
        0,
        "the failed build is removed"
    );
    let status = presence::status(root).unwrap();
    assert!(!status.ok && !status.previous_kept, "{status:?}");
    assert_eq!(status.code, "presence_venv_failed");
    let reason = presence::unavailable_reason(root).unwrap();
    assert!(reason.starts_with("The presence reader is off"), "{reason}");
    assert!(reason.contains("DO NOT MATCH THE HASHES"), "{reason}");
    assert!(reason.contains("downloading them failed too"), "{reason}");

    // no python3 at all: recorded as such, still not fatal
    py.set_missing(true);
    assert_eq!(
        node111::install::stage::presence_venv(&ctx, &manifest, &release),
        Outcome::Unavailable {
            error: PresenceVenvError::NoPython3,
            previous_kept: false
        }
    );
    assert_eq!(presence::status(root).unwrap().code, "presence_no_python");

    // fixed: the next install run (already staged) builds it
    py.set_missing(false);
    py.fail_download(false);
    py.fail_install(false);
    let again = node111::install::stage::run(&ctx, &fetcher, &manifest).unwrap();
    assert_eq!(again, release);
    assert!(presence::integrity_ok(
        root,
        &release.join(layer5::PRESENCE_LOCK)
    ));
    assert_eq!(presence::unavailable_reason(root), None);

    // a manifest without the reader runs no python at all
    let (_served2, plain) = stage_fixture(&common::build_payload(), "");
    let silent = MockRunner::new(Vec::<(String, Output)>::new());
    let ctx2 = stage_ctx(&layout, fs_root.path(), &silent, &env);
    assert_eq!(
        node111::install::stage::presence_venv(&ctx2, &plain, &release),
        Outcome::NotShipped
    );
    silent.assert_exhausted();
}

/// A presence venv that cannot be built is a receipt degrade branch (step 6),
/// with the reason its build recorded — and still never fails the install.
#[test]
fn a_failed_presence_venv_is_a_receipt_degrade_branch() {
    let payload = common::build_payload_with(&[(layer5::PRESENCE_LOCK, PRESENCE_LOCK)]);
    let flow = setup_with(None, payload, &presence_pin(PRESENCE_LOCK));
    let env = base_env("debian");
    let py = fake_python::FakePython::new();
    py.fail_install(true);
    let mock = MockRunner::new(without_python(script(&flow.layout, false, true)));
    let runner = fake_python::Routed::new(&mock, &py);
    let ctx = ctx_for(&flow, &runner, &env, false);
    let fetcher = node111::util::http::Fetcher::new(Duration::from_secs(10), 64 << 20).unwrap();
    let receipt = run_with(&ctx, &fetcher)
        .unwrap_or_else(|e| panic!("install failed: {e:#}\ncalls: {:?}", runner.order()));
    mock.assert_exhausted();
    let root = &flow.layout.root;
    assert!(std::fs::symlink_metadata(presence::venv_dir(root)).is_err());
    let reason = presence::unavailable_reason(root).unwrap();
    let branches: Vec<_> = receipt
        .degrade_branches
        .iter()
        .filter(|b| b.starts_with("presence venv unavailable: "))
        .collect();
    assert_eq!(branches.len(), 1, "{:?}", receipt.degrade_branches);
    assert!(branches[0].ends_with(&reason), "{branches:?} vs {reason}");
    assert!(
        branches[0].contains("DO NOT MATCH THE HASHES"),
        "{branches:?}"
    );
    // … and it is what the receipt on disk says
    let written = Receipt::read(&root.join("receipt.json")).unwrap().unwrap();
    assert_eq!(written.degrade_branches, receipt.degrade_branches);
}

/// A fresh Debian/Ubuntu: `python3` + `python3-venv`, but no pip module
/// ("No module named pip" — `python3-pip` is not in the package set). The rns
/// closure is still prefetched into `cache/packages/python/` (the daemon's
/// offline rns venv build needs it, or the node has no transport): a
/// throwaway venv's own pip downloads it, the host's `python3 -m pip` is
/// never asked, and the tool venv is gone before `cache/` is hashed.
#[test]
fn rns_closure_is_prefetched_without_a_host_pip() {
    let flow = setup(None);
    let env = base_env("debian");
    let py = fake_python::FakePython::new();
    py.no_host_pip(true);
    let mock = MockRunner::new(without_python(script(&flow.layout, false, true)));
    let runner = fake_python::Routed::new(&mock, &py);
    let ctx = ctx_for(&flow, &runner, &env, false);
    let fetcher = node111::util::http::Fetcher::new(Duration::from_secs(10), 64 << 20).unwrap();
    let receipt = run_with(&ctx, &fetcher)
        .unwrap_or_else(|e| panic!("install failed: {e:#}\ncalls: {:?}", runner.order()));
    mock.assert_exhausted();
    assert!(
        receipt.degrade_branches.is_empty(),
        "no transport stub on a fresh Debian: {:?}",
        receipt.degrade_branches
    );
    // the package set stays pip-free (python3-pip recommends build tools)
    assert_eq!(
        node111::install::packages::packages_for(node111::install::packages::Family::Debian),
        ["python3", "python3-venv", "iw", "ca-certificates"]
    );

    let root = &flow.layout.root;
    let tool = root.join(node111::install::packages::RNS_PIP_VENV);
    let cache = root.join("cache/packages/python");
    let calls = py.calls();
    assert!(
        !calls.iter().any(|c| c.starts_with("python3 -m pip")),
        "the host pip was asked: {calls:?}"
    );
    let venv = calls
        .iter()
        .position(|c| *c == format!("python3 -m venv {}", tool.display()))
        .unwrap_or_else(|| panic!("no tool venv: {calls:?}"));
    let download = calls
        .iter()
        .position(|c| {
            *c == format!(
                "{}/bin/pip download --require-hashes -r {}/rns-lock.txt -d {}",
                tool.display(),
                cache.display(),
                cache.display()
            )
        })
        .unwrap_or_else(|| panic!("no rns closure download: {calls:?}"));
    assert!(venv < download, "{calls:?}");
    assert!(
        std::fs::symlink_metadata(&tool).is_err(),
        "the tool venv is removed"
    );
    assert!(
        receipt.cache_hashes.keys().all(|k| !k.contains(".rns-pip")),
        "{:?}",
        receipt.cache_hashes.keys()
    );
    assert!(receipt
        .cache_hashes
        .contains_key("packages/python/rns-lock.txt"));

    // no python3 venv at all: the transport stub degrade, never an install error
    let flow = setup(None);
    let py = fake_python::FakePython::new();
    py.fail_venv(true);
    let mock = MockRunner::new(without_python(script(&flow.layout, false, true)));
    let runner = fake_python::Routed::new(&mock, &py);
    let ctx = ctx_for(&flow, &runner, &env, false);
    let receipt = run_with(&ctx, &fetcher)
        .unwrap_or_else(|e| panic!("install failed: {e:#}\ncalls: {:?}", runner.order()));
    mock.assert_exhausted();
    assert_eq!(
        receipt.degrade_branches,
        ["python3/venv absent after package step: rns closure not prefetched; transport stub"]
    );
    assert!(std::fs::symlink_metadata(
        flow.layout
            .root
            .join(node111::install::packages::RNS_PIP_VENV)
    )
    .is_err());
}
