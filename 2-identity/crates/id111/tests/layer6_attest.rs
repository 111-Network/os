//! Layer 6 amendment A10 (`os/20-identity/README.md` §13.1; `os/60-transport/README.md`
//! §14; D26 #3, 2026-10-19) — the `test-vectors/layer6/` vectors as LIVE gates over the
//! PUBLIC `id111` API.
//!
//! A10 assigns two `transport_id` registry values for bridge identities:
//!
//! * `0x02` **meshtastic** — `transport_key` = the board's X25519 public key;
//!   `transport_sig` = 64 zero bytes on 2.7.x boards ⇒ **one-sided** (`node_sig` only).
//! * `0x03` **reticulum** — `transport_key` = SHA-256 of the 64 B public key
//!   (X25519 ‖ Ed25519); `transport_sig` = Ed25519 by the identity's signing key over
//!   `"111/attest-transport/v1" ‖ node_pubkey` ⇒ both signatures.
//!
//! This suite asserts what id111 owns: the registry bytes, parsing, the node leg, the
//! byte-exact wire (re-issued deterministically from the frozen inputs), and the
//! SHA-256 binding helper. The transport-leg plug (`BridgeVerifier`) lives in
//! `node111` (`src/daemon/bridge/attest.rs`) and replays the same file; here the
//! frozen `plug` tokens are only checked for internal consistency against an
//! independent recomputation (the hash, and the strict predicate on the Ed25519 half).
//!
//! The files live in a SUBDIRECTORY on purpose: the top-level vector set is the Cat-1
//! Monocypher harness contract (D20: untouched); `monocypher-harness/run.sh` and
//! `conformance.rs` inventory the top level only. This suite keeps its own inventory.
//!
//! Regenerate (never as a side effect of a code change — frozen means frozen):
//! `ID111_LAYER6_OUT=<path> cargo test -p id111 --test layer6_attest -- --ignored`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signer, SigningKey};
use id111::attest::{
    reticulum_transport_key, AttestError, Attestation, MeshEpoch, NodeLeg, TransportId,
    MESHTASTIC_UNSIGNED, NODE_TAG, TRANSPORT_TAG,
};
use id111::keyset::{verify_strict, Keyset, NodeId};
use sha2::{Digest, Sha256};

/// Every vector file under `test-vectors/layer6/`.
const LAYER6_FILES: &[(&str, &str)] = &[(
    "attest-transport-ids.json",
    "A10: transport_id 0x02 meshtastic (one-sided) and 0x03 reticulum (hash-bound, both legs)",
)];

/// The freeze date of the Layer 6 amendment vectors.
const FROZEN: &str = "2026-10-19";

// ===========================================================================
// 1. Minimal strict JSON reader (copied from layer5_amendments.rs; no
//    serde_json in the frozen dep set)
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
}

fn layer6_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors/layer6")
}

fn parse(text: &str, name: &str) -> V {
    let mut p = Parser {
        b: text.as_bytes(),
        i: 0,
    };
    let v = p.value();
    p.ws();
    assert_eq!(p.i, p.b.len(), "json: trailing garbage in {name}");
    v
}

fn load(name: &str) -> V {
    let path = layer6_dir().join(name);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read frozen vector file {}: {e}", path.display()));
    parse(&text, name)
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

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn h64(v: &V) -> [u8; 64] {
    v.hex().try_into().expect("64-byte hex")
}

fn node_tok(r: Result<(), AttestError>) -> String {
    match r {
        Ok(()) => "ok".into(),
        Err(AttestError::InvalidNodeSignature(e)) => format!("err:node-sig:{e:?}"),
        Err(e) => format!("err:other:{e:?}"),
    }
}

/// The canonical project mnemonic (signer `a` of `attestation.json`) and signer `b`.
const MNEMONIC_A: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const MNEMONIC_B: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

// ===========================================================================
// 3. Inventory
// ===========================================================================

#[test]
fn layer6_files_are_declared_self_describing_and_frozen() {
    let mut on_disk: BTreeSet<String> = fs::read_dir(layer6_dir())
        .expect("test-vectors/layer6/ exists")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    let readme = fs::read_to_string(layer6_dir().join("README.md")).expect("layer6/README.md");
    for (file, gate) in LAYER6_FILES {
        assert!(on_disk.remove(*file), "missing {file} ({gate})");
        let v = load(file);
        assert!(v.get("spec").str().contains("A10"), "{file}: spec");
        assert_eq!(v.get("frozen").str(), FROZEN, "{file}: frozen date");
        assert!(!v.get("description").str().is_empty(), "{file}");
        assert!(!v.get("provenance").str().is_empty(), "{file}");
        assert!(
            readme.contains(*file),
            "layer6/README.md does not document {file}"
        );
    }
    assert!(
        on_disk.is_empty(),
        "undeclared layer6 vector files (add them to LAYER6_FILES): {on_disk:?}"
    );
}

// ===========================================================================
// 4. A10 — registry values, the binding inputs, every record
// ===========================================================================

#[test]
fn a10_registry_bytes_and_display() {
    let v = load("attest-transport-ids.json");
    let reg = v.get("registry").arr();
    assert_eq!(reg.len(), 3);
    for r in reg {
        let b = u8::try_from(r.get("transport_id").num()).expect("u8");
        let t = TransportId::from_byte(b).expect("assigned");
        assert_eq!(t.to_byte(), b);
        assert_eq!(t.to_string(), r.get("display").str());
    }
    assert_eq!(TransportId::Meshtastic.to_byte(), 0x02);
    assert_eq!(TransportId::Reticulum.to_byte(), 0x03);
    for b in [0x00u8, 0x04, 0xff] {
        assert_eq!(
            TransportId::from_byte(b),
            None,
            "0x{b:02x} stays fail-closed"
        );
    }
    assert_eq!(v.get("tags").get("node").str().as_bytes(), NODE_TAG);
    assert_eq!(
        v.get("tags").get("transport").str().as_bytes(),
        TRANSPORT_TAG
    );
}

#[test]
fn a10_reticulum_binding_inputs() {
    let v = load("attest-transport-ids.json");
    let r = v.get("reticulum");
    let seed = r.get("ed25519_seed").h32();
    let ed_pub = r.get("ed25519_pub").h32();
    let x_pub = r.get("x25519_pub").h32();
    let pk64 = h64(r.get("pubkey64"));
    // The Ed25519 half is the seed's public key (RFC 8032 TEST 1).
    assert_eq!(
        SigningKey::from_bytes(&seed).verifying_key().to_bytes(),
        ed_pub
    );
    // pubkey64 = X25519 ‖ Ed25519, as Reticulum carries it.
    assert_eq!(&pk64[..32], &x_pub);
    assert_eq!(&pk64[32..], &ed_pub);
    // transport_key = SHA-256(pubkey64): the crate helper and an independent digest.
    let want = r.get("transport_key").h32();
    assert_eq!(reticulum_transport_key(&pk64), want);
    let independent: [u8; 32] = Sha256::digest(pk64).into();
    assert_eq!(independent, want);
    // The signed input and the frozen transport_sig under the strict predicate.
    let node = Keyset::derive_from_mnemonic(MNEMONIC_A).unwrap().node_id();
    assert_eq!(to_hex(node.as_bytes()), v.get("node").get("node_id").str());
    let mut input = TRANSPORT_TAG.to_vec();
    input.extend_from_slice(node.as_bytes());
    assert_eq!(input, r.get("signed_input_transport").hex());
    let sig = h64(r.get("transport_sig"));
    verify_strict(&NodeId::from(ed_pub), &input, &sig).expect("frozen transport_sig");
}

#[test]
fn a10_records_parse_and_node_leg_matches() {
    let v = load("attest-transport-ids.json");
    let r = v.get("reticulum");
    let pk64 = h64(r.get("pubkey64"));
    let ed = NodeId::from(r.get("ed25519_pub").h32());
    let ka = Keyset::derive_from_mnemonic(MNEMONIC_A).unwrap();
    let expiry = MeshEpoch::from_u64(v.get("expiry_epoch").dec());
    let cases = v.get("cases").arr();
    assert!(cases.len() >= 10, "corpus unexpectedly small");
    let mut plug_tokens = BTreeSet::new();
    for c in cases {
        let name = c.get("name").str();
        let wire = c.get("wire").hex();
        assert_eq!(wire.len(), 202, "{name}");
        let att = Attestation::from_bytes(&wire).unwrap_or_else(|e| panic!("{name}: {e}"));
        // Byte values: the registry byte on the wire and in the parsed record.
        let tid = u8::try_from(c.get("transport_id").num()).unwrap();
        assert_eq!(wire[33], tid, "{name}");
        assert_eq!(att.transport_id().to_byte(), tid, "{name}");
        assert_eq!(att.to_bytes().to_vec(), wire, "{name}: round-trip");
        // The node leg (built into id111).
        let tok = node_tok(att.verify_node_leg());
        assert_eq!(tok, c.get("node_leg").str(), "{name}");
        // Deterministic re-issue: every record with a valid node leg is exactly
        // NodeLeg::issue(signer a, tid, transport_key, expiry) + its transport_sig.
        if tok == "ok" {
            assert_eq!(att.node_id(), ka.node_id(), "{name}");
            assert_eq!(att.expiry_epoch(), expiry, "{name}");
            let again = NodeLeg::issue(&ka, att.transport_id(), *att.transport_key(), expiry)
                .complete(*att.transport_sig());
            assert_eq!(again.to_bytes().to_vec(), wire, "{name}: re-issue");
        }
        // Consistency of the frozen plug token (the plug itself is node111's).
        let plug = c.get("plug").str();
        plug_tokens.insert(plug.split(':').take(2).collect::<Vec<_>>().join(":"));
        let unsigned = *att.transport_sig() == MESHTASTIC_UNSIGNED;
        match plug {
            "ok:one-sided" => {
                assert_eq!(att.transport_id(), TransportId::Meshtastic, "{name}");
                assert!(unsigned, "{name}");
            }
            "ok:both" => {
                assert_eq!(att.transport_id(), TransportId::Reticulum, "{name}");
                assert_eq!(
                    *att.transport_key(),
                    reticulum_transport_key(&pk64),
                    "{name}"
                );
                let mut input = TRANSPORT_TAG.to_vec();
                input.extend_from_slice(att.node_id().as_bytes());
                assert!(
                    verify_strict(&ed, &input, att.transport_sig()).is_ok(),
                    "{name}"
                );
            }
            "err:meshtastic-signed" => {
                assert_eq!(att.transport_id(), TransportId::Meshtastic, "{name}");
                assert!(!unsigned, "{name}");
            }
            "err:unknown-key" => {
                assert_eq!(att.transport_id(), TransportId::Reticulum, "{name}");
                assert_ne!(
                    *att.transport_key(),
                    reticulum_transport_key(&pk64),
                    "{name}"
                );
            }
            "err:transport-sig" => {
                assert_eq!(att.transport_id(), TransportId::Reticulum, "{name}");
                assert_eq!(
                    *att.transport_key(),
                    reticulum_transport_key(&pk64),
                    "{name}"
                );
                let mut input = TRANSPORT_TAG.to_vec();
                input.extend_from_slice(att.node_id().as_bytes());
                assert!(
                    verify_strict(&ed, &input, att.transport_sig()).is_err(),
                    "{name}"
                );
            }
            t if t.starts_with("err:node-sig:") => assert_eq!(t, tok, "{name}"),
            t => panic!("{name}: unknown plug token {t}"),
        }
    }
    for want in [
        "ok:both",
        "ok:one-sided",
        "err:unknown-key",
        "err:transport-sig",
        "err:meshtastic-signed",
        "err:node-sig",
    ] {
        assert!(
            plug_tokens.contains(want),
            "corpus lacks {want}: {plug_tokens:?}"
        );
    }
}

// ===========================================================================
// 5. Generator (kept behind #[ignore]; the JSON is frozen)
// ===========================================================================

/// RFC 8032 §7.1 TEST 1 secret key (the Reticulum identity's Ed25519 half).
const RET_ED_SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
/// RFC 7748 §6.1 Alice's public key (the Reticulum identity's X25519 half; any fixed 32 B).
const RET_X_PUB: &str = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a";
/// RFC 7748 §6.1 Bob's public key (the Meshtastic board's X25519 key).
const MESH_X_PUB: &str = "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f";
/// RFC 8032 §7.1 TEST 2 secret key (a stranger that signs a 0x02 record).
const STRANGER_SEED: &str = "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb";
const EXPIRY: u64 = 4096;

#[test]
#[ignore = "generator: freezes test-vectors/layer6/attest-transport-ids.json"]
fn generate_layer6_vectors() {
    let out = std::env::var("ID111_LAYER6_OUT").expect("set ID111_LAYER6_OUT=<path>");
    let ka = Keyset::derive_from_mnemonic(MNEMONIC_A).unwrap();
    let kb = Keyset::derive_from_mnemonic(MNEMONIC_B).unwrap();
    let a = ka.node_id();
    let b = kb.node_id();
    let seed: [u8; 32] = unhex(RET_ED_SEED).try_into().unwrap();
    let sk = SigningKey::from_bytes(&seed);
    let ed_pub = sk.verifying_key().to_bytes();
    let x_pub: [u8; 32] = unhex(RET_X_PUB).try_into().unwrap();
    let mut pk64 = [0u8; 64];
    pk64[..32].copy_from_slice(&x_pub);
    pk64[32..].copy_from_slice(&ed_pub);
    let tkey = reticulum_transport_key(&pk64);
    let mesh_key: [u8; 32] = unhex(MESH_X_PUB).try_into().unwrap();
    let stranger = SigningKey::from_bytes(&unhex(STRANGER_SEED).try_into().unwrap());
    let expiry = MeshEpoch::from_u64(EXPIRY);
    let input_for = |n: &NodeId| {
        let mut i = TRANSPORT_TAG.to_vec();
        i.extend_from_slice(n.as_bytes());
        i
    };
    let good_sig = sk.sign(&input_for(&a)).to_bytes();
    let issue = |tid, key: [u8; 32], sig: [u8; 64]| {
        NodeLeg::issue(&ka, tid, key, expiry)
            .complete(sig)
            .to_bytes()
    };

    // (name, rationale, wire, node_leg, plug)
    let mut cases: Vec<(&str, &str, Vec<u8>, &str, &str)> = Vec::new();
    cases.push((
        "reticulum both legs",
        "0x03: transport_key = SHA-256(X25519 || Ed25519); transport_sig = Ed25519 by the identity over tag || node_pubkey",
        issue(TransportId::Reticulum, tkey, good_sig).to_vec(),
        "ok",
        "ok:both",
    ));
    cases.push((
        "meshtastic one-sided",
        "0x02: transport_key = the board's X25519 key; transport_sig = 64 zero bytes (2.7.x cannot sign) => node_sig only",
        issue(TransportId::Meshtastic, mesh_key, MESHTASTIC_UNSIGNED).to_vec(),
        "ok",
        "ok:one-sided",
    ));
    let ed_only: [u8; 32] = Sha256::digest(ed_pub).into();
    cases.push((
        "reticulum wrong hash (Ed25519 half only)",
        "the hash covers the whole 64 B key; SHA-256 of the 32 B signing half binds nothing",
        issue(TransportId::Reticulum, ed_only, good_sig).to_vec(),
        "ok",
        "err:unknown-key",
    ));
    let mut swapped = [0u8; 64];
    swapped[..32].copy_from_slice(&ed_pub);
    swapped[32..].copy_from_slice(&x_pub);
    cases.push((
        "reticulum wrong hash (halves swapped)",
        "the 64 B key is X25519 || Ed25519 in that order",
        issue(
            TransportId::Reticulum,
            reticulum_transport_key(&swapped),
            good_sig,
        )
        .to_vec(),
        "ok",
        "err:unknown-key",
    ));
    cases.push((
        "reticulum signature over the wrong input (no tag)",
        "transport_sig covers '111/attest-transport/v1' || node_pubkey; an untagged signature is rejected",
        issue(TransportId::Reticulum, tkey, sk.sign(a.as_bytes()).to_bytes()).to_vec(),
        "ok",
        "err:transport-sig",
    ));
    let mut node_tagged = NODE_TAG.to_vec();
    node_tagged.extend_from_slice(a.as_bytes());
    cases.push((
        "reticulum domain-tag confusion (node tag)",
        "a signature under '111/attest-node/v1' never stands in for the transport tag",
        issue(
            TransportId::Reticulum,
            tkey,
            sk.sign(&node_tagged).to_bytes(),
        )
        .to_vec(),
        "ok",
        "err:transport-sig",
    ));
    cases.push((
        "reticulum cross-node transplant",
        "the identity vouched for node b; node a's record carrying that signature is rejected",
        issue(
            TransportId::Reticulum,
            tkey,
            sk.sign(&input_for(&b)).to_bytes(),
        )
        .to_vec(),
        "ok",
        "err:transport-sig",
    ));
    cases.push((
        "reticulum all-zero signature is never one-sided",
        "the one-sided rule exists for 0x02 only",
        issue(TransportId::Reticulum, tkey, MESHTASTIC_UNSIGNED).to_vec(),
        "ok",
        "err:transport-sig",
    ));
    cases.push((
        "meshtastic non-zero signature (no 2.8 verifier in v1)",
        "a 0x02 record with a signature is NOT accepted as one-sided; no XEdDSA verifier exists in v1",
        issue(TransportId::Meshtastic, mesh_key, stranger.sign(&input_for(&a)).to_bytes()).to_vec(),
        "ok",
        "err:meshtastic-signed",
    ));
    let mut one_bit = MESHTASTIC_UNSIGNED;
    one_bit[63] = 0x01;
    cases.push((
        "meshtastic one non-zero bit",
        "only exactly 64 zero bytes marks a one-sided record",
        issue(TransportId::Meshtastic, mesh_key, one_bit).to_vec(),
        "ok",
        "err:meshtastic-signed",
    ));
    let mut tampered = issue(TransportId::Meshtastic, mesh_key, MESHTASTIC_UNSIGNED);
    tampered[80] ^= 0x01;
    cases.push((
        "meshtastic one-sided with node_sig flipped",
        "a one-sided record rests on node_sig alone; a bad node_sig rejects it",
        tampered.to_vec(),
        "err:node-sig:InvalidSignature",
        "err:node-sig:InvalidSignature",
    ));
    let mut relabelled = issue(TransportId::Reticulum, tkey, good_sig);
    relabelled[33] = 0x02;
    cases.push((
        "reticulum record relabelled as 0x02",
        "node_sig covers transport_id; changing the registry byte breaks the node leg",
        relabelled.to_vec(),
        "err:node-sig:InvalidSignature",
        "err:node-sig:InvalidSignature",
    ));

    let mut j = String::new();
    j.push_str("{\n");
    j.push_str("  \"spec\": \"os/20-identity/README.md section 10 + section 13.1 A10; os/60-transport/README.md section 14 (D26 #3)\",\n");
    j.push_str(&format!("  \"frozen\": \"{FROZEN}\",\n"));
    j.push_str("  \"description\": \"A10 transport_id assignment for bridge identities. 0x02 meshtastic: transport_key = the board's X25519 public key, transport_sig = 64 zero bytes on 2.7.x boards (cannot sign) => ONE-SIDED (node_sig only; never attested-111). 0x03 reticulum: transport_key = SHA-256(pubkey64), pubkey64 = X25519(32) || Ed25519(32) as the helper reports it (ready.pubkey); transport_sig = Ed25519 by the identity's signing key (pubkey64[32..64]) over '111/attest-transport/v1' || node_pubkey => both legs. Wire and node_sig are the unchanged 202 B section 10 format. 'node_leg' is id111's built-in verify_node_leg token (ok | err:node-sig:<VerifyError>). 'plug' is the node111 BridgeVerifier outcome with exactly reticulum.pubkey64 known: ok:both | ok:one-sided | err:unknown-key (no known key hashes to transport_key) | err:transport-sig (strict Ed25519 fails) | err:meshtastic-signed (a 0x02 record with a non-zero signature; no 2.8 XEdDSA verifier in v1) | err:node-sig:<VerifyError> (node leg first). Epochs are decimal strings; expiry is stored, never evaluated without a mesh-epoch source.\",\n");
    j.push_str("  \"provenance\": \"generated by tests/layer6_attest.rs::generate_layer6_vectors (ignored) through the public API: NodeLeg::issue/complete with signer a; the Reticulum Ed25519 half is RFC 8032 TEST 1, its X25519 half and the board key are RFC 7748 section 6.1 Alice/Bob public keys, the stranger is RFC 8032 TEST 2; Ed25519 is deterministic, so every record re-issues byte-exact.\",\n");
    j.push_str(&format!(
        "  \"tags\": {{\n    \"node\": \"{}\",\n    \"transport\": \"{}\"\n  }},\n",
        std::str::from_utf8(NODE_TAG).unwrap(),
        std::str::from_utf8(TRANSPORT_TAG).unwrap()
    ));
    j.push_str("  \"registry\": [\n");
    let reg = [
        TransportId::NostrSecp256k1,
        TransportId::Meshtastic,
        TransportId::Reticulum,
    ];
    for (i, t) in reg.iter().enumerate() {
        j.push_str(&format!(
            "    {{\n      \"transport_id\": {},\n      \"display\": \"{}\"\n    }}{}\n",
            t.to_byte(),
            t,
            if i + 1 < reg.len() { "," } else { "" }
        ));
    }
    j.push_str("  ],\n");
    j.push_str(&format!(
        "  \"node\": {{\n    \"mnemonic\": \"{MNEMONIC_A}\",\n    \"node_id\": \"{}\"\n  }},\n",
        to_hex(a.as_bytes())
    ));
    j.push_str(&format!(
        "  \"other_node\": {{\n    \"mnemonic\": \"{MNEMONIC_B}\",\n    \"node_id\": \"{}\"\n  }},\n",
        to_hex(b.as_bytes())
    ));
    j.push_str(&format!("  \"expiry_epoch\": \"{EXPIRY}\",\n"));
    j.push_str(&format!(
        "  \"reticulum\": {{\n    \"ed25519_seed\": \"{RET_ED_SEED}\",\n    \"ed25519_pub\": \"{}\",\n    \"x25519_pub\": \"{RET_X_PUB}\",\n    \"pubkey64\": \"{}\",\n    \"transport_key\": \"{}\",\n    \"signed_input_transport\": \"{}\",\n    \"transport_sig\": \"{}\"\n  }},\n",
        to_hex(&ed_pub),
        to_hex(&pk64),
        to_hex(&tkey),
        to_hex(&input_for(&a)),
        to_hex(&good_sig)
    ));
    j.push_str(&format!(
        "  \"meshtastic\": {{\n    \"x25519_pub\": \"{MESH_X_PUB}\"\n  }},\n"
    ));
    j.push_str("  \"cases\": [\n");
    for (i, (name, why, wire, node, plug)) in cases.iter().enumerate() {
        j.push_str(&format!(
            "    {{\n      \"name\": \"{name}\",\n      \"rationale\": \"{why}\",\n      \"transport_id\": {},\n      \"wire\": \"{}\",\n      \"node_leg\": \"{node}\",\n      \"plug\": \"{plug}\"\n    }}{}\n",
            wire[33],
            to_hex(wire),
            if i + 1 < cases.len() { "," } else { "" }
        ));
    }
    j.push_str("  ]\n}\n");
    // Self-check before writing: the output parses with the strict reader.
    let _ = parse(&j, "generated");
    fs::write(&out, j).expect("write vectors");
}
