//! Conditional transport (Bundle B; Layer 6 transport spine,
//! `os/6-transport/README.md` §11): hash-locked rnsd child of the daemon when
//! python3+venv exist; detection-only stub otherwise. rns identity is
//! independent of node identity in v1.
//!
//! Layer 6 (B4): `state/interfaces.toml` ([`interfaces`]) is the source of
//! truth; rnsd's config is rendered from it ([`config`], renderer v2, 0600);
//! changes go through the [`posture::Spine`] (re-render → diff →
//! `rnstatus --attach|--detach|--reload`, or an rnsd restart with its
//! dependants in order — never `pkill`); interface facts come from a
//! tick-driven [`rnstatus`] cache ([`TransportHandle::facts`]); the phone
//! relay's descriptor and client entry are [`join`]'s.

pub mod config;
pub mod interfaces;
pub mod join;
pub mod netlink;
pub mod posture;
pub mod rnstatus;
pub mod venv;

use crate::daemon::health::TransportHealth;
use crate::daemon::supervise::{self, ChildControl, ChildHandle, ChildSpec};
use crate::daemon::DaemonLayout;
use crate::util::run::Runner;
use posture::Spine;
use rnstatus::{FactsCache, FactsSnapshot, Rnstatus, TickConfig};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

/// Transport bring-up inputs (constructed by the daemon at start).
pub struct TransportArgs<'a> {
    pub layout: DaemonLayout,
    pub runner: &'a dyn Runner,
    pub lockfile: PathBuf,
    pub cache_py: PathBuf,
    /// Install time only: PyPI index allowed for the first build.
    pub allow_index: bool,
}

pub struct TransportHandle {
    child: RwLock<Option<ChildHandle>>,
    peers: Arc<RwLock<PeerState>>,
    stub: bool,
    spine: Arc<Spine>,
    facts: Arc<FactsCache>,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeerState {
    Unknown,
    Isolated,
    Joined,
}

impl TransportHandle {
    /// Child state (test/observability; None on stub).
    pub fn child_state(&self) -> Option<crate::daemon::supervise::ChildState> {
        self.child.read().unwrap().as_ref().map(|c| c.state())
    }

    /// Control of the rnsd child (`None` on the stub or after shutdown).
    pub fn control(&self) -> Option<ChildControl> {
        self.child
            .read()
            .unwrap()
            .as_ref()
            .map(ChildHandle::control)
    }

    /// The transport spine (`interfaces.toml`, posture, applies, join).
    pub fn spine(&self) -> Arc<Spine> {
        Arc::clone(&self.spine)
    }

    /// `transport.read` (§11): the cached rnstatus facts. Never runs
    /// rnstatus; a stale cache wakes the tick.
    pub fn facts(&self) -> FactsSnapshot {
        self.facts.snapshot()
    }

    /// The facts cache (the component runner may read `serving()` from it).
    pub fn facts_cache(&self) -> Arc<FactsCache> {
        Arc::clone(&self.facts)
    }

    fn abort_tasks(&self) {
        for t in self
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain(..)
        {
            t.abort();
        }
    }

    /// Stop the rnsd child without waiting.
    pub fn stop(&self) {
        self.abort_tasks();
        if let Some(c) = self.child.write().unwrap().take() {
            c.stop();
        }
    }

    /// Daemon shutdown: stop the rnsd child and join it (SIGTERM, `grace`,
    /// SIGKILL — [`ChildControl::stop_and_wait`]).
    pub async fn stop_and_wait(&self, grace: std::time::Duration) {
        self.abort_tasks();
        let child = self.child.write().unwrap().take();
        if let Some(c) = child {
            c.stop_and_wait(grace).await;
        }
    }

    pub fn health(&self) -> TransportHealth {
        if self.stub {
            return TransportHealth::DeliberatelyAbsent;
        }
        let child = self.child.read().unwrap();
        match child.as_ref().map(|c| c.state()) {
            Some(supervise::ChildState::Running) => TransportHealth::Alive, // peers refine the display state
            Some(supervise::ChildState::Backoff) => TransportHealth::Alive,
            Some(supervise::ChildState::Degraded) => TransportHealth::Degraded,
            Some(supervise::ChildState::Stopped) | None => TransportHealth::Down,
        }
    }

    /// Dashboard display state (pinned enum values): "joined" from a fresh
    /// rnstatus reading, else the rnsd.log heuristic (kept until facts exist).
    pub fn display_state(&self) -> crate::daemon::dashboard::TransportState {
        use crate::daemon::dashboard::TransportState;
        if self.stub {
            return TransportState::NoTransport;
        }
        match self.health() {
            TransportHealth::Degraded | TransportHealth::Down => TransportState::Degraded,
            _ => {
                let joined = self
                    .facts
                    .joined()
                    .unwrap_or(*self.peers.read().unwrap() == PeerState::Joined);
                if joined {
                    TransportState::Joined
                } else {
                    TransportState::Isolated
                }
            }
        }
    }
}

fn rnsd_spec(layout: &DaemonLayout, venv: &std::path::Path) -> ChildSpec {
    ChildSpec {
        name: "rnsd".into(),
        program: venv.join("bin/rnsd"),
        args: vec![
            "--config".into(),
            config::config_dir(layout).to_string_lossy().into_owned(),
        ],
        env: vec![("HOME".into(), layout.root.to_string_lossy().into_owned())],
        backoff: supervise::BackoffPolicy::default(),
        stdout_log: Some(layout.root.join("rns/rnsd.log")),
    }
}

/// Bring transport up per spec: `interfaces.toml` (load, or migrate on the
/// first Layer 6 start) → python3 probe → venv ensure (integrity check →
/// rebuild from cache when torn) → config render v2 → supervised rnsd child →
/// netlink-triggered re-render/diff/`--reload` → the rnstatus facts tick.
/// Degrades, never panics: no python3 ⇒ `no transport` stub (the spine still
/// holds the file and the Meshtastic binding).
pub async fn start(args: TransportArgs<'_>) -> TransportHandle {
    let layout = args.layout.clone();
    let state_dir = layout.state_dir();
    let startup = interfaces::startup(
        &state_dir,
        &state_dir.join(crate::daemon::devices::store::FILE),
        &config::config_path(&layout),
    );
    let rn = Arc::new(Rnstatus::production(&layout.root));
    let spine = Spine::new(
        layout.clone(),
        startup,
        Arc::clone(&rn),
        config::lan_sharing(&layout),
        config::category_of(&layout),
    );
    let facts = FactsCache::new(TickConfig::default());
    let stub = |spine: Arc<Spine>, facts: Arc<FactsCache>| TransportHandle {
        child: RwLock::new(None),
        peers: Arc::new(RwLock::new(PeerState::Isolated)),
        stub: true,
        spine,
        facts,
        tasks: Mutex::new(Vec::new()),
    };

    if !venv::python3_present(args.runner) {
        eprintln!("111 transport: python3 absent — detection-only stub (no transport)");
        return stub(spine, facts);
    }

    let venv = match venv::ensure(
        &args.layout,
        args.runner,
        &args.lockfile,
        &args.cache_py,
        args.allow_index,
    ) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("111 transport: venv unavailable: {e:#} — no transport");
            return stub(spine, facts);
        }
    };

    // Config: render v2 unless a newer schema-version owns the file.
    let prior = config::existing_schema_version(&layout);
    match spine.write_config() {
        Ok(_) => {
            if matches!(prior, Some(v) if v < config::SCHEMA_VERSION) {
                eprintln!(
                    "111 transport: rns config schema-version {} → {}: rendered from \
                     state/interfaces.toml (share_instance = yes, rpc_key); rnsd starts once with \
                     the new posture after this upgrade",
                    prior.unwrap_or(0),
                    config::SCHEMA_VERSION
                );
            }
        }
        Err(posture::ApplyError::NewerConfig) => eprintln!(
            "111 transport: rns config schema-version {} > {} — a newer release owns it, not \
             regenerating",
            prior.unwrap_or(0),
            config::SCHEMA_VERSION
        ),
        Err(e) => eprintln!("111 transport: {e}"),
    }

    let child = supervise::spawn_child(rnsd_spec(&args.layout, &venv));
    let control = child.control();
    spine.set_rnsd(Arc::new(control.clone()));
    let handle = TransportHandle {
        child: RwLock::new(Some(child)),
        peers: Arc::new(RwLock::new(PeerState::Unknown)),
        stub: false,
        spine: Arc::clone(&spine),
        facts: Arc::clone(&facts),
        tasks: Mutex::new(Vec::new()),
    };
    let mut tasks = Vec::new();

    // netlink → debounced re-render → diff → --reload the AutoInterface
    // (restart only for posture), never pkill (§11).
    if let Ok(events) = netlink::NetlinkEvents::new() {
        let spine = Arc::clone(&spine);
        tasks.push(tokio::spawn(async move {
            netlink::debounced(events, std::time::Duration::from_secs(2), || {
                let spine = Arc::clone(&spine);
                tokio::spawn(async move {
                    match spine.on_link_change().await {
                        Ok(a) if a.restarted || !a.diff.is_empty() => eprintln!(
                            "111 transport: interfaces changed — {}",
                            a.actions.join(", ")
                        ),
                        Ok(_) | Err(posture::ApplyError::NewerConfig) => {}
                        Err(e) => eprintln!("111 transport: link change: {e}"),
                    }
                });
            })
            .await;
        }));
    }

    // the facts tick (rnstatus into the cache); the rnsd.log heuristic is
    // the display fallback until a fresh reading exists
    {
        let peers = Arc::clone(&handle.peers);
        let log = layout.root.join("rns/rnsd.log");
        let running: Arc<dyn Fn() -> bool + Send + Sync> =
            Arc::new(move || control.state() == supervise::ChildState::Running);
        tasks.push(tokio::spawn(rnstatus::tick(
            Arc::clone(&facts),
            rn,
            running,
            move |cache: &FactsCache| {
                if cache.joined().is_none() {
                    *peers.write().unwrap() = peer_state_from_log(&log, 60);
                }
            },
        )));
    }
    *handle.tasks.lock().unwrap_or_else(|p| p.into_inner()) = tasks;

    handle
}

/// Parse the tail of rnsd.log for a recent peer-discovery line.
/// rnsd timestamps look like `[2026-10-12 13:45:02] [Notice] Peer … discovered`.
fn peer_state_from_log(log: &std::path::Path, window_secs: u64) -> PeerState {
    let Ok(text) = std::fs::read_to_string(log) else {
        return PeerState::Unknown;
    };
    let now = time::OffsetDateTime::now_utc();
    let fmt = time::format_description::parse_borrowed::<2>(
        "[year]-[month]-[day] [hour]:[minute]:[second]",
    )
    .expect("static date format is valid");
    for line in text.lines().rev().take(200) {
        if !line.contains("discovered") {
            continue;
        }
        let Some((stamp, _)) = line.trim_start_matches('[').split_once(']') else {
            continue;
        };
        if let Ok(t) = time::PrimitiveDateTime::parse(stamp, &fmt) {
            let age = now - t.assume_utc();
            if age >= time::Duration::ZERO && age <= time::Duration::seconds(window_secs as i64) {
                return PeerState::Joined;
            }
        }
        return PeerState::Isolated; // found a discovery line, but stale
    }
    PeerState::Isolated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_state_parsing() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("rnsd.log");
        // no file
        assert_eq!(peer_state_from_log(&log, 60), PeerState::Unknown);
        // fresh discovery
        let now = time::OffsetDateTime::now_utc();
        let stamp = now
            .format(
                &time::format_description::parse_borrowed::<2>(
                    "[year]-[month]-[day] [hour]:[minute]:[second]",
                )
                .unwrap(),
            )
            .unwrap();
        std::fs::write(
            &log,
            format!("[{stamp}] [Notice] Peer abc123 discovered on eth0\n"),
        )
        .unwrap();
        assert_eq!(peer_state_from_log(&log, 60), PeerState::Joined);
        // stale discovery
        let old = now - time::Duration::minutes(10);
        let stamp = old
            .format(
                &time::format_description::parse_borrowed::<2>(
                    "[year]-[month]-[day] [hour]:[minute]:[second]",
                )
                .unwrap(),
            )
            .unwrap();
        std::fs::write(
            &log,
            format!("[{stamp}] [Notice] Peer abc123 discovered on eth0\n"),
        )
        .unwrap();
        assert_eq!(peer_state_from_log(&log, 60), PeerState::Isolated);
        // no discovery lines at all
        std::fs::write(&log, "[2026-10-12 00:00:00] [Notice] rnsd started\n").unwrap();
        assert_eq!(peer_state_from_log(&log, 60), PeerState::Isolated);
    }
}
