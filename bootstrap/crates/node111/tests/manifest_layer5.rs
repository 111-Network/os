//! Layer 5 manifest entries (D20; `os/messaging/README.md` §2, §13, §14, §19 L1): the
//! per-target `meshmud` binaries, the Meshtastic web bundle and the presence venv
//! lock — parse rules, the payload-tree cross-check (`verify_layer5_tree`), the
//! bundle hash's byte-compatibility with `scripts/meshtastic-web-build.sh`, and the
//! generator round-trip (`manifest/gen-manifest.py`).

use node111::manifest::{bundle_tree_sha256, layer5, ChannelManifest};
use node111::util::hash::sha256_bytes;
use std::path::{Path, PathBuf};
use std::process::Command;

const X86: &str = "x86_64-unknown-linux-musl";
const ARM: &str = "aarch64-unknown-linux-musl";
const MESHMUD_X86: &[u8] = b"meshmud v1.0.0 linux/amd64 static";
const MESHMUD_ARM: &[u8] = b"meshmud v1.0.0 linux/arm64 static";
const LOCK: &[u8] = b"meshtastic==2.7.11 \\\n    --hash=sha256:0000\n";

/// Bundle files, chosen to exercise the C-locale order: uppercase before
/// lowercase, `.` (0x2e) before `/` (0x2f), nested directories.
const BUNDLE: &[(&str, &[u8])] = &[
    (
        "index.html",
        b"<!doctype html><script src=\"/assets/app.js\"></script>",
    ),
    ("assets/app.js", b"fetch(\"/assets/db.wasm\")"),
    ("assets/db.wasm", b"\0asm\x01\0\0\0"),
    ("assets/fonts/inter.woff2", b"wOF2"),
    ("assets.txt", b"sorts before assets/"),
    ("Zebra.txt", b"sorts before every lowercase name"),
];

fn fixture() -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/manifest/channel-111-stable.toml",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn write_bundle(dir: &Path) {
    for (rel, bytes) in BUNDLE {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
}

/// A scratch bundle's tree hash (the value CI pins as WEB_TREE_SHA256).
fn bundle_hash() -> String {
    let d = tempfile::tempdir().unwrap();
    write_bundle(d.path());
    bundle_tree_sha256(d.path()).unwrap()
}

fn meshmud_section(triple: &str, bytes: &[u8]) -> String {
    format!(
        "[pkg.meshmud.target.{triple}]\nurl = \"https://example.invalid/v2026.10.12/meshmud-{triple}\"\nsha256 = \"{}\"\nsize = {}\n\n",
        sha256_bytes(bytes),
        bytes.len()
    )
}

fn bundle_section(tree_sha256: &str) -> String {
    format!(
        "[bundle.meshtastic-web]\nurl = \"https://example.invalid/v2026.10.12/meshtastic-web-v2.7.2.tar\"\nsha256 = \"{}\"\nsize = 40960\ntree-sha256 = \"{tree_sha256}\"\n\n",
        "1".repeat(64)
    )
}

fn lock_section(bytes: &[u8]) -> String {
    format!(
        "[presence-lock]\nurl = \"https://example.invalid/v2026.10.12/meshtastic-presence-lock.txt\"\nsha256 = \"{}\"\nsize = {}\n\n",
        sha256_bytes(bytes),
        bytes.len()
    )
}

/// The canonical fixture plus every Layer 5 section.
fn full_manifest() -> String {
    format!(
        "{}\n{}{}{}{}",
        fixture(),
        meshmud_section(X86, MESHMUD_X86),
        meshmud_section(ARM, MESHMUD_ARM),
        bundle_section(&bundle_hash()),
        lock_section(LOCK)
    )
}

/// An x86_64 payload tree carrying all three Layer 5 assets.
fn full_tree() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::write(root.join("bin/111"), b"node binary").unwrap();
    std::fs::write(root.join(layer5::MESHMUD_BIN), MESHMUD_X86).unwrap();
    write_bundle(&root.join(layer5::PANELS_DIR).join("meshtastic-web"));
    std::fs::create_dir_all(root.join(layer5::PRESENCE_DIR)).unwrap();
    std::fs::write(root.join(layer5::PRESENCE_LOCK), LOCK).unwrap();
    std::fs::write(
        root.join(layer5::PRESENCE_DIR)
            .join("meshtastic_presence.py"),
        b"# reader",
    )
    .unwrap();
    t
}

fn refused(m: &ChannelManifest, tree: &Path, triple: &str, needle: &str) {
    let err = m
        .verify_layer5_tree(tree, triple)
        .expect_err("cross-check must refuse")
        .to_string();
    assert!(err.contains(needle), "wanted {needle:?} in: {err}");
}

#[test]
fn canonical_fixture_ships_no_layer5_assets() {
    let m = ChannelManifest::parse(&fixture()).unwrap();
    assert!(m.meshmud_for(X86).is_none());
    assert!(m.bundles().is_empty());
    assert!(m.bundle("meshtastic-web").is_none());
    assert!(m.presence_lock().is_none());
    // A plain payload (bin/111 + share/kb) passes; any Layer 5 asset in it is unpinned.
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join("bin")).unwrap();
    std::fs::write(t.path().join("bin/111"), b"node binary").unwrap();
    m.verify_layer5_tree(t.path(), X86).unwrap();
    let full = full_tree();
    refused(&m, full.path(), X86, "pins no meshmud");
}

#[test]
fn layer5_sections_parse() {
    let m = ChannelManifest::parse(&full_manifest()).unwrap();
    let x = m.meshmud_for(X86).unwrap();
    assert_eq!(x.artifact.sha256, sha256_bytes(MESHMUD_X86));
    assert_eq!(x.artifact.size, MESHMUD_X86.len() as u64);
    assert_eq!(
        m.meshmud_for(ARM).unwrap().artifact.sha256,
        sha256_bytes(MESHMUD_ARM)
    );
    assert!(m.meshmud_for("riscv64gc-unknown-linux-musl").is_none());
    // `binary_for` still answers the `111` package only.
    assert_eq!(m.binary_for(X86).unwrap().name, "111");
    let b = m.bundle("meshtastic-web").unwrap();
    assert_eq!(b.tree_sha256, bundle_hash());
    assert_eq!(b.artifact.size, 40960);
    assert_eq!(m.bundles().len(), 1);
    let lock = m.presence_lock().unwrap();
    assert_eq!(lock.sha256, sha256_bytes(LOCK));
    assert_eq!(lock.name, "meshtastic-presence-lock.txt");
    // Paths the daemon reads (meshtastic/mod.rs, panels manifest).
    assert_eq!(layer5::MESHMUD_BIN, "bin/meshmud");
    assert_eq!(layer5::PRESENCE_VENV, "venvs/presence");
}

#[test]
fn layer5_parse_refusals() {
    let base = fixture();
    let ok_tree = bundle_hash();
    let cases: Vec<(&str, String)> = vec![
        (
            "unknown package",
            format!(
                "{base}\n{}",
                meshmud_section(X86, MESHMUD_X86).replace("pkg.meshmud", "pkg.rnsd")
            ),
        ),
        (
            "meshmud without a 111 binary / payload for the target",
            format!(
                "{base}\n{}",
                meshmud_section("riscv64gc-unknown-linux-musl", MESHMUD_X86)
            ),
        ),
        (
            "unknown bundle",
            format!(
                "{base}\n{}",
                bundle_section(&ok_tree).replace("bundle.meshtastic-web", "bundle.other-web")
            ),
        ),
        (
            "bundle without tree-sha256",
            format!(
                "{base}\n{}",
                bundle_section(&ok_tree).replace(&format!("tree-sha256 = \"{ok_tree}\"\n"), "")
            ),
        ),
        (
            "uppercase tree-sha256",
            format!("{base}\n{}", bundle_section(&ok_tree.to_uppercase())),
        ),
        (
            "short tree-sha256",
            format!("{base}\n{}", bundle_section("abcd")),
        ),
        (
            "unknown bundle field",
            format!(
                "{base}\n{}\ndir = \"panels/elsewhere\"\n",
                bundle_section(&ok_tree).trim_end()
            ),
        ),
        (
            "bundle with zero size",
            format!(
                "{base}\n{}",
                bundle_section(&ok_tree).replace("size = 40960", "size = 0")
            ),
        ),
        (
            "presence-lock with a bad sha256",
            format!(
                "{base}\n{}",
                lock_section(LOCK).replace(&sha256_bytes(LOCK), "nothex")
            ),
        ),
        (
            "presence-lock with a non-http url",
            format!(
                "{base}\n{}",
                lock_section(LOCK).replace("https://", "file://")
            ),
        ),
    ];
    for (what, text) in cases {
        assert!(
            ChannelManifest::parse(&text).is_err(),
            "{what}: must be refused"
        );
    }
    // Sanity: the unmodified sections parse.
    ChannelManifest::parse(&full_manifest()).unwrap();
}

#[test]
fn payload_tree_cross_check() {
    let m = ChannelManifest::parse(&full_manifest()).unwrap();
    let t = full_tree();
    m.verify_layer5_tree(t.path(), X86).unwrap();
    // The x86_64 payload carries the amd64 meshmud: an aarch64 check refuses it.
    refused(&m, t.path(), ARM, "bin/meshmud sha256");
}

#[test]
fn meshmud_tampering_is_refused() {
    let m = ChannelManifest::parse(&full_manifest()).unwrap();

    let t = full_tree();
    let mut altered = MESHMUD_X86.to_vec();
    altered[0] ^= 1; // same size, other bytes
    std::fs::write(t.path().join(layer5::MESHMUD_BIN), &altered).unwrap();
    refused(&m, t.path(), X86, "bin/meshmud sha256");

    let t = full_tree();
    std::fs::write(t.path().join(layer5::MESHMUD_BIN), b"short").unwrap();
    refused(&m, t.path(), X86, "bin/meshmud size");

    let t = full_tree();
    std::fs::remove_file(t.path().join(layer5::MESHMUD_BIN)).unwrap();
    refused(&m, t.path(), X86, "lacks pinned bin/meshmud");

    // A link to the right bytes is still refused (payloads are plain trees).
    let t = full_tree();
    let real = t.path().join("bin/meshmud.real");
    std::fs::write(&real, MESHMUD_X86).unwrap();
    std::fs::remove_file(t.path().join(layer5::MESHMUD_BIN)).unwrap();
    std::os::unix::fs::symlink(&real, t.path().join(layer5::MESHMUD_BIN)).unwrap();
    refused(&m, t.path(), X86, "not a regular file");
}

#[test]
fn bundle_tampering_is_refused() {
    let m = ChannelManifest::parse(&full_manifest()).unwrap();
    let web = |t: &tempfile::TempDir| t.path().join("panels/meshtastic-web");

    let t = full_tree();
    std::fs::write(web(&t).join("assets/extra.js"), b"injected").unwrap();
    refused(&m, t.path(), X86, "tree hash");

    let t = full_tree();
    std::fs::remove_file(web(&t).join("assets/db.wasm")).unwrap();
    refused(&m, t.path(), X86, "tree hash");

    let t = full_tree();
    std::fs::write(web(&t).join("index.html"), b"<!doctype html>altered").unwrap();
    refused(&m, t.path(), X86, "tree hash");

    let t = full_tree();
    std::os::unix::fs::symlink("/etc/passwd", web(&t).join("assets/passwd")).unwrap();
    refused(&m, t.path(), X86, "link or special file");

    let t = full_tree();
    std::fs::remove_dir_all(web(&t)).unwrap();
    refused(&m, t.path(), X86, "lacks pinned bundle");

    // The bundle dir itself as a link to an identical tree: refused.
    let t = full_tree();
    let elsewhere = t.path().join("elsewhere");
    std::fs::rename(web(&t), &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, web(&t)).unwrap();
    refused(&m, t.path(), X86, "not a directory");

    // An unpinned bundle beside the pinned one.
    let t = full_tree();
    write_bundle(&t.path().join("panels/other-web"));
    refused(&m, t.path(), X86, "pins no such bundle");

    // A bundle the manifest does not pin at all.
    let no_bundle = ChannelManifest::parse(&format!(
        "{}\n{}{}{}",
        fixture(),
        meshmud_section(X86, MESHMUD_X86),
        meshmud_section(ARM, MESHMUD_ARM),
        lock_section(LOCK)
    ))
    .unwrap();
    let t = full_tree();
    refused(&no_bundle, t.path(), X86, "pins no such bundle");
    std::fs::remove_dir_all(t.path().join("panels")).unwrap();
    no_bundle.verify_layer5_tree(t.path(), X86).unwrap();
}

#[test]
fn presence_lock_tampering_is_refused() {
    let m = ChannelManifest::parse(&full_manifest()).unwrap();

    let t = full_tree();
    let mut altered = LOCK.to_vec();
    let last = altered.len() - 2;
    altered[last] = b'1';
    std::fs::write(t.path().join(layer5::PRESENCE_LOCK), &altered).unwrap();
    refused(&m, t.path(), X86, "meshtastic-presence-lock.txt sha256");

    let t = full_tree();
    std::fs::remove_file(t.path().join(layer5::PRESENCE_LOCK)).unwrap();
    refused(&m, t.path(), X86, "lacks pinned share/meshtastic-presence");

    // Shipped but not pinned.
    let unpinned = ChannelManifest::parse(&format!(
        "{}\n{}{}{}",
        fixture(),
        meshmud_section(X86, MESHMUD_X86),
        meshmud_section(ARM, MESHMUD_ARM),
        bundle_section(&bundle_hash())
    ))
    .unwrap();
    let t = full_tree();
    refused(&unpinned, t.path(), X86, "pins no presence-lock");
}

#[test]
fn bundle_tree_hash_refuses_unhashable_names() {
    let d = tempfile::tempdir().unwrap();
    write_bundle(d.path());
    std::fs::write(d.path().join("back\\slash.js"), b"x").unwrap();
    assert!(bundle_tree_sha256(d.path()).is_err());

    let d = tempfile::tempdir().unwrap();
    write_bundle(d.path());
    std::fs::write(d.path().join("new\nline.js"), b"x").unwrap();
    assert!(bundle_tree_sha256(d.path()).is_err());
}

/// The Rust bundle hash must equal the hash of the `.tree` file that
/// `scripts/meshtastic-web-build.sh` writes (its `tree_manifest` function, taken
/// verbatim from the script) — otherwise a CI pin could never verify on a node.
#[test]
fn bundle_tree_hash_matches_the_build_script() {
    for tool in ["sh", "find", "sort", "xargs", "sha256sum", "sed"] {
        if Command::new("sh")
            .args(["-c", &format!("command -v {tool}")])
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            eprintln!("{tool} absent — skipping the build-script compatibility check");
            return;
        }
    }
    let script = std::fs::read_to_string(format!(
        "{}/../../scripts/meshtastic-web-build.sh",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let start = script
        .find("tree_manifest() {")
        .expect("meshtastic-web-build.sh defines tree_manifest");
    let len = script[start..]
        .find("\n}\n")
        .expect("tree_manifest ends with a lone `}`");
    let func = &script[start..start + len + 3];

    let d = tempfile::tempdir().unwrap();
    let bundle = d.path().join("meshtastic-web");
    write_bundle(&bundle);
    let runner: PathBuf = d.path().join("tree.sh");
    std::fs::write(
        &runner,
        format!("set -eu\n{func}tree_manifest \"$1\" \"$2\"\n"),
    )
    .unwrap();
    let out_file = d.path().join("bundle.tree");
    let status = Command::new("sh")
        .arg(&runner)
        .arg(&bundle)
        .arg(&out_file)
        .env("LC_ALL", "C")
        .status()
        .unwrap();
    assert!(status.success(), "tree_manifest failed");
    let listing = std::fs::read(&out_file).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&listing).lines().count(),
        BUNDLE.len()
    );
    assert_eq!(bundle_tree_sha256(&bundle).unwrap(), sha256_bytes(&listing));
}

/// T15 round-trip for the Layer 5 artifacts: the generator takes the build scripts'
/// own output names and its pins verify a payload tree built from the same bytes.
#[test]
fn generator_round_trip_with_layer5() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("python3 absent — skipping generator round-trip");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let arts = dir.path().join("artifacts");
    std::fs::create_dir_all(&arts).unwrap();
    for (name, bytes) in [
        ("111-x86_64-unknown-linux-musl", &b"fake binary x86"[..]),
        ("111-aarch64-unknown-linux-musl", b"fake binary arm"),
        (
            "payload-x86_64-unknown-linux-musl.tar.gz",
            b"fake payload x86",
        ),
        (
            "payload-aarch64-unknown-linux-musl.tar.gz",
            b"fake payload arm",
        ),
        ("rns-lock.txt", b"rns==1.5.6 --hash=sha256:ab\n"),
        ("kb.tar.gz", b"fake kb"),
        ("meshmud-v1.0.0-linux-amd64", MESHMUD_X86),
        ("meshmud-v1.0.0-linux-arm64", MESHMUD_ARM),
        ("meshtastic-web-v2.7.2.tar", b"deterministic tar bytes"),
        ("meshtastic-presence-lock.txt", LOCK),
    ] {
        std::fs::write(arts.join(name), bytes).unwrap();
    }
    // The .tree file exactly as the build script writes it for this bundle.
    let bundle = dir.path().join("bundle");
    write_bundle(&bundle);
    let mut rows: Vec<(String, String)> = BUNDLE
        .iter()
        .map(|(rel, bytes)| (rel.to_string(), sha256_bytes(bytes)))
        .collect();
    rows.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let tree_text: String = rows
        .iter()
        .map(|(rel, sha)| format!("{sha}  {rel}\n"))
        .collect();
    std::fs::write(arts.join("meshtastic-web-v2.7.2.tree"), &tree_text).unwrap();

    let gen = |out: &Path| {
        Command::new("python3")
            .arg("-I")
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
                "--pubkey",
                "RWTestKey000000000000000000000000000000000000000=",
                "--out",
                out.to_str().unwrap(),
            ])
            .output()
            .unwrap()
    };
    let out = dir.path().join("channel-111-stable.toml");
    let o = gen(&out);
    assert!(
        o.status.success(),
        "gen-manifest failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let m = ChannelManifest::parse(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let x = m.meshmud_for(X86).unwrap();
    assert_eq!(x.artifact.sha256, sha256_bytes(MESHMUD_X86));
    assert!(x.artifact.url.ends_with("/meshmud-v1.0.0-linux-amd64"));
    assert_eq!(
        m.meshmud_for(ARM).unwrap().artifact.sha256,
        sha256_bytes(MESHMUD_ARM)
    );
    let b = m.bundle("meshtastic-web").unwrap();
    assert_eq!(b.tree_sha256, sha256_bytes(tree_text.as_bytes()));
    assert_eq!(b.tree_sha256, bundle_tree_sha256(&bundle).unwrap());
    assert_eq!(b.artifact.sha256, sha256_bytes(b"deterministic tar bytes"));
    assert_eq!(m.presence_lock().unwrap().sha256, sha256_bytes(LOCK));
    // …and those pins verify a payload tree built from the same bytes.
    m.verify_layer5_tree(full_tree().path(), X86).unwrap();

    // A .tar without its .tree is refused by the generator.
    std::fs::remove_file(arts.join("meshtastic-web-v2.7.2.tree")).unwrap();
    let o = gen(&dir.path().join("broken.toml"));
    assert!(
        !o.status.success(),
        "a bundle tar without its tree must fail"
    );
    // Two meshmud builds for one arch are ambiguous.
    std::fs::write(arts.join("meshtastic-web-v2.7.2.tree"), &tree_text).unwrap();
    std::fs::write(arts.join("meshmud-v0.9.0-linux-amd64"), b"old").unwrap();
    let o = gen(&dir.path().join("ambiguous.toml"));
    assert!(!o.status.success(), "two meshmud amd64 builds must fail");
}
