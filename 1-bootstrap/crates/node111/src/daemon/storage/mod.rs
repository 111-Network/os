//! Layer 8 (§4, §13, §15, §16.1 to §16.3): T0 stub; filled by row D14 (`GET /api/v1/storage`'s
//! status and the `storage` frame) and row D21 (the `update_refused_space` notice).
//!
//! One node budget and the stores' housekeeping: `state/storage.json` (D9), the budget on the pass
//! (D10), the `agent` class's reclaimer (D11), the child log rings (D12), the on-request walks
//! (D13), `node/lock` (D16), the migration window's clean-up (D17), the integrity checks (D18),
//! Layer 8's consent copy (D19) and the set-aside store files (D20). `StorageStatus` (§16.2) is
//! computed on request and writes nothing; [`on_pass_end`], §4.3 step 8, publishes one `storage`
//! frame only on a change §16.3 lists, never on a timer or an unchanged pass.
//!
//! T0: [`on_pass_end`] returns at once.
#![allow(dead_code)]

pub mod budget;
pub mod consent;
pub mod integrity;
pub mod lock;
pub mod logs;
pub mod migrate;
pub mod reclaim;
pub mod setaside;
pub mod settings;
pub mod walk;

use crate::daemon::housekeeping::PassCtx;

/// `GET /api/v1/storage`'s answer (§16.2), computed on request. T0: the name only.
pub struct StorageStatus;

/// §4.3 step 8: one `storage` frame when a change §16.3 lists happened on this pass. T0: returns at
/// once.
pub fn on_pass_end(_ctx: &PassCtx) {}
