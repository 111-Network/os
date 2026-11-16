//! Layer 9 (§7.4 step 2 and the phases; §7.5 "Armed at start = revert"; Layer 8's J2): T0 stub;
//! filled by row D20 (the undo file and the guard) and row D21 (the reconciliation at start).
//!
//! `state/heal/undo` = `{schema: 1, phase, proposal_id|null, attempt_id, action, target, snapshot,
//! release, starts, armed: {boot_id, mono_ms}}`, atomic 0600 with fsync, written after
//! `decision_begin` and before an action's first effect; a record that does not re-read equal
//! refuses the action. Its phases are `armed`, `keeping`, `restoring`, `restored` and `blocked`;
//! the file is removed last. While it exists, or the node-wide lock is held, every accept and every
//! `apply_unrecorded` answers `409 busy`. At start, before serving and with the store closed, the
//! file is reconciled by its phase: the previous run's `armed` record is restored cold, at most
//! once; a stale one, or `restoring`, reads `revert_failed`; `restored` and `keeping` are finished
//! once the store opens; `blocked` stays for the human.
//!
//! T0: the names only; nothing is read or written.
#![allow(dead_code)]

/// The undo record `state/heal/undo` (§7.4 step 2). T0: the name only.
pub struct UndoRecord;

/// The undo file's phase: `armed`, `keeping`, `restoring`, `restored` or `blocked` (§7.5). T0: the
/// name only.
pub struct Phase;
