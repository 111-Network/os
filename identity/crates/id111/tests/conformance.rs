//! Layer 2 §14 conformance suite — the frozen test vectors as LIVE gates.
//!
//! Every file in `os/identity/test-vectors/` (frozen 2026-10-13, task T8) is
//! loaded here and re-asserted against the PUBLIC `id111` API, so a vector
//! can never rot into dead data and an implementation drift can never slip
//! past a frozen answer. Only the public API is used: this suite sees the
//! crate exactly as Layer 1/4/5/7 do.
//!
//! # Vector format (decision recorded for the reviewer)
//!
//! The files are real JSON (hex-encoded bytes; u64 counters/epochs as decimal
//! strings to dodge JSON's 2^53 integer ceiling; small integers as numbers).
//! `serde_json` is NOT in the frozen dependency set (spec §3 / TASKS.md), so
//! this file carries a ~100-line strict reader for the JSON subset the
//! vectors use (objects, arrays, strings with the standard escapes, unsigned
//! integers, true/false/null). It rejects duplicate object keys and trailing
//! garbage, so a vector file cannot silently shadow itself.
//!
//! # Outcome tokens
//!
//! Expected outcomes are compact strings (`ok:pinned`, `err:stale:5:2`, …)
//! produced by the `*_tok` functions below from the public error enums. The
//! token grammar is documented per file in each vector's `description`.
//!
//! # Independence
//!
//! Where a recipe is fixed by the spec rather than by the crate (§6.4 sealed
//! packet; SLIP-0010 chain codes), the suite recomputes the answer with a
//! small in-test model built straight from the primitive crates — so the
//! frozen vectors are checked against the SPEC recipe, not only against the
//! code that produced them. `test-vectors/crosscheck.py` is the third leg
//! (a from-scratch Python re-derivation).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use id111::announce::{self, process_announce, Announce, AnnounceError};
use id111::attest::{
    AttestError, Attestation, AttestationSet, MeshEpoch, NodeLeg, Resolution, TransportId,
    TransportVerifier,
};
use id111::backup::{
    self, BackupArtifacts, BackupDescriptor, CounterFloor, RestoreError, RestoreOptions,
    RestoreOutcome,
};
use id111::bip39::{Bip39Error, RootMnemonic, WordCount};
use id111::counters::{CounterError, CounterStore, ReceiveWindow, WindowReject};
use id111::encoding::{self, EncodingError, KeyKind, NodeIdDisplay};
use id111::keyset::{
    validate_x25519_public_key, verify_strict, verify_strict_tagged, weak_key_self_test, EcdhError,
    Keyset, KeysetError, NodeId, PublicKeyset, VerifyError, WeakKeyError,
};
use id111::seal::{self, OpenError, SealError, SealHeader};
use id111::slip0010::{self, Slip0010Error};
use id111::store::{PinOutcome, PinStore};

// ===========================================================================
// 1. Minimal strict JSON reader (no serde_json in the frozen dep set)
// ===========================================================================

// `Bool` is part of the reader's JSON subset even though no current vector
// file carries a boolean.
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum V {
    Null,
    Bool(bool),
    Num(u64),
    Str(String),
    Arr(Vec<V>),
    Obj(Vec<(String, V)>),
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\n' | b'\t' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> u8 {
        self.b.get(self.i).copied().unwrap_or(0)
    }

    fn expect(&mut self, c: u8) {
        assert_eq!(
            self.peek(),
            c,
            "json: expected {:?} at byte {}",
            c as char,
            self.i
        );
        self.i += 1;
    }

    fn lit(&mut self, word: &str) {
        assert!(
            self.b[self.i..].starts_with(word.as_bytes()),
            "json: bad literal at byte {}",
            self.i
        );
        self.i += word.len();
    }

    fn value(&mut self) -> V {
        self.ws();
        match self.peek() {
            b'{' => {
                self.i += 1;
                let mut members: Vec<(String, V)> = Vec::new();
                self.ws();
                if self.peek() == b'}' {
                    self.i += 1;
                    return V::Obj(members);
                }
                loop {
                    self.ws();
                    let key = self.string();
                    assert!(
                        members.iter().all(|(k, _)| *k != key),
                        "json: duplicate key {key:?}"
                    );
                    self.ws();
                    self.expect(b':');
                    let val = self.value();
                    members.push((key, val));
                    self.ws();
                    match self.peek() {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return V::Obj(members);
                        }
                        c => panic!(
                            "json: unexpected {:?} in object at byte {}",
                            c as char, self.i
                        ),
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.peek() == b']' {
                    self.i += 1;
                    return V::Arr(items);
                }
                loop {
                    items.push(self.value());
                    self.ws();
                    match self.peek() {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return V::Arr(items);
                        }
                        c => panic!(
                            "json: unexpected {:?} in array at byte {}",
                            c as char, self.i
                        ),
                    }
                }
            }
            b'"' => V::Str(self.string()),
            b't' => {
                self.lit("true");
                V::Bool(true)
            }
            b'f' => {
                self.lit("false");
                V::Bool(false)
            }
            b'n' => {
                self.lit("null");
                V::Null
            }
            b'0'..=b'9' => {
                let start = self.i;
                while self.peek().is_ascii_digit() {
                    self.i += 1;
                }
                let text = std::str::from_utf8(&self.b[start..self.i]).unwrap();
                V::Num(text.parse().expect("json: integer out of u64 range"))
            }
            c => panic!("json: unexpected {:?} at byte {}", c as char, self.i),
        }
    }

    fn string(&mut self) -> String {
        self.expect(b'"');
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.peek();
            assert!(self.i < self.b.len(), "json: unterminated string");
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = self.peek();
                    self.i += 1;
                    match e {
                        b'"' | b'\\' | b'/' => out.push(e),
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let hex = std::str::from_utf8(&self.b[self.i..self.i + 4]).unwrap();
                            let cp = u32::from_str_radix(hex, 16).expect("json: bad \\u escape");
                            self.i += 4;
                            let ch =
                                char::from_u32(cp).expect("json: surrogate escapes unsupported");
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        e => panic!("json: bad escape \\{}", e as char),
                    }
                }
                c => out.push(c),
            }
        }
        String::from_utf8(out).expect("json: string is not UTF-8")
    }
}

fn parse_json(text: &str) -> V {
    let mut p = Parser {
        b: text.as_bytes(),
        i: 0,
    };
    let v = p.value();
    p.ws();
    assert_eq!(p.i, p.b.len(), "json: trailing garbage");
    v
}

impl V {
    fn get(&self, key: &str) -> &V {
        match self {
            V::Obj(m) => m
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v)
                .unwrap_or_else(|| panic!("vector field {key:?} missing")),
            other => panic!("expected object while reading {key:?}, got {other:?}"),
        }
    }

    fn has(&self, key: &str) -> bool {
        matches!(self, V::Obj(m) if m.iter().any(|(k, _)| k == key))
    }

    fn str(&self) -> &str {
        match self {
            V::Str(s) => s,
            other => panic!("expected string, got {other:?}"),
        }
    }

    fn num(&self) -> u64 {
        match self {
            V::Num(n) => *n,
            other => panic!("expected number, got {other:?}"),
        }
    }

    /// A u64 stored as a decimal string.
    fn dec(&self) -> u64 {
        self.str().parse().expect("decimal u64 string")
    }

    fn arr(&self) -> &[V] {
        match self {
            V::Arr(a) => a,
            other => panic!("expected array, got {other:?}"),
        }
    }

    fn hex(&self) -> Vec<u8> {
        unhex(self.str())
    }

    fn h32(&self) -> [u8; 32] {
        self.hex().try_into().expect("32-byte hex")
    }

    fn h64(&self) -> [u8; 64] {
        self.hex().try_into().expect("64-byte hex")
    }

    fn indices(&self) -> Vec<u32> {
        self.arr()
            .iter()
            .map(|n| u32::try_from(n.num()).unwrap())
            .collect()
    }
}

fn vectors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors")
}

fn load(name: &str) -> V {
    let path = vectors_dir().join(name);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read frozen vector file {}: {e}", path.display()));
    parse_json(&text)
}

// ===========================================================================
// 2. Small helpers
// ===========================================================================

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len() & 1 == 0, "odd-length hex");
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex digit"))
        .collect()
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn keyset_from(seed: &[u8; 32], scalar: &[u8; 32]) -> Keyset {
    let mut b = [0u8; 64];
    b[..32].copy_from_slice(seed);
    b[32..].copy_from_slice(scalar);
    Keyset::from_bytes(&b).expect("test keyset passes the weak-key self-test")
}

fn keyset_words(words: &str) -> Keyset {
    Keyset::derive_from_mnemonic(words).expect("test mnemonic derives")
}

/// Minimal unique temp dir (no tempfile in the frozen dep set).
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "id111-conformance-{tag}-{}-{seq}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// A not-yet-existing identity dir inside this temp dir (restore and
    /// migrate create/rename it).
    fn identity(&self) -> PathBuf {
        self.0.join("identity")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ===========================================================================
// 3. Outcome tokens (the grammar documented in each vector file)
// ===========================================================================

fn ann_tok(r: Result<PinOutcome, AnnounceError>) -> String {
    match r {
        Ok(PinOutcome::Pinned { evicted: None }) => "ok:pinned".into(),
        Ok(PinOutcome::Pinned { evicted: Some(_) }) => "ok:pinned+evicted".into(),
        Ok(PinOutcome::CounterUpdated { previous }) => format!("ok:updated:{previous}"),
        Err(AnnounceError::MalformedLength { got }) => format!("err:length:{got}"),
        Err(AnnounceError::UnsupportedVersion(v)) => format!("err:version:{v}"),
        Err(AnnounceError::LowOrderEncKey) => "err:loworder".into(),
        Err(AnnounceError::InvalidSignature(e)) => format!("err:sig:{e:?}"),
        Err(AnnounceError::IdentityCollision) => "err:collision".into(),
        Err(AnnounceError::StaleCounter { last_seen, got }) => {
            format!("err:stale:{last_seen}:{got}")
        }
        Err(AnnounceError::KeyConflict) => "err:conflict".into(),
        Err(AnnounceError::Store(_)) => "err:store".into(),
        Err(AnnounceError::AppdataTooLong { got }) => format!("err:appdata:{got}"),
    }
}

fn open_tok(r: Result<seal::OpenedPacket, OpenError>) -> String {
    match r {
        Ok(_) => "ok".into(),
        Err(OpenError::Truncated) => "err:truncated".into(),
        Err(OpenError::UnsupportedCryptoMode(m)) => format!("err:mode:{m}"),
        Err(OpenError::Ecdh(e)) => format!("err:ecdh:{e:?}"),
        Err(OpenError::AuthFailed) => "err:auth".into(),
        Err(OpenError::PlaintextTooShort) => "err:short-plaintext".into(),
    }
}

fn att_tok(e: AttestError) -> String {
    match e {
        AttestError::InvalidLength { got, .. } => format!("err:length:{got}"),
        AttestError::UnknownVersion(v) => format!("err:version:{v}"),
        AttestError::UnknownTransportId(t) => format!("err:tid:{t}"),
        AttestError::InvalidNodeSignature(e) => format!("err:node-sig:{e:?}"),
        AttestError::InvalidTransportSignature => "err:transport-sig".into(),
        AttestError::Expired {
            expiry_epoch,
            current,
        } => {
            format!("err:expired:{}:{}", expiry_epoch.as_u64(), current.as_u64())
        }
        AttestError::Superseded { retained, offered } => {
            format!("err:superseded:{}:{}", retained.as_u64(), offered.as_u64())
        }
        AttestError::ConflictingAttestation { epoch } => format!("err:conflict:{}", epoch.as_u64()),
    }
}

fn res_tok(r: Result<Resolution, AttestError>) -> String {
    match r {
        Ok(Resolution::Pinned) => "ok:pinned".into(),
        Ok(Resolution::Supersedes { previous }) => format!("ok:supersedes:{}", previous.as_u64()),
        Ok(Resolution::Duplicate) => "ok:duplicate".into(),
        Err(e) => att_tok(e),
    }
}

fn enc_tok<T>(r: Result<T, EncodingError>) -> String {
    match r {
        Ok(_) => "ok".into(),
        Err(EncodingError::InvalidLength { expected, got }) => {
            format!("InvalidLength:{expected}:{got}")
        }
        Err(EncodingError::UnknownPrefix(p)) => format!("UnknownPrefix:{:02x}{:02x}", p[0], p[1]),
        Err(EncodingError::InvalidHex) => "InvalidHex".into(),
        Err(EncodingError::InvalidBech32) => "InvalidBech32".into(),
        Err(EncodingError::WrongHrp) => "WrongHrp".into(),
        Err(EncodingError::NonCanonicalBech32) => "NonCanonicalBech32".into(),
    }
}

fn verify_tok(r: Result<(), VerifyError>) -> String {
    match r {
        Ok(()) => "ok".into(),
        Err(e) => format!("err:{e:?}"),
    }
}

/// Coarse class of an outcome token for corpus-coverage assertions:
/// signature errors keep their VerifyError variant, everything else keeps
/// its first two segments (`err:stale`, `err:length`, …).
fn class_of(tok: &str) -> String {
    let parts: Vec<&str> = tok.split(':').collect();
    if matches!(parts.get(1), Some(&"sig") | Some(&"node-sig")) {
        parts.iter().take(3).copied().collect::<Vec<_>>().join(":")
    } else {
        parts.iter().take(2).copied().collect::<Vec<_>>().join(":")
    }
}

fn restore_tok(r: Result<RestoreOutcome, RestoreError>) -> String {
    match r {
        Ok(_) => "ok".into(),
        Err(RestoreError::OldInstanceNotRetired) => "OldInstanceNotRetired".into(),
        Err(RestoreError::LiveIdentityDir) => "LiveIdentityDir".into(),
        Err(RestoreError::Mnemonic(Bip39Error::Mnemonic(e))) => format!("Mnemonic:{e:?}"),
        Err(RestoreError::Mnemonic(Bip39Error::UnsupportedWordCount(n))) => {
            format!("UnsupportedWordCount:{n}")
        }
        Err(RestoreError::Mnemonic(Bip39Error::BadEntropyLength(n))) => {
            format!("BadEntropyLength:{n}")
        }
        Err(RestoreError::Descriptor(e)) => format!("Descriptor:{e:?}"),
        Err(RestoreError::DescriptorWordCountMismatch { words, .. }) => {
            format!("DescriptorWordCountMismatch:{words}")
        }
        Err(RestoreError::DescriptorConflict) => "DescriptorConflict".into(),
        Err(RestoreError::QrPayload(_)) => "QrPayload".into(),
        Err(RestoreError::Keyset(_)) => "Keyset".into(),
        Err(RestoreError::Io(_)) => "Io".into(),
    }
}

// ===========================================================================
// 4. Gate inventory: every §14 gate has a frozen, self-describing file
// ===========================================================================

/// (file, §14 gate it freezes). The README maps the same table to prose.
const GATE_FILES: &[(&str, &str)] = &[
    (
        "ed25519-rfc8032.json",
        "RFC 8032 + Monocypher agreement: keygen-from-seed, sign/verify",
    ),
    (
        "x25519-rfc7748.json",
        "RFC 7748 + Monocypher agreement: ECDH",
    ),
    ("slip0010-published.json", "SLIP-0010 published vectors"),
    (
        "slip0010-project-paths.json",
        "SLIP-0010 pinned-path project vectors",
    ),
    (
        "keyset-project.json",
        "mnemonic -> NodeId / X25519 project keyset",
    ),
    ("encoding.json", "section 7 encodings"),
    (
        "edge-vectors.json",
        "edge/malleability: strict predicate + X25519 low-order",
    ),
    ("weak-key.json", "weak-key self-test"),
    ("announce.json", "announce encode/verify + negatives"),
    (
        "sealed-packet.json",
        "sealed-packet round-trip + negatives (Monocypher agreement)",
    ),
    (
        "backup-format.json",
        "backup format round-trip + fail-closed refusals",
    ),
    ("attestation.json", "attestation encode/verify + negatives"),
    (
        "restore-counters.json",
        "restore wire-function + sync-commit",
    ),
    ("d9-migration.json", "D9-migration round-trip"),
];

#[test]
fn gate_files_are_present_self_describing_and_frozen() {
    let mut on_disk: BTreeSet<String> = fs::read_dir(vectors_dir())
        .expect("test-vectors/ exists")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    for (file, gate) in GATE_FILES {
        assert!(
            on_disk.remove(*file),
            "missing frozen gate file {file} ({gate})"
        );
        let v = load(file);
        let spec = v.get("spec").str();
        assert!(
            spec.contains('§'),
            "{file}: spec field must name the spec section(s): {spec:?}"
        );
        assert_eq!(v.get("frozen").str(), "2026-10-13", "{file}: frozen date");
        assert!(
            !v.get("description").str().is_empty(),
            "{file}: description"
        );
        assert!(!v.get("provenance").str().is_empty(), "{file}: provenance");
    }
    assert!(
        on_disk.is_empty(),
        "undeclared vector files (add them to GATE_FILES): {on_disk:?}"
    );
}

#[test]
fn readme_documents_every_gate_file() {
    let readme =
        fs::read_to_string(vectors_dir().join("README.md")).expect("test-vectors/README.md");
    for (file, _) in GATE_FILES {
        assert!(readme.contains(file), "README.md does not mention {file}");
    }
    assert!(readme.contains("2026-10-13"));
}

// ===========================================================================
// 5. RFC 8032 / RFC 7748 / SLIP-0010 / keyset
// ===========================================================================

#[test]
fn ed25519_rfc8032_keygen_sign_verify() {
    let v = load("ed25519-rfc8032.json");
    let companion = v.get("ecdh_companion_scalar").h32();
    let vectors = v.get("vectors").arr();
    assert!(vectors.len() >= 4);
    for e in vectors {
        let name = e.get("name").str();
        let sk = e.get("secret_key").h32();
        let pk = e.get("public_key").h32();
        let msg = e.get("message").hex();
        let sig = e.get("signature").h64();

        // keygen-from-seed: the seed derives the published pubkey; NodeId = pubkey (B2).
        let ks = keyset_from(&sk, &companion);
        assert_eq!(ks.public().sign_pubkey(), &pk, "{name}: pubkey");
        assert_eq!(
            ks.node_id().as_bytes(),
            &pk,
            "{name}: NodeId is the raw pubkey"
        );

        // sign: an EMPTY tag is exactly raw RFC 8032 signing; Ed25519 is deterministic.
        assert_eq!(ks.sign_with_tag(b"", &msg), sig, "{name}: signature");
        // verify under the strict predicate, tagged and untagged entry points.
        let node = NodeId::from(pk);
        verify_strict(&node, &msg, &sig).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        verify_strict_tagged(&node, b"", &msg, &sig).unwrap();

        // the tag is part of the signed region: a tagged signature differs and
        // does not verify as the bare message.
        let tagged = ks.sign_with_tag(b"111/announce/v1", &msg);
        assert_ne!(tagged, sig, "{name}: domain tag must change the signature");
        assert!(verify_strict(&node, &msg, &tagged).is_err(), "{name}");
        verify_strict_tagged(&node, b"111/announce/v1", &msg, &tagged).unwrap();

        // negatives: message bit, every signature region.
        let mut bad_msg = msg.clone();
        if bad_msg.is_empty() {
            bad_msg.push(0);
        } else {
            bad_msg[0] ^= 1;
        }
        assert!(
            verify_strict(&node, &bad_msg, &sig).is_err(),
            "{name}: tampered message"
        );
        for idx in [0usize, 31, 32, 63] {
            let mut bad = sig;
            bad[idx] ^= 1;
            assert!(
                verify_strict(&node, &msg, &bad).is_err(),
                "{name}: sig byte {idx}"
            );
        }
    }
}

#[test]
fn x25519_rfc7748_agreement_and_scalarmult() {
    let v = load("x25519-rfc7748.json");
    let seed = v.get("ed25519_companion_seed").h32();

    let dh = v.get("diffie_hellman");
    let (a_s, a_p) = (dh.get("alice_scalar").h32(), dh.get("alice_public").h32());
    let (b_s, b_p) = (dh.get("bob_scalar").h32(), dh.get("bob_public").h32());
    let shared = dh.get("shared").h32();
    let alice = keyset_from(&seed, &a_s);
    let bob = keyset_from(&seed, &b_s);
    assert_eq!(alice.x25519_public(), a_p);
    assert_eq!(bob.x25519_public(), b_p);
    assert_eq!(alice.public().enc_pubkey(), &a_p);
    assert_eq!(&alice.ecdh(&b_p).unwrap()[..], &shared[..]);
    assert_eq!(&bob.ecdh(&a_p).unwrap()[..], &shared[..]);

    // RFC 7748 section 5.2: arbitrary scalar x arbitrary u.
    for e in v.get("scalarmult").arr() {
        let k = keyset_from(&seed, &e.get("scalar").h32());
        let out = k
            .ecdh(&e.get("u").h32())
            .expect("RFC 7748 5.2 vector is contributory");
        assert_eq!(&out[..], &e.get("output").hex()[..]);
    }
}

#[test]
fn x25519_rfc7748_iterated() {
    let v = load("x25519-rfc7748.json");
    let seed = v.get("ed25519_companion_seed").h32();
    let it = v.get("iterated");
    let mut k = it.get("start").h32();
    let mut u = k;
    for i in 1..=1000u32 {
        let out = keyset_from(&seed, &k)
            .ecdh(&u)
            .expect("iterated vector is contributory");
        u = k;
        k = *out;
        if i == 1 {
            assert_eq!(to_hex(&k), it.get("after_1").str());
        }
    }
    assert_eq!(to_hex(&k), it.get("after_1000").str());
}

/// Independent SLIP-0010 (ed25519) reference: HMAC-SHA512, hardened-only.
fn slip10_reference(seed: &[u8], path: &[u32]) -> ([u8; 32], [u8; 32]) {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha512;
    fn mac(key: &[u8], data: &[u8]) -> ([u8; 32], [u8; 32]) {
        let mut m = <Hmac<Sha512> as KeyInit>::new_from_slice(key).unwrap();
        m.update(data);
        let out: [u8; 64] = m.finalize().into_bytes().into();
        (out[..32].try_into().unwrap(), out[32..].try_into().unwrap())
    }
    let (mut key, mut chain) = mac(b"ed25519 seed", seed);
    for &idx in path {
        let mut data = vec![0u8];
        data.extend_from_slice(&key);
        data.extend_from_slice(&(idx | 0x8000_0000).to_be_bytes());
        (key, chain) = mac(&chain, &data);
    }
    (key, chain)
}

#[test]
fn slip0010_published_vectors() {
    let v = load("slip0010-published.json");
    let mut checked = 0;
    for vec in v.get("vectors").arr() {
        let seed = vec.get("seed").hex();
        for c in vec.get("chains").arr() {
            let idx = c.get("indices").indices();
            let want_key = c.get("key").h32();
            let want_chain = c.get("chain").h32();
            // the crate (key half only: the chain half is discarded, section 2)
            let got = slip0010::derive_key_half(&seed, &idx).unwrap();
            assert_eq!(
                &got[..],
                &want_key[..],
                "crate key half, path {}",
                c.get("path").str()
            );
            // the independent reference (both halves)
            let (k, ch) = slip10_reference(&seed, &idx);
            assert_eq!(
                k,
                want_key,
                "reference key half, path {}",
                c.get("path").str()
            );
            assert_eq!(
                ch,
                want_chain,
                "reference chain half, path {}",
                c.get("path").str()
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 12, "SLIP-0010 vectors 1 and 2: six chains each");
}

#[test]
fn slip0010_project_paths_and_bip39_seed() {
    let v = load("slip0010-project-paths.json");
    let paths = v.get("paths");
    assert_eq!(paths.get("signing").str(), "m/111'/0'/0'");
    assert_eq!(paths.get("ecdh").str(), "m/111'/0'/1'");
    assert_eq!(
        paths.get("signing_indices").indices(),
        slip0010::SIGNING_PATH
    );
    assert_eq!(paths.get("ecdh_indices").indices(), slip0010::ECDH_PATH);

    for e in v.get("vectors").arr() {
        let name = e.get("name").str();
        let words = e.get("mnemonic").str();
        let m = RootMnemonic::parse(words).unwrap();
        // BIP-39: entropy <-> words, and the PBKDF2 seed with the empty passphrase.
        assert_eq!(
            &m.entropy()[..],
            &e.get("entropy").hex()[..],
            "{name}: entropy"
        );
        assert_eq!(
            RootMnemonic::from_entropy(&e.get("entropy").hex())
                .unwrap()
                .phrase(),
            words
        );
        let seed = m.to_seed();
        assert_eq!(
            &seed[..],
            &e.get("bip39_seed").hex()[..],
            "{name}: BIP-39 seed"
        );
        // SLIP-0010 at the pinned paths: crate API == generic API == reference == frozen.
        let sign = e.get("signing_seed").h32();
        let ecdh = e.get("ecdh_scalar").h32();
        assert_eq!(
            &slip0010::derive_signing_seed(&seed[..])[..],
            &sign[..],
            "{name}: signing seed"
        );
        assert_eq!(
            &slip0010::derive_ecdh_scalar(&seed[..])[..],
            &ecdh[..],
            "{name}: ecdh scalar"
        );
        assert_eq!(
            &slip0010::derive_key_half(&seed[..], &slip0010::SIGNING_PATH).unwrap()[..],
            &sign[..]
        );
        assert_eq!(slip10_reference(&seed[..], &slip0010::SIGNING_PATH).0, sign);
        assert_eq!(slip10_reference(&seed[..], &slip0010::ECDH_PATH).0, ecdh);
        // the two pinned paths yield distinct keys (no shared branch).
        assert_ne!(sign, ecdh);
    }

    for n in v.get("negatives").arr() {
        let r = slip0010::derive_key_half(&[0u8; 64], &n.get("indices").indices());
        let tok = match r {
            Ok(_) => "ok".to_string(),
            Err(Slip0010Error::IndexOutOfRange(i)) => format!("IndexOutOfRange:{i}"),
        };
        assert_eq!(tok, n.get("expect").str(), "{}", n.get("name").str());
    }
}

#[test]
fn keyset_project_vectors() {
    let v = load("keyset-project.json");
    for e in v.get("vectors").arr() {
        let name = e.get("name").str();
        let ks = keyset_words(e.get("mnemonic").str());
        let (node, xpub) = (e.get("node_id").h32(), e.get("x25519_public").h32());
        let public = e.get("public_keyset").h64();
        let private = e.get("private_keyset").h64();

        let p = ks.public();
        assert_eq!(p.sign_pubkey(), &node, "{name}: NodeId");
        assert_eq!(p.enc_pubkey(), &xpub, "{name}: X25519 pub");
        assert_eq!(ks.node_id().as_bytes(), &node);
        assert_eq!(ks.x25519_public(), xpub);
        assert_eq!(p.to_bytes(), public, "{name}: 64B public keyset");
        assert_eq!(
            &ks.to_bytes()[..],
            &private[..],
            "{name}: 64B private keyset"
        );

        // 64B round trips (public ingress and the section 13 load path).
        let reloaded = Keyset::from_bytes(&private).unwrap();
        assert_eq!(reloaded.public(), p);
        assert_eq!(PublicKeyset::from_bytes(&public).unwrap(), p);
        assert_eq!(p.node_id(), NodeId::from(node));
        // B2: NodeId bytes ARE the pubkey; the signing seed's public half is the NodeId.
        assert_eq!(
            &private[..32],
            &slip0010_seed_for(e.get("mnemonic").str())[..]
        );
        // a self-signature verifies under the NodeId.
        let sig = ks.sign_with_tag(b"111/test", b"conformance");
        verify_strict_tagged(&ks.node_id(), b"111/test", b"conformance", &sig).unwrap();
        // secrets never print.
        assert_eq!(format!("{ks:?}"), "Keyset(redacted)");
    }
}

fn slip0010_seed_for(words: &str) -> [u8; 32] {
    let seed = RootMnemonic::parse(words).unwrap().to_seed();
    *slip0010::derive_signing_seed(&seed[..])
}

#[test]
fn fresh_keygen_round_trips_through_the_words() {
    // Real getrandom() keygen (spec section 4): generate -> words -> restore
    // derivation -> identical keyset, at both v1 profiles.
    for wc in [WordCount::Twelve, WordCount::TwentyFour] {
        let (m, ks) = Keyset::generate(wc).expect("entropy available");
        assert_eq!(m.word_count(), wc.word_count());
        let again = Keyset::derive_from_mnemonic(&m.phrase()).unwrap();
        assert_eq!(ks.public(), again.public());
        assert_eq!(&ks.to_bytes()[..], &again.to_bytes()[..]);
    }
}

// ===========================================================================
// 6. Encodings (section 7)
// ===========================================================================

#[test]
fn encoding_known_answers() {
    let v = load("encoding.json");
    let kats = v.get("kats").arr();
    assert!(kats.len() >= 5);
    for e in kats {
        let name = e.get("name").str();
        let key = e.get("key").h32();
        assert_eq!(encoding::hex_encode(&key), e.get("hex").str(), "{name}");
        assert_eq!(
            encoding::bech32_encode(&key),
            e.get("bech32").str(),
            "{name}"
        );
        assert_eq!(
            encoding::bech32_encode_qr(&key),
            e.get("bech32_qr").str(),
            "{name}"
        );
        assert_eq!(
            encoding::operator_label(&key),
            e.get("label").str(),
            "{name}"
        );
        assert_eq!(encoding::bech32_encode(&key).len(), 63);
        let rec_ed = e.get("record_ed25519").hex();
        let rec_x = e.get("record_x25519").hex();
        assert_eq!(
            encoding::encode_record(KeyKind::Ed25519Pub, &key)[..],
            rec_ed[..],
            "{name}"
        );
        assert_eq!(
            encoding::encode_record(KeyKind::X25519Pub, &key)[..],
            rec_x[..],
            "{name}"
        );
        assert_eq!(&rec_ed[..2], &[0xed, 0x01]);
        assert_eq!(&rec_x[..2], &[0xec, 0x01]);
        // decode round trips, both case forms.
        assert_eq!(encoding::hex_decode(e.get("hex").str()), Ok(key));
        assert_eq!(encoding::bech32_decode(e.get("bech32").str()), Ok(key));
        assert_eq!(encoding::bech32_decode(e.get("bech32_qr").str()), Ok(key));
        assert_eq!(
            encoding::parse_record(&rec_ed),
            Ok((KeyKind::Ed25519Pub, key))
        );
        assert_eq!(
            encoding::parse_record(&rec_x),
            Ok((KeyKind::X25519Pub, key))
        );
        // label shape: xxxx-xxxx-xx, lowercase base32.
        let label = e.get("label").str();
        assert_eq!(label.len(), 12);
        assert_eq!((label.as_bytes()[4], label.as_bytes()[9]), (b'-', b'-'));
        // the presentation API agrees with the free functions.
        let d = NodeIdDisplay::new(&key);
        assert_eq!(d.hex(), e.get("hex").str());
        assert_eq!(d.bech32(), e.get("bech32").str());
        assert_eq!(d.bech32_qr(), e.get("bech32_qr").str());
        assert_eq!(d.label(), label);
        assert_eq!(d.record()[..], rec_ed[..]);
        assert_eq!(format!("{d}"), e.get("hex").str());
        // NodeId and the Ed25519 record carry the same raw key (B2).
        assert_eq!(&rec_ed[2..], &key);
    }
}

#[test]
fn encoding_fail_closed_negatives() {
    let v = load("encoding.json");
    let neg = v.get("negatives");
    let mut count = 0;
    for e in neg.get("record").arr() {
        let tok = enc_tok(encoding::parse_record(&e.get("record").hex()));
        assert_eq!(
            tok,
            e.get("expect").str(),
            "record: {}",
            e.get("name").str()
        );
        count += 1;
    }
    for e in neg.get("hex").arr() {
        let tok = enc_tok(encoding::hex_decode(e.get("input").str()));
        assert_eq!(tok, e.get("expect").str(), "hex: {}", e.get("name").str());
        count += 1;
    }
    let mut saw = BTreeSet::new();
    for e in neg.get("bech32").arr() {
        let tok = enc_tok(encoding::bech32_decode(e.get("input").str()));
        assert_eq!(
            tok,
            e.get("expect").str(),
            "bech32: {}",
            e.get("name").str()
        );
        saw.insert(tok);
        count += 1;
    }
    // the section 7 bans and the embedded#15 length rule are all exercised.
    for needle in [
        "InvalidBech32",
        "WrongHrp",
        "NonCanonicalBech32",
        "InvalidLength:32:31",
        "InvalidLength:32:33",
    ] {
        assert!(
            saw.contains(needle),
            "bech32 negatives must include {needle}"
        );
    }
    assert!(count >= 35, "negative corpus unexpectedly small: {count}");
}

// ===========================================================================
// 7. Edge / malleability vectors (section 3)
// ===========================================================================

#[test]
fn strict_predicate_edge_vectors() {
    let v = load("edge-vectors.json");

    // constants: l = 2^252 + 27742317777372353535851937790883648493, p = 2^255 - 19.
    let consts = v.get("constants");
    let l_low: u128 = 27742317777372353535851937790883648493;
    let mut want_l = [0u8; 32];
    want_l[..16].copy_from_slice(&l_low.to_le_bytes());
    want_l[31] = 0x10; // 2^252
    assert_eq!(consts.get("group_order_l_le").h32(), want_l);
    let mut want_p = [0xffu8; 32];
    want_p[0] = 0xed;
    want_p[31] = 0x7f;
    assert_eq!(consts.get("field_prime_p_le").h32(), want_p);

    let cases = v.get("ed25519_strict").arr();
    let mut accepted = 0;
    let mut rejected = 0;
    for e in cases {
        let name = e.get("name").str();
        let node = NodeId::from(e.get("public_key").h32());
        let msg = e.get("message").hex();
        let sig = e.get("signature").h64();
        let tok = verify_tok(verify_strict(&node, &msg, &sig));
        assert_eq!(tok, e.get("expect").str(), "{name}");
        // the tagged entry point with an empty tag is the same predicate.
        assert_eq!(
            verify_tok(verify_strict_tagged(&node, b"", &msg, &sig)),
            tok,
            "{name} (tagged)"
        );
        if tok == "ok" {
            accepted += 1;
        } else {
            rejected += 1;
        }
    }
    assert_eq!(accepted, 1, "exactly the positive control verifies");
    assert!(rejected >= 35);

    // every rejection class from section 14's list is present.
    let all: String = cases
        .iter()
        .map(|e| e.get("expect").str().to_string())
        .collect::<Vec<_>>()
        .join(",");
    for class in [
        "NonCanonicalS",
        "NonCanonicalR",
        "NonCanonicalPublicKey",
        "MalformedPublicKey",
        "InvalidSignature",
    ] {
        assert!(all.contains(class), "edge corpus lacks {class}");
    }
}

#[test]
fn torsion_subgroup_is_exactly_the_eight_small_order_points() {
    // The frozen torsion list must be the full small-order subgroup: eight
    // distinct canonical encodings, each rejected as a verification key.
    let v = load("edge-vectors.json");
    let tor = v.get("torsion_subgroup").arr();
    assert_eq!(tor.len(), 8);
    let encs: BTreeSet<String> = tor
        .iter()
        .map(|e| e.get("encoding").str().to_string())
        .collect();
    assert_eq!(encs.len(), 8, "eight distinct encodings");
    let msg = b"torsion";
    for e in tor {
        let a = NodeId::from(e.get("encoding").h32());
        for r in tor {
            let mut sig = [0u8; 64];
            sig[..32].copy_from_slice(&r.get("encoding").h32());
            // S = 0 and S = 1: no small-order A, whatever R, may ever verify.
            assert!(
                verify_strict(&a, msg, &sig).is_err(),
                "A={} R={}",
                e.get("name").str(),
                r.get("name").str()
            );
            sig[32] = 1;
            assert!(verify_strict(&a, msg, &sig).is_err());
        }
    }
}

#[test]
fn x25519_low_order_rejected_everywhere() {
    let v = load("edge-vectors.json");
    let me = keyset_words("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about");
    let sign_pub = *me.public().sign_pubkey();
    let low = v.get("x25519_low_order").arr();
    assert!(low.len() >= 11);
    for e in low {
        let name = e.get("name").str();
        let u = e.get("u").h32();
        assert_eq!(e.get("expect").str(), "LowOrderPublicKey");
        // ingress validation
        assert_eq!(
            validate_x25519_public_key(&u),
            Err(EcdhError::LowOrderPublicKey),
            "{name}"
        );
        // exchange-time re-check (the all-zero-DH abort is behind this gate;
        // the zero secret can never be returned)
        assert!(
            matches!(me.ecdh(&u), Err(EcdhError::LowOrderPublicKey)),
            "{name}: ecdh"
        );
        // pinning/ingress of a public keyset carrying it
        let mut ks64 = [0u8; 64];
        ks64[..32].copy_from_slice(&sign_pub);
        ks64[32..].copy_from_slice(&u);
        assert!(
            matches!(
                PublicKeyset::from_bytes(&ks64),
                Err(EcdhError::LowOrderPublicKey)
            ),
            "{name}: keyset ingress"
        );
        // sealing to it is refused
        let hdr = SealHeader::v1(1);
        assert!(
            matches!(
                seal::seal(&u, &hdr, &me.node_id(), b"x"),
                Err(SealError::Ecdh(EcdhError::LowOrderPublicKey))
            ),
            "{name}: seal"
        );
    }
    for e in v.get("x25519_accepted_controls").arr() {
        let u = e.get("u").h32();
        assert!(
            validate_x25519_public_key(&u).is_ok(),
            "{}",
            e.get("name").str()
        );
        assert!(me.ecdh(&u).is_ok(), "{}", e.get("name").str());
    }
}

#[test]
fn weak_key_self_test_vectors() {
    let v = load("weak-key.json");
    for e in v.get("reject").arr() {
        let name = e.get("name").str();
        let k = e.get("private_keyset").h64();
        let tok = match weak_key_self_test(&k) {
            Ok(()) => "ok".to_string(),
            Err(w) => format!("{w:?}"),
        };
        assert_eq!(tok, e.get("expect").str(), "{name}");
        // the section 13 load path applies the same gate.
        assert!(
            matches!(Keyset::from_bytes(&k), Err(KeysetError::WeakKey(w)) if format!("{w:?}") == e.get("expect").str()),
            "{name}: Keyset::from_bytes"
        );
    }
    for e in v.get("accept").arr() {
        let name = e.get("name").str();
        let k = e.get("private_keyset").h64();
        assert_eq!(weak_key_self_test(&k), Ok(()), "{name}");
        assert!(Keyset::from_bytes(&k).is_ok(), "{name}");
    }
    // the variants named in section 4.5 are all exercised by the corpus.
    let kinds: BTreeSet<String> = v
        .get("reject")
        .arr()
        .iter()
        .map(|e| e.get("expect").str().to_string())
        .collect();
    for want in [
        WeakKeyError::AllZero,
        WeakKeyError::UniformByte,
        WeakKeyError::CounterPattern,
    ] {
        assert!(kinds.contains(&format!("{want:?}")), "{want:?} not covered");
    }
}

// ===========================================================================
// 8. Announce (section 6.2)
// ===========================================================================

fn signer(v: &V, which: &str) -> Keyset {
    let s = v.get("signers").get(which);
    let ks = keyset_words(s.get("mnemonic").str());
    assert_eq!(
        ks.node_id().as_bytes(),
        &s.get("node_id").h32(),
        "signer {which} NodeId"
    );
    ks
}

#[test]
fn announce_kat_round_trip() {
    let v = load("announce.json");
    assert_eq!(v.get("tag").str().as_bytes(), announce::ANNOUNCE_TAG);
    let a = signer(&v, "a");
    let kat = v.get("kat");
    let wire = kat.get("wire").hex();

    // encode: deterministic Ed25519 => the frozen bytes come out of create().
    let counter = kat.get("counter").dec();
    let appdata = kat.get("appdata").hex();
    let created = Announce::create(&a, counter, &appdata).unwrap();
    assert_eq!(created.encode(), wire, "announce wire bytes");
    assert_eq!(&created.signature()[..], &kat.get("signature").h64()[..]);

    // layout (section 6.2 table).
    assert_eq!(wire.len(), announce::BASE_LEN + appdata.len());
    assert_eq!(wire[0], announce::VERSION_V1);
    assert_eq!(&wire[1..33], a.node_id().as_bytes());
    assert_eq!(&wire[33..65], &a.x25519_public());
    assert_eq!(&wire[65..73], &counter.to_le_bytes());
    assert_eq!(&wire[73..73 + appdata.len()], &appdata[..]);
    assert!(
        !wire.starts_with(announce::ANNOUNCE_TAG),
        "the tag is never a wire field"
    );

    // signed input = tag || wire[..len-64], and it verifies RAW under the strict predicate.
    let signed = kat.get("signed_input").hex();
    assert_eq!(
        signed,
        cat(&[announce::ANNOUNCE_TAG, &wire[..wire.len() - 64]])
    );
    verify_strict(&a.node_id(), &signed, &kat.get("signature").h64()).unwrap();

    // parse + verify.
    let parsed = Announce::parse(&wire).unwrap();
    assert_eq!(parsed, created);
    assert_eq!(parsed.node_id(), a.node_id());
    assert_eq!(parsed.enc_pubkey(), &a.x25519_public());
    assert_eq!(parsed.counter(), counter);
    assert_eq!(parsed.appdata(), &appdata[..]);
    parsed.verify().unwrap();
    assert_eq!(parsed.encode(), wire);
}

#[test]
fn announce_positive_vectors_round_trip_and_pin() {
    let v = load("announce.json");
    let a = signer(&v, "a");
    let own = signer(&v, "b").node_id();
    for e in v.get("positives").arr() {
        let name = e.get("name").str();
        let wire = e.get("wire").hex();
        let counter = e.get("counter").dec();
        let appdata = e.get("appdata").hex();
        assert!(
            (announce::BASE_LEN..=announce::MAX_LEN).contains(&wire.len()),
            "{name}"
        );
        assert_eq!(
            Announce::create(&a, counter, &appdata).unwrap().encode(),
            wire,
            "{name}"
        );
        let parsed = Announce::parse(&wire).unwrap();
        parsed.verify().unwrap();
        assert_eq!(parsed.counter(), counter);
        assert_eq!(parsed.appdata(), &appdata[..]);
        assert_eq!(parsed.encode(), wire);
        let t = TempDir::new("annpos");
        let mut store = PinStore::open(t.path()).unwrap();
        assert_eq!(
            ann_tok(process_announce(&mut store, &own, &wire)),
            "ok:pinned",
            "{name}"
        );
        let pinned = store.get(&a.node_id()).expect("pinned");
        assert_eq!(pinned.enc_pubkey(), &a.x25519_public());
        assert_eq!(pinned.highest_announce_counter(), counter);
    }
    // appdata bound is enforced at construction too.
    assert!(matches!(
        Announce::create(&a, 1, &[0u8; 33]),
        Err(AnnounceError::AppdataTooLong { got: 33 })
    ));
}

#[test]
fn announce_negatives_each_rejected_before_any_state_change() {
    let v = load("announce.json");
    let own = signer(&v, "b").node_id();
    let mal = v.get("malformed").arr();
    assert!(mal.len() >= 20);
    let mut classes = BTreeSet::new();
    for e in mal {
        let name = e.get("name").str();
        let wire = e.get("wire").hex();
        let t = TempDir::new("annneg");
        let mut store = PinStore::open(t.path()).unwrap();
        let tok = ann_tok(process_announce(&mut store, &own, &wire));
        assert_eq!(tok, e.get("expect").str(), "{name}");
        assert!(
            store.is_empty(),
            "{name}: a rejected announce must not touch the store"
        );
        assert!(
            !t.path().join("peers").exists(),
            "{name}: nothing may be persisted"
        );
        classes.insert(class_of(&tok));
    }
    // downgrade/unknown version, domain-tag confusion, length, low-order, strict predicate.
    for want in [
        "err:version",
        "err:sig:InvalidSignature",
        "err:length",
        "err:loworder",
        "err:sig:NonCanonicalS",
    ] {
        assert!(
            classes.contains(want),
            "announce corpus lacks {want}: {classes:?}"
        );
    }
}

#[test]
fn announce_stateful_scenarios() {
    let v = load("announce.json");
    for sc in v.get("scenarios").arr() {
        let name = sc.get("name").str();
        let own = NodeId::from(sc.get("own_node_id").h32());
        let t = TempDir::new("annscn");
        let mut store = PinStore::open(t.path()).unwrap();
        for (i, step) in sc.get("steps").arr().iter().enumerate() {
            let tok = ann_tok(process_announce(&mut store, &own, &step.get("wire").hex()));
            assert_eq!(tok, step.get("expect").str(), "{name}: step {i}");
        }
        // reload from disk: the pins survived with the exact counters.
        drop(store);
        let reopened = PinStore::open(t.path()).unwrap();
        if name.starts_with("self-NodeId") {
            assert!(
                reopened.is_empty(),
                "the receiver's own NodeId is never pinned"
            );
        } else {
            assert!(!reopened.is_empty());
        }
    }
}

#[test]
fn announce_counter_is_the_announce_sequence_not_traffic() {
    // The announce counter (section 6.2) and the per-peer traffic counters
    // (section 6.5) are distinct state: issuing announces does not move any
    // peer's traffic window and vice versa.
    let t = TempDir::new("annctr");
    let mut cs = CounterStore::create(t.path()).unwrap();
    let peer = NodeId::from([0x42u8; 32]);
    assert_eq!(cs.next_announce_counter().unwrap(), 1);
    assert_eq!(cs.next_send_counter(&peer).unwrap(), 1);
    assert_eq!(cs.next_announce_counter().unwrap(), 2);
    assert_eq!(cs.next_send_counter(&peer).unwrap(), 2);
    assert_eq!(cs.announce_last(), 2);
}

// ===========================================================================
// 9. Sealed packet (section 6.4)
// ===========================================================================

/// Every intermediate of the section 6.4 recipe, as computed by the in-test model.
struct SealModel {
    packet: Vec<u8>,
    epk: [u8; 32],
    shared: [u8; 32],
    nonce: [u8; 24],
    key: [u8; 32],
}

/// Independent model of the section 6.4 recipe, built straight from the
/// primitive crates (NOT from id111).
fn seal_model(esk: &[u8; 32], rpk: &[u8; 32], header: &[u8], inner: &[u8]) -> SealModel {
    use blake2::digest::consts::{U24, U32};
    use blake2::{Blake2b, Digest};
    use chacha20poly1305::aead::{Aead, Payload};
    use chacha20poly1305::{KeyInit, XChaCha20Poly1305};
    let secret = x25519_dalek::StaticSecret::from(*esk);
    let epk = x25519_dalek::PublicKey::from(&secret).to_bytes();
    let shared = secret
        .diffie_hellman(&x25519_dalek::PublicKey::from(*rpk))
        .to_bytes();
    assert_ne!(shared, [0u8; 32], "step 2: abort on all-zero shared");
    let mut kh = Blake2b::<U32>::new();
    kh.update(b"111/seal/v1");
    kh.update(shared);
    kh.update(epk);
    kh.update(rpk);
    let key: [u8; 32] = kh.finalize().into();
    let mut nh = Blake2b::<U24>::new();
    nh.update(b"111/seal-n/v1");
    nh.update(epk);
    nh.update(rpk);
    let nonce: [u8; 24] = nh.finalize().into();
    let ct = XChaCha20Poly1305::new((&key).into())
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: inner,
                aad: header,
            },
        )
        .expect("encrypt");
    let mut packet = header.to_vec();
    packet.extend_from_slice(&epk);
    packet.extend_from_slice(&ct);
    SealModel {
        packet,
        epk,
        shared,
        nonce,
        key,
    }
}

fn sealed_recipient(v: &V) -> Keyset {
    let r = v.get("recipient");
    let ks = keyset_from(&r.get("ed25519_seed").h32(), &r.get("x25519_scalar").h32());
    assert_eq!(ks.x25519_public(), r.get("x25519_public").h32());
    ks
}

#[test]
fn sealed_packet_known_answer_matches_the_spec_recipe_and_the_crate() {
    let v = load("sealed-packet.json");
    let kat = v.get("kat");
    let bob = sealed_recipient(&v);
    let (esk, rpk) = (kat.get("esk").h32(), kat.get("rpk").h32());
    let sender = kat.get("sender_node_id").h32();
    let plaintext = kat.get("plaintext").hex();
    let header = kat.get("header");
    let (hv, hm, hc) = (
        header.get("version").num() as u8,
        header.get("crypto_mode").num() as u8,
        header.get("counter").dec(),
    );
    let hextra = header.get("extra").hex();

    // header serialization: version || crypto_mode || counter(LE) || extra_len || extra.
    let mut header_bytes = vec![hv, hm];
    header_bytes.extend_from_slice(&hc.to_le_bytes());
    header_bytes.push(hextra.len() as u8);
    header_bytes.extend_from_slice(&hextra);
    assert_eq!(header_bytes, kat.get("header_bytes").hex());

    // inner pin: sender NodeId inside the ciphertext.
    let inner = cat(&[&sender, &plaintext]);
    assert_eq!(inner, kat.get("inner_plaintext").hex());

    // (1) the spec recipe, recomputed independently from the primitives.
    let SealModel {
        packet,
        epk,
        shared,
        nonce,
        key,
    } = seal_model(&esk, &rpk, &header_bytes, &inner);
    assert_eq!(epk, kat.get("epk").h32());
    assert_eq!(shared, kat.get("shared").h32());
    assert_eq!(key, kat.get("key").h32());
    assert_eq!(nonce.to_vec(), kat.get("nonce").hex());
    assert_eq!(
        packet,
        kat.get("packet").hex(),
        "section 6.4 recipe vs frozen packet"
    );
    let hl = header_bytes.len();
    assert_eq!(&packet[..hl], &header_bytes[..]);
    assert_eq!(&packet[hl..hl + 32], &epk);
    assert_eq!(&packet[hl + 32..], &kat.get("ciphertext_and_tag").hex()[..]);
    assert_eq!(
        packet.len(),
        hl + 32 + inner.len() + 16,
        "epk + 16B tag overhead"
    );

    // (2) the crate: open() the frozen packet.
    let opened = seal::open(&bob, &packet).unwrap();
    assert_eq!(opened.header.version, hv);
    assert_eq!(opened.header.crypto_mode, hm);
    assert_eq!(opened.header.counter, hc);
    assert_eq!(opened.header.extra, hextra);
    assert_eq!(opened.sender.as_bytes(), &sender);
    assert_eq!(opened.plaintext, plaintext);

    // (3) the sender NodeId is NOT in the cleartext header/epk region.
    assert!(
        !packet.windows(32).any(|w| w == sender),
        "sender NodeId must travel only inside the ciphertext"
    );
}

#[test]
fn sealed_packet_fresh_round_trips() {
    let v = load("sealed-packet.json");
    let bob = sealed_recipient(&v);
    let sender = NodeId::from([0x11u8; 32]);
    let bob_pub = bob.x25519_public();
    for extra_len in [0usize, 2, 255] {
        let hdr = SealHeader {
            version: 1,
            crypto_mode: 1,
            counter: 0xfeed_beef_0000_0001,
            extra: vec![0xa5; extra_len],
        };
        for pt in [&b""[..], b"x", &[0x42u8; 300]] {
            let p1 = seal::seal(&bob_pub, &hdr, &sender, pt).unwrap();
            let p2 = seal::seal(&bob_pub, &hdr, &sender, pt).unwrap();
            assert_ne!(p1, p2, "fresh ephemeral per packet");
            assert_eq!(
                p1.len(),
                seal::HEADER_FIXED_LEN + extra_len + seal::EPK_LEN + 32 + pt.len() + seal::TAG_LEN
            );
            let o = seal::open(&bob, &p1).unwrap();
            assert_eq!(o.header, hdr);
            assert_eq!(o.sender, sender);
            assert_eq!(o.plaintext, pt);
        }
    }
}

#[test]
fn sealed_packet_negatives() {
    let v = load("sealed-packet.json");
    let bob = sealed_recipient(&v);
    let kat_packet = v.get("kat").get("packet").hex();

    // wrong recipient: authentic packet, different keyset => AuthFailed, no plaintext.
    let wr = v.get("wrong_recipient");
    let other = keyset_words(wr.get("mnemonic").str());
    assert_ne!(other.x25519_public(), bob.x25519_public());
    assert_eq!(
        open_tok(seal::open(&other, &kat_packet)),
        wr.get("expect").str()
    );

    let negs = v.get("negatives").arr();
    assert!(negs.len() >= 18);
    let mut classes = BTreeSet::new();
    for e in negs {
        let tok = open_tok(seal::open(&bob, &e.get("packet").hex()));
        assert_eq!(tok, e.get("expect").str(), "{}", e.get("name").str());
        classes.insert(
            tok.split(':').next().unwrap().to_string() + ":" + tok.split(':').nth(1).unwrap_or(""),
        );
    }
    for want in [
        "err:auth",
        "err:mode",
        "err:truncated",
        "err:ecdh",
        "err:short-plaintext",
    ] {
        assert!(
            classes.iter().any(|c| c.starts_with(want)),
            "sealed corpus lacks {want}: {classes:?}"
        );
    }

    // exhaustive: flipping ANY single bit-0 of ANY byte of the frozen packet fails closed.
    for i in 0..kat_packet.len() {
        let mut p = kat_packet.clone();
        p[i] ^= 1;
        assert!(seal::open(&bob, &p).is_err(), "byte {i} flip must not open");
    }
    // exhaustive: no strict prefix of the packet opens.
    for n in 0..kat_packet.len() {
        assert!(
            seal::open(&bob, &kat_packet[..n]).is_err(),
            "prefix {n} must not open"
        );
    }

    // seal-side refusals.
    let sender = NodeId::from([0x11u8; 32]);
    for e in v.get("seal_negatives").arr() {
        let hdr = SealHeader {
            version: 1,
            crypto_mode: e.get("crypto_mode").num() as u8,
            counter: 1,
            extra: vec![0; e.get("extra_len").num() as usize],
        };
        let tok = match seal::seal(&e.get("recipient").h32(), &hdr, &sender, b"x") {
            Ok(_) => "ok".to_string(),
            Err(SealError::UnsupportedCryptoMode(m)) => format!("UnsupportedCryptoMode:{m}"),
            Err(SealError::HeaderTooLarge(n)) => format!("HeaderTooLarge:{n}"),
            Err(SealError::Ecdh(e)) => format!("Ecdh:{e:?}"),
            Err(other) => format!("{other:?}"),
        };
        assert_eq!(tok, e.get("expect").str(), "{}", e.get("name").str());
    }
}

#[test]
fn sealed_packet_replay_decrypts_but_counters_reject_it() {
    // Section 6.4/6.5 division of labour: open() is replay-blind by design;
    // the per-peer counter window rejects the second delivery.
    let v = load("sealed-packet.json");
    let bob = sealed_recipient(&v);
    let packet = v.get("kat").get("packet").hex();
    let t = TempDir::new("replay");
    let mut cs = CounterStore::create(t.path()).unwrap();

    let first = seal::open(&bob, &packet).unwrap();
    cs.on_receive(&first.sender, first.header.counter).unwrap();
    let again = seal::open(&bob, &packet).unwrap();
    assert_eq!(again, first, "a replayed packet decrypts identically");
    assert!(matches!(
        cs.on_receive(&again.sender, again.header.counter),
        Err(CounterError::Window(WindowReject::Replayed))
    ));
}

// ===========================================================================
// 10. Backup format & restore (sections 8.2-8.4)
// ===========================================================================

fn opts(floor: CounterFloor) -> RestoreOptions {
    RestoreOptions {
        old_instance_retired: true,
        descriptor: None,
        counter_floor: floor,
    }
}

#[test]
fn backup_format_known_answers() {
    let v = load("backup-format.json");
    for e in v.get("kats").arr() {
        let name = e.get("name").str();
        let words = e.get("mnemonic").str();
        let m = RootMnemonic::parse(words).unwrap();
        let ks = Keyset::derive_from_root(&m).unwrap();
        let art = BackupArtifacts::new(&m);

        assert_eq!(&m.entropy()[..], &e.get("entropy").hex()[..], "{name}");
        assert_eq!(
            art.descriptor.to_string(),
            e.get("descriptor").str(),
            "{name}"
        );
        assert_eq!(
            art.descriptor.to_bytes().to_vec(),
            e.get("descriptor_file").hex()
        );
        assert_eq!(art.descriptor, BackupDescriptor::for_mnemonic(&m));
        assert_eq!(art.words(), words);
        // QR payload = 4B descriptor || raw entropy (20B / 36B).
        let qr = e.get("qr_payload").hex();
        assert_eq!(art.qr_payload(), &qr[..], "{name}");
        assert_eq!(
            qr,
            cat(&[&e.get("descriptor_file").hex(), &e.get("entropy").hex()])
        );
        assert!(matches!(qr.len(), 20 | 36));
        let (d, entropy) = backup::parse_qr_payload(&qr).unwrap();
        assert_eq!(d, art.descriptor);
        assert_eq!(&entropy[..], &e.get("entropy").hex()[..]);
        assert!(backup::qr_payload_matches(&m, &qr));
        let mut bad = qr.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(!backup::qr_payload_matches(&m, &bad));

        // identity-dir file bodies and the NodeId presentation surface.
        let d = NodeIdDisplay::new(ks.node_id().as_bytes());
        assert_eq!(d.hex(), e.get("node_id_hex").str());
        assert_eq!(d.bech32(), e.get("node_id_bech32").str());
        assert_eq!(d.label(), e.get("label").str());
        assert_eq!(d.record().to_vec(), e.get("nodeid_file").hex());
        assert_eq!(&ks.to_bytes()[..], &e.get("keyset_file").hex()[..]);
        assert_eq!(format!("{words}\n"), e.get("mnemonic_file_body").str());
    }
}

#[test]
fn backup_restore_round_trip_identical_nodeid_and_files() {
    use std::os::unix::fs::PermissionsExt;
    let v = load("backup-format.json");
    for e in v.get("kats").arr() {
        let name = e.get("name").str();
        let words = e.get("mnemonic").str();
        let want_node = e.get("node_id_hex").str();
        let qr = e.get("qr_payload").hex();

        for (path_name, from_qr) in [("words", false), ("qr", true)] {
            let t = TempDir::new("restore");
            let dir = t.identity();
            let out = if from_qr {
                backup::restore_from_qr(&dir, &qr, &opts(CounterFloor::NeverDeployed))
            } else {
                backup::restore(&dir, words, &opts(CounterFloor::NeverDeployed))
            }
            .unwrap_or_else(|e| panic!("{name}/{path_name}: {e}"));

            // identical NodeId, confirmation surface included (section 8.4).
            assert_eq!(
                to_hex(out.node_id.as_bytes()),
                want_node,
                "{name}/{path_name}"
            );
            assert_eq!(out.display().hex(), want_node);
            assert_eq!(out.display().label(), e.get("label").str());
            assert_eq!(out.descriptor.to_string(), e.get("descriptor").str());
            assert_eq!(
                out.announce_counter_init, 1024,
                "never-deployed floor 0 + 1024"
            );
            assert!(!out.counter_floor_pending);

            // the five section 13 files, byte-exact against the frozen bodies.
            assert_eq!(
                fs::read_to_string(dir.join("mnemonic")).unwrap(),
                e.get("mnemonic_file_body").str()
            );
            assert_eq!(
                fs::read(dir.join("keyset")).unwrap(),
                e.get("keyset_file").hex()
            );
            assert_eq!(
                fs::read(dir.join("nodeid")).unwrap(),
                e.get("nodeid_file").hex()
            );
            assert_eq!(
                fs::read(dir.join("descriptor")).unwrap(),
                e.get("descriptor_file").hex()
            );
            assert_eq!(CounterStore::open(&dir).unwrap().announce_last(), 1024);
            // section 13 modes: secrets 0600, public files 0644, dir 0700.
            let mode = |p: &str| fs::metadata(dir.join(p)).unwrap().permissions().mode() & 0o777;
            assert_eq!(
                (mode("mnemonic"), mode("keyset"), mode("counters")),
                (0o600, 0o600, 0o600)
            );
            assert_eq!((mode("nodeid"), mode("descriptor")), (0o644, 0o644));
            assert_eq!(
                fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
                0o700
            );

            // the restored keyset file loads and signs as the same node.
            let ks = Keyset::from_bytes(&fs::read(dir.join("keyset")).unwrap().try_into().unwrap())
                .unwrap();
            assert_eq!(to_hex(ks.node_id().as_bytes()), want_node);
        }
    }
}

#[test]
fn backup_descriptor_absent_falls_back_to_v1_defaults() {
    let v = load("backup-format.json");
    for e in v.get("kats").arr() {
        let name = e.get("name").str();
        let t = TempDir::new("nodesc");
        // descriptor: None  =>  v1 defaults; the encoding byte follows the actual word count.
        let out = backup::restore(
            &t.identity(),
            e.get("mnemonic").str(),
            &opts(CounterFloor::NeverDeployed),
        )
        .unwrap();
        assert_eq!(
            out.descriptor.to_string(),
            e.get("descriptor").str(),
            "{name}"
        );
        assert_eq!(
            fs::read(t.identity().join("descriptor")).unwrap(),
            e.get("descriptor_file").hex()
        );
        let want = if e.get("entropy").hex().len() == 16 {
            WordCount::Twelve
        } else {
            WordCount::TwentyFour
        };
        assert_eq!(out.descriptor.word_count(), want);
        assert_eq!(out.descriptor.version(), 1);

        // a correct supplied descriptor is honored identically.
        let t2 = TempDir::new("desc-ok");
        let mut o = opts(CounterFloor::NeverDeployed);
        o.descriptor = Some(e.get("descriptor_file").hex().try_into().unwrap());
        let out2 = backup::restore(&t2.identity(), e.get("mnemonic").str(), &o).unwrap();
        assert_eq!(out2.descriptor, out.descriptor);
    }
}

#[test]
fn backup_fail_closed_refusals() {
    let v = load("backup-format.json");
    let w12 = v.get("kats").arr()[0].get("mnemonic").str().to_string();
    let w24 = v.get("kats").arr()[1].get("mnemonic").str().to_string();
    let qr12 = v.get("kats").arr()[0].get("qr_payload").hex();

    // unknown descriptor fields / bad lengths.
    for e in v.get("descriptor_negatives").arr() {
        let tok = match BackupDescriptor::from_bytes(&e.get("bytes").hex()) {
            Ok(_) => "ok".to_string(),
            Err(err) => format!("{err:?}"),
        };
        assert_eq!(tok, e.get("expect").str(), "{}", e.get("name").str());
    }
    // QR payload refusals.
    for e in v.get("qr_negatives").arr() {
        let tok = match backup::parse_qr_payload(&e.get("payload").hex()) {
            Ok(_) => "ok".to_string(),
            Err(err) => format!("{err:?}"),
        };
        assert_eq!(tok, e.get("expect").str(), "{}", e.get("name").str());
    }
    // checksum failure and friends: refused, and NOTHING is written.
    for e in v.get("restore_negatives").arr() {
        let t = TempDir::new("rneg");
        let tok = restore_tok(backup::restore(
            &t.identity(),
            e.get("words").str(),
            &opts(CounterFloor::NeverDeployed),
        ));
        assert_eq!(tok, e.get("expect").str(), "{}", e.get("name").str());
        assert!(
            !t.identity().exists(),
            "{}: a refused restore writes nothing",
            e.get("name").str()
        );
    }

    // unknown descriptor field supplied at restore time: fail closed.
    let t = TempDir::new("udesc");
    let mut o = opts(CounterFloor::NeverDeployed);
    o.descriptor = Some([0x01, 0x01, 0x99, 0x01]);
    assert_eq!(
        restore_tok(backup::restore(&t.identity(), &w12, &o)),
        "Descriptor:UnknownDerivation(153)"
    );
    assert!(!t.identity().exists());

    // 12-word descriptor over 24 words: never reinterpreted.
    let t = TempDir::new("dmis");
    let mut o = opts(CounterFloor::NeverDeployed);
    o.descriptor = Some(BackupDescriptor::V1_12_WORDS.to_bytes());
    assert_eq!(
        restore_tok(backup::restore(&t.identity(), &w24, &o)),
        "DescriptorWordCountMismatch:24"
    );
    assert!(!t.identity().exists());

    // QR payload vs a conflicting separately supplied descriptor.
    let t = TempDir::new("dconf");
    let mut o = opts(CounterFloor::NeverDeployed);
    o.descriptor = Some(BackupDescriptor::V1_24_WORDS.to_bytes());
    assert_eq!(
        restore_tok(backup::restore_from_qr(&t.identity(), &qr12, &o)),
        "DescriptorConflict"
    );
    assert!(!t.identity().exists());

    // contradictory / malformed QR payloads through restore_from_qr.
    let t = TempDir::new("qrbad");
    assert_eq!(
        restore_tok(backup::restore_from_qr(
            &t.identity(),
            &qr12[..19],
            &opts(CounterFloor::NeverDeployed)
        )),
        "QrPayload"
    );
    assert!(!t.identity().exists());

    // old-instance retirement is a hard precondition (two live instances flap the fleet).
    let t = TempDir::new("retire");
    let mut o = opts(CounterFloor::NeverDeployed);
    o.old_instance_retired = false;
    assert_eq!(
        restore_tok(backup::restore(&t.identity(), &w12, &o)),
        "OldInstanceNotRetired"
    );
    assert_eq!(
        restore_tok(backup::restore_from_qr(&t.identity(), &qr12, &o)),
        "OldInstanceNotRetired"
    );
    assert!(!t.identity().exists());

    // never overwrite a live, unquarantined identity dir (words AND QR paths).
    let t = TempDir::new("live");
    backup::restore(&t.identity(), &w12, &opts(CounterFloor::NeverDeployed)).unwrap();
    let before = fs::read(t.identity().join("keyset")).unwrap();
    assert_eq!(
        restore_tok(backup::restore(
            &t.identity(),
            &w24,
            &opts(CounterFloor::NeverDeployed)
        )),
        "LiveIdentityDir"
    );
    assert_eq!(
        restore_tok(backup::restore_from_qr(
            &t.identity(),
            &qr12,
            &opts(CounterFloor::NeverDeployed)
        )),
        "LiveIdentityDir"
    );
    assert_eq!(
        fs::read(t.identity().join("keyset")).unwrap(),
        before,
        "live identity untouched"
    );
    // a dir holding only D9-legacy material is equally live.
    let t = TempDir::new("livelegacy");
    fs::create_dir_all(t.identity()).unwrap();
    fs::write(t.identity().join("secret.hex"), "00\n").unwrap();
    assert_eq!(
        restore_tok(backup::restore(
            &t.identity(),
            &w12,
            &opts(CounterFloor::NeverDeployed)
        )),
        "LiveIdentityDir"
    );
}

#[test]
fn backup_restore_input_hygiene() {
    let v = load("backup-format.json");
    for e in v.get("input_hygiene").arr() {
        let name = e.get("name").str();
        let canonical = e.get("canonical").str();
        let want = keyset_words(canonical).node_id();
        let t = TempDir::new("hyg");
        let out = backup::restore(
            &t.identity(),
            e.get("words").str(),
            &opts(CounterFloor::NeverDeployed),
        )
        .unwrap_or_else(|err| panic!("{name}: {err}"));
        assert_eq!(out.node_id, want, "{name}");
        // the mnemonic file always stores the canonical NFKC space-joined phrase.
        assert_eq!(
            fs::read_to_string(t.identity().join("mnemonic")).unwrap(),
            format!("{canonical}\n"),
            "{name}"
        );
    }
}

#[test]
fn backup_verify_on_write_down() {
    let v = load("backup-format.json");
    let wd = v.get("write_down");
    let m = RootMnemonic::parse(wd.get("mnemonic").str()).unwrap();
    let positions: Vec<usize> = wd
        .get("positions")
        .arr()
        .iter()
        .map(|n| n.num() as usize)
        .collect();
    let strs = |k: &str| -> Vec<String> {
        wd.get(k)
            .arr()
            .iter()
            .map(|s| s.str().to_string())
            .collect()
    };
    let as_refs = |v: &[String]| -> Vec<String> { v.to_vec() };
    let check = |answers: Vec<String>| {
        let refs: Vec<&str> = answers.iter().map(String::as_str).collect();
        backup::check_words(&m, &positions, &refs).expect("well-formed check")
    };
    assert!(
        check(as_refs(&strs("answers"))),
        "correct sampled words pass"
    );
    assert!(
        check(as_refs(&strs("messy_answers"))),
        "section 8.4 hygiene applies to typed answers"
    );
    assert!(
        !check(as_refs(&strs("wrong_answers"))),
        "a wrong word fails the gate (not an error)"
    );

    // caller bugs are errors, never a silent pass.
    assert!(backup::check_words(&m, &[12], &["abandon"]).is_err());
    assert!(backup::check_words(&m, &positions, &["abandon"]).is_err());

    // sampling: unique, ascending, in range, requested count.
    for wc in [WordCount::Twelve, WordCount::TwentyFour] {
        for sample in [1, backup::DEFAULT_SAMPLE_COUNT, wc.word_count()] {
            let p = backup::sample_positions(wc, sample).unwrap();
            assert_eq!(p.len(), sample);
            assert!(p.windows(2).all(|w| w[0] < w[1]), "strictly ascending");
            assert!(p.iter().all(|&i| i < wc.word_count()));
        }
        assert!(backup::sample_positions(wc, 0).is_err());
        assert!(backup::sample_positions(wc, wc.word_count() + 1).is_err());
    }
}

// ===========================================================================
// 11. Attestation (section 10)
// ===========================================================================

const TID: TransportId = TransportId::NostrSecp256k1;

/// Stand-in for Layer 7's secp256k1 verifier: accepts exactly one frozen
/// (transport_id, key, signed_input, signature) tuple. Passing proves the
/// signed input id111 assembles for the seam is byte-exact.
struct ExactVerifier {
    key: [u8; 32],
    input: Vec<u8>,
    sig: [u8; 64],
}

impl TransportVerifier for ExactVerifier {
    fn verify_transport_signature(
        &self,
        tid: TransportId,
        key: &[u8; 32],
        input: &[u8],
        sig: &[u8; 64],
    ) -> bool {
        tid == TID && key == &self.key && input == &self.input[..] && sig == &self.sig
    }
}

struct RejectAll;

impl TransportVerifier for RejectAll {
    fn verify_transport_signature(
        &self,
        _: TransportId,
        _: &[u8; 32],
        _: &[u8],
        _: &[u8; 64],
    ) -> bool {
        false
    }
}

fn att_wire_tok(w: &[u8]) -> String {
    match Attestation::from_bytes(w) {
        Err(e) => att_tok(e),
        Ok(att) => match att.verify_node_leg() {
            Ok(()) => "ok".into(),
            Err(e) => att_tok(e),
        },
    }
}

fn exact_verifier(kat: &V) -> ExactVerifier {
    ExactVerifier {
        key: kat.get("transport_key").h32(),
        input: kat.get("signed_input_transport").hex(),
        sig: kat.get("transport_sig_opaque").h64(),
    }
}

#[test]
fn attestation_kat_round_trip() {
    let v = load("attestation.json");
    let a = signer(&v, "a");
    let kat = v.get("kat");
    let wire = kat.get("wire").hex();
    let (tkey, tsig) = (
        kat.get("transport_key").h32(),
        kat.get("transport_sig_opaque").h64(),
    );
    let expiry = kat.get("expiry_epoch").dec();
    assert_eq!(kat.get("transport_id").num(), 1);
    assert_eq!(kat.get("expiry_epoch_le").hex(), expiry.to_le_bytes());

    // build: node leg (signed here) + transport leg (opaque, L7's) => 202B.
    let leg = NodeLeg::issue(&a, TID, tkey, MeshEpoch::from_u64(expiry));
    leg.verify_node_leg().unwrap();
    assert_eq!(&leg.node_sig()[..], &kat.get("node_sig").h64()[..]);
    let att = leg.complete(tsig);
    assert_eq!(att.to_bytes().to_vec(), wire, "attestation wire bytes");
    assert_eq!(wire.len(), 202);

    // normative offsets (section 10).
    assert_eq!(wire[0], 0x01);
    assert_eq!(&wire[1..33], a.node_id().as_bytes());
    assert_eq!(wire[33], 0x01);
    assert_eq!(&wire[34..66], &tkey);
    assert_eq!(&wire[66..74], &expiry.to_le_bytes());
    assert_eq!(&wire[74..138], &kat.get("node_sig").h64()[..]);
    assert_eq!(&wire[138..202], &tsig[..]);

    // signed inputs: node input order is version || node || transport_key || transport_id || expiry
    // (transport_key BEFORE transport_id: differs from the wire order).
    let mut node_body = vec![0x01u8];
    node_body.extend_from_slice(a.node_id().as_bytes());
    node_body.extend_from_slice(&tkey);
    node_body.push(0x01);
    node_body.extend_from_slice(&expiry.to_le_bytes());
    let signed_node = cat(&[b"111/attest-node/v1", &node_body]);
    assert_eq!(signed_node, kat.get("signed_input_node").hex());
    verify_strict(&a.node_id(), &signed_node, &kat.get("node_sig").h64()).unwrap();
    let signed_transport = cat(&[b"111/attest-transport/v1", a.node_id().as_bytes()]);
    assert_eq!(signed_transport, kat.get("signed_input_transport").hex());
    assert_eq!(signed_transport.len(), 55);

    // parse round trip + accessors.
    let parsed = Attestation::from_bytes(&wire).unwrap();
    assert_eq!(parsed, att);
    assert_eq!(parsed.node_id(), a.node_id());
    assert_eq!(parsed.transport_id(), TID);
    assert_eq!(parsed.transport_key(), &tkey);
    assert_eq!(parsed.expiry_epoch(), MeshEpoch::from_u64(expiry));
    assert_eq!(parsed.transport_sig(), &tsig);
    assert_eq!(parsed.to_bytes().to_vec(), wire);

    // both legs: the plug sees exactly the frozen transport signed input.
    parsed.verify_node_leg().unwrap();
    let ev = exact_verifier(kat);
    parsed.verify_transport_leg(&ev).unwrap();
    parsed
        .verify_full(&ev, MeshEpoch::from_u64(expiry - 1))
        .unwrap();
    assert!(parsed.verify_transport_leg(&RejectAll).is_err());
}

#[test]
fn attestation_negatives() {
    let v = load("attestation.json");
    let negs = v.get("negatives").arr();
    assert!(negs.len() >= 20);
    let mut classes = BTreeSet::new();
    for e in negs {
        let tok = att_wire_tok(&e.get("wire").hex());
        assert_eq!(tok, e.get("expect").str(), "{}", e.get("name").str());
        classes.insert(class_of(&tok));
    }
    for want in [
        "err:length",
        "err:version",
        "err:tid",
        "err:node-sig:InvalidSignature",
        "err:node-sig:NonCanonicalS",
    ] {
        assert!(
            classes.contains(want),
            "attestation corpus lacks {want}: {classes:?}"
        );
    }
    // the named section 14 classes are all in the corpus by name.
    let names: String = negs
        .iter()
        .map(|e| e.get("name").str().to_string())
        .collect::<Vec<_>>()
        .join("|");
    for needle in [
        "field-swap",
        "cross-node transplant",
        "domain-tag confusion",
    ] {
        assert!(names.contains(needle), "missing negative class {needle}");
    }
}

#[test]
fn attestation_expiry_and_check_ordering() {
    let v = load("attestation.json");
    let kat = v.get("kat");
    let att = Attestation::from_bytes(&kat.get("wire").hex()).unwrap();
    let ev = exact_verifier(kat);
    let expiry = kat.get("expiry_epoch").dec();

    // expiry is dead exactly AT the expiry epoch (current >= expiry_epoch).
    for c in v.get("expiry").arr() {
        let cur = c.get("current_epoch").dec();
        let tok = match att.verify_full(&ev, MeshEpoch::from_u64(cur)) {
            Ok(()) => "ok".to_string(),
            Err(e) => att_tok(e),
        };
        assert_eq!(tok, c.get("expect").str(), "current epoch {cur}");
        assert_eq!(att.is_expired(MeshEpoch::from_u64(cur)), cur >= expiry);
    }

    // the order of checks: node leg, then transport leg, then expiry.
    for c in v.get("ordering").arr() {
        let name = c.get("name").str();
        let a = Attestation::from_bytes(&c.get("wire").hex()).unwrap();
        let cur = MeshEpoch::from_u64(c.get("current_epoch").dec());
        let tok = {
            let r = match c.get("transport_verifier").str() {
                "accept" => a.verify_full(&ev, cur),
                "reject" => a.verify_full(&RejectAll, cur),
                other => panic!("unknown verifier {other}"),
            };
            match r {
                Ok(()) => "ok".to_string(),
                Err(e) => att_tok(e),
            }
        };
        assert_eq!(tok, c.get("expect").str(), "{name}");
    }
}

#[test]
fn attestation_supersedure_and_replay() {
    let v = load("attestation.json");
    let steps = v.get("supersedure").arr();
    let mut set = AttestationSet::new();
    let mut last_good: Option<MeshEpoch> = None;
    for s in steps {
        let att = Attestation::from_bytes(&s.get("wire").hex()).unwrap();
        let tok = res_tok(set.insert(att));
        assert_eq!(tok, s.get("expect").str(), "{}", s.get("name").str());
        if att.node_id() == signer(&v, "a").node_id() && tok.starts_with("ok:") {
            last_good = Some(set.get(&att.node_id(), TID).unwrap().expiry_epoch());
        }
    }
    // only the greatest-expiry attestation per (node, transport) is retained.
    let a = signer(&v, "a").node_id();
    let b = signer(&v, "b").node_id();
    assert_eq!(set.len(), 2);
    assert_eq!(
        set.get(&a, TID).unwrap().expiry_epoch(),
        MeshEpoch::from_u64(300)
    );
    assert_eq!(
        set.get(&b, TID).unwrap().expiry_epoch(),
        MeshEpoch::from_u64(1)
    );
    assert_eq!(last_good, Some(MeshEpoch::from_u64(300)));

    // full bidirectional insert: a rejecting transport verifier changes nothing.
    let att = Attestation::from_bytes(&v.get("kat").get("wire").hex()).unwrap();
    let mut fresh = AttestationSet::new();
    assert!(matches!(
        fresh.insert_with_transport(att, &RejectAll),
        Err(AttestError::InvalidTransportSignature)
    ));
    assert!(fresh.is_empty(), "rejected before any state change");
    let ev = exact_verifier(v.get("kat"));
    assert_eq!(
        fresh.insert_with_transport(att, &ev),
        Ok(Resolution::Pinned)
    );
}

// ===========================================================================
// 12. Restore wire function, sync-commit, unclean reboot (sections 6.2, 6.5, 8.4)
// ===========================================================================

#[test]
fn restore_reannounce_floor_plus_1024_then_next_is_plus_one() {
    let v = load("restore-counters.json");
    assert_eq!(v.get("restore_jump").num(), 1024);
    assert_eq!(v.get("restore_jump").num(), backup::RESTORE_COUNTER_JUMP);
    assert_eq!(v.get("restore_jump").num(), id111::counters::RESTORE_JUMP);

    let me = keyset_words("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about");
    let peer_id =
        keyset_words("legal winner thank year wave sausage worth useful legal winner thank yellow")
            .node_id();

    for e in v.get("restore_reannounce").arr() {
        let floor = e.get("floor").dec();
        let t = TempDir::new("reann");
        let mut st = CounterStore::restore_pending(t.path()).unwrap();

        // MUST NOT announce before a floor is set; nothing is persisted yet.
        assert!(st.is_restore_pending());
        assert!(matches!(
            st.next_announce_counter(),
            Err(CounterError::RestoreFloorRequired)
        ));
        assert!(!t.path().join("counters").exists());

        st.set_restore_floor(floor).unwrap();
        assert!(!st.is_restore_pending());
        assert_eq!(
            st.announce_last(),
            e.get("announce_last").dec(),
            "floor {floor}: announce_last = floor + 1024"
        );
        assert_eq!(st.traffic_base(), 1024);
        let first = st.next_announce_counter().unwrap();
        assert_eq!(
            first,
            e.get("first_announce").dec(),
            "floor {floor}: next = +1"
        );
        assert_eq!(first, floor + 1024 + 1);
        assert_eq!(st.next_announce_counter().unwrap(), first + 1);
        // the floor is set once.
        assert!(matches!(
            st.set_restore_floor(floor + 1),
            Err(CounterError::FloorAlreadySet)
        ));

        // restore -> send: after a CLEAN shutdown the first send to a NEW peer
        // resumes at base + 1 (an unclean reopen would additionally apply the
        // §6.5 jump J = N to the base — covered by the counters unit tests).
        st.shutdown().unwrap();
        let mut st2 = CounterStore::open(t.path()).unwrap();
        assert_eq!(
            st2.next_send_counter(&peer_id).unwrap(),
            e.get("first_send_to_new_peer").dec()
        );

        // wire function: a peer that last pinned `floor` accepts the restored node's announce.
        let tp = TempDir::new("reann-peer");
        let mut store = PinStore::open(tp.path()).unwrap();
        if floor > 0 {
            let pre = Announce::create(&me, floor, b"").unwrap().encode();
            assert_eq!(
                ann_tok(process_announce(&mut store, &peer_id, &pre)),
                "ok:pinned"
            );
            let post = Announce::create(&me, first, b"").unwrap().encode();
            assert_eq!(
                ann_tok(process_announce(&mut store, &peer_id, &post)),
                format!("ok:updated:{floor}")
            );
        } else {
            let post = Announce::create(&me, first, b"").unwrap().encode();
            assert_eq!(
                ann_tok(process_announce(&mut store, &peer_id, &post)),
                "ok:pinned"
            );
        }
    }
}

#[test]
fn under_floored_restore_is_stale_at_peers() {
    // Negative control for the rule above: if a node whose peers saw counter
    // 5000 restored as "never deployed" (floor 0 -> first announce 1025) it
    // would be silently ignored by every peer (stale) until it out-counted 5000.
    let me = keyset_words("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about");
    let other =
        keyset_words("legal winner thank year wave sausage worth useful legal winner thank yellow")
            .node_id();
    let tp = TempDir::new("underfloor");
    let mut store = PinStore::open(tp.path()).unwrap();
    let pre = Announce::create(&me, 5000, b"").unwrap().encode();
    assert_eq!(
        ann_tok(process_announce(&mut store, &other, &pre)),
        "ok:pinned"
    );

    let t = TempDir::new("underfloor-node");
    let mut st = CounterStore::restore_pending(t.path()).unwrap();
    st.set_restore_floor(0).unwrap();
    let wrong_first = st.next_announce_counter().unwrap();
    assert_eq!(wrong_first, 1025);
    let wire = Announce::create(&me, wrong_first, b"").unwrap().encode();
    assert_eq!(
        ann_tok(process_announce(&mut store, &other, &wire)),
        "err:stale:5000:1025"
    );
}

#[test]
fn restore_counter_edges() {
    let v = load("restore-counters.json");
    for e in v.get("restore_edges").arr() {
        let name = e.get("name").str();
        let t = TempDir::new("edge");
        let mut st = CounterStore::restore_pending(t.path()).unwrap();
        let floor = e.get("floor").dec();
        if e.has("expect_set_floor") {
            assert_eq!(e.get("expect_set_floor").str(), "Overflow");
            assert!(
                matches!(st.set_restore_floor(floor), Err(CounterError::Overflow)),
                "{name}"
            );
            assert!(
                st.is_restore_pending(),
                "{name}: a refused floor leaves the store restore-pending"
            );
        } else {
            st.set_restore_floor(floor).unwrap();
            assert_eq!(st.announce_last(), e.get("announce_last").dec(), "{name}");
            assert_eq!(e.get("expect_first_announce").str(), "Overflow");
            assert!(
                matches!(st.next_announce_counter(), Err(CounterError::Overflow)),
                "{name}: no wrap"
            );
            assert_eq!(
                st.announce_last(),
                e.get("announce_last").dec(),
                "{name}: failed issue leaves state"
            );
        }
    }
}

#[test]
fn restore_send_counter_resumption_and_window_gap() {
    // Receiver side of restore -> send: the restored sender resumes at
    // base + 1 = 1025; a receiver whose window last saw 7 accepts the gap
    // (gaps only advance windows) and thereafter treats the pre-disaster
    // counters as out-of-window.
    let mut w = ReceiveWindow::new();
    for c in 1..=7 {
        w.offer(c).unwrap();
    }
    assert_eq!(w.high(), 7);
    w.offer(1025).expect("a forward gap is accepted harmlessly");
    assert_eq!(w.high(), 1025);
    assert_eq!(w.offer(7), Err(WindowReject::OutsideWindow));
    assert_eq!(w.offer(1025), Err(WindowReject::Replayed));
    w.offer(1026).unwrap();
}

#[test]
fn restore_via_backup_api_commits_the_counter_rule() {
    let v = load("backup-format.json");
    let words = v.get("kats").arr()[0].get("mnemonic").str().to_string();
    let peer = NodeId::from([0x33u8; 32]);

    // Known floor: announce_last = floor + 1024 (persisted with the identity files).
    let t = TempDir::new("floor-known");
    let out = backup::restore(&t.identity(), &words, &opts(CounterFloor::Known(41))).unwrap();
    assert_eq!(out.announce_counter_init, 41 + 1024);
    assert!(!out.counter_floor_pending);
    let mut cs = CounterStore::open(&t.identity()).unwrap();
    assert_eq!(cs.announce_last(), 41 + 1024);
    assert_eq!(cs.traffic_base(), 1024);
    assert_eq!(cs.next_announce_counter().unwrap(), 41 + 1024 + 1);
    assert_eq!(cs.next_send_counter(&peer).unwrap(), 1025);

    // Pending mesh-learn: provisional lower bound persisted, flag raised.
    let t = TempDir::new("floor-pending");
    let out =
        backup::restore(&t.identity(), &words, &opts(CounterFloor::PendingMeshLearn)).unwrap();
    assert!(out.counter_floor_pending);
    assert_eq!(out.announce_counter_init, 1024);

    // State present (a live dir image restored at file level): counters reused as-is —
    // there is no code path in restore() for it; CounterStore::open is the reuse path.
    let t = TempDir::new("state-present");
    backup::restore(&t.identity(), &words, &opts(CounterFloor::Known(10))).unwrap();
    {
        let mut cs = CounterStore::open(&t.identity()).unwrap();
        for _ in 0..3 {
            cs.next_announce_counter().unwrap();
        }
        cs.shutdown().unwrap();
    }
    let reused = CounterStore::open(&t.identity()).unwrap();
    assert_eq!(
        reused.announce_last(),
        10 + 1024 + 3,
        "persisted counters reused as-is"
    );
}

#[test]
fn announce_counter_is_durable_before_it_is_issued_sync_commit_rule() {
    let v = load("restore-counters.json");
    let sc = v.get("sync_commit");
    let t = TempDir::new("sync");
    let mut st = CounterStore::create(t.path()).unwrap();
    assert_eq!(
        st.announce_last(),
        sc.get("fresh_identity_announce_last").num()
    );
    let seq: Vec<u64> = sc.get("sequence").arr().iter().map(V::num).collect();
    assert_eq!(seq[0], sc.get("genesis_announce_counter").num());
    for want in &seq {
        let c = st.next_announce_counter().unwrap();
        assert_eq!(c, *want);
        // crash model: a second handle reads the file while the first never
        // shut down. The counter that was JUST issued (i.e. may be transmitted
        // next) is already durable.
        let after_crash = CounterStore::open(t.path()).unwrap();
        assert_eq!(
            after_crash.announce_last(),
            c,
            "counter {c} durable before its announce can be sent"
        );
    }
    // after a crash the node resumes strictly above everything it ever issued.
    drop(st);
    let mut resumed = CounterStore::open(t.path()).unwrap();
    assert_eq!(
        resumed.next_announce_counter().unwrap(),
        seq.last().unwrap() + 1
    );
}

#[test]
fn unclean_reboot_forward_jump_applies_to_send_and_receive_never_announce() {
    let v = load("restore-counters.json");
    let ub = v.get("unclean_reboot");
    let n = ub.get("batch_n").num();
    let peer = NodeId::from([0x55u8; 32]);
    let batch = std::num::NonZeroU64::new(n).unwrap();

    // --- send side (+ the announce counter, which is never jumped) ---
    let t = TempDir::new("unclean-send");
    {
        let mut st = CounterStore::create(t.path()).unwrap();
        st.set_batch_n(batch);
        st.commit().unwrap();
        assert_eq!(st.next_announce_counter().unwrap(), 1);
        let mut last = 0;
        for _ in 0..ub.get("sends_before_crash").num() {
            last = st.next_send_counter(&peer).unwrap();
        }
        assert_eq!(last, ub.get("sends_before_crash").num());
        // CRASH: no shutdown().
    }
    let mut st = CounterStore::open(t.path()).unwrap();
    assert_eq!(
        st.peer_traffic(&peer).unwrap().send_last(),
        ub.get("last_committed_send").num() + n
    );
    let first = st.next_send_counter(&peer).unwrap();
    assert_eq!(
        first,
        ub.get("first_send_after_reboot").num(),
        "send counter jumps past every emitted value (J >= N)"
    );
    assert!(first > ub.get("sends_before_crash").num());
    assert_eq!(
        st.announce_last(),
        1,
        "the announce counter is sync-committed, never jumped"
    );

    // --- receive side: the window high-water jumps by J as well ---
    let t = TempDir::new("unclean-recv");
    {
        let mut st = CounterStore::create(t.path()).unwrap();
        st.set_batch_n(batch);
        st.commit().unwrap();
        for c in 1..=ub.get("receives_before_crash").num() {
            st.on_receive(&peer, c).unwrap();
        }
        // CRASH.
    }
    let st = CounterStore::open(t.path()).unwrap();
    let high = st.peer_traffic(&peer).unwrap().receive_window().high();
    assert_eq!(high, ub.get("receive_high_water_after_reboot").num());
    assert_eq!(high, ub.get("last_committed_receive_high").num() + n);

    // --- clean shutdown => no jump ---
    let t = TempDir::new("clean");
    {
        let mut st = CounterStore::create(t.path()).unwrap();
        for _ in 0..3 {
            st.next_send_counter(&peer).unwrap();
        }
        st.shutdown().unwrap();
    }
    let mut st = CounterStore::open(t.path()).unwrap();
    assert_eq!(
        st.next_send_counter(&peer).unwrap(),
        4,
        "clean shutdown applies no forward jump"
    );
}

// ===========================================================================
// 13. D9 migration (section 8.4, 13; D14d)
// ===========================================================================

fn write_legacy(dir: &Path, files: &[(String, String)]) {
    fs::create_dir_all(dir).unwrap();
    for (name, body) in files {
        fs::write(dir.join(name), body).unwrap();
    }
}

#[test]
fn d9_migration_round_trip() {
    let v = load("d9-migration.json");
    let legacy = v.get("legacy");
    let exp = v.get("expect");
    let words = legacy.get("mnemonic").str();

    let t = TempDir::new("d9");
    let dir = t.identity();
    let original: Vec<(String, String)> = vec![
        (
            "secret.hex".into(),
            format!("{}\n", legacy.get("secret_hex").str()),
        ),
        ("npub".into(), format!("{}\n", legacy.get("npub").str())),
        ("mnemonic".into(), format!("{words}\n")),
    ];
    write_legacy(&dir, &original);
    assert!(id111::migrate::is_legacy_dir(&dir));

    let out = id111::migrate::migrate(&dir).unwrap();

    // the old -> new mapping: same words, new derivation; the NodeId changes once.
    assert_eq!(
        to_hex(out.new_node_id.as_bytes()),
        exp.get("new_node_id").str()
    );
    assert_eq!(out.new_node_id, keyset_words(words).node_id());
    assert_eq!(out.old_npub, exp.get("old_npub").str());
    assert_eq!(out.descriptor.to_string(), exp.get("descriptor").str());
    assert_eq!(
        out.announce_counter_init,
        exp.get("announce_counter_init").num()
    );

    // quarantined whole, never deleted; legacy bytes untouched.
    let q = t
        .path()
        .join(format!("identity{}", exp.get("quarantine_suffix").str()));
    assert_eq!(out.quarantine_dir, q);
    for f in exp.get("quarantined_files").arr() {
        let name = f.str();
        let want = &original.iter().find(|(n, _)| n == name).unwrap().1;
        assert_eq!(
            &fs::read_to_string(q.join(name)).unwrap(),
            want,
            "quarantined {name}"
        );
    }
    assert!(
        !dir.join("secret.hex").exists() && !dir.join("npub").exists(),
        "legacy files are out of the live dir"
    );

    // the new section 13 file set.
    for f in exp.get("new_files").arr() {
        assert!(dir.join(f.str()).is_file(), "new file {}", f.str());
    }
    assert_eq!(
        fs::read_to_string(dir.join("mnemonic")).unwrap(),
        exp.get("new_mnemonic_file_body").str()
    );
    let rec = fs::read(dir.join("nodeid")).unwrap();
    assert_eq!(
        encoding::parse_record(&rec),
        Ok((KeyKind::Ed25519Pub, *out.new_node_id.as_bytes()))
    );
    let ks =
        Keyset::from_bytes(&fs::read(dir.join("keyset")).unwrap().try_into().unwrap()).unwrap();
    assert_eq!(ks.node_id(), out.new_node_id);
    assert_eq!(
        fs::read(dir.join("descriptor")).unwrap(),
        BackupDescriptor::V1_12_WORDS.to_bytes()
    );

    // fresh NodeId => fresh counter: genesis announce is 1 (the floor+1024 rule does not apply).
    let mut cs = CounterStore::open(&dir).unwrap();
    assert_eq!(cs.announce_last(), 0);
    assert_eq!(cs.next_announce_counter().unwrap(), 1);

    // steady state: migrating again is "nothing to migrate", never a re-key.
    assert!(!id111::migrate::is_legacy_dir(&dir));
    assert!(matches!(
        id111::migrate::migrate(&dir),
        Err(id111::migrate::MigrationError::NotLegacyDir)
    ));
}

#[test]
fn d9_migration_fail_closed_negatives() {
    use id111::migrate::MigrationError as E;
    let v = load("d9-migration.json");
    for n in v.get("negatives").arr() {
        let name = n.get("name").str();
        let files: Vec<(String, String)> = n
            .get("files")
            .arr()
            .iter()
            .map(|f| {
                (
                    f.get("name").str().to_string(),
                    f.get("body").str().to_string(),
                )
            })
            .collect();
        let t = TempDir::new("d9neg");
        let dir = t.identity();
        write_legacy(&dir, &files);
        let tok = match id111::migrate::migrate(&dir) {
            Ok(_) => "ok",
            Err(E::NotLegacyDir) => "NotLegacyDir",
            Err(E::CorruptLegacy(_)) => "CorruptLegacy",
            Err(E::MixedState(_)) => "MixedState",
            Err(E::Mnemonic(_)) => "Mnemonic",
            Err(E::Keyset(_)) => "Keyset",
            Err(E::Io(_)) => "Io",
        };
        assert_eq!(tok, n.get("expect").str(), "{name}");
        // nothing moved, nothing re-keyed, nothing quarantined.
        for (fname, body) in &files {
            assert_eq!(
                &fs::read_to_string(dir.join(fname)).unwrap(),
                body,
                "{name}: {fname} untouched"
            );
        }
        assert!(
            !t.path().join("identity.quarantine-d9").exists(),
            "{name}: no quarantine on refusal"
        );
    }
}

// ===========================================================================
// 14. End-to-end wiring smoke: every module in one lifecycle
// ===========================================================================

#[test]
fn lifecycle_end_to_end_wiring() {
    // keygen -> backup ceremony -> restore elsewhere -> announce -> pin ->
    // sealed traffic -> counters -> display. A wiring check across modules.
    let (mnemonic, original) = Keyset::generate(WordCount::Twelve).unwrap();

    // section 8.3: ceremony artifacts and verify-on-write-down.
    let art = BackupArtifacts::new(&mnemonic);
    let positions =
        backup::sample_positions(WordCount::Twelve, backup::DEFAULT_SAMPLE_COUNT).unwrap();
    let phrase = mnemonic.phrase();
    let words: Vec<&str> = phrase.split(' ').collect();
    let typed: Vec<&str> = positions.iter().map(|&p| words[p]).collect();
    assert!(backup::check_words(&mnemonic, &positions, &typed).unwrap());
    assert!(backup::qr_payload_matches(&mnemonic, art.qr_payload()));

    // section 8.4: disaster recovery from the QR on a fresh device (old instance retired).
    let t = TempDir::new("lifecycle");
    let out = backup::restore_from_qr(
        &t.identity(),
        art.qr_payload(),
        &opts(CounterFloor::NeverDeployed),
    )
    .unwrap();
    assert_eq!(
        out.node_id,
        original.node_id(),
        "identical NodeId after restore"
    );
    let restored = Keyset::from_bytes(
        &fs::read(t.identity().join("keyset"))
            .unwrap()
            .try_into()
            .unwrap(),
    )
    .unwrap();

    // section 7: the confirmation surface round-trips.
    let shown = out.display();
    assert_eq!(
        encoding::bech32_decode(&shown.bech32()),
        Ok(*original.node_id().as_bytes())
    );
    assert_eq!(
        encoding::hex_decode(&shown.hex()),
        Ok(*original.node_id().as_bytes())
    );

    // section 6.2: the restored node announces with the post-restore counter.
    let mut my_counters = CounterStore::open(&t.identity()).unwrap();
    let ann_counter = my_counters.next_announce_counter().unwrap();
    assert_eq!(ann_counter, 1025);
    let wire = Announce::create(&restored, ann_counter, b"hub")
        .unwrap()
        .encode();

    // a peer pins it (section 6.3).
    let (_pm, peer) = Keyset::generate(WordCount::Twelve).unwrap();
    let tp = TempDir::new("lifecycle-peer");
    let mut pins = PinStore::open(tp.path()).unwrap();
    assert_eq!(
        ann_tok(process_announce(&mut pins, &peer.node_id(), &wire)),
        "ok:pinned"
    );
    let pinned_enc = *pins.get(&restored.node_id()).unwrap().enc_pubkey();
    assert_eq!(pinned_enc, restored.x25519_public());

    // section 6.4/6.5: the peer seals traffic to the pinned key; the restored node opens it.
    let mut peer_counters = CounterStore::create(tp.path()).unwrap();
    let c = peer_counters
        .next_send_counter(&restored.node_id())
        .unwrap();
    let packet = seal::seal(
        &pinned_enc,
        &SealHeader::v1(c),
        &peer.node_id(),
        b"hello hub",
    )
    .unwrap();
    let opened = seal::open(&restored, &packet).unwrap();
    assert_eq!(opened.sender, peer.node_id());
    assert_eq!(opened.plaintext, b"hello hub");
    my_counters
        .on_receive(&opened.sender, opened.header.counter)
        .unwrap();
    assert!(
        my_counters
            .on_receive(&opened.sender, opened.header.counter)
            .is_err(),
        "replay rejected"
    );

    // the original (pre-restore) keyset opens it too: same identity.
    assert_eq!(
        seal::open(&original, &packet).unwrap().plaintext,
        b"hello hub"
    );
}
