//! Layer 8 (§3 J2, J8; §21 "Hand-off to Layer 9"): T0 stub; filled by row D4.
//!
//! The three plaintext files a shell hook writes or that must be read before the store opens:
//! `state/heal/run`, `state/heal/inbox` (one line per daemon exit; an exit with status 75, the
//! answer to a store verb holding `node/lock`, is recorded and never counted as a crash) and
//! `state/heal/undo`. `111d` reads them at start before serving, hands them to the folder Layer 9
//! registers, which writes them into `heal/`, and clears them only after that write committed; with
//! the store unopenable they stay and `node_state.reason` says so.
#![allow(dead_code)]

use crate::daemon::DaemonLayout;

/// The J2 fold at start (§3 J2). T0: returns at once; nothing is read.
pub fn fold_at_start(_layout: &DaemonLayout) {}
