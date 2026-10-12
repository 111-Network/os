//! Update client: fetch → verify (minisig + floor) → stage into `state/staging/` →
//! drop `state/update-request.json`. The daemon CANNOT write `releases/` or flip
//! `current` — the privileged root helper is the only flip actor (D12).

pub mod fetch;
pub mod stage;

/// Run the updater loop (interval poll, exp backoff + jitter, last-successful-check
/// persistence, clock-fault surfacing).
pub async fn run() {
    todo!("T10")
}
