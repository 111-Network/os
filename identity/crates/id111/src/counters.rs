//! §6.5 — replay protection: counters, not clocks (plus §6.2's announce
//! counter and §8.4's restore semantics).
//!
//! Implements (owner: T4): per-peer persistent 64-bit counters carried in
//! AEAD associated data (§6.4) + a 32-entry sliding acceptance window. No
//! wall-clock anywhere.
//!
//! Commit discipline: receive windows and per-peer send counters are
//! batch-committed every N packets and on clean shutdown (the §6.2 announce
//! counter instead commits synchronously BEFORE transmit — never batched).
//! After an unclean reboot BOTH sides apply a conservative forward jump with
//! **J ≥ N** (send-side counter and receive-side high-water alike). Counter
//! state is protocol state, not secret (§13 `counters` file / Cat-1 NVS map).
//!
//! Restore (§8.4): state present ⇒ reuse as-is; state absent ⇒
//! `counter = floor + 1024` with the floor learned per §8.4, and the same
//! jump applied to per-peer traffic send-counters on new sessions.
