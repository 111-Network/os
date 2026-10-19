//! `111 daemon` — the node kernel: tokio supervision tree (systemd/runit are thin
//! Restart=always wrappers, never the supervisor of record), watchdog heartbeat,
//! dashboard, updater, KB server, conditional rnsd child.

pub mod bridge;
pub mod dashboard;
pub mod devices;
pub mod fabric;
pub mod health;
pub mod intelligence;
pub mod kb;
pub mod meshtastic;
pub mod panels;
pub mod ports;
pub mod services;
pub mod supervise;
pub mod transport;
pub mod updater;
pub mod watchdog;

use crate::install::categorize::{categorize, Classification, Refusal};
use crate::install::probe::{self, Env, Facts};
use dashboard::{DashboardState, ErrorState, OfferState, SharedState};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Runtime layout resolved once at daemon start.
#[derive(Debug, Clone)]
pub struct DaemonLayout {
    /// Node root: $NODE111_ROOT or /var/lib/111.
    pub root: PathBuf,
    /// Config dir: /etc/111 (host) — under Termux a sibling of root.
    pub etc: PathBuf,
}

impl DaemonLayout {
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }
    pub fn identity_dir(&self) -> PathBuf {
        self.root.join("identity")
    }
    pub fn current_release(&self) -> PathBuf {
        self.root.join("current")
    }
}

/// Parse `/etc/111/env`-style KEY=VALUE files (comments + blanks skipped).
pub fn parse_env_file(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Load node config from `<etc>/env` (missing file = empty config, not an error).
pub fn load_env(etc: &Path) -> anyhow::Result<Vec<(String, String)>> {
    match std::fs::read_to_string(etc.join("env")) {
        Ok(text) => Ok(parse_env_file(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// Category re-probe (daemon start + udev trigger, spec §Detection): collect
/// facts, reclassify, persist facts.json (best-effort write — probing must not
/// kill the daemon).
pub fn reprobe(env: &dyn Env, layout: &DaemonLayout) -> (Facts, Result<Classification, Refusal>) {
    let facts = probe::collect(env);
    let class = categorize(&facts);
    if let Ok(class) = &class {
        let _ = probe::write_facts_json(&layout.state_dir().join("facts.json"), &facts, class);
    }
    (facts, class)
}

/// SIGTERM → SIGKILL grace for each supervised child on daemon shutdown (and
/// for a dashboard per-service restart).
pub const CHILD_STOP_GRACE: Duration = Duration::from_secs(5);

/// Release version currently linked (basename of `current`).
fn current_version(layout: &DaemonLayout) -> String {
    crate::update_helper::current_version(layout).unwrap_or_else(|| "unknown".into())
}

/// Read the offer stub (state/offer.json) into dashboard shape.
fn read_offer(layout: &DaemonLayout) -> Option<OfferState> {
    let text = std::fs::read_to_string(layout.state_dir().join("offer.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(OfferState {
        tier: v["tier"].as_str()?.to_string(),
        state: v["state"].as_str()?.to_string(),
    })
}

/// Probation monitor (daemon side, spec §Updates): while a journal is pending,
/// sustained-green (60s) ⇒ drop `health-confirmed.json`; sustained-red ⇒
/// drop `revert-request.json` (READY-but-503). The root helper acts on both.
async fn probation_monitor(layout: DaemonLayout, state: SharedState) {
    let mut green_since: Option<std::time::Instant> = None;
    let mut red_since: Option<std::time::Instant> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    loop {
        tick.tick().await;
        let journal = match crate::probation::Journal::read(&layout.state_dir().join("update.json"))
        {
            Ok(Some(j)) if j.pending => j,
            _ => {
                green_since = None;
                red_since = None;
                continue;
            }
        };
        let _ = journal;
        let green = health::evaluate(&state.read().unwrap().components) == health::HealthStatus::Ok;
        if green {
            red_since = None;
            let since = green_since.get_or_insert_with(std::time::Instant::now);
            if since.elapsed() >= Duration::from_secs(60) {
                let body = serde_json::json!({"schema": 1, "at": rfc3339_now()});
                let _ = crate::util::fs::atomic_write(
                    &layout.state_dir().join("health-confirmed.json"),
                    body.to_string().as_bytes(),
                    0o644,
                );
                green_since = None;
            }
        } else {
            green_since = None;
            let since = red_since.get_or_insert_with(std::time::Instant::now);
            if since.elapsed() >= Duration::from_secs(60) {
                let body = serde_json::json!({"schema": 1, "reason": "ready-but-unhealthy", "at": rfc3339_now()});
                let _ = crate::util::fs::atomic_write(
                    &layout.state_dir().join("revert-request.json"),
                    body.to_string().as_bytes(),
                    0o644,
                );
                red_since = None;
            }
        }
    }
}

fn rfc3339_now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}

async fn daemon_main() -> anyhow::Result<()> {
    let layout = DaemonLayout {
        root: crate::util::paths::root(),
        etc: if cfg!(target_os = "android") || std::env::var("NODE111_ROOT").is_ok() {
            crate::util::paths::root().join("etc")
        } else {
            PathBuf::from("/etc/111")
        },
    };
    let env_pairs = load_env(&layout.etc)?;
    for (k, v) in &env_pairs {
        if std::env::var(k).is_err() {
            unsafe { std::env::set_var(k, v) };
        }
    }

    let version = current_version(&layout);
    let state: SharedState = Arc::new(RwLock::new(DashboardState::new(&version)));

    // identity (corruption ⇒ distinct error state, never silent re-key).
    // Layer 5 F2: the loaded identity is kept (not dropped) and handed to
    // the fabric below — the identity handle `mesh111` seals through (§1,
    // L2 §13 signer pattern); nothing else holds the keyset.
    let identity = match crate::identity::load(&layout.identity_dir()) {
        Ok(id) => {
            state.write().unwrap().node_id = Some(id.node_id_hex());
            state.write().unwrap().components.identity = true;
            Some(id)
        }
        Err(e) => {
            eprintln!("111 daemon: identity error: {e:#}");
            state.write().unwrap().error_state = Some(ErrorState {
                kind: "identity".into(),
                detail: "identity failed integrity check — `111 identity restore` required".into(),
            });
            None
        }
    };

    // KB snapshot (integrity-verify at start)
    match kb::verify_current(&layout.root) {
        Ok(status) => {
            state.write().unwrap().kb = Some(status);
            state.write().unwrap().components.kb = true;
        }
        Err(e) => eprintln!("111 daemon: KB integrity: {e:#}"),
    }

    // category re-probe + floor + offer stub
    let sysenv = probe::SystemEnv;
    let (_facts, class) = reprobe(&sysenv, &layout);
    if let Ok(class) = &class {
        state.write().unwrap().category = Some(format!("{:?}", class.category));
    }
    if let Ok(Some(floor)) = crate::floor::Floor::read(&layout.state_dir().join("floor.json")) {
        state.write().unwrap().floor_release = Some(floor.highest_release);
    }
    state.write().unwrap().offer = read_offer(&layout);

    // unconditional watchdog heartbeat FIRST (health gates the API, never this)
    let _heartbeat = watchdog::start_heartbeat();

    // transport (conditional child; stub when no python3)
    let runner = crate::util::run::SystemRunner;
    let transport = transport::start(transport::TransportArgs {
        layout: layout.clone(),
        runner: &runner,
        lockfile: layout.root.join("cache/packages/python/rns-lock.txt"),
        cache_py: layout.root.join("cache/packages/python"),
        allow_index: false,
    })
    .await;

    let transport = Arc::new(transport);
    // mirror transport → dashboard components/display state
    {
        let state = Arc::clone(&state);
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        let transport = Arc::clone(&transport);
        tokio::spawn(async move {
            loop {
                tick.tick().await;
                let mut s = state.write().unwrap();
                s.components.transport = transport.health();
                s.transport_state = transport.display_state();
            }
        });
    }

    // Layer 3 children (llama-server + 111-agent; each `absent` unless installed)
    let intelligence = {
        let (category, node_id) = {
            let s = state.read().unwrap();
            (s.category.clone(), s.node_id.clone())
        };
        Arc::new(intelligence::start(
            &layout,
            category.as_deref(),
            node_id.as_deref(),
        ))
    };
    {
        let state = Arc::clone(&state);
        let intelligence = Arc::clone(&intelligence);
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        tokio::spawn(async move {
            loop {
                tick.tick().await;
                state.write().unwrap().intelligence = intelligence.state();
            }
        });
    }

    // probation monitor (health-confirmed / revert-request signals)
    tokio::spawn(probation_monitor(layout.clone(), Arc::clone(&state)));

    // dashboard (Layer 4, D15): loopback unless LAN sharing is on (settings.json
    // lan_enabled, unless DASHBOARD_LAN in the env file overrides it —
    // bind::lan_config); :11180 + :80
    // best-effort; pairing auth for non-loopback peers; mDNS when LAN.
    let lan = dashboard::bind::lan_configured(&env_pairs, &layout.state_dir());
    let dashboard = dashboard::start(Arc::clone(&state), layout.clone(), lan).await;
    // Layer 5 §17: the agent's per-spawn bearer token, checked by require_session
    if let Some(token) = intelligence.daemon_token() {
        dashboard.api.insert_ext(token);
    }

    // updater loop (fetch/verify/stage; never flips — root helper owns that).
    // Started after the dashboard so `POST /api/v1/update/check` can wake it
    // (`ApiState::update_check`) and `/api/v1/status.update` can read it.
    {
        let cfg = updater::fetch::UpdaterConfig::production(
            version
                .parse()
                .unwrap_or_else(|_| "2026.01.01".parse().unwrap()),
        );
        let hooks = updater::Hooks {
            wake: Arc::clone(&dashboard.api.update_check),
            status: Arc::new(updater::UpdaterStatus::default()),
        };
        dashboard.api.insert_ext(Arc::clone(&hooks.status));
        tokio::spawn(updater::run_with_hooks(
            layout.clone(),
            Arc::clone(&state),
            cfg,
            hooks,
        ));
    }

    // Layer 4 R2: services registry (§7 probes, `services` frames), the
    // `status` heartbeat, and the dashboard's restart path (POST
    // /api/v1/node/restart → graceful exit 0 below; the supervisor restarts).
    // Layer 5 §13: hosted panels — embedded declarations, slots, listeners of
    // enabled panels, tickets/sessions; `PanelInfo` facts feed the registry.
    // Built-in shims register here (`meshtastic-http`: the Meshtastic module).
    let mut panel_shims = panels::ShimRegistry::new();
    // Layer 5 §14: the Meshtastic module (meshmud + presence reader children,
    // consented identify, Nearby, the `meshtastic-http` shim) and the
    // daemon-wide port-owner table it claims board ports in.
    let port_table = Arc::new(ports::PortTable::new());
    dashboard.api.insert_ext(Arc::clone(&port_table));
    // §14: the presence reader's venv for the lock `current/` carries, before
    // the module exists (its reader start checks `reader_installed()`)
    ensure_presence_venv(&layout, Arc::new(crate::util::run::SystemRunner)).await;
    let meshtastic = meshtastic::Meshtastic::new(
        meshtastic::MeshtasticConfig::production(
            &layout,
            cfg!(target_os = "android")
                || state.read().unwrap().category.as_deref() == Some("Node2A"),
        ),
        Arc::clone(&port_table),
    );
    dashboard.api.insert_ext(Arc::clone(&meshtastic));
    if let Err(e) = panel_shims.register(Arc::new(meshtastic.shim())) {
        eprintln!("111 daemon: {e}");
    }
    let panel_host = Arc::new(panels::PanelHost::load(
        panels::PanelsConfig::new(&layout),
        &panel_shims,
        dashboard.api.lan_enabled,
    ));
    dashboard.api.insert_ext(Arc::clone(&panel_host));
    // Layer 5 F2: the messaging fabric (store, identity handle, tcp-lan,
    // routes' state, SSE `message`/`contact`/`fabric`, `fabric` service facts).
    let fabric = {
        let category = state.read().unwrap().category.clone();
        fabric::start(
            fabric::FabricConfig::production(&layout, category.as_deref()),
            identity,
            &dashboard.api,
        )
        .await
    };
    dashboard.api.insert_ext(Arc::clone(&fabric));
    let mut sources =
        services::probe::Sources::new(Arc::clone(&intelligence), Arc::clone(&transport));
    sources.panels = Some(panels::provider(&panel_host));
    sources.fabric = Some(fabric.provider());
    meshtastic.install_providers(&mut sources);
    services::probe::spawn(Arc::clone(&dashboard.api), sources);
    let _panels = panels::spawn(Arc::clone(&dashboard.api), Arc::clone(&panel_host));
    // revoked Meshtastic panel sessions close their shim downlink; drop count
    let _mesh_panels = meshtastic.attach_panels(&panel_host);
    // POST /api/v1/services/{id}/restart: the children this daemon supervises
    // (the Meshtastic module re-inserts it with meshmud + the reader added)
    let supervised = services::Supervised::default()
        .with(services::ServiceId::Agent, intelligence.agent_control())
        .with(services::ServiceId::Model, intelligence.model_control())
        .with(services::ServiceId::Transport, transport.control());
    dashboard.api.insert_ext(Arc::new(supervised.clone()));
    dashboard::api_v1::status::spawn_heartbeat(Arc::clone(&dashboard.api));
    let restart = Arc::new(tokio::sync::Notify::new());
    let restart_signal = dashboard::api_v1::actions::RestartSignal(Arc::clone(&restart));
    dashboard.api.insert_ext(restart_signal);
    // device registry (Layer 4 R4): detection + paired sessions → /api/v1/devices
    let devices = devices::start(Arc::clone(&dashboard.api), {
        let transport = Arc::clone(&transport);
        move || transport.child_state() == Some(supervise::ChildState::Running)
    });
    // meshmud + the presence reader from the devices.json binding (Layer 5 §14)
    meshtastic
        .start(Arc::clone(&dashboard.api), devices, supervised)
        .await;
    // agent activity (Layer 4 R3): 111-agent /v1/agent/events → `agent` + `proposal` frames
    let _agent_events = dashboard::api_v1::relay::start_events(&dashboard.api);

    // READY once serving (components may still be warming — /health reflects)
    watchdog::ready();

    // signals: SIGTERM/SIGINT → graceful; voluntary exit(0) = Layer 9 primitive
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = sigterm.recv() => {},
        _ = restart.notified() => {
            eprintln!("111 daemon: restart requested from the dashboard — exiting 0 for the supervisor");
        },
    }
    dashboard.stop();
    // Layer 5: close the fabric cleanly (store clean record + L2 counters)
    fabric.shutdown().await;
    // join every child (SIGTERM, grace, SIGKILL) before exiting: an orphaned
    // llama-server would keep the port and the RAM the next daemon needs
    tokio::join!(
        transport.stop_and_wait(CHILD_STOP_GRACE),
        intelligence.stop_and_wait(CHILD_STOP_GRACE),
        meshtastic.stop_and_wait(CHILD_STOP_GRACE),
    );
    eprintln!("111 daemon: shutting down");
    Ok(())
}

/// Daemon start, before the Meshtastic module is built (os/50-messaging/
/// README.md §14, §19 L1; every promotion restarts the daemon): ensure
/// `venvs/presence` was built from the lock the release in `current/` carries
/// — rebuilt offline from `cache/packages/presence/` when an update changed
/// the lock, nothing at all when it did not, nothing when the release ships no
/// reader (`transport::venv::presence::ensure_current`). Runs on a blocking
/// thread and completes before the reader can start, so the reader starts on
/// the venv this release needs. Never fatal: an unavailable venv is logged
/// here and recorded in `venvs/presence.status.json`, whose reason the
/// `meshtastic` service row shows.
pub async fn ensure_presence_venv(
    layout: &DaemonLayout,
    runner: Arc<dyn crate::util::run::Runner>,
) -> transport::venv::presence::Outcome {
    use transport::venv::presence::{self, Outcome, PresenceVenvError};
    let at = layout.clone();
    let outcome = tokio::task::spawn_blocking(move || presence::ensure_current(&at, &*runner))
        .await
        .unwrap_or_else(|e| Outcome::Unavailable {
            error: PresenceVenvError::Io(format!("the build task failed: {e}")),
            previous_kept: presence::python(&layout.root).is_file(),
        });
    match &outcome {
        Outcome::Built => eprintln!("111 daemon: presence venv rebuilt for this release's lock"),
        Outcome::Unavailable { error, .. } => eprintln!("111 daemon: presence venv: {error}"),
        Outcome::NotShipped | Outcome::Unchanged => {}
    }
    outcome
}

/// `111 daemon` entry (called by systemd/runit).
pub fn run() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("111d")
        .build()?;
    rt.block_on(daemon_main())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_file_parsing() {
        let parsed = parse_env_file("# comment\n\nDASHBOARD_LAN=true\nNODE111_X= y \n");
        assert_eq!(
            parsed,
            vec![
                ("DASHBOARD_LAN".to_string(), "true".to_string()),
                ("NODE111_X".to_string(), "y".to_string())
            ]
        );
    }
}
