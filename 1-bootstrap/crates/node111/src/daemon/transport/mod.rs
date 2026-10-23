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
//!
//! **rnsd owns its instance (fix wave FA1, §11, §16 G4).** rnstatus is gated
//! on the supervised rnsd holding `@rns/<instance>` ([`rnstatus::owner`]),
//! and after **every** rnsd spawn [`verify_spawns`] checks, within
//! [`SpawnCheck::settle`], that the new rnsd took the socket; one that
//! attached as a client of another holder (its log says "connected to
//! another shared local instance", or it never holds the socket by the end
//! of the settle) is terminated, so the supervisor respawns it under its
//! own back-off and crash-loop policy (counted, one log line each). It runs
//! in the facts tick's task (no task or timer of its own): it waits on the
//! child's state watch and polls `/proc` only inside the settle after a
//! spawn.

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
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

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
    /// The daemon root and its [`rnstatus::owner::register`] token.
    root: PathBuf,
    owner_token: u64,
    /// rnsd spawns terminated for not owning the instance (FA1).
    client_restarts: Arc<AtomicU64>,
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

    /// How many rnsd spawns were terminated because they did not own the
    /// shared instance (attached as a client of another holder, FA1).
    pub fn client_restarts(&self) -> u64 {
        self.client_restarts.load(Ordering::SeqCst)
    }

    fn abort_tasks(&self) {
        rnstatus::owner::unregister(&self.root, self.owner_token);
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
        env: vec![
            ("HOME".into(), layout.root.to_string_lossy().into_owned()),
            // rnsd logs with print(): unbuffered, its "connected to another
            // shared local instance" line reaches rnsd.log at once (FA1)
            ("PYTHONUNBUFFERED".into(), "1".into()),
        ],
        backoff: supervise::BackoffPolicy::default(),
        stdout_log: Some(layout.root.join("rns/rnsd.log")),
    }
}

/// Timing of the post-spawn ownership check ([`verify_spawns`], FA1).
#[derive(Debug, Clone)]
pub struct SpawnCheck {
    /// How long a fresh rnsd may take to hold the shared instance (Python
    /// start-up on a Pi under load included) before it is restarted.
    pub settle: Duration,
    /// The `/proc` poll interval inside the settle.
    pub poll: Duration,
    /// SIGTERM → SIGKILL grace when a client rnsd is terminated.
    pub grace: Duration,
    /// Consecutive terminations without an rnsd that owns the instance
    /// after which the check stops terminating (another program holds the
    /// instance for good: restarting rnsd cannot fix it, the gates stay
    /// closed, one log line says so). An owning spawn resets the count.
    pub max_consecutive: u32,
}

impl Default for SpawnCheck {
    fn default() -> Self {
        SpawnCheck {
            settle: Duration::from_secs(30),
            poll: Duration::from_millis(250),
            grace: crate::daemon::CHILD_STOP_GRACE,
            max_consecutive: 5,
        }
    }
}

/// The phrase rnsd logs when it started as a client of another instance
/// (RNS 1.5.7 `Utilities/rnsd.py`).
pub const CLIENT_LINE: &str = "connected to another shared local instance";
/// Most of rnsd.log scanned per check (bytes past the spawn's mark).
const MAX_LOG_SCAN: u64 = 64 * 1024;

fn log_len(log: &Path) -> u64 {
    std::fs::metadata(log).map(|m| m.len()).unwrap_or(0)
}

/// Whether rnsd.log, past `mark`, carries [`CLIENT_LINE`].
fn log_says_client(log: &Path, mark: u64) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(log) else {
        return false;
    };
    if f.seek(SeekFrom::Start(mark)).is_err() {
        return false;
    }
    let mut buf = Vec::new();
    if f.take(MAX_LOG_SCAN).read_to_end(&mut buf).is_err() {
        return false;
    }
    buf.windows(CLIENT_LINE.len())
        .any(|w| w == CLIENT_LINE.as_bytes())
}

/// The post-spawn check of one rnsd `pid`: `None` when it holds the
/// instance (or exited, or no instance is shared), else why it is a client.
async fn settle(
    control: &ChildControl,
    rn: &Rnstatus,
    pid: u32,
    log: &Path,
    mark: u64,
    cfg: &SpawnCheck,
) -> Option<String> {
    let t0 = tokio::time::Instant::now();
    loop {
        if control.pid() != Some(pid) || !rn.shares_instance() {
            return None;
        }
        if rnstatus::owner::owned_by(rn.root(), pid) {
            return None;
        }
        if log_says_client(log, mark) {
            return Some(format!(
                "started as a client of another holder of the shared instance (rnsd.log: \
                 \"{CLIENT_LINE}\")"
            ));
        }
        if t0.elapsed() >= cfg.settle {
            return Some(format!(
                "did not take the shared instance within {:?}",
                cfg.settle
            ));
        }
        tokio::time::sleep(cfg.poll).await;
    }
}

/// SIGTERM `pid` (still the supervised child), SIGKILL after `grace`.
async fn terminate(control: &ChildControl, pid: u32, grace: Duration) {
    if control.pid() != Some(pid) {
        return;
    }
    // SAFETY: kill(2) on our own supervised child's pid, just re-checked
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    let t0 = tokio::time::Instant::now();
    while control.pid() == Some(pid) && t0.elapsed() < grace {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if control.pid() == Some(pid) {
        // SAFETY: as above
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    }
}

/// After every rnsd spawn (module doc, FA1): wait on `control`'s state; for
/// each new pid, check within `cfg.settle` that it holds the shared
/// instance; terminate one that does not, so the supervisor respawns it
/// under its back-off and crash-loop policy. `restarts` counts the
/// terminations. Ends when the supervisor is gone.
pub async fn verify_spawns(
    control: ChildControl,
    rn: Arc<Rnstatus>,
    log: PathBuf,
    cfg: SpawnCheck,
    restarts: Arc<AtomicU64>,
) {
    let mut w = control.state_watch();
    let mut checked: Option<u32> = None;
    let mut consecutive = 0u32;
    // where the next spawn's lines start: moved on while rnsd is not
    // running and after each check, so a new spawn's first line is never
    // before it (a mark taken when the spawn is seen could be late)
    let mut mark = log_len(&log);
    loop {
        let running = *w.borrow_and_update() == supervise::ChildState::Running;
        if !running {
            mark = log_len(&log);
        }
        if let (true, Some(pid)) = (running, control.pid()) {
            if checked != Some(pid) {
                checked = Some(pid);
                let verdict = settle(&control, &rn, pid, &log, mark, &cfg).await;
                match verdict {
                    None => {
                        if rnstatus::owner::owned_by(rn.root(), pid) {
                            consecutive = 0;
                        }
                    }
                    Some(why) if consecutive >= cfg.max_consecutive => {
                        if consecutive == cfg.max_consecutive {
                            consecutive += 1;
                            eprintln!(
                                "111 transport: rnsd (pid {pid}) {why}; {}; {} restarts did not \
                                 help — left running, nothing is served through it until it \
                                 owns the instance",
                                holder(&rn, pid),
                                cfg.max_consecutive
                            );
                        }
                    }
                    Some(why) => {
                        consecutive += 1;
                        let n = restarts.fetch_add(1, Ordering::SeqCst) + 1;
                        eprintln!(
                            "111 transport: rnsd (pid {pid}) {why}; {}; restarting it (client \
                             restart {n})",
                            holder(&rn, pid)
                        );
                        terminate(&control, pid, cfg.grace).await;
                    }
                }
                if control.pid().is_none() {
                    // between spawns: the next one's lines come after this
                    mark = log_len(&log);
                }
                continue;
            }
        }
        if w.changed().await.is_err() {
            return;
        }
    }
}

/// "the instance is held by pid N (comm)" or "nothing holds the instance".
fn holder(rn: &Rnstatus, rnsd: u32) -> String {
    match rnstatus::owner::shared_instance_owner(rn.root(), Some(rnsd)) {
        Some(p) => format!(
            "the instance is held by pid {p} ({})",
            rnstatus::owner::comm(p)
        ),
        None => "nothing holds the instance".into(),
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
    // FA1: rnstatus runs only while the supervised rnsd holds the shared
    // instance; until rnsd is spawned (and on the stub) the slot is empty,
    // so nothing runs (fail closed). The components runner's serving probe
    // reads the same pid through the registry.
    let rnsd_slot: Arc<RwLock<Option<ChildControl>>> = Arc::new(RwLock::new(None));
    let rnsd_pid: rnstatus::PidSource = {
        let slot = Arc::clone(&rnsd_slot);
        Arc::new(move || {
            slot.read()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
                .and_then(ChildControl::pid)
        })
    };
    rn.set_owner_gate(Arc::clone(&rnsd_pid));
    let owner_token = rnstatus::owner::register(&layout.root, rnsd_pid);
    let client_restarts = Arc::new(AtomicU64::new(0));
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
        root: layout.root.clone(),
        owner_token,
        client_restarts: Arc::clone(&client_restarts),
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
    *rnsd_slot.write().unwrap_or_else(|p| p.into_inner()) = Some(control.clone());
    spine.set_rnsd(Arc::new(control.clone()));
    let handle = TransportHandle {
        child: RwLock::new(Some(child)),
        peers: Arc::new(RwLock::new(PeerState::Unknown)),
        stub: false,
        spine: Arc::clone(&spine),
        facts: Arc::clone(&facts),
        tasks: Mutex::new(Vec::new()),
        root: layout.root.clone(),
        owner_token,
        client_restarts: Arc::clone(&client_restarts),
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
    // the display fallback until a fresh reading exists. The post-spawn
    // ownership check (FA1) shares its task: no task of its own.
    {
        let peers = Arc::clone(&handle.peers);
        let log = layout.root.join("rns/rnsd.log");
        let verify = verify_spawns(
            control.clone(),
            Arc::clone(&rn),
            log.clone(),
            SpawnCheck::default(),
            client_restarts,
        );
        let running: Arc<dyn Fn() -> bool + Send + Sync> =
            Arc::new(move || control.state() == supervise::ChildState::Running);
        let tick = rnstatus::tick(
            Arc::clone(&facts),
            rn,
            running,
            move |cache: &FactsCache| {
                if cache.joined().is_none() {
                    *peers.write().unwrap() = peer_state_from_log(&log, 60);
                }
            },
        );
        tasks.push(tokio::spawn(async move {
            tokio::join!(tick, verify);
        }));
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
