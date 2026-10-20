//! `/api/v1/networks*` (Layer 6 `os/60-transport/README.md` §3, §9, §10; contract
//! `listNetworks`, `listForeignConversations`, `openForeignConversation`,
//! `getForeignConversation`, `sendForeignMessage`).
//!
//! - `GET /networks` → `{schema, networks: [Network]}`: one entry per declared adapter
//!   (disabled optional components included, with their cached and installed size), in
//!   the user's preferred order (`interfaces.toml` `[order]`; networks it does not name
//!   follow in release order). The facts walk the components' installed trees, so the
//!   view is built on a blocking thread.
//! - `GET /networks/{net}/conversations` → the network's conversations (`404` for a
//!   network that is neither declared nor enabled).
//! - `POST /networks/{net}/conversations {kind: "dm", native_id}` → open (or find) a
//!   foreign DM from this node's first account, validated against the network's
//!   recipient syntax. Idempotent; a foreign DM is never a 111 contact.
//! - `GET /networks/{net}/conversations/{conv}?limit&since_seq` → `ForeignThread`
//!   (`limit` 1..=500, default 100). Each own row carries `uncertain` (§4 "may have been
//!   sent; resend?": handed to a carrier that never answered, and not in flight in this
//!   run; such a row without another reason reads `reason_code: handoff_pending`) and
//!   `not_relayed` (`marker`: kept from relaying because it already carries a relay
//!   marker, §8 guard 3). A person's read of the newest window (no `since_seq`, no
//!   bearer) clears the conversation's `unread` (local only; the agent's reads never do).
//! - `POST /networks/{net}/conversations/{conv}/messages {text, size_mode?}` → `202
//!   ForeignSendAccepted {id, ids?, plan}`: a one-target send group (§9). `409 oversize`
//!   in refuse mode, `409 consent_required` for a gate (the text and hash; confirm via
//!   `PATCH /bridge/settings`), `409 component_unavailable` while the network's adapter is
//!   off; adapter *state* (region unset, board away, queue full) is not an error: the row
//!   queues with its reason.
//!
//! `{net}` arrives percent-decoded (`app%3Adeltachat`); a value outside the `NetworkId`
//! pattern names nothing (`404`). `{conv}` is an `fc-` id; anything else is `404`. Every
//! write refuses the agent's bearer (the global gate, §13).

use super::bridge::{bad_request, body, bridge_of, conv_id, not_found};
use super::{ApiRouter, ApiState};
use crate::daemon::bridge::{Bridge, SizeMode};
use crate::daemon::fabric::json as fjson;
use axum::body::Bytes;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

/// `limit` bounds (contract).
pub const LIMIT_MAX: usize = 500;
/// `limit` default.
pub const LIMIT_DEFAULT: usize = 100;

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/networks", get(list))
        .route(
            "/api/v1/networks/{net}/conversations",
            get(conversations).post(open_dm),
        )
        .route("/api/v1/networks/{net}/conversations/{conv}", get(thread))
        .route(
            "/api/v1/networks/{net}/conversations/{conv}/messages",
            post(send),
        )
}

/// The contract's `NetworkId` pattern (`^[a-z0-9][a-z0-9:_-]{0,39}$`).
pub fn valid_network(net: &str) -> bool {
    let b = net.as_bytes();
    !b.is_empty()
        && b.len() <= 40
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b":_-".contains(c))
}

/// `{net}` (and `{conv}`) from the path, or `404`.
fn net_of(p: Result<Path<String>, PathRejection>) -> Result<String, super::Problem> {
    match p {
        Ok(Path(n)) if valid_network(&n) => Ok(n),
        _ => Err(not_found("network")),
    }
}

fn net_conv(
    p: Result<Path<(String, String)>, PathRejection>,
) -> Result<(String, i64), super::Problem> {
    let Ok(Path((net, conv))) = p else {
        return Err(not_found("conversation"));
    };
    if !valid_network(&net) {
        return Err(not_found("network"));
    }
    Ok((net, conv_id(&conv)?))
}

/// Reorder `networks` by the preferred `order` (unnamed ones keep their place after).
pub fn in_order(networks: &mut [Value], order: &[String]) {
    let key = |n: &Value| {
        let id = n["id"].as_str().unwrap_or_default();
        order.iter().position(|o| o == id).unwrap_or(order.len())
    };
    networks.sort_by_key(key);
}

/// The networks view (module doc), built off the runtime's workers.
pub async fn networks_view(api: &ApiState, bridge: Arc<Bridge>) -> Result<Value, Response> {
    let h = tokio::runtime::Handle::current();
    let b = Arc::clone(&bridge);
    let v = tokio::task::spawn_blocking(move || h.block_on(b.networks()))
        .await
        .map_err(|_| {
            super::Problem::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "The networks view failed.",
            )
            .into_response()
        })?
        .map_err(|e| e.problem().into_response())?;
    let mut v = v;
    if let Some(t) = api.ext::<Arc<super::transport::TransportApi>>() {
        let order = t.spine.file().order.networks;
        if let Some(list) = v["networks"].as_array_mut() {
            in_order(list, &order);
        }
    }
    Ok(v)
}

async fn list(State(api): State<Arc<ApiState>>) -> Response {
    let b = match bridge_of(&api) {
        Ok(b) => b,
        Err(p) => return p.into_response(),
    };
    match networks_view(&api, b).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

async fn conversations(
    State(api): State<Arc<ApiState>>,
    net: Result<Path<String>, PathRejection>,
) -> Response {
    let (net, b) = match net_of(net).and_then(|n| Ok((n, bridge_of(&api)?))) {
        Ok(x) => x,
        Err(p) => return p.into_response(),
    };
    match b.conversations(&net).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.problem().into_response(),
    }
}

/// `ForeignDmOpen`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DmOpen {
    kind: String,
    native_id: String,
}

async fn open_dm(
    State(api): State<Arc<ApiState>>,
    net: Result<Path<String>, PathRejection>,
    bytes: Bytes,
) -> Response {
    let (net, b) = match net_of(net).and_then(|n| Ok((n, bridge_of(&api)?))) {
        Ok(x) => x,
        Err(p) => return p.into_response(),
    };
    let req: DmOpen = match body(&bytes) {
        Ok(r) => r,
        Err(p) => return p.into_response(),
    };
    if req.kind != "dm" {
        return bad_request(
            "Only a DM is opened here: channels come from the board, recipient lists from PATCH /bridge/settings.",
        )
        .into_response();
    }
    match b.open_dm(&net, &req.native_id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.problem().into_response(),
    }
}

/// `?limit&since_seq` (`400` for a value out of range or not an integer).
fn page(q: Option<&str>) -> Result<(usize, Option<u64>), super::Problem> {
    let mut limit = LIMIT_DEFAULT;
    let mut since = None;
    for pair in q.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k {
            "limit" => {
                limit = v
                    .parse::<usize>()
                    .ok()
                    .filter(|n| (1..=LIMIT_MAX).contains(n))
                    .ok_or_else(|| bad_request("limit must be 1..=500."))?;
            }
            "since_seq" => {
                since = Some(
                    v.parse::<u64>()
                        .map_err(|_| bad_request("since_seq must be a non-negative integer."))?,
                );
            }
            _ => {}
        }
    }
    Ok((limit, since))
}

/// The `reason_code` an uncertain row reads when nothing else explains it (the UI's
/// "may have been sent; resend?" keys on it, C2).
pub const HANDOFF_PENDING: &str = "handoff_pending";

/// `uncertain` refined by what this run has in flight (a row handed now is not
/// uncertain: its carrier may still answer); an uncertain row without another reason
/// reads `reason_code: handoff_pending`.
fn refine_uncertain(bridge: &Bridge, v: &mut Value) {
    let Some(list) = v["messages"].as_array_mut() else {
        return;
    };
    for m in list {
        if m["uncertain"] == Value::Bool(true) {
            let tracked = m["id"]
                .as_str()
                .and_then(fjson::parse_fm_id)
                .is_some_and(|id| bridge.is_tracked(id));
            if tracked {
                m["uncertain"] = Value::Bool(false);
            } else if m["reason_code"].is_null() {
                m["reason_code"] = Value::from(HANDOFF_PENDING);
                m["reason"] = Value::from(
                    "It may have been sent: it was handed to the carrier and no answer came. \
                     It is never sent again by itself; resend it if you want.",
                );
            }
        }
    }
}

async fn thread(
    State(api): State<Arc<ApiState>>,
    p: Result<Path<(String, String)>, PathRejection>,
    headers: axum::http::HeaderMap,
    RawQuery(q): RawQuery,
) -> Response {
    let ((net, conv), b) = match net_conv(p).and_then(|x| Ok((x, bridge_of(&api)?))) {
        Ok(x) => x,
        Err(p) => return p.into_response(),
    };
    let (limit, since) = match page(q.as_deref()) {
        Ok(x) => x,
        Err(p) => return p.into_response(),
    };
    match b.conversation(&net, conv, since, limit).await {
        Ok(mut v) => {
            refine_uncertain(&b, &mut v);
            // a person reading the newest window has read the conversation (local
            // only: nothing is acknowledged to the foreign side); the agent's reads
            // (a bearer) never clear the user's unread count
            let human = super::auth::bearer_tokens(&headers).is_empty();
            if since.is_none() && human && v["conversation"]["unread"].as_u64() > Some(0) {
                if let Some(f) = api
                    .ext::<Arc<crate::daemon::fabric::FabricHandle>>()
                    .and_then(|h| h.fabric().cloned())
                {
                    if f.foreign_conversation_mark_read(conv).await.is_ok() {
                        v["conversation"]["unread"] = Value::from(0);
                    }
                }
            }
            Json(v).into_response()
        }
        Err(e) => e.problem().into_response(),
    }
}

/// `ForeignSendRequest`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SendReq {
    text: String,
    #[serde(default)]
    size_mode: Option<SizeMode>,
}

async fn send(
    State(api): State<Arc<ApiState>>,
    p: Result<Path<(String, String)>, PathRejection>,
    bytes: Bytes,
) -> Response {
    let ((net, conv), b) = match net_conv(p).and_then(|x| Ok((x, bridge_of(&api)?))) {
        Ok(x) => x,
        Err(p) => return p.into_response(),
    };
    let req: SendReq = match body(&bytes) {
        Ok(r) => r,
        Err(p) => return p.into_response(),
    };
    match b.send_text(&net, conv, &req.text, req.size_mode).await {
        Ok(v) => (StatusCode::ACCEPTED, Json(v)).into_response(),
        Err(e) => e.problem().into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn network_ids_pages_and_order() {
        for ok in ["meshtastic", "lxmf", "app:deltachat", "111", "a-b_c"] {
            assert!(valid_network(ok), "{ok}");
        }
        for bad in ["", "App", ":x", "a b", "a/b", &"x".repeat(41), "é"] {
            assert!(!valid_network(bad), "{bad:?}");
        }
        assert_eq!(page(None).unwrap(), (LIMIT_DEFAULT, None));
        assert_eq!(page(Some("limit=5&since_seq=9")).unwrap(), (5, Some(9)));
        for bad in ["limit=0", "limit=501", "limit=x", "since_seq=-1"] {
            assert_eq!(page(Some(bad)).unwrap_err().status, 400, "{bad}");
        }
        let mut list = vec![
            json!({"id": "meshtastic"}),
            json!({"id": "lxmf"}),
            json!({"id": "app:simplex"}),
            json!({"id": "app:deltachat"}),
        ];
        in_order(
            &mut list,
            &["111".into(), "lxmf".into(), "app:deltachat".into()],
        );
        let ids: Vec<&str> = list.iter().map(|n| n["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["lxmf", "app:deltachat", "meshtastic", "app:simplex"]);
    }
}
