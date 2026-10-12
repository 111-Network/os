//! §7 — encodings & display.
//!
//! Implements (owner: T7):
//!
//! - Wire/disk binary records: raw 32B key prefixed with the 2-byte multicodec
//!   varint `0xed 0x01` (ed25519-pub) = 34B (`0xec 0x01` x25519-pub where a
//!   record needs it) — no multiformats dependency.
//! - Machine string: lowercase hex of the raw 32B key (canonical in protocol
//!   messages, logs, APIs).
//! - Human-transfer string: bech32 per BIP-173 — explicitly NOT bech32m —
//!   HRP `node`; display lowercase, QR transfer UPPERCASE, mixed case invalid;
//!   decoders MUST enforce the expected data-part length (embedded#15).
//! - Operator label: SHA-256(pubkey) truncated to 10 base32 chars displayed
//!   grouped `xxxx-xxxx-xx` — display-only, never an authenticator; plus the
//!   editable local alias (stored locally, never gossiped, never
//!   authoritative).
//!
//! Metadata versioning uses monotonic counters, never timestamps. BANNED per
//! §7: X.509/PEM/PKCS#8, JOSE/JWK, bech32m, `nsec`/`npub` HRP reuse.
