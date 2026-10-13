//! `POST /api/v1/node/restart` · `POST /api/v1/update/check` (spec §6.7, §10).
//! Owned by Layer 4 task R2. Both sit behind the session gate (loopback or
//! paired) and the CSRF guard (JSON content type, same-origin `Origin`); the
//! request body is ignored.
//!
//! - **restart**: replies `202 {schema:1, restarting:true}`, then after
//!   [`RESTART_DELAY`] notifies the daemon's [`RestartSignal`]; the daemon's
//!   signal `select!` treats it like SIGTERM (graceful stop, exit 0) and the
//!   supervisor (`Restart=always`) starts it again. Without a wired signal
//!   (compat `serve()`, tools) the route answers `503`.
//! - **update check**: wakes the updater loop through `ApiState::update_check`
//!   and marks a check as running so `/status.update.state` reads `checking`
//!   right away. A request while a check is already running joins it (no
//!   second check is queued). Replies `202 {schema:1, started:true}`;
//!   `started:false` only when no updater runs in this process.

use super::{status, ApiRouter, ApiState, Problem};
use crate::daemon::updater::UpdaterStatus;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

/// Gap between the `202` and the shutdown (lets the reply flush).
pub const RESTART_DELAY: Duration = Duration::from_millis(300);

/// The daemon's dashboard-triggered shutdown path (inserted into `ApiState`
/// ext by the daemon start hook; `notify_one` keeps a permit, so a request
/// before the daemon awaits it is not lost).
#[derive(Clone)]
pub struct RestartSignal(pub Arc<Notify>);

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/node/restart", post(restart))
        .route("/api/v1/update/check", post(update_check))
}

async fn restart(State(api): State<Arc<ApiState>>) -> Response {
    let Some(RestartSignal(signal)) = api.ext::<RestartSignal>() else {
        return Problem::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "internal",
            "restart is not available in this process",
        )
        .into_response();
    };
    eprintln!("111 daemon: restart requested from the dashboard");
    tokio::spawn(async move {
        tokio::time::sleep(RESTART_DELAY).await;
        signal.notify_one();
    });
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"schema": 1, "restarting": true})),
    )
        .into_response()
}

async fn update_check(State(api): State<Arc<ApiState>>) -> Response {
    let started = match api.ext::<Arc<UpdaterStatus>>() {
        Some(st) => {
            if !st.begin_check() {
                api.update_check.notify_one();
            }
            true
        }
        None => {
            api.update_check.notify_one();
            false
        }
    };
    status::publish_now(&api);
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"schema": 1, "started": started})),
    )
        .into_response()
}
