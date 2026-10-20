//! Test-only identity handle (C2) — an in-memory [`Sealer`] double over a
//! real `id111` keyset (Ed25519 signing, X25519 pair-key ECDH) with a FAKE
//! AEAD: a keystream + keyed-BLAKE2b tag derived from public values. It keeps
//! the real packet geometry (header ‖ epk ‖ ct ‖ tag16) so size and parsing
//! tests see true lengths; it provides no confidentiality and is never
//! compiled outside `cfg(test)`. The real handle is F2's, over
//! `id111::seal::{seal_payload, open_payload}`.
//!
//! Also carries the tiny strict JSON reader the frozen-vector tests use
//! (`serde_json` is not in the crate's dependency set).

use std::collections::HashMap;
use std::sync::Mutex;

use id111::announce::Announce;
use id111::keyset::{Keyset, NodeId};
use id111::seal::{OpenError, SealHeader};

use crate::envelope::{blake2b_256, derive_pair_key, header_bytes, keyed_blake2b_128, PeerKeys};
use crate::sealer::{Direction, Freshness, Key32, Sealer, SealerError};

/// Canonical-12 test identity (same as `2-identity/test-vectors`).
pub const MNEMONIC_A: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
/// Canonical-24 test identity (same as `2-identity/test-vectors`).
pub const MNEMONIC_B: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

pub struct TestSealer {
    keyset: Keyset,
    send: Mutex<HashMap<NodeId, u64>>,
    high: Mutex<HashMap<NodeId, u64>>,
    observed: Mutex<Vec<(NodeId, u64)>>,
    forgotten: Mutex<Vec<Key32>>,
    announce: Mutex<u64>,
    seq: Mutex<u64>,
}

impl TestSealer {
    pub fn from_keyset(keyset: Keyset) -> Self {
        TestSealer {
            keyset,
            send: Mutex::new(HashMap::new()),
            high: Mutex::new(HashMap::new()),
            observed: Mutex::new(Vec::new()),
            forgotten: Mutex::new(Vec::new()),
            announce: Mutex::new(0),
            seq: Mutex::new(0),
        }
    }

    pub fn from_words(words: &str) -> Self {
        Self::from_keyset(Keyset::derive_from_mnemonic(words).expect("test mnemonic"))
    }

    /// A deterministic throwaway identity.
    pub fn from_seed(seed: u8) -> Self {
        let mut b = [0u8; 64];
        b[..32].copy_from_slice(&blake2b_256(&[b"c2-test-sign", &[seed]]));
        b[32..].copy_from_slice(&blake2b_256(&[b"c2-test-ecdh", &[seed]]));
        Self::from_keyset(Keyset::from_bytes(&b).expect("test keyset"))
    }

    /// A deterministic throwaway identity named by `label`.
    pub fn from_label(label: &str) -> Self {
        let mut b = [0u8; 64];
        b[..32].copy_from_slice(&blake2b_256(&[b"c2-test-sign", label.as_bytes()]));
        b[32..].copy_from_slice(&blake2b_256(&[b"c2-test-ecdh", label.as_bytes()]));
        Self::from_keyset(Keyset::from_bytes(&b).expect("test keyset"))
    }

    /// This identity's NodeId (inherent; same as `Sealer::node_id`).
    pub fn node_id_value(&self) -> NodeId {
        self.keyset.node_id()
    }

    pub fn keyset(&self) -> &Keyset {
        &self.keyset
    }

    pub fn enc_pubkey(&self) -> Key32 {
        self.keyset.x25519_public()
    }

    pub fn keys(&self) -> PeerKeys {
        PeerKeys {
            node_id: self.keyset.node_id(),
            enc_pubkey: self.enc_pubkey(),
        }
    }

    /// Every `observe_counter` call so far (tests assert untouched windows).
    pub fn observed(&self) -> Vec<(NodeId, u64)> {
        self.observed.lock().map(|v| v.clone()).unwrap_or_default()
    }

    /// Every `forget_pair_key` call so far, in order.
    pub fn forgotten(&self) -> Vec<Key32> {
        self.forgotten.lock().map(|v| v.clone()).unwrap_or_default()
    }

    fn fake_key(epk: &Key32, recipient: &Key32) -> Key32 {
        blake2b_256(&[b"c2-test-aead", epk, recipient])
    }

    fn keystream_xor(key: &Key32, data: &mut [u8]) {
        for (i, chunk) in data.chunks_mut(32).enumerate() {
            let block = blake2b_256(&[key, &(i as u64).to_le_bytes()]);
            for (d, k) in chunk.iter_mut().zip(block.iter()) {
                *d ^= k;
            }
        }
    }
}

impl Sealer for TestSealer {
    fn node_id(&self) -> NodeId {
        self.keyset.node_id()
    }

    fn own_announce(&self) -> Vec<u8> {
        Announce::create(&self.keyset, 1, &[1, 0, 0, 0])
            .map(|a| a.encode())
            .unwrap_or_default()
    }

    fn seal_payload(
        &self,
        recipient_enc_pubkey: &Key32,
        header: &SealHeader,
        inner: &mut dyn FnMut(&Key32) -> Vec<u8>,
    ) -> Result<Vec<u8>, SealerError> {
        let n = {
            let mut s = self.seq.lock().map_err(|_| SealerError::Entropy)?;
            *s += 1;
            *s
        };
        let epk = blake2b_256(&[
            b"c2-test-epk",
            self.keyset.node_id().as_bytes(),
            &n.to_le_bytes(),
        ]);
        let hb = header_bytes(header).map_err(|_| SealerError::Entropy)?;
        let mut ct = inner(&epk);
        let k = Self::fake_key(&epk, recipient_enc_pubkey);
        Self::keystream_xor(&k, &mut ct);
        let tag = keyed_blake2b_128(&k, &[&hb, &epk, &ct]).map_err(|_| SealerError::Entropy)?;
        let mut out = hb;
        out.extend_from_slice(&epk);
        out.extend_from_slice(&ct);
        out.extend_from_slice(&tag);
        Ok(out)
    }

    fn open_payload(&self, packet: &[u8]) -> Result<(SealHeader, Key32, Vec<u8>), SealerError> {
        let bad = || SealerError::Open(OpenError::Truncated);
        if packet.len() < 11 + 32 + 16 {
            return Err(bad());
        }
        let extra_len = usize::from(packet[10]);
        let hlen = 11 + extra_len;
        if packet.len() < hlen + 32 + 16 {
            return Err(bad());
        }
        if packet[1] != 0x01 {
            return Err(SealerError::Open(OpenError::UnsupportedCryptoMode(
                packet[1],
            )));
        }
        let mut counter = [0u8; 8];
        counter.copy_from_slice(&packet[2..10]);
        let header = SealHeader {
            version: packet[0],
            crypto_mode: packet[1],
            counter: u64::from_le_bytes(counter),
            extra: packet[11..hlen].to_vec(),
        };
        let mut epk = [0u8; 32];
        epk.copy_from_slice(&packet[hlen..hlen + 32]);
        let (ct, tag) = packet[hlen + 32..].split_at(packet.len() - hlen - 32 - 16);
        let k = Self::fake_key(&epk, &self.enc_pubkey());
        let want = keyed_blake2b_128(&k, &[&packet[..hlen], &epk, ct])
            .map_err(|_| SealerError::Entropy)?;
        if want != tag {
            return Err(SealerError::Open(OpenError::AuthFailed));
        }
        let mut inner = ct.to_vec();
        Self::keystream_xor(&k, &mut inner);
        Ok((header, epk, inner))
    }

    fn pair_key(
        &self,
        peer_enc_pubkey: &Key32,
        direction: Direction,
    ) -> Result<Key32, SealerError> {
        let ss = self
            .keyset
            .ecdh(peer_enc_pubkey)
            .map_err(|e| SealerError::Open(OpenError::Ecdh(e)))?;
        let own = self.enc_pubkey();
        let key = match direction {
            Direction::Outbound => derive_pair_key(&ss, &own, peer_enc_pubkey),
            Direction::Inbound => derive_pair_key(&ss, peer_enc_pubkey, &own),
        };
        Ok(*key)
    }

    fn forget_pair_key(&self, peer_enc_pubkey: &Key32) {
        if let Ok(mut v) = self.forgotten.lock() {
            v.push(*peer_enc_pubkey);
        }
    }

    fn sign(&self, tag: &[u8], msg: &[u8]) -> [u8; 64] {
        self.keyset.sign_with_tag(tag, msg)
    }

    fn next_send_counter(&self, peer: &NodeId) -> Result<u64, SealerError> {
        let mut m = self
            .send
            .lock()
            .map_err(|_| SealerError::Counters("poisoned".into()))?;
        let c = m.entry(*peer).or_insert(0);
        *c += 1;
        Ok(*c)
    }

    fn observe_counter(&self, peer: &NodeId, counter: u64) -> Result<Freshness, SealerError> {
        self.observed
            .lock()
            .map_err(|_| SealerError::Counters("poisoned".into()))?
            .push((*peer, counter));
        let mut m = self
            .high
            .lock()
            .map_err(|_| SealerError::Counters("poisoned".into()))?;
        let hw = m.entry(*peer).or_insert(0);
        let fresh = counter > *hw;
        *hw = (*hw).max(counter);
        Ok(if fresh {
            Freshness::Fresh
        } else {
            Freshness::Stale
        })
    }

    fn next_announce_counter(&self) -> Result<u64, SealerError> {
        let mut a = self
            .announce
            .lock()
            .map_err(|_| SealerError::Counters("poisoned".into()))?;
        *a += 1;
        Ok(*a)
    }

    fn announce_last(&self) -> u64 {
        self.announce.lock().map(|a| *a).unwrap_or(0)
    }

    fn random(&self, out: &mut [u8]) -> Result<(), SealerError> {
        let n = {
            let mut s = self.seq.lock().map_err(|_| SealerError::Entropy)?;
            *s += 1;
            *s
        };
        for (i, chunk) in out.chunks_mut(32).enumerate() {
            let block = blake2b_256(&[b"c2-test-rng", &n.to_le_bytes(), &(i as u64).to_le_bytes()]);
            chunk.copy_from_slice(&block[..chunk.len()]);
        }
        Ok(())
    }

    fn store_secret(&self) -> Result<Key32, SealerError> {
        // Test stand-in for §10.2: a fixed per-node secret (never the real
        // derivation — the X25519 private key stays behind the real handle).
        Ok(blake2b_256(&[
            b"c2-test-store-secret",
            self.node_id().as_bytes(),
        ]))
    }
}

// ===========================================================================
// Strict JSON reader (subset: objects, arrays, strings, unsigned ints,
// true/false/null; duplicate keys and trailing garbage rejected) and writer
// helpers for the frozen vectors. Test-only.
// ===========================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    Num(u64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\n' | b'\r' | b'\t') {
            self.i += 1;
        }
    }
    fn peek(&self) -> u8 {
        *self.b.get(self.i).expect("json: unexpected end")
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
    fn lit(&mut self, w: &str) {
        assert!(
            self.b[self.i..].starts_with(w.as_bytes()),
            "json: bad literal"
        );
        self.i += w.len();
    }
    fn string(&mut self) -> String {
        self.eat(b'"');
        let mut out = Vec::new();
        loop {
            let c = self.peek();
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
                        _ => panic!("json: unsupported escape"),
                    }
                }
                c => out.push(c),
            }
        }
        String::from_utf8(out).expect("json: utf-8")
    }
    fn value(&mut self) -> J {
        self.ws();
        match self.peek() {
            b'{' => {
                self.i += 1;
                let mut m: Vec<(String, J)> = Vec::new();
                self.ws();
                if self.peek() == b'}' {
                    self.i += 1;
                    return J::Obj(m);
                }
                loop {
                    self.ws();
                    let k = self.string();
                    assert!(m.iter().all(|(x, _)| *x != k), "json: duplicate key {k}");
                    self.ws();
                    self.eat(b':');
                    let v = self.value();
                    m.push((k, v));
                    self.ws();
                    match self.peek() {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return J::Obj(m);
                        }
                        c => panic!("json: unexpected {:?}", c as char),
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.peek() == b']' {
                    self.i += 1;
                    return J::Arr(a);
                }
                loop {
                    a.push(self.value());
                    self.ws();
                    match self.peek() {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return J::Arr(a);
                        }
                        c => panic!("json: unexpected {:?}", c as char),
                    }
                }
            }
            b'"' => J::Str(self.string()),
            b't' => {
                self.lit("true");
                J::Bool(true)
            }
            b'f' => {
                self.lit("false");
                J::Bool(false)
            }
            b'n' => {
                self.lit("null");
                J::Null
            }
            b'0'..=b'9' => {
                let s = self.i;
                while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
                    self.i += 1;
                }
                J::Num(
                    std::str::from_utf8(&self.b[s..self.i])
                        .unwrap()
                        .parse()
                        .expect("json: u64"),
                )
            }
            c => panic!("json: unexpected {:?}", c as char),
        }
    }
}

pub fn parse_json(text: &str) -> J {
    let mut p = P {
        b: text.as_bytes(),
        i: 0,
    };
    let v = p.value();
    p.ws();
    assert_eq!(p.i, p.b.len(), "json: trailing garbage");
    v
}

impl J {
    pub fn get(&self, k: &str) -> &J {
        match self {
            J::Obj(m) => m
                .iter()
                .find(|(x, _)| x == k)
                .map(|(_, v)| v)
                .unwrap_or_else(|| panic!("vector field {k:?} missing")),
            _ => panic!("not an object reading {k:?}"),
        }
    }
    pub fn s(&self) -> &str {
        match self {
            J::Str(s) => s,
            o => panic!("expected string, got {o:?}"),
        }
    }
    pub fn n(&self) -> u64 {
        match self {
            J::Num(n) => *n,
            o => panic!("expected number, got {o:?}"),
        }
    }
    pub fn dec(&self) -> u64 {
        self.s().parse().expect("decimal string")
    }
    pub fn b(&self) -> bool {
        match self {
            J::Bool(b) => *b,
            o => panic!("expected bool, got {o:?}"),
        }
    }
    pub fn a(&self) -> &[J] {
        match self {
            J::Arr(a) => a,
            o => panic!("expected array, got {o:?}"),
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, J::Null)
    }
    pub fn hex(&self) -> Vec<u8> {
        unhex(self.s())
    }
    pub fn h32(&self) -> [u8; 32] {
        self.hex().try_into().expect("32-byte hex")
    }
    pub fn h16(&self) -> [u8; 16] {
        self.hex().try_into().expect("16-byte hex")
    }
    pub fn h64(&self) -> [u8; 64] {
        self.hex().try_into().expect("64-byte hex")
    }
}

pub fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "odd hex");
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex digit"))
        .collect()
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn vectors_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors")
}

pub fn load(name: &str) -> J {
    let p = vectors_dir().join(name);
    let t = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    parse_json(&t)
}

// ===========================================================================
// Frozen vectors (spec §20 G4): generator (ignored; writes the JSON files)
// and loader tests (assert every file against the public API).
// ===========================================================================

mod frozen {
    use super::*;
    use crate::ack::{
        ack_sig_input, strong_hash, AckBlock, AckPayload, AckRange, PartialAck, StrongReceipt,
    };
    use crate::custody::{custody_sig_input, pkt_ref, CustodyAck, CustodyReceipt, CustodyStatus};
    use crate::enroll::{
        enroll_hash, parse_code, verify_enroll_hash, EnrollError, Offer, Presented,
    };
    use crate::envelope::{
        derive_pair_key, header_bytes, pair_mac, sig_dest, sig_input, verify_pair_mac, FragPos,
        Inner, Kind, OuterExtra, Priority, RawTlv, MSG_SIG_TAG,
    };
    use crate::ids::{decode_varint, put_varint, MsgSeq, Reader};
    use id111::keyset::verify_strict_tagged;
    use std::fmt::Write as _;

    const FROZEN: &str = "2026-10-14";

    // ------------------------------------------------------------ writer

    struct W;
    impl W {
        fn s(v: &str) -> String {
            format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
        }
        fn h(b: &[u8]) -> String {
            Self::s(&hex(b))
        }
        fn opt_h(b: Option<&[u8]>) -> String {
            b.map_or("null".into(), Self::h)
        }
    }

    fn obj(fields: &[(&str, String)]) -> String {
        let mut o = String::from("{");
        for (i, (k, v)) in fields.iter().enumerate() {
            if i > 0 {
                o.push_str(", ");
            }
            let _ = write!(o, "{}: {}", W::s(k), v);
        }
        o.push('}');
        o
    }

    fn arr(items: &[String], pretty: bool) -> String {
        if pretty {
            format!("[\n    {}\n  ]", items.join(",\n    "))
        } else {
            format!("[{}]", items.join(", "))
        }
    }

    fn doc(fields: &[(&str, String)]) -> String {
        let mut o = String::from("{\n");
        for (i, (k, v)) in fields.iter().enumerate() {
            let _ = write!(o, "  {}: {}", W::s(k), v);
            o.push_str(if i + 1 < fields.len() { ",\n" } else { "\n" });
        }
        o.push_str("}\n");
        o
    }

    fn ident(t: &TestSealer, words: &str) -> String {
        obj(&[
            ("mnemonic", W::s(words)),
            ("node_id", W::h(t.node_id().as_bytes())),
            ("enc_pubkey", W::h(&t.enc_pubkey())),
        ])
    }

    fn identities(a: &TestSealer, b: &TestSealer) -> String {
        format!(
            "{{\n    \"a\": {},\n    \"b\": {}\n  }}",
            ident(a, MNEMONIC_A),
            ident(b, MNEMONIC_B)
        )
    }

    fn header(meta: &str, desc: &str, prov: &str) -> Vec<(&'static str, String)> {
        vec![
            ("spec", W::s(meta)),
            ("frozen", W::s(FROZEN)),
            ("description", W::s(desc)),
            ("provenance", W::s(prov)),
        ]
    }

    fn write(name: &str, text: &str) {
        let p = vectors_dir().join(name);
        std::fs::write(&p, text).unwrap_or_else(|e| panic!("write {}: {e}", p.display()));
    }

    fn tok<E: std::fmt::Debug>(e: E) -> String {
        format!("{e:?}")
    }

    fn ab() -> (TestSealer, TestSealer) {
        (
            TestSealer::from_words(MNEMONIC_A),
            TestSealer::from_words(MNEMONIC_B),
        )
    }

    fn fixed_epk(name: &str) -> [u8; 32] {
        crate::envelope::blake2b_256(&[b"c2-vector-epk", name.as_bytes()])
    }

    // ------------------------------------------------------------ ids

    fn ids_doc() -> String {
        let vals: [u64; 10] = [
            0,
            1,
            127,
            128,
            300,
            16_383,
            16_384,
            u64::from(u32::MAX),
            1 << 63,
            u64::MAX,
        ];
        let varints: Vec<String> = vals
            .iter()
            .map(|v| {
                let mut b = Vec::new();
                put_varint(&mut b, *v);
                obj(&[("value", W::s(&v.to_string())), ("hex", W::h(&b))])
            })
            .collect();
        let seqs: Vec<String> = [
            MsgSeq::CONTROL,
            MsgSeq::new(1, 1),
            MsgSeq::new(127, 127),
            MsgSeq::new(128, 300),
            MsgSeq::new(1_000_000, u32::MAX),
            MsgSeq::new(crate::ids::MAX_EPOCH, 1),
        ]
        .iter()
        .map(|s| {
            obj(&[
                ("epoch", W::s(&s.epoch.to_string())),
                ("n", s.n.to_string()),
                ("hex", W::h(&s.to_bytes())),
            ])
        })
        .collect();
        let rejects: Vec<(&str, Vec<u8>)> = vec![
            ("empty", vec![]),
            ("continuation without end", vec![0x80]),
            ("overlong zero", vec![0x80, 0x00]),
            ("overlong 127", vec![0xff, 0x00]),
            (
                "10th byte above bit 63",
                vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02],
            ),
            ("11 bytes", vec![0xff; 11]),
        ];
        let rj: Vec<String> = rejects
            .iter()
            .map(|(n, b)| {
                obj(&[
                    ("name", W::s(n)),
                    ("hex", W::h(b)),
                    ("expect", W::s(&tok(decode_varint(b).unwrap_err()))),
                ])
            })
            .collect();
        let mut wide = vec![0x01];
        put_varint(&mut wide, u64::from(u32::MAX) + 1);
        let mut over = Vec::new();
        put_varint(&mut over, crate::ids::MAX_EPOCH + 1);
        over.push(0x01);
        let mut max = Vec::new();
        put_varint(&mut max, u64::MAX);
        max.push(0x01);
        let seq_rj: Vec<String> = [
            ("n above u32", wide),
            ("epoch above i64::MAX (2^63)", over),
            ("epoch u64::MAX", max),
        ]
        .iter()
        .map(|(n, b)| {
            obj(&[
                ("name", W::s(n)),
                ("hex", W::h(b)),
                (
                    "expect",
                    W::s(&tok(MsgSeq::decode(&mut Reader::new(b)).unwrap_err())),
                ),
            ])
        })
        .collect();
        let mut f = header(
            "§3 varints (unsigned LEB128) + §4 msg_seq = epoch(varint) ‖ n(varint)",
            "Minimal LEB128 only: over-long encodings, an 11th byte and values above u64::MAX are rejected; n must fit u32 and epoch must not exceed i64::MAX (the store's INTEGER), both typed codec errors so a hostile contact can never turn a valid-MAC packet into a store error. Values are decimal strings (u64). 'expect' is the Debug form of mesh111::ids::ReadError.",
            "Generated by mesh111 test_double::frozen::write_vectors (cargo test -- --ignored); crosscheck.py re-derives every positive.",
        );
        f.push(("varints", arr(&varints, true)));
        f.push(("msg_seq", arr(&seqs, true)));
        f.push(("varint_reject", arr(&rj, true)));
        f.push(("msg_seq_reject", arr(&seq_rj, true)));
        doc(&f)
    }

    // ------------------------------------------------------------ ack

    fn two_epoch_block() -> AckBlock {
        AckBlock {
            epoch: 7,
            prefix: 4,
            ranges: vec![
                AckRange {
                    epoch: 3,
                    start: 1,
                    len: 5,
                },
                AckRange {
                    epoch: 3,
                    start: 8,
                    len: 2,
                },
                AckRange {
                    epoch: 7,
                    start: 6,
                    len: 1,
                },
            ],
            partials: vec![PartialAck {
                seq: MsgSeq::new(7, 5),
                frag_bitmap: 0b0000_0000_0000_0101,
            }],
            strong: vec![StrongReceipt {
                seq: MsgSeq::new(3, 2),
                payload_hash: strong_hash(b"agent-op-payload"),
            }],
        }
    }

    fn blocks() -> Vec<(&'static str, AckBlock)> {
        vec![
            (
                "prefix only",
                AckBlock {
                    epoch: 1,
                    prefix: 3,
                    ..AckBlock::default()
                },
            ),
            (
                "nothing delivered yet in a fresh epoch",
                AckBlock {
                    epoch: 12,
                    prefix: 0,
                    ranges: vec![AckRange {
                        epoch: 12,
                        start: 2,
                        len: 3,
                    }],
                    ..AckBlock::default()
                },
            ),
            ("two epochs: frozen ranges + current", two_epoch_block()),
            (
                "eight ranges (max)",
                AckBlock {
                    epoch: 2,
                    prefix: 1,
                    ranges: (0..8)
                        .map(|i| AckRange {
                            epoch: 2,
                            start: 3 + 3 * i,
                            len: 1,
                        })
                        .collect(),
                    ..AckBlock::default()
                },
            ),
            (
                "partial bitmaps of in-flight Tier-1 messages",
                AckBlock {
                    epoch: 5,
                    prefix: 9,
                    partials: vec![
                        PartialAck {
                            seq: MsgSeq::new(5, 10),
                            frag_bitmap: 0x8001,
                        },
                        PartialAck {
                            seq: MsgSeq::new(5, 11),
                            frag_bitmap: 0x00ff,
                        },
                    ],
                    ..AckBlock::default()
                },
            ),
            (
                "large epoch and n (multi-byte varints)",
                AckBlock {
                    epoch: 300_000,
                    prefix: 70_000,
                    strong: vec![StrongReceipt {
                        seq: MsgSeq::new(300_000, 69_999),
                        payload_hash: strong_hash(b""),
                    }],
                    ..AckBlock::default()
                },
            ),
        ]
    }

    fn block_json(name: &str, b: &AckBlock) -> String {
        let ranges: Vec<String> = b
            .ranges
            .iter()
            .map(|r| format!("[{}, {}, {}]", W::s(&r.epoch.to_string()), r.start, r.len))
            .collect();
        let partials: Vec<String> = b
            .partials
            .iter()
            .map(|p| {
                format!(
                    "[{}, {}, {}]",
                    W::s(&p.seq.epoch.to_string()),
                    p.seq.n,
                    p.frag_bitmap
                )
            })
            .collect();
        let strong: Vec<String> = b
            .strong
            .iter()
            .map(|s| {
                format!(
                    "[{}, {}, {}]",
                    W::s(&s.seq.epoch.to_string()),
                    s.seq.n,
                    W::h(&s.payload_hash)
                )
            })
            .collect();
        obj(&[
            ("name", W::s(name)),
            ("epoch", W::s(&b.epoch.to_string())),
            ("prefix", b.prefix.to_string()),
            ("ranges", arr(&ranges, false)),
            ("partials", arr(&partials, false)),
            ("strong", arr(&strong, false)),
            ("hex", W::h(&b.encode().unwrap())),
        ])
    }

    fn block_from(j: &J) -> AckBlock {
        AckBlock {
            epoch: j.get("epoch").dec(),
            prefix: j.get("prefix").n() as u32,
            ranges: j
                .get("ranges")
                .a()
                .iter()
                .map(|r| AckRange {
                    epoch: r.a()[0].dec(),
                    start: r.a()[1].n() as u32,
                    len: r.a()[2].n() as u32,
                })
                .collect(),
            partials: j
                .get("partials")
                .a()
                .iter()
                .map(|p| PartialAck {
                    seq: MsgSeq::new(p.a()[0].dec(), p.a()[1].n() as u32),
                    frag_bitmap: p.a()[2].n() as u16,
                })
                .collect(),
            strong: j
                .get("strong")
                .a()
                .iter()
                .map(|s| StrongReceipt {
                    seq: MsgSeq::new(s.a()[0].dec(), s.a()[1].n() as u32),
                    payload_hash: s.a()[2].h16(),
                })
                .collect(),
        }
    }

    fn ack_rejects() -> Vec<(&'static str, Vec<u8>)> {
        let good = two_epoch_block().encode().unwrap();
        let mut v = Vec::new();
        let mut short = good.clone();
        short.pop();
        v.push(("ack_len one past the data", short));
        let mut trail = good.clone();
        trail[0] -= 1;
        v.push(("ack_len one short of the body", trail));
        // body: epoch 1, prefix 0, n_ranges 9 ...
        let mut nine = vec![1u8, 0, 9];
        for i in 0..9u8 {
            nine.extend_from_slice(&[1, 2 + 2 * i, 1]);
        }
        nine.extend_from_slice(&[0, 0]);
        v.push(("nine ranges", with_len(&nine)));
        v.push(("zero ack_epoch", with_len(&[0, 0, 0, 0, 0])));
        v.push(("range len 0", with_len(&[1, 0, 1, 1, 1, 0, 0, 0])));
        v.push(("range start 0", with_len(&[1, 0, 1, 1, 0, 1, 0, 0])));
        v.push((
            "range epoch above ack_epoch",
            with_len(&[1, 0, 1, 2, 1, 1, 0, 0]),
        ));
        v.push((
            "range overlapping the prefix",
            with_len(&[2, 3, 1, 2, 3, 1, 0, 0]),
        ));
        v.push((
            "ranges out of order",
            with_len(&[2, 0, 2, 2, 5, 1, 2, 2, 1, 0, 0]),
        ));
        v.push((
            "partial with empty bitmap",
            with_len(&[2, 0, 0, 1, 2, 1, 0, 0, 0]),
        ));
        v.push((
            "partial for an acknowledged message",
            with_len(&[2, 3, 0, 1, 2, 1, 1, 0, 0]),
        ));
        let mut strong = vec![2, 1, 0, 0, 1, 2, 5];
        strong.extend_from_slice(&[0u8; 16]);
        v.push((
            "strong receipt for an unacknowledged message",
            with_len(&strong),
        ));
        v.push((
            "over-long varint in ack_epoch",
            with_len(&[0x81, 0x00, 0, 0, 0, 0]),
        ));
        v
    }

    fn with_len(body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        put_varint(&mut out, body.len() as u64);
        out.extend_from_slice(body);
        out
    }

    fn ack_doc() -> String {
        let (a, b) = ab();
        let bl: Vec<String> = blocks().iter().map(|(n, b)| block_json(n, b)).collect();
        let mut payloads = Vec::new();
        for (name, transferable) in [
            ("plain ACK payload (pair_mac only)", false),
            ("transferable ACK (ack_sig)", true),
        ] {
            let p =
                AckPayload::build(&b, two_epoch_block(), &a.node_id(), 0, transferable).unwrap();
            payloads.push(obj(&[
                ("name", W::s(name)),
                ("acker", W::s("b")),
                ("sender", W::s("a")),
                ("stream", W::s("0")),
                ("block", W::h(&p.block_bytes)),
                (
                    "sig_input",
                    W::h(&ack_sig_input(
                        &b.node_id(),
                        &a.node_id(),
                        0,
                        &p.block_bytes,
                    )),
                ),
                ("ack_sig", W::opt_h(p.ack_sig.as_ref().map(|s| &s[..]))),
                ("payload", W::h(&p.encode())),
            ]));
        }
        let rj: Vec<String> = ack_rejects()
            .iter()
            .map(|(n, b)| {
                obj(&[
                    ("name", W::s(n)),
                    ("hex", W::h(b)),
                    ("expect", W::s(&tok(AckBlock::decode_exact(b).unwrap_err()))),
                ])
            })
            .collect();
        let good = two_epoch_block().encode().unwrap();
        let mut tail = good.clone();
        tail.extend_from_slice(&[0xaa; 63]);
        let prj = vec![obj(&[
            ("name", W::s("63-byte tail (neither empty nor ack_sig)")),
            ("hex", W::h(&tail)),
            ("expect", W::s(&tok(AckPayload::decode(&tail).unwrap_err()))),
        ])];
        let strong: Vec<String> = [&b""[..], b"agent-op-payload"]
            .iter()
            .map(|p| obj(&[("payload", W::h(p)), ("hash", W::h(&strong_hash(p)))]))
            .collect();
        let mut f = header(
            "§6.1 ACK block (ack_len, epochs, prefix, ranges ≤ 8, partial bitmaps, strong receipts) + ack_sig",
            "ack block = ack_len(varint) ‖ ack_epoch ‖ ack_prefix ‖ n_ranges(1) ‖ [epoch,start,len]* ‖ n_partial(1) ‖ [epoch,n,bitmap u16 LE]* ‖ n_strong(1) ‖ [epoch,n,BLAKE2b-128(payload)]*. ranges/partials/strong are [epoch(decimal string), n…]. ack_sig = Ed25519 over '111/ack-sig/v1' ‖ acker ‖ sender ‖ stream(varint) ‖ block bytes incl. ack_len; it is the last 64 B of a standalone ACK payload. The piggybacked placement is frozen in envelope.json ('msg-piggyback-two-epoch-ack'). 'expect' is the Debug form of mesh111::ack::AckError.",
            "Generated by mesh111 test_double::frozen::write_vectors; crosscheck.py re-derives blocks, sig inputs and strong hashes; signatures verified in Rust under id111::keyset::verify_strict_tagged.",
        );
        f.push(("identities", identities(&a, &b)));
        f.push(("blocks", arr(&bl, true)));
        f.push(("payloads", arr(&payloads, true)));
        f.push(("strong_hashes", arr(&strong, true)));
        f.push(("reject", arr(&rj, true)));
        f.push(("reject_payload", arr(&prj, true)));
        doc(&f)
    }

    // ------------------------------------------------------------ envelope

    struct Case {
        name: &'static str,
        from_a: bool,
        counter: u64,
        priority: Priority,
        inner: Inner,
        budget: Option<(usize, bool)>,
    }

    fn cases(a: &TestSealer, b: &TestSealer) -> Vec<Case> {
        let ai = a.node_id();
        let bi = b.node_id();
        let msg = |seq: MsgSeq, payload: &[u8]| Inner::new(&ai, Kind::Msg, seq, payload.to_vec());
        let mut v = Vec::new();
        // §3.3 CBOR {1: "hi"}
        v.push(Case {
            name: "msg-tier0-minimal",
            from_a: true,
            counter: 1,
            priority: Priority::Interactive,
            inner: msg(MsgSeq::new(1, 1), &[0xa1, 0x01, 0x62, b'h', b'i']),
            budget: None,
        });
        let full =
            crate::envelope::payload_budget(160, true, MsgSeq::new(1, 2), None, false).unwrap();
        v.push(Case {
            name: "msg-tier0-fills-160B",
            from_a: true,
            counter: 2,
            priority: Priority::Interactive,
            inner: msg(MsgSeq::new(1, 2), &vec![0x41; full]),
            budget: Some((160, true)),
        });
        let f0 = Some(FragPos { idx: 0, count: 3 });
        let fb = crate::envelope::payload_budget(160, true, MsgSeq::new(5, 7), f0, false).unwrap();
        let mut c = msg(MsgSeq::new(5, 7), &vec![0x46; fb]);
        c.frag = f0;
        v.push(Case {
            name: "msg-tier1-frag-0-of-3-fills-160B",
            from_a: true,
            counter: 3,
            priority: Priority::Interactive,
            inner: c,
            budget: Some((160, true)),
        });
        let mut c = msg(MsgSeq::new(5, 7), b"tail of the message");
        c.frag = Some(FragPos { idx: 2, count: 3 });
        v.push(Case {
            name: "msg-tier1-frag-2-of-3",
            from_a: true,
            counter: 4,
            priority: Priority::Interactive,
            inner: c,
            budget: None,
        });
        let f15 = Some(FragPos { idx: 15, count: 16 });
        let s = MsgSeq::new(300, 200);
        let fb = crate::envelope::payload_budget(233, true, s, f15, false).unwrap();
        let mut c = msg(s, &vec![0x4c; fb]);
        c.frag = f15;
        v.push(Case {
            name: "msg-tier1-frag-15-of-16-fills-233B",
            from_a: true,
            counter: 5,
            priority: Priority::Interactive,
            inner: c,
            budget: Some((233, true)),
        });
        let mut c = msg(MsgSeq::new(2, 3), b"seen + strong");
        c.ext.seen_peer_seq = Some(MsgSeq::new(7, 9));
        c.receipt_strong = true;
        v.push(Case {
            name: "msg-seen-peer-seq-receipt-strong",
            from_a: true,
            counter: 6,
            priority: Priority::Interactive,
            inner: c,
            budget: None,
        });
        let mut c = msg(MsgSeq::new(9, 12), b"reply with an ack");
        c.ack = Some(two_epoch_block());
        v.push(Case {
            name: "msg-piggyback-two-epoch-ack",
            from_a: true,
            counter: 7,
            priority: Priority::Interactive,
            inner: c,
            budget: None,
        });
        let mut c = msg(MsgSeq::new(9, 13), b"future tlv");
        c.ext.unknown.push(RawTlv {
            tlv_type: 0x42,
            value: b"abc".to_vec(),
        });
        v.push(Case {
            name: "msg-unknown-tlv-skipped",
            from_a: true,
            counter: 8,
            priority: Priority::Interactive,
            inner: c,
            budget: None,
        });
        let mut c = Inner::new(
            &ai,
            Kind::AgentOp,
            MsgSeq::new(4, 1),
            b"{\"op\":\"restart\"}".to_vec(),
        );
        c.receipt_strong = true;
        v.push(Case {
            name: "agent-op-signed-dest-recipient",
            from_a: true,
            counter: 9,
            priority: Priority::Interactive,
            inner: c,
            budget: None,
        });
        let mut c = Inner::new(
            &ai,
            Kind::AgentOp,
            MsgSeq::new(4, 3),
            b"second half of a signed op".to_vec(),
        );
        c.frag = Some(FragPos { idx: 1, count: 2 });
        v.push(Case {
            name: "agent-op-signed-frag-1-of-2",
            from_a: true,
            counter: 13,
            priority: Priority::Interactive,
            inner: c,
            budget: None,
        });
        v.push(Case {
            name: "kb-signed-dest-public",
            from_a: true,
            counter: 10,
            priority: Priority::BulkSmall,
            inner: Inner::new(&ai, Kind::Kb, MsgSeq::new(4, 2), b"kb entry".to_vec()),
            budget: None,
        });
        let mut c = msg(MsgSeq::new(1, 1), b"hello, first contact");
        c.card = Some(a.own_announce());
        v.push(Case {
            name: "msg-intro-first-contact",
            from_a: true,
            counter: 11,
            priority: Priority::Interactive,
            inner: c,
            budget: None,
        });
        let mut c = Inner::new(&ai, Kind::Enroll, MsgSeq::CONTROL, b"7BCD9KMQ".to_vec());
        c.card = Some(a.own_announce());
        v.push(Case {
            name: "enroll-intro-to-hub",
            from_a: true,
            counter: 12,
            priority: Priority::Control,
            inner: c,
            budget: None,
        });
        let ackp = AckPayload::build(b, two_epoch_block(), &ai, 0, true).unwrap();
        v.push(Case {
            name: "ack-standalone-transferable",
            from_a: false,
            counter: 1,
            priority: Priority::Control,
            inner: Inner::new(&bi, Kind::Ack, MsgSeq::CONTROL, ackp.encode()),
            budget: None,
        });
        let rec = CustodyReceipt {
            status: CustodyStatus::Accepted,
            custody_seq: 42,
            pkt_ref: pkt_ref(b"sealed item bytes"),
        };
        let ca = CustodyAck::build(b, rec, &ai, false).unwrap();
        v.push(Case {
            name: "custody-ack-accepted",
            from_a: false,
            counter: 2,
            priority: Priority::Control,
            inner: Inner::new(&bi, Kind::CustodyAck, MsgSeq::CONTROL, ca.encode().unwrap()),
            budget: None,
        });
        v
    }

    fn case_json(
        c: &Case,
        a: &TestSealer,
        b: &TestSealer,
        pk_ab: &[u8; 32],
        pk_ba: &[u8; 32],
    ) -> String {
        let (snd, rcv, key) = if c.from_a {
            (a, b, pk_ab)
        } else {
            (b, a, pk_ba)
        };
        let i = &c.inner;
        let header = OuterExtra::to(c.priority, &rcv.node_id()).header(c.counter);
        let hb = header_bytes(&header).unwrap();
        let epk = fixed_epk(c.name);
        let before = i.encode_before_auth().unwrap();
        let mac = pair_mac(key, &hb, &epk, &before).unwrap();
        let (sig_in, sig) = if i.signed {
            let si = sig_input(&snd.node_id(), &sig_dest(i.kind, &rcv.node_id()), 0, i);
            let s = snd.sign(MSG_SIG_TAG, &si);
            (Some(si), Some(s))
        } else {
            (None, None)
        };
        let inner = Inner::finish(&before, &mac, sig.as_ref());
        let budget = c.budget.map_or("null".to_string(), |(bearer, shim)| {
            obj(&[
                ("bearer", bearer.to_string()),
                ("shim", shim.to_string()),
                ("packet_len", (hb.len() + 32 + inner.len() + 16).to_string()),
            ])
        });
        obj(&[
            ("name", W::s(c.name)),
            ("sender", W::s(if c.from_a { "a" } else { "b" })),
            ("recipient", W::s(if c.from_a { "b" } else { "a" })),
            ("counter", W::s(&c.counter.to_string())),
            ("priority", (c.priority as u8).to_string()),
            ("epk", W::h(&epk)),
            ("kind", i.kind.byte().to_string()),
            ("epoch", W::s(&i.seq.epoch.to_string())),
            ("n", i.seq.n.to_string()),
            (
                "frag",
                i.frag
                    .map_or("null".into(), |f| format!("[{}, {}]", f.idx, f.count)),
            ),
            (
                "seen_peer_seq",
                i.ext.seen_peer_seq.map_or("null".into(), |s| {
                    format!("[{}, {}]", W::s(&s.epoch.to_string()), s.n)
                }),
            ),
            (
                "unknown_tlvs",
                arr(
                    &i.ext
                        .unknown
                        .iter()
                        .map(|t| format!("[{}, {}]", t.tlv_type, W::h(&t.value)))
                        .collect::<Vec<_>>(),
                    false,
                ),
            ),
            (
                "ack",
                W::opt_h(i.ack.as_ref().map(|a| a.encode().unwrap()).as_deref()),
            ),
            ("card", W::opt_h(i.card.as_deref())),
            ("receipt_strong", i.receipt_strong.to_string()),
            ("signed", i.signed.to_string()),
            ("payload", W::h(&i.payload)),
            ("header", W::h(&hb)),
            ("inner_before_auth", W::h(&before)),
            ("pair_mac", W::h(&mac)),
            ("sig_input", W::opt_h(sig_in.as_deref())),
            ("sig", W::opt_h(sig.as_ref().map(|s| &s[..]))),
            ("inner", W::h(&inner)),
            ("budget", budget),
        ])
    }

    fn inner_rejects(a: &TestSealer) -> Vec<(&'static str, Vec<u8>)> {
        let ai = a.node_id();
        let base = Inner::new(&ai, Kind::Msg, MsgSeq::new(1, 1), b"p".to_vec())
            .encode_before_auth()
            .unwrap();
        let r = &base[..4];
        let m = |kind: u8, flags: u8, rest: &[u8]| {
            let mut v = r.to_vec();
            v.push(kind);
            v.push(flags);
            v.extend_from_slice(rest);
            v
        };
        let mut card136 = vec![136u8];
        card136.extend_from_slice(&a.own_announce()[..136]);
        let mut epoch_over = Vec::new();
        put_varint(&mut epoch_over, crate::ids::MAX_EPOCH + 1);
        epoch_over.push(1);
        let card = a.own_announce();
        let mut intro_frag = vec![1u8, 1, 0, 2, card.len() as u8];
        intro_frag.extend_from_slice(&card);
        let mut intro_agent_op = vec![1u8, 1, card.len() as u8];
        intro_agent_op.extend_from_slice(&card);
        vec![
            ("truncated before msg_seq", base[..6].to_vec()),
            ("reserved flag bit 6", m(0x01, 0x40, &[1, 1])),
            ("reserved flag bit 7", m(0x01, 0x80, &[1, 1])),
            ("unknown kind 0x0a", m(0x0a, 0, &[0, 0])),
            ("BROADCAST is reserved in v1", m(0x30, 0x08, &[0, 0])),
            ("OBJ_ADVERT is v1.1", m(0x20, 0, &[0, 0])),
            ("control kind with msg_seq 1|1", m(0x02, 0, &[1, 1])),
            ("MSG with msg_seq 0|0", m(0x01, 0, &[0, 0])),
            ("MSG with n = 0", m(0x01, 0, &[1, 0])),
            ("MSG with epoch = 0", m(0x01, 0, &[0, 1])),
            ("MSG with epoch above i64::MAX", m(0x01, 0, &epoch_over)),
            ("frag_count 17", m(0x01, 0x01, &[1, 1, 0, 17])),
            ("frag_idx = frag_count", m(0x01, 0x01, &[1, 1, 3, 3])),
            ("frag_count 0", m(0x01, 0x01, &[1, 1, 0, 0])),
            ("HAS_EXT with no TLVs", m(0x01, 0x02, &[1, 1, 0])),
            (
                "TLV 0x20 delegation trailer",
                m(0x01, 0x02, &[1, 1, 1, 0x20, 0]),
            ),
            (
                "TLV 0x10 ratchet pubkey (reserved)",
                m(0x01, 0x02, &[1, 1, 1, 0x10, 0]),
            ),
            (
                "TLV 0x21 signer_pubkey (reserved)",
                m(0x01, 0x02, &[1, 1, 1, 0x21, 0]),
            ),
            ("duplicate TLV", m(0x01, 0x02, &[1, 1, 2, 0x42, 0, 0x42, 0])),
            (
                "seen_peer_seq 0|0",
                m(0x01, 0x02, &[1, 1, 1, 0x01, 2, 0, 0]),
            ),
            (
                "seen_peer_seq trailing byte",
                m(0x01, 0x02, &[1, 1, 1, 0x01, 3, 1, 1, 0]),
            ),
            (
                "TLV overruns the inner",
                m(0x01, 0x02, &[1, 1, 1, 0x42, 9, 0]),
            ),
            ("over-long msg_seq varint", m(0x01, 0, &[0x81, 0x00, 1])),
            (
                "PIGGYBACK_ACK on ACK",
                m(0x02, 0x04, &[0, 0, 5, 1, 0, 0, 0, 0]),
            ),
            (
                "piggyback ack_len mismatch",
                m(0x01, 0x04, &[1, 1, 6, 1, 0, 0, 0, 0]),
            ),
            ("AGENT_OP without SIGNED", m(0x10, 0, &[1, 1])),
            ("KB without SIGNED", m(0x11, 0, &[1, 1])),
            ("RECEIPT_STRONG on MAILBOX_FETCH", m(0x05, 0x10, &[0, 0])),
            ("HAS_FRAG on MAILBOX_FETCH", m(0x05, 0x01, &[0, 0, 0, 2])),
            ("INTRO on AGENT_OP", m(0x10, 0x28, &intro_agent_op)),
            ("ENROLL without INTRO", m(0x08, 0, &[0, 0])),
            ("INTRO with HAS_FRAG", m(0x01, 0x21, &intro_frag)),
            (
                "card length 136",
                m(0x01, 0x20, &[[1u8, 1].as_slice(), &card136].concat()),
            ),
        ]
    }

    fn envelope_doc() -> String {
        let (a, b) = ab();
        let ss = a.keyset().ecdh(&b.enc_pubkey()).unwrap();
        let pk_ab = derive_pair_key(&ss, &a.enc_pubkey(), &b.enc_pubkey());
        let pk_ba = derive_pair_key(&ss, &b.enc_pubkey(), &a.enc_pubkey());
        assert_eq!(
            *pk_ab,
            a.pair_key(&b.enc_pubkey(), crate::sealer::Direction::Outbound)
                .unwrap()
        );
        assert_eq!(
            *pk_ab,
            b.pair_key(&a.enc_pubkey(), crate::sealer::Direction::Inbound)
                .unwrap()
        );
        let cs: Vec<String> = cases(&a, &b)
            .iter()
            .map(|c| case_json(c, &a, &b, &pk_ab, &pk_ba))
            .collect();
        let rj: Vec<String> = inner_rejects(&a)
            .iter()
            .map(|(n, v)| {
                obj(&[
                    ("name", W::s(n)),
                    ("before_auth", W::h(v)),
                    (
                        "expect",
                        W::s(&tok(Inner::decode_before_auth(v).unwrap_err())),
                    ),
                ])
            })
            .collect();
        let extras: Vec<(&str, Vec<u8>)> = vec![
            ("GROUP bit 5", vec![0x60, 1, 2, 3, 4]),
            ("RATCHET bit 4", vec![0x50, 1, 2, 3, 4]),
            ("reserved bit 3", vec![0x48, 1, 2, 3, 4]),
            ("reserved bit 0", vec![0x01, 1, 2, 3, 4]),
            ("extra_len 4", vec![0x40, 1, 2, 3]),
            ("extra_len 6", vec![0x40, 1, 2, 3, 4, 5]),
        ];
        let erj: Vec<String> = extras
            .iter()
            .map(|(n, v)| {
                obj(&[
                    ("name", W::s(n)),
                    ("extra", W::h(v)),
                    ("expect", W::s(&tok(OuterExtra::decode(v).unwrap_err()))),
                ])
            })
            .collect();
        let mut hdrs = Vec::new();
        for (n, ver, mode) in [
            ("version 0x02", 2u8, 1u8),
            ("crypto_mode 0x02", 1, 2),
            ("crypto_mode 0x00", 1, 0),
        ] {
            let mut h = OuterExtra::to(Priority::Interactive, &b.node_id()).header(1);
            h.version = ver;
            h.crypto_mode = mode;
            hdrs.push(obj(&[
                ("name", W::s(n)),
                ("header", W::h(&header_bytes(&h).unwrap())),
                (
                    "expect",
                    W::s(&tok(crate::envelope::check_header(&h).unwrap_err())),
                ),
            ]));
        }
        let mut f = header(
            "§3 envelope: outer extra (prio_flags, dest_hint), inner layout, flags, TLVs, kinds §3.2, pair_mac, sig with dest; §3 size table",
            "Each case freezes the L2 header bytes, the inner bytes before auth, pair_mac = keyed BLAKE2b-128(pair key, header ‖ epk ‖ before_auth), sig (SIGNED kinds: Ed25519 over '111/msg/v1' ‖ sig_input, where sig_input = sender ‖ dest ‖ stream ‖ msg_seq ‖ kind ‖ flags ‖ frag_idx ‖ frag_count ‖ BLAKE2b-256(payload); frag fields 0 ‖ 0 when unfragmented, so a pair-key holder cannot recombine signed fragments) and the full inner (before_auth ‖ pair_mac ‖ [sig]) — the AEAD plaintext. The AEAD itself is id111's (identity/test-vectors/layer5/sealed-payload.json); epk here is a fixed value standing in for the sealer's ephemeral. pair key = BLAKE2b-256('111/pair/v1' ‖ X25519 shared ‖ sender_enc_pk ‖ recipient_enc_pk). C2 framing readings: HAS_EXT ⇒ n_ext(1) ‖ TLVs; INTRO ⇒ card_len(1) ‖ card. Every v1 packet is stream 0. 'budget' cases fill a 160/233 B bearer exactly (+1 shim byte). Rejection 'expect' tokens are the Debug form of mesh111::envelope::EnvelopeError.",
            "Generated by mesh111 test_double::frozen::write_vectors from identities a (canonical-12) and b (canonical-24); crosscheck.py re-derives headers, layouts, pair keys, pair_macs and sig inputs from the spec; signatures verified in Rust under id111::keyset::verify_strict_tagged.",
        );
        f.push(("identities", identities(&a, &b)));
        f.push((
            "pair",
            obj(&[
                ("shared", W::h(&*ss)),
                ("key_a_to_b", W::h(&*pk_ab)),
                ("key_b_to_a", W::h(&*pk_ba)),
            ]),
        ));
        f.push(("cases", arr(&cs, true)));
        f.push(("reject_inner", arr(&rj, true)));
        f.push(("reject_extra", arr(&erj, true)));
        f.push(("reject_header", arr(&hdrs, true)));
        doc(&f)
    }

    // ------------------------------------------------------------ custody

    fn custody_doc() -> String {
        let (a, b) = ab();
        let mut rs = Vec::new();
        let items: [(&str, CustodyStatus, u64, &[u8], bool); 6] = [
            ("accepted", CustodyStatus::Accepted, 1, b"item-1", false),
            ("duplicate", CustodyStatus::Duplicate, 1, b"item-1", false),
            (
                "reject full",
                CustodyStatus::RejectFull,
                0,
                b"item-2",
                false,
            ),
            (
                "reject unenrolled",
                CustodyStatus::RejectUnenrolled,
                0,
                b"item-3",
                false,
            ),
            (
                "reject unknown dest",
                CustodyStatus::RejectUnknownDest,
                0,
                b"item-4",
                false,
            ),
            (
                "accepted, transferable",
                CustodyStatus::Accepted,
                300,
                b"item-5",
                true,
            ),
        ];
        for (name, status, seq, item, signed) in items {
            let rec = CustodyReceipt {
                status,
                custody_seq: seq,
                pkt_ref: pkt_ref(item),
            };
            // hub = a, dest = b
            let ca = CustodyAck::build(&a, rec, &b.node_id(), signed).unwrap();
            rs.push(obj(&[
                ("name", W::s(name)),
                ("hub", W::s("a")),
                ("dest", W::s("b")),
                ("status", (status as u8).to_string()),
                ("custody_seq", W::s(&seq.to_string())),
                ("item", W::h(item)),
                ("pkt_ref", W::h(&rec.pkt_ref)),
                (
                    "sig_input",
                    W::opt_h(
                        signed
                            .then(|| custody_sig_input(&a.node_id(), &b.node_id(), &rec))
                            .as_deref(),
                    ),
                ),
                (
                    "custody_sig",
                    W::opt_h(ca.custody_sig.as_ref().map(|s| &s[..])),
                ),
                ("payload", W::h(&ca.encode().unwrap())),
            ]));
        }
        let mut good = vec![1u8, 1];
        good.extend_from_slice(&pkt_ref(b"x"));
        let rejects: Vec<(&str, Vec<u8>)> = vec![
            ("status 0", [&[0u8, 1][..], &good[2..]].concat()),
            ("status 6", [&[6u8, 1][..], &good[2..]].concat()),
            (
                "ACCEPTED with custody_seq 0",
                [&[1u8, 0][..], &good[2..]].concat(),
            ),
            (
                "REJECT_FULL with custody_seq 1",
                [&[3u8, 1][..], &good[2..]].concat(),
            ),
            ("pkt_ref truncated", good[..17].to_vec()),
            ("10-byte tail", [&good[..], &[0u8; 10]].concat()),
            (
                "over-long custody_seq",
                [&[1u8, 0x81, 0x00][..], &good[2..]].concat(),
            ),
        ];
        let rj: Vec<String> = rejects
            .iter()
            .map(|(n, v)| {
                obj(&[
                    ("name", W::s(n)),
                    ("payload", W::h(v)),
                    ("expect", W::s(&tok(CustodyAck::decode(v).unwrap_err()))),
                ])
            })
            .collect();
        let mut f = header(
            "§6.2 custody receipt (status 1–5, custody_seq, pkt_ref) + custody-sig",
            "CUSTODY_ACK payload = status(1) ‖ custody_seq(varint) ‖ pkt_ref(16) ‖ [custody_sig(64)]; pkt_ref = BLAKE2b-128('111/custody/v1' ‖ exact sealed item bytes); custody_sig = Ed25519 over '111/custody-sig/v1' ‖ hub ‖ dest ‖ status ‖ custody_seq(varint) ‖ pkt_ref. C2 readings: custody_seq ≥ 1 for ACCEPTED/DUPLICATE, 0 for REJECT_*; the signature is the payload's last 64 B. 'expect' is the Debug form of mesh111::custody::CustodyError.",
            "Generated by mesh111 test_double::frozen::write_vectors (hub = a, dest = b); crosscheck.py re-derives pkt_refs, payloads and sig inputs; signatures verified in Rust.",
        );
        f.push(("identities", identities(&a, &b)));
        f.push(("receipts", arr(&rs, true)));
        f.push(("reject", arr(&rj, true)));
        doc(&f)
    }

    // ------------------------------------------------------------ enroll

    fn enroll_doc() -> String {
        let (a, b) = ab();
        let secret = crate::envelope::blake2b_256(&[b"c2-vector-code-secret"]);
        let mut tags = Vec::new();
        for (name, member, code) in [
            ("member a, code 7BCD9KMQ", "a", "7BCD9KMQ"),
            ("member b, same code (bound to the member)", "b", "7BCD9KMQ"),
            ("member a, code 00000000", "a", "00000000"),
        ] {
            let id = if member == "a" {
                a.node_id()
            } else {
                b.node_id()
            };
            let c = parse_code(code.as_bytes()).unwrap();
            tags.push(obj(&[
                ("name", W::s(name)),
                ("code_secret", W::h(&secret)),
                ("member", W::s(member)),
                ("code", W::s(code)),
                ("tag", W::h(&enroll_hash(&secret, &id, &c).unwrap())),
            ]));
        }
        let bad: Vec<String> = ["7BCD9KM", "7BCD9KMQX", "7BCD9KMA", "7bcd9kmq", "7BCD-KMQ"]
            .iter()
            .map(|c| {
                obj(&[
                    ("code", W::s(c)),
                    ("expect", W::s(&tok(parse_code(c.as_bytes()).unwrap_err()))),
                ])
            })
            .collect();
        let mut f = header(
            "§7.3 enrollment code keyed hash",
            "tag = keyed BLAKE2b-128(key = code_secret (32 B), data = '111/enroll/v1' ‖ member NodeId ‖ code (8 ASCII symbols of the L4 pair-code alphabet)); compared in constant time. 'reject_code' lists codes the strict parser refuses (normalization is the UI's job); 'expect' is the Debug form of mesh111::enroll::EnrollError.",
            "Generated by mesh111 test_double::frozen::write_vectors; crosscheck.py re-derives every tag with hashlib.blake2b(key=…).",
        );
        f.push(("identities", identities(&a, &b)));
        f.push(("tags", arr(&tags, true)));
        f.push(("reject_code", arr(&bad, true)));
        doc(&f)
    }

    /// Regenerate the frozen files. Run ONLY to (re)freeze with sign-off:
    /// `cargo test -p mesh111 write_vectors -- --ignored`.
    #[test]
    #[ignore]
    fn write_vectors() {
        write("ids.json", &ids_doc());
        write("ack.json", &ack_doc());
        write("envelope.json", &envelope_doc());
        write("custody.json", &custody_doc());
        write("enroll.json", &enroll_doc());
    }

    /// The files on disk are exactly what the code generates (drift gate).
    #[test]
    fn frozen_files_match_generation() {
        for (name, text) in [
            ("ids.json", ids_doc()),
            ("ack.json", ack_doc()),
            ("envelope.json", envelope_doc()),
            ("custody.json", custody_doc()),
            ("enroll.json", enroll_doc()),
        ] {
            let disk = std::fs::read_to_string(vectors_dir().join(name)).unwrap();
            assert!(
                disk == text,
                "{name}: implementation drifted from the frozen vector"
            );
        }
    }

    // ------------------------------------------------------------ loaders

    #[test]
    fn ids_vectors() {
        let v = load("ids.json");
        for x in v.get("varints").a() {
            let mut b = Vec::new();
            put_varint(&mut b, x.get("value").dec());
            assert_eq!(hex(&b), x.get("hex").s());
            assert_eq!(decode_varint(&b).unwrap(), (x.get("value").dec(), b.len()));
        }
        for x in v.get("msg_seq").a() {
            let s = MsgSeq::new(x.get("epoch").dec(), x.get("n").n() as u32);
            assert_eq!(hex(&s.to_bytes()), x.get("hex").s());
            assert_eq!(
                MsgSeq::decode(&mut Reader::new(&x.get("hex").hex())).unwrap(),
                s
            );
        }
        for x in v.get("varint_reject").a() {
            assert_eq!(
                tok(decode_varint(&x.get("hex").hex()).unwrap_err()),
                x.get("expect").s()
            );
        }
        for x in v.get("msg_seq_reject").a() {
            let b = x.get("hex").hex();
            assert_eq!(
                tok(MsgSeq::decode(&mut Reader::new(&b)).unwrap_err()),
                x.get("expect").s()
            );
        }
    }

    fn id_of(v: &J, who: &str) -> NodeId {
        NodeId::from(v.get("identities").get(who).get("node_id").h32())
    }

    #[test]
    fn ack_vectors() {
        let v = load("ack.json");
        for x in v.get("blocks").a() {
            let b = block_from(x);
            assert_eq!(
                hex(&b.encode().unwrap()),
                x.get("hex").s(),
                "{}",
                x.get("name").s()
            );
            assert_eq!(AckBlock::decode_exact(&x.get("hex").hex()).unwrap(), b);
        }
        for x in v.get("payloads").a() {
            let acker = id_of(&v, x.get("acker").s());
            let sender = id_of(&v, x.get("sender").s());
            let p = AckPayload::decode(&x.get("payload").hex()).unwrap();
            assert_eq!(hex(&p.block_bytes), x.get("block").s());
            assert_eq!(
                hex(&ack_sig_input(
                    &acker,
                    &sender,
                    x.get("stream").dec(),
                    &p.block_bytes
                )),
                x.get("sig_input").s()
            );
            let signed = !x.get("ack_sig").is_null();
            assert_eq!(p.verify(&acker, &sender, 0).unwrap(), signed);
            if signed {
                // bound to acker, sender and stream
                assert!(p.verify(&sender, &acker, 0).is_err());
                assert!(p.verify(&acker, &sender, 1).is_err());
            }
        }
        for x in v.get("strong_hashes").a() {
            assert_eq!(
                hex(&strong_hash(&x.get("payload").hex())),
                x.get("hash").s()
            );
        }
        for x in v.get("reject").a() {
            assert_eq!(
                tok(AckBlock::decode_exact(&x.get("hex").hex()).unwrap_err()),
                x.get("expect").s(),
                "{}",
                x.get("name").s()
            );
        }
        for x in v.get("reject_payload").a() {
            assert_eq!(
                tok(AckPayload::decode(&x.get("hex").hex()).unwrap_err()),
                x.get("expect").s()
            );
        }
    }

    fn inner_from(x: &J, sender: &NodeId) -> Inner {
        let kind = Kind::from_byte(x.get("kind").n() as u8).unwrap();
        let mut i = Inner::new(
            sender,
            kind,
            MsgSeq::new(x.get("epoch").dec(), x.get("n").n() as u32),
            x.get("payload").hex(),
        );
        i.signed = x.get("signed").b();
        i.receipt_strong = x.get("receipt_strong").b();
        if !x.get("frag").is_null() {
            let f = x.get("frag").a();
            i.frag = Some(FragPos {
                idx: f[0].n() as u8,
                count: f[1].n() as u8,
            });
        }
        if !x.get("seen_peer_seq").is_null() {
            let s = x.get("seen_peer_seq").a();
            i.ext.seen_peer_seq = Some(MsgSeq::new(s[0].dec(), s[1].n() as u32));
        }
        for t in x.get("unknown_tlvs").a() {
            i.ext.unknown.push(RawTlv {
                tlv_type: t.a()[0].n() as u8,
                value: t.a()[1].hex(),
            });
        }
        if !x.get("ack").is_null() {
            i.ack = Some(AckBlock::decode_exact(&x.get("ack").hex()).unwrap());
        }
        if !x.get("card").is_null() {
            i.card = Some(x.get("card").hex());
        }
        i
    }

    #[test]
    fn envelope_vectors() {
        let v = load("envelope.json");
        let (a, b) = ab();
        assert_eq!(id_of(&v, "a"), a.node_id());
        assert_eq!(id_of(&v, "b"), b.node_id());
        let pair = v.get("pair");
        let ss = a.keyset().ecdh(&b.enc_pubkey()).unwrap();
        assert_eq!(hex(&*ss), pair.get("shared").s());
        let pk_ab = derive_pair_key(&ss, &a.enc_pubkey(), &b.enc_pubkey());
        assert_eq!(hex(&*pk_ab), pair.get("key_a_to_b").s());
        let pk_ba = derive_pair_key(&ss, &b.enc_pubkey(), &a.enc_pubkey());
        assert_eq!(hex(&*pk_ba), pair.get("key_b_to_a").s());
        for x in v.get("cases").a() {
            let name = x.get("name").s();
            let from_a = x.get("sender").s() == "a";
            let (snd, rcv, key) = if from_a {
                (&a, &b, &pk_ab)
            } else {
                (&b, &a, &pk_ba)
            };
            let inner = inner_from(x, &snd.node_id());
            let prio = match x.get("priority").n() {
                0 => Priority::Control,
                1 => Priority::Interactive,
                2 => Priority::BulkSmall,
                _ => Priority::BulkLarge,
            };
            let header = OuterExtra::to(prio, &rcv.node_id()).header(x.get("counter").dec());
            let hb = header_bytes(&header).unwrap();
            assert_eq!(hex(&hb), x.get("header").s(), "{name} header");
            let before = inner.encode_before_auth().unwrap();
            assert_eq!(
                hex(&before),
                x.get("inner_before_auth").s(),
                "{name} before_auth"
            );
            assert_eq!(
                Inner::decode_before_auth(&before).unwrap(),
                inner,
                "{name} decode"
            );
            let epk = x.get("epk").h32();
            let mac = x.get("pair_mac").h16();
            assert_eq!(
                pair_mac(key, &hb, &epk, &before).unwrap(),
                mac,
                "{name} pair_mac"
            );
            assert!(verify_pair_mac(key, &hb, &epk, &before, &mac));
            // direction-bound: the reverse key never verifies
            let rev = if from_a { &pk_ba } else { &pk_ab };
            assert!(
                !verify_pair_mac(rev, &hb, &epk, &before, &mac),
                "{name} reflection"
            );
            let full = x.get("inner").hex();
            let (b_auth, auth, _) = crate::envelope::split_auth(&full).unwrap();
            assert_eq!(b_auth, &before[..]);
            assert_eq!(auth.pair_mac, mac);
            if inner.signed {
                let si = x.get("sig_input").hex();
                assert_eq!(
                    si,
                    sig_input(
                        &snd.node_id(),
                        &sig_dest(inner.kind, &rcv.node_id()),
                        0,
                        &inner
                    ),
                    "{name} sig_input"
                );
                let sig = x.get("sig").h64();
                assert_eq!(auth.sig, Some(sig));
                assert!(verify_strict_tagged(&snd.node_id(), MSG_SIG_TAG, &si, &sig).is_ok());
                assert_eq!(snd.sign(MSG_SIG_TAG, &si), sig, "{name} deterministic sig");
            } else {
                assert!(x.get("sig").is_null() && auth.sig.is_none());
            }
            if !x.get("budget").is_null() {
                let bud = x.get("budget");
                let plen = hb.len() + 32 + full.len() + 16;
                assert_eq!(plen as u64, bud.get("packet_len").n());
                assert_eq!(
                    plen + usize::from(bud.get("shim").b()),
                    bud.get("bearer").n() as usize
                );
            }
        }
        for x in v.get("reject_inner").a() {
            assert_eq!(
                tok(Inner::decode_before_auth(&x.get("before_auth").hex()).unwrap_err()),
                x.get("expect").s(),
                "{}",
                x.get("name").s()
            );
        }
        for x in v.get("reject_extra").a() {
            assert_eq!(
                tok(OuterExtra::decode(&x.get("extra").hex()).unwrap_err()),
                x.get("expect").s()
            );
        }
        for x in v.get("reject_header").a() {
            let h = x.get("header").hex();
            let hdr = id111::seal::SealHeader {
                version: h[0],
                crypto_mode: h[1],
                counter: u64::from_le_bytes(h[2..10].try_into().unwrap()),
                extra: h[11..].to_vec(),
            };
            assert_eq!(
                tok(crate::envelope::check_header(&hdr).unwrap_err()),
                x.get("expect").s()
            );
        }
    }

    #[test]
    fn custody_vectors() {
        let v = load("custody.json");
        for x in v.get("receipts").a() {
            let hub = id_of(&v, x.get("hub").s());
            let dest = id_of(&v, x.get("dest").s());
            let ca = CustodyAck::decode(&x.get("payload").hex()).unwrap();
            assert_eq!(hex(&pkt_ref(&x.get("item").hex())), x.get("pkt_ref").s());
            assert_eq!(hex(&ca.receipt.pkt_ref), x.get("pkt_ref").s());
            assert_eq!(ca.receipt.status as u64, x.get("status").n());
            assert_eq!(ca.receipt.custody_seq, x.get("custody_seq").dec());
            assert_eq!(hex(&ca.encode().unwrap()), x.get("payload").s());
            let signed = !x.get("custody_sig").is_null();
            assert_eq!(ca.verify(&hub, &dest).unwrap(), signed);
            if signed {
                assert_eq!(
                    hex(&custody_sig_input(&hub, &dest, &ca.receipt)),
                    x.get("sig_input").s()
                );
                assert!(ca.verify(&dest, &hub).is_err());
            }
        }
        for x in v.get("reject").a() {
            assert_eq!(
                tok(CustodyAck::decode(&x.get("payload").hex()).unwrap_err()),
                x.get("expect").s(),
                "{}",
                x.get("name").s()
            );
        }
    }

    #[test]
    fn enroll_vectors() {
        let v = load("enroll.json");
        for x in v.get("tags").a() {
            let member = id_of(&v, x.get("member").s());
            let secret = x.get("code_secret").h32();
            let code = parse_code(x.get("code").s().as_bytes()).unwrap();
            let tag = x.get("tag").h16();
            assert_eq!(enroll_hash(&secret, &member, &code).unwrap(), tag);
            assert!(verify_enroll_hash(&secret, &member, &code, &tag));
            // the hub-side flow: the offered code accepted once, a wrong
            // code counted toward lockout
            let mut offer = Offer::new(code, secret);
            assert_eq!(
                offer.present(&member, b"ZZZZZZZZ"),
                Err(EnrollError::Mismatch { failures_left: 4 })
            );
            assert_eq!(offer.present(&member, &code), Ok(Presented::Enrolled));
            // a retransmitted ENROLL from the same member: ENROLL_OK again
            assert_eq!(
                offer.present(&member, &code),
                Ok(Presented::AlreadyEnrolled)
            );
            let other = TestSealer::from_label("not-the-member").node_id_value();
            assert_eq!(offer.present(&other, &code), Err(EnrollError::Spent));
        }
        for x in v.get("reject_code").a() {
            assert_eq!(
                tok(parse_code(x.get("code").s().as_bytes()).unwrap_err()),
                x.get("expect").s()
            );
        }
    }
}
