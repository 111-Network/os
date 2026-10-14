//! `GET /api/v1/panels` · `POST /api/v1/panels/{id}/session` (Layer 5 §13,
//! §15; contract `listPanels` / `openPanelSession`). Both sit behind
//! [`auth::require_session`]; the POST is also under the CSRF guard
//! (`Content-Type: application/json`, Origin = Host).
//!
//! - `GET /panels` lists the registered panels with their slots and state
//!   (`{schema: 1, panels: [Panel]}`; empty when no panel host runs).
//! - `POST /panels/{id}/session` mints a one-time ticket URL on the panel's
//!   own origin: `http://<the dashboard host as the browser typed it>:<slot>
//!   <entry.path>?p111_ticket=<128-bit hex>` — single use, 30 s, bound to the
//!   minting session (trusted loopback ⇒ the implicit session; otherwise the
//!   `s111` session; a bearer-only caller gets `401`) and the peer. The URL is
//!   only ever in this reply (`Cache-Control: no-store`). `404 not_found` for
//!   an unknown panel or one whose service is not set up; `403 loopback_only`
//!   for a loopback-only panel asked for from a non-loopback dashboard host.

use super::{auth, ApiRouter, ApiState, Problem};
use crate::daemon::dashboard::routes;
use crate::daemon::panels::ticket::{self, PeerClass};
use crate::daemon::panels::{guard, MintError, PanelHost};
use crate::daemon::services::ServiceId;
use axum::extract::{Path, State};
use axum::http::{header, Extensions, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use serde_json::json;
use std::sync::Arc;

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/panels", get(list))
        .route("/api/v1/panels/{id}/session", post(open_session))
}

async fn list(State(api): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    let panels = api
        .ext::<Arc<PanelHost>>()
        .map(|h| h.panels())
        .unwrap_or_default();
    Json(json!({ "schema": 1, "panels": panels }))
}

fn not_found() -> Response {
    Problem::new(StatusCode::NOT_FOUND, "not_found", "no such panel")
        .detail("The panel is not declared, or its service is not set up.")
        .into_response()
}

async fn open_session(
    State(api): State<Arc<ApiState>>,
    Path(id): Path<String>,
    ext: Extensions,
    headers: HeaderMap,
) -> Response {
    let Some(id) = ServiceId::parse(&id) else {
        return not_found();
    };
    let Some(host) = api.ext::<Arc<PanelHost>>() else {
        return not_found();
    };
    let Some(peer) = auth::peer_addr(&ext).map(|a| a.ip()) else {
        return Problem::new(StatusCode::FORBIDDEN, "bad_request", "unknown peer").into_response();
    };
    let Some(binding) = ticket::mint_binding(&api, &ext, &headers) else {
        return Problem::new(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "a dashboard session is required",
        )
        .detail("Panels open from a paired dashboard (or the node's own trusted browser).")
        .into_response();
    };
    // the ticket URL is built on the host the browser typed (never a snapshot)
    let typed = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .and_then(routes::parse_host);
    let own = api.own_addrs();
    let any_panel_host = guard::HostPolicy {
        lan: api.lan_enabled,
        own: &own,
        loopback_only: false,
    };
    let Some((dash_host, _)) = typed.filter(|(h, _)| guard::host_ok(h, &any_panel_host)) else {
        return Problem::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "the dashboard host cannot address a panel",
        )
        .into_response();
    };
    match host.mint(id, &dash_host, PeerClass::of(peer), binding) {
        Ok((panel, t)) => {
            let url = format!(
                "http://{}:{}{}?{}={t}",
                guard::url_host(&dash_host),
                panel.port,
                panel.manifest.entry,
                ticket::TICKET_PARAM
            );
            let mut resp = Json(json!({
                "schema": 1,
                "url": url,
                "expires_in": host.tickets.ttl().as_secs().max(1),
            }))
            .into_response();
            resp.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            resp
        }
        Err(MintError::NotFound) => not_found(),
        Err(MintError::LoopbackOnly) => Problem::new(
            StatusCode::FORBIDDEN,
            "loopback_only",
            "available on the node itself",
        )
        .detail("This panel opens only from a browser on the node (loopback).")
        .into_response(),
        Err(MintError::Random(e)) => {
            eprintln!("111 daemon: panel ticket: {e}");
            Problem::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "cannot mint a panel ticket",
            )
            .into_response()
        }
    }
}
