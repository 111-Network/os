//! Routes + handlers. Contracts (pinned in os/bootstrap/TASKS.md):
//! - GET-only (405 on anything else), Host-header allowlist
//!   127.0.0.1/localhost (any port) against DNS rebinding
//! - GET /health  → 200 iff all components green, always the component JSON
//! - GET /api/status → the Layer 4/9 seam schema (additive-only)
//! - no identity material on any endpoint, ever

use super::SharedState;
use crate::daemon::health::{evaluate, HealthStatus, TransportHealth};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use serde::Serialize;

#[derive(rust_embed::RustEmbed)]
#[folder = "assets/"]
struct DashboardAssets;

/// Host allowlist: 127.0.0.1 / localhost (any port). Missing Host (HTTP/1.0
/// health probes) is allowed; anything else is refused.
async fn host_guard(headers: HeaderMap, req: Request, next: Next) -> Response {
    let ok = match headers.get(axum::http::header::HOST) {
        None => true,
        Some(v) => {
            let host = v.to_str().unwrap_or("");
            let bare = host.split(':').next().unwrap_or("");
            bare == "127.0.0.1" || bare == "localhost"
        }
    };
    if ok {
        next.run(req).await
    } else {
        (StatusCode::BAD_REQUEST, "refused Host header").into_response()
    }
}

pub fn router(state: SharedState) -> axum::Router {
    axum::Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/api/status", get(api_status))
        .layer(middleware::from_fn(host_guard))
        .with_state(state)
}

async fn index() -> impl IntoResponse {
    match DashboardAssets::get("dashboard.html") {
        Some(page) => {
            Html(String::from_utf8_lossy(page.data.as_ref()).into_owned()).into_response()
        }
        None => (StatusCode::INTERNAL_SERVER_ERROR, "dashboard asset missing").into_response(),
    }
}

#[derive(Serialize)]
struct HealthBody {
    schema: u32,
    status: &'static str,
    components: HealthComponents,
}

#[derive(Serialize)]
struct HealthComponents {
    identity: bool,
    kb: bool,
    transport: &'static str,
    updater_idle: bool,
}

fn transport_label(t: TransportHealth) -> &'static str {
    match t {
        TransportHealth::Alive => "alive",
        TransportHealth::DeliberatelyAbsent => "absent",
        TransportHealth::Degraded => "degraded",
        TransportHealth::Down => "down",
    }
}

async fn health(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().unwrap();
    let status = evaluate(&s.components);
    let body = HealthBody {
        schema: 1,
        status: match status {
            HealthStatus::Ok => "ok",
            HealthStatus::Error => "error",
        },
        components: HealthComponents {
            identity: s.components.identity,
            kb: s.components.kb,
            transport: transport_label(s.components.transport),
            updater_idle: s.components.updater_idle,
        },
    };
    let code = match status {
        HealthStatus::Ok => StatusCode::OK,
        HealthStatus::Error => StatusCode::SERVICE_UNAVAILABLE,
    };
    (code, Json(body))
}

#[derive(Serialize)]
struct StatusBody {
    schema: u32,
    node: NodeBody,
    kb: Option<KbBody>,
    transport_state: super::TransportState,
    health: HealthBody,
    updates: UpdatesBody,
    clock_fault: bool,
    offer: Option<super::OfferState>,
    error_state: Option<super::ErrorState>,
}

#[derive(Serialize)]
struct NodeBody {
    npub: Option<String>,
    category: Option<String>,
    version: String,
    uptime_sec: u64,
}

#[derive(Serialize)]
struct KbBody {
    sha256: String,
    entries: u32,
}

#[derive(Serialize)]
struct UpdatesBody {
    last_successful_check: Option<String>,
    pending: bool,
    floor: Option<String>,
}

async fn api_status(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().unwrap();
    let status = evaluate(&s.components);
    let body = StatusBody {
        schema: 1,
        node: NodeBody {
            npub: s.npub.clone(),
            category: s.category.clone(),
            version: s.version.clone(),
            uptime_sec: s.uptime_sec(),
        },
        kb: s.kb.as_ref().map(|k| KbBody {
            sha256: k.sha256.clone(),
            entries: k.entries,
        }),
        transport_state: s.transport_state,
        health: HealthBody {
            schema: 1,
            status: match status {
                HealthStatus::Ok => "ok",
                HealthStatus::Error => "error",
            },
            components: HealthComponents {
                identity: s.components.identity,
                kb: s.components.kb,
                transport: transport_label(s.components.transport),
                updater_idle: s.components.updater_idle,
            },
        },
        updates: UpdatesBody {
            last_successful_check: s.last_successful_update_check.clone(),
            pending: s.update_pending,
            floor: s.floor_release.clone(),
        },
        clock_fault: s.clock_fault,
        offer: s.offer.clone(),
        error_state: s.error_state.clone(),
    };
    Json(body)
}
