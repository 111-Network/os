//! meshmud: the binding's uplink, the rendered config and the child spec
//! (Layer 5 §14 "One API client"; §2 ports and state).
//!
//! meshmud holds the board's single API-client slot and fans it out over TCP
//! downlinks. `111d` renders its config from the Meshtastic binding in
//! `state/devices.json`:
//!
//! - **uplink**: a serial path validated against
//!   `^/dev/(serial/by-id/)?[A-Za-z0-9_.:-]+$` (USB boards, Linux only; `.`/`..`
//!   components are refused on top of the pattern), or a parsed `host:port`
//!   (`tcp://<board-ip>:4403` for a Wi-Fi board — the phone path —,
//!   `tcp://127.0.0.1:4403` for meshtasticd driving an SPI/CH341 radio);
//! - **downlink**: exactly one TCP downlink on `127.0.0.1:11230` (loopback; no
//!   HTTP downlink is ever configured — `111d`'s shim serves the panel's HTTP
//!   API, §13/§14), `advertise = false` (no unrequested mDNS);
//! - the file is written atomically with mode 0600 under `<root>/meshtastic/`
//!   (0700), and the child's stdout+stderr go to
//!   `services::probe::meshmud_log` (`<root>/meshtastic/meshmud.log`).
//!
//! Every value that reaches the TOML file has been validated first and the
//! file is produced by the `toml` serializer, never by string templating.

use crate::daemon::supervise::{BackoffPolicy, ChildSpec};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

/// meshmud's TCP downlink (§2: loopback port 11230).
pub const DOWNLINK: SocketAddr = SocketAddr::new(
    IpAddr::V4(Ipv4Addr::LOCALHOST),
    crate::daemon::services::MESHMUD_DOWNLINK_PORT,
);

/// The uplink's id inside the rendered config (one uplink per meshmud).
pub const UPLINK_ID: &str = "board";

/// meshtasticd's TCP API (SPI/CH341 radios, §14).
pub const MESHTASTICD_PORT: u16 = crate::daemon::services::MESHTASTIC_PORT;

/// Longest serial path accepted.
pub const MAX_PATH_LEN: usize = 255;
/// Longest DNS host name accepted (RFC 1035).
pub const MAX_HOST_LEN: usize = 253;

/// Why a binding's uplink was refused (nothing is rendered or started).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UplinkError {
    /// The serial path does not match `^/dev/(serial/by-id/)?[A-Za-z0-9_.:-]+$`
    /// (or names a `.`/`..` component, or is too long).
    BadPath(String),
    /// `host:port` did not parse, or the host is not an IP literal / DNS name.
    BadAddress(String),
    /// The uplink would point meshmud at its own downlink.
    SelfLoop,
}

impl fmt::Display for UplinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UplinkError::BadPath(p) => write!(f, "serial path {p:?} is not an accepted /dev path"),
            UplinkError::BadAddress(a) => write!(f, "{a:?} is not a host:port address"),
            UplinkError::SelfLoop => f.write_str("the uplink would be meshmud's own downlink"),
        }
    }
}

impl std::error::Error for UplinkError {}

/// The board end of meshmud (persisted in the `devices.json` binding).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Uplink {
    /// A USB board's serial port (Linux only; never on a phone).
    Serial { path: String },
    /// A TCP client-API endpoint: a Wi-Fi board or meshtasticd.
    Tcp { host: String, port: u16 },
}

/// `^/dev/(serial/by-id/)?[A-Za-z0-9_.:-]+$`, minus `.`/`..` names.
pub fn valid_serial_path(path: &str) -> bool {
    if path.len() > MAX_PATH_LEN {
        return false;
    }
    let Some(rest) = path.strip_prefix("/dev/") else {
        return false;
    };
    let name = rest.strip_prefix("serial/by-id/").unwrap_or(rest);
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
}

/// An IP literal or an RFC 1123 host name (letters, digits, `-`, dots).
fn valid_host(host: &str) -> bool {
    if host.parse::<IpAddr>().is_ok() {
        return true;
    }
    if host.is_empty() || host.len() > MAX_HOST_LEN {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

impl Uplink {
    /// A validated serial uplink.
    pub fn serial(path: &str) -> Result<Uplink, UplinkError> {
        let u = Uplink::Serial {
            path: path.to_string(),
        };
        u.validate()?;
        Ok(u)
    }

    /// A validated TCP uplink from `host:port`, `tcp://host:port` or
    /// `[v6]:port`.
    pub fn tcp(spec: &str) -> Result<Uplink, UplinkError> {
        let bad = || UplinkError::BadAddress(spec.chars().take(80).collect());
        let s = spec.trim();
        let s = s
            .strip_prefix("tcp://")
            .or_else(|| s.strip_prefix("TCP://"))
            .unwrap_or(s);
        let (host, port) = if let Some(v6) = s.strip_prefix('[') {
            let (h, p) = v6.split_once("]:").ok_or_else(bad)?;
            h.parse::<std::net::Ipv6Addr>().map_err(|_| bad())?;
            (h.to_string(), p)
        } else {
            let (h, p) = s.rsplit_once(':').ok_or_else(bad)?;
            (h.to_ascii_lowercase(), p)
        };
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        let port: u16 = port.parse().map_err(|_| bad())?;
        let u = Uplink::Tcp { host, port };
        u.validate()?;
        Ok(u)
    }

    /// meshtasticd on this node (`tcp://127.0.0.1:4403`).
    pub fn meshtasticd() -> Uplink {
        Uplink::Tcp {
            host: "127.0.0.1".into(),
            port: MESHTASTICD_PORT,
        }
    }

    /// Re-check a (possibly deserialized) uplink: the path pattern, the
    /// host syntax, a non-zero port, and never meshmud's own downlink.
    pub fn validate(&self) -> Result<(), UplinkError> {
        match self {
            Uplink::Serial { path } => {
                if valid_serial_path(path) {
                    Ok(())
                } else {
                    Err(UplinkError::BadPath(path.chars().take(80).collect()))
                }
            }
            Uplink::Tcp { host, port } => {
                if *port == 0 || !valid_host(host) {
                    return Err(UplinkError::BadAddress(format!(
                        "{}:{port}",
                        host.chars().take(80).collect::<String>()
                    )));
                }
                let loopback = host.eq_ignore_ascii_case("localhost")
                    || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
                if loopback && *port == DOWNLINK.port() {
                    return Err(UplinkError::SelfLoop);
                }
                Ok(())
            }
        }
    }

    /// A USB serial board (phones cannot open one, §14).
    pub fn is_usb(&self) -> bool {
        matches!(self, Uplink::Serial { .. })
    }

    /// The address meshmud dials (`host:port`, IPv6 bracketed).
    pub fn address(&self) -> Option<String> {
        match self {
            Uplink::Serial { .. } => None,
            Uplink::Tcp { host, port } if host.contains(':') => Some(format!("[{host}]:{port}")),
            Uplink::Tcp { host, port } => Some(format!("{host}:{port}")),
        }
    }

    /// The port-owner table name of this uplink (`daemon::ports::port_key`
    /// normalizes it further).
    pub fn port_name(&self) -> String {
        match self {
            Uplink::Serial { path } => path.clone(),
            Uplink::Tcp { .. } => self.address().unwrap_or_default(),
        }
    }

    /// Display form (`/dev/…` or `tcp://host:port`).
    pub fn display(&self) -> String {
        match self {
            Uplink::Serial { path } => path.clone(),
            Uplink::Tcp { .. } => format!("tcp://{}", self.address().unwrap_or_default()),
        }
    }
}

// ---------------------------------------------------------------- config

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct SerialUplink {
    id: String,
    device: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct TcpUplink {
    id: String,
    address: String,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Uplinks {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    serial: Vec<SerialUplink>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tcp: Vec<TcpUplink>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct TcpDownlink {
    uplink: String,
    address: String,
    /// Pinned off: meshmud's mDNS announce is an unrequested LAN broadcast.
    advertise: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Downlinks {
    tcp: Vec<TcpDownlink>,
}

/// The whole meshmud config (TOML, `meshmud start-daemon -config <file>`).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Config {
    uplink: Uplinks,
    downlink: Downlinks,
}

/// Render meshmud's config for `uplink` with one TCP downlink on `downlink`
/// (production: [`DOWNLINK`]). The downlink must be a loopback address — an
/// HTTP or LAN downlink is never configured (§14).
pub fn render_config(uplink: &Uplink, downlink: SocketAddr) -> Result<String, UplinkError> {
    uplink.validate()?;
    if !downlink.ip().is_loopback() || downlink.port() == 0 {
        return Err(UplinkError::BadAddress(downlink.to_string()));
    }
    let mut uplinks = Uplinks::default();
    match uplink {
        Uplink::Serial { path } => uplinks.serial.push(SerialUplink {
            id: UPLINK_ID.into(),
            device: path.clone(),
        }),
        Uplink::Tcp { .. } => uplinks.tcp.push(TcpUplink {
            id: UPLINK_ID.into(),
            address: uplink.address().unwrap_or_default(),
        }),
    }
    let cfg = Config {
        uplink: uplinks,
        downlink: Downlinks {
            tcp: vec![TcpDownlink {
                uplink: UPLINK_ID.into(),
                address: downlink.to_string(),
                advertise: false,
            }],
        },
    };
    let body = toml::to_string(&cfg).map_err(|e| UplinkError::BadAddress(e.to_string()))?;
    Ok(format!(
        "# Rendered by 111d from the Meshtastic binding in state/devices.json \
         (os/messaging/README.md §14).\n# Do not edit: it is rewritten before every start.\n{body}"
    ))
}

/// Create `dir` with mode 0700 (and tighten it when it pre-existed wider).
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// Render and write the config atomically, mode 0600, in a 0700 directory.
pub fn write_config(path: &Path, uplink: &Uplink, downlink: SocketAddr) -> anyhow::Result<()> {
    let body = render_config(uplink, downlink)?;
    if let Some(dir) = path.parent() {
        ensure_private_dir(dir)?;
    }
    crate::util::fs::atomic_write(path, body.as_bytes(), 0o600)
}

/// meshmud's restart policy. The first respawn waits 2 s (a serial open can
/// reset the board — DTR/RTS — so a crash never turns into a rapid reopen
/// loop); otherwise the house policy (×2, 60 s cap, 5 exits in 60 s ⇒
/// degraded).
pub fn backoff() -> BackoffPolicy {
    BackoffPolicy {
        initial_ms: 2_000,
        ..BackoffPolicy::default()
    }
}

/// The **identify profile**'s restart policy: the first exit is final
/// (`Degraded` at once, no respawn), so a meshmud that dies during the
/// consented identify step can never reopen the board's port behind the
/// user's back — whatever the timing of the exit. When the identify child
/// becomes the service child in place and later exits, the Meshtastic
/// module swaps it for a child with the service policy ([`backoff`]) on its
/// next tick (the port stays claimed by `meshmud` throughout).
pub fn identify_backoff() -> BackoffPolicy {
    BackoffPolicy {
        crash_loop_threshold: 1,
        ..backoff()
    }
}

/// The meshmud child: `<bin> start-daemon -config <config>`, stdout+stderr
/// appended to `log`.
pub fn spec(bin: &Path, config: &Path, log: &Path, policy: BackoffPolicy) -> ChildSpec {
    ChildSpec {
        name: "meshmud".into(),
        program: bin.to_path_buf(),
        args: vec![
            "start-daemon".into(),
            "-config".into(),
            config.to_string_lossy().into_owned(),
        ],
        env: vec![],
        backoff: policy,
        stdout_log: Some(log.to_path_buf()),
    }
}

/// The presence reader child (§14): `<python> -I <script> --endpoint
/// <downlink> --out <presence dir>`; started after meshmud.
pub fn reader_spec(
    python: &Path,
    script: &Path,
    downlink: SocketAddr,
    out: &Path,
    log: &Path,
) -> ChildSpec {
    ChildSpec {
        name: "meshtastic-presence".into(),
        program: python.to_path_buf(),
        args: vec![
            "-I".into(),
            script.to_string_lossy().into_owned(),
            "--endpoint".into(),
            downlink.to_string(),
            "--out".into(),
            out.to_string_lossy().into_owned(),
        ],
        env: vec![],
        backoff: BackoffPolicy::default(),
        stdout_log: Some(log.to_path_buf()),
    }
}

/// `<root>/meshtastic` (0700): config, logs.
pub fn state_dir(root: &Path) -> PathBuf {
    root.join("meshtastic")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_paths_follow_the_pattern() {
        for ok in [
            "/dev/ttyACM0",
            "/dev/ttyUSB1",
            "/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0",
            "/dev/serial/by-id/usb-Espressif_USB_JTAG_serial_debug_unit_F4:12:FA:00:11:22-if00",
        ] {
            assert!(valid_serial_path(ok), "{ok}");
            assert!(Uplink::serial(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "ttyACM0",
            "/dev/",
            "/dev/..",
            "/dev/.",
            "/dev/serial/by-id/..",
            "/dev/serial/by-id/",
            "/dev/../etc/passwd",
            "/dev/serial/by-path/pci-0000:00:14.0-usb-0:1:1.0-port0",
            "/dev/tty ACM0",
            "/dev/tty\"ACM0",
            "/dev/ttyACM0\n[[downlink.http]]",
            "/dev/tty$(reboot)",
            "/dev/ttyÄ",
            "/tmp/ttyACM0",
            "/dev/serial/by-id/usb-x/../../sda",
        ] {
            assert!(!valid_serial_path(bad), "{bad:?}");
            assert!(
                matches!(Uplink::serial(bad), Err(UplinkError::BadPath(_))),
                "{bad:?}"
            );
        }
        let long = format!("/dev/{}", "a".repeat(MAX_PATH_LEN));
        assert!(!valid_serial_path(&long));
    }

    #[test]
    fn tcp_addresses_parse_and_validate() {
        let u = Uplink::tcp("tcp://10.0.0.7:4403").unwrap();
        assert_eq!(
            u,
            Uplink::Tcp {
                host: "10.0.0.7".into(),
                port: 4403
            }
        );
        assert_eq!(u.address().as_deref(), Some("10.0.0.7:4403"));
        assert_eq!(u.display(), "tcp://10.0.0.7:4403");
        assert!(!u.is_usb());
        assert_eq!(
            Uplink::tcp("Meshtastic-Board.local:4403").unwrap(),
            Uplink::Tcp {
                host: "meshtastic-board.local".into(),
                port: 4403
            }
        );
        let v6 = Uplink::tcp("[fe80::1]:4403").unwrap();
        assert_eq!(v6.address().as_deref(), Some("[fe80::1]:4403"));
        assert_eq!(
            Uplink::meshtasticd().address().as_deref(),
            Some("127.0.0.1:4403")
        );
        for bad in [
            "",
            "10.0.0.7",
            "10.0.0.7:",
            "10.0.0.7:0",
            "10.0.0.7:65536",
            "10.0.0.7:+1",
            "-bad.host:4403",
            "a..b:4403",
            "host_name:4403",
            "ho\"st:4403",
            "host:4403\n[[downlink.http]]",
            "[fe80::1:4403",
            "[nothex]:4403",
            "http://10.0.0.7:4403",
        ] {
            assert!(Uplink::tcp(bad).is_err(), "{bad:?}");
        }
        // meshmud must never dial its own downlink
        for own in ["127.0.0.1:11230", "localhost:11230", "[::1]:11230"] {
            assert_eq!(Uplink::tcp(own), Err(UplinkError::SelfLoop), "{own}");
        }
    }

    #[test]
    fn deserialized_uplinks_are_revalidated() {
        let bad: Uplink =
            serde_json::from_str(r#"{"kind":"serial","path":"/dev/../../etc/shadow"}"#).unwrap();
        assert!(bad.validate().is_err());
        assert!(
            serde_json::from_str::<Uplink>(r#"{"kind":"serial","path":"/dev/x","y":1}"#).is_err()
        );
        assert!(serde_json::from_str::<Uplink>(r#"{"kind":"http","url":"x"}"#).is_err());
        let round = Uplink::serial("/dev/ttyACM0").unwrap();
        let back: Uplink = serde_json::from_str(&serde_json::to_string(&round).unwrap()).unwrap();
        assert_eq!(back, round);
    }

    #[test]
    fn config_renders_one_loopback_tcp_downlink_without_mdns() {
        let serial = Uplink::serial("/dev/serial/by-id/usb-Heltec-if00").unwrap();
        let text = render_config(&serial, DOWNLINK).unwrap();
        let v: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(v["uplink"]["serial"][0]["id"].as_str(), Some(UPLINK_ID));
        assert_eq!(
            v["uplink"]["serial"][0]["device"].as_str(),
            Some("/dev/serial/by-id/usb-Heltec-if00")
        );
        assert!(v["uplink"].get("tcp").is_none());
        let dl = v["downlink"]["tcp"].as_array().unwrap();
        assert_eq!(dl.len(), 1);
        assert_eq!(dl[0]["address"].as_str(), Some("127.0.0.1:11230"));
        assert_eq!(dl[0]["uplink"].as_str(), Some(UPLINK_ID));
        assert_eq!(dl[0]["advertise"].as_bool(), Some(false));
        assert!(
            v["downlink"].get("http").is_none(),
            "no HTTP downlink, ever"
        );
        // the typed form round-trips (deny_unknown_fields)
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.downlink.tcp.len(), 1);

        let wifi = Uplink::tcp("10.0.0.7:4403").unwrap();
        let v: toml::Value = toml::from_str(&render_config(&wifi, DOWNLINK).unwrap()).unwrap();
        assert_eq!(
            v["uplink"]["tcp"][0]["address"].as_str(),
            Some("10.0.0.7:4403")
        );
        assert!(v["uplink"].get("serial").is_none());

        // a LAN or unspecified downlink is refused
        for dl in ["0.0.0.0:11230", "192.168.1.5:11230", "127.0.0.1:0"] {
            assert!(render_config(&wifi, dl.parse().unwrap()).is_err(), "{dl}");
        }
        // an invalid uplink never renders
        let evil = Uplink::Serial {
            path: "/dev/x\"\n[[downlink.http]]\naddress=\"0.0.0.0:80".into(),
        };
        assert!(render_config(&evil, DOWNLINK).is_err());
    }

    #[test]
    fn config_file_is_0600_in_a_0700_dir() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = state_dir(dir.path()).join("meshmud.toml");
        write_config(&path, &Uplink::serial("/dev/ttyACM0").unwrap(), DOWNLINK).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        let bad = Uplink::Serial {
            path: "/etc/passwd".into(),
        };
        assert!(write_config(&path, &bad, DOWNLINK).is_err());
        // the previous good file is untouched by the refused render
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("/dev/ttyACM0"));
    }

    #[test]
    fn child_specs() {
        let s = spec(
            Path::new("/r/current/bin/meshmud"),
            Path::new("/r/meshtastic/meshmud.toml"),
            Path::new("/r/meshtastic/meshmud.log"),
            backoff(),
        );
        assert_eq!(s.name, "meshmud");
        assert_eq!(
            s.args,
            ["start-daemon", "-config", "/r/meshtastic/meshmud.toml"]
        );
        assert_eq!(s.backoff.initial_ms, 2_000);
        // the identify profile never respawns: the first exit is final
        let id = identify_backoff();
        assert_eq!(id.crash_loop_threshold, 1);
        let mut sched = crate::daemon::supervise::BackoffSchedule::new(id, 7);
        assert_eq!(
            sched.record_exit(std::time::Instant::now()),
            crate::daemon::supervise::Decision::Degraded
        );
        let r = reader_spec(
            Path::new("/r/venvs/presence/bin/python"),
            Path::new("/r/current/share/meshtastic-presence/meshtastic_presence.py"),
            DOWNLINK,
            Path::new("/r/fabric/presence"),
            Path::new("/r/meshtastic/presence.log"),
        );
        assert_eq!(r.args[0], "-I");
        assert_eq!(r.args[2..4], ["--endpoint", "127.0.0.1:11230"]);
        assert_eq!(r.args[4..], ["--out", "/r/fabric/presence"]);
    }
}
