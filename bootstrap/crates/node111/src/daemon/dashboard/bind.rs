//! Dashboard listeners + the URL list (spec §1 "Hostnames", §11, §12; D15).
//!
//! - `:11180` always (loopback, or `0.0.0.0` when LAN sharing is on); `:80` on
//!   the same address best-effort (needs `CAP_NET_BIND_SERVICE` — present in the
//!   systemd unit, stripped in degraded containers, impossible on Termux). A
//!   failed `:80` bind is logged once and never fails startup.
//! - Loopback listeners are IPv4 `127.0.0.1` plus IPv6 `[::1]` (best effort,
//!   silently skipped when IPv6 is off): `111.localhost` resolves to `::1` on
//!   many hosts. The LAN listener is IPv4 `0.0.0.0` only.
//! - LAN sharing has ONE source of truth: `state/settings.json` `lan_enabled`
//!   (written by the dashboard's `PATCH /api/v1/settings` and by
//!   `111 config dashboard.lan`). `DASHBOARD_LAN` in the env file, when present,
//!   is an explicit override (`true` = on, any other value = off) and *locks*
//!   the setting (`lan_locked`, PATCH → 409). Read once per daemon start.
//! - The URLs that actually bound, nicest first, go into `ApiState.bind` and
//!   `<root>/state/dashboard.json` (`{schema:1, urls, lan, pid, pid_start,
//!   started_at}`) for `111 dashboard` and the installer's success print.
//!   `pid_start` (`/proc/<pid>/stat` field 22) lets readers tell a live daemon
//!   from a recycled pid after a crash ([`daemon_alive`]).

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;

pub const STATE_FILE: &str = "dashboard.json";
pub const SETTINGS_FILE: &str = "settings.json";
/// The constant fallback URL (also the install-time health base).
pub const FALLBACK_URL: &str = "http://127.0.0.1:11180";

/// URLs that bound + whether the listeners are LAN-wide (served in `/status.bind`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindInfo {
    pub urls: Vec<String>,
    pub lan: bool,
}

/// `<root>/state/dashboard.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardFile {
    pub schema: u32,
    pub urls: Vec<String>,
    pub lan: bool,
    pub pid: u32,
    /// Start time of `pid` in clock ticks since boot (`/proc/<pid>/stat` field
    /// 22). Absent in files written before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid_start: Option<u64>,
    pub started_at: String,
}

#[derive(Deserialize)]
struct SettingsLan {
    #[serde(default)]
    lan_enabled: bool,
}

/// `state/settings.json` `lan_enabled` (absent/unreadable/garbled = false).
pub fn settings_lan_enabled(state_dir: &Path) -> bool {
    std::fs::read(state_dir.join(SETTINGS_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<SettingsLan>(&b).ok())
        .is_some_and(|s| s.lan_enabled)
}

/// The env-file key that overrides (and locks) `settings.json` `lan_enabled`.
pub const LAN_ENV_KEY: &str = "DASHBOARD_LAN";

/// `DASHBOARD_LAN` from the env file: `Some(value == "true")` when present with
/// a non-empty value (last line wins, like sourcing), `None` when absent.
pub fn lan_env_override(env_pairs: &[(String, String)]) -> Option<bool> {
    env_pairs
        .iter()
        .rev()
        .find(|(k, v)| k == LAN_ENV_KEY && !v.is_empty())
        .map(|(_, v)| v == "true")
}

/// LAN sharing as configured: the effective value and whether the env file pins it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LanConfig {
    pub enabled: bool,
    /// `DASHBOARD_LAN` is present in the env file and overrides `settings.json`.
    pub locked: bool,
}

/// Effective LAN sharing: the env-file `DASHBOARD_LAN` override when present
/// (locked), else `state/settings.json` `lan_enabled`.
pub fn lan_config(env_pairs: &[(String, String)], state_dir: &Path) -> LanConfig {
    match lan_env_override(env_pairs) {
        Some(enabled) => LanConfig {
            enabled,
            locked: true,
        },
        None => LanConfig {
            enabled: settings_lan_enabled(state_dir),
            locked: false,
        },
    }
}

/// Effective LAN sharing for a daemon start (see [`lan_config`]).
pub fn lan_configured(env_pairs: &[(String, String)], state_dir: &Path) -> bool {
    lan_config(env_pairs, state_dir).enabled
}

/// Bind one listener; refuses a non-loopback address without the LAN opt-in.
pub async fn bind_listener(
    addr: SocketAddr,
    lan_opt_in: bool,
) -> anyhow::Result<tokio::net::TcpListener> {
    anyhow::ensure!(
        lan_opt_in || addr.ip().is_loopback(),
        "dashboard refuses non-loopback bind {addr} without explicit LAN opt-in (111 config dashboard.lan true)"
    );
    Ok(tokio::net::TcpListener::bind(addr).await?)
}

/// What [`bind_all`] got.
pub struct Bound {
    pub listeners: Vec<tokio::net::TcpListener>,
    pub main_ok: bool,
    pub port80_ok: bool,
}

/// Bind `ip:port` (required) and `ip:80` (best effort). Failures are logged.
/// The same ports are also bound on `[::1]` (best effort, see [`bind_v6_loopback`]).
pub async fn bind_all(lan: bool, port: u16) -> Bound {
    let ip: IpAddr = if lan {
        Ipv4Addr::UNSPECIFIED.into()
    } else {
        Ipv4Addr::LOCALHOST.into()
    };
    let mut listeners = Vec::new();
    let main_ok = match bind_listener(SocketAddr::new(ip, port), lan).await {
        Ok(l) => {
            listeners.push(l);
            true
        }
        Err(e) => {
            eprintln!("111 daemon: dashboard bind {ip}:{port} failed: {e:#}");
            false
        }
    };
    let port80_ok = port != 80
        && match bind_listener(SocketAddr::new(ip, 80), lan).await {
            Ok(l) => {
                listeners.push(l);
                true
            }
            Err(e) => {
                eprintln!("111 daemon: dashboard :80 not available ({e}); serving on :{port} only");
                false
            }
        };
    let mut v6_ports = vec![port];
    if port80_ok {
        v6_ports.push(80);
    }
    for p in v6_ports {
        if let Some(l) = bind_v6_loopback(p).await {
            listeners.push(l);
        }
    }
    Bound {
        listeners,
        main_ok,
        port80_ok,
    }
}

/// `[::1]:port`, best effort: silent when IPv6 is disabled (no `::1`, or no
/// AF_INET6 at all), one log line for anything else (e.g. port taken).
pub async fn bind_v6_loopback(port: u16) -> Option<tokio::net::TcpListener> {
    let addr = SocketAddr::new(Ipv6Addr::LOCALHOST.into(), port);
    match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => Some(l),
        Err(e) => {
            let ipv6_off = matches!(
                e.raw_os_error(),
                Some(libc::EADDRNOTAVAIL) | Some(libc::EAFNOSUPPORT) | Some(libc::EPROTONOSUPPORT)
            );
            if !ipv6_off {
                eprintln!("111 daemon: dashboard bind {addr} failed ({e}); IPv4 loopback only");
            }
            None
        }
    }
}

/// The URL list, nicest first. `mdns_host` is the announced name (`111.local`,
/// or `111-N.local` after a conflict rename) when the announce is live.
pub fn urls(
    main_ok: bool,
    port80_ok: bool,
    port: u16,
    lan: bool,
    mdns_host: Option<&str>,
    lan_ips: &[Ipv4Addr],
) -> Vec<String> {
    let mut out = Vec::new();
    if port80_ok {
        out.push("http://111.localhost".to_string());
    }
    if main_ok {
        out.push(format!("http://127.0.0.1:{port}"));
    } else if port80_ok {
        out.push("http://127.0.0.1".to_string());
    }
    if lan && (main_ok || port80_ok) {
        let suffix = if port80_ok {
            String::new()
        } else {
            format!(":{port}")
        };
        if let Some(h) = mdns_host {
            out.push(format!("http://{h}{suffix}"));
        }
        for ip in lan_ips {
            out.push(format!("http://{ip}{suffix}"));
        }
    }
    out
}

/// Write `<state>/dashboard.json` (0644: no secrets in it).
pub fn write_state(state_dir: &Path, info: &BindInfo, started_at: &str) -> anyhow::Result<()> {
    let file = DashboardFile {
        schema: 1,
        urls: info.urls.clone(),
        lan: info.lan,
        pid: std::process::id(),
        pid_start: proc_start_time(std::process::id()),
        started_at: started_at.to_string(),
    };
    crate::util::fs::atomic_write(
        &state_dir.join(STATE_FILE),
        &serde_json::to_vec_pretty(&file)?,
        0o644,
    )
}

/// Read `<state>/dashboard.json` (None when absent or garbled).
pub fn read_state(state_dir: &Path) -> Option<DashboardFile> {
    let bytes = std::fs::read(state_dir.join(STATE_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Is `pid` a live process (EPERM counts as alive)?
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    let r = unsafe { libc::kill(pid as i32, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Field 22 (`starttime`, clock ticks since boot) of a `/proc/<pid>/stat` line.
/// `comm` (field 2) may hold spaces and parentheses, so fields are counted
/// after the LAST `)`.
pub fn parse_stat_start_time(stat: &str) -> Option<u64> {
    let (_, rest) = stat.rsplit_once(')')?;
    // rest starts at field 3 (state) → field 22 is index 22 - 3.
    rest.split_whitespace().nth(22 - 3)?.parse().ok()
}

/// Start time of a live `pid` (None when gone or `/proc` hides it).
pub fn proc_start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_stat_start_time(&stat)
}

/// Is the daemon that wrote `file` still running? The pid must be alive AND,
/// when the file recorded a start time and `/proc` shows one, the two must
/// match — a crashed daemon's pid recycled by an unrelated process is not
/// mistaken for a running dashboard. Older files (no `pid_start`) and a hidden
/// `/proc` fall back to the bare pid check.
pub fn daemon_alive(file: &DashboardFile) -> bool {
    if !pid_alive(file.pid) {
        return false;
    }
    match (file.pid_start, proc_start_time(file.pid)) {
        (Some(recorded), Some(live)) => recorded == live,
        _ => true,
    }
}

/// One interface address from `getifaddrs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfAddr {
    pub name: String,
    pub ip: IpAddr,
}

/// Every address on every interface (`getifaddrs`; empty on failure).
pub fn interface_addrs() -> Vec<IfAddr> {
    let mut out = Vec::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return out;
    }
    let mut cur = head;
    while !cur.is_null() {
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_addr.is_null() {
            continue;
        }
        let name = if ifa.ifa_name.is_null() {
            String::new()
        } else {
            unsafe { std::ffi::CStr::from_ptr(ifa.ifa_name) }
                .to_string_lossy()
                .into_owned()
        };
        let family = unsafe { (*ifa.ifa_addr).sa_family } as i32;
        let ip = match family {
            libc::AF_INET => {
                let sa = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
                IpAddr::V4(Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr)))
            }
            libc::AF_INET6 => {
                let sa = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in6) };
                IpAddr::V6(std::net::Ipv6Addr::from(sa.sin6_addr.s6_addr))
            }
            _ => continue,
        };
        out.push(IfAddr { name, ip });
    }
    unsafe { libc::freeifaddrs(head) };
    out
}

/// The node's own addresses (Host allowlist when LAN is on).
pub fn own_addresses() -> Vec<IpAddr> {
    let mut v: Vec<IpAddr> = interface_addrs().into_iter().map(|a| a.ip).collect();
    v.sort();
    v.dedup();
    v
}

/// Container/VM bridge interfaces never worth printing as a LAN URL.
fn virtual_iface(name: &str) -> bool {
    [
        "docker", "veth", "br-", "virbr", "lxcbr", "lxdbr", "cni", "flannel", "podman",
    ]
    .iter()
    .any(|p| name.starts_with(p))
}

/// LAN IPv4s to print (non-loopback, non-link-local, no container bridges).
pub fn lan_ipv4s() -> Vec<Ipv4Addr> {
    let mut v: Vec<Ipv4Addr> = interface_addrs()
        .into_iter()
        .filter(|a| !virtual_iface(&a.name))
        .filter_map(|a| match a.ip {
            IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() => {
                Some(ip)
            }
            _ => None,
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_list_nicest_first() {
        let ips = ["192.168.1.5".parse().unwrap()];
        assert_eq!(
            urls(true, true, 11180, false, None, &ips),
            ["http://111.localhost", "http://127.0.0.1:11180"]
        );
        assert_eq!(
            urls(true, false, 11180, false, None, &ips),
            ["http://127.0.0.1:11180"]
        );
        assert_eq!(
            urls(true, true, 11180, true, Some("111.local"), &ips),
            [
                "http://111.localhost",
                "http://127.0.0.1:11180",
                "http://111.local",
                "http://192.168.1.5"
            ]
        );
        assert_eq!(
            urls(true, false, 11180, true, Some("111-2.local"), &ips),
            [
                "http://127.0.0.1:11180",
                "http://111-2.local:11180",
                "http://192.168.1.5:11180"
            ]
        );
        assert!(urls(false, false, 11180, true, None, &ips).is_empty());
    }

    fn env(v: &str) -> Vec<(String, String)> {
        vec![(LAN_ENV_KEY.to_string(), v.to_string())]
    }

    fn settings(dir: &Path, lan: bool) {
        std::fs::write(
            dir.join(SETTINGS_FILE),
            format!(r#"{{"schema":1,"lan_enabled":{lan},"alias":null}}"#),
        )
        .unwrap();
    }

    #[test]
    fn lan_config_settings_is_the_source_env_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let off = LanConfig {
            enabled: false,
            locked: false,
        };
        // env absent: settings.json decides (absent/garbled = off)
        assert_eq!(lan_config(&[], dir.path()), off);
        settings(dir.path(), true);
        assert_eq!(
            lan_config(&[], dir.path()),
            LanConfig {
                enabled: true,
                locked: false
            }
        );
        settings(dir.path(), false);
        assert_eq!(lan_config(&[], dir.path()), off);
        std::fs::write(dir.path().join(SETTINGS_FILE), "garbage").unwrap();
        assert_eq!(lan_config(&[], dir.path()), off);

        // env present: overrides settings in BOTH directions and locks
        settings(dir.path(), true);
        assert_eq!(
            lan_config(&env("false"), dir.path()),
            LanConfig {
                enabled: false,
                locked: true
            }
        );
        assert!(!lan_configured(&env("false"), dir.path()));
        settings(dir.path(), false);
        assert_eq!(
            lan_config(&env("true"), dir.path()),
            LanConfig {
                enabled: true,
                locked: true
            }
        );
        assert!(lan_configured(&env("true"), dir.path()));
        // any non-"true" value pins off; an empty value counts as absent;
        // the last line wins
        assert_eq!(lan_env_override(&env("yes")), Some(false));
        assert_eq!(lan_env_override(&env("")), None);
        let mut two = env("true");
        two.extend(env("false"));
        assert_eq!(lan_env_override(&two), Some(false));
        assert_eq!(lan_env_override(&[("OTHER".into(), "true".into())]), None);
    }

    #[test]
    fn stat_start_time_parsing() {
        // comm with spaces and parentheses; field 22 = 987654
        let stat = "4242 (111 (d) x) S 1 4242 4242 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 9 0 987654 1000 200 18446744073709551615";
        assert_eq!(parse_stat_start_time(stat), Some(987654));
        assert_eq!(parse_stat_start_time("garbage"), None);
        assert_eq!(parse_stat_start_time("1 (x) S 1 2"), None);
        let own = proc_start_time(std::process::id());
        assert!(own.is_some(), "/proc/self/stat must parse");
    }

    #[test]
    fn stale_dashboard_file_is_not_a_running_daemon() {
        let dir = tempfile::tempdir().unwrap();
        let info = BindInfo {
            urls: vec!["http://127.0.0.1:11180".into()],
            lan: false,
        };
        write_state(dir.path(), &info, "2026-10-14T00:00:00Z").unwrap();
        let f = read_state(dir.path()).unwrap();
        assert_eq!(f.pid_start, proc_start_time(std::process::id()));
        assert!(daemon_alive(&f), "the writer itself is alive");

        // pid recycled by another process: same pid, different start time
        let recycled = DashboardFile {
            pid_start: f.pid_start.map(|t| t + 1),
            ..f.clone()
        };
        assert!(!daemon_alive(&recycled));

        // the writer exited: its pid is gone
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead_pid = child.id();
        child.wait().unwrap();
        let dead = DashboardFile {
            pid: dead_pid,
            pid_start: Some(1),
            ..f.clone()
        };
        assert!(!daemon_alive(&dead));

        // a file from before pid_start existed: bare pid check
        let legacy: DashboardFile = serde_json::from_str(&format!(
            r#"{{"schema":1,"urls":[],"lan":false,"pid":{},"started_at":"x"}}"#,
            std::process::id()
        ))
        .unwrap();
        assert_eq!(legacy.pid_start, None);
        assert!(daemon_alive(&legacy));
    }

    #[tokio::test]
    async fn loopback_binds_ipv6_too_when_available() {
        // a free port: bind :0 on v4, release it, reuse the number
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let v6_available = std::net::TcpListener::bind("[::1]:0").is_ok();
        let bound = bind_all(false, port).await;
        assert!(bound.main_ok);
        let v6: Vec<_> = bound
            .listeners
            .iter()
            .filter_map(|l| l.local_addr().ok())
            .filter(|a| a.is_ipv6())
            .collect();
        if v6_available {
            assert!(
                v6.iter().any(|a| a.port() == port && a.ip().is_loopback()),
                "{v6:?}"
            );
        } else {
            assert!(v6.is_empty());
        }
        // never a non-loopback v6 address
        assert!(v6.iter().all(|a| a.ip().is_loopback()));
    }

    #[test]
    fn interfaces_include_loopback() {
        assert!(own_addresses().iter().any(|ip| ip.is_loopback()));
    }

    #[test]
    fn state_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let info = BindInfo {
            urls: vec!["http://127.0.0.1:11180".into()],
            lan: false,
        };
        write_state(dir.path(), &info, "2026-10-13T00:00:00Z").unwrap();
        let f = read_state(dir.path()).unwrap();
        assert_eq!(f.urls, info.urls);
        assert_eq!(f.pid, std::process::id());
        assert!(pid_alive(f.pid));
        assert!(daemon_alive(&f));
    }
}
