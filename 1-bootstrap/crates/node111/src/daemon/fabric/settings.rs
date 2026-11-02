//! `fabric/settings` (0600, JSON `{schema: 1, …}`): what the daemon persists
//! for the fabric between runs (`os/5-messaging/README.md` §9, §15
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
//! - `paired_hubs` — Layer 7 §4.6 (amendment 10, §19 C9; additive, the
//!   schema stays 1): one row per paired hub, ≤ [`MAX_PAIRED_HUBS`]. The
//!   fabric's record `{node_id, role, via, ours, theirs}`
//!   (`FabricSettings.paired_hubs`, the one record its gate reads, persisted
//!   through `FabricEvent::Settings`; `ours` and `theirs` are the two
//!   enrollments the pair holds, always written, `false` when absent: a pair
//!   is whole, and forwards, only with both) and, in the same row, the
//!   daemon's half ([`PairLink`]): the far hub's
//!   `address`, its pinned `link_spki` (§4.2), `lan` (§4.1 rule 1) and the
//!   `pair_plain` `warning_hash` (§4.6 step 6); plus `far_holds_address`
//!   (written only when `true`), the §4.6 step 5 fact the dial rule needs
//!   ([`pair_dials`]). With no pair the key is not written at all, so the
//!   file's bytes are exactly what a build before Layer 7 wrote. A daemon
//!   half whose fabric record does not exist yet (a pin recorded before the
//!   pairing's `ENROLL_OK`) is kept in memory only. A row written before
//!   the two bools existed (an M5 build of this branch only; no such file
//!   exists elsewhere) reads with both `false`: a half pair, which neither
//!   forwards nor is dialled by the daemon until it is paired again.
//!
//! Per-class quotas live in the store (§10.4), not here.
//!
//! Defaults when the file is absent (§9: "a hub = … Cat 4 by default"):
//! `hub_role` on for Cat 4 categories, everything else off. A file that is
//! present but unreadable, oversize, a symlink or invalid reads as **all
//! off** (fail closed: no LAN listener, no custody, no pair) and is **moved
//! aside** to `settings.invalid-<n>` (kept for the operator, its path logged
//! once).
//!
//! The home hub is the one fact a member cannot re-derive from its own
//! switches, so a member whose file is absent or invalid gets it back from
//! the store ([`rebuild_home_hub`], run by the fabric once the store is
//! open). This node's own stored announce is authoritative whenever a
//! verified one is stored (its 8-byte `appdata` hint, §7.3 A8, read through
//! `IdentitySealer::stored_hub_hint`): one that names no hub rebuilds
//! nothing; one that names a hub admits only the `enrolled` contact it
//! names. Only without a stored announce does the single `enrolled`
//! contact holding this node's mailbox cursor decide. A `home_hub` salvaged
//! from the invalid file is preferred when it names an admitted contact.
//! **A paired hub is never rebuilt as the home hub** (Layer 7 §19 C7): it is
//! never named in the own announce (a paired hub is never a home hub,
//! amendment 12; a former home hub keeps its cursor row for good, so the
//! cursor alone is no evidence), and the pairs salvaged from an invalid
//! file are skipped outright. The hub's address comes back only from such
//! a salvage; without it nothing dials the hub until a fresh offer (the
//! contact keeps it `enrolled`).
//!
//! Dial targets (`home_hub.address`, `peers[].address`,
//! `paired_hubs[].address`) must name a peer: an unspecified, multicast or
//! broadcast address — and loopback while the adapter refuses loopback
//! peers (§2) — makes the file invalid ([`dial_refusal`], shared with
//! `POST /fabric/hub/enroll`).

use super::sealer::StoredHubHint;
use mesh111::contacts::{card_text, parse_card_text};
use mesh111::fabric::{FabricSettings, HomeHub, PairRole, PairVia, PairedHub, MAX_PAIRED_HUBS};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use id111::keyset::NodeId;

/// File name inside the fabric directory.
pub const SETTINGS_FILE: &str = "settings";
/// Largest settings file read.
pub const MAX_FILE: u64 = 64 * 1024;
/// Hand-configured direct peers kept.
pub const MAX_PEERS: usize = 64;
/// A link key's SubjectPublicKeyInfo: Ed25519, DER (Layer 7 §4.2).
pub const LINK_SPKI_LEN: usize = 44;
/// The fixed DER prefix of an Ed25519 SubjectPublicKeyInfo (RFC 8410 §4):
/// `SEQUENCE { SEQUENCE { OID 1.3.101.112 } BIT STRING (0 unused) }`, then
/// the 32-byte key.
pub const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];
/// [`LINK_SPKI_LEN`] bytes as unpadded base32 (Layer 7 §4.6 step 1: 71).
pub const LINK_SPKI_TEXT_LEN: usize = 71;

/// One hand-configured direct peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peer {
    /// The NodeId expected at `address`.
    pub node_id: NodeId,
    /// Its `tcp-lan` listener.
    pub address: SocketAddr,
}

/// A paired hub's link key (Layer 7 §4.2): its Ed25519
/// SubjectPublicKeyInfo, 44 B of DER, pinned TOFU from the `111PAIR:`
/// bundle and compared byte-exact on every handshake (D5). Only the shape
/// is checked here: the length and the fixed Ed25519 prefix.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct LinkSpki([u8; LINK_SPKI_LEN]);

impl std::fmt::Debug for LinkSpki {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LinkSpki({})", self.text())
    }
}

impl LinkSpki {
    /// The SPKI from its DER bytes, or `None` when they are not 44 B of an
    /// Ed25519 SubjectPublicKeyInfo.
    pub fn from_der(der: &[u8]) -> Option<LinkSpki> {
        let b: [u8; LINK_SPKI_LEN] = der.try_into().ok()?;
        (b[..ED25519_SPKI_PREFIX.len()] == ED25519_SPKI_PREFIX).then_some(LinkSpki(b))
    }

    /// The SPKI from its base32 text (the card alphabet, RFC 4648, either
    /// case, exactly [`LINK_SPKI_TEXT_LEN`] characters: no padding or
    /// separator is skipped, and the trailing bits must be zero).
    pub fn from_text(text: &str) -> Option<LinkSpki> {
        if text.len() != LINK_SPKI_TEXT_LEN || !text.bytes().all(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        LinkSpki::from_der(&parse_card_text(text).ok()?)
    }

    /// The DER bytes.
    pub fn as_der(&self) -> &[u8; LINK_SPKI_LEN] {
        &self.0
    }

    /// The raw 32-byte Ed25519 key inside.
    pub fn raw_key(&self) -> &[u8] {
        &self.0[ED25519_SPKI_PREFIX.len()..]
    }

    /// Base32 text (upper case, unpadded), as the file and the bundle carry it.
    pub fn text(&self) -> String {
        card_text(&self.0)
    }
}

/// The daemon's half of one pairing row (Layer 7 §4.6, C9): what the fabric
/// does not keep. Nothing in it is a secret (the SPKI is a public key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairLink {
    /// The far hub (the row's key).
    pub node_id: NodeId,
    /// The far hub's `tcp-lan` address from its bundle (`None`: its bundle
    /// carried none, or a `carrier` pair).
    pub address: Option<SocketAddr>,
    /// The far hub's link key, pinned from its bundle (§4.2).
    pub link_spki: Option<LinkSpki>,
    /// The owner marked the pair a LAN or an adopted tunnel: plain
    /// `tcp-lan`, no TLS (§4.1 rule 1).
    pub lan: bool,
    /// SHA-256 of the `pair_plain` warning text the owner accepted for this
    /// pair (§4.6 step 6; §4.1 rule 4), `None` when none was shown.
    pub warning_hash: Option<[u8; 32]>,
    /// This hub's own bundle showed the far hub an address, so the far hub
    /// holds one for this hub (§4.6 step 5). `false` (the default, and the
    /// safe one: this hub then dials whenever it holds the far address).
    pub far_holds_address: bool,
}

impl PairLink {
    /// A row with nothing but the NodeId (no address, no pin).
    pub fn new(node_id: NodeId) -> PairLink {
        PairLink {
            node_id,
            address: None,
            link_spki: None,
            lan: false,
            warning_hash: None,
            far_holds_address: false,
        }
    }
}

/// The daemon's pairing rows, shared live with the `tcp-lan` stream wrap
/// (Layer 7 §4.4, D5: `relay::tls::wrap`), so a pin recorded while the node
/// runs reaches the next connection. Updated by the fabric handle whenever
/// [`Persisted::pairs`] changes; read-only for everyone else.
pub type PairTable = Arc<RwLock<Vec<PairLink>>>;

/// Everything in `fabric/settings`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Persisted {
    /// The fabric's own settings.
    pub fabric: FabricSettings,
    /// Direct peers to keep dialed.
    pub peers: Vec<Peer>,
    /// The daemon's half of each pairing row (Layer 7 §4.6), in no
    /// particular order; the rows a file holds are the ones the fabric
    /// recorded (`fabric.paired_hubs`), the others are in memory only.
    pub pairs: Vec<PairLink>,
}

impl Persisted {
    /// The daemon's half of the row for `node_id`.
    pub fn pair_link(&self, node_id: &NodeId) -> Option<&PairLink> {
        self.pairs.iter().find(|l| l.node_id == *node_id)
    }
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

/// One `paired_hubs` row, in the §4.6 key order (the fabric's two
/// enrollment bools after `via`).
#[derive(Serialize, Deserialize)]
struct PairFile {
    node_id: String,
    #[serde(default)]
    address: Option<String>,
    role: String,
    via: String,
    #[serde(default)]
    ours: bool,
    #[serde(default)]
    theirs: bool,
    #[serde(default)]
    link_spki: Option<String>,
    #[serde(default)]
    lan: bool,
    #[serde(default)]
    warning_hash: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    far_holds_address: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
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
    /// Layer 7 §4.6: never written while empty (byte-identical without a
    /// pair).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    paired_hubs: Vec<PairFile>,
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

/// 64 lowercase hex characters as 32 bytes.
fn parse_hex32(hex: &str) -> Option<[u8; 32]> {
    (hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .then(|| id111::encoding::hex_decode(hex).ok())
        .flatten()
}

fn parse_node_id(hex: &str) -> Option<NodeId> {
    parse_hex32(hex).map(NodeId::from)
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

/// Why `link` cannot be recorded for this node (`own`), if it cannot: it
/// names this node, or its address is no dial target ([`dial_refusal`]).
pub fn pair_link_refusal(
    link: &PairLink,
    own: &NodeId,
    refuse_loopback: bool,
) -> Option<&'static str> {
    if link.node_id == *own {
        return Some("a hub never pairs with itself");
    }
    link.address
        .as_ref()
        .and_then(|a| dial_refusal(a, refuse_loopback))
}

/// The `paired_hubs` rows of a file (Layer 7 §4.6): the fabric's records
/// and the daemon's halves, in file order. A row the fabric would refuse
/// (§4.6 step 2's role and link rules), a hub named twice, the home hub
/// (amendment 12: a paired hub is never a home hub) or a malformed field
/// makes the whole file invalid (fail closed, the module doc).
fn decode_pairs(
    rows: Vec<PairFile>,
    home_hub: Option<&HomeHub>,
    refuse_loopback: bool,
) -> Result<(Vec<PairedHub>, Vec<PairLink>), String> {
    let mut hubs: Vec<PairedHub> = Vec::new();
    let mut links = Vec::new();
    for p in rows.into_iter().take(MAX_PAIRED_HUBS) {
        let node_id = parse_node_id(&p.node_id).ok_or("paired_hubs[].node_id is not a NodeId")?;
        let role = PairRole::from_code(&p.role)
            .ok_or("paired_hubs[].role is not peer, leaf or gateway")?;
        let via = PairVia::from_code(&p.via).ok_or("paired_hubs[].via is not tcp or carrier")?;
        let hub = PairedHub {
            node_id,
            role,
            via,
            ours: p.ours,
            theirs: p.theirs,
        };
        if !hub.valid() {
            return Err(
                "paired_hubs[]: a carrier pair is a peer; leaf and gateway pairs are tcp".into(),
            );
        }
        if home_hub.is_some_and(|h| h.node_id == node_id) {
            return Err(
                "paired_hubs[] names the home hub (a paired hub is never the home hub)".into(),
            );
        }
        if hubs.iter().any(|h| h.node_id == node_id) {
            return Err("paired_hubs[] names one hub twice".into());
        }
        let address = p
            .address
            .map(|a| dial_target(&a, "paired_hubs[].address", refuse_loopback))
            .transpose()?;
        let link_spki = p
            .link_spki
            .map(|t| {
                LinkSpki::from_text(&t).ok_or(
                    "paired_hubs[].link_spki is not an Ed25519 SubjectPublicKeyInfo in base32",
                )
            })
            .transpose()?;
        let warning_hash = p
            .warning_hash
            .map(|h| parse_hex32(&h).ok_or("paired_hubs[].warning_hash is not 64 lowercase hex"))
            .transpose()?;
        hubs.push(hub);
        links.push(PairLink {
            node_id,
            address,
            link_spki,
            lan: p.lan,
            warning_hash,
            far_holds_address: p.far_holds_address,
        });
    }
    Ok((hubs, links))
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
    let (paired_hubs, pairs) = decode_pairs(f.paired_hubs, home_hub.as_ref(), refuse_loopback)?;
    Ok(Persisted {
        fabric: FabricSettings {
            hub_role: f.hub_role,
            listen_lan: f.listen_lan,
            custody_opt_in: f.custody_opt_in,
            home_hub,
            paired_hubs,
        },
        peers,
        pairs,
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
        // One row per fabric record, the daemon's half beside it (§4.6).
        paired_hubs: p
            .fabric
            .paired_hubs
            .iter()
            .map(|h| {
                let l = p.pair_link(&h.node_id);
                PairFile {
                    node_id: hex(&h.node_id),
                    address: l.and_then(|l| l.address).map(|a| a.to_string()),
                    role: h.role.code().to_string(),
                    via: h.via.code().to_string(),
                    ours: h.ours,
                    theirs: h.theirs,
                    link_spki: l.and_then(|l| l.link_spki.as_ref()).map(LinkSpki::text),
                    lan: l.is_some_and(|l| l.lan),
                    warning_hash: l
                        .and_then(|l| l.warning_hash.as_ref())
                        .map(id111::encoding::hex_encode),
                    far_holds_address: l.is_some_and(|l| l.far_holds_address),
                }
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
    /// The `paired_hubs[].node_id`s read leniently from the invalid file
    /// (Layer 7 §19 C7): never rebuilt as the home hub. The pairs themselves
    /// are not restored (an invalid file runs with everything off).
    pub pairs: Vec<NodeId>,
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

/// The paired hubs' NodeIds of an invalid file, read leniently (every
/// `paired_hubs[].node_id` that parses, at most [`MAX_PAIRED_HUBS`]).
fn salvage_pairs(text: &str) -> Vec<NodeId> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    v.get("paired_hubs")
        .and_then(serde_json::Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r.get("node_id")?.as_str().and_then(parse_node_id))
                .take(MAX_PAIRED_HUBS)
                .collect()
        })
        .unwrap_or_default()
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
            None => eprintln!(
                "111 fabric: {} invalid ({why}); everything off",
                p.display()
            ),
        }
        Loaded {
            persisted: Persisted::default(),
            recover: Some(Recover {
                invalid: true,
                moved_to,
                hint: text.and_then(|t| salvage_hub(t, refuse_loopback)),
                pairs: text.map(salvage_pairs).unwrap_or_default(),
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
                    ..Persisted::default()
                },
                recover: Some(Recover {
                    invalid: false,
                    moved_to: None,
                    hint: None,
                    pairs: Vec::new(),
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
                    pairs: Vec::new(),
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
/// (module doc). `pairs` are never picked (Layer 7 §19 C7). The node's own
/// stored announce (`announced`, [`StoredHubHint`]) is authoritative
/// whenever a verified one is stored: it is re-minted on every home-hub
/// change (§7.3 A8) and a paired hub is never named there (amendment 12).
///
/// - [`StoredHubHint::NoHub`]: the node's last signed statement names no
///   home hub, so **nothing is rebuilt** — not even from a mailbox cursor.
///   A cursor row is never deleted (a former home hub keeps its cursor > 0
///   for good, even after it is forgotten and paired with), so a cursor is
///   no evidence once the announce says otherwise.
/// - [`StoredHubHint::Hub`]: only an `enrolled` contact whose NodeId begins
///   with the hint and that is a custody holder or holds this node's
///   mailbox cursor can be picked: the salvaged `hint` when it names one,
///   else the single one (the 8-byte prefix is a locator; two such
///   contacts are ambiguous and nothing is guessed).
/// - [`StoredHubHint::NoAnnounce`] (none stored, or another node's or junk
///   bytes): the salvaged `hint` when it names an `enrolled` contact, else
///   the single `enrolled` contact that holds this node's mailbox cursor
///   (this node fetched from it; a pairing writes no cursor, amendment 10).
///   This fallback cannot tell a former home hub that became a pair from
///   the home hub when the file and the announce are both gone (no salvage
///   names the pairs then).
///
/// Anything else is ambiguous and nothing is guessed. Both sides of an
/// enrollment pin each other `enrolled`, so on a hub its `enrolled`
/// contacts are members and, when paired, the paired hubs: the members
/// hold no cursor of this node and are never named in its announce.
pub fn rebuild_home_hub(
    enrolled: &[Enrolled],
    hint: Option<&HomeHub>,
    announced: StoredHubHint,
    pairs: &[NodeId],
) -> Option<HomeHub> {
    let candidates: Vec<&Enrolled> = match announced {
        StoredHubHint::NoHub => return None,
        StoredHubHint::Hub(a) => enrolled
            .iter()
            .filter(|e| e.node_id.as_bytes()[..8] == a && (e.custody || e.cursor > 0))
            .collect(),
        StoredHubHint::NoAnnounce => enrolled.iter().collect(),
    };
    let candidates: Vec<&Enrolled> = candidates
        .into_iter()
        .filter(|e| !pairs.contains(&e.node_id))
        .collect();
    if let Some(h) = hint {
        if candidates.iter().any(|e| e.node_id == h.node_id) {
            return Some(*h);
        }
    }
    let pick: Vec<&Enrolled> = match announced {
        StoredHubHint::Hub(_) => candidates,
        _ => candidates.into_iter().filter(|e| e.cursor > 0).collect(),
    };
    match pick.as_slice() {
        [one] => Some(HomeHub {
            node_id: one.node_id,
            address: None,
        }),
        _ => None,
    }
}

/// Layer 7 §4.6 step 5, one connection per pair: the paired hubs this hub
/// dials through `peers` (`Fabric::dial_peer`), as `(far hub, address)`.
/// Only a **whole** pair (`PairedHub::whole`: both enrollments recorded) is
/// dialled here: a half pair's second `ENROLL` rides the fabric's own
/// pairing dial, which the fabric keeps while the pair is half and releases
/// once it is whole. A whole `tcp` pair is dialled when this hub holds
/// the far hub's address and either the far hub holds none for this hub
/// (`far_holds_address` false, the default) or this NodeId sorts lower
/// (bytewise, the 64-hex order). Nothing without the hub role (§5.11), for a
/// `carrier` pair (it talks only through its carrier), for a daemon half
/// with no fabric record, or for this node itself: an empty list without a
/// pair.
pub fn pair_dials(own: &NodeId, p: &Persisted) -> Vec<(NodeId, SocketAddr)> {
    if !p.fabric.hub_role {
        return Vec::new();
    }
    p.fabric
        .paired_hubs
        .iter()
        .filter(|h| h.whole() && h.via == PairVia::Tcp && h.node_id != *own)
        .filter_map(|h| {
            let l = p.pair_link(&h.node_id)?;
            let address = l.address?;
            let dials = !l.far_holds_address || own.as_bytes() < h.node_id.as_bytes();
            dials.then_some((h.node_id, address))
        })
        .collect()
}

/// Does the home hub or a configured peer use `address` (a dial no pair
/// change may release)?
pub fn holds_address(p: &Persisted, address: SocketAddr) -> bool {
    p.peers.iter().any(|x| x.address == address)
        || p.fabric.home_hub.and_then(|h| h.address) == Some(address)
}

/// Fold an unpairing the pump has not applied yet into `p` (Layer 7 §4.6;
/// M5c S2): the records `current` (the fabric's current `paired_hubs`)
/// no longer holds leave `p.fabric.paired_hubs`, and the daemon's halves
/// beside them leave `p.pairs`. Records `current` adds are not taken here
/// (they arrive with their `Settings` event, the daemon's half written
/// then). Returns whether anything was dropped.
pub fn fold_dropped_records(p: &mut Persisted, current: &[PairedHub]) -> bool {
    let gone: Vec<NodeId> = p
        .fabric
        .paired_hubs
        .iter()
        .map(|h| h.node_id)
        .filter(|n| !current.iter().any(|c| c.node_id == *n))
        .collect();
    if gone.is_empty() {
        return false;
    }
    p.fabric.paired_hubs.retain(|h| !gone.contains(&h.node_id));
    p.pairs.retain(|l| !gone.contains(&l.node_id));
    true
}

/// Why `link`'s address cannot be recorded beside `p`, if it cannot: the
/// home hub, a configured peer or another paired hub already uses it. A
/// second dial to an address the adapter already keeps is a no-op that
/// keeps the first dial's expected NodeId (`Dialer::maintain`), so such a
/// pair could never connect (§4.6 step 5).
pub fn pair_address_refusal(p: &Persisted, link: &PairLink) -> Option<&'static str> {
    let a = link.address?;
    if holds_address(p, a) {
        Some("that address is the home hub's or a configured peer's")
    } else if p
        .pairs
        .iter()
        .any(|x| x.node_id != link.node_id && x.address == Some(a))
    {
        Some("that address is another paired hub's")
    } else {
        None
    }
}

/// The bytes [`save`] writes for `p` (built under the fabric handle's
/// lock, written after it in order).
pub(super) fn encoded(p: &Persisted) -> String {
    encode(p)
}

/// Write `fabric/settings` atomically (0600) from bytes [`encoded`] built.
pub(super) fn save_encoded(fabric_dir: &Path, text: &str) -> anyhow::Result<()> {
    crate::util::fs::atomic_write(&path(fabric_dir), text.as_bytes(), 0o600)
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
        assert!(
            load(d, true, true).persisted.fabric.hub_role,
            "Cat 4 default"
        );
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
                paired_hubs: Vec::new(),
            },
            peers: vec![Peer {
                node_id: NodeId::from([1u8; 32]),
                address: "10.0.0.2:11183".parse().unwrap(),
            }],
            pairs: Vec::new(),
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
        let lo =
            format!(r#"{{"schema":1,"peers":[{{"node_id":"{hub}","address":"127.0.0.1:1"}}]}}"#);
        assert!(decode(&lo, true).is_err(), "loopback refused in production");
        assert!(decode(&lo, false).is_ok(), "allowed when tests turn it off");
    }

    /// F2 review m3: the home hub comes back from the store's `enrolled`
    /// contact; a salvaged address only when the hint names that contact.
    /// Layer 7 §19 C7: the "nothing fetched yet" case needs the own
    /// announce's home-hub hint (a sole custody contact may be a pair).
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
        let says = |n: &NodeId| StoredHubHint::Hub(prefix8(n));
        // a member: one enrolled custody holder its own announce names
        let got = rebuild_home_hub(&[e(hub, true, 0)], None, says(&hub), &[]).unwrap();
        assert_eq!((got.node_id, got.address), (hub, None));
        // … and without that announce hint nothing is guessed (it might be a pair)
        assert_eq!(
            rebuild_home_hub(&[e(hub, true, 0)], None, StoredHubHint::NoAnnounce, &[]),
            None
        );
        assert_eq!(
            rebuild_home_hub(&[e(hub, true, 0)], None, says(&other), &[]),
            None
        );
        // the hint names a hub, but its card carries no mailbox role
        assert_eq!(
            rebuild_home_hub(&[e(hub, false, 0)], None, says(&hub), &[]),
            None
        );
        // or one that holds this node's mailbox cursor
        assert_eq!(
            rebuild_home_hub(&[e(hub, false, 7)], None, StoredHubHint::NoAnnounce, &[])
                .map(|h| h.node_id),
            Some(hub)
        );
        // a hub's enrolled contacts are members: nothing to rebuild
        assert_eq!(
            rebuild_home_hub(&[e(member, false, 0)], None, StoredHubHint::NoAnnounce, &[]),
            None
        );
        assert_eq!(
            rebuild_home_hub(
                &[e(member, false, 0), e(other, false, 0)],
                None,
                StoredHubHint::NoAnnounce,
                &[]
            ),
            None
        );
        // two custody holders: ambiguous unless the own announce names one
        assert_eq!(
            rebuild_home_hub(
                &[e(hub, true, 0), e(other, true, 0)],
                None,
                StoredHubHint::NoAnnounce,
                &[]
            ),
            None
        );
        assert_eq!(
            rebuild_home_hub(
                &[e(hub, true, 0), e(other, true, 0)],
                None,
                says(&other),
                &[]
            )
            .map(|h| h.node_id),
            Some(other)
        );
        // a hub whose members include one that opted into custody: that
        // member is no home hub (its own announce names none)
        assert_eq!(
            rebuild_home_hub(
                &[e(member, true, 0), e(other, false, 0)],
                None,
                StoredHubHint::NoAnnounce,
                &[]
            ),
            None
        );
        // the mailbox cursor is decisive: a custody-holding member of its
        // own (a laptop that opted in) still finds its hub
        assert_eq!(
            rebuild_home_hub(
                &[e(hub, true, 3), e(member, true, 0)],
                None,
                StoredHubHint::NoAnnounce,
                &[]
            )
            .map(|h| h.node_id),
            Some(hub)
        );
        // two cursors: ambiguous
        assert_eq!(
            rebuild_home_hub(
                &[e(hub, true, 3), e(other, true, 1)],
                None,
                StoredHubHint::NoAnnounce,
                &[]
            ),
            None
        );
        // a salvaged hint wins when it names an enrolled contact
        let hint = HomeHub {
            node_id: hub,
            address: Some("192.168.1.9:11183".parse().unwrap()),
        };
        assert_eq!(
            rebuild_home_hub(
                &[e(hub, false, 0), e(other, true, 0)],
                Some(&hint),
                StoredHubHint::NoAnnounce,
                &[]
            ),
            Some(hint)
        );
        let stranger = HomeHub {
            node_id: NodeId::from([0x77; 32]),
            ..hint
        };
        assert_eq!(
            rebuild_home_hub(&[e(hub, true, 0)], Some(&stranger), says(&hub), &[])
                .map(|h| h.address),
            Some(None)
        );
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
        assert!(!r.invalid && r.hint.is_none() && r.moved_to.is_none() && r.pairs.is_empty());
    }

    fn prefix8(n: &NodeId) -> [u8; 8] {
        let mut p = [0u8; 8];
        p.copy_from_slice(&n.as_bytes()[..8]);
        p
    }

    // ------------------------------------------------------------ Layer 7

    const FAR: [u8; 32] = [0xcd; 32];

    fn spki(fill: u8) -> LinkSpki {
        let mut der = [fill; LINK_SPKI_LEN];
        der[..ED25519_SPKI_PREFIX.len()].copy_from_slice(&ED25519_SPKI_PREFIX);
        LinkSpki::from_der(&der).unwrap()
    }

    fn paired(node: [u8; 32], role: PairRole, via: PairVia) -> PairedHub {
        PairedHub {
            node_id: NodeId::from(node),
            role,
            via,
            ours: true,
            theirs: true,
        }
    }

    fn hubs_file(p: &Persisted) -> serde_json::Value {
        serde_json::from_str(&encode(p)).unwrap()
    }

    /// §19 C7: a deleted or invalid `fabric/settings` never rebuilds a
    /// paired hub as the home hub.
    #[test]
    fn a_paired_hub_is_never_rebuilt_as_the_home_hub() {
        let pair = NodeId::from(FAR);
        let member = NodeId::from([0x01; 32]);
        let e = |node_id, custody, cursor| Enrolled {
            node_id,
            custody,
            cursor,
        };
        // deleted file: a hub whose only enrolled contact is its whole pair
        // (custody role, no cursor, never named in the own announce)
        assert_eq!(
            rebuild_home_hub(&[e(pair, true, 0)], None, StoredHubHint::NoAnnounce, &[]),
            None
        );
        assert_eq!(
            rebuild_home_hub(
                &[e(pair, true, 0), e(member, false, 0)],
                None,
                StoredHubHint::NoAnnounce,
                &[]
            ),
            None
        );
        // M5b F1: a former home hub that became a pair keeps its cursor
        // row for good; with the file deleted (no salvage names the pair)
        // the own announce, which names no hub, is authoritative: nothing
        // is rebuilt, the cursor rule included
        let none = StoredHubHint::NoHub;
        assert_eq!(rebuild_home_hub(&[e(pair, true, 5)], None, none, &[]), None);
        assert_eq!(
            rebuild_home_hub(&[e(pair, true, 5), e(member, false, 0)], None, none, &[]),
            None
        );
        let back = HomeHub {
            node_id: pair,
            address: None,
        };
        assert_eq!(
            rebuild_home_hub(&[e(pair, true, 5)], Some(&back), none, &[]),
            None,
            "a salvaged home_hub against an announce that names none"
        );
        // an announce that names another hub: the pair's cursor does not
        // win, the named hub does (even with two cursors)
        let now = NodeId::from([0x5a; 32]);
        let names_now = StoredHubHint::Hub(prefix8(&now));
        assert_eq!(
            rebuild_home_hub(&[e(pair, true, 5), e(now, true, 2)], None, names_now, &[])
                .map(|h| h.node_id),
            Some(now)
        );
        assert_eq!(
            rebuild_home_hub(&[e(pair, true, 5)], None, names_now, &[]),
            None,
            "the named hub is not enrolled: nothing"
        );
        assert_eq!(
            rebuild_home_hub(
                &[e(pair, true, 5), e(now, true, 0)],
                Some(&back),
                names_now,
                &[]
            )
            .map(|h| h.node_id),
            Some(now),
            "a salvaged hint the announce does not name is ignored"
        );
        // without any stored announce the cursor rule is the only evidence
        // left (the documented fallback)
        assert_eq!(
            rebuild_home_hub(&[e(pair, true, 5)], None, StoredHubHint::NoAnnounce, &[])
                .map(|h| h.node_id),
            Some(pair)
        );
        // invalid file: the salvaged pairs are skipped by every rule, even a
        // (tampered) salvaged home_hub, a stale cursor or an announce hint
        let as_hub = HomeHub {
            node_id: pair,
            address: None,
        };
        let pairs = [pair];
        assert_eq!(
            rebuild_home_hub(
                &[e(pair, true, 0)],
                Some(&as_hub),
                StoredHubHint::NoAnnounce,
                &pairs
            ),
            None
        );
        assert_eq!(
            rebuild_home_hub(&[e(pair, true, 9)], None, StoredHubHint::NoAnnounce, &pairs),
            None
        );
        assert_eq!(
            rebuild_home_hub(
                &[e(pair, true, 0)],
                None,
                StoredHubHint::Hub(prefix8(&pair)),
                &pairs
            ),
            None
        );
        // the salvage reads the pairs of a broken file (only the NodeIds)
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        let hex = id111::encoding::hex_encode(&FAR);
        std::fs::write(
            path(d),
            format!(
                r#"{{"schema":1,"hub_role":"yes","paired_hubs":[{{"node_id":"{hex}","role":"peer","via":"tcp"}},{{"node_id":"zz"}}]}}"#
            ),
        )
        .unwrap();
        let l = load(d, true, true);
        assert_eq!(
            l.persisted,
            Persisted::default(),
            "fail closed: no pair restored"
        );
        assert_eq!(l.recover.unwrap().pairs, vec![pair]);
    }

    /// The Layer 7 "never adds" clause: with no pair the file's bytes are
    /// exactly what a build before Layer 7 wrote (no `paired_hubs` key).
    #[test]
    fn no_pair_writes_the_bytes_a_layer6_build_wrote() {
        let s = Persisted {
            fabric: FabricSettings {
                hub_role: true,
                listen_lan: false,
                custody_opt_in: false,
                home_hub: Some(HomeHub {
                    node_id: NodeId::from([0xab; 32]),
                    address: Some("192.168.1.9:11183".parse().unwrap()),
                }),
                paired_hubs: Vec::new(),
            },
            peers: vec![Peer {
                node_id: NodeId::from([1u8; 32]),
                address: "10.0.0.2:11183".parse().unwrap(),
            }],
            // a daemon half without a fabric record is memory only
            pairs: vec![PairLink::new(NodeId::from(FAR))],
        };
        let want = format!(
            "{{\n  \"schema\": 1,\n  \"hub_role\": true,\n  \"listen_lan\": false,\n  \
             \"custody_opt_in\": false,\n  \"home_hub\": {{\n    \"node_id\": \"{}\",\n    \
             \"address\": \"192.168.1.9:11183\"\n  }},\n  \"peers\": [\n    {{\n      \
             \"node_id\": \"{}\",\n      \"address\": \"10.0.0.2:11183\"\n    }}\n  ]\n}}\n",
            "ab".repeat(32),
            "01".repeat(32)
        );
        assert_eq!(encode(&s), want);
        let empty = Persisted::default();
        assert_eq!(
            encode(&empty),
            "{\n  \"schema\": 1,\n  \"hub_role\": false,\n  \"listen_lan\": false,\n  \
             \"custody_opt_in\": false,\n  \"home_hub\": null,\n  \"peers\": []\n}\n"
        );
        // an explicit empty list reads as no pair and is not written back
        let t = tempfile::tempdir().unwrap();
        std::fs::write(path(t.path()), r#"{"schema":1,"paired_hubs":[]}"#).unwrap();
        let l = load(t.path(), true, true);
        assert_eq!(l.recover, None);
        assert_eq!(encode(&l.persisted), encode(&empty));
    }

    /// §4.6: the row is the fabric's `{node_id, role, via, ours, theirs}`
    /// with the daemon's `address`, `link_spki`, `lan`, `warning_hash`
    /// beside it.
    #[test]
    fn paired_hubs_round_trip_in_the_4_6_shape() {
        let far = NodeId::from(FAR);
        let gw = NodeId::from([0xee; 32]);
        let carrier = NodeId::from([0x0c; 32]);
        let s = Persisted {
            fabric: FabricSettings {
                hub_role: true,
                paired_hubs: vec![
                    paired(FAR, PairRole::Peer, PairVia::Tcp),
                    PairedHub {
                        theirs: false,
                        ..paired([0xee; 32], PairRole::Gateway, PairVia::Tcp)
                    },
                    PairedHub {
                        ours: false,
                        ..paired([0x0c; 32], PairRole::Peer, PairVia::Carrier)
                    },
                ],
                ..FabricSettings::default()
            },
            peers: Vec::new(),
            pairs: vec![
                PairLink {
                    node_id: far,
                    address: Some("203.0.113.7:11183".parse().unwrap()),
                    link_spki: Some(spki(0x5a)),
                    lan: false,
                    warning_hash: None,
                    far_holds_address: false,
                },
                PairLink {
                    node_id: gw,
                    address: Some("[2001:db8::7]:11183".parse().unwrap()),
                    link_spki: Some(spki(0x21)),
                    lan: true,
                    warning_hash: Some([0x9f; 32]),
                    far_holds_address: true,
                },
                PairLink {
                    link_spki: Some(spki(0x33)),
                    ..PairLink::new(carrier)
                },
            ],
        };
        let v = hubs_file(&s);
        let rows = v["paired_hubs"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            serde_json::json!({
                "node_id": "cd".repeat(32), "address": "203.0.113.7:11183", "role": "peer",
                "via": "tcp", "ours": true, "theirs": true, "link_spki": spki(0x5a).text(),
                "lan": false, "warning_hash": null,
            })
        );
        let keys = [
            "node_id",
            "address",
            "role",
            "via",
            "ours",
            "theirs",
            "link_spki",
            "lan",
            "warning_hash",
        ];
        let text = encode(&s);
        let order: Vec<usize> = keys
            .iter()
            .map(|k| text.find(&format!("\"{k}\"")).unwrap())
            .collect();
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "§4.6 key order: {text}"
        );
        assert_eq!(rows[1]["role"], "gateway");
        assert_eq!(rows[1]["ours"], true);
        assert_eq!(rows[1]["theirs"], false, "a half pair: written false");
        assert_eq!(rows[2]["ours"], false);
        assert_eq!(rows[2]["theirs"], true);
        assert_eq!(rows[1]["lan"], true);
        assert_eq!(rows[1]["far_holds_address"], true);
        assert_eq!(rows[1]["warning_hash"], "9f".repeat(32));
        assert_eq!(rows[2]["via"], "carrier");
        assert_eq!(rows[2]["address"], serde_json::Value::Null);
        assert!(
            rows[2].get("far_holds_address").is_none(),
            "written only when true"
        );
        let t = tempfile::tempdir().unwrap();
        save(t.path(), &s).unwrap();
        let l = load(t.path(), false, true);
        assert_eq!(l.recover, None);
        assert_eq!(l.persisted, s);
        // a fabric record without a daemon half is written with nulls and
        // reads back as an empty half
        let mut bare = s.clone();
        bare.pairs.retain(|l| l.node_id != far);
        save(t.path(), &bare).unwrap();
        let back = load(t.path(), false, true).persisted;
        assert_eq!(back.fabric.paired_hubs, s.fabric.paired_hubs);
        assert_eq!(back.pair_link(&far), Some(&PairLink::new(far)));
    }

    #[test]
    fn bad_pair_rows_make_the_file_invalid() {
        let hex = "cd".repeat(32);
        let good = spki(0x5a).text();
        let row = |extra: &str| {
            format!(
                r#"{{"schema":1,"hub_role":true,"paired_hubs":[{{"node_id":"{hex}",{extra}}}]}}"#
            )
        };
        let ok = row(r#""role":"peer","via":"tcp""#);
        let p = decode(&ok, true).unwrap();
        let h = p.fabric.paired_hubs[0];
        assert!(!h.ours && !h.theirs, "the two bools are false when absent");
        let half = row(r#""role":"peer","via":"tcp","theirs":true"#);
        let h = decode(&half, true).unwrap().fabric.paired_hubs[0];
        assert!(!h.ours && h.theirs && !h.whole());
        let mut short = good.clone();
        short.pop();
        let mut not_ed25519 = [0u8; LINK_SPKI_LEN];
        not_ed25519[0] = 0x30;
        for bad in [
            row(r#""role":"boss","via":"tcp""#),
            row(r#""role":"peer","via":"pigeon""#),
            row(r#""role":"peer""#),
            row(r#""role":"leaf","via":"carrier""#),
            row(r#""role":"gateway","via":"carrier""#),
            row(r#""role":"peer","via":"tcp","address":"0.0.0.0:11183""#),
            row(r#""role":"peer","via":"tcp","address":"127.0.0.1:11183""#),
            row(r#""role":"peer","via":"tcp","address":"hub.local""#),
            row(&format!(
                r#""role":"peer","via":"tcp","link_spki":"{short}""#
            )),
            row(&format!(
                r#""role":"peer","via":"tcp","link_spki":"{}""#,
                good.to_lowercase() + "="
            )),
            row(&format!(
                r#""role":"peer","via":"tcp","link_spki":"{}""#,
                card_text(&not_ed25519)
            )),
            row(r#""role":"peer","via":"tcp","warning_hash":"ABC""#),
            row(r#""role":"peer","via":"tcp","ours":"yes""#),
            row(r#""role":"peer","via":"tcp","theirs":1"#),
            row(&format!(
                r#""role":"peer","via":"tcp","warning_hash":"{}""#,
                "AB".repeat(32)
            )),
            r#"{"schema":1,"paired_hubs":[{"node_id":"CD","role":"peer","via":"tcp"}]}"#
                .to_string(),
            format!(
                r#"{{"schema":1,"paired_hubs":[{{"node_id":"{hex}","role":"peer","via":"tcp"}},{{"node_id":"{hex}","role":"leaf","via":"tcp"}}]}}"#
            ),
            format!(
                r#"{{"schema":1,"home_hub":{{"node_id":"{hex}"}},"paired_hubs":[{{"node_id":"{hex}","role":"peer","via":"tcp"}}]}}"#
            ),
        ] {
            assert!(decode(&bad, true).is_err(), "{bad}");
        }
        // either case of the base32 reads as the same key
        let lower = row(&format!(
            r#""role":"peer","via":"tcp","link_spki":"{}""#,
            good.to_lowercase()
        ));
        let p = decode(&lower, true).unwrap();
        assert_eq!(p.pairs[0].link_spki, Some(spki(0x5a)));
        // more rows than MAX_PAIRED_HUBS: the rest are dropped, as for peers
        let many: Vec<String> = (0..=MAX_PAIRED_HUBS)
            .map(|i| {
                format!(
                    r#"{{"node_id":"{}","role":"peer","via":"tcp"}}"#,
                    format!("{i:02x}").repeat(32)
                )
            })
            .collect();
        let text = format!(r#"{{"schema":1,"paired_hubs":[{}]}}"#, many.join(","));
        let p = decode(&text, true).unwrap();
        assert_eq!(p.fabric.paired_hubs.len(), MAX_PAIRED_HUBS);
        assert_eq!(p.pairs.len(), MAX_PAIRED_HUBS);
    }

    #[test]
    fn link_spki_is_44_bytes_of_ed25519_der() {
        let k = spki(0x42);
        assert_eq!(k.as_der().len(), 44);
        assert_eq!(k.raw_key(), &[0x42; 32][..]);
        assert_eq!(k.text().len(), LINK_SPKI_TEXT_LEN);
        assert_eq!(LinkSpki::from_text(&k.text()), Some(k));
        assert_eq!(LinkSpki::from_der(&k.as_der()[..43]), None);
        let mut x509ish = *k.as_der();
        x509ish[9] = 0x71; // another OID
        assert_eq!(LinkSpki::from_der(&x509ish), None);
        assert_eq!(LinkSpki::from_text(&format!("{}-", &k.text()[..70])), None);
        assert!(format!("{k:?}").starts_with("LinkSpki("));
    }

    /// §4.6 step 5: one connection per pair.
    #[test]
    fn the_dial_rule_is_one_connection_per_pair() {
        let low = NodeId::from([0x10; 32]);
        let high = NodeId::from([0xf0; 32]);
        let a_addr: SocketAddr = "10.0.0.1:11183".parse().unwrap();
        let b_addr: SocketAddr = "10.0.0.2:11183".parse().unwrap();
        // what each side holds: (far, its address, does the far hold mine?)
        let side = |far: NodeId, addr: Option<SocketAddr>, far_holds: bool| Persisted {
            fabric: FabricSettings {
                hub_role: true,
                paired_hubs: vec![PairedHub {
                    node_id: far,
                    role: PairRole::Peer,
                    via: PairVia::Tcp,
                    ours: true,
                    theirs: true,
                }],
                ..FabricSettings::default()
            },
            peers: Vec::new(),
            pairs: vec![PairLink {
                address: addr,
                far_holds_address: far_holds,
                ..PairLink::new(far)
            }],
        };
        let dials = |own: &NodeId, p: &Persisted| !pair_dials(own, p).is_empty();
        // both hold an address: the lower NodeId dials, the higher waits
        let at_low = side(high, Some(b_addr), true);
        let at_high = side(low, Some(a_addr), true);
        assert_eq!(pair_dials(&low, &at_low), vec![(high, b_addr)]);
        assert!(!dials(&high, &at_high));
        // one public address: the hub that holds it dials, whatever the order
        let at_high = side(low, Some(a_addr), false); // low showed nothing
        let at_low = side(high, None, true);
        assert!(dials(&high, &at_high));
        assert!(!dials(&low, &at_low));
        // nothing without an address, for a carrier pair, without a fabric
        // record, without the hub role, for this node itself
        assert!(pair_dials(&low, &Persisted::default()).is_empty());
        let mut carrier = side(high, Some(b_addr), false);
        carrier.fabric.paired_hubs[0].via = PairVia::Carrier;
        assert!(!dials(&low, &carrier));
        let mut pending = side(high, Some(b_addr), false);
        pending.fabric.paired_hubs.clear();
        assert!(!dials(&low, &pending));
        let mut off = side(high, Some(b_addr), false);
        off.fabric.hub_role = false;
        assert!(!dials(&low, &off));
        // M5b F8: a half pair (one enrollment recorded, or a row written
        // before the two bools existed) rides the fabric's pairing dial
        for (ours, theirs) in [(true, false), (false, true), (false, false)] {
            let mut half = side(high, Some(b_addr), false);
            half.fabric.paired_hubs[0].ours = ours;
            half.fabric.paired_hubs[0].theirs = theirs;
            assert!(!dials(&low, &half), "{ours} {theirs}");
        }
        assert!(!dials(&high, &side(high, Some(b_addr), false)));
        // the home hub's and the peers' addresses are held
        let mut held = side(high, Some(b_addr), false);
        held.peers.push(Peer {
            node_id: NodeId::from([3; 32]),
            address: a_addr,
        });
        assert!(holds_address(&held, a_addr));
        assert!(!holds_address(&held, b_addr));
        // a row is refused for this node itself and for a non-target address
        let me = PairLink::new(low);
        assert!(pair_link_refusal(&me, &low, true).is_some());
        let lo = PairLink {
            address: Some("127.0.0.1:11183".parse().unwrap()),
            ..PairLink::new(high)
        };
        assert!(pair_link_refusal(&lo, &low, true).is_some());
        assert!(pair_link_refusal(&lo, &low, false).is_none());
        // M5b F7a: an address the home hub, a peer or another pair uses
        let at = |n: NodeId, a: SocketAddr| PairLink {
            address: Some(a),
            ..PairLink::new(n)
        };
        let other = NodeId::from([0x44; 32]);
        assert!(pair_address_refusal(&held, &at(other, a_addr)).is_some());
        let mut homed = side(high, Some(b_addr), false);
        homed.fabric.home_hub = Some(HomeHub {
            node_id: NodeId::from([5; 32]),
            address: Some(a_addr),
        });
        assert!(pair_address_refusal(&homed, &at(other, a_addr)).is_some());
        assert!(pair_address_refusal(&homed, &at(other, b_addr)).is_some());
        assert_eq!(
            pair_address_refusal(&homed, &at(high, b_addr)),
            None,
            "the pair's own address is its own"
        );
        assert_eq!(pair_address_refusal(&homed, &PairLink::new(other)), None);
        let free: SocketAddr = "10.0.0.9:11183".parse().unwrap();
        assert_eq!(pair_address_refusal(&homed, &at(other, free)), None);
    }

    /// M5c S2: an unpairing the pump has not applied yet is folded in: the
    /// dropped record and its half leave, the others stay, an added record
    /// is not taken.
    #[test]
    fn a_pending_unpairing_is_folded_into_the_records() {
        let (b, c) = (NodeId::from(FAR), NodeId::from([0x44; 32]));
        let mut p = Persisted {
            fabric: FabricSettings {
                hub_role: true,
                paired_hubs: vec![
                    paired(FAR, PairRole::Peer, PairVia::Tcp),
                    paired([0x44; 32], PairRole::Peer, PairVia::Tcp),
                ],
                ..FabricSettings::default()
            },
            pairs: vec![PairLink::new(b), PairLink::new(c)],
            ..Persisted::default()
        };
        let current = vec![paired([0x44; 32], PairRole::Peer, PairVia::Tcp)];
        assert!(!fold_dropped_records(
            &mut p.clone(),
            &p.fabric.paired_hubs.clone()
        ));
        assert!(fold_dropped_records(&mut p, &current));
        assert_eq!(p.fabric.paired_hubs, current);
        assert_eq!(p.pairs, vec![PairLink::new(c)]);
        let added = vec![
            paired([0x44; 32], PairRole::Peer, PairVia::Tcp),
            paired([0x55; 32], PairRole::Peer, PairVia::Tcp),
        ];
        assert!(!fold_dropped_records(&mut p, &added));
        assert_eq!(
            p.fabric.paired_hubs, current,
            "an added record waits for its event"
        );
    }

    #[test]
    fn hub_categories() {
        assert!(is_hub_category(Some("Hub4A")));
        assert!(is_hub_category(Some("Hub4B")));
        assert!(!is_hub_category(Some("Node2A")));
        assert!(!is_hub_category(None));
    }
}
