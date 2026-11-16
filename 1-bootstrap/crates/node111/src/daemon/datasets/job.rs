//! Layer 8 (§7.4; §4.3 step 6; §4.2 item 5; §16.3 `dataset`): T0 stub; filled by row D32.
//!
//! Jobs: at most one dataset job per node, accepted fetches queued in order, on one named thread
//! `111d-dataset` that exists only while a job runs, with one block buffer. The states of §7.4 (a
//! pause is a state, never an error); what resumes a paused job, with no timer: the source hub's
//! link attribution, the user's Resume, a daemon start, and the capacity pass ([`on_pass`]: one
//! comparison and no I/O for the metered cap and low disk, a doubling gap of passes for a busy
//! source). `dataset` frames are coalesced: at most one per 5 s per job, plus one per state change.
//!
//! T0: [`on_pass`] returns at once; no thread exists.
#![allow(dead_code)]

use crate::daemon::housekeeping::PassCtx;

/// One dataset job (§7.4; its id `job-…`). T0: the name only.
pub struct Job;

/// §4.3 step 6: a paused job checked against its condition. T0: returns at once.
pub fn on_pass(_ctx: &PassCtx) {}
