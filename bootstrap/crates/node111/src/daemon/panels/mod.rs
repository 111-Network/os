//! Hosted panels (Layer 5 §13; D19 d, amends D15): an adopted project's own
//! web UI, framed by the dashboard on the service's LIVE page, each on its own
//! origin. Only registry-declared panels are ever framed.
//!
//! Stub (Layer 5 task C4): only the built-in **shim** seam ([`shim`]) and the
//! fixed port block land here. The manifest loader, listeners, tickets, guards,
//! response policy and static serving are task P1, which owns this file next.

pub mod shim;

pub use shim::{Shim, ShimError, ShimRegistry, ShimRequest, ShimResponse, ShimSession};

/// Panel listener ports (§13: one per *enabled* panel, slot stable in
/// `state/panels.json`).
pub const PANEL_PORTS: std::ops::RangeInclusive<u16> = 11190..=11229;

#[cfg(test)]
mod tests {
    #[test]
    fn port_block_is_the_spec_block() {
        assert_eq!(super::PANEL_PORTS.clone().count(), 40);
        assert!(!super::PANEL_PORTS.contains(&crate::daemon::dashboard::DEFAULT_PORT));
    }
}
