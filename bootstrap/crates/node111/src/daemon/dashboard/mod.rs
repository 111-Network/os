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

/// What the URL list is built from. Fixed for the daemon lifetime except the
/// announced mDNS name (a conflict renames it, possibly more than once) and
/// the LAN IPs (re-read on every [`publish_urls`]).
struct UrlInputs {
    main_ok: bool,
    port80_ok: bool,
    lan: bool,
    mdns_host: std::sync::Mutex<Option<String>>,
    started_at: String,
}

/// Recompute the URL list into `ApiState.bind` + `state/dashboard.json`.
/// Writes only on a change unless `force` (the first publish). Returns the
/// list when it was (re)published.
fn publish_urls(api: &ApiState, inp: &UrlInputs, force: bool) -> Option<BindInfo> {
    let lan_ips = if inp.lan {
        bind::lan_ipv4s()
    } else {
        Vec::new()
    };
    let host = inp.mdns_host.lock().unwrap().clone();
    let urls = bind::urls(
        inp.main_ok,
        inp.port80_ok,
        DEFAULT_PORT,
        inp.lan,
        host.as_deref(),
        &lan_ips,
    );
    let info = {
        let mut b = api.bind.write().unwrap();
        if !force && b.urls == urls {
            return None;
        }
        b.urls = urls;
        b.lan = inp.lan;
        b.clone()
    };
    if let Err(e) = bind::write_state(&api.layout.state_dir(), &info, &inp.started_at) {
        eprintln!("111 daemon: cannot write state/dashboard.json: {e:#}");
    }
    Some(info)
}

/// Daemon entry: pairing state, binds (`:11180` + best-effort `:80`), mDNS when
/// LAN, URL list → `ApiState.bind` + `state/dashboard.json`, serve tasks, and
/// (LAN) a 30 s refresh of the own addresses and the URL list (DHCP can move
/// the LAN IP). Never fails: bind errors are logged (the daemon keeps running
/// headless, as before).
pub async fn start(
    state: SharedState,
    layout: crate::daemon::DaemonLayout,
    lan: bool,
) -> DashboardHandle {
    let api = ApiState::new(state, ApiConfig::new(layout.clone(), lan));
    if !routes::ui_embedded() {
        eprintln!("111 daemon: dashboard UI not embedded in this build — / serves a notice; /v1 is the minimal page");
    }
    if !api.auth.loopback_trusted() {
        eprintln!(
            "111 daemon: loopback is not trusted here (Termux, or {}=false) — the phone's own browser pairs like a LAN visitor",
            api_v1::auth::TRUST_LOOPBACK_ENV
        );
    }
    if lan {
        api.set_own_addrs(bind::own_addresses());
        eprintln!("111 daemon: dashboard LAN sharing ON — LAN visitors must pair (`111 dashboard` shows the code)");
    }
    let bound = bind::bind_all(lan, DEFAULT_PORT).await;
    let inputs = Arc::new(UrlInputs {
        main_ok: bound.main_ok,
        port80_ok: bound.port80_ok,
        lan,
        mdns_host: std::sync::Mutex::new(None),
        started_at: rfc3339_now(),
    });

    let mdns = if lan && (bound.main_ok || bound.port80_ok) {
        let port = if bound.port80_ok { 80 } else { DEFAULT_PORT };
        let weak = Arc::downgrade(&api);
        let inp = Arc::clone(&inputs);
        mdns::Announcer::start(
            port,
            Box::new(move |host: String| {
                *inp.mdns_host.lock().unwrap() = Some(host);
                if let Some(api) = weak.upgrade() {
                    publish_urls(&api, &inp, false);
                }
            }),
        )
    } else {
        None
    };
    if mdns.is_some() {
        // a rename may already have landed from the monitor thread; keep it
        inputs
            .mdns_host
            .lock()
            .unwrap()
            .get_or_insert_with(|| mdns::HOSTNAME.trim_end_matches('.').to_string());
    }
    let info = publish_urls(&api, &inputs, true).unwrap_or_default();
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
                if let Some(info) = publish_urls(&api, &inputs, false) {
                    eprintln!(
                        "111 daemon: dashboard URLs changed: {}",
                        info.urls.join(" · ")
                    );
                }
            }
        });
    }
    DashboardHandle { api, mdns }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api_in(dir: &std::path::Path, lan: bool) -> Arc<ApiState> {
        std::fs::create_dir_all(dir.join("state")).unwrap();
        let layout = crate::daemon::DaemonLayout {
            root: dir.to_path_buf(),
            etc: dir.join("etc"),
        };
        let mut cfg = ApiConfig::new(layout, lan);
        cfg.persist_auth = false;
        ApiState::new(Arc::new(RwLock::new(DashboardState::new("t"))), cfg)
    }

    #[test]
    fn url_list_is_republished_when_its_inputs_change() {
        let dir = tempfile::tempdir().unwrap();
        let api = api_in(dir.path(), true);
        let inp = UrlInputs {
            main_ok: true,
            port80_ok: false,
            lan: true,
            mdns_host: std::sync::Mutex::new(Some("111.local".into())),
            started_at: "2026-10-14T00:00:00Z".into(),
        };
        let first = publish_urls(&api, &inp, true).expect("forced");
        assert!(first.urls.contains(&"http://111.local:11180".to_string()));
        assert_eq!(
            bind::read_state(&api.layout.state_dir()).unwrap().urls,
            first.urls
        );
        // nothing changed: no rewrite
        assert!(publish_urls(&api, &inp, false).is_none());
        // a second mDNS rename lands in both the API state and the file
        for host in ["111-2.local", "111-3.local"] {
            *inp.mdns_host.lock().unwrap() = Some(host.into());
            let info = publish_urls(&api, &inp, false).expect("changed");
            let want = format!("http://{host}:11180");
            assert!(info.urls.contains(&want), "{:?}", info.urls);
            assert!(api.bind.read().unwrap().urls.contains(&want));
            assert!(bind::read_state(&api.layout.state_dir())
                .unwrap()
                .urls
                .contains(&want));
        }
    }
}
