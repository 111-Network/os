//! `/api/v1/messages*` (`os/5-messaging/README.md` §15; contract
//! `listThreads`, `getThread`, `sendMessage`, `retryMessage`,
//! `cancelMessage`).
//!
//! - `GET /messages/threads` → `{schema: 1, threads: [Thread]}`, most recent
//!   activity first (message-request threads included).
//! - `GET /messages/threads/{peer}?limit=&since_seq=` → `{schema, peer,
//!   request, messages, more}`, ascending by `local_seq`. Without `since_seq`
//!   the newest `limit` (1–500, default 100); with it the first `limit` above
//!   that sequence (`more` = newer ones exist). A peer that is not 64
//!   lowercase hex, or names no thread, is `404`.
//! - `POST /messages {to, text}` → `202 {schema, id}`: the fabric numbers and
//!   commits the message (§4) and it stays `queued` until a path exists.
//!   `404` when `to` is not a contact, `400` for an empty or oversize text,
//!   `409 restore_pending` while the identity holder reports a restore floor
//!   (nothing is numbered then — §4; additive contract code, F2).
//! - `POST /messages/{id}/retry` → `200 Message` (`409` unless an own
//!   message is `queued` or `sent`).
//! - `POST /messages/{id}/cancel` → `202 Message` (additive, lead's
//!   Wave 2 call): an own message still `queued` with nothing sent yet is
//!   cancelled through `Fabric::cancel` — §4: an allocated number is never
//!   abandoned, so an empty tombstone goes in its place and the message reads
//!   `queued`, `reason_code: cancelled` until it leaves. Any other state, or
//!   an incoming message, is `409 bad_request`; an unknown id `404`.
//!
//! - **Foreign rows** (Layer 6 `6-transport/README.md` §4, §9, §10; W1): the
//!   same two routes take a `fm-…` id (`ForeignMessageId`, [`json::parse_fm_id`]);
//!   every other id keeps the Layer 5 path above unchanged. `retry` is
//!   `Bridge::resend`: an own row that is uncertain ("may have been sent;
//!   resend?", `handoff_pending`) or `failed` is sent again as a new row (the
//!   old one keeps its state and records `meta.resent_as`); the answer is the
//!   bridge's `ForeignSendAccepted` `{schema, id, plan}` with the NEW row's id
//!   (the contract's `Message` cannot carry a foreign row; the UI re-reads the
//!   thread and ignores the body). Any other state is `409 bad_request`, a
//!   relay copy too; a consent gate or an oversize text answers its own `409`.
//!   `cancel` is `Bridge::cancel`: an own row still `queued` and never handed
//!   to a carrier is deleted (§9: nothing was numbered on the foreign side), the
//!   answer `202 {schema, id}`; anything else is `409 bad_request` with the
//!   Layer 5 copy, an unknown id `404`.
//!
//! Auth: behind `require_session` like every gated route; the agent's bearer
//! may **read** (§17) but never send, retry or cancel — it proposes (`kind:
//! message`) and the human accepts in the UI (§18 rule 9): the global write
//! gate (`auth::bearer_write_refused`, first in `require_session`) answers a
//! bearer on any write `401` before a handler runs.

use super::{ApiRouter, ApiState, Problem};
use crate::daemon::bridge::{Bridge, BridgeError};
use crate::daemon::fabric::{self, json, FabricHandle};
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use mesh111::fabric::FabricError;
use mesh111::store::Dir;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

/// `limit` bounds (contract).
pub const LIMIT_MAX: usize = 500;
/// `limit` default.
pub const LIMIT_DEFAULT: usize = 100;
/// Rows a `since_seq` page may scan (newest first) before it answers with
/// the oldest page it reached (`more: true`).
pub const SINCE_SCAN_MAX: usize = 4096;

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/messages/threads", get(threads))
        .route("/api/v1/messages/threads/{peer}", get(thread))
        .route("/api/v1/messages", post(send))
        .route("/api/v1/messages/{id}/retry", post(retry))
        .route("/api/v1/messages/{id}/cancel", post(cancel))
}

/// The fabric handle, or the `503` problem.
pub(crate) fn handle(api: &ApiState) -> Result<Arc<FabricHandle>, Problem> {
    api.ext::<Arc<FabricHandle>>()
        .filter(|h| h.running())
        .ok_or_else(fabric::not_running)
}

pub(crate) fn bad_request(detail: impl Into<String>) -> Problem {
    Problem::new(
        StatusCode::BAD_REQUEST,
        "bad_request",
        "The request was refused.",
    )
    .detail(detail)
}

fn not_found(what: &str) -> Problem {
    Problem::new(
        StatusCode::NOT_FOUND,
        "not_found",
        format!("No such {what}."),
    )
}

async fn threads(State(api): State<Arc<ApiState>>) -> Response {
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    let Some(f) = h.fabric() else {
        return fabric::not_running().into_response();
    };
    match f.threads().await {
        Ok(list) => {
            let mut at = |id: i64| h.at_of(id);
            let threads: Vec<Value> = list.iter().map(|t| json::thread(t, &mut at)).collect();
            Json(json!({ "schema": 1, "threads": threads })).into_response()
        }
        Err(e) => fabric::problem(&e).into_response(),
    }
}

/// `limit` / `since_seq` from the raw query (unknown keys ignored).
fn page_params(q: Option<&str>) -> Result<(usize, Option<u64>), Problem> {
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
                    .ok_or_else(|| bad_request("limit must be an integer from 1 to 500."))?;
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

async fn thread(
    State(api): State<Arc<ApiState>>,
    Path(peer): Path<String>,
    RawQuery(q): RawQuery,
) -> Response {
    let (limit, since) = match page_params(q.as_deref()) {
        Ok(x) => x,
        Err(p) => return p.into_response(),
    };
    let Some(peer) = json::parse_node_id(&peer) else {
        return not_found("conversation").into_response();
    };
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    let Some(f) = h.fabric() else {
        return fabric::not_running().into_response();
    };
    let page = match since {
        None => f.thread(peer, limit).await,
        // validates the peer and reads `request`; the window comes below
        Some(_) => f.thread(peer, 1).await,
    };
    let page = match page {
        Ok(p) => p,
        Err(FabricError::NotFound) => return not_found("conversation").into_response(),
        Err(e) => return fabric::problem(&e).into_response(),
    };
    let (messages, more) = match since {
        None => (page.messages, page.more),
        Some(after) => match since_window(&h, peer, after, limit).await {
            Ok(x) => x,
            Err(e) => return fabric::problem(&e).into_response(),
        },
    };
    let messages: Vec<Value> = messages
        .iter()
        .filter(|m| json::shown(m))
        .map(|m| h.message_json(m))
        .collect();
    Json(json!({
        "schema": 1,
        "peer": json::hex(&peer),
        "request": page.request,
        "messages": messages,
        "more": more,
    }))
    .into_response()
}

/// The first `limit` messages with `peer` above `after` (`local_seq`), and
/// whether newer ones exist. The store answers newest-N pages only, so the
/// page grows (doubling, inside the fabric actor) until it reaches `after`,
/// at most [`SINCE_SCAN_MAX`] rows — past that the oldest reached page is
/// answered with `more: true` (recorded for the lead: a `messages_after`
/// store query would make this exact).
async fn since_window(
    h: &FabricHandle,
    peer: id111::keyset::NodeId,
    after: u64,
    limit: usize,
) -> Result<(Vec<mesh111::fabric::MessageView>, bool), FabricError> {
    let f = h.fabric().ok_or(FabricError::Stopped)?;
    let rows = f
        .with_store(move |s| {
            let mut k = (limit + 1).min(SINCE_SCAN_MAX);
            loop {
                let rows = s.messages_newest(peer.as_bytes(), k)?;
                let reached = rows.len() < k || rows.first().is_none_or(|r| r.local_seq <= after);
                if reached || k >= SINCE_SCAN_MAX {
                    let above: Vec<_> = rows.into_iter().filter(|r| r.local_seq > after).collect();
                    return Ok::<_, mesh111::store::StoreError>(above);
                }
                k = (k * 2).min(SINCE_SCAN_MAX);
            }
        })
        .await??;
    let more = rows.len() > limit;
    let views = rows.iter().take(limit).map(view_of_row).collect::<Vec<_>>();
    Ok((views, more))
}

/// The fabric's `MessageView` of a store row (mirrors `mesh111::fabric`'s
/// private conversion: `MSG` bodies decoded, the claim only for `in`).
fn view_of_row(row: &mesh111::store::MessageRow) -> mesh111::fabric::MessageView {
    use mesh111::envelope::Kind;
    use mesh111::outbox::{MessageBody, Reason};
    let body = row.body.clone().unwrap_or_default();
    let (text, sent_at) = if row.kind == Kind::Msg.byte() {
        match MessageBody::decode(&body) {
            Ok(b) => (Some(b.text), b.sent_at),
            Err(_) => (None, None),
        }
    } else {
        (None, None)
    };
    mesh111::fabric::MessageView {
        id: row.id,
        peer: id111::keyset::NodeId::from(row.peer),
        dir: row.dir,
        kind: row.kind,
        text,
        tombstone: body.is_empty(),
        state: row.state,
        reason: row.reason.as_deref().and_then(Reason::from_code),
        local_seq: row.local_seq,
        received_seq: row.received_local_seq,
        sent_at_claim: if row.dir == Dir::In { sent_at } else { None },
        stream: row.stream,
        seq: mesh111::ids::MsgSeq::new(row.seq.epoch, u32::try_from(row.seq.n).unwrap_or(u32::MAX)),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendReq {
    to: String,
    text: String,
}

async fn send(State(api): State<Arc<ApiState>>, body: Bytes) -> Response {
    let req: SendReq = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => {
            return bad_request("Body must be {\"to\": <NodeId>, \"text\": <non-empty string>}.")
                .into_response()
        }
    };
    let Some(to) = json::parse_node_id(&req.to) else {
        return bad_request("`to` must be a NodeId: 64 lowercase hex.").into_response();
    };
    if req.text.is_empty() {
        return bad_request("`text` must not be empty.").into_response();
    }
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    match h.send(to, &req.text).await {
        Ok(id) => (
            StatusCode::ACCEPTED,
            Json(json!({ "schema": 1, "id": json::msg_id(id) })),
        )
            .into_response(),
        Err(FabricError::NotFound) => Problem::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "That node is not a contact.",
        )
        .detail("Add them in Contacts first.")
        .into_response(),
        Err(e) => fabric::problem(&e).into_response(),
    }
}

async fn retry(State(api): State<Arc<ApiState>>, Path(id): Path<String>) -> Response {
    if id.starts_with(FOREIGN_PREFIX) {
        return foreign_retry(&api, &id).await;
    }
    let Some(id) = json::parse_msg_id(&id) else {
        return not_found("message").into_response();
    };
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    let Some(f) = h.fabric() else {
        return fabric::not_running().into_response();
    };
    match f.retry(id).await {
        Ok(v) => Json(h.message_json(&v)).into_response(),
        Err(FabricError::NotFound) => not_found("message").into_response(),
        Err(e) => fabric::problem(&e).into_response(),
    }
}

async fn cancel(State(api): State<Arc<ApiState>>, Path(id): Path<String>) -> Response {
    if id.starts_with(FOREIGN_PREFIX) {
        return foreign_cancel(&api, &id).await;
    }
    let Some(id) = json::parse_msg_id(&id) else {
        return not_found("message").into_response();
    };
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    let Some(f) = h.fabric() else {
        return fabric::not_running().into_response();
    };
    match f.cancel(id).await {
        Ok(v) => (StatusCode::ACCEPTED, Json(h.message_json(&v))).into_response(),
        // `NotFound` from the fabric: not an own message in flight — it
        // exists (incoming, or done) ⇒ 409; it does not ⇒ 404
        Err(FabricError::NotFound) => match f.message(id).await {
            Ok(_) => not_cancellable().into_response(),
            Err(FabricError::NotFound) => not_found("message").into_response(),
            Err(e) => fabric::problem(&e).into_response(),
        },
        Err(FabricError::NotRetryable) => not_cancellable().into_response(),
        Err(e) => fabric::problem(&e).into_response(),
    }
}

/// The `409` for a message that is not cancellable (Layer 5 copy; foreign rows share it).
fn not_cancellable() -> Problem {
    Problem::new(
        StatusCode::CONFLICT,
        "bad_request",
        "Only your own message that has not left yet can be cancelled.",
    )
    .detail("It was already sent (or it is an incoming message); it cannot be called back.")
}

// ---------------------------------------------------------------- foreign rows (Layer 6)

/// The prefix of a `ForeignMessageId` (contract): such an id never takes the Layer 5 path.
pub const FOREIGN_PREFIX: &str = "fm-";

/// `fm-<n>` → the foreign row id, the bridge core, or the problem (`404` for an id that
/// names nothing; `503` without a bridge).
fn foreign_target(api: &ApiState, id: &str) -> Result<(i64, Arc<Bridge>), Problem> {
    let id = json::parse_fm_id(id).ok_or_else(|| not_found("message"))?;
    Ok((id, super::bridge::bridge_of(api)?))
}

/// `POST /messages/fm-…/retry` (module doc): `Bridge::resend`.
async fn foreign_retry(api: &ApiState, id: &str) -> Response {
    let (id, b) = match foreign_target(api, id) {
        Ok(x) => x,
        Err(p) => return p.into_response(),
    };
    match b.resend(id).await {
        Ok(v) => Json(v).into_response(),
        Err(BridgeError::NotFound(_) | BridgeError::Fabric(FabricError::NotFound)) => {
            not_found("message").into_response()
        }
        // not an own row waiting for a decision, or a relay copy: the Layer 5 code
        Err(BridgeError::BadRequest(why)) => Problem::new(
            StatusCode::CONFLICT,
            "bad_request",
            "This message cannot be sent again in its current state.",
        )
        .detail(why)
        .into_response(),
        Err(e) => e.into_response(),
    }
}

/// `POST /messages/fm-…/cancel` (module doc): `Bridge::cancel`.
async fn foreign_cancel(api: &ApiState, id: &str) -> Response {
    let (row, b) = match foreign_target(api, id) {
        Ok(x) => x,
        Err(p) => return p.into_response(),
    };
    match b.cancel(row).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(json!({ "schema": 1, "id": json::fm_id(row) })),
        )
            .into_response(),
        Err(BridgeError::NotFound(_) | BridgeError::Fabric(FabricError::NotFound)) => {
            not_found("message").into_response()
        }
        // an inbound row, a relay copy, a handed, accepted or sent row (refused by the
        // fabric or the store)
        Err(
            BridgeError::Fabric(FabricError::NotRetryable | FabricError::Invalid(_))
            | BridgeError::BadRequest(_),
        ) => not_cancellable().into_response(),
        Err(e) => e.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_query() {
        assert_eq!(page_params(None).unwrap(), (100, None));
        assert_eq!(
            page_params(Some("limit=5&since_seq=9&x=1")).unwrap(),
            (5, Some(9))
        );
        for bad in [
            "limit=0",
            "limit=501",
            "limit=x",
            "since_seq=-1",
            "since_seq=",
        ] {
            assert_eq!(
                page_params(Some(bad)).unwrap_err().code,
                "bad_request",
                "{bad}"
            );
        }
    }
}
