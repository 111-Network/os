//! §2 — SLIP-0010 derivation at the pinned identity paths.
//!
//! Implements (owner: T2): SLIP-0010 hardened-only Ed25519 derivation
//! (HMAC-SHA512 keyed `"ed25519 seed"`) from the 64-byte BIP-39 seed:
//!
//! - `m/111'/0'/0'` → the Ed25519 signing key (key half = first 32 bytes of
//!   the SLIP-0010 output, used directly as the RFC 8032 private seed; the
//!   chain-code half is discarded);
//! - `m/111'/0'/1'` → the X25519 ECDH key (32-byte key half used as the
//!   private scalar, clamped per RFC 7748 at point of use).
//!
//! All components hardened; one account, fixed key indices. This is NOT
//! BIP32-Ed25519/Cardano — the two are incompatible and restore different
//! keys from the same words (§2, normative). Test vectors: §14.
