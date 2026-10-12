//! Detached minisign verification (verify-only). The verify key is compiled into the
//! binary; a manifest's minisig MUST verify before any pin inside it is trusted.

/// Verify a detached minisign signature over `data`. Err on any failure —
/// callers never get a silent bool.
pub fn verify_detached(data: &[u8], signature_b64: &str, pubkey_b64: &str) -> anyhow::Result<()> {
    let _ = (data, signature_b64, pubkey_b64);
    todo!("T2")
}
