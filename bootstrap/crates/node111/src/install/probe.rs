//! Hardware/environment facts. Every fact degrades to "unknown" (None) — probing
//! must never hard-fail an install on a weird box.

use serde::{Deserialize, Serialize};

/// I/O the probe needs, injectable for fixture-tree tests.
pub trait Env {
    fn read_file(&self, path: &str) -> std::io::Result<String>;
    fn read_dir(&self, path: &str) -> std::io::Result<Vec<String>>;
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<String>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoraDevice {
    pub vid: String,
    pub pid: String,
    pub product: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Gpu {
    /// PCI vendor id, lowercase hex (e.g. "10de", "1002", "8086").
    pub vendor: String,
    /// PCI class, lowercase hex (0300/0302/0380 family).
    pub class: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Facts {
    pub arch: Option<String>,
    pub cores: Option<u32>,
    pub ram_bytes: Option<u64>,
    /// Free bytes on the filesystem that holds (or will hold) /var/lib/111.
    pub free_storage_bytes: Option<u64>,
    /// Distro family: "debian" | "fedora" | "arch" | other id.
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
    pub internet: Option<bool>,
    /// True when probing through the phone flow (`--phone`).
    pub is_phone: bool,
}

/// Collect facts from the live system (production env).
pub fn collect(env: &dyn Env) -> Facts {
    let _ = env;
    todo!("T4")
}
