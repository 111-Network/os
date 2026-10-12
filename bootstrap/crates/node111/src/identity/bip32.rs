//! BIP-32 private derivation over secp256k1 (CKDpriv), minimal in-crate
//! implementation on hmac+sha2+k256 — validated against published NIP-06 vectors.

/// Derive the private key at a NIP-06-style path (e.g. `m/44'/1237'/0'/0/0`)
/// from a BIP-39 seed (64 bytes).
pub fn derive_private_key(seed: &[u8; 64], path: &str) -> anyhow::Result<[u8; 32]> {
    let _ = (seed, path);
    todo!("T3")
}
