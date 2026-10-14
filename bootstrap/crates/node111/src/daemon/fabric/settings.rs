//! `fabric/settings` (0600, JSON `{schema: 1, …}`): what the daemon persists
//! for the fabric between runs (`os/messaging/README.md` §9, §15
//! `PATCH /fabric/settings`; the fabric emits `FabricEvent::Settings` whenever
//! they change — enrollment sets `home_hub`, forgetting the hub clears it —
//! and the daemon writes them here).
//!
//! - `hub_role`, `listen_lan`, `custody_opt_in` — §15 settings.
//! - `home_hub` — `{node_id, address}` once enrolled (§7.3).
//! - `peers` — optional, hand-configured direct `tcp-lan` peers
//!   (`[{node_id, address}]`, ≤ [`MAX_PEERS`]): the daemon keeps a dial up to
//!   each (`Dialer::maintain`, the far end's announce must name `node_id`).
//!   Not on the API; a node-local file fact (two members that should talk
//!   directly on a LAN without a hub).
//!
//! Per-class quotas live in the store (§10.4), not here.
//!
//! Defaults when the file is absent (§9: "a hub = … Cat 4 by default"):
//! `hub_role` on for Cat 4 categories, everything else off. A file that is
//! present but unreadable, oversize, a symlink or invalid reads as **all
//! off** (fail closed: no LAN listener, no custody) and is logged; the next
//! settings change rewrites it.

use mesh111::fabric::{FabricSettings, HomeHub};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use id111::keyset::NodeId;

/// File name inside the fabric directory.
pub const SETTINGS_FILE: &str = "settings";
/// Largest settings file read.
pub const MAX_FILE: u64 = 64 * 1024;
/// Hand-configured direct peers kept.
pub const MAX_PEERS: usize = 64;

/// One hand-configured direct peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peer {
    /// The NodeId expected at `address`.
    pub node_id: NodeId,
    /// Its `tcp-lan` listener.
    pub address: SocketAddr,
}

/// Everything in `fabric/settings`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Persisted {
    /// The fabric's own settings.
    pub fabric: FabricSettings,
    /// Direct peers to keep dialed.
    pub peers: Vec<Peer>,
}

#[derive(Serialize, Deserialize)]
struct HubFile {
    node_id: String,
    #[serde(default)]
    address: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct PeerFile {
    node_id: String,
    address: String,
}

#[derive(Serialize, Deserialize)]
struct SettingsFile {
    schema: u32,
    #[serde(default)]
    hub_role: bool,
    #[serde(default)]
    listen_lan: bool,
    #[serde(default)]
    custody_opt_in: bool,
    #[serde(default)]
    home_hub: Option<HubFile>,
    #[serde(default)]
    peers: Vec<PeerFile>,
}

/// Is this Layer 1 category a Cat 4 hub (`Hub4A`, `Hub4B`)?
pub fn is_hub_category(category: Option<&str>) -> bool {
    category.is_some_and(|c| c.starts_with("Hub4"))
}

fn parse_node_id(hex: &str) -> Option<NodeId> {
    (hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .then(|| id111::encoding::hex_decode(hex).ok().map(NodeId::from))
        .flatten()
}

fn decode(text: &str) -> Result<Persisted, String> {
    let f: SettingsFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if f.schema != 1 {
        return Err(format!("unknown schema {}", f.schema));
    }
    let home_hub = match f.home_hub {
        None => None,
        Some(h) => {
            let node_id = parse_node_id(&h.node_id).ok_or("home_hub.node_id is not a NodeId")?;
            let address = match h.address {
                None => None,
                Some(a) => Some(
                    a.parse::<SocketAddr>()
                        .map_err(|_| "home_hub.address is not host:port")?,
                ),
            };
            Some(HomeHub { node_id, address })
        }
    };
    let mut peers = Vec::new();
    for p in f.peers.into_iter().take(MAX_PEERS) {
        let node_id = parse_node_id(&p.node_id).ok_or("peers[].node_id is not a NodeId")?;
        let address = p
            .address
            .parse::<SocketAddr>()
            .map_err(|_| "peers[].address is not ip:port")?;
        peers.push(Peer { node_id, address });
    }
    Ok(Persisted {
        fabric: FabricSettings {
            hub_role: f.hub_role,
            listen_lan: f.listen_lan,
            custody_opt_in: f.custody_opt_in,
            home_hub,
        },
        peers,
    })
}

fn encode(p: &Persisted) -> String {
    let hex = |n: &NodeId| id111::encoding::hex_encode(n.as_bytes());
    let f = SettingsFile {
        schema: 1,
        hub_role: p.fabric.hub_role,
        listen_lan: p.fabric.listen_lan,
        custody_opt_in: p.fabric.custody_opt_in,
        home_hub: p.fabric.home_hub.map(|h| HubFile {
            node_id: hex(&h.node_id),
            address: h.address.map(|a| a.to_string()),
        }),
        peers: p
            .peers
            .iter()
            .map(|x| PeerFile {
                node_id: hex(&x.node_id),
                address: x.address.to_string(),
            })
            .collect(),
    };
    format!(
        "{}\n",
        serde_json::to_string_pretty(&f).unwrap_or_else(|_| "{\"schema\":1}".into())
    )
}

/// The settings file path.
pub fn path(fabric_dir: &Path) -> PathBuf {
    fabric_dir.join(SETTINGS_FILE)
}

/// Load `fabric/settings` (see the module doc for defaults and failures).
pub fn load(fabric_dir: &Path, hub_default: bool) -> Persisted {
    let p = path(fabric_dir);
    let meta = match std::fs::symlink_metadata(&p) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Persisted {
                fabric: FabricSettings {
                    hub_role: hub_default,
                    ..FabricSettings::default()
                },
                peers: Vec::new(),
            }
        }
        Err(e) => {
            eprintln!("111 fabric: settings unreadable ({e}); everything off");
            return Persisted::default();
        }
    };
    if !meta.file_type().is_file() || meta.len() > MAX_FILE {
        eprintln!(
            "111 fabric: {} is not a small regular file; everything off",
            p.display()
        );
        return Persisted::default();
    }
    match std::fs::read_to_string(&p)
        .map_err(|e| e.to_string())
        .and_then(|t| decode(&t))
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("111 fabric: {} invalid ({e}); everything off", p.display());
            Persisted::default()
        }
    }
}

/// Write `fabric/settings` atomically (0600).
pub fn save(fabric_dir: &Path, p: &Persisted) -> anyhow::Result<()> {
    crate::util::fs::atomic_write(&path(fabric_dir), encode(p).as_bytes(), 0o600)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_and_fail_closed() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        assert!(load(d, true).fabric.hub_role, "Cat 4 default");
        assert!(!load(d, false).fabric.hub_role);
        let hub = NodeId::from([0xabu8; 32]);
        let s = Persisted {
            fabric: FabricSettings {
                hub_role: false,
                listen_lan: true,
                custody_opt_in: true,
                home_hub: Some(HomeHub {
                    node_id: hub,
                    address: Some("192.168.1.9:11183".parse().unwrap()),
                }),
            },
            peers: vec![Peer {
                node_id: NodeId::from([1u8; 32]),
                address: "10.0.0.2:11183".parse().unwrap(),
            }],
        };
        save(d, &s).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path(d)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(load(d, true), s, "the file wins over the default");
        // corrupt ⇒ everything off (no listener, no custody)
        std::fs::write(path(d), "{\"schema\":1,\"hub_role\":\"yes\"}").unwrap();
        assert_eq!(load(d, true), Persisted::default());
        std::fs::write(path(d), "{\"schema\":2}").unwrap();
        assert_eq!(load(d, true), Persisted::default());
        std::fs::write(
            path(d),
            "{\"schema\":1,\"home_hub\":{\"node_id\":\"ABCD\"}}",
        )
        .unwrap();
        assert_eq!(load(d, true), Persisted::default());
        // a symlink is never followed
        std::fs::remove_file(path(d)).unwrap();
        std::os::unix::fs::symlink("/etc/hostname", path(d)).unwrap();
        assert_eq!(load(d, true), Persisted::default());
    }

    #[test]
    fn hub_categories() {
        assert!(is_hub_category(Some("Hub4A")));
        assert!(is_hub_category(Some("Hub4B")));
        assert!(!is_hub_category(Some("Node2A")));
        assert!(!is_hub_category(None));
    }
}
