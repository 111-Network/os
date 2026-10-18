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
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let journal: Journal = serde_json::from_slice(&bytes)
            .map_err(|e| anyhow::anyhow!("corrupt probation journal {}: {e}", path.display()))?;
        anyhow::ensure!(
            journal.schema == 1,
            "journal schema {} unsupported",
            journal.schema
        );
        Ok(Some(journal))
    }

    /// Atomic write; root-only caller.
    pub fn write(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let json = serde_json::to_vec_pretty(self)?;
        crate::util::fs::atomic_write(path, &json, 0o644)
    }

    /// Clear the journal (probation finalized or reverted): write a non-pending
    /// tombstone (kept for forensics) rather than unlinking.
    pub fn clear(path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(mut j) = Self::read(path)? {
            j.pending = false;
            j.write(path)?;
        }
        Ok(())
    }
}

/// Probation window: from flip to confirmed-sustained health (spec: the health
/// checklist must hold across the window; watchdog kills during probation
/// count as probation failures).
pub const PROBATION_SECS: u64 = 600;
