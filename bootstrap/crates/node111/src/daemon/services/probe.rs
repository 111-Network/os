//! Production probes + the registry task (spec §7). Everything here only
//! gathers [`Facts`]; the rules live in [`super::compute`].
//!
//! Cadence: one 2 s tick (child states, `GET :11182/health` and
//! `GET :11181/health` with a 2 s timeout, TCP connect to `127.0.0.1:4403` with
//! a 1 s timeout — all concurrent); every 5th tick (10 s) the agent's own
//! status (`GET :11182/v1/agent/status`, for `/status.intelligence`), PATH
//! lookups (`opencode`, `python3`) and one board-detection pass (R4's
//! `devices::detect::scan` + `devices::rns::bindings`, matched against the
//! LoRa table here); a separate WAN
//! task every 60 s (HTTP 204 probe, 3 s timeout per URL, same URLs as the
//! installer's connectivity check).

use super::{
    rfc3339_now, AgentStatusProbe, ChildFacts, Facts, GatewayFacts, Probe, Radio, RadioFacts,
    Registry, RnodeBound, MESHTASTIC_PORT,
};
use crate::daemon::dashboard::ApiState;
use crate::daemon::devices::detect::{self, Roots, Scan, UsbInfo};
use crate::daemon::devices::rns::{self, RnodeBinding};
use crate::daemon::intelligence::{self, ChildDisplay, IntelligenceHandle};
use crate::daemon::supervise::ChildState;
use crate::daemon::transport::TransportHandle;
use crate::daemon::DaemonLayout;
use crate::install::probe::{lora_table, RadioRule};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const TICK: Duration = Duration::from_secs(2);
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
pub const PORT_TIMEOUT: Duration = Duration::from_secs(1);
pub const WAN_INTERVAL: Duration = Duration::from_secs(60);
pub const WAN_TIMEOUT: Duration = Duration::from_secs(3);
/// Ticks between the slower probes (agent status, PATH, sysfs): 10 s.
const SLOW_EVERY: u64 = 5;

/// Live handles the registry reads (daemon start hook).
pub struct Sources {
    pub intelligence: Arc<IntelligenceHandle>,
    pub transport: Arc<TransportHandle>,
}

// ---------------------------------------------------------------- loopback HTTP

/// Write all of `buf` (tokio's `io-util` is not enabled in this crate; the
/// readiness API needs only `net`).
pub async fn write_all(s: &tokio::net::TcpStream, mut buf: &[u8]) -> std::io::Result<()> {
    while !buf.is_empty() {
        s.writable().await?;
        match s.try_write(buf) {
            Ok(n) => buf = &buf[n..],
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Read until EOF or `cap` bytes.
pub async fn read_capped(s: &tokio::net::TcpStream, cap: usize) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 4096];
    while out.len() < cap {
        s.readable().await?;
        match s.try_read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&chunk[..n.min(cap - out.len())]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// Minimal loopback `GET` (HTTP/1.0, so no chunked bodies): `(status, body)`.
/// The body is capped at `cap` bytes; the whole exchange at `timeout`.
pub async fn http_get(
    port: u16,
    path: &str,
    timeout: Duration,
    cap: usize,
) -> Result<(u16, Vec<u8>), String> {
    let exchange = async {
        let s = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|e| e.to_string())?;
        let req = format!(
            "GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\n\r\n"
        );
        write_all(&s, req.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let buf = read_capped(&s, cap).await.map_err(|e| e.to_string())?;
        parse_response(&buf)
    };
    tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| format!("no answer within {} ms", timeout.as_millis()))?
}

fn parse_response(buf: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let split = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("malformed HTTP response")?;
    let head = String::from_utf8_lossy(&buf[..split]);
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or("malformed HTTP status line")?;
    Ok((status, buf[split + 4..].to_vec()))
}

async fn health(port: u16) -> Probe {
    match http_get(port, "/health", HEALTH_TIMEOUT, 16 * 1024).await {
        Ok((code, _)) => Probe::Status(code),
        Err(e) => Probe::Unreachable(e),
    }
}

/// `GET :11182/v1/agent/status` → `{state, model.id}`; `None` when unreachable.
pub async fn agent_status() -> Option<AgentStatusProbe> {
    let (code, body) = http_get(
        intelligence::AGENT_PORT,
        "/v1/agent/status",
        HEALTH_TIMEOUT,
        64 * 1024,
    )
    .await
    .ok()?;
    if code != 200 {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let state = v["state"]
        .as_str()
        .filter(|s| matches!(*s, "local" | "offloaded" | "unavailable"))
        .map(str::to_owned);
    let model_id = v["model"]["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    Some(AgentStatusProbe { state, model_id })
}

/// TCP connect to `127.0.0.1:port` within `timeout`.
pub async fn tcp_open(port: u16, timeout: Duration) -> bool {
    matches!(
        tokio::time::timeout(
            timeout,
            tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        )
        .await,
        Ok(Ok(_))
    )
}

// ---------------------------------------------------------------- WAN

/// The installer's HTTP-204 probe URLs (`NODE111_PROBE_204` overrides,
/// comma-separated — same knob as `111 install`).
pub fn wan_probe_urls() -> Vec<String> {
    match std::env::var("NODE111_PROBE_204") {
        Ok(v) if !v.trim().is_empty() => v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => vec![
            crate::install::preflight::PROBE_204_PRIMARY.to_string(),
            crate::install::preflight::PROBE_204_FALLBACK.to_string(),
        ],
    }
}

/// Blocking WAN probe: online iff some URL answers exactly `204` (a captive
/// portal's 200 page does not count).
pub fn wan_probe(urls: &[String], timeout: Duration) -> GatewayFacts {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::WebPki)
        .build();
    let config = ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut last = GatewayFacts {
        online: Some(false),
        checked_at: Some(rfc3339_now()),
        ..GatewayFacts::default()
    };
    for url in urls {
        last.url = Some(url.clone());
        match agent.head(url).call() {
            Ok(resp) if resp.status().as_u16() == 204 => {
                last.online = Some(true);
                last.error = None;
                return last;
            }
            Ok(resp) => last.error = Some(format!("answered {}", resp.status().as_u16())),
            Err(e) => last.error = Some(e.to_string()),
        }
    }
    if urls.is_empty() {
        last.error = Some("no probe URLs configured".into());
    }
    last
}

// ---------------------------------------------------------------- files / PATH / sysfs

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `which name` over `$PATH`, then `extra` directories.
pub fn which(name: &str, extra: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(extra.iter().map(PathBuf::from))
        .map(|d| d.join(name))
        .find(|p| is_executable(p))
}

/// The LoRa-table row a USB device matches (installer rule: VID:PID and, when
/// the row has one, a case-insensitive product substring).
fn rule_hit<'a>(rules: &'a [RadioRule], u: &UsbInfo) -> Option<&'a RadioRule> {
    let product = u.product.as_deref().unwrap_or("").to_lowercase();
    rules.iter().find(|r| {
        r.vid == u.vendor_id
            && (r.pid == "*" || r.pid == u.product_id)
            && r.product_contains
                .as_ref()
                .is_none_or(|want| product.contains(want.as_str()))
    })
}

/// LoRa boards in one detection pass: serial ports and tty-less USB devices
/// (bootloader mode) whose USB ids match the table.
pub fn boards_from_scan(scan: &Scan, rules: &[RadioRule]) -> Vec<Radio> {
    let board = |u: &UsbInfo, tty: Option<&str>, by_id: Option<&str>| {
        rule_hit(rules, u).map(|r| Radio {
            vid: u.vendor_id.clone(),
            pid: u.product_id.clone(),
            product: u.product.clone().unwrap_or_default(),
            name: r.name.clone(),
            tty: tty.map(str::to_owned),
            by_id: by_id.map(str::to_owned),
        })
    };
    let serial = scan.serial.iter().filter_map(|p| {
        p.usb
            .as_ref()
            .and_then(|u| board(u, Some(&p.tty), p.by_id.as_deref()))
    });
    let other = scan.usb_other.iter().filter_map(|u| board(u, None, None));
    serial.chain(other).collect()
}

/// RNode stanzas whose port resolves to a serial device present in `scan`
/// (any serial device: a configured RNode port is a radio by declaration).
pub fn bound_rnodes(bindings: &[RnodeBinding], scan: &Scan) -> Vec<RnodeBound> {
    bindings
        .iter()
        .filter_map(|b| {
            let tty = b.tty.as_deref()?;
            scan.serial
                .iter()
                .any(|p| p.tty == tty)
                .then(|| RnodeBound {
                    interface: b.name.clone(),
                    port: b.port.clone(),
                    tty: tty.to_string(),
                })
        })
        .collect()
}

/// Facts refreshed every [`SLOW_EVERY`] ticks.
#[derive(Debug, Clone, Default)]
struct SlowFacts {
    python3: bool,
    opencode: Option<String>,
    boards: Option<Vec<Radio>>,
    rnode: Vec<RnodeBound>,
}

fn slow_facts(layout: &DaemonLayout) -> SlowFacts {
    let roots = Roots::default();
    let scan = detect::scan(&roots);
    let usb_readable = roots.sys.join("bus/usb/devices").is_dir();
    let bindings = rns::bindings(
        &crate::daemon::transport::config::config_path(layout),
        &roots,
    );
    SlowFacts {
        python3: which("python3", &[]).is_some(),
        opencode: which("opencode", &[]).map(|p| p.to_string_lossy().into_owned()),
        boards: usb_readable.then(|| boards_from_scan(&scan, &lora_table())),
        rnode: bound_rnodes(&bindings, &scan),
    }
}

/// Tracks whether a radio stack is bound and for how long a bound stack has
/// not been answering.
#[derive(Debug, Default)]
pub struct StackWatch {
    meshtastic_seen: bool,
    down_since: Option<Instant>,
}

impl StackWatch {
    /// One observation → `(meshtastic_seen, down_for)`. Bound = an RNode
    /// stanza on a present port, or meshtasticd answered this lifetime; live
    /// = rnsd running with such a stanza, or meshtasticd answering now.
    pub fn observe(
        &mut self,
        rnode_configured: bool,
        rnsd_running: bool,
        meshtastic_open: bool,
        now: Instant,
    ) -> (bool, Option<Duration>) {
        self.meshtastic_seen |= meshtastic_open;
        let live = (rnode_configured && rnsd_running) || meshtastic_open;
        let bound = rnode_configured || self.meshtastic_seen;
        self.down_since = if live || !bound {
            None
        } else {
            Some(self.down_since.unwrap_or(now))
        };
        (
            self.meshtastic_seen,
            self.down_since.map(|t| now.saturating_duration_since(t)),
        )
    }
}

/// Per-tick file facts under the node root.
#[derive(Debug, Clone, Default)]
struct FileFacts {
    agent_binary: bool,
    model_binary: bool,
    model_file: Option<String>,
    agent_log: Option<String>,
    model_log: Option<String>,
    transport_log: Option<String>,
    kb_payload_present: bool,
}

/// `<root>/rns/rnsd.log` (the rnsd child spec's stdout log).
pub fn rnsd_log(layout: &DaemonLayout) -> PathBuf {
    layout.root.join("rns/rnsd.log")
}

fn file_facts(layout: &DaemonLayout) -> FileFacts {
    let exists = |p: PathBuf| p.is_file().then(|| p.to_string_lossy().into_owned());
    FileFacts {
        agent_binary: intelligence::paths::agent_binary(layout).is_file(),
        model_binary: intelligence::paths::model_binary(layout).is_file(),
        model_file: intelligence::paths::model_file(layout)
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())),
        agent_log: exists(intelligence::paths::agent_log(layout)),
        model_log: exists(intelligence::paths::model_log(layout)),
        transport_log: exists(rnsd_log(layout)),
        kb_payload_present: layout.current_release().join("share/kb").is_dir(),
    }
}

/// Remembers when each child PID first appeared (start of its grace window).
#[derive(Debug, Default)]
pub struct SpawnTracker {
    pid: Option<u32>,
    at: Option<Instant>,
}

impl SpawnTracker {
    /// Observe the current PID; returns time since the latest spawn.
    pub fn observe(&mut self, pid: Option<u32>, now: Instant) -> Option<Duration> {
        if let Some(p) = pid {
            if self.pid != Some(p) {
                self.pid = Some(p);
                self.at = Some(now);
            }
        }
        self.at.map(|t| now.saturating_duration_since(t))
    }
}

// ---------------------------------------------------------------- the task

/// Start the registry: inserts `Arc<Registry>` into `api` ext, spawns the 2 s
/// tick and the 60 s WAN probe, publishes `services` frames on change.
pub fn spawn(api: Arc<ApiState>, src: Sources) -> Arc<Registry> {
    let registry = Arc::new(Registry::new());
    api.insert_ext(Arc::clone(&registry));
    let gateway = Arc::new(Mutex::new(GatewayFacts::default()));

    {
        let gateway = Arc::clone(&gateway);
        tokio::spawn(async move {
            let urls = wan_probe_urls();
            loop {
                let urls = urls.clone();
                let g = tokio::task::spawn_blocking(move || wan_probe(&urls, WAN_TIMEOUT))
                    .await
                    .unwrap_or_default();
                *gateway.lock().unwrap() = g;
                tokio::time::sleep(WAN_INTERVAL).await;
            }
        });
    }

    {
        let registry = Arc::clone(&registry);
        tokio::spawn(async move {
            let started = Instant::now();
            let mut agent_spawn = SpawnTracker::default();
            let mut model_spawn = SpawnTracker::default();
            let mut stack = StackWatch::default();
            let mut slow = SlowFacts::default();
            let mut n: u64 = 0;
            let mut tick = tokio::time::interval(TICK);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let layout = api.layout.clone();
                let files = tokio::task::spawn_blocking(move || file_facts(&layout))
                    .await
                    .unwrap_or_default();
                if n.is_multiple_of(SLOW_EVERY) {
                    let layout = api.layout.clone();
                    slow = tokio::task::spawn_blocking(move || slow_facts(&layout))
                        .await
                        .unwrap_or_default();
                }
                let intel = src.intelligence.state();
                let agent_up = intel.agent == ChildDisplay::Running;
                let model_up = intel.model_server == ChildDisplay::Running;
                let (agent_health, model_health, port_open) = tokio::join!(
                    async {
                        if agent_up {
                            health(intelligence::AGENT_PORT).await
                        } else {
                            Probe::NotRun
                        }
                    },
                    async {
                        if model_up {
                            health(intelligence::MODEL_PORT).await
                        } else {
                            Probe::NotRun
                        }
                    },
                    tcp_open(MESHTASTIC_PORT, PORT_TIMEOUT),
                );
                if !agent_up {
                    registry.set_agent_status(None);
                } else if n.is_multiple_of(SLOW_EVERY) {
                    registry.set_agent_status(agent_status().await);
                }
                let now = Instant::now();
                let (agent_pid, model_pid) =
                    (src.intelligence.agent_pid(), src.intelligence.model_pid());
                let rnsd_running = src.transport.child_state() == Some(ChildState::Running);
                let (meshtastic_seen, down_for) =
                    stack.observe(!slow.rnode.is_empty(), rnsd_running, port_open, now);

                let facts = {
                    let d = api.dash.read().unwrap();
                    Facts {
                        phone_like: cfg!(target_os = "android")
                            || d.category.as_deref() == Some("Node2A"),
                        category: d.category.clone(),
                        boot_elapsed: started.elapsed(),
                        agent: ChildFacts {
                            display: intel.agent,
                            pid: agent_pid,
                            since_spawn: agent_spawn.observe(agent_pid, now),
                            health: agent_health,
                            log: files.agent_log.clone(),
                        },
                        agent_binary: files.agent_binary,
                        agent_status: registry.agent_status(),
                        model: ChildFacts {
                            display: intel.model_server,
                            pid: model_pid,
                            since_spawn: model_spawn.observe(model_pid, now),
                            health: model_health,
                            log: files.model_log.clone(),
                        },
                        model_binary: files.model_binary,
                        model_file: files.model_file.clone(),
                        offer: d.offer.clone(),
                        transport_state: d.transport_state,
                        transport_child: ChildDisplay::from(src.transport.child_state()),
                        transport_log: files.transport_log.clone(),
                        python3: slow.python3,
                        radio: RadioFacts {
                            boards: slow.boards.clone(),
                            rnode: slow.rnode.clone(),
                            rnsd_running,
                            meshtastic_open: port_open,
                            meshtastic_seen,
                            down_for,
                        },
                        kb: d.kb.clone(),
                        kb_payload_present: files.kb_payload_present,
                        gateway: gateway.lock().unwrap().clone(),
                        opencode: slow.opencode.clone(),
                    }
                };
                super::tick_and_publish(&api, &registry, &facts, &rfc3339_now());
                n = n.wrapping_add(1);
            }
        });
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_parsing() {
        assert_eq!(
            parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").unwrap(),
            (200, b"ok".to_vec())
        );
        assert_eq!(
            parse_response(b"HTTP/1.0 503 Service Unavailable\r\n\r\n").unwrap(),
            (503, Vec::new())
        );
        assert!(parse_response(b"garbage").is_err());
        assert!(parse_response(b"HTTP/1.1 abc\r\n\r\n").is_err());
    }

    #[test]
    fn spawn_tracker_restarts_grace_on_new_pid() {
        let t0 = Instant::now();
        let mut t = SpawnTracker::default();
        assert_eq!(t.observe(None, t0), None);
        assert_eq!(t.observe(Some(10), t0), Some(Duration::ZERO));
        let t1 = t0 + Duration::from_secs(30);
        assert_eq!(t.observe(Some(10), t1), Some(Duration::from_secs(30)));
        // backoff (no pid) keeps the last spawn time
        assert_eq!(t.observe(None, t1), Some(Duration::from_secs(30)));
        // a new pid restarts the window
        assert_eq!(t.observe(Some(11), t1), Some(Duration::ZERO));
    }

    fn usb(vid: &str, pid: &str, product: Option<&str>) -> UsbInfo {
        UsbInfo {
            vendor_id: vid.into(),
            product_id: pid.into(),
            product: product.map(str::to_owned),
            ..UsbInfo::default()
        }
    }

    fn port(tty: &str, by_id: Option<&str>, u: Option<UsbInfo>) -> detect::SerialPort {
        detect::SerialPort {
            by_id: by_id.map(str::to_owned),
            tty: tty.into(),
            usb: u,
        }
    }

    #[test]
    fn boards_and_rnode_bindings_from_a_scan() {
        let scan = Scan {
            serial: vec![
                port(
                    "ttyUSB0",
                    Some("usb-Silicon_Labs_CP2102_0001-if00-port0"),
                    Some(usb(
                        "10c4",
                        "ea60",
                        Some("CP2102 USB to UART Bridge Controller"),
                    )),
                ),
                // same chip, other product string: not this table row
                port(
                    "ttyUSB1",
                    None,
                    Some(usb("10c4", "ea60", Some("Other bridge"))),
                ),
                // vid-only row (1a86 UART)
                port("ttyACM0", None, Some(usb("1a86", "7523", None))),
                // no USB info (platform UART)
                port("ttyS0", None, None),
            ],
            // ESP32-S3 in bootloader mode: a board without a tty
            usb_other: vec![usb("303a", "1001", None), usb("046d", "c52b", None)],
            bluetooth: vec![],
        };
        let boards = boards_from_scan(&scan, &lora_table());
        let names: Vec<_> = boards
            .iter()
            .map(|b| (b.tty.as_deref(), b.vid.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                (Some("ttyUSB0"), "10c4"),
                (Some("ttyACM0"), "1a86"),
                (None, "303a")
            ]
        );
        assert_eq!(boards[0].name, "Heltec WiFi LoRa 32 V3 (CP2102)");
        assert_eq!(
            boards[0].by_id.as_deref(),
            Some("usb-Silicon_Labs_CP2102_0001-if00-port0")
        );

        let bindings = vec![
            RnodeBinding {
                name: "RNode LoRa".into(),
                port: "/dev/ttyACM0".into(),
                tty: Some("ttyACM0".into()),
            },
            RnodeBinding {
                name: "Gone".into(),
                port: "/dev/ttyACM9".into(),
                tty: Some("ttyACM9".into()),
            },
            RnodeBinding {
                name: "Unresolved".into(),
                port: "/dev/serial/by-id/x".into(),
                tty: None,
            },
            // a configured port that is not in the table still counts
            RnodeBinding {
                name: "Platform".into(),
                port: "/dev/ttyS0".into(),
                tty: Some("ttyS0".into()),
            },
        ];
        let bound = bound_rnodes(&bindings, &scan);
        let ttys: Vec<_> = bound.iter().map(|b| b.tty.as_str()).collect();
        assert_eq!(ttys, ["ttyACM0", "ttyS0"]);
        assert_eq!(bound[0].interface, "RNode LoRa");
        assert_eq!(bound[0].port, "/dev/ttyACM0");
    }

    #[test]
    fn stack_watch_bound_and_down_time() {
        let t0 = Instant::now();
        let s = |n: u64| t0 + Duration::from_secs(n);
        let mut w = StackWatch::default();
        // nothing bound: never down
        assert_eq!(w.observe(false, false, false, s(0)), (false, None));
        // RNode stanza, rnsd up: live
        assert_eq!(w.observe(true, true, false, s(1)), (false, None));
        // rnsd stops: down time counts from here
        assert_eq!(
            w.observe(true, false, false, s(2)),
            (false, Some(Duration::ZERO))
        );
        assert_eq!(
            w.observe(true, false, false, s(70)),
            (false, Some(Duration::from_secs(68)))
        );
        // stanza removed: nothing bound any more
        assert_eq!(w.observe(false, false, false, s(71)), (false, None));
        // meshtasticd answers once, then stops: bound by memory
        assert_eq!(w.observe(false, false, true, s(80)), (true, None));
        assert_eq!(
            w.observe(false, false, false, s(85)),
            (true, Some(Duration::ZERO))
        );
        assert_eq!(
            w.observe(false, false, false, s(150)),
            (true, Some(Duration::from_secs(65)))
        );
        assert_eq!(w.observe(false, false, true, s(151)), (true, None));
    }

    #[test]
    fn which_finds_executables_only() {
        let dir = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        let exe = dir.path().join("tool-x");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let plain = dir.path().join("tool-y");
        std::fs::write(&plain, "data").unwrap();
        let d = dir.path().to_str().unwrap();
        assert_eq!(which("tool-x", &[d]), Some(exe));
        assert_eq!(which("tool-y", &[d]), None);
        assert_eq!(which("tool-z", &[d]), None);
    }

    #[tokio::test]
    async fn loopback_get_and_refused_port() {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            s.readable().await.unwrap();
            let mut buf = [0u8; 512];
            let _ = s.try_read(&mut buf);
            write_all(
                &s,
                b"HTTP/1.0 200 OK\r\n\r\n{\"state\":\"local\",\"model\":{\"id\":\"qwen3-4b\"}}",
            )
            .await
            .unwrap();
        });
        let (code, body) = http_get(port, "/v1/agent/status", HEALTH_TIMEOUT, 4096)
            .await
            .unwrap();
        assert_eq!(code, 200);
        assert!(body.starts_with(b"{\"state\""));
        // closed port: unreachable, quickly
        let closed = {
            let l = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            l.local_addr().unwrap().port()
        };
        assert!(http_get(closed, "/health", HEALTH_TIMEOUT, 64)
            .await
            .is_err());
        assert!(!tcp_open(closed, PORT_TIMEOUT).await);
        assert!(matches!(health(closed).await, Probe::Unreachable(_)));
    }
}
