//! Layer 8 (`os/8-storage/README.md` §2.1, §2.2 rules 1 to 9, §3 J1, J4, J8; §22 D2; row D2): the
//! node-state store `<root>/node/state.db`. Row D17 adds the migration window (§13.6 (1)) and row
//! D18 the integrity checks the unclean signal starts (§13.7).
//!
//! **What it is** (§2.2 rules 1 and 6): a second SQLCipher file beside the fabric's, `node/` 0700
//! and its files 0600, owned by the daemon's user (`node111`), which only `111d` opens while it
//! runs. A separate file, never tables in `fabric/store.db`, so a failed fabric migration or a
//! rollback never takes the agent's and the repairs' state down with messaging.
//!
//! **The key** (rule 2): `key_for("111/node-state/v1")` (§2.7 S1) of the Mode S custody over the
//! identity the daemon holds ([`sealer::node_state_keys`]: the `store_secret` the identity handle
//! gives the fabric, taken outside `fabric::boot`), derived once, in [`start`], before the fabric
//! boots, so it derives with the fabric down. Mode T answers no key, and with no identity loaded
//! there is none: the store is then unavailable for the run. Only the derived 32-byte key is kept,
//! in zeroizing memory; nothing of it is written anywhere (no key on disk).
//!
//! **The engine** (rule 3): mesh111's `open_encrypted` (§2.7 S2): the raw key with
//! `cipher_compatibility = 4`, a plaintext file and a wrong key refused on page 1, WAL,
//! `temp_store = MEMORY`, `secure_delete`, `auto_vacuum = INCREMENTAL`, `synchronous = NORMAL` and
//! `journal_size_limit` 4 MiB; then [`schema::migrate`] (rule 4; mesh111's `migrate_with`) and
//! `recursive_triggers` on, so a `REPLACE` keeps `ns_totals` through the DELETE trigger. A write
//! asks for [`Durability::Full`] where rule 3 needs it (a decision, a spool ingest, the legacy
//! import; migrations run `FULL` inside `migrate_with`): `synchronous = FULL` for that one
//! transaction, `NORMAL` again after it.
//!
//! **Open on use, closed on the idle pass** (rule 7): [`NodeState::read`], [`NodeState::write`]
//! and [`NodeState::load_totals`] open the store at their first use; a turn or an operation in
//! flight holds a [`Hold`], a use on every pass it spans, so a slow turn keeps the store open;
//! [`on_pass`] (§4.3 step 7) closes it on the first capacity pass that finds no use since the
//! previous one. A close runs `wal_checkpoint(TRUNCATE)` and closes the connection, so a clean close
//! leaves no `state.db-wal` (rule 9). No timer and no task: the housekeeping interval's pass is the
//! only clock. The daemon's [`Started`] closes the store when the daemon stops.
//!
//! **Reads write nothing** (rule 9): a read runs with `query_only` on, and the open of an existing
//! v1 file commits nothing (mesh111's open sets nothing a file holds already, and `migrate_with`
//! only reads a current file), so a status read or a history read leaves `state.db` and its WAL as
//! they were. A read never migrates: a store that was never written (no `state.db`; an empty one;
//! one whose first write stopped between mesh111's header page and the v1 commit, a power cut or a
//! full disk, [`schema::Held::Unwritten`]) is read as an empty v1 schema in memory, the file and
//! its WAL left as they were, and only a write migrates it.
//!
//! **The totals** (rule 7): `ns_totals` are read at the first open after a daemon start, then kept
//! in memory from the daemon's own writes (each write reads them inside its transaction), so the
//! pass reads [`NodeState::totals`] and opens nothing.
//!
//! **No clean record** (rule 9): unlike the fabric store, nothing marks a clean close; a non-empty
//! `state.db-wal` found by a `stat` before an open is the unclean signal
//! ([`NodeState::take_unclean`], handed to row D18's checks).
//!
//! **When it does not open** (rule 8): no key, a newer `user_version`, or a file that does not open
//! with this node's key (a damaged file, a plaintext one, another node's): the store stays closed
//! with a [`Reason`] every caller reads ([`NodeState::status`]'s `unavailable`, and
//! [`StateError::Unavailable`] from every call), and a refused open writes nothing. A failure a
//! later use may not meet (an I/O error, a full disk) reads the same way until the next use
//! retries the open.
//!
//! [`start`] runs once, from `daemon/mod.rs`, before the fabric boots: it builds the store's handle,
//! makes it [`current`] for the callers that hold none (the pass, the start's own steps), then the
//! J2 fold, the legacy import and the spool sweep, in that order (rows D4, D8, D6); D17 adds the
//! migration window there (the copy and the migration before the daemon reports health).
//!
//! **Never adds** (§1 rule 2): an open file on an idle node (closed on the first idle pass), a
//! timer, a task, a write on a read-only open, a clean record, a key on disk.

pub mod heal;
pub mod legacy;
pub mod schema;
pub mod service;
pub mod spool;

use crate::daemon::fabric::sealer;
use crate::daemon::housekeeping::PassCtx;
use crate::daemon::DaemonLayout;
use crate::identity::Identity;
use mesh111::keys::{KeyProvider, StoreKey, NODE_STATE_KDF_TAG};
use mesh111::store::{open_encrypted, StoreError};
use rusqlite::config::DbConfig;
use rusqlite::{Connection, ErrorCode, Transaction, TransactionBehavior};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Read;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

/// The store's directory under the node root (§2.2 rule 1), 0700.
pub const DIR: &str = "node";
/// The store's file inside [`DIR`] (§2.2 rule 1), 0600.
pub const DB_FILE: &str = "state.db";
/// The kernel's id of this boot (§2.2 rule 4: `boot_id` beside `seq`).
pub const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";
/// The most of [`BOOT_ID_PATH`] read (a UUID is 36 characters and a newline).
const BOOT_ID_MAX: u64 = 64;

/// The store's path under `root`: `<root>/node/state.db` (§2.2 rule 1).
pub fn db_path(root: &Path) -> PathBuf {
    root.join(DIR).join(DB_FILE)
}

/// The SQLite companion of `db` with `suffix` (`-wal`, `-shm`).
pub fn companion(db: &Path, suffix: &str) -> PathBuf {
    let mut name = db.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// How a write commits (§2.2 rule 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    /// `synchronous = NORMAL`, the store's level: a chat turn, a Layer 9 record.
    Normal,
    /// `synchronous = FULL` for this one transaction: a write that gates an applied action or
    /// precedes an unlink (a decision, a spool ingest, the legacy import).
    Full,
}

/// Why the store is unavailable (§2.2 rule 8): `GET /api/v1/storage`'s `node_state.reason`, the
/// chat view's "History is not being saved: <reason>". Display-only text; never a key, a path
/// outside the root or a record's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// No key: no identity is loaded, or the custody answers none (Mode T, §2.2 rule 2).
    NoKey(String),
    /// The file's `user_version` is newer than this build writes (§2.2 rule 8; mesh111's
    /// `SchemaTooNew`): never downgraded.
    NewerSchema {
        /// The file's `user_version`.
        found: i64,
        /// The newest version this build writes ([`schema::SCHEMA_VERSION`]).
        supported: i64,
    },
    /// The file does not open: a wrong key or a plaintext file (refused on page 1), a damaged
    /// file, a path that is not a regular file (§2.2 rule 8).
    Refused(String),
    /// The open failed for a reason a later use may not meet (an I/O error, a full disk, a busy
    /// file): retried at the next use.
    OpenFailed(String),
}

impl Reason {
    /// Whether the store stays closed with this reason for the rest of the daemon's run (no key, a
    /// newer schema, a file that does not open), or retries the open at its next use.
    pub fn is_sticky(&self) -> bool {
        !matches!(self, Reason::OpenFailed(_))
    }

    /// The reason a failed open gives (§2.2 rule 8).
    pub fn of_open(e: &StoreError) -> Reason {
        match e {
            StoreError::Key(k) => Reason::NoKey(k.to_string()),
            StoreError::SchemaTooNew { found, supported } => Reason::NewerSchema {
                found: *found,
                supported: *supported,
            },
            StoreError::WrongKey => Reason::Refused(
                "it does not open with this node's key (a damaged file, a plaintext one, or \
                 another node's)"
                    .into(),
            ),
            StoreError::Io(_) => Reason::OpenFailed(e.to_string()),
            StoreError::Sql(rusqlite::Error::SqliteFailure(f, _)) if transient(f.code) => {
                Reason::OpenFailed(e.to_string())
            }
            StoreError::Pragma(p) if !matches!(*p, "auto_vacuum" | "cipher_compatibility") => {
                Reason::OpenFailed(e.to_string())
            }
            other => Reason::Refused(other.to_string()),
        }
    }

    /// The reason a failed read at the open gives (§2.2 rules 7 and 8: the totals read at the
    /// first open after a start): SQLite's answer sorted as [`Reason::of_open`] sorts it, so a
    /// damaged page (a failed page HMAC, a malformed b-tree) closes the store for the run and only
    /// a busy file, an I/O error or a full disk is retried; a value the schema never writes is
    /// damage too.
    pub fn of_state(e: StateError) -> Reason {
        match e {
            StateError::Unavailable(r) => r,
            StateError::Sql(e) => Reason::of_open(&StoreError::Sql(e)),
            StateError::Corrupt(what) => Reason::Refused(format!("a damaged {what}")),
        }
    }
}

/// [`Reason::Refused`]'s text for a file at `user_version` 0 that holds schema objects
/// ([`schema::Held::Unversioned`]): not a store this schema wrote (§2.2 rule 8).
const UNVERSIONED: &str = "it holds tables but no schema version (not a store this release wrote)";

/// SQLite answers a later open may not meet: an I/O or permission failure, a full disk, a busy or
/// locked file, memory.
fn transient(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::PermissionDenied
            | ErrorCode::DatabaseBusy
            | ErrorCode::DatabaseLocked
            | ErrorCode::OutOfMemory
            | ErrorCode::ReadOnly
            | ErrorCode::OperationInterrupted
            | ErrorCode::SystemIoFailure
            | ErrorCode::DiskFull
            | ErrorCode::CannotOpen
            | ErrorCode::FileLockingProtocolFailed
    )
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::NoKey(why) => write!(f, "the node-state store has no key: {why}"),
            Reason::NewerSchema { found, supported } => write!(
                f,
                "node/state.db was written by a newer release (schema {found}; this release \
                 reads up to {supported})"
            ),
            Reason::Refused(why) => write!(f, "node/state.db does not open: {why}"),
            Reason::OpenFailed(why) => write!(f, "node/state.db could not be opened: {why}"),
        }
    }
}

/// A node-state store call's failure.
#[derive(Debug)]
pub enum StateError {
    /// The store is closed with this reason (§2.2 rule 8): nothing was read or written.
    Unavailable(Reason),
    /// SQLite refused a statement; the call's transaction rolled back.
    Sql(rusqlite::Error),
    /// A value the store holds is not what the schema writes (§2.2 rule 4).
    Corrupt(&'static str),
}

impl From<rusqlite::Error> for StateError {
    fn from(e: rusqlite::Error) -> Self {
        StateError::Sql(e)
    }
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateError::Unavailable(r) => write!(f, "the node-state store is unavailable: {r}"),
            StateError::Sql(e) => write!(f, "node-state store: {e}"),
            StateError::Corrupt(what) => write!(f, "node-state store: a damaged {what}"),
        }
    }
}

impl std::error::Error for StateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StateError::Sql(e) => Some(e),
            _ => None,
        }
    }
}

/// One namespace's `ns_totals` row (§2.2 rule 4): its rows, and Σ `length(body)` plus
/// [`schema::ROW_OVERHEAD`] per row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NsTotal {
    /// The namespace's records and entries.
    pub rows: u64,
    /// Σ `length(body)` + 64 per row.
    pub bytes: u64,
}

/// The totals per namespace (§2.2 rule 4; J9's `usage()`), the namespaces that hold rows.
pub type Totals = BTreeMap<String, NsTotal>;

/// The store's state on `GET /api/v1/storage` (§16.2 `node_state.state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreState {
    /// Open: a use since the last idle pass.
    Open,
    /// Closed, the idle state (§2.2 rule 7): not a fault.
    Closed,
    /// It does not open (§2.2 rule 8): `reason` says why.
    Unavailable,
}

/// `StorageNodeState` (§16.2; `API` `components.schemas.StorageNodeState`): the store's state, the
/// reason when it is unavailable, and `state.db`'s size from one `stat` (null with no file).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    /// `open`, `closed` or `unavailable`.
    pub state: StoreState,
    /// Why the store is unavailable, else `None`.
    pub reason: Option<String>,
    /// `state.db`'s size in bytes; `None` when there is no file.
    pub file_bytes: Option<u64>,
}

/// What a write stamps on a record beside its data (§2.2 rule 4): a new `seq`, this boot's id, the
/// boot-time milliseconds and the wall time, a display claim that orders nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    /// The store's next `seq` (`meta.next_seq`), taken inside the writing transaction.
    pub seq: i64,
    /// `/proc/sys/kernel/random/boot_id` as its 16 bytes (empty when unreadable).
    pub boot_id: Vec<u8>,
    /// `CLOCK_BOOTTIME` in milliseconds (monotonic across suspend; 0 when unreadable).
    pub mono_ms: i64,
    /// The wall clock as RFC 3339 UTC, a display claim only (§2.2 rule 4, J4).
    pub wall: Option<String>,
}

/// One writing transaction (§2.2 rule 4; J9 "each call one transaction"): the statements of
/// [`NodeState::write`]'s closure run on [`Writer::tx`], every write takes its `seq` from
/// [`Writer::next_seq`] or [`Writer::stamp`], and the `ns_totals` triggers run inside it.
pub struct Writer<'c> {
    tx: Transaction<'c>,
    boot_id: &'c [u8],
}

impl<'c> Writer<'c> {
    /// The transaction the closure's statements run in.
    pub fn tx(&self) -> &Transaction<'c> {
        &self.tx
    }

    /// The next `seq` from `meta.next_seq` (§2.2 rule 4): a new one on every call.
    pub fn next_seq(&self) -> Result<i64, StateError> {
        schema::next_seq(&self.tx)
    }

    /// A new `seq` with this boot's id, the boot-time milliseconds and the wall time (§2.2 rule
    /// 4): what every write of a record carries.
    pub fn stamp(&self) -> Result<Stamp, StateError> {
        Ok(Stamp {
            seq: self.next_seq()?,
            boot_id: self.boot_id.to_vec(),
            mono_ms: mono_ms(),
            wall: wall_now(),
        })
    }
}

/// A turn or an operation in flight (§2.2 rule 7): while one is held, every pass counts a use and
/// the store stays open; dropping it is a use too, so the store closes on the first pass after the
/// one that follows it.
#[must_use = "the store may close on the next pass once the hold is gone"]
pub struct Hold {
    store: Arc<NodeState>,
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.store.used.store(true, SeqCst);
        self.store.holds.fetch_sub(1, SeqCst);
    }
}

impl fmt::Debug for Hold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hold").finish_non_exhaustive()
    }
}

/// [`NodeState::busy`]'s guard: a [`Hold`] for the length of one call.
struct Busy<'a>(&'a NodeState);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.used.store(true, SeqCst);
        self.0.holds.fetch_sub(1, SeqCst);
    }
}

/// What the store's state shows without its I/O lock.
#[derive(Default)]
struct Meta {
    /// Why the store does not open: sticky ([`Reason::is_sticky`]) for the run, or the last
    /// failed open's, cleared by an open that succeeds.
    unavailable: Option<Reason>,
    /// `ns_totals`, known from the first open after the start (or, with nothing written, empty).
    totals: Option<Totals>,
}

/// What a use finds.
enum Found {
    /// The connection is open.
    Open,
    /// Nothing was ever written: no `state.db`, an empty one, or one whose first write's
    /// migration did not commit (a read sees an empty store, and writes nothing).
    Absent,
}

/// What one `stat` finds at `state.db` before a use that only reads (§2.2 rules 7 and 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnDisk {
    /// Nothing at the path.
    Absent,
    /// A regular file of 0 bytes: a first write's open (mesh111 creates the file before SQLite
    /// touches it) that stopped before SQLite wrote its header page. It holds nothing, and an
    /// open would write that page.
    Empty,
    /// Anything else: the open says what it is.
    Present,
}

/// What an open hands its use.
enum Opened {
    /// The store, at this schema version, with `ns_totals` when they were asked for.
    Ready(Connection, Option<Totals>),
    /// A use that only reads found a file never written ([`schema::Held::Unwritten`]): the
    /// connection was closed again with the file and its WAL as they were.
    Unwritten,
}

/// Serializes the opens of every node-state store in the process (one per daemon; tests open
/// several), as the fabric serializes its own: concurrent first SQLCipher opens failed rarely
/// under load (`fabric/mod.rs`'s `OPEN_LOCK`).
static OPEN_LOCK: Mutex<()> = Mutex::new(());

/// The node-state store (§2.2): closed until its first use, open while used, closed by the first
/// idle pass, `unavailable` with a [`Reason`] when it does not open. One per daemon, shared as an
/// `Arc`; every call is a blocking call, run off the runtime's workers.
pub struct NodeState {
    path: PathBuf,
    key: Option<StoreKey>,
    conn: Mutex<Option<Connection>>,
    meta: Mutex<Meta>,
    is_open: AtomicBool,
    used: AtomicBool,
    holds: AtomicUsize,
    unclean: AtomicBool,
    stopping: AtomicBool,
    boot_id: OnceLock<Vec<u8>>,
}

impl fmt::Debug for NodeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeState")
            .field("path", &self.path)
            .field("open", &self.is_open.load(SeqCst))
            .finish_non_exhaustive()
    }
}

impl NodeState {
    /// The store under `root` keyed by `keys.key_for(NODE_STATE_KDF_TAG)` (§2.2 rule 2; §2.7 S1),
    /// derived once, here. A custody that answers no key (Mode T) leaves the store unavailable for
    /// the run. Nothing is opened, read or written.
    pub fn new(root: &Path, keys: &dyn KeyProvider) -> NodeState {
        match keys.key_for(NODE_STATE_KDF_TAG) {
            Ok(key) => NodeState::build(db_path(root), Some(key), None),
            Err(e) => NodeState::build(db_path(root), None, Some(Reason::NoKey(e.to_string()))),
        }
    }

    /// The store under `root` with no key (`why`: no identity is loaded): unavailable for the run
    /// (§2.2 rule 8).
    pub fn without_key(root: &Path, why: &str) -> NodeState {
        NodeState::build(db_path(root), None, Some(Reason::NoKey(why.to_string())))
    }

    fn build(path: PathBuf, key: Option<StoreKey>, unavailable: Option<Reason>) -> NodeState {
        NodeState {
            path,
            key,
            conn: Mutex::new(None),
            meta: Mutex::new(Meta {
                unavailable,
                totals: None,
            }),
            is_open: AtomicBool::new(false),
            used: AtomicBool::new(false),
            holds: AtomicUsize::new(0),
            unclean: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            boot_id: OnceLock::new(),
        }
    }

    /// `<root>/node/state.db`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `node_state` of `GET /api/v1/storage` (§16.2): open, closed or unavailable with the reason,
    /// and the file's size from one `stat`. Opens nothing, waits on no I/O.
    pub fn status(&self) -> Status {
        let reason = self.meta().unavailable.as_ref().map(ToString::to_string);
        let state = if reason.is_some() {
            StoreState::Unavailable
        } else if self.is_open.load(SeqCst) {
            StoreState::Open
        } else {
            StoreState::Closed
        };
        Status {
            state,
            reason,
            file_bytes: self.file_bytes(),
        }
    }

    /// Why the store is unavailable, if it is (§2.2 rule 8).
    pub fn unavailable(&self) -> Option<Reason> {
        self.meta().unavailable.clone()
    }

    /// Whether the connection is open now.
    pub fn is_open(&self) -> bool {
        self.is_open.load(SeqCst)
    }

    /// `state.db`'s size from one `stat`; `None` with no file.
    pub fn file_bytes(&self) -> Option<u64> {
        fs::metadata(&self.path)
            .ok()
            .filter(fs::Metadata::is_file)
            .map(|m| m.len())
    }

    /// The totals kept in memory (§2.2 rule 7), `None` before the first open after the start:
    /// what the pass reads. Opens nothing.
    pub fn totals(&self) -> Option<Totals> {
        self.meta().totals.clone()
    }

    /// The totals, opening the store when they are not known yet (§2.2 rule 7: "a status read
    /// before the totals are known"); with nothing written (no file, an empty one, one whose first
    /// migration did not commit) they are empty and nothing is written. The open reads and writes
    /// nothing else (rule 9).
    pub fn load_totals(&self) -> Result<Totals, StateError> {
        if let Some(t) = self.totals() {
            return Ok(t);
        }
        let _busy = self.busy();
        blocking(|| {
            let mut slot = self.lock_conn();
            let found = self.prepare(&mut slot, false)?;
            let totals = match found {
                Found::Absent => {
                    let empty = Totals::new();
                    self.meta().totals = Some(empty.clone());
                    empty
                }
                Found::Open => self.totals().unwrap_or_default(),
            };
            self.settle(&mut slot);
            Ok(totals)
        })
    }

    /// Run `f` on the store, read-only (§2.2 rules 7 and 9): opens the store on its first use,
    /// runs `f` with `query_only` on, so it can write nothing, and commits nothing. With nothing
    /// written (no `state.db`, an empty one, one whose first migration did not commit) `f` reads
    /// an empty v1 schema in memory and the file is neither created nor migrated. `f`'s error, or
    /// [`StateError::Unavailable`] with the reason when the store does not open.
    pub fn read<T, E>(&self, f: impl FnOnce(&Connection) -> Result<T, E>) -> Result<T, E>
    where
        E: From<StateError>,
    {
        let _busy = self.busy();
        blocking(|| {
            let mut slot = self.lock_conn();
            let res = match self.prepare(&mut slot, false)? {
                Found::Absent => {
                    let mem = empty_store()?;
                    query_only(&mem, f).0
                }
                Found::Open => match slot.as_ref() {
                    Some(conn) => {
                        let (res, clean) = query_only(conn, f);
                        if !clean {
                            self.close(&mut slot);
                        }
                        res
                    }
                    None => Err(self.not_open().into()),
                },
            };
            self.settle(&mut slot);
            res
        })
    }

    /// Run `f` in one IMMEDIATE transaction (§2.2 rules 3, 4 and 7; J9 "each call one
    /// transaction"), at `durability`: opens the store on its first use (creating `node/` 0700 and
    /// `state.db` 0600 with schema v1 when absent), commits when `f` returns `Ok` and rolls back
    /// otherwise. Before the commit the transaction reads `ns_totals`, which the triggers kept,
    /// and the totals in memory take them after it.
    pub fn write<T, E>(
        &self,
        durability: Durability,
        f: impl FnOnce(&Writer<'_>) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<StateError>,
    {
        let _busy = self.busy();
        blocking(|| {
            let mut slot = self.lock_conn();
            self.prepare(&mut slot, true)?;
            let boot_id = self.boot_id();
            let res = match slot.as_mut() {
                Some(conn) => transact(conn, durability, boot_id, f),
                None => Err(self.not_open().into()),
            };
            let out = match res {
                Ok((v, totals)) => {
                    self.meta().totals = Some(totals);
                    Ok(v)
                }
                Err(e) => Err(e),
            };
            self.settle(&mut slot);
            out
        })
    }

    /// A turn or an operation in flight (§2.2 rule 7): a use on every pass until the [`Hold`] is
    /// dropped. Opens nothing.
    pub fn hold(self: &Arc<Self>) -> Hold {
        self.holds.fetch_add(1, SeqCst);
        self.used.store(true, SeqCst);
        Hold {
            store: Arc::clone(self),
        }
    }

    /// §4.3 step 7 (§2.2 rule 7): close the store when nothing used it since the previous pass, no
    /// [`Hold`] is held and no call is in progress. A close runs `wal_checkpoint(TRUNCATE)`, which
    /// writes nothing when only reads ran. Reads nothing, opens nothing; on an idle node it is
    /// three atomic loads.
    pub fn on_pass(&self) {
        let used = self.used.swap(false, SeqCst);
        if used || self.holds.load(SeqCst) > 0 || !self.is_open.load(SeqCst) {
            return;
        }
        blocking(|| {
            let mut slot = self.lock_conn();
            // a use that came in while this pass waited for the lock keeps it open
            if self.used.load(SeqCst) || self.holds.load(SeqCst) > 0 {
                return;
            }
            self.close(&mut slot);
        });
    }

    /// Whether an open since the last call found a non-empty `state.db-wal` (§2.2 rule 9: the
    /// unclean signal, row D18's), clearing it.
    pub fn take_unclean(&self) -> bool {
        self.unclean.swap(false, SeqCst)
    }

    /// The daemon stops (§2.2 rule 9): close the store now, and from here on close it after every
    /// use, so the process leaves no `state.db-wal` behind and the next start finds no unclean
    /// signal.
    pub fn stop(&self) {
        self.stopping.store(true, SeqCst);
        blocking(|| {
            let mut slot = self.lock_conn();
            self.close(&mut slot);
        });
    }

    /// A call in progress (§2.2 rule 7: an operation still running is a use): a use on every pass
    /// it spans, from before it waits for the store to after it is done.
    fn busy(&self) -> Busy<'_> {
        self.holds.fetch_add(1, SeqCst);
        self.used.store(true, SeqCst);
        Busy(self)
    }

    /// The store's I/O lock. A call that panicked while it held it leaves the connection in an
    /// unknown state (its transaction rolled back, `query_only` maybe still on): the store starts
    /// again from closed.
    fn lock_conn(&self) -> MutexGuard<'_, Option<Connection>> {
        match self.conn.lock() {
            Ok(slot) => slot,
            Err(poisoned) => {
                let mut slot = poisoned.into_inner();
                self.close(&mut slot);
                self.conn.clear_poison();
                slot
            }
        }
    }

    fn meta(&self) -> MutexGuard<'_, Meta> {
        self.meta.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn sticky(&self) -> Option<Reason> {
        self.meta().unavailable.clone().filter(Reason::is_sticky)
    }

    fn not_open(&self) -> StateError {
        StateError::Unavailable(
            self.unavailable()
                .unwrap_or_else(|| Reason::OpenFailed("the store is not open".into())),
        )
    }

    fn boot_id(&self) -> &[u8] {
        self.boot_id
            .get_or_init(|| read_boot_id(Path::new(BOOT_ID_PATH)))
    }

    /// The store ready for a use: open, or (`create` false) absent; else the reason it does not
    /// open. A sticky reason is answered without touching the file. A use that only reads opens
    /// no file that holds nothing (absent or empty, one `stat`) and migrates none (§2.2 rule 9).
    fn prepare(&self, slot: &mut Option<Connection>, create: bool) -> Result<Found, StateError> {
        if let Some(r) = self.sticky() {
            return Err(StateError::Unavailable(r));
        }
        if slot.is_some() {
            return Ok(Found::Open);
        }
        if !create && self.on_disk() != OnDisk::Present {
            return Ok(Found::Absent);
        }
        let Some(key) = self.key.as_ref() else {
            return Err(self.fail(Reason::NoKey("no key was derived".into())));
        };
        let want_totals = self.meta().totals.is_none();
        match self.open_now(key, want_totals, create) {
            Ok(Opened::Ready(conn, totals)) => {
                *slot = Some(conn);
                self.is_open.store(true, SeqCst);
                let mut m = self.meta();
                m.unavailable = None;
                if let Some(t) = totals {
                    m.totals = Some(t);
                }
                Ok(Found::Open)
            }
            Ok(Opened::Unwritten) => {
                self.meta().unavailable = None;
                Ok(Found::Absent)
            }
            Err(r) => Err(self.fail(r)),
        }
    }

    fn fail(&self, r: Reason) -> StateError {
        self.meta().unavailable = Some(r.clone());
        StateError::Unavailable(r)
    }

    /// One `stat` of `state.db` (§2.2 rules 7 and 9): absent, an empty regular file (a link is
    /// followed, as mesh111's open follows it), or anything else, whose open then says what it
    /// is.
    fn on_disk(&self) -> OnDisk {
        match fs::symlink_metadata(&self.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => OnDisk::Absent,
            _ => match fs::metadata(&self.path) {
                Ok(m) if m.is_file() && m.len() == 0 => OnDisk::Empty,
                _ => OnDisk::Present,
            },
        }
    }

    /// The open (§2.2 rules 1, 3, 4, 7, 8 and 9): `node/` 0700, the unclean signal by one `stat`
    /// of the WAL, mesh111's `open_encrypted`, what the file holds ([`schema::held`]), the schema,
    /// the totals at the first open after the start. A use that only reads (`create` false)
    /// never migrates: a file never written is closed again as it was ([`Opened::Unwritten`]),
    /// and only a write migrates it. A file at version 0 that holds tables is refused (rule 8).
    /// A refused open writes nothing to the file: a WAL a crash left stays as it was.
    fn open_now(&self, key: &StoreKey, want_totals: bool, create: bool) -> Result<Opened, Reason> {
        let _one = OPEN_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(dir) = self.path.parent() {
            ensure_dir(dir).map_err(|e| Reason::OpenFailed(format!("node/: {e}")))?;
        }
        let wal = companion(&self.path, "-wal");
        let crashed = fs::metadata(&wal).is_ok_and(|m| m.len() > 0);
        if crashed {
            self.unclean.store(true, SeqCst);
        }
        let conn = open_encrypted(&self.path, key).map_err(|e| Reason::of_open(&e))?;
        let ready = match schema::held(&conn) {
            Ok(schema::Held::Unwritten) if !create => {
                close_untouched(conn, crashed);
                return Ok(Opened::Unwritten);
            }
            Ok(schema::Held::Unversioned) => Err(Reason::Refused(UNVERSIONED.into())),
            Ok(schema::Held::Unwritten | schema::Held::Versioned) => {
                (|| -> Result<Option<Totals>, Reason> {
                    schema::migrate(&conn).map_err(|e| Reason::of_open(&e))?;
                    conn.pragma_update(None, "recursive_triggers", true)
                        .map_err(|e| Reason::of_open(&StoreError::from(e)))?;
                    if !want_totals {
                        return Ok(None);
                    }
                    schema::read_totals(&conn)
                        .map(Some)
                        .map_err(Reason::of_state)
                })()
            }
            Err(e) => Err(Reason::of_open(&e)),
        };
        match ready {
            Ok(totals) => Ok(Opened::Ready(conn, totals)),
            Err(r) => {
                close_untouched(conn, crashed);
                Err(r)
            }
        }
    }

    /// After a use: a stopping daemon closes the store at once.
    fn settle(&self, slot: &mut Option<Connection>) {
        if self.stopping.load(SeqCst) {
            self.close(slot);
        }
    }

    fn close(&self, slot: &mut Option<Connection>) {
        if let Some(conn) = slot.take() {
            self.is_open.store(false, SeqCst);
            close_conn(conn);
        }
    }
}

impl Drop for NodeState {
    fn drop(&mut self) {
        let slot = self.conn.get_mut().unwrap_or_else(PoisonError::into_inner);
        if let Some(conn) = slot.take() {
            close_conn(conn);
        }
    }
}

/// The close of a connection its use wrote nothing through (a refused open, a read of a file never
/// written; §2.2 rule 9): a WAL a crash left is not checkpointed into the file (it stays as it was,
/// the unclean signal of the next open and of the build that reads this file); with none, the
/// close only removes the empty `-wal` and the `-shm` the open made.
fn close_untouched(conn: Connection, crashed: bool) {
    if crashed {
        let _ = conn.set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true);
    }
    drop(conn);
}

/// A close (§2.2 rule 7): `wal_checkpoint(TRUNCATE)`, then the connection's close, which as the
/// file's last connection removes the empty `-wal` and the `-shm`.
fn close_conn(conn: Connection) {
    if let Err(e) = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())) {
        eprintln!("111 state: node/state.db: the checkpoint at close failed: {e}");
    }
    if let Err((_, e)) = conn.close() {
        eprintln!("111 state: node/state.db did not close: {e}");
    }
}

/// `f` on `conn` with `query_only` on (§2.2 rule 9), and whether the connection is as it was
/// after it (`query_only` off again, no transaction left open).
fn query_only<T, E>(
    conn: &Connection,
    f: impl FnOnce(&Connection) -> Result<T, E>,
) -> (Result<T, E>, bool)
where
    E: From<StateError>,
{
    if let Err(e) = conn.pragma_update(None, "query_only", true) {
        return (Err(StateError::from(e).into()), true);
    }
    let res = f(conn);
    let mut clean = true;
    if !conn.is_autocommit() {
        clean = conn.execute_batch("ROLLBACK").is_ok();
    }
    if conn.pragma_update(None, "query_only", false).is_err() {
        clean = false;
    }
    (res, clean)
}

/// One writing transaction at `durability` (§2.2 rules 3 and 4): `f`, then `ns_totals` read
/// inside it, then the commit; `synchronous` back to NORMAL after a FULL one.
fn transact<T, E>(
    conn: &mut Connection,
    durability: Durability,
    boot_id: &[u8],
    f: impl FnOnce(&Writer<'_>) -> Result<T, E>,
) -> Result<(T, Totals), E>
where
    E: From<StateError>,
{
    let full = durability == Durability::Full;
    if full {
        conn.pragma_update(None, "synchronous", "FULL")
            .map_err(StateError::from)?;
    }
    let res = (|| {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StateError::from)?;
        let w = Writer { tx, boot_id };
        let v = f(&w)?;
        let totals = schema::read_totals(&w.tx)?;
        let Writer { tx, .. } = w;
        tx.commit().map_err(StateError::from)?;
        Ok((v, totals))
    })();
    if full {
        let reset = conn.pragma_update(None, "synchronous", "NORMAL");
        if res.is_ok() {
            reset.map_err(StateError::from)?;
        }
    }
    res
}

/// An empty v1 store in memory: what a read sees before anything was ever written (§2.2 rule 9:
/// the read creates no file).
fn empty_store() -> Result<Connection, StateError> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(schema::V1)?;
    conn.pragma_update(None, "user_version", schema::SCHEMA_VERSION)?;
    Ok(conn)
}

/// `node/` 0700 (§2.2 rule 1): created when absent, its mode set when it is another (an
/// existing directory with the right mode is left untouched).
fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    match fs::metadata(dir) {
        Ok(m) if m.is_dir() => {
            if m.permissions().mode() & 0o7777 != 0o700 {
                fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
            }
            Ok(())
        }
        Ok(_) => Err(std::io::Error::other("not a directory")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
        }
        Err(e) => Err(e),
    }
}

/// `CLOCK_BOOTTIME` in milliseconds (§2.2 rule 4 `mono_ms`): monotonic, counts suspend; 0 when the
/// clock cannot be read.
fn mono_ms() -> i64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec; CLOCK_BOOTTIME is a valid clock id on Linux and
    // Android.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) };
    if rc != 0 {
        return 0;
    }
    let s = u64::try_from(ts.tv_sec).unwrap_or(0);
    let ns = u64::try_from(ts.tv_nsec).unwrap_or(0);
    let ms = s.saturating_mul(1000).saturating_add(ns / 1_000_000);
    i64::try_from(ms).unwrap_or(i64::MAX)
}

/// The wall clock as RFC 3339 UTC: `wall`, a display claim only (§2.2 rule 4; J4).
fn wall_now() -> Option<String> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}

/// This boot's id from `path` as its 16 bytes (§2.2 rule 4), the size checked before the parse;
/// empty when unreadable or not a UUID.
fn read_boot_id(path: &Path) -> Vec<u8> {
    let mut text = Vec::new();
    let read = fs::File::open(path).and_then(|f| f.take(BOOT_ID_MAX + 1).read_to_end(&mut text));
    if read.is_err() || text.len() as u64 > BOOT_ID_MAX {
        return Vec::new();
    }
    parse_boot_id(&text).map(Vec::from).unwrap_or_default()
}

/// A UUID's 16 bytes: 32 hex digits, dashes and surrounding whitespace ignored.
fn parse_boot_id(text: &[u8]) -> Option<[u8; 16]> {
    let hex: Vec<u8> = std::str::from_utf8(text)
        .ok()?
        .trim()
        .bytes()
        .filter(|&c| c != b'-')
        .collect();
    if hex.len() != 32 || !hex.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let mut out = [0u8; 16];
    let (pairs, _) = hex.as_chunks::<2>();
    for (o, pair) in out.iter_mut().zip(pairs) {
        *o = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

/// Runs `f`, which waits on the disk, without holding a runtime worker (the A1 pattern,
/// `A/src/store.rs`): under `block_in_place` on a worker of a multi-thread runtime, in place
/// anywhere else (a current-thread runtime, the blocking pool, a plain thread).
fn blocking<R>(f: impl FnOnce() -> R) -> R {
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current() {
        Ok(h) if h.runtime_flavor() == RuntimeFlavor::MultiThread => tokio::task::block_in_place(f),
        _ => f(),
    }
}

/// The store [`start`] made current: the one the housekeeping pass closes (§4.3 step 7) and the
/// start's own steps reach.
static CURRENT: Mutex<Option<Arc<NodeState>>> = Mutex::new(None);

/// The daemon's node-state store, once [`start`] made it ([`on_pass`] and the callers that hold no
/// handle); `None` before, and after the daemon stopped.
pub fn current() -> Option<Arc<NodeState>> {
    CURRENT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// The daemon's hold on its node-state store ([`start`]): dropping it, when `daemon_main` ends,
/// closes the store (a checkpoint with TRUNCATE, so a clean stop leaves no `state.db-wal`, §2.2
/// rule 9) and takes it out of [`current`].
#[must_use = "dropping it closes the node-state store"]
pub struct Started {
    store: Arc<NodeState>,
}

impl Started {
    /// The daemon's store.
    pub fn store(&self) -> &Arc<NodeState> {
        &self.store
    }
}

impl fmt::Debug for Started {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Started")
            .field("store", &self.store)
            .finish()
    }
}

impl Drop for Started {
    fn drop(&mut self) {
        self.store.stop();
        let mut slot = CURRENT.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.as_ref().is_some_and(|s| Arc::ptr_eq(s, &self.store)) {
            *slot = None;
        }
    }
}

/// Layer 8's start (§2.2, §2.4 rule 2, §2.5, §3 J2), once, before the fabric boots: the store's
/// handle under `layout.root`, keyed from `identity`, the identity handle the daemon holds
/// (§2.2 rule 2: `key_for("111/node-state/v1")` of its Mode S custody,
/// [`sealer::node_state_keys`], outside `fabric::boot`; no identity, no key), made [`current`];
/// then the J2 files folded into `heal/`, the legacy agent files imported, the spool swept, in
/// that order. Nothing is opened until a use.
pub fn start(layout: &DaemonLayout, identity: Option<&Identity>) -> Started {
    let store = Arc::new(match identity.map(sealer::node_state_keys) {
        Some(Ok(keys)) => NodeState::new(&layout.root, &keys),
        Some(Err(e)) => {
            NodeState::without_key(&layout.root, &format!("the identity's store secret: {e}"))
        }
        None => NodeState::without_key(&layout.root, "no identity is loaded"),
    });
    *CURRENT.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&store));
    heal::fold_at_start(layout);
    legacy::import_at_start(layout);
    spool::sweep(layout);
    Started { store }
}

/// §4.3 step 7: the node-state store closes if no operation used it since the previous pass
/// (§2.2 rule 7; [`NodeState::on_pass`]). No store started: nothing.
pub fn on_pass(_ctx: &PassCtx) {
    if let Some(store) = current() {
        store.on_pass();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh111::keys::{ModeS, ModeT, NODE_STATE_KDF_TAG, STORE_KDF_TAG};
    use rusqlite::params;
    use zeroize::Zeroizing;

    fn keys(seed: u8) -> ModeS {
        ModeS::new(Zeroizing::new([seed; 32]), [seed ^ 0x5a; 32])
    }

    fn store(root: &Path) -> Arc<NodeState> {
        Arc::new(NodeState::new(root, &keys(7)))
    }

    /// A head record and one entry, as a turn writes them (raw SQL: J9's calls are row D3's).
    fn turn(s: &NodeState, id: &str, text: &[u8]) -> (i64, i64) {
        s.write(Durability::Normal, |w| -> Result<_, StateError> {
            let e = w.stamp()?;
            w.tx().execute(
                "INSERT INTO entries (ns, stream, seq, body) VALUES (?1, ?2, ?3, ?4)",
                params![
                    "agent/conversations",
                    format!("agent/conversations/{id}"),
                    e.seq,
                    text
                ],
            )?;
            let h = w.stamp()?;
            w.tx().execute(
                "INSERT INTO records (ns, id, seq, boot_id, mono_ms, wall, state, body)
                     VALUES ('agent/conversations', ?1, ?2, ?3, ?4, ?5, 'open', x'7b7d')
                     ON CONFLICT (ns, id) DO UPDATE SET seq = excluded.seq,
                         boot_id = excluded.boot_id, mono_ms = excluded.mono_ms,
                         wall = excluded.wall",
                params![id, h.seq, h.boot_id, h.mono_ms, h.wall],
            )?;
            Ok((e.seq, h.seq))
        })
        .unwrap()
    }

    fn count(s: &NodeState, sql: &str) -> i64 {
        s.read(|c| -> Result<i64, StateError> { Ok(c.query_row(sql, [], |r| r.get(0))?) })
            .unwrap()
    }

    #[test]
    fn the_first_write_creates_the_store_and_every_write_takes_a_new_seq() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        assert_eq!(s.status().state, StoreState::Closed);
        assert_eq!(s.status().file_bytes, None);
        assert_eq!(turn(&s, "c1", b"hello"), (1, 2));
        assert_eq!(
            turn(&s, "c1", b"again"),
            (3, 4),
            "the head's re-put takes a new seq"
        );
        assert!(s.is_open());
        assert_eq!(s.status().state, StoreState::Open);
        assert_eq!(count(&s, "SELECT seq FROM records WHERE id = 'c1'"), 4);
        assert_eq!(count(&s, "PRAGMA user_version"), schema::SCHEMA_VERSION);
        // the allocator survives a close and a reopen
        s.on_pass();
        s.on_pass();
        assert!(!s.is_open());
        assert_eq!(turn(&s, "c2", b"x"), (5, 6));
    }

    #[test]
    fn a_write_stamps_this_boot_the_boot_clock_and_a_wall_claim() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        let (a, b) = s
            .write(Durability::Normal, |w| -> Result<_, StateError> {
                Ok((w.stamp()?, w.stamp()?))
            })
            .unwrap();
        assert_eq!(b.seq, a.seq + 1);
        assert!(b.mono_ms >= a.mono_ms && a.mono_ms > 0);
        let kernel = fs::read(BOOT_ID_PATH).ok().and_then(|t| parse_boot_id(&t));
        assert_eq!(a.boot_id, kernel.map(Vec::from).unwrap_or_default());
        let wall = a.wall.unwrap();
        assert!(
            time::OffsetDateTime::parse(&wall, &time::format_description::well_known::Rfc3339)
                .is_ok()
        );
    }

    #[test]
    fn a_failed_write_rolls_back_and_hands_its_seq_out_again() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        turn(&s, "c1", b"a");
        let err = s
            .write(Durability::Normal, |w| -> Result<(), StateError> {
                w.next_seq()?;
                w.tx().execute(
                    "INSERT INTO entries (ns, stream, seq, body) VALUES ('n', 's', 99, x'')",
                    [],
                )?;
                Err(StateError::Corrupt("test"))
            })
            .unwrap_err();
        assert!(matches!(err, StateError::Corrupt("test")));
        assert_eq!(count(&s, "SELECT count(*) FROM entries WHERE seq = 99"), 0);
        assert_eq!(turn(&s, "c1", b"b"), (3, 4));
        assert_eq!(
            s.totals().unwrap()["agent/conversations"].rows,
            3,
            "the totals in memory follow the commits only"
        );
    }

    #[test]
    fn full_durability_holds_for_its_transaction_only() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        let level = |c: &Connection| -> Result<i64, StateError> {
            Ok(c.query_row("PRAGMA synchronous", [], |r| r.get(0))?)
        };
        let full = s.write(Durability::Full, |w| level(w.tx())).unwrap();
        let normal = s.write(Durability::Normal, |w| level(w.tx())).unwrap();
        assert_eq!((full, normal), (2, 1));
        assert_eq!(s.read(level).unwrap(), 1, "NORMAL again after a FULL one");
        // a FULL transaction that fails leaves NORMAL too
        let _ = s.write(Durability::Full, |_| -> Result<(), StateError> {
            Err(StateError::Corrupt("test"))
        });
        assert_eq!(s.read(level).unwrap(), 1);
    }

    #[test]
    fn a_read_can_write_nothing() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        turn(&s, "c1", b"a");
        let err = s
            .read(|c| -> Result<usize, StateError> { Ok(c.execute("DELETE FROM entries", [])?) })
            .unwrap_err();
        assert!(matches!(err, StateError::Sql(_)), "query_only refuses it");
        // a transaction a read leaves open is rolled back, and the next write works
        let _ = s.read(|c| -> Result<(), StateError> { Ok(c.execute_batch("BEGIN")?) });
        assert_eq!(count(&s, "SELECT count(*) FROM entries"), 1);
        assert_eq!(turn(&s, "c1", b"b"), (3, 4));
    }

    #[test]
    fn a_store_never_written_reads_empty_and_creates_nothing() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        assert_eq!(count(&s, "SELECT count(*) FROM records"), 0);
        assert_eq!(s.load_totals().unwrap(), Totals::new());
        assert!(!t.path().join(DIR).exists(), "no node/ for a read");
        assert!(!s.is_open());
        assert_eq!(s.status().state, StoreState::Closed);
    }

    #[test]
    fn the_totals_are_read_at_the_first_open_then_kept_from_the_writes() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        turn(&s, "c1", &[1u8; 100]);
        let want = NsTotal {
            rows: 2,
            bytes: 100 + 2 + 2 * schema::ROW_OVERHEAD,
        };
        assert_eq!(s.totals().unwrap()["agent/conversations"], want);
        drop(s);
        // a new start: unknown until the first open, which a status read makes
        let s = store(t.path());
        assert_eq!(s.totals(), None);
        assert_eq!(s.load_totals().unwrap()["agent/conversations"], want);
        assert!(
            s.is_open(),
            "a status read before the totals are known opens it"
        );
        s.on_pass();
        s.on_pass();
        assert!(!s.is_open());
        assert_eq!(
            s.totals().unwrap()["agent/conversations"],
            want,
            "kept closed"
        );
        turn(&s, "c1", &[1u8; 10]);
        let held = s.read(schema::read_totals).unwrap();
        assert_eq!(s.totals().unwrap(), held, "kept from its own writes");
    }

    #[test]
    fn the_idle_pass_closes_and_a_hold_keeps_it_open_across_passes() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        turn(&s, "c1", b"a");
        s.on_pass();
        assert!(s.is_open(), "a use since the previous pass");
        s.on_pass();
        assert!(
            !s.is_open(),
            "the first pass with no use since the previous one"
        );
        let hold = s.hold();
        count(&s, "SELECT count(*) FROM records");
        for _ in 0..3 {
            s.on_pass();
            assert!(s.is_open(), "a turn in flight is a use on every pass");
        }
        drop(hold);
        s.on_pass();
        assert!(s.is_open(), "the hold's end is a use");
        s.on_pass();
        assert!(!s.is_open());
        assert!(
            !companion(s.path(), "-wal").exists(),
            "a close leaves no WAL"
        );
        assert!(!companion(s.path(), "-shm").exists());
    }

    #[test]
    fn no_key_and_mode_t_leave_it_unavailable_with_the_reason() {
        let t = tempfile::tempdir().unwrap();
        for s in [
            NodeState::new(t.path(), &ModeT),
            NodeState::without_key(t.path(), "no identity is loaded"),
        ] {
            let st = s.status();
            assert_eq!(st.state, StoreState::Unavailable);
            assert!(st
                .reason
                .unwrap()
                .starts_with("the node-state store has no key: "));
            assert!(matches!(
                s.read(|_| -> Result<(), StateError> { Ok(()) }),
                Err(StateError::Unavailable(Reason::NoKey(_)))
            ));
            assert!(matches!(
                s.write(Durability::Full, |_| -> Result<(), StateError> { Ok(()) }),
                Err(StateError::Unavailable(Reason::NoKey(_)))
            ));
            assert!(matches!(s.load_totals(), Err(StateError::Unavailable(_))));
        }
        assert!(
            !t.path().join(DIR).exists(),
            "nothing created without a key"
        );
    }

    #[test]
    fn the_key_is_key_for_the_node_state_tag() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        turn(&s, "c1", b"a");
        s.stop();
        let k = keys(7);
        let node = k.key_for(NODE_STATE_KDF_TAG).unwrap();
        let conn = open_encrypted(&db_path(t.path()), &node).unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM entries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        drop(conn);
        let fabric = k.key_for(STORE_KDF_TAG).unwrap();
        assert!(matches!(
            open_encrypted(&db_path(t.path()), &fabric),
            Err(StoreError::WrongKey)
        ));
    }

    #[test]
    fn the_open_reasons_are_sorted_into_sticky_and_retried() {
        let sticky = [
            StoreError::WrongKey,
            StoreError::SchemaTooNew {
                found: 2,
                supported: 1,
            },
            StoreError::Invalid("store path is not a regular file"),
            StoreError::Corrupt("user_version"),
            StoreError::Pragma("auto_vacuum"),
            StoreError::Key(mesh111::keys::KeyError::Kdf),
            StoreError::Sql(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
                None,
            )),
        ];
        for e in &sticky {
            assert!(Reason::of_open(e).is_sticky(), "{e}");
        }
        let retried = [
            StoreError::Io(std::io::Error::other("EIO")),
            StoreError::Sql(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
                None,
            )),
            StoreError::Sql(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
                None,
            )),
            StoreError::Pragma("journal_mode"),
        ];
        for e in &retried {
            assert!(!Reason::of_open(e).is_sticky(), "{e}");
        }
        assert_eq!(
            Reason::of_open(&sticky[1]).to_string(),
            "node/state.db was written by a newer release (schema 2; this release reads up to 1)"
        );
        // a failed read at the open (the totals) is sorted the same way
        let sql = |code| {
            StateError::Sql(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                None,
            ))
        };
        for code in [rusqlite::ffi::SQLITE_CORRUPT, rusqlite::ffi::SQLITE_ERROR] {
            assert!(
                matches!(Reason::of_state(sql(code)), Reason::Refused(_)),
                "{code}"
            );
        }
        for code in [
            rusqlite::ffi::SQLITE_BUSY,
            rusqlite::ffi::SQLITE_IOERR,
            rusqlite::ffi::SQLITE_FULL,
        ] {
            assert!(
                matches!(Reason::of_state(sql(code)), Reason::OpenFailed(_)),
                "{code}"
            );
        }
        assert_eq!(
            Reason::of_state(StateError::Corrupt("ns_totals")),
            Reason::Refused("a damaged ns_totals".into())
        );
        assert_eq!(
            Reason::of_state(StateError::Unavailable(Reason::NoKey("k".into()))),
            Reason::NoKey("k".into())
        );
    }

    #[test]
    fn a_file_with_tables_and_no_schema_version_is_refused_and_never_migrated() {
        let t = tempfile::tempdir().unwrap();
        let path = db_path(t.path());
        {
            let key = keys(7).key_for(NODE_STATE_KDF_TAG).unwrap();
            let conn = open_encrypted(&path, &key).unwrap();
            conn.execute_batch("CREATE TABLE notes (x TEXT) STRICT;")
                .unwrap();
        }
        let before = fs::read(&path).unwrap();
        // a read first, then a write first: both refuse it, for the run
        fn refused(r: Result<(), StateError>) -> bool {
            match r {
                Err(StateError::Unavailable(Reason::Refused(why))) => why == UNVERSIONED,
                _ => false,
            }
        }
        let reader = store(t.path());
        assert!(refused(
            reader.read(|_| -> Result<(), StateError> { Ok(()) })
        ));
        assert!(refused(reader.write(Durability::Full, |_| Ok(()))));
        let writer = store(t.path());
        assert!(refused(writer.write(Durability::Full, |_| Ok(()))));
        assert!(refused(writer.load_totals().map(drop)));
        assert_eq!(
            writer.status().reason.as_deref(),
            Some(
                "node/state.db does not open: it holds tables but no schema version (not a store \
                 this release wrote)"
            )
        );
        assert_eq!(fs::read(&path).unwrap(), before, "never migrated over");
        assert!(!companion(&path, "-wal").exists());
    }

    #[test]
    fn a_failed_open_a_later_use_may_not_meet_is_retried() {
        let t = tempfile::tempdir().unwrap();
        // node/ is a file: the open fails, and is retried once it is a directory
        fs::write(t.path().join(DIR), b"").unwrap();
        let s = store(t.path());
        assert!(matches!(
            s.write(Durability::Normal, |_| -> Result<(), StateError> { Ok(()) }),
            Err(StateError::Unavailable(Reason::OpenFailed(_)))
        ));
        assert_eq!(s.status().state, StoreState::Unavailable);
        fs::remove_file(t.path().join(DIR)).unwrap();
        assert_eq!(turn(&s, "c1", b"a"), (1, 2));
        assert_eq!(s.status().state, StoreState::Open);
        assert_eq!(s.status().reason, None);
    }

    #[test]
    fn stop_closes_and_a_later_use_closes_after_itself() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        turn(&s, "c1", b"a");
        s.stop();
        assert!(!s.is_open());
        assert!(!companion(s.path(), "-wal").exists());
        turn(&s, "c1", b"b");
        assert!(!s.is_open(), "closed again after the use");
        assert!(!companion(s.path(), "-wal").exists());
        assert_eq!(count(&s, "SELECT count(*) FROM entries"), 2);
    }

    #[test]
    fn a_call_in_progress_is_a_use_on_every_pass_it_spans() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        turn(&s, "c1", b"a");
        let (started, running) = mpsc::channel();
        let (release, held) = mpsc::channel::<()>();
        let writer = {
            let s = Arc::clone(&s);
            std::thread::spawn(move || {
                s.write(Durability::Normal, |w| -> Result<i64, StateError> {
                    started.send(()).unwrap();
                    held.recv().unwrap();
                    w.next_seq()
                })
                .unwrap()
            })
        };
        running.recv().unwrap();
        // two passes while the write runs: each finds a use, neither waits for the store
        let passes = {
            let s = Arc::clone(&s);
            std::thread::spawn(move || {
                s.on_pass();
                s.on_pass();
            })
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !passes.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let returned = passes.is_finished();
        release.send(()).unwrap();
        assert_eq!(writer.join().unwrap(), 3);
        passes.join().unwrap();
        assert!(returned, "a pass waited for a call in progress");
        assert!(
            s.is_open(),
            "a write spanning two passes kept the store open"
        );
        s.on_pass();
        assert!(s.is_open(), "the call's end is a use");
        s.on_pass();
        assert!(!s.is_open());
    }

    #[test]
    fn a_call_that_panics_leaves_the_store_usable() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        turn(&s, "c1", b"a");
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = s.read(|_| -> Result<(), StateError> { panic!("a caller's bug") });
        }));
        assert!(caught.is_err());
        assert_eq!(turn(&s, "c1", b"b"), (3, 4), "not left read-only");
        idle_passes_close(&s);
    }

    /// Two passes with no use between them close the store.
    fn idle_passes_close(s: &NodeState) {
        s.on_pass();
        s.on_pass();
        assert!(!s.is_open());
    }

    #[test]
    fn an_existing_node_dir_takes_0700_and_the_files_0600() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join(DIR);
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let s = store(t.path());
        // a read of an absent store creates nothing and changes nothing
        count(&s, "SELECT count(*) FROM records");
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o7777,
            0o755
        );
        turn(&s, "c1", b"a");
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o7777,
            0o700
        );
        for p in [
            db_path(t.path()),
            companion(s.path(), "-wal"),
            companion(s.path(), "-shm"),
        ] {
            assert_eq!(
                fs::metadata(&p).unwrap().permissions().mode() & 0o7777,
                0o600
            );
        }
    }

    #[test]
    fn boot_ids_parse_as_their_16_bytes_and_nothing_else() {
        assert_eq!(
            parse_boot_id(b"0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0\n"),
            Some([
                0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2,
                0xe1, 0xf0
            ])
        );
        for bad in [
            &b""[..],
            b"0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f",
            b"0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f00",
            b"+f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0",
            b"0f1e2d3c 4b5a 6978 8796 a5b4c3d2e1f0",
            b"\xff\xfe",
        ] {
            assert_eq!(parse_boot_id(bad), None, "{bad:?}");
        }
        let t = tempfile::tempdir().unwrap();
        let big = t.path().join("boot_id");
        fs::write(&big, [b'0'; 65]).unwrap();
        assert!(read_boot_id(&big).is_empty(), "over the size: not read");
        assert!(read_boot_id(&t.path().join("absent")).is_empty());
    }

    #[test]
    fn start_makes_the_store_current_and_its_end_closes_it() {
        let t = tempfile::tempdir().unwrap();
        let layout = DaemonLayout {
            root: t.path().to_path_buf(),
            etc: t.path().join("etc"),
        };
        let id = crate::identity::generate(&t.path().join("identity")).unwrap();
        let started = start(&layout, Some(&id));
        assert!(
            !t.path().join(DIR).exists(),
            "the start opens nothing until a use"
        );
        let s = Arc::clone(started.store());
        // The housekeeping tests of this binary run passes on the current store too: a hold
        // keeps it open across the write and its assert whatever they do.
        let in_flight = s.hold();
        turn(&s, "c1", b"a");
        assert!(s.is_open());
        drop(in_flight);
        // other tests' passes may run this one's too: only the eventual close is asserted
        on_pass(&PassCtx::default());
        on_pass(&PassCtx::default());
        assert!(!s.is_open());
        turn(&s, "c1", b"b");
        drop(started);
        assert!(!s.is_open(), "the daemon's end closes the store");
        assert!(!companion(s.path(), "-wal").exists());
        assert!(current().is_none_or(|c| !Arc::ptr_eq(&c, &s)));
        // the same identity, loaded again, derives the same key
        drop(id);
        let id = crate::identity::load(&t.path().join("identity")).unwrap();
        let again = start(&layout, Some(&id));
        assert_eq!(count(again.store(), "SELECT count(*) FROM entries"), 2);
        let none = start(&layout, None);
        assert_eq!(none.store().status().state, StoreState::Unavailable);
    }
}
