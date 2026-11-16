//! Layer 9 (§3.4 the fingerprint; §3.5 the record and the evidence hash; §18.2): T0 stub; filled by
//! row D6.
//!
//! `Incident` serialises to §3.5's record: `{schema: 1, id: "inc-<12 hex>", fingerprint: {layer,
//! subject, reason_code}, severity, count, state, opened, last, closed, evidence, checks,
//! ruled_out, diagnosis, explanation, attempts, dismissed, outcome}`. One open incident per
//! fingerprint (a repeat bumps `count` and `last`), at most 64 open (a 65th fingerprint opens
//! nothing and counts `overflow`), and a closed one re-opens on recurrence, keeping its diagnosis
//! until the evidence hash changes. The ids `inc-`, `att-`, `del-` and `dft-` (each plus 12 hex)
//! come from the OS random source before any store write, never from `seq`. The evidence hash is
//! BLAKE3-256 over the fingerprint, the sorted check results, the exit kind and the sorted
//! satisfied causes: never an excerpt, a count, a sequence number or a time. The table is held in
//! memory; row D15 writes it.
//!
//! T0: the names only; no table is held.
#![allow(dead_code)]

/// `(layer, subject, reason_code)` (§3.4). T0: the name only.
pub struct Fingerprint;

/// One incident record (§3.5). T0: the name only.
pub struct Incident;

/// The open incidents held in memory, at most 64, and the `overflow` count (§3.4). T0: the name
/// only.
pub struct Incidents;
