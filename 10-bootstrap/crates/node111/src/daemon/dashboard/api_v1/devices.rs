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
//!
//! Layer 5 §14 adds `POST /devices/{id}/identify` — the consented Meshtastic
//! step (`daemon::meshtastic::identify`). The body must be exactly
//! `{"consent": true}` (anything else — missing, `false`, another type —
//! `400 bad_request`; the check runs before anything is looked up or opened).
//! A device that does not offer `identify` is `400`; a port with another
//! owner `409 port_busy`; meshmud or the reader failing `502 upstream_error`;
//! success answers the updated `DeviceRecord` (`200`). Same gate as every
//! write (loopback or a paired session, CSRF/JSON rules). The step runs in
//! its own task, so a client that disconnects mid-probe never leaves a child
//! running or a port claimed.
//!
//! Layer 6 §12, §13 (D26 #2): the agent's bearer is refused here by the
//! global write gate (`auth::require_session`) — the consent flag in the body
//! is the human's, never the agent's. With the device core installed
//! (`devices::identify::Identify` in ext, `devices::start`), the route runs
//! its consented RNode → Meshtastic → MeshCore sequence
//! (`Identify::run(.., Consent::Given)`, the consent remembered per device);
//! without it (the Layer 5 harnesses) the Meshtastic step alone. The agent
//! files a `kind: device` proposal with the one-shot `identify {device_id}`;
//! accepting it **is** the consent and the device core's applier runs the
//! same sequence.

use super::{ApiRouter, ApiState, Problem};
use crate::daemon::devices::{self, DeviceRegistry, MAX_ALIAS};
use crate::daemon::meshtastic::identify::IdentifyError;
use crate::daemon::meshtastic::Meshtastic;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
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

/// The identify body: exactly `{"consent": true}` (other keys refused).
fn parse_consent(body: &[u8]) -> Result<(), Problem> {
    let need = || {
        bad_request("identify needs the user's consent").detail(
            "Send {\"consent\": true} once the user has agreed: opening the board's port can \
             reset it.",
        )
    };
    let v: Value = serde_json::from_slice(body).map_err(|_| need())?;
    let obj = v.as_object().ok_or_else(need)?;
    if obj.len() != 1 || obj.get("consent") != Some(&Value::Bool(true)) {
        return Err(need());
    }
    Ok(())
}

async fn identify(
    State(api): State<Arc<ApiState>>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    if let Err(p) = check_id(&id).and_then(|()| parse_consent(&body)) {
        return p.into_response();
    }
    match identify_device(&api, &id).await {
        Ok(rec) => Json(rec).into_response(),
        Err(p) => p.into_response(),
    }
}

/// The consented identify of device `id` (the caller holds the consent: the
/// route's `{"consent": true}`). Answers the updated `DeviceRecord` as JSON.
/// The steps run to completion in their own task: a client that goes away
/// mid-probe must not cancel them half-way (child running, port claimed).
pub async fn identify_device(api: &Arc<ApiState>, id: &str) -> Result<Value, Problem> {
    use crate::daemon::devices::identify::{Consent, Identify};
    check_id(id)?;
    registry(api)?;
    if let Some(ident) = api.ext::<Arc<Identify>>() {
        let (api2, id2) = (Arc::clone(api), id.to_string());
        let run = tokio::spawn(async move { ident.run(&api2, &id2, Consent::Given).await });
        return match run.await {
            Ok(Ok(report)) => Ok(serde_json::to_value(report.record).unwrap_or(Value::Null)),
            Ok(Err(e)) => Err(e.problem()),
            Err(_) => Err(IdentifyError::Upstream("the identify step failed".into()).problem()),
        };
    }
    let Some(m) = api.ext::<Arc<Meshtastic>>() else {
        return Err(IdentifyError::Unavailable.problem());
    };
    let id = id.to_string();
    let step = tokio::spawn(async move { m.identify(&id).await });
    match step.await {
        Ok(Ok(rec)) => Ok(serde_json::to_value(rec).unwrap_or(Value::Null)),
        Ok(Err(e)) => Err(e.problem()),
        Err(_) => Err(IdentifyError::Upstream("the identify step failed".into()).problem()),
    }
}

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/devices", get(list))
        .route(
            "/api/v1/devices/{id}",
            get(get_one).patch(patch).delete(forget),
        )
        .route("/api/v1/devices/{id}/identify", post(identify))
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

    #[test]
    fn identify_needs_exactly_consent_true() {
        assert!(parse_consent(br#"{"consent":true}"#).is_ok());
        for bad in [
            &b""[..],
            b"null",
            b"true",
            br#"{}"#,
            br#"{"consent":false}"#,
            br#"{"consent":"true"}"#,
            br#"{"consent":1}"#,
            br#"{"consent":true,"port":"/dev/sda"}"#,
            br#"[{"consent":true}]"#,
        ] {
            let p = parse_consent(bad).unwrap_err();
            assert_eq!((p.status, p.code), (400, "bad_request"), "{bad:?}");
        }
    }
}
