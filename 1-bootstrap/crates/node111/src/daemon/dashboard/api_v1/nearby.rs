//! `GET /api/v1/nearby` — presence per source (Layer 5 §14 "Nearby", §15;
//! contract `listNearby`). Two kinds of source:
//!
//! - `meshtastic`: the bound Meshtastic board's node DB, as validated by
//!   `daemon::meshtastic::presence` (or the bridge client's node DB), grouped
//!   as "heard via Meshtastic on <board alias>";
//! - `reticulum` (Layer 6 `6-transport/README.md` §6, W1): the announces the
//!   LXMF helper hears. The LXMF adapter reports each surfaced `heard` as
//!   `Inbound::Nearby`; the bridge core hands it to its `NearbySink`, which
//!   `start_layer6` sets to [`sink`] over the daemon's [`BridgeNearby`]. The
//!   sink keeps the latest node per destination (bounded, oldest heard
//!   forgotten), merged per source; no timer, no poll — `GET` reads what is
//!   there, and a change publishes the SSE `nearby` frame (`{sources}`, the
//!   merged list, the frame the Meshtastic module publishes on its changes).
//!   A disabled adapter's source is cleared ([`BridgeNearby::clear`]).
//!
//! Every node string is an **untrusted RF-origin value** (the contract's
//! `NearbyNode` description says so): re-validated here whatever the adapter
//! did (names cleaned and cut to 40 bytes, ids in their own alphabet, a
//! closed set of kinds per network), rendered as text only, handed to the
//! agent only inside a labelled untrusted block. A node is never a contact.
//!
//! Without the Meshtastic module, without a bound board, before the reader
//! has produced a node DB, and with no LXMF heard, the list is empty — no
//! fake peers.

use super::{ApiRouter, ApiState};
use crate::daemon::bridge::{NearbySink, NETWORK_111, NETWORK_LXMF};
use crate::daemon::meshtastic::presence::{sanitize_name, NearbyNode, NearbySource};
use crate::daemon::meshtastic::Meshtastic;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

/// Nodes kept per bridge source (the oldest heard is forgotten first).
pub const MAX_BRIDGE_NODES: usize = 512;
/// Nodes looked at in one reported document (the LXMF adapter reports one).
pub const MAX_PUSH_NODES: usize = 16;
/// A node name at most (the contract's `NearbyNode`: 40 bytes).
pub const MAX_NODE_NAME: usize = 40;
/// A reported name looked at at most (longer: the node is dropped, not cut).
const MAX_RAW_NAME: usize = 1024;
/// Bridge sources kept at most (one per account of a network).
pub const MAX_BRIDGE_SOURCES: usize = 8;
/// The `reticulum` source's heading.
pub const RETICULUM_LABEL: &str = "heard via Reticulum";

/// `NearbyNode.evidence[].transport` (Layer 7 `7-relay/README.md` §2.3, contract `NearbyEvidenceTransport`): the closed set a `111` row's
/// evidence may name. `tcp-lan` is a connection attributed to the peer (also the home hub's row and a `tcp` pair's row), `hub` a contact
/// heard through the home hub, `carrier` a `carrier` pair's row. The `111` source itself is built by the presence module (row D2).
pub const EVIDENCE_TRANSPORTS: [&str; 3] = ["tcp-lan", "hub", "carrier"];

/// The `NearbySource.kind` of a network's source (closed: anything else is dropped). Layer 7 adds `111` (the presence module's own
/// source, §2.3); only the bridge's `reticulum` source is ever pushed by an adapter ([`BridgeNearby::push`]).
fn kind_for(network: &str) -> Option<&'static str> {
    if network == NETWORK_111 {
        Some("111")
    } else {
        (network == NETWORK_LXMF).then_some("reticulum")
    }
}

fn is_hex32(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

struct BridgeSource {
    network: String,
    /// Oldest heard first; one entry per node id.
    nodes: VecDeque<NearbyNode>,
}

/// The Nearby sources the bridge's adapters report (module doc). Cheap; in memory only.
#[derive(Default)]
pub struct BridgeNearby {
    sources: Mutex<BTreeMap<String, BridgeSource>>,
    version: AtomicU64,
}

impl BridgeNearby {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, BridgeSource>> {
        self.sources.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Merge one reported `NearbySource` document from `network`'s adapter. Returns
    /// whether what `GET /nearby` shows changed (a new node, a new name); a document that
    /// does not validate changes nothing.
    pub fn push(&self, network: &str, doc: &Value) -> bool {
        let Some(kind) = kind_for(network).filter(|k| *k == "reticulum") else {
            return false;
        };
        let Some(o) = doc.as_object() else {
            return false;
        };
        if o.get("kind").and_then(Value::as_str) != Some(kind) {
            return false;
        }
        let Some(id) = o
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| id.strip_prefix("reticulum:").is_some_and(is_hex32))
        else {
            return false;
        };
        let Some(nodes) = o.get("nodes").and_then(Value::as_array) else {
            return false;
        };
        let nodes: Vec<NearbyNode> = nodes
            .iter()
            .take(MAX_PUSH_NODES)
            .filter_map(reticulum_node)
            .collect();
        if nodes.is_empty() {
            return false;
        }
        let mut all = self.lock();
        if !all.contains_key(id) && all.len() >= MAX_BRIDGE_SOURCES {
            return false;
        }
        let src = all.entry(id.to_string()).or_insert_with(|| BridgeSource {
            network: network.to_string(),
            nodes: VecDeque::new(),
        });
        let mut changed = false;
        for n in nodes {
            match src.nodes.iter().position(|x| x.id == n.id) {
                Some(i) => {
                    let old = src.nodes.remove(i);
                    changed |= old.as_ref() != Some(&n);
                }
                None => changed = true,
            }
            src.nodes.push_back(n);
            while src.nodes.len() > MAX_BRIDGE_NODES {
                src.nodes.pop_front();
            }
        }
        if changed {
            self.version.fetch_add(1, Ordering::SeqCst);
        }
        changed
    }

    /// Forget every source `network` reported (its adapter was disabled). Returns whether
    /// anything was removed.
    pub fn clear(&self, network: &str) -> bool {
        let mut all = self.lock();
        let before = all.len();
        all.retain(|_, s| s.network != network);
        let changed = all.len() != before;
        if changed {
            self.version.fetch_add(1, Ordering::SeqCst);
        }
        changed
    }

    /// Bumped on every change (tests; a reader that caches).
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    /// The sources, by id; each source's nodes most recently heard first.
    pub fn sources(&self) -> Vec<NearbySource> {
        self.lock()
            .iter()
            .map(|(id, s)| NearbySource {
                id: id.clone(),
                kind: kind_for(&s.network).unwrap_or("reticulum"),
                label: RETICULUM_LABEL.into(),
                device_id: None,
                nodes: s.nodes.iter().rev().cloned().collect(),
            })
            .collect()
    }
}

/// One `reticulum` node from a reported document: `id` the 32-hex destination, the
/// announced name cleaned and cut to 40 bytes, `short_name` its first 8 hex, `num` 0,
/// every RF field null (the contract's `reticulum` kind). `None`: it does not validate.
fn reticulum_node(v: &Value) -> Option<NearbyNode> {
    let o = v.as_object()?;
    let id = o
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| is_hex32(s))?;
    let long_name = match o.get("long_name") {
        Some(Value::String(s)) if s.len() <= MAX_RAW_NAME => sanitize_name(s, MAX_NODE_NAME),
        None | Some(Value::Null) => String::new(),
        _ => return None,
    };
    Some(NearbyNode {
        id: id.to_string(),
        num: 0,
        long_name,
        short_name: id[..8].to_string(),
        pubkey: None,
        snr: None,
        rssi: None,
        hops_away: None,
        last_heard_hint: None,
        position: None,
        channel_utilization: None,
    })
}

/// Every source `GET /nearby` lists: the Meshtastic module's, then the bridge's.
pub fn sources(api: &ApiState) -> Vec<NearbySource> {
    let mut out = api
        .ext::<Arc<Meshtastic>>()
        .map(|m| m.nearby_sources())
        .unwrap_or_default();
    if let Some(b) = api.ext::<Arc<BridgeNearby>>() {
        out.extend(b.sources());
    }
    out
}

/// Publish the SSE `nearby` frame with the merged list.
pub fn publish(api: &ApiState) {
    api.events
        .publish("nearby", &serde_json::json!({ "sources": sources(api) }));
}

/// The bridge core's Nearby sink (module doc): merge into `store`, publish on a change.
/// Holds the API state weakly (the bridge lives in its ext).
pub fn sink(api: &Arc<ApiState>, store: Arc<BridgeNearby>) -> NearbySink {
    let api: Weak<ApiState> = Arc::downgrade(api);
    Arc::new(move |network: &str, doc: Value| {
        if store.push(network, &doc) {
            if let Some(api) = api.upgrade() {
                publish(&api);
            }
        }
    })
}

async fn list(State(api): State<Arc<ApiState>>) -> Response {
    Json(serde_json::json!({ "schema": 1, "sources": sources(&api) })).into_response()
}

pub fn router() -> ApiRouter {
    axum::Router::new().route("/api/v1/nearby", get(list))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ME: &str = "0123456789abcdef0123456789abcdef";
    const D: &str = "fedcba9876543210fedcba9876543210";

    fn doc(dest: &str, name: &str) -> Value {
        crate::daemon::bridge::lxmf::nearby_source(ME, dest, name)
    }

    /// An `enum: [..]` list from the first contract line that starts with `prefix` (skipped without the checkout).
    fn contract_enum(prefix: &str) -> Option<Vec<String>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../4-interface/api/openapi.yaml");
        let text = std::fs::read_to_string(path).ok()?;
        let line = text.lines().find(|l| l.trim_start().starts_with(prefix))?;
        let list = line.split("enum: [").nth(1)?.split(']').next()?;
        Some(
            list.split(',')
                .map(|k| k.trim().trim_matches('"').to_string())
                .collect(),
        )
    }

    #[test]
    fn the_111_kind_and_the_evidence_set_mirror_the_contract() {
        assert_eq!(kind_for(NETWORK_111), Some("111"));
        assert_eq!(kind_for(NETWORK_LXMF), Some("reticulum"));
        assert_eq!(kind_for("meshtastic"), None);
        assert_eq!(EVIDENCE_TRANSPORTS, ["tcp-lan", "hub", "carrier"]);
        if let Some(c) = contract_enum("NearbyEvidenceTransport:") {
            assert_eq!(
                c, EVIDENCE_TRANSPORTS,
                "openapi.yaml NearbyEvidenceTransport and EVIDENCE_TRANSPORTS differ"
            );
        }
        if let Some(c) = contract_enum("kind: { type: string, enum: [meshtastic, reticulum") {
            assert_eq!(c, ["meshtastic", "reticulum", "111"]);
        }
    }

    #[test]
    fn an_adapter_can_never_push_the_111_source() {
        // The `111` source is the presence module's: a document naming network `111` is dropped like any unknown network.
        let b = BridgeNearby::default();
        let mut d = doc(D, "x");
        d["kind"] = json!("111");
        assert!(!b.push("111", &d));
        assert!(!b.push("111", &doc(D, "x")));
        assert!(b.sources().is_empty());
    }

    #[test]
    fn heards_merge_per_destination_latest_first_and_bounded() {
        let b = BridgeNearby::default();
        assert!(b.push("lxmf", &doc(D, "Bob")));
        assert!(!b.push("lxmf", &doc(D, "Bob")), "the same node: no change");
        assert!(b.push("lxmf", &doc(D, "Robert")), "a new name is a change");
        let other = "00112233445566778899aabbccddeeff";
        assert!(b.push("lxmf", &doc(other, "")));
        let s = b.sources();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].id, format!("reticulum:{ME}"));
        assert_eq!(s[0].kind, "reticulum");
        assert_eq!(s[0].device_id, None);
        let ids: Vec<&str> = s[0].nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, [other, D], "most recently heard first");
        assert_eq!(s[0].nodes[1].long_name, "Robert");
        assert_eq!(s[0].nodes[1].short_name, &D[..8]);
        for i in 0..(MAX_BRIDGE_NODES + 5) {
            b.push("lxmf", &doc(&format!("{i:032x}"), "n"));
        }
        assert_eq!(b.sources()[0].nodes.len(), MAX_BRIDGE_NODES);
        let v = b.version();
        assert!(b.clear("lxmf"));
        assert!(b.sources().is_empty());
        assert!(b.version() > v);
        assert!(!b.clear("lxmf"));
    }

    #[test]
    fn untrusted_documents_are_revalidated_and_fail_closed() {
        let b = BridgeNearby::default();
        let mut long = doc(D, &format!("a\u{202e}\u{7}{}", "\u{20ac}".repeat(30)));
        assert!(b.push("lxmf", &long));
        let n = &b.sources()[0].nodes[0];
        assert!(n.long_name.len() <= MAX_NODE_NAME, "{}", n.long_name);
        assert!(n.long_name.starts_with('a') && !n.long_name.contains('\u{202e}'));
        // a closed set of kinds per network, ids in their alphabet
        assert!(
            !b.push("meshtastic", &doc(D, "x")),
            "meshtastic has its own source"
        );
        assert!(!b.push("app:deltachat", &doc(D, "x")));
        long["kind"] = json!("meshtastic");
        assert!(!b.push("lxmf", &long));
        let mut bad = doc(D, "x");
        bad["id"] = json!("reticulum:XYZ");
        assert!(!b.push("lxmf", &bad));
        let mut bad = doc(D, "x");
        bad["nodes"][0]["id"] = json!("!a1b2c3d4");
        assert!(!b.push("lxmf", &bad));
        let mut bad = doc(D, "x");
        bad["nodes"][0]["long_name"] = json!(42);
        assert!(!b.push("lxmf", &bad));
        assert!(!b.push("lxmf", &json!("x")));
        assert!(!b.push("lxmf", &json!({"kind": "reticulum"})));
        // what the adapter sends beyond the shape is not taken
        let mut extra = doc("00112233445566778899aabbccddeeff", "y");
        extra["nodes"][0]["rssi"] = json!(-50);
        extra["nodes"][0]["short_name"] = json!("<b>");
        extra["label"] = json!("trust me");
        assert!(b.push("lxmf", &extra));
        let s = b.sources();
        assert_eq!(s[0].label, RETICULUM_LABEL);
        assert_eq!(s[0].nodes[0].rssi, None);
        assert_eq!(s[0].nodes[0].short_name, "00112233");
    }
}
