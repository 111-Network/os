//! Layer 8 (`os/8-storage/README.md` §3 J9, J3, J4; §2.2 rules 4, 5 and 8; §2.4 rules 2 and 5;
//! §4.4, §4.5 `full`; row D3): the state service API over the node-state store.
//!
//! **In process only** (J9): Layer 9's modules call it, and Layer 8's own (the chat relay, the
//! spool, the decision route, the legacy import, the budget); no route reaches it and the agent
//! never does. The calls are J9's: [`StateService::put`], [`get`](StateService::get),
//! [`list`](StateService::list), [`append`](StateService::append),
//! [`read_stream`](StateService::read_stream), [`set_state`](StateService::set_state),
//! [`delete`](StateService::delete), [`delete_stream`](StateService::delete_stream),
//! [`trim_stream`](StateService::trim_stream), [`usage`](StateService::usage),
//! [`reclaim`](StateService::reclaim), [`decision_begin`](StateService::decision_begin) (`FULL`),
//! [`decision_complete`](StateService::decision_complete) (`FULL`) and
//! [`decision_reopen`](StateService::decision_reopen), plus §2.4 rule 2's spool
//! [`ingest`](StateService::ingest) (`FULL`).
//!
//! **One transaction each** (J9): every call is one transaction of the store's one connection
//! ([`NodeState::write`] or [`NodeState::read`], so a store that is not open is opened on the use,
//! §2.2 rule 7), but a reclaim, which commits one per step (below). Every write of a record or an
//! entry takes a new `seq` from `meta.next_seq` and returns it (§2.2 rule 4): a `put` or a
//! `set_state` on an existing record included, so "oldest" is "least recently written"; `boot_id`,
//! `mono_ms` (`CLOCK_BOOTTIME`) and `wall` sit beside it, and only `seq` orders anything (J4: no
//! clock in an order). A delete takes no `seq`: it returns what it removed. Layer 8's own modules
//! compose J9's writes in one transaction of their own through [`Ops`] (a chat turn appends its
//! messages and bumps its head in one, §2.3 rule 3).
//!
//! **Names** (§2.2 rule 5): a record namespace is `<top>/<name>[/<name>…]` (`agent/conversations`,
//! `heal/incidents`, `storage/…`), its top one of [`TOPS`], its names `[a-z0-9_-]`; an id is
//! `[A-Za-z0-9_-]{1,128}`, the project's id rule; a state is `[a-z0-9_-]{1,64}`. A stream
//! `<ns>/<id>` is the stream of the record (`ns`, `id`) when `ns` is a record namespace (a
//! conversation's messages, `agent/conversations/<id>`, beside its head), and a stream of its own
//! when `ns` is a top (Layer 9's audit trail, `heal/audit`); `entries.ns` is that `ns`
//! ([`stream_ns`]), so `ns_totals` keys it without parsing. A record's stream lives with its
//! record: an `append` to it while the record does not exist is refused
//! ([`ServiceError::NotFound`]; a chat turn puts its head first, in the same transaction), and
//! every delete and reclaim of the record takes its stream, so no stream is left that no reclaim
//! reaches. Every name, state, body size and limit is checked before any I/O; a body is at most
//! [`MAX_BODY`].
//!
//! **Pages** (J9): `list` and `read_stream` return at most `limit` rows ([`MAX_LIMIT`] at most;
//! default, measured at build) after the `seq` cursor, oldest `seq` first, and stop early before
//! their bodies pass [`MAX_PAGE_BYTES`]; [`Page::next_after_seq`] is the cursor of the next page,
//! `None` when no row follows. Every statement is an index seek (`records`' (`ns`, `state`, `seq`)
//! index, its two keys, `entries`' (`stream`, `seq`) key): a call's work never grows with the rows
//! held, only with the rows it returns or removes.
//!
//! **`full`** (§4.5): while the budget (row D10) holds a class `full`, a `put`, an `append` and a
//! `set_state` in it answer [`ServiceError::Full`] and write nothing; the three decision calls and
//! a spool ingest are written whatever the class's state (§3 J9; R20), and a delete always runs (it
//! makes room). The ingest takes only the spool's two namespaces ([`SPOOL`]: a proposal or a task,
//! §2.4 rule 2), so the exemption reaches no other record.
//!
//! **Decisions** (§2.4 rule 5; J9): the three decision calls are the only writers of
//! [`DECISIONS_NS`]; every other write to it is refused. [`decision_begin`](StateService::decision_begin)
//! writes the record with `applied: {pending: true}` before the action and refuses an id already
//! decided ([`ServiceError::AlreadyDecided`]: "the record on disk is the guard");
//! [`decision_complete`](StateService::decision_complete) replaces it with the result;
//! [`decision_reopen`](StateService::decision_reopen) removes a pending record whose apply was
//! refused before anything ran.
//!
//! **Reclaim** (J9, J3, §4.4): [`StateService::reclaim`] calls the [`Reclaimer`] each owning module
//! registered for the class's namespaces ([`StateService::register`], its order) and Layer 8's
//! default for the namespaces no module claims: oldest `seq` first across the class's records,
//! protected states skipped, a record's stream deleted with it; then (the J3 order) the class's own
//! streams (`heal/audit`) from their oldest `seq`, after the records. The states a reclaim never
//! touches are declared per namespace by its owner ([`StateService::declare`]); a namespace no
//! owner declared is never reclaimed (fail closed: the owner has not said which of its states may
//! go, so a reclaim that runs before Layer 9 or row D11 registered, at a start, removes none of
//! their records), and while one holds rows the default leaves the class's own streams too (J3:
//! they go after the records). Every removal goes through one guard ([`Reclaim`]): a protected
//! record, its stream, a `pending` decision, the J3 marker and a record in use
//! ([`StateService::in_use`]: the conversation of a turn in flight, §4.4) are never removed, by
//! the default or by a registered reclaimer, and a decision goes only with its proposal (§2.4 rule
//! 5: the record on disk is the guard, so it never goes while its proposal is listed; §4.4).
//!
//! A reclaim of `heal` that removed anything leaves one marker record ([`MARKER_NS`], `pruned
//! through seq N, K records`, [`Marker`]). A reclaim commits in steps of about [`RECLAIM_STEP`]
//! bytes: each step one transaction that ends with `PRAGMA incremental_vacuum` stepped to its end
//! (one step of the pragma frees one page), then a `wal_checkpoint(TRUNCATE)` after its commit,
//! which puts the shrink on disk at once. A freed page is zeroed (`secure_delete`) and a page the
//! vacuum moves is written again, both through the WAL before the checkpoint gives the space back,
//! so the step bounds the room a reclaim needs beside the file ([`Reclaimed::wal_bytes`]): on a
//! disk below its floor (§4.3 step 4, `disk_low`) each step fits and frees its part, where one
//! transaction of the whole would fail and free nothing. It returns the bytes freed from the file
//! ([`Reclaimed::file_bytes`]).
//!
//! **Never adds** (§1 rule 2): a task, a timer, a call whose work grows with the rows held, a clock
//! in an order. The service holds no thread and no file: its registry, the `full` flags and the
//! records in use live in memory beside the daemon's store ([`StateService::current`]).

use super::schema::ROW_OVERHEAD;
use super::{Durability, NodeState, NsTotal, Reason, StateError, Totals, Writer};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::{json, Value};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

/// The most rows one [`list`](StateService::list) or [`read_stream`](StateService::read_stream)
/// page holds (§3 J9: 1,000 at most; default, measured at build). A larger `limit` reads as this.
pub const MAX_LIMIT: usize = 1000;
/// The most body bytes a page holds before it stops early (it always holds one row; its
/// [`Page::next_after_seq`] continues it), so a page's memory stays bounded whatever its rows hold.
pub const MAX_PAGE_BYTES: usize = 4 << 20;
/// The largest body a record or an entry takes: the spool's untrusted-file cap (1 MiB,
/// `N/dashboard/api_v1/proposals.rs`); a chat message frame is at most 512 KiB (§2.3 rule 2).
pub const MAX_BODY: usize = 1 << 20;
/// The longest namespace and the longest id (the project's id rule: 128).
pub const MAX_NAME: usize = 128;
/// The longest stream name: a namespace, `/`, an id.
pub const MAX_STREAM: usize = MAX_NAME * 2 + 1;
/// The longest state.
pub const MAX_STATE: usize = 64;
/// The namespace tops of v1 (§2.2 rule 5): Layer 3's data, Layer 9's, Layer 8's own markers.
pub const TOPS: [&str; 3] = ["agent", "heal", "storage"];
/// Layer 3's decisions (§2.2 rule 5; §2.4 rule 5): written only by the three decision calls. A
/// decision's id is its proposal's ([`PROPOSALS_NS`]), and it is reclaimed only with it.
pub const DECISIONS_NS: &str = "agent/decisions";
/// Layer 3's proposals (§2.2 rule 5; §2.4 rules 2 and 5), Layer 9's included: each one's decision,
/// when it has one, is the record of the same id in [`DECISIONS_NS`].
pub const PROPOSALS_NS: &str = "agent/proposals";
/// The spool's namespaces (§2.4 rule 2): the only ones [`StateService::ingest`] writes.
pub const SPOOL: [&str; 2] = [PROPOSALS_NS, "agent/tasks"];
/// About the most bytes (`ns_totals` measure) one step of a reclaim removes before it commits,
/// vacuums and checkpoints (§4.3 step 4): the WAL a step leaves beside the file stays near it (a
/// freed page is zeroed, `secure_delete`, and a page the vacuum moves is written again, both
/// through the WAL; 1.1 MiB measured for a step that moves every page it frees). A step always
/// removes one row when one may go, and the default trims a longer record's stream from its
/// oldest entry first. It equals [`MAX_BODY`]: a step's WAL is about what the largest write leaves.
pub const RECLAIM_STEP: u64 = 1 << 20;
/// A decision's state from [`StateService::decision_begin`] to its completion: never reclaimed.
pub const PENDING: &str = "pending";
/// A decision's state once [`StateService::decision_complete`] wrote its result.
pub const DECIDED: &str = "decided";
/// The J3 marker's namespace: Layer 8's record in `heal/`, written only by a reclaim of `heal`
/// and never reclaimed.
pub const MARKER_NS: &str = "heal/pruned";
/// The J3 marker's id: one record, rewritten by every reclaim of `heal` that removed anything.
pub const MARKER_ID: &str = "marker";
/// The J3 marker's state.
pub const MARKER_STATE: &str = "marker";

/// A budgeted class the node-state store holds (§4.1 `StorageClass.id`; §4.5 `full`): `agent` is
/// `agent/*`, `heal` is `heal/*`; `storage/` counts in neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Class {
    /// `agent`: Layer 3's conversations, proposals, tasks and decisions (§4.4).
    Agent,
    /// `heal`: Layer 9's repair records (J3).
    Heal,
}

impl Class {
    /// Both classes.
    pub const ALL: [Class; 2] = [Class::Agent, Class::Heal];

    /// The class's id and the top of its namespaces: `agent`, `heal`.
    pub fn id(self) -> &'static str {
        match self {
            Class::Agent => "agent",
            Class::Heal => "heal",
        }
    }

    /// The class `ns` (a record namespace or a top) counts in, by its top; `None` for `storage/`.
    pub fn of(ns: &str) -> Option<Class> {
        match ns.split('/').next() {
            Some("agent") => Some(Class::Agent),
            Some("heal") => Some(Class::Heal),
            _ => None,
        }
    }

    fn index(self) -> usize {
        match self {
            Class::Agent => 0,
            Class::Heal => 1,
        }
    }
}

impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// The states of one namespace a reclaim never touches (§3 J9: declared per namespace, by its
/// owner). A namespace with no declaration reads as [`Protected::All`] (fail closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Protected {
    /// Every record of the namespace (Layer 9's loop ledger).
    All,
    /// The records in these states; the others may go.
    States(Vec<String>),
}

/// One record (§2.2 rule 4 `records`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Its namespace.
    pub ns: String,
    /// Its id inside the namespace.
    pub id: String,
    /// The `seq` of its last write: the only order (J4).
    pub seq: i64,
    /// The boot of its last write (16 bytes, empty when unreadable).
    pub boot_id: Vec<u8>,
    /// `CLOCK_BOOTTIME` in milliseconds at its last write.
    pub mono_ms: i64,
    /// The wall clock at its last write: a display claim only (J4).
    pub wall: Option<String>,
    /// Its state.
    pub state: String,
    /// Its body.
    pub body: Vec<u8>,
}

/// One entry of a stream (§2.2 rule 4 `entries`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its stream.
    pub stream: String,
    /// Its `seq`.
    pub seq: i64,
    /// Its body.
    pub body: Vec<u8>,
}

/// One page of a `list` or a `read_stream` (§3 J9), oldest `seq` first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    /// At most `limit` rows after the cursor.
    pub items: Vec<T>,
    /// The `seq` of the last row when another row follows (the next page's `after_seq`); `None`
    /// on the last page.
    pub next_after_seq: Option<i64>,
}

/// What a [`StateService::reclaim`] removed (§3 J9).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reclaimed {
    /// Records and entries removed.
    pub rows: u64,
    /// Their bytes as `ns_totals` counts them (Σ `length(body)` + 64 per row).
    pub bytes: u64,
    /// The bytes freed from the file (J9's return): its page count times its page size before
    /// each step, less after its `incremental_vacuum`, over the steps.
    pub file_bytes: u64,
    /// The highest `seq` removed.
    pub through_seq: Option<i64>,
    /// The largest `state.db-wal` a step left before its checkpoint (one `stat`): the room the
    /// reclaim needed beside the file (§4.3 step 4 on a full disk; [`RECLAIM_STEP`]).
    pub wal_bytes: u64,
}

impl Reclaimed {
    /// One more step's figures.
    fn absorb(&mut self, step: &Reclaimed) {
        self.rows = self.rows.saturating_add(step.rows);
        self.bytes = self.bytes.saturating_add(step.bytes);
        self.file_bytes = self.file_bytes.saturating_add(step.file_bytes);
        self.through_seq = self.through_seq.max(step.through_seq);
        self.wal_bytes = self.wal_bytes.max(step.wal_bytes);
    }
}

/// The J3 marker's body (§3 J3): "pruned through seq N, K records", over every reclaim of `heal`
/// so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    /// N: the highest `seq` a reclaim of `heal` removed.
    pub through_seq: i64,
    /// K: the records and entries the reclaims of `heal` removed, all together.
    pub records: u64,
}

impl Marker {
    /// The marker in `body` (the record [`MARKER_NS`]/[`MARKER_ID`]), the size checked before the
    /// parse; `None` when it is not one.
    pub fn from_body(body: &[u8]) -> Option<Marker> {
        if body.len() > 4096 {
            return None;
        }
        let v: Value = serde_json::from_slice(body).ok()?;
        Some(Marker {
            through_seq: v.get("pruned_through_seq")?.as_i64()?,
            records: v.get("records")?.as_u64()?,
        })
    }

    fn body(&self) -> Vec<u8> {
        json!({
            "schema": 1,
            "pruned_through_seq": self.through_seq,
            "records": self.records,
        })
        .to_string()
        .into_bytes()
    }
}

/// A state service call's failure.
#[derive(Debug)]
pub enum ServiceError {
    /// The store: unavailable with its reason (§2.2 rule 8), a statement SQLite refused (the
    /// call's transaction rolled back), a damaged value.
    State(StateError),
    /// The class is `full` (§4.5): nothing was written.
    Full(Class),
    /// `decision_begin` on an id that already has a decision record, pending or decided (§2.4
    /// rule 5): the record is unchanged.
    AlreadyDecided,
    /// `decision_complete` or `decision_reopen` on a decision that is not pending.
    NotPending,
    /// No such record: `decision_complete` on an id with no decision record, or an `append` to
    /// the stream of a record that does not exist (a record's stream lives with its record):
    /// nothing was written.
    NotFound,
    /// A name, a state, a body, a limit or a use the service refuses, before any I/O.
    Invalid(&'static str),
}

impl From<StateError> for ServiceError {
    fn from(e: StateError) -> Self {
        ServiceError::State(e)
    }
}

impl From<rusqlite::Error> for ServiceError {
    fn from(e: rusqlite::Error) -> Self {
        ServiceError::State(StateError::Sql(e))
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServiceError::State(e) => e.fmt(f),
            ServiceError::Full(c) => write!(f, "the {c} class is full: nothing was written"),
            ServiceError::AlreadyDecided => f.write_str("this proposal was already decided"),
            ServiceError::NotPending => f.write_str("the decision is not pending"),
            ServiceError::NotFound => f.write_str("no such record"),
            ServiceError::Invalid(why) => write!(f, "the state service refused the call: {why}"),
        }
    }
}

impl std::error::Error for ServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServiceError::State(e) => Some(e),
            _ => None,
        }
    }
}

/// A class's reclaimer, registered by the module that owns its namespaces (§3 J9: "its order and
/// the states it never touches"; Layer 9's for `heal`, row D11's for `agent`), which declares
/// those states ([`StateService::declare`]) for every namespace it reclaims: an undeclared one is
/// out of reach. It removes rows through `r` only, in its own order, until [`Reclaim::done`] or
/// nothing more may go; the guard in [`Reclaim`] refuses a protected record whatever it asks. It
/// is called once per step of the reclaim ([`RECLAIM_STEP`]; [`Reclaim::wanted`] is the step's
/// part), inside that step's transaction, under the store's I/O lock: a call into the
/// [`StateService`] from inside it is refused ([`ServiceError::Invalid`]) rather than left waiting
/// on itself. A step that removed less than it wanted is the last. An error rolls the step back
/// and ends the reclaim with it (the steps before it stay committed). [`Reclaim::remove`] takes a
/// record with its whole stream: a reclaimer that removes a long one ([`Reclaim::left`]) trims it
/// first ([`Reclaim::trim`], oldest first), so the step's WAL stays near the step.
pub trait Reclaimer: Send + Sync {
    /// Free [`Reclaim::wanted`] bytes from the namespaces of [`Reclaim::namespaces`].
    fn reclaim(&self, r: &mut Reclaim<'_>) -> Result<(), ServiceError>;
}

/// Which namespaces a [`Reclaim`] may touch.
#[derive(Debug, Clone)]
enum Allow {
    /// A registered reclaimer: the namespaces of its scope.
    Scope(String),
    /// Layer 8's default: the class's namespaces no registered scope covers.
    Rest(Vec<String>),
}

/// One reclaim step's removals inside its transaction (§3 J9, J3): the only way a reclaimer
/// removes a row, through one guard. A record in a protected state (its namespace's declaration;
/// every state of a namespace with none), its stream, a `pending` decision, the J3 marker and a
/// record in use ([`StateService::in_use`]) are never removed; a decision goes only with its
/// proposal (removing the proposal takes its decision, and a proposal whose decision may not go
/// stays); a row outside the reclaimer's scope is refused. It counts what it removed (rows, bytes
/// as `ns_totals` counts them, the highest `seq`).
pub struct Reclaim<'a> {
    w: &'a Writer<'a>,
    class: Class,
    want: u64,
    freed: u64,
    rows: u64,
    through: Option<i64>,
    declared: &'a BTreeMap<String, Protected>,
    in_use: &'a Mutex<BTreeMap<(String, String), usize>>,
    allow: Allow,
}

impl fmt::Debug for Reclaim<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reclaim")
            .field("class", &self.class)
            .field("want", &self.want)
            .field("freed", &self.freed)
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

impl<'a> Reclaim<'a> {
    /// The class being reclaimed.
    pub fn class(&self) -> Class {
        self.class
    }

    /// The bytes this step asks for (`ns_totals` measure): at most [`RECLAIM_STEP`].
    pub fn wanted(&self) -> u64 {
        self.want
    }

    /// The bytes removed so far in this step, by every reclaimer of it.
    pub fn freed(&self) -> u64 {
        self.freed
    }

    /// The bytes this step still asks for.
    pub fn left(&self) -> u64 {
        self.want.saturating_sub(self.freed)
    }

    /// Whether the bytes this step asks for are removed.
    pub fn done(&self) -> bool {
        self.freed >= self.want
    }

    /// Whether a record of `ns` in `state` is out of a reclaim's reach (§3 J9; §2.4 rule 5): the
    /// J3 marker, a `pending` decision, else the namespace's declaration; every state of a
    /// namespace with none.
    pub fn is_protected(&self, ns: &str, state: &str) -> bool {
        protected_in(self.declared, ns, state)
    }

    /// Whether a turn or an operation holds the record (`ns`, `id`) ([`StateService::in_use`]).
    pub fn is_in_use(&self, ns: &str, id: &str) -> bool {
        held_in_use(self.in_use, ns, id)
    }

    /// The class's namespaces this reclaimer may touch that hold rows (from `ns_totals`, one
    /// index range): record namespaces, and the class's top when its own streams hold entries.
    pub fn namespaces(&self) -> Result<Vec<String>, ServiceError> {
        let top = self.class.id();
        let hi = format!("{top}0");
        let mut st = self.conn().prepare_cached(CLASS_NS)?;
        let rows = st.query_map(params![top, hi], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for ns in rows {
            let ns = ns?;
            if within(&ns, top) && self.allows(&ns) {
                out.push(ns);
            }
        }
        Ok(out)
    }

    /// J9's `get`, inside the reclaim's transaction.
    pub fn get(&self, ns: &str, id: &str) -> Result<Option<Record>, ServiceError> {
        check_ns(ns)?;
        check_id(id)?;
        get_in(self.conn(), ns, id)
    }

    /// J9's `list`, inside the reclaim's transaction.
    pub fn list(
        &self,
        ns: &str,
        state: Option<&str>,
        after_seq: Option<i64>,
        limit: usize,
    ) -> Result<Page<Record>, ServiceError> {
        check_list(ns, state, limit)?;
        list_in(self.conn(), ns, state, after_seq.unwrap_or(0), limit)
    }

    /// J9's `read_stream`, inside the reclaim's transaction.
    pub fn read_stream(
        &self,
        stream: &str,
        after_seq: Option<i64>,
        limit: usize,
    ) -> Result<Page<Entry>, ServiceError> {
        check_stream(stream)?;
        check_limit(limit)?;
        read_stream_in(self.conn(), stream, after_seq.unwrap_or(0), limit)
    }

    /// Remove the record (`ns`, `id`) with its stream `<ns>/<id>`: `Ok(true)` when removed,
    /// `Ok(false)` when there is none, it is protected or in use, or it is a decision whose
    /// proposal is still held (then nothing of it is touched). A proposal goes with its decision
    /// when it has one, and stays when that decision may not go (`pending`, or protected by its
    /// namespace's declaration): §2.4 rule 5's guard and its proposal leave together (§4.4). A
    /// namespace outside this reclaimer's scope is refused.
    pub fn remove(&mut self, ns: &str, id: &str) -> Result<bool, ServiceError> {
        check_ns(ns)?;
        check_id(id)?;
        if !self.allows(ns) {
            return Err(ServiceError::Invalid("outside the reclaimer's scope"));
        }
        let Some((seq, state, len)) = head(self.conn(), ns, id)? else {
            return Ok(false);
        };
        if self.is_protected(ns, &state) || self.is_in_use(ns, id) {
            return Ok(false);
        }
        // the record on disk is the guard (§2.4 rule 5): a decision never goes while its proposal
        // is listed, so a decided proposal can never read open again and be applied twice
        if ns == DECISIONS_NS && exists(self.conn(), PROPOSALS_NS, id)? {
            return Ok(false);
        }
        let decision = if ns == PROPOSALS_NS {
            head(self.conn(), DECISIONS_NS, id)?
        } else {
            None
        };
        if let Some((_, decided, _)) = &decision {
            if self.is_protected(DECISIONS_NS, decided) {
                return Ok(false);
            }
        }
        self.conn()
            .prepare_cached(DELETE_RECORD)?
            .execute(params![ns, id])?;
        self.count(1, u64::try_from(len).unwrap_or(0), Some(seq));
        if let Some((dseq, _, dlen)) = decision {
            self.conn()
                .prepare_cached(DELETE_RECORD)?
                .execute(params![DECISIONS_NS, id])?;
            self.count(1, u64::try_from(dlen).unwrap_or(0), Some(dseq));
        }
        let stream = stream_of(ns, id);
        let (n, bytes, top) = span(self.conn(), &stream, i64::MAX)?;
        if n > 0 {
            self.conn()
                .prepare_cached(TRIM)?
                .execute(params![stream, i64::MAX])?;
            self.count(n, bytes, top);
        }
        Ok(true)
    }

    /// Remove `stream`'s entries through `through_seq` (oldest first): the entries removed. A
    /// stream whose head record is protected or in use is left whole (`Ok(0)`); a stream outside
    /// this reclaimer's scope is refused.
    pub fn trim(&mut self, stream: &str, through_seq: i64) -> Result<u64, ServiceError> {
        let ns = check_stream(stream)?;
        if !self.allows(ns) {
            return Err(ServiceError::Invalid("outside the reclaimer's scope"));
        }
        if let Some((head_ns, id)) = head_of(stream) {
            let state: Option<String> = self
                .conn()
                .prepare_cached(STATE_OF)?
                .query_row(params![head_ns, id], |r| r.get(0))
                .optional()?;
            if state.is_some_and(|s| self.is_protected(head_ns, &s)) || self.is_in_use(head_ns, id)
            {
                return Ok(0);
            }
        }
        let (n, bytes, top) = span(self.conn(), stream, through_seq)?;
        if n == 0 {
            return Ok(0);
        }
        self.conn()
            .prepare_cached(TRIM)?
            .execute(params![stream, through_seq])?;
        self.count(n, bytes, top);
        Ok(n)
    }

    /// Remove all of `stream` (as [`Reclaim::trim`] through every `seq`).
    pub fn remove_stream(&mut self, stream: &str) -> Result<u64, ServiceError> {
        self.trim(stream, i64::MAX)
    }

    fn conn(&self) -> &'a Connection {
        self.w.tx()
    }

    fn allows(&self, ns: &str) -> bool {
        match &self.allow {
            Allow::Scope(scope) => within(ns, scope),
            Allow::Rest(claimed) => {
                Class::of(ns) == Some(self.class) && !claimed.iter().any(|c| within(ns, c))
            }
        }
    }

    fn count(&mut self, rows: u64, body_bytes: u64, top: Option<i64>) {
        self.rows = self.rows.saturating_add(rows);
        self.freed = self
            .freed
            .saturating_add(body_bytes)
            .saturating_add(rows.saturating_mul(ROW_OVERHEAD));
        if let Some(s) = top {
            self.through = Some(self.through.map_or(s, |t| t.max(s)));
        }
    }
}

/// J9's writes and reads inside a caller's own transaction ([`NodeState::write`]): Layer 8's own
/// modules compose them where one transaction must hold several (a chat turn's messages and its
/// head, §2.3 rule 3; a spool ingest, §2.4 rule 2; the legacy import, §2.5). The same names,
/// checks and `seq` rule as the [`StateService`] calls; no `full` check (the caller reads
/// [`StateService::is_full`]), and [`DECISIONS_NS`] and [`MARKER_NS`] are refused here too.
pub struct Ops<'a> {
    w: &'a Writer<'a>,
}

impl fmt::Debug for Ops<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ops").finish_non_exhaustive()
    }
}

impl<'a> Ops<'a> {
    /// The calls over `w`'s transaction.
    pub fn new(w: &'a Writer<'a>) -> Ops<'a> {
        Ops { w }
    }

    /// J9's `put` (a new `seq`, returned).
    pub fn put(&self, ns: &str, id: &str, state: &str, body: &[u8]) -> Result<i64, ServiceError> {
        check_record_write(ns, id, state, body)?;
        put_raw(self.w, ns, id, state, body)
    }

    /// J9's `append` (a new `seq`, returned). The stream of a record takes an entry only while
    /// the record exists ([`ServiceError::NotFound`] otherwise; put the head first, in this
    /// transaction).
    pub fn append(&self, stream: &str, body: &[u8]) -> Result<i64, ServiceError> {
        let ns = check_stream_write(stream, body)?;
        if let Some((head_ns, id)) = head_of(stream) {
            if !exists(self.w.tx(), head_ns, id)? {
                return Err(ServiceError::NotFound);
            }
        }
        let seq = self.w.next_seq()?;
        self.w
            .tx()
            .prepare_cached(APPEND)?
            .execute(params![ns, stream, seq, body])?;
        Ok(seq)
    }

    /// J9's `set_state`: the new `seq`, or `None` (nothing written) with no such record.
    pub fn set_state(&self, ns: &str, id: &str, state: &str) -> Result<Option<i64>, ServiceError> {
        check_record_write(ns, id, state, b"")?;
        if !exists(self.w.tx(), ns, id)? {
            return Ok(None);
        }
        let s = self.w.stamp()?;
        self.w
            .tx()
            .prepare_cached(SET_STATE)?
            .execute(params![ns, id, state, s.seq, s.boot_id, s.mono_ms, s.wall])?;
        Ok(Some(s.seq))
    }

    /// J9's `delete`: the record and its stream `<ns>/<id>`; whether the record was there.
    pub fn delete(&self, ns: &str, id: &str) -> Result<bool, ServiceError> {
        check_record_delete(ns, id)?;
        let n = self
            .w
            .tx()
            .prepare_cached(DELETE_RECORD)?
            .execute(params![ns, id])?;
        self.w
            .tx()
            .prepare_cached(TRIM)?
            .execute(params![stream_of(ns, id), i64::MAX])?;
        Ok(n > 0)
    }

    /// J9's `delete_stream`: the entries removed.
    pub fn delete_stream(&self, stream: &str) -> Result<u64, ServiceError> {
        self.trim_stream(stream, i64::MAX)
    }

    /// J9's `trim_stream`: `stream`'s entries through `through_seq` removed; the entries removed.
    pub fn trim_stream(&self, stream: &str, through_seq: i64) -> Result<u64, ServiceError> {
        check_stream_write(stream, b"")?;
        let n = self
            .w
            .tx()
            .prepare_cached(TRIM)?
            .execute(params![stream, through_seq])?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// An insert by id (§2.4 rule 2's `INSERT OR IGNORE`, §2.5's idempotent import): the record
    /// when the id is new (its new `seq`), `None` when it is there already (nothing written). Any
    /// record namespace but the two the service keeps ([`DECISIONS_NS`], [`MARKER_NS`]), for the
    /// spool and the legacy import; no `full` check here either.
    pub fn ingest(
        &self,
        ns: &str,
        id: &str,
        state: &str,
        body: &[u8],
    ) -> Result<Option<i64>, ServiceError> {
        check_record_write(ns, id, state, body)?;
        if exists(self.w.tx(), ns, id)? {
            return Ok(None);
        }
        put_raw(self.w, ns, id, state, body).map(Some)
    }

    /// J9's `get`, inside the transaction.
    pub fn get(&self, ns: &str, id: &str) -> Result<Option<Record>, ServiceError> {
        check_ns(ns)?;
        check_id(id)?;
        get_in(self.w.tx(), ns, id)
    }

    /// J9's `list`, inside the transaction.
    pub fn list(
        &self,
        ns: &str,
        state: Option<&str>,
        after_seq: Option<i64>,
        limit: usize,
    ) -> Result<Page<Record>, ServiceError> {
        check_list(ns, state, limit)?;
        list_in(self.w.tx(), ns, state, after_seq.unwrap_or(0), limit)
    }

    /// J9's `read_stream`, inside the transaction.
    pub fn read_stream(
        &self,
        stream: &str,
        after_seq: Option<i64>,
        limit: usize,
    ) -> Result<Page<Entry>, ServiceError> {
        check_stream(stream)?;
        check_limit(limit)?;
        read_stream_in(self.w.tx(), stream, after_seq.unwrap_or(0), limit)
    }
}

/// What the service keeps beside one store: the registry (§3 J9), the `full` flags (§4.5) and the
/// records in use (§4.4), each a count of the [`InUse`] marks held.
#[derive(Default)]
struct Inner {
    full: [AtomicBool; 2],
    reclaimers: Mutex<Vec<(String, Arc<dyn Reclaimer>)>>,
    declared: Mutex<BTreeMap<String, Protected>>,
    in_use: Mutex<BTreeMap<(String, String), usize>>,
}

/// A record a turn or an operation is using ([`StateService::in_use`]; §4.4: "never ... the
/// conversation of a turn in flight"): while one lives, no reclaim removes the record or trims its
/// stream. Memory only; dropping it ends the mark.
#[must_use = "the record may be reclaimed once the mark is dropped"]
pub struct InUse {
    inner: Arc<Inner>,
    key: (String, String),
}

impl fmt::Debug for InUse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InUse")
            .field("ns", &self.key.0)
            .field("id", &self.key.1)
            .finish()
    }
}

impl Drop for InUse {
    fn drop(&mut self) {
        let mut held = lock(&self.inner.in_use);
        if let Some(n) = held.get_mut(&self.key) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                held.remove(&self.key);
            }
        }
    }
}

/// Whether an [`InUse`] mark holds the record (`ns`, `id`).
fn held_in_use(in_use: &Mutex<BTreeMap<(String, String), usize>>, ns: &str, id: &str) -> bool {
    lock(in_use).contains_key(&(ns.to_string(), id.to_string()))
}

/// The services of the stores alive in this process (one per daemon), keyed by the store.
static SERVICES: Mutex<Vec<(Weak<NodeState>, Arc<Inner>)>> = Mutex::new(Vec::new());

thread_local! {
    /// Set while a reclaim runs on this thread (its reclaimers run inside the store's I/O lock).
    static IN_RECLAIM: Cell<bool> = const { Cell::new(false) };
}

/// The reclaim's mark on its thread, cleared on every way out (a panic included).
struct InReclaim;

impl InReclaim {
    fn enter() -> InReclaim {
        IN_RECLAIM.with(|c| c.set(true));
        InReclaim
    }
}

impl Drop for InReclaim {
    fn drop(&mut self) {
        IN_RECLAIM.with(|c| c.set(false));
    }
}

/// Every call's first check: a reclaimer that called the service would wait on the store's I/O
/// lock its own reclaim holds.
fn enter() -> Result<(), ServiceError> {
    if IN_RECLAIM.with(Cell::get) {
        return Err(ServiceError::Invalid(
            "a reclaimer called the state service; it reaches the store only through its Reclaim",
        ));
    }
    Ok(())
}

/// The J9 state service (§3 J9) over one node-state store: the calls, the reclaimer registry and
/// the `full` flags. Cheap to clone; every clone of [`StateService::of`] the same store shares one
/// registry and one set of flags. Every call is a blocking call (the store's own rule), run off the
/// runtime's workers.
#[derive(Clone)]
pub struct StateService {
    store: Arc<NodeState>,
    inner: Arc<Inner>,
}

impl fmt::Debug for StateService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StateService")
            .field("store", &self.store)
            .field("agent_full", &self.is_full(Class::Agent))
            .field("heal_full", &self.is_full(Class::Heal))
            .finish_non_exhaustive()
    }
}

impl StateService {
    /// The service of the daemon's store ([`super::current`]): `None` before `state::start` and
    /// after the daemon stopped. Layer 9's modules, the budget and Layer 8's routes reach it here.
    pub fn current() -> Option<StateService> {
        super::current().map(StateService::of)
    }

    /// The service of `store`: the one registry and set of flags kept beside it while it lives.
    pub fn of(store: Arc<NodeState>) -> StateService {
        let mut all = SERVICES.lock().unwrap_or_else(PoisonError::into_inner);
        all.retain(|(w, _)| w.strong_count() > 0);
        // `store` holds its allocation: a live entry at the same address is this store
        let found = all
            .iter()
            .find(|(w, _)| std::ptr::eq(w.as_ptr(), Arc::as_ptr(&store)))
            .map(|(_, inner)| Arc::clone(inner));
        let inner = found.unwrap_or_else(|| {
            let inner = Arc::new(Inner::default());
            all.push((Arc::downgrade(&store), Arc::clone(&inner)));
            inner
        });
        StateService { store, inner }
    }

    /// The store under the service.
    pub fn store(&self) -> &Arc<NodeState> {
        &self.store
    }

    /// J9 `put(ns, id, state, body)`: the record written (a new one, or the existing one replaced),
    /// with a new `seq`, returned. `Full` while the namespace's class is `full`.
    pub fn put(&self, ns: &str, id: &str, state: &str, body: &[u8]) -> Result<i64, ServiceError> {
        enter()?;
        check_record_write(ns, id, state, body)?;
        self.not_full(ns)?;
        self.store
            .write(Durability::Normal, |w| Ops::new(w).put(ns, id, state, body))
    }

    /// J9 `get(ns, id)`: the record, or `None`. Reads only (§2.2 rule 9).
    pub fn get(&self, ns: &str, id: &str) -> Result<Option<Record>, ServiceError> {
        enter()?;
        check_ns(ns)?;
        check_id(id)?;
        self.store.read(|c| get_in(c, ns, id))
    }

    /// J9 `list(ns, state?, after_seq?, limit)`: at most `limit` records of `ns` ([`MAX_LIMIT`]
    /// at most), in `state` when given, with a `seq` above `after_seq`, oldest `seq` first. Reads
    /// only.
    pub fn list(
        &self,
        ns: &str,
        state: Option<&str>,
        after_seq: Option<i64>,
        limit: usize,
    ) -> Result<Page<Record>, ServiceError> {
        enter()?;
        check_list(ns, state, limit)?;
        self.store
            .read(|c| list_in(c, ns, state, after_seq.unwrap_or(0), limit))
    }

    /// J9 `append(stream, body) -> seq`: one entry with a new `seq`, returned. `Full` while the
    /// stream's class is `full`; [`ServiceError::NotFound`] for the stream of a record that does
    /// not exist (nothing written).
    pub fn append(&self, stream: &str, body: &[u8]) -> Result<i64, ServiceError> {
        enter()?;
        let ns = check_stream_write(stream, body)?;
        self.not_full(ns)?;
        if head_of(stream).is_some() && self.nothing_held()? {
            return Err(ServiceError::NotFound);
        }
        self.store
            .write(Durability::Normal, |w| Ops::new(w).append(stream, body))
    }

    /// J9 `read_stream(stream, after_seq?, limit)`: at most `limit` entries ([`MAX_LIMIT`] at
    /// most) with a `seq` above `after_seq`, oldest first. Reads only.
    pub fn read_stream(
        &self,
        stream: &str,
        after_seq: Option<i64>,
        limit: usize,
    ) -> Result<Page<Entry>, ServiceError> {
        enter()?;
        check_stream(stream)?;
        check_limit(limit)?;
        self.store
            .read(|c| read_stream_in(c, stream, after_seq.unwrap_or(0), limit))
    }

    /// J9 `set_state(ns, id, state)`: the record's state, with a new `seq`, returned; `None` with
    /// no such record (nothing written). `Full` while the namespace's class is `full`.
    pub fn set_state(&self, ns: &str, id: &str, state: &str) -> Result<Option<i64>, ServiceError> {
        enter()?;
        check_record_write(ns, id, state, b"")?;
        self.not_full(ns)?;
        if self.nothing_held()? {
            return Ok(None);
        }
        self.store
            .write(Durability::Normal, |w| Ops::new(w).set_state(ns, id, state))
    }

    /// J9 `delete(ns, id)`: the record and its stream `<ns>/<id>` (a stream goes with its head, as
    /// in a reclaim); whether the record was there. Runs whatever the class's state.
    pub fn delete(&self, ns: &str, id: &str) -> Result<bool, ServiceError> {
        enter()?;
        check_record_delete(ns, id)?;
        if self.nothing_held()? {
            return Ok(false);
        }
        self.store
            .write(Durability::Normal, |w| Ops::new(w).delete(ns, id))
    }

    /// J9 `delete_stream(stream)`: every entry of `stream`; the entries removed.
    pub fn delete_stream(&self, stream: &str) -> Result<u64, ServiceError> {
        self.trim_stream(stream, i64::MAX)
    }

    /// J9 `trim_stream(stream, through_seq)`: `stream`'s entries with a `seq` up to `through_seq`;
    /// the entries removed. Runs whatever the class's state.
    pub fn trim_stream(&self, stream: &str, through_seq: i64) -> Result<u64, ServiceError> {
        enter()?;
        check_stream_write(stream, b"")?;
        if self.nothing_held()? {
            return Ok(0);
        }
        self.store.write(Durability::Normal, |w| {
            Ops::new(w).trim_stream(stream, through_seq)
        })
    }

    /// J9 `usage() -> {ns: (rows, bytes)}`: the totals the daemon keeps in memory (§2.2 rule 7);
    /// the first call after a start with none known yet opens the store to read them.
    pub fn usage(&self) -> Result<Totals, ServiceError> {
        enter()?;
        Ok(self.store.load_totals()?)
    }

    /// The rows and bytes of `class` (§4.1: `ns_totals` of `agent/*`, of `heal/*`), from
    /// [`StateService::usage`].
    pub fn usage_of(&self, class: Class) -> Result<NsTotal, ServiceError> {
        Ok(sum_of(&self.usage()?, class))
    }

    /// §2.4 rule 2: a spool ingest, the record inserted by id (`INSERT OR IGNORE`), committed
    /// `FULL` before the caller unlinks the file: its new `seq`, or `None` when the id is there
    /// already (nothing written, a repeat after a crash between commit and unlink). Written
    /// whatever the class's state (§4.5; R20), so it takes only the spool's namespaces ([`SPOOL`]:
    /// a proposal or a task); any other is refused ([`ServiceError::Invalid`]) and goes through
    /// `put`, which answers `Full`.
    pub fn ingest(
        &self,
        ns: &str,
        id: &str,
        state: &str,
        body: &[u8],
    ) -> Result<Option<i64>, ServiceError> {
        enter()?;
        check_record_write(ns, id, state, body)?;
        if !SPOOL.contains(&ns) {
            return Err(ServiceError::Invalid(
                "a spool ingest is a proposal or a task (agent/proposals, agent/tasks)",
            ));
        }
        self.store.write(Durability::Full, |w| {
            Ops::new(w).ingest(ns, id, state, body)
        })
    }

    /// J9 `decision_begin(id, pending)` (`FULL`; §2.4 rule 5): the decision record `pending` (a
    /// JSON object, `{schema, decision, at}`) written with `applied: {pending: true}` in state
    /// [`PENDING`], before the action; its new `seq`. An id that has a decision record already,
    /// pending or decided, is refused with [`ServiceError::AlreadyDecided`] and left unchanged.
    /// Written whatever the class's state.
    pub fn decision_begin(&self, id: &str, pending: &Value) -> Result<i64, ServiceError> {
        enter()?;
        check_id(id)?;
        let Value::Object(fields) = pending else {
            return Err(ServiceError::Invalid("a decision record is a JSON object"));
        };
        let mut fields = fields.clone();
        fields.insert("applied".into(), json!({ "pending": true }));
        let body = Value::Object(fields).to_string().into_bytes();
        check_body(&body)?;
        self.store.write(Durability::Full, |w| {
            if exists(w.tx(), DECISIONS_NS, id)? {
                return Err(ServiceError::AlreadyDecided);
            }
            put_raw(w, DECISIONS_NS, id, PENDING, &body)
        })
    }

    /// J9 `decision_complete(id, applied)` (`FULL`): the pending record's `applied` replaced with
    /// the result (`None` removes it: a decision that applied nothing), state [`DECIDED`]; its new
    /// `seq`. [`ServiceError::NotFound`] with no record, [`ServiceError::NotPending`] when it is not
    /// pending. Written whatever the class's state.
    pub fn decision_complete(
        &self,
        id: &str,
        applied: Option<&Value>,
    ) -> Result<i64, ServiceError> {
        enter()?;
        check_id(id)?;
        if self.nothing_held()? {
            return Err(ServiceError::NotFound);
        }
        self.store.write(Durability::Full, |w| {
            let held: Option<(String, Vec<u8>)> = w
                .tx()
                .prepare_cached(STATE_AND_BODY)?
                .query_row(params![DECISIONS_NS, id], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            let Some((state, body)) = held else {
                return Err(ServiceError::NotFound);
            };
            if state != PENDING {
                return Err(ServiceError::NotPending);
            }
            let mut fields = decision_fields(&body)?;
            match applied {
                Some(v) => {
                    fields.insert("applied".into(), v.clone());
                }
                None => {
                    fields.remove("applied");
                }
            }
            let body = Value::Object(fields).to_string().into_bytes();
            check_body(&body)?;
            put_raw(w, DECISIONS_NS, id, DECIDED, &body)
        })
    }

    /// J9 `decision_reopen(id)`: a pending record whose apply was refused before anything ran is
    /// removed (the proposal is open again); `Ok(false)` with no record,
    /// [`ServiceError::NotPending`] for a decided one (never removed here).
    pub fn decision_reopen(&self, id: &str) -> Result<bool, ServiceError> {
        enter()?;
        check_id(id)?;
        if self.nothing_held()? {
            return Ok(false);
        }
        self.store.write(Durability::Normal, |w| {
            let state: Option<String> = w
                .tx()
                .prepare_cached(STATE_OF)?
                .query_row(params![DECISIONS_NS, id], |r| r.get(0))
                .optional()?;
            match state.as_deref() {
                None => Ok(false),
                Some(PENDING) => {
                    w.tx()
                        .prepare_cached(DELETE_RECORD)?
                        .execute(params![DECISIONS_NS, id])?;
                    Ok(true)
                }
                Some(_) => Err(ServiceError::NotPending),
            }
        })
    }

    /// J9 `reclaim(class, bytes) -> freed` (§3 J9, J3; §4.3 step 4): remove at least `bytes`
    /// (`ns_totals` measure) of `class`, or all it may, in steps of about [`RECLAIM_STEP`]. Each
    /// step is one transaction: each registered reclaimer of the class over its scope, in
    /// registration order, then Layer 8's default over the namespaces none claims (records oldest
    /// `seq` first, protected states skipped, a record's stream with it; then the class's own
    /// streams from their oldest `seq`); the J3 marker when it removed anything of `heal`; `PRAGMA
    /// incremental_vacuum`; the commit; then a `wal_checkpoint(TRUNCATE)`, so each step's shrink
    /// is on disk before the next begins and the WAL beside the file stays near one step. The
    /// steps end once `bytes` are removed, once a step removed less than it asked for (nothing more
    /// may go), or once a checkpoint did not complete (a reader holds the WAL: another step would
    /// only grow it). An error rolls its step back and ends the reclaim with it; the steps before
    /// it stay. What it removed and the bytes freed from the file. A class that holds no row (by
    /// the totals in memory, read first when none are known yet, §2.2 rule 7) writes nothing.
    pub fn reclaim(&self, class: Class, bytes: u64) -> Result<Reclaimed, ServiceError> {
        enter()?;
        if bytes == 0 || sum_of(&self.store.load_totals()?, class).rows == 0 {
            return Ok(Reclaimed::default());
        }
        let registered: Vec<(String, Arc<dyn Reclaimer>)> = lock(&self.inner.reclaimers)
            .iter()
            .filter(|(scope, _)| Class::of(scope) == Some(class))
            .map(|(scope, r)| (scope.clone(), Arc::clone(r)))
            .collect();
        let declared = lock(&self.inner.declared).clone();
        let mut out = Reclaimed::default();
        loop {
            let want = bytes.saturating_sub(out.bytes).min(RECLAIM_STEP);
            let mut step = self.store.write(Durability::Normal, |w| {
                self.step(w, class, want, &registered, &declared)
            })?;
            let wrote = step.rows > 0 || step.file_bytes > 0;
            let checkpointed = if wrote {
                step.wal_bytes = self.wal_len();
                self.checkpoint()
            } else {
                true
            };
            out.absorb(&step);
            if !checkpointed || step.rows == 0 || step.bytes < want || out.bytes >= bytes {
                return Ok(out);
            }
        }
    }

    /// One step of [`StateService::reclaim`], inside its transaction: at least `want` bytes, or
    /// all that may go; the marker; the vacuum.
    fn step(
        &self,
        w: &Writer<'_>,
        class: Class,
        want: u64,
        registered: &[(String, Arc<dyn Reclaimer>)],
        declared: &BTreeMap<String, Protected>,
    ) -> Result<Reclaimed, ServiceError> {
        let _mark = InReclaim::enter();
        let before = file_size(w.tx())?;
        let mut r = Reclaim {
            w,
            class,
            want,
            freed: 0,
            rows: 0,
            through: None,
            declared,
            in_use: &self.inner.in_use,
            allow: Allow::Rest(Vec::new()),
        };
        for (scope, reclaimer) in registered {
            if r.done() {
                break;
            }
            r.allow = Allow::Scope(scope.clone());
            reclaimer.reclaim(&mut r)?;
        }
        if !r.done() {
            r.allow = Allow::Rest(registered.iter().map(|(s, _)| s.clone()).collect());
            default_reclaim(&mut r)?;
        }
        let (rows, freed, through) = (r.rows, r.freed, r.through);
        if let (Class::Heal, Some(top)) = (class, through) {
            write_marker(w, top, rows)?;
        }
        vacuum(w.tx())?;
        let after = file_size(w.tx())?;
        Ok(Reclaimed {
            rows,
            bytes: freed,
            file_bytes: before.saturating_sub(after),
            through_seq: through,
            wal_bytes: 0,
        })
    }

    /// Mark the record (`ns`, `id`) in use while the returned [`InUse`] lives (§4.4: "never ...
    /// the conversation of a turn in flight"; a chat turn holds its conversation's head from its
    /// history read to its append): no reclaim removes it or trims its stream, by the default or
    /// by a registered reclaimer. Memory only, no I/O; marks of one record count.
    pub fn in_use(&self, ns: &str, id: &str) -> Result<InUse, ServiceError> {
        check_ns(ns)?;
        check_id(id)?;
        let key = (ns.to_string(), id.to_string());
        *lock(&self.inner.in_use).entry(key.clone()).or_insert(0) += 1;
        Ok(InUse {
            inner: Arc::clone(&self.inner),
            key,
        })
    }

    /// Register `reclaimer` for the namespaces of `scope` (§3 J9: a class's top, `agent` or `heal`,
    /// or one of its record namespaces): it runs in place of the default for every namespace the
    /// scope covers. A scope registered again is replaced; one that overlaps another (an ancestor
    /// or a descendant of it) is refused. Its module declares the states of each namespace it
    /// reclaims ([`StateService::declare`]): an undeclared one is out of every reclaim's reach.
    pub fn register(&self, scope: &str, reclaimer: Arc<dyn Reclaimer>) -> Result<(), ServiceError> {
        if !(Class::of(scope).is_some() && (TOPS.contains(&scope) || valid_ns(scope))) {
            return Err(ServiceError::Invalid(
                "a reclaimer scope is agent, heal or one of their namespaces",
            ));
        }
        let mut all = lock(&self.inner.reclaimers);
        if all
            .iter()
            .any(|(s, _)| s != scope && (within(s, scope) || within(scope, s)))
        {
            return Err(ServiceError::Invalid("a reclaimer scope overlaps another"));
        }
        match all.iter_mut().find(|(s, _)| s == scope) {
            Some(slot) => slot.1 = reclaimer,
            None => all.push((scope.to_string(), reclaimer)),
        }
        Ok(())
    }

    /// Declare the states of `ns` a reclaim never touches (§3 J9: each namespace's own
    /// declaration, by its owner, before the first reclaim that should reach it: until then every
    /// record of `ns` is out of reach, fail closed). A `pending` decision, the J3 marker and a
    /// record in use stay protected whatever is declared.
    pub fn declare(&self, ns: &str, protected: Protected) -> Result<(), ServiceError> {
        check_ns(ns)?;
        if let Protected::States(states) = &protected {
            if !states.iter().all(|s| valid_state(s)) {
                return Err(ServiceError::Invalid("a declared state is not a state"));
            }
        }
        lock(&self.inner.declared).insert(ns.to_string(), protected);
        Ok(())
    }

    /// Whether a reclaim would leave a record of `ns` in `state` (the declarations in force).
    pub fn is_protected(&self, ns: &str, state: &str) -> bool {
        protected_in(&lock(&self.inner.declared), ns, state)
    }

    /// §4.5: hold `class` `full` (`true`) or let it grow again (`false`), as the budget decides on
    /// its pass (row D10); the previous value.
    pub fn set_full(&self, class: Class, full: bool) -> bool {
        self.inner.full[class.index()].swap(full, SeqCst)
    }

    /// Whether `class` is `full` (§4.5): a `put`, an `append` or a `set_state` in it is refused.
    pub fn is_full(&self, class: Class) -> bool {
        self.inner.full[class.index()].load(SeqCst)
    }

    fn not_full(&self, ns: &str) -> Result<(), ServiceError> {
        match Class::of(ns) {
            Some(c) if self.is_full(c) => Err(ServiceError::Full(c)),
            _ => Ok(()),
        }
    }

    /// Whether there is no `state.db` at all (one `stat`): a call that only removes or updates then
    /// answers at once and does not create the store. A store closed for the run (§2.2 rule 8)
    /// answers its reason, as every call does.
    fn nothing_held(&self) -> Result<bool, ServiceError> {
        if let Some(r) = self.store.unavailable().filter(Reason::is_sticky) {
            return Err(StateError::Unavailable(r).into());
        }
        Ok(self.store.file_bytes().is_none() && !self.store.is_open())
    }

    /// The shrink on disk after a reclaim step: the WAL's pages copied into the file, the file cut
    /// to its size and the WAL to zero. Whether it completed: a reader that blocks it (`busy`)
    /// leaves the work to the next close (§2.2 rule 7). A store a stopping daemon closed after the
    /// step was checkpointed by its close.
    fn checkpoint(&self) -> bool {
        if !self.store.is_open() {
            return true;
        }
        let res = self.store.read(|c| -> Result<i64, ServiceError> {
            Ok(c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?)
        });
        match res {
            Ok(0) => true,
            Ok(_) => {
                eprintln!(
                    "111 state: node/state.db: a reader held the WAL; the reclaim stops here"
                );
                false
            }
            Err(e) => {
                eprintln!("111 state: node/state.db: the checkpoint after a reclaim failed: {e}");
                false
            }
        }
    }

    /// `state.db-wal`'s size, one `stat` (0 with none).
    fn wal_len(&self) -> u64 {
        fs::metadata(super::companion(self.store.path(), "-wal")).map_or(0, |m| m.len())
    }
}

/// Layer 8's default reclaimer (§3 J9, J3): the class's records oldest `seq` first across its
/// declared namespaces, protected states skipped, each with its stream; then the class's own
/// streams (`heal/audit`) from their oldest `seq`. A namespace no owner declared is left whole,
/// and while one holds rows the class's own streams wait too (J3: they go after the records the
/// owner's states let go). A record whose stream holds more than the step has left loses its
/// stream's oldest entries up to it now, and the record with the rest in a later step. One cursor
/// per (namespace, unprotected state) on the (`ns`, `state`, `seq`) index and one per stream on its
/// key: the work follows the rows removed.
fn default_reclaim(r: &mut Reclaim<'_>) -> Result<(), ServiceError> {
    /// The oldest record left in one (namespace, unprotected state).
    struct Cursor {
        ns: String,
        state: String,
        head: Option<(i64, String)>,
    }
    let c = r.conn();
    let mut cursors: Vec<Cursor> = Vec::new();
    let mut undeclared = false;
    for ns in r.namespaces()? {
        if !valid_ns(&ns) || ns == MARKER_NS {
            continue;
        }
        if !r.declared.contains_key(&ns) {
            undeclared = true;
            continue;
        }
        for state in states_of(c, &ns)? {
            if r.is_protected(&ns, &state) {
                continue;
            }
            let head = oldest_record(c, &ns, &state, i64::MIN)?;
            cursors.push(Cursor {
                ns: ns.clone(),
                state,
                head,
            });
        }
    }
    while !r.done() {
        let next = cursors
            .iter()
            .enumerate()
            .filter_map(|(i, cur)| cur.head.as_ref().map(|(seq, _)| (*seq, i)))
            .min();
        let Some((seq, i)) = next else { break };
        let cur = &mut cursors[i];
        if let Some((_, id)) = cur.head.take() {
            let stream = stream_of(&cur.ns, &id);
            let (n, bytes, _) = span(c, &stream, i64::MAX)?;
            let cost = bytes.saturating_add(n.saturating_mul(ROW_OVERHEAD));
            if n > 1 && cost > r.left() {
                // the step's WAL stays near the step: this much of the stream now, oldest first
                let through = cut(c, &stream, r.left())?;
                if r.trim(&stream, through)? > 0 {
                    cur.head = Some((seq, id));
                    continue;
                }
            } else {
                r.remove(&cur.ns, &id)?;
            }
        }
        cur.head = oldest_record(c, &cur.ns, &cur.state, seq)?;
    }
    let top = r.class.id();
    if r.done() || undeclared || !r.allows(top) {
        return Ok(());
    }
    let mut streams: Vec<(String, Option<i64>)> = Vec::new();
    for stream in top_streams(c, top)? {
        let head = oldest_entry(c, &stream, i64::MIN)?;
        streams.push((stream, head));
    }
    while !r.done() {
        let next = streams
            .iter()
            .enumerate()
            .filter_map(|(i, (_, head))| head.map(|seq| (seq, i)))
            .min();
        let Some((seq, i)) = next else { break };
        let (stream, head) = &mut streams[i];
        r.trim(stream, seq)?;
        *head = oldest_entry(c, stream, seq)?;
    }
    Ok(())
}

/// The J3 marker after a reclaim of `heal` that removed `rows` rows up to `through`: one record,
/// "pruned through seq N, K records" over every such reclaim so far (a damaged one starts again
/// from this reclaim's figures).
fn write_marker(w: &Writer<'_>, through: i64, rows: u64) -> Result<(), ServiceError> {
    let held = get_in(w.tx(), MARKER_NS, MARKER_ID)?.and_then(|r| Marker::from_body(&r.body));
    let marker = match held {
        Some(m) => Marker {
            through_seq: m.through_seq.max(through),
            records: m.records.saturating_add(rows),
        },
        None => Marker {
            through_seq: through,
            records: rows,
        },
    };
    put_raw(w, MARKER_NS, MARKER_ID, MARKER_STATE, &marker.body())?;
    Ok(())
}

/// `PRAGMA incremental_vacuum` stepped to its end (§3 J9): the statement frees one page per step,
/// so a single step (rusqlite's `execute_batch`) would free one page only. Nothing when the
/// freelist is empty.
fn vacuum(c: &Connection) -> Result<(), ServiceError> {
    if pragma_i64(c, "freelist_count")? > 0 {
        let mut st = c.prepare("PRAGMA incremental_vacuum")?;
        let mut rows = st.query([])?;
        while rows.next()?.is_some() {}
    }
    Ok(())
}

/// The database's size as its page count times its page size (what the file holds once
/// checkpointed).
fn file_size(c: &Connection) -> Result<u64, ServiceError> {
    let pages = u64::try_from(pragma_i64(c, "page_count")?).unwrap_or(0);
    let size = u64::try_from(pragma_i64(c, "page_size")?).unwrap_or(0);
    Ok(pages.saturating_mul(size))
}

/// An integer pragma; SQLCipher answers some as text.
fn pragma_i64(c: &Connection, name: &'static str) -> Result<i64, ServiceError> {
    let v: SqlValue = c.pragma_query_value(None, name, |r| r.get(0))?;
    match v {
        SqlValue::Integer(i) => Ok(i),
        SqlValue::Text(t) => t
            .trim()
            .parse()
            .map_err(|_| ServiceError::State(StateError::Corrupt(name))),
        _ => Err(ServiceError::State(StateError::Corrupt(name))),
    }
}

/// A decision record's fields (the service's own JSON object), the size checked before the parse.
fn decision_fields(body: &[u8]) -> Result<serde_json::Map<String, Value>, ServiceError> {
    let damaged = ServiceError::State(StateError::Corrupt("decision record"));
    if body.len() > MAX_BODY {
        return Err(damaged);
    }
    match serde_json::from_slice(body) {
        Ok(Value::Object(fields)) => Ok(fields),
        _ => Err(damaged),
    }
}

fn sum_of(totals: &Totals, class: Class) -> NsTotal {
    totals
        .iter()
        .filter(|(ns, _)| Class::of(ns) == Some(class))
        .fold(NsTotal::default(), |acc, (_, t)| NsTotal {
            rows: acc.rows.saturating_add(t.rows),
            bytes: acc.bytes.saturating_add(t.bytes),
        })
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The guard's rule for a record of `ns` in `state` (§3 J9, J3; §2.4 rule 5): the J3 marker and a
/// `pending` decision always; else the namespace's declaration; every state of a namespace with
/// none (fail closed: until its owner declares, a reclaim that runs first removes none of it).
fn protected_in(declared: &BTreeMap<String, Protected>, ns: &str, state: &str) -> bool {
    if ns == MARKER_NS || (ns == DECISIONS_NS && state == PENDING) {
        return true;
    }
    match declared.get(ns) {
        Some(Protected::States(states)) => states.iter().any(|s| s == state),
        Some(Protected::All) | None => true,
    }
}

// ---- names (§2.2 rule 5) ----

fn segment(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// A record namespace (§2.2 rule 5): `<top>/<name>[/<name>…]`, its top one of [`TOPS`], its names
/// `[a-z0-9_-]+`, at most [`MAX_NAME`] bytes.
pub fn valid_ns(ns: &str) -> bool {
    if ns.len() > MAX_NAME {
        return false;
    }
    let mut parts = ns.split('/');
    let top_ok = parts.next().is_some_and(|t| TOPS.contains(&t));
    let mut names = 0usize;
    for p in parts {
        if !segment(p) {
            return false;
        }
        names += 1;
    }
    top_ok && names > 0
}

/// An id: `[A-Za-z0-9_-]{1,128}` (the project's id rule).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_NAME
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A state: `[a-z0-9_-]{1,64}`.
pub fn valid_state(state: &str) -> bool {
    state.len() <= MAX_STATE && segment(state)
}

/// The namespace of `stream` (§2.2 rule 4 `entries.ns`): for `<ns>/<id>`, `ns`, a record
/// namespace (the stream of that record) or a top (a stream of its own, `heal/audit`); `None` when
/// `stream` is not a stream name.
pub fn stream_ns(stream: &str) -> Option<&str> {
    if stream.len() > MAX_STREAM {
        return None;
    }
    let (ns, id) = stream.rsplit_once('/')?;
    ((TOPS.contains(&ns) || valid_ns(ns)) && valid_id(id)).then_some(ns)
}

/// The stream of the record (`ns`, `id`): `<ns>/<id>`.
pub fn stream_of(ns: &str, id: &str) -> String {
    format!("{ns}/{id}")
}

/// The head record of `stream` (`ns`, `id`) when its namespace is a record namespace.
fn head_of(stream: &str) -> Option<(&str, &str)> {
    let (ns, id) = stream.rsplit_once('/')?;
    valid_ns(ns).then_some((ns, id))
}

/// Whether `ns` is `scope` or below it.
fn within(ns: &str, scope: &str) -> bool {
    ns == scope
        || (ns.len() > scope.len()
            && ns.starts_with(scope)
            && ns.as_bytes().get(scope.len()) == Some(&b'/'))
}

fn check_ns(ns: &str) -> Result<(), ServiceError> {
    if valid_ns(ns) {
        Ok(())
    } else {
        Err(ServiceError::Invalid("not a record namespace"))
    }
}

fn check_id(id: &str) -> Result<(), ServiceError> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(ServiceError::Invalid("not an id"))
    }
}

fn check_stream(stream: &str) -> Result<&str, ServiceError> {
    stream_ns(stream).ok_or(ServiceError::Invalid("not a stream name"))
}

fn check_limit(limit: usize) -> Result<(), ServiceError> {
    if limit == 0 {
        return Err(ServiceError::Invalid("a page holds at least one row"));
    }
    Ok(())
}

fn check_list(ns: &str, state: Option<&str>, limit: usize) -> Result<(), ServiceError> {
    check_ns(ns)?;
    if state.is_some_and(|s| !valid_state(s)) {
        return Err(ServiceError::Invalid("not a state"));
    }
    check_limit(limit)
}

fn check_body(body: &[u8]) -> Result<(), ServiceError> {
    if body.len() > MAX_BODY {
        return Err(ServiceError::Invalid("a body is at most 1 MiB"));
    }
    Ok(())
}

/// A namespace only the decision calls ([`DECISIONS_NS`]) or a reclaim ([`MARKER_NS`]) write.
fn check_writable(ns: &str) -> Result<(), ServiceError> {
    if ns == DECISIONS_NS {
        return Err(ServiceError::Invalid(
            "agent/decisions is written only by the decision calls",
        ));
    }
    if ns == MARKER_NS {
        return Err(ServiceError::Invalid(
            "heal/pruned is the reclaim's marker, written only by a reclaim",
        ));
    }
    Ok(())
}

fn check_record_delete(ns: &str, id: &str) -> Result<(), ServiceError> {
    check_ns(ns)?;
    check_id(id)?;
    check_writable(ns)
}

fn check_record_write(ns: &str, id: &str, state: &str, body: &[u8]) -> Result<(), ServiceError> {
    check_ns(ns)?;
    check_id(id)?;
    if !valid_state(state) {
        return Err(ServiceError::Invalid("not a state"));
    }
    check_body(body)?;
    check_writable(ns)
}

fn check_stream_write<'s>(stream: &'s str, body: &[u8]) -> Result<&'s str, ServiceError> {
    let ns = check_stream(stream)?;
    check_body(body)?;
    check_writable(ns)?;
    Ok(ns)
}

// ---- statements (each an index seek; `tests::every_statement_is_an_index_seek`) ----

const GET: &str = "SELECT ns, id, seq, boot_id, mono_ms, wall, state, body FROM records
     WHERE ns = ?1 AND id = ?2";
const BY_SEQ: &str = "SELECT ns, id, seq, boot_id, mono_ms, wall, state, body FROM records
     WHERE seq = ?1";
const EXISTS: &str = "SELECT 1 FROM records WHERE ns = ?1 AND id = ?2";
const STATE_OF: &str = "SELECT state FROM records WHERE ns = ?1 AND id = ?2";
const STATE_AND_BODY: &str = "SELECT state, body FROM records WHERE ns = ?1 AND id = ?2";
const HEAD: &str = "SELECT seq, state, length(body) FROM records WHERE ns = ?1 AND id = ?2";
const STATE_FIRST: &str = "SELECT state FROM records WHERE ns = ?1 ORDER BY state LIMIT 1";
const STATE_NEXT: &str =
    "SELECT state FROM records WHERE ns = ?1 AND state > ?2 ORDER BY state LIMIT 1";
const SEQS_IN_STATE: &str = "SELECT seq FROM records WHERE ns = ?1 AND state = ?2 AND seq > ?3
     ORDER BY seq LIMIT ?4";
const OLDEST_IN_STATE: &str = "SELECT seq, id FROM records WHERE ns = ?1 AND state = ?2
     AND seq > ?3 ORDER BY seq LIMIT 1";
const PUT: &str = "INSERT INTO records (ns, id, seq, boot_id, mono_ms, wall, state, body)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
     ON CONFLICT (ns, id) DO UPDATE SET seq = excluded.seq, boot_id = excluded.boot_id,
         mono_ms = excluded.mono_ms, wall = excluded.wall, state = excluded.state,
         body = excluded.body";
const SET_STATE: &str = "UPDATE records SET state = ?3, seq = ?4, boot_id = ?5, mono_ms = ?6,
     wall = ?7 WHERE ns = ?1 AND id = ?2";
const DELETE_RECORD: &str = "DELETE FROM records WHERE ns = ?1 AND id = ?2";
const APPEND: &str = "INSERT INTO entries (ns, stream, seq, body) VALUES (?1, ?2, ?3, ?4)";
const READ_STREAM: &str = "SELECT seq, body FROM entries WHERE stream = ?1 AND seq > ?2
     ORDER BY seq LIMIT ?3";
const SPAN: &str = "SELECT count(*), coalesce(sum(length(body)), 0), max(seq) FROM entries
     WHERE stream = ?1 AND seq <= ?2";
const TRIM: &str = "DELETE FROM entries WHERE stream = ?1 AND seq <= ?2";
const OLDEST_ENTRY: &str =
    "SELECT seq FROM entries WHERE stream = ?1 AND seq > ?2 ORDER BY seq LIMIT 1";
const ENTRY_LENGTHS: &str = "SELECT seq, length(body) FROM entries WHERE stream = ?1 AND seq > ?2
     ORDER BY seq LIMIT ?3";
const STREAM_FROM: &str =
    "SELECT stream FROM entries WHERE stream >= ?1 AND stream < ?2 ORDER BY stream LIMIT 1";
const STREAM_AFTER: &str =
    "SELECT stream FROM entries WHERE stream > ?1 AND stream < ?2 ORDER BY stream LIMIT 1";
const CLASS_NS: &str = "SELECT ns FROM ns_totals WHERE ns >= ?1 AND ns < ?2 ORDER BY ns";

/// Every read and delete above, for the plan test.
#[cfg(test)]
const SEEKS: [&str; 20] = [
    GET,
    BY_SEQ,
    EXISTS,
    STATE_OF,
    STATE_AND_BODY,
    HEAD,
    STATE_FIRST,
    STATE_NEXT,
    SEQS_IN_STATE,
    OLDEST_IN_STATE,
    SET_STATE,
    DELETE_RECORD,
    READ_STREAM,
    SPAN,
    TRIM,
    OLDEST_ENTRY,
    ENTRY_LENGTHS,
    STREAM_FROM,
    STREAM_AFTER,
    CLASS_NS,
];

fn record(r: &Row<'_>) -> rusqlite::Result<Record> {
    Ok(Record {
        ns: r.get(0)?,
        id: r.get(1)?,
        seq: r.get(2)?,
        boot_id: r.get(3)?,
        mono_ms: r.get(4)?,
        wall: r.get(5)?,
        state: r.get(6)?,
        body: r.get(7)?,
    })
}

fn exists(c: &Connection, ns: &str, id: &str) -> Result<bool, ServiceError> {
    Ok(c.prepare_cached(EXISTS)?
        .query_row(params![ns, id], |_| Ok(()))
        .optional()?
        .is_some())
}

fn get_in(c: &Connection, ns: &str, id: &str) -> Result<Option<Record>, ServiceError> {
    Ok(c.prepare_cached(GET)?
        .query_row(params![ns, id], record)
        .optional()?)
}

/// The record (`ns`, `id`)'s `seq`, state and body length, without its body.
fn head(c: &Connection, ns: &str, id: &str) -> Result<Option<(i64, String, i64)>, ServiceError> {
    Ok(c.prepare_cached(HEAD)?
        .query_row(params![ns, id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?)
}

/// The `seq` through which `stream`'s oldest entries come to `budget` bytes (`ns_totals`
/// measure), its oldest entry at least; its last entry's when the whole stream is less. Pages of
/// 256 lengths on its key: the work follows the entries it covers.
fn cut(c: &Connection, stream: &str, budget: u64) -> Result<i64, ServiceError> {
    const PAGE: i64 = 256;
    let mut st = c.prepare_cached(ENTRY_LENGTHS)?;
    let (mut after, mut sum) = (i64::MIN, 0u64);
    loop {
        let mut rows = st.query(params![stream, after, PAGE])?;
        let mut seen = 0i64;
        while let Some(row) = rows.next()? {
            let (seq, len): (i64, i64) = (row.get(0)?, row.get(1)?);
            sum = sum
                .saturating_add(u64::try_from(len).unwrap_or(0))
                .saturating_add(ROW_OVERHEAD);
            after = seq;
            seen += 1;
            if sum >= budget {
                return Ok(seq);
            }
        }
        if seen < PAGE {
            return Ok(after);
        }
    }
}

/// A new `seq`, this boot, the boot clock and the wall claim, and the record written (§2.2 rule
/// 4). The caller checked the names.
fn put_raw(
    w: &Writer<'_>,
    ns: &str,
    id: &str,
    state: &str,
    body: &[u8],
) -> Result<i64, ServiceError> {
    let s = w.stamp()?;
    w.tx().prepare_cached(PUT)?.execute(params![
        ns, id, s.seq, s.boot_id, s.mono_ms, s.wall, state, body
    ])?;
    Ok(s.seq)
}

/// `stream`'s entries through `through`: how many, their bodies' bytes, the highest `seq`.
fn span(
    c: &Connection,
    stream: &str,
    through: i64,
) -> Result<(u64, u64, Option<i64>), ServiceError> {
    let (n, bytes, top): (i64, i64, Option<i64>) = c
        .prepare_cached(SPAN)?
        .query_row(params![stream, through], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    Ok((
        u64::try_from(n).unwrap_or(0),
        u64::try_from(bytes).unwrap_or(0),
        top,
    ))
}

/// The states `ns`'s records hold, one index seek each (a loose scan of (`ns`, `state`)).
fn states_of(c: &Connection, ns: &str) -> Result<Vec<String>, ServiceError> {
    let mut first = c.prepare_cached(STATE_FIRST)?;
    let mut next = c.prepare_cached(STATE_NEXT)?;
    let mut out: Vec<String> = Vec::new();
    let mut at: Option<String> = first.query_row([ns], |r| r.get(0)).optional()?;
    while let Some(state) = at {
        at = next
            .query_row(params![ns, state], |r| r.get(0))
            .optional()?;
        out.push(state);
    }
    Ok(out)
}

fn oldest_record(
    c: &Connection,
    ns: &str,
    state: &str,
    after: i64,
) -> Result<Option<(i64, String)>, ServiceError> {
    Ok(c.prepare_cached(OLDEST_IN_STATE)?
        .query_row(params![ns, state, after], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?)
}

fn oldest_entry(c: &Connection, stream: &str, after: i64) -> Result<Option<i64>, ServiceError> {
    Ok(c.prepare_cached(OLDEST_ENTRY)?
        .query_row(params![stream, after], |r| r.get(0))
        .optional()?)
}

/// The class's own streams (`<top>/<id>`, `heal/audit`), one index seek each: a loose scan of
/// `entries`' (`stream`, `seq`) key that steps over each record namespace's streams in one seek.
fn top_streams(c: &Connection, top: &str) -> Result<Vec<String>, ServiceError> {
    let hi = format!("{top}0");
    let mut from = c.prepare_cached(STREAM_FROM)?;
    let mut after = c.prepare_cached(STREAM_AFTER)?;
    let mut out = Vec::new();
    let mut at: Option<String> = from
        .query_row(params![format!("{top}/"), hi], |r| r.get(0))
        .optional()?;
    while let Some(stream) = at {
        at = match stream.rsplit_once('/') {
            Some((ns, _)) if ns == top => {
                let next = after
                    .query_row(params![stream, hi], |r| r.get(0))
                    .optional()?;
                out.push(stream);
                next
            }
            // a record namespace's streams sort below `<ns>0`: one seek past them all
            Some((ns, _)) if within(ns, top) => from
                .query_row(params![format!("{ns}0"), hi], |r| r.get(0))
                .optional()?,
            _ => after
                .query_row(params![stream, hi], |r| r.get(0))
                .optional()?,
        };
    }
    Ok(out)
}

/// Collects rows into one page (§3 J9): at most `take`, and no more once the bodies would pass
/// [`MAX_PAGE_BYTES`] (one row always fits).
struct Pager<T> {
    items: Vec<T>,
    take: usize,
    bytes: usize,
    last: Option<i64>,
    next: Option<i64>,
}

impl<T> Pager<T> {
    fn new(limit: usize) -> Pager<T> {
        Pager {
            items: Vec::new(),
            take: limit.min(MAX_LIMIT),
            bytes: 0,
            last: None,
            next: None,
        }
    }

    /// Whether `item` was taken; `false` ends the page (another row follows).
    fn push(&mut self, seq: i64, len: usize, item: T) -> bool {
        let over = !self.items.is_empty() && self.bytes.saturating_add(len) > MAX_PAGE_BYTES;
        if self.items.len() >= self.take || over {
            self.next = self.last;
            return false;
        }
        self.bytes = self.bytes.saturating_add(len);
        self.last = Some(seq);
        self.items.push(item);
        true
    }

    fn finish(self) -> Page<T> {
        Page {
            items: self.items,
            next_after_seq: self.next,
        }
    }

    /// `take + 1`: one row past the page tells whether another follows.
    fn probe(&self) -> i64 {
        i64::try_from(self.take.saturating_add(1)).unwrap_or(i64::MAX)
    }
}

fn list_in(
    c: &Connection,
    ns: &str,
    state: Option<&str>,
    after: i64,
    limit: usize,
) -> Result<Page<Record>, ServiceError> {
    let mut pager = Pager::new(limit);
    let probe = pager.probe();
    let mut seqs: Vec<i64> = Vec::new();
    {
        let mut st = c.prepare_cached(SEQS_IN_STATE)?;
        let mut collect = |state: &str| -> Result<(), ServiceError> {
            let rows = st.query_map(params![ns, state, after, probe], |r| r.get::<_, i64>(0))?;
            for seq in rows {
                seqs.push(seq?);
            }
            Ok(())
        };
        match state {
            Some(s) => collect(s)?,
            // one index range per state, merged by `seq`: never a sort of the namespace
            None => {
                for s in states_of(c, ns)? {
                    collect(&s)?;
                }
            }
        }
    }
    seqs.sort_unstable();
    let mut by_seq = c.prepare_cached(BY_SEQ)?;
    for seq in seqs {
        let Some(rec) = by_seq.query_row([seq], record).optional()? else {
            continue;
        };
        if !pager.push(rec.seq, rec.body.len(), rec) {
            break;
        }
    }
    Ok(pager.finish())
}

fn read_stream_in(
    c: &Connection,
    stream: &str,
    after: i64,
    limit: usize,
) -> Result<Page<Entry>, ServiceError> {
    let mut pager = Pager::new(limit);
    let mut st = c.prepare_cached(READ_STREAM)?;
    let mut rows = st.query(params![stream, after, pager.probe()])?;
    while let Some(row) = rows.next()? {
        let seq: i64 = row.get(0)?;
        let body: Vec<u8> = row.get(1)?;
        let entry = Entry {
            stream: stream.to_string(),
            seq,
            body,
        };
        if !pager.push(seq, entry.body.len(), entry) {
            break;
        }
    }
    Ok(pager.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh111::keys::ModeS;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::AtomicUsize;
    use zeroize::Zeroizing;

    fn store(root: &Path) -> Arc<NodeState> {
        Arc::new(NodeState::new(
            root,
            &ModeS::new(Zeroizing::new([7u8; 32]), [0x5d; 32]),
        ))
    }

    fn service(root: &Path) -> StateService {
        StateService::of(store(root))
    }

    fn states(list: &[&str]) -> Protected {
        Protected::States(list.iter().map(|s| (*s).to_string()).collect())
    }

    /// Row D11's declarations as its module makes them at start (§4.4): a running task, an
    /// undecided proposal (a pending decision always), a conversation these tests keep `open`.
    fn declare_agent(s: &StateService) {
        s.declare("agent/conversations", states(&["open"])).unwrap();
        s.declare("agent/tasks", states(&["running"])).unwrap();
        s.declare(PROPOSALS_NS, states(&["undecided"])).unwrap();
        s.declare(DECISIONS_NS, states(&[])).unwrap();
    }

    /// A fake Layer 9 writer (G13 (7)): its records and its audit trail through J9 calls only.
    struct Layer9<'s>(&'s StateService);

    impl Layer9<'_> {
        /// Layer 9's §4.2 "never reclaimed" column, declared as its module does at start.
        fn declare(&self) {
            let s = self.0;
            s.declare("heal/incidents", states(&["open"])).unwrap();
            let attempts = [
                "armed",
                "awaiting_confirm",
                "revert_blocked",
                "revert_failed",
            ];
            s.declare("heal/attempts", states(&attempts)).unwrap();
            s.declare("heal/proposals", states(&["undecided"])).unwrap();
            s.declare("heal/fixes", states(&["undecided", "high_water"]))
                .unwrap();
            s.declare("heal/ledger", Protected::All).unwrap();
            s.declare("heal/snapshots", states(&[])).unwrap();
        }

        fn incident(&self, id: &str, state: &str) -> i64 {
            let body = format!(r#"{{"id":"{id}","cause":"(1, 111d, unclean_exit)"}}"#);
            self.0
                .put("heal/incidents", id, state, body.as_bytes())
                .unwrap()
        }

        fn attempt(&self, id: &str, state: &str) -> i64 {
            self.0.put("heal/attempts", id, state, b"{}").unwrap()
        }

        fn proposal(&self, id: &str, state: &str) -> i64 {
            self.0
                .put("heal/proposals", id, state, b"{\"actions\":[]}")
                .unwrap()
        }

        fn audit(&self, kind: &str) -> i64 {
            let body = format!(r#"{{"kind":"{kind}","by":"daemon"}}"#);
            self.0.append("heal/audit", body.as_bytes()).unwrap()
        }
    }

    /// What `ns_totals` counts for the record (`ns`, `id`).
    fn bytes_of(s: &StateService, ns: &str, id: &str) -> u64 {
        s.get(ns, id).unwrap().unwrap().body.len() as u64 + ROW_OVERHEAD
    }

    fn entry_bytes(s: &StateService, stream: &str, seq: i64) -> u64 {
        let page = s.read_stream(stream, Some(seq - 1), 1).unwrap();
        assert_eq!(page.items[0].seq, seq);
        page.items[0].body.len() as u64 + ROW_OVERHEAD
    }

    fn ids(s: &StateService, ns: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut after = None;
        loop {
            let page = s.list(ns, None, after, MAX_LIMIT).unwrap();
            out.extend(page.items.iter().map(|r| r.id.clone()));
            match page.next_after_seq {
                Some(n) => after = Some(n),
                None => return out,
            }
        }
    }

    fn seqs(s: &StateService, stream: &str) -> Vec<i64> {
        let page = s.read_stream(stream, None, MAX_LIMIT).unwrap();
        assert_eq!(page.next_after_seq, None);
        page.items.iter().map(|e| e.seq).collect()
    }

    fn marker(s: &StateService) -> Option<Marker> {
        s.get(MARKER_NS, MARKER_ID)
            .unwrap()
            .and_then(|r| Marker::from_body(&r.body))
    }

    fn decision(s: &StateService, id: &str) -> (String, Value) {
        let r = s.get(DECISIONS_NS, id).unwrap().unwrap();
        (r.state, serde_json::from_slice(&r.body).unwrap())
    }

    /// A reclaimer that removes nothing: the default never runs where it is registered.
    struct Nothing(AtomicUsize);

    impl Reclaimer for Nothing {
        fn reclaim(&self, _r: &mut Reclaim<'_>) -> Result<(), ServiceError> {
            self.0.fetch_add(1, SeqCst);
            Ok(())
        }
    }

    #[test]
    fn every_write_takes_a_new_seq_and_returns_it() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        let a = s.put("heal/incidents", "inc-1", "open", b"one").unwrap();
        let b = s.put("heal/incidents", "inc-1", "open", b"two").unwrap();
        assert!(b > a, "a put on an existing record takes a new seq");
        let rec = s.get("heal/incidents", "inc-1").unwrap().unwrap();
        assert_eq!(
            (rec.seq, rec.state.as_str(), rec.body.as_slice()),
            (b, "open", &b"two"[..])
        );
        assert!(rec.mono_ms > 0, "the boot clock beside the seq");
        let c = s
            .set_state("heal/incidents", "inc-1", "closed")
            .unwrap()
            .unwrap();
        assert!(c > b, "a set_state takes a new seq");
        let rec = s.get("heal/incidents", "inc-1").unwrap().unwrap();
        assert_eq!(
            (rec.seq, rec.state.as_str(), rec.body.as_slice()),
            (c, "closed", &b"two"[..])
        );
        let d = s.append("heal/audit", b"a").unwrap();
        assert!(d > c);
        let e = s
            .ingest("agent/proposals", "prop-1", "undecided", b"{}")
            .unwrap()
            .unwrap();
        assert!(e > d);
        // a call that writes nothing takes no seq
        assert_eq!(
            s.ingest("agent/proposals", "prop-1", "undecided", b"{}")
                .unwrap(),
            None
        );
        assert_eq!(
            s.set_state("heal/incidents", "inc-none", "closed").unwrap(),
            None
        );
        assert!(!s.delete("heal/incidents", "inc-none").unwrap());
        assert_eq!(s.trim_stream("heal/audit", 0).unwrap(), 0);
        assert_eq!(
            s.put("heal/incidents", "inc-2", "open", b"").unwrap(),
            e + 1
        );
        // and a delete takes none either
        assert!(s.delete("heal/incidents", "inc-2").unwrap());
        assert_eq!(
            s.put("heal/incidents", "inc-3", "open", b"").unwrap(),
            e + 2
        );
        // the order is the seq: a record written again goes after the newer one
        s.put("heal/incidents", "inc-2", "open", b"").unwrap();
        s.put("heal/incidents", "inc-3", "open", b"x").unwrap();
        assert_eq!(ids(&s, "heal/incidents"), ["inc-1", "inc-2", "inc-3"]);
        s.put("heal/incidents", "inc-1", "closed", b"two").unwrap();
        assert_eq!(ids(&s, "heal/incidents"), ["inc-2", "inc-3", "inc-1"]);
    }

    #[test]
    fn list_and_read_stream_return_at_most_limit_rows_after_the_seq_cursor() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        let states = ["open", "closed", "dismissed"];
        s.store()
            .write(Durability::Normal, |w| -> Result<(), ServiceError> {
                let ops = Ops::new(w);
                for i in 0..2500usize {
                    let id = format!("inc-{i}");
                    ops.put("heal/incidents", &id, states[i % 3], b"x")?;
                    ops.append("heal/audit", i.to_string().as_bytes())?;
                }
                Ok(())
            })
            .unwrap();
        // other namespaces' and streams' rows never show
        s.put("heal/attempts", "att-1", "open", b"").unwrap();
        s.put("agent/conversations", "c1", "active", b"").unwrap();
        s.append("agent/conversations/c1", b"m").unwrap();

        let first = s.list("heal/incidents", None, None, 5000).unwrap();
        assert_eq!(first.items.len(), MAX_LIMIT, "1,000 at most");
        assert!(first.items.windows(2).all(|w| w[0].seq < w[1].seq));
        assert_eq!(first.items[0].id, "inc-0");
        assert_eq!(first.next_after_seq, Some(first.items[999].seq));
        let mut seen = Vec::new();
        let mut after = None;
        loop {
            let page = s.list("heal/incidents", None, after, 700).unwrap();
            assert!(page.items.len() <= 700);
            assert!(page.items.iter().all(|r| after.is_none_or(|a| r.seq > a)));
            seen.extend(page.items.iter().map(|r| r.id.clone()));
            match page.next_after_seq {
                Some(n) => after = Some(n),
                None => break,
            }
        }
        let want: Vec<String> = (0..2500).map(|i| format!("inc-{i}")).collect();
        assert_eq!(seen, want, "every record once, in seq order");
        // in one state
        let closed = s
            .list("heal/incidents", Some("closed"), None, 1000)
            .unwrap();
        assert_eq!(closed.items.len(), 833);
        assert_eq!(closed.next_after_seq, None, "the last page has no cursor");
        assert!(closed.items.iter().all(|r| r.state == "closed"));
        assert!(closed.items.windows(2).all(|w| w[0].seq < w[1].seq));
        let two = s.list("heal/incidents", Some("open"), None, 2).unwrap();
        assert_eq!(two.items.len(), 2);
        assert_eq!(two.next_after_seq, Some(two.items[1].seq));
        // the cursor
        let mid = first.items[499].seq;
        let one = s.list("heal/incidents", None, Some(mid), 1).unwrap();
        assert_eq!(one.items[0].seq, first.items[500].seq);
        assert_eq!(one.next_after_seq, Some(one.items[0].seq));
        let past = s.list("heal/incidents", None, Some(i64::MAX), 10).unwrap();
        assert_eq!(
            past,
            Page {
                items: vec![],
                next_after_seq: None
            }
        );
        assert!(matches!(
            s.list("heal/incidents", None, None, 0),
            Err(ServiceError::Invalid(_))
        ));
        // a stream, the same way
        let e1 = s.read_stream("heal/audit", None, 5000).unwrap();
        assert_eq!(e1.items.len(), MAX_LIMIT);
        assert_eq!(e1.items[0].body, b"0");
        assert_eq!(e1.next_after_seq, Some(e1.items[999].seq));
        let mut bodies = Vec::new();
        let mut after = None;
        loop {
            let page = s.read_stream("heal/audit", after, 999).unwrap();
            bodies.extend(page.items.iter().map(|e| e.body.clone()));
            match page.next_after_seq {
                Some(n) => after = Some(n),
                None => break,
            }
        }
        let want: Vec<Vec<u8>> = (0..2500).map(|i: i32| i.to_string().into_bytes()).collect();
        assert_eq!(bodies, want);
        assert_eq!(
            s.read_stream("heal/audit", Some(i64::MAX), 10).unwrap(),
            Page {
                items: vec![],
                next_after_seq: None
            }
        );
        assert!(matches!(
            s.read_stream("heal/audit", None, 0),
            Err(ServiceError::Invalid(_))
        ));
    }

    #[test]
    fn a_page_stops_before_its_bodies_pass_four_mib() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        let big = vec![b'x'; MAX_BODY];
        for i in 0..6 {
            s.append("heal/audit", &big).unwrap();
            s.put("heal/snapshots", &format!("snap-{i}"), "kept", &big)
                .unwrap();
        }
        let p = s.read_stream("heal/audit", None, MAX_LIMIT).unwrap();
        assert_eq!(p.items.len(), 4);
        assert_eq!(p.next_after_seq, Some(p.items[3].seq));
        let rest = s.read_stream("heal/audit", p.next_after_seq, 1000).unwrap();
        assert_eq!((rest.items.len(), rest.next_after_seq), (2, None));
        let p = s.list("heal/snapshots", None, None, MAX_LIMIT).unwrap();
        assert_eq!(p.items.len(), 4);
        assert_eq!(p.next_after_seq, Some(p.items[3].seq));
        let rest = s
            .list("heal/snapshots", Some("kept"), p.next_after_seq, 1000)
            .unwrap();
        assert_eq!((rest.items.len(), rest.next_after_seq), (2, None));
        // a body over the cap is refused before any I/O
        let over = vec![0u8; MAX_BODY + 1];
        assert!(matches!(
            s.put("heal/snapshots", "snap-9", "kept", &over),
            Err(ServiceError::Invalid(_))
        ));
        assert!(matches!(
            s.append("heal/audit", &over),
            Err(ServiceError::Invalid(_))
        ));
    }

    #[test]
    fn trim_stream_and_delete_stream_remove_what_they_name() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        let audit: Vec<i64> = (0..5)
            .map(|i| s.append("heal/audit", format!("a{i}").as_bytes()).unwrap())
            .collect();
        s.put("agent/conversations", "c1", "active", b"{}").unwrap();
        let c1: Vec<i64> = (0..3)
            .map(|i| {
                s.append("agent/conversations/c1", format!("m{i}").as_bytes())
                    .unwrap()
            })
            .collect();
        s.put("agent/conversations", "c2", "active", b"{}").unwrap();
        s.append("agent/conversations/c2", b"z").unwrap();
        assert_eq!(s.trim_stream("heal/audit", audit[2]).unwrap(), 3);
        assert_eq!(seqs(&s, "heal/audit"), audit[3..]);
        assert_eq!(
            seqs(&s, "agent/conversations/c1"),
            c1,
            "another stream: untouched"
        );
        assert_eq!(s.trim_stream("heal/audit", audit[2]).unwrap(), 0);
        assert_eq!(s.delete_stream("agent/conversations/c1").unwrap(), 3);
        assert!(seqs(&s, "agent/conversations/c1").is_empty());
        assert!(
            s.get("agent/conversations", "c1").unwrap().is_some(),
            "delete_stream leaves the head"
        );
        assert_eq!(seqs(&s, "heal/audit"), audit[3..]);
        assert_eq!(seqs(&s, "agent/conversations/c2").len(), 1);
        // delete: the record with its stream, and nothing else
        assert!(s.delete("agent/conversations", "c2").unwrap());
        assert!(s.get("agent/conversations", "c2").unwrap().is_none());
        assert!(seqs(&s, "agent/conversations/c2").is_empty());
        assert!(!s.delete("agent/conversations", "c2").unwrap());
        // the totals in memory follow, as the store counts them
        let held = s.store().read(super::super::schema::read_totals).unwrap();
        let usage = s.usage().unwrap();
        assert_eq!(usage, held);
        assert_eq!(
            usage["heal"],
            NsTotal {
                rows: 2,
                bytes: 2 * (2 + ROW_OVERHEAD)
            }
        );
        assert_eq!(
            usage["agent/conversations"],
            NsTotal {
                rows: 1,
                bytes: 2 + ROW_OVERHEAD
            }
        );
    }

    #[test]
    fn a_full_class_refuses_put_append_and_set_state_and_still_takes_decisions_and_ingests() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        s.put("heal/incidents", "inc-1", "open", b"").unwrap();
        assert!(!s.set_full(Class::Heal, true));
        assert!(s.is_full(Class::Heal) && !s.is_full(Class::Agent));
        let before = s.usage().unwrap();
        assert!(matches!(
            s.put("heal/incidents", "inc-2", "open", b""),
            Err(ServiceError::Full(Class::Heal))
        ));
        assert!(matches!(
            s.append("heal/audit", b"x"),
            Err(ServiceError::Full(Class::Heal))
        ));
        assert!(matches!(
            s.set_state("heal/incidents", "inc-1", "closed"),
            Err(ServiceError::Full(Class::Heal))
        ));
        assert_eq!(s.usage().unwrap(), before, "nothing written");
        assert_eq!(
            s.get("heal/incidents", "inc-1").unwrap().unwrap().state,
            "open"
        );
        // the other class and storage/ grow
        s.put("agent/conversations", "c1", "active", b"").unwrap();
        s.put("storage/notes", "n1", "kept", b"").unwrap();
        // a delete makes room whatever the class's state
        assert!(s.delete("heal/incidents", "inc-1").unwrap());
        // with agent full: the decision calls and a spool ingest are written
        s.set_full(Class::Agent, true);
        assert!(matches!(
            s.put("agent/conversations", "c2", "active", b""),
            Err(ServiceError::Full(Class::Agent))
        ));
        assert!(matches!(
            s.append("agent/conversations/c1", b"m"),
            Err(ServiceError::Full(Class::Agent))
        ));
        assert!(s
            .ingest("agent/proposals", "prop-1", "undecided", b"{}")
            .unwrap()
            .is_some());
        let rec = json!({"schema": 1, "decision": "accepted"});
        s.decision_begin("prop-1", &rec).unwrap();
        assert_eq!(decision(&s, "prop-1").0, PENDING);
        s.decision_complete("prop-1", Some(&json!({"message_id": "msg-1"})))
            .unwrap();
        assert_eq!(decision(&s, "prop-1").0, DECIDED);
        s.decision_begin("prop-2", &rec).unwrap();
        assert!(s.decision_reopen("prop-2").unwrap());
        // and once the budget lets it grow again
        assert!(s.set_full(Class::Heal, false));
        s.put("heal/incidents", "inc-2", "open", b"").unwrap();
        assert!(s.set_full(Class::Agent, false));
        s.append("agent/conversations/c1", b"m").unwrap();
    }

    #[test]
    fn decisions_begin_pending_complete_once_and_reopen_only_while_pending() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        let rec = json!({"schema": 1, "decision": "accepted", "at": "2026-11-16T00:00:00Z"});
        let a = s.decision_begin("prop-1", &rec).unwrap();
        let held = s.get(DECISIONS_NS, "prop-1").unwrap().unwrap();
        assert_eq!((held.seq, held.state.as_str()), (a, PENDING));
        assert_eq!(
            serde_json::from_slice::<Value>(&held.body).unwrap(),
            json!({"schema": 1, "decision": "accepted", "at": "2026-11-16T00:00:00Z",
                   "applied": {"pending": true}})
        );
        // a retried accept: already_decided, the record unchanged
        assert!(matches!(
            s.decision_begin("prop-1", &rec),
            Err(ServiceError::AlreadyDecided)
        ));
        assert_eq!(s.get(DECISIONS_NS, "prop-1").unwrap().unwrap(), held);
        let b = s
            .decision_complete("prop-1", Some(&json!({"message_id": "msg-1"})))
            .unwrap();
        assert!(b > a);
        let done = s.get(DECISIONS_NS, "prop-1").unwrap().unwrap();
        assert_eq!((done.seq, done.state.as_str()), (b, DECIDED));
        assert_eq!(
            serde_json::from_slice::<Value>(&done.body).unwrap()["applied"],
            json!({"message_id": "msg-1"})
        );
        // decided: every decision call refused, the record unchanged
        assert!(matches!(
            s.decision_begin("prop-1", &rec),
            Err(ServiceError::AlreadyDecided)
        ));
        assert!(matches!(
            s.decision_complete("prop-1", None),
            Err(ServiceError::NotPending)
        ));
        assert!(matches!(
            s.decision_reopen("prop-1"),
            Err(ServiceError::NotPending)
        ));
        assert_eq!(s.get(DECISIONS_NS, "prop-1").unwrap().unwrap(), done);
        // a refused apply: reopened, the proposal open again, then decided from the start
        s.decision_begin("prop-2", &rec).unwrap();
        assert!(s.decision_reopen("prop-2").unwrap());
        assert!(s.get(DECISIONS_NS, "prop-2").unwrap().is_none());
        assert!(!s.decision_reopen("prop-2").unwrap());
        s.decision_begin("prop-2", &json!({"schema": 1, "decision": "dismissed"}))
            .unwrap();
        s.decision_complete("prop-2", None).unwrap();
        assert_eq!(
            decision(&s, "prop-2"),
            (
                DECIDED.to_string(),
                json!({"schema": 1, "decision": "dismissed"})
            ),
            "a decision that applied nothing keeps no applied"
        );
        assert!(matches!(
            s.decision_complete("prop-3", None),
            Err(ServiceError::NotFound)
        ));
        assert!(matches!(
            s.decision_begin("prop-3", &json!(["accepted"])),
            Err(ServiceError::Invalid(_))
        ));
        assert!(matches!(
            s.decision_begin("../prop", &rec),
            Err(ServiceError::Invalid(_))
        ));
    }

    #[test]
    fn only_the_decision_calls_write_agent_decisions_and_only_a_reclaim_the_marker() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        s.decision_begin("prop-1", &json!({"schema": 1, "decision": "accepted"}))
            .unwrap();
        let held = s.get(DECISIONS_NS, "prop-1").unwrap().unwrap();
        let refused = [
            s.put(DECISIONS_NS, "prop-1", "decided", b"{}").map(drop),
            s.put(DECISIONS_NS, "prop-9", "decided", b"{}").map(drop),
            s.set_state(DECISIONS_NS, "prop-1", "decided").map(drop),
            s.delete(DECISIONS_NS, "prop-1").map(drop),
            s.ingest(DECISIONS_NS, "prop-9", "decided", b"{}").map(drop),
            s.append("agent/decisions/prop-1", b"x").map(drop),
            s.delete_stream("agent/decisions/prop-1").map(drop),
            s.put(MARKER_NS, MARKER_ID, MARKER_STATE, b"{}").map(drop),
            s.delete(MARKER_NS, MARKER_ID).map(drop),
        ];
        for r in refused {
            assert!(matches!(r, Err(ServiceError::Invalid(_))), "{r:?}");
        }
        s.store()
            .write(Durability::Normal, |w| -> Result<(), ServiceError> {
                let ops = Ops::new(w);
                assert!(ops.put(DECISIONS_NS, "prop-1", "decided", b"{}").is_err());
                assert!(ops
                    .ingest(DECISIONS_NS, "prop-9", "decided", b"{}")
                    .is_err());
                assert!(ops.delete(DECISIONS_NS, "prop-1").is_err());
                assert!(ops.put(MARKER_NS, MARKER_ID, MARKER_STATE, b"{}").is_err());
                Ok(())
            })
            .unwrap();
        assert_eq!(s.get(DECISIONS_NS, "prop-1").unwrap().unwrap(), held);
        assert!(s.get(DECISIONS_NS, "prop-9").unwrap().is_none());
    }

    #[test]
    fn the_default_reclaim_of_heal_takes_the_closed_records_first_then_the_audit_from_its_oldest_seq(
    ) {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        let l9 = Layer9(&s);
        l9.declare();
        let a1 = l9.audit("proposed");
        l9.incident("inc-open", "open");
        l9.incident("inc-a", "closed");
        let a2 = l9.audit("accepted");
        l9.attempt("att-armed", "armed");
        l9.attempt("att-kept", "kept");
        l9.proposal("hp-1", "undecided");
        l9.incident("inc-b", "closed");
        let a3 = l9.audit("kept");
        let protected = [
            ("heal/incidents", "inc-open"),
            ("heal/attempts", "att-armed"),
            ("heal/proposals", "hp-1"),
        ];
        let inc_a = s.get("heal/incidents", "inc-a").unwrap().unwrap().seq;
        // the oldest row is an audit entry, but the closed records go first (J3)
        let r1 = s.reclaim(Class::Heal, 1).unwrap();
        assert_eq!(r1.rows, 1);
        assert_eq!(r1.through_seq, Some(inc_a));
        assert!(s.get("heal/incidents", "inc-a").unwrap().is_none());
        assert_eq!(seqs(&s, "heal/audit"), [a1, a2, a3], "the audit waits");
        assert_eq!(
            marker(&s),
            Some(Marker {
                through_seq: inc_a,
                records: 1
            })
        );
        // the rest of the closed records, then the audit's oldest entry
        let want =
            bytes_of(&s, "heal/attempts", "att-kept") + bytes_of(&s, "heal/incidents", "inc-b") + 1;
        let first_entry = entry_bytes(&s, "heal/audit", a1);
        let inc_b = s.get("heal/incidents", "inc-b").unwrap().unwrap().seq;
        let r2 = s.reclaim(Class::Heal, want).unwrap();
        assert_eq!(r2.rows, 3);
        assert_eq!(r2.bytes, want - 1 + first_entry);
        assert_eq!(r2.through_seq, Some(inc_b));
        assert!(s.get("heal/attempts", "att-kept").unwrap().is_none());
        assert!(s.get("heal/incidents", "inc-b").unwrap().is_none());
        assert_eq!(seqs(&s, "heal/audit"), [a2, a3], "from its oldest seq");
        // all it may take: the protected records stay, and one marker
        let r3 = s.reclaim(Class::Heal, u64::MAX).unwrap();
        assert_eq!(r3.rows, 2);
        assert!(seqs(&s, "heal/audit").is_empty());
        for (ns, id) in protected {
            assert!(s.get(ns, id).unwrap().is_some(), "{ns}/{id} is protected");
        }
        assert_eq!(
            marker(&s),
            Some(Marker {
                through_seq: a3.max(inc_b),
                records: 6
            })
        );
        assert_eq!(ids(&s, MARKER_NS), [MARKER_ID], "one marker");
        let left: Vec<String> = ["heal/incidents", "heal/attempts", "heal/proposals"]
            .iter()
            .flat_map(|ns| ids(&s, ns))
            .collect();
        assert_eq!(left, ["inc-open", "att-armed", "hp-1"]);
        // nothing more may go: nothing removed, the marker unchanged
        let m = s.get(MARKER_NS, MARKER_ID).unwrap().unwrap();
        assert_eq!(s.reclaim(Class::Heal, u64::MAX).unwrap().rows, 0);
        assert_eq!(s.get(MARKER_NS, MARKER_ID).unwrap().unwrap(), m);
        // the agent class leaves no marker in heal/ and none of its own
        declare_agent(&s);
        s.put("agent/tasks", "task-1", "done", b"{}").unwrap();
        assert_eq!(s.reclaim(Class::Agent, u64::MAX).unwrap().rows, 1);
        assert_eq!(s.get(MARKER_NS, MARKER_ID).unwrap().unwrap(), m);
    }

    #[test]
    fn the_default_takes_a_records_stream_with_it_and_leaves_a_protected_ones_whole() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        declare_agent(&s);
        s.put("agent/conversations", "c-old", "idle", b"{}")
            .unwrap();
        s.append("agent/conversations/c-old", b"hello").unwrap();
        s.put("agent/conversations", "c-open", "open", b"{}")
            .unwrap();
        s.append("agent/conversations/c-open", b"turn").unwrap();
        s.put("agent/tasks", "task-run", "running", b"{}").unwrap();
        let r = s.reclaim(Class::Agent, u64::MAX).unwrap();
        assert_eq!(r.rows, 2, "c-old's head and its one entry");
        assert!(seqs(&s, "agent/conversations/c-old").is_empty());
        assert_eq!(seqs(&s, "agent/conversations/c-open").len(), 1);
        assert!(s.get("agent/tasks", "task-run").unwrap().is_some());
        assert_eq!(
            s.usage_of(Class::Agent).unwrap().rows,
            3,
            "c-open's head and entry, the running task"
        );
    }

    /// Tries to take every row of its scope, the protected ones included, and a row outside it.
    struct Greedy {
        calls: AtomicUsize,
        refused: AtomicUsize,
        outside: AtomicBool,
    }

    impl Reclaimer for Greedy {
        fn reclaim(&self, r: &mut Reclaim<'_>) -> Result<(), ServiceError> {
            self.calls.fetch_add(1, SeqCst);
            let outside = matches!(
                r.remove("agent/conversations", "c1"),
                Err(ServiceError::Invalid(_))
            ) && matches!(
                r.trim("agent/conversations/c1", i64::MAX),
                Err(ServiceError::Invalid(_))
            );
            self.outside.store(outside, SeqCst);
            // a protected record's stream is left whole
            let kept = r.remove_stream("heal/incidents/inc-open")?;
            self.refused.fetch_add(usize::from(kept == 0), SeqCst);
            // its own order: the audit first, then every record oldest first
            r.remove_stream("heal/audit")?;
            for ns in r.namespaces()? {
                if !valid_ns(&ns) {
                    continue;
                }
                let mut after = None;
                loop {
                    let page = r.list(&ns, None, after, 100)?;
                    for rec in &page.items {
                        if !r.remove(&ns, &rec.id)? {
                            self.refused.fetch_add(1, SeqCst);
                        }
                    }
                    match page.next_after_seq {
                        Some(n) => after = Some(n),
                        None => break,
                    }
                }
            }
            Ok(())
        }
    }

    #[test]
    fn a_registered_reclaimer_runs_in_place_of_the_default_and_never_touches_a_protected_state() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        let l9 = Layer9(&s);
        l9.declare();
        l9.incident("inc-open", "open");
        l9.incident("inc-a", "closed");
        l9.attempt("att-armed", "armed");
        l9.attempt("att-kept", "kept");
        l9.proposal("hp-1", "undecided");
        l9.audit("proposed");
        l9.audit("accepted");
        s.append("heal/incidents/inc-open", b"evidence").unwrap();
        s.put("agent/conversations", "c1", "idle", b"{}").unwrap();
        s.append("agent/conversations/c1", b"m").unwrap();
        // a reclaimer that takes nothing: the default does not run in its place
        let idle = Arc::new(Nothing(AtomicUsize::new(0)));
        s.register("heal", idle.clone()).unwrap();
        let none = s.reclaim(Class::Heal, u64::MAX).unwrap();
        assert_eq!((none.rows, idle.0.load(SeqCst)), (0, 1));
        assert!(s.get("heal/incidents", "inc-a").unwrap().is_some());
        assert_eq!(marker(&s), None, "nothing pruned, no marker");
        // the greedy one, registered again for the same scope, in place of the first
        let greedy = Arc::new(Greedy {
            calls: AtomicUsize::new(0),
            refused: AtomicUsize::new(0),
            outside: AtomicBool::new(false),
        });
        s.register("heal", greedy.clone()).unwrap();
        let r = s.reclaim(Class::Heal, u64::MAX).unwrap();
        assert_eq!(greedy.calls.load(SeqCst), 1);
        assert_eq!(idle.0.load(SeqCst), 1);
        assert!(
            greedy.outside.load(SeqCst),
            "a row outside its scope is refused"
        );
        assert_eq!(
            greedy.refused.load(SeqCst),
            4,
            "the three protected records and inc-open's stream"
        );
        assert_eq!(seqs(&s, "heal/incidents/inc-open").len(), 1);
        assert_eq!(r.rows, 4, "the two audit entries, inc-a, att-kept");
        assert_eq!(ids(&s, "heal/incidents"), ["inc-open"]);
        assert_eq!(ids(&s, "heal/attempts"), ["att-armed"]);
        assert_eq!(ids(&s, "heal/proposals"), ["hp-1"]);
        assert!(seqs(&s, "heal/audit").is_empty());
        assert_eq!(marker(&s).map(|m| m.records), Some(4), "one marker");
        assert_eq!(
            seqs(&s, "agent/conversations/c1").len(),
            1,
            "agent untouched"
        );
        // a second reclaim: the marker is refused to it too, and stays one
        s.reclaim(Class::Heal, u64::MAX).unwrap();
        assert_eq!(greedy.refused.load(SeqCst), 4 + 5, "and the marker");
        assert_eq!(ids(&s, MARKER_NS), [MARKER_ID]);
    }

    #[test]
    fn the_default_covers_the_namespaces_no_registered_reclaimer_claims() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        declare_agent(&s);
        s.put("agent/conversations", "c1", "idle", b"{}").unwrap();
        s.append("agent/conversations/c1", b"m").unwrap();
        s.put("agent/tasks", "task-1", "done", b"{}").unwrap();
        let conv = Arc::new(Nothing(AtomicUsize::new(0)));
        s.register("agent/conversations", conv.clone()).unwrap();
        let r = s.reclaim(Class::Agent, u64::MAX).unwrap();
        assert_eq!((r.rows, conv.0.load(SeqCst)), (1, 1));
        assert!(s.get("agent/tasks", "task-1").unwrap().is_none());
        assert!(s.get("agent/conversations", "c1").unwrap().is_some());
        assert_eq!(seqs(&s, "agent/conversations/c1").len(), 1);
        // scopes: a class's top or one of its namespaces, never overlapping
        let other = || -> Arc<dyn Reclaimer> { Arc::new(Nothing(AtomicUsize::new(0))) };
        s.register("agent/tasks", other()).unwrap();
        for bad in [
            "agent",
            "agent/conversations/x",
            "storage",
            "storage/x",
            "heal/",
            "x",
        ] {
            assert!(
                matches!(s.register(bad, other()), Err(ServiceError::Invalid(_))),
                "{bad}"
            );
        }
        s.register("heal/incidents", other()).unwrap();
        assert!(matches!(
            s.register("heal", other()),
            Err(ServiceError::Invalid(_))
        ));
    }

    #[test]
    fn the_protected_states_are_each_namespaces_own_declaration() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        s.declare("heal/ledger", Protected::All).unwrap();
        s.declare(
            "heal/attempts",
            Protected::States(vec!["armed".into(), "awaiting_confirm".into()]),
        )
        .unwrap();
        s.declare(
            "heal/fixes",
            Protected::States(vec!["undecided".into(), "high_water".into()]),
        )
        .unwrap();
        // a declaration that forgets `pending`
        s.declare(DECISIONS_NS, Protected::States(vec![])).unwrap();
        assert!(s.is_protected("heal/ledger", "anything"));
        assert!(s.is_protected("heal/attempts", "awaiting_confirm"));
        assert!(
            !s.is_protected("heal/attempts", "open"),
            "its own declaration, exactly"
        );
        for state in ["open", "closed", "anything"] {
            assert!(
                s.is_protected("heal/incidents", state),
                "undeclared: every state (fail closed)"
            );
        }
        assert!(
            s.is_protected(DECISIONS_NS, PENDING),
            "whatever is declared"
        );
        assert!(s.is_protected(MARKER_NS, MARKER_STATE));
        s.put("heal/ledger", "l1", "counted", b"{}").unwrap();
        s.put("heal/attempts", "att-1", "awaiting_confirm", b"{}")
            .unwrap();
        s.put("heal/attempts", "att-2", "open", b"{}").unwrap();
        s.put("heal/fixes", "author-1", "high_water", b"{}")
            .unwrap();
        s.put("heal/fixes", "fix-1", "decided", b"{}").unwrap();
        s.put("heal/incidents", "inc-1", "closed", b"{}").unwrap();
        s.put("heal/incidents", "inc-2", "open", b"{}").unwrap();
        let rec = json!({"schema": 1, "decision": "accepted"});
        s.decision_begin("prop-pending", &rec).unwrap();
        s.decision_begin("prop-done", &rec).unwrap();
        s.decision_complete("prop-done", None).unwrap();
        assert_eq!(s.reclaim(Class::Heal, u64::MAX).unwrap().rows, 2);
        assert_eq!(ids(&s, "heal/ledger"), ["l1"]);
        assert_eq!(ids(&s, "heal/attempts"), ["att-1"]);
        assert_eq!(ids(&s, "heal/fixes"), ["author-1"]);
        assert_eq!(
            ids(&s, "heal/incidents"),
            ["inc-1", "inc-2"],
            "undeclared: nothing of it goes"
        );
        s.declare("heal/incidents", states(&["open"])).unwrap();
        assert_eq!(s.reclaim(Class::Heal, u64::MAX).unwrap().rows, 1);
        assert_eq!(ids(&s, "heal/incidents"), ["inc-2"]);
        // a decided decision whose proposal is gone may go alone; a pending one never
        assert_eq!(s.reclaim(Class::Agent, u64::MAX).unwrap().rows, 1);
        assert_eq!(
            ids(&s, DECISIONS_NS),
            ["prop-pending"],
            "a pending decision survives"
        );
        for bad in ["heal", "heal/", "Heal/x"] {
            assert!(matches!(
                s.declare(bad, Protected::All),
                Err(ServiceError::Invalid(_))
            ));
        }
        assert!(matches!(
            s.declare("heal/x", Protected::States(vec!["Not A State".into()])),
            Err(ServiceError::Invalid(_))
        ));
    }

    #[test]
    fn a_reclaim_vacuums_every_freed_page_and_the_file_shrinks_at_once() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        Layer9(&s).declare();
        let big = vec![0xa5u8; 64 * 1024];
        for i in 0..64 {
            s.put("heal/incidents", &format!("inc-{i:02}"), "closed", &big)
                .unwrap();
        }
        s.put("heal/incidents", "inc-open", "open", &big).unwrap();
        // closed by two idle passes: the file holds it all, no WAL
        s.store().on_pass();
        s.store().on_pass();
        assert!(!s.store().is_open());
        let before = s.store().file_bytes().unwrap();
        let row = big.len() as u64 + ROW_OVERHEAD;
        let out = s.reclaim(Class::Heal, 48 * row).unwrap();
        assert_eq!((out.rows, out.bytes), (48, 48 * row));
        assert!(out.file_bytes >= 48 * big.len() as u64, "{out:?}");
        let after = s.store().file_bytes().unwrap();
        assert_eq!(before - after, out.file_bytes, "on disk at once");
        let wal = super::super::companion(s.store().path(), "-wal");
        assert_eq!(fs::metadata(&wal).map(|m| m.len()).unwrap_or(0), 0);
        let free = s.store().read(|c| pragma_i64(c, "freelist_count")).unwrap();
        assert_eq!(free, 0, "every freed page vacuumed, not one per call");
        assert_eq!(ids(&s, "heal/incidents").len(), 17);
        // the store is a SQLCipher file with auto_vacuum INCREMENTAL (mesh111's open)
        let mode = s.store().read(|c| pragma_i64(c, "auto_vacuum")).unwrap();
        assert_eq!(mode, 2);
    }

    #[test]
    fn a_reclaim_step_is_one_transaction_and_a_failing_reclaimer_leaves_it_whole() {
        struct Failing;
        impl Reclaimer for Failing {
            fn reclaim(&self, r: &mut Reclaim<'_>) -> Result<(), ServiceError> {
                assert!(r.remove("heal/incidents", "inc-a")?);
                Err(ServiceError::Invalid("the reclaimer's own failure"))
            }
        }
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        Layer9(&s).declare();
        Layer9(&s).incident("inc-a", "closed");
        Layer9(&s).audit("kept");
        let before = s.usage().unwrap();
        s.register("heal", Arc::new(Failing)).unwrap();
        assert!(matches!(
            s.reclaim(Class::Heal, u64::MAX),
            Err(ServiceError::Invalid("the reclaimer's own failure"))
        ));
        assert!(s.get("heal/incidents", "inc-a").unwrap().is_some());
        assert_eq!(s.usage().unwrap(), before);
        assert_eq!(marker(&s), None);
    }

    #[test]
    fn a_reclaimer_that_calls_the_service_is_refused_not_left_waiting() {
        struct Reentrant {
            service: Mutex<Option<StateService>>,
            answer: Mutex<Option<String>>,
        }
        impl Reclaimer for Reentrant {
            fn reclaim(&self, _r: &mut Reclaim<'_>) -> Result<(), ServiceError> {
                let s = self.service.lock().unwrap().take();
                if let Some(s) = s {
                    let got = s.get("heal/incidents", "inc-a").map(|r| r.is_some());
                    *self.answer.lock().unwrap() = Some(format!("{got:?}"));
                }
                Ok(())
            }
        }
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        Layer9(&s).incident("inc-a", "closed");
        let r = Arc::new(Reentrant {
            service: Mutex::new(Some(s.clone())),
            answer: Mutex::new(None),
        });
        s.register("heal", r.clone()).unwrap();
        // on its own thread: a reclaimer left waiting on the store's lock fails the test, not
        // the suite
        let (done, finished) = std::sync::mpsc::channel();
        let svc = s.clone();
        std::thread::spawn(move || {
            let _ = done.send(svc.reclaim(Class::Heal, u64::MAX).map(|r| r.rows));
        });
        let rows = finished
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the reclaim returned")
            .unwrap();
        assert_eq!(rows, 0);
        let answer = r.answer.lock().unwrap().clone().unwrap();
        assert!(answer.starts_with("Err(Invalid("), "{answer}");
        // the mark is gone with the reclaim
        assert!(s.get("heal/incidents", "inc-a").unwrap().is_some());
    }

    /// Review finding 1 (§2.4 rule 5; §4.4): the record on disk is the guard, so a decision never
    /// goes while its proposal is listed, a proposal takes its decision with it, and one whose
    /// decision may not go stays.
    #[test]
    fn a_decision_goes_only_with_its_proposal() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        declare_agent(&s);
        let rec = json!({"schema": 1, "decision": "accepted"});
        // prop-1: decided, its proposal still `undecided` in the store: the oldest decision
        s.ingest(PROPOSALS_NS, "prop-1", "undecided", b"{}")
            .unwrap();
        s.decision_begin("prop-1", &rec).unwrap();
        s.decision_complete("prop-1", Some(&json!({"message_id": "msg-1"})))
            .unwrap();
        // prop-2: decided, its proposal too: the two go together
        s.ingest(PROPOSALS_NS, "prop-2", "undecided", b"{}")
            .unwrap();
        s.decision_begin("prop-2", &rec).unwrap();
        s.decision_complete("prop-2", None).unwrap();
        s.set_state(PROPOSALS_NS, "prop-2", "decided").unwrap();
        // prop-3: mid-apply, its proposal marked decided: neither goes
        s.ingest(PROPOSALS_NS, "prop-3", "undecided", b"{}")
            .unwrap();
        s.set_state(PROPOSALS_NS, "prop-3", "decided").unwrap();
        s.decision_begin("prop-3", &rec).unwrap();
        // prop-4: a decision whose proposal is gone: it guards nothing listed, it may go
        s.decision_begin("prop-4", &rec).unwrap();
        s.decision_complete("prop-4", None).unwrap();
        let r = s.reclaim(Class::Agent, u64::MAX).unwrap();
        assert_eq!(r.rows, 3, "prop-2 with its decision, prop-4's decision");
        assert!(s.get(DECISIONS_NS, "prop-1").unwrap().is_some());
        assert!(
            matches!(
                s.decision_begin("prop-1", &rec),
                Err(ServiceError::AlreadyDecided)
            ),
            "a retried accept of the listed proposal is still refused"
        );
        assert_eq!(ids(&s, PROPOSALS_NS), ["prop-1", "prop-3"]);
        assert_eq!(ids(&s, DECISIONS_NS), ["prop-1", "prop-3"]);
        // a registered reclaimer meets the same guard
        struct Pairs(AtomicUsize);
        impl Reclaimer for Pairs {
            fn reclaim(&self, r: &mut Reclaim<'_>) -> Result<(), ServiceError> {
                let lone = r.remove(DECISIONS_NS, "prop-1")?;
                let mid_apply = r.remove(PROPOSALS_NS, "prop-3")?;
                self.0.store(usize::from(!lone && !mid_apply), SeqCst);
                Ok(())
            }
        }
        let pairs = Arc::new(Pairs(AtomicUsize::new(0)));
        s.register("agent", pairs.clone()).unwrap();
        assert_eq!(s.reclaim(Class::Agent, u64::MAX).unwrap().rows, 0);
        assert_eq!(pairs.0.load(SeqCst), 1, "both refused");
        // with agent/decisions undeclared, a decided proposal stays with its decision
        let t2 = tempfile::tempdir().unwrap();
        let s2 = service(t2.path());
        s2.declare(PROPOSALS_NS, states(&["undecided"])).unwrap();
        s2.ingest(PROPOSALS_NS, "prop-5", "decided", b"{}").unwrap();
        s2.decision_begin("prop-5", &rec).unwrap();
        s2.decision_complete("prop-5", None).unwrap();
        assert_eq!(s2.reclaim(Class::Agent, u64::MAX).unwrap().rows, 0);
        assert_eq!(ids(&s2, PROPOSALS_NS), ["prop-5"]);
        assert_eq!(ids(&s2, DECISIONS_NS), ["prop-5"]);
    }

    /// Review finding 1 (§4.4: "never ... the conversation of a turn in flight"): a record marked
    /// in use is out of every reclaim's reach, its stream included, until its last mark goes.
    #[test]
    fn a_record_in_use_is_never_reclaimed() {
        /// Tries to take the conversation and its stream.
        struct Takes(AtomicUsize);
        impl Reclaimer for Takes {
            fn reclaim(&self, r: &mut Reclaim<'_>) -> Result<(), ServiceError> {
                let trimmed = r.trim("agent/conversations/c-turn", i64::MAX)?;
                let removed = r.remove("agent/conversations", "c-turn")?;
                if trimmed == 0 && !removed {
                    self.0.fetch_add(1, SeqCst);
                }
                Ok(())
            }
        }
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        declare_agent(&s);
        s.put("agent/conversations", "c-turn", "idle", b"{}")
            .unwrap();
        s.append("agent/conversations/c-turn", b"hello").unwrap();
        s.put("agent/tasks", "task-1", "done", b"{}").unwrap();
        // a turn on c-turn, and a second one on it
        let turn = s.in_use("agent/conversations", "c-turn").unwrap();
        let second = s.in_use("agent/conversations", "c-turn").unwrap();
        assert_eq!(
            s.reclaim(Class::Agent, u64::MAX).unwrap().rows,
            1,
            "the task only"
        );
        assert!(s.get("agent/conversations", "c-turn").unwrap().is_some());
        assert_eq!(seqs(&s, "agent/conversations/c-turn").len(), 1);
        let takes = Arc::new(Takes(AtomicUsize::new(0)));
        s.register("agent/conversations", takes.clone()).unwrap();
        assert_eq!(s.reclaim(Class::Agent, u64::MAX).unwrap().rows, 0);
        assert_eq!(takes.0.load(SeqCst), 1, "refused to a registered one too");
        drop(turn);
        assert_eq!(
            s.reclaim(Class::Agent, u64::MAX).unwrap().rows,
            0,
            "one mark is still held"
        );
        drop(second);
        assert_eq!(
            s.reclaim(Class::Agent, u64::MAX).unwrap().rows,
            2,
            "the head and its entry"
        );
        assert!(s.get("agent/conversations", "c-turn").unwrap().is_none());
        assert!(matches!(
            s.in_use("agent/x y", "c"),
            Err(ServiceError::Invalid(_))
        ));
    }

    /// Review finding 2 (§4.3 step 4, `disk_low`): a reclaim commits in steps of about
    /// [`RECLAIM_STEP`], each vacuumed and checkpointed, so the WAL it needs beside the file stays
    /// near one step however much it frees; a record's long stream is trimmed within the step.
    #[test]
    fn a_large_reclaim_commits_in_steps_and_its_wal_stays_near_one() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        Layer9(&s).declare();
        declare_agent(&s);
        let big = vec![0x5au8; 64 * 1024];
        let row = big.len() as u64 + ROW_OVERHEAD;
        s.store()
            .write(Durability::Normal, |w| -> Result<(), ServiceError> {
                let ops = Ops::new(w);
                for i in 0..160 {
                    ops.put("heal/incidents", &format!("inc-{i:03}"), "closed", &big)?;
                }
                ops.put("agent/conversations", "c-long", "idle", b"{}")?;
                for _ in 0..96 {
                    ops.append("agent/conversations/c-long", &big)?;
                }
                Ok(())
            })
            .unwrap();
        s.store().on_pass();
        s.store().on_pass();
        let wal = super::super::companion(s.store().path(), "-wal");
        let wal_now = || fs::metadata(&wal).map_or(0, |m| m.len());
        assert_eq!(wal_now(), 0);
        // 16 MiB reclaimed in all; the WAL beside the file stays under twice the 1 MiB step (one
        // transaction of the whole leaves it near the whole)
        let bound: u64 = 2 << 20;
        let before = s.store().file_bytes().unwrap();
        let heal = s.reclaim(Class::Heal, u64::MAX).unwrap();
        eprintln!("MEASURE heal reclaim of 160 x 64 KiB: {heal:?}");
        assert_eq!((heal.rows, heal.bytes), (160, 160 * row));
        assert!(heal.wal_bytes > 0, "{heal:?}");
        assert!(heal.wal_bytes <= bound, "{heal:?}");
        assert_eq!(wal_now(), 0, "every step checkpointed");
        let mid = s.store().file_bytes().unwrap();
        assert_eq!(before - mid, heal.file_bytes);
        let agent = s.reclaim(Class::Agent, u64::MAX).unwrap();
        eprintln!("MEASURE agent reclaim of one 96 x 64 KiB stream: {agent:?}");
        assert_eq!(agent.rows, 97, "the head and its 96 entries");
        assert!(agent.wal_bytes <= bound, "{agent:?}");
        assert!(s.get("agent/conversations", "c-long").unwrap().is_none());
        assert!(seqs(&s, "agent/conversations/c-long").is_empty());
        assert_eq!(wal_now(), 0);
        let free = s.store().read(|c| pragma_i64(c, "freelist_count")).unwrap();
        assert_eq!(free, 0);
        assert_eq!(mid - s.store().file_bytes().unwrap(), agent.file_bytes);
    }

    #[test]
    fn a_failing_step_ends_the_reclaim_and_keeps_the_steps_before_it() {
        /// Takes closed incidents up to its step, then fails at the next.
        struct FailsSecond(AtomicUsize);
        impl Reclaimer for FailsSecond {
            fn reclaim(&self, r: &mut Reclaim<'_>) -> Result<(), ServiceError> {
                if self.0.fetch_add(1, SeqCst) > 0 {
                    return Err(ServiceError::Invalid("the second step fails"));
                }
                let page = r.list("heal/incidents", Some("closed"), None, MAX_LIMIT)?;
                for rec in &page.items {
                    if r.done() {
                        break;
                    }
                    r.remove("heal/incidents", &rec.id)?;
                }
                Ok(())
            }
        }
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        Layer9(&s).declare();
        let big = vec![1u8; 64 * 1024];
        for i in 0..40 {
            s.put("heal/incidents", &format!("inc-{i:02}"), "closed", &big)
                .unwrap();
        }
        s.register("heal", Arc::new(FailsSecond(AtomicUsize::new(0))))
            .unwrap();
        assert!(matches!(
            s.reclaim(Class::Heal, u64::MAX),
            Err(ServiceError::Invalid("the second step fails"))
        ));
        let per_step = RECLAIM_STEP.div_ceil(big.len() as u64 + ROW_OVERHEAD);
        assert_eq!(
            ids(&s, "heal/incidents").len() as u64,
            40 - per_step,
            "the first step stays"
        );
        assert_eq!(marker(&s).map(|m| m.records), Some(per_step));
    }

    /// A reader that holds the WAL (row D18's second connection) blocks a step's checkpoint: the
    /// reclaim stops after that step rather than grow the WAL with the next, and goes on at the
    /// next call once the reader is gone.
    #[test]
    fn a_blocked_checkpoint_ends_the_reclaim_after_its_step() {
        use mesh111::keys::{KeyProvider, NODE_STATE_KDF_TAG};
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        Layer9(&s).declare();
        let big = vec![2u8; 64 * 1024];
        for i in 0..48 {
            s.put("heal/incidents", &format!("inc-{i:02}"), "closed", &big)
                .unwrap();
        }
        let key = ModeS::new(Zeroizing::new([7u8; 32]), [0x5d; 32])
            .key_for(NODE_STATE_KDF_TAG)
            .unwrap();
        // the store's connection waits rusqlite's 5 s for a blocked checkpoint: shorter here
        s.store()
            .read(|c| -> Result<(), ServiceError> {
                c.busy_timeout(std::time::Duration::from_millis(50))?;
                Ok(())
            })
            .unwrap();
        let reader = mesh111::store::open_encrypted(s.store().path(), &key).unwrap();
        reader.execute_batch("BEGIN").unwrap();
        let held: i64 = reader
            .query_row("SELECT count(*) FROM records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(held, 48);
        let per_step = RECLAIM_STEP.div_ceil(big.len() as u64 + ROW_OVERHEAD);
        let out = s.reclaim(Class::Heal, u64::MAX).unwrap();
        assert_eq!(out.rows, per_step, "one step, then the stop");
        let wal = super::super::companion(s.store().path(), "-wal");
        assert!(fs::metadata(&wal).map_or(0, |m| m.len()) > 0);
        reader.execute_batch("COMMIT").unwrap();
        drop(reader);
        let rest = s.reclaim(Class::Heal, u64::MAX).unwrap();
        assert_eq!(out.rows + rest.rows, 48);
        assert_eq!(fs::metadata(&wal).map_or(0, |m| m.len()), 0);
    }

    /// Review finding 3: a record's stream takes entries only while its record exists, so no
    /// stream is left that no reclaim reaches.
    #[test]
    fn an_append_to_a_records_stream_needs_its_record() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        assert!(matches!(
            s.append("agent/conversations/c1", b"m"),
            Err(ServiceError::NotFound)
        ));
        assert!(
            !t.path().join(super::super::DIR).exists(),
            "refused without creating the store"
        );
        s.put("heal/incidents", "inc-1", "closed", b"{}").unwrap();
        let before = s.usage().unwrap();
        for stream in ["heal/incidents/inc-2", "agent/conversations/c1"] {
            assert!(
                matches!(s.append(stream, b"evidence"), Err(ServiceError::NotFound)),
                "{stream}"
            );
        }
        assert_eq!(s.usage().unwrap(), before, "nothing written");
        // a stream of its own needs none; a record's stream once its record exists
        s.append("heal/audit", b"a").unwrap();
        s.append("heal/incidents/inc-1", b"evidence").unwrap();
        // one transaction: the head first, then its messages (a chat turn, §2.3 rule 3)
        s.store()
            .write(Durability::Normal, |w| -> Result<(), ServiceError> {
                let ops = Ops::new(w);
                assert!(matches!(
                    ops.append("agent/conversations/c1", b"m"),
                    Err(ServiceError::NotFound)
                ));
                ops.put("agent/conversations", "c1", "idle", b"{}")?;
                ops.append("agent/conversations/c1", b"m")?;
                Ok(())
            })
            .unwrap();
        assert_eq!(seqs(&s, "agent/conversations/c1").len(), 1);
        // its record deleted, the stream goes with it and takes nothing more
        assert!(s.delete("agent/conversations", "c1").unwrap());
        assert!(seqs(&s, "agent/conversations/c1").is_empty());
        assert!(matches!(
            s.append("agent/conversations/c1", b"m"),
            Err(ServiceError::NotFound)
        ));
        // so the class can give up everything its owner lets go: only the marker stays
        s.declare("heal/incidents", states(&[])).unwrap();
        assert_eq!(s.reclaim(Class::Heal, u64::MAX).unwrap().rows, 3);
        assert_eq!(
            s.usage_of(Class::Heal).unwrap().rows,
            1,
            "the J3 marker alone"
        );
    }

    /// Review finding 4 (§3 J9; Layer 9 §4.2): before its owner declares a namespace's states,
    /// no reclaim removes a record of it, and the class's own streams wait (J3: after the
    /// records), so a reclaim at a start, before Layer 9 or row D11 registered, removes nothing
    /// of theirs.
    #[test]
    fn an_undeclared_namespace_is_never_reclaimed_and_the_class_streams_wait() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        let l9 = Layer9(&s);
        // a start: the records the last run (or the J7 import) left, no module registered yet
        l9.audit("proposed");
        l9.incident("inc-a", "closed");
        l9.attempt("att-wait", "awaiting_confirm");
        l9.attempt("att-blocked", "revert_blocked");
        l9.attempt("att-failed", "revert_failed");
        l9.attempt("att-kept", "kept");
        s.put("heal/ledger", "led-1", "counted", b"{}").unwrap();
        s.put("heal/fixes", "author-1", "high_water", b"{}")
            .unwrap();
        s.put("agent/tasks", "task-1", "done", b"{}").unwrap();
        s.put("agent/conversations", "c1", "idle", b"{}").unwrap();
        let before = s.usage().unwrap();
        assert_eq!(s.reclaim(Class::Heal, u64::MAX).unwrap().rows, 0);
        assert_eq!(s.reclaim(Class::Agent, u64::MAX).unwrap().rows, 0);
        assert_eq!(s.usage().unwrap(), before, "nothing, the audit included");
        assert_eq!(marker(&s), None);
        // nor through a reclaimer registered before its module declared: the guard refuses it
        struct Early(AtomicUsize);
        impl Reclaimer for Early {
            fn reclaim(&self, r: &mut Reclaim<'_>) -> Result<(), ServiceError> {
                for (ns, id) in [("agent/tasks", "task-1"), ("agent/conversations", "c1")] {
                    if !r.remove(ns, id)? {
                        self.0.fetch_add(1, SeqCst);
                    }
                }
                Ok(())
            }
        }
        let early = Arc::new(Early(AtomicUsize::new(0)));
        s.register("agent", early.clone()).unwrap();
        assert_eq!(s.reclaim(Class::Agent, u64::MAX).unwrap().rows, 0);
        assert_eq!(early.0.load(SeqCst), 2, "both refused");
        assert_eq!(s.usage().unwrap(), before);
        // Layer 9's module declares (§4.2): the closed incident, the kept attempt, the audit
        l9.declare();
        assert_eq!(s.reclaim(Class::Heal, u64::MAX).unwrap().rows, 3);
        assert_eq!(ids(&s, "heal/incidents"), Vec::<String>::new());
        assert_eq!(
            ids(&s, "heal/attempts"),
            ["att-wait", "att-blocked", "att-failed"]
        );
        assert_eq!(ids(&s, "heal/ledger"), ["led-1"]);
        assert_eq!(ids(&s, "heal/fixes"), ["author-1"]);
        assert!(seqs(&s, "heal/audit").is_empty());
        // a namespace still undeclared that holds rows: its records stay and the audit waits
        s.put("heal/grants", "g-1", "kept", b"{}").unwrap();
        let a = l9.audit("kept");
        assert_eq!(s.reclaim(Class::Heal, u64::MAX).unwrap().rows, 0);
        assert_eq!(seqs(&s, "heal/audit"), [a]);
        assert_eq!(ids(&s, "heal/grants"), ["g-1"]);
    }

    /// Review finding 5 (§3 J9; §4.5): the `full` exemption is a spool ingest's only, so
    /// `ingest` takes a proposal or a task and nothing else.
    #[test]
    fn a_spool_ingest_takes_only_the_spools_namespaces() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        s.set_full(Class::Heal, true);
        s.set_full(Class::Agent, true);
        for ns in [
            "heal/incidents",
            "agent/conversations",
            "agent/proposals/x",
            "storage/notes",
        ] {
            assert!(
                matches!(
                    s.ingest(ns, "x-1", "open", b"{}"),
                    Err(ServiceError::Invalid(_))
                ),
                "{ns}"
            );
        }
        assert!(
            !t.path().join(super::super::DIR).exists(),
            "refused before any I/O"
        );
        // the spool's two, whatever the class's state
        assert!(s
            .ingest(PROPOSALS_NS, "prop-1", "undecided", b"{}")
            .unwrap()
            .is_some());
        assert!(s
            .ingest("agent/tasks", "task-1", "done", b"{}")
            .unwrap()
            .is_some());
        assert!(s.get("heal/incidents", "x-1").unwrap().is_none());
        // the legacy import's insert by id (§2.5, the J7 ring into heal/) runs in its own
        // transaction through Ops, which reads the flags itself
        let seq = s
            .store()
            .write(Durability::Full, |w| {
                Ops::new(w).ingest("heal/incidents", "inc-1", "closed", b"{}")
            })
            .unwrap();
        assert!(seq.is_some());
    }

    #[test]
    fn every_statement_is_an_index_seek() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(super::super::schema::V1).unwrap();
        for sql in SEEKS {
            let mut st = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let args: Vec<SqlValue> = (0..st.parameter_count())
                .map(|_| SqlValue::Text("x".into()))
                .collect();
            let plan: Vec<String> = st
                .query_map(rusqlite::params_from_iter(args), |r| r.get(3))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            assert!(!plan.is_empty(), "{sql}");
            for step in &plan {
                assert!(step.starts_with("SEARCH "), "{sql}: {step}");
                assert!(!step.contains("TEMP B-TREE"), "{sql}: {step}");
            }
        }
    }

    #[test]
    fn names_states_bodies_and_limits_are_checked_before_any_io() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        let long_id = "x".repeat(129);
        let long_state = "s".repeat(65);
        let long_ns = format!("heal/{}", "n".repeat(124));
        for (ns, id, state) in [
            ("heal", "x", "open"),
            ("heal/", "x", "open"),
            ("heal//x", "x", "open"),
            ("Heal/x", "x", "open"),
            ("other/x", "x", "open"),
            ("heal/x y", "x", "open"),
            (long_ns.as_str(), "x", "open"),
            ("heal/x", "a/b", "open"),
            ("heal/x", "", "open"),
            ("heal/x", "a.b", "open"),
            ("heal/x", long_id.as_str(), "open"),
            ("heal/x", "id", ""),
            ("heal/x", "id", "Open"),
            ("heal/x", "id", long_state.as_str()),
        ] {
            assert!(
                matches!(s.put(ns, id, state, b""), Err(ServiceError::Invalid(_))),
                "{ns} {id} {state}"
            );
        }
        let long_stream = format!("heal/{long_id}");
        for stream in [
            "heal",
            "heal/",
            "/x",
            "x/y",
            "heal/a/b c",
            "heal/incidents/a/",
            long_stream.as_str(),
        ] {
            assert!(
                matches!(s.append(stream, b""), Err(ServiceError::Invalid(_))),
                "{stream}"
            );
        }
        assert!(
            !t.path().join(super::super::DIR).exists(),
            "nothing created"
        );
        assert_eq!(stream_ns("heal/audit"), Some("heal"));
        assert_eq!(
            stream_ns("agent/conversations/conv-1"),
            Some("agent/conversations")
        );
        assert_eq!(
            stream_of("agent/conversations", "c1"),
            "agent/conversations/c1"
        );
        assert_eq!(
            head_of("heal/audit"),
            None,
            "a stream of its own has no head"
        );
        assert_eq!(
            head_of("agent/conversations/c1"),
            Some(("agent/conversations", "c1"))
        );
        assert!(valid_ns(&format!("heal/{}", "n".repeat(123))));
        assert!(within("heal/incidents", "heal") && within("heal", "heal"));
        assert!(!within("healx", "heal") && !within("heal", "heal/incidents"));
        assert_eq!(Class::of("heal"), Some(Class::Heal));
        assert_eq!(Class::of("agent/tasks"), Some(Class::Agent));
        assert_eq!(Class::of("storage/x"), None);
    }

    #[test]
    fn with_the_store_unavailable_every_call_answers_its_reason() {
        let t = tempfile::tempdir().unwrap();
        let s = StateService::of(Arc::new(NodeState::without_key(
            t.path(),
            "no identity is loaded",
        )));
        fn unavailable<T: fmt::Debug>(r: Result<T, ServiceError>) {
            assert!(
                matches!(r, Err(ServiceError::State(StateError::Unavailable(_)))),
                "{r:?}"
            );
        }
        let rec = json!({"schema": 1, "decision": "accepted"});
        unavailable(s.put("heal/incidents", "i", "open", b""));
        unavailable(s.get("heal/incidents", "i"));
        unavailable(s.list("heal/incidents", None, None, 10));
        unavailable(s.append("heal/audit", b""));
        unavailable(s.append("heal/incidents/i", b""));
        unavailable(s.read_stream("heal/audit", None, 10));
        unavailable(s.set_state("heal/incidents", "i", "closed"));
        unavailable(s.delete("heal/incidents", "i"));
        unavailable(s.delete_stream("heal/audit"));
        unavailable(s.trim_stream("heal/audit", 1));
        unavailable(s.usage());
        unavailable(s.reclaim(Class::Heal, 1));
        unavailable(s.ingest("agent/proposals", "p", "undecided", b""));
        unavailable(s.decision_begin("p", &rec));
        unavailable(s.decision_complete("p", None));
        unavailable(s.decision_reopen("p"));
        assert!(!t.path().join(super::super::DIR).exists());
    }

    #[test]
    fn calls_that_only_remove_or_read_create_no_store() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        assert!(!s.delete("heal/incidents", "i").unwrap());
        assert_eq!(s.delete_stream("heal/audit").unwrap(), 0);
        assert!(matches!(
            s.append("heal/incidents/i", b"evidence"),
            Err(ServiceError::NotFound)
        ));
        assert_eq!(s.set_state("heal/incidents", "i", "closed").unwrap(), None);
        assert!(!s.decision_reopen("p").unwrap());
        assert!(matches!(
            s.decision_complete("p", None),
            Err(ServiceError::NotFound)
        ));
        assert_eq!(
            s.reclaim(Class::Heal, u64::MAX).unwrap(),
            Reclaimed::default()
        );
        assert_eq!(s.get("heal/incidents", "i").unwrap(), None);
        assert!(s
            .list("heal/incidents", None, None, 1)
            .unwrap()
            .items
            .is_empty());
        assert!(s.usage().unwrap().is_empty());
        assert!(
            !t.path().join(super::super::DIR).exists(),
            "no node/ written"
        );
    }

    #[test]
    fn usage_is_the_in_memory_totals_by_namespace_and_by_class() {
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        s.put("heal/incidents", "i1", "open", &[1u8; 10]).unwrap();
        s.append("heal/audit", &[2u8; 20]).unwrap();
        s.put("agent/tasks", "t1", "done", &[3u8; 30]).unwrap();
        s.put("storage/notes", "n1", "kept", &[4u8; 40]).unwrap();
        let usage = s.usage().unwrap();
        assert_eq!(
            Some(usage.clone()),
            s.store().totals(),
            "the totals in memory"
        );
        assert_eq!(
            s.usage_of(Class::Heal).unwrap(),
            NsTotal {
                rows: 2,
                bytes: 30 + 2 * ROW_OVERHEAD
            }
        );
        assert_eq!(
            s.usage_of(Class::Agent).unwrap(),
            NsTotal {
                rows: 1,
                bytes: 30 + ROW_OVERHEAD
            }
        );
        assert_eq!(usage["storage/notes"].bytes, 40 + ROW_OVERHEAD);
        // a new start reads them once, then from memory
        drop(s);
        let s = StateService::of(store(t.path()));
        assert_eq!(s.store().totals(), None);
        assert_eq!(s.usage().unwrap(), usage);
        assert_eq!(s.store().totals(), Some(usage.clone()));
        s.store().on_pass();
        s.store().on_pass();
        assert!(!s.store().is_open());
        assert_eq!(s.usage().unwrap(), usage);
        assert!(
            !s.store().is_open(),
            "known totals are read from memory: no open"
        );
    }

    #[test]
    fn of_keeps_one_registry_and_one_set_of_flags_per_store() {
        let t = tempfile::tempdir().unwrap();
        let st = store(t.path());
        let a = StateService::of(Arc::clone(&st));
        let b = StateService::of(Arc::clone(&st));
        a.set_full(Class::Heal, true);
        assert!(b.is_full(Class::Heal));
        a.declare("heal/snapshots", states(&[])).unwrap();
        assert!(!b.is_protected("heal/snapshots", "kept"));
        let marks = a.in_use("agent/conversations", "c1").unwrap();
        assert!(held_in_use(&b.inner.in_use, "agent/conversations", "c1"));
        let t2 = tempfile::tempdir().unwrap();
        let other = StateService::of(store(t2.path()));
        assert!(!other.is_full(Class::Heal));
        assert!(
            other.is_protected("heal/snapshots", "kept"),
            "undeclared there"
        );
        assert!(!held_in_use(
            &other.inner.in_use,
            "agent/conversations",
            "c1"
        ));
        drop(marks);
        assert!(!held_in_use(&b.inner.in_use, "agent/conversations", "c1"));
        // a store that is gone takes its registry with it
        drop((a, b, st));
        let again = StateService::of(store(t.path()));
        assert!(!again.is_full(Class::Heal));
    }

    /// The figure J9's `limit` rests on (§3 J9: 1,000 rows; default, measured at build): the time
    /// of one full page of `list` and of `read_stream`, and of a reclaim of 1,000 records, over a
    /// store of 20,000 records and 20,000 entries. `cargo test -p node111 --lib
    /// state::service::tests::measure -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, run by hand"]
    fn measure_a_1000_row_page_and_a_1000_record_reclaim() {
        use std::time::Instant;
        let t = tempfile::tempdir().unwrap();
        let s = service(t.path());
        Layer9(&s).declare();
        let body = vec![b'b'; 200];
        s.store()
            .write(Durability::Normal, |w| -> Result<(), ServiceError> {
                let ops = Ops::new(w);
                for i in 0..20_000usize {
                    let state = if i % 4 == 0 { "open" } else { "closed" };
                    ops.put("heal/incidents", &format!("inc-{i}"), state, &body)?;
                    ops.append("heal/audit", &body)?;
                }
                Ok(())
            })
            .unwrap();
        s.store().on_pass();
        s.store().on_pass();
        let at = Instant::now();
        let p = s.list("heal/incidents", None, None, MAX_LIMIT).unwrap();
        let list = at.elapsed();
        let at = Instant::now();
        let p2 = s
            .list("heal/incidents", Some("closed"), Some(30_000), MAX_LIMIT)
            .unwrap();
        let list_state = at.elapsed();
        let at = Instant::now();
        let e = s
            .read_stream("heal/audit", Some(20_000), MAX_LIMIT)
            .unwrap();
        let read = at.elapsed();
        let at = Instant::now();
        let r = s.reclaim(Class::Heal, 1000 * (200 + ROW_OVERHEAD)).unwrap();
        let reclaim = at.elapsed();
        assert_eq!(
            (p.items.len(), p2.items.len(), e.items.len(), r.rows),
            (1000, 1000, 1000, 1000)
        );
        eprintln!(
            "MEASURE list 1000 (cold open): {list:?} · list 1000 in a state: {list_state:?} · \
             read_stream 1000: {read:?} · reclaim 1000 records: {reclaim:?} ({} B from the file)",
            r.file_bytes
        );
    }
}
