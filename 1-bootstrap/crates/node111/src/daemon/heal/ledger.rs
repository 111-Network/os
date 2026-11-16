//! Layer 9 (§7.7 the loop ledger; §4 the ledger row): T0 stub; filled by row D26 (the record and
//! its write) and row D34 (`unresolved` and the block).
//!
//! `heal/ledger`, keyed `sha256(action ‖ target)`: `{applied, reverted, unresolved, last_cause,
//! last_evidence_hash}`, rewritten in place and never reclaimed. After a revert, or after an
//! applied action that is `unresolved` (a new unrequested event bumps its incident's `count` after
//! the keep), the same action for the same cause is not proposed again until the human asks from
//! the incident or the evidence hash changes.
//!
//! T0: the name only.
#![allow(dead_code)]

/// One `heal/ledger` record (§7.7). T0: the name only.
pub struct LedgerRecord;
