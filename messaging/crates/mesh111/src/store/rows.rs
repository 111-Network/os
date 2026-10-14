//! §10.3 rows — typed access to `messages`, `contacts`, `outbox`, `custody`,
//! `cursors` and `requests`, at plain integers/bytes (the envelope types of
//! §3 live elsewhere; the store never parses a body or a sealed packet).
//!
//! Size limits are checked before anything is written.

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::tx::Seq;
use super::{
    current_local_seq, id32, next_local_seq, to_i64, to_u64, Durability, Result, Store, StoreError,
};

/// Largest message body kept in the DB (Tier 0/1 bodies are far smaller;
/// larger objects are blobs, §10.3).
pub const MAX_BODY: usize = 1 << 20;
/// Largest custody item: one sealed packet in one `tcp-lan` frame (§8.1).
pub const MAX_CUSTODY_ITEM: usize = 65_535;
/// Largest stored announce (L2 §6.2: 137–169 B; slack for growth).
pub const MAX_ANNOUNCE: usize = 512;
/// Largest sealed fragment kept in the outbox (one `tcp-lan` frame).
pub const MAX_SEALED: usize = 65_535;
/// Largest alias.
pub const MAX_ALIAS: usize = 256;
/// Tier 1 fragment bound (§5).
pub const MAX_FRAGS: u8 = 16;

/// Storage/pruning class (§10.4 quotas; §3.2 kinds map onto it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Class {
    /// Control kinds (ACK, custody, mailbox, enroll).
    Control = 0,
    /// `MSG` — human conversation.
    Human = 1,
    /// `AGENT_OP`.
    Agent = 2,
    /// `KB` objects / deltas (bulk).
    Kb = 3,
    /// `UPDATE` payloads (bulk).
    Update = 4,
    /// `PATCH` objects (bulk).
    Patch = 5,
    /// `SYNC` ops (bulk).
    Sync = 6,
    /// Other objects: attachments, public chunks (bulk).
    Object = 7,
    /// Transit custody held for members (hub role, §9).
    Custody = 8,
}

impl Class {
    /// Every class.
    pub const ALL: [Class; 9] = [
        Class::Control,
        Class::Human,
        Class::Agent,
        Class::Kb,
        Class::Update,
        Class::Patch,
        Class::Sync,
        Class::Object,
        Class::Custody,
    ];

    /// The class of a §3.2 kind byte.
    pub fn from_kind(kind: u8) -> Class {
        match kind {
            0x01 => Class::Human,
            0x10 => Class::Agent,
            0x11 => Class::Kb,
            0x12 => Class::Update,
            0x13 => Class::Patch,
            0x14 => Class::Sync,
            0x20..=0x22 => Class::Object,
            _ => Class::Control,
        }
    }

    /// Bulk and custody classes: the ones capacity may refuse and quotas
    /// shape (§10.4: "refuses new custody and new bulk").
    pub fn is_refusable(self) -> bool {
        matches!(
            self,
            Class::Kb | Class::Update | Class::Patch | Class::Sync | Class::Object | Class::Custody
        )
    }

    /// Stable lower-case name (settings keys, logs).
    pub fn name(self) -> &'static str {
        match self {
            Class::Control => "control",
            Class::Human => "human",
            Class::Agent => "agent",
            Class::Kb => "kb",
            Class::Update => "update",
            Class::Patch => "patch",
            Class::Sync => "sync",
            Class::Object => "object",
            Class::Custody => "custody",
        }
    }

    pub(crate) fn from_i64(v: i64) -> Result<Class> {
        Class::ALL
            .into_iter()
            .find(|c| *c as i64 == v)
            .ok_or(StoreError::Corrupt("class"))
    }
}

/// `messages.dir` (§10.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Dir {
    Own = 0,
    In = 1,
    Transit = 2,
}

impl Dir {
    fn from_i64(v: i64) -> Result<Dir> {
        match v {
            0 => Ok(Dir::Own),
            1 => Ok(Dir::In),
            2 => Ok(Dir::Transit),
            _ => Err(StoreError::Corrupt("dir")),
        }
    }
}

/// §9 UI states (normative evidence). Ordered: a state never moves back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum MsgState {
    Queued = 0,
    Sent = 1,
    Relayed = 2,
    Delivered = 3,
}

impl MsgState {
    fn from_i64(v: i64) -> Result<MsgState> {
        match v {
            0 => Ok(MsgState::Queued),
            1 => Ok(MsgState::Sent),
            2 => Ok(MsgState::Relayed),
            3 => Ok(MsgState::Delivered),
            _ => Err(StoreError::Corrupt("state")),
        }
    }
}

/// §7.1 trust. Stored values are fixed by the schema; the **order** is
/// [`Trust::rank`]: `tofu` < `oob` < `enrolled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Trust {
    /// Card / QR (out of band).
    Oob = 1,
    /// INTRO (trust on first use).
    Tofu = 2,
    /// Hub ↔ member enrollment.
    Enrolled = 3,
}

impl Trust {
    /// Strength order (`tofu` 0 < `oob` 1 < `enrolled` 2): a contact's
    /// trust only ever moves up this order.
    pub fn rank(self) -> u8 {
        match self {
            Trust::Tofu => 0,
            Trust::Oob => 1,
            Trust::Enrolled => 2,
        }
    }

    /// The stronger of two trusts.
    pub fn max(self, other: Trust) -> Trust {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }

    fn from_i64(v: i64) -> Result<Trust> {
        match v {
            1 => Ok(Trust::Oob),
            2 => Ok(Trust::Tofu),
            3 => Ok(Trust::Enrolled),
            _ => Err(StoreError::Corrupt("trust")),
        }
    }
}

/// A message to store (own: before sealing; inbound: after `pair_mac` and
/// dedup). `peer` is the other party; `sender` is this node for own
/// messages and `peer` for inbound ones.
#[derive(Debug, Clone, Copy)]
pub struct NewMessage<'a> {
    pub peer: [u8; 32],
    pub sender: [u8; 32],
    pub stream: u64,
    pub kind: u8,
    pub class: Class,
    pub body: &'a [u8],
    pub blob_refs: Option<&'a [u8]>,
    pub reason: Option<&'a str>,
    pub pinned: bool,
}

/// A stored message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRow {
    pub id: i64,
    pub peer: [u8; 32],
    pub sender: [u8; 32],
    pub stream: u64,
    pub seq: Seq,
    pub dir: Dir,
    pub kind: u8,
    pub class: Class,
    pub size: u64,
    pub local_seq: u64,
    pub state: MsgState,
    pub reason: Option<String>,
    pub received_local_seq: Option<u64>,
    pub body: Option<Vec<u8>>,
    pub blob_refs: Option<Vec<u8>>,
    pub pinned: bool,
}

const MSG_COLS: &str = "id, peer, sender, stream, epoch, n, dir, kind, class, size, local_seq, \
                        state, reason, received_local_seq, body, blob_refs, pinned";

fn message_from_row(r: &Row<'_>) -> rusqlite::Result<MessageRowRaw> {
    Ok(MessageRowRaw {
        id: r.get(0)?,
        peer: r.get(1)?,
        sender: r.get(2)?,
        stream: r.get(3)?,
        epoch: r.get(4)?,
        n: r.get(5)?,
        dir: r.get(6)?,
        kind: r.get(7)?,
        class: r.get(8)?,
        size: r.get(9)?,
        local_seq: r.get(10)?,
        state: r.get(11)?,
        reason: r.get(12)?,
        received_local_seq: r.get(13)?,
        body: r.get(14)?,
        blob_refs: r.get(15)?,
        pinned: r.get(16)?,
    })
}

struct MessageRowRaw {
    id: i64,
    peer: Vec<u8>,
    sender: Vec<u8>,
    stream: i64,
    epoch: i64,
    n: i64,
    dir: i64,
    kind: i64,
    class: i64,
    size: i64,
    local_seq: i64,
    state: i64,
    reason: Option<String>,
    received_local_seq: Option<i64>,
    body: Option<Vec<u8>>,
    blob_refs: Option<Vec<u8>>,
    pinned: bool,
}

impl MessageRowRaw {
    fn into_row(self) -> Result<MessageRow> {
        Ok(MessageRow {
            id: self.id,
            peer: id32(self.peer, "messages.peer")?,
            sender: id32(self.sender, "messages.sender")?,
            stream: to_u64(self.stream, "messages.stream")?,
            seq: Seq {
                epoch: to_u64(self.epoch, "messages.epoch")?,
                n: to_u64(self.n, "messages.n")?,
            },
            dir: Dir::from_i64(self.dir)?,
            kind: u8::try_from(self.kind).map_err(|_| StoreError::Corrupt("messages.kind"))?,
            class: Class::from_i64(self.class)?,
            size: to_u64(self.size, "messages.size")?,
            local_seq: to_u64(self.local_seq, "messages.local_seq")?,
            state: MsgState::from_i64(self.state)?,
            reason: self.reason,
            received_local_seq: self
                .received_local_seq
                .map(|v| to_u64(v, "messages.received_local_seq"))
                .transpose()?,
            body: self.body,
            blob_refs: self.blob_refs,
            pinned: self.pinned,
        })
    }
}

/// Insert a message row inside the caller's transaction.
pub(crate) fn insert_message(
    c: &Connection,
    m: &NewMessage<'_>,
    seq: Seq,
    dir: Dir,
    state: MsgState,
) -> Result<i64> {
    if m.body.len() > MAX_BODY {
        return Err(StoreError::Invalid("message body too large"));
    }
    if m.blob_refs.is_some_and(|b| b.len() > MAX_BODY) {
        return Err(StoreError::Invalid("blob refs too large"));
    }
    if m.reason.is_some_and(|r| r.len() > MAX_ALIAS) {
        return Err(StoreError::Invalid("reason too long"));
    }
    let local = next_local_seq(c)?;
    let received = (dir == Dir::In).then_some(to_i64(local, "local_seq")?);
    c.execute(
        "INSERT INTO messages (peer, sender, stream, epoch, n, dir, kind, class, size, local_seq,
                               state, reason, received_local_seq, body, blob_refs, pinned)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            &m.peer[..],
            &m.sender[..],
            to_i64(m.stream, "stream")?,
            to_i64(seq.epoch, "epoch")?,
            to_i64(seq.n, "n")?,
            dir as i64,
            i64::from(m.kind),
            m.class as i64,
            m.body.len() as i64,
            to_i64(local, "local_seq")?,
            state as i64,
            m.reason,
            received,
            m.body,
            m.blob_refs,
            m.pinned,
        ],
    )?;
    Ok(c.last_insert_rowid())
}

/// A §7.1 contact row. On upsert the store assigns `first_seen_seq` /
/// `last_seen_seq` (local sequence); the values passed in are ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactRow {
    pub node_id: [u8; 32],
    pub enc_pubkey: [u8; 32],
    pub announce: Vec<u8>,
    pub announce_counter: u64,
    pub trust: Trust,
    pub alias: Option<String>,
    pub roles: u16,
    pub home_hub: Option<Vec<u8>>,
    pub first_seen_seq: u64,
    pub last_seen_seq: u64,
    pub request: bool,
    pub pinned: bool,
}

/// One outbox fragment (§10.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxFrag {
    pub message_id: i64,
    pub frag_idx: u8,
    pub frag_count: u8,
    pub peer: [u8; 32],
    pub sealed: Option<Vec<u8>>,
    pub outstanding: bool,
    pub acked: bool,
    pub local_seq: u64,
}

/// A custody item held for a member (§9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustodyItem {
    pub custody_seq: u64,
    pub dest: [u8; 32],
    pub bytes: Vec<u8>,
    pub committed: bool,
}

/// The store's answer to filing a message request (§7.2 (3)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestPut {
    /// Filed (committed FULL).
    Filed,
    /// Already pending for this NodeId: nothing changed.
    AlreadyPending,
    /// Refused by capacity: the fabric drops the INTRO **without** an ACK
    /// (the sender retries later) and counts it.
    Refused(RequestRefusal),
}

/// Why a message request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestRefusal {
    /// `store_full` (§10.4 (4)): pending requests are never pruned, so new
    /// ones are refused like new custody and new bulk.
    StoreFull,
    /// The pending-request quota is reached ([`Store::request_quota`]).
    Quota,
}

/// The store's answer to a `CUSTODY_PUT` (§6.2 statuses it decides; the
/// enrollment checks are the mailbox's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustodyPut {
    /// Stored durably under `custody_seq`.
    Accepted(u64),
    /// Same `pkt_ref` already held, under `custody_seq`.
    Duplicate(u64),
    /// Capacity refuses new custody (`REJECT_FULL`, §10.4).
    RejectFull,
}

impl Store {
    // -- messages ----------------------------------------------------------

    /// One message by id.
    pub fn message(&self, id: i64) -> Result<Option<MessageRow>> {
        let raw = self
            .conn
            .query_row(
                &format!("SELECT {MSG_COLS} FROM messages WHERE id = ?1"),
                [id],
                message_from_row,
            )
            .optional()?;
        raw.map(MessageRowRaw::into_row).transpose()
    }

    /// The conversation with `peer`, oldest first by local sequence (§4:
    /// threads sort by receipt sequence, never by claimed time).
    pub fn messages_with(&self, peer: &[u8; 32], limit: usize) -> Result<Vec<MessageRow>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {MSG_COLS} FROM messages WHERE peer = ?1 ORDER BY local_seq ASC LIMIT ?2"
        ))?;
        let rows = st.query_map(
            params![&peer[..], i64::try_from(limit).unwrap_or(i64::MAX)],
            message_from_row,
        )?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?.into_row()?);
        }
        Ok(out)
    }

    /// Number of stored messages (diagnostics, tests).
    pub fn message_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))?;
        to_u64(n, "count")
    }

    /// Advance a message's §9 state (never backwards: a late `CUSTODY_ACK`
    /// after `delivered` changes nothing). Returns whether it changed.
    /// Reaching `delivered` drops its outbox fragments (the sender's copy is
    /// released only by the recipient's ACK, §6.1).
    pub fn set_message_state(
        &mut self,
        id: i64,
        state: MsgState,
        reason: Option<&str>,
    ) -> Result<bool> {
        if reason.is_some_and(|r| r.len() > MAX_ALIAS) {
            return Err(StoreError::Invalid("reason too long"));
        }
        self.with_tx(Durability::Normal, |tx| {
            let cur: Option<i64> = tx
                .query_row("SELECT state FROM messages WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .optional()?;
            let cur = MsgState::from_i64(cur.ok_or(StoreError::NotFound)?)?;
            if state < cur {
                return Ok(false);
            }
            tx.execute(
                "UPDATE messages SET state = ?2, reason = ?3 WHERE id = ?1",
                params![id, state as i64, reason],
            )?;
            if state == MsgState::Delivered {
                tx.execute("DELETE FROM outbox WHERE message_id = ?1", [id])?;
            }
            Ok(state != cur || reason.is_some())
        })
    }

    /// Pin or unpin a message (pinned items are never pruned, §10.4).
    pub fn set_message_pinned(&mut self, id: i64, pinned: bool) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE messages SET pinned = ?2 WHERE id = ?1",
            params![id, pinned],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    // -- contacts ----------------------------------------------------------

    /// Insert or update a contact. Key material is immutable once pinned
    /// (§7.1, L2 §6.3): a different `enc_pubkey` for a known NodeId is
    /// [`StoreError::KeyConflict`]. The stored announce is replaced only by a
    /// higher counter (§7 stale rule: same keys, counter ≤ stored ⇒ no change
    /// to the announce).
    ///
    /// On a **refresh** (the contact exists): trust is monotone — the stored
    /// value becomes the stronger of the two ([`Trust::rank`]), so an INTRO
    /// or card refresh never downgrades `enrolled` or `oob`; `pinned` and
    /// `request` are never touched (they change only through
    /// [`Store::contact_set_pinned`] / [`Store::contact_set_request`]).
    pub fn contact_upsert(&mut self, c: &ContactRow) -> Result<()> {
        if c.announce.len() > MAX_ANNOUNCE {
            return Err(StoreError::Invalid("announce too large"));
        }
        if c.alias.as_ref().is_some_and(|a| a.len() > MAX_ALIAS) {
            return Err(StoreError::Invalid("alias too long"));
        }
        if c.home_hub.as_ref().is_some_and(|h| h.len() > 32) {
            return Err(StoreError::Invalid("home_hub too long"));
        }
        let counter = to_i64(c.announce_counter, "announce_counter")?;
        self.with_tx(Durability::Full, |tx| {
            let existing: Option<(Vec<u8>, i64, i64)> = tx
                .query_row(
                    "SELECT enc_pubkey, announce_counter, trust FROM contacts WHERE node_id = ?1",
                    [&c.node_id[..]],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let seq = to_i64(next_local_seq(tx)?, "local_seq")?;
            match existing {
                None => {
                    tx.execute(
                        "INSERT INTO contacts (node_id, enc_pubkey, announce, announce_counter, trust,
                             alias, roles, home_hub, first_seen_seq, last_seen_seq, request, pinned)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10, ?11)",
                        params![
                            &c.node_id[..],
                            &c.enc_pubkey[..],
                            &c.announce,
                            counter,
                            c.trust as i64,
                            c.alias,
                            i64::from(c.roles),
                            c.home_hub,
                            seq,
                            c.request,
                            c.pinned,
                        ],
                    )?;
                }
                Some((enc, stored_counter, stored_trust)) => {
                    if enc.as_slice() != c.enc_pubkey.as_slice() {
                        return Err(StoreError::KeyConflict);
                    }
                    let trust = Trust::from_i64(stored_trust)?.max(c.trust);
                    if counter > stored_counter {
                        tx.execute(
                            "UPDATE contacts SET announce = ?2, announce_counter = ?3 WHERE node_id = ?1",
                            params![&c.node_id[..], &c.announce, counter],
                        )?;
                    }
                    tx.execute(
                        "UPDATE contacts SET trust = ?2, alias = ?3, roles = ?4, home_hub = ?5,
                             last_seen_seq = ?6
                         WHERE node_id = ?1",
                        params![
                            &c.node_id[..],
                            trust as i64,
                            c.alias,
                            i64::from(c.roles),
                            c.home_hub,
                            seq,
                        ],
                    )?;
                }
            }
            Ok(())
        })
    }

    /// Pin or unpin a contact (the user's act; a refresh never changes it).
    pub fn contact_set_pinned(&mut self, node_id: &[u8; 32], pinned: bool) -> Result<()> {
        self.contact_flag(node_id, "pinned", pinned)
    }

    /// Set or clear a contact's pending message-request flag (accepting or
    /// filing a request; a refresh never changes it).
    pub fn contact_set_request(&mut self, node_id: &[u8; 32], request: bool) -> Result<()> {
        self.contact_flag(node_id, "request", request)
    }

    fn contact_flag(&mut self, node_id: &[u8; 32], col: &'static str, v: bool) -> Result<()> {
        let n = self.with_tx(Durability::Full, |tx| {
            Ok(tx.execute(
                &format!("UPDATE contacts SET {col} = ?2 WHERE node_id = ?1"),
                params![&node_id[..], v],
            )?)
        })?;
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    /// One contact.
    pub fn contact(&self, node_id: &[u8; 32]) -> Result<Option<ContactRow>> {
        let row = self
            .conn
            .query_row(
                "SELECT node_id, enc_pubkey, announce, announce_counter, trust, alias, roles,
                        home_hub, first_seen_seq, last_seen_seq, request, pinned
                 FROM contacts WHERE node_id = ?1",
                [&node_id[..]],
                contact_raw,
            )
            .optional()?;
        row.map(contact_from_raw).transpose()
    }

    /// Every contact, by first-seen order.
    pub fn contacts(&self) -> Result<Vec<ContactRow>> {
        let mut st = self.conn.prepare(
            "SELECT node_id, enc_pubkey, announce, announce_counter, trust, alias, roles,
                    home_hub, first_seen_seq, last_seen_seq, request, pinned
             FROM contacts ORDER BY first_seen_seq",
        )?;
        let rows = st.query_map([], contact_raw)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(contact_from_raw(r?)?);
        }
        Ok(out)
    }

    /// Forget a contact (the user's act; never a pruning step). Receive and
    /// transmit sequence state is kept — below a prefix nothing is forgotten.
    pub fn contact_forget(&mut self, node_id: &[u8; 32]) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM contacts WHERE node_id = ?1", [&node_id[..]])?
            > 0)
    }

    // -- requests (pending INTROs, §7.2) -----------------------------------

    /// File a message request (idempotent; never pruned while pending).
    ///
    /// Admission (§10.4 (4)): pending requests cannot be pruned, so a new
    /// one is [`RequestPut::Refused`] while the store is full (after a
    /// pruning pass in the normative order, like new custody and new bulk)
    /// or when the pending count has reached [`Store::request_quota`]. A
    /// NodeId already pending is never refused. Request rows live in the DB,
    /// so they count in [`Store::live_bytes`] like every other row.
    pub fn request_put(&mut self, node_id: &[u8; 32], announce: &[u8]) -> Result<RequestPut> {
        if announce.len() > MAX_ANNOUNCE {
            return Err(StoreError::Invalid("announce too large"));
        }
        if self.request_pending(node_id)? {
            return Ok(RequestPut::AlreadyPending);
        }
        if self.request_count()? >= self.request_quota()? {
            return Ok(RequestPut::Refused(RequestRefusal::Quota));
        }
        let bytes = (node_id.len() + announce.len()) as u64 + super::prune::ROW_OVERHEAD;
        if !self.admit_unclassed(bytes)? {
            return Ok(RequestPut::Refused(RequestRefusal::StoreFull));
        }
        self.with_tx(Durability::Full, |tx| {
            let seq = to_i64(next_local_seq(tx)?, "local_seq")?;
            let n = tx.execute(
                "INSERT INTO requests (node_id, announce, local_seq) VALUES (?1, ?2, ?3)
                 ON CONFLICT (node_id) DO NOTHING",
                params![&node_id[..], announce, seq],
            )?;
            Ok(if n == 0 {
                RequestPut::AlreadyPending
            } else {
                RequestPut::Filed
            })
        })
    }

    /// Whether a request from `node_id` is pending.
    pub fn request_pending(&self, node_id: &[u8; 32]) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM requests WHERE node_id = ?1",
                [&node_id[..]],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Number of pending requests.
    pub fn request_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM requests", [], |r| r.get(0))?;
        to_u64(n, "count")
    }

    /// Resolve (accept or dismiss) a request.
    pub fn request_remove(&mut self, node_id: &[u8; 32]) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM requests WHERE node_id = ?1", [&node_id[..]])?
            > 0)
    }

    /// Pending requests, oldest first: `(node_id, announce)`.
    pub fn requests(&self) -> Result<Vec<([u8; 32], Vec<u8>)>> {
        let mut st = self
            .conn
            .prepare("SELECT node_id, announce FROM requests ORDER BY local_seq")?;
        let rows = st.query_map([], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id, a) = r?;
            out.push((id32(id, "requests.node_id")?, a));
        }
        Ok(out)
    }

    // -- outbox (§10.3) ----------------------------------------------------

    /// Put (or replace) one fragment of an own message awaiting a path.
    /// `sealed` is replaced on re-seal (fresh epk + counter, §5).
    pub fn outbox_put(
        &mut self,
        message_id: i64,
        frag_idx: u8,
        frag_count: u8,
        peer: &[u8; 32],
        sealed: Option<&[u8]>,
    ) -> Result<()> {
        if frag_count == 0 || frag_count > MAX_FRAGS || frag_idx >= frag_count {
            return Err(StoreError::Invalid("fragment bounds"));
        }
        if sealed.is_some_and(|s| s.len() > MAX_SEALED) {
            return Err(StoreError::Invalid("sealed fragment too large"));
        }
        self.with_tx(Durability::Normal, |tx| {
            let seq = to_i64(next_local_seq(tx)?, "local_seq")?;
            tx.execute(
                "INSERT INTO outbox (message_id, frag_idx, frag_count, peer, sealed, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (message_id, frag_idx) DO UPDATE SET sealed = excluded.sealed",
                params![
                    message_id,
                    i64::from(frag_idx),
                    i64::from(frag_count),
                    &peer[..],
                    sealed,
                    seq
                ],
            )?;
            Ok(())
        })
    }

    /// Mark a fragment as written to a link and not yet ACKed (or not).
    pub fn outbox_set_outstanding(
        &mut self,
        message_id: i64,
        frag_idx: u8,
        outstanding: bool,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE outbox SET outstanding = ?3 WHERE message_id = ?1 AND frag_idx = ?2",
            params![message_id, i64::from(frag_idx), outstanding],
        )?;
        Ok(())
    }

    /// Apply a SACK bitmap (§6.1 partials): set fragments acked, clear their
    /// outstanding flag. Returns whether every fragment is now acked.
    pub fn outbox_ack(&mut self, message_id: i64, bitmap: u16) -> Result<bool> {
        self.with_tx(Durability::Normal, |tx| {
            for idx in 0..MAX_FRAGS {
                if bitmap & (1 << idx) != 0 {
                    tx.execute(
                        "UPDATE outbox SET acked = 1, outstanding = 0
                         WHERE message_id = ?1 AND frag_idx = ?2",
                        params![message_id, i64::from(idx)],
                    )?;
                }
            }
            let left: i64 = tx.query_row(
                "SELECT count(*) FROM outbox WHERE message_id = ?1 AND acked = 0",
                [message_id],
                |r| r.get(0),
            )?;
            Ok(left == 0)
        })
    }

    /// Un-acked fragments for `peer`, oldest first.
    pub fn outbox_for(&self, peer: &[u8; 32]) -> Result<Vec<OutboxFrag>> {
        let mut st = self.conn.prepare(
            "SELECT message_id, frag_idx, frag_count, peer, sealed, outstanding, acked, local_seq
             FROM outbox WHERE peer = ?1 AND acked = 0 ORDER BY local_seq, frag_idx",
        )?;
        let rows = st.query_map([&peer[..]], outbox_raw)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(outbox_from_raw(r?)?);
        }
        Ok(out)
    }

    /// Peers with anything in the outbox (retry on link/contact events, §9).
    pub fn outbox_peers(&self) -> Result<Vec<[u8; 32]>> {
        let mut st = self
            .conn
            .prepare("SELECT DISTINCT peer FROM outbox WHERE acked = 0 ORDER BY peer")?;
        let rows = st.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(id32(r?, "outbox.peer")?);
        }
        Ok(out)
    }

    /// Outstanding (written, un-ACKed) fragments to `peer` (§5: ≤ 16).
    pub fn outbox_outstanding(&self, peer: &[u8; 32]) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM outbox WHERE peer = ?1 AND outstanding = 1 AND acked = 0",
            [&peer[..]],
            |r| r.get(0),
        )?;
        to_u64(n, "count")
    }

    // -- custody (hub role, §9) --------------------------------------------

    /// Hold a sealed item for member `dest`. Dedup by `pkt_ref`
    /// (`BLAKE2b-128("111/custody/v1" ‖ bytes)`, §6.2), computed **here**
    /// from the exact item bytes — never taken from the caller, so a row's
    /// `pkt_ref` always matches what it holds ([`crate::custody::pkt_ref`]
    /// gives the same value for the receipt). `Accepted` is committed with
    /// `synchronous = FULL` — the receipt promises the hub holds it.
    /// Capacity may answer `RejectFull` (§10.4).
    pub fn custody_put(&mut self, dest: &[u8; 32], bytes: &[u8]) -> Result<CustodyPut> {
        if bytes.len() > MAX_CUSTODY_ITEM {
            return Err(StoreError::Invalid("custody item too large"));
        }
        let pkt_ref = &crate::custody::pkt_ref(bytes);
        if let Some(seq) = self.custody_by_ref(pkt_ref)? {
            return Ok(CustodyPut::Duplicate(seq));
        }
        if !self.admit(super::Class::Custody, bytes.len() as u64)? {
            return Ok(CustodyPut::RejectFull);
        }
        self.with_tx(Durability::Full, |tx| {
            let seq = to_i64(next_local_seq(tx)?, "local_seq")?;
            tx.execute(
                "INSERT INTO custody (dest, pkt_ref, bytes, local_seq) VALUES (?1, ?2, ?3, ?4)",
                params![&dest[..], &pkt_ref[..], bytes, seq],
            )?;
            Ok(CustodyPut::Accepted(to_u64(
                tx.last_insert_rowid(),
                "custody_seq",
            )?))
        })
    }

    fn custody_by_ref(&self, pkt_ref: &[u8; 16]) -> Result<Option<u64>> {
        let seq: Option<i64> = self
            .conn
            .query_row(
                "SELECT custody_seq FROM custody WHERE pkt_ref = ?1",
                [&pkt_ref[..]],
                |r| r.get(0),
            )
            .optional()?;
        seq.map(|s| to_u64(s, "custody_seq")).transpose()
    }

    /// Items for `dest` above `cursor` (exclusive), ascending, at most
    /// `limit` (§9 `MAILBOX_FETCH`: that member's own items, nothing else).
    pub fn custody_fetch(
        &self,
        dest: &[u8; 32],
        cursor: u64,
        limit: usize,
    ) -> Result<Vec<CustodyItem>> {
        let mut st = self.conn.prepare(
            "SELECT custody_seq, dest, bytes, committed FROM custody
             WHERE dest = ?1 AND custody_seq > ?2 ORDER BY custody_seq LIMIT ?3",
        )?;
        let rows = st.query_map(
            params![
                &dest[..],
                to_i64(cursor, "cursor")?,
                i64::try_from(limit).unwrap_or(i64::MAX)
            ],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, bool>(3)?,
                ))
            },
        )?;
        let mut out = Vec::new();
        for r in rows {
            let (seq, d, bytes, committed) = r?;
            out.push(CustodyItem {
                custody_seq: to_u64(seq, "custody_seq")?,
                dest: id32(d, "custody.dest")?,
                bytes,
                committed,
            });
        }
        Ok(out)
    }

    /// §9 `MAILBOX_COMMIT {cursor}`: mark `dest`'s items ≤ `cursor`
    /// committed (evict-first, not deleted). Idempotent and monotonic — the
    /// effective cursor is `max(cursor)`. Returns rows newly committed.
    pub fn custody_commit(&mut self, dest: &[u8; 32], cursor: u64) -> Result<u64> {
        let n = self.conn.execute(
            "UPDATE custody SET committed = 1
             WHERE dest = ?1 AND custody_seq <= ?2 AND committed = 0",
            params![&dest[..], to_i64(cursor, "cursor")?],
        )?;
        Ok(n as u64)
    }

    /// The highest custody sequence ever issued (0 if none) — never lowered
    /// by pruning (`ENROLL_OK` `custody_seq_floor`, §7.3).
    pub fn custody_high(&self) -> Result<u64> {
        let v: Option<i64> = self
            .conn
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name = 'custody'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        to_u64(v.unwrap_or(0), "custody seq")
    }

    /// `(count, bytes)` of custody held (`GET /fabric`).
    pub fn custody_stats(&self) -> Result<(u64, u64)> {
        let (n, b): (i64, i64) = self.conn.query_row(
            "SELECT count(*), coalesce(sum(length(bytes)), 0) FROM custody",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((to_u64(n, "count")?, to_u64(b, "bytes")?))
    }

    // -- cursors (member side, §9) -----------------------------------------

    /// This member's cursor at `hub` (0 if never fetched).
    pub fn cursor(&self, hub: &[u8; 32]) -> Result<u64> {
        let v: Option<i64> = self
            .conn
            .query_row(
                "SELECT cursor FROM cursors WHERE hub = ?1",
                [&hub[..]],
                |r| r.get(0),
            )
            .optional()?;
        to_u64(v.unwrap_or(0), "cursor")
    }

    /// Advance the cursor at `hub` to `max(stored, cursor)`; returns it.
    pub fn advance_cursor(&mut self, hub: &[u8; 32], cursor: u64) -> Result<u64> {
        let c = to_i64(cursor, "cursor")?;
        self.with_tx(Durability::Full, |tx| {
            tx.execute(
                "INSERT INTO cursors (hub, cursor) VALUES (?1, ?2)
                 ON CONFLICT (hub) DO UPDATE SET cursor = max(cursor, excluded.cursor)",
                params![&hub[..], c],
            )?;
            let v: i64 = tx.query_row(
                "SELECT cursor FROM cursors WHERE hub = ?1",
                [&hub[..]],
                |r| r.get(0),
            )?;
            to_u64(v, "cursor")
        })
    }

    /// The current local sequence (the store's age clock; diagnostics).
    pub fn local_seq(&self) -> Result<u64> {
        current_local_seq(&self.conn)
    }
}

type ContactRaw = (
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    i64,
    i64,
    Option<String>,
    i64,
    Option<Vec<u8>>,
    i64,
    i64,
    bool,
    bool,
);

fn contact_raw(r: &Row<'_>) -> rusqlite::Result<ContactRaw> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
        r.get(11)?,
    ))
}

fn contact_from_raw(v: ContactRaw) -> Result<ContactRow> {
    let (node_id, enc, announce, counter, trust, alias, roles, home_hub, first, last, req, pin) = v;
    Ok(ContactRow {
        node_id: id32(node_id, "contacts.node_id")?,
        enc_pubkey: id32(enc, "contacts.enc_pubkey")?,
        announce,
        announce_counter: to_u64(counter, "contacts.announce_counter")?,
        trust: Trust::from_i64(trust)?,
        alias,
        roles: u16::try_from(roles).map_err(|_| StoreError::Corrupt("contacts.roles"))?,
        home_hub,
        first_seen_seq: to_u64(first, "contacts.first_seen_seq")?,
        last_seen_seq: to_u64(last, "contacts.last_seen_seq")?,
        request: req,
        pinned: pin,
    })
}

type OutboxRaw = (i64, i64, i64, Vec<u8>, Option<Vec<u8>>, bool, bool, i64);

fn outbox_raw(r: &Row<'_>) -> rusqlite::Result<OutboxRaw> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
    ))
}

fn outbox_from_raw(v: OutboxRaw) -> Result<OutboxFrag> {
    let (message_id, idx, count, peer, sealed, outstanding, acked, local) = v;
    Ok(OutboxFrag {
        message_id,
        frag_idx: u8::try_from(idx).map_err(|_| StoreError::Corrupt("outbox.frag_idx"))?,
        frag_count: u8::try_from(count).map_err(|_| StoreError::Corrupt("outbox.frag_count"))?,
        peer: id32(peer, "outbox.peer")?,
        sealed,
        outstanding,
        acked,
        local_seq: to_u64(local, "outbox.local_seq")?,
    })
}
