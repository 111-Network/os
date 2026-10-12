//! Manifest parse/validate/floor tests against the canonical fixture. The fixture
//! doubles as the contract T15's generator must round-trip through this parser.

use node111::floor::Floor;
use node111::manifest::ChannelManifest;

fn fixture_text() -> String {
    String::from_utf8(
        std::fs::read(format!(
            "{}/tests/fixtures/manifest/channel-111-stable.toml",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap()
}

fn fixture_bytes() -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/manifest/channel-111-stable.toml",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn sig() -> String {
    String::from_utf8(
        std::fs::read(format!(
            "{}/tests/fixtures/manifest/channel-111-stable.toml.minisig",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap()
}

fn pubkey() -> String {
    let raw = String::from_utf8(
        std::fs::read(format!(
            "{}/tests/fixtures/minisign/test.pub",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap();
    raw.lines().last().unwrap().to_string()
}

#[test]
fn canonical_fixture_parses() {
    let m = ChannelManifest::parse(&fixture_text()).unwrap();
    assert_eq!(m.manifest_version, 1);
    assert_eq!(m.channel(), "stable");
    assert_eq!(m.release().to_string(), "2026.10.12");
    assert_eq!(m.date().to_string(), "2026.10.12");
    assert_eq!(m.min_upgrade_from().to_string(), "2026.10.01");

    let x86 = m.binary_for("x86_64-unknown-linux-musl").unwrap();
    assert_eq!(x86.artifact.size, 5242880);
    assert!(m.binary_for("aarch64-unknown-linux-musl").is_some());
    assert!(m.binary_for("armv7-unknown-linux-musleabihf").is_none());

    assert!(m.payload_for("x86_64-unknown-linux-musl").is_some());
    assert!(m.payload_for("riscv64").is_none());

    assert_eq!(m.termux_apks().len(), 3);
    let names: Vec<&str> = m.termux_apks().iter().map(|a| a.name.as_str()).collect();
    assert!(
        names.contains(&"termux")
            && names.contains(&"termux-boot")
            && names.contains(&"termux-api")
    );

    // Spec-pinned minimum deb closure.
    let debs: Vec<&str> = m.termux_debs().iter().map(|d| d.name.as_str()).collect();
    for need in [
        "openssh",
        "termux-services",
        "cronie",
        "python",
        "python-cryptography",
    ] {
        assert!(debs.contains(&need), "closure missing {need}");
    }

    assert_eq!(m.rns_lock().size, 2048);
    assert_eq!(m.kb().size, 1048576);
    assert_eq!(m.keychain().len(), 2);
    assert_eq!(m.keychain()[0].generation, 0);
    assert!(m.keychain()[0].signature_by_previous.is_none());
    assert!(m.keychain()[1].signature_by_previous.is_some());
}

#[test]
fn parse_verified_happy_path_and_floor_refusal() {
    let floor = Floor::initial();
    let m = ChannelManifest::parse_verified(&fixture_bytes(), &sig(), &pubkey(), &floor)
        .expect("signed fixture over empty floor must verify");
    assert_eq!(m.release().to_string(), "2026.10.12");

    // Anti-replay: a VALIDLY SIGNED manifest at/below the floor is still refused.
    let mut high = Floor::initial();
    high.highest_release = "2026.10.13".into();
    assert!(ChannelManifest::parse_verified(&fixture_bytes(), &sig(), &pubkey(), &high).is_err());

    let mut high_gen = Floor::initial();
    high_gen.highest_keychain_generation = 1; // fixture's max gen is 1 -> equal is fine
    assert!(
        ChannelManifest::parse_verified(&fixture_bytes(), &sig(), &pubkey(), &high_gen).is_ok()
    );
    high_gen.highest_keychain_generation = 2;
    assert!(
        ChannelManifest::parse_verified(&fixture_bytes(), &sig(), &pubkey(), &high_gen).is_err()
    );
}

/// T15 round-trip: the generator's output parses through the T2 parser.
#[test]
fn generated_manifest_parses() {
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("python3 absent — skipping generator round-trip");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let arts = dir.path().join("artifacts");
    std::fs::create_dir_all(&arts).unwrap();
    std::fs::write(arts.join("111-x86_64-unknown-linux-musl"), b"fake binary").unwrap();
    std::fs::write(
        arts.join("payload-x86_64-unknown-linux-musl.tar.gz"),
        b"fake payload",
    )
    .unwrap();
    std::fs::write(
        arts.join("rns-lock.txt"),
        b"rns==1.5.6 --hash=sha256:ab
",
    )
    .unwrap();
    std::fs::write(arts.join("kb.tar.gz"), b"fake kb").unwrap();
    let out = dir.path().join("channel-111-stable.toml");
    let status = std::process::Command::new("python3")
        .arg(format!(
            "{}/../../manifest/gen-manifest.py",
            env!("CARGO_MANIFEST_DIR")
        ))
        .args([
            "--version",
            "2026.10.12",
            "--artifacts-dir",
            arts.to_str().unwrap(),
            "--min-upgrade-from",
            "2026.10.01",
            "--url-base",
            "https://example.invalid/v2026.10.12",
            "--keychain-gen",
            "0",
            "--pubkey",
            "RWTestKey000000000000000000000000000000000000000=",
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "gen-manifest failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let text = std::fs::read_to_string(&out).unwrap();
    let m = ChannelManifest::parse(&text).unwrap();
    assert_eq!(m.release().to_string(), "2026.10.12");
    assert!(m.binary_for("x86_64-unknown-linux-musl").is_some());
    assert!(m.payload_for("x86_64-unknown-linux-musl").is_some());
    let payload = m.payload_for("x86_64-unknown-linux-musl").unwrap();
    assert_eq!(
        payload.sha256,
        node111::util::hash::sha256_bytes(b"fake payload")
    );
    assert_eq!(payload.size, 12);
    assert_eq!(m.keychain().len(), 1);
}

#[test]
fn tamper_negatives() {
    let base = fixture_text();

    // Uppercase hex sha256 must be rejected.
    let upper = base.replacen(&"a".repeat(64), &"A".repeat(64), 1);
    assert!(ChannelManifest::parse(&upper).is_err());

    // Short sha.
    let short = base.replacen(&"a".repeat(64), "abcd", 1);
    assert!(ChannelManifest::parse(&short).is_err());

    // Non-http(s) URL scheme.
    let ftp = base.replacen(
        "https://example.invalid/v2026.10.12/111-x86_64-unknown-linux-musl",
        "ftp://example.invalid/111",
        1,
    );
    assert!(ChannelManifest::parse(&ftp).is_err());

    // Zero size.
    let zero = base.replacen("size = 5242880", "size = 0", 1);
    assert!(ChannelManifest::parse(&zero).is_err());

    // Unknown top-level field.
    let extra = format!("{base}\nsurprise = 1\n");
    assert!(ChannelManifest::parse(&extra).is_err());

    // Bad manifest-version.
    let v2 = base.replacen("manifest-version = 1", "manifest-version = 2", 1);
    assert!(ChannelManifest::parse(&v2).is_err());

    // Bad calver.
    let bad_date = base.replacen("date = \"2026.10.12\"", "date = \"2026.13.99\"", 1);
    assert!(ChannelManifest::parse(&bad_date).is_err());

    // Keychain: generation-1 without signature_by_previous.
    let no_sig = base.replace(
        "signature_by_previous = \"untrusted comment: fixture\\nplaceholder\"\n",
        "",
    );
    assert!(ChannelManifest::parse(&no_sig).is_err());
}
