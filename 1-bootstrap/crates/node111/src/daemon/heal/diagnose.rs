//! Layer 9 (§5.1 the tiers; §5.2 the fold; §5.3 containment; §12.4; §12.5): T0 stub; filled by rows
//! D13 (T0 and the autonomy ladder), D23 (T1: one model task, the fold and the containment), D42
//! (the T2 branch) and D54 (the one-task queue a hub's delegated run shares).
//!
//! At an incident's open and at each change of its `count`, never on a timer, the playbooks whose
//! `[match]` equals the fingerprint are parsed and evaluated (T0): exactly one satisfied cause is
//! the diagnosis, `by: "t0"`. Where the `model` row is live, under `diagnose` or `propose` and only
//! when a loaded playbook matches, several causes or none make one T1 task through the loopback
//! task API with the `incident` input and a cause enum as `response_format`; its result is folded
//! into the incident, never into `agent/proposals`. A member with `heal.ask_hub` on asks its home
//! hub (T2). One model task at a time per node; Layer 9 starts, stops and reloads no model. With
//! none of the three the incident reads `unable_to_diagnose`.
//!
//! T0: the name only.
#![allow(dead_code)]

/// A diagnosis on an incident: `{cause_id, by, confidence}` (§3.5, §5.3). T0: the name only.
pub struct Diagnosis;
