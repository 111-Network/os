//! §8.1–§8.4 — backup & restore: the versioned self-describing backup format,
//! ceremony artifacts, verify-on-write-down support, and the words/QR restore.
//!
//! Implements (owner: T5):
//!
//! - §8.2: the 4-byte descriptor `version(0x01) ‖ curve(0x01 =
//!   ed25519+x25519 dual) ‖ derivation(0x01 = SLIP-0010 m/111'/0'/{0',1'}) ‖
//!   encoding(0x01 = BIP-39 English 12 words · 0x02 = 24 words)`. Restore
//!   MUST honor the descriptor and MUST fail closed on any unknown field; a
//!   v1 restore with no descriptor present assumes the v1 defaults (the
//!   encoding byte is then taken from the actual word count — 12↔0x01,
//!   24↔0x02 — which is self-evident from the parsed words).
//! - §8.3: ceremony artifacts on the commissioning device — the words plus a
//!   single-frame CompactSeedQR-style project QR payload (4B descriptor ‖
//!   raw entropy, 20–36B; bytes only — QR rendering is the CLI's job) and the
//!   verify-on-write-down gate support (sampled-word positions + check, QR
//!   re-scan check). No passphrase; paper default, metal for hubs.
//! - §8.4: restore — NFKC + case/whitespace-folded words (or a QR scan), the
//!   BIP-39 checksum MUST validate before derivation (`RootMnemonic::parse`),
//!   descriptor check (fail closed), SLIP-0010 derivation (§2, in
//!   `keyset.rs`), atomic §13 identity-dir write (never overwrite — MUST
//!   refuse against a live, unquarantined identity dir), and the restored
//!   NodeId returned as the operator-confirmation surface (the confirmation
//!   MUST itself is the CLI ceremony's). Counter semantics: state present
//!   (a live dir image restored at the file level) ⇒ persisted counters are
//!   reused as-is and no code path here applies; state absent (words/QR
//!   disaster recovery) ⇒ the announce counter starts at `floor + 1024` per
//!   §8.4 — see [`CounterFloor`] and the [`initial_counters_file`] call
//!   boundary (the counters state machine is the sibling `counters`
//!   module's, T4).
//!
//! The TTY-only display/refusal rules of §8.3 (backup MUST refuse non-TTY
//! stdout) and the §8.4 old-instance-retirement ceremony prompt are enforced
//! by the CLI wiring (node111), not here — the library refuses unless the
//! caller asserts the confirmation ([`RestoreOptions::old_instance_retired`]).

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::bip39::{Bip39Error, RootMnemonic, WordCount};
use crate::counters::{CounterError, CounterStore};
use crate::encoding::{self, KeyKind, NodeIdDisplay};
use crate::keyset::{Keyset, KeysetError, NodeId};

// ---------------------------------------------------------------------------
// §13 — identity-dir file names (the five-file set this module writes)
// ---------------------------------------------------------------------------

/// §13: BIP-39 words, NFKC UTF-8, space-separated, trailing newline — 0600.
pub const MNEMONIC_FILE: &str = "mnemonic";
/// §13: the 64B private keyset (derived cache, regenerable) — 0600.
pub const KEYSET_FILE: &str = "keyset";
/// §13: the 34B NodeId record (`0xed 0x01` ‖ 32B pubkey, §7) — 0644.
pub const NODEID_FILE: &str = "nodeid";
/// §13: the 4B §8.2 descriptor — 0644.
pub const DESCRIPTOR_FILE: &str = "descriptor";
/// §13: announce counter + per-peer traffic counters/windows — 0600.
/// Format owner: the sibling `counters` module (T4) — see
/// [`initial_counters_file`] for the call boundary used here.
pub const COUNTERS_FILE: &str = "counters";

/// Every filename that makes an identity dir "live" for the §8.4 never-
/// overwrite refusal: the §13 five plus the two D9-legacy-only files (a dir
/// holding legacy material is a live provisional identity — migrate, don't
/// restore over it; §8.4/D14d).
const IDENTITY_FILE_NAMES: [&str; 7] = [
    MNEMONIC_FILE,
    KEYSET_FILE,
    NODEID_FILE,
    DESCRIPTOR_FILE,
    COUNTERS_FILE,
    crate::migrate::LEGACY_SECRET_FILE,
    crate::migrate::LEGACY_NPUB_FILE,
];

// ---------------------------------------------------------------------------
// §8.2 — the 4-byte backup descriptor
// ---------------------------------------------------------------------------

/// Descriptor length (§8.2, normative): 4 bytes.
pub const DESCRIPTOR_LEN: usize = 4;

/// The only version byte v1 knows (§8.2): `0x01`.
pub const VERSION_V1: u8 = 0x01;

/// §8.2 `curve` field — the only value v1 knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveKind {
    /// `0x01` = ed25519+x25519 dual keyset (§2, D13 A1).
    Ed25519X25519Dual,
}

impl CurveKind {
    const BYTE: u8 = 0x01;
    fn from_byte(b: u8) -> Result<Self, DescriptorError> {
        match b {
            0x01 => Ok(CurveKind::Ed25519X25519Dual),
            other => Err(DescriptorError::UnknownCurve(other)),
        }
    }
}

/// §8.2 `derivation` field — the only value v1 knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DerivationKind {
    /// `0x01` = SLIP-0010 hardened-only at the pinned paths
    /// `m/111'/0'/{0',1'}` (§2). NOT BIP32-Ed25519/Cardano.
    Slip0010PinnedPaths,
}

impl DerivationKind {
    const BYTE: u8 = 0x01;
    fn from_byte(b: u8) -> Result<Self, DescriptorError> {
        match b {
            0x01 => Ok(DerivationKind::Slip0010PinnedPaths),
            other => Err(DescriptorError::UnknownDerivation(other)),
        }
    }
}

/// §8.2 `encoding` field — both v1 values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodingKind {
    /// `0x01` = BIP-39 English, 12 words (128-bit) — the fleet default (§8.1).
    Bip39English12,
    /// `0x02` = BIP-39 English, 24 words (256-bit) — hubs (Cat 4 SHOULD, §8.1).
    Bip39English24,
}

impl EncodingKind {
    fn to_byte(self) -> u8 {
        match self {
            EncodingKind::Bip39English12 => 0x01,
            EncodingKind::Bip39English24 => 0x02,
        }
    }
    fn from_byte(b: u8) -> Result<Self, DescriptorError> {
        match b {
            0x01 => Ok(EncodingKind::Bip39English12),
            0x02 => Ok(EncodingKind::Bip39English24),
            other => Err(DescriptorError::UnknownEncoding(other)),
        }
    }
    /// The §8.1 word-count profile this encoding byte names.
    pub fn word_count(self) -> WordCount {
        match self {
            EncodingKind::Bip39English12 => WordCount::Twelve,
            EncodingKind::Bip39English24 => WordCount::TwentyFour,
        }
    }
    /// The encoding byte for a §8.1 word-count profile.
    pub fn for_word_count(wc: WordCount) -> Self {
        match wc {
            WordCount::Twelve => EncodingKind::Bip39English12,
            WordCount::TwentyFour => EncodingKind::Bip39English24,
        }
    }
}

/// §8.2 — the versioned, self-describing 4-byte backup descriptor:
/// `version ‖ curve ‖ derivation ‖ encoding`. Every backup artifact carries
/// it: printed under the words on paper, as the QR payload header (§8.3), and
/// as the `descriptor` file (0644, 4B) in the identity dir (§13).
///
/// Parse fails closed on ANY unknown field (§8.2 MUST, §12 rule 1); an absent
/// descriptor implies the v1 defaults ([`BackupDescriptor::for_word_count`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackupDescriptor {
    version: u8,
    curve: CurveKind,
    derivation: DerivationKind,
    encoding: EncodingKind,
}

impl BackupDescriptor {
    /// The v1 descriptor for 12-word backups: `01 01 01 01`.
    pub const V1_12_WORDS: Self = BackupDescriptor {
        version: VERSION_V1,
        curve: CurveKind::Ed25519X25519Dual,
        derivation: DerivationKind::Slip0010PinnedPaths,
        encoding: EncodingKind::Bip39English12,
    };

    /// The v1 descriptor for 24-word backups: `01 01 01 02`.
    pub const V1_24_WORDS: Self = BackupDescriptor {
        version: VERSION_V1,
        curve: CurveKind::Ed25519X25519Dual,
        derivation: DerivationKind::Slip0010PinnedPaths,
        encoding: EncodingKind::Bip39English24,
    };

    /// The v1 descriptor for a §8.1 word-count profile (the "v1 defaults" an
    /// absent descriptor implies — §8.2).
    pub fn for_word_count(wc: WordCount) -> Self {
        match EncodingKind::for_word_count(wc) {
            EncodingKind::Bip39English12 => Self::V1_12_WORDS,
            EncodingKind::Bip39English24 => Self::V1_24_WORDS,
        }
    }

    /// The descriptor for a parsed root mnemonic (word count → encoding byte).
    pub fn for_mnemonic(mnemonic: &RootMnemonic) -> Self {
        let wc = match mnemonic.word_count() {
            12 => WordCount::Twelve,
            24 => WordCount::TwentyFour,
            // RootMnemonic::parse holds the 12/24 profile (§8.1).
            _ => unreachable!("RootMnemonic holds only the 12/24-word v1 profile"),
        };
        Self::for_word_count(wc)
    }

    /// The 4-byte wire/disk form: `version ‖ curve ‖ derivation ‖ encoding`.
    pub fn to_bytes(self) -> [u8; DESCRIPTOR_LEN] {
        [
            self.version,
            match self.curve {
                CurveKind::Ed25519X25519Dual => CurveKind::BYTE,
            },
            match self.derivation {
                DerivationKind::Slip0010PinnedPaths => DerivationKind::BYTE,
            },
            self.encoding.to_byte(),
        ]
    }

    /// §8.2 parse — **fail closed on any unknown field** (MUST). Wrong length
    /// is equally fatal. No field is ever guessed or defaulted here; the
    /// absent-descriptor fallback lives at the restore call site.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DescriptorError> {
        if bytes.len() != DESCRIPTOR_LEN {
            return Err(DescriptorError::BadLength(bytes.len()));
        }
        let version = match bytes[0] {
            VERSION_V1 => VERSION_V1,
            other => return Err(DescriptorError::UnknownVersion(other)),
        };
        Ok(BackupDescriptor {
            version,
            curve: CurveKind::from_byte(bytes[1])?,
            derivation: DerivationKind::from_byte(bytes[2])?,
            encoding: EncodingKind::from_byte(bytes[3])?,
        })
    }

    /// The §8.1 word-count profile this descriptor pins.
    pub fn word_count(self) -> WordCount {
        self.encoding.word_count()
    }

    /// The version byte (0x01 in v1; parse rejects anything else).
    pub fn version(self) -> u8 {
        self.version
    }
}

impl fmt::Display for BackupDescriptor {
    /// Lowercase hex of the 4 bytes — the form printed under the words on
    /// paper (§8.2). Not secret.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.to_bytes() {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

/// §8.2 descriptor failures — every variant is a fail-closed rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorError {
    /// The descriptor is not exactly 4 bytes (§8.2).
    BadLength(usize),
    /// Unknown `version` byte (v1 knows only 0x01) — fail closed.
    UnknownVersion(u8),
    /// Unknown `curve` byte (v1 knows only 0x01) — fail closed.
    UnknownCurve(u8),
    /// Unknown `derivation` byte (v1 knows only 0x01) — fail closed.
    UnknownDerivation(u8),
    /// Unknown `encoding` byte (v1 knows only 0x01/0x02) — fail closed.
    UnknownEncoding(u8),
}

impl fmt::Display for DescriptorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DescriptorError::BadLength(n) => {
                write!(f, "§8.2: descriptor must be 4 bytes, got {n}")
            }
            DescriptorError::UnknownVersion(b) => {
                write!(f, "§8.2: unknown descriptor version byte {b:#04x} (fail closed)")
            }
            DescriptorError::UnknownCurve(b) => {
                write!(f, "§8.2: unknown descriptor curve byte {b:#04x} (fail closed)")
            }
            DescriptorError::UnknownDerivation(b) => {
                write!(f, "§8.2: unknown descriptor derivation byte {b:#04x} (fail closed)")
            }
            DescriptorError::UnknownEncoding(b) => {
                write!(f, "§8.2: unknown descriptor encoding byte {b:#04x} (fail closed)")
            }
        }
    }
}

impl std::error::Error for DescriptorError {}

impl From<DescriptorError> for BackupError {
    fn from(e: DescriptorError) -> Self {
        BackupError::Descriptor(e)
    }
}

// ---------------------------------------------------------------------------
// §8.3 — ceremony artifacts: the words + the project QR payload
// ---------------------------------------------------------------------------

/// §8.3 step 2 — everything the ceremony displays: the words (paper default,
/// metal for hubs) and the single-frame CompactSeedQR-style project QR
/// payload (**bytes only** — rendering is the CLI's job). The payload is the
/// 4B §8.2 descriptor ‖ the raw entropy (20B at 12 words, 36B at 24).
///
/// Both bodies are secret (the entropy IS the root secret); they zeroize on
/// drop (§12: the mnemonic is the master of both keys). The descriptor is
/// public — it is printed under the words on the backup itself.
pub struct BackupArtifacts {
    /// The §8.2 descriptor — printed under the words, heads the QR payload.
    pub descriptor: BackupDescriptor,
    words: Zeroizing<String>,
    qr_payload: Zeroizing<Vec<u8>>,
}

impl BackupArtifacts {
    /// §8.3 step 2 — compose the ceremony artifacts for a root mnemonic.
    pub fn new(mnemonic: &RootMnemonic) -> Self {
        let descriptor = BackupDescriptor::for_mnemonic(mnemonic);
        let entropy = mnemonic.entropy();
        let mut qr = Zeroizing::new(Vec::with_capacity(DESCRIPTOR_LEN + entropy.len()));
        qr.extend_from_slice(&descriptor.to_bytes());
        qr.extend_from_slice(&entropy);
        BackupArtifacts {
            descriptor,
            words: Zeroizing::new(mnemonic.phrase()),
            qr_payload: qr,
        }
    }

    /// The canonical space-joined words (secret — display only at the §8.3
    /// local-TTY ceremony; the TTY refusal is the CLI's, §8.3).
    pub fn words(&self) -> &str {
        &self.words
    }

    /// The project QR payload: 4B descriptor ‖ raw entropy, 20–36B (§8.3
    /// step 2; one static frame, QR version 2–3 at EC level L — rendering is
    /// the CLI's job). A project variant of SeedSigner's CompactSeedQR;
    /// raw-entropy-only tooling will NOT parse it (§8.3, named accordingly).
    pub fn qr_payload(&self) -> &[u8] {
        &self.qr_payload
    }
}

impl fmt::Debug for BackupArtifacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // §5/§12: words and entropy are secret — never Debug-print them.
        write!(f, "BackupArtifacts(descriptor {}, redacted)", self.descriptor)
    }
}

/// §8.4/§8.3 — parse a scanned project QR payload (4B descriptor ‖ raw
/// entropy). Fail-closed throughout: the descriptor parse rejects unknown
/// fields, the length must be exactly 20 or 36, and the descriptor's
/// `encoding` byte MUST agree with the entropy length (0x01 ↔ 16B,
/// 0x02 ↔ 32B) — a contradictory payload is rejected, never interpreted.
///
/// Returns the descriptor and the raw entropy (zeroizing). The entropy feeds
/// `RootMnemonic::from_entropy` — checksum-valid by construction; the BIP-39
/// checksum gate applies to typed words, and the QR's raw entropy is the
/// pre-checksum form (§8.3).
pub fn parse_qr_payload(
    payload: &[u8],
) -> Result<(BackupDescriptor, Zeroizing<Vec<u8>>), BackupError> {
    let entropy_len = match payload.len() {
        20 => 16,
        36 => 32,
        n => return Err(BackupError::QrPayloadLength(n)),
    };
    let descriptor = BackupDescriptor::from_bytes(&payload[..DESCRIPTOR_LEN])?;
    if descriptor.word_count().entropy_bytes() != entropy_len {
        return Err(BackupError::QrEncodingMismatch);
    }
    Ok((
        descriptor,
        Zeroizing::new(payload[DESCRIPTOR_LEN..].to_vec()),
    ))
}

/// §8.3 step 3 — the QR re-scan half of verify-on-write-down: true iff the
/// re-scanned payload parses, its descriptor matches this mnemonic's, and its
/// entropy IS this mnemonic's entropy. Any failure is a plain `false` (this
/// is a check, not a diagnostic path).
pub fn qr_payload_matches(mnemonic: &RootMnemonic, payload: &[u8]) -> bool {
    match parse_qr_payload(payload) {
        Ok((descriptor, entropy)) => {
            descriptor == BackupDescriptor::for_mnemonic(mnemonic)
                && *entropy == *mnemonic.entropy()
        }
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// §8.3 step 3 — verify-on-write-down: sampled-word positions + check
// ---------------------------------------------------------------------------

/// Suggested number of sampled words for the §8.3 step-3 re-entry gate
/// (Codex32-inspired; the count itself is a CLI/UX choice — the spec pins the
/// gate, not the sample size). The backup is not considered made until
/// verification passes.
pub const DEFAULT_SAMPLE_COUNT: usize = 4;

/// Uniform unbiased draw from `0..n` via rejection sampling over a 64-bit
/// §4.2 `getrandom()` read. No silent weak fallback — an entropy failure
/// aborts the ceremony loudly (§4 rule 2).
fn random_below(n: usize) -> Result<usize, BackupError> {
    debug_assert!(n > 0);
    let n64 = n as u64;
    let zone = u64::MAX - (u64::MAX % n64); // largest multiple of n ≤ u64::MAX
    let mut buf = [0u8; 8];
    loop {
        getrandom::getrandom(&mut buf).map_err(BackupError::Entropy)?;
        let x = u64::from_le_bytes(buf);
        if x < zone {
            return Ok((x % n64) as usize);
        }
    }
}

/// §8.3 step 3 — draw `sample` unique word positions (0-based, ascending) for
/// the write-down verification. Selection is `getrandom()`-drawn (§4.2) with
/// a partial Fisher–Yates shuffle; display numbering (1-based "word #n") is
/// the CLI's job. `sample` must be in `1..=word_count`.
pub fn sample_positions(word_count: WordCount, sample: usize) -> Result<Vec<usize>, BackupError> {
    let total = word_count.word_count();
    if sample == 0 || sample > total {
        return Err(BackupError::SampleCountOutOfRange {
            requested: sample,
            words: total,
        });
    }
    let mut pool: Vec<usize> = (0..total).collect();
    let mut out = Vec::with_capacity(sample);
    for i in 0..sample {
        let j = i + random_below(total - i)?;
        pool.swap(i, j);
        out.push(pool[i]);
    }
    out.sort_unstable();
    Ok(out)
}

/// §8.3 step 3 — check the typed-back answers against the mnemonic's words at
/// `positions` (0-based, as returned by [`sample_positions`]). Answers get the
/// §8.4 input hygiene (trimmed, case-folded, NFKC-normalized) before
/// comparison. `true` only if every sampled word matches; the backup is not
/// considered made until this passes (§8.3).
///
/// Errors are caller bugs (position out of range, answer count mismatch) —
/// a wrong word is `Ok(false)`, never an error.
pub fn check_words(
    mnemonic: &RootMnemonic,
    positions: &[usize],
    answers: &[&str],
) -> Result<bool, BackupError> {
    if positions.len() != answers.len() {
        return Err(BackupError::AnswerCountMismatch {
            expected: positions.len(),
            got: answers.len(),
        });
    }
    let phrase = mnemonic.phrase();
    let words: Vec<&str> = phrase.split(' ').collect();
    for (&pos, answer) in positions.iter().zip(answers.iter()) {
        let expected = words
            .get(pos)
            .ok_or(BackupError::PositionOutOfRange {
                position: pos,
                words: words.len(),
            })?;
        // §8.4 hygiene on the typed answer: trim, case-fold, NFKC.
        let folded = answer.trim().to_lowercase();
        let mut cow = std::borrow::Cow::Borrowed(folded.as_str());
        ::bip39::Mnemonic::normalize_utf8_cow(&mut cow);
        if cow.as_ref() != *expected {
            return Ok(false);
        }
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// §8.4 — restore
// ---------------------------------------------------------------------------

/// §8.4 state-absent restore jump (normative): on a words/QR-only recovery
/// the announce counter is set to `floor + 1024` (safety margin; over-jumping
/// is harmless — gaps only advance windows, §6.5).
pub const RESTORE_COUNTER_JUMP: u64 = 1024;

/// §8.4 — the counter floor for a state-absent (words/QR) restore. The
/// mechanics of the counter state machine live in the sibling `counters`
/// module (T4); this type names the three floor sources the spec orders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterFloor {
    /// §8.4: a never-before-deployed identity — floor 0. The restored
    /// announce counter starts at `0 + RESTORE_COUNTER_JUMP`.
    NeverDeployed,
    /// §8.4: a known floor — operator-supplied, or learned from mesh-gossiped
    /// copies of the node's own prior announces by the wiring layer before
    /// restore. The restored announce counter starts at
    /// `floor + RESTORE_COUNTER_JUMP`.
    Known(u64),
    /// §8.4: the identity may have been deployed but no floor is known at
    /// restore time — it must be learned from mesh gossip (where a mesh
    /// exists) before the node announces. A provisional `0 + JUMP` floor is
    /// persisted (a harmless lower bound — over-jumping only advances
    /// windows), and the outcome flags `counter_floor_pending`: the node
    /// MUST NOT announce until the wiring layer resolves and commits the
    /// learned floor through the `counters` module (call boundary, T4).
    PendingMeshLearn,
}

impl CounterFloor {
    /// The persisted initial announce counter (`floor + 1024`, §8.4).
    fn initial_announce_counter(self) -> u64 {
        let floor = match self {
            CounterFloor::NeverDeployed | CounterFloor::PendingMeshLearn => 0,
            CounterFloor::Known(floor) => floor,
        };
        floor + RESTORE_COUNTER_JUMP
    }
}

/// §8.4 restore parameters. No `Default`: both the old-instance-retirement
/// confirmation and the counter floor are conscious ceremony choices.
pub struct RestoreOptions {
    /// §8.4 (normative): the ceremony MUST instruct retiring/quarantining the
    /// old instance and MUST refuse to proceed until the operator confirms —
    /// two live instances on one keyset produce diverging counters and
    /// flapping announces fleet-wide (§6.2's identity-collision state is the
    /// backstop, not the remedy). Restore refuses unless this is `true`.
    pub old_instance_retired: bool,
    /// The §8.2 descriptor bytes from the backup artifact (printed under the
    /// words / read off paper), when the operator has them. Honored and
    /// fail-closed when present; absent ⇒ v1 defaults (§8.2). For QR restores
    /// the payload's own descriptor header is authoritative and a supplied
    /// descriptor must EQUAL it, else fail closed.
    pub descriptor: Option<[u8; DESCRIPTOR_LEN]>,
    /// §8.4 state-absent counter floor (see [`CounterFloor`]).
    pub counter_floor: CounterFloor,
}

/// §8.4 restore result — the NodeId confirmation surface plus the counter
/// state the restore committed.
#[derive(Debug)]
pub struct RestoreOutcome {
    /// §8.4 (normative): the restored NodeId is displayed and the operator
    /// MUST confirm it against a previously recorded reference when one
    /// exists (label or bech32, §7 — the 4-bit BIP-39 checksum weakness of
    /// §8.1 is why this rule exists). The confirmation prompt is the CLI
    /// ceremony's; this is the value it displays.
    pub node_id: NodeId,
    /// The descriptor as parsed/honored and written to the dir (§8.2).
    pub descriptor: BackupDescriptor,
    /// The initial announce counter persisted in the `counters` file:
    /// `floor + RESTORE_COUNTER_JUMP` (§8.4 state-absent rule).
    pub announce_counter_init: u64,
    /// True when the floor was [`CounterFloor::PendingMeshLearn`] — the node
    /// MUST NOT announce until the wiring layer learns the floor from mesh
    /// gossip and commits it via the `counters` module (T4 call boundary).
    pub counter_floor_pending: bool,
}

impl RestoreOutcome {
    /// The §7 presentation surface (hex / bech32 `node1…` / operator label)
    /// for the §8.4 NodeId confirmation MUST.
    pub fn display(&self) -> NodeIdDisplay {
        NodeIdDisplay::new(self.node_id.as_bytes())
    }
}

/// §8.4 restore failures. Every refusal is loud; nothing is ever silently
/// overwritten or re-keyed (§13 boot integrity carried).
#[derive(Debug)]
pub enum RestoreError {
    /// §8.4: the operator has not confirmed retiring/quarantining the old
    /// instance — the ceremony MUST refuse until confirmed.
    OldInstanceNotRetired,
    /// §8.4: the target identity dir is live and unquarantined (it holds
    /// identity or legacy files) — restore MUST refuse; never overwrite.
    /// Quarantine it first (§13) or migrate it (§8.4/D14d).
    LiveIdentityDir,
    /// §8.4 input hygiene rejected the words (NFKC/case/whitespace folded;
    /// the BIP-39 checksum MUST validate before derivation — it did not, or
    /// the phrase is out of the 12/24-word v1 profile).
    Mnemonic(Bip39Error),
    /// §8.2: a supplied descriptor failed to parse (fail closed on any
    /// unknown field or wrong length).
    Descriptor(DescriptorError),
    /// §8.2 honored: the supplied descriptor's `encoding` byte contradicts
    /// the parsed word count (e.g. 0x01=12-word descriptor over 24 words) —
    /// fail closed, never reinterpret.
    DescriptorWordCountMismatch {
        /// What the descriptor's encoding byte names.
        descriptor: &'static str,
        /// The parsed phrase's actual word count.
        words: usize,
    },
    /// A QR payload and a separately supplied descriptor disagree — fail
    /// closed (the scanned entropy is never reinterpreted).
    DescriptorConflict,
    /// The QR payload itself is malformed (length, descriptor, or
    /// encoding/entropy agreement).
    QrPayload(String),
    /// Keyset derivation or the §4.5 self-test failed.
    Keyset(KeysetError),
    /// Filesystem failure during the atomic write (§13).
    Io(io::Error),
}

impl fmt::Display for RestoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RestoreError::OldInstanceNotRetired => write!(
                f,
                "§8.4: old instance not confirmed retired/quarantined — restore refuses \
                 (two live instances on one keyset flap announces fleet-wide)"
            ),
            RestoreError::LiveIdentityDir => write!(
                f,
                "§8.4: live, unquarantined identity dir — restore refuses; never overwrite \
                 (quarantine per §13, or migrate per §8.4/D14d)"
            ),
            RestoreError::Mnemonic(e) => write!(f, "{e}"),
            RestoreError::Descriptor(e) => write!(f, "{e}"),
            RestoreError::DescriptorWordCountMismatch { descriptor, words } => write!(
                f,
                "§8.2: descriptor encoding ({descriptor}) contradicts the parsed \
                 {words}-word phrase — fail closed"
            ),
            RestoreError::DescriptorConflict => write!(
                f,
                "§8.2: supplied descriptor disagrees with the QR payload header — fail closed"
            ),
            RestoreError::QrPayload(why) => write!(f, "§8.4: QR payload rejected: {why}"),
            RestoreError::Keyset(e) => write!(f, "{e}"),
            RestoreError::Io(e) => write!(f, "§13 identity-dir write failed: {e}"),
        }
    }
}

impl std::error::Error for RestoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RestoreError::Mnemonic(e) => Some(e),
            RestoreError::Descriptor(e) => Some(e),
            RestoreError::Keyset(e) => Some(e),
            RestoreError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<Bip39Error> for RestoreError {
    fn from(e: Bip39Error) -> Self {
        RestoreError::Mnemonic(e)
    }
}

impl From<DescriptorError> for RestoreError {
    fn from(e: DescriptorError) -> Self {
        RestoreError::Descriptor(e)
    }
}

impl From<KeysetError> for RestoreError {
    fn from(e: KeysetError) -> Self {
        RestoreError::Keyset(e)
    }
}

impl From<io::Error> for RestoreError {
    fn from(e: io::Error) -> Self {
        RestoreError::Io(e)
    }
}

/// Fail-closed descriptor resolution for a words restore (§8.2: honor a
/// present descriptor; absent ⇒ v1 defaults, with the encoding byte taken
/// from the self-evident parsed word count).
fn resolve_descriptor(
    mnemonic: &RootMnemonic,
    supplied: Option<&[u8; DESCRIPTOR_LEN]>,
) -> Result<BackupDescriptor, RestoreError> {
    match supplied {
        None => Ok(BackupDescriptor::for_mnemonic(mnemonic)),
        Some(bytes) => {
            let descriptor = BackupDescriptor::from_bytes(bytes)?;
            if descriptor.word_count().word_count() != mnemonic.word_count() {
                return Err(RestoreError::DescriptorWordCountMismatch {
                    descriptor: match descriptor.word_count() {
                        WordCount::Twelve => "0x01 = 12 words",
                        WordCount::TwentyFour => "0x02 = 24 words",
                    },
                    words: mnemonic.word_count(),
                });
            }
            Ok(descriptor)
        }
    }
}

/// Shared §8.4 restore core: parse/hygiene + descriptor check happened on the
/// way in; this refuses against a live dir, derives (§2 + §4.5 self-test),
/// writes the §13 file set atomically, and returns the confirmation surface.
fn restore_commit(
    dir: &Path,
    mnemonic: &RootMnemonic,
    descriptor: BackupDescriptor,
    counter_floor: CounterFloor,
) -> Result<RestoreOutcome, RestoreError> {
    if identity_files_present(dir) {
        return Err(RestoreError::LiveIdentityDir);
    }
    let keyset = Keyset::derive_from_root(mnemonic)?;
    let announce_counter_init = counter_floor.initial_announce_counter();
    write_identity_dir(dir, mnemonic, &keyset, descriptor, announce_counter_init)
        .map_err(|e| match e {
            DirWriteError::Exists => RestoreError::LiveIdentityDir,
            DirWriteError::Io(e) => RestoreError::Io(e),
        })?;
    Ok(RestoreOutcome {
        node_id: keyset.node_id(),
        descriptor,
        announce_counter_init,
        counter_floor_pending: matches!(counter_floor, CounterFloor::PendingMeshLearn),
    })
}

/// §8.4 — restore from typed/written words. Input hygiene (normative):
/// NFKC-normalized, case/whitespace folded (`RootMnemonic::parse`), and the
/// BIP-39 checksum MUST validate before any derivation. Then: descriptor
/// check (fail closed; absent ⇒ v1 defaults) → SLIP-0010 derivation (§2) →
/// atomic identity-dir write (§13, never overwrite — refuses a live,
/// unquarantined dir) → the restored NodeId returned for the operator
/// confirmation MUST. The old-instance-retirement confirmation is a hard
/// precondition (§8.4).
///
/// Counter rules (§8.4): this words-restore is the state-ABSENT case —
/// `announce counter = floor + 1024` per `options.counter_floor`, persisted
/// atomically with the identity files, and the same jump is applied to
/// per-peer traffic send-counters on new sessions by the `counters` module
/// (T4 — call boundary at [`initial_counters_file`]). The state-PRESENT case
/// (restore from a live dir image) reuses persisted counters as-is and has
/// no code path here by construction.
pub fn restore(
    dir: &Path,
    words: &str,
    options: &RestoreOptions,
) -> Result<RestoreOutcome, RestoreError> {
    if !options.old_instance_retired {
        return Err(RestoreError::OldInstanceNotRetired);
    }
    let mnemonic = RootMnemonic::parse(words)?;
    let descriptor = resolve_descriptor(&mnemonic, options.descriptor.as_ref())?;
    restore_commit(dir, &mnemonic, descriptor, options.counter_floor)
}

/// §8.4 — restore from a scanned project QR payload (§8.3: 4B descriptor ‖
/// raw entropy). The payload header is the honored descriptor (fail-closed
/// parse); a separately supplied `options.descriptor` must EQUAL it, else
/// fail closed. Everything else matches [`restore`]: old-instance confirmation
/// precondition, live-dir refusal, atomic §13 write, NodeId confirmation
/// surface, `floor + 1024` counter semantics.
pub fn restore_from_qr(
    dir: &Path,
    payload: &[u8],
    options: &RestoreOptions,
) -> Result<RestoreOutcome, RestoreError> {
    if !options.old_instance_retired {
        return Err(RestoreError::OldInstanceNotRetired);
    }
    let (descriptor, entropy) =
        parse_qr_payload(payload).map_err(|e| RestoreError::QrPayload(e.to_string()))?;
    if let Some(supplied) = options.descriptor {
        if BackupDescriptor::from_bytes(&supplied)? != descriptor {
            return Err(RestoreError::DescriptorConflict);
        }
    }
    let mnemonic = RootMnemonic::from_entropy(&entropy)?;
    restore_commit(dir, &mnemonic, descriptor, options.counter_floor)
}

// ---------------------------------------------------------------------------
// §13 — the atomic identity-dir write (shared with migrate.rs)
// ---------------------------------------------------------------------------

/// True iff `dir` holds any identity material — the §13 five or the
/// D9-legacy files (§8.4: a live, unquarantined dir refuses restore;
/// never overwrite).
pub(crate) fn identity_files_present(dir: &Path) -> bool {
    IDENTITY_FILE_NAMES.iter().any(|name| dir.join(name).exists())
}

/// §13/§8.4 — write the initial `counters` file through the owning
/// [`CounterStore`] API (never a hand-rolled byte layout — the two modules
/// were built in parallel and the reviewer caught the drift).
///
/// `announce_counter == 0` ⇒ a never-deployed or migrated identity:
/// [`CounterStore::create`] (announce counter 0 committed). Otherwise it is
/// a state-absent restore value `floor + RESTORE_COUNTER_JUMP` (§8.4): the
/// store is opened in restore-pending mode and the floor committed via
/// [`CounterStore::set_restore_floor`], which applies the same jump — the
/// persisted `announce_last` equals `announce_counter` exactly.
fn write_initial_counters(dir: &Path, announce_counter: u64) -> Result<(), CounterError> {
    if announce_counter == 0 {
        CounterStore::create(dir)?;
    } else {
        let floor = announce_counter
            .checked_sub(RESTORE_COUNTER_JUMP)
            .ok_or(CounterError::Overflow)?;
        let mut store = CounterStore::restore_pending(dir)?;
        store.set_restore_floor(floor)?;
    }
    Ok(())
}

/// Internal write outcome: the never-overwrite refusal is distinct from I/O
/// failure so callers can map it to their own error class (§8.4's
/// `LiveIdentityDir`, §13's Exists class).
#[derive(Debug)]
pub(crate) enum DirWriteError {
    /// The target dir already holds identity material (never overwrite, §13).
    Exists,
    Io(io::Error),
}

impl From<io::Error> for DirWriteError {
    fn from(e: io::Error) -> Self {
        DirWriteError::Io(e)
    }
}

/// Write one file `create_new` (never truncate an existing path), fsync it,
/// and pin its mode (§13's per-file modes; umask-independent).
fn write_file(path: &Path, body: &[u8], mode: u32) -> io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)?;
    f.write_all(body)?;
    f.sync_all()?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

fn fsync_dir(dir: &Path) -> io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

/// §13 — write the complete identity-dir file set atomically:
/// `mnemonic` (0600, words + trailing newline) · `keyset` (0600, 64B) ·
/// `nodeid` (0644, 34B §7 record) · `descriptor` (0644, 4B §8.2) ·
/// `counters` (0600, [`initial_counters_file`] boundary). Built in a temp
/// sibling dir (0700), every file fsynced, then renamed into place and the
/// parent fsynced — an interrupted write never leaves a partial identity.
///
/// **Never overwrite** (§13/§8.4): refused up front when the target holds
/// identity files; rename(2) then only replaces an absent or EMPTY dir, so a
/// dir that gains material mid-write fails loudly instead of replacing it.
pub(crate) fn write_identity_dir(
    dir: &Path,
    mnemonic: &RootMnemonic,
    keyset: &Keyset,
    descriptor: BackupDescriptor,
    announce_counter: u64,
) -> Result<(), DirWriteError> {
    use std::os::unix::fs::PermissionsExt;

    if identity_files_present(dir) {
        return Err(DirWriteError::Exists);
    }
    let parent = dir.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp: PathBuf = parent.join(format!(
        ".{}.tmp-{}",
        dir.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    if tmp.exists() {
        // A stale temp dir from a crashed earlier write — safe to clear: the
        // pattern is ours and a completed write renames the temp away.
        fs::remove_dir_all(&tmp)?;
    }
    fs::create_dir(&tmp)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o700))?;

    let result = (|| -> io::Result<()> {
        // §13 file bodies. The mnemonic is the canonical NFKC space-joined
        // phrase with the §13 trailing newline; the nodeid is the §7 34B
        // record (`0xed 0x01` ‖ 32B pubkey).
        let mut mnemonic_body = Zeroizing::new(mnemonic.phrase());
        mnemonic_body.push('\n');
        write_file(&tmp.join(MNEMONIC_FILE), mnemonic_body.as_bytes(), 0o600)?;
        write_file(&tmp.join(KEYSET_FILE), &keyset.to_bytes()[..], 0o600)?;
        let nodeid_record = encoding::encode_record(KeyKind::Ed25519Pub, keyset.node_id().as_bytes());
        write_file(&tmp.join(NODEID_FILE), &nodeid_record, 0o644)?;
        write_file(&tmp.join(DESCRIPTOR_FILE), &descriptor.to_bytes(), 0o644)?;
        // §13 `counters` — the format is owned by `crate::counters` (T4):
        // never hand-rolled here. Fresh/migrated identities start at 0
        // (genesis announce = 1, §6.2); a state-absent restore commits the
        // §8.4 `floor + 1024` jump through the store's own restore path.
        write_initial_counters(&tmp, announce_counter).map_err(io::Error::other)?;
        fsync_dir(&tmp)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e.into());
    }

    // Commit: rename(2) replaces only an absent or EMPTY dir — a live
    // identity can never be silently replaced (§8.4/§13).
    if let Err(e) = fs::rename(&tmp, dir) {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e.into());
    }
    fsync_dir(parent)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Backup-ceremony errors
// ---------------------------------------------------------------------------

/// §8.3 ceremony-artifact and verify-on-write-down failures.
#[derive(Debug)]
pub enum BackupError {
    /// §8.2 descriptor parse failure (fail closed).
    Descriptor(DescriptorError),
    /// §8.3: a project QR payload is exactly 20B (12 words) or 36B (24
    /// words) — anything else is not this format.
    QrPayloadLength(usize),
    /// §8.3/§8.2: the payload's descriptor `encoding` byte contradicts its
    /// entropy length (0x01 ↔ 16B, 0x02 ↔ 32B) — fail closed.
    QrEncodingMismatch,
    /// Sampled-word count outside `1..=word_count` (§8.3 step 3).
    SampleCountOutOfRange {
        /// The requested sample size.
        requested: usize,
        /// The phrase's word count (12/24).
        words: usize,
    },
    /// A sampled position outside the phrase (caller bug).
    PositionOutOfRange {
        /// The offending 0-based position.
        position: usize,
        /// The phrase's word count.
        words: usize,
    },
    /// Fewer/more typed answers than sampled positions (caller bug).
    AnswerCountMismatch {
        /// Sampled positions count.
        expected: usize,
        /// Answers supplied.
        got: usize,
    },
    /// §4.2: `getrandom()` failed drawing sample positions — no silent weak
    /// fallback (§4 rule 2); the ceremony aborts loudly.
    Entropy(getrandom::Error),
}

impl fmt::Display for BackupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackupError::Descriptor(e) => write!(f, "{e}"),
            BackupError::QrPayloadLength(n) => write!(
                f,
                "§8.3: project QR payload must be 20 or 36 bytes (4B descriptor ‖ entropy), got {n}"
            ),
            BackupError::QrEncodingMismatch => write!(
                f,
                "§8.3: QR descriptor encoding byte contradicts its entropy length — fail closed"
            ),
            BackupError::SampleCountOutOfRange { requested, words } => write!(
                f,
                "§8.3: sample size {requested} outside 1..={words}"
            ),
            BackupError::PositionOutOfRange { position, words } => write!(
                f,
                "§8.3: sampled position {position} outside a {words}-word phrase"
            ),
            BackupError::AnswerCountMismatch { expected, got } => write!(
                f,
                "§8.3: {got} answers for {expected} sampled positions"
            ),
            BackupError::Entropy(e) => write!(f, "§4.2: getrandom() failed: {e}"),
        }
    }
}

impl std::error::Error for BackupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BackupError::Descriptor(e) => Some(e),
            // getrandom 0.2's Error lacks std::error::Error without its "std"
            // feature (dep set frozen — no feature change; same as keyset.rs).
            BackupError::Entropy(_) => None,
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    const CANONICAL_WORDS: &str =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const WORDS_24: &str =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    fn to_hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    /// Minimal temp dir (no tempfile in the frozen dep set): unique per test,
    /// removed on drop.
    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "id111-backup-{name}-{}-{n}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TestDir(path)
        }

        fn identity(&self) -> PathBuf {
            self.0.join("identity")
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        path.metadata().unwrap().permissions().mode() & 0o777
    }

    fn opts() -> RestoreOptions {
        RestoreOptions {
            old_instance_retired: true,
            descriptor: None,
            counter_floor: CounterFloor::NeverDeployed,
        }
    }

    // ------------------------------------------------------------------
    // §8.2 descriptor: round-trip, fail-closed negatives.
    // ------------------------------------------------------------------

    #[test]
    fn descriptor_round_trip() {
        for (d, want) in [
            (BackupDescriptor::V1_12_WORDS, [0x01, 0x01, 0x01, 0x01]),
            (BackupDescriptor::V1_24_WORDS, [0x01, 0x01, 0x01, 0x02]),
        ] {
            let bytes = d.to_bytes();
            assert_eq!(bytes, want);
            assert_eq!(BackupDescriptor::from_bytes(&bytes).unwrap(), d);
        }
        assert_eq!(BackupDescriptor::V1_12_WORDS.word_count(), WordCount::Twelve);
        assert_eq!(BackupDescriptor::V1_24_WORDS.word_count(), WordCount::TwentyFour);
        assert_eq!(to_hex(&BackupDescriptor::V1_12_WORDS.to_bytes()), "01010101");
        assert_eq!(to_hex(&BackupDescriptor::V1_24_WORDS.to_bytes()), "01010102");
        assert_eq!(BackupDescriptor::V1_12_WORDS.to_string(), "01010101");
    }

    #[test]
    fn descriptor_fails_closed_on_unknown_fields() {
        // Every unknown field value MUST be rejected (§8.2 MUST, §12 rule 1).
        assert!(matches!(
            BackupDescriptor::from_bytes(&[0x02, 0x01, 0x01, 0x01]),
            Err(DescriptorError::UnknownVersion(0x02))
        ));
        assert!(matches!(
            BackupDescriptor::from_bytes(&[0x00, 0x01, 0x01, 0x01]),
            Err(DescriptorError::UnknownVersion(0x00))
        ));
        assert!(matches!(
            BackupDescriptor::from_bytes(&[0x01, 0x02, 0x01, 0x01]),
            Err(DescriptorError::UnknownCurve(0x02))
        ));
        assert!(matches!(
            BackupDescriptor::from_bytes(&[0x01, 0x01, 0x7f, 0x01]),
            Err(DescriptorError::UnknownDerivation(0x7f))
        ));
        assert!(matches!(
            BackupDescriptor::from_bytes(&[0x01, 0x01, 0x01, 0x00]),
            Err(DescriptorError::UnknownEncoding(0x00))
        ));
        assert!(matches!(
            BackupDescriptor::from_bytes(&[0x01, 0x01, 0x01, 0x03]),
            Err(DescriptorError::UnknownEncoding(0x03))
        ));
        // Wrong lengths.
        for n in [0usize, 3, 5, 8] {
            let buf = vec![0x01u8; n];
            assert!(matches!(
                BackupDescriptor::from_bytes(&buf),
                Err(DescriptorError::BadLength(m)) if m == n
            ));
        }
    }

    // ------------------------------------------------------------------
    // §8.3 ceremony artifacts + QR payload.
    // ------------------------------------------------------------------

    #[test]
    fn artifacts_and_qr_payload_round_trip() {
        for (words, want_len) in [(CANONICAL_WORDS, 20usize), (WORDS_24, 36)] {
            let m = RootMnemonic::parse(words).unwrap();
            let art = BackupArtifacts::new(&m);
            assert_eq!(art.words(), m.phrase());
            assert_eq!(art.qr_payload().len(), want_len);
            // Payload = 4B descriptor ‖ raw entropy (§8.3 step 2).
            let (d, entropy) = parse_qr_payload(art.qr_payload()).unwrap();
            assert_eq!(d, art.descriptor);
            assert_eq!(&entropy[..], &m.entropy()[..]);
            assert_eq!(&art.qr_payload()[..4], &d.to_bytes());
            assert!(qr_payload_matches(&m, art.qr_payload()));
            // Tampered and foreign payloads do not match.
            let mut tampered = art.qr_payload().to_vec();
            *tampered.last_mut().unwrap() ^= 1;
            assert!(!qr_payload_matches(&m, &tampered));
        }
        // Debug is redacted (§12).
        let m = RootMnemonic::parse(CANONICAL_WORDS).unwrap();
        let dbg = format!("{:?}", BackupArtifacts::new(&m));
        assert!(dbg.contains("redacted") && !dbg.contains("abandon"));
    }

    #[test]
    fn qr_payload_fail_closed_negatives() {
        // Lengths other than 20/36 are not this format.
        for n in [0usize, 4, 19, 21, 35, 37, 64] {
            let buf = vec![0x01u8; n];
            assert!(matches!(
                parse_qr_payload(&buf),
                Err(BackupError::QrPayloadLength(m)) if m == n
            ));
        }
        // Unknown descriptor field inside the payload — fail closed.
        let mut bad = vec![0x01, 0x09, 0x01, 0x01];
        bad.extend_from_slice(&[0u8; 16]);
        assert!(matches!(
            parse_qr_payload(&bad),
            Err(BackupError::Descriptor(DescriptorError::UnknownCurve(0x09)))
        ));
        // Encoding byte (0x02 = 24 words) contradicts 16B entropy — fail closed.
        let mut mismatch = vec![0x01, 0x01, 0x01, 0x02];
        mismatch.extend_from_slice(&[0u8; 16]);
        assert!(matches!(
            parse_qr_payload(&mismatch),
            Err(BackupError::QrEncodingMismatch)
        ));
    }

    // ------------------------------------------------------------------
    // §8.3 step 3 — verify-on-write-down.
    // ------------------------------------------------------------------

    #[test]
    fn sampled_positions_and_word_check() {
        let m = RootMnemonic::parse(CANONICAL_WORDS).unwrap();
        let phrase: Vec<String> = m.phrase().split(' ').map(str::to_string).collect();

        // Positions: in range, unique, ascending, requested count.
        let positions = sample_positions(WordCount::Twelve, DEFAULT_SAMPLE_COUNT).unwrap();
        assert_eq!(positions.len(), DEFAULT_SAMPLE_COUNT);
        assert!(positions.iter().all(|&p| p < 12));
        let mut dedup = positions.clone();
        dedup.dedup();
        assert_eq!(dedup, positions, "unique and ascending");
        // Full-coverage draw: all 12 positions, shuffled.
        let all = sample_positions(WordCount::Twelve, 12).unwrap();
        assert_eq!(all, (0..12).collect::<Vec<_>>());
        // Out-of-range sample sizes refused.
        assert!(matches!(
            sample_positions(WordCount::Twelve, 0),
            Err(BackupError::SampleCountOutOfRange { .. })
        ));
        assert!(matches!(
            sample_positions(WordCount::Twelve, 13),
            Err(BackupError::SampleCountOutOfRange { .. })
        ));

        // Correct answers pass — including with §8.4 hygiene (case/whitespace).
        let answers: Vec<&str> = positions.iter().map(|&p| phrase[p].as_str()).collect();
        assert!(check_words(&m, &positions, &answers).unwrap());
        let messy: Vec<String> = answers.iter().map(|a| format!("  {} ", a.to_uppercase())).collect();
        let messy_refs: Vec<&str> = messy.iter().map(String::as_str).collect();
        assert!(check_words(&m, &positions, &messy_refs).unwrap());

        // A wrong word fails the check (Ok(false), not an error).
        let mut wrong = answers.clone();
        wrong[0] = "zebra";
        assert!(!check_words(&m, &positions, &wrong).unwrap());

        // Caller bugs are errors.
        assert!(matches!(
            check_words(&m, &[12], &["abandon"]),
            Err(BackupError::PositionOutOfRange { position: 12, .. })
        ));
        assert!(matches!(
            check_words(&m, &positions, &answers[..2]),
            Err(BackupError::AnswerCountMismatch { .. })
        ));
    }

    // ------------------------------------------------------------------
    // §8.4 restore — round-trips, refusals, counter floor.
    // ------------------------------------------------------------------

    #[test]
    fn backup_restore_round_trip_identical_nodeid() {
        for (words, wc) in [
            (CANONICAL_WORDS, WordCount::Twelve),
            (WORDS_24, WordCount::TwentyFour),
        ] {
            let source = Keyset::derive_from_mnemonic(words).unwrap();
            let m = RootMnemonic::parse(words).unwrap();
            let art = BackupArtifacts::new(&m);
            assert_eq!(art.descriptor.word_count(), wc);

            // Words path.
            let t = TestDir::new("rt-words");
            let out = restore(&t.identity(), words, &opts()).unwrap();
            assert_eq!(out.node_id, source.node_id());
            assert_eq!(out.descriptor, BackupDescriptor::for_word_count(wc));
            assert_eq!(out.announce_counter_init, RESTORE_COUNTER_JUMP);
            assert!(!out.counter_floor_pending);

            // QR path (§8.4 accepts words or a QR scan).
            let t2 = TestDir::new("rt-qr");
            let out2 = restore_from_qr(&t2.identity(), art.qr_payload(), &opts()).unwrap();
            assert_eq!(out2.node_id, source.node_id());
            assert_eq!(out2.descriptor, art.descriptor);

            // The two restores agree byte-for-byte on disk.
            for name in [MNEMONIC_FILE, KEYSET_FILE, NODEID_FILE, DESCRIPTOR_FILE, COUNTERS_FILE] {
                assert_eq!(
                    fs::read(t.identity().join(name)).unwrap(),
                    fs::read(t2.identity().join(name)).unwrap(),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn restore_writes_s13_layout_modes_and_bodies() {
        let t = TestDir::new("layout");
        let dir = t.identity();
        let words = CANONICAL_WORDS;
        let source = Keyset::derive_from_mnemonic(words).unwrap();
        let out = restore(&dir, words, &opts()).unwrap();

        assert_eq!(mode(&dir), 0o700);
        // mnemonic 0600, NFKC space-joined, trailing newline (§13).
        assert_eq!(mode(&dir.join(MNEMONIC_FILE)), 0o600);
        assert_eq!(
            fs::read_to_string(dir.join(MNEMONIC_FILE)).unwrap(),
            format!("{words}\n")
        );
        // keyset 0600, 64B, equals the derived keyset (§2).
        assert_eq!(mode(&dir.join(KEYSET_FILE)), 0o600);
        let keyset_bytes = fs::read(dir.join(KEYSET_FILE)).unwrap();
        assert_eq!(keyset_bytes.len(), 64);
        assert_eq!(&keyset_bytes[..], &source.to_bytes()[..]);
        // nodeid 0644, 34B §7 record: 0xed 0x01 ‖ 32B pubkey.
        assert_eq!(mode(&dir.join(NODEID_FILE)), 0o644);
        let record = fs::read(dir.join(NODEID_FILE)).unwrap();
        assert_eq!(record.len(), 34);
        assert_eq!(&record[..2], &[0xed, 0x01]);
        assert_eq!(&record[2..], source.node_id().as_bytes());
        assert_eq!(&record[2..], out.node_id.as_bytes());
        // descriptor 0644, 4B §8.2.
        assert_eq!(mode(&dir.join(DESCRIPTOR_FILE)), 0o644);
        assert_eq!(
            fs::read(dir.join(DESCRIPTOR_FILE)).unwrap(),
            BackupDescriptor::V1_12_WORDS.to_bytes()
        );
        // counters 0600: floor 0 + 1024 (§8.4) — read back through the owning
        // `CounterStore` (format owner), never as raw bytes.
        assert_eq!(mode(&dir.join(COUNTERS_FILE)), 0o600);
        let cs = CounterStore::open(&dir).unwrap();
        assert_eq!(cs.announce_last(), RESTORE_COUNTER_JUMP);
    }

    #[test]
    fn restore_counter_floor_semantics() {
        // §8.4 state-absent: counter = floor + 1024.
        let t = TestDir::new("floor-known");
        let o = RestoreOptions {
            counter_floor: CounterFloor::Known(41),
            ..opts()
        };
        let out = restore(&t.identity(), CANONICAL_WORDS, &o).unwrap();
        assert_eq!(out.announce_counter_init, 41 + RESTORE_COUNTER_JUMP);
        assert_eq!(
            CounterStore::open(&t.identity()).unwrap().announce_last(),
            41 + RESTORE_COUNTER_JUMP
        );
        assert!(!out.counter_floor_pending);

        // Pending mesh-learn: provisional floor persisted, flag surfaced.
        let t2 = TestDir::new("floor-pending");
        let o2 = RestoreOptions {
            counter_floor: CounterFloor::PendingMeshLearn,
            ..opts()
        };
        let out2 = restore(&t2.identity(), CANONICAL_WORDS, &o2).unwrap();
        assert_eq!(out2.announce_counter_init, RESTORE_COUNTER_JUMP);
        assert!(out2.counter_floor_pending);
    }

    #[test]
    fn restore_refuses_bad_checksum_and_word_counts() {
        // §8.4: the BIP-39 checksum MUST be validated before derivation.
        let t = TestDir::new("checksum");
        let r = restore(
            &t.identity(),
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon",
            &opts(),
        );
        assert!(matches!(
            r,
            Err(RestoreError::Mnemonic(Bip39Error::Mnemonic(
                ::bip39::Error::InvalidChecksum
            )))
        ));
        assert!(!t.identity().exists(), "a refused restore writes nothing");

        // Wrong word counts: 3 words (rejected by the BIP-39 layer) and a
        // checksum-VALID 15-word phrase (outside the §8.1 v1 profile).
        let t2 = TestDir::new("count3");
        assert!(matches!(
            restore(&t2.identity(), "abandon abandon abandon", &opts()),
            Err(RestoreError::Mnemonic(Bip39Error::Mnemonic(
                ::bip39::Error::BadWordCount(3)
            )))
        ));
        let t3 = TestDir::new("count15");
        let fifteen = ::bip39::Mnemonic::from_entropy(&[0u8; 20]).unwrap().to_string();
        assert!(matches!(
            restore(&t3.identity(), &fifteen, &opts()),
            Err(RestoreError::Mnemonic(Bip39Error::UnsupportedWordCount(15)))
        ));
    }

    #[test]
    fn restore_descriptor_honored_and_fail_closed() {
        // Honored: a correct descriptor restores fine (12- and 24-word).
        let t = TestDir::new("desc-ok");
        let o = RestoreOptions {
            descriptor: Some(BackupDescriptor::V1_12_WORDS.to_bytes()),
            ..opts()
        };
        let out = restore(&t.identity(), CANONICAL_WORDS, &o).unwrap();
        assert_eq!(out.descriptor, BackupDescriptor::V1_12_WORDS);

        // Mismatch: 12-word descriptor over a 24-word phrase — fail closed.
        let t2 = TestDir::new("desc-mismatch");
        assert!(matches!(
            restore(&t2.identity(), WORDS_24, &o),
            Err(RestoreError::DescriptorWordCountMismatch { words: 24, .. })
        ));
        // Unknown field — fail closed.
        let t3 = TestDir::new("desc-unknown");
        let o3 = RestoreOptions {
            descriptor: Some([0x01, 0x01, 0x99, 0x01]),
            ..opts()
        };
        assert!(matches!(
            restore(&t3.identity(), CANONICAL_WORDS, &o3),
            Err(RestoreError::Descriptor(DescriptorError::UnknownDerivation(0x99)))
        ));

        // QR restore: a conflicting separately-supplied descriptor — fail closed.
        let m = RootMnemonic::parse(CANONICAL_WORDS).unwrap();
        let art = BackupArtifacts::new(&m);
        let t4 = TestDir::new("desc-conflict");
        let o4 = RestoreOptions {
            descriptor: Some(BackupDescriptor::V1_24_WORDS.to_bytes()),
            ..opts()
        };
        assert!(matches!(
            restore_from_qr(&t4.identity(), art.qr_payload(), &o4),
            Err(RestoreError::DescriptorConflict)
        ));
        // An equal one is accepted.
        let t5 = TestDir::new("desc-eq");
        let o5 = RestoreOptions {
            descriptor: Some(BackupDescriptor::V1_12_WORDS.to_bytes()),
            ..opts()
        };
        assert!(restore_from_qr(&t5.identity(), art.qr_payload(), &o5).is_ok());
    }

    #[test]
    fn restore_refuses_live_unquarantined_dir() {
        let t = TestDir::new("live");
        let dir = t.identity();
        restore(&dir, CANONICAL_WORDS, &opts()).unwrap();
        // A second restore over the live dir MUST be refused (§8.4; never
        // overwrite) — and the live dir is untouched.
        let r = restore(&dir, CANONICAL_WORDS, &opts());
        assert!(matches!(r, Err(RestoreError::LiveIdentityDir)));
        assert!(dir.join(KEYSET_FILE).exists());

        // A dir holding only D9-legacy material is equally live.
        let t2 = TestDir::new("live-legacy");
        let dir2 = t2.identity();
        fs::create_dir_all(&dir2).unwrap();
        fs::write(dir2.join(crate::migrate::LEGACY_SECRET_FILE), "00\n").unwrap();
        assert!(matches!(
            restore(&dir2, CANONICAL_WORDS, &opts()),
            Err(RestoreError::LiveIdentityDir)
        ));

        // QR restore refuses too.
        let m = RootMnemonic::parse(CANONICAL_WORDS).unwrap();
        let art = BackupArtifacts::new(&m);
        assert!(matches!(
            restore_from_qr(&dir, art.qr_payload(), &opts()),
            Err(RestoreError::LiveIdentityDir)
        ));
    }

    #[test]
    fn restore_refuses_until_old_instance_retired() {
        // §8.4: the ceremony MUST refuse to proceed until the operator
        // confirms retiring/quarantining the old instance.
        let t = TestDir::new("retire");
        let o = RestoreOptions {
            old_instance_retired: false,
            ..opts()
        };
        assert!(matches!(
            restore(&t.identity(), CANONICAL_WORDS, &o),
            Err(RestoreError::OldInstanceNotRetired)
        ));
        assert!(!t.identity().exists());
        let m = RootMnemonic::parse(CANONICAL_WORDS).unwrap();
        let art = BackupArtifacts::new(&m);
        assert!(matches!(
            restore_from_qr(&t.identity(), art.qr_payload(), &o),
            Err(RestoreError::OldInstanceNotRetired)
        ));
    }

    /// §14 — the backup-format known-answer vector, PRINTED for freezing by
    /// the vector task (T8; run with --nocapture). Every printed line is
    /// asserted here, so the frozen file can carry the values verbatim.
    #[test]
    fn backup_format_kat() {
        let m = RootMnemonic::parse(CANONICAL_WORDS).unwrap();
        let ks = Keyset::derive_from_root(&m).unwrap();
        let art = BackupArtifacts::new(&m);
        let nodeid_record = encoding::encode_record(KeyKind::Ed25519Pub, ks.node_id().as_bytes());
        let display = NodeIdDisplay::new(ks.node_id().as_bytes());

        println!("Backup-format KAT (frozen by the §14 vector task, T8):");
        println!("  mnemonic = abandon×11 about (BIP-39, empty passphrase)");
        println!("  descriptor (4B hex)                     = {art_desc}", art_desc = art.descriptor);
        println!("  QR payload (20B = descriptor‖entropy) = {}", to_hex(art.qr_payload()));
        println!("  NodeId (hex)                          = {}", display.hex());
        println!("  NodeId (bech32)                       = {}", display.bech32());
        println!("  operator label (display-only, §7)     = {}", display.label());
        println!("  nodeid file (34B record, hex)         = {}", to_hex(&nodeid_record));
        println!("  mnemonic file body                    = \"{}\\n\"", m.phrase());
        println!("  counters file                         = owned by crate::counters (magic 111c); restore ⇒ announce_last = floor + 1024");

        assert_eq!(art.descriptor.to_bytes(), [0x01, 0x01, 0x01, 0x01]);
        assert_eq!(
            to_hex(art.qr_payload()),
            "0101010100000000000000000000000000000000"
        );
        assert_eq!(
            display.hex(),
            "6f1d5331e36e16d341fa06c9f622cc8569710f2a62d7491e9be8e953df844f01"
        );
        assert_eq!(&nodeid_record[..2], &[0xed, 0x01]);
        assert_eq!(&nodeid_record[2..], ks.node_id().as_bytes());
    }

    /// Cross-module contract (reviewer-caught drift): the `counters` file
    /// written at restore/fresh time MUST be readable by the owning
    /// `CounterStore` with the §8.4 values intact.
    #[test]
    fn counters_file_written_at_dir_creation_opens_via_counter_store() {
        let base = std::env::temp_dir().join(format!(
            "id111-backup-counters-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let fresh = base.join("fresh");
        let restored = base.join("restored");
        fs::create_dir_all(&fresh).unwrap();
        fs::create_dir_all(&restored).unwrap();

        write_initial_counters(&fresh, 0).unwrap();
        let s = CounterStore::open(&fresh).unwrap();
        assert_eq!(s.announce_last(), 0);
        assert!(!s.is_restore_pending());

        write_initial_counters(&restored, 41 + RESTORE_COUNTER_JUMP).unwrap();
        let mut s = CounterStore::open(&restored).unwrap();
        assert_eq!(s.announce_last(), 41 + RESTORE_COUNTER_JUMP);
        assert_eq!(s.traffic_base(), RESTORE_COUNTER_JUMP);
        // First post-restore announce exceeds the floor with the §8.4 margin.
        assert_eq!(s.next_announce_counter().unwrap(), 42 + RESTORE_COUNTER_JUMP);

        let _ = fs::remove_dir_all(&base);
    }
}
