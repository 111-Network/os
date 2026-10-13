//! Update client: fetch → verify (minisig + floor) → stage into `state/staging/` →
//! drop `state/update-request.json`. The daemon CANNOT write `releases/` or flip
//! `current` — the privileged root helper is the only flip actor (D12).

pub mod fetch;
pub mod stage;

use crate::daemon::dashboard::SharedState;
use crate::daemon::watchdog;
use crate::daemon::DaemonLayout;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

/// The minisign verify key compiled into the binary (keys/manifest-pub.key —
/// DEV key in this tree; release machinery substitutes the real one, T15).
pub const MANIFEST_PUBKEY: &str = include_str!("../../../../../keys/manifest-pub.key");

/// Poll interval and failure backoff (exp + jitter, capped).
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(60 * 60); // 1h
pub const MAX_BACKOFF: Duration = Duration::from_secs(24 * 60 * 60); // 24h cap

/// Result of the last check this daemon lifetime (`/api/v1/status`
/// `update.last_result`; Layer 4 R2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LastResult {
    UpToDate,
    Staged,
    Refused,
    /// Fetch/clock problems and check errors.
    Degraded,
}

/// Live updater facts the dashboard reads (Layer 4 R2): whether a check is
/// running right now, the last outcome, and a completed-check counter.
#[derive(Debug, Default)]
pub struct UpdaterStatus {
    checking: AtomicBool,
    last_result: Mutex<Option<LastResult>>,
    checks: AtomicU64,
}

impl UpdaterStatus {
    pub fn checking(&self) -> bool {
        self.checking.load(Ordering::SeqCst)
    }

    /// Mark a check as running; returns the previous value. `POST
    /// /api/v1/update/check` uses it so a request during a running check
    /// coalesces with it instead of queueing a second one.
    pub fn begin_check(&self) -> bool {
        self.checking.swap(true, Ordering::SeqCst)
    }

    pub fn last_result(&self) -> Option<LastResult> {
        *self.last_result.lock().unwrap()
    }

    /// Checks completed this lifetime.
    pub fn checks(&self) -> u64 {
        self.checks.load(Ordering::SeqCst)
    }

    /// Record a finished check (the loop calls it; public so tests can set
    /// an outcome without a network round).
    pub fn finish(&self, result: LastResult) {
        *self.last_result.lock().unwrap() = Some(result);
        self.checks.fetch_add(1, Ordering::SeqCst);
        self.checking.store(false, Ordering::SeqCst);
    }
}

/// Dashboard hooks into the loop: `wake` (= `ApiState::update_check`) cuts the
/// current sleep short; `status` is what `/api/v1/status.update` reads.
#[derive(Clone, Default)]
pub struct Hooks {
    pub wake: Arc<Notify>,
    pub status: Arc<UpdaterStatus>,
}

/// Updater loop. Owns the dashboard's update fields (last-successful-check,
/// pending, clock-fault). Runs until the daemon exits; never panics on
/// transient failures.
pub async fn run(layout: DaemonLayout, state: SharedState, cfg: fetch::UpdaterConfig) {
    run_with_hooks(layout, state, cfg, Hooks::default()).await
}

/// [`run`] with the dashboard hooks: sleeps between checks end early when
/// `hooks.wake` is notified (`POST /api/v1/update/check`).
pub async fn run_with_hooks(
    layout: DaemonLayout,
    state: SharedState,
    cfg: fetch::UpdaterConfig,
    hooks: Hooks,
) {
    let mut backoff = Duration::from_secs(60);
    loop {
        hooks.status.begin_check();
        // Extend the watchdog during the fetch window; restore right after.
        let _ = watchdog::extend(10 * 60 * 1_000_000);
        let outcome = tokio::task::spawn_blocking({
            let layout = layout.clone();
            let cfg = cfg.clone();
            move || fetch::check_once(&cfg, &layout)
        })
        .await;
        let _ = watchdog::restore(30 * 1_000_000);
        hooks.status.finish(match &outcome {
            Ok(Ok(fetch::FetchOutcome::UpToDate)) => LastResult::UpToDate,
            Ok(Ok(fetch::FetchOutcome::Staged { .. })) => LastResult::Staged,
            Ok(Ok(fetch::FetchOutcome::Refused { .. })) => LastResult::Refused,
            _ => LastResult::Degraded,
        });

        let sleep_for = match outcome {
            Ok(Ok(fetch::FetchOutcome::UpToDate)) => {
                state.write().unwrap().clock_fault = false;
                backoff = Duration::from_secs(60);
                DEFAULT_INTERVAL
            }
            Ok(Ok(fetch::FetchOutcome::Staged { version })) => {
                eprintln!("111 updater: staged {version}, root helper notified");
                state.write().unwrap().update_pending = true;
                backoff = Duration::from_secs(60);
                DEFAULT_INTERVAL
            }
            Ok(Ok(fetch::FetchOutcome::Refused { reason })) => {
                eprintln!("111 updater: update refused: {reason}");
                backoff = jittered(backoff.saturating_mul(2).min(MAX_BACKOFF));
                backoff
            }
            Ok(Ok(fetch::FetchOutcome::Degraded {
                reason,
                clock_fault,
            })) => {
                eprintln!("111 updater: degraded: {reason}");
                state.write().unwrap().clock_fault = clock_fault;
                backoff = jittered(backoff.saturating_mul(2).min(MAX_BACKOFF));
                backoff
            }
            Ok(Err(e)) => {
                eprintln!("111 updater: check failed: {e:#}");
                backoff = jittered(backoff.saturating_mul(2).min(MAX_BACKOFF));
                backoff
            }
            Err(join_err) => {
                eprintln!("111 updater: task join error: {join_err}");
                jittered(Duration::from_secs(300))
            }
        };

        tokio::select! {
            _ = tokio::time::sleep(sleep_for) => {}
            _ = hooks.wake.notified() => {
                eprintln!("111 updater: check requested from the dashboard");
            }
        }
    }
}

fn jittered(d: Duration) -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.subsec_nanos() as u64)
        .unwrap_or(0);
    let pct = 10 + (nanos % 21); // +10–30% jitter
    d.saturating_mul(100 + pct as u32) / 100
}
