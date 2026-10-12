//! `111 update-helper` — root oneshot (via 111node-update.path/service). The ONLY
//! actor that promotes staged payloads into `releases/`, flips `current`, and writes
//! the probation journal + monotonic floor. Re-verifies manifest minisig + staged-tree
//! hashes before promoting (closes the node111→root hole).

/// Entry point. Also services `state/revert-request.json` and probation
/// finalization (clear journal; prune to exactly 2 releases, never mid-probation).
pub fn run() -> anyhow::Result<()> {
    todo!("T11")
}
