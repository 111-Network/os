//! Layer 8 (§5.2 (1) to (4), §5.6): T0 stub; filled by row D23.
//!
//! The sidecar: the bao-tree post-order outboard at `ob_log` 14 (16 MiB blocks) for files of 1 GiB
//! and more, 12 below, stored as one more blob named by its own sha256. Its exact length,
//! `(ceil(size / 2^(10 + ob_log)) − 1) × 64`, comes from the signed `size` and `ob_log` (any other
//! length refused; a figure over 8 MiB refused). `valid_outboard_ranges` checks it against the
//! entry's BLAKE3 root and `valid_ranges` each block; one pass computes sha256, BLAKE3 and the
//! sidecar together, with one block-sized buffer.
#![allow(dead_code)]

/// A dataset's bao-tree outboard (§5.2). T0: the name only.
pub struct Sidecar;
