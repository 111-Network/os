//! systemd watchdog: a dedicated task heartbeats `WATCHDOG=1` unconditionally at
//! ≤ half of WatchdogSec (30s ⇒ ≤15s), independent of health. Update windows extend
//! the watchdog via `WATCHDOG_USEC=` and restore after READY=1.

/// Start the unconditional heartbeat task. No-op when NOTIFY_SOCKET is unset
/// (dev runs, Termux).
pub fn start_heartbeat() -> tokio::task::JoinHandle<()> {
    todo!("T7")
}

/// Temporarily extend the watchdog deadline (update windows). Restore with
/// `restore()`. Must never pause the heartbeat itself.
pub fn extend(usec: u64) -> anyhow::Result<()> {
    let _ = usec;
    todo!("T7")
}

pub fn restore(default_usec: u64) -> anyhow::Result<()> {
    let _ = default_usec;
    todo!("T7")
}
