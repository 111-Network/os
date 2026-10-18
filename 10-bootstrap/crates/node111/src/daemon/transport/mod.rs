//! Conditional transport (Bundle B): hash-locked rnsd child of the daemon when
//! python3+venv exist; detection-only stub otherwise. rns identity is independent
//! of node identity in v1.

pub mod config;
pub mod netlink;
pub mod venv;

use crate::daemon::health::TransportHealth;
use crate::daemon::supervise::{self, ChildControl, ChildHandle, ChildSpec};
use crate::daemon::DaemonLayout;
use crate::util::run::Runner;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

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

    /// Stop the rnsd child without waiting.
    pub fn stop(&self) {
        if let Some(c) = self.child.write().unwrap().take() {
            c.stop();
        }
    }

    /// Daemon shutdown: stop the rnsd child and join it (SIGTERM, `grace`,
    /// SIGKILL — [`ChildControl::stop_and_wait`]).
    pub async fn stop_and_wait(&self, grace: std::time::Duration) {
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
            Some(supervise::ChildState::Running) => match *self.peers.read().unwrap() {
                PeerState::Joined => TransportHealth::Alive,
                _ => TransportHealth::Alive, // rnsd up; peers refine the display state
            },
            Some(supervise::ChildState::Backoff) => TransportHealth::Alive,
            Some(supervise::ChildState::Degraded) => TransportHealth::Degraded,
            Some(supervise::ChildState::Stopped) | None => TransportHealth::Down,
        }
    }

    /// Dashboard display state (pinned enum values).
    pub fn display_state(&self) -> crate::daemon::dashboard::TransportState {
        if self.stub {
            return crate::daemon::dashboard::TransportState::NoTransport;
        }
        match self.health() {
            TransportHealth::Degraded => crate::daemon::dashboard::TransportState::Degraded,
            TransportHealth::Down => crate::daemon::dashboard::TransportState::Degraded,
            _ => match *self.peers.read().unwrap() {
                PeerState::Joined => crate::daemon::dashboard::TransportState::Joined,
                _ => crate::daemon::dashboard::TransportState::Isolated,
            },
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

/// Bring transport up per spec: python3 probe → venv ensure (integrity check →
/// rebuild from cache when torn) → config render → supervised rnsd child →
/// netlink-triggered config regen → peer watch. Degrades, never panics:
/// no python3 ⇒ `no transport` stub (receipt records the degrade branch).
pub async fn start(args: TransportArgs<'_>) -> TransportHandle {
    if !venv::python3_present(args.runner) {
        eprintln!("111 transport: python3 absent — detection-only stub (no transport)");
        return TransportHandle {
            child: RwLock::new(None),
            peers: Arc::new(RwLock::new(PeerState::Isolated)),
            stub: true,
        };
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
            return TransportHandle {
                child: RwLock::new(None),
                peers: Arc::new(RwLock::new(PeerState::Isolated)),
                stub: true,
            };
        }
    };

    // Config: regenerate unless a newer schema-version owns the file (Layer 6 seam).
    match config::existing_schema_version(&args.layout) {
        Some(v) if v > config::SCHEMA_VERSION => {
            eprintln!("111 transport: rns config schema-version {v} > v1 — Layer 6 owns it, not regenerating");
        }
        _ => {
            let _ = config::write_if_changed(&args.layout);
        }
    }

    let child = supervise::spawn_child(rnsd_spec(&args.layout, &venv));
    let handle = TransportHandle {
        child: RwLock::new(Some(child)),
        peers: Arc::new(RwLock::new(PeerState::Unknown)),
        stub: false,
    };

    // netlink → debounced config regen + child restart (skip when Layer 6 owns the file).
    if let Ok(events) = netlink::NetlinkEvents::new() {
        let layout = args.layout.clone();
        tokio::spawn(async move {
            netlink::debounced(events, std::time::Duration::from_secs(2), || {
                match config::existing_schema_version(&layout) {
                    Some(v) if v > config::SCHEMA_VERSION => {}
                    _ => {
                        if let Ok(true) = config::write_if_changed(&layout) {
                            eprintln!("111 transport: interfaces changed — rnsd config regenerated; restart picks it up");
                            // rnsd reads config at process start: terminate and let
                            // the supervisor respawn with the fresh config.
                            let rnsd = layout.root.join("venvs/rns/bin/rnsd");
                            let _ = std::process::Command::new("pkill")
                                .arg("-f")
                                .arg(rnsd.to_string_lossy().into_owned())
                                .status();
                        }
                    }
                }
            })
            .await;
        });
    }

    // peer watch: "joined" = a peer discovery line in the last 60s of rnsd.log.
    {
        let peers = Arc::clone(&handle.peers);
        let log = args.layout.root.join("rns/rnsd.log");
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                tick.tick().await;
                *peers.write().unwrap() = peer_state_from_log(&log, 60);
            }
        });
    }

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
