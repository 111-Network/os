//! Layer 8 (§13.1; §3 J6): T0 stub; filled by row L2.
//!
//! The export format, L2 §8.5(i)'s layout byte for byte: `version` 0x01; `ephemeral_pk`, fresh per
//! export, the key from `X25519(esk, own_enc_pk)` through BLAKE2b-256 under `111/seal/v1` (L2
//! §6.4); the ciphertext per 64 KiB plaintext chunk, XChaCha20-Poly1305 with the chunk index as the
//! nonce and the chunk index and `last_chunk` as the associated data (Layer 5's private-object
//! framing); `node_sig`, Ed25519 by the node key over `"111/export/v1" ‖ NodeId ‖ u64 LE length ‖
//! BLAKE3(every byte before node_sig)`. Streaming both ways, never a store in memory; a restore
//! verifies the signature (one BLAKE3 pass) before any decryption. No new crate (`age` stays out).
#![allow(dead_code)]

/// Writes one export, chunk by chunk (§13.1). T0: the name only.
pub struct ExportWriter;

/// Reads one export: the signature first, then the chunks (§13.1). T0: the name only.
pub struct ExportReader;
