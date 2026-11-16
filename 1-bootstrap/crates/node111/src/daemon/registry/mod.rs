//! Layer 7 (§7.1, §7.4, §7.6, §7.7; row D8): the Service Discovery Registry.
//!
//! - **The record (§7.1).** A second `ServiceInfo`, `_n111._tcp.local.`, on the mdns-sd daemon
//!   the dashboard already runs while LAN-bound ([`Record111`]: instance = the L2 label, host =
//!   the dashboard's mDNS host, SRV = the fabric listener's port, A records only, TXT `v=1`,
//!   `r=<roles>`, `k=<NodeId>`, 78 B). Present only while `discovery.lan_record`, a LAN-bound
//!   dashboard (the announcer runs) and a LAN-bound fabric listener (`hub_role` or `listen_lan`,
//!   bound), and never on Termux (§7.1: no browse or answer there): [`Registry::sync_record`]
//!   compares and registers, updates in place (`register` again: no goodbye) or withdraws. It
//!   runs at [`install`], after a rename of the mDNS host, when D10 sets the switch
//!   ([`Registry::set_lan_record`]) and on the dashboard's
//!   existing 30 s LAN refresh (a fabric settings change shows there; no timer of its own). A
//!   sync with nothing changed costs a few locks and a copy of the own announce's bytes: the
//!   fabric's settings are cloned only after a settings change (the fabric's pair generation)
//!   and the own announce verified only when its bytes changed (a re-mint bumps no
//!   generation); a record the daemon refused is not tried again until one of its inputs
//!   changes. The registry **never starts** the mDNS daemon: without the announcer it holds no daemon and
//!   registers nothing. The goodbye at shutdown ([`Registry::withdraw`]) never misses a record
//!   a sync registers meanwhile.
//! - **The browse (§7.1, §7.7).** On demand only: a human's `POST /registry/browse`, or an
//!   address event on a member whose home hub's redial back-off sits at its 60 s maximum
//!   ([`Registry::on_address_event`], from the dashboard's §3.3 re-arm task). At most
//!   [`BROWSE_FOR`] (5 s), then `stop_browse`, on the blocking pool; one at a time; never on
//!   Termux (no MulticastLock, R01 §3 finding 8). Then the rows are replaced and the SSE
//!   `registry` frame published. A human's call while the lost-hub phase of a browse runs is
//!   queued: one more browse follows that phase, so the frame its 202 promises arrives.
//! - **Rows (§7.4, §7.6).** A hint row per resolved `_n111._tcp` instance ("hint
//!   (unverified)", `answered_by: hint`, `id` = the DNS-SD service instance name, which holds
//!   dots and so never equals a NodeId; with a digest of the name as received when cleaning
//!   changed it, so a lookalike never shadows the instance it imitates: [`hint_id`]), and a signed row for each hint whose `k` names a
//!   contact whose signed announce this node holds ("signed by this contact": `id` = the
//!   NodeId, the roles from that announce, no address). Every string from the network is
//!   untrusted, bounded and stripped of every Unicode control and format character (general
//!   categories Cc, Cf, Zl, Zp); a row never carries an alias, load, battery, position or model
//!   (the label is the L2 label: the claimed instance for a hint, the one derived from the
//!   NodeId for a signed row).
//! - **Lost hub (§7.7).** After a browse on a member with a home hub whose connection is down,
//!   every hint is matched against the hub's NodeId (`k`). A candidate address must be a LAN
//!   address (on-link for one of this node's interfaces, or RFC 1918, RFC 6598, IPv4
//!   link-local, IPv6 ULA, IPv6 link-local with a scope): anything else is dropped before any
//!   dial, and so is the address held and dialled now. One candidate per record (its best A
//!   record); on-link candidates go first, then the ones no earlier browse refused, in random
//!   order; at most [`REHOME_MAX`] are tried with [`FabricHandle::replace_home_hub_address`]
//!   (M5's call: kept only after a handshake there named the hub). Against forged records
//!   naming the hub this bounds the cost, not the outcome: arrival order decides nothing and a
//!   record weighs one candidate whatever its A records, so each browse dials at most
//!   [`REHOME_MAX`] forged addresses and reaches the real record with a chance of about
//!   `REHOME_MAX / (forged records + 1)`; a static flood of up to [`REFUSED_MAX`] addresses is
//!   used up within a few browses, but one that changes its addresses every browse (the
//!   handshake is the only proof) can keep the hub unfound while it lasts. No `/24` sweep;
//!   otherwise the owner rescans the hub's QR.
//!
//! **Never adds:** a listener (the record rides mdns-sd's socket), a timer, a poll. The sealed
//! query (§7.3) stays unbuilt.

use crate::daemon::dashboard::api_v1::ApiState;
use crate::daemon::dashboard::mdns::{BrowseHit, MdnsOps, Record111, SERVICE_111};
use crate::daemon::fabric::FabricHandle;
use id111::encoding::NodeIdDisplay;
use id111::keyset::NodeId;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// `GET /registry` `schema`.
pub const SCHEMA: u32 = 1;

/// The browse's length (§7.1: at most 5 s; a default the owner may change).
pub const BROWSE_FOR: Duration = Duration::from_secs(5);

/// How long one lost-hub candidate address is given to answer (`replace_home_hub_address`).
pub const REHOME_WAIT: Duration = Duration::from_secs(5);

/// At most this many hub-address candidates are tried after one browse (a flood of forged
/// records naming the hub cannot keep the member dialling).
pub const REHOME_MAX: usize = 4;

/// Candidate addresses whose handshake did not name the hub, remembered (most recent last) so
/// a later browse tries the others first; cleared once the hub is found. As large as one
/// browse's candidate space (one per record, at most `BROWSE_MAX_HITS` records).
pub const REFUSED_MAX: usize = 64;

/// At most this many A records of one record become candidates.
pub const A_PER_RECORD: usize = 8;

/// The longest instance label kept from a record (one DNS label).
pub const LABEL_MAX: usize = 63;

/// How long the shutdown goodbye waits for the daemon to send it.
pub const GOODBYE_WAIT: Duration = Duration::from_secs(1);

/// An instance whose goodbye left less than this ago is not registered again by a sync (a
/// later one does: an input change or the 30 s refresh). mdns-sd sends a goodbye twice, the
/// second 120 ms after the first, and reads its own packets back: a registration in between
/// whose TXT or SRV differs from the withdrawn one is probed against its own goodbye and
/// renamed (`<label> (2)`, kept until mdns-sd forgets its names) (row D8 follow-up).
pub const GOODBYE_GUARD: Duration = Duration::from_secs(1);

/// Which layer answered (§7.6; the contract's `RegistryAnswer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// A DNS-SD record: "hint (unverified)".
    Hint,
    /// A role bit of a contact's signed announce: "signed by this contact".
    Signed,
}

impl Answer {
    pub fn as_str(self) -> &'static str {
        match self {
            Answer::Hint => "hint",
            Answer::Signed => "signed",
        }
    }
}

/// One registry row (the contract's `RegistryRow`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Stable within the list (the contract: "the NodeId when known, else the DNS-SD
    /// instance"): a signed row's NodeId; a hint's `k` is unverified, so a hint's id is its
    /// DNS-SD service instance name (`<instance>._n111._tcp.local.`), which holds dots and so
    /// can never equal (or pass for) a NodeId, whatever the record names its instance.
    pub id: String,
    /// The L2 label (claimed by the record for a hint, derived from the NodeId when signed).
    pub label: String,
    /// The TXT `k` (unverified) or the contact's NodeId, 64 lowercase hex.
    pub node_id: Option<String>,
    /// `ip:port` from the record; `None` for a signed row.
    pub address: Option<SocketAddr>,
    /// Role names (`mailbox`, `hub`, `gateway`, `offload`, `origin`).
    pub roles: Vec<&'static str>,
    pub answered_by: Answer,
    /// When it was heard (the browse that resolved it).
    pub heard: Option<Instant>,
}

impl Row {
    /// The contract JSON (`heard_ago_s` against `now`).
    pub fn json(&self, now: Instant) -> Value {
        json!({
            "id": self.id,
            "label": self.label,
            "node_id": self.node_id,
            "address": self.address.map(|a| a.to_string()),
            "roles": self.roles,
            "answered_by": self.answered_by.as_str(),
            "heard_ago_s": self.heard.map(|t| now.saturating_duration_since(t).as_secs()),
        })
    }
}

/// The contract's role names for `roles` (§7.2 name map; `RegistryRow.roles`): bit 0
/// `mailbox`, bit 1 `hub` (mesh111's `always_on_hub`), bits 3–5 `gateway`, `offload`,
/// `origin`. Bit 2 (`lora_attached`, defined and never set) and unknown bits are left out.
pub fn role_names(roles: u16) -> Vec<&'static str> {
    crate::daemon::fabric::sealer::role_names(roles)
        .into_iter()
        .filter_map(|n| match n {
            "always_on_hub" => Some("hub"),
            "mailbox" | "gateway" | "offload" | "origin" => Some(n),
            _ => None,
        })
        .collect()
}

/// Every code point of the Unicode general categories Cc (control), Cf (format: soft hyphen,
/// Arabic letter mark, zero-width and bidi controls, tags …), Zl (line separator) and Zp
/// (paragraph separator), as inclusive ranges. Generated, not hand-picked: Python 3's
/// `unicodedata` (Unicode 15.1.0), `[cp for cp in range(0x110000) if unicodedata.category(chr(cp))
/// in {"Cc", "Cf", "Zl", "Zp"}]` folded into ranges (Unicode 16.0 adds none).
const FORMAT_OR_CONTROL: [(u32, u32); 23] = [
    (0x0000, 0x001F),
    (0x007F, 0x009F),
    (0x00AD, 0x00AD),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x180E, 0x180E),
    (0x200B, 0x200F),
    (0x2028, 0x202E),
    (0x2060, 0x2064),
    (0x2066, 0x206F),
    (0xFEFF, 0xFEFF),
    (0xFFF9, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x13430, 0x1343F),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0001, 0xE0001),
    (0xE0020, 0xE007F),
];

/// Is `c` of general category Cc, Cf, Zl or Zp?
pub fn is_format_or_control(c: char) -> bool {
    let cp = u32::from(c);
    FORMAT_OR_CONTROL
        .binary_search_by(|&(lo, hi)| {
            if hi < cp {
                std::cmp::Ordering::Less
            } else if lo > cp {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Untrusted text from the network made safe to hold: every Cc, Cf, Zl and Zp character
/// dropped, at most `max` characters.
pub fn clean(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !is_format_or_control(*c))
        .take(max)
        .collect()
}

/// 64 hex digits (either case) as lowercase, else `None`.
pub fn parse_k(s: &str) -> Option<String> {
    (s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())).then(|| s.to_ascii_lowercase())
}

/// Exactly 4 hex digits as role bits, else `None`.
pub fn parse_r(s: &str) -> Option<u16> {
    (s.len() == 4 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u16::from_str_radix(s, 16).ok())
        .flatten()
}

/// The TXT value of `key` in `hit` (first match, any case).
fn txt<'a>(hit: &'a BrowseHit, key: &str) -> Option<&'a str> {
    hit.txt
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
}

/// The cleaned instance label of a v1 `_n111._tcp` record, or `None` when it is not one
/// (another version, another type, no instance).
fn v1_instance(hit: &BrowseHit) -> Option<String> {
    let suffix = format!(".{SERVICE_111}");
    let full = hit.fullname.as_str();
    if full.len() <= suffix.len() || !full.to_ascii_lowercase().ends_with(&suffix) {
        return None;
    }
    let instance = clean(full.get(..full.len() - suffix.len())?, LABEL_MAX);
    (!instance.is_empty() && txt(hit, "v") == Some("1")).then_some(instance)
}

/// A hint row's `id`: the DNS-SD service instance name (it holds dots, so never a NodeId).
/// When cleaning changed the instance (a character stripped, the label cut), the id also
/// carries a digest of the instance as received, so a lookalike (`abcd` plus a zero-width
/// space, a long name cut to the same 63 characters) never takes the id of the instance it
/// imitates, nor shadows its row; two lookalikes keep two ids.
pub fn hint_id(instance: &str, raw: &str) -> String {
    use std::hash::{Hash, Hasher};
    if instance == raw {
        return format!("{instance}.{SERVICE_111}");
    }
    let mut h = std::hash::DefaultHasher::new();
    raw.hash(&mut h);
    format!("{instance}~{:016x}.{SERVICE_111}", h.finish())
}

/// A browse hit as a hint row, or `None` when it is not a v1 `_n111._tcp` record. Every
/// field is bounded; the address is the lowest A record (a stable choice) with the SRV port.
pub fn hint_row(hit: &BrowseHit, heard: Instant) -> Option<Row> {
    let instance = v1_instance(hit)?;
    let raw = &hit.fullname[..hit.fullname.len() - SERVICE_111.len() - 1];
    let address = (hit.port != 0)
        .then(|| {
            hit.v4
                .iter()
                .min()
                .map(|ip| SocketAddr::from((*ip, hit.port)))
        })
        .flatten();
    Some(Row {
        id: hint_id(&instance, raw),
        label: instance,
        node_id: txt(hit, "k").and_then(parse_k),
        address,
        roles: txt(hit, "r")
            .and_then(parse_r)
            .map(role_names)
            .unwrap_or_default(),
        answered_by: Answer::Hint,
        heard: Some(heard),
    })
}

/// The hint rows of one browse, minus this node's own record, at most one per instance.
pub fn hint_rows(hits: &[BrowseHit], own: Option<&str>, heard: Instant) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for row in hits.iter().filter_map(|h| hint_row(h, heard)) {
        if own.is_some() && row.node_id.as_deref() == own {
            continue;
        }
        if !rows.iter().any(|r| r.id == row.id) {
            rows.push(row);
        }
    }
    rows
}

// ---------------------------------------------------------------- lost-hub candidates (§7.7)

/// How a lost-hub candidate is reached (the D8 review's ruling 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reach {
    /// Inside the subnet of one of this node's interface addresses.
    OnLink,
    /// A LAN range that is not on-link here: RFC 1918, RFC 6598, IPv4 link-local, IPv6 ULA,
    /// IPv6 link-local with a scope.
    Private,
}

/// Is `ip` inside `net/prefix`? A prefix shorter than /8 (IPv4) or /16 (IPv6) is no LAN and
/// matches nothing; an unknown one (`u8::MAX`) is the host alone.
fn in_net(ip: IpAddr, net: IpAddr, prefix: u8) -> bool {
    match (ip, net.to_canonical()) {
        (IpAddr::V4(a), IpAddr::V4(n)) => {
            let p = if prefix == u8::MAX { 32 } else { prefix };
            if !(8..=32).contains(&p) {
                return false;
            }
            let mask = u32::MAX << (32 - u32::from(p));
            u32::from(a) & mask == u32::from(n) & mask
        }
        (IpAddr::V6(a), IpAddr::V6(n)) => {
            let p = if prefix == u8::MAX { 128 } else { prefix };
            if !(16..=128).contains(&p) {
                return false;
            }
            let mask = u128::MAX << (128 - u32::from(p));
            u128::from(a) & mask == u128::from(n) & mask
        }
        _ => false,
    }
}

/// Whether `addr` is a LAN address a lost-hub candidate may dial (ruling 2), and how: on-link
/// for one of `nets` (this node's interface addresses with their prefix lengths), or a private
/// LAN range; `None` for anything else (a public or documentation address, multicast,
/// broadcast, unspecified, an IPv6 link-local address with no scope), which is never dialled.
/// Loopback is on-link only through the loopback interface's own `127.0.0.1/8`; the fabric
/// refuses it in production anyway ([`FabricHandle::refuse_loopback`]).
pub fn lan_reach(addr: &SocketAddr, nets: &[(IpAddr, u8)]) -> Option<Reach> {
    let ip = addr.ip().to_canonical();
    if addr.port() == 0
        || ip.is_unspecified()
        || ip.is_multicast()
        || matches!(ip, IpAddr::V4(v) if v.is_broadcast())
    {
        return None;
    }
    let v6_link_local = matches!(ip, IpAddr::V6(v) if v.segments()[0] & 0xffc0 == 0xfe80);
    if v6_link_local && !matches!(addr, SocketAddr::V6(s) if s.scope_id() != 0) {
        return None;
    }
    if nets.iter().any(|(n, p)| in_net(ip, *n, *p)) {
        return Some(Reach::OnLink);
    }
    let private = match ip {
        IpAddr::V4(v) => {
            v.is_private() || v.is_link_local() || in_net(ip, IpAddr::from([100, 64, 0, 0]), 10)
        }
        IpAddr::V6(v) => v.segments()[0] & 0xfe00 == 0xfc00 || v6_link_local,
    };
    private.then_some(Reach::Private)
}

/// This node's interface addresses with their prefix lengths (one `getifaddrs(3)` walk; empty
/// when it fails: then only the private ranges qualify).
pub fn local_nets() -> Vec<(IpAddr, u8)> {
    use crate::daemon::relay::netwatch::NetEntry;
    crate::daemon::relay::netwatch::read_interfaces()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| match e {
            NetEntry::Addr {
                family,
                addr,
                prefix,
                ..
            } => {
                let ip = if i32::from(family) == libc::AF_INET {
                    IpAddr::from(<[u8; 4]>::try_from(addr.as_slice()).ok()?)
                } else {
                    IpAddr::from(<[u8; 16]>::try_from(addr.as_slice()).ok()?)
                };
                Some((ip, prefix))
            }
            NetEntry::Link { .. } => None,
        })
        .collect()
}

/// The addresses to try for the home hub after one browse, best first (§7.7; the D8 review's
/// rulings 1 and 2): one per v1 record whose `k` is `hub_k`, with its SRV port: the record's
/// best A record (of its first [`A_PER_RECORD`]: one no earlier browse refused, then on-link,
/// then random), among those not taken by
/// an earlier record, not `current` (the hub's address as held and dialled: trying it again
/// proves nothing), not refused by [`lan_reach`] (nor loopback when `refuse_loopback`). Order:
/// on-link first; within that, addresses not in `refused` (an earlier browse's failures)
/// first; then random, so the order records arrived in decides nothing. The caller tries the
/// first [`REHOME_MAX`].
pub fn hub_candidates(
    hits: &[BrowseHit],
    hub_k: &str,
    current: &[SocketAddr],
    nets: &[(IpAddr, u8)],
    refuse_loopback: bool,
    refused: &[SocketAddr],
) -> Vec<SocketAddr> {
    use std::hash::BuildHasher;
    let mut out: Vec<(Reach, bool, u64, SocketAddr)> = Vec::new();
    let order = std::collections::hash_map::RandomState::new();
    for hit in hits {
        if hit.port == 0 || v1_instance(hit).is_none() {
            continue;
        }
        if txt(hit, "k").and_then(parse_k).as_deref() != Some(hub_k) {
            continue;
        }
        // one candidate per record (its best A record): a record carrying many A records
        // weighs no more than one with a single address
        let best = hit
            .v4
            .iter()
            .take(A_PER_RECORD)
            .map(|ip| SocketAddr::from((*ip, hit.port)))
            .filter(|addr| {
                !current.contains(addr)
                    && !out.iter().any(|c| c.3 == *addr)
                    && !(refuse_loopback && addr.ip().is_loopback())
            })
            .filter_map(|addr| {
                lan_reach(&addr, nets)
                    .map(|reach| (reach, refused.contains(&addr), order.hash_one(addr), addr))
            })
            .min_by_key(|c| (c.1, c.0, c.2));
        out.extend(best);
    }
    out.sort_unstable();
    out.into_iter().map(|c| c.3).collect()
}

// ---------------------------------------------------------------- the record's inputs (§7.1)

/// The record this node should carry now (§7.1), from its inputs; `None` = no record.
#[derive(Debug, Clone, Default)]
pub struct RecordInputs {
    /// `discovery.lan_record`.
    pub lan_record: bool,
    /// The dashboard's mDNS daemon runs (the dashboard is LAN-bound).
    pub daemon: bool,
    /// A Termux node (§7.1: no record there).
    pub termux: bool,
    /// `hub_role || listen_lan` in the fabric settings.
    pub fabric_lan: bool,
    /// The fabric listener's bound address.
    pub listener: Option<SocketAddr>,
    /// This node's NodeId and signed role bits (from its stored, verified announce).
    pub own: Option<([u8; 32], u16)>,
    /// The dashboard's mDNS host (`111.local` or its rename).
    pub host: String,
}

/// §7.1's presence rule: every input must hold.
pub fn wanted(i: &RecordInputs) -> Option<Record111> {
    if !(i.lan_record && i.daemon && i.fabric_lan) || i.termux {
        return None;
    }
    let port = i.listener?.port();
    let (node_id, roles) = i.own?;
    Some(Record111 {
        instance: NodeIdDisplay::new(&node_id).label(),
        host: i.host.trim_end_matches('.').to_string(),
        port,
        roles,
        node_id,
    })
}

/// Why a browse was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// No mDNS daemon runs here (LAN sharing off), or this is Termux: `409 lan_off`.
    LanOff,
}

/// What started a browse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// A human's `POST /registry/browse` ("Look for 111 nodes here", "Find my hub").
    Asked,
    /// An address event while the home hub's redial back-off sat at its maximum (§7.7).
    AddressEvent,
}

/// Where the one browse task is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    /// The DNS-SD browse (≤ 5 s); its rows and frame come at its end.
    Browsing,
    /// The lost-hub tries after the frame was published; `queued` = a human asked meanwhile
    /// (one more browse follows).
    Rehoming {
        queued: bool,
    },
}

/// The registered record (with its full name), a record the daemon refused (not tried again
/// until an input changes), the last goodbye a sync sent (its instance and when:
/// [`GOODBYE_GUARD`]), and whether the shutdown goodbye was sent (no record after it).
#[derive(Default)]
struct RecordState {
    cur: Option<(Record111, String)>,
    refused: Option<Record111>,
    goodbye: Option<(String, Instant)>,
    closed: bool,
}

impl RecordState {
    /// Withdraw `rec` (its full name `full`): the goodbye, remembered for [`GOODBYE_GUARD`].
    fn goodbye(&mut self, mdns: &dyn MdnsOps, rec: &Record111, full: &str) {
        mdns.unregister(full, Duration::ZERO);
        self.goodbye = Some((rec.instance.clone(), Instant::now()));
    }

    /// `rec` would be a new registration of an instance whose goodbye is still in flight.
    fn too_soon(&self, rec: &Record111) -> bool {
        let fresh = self
            .cur
            .as_ref()
            .is_none_or(|(r, _)| !r.instance.eq_ignore_ascii_case(&rec.instance));
        fresh
            && self.goodbye.as_ref().is_some_and(|(instance, at)| {
                instance.eq_ignore_ascii_case(&rec.instance) && at.elapsed() < GOODBYE_GUARD
            })
    }
}

/// The fabric-derived inputs as last read: the settings part kept until the fabric's settings
/// change (its pair generation), the own announce's NodeId and roles until its bytes change, so
/// an idle sync neither clones the settings nor verifies the own announce.
struct FabricInputs {
    generation: u64,
    fabric_lan: bool,
    announce: Vec<u8>,
    own: Option<([u8; 32], u16)>,
}

/// The local-networks reader (production: [`local_nets`]; tests pass their own).
pub type NetsFn = Arc<dyn Fn() -> Vec<(IpAddr, u8)> + Send + Sync>;

/// The registry (one per daemon, in `ApiState` ext as `Arc<Registry>`).
pub struct Registry {
    mdns: Option<Arc<dyn MdnsOps>>,
    termux: bool,
    browse_for: Duration,
    rehome_wait: Duration,
    nets: NetsFn,
    api: Mutex<Weak<ApiState>>,
    fabric: Mutex<Weak<FabricHandle>>,
    host: Mutex<String>,
    lan_record: AtomicBool,
    /// Serialises syncs (held across the fabric reads).
    sync_one: tokio::sync::Mutex<()>,
    /// The record as registered; never held across an await.
    state: Mutex<RecordState>,
    inputs_cache: Mutex<Option<FabricInputs>>,
    rows: Mutex<Vec<Row>>,
    phase: Mutex<Phase>,
    /// Candidate addresses whose handshake did not name the hub ([`REFUSED_MAX`]).
    refused_at: Mutex<VecDeque<SocketAddr>>,
    /// Browses run so far (tests, diagnostics).
    browses: AtomicU32,
    /// Lost-hub candidates tried so far (tests, diagnostics).
    rehome_tries: AtomicU32,
    /// Own-announce verifications and settings reads so far (tests: an idle sync does none).
    fabric_reads: AtomicU32,
}

impl Registry {
    /// A registry over the dashboard's running mDNS daemon (`None` while the dashboard is not
    /// LAN-bound: no record, no browse), its current mDNS host, on a Termux node or not.
    pub fn new(mdns: Option<Arc<dyn MdnsOps>>, host: &str, termux: bool) -> Registry {
        Registry {
            mdns,
            termux,
            browse_for: BROWSE_FOR,
            rehome_wait: REHOME_WAIT,
            nets: Arc::new(local_nets),
            api: Mutex::new(Weak::new()),
            fabric: Mutex::new(Weak::new()),
            host: Mutex::new(host.trim_end_matches('.').to_string()),
            lan_record: AtomicBool::new(false),
            sync_one: tokio::sync::Mutex::new(()),
            state: Mutex::new(RecordState::default()),
            inputs_cache: Mutex::new(None),
            rows: Mutex::new(Vec::new()),
            phase: Mutex::new(Phase::Idle),
            refused_at: Mutex::new(VecDeque::new()),
            browses: AtomicU32::new(0),
            rehome_tries: AtomicU32::new(0),
            fabric_reads: AtomicU32::new(0),
        }
    }

    /// Shorter browse and candidate waits (tests).
    pub fn with_timings(mut self, browse_for: Duration, rehome_wait: Duration) -> Registry {
        self.browse_for = browse_for.min(BROWSE_FOR);
        self.rehome_wait = rehome_wait;
        self
    }

    /// Another local-networks reader (tests).
    pub fn with_nets(mut self, nets: NetsFn) -> Registry {
        self.nets = nets;
        self
    }

    /// The API state the `registry` frame is published on.
    pub fn bind_api(&self, api: &Arc<ApiState>) {
        *lock(&self.api) = Arc::downgrade(api);
    }

    /// The fabric (the own NodeId and roles, the listener, contacts, the home hub).
    pub fn bind_fabric(&self, fabric: &Arc<FabricHandle>) {
        *lock(&self.fabric) = Arc::downgrade(fabric);
        *lock(&self.inputs_cache) = None;
    }

    /// An mDNS daemon runs here (the dashboard is LAN-bound).
    pub fn has_daemon(&self) -> bool {
        self.mdns.is_some()
    }

    /// The mDNS host was renamed (a conflict): the record follows at the next sync.
    pub fn set_host(&self, host: &str) {
        *lock(&self.host) = host.trim_end_matches('.').to_string();
    }

    /// `discovery.lan_record` (D10 calls this after a settings write, then
    /// [`Registry::sync_record`]).
    pub fn set_lan_record(&self, on: bool) {
        self.lan_record.store(on, Ordering::SeqCst);
    }

    /// The record registered now, if any.
    pub async fn record(&self) -> Option<Record111> {
        lock(&self.state).cur.as_ref().map(|(r, _)| r.clone())
    }

    /// Browses run so far.
    pub fn browses(&self) -> u32 {
        self.browses.load(Ordering::SeqCst)
    }

    /// Lost-hub candidates tried so far.
    pub fn rehome_tries(&self) -> u32 {
        self.rehome_tries.load(Ordering::SeqCst)
    }

    /// Settings reads and own-announce verifications so far.
    pub fn fabric_reads(&self) -> u32 {
        self.fabric_reads.load(Ordering::SeqCst)
    }

    /// A browse task runs now (its browse or its lost-hub phase).
    pub fn browsing(&self) -> bool {
        *lock(&self.phase) != Phase::Idle
    }

    /// The running browse task is in its lost-hub phase (its frame is out).
    pub fn rehoming(&self) -> bool {
        matches!(*lock(&self.phase), Phase::Rehoming { .. })
    }

    fn fabric(&self) -> Option<Arc<FabricHandle>> {
        lock(&self.fabric).upgrade()
    }

    /// This node's NodeId and signed role bits, from its stored announce.
    fn own_of(announce: &[u8]) -> Option<([u8; 32], u16)> {
        let ann = mesh111::link::tcp_lan::verify_announce(announce)?;
        let roles = mesh111::contacts::AppData::parse(ann.appdata()).roles;
        Some((*ann.node_id().as_bytes(), roles))
    }

    /// The fabric-derived inputs. `fabric_lan` is read again (a settings clone) only after a
    /// settings change (the fabric's pair generation, which every settings write bumps); the own
    /// announce is compared on every sync (one lock and a copy of its bytes) and verified only
    /// when its bytes changed, since it is re-minted with no generation bump (the relay switches,
    /// `FabricHandle::set_relay_roles`; a restore-pending announce minted later; the settings
    /// pump's re-mint, which lands after its bump).
    fn fabric_inputs(&self, fh: &FabricHandle) -> (bool, Option<([u8; 32], u16)>) {
        let generation = fh.pair_generation();
        let announce = fh.own_announce();
        let mut cache = lock(&self.inputs_cache);
        let fabric_lan = match cache.as_ref().filter(|c| c.generation == generation) {
            Some(c) => c.fabric_lan,
            None => {
                self.fabric_reads.fetch_add(1, Ordering::SeqCst);
                let p = fh.persisted();
                p.fabric.hub_role || p.fabric.listen_lan
            }
        };
        let own = match cache.as_ref() {
            Some(c) if c.announce == announce => c.own,
            _ => {
                self.fabric_reads.fetch_add(1, Ordering::SeqCst);
                Self::own_of(&announce)
            }
        };
        *cache = Some(FabricInputs {
            generation,
            fabric_lan,
            announce,
            own,
        });
        (fabric_lan, own)
    }

    /// This node's NodeId and roles (cached as above).
    fn own(&self, fh: &FabricHandle) -> Option<([u8; 32], u16)> {
        self.fabric_inputs(fh).1
    }

    /// The §7.1 inputs as they stand.
    pub async fn inputs(&self) -> RecordInputs {
        let host = lock(&self.host).clone();
        let mut i = RecordInputs {
            lan_record: self.lan_record.load(Ordering::SeqCst),
            daemon: self.mdns.is_some(),
            termux: self.termux,
            host,
            ..RecordInputs::default()
        };
        if let Some(fh) = self.fabric() {
            let (fabric_lan, own) = self.fabric_inputs(&fh);
            i.fabric_lan = fabric_lan;
            i.own = own;
            if i.fabric_lan {
                i.listener = fh.listen_addr().await;
            }
        }
        i
    }

    /// Register, update or withdraw the `_n111._tcp` record so it matches §7.1's rule now.
    /// Only on the daemon the dashboard runs, never on Termux; nothing is started here. A
    /// record the daemon refused is not tried again until one of its inputs changes.
    ///
    /// A registered record whose roles, port or host change is registered again under the
    /// same full name: mdns-sd's documented update, with no unregister first (the changed
    /// records are probed again: about a second unanswered). A goodbye is sent only when the
    /// record goes (withdrawn, refused, or its full name changed), and an instance whose
    /// goodbye left less than [`GOODBYE_GUARD`] ago is registered by a later sync, not this
    /// one. An unregister before each update was the G8 phase 12 busy loop (row D8 follow-up):
    /// the daemon reads its own goodbye back (multicast loop and `lo`; TTL 0 read as 1; the
    /// resend 120 ms later leaves on whichever interface the socket last named) as a conflict
    /// with the probe the new record starts, renames the instance (`<label> (2)`) or the host
    /// (`111-N.local`), and every host rename came back here as one more update. After a host
    /// rename the dashboard's announcer has already registered its own service under the new
    /// name, so the record's A records join that service's probe of the name
    /// (`dashboard::mdns::follow_host`).
    pub async fn sync_record(&self) {
        let Some(mdns) = self.mdns.as_ref() else {
            return;
        };
        if self.termux {
            return;
        }
        let _one = self.sync_one.lock().await;
        {
            let st = lock(&self.state);
            if st.closed || (st.cur.is_none() && !self.lan_record.load(Ordering::SeqCst)) {
                return; // the switch is off and nothing is registered: no fabric read
            }
        }
        let want = wanted(&self.inputs().await);
        let mut st = lock(&self.state);
        if st.closed || st.cur.as_ref().map(|(r, _)| r) == want.as_ref() {
            return;
        }
        if want.is_some() && st.refused == want {
            return; // refused as it stands: wait for an input to change
        }
        if want.as_ref().is_some_and(|rec| st.too_soon(rec)) {
            return; // its goodbye is still in flight: a later sync registers it
        }
        st.refused = None;
        let old = st.cur.take();
        let Some(rec) = want else {
            if let Some((gone, full)) = old {
                st.goodbye(&**mdns, &gone, &full); // withdrawn
            }
            return;
        };
        match mdns.register(&rec) {
            Ok(full) => {
                // the same full name was updated in place; another one leaves with a goodbye
                if let Some((gone, f)) = old.filter(|(_, f)| !f.eq_ignore_ascii_case(&full)) {
                    st.goodbye(&**mdns, &gone, &f);
                }
                st.cur = Some((rec, full));
            }
            Err(e) => {
                if let Some((gone, f)) = old {
                    st.goodbye(&**mdns, &gone, &f);
                }
                eprintln!("111 daemon: the _n111._tcp record could not be registered: {e}");
                st.refused = Some(rec);
            }
        }
    }

    /// Withdraw the record (daemon shutdown; the announcer's own stop follows): the goodbye
    /// waits at most [`GOODBYE_WAIT`] for the daemon to send it, and no sync registers after
    /// it (one in flight finishes first and its record is the one withdrawn).
    pub fn withdraw(&self) {
        let Some(mdns) = self.mdns.as_ref() else {
            return;
        };
        let mut st = lock(&self.state);
        st.closed = true;
        if let Some((_, full)) = st.cur.take() {
            mdns.unregister(&full, GOODBYE_WAIT);
        }
    }

    /// The rows now (`GET /registry`, the SSE frame).
    pub fn rows_json(&self) -> Vec<Value> {
        let now = Instant::now();
        lock(&self.rows).iter().map(|r| r.json(now)).collect()
    }

    /// Start a browse (§7.1): `Ok(true)` started; `Ok(false)` a browse task runs already and
    /// a `registry` frame will still arrive (its browse is under way, or, in its lost-hub
    /// phase, a human's call queues one more browse after it); `Err(LanOff)` where no mDNS
    /// daemon runs or on Termux. Must run inside a tokio runtime.
    pub fn browse(self: &Arc<Self>, why: Why) -> Result<bool, Refused> {
        if self.mdns.is_none() || self.termux {
            return Err(Refused::LanOff);
        }
        {
            let mut p = lock(&self.phase);
            match *p {
                Phase::Idle => *p = Phase::Browsing,
                Phase::Browsing => return Ok(false),
                Phase::Rehoming { .. } => {
                    if why == Why::Asked {
                        *p = Phase::Rehoming { queued: true };
                    }
                    return Ok(false);
                }
            }
        }
        let this = Arc::clone(self);
        tokio::spawn(async move {
            // back to idle however the task ends (a panic in it included)
            struct Done<'a>(&'a Mutex<Phase>);
            impl Drop for Done<'_> {
                fn drop(&mut self) {
                    *lock(self.0) = Phase::Idle;
                }
            }
            let _done = Done(&this.phase);
            let mut why = why;
            loop {
                let (hits, fh) = this.run_browse().await;
                *lock(&this.phase) = Phase::Rehoming { queued: false };
                if let Some(fh) = fh {
                    this.rehome(&fh, &hits, why).await;
                }
                let mut p = lock(&this.phase);
                if *p == (Phase::Rehoming { queued: true }) {
                    *p = Phase::Browsing;
                    why = Why::Asked;
                    continue;
                }
                *p = Phase::Idle;
                break;
            }
        });
        Ok(true)
    }

    /// One browse: the hits, the rows replaced and the `registry` frame published.
    async fn run_browse(&self) -> (Vec<BrowseHit>, Option<Arc<FabricHandle>>) {
        let Some(mdns) = self.mdns.clone() else {
            return (Vec::new(), None);
        };
        self.browses.fetch_add(1, Ordering::SeqCst);
        let limit = self.browse_for;
        let hits = tokio::task::spawn_blocking(move || mdns.browse(SERVICE_111, limit)).await;
        let hits = match hits {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => {
                eprintln!("111 daemon: registry browse failed: {e}");
                Vec::new()
            }
            Err(_) => Vec::new(),
        };
        let heard = Instant::now();
        let fh = self.fabric();
        let own = fh
            .as_deref()
            .and_then(|h| self.own(h))
            .map(|(id, _)| hex(&id));
        let mut rows = hint_rows(&hits, own.as_deref(), heard);
        if let Some(f) = fh.as_deref().and_then(|h| h.fabric()) {
            let mut signed = Vec::new();
            for k in rows.iter().filter_map(|r| r.node_id.clone()) {
                if signed.iter().any(|r: &Row| r.id == k) {
                    continue;
                }
                let Some(id) = node_id(&k) else { continue };
                if let Ok(c) = f.contact(id).await {
                    if c.has_key {
                        signed.push(Row {
                            id: k.clone(),
                            label: NodeIdDisplay::new(id.as_bytes()).label(),
                            node_id: Some(k),
                            address: None,
                            roles: role_names(c.roles),
                            answered_by: Answer::Signed,
                            heard: None,
                        });
                    }
                }
            }
            rows.extend(signed);
        }
        *lock(&self.rows) = rows;
        if let Some(api) = lock(&self.api).upgrade() {
            api.events
                .publish("registry", &json!({ "rows": self.rows_json() }));
        }
        (hits, fh)
    }

    /// §7.7: on a member whose home hub is not connected, try the best [`REHOME_MAX`]
    /// candidates of [`hub_candidates`]; an address is kept only after a handshake there named
    /// the hub ([`FabricHandle::replace_home_hub_address`]), and success is reported only once
    /// the fabric shows the hub connected.
    async fn rehome(&self, fh: &FabricHandle, hits: &[BrowseHit], why: Why) {
        let Some(f) = fh.fabric() else { return };
        let Ok(st) = f.status().await else { return };
        let (Some(hub), false) = (st.hub.node_id, st.hub.connected) else {
            return;
        };
        // the address held and dialled: the shown override when the daemon holds one for this
        // hub, else the fabric's own copy (`replace_home_hub_address` answers `Ok(true)` with no
        // handshake for exactly this one). After a rehome the fabric's copy is the old,
        // released address and stays a candidate: the hub may come back there.
        let held = match fh.persisted().fabric.home_hub.filter(|h| h.node_id == hub) {
            Some(h) => h.address,
            None => st.hub.address,
        };
        let current: Vec<SocketAddr> = held.into_iter().collect();
        let nets = (self.nets)();
        let refused: Vec<SocketAddr> = lock(&self.refused_at).iter().copied().collect();
        let list = hub_candidates(
            hits,
            &hex(hub.as_bytes()),
            &current,
            &nets,
            fh.refuse_loopback(),
            &refused,
        );
        for addr in list.into_iter().take(REHOME_MAX) {
            self.rehome_tries.fetch_add(1, Ordering::SeqCst);
            match fh.replace_home_hub_address(addr, self.rehome_wait).await {
                Ok(true) if f.status().await.is_ok_and(|s| s.hub.connected) => {
                    eprintln!(
                        "111 daemon: home hub found at {addr} ({}): its handshake named it",
                        match why {
                            Why::Asked => "Find my hub",
                            Why::AddressEvent => "after an address change",
                        }
                    );
                    lock(&self.refused_at).clear();
                    return;
                }
                Ok(true) => {
                    eprintln!("111 daemon: home hub candidate {addr} taken, but no handshake shows the hub connected yet");
                }
                Ok(false) => self.remember_refused(addr),
                Err(e) => {
                    eprintln!("111 daemon: home hub candidate {addr} refused: {e}");
                    self.remember_refused(addr);
                }
            }
        }
    }

    fn remember_refused(&self, addr: SocketAddr) {
        let mut r = lock(&self.refused_at);
        r.retain(|a| *a != addr);
        if r.len() == REFUSED_MAX {
            r.pop_front();
        }
        r.push_back(addr);
    }

    /// An address event from the dashboard's §3.3 re-arm (§7.7): on a member (not Termux, the
    /// dashboard LAN-bound) whose home hub is down and whose redial back-off sits at its
    /// maximum, start a browse with no call. A DHCP renew with the hub up costs one status read.
    pub fn on_address_event(self: &Arc<Self>) {
        if self.mdns.is_none() || self.termux || self.browsing() {
            return;
        }
        let this = Arc::clone(self);
        tokio::spawn(async move {
            if this.hub_lost().await {
                let _ = this.browse(Why::AddressEvent);
            }
        });
    }

    /// The home hub is enrolled, not connected, and its redial back-off reached its maximum.
    pub async fn hub_lost(&self) -> bool {
        let Some(fh) = self.fabric() else {
            return false;
        };
        let (Some(f), Some(tcp)) = (fh.fabric(), fh.tcp()) else {
            return false;
        };
        let Ok(st) = f.status().await else {
            return false;
        };
        if !st.hub.enrolled || st.hub.connected {
            return false;
        }
        let shown = fh.persisted().fabric.home_hub.and_then(|h| h.address);
        [st.hub.address, shown]
            .into_iter()
            .flatten()
            .any(|a| tcp.redial_at_max(a))
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn node_id(k: &str) -> Option<NodeId> {
    let mut b = [0u8; 32];
    for (i, out) in b.iter_mut().enumerate() {
        *out = u8::from_str_radix(k.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(NodeId::from(b))
}

/// The daemon's registry (from ext), if the dashboard installed one.
pub fn get(api: &ApiState) -> Option<Arc<Registry>> {
    api.ext::<Arc<Registry>>()
}

/// Bind the fabric and the `discovery.lan_record` switch, then sync the record once (called
/// from `relay::start` after the fabric runs). Reads `state/relay.json` only where an mDNS
/// daemon runs; spawns one short sync, nothing resident.
pub fn install(api: &Arc<ApiState>, fabric: &Arc<FabricHandle>) {
    let Some(reg) = get(api) else { return };
    reg.bind_fabric(fabric);
    if !reg.has_daemon() {
        return;
    }
    let loaded = crate::daemon::relay::settings::load(&api.layout.state_dir());
    reg.set_lan_record(loaded.settings.lan_record());
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::spawn(async move { reg.sync_record().await });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn hit(instance: &str, txt: &[(&str, &str)], ip: [u8; 4]) -> BrowseHit {
        BrowseHit {
            fullname: format!("{instance}.{SERVICE_111}"),
            port: 11183,
            v4: vec![Ipv4Addr::from(ip)],
            txt: txt
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn a_v1_record_becomes_a_hint_row() {
        let k = "AB".repeat(32);
        let now = Instant::now();
        let row = hint_row(
            &hit(
                "abcd-efgh-ij",
                &[("v", "1"), ("r", "0003"), ("k", &k)],
                [192, 168, 1, 7],
            ),
            now,
        )
        .unwrap();
        assert_eq!(row.id, format!("abcd-efgh-ij.{SERVICE_111}"));
        assert_eq!(row.label, "abcd-efgh-ij");
        assert_eq!(row.node_id.as_deref(), Some("ab".repeat(32).as_str()));
        assert_eq!(row.address, Some("192.168.1.7:11183".parse().unwrap()));
        assert_eq!(row.roles, vec!["mailbox", "hub"]);
        assert_eq!(row.answered_by, Answer::Hint);
        let v = row.json(now);
        assert_eq!(v["answered_by"], "hint");
        assert_eq!(v["heard_ago_s"], 0);
        let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "address",
                "answered_by",
                "heard_ago_s",
                "id",
                "label",
                "node_id",
                "roles"
            ],
            "the contract's seven keys, nothing else (§7.4)"
        );
    }

    #[test]
    fn untrusted_fields_are_checked_and_bounded() {
        let now = Instant::now();
        // another version, another type: no row
        assert!(hint_row(&hit("x", &[("v", "2")], [10, 0, 0, 1]), now).is_none());
        assert!(hint_row(&hit("x", &[], [10, 0, 0, 1]), now).is_none());
        let mut other = hit("x", &[("v", "1")], [10, 0, 0, 1]);
        other.fullname = "x._http._tcp.local.".into();
        assert!(hint_row(&other, now).is_none());
        // bad k, bad r: kept as unknown, never guessed
        let row = hint_row(
            &hit("x", &[("v", "1"), ("r", "zz"), ("k", "12")], [10, 0, 0, 1]),
            now,
        )
        .unwrap();
        assert_eq!(row.node_id, None);
        assert!(row.roles.is_empty());
        // control characters and bidi overrides dropped, length bounded
        let long = format!("a\u{202e}b\u{7}{}", "c".repeat(100));
        let row = hint_row(&hit(&long, &[("v", "1")], [10, 0, 0, 1]), now).unwrap();
        assert!(row.label.starts_with("abc"));
        assert_eq!(row.label.chars().count(), LABEL_MAX);
        // no A record or port 0: no address
        let mut none = hit("y", &[("v", "1")], [10, 0, 0, 1]);
        none.v4.clear();
        assert_eq!(hint_row(&none, now).unwrap().address, None);
        // unknown and bit-2 roles are never shown
        assert_eq!(
            role_names(0x003f | 0x8000),
            vec!["mailbox", "hub", "gateway", "offload", "origin"]
        );
    }

    #[test]
    fn the_own_record_and_duplicates_are_dropped() {
        let me = "11".repeat(32);
        let them = "22".repeat(32);
        let hits = [
            hit("me", &[("v", "1"), ("k", &me)], [10, 0, 0, 1]),
            hit("them", &[("v", "1"), ("k", &them)], [10, 0, 0, 2]),
            hit("them", &[("v", "1"), ("k", &them)], [10, 0, 0, 3]),
        ];
        let rows = hint_rows(&hits, Some(&me), Instant::now());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "them");
    }

    #[test]
    fn the_record_needs_every_input() {
        let full = RecordInputs {
            lan_record: true,
            daemon: true,
            termux: false,
            fabric_lan: true,
            listener: Some("0.0.0.0:11183".parse().unwrap()),
            own: Some(([7; 32], 0x0002)),
            host: "111-2.local.".into(),
        };
        let rec = wanted(&full).unwrap();
        assert_eq!(rec.port, 11183);
        assert_eq!(rec.host, "111-2.local");
        assert_eq!(rec.instance, NodeIdDisplay::new(&[7; 32]).label());
        assert_eq!(rec.txt()[1].1, "0002");
        for off in 0..6 {
            let mut i = full.clone();
            match off {
                0 => i.lan_record = false,
                1 => i.daemon = false,
                2 => i.fabric_lan = false,
                3 => i.listener = None,
                4 => i.termux = true,
                _ => i.own = None,
            }
            assert!(wanted(&i).is_none(), "input {off}");
        }
    }

    /// Ruling 7: every character of general category Cc, Cf, Zl or Zp is dropped (not a hand
    /// list): U+2028/2029, U+061C, the soft hyphen, zero-width joiners, bidi controls, tag
    /// characters; letters, marks, spaces and emoji stay. The table is sorted and disjoint.
    #[test]
    fn clean_drops_every_control_and_format_character() {
        for w in FORMAT_OR_CONTROL.windows(2) {
            assert!(w[0].0 <= w[0].1 && w[0].1 < w[1].0, "{w:?}");
        }
        let dropped = [
            '\u{0}',
            '\u{1f}',
            '\u{7f}',
            '\u{85}',
            '\u{ad}',
            '\u{600}',
            '\u{61c}',
            '\u{6dd}',
            '\u{180e}',
            '\u{200b}',
            '\u{200d}',
            '\u{200e}',
            '\u{2028}',
            '\u{2029}',
            '\u{202e}',
            '\u{2060}',
            '\u{2066}',
            '\u{2069}',
            '\u{206f}',
            '\u{feff}',
            '\u{fff9}',
            '\u{110bd}',
            '\u{13430}',
            '\u{1bca0}',
            '\u{1d173}',
            '\u{e0001}',
            '\u{e0041}',
            '\u{e007f}',
        ];
        for c in dropped {
            assert!(is_format_or_control(c), "U+{:04X}", u32::from(c));
            assert_eq!(clean(&format!("a{c}b"), 10), "ab", "U+{:04X}", u32::from(c));
        }
        for c in [
            'a', 'é', '\u{301}', ' ', '\u{a0}', '-', '😀', '\u{2027}', '\u{202f}', '\u{e000}',
        ] {
            assert!(!is_format_or_control(c), "U+{:04X}", u32::from(c));
        }
        assert_eq!(clean("tag\u{e0074}\u{e0061}s", 10), "tags");
    }

    /// Ruling 6: a hint's id is its DNS-SD service instance name, never a NodeId, so a forged
    /// instance named like a contact's NodeId cannot collide with that contact's signed row.
    #[test]
    fn a_hint_s_id_can_never_equal_a_node_id() {
        let k = "c3".repeat(32);
        let row = hint_row(
            &hit(&k, &[("v", "1"), ("k", &k)], [10, 0, 0, 9]),
            Instant::now(),
        )
        .unwrap();
        assert_eq!(
            row.label,
            k[..LABEL_MAX],
            "the claimed label is shown as text, bounded"
        );
        assert_ne!(row.id, k);
        assert!(
            row.id.starts_with(&format!("{}~", &k[..LABEL_MAX])),
            "{}",
            row.id
        );
        assert!(row.id.ends_with(&format!(".{SERVICE_111}")));
        assert!(
            parse_k(&row.id).is_none(),
            "an id with dots is never 64 hex"
        );
        assert_eq!(row.answered_by, Answer::Hint);
    }

    /// A lookalike instance (the real label plus a zero-width space, or cut to the same 63
    /// characters) never takes the real hint's id nor drops its row, wherever it sits in the
    /// hits.
    #[test]
    fn a_lookalike_hint_never_shadows_the_instance_it_imitates() {
        let (real_k, fake_k) = ("a1".repeat(32), "f0".repeat(32));
        let long = "x".repeat(70);
        let real = hit("abcd-efgh-ij", &[("v", "1"), ("k", &real_k)], [10, 0, 0, 1]);
        let fake = hit(
            "abcd-efgh-ij\u{200b}",
            &[("v", "1"), ("k", &fake_k)],
            [10, 0, 0, 2],
        );
        let long_a = hit(
            &format!("{long}a"),
            &[("v", "1"), ("k", &fake_k)],
            [10, 0, 0, 3],
        );
        let long_b = hit(
            &format!("{long}b"),
            &[("v", "1"), ("k", &fake_k)],
            [10, 0, 0, 4],
        );
        for hits in [
            vec![fake.clone(), real.clone(), long_a.clone(), long_b.clone()],
            vec![real.clone(), fake.clone(), long_b.clone(), long_a.clone()],
        ] {
            let rows = hint_rows(&hits, None, Instant::now());
            assert_eq!(rows.len(), 4, "every instance keeps its row");
            let real_row = rows
                .iter()
                .find(|r| r.id == format!("abcd-efgh-ij.{SERVICE_111}"))
                .unwrap();
            assert_eq!(real_row.node_id.as_deref(), Some(real_k.as_str()));
            let mut ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), 4, "{ids:?}");
        }
    }

    fn sa(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    /// Ruling 2: only LAN addresses are candidates.
    #[test]
    fn a_candidate_must_be_a_lan_address() {
        let nets: Vec<(IpAddr, u8)> = vec![
            ("203.0.113.7".parse().unwrap(), 24), // a public LAN: on-link
            ("127.0.0.1".parse().unwrap(), 8),
            ("0.0.0.0".parse().unwrap(), 0), // a /0 matches nothing
        ];
        let reach = |s: &str| lan_reach(&sa(s), &nets);
        assert_eq!(reach("203.0.113.99:11183"), Some(Reach::OnLink));
        assert_eq!(reach("127.0.0.1:11183"), Some(Reach::OnLink));
        for private in [
            "10.1.2.3:1",
            "172.16.0.1:1",
            "172.31.255.254:1",
            "192.168.1.7:1",
            "100.64.0.1:1",
            "100.127.255.1:1",
            "169.254.3.4:1",
            "[fd00::1]:1",
            "[fc12::1]:1",
            "[fe80::1%2]:1",
        ] {
            assert_eq!(reach(private), Some(Reach::Private), "{private}");
        }
        for never in [
            "8.8.8.8:11183",
            "1.1.1.1:443",
            "192.0.2.1:1",
            "172.32.0.1:1",
            "100.128.0.1:1",
            "224.0.0.251:5353",
            "255.255.255.255:1",
            "0.0.0.0:1",
            "10.0.0.1:0",
            "[2001:db8::1]:1",
            "[2606:4700::1111]:1",
            "[fe80::1]:1", // link-local with no scope
            "[ff02::fb]:5353",
        ] {
            assert_eq!(reach(never), None, "{never}");
        }
        // a v4-mapped address is judged as its v4 address
        assert_eq!(reach("[::ffff:192.168.1.7]:1"), Some(Reach::Private));
        assert_eq!(reach("[::ffff:8.8.8.8]:1"), None);
    }

    /// Rulings 1 and 2: the candidates for the hub's `k`, minus the address held now, minus
    /// non-LAN addresses, deduplicated; on-link first, earlier failures last.
    #[test]
    fn the_hub_candidates_are_ordered_and_filtered() {
        let hub = "ab".repeat(32);
        let rec = |name: &str, ips: &[[u8; 4]]| BrowseHit {
            fullname: format!("{name}.{SERVICE_111}"),
            port: 11183,
            v4: ips.iter().map(|ip| Ipv4Addr::from(*ip)).collect(),
            txt: vec![("v".into(), "1".into()), ("k".into(), hub.clone())],
        };
        let mut other = rec("other", &[[192, 168, 1, 50]]);
        other.txt[1].1 = "cd".repeat(32);
        let hits = vec![
            rec("held", &[[192, 168, 1, 10]]), // the address held now
            rec("public", &[[8, 8, 8, 8], [192, 0, 2, 1]]), // never dialled
            rec("far", &[[10, 9, 9, 9]]),      // private, not on-link
            rec("near", &[[192, 168, 1, 20], [192, 168, 1, 20]]),
            rec("near-again", &[[192, 168, 1, 20]]), // the same address: once
            rec("tried", &[[192, 168, 1, 30]]),      // an earlier browse's failure
            other,                                   // another node's k
            rec("loop", &[[127, 0, 0, 1]]),
        ];
        let nets: Vec<(IpAddr, u8)> = vec![("192.168.1.5".parse().unwrap(), 24)];
        let current = [sa("192.168.1.10:11183")];
        let refused = [sa("192.168.1.30:11183")];
        let got = hub_candidates(&hits, &hub, &current, &nets, true, &refused);
        assert_eq!(
            got,
            vec![
                sa("192.168.1.20:11183"),
                sa("192.168.1.30:11183"),
                sa("10.9.9.9:11183"),
            ]
        );
        // loopback only where the fabric allows it (tests): then on-link through lo
        let lo: Vec<(IpAddr, u8)> = vec![("127.0.0.1".parse().unwrap(), 8)];
        let got = hub_candidates(&hits, &hub, &current, &lo, false, &[]);
        assert_eq!(got[0], sa("127.0.0.1:11183"));
        assert!(!got.contains(&sa("8.8.8.8:11183")) && !got.contains(&sa("192.0.2.1:11183")));
    }

    /// One candidate per record (a forger's eight A records weigh as one), its best address:
    /// not refused over refused, then on-link over private; a record whose addresses are all held,
    /// taken or refused by `lan_reach` gives none.
    #[test]
    fn a_record_gives_one_candidate_its_best_address() {
        let hub = "ab".repeat(32);
        let rec = |name: &str, ips: &[[u8; 4]]| BrowseHit {
            fullname: format!("{name}.{SERVICE_111}"),
            port: 11183,
            v4: ips.iter().map(|ip| Ipv4Addr::from(*ip)).collect(),
            txt: vec![("v".into(), "1".into()), ("k".into(), hub.clone())],
        };
        let many: Vec<[u8; 4]> = (100..108u8).map(|i| [192, 168, 1, i]).collect();
        let hits = vec![
            rec("forged", &many),
            rec("two-homed", &[[10, 9, 9, 9], [192, 168, 1, 40]]),
            rec("held", &[[192, 168, 1, 10], [8, 8, 8, 8]]),
        ];
        let nets: Vec<(IpAddr, u8)> = vec![("192.168.1.5".parse().unwrap(), 24)];
        let current = [sa("192.168.1.10:11183")];
        let got = hub_candidates(&hits, &hub, &current, &nets, true, &[]);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(
            got.contains(&sa("192.168.1.40:11183")),
            "on-link over private"
        );
        assert!(many
            .iter()
            .any(|ip| got.contains(&SocketAddr::from((*ip, 11183)))));
        // a refused address yields to the record's next one
        let refused = [sa("192.168.1.40:11183")];
        let got = hub_candidates(&hits, &hub, &current, &nets, true, &refused);
        assert!(got.contains(&sa("10.9.9.9:11183")), "{got:?}");
    }

    /// Ruling 1: among candidates of one rank arrival order decides nothing (a flood heard
    /// first does not always fill the [`REHOME_MAX`] tries).
    #[test]
    fn equal_candidates_are_tried_in_random_order() {
        let hub = "ab".repeat(32);
        let hits: Vec<BrowseHit> = (1..=12u8)
            .map(|i| BrowseHit {
                fullname: format!("n{i}.{SERVICE_111}"),
                port: 11183,
                v4: vec![Ipv4Addr::new(192, 168, 1, i)],
                txt: vec![("v".into(), "1".into()), ("k".into(), hub.clone())],
            })
            .collect();
        let nets: Vec<(IpAddr, u8)> = vec![("192.168.1.200".parse().unwrap(), 24)];
        let last = sa("192.168.1.12:11183");
        let mut in_first_tries = 0;
        for _ in 0..200 {
            let got = hub_candidates(&hits, &hub, &[], &nets, true, &[]);
            assert_eq!(got.len(), 12);
            in_first_tries += usize::from(got[..REHOME_MAX].contains(&last));
        }
        // p = 4/12: about 67 of 200; never 0
        assert!((25..=120).contains(&in_first_tries), "{in_first_tries}");
    }

    /// A fake daemon: records registrations; a browse returns fixed hits after `delay`.
    struct Fake {
        hits: Vec<BrowseHit>,
        delay: Duration,
        log: Mutex<Vec<String>>,
    }

    impl MdnsOps for Fake {
        fn register(&self, rec: &Record111) -> Result<String, String> {
            self.log
                .lock()
                .unwrap()
                .push(format!("register {}", rec.instance));
            Ok(format!("{}.{SERVICE_111}", rec.instance))
        }
        fn unregister(&self, fullname: &str, _wait: Duration) {
            self.log
                .lock()
                .unwrap()
                .push(format!("unregister {fullname}"));
        }
        fn browse(&self, service: &str, limit: Duration) -> Result<Vec<BrowseHit>, String> {
            assert_eq!(service, SERVICE_111);
            assert!(limit <= BROWSE_FOR);
            std::thread::sleep(self.delay.min(limit));
            self.log.lock().unwrap().push("stop_browse".into());
            Ok(self.hits.clone())
        }
    }

    #[tokio::test]
    async fn no_daemon_means_lan_off_and_no_record() {
        let reg = Arc::new(Registry::new(None, "111.local", false));
        reg.set_lan_record(true);
        assert_eq!(reg.browse(Why::Asked), Err(Refused::LanOff));
        reg.sync_record().await;
        assert!(reg.record().await.is_none());
        // Termux: never a browse, even with a daemon
        let fake = Arc::new(Fake {
            hits: vec![],
            delay: Duration::ZERO,
            log: Mutex::default(),
        });
        let reg = Arc::new(Registry::new(Some(fake), "111.local", true));
        assert_eq!(reg.browse(Why::Asked), Err(Refused::LanOff));
    }

    #[tokio::test]
    async fn one_browse_at_a_time_and_its_rows_replace_the_list() {
        let fake = Arc::new(Fake {
            hits: vec![hit("them", &[("v", "1"), ("r", "0001")], [10, 0, 0, 2])],
            delay: Duration::from_millis(200),
            log: Mutex::default(),
        });
        let reg = Arc::new(Registry::new(Some(fake.clone()), "111.local", false));
        assert!(reg.rows_json().is_empty());
        assert_eq!(reg.browse(Why::Asked), Ok(true));
        assert_eq!(reg.browse(Why::Asked), Ok(false), "one runs already");
        let end = Instant::now() + Duration::from_secs(5);
        while reg.browsing() {
            assert!(Instant::now() < end);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(reg.browses(), 1);
        assert_eq!(fake.log.lock().unwrap().as_slice(), ["stop_browse"]);
        let rows = reg.rows_json();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["label"], "them");
        assert_eq!(rows[0]["roles"], json!(["mailbox"]));
    }
}
