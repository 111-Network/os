//! id111 — the 111 Network identity core (the spec's `111id` module, §13/D13 H1):
//! every node's cryptographic self per `os/identity/README.md` (spec v2, LOCKED —
//! D13/D14). One BIP-39 root secret, one SLIP-0010-derived dual Ed25519+X25519
//! keyset; NodeId = the raw 32-byte Ed25519 public key.
//!
//! Module tree frozen at T1 per `os/identity/TASKS.md`; stubs are filled by their
//! owning tasks (W2). Each module doc names the spec section it implements.

/// §2–§4 — the dual keyset: Ed25519 signing + X25519 ECDH from one seed;
/// entropy-gated keygen (§4), the strict verification predicate (§3), the
/// X25519 all-zero abort, and the §4.5 weak-key self-test.
pub mod keyset;

/// §2 — SLIP-0010 hardened-only Ed25519 derivation (HMAC-SHA512 keyed
/// `"ed25519 seed"`) at the pinned paths `m/111'/0'/0'` (sign) and
/// `m/111'/0'/1'` (ECDH). NOT BIP32-Ed25519/Cardano.
pub mod slip0010;

/// §8.1 — the BIP-39 root secret: 12 words (128-bit) default, 24 (256-bit) for
/// hubs; checksum-validating parse, NFKC-normalized restore input; no
/// passphrase in v1.
pub mod bip39;

/// §6.4 — the sealed-packet construction, byte-exact: per-packet ephemeral
/// X25519, all-zero shared-secret abort, `"111/seal/v1"` BLAKE2b-256 key KDF,
/// `"111/seal-n/v1"` BLAKE2b-192 derived nonce, XChaCha20-Poly1305 with all
/// cleartext header bytes as AD. Crypto-mode registry (`0x01`), fail-closed.
/// NOT libsodium `crypto_box_seal`.
pub mod seal;

/// §6.2 — the announce format (137–169B): domain tag `"111/announce/v1"`
/// inside the signed region, strict-predicate verification before any state
/// change, strictly-monotonic counter (committed synchronously BEFORE
/// transmit), self-NodeId ⇒ identity-collision error state.
pub mod announce;

/// §6.3 — key distribution: the TOFU pinning store (`{sign_pubkey,
/// enc_pubkey, highest_announce_counter, receive_window}`), immutable pinned
/// key material, bounded (v1 cap 256 peers) with LRU eviction, no wall-clock
/// fields.
pub mod store;

/// §6.5 (and §6.2's announce counter) — replay protection: per-peer persistent
/// 64-bit counters + 32-entry sliding acceptance window; batch-commit every N
/// packets and on clean shutdown; post-unclean-reboot forward jump J ≥ N on
/// both sides; §8.4 restore floor + 1024 semantics.
pub mod counters;

/// §8.1–§8.4 — backup AND restore: the 4-byte self-describing descriptor
/// (version‖curve‖derivation‖encoding, fail-closed on unknown fields, absent ⇒
/// v1 defaults), ceremony artifacts (words + single-frame CompactSeedQR-style
/// project QR payload = 4B descriptor ‖ raw entropy), verify-on-write-down.
pub mod backup;

/// §8.4/§13/D14(d) — the D9-format Layer-1 migration: legacy dir detection,
/// same-words/new-derivation keyset, atomic new file set, legacy dir
/// quarantined as a sibling `<dir>.quarantine-d9` (Layer 1's own quarantine
/// shape — never deleted), old→new NodeId mapping. Restore itself lives in
/// `backup` (§8.4).
pub mod migrate;

/// §10 — transport attestation (F1): the 202B cross-signed format with domain
/// tags `"111/attest-node/v1"` / `"111/attest-transport/v1"`, supersedure by
/// greatest `expiry_epoch`, fail-closed on unknown version.
pub mod attest;

/// §7 — encodings & display: 34B multicodec-prefixed binary records
/// (`0xed 0x01` ed25519-pub / `0xec 0x01` x25519-pub), lowercase-hex machine
/// string, BIP-173 bech32 (NOT bech32m) with HRP `node` and enforced data-part
/// length, and the 10-char base32 operator label grouped `xxxx-xxxx-xx`
/// (display-only, never an authenticator).
pub mod encoding;

/// §8.3/§13 — the Cat-1 commissioning ceremony, commissioner side (owned by
/// the operator's computer running the `111` CLI per D14): verify-then-provision
/// ordering, provisioning-bundle export under the §5 carve-out channel
/// minimums, challenge-verify nonce, commissioner wipe at ceremony end.
/// Relay-side byte protocol: documented stub (§15).
pub mod commission;
