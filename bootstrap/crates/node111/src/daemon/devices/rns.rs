//! Which serial ports the Reticulum stack is configured to drive: the enabled
//! `RNodeInterface` / `RNodeMultiInterface` stanzas of `<root>/rns/config`
//! (Reticulum's ConfigObj-style INI: `[interfaces]` → `[[Name]]` → `key = value`).
//!
//! A stanza binds a device when its `port` resolves to the device's tty: a
//! `/dev/serial/by-id/…` (or any `/dev/…` symlink) is followed inside
//! [`Roots::dev`], so by-id and plain `/dev/ttyACM0` forms both match. Network
//! ports (`tcp://`, `ble://`) are not serial and never match. Reticulum only
//! starts interfaces with `enabled`/`interface_enabled` set true, so stanzas
//! without it are ignored.

use super::{clean, Roots, MAX_FACT};
use std::io::Read;
use std::path::Path;

/// Config read cap (the rendered file is < 1 KB; Layer 6 adds a few stanzas).
const MAX_CONFIG_BYTES: u64 = 256 * 1024;

/// One enabled RNode interface stanza.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RnodeBinding {
    /// Stanza name (`[[RNode LoRa]]`), sanitized.
    pub name: String,
    /// The `port` value as written (sanitized, for `detail`).
    pub port: String,
    /// The tty the port resolves to (`ttyACM0`), when it resolves.
    pub tty: Option<String>,
}

#[derive(Debug, Default)]
struct Stanza {
    name: String,
    kind: Option<String>,
    port: Option<String>,
    enabled: bool,
}

fn truthy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "yes" | "true" | "on" | "1"
    )
}

fn unquote(v: &str) -> &str {
    let v = v.trim();
    v.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(v)
}

/// Enabled RNode stanzas in config `text` (ports unresolved).
pub fn parse(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut section = String::new();
    let mut cur: Option<Stanza> = None;
    let flush = |s: Option<Stanza>, out: &mut Vec<(String, String)>| {
        if let Some(s) = s {
            let rnode = s
                .kind
                .as_deref()
                .is_some_and(|k| k == "RNodeInterface" || k == "RNodeMultiInterface");
            if let (true, true, Some(port)) = (rnode, s.enabled, s.port) {
                out.push((s.name, port));
            }
        }
    };
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(inner) = line.strip_prefix("[[").and_then(|l| l.strip_suffix("]]")) {
            flush(cur.take(), &mut out);
            if section == "interfaces" {
                cur = Some(Stanza {
                    name: inner.trim().to_string(),
                    ..Default::default()
                });
            }
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            flush(cur.take(), &mut out);
            section = inner.trim().to_ascii_lowercase();
            continue;
        }
        let (Some(s), Some((k, v))) = (cur.as_mut(), line.split_once('=')) else {
            continue;
        };
        match k.trim().to_ascii_lowercase().as_str() {
            "type" => s.kind = Some(unquote(v).to_string()),
            "port" => s.port = Some(unquote(v).to_string()),
            "enabled" | "interface_enabled" => s.enabled = truthy(unquote(v)),
            _ => {}
        }
    }
    flush(cur.take(), &mut out);
    out
}

/// The tty a `/dev/…` port path names, following symlinks inside `roots.dev`.
fn resolve_tty(port: &str, roots: &Roots) -> Option<String> {
    let rel = port
        .strip_prefix(&*roots.dev.to_string_lossy())
        .or_else(|| port.strip_prefix("/dev"))?
        .trim_start_matches('/');
    if rel.is_empty() || rel.split('/').any(|c| c == "..") {
        return None;
    }
    let path = roots.dev.join(rel);
    let target = std::fs::canonicalize(&path).ok()?;
    let name = target.file_name()?.to_string_lossy().into_owned();
    Some(name)
}

/// Read `config` and return its enabled RNode bindings (empty when the file is
/// absent or unreadable).
pub fn bindings(config: &Path, roots: &Roots) -> Vec<RnodeBinding> {
    let Ok(f) = std::fs::File::open(config) else {
        return Vec::new();
    };
    let mut buf = Vec::new();
    if f.take(MAX_CONFIG_BYTES).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    parse(&String::from_utf8_lossy(&buf))
        .into_iter()
        .map(|(name, port)| RnodeBinding {
            tty: resolve_tty(&port, roots),
            name: clean(&name, MAX_FACT).unwrap_or_else(|| "RNode".into()),
            port: clean(&port, super::MAX_PATH_FACT).unwrap_or_default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
schema-version = 1

[reticulum]
  enable_transport = no
  share_instance = no

[interfaces]
  [[Auto Interface]]
    type = AutoInterface
    enabled = yes

  [[Roof RNode]]   # the 868 MHz board
    type = RNodeInterface
    interface_enabled = True
    port = /dev/serial/by-id/usb-Silicon_Labs_CP2102-if00-port0
    frequency = 867200000

  [[Spare]]
    type = RNodeInterface
    enabled = no
    port = /dev/ttyUSB3

  [[Wifi RNode]]
    type = RNodeInterface
    enabled = yes
    port = tcp://10.0.0.5

  [[Quoted]]
    type = "RNodeMultiInterface"
    enabled = yes
    port = "/dev/ttyACM1"

[logging]
  loglevel = 4
"#;

    #[test]
    fn parses_enabled_rnode_stanzas_only() {
        assert_eq!(
            parse(CONFIG),
            vec![
                (
                    "Roof RNode".to_string(),
                    "/dev/serial/by-id/usb-Silicon_Labs_CP2102-if00-port0".to_string()
                ),
                ("Wifi RNode".to_string(), "tcp://10.0.0.5".to_string()),
                ("Quoted".to_string(), "/dev/ttyACM1".to_string()),
            ]
        );
        assert!(parse("").is_empty());
        // a stanza outside [interfaces] is not an interface
        assert!(parse(
            "[other]\n[[X]]\ntype = RNodeInterface\nenabled = yes\nport = /dev/ttyACM0\n"
        )
        .is_empty());
    }

    #[test]
    fn ports_resolve_through_dev_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let dev = dir.path().join("dev");
        std::fs::create_dir_all(dev.join("serial/by-id")).unwrap();
        std::fs::write(dev.join("ttyACM0"), "").unwrap();
        std::os::unix::fs::symlink("../../ttyACM0", dev.join("serial/by-id/usb-X-if00")).unwrap();
        let roots = Roots {
            dev: dev.clone(),
            sys: dir.path().join("sys"),
        };
        assert_eq!(
            resolve_tty("/dev/serial/by-id/usb-X-if00", &roots).as_deref(),
            Some("ttyACM0")
        );
        assert_eq!(
            resolve_tty("/dev/ttyACM0", &roots).as_deref(),
            Some("ttyACM0")
        );
        let rooted = format!("{}/ttyACM0", dev.display());
        assert_eq!(resolve_tty(&rooted, &roots).as_deref(), Some("ttyACM0"));
        assert_eq!(resolve_tty("tcp://10.0.0.5", &roots), None);
        assert_eq!(resolve_tty("/dev/../etc/passwd", &roots), None);
        assert_eq!(resolve_tty("/dev/ttyUSB9", &roots), None, "absent");
    }
}
