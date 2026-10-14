//! §10 — the encrypted local store (`fabric/store.db`, SQLCipher via
//! `rusqlite`), plus the §4 persistence of transmit/receive sequence state.
//!
//! - **Engine (§10.1):** raw 32-byte key (`PRAGMA key = "x'…'"`, no PBKDF2 at
//!   open), `cipher_compatibility = 4` (file format pinned to SQLCipher 4),
//!   WAL, `temp_store = MEMORY`, `auto_vacuum = INCREMENTAL`,
//!   `secure_delete = ON`, `synchronous = FULL` on sequence-allocating (and
//!   other promise-bearing) transactions, NORMAL otherwise. One DB, 0600.
//! - **Clean-shutdown record (§10.1):** `settings.clean = 1` **and** the L2
//!   `announce_last`, written by [`Store::close`]; [`Store::open`] reads both,
//!   then erases `clean` durably so a crash in this run is seen next time.
//! - **Epochs (§4):** [`Store::open`] decides whether a new epoch must be
//!   minted ([`EpochDecision`]); the caller mints through the identity handle
//!   (`Sealer::next_announce_counter`) and hands the value back with
//!   [`Store::record_epoch`]. Until then — or while the identity holder
//!   reports `RestoreFloorRequired` ([`Store::set_restore_pending`]) — no
//!   sequence number is allocated, so nothing can be sealed.
//! - **Schema (§10.3)** with migrations via `PRAGMA user_version`
//!   ([`schema`]); tx/rx state ([`tx`], [`rx`]); rows ([`rows`]); blob
//!   refcounts ([`blobs`]); capacity-only pruning ([`prune`], §10.4).
//!
//! No wall clock anywhere: ages are the store's local insertion sequence.

use std::fs::{self, DirBuilder, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior};

use crate::blobs::{BlobDir, BlobError};
use crate::keys::{KeyError, KeyProvider, StoreKey};

pub mod blobs;
pub mod prune;
pub mod rows;
pub mod rx;
pub mod schema;
pub mod tx;

#[cfg(test)]
mod tests;

pub use blobs::BlobRow;
pub use prune::{DiskSpace, PruneReport, PruneStep, PrunedItem, PrunedRef, Watermarks};
pub use rows::{
    Class, ContactRow, CustodyItem, CustodyPut, Dir, MessageRow, MsgState, NewMessage, OutboxFrag,
    RequestPut, RequestRefusal, Trust,
};
pub use rx::{FragOffer, Offer, Partial, PartialRef, RxEpoch};
pub use tx::Seq;

/// The DB file name inside the fabric directory (§2: `fabric/store.db`).
pub const DB_FILE: &str = "store.db";
/// The blob directory inside the fabric directory (§2: `fabric/blobs/`).
pub const BLOB_DIR: &str = "blobs";
/// The largest `n` within one epoch: `n` would wrap 2^32 beyond it (§4).
pub const N_MAX: u64 = u32::MAX as u64;

/// Typed store failures (never a panic on input).
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("blob: {0}")]
    Blob(#[from] BlobError),
    #[error("key: {0}")]
    Key(#[from] KeyError),
    #[error("wrong store key, or not a store")]
    WrongKey,
    #[error("SQLCipher is not linked (refusing a plaintext store)")]
    NoCipher,
    #[error("pragma {0} did not take effect")]
    Pragma(&'static str),
    #[error("schema version {found} is newer than this build ({supported})")]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("restore pending: nothing is sealed until the restore floor is set")]
    RestorePending,
    #[error("no epoch minted for this run yet")]
    EpochNotMinted,
    #[error("epoch exhausted: n would wrap 2^32; mint a new epoch")]
    EpochExhausted,
    #[error("epoch {new} does not exceed the recorded epoch {recorded}")]
    EpochNotIncreasing { new: u64, recorded: u64 },
    #[error("value out of range: {0}")]
    OutOfRange(&'static str),
    #[error("invalid input: {0}")]
    Invalid(&'static str),
    #[error("corrupt store row: {0}")]
    Corrupt(&'static str),
    #[error("key material differs from the pinned contact")]
    KeyConflict,
    #[error("store full: new custody and new bulk are refused")]
    StoreFull,
    #[error("not found")]
    NotFound,
    #[error("object key not held (a ciphertext-only blob)")]
    NoObjectKey,
}

/// Result alias.
pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// What the caller learned about this run at open (§4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenParams {
    /// The live identity's L2 `announce_last` (`Sealer::announce_last`).
    pub announce_last: u64,
    /// A restore floor was set since the last run (L2 §8.4).
    pub restore_floor_set: bool,
}

/// Why a new epoch must be minted at this open (§4: any one suffices).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MintReasons {
    /// The clean-shutdown record is absent (crash, first open, or a mint
    /// still pending from an earlier run).
    pub unclean: bool,
    /// The recorded `announce_last` differs from the live identity's
    /// (e.g. `store.db` restored from a backup onto a live identity).
    pub announce_mismatch: bool,
    /// A restore floor was set since the last run.
    pub restore_floor: bool,
    /// A stream's `n` in the recorded epoch would wrap 2^32.
    pub n_exhausted: bool,
}

impl MintReasons {
    /// Whether any condition holds.
    pub fn any(&self) -> bool {
        self.unclean || self.announce_mismatch || self.restore_floor || self.n_exhausted
    }
}

/// [`Store::open`]'s §4 verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochDecision {
    /// Keep the recorded epoch; sequence allocation may proceed.
    Keep { epoch: u64 },
    /// Mint a fresh epoch (`Sealer::next_announce_counter`, committed) and
    /// pass it to [`Store::record_epoch`] before anything is sealed.
    Mint(MintReasons),
}

/// The store's epoch state for this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochState {
    /// Sequence numbers are allocated under `epoch`.
    Ready { epoch: u64 },
    /// A mint is owed; allocation refused ([`StoreError::EpochNotMinted`]).
    MintRequired(MintReasons),
    /// The identity holder reports `RestoreFloorRequired`; allocation
    /// refused ([`StoreError::RestorePending`]), outbox stays `queued`.
    RestorePending,
}

/// Commit durability of one transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Durability {
    /// `synchronous = NORMAL` (WAL): routine bookkeeping.
    Normal,
    /// `synchronous = FULL`: sequence allocation, epochs, the clean record,
    /// and anything a receipt promises (§4, §10.1).
    Full,
}

/// The open store: one SQLCipher connection plus the blob directory.
pub struct Store {
    pub(crate) conn: Connection,
    pub(crate) dir: PathBuf,
    pub(crate) blobs: BlobDir,
    epoch: EpochState,
    pub(crate) store_full: bool,
    pub(crate) full_classes: Vec<Class>,
    pub(crate) last_disk: Option<DiskSpace>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("dir", &self.dir)
            .field("epoch", &self.epoch)
            .field("store_full", &self.store_full)
            .finish_non_exhaustive()
    }
}

// Settings keys (§10.1 clean record, §4 epoch, §10.4 capacity).
pub(crate) const K_CLEAN: &str = "clean";
pub(crate) const K_ANNOUNCE_LAST: &str = "announce_last";
pub(crate) const K_EPOCH: &str = "epoch";
pub(crate) const K_LOCAL_SEQ: &str = "local_seq";

impl Store {
    /// Open (or create) the store in the fabric directory `dir` with a key
    /// from `provider` (§10.2). See [`Store::open`].
    pub fn open_with(
        dir: &Path,
        provider: &dyn KeyProvider,
        params: &OpenParams,
    ) -> Result<(Store, EpochDecision)> {
        let key = provider.store_key()?;
        Store::open(dir, &key, params)
    }

    /// Open (or create) `dir/store.db` and `dir/blobs/`, apply the §10.1
    /// pragmas, migrate the schema, and decide the §4 epoch for this run.
    ///
    /// The clean-shutdown record is consumed: after this returns, a crash
    /// leaves it absent, so the next open mints.
    pub fn open(dir: &Path, key: &StoreKey, params: &OpenParams) -> Result<(Store, EpochDecision)> {
        if !dir.exists() {
            DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        let db_path = dir.join(DB_FILE);
        // 0600 before SQLite touches it; SQLite gives -wal/-shm the same mode.
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&db_path)
        {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                fs::set_permissions(&db_path, fs::Permissions::from_mode(0o600))?;
            }
            Err(e) => return Err(e.into()),
        }
        let conn = Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        apply_key_and_pragmas(&conn, key)?;
        schema::migrate(&conn)?;

        let blobs = BlobDir::open(dir.join(BLOB_DIR))?;
        let mut store = Store {
            conn,
            dir: dir.to_path_buf(),
            blobs,
            epoch: EpochState::MintRequired(MintReasons::default()),
            store_full: false,
            full_classes: Vec::new(),
            last_disk: None,
        };
        let decision = store.decide_epoch(params)?;
        store.epoch = match decision {
            EpochDecision::Keep { epoch } => EpochState::Ready { epoch },
            EpochDecision::Mint(r) => EpochState::MintRequired(r),
        };
        store.load_capacity()?;
        store.sweep_blob_orphans()?;
        Ok((store, decision))
    }

    /// §4: the four mint conditions, then consume the clean record.
    fn decide_epoch(&mut self, params: &OpenParams) -> Result<EpochDecision> {
        let decision = self.with_tx(Durability::Full, |tx| {
            let clean = get_i64(tx, K_CLEAN)? == Some(1);
            let recorded_al = get_u64(tx, K_ANNOUNCE_LAST)?;
            let epoch = get_u64(tx, K_EPOCH)?;
            let mut r = MintReasons {
                unclean: !clean || epoch.is_none(),
                announce_mismatch: recorded_al != Some(params.announce_last),
                restore_floor: params.restore_floor_set,
                n_exhausted: false,
            };
            if let Some(e) = epoch {
                r.n_exhausted = tx::epoch_exhausted(tx, e)?;
            }
            // Erase the clean record (absent ⇒ unclean next time).
            del(tx, K_CLEAN)?;
            Ok(match (r.any(), epoch) {
                (false, Some(epoch)) => EpochDecision::Keep { epoch },
                _ => EpochDecision::Mint(r),
            })
        })?;
        Ok(decision)
    }

    /// The epoch state of this run.
    pub fn epoch_state(&self) -> EpochState {
        self.epoch
    }

    /// Record a freshly minted epoch (a committed L2 announce-counter value,
    /// §4). Must exceed every epoch recorded before; committed with
    /// `synchronous = FULL`. Clears `MintRequired`/`RestorePending`; also
    /// used mid-run after [`StoreError::EpochExhausted`].
    pub fn record_epoch(&mut self, epoch: u64) -> Result<()> {
        to_i64(epoch, "epoch")?;
        self.with_tx(Durability::Full, |tx| {
            if let Some(recorded) = get_u64(tx, K_EPOCH)? {
                if epoch <= recorded {
                    return Err(StoreError::EpochNotIncreasing {
                        new: epoch,
                        recorded,
                    });
                }
            }
            set_u64(tx, K_EPOCH, epoch)
        })?;
        self.epoch = EpochState::Ready { epoch };
        Ok(())
    }

    /// The identity holder reports `RestoreFloorRequired` (§4): refuse
    /// sequence allocation until a new epoch is recorded.
    pub fn set_restore_pending(&mut self) {
        self.epoch = EpochState::RestorePending;
    }

    /// The current epoch, or the reason allocation is refused.
    pub(crate) fn ready_epoch(&self) -> Result<u64> {
        match self.epoch {
            EpochState::Ready { epoch } => Ok(epoch),
            EpochState::MintRequired(_) => Err(StoreError::EpochNotMinted),
            EpochState::RestorePending => Err(StoreError::RestorePending),
        }
    }

    /// §10.1 clean close: write `clean = 1` and the live `announce_last`
    /// (FULL), checkpoint the WAL, close. When no epoch was recorded this run
    /// (a mint is still owed), the record is **not** written, so the next
    /// open mints (fail closed). Dropping a `Store` without `close` is an
    /// unclean shutdown.
    pub fn close(mut self, announce_last: u64) -> Result<()> {
        to_i64(announce_last, "announce_last")?;
        if matches!(self.epoch, EpochState::Ready { .. }) {
            self.with_tx(Durability::Full, |tx| {
                set_u64(tx, K_ANNOUNCE_LAST, announce_last)?;
                set_i64(tx, K_CLEAN, 1)
            })?;
        }
        self.conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        self.conn.close().map_err(|(_, e)| StoreError::Sql(e))
    }

    /// The fabric directory this store lives in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The blob directory (`fabric/blobs/`).
    pub fn blob_dir(&self) -> &BlobDir {
        &self.blobs
    }

    /// Run `f` in one IMMEDIATE transaction at the given durability.
    pub(crate) fn with_tx<T>(
        &mut self,
        sync: Durability,
        f: impl FnOnce(&Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        if sync == Durability::Full {
            self.conn.pragma_update(None, "synchronous", "FULL")?;
        }
        let res = (|| {
            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let v = f(&tx)?;
            tx.commit()?;
            Ok(v)
        })();
        if sync == Durability::Full {
            let reset = self.conn.pragma_update(None, "synchronous", "NORMAL");
            if res.is_ok() {
                reset?;
            }
        }
        res
    }

    /// The current `synchronous` level (0 OFF, 1 NORMAL, 2 FULL, 3 EXTRA).
    pub fn synchronous_level(&self) -> Result<i64> {
        pragma_int(&self.conn, "synchronous")
    }
}

/// §10.1 — key first, then the format pin, then verify the key opens the
/// file, then the operational pragmas.
fn apply_key_and_pragmas(conn: &Connection, key: &StoreKey) -> Result<()> {
    {
        let lit = key.sqlcipher_literal();
        let mut sql = zeroize::Zeroizing::new(String::with_capacity(80));
        sql.push_str("PRAGMA key = \"");
        sql.push_str(&lit);
        sql.push_str("\";");
        conn.execute_batch(&sql)?;
    }
    conn.execute_batch("PRAGMA cipher_compatibility = 4;")?;
    // A plaintext SQLite would silently ignore `PRAGMA key`: refuse it.
    let version: Option<String> = conn
        .query_row("PRAGMA cipher_version", [], |r| r.get(0))
        .optional()?;
    if version.as_deref().is_none_or(str::is_empty) {
        return Err(StoreError::NoCipher);
    }
    // The first read decrypts page 1: a wrong key fails here.
    match conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    }) {
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::NotADatabase =>
        {
            return Err(StoreError::WrongKey)
        }
        Err(e) => return Err(e.into()),
    }
    // auto_vacuum only takes on a database with no tables yet (fresh file);
    // every store this code creates has it, and it is verified below.
    conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
    let mode: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::Pragma("journal_mode"));
    }
    conn.execute_batch(
        "PRAGMA temp_store = MEMORY;
         PRAGMA secure_delete = ON;
         PRAGMA foreign_keys = ON;
         PRAGMA synchronous = NORMAL;",
    )?;
    check_pragma(conn, "temp_store", 2)?;
    check_pragma(conn, "secure_delete", 1)?;
    check_pragma(conn, "foreign_keys", 1)?;
    // `cipher_compatibility` is write-only; verify the SQLCipher 4 format
    // parameters it pins instead (4 KiB pages, HMAC-SHA512, PBKDF2-SHA512).
    for (pragma, want) in [
        ("cipher_page_size", "4096"),
        ("cipher_hmac_algorithm", "HMAC_SHA512"),
        ("cipher_kdf_algorithm", "PBKDF2_HMAC_SHA512"),
    ] {
        let got: Option<String> = conn
            .query_row(&format!("PRAGMA {pragma}"), [], |r| r.get(0))
            .optional()?;
        if got.as_deref() != Some(want) {
            return Err(StoreError::Pragma("cipher_compatibility"));
        }
    }
    Ok(())
}

/// An integer pragma. SQLCipher answers some (e.g. `page_size`, routed to
/// `cipher_page_size`) as text, so both forms are accepted.
pub(crate) fn pragma_int(conn: &Connection, name: &'static str) -> Result<i64> {
    let v: rusqlite::types::Value = conn.pragma_query_value(None, name, |r| r.get(0))?;
    match v {
        rusqlite::types::Value::Integer(i) => Ok(i),
        rusqlite::types::Value::Text(t) => t.trim().parse().map_err(|_| StoreError::Pragma(name)),
        _ => Err(StoreError::Pragma(name)),
    }
}

fn check_pragma(conn: &Connection, name: &'static str, want: i64) -> Result<()> {
    let got = pragma_int(conn, name)?;
    if got == want {
        Ok(())
    } else {
        Err(StoreError::Pragma(name))
    }
}

// ---------------------------------------------------------------------------
// Settings + integer helpers (shared by the submodules)
// ---------------------------------------------------------------------------

/// u64 → SQLite INTEGER, refusing values above `i64::MAX` (fail closed).
pub(crate) fn to_i64(v: u64, what: &'static str) -> Result<i64> {
    i64::try_from(v).map_err(|_| StoreError::OutOfRange(what))
}

/// SQLite INTEGER → u64, refusing negatives (a corrupt row).
pub(crate) fn to_u64(v: i64, what: &'static str) -> Result<u64> {
    u64::try_from(v).map_err(|_| StoreError::Corrupt(what))
}

pub(crate) fn get_i64(c: &Connection, key: &str) -> Result<Option<i64>> {
    Ok(
        c.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get::<_, i64>(0)
        })
        .optional()?,
    )
}

pub(crate) fn set_i64(c: &Connection, key: &str, v: i64) -> Result<()> {
    c.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, v],
    )?;
    Ok(())
}

pub(crate) fn get_u64(c: &Connection, key: &str) -> Result<Option<u64>> {
    match get_i64(c, key)? {
        Some(v) => Ok(Some(to_u64(v, "settings value")?)),
        None => Ok(None),
    }
}

pub(crate) fn set_u64(c: &Connection, key: &str, v: u64) -> Result<()> {
    set_i64(c, key, to_i64(v, "settings value")?)
}

pub(crate) fn del(c: &Connection, key: &str) -> Result<()> {
    c.execute("DELETE FROM settings WHERE key = ?1", [key])?;
    Ok(())
}

/// The next local insertion sequence — the store's only notion of age
/// (§10.4: never a clock). Must run inside the caller's transaction.
pub(crate) fn next_local_seq(c: &Connection) -> Result<u64> {
    let next = get_u64(c, K_LOCAL_SEQ)?.unwrap_or(0) + 1;
    set_u64(c, K_LOCAL_SEQ, next)?;
    Ok(next)
}

/// The current (last issued) local sequence.
pub(crate) fn current_local_seq(c: &Connection) -> Result<u64> {
    Ok(get_u64(c, K_LOCAL_SEQ)?.unwrap_or(0))
}

/// A 32-byte id column → array (length checked before use).
pub(crate) fn id32(v: Vec<u8>, what: &'static str) -> Result<[u8; 32]> {
    <[u8; 32]>::try_from(v.as_slice()).map_err(|_| StoreError::Corrupt(what))
}
