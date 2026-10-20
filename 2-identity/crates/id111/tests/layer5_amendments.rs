//! Layer 5 amendments to Layer 2 (task L5-I1) — the `test-vectors/layer5/`
//! vectors as LIVE gates over the PUBLIC `id111` API.
//!
//! `os/5-messaging/README.md` §19 asks Layer 2 for four code amendments; this
//! suite freezes the three with observable wire/state behaviour:
//!
//! * **A2** (`sealed-payload.json`) — `seal::seal_payload` / `seal::open_payload`:
//!   the §6.4 construction with a caller-owned inner (no 32 B sender NodeId);
//!   the closure receives the packet's `epk`, `open_payload` returns it.
//! * **A6** (`announce-a6.json`) — a same-key announce at a counter ≤ the
//!   stored one is `PinOutcome::Unchanged` (no state change); a key conflict
//!   stays a hard rejection at ANY counter.
//! * **A11** (`observe-counter.json`) — `CounterStore::observe_counter`: the
//!   §6.5 window's verdict, never a rejection.
//!
//! (A3 — `next_announce_counter` as the `msg_seq` epoch source — is a
//! documentation + unit-test amendment in `counters.rs`; it adds no wire.
//! So is A12 — `observe_counter`'s verdict after an unclean reboot, from the
//! Layer 5 integrated review: the `counters::tests::a12_*` unit tests, no
//! wire, no vector.)
//!
//! The files live in a SUBDIRECTORY on purpose: the top-level vector set is
//! the Cat-1 Monocypher harness contract (D20: untouched), and both its
//! inventory (`monocypher-harness/run.sh`) and `conformance.rs`'s are
//! top-level only. This suite keeps its own inventory of `layer5/`.
//!
//! Like `conformance.rs`, it carries a small strict JSON reader (no
//! `serde_json` in the frozen dep set) and checks the A2 known answers
//! against an in-test model built straight from the primitive crates, so the
//! frozen bytes are tied to the SPEC recipe and not only to the code.
//! `test-vectors/crosscheck.py` (`check_layer5_*`) is the independent
//! Python leg.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use id111::announce::{process_announce, Announce, AnnounceError};
use id111::counters::{CounterError, CounterStore, Freshness};
use id111::keyset::{EcdhError, Keyset, NodeId};
use id111::seal::{self, OpenError, SealError, SealHeader};
use id111::store::{PinOutcome, PinStore};

/// Every vector file under `test-vectors/layer5/`, with the gate it freezes.
const LAYER5_FILES: &[(&str, &str)] = &[
    (
        "sealed-payload.json",
        "A2: seal_payload/open_payload caller-owned inner",
    ),
    (
        "announce-a6.json",
        "A6: same-key stale announce is Unchanged; conflict rejected",
    ),
    (
        "observe-counter.json",
        "A11: observe_counter fresh/stale verdict",
    ),
];

/// The freeze date of the Layer 5 amendment vectors.
const FROZEN: &str = "2026-10-14";

// ===========================================================================
// 1. Minimal strict JSON reader (the subset the vectors use)
// ===========================================================================

#[derive(Debug, Clone)]
enum V {
    Null,
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

    fn eat(&mut self, c: u8) {
        assert_eq!(
            self.peek(),
            c,
            "json: expected {:?} at {}",
            c as char,
            self.i
        );
        self.i += 1;
    }

    fn value(&mut self) -> V {
        self.ws();
        match self.peek() {
            b'{' => {
                self.i += 1;
                let mut m: Vec<(String, V)> = Vec::new();
                self.ws();
                if self.peek() == b'}' {
                    self.i += 1;
                    return V::Obj(m);
                }
                loop {
                    self.ws();
                    let k = self.string();
                    assert!(m.iter().all(|(x, _)| *x != k), "json: duplicate key {k:?}");
                    self.ws();
                    self.eat(b':');
                    let v = self.value();
                    m.push((k, v));
                    self.ws();
                    match self.peek() {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return V::Obj(m);
                        }
                        c => panic!("json: unexpected {:?} at {}", c as char, self.i),
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.peek() == b']' {
                    self.i += 1;
                    return V::Arr(a);
                }
                loop {
                    a.push(self.value());
                    self.ws();
                    match self.peek() {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return V::Arr(a);
                        }
                        c => panic!("json: unexpected {:?} at {}", c as char, self.i),
                    }
                }
            }
            b'"' => V::Str(self.string()),
            b'n' => {
                assert!(self.b[self.i..].starts_with(b"null"), "json: bad literal");
                self.i += 4;
                V::Null
            }
            b'0'..=b'9' => {
                let start = self.i;
                while self.peek().is_ascii_digit() {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.b[start..self.i]).unwrap();
                V::Num(t.parse().expect("json: integer out of u64 range"))
            }
            c => panic!("json: unexpected {:?} at {}", c as char, self.i),
        }
    }

    /// Strings: the vectors are ASCII-only (generated with `ensure_ascii`);
    /// the only escapes they use are `\"` and `\\`, plus `\uXXXX` (BMP).
    fn string(&mut self) -> String {
        self.eat(b'"');
        let mut out = String::new();
        loop {
            assert!(self.i < self.b.len(), "json: unterminated string");
            let c = self.b[self.i];
            self.i += 1;
            match c {
                b'"' => return out,
                b'\\' => {
                    let e = self.b[self.i];
                    self.i += 1;
                    match e {
                        b'"' | b'\\' | b'/' => out.push(e as char),
                        b'n' => out.push('\n'),
                        b'u' => {
                            let h = std::str::from_utf8(&self.b[self.i..self.i + 4]).unwrap();
                            let cp = u32::from_str_radix(h, 16).expect("json: bad \\u");
                            self.i += 4;
                            out.push(char::from_u32(cp).expect("json: surrogates unsupported"));
                        }
                        e => panic!("json: unsupported escape \\{}", e as char),
                    }
                }
                c => {
                    assert!(c.is_ascii(), "json: vectors are ASCII-only");
                    out.push(c as char);
                }
            }
        }
    }
}

impl V {
    fn get(&self, key: &str) -> &V {
        match self {
            V::Obj(m) => m
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v)
                .unwrap_or_else(|| panic!("vector field {key:?} missing")),
            other => panic!("expected object reading {key:?}, got {other:?}"),
        }
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

    fn is_null(&self) -> bool {
        matches!(self, V::Null)
    }
}

fn layer5_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors/layer5")
}

fn load(name: &str) -> V {
    let path = layer5_dir().join(name);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read frozen vector file {}: {e}", path.display()));
    let mut p = Parser {
        b: text.as_bytes(),
        i: 0,
    };
    let v = p.value();
    p.ws();
    assert_eq!(p.i, p.b.len(), "json: trailing garbage in {name}");
    v
}

// ===========================================================================
// 2. Helpers
// ===========================================================================

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len() & 1 == 0, "odd-length hex");
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex digit"))
        .collect()
}

fn keyset_from(seed: &[u8; 32], scalar: &[u8; 32]) -> Keyset {
    let mut b = [0u8; 64];
    b[..32].copy_from_slice(seed);
    b[32..].copy_from_slice(scalar);
    Keyset::from_bytes(&b).expect("test keyset passes the weak-key self-test")
}

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
            "id111-layer5-{tag}-{}-{seq}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `open_payload` outcome tokens (grammar in `sealed-payload.json`).
fn payload_tok(r: &Result<(SealHeader, [u8; 32], Vec<u8>), OpenError>) -> String {
    match r {
        Ok(_) => "ok".into(),
        Err(OpenError::Truncated) => "err:truncated".into(),
        Err(OpenError::UnsupportedCryptoMode(m)) => format!("err:mode:{m}"),
        Err(OpenError::Ecdh(e)) => format!("err:ecdh:{e:?}"),
        Err(OpenError::AuthFailed) => "err:auth".into(),
        Err(OpenError::PlaintextTooShort) => "err:short-plaintext".into(),
    }
}

/// Announce outcome tokens (grammar in `announce-a6.json`).
fn ann_tok(r: Result<PinOutcome, AnnounceError>) -> String {
    match r {
        Ok(PinOutcome::Pinned { evicted: None }) => "ok:pinned".into(),
        Ok(PinOutcome::Pinned { evicted: Some(_) }) => "ok:pinned+evicted".into(),
        Ok(PinOutcome::CounterUpdated { previous }) => format!("ok:updated:{previous}"),
        Ok(PinOutcome::Unchanged { last_seen }) => format!("ok:unchanged:{last_seen}"),
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

// ===========================================================================
// 3. Inventory
// ===========================================================================

#[test]
fn layer5_files_are_declared_self_describing_and_frozen() {
    let mut on_disk: BTreeSet<String> = fs::read_dir(layer5_dir())
        .expect("test-vectors/layer5/ exists")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    let readme = fs::read_to_string(layer5_dir().join("../README.md")).expect("README.md");
    for (file, gate) in LAYER5_FILES {
        assert!(on_disk.remove(*file), "missing {file} ({gate})");
        let v = load(file);
        assert!(v.get("spec").str().contains("section 19"), "{file}: spec");
        assert_eq!(v.get("frozen").str(), FROZEN, "{file}: frozen date");
        assert!(!v.get("description").str().is_empty(), "{file}");
        assert!(!v.get("provenance").str().is_empty(), "{file}");
        assert!(
            readme.contains(&format!("layer5/{file}")),
            "README.md does not document layer5/{file}"
        );
    }
    assert!(
        on_disk.is_empty(),
        "undeclared layer5 vector files (add them to LAYER5_FILES): {on_disk:?}"
    );
}

// ===========================================================================
// 4. A2 — sealed packet with a caller-owned inner
// ===========================================================================

struct Model {
    packet: Vec<u8>,
    epk: [u8; 32],
    shared: [u8; 32],
    key: [u8; 32],
    nonce: [u8; 24],
}

/// Independent model of the §6.4 recipe from the primitive crates (NOT
/// id111), with the A2 inner: the caller's bytes, nothing prepended.
fn model(esk: &[u8; 32], rpk: &[u8; 32], header: &[u8], inner: &[u8]) -> Model {
    use blake2::digest::consts::{U24, U32};
    use blake2::{Blake2b, Digest};
    use chacha20poly1305::aead::{Aead, Payload};
    use chacha20poly1305::{KeyInit, XChaCha20Poly1305};
    let secret = x25519_dalek::StaticSecret::from(*esk);
    let epk = x25519_dalek::PublicKey::from(&secret).to_bytes();
    let shared = secret
        .diffie_hellman(&x25519_dalek::PublicKey::from(*rpk))
        .to_bytes();
    assert_ne!(shared, [0u8; 32]);
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
    Model {
        packet,
        epk,
        shared,
        key,
        nonce,
    }
}

fn recipient(v: &V) -> Keyset {
    let r = v.get("recipient");
    let ks = keyset_from(&r.get("ed25519_seed").h32(), &r.get("x25519_scalar").h32());
    assert_eq!(ks.x25519_public(), r.get("x25519_public").h32());
    assert_eq!(ks.node_id().as_bytes(), &r.get("node_id").h32());
    ks
}

#[test]
fn a2_known_answers_match_the_spec_recipe_and_the_crate() {
    let v = load("sealed-payload.json");
    let bob = recipient(&v);
    let kats = v.get("kats").arr();
    assert_eq!(kats.len(), 3);
    let mut sizes = BTreeSet::new();
    for k in kats {
        let name = k.get("name").str();
        let hd = k.get("header");
        let header = SealHeader {
            version: hd.get("version").num() as u8,
            crypto_mode: hd.get("crypto_mode").num() as u8,
            counter: hd.get("counter").dec(),
            extra: hd.get("extra").hex(),
        };
        let mut hb = vec![header.version, header.crypto_mode];
        hb.extend_from_slice(&header.counter.to_le_bytes());
        hb.push(header.extra.len() as u8);
        hb.extend_from_slice(&header.extra);
        assert_eq!(hb, k.get("header_bytes").hex(), "{name}: header bytes");
        let inner = k.get("inner").hex();

        // (1) the spec recipe, recomputed from the primitives.
        let m = model(&k.get("esk").h32(), &k.get("rpk").h32(), &hb, &inner);
        assert_eq!(m.epk, k.get("epk").h32(), "{name}: epk");
        assert_eq!(m.shared, k.get("shared").h32(), "{name}: shared");
        assert_eq!(m.key, k.get("key").h32(), "{name}: key");
        assert_eq!(m.nonce.to_vec(), k.get("nonce").hex(), "{name}: nonce");
        let packet = k.get("packet").hex();
        assert_eq!(m.packet, packet, "{name}: recipe vs frozen packet");
        assert_eq!(
            &packet[hb.len() + 32..],
            &k.get("ciphertext_and_tag").hex()[..],
            "{name}"
        );
        // A2: exactly header + epk + inner + tag — no 32 B NodeId inside.
        assert_eq!(packet.len(), hb.len() + 32 + inner.len() + 16, "{name}");
        sizes.insert(packet.len());

        // (2) the crate opens the frozen packet and returns the wire epk.
        let (h, epk, got) = seal::open_payload(&bob, &packet).unwrap();
        assert_eq!(h, header, "{name}");
        assert_eq!(epk, m.epk, "{name}: open_payload returns the wire epk");
        assert_eq!(got, inner, "{name}");
    }
    // The bare-tag KAT sits exactly on the MIN_PACKET_LEN boundary.
    assert!(sizes.contains(&seal::MIN_PACKET_LEN));
}

#[test]
fn a2_fresh_seal_payload_round_trips_and_hands_the_closure_the_epk() {
    let v = load("sealed-payload.json");
    let bob = recipient(&v);
    for k in v.get("kats").arr() {
        let hd = k.get("header");
        let header = SealHeader {
            version: 1,
            crypto_mode: 1,
            counter: hd.get("counter").dec(),
            extra: hd.get("extra").hex(),
        };
        let inner = k.get("inner").hex();
        let mut seen = None;
        let p1 = seal::seal_payload(&bob.x25519_public(), &header, |epk| {
            seen = Some(*epk);
            inner.clone()
        })
        .unwrap();
        let p2 = seal::seal_payload(&bob.x25519_public(), &header, |_| inner.clone()).unwrap();
        assert_ne!(p1, p2, "fresh ephemeral per packet");
        let hl = seal::HEADER_FIXED_LEN + header.extra.len();
        assert_eq!(Some(<[u8; 32]>::try_from(&p1[hl..hl + 32]).unwrap()), seen);
        let (h, epk, got) = seal::open_payload(&bob, &p1).unwrap();
        assert_eq!((h, Some(epk), got), (header, seen, inner));
    }
}

#[test]
fn a2_negatives_fail_closed() {
    let v = load("sealed-payload.json");
    let bob = recipient(&v);
    let kat0 = v.get("kats").arr()[0].get("packet").hex();

    let wr = v.get("wrong_recipient");
    let other = Keyset::derive_from_mnemonic(wr.get("mnemonic").str()).unwrap();
    assert_eq!(
        payload_tok(&seal::open_payload(&other, &kat0)),
        wr.get("expect").str()
    );

    let negs = v.get("negatives").arr();
    assert!(negs.len() >= 20);
    let mut classes = BTreeSet::new();
    for e in negs {
        let tok = payload_tok(&seal::open_payload(&bob, &e.get("packet").hex()));
        assert_eq!(tok, e.get("expect").str(), "{}", e.get("name").str());
        let class: Vec<&str> = tok.splitn(3, ':').take(2).collect();
        classes.insert(class.join(":"));
    }
    for want in ["err:auth", "err:mode", "err:truncated", "err:ecdh"] {
        assert!(
            classes.contains(want),
            "A2 corpus lacks {want}: {classes:?}"
        );
    }
    assert!(
        !classes.contains("err:short-plaintext"),
        "open_payload has no inner minimum"
    );

    // seal-side refusals happen before the inner closure ever runs.
    for e in v.get("seal_negatives").arr() {
        let hdr = SealHeader {
            version: 1,
            crypto_mode: e.get("crypto_mode").num() as u8,
            counter: 1,
            extra: vec![0; e.get("extra_len").num() as usize],
        };
        let mut invoked = false;
        let r = seal::seal_payload(&e.get("recipient").h32(), &hdr, |_| {
            invoked = true;
            b"x".to_vec()
        });
        let tok = match r {
            Ok(_) => "ok".to_string(),
            Err(SealError::UnsupportedCryptoMode(m)) => format!("UnsupportedCryptoMode:{m}"),
            Err(SealError::HeaderTooLarge(n)) => format!("HeaderTooLarge:{n}"),
            Err(SealError::Ecdh(e)) => format!("Ecdh:{e:?}"),
            Err(other) => format!("{other:?}"),
        };
        assert_eq!(tok, e.get("expect").str(), "{}", e.get("name").str());
        assert!(!invoked, "{}: closure ran", e.get("name").str());
    }
    // Spot-check the one EcdhError variant the corpus names.
    assert!(matches!(
        seal::seal_payload(&[0u8; 32], &SealHeader::v1(1), |_| Vec::new()),
        Err(SealError::Ecdh(EcdhError::LowOrderPublicKey))
    ));
}

#[test]
fn a2_layout_ambiguity_is_frozen() {
    let v = load("sealed-payload.json");
    let bob = recipient(&v);
    let la = v.get("layout_ambiguity");
    let a = la.get("payload_open_of_32b_kat");
    let r = seal::open_payload(&bob, &a.get("packet").hex());
    assert_eq!(payload_tok(&r), a.get("expect").str());
    assert_eq!(r.unwrap().2, a.get("expect_inner").hex());
    let b = la.get("open_of_bare_tag_kat");
    let tok = match seal::open(&bob, &b.get("packet").hex()) {
        Ok(_) => "ok".to_string(),
        Err(OpenError::PlaintextTooShort) => "err:short-plaintext".to_string(),
        Err(e) => format!("{e:?}"),
    };
    assert_eq!(tok, b.get("expect").str());
}

// ===========================================================================
// 5. A6 — same-key stale announce is Unchanged
// ===========================================================================

#[test]
fn a6_scenarios_through_process_announce() {
    let v = load("announce-a6.json");
    let signers = v.get("signers");
    for who in ["a", "b"] {
        let s = signers.get(who);
        let k = Keyset::derive_from_mnemonic(s.get("mnemonic").str()).unwrap();
        assert_eq!(k.node_id().as_bytes(), &s.get("node_id").h32());
        assert_eq!(k.x25519_public(), s.get("enc_pubkey").h32());
    }
    let a_id = NodeId::from(signers.get("a").get("node_id").h32());
    let own = NodeId::from(v.get("own_node_id").h32());
    let (mut unchanged, mut conflicts) = (0, 0);
    for sc in v.get("scenarios").arr() {
        let name = sc.get("name").str();
        let t = TempDir::new("a6");
        let mut store = PinStore::open(t.path()).unwrap();
        for st in sc.get("steps").arr() {
            let step = st.get("step").str();
            let wire = st.get("wire").hex();
            let before = store.get(&a_id).copied();
            let file_before = fs::read(t.path().join("peers")).ok();
            let tok = ann_tok(process_announce(&mut store, &own, &wire));
            assert_eq!(tok, st.get("expect").str(), "{name}: {step}");
            // The pin after the step, as frozen.
            let after = st.get("after");
            match store.get(&a_id) {
                None => assert!(after.is_null(), "{name}: {step}: expected a pin"),
                Some(e) => {
                    assert_eq!(
                        e.enc_pubkey(),
                        &after.get("enc_pubkey").h32(),
                        "{name}: {step}"
                    );
                    assert_eq!(
                        e.highest_announce_counter(),
                        after.get("counter").dec(),
                        "{name}: {step}"
                    );
                }
            }
            if tok.starts_with("ok:unchanged") || tok.starts_with("err:") {
                // Unchanged and every rejection: no state change at all —
                // not in memory, not on disk.
                assert_eq!(store.get(&a_id).copied(), before, "{name}: {step}");
                assert_eq!(
                    fs::read(t.path().join("peers")).ok(),
                    file_before,
                    "{name}: {step}: persisted"
                );
            }
            unchanged += usize::from(tok.starts_with("ok:unchanged"));
            conflicts += usize::from(tok == "err:conflict");
        }
    }
    assert!(
        unchanged >= 5,
        "A6 corpus too thin: {unchanged} unchanged steps"
    );
    assert!(conflicts >= 3, "the conflict distinction must stay frozen");
    // The A6 conflict steps include a counter at or below the pin: the case
    // a careless A6 would have reported as Unchanged.
    let wire = |s: &V| Announce::parse(&s.get("wire").hex()).unwrap();
    let conflict_low = v
        .get("scenarios")
        .arr()
        .iter()
        .flat_map(|sc| sc.get("steps").arr())
        .filter(|s| s.get("expect").str() == "err:conflict")
        .any(|s| wire(s).counter() <= s.get("after").get("counter").dec());
    assert!(conflict_low);
}

// ===========================================================================
// 6. A11 — observe_counter verdicts
// ===========================================================================

#[test]
fn a11_observe_counter_sequences_with_on_receive_parity() {
    let v = load("observe-counter.json");
    assert_eq!(v.get("window_size").num(), 32);
    let peer = NodeId::from([0x5au8; 32]);
    let mut stale_seen = 0;
    for sq in v.get("sequences").arr() {
        let name = sq.get("name").str();
        let (ta, tb) = (TempDir::new("a11-obs"), TempDir::new("a11-rcv"));
        let mut obs = CounterStore::create(ta.path()).unwrap();
        let mut rcv = CounterStore::create(tb.path()).unwrap();
        for (i, st) in sq.get("steps").arr().iter().enumerate() {
            let c = st.get("counter").dec();
            let verdict = obs.observe_counter(&peer, c).unwrap();
            let want = st.get("expect").str();
            assert_eq!(
                match verdict {
                    Freshness::Fresh => "fresh",
                    Freshness::Stale => "stale",
                },
                want,
                "{name}: step {i} (counter {c})"
            );
            let w = obs.peer_traffic(&peer).unwrap().receive_window();
            assert_eq!(w.high(), st.get("high").dec(), "{name}: step {i}");
            assert_eq!(
                u64::from(w.bitmap()),
                st.get("bitmap").num(),
                "{name}: step {i}"
            );
            // Parity: on_receive accepts exactly the fresh ones and rejects
            // exactly the stale ones, leaving an identical window.
            let r = rcv.on_receive(&peer, c);
            match verdict {
                Freshness::Fresh => assert!(r.is_ok(), "{name}: step {i}"),
                Freshness::Stale => {
                    stale_seen += 1;
                    assert!(
                        matches!(r, Err(CounterError::Window(_))),
                        "{name}: step {i}"
                    )
                }
            }
            assert_eq!(rcv.peer_traffic(&peer).unwrap().receive_window(), w);
        }
    }
    assert!(stale_seen >= 8);
}
