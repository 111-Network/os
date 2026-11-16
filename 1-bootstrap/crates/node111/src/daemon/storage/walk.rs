//! Layer 8 (§4.1, §15.4): T0 stub; filled by row D13.
//!
//! The on-request walks: the `updates`, `bridge` and `kb` figures and the `agent` row's spool,
//! `agent/work/` and `agent/legacy-v0/`, one `tree_bytes` walk each, on request only (never on the
//! pass); each result kept in memory until a release flip, a component change or 60 s, each walk
//! stopping at 100,000 entries and then reporting its figure as a lower bound. `components` comes
//! from the registry's own figures.
#![allow(dead_code)]

/// The walked figures and when each was taken (§4.1). T0: the name only.
pub struct Walks;
