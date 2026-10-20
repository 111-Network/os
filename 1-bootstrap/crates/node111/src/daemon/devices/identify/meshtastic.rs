//! Identify step 2: Meshtastic (`os/6-transport/README.md` §12: "meshmud
//! with the identify profile, as built"; Layer 5 §14).
//!
//! The step's child mechanics stay in `meshtastic::identify`
//! (`Meshtastic::identify_step`: meshmud started on the board port with the
//! identify profile, the presence reader's `--once` through the downlink,
//! the reader's exit code 3 = "nothing Meshtastic answered"). This adapter
//! maps that onto the sequence:
//!
//! - a Meshtastic answer ⇒ `Match` — the binding and the firmware are
//!   already persisted, the board-port and downlink claims were
//!   **transferred** to `meshmud` and the identify child is the service
//!   child (the orchestrator's release of the board port then finds it
//!   handed over and leaves it);
//! - exit code 3 ⇒ `NoMatch` (no longer persisted as `unknown` by itself);
//! - meshmud or the reader missing ⇒ `Error` (a missing tool);
//! - the downlink held by another client, meshmud failing, the reader
//!   failing or timing out ⇒ `Error`;
//! - the device forgotten during the probe ⇒ the whole sequence aborts
//!   (`404`); the daemon shutting down ⇒ it stops.
//!
//! The step's timeouts are the module's own (`downlink_wait`,
//! `identify_timeout`): this future is never dropped half-way, because the
//! module tears its child down on every path itself.

use super::{Abort, BoxFut, Outcome, Probe, Step, StepKind, UplinkKind};
use crate::daemon::devices::{DeviceFirmware, FirmwareFamily, FirmwareSource};
use crate::daemon::meshtastic::identify::{IdentifyError, StepResult};
use crate::daemon::meshtastic::Meshtastic;
use std::sync::Arc;

/// The Meshtastic step (module doc).
pub struct MeshtasticStep {
    m: Option<Arc<Meshtastic>>,
}

impl MeshtasticStep {
    /// `None` = no Meshtastic module on this node (the step answers "not
    /// installed").
    pub fn new(m: Option<Arc<Meshtastic>>) -> Self {
        MeshtasticStep { m }
    }
}

impl Step for MeshtasticStep {
    fn kind(&self) -> StepKind {
        StepKind::Meshtastic
    }

    fn tool(&self) -> &'static str {
        "meshmud and the Meshtastic presence reader"
    }

    fn available(&self, kind: UplinkKind) -> bool {
        self.m
            .as_ref()
            .is_some_and(|m| m.identify_available(kind == UplinkKind::Serial))
    }

    fn run<'a>(&'a self, probe: &'a Probe) -> BoxFut<'a, Result<Outcome, Abort>> {
        Box::pin(async move {
            let Some(m) = &self.m else {
                return Ok(Outcome::Error(format!("{} is not installed", self.tool())));
            };
            match m
                .identify_step(&probe.device_id, &probe.uplink, &probe.task)
                .await
            {
                Ok(StepResult::Bound(rec)) => {
                    Ok(Outcome::Match(rec.firmware.unwrap_or(DeviceFirmware {
                        family: FirmwareFamily::Meshtastic,
                        version: None,
                        source: FirmwareSource::Probe,
                    })))
                }
                Ok(StepResult::NotMeshtastic) => Ok(Outcome::NoMatch),
                Err(IdentifyError::NotFound) => Err(Abort::NotFound),
                Err(IdentifyError::Unavailable) => {
                    Ok(Outcome::Error(format!("{} is not installed", self.tool())))
                }
                Err(IdentifyError::Busy(b)) => Ok(Outcome::Error(b.to_string())),
                Err(IdentifyError::NotOffered(why)) => Ok(Outcome::Error(why.to_string())),
                Err(IdentifyError::Upstream(e)) if m.is_stopping() => Err(Abort::Stopped(e)),
                Err(IdentifyError::Upstream(e)) => Ok(Outcome::Error(e)),
            }
        })
    }
}
