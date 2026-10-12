//! Update client: fetch → verify (minisig + floor) → stage into `state/staging/` →
//! drop `state/update-request.json`. The daemon CANNOT write `releases/` or flip
//! `current` — the privileged root helper is the only flip actor (D12).

pub mod fetch;
pub mod stage;

use crate::daemon::dashboard::SharedState;
use crate::daemon::watchdog;
use crate::daemon::DaemonLayout;
use std::time::Duration;

/// The minisign verify key compiled into the binary (keys/manifest-pub.key —
/// DEV key in this tree; release machinery substitutes the real one, T15).
pub const MANIFEST_PUBKEY: &str = include_str!("../../../../../keys/manifest-pub.key");

/// Poll interval and failure backoff (exp + jitter, capped).
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(60 * 60); // 1h
pub const MAX_BACKOFF: Duration = Duration::from_secs(24 * 60 * 60); // 24h cap

/// Updater loop. Owns the dashboard's update fields (last-successful-check,
/// pending, clock-fault). Runs until the daemon exits; never panics on
/// transient failures.
pub async fn run(layout: DaemonLayout, state: SharedState, cfg: fetch::UpdaterConfig) {
    let mut backoff = Duration::from_secs(60);
    loop {
        // Extend the watchdog during the fetch window; restore right after.
        let _ = watchdog::extend(10 * 60 * 1_000_000);
        let outcome = tokio::task::spawn_blocking({
            let layout = layout.clone();
            let cfg = cfg.clone();
            move || fetch::check_once(&cfg, &layout)
        })
        .await;
        let _ = watchdog::restore(30 * 1_000_000);

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

        tokio::time::sleep(sleep_for).await;
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
