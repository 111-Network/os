//! `111 daemon` — the node kernel: tokio supervision tree (systemd/runit are thin
//! Restart=always wrappers, never the supervisor of record), watchdog heartbeat,
//! dashboard, updater, KB server, conditional rnsd child.

pub mod dashboard;
pub mod health;
pub mod intelligence;
pub mod kb;
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

    // identity (corruption ⇒ distinct error state, never silent re-key)
    match crate::identity::load(&layout.identity_dir()) {
        Ok(id) => {
            state.write().unwrap().node_id = Some(id.node_id_hex());
            state.write().unwrap().components.identity = true;
        }
        Err(e) => {
            eprintln!("111 daemon: identity error: {e:#}");
            state.write().unwrap().error_state = Some(ErrorState {
                kind: "identity".into(),
                detail: "identity failed integrity check — `111 identity restore` required".into(),
            });
        }
    }

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

    // updater loop (fetch/verify/stage; never flips — root helper owns that)
    {
        let cfg = updater::fetch::UpdaterConfig::production(
            version
                .parse()
                .unwrap_or_else(|_| "2026.01.01".parse().unwrap()),
        );
        tokio::spawn(updater::run(layout.clone(), Arc::clone(&state), cfg));
    }

    // probation monitor (health-confirmed / revert-request signals)
    tokio::spawn(probation_monitor(layout.clone(), Arc::clone(&state)));

    // dashboard (loopback unless LAN explicitly opted in)
    let lan_opt_in = env_pairs
        .iter()
        .any(|(k, v)| k == "DASHBOARD_LAN" && v == "true");
    let bind: std::net::SocketAddr = if lan_opt_in {
        ([0, 0, 0, 0], dashboard::DEFAULT_PORT).into()
    } else {
        ([127, 0, 0, 1], dashboard::DEFAULT_PORT).into()
    };
    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(e) = dashboard::serve(state, bind, lan_opt_in).await {
                eprintln!("111 daemon: dashboard failed: {e:#}");
            }
        });
    }

    // READY once serving (components may still be warming — /health reflects)
    watchdog::ready();

    // signals: SIGTERM/SIGINT → graceful; voluntary exit(0) = Layer 9 primitive
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = sigterm.recv() => {},
    }
    transport.stop();
    intelligence.stop();
    eprintln!("111 daemon: shutting down");
    Ok(())
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
