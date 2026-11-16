//! Layer 9 (§7.1 the trait; §7.4 the loop; §7.6 the reach class; §7.7 after the keep): T0 stub;
//! filled by rows D19 (the trait over the Layer 6 applier), D20 (the guard), D21 (`finish_keep`),
//! D26 (the loop behind the decision route), D31 (the reach confirm), D32 (restore and dismiss),
//! D33 (Undo and the snapshots), D39 (a fix's `reverted` hook), D52 (an attempt with no incident)
//! and D53 (the `heal_remove` text and hash).
//!
//! The Layer 6 applier generalised: per action, in order, `plan()` (pure: `{diff, preflight, reach,
//! removes_capability, evicts}`), the undo armed, `apply()`, then `probe()` within its window, then
//! keep or revert; `snapshot()`, `restore()` and `restore_cold()` per action. One apply at a time
//! on a node: the applier's lock becomes the node-wide lock, held from arm to keep, revert,
//! `revert_blocked` or confirm. A `may_affect` action waits up to 90 s for its confirm; an
//! automatic revert never passes a consent gate. Debug builds read `NODE111_HEAL_PAUSE_AT`; release
//! builds ignore it.
//!
//! T0: the names only; nothing is applied and no lock is taken.
#![allow(dead_code)]

/// A typed action behind the engine (§7.1). T0: the name only.
pub trait Action {}

/// `plan()`'s answer: `{diff, preflight, reach, removes_capability, evicts}` (§7.1). T0: the name
/// only.
pub struct Plan;

/// An action's reach class: `none` or `may_affect` (§7.6). T0: the name only.
pub struct Reach;
