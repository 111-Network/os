//! Shared test helpers: a minimal loopback HTTP/1.1 file server (no deps —
//! TASKS.md bars tiny_http et al).
//!
//! Compiled separately into every integration binary — helpers unused by one
//! binary are dead code there; the allow keeps `-D warnings` honest.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

/// Serve `dir` over HTTP on 127.0.0.1 (ephemeral port). Returns the port; the
/// server runs on a daemon thread until the process exits. `date_header` is
/// sent verbatim when Some (clock-skew tests).
pub fn serve_dir(dir: PathBuf, date_header: Option<String>) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { break };
            let dir = dir.clone();
            let date_header = date_header.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let Ok(n) = conn.read(&mut buf) else { return };
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .trim_start_matches('/');
                let (status, body) = if path.trim_end_matches('/') == "204" {
                    ("204 No Content", Vec::new())
                } else {
                    let file = dir.join(path);
                    match std::fs::read(&file) {
                        Ok(bytes) => ("200 OK", bytes),
                        Err(_) => ("404 Not Found", b"not found".to_vec()),
                    }
                };
                let date = date_header
                    .map(|d| format!("Date: {d}\r\n"))
                    .unwrap_or_default();
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{date}\r\n",
                    body.len()
                );
                let _ = conn.write_all(head.as_bytes());
                let _ = conn.write_all(&body);
            });
        }
    });
    port
}

/// Sign a file with the throwaway test key (minisign CLI).
pub fn minisign_fixture_key() -> PathBuf {
    PathBuf::from(format!(
        "{}/tests/fixtures/minisign/test.key",
        env!("CARGO_MANIFEST_DIR")
    ))
}

pub fn minisign_other_key() -> PathBuf {
    PathBuf::from(format!(
        "{}/tests/fixtures/minisign/other.key",
        env!("CARGO_MANIFEST_DIR")
    ))
}

pub fn sign_file(key: &std::path::Path, file: &std::path::Path) {
    let minisign = format!("{}/.local/bin/minisign", std::env::var("HOME").unwrap());
    let out = std::process::Command::new(minisign)
        .args([
            "-S",
            "-W",
            "-s",
            &key.to_string_lossy(),
            "-m",
            &file.to_string_lossy(),
            "-x",
            &format!("{}.minisig", file.to_string_lossy()),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "minisign failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Build a payload tarball (gz) with bin/111 + share/kb/* + share/payload.manifest.
pub fn build_payload() -> Vec<u8> {
    build_payload_with(&[])
}

/// [`build_payload`] plus `extra` files (relpath, bytes; 0o644, 0o755 under
/// `bin/`), each listed in `share/payload.manifest` like every payload file — the
/// Layer 5 assets (`bin/meshmud`, `panels/<bundle>/…`, `share/meshtastic-presence/…`).
pub fn build_payload_with(extra: &[(&str, &[u8])]) -> Vec<u8> {
    build_payload_listing(extra, &[], &[], true)
}

/// [`build_payload_with`] with a `share/payload.manifest` that can disagree with the
/// files (the generator's release-time listing guard, `manifest/gen-manifest.py`):
/// the `unlisted` relpaths are packed but left out of the listing, each `phantom`
/// (relpath, bytes) is listed with the hash of its bytes but not packed, and
/// `listing: false` packs no `share/payload.manifest` at all.
pub fn build_payload_listing(
    extra: &[(&str, &[u8])],
    unlisted: &[&str],
    phantom: &[(&str, &[u8])],
    listing: bool,
) -> Vec<u8> {
    let mut files: Vec<(String, Vec<u8>, u32)> = vec![
        (
            "bin/111".into(),
            b"#!/bin/sh\necho fake 111 daemon\n".to_vec(),
            0o755,
        ),
        ("share/kb/llms.txt".into(), b"index\n".to_vec(), 0o644),
        ("share/kb/reticulum.md".into(), b"entry\n".to_vec(), 0o644),
    ];
    for (path, bytes) in extra {
        let mode = if path.starts_with("bin/") {
            0o755
        } else {
            0o644
        };
        files.push((path.to_string(), bytes.to_vec(), mode));
    }
    let mut manifest = String::new();
    for (path, bytes, _) in &files {
        if unlisted.contains(&path.as_str()) {
            continue;
        }
        manifest.push_str(&format!(
            "{}  {}\n",
            node111::util::hash::sha256_bytes(bytes),
            path
        ));
    }
    for (path, bytes) in phantom {
        manifest.push_str(&format!(
            "{}  {}\n",
            node111::util::hash::sha256_bytes(bytes),
            path
        ));
    }
    if listing {
        files.push((
            "share/payload.manifest".into(),
            manifest.into_bytes(),
            0o644,
        ));
    }

    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    {
        let mut builder = tar::Builder::new(&mut gz);
        for (path, bytes, mode) in &files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(*mode);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder
                .append_data(&mut header, path, bytes.as_slice())
                .unwrap();
        }
        builder.finish().unwrap();
    }
    gz.finish().unwrap()
}
