//! §8.1 — the BIP-39 root secret.
//!
//! Implements (owner: T2): mnemonic generation and validation over the
//! rust-bitcoin `bip39` crate (2048-word English list):
//!
//! - 12 words (128-bit) default; 24 words (256-bit) for hubs (Cat 4 SHOULD);
//! - generation draws entropy per §4 (`getrandom`) and composes via
//!   `Mnemonic::from_entropy` — no crate RNG path;
//! - restore input is NFKC-normalized (case/whitespace folded) and the BIP-39
//!   checksum MUST be validated before derivation (§8.4 hygiene);
//! - **no passphrase in v1** (the silent-wrong-wallet trap, §8.1);
//! - the 64-byte BIP-39 seed feeds `slip0010` (§2).
//!
//! Known limit, stated per §8.1: the checksum is 4 bits at 12 words (8 at
//! 24) — the §8.4 NodeId confirmation exists because of it.
