//! Known-answer + tamper tests for detached minisign verification.
//! Fixtures: throwaway keypair under tests/fixtures/minisign/ (TEST-ONLY,
//! generated at build time by the reviewer — never used for real releases).

use node111::minisign::verify_detached;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn valid_signature_verifies() {
    let data = fixture("manifest/channel-111-stable.toml");
    let sig = String::from_utf8(fixture("manifest/channel-111-stable.toml.minisig")).unwrap();
    let pubkey = String::from_utf8(fixture("minisign/test.pub")).unwrap();
    let pubkey_line = pubkey.lines().last().unwrap();
    verify_detached(&data, &sig, pubkey_line).expect("valid fixture must verify");
}

#[test]
fn tampered_payload_refused() {
    let mut data = fixture("manifest/channel-111-stable.toml");
    let n = data.len();
    data[n - 3] ^= 0x01;
    let sig = String::from_utf8(fixture("manifest/channel-111-stable.toml.minisig")).unwrap();
    let pubkey = String::from_utf8(fixture("minisign/test.pub")).unwrap();
    assert!(verify_detached(&data, &sig, pubkey.lines().last().unwrap()).is_err());
}

#[test]
fn wrong_key_refused() {
    let data = fixture("manifest/channel-111-stable.toml");
    let sig = String::from_utf8(fixture("manifest/channel-111-stable.toml.minisig")).unwrap();
    let other = String::from_utf8(fixture("minisign/other.pub")).unwrap();
    assert!(verify_detached(&data, &sig, other.lines().last().unwrap()).is_err());
}

#[test]
fn garbage_signature_refused() {
    let pubkey = String::from_utf8(fixture("minisign/test.pub")).unwrap();
    assert!(verify_detached(b"data", "not a signature", pubkey.lines().last().unwrap()).is_err());
}
