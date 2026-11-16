//! Layer 8 (§9.2, §9.3 (2) to (4), §4.3 step 9, §5.5): T0 stub; filled by row D42.
//!
//! The kiwix library and the idle stop: one symlink per held ZIM, `state/kiwix/zim/<publisher
//! short>-<id>-<version>.zim` → its blob; `library.xml` rebuilt with `kiwix-manage` when a ZIM is
//! added or removed ([`rebuild`]), then `SIGHUP` to a running kiwix-serve (no restart, no watcher);
//! one `stat` per in-place ZIM against its tuple before a start and before each rebuild. A running
//! kiwix-serve that no request through its panel's proxy and no search reached for 30 passes is
//! stopped ([`on_pass`]); the proxy and the search route reset that counter ([`touch`]).
//!
//! T0: all three return at once; nothing is started, written or signalled.
#![allow(dead_code)]

use crate::daemon::housekeeping::PassCtx;

/// Rebuild the library after a ZIM was added or removed (§9.2). T0: a no-op.
pub fn rebuild() {}

/// A request reached kiwix-serve through the panel's proxy or the search route: the idle counter
/// starts again (§9.3 (3)). T0: a no-op.
pub fn touch() {}

/// §4.3 step 9: a kiwix-serve no request reached for 30 passes stops (§9.3 (3)). T0: returns at
/// once.
pub fn on_pass(_ctx: &PassCtx) {}
