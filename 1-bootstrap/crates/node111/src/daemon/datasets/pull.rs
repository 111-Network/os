//! Layer 8 (§7.3 (3) to (5), §5.1 (3), §7.10, §7.11): T0 stub; filled by row D31.
//!
//! The pull from a node origin, one job's work on the `111d-dataset` thread: the sidecar first (its
//! exact length, its root, its name proven before its rename), then one block per `get_to_file`
//! call into `state/origin/incoming/<sha256>.part`, each block checked with `valid_ranges` (a
//! failed block fetched once more from the same source, then from the next) and its bit set in
//! `.blocks`; after the last block the file's sha256 proves its name before the rename into
//! `blob/`, and an existing blob is never replaced. The `.blocks` header's boot id decides how much
//! a resume re-checks.
#![allow(dead_code)]

/// One pull of one entry from a node origin (§7.3 (3)). T0: the name only.
pub struct Pull;
