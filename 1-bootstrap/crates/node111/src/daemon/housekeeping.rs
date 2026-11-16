//! Layer 8 (§4.3, §1 rule 2, §18 "The pass knob"): T0 stub; filled by row D1 (the housekeeping
//! interval's owner).
//!
//! The daemon-level owner of the one 5 s housekeeping interval (`FabricConfig.housekeeping`,
//! spawned today inside `FabricHandle` and only while the fabric is live): row D1 spawns it once
//! from `daemon/mod.rs`, with or without a live fabric, and removes the spawn from the fabric, so
//! the daemon still holds exactly one interval (it changes owner, not count; no timer is added).
//! Every `PRUNE_EVERY` = 12 ticks, the first at `n = 0`, the pass runs: step 1 is the fabric's
//! `capacity_pass` with the `statvfs` figure it takes while the fabric is live, else the pass's own
//! `statvfs` of the root; steps 2 to 9 are the calls of [`pass`], in §4.3's order, whether or not
//! the fabric is live (step 3, `set_watermarks`, only while it is). The pass knob
//! `NODE111_TEST_HOUSEKEEPING_MS`, read by a debug build only, shortens the interval for the gates.
//!
//! T0: nobody calls [`pass`] (the interval stays in `daemon/fabric/mod.rs` until D1), and every
//! step returns at once: nothing is read, written, opened, spawned or scheduled.
#![allow(dead_code)]

use crate::daemon::{datasets, state, storage};
use mesh111::store::DiskSpace;

/// What one capacity pass knows (§4.3; row D1): the figures step 1 takes, handed to every later
/// step so that no step takes its own. Plain fields; nothing here is read from disk.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PassCtx {
    /// The `statvfs` figure of step 1: the fabric's while it is live, else the pass's own of the
    /// root; `None` until step 1 took one.
    pub disk: Option<DiskSpace>,
    /// Free space is below the fabric's 1 % floor (`DiskSpace::floor`; §4.2 item 5).
    pub disk_low: bool,
    /// The fabric's live bytes (the `fabric` class, §4.1); `None` with the fabric down.
    pub fabric_live_bytes: Option<u64>,
    /// The housekeeping tick count `n` of this pass (the first pass runs at `n = 0`).
    pub tick: u64,
}

/// One capacity pass: §4.3 steps 2 to 9, in order (step 1, the fabric's `prune`, fills `ctx` first
/// once row D1 lands). T0: nobody calls it, and each step returns at once.
pub fn pass(ctx: &mut PassCtx) {
    // steps 2-4: the budget's figures, `set_watermarks` on a change, the reclaim (§4.2; D10)
    storage::budget::on_pass(ctx);
    // step 5: a log over its cap rotates (§13.8; D12)
    storage::logs::on_pass(ctx);
    // step 6: a paused dataset job checked against its condition (§7.4; D32)
    datasets::job::on_pass(ctx);
    // step 7: the node-state store closes after a pass with no use (§2.2 rule 7; D2)
    state::on_pass(ctx);
    // step 8: one `storage` frame on a change §16.3 lists (D14)
    storage::on_pass_end(ctx);
    // step 9: a kiwix-serve no request reached for 30 passes stops (§9.3 (3); D42)
    datasets::kiwix::on_pass(ctx);
}
