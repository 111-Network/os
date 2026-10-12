//! §8.1–§8.3 — backup: the versioned, self-describing format and ceremony
//! artifacts.
//!
//! Implements (owner: T5):
//!
//! - §8.2: the 4-byte descriptor `version(0x01) ‖ curve(0x01 =
//!   ed25519+x25519 dual) ‖ derivation(0x01 = SLIP-0010 m/111'/0'/{0',1'}) ‖
//!   encoding(0x01 = BIP-39 English 12 words · 0x02 = 24 words)`. Restore
//!   MUST honor the descriptor and MUST fail closed on any unknown field; a
//!   v1 restore with no descriptor present assumes the v1 defaults.
//! - §8.3: ceremony artifacts on the commissioning device — the words plus a
//!   single-frame CompactSeedQR-style project QR (payload = 4B descriptor ‖
//!   raw entropy, 20–36B) and the verify-on-write-down gate (sampled-word
//!   re-entry or QR re-scan; the backup is not made until verification
//!   passes). No passphrase; paper default, metal for hubs.
//!
//! The TTY-only display/refusal rules of §8.3 are enforced by the CLI wiring
//! (node111), not here.
