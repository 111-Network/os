//! Layer 8 (§7.8; §12.4): T0 stub; filled by row D34.
//!
//! The metered window for dataset bytes: before every block the job asks Layer 7's metered module
//! which `metered` rule matches its connection (a rule the accepted consent did not name pauses it
//! `paused (metered link)`); on a marked link it pauses `paused (metered cap)` while the window has
//! less than the carrier's reserve left (10 % of the daily cap, at least 1 MiB). Dataset bytes
//! count in Layer 7's rolling 24 h window on `CLOCK_BOOTTIME`; override bytes count on the job
//! only, never in the window. It also gives the consent its `days` fact.
#![allow(dead_code)]

/// The per-block question to Layer 7's metered module (§7.8). T0: the name only.
pub struct MeteredGate;
