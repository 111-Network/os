//! Detached minisign verification (verify-only). The verify key is compiled into the
//! binary; a manifest's minisig MUST verify before any pin inside it is trusted.

use anyhow::Context;

/// Verify a detached minisign signature over `data`. Err on any failure —
/// callers never get a silent bool.
///
/// `signature_b64` is the full `.minisig` file content (the 4-line form:
/// untrusted comment, base64 signature, trusted comment, base64 global
/// signature) as produced by `minisign -Sm`. `pubkey_b64` is the base64 line
/// from the `.pub` file (the `RW...` string). Only modern pre-hashed
/// signatures are accepted (`minisign -H` legacy mode is refused).
pub fn verify_detached(data: &[u8], signature_b64: &str, pubkey_b64: &str) -> anyhow::Result<()> {
    let pubkey = minisign_verify::PublicKey::from_base64(pubkey_b64.trim())
        .context("invalid minisign public key (base64 decode)")?;
    let signature = minisign_verify::Signature::decode(signature_b64)
        .context("invalid minisign signature encoding")?;
    pubkey
        .verify(data, &signature, false)
        .context("minisign signature verification failed")
}

#[cfg(test)]
mod tests {
    // Known-answer vectors live in tests/minisign_verify.rs against the committed
    // throwaway fixtures (tests/fixtures/minisign/).
    #[test]
    fn garbage_inputs_err_not_panic() {
        assert!(super::verify_detached(b"x", "garbage", "garbage").is_err());
    }
}
