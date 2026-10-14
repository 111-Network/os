//! `GET /api/v1/services` · `/services/{id}` · `/services/{id}/log?n=` — the
//! services registry (spec §7; logic in `crate::daemon::services`) — and
//! `POST /services/{id}/restart`. Owned by Layer 4 task R2.
//!
//! - The registry is started by the daemon hook (`services::probe::spawn`) and
//!   read from `ApiState` ext; without it (compat `serve()`, tools) the list
//!   routes answer `503`.
//! - Unknown ids are `404 not_found` (problem+json); the detail lists every
//!   id of [`ServiceId::ALL`] (nine since Layer 5: + `fabric`, `meshtastic`).
//! - Log tails: `n` clamps to 1..=500 (default 100; a non-integer is `400`).
//!   `agent` → `<root>/agent/111-agent.log`, `model` →
//!   `<root>/agent/llama-server.log`, `transport` → `<root>/rns/rnsd.log`,
//!   `meshtastic` → `<root>/meshtastic/meshmud.log` (Layer 5 §14).
//!   Services without a log file — or whose file does not exist yet — answer
//!   `source: "none"` with no lines. Lines are sanitized (control chars except
//!   `\t` stripped) by `services::log`. Log tails work without the registry.
//! - Restart: only the children this daemon supervises (`services::Supervised`
//!   in `ApiState` ext: agent, model, transport while rnsd is a child) →
//!   `202 {schema, id, restarting}` at once; the restart runs in the
//!   background (SIGTERM, [`RESTART_GRACE`], SIGKILL, immediate respawn with
//!   the back-off cleared — also out of `child_degraded`). A service with
//!   several children restarts them one after another in registration order
//!   (Layer 5 §14: meshmud, then the presence reader). Anything else →
//!   `409 not_supervised`. Same gate as every write (loopback or session,
//!   CSRF).

use super::{ApiRouter, ApiState, Problem};
use crate::daemon::intelligence;
use crate::daemon::services::{self, log, Registry, ServiceId, Supervised};
use crate::daemon::supervise::ChildState;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// SIGTERM → SIGKILL grace of a dashboard-requested child restart.
pub const RESTART_GRACE: Duration = crate::daemon::CHILD_STOP_GRACE;

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/services", get(list))
        .route("/api/v1/services/{id}", get(one))
        .route("/api/v1/services/{id}/log", get(log_tail))
        .route("/api/v1/services/{id}/restart", post(restart))
}

fn not_supervised(id: ServiceId) -> Response {
    let detail = match id {
        ServiceId::Agent | ServiceId::Model => {
            "It is not installed on this node, so there is nothing to restart.".to_string()
        }
        ServiceId::Transport => "Mesh transport runs as the detection-only stub here (no python3 \
            or Reticulum environment), so there is no rnsd process to restart."
            .to_string(),
        ServiceId::Meshtastic => "No Meshtastic board is bound to this node, so meshmud is not \
            running and there is nothing to restart."
            .to_string(),
        other => format!(
            "{} is not a process the node runs; use \"Restart node\" in Settings to restart \
             everything.",
            services::catalogue(other).plain_name
        ),
    };
    Problem::new(
        StatusCode::CONFLICT,
        "not_supervised",
        "This service is not run by the node itself.",
    )
    .detail(detail)
    .into_response()
}

async fn restart(State(api): State<Arc<ApiState>>, Path(id): Path<String>) -> Response {
    let Some(sid) = ServiceId::parse(&id) else {
        return unknown(&id);
    };
    let children = api
        .ext::<Arc<Supervised>>()
        .map(|s| s.children(sid).to_vec())
        .unwrap_or_default();
    // the primary child decides: stopped for good (shutdown) ⇒ not supervised
    if children
        .first()
        .is_none_or(|c| c.state() == ChildState::Stopped)
    {
        return not_supervised(sid);
    }
    let names: Vec<_> = children.iter().map(|c| c.name().to_string()).collect();
    eprintln!(
        "111 daemon: restart of {} ({}) requested from the dashboard",
        sid.as_str(),
        names.join(", ")
    );
    tokio::spawn(async move {
        for child in children {
            if child.state() != ChildState::Stopped {
                child.restart(RESTART_GRACE).await;
            }
        }
    });
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"schema": 1, "id": sid.as_str(), "restarting": true})),
    )
        .into_response()
}

fn registry_down() -> Response {
    Problem::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "internal",
        "services registry is not running",
    )
    .into_response()
}

fn unknown(id: &str) -> Response {
    let known: Vec<_> = ServiceId::ALL.iter().map(|i| i.as_str()).collect();
    Problem::new(StatusCode::NOT_FOUND, "not_found", "no such service")
        .detail(format!(
            "unknown service id {id:?} (one of: {})",
            known.join(", ")
        ))
        .into_response()
}

async fn list(State(api): State<Arc<ApiState>>) -> Response {
    match api.ext::<Arc<Registry>>() {
        Some(r) => Json(serde_json::json!({"schema": 1, "services": r.snapshot()})).into_response(),
        None => registry_down(),
    }
}

async fn one(State(api): State<Arc<ApiState>>, Path(id): Path<String>) -> Response {
    let Some(sid) = ServiceId::parse(&id) else {
        return unknown(&id);
    };
    let Some(r) = api.ext::<Arc<Registry>>() else {
        return registry_down();
    };
    match r.get(sid) {
        Some(rec) => Json(rec).into_response(),
        None => Problem::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "internal",
            "services registry has not finished its first check",
        )
        .into_response(),
    }
}

/// The log file a service writes, if it has one.
pub fn log_path(api: &ApiState, id: ServiceId) -> Option<PathBuf> {
    match id {
        ServiceId::Agent => Some(intelligence::paths::agent_log(&api.layout)),
        ServiceId::Model => Some(intelligence::paths::model_log(&api.layout)),
        ServiceId::Transport => Some(services::probe::rnsd_log(&api.layout)),
        ServiceId::Meshtastic => Some(services::probe::meshmud_log(&api.layout)),
        ServiceId::Radio
        | ServiceId::Kb
        | ServiceId::Gateway
        | ServiceId::Codeagent
        | ServiceId::Fabric => None,
    }
}

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    pub n: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LogTail {
    pub schema: u32,
    pub service: ServiceId,
    pub source: String,
    pub lines: Vec<String>,
}

async fn log_tail(
    State(api): State<Arc<ApiState>>,
    Path(id): Path<String>,
    query: Result<Query<LogQuery>, QueryRejection>,
) -> Response {
    let Some(sid) = ServiceId::parse(&id) else {
        return unknown(&id);
    };
    let n = match query.ok().and_then(|Query(q)| q.n) {
        None => log::clamp_lines(None),
        Some(raw) => match raw.trim().parse::<i64>() {
            Ok(v) => log::clamp_lines(Some(v)),
            Err(_) => {
                return Problem::new(StatusCode::BAD_REQUEST, "bad_request", "invalid n")
                    .detail("n must be an integer (1..=500)")
                    .into_response()
            }
        },
    };
    let mut body = LogTail {
        schema: 1,
        service: sid,
        source: "none".into(),
        lines: Vec::new(),
    };
    if let Some(path) = log_path(&api, sid) {
        let tail_path = path.clone();
        match tokio::task::spawn_blocking(move || log::tail(&tail_path, n)).await {
            Ok(Ok(Some(lines))) => {
                body.source = path.to_string_lossy().into_owned();
                body.lines = lines;
            }
            Ok(Ok(None)) => {}
            Ok(Err(e)) => {
                return Problem::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "cannot read the log file",
                )
                .detail(format!("{}: {e}", path.display()))
                .into_response()
            }
            Err(_) => {
                return Problem::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "log read failed",
                )
                .into_response()
            }
        }
    }
    Json(body).into_response()
}
