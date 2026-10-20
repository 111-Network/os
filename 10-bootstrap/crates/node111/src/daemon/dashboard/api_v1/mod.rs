//! `/api/v1` — the Layer 4 dashboard contract (`os/40-interface/api/openapi.yaml`,
//! conventions in `os/40-interface/api/README.md`).
//!
//! ## How the router is assembled (R2/R3: read this, then edit only your file)
//!
//! Every submodule exposes `pub fn router() -> ApiRouter` with the FULL paths it
//! serves (`/api/v1/...`, literal strings — grep-able, no `nest`). This file
//! merges them once and never needs editing again:
//!
//! - **gated** submodules (everything except `auth`) sit behind
//!   [`auth::require_session`]: loopback peers pass (unless the loopback-trust
//!   policy is off — Termux), others need a valid `s111` session cookie, else
//!   `401 unauthenticated`. Its first check is the global write gate (Layer 6
//!   §13, D26 #2): the agent's bearer is refused on every non-`GET`, no
//!   allowlist; the open `auth` writes carry the same gate
//!   ([`auth::bearer_write_gate`]).
//! - Layer 6 state in [`ApiState::ext`] (inserted by `daemon/mod.rs`):
//!   `Arc<crate::daemon::bridge::Bridge>` (networks, fan-out, edges,
//!   settings), `crate::daemon::components::Components` (optional
//!   components), `Arc<transport::TransportApi>` (the spine and the rnstatus
//!   facts cache) and `Arc<bridge::ComponentAdapters>` (the component →
//!   adapter seam).
//! - `auth` routes (`/api/v1/auth/*`) are open (that is how a LAN visitor pairs).
//! - [`auth::csrf_guard`] wraps BOTH groups: every POST/PATCH/PUT/DELETE must be
//!   `Content-Type: application/json` (else 415) and, when `Origin` is present,
//!   its authority must equal `Host` (else 403). It applies to loopback peers too
//!   — the loopback browser is exactly who CSRF targets.
//! - Errors: handlers answer [`Problem`]s; anything the framework produces
//!   instead (extractor rejections, 413, 405, unknown routes) is rewritten to
//!   problem+json with the same status by `routes::api_problems`, which wraps
//!   every `/api/*` path of the app.
//!
//! Every contract route is built (R1–R4). A future route still follows the same
//! pattern: register the path in its submodule's `router()`, merge it below once.
//!
//! ## Shared state
//!
//! [`ApiState`] carries what every route needs (dashboard state, layout, auth,
//! events bus, bind info). A submodule that needs its own long-lived state (R2's
//! services registry, R3's relay client) inserts an `Arc<T>` into
//! [`ApiState::ext`] at daemon start (from the `daemon/mod.rs` hook, after
//! `dashboard::start` returns the `Arc<ApiState>`) and reads it back with
//! `api.ext::<T>()` — no field additions here.
//!
//! Loopback-only operations (R2: `settings.lan_enabled`, `settings.offer`) use
//! [`auth::LoopbackOnly`] (whole route) or [`auth::peer_is_loopback`] (per field)
//! and answer `403 loopback_only`.

pub mod actions;
pub mod auth;
pub mod bridge;
pub mod contacts;
pub mod devices;
pub mod events;
pub mod fabric;
pub mod fanout;
pub mod kb;
pub mod messages;
pub mod nearby;
pub mod networks;
pub mod panels;
pub mod proposals;
pub mod relay;
pub mod services;
pub mod settings;
pub mod status;
pub mod transport;

use super::bind::BindInfo;
use super::SharedState;
use crate::daemon::DaemonLayout;
use axum::http::{header, StatusCode};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Router type every submodule returns (state applied once in [`router`]).
pub type ApiRouter = axum::Router<Arc<ApiState>>;

/// SSE keep-alive comment interval on `/api/v1/events` (contract: 15 s).
pub const KEEPALIVE: Duration = Duration::from_secs(15);

/// Construction options for [`ApiState`].
#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub layout: DaemonLayout,
    /// LAN sharing effective for this daemon lifetime (`state/settings.json`
    /// `lan_enabled`, unless `DASHBOARD_LAN` in the env file overrides it —
    /// `bind::lan_config`; read once at start).
    pub lan_enabled: bool,
    /// Persist pairing state under `<root>/state/` (daemon: true). `false` keeps
    /// the pair code + sessions in memory only (the compat `serve()` entry).
    pub persist_auth: bool,
    /// Serve the UI from this directory instead of the embedded `40-interface/ui/dist`
    /// (tests use a fixture dist; production leaves it `None`).
    pub ui_dir: Option<PathBuf>,
    /// `/api/v1/events` keep-alive interval (tests shorten it).
    pub keepalive: Duration,
    /// Loopback peers bypass pairing ([`auth::loopback_trusted`]: false under
    /// Termux unless `DASHBOARD_TRUST_LOOPBACK=true`). Computed once here.
    pub trust_loopback: bool,
}

impl ApiConfig {
    pub fn new(layout: DaemonLayout, lan_enabled: bool) -> Self {
        ApiConfig {
            layout,
            lan_enabled,
            persist_auth: true,
            ui_dir: None,
            keepalive: KEEPALIVE,
            trust_loopback: auth::loopback_trusted(),
        }
    }
}

/// Everything `/api/v1` handlers share. One per daemon lifetime.
pub struct ApiState {
    /// The Layer 1 dashboard state (status facts the daemon core mirrors in).
    pub dash: SharedState,
    pub layout: DaemonLayout,
    pub auth: auth::AuthStore,
    pub events: events::EventBus,
    /// URLs that actually bound + whether the listeners are LAN-wide.
    pub bind: RwLock<BindInfo>,
    /// LAN sharing effective this lifetime (see [`ApiConfig::lan_enabled`]).
    pub lan_enabled: bool,
    /// The node's own interface addresses (Host allowlist when LAN is on);
    /// refreshed every 30 s by the daemon.
    own_addrs: RwLock<Vec<IpAddr>>,
    pub ui_dir: Option<PathBuf>,
    pub keepalive: Duration,
    /// Wakes the updater loop (`POST /api/v1/update/check`, R2 wires the loop).
    pub update_check: Arc<tokio::sync::Notify>,
    /// Typed extension slots for submodule state (see module docs).
    ext: RwLock<axum::http::Extensions>,
}

impl ApiState {
    pub fn new(dash: SharedState, cfg: ApiConfig) -> Arc<Self> {
        let auth = if cfg.persist_auth {
            auth::AuthStore::open(Some(cfg.layout.state_dir()))
        } else {
            auth::AuthStore::open(None)
        }
        .with_loopback_trust(cfg.trust_loopback);
        Arc::new(ApiState {
            dash,
            auth,
            events: events::EventBus::new(),
            bind: RwLock::new(BindInfo {
                urls: Vec::new(),
                lan: cfg.lan_enabled,
            }),
            lan_enabled: cfg.lan_enabled,
            own_addrs: RwLock::new(Vec::new()),
            ui_dir: cfg.ui_dir,
            keepalive: cfg.keepalive,
            update_check: Arc::new(tokio::sync::Notify::new()),
            ext: RwLock::new(axum::http::Extensions::new()),
            layout: cfg.layout,
        })
    }

    /// Replace the own-address set (daemon refresh task; tests).
    pub fn set_own_addrs(&self, addrs: Vec<IpAddr>) {
        *self.own_addrs.write().unwrap() = addrs;
    }

    pub fn own_addrs(&self) -> Vec<IpAddr> {
        self.own_addrs.read().unwrap().clone()
    }

    pub fn is_own_addr(&self, ip: &IpAddr) -> bool {
        self.own_addrs.read().unwrap().contains(ip)
    }

    /// Insert a submodule's shared state (once, at daemon start).
    pub fn insert_ext<T: Clone + Send + Sync + 'static>(&self, value: T) {
        self.ext.write().unwrap().insert(value);
    }

    /// Read a submodule's shared state back (typically an `Arc<T>`).
    pub fn ext<T: Clone + Send + Sync + 'static>(&self) -> Option<T> {
        self.ext.read().unwrap().get::<T>().cloned()
    }
}

/// RFC 9457 `application/problem+json` envelope (`Problem` schema; `code` is the
/// machine key from the contract enum).
#[derive(Debug, Clone, Serialize)]
pub struct Problem {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub title: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub code: &'static str,
}

impl Problem {
    pub fn new(status: StatusCode, code: &'static str, title: impl Into<String>) -> Self {
        Problem {
            kind: "about:blank",
            title: title.into(),
            status: status.as_u16(),
            detail: None,
            code,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::to_string(&self).unwrap_or_else(|_| "{}".into());
        (
            status,
            [(header::CONTENT_TYPE, "application/problem+json")],
            body,
        )
            .into_response()
    }
}

/// Placeholder handler for contract routes whose owning task has not landed.
pub async fn not_built() -> Problem {
    Problem::new(StatusCode::NOT_IMPLEMENTED, "internal", "not built yet")
}

/// The whole `/api/v1` surface with state applied (merged by `routes::app`).
pub fn router(state: Arc<ApiState>) -> axum::Router {
    let gated: ApiRouter = axum::Router::new()
        .merge(events::router())
        .merge(status::router())
        .merge(services::router())
        .merge(devices::router())
        .merge(settings::router())
        .merge(actions::router())
        .merge(kb::router())
        .merge(relay::router())
        .merge(proposals::router())
        .merge(nearby::router())
        .merge(panels::router())
        .merge(messages::router())
        .merge(contacts::router())
        .merge(fabric::router())
        // Layer 6 (§10): networks, fan-out, bridge, transport
        .merge(networks::router())
        .merge(fanout::router())
        .merge(bridge::router())
        .merge(transport::router())
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            auth::require_session,
        ));
    axum::Router::new()
        .merge(gated)
        .merge(auth::router())
        .route_layer(middleware::from_fn(auth::csrf_guard))
        .with_state(state)
}
