//! Layer 8 (§13.8; §4.1 `logs`; §4.2 item 3; §4.3 step 5): T0 stub; filled by row D12.
//!
//! The child log rings: every file `open_log` opens (`agent/111-agent.log`,
//! `agent/llama-server.log`, each component's and child's log) is capped; on the pass, a log above
//! `logs.max_file_bytes` (default 4 MiB) has its last half copied to `<name>.1`, replacing it, and
//! is truncated to zero, while the child keeps appending through its `O_APPEND` descriptor. The
//! `logs` figure and its reserve read one `stat` per capped file that exists; a log below its cap
//! is never read.
#![allow(dead_code)]

use crate::daemon::housekeeping::PassCtx;

/// §4.3 step 5: each capped log over its cap rotates (§13.8). T0: returns at once.
pub fn on_pass(_ctx: &PassCtx) {}
