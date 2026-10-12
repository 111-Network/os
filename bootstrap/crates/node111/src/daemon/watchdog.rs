//! systemd watchdog: a dedicated task heartbeats `WATCHDOG=1` unconditionally at
//! ≤ half of WatchdogSec (30s ⇒ ≤15s), independent of health. Update windows extend
//! the watchdog via `WATCHDOG_USEC=` and restore after READY=1.
//!
//! Health gates the dashboard/API — NEVER the heartbeat (spec §Supervision:
//! the watchdog exists to kill a hung daemon, and a daemon in trouble must not
//! be able to silence its own killer).

use std::time::Duration;

/// Default production interval: WatchdogSec=30 ⇒ heartbeat every 15s.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(15);

fn sd_notify(state: sd_notify::NotifyState<'_>) -> bool {
    sd_notify::notify(false, &[state]).is_ok()
}

fn send(msg: &str) {
    // Direct datagram for messages the sd-notify crate doesn't model
    // (WATCHDOG_USEC). Best-effort: unset NOTIFY_SOCKET ⇒ no-op.
    if let Ok(path) = std::env::var("NOTIFY_SOCKET") {
        if let Ok(sock) = std::os::unix::net::UnixDatagram::unbound() {
            let _ = sock.send_to(msg.as_bytes(), path);
        }
    }
}

/// Start the unconditional heartbeat task (production interval).
pub fn start_heartbeat() -> tokio::task::JoinHandle<()> {
    start_heartbeat_with(DEFAULT_INTERVAL)
}

/// Interval-injectable heartbeat. No-op (but still returns a parked task) when
/// NOTIFY_SOCKET is unset — dev runs and Termux have no systemd notify socket.
pub fn start_heartbeat_with(interval: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if std::env::var("NOTIFY_SOCKET").is_err() {
            std::future::pending::<()>().await;
            return;
        }
        let mut tick = tokio::time::interval(interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let _ = sd_notify(sd_notify::NotifyState::Watchdog);
        }
    })
}

/// Temporarily extend the watchdog deadline (update windows). The heartbeat
/// itself never pauses. `usec` is the new total budget in microseconds.
pub fn extend(usec: u64) -> anyhow::Result<()> {
    send(&format!("WATCHDOG_USEC={usec}"));
    Ok(())
}

/// Restore the default budget after the update window (READY=1 follows).
pub fn restore(default_usec: u64) -> anyhow::Result<()> {
    send(&format!("WATCHDOG_USEC={default_usec}"));
    Ok(())
}

/// READY=1 (dashboard serving; components may still be warming — /health
/// reflects those).
pub fn ready() {
    let _ = sd_notify(sd_notify::NotifyState::Ready);
}
