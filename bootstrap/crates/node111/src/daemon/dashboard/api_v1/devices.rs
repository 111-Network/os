//! GET /api/v1/devices · GET/PATCH/DELETE /api/v1/devices/{id} (device registry,
//! spec §6.3, §7.1; task R4). The registry itself lives in `daemon::devices`
//! (detection, store, 5 s tick); these handlers only read its published
//! snapshot and forward the two user mutations:
//!
//! - `PATCH {alias}` — trimmed; `null`/empty clears; > 64 chars or control
//!   characters → `400 bad_request`; a body without `alias` changes nothing.
//! - `DELETE` — forget (`204`): hides the device, drops its alias; a companion's
//!   dashboard session is revoked — also when it is the session making this
//!   request (the reply is still `204`; that browser's next call gets `401`).
//!   Detected hardware reappears once it has been unplugged and plugged in again.
//!
//! `{id}` must match `^[A-Za-z0-9_-]{4,64}$` (else `400`); an unknown or hidden
//! id is `404 not_found`. Both mutations publish a `devices` frame at once.

use super::{ApiRouter, ApiState, Problem};
use crate::daemon::devices::{self, DeviceRegistry, MAX_ALIAS};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;

fn registry(api: &ApiState) -> Result<Arc<DeviceRegistry>, Problem> {
    api.ext::<Arc<DeviceRegistry>>().ok_or_else(|| {
        Problem::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "internal",
            "device registry not running",
        )
    })
}

fn not_found() -> Problem {
    Problem::new(StatusCode::NOT_FOUND, "not_found", "no such device")
}

fn bad_request(title: &str) -> Problem {
    Problem::new(StatusCode::BAD_REQUEST, "bad_request", title.to_string())
}

fn check_id(id: &str) -> Result<(), Problem> {
    if devices::valid_id(id) {
        Ok(())
    } else {
        Err(bad_request("invalid device id"))
    }
}

async fn list(State(api): State<Arc<ApiState>>) -> Response {
    match registry(&api) {
        Ok(reg) => Json(json!({
            "schema": 1,
            "devices": reg.list().unwrap_or_default(),
        }))
        .into_response(),
        Err(p) => p.into_response(),
    }
}

async fn get_one(State(api): State<Arc<ApiState>>, Path(id): Path<String>) -> Response {
    let run = || -> Result<Response, Problem> {
        check_id(&id)?;
        let rec = registry(&api)?.get(&id).ok_or_else(not_found)?;
        Ok(Json(rec).into_response())
    };
    run().unwrap_or_else(IntoResponse::into_response)
}

/// Validate the PATCH body: `Ok(None)` = no `alias` key (no change),
/// `Ok(Some(None))` = clear, `Ok(Some(Some(a)))` = set.
fn parse_alias(body: &[u8]) -> Result<Option<Option<String>>, Problem> {
    let v: Value = serde_json::from_slice(body)
        .map_err(|_| bad_request("body must be a JSON object like {\"alias\": \"…\"}"))?;
    let obj = v
        .as_object()
        .ok_or_else(|| bad_request("body must be a JSON object like {\"alias\": \"…\"}"))?;
    match obj.get("alias") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.chars().any(char::is_control) {
                return Err(bad_request("alias must not contain control characters"));
            }
            if t.chars().count() > MAX_ALIAS {
                return Err(bad_request("alias is longer than 64 characters"));
            }
            Ok(Some((!t.is_empty()).then(|| t.to_string())))
        }
        Some(_) => Err(bad_request("alias must be a string or null")),
    }
}

async fn patch(
    State(api): State<Arc<ApiState>>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let run = || -> Result<Response, Problem> {
        check_id(&id)?;
        let alias = parse_alias(&body)?;
        let reg = registry(&api)?;
        let rec = match alias {
            None => reg.get(&id).ok_or_else(not_found)?,
            Some(a) => reg.set_alias(&api, &id, a).map_err(|_| not_found())?,
        };
        Ok(Json(rec).into_response())
    };
    run().unwrap_or_else(IntoResponse::into_response)
}

async fn forget(State(api): State<Arc<ApiState>>, Path(id): Path<String>) -> Response {
    let run = || -> Result<Response, Problem> {
        check_id(&id)?;
        registry(&api)?.forget(&api, &id).map_err(|_| not_found())?;
        Ok(StatusCode::NO_CONTENT.into_response())
    };
    run().unwrap_or_else(IntoResponse::into_response)
}

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/devices", get(list))
        .route(
            "/api/v1/devices/{id}",
            get(get_one).patch(patch).delete(forget),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_body_rules() {
        assert_eq!(parse_alias(br#"{}"#).unwrap(), None);
        assert_eq!(parse_alias(br#"{"alias":null}"#).unwrap(), Some(None));
        assert_eq!(parse_alias(br#"{"alias":"   "}"#).unwrap(), Some(None));
        assert_eq!(
            parse_alias(br#"{"alias":"  Roof radio "}"#).unwrap(),
            Some(Some("Roof radio".into()))
        );
        let max = format!(r#"{{"alias":"{}"}}"#, "é".repeat(64));
        assert!(parse_alias(max.as_bytes()).is_ok());
        let long = format!(r#"{{"alias":"{}"}}"#, "x".repeat(65));
        assert_eq!(parse_alias(long.as_bytes()).unwrap_err().status, 400);
        for bad in [
            &br#"{"alias":"a\nb"}"#[..],
            br#"{"alias":5}"#,
            br#"[1]"#,
            b"nope",
        ] {
            assert_eq!(parse_alias(bad).unwrap_err().code, "bad_request");
        }
    }
}
