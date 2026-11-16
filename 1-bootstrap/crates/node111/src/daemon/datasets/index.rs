//! Layer 8 (§5.5; §5.1 `state/datasets.json`): T0 stub; filled by row D25.
//!
//! The holdings index `state/datasets.json` (0600): per held (publisher, `id`, `version`) its
//! `sha256`, its location (`origin`, or `in_place` with an absolute path), its state (`partial`,
//! `present`, `unverified` or `disk_absent`), the tuple (device, inode, mtime ns, size) of a file
//! outside the tree, the consent hash and its job. Written atomically and only on a state change;
//! an unknown key kept. A file outside the tree is hashed once and then checked by its tuple, never
//! hashed per request; a `partial` record whose blob exists reads `present` at start.
#![allow(dead_code)]

/// The holdings index (§5.5). T0: the name only.
pub struct HoldingsIndex;
