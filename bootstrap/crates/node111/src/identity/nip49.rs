//! NIP-49 `ncryptsec` encryption (scrypt + XChaCha20-Poly1305 + bech32),
//! validated against the published NIP-49 decrypt vector.
//!
//! Layout (91 bytes before bech32):
//!   0x02 || LOG_N || SALT(16) || NONCE(24) || KEY_SECURITY_BYTE(1) || CIPHERTEXT(48)
//! KEY_SECURITY_BYTE: 0x00 — our key sits unencrypted in identity/secret.hex,
//! which NIP-49 defines as "known to have been handled insecurely" (stored
//! unencrypted). Honest over 0x02.
//!
//! Passphrases are NFKC-normalized before scrypt (NIP-49 requirement so the
//! same passphrase types identically across clients).

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use unicode_normalization::UnicodeNormalization;

const VERSION: u8 = 0x02;
const KEY_SECURITY_BYTE: u8 = 0x00; // stored unencrypted on disk — see module docs
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
/// NIP-49 default work factor (64 MiB, ~100ms on a fast machine).
pub const DEFAULT_LOG_N: u8 = 16;
const PAYLOAD_LEN: usize = 1 + 1 + SALT_LEN + NONCE_LEN + 1 + 48;

fn scrypt_key(passphrase: &str, salt: &[u8], log_n: u8) -> anyhow::Result<[u8; 32]> {
    let normalized: String = passphrase.chars().nfkc().collect();
    let params = scrypt::Params::new(log_n, 8, 1, 32)
        .map_err(|_| anyhow::anyhow!("invalid scrypt params (log_n={log_n})"))?;
    let mut key = [0u8; 32];
    scrypt::scrypt(normalized.as_bytes(), salt, &params, &mut key)
        .map_err(|_| anyhow::anyhow!("scrypt failed"))?;
    Ok(key)
}

fn bech32_encode_ncryptsec(payload: &[u8]) -> anyhow::Result<String> {
    let hrp = bech32::Hrp::parse("ncryptsec").expect("constant hrp is valid");
    bech32::encode::<bech32::Bech32>(hrp, payload)
        .map_err(|e| anyhow::anyhow!("bech32 encode: {e}"))
}

/// Encrypt a 32-byte secret key into `ncryptsec…` (random salt+nonce from the OS).
pub fn encrypt(secret_key: &[u8; 32], passphrase: &str) -> anyhow::Result<String> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    crate::identity::os_entropy(&mut salt)?;
    crate::identity::os_entropy(&mut nonce)?;
    encrypt_deterministic(secret_key, passphrase, &salt, &nonce)
}

/// Split out for tests (fixed salt/nonce) — production uses `encrypt`.
pub(crate) fn encrypt_deterministic(
    secret_key: &[u8; 32],
    passphrase: &str,
    salt: &[u8; SALT_LEN],
    nonce: &[u8; NONCE_LEN],
) -> anyhow::Result<String> {
    let key = scrypt_key(passphrase, salt, DEFAULT_LOG_N)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: secret_key,
                aad: &[KEY_SECURITY_BYTE],
            },
        )
        .map_err(|_| anyhow::anyhow!("XChaCha20-Poly1305 encrypt failed"))?;

    let mut payload = Vec::with_capacity(PAYLOAD_LEN);
    payload.push(VERSION);
    payload.push(DEFAULT_LOG_N);
    payload.extend_from_slice(salt);
    payload.extend_from_slice(nonce);
    payload.push(KEY_SECURITY_BYTE);
    payload.extend_from_slice(&ciphertext);
    debug_assert_eq!(payload.len(), PAYLOAD_LEN);
    bech32_encode_ncryptsec(&payload)
}

/// Decrypt an `ncryptsec…` string back to the 32-byte secret key.
pub fn decrypt(ncryptsec: &str, passphrase: &str) -> anyhow::Result<[u8; 32]> {
    let (hrp, payload) =
        bech32::decode(ncryptsec.trim()).map_err(|e| anyhow::anyhow!("ncryptsec bech32: {e}"))?;
    anyhow::ensure!(
        hrp.as_str() == "ncryptsec",
        "not an ncryptsec string (hrp {})",
        hrp.as_str()
    );
    anyhow::ensure!(
        payload.len() == PAYLOAD_LEN,
        "ncryptsec payload {} bytes, want {PAYLOAD_LEN}",
        payload.len()
    );
    anyhow::ensure!(
        payload[0] == VERSION,
        "ncryptsec version {:#x} unsupported",
        payload[0]
    );
    let log_n = payload[1];
    let salt = &payload[2..2 + SALT_LEN];
    let nonce = &payload[2 + SALT_LEN..2 + SALT_LEN + NONCE_LEN];
    let aad = &payload[2 + SALT_LEN + NONCE_LEN..3 + SALT_LEN + NONCE_LEN];
    let ciphertext = &payload[3 + SALT_LEN + NONCE_LEN..];

    let key = scrypt_key(passphrase, salt, log_n)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let plain = cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| anyhow::anyhow!("ncryptsec decrypt failed (wrong passphrase or corrupt)"))?;
    let mut out = [0u8; 32];
    out.copy_from_slice(&plain);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex32(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, c) in s.as_bytes().chunks(2).enumerate() {
            out[i] = u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap();
        }
        out
    }

    fn hex_of(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// Published NIP-49 decrypt vector.
    #[test]
    fn nip49_decrypt_vector() {
        let ncryptsec = "ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p";
        let key = decrypt(ncryptsec, "nostr").unwrap();
        assert_eq!(
            hex_of(&key),
            "3501454135014541350145413501453fefb02227e449e57cf4d3a3ce05378683"
        );
    }

    #[test]
    fn roundtrip_and_wrong_password() {
        let secret = hex32("3501454135014541350145413501453fefb02227e449e57cf4d3a3ce05378683");
        let enc = encrypt(&secret, "correct horse").unwrap();
        assert!(enc.starts_with("ncryptsec1"));
        assert_eq!(decrypt(&enc, "correct horse").unwrap(), secret);
        assert!(decrypt(&enc, "wrong").is_err());
    }

    /// NIP-49 requires NFKC before scrypt: a decomposed-form passphrase must
    /// decrypt a key encrypted with the composed form.
    #[test]
    fn nfkc_equivalence() {
        let secret = hex32("3501454135014541350145413501453fefb02227e449e57cf4d3a3ce05378683");
        let composed = "\u{00C5}\u{03A9}\u{1E69}"; // Å Ω ṩ
        let decomposed = "\u{212B}\u{2126}\u{1E9B}\u{0323}"; // Å Ω ṛ̣
        let enc = encrypt(&secret, composed).unwrap();
        assert_eq!(decrypt(&enc, decomposed).unwrap(), secret);
    }
}
