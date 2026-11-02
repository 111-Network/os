//! Layer 7 (§12 metered and licensed links): T0 stub; filled by row D11.
//!
//! The metered budget: carrier bytes both ways in a rolling 24 h `CLOCK_BOOTTIME` window,
//! persisted, never the wall clock; past the cap no flush and the badge "metered cap reached" (§12;
//! §6.1 rule 6). Read before every flush.
#![allow(dead_code)]

/// The rolling metered window (§12). T0: the name only.
pub struct MeteredBudget;
