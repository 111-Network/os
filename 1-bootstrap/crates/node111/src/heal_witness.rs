//! Layer 9 (§3.3 rules 2 and 4; §23 L1 item 2): T0 stub; filled by row L1.
//!
//! `111 heal-witness`, the hidden verb the unit's `ExecStopPost=/usr/local/bin/111 heal-witness`
//! runs as `node111` after each daemon exit: it reads `$SERVICE_RESULT`, `$EXIT_CODE` and
//! `$EXIT_STATUS` (a signal name mapped to its number through a compiled table, `-` for a name the
//! table lacks) and appends one ASCII line of at most 96 B, `v1 boot=<32 hex|-> result=<word ≤16>
//! code=<≤3 digits|-> signal=<≤3 digits|->`, to `state/heal/inbox` through a temp file and a rename
//! that keeps the newest 256 lines, 0600. It exits 0 on every error, a missing `state/heal/`
//! included, so `ExecStopPost=` never fails the unit. One transient process per daemon exit, no
//! network call.
//!
//! T0: usage text only; `cli.rs` does not route the verb yet (row L1 does, as a hidden verb).
#![allow(dead_code)]

/// The hidden verb's one line (§3.3 rule 2). Row L1 keeps the verb out of `cli.rs`'s USAGE (absent
/// from the usage text, as `identity generate` is); this text names it here only. T0: text only.
pub const USAGE: &str =
    "  111 heal-witness                 (hidden: the unit's ExecStopPost= runs it after each daemon exit)";
