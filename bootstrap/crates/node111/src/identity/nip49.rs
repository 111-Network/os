//! NIP-49 `ncryptsec` encryption (scrypt + XChaCha20-Poly1305 + bech32),
//! validated against published NIP-49 vectors.

pub fn encrypt(secret_key: &[u8; 32], passphrase: &str) -> anyhow::Result<String> {
    let _ = (secret_key, passphrase);
    todo!("T3")
}
