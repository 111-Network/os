//! Layer 5 M1 acceptance (os/5-messaging/README.md §14, TASKS.md row M1):
//! Meshtastic in `111d` without hardware.
//!
//! - A **stub meshmud** TCP server (in-process, std threads) plays meshmud's
//!   downlink: it deframes `0x94 0xC3 len` frames strictly (any byte that is
//!   not a frame header is counted as a framing error), answers the reader's
//!   `want_config` with a fake `FromRadio` stream (debug noise, then a
//!   `my_node_info` frame and a node-list frame — JSON payloads stand in for
//!   protobufs, which 111d never decodes), and can push frames to every open
//!   connection.
//! - A **fake meshmud** binary (`sh`) logs its launch (pid + the rendered
//!   config's mode) and idles until SIGTERM (`exec sleep`, so the supervised
//!   PID is the logged one); a `CRASH` flag makes it exit at once.
//! - A **fake presence reader** (Python, run by the real `python3` behind a
//!   `venvs/presence/bin/python` link) speaks the stub's framing through the
//!   downlink: `--once` prints `my_node_info` (exit 3 when the board is not
//!   Meshtastic); the service mode writes `meshtastic.json` atomically.
//!
//! Covered: config rendering + validation; the consented identify flow
//! (consent, `409 port_busy` for a held board port or downlink, not
//! Meshtastic, meshmud crashing under the identify profile, binding + firmware
//! persisted, the identify child becoming the service child in place — one
//! launch, claims transferred); restart = both children; the identify-born
//! child swapped for a service-policy child after it exits; forget unbinds;
//! the `meshtastic-http` shim round-trip (framing, empty answers, bounded
//! drop-oldest buffer + drop count, per-session connections, revocation, idle,
//! session cap, upstream down); presence ingest hardening; `GET /nearby` + SSE
//! `nearby`; `meshtastic` service LIVE / NOT SET / ERROR through the
//! registry's rules (and the `radio` row agreeing); a presence venv that could
//! not be built (`venvs/presence.status.json`, made by a fake python3/pip:
//! `fake_python`) is the row's ERROR reason; phones never drive USB boards
//! (`can_run_here = false`) while a Wi-Fi board runs.

mod fake_python;

use node111::daemon::dashboard::{
    routes, serve_listener, ApiConfig, ApiState, DashboardState, SharedState,
};
use node111::daemon::devices::{self, DeviceRecord, DeviceRegistry, RegistryConfig, Roots};
use node111::daemon::intelligence::ChildDisplay;
use node111::daemon::meshtastic::meshmud::{self, Uplink, UplinkError};
use node111::daemon::meshtastic::presence::{self, IngestError, Presence, Refresh};
use node111::daemon::meshtastic::shim::{self, MeshtasticShim, ShimConfig};
use node111::daemon::meshtastic::{self as mt, Meshtastic, MeshtasticConfig};
use node111::daemon::panels::ticket::Revoked;
use node111::daemon::panels::{Shim, ShimError, ShimRequest, ShimSession};
use node111::daemon::ports::PortTable;
use node111::daemon::services::{
    self, Computed, Facts, PanelHealth, PanelInfo, PanelState, ServiceId, ServiceState, Supervised,
};
use node111::daemon::supervise::BackoffPolicy;
use node111::daemon::transport::venv::presence as venv_presence;
use node111::daemon::DaemonLayout;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

// ---------------------------------------------------------------- framing

const WANT_CONFIG: &[u8] = b"want_config";
const MY_NUM: u32 = 0x0a0a_0a0a;

fn framed(p: &[u8]) -> Vec<u8> {
    let mut v = vec![0x94, 0xc3, (p.len() >> 8) as u8, p.len() as u8];
    v.extend_from_slice(p);
    v
}

fn my_node_info() -> Value {
    json!({"my_node_info": {
        "num": MY_NUM, "id": "!0a0a0a0a", "long_name": "Bench board", "short_name": "BB",
        "hw_model": "HELTEC_V3", "firmware_version": "2.7.11.abcdef0", "pio_env": "heltec-v3"
    }})
}

/// Two heard nodes; one with a hostile 60-byte name (111d cuts it to 40
/// bytes and strips the escape even though a real reader already would).
fn node_list() -> Value {
    json!({"nodes": [
        {"id": "!1b2c3d4e", "num": 0x1b2c_3d4e_u32, "long_name": "Hilltop", "short_name": "HT",
         "snr": 5.5, "rssi": -97, "hops_away": 1, "last_heard_hint": 1_760_000_000_u32,
         "position": {"lat": 51.5074, "lon": -0.1278, "alt": 35}, "channel_utilization": 7.25},
        {"id": "!00000002", "num": 2, "long_name": format!("\u{1b}[2J{}", "w".repeat(60)),
         "short_name": "W"}
    ]})
}

// ---------------------------------------------------------------- stub meshmud

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Answers `want_config` with `my_node_info` + the node list.
    Meshtastic,
    /// Answers `want_config` with something that is not Meshtastic.
    NotMeshtastic,
}

struct StubState {
    mode: Mode,
    conns: Vec<(usize, TcpStream)>,
    accepted: usize,
    closed: usize,
    received: Vec<Vec<u8>>,
    framing_errors: usize,
}

/// meshmud's TCP downlink, in process.
struct Stub {
    addr: SocketAddr,
    st: Arc<Mutex<StubState>>,
}

impl Stub {
    fn start(mode: Mode) -> Stub {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let st = Arc::new(Mutex::new(StubState {
            mode,
            conns: Vec::new(),
            accepted: 0,
            closed: 0,
            received: Vec::new(),
            framing_errors: 0,
        }));
        let shared = Arc::clone(&st);
        std::thread::spawn(move || {
            for c in l.incoming() {
                let Ok(c) = c else { break };
                let id = {
                    let mut g = shared.lock().unwrap();
                    g.accepted += 1;
                    let id = g.accepted;
                    g.conns.push((id, c.try_clone().unwrap()));
                    id
                };
                let st = Arc::clone(&shared);
                std::thread::spawn(move || serve_conn(id, c, st));
            }
        });
        Stub { addr, st }
    }

    fn g(&self) -> std::sync::MutexGuard<'_, StubState> {
        self.st.lock().unwrap()
    }

    /// Write raw bytes to every open connection.
    fn send_all(&self, bytes: &[u8]) {
        for (_, c) in self.g().conns.iter_mut() {
            let _ = c.write_all(bytes);
        }
    }

    fn accepted(&self) -> usize {
        self.g().accepted
    }

    fn closed(&self) -> usize {
        self.g().closed
    }

    fn received(&self) -> Vec<Vec<u8>> {
        self.g().received.clone()
    }

    fn framing_errors(&self) -> usize {
        self.g().framing_errors
    }
}

fn config_reply(mode: Mode) -> Vec<u8> {
    let mut out = b"DEBUG | 12:00:00 3 [Router] boot\r\n".to_vec();
    match mode {
        Mode::Meshtastic => {
            out.extend(framed(my_node_info().to_string().as_bytes()));
            out.extend(framed(node_list().to_string().as_bytes()));
        }
        Mode::NotMeshtastic => out.extend(framed(br#"{"rebooted":true}"#)),
    }
    out
}

/// Strict deframer: 111d (and the reader) must only ever send whole frames.
fn serve_conn(id: usize, mut c: TcpStream, st: Arc<Mutex<StubState>>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = match c.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);
        while buf.len() >= 4 {
            if buf[..2] != [0x94, 0xc3] {
                st.lock().unwrap().framing_errors += 1;
                buf.clear();
                break;
            }
            let len = usize::from(u16::from_be_bytes([buf[2], buf[3]]));
            if buf.len() < 4 + len {
                break;
            }
            let p = buf[4..4 + len].to_vec();
            buf.drain(..4 + len);
            let mode = {
                let mut g = st.lock().unwrap();
                g.received.push(p.clone());
                g.mode
            };
            if p == WANT_CONFIG {
                let _ = c.write_all(&config_reply(mode));
            }
        }
    }
    let mut g = st.lock().unwrap();
    g.closed += 1;
    g.conns.retain(|(i, _)| *i != id);
}

// ---------------------------------------------------------------- fake tree

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
        let node = self.root.join("dev/ttyUSB0");
        w(&node, "");
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o660)).unwrap();
        rel_link(
            &node,
            &self.root.join("dev/serial/by-id").join(HELTEC_BY_ID),
        );
    }
}

// ---------------------------------------------------------------- fake children

const FAKE_MESHMUD: &str = r#"#!/bin/sh
# fake meshmud (tests/api_v1_meshtastic.rs): log the launch and the rendered
# config's mode, then idle until SIGTERM (exec keeps the logged pid).
if [ "$1" != "start-daemon" ] || [ "$2" != "-config" ] || [ ! -f "$3" ]; then
  echo "fake meshmud: bad arguments: $*" >&2
  exit 2
fi
echo "meshmud $$ $(stat -c %a "$3")" >> '@LAUNCHES@'
if [ -f '@CRASH@' ]; then exit 1; fi
exec sleep 120
"#;

const FAKE_READER: &str = r#"
# fake meshtastic-presence (tests/api_v1_meshtastic.rs): the real CLI's
# arguments, the stub's framing, JSON payloads instead of protobufs.
import json, os, socket, sys, time

args = sys.argv[1:]

def opt(name):
    return args[args.index(name) + 1] if name in args else None

once = "--once" in args
with open("@LAUNCHES@", "a") as f:
    f.write("reader %d %s\n" % (os.getpid(), "once" if once else "service"))
# the tests hold the identify probe open while @HOLD@ exists
while once and os.path.exists("@HOLD@"):
    time.sleep(0.02)
host, port = opt("--endpoint").rsplit(":", 1)
s = socket.create_connection((host, int(port)), timeout=float(opt("--timeout") or 30))
s.settimeout(None if not once else float(opt("--timeout") or 30))
payload = b"want_config"
s.sendall(bytes([0x94, 0xC3, len(payload) >> 8, len(payload) & 0xFF]) + payload)
buf = b""

def frames():
    global buf
    while True:
        i = buf.find(b"\x94\xc3")
        if i >= 0 and len(buf) >= i + 4:
            n = (buf[i + 2] << 8) | buf[i + 3]
            if len(buf) >= i + 4 + n:
                p = buf[i + 4:i + 4 + n]
                buf = buf[i + 4 + n:]
                yield p
                continue
        chunk = s.recv(4096)
        if not chunk:
            return
        buf += chunk

for p in frames():
    msg = json.loads(p)
    if once:
        if "my_node_info" in msg:
            print(json.dumps({"schema": 1, "my_node_info": msg["my_node_info"]}))
            sys.exit(0)
        sys.exit(3)
    if "nodes" in msg:
        out = opt("--out")
        tmp = os.path.join(out, ".meshtastic.json.tmp")
        with open(tmp, "w") as f:
            json.dump({"schema": 1, "nodes": msg["nodes"]}, f)
        os.replace(tmp, os.path.join(out, "meshtastic.json"))
sys.exit(3 if once else 0)
"#;

fn python3() -> Option<PathBuf> {
    node111::daemon::services::probe::which("python3", &[])
}

// ---------------------------------------------------------------- harness

struct Opts {
    phone: bool,
    mode: Mode,
    heltec: bool,
    /// `state/devices.json` written before start.
    seed: Option<Value>,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            phone: false,
            mode: Mode::Meshtastic,
            heltec: true,
            seed: None,
        }
    }
}

struct H {
    dir: tempfile::TempDir,
    api: Arc<ApiState>,
    reg: Arc<DeviceRegistry>,
    m: Arc<Meshtastic>,
    ports: Arc<PortTable>,
    stub: Stub,
    launches: PathBuf,
    crash: PathBuf,
    /// While this file exists the fake reader's `--once` waits before it
    /// connects (holds the identify probe open).
    hold: PathBuf,
}

/// Set to `1` to skip the flow tests on purpose on a machine without
/// `python3`; each skipped test then says so on stderr.
const SKIP_ENV: &str = "NODE111_SKIP_PYTHON_TESTS";

/// The flow-test harness. The fake presence reader needs `python3`; without
/// it the tests **fail** (a silent pass would hide regressions in identify,
/// restart, the states and the shim) unless [`SKIP_ENV`]`=1` asks for a
/// visible skip (`None`).
fn harness(o: Opts) -> Option<H> {
    let Some(py) = python3() else {
        if std::env::var_os(SKIP_ENV).is_some_and(|v| v == "1") {
            eprintln!("python3 not found — Meshtastic flow test SKIPPED ({SKIP_ENV}=1)");
            return None;
        }
        panic!(
            "python3 not found: the Meshtastic flow tests need it for the fake presence reader \
             (install python3, or set {SKIP_ENV}=1 to skip these tests on purpose)"
        );
    };
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    if let Some(seed) = &o.seed {
        w(&state.join("devices.json"), &seed.to_string());
    }
    let tree = Tree::new(dir.path());
    if o.heltec {
        tree.heltec();
    }
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.10.14")));
    let api = ApiState::new(dash, ApiConfig::new(layout.clone(), false));
    let mut rcfg = RegistryConfig::production(&layout);
    rcfg.roots = tree.roots();
    rcfg.probe_addr = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    rcfg.probe_timeout = Duration::from_millis(300);
    let reg = devices::install(&api, rcfg);

    let launches = dir.path().join("launches.log");
    let crash = dir.path().join("CRASH");
    let hold = dir.path().join("HOLD");
    let bin = dir.path().join("fakebin/meshmud");
    w(
        &bin,
        &FAKE_MESHMUD
            .replace("@LAUNCHES@", launches.to_str().unwrap())
            .replace("@CRASH@", crash.to_str().unwrap()),
    );
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let python = dir.path().join("venvs/presence/bin/python");
    std::fs::create_dir_all(python.parent().unwrap()).unwrap();
    symlink(&py, &python).unwrap();
    let script = dir.path().join("reader/meshtastic_presence.py");
    w(
        &script,
        &FAKE_READER
            .replace("@LAUNCHES@", launches.to_str().unwrap())
            .replace("@HOLD@", hold.to_str().unwrap()),
    );

    let stub = Stub::start(o.mode);
    let mut cfg = MeshtasticConfig::production(&layout, o.phone);
    cfg.downlink = stub.addr;
    cfg.shim = ShimConfig::production(stub.addr);
    cfg.meshmud_bin = bin;
    cfg.reader_python = python;
    cfg.reader_script = script;
    cfg.dev_root = tree.root.clone();
    cfg.tick = Duration::from_secs(3600); // the tests tick by hand
    cfg.identify_timeout = Duration::from_secs(20);
    cfg.downlink_wait = Duration::from_secs(5);
    cfg.stop_grace = Duration::from_secs(2);
    cfg.meshmud_backoff = BackoffPolicy {
        initial_ms: 100,
        max_ms: 200,
        jitter_pct: 0,
        ..BackoffPolicy::default()
    };
    let ports = Arc::new(PortTable::new());
    let m = Meshtastic::new(cfg, Arc::clone(&ports));
    api.insert_ext(Arc::clone(&m));
    api.insert_ext(Arc::clone(&ports));
    Some(H {
        dir,
        api,
        reg,
        m,
        ports,
        stub,
        launches,
        crash,
        hold,
    })
}

impl H {
    async fn start(&self) {
        self.m
            .start(
                Arc::clone(&self.api),
                Arc::clone(&self.reg),
                Supervised::default(),
            )
            .await;
        self.reg.tick(&self.api).await;
    }

    fn by_name(&self, name: &str) -> DeviceRecord {
        self.reg
            .list()
            .expect("published")
            .into_iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("no device named {name:?}"))
    }

    fn store(&self) -> Value {
        let raw = std::fs::read_to_string(self.dir.path().join("state/devices.json")).unwrap();
        serde_json::from_str(&raw).unwrap()
    }

    /// Launch-log lines starting with `what` (`meshmud`, `reader`).
    fn launched(&self, what: &str) -> Vec<String> {
        std::fs::read_to_string(&self.launches)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with(what))
            .map(str::to_string)
            .collect()
    }

    fn meshmud_pids(&self) -> Vec<u32> {
        self.launched("meshmud")
            .iter()
            .map(|l| l.split_whitespace().nth(1).unwrap().parse().unwrap())
            .collect()
    }

    /// The meshmud launch PIDs once there are `n` of them. A spawned child
    /// counts as running before its shell has written the launch line, so a
    /// positive count is waited for, never read once.
    async fn meshmud_launches(&self, n: usize) -> Vec<u32> {
        wait_until(
            &format!("{n} meshmud launch(es) logged"),
            Duration::from_secs(5),
            || self.meshmud_pids().len() >= n,
        )
        .await;
        self.meshmud_pids()
    }

    /// PIDs of the reader launches whose mode is `mode` (`once`, `service`).
    fn reader_pids(&self, mode: &str) -> Vec<u32> {
        self.launched("reader")
            .iter()
            .filter(|l| l.ends_with(mode))
            .map(|l| l.split_whitespace().nth(1).unwrap().parse().unwrap())
            .collect()
    }

    fn held(&self) -> Vec<(String, String)> {
        self.ports
            .held()
            .into_iter()
            .map(|(k, o)| (k, o.task))
            .collect()
    }

    fn presence_file(&self) -> PathBuf {
        self.dir.path().join("fabric/presence/meshtastic.json")
    }

    fn meshtastic_row(&self, phone: bool) -> Computed {
        let mut f = Facts {
            phone_like: phone,
            meshmud: Some(self.m.facts()),
            ..Facts::default()
        };
        f.radio.meshmud_bound = self.m.bound_ports();
        service(&f, ServiceId::Meshtastic)
    }

    fn radio_row(&self) -> Computed {
        let mut f = Facts {
            meshmud: Some(self.m.facts()),
            ..Facts::default()
        };
        f.radio.meshmud_bound = self.m.bound_ports();
        f.radio.boards = Some(Vec::new());
        service(&f, ServiceId::Radio)
    }

    fn frames_since(&self, head: u64, event: &str) -> Vec<Value> {
        self.api
            .events
            .subscribe(Some(head))
            .replay
            .iter()
            .filter(|f| f.event == event)
            .map(|f| serde_json::from_str(&f.data).unwrap())
            .collect()
    }

    async fn serve(&self) -> u16 {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(serve_listener(listener, routes::app(Arc::clone(&self.api))));
        port
    }

    async fn stop(&self) {
        self.m.stop_and_wait(Duration::from_secs(2)).await;
    }
}

fn service(f: &Facts, id: ServiceId) -> Computed {
    services::compute(f)
        .into_iter()
        .find(|c| c.id == id)
        .unwrap()
}

async fn wait_until(what: &str, timeout: Duration, mut ok: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + timeout;
    while !ok() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn kill9(pid: u32) {
    let st = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
    assert!(st.success());
}

fn alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
        && !std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .unwrap_or_default()
            .contains(") Z ")
}

// ---------------------------------------------------------------- HTTP

struct Resp {
    status: u16,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {:?}", self.body))
    }
    fn problem(&self, status: u16, code: &str) {
        assert_eq!(self.status, status, "{}", self.body);
        assert_eq!(self.json()["code"], code, "{}", self.body);
    }
}

fn request(port: u16, method: &str, path: &str, body: Option<&str>) -> Resp {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    conn.set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    if method != "GET" {
        req.push_str("Content-Type: application/json\r\n");
    }
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    if let Some(b) = body {
        req.push_str(b);
    }
    conn.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    conn.read_to_end(&mut raw).unwrap();
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Resp {
        status,
        body: body.to_string(),
    }
}

/// Blocking HTTP off the runtime's worker threads.
async fn http(port: u16, method: &'static str, path: String, body: Option<&'static str>) -> Resp {
    tokio::task::spawn_blocking(move || request(port, method, &path, body))
        .await
        .unwrap()
}

fn heltec_uplink() -> String {
    format!("/dev/serial/by-id/{HELTEC_BY_ID}")
}

// ================================================================ tests

#[test]
fn config_rendering_and_uplink_validation() {
    for ok in ["/dev/ttyACM0", "/dev/ttyUSB1", &heltec_uplink()] {
        assert!(Uplink::serial(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "ttyACM0",
        "/dev/",
        "/dev/..",
        "/dev/../etc/shadow",
        "/dev/serial/by-id/../../sda",
        "/dev/serial/by-path/pci-0000:00:14.0-usb-0:1:1.0-port0",
        "/dev/tty ACM0",
        "/dev/ttyACM0\n[[downlink.http]]",
        "/tmp/ttyACM0",
    ] {
        assert!(
            matches!(Uplink::serial(bad), Err(UplinkError::BadPath(_))),
            "{bad:?} must be refused"
        );
    }
    assert_eq!(
        Uplink::tcp("tcp://10.0.0.7:4403").unwrap().display(),
        "tcp://10.0.0.7:4403"
    );
    assert_eq!(Uplink::meshtasticd().display(), "tcp://127.0.0.1:4403");
    for bad in [
        "10.0.0.7",
        "10.0.0.7:0",
        "host:4403\nx",
        "http://10.0.0.7:4403",
    ] {
        assert!(Uplink::tcp(bad).is_err(), "{bad:?}");
    }
    assert_eq!(Uplink::tcp("127.0.0.1:11230"), Err(UplinkError::SelfLoop));

    // one loopback TCP downlink, no HTTP downlink, no mDNS
    let text = meshmud::render_config(
        &Uplink::serial(&heltec_uplink()).unwrap(),
        meshmud::DOWNLINK,
    )
    .unwrap();
    let v: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(
        v["uplink"]["serial"][0]["device"].as_str(),
        Some(heltec_uplink().as_str())
    );
    assert_eq!(
        v["downlink"]["tcp"][0]["address"].as_str(),
        Some("127.0.0.1:11230")
    );
    assert_eq!(v["downlink"]["tcp"][0]["advertise"].as_bool(), Some(false));
    assert!(v["downlink"].get("http").is_none());
    let wifi =
        meshmud::render_config(&Uplink::tcp("10.0.0.7:4403").unwrap(), meshmud::DOWNLINK).unwrap();
    let v: toml::Value = toml::from_str(&wifi).unwrap();
    assert_eq!(
        v["uplink"]["tcp"][0]["address"].as_str(),
        Some("10.0.0.7:4403")
    );
    assert!(meshmud::render_config(
        &Uplink::tcp("10.0.0.7:4403").unwrap(),
        "0.0.0.0:11230".parse().unwrap()
    )
    .is_err());

    let dir = tempfile::tempdir().unwrap();
    let path = meshmud::state_dir(dir.path()).join("meshmud.toml");
    meshmud::write_config(
        &path,
        &Uplink::serial("/dev/ttyACM0").unwrap(),
        meshmud::DOWNLINK,
    )
    .unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);
    let hostile = Uplink::Serial {
        path: "/dev/x\"\n[[downlink.http]]\naddress=\"0.0.0.0:80".into(),
    };
    assert!(meshmud::write_config(&path, &hostile, meshmud::DOWNLINK).is_err());
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("/dev/ttyACM0"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn identify_binds_the_board_and_the_identify_child_becomes_the_service() {
    let Some(h) = harness(Opts::default()) else {
        return;
    };
    h.start().await;
    let port = h.serve().await;
    let board = h.by_name(HELTEC);
    assert!(board.actions.iter().any(|a| a == "identify"), "{board:?}");
    assert!(board.firmware.as_ref().is_none_or(|f| f.version.is_none()));
    let path = format!("/api/v1/devices/{}/identify", board.id);

    // NOT SET before identify: nothing bound, nothing identified
    assert_eq!(h.meshtastic_row(false).state, ServiceState::Unset);
    assert_eq!(h.meshtastic_row(false).reason_code, "no_meshtastic");

    // consent is required, checked before anything is claimed or started
    for body in [
        r#"{}"#,
        r#"{"consent":false}"#,
        r#"{"consent":"yes"}"#,
        "nope",
    ] {
        http(port, "POST", path.clone(), Some(body))
            .await
            .problem(400, "bad_request");
    }
    assert!(h.held().is_empty());
    assert!(h.launched("").is_empty(), "nothing started without consent");

    let head = h.api.events.head();
    let r = http(port, "POST", path.clone(), Some(r#"{"consent":true}"#)).await;
    assert_eq!(r.status, 200, "{}", r.body);
    let rec = r.json();
    assert_eq!(
        rec["firmware"],
        json!({"family": "meshtastic", "version": "2.7.11.abcdef0", "source": "probe"})
    );
    assert_eq!(rec["service_id"], "meshtastic");
    assert_eq!(rec["driver"]["stack"], "meshtastic");
    assert_eq!(rec["driver"]["interface"], "meshmud");
    assert_eq!(rec["driver"]["live"], true);
    assert!(rec["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("meshtastic-firmware")));
    assert!(!rec["actions"]
        .as_array()
        .unwrap()
        .contains(&json!("identify")));
    assert!(
        !h.frames_since(head, "devices").is_empty(),
        "the identify result is published at once"
    );

    // one meshmud launch (0600 config), the --once reader through the downlink
    let pids = h.meshmud_launches(1).await;
    assert_eq!(pids.len(), 1, "{:?}", h.launched(""));
    assert!(h.launched("meshmud")[0].ends_with(" 600"));
    assert_eq!(
        h.launched("reader")[0].split_whitespace().nth(2),
        Some("once")
    );
    assert!(h.stub.received().iter().any(|p| p == WANT_CONFIG));
    assert_eq!(h.stub.framing_errors(), 0);
    // … and that very process is now the service child; claims moved
    let (dev, mm_pid, _) = h.m.service_pids().unwrap();
    assert_eq!(dev, board.id);
    assert_eq!(mm_pid, Some(pids[0]));
    let downlink = h.stub.addr.to_string();
    let mut want = vec![
        (heltec_uplink(), "meshmud".to_string()),
        (downlink.clone(), "meshmud".to_string()),
    ];
    want.sort();
    assert_eq!(h.held(), want);

    // the binding and the identify result are persisted
    let e = &h.store()["devices"][&board.id];
    assert_eq!(e["firmware"]["family"], "meshtastic");
    assert_eq!(e["firmware"]["source"], "probe");
    assert_eq!(
        e["meshtastic"]["uplink"],
        json!({"kind": "serial", "path": heltec_uplink()})
    );
    assert_eq!(e["meshtastic"]["node_num"], MY_NUM);
    assert_eq!(e["meshtastic"]["node_id"], "!0a0a0a0a");
    assert_eq!(e["meshtastic"]["hw_model"], "HELTEC_V3");
    let cfg_text = std::fs::read_to_string(h.dir.path().join("meshtastic/meshmud.toml")).unwrap();
    assert!(cfg_text.contains(&heltec_uplink()));
    assert!(cfg_text.contains(&downlink));
    assert!(cfg_text.contains("advertise = false"));

    // the service reader (started after meshmud) produces the node DB
    wait_until("the service reader", Duration::from_secs(10), || {
        h.launched("reader").iter().any(|l| l.ends_with("service"))
    })
    .await;
    wait_until("meshtastic.json", Duration::from_secs(10), || {
        h.presence_file().is_file()
    })
    .await;
    let head = h.api.events.head();
    h.m.tick_once().await;
    let nearby = h.frames_since(head, "nearby");
    assert_eq!(nearby.len(), 1, "one SSE nearby frame on the change");
    assert_eq!(
        nearby[0]["sources"][0]["nodes"].as_array().unwrap().len(),
        2
    );

    // GET /nearby: grouped by source, names cut to 40 bytes, escape stripped
    let r = http(port, "GET", "/api/v1/nearby".into(), None).await;
    assert_eq!(r.status, 200, "{}", r.body);
    let v = r.json();
    assert_eq!(v["schema"], 1);
    let src = &v["sources"][0];
    assert_eq!(src["kind"], "meshtastic");
    assert_eq!(src["device_id"], board.id.as_str());
    assert_eq!(src["id"], format!("meshtastic:{}", board.id));
    assert_eq!(src["label"], format!("heard via Meshtastic on {HELTEC}"));
    let n0 = &src["nodes"][0];
    assert_eq!(n0["id"], "!00000002");
    assert_eq!(n0["long_name"], format!("[2J{}", "w".repeat(37)));
    assert!(n0["snr"].is_null() && n0["last_heard_hint"].is_null());
    let n1 = &src["nodes"][1];
    assert_eq!(n1["long_name"], "Hilltop");
    assert_eq!(n1["last_heard_hint"], "2025-10-09T08:53:20Z");
    assert_eq!(n1["position"]["alt_m"], 35.0);

    // LIVE: meshmud up, downlink listening, node DB in; `radio` agrees
    let f = h.m.facts();
    assert!(
        f.running && f.downlink_listening && f.node_db_present,
        "{f:?}"
    );
    assert_eq!(f.child_state, ChildDisplay::Running);
    assert_eq!(f.nodes_heard, 2);
    assert_eq!(f.board.as_deref(), Some(HELTEC));
    assert_eq!(f.firmware_version.as_deref(), Some("2.7.11.abcdef0"));
    assert!(f.usb_uplink && f.uplink_lost_secs.is_none() && f.port_held_by.is_none());
    let row = h.meshtastic_row(false);
    assert_eq!(
        (row.state, row.reason_code),
        (ServiceState::Live, "meshtastic_live")
    );
    assert_eq!(row.detail["nodes_heard"], 2);
    let radio = h.radio_row();
    assert_eq!(
        (radio.state, radio.reason_code),
        (ServiceState::Live, "radio_bound_meshtastic")
    );
    assert_eq!(h.m.bound_ports(), vec![heltec_uplink()]);

    // reconcile ticks never reopen the port
    h.m.tick_once().await;
    h.reg.tick(&h.api).await;
    h.m.tick_once().await;
    assert_eq!(h.meshmud_pids(), pids, "still the one identify launch");

    // a bound board is not offered identify again; a second try is port_busy
    http(port, "POST", path.clone(), Some(r#"{"consent":true}"#))
        .await
        .problem(409, "port_busy");
    assert_eq!(h.meshmud_pids(), pids);

    // forget unbinds: both children stop, claims released, Nearby empties
    let head = h.api.events.head();
    let r = http(
        port,
        "DELETE",
        format!("/api/v1/devices/{}", board.id),
        None,
    )
    .await;
    assert!(
        r.status == 200 || r.status == 204,
        "{} {}",
        r.status,
        r.body
    );
    h.m.tick_once().await;
    assert!(h.m.service_pids().is_none());
    assert!(h.held().is_empty(), "{:?}", h.held());
    assert!(!h.presence_file().exists(), "the old node DB is removed");
    assert!(!alive(pids[0]), "meshmud stopped");
    let nearby = h.frames_since(head, "nearby");
    assert_eq!(nearby.last().unwrap()["sources"], json!([]));
    let v = http(port, "GET", "/api/v1/nearby".into(), None)
        .await
        .json();
    assert_eq!(v["sources"], json!([]));
    assert!(h.store()["devices"][&board.id].get("meshtastic").is_none());
    h.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restart_is_both_children_and_the_identify_child_is_swapped_after_it_exits() {
    let Some(h) = harness(Opts::default()) else {
        return;
    };
    h.start().await;
    let port = h.serve().await;
    let board = h.by_name(HELTEC);
    let r = http(
        port,
        "POST",
        format!("/api/v1/devices/{}/identify", board.id),
        Some(r#"{"consent":true}"#),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.body);
    wait_until("the service reader", Duration::from_secs(10), || {
        h.launched("reader").iter().any(|l| l.ends_with("service"))
    })
    .await;

    // restart = meshmud first, then the reader (Supervised registration order)
    let sup = h.api.ext::<Arc<Supervised>>().unwrap();
    let names: Vec<_> = sup
        .children(ServiceId::Meshtastic)
        .iter()
        .map(|c| c.name().to_string())
        .collect();
    assert_eq!(names, ["meshmud", "meshtastic-presence"]);
    let r = http(
        port,
        "POST",
        "/api/v1/services/meshtastic/restart".into(),
        Some("{}"),
    )
    .await;
    assert_eq!(r.status, 202, "{}", r.body);
    wait_until("both children restarted", Duration::from_secs(10), || {
        h.meshmud_pids().len() == 2
            && h.launched("reader")
                .iter()
                .filter(|l| l.ends_with("service"))
                .count()
                == 2
    })
    .await;
    let pid = *h.meshmud_pids().last().unwrap();
    wait_until("meshmud running again", Duration::from_secs(5), || {
        h.m.service_pids().and_then(|s| s.1) == Some(pid)
    })
    .await;

    // the identify-born child never respawns by itself (identify profile) …
    kill9(pid);
    wait_until(
        "the identify child given up",
        Duration::from_secs(5),
        || h.m.facts().child_state == ChildDisplay::Degraded,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        h.meshmud_pids().len(),
        2,
        "no respawn under the identify profile"
    );
    assert_eq!(h.meshtastic_row(false).state, ServiceState::Error);
    // … the monitor swaps it for a service-policy child; claims never move
    h.m.tick_once().await;
    wait_until("the service meshmud", Duration::from_secs(5), || {
        h.meshmud_pids().len() == 3 && h.m.facts().running
    })
    .await;
    assert!(
        h.held().iter().all(|(_, t)| t == "meshmud"),
        "{:?}",
        h.held()
    );
    let swapped = *h.meshmud_pids().last().unwrap();
    // a service-policy child is respawned by its supervisor
    kill9(swapped);
    wait_until("the supervisor respawn", Duration::from_secs(5), || {
        h.meshmud_pids().len() == 4
    })
    .await;
    let sup = h.api.ext::<Arc<Supervised>>().unwrap();
    assert_eq!(sup.children(ServiceId::Meshtastic).len(), 2);

    // shutdown joins both children and releases the claims
    let (_, mm, rd) = h.m.service_pids().unwrap();
    h.stop().await;
    for p in [mm, rd].into_iter().flatten() {
        assert!(!alive(p), "pid {p} still alive after shutdown");
    }
    assert!(h.held().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn identify_refusals_open_nothing_twice() {
    // the board's port held by another owner → 409 port_busy, nothing opened
    let Some(h) = harness(Opts::default()) else {
        return;
    };
    h.ports.claim(&heltec_uplink(), "rnsd", 1).unwrap();
    h.start().await;
    let port = h.serve().await;
    let board = h.by_name(HELTEC);
    assert_eq!(board.reason_code, "port_busy", "{board:?}");
    assert_eq!(board.detail["port_held_by"], "rnsd", "{board:?}");
    assert!(!board.actions.iter().any(|a| a == "identify"));
    let path = format!("/api/v1/devices/{}/identify", board.id);
    let r = http(port, "POST", path.clone(), Some(r#"{"consent":true}"#)).await;
    r.problem(409, "port_busy");
    assert!(r.json()["detail"].as_str().unwrap().contains("rnsd"));
    assert!(h.launched("").is_empty());

    // the downlink held (e.g. a meshtastic-client) → 409, the board untouched
    h.ports.release(&heltec_uplink(), "rnsd").unwrap();
    h.ports
        .claim(&h.stub.addr.to_string(), "meshtastic-client", 2)
        .unwrap();
    h.m.tick_once().await;
    h.reg.tick(&h.api).await;
    h.reg.tick(&h.api).await; // the registry publishes a change on its 2nd tick
    assert!(h.by_name(HELTEC).actions.iter().any(|a| a == "identify"));
    http(port, "POST", path.clone(), Some(r#"{"consent":true}"#))
        .await
        .problem(409, "port_busy");
    assert!(h.launched("").is_empty());
    assert_eq!(
        h.held(),
        [(h.stub.addr.to_string(), "meshtastic-client".to_string())]
    );
    h.ports
        .release(&h.stub.addr.to_string(), "meshtastic-client")
        .unwrap();

    // meshmud dying during identify: 502, never respawned (identify profile).
    // The reader's answer is held until meshmud has exited, so the board
    // answers after the exit whichever of the two the daemon notices first.
    std::fs::write(&h.crash, "").unwrap();
    std::fs::write(&h.hold, "").unwrap();
    let step = tokio::spawn(http(
        port,
        "POST",
        path.clone(),
        Some(r#"{"consent":true}"#),
    ));
    let pids = h.meshmud_launches(1).await;
    wait_until(
        "the identify meshmud exited",
        Duration::from_secs(5),
        || !alive(pids[0]),
    )
    .await;
    std::fs::remove_file(&h.hold).unwrap();
    step.await.unwrap().problem(502, "upstream_error");
    tokio::time::sleep(Duration::from_millis(2_600)).await; // > the service policy's first back-off
    assert_eq!(h.meshmud_pids().len(), 1, "{:?}", h.launched(""));
    assert!(h.held().is_empty(), "claims released: {:?}", h.held());
    assert!(h.m.service_pids().is_none());
    assert!(h.store()["devices"][&board.id].get("firmware").is_none());
    std::fs::remove_file(&h.crash).unwrap();
    h.stop().await;

    // a board that answers but is not Meshtastic: one NoMatch is not `unknown` (§12, FB1
    // item 4 — `unknown` only after all three families answered NoMatch, which this
    // single-step entry never probes): 502, nothing persisted, unbound
    let Some(h) = harness(Opts {
        mode: Mode::NotMeshtastic,
        ..Opts::default()
    }) else {
        return;
    };
    h.start().await;
    let port = h.serve().await;
    let board = h.by_name(HELTEC);
    let r = http(
        port,
        "POST",
        format!("/api/v1/devices/{}/identify", board.id),
        Some(r#"{"consent":true}"#),
    )
    .await;
    r.problem(502, "upstream_error");
    assert!(r.body.contains("nothing was recorded"), "{}", r.body);
    let pids = h.meshmud_launches(1).await;
    assert_eq!(pids.len(), 1);
    assert!(!alive(pids[0]), "the identify meshmud was stopped");
    assert!(h.held().is_empty());
    assert!(h.m.service_pids().is_none());
    let e = &h.store()["devices"][&board.id];
    assert!(e.get("firmware").is_none(), "nothing persisted: {e}");
    assert!(e.get("meshtastic").is_none());
    assert_eq!(h.meshtastic_row(false).reason_code, "no_meshtastic");
    h.stop().await;
}

/// Start an identify on `id` whose probe stays open (the fake reader's
/// `--once` waits on HOLD); returns once that reader runs.
async fn identify_held_open(h: &H, port: u16, id: &str) -> tokio::task::JoinHandle<Resp> {
    std::fs::write(&h.hold, "").unwrap();
    let step = tokio::spawn(http(
        port,
        "POST",
        format!("/api/v1/devices/{id}/identify"),
        Some(r#"{"consent":true}"#),
    ));
    wait_until("the identify reader", Duration::from_secs(10), || {
        !h.reader_pids("once").is_empty()
    })
    .await;
    step
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forget_during_an_identify_wins_nothing_stays_bound_or_running() {
    let Some(h) = harness(Opts::default()) else {
        return;
    };
    h.start().await;
    let port = h.serve().await;
    let board = h.by_name(HELTEC);
    let step = identify_held_open(&h, port, &board.id).await;

    // the user forgets the board while the probe runs
    let r = http(
        port,
        "DELETE",
        format!("/api/v1/devices/{}", board.id),
        None,
    )
    .await;
    assert_eq!(r.status, 204, "{}", r.body);
    // … then the board answers as Meshtastic
    std::fs::remove_file(&h.hold).unwrap();
    step.await.unwrap().problem(404, "not_found");

    // nothing was written onto the forgotten entry
    let e = &h.store()["devices"][&board.id];
    assert_eq!(e["forgotten"], true, "{e}");
    assert!(e.get("meshtastic").is_none(), "binding persisted: {e}");
    assert!(e.get("firmware").is_none(), "firmware persisted: {e}");
    assert!(h.reg.meshtastic_binding().is_none());
    // the service the identify child was becoming is torn down at once
    assert!(h.m.service_pids().is_none());
    assert!(h.held().is_empty(), "{:?}", h.held());
    let pids = h.meshmud_launches(1).await;
    assert_eq!(pids.len(), 1, "{:?}", h.launched(""));
    wait_until("meshmud stopped", Duration::from_secs(5), || {
        !alive(pids[0])
    })
    .await;
    for pid in h.reader_pids("service") {
        wait_until("the service reader stopped", Duration::from_secs(5), || {
            !alive(pid)
        })
        .await;
    }
    let sup = h.api.ext::<Arc<Supervised>>().unwrap();
    assert!(sup.children(ServiceId::Meshtastic).is_empty());
    let row = h.meshtastic_row(false);
    assert_eq!(
        (row.state, row.reason_code),
        (ServiceState::Unset, "no_meshtastic")
    );

    // later ticks start nothing and the board stays hidden
    h.m.tick_once().await;
    h.reg.tick(&h.api).await;
    h.reg.tick(&h.api).await;
    h.m.tick_once().await;
    assert!(h.m.service_pids().is_none());
    assert_eq!(h.meshmud_pids().len(), 1);
    assert!(h.held().is_empty(), "{:?}", h.held());
    assert!(h.reg.list().unwrap().iter().all(|r| r.id != board.id));
    h.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_does_not_wait_for_an_identify_in_flight() {
    let Some(h) = harness(Opts::default()) else {
        return;
    };
    h.start().await;
    let port = h.serve().await;
    let board = h.by_name(HELTEC);
    let step = identify_held_open(&h, port, &board.id).await;
    let once = h.reader_pids("once");

    // the harness gives the reader 20 s; shutdown must not wait for it
    let t0 = std::time::Instant::now();
    h.stop().await;
    let took = t0.elapsed();
    assert!(
        took < Duration::from_secs(8),
        "shutdown waited {took:?} for the identify step"
    );
    let r = step.await.unwrap();
    r.problem(502, "upstream_error");
    // the step ended fail-closed: children stopped, claims released, nothing kept
    assert!(h.held().is_empty(), "{:?}", h.held());
    assert!(h.m.service_pids().is_none());
    let pids = h.meshmud_launches(1).await;
    assert_eq!(pids.len(), 1);
    for pid in pids.into_iter().chain(once) {
        wait_until("identify children gone", Duration::from_secs(5), || {
            !alive(pid)
        })
        .await;
    }
    let _ = std::fs::remove_file(&h.hold);
    assert!(h.store()["devices"][&board.id].get("firmware").is_none());
    assert!(h.reader_pids("service").is_empty());
}

fn seed_binding(id: &str, uplink: Value, firmware: bool) -> Value {
    let mut e = json!({
        "first_seen": "2026-10-01T00:00:00Z", "last_seen": "2026-10-01T00:00:00Z",
        "meshtastic": {"uplink": uplink, "node_num": MY_NUM, "node_id": "!0a0a0a0a",
                       "hw_model": "HELTEC_V3"}
    });
    if firmware {
        e["firmware"] = json!({"family": "meshtastic", "version": "2.7.11.abcdef0",
                               "source": "probe"});
    }
    json!({"schema": 1, "devices": {id: e}})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn service_states_follow_the_facts() {
    const ID: &str = "abcd1234abcd1234";
    // identified but not bound → NOT SET, "set up with the agent"
    let Some(h) = harness(Opts {
        heltec: false,
        seed: Some(json!({"schema": 1, "devices": {ID: {
            "first_seen": "2026-10-01T00:00:00Z", "last_seen": "2026-10-01T00:00:00Z",
            "firmware": {"family": "meshtastic", "version": "2.7.11.abcdef0", "source": "probe"},
            "last_record_summary": {"name": "Bench board", "kind": "radio", "transport": "usb-serial",
                "match": {"confidence": "likely", "rule": "lora-table 10c4:ea60"},
                "capabilities": ["lora"]}
        }}})),
        ..Opts::default()
    }) else {
        return;
    };
    h.start().await;
    let row = h.meshtastic_row(false);
    assert_eq!(
        (row.state, row.reason_code, row.can_run_here),
        (ServiceState::Unset, "meshtastic_unbound", true),
        "{row:?}"
    );
    assert!(h.launched("").is_empty());
    h.stop().await;

    // bound, but another owner holds the port → ERROR port_busy; freed → runs
    let Some(h) = harness(Opts {
        seed: Some(seed_binding(
            ID,
            json!({"kind": "serial", "path": heltec_uplink()}),
            true,
        )),
        ..Opts::default()
    }) else {
        return;
    };
    h.ports.claim(&heltec_uplink(), "rnsd", 1).unwrap();
    h.start().await;
    let f = h.m.facts();
    assert_eq!(f.port_held_by.as_deref(), Some("rnsd"));
    let row = h.meshtastic_row(false);
    assert_eq!(
        (row.state, row.reason_code),
        (ServiceState::Error, "port_busy")
    );
    assert!(
        h.launched("").is_empty(),
        "meshmud never opened a held port"
    );
    assert!(
        h.m.bound_ports() == vec![heltec_uplink()],
        "still bound for the radio rule"
    );
    h.ports.release(&heltec_uplink(), "rnsd").unwrap();
    h.m.tick_once().await;
    wait_until(
        "meshmud started on the freed port",
        Duration::from_secs(5),
        || h.m.facts().running,
    )
    .await;
    assert_eq!(h.meshmud_launches(1).await.len(), 1);
    // the by-id node exists in the fake tree: the uplink is present
    h.m.tick_once().await;
    assert!(h.m.facts().uplink_lost_secs.is_none());
    wait_until("node DB", Duration::from_secs(10), || {
        h.presence_file().is_file()
    })
    .await;
    h.m.tick_once().await;
    assert_eq!(h.meshtastic_row(false).state, ServiceState::Live);
    h.stop().await;

    // bound to a board that is gone → uplink lost (starting, then ERROR > 60 s)
    let gone = "/dev/serial/by-id/usb-unplugged-board-if00";
    let Some(h) = harness(Opts {
        heltec: false,
        seed: Some(seed_binding(
            ID,
            json!({"kind": "serial", "path": gone}),
            true,
        )),
        ..Opts::default()
    }) else {
        return;
    };
    h.start().await;
    wait_until("meshmud", Duration::from_secs(5), || h.m.facts().running).await;
    h.m.tick_once().await;
    let f = h.m.facts();
    assert!(f.uplink_lost_secs.is_some(), "{f:?}");
    let row = h.meshtastic_row(false);
    assert_eq!(
        (row.state, row.reason_code),
        (ServiceState::Starting, "uplink_waiting")
    );
    let late = Facts {
        meshmud: Some(services::MeshmudFacts {
            uplink_lost_secs: Some(61),
            ..f
        }),
        ..Facts::default()
    };
    let row = service(&late, ServiceId::Meshtastic);
    assert_eq!(
        (row.state, row.reason_code),
        (ServiceState::Error, "uplink_lost")
    );
    h.stop().await;

    // a crash-looping meshmud → ERROR (degraded), restart offered
    let Some(h) = harness(Opts {
        seed: Some(seed_binding(
            ID,
            json!({"kind": "serial", "path": heltec_uplink()}),
            true,
        )),
        ..Opts::default()
    }) else {
        return;
    };
    std::fs::write(&h.crash, "").unwrap();
    h.start().await;
    wait_until("crash loop detected", Duration::from_secs(10), || {
        h.m.facts().child_state == ChildDisplay::Degraded
    })
    .await;
    let row = h.meshtastic_row(false);
    assert_eq!(
        (row.state, row.reason_code),
        (ServiceState::Error, "child_degraded")
    );
    assert!(row
        .actions
        .iter()
        .any(|a| format!("{a:?}").to_lowercase().contains("restart")));
    h.stop().await;

    // a binding without a meshmud binary: honestly absent, never started
    let Some(h) = harness(Opts {
        seed: Some(seed_binding(
            ID,
            json!({"kind": "serial", "path": heltec_uplink()}),
            true,
        )),
        ..Opts::default()
    }) else {
        return;
    };
    let bin = h.m.config().meshmud_bin.clone();
    let fake = std::fs::read(&bin).unwrap();
    std::fs::remove_file(&bin).unwrap();
    h.start().await;
    let f = h.m.facts();
    assert_eq!(f.child_state, ChildDisplay::Absent);
    assert!(!f.running);
    assert!(f.bound && !f.installed, "{f:?}");
    assert!(h.held().is_empty(), "nothing claimed for an absent meshmud");
    // the row names the real cause (not "set up with the agent")
    let row = h.meshtastic_row(false);
    assert_eq!(
        (row.state, row.reason_code),
        (ServiceState::Error, "binary_absent"),
        "{row:?}"
    );
    assert!(
        row.reason.contains("meshmud is not installed"),
        "{}",
        row.reason
    );
    assert!(h.launched("").is_empty());
    // an upgrade installs the binary: the next monitor tick starts it
    std::fs::write(&bin, &fake).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    h.m.tick_once().await;
    wait_until(
        "meshmud started once its binary appeared",
        Duration::from_secs(5),
        || h.m.facts().running,
    )
    .await;
    assert_eq!(h.meshmud_launches(1).await.len(), 1);
    assert!(
        h.held()
            .iter()
            .any(|(k, t)| *k == heltec_uplink() && t == "meshmud"),
        "{:?}",
        h.held()
    );
    h.m.tick_once().await;
    assert!(h.m.facts().installed);
    assert_eq!(h.meshmud_pids().len(), 1, "started once, not every tick");
    h.stop().await;
}

/// The presence reader's venv could not be built (§14: its build records why
/// in `venvs/presence.status.json`): meshmud runs and its downlink listens,
/// but no node DB can come, so the row is ERROR `presence_unavailable` with
/// the recorded reason — never an endless `node_db_pending`. With the status
/// reading ready and the reader installed the row is what it always was.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unbuilt_presence_venv_is_the_meshtastic_rows_reason() {
    const ID: &str = "abcd1234abcd1234";
    let seed = || {
        Some(seed_binding(
            ID,
            json!({"kind": "serial", "path": heltec_uplink()}),
            true,
        ))
    };
    let Some(h) = harness(Opts {
        seed: seed(),
        ..Opts::default()
    }) else {
        return;
    };
    let root = h.dir.path();
    // no interpreter in the venv, and a rebuild that failed (hash mismatch)
    std::fs::remove_file(&h.m.config().reader_python).unwrap();
    let lock = root.join("presence-lock.txt");
    std::fs::write(&lock, b"meshtastic==2.7.11 --hash=sha256:abcd\n").unwrap();
    let py = fake_python::FakePython::new();
    py.fail_install(true);
    let built = venv_presence::ensure(
        root,
        &py,
        &lock,
        &venv_presence::cache_dir(root),
        venv_presence::Mode::Offline,
    );
    assert!(
        matches!(
            built,
            venv_presence::Outcome::Unavailable {
                previous_kept: false,
                ..
            }
        ),
        "{built:?}"
    );
    let why = venv_presence::unavailable_reason(root).unwrap();
    assert!(why.starts_with("The presence reader is off"), "{why}");

    h.start().await;
    wait_until("meshmud", Duration::from_secs(5), || h.m.facts().running).await;
    assert_eq!(h.meshmud_launches(1).await.len(), 1);
    h.m.tick_once().await;
    let f = h.m.facts();
    assert!(f.downlink_listening && !f.node_db_present, "{f:?}");
    assert_eq!(f.reader_unavailable.as_deref(), Some(why.as_str()));
    assert!(
        h.reader_pids("service").is_empty(),
        "no reader without its venv"
    );
    let row = h.meshtastic_row(false);
    assert_eq!(
        (row.state, row.reason_code),
        (ServiceState::Error, "presence_unavailable"),
        "{row:?}"
    );
    assert_eq!(row.reason, why);
    assert_eq!(row.detail["reader_unavailable"], why.as_str());
    // nothing recorded, the reader simply not there: a plain reason
    std::fs::remove_file(venv_presence::status_path(root)).unwrap();
    h.m.tick_once().await;
    let row = h.meshtastic_row(false);
    assert_eq!(
        (row.state, row.reason_code, row.reason.as_str()),
        (
            ServiceState::Error,
            "presence_unavailable",
            "The presence reader is not installed on this node."
        )
    );
    h.stop().await;

    // the status reads ready and the reader is installed: unchanged
    let Some(h) = harness(Opts {
        seed: seed(),
        ..Opts::default()
    }) else {
        return;
    };
    let ready = venv_presence::Status {
        schema: 1,
        ok: true,
        code: "presence_venv_ready".into(),
        reason: None,
        lock_sha256: "ab".repeat(32),
        previous_kept: false,
    };
    std::fs::write(
        venv_presence::status_path(h.dir.path()),
        serde_json::to_vec(&ready).unwrap(),
    )
    .unwrap();
    assert_eq!(venv_presence::status(h.dir.path()), Some(ready));
    h.start().await;
    wait_until("meshmud", Duration::from_secs(5), || h.m.facts().running).await;
    h.m.tick_once().await;
    assert_eq!(h.m.facts().reader_unavailable, None);
    assert_ne!(h.meshtastic_row(false).reason_code, "presence_unavailable");
    wait_until("node DB", Duration::from_secs(10), || {
        h.presence_file().is_file()
    })
    .await;
    h.m.tick_once().await;
    let row = h.meshtastic_row(false);
    assert_eq!(
        (row.state, row.reason_code),
        (ServiceState::Live, "meshtastic_live"),
        "{row:?}"
    );
    assert_eq!(row.detail["reader_unavailable"], Value::Null);
    h.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn phones_never_drive_usb_boards_but_run_wifi_boards() {
    const ID: &str = "abcd1234abcd1234";
    // a USB board on a phone: identify is not offered and refused
    let Some(h) = harness(Opts {
        phone: true,
        ..Opts::default()
    }) else {
        return;
    };
    h.start().await;
    let port = h.serve().await;
    let board = h.by_name(HELTEC);
    assert!(!board.actions.iter().any(|a| a == "identify"), "{board:?}");
    http(
        port,
        "POST",
        format!("/api/v1/devices/{}/identify", board.id),
        Some(r#"{"consent":true}"#),
    )
    .await
    .problem(400, "bad_request");
    assert!(h.launched("").is_empty());
    h.stop().await;

    // a USB binding on a phone: never started; NOT SET with can_run_here=false
    let Some(h) = harness(Opts {
        phone: true,
        heltec: false,
        seed: Some(seed_binding(
            ID,
            json!({"kind": "serial", "path": heltec_uplink()}),
            true,
        )),
        ..Opts::default()
    }) else {
        return;
    };
    h.start().await;
    let f = h.m.facts();
    assert!(
        f.usb_uplink && f.child_state == ChildDisplay::Absent,
        "{f:?}"
    );
    let row = h.meshtastic_row(true);
    assert_eq!(row.state, ServiceState::Unset);
    assert!(!row.can_run_here, "{row:?}");
    assert!(
        h.m.bound_ports().is_empty(),
        "a binding that cannot run binds nothing"
    );
    assert!(h.launched("").is_empty());
    assert!(h.held().is_empty());
    h.stop().await;

    // a Wi-Fi board bound by address runs on the phone
    let Some(h) = harness(Opts {
        phone: true,
        heltec: false,
        seed: Some(seed_binding(
            ID,
            json!({"kind": "tcp", "host": "10.0.0.7", "port": 4403}),
            true,
        )),
        ..Opts::default()
    }) else {
        return;
    };
    h.start().await;
    wait_until("meshmud on the phone", Duration::from_secs(5), || {
        h.m.facts().running
    })
    .await;
    let text = std::fs::read_to_string(h.dir.path().join("meshtastic/meshmud.toml")).unwrap();
    let v: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(
        v["uplink"]["tcp"][0]["address"].as_str(),
        Some("10.0.0.7:4403")
    );
    assert!(v["uplink"].get("serial").is_none());
    assert_eq!(v["downlink"]["tcp"][0]["advertise"].as_bool(), Some(false));
    assert!(h
        .held()
        .iter()
        .any(|(k, t)| k == "10.0.0.7:4403" && t == "meshmud"));
    assert_eq!(h.m.bound_ports(), vec!["tcp://10.0.0.7:4403".to_string()]);
    let f = h.m.facts();
    assert!(!f.usb_uplink);
    assert!(h.meshtastic_row(true).can_run_here);
    h.stop().await;
}

// ---------------------------------------------------------------- shim

fn sess(key: &str) -> ShimSession {
    ShimSession {
        panel: "meshtastic".into(),
        session: key.into(),
        peer: "127.0.0.1".parse().unwrap(),
    }
}

fn req(method: axum::http::Method, path: &str, body: &[u8]) -> ShimRequest {
    ShimRequest {
        method,
        path: path.into(),
        query: Some("all=true".into()),
        content_type: Some(shim::PROTOBUF.into()),
        body: axum::body::Bytes::copy_from_slice(body),
    }
}

async fn put(s: &MeshtasticShim, key: &str, body: &[u8]) -> Result<u16, ShimError> {
    s.handle(
        &sess(key),
        req(axum::http::Method::PUT, "/api/v1/toradio", body),
    )
    .await
    .map(|r| {
        assert!(r.body.is_empty());
        r.status.as_u16()
    })
}

async fn get(s: &MeshtasticShim, key: &str) -> Result<Vec<u8>, ShimError> {
    s.handle(
        &sess(key),
        req(axum::http::Method::GET, "/api/v1/fromradio", b""),
    )
    .await
    .map(|r| {
        assert_eq!(r.status.as_u16(), 200);
        assert_eq!(r.content_type.as_deref(), Some(shim::PROTOBUF));
        r.body.to_vec()
    })
}

/// Poll `GET /fromradio` until a frame arrives (empty answers are "none yet").
async fn next_frame(s: &MeshtasticShim, key: &str) -> Vec<u8> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let f = get(s, key).await.unwrap();
        if !f.is_empty() {
            return f;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no FromRadio for {key}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shim_round_trip_buffer_and_session_lifecycle() {
    use axum::http::Method;
    let stub = Stub::start(Mode::Meshtastic);
    let s = MeshtasticShim::new(ShimConfig {
        downlink: stub.addr,
        capacity: 4,
        idle: Duration::from_millis(600),
        max_sessions: 2,
        connect_timeout: Duration::from_secs(2),
        write_timeout: Duration::from_secs(2),
    });
    assert_eq!(s.name(), "meshtastic-http");

    // refusals before any connection is opened
    let other = ShimSession {
        panel: "fabric".into(),
        ..sess("a")
    };
    assert_eq!(
        s.handle(&other, req(Method::PUT, "/api/v1/toradio", b"x"))
            .await,
        Err(ShimError::NotFound)
    );
    assert_eq!(
        s.handle(&sess("a"), req(Method::GET, "/api/v1/other", b""))
            .await,
        Err(ShimError::NotFound)
    );
    assert_eq!(
        s.handle(&sess("a"), req(Method::POST, "/api/v1/toradio", b"x"))
            .await,
        Err(ShimError::MethodNotAllowed)
    );
    assert_eq!(
        s.handle(&sess("a"), req(Method::PUT, "/api/v1/fromradio", b""))
            .await,
        Err(ShimError::MethodNotAllowed)
    );
    assert!(matches!(
        put(&s, "a", b"").await,
        Err(ShimError::BadRequest(_))
    ));
    assert_eq!(
        put(&s, "a", &[1u8; shim::MAX_FRAME + 1]).await,
        Err(ShimError::TooLarge {
            limit: shim::MAX_FRAME
        })
    );
    assert_eq!(
        stub.accepted(),
        0,
        "a session's downlink opens on its first call"
    );

    // PUT → exactly one framed ToRadio; GET with nothing queued → empty 200
    assert_eq!(put(&s, "a", b"\x08\x01hello").await, Ok(200));
    assert_eq!(put(&s, "a", &[7u8; shim::MAX_FRAME]).await, Ok(200));
    wait_until("ToRadio at the stub", Duration::from_secs(5), || {
        stub.received().len() == 2
    })
    .await;
    assert_eq!(stub.received()[0], b"\x08\x01hello");
    assert_eq!(stub.received()[1], vec![7u8; shim::MAX_FRAME]);
    assert_eq!(stub.framing_errors(), 0);
    assert_eq!(stub.accepted(), 1);
    assert_eq!(s.sessions(), 1);
    assert_eq!(get(&s, "a").await, Ok(Vec::new()));

    // FromRadio: debug noise skipped, a frame split across writes, then a
    // burst over capacity → drop-oldest, counted
    let first = framed(b"f0");
    let mut head = b"INFO | 12:00:01 boot\r\n".to_vec();
    head.extend_from_slice(&first[..3]);
    stub.send_all(&head);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut burst = first[3..].to_vec();
    for i in 1..10u8 {
        burst.extend(framed(&[b'f', b'0' + i]));
    }
    stub.send_all(&burst);
    wait_until("6 frames dropped", Duration::from_secs(5), || {
        s.dropped() == 6
    })
    .await;
    for i in 6..10u8 {
        assert_eq!(
            get(&s, "a").await,
            Ok(vec![b'f', b'0' + i]),
            "one frame per call"
        );
    }
    assert_eq!(get(&s, "a").await, Ok(Vec::new()));
    assert_eq!(s.dropped(), 6);

    // the drop count reaches panel.state through the panels provider
    let panels: node111::daemon::services::probe::Provider<Vec<PanelInfo>> = Arc::new(|| {
        Some(vec![PanelInfo {
            id: ServiceId::Meshtastic,
            port: 11190,
            entry: "/".into(),
            needs_secure_context: false,
            allow: vec![],
            optional_hosts: vec![],
            state: PanelState {
                enabled: true,
                listening: true,
                health: PanelHealth::Up,
                dropped: None,
            },
            title: None,
            secure_note: None,
        }])
    });
    let wrapped = mt::with_dropped(Some(panels), s.clone()).unwrap();
    assert_eq!(wrapped().unwrap()[0].state.dropped, Some(6));

    // a second viewer owns its own downlink and its own queue
    assert_eq!(put(&s, "b", b"b1").await, Ok(200));
    wait_until("second connection", Duration::from_secs(5), || {
        stub.accepted() == 2 && stub.received().len() == 3
    })
    .await;
    stub.send_all(&framed(b"both"));
    assert_eq!(next_frame(&s, "a").await, b"both");
    assert_eq!(next_frame(&s, "b").await, b"both");
    // the session cap
    assert!(matches!(
        put(&s, "c", b"c1").await,
        Err(ShimError::Upstream(_))
    ));
    assert_eq!(stub.accepted(), 2);

    // revocation closes that session's connection at once; a later call of
    // the same key opens a fresh one
    assert!(mt::on_revocation(
        &s,
        Ok(Revoked {
            panel: "fabric".into(),
            session: "a".into()
        })
    ));
    assert_eq!(s.sessions(), 2, "another panel's revocation is not ours");
    assert!(mt::on_revocation(
        &s,
        Ok(Revoked {
            panel: "meshtastic".into(),
            session: "a".into()
        })
    ));
    wait_until("connection a closed", Duration::from_secs(5), || {
        stub.closed() == 1
    })
    .await;
    assert_eq!(s.sessions(), 1);
    assert_eq!(get(&s, "a").await, Ok(Vec::new()));
    // the connect returns on the kernel handshake; the stub counts it when
    // its accept thread gets there
    wait_until("fresh connection a", Duration::from_secs(5), || {
        stub.accepted() == 3
    })
    .await;
    // falling behind the revocation stream closes everything (fail closed)
    assert!(mt::on_revocation(
        &s,
        Err(tokio::sync::broadcast::error::RecvError::Lagged(3))
    ));
    assert_eq!(s.sessions(), 0);
    assert!(!mt::on_revocation(
        &s,
        Err(tokio::sync::broadcast::error::RecvError::Closed)
    ));
    wait_until("all closed", Duration::from_secs(5), || stub.closed() == 3).await;

    // idle sessions lose their connection
    assert_eq!(put(&s, "d", b"d1").await, Ok(200));
    assert_eq!(s.sessions(), 1);
    wait_until("idle close", Duration::from_secs(5), || {
        s.sessions() == 0 && stub.closed() == 4
    })
    .await;

    // meshmud's downlink gone → 502, no session kept
    let down = MeshtasticShim::new(ShimConfig {
        downlink: {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        },
        ..s.config().clone()
    });
    let e = get(&down, "a").await.unwrap_err();
    assert!(matches!(e, ShimError::Upstream(_)));
    assert_eq!(e.problem().code, "upstream_error");
    assert_eq!(down.sessions(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shim_skips_empty_frames_and_caps_sessions_strictly() {
    let stub = Stub::start(Mode::Meshtastic);
    let s = MeshtasticShim::new(ShimConfig {
        downlink: stub.addr,
        capacity: 4,
        idle: Duration::from_secs(60),
        max_sessions: 2,
        connect_timeout: Duration::from_secs(2),
        write_timeout: Duration::from_secs(2),
    });

    // a zero-length FromRadio is never queued: it would read as "queue
    // empty" and end the client's poll loop with frames still behind it
    assert_eq!(put(&s, "a", b"a1").await, Ok(200));
    wait_until("connection a", Duration::from_secs(5), || {
        stub.received().len() == 1
    })
    .await;
    let mut burst = Vec::new();
    for _ in 0..8 {
        burst.extend(framed(b""));
    }
    burst.extend(framed(b"x"));
    stub.send_all(&burst);
    assert_eq!(next_frame(&s, "a").await, b"x");
    assert_eq!(s.dropped(), 0, "empty frames neither queued nor dropped");
    assert_eq!(get(&s, "a").await, Ok(Vec::new()));

    // the session cap holds under concurrent first calls on distinct keys
    s.close_all();
    wait_until("connection a closed", Duration::from_secs(5), || {
        stub.closed() == 1
    })
    .await;
    let calls: Vec<_> = (0..8)
        .map(|i| {
            let s = s.clone();
            tokio::spawn(async move { put(&s, &format!("k{i}"), b"k").await })
        })
        .collect();
    let mut ok = 0;
    for c in calls {
        match c.await.unwrap() {
            Ok(200) => ok += 1,
            Ok(other) => panic!("status {other}"),
            Err(e) => assert!(matches!(e, ShimError::Upstream(_)), "{e:?}"),
        }
    }
    assert_eq!(ok, 2, "exactly max_sessions first calls succeed");
    assert_eq!(s.sessions(), 2);
    wait_until("two new connections", Duration::from_secs(5), || {
        stub.accepted() >= 3
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(stub.accepted(), 3, "a refused call opens no downlink");
    s.close_all();
}

// ---------------------------------------------------------------- presence

#[test]
fn presence_ingest_is_hardened() {
    let dir = tempfile::tempdir().unwrap();
    let path = presence::presence_dir(dir.path()).join(presence::FILE);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let p = Presence::new(path.clone());
    let doc = |nodes: Value| json!({"schema": 1, "nodes": nodes}).to_string();

    std::fs::write(&path, doc(node_list()["nodes"].clone())).unwrap();
    assert_eq!(p.refresh(), Refresh::Changed);
    assert_eq!(p.count(), 2);
    let names: Vec<_> = p
        .nodes()
        .unwrap()
        .into_iter()
        .map(|n| n.long_name)
        .collect();
    assert_eq!(names[0].len(), 40, "cut to 40 bytes");
    assert!(!names[0].contains('\u{1b}'));

    // invalid schema → keep the last snapshot, count it
    std::fs::write(&path, json!({"schema": 2, "nodes": []}).to_string()).unwrap();
    assert!(matches!(
        p.refresh(),
        Refresh::Rejected(IngestError::Invalid(_))
    ));
    assert_eq!((p.count(), p.rejected()), (2, 1));
    // a wrongly typed field
    std::fs::write(
        &path,
        doc(json!([{"id": "!00000003", "num": 3, "long_name": 5, "short_name": ""}])),
    )
    .unwrap();
    assert!(matches!(
        p.refresh(),
        Refresh::Rejected(IngestError::Parse(_))
    ));
    assert_eq!((p.count(), p.rejected()), (2, 2));
    // too many nodes
    let many: Vec<_> = (1..=(presence::MAX_NODES as u32 + 1))
        .map(|n| json!({"id": format!("!{n:08x}"), "num": n, "long_name": "", "short_name": ""}))
        .collect();
    std::fs::write(&path, doc(json!(many))).unwrap();
    assert!(matches!(
        p.refresh(),
        Refresh::Rejected(IngestError::Invalid(_))
    ));
    // oversize: refused before parsing
    std::fs::write(&path, vec![b' '; presence::MAX_FILE_BYTES as usize + 1]).unwrap();
    assert!(matches!(
        p.refresh(),
        Refresh::Rejected(IngestError::TooLarge(_))
    ));
    // a symlink (to a valid file) is refused: O_NOFOLLOW
    let elsewhere = dir.path().join("elsewhere.json");
    std::fs::write(&elsewhere, doc(json!([]))).unwrap();
    std::fs::remove_file(&path).unwrap();
    symlink(&elsewhere, &path).unwrap();
    assert_eq!(p.refresh(), Refresh::Rejected(IngestError::Symlink));
    // a FIFO never blocks the daemon and is refused
    std::fs::remove_file(&path).unwrap();
    assert!(std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .unwrap()
        .success());
    assert_eq!(p.refresh(), Refresh::Rejected(IngestError::NotRegular));
    assert_eq!(
        p.count(),
        2,
        "the last good snapshot survives every refusal"
    );
    assert!(p.rejected() >= 6);
    // valid again → accepted
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, doc(json!([]))).unwrap();
    assert_eq!(p.refresh(), Refresh::Changed);
    assert_eq!((p.present(), p.count()), (true, 0));
}
