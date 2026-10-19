//! Child ordering for optional components (Layer 6 §6 "Start order", §2
//! "Process shape", §18 Layer 1: **bridge-core orchestration over
//! `stop_and_wait`/`restart` and fresh spawns, not a supervisor feature**).
//!
//! A [`ComponentRunner`] owns one component child (piped stdio, §7) and, when
//! the component names a dependency (`[requires] service`, e.g. rnsd for the
//! LXMF helper), a [`Gate`]:
//! - it **spawns only while** the dependency's supervised child is `Running`
//!   **and** the injectable `serving` probe answers true (production:
//!   [`rnstatus_serving`], `rnstatus` from `venvs/rns` showing the shared
//!   instance), else it waits (`Waiting {dependency_down | not_serving}`) on the
//!   dependency's state watch, re-probing every `retry`;
//! - after **every** spawn, the supervisor's back-off respawns included (each
//!   publishes a new [`PipedIo`] generation), it re-runs the probe; on failure
//!   it stops the child, reports `AdapterDown { partitioned }` (the helper may
//!   have become the instance itself, R07 check 1b) and goes back to the gate,
//!   so it retries once the dependency serves;
//! - when the dependency leaves `Running` it stops the child and waits again
//!   (fail closed: never a helper without its instance);
//! - [`ComponentRunner::stop_before`] / [`ComponentRunner::start_after`] are
//!   the hooks a planned dependency restart calls (the transport spine, B4):
//!   the child is stopped and held until resumed.
//!
//! The adapter attaches through [`ComponentRunner::stdio`]: the current
//! child's [`PipedIo`], forwarded only after the post-spawn check passed and
//! re-published on every respawn; `None` while nothing runs.
//!
//! Weight: a runner exists only while its component is enabled (§2.1 rule 2);
//! its retry timer and probe run only then. No wall clock is read.

use crate::daemon::supervise::{
    spawn_child, ChildControl, ChildHandle, ChildSpec, ChildState, ChildStdio, PipedIo,
};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};

/// The serving probe (blocking; run off the async threads).
pub type ServingProbe = Arc<dyn Fn() -> bool + Send + Sync>;
/// Where the runner finds the dependency's child each time it gates (the
/// supervised child may be replaced by a restart path).
pub type DependencySource = Arc<dyn Fn() -> Option<ChildControl> + Send + Sync>;

/// The dependency a component starts after (§6 "Start order").
#[derive(Clone)]
pub struct Gate {
    pub dependency: DependencySource,
    pub serving: ServingProbe,
}

/// Timing of a runner.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// Re-probe interval while waiting for the dependency to serve.
    pub retry: Duration,
    /// Wait after a spawn before the post-spawn probe (the child attaches).
    pub settle: Duration,
    /// SIGTERM → SIGKILL grace when the runner stops the child.
    pub grace: Duration,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        RunnerConfig {
            retry: Duration::from_secs(5),
            settle: Duration::from_secs(1),
            grace: crate::daemon::CHILD_STOP_GRACE,
        }
    }
}

/// What the runner is doing (B2's adapter registry reads it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerStatus {
    /// Not spawned: `dependency_down` (its child is not running) or
    /// `not_serving` (the probe said no).
    Waiting { reason: &'static str },
    /// The child runs and passed the post-spawn check.
    Running { generation: u64 },
    /// `adapter_down`: the post-spawn check failed (`partitioned`); the child
    /// was stopped and the runner retries once the dependency serves.
    AdapterDown { reason: &'static str },
    /// Held by [`ComponentRunner::stop_before`].
    Held,
    /// Shut down.
    Stopped,
}

enum Cmd {
    Hold(oneshot::Sender<()>),
    Resume,
    Shutdown(oneshot::Sender<()>),
}

/// One component's child and its ordering task (module doc).
pub struct ComponentRunner {
    status_rx: watch::Receiver<RunnerStatus>,
    stdio_rx: watch::Receiver<Option<PipedIo>>,
    child_rx: watch::Receiver<Option<ChildControl>>,
    cmd_tx: mpsc::Sender<Cmd>,
    task: tokio::task::JoinHandle<()>,
}

impl ComponentRunner {
    /// Start ordering `spec` (spawned with piped stdio) after `gate`
    /// (`None`: no dependency, spawn at once). Must be called within a tokio
    /// runtime.
    pub fn start(spec: ChildSpec, gate: Option<Gate>, cfg: RunnerConfig) -> Self {
        let first = if gate.is_some() {
            RunnerStatus::Waiting {
                reason: "dependency_down",
            }
        } else {
            RunnerStatus::Waiting { reason: "starting" }
        };
        let (status_tx, status_rx) = watch::channel(first);
        let (stdio_tx, stdio_rx) = watch::channel(None);
        let (child_tx, child_rx) = watch::channel(None);
        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let task = tokio::spawn(
            Task {
                spec,
                gate,
                cfg,
                status_tx,
                stdio_tx,
                child_tx,
                cmd_rx,
            }
            .run(),
        );
        ComponentRunner {
            status_rx,
            stdio_rx,
            child_rx,
            cmd_tx,
            task,
        }
    }

    pub fn status(&self) -> RunnerStatus {
        *self.status_rx.borrow()
    }

    pub fn status_watch(&self) -> watch::Receiver<RunnerStatus> {
        self.status_rx.clone()
    }

    /// The current child's stdio, after its post-spawn check (module doc).
    pub fn stdio(&self) -> watch::Receiver<Option<PipedIo>> {
        self.stdio_rx.clone()
    }

    /// The current child, while one is spawned.
    pub fn child(&self) -> Option<ChildControl> {
        self.child_rx.borrow().clone()
    }

    /// The child is up and passed its post-spawn check.
    pub fn is_running(&self) -> bool {
        matches!(self.status(), RunnerStatus::Running { .. })
            && self
                .child()
                .is_some_and(|c| c.state() == ChildState::Running)
    }

    /// Before a planned dependency restart: stop the child and hold it until
    /// [`Self::start_after`]. Returns once it is stopped.
    pub async fn stop_before(&self) {
        let (tx, rx) = oneshot::channel();
        if self.cmd_tx.send(Cmd::Hold(tx)).await.is_ok() {
            let _ = rx.await;
        }
    }

    /// After the dependency restarted: gate and spawn again.
    pub async fn start_after(&self) {
        let _ = self.cmd_tx.send(Cmd::Resume).await;
    }

    /// Stop the child and end the runner.
    pub async fn shutdown(self) {
        let (tx, rx) = oneshot::channel();
        if self.cmd_tx.send(Cmd::Shutdown(tx)).await.is_ok() {
            let _ = rx.await;
        }
        let _ = self.task.await;
    }
}

struct Task {
    spec: ChildSpec,
    gate: Option<Gate>,
    cfg: RunnerConfig,
    status_tx: watch::Sender<RunnerStatus>,
    stdio_tx: watch::Sender<Option<PipedIo>>,
    child_tx: watch::Sender<Option<ChildControl>>,
    cmd_rx: mpsc::Receiver<Cmd>,
}

/// What a wait ended with.
enum Next {
    /// Gate again.
    Again,
    /// Hold until resumed.
    Hold(oneshot::Sender<()>),
    /// End (`None`: the runner handle is gone).
    Shutdown(Option<oneshot::Sender<()>>),
}

async fn probe(serving: &ServingProbe) -> bool {
    let f = Arc::clone(serving);
    tokio::task::spawn_blocking(move || f())
        .await
        .unwrap_or(false)
}

/// Wait for a change of `dep`'s state, at most `retry`.
async fn dep_change_or(dep: Option<&ChildControl>, retry: Duration) {
    match dep {
        Some(d) => {
            let mut w = d.state_watch();
            let _ = tokio::time::timeout(retry, async {
                if w.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            })
            .await;
        }
        None => tokio::time::sleep(retry).await,
    }
}

/// Resolves when `w`'s child leaves `Running` (or its supervisor is gone);
/// never without a dependency.
async fn left_running(w: &mut Option<watch::Receiver<ChildState>>) {
    match w {
        Some(w) => {
            let _ = w.wait_for(|s| *s != ChildState::Running).await;
        }
        None => std::future::pending().await,
    }
}

impl Task {
    fn set(&self, s: RunnerStatus) {
        self.status_tx.send_replace(s);
    }

    async fn stop_child(&self, h: ChildHandle) {
        self.stdio_tx.send_replace(None);
        h.stop_and_wait(self.cfg.grace).await;
        self.child_tx.send_replace(None);
    }

    fn on_cmd(cmd: Option<Cmd>) -> Next {
        match cmd {
            Some(Cmd::Hold(ack)) => Next::Hold(ack),
            Some(Cmd::Resume) => Next::Again,
            Some(Cmd::Shutdown(ack)) => Next::Shutdown(Some(ack)),
            None => Next::Shutdown(None),
        }
    }

    async fn run(mut self) {
        let mut partitioned = false;
        let mut next = Next::Again;
        loop {
            match next {
                Next::Shutdown(ack) => {
                    self.set(RunnerStatus::Stopped);
                    if let Some(ack) = ack {
                        let _ = ack.send(());
                    }
                    return;
                }
                Next::Hold(ack) => {
                    self.set(RunnerStatus::Held);
                    let _ = ack.send(());
                    // held: only a resume or a shutdown moves on
                    next = loop {
                        match Self::on_cmd(self.cmd_rx.recv().await) {
                            Next::Hold(ack) => {
                                let _ = ack.send(());
                            }
                            other => break other,
                        }
                    };
                    continue;
                }
                Next::Again => {}
            }

            // ---- gate (§6: spawn only while the dependency runs and serves)
            if let Some(gate) = self.gate.clone() {
                let dep = (gate.dependency)();
                let running = dep
                    .as_ref()
                    .is_some_and(|d| d.state() == ChildState::Running);
                let reason = if !running {
                    Some("dependency_down")
                } else if !probe(&gate.serving).await {
                    Some("not_serving")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    if !partitioned {
                        self.set(RunnerStatus::Waiting { reason });
                    }
                    next = tokio::select! {
                        cmd = self.cmd_rx.recv() => Self::on_cmd(cmd),
                        _ = dep_change_or(dep.as_ref(), self.cfg.retry) => Next::Again,
                    };
                    continue;
                }
            }

            // ---- spawn
            let handle = spawn_child(self.spec.clone().with_stdio(ChildStdio::Piped));
            self.child_tx.send_replace(Some(handle.control()));
            let mut stdio = handle.stdio();
            let mut dep_state = self
                .gate
                .as_ref()
                .and_then(|g| (g.dependency)())
                .map(|d| d.state_watch());
            let mut seen = 0u64;
            next = loop {
                let current = stdio.borrow_and_update().clone();
                match current {
                    Some(io) if io.generation() != seen => {
                        seen = io.generation();
                        // the post-spawn check, after every spawn (respawns too)
                        if let Some(gate) = self.gate.clone() {
                            tokio::time::sleep(self.cfg.settle).await;
                            if !probe(&gate.serving).await {
                                eprintln!(
                                    "111 components: {} started but its dependency no longer \
                                     serves (partitioned); stopped, retrying once it serves",
                                    self.spec.name
                                );
                                self.stop_child(handle).await;
                                partitioned = true;
                                self.set(RunnerStatus::AdapterDown {
                                    reason: "partitioned",
                                });
                                break Next::Again;
                            }
                        }
                        partitioned = false;
                        self.stdio_tx.send_replace(Some(io));
                        self.set(RunnerStatus::Running { generation: seen });
                    }
                    Some(_) => {}
                    None => {
                        self.stdio_tx.send_replace(None);
                    }
                }
                tokio::select! {
                    changed = stdio.changed() => {
                        if changed.is_err() {
                            // the supervisor task is gone: stop and gate again
                            self.stop_child(handle).await;
                            break Next::Again;
                        }
                    }
                    _ = left_running(&mut dep_state) => {
                        self.stop_child(handle).await;
                        self.set(RunnerStatus::Waiting { reason: "dependency_down" });
                        break Next::Again;
                    }
                    cmd = self.cmd_rx.recv() => {
                        match Self::on_cmd(cmd) {
                            Next::Again => {}
                            other => {
                                self.stop_child(handle).await;
                                break other;
                            }
                        }
                    }
                }
            };
        }
    }
}

/// Longest an `rnstatus` probe may run.
pub const RNSTATUS_TIMEOUT: Duration = Duration::from_secs(10);

/// The production serving probe for `transport` (§6): `rnstatus` from
/// `venvs/rns`, run against the daemon's rnsd config dir, exits 0 and
/// reports a shared instance. *Unverified* against a live rnsd here (G3/G4
/// run it); the transport spine may replace it with its cached `rnstatus`
/// reading. Blocking; a probe that hangs is killed after
/// [`RNSTATUS_TIMEOUT`] and answers false.
pub fn rnstatus_serving(root: &Path) -> bool {
    let bin = root.join("venvs/rns/bin/rnstatus");
    let config = root.join("rns");
    let child = std::process::Command::new(&bin)
        .arg("--config")
        .arg(&config)
        .env("HOME", root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    let Some(mut out) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    };
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        let _ = (&mut out).take(256 * 1024).read_to_end(&mut buf);
        buf
    });
    let t0 = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if t0.elapsed() < RNSTATUS_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let text = reader.join().unwrap_or_default();
    status.is_some_and(|s| s.success())
        && String::from_utf8_lossy(&text).contains("Shared Instance")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::supervise::BackoffPolicy;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const LIMIT: Duration = Duration::from_secs(5);

    fn sh(name: &str, script: &str) -> ChildSpec {
        ChildSpec {
            name: name.into(),
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), script.into(), "sh".into()],
            env: vec![],
            backoff: BackoffPolicy {
                initial_ms: 20,
                max_ms: 40,
                jitter_pct: 0,
                ..BackoffPolicy::default()
            },
            stdout_log: None,
        }
    }

    fn fast() -> RunnerConfig {
        RunnerConfig {
            retry: Duration::from_millis(30),
            settle: Duration::ZERO,
            grace: Duration::from_secs(2),
        }
    }

    async fn status_is(r: &ComponentRunner, f: impl Fn(RunnerStatus) -> bool) -> bool {
        let mut w = r.status_watch();
        let reached = tokio::time::timeout(LIMIT, w.wait_for(|s| f(*s)))
            .await
            .is_ok();
        reached
    }

    #[tokio::test]
    async fn without_a_gate_it_spawns_and_forwards_stdio() {
        let r = ComponentRunner::start(sh("c", "echo ready; exec cat"), None, fast());
        assert!(status_is(&r, |s| matches!(s, RunnerStatus::Running { .. })).await);
        let io = r.stdio().borrow().clone().unwrap();
        assert_eq!(io.first_line().await.as_deref(), Some("ready"));
        assert!(r.is_running());
        r.shutdown().await;
    }

    #[tokio::test]
    async fn the_gate_waits_for_the_dependency_then_for_the_probe() {
        let dep_slot: Arc<std::sync::Mutex<Option<ChildControl>>> = Arc::default();
        let serving = Arc::new(AtomicBool::new(false));
        let gate = Gate {
            dependency: {
                let s = Arc::clone(&dep_slot);
                Arc::new(move || s.lock().unwrap().clone())
            },
            serving: {
                let s = Arc::clone(&serving);
                Arc::new(move || s.load(Ordering::SeqCst))
            },
        };
        let r = ComponentRunner::start(sh("c", "exec cat"), Some(gate), fast());
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            r.status(),
            RunnerStatus::Waiting {
                reason: "dependency_down"
            }
        );
        assert!(r.child().is_none(), "no dependency: not spawned");
        // the dependency runs, the probe still says no
        let dep = spawn_child(sh("dep", "exec sleep 30"));
        *dep_slot.lock().unwrap() = Some(dep.control());
        assert!(
            status_is(&r, |s| s
                == RunnerStatus::Waiting {
                    reason: "not_serving"
                })
            .await
        );
        assert!(r.child().is_none(), "not serving: not spawned");
        serving.store(true, Ordering::SeqCst);
        assert!(status_is(&r, |s| matches!(s, RunnerStatus::Running { .. })).await);
        // the dependency stops: the component stops with it
        dep.stop_and_wait(LIMIT).await;
        assert!(
            status_is(&r, |s| s
                == RunnerStatus::Waiting {
                    reason: "dependency_down"
                })
            .await
        );
        assert!(r.child().is_none());
        r.shutdown().await;
    }

    #[tokio::test]
    async fn a_failed_post_spawn_check_stops_it_as_partitioned_and_it_retries() {
        let dep = spawn_child(sh("dep", "exec sleep 30"));
        // call 1 (gate) true, call 2 (post-spawn) false, later calls as `allow` says
        let calls = Arc::new(AtomicUsize::new(0));
        let allow = Arc::new(AtomicBool::new(false));
        let gate = Gate {
            dependency: {
                let d = dep.control();
                Arc::new(move || Some(d.clone()))
            },
            serving: {
                let c = Arc::clone(&calls);
                let a = Arc::clone(&allow);
                Arc::new(move || match c.fetch_add(1, Ordering::SeqCst) {
                    0 => true,
                    1 => false,
                    _ => a.load(Ordering::SeqCst),
                })
            },
        };
        let r = ComponentRunner::start(sh("c", "exec cat"), Some(gate), fast());
        assert!(
            status_is(&r, |s| s
                == RunnerStatus::AdapterDown {
                    reason: "partitioned"
                })
            .await
        );
        assert!(r.child().is_none(), "stopped");
        assert!(r.stdio().borrow().is_none(), "never forwarded");
        // while it does not serve, the report stays `partitioned` and nothing runs
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            r.status(),
            RunnerStatus::AdapterDown {
                reason: "partitioned"
            }
        );
        assert!(r.child().is_none());
        // it serves again: it comes back
        allow.store(true, Ordering::SeqCst);
        assert!(status_is(&r, |s| matches!(s, RunnerStatus::Running { .. })).await);
        r.shutdown().await;
        dep.stop_and_wait(LIMIT).await;
    }

    #[tokio::test]
    async fn every_respawn_is_checked_again() {
        let dep = spawn_child(sh("dep", "exec sleep 30"));
        let calls = Arc::new(AtomicUsize::new(0));
        let fail_from = Arc::new(AtomicUsize::new(usize::MAX));
        let gate = Gate {
            dependency: {
                let d = dep.control();
                Arc::new(move || Some(d.clone()))
            },
            serving: {
                let c = Arc::clone(&calls);
                let f = Arc::clone(&fail_from);
                Arc::new(move || c.fetch_add(1, Ordering::SeqCst) < f.load(Ordering::SeqCst))
            },
        };
        let r = ComponentRunner::start(sh("c", "exec cat"), Some(gate), fast());
        assert!(status_is(&r, |s| matches!(s, RunnerStatus::Running { .. })).await);
        let RunnerStatus::Running { generation } = r.status() else {
            unreachable!()
        };
        // from now on the probe fails; a supervisor respawn must be checked
        fail_from.store(calls.load(Ordering::SeqCst), Ordering::SeqCst);
        let child = r.child().unwrap();
        assert!(child.restart(LIMIT).await, "a respawn of the same child");
        assert!(
            status_is(&r, |s| s
                == RunnerStatus::AdapterDown {
                    reason: "partitioned"
                })
            .await,
            "the respawn (after generation {generation}) failed its check"
        );
        r.shutdown().await;
        dep.stop_and_wait(LIMIT).await;
    }

    #[tokio::test]
    async fn stop_before_holds_and_start_after_resumes() {
        let r = ComponentRunner::start(sh("c", "exec cat"), None, fast());
        assert!(status_is(&r, |s| matches!(s, RunnerStatus::Running { .. })).await);
        let pid = r.child().unwrap().pid().unwrap();
        r.stop_before().await;
        assert_eq!(r.status(), RunnerStatus::Held);
        assert!(r.child().is_none());
        assert!(
            !std::path::Path::new(&format!("/proc/{pid}")).exists(),
            "stopped and reaped"
        );
        r.start_after().await;
        assert!(status_is(&r, |s| matches!(s, RunnerStatus::Running { .. })).await);
        assert_ne!(r.child().unwrap().pid(), Some(pid));
        r.shutdown().await;
    }

    #[test]
    fn rnstatus_without_a_venv_is_not_serving() {
        let d = tempfile::tempdir().unwrap();
        assert!(!rnstatus_serving(d.path()));
    }

    #[test]
    fn rnstatus_reports_a_shared_instance() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("venvs/rns/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let write = |body: &str| {
            let p = bin.join("rnstatus");
            std::fs::write(&p, body).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        write("#!/bin/sh\necho ' Shared Instance[rnsd]'\necho '    Status    : Up'\n");
        assert!(rnstatus_serving(d.path()));
        write("#!/bin/sh\necho 'No shared RNS instance available to get status from'\nexit 1\n");
        assert!(!rnstatus_serving(d.path()));
    }
}
