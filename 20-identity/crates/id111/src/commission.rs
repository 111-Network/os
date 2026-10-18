//! §8.3/§13 — the Cat-1 commissioning ceremony, commissioner side (D14b: owned
//! by the operator's computer running the `111` CLI, `111 identity
//! commission`). Relays never touch mnemonics.
//!
//! Implements (owner: T11):
//!
//! - entropy-verified keygen per §4 on the commissioning device, with the
//!   backup ceremony artifacts (words + single-frame QR) and the
//!   verify-on-write-down gate BEFORE any provisioning (verify-then-provision
//!   order is normative);
//! - provisioning under the §5 carve-out: the derived 64B keyset crosses
//!   exactly one point-to-point local physical channel (USB-UART provisioning
//!   mode or BLE numeric-comparison pairing — never a routed path, never
//!   broadcast radio); the relay-side keyset write is atomic (commit flag),
//!   and re-provisioning is accepted iff no committed keyset exists;
//! - ceremony end: challenge-verify (relay signs a commissioner-chosen nonce
//!   with the provisioned signing key — a misprovisioned relay fails loudly),
//!   then the commissioner MUST wipe its copy of the derived keyset (only the
//!   event is logged, never the material).
//!
//! The relay-side byte-level protocol is a documented stub here — spec'd when
//! Cat-1 firmware lands (§15).
//!
//! # Shape: a pure state machine
//!
//! [`CommissionCeremony`] performs **no I/O** — no TTY, no files, no logging,
//! no transport. The CLI (node111's `111 identity commission`) drives it and
//! owns every side effect: it displays the artifacts, collects the typed-back
//! words, and moves bytes over a [`RelayChannel`]. That split is what makes
//! the whole ceremony testable on the host ([`LoopbackRelay`]) and keeps the
//! §8.3 TTY-only rules in the layer that can enforce them.
//!
//! ```text
//!  begin() ──► Keygen ──verify_write_down()──► WriteDownVerified
//!                                                   │ provisioning_bundle()
//!                                                   ▼
//!   Wiped ◄──finish()── ChallengeVerified ◄─confirm_relay()── BundleIssued
//!     ▲                                                          │ (retry:
//!     └────────────── abort() / drop (any step) ◄────────────────┘  fresh nonce)
//! ```
//!
//! Every secret the commissioner holds lives **inside** the ceremony value —
//! the mnemonic, the derived keyset, the display artifacts, and the single
//! outstanding [`ProvisioningBundle`] (handed out only by reference, so no
//! copy can outlive the ceremony). `finish()`, `abort()` and `Drop` all wipe
//! the same set. The only thing retained past the ceremony is the public
//! [`CommissionRecord`] / [`CommissionAbort`] (NodeId + descriptor + step).
//!
//! Wipe coverage: every byte buffer this module and `backup`/`keyset` own is
//! zeroized, and the `bip39` crate's `Mnemonic` (the word-index array inside
//! [`RootMnemonic`]) is `ZeroizeOnDrop` because the crate's `zeroize` feature
//! is enabled in `Cargo.toml` (reviewer-approved at T11; no new crate).

use std::fmt;

use zeroize::{Zeroize, Zeroizing};

use crate::backup::{
    check_words, qr_payload_matches, sample_positions, BackupArtifacts, BackupDescriptor,
    BackupError, DEFAULT_SAMPLE_COUNT,
};
use crate::bip39::{RootMnemonic, WordCount};
use crate::encoding::NodeIdDisplay;
use crate::keyset::{verify_strict_tagged, Keyset, KeysetError, NodeId, VerifyError};

/// §6.1 domain tag for the ceremony's challenge-verify: the relay signs
/// `CHALLENGE_TAG ‖ nonce` with the provisioned Ed25519 signing key. The tag
/// is part of the signed region and never a wire field (§6.1) — a signature
/// made under any other tag (e.g. `"111/announce/v1"`) or over any other
/// nonce MUST fail.
pub const CHALLENGE_TAG: &[u8] = b"111/commission-challenge/v1";

/// Length of the commissioner-chosen challenge nonce (§8.3 step 4).
pub const CHALLENGE_NONCE_LEN: usize = 32;

// ---------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------

/// Where a ceremony is. Ordered: later steps compare greater.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CeremonyStep {
    /// §8.3 steps 1–2 done: entropy-verified keygen, artifacts available for
    /// display. The write-down is NOT yet verified — provisioning is locked.
    Keygen,
    /// §8.3 step 3 passed: the backup is considered made; provisioning is
    /// unlocked (verify-THEN-provision).
    WriteDownVerified,
    /// §8.3 step 4 in flight: a [`ProvisioningBundle`] is outstanding and the
    /// challenge-verify has not yet passed.
    BundleIssued,
    /// §8.3 step 4 end: the relay signed the challenge with the provisioned
    /// signing key. Only `finish()` remains.
    ChallengeVerified,
    /// The commissioner's copies are wiped (`finish()`, `abort()` or drop).
    Wiped,
}

impl CeremonyStep {
    /// Stable lowercase token for event-log lines.
    pub fn as_str(self) -> &'static str {
        match self {
            CeremonyStep::Keygen => "keygen",
            CeremonyStep::WriteDownVerified => "write-down-verified",
            CeremonyStep::BundleIssued => "bundle-issued",
            CeremonyStep::ChallengeVerified => "challenge-verified",
            CeremonyStep::Wiped => "wiped",
        }
    }
}

impl fmt::Display for CeremonyStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Ceremony failures. No variant (or its `Display`) ever carries key material,
/// words, or the keyset.
#[derive(Debug)]
pub enum CommissionError {
    /// §4: keygen failed (entropy source, mnemonic layer, or §4.5 weak-key
    /// self-test). Nothing was produced.
    Keygen(KeysetError),
    /// §4.2: `getrandom()` failed drawing the challenge nonce — no silent
    /// weak fallback.
    Entropy(getrandom::Error),
    /// The drawn challenge nonce failed the weak-value check (uniform bytes,
    /// e.g. a stuck RNG) — aborts loudly, like the §4.5 key self-test.
    WeakNonce,
    /// §8.3 ceremony-artifact / verify-on-write-down layer error (caller
    /// bugs such as an answer-count mismatch, or an entropy failure while
    /// re-sampling).
    Backup(BackupError),
    /// The freshly generated backup does not restore to the keyset about to
    /// be provisioned (the words/QR on paper would NOT recover this node).
    /// Aborts before anything is shown.
    BackupParity,
    /// §8.3 verify-then-provision: the bundle was requested before the
    /// write-down passed verification. Nothing was produced.
    WriteDownNotVerified,
    /// §8.3 step 3: the typed-back words (or re-scanned QR) did not match.
    /// The ceremony does not advance; positions are re-sampled.
    WriteDownMismatch {
        /// Failed verifications so far (this one included).
        attempts: u32,
    },
    /// A ceremony operation was called out of order.
    OutOfOrder {
        /// The operation attempted.
        op: &'static str,
        /// The step the ceremony was at.
        at: CeremonyStep,
    },
    /// §8.3 step 4: the challenge signature did not verify against the
    /// provisioned NodeId — the relay holds a different key (misprovisioned)
    /// or the response was corrupted. FAILS LOUDLY; the relay MUST NOT be
    /// deployed.
    ChallengeFailed(VerifyError),
    /// The relay channel failed or the relay refused.
    Relay(RelayError),
    /// The ceremony has been wiped (`finish()`/`abort()`); nothing further can
    /// be produced from it.
    Wiped,
}

impl fmt::Display for CommissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommissionError::Keygen(e) => write!(f, "§4 keygen failed: {e}"),
            CommissionError::Entropy(e) => write!(f, "§4.2: getrandom() failed (challenge nonce): {e}"),
            CommissionError::WeakNonce => {
                write!(f, "§4: challenge nonce failed the weak-value check — RNG fault, aborting")
            }
            CommissionError::Backup(e) => write!(f, "{e}"),
            CommissionError::BackupParity => write!(
                f,
                "§8.3: the generated words/QR do not restore the keyset about to be provisioned — aborted before display"
            ),
            CommissionError::WriteDownNotVerified => write!(
                f,
                "§8.3: verify-then-provision — the write-down has not passed verification; no provisioning bundle is available"
            ),
            CommissionError::WriteDownMismatch { attempts } => write!(
                f,
                "§8.3 step 3: typed-back words do not match (failed attempts: {attempts}) — the backup is not considered made"
            ),
            CommissionError::OutOfOrder { op, at } => {
                write!(f, "ceremony step `{op}` is not valid at `{at}`")
            }
            CommissionError::ChallengeFailed(e) => write!(
                f,
                "§8.3 step 4: relay challenge-verify FAILED ({e}) — the relay does not hold the provisioned signing key \
                 (misprovisioned or corrupted in transit); do NOT deploy it"
            ),
            CommissionError::Relay(e) => write!(f, "relay channel: {e}"),
            CommissionError::Wiped => {
                write!(f, "the commissioner's copies are wiped — the ceremony is over")
            }
        }
    }
}

impl std::error::Error for CommissionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CommissionError::Keygen(e) => Some(e),
            CommissionError::Backup(e) => Some(e),
            CommissionError::ChallengeFailed(e) => Some(e),
            CommissionError::Relay(e) => Some(e),
            // getrandom 0.2's Error lacks std::error::Error without its "std"
            // feature (dep set frozen — same as keyset.rs).
            _ => None,
        }
    }
}

impl From<KeysetError> for CommissionError {
    fn from(e: KeysetError) -> Self {
        CommissionError::Keygen(e)
    }
}

impl From<BackupError> for CommissionError {
    fn from(e: BackupError) -> Self {
        CommissionError::Backup(e)
    }
}

impl From<RelayError> for CommissionError {
    fn from(e: RelayError) -> Self {
        CommissionError::Relay(e)
    }
}

// ---------------------------------------------------------------------------
// What survives the ceremony
// ---------------------------------------------------------------------------

/// The ONLY thing a completed ceremony retains (§8.3 step 4: "only the event
/// is logged, never the material"): the commissioned node's public identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommissionRecord {
    /// The commissioned relay's NodeId (public).
    pub node_id: NodeId,
    /// The §8.2 descriptor of the backup made (public — printed on paper).
    pub descriptor: BackupDescriptor,
    /// The step at which the relay was verified — always
    /// [`CeremonyStep::ChallengeVerified`] for a record `finish()` returns
    /// (the challenge-verify is what makes a commission real).
    pub verified_at_step: CeremonyStep,
}

impl CommissionRecord {
    /// The event-log line: NodeId (hex + label), descriptor, step. Public
    /// values only — safe for any log.
    pub fn log_line(&self) -> String {
        let d = NodeIdDisplay::new(self.node_id.as_bytes());
        format!(
            "commissioned node={} label={} descriptor={} verified_at={}",
            d.hex(),
            d.label(),
            self.descriptor,
            self.verified_at_step
        )
    }
}

/// What an abandoned ceremony leaves behind (public values only): enough for
/// an operator to know which NodeId may exist on a half-provisioned relay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommissionAbort {
    /// The NodeId the ceremony generated (public).
    pub node_id: NodeId,
    /// The §8.2 descriptor (public).
    pub descriptor: BackupDescriptor,
    /// The step the ceremony had reached when it was abandoned.
    pub reached: CeremonyStep,
}

impl CommissionAbort {
    /// The event-log line. Public values only.
    pub fn log_line(&self) -> String {
        let d = NodeIdDisplay::new(self.node_id.as_bytes());
        format!(
            "aborted node={} label={} descriptor={} reached={} (commissioner copies wiped)",
            d.hex(),
            d.label(),
            self.descriptor,
            self.reached
        )
    }
}

// ---------------------------------------------------------------------------
// The provisioning bundle (§8.3 step 4 payload)
// ---------------------------------------------------------------------------

/// §8.3 step 4 payload: the derived 64B private keyset + the §8.2 descriptor +
/// a fresh 32B challenge nonce. Relays never receive the mnemonic.
///
/// The keyset is the §5 carve-out transit: [`keyset_bytes`](Self::keyset_bytes)
/// exists for [`RelayChannel`] implementations only. The bundle is owned by the
/// ceremony and handed out by reference, so no copy outlives `finish()` /
/// `abort()` / drop. Not `Clone`, `Debug` redacts.
pub struct ProvisioningBundle {
    node_id: NodeId,
    descriptor: BackupDescriptor,
    keyset: Zeroizing<[u8; 64]>,
    nonce: [u8; CHALLENGE_NONCE_LEN],
}

/// The non-secret face of a [`ProvisioningBundle`] — safe to print and log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BundleSummary {
    /// The NodeId the relay must end up holding (public).
    pub node_id: NodeId,
    /// The §8.2 descriptor (public).
    pub descriptor: BackupDescriptor,
    /// The challenge nonce the relay must sign (public by construction — it
    /// is a challenge, not a secret).
    pub challenge_nonce: [u8; CHALLENGE_NONCE_LEN],
}

impl ProvisioningBundle {
    /// SECRET — the 64B private keyset (`Ed25519 seed ‖ X25519 scalar`, §2).
    /// For [`RelayChannel`] implementations moving it over the §8.3 step-4
    /// local channel; never log, display, or persist it.
    pub fn keyset_bytes(&self) -> &[u8; 64] {
        &self.keyset
    }

    /// The §8.2 descriptor (public).
    pub fn descriptor(&self) -> BackupDescriptor {
        self.descriptor
    }

    /// The challenge nonce the relay signs (`CHALLENGE_TAG ‖ nonce`).
    pub fn challenge_nonce(&self) -> &[u8; CHALLENGE_NONCE_LEN] {
        &self.nonce
    }

    /// The NodeId the provisioned relay must prove it holds (public).
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    /// The non-secret summary (NodeId, descriptor, nonce).
    pub fn summary(&self) -> BundleSummary {
        BundleSummary {
            node_id: self.node_id,
            descriptor: self.descriptor,
            challenge_nonce: self.nonce,
        }
    }
}

impl fmt::Debug for ProvisioningBundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // §5/§12: the keyset is THE asset — never Debug-print it.
        write!(
            f,
            "ProvisioningBundle(node {:?}, descriptor {}, keyset redacted)",
            self.node_id, self.descriptor
        )
    }
}

// ---------------------------------------------------------------------------
// The relay channel (documented stub — §15)
// ---------------------------------------------------------------------------

/// The closed set of channels the §8.3 step-4 minimums permit. There is
/// deliberately **no** routed-network and **no** broadcast-radio variant: a
/// channel that cannot name itself here cannot honestly implement
/// [`RelayChannel`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalChannel {
    /// USB-UART provisioning mode (point-to-point wired).
    UsbUartProvisioning,
    /// BLE with numeric-comparison pairing (LE Secure Connections).
    BleNumericComparison,
    /// In-process host test double ([`LoopbackRelay`]) — nothing leaves the
    /// process; NEVER valid for a real relay.
    HostLoopback,
}

impl LocalChannel {
    /// Human-readable name for ceremony output.
    pub fn as_str(self) -> &'static str {
        match self {
            LocalChannel::UsbUartProvisioning => "USB-UART provisioning mode",
            LocalChannel::BleNumericComparison => "BLE numeric-comparison pairing",
            LocalChannel::HostLoopback => "host loopback (in-process test double)",
        }
    }
}

/// Relay-channel failures.
#[derive(Debug, PartialEq, Eq)]
pub enum RelayError {
    /// §8.3: the relay already holds a committed keyset and refuses
    /// re-provisioning (never overwrite, §13).
    AlreadyCommitted,
    /// Relay-side validation refused the bundle (e.g. the §4.5 weak-key
    /// self-test). The payload is a fixed diagnostic — never key bytes.
    Rejected(&'static str),
    /// A challenge signature was requested but none is available (nothing
    /// committed yet, or it was already read).
    NoChallengeSignature,
    /// Transport failure (real channels). MUST NOT contain key bytes.
    Channel(String),
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelayError::AlreadyCommitted => write!(
                f,
                "relay already holds a committed keyset and refuses re-provisioning (§8.3/§13 never-overwrite)"
            ),
            RelayError::Rejected(why) => write!(f, "relay rejected the bundle: {why}"),
            RelayError::NoChallengeSignature => write!(f, "no challenge signature available from the relay"),
            RelayError::Channel(why) => write!(f, "transport failure: {why}"),
        }
    }
}

impl std::error::Error for RelayError {}

/// The commissioning channel to a Cat-1 relay — **the relay-side byte protocol
/// is a documented stub** (framing, retries, and the firmware half are spec'd
/// when Cat-1 firmware lands, §15). This trait is the contract a real
/// implementation (USB-UART provisioning mode, BLE numeric-comparison) must
/// meet.
///
/// # Channel minimums (normative, §8.3 step 4 / §5 carve-out)
///
/// - **Point-to-point, local, physical.** USB-UART provisioning mode, or BLE
///   with numeric-comparison pairing (LE Secure Connections) — the operator
///   compares the displayed number on both ends. Nothing else.
/// - **Never a routed network path** (no IP, no mesh hop, no relay-forwarded
///   transport) and **never broadcast radio** (no unpaired BLE advertising,
///   LoRa, ESP-NOW broadcast). [`channel`](Self::channel) must name one of the
///   permitted [`LocalChannel`] kinds.
/// - **The keyset crosses exactly once**, from [`send_bundle`](Self::send_bundle)
///   to the relay, and is never logged, buffered to disk, or echoed back. An
///   implementation zeroizes every transmit buffer it allocates and never puts
///   key bytes in a [`RelayError`].
///
/// # Relay-side obligations (the firmware half, deferred — §15)
///
/// - The relay writes the keyset **atomically** (commit flag): a power loss
///   mid-write leaves "no committed keyset", never half a key.
/// - The relay accepts (re-)provisioning **iff no committed keyset exists**;
///   otherwise it refuses with [`RelayError::AlreadyCommitted`] — it never
///   overwrites.
/// - After the commit the relay signs `CHALLENGE_TAG ‖ nonce` (the nonce
///   arrived in the bundle) with the provisioned Ed25519 signing key and
///   returns the 64B signature via [`recv_challenge_sig`](Self::recv_challenge_sig).
///   The commissioner verifies it with the strict predicate (§3) against the
///   NodeId: a misprovisioned relay fails loudly, not silently.
/// - The relay never receives, derives from, or stores a mnemonic (D14b).
pub trait RelayChannel {
    /// Which §8.3-permitted local channel this is.
    fn channel(&self) -> LocalChannel;

    /// Transfer the bundle (keyset + descriptor + nonce) to the relay and
    /// return once the relay has committed (or refused).
    fn send_bundle(&mut self, bundle: &ProvisioningBundle) -> Result<(), RelayError>;

    /// Read the relay's 64B Ed25519 signature over `CHALLENGE_TAG ‖ nonce`.
    fn recv_challenge_sig(&mut self) -> Result<[u8; 64], RelayError>;
}

/// Host test double for the relay side (§15 stub): stores the received keyset
/// in memory under an atomic "commit flag" (the `Option` is the flag — set by
/// one assignment after validation), refuses re-provisioning once committed,
/// and signs the challenge with whatever key it holds. This makes the whole
/// ceremony end-to-end testable without hardware.
///
/// [`misprovisioned`](Self::misprovisioned) builds the failure case: a relay
/// that commits the wrong key.
pub struct LoopbackRelay {
    committed: Option<LoopbackCommit>,
    rogue: Option<Keyset>,
}

struct LoopbackCommit {
    keyset: Keyset,
    descriptor: BackupDescriptor,
    challenge_sig: Option<[u8; 64]>,
}

impl LoopbackRelay {
    /// A healthy blank relay (no committed keyset).
    pub fn new() -> Self {
        LoopbackRelay {
            committed: None,
            rogue: None,
        }
    }

    /// A faulty relay that, on `send_bundle`, commits `rogue` instead of the
    /// keyset it was sent — the "misprovisioned relay" the challenge-verify
    /// exists to catch.
    pub fn misprovisioned(rogue: Keyset) -> Self {
        LoopbackRelay {
            committed: None,
            rogue: Some(rogue),
        }
    }

    /// Has the (atomic) commit flag been set?
    pub fn is_committed(&self) -> bool {
        self.committed.is_some()
    }

    /// The NodeId of the key the relay committed (public), if any.
    pub fn committed_node_id(&self) -> Option<NodeId> {
        self.committed.as_ref().map(|c| c.keyset.node_id())
    }

    /// The descriptor the relay committed alongside the key, if any.
    pub fn committed_descriptor(&self) -> Option<BackupDescriptor> {
        self.committed.as_ref().map(|c| c.descriptor)
    }

    /// Test-only: the committed private keyset bytes (the real relay never
    /// exposes these; the double does not either outside its own tests).
    #[cfg(test)]
    fn stored_keyset_bytes(&self) -> Option<Zeroizing<[u8; 64]>> {
        self.committed.as_ref().map(|c| c.keyset.to_bytes())
    }
}

impl Default for LoopbackRelay {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for LoopbackRelay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LoopbackRelay(committed: {:?}, keyset redacted)",
            self.committed_node_id()
        )
    }
}

impl RelayChannel for LoopbackRelay {
    fn channel(&self) -> LocalChannel {
        LocalChannel::HostLoopback
    }

    fn send_bundle(&mut self, bundle: &ProvisioningBundle) -> Result<(), RelayError> {
        // §8.3: re-provisioning accepted iff no committed keyset exists.
        if self.committed.is_some() {
            return Err(RelayError::AlreadyCommitted);
        }
        // Relay-side validation (§4.5 weak-key self-test on the received blob)
        // happens BEFORE the commit flag is set.
        let keyset = match self.rogue.take() {
            Some(rogue) => rogue,
            None => Keyset::from_bytes(bundle.keyset_bytes()).map_err(|_| {
                RelayError::Rejected("keyset failed the relay-side §4.5 weak-key self-test")
            })?,
        };
        let challenge_sig = keyset.sign_with_tag(CHALLENGE_TAG, bundle.challenge_nonce());
        // Atomic commit: one assignment publishes key + descriptor + response.
        self.committed = Some(LoopbackCommit {
            keyset,
            descriptor: bundle.descriptor(),
            challenge_sig: Some(challenge_sig),
        });
        Ok(())
    }

    fn recv_challenge_sig(&mut self) -> Result<[u8; 64], RelayError> {
        self.committed
            .as_mut()
            .and_then(|c| c.challenge_sig.take())
            .ok_or(RelayError::NoChallengeSignature)
    }
}

// ---------------------------------------------------------------------------
// The ceremony
// ---------------------------------------------------------------------------

/// Everything secret the commissioner holds. One struct so one wipe covers it.
struct Secrets {
    mnemonic: RootMnemonic,
    keyset: Keyset,
    artifacts: BackupArtifacts,
    bundle: Option<ProvisioningBundle>,
}

impl Secrets {
    /// Overwrite what can be overwritten in place; the remaining zeroizing
    /// buffers (`artifacts`, the mnemonic's byte-owning parts) wipe in their
    /// own `Drop` when this value is dropped right after.
    fn wipe_in_place(&mut self) {
        self.keyset.zeroize();
        if let Some(b) = self.bundle.as_mut() {
            b.keyset.zeroize();
            b.nonce.zeroize();
        }
    }
}

/// The Cat-1 commissioning ceremony (§8.3), commissioner side — a pure state
/// machine; see the module docs for the diagram and the I/O split.
pub struct CommissionCeremony {
    step: CeremonyStep,
    node_id: NodeId,
    descriptor: BackupDescriptor,
    /// 0-based sampled word positions for the current write-down check.
    positions: Vec<usize>,
    failed_verifications: u32,
    failed_challenges: u32,
    secrets: Option<Secrets>,
}

impl CommissionCeremony {
    /// §8.3 steps 1–2: entropy-verified keygen (12 words — Cat 1 relays; see
    /// [`begin_with`](Self::begin_with) for hubs) and the display artifacts.
    pub fn begin() -> Result<Self, CommissionError> {
        Self::begin_with(WordCount::default())
    }

    /// As [`begin`](Self::begin) with an explicit word count (24 words: hubs).
    pub fn begin_with(words: WordCount) -> Result<Self, CommissionError> {
        // §4: getrandom() entropy (no silent weak fallback) + §4.5 weak-key
        // self-test run inside `Keyset::generate`.
        let (mnemonic, keyset) = Keyset::generate(words)?;
        Self::from_generated(mnemonic, keyset)
    }

    fn from_generated(mnemonic: RootMnemonic, keyset: Keyset) -> Result<Self, CommissionError> {
        let artifacts = BackupArtifacts::new(&mnemonic);
        let node_id = keyset.node_id();

        // Parity gate: the words and QR about to be shown MUST restore exactly
        // the keyset about to be provisioned — else the "backup" is a lie.
        let restored = Keyset::derive_from_mnemonic(artifacts.words())?;
        if restored.node_id() != node_id || !qr_payload_matches(&mnemonic, artifacts.qr_payload()) {
            return Err(CommissionError::BackupParity);
        }
        drop(restored);

        let descriptor = artifacts.descriptor;
        let positions = sample_positions(descriptor.word_count(), DEFAULT_SAMPLE_COUNT)?;
        Ok(CommissionCeremony {
            step: CeremonyStep::Keygen,
            node_id,
            descriptor,
            positions,
            failed_verifications: 0,
            failed_challenges: 0,
            secrets: Some(Secrets {
                mnemonic,
                keyset,
                artifacts,
                bundle: None,
            }),
        })
    }

    // -- public, non-secret state ------------------------------------------

    /// Current step.
    pub fn step(&self) -> CeremonyStep {
        self.step
    }

    /// The generated NodeId (public; valid for the ceremony's whole life).
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    /// §7 presentation surface of the NodeId (hex · bech32 · label).
    pub fn display(&self) -> NodeIdDisplay {
        NodeIdDisplay::new(self.node_id.as_bytes())
    }

    /// The §8.2 descriptor (public).
    pub fn descriptor(&self) -> BackupDescriptor {
        self.descriptor
    }

    /// 0-based word positions the operator must type back now (display
    /// numbering is 1-based — the CLI's job). Re-sampled after every failed
    /// attempt. Empty after the ceremony is wiped.
    pub fn write_down_positions(&self) -> &[usize] {
        &self.positions
    }

    /// Failed write-down verifications so far.
    pub fn failed_verifications(&self) -> u32 {
        self.failed_verifications
    }

    /// Failed relay challenge-verifies so far.
    pub fn failed_challenges(&self) -> u32 {
        self.failed_challenges
    }

    // -- §8.3 steps 2–3 -----------------------------------------------------

    /// SECRET — the words + project QR payload for the §8.3 step-2 display
    /// (local-TTY ceremony only; the TTY rules are the CLI's). `Err(Wiped)`
    /// once the ceremony is over.
    pub fn artifacts(&self) -> Result<&BackupArtifacts, CommissionError> {
        self.secrets
            .as_ref()
            .map(|s| &s.artifacts)
            .ok_or(CommissionError::Wiped)
    }

    /// §8.3 step 3 — Codex32-style sampled-word type-back. `answers[i]` is the
    /// operator's word for `write_down_positions()[i]` (trim/case/NFKC hygiene
    /// applied, as in §8.4). On success provisioning unlocks; on a wrong word
    /// the ceremony does NOT advance, the failure is counted, and fresh
    /// positions are sampled. A wrong answer *count* is a caller bug
    /// (`Backup(AnswerCountMismatch)`), not a failed attempt.
    pub fn verify_write_down(&mut self, answers: &[&str]) -> Result<(), CommissionError> {
        self.require_step("verify_write_down", CeremonyStep::Keygen)?;
        let secrets = self.secrets.as_ref().ok_or(CommissionError::Wiped)?;
        if check_words(&secrets.mnemonic, &self.positions, answers)? {
            self.step = CeremonyStep::WriteDownVerified;
            return Ok(());
        }
        self.record_failed_verification()
    }

    /// §8.3 step 3, QR half — the operator re-scanned the project QR; true
    /// match (descriptor + entropy) unlocks provisioning, anything else is a
    /// counted failed attempt.
    pub fn verify_write_down_qr(&mut self, scanned_payload: &[u8]) -> Result<(), CommissionError> {
        self.require_step("verify_write_down_qr", CeremonyStep::Keygen)?;
        let secrets = self.secrets.as_ref().ok_or(CommissionError::Wiped)?;
        if qr_payload_matches(&secrets.mnemonic, scanned_payload) {
            self.step = CeremonyStep::WriteDownVerified;
            return Ok(());
        }
        self.record_failed_verification()
    }

    fn record_failed_verification(&mut self) -> Result<(), CommissionError> {
        self.failed_verifications += 1;
        // Fresh positions: a retry cannot be passed by trial on a fixed
        // subset, and the operator re-checks different words on the paper.
        self.positions = sample_positions(self.descriptor.word_count(), DEFAULT_SAMPLE_COUNT)?;
        Err(CommissionError::WriteDownMismatch {
            attempts: self.failed_verifications,
        })
    }

    // -- §8.3 step 4 --------------------------------------------------------

    /// §8.3 step 4 — issue the provisioning bundle (64B keyset + descriptor +
    /// a **fresh** 32B challenge nonce). Available ONLY after the write-down
    /// passed verification ([`CommissionError::WriteDownNotVerified`]
    /// otherwise). Calling again before the challenge-verify (e.g. a failed
    /// transfer) replaces the outstanding bundle with a fresh nonce, so a
    /// stale signature can never confirm a newer bundle.
    ///
    /// The bundle is returned by reference: the ceremony owns every copy of
    /// the keyset, and `finish()`/`abort()`/drop wipes them all.
    pub fn provisioning_bundle(&mut self) -> Result<&ProvisioningBundle, CommissionError> {
        self.issue_bundle()?;
        self.current_bundle()
    }

    fn issue_bundle(&mut self) -> Result<(), CommissionError> {
        match self.step {
            CeremonyStep::Wiped => return Err(CommissionError::Wiped),
            CeremonyStep::Keygen => return Err(CommissionError::WriteDownNotVerified),
            CeremonyStep::ChallengeVerified => {
                return Err(CommissionError::OutOfOrder {
                    op: "provisioning_bundle",
                    at: self.step,
                })
            }
            CeremonyStep::WriteDownVerified | CeremonyStep::BundleIssued => {}
        }
        let nonce = fresh_nonce()?;
        let (node_id, descriptor) = (self.node_id, self.descriptor);
        let secrets = self.secrets.as_mut().ok_or(CommissionError::Wiped)?;
        // Replacing drops (and thereby zeroizes) any previously issued bundle.
        secrets.bundle = Some(ProvisioningBundle {
            node_id,
            descriptor,
            keyset: secrets.keyset.to_bytes(),
            nonce,
        });
        self.step = CeremonyStep::BundleIssued;
        Ok(())
    }

    fn current_bundle(&self) -> Result<&ProvisioningBundle, CommissionError> {
        match &self.secrets {
            None => Err(CommissionError::Wiped),
            Some(s) => s.bundle.as_ref().ok_or(CommissionError::OutOfOrder {
                op: "current_bundle",
                at: self.step,
            }),
        }
    }

    /// §8.3 step 4 — verify the relay's challenge response: the 64B signature
    /// must be a strict-predicate (§3) Ed25519 signature by the provisioned
    /// NodeId over `CHALLENGE_TAG ‖ nonce` of the OUTSTANDING bundle. Any
    /// other key, tag, or nonce fails loudly
    /// ([`CommissionError::ChallengeFailed`]) and the ceremony stays at
    /// `BundleIssued` (a retry is allowed; `finish()` is not). On success the
    /// bundle — and with it the commissioner's transit copy of the keyset —
    /// is wiped immediately.
    pub fn confirm_relay(&mut self, challenge_signature: &[u8; 64]) -> Result<(), CommissionError> {
        self.require_step("confirm_relay", CeremonyStep::BundleIssued)?;
        let nonce = *self.current_bundle()?.challenge_nonce();
        match verify_strict_tagged(&self.node_id, CHALLENGE_TAG, &nonce, challenge_signature) {
            Ok(()) => {
                self.step = CeremonyStep::ChallengeVerified;
                if let Some(s) = self.secrets.as_mut() {
                    s.bundle = None; // Zeroizing keyset copy dropped
                }
                Ok(())
            }
            Err(e) => {
                self.failed_challenges += 1;
                Err(CommissionError::ChallengeFailed(e))
            }
        }
    }

    /// Convenience driver for the whole step-4 transfer over a
    /// [`RelayChannel`]: issue a fresh bundle, `send_bundle`,
    /// `recv_challenge_sig`, `confirm_relay`. Returns the bundle's non-secret
    /// summary (what was sent) on success. The lower-level calls remain public
    /// for CLIs that interleave display.
    pub fn provision_via<R: RelayChannel>(
        &mut self,
        relay: &mut R,
    ) -> Result<BundleSummary, CommissionError> {
        self.issue_bundle()?;
        let summary = {
            let bundle = self.current_bundle()?;
            relay.send_bundle(bundle)?;
            bundle.summary()
        };
        let signature = relay.recv_challenge_sig()?;
        self.confirm_relay(&signature)?;
        Ok(summary)
    }

    // -- ceremony end -------------------------------------------------------

    /// §8.3 step 4 end — requires the challenge-verify to have passed. Wipes
    /// the commissioner's copies (keyset, mnemonic, artifacts, any bundle) and
    /// returns the [`CommissionRecord`]: the ONLY thing retained. Afterwards
    /// the ceremony produces nothing (`Err(Wiped)`).
    pub fn finish(&mut self) -> Result<CommissionRecord, CommissionError> {
        self.require_step("finish", CeremonyStep::ChallengeVerified)?;
        let record = CommissionRecord {
            node_id: self.node_id,
            descriptor: self.descriptor,
            verified_at_step: self.step,
        };
        self.wipe();
        Ok(record)
    }

    /// Abandon the ceremony at any step: wipe everything and report (public
    /// values only) how far it got. Dropping the ceremony wipes identically;
    /// `abort()` additionally yields the line an operator needs to know which
    /// NodeId may exist on a half-provisioned relay.
    pub fn abort(&mut self) -> CommissionAbort {
        let abort = CommissionAbort {
            node_id: self.node_id,
            descriptor: self.descriptor,
            reached: self.step,
        };
        self.wipe();
        abort
    }

    fn wipe(&mut self) {
        if let Some(mut secrets) = self.secrets.take() {
            secrets.wipe_in_place();
            // `secrets` drops here: artifacts' Zeroizing buffers, the keyset
            // (ZeroizeOnDrop), the mnemonic.
        }
        self.positions.clear();
        self.step = CeremonyStep::Wiped;
    }

    fn require_step(
        &self,
        op: &'static str,
        expected: CeremonyStep,
    ) -> Result<(), CommissionError> {
        if self.step == CeremonyStep::Wiped {
            return Err(CommissionError::Wiped);
        }
        if self.step != expected {
            return Err(CommissionError::OutOfOrder { op, at: self.step });
        }
        Ok(())
    }
}

impl Drop for CommissionCeremony {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl fmt::Debug for CommissionCeremony {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // §5/§12: mnemonic and keyset are secret — never Debug-print them.
        write!(
            f,
            "CommissionCeremony(step {}, node {:?}, descriptor {}, secrets redacted)",
            self.step, self.node_id, self.descriptor
        )
    }
}

// ---------------------------------------------------------------------------
// Nonce
// ---------------------------------------------------------------------------

/// Draw a fresh challenge nonce from `getrandom()` (§4.2), weak-value-checked.
fn fresh_nonce() -> Result<[u8; CHALLENGE_NONCE_LEN], CommissionError> {
    let mut nonce = [0u8; CHALLENGE_NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(CommissionError::Entropy)?;
    reject_weak_nonce(&nonce)?;
    Ok(nonce)
}

/// §4-style weak-value gate for the nonce: uniform-byte output (all-zero
/// included) means a stuck or absent entropy source.
fn reject_weak_nonce(nonce: &[u8; CHALLENGE_NONCE_LEN]) -> Result<(), CommissionError> {
    if nonce.iter().all(|b| *b == nonce[0]) {
        return Err(CommissionError::WeakNonce);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyset::generate_mnemonic;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The correct typed-back answers for the ceremony's CURRENT positions
    /// (test-only: reads the words the way an operator reads their paper).
    fn correct_answers(c: &CommissionCeremony) -> Vec<String> {
        let phrase = c.artifacts().unwrap().words().to_string();
        let words: Vec<&str> = phrase.split(' ').collect();
        c.write_down_positions()
            .iter()
            .map(|&p| words[p].to_string())
            .collect()
    }

    fn pass_write_down(c: &mut CommissionCeremony) {
        let answers = correct_answers(c);
        let refs: Vec<&str> = answers.iter().map(String::as_str).collect();
        c.verify_write_down(&refs).unwrap();
        assert_eq!(c.step(), CeremonyStep::WriteDownVerified);
    }

    fn verified() -> CommissionCeremony {
        let mut c = CommissionCeremony::begin().unwrap();
        pass_write_down(&mut c);
        c
    }

    #[test]
    fn full_ceremony_end_to_end_with_loopback_relay() {
        let mut c = CommissionCeremony::begin().unwrap();
        assert_eq!(c.step(), CeremonyStep::Keygen);

        // §8.3 steps 1–2: 12-word v1 profile, descriptor 01010101, 20B QR payload.
        let artifacts = c.artifacts().unwrap();
        assert_eq!(artifacts.words().split(' ').count(), 12);
        assert_eq!(artifacts.descriptor, BackupDescriptor::V1_12_WORDS);
        assert_eq!(artifacts.qr_payload().len(), 20);
        assert_eq!(&artifacts.qr_payload()[..4], &[1, 1, 1, 1]);
        assert_eq!(c.write_down_positions().len(), DEFAULT_SAMPLE_COUNT);
        let phrase = artifacts.words().to_string();
        let expected_node = Keyset::derive_from_mnemonic(&phrase).unwrap().node_id();
        assert_eq!(
            c.node_id(),
            expected_node,
            "the words restore the commissioned NodeId"
        );

        // §8.3 step 3, then step 4 over the loopback relay.
        pass_write_down(&mut c);
        let mut relay = LoopbackRelay::new();
        assert!(!relay.is_committed());
        let summary = c.provision_via(&mut relay).unwrap();
        assert_eq!(c.step(), CeremonyStep::ChallengeVerified);
        assert_eq!(summary.node_id, expected_node);
        assert_eq!(summary.descriptor, BackupDescriptor::V1_12_WORDS);

        // The relay holds exactly the keyset the words derive (private bytes
        // compared inside the test only).
        assert!(relay.is_committed());
        assert_eq!(relay.committed_node_id(), Some(expected_node));
        assert_eq!(
            relay.committed_descriptor(),
            Some(BackupDescriptor::V1_12_WORDS)
        );
        let derived = Keyset::derive_from_mnemonic(&phrase).unwrap().to_bytes();
        assert_eq!(*relay.stored_keyset_bytes().unwrap(), *derived);

        // Ceremony end.
        let record = c.finish().unwrap();
        assert_eq!(record.node_id, expected_node);
        assert_eq!(record.descriptor, BackupDescriptor::V1_12_WORDS);
        assert_eq!(record.verified_at_step, CeremonyStep::ChallengeVerified);
        assert_eq!(c.step(), CeremonyStep::Wiped);
    }

    #[test]
    fn full_ceremony_24_words_for_hubs() {
        let mut c = CommissionCeremony::begin_with(WordCount::TwentyFour).unwrap();
        assert_eq!(c.descriptor(), BackupDescriptor::V1_24_WORDS);
        assert_eq!(c.artifacts().unwrap().words().split(' ').count(), 24);
        assert_eq!(c.artifacts().unwrap().qr_payload().len(), 36);
        pass_write_down(&mut c);
        c.provision_via(&mut LoopbackRelay::new()).unwrap();
        assert_eq!(
            c.finish().unwrap().descriptor,
            BackupDescriptor::V1_24_WORDS
        );
    }

    #[test]
    fn bundle_refused_before_verification() {
        let mut c = CommissionCeremony::begin().unwrap();
        assert!(matches!(
            c.provisioning_bundle(),
            Err(CommissionError::WriteDownNotVerified)
        ));
        // The driver is gated identically, and the relay saw nothing.
        let mut relay = LoopbackRelay::new();
        assert!(matches!(
            c.provision_via(&mut relay),
            Err(CommissionError::WriteDownNotVerified)
        ));
        assert!(!relay.is_committed());
        // confirm_relay / finish are out of order too.
        assert!(matches!(
            c.confirm_relay(&[0u8; 64]),
            Err(CommissionError::OutOfOrder {
                op: "confirm_relay",
                at: CeremonyStep::Keygen
            })
        ));
        assert!(matches!(
            c.finish(),
            Err(CommissionError::OutOfOrder {
                op: "finish",
                at: CeremonyStep::Keygen
            })
        ));
        assert_eq!(c.step(), CeremonyStep::Keygen);
    }

    #[test]
    fn wrong_answer_is_refused_and_does_not_advance() {
        let mut c = CommissionCeremony::begin().unwrap();
        let mut answers = correct_answers(&c);
        answers[0] = "not-a-bip39-word".into();
        let refs: Vec<&str> = answers.iter().map(String::as_str).collect();
        match c.verify_write_down(&refs) {
            Err(CommissionError::WriteDownMismatch { attempts: 1 }) => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(c.step(), CeremonyStep::Keygen);
        assert_eq!(c.failed_verifications(), 1);
        assert!(matches!(
            c.provisioning_bundle(),
            Err(CommissionError::WriteDownNotVerified)
        ));
        // Positions were re-sampled; the right words for the NEW positions pass.
        assert_eq!(c.write_down_positions().len(), DEFAULT_SAMPLE_COUNT);
        pass_write_down(&mut c);
        assert!(c.provisioning_bundle().is_ok());
    }

    #[test]
    fn answer_hygiene_and_count_mismatch() {
        let mut c = CommissionCeremony::begin().unwrap();
        // Wrong COUNT is a caller bug, not a failed attempt.
        assert!(matches!(
            c.verify_write_down(&["only-one"]),
            Err(CommissionError::Backup(
                BackupError::AnswerCountMismatch { .. }
            ))
        ));
        assert_eq!(c.failed_verifications(), 0);
        // §8.4 hygiene: case/whitespace folded.
        let answers: Vec<String> = correct_answers(&c)
            .iter()
            .map(|w| format!("  {}\t", w.to_uppercase()))
            .collect();
        let refs: Vec<&str> = answers.iter().map(String::as_str).collect();
        c.verify_write_down(&refs).unwrap();
    }

    #[test]
    fn qr_rescan_verification() {
        let mut c = CommissionCeremony::begin().unwrap();
        // A different identity's QR does not verify.
        let other = BackupArtifacts::new(&generate_mnemonic(WordCount::Twelve).unwrap());
        assert!(matches!(
            c.verify_write_down_qr(other.qr_payload()),
            Err(CommissionError::WriteDownMismatch { attempts: 1 })
        ));
        // Garbage (wrong length) does not verify.
        assert!(matches!(
            c.verify_write_down_qr(&[1, 2, 3]),
            Err(CommissionError::WriteDownMismatch { attempts: 2 })
        ));
        assert_eq!(c.step(), CeremonyStep::Keygen);
        // The ceremony's own payload does.
        let payload = c.artifacts().unwrap().qr_payload().to_vec();
        c.verify_write_down_qr(&payload).unwrap();
        assert_eq!(c.step(), CeremonyStep::WriteDownVerified);
    }

    #[test]
    fn verify_write_down_is_one_shot() {
        let mut c = verified();
        assert!(matches!(
            c.verify_write_down(&["a", "b", "c", "d"]),
            Err(CommissionError::OutOfOrder {
                op: "verify_write_down",
                at: CeremonyStep::WriteDownVerified
            })
        ));
    }

    #[test]
    fn bundle_carries_keyset_descriptor_and_fresh_nonce() {
        let mut c = verified();
        let phrase = c.artifacts().unwrap().words().to_string();
        let derived = Keyset::derive_from_mnemonic(&phrase).unwrap().to_bytes();
        let b = c.provisioning_bundle().unwrap();
        assert_eq!(*b.keyset_bytes(), *derived);
        assert_eq!(b.descriptor(), BackupDescriptor::V1_12_WORDS);
        assert_eq!(b.node_id(), Keyset::from_bytes(&derived).unwrap().node_id());
        let n1 = *b.challenge_nonce();
        assert_eq!(b.summary().challenge_nonce, n1);
        assert_ne!(n1, [0u8; 32]);
        assert_eq!(c.step(), CeremonyStep::BundleIssued);

        // Re-issue (failed transfer) => a FRESH nonce, same keyset.
        let n2 = *c.provisioning_bundle().unwrap().challenge_nonce();
        assert_ne!(n1, n2);
    }

    #[test]
    fn challenge_signed_by_a_different_key_is_refused() {
        // (a) A misprovisioned relay: commits a rogue key, signs correctly with it.
        let mut c = verified();
        let (_m, rogue) = Keyset::generate(WordCount::Twelve).unwrap();
        let mut relay = LoopbackRelay::misprovisioned(rogue);
        match c.provision_via(&mut relay) {
            Err(CommissionError::ChallengeFailed(VerifyError::InvalidSignature)) => {}
            other => panic!("{other:?}"),
        }
        assert_ne!(relay.committed_node_id(), Some(c.node_id()));
        // Fails loudly: the ceremony does not complete, finish is refused.
        assert_eq!(c.step(), CeremonyStep::BundleIssued);
        assert_eq!(c.failed_challenges(), 1);
        assert!(matches!(
            c.finish(),
            Err(CommissionError::OutOfOrder {
                op: "finish",
                at: CeremonyStep::BundleIssued
            })
        ));
        let msg = CommissionError::ChallengeFailed(VerifyError::InvalidSignature).to_string();
        assert!(
            msg.contains("FAILED") && msg.contains("do NOT deploy"),
            "{msg}"
        );

        // (b) A hand-built signature from another key over the right message.
        let mut c = verified();
        let nonce = *c.provisioning_bundle().unwrap().challenge_nonce();
        let (_m2, other) = Keyset::generate(WordCount::Twelve).unwrap();
        let sig = other.sign_with_tag(CHALLENGE_TAG, &nonce);
        assert!(matches!(
            c.confirm_relay(&sig),
            Err(CommissionError::ChallengeFailed(_))
        ));
        assert_eq!(c.step(), CeremonyStep::BundleIssued);
    }

    #[test]
    fn challenge_is_domain_tag_and_nonce_bound() {
        let mut c = verified();
        let phrase = c.artifacts().unwrap().words().to_string();
        let key = Keyset::derive_from_mnemonic(&phrase).unwrap();
        let nonce1 = *c.provisioning_bundle().unwrap().challenge_nonce();

        // Right key, WRONG TAG (domain-tag confusion, §6.1) — refused.
        let wrong_tag = key.sign_with_tag(b"111/announce/v1", &nonce1);
        assert!(c.confirm_relay(&wrong_tag).is_err());
        // Right key, NO tag — refused.
        let untagged = key.sign_with_tag(b"", &nonce1);
        assert!(c.confirm_relay(&untagged).is_err());
        // Right key, right tag, WRONG NONCE — refused.
        let wrong_nonce = key.sign_with_tag(CHALLENGE_TAG, &[7u8; 32]);
        assert!(c.confirm_relay(&wrong_nonce).is_err());
        // A signature corrupted in transit — refused.
        let mut flipped = key.sign_with_tag(CHALLENGE_TAG, &nonce1);
        flipped[10] ^= 0x01;
        assert!(c.confirm_relay(&flipped).is_err());
        assert_eq!(c.failed_challenges(), 4);
        assert_eq!(c.step(), CeremonyStep::BundleIssued);

        // Replay: the signature over the OLD nonce fails once a fresh bundle
        // is issued; the signature over the new nonce passes.
        let old_sig = key.sign_with_tag(CHALLENGE_TAG, &nonce1);
        let nonce2 = *c.provisioning_bundle().unwrap().challenge_nonce();
        assert!(c.confirm_relay(&old_sig).is_err());
        c.confirm_relay(&key.sign_with_tag(CHALLENGE_TAG, &nonce2))
            .unwrap();
        assert_eq!(c.step(), CeremonyStep::ChallengeVerified);
        // Single-use: a second confirm is out of order.
        assert!(matches!(
            c.confirm_relay(&key.sign_with_tag(CHALLENGE_TAG, &nonce2)),
            Err(CommissionError::OutOfOrder { .. })
        ));
    }

    #[test]
    fn challenge_wire_message_is_tag_then_nonce() {
        // Pins the on-wire contract: the signed bytes are the ASCII tag
        // "111/commission-challenge/v1" immediately followed by the 32B nonce.
        assert_eq!(CHALLENGE_TAG, b"111/commission-challenge/v1");
        let key = Keyset::derive_from_mnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let nonce: [u8; 32] = std::array::from_fn(|i| i as u8);
        let sig = key.sign_with_tag(CHALLENGE_TAG, &nonce);
        let mut msg = b"111/commission-challenge/v1".to_vec();
        msg.extend_from_slice(&nonce);
        crate::keyset::verify_strict(&key.node_id(), &msg, &sig).unwrap();
    }

    #[test]
    fn relay_refuses_reprovisioning_once_committed() {
        let mut relay = LoopbackRelay::new();
        let mut first = verified();
        first.provision_via(&mut relay).unwrap();
        let committed = relay.committed_node_id().unwrap();

        let mut second = verified();
        match second.provision_via(&mut relay) {
            Err(CommissionError::Relay(RelayError::AlreadyCommitted)) => {}
            other => panic!("{other:?}"),
        }
        // Never overwritten.
        assert_eq!(relay.committed_node_id(), Some(committed));
        assert_ne!(second.node_id(), committed);
        // The failed ceremony can still be aborted cleanly.
        let abort = second.abort();
        assert_eq!(abort.reached, CeremonyStep::BundleIssued);
    }

    #[test]
    fn relay_rejects_a_weak_keyset_before_committing() {
        // A bundle carrying a weak (uniform) keyset must not be committed.
        let mut relay = LoopbackRelay::new();
        let c = verified();
        let bad = ProvisioningBundle {
            node_id: c.node_id(),
            descriptor: BackupDescriptor::V1_12_WORDS,
            keyset: Zeroizing::new([0u8; 64]),
            nonce: [9u8; 32],
        };
        assert!(matches!(
            relay.send_bundle(&bad),
            Err(RelayError::Rejected(_))
        ));
        assert!(!relay.is_committed());
        assert_eq!(
            relay.recv_challenge_sig(),
            Err(RelayError::NoChallengeSignature)
        );
    }

    #[test]
    fn loopback_signature_is_single_read_and_reports_channel() {
        let mut relay = LoopbackRelay::new();
        assert_eq!(relay.channel(), LocalChannel::HostLoopback);
        assert_eq!(
            relay.recv_challenge_sig(),
            Err(RelayError::NoChallengeSignature)
        );
        let mut c = verified();
        c.provision_via(&mut relay).unwrap();
        // provision_via consumed the signature already.
        assert_eq!(
            relay.recv_challenge_sig(),
            Err(RelayError::NoChallengeSignature)
        );
    }

    #[test]
    fn finish_requires_challenge_verified() {
        let mut c = verified();
        assert!(matches!(
            c.finish(),
            Err(CommissionError::OutOfOrder { .. })
        ));
        c.provisioning_bundle().unwrap();
        assert!(matches!(
            c.finish(),
            Err(CommissionError::OutOfOrder { .. })
        ));
    }

    #[test]
    fn finish_wipes_and_the_record_carries_no_key_material() {
        let mut c = CommissionCeremony::begin().unwrap();
        let phrase = c.artifacts().unwrap().words().to_string();
        let entropy_hex = hex(&c.artifacts().unwrap().qr_payload()[4..]);
        let qr_hex = hex(c.artifacts().unwrap().qr_payload());
        let keyset_hex = hex(&Keyset::derive_from_mnemonic(&phrase).unwrap().to_bytes()[..]);
        let seed_half_hex = &keyset_hex[..64];
        pass_write_down(&mut c);
        c.provision_via(&mut LoopbackRelay::new()).unwrap();
        let record = c.finish().unwrap();

        // The record is public values only.
        let surfaces = [format!("{record:?}"), record.log_line(), format!("{c:?}")];
        for s in &surfaces {
            assert!(!s.contains(&phrase), "words leaked: {s}");
            assert!(!s.contains(&keyset_hex), "keyset leaked: {s}");
            assert!(!s.contains(seed_half_hex), "signing seed leaked: {s}");
            assert!(!s.contains(&entropy_hex), "entropy leaked: {s}");
            assert!(!s.contains(&qr_hex), "QR payload leaked: {s}");
            // No two adjacent phrase words appear together (single words can
            // coincide with template text such as "label"; pairs cannot).
            let words: Vec<&str> = phrase.split(' ').collect();
            for pair in words.windows(2) {
                let joined = pair.join(" ");
                assert!(!s.contains(&joined), "words leaked: {s}");
            }
        }
        let d = NodeIdDisplay::new(record.node_id.as_bytes());
        assert!(record.log_line().contains(&d.hex()));
        assert!(record.log_line().contains("verified_at=challenge-verified"));

        // The ceremony can produce nothing after finish().
        assert_eq!(c.step(), CeremonyStep::Wiped);
        assert!(matches!(
            c.provisioning_bundle(),
            Err(CommissionError::Wiped)
        ));
        assert!(matches!(c.artifacts(), Err(CommissionError::Wiped)));
        assert!(matches!(
            c.verify_write_down(&["a", "b", "c", "d"]),
            Err(CommissionError::Wiped)
        ));
        assert!(matches!(
            c.verify_write_down_qr(&[0; 20]),
            Err(CommissionError::Wiped)
        ));
        assert!(matches!(
            c.confirm_relay(&[0; 64]),
            Err(CommissionError::Wiped)
        ));
        assert!(matches!(c.finish(), Err(CommissionError::Wiped)));
        assert!(matches!(
            c.provision_via(&mut LoopbackRelay::new()),
            Err(CommissionError::Wiped)
        ));
        assert!(c.write_down_positions().is_empty());
        assert!(c.secrets.is_none());
        // Public identity survives (it is the record's content).
        assert_eq!(c.node_id(), record.node_id);
    }

    #[test]
    fn wipe_overwrites_keyset_and_bundle_in_place() {
        let mut c = verified();
        c.provisioning_bundle().unwrap();
        let secrets = c.secrets.as_mut().unwrap();
        assert_ne!(*secrets.keyset.to_bytes(), [0u8; 64]);
        assert_ne!(*secrets.bundle.as_ref().unwrap().keyset_bytes(), [0u8; 64]);
        secrets.wipe_in_place();
        assert_eq!(
            *secrets.keyset.to_bytes(),
            [0u8; 64],
            "keyset zeroized in place"
        );
        let b = secrets.bundle.as_ref().unwrap();
        assert_eq!(
            *b.keyset_bytes(),
            [0u8; 64],
            "bundle keyset zeroized in place"
        );
        assert_eq!(
            *b.challenge_nonce(),
            [0u8; 32],
            "bundle nonce zeroized in place"
        );
    }

    #[test]
    fn successful_confirm_drops_the_transit_copy_immediately() {
        let mut c = verified();
        c.provision_via(&mut LoopbackRelay::new()).unwrap();
        assert_eq!(c.step(), CeremonyStep::ChallengeVerified);
        assert!(c.secrets.as_ref().unwrap().bundle.is_none());
        assert!(matches!(
            c.provisioning_bundle(),
            Err(CommissionError::OutOfOrder {
                op: "provisioning_bundle",
                ..
            })
        ));
    }

    #[test]
    fn abort_wipes_at_any_step_and_reports_how_far_it_got() {
        let mut c = CommissionCeremony::begin().unwrap();
        let node = c.node_id();
        let a = c.abort();
        assert_eq!(a.reached, CeremonyStep::Keygen);
        assert_eq!(a.node_id, node);
        assert!(a.log_line().contains("reached=keygen"));
        assert!(matches!(c.artifacts(), Err(CommissionError::Wiped)));
        assert!(matches!(
            c.provisioning_bundle(),
            Err(CommissionError::Wiped)
        ));

        let mut c = verified();
        c.provisioning_bundle().unwrap();
        let a = c.abort();
        assert_eq!(a.reached, CeremonyStep::BundleIssued);
        assert!(c.secrets.is_none());
    }

    #[test]
    fn debug_impls_redact() {
        let mut c = verified();
        let phrase = c.artifacts().unwrap().words().to_string();
        let keyset_hex = hex(&Keyset::derive_from_mnemonic(&phrase).unwrap().to_bytes()[..]);
        let dbg_c = format!("{c:?}");
        let bundle = c.provisioning_bundle().unwrap();
        let dbg_b = format!("{bundle:?}");
        for s in [dbg_c, dbg_b] {
            assert!(s.contains("redacted"), "{s}");
            assert!(!s.contains(&keyset_hex) && !s.contains(&phrase), "{s}");
        }
    }

    #[test]
    fn nonce_weak_value_gate_and_freshness() {
        assert!(matches!(
            reject_weak_nonce(&[0u8; 32]),
            Err(CommissionError::WeakNonce)
        ));
        assert!(matches!(
            reject_weak_nonce(&[0xAB; 32]),
            Err(CommissionError::WeakNonce)
        ));
        let mut ok = [0u8; 32];
        ok[31] = 1;
        assert!(reject_weak_nonce(&ok).is_ok());
        assert_ne!(fresh_nonce().unwrap(), fresh_nonce().unwrap());
    }

    #[test]
    fn two_ceremonies_never_share_an_identity() {
        // §4 rule 6 / rule 1: every ceremony generates its own keyset.
        let a = CommissionCeremony::begin().unwrap();
        let b = CommissionCeremony::begin().unwrap();
        assert_ne!(a.node_id(), b.node_id());
        assert_ne!(
            a.artifacts().unwrap().words(),
            b.artifacts().unwrap().words()
        );
    }

    #[test]
    fn dropping_an_unfinished_ceremony_is_safe() {
        let mut c = verified();
        c.provisioning_bundle().unwrap();
        drop(c); // Drop => wipe(); must not panic
    }

    #[test]
    fn from_generated_parity_gate_accepts_a_consistent_pair() {
        // The parity gate also holds for a hand-supplied consistent pair.
        let m = generate_mnemonic(WordCount::TwentyFour).unwrap();
        let k = Keyset::derive_from_root(&m).unwrap();
        let c = CommissionCeremony::from_generated(m, k).unwrap();
        assert_eq!(c.descriptor(), BackupDescriptor::V1_24_WORDS);
    }
}
