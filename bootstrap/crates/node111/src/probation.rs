//! Probation journal (`state/update.json`, root-written 0644) — the update actor
//! that survives restarts. The root helper writes it BEFORE flipping `current`;
//! `update-guard` (ExecStartPre, root) reverts expired unconfirmed probations.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Journal {
    pub schema: u32,
    pub pending: bool,
    /// Calver being tried.
    pub version: String,
    /// Calver to revert to.
    pub previous: String,
    /// Absolute deadline; health must be confirmed sustained before it.
    pub deadline_epoch: u64,
    pub health_confirmed: bool,
}

impl Journal {
    pub fn read(path: &std::path::Path) -> anyhow::Result<Option<Self>> {
        let _ = path;
        todo!("T11")
    }

    /// Atomic write; root-only caller.
    pub fn write(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let _ = path;
        todo!("T11")
    }

    /// Clear the journal (probation finalized or reverted).
    pub fn clear(path: &std::path::Path) -> anyhow::Result<()> {
        let _ = path;
        todo!("T11")
    }
}
