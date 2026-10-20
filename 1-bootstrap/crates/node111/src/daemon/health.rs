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
    let transport_ok = matches!(
        c.transport,
        TransportHealth::Alive | TransportHealth::DeliberatelyAbsent
    );
    if c.identity && c.kb && c.updater_idle && transport_ok {
        HealthStatus::Ok
    } else {
        HealthStatus::Error
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn green_matrix() {
        let green = Components {
            identity: true,
            kb: true,
            transport: TransportHealth::Alive,
            updater_idle: true,
        };
        assert_eq!(evaluate(&green), HealthStatus::Ok);

        let absent = Components {
            transport: TransportHealth::DeliberatelyAbsent,
            ..green
        };
        assert_eq!(evaluate(&absent), HealthStatus::Ok);

        for transport in [TransportHealth::Degraded, TransportHealth::Down] {
            assert_eq!(
                evaluate(&Components { transport, ..green }),
                HealthStatus::Error
            );
        }
        assert_eq!(
            evaluate(&Components {
                identity: false,
                ..green
            }),
            HealthStatus::Error
        );
        assert_eq!(
            evaluate(&Components { kb: false, ..green }),
            HealthStatus::Error
        );
        assert_eq!(
            evaluate(&Components {
                updater_idle: false,
                ..green
            }),
            HealthStatus::Error
        );
    }
}
