//! Layer 7 (§13.1, §9.3, §9.5 T4; row L4): `GET /api/v1/update/origins`.
//!
//! The origin order `[home hub, install.111.network, GitHub Releases]`, then each directory or
//! URL typed for `111 update --from`, with each origin's last answer (`ok`, its age on the
//! monotonic clock, a detail) and the newest release it served that this node verified (T4:
//! "newest release seen: <calver> from <origin>"). The home hub is listed only while its
//! announce carries `ROLE_ORIGIN` (one `contact` read of the fabric actor); the internet bases
//! are listed with `updates.check_internet` off too (shown, never contacted). A GET reads: no
//! request to any origin, no task, no file write. The `/origin/v1/*` files are outside
//! `/api/v1` (row D7, `dashboard/routes.rs`). The session gate covers the route as every
//! `/api/v1` read (the agent's bearer reads it; no secret is in it: a `--from` URL with a user
//! or password is refused by the verb).

use super::{ApiRouter, ApiState};
use crate::daemon::fabric::FabricHandle;
use crate::daemon::updater::fetch::{visible_origins, BootClock, UpdaterConfig};
use crate::daemon::updater::hub;
use axum::extract::State;
use axum::routing::get;
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;

/// The envelope's `schema`.
pub const SCHEMA: u32 = 1;

/// The bases the daemon's updater uses (`ApiState` ext `Arc<UpdaterConfig>`, inserted at daemon
/// start), else the production ones.
fn config(api: &ApiState) -> UpdaterConfig {
    api.ext::<Arc<UpdaterConfig>>()
        .map(|c| (*c).clone())
        .unwrap_or_else(|| {
            // only the bases are read here; the running release plays no part
            UpdaterConfig::production(crate::calver::Calver {
                year: 0,
                month: 0,
                day: 0,
                respin: None,
            })
        })
}

async fn list(State(api): State<Arc<ApiState>>) -> Json<Value> {
    let home_hub = match api.ext::<Arc<FabricHandle>>() {
        Some(f) => hub::listed(&*f).await,
        None => None,
    };
    let rows = visible_origins(
        &config(&api),
        home_hub,
        &api.layout.state_dir(),
        &BootClock::now(),
    );
    Json(json!({ "schema": SCHEMA, "origins": rows }))
}

/// The Layer 7 routes of this file (§13.1).
pub fn router() -> ApiRouter {
    axum::Router::new().route("/api/v1/update/origins", get(list))
}
