//! §6.3 — key distribution: the TOFU pinning store.
//!
//! Implements (owner: T4): keys distribute exclusively via signed announces;
//! there is no registry. The store holds `{sign_pubkey, enc_pubkey,
//! highest_announce_counter, receive_window}` per peer (≈80–130B; no
//! redundant `node_id` field — it IS `sign_pubkey` under B2).
//!
//! First-seen pins. **Pinned key material is immutable while held**: any
//! announce presenting different key material for a known NodeId is rejected
//! (Meshtastic CVE-2025-55293). The store is bounded — v1 default cap 256
//! peers with LRU eviction permitted (safe under B2: no attacker can
//! impersonate an evicted NodeId). No wall-clock fields (§12 rule 15).
