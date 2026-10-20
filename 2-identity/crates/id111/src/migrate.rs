//! §8.4 + §13 + D14d — the Layer-1 (D9-format) identity-dir migration.
//!
//! Implements (owner: T5): on first run of the L2-wired binary against a
//! provisional D9 identity dir (`secret.hex` + `npub` + `mnemonic`), this
//! path:
//!
//! 1. detects the legacy dir (all three files present; partial presence is
//!    corrupt, fail closed; legacy+new-format coexistence is mixed state,
//!    fail closed);
//! 2. reads the preserved `mnemonic` (the §8.4 hygiene applies: NFKC +
//!    case/whitespace folding, BIP-39 checksum MUST validate before
//!    derivation) and the legacy `npub` (carried through verbatim — no
//!    secp256k1 math anywhere in this crate);
//! 3. derives the L2 keyset (§2 — same words, new SLIP-0010 derivation; the
//!    NodeId changes once, pre-accepted D9/D13);
//! 4. quarantines the legacy dir by rename (never delete) into a
//!    `<dir>.quarantine-d9[-n]` sibling, and writes the new §13 file set
//!    (`mnemonic`/`keyset`/`nodeid`/`descriptor`/`counters`) atomically via
//!    [`crate::backup::write_identity_dir`];
//! 5. returns the old→new NodeId mapping for the log.
//!
//! Quarantine shape: the legacy dir is renamed ASIDE WHOLE (Layer 1's own
//! boot-quarantine shape, carried), not in place — rename(2) cannot replace a
//! non-empty directory, so an in-place quarantine subdir would break §13's
//! atomic temp-dir-then-rename write of the new file set. `secret.hex` and
//! `npub` (and the legacy `mnemonic` copy) ride along into quarantine; the
//! new dir gets a freshly canonicalized `mnemonic`. The legacy secret is
//! never read into memory — only renamed. Crash window, stated plainly: a
//! crash between the quarantine rename and the commit rename leaves the dir
//! quarantined with no new identity — the node reports Missing and explicit
//! `restore` (§8.4) is the way forward; never a silent re-key. A failed new-
//! dir write is rolled back best-effort (quarantine renamed back).
//!
//! Counter state: the migrated NodeId is NEW (the derivation changed), so no
//! peer holds announce-counter state for it — the §8.4 floor+jump rule does
//! not apply. The fresh §6.2 initial value 0 is persisted (the genesis
//! announce uses 1) through the [`crate::backup::initial_counters_file`]
//! call boundary (T4's `counters` module owns the format from there).

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::backup::{self, BackupDescriptor};
use crate::bip39::{Bip39Error, RootMnemonic};
use crate::keyset::{Keyset, KeysetError, NodeId};

/// D9 legacy layout (Layer 1 provisional, `identity/mod.rs`): the secp256k1
/// secret as hex — 0600. Quarantined by rename, NEVER deleted and never read
/// into memory by this migration.
pub const LEGACY_SECRET_FILE: &str = "secret.hex";
/// D9 legacy layout: the bech32 `npub` (x-only BIP-340 pubkey) — 0644.
/// Carried through verbatim into the old→new mapping; not parsed (no
/// secp256k1/bech32-nostr machinery in this crate).
pub const LEGACY_NPUB_FILE: &str = "npub";
/// D9 legacy layout: the preserved BIP-39 words — 0600. THE migration input.
pub const LEGACY_MNEMONIC_FILE: &str = backup::MNEMONIC_FILE;

/// Quarantine sibling-dir base name: `<identity-dir-name>.quarantine-d9`,
/// suffixed `-1`, `-2`, … on collision (never overwrite a prior quarantine).
const QUARANTINE_SUFFIX: &str = "quarantine-d9";

/// §13 new-format filenames whose presence alongside legacy files means a
/// half-migrated or hand-assembled dir — fail closed (MixedState).
const NEW_FORMAT_FILES: [&str; 4] = [
    backup::KEYSET_FILE,
    backup::NODEID_FILE,
    backup::DESCRIPTOR_FILE,
    backup::COUNTERS_FILE,
];

/// What the migration did and where the legacy material went — the old→new
/// NodeId mapping §8.4/D14d requires logging.
#[derive(Debug)]
pub struct MigrationOutcome {
    /// The legacy dir's `npub` file content, trimmed, carried through
    /// verbatim (the old secp256k1 identity's public handle; no secp256k1
    /// math is done — the mapping records the handle as stored).
    pub old_npub: String,
    /// The new L2 NodeId (raw 32B Ed25519 pubkey, §2 B2) derived from the
    /// same words. The NodeId changes once — pre-accepted D9/D13.
    pub new_node_id: NodeId,
    /// Where the legacy dir went (renamed, never deleted): the
    /// `*.quarantine-d9[-n]` sibling.
    pub quarantine_dir: PathBuf,
    /// The §8.2 descriptor written with the new file set.
    pub descriptor: BackupDescriptor,
    /// The persisted initial announce counter — always 0 here: §6.2's fresh
    /// initial value for a NodeId no peer has seen (the §8.4 floor+1024 rule
    /// is for state-absent restores of a KNOWN NodeId; migration mints a new
    /// one). The genesis announce uses 1.
    pub announce_counter_init: u64,
}

/// Migration failures. Detection is fail-closed: anything that is not
/// cleanly a legacy dir or cleanly not one is an error, never a guess.
#[derive(Debug)]
pub enum MigrationError {
    /// No legacy trio present (absent dir, empty dir, or an already
    /// new-format §13 dir) — nothing to migrate. Distinct from corrupt: this
    /// is the normal steady state after migration.
    NotLegacyDir,
    /// Some but not all of the legacy trio (`secret.hex`/`npub`/`mnemonic`)
    /// is present, or a legacy file is unreadable — a partial provisional
    /// identity. Operator intervention (or §13 quarantine boot semantics),
    /// never a guess.
    CorruptLegacy(String),
    /// Legacy files AND new-format §13 files coexist — fail closed.
    MixedState(String),
    /// The preserved `mnemonic` failed §8.4 hygiene (NFKC/checksum/profile) —
    /// the legacy dir is left exactly where it was.
    Mnemonic(Bip39Error),
    /// Keyset derivation or the §4.5 self-test failed.
    Keyset(KeysetError),
    /// Filesystem failure (quarantine rename or the §13 atomic write). A
    /// failed new-dir write is rolled back best-effort before this surfaces.
    Io(io::Error),
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrationError::NotLegacyDir => {
                write!(f, "§8.4/D9: not a legacy identity dir — nothing to migrate")
            }
            MigrationError::CorruptLegacy(why) => {
                write!(
                    f,
                    "§8.4/D9: legacy identity dir incomplete/unreadable: {why}"
                )
            }
            MigrationError::MixedState(why) => {
                write!(
                    f,
                    "§8.4/D9: legacy and new-format files coexist: {why} — fail closed"
                )
            }
            MigrationError::Mnemonic(e) => write!(f, "{e}"),
            MigrationError::Keyset(e) => write!(f, "{e}"),
            MigrationError::Io(e) => write!(f, "§8.4/D9 migration I/O failed: {e}"),
        }
    }
}

impl std::error::Error for MigrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MigrationError::Mnemonic(e) => Some(e),
            MigrationError::Keyset(e) => Some(e),
            MigrationError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<Bip39Error> for MigrationError {
    fn from(e: Bip39Error) -> Self {
        MigrationError::Mnemonic(e)
    }
}

impl From<KeysetError> for MigrationError {
    fn from(e: KeysetError) -> Self {
        MigrationError::Keyset(e)
    }
}

impl From<io::Error> for MigrationError {
    fn from(e: io::Error) -> Self {
        MigrationError::Io(e)
    }
}

/// Detection (§8.4/D14d): true iff `dir` holds the complete D9 legacy trio.
/// Use [`migrate`] for the fail-closed partial/mixed distinctions.
pub fn is_legacy_dir(dir: &Path) -> bool {
    dir.is_dir()
        && [LEGACY_SECRET_FILE, LEGACY_NPUB_FILE, LEGACY_MNEMONIC_FILE]
            .iter()
            .all(|name| dir.join(name).is_file())
}

/// Pick a quarantine sibling that does not exist yet:
/// `<name>.quarantine-d9`, then `-1`, `-2`, … (a prior quarantine is never
/// overwritten).
fn free_quarantine_dir(dir: &Path) -> Result<PathBuf, MigrationError> {
    let parent = dir.parent().unwrap_or_else(|| Path::new("."));
    let base = dir.file_name().unwrap_or_default().to_string_lossy();
    for n in 0..1000u32 {
        let candidate = match n {
            0 => parent.join(format!("{base}.{QUARANTINE_SUFFIX}")),
            _ => parent.join(format!("{base}.{QUARANTINE_SUFFIX}-{n}")),
        };
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(MigrationError::Io(io::Error::other(
        "no free quarantine dir name (1000 collisions)",
    )))
}

fn fsync_dir(dir: &Path) -> io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

/// §8.4/D14d — migrate a D9 provisional identity dir to the §13 L2 layout.
/// Errors with [`MigrationError::NotLegacyDir`] on anything that is not a
/// complete legacy trio (idempotent steady state after migration).
pub fn migrate(dir: &Path) -> Result<MigrationOutcome, MigrationError> {
    // 1. Detect, fail closed. The legacy-DISTINGUISHING files are
    //    `secret.hex` and `npub` — `mnemonic` is shared between the D9 and
    //    §13 layouts, so its presence alone says nothing. Mixed state: any
    //    new-format file beside any legacy-only file is a hand-assembled or
    //    half-migrated dir.
    let legacy_files: Vec<&str> = [LEGACY_SECRET_FILE, LEGACY_NPUB_FILE]
        .into_iter()
        .filter(|name| dir.join(name).exists())
        .collect();
    let mnemonic_present = dir.join(LEGACY_MNEMONIC_FILE).exists();
    let new_present: Vec<&str> = NEW_FORMAT_FILES
        .into_iter()
        .filter(|name| dir.join(name).exists())
        .collect();
    if !new_present.is_empty() && !legacy_files.is_empty() {
        return Err(MigrationError::MixedState(format!(
            "legacy {legacy_files:?} beside new-format {new_present:?}"
        )));
    }
    match legacy_files.len() {
        0 => return Err(MigrationError::NotLegacyDir),
        2 => {
            if !mnemonic_present {
                return Err(MigrationError::CorruptLegacy(
                    "secret.hex/npub present but the preserved mnemonic is missing — \
                     unmigratable; explicit restore (§8.4) is the path"
                        .into(),
                ));
            }
        }
        n => {
            return Err(MigrationError::CorruptLegacy(format!(
                "{n} of 2 legacy files present ({legacy_files:?})"
            )))
        }
    }

    // 2. Read the preserved mnemonic (§8.4 hygiene: NFKC/case/whitespace
    //    folded, checksum MUST validate before derivation) and the legacy
    //    npub (carried through verbatim). The legacy secret.hex is never
    //    read — only renamed into quarantine.
    let read_trimmed = |name: &str| -> Result<String, MigrationError> {
        fs::read_to_string(dir.join(name))
            .map(|s| s.trim().to_string())
            .map_err(|e| MigrationError::CorruptLegacy(format!("{name} unreadable: {e}")))
    };
    let words = read_trimmed(LEGACY_MNEMONIC_FILE)?;
    let old_npub = read_trimmed(LEGACY_NPUB_FILE)?;
    if old_npub.is_empty() {
        return Err(MigrationError::CorruptLegacy("npub file empty".into()));
    }
    let mnemonic = RootMnemonic::parse(&words)?;

    // 3. Same words, new derivation (§2): the NodeId changes once,
    //    pre-accepted D9/D13. The §4.5 self-test runs inside.
    let keyset = Keyset::derive_from_root(&mnemonic)?;
    let new_node_id = keyset.node_id();
    let descriptor = BackupDescriptor::for_mnemonic(&mnemonic);

    // 4. Quarantine the legacy dir aside (rename, never delete), then write
    //    the new §13 file set atomically. Roll back best-effort if the write
    //    fails — a crash between the two renames leaves the dir quarantined
    //    and the node reports Missing (never a silent re-key, §13).
    let quarantine_dir = free_quarantine_dir(dir)?;
    let parent = dir.parent().unwrap_or_else(|| Path::new("."));
    fs::rename(dir, &quarantine_dir)?;
    fsync_dir(parent)?;
    // §6.2 fresh initial announce counter for a NodeId no peer has seen.
    let announce_counter_init = 0u64;
    if let Err(e) =
        backup::write_identity_dir(dir, &mnemonic, &keyset, descriptor, announce_counter_init)
    {
        let _ = fs::rename(&quarantine_dir, dir); // best-effort rollback
        let _ = fsync_dir(parent);
        return Err(match e {
            backup::DirWriteError::Exists => {
                MigrationError::Io(io::Error::other("identity dir reappeared during migration"))
            }
            backup::DirWriteError::Io(e) => MigrationError::Io(e),
        });
    }

    // 5. The old→new mapping (the caller logs it — the event is logged,
    //    never the secret material; §8.3 discipline carried).
    Ok(MigrationOutcome {
        old_npub,
        new_node_id,
        quarantine_dir,
        descriptor,
        announce_counter_init,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Layer 1's own published NIP-06 test values (os/10-bootstrap identity
    /// tests) — a realistic D9 dir body.
    const V1_WORDS: &str =
        "leader monkey parrot ring guide accident before fence cannon height naive bean";
    const V1_SECRET_HEX: &str = "7f7ff03d123792d6ac594bfa67bf6d0c0ab55b6b1fdb6249303fe861f1ccba9a";
    const V1_NPUB: &str = "npub1zutzeysacnf9rru6zqwmxd54mud0k44tst6l70ja5mhv8jjumytsd2x7nu";

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    /// Minimal temp dir (no tempfile in the frozen dep set): unique per test,
    /// removed on drop.
    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("id111-migrate-{name}-{}-{n}", std::process::id()));
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

    /// Build a realistic D9 dir: secret.hex + npub + mnemonic (+ trailing
    /// newlines as Layer 1 wrote them).
    fn make_legacy_dir(dir: &Path, words: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(LEGACY_SECRET_FILE), format!("{V1_SECRET_HEX}\n")).unwrap();
        fs::write(dir.join(LEGACY_NPUB_FILE), format!("{V1_NPUB}\n")).unwrap();
        fs::write(dir.join(LEGACY_MNEMONIC_FILE), format!("{words}\n")).unwrap();
    }

    #[test]
    fn migration_round_trip_with_quarantine_disposition() {
        let t = TestDir::new("roundtrip");
        let dir = t.identity();
        make_legacy_dir(&dir, V1_WORDS);
        assert!(is_legacy_dir(&dir));

        let out = migrate(&dir).unwrap();

        // The old→new mapping: npub carried through verbatim; the new NodeId
        // is the §2 derivation from the same words.
        assert_eq!(out.old_npub, V1_NPUB);
        let expected = Keyset::derive_from_mnemonic(V1_WORDS).unwrap();
        assert_eq!(out.new_node_id, expected.node_id());
        assert_eq!(out.descriptor, BackupDescriptor::V1_12_WORDS);
        assert_eq!(out.announce_counter_init, 0);

        // New §13 file set in place, atomic-write shape, correct modes.
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(
            fs::read_to_string(dir.join(backup::MNEMONIC_FILE)).unwrap(),
            format!("{V1_WORDS}\n")
        );
        assert_eq!(mode(dir.join(backup::MNEMONIC_FILE).as_path()), 0o600);
        assert_eq!(fs::read(dir.join(backup::KEYSET_FILE)).unwrap().len(), 64);
        let record = fs::read(dir.join(backup::NODEID_FILE)).unwrap();
        assert_eq!(&record[..2], &[0xed, 0x01]);
        assert_eq!(&record[2..], out.new_node_id.as_bytes());
        assert_eq!(
            fs::read(dir.join(backup::DESCRIPTOR_FILE)).unwrap(),
            BackupDescriptor::V1_12_WORDS.to_bytes()
        );
        // Fresh §6.2 counter state (new NodeId — no floor+jump), read back
        // through the owning `CounterStore`.
        assert_eq!(
            crate::counters::CounterStore::open(&dir)
                .unwrap()
                .announce_last(),
            0u64
        );
        assert_eq!(mode(dir.join(backup::COUNTERS_FILE).as_path()), 0o600);

        // Quarantine disposition: legacy files are GONE from the live dir,
        // present (never deleted) in the quarantine sibling — secret.hex
        // byte-identical, npub byte-identical, the legacy mnemonic copy too.
        assert!(!dir.join(LEGACY_SECRET_FILE).exists());
        assert!(!dir.join(LEGACY_NPUB_FILE).exists());
        assert_eq!(out.quarantine_dir, t.0.join("identity.quarantine-d9"));
        assert_eq!(
            fs::read_to_string(out.quarantine_dir.join(LEGACY_SECRET_FILE)).unwrap(),
            format!("{V1_SECRET_HEX}\n")
        );
        assert_eq!(
            fs::read_to_string(out.quarantine_dir.join(LEGACY_NPUB_FILE)).unwrap(),
            format!("{V1_NPUB}\n")
        );
        assert_eq!(
            fs::read_to_string(out.quarantine_dir.join(LEGACY_MNEMONIC_FILE)).unwrap(),
            format!("{V1_WORDS}\n")
        );

        // Re-running against the migrated dir is a clean no-op error.
        assert!(matches!(migrate(&dir), Err(MigrationError::NotLegacyDir)));
        assert!(!is_legacy_dir(&dir));
    }

    #[test]
    fn migration_detection_fail_closed() {
        // Absent dir.
        let t = TestDir::new("absent");
        assert!(matches!(
            migrate(&t.identity()),
            Err(MigrationError::NotLegacyDir)
        ));

        // Empty dir.
        let t2 = TestDir::new("empty");
        let dir2 = t2.identity();
        fs::create_dir_all(&dir2).unwrap();
        assert!(matches!(migrate(&dir2), Err(MigrationError::NotLegacyDir)));

        // Partial trio (secret.hex only) — corrupt, not silent.
        let t3 = TestDir::new("partial");
        let dir3 = t3.identity();
        fs::create_dir_all(&dir3).unwrap();
        fs::write(dir3.join(LEGACY_SECRET_FILE), "00\n").unwrap();
        assert!(matches!(
            migrate(&dir3),
            Err(MigrationError::CorruptLegacy(_))
        ));
        assert!(!is_legacy_dir(&dir3));

        // Legacy pair without the preserved mnemonic — unmigratable, corrupt.
        let t3b = TestDir::new("no-mnemonic");
        let dir3b = t3b.identity();
        fs::create_dir_all(&dir3b).unwrap();
        fs::write(dir3b.join(LEGACY_SECRET_FILE), "00\n").unwrap();
        fs::write(dir3b.join(LEGACY_NPUB_FILE), "npub1xxx\n").unwrap();
        assert!(matches!(
            migrate(&dir3b),
            Err(MigrationError::CorruptLegacy(_))
        ));

        // A lone mnemonic (shared filename across D9/§13) is NOT a legacy dir.
        let t3c = TestDir::new("mnemonic-only");
        let dir3c = t3c.identity();
        fs::create_dir_all(&dir3c).unwrap();
        fs::write(dir3c.join(LEGACY_MNEMONIC_FILE), format!("{V1_WORDS}\n")).unwrap();
        assert!(matches!(migrate(&dir3c), Err(MigrationError::NotLegacyDir)));

        // Mixed state: full trio + a new-format file — fail closed.
        let t4 = TestDir::new("mixed");
        let dir4 = t4.identity();
        make_legacy_dir(&dir4, V1_WORDS);
        fs::write(dir4.join(backup::KEYSET_FILE), [0u8; 64]).unwrap();
        assert!(matches!(migrate(&dir4), Err(MigrationError::MixedState(_))));
        // Nothing moved.
        assert!(dir4.join(LEGACY_SECRET_FILE).exists());
    }

    #[test]
    fn migration_refuses_bad_mnemonic_without_touching_dir() {
        // §8.4: the checksum MUST validate before derivation — a corrupt
        // preserved mnemonic fails BEFORE anything is quarantined.
        let t = TestDir::new("badsum");
        let dir = t.identity();
        make_legacy_dir(
            &dir,
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon",
        );
        assert!(matches!(
            migrate(&dir),
            Err(MigrationError::Mnemonic(Bip39Error::Mnemonic(
                ::bip39::Error::InvalidChecksum
            )))
        ));
        // The legacy dir is exactly where it was; no quarantine created.
        assert!(is_legacy_dir(&dir));
        assert!(!t.0.join("identity.quarantine-d9").exists());
    }

    #[test]
    fn migration_quarantine_name_collision_uses_suffix() {
        let t = TestDir::new("collision");
        let dir = t.identity();
        // A prior quarantine occupies the base name (never overwritten).
        fs::create_dir_all(t.0.join("identity.quarantine-d9")).unwrap();
        fs::write(t.0.join("identity.quarantine-d9").join("marker"), b"x").unwrap();
        make_legacy_dir(&dir, V1_WORDS);

        let out = migrate(&dir).unwrap();
        assert_eq!(out.quarantine_dir, t.0.join("identity.quarantine-d9-1"));
        assert!(out.quarantine_dir.join(LEGACY_SECRET_FILE).exists());
        // The prior quarantine is untouched.
        assert!(t.0.join("identity.quarantine-d9").join("marker").exists());
    }

    #[test]
    fn migration_24_word_legacy_mnemonic() {
        // D9 generated 12 words, but a hand-restored 24-word legacy mnemonic
        // migrates with the matching §8.2 encoding byte.
        let words24 =
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
        let t = TestDir::new("w24");
        let dir = t.identity();
        make_legacy_dir(&dir, words24);
        let out = migrate(&dir).unwrap();
        assert_eq!(out.descriptor, BackupDescriptor::V1_24_WORDS);
        let expected = Keyset::derive_from_mnemonic(words24).unwrap();
        assert_eq!(out.new_node_id, expected.node_id());
    }

    #[test]
    fn migration_outcome_is_the_log_mapping() {
        // §8.4: the old→new NodeId mapping is produced for the log — public
        // material only (npub + NodeId; the legacy secret is never surfaced).
        let t = TestDir::new("mapping");
        let dir = t.identity();
        make_legacy_dir(&dir, V1_WORDS);
        let out = migrate(&dir).unwrap();
        let rendered = format!(
            "D9 migration: old npub {} → new NodeId {}",
            out.old_npub,
            crate::encoding::hex_encode(out.new_node_id.as_bytes())
        );
        assert!(rendered.contains(V1_NPUB));
        assert!(rendered.contains(&crate::encoding::hex_encode(
            Keyset::derive_from_mnemonic(V1_WORDS)
                .unwrap()
                .node_id()
                .as_bytes()
        )));
    }
}
