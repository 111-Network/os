//! Dashboard listeners + the URL list (spec §1 "Hostnames", §11, §12; D15).
//!
//! - `:11180` always (loopback, or `0.0.0.0` when LAN sharing is on); `:80` on
//!   the same address best-effort (needs `CAP_NET_BIND_SERVICE` — present in the
//!   systemd unit, stripped in degraded containers, impossible on Termux). A
//!   failed `:80` bind is logged once and never fails startup.
//! - LAN sharing = env `DASHBOARD_LAN=true` OR `state/settings.json`
//!   `lan_enabled: true` (R2 writes that file; it is only read here), read once
//!   per daemon start.
//! - The URLs that actually bound, nicest first, go into `ApiState.bind` and
//!   `<root>/state/dashboard.json` (`{schema:1, urls, lan, pid, started_at}`) for
//!   `111 dashboard` and the installer's success print.

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
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

/// Effective LAN sharing for a daemon start: env file `DASHBOARD_LAN=true` OR
/// settings `lan_enabled`.
pub fn lan_configured(env_pairs: &[(String, String)], state_dir: &Path) -> bool {
    env_pairs
        .iter()
        .any(|(k, v)| k == "DASHBOARD_LAN" && v == "true")
        || settings_lan_enabled(state_dir)
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
    Bound {
        listeners,
        main_ok,
        port80_ok,
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

    #[test]
    fn lan_config_sources() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!lan_configured(&[], dir.path()));
        assert!(lan_configured(
            &[("DASHBOARD_LAN".into(), "true".into())],
            dir.path()
        ));
        std::fs::write(
            dir.path().join(SETTINGS_FILE),
            r#"{"schema":1,"lan_enabled":true,"alias":null}"#,
        )
        .unwrap();
        assert!(lan_configured(&[], dir.path()));
        std::fs::write(dir.path().join(SETTINGS_FILE), "garbage").unwrap();
        assert!(!lan_configured(&[], dir.path()));
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
    }
}
