//! Layer 7 §13.1, §4.6 (row D6): the pairing routes. The work is `daemon::relay::pair`'s; this
//! file reads the bodies and answers the contract's shapes (`offerRelayPair`, `pairRelayHub`,
//! `unpairRelayHub`).
//!
//! - `POST /api/v1/relay/pair/offer {role, via}` → `200 PairOffer {schema, bundle: Card, code,
//!   link_spki}` (`Cache-Control: no-store`: the code is shown once); dismissed with the Layer 5
//!   `DELETE /fabric/hub/offer`. `409 needs_hub` without the hub role, `409 pair_role_mismatch`
//!   for a pair §4.6 forbids (`carrier` ⇒ `peer`; `leaf`, `gateway` ⇒ `tcp`), `409
//!   restore_pending` with no card yet.
//! - `POST /api/v1/relay/pair {bundle, role, via, lan?, accept_warning?}` → `202 PairedHub`:
//!   the enrolment and the first connection finish in the background. `409 pair_role_mismatch`
//!   (nothing recorded), `409 needs_hub`, `409 consent_required` (`pair_plain`), `409
//!   bad_request` (already paired with that hub), `400` for a malformed body or bundle, `502
//!   upstream_error` when the far hub refuses the code within the first wait.
//! - `DELETE /api/v1/relay/paired/{node_id}` → `204`, idempotent: the pair and its pinned key
//!   are forgotten; the forward rows held for it return to the other targets.
//!
//! All three are loopback-only (I15: `403 loopback_only` on a LAN session, like their Layer 5
//! siblings); the agent's bearer is refused on each (`401`, the global write gate in
//! `auth::require_session`), so no code or bundle is ever taken from the agent (row A1: its
//! `hub.pair {}` proposal only opens the pairing screen).

use super::auth::LoopbackOnly;
use super::messages::{bad_request, handle};
use super::{ApiRouter, ApiState, Problem};
use crate::daemon::fabric::json;
use crate::daemon::relay::pair::{self, PairAttempts, PairPick, BUNDLE_MAX};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, post};
use axum::Json;
use mesh111::fabric::{PairRole, PairVia};
use serde_json::Value;
use std::sync::Arc;

/// The Layer 7 pairing routes (§13.1).
pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/relay/pair/offer", post(offer))
        .route("/api/v1/relay/pair", post(pair_hub))
        .route("/api/v1/relay/paired/{node_id}", delete(unpair))
}

fn role_of(v: &Value) -> Option<PairRole> {
    v.as_str().and_then(PairRole::from_code)
}

fn via_of(v: &Value) -> Option<PairVia> {
    v.as_str().and_then(PairVia::from_code)
}

/// `PairOfferRequest`: exactly `{role, via}`.
fn parse_offer(body: &[u8]) -> Result<(PairRole, PairVia), Problem> {
    let shape =
        "Body must be {\"role\": \"peer\"|\"leaf\"|\"gateway\", \"via\": \"tcp\"|\"carrier\"}.";
    let obj = match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(o)) => o,
        _ => return Err(bad_request(shape)),
    };
    let (mut role, mut via) = (None, None);
    for (k, v) in obj {
        match k.as_str() {
            "role" => role = Some(role_of(&v).ok_or_else(|| bad_request(shape))?),
            "via" => via = Some(via_of(&v).ok_or_else(|| bad_request(shape))?),
            _ => return Err(bad_request(shape)),
        }
    }
    match (role, via) {
        (Some(r), Some(v)) => Ok((r, v)),
        _ => Err(bad_request(shape)),
    }
}

/// 64 lowercase hex characters (the contract's `Sha256Hex`).
fn sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `PairRequest`: `{bundle, role, via}` plus the optional `lan` (default `false`) and
/// `accept_warning`; any other key, a wrong type or a bundle outside 1–1024 characters is `400`.
fn parse_pair(body: &[u8]) -> Result<PairPick, Problem> {
    let shape =
        "Body must be {\"bundle\": \"111PAIR:…\", \"role\": \"peer\"|\"leaf\"|\"gateway\", \
         \"via\": \"tcp\"|\"carrier\", \"lan\"?: bool, \"accept_warning\"?: <64 hex>}.";
    let obj = match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(o)) => o,
        _ => return Err(bad_request(shape)),
    };
    let (mut bundle, mut role, mut via, mut lan, mut accept) = (None, None, None, false, None);
    for (k, v) in obj {
        match (k.as_str(), v) {
            ("bundle", Value::String(s)) if !s.is_empty() && s.chars().count() <= BUNDLE_MAX => {
                bundle = Some(s)
            }
            ("role", v) => role = Some(role_of(&v).ok_or_else(|| bad_request(shape))?),
            ("via", v) => via = Some(via_of(&v).ok_or_else(|| bad_request(shape))?),
            ("lan", Value::Bool(b)) => lan = b,
            ("accept_warning", Value::String(s)) if sha256_hex(&s) => accept = Some(s),
            _ => return Err(bad_request(shape)),
        }
    }
    match (bundle, role, via) {
        (Some(bundle), Some(role), Some(via)) => Ok(PairPick {
            bundle,
            role,
            via,
            lan,
            accept_warning: accept,
        }),
        _ => Err(bad_request(shape)),
    }
}

fn attempts(api: &ApiState) -> Arc<PairAttempts> {
    api.ext_or_insert_with(|| Arc::new(PairAttempts::default()))
}

async fn offer(_lo: LoopbackOnly, State(api): State<Arc<ApiState>>, body: Bytes) -> Response {
    let (role, via) = match parse_offer(&body) {
        Ok(x) => x,
        Err(p) => return p.into_response(),
    };
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    match pair::offer(&h, &api.layout.root, role, via).await {
        Ok(v) => {
            let mut resp = Json(v).into_response();
            resp.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            resp
        }
        Err(p) => p.into_response(),
    }
}

async fn pair_hub(_lo: LoopbackOnly, State(api): State<Arc<ApiState>>, body: Bytes) -> Response {
    let pick = match parse_pair(&body) {
        Ok(x) => x,
        Err(p) => return p.into_response(),
    };
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    match pair::pair(&h, &attempts(&api), &api.layout.root, pick).await {
        Ok(v) => (StatusCode::ACCEPTED, Json(v)).into_response(),
        Err(r) => r.into_response(),
    }
}

async fn unpair(
    _lo: LoopbackOnly,
    State(api): State<Arc<ApiState>>,
    Path(node_id): Path<String>,
) -> Response {
    let Some(far) = json::parse_node_id(&node_id) else {
        return bad_request("node_id must be 64 lowercase hex characters.").into_response();
    };
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    match pair::unpair(&h, &attempts(&api), far).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(p) => p.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offer_body() {
        assert_eq!(
            parse_offer(br#"{"role":"leaf","via":"tcp"}"#).unwrap(),
            (PairRole::Leaf, PairVia::Tcp)
        );
        for bad in [
            &br#"{}"#[..],
            br#"{"role":"peer"}"#,
            br#"{"via":"tcp"}"#,
            br#"{"role":"PEER","via":"tcp"}"#,
            br#"{"role":"peer","via":"udp"}"#,
            br#"{"role":"peer","via":"tcp","address":"1.2.3.4:5"}"#,
            br#"[]"#,
            b"",
        ] {
            assert!(
                parse_offer(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn pair_body() {
        let p = parse_pair(br#"{"bundle":"111PAIR:x","role":"peer","via":"carrier"}"#).unwrap();
        assert_eq!(p.bundle, "111PAIR:x");
        assert_eq!(
            (p.role, p.via, p.lan),
            (PairRole::Peer, PairVia::Carrier, false)
        );
        assert_eq!(p.accept_warning, None);
        let h = "ab".repeat(32);
        let p = parse_pair(
            format!(
                r#"{{"bundle":"b","role":"gateway","via":"tcp","lan":true,"accept_warning":"{h}"}}"#
            )
            .as_bytes(),
        )
        .unwrap();
        assert!(p.lan);
        assert_eq!(p.accept_warning.as_deref(), Some(h.as_str()));
        let long = format!(
            r#"{{"bundle":"{}","role":"peer","via":"tcp"}}"#,
            "a".repeat(1025)
        );
        let upper = format!(
            r#"{{"bundle":"b","role":"peer","via":"tcp","accept_warning":"{}"}}"#,
            "AB".repeat(32)
        );
        for bad in [
            br#"{"role":"peer","via":"tcp"}"#.to_vec(),
            br#"{"bundle":"","role":"peer","via":"tcp"}"#.to_vec(),
            br#"{"bundle":"b","via":"tcp"}"#.to_vec(),
            br#"{"bundle":"b","role":"peer"}"#.to_vec(),
            br#"{"bundle":"b","role":"peer","via":"tcp","lan":"yes"}"#.to_vec(),
            br#"{"bundle":"b","role":"peer","via":"tcp","accept_warning":"x"}"#.to_vec(),
            br#"{"bundle":"b","role":"peer","via":"tcp","code":"BCDF2345"}"#.to_vec(),
            br#"{"bundle":5,"role":"peer","via":"tcp"}"#.to_vec(),
            long.into_bytes(),
            upper.into_bytes(),
            b"[]".to_vec(),
        ] {
            assert!(
                parse_pair(&bad).is_err(),
                "{}",
                String::from_utf8_lossy(&bad)
            );
        }
    }
}
