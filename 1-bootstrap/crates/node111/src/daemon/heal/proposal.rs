//! Layer 9 (§7.3 the `repair` proposal and its rules; §2.3; §11.4 step 7): T0 stub; filled by row
//! D25 (`repair` and `human_step` proposals), row D34 (the ledger check) and row D38 (the `fix`
//! kind).
//!
//! Under `propose`, for a diagnosed cause whose action is typed, `111d` composes `kind: repair`
//! with the body `{"why", "incident", "playbook", "cause", "actions": [Action]}` (at most 8 actions
//! and 16 KiB, this node's id, `task_id: null`); an `advise` or `human_step` cause files
//! `human_step`. `repair`, `fix` and `human_step` are written only by `111d`, through J9, with a
//! `heal/proposals` record holding their actions and `action_hash` in the same call; the agent
//! spool's ingest refuses them. A proposal holds at most one consent-gated action; a cause matched
//! on log text alone files `advise` only; a proposal whose incident closes undecided is withdrawn
//! (`dismissed`, `applied: {withdrawn: true}`).
//!
//! T0: the name only; nothing is filed.
#![allow(dead_code)]

/// The body of a `repair` proposal (§7.3). T0: the name only.
pub struct RepairBody;
