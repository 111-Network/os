//! Hardware detection for the device registry (spec §7.1): USB serial ports
//! (`/dev/serial/by-id`), every non-hub USB device (`/sys/bus/usb/devices`),
//! Bluetooth adapters (`/sys/class/bluetooth`). Read-only and side-effect free:
//! nothing here opens a tty (opening a CDC port toggles DTR and resets most
//! ESP32 boards) — port access is checked with `access(2)` instead.
//!
//! Every path is rooted in [`Roots`] so tests run against a fake tree. Every
//! string read from sysfs is untrusted: [`super::clean`] trims it, strips
//! control/bidi characters and caps it at [`super::MAX_FACT`] chars. The scan
//! is bounded (entry caps below) so a pathological tree cannot stall the tick.

use super::{clean, MAX_FACT};
use std::collections::HashSet;
use std::ffi::CString;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Filesystem roots (production: `/dev`, `/sys`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    pub dev: PathBuf,
    pub sys: PathBuf,
}

impl Default for Roots {
    fn default() -> Self {
        Roots {
            dev: PathBuf::from("/dev"),
            sys: PathBuf::from("/sys"),
        }
    }
}

/// Entry caps per directory read (bounded work per tick).
const MAX_BY_ID: usize = 64;
const MAX_USB: usize = 256;
const MAX_HCI: usize = 16;
const MAX_ATTR_BYTES: u64 = 4096;
/// Ancestor hops from a tty/hci `device` link up to the USB device dir.
const MAX_WALK_UP: usize = 8;

/// USB (interface) classes that are never "a device the user attached for 111":
/// audio, HID, imaging, printer, mass storage, hub, smart card, video, A/V,
/// billboard, wireless controller (BT dongles surface as `hci*` adapters).
/// A USB device is skipped only when ALL of its classes are in this set.
pub const IGNORED_USB_CLASSES: &[&str] = &[
    "01", "03", "06", "07", "08", "09", "0b", "0e", "10", "11", "e0",
];

/// One USB device as sysfs describes it (strings already sanitized).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsbInfo {
    /// sysfs bus id (`1-1.2`): the directory name under `/sys/bus/usb/devices`.
    pub bus_id: String,
    /// Canonical sysfs path (dedupe key between the three sources; not exported).
    pub path: PathBuf,
    /// 4 lowercase hex digits.
    pub vendor_id: String,
    pub product_id: String,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    /// `bDeviceClass` (2 lowercase hex), when readable.
    pub device_class: Option<String>,
    /// `bInterfaceClass` of every interface (sorted, deduped).
    pub interface_classes: Vec<String>,
    /// tty names the kernel created under this device (cdc_acm / usb-serial).
    pub ttys: Vec<String>,
}

/// A serial port backed (usually) by a USB device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialPort {
    /// Raw `/dev/serial/by-id/<name>` symlink name (the stable key; udev-escaped).
    /// `None` when found through sysfs only (no udev, e.g. minimal containers).
    pub by_id: Option<String>,
    /// tty node name (`ttyACM0`).
    pub tty: String,
    pub usb: Option<UsbInfo>,
}

/// A Bluetooth adapter (`/sys/class/bluetooth/hciN`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtAdapter {
    pub hci: String,
    /// Uppercase `AA:BB:CC:DD:EE:FF`, when readable and well-formed.
    pub address: Option<String>,
    pub name: Option<String>,
    /// The USB device behind the adapter, if it is USB-attached.
    pub usb: Option<UsbInfo>,
}

/// One detection pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    pub serial: Vec<SerialPort>,
    /// USB devices with no tty and no Bluetooth adapter on top (bootloader-mode
    /// boards, vendor-specific gadgets). Hubs and ignored classes are skipped.
    pub usb_other: Vec<UsbInfo>,
    pub bluetooth: Vec<BtAdapter>,
}

/// Read a small sysfs attribute (bounded), lossy UTF-8.
fn read_attr(path: &Path) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    f.take(MAX_ATTR_BYTES).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// A sanitized attribute (see module docs).
fn read_fact(dir: &Path, name: &str) -> Option<String> {
    read_attr(&dir.join(name)).and_then(|s| clean(&s, MAX_FACT))
}

/// `n` lowercase hex digits, else `None`.
fn hex_id(raw: Option<String>, n: usize) -> Option<String> {
    let s = raw?.trim().to_ascii_lowercase();
    let s = s.strip_prefix("0x").unwrap_or(&s).to_string();
    (s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit())).then_some(s)
}

/// Directory entry names (lossy), sorted, capped.
fn list_dir(dir: &Path, cap: usize) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .take(cap * 4)
        .collect();
    names.sort();
    names.truncate(cap);
    names
}

/// Kernel device-node names: `ttyACM0`, `ttyUSB12`, `hci0` (no separators).
fn is_node_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Normalize a MAC (`aa:bb:..` → `AA:BB:..`); `None` unless 6 hex octets.
pub fn normalize_mac(s: &str) -> Option<String> {
    let s = s.trim();
    let parts: Vec<&str> = s.split(':').collect();
    let ok = parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()));
    ok.then(|| s.to_ascii_uppercase())
}

struct Ctx {
    sys_canon: PathBuf,
}

impl Ctx {
    fn new(roots: &Roots) -> Self {
        Ctx {
            sys_canon: std::fs::canonicalize(&roots.sys).unwrap_or_else(|_| roots.sys.clone()),
        }
    }

    /// From a sysfs path (tty/hci `device` link), walk up to the first ancestor
    /// that is a USB device (has `idVendor` + `idProduct`), staying inside sysfs.
    fn usb_ancestor(&self, start: &Path) -> Option<UsbInfo> {
        let mut cur = std::fs::canonicalize(start).ok()?;
        for _ in 0..MAX_WALK_UP {
            if !cur.starts_with(&self.sys_canon) {
                return None;
            }
            if cur.join("idVendor").is_file() && cur.join("idProduct").is_file() {
                return self.usb_info(&cur);
            }
            if !cur.pop() {
                return None;
            }
        }
        None
    }

    /// Describe the USB device at canonical sysfs dir `dir`.
    fn usb_info(&self, dir: &Path) -> Option<UsbInfo> {
        let vendor_id = hex_id(read_attr(&dir.join("idVendor")), 4)?;
        let product_id = hex_id(read_attr(&dir.join("idProduct")), 4)?;
        let bus_id = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .and_then(|n| clean(&n, MAX_FACT))
            .unwrap_or_default();
        let mut interface_classes = Vec::new();
        let mut ttys = Vec::new();
        for entry in list_dir(dir, 32) {
            // interfaces are `<bus_id>:<config>.<n>`
            if !entry.contains(':') {
                continue;
            }
            let intf = dir.join(&entry);
            if let Some(c) = hex_id(read_attr(&intf.join("bInterfaceClass")), 2) {
                interface_classes.push(c);
            }
            // cdc_acm: <intf>/tty/ttyACM0 · usb-serial: <intf>/ttyUSB0
            for t in list_dir(&intf.join("tty"), 8) {
                if is_node_name(&t) {
                    ttys.push(t);
                }
            }
            for t in list_dir(&intf, 64) {
                if t.starts_with("tty") && t != "tty" && is_node_name(&t) {
                    ttys.push(t);
                }
            }
        }
        interface_classes.sort();
        interface_classes.dedup();
        ttys.sort();
        ttys.dedup();
        Some(UsbInfo {
            bus_id,
            path: dir.to_path_buf(),
            vendor_id,
            product_id,
            manufacturer: read_fact(dir, "manufacturer"),
            product: read_fact(dir, "product"),
            serial: read_fact(dir, "serial"),
            device_class: hex_id(read_attr(&dir.join("bDeviceClass")), 2),
            interface_classes,
            ttys,
        })
    }
}

/// Is this USB device something to list (not a hub, not only ignored classes)?
fn listable(u: &UsbInfo) -> bool {
    if u.device_class.as_deref() == Some("09") {
        return false;
    }
    let classes: Vec<&str> = if u.interface_classes.is_empty() {
        u.device_class.iter().map(String::as_str).collect()
    } else {
        u.interface_classes.iter().map(String::as_str).collect()
    };
    classes.is_empty() || classes.iter().any(|c| !IGNORED_USB_CLASSES.contains(c))
}

/// One detection pass over `roots` (blocking filesystem reads; cheap).
pub fn scan(roots: &Roots) -> Scan {
    let ctx = Ctx::new(roots);
    let mut out = Scan::default();
    let mut claimed: HashSet<PathBuf> = HashSet::new();
    let mut claimed_ttys: HashSet<String> = HashSet::new();

    // (a) USB serial ports by stable udev name.
    let by_id_dir = roots.dev.join("serial/by-id");
    if let Ok(rd) = std::fs::read_dir(&by_id_dir) {
        let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).take(MAX_BY_ID * 4).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries.into_iter().take(MAX_BY_ID) {
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(target) = std::fs::read_link(e.path()) else {
                continue;
            };
            let Some(tty) = target
                .file_name()
                .map(|t| t.to_string_lossy().into_owned())
                .filter(|t| is_node_name(t))
            else {
                continue;
            };
            let usb = ctx.usb_ancestor(&roots.sys.join("class/tty").join(&tty).join("device"));
            if let Some(u) = &usb {
                claimed.insert(u.path.clone());
            }
            claimed_ttys.insert(tty.clone());
            out.serial.push(SerialPort {
                by_id: Some(name),
                tty,
                usb,
            });
        }
    }

    // (c) Bluetooth adapters (before USB so a dongle is not listed twice).
    for hci in list_dir(&roots.sys.join("class/bluetooth"), MAX_HCI) {
        // `hci0:256` entries are connections, not adapters.
        if !hci.starts_with("hci") || !is_node_name(&hci) {
            continue;
        }
        let dir = roots.sys.join("class/bluetooth").join(&hci);
        let usb = ctx.usb_ancestor(&dir.join("device"));
        if let Some(u) = &usb {
            claimed.insert(u.path.clone());
        }
        out.bluetooth.push(BtAdapter {
            address: read_attr(&dir.join("address")).and_then(|a| normalize_mac(&a)),
            name: read_fact(&dir, "name"),
            hci,
            usb,
        });
    }

    // (b) Every other USB device (bootloader-mode boards, tty without udev).
    for entry in list_dir(&roots.sys.join("bus/usb/devices"), MAX_USB) {
        if entry.contains(':') {
            continue; // interface, not a device
        }
        let Ok(dir) = std::fs::canonicalize(roots.sys.join("bus/usb/devices").join(&entry)) else {
            continue;
        };
        if claimed.contains(&dir) {
            continue;
        }
        let Some(u) = ctx.usb_info(&dir) else {
            continue;
        };
        if !listable(&u) {
            continue;
        }
        claimed.insert(dir);
        let free: Vec<String> = u
            .ttys
            .iter()
            .filter(|t| !claimed_ttys.contains(*t))
            .cloned()
            .collect();
        if free.is_empty() {
            out.usb_other.push(u);
        } else {
            for tty in free {
                claimed_ttys.insert(tty.clone());
                out.serial.push(SerialPort {
                    by_id: None,
                    tty,
                    usb: Some(u.clone()),
                });
            }
        }
    }
    out
}

/// Can this process read+write the tty (without opening it)?
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortAccess {
    /// `ok` · `denied` · `missing`
    pub access: &'static str,
    /// Permission bits, `0660` style.
    pub mode: Option<String>,
    /// Owning group name (numeric when unnamed).
    pub group: Option<String>,
}

fn group_name(gid: u32) -> String {
    let mut grp: libc::group = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut result: *mut libc::group = std::ptr::null_mut();
    let rc = unsafe { libc::getgrgid_r(gid, &mut grp, buf.as_mut_ptr(), buf.len(), &mut result) };
    if rc == 0 && !result.is_null() && !grp.gr_name.is_null() {
        let name = unsafe { std::ffi::CStr::from_ptr(grp.gr_name) };
        if let Some(n) = clean(&name.to_string_lossy(), MAX_FACT) {
            return n;
        }
    }
    gid.to_string()
}

/// `access(2)` R+W on `path` plus its mode/group (for the agent's facts).
pub fn port_access(path: &Path) -> PortAccess {
    let Ok(meta) = std::fs::metadata(path) else {
        return PortAccess {
            access: "missing",
            mode: None,
            group: None,
        };
    };
    let ok = CString::new(path.as_os_str().as_bytes())
        .map(|c| unsafe { libc::access(c.as_ptr(), libc::R_OK | libc::W_OK) } == 0)
        .unwrap_or(false);
    PortAccess {
        access: if ok { "ok" } else { "denied" },
        mode: Some(format!("{:04o}", meta.mode() & 0o7777)),
        group: Some(group_name(meta.gid())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_normalization() {
        assert_eq!(
            normalize_mac(" aa:bb:cc:dd:ee:0f\n").as_deref(),
            Some("AA:BB:CC:DD:EE:0F")
        );
        for bad in [
            "",
            "aa:bb",
            "aa:bb:cc:dd:ee:gg",
            "aabbccddeeff",
            "a:b:c:d:e:f",
        ] {
            assert_eq!(normalize_mac(bad), None, "{bad}");
        }
    }

    #[test]
    fn hex_ids() {
        assert_eq!(hex_id(Some("303A\n".into()), 4).as_deref(), Some("303a"));
        assert_eq!(hex_id(Some("0x10c4".into()), 4).as_deref(), Some("10c4"));
        assert_eq!(hex_id(Some("30".into()), 4), None);
        assert_eq!(hex_id(Some("zzzz".into()), 4), None);
        assert_eq!(hex_id(None, 4), None);
    }

    #[test]
    fn listable_rules() {
        let mut u = UsbInfo {
            device_class: Some("09".into()),
            ..Default::default()
        };
        assert!(!listable(&u), "hub");
        u.device_class = Some("00".into());
        u.interface_classes = vec!["03".into()];
        assert!(!listable(&u), "HID only");
        u.interface_classes = vec!["03".into(), "ff".into()];
        assert!(listable(&u), "HID + vendor-specific");
        u.interface_classes.clear();
        u.device_class = Some("e0".into());
        assert!(!listable(&u), "BT dongle device class");
        u.device_class = None;
        assert!(listable(&u), "nothing known → list it");
    }

    #[test]
    fn node_names() {
        assert!(is_node_name("ttyACM0"));
        assert!(!is_node_name("../tty"));
        assert!(!is_node_name("hci0:256"));
        assert!(!is_node_name(""));
    }
}
