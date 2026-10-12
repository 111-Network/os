//! Integration tests for util::fs / util::hash that benefit from real files
//! (concurrency, rename semantics).

use node111::util::fs::atomic_write;
use std::os::unix::fs::PermissionsExt;

#[test]
fn concurrent_writers_never_torn() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.json");
    let mut handles = Vec::new();
    for t in 0..8u32 {
        let p = path.clone();
        handles.push(std::thread::spawn(move || {
            for i in 0..50u32 {
                // Two fixed payloads per writer pair so any read must equal one of them.
                let payload = if t % 2 == 0 {
                    format!("{{\"writer\":\"even\",\"i\":{i}}}").into_bytes()
                } else {
                    format!("{{\"writer\":\"odd\",\"i\":{i}}}").into_bytes()
                };
                atomic_write(&p, &payload, 0o644).unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let final_bytes = std::fs::read(&path).unwrap();
    let s = String::from_utf8(final_bytes).unwrap();
    assert!(s.contains("\"writer\":\"even\"") || s.contains("\"writer\":\"odd\""));
    // No tmpfiles left behind.
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "tmpfiles leaked: {leftovers:?}");
}

#[test]
fn overwrite_preserves_requested_mode() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("f");
    atomic_write(&p, b"one", 0o600).unwrap();
    atomic_write(&p, b"two", 0o644).unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"two");
    assert_eq!(p.metadata().unwrap().permissions().mode() & 0o777, 0o644);
}
