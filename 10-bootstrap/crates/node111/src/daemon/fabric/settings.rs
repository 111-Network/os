//! `fabric/settings` (0600, JSON `{schema: 1, …}`): what the daemon persists
//! for the fabric between runs (`os/50-messaging/README.md` §9, §15
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
//! off** (fail closed: no LAN listener, no custody) and is **moved aside**
//! to `settings.invalid-<n>` (kept for the operator, its path logged once).
//!
//! The home hub is the one fact a member cannot re-derive from its own
//! switches, so a member whose file is absent or invalid gets it back from
//! the store ([`rebuild_home_hub`], run by the fabric once the store is
//! open): the single `enrolled` contact that is a custody holder (its card
//! carries the mailbox role) or holds this node's mailbox cursor, preferring
//! a `home_hub` salvaged from the invalid file when that names one. The hub's
//! address comes back only from such a salvage; without it nothing dials
//! the hub until a fresh offer (the contact keeps it `enrolled`).
//!
//! Dial targets (`home_hub.address`, `peers[].address`) must name a peer:
//! an unspecified, multicast or broadcast address — and loopback while the
//! adapter refuses loopback peers (§2) — makes the file invalid
//! ([`dial_refusal`], shared with `POST /fabric/hub/enroll`).

use mesh111::fabric::{FabricSettings, HomeHub};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
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

/// Why `addr` is no `tcp-lan` dial target, if it is not: port 0, an
/// unspecified address (a connect to `0.0.0.0` reaches this node itself), a
/// multicast or the IPv4 broadcast address (no single peer), or loopback
/// while `refuse_loopback` is on (§2: loopback is never a peer). IPv4-mapped
/// IPv6 addresses are judged as IPv4.
pub fn dial_refusal(addr: &SocketAddr, refuse_loopback: bool) -> Option<&'static str> {
    let ip = addr.ip().to_canonical();
    if addr.port() == 0 {
        Some("port 0 names no listener")
    } else if ip.is_unspecified() {
        Some("an unspecified address (0.0.0.0, ::) names no peer")
    } else if ip.is_multicast() {
        Some("a multicast address names no single peer")
    } else if matches!(ip, IpAddr::V4(v4) if v4.is_broadcast()) {
        Some("the broadcast address names no single peer")
    } else if refuse_loopback && ip.is_loopback() {
        Some("loopback is never a peer (§2)")
    } else {
        None
    }
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

fn dial_target(raw: &str, field: &str, refuse_loopback: bool) -> Result<SocketAddr, String> {
    let a = raw
        .parse::<SocketAddr>()
        .map_err(|_| format!("{field} is not ip:port"))?;
    match dial_refusal(&a, refuse_loopback) {
        Some(why) => Err(format!("{field} {a}: {why}")),
        None => Ok(a),
    }
}

fn decode(text: &str, refuse_loopback: bool) -> Result<Persisted, String> {
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
                Some(a) => Some(dial_target(&a, "home_hub.address", refuse_loopback)?),
            };
            Some(HomeHub { node_id, address })
        }
    };
    let mut peers = Vec::new();
    for p in f.peers.into_iter().take(MAX_PEERS) {
        let node_id = parse_node_id(&p.node_id).ok_or("peers[].node_id is not a NodeId")?;
        let address = dial_target(&p.address, "peers[].address", refuse_loopback)?;
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

/// What [`load`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// The settings to run with.
    pub persisted: Persisted,
    /// `Some` when the file was absent or invalid: the home hub may have to
    /// be rebuilt from the store ([`rebuild_home_hub`]).
    pub recover: Option<Recover>,
}

/// An absent or invalid settings file (module doc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recover {
    /// The file was present but invalid.
    pub invalid: bool,
    /// Where the invalid file was moved (`None`: absent, or the move
    /// failed — then nothing rewrites it until the next settings change).
    pub moved_to: Option<PathBuf>,
    /// A `home_hub` read leniently from the invalid file (unverified: used
    /// only when it names an `enrolled` contact).
    pub hint: Option<HomeHub>,
}

/// Attempts at a free `settings.invalid-<n>` name.
const MOVE_ASIDE_TRIES: u32 = 1000;

/// Move the invalid file aside (never deleted; the symlink itself, never
/// its target).
fn move_aside(fabric_dir: &Path) -> Option<PathBuf> {
    let from = path(fabric_dir);
    for n in 1..=MOVE_ASIDE_TRIES {
        let to = fabric_dir.join(format!("{SETTINGS_FILE}.invalid-{n}"));
        if std::fs::symlink_metadata(&to).is_ok() {
            continue;
        }
        return match std::fs::rename(&from, &to) {
            Ok(()) => Some(to),
            Err(e) => {
                eprintln!("111 fabric: cannot move {} aside: {e}", from.display());
                None
            }
        };
    }
    eprintln!(
        "111 fabric: cannot move {} aside: no free {SETTINGS_FILE}.invalid-<n> name",
        from.display()
    );
    None
}

/// The `home_hub` of an invalid file, read leniently (its NodeId and, if it
/// is a dial target, its address).
fn salvage_hub(text: &str, refuse_loopback: bool) -> Option<HomeHub> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let h = v.get("home_hub")?;
    let node_id = parse_node_id(h.get("node_id")?.as_str()?)?;
    let address = h
        .get("address")
        .and_then(serde_json::Value::as_str)
        .and_then(|a| dial_target(a, "home_hub.address", refuse_loopback).ok());
    Some(HomeHub { node_id, address })
}

/// Load `fabric/settings` (see the module doc for defaults and failures).
pub fn load(fabric_dir: &Path, hub_default: bool, refuse_loopback: bool) -> Loaded {
    let p = path(fabric_dir);
    let invalid = |why: String, text: Option<&str>| {
        let moved_to = move_aside(fabric_dir);
        match &moved_to {
            Some(to) => eprintln!(
                "111 fabric: {} invalid ({why}); moved to {}; everything off",
                p.display(),
                to.display()
            ),
            None => eprintln!("111 fabric: {} invalid ({why}); everything off", p.display()),
        }
        Loaded {
            persisted: Persisted::default(),
            recover: Some(Recover {
                invalid: true,
                moved_to,
                hint: text.and_then(|t| salvage_hub(t, refuse_loopback)),
            }),
        }
    };
    let meta = match std::fs::symlink_metadata(&p) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Loaded {
                persisted: Persisted {
                    fabric: FabricSettings {
                        hub_role: hub_default,
                        ..FabricSettings::default()
                    },
                    peers: Vec::new(),
                },
                recover: Some(Recover {
                    invalid: false,
                    moved_to: None,
                    hint: None,
                }),
            }
        }
        Err(e) => {
            // Not even its metadata: leave it in place, run with all off.
            eprintln!("111 fabric: settings unreadable ({e}); everything off");
            return Loaded {
                persisted: Persisted::default(),
                recover: Some(Recover {
                    invalid: true,
                    moved_to: None,
                    hint: None,
                }),
            };
        }
    };
    if !meta.file_type().is_file() || meta.len() > MAX_FILE {
        return invalid("not a small regular file".into(), None);
    }
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) => return invalid(e.to_string(), None),
    };
    match decode(&text, refuse_loopback) {
        Ok(s) => Loaded {
            persisted: s,
            recover: None,
        },
        Err(e) => invalid(e, Some(&text)),
    }
}

/// One `enrolled` contact as the store holds it (input to
/// [`rebuild_home_hub`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Enrolled {
    /// Its NodeId.
    pub node_id: NodeId,
    /// Its card carries the mailbox role (it holds custody: a hub).
    pub custody: bool,
    /// This node's mailbox cursor at it (member side; 0 = never fetched).
    pub cursor: u64,
}

/// The home hub of a member whose settings file was absent or invalid
/// (module doc): the salvaged `hint` when it names an `enrolled` contact,
/// else the single `enrolled` contact that holds custody or this node's
/// mailbox cursor. On a hub its `enrolled` contacts are members — they hold
/// no custody and no cursor of this node, so nothing is rebuilt there.
pub fn rebuild_home_hub(enrolled: &[Enrolled], hint: Option<&HomeHub>) -> Option<HomeHub> {
    if let Some(h) = hint {
        if enrolled.iter().any(|e| e.node_id == h.node_id) {
            return Some(*h);
        }
    }
    let mut hubs = enrolled.iter().filter(|e| e.custody || e.cursor > 0);
    match (hubs.next(), hubs.next()) {
        (Some(one), None) => Some(HomeHub {
            node_id: one.node_id,
            address: None,
        }),
        _ => None,
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
        assert!(load(d, true, true).persisted.fabric.hub_role, "Cat 4 default");
        assert!(!load(d, false, true).persisted.fabric.hub_role);
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
        let l = load(d, true, true);
        assert_eq!(l.persisted, s, "the file wins over the default");
        assert_eq!(l.recover, None);
        // corrupt ⇒ everything off (no listener, no custody), moved aside
        for (n, bad) in [
            "{\"schema\":1,\"hub_role\":\"yes\"}",
            "{\"schema\":2}",
            "{\"schema\":1,\"home_hub\":{\"node_id\":\"ABCD\"}}",
        ]
        .into_iter()
        .enumerate()
        {
            std::fs::write(path(d), bad).unwrap();
            let l = load(d, true, true);
            assert_eq!(l.persisted, Persisted::default(), "{bad}");
            let r = l.recover.unwrap();
            assert!(r.invalid);
            let aside = d.join(format!("settings.invalid-{}", n + 1));
            assert_eq!(r.moved_to.as_deref(), Some(aside.as_path()));
            assert_eq!(std::fs::read_to_string(&aside).unwrap(), bad, "kept");
            assert!(!path(d).exists(), "moved, not copied");
        }
        // a symlink is never followed (the link itself is moved aside)
        std::os::unix::fs::symlink("/etc/hostname", path(d)).unwrap();
        let l = load(d, true, true);
        assert_eq!(l.persisted, Persisted::default());
        let aside = l.recover.unwrap().moved_to.unwrap();
        assert!(std::fs::symlink_metadata(&aside)
            .unwrap()
            .file_type()
            .is_symlink());
        // oversize
        std::fs::write(path(d), vec![b' '; MAX_FILE as usize + 1]).unwrap();
        assert!(load(d, true, true).recover.unwrap().invalid);
    }

    /// F2 review m1: dial targets must name a peer, in the file as on the
    /// enroll route.
    #[test]
    fn dial_targets_refuse_unspecified_multicast_broadcast_loopback() {
        for bad in [
            "0.0.0.0:11183",
            "[::]:11183",
            "224.0.0.1:11183",
            "[ff02::1]:11183",
            "255.255.255.255:11183",
            "[::ffff:0.0.0.0]:11183",
            "10.0.0.1:0",
        ] {
            let a: SocketAddr = bad.parse().unwrap();
            assert!(dial_refusal(&a, false).is_some(), "{bad}");
        }
        for lo in ["127.0.0.1:11183", "[::1]:11183", "[::ffff:127.0.0.1]:11183"] {
            let a: SocketAddr = lo.parse().unwrap();
            assert!(dial_refusal(&a, true).is_some(), "{lo}");
            assert!(dial_refusal(&a, false).is_none(), "{lo} (tests)");
        }
        for ok in ["192.168.1.9:11183", "[fe80::1]:11183", "10.0.0.255:11183"] {
            assert!(dial_refusal(&ok.parse().unwrap(), true).is_none(), "{ok}");
        }
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        let hub = id111::encoding::hex_encode(&[0xab; 32]);
        for addr in [
            "0.0.0.0:11183",
            "[::]:11183",
            "224.0.0.1:11183",
            "255.255.255.255:11183",
        ] {
            for text in [
                format!(r#"{{"schema":1,"home_hub":{{"node_id":"{hub}","address":"{addr}"}}}}"#),
                format!(r#"{{"schema":1,"peers":[{{"node_id":"{hub}","address":"{addr}"}}]}}"#),
            ] {
                assert!(decode(&text, false).is_err(), "{text}");
                std::fs::write(path(d), &text).unwrap();
                assert!(load(d, false, false).recover.unwrap().invalid, "{text}");
            }
        }
        let lo = format!(r#"{{"schema":1,"peers":[{{"node_id":"{hub}","address":"127.0.0.1:1"}}]}}"#);
        assert!(decode(&lo, true).is_err(), "loopback refused in production");
        assert!(decode(&lo, false).is_ok(), "allowed when tests turn it off");
    }

    /// F2 review m3: the home hub comes back from the store's `enrolled`
    /// contact; a salvaged address only when the hint names that contact.
    #[test]
    fn home_hub_is_rebuilt_from_the_enrolled_contact() {
        let hub = NodeId::from([0xab; 32]);
        let member = NodeId::from([0x01; 32]);
        let other = NodeId::from([0x02; 32]);
        let e = |node_id, custody, cursor| Enrolled {
            node_id,
            custody,
            cursor,
        };
        // a member: one enrolled contact, a custody holder
        let got = rebuild_home_hub(&[e(hub, true, 0)], None).unwrap();
        assert_eq!((got.node_id, got.address), (hub, None));
        // or one that holds this node's mailbox cursor
        assert_eq!(
            rebuild_home_hub(&[e(hub, false, 7)], None).map(|h| h.node_id),
            Some(hub)
        );
        // a hub's enrolled contacts are members: nothing to rebuild
        assert_eq!(rebuild_home_hub(&[e(member, false, 0)], None), None);
        assert_eq!(
            rebuild_home_hub(&[e(member, false, 0), e(other, false, 0)], None),
            None
        );
        // two custody holders: ambiguous, nothing guessed
        assert_eq!(
            rebuild_home_hub(&[e(hub, true, 0), e(other, true, 0)], None),
            None
        );
        // a salvaged hint wins when it names an enrolled contact
        let hint = HomeHub {
            node_id: hub,
            address: Some("192.168.1.9:11183".parse().unwrap()),
        };
        assert_eq!(
            rebuild_home_hub(&[e(hub, false, 0), e(other, true, 0)], Some(&hint)),
            Some(hint)
        );
        let stranger = HomeHub {
            node_id: NodeId::from([0x77; 32]),
            ..hint
        };
        assert_eq!(rebuild_home_hub(&[e(hub, true, 0)], Some(&stranger)).map(|h| h.address), Some(None));
        // salvage reads a type-broken file's home_hub (and refuses a bad address)
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        let hex = id111::encoding::hex_encode(hub.as_bytes());
        std::fs::write(
            path(d),
            format!(r#"{{"schema":1,"hub_role":"yes","home_hub":{{"node_id":"{hex}","address":"192.168.1.9:11183"}}}}"#),
        )
        .unwrap();
        assert_eq!(load(d, false, true).recover.unwrap().hint, Some(hint));
        std::fs::write(
            path(d),
            format!(r#"{{"schema":9,"home_hub":{{"node_id":"{hex}","address":"0.0.0.0:1"}}}}"#),
        )
        .unwrap();
        let h = load(d, false, true).recover.unwrap().hint.unwrap();
        assert_eq!((h.node_id, h.address), (hub, None));
        std::fs::write(path(d), "{not json").unwrap();
        assert_eq!(load(d, false, true).recover.unwrap().hint, None);
        // absent: defaults, recover without a hint
        let t = tempfile::tempdir().unwrap();
        let l = load(t.path(), true, true);
        assert!(l.persisted.fabric.hub_role);
        let r = l.recover.unwrap();
        assert!(!r.invalid && r.hint.is_none() && r.moved_to.is_none());
    }

    #[test]
    fn hub_categories() {
        assert!(is_hub_category(Some("Hub4A")));
        assert!(is_hub_category(Some("Hub4B")));
        assert!(!is_hub_category(Some("Node2A")));
        assert!(!is_hub_category(None));
    }
}
