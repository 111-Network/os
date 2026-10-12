//! Install receipt (`/var/lib/111/receipt.json`, root 0644). Root-owned so the
//! node can't forge it; treated as UNTRUSTED input on re-read (re-probe, never
//! trust — spec §On-disk layout).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhoneReceipt {
    pub serial: String,
    pub model: String,
    pub sdk: u32,
    pub oem: String,
    /// On-device taps performed (for the guided-path transparency).
    pub taps: Vec<String>,
    /// BFU/FBE reality note ("starts at first unlock" when a lockscreen is set).
    pub bf_note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub schema: u32,
    pub version: String,
    pub installed_at: String,
    pub category: String,
    pub gateway: bool,
    pub npub: String,
    /// sha256 of every cached offline artifact (relpath under cache/ → hash).
    pub cache_hashes: BTreeMap<String, String>,
    /// Degradations taken (no python3, container relay, unknown distro, …).
    pub degrade_branches: Vec<String>,
    pub phone: Option<PhoneReceipt>,
}

impl Receipt {
    pub fn new(version: &str) -> Self {
        Receipt {
            schema: 1,
            version: version.to_string(),
            installed_at: time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| "unknown".into()),
            category: String::new(),
            gateway: false,
            npub: String::new(),
            cache_hashes: BTreeMap::new(),
            degrade_branches: Vec::new(),
            phone: None,
        }
    }

    /// Read the receipt. Caller treats every field as a HINT (re-probe) — a
    /// forged or stale receipt must never change security decisions.
    pub fn read(path: &Path) -> anyhow::Result<Option<Self>> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let r: Receipt = serde_json::from_slice(&bytes)
            .map_err(|e| anyhow::anyhow!("corrupt receipt {}: {e}", path.display()))?;
        anyhow::ensure!(r.schema == 1, "receipt schema {} unsupported", r.schema);
        Ok(Some(r))
    }

    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        let json = serde_json::to_vec_pretty(self)?;
        crate::util::fs::atomic_write(path, &json, 0o644)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("receipt.json");
        assert!(Receipt::read(&p).unwrap().is_none());
        let mut r = Receipt::new("2026.10.12");
        r.cache_hashes
            .insert("packages/python/rns.whl".into(), "ab".into());
        r.write(&p).unwrap();
        let back = Receipt::read(&p).unwrap().unwrap();
        assert_eq!(back.version, "2026.10.12");
        assert_eq!(back.cache_hashes.len(), 1);
        std::fs::write(&p, b"{nope").unwrap();
        assert!(Receipt::read(&p).is_err());
    }
}
