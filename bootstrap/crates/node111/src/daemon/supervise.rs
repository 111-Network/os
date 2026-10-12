//! Supervision tree: restart-in-place per child with exponential backoff + cap,
//! crash-loop detection (→ dashboard "degraded"), the Layer 9 seam.
//!
//! Timing decisions live in the pure `BackoffSchedule` (unit-tested without
//! processes); the supervisor task only does I/O.

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

pub struct ChildHandle {
    state_rx: tokio::sync::watch::Receiver<ChildState>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    pid: Arc<Mutex<Option<u32>>>,
    #[allow(dead_code)] // task kept alive by the handle
    task: tokio::task::JoinHandle<()>,
}

impl ChildHandle {
    pub fn state(&self) -> ChildState {
        *self.state_rx.borrow()
    }

    /// Current child PID when running (None while backing off/stopped).
    pub fn pid(&self) -> Option<u32> {
        *self.pid.lock().unwrap()
    }

    /// Ask the child to stop: SIGTERM when running, then the supervisor exits
    /// instead of restarting.
    pub fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(pid) = *self.pid.lock().unwrap() {
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        }
    }
}

/// Spawn a supervised child: restart-in-place, backoff per policy, sustained
/// crash-loop ⇒ Degraded (surfaced by the dashboard, not fatal to the daemon).
/// Must be called within a tokio runtime.
pub fn spawn_child(spec: ChildSpec) -> ChildHandle {
    let (state_tx, state_rx) = tokio::sync::watch::channel(ChildState::Backoff);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let pid: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));

    let task = {
        let stop = Arc::clone(&stop);
        let pid = Arc::clone(&pid);
        let mut schedule = BackoffSchedule::new(
            spec.backoff.clone(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as u64 ^ (std::process::id() as u64) << 32)
                .unwrap_or(0x9e37_79b9),
        );
        tokio::spawn(async move {
            let window = Duration::from_secs(spec.backoff.crash_loop_window_secs);
            let mut last_spawn: Option<Instant> = None;
            loop {
                if stop.load(std::sync::atomic::Ordering::SeqCst) {
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
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::inherit())
                        .stderr(std::process::Stdio::inherit());
                    cmd.spawn()
                };
                let mut child = match spawn_result {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!(
                            "111 supervise: spawn {} failed: {e}",
                            spec.program.display()
                        );
                        match schedule.record_exit(Instant::now()) {
                            Decision::Delay(d) => {
                                let _ = state_tx.send(ChildState::Backoff);
                                tokio::time::sleep(d).await;
                                continue;
                            }
                            Decision::Degraded => {
                                let _ = state_tx.send(ChildState::Degraded);
                                return;
                            }
                        }
                    }
                };
                last_spawn = Some(Instant::now());
                *pid.lock().unwrap() = child.id();
                let _ = state_tx.send(ChildState::Running);

                let _ = child.wait().await;
                *pid.lock().unwrap() = None;

                if stop.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                match schedule.record_exit(Instant::now()) {
                    Decision::Delay(d) => {
                        let _ = state_tx.send(ChildState::Backoff);
                        tokio::time::sleep(d).await;
                    }
                    Decision::Degraded => {
                        let _ = state_tx.send(ChildState::Degraded);
                        return;
                    }
                }
            }
            let _ = state_tx.send(ChildState::Stopped);
        })
    };

    ChildHandle {
        state_rx,
        stop,
        pid,
        task,
    }
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
        };
        let h = spawn_child(spec);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(h.state(), ChildState::Running);
        assert!(h.pid().is_some());
        h.stop();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(h.state(), ChildState::Stopped);
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
        };
        let h = spawn_child(spec);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(h.state(), ChildState::Degraded);
    }
}
