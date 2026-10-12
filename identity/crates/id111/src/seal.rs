//! §6.4 — the sealed-packet construction (byte-exact, normative).
//!
//! Implements (owner: T3), per protected packet:
//!
//! 1. fresh ephemeral X25519 keypair `(epk, esk)` per §4 entropy discipline;
//! 2. `shared = X25519(esk, recipient_enc_pubkey)` — MUST abort on all-zero
//!    `shared` (RFC 7748 §6.1);
//! 3. `key = BLAKE2b-256("111/seal/v1" ‖ shared ‖ epk ‖ rpk)`,
//!    `nonce = BLAKE2b-192("111/seal-n/v1" ‖ epk ‖ rpk)` — the nonce is
//!    derived, never transmitted;
//! 4. `XChaCha20-Poly1305-Encrypt(key, nonce, plaintext, AD)` with
//!    AD = `version ‖ crypto_mode ‖ counter(LE u64) ‖ all other cleartext
//!    header bytes` (tag 16B);
//! 5. wire = cleartext header ‖ `epk` (32B) ‖ ciphertext+tag (56B overhead).
//!
//! Crypto-mode registry (owned by this layer): `0x01` = this construction;
//! receivers MUST reject unknown or weaker modes. NOT wire-compatible with
//! libsodium `crypto_box_seal` — nobody calls a sealed-box API verbatim.
//! The sender's NodeId travels INSIDE the ciphertext; MAC auth MUST NOT be
//! presented as proof of authorship (§12 rule 13).
