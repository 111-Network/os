//! Layer 4 R4 acceptance (os/interface/README.md §6.3, §7.1): the device
//! registry over a fake `/sys` + `/dev` tree in a temp dir — detection, LoRa
//! table match, stable ids, companions from dashboard sessions, alias PATCH,
//! forget (+ session revoke, incl. your own session), reappear, disconnected
//! memory, 2-tick debounce, `match` likely/unknown + `firmware: unknown`,
//! `driver` from an `RNodeInterface` stanza in a fake `<root>/rns/config` (and
//! rnsd down → `error`), meshtasticd driving SPI bridges only, `error` on an
//! unopenable tty, `detail` bound, sysfs string sanitization.
//!
//! Fixture note: the devices follow the SHIPPED `assets/lora-table.toml`.
//! Heltec V3 is its CP2102 row (`10c4:ea60` + product "CP2102"); `303a:*` is
//! "ESP32-S3 native USB"; `1a86:*` (CH340/CH9102) matches the T-Beam row by
//! design (the table says so: "Layer 6 refines"), so the "unidentified serial
//! device" here is an FTDI FT232R (`0403:6001`), which no row claims.

use axum::extract::connect_info::MockConnectInfo;
use node111::daemon::dashboard::api_v1::auth;
use node111::daemon::dashboard::{
    routes, serve_listener, ApiConfig, ApiState, DashboardState, SharedState,
};
use node111::daemon::devices::{
    self, Confidence, DeviceDriver, DeviceFirmware, DeviceKind, DeviceRecord, DeviceRegistry,
    DeviceState, FirmwareFamily, FirmwareSource, RegistryConfig, Roots, Stack, Transport,
};
use node111::daemon::DaemonLayout;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

// ---------------------------------------------------------------- fake tree

const HUB: &str = "sys/devices/pci0000:00/0000:00:14.0/usb1";

const HELTEC_BY_ID: &str = "usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0";
const ESP_BY_ID: &str = "usb-Espressif_USB_JTAG_serial_debug_unit_F4:12:FA:00:11:22-if00";
const CH340_BY_ID: &str = "usb-1a86_USB_Serial-if00-port0";
const FTDI_BY_ID: &str = "usb-FTDI_FT232R_USB_UART_A50285BI-if00-port0";

struct Tree {
    root: PathBuf,
}

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

struct Usb<'a> {
    bus_id: &'a str,
    vid: &'a str,
    pid: &'a str,
    class: &'a str,
    product: Option<&'a str>,
    manufacturer: Option<&'a str>,
    serial: Option<&'a str>,
    interfaces: &'a [&'a str],
}

impl Tree {
    fn new(dir: &Path) -> Tree {
        let t = Tree {
            root: dir.join("fake"),
        };
        // the root hub (always skipped: class 09)
        t.usb(&Usb {
            bus_id: "usb1",
            vid: "1d6b",
            pid: "0002",
            class: "09",
            product: Some("xHCI Host Controller"),
            manufacturer: None,
            serial: None,
            interfaces: &["09"],
        });
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

    fn usb(&self, u: &Usb) {
        let d = self.dev_dir(u.bus_id);
        w(&d.join("idVendor"), &format!("{}\n", u.vid));
        w(&d.join("idProduct"), &format!("{}\n", u.pid));
        w(&d.join("bDeviceClass"), &format!("{}\n", u.class));
        if let Some(p) = u.product {
            w(&d.join("product"), &format!("{p}\n"));
        }
        if let Some(m) = u.manufacturer {
            w(&d.join("manufacturer"), &format!("{m}\n"));
        }
        if let Some(s) = u.serial {
            w(&d.join("serial"), &format!("{s}\n"));
        }
        for (i, class) in u.interfaces.iter().enumerate() {
            let intf = d.join(format!("{}:1.{i}", u.bus_id));
            w(&intf.join("bInterfaceClass"), &format!("{class}\n"));
            // interfaces are listed in /sys/bus/usb/devices too (must be skipped)
            rel_link(
                &intf,
                &self
                    .root
                    .join("sys/bus/usb/devices")
                    .join(format!("{}:1.{i}", u.bus_id)),
            );
        }
        rel_link(&d, &self.root.join("sys/bus/usb/devices").join(u.bus_id));
    }

    /// cdc_acm tty: `<intf>/tty/<tty>`, class link → the interface.
    fn acm(&self, bus_id: &str, tty: &str, by_id: Option<&str>) {
        let intf = self.dev_dir(bus_id).join(format!("{bus_id}:1.0"));
        std::fs::create_dir_all(intf.join("tty").join(tty)).unwrap();
        rel_link(
            &intf,
            &self.root.join("sys/class/tty").join(tty).join("device"),
        );
        self.dev_node(tty, by_id);
    }

    /// usb-serial tty (cp210x/ch341/ftdi_sio): `<intf>/<tty>/`, class link → the port dir.
    fn usb_serial(&self, bus_id: &str, tty: &str, by_id: Option<&str>) {
        let port = self.dev_dir(bus_id).join(format!("{bus_id}:1.0")).join(tty);
        std::fs::create_dir_all(&port).unwrap();
        rel_link(
            &port,
            &self.root.join("sys/class/tty").join(tty).join("device"),
        );
        self.dev_node(tty, by_id);
    }

    fn dev_node(&self, tty: &str, by_id: Option<&str>) {
        let node = self.root.join("dev").join(tty);
        w(&node, "");
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o660)).unwrap();
        if let Some(b) = by_id {
            rel_link(&node, &self.root.join("dev/serial/by-id").join(b));
        }
    }

    fn hci(&self, name: &str, address: &str, bus_id: Option<&str>) {
        let d = self.root.join("sys/class/bluetooth").join(name);
        w(&d.join("address"), &format!("{address}\n"));
        if let Some(b) = bus_id {
            rel_link(&self.dev_dir(b).join(format!("{b}:1.0")), &d.join("device"));
        }
        // a connection entry next to the adapter (must be skipped)
        std::fs::create_dir_all(
            self.root
                .join("sys/class/bluetooth")
                .join(format!("{name}:256")),
        )
        .unwrap();
    }

    fn unplug(&self, bus_id: &str, tty: Option<&str>, by_id: Option<&str>) {
        let _ = std::fs::remove_dir_all(self.dev_dir(bus_id));
        let usb = self.root.join("sys/bus/usb/devices");
        for e in std::fs::read_dir(&usb).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n == bus_id || n.starts_with(&format!("{bus_id}:")) {
                std::fs::remove_file(e.path()).unwrap();
            }
        }
        if let Some(t) = tty {
            let _ = std::fs::remove_dir_all(self.root.join("sys/class/tty").join(t));
            let _ = std::fs::remove_file(self.root.join("dev").join(t));
        }
        if let Some(b) = by_id {
            let _ = std::fs::remove_file(self.root.join("dev/serial/by-id").join(b));
        }
    }

    // ---- the fixture devices

    fn heltec(&self) {
        self.usb(&Usb {
            bus_id: "1-1",
            vid: "10c4",
            pid: "ea60",
            class: "00",
            product: Some("CP2102 USB to UART Bridge Controller"),
            manufacturer: Some("Silicon Labs"),
            serial: Some("0001"),
            interfaces: &["ff"],
        });
        self.usb_serial("1-1", "ttyUSB0", Some(HELTEC_BY_ID));
    }

    fn esp32s3(&self) {
        self.usb(&Usb {
            bus_id: "1-2",
            vid: "303a",
            pid: "1001",
            class: "ef",
            product: Some("USB JTAG/serial debug unit"),
            manufacturer: Some("Espressif"),
            serial: Some("F4:12:FA:00:11:22"),
            interfaces: &["02", "0a", "ff"],
        });
        self.acm("1-2", "ttyACM0", Some(ESP_BY_ID));
    }

    fn ch340(&self) {
        self.usb(&Usb {
            bus_id: "1-3",
            vid: "1a86",
            pid: "7523",
            class: "ff",
            product: Some("USB Serial"),
            manufacturer: None,
            serial: None,
            interfaces: &["ff"],
        });
        self.usb_serial("1-3", "ttyUSB1", Some(CH340_BY_ID));
    }

    fn ftdi(&self) {
        self.usb(&Usb {
            bus_id: "1-4",
            vid: "0403",
            pid: "6001",
            class: "00",
            product: Some("FT232R USB UART"),
            manufacturer: Some("FTDI"),
            serial: Some("A50285BI"),
            interfaces: &["ff"],
        });
        self.usb_serial("1-4", "ttyUSB2", Some(FTDI_BY_ID));
    }

    /// CH341 in USB-to-SPI mode (meshtasticd's Linux-native radio path): no tty.
    fn ch341_spi(&self) {
        self.usb(&Usb {
            bus_id: "1-10",
            vid: "1a86",
            pid: "5512",
            class: "ff",
            product: Some("USB UART-LPT"),
            manufacturer: None,
            serial: None,
            interfaces: &["ff"],
        });
    }

    fn unplug_ftdi(&self) {
        self.unplug("1-4", Some("ttyUSB2"), Some(FTDI_BY_ID));
    }

    fn full(&self) {
        self.heltec();
        self.esp32s3();
        self.ch340();
        self.ftdi();
        // external hub (skipped)
        self.usb(&Usb {
            bus_id: "1-5",
            vid: "05e3",
            pid: "0608",
            class: "09",
            product: Some("USB2.0 Hub"),
            manufacturer: None,
            serial: None,
            interfaces: &["09"],
        });
        // keyboard: HID only (skipped)
        self.usb(&Usb {
            bus_id: "1-6",
            vid: "046d",
            pid: "c31c",
            class: "00",
            product: Some("USB Keyboard"),
            manufacturer: Some("Logitech"),
            serial: None,
            interfaces: &["03", "03"],
        });
        // nRF52 board in DFU: no tty, no serial → radio via sysfs path
        self.usb(&Usb {
            bus_id: "1-7",
            vid: "239a",
            pid: "0029",
            class: "00",
            product: Some("T-Echo DFU"),
            manufacturer: None,
            serial: None,
            interfaces: &["fe"],
        });
        // hostile strings: control chars + 300-char product
        let product = format!("Mystery\u{7} gadget\u{1b}[2J {}", "x".repeat(300));
        self.usb(&Usb {
            bus_id: "1-8",
            vid: "1234",
            pid: "5678",
            class: "ff",
            product: Some(&product),
            manufacturer: Some("\u{202e}Evil Corp"),
            serial: Some("\u{1b}[31mSN-1"),
            interfaces: &["ff"],
        });
        // BT dongle (class e0) behind hci0: listed once, as the adapter
        self.usb(&Usb {
            bus_id: "1-9",
            vid: "0bda",
            pid: "8771",
            class: "e0",
            product: Some("Bluetooth Radio"),
            manufacturer: Some("Realtek"),
            serial: None,
            interfaces: &["e0", "e0"],
        });
        self.hci("hci0", "aa:bb:cc:dd:ee:ff", Some("1-9"));
    }
}

// ---------------------------------------------------------------- node harness

const UA_ANDROID: &str =
    "Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Mobile Safari/537.36";
const TOKEN_FRESH: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const TOKEN_IDLE: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn sha_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn ago(secs: i64) -> String {
    (OffsetDateTime::now_utc() - time::Duration::seconds(secs))
        .format(&Rfc3339)
        .unwrap()
}

struct Node {
    api: Arc<ApiState>,
    reg: Arc<DeviceRegistry>,
    tree: Tree,
    cfg: RegistryConfig,
    dir: tempfile::TempDir,
}

/// A port with nothing listening (connect → refused).
fn closed_port() -> SocketAddr {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap()
}

fn write_sessions(state: &Path) {
    let file = serde_json::json!({
        "schema": 1,
        "sessions": [
            {"hash": sha_hex(TOKEN_FRESH), "created": ago(3600), "last_seen": ago(60),
             "peer": "192.168.1.20", "user_agent": UA_ANDROID},
            // paired before user agents were recorded; idle for 30 min
            {"hash": sha_hex(TOKEN_IDLE), "created": ago(2 * 86400), "last_seen": ago(1800),
             "peer": "192.168.1.21"}
        ]
    });
    w(&state.join(auth::SESSIONS_FILE), &file.to_string());
}

fn node_with(setup: impl FnOnce(&Tree), sessions: bool) -> Node {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    if sessions {
        write_sessions(&state);
    }
    let tree = Tree::new(dir.path());
    setup(&tree);
    let kb = dir.path().join("current/share/kb");
    w(
        &kb.join("devices/heltec-wifi-lora-32-v3.md"),
        "---\ntitle: Heltec WiFi LoRa 32 V3\n---\n",
    );
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.10.13")));
    let api = ApiState::new(dash, ApiConfig::new(layout.clone(), false));
    let mut cfg = RegistryConfig::production(&layout);
    cfg.roots = tree.roots();
    cfg.probe_addr = closed_port();
    cfg.probe_timeout = Duration::from_millis(300);
    assert_eq!(
        cfg.kb_dir, kb,
        "production KB path is <root>/current/share/kb"
    );
    let reg = devices::install(&api, cfg.clone());
    Node {
        api,
        reg,
        tree,
        cfg,
        dir,
    }
}

fn node() -> Node {
    node_with(|t| t.full(), true)
}

impl Node {
    async fn tick(&self) -> bool {
        self.reg.tick(&self.api).await
    }

    fn list(&self) -> Vec<DeviceRecord> {
        self.reg.list().expect("published")
    }

    fn by_name(&self, name: &str) -> DeviceRecord {
        self.list()
            .into_iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("no device named {name:?}: {:#?}", self.names()))
    }

    fn names(&self) -> Vec<String> {
        self.list().into_iter().map(|r| r.name).collect()
    }

    fn store(&self) -> serde_json::Value {
        let raw = std::fs::read_to_string(self.dir.path().join("state/devices.json")).unwrap();
        serde_json::from_str(&raw).unwrap()
    }

    /// `devices` frames published after `head`.
    fn frames_since(&self, head: u64) -> Vec<serde_json::Value> {
        self.api
            .events
            .subscribe(Some(head))
            .replay
            .iter()
            .filter(|f| f.event == "devices")
            .map(|f| serde_json::from_str(&f.data).unwrap())
            .collect()
    }

    async fn serve(&self, peer: Option<&str>) -> u16 {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = routes::app(Arc::clone(&self.api));
        match peer {
            None => {
                tokio::spawn(serve_listener(listener, app));
            }
            Some(p) => {
                let addr: SocketAddr = p.parse().unwrap();
                let app = app.layer(MockConnectInfo(addr));
                tokio::spawn(async move { axum::serve(listener, app).await });
            }
        }
        port
    }
}

fn ts(s: &str) -> OffsetDateTime {
    OffsetDateTime::parse(s, &Rfc3339).unwrap_or_else(|e| panic!("{s:?}: {e}"))
}

fn expected_id(kind: &str, key: &str) -> String {
    sha_hex(&format!("{kind}:{key}"))[..16].to_string()
}

// ---------------------------------------------------------------- HTTP

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Resp {
    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {:?}", self.body))
    }
    fn problem(&self, status: u16, code: &str) {
        assert_eq!(self.status, status, "{}", self.body);
        assert_eq!(self.json()["code"], code, "{}", self.body);
    }
    /// The `s111=<token>` pair from `Set-Cookie`.
    fn cookie(&self) -> String {
        let sc = self
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
            .map(|(_, v)| v.clone())
            .expect("Set-Cookie");
        sc.split(';').next().unwrap().to_string()
    }
}

fn request(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Resp {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    conn.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
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
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    Resp {
        status,
        headers,
        body: body.to_string(),
    }
}

fn get(port: u16, path: &str) -> Resp {
    request(port, "GET", path, &[], None)
}

fn send_json(port: u16, method: &str, path: &str, body: &str) -> Resp {
    request(
        port,
        method,
        path,
        &[("Content-Type", "application/json")],
        Some(body),
    )
}

// ---------------------------------------------------------------- names

const HELTEC: &str = "Likely: Heltec WiFi LoRa 32 V3";
const ESP: &str = "Likely: ESP32-S3 native USB";
const TBEAM: &str = "Likely: LILYGO T-Beam";
const TECHO: &str = "Likely: LILYGO T-Echo";
const FTDI: &str = "USB serial device (FT232R USB UART)";
const SPI: &str = "Likely: CH341 USB-to-SPI LoRa adapter";

// ---------------------------------------------------------------- tests

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn detects_matches_and_shapes_records() {
    let n = node();
    assert!(n.tick().await, "first tick publishes the baseline");
    let all = n.list();

    // hubs, the HID keyboard, interface links, the BT dongle's USB side and the
    // `hci0:256` connection entry are not devices
    let mut names = n.names();
    names.sort();
    let mystery = format!("USB device (Mystery gadget[2J {})", "x".repeat(46));
    assert_eq!(
        names,
        [
            "Bluetooth adapter (Bluetooth Radio)",
            "Browser on Android device",
            ESP,
            HELTEC,
            TBEAM,
            TECHO,
            "Paired browser",
            mystery.as_str(),
            FTDI,
        ],
        "{all:#?}"
    );

    // Heltec V3: LoRa table (likely) + KB entry + by-id stable id, nothing bound
    let h = n.by_name(HELTEC);
    assert_eq!(h.id, expected_id("radio", HELTEC_BY_ID));
    assert_eq!(h.kind, DeviceKind::Radio);
    assert_eq!(h.transport, Transport::UsbSerial);
    assert_eq!(h.capabilities, ["lora"]);
    assert_eq!(h.service_id.as_deref(), Some("radio"));
    assert_eq!(
        h.kb_entry.as_deref(),
        Some("devices/heltec-wifi-lora-32-v3.md")
    );
    assert_eq!(h.state, DeviceState::NeedsSetup);
    assert_eq!(h.reason_code, "radio_unbound");
    assert!(h.driver.is_none());
    assert!(h.role.is_none());
    assert_eq!(h.identity["vendor_id"], "10c4");
    assert_eq!(h.identity["product_id"], "ea60");
    assert_eq!(h.identity["serial"], "0001");
    assert!(h.identity["port"]
        .as_str()
        .unwrap()
        .ends_with("/dev/ttyUSB0"));
    assert!(h.identity["by_id"]
        .as_str()
        .unwrap()
        .ends_with(HELTEC_BY_ID));
    assert_eq!(h.detail["id_basis"], "serial-by-id");
    assert_eq!(
        h.detail["lora_table_match"],
        "Heltec WiFi LoRa 32 V3 (CP2102)"
    );
    assert!(
        h.detail.get("service_probe").is_none(),
        "USB boards are never probed"
    );
    assert_eq!(h.detail["port_access"], "ok");
    assert_eq!(h.detail["port_mode"], "0660");
    assert_eq!(h.detail["usb_bus_id"], "1-1");
    assert_eq!(h.detail["usb_manufacturer"], "Silicon Labs");
    assert_eq!(h.actions, ["rename", "forget", "agent"]);
    assert!(h.alias.is_none());

    // ESP32-S3 native USB: cdc_acm tty, pid wildcard row, no KB entry
    let e = n.by_name(ESP);
    assert_eq!(e.id, expected_id("radio", ESP_BY_ID));
    assert!(e.kb_entry.is_none());
    assert!(e.identity["port"]
        .as_str()
        .unwrap()
        .ends_with("/dev/ttyACM0"));
    assert_eq!(e.detail["usb_interface_classes"], "02,0a,ff");
    assert_eq!(e.detail["usb_product"], "USB JTAG/serial debug unit");

    // CH340: the table's 1a86:* row over-matches on purpose
    let c = n.by_name(TBEAM);
    assert_eq!(c.kind, DeviceKind::Radio);
    assert_eq!(c.id, expected_id("radio", CH340_BY_ID));

    // unidentified serial device: needs setup (nothing drives it)
    let f = n.by_name(FTDI);
    assert_eq!(f.kind, DeviceKind::Unknown);
    assert_eq!(f.transport, Transport::UsbSerial);
    assert_eq!(f.state, DeviceState::NeedsSetup);
    assert_eq!(f.reason_code, "unbound");
    assert!(f.service_id.is_none() && f.kb_entry.is_none());
    assert!(f.capabilities.is_empty());

    // radio without a tty (DFU), no USB serial number → sysfs-path id
    let d = n.by_name(TECHO);
    assert_eq!(d.id, expected_id("radio", "239a:0029:1-7"));
    assert_eq!(d.detail["id_basis"], "usb-sysfs-path");
    assert_eq!(d.state, DeviceState::NeedsSetup);
    assert!(d.identity.get("port").is_none());

    // hostile sysfs strings: control/bidi chars stripped, product capped at 64
    let m = all
        .iter()
        .find(|r| r.identity.get("vendor_id").is_some_and(|v| v == "1234"))
        .unwrap();
    assert_eq!(m.kind, DeviceKind::Unknown);
    assert_eq!(m.transport, Transport::Unknown);
    assert_eq!(m.reason_code, "unbound");
    let product = m.detail["usb_product"].as_str().unwrap();
    assert_eq!(product.chars().count(), 64);
    assert!(product.starts_with("Mystery gadget[2J xxx"), "{product}");
    assert_eq!(m.identity["product"], product);
    assert_eq!(m.detail["usb_manufacturer"], "Evil Corp");
    assert_eq!(m.identity["serial"], "[31mSN-1");
    assert_eq!(m.id, expected_id("unknown", "1234:5678:[31mSN-1"));
    assert_eq!(m.detail["id_basis"], "usb-serial-number");

    // Bluetooth adapter: MAC id, USB facts of the dongle, nothing uses it yet
    let b = n.by_name("Bluetooth adapter (Bluetooth Radio)");
    assert_eq!(b.kind, DeviceKind::Bluetooth);
    assert_eq!(b.transport, Transport::Bluetooth);
    assert_eq!(b.capabilities, ["ble"]);
    assert_eq!(b.identity["mac"], "AA:BB:CC:DD:EE:FF");
    assert_eq!(b.identity["adapter"], "hci0");
    assert_eq!(b.identity["vendor_id"], "0bda");
    assert_eq!(b.id, expected_id("bluetooth", "AA:BB:CC:DD:EE:FF"));
    assert_eq!(b.detail["id_basis"], "mac");
    assert_eq!(b.matched.confidence, Confidence::Exact);
    assert!(b.driver.is_none());
    assert!(b.firmware.is_none());
    assert_eq!(b.state, DeviceState::Connected, "nothing to set up today");
    assert_eq!(b.reason_code, "adapter_idle");
    assert_eq!(
        b.reason,
        "Present. Nothing on this node uses Bluetooth yet (arrives with Layer 6)."
    );

    // companions from the dashboard sessions: controllers driven by the dashboard
    let fresh = n.by_name("Browser on Android device");
    assert_eq!(fresh.kind, DeviceKind::Companion);
    assert_eq!(fresh.transport, Transport::Lan);
    assert_eq!(fresh.state, DeviceState::Connected);
    assert_eq!(fresh.reason_code, "session_active");
    assert_eq!(fresh.capabilities, ["dashboard-session"]);
    assert_eq!(fresh.identity["ip"], "192.168.1.20");
    assert_eq!(fresh.identity["user_agent"], UA_ANDROID);
    assert_eq!(fresh.id, expected_id("companion", &sha_hex(TOKEN_FRESH)));
    assert_eq!(fresh.actions, ["rename", "forget"]);
    assert_eq!(fresh.role.as_deref(), Some("controller"));
    let drv = fresh.driver.clone().unwrap();
    assert_eq!(
        (drv.stack, drv.interface.as_deref(), drv.live),
        (Stack::Dashboard, Some("session"), true)
    );
    assert_eq!(fresh.matched.confidence, Confidence::Exact);
    assert_eq!(fresh.matched.rule, "session");
    assert!(fresh.firmware.is_none());
    let idle = n.by_name("Paired browser");
    assert_eq!(idle.state, DeviceState::Disconnected);
    assert_eq!(idle.reason_code, "session_idle");
    assert!(idle.identity["user_agent"].is_null());
    assert_eq!(idle.identity["ip"], "192.168.1.21");
    assert_eq!(idle.role.as_deref(), Some("controller"));
    // the session hash is never exported
    let dump = serde_json::to_string(&all).unwrap();
    assert!(!dump.contains(&sha_hex(TOKEN_FRESH)));

    // present first; the idle companion sorts last
    assert_eq!(all.last().unwrap().name, "Paired browser");
    assert!(all[..all.len() - 1].iter().all(|r| r.state.present()));
    // bounded detail, valid ids everywhere
    for r in &all {
        assert!(serde_json::to_string(&r.detail).unwrap().len() <= devices::MAX_DETAIL_BYTES);
        assert_eq!(r.id.len(), 16);
        assert!(devices::valid_id(&r.id));
        assert!(!r.name.chars().any(char::is_control), "{:?}", r.name);
    }

    // HTTP shape (loopback peer)
    let port = n.serve(None).await;
    let r = get(port, "/api/v1/devices");
    assert_eq!(r.status, 200, "{}", r.body);
    let v = r.json();
    assert_eq!(v["schema"], 1);
    let devs = v["devices"].as_array().unwrap();
    assert_eq!(devs.len(), all.len());
    for d in devs {
        for key in [
            "id",
            "kind",
            "alias",
            "name",
            "state",
            "transport",
            "first_seen",
            "last_seen",
            "since",
            "reason_code",
            "reason",
            "capabilities",
            "match",
            "firmware",
            "driver",
            "role",
            "identity",
            "actions",
            "detail",
        ] {
            assert!(d.get(key).is_some(), "{key} missing in {d}");
        }
        for a in d["actions"].as_array().unwrap() {
            assert!(
                ["rename", "forget", "agent"].contains(&a.as_str().unwrap()),
                "Layer 6 actions are not emitted yet: {a}"
            );
        }
    }
    assert_eq!(
        devs[0]["state"].as_str().map(|s| s != "disconnected"),
        Some(true)
    );
    let one = get(port, &format!("/api/v1/devices/{}", h.id)).json();
    assert_eq!(one["kind"], "radio");
    assert_eq!(one["state"], "needs_setup");
    assert_eq!(one["transport"], "usb-serial");
    assert!(one["driver"].is_null());
    assert!(one["role"].is_null());
    assert_eq!(
        one["match"],
        serde_json::json!({"confidence": "likely", "rule": "lora-table 10c4:ea60"})
    );
    assert_eq!(
        one["firmware"],
        serde_json::json!({"family": "unknown", "version": null, "source": "unknown"})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn table_hits_are_likely_and_everything_else_unknown() {
    let n = node();
    n.tick().await;
    // a VID:PID names the serial chip, not the board: table hits are "likely",
    // the raw USB product string stays in identity.product
    for (name, rule, product) in [
        (
            HELTEC,
            "lora-table 10c4:ea60",
            "CP2102 USB to UART Bridge Controller",
        ),
        (ESP, "lora-table 303a:1001", "USB JTAG/serial debug unit"),
        (TBEAM, "lora-table 1a86:7523", "USB Serial"),
        (TECHO, "lora-table 239a:0029", "T-Echo DFU"),
    ] {
        let r = n.by_name(name);
        assert_eq!(r.matched.confidence, Confidence::Likely, "{name}");
        assert_eq!(r.matched.rule, rule, "{name}");
        assert_eq!(r.identity["product"], product, "{name}");
        assert_eq!(
            r.firmware,
            Some(DeviceFirmware {
                family: FirmwareFamily::Unknown,
                version: None,
                source: FirmwareSource::Unknown,
            }),
            "{name}: firmware is unknowable without opening the port"
        );
    }
    // no table row → unknown; serial hardware still says firmware unknown
    let f = n.by_name(FTDI);
    assert_eq!(f.matched.confidence, Confidence::Unknown);
    assert_eq!(f.matched.rule, "usb-ids 0403:6001");
    assert_eq!(f.identity["product"], "FT232R USB UART");
    assert_eq!(f.firmware.as_ref().unwrap().family, FirmwareFamily::Unknown);
    assert_eq!(f.state, DeviceState::NeedsSetup);
    assert_eq!(f.reason_code, "unbound");
    let m = n
        .list()
        .into_iter()
        .find(|r| r.identity.get("vendor_id").is_some_and(|v| v == "1234"))
        .unwrap();
    assert_eq!(m.matched.confidence, Confidence::Unknown);
    assert_eq!(m.matched.rule, "usb-ids 1234:5678");
    // remembered records keep what was known about them
    n.tree.unplug("1-1", Some("ttyUSB0"), Some(HELTEC_BY_ID));
    n.tick().await;
    n.tick().await;
    let gone = n.by_name(HELTEC);
    assert_eq!(gone.state, DeviceState::Disconnected);
    assert_eq!(gone.matched.confidence, Confidence::Likely);
    assert_eq!(gone.firmware.unwrap().family, FirmwareFamily::Unknown);
    assert!(gone.driver.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rnode_interface_binds_the_board_to_reticulum() {
    let n = node();
    let rns = n.dir.path().join("rns/config");
    assert_eq!(
        n.cfg.rns_config, rns,
        "production path is <root>/rns/config"
    );
    w(
        &rns,
        &format!(
            "schema-version = 1\n\n[reticulum]\n  share_instance = no\n\n[interfaces]\n\
             \x20 [[Auto Interface]]\n    type = AutoInterface\n    enabled = yes\n\n\
             \x20 [[Roof RNode]]\n    type = RNodeInterface\n    enabled = yes\n\
             \x20   port = /dev/serial/by-id/{HELTEC_BY_ID}\n    frequency = 867200000\n\n\
             \x20 [[Desk RNode]]\n    type = RNodeInterface\n    interface_enabled = true\n\
             \x20   port = /dev/ttyACM0\n\n\
             \x20 [[Spare]]\n    type = RNodeInterface\n    enabled = no\n    port = /dev/ttyUSB1\n"
        ),
    );
    let rnsd = Arc::new(AtomicBool::new(false));
    {
        let rnsd = Arc::clone(&rnsd);
        n.reg.set_rnsd_running(move || rnsd.load(Ordering::SeqCst));
    }
    // configured, but rnsd has not run: unbound, not an error
    n.tick().await;
    let h = n.by_name(HELTEC);
    assert_eq!(h.state, DeviceState::NeedsSetup);
    assert_eq!(h.reason_code, "radio_unbound");
    assert!(h.driver.is_none());
    assert!(h.reason.contains("Roof RNode"), "{}", h.reason);
    assert_eq!(h.detail["rns_interface"], "Roof RNode");
    assert_eq!(h.detail["rnsd_running"], false);

    // rnsd runs → bound (after the debounce)
    rnsd.store(true, Ordering::SeqCst);
    n.tick().await;
    n.tick().await;

    // by-id port form
    let h = n.by_name(HELTEC);
    assert_eq!(h.state, DeviceState::Connected);
    assert_eq!(h.reason_code, "stack_bound");
    assert_eq!(
        h.driver,
        Some(DeviceDriver {
            stack: Stack::Reticulum,
            interface: Some("Roof RNode".into()),
            live: true,
        })
    );
    assert_eq!(h.detail["rns_interface"], "Roof RNode");
    assert_eq!(h.detail["rnsd_running"], true);
    assert!(h.detail["rns_port"]
        .as_str()
        .unwrap()
        .ends_with(HELTEC_BY_ID));
    // binding says nothing about firmware: still a likely match, firmware unknown
    assert_eq!(h.matched.confidence, Confidence::Likely);
    assert_eq!(h.firmware.as_ref().unwrap().family, FirmwareFamily::Unknown);
    // plain /dev/ttyACM0 port form
    let e = n.by_name(ESP);
    assert_eq!(e.state, DeviceState::Connected);
    assert_eq!(
        e.driver.as_ref().unwrap().interface.as_deref(),
        Some("Desk RNode")
    );
    // a disabled stanza binds nothing
    let c = n.by_name(TBEAM);
    assert_eq!(c.state, DeviceState::NeedsSetup);
    assert_eq!(c.reason_code, "radio_unbound");
    assert!(c.driver.is_none());
    // JSON shape
    let v = serde_json::to_value(&h).unwrap();
    assert_eq!(
        v["driver"],
        serde_json::json!({"stack": "reticulum", "interface": "Roof RNode", "live": true})
    );

    // rnsd stops: a previously live stack is down → error (after the debounce)
    rnsd.store(false, Ordering::SeqCst);
    n.tick().await;
    assert_eq!(n.by_name(HELTEC).state, DeviceState::Connected, "debounced");
    n.tick().await;
    let h = n.by_name(HELTEC);
    assert_eq!(h.state, DeviceState::Error);
    assert_eq!(h.reason_code, "stack_down");
    assert_eq!(
        h.driver,
        Some(DeviceDriver {
            stack: Stack::Reticulum,
            interface: Some("Roof RNode".into()),
            live: false,
        })
    );
    assert!(h.reason.contains("Roof RNode"), "{}", h.reason);
    // stays down tick after tick (not flipped back to unbound)
    n.tick().await;
    assert_eq!(n.by_name(HELTEC).reason_code, "stack_down");

    // the stanza is removed: nothing bound → needs_setup
    rnsd.store(true, Ordering::SeqCst);
    w(
        &rns,
        "[interfaces]\n  [[Auto Interface]]\n    type = AutoInterface\n    enabled = yes\n",
    );
    n.tick().await;
    n.tick().await;
    let h = n.by_name(HELTEC);
    assert_eq!(h.state, DeviceState::NeedsSetup);
    assert_eq!(h.reason_code, "radio_unbound");
    assert!(h.driver.is_none());
    assert!(h.detail.get("rns_interface").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn meshtasticd_drives_spi_bridges_only() {
    let mut n = node_with(
        |t| {
            t.heltec();
            t.ch341_spi();
        },
        false,
    );
    // a fake meshtasticd on the probe port
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    n.cfg.probe_addr = listener.local_addr().unwrap();
    n.reg = devices::install(&n.api, n.cfg.clone());
    n.tick().await;

    let s = n.by_name(SPI);
    assert_eq!(s.kind, DeviceKind::Radio);
    assert_eq!(s.transport, Transport::Spi);
    assert_eq!(s.service_id.as_deref(), Some("radio"));
    assert_eq!(s.matched.rule, "spi-bridge 1a86:5512");
    assert_eq!(s.matched.confidence, Confidence::Likely);
    assert_eq!(s.state, DeviceState::Connected);
    assert_eq!(
        s.driver,
        Some(DeviceDriver {
            stack: Stack::Meshtastic,
            interface: Some("meshtasticd".into()),
            live: true,
        })
    );
    assert_eq!(s.detail["service_probe"], "open");
    assert_eq!(
        s.detail["service_probe_addr"],
        listener.local_addr().unwrap().to_string()
    );
    assert_eq!(serde_json::to_value(&s).unwrap()["transport"], "spi");
    // meshtasticd never drives a USB serial board, even when it answers
    let h = n.by_name(HELTEC);
    assert_eq!(h.state, DeviceState::NeedsSetup);
    assert!(h.driver.is_none());

    // meshtasticd stops answering after it was live → stack_down (debounced)
    drop(listener);
    n.tick().await;
    n.tick().await;
    let s = n.by_name(SPI);
    assert_eq!(s.state, DeviceState::Error);
    assert_eq!(s.reason_code, "stack_down");
    assert_eq!(
        s.driver,
        Some(DeviceDriver {
            stack: Stack::Meshtastic,
            interface: Some("meshtasticd".into()),
            live: false,
        })
    );
    assert_eq!(s.detail["service_probe"], "refused");

    // unplugged and plugged in again with meshtasticd still down: a fresh
    // device, never seen live → unbound
    n.tree.unplug("1-10", None, None);
    n.tick().await;
    n.tick().await;
    assert_eq!(n.by_name(SPI).state, DeviceState::Disconnected);
    n.tree.ch341_spi();
    n.tick().await;
    n.tick().await;
    let s = n.by_name(SPI);
    assert_eq!(s.state, DeviceState::NeedsSetup);
    assert_eq!(s.reason_code, "radio_unbound");
    assert!(s.driver.is_none());

    // an SPI bridge with no meshtasticd from the start is unbound too
    let fresh = node_with(|t| t.ch341_spi(), false);
    fresh.tick().await;
    let s = fresh.by_name(SPI);
    assert_eq!(s.state, DeviceState::NeedsSetup);
    assert_eq!(s.reason_code, "radio_unbound");
    assert_eq!(s.detail["service_probe"], "refused");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ids_are_stable_across_scans_and_restarts() {
    let n = node();
    n.tick().await;
    let first: Vec<(String, String, String)> = n
        .list()
        .into_iter()
        .map(|r| (r.name, r.id, r.first_seen))
        .collect();
    n.tick().await;
    let second: Vec<(String, String, String)> = n
        .list()
        .into_iter()
        .map(|r| (r.name, r.id, r.first_seen))
        .collect();
    assert_eq!(first, second);
    // a fresh registry (daemon restart) over the same store and hardware
    let again = DeviceRegistry::new(n.cfg.clone());
    again.tick(&n.api).await;
    let third: Vec<(String, String, String)> = again
        .list()
        .unwrap()
        .into_iter()
        .map(|r| (r.name, r.id, r.first_seen))
        .collect();
    assert_eq!(first, third, "ids and first_seen survive a restart");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn alias_patch_persists_validates_and_publishes() {
    let n = node();
    n.tick().await;
    let port = n.serve(None).await;
    let h = n.by_name(HELTEC);
    let path = format!("/api/v1/devices/{}", h.id);

    let head = n.api.events.head();
    let r = send_json(port, "PATCH", &path, r#"{"alias":"  Roof radio  "}"#);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["alias"], "Roof radio");
    assert_eq!(r.json()["id"], h.id.as_str());
    // persisted in devices.json, keyed by id
    let store = n.store();
    assert_eq!(store["schema"], 1);
    assert_eq!(store["devices"][&h.id]["alias"], "Roof radio");
    assert_eq!(store["devices"][&h.id]["forgotten"], false);
    assert_eq!(
        store["devices"][&h.id]["last_record_summary"]["name"],
        HELTEC
    );
    assert_eq!(
        store["devices"][&h.id]["last_record_summary"]["match"]["confidence"],
        "likely"
    );
    // published at once, and visible on GET
    let frames = n.frames_since(head);
    assert_eq!(frames.len(), 1);
    assert!(frames[0]["devices"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["alias"] == "Roof radio"));
    assert_eq!(get(port, &path).json()["alias"], "Roof radio");
    // the next tick does not re-announce the same snapshot
    assert!(!n.tick().await);
    // survives a restart
    let again = DeviceRegistry::new(n.cfg.clone());
    again.tick(&n.api).await;
    assert_eq!(
        again.get(&h.id).unwrap().alias.as_deref(),
        Some("Roof radio")
    );

    // body without alias: no change, current record returned
    let same = send_json(port, "PATCH", &path, "{}");
    assert_eq!(same.status, 200);
    assert_eq!(same.json()["alias"], "Roof radio");
    // "" clears exactly like null
    assert!(send_json(port, "PATCH", &path, r#"{"alias":""}"#).json()["alias"].is_null());
    assert!(n.store()["devices"][&h.id]["alias"].is_null());
    send_json(port, "PATCH", &path, r#"{"alias":"x"}"#);
    assert!(send_json(port, "PATCH", &path, r#"{"alias":null}"#).json()["alias"].is_null());
    assert!(n.store()["devices"][&h.id]["alias"].is_null());
    send_json(port, "PATCH", &path, r#"{"alias":"x"}"#);
    assert!(send_json(port, "PATCH", &path, r#"{"alias":"   "}"#).json()["alias"].is_null());
    // validation
    let long = format!(r#"{{"alias":"{}"}}"#, "a".repeat(65));
    send_json(port, "PATCH", &path, &long).problem(400, "bad_request");
    send_json(port, "PATCH", &path, r#"{"alias":"a\u0007b"}"#).problem(400, "bad_request");
    send_json(port, "PATCH", &path, r#"{"alias":7}"#).problem(400, "bad_request");
    send_json(port, "PATCH", &path, "not json").problem(400, "bad_request");
    send_json(port, "PATCH", "/api/v1/devices/abc", r#"{"alias":"x"}"#).problem(400, "bad_request");
    send_json(port, "PATCH", "/api/v1/devices/a.b.c.d", r#"{"alias":"x"}"#)
        .problem(400, "bad_request");
    send_json(
        port,
        "PATCH",
        "/api/v1/devices/0000000000000000",
        r#"{"alias":"x"}"#,
    )
    .problem(404, "not_found");
    get(port, "/api/v1/devices/0000000000000000").problem(404, "not_found");
    get(port, "/api/v1/devices/ab").problem(400, "bad_request");
    // CSRF guard applies (mutations must be JSON)
    assert_eq!(
        request(
            port,
            "PATCH",
            &path,
            &[("Content-Type", "text/plain")],
            Some("{}")
        )
        .status,
        415
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forget_hides_until_hardware_reappears() {
    let n = node();
    n.tick().await;
    let port = n.serve(None).await;
    let f = n.by_name(FTDI);
    let path = format!("/api/v1/devices/{}", f.id);
    send_json(port, "PATCH", &path, r#"{"alias":"Bench FTDI"}"#);

    let head = n.api.events.head();
    let r = request(
        port,
        "DELETE",
        &path,
        &[("Content-Type", "application/json")],
        None,
    );
    assert_eq!(r.status, 204, "{}", r.body);
    assert_eq!(n.frames_since(head).len(), 1, "forget publishes at once");
    get(port, &path).problem(404, "not_found");
    assert!(!n.names().contains(&f.name));
    let store = n.store();
    assert_eq!(store["devices"][&f.id]["forgotten"], true);
    assert!(
        store["devices"][&f.id]["alias"].is_null(),
        "forget drops the alias"
    );
    // forgetting twice → 404
    request(
        port,
        "DELETE",
        &path,
        &[("Content-Type", "application/json")],
        None,
    )
    .problem(404, "not_found");

    // still plugged in: stays hidden, tick after tick (and across a restart)
    n.tick().await;
    n.tick().await;
    assert!(!n.names().contains(&f.name));
    let again = DeviceRegistry::new(n.cfg.clone());
    again.tick(&n.api).await;
    assert!(again.get(&f.id).is_none());

    // unplugged for 2 ticks → the forget mark is dropped with the record
    n.tree.unplug_ftdi();
    n.tick().await;
    n.tick().await;
    assert!(n.store()["devices"].get(&f.id).is_none());
    assert!(
        !n.names().contains(&f.name),
        "forgotten + absent: not remembered"
    );

    // plugged in again → a fresh device (same stable id, no alias)
    n.tree.ftdi();
    n.tick().await;
    n.tick().await;
    let back = n.by_name(FTDI);
    assert_eq!(back.id, f.id);
    assert!(back.alias.is_none());
    assert!(back.state.present());
    assert_eq!(n.store()["devices"][&f.id]["forgotten"], false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forget_companion_revokes_its_session() {
    let n = node();
    n.tick().await;
    let port = n.serve(None).await;
    let c = n.by_name("Browser on Android device");
    assert!(n.api.auth.session_for(TOKEN_FRESH).is_some());

    let r = request(
        port,
        "DELETE",
        &format!("/api/v1/devices/{}", c.id),
        &[("Content-Type", "application/json")],
        None,
    );
    assert_eq!(r.status, 204, "{}", r.body);
    assert!(
        n.api.auth.session_for(TOKEN_FRESH).is_none(),
        "session revoked"
    );
    assert!(!n
        .api
        .auth
        .sessions()
        .iter()
        .any(|s| s.hash == sha_hex(TOKEN_FRESH)));
    let raw =
        std::fs::read_to_string(n.dir.path().join("state").join(auth::SESSIONS_FILE)).unwrap();
    assert!(!raw.contains(&sha_hex(TOKEN_FRESH)), "revocation persisted");
    // the other companion is untouched; the forgotten one stays gone
    n.tick().await;
    n.tick().await;
    assert!(!n.names().contains(&c.name));
    assert!(n.names().contains(&"Paired browser".to_string()));
    assert!(n.store()["devices"].get(&c.id).is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forgetting_your_own_companion_logs_you_out() {
    let n = node_with(|t| t.heltec(), false);
    let lan = n.serve(Some("192.168.1.77:40000")).await;
    let code = auth::read_pair_code(&n.api.layout.state_dir())
        .unwrap()
        .unwrap();
    let r = request(
        lan,
        "POST",
        "/api/v1/auth/pair",
        &[
            ("Content-Type", "application/json"),
            ("User-Agent", "Mozilla/5.0 (Linux; Android 14)"),
        ],
        Some(&format!(r#"{{"code":"{code}"}}"#)),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let cookie = r.cookie();
    n.tick().await;

    // the paired browser finds itself in the registry ...
    let list = request(lan, "GET", "/api/v1/devices", &[("Cookie", &cookie)], None);
    assert_eq!(list.status, 200, "{}", list.body);
    let me = list.json()["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["kind"] == "companion" && d["identity"]["ip"] == "192.168.1.77")
        .cloned()
        .expect("own companion record");
    // ... forgets itself: allowed, 204 ...
    let r = request(
        lan,
        "DELETE",
        &format!("/api/v1/devices/{}", me["id"].as_str().unwrap()),
        &[("Content-Type", "application/json"), ("Cookie", &cookie)],
        None,
    );
    assert_eq!(r.status, 204, "{}", r.body);
    // ... and its next request is unauthenticated
    request(lan, "GET", "/api/v1/devices", &[("Cookie", &cookie)], None)
        .problem(401, "unauthenticated");
    assert!(n.api.auth.sessions().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn auth_session_names_the_callers_own_companion() {
    let n = node_with(|t| t.heltec(), true); // two pre-existing sessions too
    let lan = n.serve(Some("192.168.1.78:40000")).await;
    let code = auth::read_pair_code(&n.api.layout.state_dir())
        .unwrap()
        .unwrap();
    let r = request(
        lan,
        "POST",
        "/api/v1/auth/pair",
        &[("Content-Type", "application/json")],
        Some(&format!(r#"{{"code":"{code}"}}"#)),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let cookie = r.cookie();
    let paired_id = r.json()["device_id"].as_str().unwrap().to_string();
    n.tick().await;

    let session = request(
        lan,
        "GET",
        "/api/v1/auth/session",
        &[("Cookie", &cookie)],
        None,
    );
    assert_eq!(session.json()["device_id"], paired_id.as_str());
    let list = request(lan, "GET", "/api/v1/devices", &[("Cookie", &cookie)], None).json();
    let companions: Vec<_> = list["devices"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["kind"] == "companion")
        .collect();
    assert!(companions.len() >= 2, "{list}");
    let mine: Vec<_> = companions
        .iter()
        .filter(|d| d["id"] == paired_id.as_str())
        .collect();
    assert_eq!(
        mine.len(),
        1,
        "exactly one companion is this browser: {list}"
    );
    assert_eq!(mine[0]["identity"]["ip"], "192.168.1.78");
    // unpaired: null
    let v = request(lan, "GET", "/api/v1/auth/session", &[], None).json();
    assert!(v["device_id"].is_null());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn absent_hardware_is_remembered_with_its_alias() {
    let n = node();
    n.tick().await;
    let f = n.by_name(FTDI);
    n.reg
        .set_alias(&n.api, &f.id, Some("Bench FTDI".into()))
        .unwrap();
    let seen = n.reg.get(&f.id).unwrap().last_seen;

    n.tree.unplug_ftdi();
    n.tick().await;
    n.tick().await;
    let d = n.reg.get(&f.id).expect("remembered");
    assert_eq!(d.state, DeviceState::Disconnected);
    assert_eq!(d.alias.as_deref(), Some("Bench FTDI"));
    assert_eq!(d.reason_code, "unplugged");
    assert_eq!(d.name, FTDI);
    assert_eq!(d.identity["vendor_id"], "0403");
    assert_eq!(d.detail["remembered"], true);
    assert_eq!(d.detail["id_basis"], "serial-by-id");
    assert!(d.driver.is_none());
    assert_eq!(
        ts(&d.last_seen),
        ts(&seen),
        "last_seen = when it was last present"
    );
    // disconnected rows sort after present ones
    let all = n.list();
    let pos = all.iter().position(|r| r.id == f.id).unwrap();
    assert!(all[pos..].iter().all(|r| !r.state.present()));

    // after a restart (store only): still there, disconnected, alias kept,
    // `since` = when it was last seen
    let again = DeviceRegistry::new(n.cfg.clone());
    again.tick(&n.api).await;
    let r = again.get(&f.id).unwrap();
    assert_eq!(r.state, DeviceState::Disconnected);
    assert_eq!(r.alias.as_deref(), Some("Bench FTDI"));
    assert_eq!(r.since, r.last_seen);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changes_publish_only_after_two_identical_ticks() {
    let n = node();
    let h0 = n.api.events.head();
    assert!(n.tick().await, "baseline");
    assert_eq!(n.frames_since(h0).len(), 1);
    let f = n.by_name(FTDI);
    let since0 = n.by_name(HELTEC).since;
    assert!(!n.tick().await, "nothing changed");

    // a one-tick flap is never published
    let head = n.api.events.head();
    n.tree.unplug_ftdi();
    assert!(!n.tick().await);
    n.tree.ftdi();
    assert!(!n.tick().await);
    assert!(!n.tick().await);
    assert_eq!(n.api.events.head(), head, "no frames for a flap");
    assert!(n.reg.get(&f.id).unwrap().state.present());

    // a real change: tick 1 holds, tick 2 publishes
    n.tree.unplug_ftdi();
    assert!(!n.tick().await, "one tick: no publish");
    assert_eq!(n.api.events.head(), head);
    assert!(n.reg.get(&f.id).unwrap().state.present());
    assert!(n.tick().await, "two ticks: publish");
    let frames = n.frames_since(head);
    assert_eq!(frames.len(), 1);
    let fd = frames[0]["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == f.id.as_str())
        .unwrap()
        .clone();
    assert_eq!(fd["state"], "disconnected");
    // `since` moved only for the device whose state changed
    let now = n.reg.get(&f.id).unwrap();
    assert!(
        ts(&now.since) > ts(&f.since),
        "{} vs {}",
        now.since,
        f.since
    );
    assert_eq!(n.by_name(HELTEC).since, since0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unopenable_tty_is_an_error() {
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("running as root: access(2) always succeeds — skipped");
        return;
    }
    let n = node();
    for tty in ["ttyUSB2", "ttyUSB0"] {
        std::fs::set_permissions(
            n.tree.root.join("dev").join(tty),
            std::fs::Permissions::from_mode(0o000),
        )
        .unwrap();
    }
    n.tick().await;
    let f = n.by_name(FTDI);
    assert_eq!(f.state, DeviceState::Error);
    assert_eq!(f.reason_code, "no_port_access");
    assert_eq!(f.detail["port_access"], "denied");
    assert_eq!(f.detail["port_mode"], "0000");
    // a radio with an unreadable port: error too
    let h = n.by_name(HELTEC);
    assert_eq!(h.state, DeviceState::Error);
    assert_eq!(h.reason_code, "no_port_access");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pairing_records_the_user_agent_for_companions() {
    let n = node_with(|t| t.heltec(), false);
    let lan = n.serve(Some("192.168.1.77:40000")).await;
    // LAN peers need a session for the registry like every gated route
    get(lan, "/api/v1/devices").problem(401, "unauthenticated");
    let code = auth::read_pair_code(&n.api.layout.state_dir())
        .unwrap()
        .unwrap();
    // an oversized user agent is capped at 256 chars when stored
    let ua = format!(
        "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) {}",
        "x".repeat(300)
    );
    let r = request(
        lan,
        "POST",
        "/api/v1/auth/pair",
        &[("Content-Type", "application/json"), ("User-Agent", &ua)],
        Some(&format!(r#"{{"code":"{code}"}}"#)),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let sessions = n.api.auth.sessions();
    assert_eq!(sessions.len(), 1);
    let stored = sessions[0].user_agent.clone().unwrap();
    assert_eq!(stored.chars().count(), 256);
    assert!(ua.starts_with(&stored));
    n.tick().await;
    let c = n.by_name("Browser on iPhone");
    assert_eq!(c.identity["ip"], "192.168.1.77");
    assert_eq!(c.identity["user_agent"], stored.as_str());
    assert_eq!(c.state, DeviceState::Connected);
    assert_eq!(c.first_seen, sessions[0].created);

    // a session ended elsewhere (logout / rotate) drops the companion
    n.api.auth.rotate_all();
    n.tick().await;
    n.tick().await;
    assert!(!n.names().contains(&"Browser on iPhone".to_string()));
    assert!(n.store()["devices"].get(&c.id).is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn routes_answer_503_when_the_registry_is_not_running() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let dash: SharedState = Arc::new(RwLock::new(DashboardState::new("2026.10.13")));
    let api = ApiState::new(dash, ApiConfig::new(layout, false));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve_listener(listener, routes::app(api)));
    get(port, "/api/v1/devices").problem(503, "internal");
}
