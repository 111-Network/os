//! `GET /api/v1/nearby` — presence per source (Layer 5 §14 "Nearby", §15;
//! contract `listNearby`) — and `POST /api/v1/nearby/hops` (Layer 7
//! `os/7-relay/README.md` §10.1 rule 1, §13.1; contract `runNearbyHops`).
//! Three kinds of source:
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
//!   A disabled adapter's source is cleared ([`BridgeNearby::clear`]). Its
//!   `hops_away` comes only from a human's hop read (below) and stays with
//!   the node when the node is heard again;
//! - `111` (Layer 7 §2.3): "111 nodes heard by this node", built by
//!   `relay::presence` from the fabric's events (its own path: no adapter can
//!   push it, [`BridgeNearby::push`] drops it), listed last whenever presence
//!   is installed (empty until a fact). Its rows' `id` is the 64-hex NodeId
//!   presence writes from a NodeId; nothing foreign is parsed for it, so the
//!   32-hex check of the `reticulum` ids stays theirs alone.
//!
//! **The hop read (§10.1 rule 1; I6).** `POST /nearby/hops` (a human: the
//! global write gate refuses the agent's bearer, 401) runs one read-only
//! `rnpath -t -j` through the `rnstatus` runner, its timeout and its owner
//! guard ([`Rnstatus::path_table`]), off the request path; the hop counts
//! land in `hops_away` of the `reticulum` source only and go out on the
//! `nearby` frame; a failed read (a spawn failure or timeout, a non-zero
//! exit, a table that does not parse or is over its own cap) changes
//! nothing and is one fixed log line (the contract's 202 has no later
//! answer). `202 RelayAccepted` once started; `409
//! component_unavailable` while the Reticulum component (the LXMF helper, the
//! source's only producer) is off, the transport spine is absent, or the
//! supervised rnsd does not hold the shared instance (nothing spawned); `429
//! rate_limited` within [`HOPS_MIN_INTERVAL`] of the last run. A `GET` never
//! spawns anything.
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

use super::transport::TransportApi;
use super::{ApiRouter, ApiState, Problem};
use crate::daemon::bridge::{Bridge, NearbySink, NETWORK_111, NETWORK_LXMF};
use crate::daemon::meshtastic::presence::{sanitize_name, NearbyNode, NearbySource};
use crate::daemon::meshtastic::Meshtastic;
use crate::daemon::relay::presence::Presence;
use crate::daemon::transport::rnstatus::{PathHops, Rnstatus, RnstatusError};
use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

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
/// `POST /nearby/hops` answers `429 rate_limited` within this long of the last run (§13.1).
pub const HOPS_MIN_INTERVAL: Duration = Duration::from_secs(30);

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
    /// When the last hop read started (`POST /nearby/hops`, §13.1): the 30 s window.
    hops_run: Mutex<Option<Instant>>,
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
        for mut n in nodes {
            match src.nodes.iter().position(|x| x.id == n.id) {
                Some(i) => {
                    let old = src.nodes.remove(i);
                    // the hop count is the human's last read (§10.1), never the adapter's
                    n.hops_away = old.as_ref().and_then(|o| o.hops_away);
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

    /// One hop read's result (§10.1 rule 1): every `reticulum` node's `hops_away` becomes its
    /// count in `table`, or unknown when the table does not list it (the read replaces the last
    /// one). Only the `reticulum` sources are touched; no node is added. Returns whether what
    /// `GET /nearby` shows changed.
    pub fn set_hops(&self, table: &PathHops) -> bool {
        let mut all = self.lock();
        let mut changed = false;
        for src in all.values_mut().filter(|s| s.network == NETWORK_LXMF) {
            for n in src.nodes.iter_mut() {
                let hops = table.get(&n.id).copied();
                if n.hops_away != hops {
                    n.hops_away = hops;
                    changed = true;
                }
            }
        }
        if changed {
            self.version.fetch_add(1, Ordering::SeqCst);
        }
        changed
    }

    /// Reserve a hop read at `now`: `Err(retry_after_s)` within [`HOPS_MIN_INTERVAL`] of the last
    /// one; `Ok(previous)` holds what [`BridgeNearby::cancel_hops`] restores when nothing ran.
    fn begin_hops(&self, now: Instant) -> Result<Option<Instant>, u64> {
        let mut last = self.hops_run.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(t) = *last {
            let since = now.saturating_duration_since(t);
            if since < HOPS_MIN_INTERVAL {
                let left = HOPS_MIN_INTERVAL - since;
                return Err(left.as_secs() + u64::from(left.subsec_nanos() > 0));
            }
        }
        Ok(last.replace(now))
    }

    /// Undo a reservation that spawned nothing (the owner gate refused).
    fn cancel_hops(&self, previous: Option<Instant>) {
        *self.hops_run.lock().unwrap_or_else(|p| p.into_inner()) = previous;
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

/// Every source `GET /nearby` lists: the Meshtastic module's, then the bridge's, then the `111`
/// source while presence is installed (Layer 7 §2.3).
pub fn sources(api: &ApiState) -> Vec<Value> {
    let mut out: Vec<NearbySource> = api
        .ext::<Arc<Meshtastic>>()
        .map(|m| m.nearby_sources())
        .unwrap_or_default();
    if let Some(b) = api.ext::<Arc<BridgeNearby>>() {
        out.extend(b.sources());
    }
    let mut out: Vec<Value> = out
        .iter()
        .filter_map(|s| serde_json::to_value(s).ok())
        .collect();
    if let Some(p) = api.ext::<Arc<Presence>>() {
        out.push(p.source_now());
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

/// `409 component_unavailable` for the hop read (module doc); nothing was spawned.
fn hops_unavailable(detail: &str) -> Problem {
    Problem::new(
        StatusCode::CONFLICT,
        "component_unavailable",
        "Reticulum hop counts are not available now.",
    )
    .detail(detail.to_string())
}

/// Why the hop read was not started (module doc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HopsRefusal {
    /// `409 component_unavailable`, with a plain detail.
    Unavailable(&'static str),
    /// `429 rate_limited`, with the seconds left.
    RateLimited(u64),
}

impl IntoResponse for HopsRefusal {
    fn into_response(self) -> Response {
        match self {
            HopsRefusal::Unavailable(why) => hops_unavailable(why).into_response(),
            HopsRefusal::RateLimited(left) => {
                let mut resp = Problem::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limited",
                    "Hop counts were read less than 30 seconds ago.",
                )
                .detail(format!("Wait {left} s before reading them again."))
                .into_response();
                resp.headers_mut()
                    .insert(header::RETRY_AFTER, HeaderValue::from(left));
                resp
            }
        }
    }
}

/// Start one hop read (§10.1 rule 1): the component checks, the 30 s window, the owner gate
/// (before anything is spawned; a refusal there releases the window), then ONE blocking task that
/// runs `rnpath -t -j`, writes the counts into `heard` and, on a change, publishes the `nearby`
/// frame through `publish` (the daemon's: [`publish`] on the API state, held weakly). The
/// returned handle answers whether the read changed what `GET /nearby` shows (tests await it).
pub async fn start_hops(
    lxmf_enabled: bool,
    rn: Option<Arc<Rnstatus>>,
    heard: Option<Arc<BridgeNearby>>,
    publish: impl Fn() + Send + 'static,
) -> Result<tokio::task::JoinHandle<bool>, HopsRefusal> {
    if !lxmf_enabled {
        return Err(HopsRefusal::Unavailable(
            "The Reticulum component (the LXMF helper) is off.",
        ));
    }
    let (Some(rn), Some(heard)) = (rn, heard) else {
        return Err(HopsRefusal::Unavailable(
            "The Reticulum transport is not running on this node.",
        ));
    };
    let previous = heard
        .begin_hops(Instant::now())
        .map_err(HopsRefusal::RateLimited)?;
    let gate = {
        let rn = Arc::clone(&rn);
        tokio::task::spawn_blocking(move || rn.gate()).await
    };
    if !matches!(gate, Ok(Ok(()))) {
        heard.cancel_hops(previous);
        return Err(HopsRefusal::Unavailable(
            "The supervised rnsd does not hold the shared Reticulum instance.",
        ));
    }
    Ok(tokio::task::spawn_blocking(move || match rn.path_table() {
        Ok(table) => {
            let changed = heard.set_hops(&table);
            if changed {
                publish();
            }
            changed
        }
        Err(e) => {
            // a fixed line: nothing rnpath printed reaches the log
            let why = match e {
                RnstatusError::Exit { status, .. } => format!("rnpath exited {status}"),
                RnstatusError::Spawn(_) => {
                    "rnpath did not run or was stopped at the timeout".into()
                }
                RnstatusError::NotOwned(_) => {
                    "the supervised rnsd no longer holds the instance".into()
                }
                RnstatusError::NoInstance | RnstatusError::NotFound(_) => {
                    "no shared rnsd instance answered".into()
                }
                // ours, a fixed phrase: a table over its cap says so
                RnstatusError::Unparsed(what) => format!("{what} did not parse"),
            };
            eprintln!("111 nearby: the hop read failed: {why}");
            false
        }
    }))
}

/// `POST /api/v1/nearby/hops` (module doc): a human's; `202 RelayAccepted` once started.
async fn hops(State(api): State<Arc<ApiState>>) -> Response {
    let lxmf_enabled = api
        .ext::<Arc<Bridge>>()
        .is_some_and(|b| b.enabled().iter().any(|n| n == NETWORK_LXMF));
    let rn = api.ext::<Arc<TransportApi>>().map(|t| t.spine.rnstatus());
    let heard = api.ext::<Arc<BridgeNearby>>();
    let weak: Weak<ApiState> = Arc::downgrade(&api);
    let publish_frame = move || {
        if let Some(api) = weak.upgrade() {
            publish(&api);
        }
    };
    match start_hops(lxmf_enabled, rn, heard, publish_frame).await {
        Ok(_) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "schema": 1, "started": true })),
        )
            .into_response(),
        Err(r) => r.into_response(),
    }
}

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/nearby", get(list))
        .route("/api/v1/nearby/hops", post(hops))
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

    fn api_state() -> (Arc<ApiState>, tempfile::TempDir) {
        use crate::daemon::dashboard::{ApiConfig, DashboardState};
        use crate::daemon::DaemonLayout;
        let dir = tempfile::tempdir().unwrap();
        let layout = DaemonLayout {
            root: dir.path().to_path_buf(),
            etc: dir.path().join("etc"),
        };
        let dash = Arc::new(std::sync::RwLock::new(DashboardState::new("2026.11.13")));
        (ApiState::new(dash, ApiConfig::new(layout, false)), dir)
    }

    /// Layer 7 §2.3 (A1's read): the `111` source is listed last whenever presence is installed,
    /// empty until a fact, beside the bridge's sources; without presence it is absent.
    #[test]
    fn the_111_source_is_listed_last_and_empty_until_a_fact() {
        let (api, _dir) = api_state();
        let heard = Arc::new(BridgeNearby::default());
        assert!(heard.push("lxmf", &doc(D, "Bob")));
        api.insert_ext(Arc::clone(&heard));
        let kinds = |api: &ApiState| -> Vec<String> {
            sources(api)
                .iter()
                .map(|s| s["kind"].as_str().unwrap_or("").to_string())
                .collect()
        };
        assert_eq!(kinds(&api), ["reticulum"]);
        api.insert_ext(Arc::new(Presence::default()));
        assert_eq!(kinds(&api), ["reticulum", "111"]);
        let s = sources(&api);
        assert_eq!(s[1]["label"], "111 nodes heard by this node");
        assert_eq!(s[1]["device_id"], Value::Null);
        assert_eq!(s[1]["nodes"], json!([]));
        // the reticulum rows keep their contract shape through the Value form
        assert_eq!(s[0]["nodes"][0]["id"], D);
        assert_eq!(s[0]["nodes"][0]["hops_away"], Value::Null);
        // the frame carries the same list
        let head = api.events.head();
        publish(&api);
        let sub = api.events.subscribe(Some(head));
        let frame = sub
            .replay
            .iter()
            .find(|f| f.event == "nearby")
            .expect("frame");
        let v: Value = serde_json::from_str(&frame.data).unwrap();
        assert_eq!(v["sources"][1]["kind"], "111");
    }

    /// §10.1 rule 1: a hop read sets `hops_away` of the `reticulum` nodes only (unlisted:
    /// unknown), adds no node, and the count stays with a node heard again.
    #[test]
    fn hop_counts_land_on_reticulum_nodes_only() {
        let b = BridgeNearby::default();
        let other = "00112233445566778899aabbccddeeff";
        assert!(b.push("lxmf", &doc(D, "Bob")));
        assert!(b.push("lxmf", &doc(other, "Ann")));
        let mut t = PathHops::new();
        t.insert(D.to_string(), 2);
        t.insert("ffffffffffffffffffffffffffffffff".into(), 1);
        let v = b.version();
        assert!(b.set_hops(&t));
        assert!(b.version() > v);
        let hops = |b: &BridgeNearby| -> Vec<(String, Option<u8>)> {
            b.sources()[0]
                .nodes
                .iter()
                .map(|n| (n.id.clone(), n.hops_away))
                .collect()
        };
        assert_eq!(
            hops(&b),
            [(other.to_string(), None), (D.to_string(), Some(2))],
            "no node added from the table"
        );
        assert!(!b.set_hops(&t), "the same read: no change");
        // heard again: the count stays (the adapter never reports one)
        assert!(!b.push("lxmf", &doc(D, "Bob")), "same node, same count");
        assert!(b.push("lxmf", &doc(D, "Robert")));
        assert_eq!(hops(&b)[0], (D.to_string(), Some(2)));
        // a later read that no longer lists it: unknown
        assert!(b.set_hops(&PathHops::new()));
        assert!(hops(&b).iter().all(|(_, h)| h.is_none()));
    }

    /// §13.1: `429` within 30 s of the last run, with the seconds left (rounded up); a refusal
    /// that spawned nothing gives the window back.
    #[test]
    fn the_hop_read_window_is_thirty_seconds() {
        let b = BridgeNearby::default();
        let t0 = Instant::now();
        assert_eq!(b.begin_hops(t0), Ok(None));
        assert_eq!(b.begin_hops(t0 + Duration::from_secs(10)), Err(20));
        assert_eq!(b.begin_hops(t0 + Duration::from_millis(29_500)), Err(1));
        assert_eq!(b.begin_hops(t0 + HOPS_MIN_INTERVAL), Ok(Some(t0)));
        let t1 = t0 + HOPS_MIN_INTERVAL;
        b.cancel_hops(Some(t0));
        assert_eq!(b.begin_hops(t1), Ok(Some(t0)), "cancelled: free again");
        let r = HopsRefusal::RateLimited(12).into_response();
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(r.headers()[header::RETRY_AFTER], "12");
        let r = HopsRefusal::Unavailable("off").into_response();
        assert_eq!(r.status(), StatusCode::CONFLICT);
    }

    fn rnpath_cmd(root: &std::path::Path) -> String {
        format!(
            "{} --config {} -t -j",
            crate::daemon::transport::rnstatus::rnpath_program(root).display(),
            root.join("rns").display()
        )
    }

    /// §10.1 rule 1, G8 (I6): one `rnpath` per run through the rnstatus runner, the counts on
    /// the `reticulum` source and one published frame; a second POST within 30 s is `429` and
    /// spawns nothing.
    #[tokio::test]
    async fn a_hop_read_runs_rnpath_once_and_publishes_once() {
        use crate::testkit::MockRunner;
        use crate::util::run::Output;
        let dir = tempfile::tempdir().unwrap();
        let table = format!(
            r#"[{{"hash":"{D}","timestamp":1.5,"via":"{ME}","hops":3,"expires":9.0,"interface":"x"}}]"#
        );
        let runner = Arc::new(MockRunner::new(vec![(
            rnpath_cmd(dir.path()),
            Output {
                status: 0,
                stdout: table,
                stderr: String::new(),
            },
        )]));
        let rn = Arc::new(Rnstatus::new(
            Arc::clone(&runner) as Arc<dyn crate::util::run::Runner>,
            dir.path(),
        ));
        let heard = Arc::new(BridgeNearby::default());
        assert!(heard.push("lxmf", &doc(D, "Bob")));
        let published = Arc::new(AtomicU64::new(0));
        let count = {
            let p = Arc::clone(&published);
            move || {
                p.fetch_add(1, Ordering::SeqCst);
            }
        };
        let run = start_hops(true, Some(Arc::clone(&rn)), Some(Arc::clone(&heard)), count)
            .await
            .expect("started");
        assert!(run.await.expect("joined"), "the count changed the source");
        assert_eq!(published.load(Ordering::SeqCst), 1);
        assert_eq!(heard.sources()[0].nodes[0].hops_away, Some(3));
        let again = start_hops(true, Some(rn), Some(heard), || {}).await;
        assert!(
            matches!(again, Err(HopsRefusal::RateLimited(s)) if (1..=30).contains(&s)),
            "{again:?}"
        );
        assert_eq!(runner.calls().len(), 1, "one spawn for one accepted POST");
        runner.assert_exhausted();
    }

    /// §13.1: `409 component_unavailable` while the Reticulum component is off, without the
    /// transport spine, or while the supervised rnsd does not hold the instance; nothing spawned,
    /// and the 30 s window is not taken by a refusal.
    #[tokio::test]
    async fn the_hop_read_is_refused_while_the_component_is_off() {
        use crate::testkit::MockRunner;
        let dir = tempfile::tempdir().unwrap();
        let none: Vec<(String, crate::util::run::Output)> = Vec::new();
        let runner = Arc::new(MockRunner::new(none));
        let rn = Arc::new(Rnstatus::new(
            Arc::clone(&runner) as Arc<dyn crate::util::run::Runner>,
            dir.path(),
        ));
        let heard = Arc::new(BridgeNearby::default());
        let off = start_hops(
            false,
            Some(Arc::clone(&rn)),
            Some(Arc::clone(&heard)),
            || {},
        )
        .await;
        assert!(matches!(off, Err(HopsRefusal::Unavailable(_))), "{off:?}");
        let no_spine = start_hops(true, None, Some(Arc::clone(&heard)), || {}).await;
        assert!(matches!(no_spine, Err(HopsRefusal::Unavailable(_))));
        // FA1's gate: no supervised rnsd pid, so nothing may run
        rn.set_owner_gate(Arc::new(|| None));
        for _ in 0..2 {
            let gated =
                start_hops(true, Some(Arc::clone(&rn)), Some(Arc::clone(&heard)), || {}).await;
            assert!(
                matches!(gated, Err(HopsRefusal::Unavailable(_))),
                "never 429 after a refusal: {gated:?}"
            );
        }
        assert!(runner.calls().is_empty(), "nothing spawned");
        let r = HopsRefusal::Unavailable("x").into_response();
        assert_eq!(r.status(), StatusCode::CONFLICT);
    }
}
