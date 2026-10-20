//! HTTP surface: routes + Host allowlist (loopback trust model, mirroring
//! the bootstrap dashboard contract). Full contract in 30-intelligence/TASKS.md.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::KeepAlive;
use axum::response::{IntoResponse, Response, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::config::Config;
use crate::events::{self, Journal};
use crate::kb::Kb;
use crate::model::Client;
use crate::store::Store;
use crate::tasks::{self, TaskEngine};
use crate::{chat, health, status};

/// Comment-frame cadence that keeps idle SSE connections (and any proxy in
/// front of them) from timing out.
const SSE_KEEP_ALIVE: std::time::Duration = std::time::Duration::from_secs(15);

pub struct AppState {
    pub cfg: Arc<Config>,
    pub model: Arc<Client>,
    pub kb: Arc<Kb>,
    pub store: Arc<Store>,
    pub journal: Journal,
    pub tasks: Arc<TaskEngine>,
    pub started: std::time::Instant,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/v1/agent/status", get(status_handler))
        .route("/v1/agent/events", get(events_handler))
        .route("/v1/chat", post(chat::handler))
        .route("/v1/chat/history", get(chat::history_handler))
        .route("/v1/agent/task", post(task_handler))
        .layer(middleware::from_fn(host_guard))
        .with_state(state)
}

/// Host allowlist: 127.0.0.1 / localhost / [::1], any port. Missing Host
/// (HTTP/1.0-style probes) is tolerated; a foreign Host is refused.
async fn host_guard(req: Request, next: Next) -> Response {
    if let Some(host) = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
    {
        if !is_allowed_host(host) {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"schema": 1, "error": "host not allowed"})),
            )
                .into_response();
        }
    }
    next.run(req).await
}

fn is_allowed_host(host_header: &str) -> bool {
    matches!(
        host_name(host_header).as_str(),
        "127.0.0.1" | "localhost" | "[::1]"
    )
}

fn host_name(host_header: &str) -> String {
    if host_header.starts_with('[') {
        if let Some(end) = host_header.find(']') {
            return host_header[..=end].to_string();
        }
    }
    host_header
        .split(':')
        .next()
        .unwrap_or(host_header)
        .to_string()
}

async fn health_handler(State(s): State<Arc<AppState>>) -> Response {
    let model = s.model.clone();
    let reachable = tokio::task::spawn_blocking(move || model.reachable())
        .await
        .unwrap_or(false);
    let components = health::Components {
        model: reachable,
        kb: s.kb.is_present(),
        state_dir: state_dir_writable(&s),
    };
    let (code, body) = health::evaluate(&components);
    (code, Json(body)).into_response()
}

fn state_dir_writable(s: &AppState) -> bool {
    let probe = s.store.dir().join(".health-probe");
    std::fs::write(&probe, b"ok")
        .and_then(|_| std::fs::remove_file(&probe))
        .is_ok()
}

async fn status_handler(State(s): State<Arc<AppState>>) -> Response {
    let model = s.model.clone();
    let reachable = tokio::task::spawn_blocking(move || model.reachable())
        .await
        .unwrap_or(false);
    let body = status::StatusBody {
        schema: 1,
        state: status::derive_state(s.model.base(), reachable).as_str(),
        model: status::ModelBody {
            base: s.model.base().to_string(),
            reachable,
            id: s.model.model_id().to_string(),
        },
        kb: status::KbBody {
            present: s.kb.is_present(),
            entries: s.kb.entries(),
        },
        conversations: s.store.conversation_count(),
        tasks_open: s.tasks.open_count(),
        uptime_sec: s.started.elapsed().as_secs(),
        version: env!("CARGO_PKG_VERSION"),
        node_id: s.cfg.node_id.clone(),
        category: s.cfg.category.clone(),
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// SSE: replay the last 50 journal events, then live. Replay and
/// subscription are taken atomically, so nothing is lost or duplicated at the
/// seam. Lagging subscribers drop events rather than block the journal.
async fn events_handler(State(s): State<Arc<AppState>>) -> Response {
    use tokio_stream::StreamExt;
    type SseItem = Result<axum::response::sse::Event, std::convert::Infallible>;
    let (backlog, rx) = s.journal.replay_and_subscribe(50);
    let replay = tokio_stream::iter(backlog).map(|ev| -> SseItem { Ok(to_sse(ev)) });
    let live = tokio_stream::wrappers::BroadcastStream::new(rx)
        .filter_map(|r| r.ok())
        .map(|ev| -> SseItem { Ok(to_sse(ev)) });
    Sse::new(replay.chain(live))
        .keep_alive(KeepAlive::new().interval(SSE_KEEP_ALIVE).text("keep-alive"))
        .into_response()
}

fn to_sse(ev: events::Event) -> axum::response::sse::Event {
    axum::response::sse::Event::default()
        .json_data(ev)
        .unwrap_or_else(|_| axum::response::sse::Event::default().data("{}"))
}

#[derive(Debug, Deserialize)]
pub struct TaskRequest {
    pub kind: tasks::TaskKind,
    #[serde(default)]
    pub refs: Vec<String>,
    pub note: Option<String>,
}

async fn task_handler(State(s): State<Arc<AppState>>, Json(req): Json<TaskRequest>) -> Response {
    match s.tasks.submit(req.kind, req.refs, req.note) {
        Ok(task_id) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({"schema": 1, "task_id": task_id})),
        )
            .into_response(),
        Err(tasks::SubmitError::NeedsHub) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"schema": 1, "error": "needs hub"})),
        )
            .into_response(),
        Err(tasks::SubmitError::Other(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"schema": 1, "error": e.to_string()})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_allowlist() {
        assert!(is_allowed_host("127.0.0.1:11182"));
        assert!(is_allowed_host("localhost"));
        assert!(is_allowed_host("[::1]:11182"));
        assert!(!is_allowed_host("evil.example.com"));
        assert!(!is_allowed_host("127.0.0.1.evil.com"));
    }
}
