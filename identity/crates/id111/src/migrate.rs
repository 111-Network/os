//! §8.4 — restore and the Layer-1 (D9-format) migration.
//!
//! Implements (owner: T5):
//!
//! - Restore: NFKC + checksum-validated words (or QR scan) → descriptor check
//!   (fail closed; absent ⇒ v1 defaults) → SLIP-0010 derivation (§2) → atomic
//!   identity-dir write (§13; never overwrite) → the restored NodeId is
//!   surfaced for operator confirmation against a recorded reference when one
//!   exists. MUST refuse against a live, unquarantined identity dir.
//! - Wire-function recovery: counter floor rules per §8.4 (state present ⇒
//!   reuse; state absent ⇒ floor from mesh-gossiped prior announces / operator
//!   floor / 0 for never-deployed, then `counter = floor + 1024`).
//! - Old-instance retirement: the ceremony instructs and requires
//!   confirmation of retiring the old instance (duplicate live instances ⇒
//!   §6.2 identity-collision).
//! - D9 migration: read the provisional `mnemonic` file, derive the L2 keyset
//!   (same words, new derivation — the NodeId changes once, pre-accepted),
//!   write the §13 files, quarantine legacy `secret.hex`/`npub`, log the
//!   old→new NodeId mapping.
//!
//! Compromise remedy is a burn (new mnemonic, new NodeId); identity lost
//! without backup is unrecoverable by design.
