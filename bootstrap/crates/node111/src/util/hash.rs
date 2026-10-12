//! Streaming SHA-256 (hex, lowercase) — the pin format used by shim, manifest, receipt.

pub fn sha256_bytes(data: &[u8]) -> String {
    let _ = data;
    todo!("T2")
}

pub fn sha256_file(path: &std::path::Path) -> anyhow::Result<String> {
    let _ = path;
    todo!("T2")
}
