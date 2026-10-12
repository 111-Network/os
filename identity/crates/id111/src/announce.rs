//! §6.2 — the announce format and verification rules (normative).
//!
//! Implements (owner: T4): the 137–169B announce —
//! `version(0x01) ‖ sign_pubkey(32=NodeId) ‖ enc_pubkey(32) ‖ counter(LE u64)
//! ‖ appdata(0–32) ‖ signature(64)` — with the ASCII domain tag
//! `"111/announce/v1"` prepended inside the signed region (never a wire field).
//!
//! Rules pinned here: unknown version ⇒ reject (fail closed); the signature
//! MUST verify under the §3 strict predicate before any state change;
//! `counter` MUST exceed the last-seen value for that NodeId (stale ⇒ drop)
//! and is committed synchronously (fsync) BEFORE the announce it authorizes
//! is transmitted; conflicting key material for a pinned NodeId ⇒ reject
//! (§6.3); an announce carrying the receiver's OWN NodeId MUST NOT be pinned
//! and MUST surface the distinct identity-collision error state.
