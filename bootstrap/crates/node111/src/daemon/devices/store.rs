//! `<root>/state/devices.json` — what the registry remembers about devices:
//!
//! ```json
//! {"schema": 1, "devices": {"<id>": {"alias": null, "forgotten": false,
//!   "first_seen": "…", "last_seen": "…", "last_record_summary": {…}}}}
//! ```
//!
//! Written atomically (`util::fs::atomic_write`, 0600: it carries device
//! serials and companion IPs). Structural changes (new device, alias, forget,
//! summary change) persist at once; `last_seen` alone persists at most every
//! [`LAST_SEEN_PERSIST`] (SD-card wear on Pi nodes). The file is re-validated
//! on load (ids, alias caps) and a corrupt file is replaced, never trusted.

use super::{clean, valid_id, DeviceFirmware, DeviceKind, DeviceMatch, Transport, MAX_ALIAS};
use crate::util::fs::atomic_write;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const FILE: &str = "devices.json";
/// Remembered entries kept (oldest absent ones dropped beyond this).
pub const MAX_ENTRIES: usize = 256;
pub const LAST_SEEN_PERSIST: Duration = Duration::from_secs(600);

/// Enough of the last record to render the device while it is absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub kind: DeviceKind,
    pub name: String,
    pub transport: Transport,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kb_entry: Option<String>,
    #[serde(default)]
    pub identity: Map<String, Value>,
    #[serde(default)]
    pub id_basis: String,
    #[serde(default, rename = "match", skip_serializing_if = "Option::is_none")]
    pub matched: Option<DeviceMatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware: Option<DeviceFirmware>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub forgotten: bool,
    pub first_seen: String,
    pub last_seen: String,
    #[serde(default)]
    pub last_record_summary: Option<Summary>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoreFile {
    pub schema: u32,
    #[serde(default)]
    pub devices: BTreeMap<String, Entry>,
}

/// In-memory store + write policy.
pub struct Store {
    path: PathBuf,
    pub devices: BTreeMap<String, Entry>,
    dirty: bool,
    last_persist: Option<Instant>,
    warned: bool,
}

fn sanitize(mut f: StoreFile) -> BTreeMap<String, Entry> {
    f.devices.retain(|id, _| valid_id(id));
    for e in f.devices.values_mut() {
        e.alias = e.alias.as_deref().and_then(|a| clean(a, MAX_ALIAS));
        if let Some(s) = &mut e.last_record_summary {
            s.name = clean(&s.name, 128).unwrap_or_else(|| "Device".into());
        }
    }
    f.devices
}

impl Store {
    /// Load `path` (missing → empty; corrupt → empty + logged, rewritten on the
    /// next change).
    pub fn load(path: &Path) -> Self {
        let devices = match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<StoreFile>(&bytes) {
                Ok(f) => sanitize(f),
                Err(e) => {
                    eprintln!(
                        "111 daemon: devices: {} is corrupt ({e}); starting empty",
                        path.display()
                    );
                    BTreeMap::new()
                }
            },
            Err(_) => BTreeMap::new(),
        };
        Store {
            path: path.to_path_buf(),
            devices,
            dirty: false,
            last_persist: None,
            warned: false,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Drop the oldest absent entries beyond [`MAX_ENTRIES`].
    pub fn gc(&mut self, present: &dyn Fn(&str) -> bool) {
        while self.devices.len() > MAX_ENTRIES {
            let oldest = self
                .devices
                .iter()
                .filter(|(id, _)| !present(id))
                .min_by(|a, b| a.1.last_seen.cmp(&b.1.last_seen))
                .map(|(id, _)| id.clone());
            match oldest {
                Some(id) => {
                    self.devices.remove(&id);
                    self.dirty = true;
                }
                None => break,
            }
        }
    }

    /// Persist when dirty, or when only `last_seen` moved and the throttle
    /// elapsed. `force` writes now (user mutations).
    pub fn flush(&mut self, force: bool) {
        let due = self
            .last_persist
            .is_none_or(|t| t.elapsed() >= LAST_SEEN_PERSIST);
        if !(force || self.dirty || due) {
            return;
        }
        let file = StoreFile {
            schema: 1,
            devices: self.devices.clone(),
        };
        let body = match serde_json::to_vec_pretty(&file) {
            Ok(b) => b,
            Err(_) => return,
        };
        match atomic_write(&self.path, &body, 0o600) {
            Ok(()) => {
                self.dirty = false;
                self.last_persist = Some(Instant::now());
            }
            Err(e) => {
                if !self.warned {
                    eprintln!(
                        "111 daemon: devices: cannot persist {}: {e:#} (continuing in memory)",
                        self.path.display()
                    );
                    self.warned = true;
                }
                // retry on the next throttle window, not every tick
                self.last_persist = Some(Instant::now());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(last_seen: &str) -> Entry {
        Entry {
            alias: None,
            forgotten: false,
            first_seen: "2026-10-01T00:00:00Z".into(),
            last_seen: last_seen.into(),
            last_record_summary: None,
        }
    }

    #[test]
    fn roundtrip_sanitize_and_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE);
        // corrupt → empty
        std::fs::write(&p, b"{not json").unwrap();
        assert!(Store::load(&p).devices.is_empty());
        // invalid ids dropped, alias re-capped/cleaned
        let raw = serde_json::json!({
            "schema": 1,
            "devices": {
                "../../etc": {"first_seen": "a", "last_seen": "b"},
                "abcd1234abcd1234": {"alias": format!("  a\u{7}{}", "z".repeat(100)),
                                      "first_seen": "a", "last_seen": "b"}
            }
        });
        std::fs::write(&p, raw.to_string()).unwrap();
        let mut s = Store::load(&p);
        assert_eq!(s.devices.len(), 1);
        let alias = s.devices["abcd1234abcd1234"].alias.clone().unwrap();
        assert_eq!(alias.chars().count(), MAX_ALIAS);
        assert!(alias.starts_with("az"));
        s.mark_dirty();
        s.flush(false);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(p.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        let again = Store::load(&p);
        assert_eq!(again.devices, s.devices);
    }

    #[test]
    fn gc_drops_oldest_absent_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::load(&dir.path().join(FILE));
        for i in 0..(MAX_ENTRIES + 3) {
            s.devices.insert(
                format!("id{i:05}"),
                entry(&format!("2026-10-01T00:{:02}:00Z", i % 60)),
            );
        }
        // keep id00000 (oldest timestamp) because it is "present"
        s.gc(&|id| id == "id00000");
        assert_eq!(s.devices.len(), MAX_ENTRIES);
        assert!(s.devices.contains_key("id00000"));
    }
}
