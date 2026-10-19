//! Supervision tree: restart-in-place per child with exponential backoff + cap,
//! crash-loop detection (→ dashboard "degraded"), the Layer 9 seam.
//!
//! Timing decisions live in the pure `BackoffSchedule` (unit-tested without
//! processes); the supervisor task only does I/O.
//!
//! **Parent death.** Every child is started with `PR_SET_PDEATHSIG = SIGTERM`
//! (the `pre_exec` hook in [`set_parent_death_signal`]), so a `111d` that dies
//! without running its stop path (SIGKILL, OOM kill, abort) takes its children
//! with it instead of leaving them running unsupervised. The kernel sends that
//! signal when the *thread* that forked the child exits, not when the process
//! does. A tokio worker can retire while the daemon lives on: after
//! `block_in_place` its core moves to another thread, and an idle blocking-pool
//! thread exits. A child forked by such a thread would get a SIGTERM while
//! `111d` runs, and the supervisor would count it as a crash. So no supervisor
//! task forks: every child is forked by one dedicated thread
//! ([`spawn_on_forker`]) that never exits, and the signal comes only when the
//! process is gone. What it does not cover: a child's own children (the
//! kernel clears the flag in every fork, so a child that forks must arrange
//! its own), a child that catches or ignores SIGTERM (no SIGKILL follows
//! without a live parent), and a set-user-ID or file-capability program (the
//! kernel clears the flag at exec; none is supervised here). The service
//! manager's cgroup kill stays the backstop for those.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct BackoffPolicy {
    pub initial_ms: u64,
    pub max_ms: u64,
    pub factor: f64,
    pub jitter_pct: u32,
    /// Restarts within `crash_loop_window_secs` that mark the child Degraded.
    pub crash_loop_threshold: u32,
    pub crash_loop_window_secs: u64,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        BackoffPolicy {
            initial_ms: 250,
            max_ms: 60_000,
            factor: 2.0,
            jitter_pct: 20,
            crash_loop_threshold: 5,
            crash_loop_window_secs: 60,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChildSpec {
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub backoff: BackoffPolicy,
    /// When set, stdout+stderr append to this file (children whose logs are
    /// signal, e.g. rnsd peer discovery). Inherit otherwise.
    pub stdout_log: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildState {
    Running,
    Backoff,
    Degraded,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Sleep this long before the next spawn.
    Delay(Duration),
    /// Sustained crash-loop: stop restarting, surface degraded.
    Degraded,
}

/// Pure backoff/crash-loop logic (no I/O — fully unit-testable).
pub struct BackoffSchedule {
    policy: BackoffPolicy,
    restart_epochs: VecDeque<Instant>,
    consecutive: u32,
    rng: u64,
}

impl BackoffSchedule {
    pub fn new(policy: BackoffPolicy, rng_seed: u64) -> Self {
        BackoffSchedule {
            policy,
            restart_epochs: VecDeque::new(),
            consecutive: 0,
            rng: rng_seed | 1,
        }
    }

    /// Called when a child run lasted longer than the crash-loop window: the
    /// backoff resets (the child is considered stable again).
    pub fn reset(&mut self) {
        self.restart_epochs.clear();
        self.consecutive = 0;
    }

    fn rand01(&mut self) -> f64 {
        // xorshift64* — non-crypto jitter only.
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        ((self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D)) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Record that the child just exited at `now`; decide the next step.
    pub fn record_exit(&mut self, now: Instant) -> Decision {
        let window = Duration::from_secs(self.policy.crash_loop_window_secs);
        self.restart_epochs.push_back(now);
        while let Some(&t) = self.restart_epochs.front() {
            if now.duration_since(t) > window {
                self.restart_epochs.pop_front();
            } else {
                break;
            }
        }
        if self.restart_epochs.len() >= self.policy.crash_loop_threshold as usize {
            return Decision::Degraded;
        }
        let base = (self.policy.initial_ms as f64
            * self.policy.factor.powi(self.consecutive as i32))
        .min(self.policy.max_ms as f64);
        self.consecutive += 1;
        let spread = self.policy.jitter_pct as f64 / 100.0;
        let jittered = base * (1.0 - spread + 2.0 * spread * self.rand01());
        Decision::Delay(Duration::from_millis(jittered as u64))
    }
}

/// Pid + pending restart, under one lock so a restart request and the
/// supervisor's spawn/exit bookkeeping never interleave (see [`ChildControl::restart`]).
#[derive(Debug, Default)]
struct Shared {
    pid: Option<u32>,
    /// A restart was requested and not yet satisfied by a fresh spawn.
    restart: bool,
}

/// After SIGKILL, how long to wait for the supervisor to reap the child.
pub const KILL_WAIT: Duration = Duration::from_secs(2);

/// The cloneable control side of one supervised child: read its state and
/// pid, stop it, restart it. Obtained from [`ChildHandle::control`]; the
/// dashboard's per-service restart route holds these.
#[derive(Clone)]
pub struct ChildControl {
    name: String,
    state_rx: tokio::sync::watch::Receiver<ChildState>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    shared: Arc<Mutex<Shared>>,
    /// Wakes the supervisor out of a back-off sleep or the degraded wait.
    kick: Arc<tokio::sync::Notify>,
}

fn signal(pid: u32, sig: libc::c_int) {
    unsafe { libc::kill(pid as libc::pid_t, sig) };
}

/// The child side of the parent-death link (module doc, "Parent death"): the
/// child asks for SIGTERM when its forking thread (the forker, which lives as
/// long as `111d`) dies. Also refuses to exec when `111d` already died between
/// the fork and the `prctl`, since then the signal would never come.
fn set_parent_death_signal(cmd: &mut tokio::process::Command) {
    let parent = std::process::id() as libc::pid_t;
    // SAFETY: runs in the child between fork and exec, so it must be
    // async-signal-safe: it makes two syscalls (prctl, getppid) and builds its
    // error from a raw errno, without allocating or taking a lock.
    unsafe {
        cmd.pre_exec(move || {
            let sig = libc::SIGTERM as libc::c_ulong;
            if libc::prctl(
                libc::PR_SET_PDEATHSIG,
                sig,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
            ) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
}

/// One spawn handed to the forker thread: the command, the runtime its child
/// registers with (for `wait`), and where the result goes.
struct SpawnRequest {
    cmd: tokio::process::Command,
    rt: tokio::runtime::Handle,
    reply: tokio::sync::oneshot::Sender<std::io::Result<tokio::process::Child>>,
}

/// The forker thread's queue; the thread is started on first use (and again
/// only if it were ever gone).
static FORKER: Mutex<Option<std::sync::mpsc::Sender<SpawnRequest>>> = Mutex::new(None);

/// The forker thread: forks every supervised child and never returns while
/// `111d` runs (the queue's sender lives in [`FORKER`]), so the children's
/// parent-death signal fires only when the process dies.
fn forker_loop(rx: std::sync::mpsc::Receiver<SpawnRequest>) {
    for SpawnRequest { mut cmd, rt, reply } in rx {
        let _in_runtime = rt.enter();
        let spawned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cmd.spawn()))
            .unwrap_or_else(|_| Err(std::io::Error::other("spawning the child panicked")));
        if let Err(Ok(mut child)) = reply.send(spawned) {
            // the supervisor went away meanwhile (its runtime shut down): leave
            // no unsupervised child behind
            let _ = child.start_kill();
        }
    }
}

/// Spawn `cmd` on the forker thread (module doc, "Parent death") and wait for
/// the result. The child registers with the caller's runtime, exactly as a
/// `cmd.spawn()` here would.
async fn spawn_on_forker(cmd: tokio::process::Command) -> std::io::Result<tokio::process::Child> {
    let (reply, result) = tokio::sync::oneshot::channel();
    submit_to_forker(SpawnRequest {
        cmd,
        rt: tokio::runtime::Handle::current(),
        reply,
    })?;
    result.await.unwrap_or_else(|_| {
        Err(std::io::Error::other(
            "the child-forking thread dropped the spawn",
        ))
    })
}

/// Queue `req` for the forker thread, starting the thread on first use.
fn submit_to_forker(mut req: SpawnRequest) -> std::io::Result<()> {
    let mut slot = FORKER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(tx) = slot.as_ref() {
        match tx.send(req) {
            Ok(()) => return Ok(()),
            Err(std::sync::mpsc::SendError(back)) => {
                req = back;
                *slot = None;
            }
        }
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("111d-forker".into())
        .spawn(move || forker_loop(rx))?;
    tx.send(req)
        .map_err(|_| std::io::Error::other("the child-forking thread exited at start"))?;
    *slot = Some(tx);
    Ok(())
}

impl ChildControl {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn state(&self) -> ChildState {
        *self.state_rx.borrow()
    }

    /// Current child PID when running (None while backing off/stopped).
    pub fn pid(&self) -> Option<u32> {
        self.shared.lock().unwrap().pid
    }

    /// Ask the child to stop without waiting: SIGTERM when running, then the
    /// supervisor exits instead of restarting. Prefer [`Self::stop_and_wait`].
    pub fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        self.kick.notify_one();
        if let Some(pid) = self.pid() {
            signal(pid, libc::SIGTERM);
        }
    }

    /// Stop and join: SIGTERM, wait up to `grace` for the supervisor to reap
    /// the child and report [`ChildState::Stopped`], then SIGKILL and wait up
    /// to [`KILL_WAIT`] more. Returns once the supervisor is done (or gave up).
    pub async fn stop_and_wait(&self, grace: Duration) {
        self.stop();
        let mut rx = self.state_rx.clone();
        // `Err` from wait_for = the supervisor task is gone: also done.
        let stopped = |s: &ChildState| *s == ChildState::Stopped;
        if tokio::time::timeout(grace, rx.wait_for(stopped))
            .await
            .is_ok()
        {
            return;
        }
        if let Some(pid) = self.pid() {
            eprintln!(
                "111 supervise: {} (pid {pid}) ignored SIGTERM for {grace:?} — SIGKILL",
                self.name
            );
            signal(pid, libc::SIGKILL);
        }
        if tokio::time::timeout(KILL_WAIT, rx.wait_for(stopped))
            .await
            .is_err()
        {
            eprintln!("111 supervise: {} did not stop after SIGKILL", self.name);
        }
    }

    /// Restart in place: SIGTERM the running child, wait up to `grace` for it
    /// to exit, SIGKILL if it did not; the supervisor respawns it at once with
    /// a fresh back-off schedule. A child in back-off or `Degraded` (crash
    /// loop: the supervisor had stopped respawning) is respawned immediately.
    /// Returns `false` when the child is stopped for good (daemon shutdown);
    /// a restart already in flight is joined, not repeated.
    pub async fn restart(&self, grace: Duration) -> bool {
        if self.stop.load(std::sync::atomic::Ordering::SeqCst)
            || self.state() == ChildState::Stopped
        {
            return false;
        }
        let pid = {
            let mut sh = self.shared.lock().unwrap();
            if sh.restart {
                return true; // already pending
            }
            sh.restart = true;
            sh.pid
        };
        self.kick.notify_one();
        let Some(pid) = pid else {
            return true; // backing off / degraded: the kick respawns it
        };
        signal(pid, libc::SIGTERM);
        let deadline = Instant::now() + grace;
        while self.pid() == Some(pid) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if self.pid() == Some(pid) {
            eprintln!(
                "111 supervise: {} (pid {pid}) ignored SIGTERM for {grace:?} — SIGKILL",
                self.name
            );
            signal(pid, libc::SIGKILL);
            let deadline = Instant::now() + KILL_WAIT;
            while self.pid() == Some(pid) && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        true
    }

    /// Supervisor side: wait until `dur` elapses (`None` = forever) or a
    /// stop/restart request arrives. A stale wake-up (a permit left from a
    /// request that was already served) does not cut the wait short.
    async fn wait_for_request(&self, dur: Option<Duration>) {
        let deadline = dur.map(|d| tokio::time::Instant::now() + d);
        loop {
            if self.stop.load(std::sync::atomic::Ordering::SeqCst)
                || self.shared.lock().unwrap().restart
            {
                return;
            }
            match deadline {
                Some(dl) => {
                    if tokio::time::timeout_at(dl, self.kick.notified())
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                None => self.kick.notified().await,
            }
        }
    }
}

/// Owner of one supervised child: the [`ChildControl`] (via `Deref`) plus the
/// supervisor task.
pub struct ChildHandle {
    ctl: ChildControl,
    #[allow(dead_code)] // task kept alive by the handle
    task: tokio::task::JoinHandle<()>,
}

impl std::ops::Deref for ChildHandle {
    type Target = ChildControl;
    fn deref(&self) -> &ChildControl {
        &self.ctl
    }
}

impl ChildHandle {
    /// A cloneable control for this child (restart route, joins).
    pub fn control(&self) -> ChildControl {
        self.ctl.clone()
    }
}

/// Spawn a supervised child: restart-in-place, backoff per policy, sustained
/// crash-loop ⇒ Degraded (surfaced by the dashboard, not fatal to the daemon;
/// the supervisor then waits for a [`ChildControl::restart`] or a stop).
/// Each run is forked by the forker thread with the parent-death signal set
/// (module doc). Must be called within a tokio runtime.
pub fn spawn_child(spec: ChildSpec) -> ChildHandle {
    let (state_tx, state_rx) = tokio::sync::watch::channel(ChildState::Backoff);
    let ctl = ChildControl {
        name: spec.name.clone(),
        state_rx,
        stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        shared: Arc::new(Mutex::new(Shared::default())),
        kick: Arc::new(tokio::sync::Notify::new()),
    };

    let task = {
        let ctl = ctl.clone();
        let mut schedule = BackoffSchedule::new(
            spec.backoff.clone(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as u64 ^ (std::process::id() as u64) << 32)
                .unwrap_or(0x9e37_79b9),
        );
        tokio::spawn(async move {
            let stopping = || ctl.stop.load(std::sync::atomic::Ordering::SeqCst);
            let window = Duration::from_secs(spec.backoff.crash_loop_window_secs);
            let mut last_spawn: Option<Instant> = None;
            'run: loop {
                if stopping() {
                    break;
                }
                // Stable run since last spawn? reset backoff.
                if last_spawn.is_some_and(|t| t.elapsed() > window) {
                    schedule.reset();
                }
                let spawn_result = {
                    let mut cmd = tokio::process::Command::new(&spec.program);
                    cmd.args(&spec.args)
                        .envs(spec.env.iter().map(|(k, v)| (k, v)))
                        .stdin(std::process::Stdio::null());
                    match &spec.stdout_log {
                        Some(path) => match std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(path)
                        {
                            Ok(f) => {
                                let err_f = f.try_clone();
                                match err_f {
                                    Ok(e2) => {
                                        cmd.stdout(std::process::Stdio::from(f));
                                        cmd.stderr(std::process::Stdio::from(e2));
                                    }
                                    Err(_) => {
                                        cmd.stdout(std::process::Stdio::from(f));
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "111 supervise: open {} failed: {e} — inheriting",
                                    path.display()
                                );
                                cmd.stdout(std::process::Stdio::inherit())
                                    .stderr(std::process::Stdio::inherit());
                            }
                        },
                        None => {
                            cmd.stdout(std::process::Stdio::inherit())
                                .stderr(std::process::Stdio::inherit());
                        }
                    }
                    set_parent_death_signal(&mut cmd);
                    spawn_on_forker(cmd).await
                };
                let mut child = match spawn_result {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!(
                            "111 supervise: spawn {} failed: {e}",
                            spec.program.display()
                        );
                        if std::mem::take(&mut ctl.shared.lock().unwrap().restart) {
                            schedule.reset(); // a requested restart gets a fresh schedule
                        }
                        match schedule.record_exit(Instant::now()) {
                            Decision::Delay(d) => {
                                let _ = state_tx.send(ChildState::Backoff);
                                ctl.wait_for_request(Some(d)).await;
                                continue;
                            }
                            Decision::Degraded => {
                                let _ = state_tx.send(ChildState::Degraded);
                                ctl.wait_for_request(None).await;
                                continue;
                            }
                        }
                    }
                };
                last_spawn = Some(Instant::now());
                let pid = child.id();
                {
                    let mut sh = ctl.shared.lock().unwrap();
                    sh.pid = pid;
                    // a restart requested before this spawn is satisfied by it
                    if std::mem::take(&mut sh.restart) {
                        schedule.reset();
                    }
                }
                let _ = state_tx.send(ChildState::Running);
                if stopping() {
                    // stop raced the spawn (it saw no pid to signal)
                    if let Some(pid) = pid {
                        signal(pid, libc::SIGTERM);
                    }
                }

                let _ = child.wait().await;
                let restarting = {
                    let mut sh = ctl.shared.lock().unwrap();
                    sh.pid = None;
                    std::mem::take(&mut sh.restart)
                };

                if stopping() {
                    break;
                }
                if restarting {
                    // requested restart: respawn now, fresh back-off
                    schedule.reset();
                    continue;
                }
                match schedule.record_exit(Instant::now()) {
                    Decision::Delay(d) => {
                        let _ = state_tx.send(ChildState::Backoff);
                        ctl.wait_for_request(Some(d)).await;
                    }
                    Decision::Degraded => {
                        let _ = state_tx.send(ChildState::Degraded);
                        // crash loop: no more respawns until a restart request
                        loop {
                            ctl.wait_for_request(None).await;
                            if stopping() {
                                break 'run;
                            }
                            if ctl.shared.lock().unwrap().restart {
                                break;
                            }
                        }
                    }
                }
            }
            let _ = state_tx.send(ChildState::Stopped);
        })
    };

    ChildHandle { ctl, task }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(initial_ms: u64) -> BackoffPolicy {
        BackoffPolicy {
            initial_ms,
            max_ms: 2_000,
            factor: 2.0,
            jitter_pct: 0, // deterministic in tests
            crash_loop_threshold: 5,
            crash_loop_window_secs: 60,
        }
    }

    #[test]
    fn backoff_grows_and_caps() {
        let mut s = BackoffSchedule::new(policy(100), 42);
        let t0 = Instant::now();
        let mut delays = Vec::new();
        for i in 0..4 {
            match s.record_exit(t0 + Duration::from_millis(i * 10)) {
                Decision::Delay(d) => delays.push(d.as_millis()),
                Decision::Degraded => panic!("not degraded yet"),
            }
        }
        assert_eq!(delays, vec![100, 200, 400, 800]);
    }

    #[test]
    fn crash_loop_degrades_at_threshold() {
        let mut s = BackoffSchedule::new(policy(100), 42);
        let t0 = Instant::now();
        let mut degraded_at = None;
        for i in 0..10u64 {
            if let Decision::Degraded = s.record_exit(t0 + Duration::from_millis(i * 10)) {
                degraded_at = Some(i);
                break;
            }
        }
        assert_eq!(degraded_at, Some(4), "5th exit (index 4) degrades");
    }

    #[test]
    fn spread_out_restarts_never_degrade() {
        let mut s = BackoffSchedule::new(policy(100), 42);
        let t0 = Instant::now();
        for i in 0..20u64 {
            assert!(matches!(
                s.record_exit(t0 + Duration::from_secs(i * 61)),
                Decision::Delay(_)
            ));
        }
    }

    #[test]
    fn reset_restores_initial_delay() {
        let mut s = BackoffSchedule::new(policy(100), 42);
        let t0 = Instant::now();
        let _ = s.record_exit(t0);
        let _ = s.record_exit(t0 + Duration::from_millis(10));
        s.reset();
        match s.record_exit(t0 + Duration::from_millis(20)) {
            Decision::Delay(d) => assert_eq!(d.as_millis(), 100),
            Decision::Degraded => panic!("not degraded"),
        }
    }

    #[tokio::test]
    async fn supervisor_restarts_and_degrades() {
        let spec = ChildSpec {
            name: "crasher".into(),
            program: PathBuf::from("/bin/false"),
            args: vec![],
            env: vec![],
            backoff: policy(20),
            stdout_log: None,
        };
        let h = spawn_child(spec);
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(h.state(), ChildState::Degraded);
    }

    #[tokio::test]
    async fn supervisor_runs_and_stops() {
        let spec = ChildSpec {
            name: "sleeper".into(),
            program: PathBuf::from("/bin/sleep"),
            args: vec!["30".into()],
            env: vec![],
            backoff: policy(20),
            stdout_log: None,
        };
        let h = spawn_child(spec);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(h.state(), ChildState::Running);
        assert!(h.pid().is_some());
        h.stop();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(h.state(), ChildState::Stopped);
    }

    fn sh(script: &str, args: &[&str]) -> ChildSpec {
        let mut a = vec!["-c".to_string(), script.to_string(), "sh".to_string()];
        a.extend(args.iter().map(|s| s.to_string()));
        ChildSpec {
            name: "sh-test".into(),
            program: PathBuf::from("/bin/sh"),
            args: a,
            env: vec![],
            backoff: BackoffPolicy {
                crash_loop_threshold: 3,
                ..policy(20)
            },
            stdout_log: None,
        }
    }

    async fn until(limit: Duration, f: impl Fn() -> bool) -> bool {
        let t0 = Instant::now();
        while t0.elapsed() < limit {
            if f() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        f()
    }

    fn alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    /// Running, not a zombie: /proc/<pid>/stat's state field (after the
    /// parenthesised comm) is neither `Z` nor `X`.
    fn running(pid: u32) -> bool {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        let state = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.trim_start().chars().next());
        !matches!(state, None | Some('Z') | Some('X'))
    }

    /// PR_SET_PDEATHSIG fires when the *thread* that forked the child exits.
    /// A runtime on a thread that then ends stands in for a retired tokio
    /// worker: the child must not die with it, because the forker thread, not
    /// the caller's thread, forked it (module doc, "Parent death").
    #[test]
    fn a_child_outlives_the_thread_whose_runtime_started_it() {
        let pid = std::thread::spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let h = spawn_child(sh("exec sleep 30", &[]));
                assert!(until(Duration::from_secs(2), || h.pid().is_some()).await);
                h.pid().unwrap()
            })
        })
        .join()
        .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let survived = running(pid);
        signal(pid, libc::SIGKILL);
        unsafe { libc::waitpid(pid as libc::pid_t, std::ptr::null_mut(), 0) };
        assert!(survived, "the child died with the thread that asked for it");
    }

    #[tokio::test]
    async fn stop_and_wait_joins_a_cooperative_child() {
        let h = spawn_child(sh("exec sleep 30", &[]));
        assert!(until(Duration::from_secs(2), || h.pid().is_some()).await);
        let pid = h.pid().unwrap();
        let t0 = Instant::now();
        h.stop_and_wait(Duration::from_secs(5)).await;
        assert!(t0.elapsed() < Duration::from_secs(2), "{:?}", t0.elapsed());
        assert_eq!(h.state(), ChildState::Stopped);
        assert_eq!(h.pid(), None);
        assert!(!alive(pid), "reaped");
    }

    #[tokio::test]
    async fn stop_and_wait_kills_a_child_that_ignores_sigterm() {
        let h = spawn_child(sh("trap '' TERM; while :; do sleep 0.05; done", &[]));
        assert!(until(Duration::from_secs(2), || h.pid().is_some()).await);
        tokio::time::sleep(Duration::from_millis(100)).await; // trap installed
        let pid = h.pid().unwrap();
        let t0 = Instant::now();
        h.stop_and_wait(Duration::from_millis(300)).await;
        let took = t0.elapsed();
        assert!(
            took >= Duration::from_millis(300),
            "waited the grace: {took:?}"
        );
        assert!(took < Duration::from_secs(3), "{took:?}");
        assert_eq!(h.state(), ChildState::Stopped);
        assert!(!alive(pid), "SIGKILLed and reaped");
    }

    #[tokio::test]
    async fn stop_and_wait_ends_a_degraded_supervisor() {
        let h = spawn_child(sh("exit 1", &[]));
        assert!(until(Duration::from_secs(3), || h.state() == ChildState::Degraded).await);
        h.stop_and_wait(Duration::from_secs(5)).await;
        assert_eq!(h.state(), ChildState::Stopped);
        assert!(
            !h.control().restart(Duration::from_secs(1)).await,
            "stopped for good"
        );
    }

    #[tokio::test]
    async fn restart_respawns_now_with_a_new_pid() {
        let h = spawn_child(sh("exec sleep 30", &[]));
        assert!(until(Duration::from_secs(2), || h.pid().is_some()).await);
        let old = h.pid().unwrap();
        let ctl = h.control();
        let t0 = Instant::now();
        assert!(ctl.restart(Duration::from_secs(5)).await);
        assert!(
            until(Duration::from_secs(2), || h.pid().is_some_and(|p| p != old)).await,
            "respawned"
        );
        // no back-off sleep in between (policy initial is 20 ms; well under 1 s)
        assert!(t0.elapsed() < Duration::from_secs(1), "{:?}", t0.elapsed());
        assert_eq!(h.state(), ChildState::Running);
        assert!(!alive(old));
        h.stop_and_wait(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn restart_takes_a_term_ignoring_child_down_with_sigkill() {
        let h = spawn_child(sh("trap '' TERM; while :; do sleep 0.05; done", &[]));
        assert!(until(Duration::from_secs(2), || h.pid().is_some()).await);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let old = h.pid().unwrap();
        assert!(h.control().restart(Duration::from_millis(300)).await);
        assert!(until(Duration::from_secs(2), || h.pid().is_some_and(|p| p != old)).await);
        assert!(!alive(old));
        h.stop_and_wait(Duration::from_millis(300)).await;
    }

    #[tokio::test]
    async fn restart_clears_a_crash_loop() {
        let dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("ok");
        let m = mark.to_str().unwrap();
        // crashes until the marker exists, then runs
        let h = spawn_child(sh("test -f \"$1\" || exit 1; exec sleep 30", &[m]));
        assert!(until(Duration::from_secs(3), || h.state() == ChildState::Degraded).await);
        // degraded means the supervisor stopped respawning: it stays down
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(h.state(), ChildState::Degraded);
        assert_eq!(h.pid(), None);
        std::fs::write(&mark, "").unwrap();
        assert!(h.control().restart(Duration::from_secs(1)).await);
        assert!(
            until(Duration::from_secs(2), || h.state() == ChildState::Running).await,
            "fresh start out of degraded"
        );
        assert!(h.pid().is_some());
        h.stop_and_wait(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn spawn_failure_backs_off_not_panics() {
        let spec = ChildSpec {
            name: "ghost".into(),
            program: PathBuf::from("/definitely/not/a/real/binary-111"),
            args: vec![],
            env: vec![],
            backoff: BackoffPolicy {
                crash_loop_threshold: 3,
                ..policy(20)
            },
            stdout_log: None,
        };
        let h = spawn_child(spec);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(h.state(), ChildState::Degraded);
    }
}
