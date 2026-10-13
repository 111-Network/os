//! Dashboard (Layer 1 seam, Layer 4 surface — D15). `111d` serves the embedded
//! Layer 4 UI on `:11180` (+ `:80` best-effort), `/health` + `/api/status` per
//! the pinned Layer 1 contracts, the Layer 1 minimal page at `/v1`, and the
//! `/api/v1` contract (`os/interface/api/openapi.yaml`) with pairing auth.
//!
//! - `routes`  — app assembly, Host guard, static UI, legacy routes
//! - `api_v1`  — `/api/v1/*` (auth, events bus, R2/R3 submodules)
//! - `bind`    — listeners, LAN config, own addresses, `state/dashboard.json`
//! - `mdns`    — `111.local` announce while LAN-bound

pub mod api_v1;
pub mod bind;
pub mod mdns;
pub mod routes;

pub use api_v1::{ApiConfig, ApiState};
pub use bind::BindInfo;

use crate::daemon::health::Components;
use crate::daemon::intelligence::IntelligenceState;
use serde::Serialize;
use std::sync::{Arc, RwLock};
use std::time::Instant;

pub const DEFAULT_PORT: u16 = 11180;

/// Transport display state (pinned enum — Layer 5/6 seam).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TransportState {
    #[serde(rename = "no transport")]
    NoTransport,
    #[serde(rename = "isolated")]
    Isolated,
    #[serde(rename = "joined")]
    Joined,
    #[serde(rename = "transport degraded")]
    Degraded,
}

/// Standard-tier offer stub (v1 records intent only; Layer 3 replaces the hook).
#[derive(Debug, Clone, Serialize)]
pub struct OfferState {
    pub tier: String,
    pub state: String, // "offered" | "accepted" | "dismissed"
}

/// Distinct error surfaces (identity corruption, …) — never silent.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorState {
    pub kind: String,
    pub detail: String,
}

/// Everything the dashboard/API serves. Writers: daemon core (identity, kb,
/// transport, health), updater (last-successful-check, pending), offer stub.
pub struct DashboardState {
    /// NodeId, §7 machine string (lowercase hex of the 32B Ed25519 public key).
    pub node_id: Option<String>,
    pub category: Option<String>,
    pub version: String,
    pub started: Instant,
    pub kb: Option<crate::daemon::kb::KbStatus>,
    pub transport_state: TransportState,
    pub components: Components,
    pub last_successful_update_check: Option<String>, // RFC3339
    pub update_pending: bool,
    pub floor_release: Option<String>,
    pub clock_fault: bool,
    pub offer: Option<OfferState>,
    pub error_state: Option<ErrorState>,
    /// Layer 3 children (agent + model server); written by the daemon's mirror task.
    pub intelligence: IntelligenceState,
}

impl DashboardState {
    pub fn new(version: &str) -> Self {
        DashboardState {
            node_id: None,
            category: None,
            version: version.to_string(),
            started: Instant::now(),
            kb: None,
            transport_state: TransportState::NoTransport,
            components: Components {
                identity: false,
                kb: false,
                transport: crate::daemon::health::TransportHealth::Down,
                updater_idle: true,
            },
            last_successful_update_check: None,
            update_pending: false,
            floor_release: None,
            clock_fault: false,
            offer: None,
            error_state: None,
            intelligence: IntelligenceState::default(),
        }
    }

    pub fn uptime_sec(&self) -> u64 {
        self.started.elapsed().as_secs()
    }
}

pub type SharedState = Arc<RwLock<DashboardState>>;

/// Serve `app` on `listener` with peer addresses attached (auth needs them).
pub async fn serve_listener(
    listener: tokio::net::TcpListener,
    app: axum::Router,
) -> std::io::Result<()> {
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
}

/// Compat entry (tests, tools): serve on one address with an in-memory auth
/// store and a throwaway layout. Refuses non-loopback without `lan_opt_in`.
/// The daemon uses [`start`].
pub async fn serve(
    state: SharedState,
    bind: std::net::SocketAddr,
    lan_opt_in: bool,
) -> anyhow::Result<()> {
    let listener = bind::bind_listener(bind, lan_opt_in).await?;
    let layout = crate::daemon::DaemonLayout {
        root: std::env::temp_dir().join("111-dashboard-ephemeral"),
        etc: std::env::temp_dir().join("111-dashboard-ephemeral/etc"),
    };
    let mut cfg = ApiConfig::new(layout, lan_opt_in);
    cfg.persist_auth = false;
    let api = ApiState::new(state, cfg);
    if lan_opt_in {
        api.set_own_addrs(bind::own_addresses());
    }
    serve_listener(listener, routes::app(api)).await?;
    Ok(())
}

/// Running dashboard: the shared API state (R2/R3 hooks publish on
/// `api.events`, insert into `api.ext`) + the mDNS announcer.
pub struct DashboardHandle {
    pub api: Arc<ApiState>,
    mdns: Option<mdns::Announcer>,
}

impl DashboardHandle {
    /// Withdraw the mDNS record (listeners die with the runtime).
    pub fn stop(&self) {
        if let Some(a) = &self.mdns {
            a.stop();
        }
    }
}

fn rfc3339_now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}

/// Daemon entry: pairing state, binds (`:11180` + best-effort `:80`), mDNS when
/// LAN, URL list → `ApiState.bind` + `state/dashboard.json`, serve tasks, own-
/// address refresh (30 s). Never fails: bind errors are logged (the daemon keeps
/// running headless, as before).
pub async fn start(
    state: SharedState,
    layout: crate::daemon::DaemonLayout,
    lan: bool,
) -> DashboardHandle {
    let api = ApiState::new(state, ApiConfig::new(layout.clone(), lan));
    if !routes::ui_embedded() {
        eprintln!("111 daemon: dashboard UI not embedded in this build — / serves a notice; /v1 is the minimal page");
    }
    if lan {
        api.set_own_addrs(bind::own_addresses());
        eprintln!("111 daemon: dashboard LAN sharing ON — LAN visitors must pair (`111 dashboard` shows the code)");
    }
    let bound = bind::bind_all(lan, DEFAULT_PORT).await;
    let started_at = rfc3339_now();

    let mdns = if lan && (bound.main_ok || bound.port80_ok) {
        let port = if bound.port80_ok { 80 } else { DEFAULT_PORT };
        let weak = Arc::downgrade(&api);
        let (main_ok, port80_ok, started) = (bound.main_ok, bound.port80_ok, started_at.clone());
        mdns::Announcer::start(
            port,
            Box::new(move |host: String| {
                if let Some(api) = weak.upgrade() {
                    let urls = bind::urls(
                        main_ok,
                        port80_ok,
                        DEFAULT_PORT,
                        true,
                        Some(&host),
                        &bind::lan_ipv4s(),
                    );
                    let info = {
                        let mut b = api.bind.write().unwrap();
                        b.urls = urls;
                        b.clone()
                    };
                    let _ = bind::write_state(&api.layout.state_dir(), &info, &started);
                }
            }),
        )
    } else {
        None
    };
    let mdns_host = mdns
        .as_ref()
        .map(|_| mdns::HOSTNAME.trim_end_matches('.').to_string());
    let lan_ips = if lan { bind::lan_ipv4s() } else { Vec::new() };
    let info = BindInfo {
        urls: bind::urls(
            bound.main_ok,
            bound.port80_ok,
            DEFAULT_PORT,
            lan,
            mdns_host.as_deref(),
            &lan_ips,
        ),
        lan,
    };
    *api.bind.write().unwrap() = info.clone();
    if let Err(e) = bind::write_state(&layout.state_dir(), &info, &started_at) {
        eprintln!("111 daemon: cannot write state/dashboard.json: {e:#}");
    }
    if !info.urls.is_empty() {
        eprintln!("111 daemon: dashboard at {}", info.urls.join(" · "));
    }

    let app = routes::app(Arc::clone(&api));
    for listener in bound.listeners {
        let app = app.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_listener(listener, app).await {
                eprintln!("111 daemon: dashboard failed: {e:#}");
            }
        });
    }
    if lan {
        let weak = Arc::downgrade(&api);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
            tick.tick().await;
            loop {
                tick.tick().await;
                let Some(api) = weak.upgrade() else { break };
                api.set_own_addrs(bind::own_addresses());
            }
        });
    }
    DashboardHandle { api, mdns }
}
