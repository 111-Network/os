//! Layer 8 (§2.5; §3 J7, J8): T0 stub; filled by row D8.
//!
//! The legacy import, once, at the first start of a release that carries the node-state store:
//! every `agent/conversations/*.jsonl`, `agent/proposals/*.json`, `agent/decisions/*.json` and
//! finished `agent/tasks/*.json`, one `FULL` transaction per file, then moved to
//! `agent/legacy-v0/`; a conversation merged by line count (`legacy_lines`). The J7 ring
//! `state/heal/journal.jsonl`, when Layer 9 built before the store, is ingested into `heal/` and
//! moved to `state/heal/legacy-v0.jsonl`. The moves back are the update guard's (row L4), the
//! deletion at confirmation row D17's.
#![allow(dead_code)]

use crate::daemon::DaemonLayout;

/// The legacy import at start (§2.5). T0: returns at once; no directory is read.
pub fn import_at_start(_layout: &DaemonLayout) {}
