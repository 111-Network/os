//! Layer 8 (§2.2, §2.4 rule 2, §2.5, §3 J2, §13.6 (1), §13.7): T0 stub; filled by row D2 (the
//! node-state store), row D17 (the migration window) and row D18 (the unclean signal handed to the
//! integrity checks).
//!
//! The node-state store `<root>/node/state.db` (§2.2): a second SQLCipher file that only `111d`
//! opens, keyed from `111/node-state/v1` through the identity handle the daemon holds (outside
//! `fabric::boot`, so it derives with the fabric down; D2 hands the handle to [`start`]), opened
//! through mesh111's `open_encrypted` on its first use and closed by [`on_pass`] on the first
//! capacity pass with no use since the previous one. It keeps no clean record: a non-empty
//! `state.db-wal` found before the open is the unclean signal. When it does not open (no key, a
//! newer `user_version`, a damaged file) it stays closed with a reason every caller reads.
//! [`start`] runs once from `daemon/mod.rs` before the fabric boots: the J2 fold, the legacy import
//! and the spool sweep, in that order; D17 adds the migration window (the copy and the migration
//! before the daemon reports health).
//!
//! T0: [`start`] and [`on_pass`] return at once; nothing is opened, read, written or spawned, and
//! no file of Layer 8 is open on an idle node (§1 rule 1).
#![allow(dead_code)]

pub mod heal;
pub mod legacy;
pub mod schema;
pub mod service;
pub mod spool;

use crate::daemon::housekeeping::PassCtx;
use crate::daemon::DaemonLayout;

/// The node-state store's handle (§2.2 rules 6 to 9): closed until its first use, `unavailable`
/// with a reason when it does not open. T0: the name only.
pub struct NodeState;

/// Layer 8's start (§2.2, §2.4 rule 2, §2.5, §3 J2), once, before the fabric boots: the J2 files
/// folded into `heal/`, the legacy agent files imported, the spool swept, in that order. T0: each
/// call returns at once.
pub fn start(layout: &DaemonLayout) {
    heal::fold_at_start(layout);
    legacy::import_at_start(layout);
    spool::sweep(layout);
}

/// §4.3 step 7: the store closes if no operation used it since the previous pass (§2.2 rule 7).
/// T0: returns at once.
pub fn on_pass(_ctx: &PassCtx) {}
