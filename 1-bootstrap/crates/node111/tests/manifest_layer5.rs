//! Layer 5 manifest entries (D20; `os/5-messaging/README.md` §2, §13, §14, §19 L1): the
//! per-target `meshmud` binaries, the Meshtastic web bundle and the presence venv
//! lock — parse rules (open package/bundle names), the payload-tree cross-check
//! (`verify_layer5_tree`, no link anywhere on the way), the bundle hash's
//! byte-compatibility with `scripts/meshtastic-web-build.sh`, and the generator
//! (`manifest/gen-manifest.py`): its round-trip and its refusal of payloads that
//! disagree with the pins it writes.
//!
//! The tests that drive external tools (`sh`, coreutils, `python3`) fail when a tool
//! is missing — a gate test never passes by skipping silently.

mod common;

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
            "meshmud without a 111 binary / payload for the target",
            format!(
                "{base}\n{}",
                meshmud_section("riscv64gc-unknown-linux-musl", MESHMUD_X86)
            ),
        ),
        (
            "bundle name that climbs out of panels/",
            format!(
                "{base}\n{}",
                bundle_section(&ok_tree).replace("bundle.meshtastic-web", "bundle.\"../escape\"")
            ),
        ),
        (
            "bundle name with a path separator",
            format!(
                "{base}\n{}",
                bundle_section(&ok_tree).replace("bundle.meshtastic-web", "bundle.\"a/b\"")
            ),
        ),
        (
            "hidden bundle name",
            format!(
                "{base}\n{}",
                bundle_section(&ok_tree).replace("bundle.meshtastic-web", "bundle.\".web\"")
            ),
        ),
        (
            "uppercase bundle name",
            format!(
                "{base}\n{}",
                bundle_section(&ok_tree).replace("bundle.meshtastic-web", "bundle.Web")
            ),
        ),
        (
            "unknown package with a bad pin",
            format!(
                "{base}\n{}",
                meshmud_section(X86, MESHMUD_X86)
                    .replace("pkg.meshmud", "pkg.kiss-modem")
                    .replace("https://", "file://")
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

/// Open names (Wave 2 review B1 m6, lead decision): a package or bundle this
/// binary does not know parses and is otherwise ignored, so a later release can add
/// one without stranding nodes; the closure stays in `verify_layer5_tree`.
#[test]
fn unknown_names_parse_and_are_ignored() {
    let text = format!(
        "{}{}{}",
        full_manifest(),
        meshmud_section(X86, b"a KISS modem image").replace("pkg.meshmud", "pkg.kiss-modem"),
        bundle_section(&bundle_hash()).replace("bundle.meshtastic-web", "bundle.mesh-map")
    );
    let m = ChannelManifest::parse(&text).expect("unknown names must parse");
    // Ignored by every lookup of a known name…
    assert_eq!(m.binary_for(X86).unwrap().name, "111");
    assert_eq!(
        m.meshmud_for(X86).unwrap().artifact.sha256,
        sha256_bytes(MESHMUD_X86)
    );
    assert_eq!(
        m.bundle("meshtastic-web").unwrap().tree_sha256,
        bundle_hash()
    );
    assert!(m.packages().iter().any(|p| p.name == "kiss-modem"));
    // …but a pinned bundle is a pinned bundle: it must sit at panels/<name>/.
    let t = full_tree();
    refused(&m, t.path(), X86, "lacks pinned bundle panels/mesh-map");
    write_bundle(&t.path().join("panels/mesh-map"));
    m.verify_layer5_tree(t.path(), X86).unwrap();
    std::fs::write(t.path().join("panels/mesh-map/extra.js"), b"x").unwrap();
    refused(&m, t.path(), X86, "panels/mesh-map/ tree hash");
    // Nothing unpinned under panels/, whatever this binary knows.
    let t = full_tree();
    write_bundle(&t.path().join("panels/mesh-map"));
    write_bundle(&t.path().join("panels/other-web"));
    refused(&m, t.path(), X86, "pins no such bundle");
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

/// Every component between the tree and a target is lstat'ed (Wave 2 review B1
/// m7): a linked parent can neither supply the pinned bytes nor hide an asset.
#[test]
fn linked_components_are_refused() {
    use std::os::unix::fs::symlink;
    let m = ChannelManifest::parse(&full_manifest()).unwrap();
    let relink = |t: &tempfile::TempDir, rel: &str| {
        let real = t.path().join(format!("real-{}", rel.replace('/', "-")));
        std::fs::rename(t.path().join(rel), &real).unwrap();
        symlink(&real, t.path().join(rel)).unwrap();
    };

    // bin/ as a link to a directory holding the exact pinned meshmud.
    let t = full_tree();
    relink(&t, "bin");
    refused(&m, t.path(), X86, "payload bin is a link");

    // share/ and share/meshtastic-presence/ as links to the right lock.
    let t = full_tree();
    relink(&t, "share");
    refused(&m, t.path(), X86, "payload share is a link");
    let t = full_tree();
    relink(&t, layer5::PRESENCE_DIR);
    refused(
        &m,
        t.path(),
        X86,
        "payload share/meshtastic-presence is a link",
    );

    // panels/ as a link is refused before any bundle is hashed (an unhashable
    // name inside would otherwise be the first error).
    let t = full_tree();
    std::fs::write(t.path().join("panels/meshtastic-web/back\\slash.js"), b"x").unwrap();
    relink(&t, "panels");
    refused(&m, t.path(), X86, "payload panels is not a directory");

    // Absence is judged without following a linked parent either.
    let no_meshmud = ChannelManifest::parse(&format!(
        "{}\n{}{}",
        fixture(),
        bundle_section(&bundle_hash()),
        lock_section(LOCK)
    ))
    .unwrap();
    let t = full_tree();
    std::fs::remove_file(t.path().join(layer5::MESHMUD_BIN)).unwrap();
    no_meshmud.verify_layer5_tree(t.path(), X86).unwrap();
    relink(&t, "bin");
    refused(&no_meshmud, t.path(), X86, "payload bin is a link");

    // A parent that is not a directory at all.
    let t = full_tree();
    std::fs::remove_dir_all(t.path().join(layer5::PRESENCE_DIR)).unwrap();
    std::fs::write(t.path().join(layer5::PRESENCE_DIR), b"not a dir").unwrap();
    refused(
        &m,
        t.path(),
        X86,
        "payload share/meshtastic-presence is not a directory",
    );
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

/// A gate test's external tool must be on PATH: the test fails rather than
/// passing by skipping (Wave 2 review B1 m8).
fn require_tool(tool: &str) {
    let found = Command::new("sh")
        .args(["-c", &format!("command -v {tool}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    assert!(
        found,
        "`{tool}` is required by this gate test and is not on PATH — install it (the test \
         fails instead of skipping, so the check it carries never goes silently unrun)"
    );
}

/// The Rust bundle hash must equal the hash of the `.tree` file that
/// `scripts/meshtastic-web-build.sh` writes (its `tree_manifest` function, taken
/// verbatim from the script) — otherwise a CI pin could never verify on a node.
#[test]
fn bundle_tree_hash_matches_the_build_script() {
    for tool in ["sh", "find", "sort", "xargs", "sha256sum", "sed"] {
        require_tool(tool);
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

/// One tar member with an exact name: the tar crate strips a leading `./`, while
/// the build script's `tar -C dir .` keeps it (and payloads must not have it).
fn raw_member(b: &mut tar::Builder<&mut Vec<u8>>, name: &str, data: &[u8], dir: bool) {
    let mut h = tar::Header::new_gnu();
    h.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
    h.set_size(data.len() as u64);
    h.set_mode(if dir { 0o755 } else { 0o644 });
    h.set_entry_type(if dir {
        tar::EntryType::Directory
    } else {
        tar::EntryType::Regular
    });
    h.set_cksum();
    b.append(&h, data).unwrap();
}

/// `meshtastic-web-<tag>.tar` as the build script writes it (`./`-prefixed names).
fn bundle_tar(extra: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut b = tar::Builder::new(&mut out);
        for d in ["./", "./assets/", "./assets/fonts/"] {
            raw_member(&mut b, d, b"", true);
        }
        for (rel, bytes) in BUNDLE.iter().chain(extra.iter()) {
            raw_member(&mut b, &format!("./{rel}"), bytes, false);
        }
        b.finish().unwrap();
    }
    out
}

/// The `.tree` file the build script writes for [`BUNDLE`].
fn tree_text() -> String {
    let mut rows: Vec<(String, String)> = BUNDLE
        .iter()
        .map(|(rel, bytes)| (rel.to_string(), sha256_bytes(bytes)))
        .collect();
    rows.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    rows.iter()
        .map(|(rel, sha)| format!("{sha}  {rel}\n"))
        .collect()
}

/// The files of a release payload beyond the base ones (bin/111, share/kb), as
/// (relpath, bytes): the given Layer 5 assets; `bundle` lists the files placed under
/// panels/meshtastic-web/.
fn layer5_files(
    meshmud: Option<&[u8]>,
    bundle: &[(&str, &[u8])],
    lock: bool,
    extra: &[(&str, &[u8])],
) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    if let Some(bytes) = meshmud {
        files.push((layer5::MESHMUD_BIN.into(), bytes.to_vec()));
    }
    for (rel, bytes) in bundle {
        files.push((format!("panels/meshtastic-web/{rel}"), bytes.to_vec()));
    }
    if lock {
        files.push((layer5::PRESENCE_LOCK.into(), LOCK.to_vec()));
        files.push((
            format!("{}/meshtastic_presence.py", layer5::PRESENCE_DIR),
            b"# reader".to_vec(),
        ));
    }
    for (rel, bytes) in extra {
        files.push((rel.to_string(), bytes.to_vec()));
    }
    files
}

/// A release payload (bin/111, share/kb, share/payload.manifest) carrying the
/// given Layer 5 assets; `bundle` lists the files placed under panels/meshtastic-web/.
fn layer5_payload(
    meshmud: Option<&[u8]>,
    bundle: &[(&str, &[u8])],
    lock: bool,
    extra: &[(&str, &[u8])],
) -> Vec<u8> {
    let files = layer5_files(meshmud, bundle, lock, extra);
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(r, b)| (r.as_str(), b.as_slice()))
        .collect();
    common::build_payload_with(&refs)
}

/// The full x86_64 Layer 5 payload with a `share/payload.manifest` that disagrees
/// with its files: `unlisted` paths are packed but not listed, `phantom` paths are
/// listed but not packed, `listing: false` packs no manifest (see
/// `common::build_payload_listing`).
fn layer5_payload_listing(
    unlisted: &[&str],
    phantom: &[(&str, &[u8])],
    listing: bool,
    extra: &[(&str, &[u8])],
) -> Vec<u8> {
    let files = layer5_files(Some(MESHMUD_X86), BUNDLE, true, extra);
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(r, b)| (r.as_str(), b.as_slice()))
        .collect();
    common::build_payload_listing(&refs, unlisted, phantom, listing)
}

/// A complete release artifacts dir, every Layer 5 pin consistent with both payloads.
fn release_artifacts() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let arts = dir.path().join("artifacts");
    std::fs::create_dir_all(&arts).unwrap();
    let payload_x86 = layer5_payload(Some(MESHMUD_X86), BUNDLE, true, &[]);
    let payload_arm = layer5_payload(Some(MESHMUD_ARM), BUNDLE, true, &[]);
    let tar = bundle_tar(&[]);
    let tree = tree_text();
    for (name, bytes) in [
        ("111-x86_64-unknown-linux-musl", &b"fake binary x86"[..]),
        ("111-aarch64-unknown-linux-musl", b"fake binary arm"),
        (
            "payload-x86_64-unknown-linux-musl.tar.gz",
            payload_x86.as_slice(),
        ),
        (
            "payload-aarch64-unknown-linux-musl.tar.gz",
            payload_arm.as_slice(),
        ),
        ("rns-lock.txt", b"rns==1.5.6 --hash=sha256:ab\n"),
        ("kb.tar.gz", b"fake kb"),
        ("meshmud-v1.0.0-linux-amd64", MESHMUD_X86),
        ("meshmud-v1.0.0-linux-arm64", MESHMUD_ARM),
        ("meshtastic-web-v2.7.2.tar", tar.as_slice()),
        ("meshtastic-web-v2.7.2.tree", tree.as_bytes()),
        ("meshtastic-presence-lock.txt", LOCK),
    ] {
        std::fs::write(arts.join(name), bytes).unwrap();
    }
    (dir, arts)
}

fn gen_manifest(arts: &Path, out: &Path) -> std::process::Output {
    require_tool("python3");
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
}

/// T15 round-trip for the Layer 5 artifacts: the generator takes the build scripts'
/// own output names, and its pins verify the payload a node extracts.
#[test]
fn generator_round_trip_with_layer5() {
    let (dir, arts) = release_artifacts();
    let out = dir.path().join("channel-111-stable.toml");
    let o = gen_manifest(&arts, &out);
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
    assert_eq!(b.tree_sha256, sha256_bytes(tree_text().as_bytes()));
    assert_eq!(b.tree_sha256, bundle_hash());
    assert_eq!(
        b.artifact.sha256,
        sha256_bytes(&std::fs::read(arts.join("meshtastic-web-v2.7.2.tar")).unwrap())
    );
    assert_eq!(m.presence_lock().unwrap().sha256, sha256_bytes(LOCK));
    // …and those pins verify the payload as a node stages it (pin check, safe
    // unpack, payload manifest) — for both targets.
    for (triple, file) in [
        (X86, "payload-x86_64-unknown-linux-musl.tar.gz"),
        (ARM, "payload-aarch64-unknown-linux-musl.tar.gz"),
    ] {
        let staging = tempfile::tempdir().unwrap();
        let staged = node111::daemon::updater::stage::stage_payload(
            &std::fs::read(arts.join(file)).unwrap(),
            m.payload_for(triple).unwrap(),
            staging.path(),
            &m.release(),
        )
        .unwrap();
        m.verify_layer5_tree(&staged.join("tree"), triple).unwrap();
    }
    m.verify_layer5_tree(full_tree().path(), X86).unwrap();
}

/// The generator refuses to write pins its own payloads would fail on a node
/// (Wave 2 review B1 m5), and the earlier artifact-set refusals still hold.
#[test]
fn generator_refuses_payloads_that_disagree_with_its_pins() {
    let x86_payload = "payload-x86_64-unknown-linux-musl.tar.gz";
    let altered_meshmud = layer5_payload(Some(b"other meshmud build"), BUNDLE, true, &[]);
    let short_bundle = layer5_payload(Some(MESHMUD_X86), &BUNDLE[1..], true, &[]);
    let extra_panel = layer5_payload(
        Some(MESHMUD_X86),
        BUNDLE,
        true,
        &[("panels/other-web/index.html", b"<!doctype html>")],
    );
    let other_lock = {
        let mut files: Vec<(&str, &[u8])> = vec![(layer5::MESHMUD_BIN, MESHMUD_X86)];
        let web: Vec<(String, &[u8])> = BUNDLE
            .iter()
            .map(|(r, b)| (format!("panels/meshtastic-web/{r}"), *b))
            .collect();
        files.extend(web.iter().map(|(r, b)| (r.as_str(), *b)));
        files.push((layer5::PRESENCE_LOCK, b"meshtastic==0.0.1\n"));
        common::build_payload_with(&files)
    };
    let dot_prefixed = {
        let mut tar_bytes = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_bytes);
            raw_member(&mut b, "./bin/111", b"node binary", false);
            b.finish().unwrap();
        }
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar_bytes).unwrap();
        gz.finish().unwrap()
    };
    let with_link = {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        {
            let mut b = tar::Builder::new(&mut gz);
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            h.set_cksum();
            b.append_link(&mut h, "bin/meshmud", "/usr/bin/meshmud")
                .unwrap();
            b.finish().unwrap();
        }
        gz.finish().unwrap()
    };

    type Mutate = Box<dyn Fn(&Path)>;
    let write = |name: &'static str, bytes: Vec<u8>| -> Mutate {
        Box::new(move |arts: &Path| std::fs::write(arts.join(name), &bytes).unwrap())
    };
    let remove = |name: &'static str| -> Mutate {
        Box::new(move |arts: &Path| std::fs::remove_file(arts.join(name)).unwrap())
    };
    let cases: Vec<(&str, Mutate, &str)> = vec![
        (
            "payload meshmud differs from the pinned build",
            write(x86_payload, altered_meshmud),
            "bin/meshmud (sha256",
        ),
        (
            "payload bundle tree differs from the pinned .tree",
            write(x86_payload, short_bundle),
            "panels/meshtastic-web/ tree",
        ),
        (
            "payload carries an unpinned bundle",
            write(x86_payload, extra_panel),
            "panels/other-web but the manifest pins no such bundle",
        ),
        (
            "payload presence lock differs from the pinned lock",
            write(x86_payload, other_lock),
            "meshtastic-presence-lock.txt (sha256",
        ),
        (
            "payload carries meshmud but no build is pinned for it",
            remove("meshmud-v1.0.0-linux-amd64"),
            "no meshmud build for x86_64-unknown-linux-musl is pinned",
        ),
        (
            "payload carries the presence lock but none is pinned",
            remove("meshtastic-presence-lock.txt"),
            "no presence lock is pinned",
        ),
        (
            "meshmud build for a triple with no payload",
            remove("payload-aarch64-unknown-linux-musl.tar.gz"),
            "payload-aarch64-unknown-linux-musl.tar.gz is missing",
        ),
        (
            "meshmud build for a triple with no 111 binary",
            remove("111-aarch64-unknown-linux-musl"),
            "111-aarch64-unknown-linux-musl is missing",
        ),
        (
            "the .tree does not describe the .tar beside it",
            write(
                "meshtastic-web-v2.7.2.tar",
                bundle_tar(&[("assets/late.js", b"added after the .tree")]),
            ),
            "does not match the listing of meshtastic-web-v2.7.2.tar",
        ),
        (
            "an unreadable payload while Layer 5 pins are written",
            write(x86_payload, b"fake payload".to_vec()),
            "not a readable .tar.gz",
        ),
        (
            "a payload member the node refuses (./ prefix)",
            write(x86_payload, dot_prefixed),
            "starts with './'",
        ),
        (
            "a link in the payload",
            write(x86_payload, with_link),
            "link or special file",
        ),
        (
            "a bundle .tar without its .tree",
            remove("meshtastic-web-v2.7.2.tree"),
            "must both be present",
        ),
        (
            "two meshmud builds for one arch",
            write("meshmud-v0.9.0-linux-amd64", b"old".to_vec()),
            "more than one artifact matches",
        ),
    ];
    for (what, mutate, needle) in cases {
        let (dir, arts) = release_artifacts();
        mutate(&arts);
        let out = dir.path().join("refused.toml");
        let o = gen_manifest(&arts, &out);
        let stderr = String::from_utf8_lossy(&o.stderr);
        assert!(!o.status.success(), "{what}: gen-manifest must refuse");
        assert!(
            stderr.contains(needle),
            "{what}: wanted {needle:?} in: {stderr}"
        );
        assert!(!out.exists(), "{what}: no manifest may be written");
    }

    // No Layer 5 artifact at all: a payload that is not a tarball passes through
    // with a visible warning (nothing to cross-check; nodes refuse it at unpack).
    let (dir, arts) = release_artifacts();
    for name in [
        "meshmud-v1.0.0-linux-amd64",
        "meshmud-v1.0.0-linux-arm64",
        "meshtastic-web-v2.7.2.tar",
        "meshtastic-web-v2.7.2.tree",
        "meshtastic-presence-lock.txt",
        "payload-aarch64-unknown-linux-musl.tar.gz",
    ] {
        std::fs::remove_file(arts.join(name)).unwrap();
    }
    std::fs::write(arts.join(x86_payload), b"fake payload").unwrap();
    let out = dir.path().join("plain.toml");
    let o = gen_manifest(&arts, &out);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stderr).contains("warning"));
    let m = ChannelManifest::parse(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert!(m.meshmud_for(X86).is_none() && m.bundles().is_empty());
}

/// Release-time guard for the payload listing (DOCS agent gap; B1 reviewer deviation
/// 5): `update_guard.rs` re-verifies the current release at every start with
/// `verify_tree`, which hashes only the files `share/payload.manifest` lists, so the
/// generator refuses a payload whose listing misses a file the tarball carries (or
/// names one it does not), naming the first offending path.
#[test]
fn generator_refuses_an_incomplete_payload_listing() {
    let x86_payload = "payload-x86_64-unknown-linux-musl.tar.gz";
    let app_js = "panels/meshtastic-web/assets/app.js";
    let reader = "share/meshtastic-presence/meshtastic_presence.py";
    // Every file of the full x86_64 payload: listing none of them leaves nothing pinned.
    let every_file: Vec<String> = ["bin/111", "share/kb/llms.txt", "share/kb/reticulum.md"]
        .into_iter()
        .map(String::from)
        .chain(
            layer5_files(Some(MESHMUD_X86), BUNDLE, true, &[])
                .into_iter()
                .map(|(rel, _)| rel),
        )
        .collect();
    let every_ref: Vec<&str> = every_file.iter().map(String::as_str).collect();
    let no_app_js =
        format!("share/payload.manifest does not list {app_js}, which the tarball carries");
    let no_reader =
        format!("share/payload.manifest does not list {reader}, which the tarball carries");

    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        (
            "bin/meshmud packed but unlisted",
            layer5_payload_listing(&[layer5::MESHMUD_BIN], &[], true, &[]),
            "share/payload.manifest does not list bin/meshmud, which the tarball carries",
        ),
        (
            "a panels/ file packed but unlisted",
            layer5_payload_listing(&[app_js], &[], true, &[]),
            &no_app_js,
        ),
        (
            "the presence reader script packed but unlisted",
            layer5_payload_listing(&[reader], &[], true, &[]),
            &no_reader,
        ),
        (
            "a base file (share/kb) packed but unlisted",
            layer5_payload_listing(&["share/kb/llms.txt"], &[], true, &[]),
            "share/payload.manifest does not list share/kb/llms.txt,",
        ),
        (
            "a file whose name holds whitespace can never be listed",
            layer5_payload_listing(&[], &[], true, &[("share/kb/two words.md", b"x")]),
            "does not list share/kb/two words.md, which the tarball carries",
        ),
        (
            "a listed file that is not in the tarball",
            layer5_payload_listing(&[], &[("bin/ghost", b"never packed")], true, &[]),
            "share/payload.manifest lists bin/ghost, which is not a regular file in the tarball",
        ),
        (
            "a listed bundle file that is not in the tarball",
            layer5_payload_listing(
                &[],
                &[("panels/meshtastic-web/assets/gone.js", b"gone")],
                true,
                &[],
            ),
            "lists panels/meshtastic-web/assets/gone.js, which is not a regular file",
        ),
        (
            "a listing path that climbs out of the tree",
            layer5_payload_listing(&[], &[("../escape", b"x")], true, &[]),
            "climbs out with '..'",
        ),
        (
            "a payload without share/payload.manifest",
            layer5_payload_listing(&[], &[], false, &[]),
            "carries no share/payload.manifest file",
        ),
        (
            "a listing that pins nothing",
            layer5_payload_listing(&every_ref, &[], true, &[]),
            "share/payload.manifest lists nothing",
        ),
    ];
    for (what, payload, needle) in cases {
        let (dir, arts) = release_artifacts();
        std::fs::write(arts.join(x86_payload), payload).unwrap();
        let out = dir.path().join("refused.toml");
        let o = gen_manifest(&arts, &out);
        let stderr = String::from_utf8_lossy(&o.stderr);
        assert!(!o.status.success(), "{what}: gen-manifest must refuse");
        assert!(
            stderr.contains(needle),
            "{what}: wanted {needle:?} in: {stderr}"
        );
        assert!(
            stderr.contains(x86_payload),
            "{what}: the error names the payload: {stderr}"
        );
        assert!(!out.exists(), "{what}: no manifest may be written");
    }

    // The error names the FIRST offending path: unlisted files in bytewise order…
    let (dir, arts) = release_artifacts();
    std::fs::write(
        arts.join(x86_payload),
        layer5_payload_listing(&[app_js, layer5::MESHMUD_BIN, reader], &[], true, &[]),
    )
    .unwrap();
    let o = gen_manifest(&arts, &dir.path().join("refused.toml"));
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success());
    assert!(
        stderr.contains("does not list bin/meshmud,"),
        "wanted bin/meshmud first: {stderr}"
    );
    assert!(
        !stderr.contains("app.js") && !stderr.contains("meshtastic_presence.py"),
        "only the first offender is named: {stderr}"
    );
    // …then, with nothing unlisted, the listed-but-absent ones in listing order.
    std::fs::write(
        arts.join(x86_payload),
        layer5_payload_listing(&[], &[("zz/last", b"2"), ("aa/first", b"1")], true, &[]),
    )
    .unwrap();
    let o = gen_manifest(&arts, &dir.path().join("refused.toml"));
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success());
    assert!(
        stderr.contains("lists zz/last,") && !stderr.contains("aa/first"),
        "wanted the first listed absentee (zz/last) only: {stderr}"
    );
    // An unlisted file is reported before a phantom one.
    std::fs::write(
        arts.join(x86_payload),
        layer5_payload_listing(&[app_js], &[("bin/ghost", b"x")], true, &[]),
    )
    .unwrap();
    let o = gen_manifest(&arts, &dir.path().join("refused.toml"));
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success());
    assert!(
        stderr.contains(&format!("does not list {app_js},")) && !stderr.contains("bin/ghost"),
        "wanted the unlisted file first: {stderr}"
    );

    // The guard holds with no Layer 5 pin written too: a plain readable payload with
    // an unlisted file is refused, and a complete one passes without a warning.
    let (dir, arts) = release_artifacts();
    for name in [
        "meshmud-v1.0.0-linux-amd64",
        "meshmud-v1.0.0-linux-arm64",
        "meshtastic-web-v2.7.2.tar",
        "meshtastic-web-v2.7.2.tree",
        "meshtastic-presence-lock.txt",
        "payload-aarch64-unknown-linux-musl.tar.gz",
    ] {
        std::fs::remove_file(arts.join(name)).unwrap();
    }
    std::fs::write(
        arts.join(x86_payload),
        common::build_payload_listing(&[], &["share/kb/llms.txt"], &[], true),
    )
    .unwrap();
    let out = dir.path().join("plain.toml");
    let o = gen_manifest(&arts, &out);
    assert!(!o.status.success(), "an unlisted file must be refused");
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("does not list share/kb/llms.txt,"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!out.exists());
    std::fs::write(arts.join(x86_payload), common::build_payload()).unwrap();
    let o = gen_manifest(&arts, &out);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!String::from_utf8_lossy(&o.stderr).contains("warning"));
    let m = ChannelManifest::parse(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert!(m.payload_for(X86).is_some());
}

/// What the node does with a payload at stage time (`stage_payload`: pin check, safe
/// unpack, `verify_tree`), pinned to the payload's own bytes.
fn node_stages(payload: &[u8]) -> anyhow::Result<()> {
    use std::str::FromStr;
    let artifact = node111::manifest::PinnedArtifact {
        name: X86.into(),
        url: "http://x".into(),
        sha256: sha256_bytes(payload),
        size: payload.len() as u64,
    };
    let staging = tempfile::tempdir().unwrap();
    node111::daemon::updater::stage::stage_payload(
        payload,
        &artifact,
        staging.path(),
        &node111::calver::Calver::from_str("2026.10.12").unwrap(),
    )
    .map(|_| ())
}

/// The listing's hashes (Layer 5 integrated review m7): each listed sha256 must be the
/// tarball member's, compared exactly as `verify_tree` compares it, so a payload that
/// every node would refuse at stage time is refused at release time instead, naming
/// the first offending path.
#[test]
fn generator_refuses_a_payload_listing_with_a_wrong_hash() {
    let x86_payload = "payload-x86_64-unknown-linux-musl.tar.gz";
    let wrong: &[u8] = b"not the packed bytes";
    let wrong_sha = sha256_bytes(wrong);
    let app_js = "panels/meshtastic-web/assets/app.js";
    let app_js_bytes = BUNDLE
        .iter()
        .find(|(rel, _)| *rel == "assets/app.js")
        .unwrap()
        .1;
    let llms = "share/kb/llms.txt";
    let mismatch = |rel: &str, packed: &[u8]| {
        format!(
            "share/payload.manifest lists {rel} with sha256 {wrong_sha}, but the tarball's {rel} \
             has sha256 {}",
            sha256_bytes(packed)
        )
    };

    // `unlisted` + `phantom` on the same path: packed with its own bytes, listed once
    // with the hash of other bytes.
    let cases: Vec<(&str, Vec<u8>, String)> = vec![
        (
            "bin/meshmud listed with another file's hash",
            layer5_payload_listing(
                &[layer5::MESHMUD_BIN],
                &[(layer5::MESHMUD_BIN, wrong)],
                true,
                &[],
            ),
            mismatch(layer5::MESHMUD_BIN, MESHMUD_X86),
        ),
        (
            "a bundle file listed with another file's hash",
            layer5_payload_listing(&[app_js], &[(app_js, wrong)], true, &[]),
            mismatch(app_js, app_js_bytes),
        ),
        (
            "a base file (share/kb) listed with another file's hash",
            layer5_payload_listing(&[llms], &[(llms, wrong)], true, &[]),
            mismatch(llms, b"index\n"),
        ),
        (
            "a path listed twice, right then wrong (every line must hold)",
            layer5_payload_listing(&[], &[(llms, wrong)], true, &[]),
            mismatch(llms, b"index\n"),
        ),
    ];
    for (what, payload, needle) in cases {
        let (dir, arts) = release_artifacts();
        std::fs::write(arts.join(x86_payload), &payload).unwrap();
        let out = dir.path().join("refused.toml");
        let o = gen_manifest(&arts, &out);
        let stderr = String::from_utf8_lossy(&o.stderr);
        assert!(!o.status.success(), "{what}: gen-manifest must refuse");
        assert!(
            stderr.contains(&needle),
            "{what}: wanted {needle:?} in: {stderr}"
        );
        assert!(
            stderr.contains(x86_payload),
            "{what}: the error names the payload: {stderr}"
        );
        assert!(!out.exists(), "{what}: no manifest may be written");
        // the node agrees: its stage-time verify_tree refuses the same payload
        let err = node_stages(&payload).unwrap_err().to_string();
        assert!(
            err.contains("payload tree hash mismatch"),
            "{what}: the node refuses it too: {err}"
        );
    }

    // The comparison is exact, as the node's: the right hash in uppercase is refused by both.
    let mut files: Vec<(String, Vec<u8>)> = vec![
        (
            "bin/111".into(),
            b"#!/bin/sh\necho fake 111 daemon\n".to_vec(),
        ),
        (llms.into(), b"index\n".to_vec()),
        ("share/kb/reticulum.md".into(), b"entry\n".to_vec()),
    ];
    files.extend(layer5_files(Some(MESHMUD_X86), BUNDLE, true, &[]));
    let mut listing = String::new();
    for (rel, bytes) in &files {
        let sha = sha256_bytes(bytes);
        let sha = if rel == llms { sha.to_uppercase() } else { sha };
        listing.push_str(&format!("{sha}  {rel}\n"));
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    {
        let mut b = tar::Builder::new(&mut gz);
        for (rel, bytes) in files
            .iter()
            .map(|(r, b)| (r.as_str(), b.as_slice()))
            .chain([("share/payload.manifest", listing.as_bytes())])
        {
            let mut h = tar::Header::new_gnu();
            h.set_size(bytes.len() as u64);
            h.set_mode(0o644);
            h.set_entry_type(tar::EntryType::Regular);
            h.set_cksum();
            b.append_data(&mut h, rel, bytes).unwrap();
        }
        b.finish().unwrap();
    }
    let upper = gz.finish().unwrap();
    let (dir, arts) = release_artifacts();
    std::fs::write(arts.join(x86_payload), &upper).unwrap();
    let o = gen_manifest(&arts, &dir.path().join("refused.toml"));
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success(), "an uppercase hash must be refused");
    assert!(
        stderr.contains(&format!(
            "lists {llms} with sha256 {}",
            sha256_bytes(b"index\n").to_uppercase()
        )),
        "{stderr}"
    );
    assert!(node_stages(&upper)
        .unwrap_err()
        .to_string()
        .contains("payload tree hash mismatch"));

    // Listing order across absent and wrong-hash lines: the first offending line is named.
    let (dir, arts) = release_artifacts();
    for (phantom, first, not) in [
        (
            [(layer5::MESHMUD_BIN, wrong), ("aa/absent", wrong)],
            "lists bin/meshmud with sha256",
            "aa/absent",
        ),
        (
            [("aa/absent", wrong), (layer5::MESHMUD_BIN, wrong)],
            "lists aa/absent, which is not a regular file",
            "lists bin/meshmud with",
        ),
    ] {
        std::fs::write(
            arts.join(x86_payload),
            layer5_payload_listing(&[layer5::MESHMUD_BIN], &phantom, true, &[]),
        )
        .unwrap();
        let o = gen_manifest(&arts, &dir.path().join("refused.toml"));
        let stderr = String::from_utf8_lossy(&o.stderr);
        assert!(!o.status.success());
        assert!(
            stderr.contains(first) && !stderr.contains(not),
            "wanted only {first:?}: {stderr}"
        );
    }

    // And an unlisted file is still reported before a wrong hash.
    std::fs::write(
        arts.join(x86_payload),
        layer5_payload_listing(&[app_js, llms], &[(llms, wrong)], true, &[]),
    )
    .unwrap();
    let o = gen_manifest(&arts, &dir.path().join("refused.toml"));
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success());
    assert!(
        stderr.contains(&format!("does not list {app_js},")) && !stderr.contains("with sha256"),
        "wanted the unlisted file first: {stderr}"
    );
}

/// The accepting side of the listing guard: a listing in the forms the node reads
/// (`./` and doubled-slash paths, blank lines, a CRLF listing, extra columns) still
/// covers its files, and the Layer 5 round trip is unchanged.
#[test]
fn generator_accepts_listings_the_node_reads() {
    // The node resolves `./bin/111` and `bin//111` to the same file, so they cover it.
    let files = layer5_files(Some(MESHMUD_X86), BUNDLE, true, &[]);
    let mut all: Vec<(String, Vec<u8>)> = vec![
        (
            "bin/111".into(),
            b"#!/bin/sh\necho fake 111 daemon\n".to_vec(),
        ),
        ("share/kb/llms.txt".into(), b"index\n".to_vec()),
        ("share/kb/reticulum.md".into(), b"entry\n".to_vec()),
    ];
    all.extend(files);
    let mut listing = String::from("\r\n\n");
    for (i, (rel, bytes)) in all.iter().enumerate() {
        let sha = sha256_bytes(bytes);
        let path = match i % 3 {
            0 => format!("./{rel}"),
            1 => rel.replacen('/', "//", 1),
            _ => rel.clone(),
        };
        listing.push_str(&format!("{sha}  {path}  trailing columns are ignored\r\n"));
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    {
        let mut b = tar::Builder::new(&mut gz);
        for (rel, bytes) in all
            .iter()
            .map(|(r, b)| (r.as_str(), b.as_slice()))
            .chain([("share/payload.manifest", listing.as_bytes())])
        {
            let mut h = tar::Header::new_gnu();
            h.set_size(bytes.len() as u64);
            h.set_mode(0o644);
            h.set_entry_type(tar::EntryType::Regular);
            h.set_cksum();
            b.append_data(&mut h, rel, bytes).unwrap();
        }
        b.finish().unwrap();
    }
    let payload = gz.finish().unwrap();

    let (dir, arts) = release_artifacts();
    std::fs::write(
        arts.join("payload-x86_64-unknown-linux-musl.tar.gz"),
        &payload,
    )
    .unwrap();
    let out = dir.path().join("channel-111-stable.toml");
    let o = gen_manifest(&arts, &out);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    ChannelManifest::parse(&std::fs::read_to_string(&out).unwrap()).unwrap();
}
