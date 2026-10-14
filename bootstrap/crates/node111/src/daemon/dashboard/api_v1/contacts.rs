//! `/api/v1/contacts*` (`os/messaging/README.md` §7, §15; contract
//! `listContacts`, `addContact`, `getOwnCard`, `getContact`, `patchContact`,
//! `forgetContact`).
//!
//! - `GET /contacts` → `{schema: 1, contacts: [Contact]}` (pending message
//!   requests included, `request: true`; `node1…` contacts waiting for their
//!   card have `has_key: false`).
//! - `POST /contacts` with exactly one of `{card}` (§7.2 (1), `trust = oob`;
//!   scanning a requester's card out of band accepts their request — §7.2
//!   "requests accept = add") or `{node_id}` (64 hex or `node1…`, §7.2 (2))
//!   → `200 Contact` (an already-known contact is returned unchanged).
//!   `409` when a card's keys conflict with the pinned ones (§7.1).
//! - `GET /contacts/me/card` → `Card` (`{text, qr}`; stored announce bytes,
//!   never minted — §7): the identity handle's current announce, so a
//!   settings change that re-minted it (roles, home-hub hint) shows at once;
//!   `409 restore_pending` while the node has none (§4).
//! - `GET /contacts/{id}` · `PATCH /contacts/{id}` (`alias` (null clears,
//!   ≤ 64), `accept: true`, `pinned`, `read_seq`) · `DELETE /contacts/{id}`
//!   (`204`; the pair key is zeroized, history kept until capacity pruning).
//!
//! The agent's bearer may read (§17 `contacts.read`); every write is `401`
//! for it.

use super::messages::{bad_request, handle, refuse_bearer};
use super::{ApiRouter, ApiState, Problem};
use crate::daemon::fabric::{self, json};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use mesh111::fabric::{ContactPatch, FabricError};
use serde_json::{json, Value};
use std::sync::Arc;

/// `Contact.alias` cap (contract `maxLength: 64`, characters).
pub const ALIAS_MAX: usize = 64;

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/contacts", get(list).post(add))
        .route("/api/v1/contacts/me/card", get(own_card))
        .route(
            "/api/v1/contacts/{id}",
            get(get_one).patch(patch).delete(forget),
        )
}

fn not_found() -> Problem {
    Problem::new(StatusCode::NOT_FOUND, "not_found", "No such contact.")
}

fn internal(e: impl std::fmt::Display) -> Problem {
    Problem::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
        "The card could not be drawn.",
    )
    .detail(e.to_string())
}

async fn list(State(api): State<Arc<ApiState>>) -> Response {
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    let Some(f) = h.fabric() else {
        return fabric::not_running().into_response();
    };
    match f.contacts().await {
        Ok(list) => {
            let contacts: Vec<Value> = list.iter().map(json::contact).collect();
            Json(json!({ "schema": 1, "contacts": contacts })).into_response()
        }
        Err(e) => fabric::problem(&e).into_response(),
    }
}

async fn add(State(api): State<Arc<ApiState>>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(p) = refuse_bearer(&headers) {
        return p.into_response();
    }
    let shape = "Body must be exactly one of {\"card\": <card text>} or {\"node_id\": <NodeId>}.";
    let obj = match serde_json::from_slice::<Value>(&body) {
        Ok(Value::Object(o)) if o.len() == 1 => o,
        _ => return bad_request(shape).into_response(),
    };
    let (key, value) = obj.into_iter().next().unwrap_or_default();
    let Some(text) = value.as_str().filter(|t| !t.is_empty()) else {
        return bad_request(shape).into_response();
    };
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    let Some(f) = h.fabric() else {
        return fabric::not_running().into_response();
    };
    let r = match key.as_str() {
        "card" => f.add_contact_card(text).await,
        "node_id" => f.add_contact_node_id(text).await,
        _ => return bad_request(shape).into_response(),
    };
    match r {
        Ok(c) => Json(json::contact(&c)).into_response(),
        Err(e) => fabric::problem(&e).into_response(),
    }
}

async fn own_card(State(api): State<Arc<ApiState>>) -> Response {
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    if h.fabric().is_none() {
        return fabric::not_running().into_response();
    }
    let announce = h.own_announce();
    if announce.is_empty() {
        return Problem::new(
            StatusCode::CONFLICT,
            "restore_pending",
            "This node has no card yet.",
        )
        .detail("Nothing is announced until the identity restore completes (§4).")
        .into_response();
    }
    match json::card(&mesh111::contacts::card_text(&announce)) {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e).into_response(),
    }
}

async fn get_one(State(api): State<Arc<ApiState>>, Path(id): Path<String>) -> Response {
    let Some(id) = json::parse_node_id(&id) else {
        return not_found().into_response();
    };
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    let Some(f) = h.fabric() else {
        return fabric::not_running().into_response();
    };
    match f.contact(id).await {
        Ok(c) => Json(json::contact(&c)).into_response(),
        Err(FabricError::NotFound) => not_found().into_response(),
        Err(e) => fabric::problem(&e).into_response(),
    }
}

/// `ContactPatch` from the body (unknown keys and wrong types refused).
fn parse_patch(body: &[u8]) -> Result<ContactPatch, Problem> {
    let obj = match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(o)) => o,
        _ => return Err(bad_request("Body must be a JSON object.")),
    };
    let mut p = ContactPatch::default();
    for (k, v) in obj {
        match (k.as_str(), v) {
            ("alias", Value::Null) => p.alias = Some(None),
            ("alias", Value::String(s)) => {
                if s.chars().count() > ALIAS_MAX {
                    return Err(bad_request("alias is longer than 64 characters."));
                }
                p.alias = Some(Some(s));
            }
            ("accept", Value::Bool(true)) => p.accept = true,
            ("accept", _) => return Err(bad_request("accept must be true.")),
            ("pinned", Value::Bool(b)) => p.pinned = Some(b),
            ("read_seq", Value::Number(n)) if n.as_u64().is_some() => p.read_seq = n.as_u64(),
            (other, _) => {
                return Err(bad_request(format!(
                    "{other:?} is not a contact field (alias, accept, pinned, read_seq) or has the wrong type."
                )))
            }
        }
    }
    Ok(p)
}

async fn patch(
    State(api): State<Arc<ApiState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(p) = refuse_bearer(&headers) {
        return p.into_response();
    }
    let Some(id) = json::parse_node_id(&id) else {
        return not_found().into_response();
    };
    let patch = match parse_patch(&body) {
        Ok(p) => p,
        Err(p) => return p.into_response(),
    };
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    let Some(f) = h.fabric() else {
        return fabric::not_running().into_response();
    };
    match f.patch_contact(id, patch).await {
        Ok(c) => Json(json::contact(&c)).into_response(),
        Err(FabricError::NotFound) => not_found().into_response(),
        Err(e) => fabric::problem(&e).into_response(),
    }
}

async fn forget(
    State(api): State<Arc<ApiState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(p) = refuse_bearer(&headers) {
        return p.into_response();
    }
    let Some(id) = json::parse_node_id(&id) else {
        return not_found().into_response();
    };
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    let Some(f) = h.fabric() else {
        return fabric::not_running().into_response();
    };
    match f.forget_contact(id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(FabricError::NotFound) => not_found().into_response(),
        Err(e) => fabric::problem(&e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_body() {
        let p =
            parse_patch(br#"{"alias":"Bob","pinned":true,"read_seq":4,"accept":true}"#).unwrap();
        assert_eq!(p.alias, Some(Some("Bob".into())));
        assert_eq!(p.pinned, Some(true));
        assert_eq!(p.read_seq, Some(4));
        assert!(p.accept);
        assert_eq!(parse_patch(br#"{"alias":null}"#).unwrap().alias, Some(None));
        for bad in [
            &br#"{"accept":false}"#[..],
            br#"{"alias":5}"#,
            br#"{"read_seq":-1}"#,
            br#"{"pinned":"yes"}"#,
            br#"{"trust":"oob"}"#,
            br#"[]"#,
        ] {
            assert!(
                parse_patch(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        let long = format!("{{\"alias\":\"{}\"}}", "é".repeat(65));
        assert!(parse_patch(long.as_bytes()).is_err());
        let ok = format!("{{\"alias\":\"{}\"}}", "é".repeat(64));
        assert!(parse_patch(ok.as_bytes()).is_ok());
    }
}
