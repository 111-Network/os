//! Layer 8 (§4.2, §4.3 steps 2 to 4, §4.5, §4.6): T0 stub; filled by row D10.
//!
//! The budget on the pass. `keep_default` is computed once, at the first start of a Layer 8 release
//! (that start's one write); the shares are the `logs` reserve, then `fabric` 3/4 and the
//! node-state store 1/4, inside it `heal` at most a quarter. Step 2 reads the pass's figures, the
//! in-memory `ns_totals` and one `stat` per capped log: no walk, no store open. Step 3, on a change
//! of `keep` or a share, calls `set_watermarks` once (never below the built 512 MiB while
//! `keep_bytes` is unset; `quota.transit` written once). Step 4, with the budgeted classes over
//! `keep` or `disk_low` set, reclaims `agent` and `heal` through J9, the class furthest over its
//! share first, unless its export offer is unanswered (then the class is `full`). The §4.6 fit
//! check serves every fetch, draft, import and component cache check.
#![allow(dead_code)]

use crate::daemon::housekeeping::PassCtx;

/// §4.3 steps 2 to 4 (step 3 only while the fabric is live). T0: returns at once.
pub fn on_pass(_ctx: &PassCtx) {}
