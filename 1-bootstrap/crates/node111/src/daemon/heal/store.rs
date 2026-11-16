//! Layer 9 (§4, §4.1, §4.2; Layer 8's J1 to J9): T0 stub; filled by row D15 (the `heal/` records
//! through J9), row D17 (the reclaimer and the protected states) and row D18 (the degraded modes).
//!
//! One adapter over Layer 8's J9 calls for every `heal/` record kind (`incidents`, `attempts`,
//! `proposals`, `ledger`, `snapshots`, `fixes` with the `high_water` records, `delegations`,
//! `drafts`) and the `heal/audit` stream, each record's store state one of §4.2's. One reclaimer
//! for `heal/*`, run only in the pass's step 4, never touching a protected state and leaving the J3
//! marker. With the store closed the observer keeps at most 64 incidents in memory and nothing is
//! filed; while `heal` is `full` observation continues and no proposal is filed; under J7 the ring
//! `state/heal/journal.jsonl` holds `heal/`. Nothing in `heal/` is signed or leaves the node.
//!
//! T0: the names only; nothing is opened, read or written.
#![allow(dead_code)]

/// The `heal/` namespace through J9 (§4). T0: the name only.
pub struct HealStore;

/// One `heal/audit` entry: `{seq, boot_id, mono_ms, kind, proposal_id, action_hash, probe_result,
/// revert_result, confirmer, by}` (§4). T0: the name only.
pub struct AuditEntry;
