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

use id111::encoding::NodeIdDisplay;
use id111::keyset::NodeId;
use mesh111::contacts::AppData;
use mesh111::fabric::{
    ContactView, Counters, FabricSettings, FabricStatus, HubView, LinkView, MessageView, OfferView,
    ThreadView,
};
use mesh111::link::{CostClass, Framing, LinkState};
use mesh111::store::{Class, Dir, MsgState, Trust};
use serde_json::{json, Map, Value};

use super::times::rfc3339;

/// `msg-<id>` (contract `MessageId`).
pub fn msg_id(id: i64) -> String {
    format!("msg-{id}")
}

/// Parse a `MessageId` back to the store row id (`None` names nothing).
pub fn parse_msg_id(s: &str) -> Option<i64> {
    let digits = s.strip_prefix("msg-")?;
    if digits.is_empty() || digits.len() > 19 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i64>().ok().filter(|n| *n > 0)
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

/// The quota classes the contract's `quotas` map names (capacity shaping
/// applies to the refusable classes only, §10.4).
pub fn quota_classes() -> impl Iterator<Item = Class> {
    Class::ALL.into_iter().filter(|c| c.is_refusable())
}

/// A quota class by its contract key (`kb`, `update`, `patch`, `sync`,
/// `object`, `custody`).
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
