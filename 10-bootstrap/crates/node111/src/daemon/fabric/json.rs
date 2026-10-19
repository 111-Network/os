//! The contract shapes (`40-interface/api/openapi.yaml` Layer 5 schemas) built
//! from `mesh111`'s typed views: `Message`, `Thread`, `Contact`, `Card`
//! (+ `QrMatrix`), `FabricStatus`, `FabricHub`, `FabricSettings`,
//! `HubOffer`. One place, so routes and SSE frames agree byte for byte.
//!
//! - `Message.id` is `msg-<store row id>` (111d-minted, never the wire
//!   `msg_id`, which only `Message.msg_id` exposes — §15).
//! - `Message.text` of a cancelled message's tombstone (§4) is `""`;
//!   `reason_code`/`reason` come from the fabric's §9 reasons (the emitted
//!   set grows with `not_enrolled` and `cancelled`, F2).
//! - `sent_at_claim` is the sender's unverified `sent_at` (§3.3), incoming
//!   only; `at` is the daemon's display time ([`super::times`]).
//! - Peer strings are never interpreted: texts and aliases go out verbatim
//!   as JSON strings (the UI renders them as text).
//! - `FabricStatus.counters` (diagnostics for Advanced) are mesh111's
//!   bounded `Counters` since this fabric started (§18 rule 1: a failure
//!   leaves no trace but a counter); the down shape carries none.
//!
//! **Layer 6 foreign shapes** (`60-transport/README.md` §3 "Ids", §10; the
//! contract's `ForeignConversation`, `ForeignMessage`, `SendGroup`,
//! `BridgeEdge`, `SseNetworkMessage`):
//!
//! - Ids are `fc-<row>` (conversations), `fm-<row>` (foreign rows, one per
//!   part), `be-<row>` (edges), `sg-<row>` (send groups); each parser takes
//!   only its own prefix and digits, like [`parse_msg_id`], and `msg-` stays
//!   111-only. API bodies name conversations by `fc-` id, never by native id
//!   or name.
//! - Foreign text (`text`, `sender.name`, `title`, `native_id`) is
//!   untrusted: it goes out verbatim as JSON strings, already through the
//!   bridge core's one ingest function. A missing name or title is `""`
//!   (the UI shows the native id beside it); nothing is invented.
//! - `at` is the foreign display-times instance's time
//!   ([`super::times::TimesFile::Foreign`]), `claim_ts` the sender's claim
//!   (`null` when absent or not a representable time: never a made-up
//!   date). `via_internet` is `meta.via_mqtt == true`; `truncated` is
//!   `meta.truncated` (the original size; a bare `true` reports the stored
//!   size, a lower bound — never silently `null`).
//! - Facts the store does not hold come from the caller: a conversation's
//!   live `uplink` / `send_enabled` ([`ConvLive`]), an edge's adapter state
//!   and pause reason ([`EdgeLive`]), a send group's per-target plans and
//!   111 row ids (the stored `targets` JSON, [`send_group_targets`]).

use id111::encoding::NodeIdDisplay;
use id111::keyset::NodeId;
use mesh111::contacts::AppData;
use mesh111::fabric::ForeignChange;
use mesh111::fabric::{
    ContactView, Counters, FabricSettings, FabricStatus, HubView, LinkView, MessageView, OfferView,
    ThreadView,
};
use mesh111::link::{CostClass, Framing, LinkState};
use mesh111::store::{
    Class, Dir, EdgeRow, ForeignConversationRow, ForeignDir, ForeignMessageRow, MsgState,
    SendGroupRow, Trust,
};
use serde_json::{json, Map, Value};

use super::times::rfc3339;

/// `msg-<id>` (contract `MessageId`).
pub fn msg_id(id: i64) -> String {
    format!("msg-{id}")
}

/// Parse a `MessageId` back to the store row id (`None` names nothing).
pub fn parse_msg_id(s: &str) -> Option<i64> {
    parse_prefixed(s, "msg-")
}

/// `<prefix><positive decimal row id>`, digits only (no sign, ≤ 19 digits).
fn parse_prefixed(s: &str, prefix: &str) -> Option<i64> {
    let digits = s.strip_prefix(prefix)?;
    if digits.is_empty() || digits.len() > 19 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i64>().ok().filter(|n| *n > 0)
}

/// `fc-<id>` (contract `ForeignConversationId`, Layer 6 §3).
pub fn fc_id(id: i64) -> String {
    format!("fc-{id}")
}

/// `fm-<id>` (contract `ForeignMessageId`: one per stored row).
pub fn fm_id(id: i64) -> String {
    format!("fm-{id}")
}

/// `be-<id>` (contract `BridgeEdgeId`).
pub fn be_id(id: i64) -> String {
    format!("be-{id}")
}

/// `sg-<id>` (contract `SendGroupId`).
pub fn sg_id(id: i64) -> String {
    format!("sg-{id}")
}

/// Parse a `ForeignConversationId` (`None` names nothing).
pub fn parse_fc_id(s: &str) -> Option<i64> {
    parse_prefixed(s, "fc-")
}

/// Parse a `ForeignMessageId` (`None` names nothing; never a `msg-` id).
pub fn parse_fm_id(s: &str) -> Option<i64> {
    parse_prefixed(s, "fm-")
}

/// Parse a `BridgeEdgeId`.
pub fn parse_be_id(s: &str) -> Option<i64> {
    parse_prefixed(s, "be-")
}

/// Parse a `SendGroupId`.
pub fn parse_sg_id(s: &str) -> Option<i64> {
    parse_prefixed(s, "sg-")
}

/// 64 lowercase hex.
pub fn hex(n: &NodeId) -> String {
    id111::encoding::hex_encode(n.as_bytes())
}

/// A contract `NodeId` (`^[0-9a-f]{64}$`; nothing else is accepted in paths
/// and bodies — `node1…` only on `POST /contacts`).
pub fn parse_node_id(s: &str) -> Option<NodeId> {
    let ok = s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if !ok {
        return None;
    }
    id111::encoding::hex_decode(s).ok().map(NodeId::from)
}

fn state(s: MsgState) -> &'static str {
    match s {
        MsgState::Queued => "queued",
        MsgState::Sent => "sent",
        MsgState::Relayed => "relayed",
        MsgState::Delivered => "delivered",
    }
}

fn trust(t: Trust) -> &'static str {
    match t {
        Trust::Oob => "oob",
        Trust::Tofu => "tofu",
        Trust::Enrolled => "enrolled",
    }
}

/// Messages that belong in a thread (`dir` own|in; transit copies never do).
pub fn shown(v: &MessageView) -> bool {
    matches!(v.dir, Dir::Own | Dir::In)
}

/// `Message` with its display time `at` (Unix seconds).
pub fn message(v: &MessageView, at: i64) -> Value {
    let incoming = v.dir == Dir::In;
    json!({
        "id": msg_id(v.id),
        "peer": hex(&v.peer),
        "dir": if incoming { "in" } else { "own" },
        "text": v.text.clone().unwrap_or_default(),
        "state": state(v.state),
        "reason_code": v.reason.map(|r| r.code()),
        "reason": v.reason.map(|r| r.sentence()),
        "local_seq": v.local_seq,
        "received_seq": if incoming { v.received_seq } else { None },
        "sent_at_claim": if incoming {
            v.sent_at_claim
                .and_then(|s| i64::try_from(s).ok())
                .map(rfc3339)
        } else {
            None
        },
        "at": rfc3339(at),
        "msg_id": { "stream": v.stream, "epoch": v.seq.epoch, "n": v.seq.n },
    })
}

/// `Thread` (`at_of` gives each message's display time).
pub fn thread(t: &ThreadView, at_of: &mut dyn FnMut(i64) -> i64) -> Value {
    let d = NodeIdDisplay::new(t.peer.as_bytes());
    json!({
        "peer": d.hex(),
        "alias": t.alias,
        "label": d.label(),
        "request": t.request,
        "unread": t.unread,
        "last": t.last.as_ref().filter(|m| shown(m)).map(|m| message(m, at_of(m.id))),
        "states": {
            "queued": t.states.queued,
            "sent": t.states.sent,
            "relayed": t.states.relayed,
            "delivered": t.states.delivered,
        },
    })
}

/// `Contact`.
pub fn contact(c: &ContactView) -> Value {
    let d = NodeIdDisplay::new(c.node_id.as_bytes());
    json!({
        "node_id": d.hex(),
        "bech32": d.bech32(),
        "label": d.label(),
        "alias": c.alias,
        "trust": trust(c.trust),
        "request": c.request,
        "pinned": c.pinned,
        "has_key": c.has_key,
        "roles": AppData::role_names(c.roles),
        "home_hub": c.home_hub.as_ref().map(|h| {
            h.iter().map(|b| format!("{b:02x}")).collect::<String>()
        }),
    })
}

/// A minimal `Contact` for an id the daemon holds nothing else about (an SSE
/// `removed` frame for a contact it never listed).
pub fn contact_stub(id: &NodeId) -> Value {
    contact(&ContactView {
        node_id: *id,
        alias: None,
        trust: Trust::Oob,
        request: false,
        pinned: false,
        has_key: false,
        roles: 0,
        home_hub: None,
    })
}

/// `QrMatrix` of `text` (byte/alphanumeric mode chosen by the encoder, EC
/// level M, quiet zone not included).
pub fn qr(text: &str) -> Result<Value, String> {
    let code = qrcode::QrCode::new(text.as_bytes()).map_err(|e| e.to_string())?;
    let size = code.width();
    let colors = code.to_colors();
    let rows: Vec<String> = colors
        .chunks(size.max(1))
        .map(|row| {
            row.iter()
                .map(|c| if *c == qrcode::Color::Dark { '1' } else { '0' })
                .collect()
        })
        .collect();
    Ok(json!({ "size": size, "rows": rows }))
}

/// `Card` (`{text, qr}`).
pub fn card(text: &str) -> Result<Value, String> {
    Ok(json!({ "text": text, "qr": qr(text)? }))
}

/// `FabricHub`.
pub fn hub(h: &HubView) -> Value {
    json!({
        "enrolled": h.enrolled,
        "connected": h.connected,
        "reachable": h.reachable,
        "cursor": h.cursor,
        "node_id": h.node_id.as_ref().map(hex),
        "address": h.address.map(|a| a.to_string()),
        "members": h.members,
        "offer_active": h.offer_active,
    })
}

fn link(l: &LinkView) -> Value {
    let (st, detail) = match &l.state {
        LinkState::Down => ("down", None),
        LinkState::Up { .. } => ("up", None),
        LinkState::Degraded(why) => ("degraded", Some(why.clone())),
    };
    let p = &l.profile;
    json!({
        "id": l.id.as_str(),
        "kind": l.kind.as_str(),
        "state": st,
        "peers": if st == "up" { l.peers } else { 0 },
        "detail": detail,
        "profile": {
            "mtu_bytes": p.mtu_bytes,
            "framing": match p.framing { Framing::Stream => "stream", Framing::Datagram => "datagram" },
            "reliable": p.reliable,
            "broadcast": p.broadcast,
            "cost_class": match p.cost_class { CostClass::Bulk => "bulk", CostClass::LowBandwidth => "low_bandwidth" },
            "airtime": p.airtime.map(|a| json!({
                "duty_cycle_permille": a.duty_cycle_permille,
                "bits_per_s": a.bits_per_s,
            })),
        },
    })
}

/// The quota classes the contract's `quotas` map names: the refusable
/// classes (§10.4 capacity shaping) plus `foreign`, Layer 6 §3's shaping
/// quota (evict foreign inbound first, never refuse) — `Class::has_quota`.
pub fn quota_classes() -> impl Iterator<Item = Class> {
    Class::ALL.into_iter().filter(|c| c.has_quota())
}

/// A quota class by its contract key (`kb`, `update`, `patch`, `sync`,
/// `object`, `custody`, `foreign`).
pub fn quota_class(name: &str) -> Option<Class> {
    quota_classes().find(|c| c.name() == name)
}

/// `FabricSettings` (`quotas` from the store: every refusable class, `null`
/// when unset).
pub fn settings(s: &FabricSettings, quotas: &[(Class, Option<u64>)]) -> Value {
    let mut q = Map::new();
    for (c, v) in quotas {
        q.insert(c.name().to_string(), json!(v));
    }
    json!({
        "hub_role": s.hub_role,
        "listen_lan": s.listen_lan,
        "custody_opt_in": s.custody_opt_in,
        "quotas": Value::Object(q),
    })
}

/// `FabricCounters`: every counter mesh111 keeps (destructured, so a new
/// counter does not compile until the contract names it).
pub fn counters(c: &Counters) -> Value {
    let Counters {
        dropped,
        stale,
        refused_requests,
        retransmits,
        inferred_lost,
        max_outstanding,
        regressed,
        piggybacked,
        items_pushed,
        items_received,
        conflicts,
        errors,
    } = *c;
    json!({
        "dropped": dropped,
        "stale": stale,
        "refused_requests": refused_requests,
        "retransmits": retransmits,
        "inferred_lost": inferred_lost,
        "max_outstanding": max_outstanding,
        "regressed": regressed,
        "piggybacked": piggybacked,
        "items_pushed": items_pushed,
        "items_received": items_received,
        "conflicts": conflicts,
        "errors": errors,
    })
}

/// `FabricStatus` of a running fabric.
pub fn status(st: &FabricStatus, quotas: &[(Class, Option<u64>)]) -> Value {
    json!({
        "schema": 1,
        "links": st.links.iter().map(link).collect::<Vec<_>>(),
        "hub": hub(&st.hub),
        "custody": { "count": st.custody.0, "bytes": st.custody.1 },
        "store": {
            "open": true,
            "used": st.store.used,
            "watermark": st.store.watermark,
            "store_full": st.store.store_full,
            "restore_pending": st.store.restore_pending,
        },
        "settings": settings(&st.settings, quotas),
        "counters": counters(&st.counters),
    })
}

/// `FabricStatus` while the fabric is not running (no identity, the store
/// failed to open): the persisted settings, nothing reachable.
pub fn status_down(s: &FabricSettings) -> Value {
    json!({
        "schema": 1,
        "links": [],
        "hub": {
            "enrolled": s.home_hub.is_some(),
            "connected": false,
            "reachable": false,
            "cursor": 0,
            "node_id": s.home_hub.as_ref().map(|h| hex(&h.node_id)),
            "address": s.home_hub.and_then(|h| h.address).map(|a| a.to_string()),
            "members": 0,
            "offer_active": false,
        },
        "custody": { "count": 0, "bytes": 0 },
        "store": {
            "open": false,
            "used": 0,
            "watermark": 0,
            "store_full": false,
            "restore_pending": false,
        },
        "settings": settings(s, &[]),
    })
}

/// `HubOffer`. Without a LAN address the bundle still carries card and code
/// (`111HUB:<card>:<code>:` — an empty address field) and `address` is
/// `null`, so the UI can say the hub is not reachable on the LAN yet.
pub fn offer(o: &OfferView) -> Result<Value, String> {
    let bundle = match &o.bundle {
        Some(b) => b.clone(),
        None => format!(
            "{}{}:{}:",
            mesh111::contacts::BUNDLE_PREFIX,
            o.card,
            o.code.replace('-', "")
        ),
    };
    Ok(json!({
        "schema": 1,
        "code": o.code,
        "address": o.address.map(|a| a.to_string()),
        "card": card(&o.card)?,
        "bundle": card(&bundle)?,
    }))
}

// ---------------------------------------------------------------------------
// Layer 6 foreign shapes (module doc)
// ---------------------------------------------------------------------------

/// RFC 3339 of a foreign claim (`null` when it is not a representable
/// time: a garbage claim is never shown as a made-up date).
fn claim_rfc3339(secs: i64) -> Option<String> {
    let t = time::OffsetDateTime::from_unix_timestamp(secs).ok()?;
    t.format(&time::format_description::well_known::Rfc3339)
        .ok()
}

/// One plain sentence for a foreign `reason_code` (contract
/// `ForeignReasonCode`, §4, §5, §8; unknown codes get a neutral sentence,
/// the UI renders the code as `unknown`).
pub fn foreign_reason(code: &str) -> &'static str {
    match code {
        "region_unset" => "The radio's region is not set, so it may not transmit yet.",
        "no_channel" => "The radio has no channel for this conversation.",
        "too_large" => "The radio refused the text as too large.",
        "not_authorized" => "The radio refused to send it (not authorized).",
        "pki_failed" => "The radio could not encrypt it for the recipient.",
        "got_nak" => "The radio refused it before transmitting.",
        "adapter_down" => "This network's adapter is not running.",
        "no_route" => "Transmitted, not confirmed: no route to the recipient was found.",
        "timeout" => "Transmitted, not confirmed: no answer arrived in time.",
        "max_retransmit" => "Transmitted, not confirmed: the radio gave up after its retries.",
        "no_response" => "Transmitted, not confirmed: the recipient did not respond.",
        "duty_cycle" => "Waiting: the region's duty-cycle limit is reached; it retries later.",
        "rate_limited" => "Waiting: the carrier asked to slow down; it retries later.",
        "key_unknown" => "Waiting: the recipient's key is not known yet.",
        "peer_unannounced" => "Waiting: the recipient has not announced itself yet.",
        "uplink_off" => "Not sent: this channel is uplinked to the internet and sending is off.",
        "ham_unconfirmed" => "Not sent: the licensed-mode confirmation is missing.",
        "consent_paused" => "Paused: a consent fact changed and needs a new confirmation.",
        "cancelled" => "Cancelled before it left this node.",
        "oversize" => "Not sent: the text is longer than this network carries.",
        _ => "The adapter reported a condition this version does not describe.",
    }
}

/// The parsed `meta` object of a foreign row (bounded by the store; an
/// unparsable one reads as empty).
fn meta_of(m: &ForeignMessageRow) -> Map<String, Value> {
    match serde_json::from_str::<Value>(&m.meta) {
        Ok(Value::Object(o)) => o,
        _ => Map::new(),
    }
}

/// `ForeignMessage` with its display time `at` (Unix seconds, from the
/// foreign display-times instance).
pub fn foreign_message(m: &ForeignMessageRow, at: i64) -> Value {
    let meta = meta_of(m);
    let inbound = m.dir == ForeignDir::In;
    let truncated = match meta.get("truncated") {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::Bool(true)) => Some(m.body.len() as u64),
        _ => None,
    };
    let reason_code = m.reason_code.as_deref().filter(|c| !c.is_empty());
    json!({
        "id": fm_id(m.id),
        "conv": fc_id(m.conv),
        "dir": if inbound { "in" } else { "own" },
        "sender": {
            "native": m.sender_native,
            "name": m.sender_name.clone().unwrap_or_default(),
        },
        "auth": m.auth.as_str(),
        "text": m.body,
        "state": m.state.as_str(),
        "evidence": if inbound { None } else { m.evidence.clone() },
        "reason_code": reason_code,
        "reason": reason_code.map(foreign_reason),
        "group_id": m.group_id.map(sg_id),
        "part": (m.part_n >= 2).then(|| json!({ "k": m.part_k, "n": m.part_n })),
        "relayed_from": match (m.origin_msg, m.edge_id) {
            (Some(o), Some(e)) => json!({ "msg": fm_id(o), "edge": be_id(e) }),
            _ => Value::Null,
        },
        "via_internet": meta.get("via_mqtt") == Some(&Value::Bool(true)),
        "seq": m.local_seq,
        "at": rfc3339(at),
        "claim_ts": m.claim_ts.and_then(claim_rfc3339),
        "truncated": truncated,
    })
}

/// Live facts of a conversation the store does not hold (the bridge
/// core's, §10): `uplink` (channels: the board publishes it to MQTT) and
/// the per-channel send switch. `None` is the contract's `null` ("not
/// applicable"; unknown reads the same — the UI shows no switch).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConvLive {
    pub uplink: Option<bool>,
    pub send_enabled: Option<bool>,
}

/// `ForeignConversation`.
pub fn foreign_conversation(c: &ForeignConversationRow, live: &ConvLive) -> Value {
    json!({
        "id": fc_id(c.id),
        "network": c.network,
        "account": c.account,
        "kind": c.kind.as_str(),
        "native_id": c.native_id,
        "title": c.title.clone().unwrap_or_default(),
        "last_seq": c.last_seq,
        "unread": c.unread,
        "pinned": c.pinned,
        "uplink": live.uplink,
        "send_enabled": live.send_enabled,
    })
}

/// `ForeignThread` (`at_of` gives each row's display time).
pub fn foreign_thread(
    c: &ForeignConversationRow,
    live: &ConvLive,
    rows: &[ForeignMessageRow],
    more: bool,
    at_of: &mut dyn FnMut(i64) -> i64,
) -> Value {
    json!({
        "schema": 1,
        "conversation": foreign_conversation(c, live),
        "messages": rows.iter().map(|m| foreign_message(m, at_of(m.id))).collect::<Vec<_>>(),
        "more": more,
    })
}

/// One target of a send group as stored in `send_groups.targets` (the
/// store keeps the JSON opaque; this is its one format): the contract's
/// `FanoutTarget`, the `SendPlan` it was queued with, and for a 111
/// contact target the 111 `messages` row id.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupTarget {
    pub target: Value,
    pub plan: Value,
    pub msg: Option<i64>,
}

/// The `send_groups.targets` JSON for `targets` (an array, request order).
pub fn send_group_targets(targets: &[GroupTarget]) -> String {
    let v: Vec<Value> = targets
        .iter()
        .map(|t| {
            let mut o = Map::new();
            o.insert("target".into(), t.target.clone());
            o.insert("plan".into(), t.plan.clone());
            if let Some(m) = t.msg {
                o.insert("msg".into(), json!(m));
            }
            Value::Object(o)
        })
        .collect();
    Value::Array(v).to_string()
}

/// `SendGroup`: one `SendItem` per stored target. A foreign target names
/// its rows (`id` the first part, `ids` every part when split); a 111
/// target its `msg-` row. A target whose rows are gone (pruned after
/// delivery) or whose entry does not parse is left out: an item never
/// names a row that does not exist.
pub fn send_group(g: &SendGroupRow, rows: &[ForeignMessageRow]) -> Value {
    let entries = match serde_json::from_str::<Value>(&g.targets) {
        Ok(Value::Array(a)) => a,
        _ => Vec::new(),
    };
    let mut items = Vec::new();
    for e in entries {
        let (Some(target), Some(plan)) = (e.get("target"), e.get("plan")) else {
            continue;
        };
        if let Some(conv) = target.get("conv").and_then(Value::as_str) {
            let Some(conv) = parse_fc_id(conv) else {
                continue;
            };
            let mut parts: Vec<&ForeignMessageRow> = rows
                .iter()
                .filter(|m| m.conv == conv && m.group_id == Some(g.id))
                .collect();
            parts.sort_by_key(|m| (m.part_k, m.local_seq));
            let Some(first) = parts.first() else { continue };
            let mut item = json!({ "target": target, "id": fm_id(first.id), "plan": plan });
            if parts.len() > 1 {
                item["ids"] = parts.iter().map(|m| json!(fm_id(m.id))).collect();
            }
            items.push(item);
        } else if target.get("node_id").is_some() {
            let Some(msg) = e.get("msg").and_then(Value::as_i64).filter(|m| *m > 0) else {
                continue;
            };
            items.push(json!({ "target": target, "id": msg_id(msg), "plan": plan }));
        }
    }
    json!({ "id": sg_id(g.id), "text": g.text, "items": items })
}

/// Live facts of an edge the store does not hold (the bridge core's, §8):
/// both endpoints' adapters are enabled, and why it is paused (`user` or
/// `consent_paused`; a paused edge without one reads `user`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeLive<'a> {
    pub adapters_enabled: bool,
    pub pause_reason: Option<&'a str>,
}

fn sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `BridgeEdge`. `enabled` is false while either endpoint's adapter is
/// off (the edge is kept, nothing relays); a stored consent hash that is
/// not lowercase SHA-256 hex reads `null` (never a malformed value).
pub fn edge(e: &EdgeRow, live: &EdgeLive<'_>) -> Value {
    json!({
        "id": be_id(e.id),
        "src": fc_id(e.src_conv),
        "dst": fc_id(e.dst_conv),
        "src_kind": e.src_kind.as_str(),
        "dst_kind": e.dst_kind.as_str(),
        "enabled": e.enabled && live.adapters_enabled,
        "enabled_by": e.enabled_by,
        "paused": e.paused,
        "cursor_seq": e.cursor_seq,
        "relayed_seq": e.relayed_seq,
        "honour_ok_to_mqtt": e.honour_ok_to_mqtt,
        "oversize": e.oversize.as_str(),
        "consent_hash": sha256_hex(&e.consent_hash).then(|| e.consent_hash.clone()),
        "counters": { "relayed": e.relayed, "shed": e.shed, "refused": e.refused },
        "pause_reason": e.paused.then(|| live.pause_reason.unwrap_or("user")),
    })
}

/// `SseNetworkMessage`: `id` is absent on a coalesced channel frame and on
/// a `conversation` change; `seq` is the conversation's highest `seq`
/// after the change. The frame carries no text.
pub fn network_message(
    network: &str,
    conv: i64,
    id: Option<i64>,
    change: ForeignChange,
    seq: u64,
) -> Value {
    let mut o = Map::new();
    o.insert("network".into(), json!(network));
    o.insert("conv".into(), json!(fc_id(conv)));
    if let Some(id) = id {
        o.insert("id".into(), json!(fm_id(id)));
    }
    let change = match change {
        ForeignChange::New => "new",
        ForeignChange::State => "state",
        ForeignChange::Conversation => "conversation",
    };
    o.insert("change".into(), json!(change));
    o.insert("seq".into(), json!(seq));
    Value::Object(o)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh111::ids::MsgSeq;
    use mesh111::outbox::Reason;

    fn view(dir: Dir, reason: Option<Reason>) -> MessageView {
        MessageView {
            id: 42,
            peer: NodeId::from([0xab; 32]),
            dir,
            kind: 1,
            text: Some("hi".into()),
            tombstone: false,
            state: if dir == Dir::In {
                MsgState::Delivered
            } else {
                MsgState::Queued
            },
            reason,
            local_seq: 9,
            received_seq: Some(9),
            sent_at_claim: Some(1_700_000_000),
            stream: 0,
            seq: MsgSeq::new(3, 1),
        }
    }

    #[test]
    fn ids() {
        assert_eq!(msg_id(7), "msg-7");
        assert_eq!(parse_msg_id("msg-7"), Some(7));
        for bad in [
            "msg-",
            "msg-0",
            "msg--1",
            "msg-1a",
            "7",
            "msg-99999999999999999999",
        ] {
            assert_eq!(parse_msg_id(bad), None, "{bad}");
        }
        let n = NodeId::from([0x0f; 32]);
        assert_eq!(parse_node_id(&hex(&n)), Some(n));
        assert_eq!(parse_node_id(&hex(&n).to_uppercase()), None);
        assert_eq!(parse_node_id("node1xyz"), None);
    }

    #[test]
    fn message_shapes() {
        let own = message(&view(Dir::Own, Some(Reason::Cancelled)), 1_700_000_000);
        assert_eq!(own["id"], "msg-42");
        assert_eq!(own["dir"], "own");
        assert_eq!(own["state"], "queued");
        assert_eq!(own["reason_code"], "cancelled");
        assert!(own["received_seq"].is_null());
        assert!(
            own["sent_at_claim"].is_null(),
            "own messages carry no claim"
        );
        assert_eq!(own["at"], "2023-11-14T22:13:20Z");
        assert_eq!(own["msg_id"], json!({"stream": 0, "epoch": 3, "n": 1}));
        let inc = message(&view(Dir::In, None), 0);
        assert_eq!(inc["dir"], "in");
        assert_eq!(inc["received_seq"], 9);
        assert_eq!(inc["sent_at_claim"], "2023-11-14T22:13:20Z");
        assert!(inc["reason_code"].is_null() && inc["reason"].is_null());
        let ne = message(&view(Dir::Own, Some(Reason::NotEnrolled)), 0);
        assert_eq!(ne["reason_code"], "not_enrolled");
        assert!(ne["reason"].as_str().is_some());
    }

    #[test]
    fn qr_matrix_is_square_binary() {
        let q = qr("HELLO WORLD").unwrap();
        let size = q["size"].as_u64().unwrap() as usize;
        let rows = q["rows"].as_array().unwrap();
        assert_eq!(rows.len(), size);
        for r in rows {
            let r = r.as_str().unwrap();
            assert_eq!(r.len(), size);
            assert!(r.bytes().all(|b| b == b'0' || b == b'1'));
        }
    }
}

/// Layer 6 foreign shapes against the contract digest
/// (`40-interface/ui/test/contract.gen.ts`, generated from `openapi.yaml`):
/// every emitted key is declared, every required key is emitted, and the
/// values validate (a JSON-Schema subset: `$ref`, `type`, `enum`, `const`,
/// `minimum`, `maximum`, `minLength`, `maxLength` (Unicode scalar values,
/// as JSON Schema counts them), `minItems`, `maxItems`, `pattern`,
/// `format: date-time`, `required`, `properties`, `items`, `oneOf`).
/// Skipped keywords of the digest (B1-bounce F2): `additionalProperties`
/// (stricter here: an object with `properties` may carry no other key),
/// `format` other than `date-time`, and the annotations `default` and
/// `description`.
#[cfg(test)]
pub(super) mod foreign_tests {
    use super::*;
    use mesh111::store::{EdgeOversize, ForeignAuth, ForeignKind, ForeignState};
    use std::sync::OnceLock;

    fn schemas() -> &'static Map<String, Value> {
        static S: OnceLock<Map<String, Value>> = OnceLock::new();
        S.get_or_init(|| {
            let path = format!(
                "{}/../../../40-interface/ui/test/contract.gen.ts",
                env!("CARGO_MANIFEST_DIR")
            );
            let text = std::fs::read_to_string(path).expect("contract.gen.ts");
            let marker = "export const schemas: Record<string, JsonSchema> = ";
            let start = text.find(marker).expect("schemas marker") + marker.len();
            let mut it = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
            match it.next() {
                Some(Ok(Value::Object(m))) => m,
                other => panic!("contract digest: {other:?}"),
            }
        })
    }

    /// `^…$` patterns of the contract: literals, `[…]` classes, `{n}`,
    /// `{n,m}`, `+` (no alternation: the contract has none).
    fn re_match(pat: &str, s: &str) -> bool {
        let p: Vec<char> = pat
            .strip_prefix('^')
            .and_then(|p| p.strip_suffix('$'))
            .unwrap_or_else(|| panic!("unanchored pattern {pat}"))
            .chars()
            .collect();
        /// A character class with its repeat bounds.
        type Atom = (Vec<(char, char)>, usize, usize);
        let mut atoms: Vec<Atom> = Vec::new();
        let mut i = 0;
        while i < p.len() {
            let mut ranges = Vec::new();
            if p[i] == '[' {
                i += 1;
                while p[i] != ']' {
                    if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
                        ranges.push((p[i], p[i + 2]));
                        i += 3;
                    } else {
                        ranges.push((p[i], p[i]));
                        i += 1;
                    }
                }
            } else {
                ranges.push((p[i], p[i]));
            }
            i += 1;
            let (min, max) = match p.get(i) {
                Some('+') => {
                    i += 1;
                    (1, usize::MAX)
                }
                Some('{') => {
                    let end = p[i..].iter().position(|c| *c == '}').unwrap() + i;
                    let inner: String = p[i + 1..end].iter().collect();
                    i = end + 1;
                    match inner.split_once(',') {
                        Some((a, b)) => (a.parse().unwrap(), b.parse().unwrap()),
                        None => (inner.parse().unwrap(), inner.parse().unwrap()),
                    }
                }
                _ => (1, 1),
            };
            atoms.push((ranges, min, max));
        }
        let s: Vec<char> = s.chars().collect();
        let mut at = 0;
        for (ranges, min, max) in atoms {
            let mut n = 0;
            while n < max && at < s.len() && ranges.iter().any(|(a, b)| s[at] >= *a && s[at] <= *b)
            {
                n += 1;
                at += 1;
            }
            if n < min {
                return false;
            }
        }
        at == s.len()
    }

    fn type_ok(v: &Value, t: &str) -> bool {
        match t {
            "string" => v.is_string(),
            "integer" => v.is_i64() || v.is_u64(),
            "number" => v.is_number(),
            "boolean" => v.is_boolean(),
            "null" => v.is_null(),
            "array" => v.is_array(),
            "object" => v.is_object(),
            _ => false,
        }
    }

    fn validate(v: &Value, schema: &Value, path: &str, errs: &mut Vec<String>) {
        if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
            let name = r.strip_prefix("#/components/schemas/").expect("local ref");
            let s = schemas()
                .get(name)
                .unwrap_or_else(|| panic!("no schema {name}"));
            return validate(v, s, path, errs);
        }
        if let Some(alts) = schema.get("oneOf").and_then(Value::as_array) {
            let ok = alts
                .iter()
                .filter(|a| {
                    let mut e = Vec::new();
                    validate(v, a, path, &mut e);
                    e.is_empty()
                })
                .count();
            if ok != 1 {
                errs.push(format!("{path}: {ok} oneOf branches match {v}"));
            }
            return;
        }
        if let Some(t) = schema.get("type") {
            let types: Vec<&str> = match t {
                Value::String(s) => vec![s.as_str()],
                Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
                _ => vec![],
            };
            if !types.iter().any(|t| type_ok(v, t)) {
                errs.push(format!("{path}: expected {types:?}, got {v}"));
                return;
            }
        }
        if let Some(e) = schema.get("enum").and_then(Value::as_array) {
            if !e.contains(v) {
                errs.push(format!("{path}: {v} not in enum"));
            }
        }
        if let Some(k) = schema.get("const") {
            if k != v {
                errs.push(format!("{path}: expected const {k}"));
            }
        }
        if let (Some(n), Some(m)) = (v.as_f64(), schema.get("minimum").and_then(Value::as_f64)) {
            if n < m {
                errs.push(format!("{path}: {n} below minimum {m}"));
            }
        }
        if let (Some(n), Some(m)) = (v.as_f64(), schema.get("maximum").and_then(Value::as_f64)) {
            if n > m {
                errs.push(format!("{path}: {n} above maximum {m}"));
            }
        }
        if let Some(s) = v.as_str() {
            let len = s.chars().count() as u64;
            if let Some(m) = schema.get("minLength").and_then(Value::as_u64) {
                if len < m {
                    errs.push(format!("{path}: length {len} below minLength {m}"));
                }
            }
            if let Some(m) = schema.get("maxLength").and_then(Value::as_u64) {
                if len > m {
                    errs.push(format!("{path}: length {len} above maxLength {m}"));
                }
            }
        }
        if let Some(a) = v.as_array() {
            let len = a.len() as u64;
            if let Some(m) = schema.get("minItems").and_then(Value::as_u64) {
                if len < m {
                    errs.push(format!("{path}: {len} items below minItems {m}"));
                }
            }
            if let Some(m) = schema.get("maxItems").and_then(Value::as_u64) {
                if len > m {
                    errs.push(format!("{path}: {len} items above maxItems {m}"));
                }
            }
        }
        if let Some(s) = v.as_str() {
            if let Some(p) = schema.get("pattern").and_then(Value::as_str) {
                if !re_match(p, s) {
                    errs.push(format!("{path}: {s:?} does not match {p}"));
                }
            }
            if schema.get("format").and_then(Value::as_str) == Some("date-time")
                && time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
                    .is_err()
            {
                errs.push(format!("{path}: {s:?} is not RFC 3339"));
            }
        }
        if let (Some(o), Some(props)) = (
            v.as_object(),
            schema.get("properties").and_then(Value::as_object),
        ) {
            for r in schema
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if !o.contains_key(r) {
                    errs.push(format!("{path}: required key {r} missing"));
                }
            }
            for (k, x) in o {
                match props.get(k) {
                    Some(s) => validate(x, s, &format!("{path}.{k}"), errs),
                    None => errs.push(format!("{path}: key {k} is not in the schema")),
                }
            }
        }
        if let (Some(a), Some(items)) = (v.as_array(), schema.get("items")) {
            for (i, x) in a.iter().enumerate() {
                validate(x, items, &format!("{path}[{i}]"), errs);
            }
        }
    }

    /// Validate `v` against the contract schema `name` (also used by the
    /// fabric wiring's frame tests).
    pub(in crate::daemon::fabric) fn assert_schema(v: &Value, name: &str) {
        let mut errs = Vec::new();
        validate(
            v,
            &json!({ "$ref": format!("#/components/schemas/{name}") }),
            name,
            &mut errs,
        );
        assert!(errs.is_empty(), "{name}: {errs:#?}\n{v:#}");
    }

    fn fm(id: i64) -> ForeignMessageRow {
        ForeignMessageRow {
            id,
            conv: 3,
            dir: ForeignDir::In,
            sender_native: "!a1b2c3d4".into(),
            sender_name: Some("W1AW <b>".into()),
            auth: ForeignAuth::ChannelKey,
            native_msg_id: Some("1a2b3c4d".into()),
            body: "hi from the mesh".into(),
            local_seq: 41,
            claim_ts: Some(1_700_000_000),
            state: ForeignState::Delivered,
            evidence: None,
            reason_code: None,
            group_id: None,
            part_k: 1,
            part_n: 1,
            origin_msg: None,
            edge_id: None,
            meta: r#"{"snr":6.5,"rssi":-97,"hops":2,"via_mqtt":true}"#.into(),
            handoff_pending: false,
        }
    }

    fn conv(id: i64, kind: ForeignKind, title: Option<&str>) -> ForeignConversationRow {
        ForeignConversationRow {
            id,
            network: "meshtastic".into(),
            account: "!0badc0de".into(),
            kind,
            native_id: "ch:LongFast:9f3c".into(),
            title: title.map(str::to_owned),
            last_seq: 41,
            unread: 2,
            pinned: false,
            messages: 3,
            queued: 0,
        }
    }

    fn edge_row(paused: bool, hash: &str) -> EdgeRow {
        EdgeRow {
            id: 4,
            src_conv: 3,
            dst_conv: 5,
            src_kind: ForeignKind::Channel,
            dst_kind: ForeignKind::Group,
            enabled: true,
            paused,
            cursor_seq: 10,
            relayed_seq: 12,
            honour_ok_to_mqtt: false,
            oversize: EdgeOversize::Split,
            consent_hash: hash.into(),
            enabled_by: "loopback".into(),
            relayed: 2,
            shed: 1,
            refused: 0,
        }
    }

    fn plan() -> Value {
        json!({
            "bytes": 5, "cap": 232, "parts": 1, "size_mode": "refuse",
            "airtime_estimate": { "millis": 2100, "label": "about 2.1 s (estimate)" },
            "refusal": null, "warnings": [],
        })
    }

    #[test]
    fn foreign_ids_round_trip_and_never_cross_prefixes() {
        assert_eq!(
            (fc_id(7), fm_id(8), be_id(9), sg_id(10)),
            ("fc-7".into(), "fm-8".into(), "be-9".into(), "sg-10".into())
        );
        assert_eq!(parse_fc_id("fc-7"), Some(7));
        assert_eq!(parse_fm_id("fm-8"), Some(8));
        assert_eq!(parse_be_id("be-9"), Some(9));
        assert_eq!(parse_sg_id("sg-10"), Some(10));
        // each parser takes only its own prefix; msg- stays 111-only
        assert_eq!(parse_fm_id("msg-8"), None);
        assert_eq!(parse_msg_id("fm-8"), None);
        assert_eq!(parse_fc_id("fm-7"), None);
        assert_eq!(parse_be_id("sg-9"), None);
        for bad in [
            "fc-",
            "fc-0",
            "fc--1",
            "fc-+1",
            "fc-1a",
            "fc- 1",
            "FC-1",
            "fc-01x",
            "7",
            "fc-99999999999999999999",
        ] {
            assert_eq!(parse_fc_id(bad), None, "{bad}");
        }
        // the minted ids match the contract's patterns
        for (id, schema) in [
            (fc_id(i64::MAX), "ForeignConversationId"),
            (fm_id(1), "ForeignMessageId"),
            (be_id(1), "BridgeEdgeId"),
            (sg_id(1), "SendGroupId"),
        ] {
            assert_schema(&json!(id), schema);
        }
    }

    #[test]
    fn foreign_message_matches_the_contract() {
        let inbound = foreign_message(&fm(11), 1_700_000_100);
        assert_schema(&inbound, "ForeignMessage");
        assert_eq!(inbound["id"], "fm-11");
        assert_eq!(inbound["conv"], "fc-3");
        assert_eq!(inbound["dir"], "in");
        assert_eq!(
            inbound["sender"],
            json!({"native": "!a1b2c3d4", "name": "W1AW <b>"}),
            "untrusted text verbatim"
        );
        assert_eq!(inbound["auth"], "channel-key");
        assert_eq!(inbound["state"], "delivered");
        assert_eq!(inbound["via_internet"], true);
        assert_eq!(inbound["seq"], 41);
        assert_eq!(inbound["at"], "2023-11-14T22:15:00Z");
        assert_eq!(inbound["claim_ts"], "2023-11-14T22:13:20Z");
        for k in ["evidence", "reason_code", "reason", "group_id", "part"] {
            assert!(inbound[k].is_null(), "{k}");
        }
        assert!(inbound["relayed_from"].is_null() && inbound["truncated"].is_null());

        // an own part of a split send, failed before air
        let mut own = fm(12);
        own.dir = ForeignDir::Own;
        own.sender_name = None;
        own.state = ForeignState::Failed;
        own.evidence = Some("refused by the radio".into());
        own.reason_code = Some("region_unset".into());
        own.group_id = Some(6);
        own.part_k = 2;
        own.part_n = 3;
        own.claim_ts = None;
        own.meta = r#"{"truncated":70000,"handoff":"pending"}"#.into();
        let v = foreign_message(&own, 0);
        assert_schema(&v, "ForeignMessage");
        assert_eq!(v["sender"]["name"], "", "no name is invented");
        assert_eq!(v["group_id"], "sg-6");
        assert_eq!(v["part"], json!({"k": 2, "n": 3}));
        assert_eq!(v["reason_code"], "region_unset");
        assert!(v["reason"].as_str().is_some_and(|r| r.contains("region")));
        assert_eq!(v["evidence"], "refused by the radio");
        assert_eq!(v["truncated"], 70000);
        assert_eq!(v["via_internet"], false);
        assert!(v["claim_ts"].is_null());

        // a relay copy; an unknown reason code still gets a sentence
        let mut copy = fm(13);
        copy.dir = ForeignDir::Own;
        copy.state = ForeignState::Relayed;
        copy.origin_msg = Some(11);
        copy.edge_id = Some(4);
        copy.reason_code = Some("brand_new_code".into());
        copy.meta = r#"{"truncated":true}"#.into();
        let v = foreign_message(&copy, 0);
        assert_schema(&v, "ForeignMessage");
        assert_eq!(v["relayed_from"], json!({"msg": "fm-11", "edge": "be-4"}));
        assert!(v["reason"].as_str().is_some());
        assert_eq!(v["truncated"], copy.body.len(), "never silently null");

        // an inbound row never carries evidence; a garbage claim is null
        let mut odd = fm(14);
        odd.evidence = Some("x".into());
        odd.claim_ts = Some(i64::MAX);
        odd.meta = "not json".into();
        let v = foreign_message(&odd, 0);
        assert_schema(&v, "ForeignMessage");
        assert!(v["evidence"].is_null() && v["claim_ts"].is_null());
        assert_eq!(v["via_internet"], false);
        // every reason code of the contract's vocabulary has its sentence
        let vocab = schemas()["ForeignReasonCode"]["enum"].as_array().unwrap();
        for c in vocab {
            let c = c.as_str().unwrap();
            assert_ne!(
                foreign_reason(c),
                foreign_reason("brand_new_code"),
                "{c} has its own sentence"
            );
        }
    }

    #[test]
    fn foreign_conversation_and_thread_match_the_contract() {
        let c = conv(3, ForeignKind::Channel, Some("LongFast"));
        let live = ConvLive {
            uplink: Some(true),
            send_enabled: Some(false),
        };
        let v = foreign_conversation(&c, &live);
        assert_schema(&v, "ForeignConversation");
        assert_eq!(v["id"], "fc-3");
        assert_eq!(v["kind"], "channel");
        assert_eq!(
            (v["uplink"].clone(), v["send_enabled"].clone()),
            (json!(true), json!(false))
        );
        let dm = conv(4, ForeignKind::Dm, None);
        let v = foreign_conversation(&dm, &ConvLive::default());
        assert_schema(&v, "ForeignConversation");
        assert_eq!(v["title"], "");
        assert!(v["uplink"].is_null() && v["send_enabled"].is_null());
        let rows = [fm(11), fm(12)];
        let mut seen = Vec::new();
        let t = foreign_thread(&c, &live, &rows, true, &mut |id| {
            seen.push(id);
            1_700_000_000
        });
        assert_schema(&t, "ForeignThread");
        assert_eq!(seen, vec![11, 12]);
        assert_eq!(t["more"], true);
    }

    #[test]
    fn send_group_items_name_their_rows() {
        let g = SendGroupRow {
            id: 6,
            local_seq: 30,
            text: "hello all".into(),
            targets: send_group_targets(&[
                GroupTarget {
                    target: json!({"network": "meshtastic", "conv": "fc-3", "size_mode": "split"}),
                    plan: plan(),
                    msg: None,
                },
                GroupTarget {
                    target: json!({"node_id": "ab".repeat(32)}),
                    plan: plan(),
                    msg: Some(77),
                },
                // rows pruned since: left out, never a dangling id
                GroupTarget {
                    target: json!({"network": "meshtastic", "conv": "fc-9"}),
                    plan: plan(),
                    msg: None,
                },
                GroupTarget {
                    target: json!({"network": "lxmf", "conv": "fc-5"}),
                    plan: plan(),
                    msg: None,
                },
            ]),
        };
        let part = |id: i64, conv: i64, k: u32, n: u32, seq: u64| {
            let mut m = fm(id);
            m.dir = ForeignDir::Own;
            m.state = ForeignState::Queued;
            m.conv = conv;
            m.group_id = Some(6);
            m.part_k = k;
            m.part_n = n;
            m.local_seq = seq;
            m
        };
        // parts of fc-3 arrive out of order; one row of another group
        let mut stray = part(40, 3, 1, 1, 30);
        stray.group_id = Some(99);
        let rows = vec![
            part(32, 3, 2, 2, 32),
            part(31, 3, 1, 2, 31),
            part(33, 5, 1, 1, 33),
            stray,
        ];
        let v = send_group(&g, &rows);
        assert_schema(&v, "SendGroup");
        assert_eq!(v["id"], "sg-6");
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["id"], "fm-31");
        assert_eq!(items[0]["ids"], json!(["fm-31", "fm-32"]));
        assert_eq!(items[1]["id"], "msg-77");
        assert!(items[1].get("ids").is_none());
        assert_eq!(items[2]["id"], "fm-33");
        assert!(items[2].get("ids").is_none(), "a single row has no ids");
        // unparsable targets read as an empty list
        let broken = SendGroupRow {
            targets: "{\"not\": \"an array\"}".into(),
            ..g
        };
        let v = send_group(&broken, &rows);
        assert_schema(&v, "SendGroup");
        assert_eq!(v["items"], json!([]));
    }

    #[test]
    fn bridge_edge_matches_the_contract() {
        let h = "0f".repeat(32);
        let live = EdgeLive {
            adapters_enabled: true,
            pause_reason: None,
        };
        let v = edge(&edge_row(false, &h), &live);
        assert_schema(&v, "BridgeEdge");
        assert_eq!(
            (v["id"].clone(), v["src"].clone(), v["dst"].clone()),
            (json!("be-4"), json!("fc-3"), json!("fc-5"))
        );
        assert_eq!(v["dst_kind"], "group");
        assert_eq!(v["enabled"], true);
        assert_eq!(v["consent_hash"], h);
        assert_eq!(
            v["counters"],
            json!({"relayed": 2, "shed": 1, "refused": 0})
        );
        assert!(v["pause_reason"].is_null());
        // paused without a known reason reads `user`; adapters off ⇒ not enabled
        let off = EdgeLive {
            adapters_enabled: false,
            pause_reason: None,
        };
        let v = edge(&edge_row(true, "not-a-hash"), &off);
        assert_schema(&v, "BridgeEdge");
        assert_eq!(v["enabled"], false);
        assert_eq!(v["pause_reason"], "user");
        assert!(v["consent_hash"].is_null(), "never a malformed hash");
        let consent = EdgeLive {
            adapters_enabled: true,
            pause_reason: Some("consent_paused"),
        };
        let v = edge(&edge_row(true, &h), &consent);
        assert_schema(&v, "BridgeEdge");
        assert_eq!(v["pause_reason"], "consent_paused");
    }

    #[test]
    fn network_message_frames_match_the_contract() {
        let one = network_message("meshtastic", 3, Some(11), ForeignChange::New, 41);
        assert_schema(&one, "SseNetworkMessage");
        assert_eq!(
            one,
            json!({"network": "meshtastic", "conv": "fc-3", "id": "fm-11", "change": "new", "seq": 41})
        );
        let coalesced = network_message("meshtastic", 3, None, ForeignChange::New, 90);
        assert_schema(&coalesced, "SseNetworkMessage");
        assert!(coalesced.get("id").is_none(), "absent, not null");
        let st = network_message("lxmf", 5, Some(12), ForeignChange::State, 7);
        assert_eq!(st["change"], "state");
        let conv = network_message("app:deltachat", 6, None, ForeignChange::Conversation, 0);
        assert_schema(&conv, "SseNetworkMessage");
        assert_eq!(conv["change"], "conversation");
    }

    #[test]
    fn the_validator_catches_drift() {
        let mut v = foreign_message(&fm(11), 0);
        let o = v.as_object_mut().unwrap();
        o.remove("truncated");
        o.insert("native_msg_id".into(), json!("1a2b"));
        o.insert("state".into(), json!("pending"));
        o.insert("id".into(), json!("msg-11"));
        o.insert("claim_ts".into(), json!("yesterday"));
        let mut errs = Vec::new();
        validate(
            &v,
            &json!({"$ref": "#/components/schemas/ForeignMessage"}),
            "m",
            &mut errs,
        );
        let all = errs.join("\n");
        for want in [
            "truncated missing",
            "native_msg_id is not",
            "not in enum",
            "does not match",
            "RFC 3339",
        ] {
            assert!(all.contains(want), "{want}: {all}");
        }
        let mut errs = Vec::new();
        validate(
            &json!(null),
            &json!({"$ref": "#/components/schemas/ForeignPart"}),
            "p",
            &mut errs,
        );
        assert!(!errs.is_empty());
        // B1-bounce F2: length and count bounds are checked too — a
        // 300-byte `enabled_by` (contract maxLength 256) is drift.
        let h = "0f".repeat(32);
        let live = EdgeLive {
            adapters_enabled: true,
            pause_reason: None,
        };
        let mut e = edge(&edge_row(false, &h), &live);
        let mut errs = Vec::new();
        validate(
            &e,
            &json!({"$ref": "#/components/schemas/BridgeEdge"}),
            "e",
            &mut errs,
        );
        assert!(errs.is_empty(), "{errs:#?}");
        e["enabled_by"] = json!("x".repeat(300));
        let mut errs = Vec::new();
        validate(
            &e,
            &json!({"$ref": "#/components/schemas/BridgeEdge"}),
            "e",
            &mut errs,
        );
        assert!(
            errs.iter()
                .any(|x| x.contains("enabled_by") && x.contains("maxLength")),
            "{errs:#?}"
        );
        // the other bounds, on inline schemas
        for (v, schema, want) in [
            (
                json!(""),
                json!({"type": "string", "minLength": 1}),
                "minLength",
            ),
            (
                json!(7),
                json!({"type": "integer", "maximum": 5}),
                "maximum",
            ),
            (
                json!([]),
                json!({"type": "array", "minItems": 1}),
                "minItems",
            ),
            (
                json!([1, 2, 3]),
                json!({"type": "array", "maxItems": 2}),
                "maxItems",
            ),
        ] {
            let mut errs = Vec::new();
            validate(&v, &schema, "x", &mut errs);
            assert!(errs.iter().any(|x| x.contains(want)), "{want}: {errs:#?}");
        }
        // maxLength counts characters, not bytes
        let mut errs = Vec::new();
        validate(
            &json!("éé"),
            &json!({"type": "string", "maxLength": 2}),
            "x",
            &mut errs,
        );
        assert!(errs.is_empty(), "{errs:#?}");
    }

    #[test]
    fn quota_classes_include_the_foreign_shaping_quota() {
        let names: Vec<&str> = quota_classes().map(|c| c.name()).collect();
        assert!(names.contains(&"foreign"), "{names:?}");
        assert_eq!(quota_class("foreign"), Some(Class::Foreign));
        assert_eq!(quota_class("kb"), Some(Class::Kb));
        assert_eq!(quota_class("msg"), None, "messages take no quota");
    }
}
