//! Layer 8 (§2.4 rules 2 to 4 and 6; §2.2 rule 8): T0 stub; filled by row D6.
//!
//! The proposal and task spool's one hand-off rule: `111d` ingests a proposal on `proposal.written`
//! and a task on every terminal task event, never gated on the relay's freshness test, and sweeps
//! `agent/proposals/` and `agent/tasks/` (two `readdir`, no timer) at daemon start ([`sweep`], from
//! `state::start`), at every (re)subscription to the agent's events and before the proposals routes
//! answer. Only `<valid id>.json`, never a `.tmp-` name; parsed with the untrusted-file reader,
//! inserted by id, committed `FULL`, then unlinked; a file that does not parse stays and is
//! counted. With the store unavailable nothing is ingested and the files stay.
#![allow(dead_code)]

use crate::daemon::DaemonLayout;

/// The spool sweep at daemon start (§2.4 rule 2). T0: returns at once; no directory is read.
pub fn sweep(_layout: &DaemonLayout) {}
