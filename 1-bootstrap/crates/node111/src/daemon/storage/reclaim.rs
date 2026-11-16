//! Layer 8 (§4.4 `agent`; §3 J9; §2.6): T0 stub; filled by row D11.
//!
//! The `agent` class's reclaimer, registered with J9 for `agent/*`: after the class's export offer,
//! finished tasks with no undecided proposal first (each with its record, its `agent/work/<task>/`
//! and its decided proposal and decision), oldest `seq` first; then decided proposals with no task;
//! then whole conversations by their head's `seq`. Never an undecided proposal, a running task, a
//! `pending` decision or the conversation of a turn in flight; "oldest" is `seq` distance, never a
//! clock.
#![allow(dead_code)]

/// The `agent` class's reclaimer (§4.4). T0: the name only.
pub struct AgentReclaimer;
