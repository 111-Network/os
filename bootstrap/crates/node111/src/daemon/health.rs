//! Health model: `/health` gates the dashboard/API, never the watchdog heartbeat.
//! Green = identity loaded · KB resolves · transport alive-or-deliberately-absent ·
//! updater idle. Probation requires this checklist sustained across the window.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportHealth {
    Alive,
    DeliberatelyAbsent,
    Degraded,
    Down,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Components {
    pub identity: bool,
    pub kb: bool,
    pub transport: TransportHealth,
    pub updater_idle: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Ok,
    Error,
}

pub fn evaluate(c: &Components) -> HealthStatus {
    let _ = c;
    todo!("T7")
}
