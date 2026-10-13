//! Identity binding (L1 spec v2.2, D14(d)) — a THIN seam over `id111`, Layer 2's
//! identity core (`os/identity/README.md` v2, §13). Everything cryptographic —
//! BIP-39, SLIP-0010, the dual Ed25519+X25519 keyset, §7 encodings, the §13
//! atomic file-set writer, §8.4 restore rules, the D9 migration — lives in
//! `id111`; this module only:
//!
//!  1. maps id111's results onto Layer 1's three error classes
//!     ([`IdentityError`]: Missing / Corrupt / Exists) and the boot-quarantine
//!     semantics the daemon and installer already depend on;
//!  2. enforces the §8.3 TTY-only rule for `backup` (words never reach a pipe);
//!  3. triggers the one-time D9 → L2 migration on the first `load`.
//!
//! On disk (`identity/`, mode 0700, owner node111 — the caller chowns), the
//! §13 file set, written atomically by id111 (temp dir, then rename):
//!   `mnemonic` 0600 · `keyset` 0600 (64B) · `nodeid` 0644 (34B record) ·
//!   `descriptor` 0644 (4B) · `counters` 0600 (id111 `counters` module owns the
//!   byte format; `peers` appears on first pin). Never overwritten.
//!
//! Boot integrity: `load` cross-checks the whole set (keyset ↔ nodeid ↔
//! mnemonic ↔ descriptor). Any inconsistency quarantines the dir
//! (`identity.quarantine-<epoch>`) and returns `IdentityError::Corrupt` — never
//! a silent re-key; explicit `111 identity restore` is the only way forward.
//! `load` deliberately does NOT open the `counters` file through id111's
//! `CounterStore`: opening mutates it (dirty/running marker, §6.5 unclean-
//! reboot jump), and install-time `load` runs as root — it would re-own the
//! file away from node111. The counters byte format is validated by whoever
//! owns the store at runtime (Layer 6/7); here it must exist and be non-empty.

pub mod commission;

use id111::backup::{
    self, BackupArtifacts, BackupDescriptor, CounterFloor, RestoreError, RestoreOptions,
    RestoreOutcome, COUNTERS_FILE, DESCRIPTOR_FILE, DESCRIPTOR_LEN, KEYSET_FILE, MNEMONIC_FILE,
    NODEID_FILE,
};
use id111::bip39::{RootMnemonic, WordCount};
use id111::backup::create_fresh_identity;
use id111::encoding::{self, KeyKind, NodeIdDisplay};
use id111::keyset::{Keyset, NodeId};
use id111::migrate::{self, MigrationError, LEGACY_NPUB_FILE, LEGACY_SECRET_FILE};
use std::fmt;
use std::path::{Path, PathBuf};

/// The five §13 files every complete identity dir holds at install time.
/// (`mnemonic` alone may later be shredded on hubs, §12 residuals — see
/// [`read_verified`].)
const SET_FILES: [&str; 5] = [
    MNEMONIC_FILE,
    KEYSET_FILE,
    NODEID_FILE,
    DESCRIPTOR_FILE,
    COUNTERS_FILE,
];

/// D9-legacy-only files (the provisional secp256k1 layout shared only
/// `mnemonic` with §13). Their presence routes `load` into the migration.
const LEGACY_ONLY: [&str; 2] = [LEGACY_SECRET_FILE, LEGACY_NPUB_FILE];

/// What a successful D9 → L2 migration did (carried on the loaded
/// [`Identity`] so callers/tests can see it; the binding has already logged it).
#[derive(Debug, Clone)]
pub struct MigrationNote {
    /// The old→new mapping exactly as logged (id111's `MigrationOutcome`).
    pub mapping: String,
    /// Where the legacy dir was quarantined (`<dir>.quarantine-d9[-n]`).
    pub quarantine_dir: PathBuf,
}

/// The loaded node identity: the public NodeId plus the keyset handle.
/// Secret bytes live only inside [`Keyset`] (zeroized on drop, redacted Debug).
#[derive(Debug)]
pub struct Identity {
    /// The NodeId — the raw Ed25519 public key (§2 B2).
    pub node_id: NodeId,
    keyset: Keyset,
    /// `Some` only on the load that performed the one-time D9 migration.
    pub migration: Option<MigrationNote>,
}

impl Identity {
    /// §7 machine string: lowercase hex of the 32 raw bytes (APIs, logs,
    /// receipt, `/api/status`).
    pub fn node_id_hex(&self) -> String {
        encoding::hex_encode(self.node_id.as_bytes())
    }

    /// The full §7 presentation surface (hex · bech32 `node1…` · label).
    pub fn display(&self) -> NodeIdDisplay {
        NodeIdDisplay::new(self.node_id.as_bytes())
    }

    /// The keyset handle (signing/ECDH live in id111; v1 has no consumer yet —
    /// the signer-daemon seam is Layer 3's).
    pub fn keyset(&self) -> &Keyset {
        &self.keyset
    }
}

/// Distinguishable failure classes — the daemon maps these to distinct
/// dashboard/`/api/status` error states.
#[derive(Debug)]
pub enum IdentityError {
    /// No identity directory (fresh machine before install finishes).
    Missing,
    /// File set unreadable/inconsistent → quarantined. Recovery = `restore`.
    Corrupt(String),
    /// Refused to overwrite an existing identity.
    Exists,
    /// A quarantined identity sibling exists next to the (now missing) dir —
    /// §13 "never silent re-key": regeneration is only via explicit
    /// `111 identity restore` (from the recorded words) or an explicit burn
    /// (`111 identity generate --burn`: new mnemonic, new NodeId, §8.4
    /// runbook). The payload is the quarantine path.
    QuarantinePresent(PathBuf),
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdentityError::Missing => write!(f, "identity directory missing"),
            IdentityError::Corrupt(why) => write!(f, "identity corrupt (quarantined): {why}"),
            IdentityError::Exists => write!(f, "identity exists — never overwritten"),
            IdentityError::QuarantinePresent(q) => write!(
                f,
                "a quarantined identity exists at {} — refusing to silently generate a new one (§13). \
                 Recover it with `111 identity restore` (your recorded words), or burn it explicitly \
                 with `111 identity generate --burn` (new mnemonic, NEW NodeId — peers will see a new node)",
                q.display()
            ),
        }
    }
}

impl std::error::Error for IdentityError {}

/// Is this error the fresh-machine `Missing` class (the only class the
/// installer / on-device keygen answer with a silent `generate`)?
pub fn is_missing(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<IdentityError>(),
        Some(IdentityError::Missing)
    )
}

/// Bytes read from a secret-bearing file, wiped on drop (best effort). node111
/// carries no `zeroize` dep (frozen set); id111's own types zeroize themselves.
struct Secret(Vec<u8>);

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.fill(0);
        std::hint::black_box(&self.0);
    }
}

/// Parse a §7 machine-string NodeId (64 hex chars) — used to validate what the
/// phone's on-device keygen reports back.
pub fn parse_node_id_hex(s: &str) -> Option<NodeId> {
    encoding::hex_decode(s).ok().map(NodeId::from)
}

/// The §7 human-transfer form (`node1…`, bech32) of a machine-string NodeId.
pub fn node_id_bech32(hex: &str) -> Option<String> {
    parse_node_id_hex(hex).map(|n| encoding::bech32_encode(n.as_bytes()))
}

/// Parse the optional `descriptor` operator input (8 hex digits, §8.2).
/// Empty ⇒ `None` (restore assumes the v1 defaults). A present descriptor is
/// parsed through id111, so any unknown field fails closed right here.
pub fn parse_descriptor_hex(input: &str) -> anyhow::Result<Option<[u8; DESCRIPTOR_LEN]>> {
    let cleaned: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    if cleaned.is_empty() {
        return Ok(None);
    }
    anyhow::ensure!(
        cleaned.len() == DESCRIPTOR_LEN * 2 && cleaned.chars().all(|c| c.is_ascii_hexdigit()),
        "descriptor must be {} hex digits (printed under the words), got {input:?}",
        DESCRIPTOR_LEN * 2
    );
    let mut bytes = [0u8; DESCRIPTOR_LEN];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&cleaned[i * 2..i * 2 + 2], 16)?;
    }
    BackupDescriptor::from_bytes(&bytes).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(Some(bytes))
}

fn any_file(dir: &Path, names: &[&str]) -> bool {
    names.iter().any(|n| dir.join(n).exists())
}

/// True iff `dir` holds any identity material (§13 set or D9-legacy files) —
/// the §8.4 "live, unquarantined" test, mirrored from id111 so the CLI can
/// refuse BEFORE asking the operator for words. id111's `restore` stays the
/// authority (it re-checks atomically).
pub fn is_live(dir: &Path) -> bool {
    any_file(dir, &SET_FILES) || any_file(dir, &LEGACY_ONLY)
}

/// Rename a corrupt dir aside (`identity.quarantine-<epoch>`). Best-effort, as
/// in L1: a failed rename still returns Corrupt — the node never re-keys.
fn quarantine(dir: &Path) {
    let quarantine = dir.with_file_name(format!(
        "identity.quarantine-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    ));
    let _ = std::fs::rename(dir, &quarantine);
}

fn quarantine_if_corrupt(dir: &Path, e: anyhow::Error) -> anyhow::Error {
    if matches!(
        e.downcast_ref::<IdentityError>(),
        Some(IdentityError::Corrupt(_))
    ) {
        quarantine(dir);
    }
    e
}

/// Everything `read_verified` cross-checked.
struct Verified {
    node_id: NodeId,
    keyset: Keyset,
    /// `None` only when the `mnemonic` file is absent (hubs MAY shred it after
    /// a verified backup, spec §12 — the keyset/nodeid still verify).
    mnemonic: Option<RootMnemonic>,
}

/// Read + cross-check the §13 file set: keyset (self-test) ↔ nodeid record ↔
/// mnemonic (re-derived keyset must equal the cache) ↔ descriptor (fail
/// closed, word-count agreement) ↔ counters present. Pure read — no side effects.
fn read_verified(dir: &Path) -> Result<Verified, IdentityError> {
    if !dir.exists() || !(any_file(dir, &SET_FILES) || any_file(dir, &LEGACY_ONLY)) {
        return Err(IdentityError::Missing);
    }
    let corrupt = |why: String| IdentityError::Corrupt(why);
    if any_file(dir, &LEGACY_ONLY) {
        return Err(corrupt(
            "D9 legacy layout not migrated yet (the first `load` by the daemon/installer migrates it)"
                .into(),
        ));
    }
    let read = |name: &str| -> Result<Secret, IdentityError> {
        std::fs::read(dir.join(name))
            .map(Secret)
            .map_err(|e| corrupt(format!("{name} unreadable: {e}")))
    };

    // keyset — exactly 64B, §4.5 self-test inside Keyset::from_bytes.
    let keyset_bytes = read(KEYSET_FILE)?;
    let mut fixed: [u8; 64] = keyset_bytes
        .0
        .as_slice()
        .try_into()
        .map_err(|_| corrupt(format!("keyset is {}B, expected 64", keyset_bytes.0.len())))?;
    let keyset = Keyset::from_bytes(&fixed).map_err(|e| corrupt(format!("keyset invalid: {e}")));
    fixed.fill(0);
    std::hint::black_box(&fixed);
    let keyset = keyset?;

    // nodeid — §7 34B record (ed25519-pub), must equal the keyset's NodeId.
    let record = read(NODEID_FILE)?;
    let (kind, key) = encoding::parse_record(&record.0)
        .map_err(|e| corrupt(format!("nodeid record invalid: {e}")))?;
    if kind != KeyKind::Ed25519Pub {
        return Err(corrupt("nodeid record is not an ed25519-pub record".into()));
    }
    let node_id = keyset.node_id();
    if node_id.as_bytes() != &key {
        return Err(corrupt("nodeid does not match keyset".into()));
    }

    // descriptor — fail closed on any unknown field.
    let descriptor = BackupDescriptor::from_bytes(&read(DESCRIPTOR_FILE)?.0)
        .map_err(|e| corrupt(format!("descriptor invalid: {e}")))?;

    // mnemonic — optional (shredded), but if present it must reproduce the
    // cached keyset and agree with the descriptor's word count.
    let mnemonic = if dir.join(MNEMONIC_FILE).exists() {
        let raw = read(MNEMONIC_FILE)?;
        let words =
            std::str::from_utf8(&raw.0).map_err(|_| corrupt("mnemonic is not UTF-8".into()))?;
        let m = RootMnemonic::parse(words.trim())
            .map_err(|e| corrupt(format!("mnemonic chain: {e}")))?;
        let rederived =
            Keyset::derive_from_root(&m).map_err(|e| corrupt(format!("mnemonic chain: {e}")))?;
        if rederived.to_bytes()[..] != keyset_bytes.0[..] {
            return Err(corrupt("keyset does not match mnemonic".into()));
        }
        if descriptor.word_count().word_count() != m.word_count() {
            return Err(corrupt(
                "descriptor contradicts the mnemonic's word count".into(),
            ));
        }
        Some(m)
    } else {
        None
    };

    // counters — presence only (see module docs: never opened via CounterStore here).
    match std::fs::metadata(dir.join(COUNTERS_FILE)) {
        Ok(m) if m.is_file() && m.len() > 0 => {}
        Ok(_) => return Err(corrupt("counters is empty or not a file".into())),
        Err(e) => return Err(corrupt(format!("counters unreadable: {e}"))),
    }

    Ok(Verified {
        node_id,
        keyset,
        mnemonic,
    })
}

/// D9 → L2 migration (§8.4, D14d). Runs only when a D9-legacy-only file is
/// present. Fail-closed outcomes (partial/mixed legacy dir, bad legacy words)
/// are `Corrupt` ⇒ the caller quarantines; a plain I/O failure is NOT a
/// verdict on the identity, so it surfaces as an ordinary error with no
/// quarantine (id111 already rolled its own rename back best-effort).
fn migrate_if_legacy(dir: &Path) -> anyhow::Result<Option<MigrationNote>> {
    if !any_file(dir, &LEGACY_ONLY) {
        return Ok(None);
    }
    match migrate::migrate(dir) {
        Ok(outcome) => {
            // The old→new mapping, logged once (the event, never the secret).
            // `{outcome:?}` carries the legacy handle, the new NodeId, the
            // quarantine path and the written descriptor.
            let mapping = format!(
                "identity migrated D9→L2 (same words, new derivation; one-time NodeId change): {outcome:?}"
            );
            eprintln!("111: {mapping}");
            Ok(Some(MigrationNote {
                mapping,
                quarantine_dir: outcome.quarantine_dir,
            }))
        }
        Err(MigrationError::NotLegacyDir) => Ok(None),
        Err(MigrationError::Io(e)) => Err(anyhow::anyhow!(
            "D9 identity migration failed (I/O; nothing quarantined): {e}"
        )),
        Err(e) => Err(IdentityError::Corrupt(format!("legacy D9 identity: {e}")).into()),
    }
}

/// Load + integrity-check. A D9-format dir is migrated first (see above).
/// Corruption ⇒ quarantine dir + `Corrupt` (never silent re-key; `restore` is
/// the explicit path).
pub fn load(dir: &Path) -> anyhow::Result<Identity> {
    let migration = migrate_if_legacy(dir).map_err(|e| quarantine_if_corrupt(dir, e))?;
    match read_verified(dir) {
        Ok(v) => Ok(Identity {
            node_id: v.node_id,
            keyset: v.keyset,
            migration,
        }),
        Err(e) => Err(quarantine_if_corrupt(dir, e.into())),
    }
}

fn map_restore_err(e: RestoreError) -> anyhow::Error {
    match e {
        RestoreError::LiveIdentityDir => IdentityError::Exists.into(),
        other => anyhow::anyhow!("{other}"),
    }
}

/// Silent install-time keygen (Cat 2+, §8.3): id111 entropy-gated keygen (§4:
/// `getrandom()`, §4.5 self-test) → 12-word BIP-39 → SLIP-0010 keyset →
/// id111's atomic §13 writer. Never overwrites (`IdentityError::Exists`).
/// Caller chowns the dir to node111 afterwards.
///
/// Writer note: id111's atomic identity-dir writer is crate-private; its public
/// entry that reaches it is `backup::restore`. A fresh identity therefore goes
/// through restore with `CounterFloor::NeverDeployed`, so `counters` starts at
/// the §8.4 `0 + 1024` instead of the §6.2 fresh `0` — harmless (over-jumping
/// only advances windows; v1 emits no announces). A public fresh-keygen writer
/// in id111 would remove this wrinkle (reported to the reviewer).
pub fn generate(dir: &Path) -> anyhow::Result<Identity> {
    if let Some(q) = quarantine_sibling(dir) {
        return Err(IdentityError::QuarantinePresent(q).into());
    }
    generate_burn(dir)
}

/// §8.4 burn runbook — generate a NEW identity even though a quarantined one
/// sits next to the dir (explicit operator intent only: `--burn`). The
/// quarantine is never deleted.
pub fn generate_burn(dir: &Path) -> anyhow::Result<Identity> {
    let (mnemonic, keyset) =
        Keyset::generate(WordCount::Twelve).map_err(|e| anyhow::anyhow!("keygen: {e}"))?;
    // §4/§13 fresh writer: full file set, announce counter 0 (NOT the §8.4
    // restore path, which would persist the restore-only floor + 1024).
    create_fresh_identity(dir, &mnemonic, &keyset).map_err(map_restore_err)?;
    Ok(Identity {
        node_id: keyset.node_id(),
        keyset,
        migration: None,
    })
}

/// A quarantined sibling of `dir` (`identity.quarantine-<epoch>` from the boot
/// integrity check, or `identity.quarantine-d9[-n]` from the D9 migration),
/// if any. Its presence means an identity existed here: silent regeneration
/// is forbidden (§13).
pub fn quarantine_sibling(dir: &Path) -> Option<PathBuf> {
    let parent = dir.parent()?;
    let stem = dir.file_name()?.to_string_lossy().into_owned();
    let prefix = format!("{stem}.quarantine-");
    let mut found: Vec<PathBuf> = std::fs::read_dir(parent)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().starts_with(&prefix))
                .unwrap_or(false)
        })
        .collect();
    found.sort();
    found.pop()
}

/// Everything `111 identity backup` shows: the zeroizing ceremony artifacts
/// (words + §8.2 descriptor + QR payload bytes) and the NodeId reference the
/// operator records for the §8.4 restore confirmation.
pub struct BackupView {
    pub artifacts: BackupArtifacts,
    pub node_id: NodeId,
}

impl BackupView {
    pub fn display(&self) -> NodeIdDisplay {
        NodeIdDisplay::new(self.node_id.as_bytes())
    }
}

/// `identity backup`: the words for the local TTY ceremony — never HTTP.
/// §8.3 (normative): MUST refuse non-TTY stdout (words on piped stdout land in
/// CI logs and shell history). Refuses to back up a corrupt dir (no quarantine
/// side effect — only `load` quarantines).
pub fn backup(dir: &Path) -> anyhow::Result<BackupView> {
    anyhow::ensure!(
        unsafe { libc::isatty(1) } == 1,
        "identity backup refuses to run unless stdout is a terminal \
         (the words would land in logs/shell history) — run it interactively"
    );
    backup_unchecked(dir)
}

/// The backup read with the TTY gate already applied by the caller.
fn backup_unchecked(dir: &Path) -> anyhow::Result<BackupView> {
    let v = read_verified(dir)?;
    let mnemonic = v.mnemonic.ok_or_else(|| {
        anyhow::anyhow!(
            "the mnemonic file is not on disk (shredded after a verified backup?) — \
             nothing left to display; the paper backup is the only copy"
        )
    })?;
    Ok(BackupView {
        artifacts: BackupArtifacts::new(&mnemonic),
        node_id: v.node_id,
    })
}

/// §8.4 pre-write step for the confirmation prompt: input hygiene (NFKC, case/
/// whitespace folded), BIP-39 checksum MUST validate, SLIP-0010 derivation —
/// returns the NodeId the operator confirms against their recorded reference.
/// Writes nothing.
pub fn preview_restore(words: &str) -> anyhow::Result<NodeId> {
    let keyset = Keyset::derive_from_mnemonic(words).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(keyset.node_id())
}

/// `identity restore` (§8.4): id111 `backup::restore` — checksum MUST validate,
/// descriptor fail-closed (when the operator supplies it; absent ⇒ v1
/// defaults), refuses a live unquarantined dir (`IdentityError::Exists` — the
/// quarantined or absent dir is fine), atomic §13 write. The counter floor is
/// `PendingMeshLearn` (words-only recovery: the floor is unknown ⇒ the node
/// MUST NOT announce until it is learned; v1 emits no announces). The caller
/// has already shown [`preview_restore`]'s NodeId and collected both
/// confirmations; `old_instance_retired` carries the §8.4 old-instance
/// retirement confirmation. Returns the outcome incl. the restored NodeId.
pub fn restore(
    dir: &Path,
    words: &str,
    descriptor: Option<[u8; DESCRIPTOR_LEN]>,
    old_instance_retired: bool,
) -> anyhow::Result<RestoreOutcome> {
    backup::restore(
        dir,
        words,
        &RestoreOptions {
            old_instance_retired,
            descriptor,
            counter_floor: CounterFloor::PendingMeshLearn,
        },
    )
    .map_err(map_restore_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const V1_WORDS: &str =
        "leader monkey parrot ring guide accident before fence cannon height naive bean";
    const V2_WORDS: &str = "what bleak badge arrange retreat wolf trade produce cricket blur garlic valid proud rude strong choose busy staff weather area salt hollow arm fade";
    const BAD_CHECKSUM: &str =
        "leader monkey parrot ring guide accident before fence cannon height naive abandon";

    fn id_dir() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("identity");
        (t, d)
    }

    fn mode(p: &Path) -> u32 {
        p.metadata().unwrap().permissions().mode() & 0o777
    }

    fn expected_node_id(words: &str) -> NodeId {
        Keyset::derive_from_mnemonic(words).unwrap().node_id()
    }

    fn quarantined(parent: &Path, prefix: &str) -> Vec<PathBuf> {
        std::fs::read_dir(parent)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
            .map(|e| e.path())
            .collect()
    }

    /// Legacy-shaped (D9) dir: the provisional trio. The legacy secret is never
    /// read by migration, so any bytes do.
    fn make_legacy_dir(dir: &Path, words: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(dir.join(LEGACY_SECRET_FILE), "7f7ff03d\n").unwrap();
        std::fs::write(dir.join(LEGACY_NPUB_FILE), "legacy-handle-0001\n").unwrap();
        std::fs::write(dir.join(MNEMONIC_FILE), format!("{words}\n")).unwrap();
    }

    #[test]
    fn generate_writes_the_section_13_file_set_with_perms() {
        let (_t, dir) = id_dir();
        let id = generate(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join(MNEMONIC_FILE)), 0o600);
        assert_eq!(mode(&dir.join(KEYSET_FILE)), 0o600);
        assert_eq!(mode(&dir.join(NODEID_FILE)), 0o644);
        assert_eq!(mode(&dir.join(DESCRIPTOR_FILE)), 0o644);
        assert_eq!(mode(&dir.join(COUNTERS_FILE)), 0o600);
        // legacy shape is gone
        assert!(!dir.join(LEGACY_SECRET_FILE).exists());
        assert!(!dir.join(LEGACY_NPUB_FILE).exists());
        // 34B record 0xed 0x01 ‖ NodeId; descriptor = v1 / dual / SLIP-0010 / 12 words
        let rec = std::fs::read(dir.join(NODEID_FILE)).unwrap();
        assert_eq!(rec.len(), 34);
        assert_eq!(&rec[..2], &[0xed, 0x01]);
        assert_eq!(&rec[2..], id.node_id.as_bytes());
        assert_eq!(
            std::fs::read(dir.join(DESCRIPTOR_FILE)).unwrap(),
            vec![0x01, 0x01, 0x01, 0x01]
        );
        assert_eq!(std::fs::read(dir.join(KEYSET_FILE)).unwrap().len(), 64);
        // 12 words, hex form is the §7 machine string
        let words = std::fs::read_to_string(dir.join(MNEMONIC_FILE)).unwrap();
        assert_eq!(words.split_whitespace().count(), 12);
        assert!(words.ends_with('\n'));
        let hex = id.node_id_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
        // keygen NodeId == derivation of the written words
        assert_eq!(id.node_id, expected_node_id(words.trim()));
        // load round-trip: same NodeId, no migration
        let loaded = load(&dir).unwrap();
        assert_eq!(loaded.node_id, id.node_id);
        assert!(loaded.migration.is_none());
        // handle shape
        assert!(id.display().bech32().starts_with("node1"));
        assert_eq!(node_id_bech32(&hex).unwrap(), id.display().bech32());
    }

    #[test]
    fn restore_matches_id111_derivation_and_roundtrips() {
        let (_t, dir) = id_dir();
        let out = restore(&dir, V1_WORDS, None, true).unwrap();
        assert_eq!(out.node_id, expected_node_id(V1_WORDS));
        assert!(out.counter_floor_pending); // words-only recovery: floor unknown
        let loaded = load(&dir).unwrap();
        assert_eq!(loaded.node_id, out.node_id);
        // backup returns the words (TTY gate applied by the public wrapper)
        let view = backup_unchecked(&dir).unwrap();
        assert_eq!(view.artifacts.words(), V1_WORDS);
        assert_eq!(view.node_id, out.node_id);
        // preview (no writes) agrees with what restore produced
        assert_eq!(preview_restore(V1_WORDS).unwrap(), out.node_id);
    }

    #[test]
    fn restore_24_words_uses_the_24_word_descriptor() {
        let (_t, dir) = id_dir();
        restore(&dir, V2_WORDS, None, true).unwrap();
        assert_eq!(
            std::fs::read(dir.join(DESCRIPTOR_FILE)).unwrap(),
            vec![0x01, 0x01, 0x01, 0x02]
        );
        assert_eq!(load(&dir).unwrap().node_id, expected_node_id(V2_WORDS));
    }

    #[test]
    fn never_overwrite() {
        let (_t, dir) = id_dir();
        let first = generate(&dir).unwrap();
        // generate over a live set
        let err = generate(&dir).unwrap_err();
        assert!(err.to_string().contains("never overwritten"), "{err}");
        assert!(matches!(
            err.downcast_ref::<IdentityError>(),
            Some(IdentityError::Exists)
        ));
        // restore over a live set — §8.4 refusal, same class
        let err = restore(&dir, V1_WORDS, None, true).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<IdentityError>(),
            Some(IdentityError::Exists)
        ));
        assert!(is_live(&dir));
        // untouched
        assert_eq!(load(&dir).unwrap().node_id, first.node_id);
    }

    #[test]
    fn restore_refuses_bad_input_and_writes_nothing() {
        let (_t, dir) = id_dir();
        // BIP-39 checksum MUST validate
        let err = restore(&dir, BAD_CHECKSUM, None, true).unwrap_err();
        assert!(!err.to_string().is_empty());
        assert!(preview_restore(BAD_CHECKSUM).is_err());
        assert!(!dir.exists());
        // old-instance retirement not confirmed
        assert!(restore(&dir, V1_WORDS, None, false).is_err());
        assert!(!dir.exists());
        // descriptor fail-closed: unknown version, and 24-word descriptor over 12 words
        assert!(restore(&dir, V1_WORDS, Some([0x09, 0x01, 0x01, 0x01]), true).is_err());
        assert!(restore(&dir, V1_WORDS, Some([0x01, 0x01, 0x01, 0x02]), true).is_err());
        assert!(!dir.exists());
        // a matching explicit descriptor is honored
        restore(&dir, V1_WORDS, Some([0x01, 0x01, 0x01, 0x01]), true).unwrap();
    }

    #[test]
    fn descriptor_hex_input() {
        assert_eq!(parse_descriptor_hex("").unwrap(), None);
        assert_eq!(parse_descriptor_hex("  \n").unwrap(), None);
        assert_eq!(
            parse_descriptor_hex("01 01 01 02").unwrap(),
            Some([1, 1, 1, 2])
        );
        assert_eq!(
            parse_descriptor_hex("01010101").unwrap(),
            Some([1, 1, 1, 1])
        );
        assert!(parse_descriptor_hex("0101").is_err()); // wrong length
        assert!(parse_descriptor_hex("zz010101").is_err()); // not hex
        assert!(parse_descriptor_hex("02010101").is_err()); // unknown version: fail closed
    }

    #[test]
    fn corrupt_trio_quarantines_and_never_rekeys() {
        // (Name kept: os/bootstrap/verify/README.md's F4 row cites this test.)
        let (t, dir) = id_dir();
        let id = restore(&dir, V1_WORDS, None, true).unwrap();
        // corrupt the keyset cache so it no longer matches the mnemonic
        let mut ks = std::fs::read(dir.join(KEYSET_FILE)).unwrap();
        ks[0] ^= 0xff;
        std::fs::write(dir.join(KEYSET_FILE), ks).unwrap();
        let err = load(&dir).unwrap_err();
        assert!(err.to_string().contains("corrupt"), "{err}");
        assert!(matches!(
            err.downcast_ref::<IdentityError>(),
            Some(IdentityError::Corrupt(_))
        ));
        // dir quarantined aside, no silent replacement
        assert!(!dir.exists());
        assert_eq!(quarantined(t.path(), "identity.quarantine-").len(), 1);
        // a second boot sees Missing, not a fresh key — load never generates
        let err = load(&dir).unwrap_err();
        assert!(is_missing(&err), "{err}");
        assert!(!dir.exists());
        // explicit restore is the way forward, with the same words → same NodeId
        let again = restore(&dir, V1_WORDS, None, true).unwrap();
        assert_eq!(again.node_id, id.node_id);
        assert_eq!(load(&dir).unwrap().node_id, id.node_id);
    }

    #[test]
    fn quarantine_sibling_blocks_silent_generate_until_explicit_burn() {
        // §13 "never silent re-key" + §8.4 burn runbook (review fix, wave 3):
        // after a corrupt identity is quarantined, a re-run of the installer
        // or `identity generate` MUST refuse — only `restore` or an explicit
        // `--burn` may produce an identity here.
        let (t, dir) = id_dir();
        let id = restore(&dir, V1_WORDS, None, true).unwrap();
        let mut ks = std::fs::read(dir.join(KEYSET_FILE)).unwrap();
        ks[0] ^= 0xff;
        std::fs::write(dir.join(KEYSET_FILE), ks).unwrap();
        assert!(load(&dir).is_err());
        assert!(!dir.exists());
        let q = quarantine_sibling(&dir).expect("quarantine sibling present");
        assert!(q.starts_with(t.path()));

        let err = generate(&dir).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<IdentityError>(),
            Some(IdentityError::QuarantinePresent(_))
        ), "{err}");
        assert!(!dir.exists(), "refusal must write nothing");
        assert!(err.to_string().contains("identity restore"));

        // explicit burn: a NEW identity, quarantine untouched
        let burned = generate_burn(&dir).unwrap();
        assert_ne!(burned.node_id, id.node_id);
        assert!(q.exists());
        assert_eq!(load(&dir).unwrap().node_id, burned.node_id);
    }

    #[test]
    fn fresh_generate_starts_announce_counter_at_zero() {
        // §6.2 genesis = 1 ⇒ persisted announce counter 0 (NOT the §8.4
        // restore floor + 1024). Read back via the owning CounterStore.
        let (_t, dir) = id_dir();
        let _ = generate(&dir).unwrap();
        let cs = id111::counters::CounterStore::open(&dir).unwrap();
        assert_eq!(cs.announce_last(), 0);
        assert!(!cs.is_restore_pending());
    }

    #[test]
    fn corrupt_variants_each_quarantine() {
        type Tamper = fn(&Path);
        let cases: [(&str, Tamper); 8] = [
            ("keyset truncated", |d| {
                std::fs::write(d.join(KEYSET_FILE), [0u8; 10]).unwrap()
            }),
            ("keyset all-zero (weak-key self-test)", |d| {
                std::fs::write(d.join(KEYSET_FILE), [0u8; 64]).unwrap()
            }),
            ("nodeid for another key", |d| {
                let mut r = std::fs::read(d.join(NODEID_FILE)).unwrap();
                r[5] ^= 1;
                std::fs::write(d.join(NODEID_FILE), r).unwrap()
            }),
            ("nodeid wrong length", |d| {
                std::fs::write(d.join(NODEID_FILE), [0xed, 0x01]).unwrap()
            }),
            ("descriptor unknown version", |d| {
                std::fs::write(d.join(DESCRIPTOR_FILE), [0x09, 0x01, 0x01, 0x01]).unwrap()
            }),
            ("descriptor contradicts word count", |d| {
                std::fs::write(d.join(DESCRIPTOR_FILE), [0x01, 0x01, 0x01, 0x02]).unwrap()
            }),
            ("counters missing", |d| {
                std::fs::remove_file(d.join(COUNTERS_FILE)).unwrap()
            }),
            ("mnemonic garbage", |d| {
                std::fs::write(d.join(MNEMONIC_FILE), "not a bip39 phrase\n").unwrap()
            }),
        ];
        for (name, tamper) in cases {
            let (t, dir) = id_dir();
            restore(&dir, V1_WORDS, None, true).unwrap();
            tamper(&dir);
            let err = load(&dir).unwrap_err();
            assert!(
                matches!(
                    err.downcast_ref::<IdentityError>(),
                    Some(IdentityError::Corrupt(_))
                ),
                "{name}: {err}"
            );
            assert!(!dir.exists(), "{name}: not quarantined");
            assert_eq!(
                quarantined(t.path(), "identity.quarantine-").len(),
                1,
                "{name}"
            );
        }
    }

    #[test]
    fn missing_identity_is_missing_not_corrupt() {
        let (t, dir) = id_dir();
        let err = load(&dir).unwrap_err();
        assert!(err.to_string().contains("missing"), "{err}");
        assert!(is_missing(&err));
        // an empty dir (installer mkdir before keygen) is also Missing, not Corrupt
        std::fs::create_dir(&dir).unwrap();
        assert!(is_missing(&load(&dir).unwrap_err()));
        assert!(quarantined(t.path(), "identity.quarantine-").is_empty());
        // ...and generate can write into it (rename replaces an empty dir)
        generate(&dir).unwrap();
    }

    #[test]
    fn shredded_mnemonic_loads_but_backup_refuses() {
        let (_t, dir) = id_dir();
        let id = restore(&dir, V1_WORDS, None, true).unwrap();
        std::fs::remove_file(dir.join(MNEMONIC_FILE)).unwrap();
        assert_eq!(load(&dir).unwrap().node_id, id.node_id);
        let err = backup_unchecked(&dir).err().unwrap();
        assert!(err.to_string().contains("not on disk"), "{err}");
    }

    #[test]
    fn load_does_not_touch_counters() {
        let (_t, dir) = id_dir();
        generate(&dir).unwrap();
        let before = std::fs::read(dir.join(COUNTERS_FILE)).unwrap();
        let mtime = dir
            .join(COUNTERS_FILE)
            .metadata()
            .unwrap()
            .modified()
            .unwrap();
        load(&dir).unwrap();
        load(&dir).unwrap();
        assert_eq!(std::fs::read(dir.join(COUNTERS_FILE)).unwrap(), before);
        assert_eq!(
            dir.join(COUNTERS_FILE)
                .metadata()
                .unwrap()
                .modified()
                .unwrap(),
            mtime
        );
    }

    #[test]
    fn backup_refuses_non_tty_stdout() {
        let (_t, dir) = id_dir();
        generate(&dir).unwrap();
        if unsafe { libc::isatty(1) } == 1 {
            return; // `--nocapture` on a real terminal: the refusal is not exercisable here
        }
        let err = backup(&dir).err().unwrap();
        assert!(err.to_string().contains("stdout is a terminal"), "{err}");
    }

    #[test]
    fn migrates_d9_dir_on_load() {
        let (t, dir) = id_dir();
        make_legacy_dir(&dir, V1_WORDS);
        let id = load(&dir).unwrap();

        // NodeId == id111's derivation of the same words (new derivation)
        assert_eq!(id.node_id, expected_node_id(V1_WORDS));
        // new §13 file set in place, legacy shape gone from the live dir
        for f in SET_FILES {
            assert!(dir.join(f).is_file(), "missing {f}");
        }
        assert!(!dir.join(LEGACY_SECRET_FILE).exists());
        assert!(!dir.join(LEGACY_NPUB_FILE).exists());
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join(MNEMONIC_FILE)), 0o600);
        assert_eq!(mode(&dir.join(KEYSET_FILE)), 0o600);
        assert_eq!(mode(&dir.join(NODEID_FILE)), 0o644);
        // quarantine sibling `<dir>.quarantine-d9` holds the legacy files, never deleted
        let q = t.path().join("identity.quarantine-d9");
        assert!(q.is_dir());
        assert!(q.join(LEGACY_SECRET_FILE).is_file());
        assert!(q.join(LEGACY_NPUB_FILE).is_file());
        // the logged old→new mapping
        let note = id.migration.as_ref().expect("migration note");
        assert_eq!(note.quarantine_dir, q);
        assert!(note.mapping.contains(&id.node_id_hex()), "{}", note.mapping);
        assert!(
            note.mapping.contains("legacy-handle-0001"),
            "{}",
            note.mapping
        );
        assert!(
            note.mapping.contains("identity.quarantine-d9"),
            "{}",
            note.mapping
        );
        // steady state: a second load is a plain load of the same NodeId
        let again = load(&dir).unwrap();
        assert_eq!(again.node_id, id.node_id);
        assert!(again.migration.is_none());
        // the migrated identity is fully usable: backup shows the same words
        assert_eq!(backup_unchecked(&dir).unwrap().artifacts.words(), V1_WORDS);
    }

    #[test]
    fn partial_or_mixed_legacy_dir_quarantines_as_corrupt() {
        // partial legacy: both legacy-only files but no preserved mnemonic
        let (t, dir) = id_dir();
        make_legacy_dir(&dir, V1_WORDS);
        std::fs::remove_file(dir.join(MNEMONIC_FILE)).unwrap();
        let err = load(&dir).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<IdentityError>(),
            Some(IdentityError::Corrupt(_))
        ));
        assert!(!dir.exists());
        assert_eq!(quarantined(t.path(), "identity.quarantine-").len(), 1);

        // mixed: legacy files beside a new-format file
        let (t, dir) = id_dir();
        make_legacy_dir(&dir, V1_WORDS);
        std::fs::write(dir.join(KEYSET_FILE), [7u8; 64]).unwrap();
        let err = load(&dir).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<IdentityError>(),
            Some(IdentityError::Corrupt(_))
        ));
        assert!(!dir.exists());
        assert_eq!(quarantined(t.path(), "identity.quarantine-").len(), 1);

        // legacy dir whose preserved words fail the BIP-39 checksum
        let (t, dir) = id_dir();
        make_legacy_dir(&dir, BAD_CHECKSUM);
        assert!(load(&dir).is_err());
        assert!(!dir.exists());
        assert_eq!(quarantined(t.path(), "identity.quarantine-").len(), 1);
    }

    #[test]
    fn legacy_dir_is_live_for_restore() {
        // §8.4/D14d: a provisional dir is migrated, never restored over.
        let (_t, dir) = id_dir();
        make_legacy_dir(&dir, V1_WORDS);
        assert!(is_live(&dir));
        let err = restore(&dir, V2_WORDS, None, true).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<IdentityError>(),
            Some(IdentityError::Exists)
        ));
    }

    #[test]
    fn node_id_hex_helpers() {
        let hex = "a".repeat(64);
        assert!(parse_node_id_hex(&hex).is_some());
        assert!(parse_node_id_hex("abc").is_none());
        assert!(parse_node_id_hex(&"g".repeat(64)).is_none());
        assert!(node_id_bech32(&hex).unwrap().starts_with("node1"));
        assert!(node_id_bech32("nope").is_none());
    }
}
