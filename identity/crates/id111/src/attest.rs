//! §10 — transport attestation (F1): binding a per-node transport credential
//! to the node by bidirectional cross-signed attestation.
//!
//! Implements (owner: T6): the 202B wire format
//! `version(0x01) ‖ node_pubkey(32) ‖ transport_id(1; registry owned by this
//! layer — 0x01 = nostr-secp256k1) ‖ transport_key(32) ‖ expiry_epoch(8,
//! mesh-epoch) ‖ node_sig(64) ‖ transport_sig(64)` with domain-tagged signed
//! inputs: `node_sig` covers `"111/attest-node/v1" ‖ version ‖ node_pubkey ‖
//! transport_key ‖ transport_id ‖ expiry_epoch`; `transport_sig` covers
//! `"111/attest-transport/v1" ‖ node_pubkey`.
//!
//! Verification checks both signatures and the expiry epoch. Supersedure
//! (normative): per `(node_pubkey, transport_id)` the greatest `expiry_epoch`
//! wins; verifiers retain only it and reject re-publication of a superseded
//! one. Fail closed on unknown version. This machinery is the designed future
//! rotation path (rotation = key succession under B2). A transport key MUST
//! NOT share seed material with the node key; NIP-05/NIP-26 are never
//! load-bearing. This layer defines the format only (Layer 7 owns transports).
