//! Hardware/environment facts. Every fact degrades to "unknown" (None) — probing
//! must never hard-fail an install on a weird box.
//!
//! All reads go through the `Env` trait (fixture-injectable); the production
//! env is `SystemEnv`. LoRa USB probing reads sysfs (`/sys/bus/usb/devices/*`
//! idVendor/idProduct/product) — the same VID:PID + product-string data the
//! spec table pins, testable through `Env`. (The spec's parenthetical "nusb"
//! was the research-suggested mechanism; sysfs carries identical data. The
//! nusb dep remains for Layer 6 radio work.)

use serde::{Deserialize, Serialize};
use std::path::Path;

/// I/O the probe needs, injectable for fixture-tree tests.
pub trait Env {
    fn read_file(&self, path: &str) -> std::io::Result<String>;
    fn read_dir(&self, path: &str) -> std::io::Result<Vec<String>>;
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<String>;
}

/// Production env: real filesystem + process execution.
pub struct SystemEnv;

impl Env for SystemEnv {
    fn read_file(&self, path: &str) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }
    fn read_dir(&self, path: &str) -> std::io::Result<Vec<String>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(path)? {
            out.push(entry?.file_name().to_string_lossy().into_owned());
        }
        Ok(out)
    }
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<String> {
        let out = std::process::Command::new(program).args(args).output()?;
        if !out.status.success() {
            return Err(std::io::Error::other(format!(
                "{program} exited {}",
                out.status
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoraDevice {
    pub vid: String,
    pub pid: String,
    pub product: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Gpu {
    /// PCI vendor id, lowercase hex without 0x (e.g. "10de", "1002", "8086").
    pub vendor: String,
    /// PCI class, lowercase hex without 0x (0300/0302/0380 family).
    pub class: String,
}

impl Gpu {
    /// Discrete GPU per spec rule 1: NVIDIA or AMD (Intel 8086 = integrated).
    pub fn is_discrete(&self) -> bool {
        matches!(self.vendor.as_str(), "10de" | "1002")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Facts {
    pub arch: Option<String>,
    pub cores: Option<u32>,
    pub ram_bytes: Option<u64>,
    /// Free bytes on the filesystem that holds (or will hold) /var/lib/111.
    pub free_storage_bytes: Option<u64>,
    /// Distro family: "debian" | "fedora" | "arch" | other ID.
    pub distro: Option<String>,
    /// /proc/1/comm.
    pub init: Option<String>,
    pub systemd: Option<bool>,
    pub in_container: Option<bool>,
    pub is_wsl: Option<bool>,
    pub battery: Option<bool>,
    pub wifi_ap_capable: Option<bool>,
    pub ethernet: Option<bool>,
    pub ble: Option<bool>,
    pub lora: Option<Vec<LoraDevice>>,
    pub gpu: Option<Vec<Gpu>>,
    /// /proc/device-tree/model on Pi.
    pub pi_model: Option<String>,
    /// Set by the installer's connectivity step, not by `collect`.
    pub internet: Option<bool>,
    /// True when probing through the phone flow (`--phone`).
    pub is_phone: bool,
}

// ---------- LoRa radio table (embedded; ships inside the signed release) ----------

#[derive(Debug, Clone)]
pub struct RadioRule {
    pub vid: String,
    /// "*" = any pid.
    pub pid: String,
    /// When Some, the sysfs product string must contain it (case-insensitive) —
    /// VID:PID AND product-string per spec (generic UART bridges over-match
    /// otherwise).
    pub product_contains: Option<String>,
    pub name: String,
}

const LORA_TABLE_TOML: &str = include_str!("../../assets/lora-table.toml");

#[derive(Deserialize)]
struct RawTable {
    radio: Vec<RawRadio>,
}

#[derive(Deserialize)]
struct RawRadio {
    vid: String,
    pid: String,
    product_contains: Option<String>,
    name: String,
}

/// Parse the embedded LoRa table. Panics only on a build-time-embedded corrupt
/// table (covered by unit test, so a panic means a broken build, not bad input).
pub fn lora_table() -> Vec<RadioRule> {
    let raw: RawTable =
        toml::from_str(LORA_TABLE_TOML).expect("embedded lora-table.toml is valid (unit-tested)");
    raw.radio
        .into_iter()
        .map(|r| RadioRule {
            vid: r.vid.to_lowercase(),
            pid: r.pid.to_lowercase(),
            product_contains: r.product_contains.map(|p| p.to_lowercase()),
            name: r.name,
        })
        .collect()
}

// ---------- collection ----------

fn ok<T>(r: std::io::Result<T>) -> Option<T> {
    r.ok()
}

fn read_trimmed(env: &dyn Env, path: &str) -> Option<String> {
    ok(env.read_file(path))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn probe_arch(env: &dyn Env) -> Option<String> {
    let out = ok(env.run("uname", &["-m"]))?;
    let a = out.trim();
    let norm = match a {
        "amd64" => "x86_64",
        "arm64" => "aarch64",
        other => other,
    };
    Some(norm.to_string())
}

fn probe_cores(env: &dyn Env) -> Option<u32> {
    let cpuinfo = ok(env.read_file("/proc/cpuinfo"))?;
    let n = cpuinfo
        .lines()
        .filter(|l| {
            l.split(':')
                .next()
                .map(|k| k.trim() == "processor")
                .unwrap_or(false)
        })
        .count() as u32;
    if n > 0 {
        Some(n)
    } else {
        None
    }
}

fn probe_ram(env: &dyn Env) -> Option<u64> {
    let meminfo = ok(env.read_file("/proc/meminfo"))?;
    for line in meminfo.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb: u64 = rest
                .trim()
                .strip_suffix("kB")
                .unwrap_or(rest.trim())
                .trim()
                .parse()
                .ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

/// Free bytes on the filesystem holding `path` (statvfs; not Env-injected —
/// tests set `Facts::free_storage_bytes` directly).
pub fn free_bytes(path: &Path) -> Option<u64> {
    let c_path = std::ffi::CString::new(path.to_string_lossy().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if rc != 0 {
        return None;
    }
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}

fn probe_distro(env: &dyn Env) -> Option<String> {
    let osr = ok(env.read_file("/etc/os-release"))?;
    let mut id = String::new();
    let mut id_like = String::new();
    for line in osr.lines() {
        if let Some(v) = line.strip_prefix("ID=") {
            id = v.trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("ID_LIKE=") {
            id_like = v.trim_matches('"').to_string();
        }
    }
    let hay = format!("{id} {id_like}");
    let family = if hay.contains("debian") || hay.contains("ubuntu") {
        "debian"
    } else if hay.contains("fedora") || hay.contains("rhel") || hay.contains("centos") {
        "fedora"
    } else if hay.contains("arch") {
        "arch"
    } else if id.is_empty() {
        return None;
    } else {
        // Unknown family: keep the raw ID (installer refuses politely later).
        return Some(id);
    };
    Some(family.to_string())
}

fn probe_container(env: &dyn Env) -> Option<bool> {
    // systemd-detect-virt is authoritative when present.
    if let Some(out) = ok(env.run("systemd-detect-virt", &["--container"])) {
        let v = out.trim();
        return Some(v != "none");
    }
    if ok(env.read_file("/.dockerenv")).is_some()
        || ok(env.read_file("/run/.containerenv")).is_some()
    {
        return Some(true);
    }
    if let Some(cgroup) = ok(env.read_file("/proc/1/cgroup")) {
        let l = cgroup.to_lowercase();
        if l.contains("docker")
            || l.contains("kubepods")
            || l.contains("containerd")
            || l.contains("lxc")
        {
            return Some(true);
        }
    }
    None // unknown, not "no"
}

fn probe_wsl(env: &dyn Env) -> Option<bool> {
    let version = ok(env.read_file("/proc/version"))?.to_lowercase();
    Some(version.contains("microsoft") || version.contains("wsl"))
}

fn probe_battery(env: &dyn Env) -> Option<bool> {
    let entries = ok(env.read_dir("/sys/class/power_supply"))?;
    Some(entries.iter().any(|e| e.starts_with("BAT")))
}

fn probe_wifi_ap(env: &dyn Env) -> Option<bool> {
    // iw is installed at install step 5; absent ⇒ unknown (None).
    let out = ok(env.run("iw", &["phy"]))?;
    // "Supported interface modes:" block contains "* AP"
    Some(out.lines().any(|l| l.trim() == "* AP"))
}

fn probe_ethernet(env: &dyn Env) -> Option<bool> {
    let ifaces = ok(env.read_dir("/sys/class/net"))?;
    for iface in ifaces {
        if iface == "lo" || iface.starts_with("wl") {
            continue;
        }
        // Real (non-virtual) NIC: has a device symlink; not wireless (no
        // /sys/class/net/<iface>/wireless).
        if ok(env.read_file(&format!("/sys/class/net/{iface}/device/uevent"))).is_some()
            && ok(env.read_file(&format!("/sys/class/net/{iface}/wireless/uevent"))).is_none()
        {
            return Some(true);
        }
    }
    Some(false)
}

fn probe_ble(env: &dyn Env) -> Option<bool> {
    let entries = ok(env.read_dir("/sys/class/bluetooth"))?;
    Some(entries.iter().any(|e| e.starts_with("hci")))
}

fn probe_lora(env: &dyn Env) -> Option<Vec<LoraDevice>> {
    let entries = ok(env.read_dir("/sys/bus/usb/devices"))?;
    let rules = lora_table();
    let mut found = Vec::new();
    for entry in entries {
        let base = format!("/sys/bus/usb/devices/{entry}");
        let (Some(vid), Some(pid)) = (
            read_trimmed(env, &format!("{base}/idVendor")),
            read_trimmed(env, &format!("{base}/idProduct")),
        ) else {
            continue;
        };
        let product = read_trimmed(env, &format!("{base}/product")).unwrap_or_default();
        for rule in &rules {
            if rule.vid != vid.to_lowercase() {
                continue;
            }
            if rule.pid != "*" && rule.pid != pid.to_lowercase() {
                continue;
            }
            if let Some(want) = &rule.product_contains {
                if !product.to_lowercase().contains(want.as_str()) {
                    continue;
                }
            }
            found.push(LoraDevice {
                vid: vid.clone(),
                pid: pid.clone(),
                product: product.clone(),
            });
        }
    }
    Some(found)
}

fn probe_gpu(env: &dyn Env) -> Option<Vec<Gpu>> {
    let entries = ok(env.read_dir("/sys/bus/pci/devices"))?;
    let mut gpus = Vec::new();
    for entry in entries {
        let base = format!("/sys/bus/pci/devices/{entry}");
        let (Some(class), Some(vendor)) = (
            read_trimmed(env, &format!("{base}/class")),
            read_trimmed(env, &format!("{base}/vendor")),
        ) else {
            continue;
        };
        let class = class.strip_prefix("0x").unwrap_or(&class).to_string();
        let vendor = vendor.strip_prefix("0x").unwrap_or(&vendor).to_string();
        // sysfs class is 6 hex digits (class+subclass+prog-if); the spec table
        // pins the 4-digit class+subclass (0300/0302/0380 family).
        let class4 = class.get(..4).unwrap_or("");
        // VGA/3D/Display controllers.
        if matches!(class4, "0300" | "0301" | "0302" | "0380")
            && matches!(vendor.as_str(), "10de" | "1002" | "8086")
        {
            gpus.push(Gpu {
                vendor,
                class: class4.to_string(),
            });
        }
    }
    Some(gpus)
}

/// Collect facts from the live system (production env).
pub fn collect(env: &dyn Env) -> Facts {
    Facts {
        arch: probe_arch(env),
        cores: probe_cores(env),
        ram_bytes: probe_ram(env),
        free_storage_bytes: free_bytes(Path::new("/var/lib/111")),
        distro: probe_distro(env),
        init: read_trimmed(env, "/proc/1/comm"),
        systemd: Some(ok(env.read_dir("/run/systemd/system")).is_some()),
        in_container: probe_container(env),
        is_wsl: probe_wsl(env),
        battery: probe_battery(env),
        wifi_ap_capable: probe_wifi_ap(env),
        ethernet: probe_ethernet(env),
        ble: probe_ble(env),
        lora: probe_lora(env),
        gpu: probe_gpu(env),
        pi_model: read_trimmed(env, "/proc/device-tree/model")
            .map(|s| s.trim_end_matches('\0').to_string()),
        internet: None, // set by the installer's connectivity step
        is_phone: false,
    }
}

/// Persist facts.json (schema 1) atomically.
pub fn write_facts_json(
    path: &Path,
    facts: &Facts,
    class: &crate::install::categorize::Classification,
) -> anyhow::Result<()> {
    #[derive(Serialize)]
    struct FactsFile<'a> {
        schema: u32,
        facts: &'a Facts,
        category: crate::install::categorize::Category,
        gateway: bool,
        probed_at: String,
    }
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into());
    let f = FactsFile {
        schema: 1,
        facts,
        category: class.category,
        gateway: class.gateway,
        probed_at: now,
    };
    let json = serde_json::to_vec_pretty(&f)?;
    crate::util::fs::atomic_write(path, &json, 0o644)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_lora_table_parses() {
        let t = lora_table();
        assert!(t.len() >= 5, "table must carry the spec rows");
        assert!(t.iter().any(|r| r.vid == "10c4" && r.pid == "ea60"));
        assert!(t.iter().any(|r| r.vid == "1a86" && r.pid == "*"));
        assert!(t.iter().any(|r| r.vid == "303a" && r.pid == "*"));
        assert!(t
            .iter()
            .any(|r| r.vid == "239a" && (r.pid == "4405" || r.pid == "0029")));
        assert!(t.iter().any(|r| r.vid == "239a" && r.pid == "8029"));
    }
}
