//! KB snapshot: rides inside the release payload at `current/share/kb/` (single
//! atomic unit with the binary). Integrity-verified at daemon start; no browsing
//! surface in v1.

/// Verify + summarize the KB at the current release path (sha256 + entry count
/// for /api/status). Err when missing/corrupt ⇒ health component red.
pub fn verify_current() -> anyhow::Result<KbStatus> {
    todo!("T7")
}

#[derive(Debug, Clone)]
pub struct KbStatus {
    pub sha256: String,
    pub entries: u32,
}
