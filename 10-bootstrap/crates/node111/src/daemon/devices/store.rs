//! `<root>/state/devices.json` — what the registry remembers about devices:
//!
//! ```json
//! {"schema": 1, "devices": {"<id>": {"alias": null, "forgotten": false,
//!   "first_seen": "…", "last_seen": "…", "last_record_summary": {…},
//!   "firmware": {…}?, "meshtastic": {…}?}}}
//! ```
//!
//! Layer 5 §14 adds two optional fields, written only by the consented
//! identify step (`POST /devices/{id}/identify`): `firmware` (what the probe
//! found, `source: probe`; it overrides the sysfs-only `unknown`) and
//! `meshtastic` — **the Meshtastic binding** meshmud's config is rendered from
//! (the validated uplink plus the board's own node number). At most one
//! device carries a binding (one meshmud, one downlink); Layer 6 migrates it
//! into `interfaces.toml` when that file is born, and from then on this copy
//! is a **write-only mirror** of the TOML ([`mirror_meshtastic_binding`],
//! Layer 6 §11: kept for a Layer 5 rollback, never read by Layer 6).
//!
//! Written atomically (`util::fs::atomic_write`, 0600: it carries device
//! serials and companion IPs). Structural changes (new device, alias, forget,
//! summary change) persist at once; `last_seen` alone persists at most every
//! [`LAST_SEEN_PERSIST`] (SD-card wear on Pi nodes). The file is re-validated
//! on load (ids, alias caps) and a corrupt file is replaced, never trusted.

use super::{
    clean, valid_id, DeviceFirmware, DeviceKind, DeviceMatch, FirmwareFamily, Transport, MAX_ALIAS,
    MAX_FACT,
};
use crate::daemon::meshtastic::meshmud::Uplink;
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

/// Cap for board-reported names kept in a binding (UTF-8 bytes, like §14's
/// presence names; they are untrusted).
pub const MAX_BOARD_NAME: usize = 40;

/// The Meshtastic binding (Layer 5 §14): which uplink meshmud drives for
/// this device, and who answered on it at identify time. Re-validated on
/// every load; an invalid binding is dropped (never rendered).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshtasticBinding {
    pub uplink: Uplink,
    /// The board's own node number (`my_node_info.num`).
    pub node_num: u32,
    /// `!` + `node_num` as 8 hex digits (rebuilt, never copied).
    pub node_id: String,
    /// Board-reported hardware model (untrusted, display only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hw_model: Option<String>,
    /// The board's own long name (untrusted, display only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_name: Option<String>,
}

impl MeshtasticBinding {
    /// A binding for `uplink` and the identified node; `None` when the node
    /// number is not a unicast Meshtastic number or the uplink is invalid.
    pub fn new(
        uplink: Uplink,
        node_num: u32,
        hw_model: Option<&str>,
        long_name: Option<&str>,
    ) -> Option<Self> {
        let b = MeshtasticBinding {
            uplink,
            node_num,
            node_id: format!("!{node_num:08x}"),
            hw_model: hw_model.and_then(board_name),
            long_name: long_name.and_then(board_name),
        };
        b.valid().then_some(b)
    }

    /// Every rule a loaded binding must still pass.
    pub fn valid(&self) -> bool {
        self.uplink.validate().is_ok()
            && self.node_num > 0
            && self.node_num < u32::MAX
            && self.node_id == format!("!{:08x}", self.node_num)
    }
}

/// An untrusted board-reported name → display-safe, ≤ [`MAX_BOARD_NAME`]
/// UTF-8 bytes; `None` when nothing is left.
pub fn board_name(s: &str) -> Option<String> {
    let cleaned = clean(s, MAX_FACT)?;
    let mut out = String::new();
    for c in cleaned.chars() {
        if out.len() + c.len_utf8() > MAX_BOARD_NAME {
            break;
        }
        out.push(c);
    }
    let out = out.trim_end().to_string();
    (!out.is_empty()).then_some(out)
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
    /// Layer 5 §14: what the consented identify probe found (overrides the
    /// sysfs-only `unknown`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware: Option<DeviceFirmware>,
    /// Layer 5 §14: the Meshtastic binding (at most one device has one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meshtastic: Option<MeshtasticBinding>,
}

impl Entry {
    /// A fresh entry first seen at `first_seen`.
    pub fn new(first_seen: String, last_seen: String) -> Self {
        Entry {
            alias: None,
            forgotten: false,
            first_seen,
            last_seen,
            last_record_summary: None,
            firmware: None,
            meshtastic: None,
        }
    }
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
    let mut bound = false;
    for (id, e) in f.devices.iter_mut() {
        e.alias = e.alias.as_deref().and_then(|a| clean(a, MAX_ALIAS));
        if let Some(s) = &mut e.last_record_summary {
            s.name = clean(&s.name, 128).unwrap_or_else(|| "Device".into());
        }
        if let Some(fw) = &mut e.firmware {
            fw.version = fw.version.as_deref().and_then(|v| clean(v, MAX_FACT));
        }
        if let Some(b) = e.meshtastic.take() {
            // re-validated on every load; one binding only (the first by id);
            // a forgotten (hidden) device can never be unbound through the
            // API, so it never holds one
            if !b.valid() {
                eprintln!("111 daemon: devices: dropping an invalid Meshtastic binding on {id}");
            } else if e.forgotten {
                eprintln!("111 daemon: devices: dropping the Meshtastic binding of forgotten {id}");
            } else if bound {
                eprintln!("111 daemon: devices: dropping a second Meshtastic binding on {id}");
            } else {
                bound = true;
                e.meshtastic = Some(MeshtasticBinding {
                    hw_model: b.hw_model.as_deref().and_then(board_name),
                    long_name: b.long_name.as_deref().and_then(board_name),
                    ..b
                });
                // a binding implies a Meshtastic identify result
                if e.firmware.as_ref().map(|f| f.family) != Some(FirmwareFamily::Meshtastic) {
                    e.meshtastic = None;
                    bound = false;
                    eprintln!(
                        "111 daemon: devices: dropping a Meshtastic binding without a Meshtastic \
                         identify result on {id}"
                    );
                }
            }
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

    /// The device holding the Meshtastic binding, if any (never a forgotten
    /// one: forget clears the binding, and nothing binds a forgotten entry).
    pub fn meshtastic_binding(&self) -> Option<(String, MeshtasticBinding)> {
        self.devices
            .iter()
            .filter(|(_, e)| !e.forgotten)
            .find_map(|(id, e)| e.meshtastic.clone().map(|b| (id.clone(), b)))
    }

    /// Layer 6 §11 (integration M7): copy the binding that
    /// `state/interfaces.toml` holds into this store, the **write-only
    /// mirror** kept for a Layer 5 rollback (Layer 6 never reads the binding
    /// from here). `want = Some((id, binding))` binds `id` (creating its
    /// entry when absent, un-forgetting it and marking it a Meshtastic
    /// board, because `sanitize` drops a binding without either) and clears
    /// every other binding; `None` clears them all. Returns the divergence
    /// line (one line naming both values, "interfaces.toml wins") when the
    /// store held something else; persists only then.
    pub fn mirror_meshtastic_binding(
        &mut self,
        want: Option<(&str, &MeshtasticBinding)>,
    ) -> Option<String> {
        let had = self.meshtastic_binding();
        let same = match (&had, &want) {
            (None, None) => true,
            (Some((hid, hb)), Some((wid, wb))) => hid == wid && hb == *wb,
            _ => false,
        };
        // a binding hidden on a forgotten entry is cleared too
        let stray = self
            .devices
            .values()
            .any(|e| e.forgotten && e.meshtastic.is_some());
        if same && !stray {
            return None;
        }
        for (id, e) in self.devices.iter_mut() {
            if want.is_none_or(|(wid, _)| wid != id.as_str()) {
                e.meshtastic = None;
            }
        }
        if let Some((id, b)) = want {
            let entry = self.devices.entry(id.to_string()).or_insert_with(|| {
                let now = time::OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_else(|_| "unknown".into());
                Entry::new(now.clone(), now)
            });
            entry.forgotten = false;
            if entry.firmware.as_ref().map(|f| f.family) != Some(FirmwareFamily::Meshtastic) {
                entry.firmware = Some(DeviceFirmware {
                    family: FirmwareFamily::Meshtastic,
                    version: None,
                    source: super::FirmwareSource::Unknown,
                });
            }
            entry.meshtastic = Some(b.clone());
        }
        self.mark_dirty();
        self.flush(true);
        if same {
            return None;
        }
        let show = |b: &Option<(String, MeshtasticBinding)>| match b {
            Some((id, b)) => format!("{id} {} {}", b.uplink.display(), b.node_id),
            None => "none".into(),
        };
        Some(format!(
            "the Meshtastic binding in devices.json ({}) differs from interfaces.toml ({}); \
             interfaces.toml wins and devices.json now mirrors it",
            show(&had),
            show(&want.map(|(id, b)| (id.to_string(), b.clone())))
        ))
    }

    /// Drop the oldest absent entries beyond [`MAX_ENTRIES`] (a bound device
    /// is never dropped: its binding is configuration).
    pub fn gc(&mut self, present: &dyn Fn(&str) -> bool) {
        while self.devices.len() > MAX_ENTRIES {
            let oldest = self
                .devices
                .iter()
                .filter(|(id, e)| !present(id) && e.meshtastic.is_none())
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

/// [`Store::mirror_meshtastic_binding`] on the file at `path` (Layer 6 §11:
/// called after every save of `state/interfaces.toml` while no in-memory
/// registry store is wired; the file is loaded, re-validated, mirrored and
/// written only when it diverged). Returns the divergence line.
pub fn mirror_meshtastic_binding(
    path: &Path,
    want: Option<(&str, &MeshtasticBinding)>,
) -> Option<String> {
    let mut store = Store::load(path);
    let line = store.mirror_meshtastic_binding(want);
    if line.is_some() && store.dirty {
        eprintln!(
            "111 daemon: devices: the mirrored binding could not be written to {}",
            path.display()
        );
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(last_seen: &str) -> Entry {
        Entry::new("2026-10-01T00:00:00Z".into(), last_seen.into())
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

    fn meshtastic_fw() -> DeviceFirmware {
        DeviceFirmware {
            family: FirmwareFamily::Meshtastic,
            version: Some("2.7.11.abcdef0".into()),
            source: super::super::FirmwareSource::Probe,
        }
    }

    #[test]
    fn bindings_persist_and_are_revalidated() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE);
        let mut s = Store::load(&p);
        let mut e = entry("2026-10-01T00:00:01Z");
        e.firmware = Some(meshtastic_fw());
        e.meshtastic = MeshtasticBinding::new(
            Uplink::serial("/dev/serial/by-id/usb-Heltec-if00").unwrap(),
            0x0a0a0a0a,
            Some("HELTEC_V3"),
            Some("My \u{7}Board"),
        );
        assert_eq!(e.meshtastic.as_ref().unwrap().node_id, "!0a0a0a0a");
        assert_eq!(
            e.meshtastic.as_ref().unwrap().long_name.as_deref(),
            Some("My Board")
        );
        s.devices.insert("abcd1234abcd1234".into(), e.clone());
        s.mark_dirty();
        s.flush(true);
        let again = Store::load(&p);
        assert_eq!(again.devices["abcd1234abcd1234"], e);
        assert_eq!(
            again.meshtastic_binding().map(|(id, b)| (id, b.node_num)),
            Some(("abcd1234abcd1234".into(), 0x0a0a0a0a))
        );
        // a binding is never GC'd even when absent and oldest
        let mut s = again;
        for i in 0..(MAX_ENTRIES + 3) {
            s.devices
                .insert(format!("id{i:05}"), entry("2026-10-02T00:00:00Z"));
        }
        s.gc(&|_| false);
        assert!(s.devices.contains_key("abcd1234abcd1234"));

        // hand-edited bindings: forgotten, bad path, bad id, no identify
        // result, a second one
        let raw = serde_json::json!({"schema": 1, "devices": {
            "0000aaaa0000aaaa": {"first_seen": "a", "last_seen": "b", "forgotten": true,
                "firmware": {"family": "meshtastic", "version": "2.7", "source": "probe"},
                "meshtastic": {"uplink": {"kind": "tcp", "host": "10.0.0.6", "port": 4403},
                               "node_num": 6, "node_id": "!00000006"}},
            "aaaa0000aaaa0000": {"first_seen": "a", "last_seen": "b",
                "firmware": {"family": "meshtastic", "version": "2.7", "source": "probe"},
                "meshtastic": {"uplink": {"kind": "serial", "path": "/etc/shadow"},
                               "node_num": 1, "node_id": "!00000001"}},
            "bbbb0000bbbb0000": {"first_seen": "a", "last_seen": "b",
                "firmware": {"family": "meshtastic", "version": "2.7", "source": "probe"},
                "meshtastic": {"uplink": {"kind": "tcp", "host": "10.0.0.7", "port": 4403},
                               "node_num": 2, "node_id": "!deadbeef"}},
            "cccc0000cccc0000": {"first_seen": "a", "last_seen": "b",
                "meshtastic": {"uplink": {"kind": "tcp", "host": "10.0.0.8", "port": 4403},
                               "node_num": 3, "node_id": "!00000003"}},
            "dddd0000dddd0000": {"first_seen": "a", "last_seen": "b",
                "firmware": {"family": "meshtastic", "version": "2.7", "source": "probe"},
                "meshtastic": {"uplink": {"kind": "tcp", "host": "10.0.0.9", "port": 4403},
                               "node_num": 4, "node_id": "!00000004"}},
            "eeee0000eeee0000": {"first_seen": "a", "last_seen": "b",
                "firmware": {"family": "meshtastic", "version": "2.7", "source": "probe"},
                "meshtastic": {"uplink": {"kind": "tcp", "host": "10.0.0.10", "port": 4403},
                               "node_num": 5, "node_id": "!00000005"}}
        }});
        std::fs::write(&p, raw.to_string()).unwrap();
        let s = Store::load(&p);
        let bound: Vec<_> = s
            .devices
            .iter()
            .filter(|(_, e)| e.meshtastic.is_some())
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(
            bound,
            ["dddd0000dddd0000"],
            "only the first valid binding survives"
        );
        assert_eq!(
            s.devices.len(),
            6,
            "entries stay; only bindings are dropped"
        );
        assert_eq!(
            s.meshtastic_binding().map(|(id, _)| id),
            Some("dddd0000dddd0000".to_string())
        );
        // in memory too: a forgotten entry's binding is never the binding
        let mut s = s;
        s.devices.get_mut("dddd0000dddd0000").unwrap().forgotten = true;
        assert!(s.meshtastic_binding().is_none());
    }

    #[test]
    fn mirror_is_write_only_and_reports_divergence() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE);
        let b = MeshtasticBinding::new(
            Uplink::serial("/dev/serial/by-id/usb-Heltec-if00").unwrap(),
            0x0a0a0a0a,
            None,
            None,
        )
        .unwrap();
        // absent file, nothing wanted: nothing written
        assert_eq!(mirror_meshtastic_binding(&p, None), None);
        assert!(!p.exists());
        // a binding for a device the store has never seen: entry created,
        // marked Meshtastic so a Layer 5 load keeps it
        let line = mirror_meshtastic_binding(&p, Some(("abcd1234abcd1234", &b))).unwrap();
        assert!(line.contains("(none)"), "{line}");
        assert!(line.contains("abcd1234abcd1234 /dev/serial/by-id/usb-Heltec-if00 !0a0a0a0a"));
        let s = Store::load(&p);
        assert_eq!(
            s.meshtastic_binding(),
            Some(("abcd1234abcd1234".to_string(), b.clone()))
        );
        // the same value again: no divergence, no rewrite
        let before = std::fs::metadata(&p).unwrap().modified().unwrap();
        assert_eq!(
            mirror_meshtastic_binding(&p, Some(("abcd1234abcd1234", &b))),
            None
        );
        assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), before);
        // a Layer 5 write moved the binding: the TOML wins, one line names both
        let mut s = Store::load(&p);
        let other =
            MeshtasticBinding::new(Uplink::tcp("10.0.0.6:4403").unwrap(), 6, None, None).unwrap();
        s.devices.get_mut("abcd1234abcd1234").unwrap().meshtastic = Some(other);
        s.mark_dirty();
        s.flush(true);
        let line = mirror_meshtastic_binding(&p, Some(("abcd1234abcd1234", &b))).unwrap();
        assert!(line.contains("10.0.0.6:4403"), "{line}");
        assert!(line.contains("!0a0a0a0a"), "{line}");
        assert_eq!(Store::load(&p).meshtastic_binding().unwrap().1, b);
        // cleared in the TOML: cleared here
        assert!(mirror_meshtastic_binding(&p, None).is_some());
        assert!(Store::load(&p).meshtastic_binding().is_none());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(p.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn board_names_are_capped() {
        assert_eq!(board_name("  \u{1b}[2J "), Some("[2J".into()));
        assert_eq!(board_name(&"é".repeat(40)).unwrap().len(), 40);
        assert_eq!(board_name(" \n "), None);
    }
}
