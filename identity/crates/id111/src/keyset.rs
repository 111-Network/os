//! §2–§4 — the dual keyset and key generation discipline.
//!
//! Implements (owner: T2):
//! - §2: the keyset types — public keyset 64B (32B Ed25519 pub ‖ 32B X25519
//!   pub), private keyset 64B (32B Ed25519 seed ‖ 32B X25519 scalar);
//!   **NodeId = the raw 32-byte Ed25519 public key** (D13 B2).
//! - §3: the fleet-wide verification predicate — Ed25519 `verify_strict`
//!   semantics only (canonical R/A, S < ℓ, small-order rejected); dalek's
//!   default `verify()` MUST NOT be used for wire artifacts. X25519 low-order
//!   public keys rejected on ingress; all-zero shared secret aborts.
//! - §4: entropy-gated keygen — `getrandom()` is the named Cat 2+ source, no
//!   silent weak fallback, and the §4.5 weak-key self-test (all-zero /
//!   uniform-byte / counter-pattern / X25519 low-order) before persisting.
//! - §4 rule 6: no clone/duplicate-keyset feature is ever offered here.
