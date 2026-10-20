//! Layer 6 C4 acceptance: the device core (`os/60-transport/README.md` §12,
//! §16 G5) without hardware.
//!
//! - **Hotplug.** Captured-shape kernel uevents (`tests/fixtures/uevent/*`,
//!   NUL-separated as the kernel sends them) through `FixtureSource` kick the
//!   registry tick; noise does not; a change still takes two ticks.
//! - **Ports.** A port guard holding the tty (simulated with a file the test
//!   process holds open) reads `port_busy` naming it; `PortTable` collisions
//!   among rnsd, 111d's meshmud, meshmud and an identify probe answer
//!   `port_busy`; a foreign loopback holder is caught by the components'
//!   pre-bind check; ownership comes from `interfaces.toml`, not the rendered
//!   config.
//! - **Identify.** The sequence order, the 30 s rate limit on a fake clock,
//!   consent, a missing tool is `Error` (nothing persisted), three `NoMatch`
//!   persist `unknown`, an RNode match from a fake `rnodeconf`, a MeshCore
//!   match over a pty stub modelled on the MIT `kiss_modem_protocol.md`, a
//!   Meshtastic match through the stub meshmud downlink (the Layer 5 harness
//!   shape), timeouts, the offer whenever any step can run.
//! - **Applier.** The four actions with injected probe failures: the revert
//!   restores `interfaces.toml` and `rns/config` byte-exact and the rnstatus
//!   argv sequence is as specified (a recording fake `Runner`); `region_unset`;
//!   a crash mid-apply leaves `pending: true` and a restart never re-applies;
//!   `unsupported_action`; the two one-shots route to their handlers.
//!
//! No test imports Reticulum or starts Python except the Meshtastic step's
//! fake presence reader (the Layer 5 harness; needs `python3`).

mod fake_rnodeconf;

use fake_rnodeconf::FakeRnodeconf;
use node111::daemon::components::consent::ProblemWith;
use node111::daemon::components::manifest;
use node111::daemon::components::runner::RunnerConfig;
use node111::daemon::components::{
    Components, ComponentsConfig, Dependencies, EnableError, Wiring,
};
use node111::daemon::dashboard::api_v1::Problem;
use node111::daemon::dashboard::{ApiConfig, ApiState, DashboardState, SharedState};
use node111::daemon::devices::applier::{
    self, Action, ApplierConfig, ApplierRequest, ApplyError, ApplyProbe, BodyError, DecisionFile,
    DeviceLookup, OneShotError, OneShots, Refusal, SpineProbe,
};
use node111::daemon::devices::hotplug::{self, Action as UAction, FixtureSource, Subsystem};
use node111::daemon::devices::identify::meshcore::MeshcoreStep;
use node111::daemon::devices::identify::meshtastic::MeshtasticStep;
use node111::daemon::devices::identify::rnode::{self, RnodeStep};
use node111::daemon::devices::identify::{
    Abort, Clock, Consent, Identify, IdentifyError, Outcome, Probe, Step, StepKind, UplinkKind,
    RATE_LIMIT,
};
use node111::daemon::devices::registry::{IdentifyView, TargetError};
use node111::daemon::devices::{
    self, Confidence, DeviceFirmware, DeviceRecord, DeviceRegistry, DeviceState, FirmwareFamily,
    FirmwareSource, RegistryConfig, Roots, Stack, TickTiming,
};
use node111::daemon::meshtastic::shim::ShimConfig;
use node111::daemon::meshtastic::{Meshtastic, MeshtasticConfig};
use node111::daemon::ports::PortTable;
use node111::daemon::services::{ServiceId, Supervised};
use node111::daemon::supervise::{BackoffPolicy, ChildControl};
use node111::daemon::transport::config;
use node111::daemon::transport::interfaces::{self, IfKind, IfOwner, Interface};
use node111::daemon::transport::posture::{BoxFut, RestartHooks, RnsdChild, Spine};
use node111::daemon::transport::rnstatus::{InterfaceFacts, Rnstatus, TimedRunner};
use node111::daemon::DaemonLayout;
use node111::install::categorize::Category;
use node111::testkit::MockRunner;
use node111::util::hash::sha256_bytes;
use node111::util::run::{Output, Runner};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

// ================================================================ fake tree

const HUB: &str = "sys/devices/pci0000:00/0000:00:14.0/usb1";
const HELTEC_BY_ID: &str = "usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0";
const HELTEC: &str = "Likely: Heltec WiFi LoRa 32 V3";

fn w(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Relative symlink `link` → `target` (both absolute inside the tree).
fn rel_link(target: &Path, link: &Path) {
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    let base = link.parent().unwrap();
    let depth = base.components().count();
    let common = target
        .components()
        .zip(base.components())
        .take_while(|(a, b)| a == b)
        .count();
    let mut rel = PathBuf::new();
    for _ in common..depth {
        rel.push("..");
    }
    for c in target.components().skip(common) {
        rel.push(c);
    }
    let _ = std::fs::remove_file(link);
    symlink(rel, link).unwrap();
}

struct Tree {
    root: PathBuf,
}

impl Tree {
    fn new(dir: &Path) -> Tree {
        let t = Tree {
            root: dir.join("fake"),
        };
        t.usb(
            "usb1",
            "1d6b",
            "0002",
            "09",
            Some("xHCI Host Controller"),
            None,
            "09",
        );
        std::fs::create_dir_all(t.root.join("dev")).unwrap();
        t
    }

    fn roots(&self) -> Roots {
        Roots {
            dev: self.root.join("dev"),
            sys: self.root.join("sys"),
        }
    }

    fn dev_dir(&self, bus_id: &str) -> PathBuf {
        if bus_id == "usb1" {
            self.root.join(HUB)
        } else {
            self.root.join(HUB).join(bus_id)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn usb(
        &self,
        bus_id: &str,
        vid: &str,
        pid: &str,
        class: &str,
        product: Option<&str>,
        serial: Option<&str>,
        intf_class: &str,
    ) {
        let d = self.dev_dir(bus_id);
        w(&d.join("idVendor"), &format!("{vid}\n"));
        w(&d.join("idProduct"), &format!("{pid}\n"));
        w(&d.join("bDeviceClass"), &format!("{class}\n"));
        if let Some(p) = product {
            w(&d.join("product"), &format!("{p}\n"));
        }
        if let Some(s) = serial {
            w(&d.join("serial"), &format!("{s}\n"));
        }
        let intf = d.join(format!("{bus_id}:1.0"));
        w(&intf.join("bInterfaceClass"), &format!("{intf_class}\n"));
        rel_link(
            &intf,
            &self
                .root
                .join("sys/bus/usb/devices")
                .join(format!("{bus_id}:1.0")),
        );
        rel_link(&d, &self.root.join("sys/bus/usb/devices").join(bus_id));
    }

    /// Heltec V3 (CP2102 row of the shipped LoRa table) on ttyUSB0.
    fn heltec(&self) {
        self.usb(
            "1-1",
            "10c4",
            "ea60",
            "00",
            Some("CP2102 USB to UART Bridge Controller"),
            Some("0001"),
            "ff",
        );
        let port = self.dev_dir("1-1").join("1-1:1.0").join("ttyUSB0");
        std::fs::create_dir_all(&port).unwrap();
        rel_link(
            &port,
            &self.root.join("sys/class/tty/ttyUSB0").join("device"),
        );
        let node = self.tty();
        w(&node, "");
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o660)).unwrap();
        rel_link(
            &node,
            &self.root.join("dev/serial/by-id").join(HELTEC_BY_ID),
        );
    }

    /// Unplug the Heltec (sysfs and /dev).
    fn unplug(&self) {
        let _ = std::fs::remove_file(self.root.join("dev/serial/by-id").join(HELTEC_BY_ID));
        let _ = std::fs::remove_file(self.tty());
        let _ = std::fs::remove_file(self.root.join("sys/bus/usb/devices/1-1"));
        let _ = std::fs::remove_file(self.root.join("sys/bus/usb/devices/1-1:1.0"));
        let _ = std::fs::remove_dir_all(self.dev_dir("1-1"));
        let _ = std::fs::remove_dir_all(self.root.join("sys/class/tty/ttyUSB0"));
    }

    fn tty(&self) -> PathBuf {
        self.root.join("dev/ttyUSB0")
    }

    /// Point the tty node at `target` (a pty slave) instead of a plain file.
    fn tty_to(&self, target: &Path) {
        let _ = std::fs::remove_file(self.tty());
        symlink(target, self.tty()).unwrap();
    }
}

/// The by-id uplink path the registry names (under the real `/dev`).
fn heltec_uplink() -> String {
    format!("/dev/serial/by-id/{HELTEC_BY_ID}")
}

// ================================================================ the node

struct Node {
    dir: tempfile::TempDir,
    tree: Tree,
    layout: DaemonLayout,
    api: Arc<ApiState>,
    reg: Arc<DeviceRegistry>,
    ports: Arc<PortTable>,
}

fn closed_addr() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap()
}

fn node_with(edit: impl FnOnce(&mut RegistryConfig)) -> Node {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let tree = Tree::new(dir.path());
    tree.heltec();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.10.19")));
    let api = ApiState::new(dash, ApiConfig::new(layout.clone(), false));
    let mut cfg = RegistryConfig::production(&layout);
    cfg.roots = tree.roots();
    cfg.probe_addr = closed_addr();
    cfg.probe_timeout = Duration::from_millis(200);
    edit(&mut cfg);
    let reg = devices::install(&api, cfg);
    let ports = Arc::new(PortTable::new());
    api.insert_ext(Arc::clone(&ports));
    Node {
        dir,
        tree,
        layout,
        api,
        reg,
        ports,
    }
}

fn node() -> Node {
    node_with(|_| {})
}

impl Node {
    async fn tick(&self) {
        self.reg.tick(&self.api).await;
    }

    fn find(&self, name: &str) -> Option<DeviceRecord> {
        self.reg.list()?.into_iter().find(|r| r.name == name)
    }

    fn heltec(&self) -> DeviceRecord {
        self.find(HELTEC).expect("the Heltec is listed")
    }

    fn store(&self) -> Value {
        let p = self.layout.state_dir().join("devices.json");
        serde_json::from_slice(&std::fs::read(p).unwrap_or_default()).unwrap_or(Value::Null)
    }

    fn stored_firmware(&self, id: &str) -> Value {
        self.store()["devices"][id]["firmware"].clone()
    }

    fn frames(&self, since: u64, event: &str) -> Vec<Value> {
        self.api
            .events
            .subscribe(Some(since))
            .replay
            .iter()
            .filter(|f| f.event == event)
            .map(|f| serde_json::from_str(&f.data).unwrap())
            .collect()
    }

    /// An orchestrator over `steps` on this node (dev root = the tree), its
    /// availability installed as the registry's offer.
    fn identify(&self, steps: Vec<Arc<dyn Step>>, clock: Arc<dyn Clock>) -> Arc<Identify> {
        let i = Identify::new(
            Arc::clone(&self.reg),
            Arc::clone(&self.ports),
            steps,
            clock,
            self.tree.root.clone(),
            RATE_LIMIT,
        );
        let weak = Arc::downgrade(&i);
        self.reg
            .set_identify_view(move || weak.upgrade().map(|i| i.view()).unwrap_or_default());
        i
    }
}

async fn wait_until(what: &str, timeout: Duration, mut ok: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + timeout;
    while !ok() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel)
}

// ================================================================ hotplug

fn uevent(name: &str) -> Vec<u8> {
    std::fs::read(fixture(&format!("uevent/{name}"))).unwrap()
}

#[test]
fn uevent_fixtures_parse_as_the_kernel_sends_them() {
    for (name, want) in [
        ("usb-add.uevent", Some((UAction::Add, Subsystem::Usb))),
        ("usb-bind.uevent", Some((UAction::Bind, Subsystem::Usb))),
        ("tty-add.uevent", Some((UAction::Add, Subsystem::Tty))),
        ("tty-remove.uevent", Some((UAction::Remove, Subsystem::Tty))),
        ("usb-remove.uevent", Some((UAction::Remove, Subsystem::Usb))),
        ("net-add.uevent", None),
        ("usb-change.uevent", None),
    ] {
        let got = hotplug::parse(&uevent(name)).map(|e| (e.action, e.subsystem));
        assert_eq!(got, want, "{name}");
    }
    let tty = hotplug::parse(&uevent("tty-add.uevent")).unwrap();
    assert_eq!(tty.devname.as_deref(), Some("ttyUSB0"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn uevents_kick_the_tick_and_a_change_still_takes_two_ticks() {
    let n = node();
    n.tree.unplug();
    // the debounce, tick by tick: a plugged board is published on the
    // second identical tick, never the first
    n.tick().await;
    assert!(n.find(HELTEC).is_none());
    n.tree.heltec();
    n.tick().await;
    assert!(n.find(HELTEC).is_none(), "published after one tick");
    assert!(n.reg.has_pending());
    n.tick().await;
    assert!(n.find(HELTEC).is_some(), "published after two ticks");
    n.tree.unplug();
    n.tick().await;
    n.tick().await;
    assert_eq!(n.heltec().state, DeviceState::Disconnected);

    // the loop: the poll is an hour away, so only a kick can tick
    let (tx, src) = FixtureSource::channel(16);
    let kick = Arc::new(tokio::sync::Notify::new());
    let fwd = tokio::spawn(hotplug::forward(src, Arc::clone(&kick)));
    let timing = TickTiming {
        every: Duration::from_secs(3600),
        settle: Duration::from_millis(50),
        confirm: Duration::from_millis(100),
    };
    let ticks = tokio::spawn(devices::run_ticks(
        Arc::clone(&n.reg),
        Arc::clone(&n.api),
        Arc::clone(&kick),
        timing,
    ));
    // the loop's first (immediate) tick
    tokio::time::sleep(Duration::from_millis(200)).await;
    n.tree.heltec();
    // noise does not tick
    tx.send(uevent("net-add.uevent")).await.unwrap();
    tx.send(uevent("usb-change.uevent")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(n.heltec().state, DeviceState::Disconnected, "noise ticked");
    // a plug burst: one settle, a kicked tick, its confirming tick
    let head = n.api.events.head();
    for f in ["usb-add.uevent", "usb-bind.uevent", "tty-add.uevent"] {
        tx.send(uevent(f)).await.unwrap();
    }
    wait_until("the plug is published", Duration::from_secs(5), || {
        n.find(HELTEC).is_some_and(|r| r.state.present())
    })
    .await;
    let frames = n.frames(head, "devices");
    assert_eq!(frames.len(), 1, "one frame for the burst: {frames:?}");
    // an unplug the same way
    n.tree.unplug();
    tx.send(uevent("tty-remove.uevent")).await.unwrap();
    tx.send(uevent("usb-remove.uevent")).await.unwrap();
    wait_until("the unplug is published", Duration::from_secs(5), || {
        n.find(HELTEC)
            .is_some_and(|r| r.state == DeviceState::Disconnected)
    })
    .await;
    drop(tx);
    fwd.await.unwrap();
    ticks.abort();
}

// ================================================================ ports

/// This process's `comm` (what `/proc/<pid>/comm` says for it).
fn own_comm() -> String {
    std::fs::read_to_string("/proc/self/comm")
        .unwrap()
        .trim()
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_port_guard_holding_the_tty_reads_port_busy_naming_it() {
    let comm = own_comm();
    let guard = comm.clone();
    let n = node_with(move |c| c.port_guards = vec![guard]);
    n.reg.set_identify_view(|| IdentifyView {
        serial: true,
        tcp: false,
    });
    // the guard holds the tty open (simulated: this process opens the node)
    let held = std::fs::File::open(n.tree.tty()).unwrap();
    n.tick().await;
    let h = n.heltec();
    assert_eq!(h.reason_code, "port_busy", "{h:?}");
    assert_eq!(h.detail["port_held_by"], comm.as_str());
    assert!(h.reason.contains(&comm), "{}", h.reason);
    assert!(
        h.reason.contains(&format!("pid {}", std::process::id())),
        "{}",
        h.reason
    );
    assert!(!h.actions.iter().any(|a| a == "identify"), "{h:?}");
    match n.reg.identify_target(&h.id) {
        Err(TargetError::Busy { owner, .. }) => assert_eq!(owner, comm),
        other => panic!("{other:?}"),
    }
    // the guard lets go: offered again (after the debounce)
    drop(held);
    n.tick().await;
    n.tick().await;
    let h = n.heltec();
    assert_eq!(h.reason_code, "radio_unbound", "{h:?}");
    assert!(h.actions.iter().any(|a| a == "identify"), "{h:?}");
    // a name that is not a guard is never reported
    let n2 = node_with(|c| c.port_guards = vec!["ModemManager".into()]);
    let _held = std::fs::File::open(n2.tree.tty()).unwrap();
    n2.tick().await;
    assert_ne!(n2.heltec().reason_code, "port_busy");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn port_table_collisions_answer_port_busy() {
    let t = PortTable::new();
    // rnsd holds an RNode port: an identify probe and meshmud collide
    t.claim("/dev/ttyACM0", interfaces::TASK_RNSD, 1).unwrap();
    for task in ["identify:abcd1234abcd1234", interfaces::TASK_MESHMUD] {
        let busy = t.claim("/dev/ttyACM0", task, 2).unwrap_err();
        assert_eq!(busy.owner.task, "rnsd");
        assert_eq!(
            (busy.problem().status, busy.problem().code),
            (409, "port_busy")
        );
    }
    // 111d's Meshtastic uplink (claimed for meshmud): rnsd and a probe collide
    t.claim("10.0.0.7:4403", interfaces::TASK_MESHMUD, 3)
        .unwrap();
    for task in [interfaces::TASK_RNSD, "identify:abcd1234abcd1234"] {
        assert_eq!(
            t.claim("10.0.0.7:4403", task, 4).unwrap_err().owner.task,
            "meshmud"
        );
    }
    // a probe's claim: rnsd waits for it
    t.claim("/dev/ttyACM1", "identify:abcd1234abcd1234", 5)
        .unwrap();
    assert!(t.claim("/dev/ttyACM1", interfaces::TASK_RNSD, 6).is_err());

    // the registry and the identify sequence read the same table
    let n = node();
    let fake = FakeRnodeconf::install(n.dir.path());
    let ident = n.identify(
        vec![Arc::new(RnodeStep::new(
            Arc::new(TimedRunner::new(n.dir.path())),
            fake.program.clone(),
            false,
        ))],
        Arc::new(FakeClock::default()),
    );
    n.ports
        .claim(&heltec_uplink(), interfaces::TASK_RNSD, 1)
        .unwrap();
    n.tick().await;
    let h = n.heltec();
    assert_eq!(h.reason_code, "port_busy");
    assert_eq!(h.detail["port_held_by"], "rnsd");
    let e = ident.run(&n.api, &h.id, Consent::Given).await.unwrap_err();
    assert_eq!(e.problem().code, "port_busy", "{e}");
    assert!(e.problem().detail.unwrap().contains("rnsd"));
    assert!(fake.calls().is_empty(), "nothing opened");
}

// --- a foreign loopback holder and the components' pre-bind check (S4)

const BIN: &[u8] = b"#!/bin/sh\necho '{\"type\":\"ready\",\"dest\":\"00\"}'\nexec cat\n";

fn binary_toml(id: &str, port: u16) -> String {
    format!(
        r#"id = "{id}"
adapter = "app:{id}"
title = "Demo {id}"
[payload]
kind = "binary"
url = "https://example.invalid/dl/{id}-bin"
sha256 = "{sha}"
size = {size}
[run]
program = "{id}-bin"
ports = [{port}]
health = "ready"
health_timeout_secs = 5
[license]
spdx = "MPL-2.0"
read_at = "https://example.invalid/LICENSE"
notice = "This installs demo {id} (MPL-2.0). You install it."
"#,
        sha = sha256_bytes(BIN),
        size = BIN.len(),
    )
}

struct NoDeps;

impl Dependencies for NoDeps {
    fn child(&self, _: ServiceId) -> Option<ChildControl> {
        None
    }
    fn serving(&self, _: ServiceId) -> bool {
        true
    }
}

#[tokio::test]
async fn a_foreign_loopback_holder_is_caught_by_the_pre_bind_check() {
    let free = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("var/lib/111");
    std::fs::create_dir_all(root.join("state")).unwrap();
    std::fs::create_dir_all(root.join("current")).unwrap();
    let cache = root.join("cache/packages/demo");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("demo-bin"), BIN).unwrap();
    let mut all = manifest::load(&[(
        "demo.toml".to_string(),
        binary_toml("demo", port).into_bytes(),
    )]);
    let c = all.pop().expect("the declaration registers");
    let table = Arc::new(PortTable::new());
    let mut cfg = ComponentsConfig::new(&root);
    cfg.runner = RunnerConfig {
        retry: Duration::from_millis(50),
        retry_max: Duration::from_millis(200),
        settle: Duration::ZERO,
        grace: Duration::from_secs(2),
    };
    let comps = Components::new(
        cfg,
        vec![c.clone()],
        Wiring {
            cmd: Arc::new(MockRunner::new(Vec::<(String, Output)>::new())),
            ports: Arc::clone(&table),
            deps: Arc::new(NoDeps),
            tick: Arc::new(|| 7),
        },
    );
    let key = format!("127.0.0.1:{port}");
    // an identify probe (any 111 task) holds the key: port_busy naming it
    table.claim(&key, "identify:abcd1234abcd1234", 1).unwrap();
    let e = comps
        .enable("demo", Some(&c.notice_hash()))
        .await
        .unwrap_err();
    match &e {
        EnableError::PortBusy(b) => assert_eq!(b.owner.task, "identify:abcd1234abcd1234"),
        other => panic!("{other:?}"),
    }
    table.release(&key, "identify:abcd1234abcd1234").unwrap();
    // a holder outside 111: only the bind sees it; nothing stays claimed
    let holder = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let e = comps
        .enable("demo", Some(&c.notice_hash()))
        .await
        .unwrap_err();
    match &e {
        EnableError::PortBusy(b) => assert_eq!(b.owner.task, "a program outside 111"),
        other => panic!("{other:?}"),
    }
    assert_eq!(e.problem(Some(&c)).body()["code"], "port_busy");
    assert!(table.is_held(&key).is_none());
    drop(holder);
    comps.shutdown().await;
}

fn rnode_entry(name: &str, port: &str, region: Option<&str>) -> Interface {
    let mut params = toml::Table::new();
    for (k, v) in [
        ("frequency", 869_525_000),
        ("bandwidth", 125_000),
        ("txpower", 14),
        ("spreadingfactor", 8),
        ("codingrate", 5),
    ] {
        params.insert(k.into(), toml::Value::Integer(v));
    }
    Interface {
        name: name.into(),
        owner: IfOwner::Rnsd,
        kind: IfKind::RNode,
        device: None,
        port: Some(port.into()),
        region: region.map(str::to_string),
        uplink: None,
        params,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ownership_comes_from_interfaces_toml_not_the_rendered_config() {
    let n = node();
    n.reg.set_rnsd_running(|| true);
    n.reg.set_identify_view(|| IdentifyView {
        serial: true,
        tcp: false,
    });
    let st = n.layout.state_dir();
    // the rendered config names the board, interfaces.toml does not: unowned
    w(
        &config::config_path(&n.layout),
        &format!(
            "[interfaces]\n  [[Stale]]\n    type = RNodeInterface\n    enabled = yes\n    \
             port = {}\n",
            n.tree.root.join("dev/ttyUSB0").display()
        ),
    );
    let mut f = interfaces::InterfacesFile::default();
    interfaces::save(&st, &f).unwrap();
    n.tick().await;
    let h = n.heltec();
    assert!(h.driver.is_none(), "{h:?}");
    assert!(h.detail.get("rns_interface").is_none(), "{h:?}");
    assert!(h.actions.iter().any(|a| a == "identify"));
    // an rnsd entry in interfaces.toml owns it (by-id form)
    f.radio.region = Some("EU_868".into());
    f.interfaces.push(rnode_entry(
        "Roof RNode",
        &format!("/dev/serial/by-id/{HELTEC_BY_ID}"),
        None,
    ));
    interfaces::save(&st, &f).unwrap();
    n.tick().await;
    n.tick().await;
    let h = n.heltec();
    assert_eq!(
        h.driver.as_ref().map(|d| (d.stack, d.interface.clone())),
        Some((Stack::Reticulum, Some("Roof RNode".into())))
    );
    assert!(
        h.detail.get("rns_source").is_none(),
        "the TOML, not the config"
    );
    // configured but rnsd down: still owned (identify refused naming rnsd)
    n.reg.set_rnsd_running(|| false);
    n.tick().await;
    n.tick().await;
    let h = n.heltec();
    match n.reg.identify_target(&h.id) {
        Err(TargetError::Busy { owner, .. }) => assert_eq!(owner, "rnsd"),
        other => panic!("{other:?}"),
    }
    // the applier's port lookup accepts an rnsd-owned board (an update)
    assert_eq!(
        n.reg.serial_port_for(&h.id).unwrap(),
        format!("/dev/serial/by-id/{HELTEC_BY_ID}")
    );
}

// ================================================================ identify

#[derive(Default)]
struct FakeClock(Mutex<Duration>);

impl FakeClock {
    fn advance(&self, d: Duration) {
        *self.0.lock().unwrap() += d;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        *self.0.lock().unwrap()
    }
}

struct FakeStep {
    kind: StepKind,
    outcome: Mutex<Outcome>,
    log: Arc<Mutex<Vec<String>>>,
    installed: bool,
}

impl FakeStep {
    fn new(kind: StepKind, outcome: Outcome, log: &Arc<Mutex<Vec<String>>>) -> Arc<Self> {
        Arc::new(FakeStep {
            kind,
            outcome: Mutex::new(outcome),
            log: Arc::clone(log),
            installed: true,
        })
    }
}

impl Step for FakeStep {
    fn kind(&self) -> StepKind {
        self.kind
    }
    fn tool(&self) -> &'static str {
        "the fake tool"
    }
    fn available(&self, kind: UplinkKind) -> bool {
        self.installed && kind == UplinkKind::Serial
    }
    fn run<'a>(&'a self, probe: &'a Probe) -> BoxFut<'a, Result<Outcome, Abort>> {
        Box::pin(async move {
            self.log
                .lock()
                .unwrap()
                .push(format!("{} {}", self.kind.as_str(), probe.task));
            Ok(self.outcome.lock().unwrap().clone())
        })
    }
}

fn steps3(log: &Arc<Mutex<Vec<String>>>, outcomes: [Outcome; 3]) -> Vec<Arc<dyn Step>> {
    let [a, b, c] = outcomes;
    vec![
        FakeStep::new(StepKind::Rnode, a, log),
        FakeStep::new(StepKind::Meshtastic, b, log),
        FakeStep::new(StepKind::Meshcore, c, log),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn three_no_matches_in_order_persist_unknown_and_release_the_port() {
    let n = node();
    let log = Arc::new(Mutex::new(Vec::new()));
    let ident = n.identify(
        steps3(&log, [Outcome::NoMatch, Outcome::NoMatch, Outcome::NoMatch]),
        Arc::new(FakeClock::default()),
    );
    assert_eq!(
        ident.steps(),
        [StepKind::Rnode, StepKind::Meshtastic, StepKind::Meshcore],
        "DEV §3.2 order"
    );
    n.tick().await;
    let h = n.heltec();
    assert!(h.actions.iter().any(|a| a == "identify"), "{h:?}");
    let report = ident.run(&n.api, &h.id, Consent::Given).await.unwrap();
    let task = format!("identify:{}", h.id);
    assert_eq!(
        *log.lock().unwrap(),
        [
            format!("rnode {task}"),
            format!("meshtastic {task}"),
            format!("meshcore {task}")
        ]
    );
    assert_eq!(report.steps.len(), 3);
    let fw = report.record.firmware.unwrap();
    assert_eq!(
        (fw.family, fw.source),
        (FirmwareFamily::Unknown, FirmwareSource::Probe)
    );
    assert_eq!(n.stored_firmware(&h.id)["family"], "unknown");
    assert_eq!(n.stored_firmware(&h.id)["source"], "probe");
    assert_eq!(
        report.record.matched.confidence,
        Confidence::Likely,
        "nothing confirmed"
    );
    assert!(n.ports.held().is_empty(), "the claim is released");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_tool_is_an_error_and_unknown_is_not_persisted() {
    let n = node();
    let log = Arc::new(Mutex::new(Vec::new()));
    let missing = Arc::new(RnodeStep::new(
        Arc::new(TimedRunner::new(n.dir.path())),
        n.dir.path().join("venvs/rns/bin/rnodeconf"),
        false,
    ));
    assert!(!missing.available(UplinkKind::Serial), "no rnodeconf here");
    let ident = n.identify(
        vec![
            missing,
            FakeStep::new(StepKind::Meshtastic, Outcome::NoMatch, &log),
            FakeStep::new(StepKind::Meshcore, Outcome::NoMatch, &log),
        ],
        Arc::new(FakeClock::default()),
    );
    n.tick().await;
    let h = n.heltec();
    let e = ident.run(&n.api, &h.id, Consent::Given).await.unwrap_err();
    let IdentifyError::Inconclusive(steps) = &e else {
        panic!("{e:?}")
    };
    assert!(
        matches!(&steps[0].outcome, Outcome::Error(m) if m.contains("rnodeconf")),
        "{steps:?}"
    );
    assert_eq!(steps[1].outcome, Outcome::NoMatch);
    let p = e.problem();
    assert_eq!((p.status, p.code), (502, "upstream_error"));
    assert!(p.detail.unwrap().contains("rnode: error"));
    // nothing persisted: the sysfs-only unknown stands, never `source: probe`
    assert!(
        n.stored_firmware(&h.id).is_null(),
        "{}",
        n.stored_firmware(&h.id)
    );
    assert_ne!(
        n.heltec().firmware.map(|f| f.source),
        Some(FirmwareSource::Probe)
    );
    assert!(n.ports.held().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_probe_per_30_s_on_a_fake_clock_and_consent_is_required() {
    let n = node();
    let log = Arc::new(Mutex::new(Vec::new()));
    let clock = Arc::new(FakeClock::default());
    let ident = n.identify(
        steps3(&log, [Outcome::NoMatch, Outcome::NoMatch, Outcome::NoMatch]),
        clock.clone(),
    );
    n.tick().await;
    let id = n.heltec().id;
    // no consent now, none remembered: nothing runs
    let e = ident
        .run(&n.api, &id, Consent::Remembered)
        .await
        .unwrap_err();
    assert_eq!(e, IdentifyError::ConsentRequired);
    assert_eq!(e.problem().status, 400);
    assert!(log.lock().unwrap().is_empty());
    assert!(!n.reg.has_consent(&id));
    // the acceptance is the consent, remembered (persisted)
    ident.run(&n.api, &id, Consent::Given).await.unwrap();
    assert!(n.reg.has_consent(&id));
    let saved: Value = serde_json::from_slice(
        &std::fs::read(n.layout.state_dir().join("identify-consent.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["devices"], json!([id]));
    let runs = log.lock().unwrap().len();
    // 10 s later: refused, nothing runs, the wait named
    clock.advance(Duration::from_secs(10));
    match ident.run(&n.api, &id, Consent::Remembered).await {
        Err(IdentifyError::RateLimited { retry_after_secs }) => assert_eq!(retry_after_secs, 20),
        other => panic!("{other:?}"),
    }
    assert_eq!(log.lock().unwrap().len(), runs);
    assert_eq!(
        IdentifyError::RateLimited {
            retry_after_secs: 20
        }
        .problem()
        .status,
        429
    );
    // 30 s after the first probe: runs again on the remembered consent
    clock.advance(Duration::from_secs(20));
    ident.run(&n.api, &id, Consent::Remembered).await.unwrap();
    assert_eq!(log.lock().unwrap().len(), runs * 2);
    // a reloaded registry remembers; forgetting the device drops the consent
    let reloaded = DeviceRegistry::new(n.reg.config().clone());
    assert!(reloaded.has_consent(&id));
    n.reg.forget(&n.api, &id).unwrap();
    assert!(!n.reg.has_consent(&id));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_rnode_match_from_the_fake_rnodeconf_stops_the_sequence() {
    let n = node();
    let fake = FakeRnodeconf::install(n.dir.path());
    fake.answer("info-provisioned.txt");
    let log = Arc::new(Mutex::new(Vec::new()));
    let rn: Arc<dyn Step> = Arc::new(RnodeStep::new(
        Arc::new(TimedRunner::new(n.dir.path())),
        fake.program.clone(),
        false,
    ));
    let ident = n.identify(
        vec![
            rn,
            FakeStep::new(StepKind::Meshtastic, Outcome::NoMatch, &log),
            FakeStep::new(StepKind::Meshcore, Outcome::NoMatch, &log),
        ],
        Arc::new(FakeClock::default()),
    );
    n.tick().await;
    let h = n.heltec();
    let report = ident.run(&n.api, &h.id, Consent::Given).await.unwrap();
    let fw = report.record.firmware.clone().unwrap();
    assert_eq!(
        fw,
        DeviceFirmware {
            family: FirmwareFamily::Rnode,
            version: Some("1.86".into()),
            source: FirmwareSource::Probe,
        }
    );
    assert_eq!(n.stored_firmware(&h.id)["family"], "rnode");
    assert_eq!(n.stored_firmware(&h.id)["version"], "1.86");
    // the table's `likely` is refined by the probe
    assert_eq!(
        (
            report.record.matched.confidence,
            report.record.matched.rule.as_str()
        ),
        (Confidence::Exact, "probe rnode")
    );
    // rnodeconf --info <the port under the dev root>, once; nothing after it
    assert_eq!(
        fake.calls(),
        [format!(
            "--info {}",
            n.tree
                .root
                .join(format!("dev/serial/by-id/{HELTEC_BY_ID}"))
                .display()
        )]
    );
    assert!(log.lock().unwrap().is_empty(), "the first match ends it");
    assert!(n.ports.held().is_empty());
    // the goldens, one by one
    for (file, want) in [
        ("info-tnc.txt", Some("1.85")),
        ("eeprom-invalid.txt", Some("1.74")),
    ] {
        let text = std::fs::read_to_string(fixture(&format!("rnodeconf/{file}"))).unwrap();
        match rnode::parse_info(&text) {
            Outcome::Match(fw) => assert_eq!(fw.version.as_deref(), want, "{file}"),
            o => panic!("{file}: {o:?}"),
        }
    }
    let read = |f: &str| std::fs::read_to_string(fixture(&format!("rnodeconf/{f}"))).unwrap();
    assert_eq!(rnode::parse_info(&read("no-rnode.txt")), Outcome::NoMatch);
    assert!(matches!(
        rnode::parse_info(&read("cannot-open.txt")),
        Outcome::Error(_)
    ));
    assert!(matches!(
        rnode::parse_info(&read("garbage.txt")),
        Outcome::Error(_)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hung_rnodeconf_times_out_as_an_error() {
    let n = node();
    let fake = FakeRnodeconf::install(n.dir.path());
    fake.hang(5);
    let mut runner = TimedRunner::new(n.dir.path());
    runner.timeout = Duration::from_millis(300);
    let step = RnodeStep::new(Arc::new(runner), fake.program.clone(), false);
    let probe = Probe {
        device_id: "abcd1234abcd1234".into(),
        uplink: node111::daemon::meshtastic::meshmud::Uplink::serial(&heltec_uplink()).unwrap(),
        path: n.tree.tty(),
        task: "identify:abcd1234abcd1234".into(),
    };
    let t0 = std::time::Instant::now();
    match step.run(&probe).await.unwrap() {
        Outcome::Error(e) => assert!(e.contains("timed out"), "{e}"),
        o => panic!("{o:?}"),
    }
    assert!(t0.elapsed() < Duration::from_secs(4), "{:?}", t0.elapsed());
}

// --- MeshCore over a pty (a stub modelled on kiss_modem_protocol.md only)

/// A pty pair: the master for the stub, the slave's path for the probe. The
/// slave stays open here too, so the master never sees a hang-up.
struct Pty {
    master: libc::c_int,
    slave: libc::c_int,
    path: PathBuf,
}

impl Pty {
    fn open() -> Pty {
        let (mut master, mut slave) = (0, 0);
        // SAFETY: openpty fills both fds; null name/termios/winsize allowed.
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert_eq!(rc, 0, "openpty: {}", std::io::Error::last_os_error());
        let mut buf = [0 as libc::c_char; 128];
        // SAFETY: buf is valid for its length.
        let rc = unsafe { libc::ptsname_r(master, buf.as_mut_ptr(), buf.len()) };
        assert_eq!(rc, 0);
        // SAFETY: ptsname_r NUL-terminated buf.
        let path = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        // raw on our side too: no echo, no line discipline
        // SAFETY: valid fd and termios.
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            libc::tcgetattr(slave, &mut t);
            libc::cfmakeraw(&mut t);
            libc::tcsetattr(slave, libc::TCSANOW, &t);
        }
        Pty {
            master,
            slave,
            path: PathBuf::from(path),
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        // SAFETY: both fds are ours.
        unsafe {
            libc::close(self.master);
            libc::close(self.slave);
        }
    }
}

/// How the stub modem answers.
#[derive(Clone, Copy, PartialEq)]
enum Modem {
    /// Version and Identity, as the document specifies.
    MeshCore,
    /// Version only (a partial answer).
    VersionOnly,
    /// Nothing at all (another firmware).
    Silent,
}

/// The stub modem: KISS deframing written from the document (FEND/FESC,
/// the type byte, SetHardware 0x06 sub-commands, `command | 0x80` answers).
/// Records the sub-commands it was asked. [`ModemStub::stop`] joins it before
/// the pty closes (a stray reader on a reused fd would steal bytes).
struct ModemStub {
    asked: Arc<Mutex<Vec<u8>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ModemStub {
    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }

    fn asked(&self) -> Vec<u8> {
        self.asked.lock().unwrap().clone()
    }
}

fn spawn_modem(master: libc::c_int, mode: Modem) -> ModemStub {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (a2, s2) = (Arc::clone(&asked), Arc::clone(&stop));
    let thread = std::thread::spawn(move || {
        let mut frame: Vec<u8> = Vec::new();
        let (mut inside, mut esc) = (false, false);
        let write = |bytes: &[u8]| {
            // SAFETY: bytes valid for its length.
            unsafe { libc::write(master, bytes.as_ptr() as *const libc::c_void, bytes.len()) };
        };
        while !s2.load(Ordering::SeqCst) {
            let mut pfd = libc::pollfd {
                fd: master,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one valid pollfd.
            if unsafe { libc::poll(&mut pfd, 1, 50) } <= 0 {
                continue;
            }
            let mut buf = [0u8; 256];
            // SAFETY: buf valid for its length.
            let n = unsafe { libc::read(master, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            for &b in &buf[..n as usize] {
                match b {
                    0xC0 => {
                        if inside && frame.len() >= 2 && frame[0] == 0x06 {
                            let sub = frame[1];
                            a2.lock().unwrap().push(sub);
                            match (mode, sub) {
                                (Modem::Silent, _) => {}
                                (_, 0x11) => write(&[0xC0, 0x06, 0x91, 0x03, 0x00, 0xC0]),
                                (Modem::MeshCore, 0x01) => {
                                    // a key holding FEND and FESC: escaped on the wire
                                    let mut out = vec![0xC0, 0x06, 0x81];
                                    for i in 0..32u8 {
                                        match i {
                                            0 => out.extend([0xDB, 0xDC]),
                                            1 => out.extend([0xDB, 0xDD]),
                                            _ => out.push(i),
                                        }
                                    }
                                    out.push(0xC0);
                                    write(&out);
                                }
                                _ => {}
                            }
                        }
                        frame.clear();
                        inside = true;
                        esc = false;
                    }
                    0xDB if inside => esc = true,
                    _ if inside => {
                        frame.push(match (esc, b) {
                            (true, 0xDC) => 0xC0,
                            (true, 0xDD) => 0xDB,
                            (_, b) => b,
                        });
                        esc = false;
                    }
                    _ => {}
                }
            }
        }
    });
    ModemStub {
        asked,
        stop,
        thread: Some(thread),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_meshcore_match_over_the_pty_stub() {
    let n = node();
    let pty = Pty::open();
    n.tree.tty_to(&pty.path);
    let mut modem = spawn_modem(pty.master, Modem::MeshCore);
    let fake = FakeRnodeconf::install(n.dir.path());
    fake.answer("no-rnode.txt");
    let log = Arc::new(Mutex::new(Vec::new()));
    let ident = n.identify(
        vec![
            Arc::new(RnodeStep::new(
                Arc::new(TimedRunner::new(n.dir.path())),
                fake.program.clone(),
                false,
            )),
            FakeStep::new(StepKind::Meshtastic, Outcome::NoMatch, &log),
            Arc::new(MeshcoreStep::new(Duration::from_secs(3), false)),
        ],
        Arc::new(FakeClock::default()),
    );
    n.tick().await;
    let h = n.heltec();
    let report = ident.run(&n.api, &h.id, Consent::Given).await.unwrap();
    modem.stop();
    assert_eq!(
        report
            .steps
            .iter()
            .map(|s| s.outcome.clone())
            .collect::<Vec<_>>()[..2],
        [Outcome::NoMatch, Outcome::NoMatch]
    );
    assert_eq!(
        report.record.firmware.unwrap(),
        DeviceFirmware {
            family: FirmwareFamily::Meshcore,
            version: Some("3".into()),
            source: FirmwareSource::Probe,
        }
    );
    assert_eq!(n.stored_firmware(&h.id)["family"], "meshcore");
    assert_eq!(modem.asked(), [0x11, 0x01], "GetVersion, then GetIdentity");
    assert!(n.ports.held().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meshcore_silence_is_no_match_and_a_partial_answer_an_error() {
    for (mode, want_no_match) in [(Modem::Silent, true), (Modem::VersionOnly, false)] {
        let pty = Pty::open();
        let mut modem = spawn_modem(pty.master, mode);
        let step = MeshcoreStep::new(Duration::from_millis(400), false);
        let probe = Probe {
            device_id: "abcd1234abcd1234".into(),
            uplink: node111::daemon::meshtastic::meshmud::Uplink::serial("/dev/ttyACM9").unwrap(),
            path: pty.path.clone(),
            task: "identify:abcd1234abcd1234".into(),
        };
        let t0 = std::time::Instant::now();
        let got = step.run(&probe).await.unwrap();
        modem.stop();
        assert!(t0.elapsed() < Duration::from_secs(2), "the timeout holds");
        if want_no_match {
            assert_eq!(got, Outcome::NoMatch);
            assert_eq!(modem.asked(), [0x11]);
        } else {
            assert!(matches!(got, Outcome::Error(_)), "{got:?}");
            assert_eq!(modem.asked(), [0x11, 0x01]);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_offer_appears_whenever_any_step_can_run() {
    let n = node();
    // Layer 5 alone (no Meshtastic tools, no device-core view): not offered
    n.tick().await;
    assert!(!n.heltec().actions.iter().any(|a| a == "identify"));
    // only the MeshCore probe can run here: offered
    let ident = n.identify(
        vec![
            Arc::new(RnodeStep::new(
                Arc::new(TimedRunner::new(n.dir.path())),
                n.dir.path().join("venvs/rns/bin/rnodeconf"),
                false,
            )),
            Arc::new(MeshtasticStep::new(None)),
            Arc::new(MeshcoreStep::new(Duration::from_millis(100), false)),
        ],
        Arc::new(FakeClock::default()),
    );
    assert_eq!(
        ident.view(),
        IdentifyView {
            serial: true,
            tcp: false
        }
    );
    n.tick().await;
    n.tick().await;
    assert!(n.heltec().actions.iter().any(|a| a == "identify"));
    // on a phone nothing serial can run: not offered
    let n2 = node();
    n2.identify(
        vec![Arc::new(MeshcoreStep::new(
            Duration::from_millis(100),
            true,
        ))],
        Arc::new(FakeClock::default()),
    );
    n2.tick().await;
    assert!(!n2.heltec().actions.iter().any(|a| a == "identify"));
}

// --- Meshtastic through the stub meshmud downlink (the Layer 5 harness shape)

fn framed(p: &[u8]) -> Vec<u8> {
    let mut v = vec![0x94, 0xc3, (p.len() >> 8) as u8, p.len() as u8];
    v.extend_from_slice(p);
    v
}

/// meshmud's downlink, in process: `want_config` answered with
/// `my_node_info` (JSON stands in for protobuf, which 111d never decodes).
fn stub_downlink() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(mut c) = c else { break };
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = match c.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    buf.extend_from_slice(&chunk[..n]);
                    while buf.len() >= 4 && buf[..2] == [0x94, 0xc3] {
                        let len = usize::from(u16::from_be_bytes([buf[2], buf[3]]));
                        if buf.len() < 4 + len {
                            break;
                        }
                        let p = buf[4..4 + len].to_vec();
                        buf.drain(..4 + len);
                        if p == b"want_config" {
                            let info = json!({"my_node_info": {
                                "num": 0x0a0a_0a0a_u32, "id": "!0a0a0a0a",
                                "long_name": "Bench board", "short_name": "BB",
                                "hw_model": "HELTEC_V3", "firmware_version": "2.7.11.abcdef0"}});
                            let _ = c.write_all(&framed(info.to_string().as_bytes()));
                        }
                    }
                }
            });
        }
    });
    addr
}

const FAKE_MESHMUD: &str = r#"#!/bin/sh
echo "meshmud $$" >> '@LAUNCHES@'
exec sleep 120
"#;

const FAKE_READER: &str = r#"
import json, socket, sys
args = sys.argv[1:]
def opt(n):
    return args[args.index(n) + 1] if n in args else None
once = "--once" in args
host, port = opt("--endpoint").rsplit(":", 1)
s = socket.create_connection((host, int(port)), timeout=float(opt("--timeout") or 30))
p = b"want_config"
s.sendall(bytes([0x94, 0xC3, 0, len(p)]) + p)
buf = b""
while True:
    chunk = s.recv(4096)
    if not chunk:
        sys.exit(3)
    buf += chunk
    i = buf.find(b"\x94\xc3")
    if i >= 0 and len(buf) >= i + 4:
        n = (buf[i + 2] << 8) | buf[i + 3]
        if len(buf) >= i + 4 + n:
            msg = json.loads(buf[i + 4:i + 4 + n])
            if once and "my_node_info" in msg:
                print(json.dumps({"schema": 1, "my_node_info": msg["my_node_info"]}))
                sys.exit(0)
            if once:
                sys.exit(3)
            import time
            time.sleep(120)
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_meshtastic_match_through_the_stub_binds_and_hands_the_port_to_meshmud() {
    let Some(py) = node111::daemon::services::probe::which("python3", &[]) else {
        if std::env::var_os("NODE111_SKIP_PYTHON_TESTS").is_some_and(|v| v == "1") {
            eprintln!("python3 not found — the Meshtastic step test SKIPPED");
            return;
        }
        panic!("python3 not found (set NODE111_SKIP_PYTHON_TESTS=1 to skip on purpose)");
    };
    let n = node();
    let root = n.dir.path();
    let launches = root.join("launches.log");
    let bin = root.join("fakebin/meshmud");
    w(
        &bin,
        &FAKE_MESHMUD.replace("@LAUNCHES@", launches.to_str().unwrap()),
    );
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let python = root.join("venvs/presence/bin/python");
    std::fs::create_dir_all(python.parent().unwrap()).unwrap();
    symlink(&py, &python).unwrap();
    let script = root.join("reader/meshtastic_presence.py");
    w(&script, FAKE_READER);
    let downlink = stub_downlink();
    let mut cfg = MeshtasticConfig::production(&n.layout, false);
    cfg.downlink = downlink;
    cfg.shim = ShimConfig::production(downlink);
    cfg.meshmud_bin = bin;
    cfg.reader_python = python;
    cfg.reader_script = script;
    cfg.dev_root = n.tree.root.clone();
    cfg.tick = Duration::from_secs(3600);
    cfg.identify_timeout = Duration::from_secs(20);
    cfg.downlink_wait = Duration::from_secs(5);
    cfg.stop_grace = Duration::from_secs(2);
    cfg.meshmud_backoff = BackoffPolicy {
        initial_ms: 100,
        max_ms: 200,
        jitter_pct: 0,
        ..BackoffPolicy::default()
    };
    let m = Meshtastic::new(cfg, Arc::clone(&n.ports));
    n.api.insert_ext(Arc::clone(&m));
    m.start(
        Arc::clone(&n.api),
        Arc::clone(&n.reg),
        Supervised::default(),
    )
    .await;
    let fake = FakeRnodeconf::install(root);
    fake.answer("no-rnode.txt");
    let log = Arc::new(Mutex::new(Vec::new()));
    let ident = n.identify(
        vec![
            Arc::new(RnodeStep::new(
                Arc::new(TimedRunner::new(root)),
                fake.program.clone(),
                false,
            )),
            Arc::new(MeshtasticStep::new(Some(Arc::clone(&m)))),
            FakeStep::new(StepKind::Meshcore, Outcome::NoMatch, &log),
        ],
        Arc::new(FakeClock::default()),
    );
    n.tick().await;
    let h = n.heltec();
    let report = ident.run(&n.api, &h.id, Consent::Given).await.unwrap();
    assert_eq!(report.steps[0].outcome, Outcome::NoMatch, "rnode first");
    let fw = report.record.firmware.clone().unwrap();
    assert_eq!(fw.family, FirmwareFamily::Meshtastic);
    assert_eq!(fw.version.as_deref(), Some("2.7.11.abcdef0"));
    assert!(log.lock().unwrap().is_empty(), "MeshCore never ran");
    // bound and handed over: the claims are meshmud's, the child serves
    let (bid, b) = n.reg.meshtastic_binding().expect("bound");
    assert_eq!((bid.as_str(), b.node_num), (h.id.as_str(), 0x0a0a_0a0a));
    let held: Vec<(String, String)> = n
        .ports
        .held()
        .into_iter()
        .map(|(k, o)| (k, o.task))
        .collect();
    assert!(
        held.contains(&(heltec_uplink(), "meshmud".into())),
        "{held:?}"
    );
    assert!(
        held.contains(&(downlink.to_string(), "meshmud".into())),
        "{held:?}"
    );
    assert_eq!(
        n.store()["devices"][&h.id]["meshtastic"]["node_id"],
        "!0a0a0a0a"
    );
    m.stop_and_wait(Duration::from_secs(2)).await;
}

// ================================================================ applier

fn out(stdout: &str) -> Output {
    Output {
        status: 0,
        stdout: stdout.to_string(),
        stderr: String::new(),
    }
}

/// rnstatus: records every argv (after `--config <dir>`); a reading answers
/// the current `read_out`.
#[derive(Clone)]
struct Rec {
    log: Arc<Mutex<Vec<String>>>,
    read_out: Arc<Mutex<String>>,
}

impl Runner for Rec {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        assert!(program.ends_with("venvs/rns/bin/rnstatus"), "{program}");
        assert_eq!(args[0], "--config");
        if args.len() == 2 {
            self.log.lock().unwrap().push("rnstatus read".into());
            return Ok(out(&self.read_out.lock().unwrap()));
        }
        self.log
            .lock()
            .unwrap()
            .push(format!("rnstatus {} {}", args[2], args[3]));
        Ok(out(""))
    }
    fn run_as(&self, _: &str, _: &str, _: &[&str]) -> anyhow::Result<Output> {
        anyhow::bail!("not used")
    }
}

struct Hooks {
    log: Arc<Mutex<Vec<String>>>,
}

impl RestartHooks for Hooks {
    fn dependants(&self) -> BoxFut<'_, Vec<String>> {
        Box::pin(async { vec!["lxmf".to_string()] })
    }
    fn stop_before<'a>(&'a self, id: &'a str) -> BoxFut<'a, bool> {
        self.log.lock().unwrap().push(format!("stop {id}"));
        Box::pin(async { true })
    }
    fn start_after<'a>(&'a self, id: &'a str) -> BoxFut<'a, ()> {
        self.log.lock().unwrap().push(format!("start {id}"));
        Box::pin(async {})
    }
}

struct FakeRnsd {
    log: Arc<Mutex<Vec<String>>>,
    running: AtomicBool,
}

impl RnsdChild for FakeRnsd {
    fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
    fn restart(&self, _grace: Duration) -> BoxFut<'_, bool> {
        self.log.lock().unwrap().push("restart rnsd".into());
        Box::pin(async { true })
    }
}

/// The production probe with two switches for injected failures.
struct TestProbe {
    inner: SpineProbe,
    helpers_ok: Arc<AtomicBool>,
    region_breaks: Arc<AtomicBool>,
}

impl ApplyProbe for TestProbe {
    fn interfaces(&self) -> applier::BoxFut<'_, Result<Vec<InterfaceFacts>, String>> {
        self.inner.interfaces()
    }
    fn rnsd_running(&self) -> bool {
        self.inner.rnsd_running()
    }
    fn helpers_attached(&self) -> applier::BoxFut<'_, bool> {
        let ok = self.helpers_ok.load(Ordering::SeqCst);
        Box::pin(async move { ok })
    }
    fn file_region(&self) -> applier::BoxFut<'_, Result<Option<String>, String>> {
        if self.region_breaks.load(Ordering::SeqCst) {
            return Box::pin(async { Err("interfaces.toml does not re-parse (injected)".into()) });
        }
        self.inner.file_region()
    }
}

struct Devices(HashMap<String, String>);

impl DeviceLookup for Devices {
    fn serial_port(&self, id: &str) -> Result<String, TargetError> {
        self.0.get(id).cloned().ok_or(TargetError::NotFound)
    }
}

struct Shots {
    log: Arc<Mutex<Vec<String>>>,
}

impl OneShots for Shots {
    fn identify<'a>(&'a self, id: &'a str) -> applier::BoxFut<'a, Result<Value, OneShotError>> {
        self.log.lock().unwrap().push(format!("identify {id}"));
        Box::pin(async { Ok(json!({"firmware": {"family": "rnode"}})) })
    }
    fn restart<'a>(&'a self, id: &'a str) -> applier::BoxFut<'a, Result<Value, OneShotError>> {
        self.log.lock().unwrap().push(format!("restart {id}"));
        let id = id.to_string();
        Box::pin(async move {
            if id == "agent" {
                Err(OneShotError::Refused(ProblemWith::new(Problem::new(
                    axum::http::StatusCode::CONFLICT,
                    "not_supervised",
                    "This service is not run by the node itself.",
                ))))
            } else {
                Ok(json!({"restarting": true}))
            }
        })
    }
}

const DEV: &str = "abcd1234abcd1234";
const RNODE_NAME: &str = "RNode abcd1234";

struct ApplierNode {
    _dir: tempfile::TempDir,
    root: PathBuf,
    log: Arc<Mutex<Vec<String>>>,
    rec: Rec,
    spine: Arc<Spine>,
    helpers_ok: Arc<AtomicBool>,
    region_breaks: Arc<AtomicBool>,
    decisions: PathBuf,
}

/// A text rnstatus reading listing the shared instance and `ifaces`.
fn reading(ifaces: &[(&str, bool)]) -> String {
    let mut s = String::from(
        " Shared Instance[rns/111]\n    Status    : Up\n    Serving   : 1 program\n\n",
    );
    for (name, up) in ifaces {
        s.push_str(&format!(
            " RNodeInterface[{name}]\n    Status    : {}\n    Mode      : Full\n\n",
            if *up { "Up" } else { "Down" }
        ));
    }
    s
}

impl ApplierNode {
    /// A spine on a fresh root (rnsd a fake that runs, one dependant
    /// `lxmf`); `edit` shapes the starting file.
    async fn new(edit: impl FnOnce(&mut interfaces::InterfacesFile)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let layout = DaemonLayout {
            root: root.clone(),
            etc: root.join("etc"),
        };
        let st = layout.state_dir();
        std::fs::create_dir_all(&st).unwrap();
        let mut f = interfaces::InterfacesFile::default();
        edit(&mut f);
        interfaces::save(&st, &f).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let rec = Rec {
            log: Arc::clone(&log),
            read_out: Arc::new(Mutex::new(reading(&[]))),
        };
        let startup =
            interfaces::startup(&st, &st.join("devices.json"), &config::config_path(&layout));
        let rn = Arc::new(Rnstatus::new(Arc::new(rec.clone()), &root));
        let spine = Spine::new(layout, startup, rn, false, Some(Category::Hub4A));
        spine.set_rnsd(Arc::new(FakeRnsd {
            log: Arc::clone(&log),
            running: AtomicBool::new(true),
        }));
        spine.set_restart_hooks(Arc::new(Hooks {
            log: Arc::clone(&log),
        }));
        spine.set_grace(Duration::from_millis(10));
        spine.write_config().unwrap();
        log.lock().unwrap().clear();
        ApplierNode {
            decisions: root.join("agent/decisions"),
            _dir: dir,
            root,
            log,
            rec,
            spine,
            helpers_ok: Arc::new(AtomicBool::new(true)),
            region_breaks: Arc::new(AtomicBool::new(false)),
        }
    }

    fn applier(&self, window: Duration) -> Arc<applier::Applier> {
        let rnsd: Arc<dyn RnsdChild> = Arc::new(FakeRnsd {
            log: Arc::new(Mutex::new(Vec::new())),
            running: AtomicBool::new(true),
        });
        let probe = TestProbe {
            inner: SpineProbe::new(Arc::clone(&self.spine), Some(rnsd), None),
            helpers_ok: Arc::clone(&self.helpers_ok),
            region_breaks: Arc::clone(&self.region_breaks),
        };
        let mut devs = HashMap::new();
        devs.insert(DEV.to_string(), "/dev/ttyACM0".to_string());
        applier::Applier::new(
            Arc::clone(&self.spine),
            Arc::new(probe),
            Arc::new(Devices(devs)),
            Arc::new(Shots {
                log: Arc::clone(&self.log),
            }),
            Arc::new(DecisionFile::new(self.decisions.clone())),
            ApplierConfig {
                probe_window: window,
                probe_interval: Duration::from_millis(20),
            },
        )
    }

    fn set_reading(&self, text: String) {
        *self.rec.read_out.lock().unwrap() = text;
    }

    fn files(&self) -> (Vec<u8>, Vec<u8>) {
        (
            std::fs::read(self.root.join("state/interfaces.toml")).unwrap(),
            std::fs::read(self.root.join("rns/config")).unwrap(),
        )
    }

    /// The log without the probe's readings (the control calls in order).
    fn controls(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.lock().unwrap())
            .into_iter()
            .filter(|l| l != "rnstatus read")
            .collect()
    }

    fn reads(&self) -> usize {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| *l == "rnstatus read")
            .count()
    }

    fn decision(&self, id: &str) -> Option<Value> {
        let b = std::fs::read(self.decisions.join(format!("{id}.json"))).ok()?;
        Some(serde_json::from_slice(&b).unwrap())
    }
}

fn req(id: &str, actions: Value) -> ApplierRequest {
    let body = json!({"why": "test", "actions": actions}).to_string();
    ApplierRequest::from_body(id, &body).unwrap()
}

fn upsert() -> Value {
    json!([{"action": "rns.interface.upsert", "device_id": DEV, "type": "RNodeInterface",
            "params": {"frequency": 869_525_000, "bandwidth": 125_000, "txpower": 14,
                       "spreadingfactor": 8, "codingrate": 5}}])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upsert_keeps_an_interface_rnstatus_lists_up() {
    let a = ApplierNode::new(|f| f.radio.region = Some("EU_868".into())).await;
    a.set_reading(reading(&[(RNODE_NAME, true)]));
    let report = a
        .applier(Duration::from_secs(5))
        .apply(req("prop-up-1", upsert()))
        .await
        .unwrap();
    assert!(report.applied && !report.reverted, "{report:?}");
    assert_eq!(a.controls(), [format!("rnstatus --attach {RNODE_NAME}")]);
    let f = a.spine.file();
    let i = f.interface(RNODE_NAME).unwrap();
    assert_eq!(
        (i.device.as_deref(), i.port.as_deref()),
        (Some(DEV), Some("/dev/ttyACM0"))
    );
    let d = a.decision("prop-up-1").unwrap();
    assert_eq!(d["decision"], "accepted");
    assert_eq!(d["applied"]["applied"], true);
    assert_eq!(d["applied"]["actions"][0]["action"], "rns.interface.upsert");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upsert_reverts_byte_exact_when_the_rnode_shows_down() {
    let a = ApplierNode::new(|f| f.radio.region = Some("EU_868".into())).await;
    let before = a.files();
    // an RNode that cannot open shows down while rnsd runs
    a.set_reading(reading(&[(RNODE_NAME, false)]));
    let report = a
        .applier(Duration::from_millis(300))
        .apply(req("prop-down-1", upsert()))
        .await
        .unwrap();
    assert!(!report.applied && report.reverted, "{report:?}");
    assert!(
        report.reason.as_deref().unwrap().contains("is down"),
        "{report:?}"
    );
    assert!(a.reads() >= 2, "probed until the window closed");
    assert_eq!(
        a.controls(),
        [
            format!("rnstatus --attach {RNODE_NAME}"),
            format!("rnstatus --detach {RNODE_NAME}"),
        ]
    );
    assert_eq!(
        a.files(),
        before,
        "interfaces.toml and rns/config restored byte-exact"
    );
    assert!(a.spine.file().interface(RNODE_NAME).is_none());
    let d = a.decision("prop-down-1").unwrap();
    assert_eq!(d["applied"]["reverted"], true);
    assert_eq!(d["applied"]["applied"], false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_upsert_without_a_region_is_refused_region_unset_and_writes_nothing() {
    let a = ApplierNode::new(|_| {}).await;
    let before = a.files();
    let e = a
        .applier(Duration::from_secs(1))
        .apply(req("prop-noregion", upsert()))
        .await
        .unwrap_err();
    let ApplyError::Refused(Refusal::RegionUnset(_)) = &e else {
        panic!("{e:?}")
    };
    let p = e.problem();
    assert_eq!(p.body()["reason_code"], "region_unset");
    assert_eq!(p.problem.status, 400);
    assert!(
        a.decision("prop-noregion").is_none(),
        "the proposal stays open"
    );
    assert!(a.controls().is_empty());
    assert_eq!(a.files(), before);
    // a region set earlier in the same proposal counts
    let both = json!([
        {"action": "radio.region.set", "region": "EU_868"},
        upsert()[0].clone()
    ]);
    a.set_reading(reading(&[(RNODE_NAME, true)]));
    let report = a
        .applier(Duration::from_secs(5))
        .apply(req("prop-both", both))
        .await
        .unwrap();
    assert!(report.applied, "{report:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_keeps_when_absent_and_reverts_with_a_reattach_when_still_listed() {
    let start = |f: &mut interfaces::InterfacesFile| {
        f.radio.region = Some("EU_868".into());
        f.interfaces
            .push(rnode_entry("Roof RNode", "/dev/ttyACM1", None));
    };
    // still listed after the window: revert, re-attached
    let a = ApplierNode::new(start).await;
    let before = a.files();
    a.set_reading(reading(&[("Roof RNode", true)]));
    let rm = json!([{"action": "rns.interface.remove", "name": "Roof RNode"}]);
    let report = a
        .applier(Duration::from_millis(300))
        .apply(req("prop-rm-1", rm.clone()))
        .await
        .unwrap();
    assert!(report.reverted, "{report:?}");
    assert_eq!(
        a.controls(),
        [
            "rnstatus --detach Roof RNode",
            "rnstatus --attach Roof RNode"
        ]
    );
    assert_eq!(a.files(), before);
    assert!(a.spine.file().interface("Roof RNode").is_some());
    // gone from rnstatus: kept
    let a = ApplierNode::new(start).await;
    a.set_reading(reading(&[]));
    let report = a
        .applier(Duration::from_secs(5))
        .apply(req("prop-rm-2", rm))
        .await
        .unwrap();
    assert!(report.applied && !report.reverted, "{report:?}");
    assert_eq!(a.controls(), ["rnstatus --detach Roof RNode"]);
    // an unknown name: 404, nothing written
    let e = a
        .applier(Duration::from_secs(1))
        .apply(req(
            "prop-rm-3",
            json!([{"action": "rns.interface.remove", "name": "Nope"}]),
        ))
        .await
        .unwrap_err();
    assert_eq!(e.problem().problem.status, 404);
    assert!(a.decision("prop-rm-3").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn posture_restarts_with_the_helper_and_reverts_when_it_does_not_return() {
    let a = ApplierNode::new(|_| {}).await;
    let before = a.files();
    a.helpers_ok.store(false, Ordering::SeqCst);
    let posture = json!([{"action": "rns.posture.set", "enable_transport": false}]);
    let report = a
        .applier(Duration::from_millis(300))
        .apply(req("prop-p-1", posture.clone()))
        .await
        .unwrap();
    assert!(report.reverted, "{report:?}");
    assert!(
        report.reason.as_deref().unwrap().contains("helper"),
        "{report:?}"
    );
    // stop the helper, restart rnsd, start the helper — then the same again
    // for the revert; never an rnstatus control call
    assert_eq!(
        a.controls(),
        [
            "stop lxmf",
            "restart rnsd",
            "start lxmf",
            "stop lxmf",
            "restart rnsd",
            "start lxmf"
        ]
    );
    assert_eq!(a.files(), before);
    // instance up and the helper back: kept
    a.helpers_ok.store(true, Ordering::SeqCst);
    let report = a
        .applier(Duration::from_secs(5))
        .apply(req("prop-p-2", posture))
        .await
        .unwrap();
    assert!(report.applied && !report.reverted, "{report:?}");
    assert_eq!(a.controls(), ["stop lxmf", "restart rnsd", "start lxmf"]);
    let text = std::fs::read_to_string(a.root.join("rns/config")).unwrap();
    assert!(text.contains("enable_transport = No"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn region_set_keeps_after_a_reparse_and_reverts_when_it_breaks() {
    let a = ApplierNode::new(|_| {}).await;
    let before = a.files();
    let region = json!([{"action": "radio.region.set", "region": "EU_868"}]);
    a.region_breaks.store(true, Ordering::SeqCst);
    let report = a
        .applier(Duration::from_millis(200))
        .apply(req("prop-r-1", region.clone()))
        .await
        .unwrap();
    assert!(report.reverted, "{report:?}");
    assert_eq!(a.files(), before);
    assert_eq!(a.spine.file().radio.region, None);
    a.region_breaks.store(false, Ordering::SeqCst);
    let report = a
        .applier(Duration::from_secs(5))
        .apply(req("prop-r-2", region))
        .await
        .unwrap();
    assert!(report.applied, "{report:?}");
    assert_eq!(a.spine.file().radio.region.as_deref(), Some("EU_868"));
    assert!(a.controls().is_empty(), "no radio effect by itself");
    // an unknown region: 400, nothing written
    let e = a
        .applier(Duration::from_secs(1))
        .apply(req(
            "prop-r-3",
            json!([{"action": "radio.region.set", "region": "MOON_1"}]),
        ))
        .await
        .unwrap_err();
    assert_eq!(e.problem().problem.status, 400);
    // the spine saved the file but could not render (a newer release owns
    // rns/config): the saved TOML is put back byte-exact
    let a = ApplierNode::new(|_| {}).await;
    let toml_before = a.files().0;
    std::fs::write(a.root.join("rns/config"), "schema-version = 99\n").unwrap();
    let report = a
        .applier(Duration::from_secs(1))
        .apply(req(
            "prop-r-4",
            json!([{"action": "radio.region.set", "region": "EU_868"}]),
        ))
        .await
        .unwrap();
    assert!(!report.applied, "{report:?}");
    assert!(
        report.reason.as_deref().unwrap().contains("not_applied"),
        "{report:?}"
    );
    assert_eq!(a.files().0, toml_before, "interfaces.toml put back");
    assert_eq!(a.spine.file().radio.region, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_crash_mid_apply_leaves_pending_and_a_restart_never_applies_twice() {
    let a = ApplierNode::new(|f| f.radio.region = Some("EU_868".into())).await;
    // the probe never sees it: the apply is stuck in its 60 s window
    a.set_reading(reading(&[]));
    let ap = a.applier(Duration::from_secs(60));
    let task = tokio::spawn({
        let ap = Arc::clone(&ap);
        async move { ap.apply(req("prop-crash", upsert())).await }
    });
    wait_until("the attach ran", Duration::from_secs(5), || {
        a.log
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.starts_with("rnstatus --attach"))
    })
    .await;
    // the crash
    task.abort();
    let _ = task.await;
    let d = a.decision("prop-crash").unwrap();
    assert_eq!(d["decision"], "accepted");
    assert_eq!(d["applied"]["pending"], true, "{d}");
    a.controls();
    // "restart": a fresh applier on the same records
    let again = a.applier(Duration::from_secs(1));
    let e = again.apply(req("prop-crash", upsert())).await.unwrap_err();
    assert!(matches!(e, ApplyError::AlreadyDecided), "{e:?}");
    assert_eq!(e.problem().problem.code, "already_decided");
    assert!(a.controls().is_empty(), "never a second apply");
    assert_eq!(
        a.decision("prop-crash").unwrap()["applied"]["pending"],
        true
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_actions_are_refused_and_the_one_shots_route_to_their_handlers() {
    for name in ["meshtastic.set", "ble.pair", "ble.forget", "firmware.flash"] {
        let body = json!({"why": "x", "actions": [{"action": name, "device_id": DEV}]});
        let e = ApplierRequest::from_body("prop-u", &body.to_string()).unwrap_err();
        let BodyError::Refused(Refusal::Unsupported(_)) = &e else {
            panic!("{name}: {e:?}")
        };
        let p = e.problem();
        assert_eq!(
            (p.problem.status, p.problem.code),
            (409, "unsupported_action")
        );
    }
    let a = ApplierNode::new(|_| {}).await;
    let ap = a.applier(Duration::from_secs(1));
    let report = ap
        .apply(req(
            "prop-1shot",
            json!([
                {"action": "identify", "device_id": DEV},
                {"action": "service.restart", "service_id": "transport"}
            ]),
        ))
        .await
        .unwrap();
    assert!(report.applied, "{report:?}");
    assert_eq!(
        a.controls(),
        [format!("identify {DEV}"), "restart transport".to_string()]
    );
    assert_eq!(
        report.actions[0].result.as_ref().unwrap()["firmware"]["family"],
        "rnode"
    );
    assert_eq!(
        a.decision("prop-1shot").unwrap()["applied"]["applied"],
        true
    );
    // a one-shot refused before any effect: 409, the proposal reopens
    let e = ap
        .apply(req(
            "prop-ns",
            json!([{"action": "service.restart", "service_id": "agent"}]),
        ))
        .await
        .unwrap_err();
    assert_eq!(e.problem().problem.code, "not_supervised");
    assert!(a.decision("prop-ns").is_none(), "reopened");
    // a typed request is the same entry point
    let typed = ApplierRequest::new(
        "prop-typed",
        vec![Action::Identify {
            device_id: DEV.into(),
        }],
    )
    .unwrap();
    assert!(applier::apply(&ap, typed).await.unwrap().applied);
}

// ================================================================ the Layer 5 switch

/// A spine on `layout` whose rnstatus answers nothing (no rnsd action is
/// asserted here), after `file` was saved.
fn spine_on(layout: &DaemonLayout, file: &interfaces::InterfacesFile) -> Arc<Spine> {
    let st = layout.state_dir();
    interfaces::save(&st, file).unwrap();
    let startup = interfaces::startup(&st, &st.join("devices.json"), &config::config_path(layout));
    let rec = Rec {
        log: Arc::new(Mutex::new(Vec::new())),
        read_out: Arc::new(Mutex::new(reading(&[]))),
    };
    let rn = Arc::new(Rnstatus::new(Arc::new(rec), &layout.root));
    let spine = Spine::new(layout.clone(), startup, rn, false, None);
    spine.write_config().unwrap();
    spine
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn once_a_spine_is_attached_the_binding_lives_in_interfaces_toml() {
    use node111::daemon::devices::store::MeshtasticBinding;
    use node111::daemon::meshtastic::meshmud::Uplink;
    let n = node();
    let spine = spine_on(&n.layout, &interfaces::InterfacesFile::default());
    n.reg.attach_spine(Arc::clone(&spine));
    n.tick().await;
    let h = n.heltec();
    let b = MeshtasticBinding::new(
        Uplink::serial(&heltec_uplink()).unwrap(),
        0x0a0a_0a0a,
        Some("HELTEC_V3"),
        None,
    )
    .unwrap();
    let fw = DeviceFirmware {
        family: FirmwareFamily::Meshtastic,
        version: Some("2.7.11.abcdef0".into()),
        source: FirmwareSource::Probe,
    };
    let rec = n
        .reg
        .record_identify(&n.api, &h.id, fw, Some(b))
        .await
        .unwrap();
    assert_eq!(rec.service_id.as_deref(), Some("meshtastic"));
    // the writer went through the spine: interfaces.toml holds it …
    let st = n.layout.state_dir();
    let on_disk = interfaces::read(&interfaces::path(&st)).unwrap();
    assert_eq!(on_disk.meshtastic_binding().unwrap().0, h.id);
    // … the reader reads it from there, devices.json is the mirror
    assert_eq!(n.reg.meshtastic_binding().unwrap().0, h.id);
    assert_eq!(
        n.store()["devices"][&h.id]["meshtastic"]["node_id"],
        "!0a0a0a0a"
    );
    assert_eq!(n.stored_firmware(&h.id)["source"], "probe");
    // forget: the reader drops it at once, the file follows
    n.reg.forget(&n.api, &h.id).unwrap();
    assert!(n.reg.meshtastic_binding().is_none());
    wait_until("interfaces.toml unbinds", Duration::from_secs(5), || {
        spine.meshtastic_binding().is_none()
    })
    .await;
    let on_disk = interfaces::read(&interfaces::path(&st)).unwrap();
    assert!(on_disk.meshtastic_binding().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stanza_the_spine_left_out_shows_why_on_the_device_card() {
    let n = node();
    n.reg.set_rnsd_running(|| true);
    let mut f = interfaces::InterfacesFile::default();
    // an RNode entry on the board without a region: never rendered
    f.interfaces
        .push(rnode_entry("Roof RNode", &heltec_uplink(), None));
    let spine = spine_on(&n.layout, &f);
    assert_eq!(spine.left_out()[0].name, "Roof RNode");
    n.reg.attach_spine(Arc::clone(&spine));
    n.tick().await;
    let h = n.heltec();
    assert!(h.driver.is_none(), "{h:?}");
    assert_eq!(h.detail["rns_interface"], "Roof RNode");
    let why = h.detail["rns_left_out"].as_str().unwrap();
    assert!(why.contains("region"), "{why}");
    assert!(h.reason.contains("not in use"), "{}", h.reason);
    // still rnsd's port: identify is refused naming rnsd
    match n.reg.identify_target(&h.id) {
        Err(TargetError::Busy { owner, .. }) => assert_eq!(owner, "rnsd"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_production_one_shots_refuse_before_any_effect() {
    use node111::daemon::devices::applier::DaemonOneShots;
    let n = node();
    let shots = DaemonOneShots::new(&n.api);
    let refused = |r: Result<Value, OneShotError>| match r {
        Err(OneShotError::Refused(p)) => (p.problem.status, p.problem.code),
        other => panic!("{other:?}"),
    };
    // no identify orchestrator in this ApiState: refused, nothing opened
    assert_eq!(refused(shots.identify(DEV).await), (400, "bad_request"));
    // nothing supervised: 409 not_supervised; an unknown id: 404
    assert_eq!(
        refused(shots.restart("transport").await),
        (409, "not_supervised")
    );
    assert_eq!(
        refused(shots.restart("no-such-service").await),
        (404, "not_found")
    );
    // with the orchestrator: the consented run, its problems passed through
    let log = Arc::new(Mutex::new(Vec::new()));
    let ident = n.identify(
        steps3(&log, [Outcome::NoMatch, Outcome::NoMatch, Outcome::NoMatch]),
        Arc::new(FakeClock::default()),
    );
    n.api.insert_ext(Arc::clone(&ident));
    assert_eq!(
        refused(shots.identify("ffff0000ffff0000").await),
        (404, "not_found")
    );
    n.tick().await;
    let id = n.heltec().id;
    let v = shots.identify(&id).await.unwrap();
    assert_eq!(v["firmware"]["family"], "unknown");
    assert!(n.reg.has_consent(&id), "the acceptance is the consent");
    assert_eq!(refused(shots.identify(&id).await), (429, "rate_limited"));
}
