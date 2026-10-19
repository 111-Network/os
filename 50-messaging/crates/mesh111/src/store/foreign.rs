//! Layer 6 §3 — typed access to the foreign message model (schema v3,
//! D25 F1): `foreign_conversations`, `foreign_messages`, `relay_edges`,
//! `foreign_seen`, `send_groups`.
//!
//! - **One order (§3 "Order and time").** Every foreign row (inbound, own,
//!   relay copy, send group) draws `local_seq` from the store-wide counter
//!   ([`super::next_local_seq`]), the same one 111 rows use: one key, one
//!   prune pass, one order. No clock orders or decides anything; `claim_ts`
//!   is the sender's claim, stored for display only.
//! - **One transaction per call.** Each method is synchronous and runs in
//!   one transaction; the bridge core batches ingest into
//!   [`Store::foreign_insert_batch`] (at most [`MAX_FOREIGN_BATCH`] rows,
//!   §3 "Typed writes, batched"). Sizes are checked before anything is
//!   written; a batch with one bad row writes nothing.
//! - **Text is the caller's.** The §3 ingest function (Cc/Cf strip, NFC,
//!   the 64 B name cap, the `truncated` mark) is the bridge core's; the
//!   store only enforces byte caps ([`MAX_FOREIGN_BODY`],
//!   [`MAX_FOREIGN_FIELD`]) and refuses, never cuts.
//! - **Dedupe (§3).** Key `(conv, sender_native, native_msg_id)`. A row
//!   whose id is globally unique (LXMF hash, app-peer ids) is a duplicate on
//!   the key alone; otherwise (Meshtastic: 32-bit packet ids repeat) only
//!   when the key **and** the body match within the last
//!   [`DEDUPE_WINDOW`] **inbound** rows of the conversation (exact body
//!   equality: the strongest form of the spec's body-hash match). Own rows
//!   never count in the window, so a burst of our own sends or relay copies
//!   cannot push an inbound row out of it; recognising our own echo is §8
//!   guard 2's (the native id [`Store::foreign_accepted`] records).
//! - **Monotone states (§4).** [`ForeignState::may_become`] over
//!   [`ForeignState::rank`]: `queued` < `sent` < `relayed` < `failed` <
//!   `delivered`, with one exception: `failed` is refused once a row is
//!   `relayed` (`failed` is only a refusal **before air**; a post-air NAK
//!   leaves `relayed`). A refused move is ignored
//!   ([`StateChange::Unchanged`]); `delivered` never regresses; a later
//!   `delivered` overrides `failed`.
//! - **At most once per user act (§4).** `meta.handoff = "pending"` is
//!   written durably before a handoff ([`Store::foreign_handoff_begin`]) and
//!   cleared by `Accepted` ([`Store::foreign_accepted`]), or withdrawn by
//!   [`Store::foreign_handoff_abort`] when the carrier refused the row
//!   before air without an `Accepted`; a pending row is never offered again
//!   by [`Store::foreign_queued`], never cancelled ([`Store::foreign_cancel`])
//!   and never shed as backlog (prune 2b). `handoff` is a reserved `meta`
//!   key: [`Store::foreign_set_meta`] refuses it, so the mark is
//!   representable only through those three calls.
//! - **Cancel is the user's (§9).** [`Store::foreign_cancel`] deletes one
//!   queued, never-handed **user-composed** row (`group_id` set). A relay
//!   copy is not cancelled: copies leave only by a shed (prune 2b), an edge
//!   disable or an edge delete, which the edge's counters and the
//!   destination's `Conversation` change explain.
//! - **Relay edges (§8).** Inputs are inbound rows of the source with
//!   `local_seq > max(cursor_seq, relayed_seq)` that are not relayed copies
//!   ([`Store::edge_inputs`]); queueing a copy advances `relayed_seq` past
//!   its origin in the same transaction, so a crash can never make an origin
//!   relayable twice; history before `cursor_seq` is never relayed.
//! - **One copy per origin per edge (§8 guard 6; the store is the ID map and
//!   fails closed).** A copy whose origin is at or below the edge's
//!   `relayed_seq` is refused, except for the further parts of a split copy
//!   queued in the same [`Store::foreign_queue_sends`] call; the partial
//!   `UNIQUE (edge_id, origin_msg, part_k)` index makes a duplicate part
//!   unrepresentable. Within one call the parts of one copy are checked
//!   against each other: the first is part 1, each further part is the
//!   previous `part_k + 1` with the same `part_n`, and the copy is complete
//!   (its last part is `part_n`) by the end of the call. A call queues
//!   origins in **ascending `local_seq` order**: a lower origin after a
//!   higher one on the same edge is at or below the advanced mark, so a
//!   descending batch fails as a whole (fail closed, nothing written).
//! - **Disabling an edge drops its backlog.** [`Store::edge_set_enabled`]
//!   `(false)` deletes the edge's copies still `queued` without a pending
//!   handoff (never handed to a carrier), as [`Store::edge_delete`] does: a
//!   disabled edge has no backlog, and re-enabling moves `cursor_seq` to the
//!   current `local_seq` anyway, so nothing dropped is ever re-queued. Copies
//!   with a pending handoff (may have been transmitted) are kept.
//!
//! Ids are plain row ids; the `fc-`/`fm-`/`be-`/`sg-` prefixes are the
//! daemon's (contract layer).

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::{
    current_local_seq, next_local_seq, to_i64, to_u64, Durability, Result, Store, StoreError,
};

/// Largest stored foreign body or send text (§3: 64 KiB; a longer text is
/// stored cut by the caller with `meta.truncated`).
pub const MAX_FOREIGN_BODY: usize = 64 * 1024;
/// Rows per [`Store::foreign_insert_batch`] / sends per
/// [`Store::foreign_queue_sends`] (§3: a bounded batch per adapter tick).
pub const MAX_FOREIGN_BATCH: usize = 256;
/// Loop-guard keys kept per edge (§3 `foreign_seen`; oldest evicted).
pub const FOREIGN_SEEN_CAP: u64 = 4096;
/// Rows of a conversation a repeating-id duplicate is looked for in (§3).
pub const DEDUPE_WINDOW: i64 = 32;
/// Largest native id, account, title, sender name, evidence, reason code,
/// consent hash or `enabled_by` (bytes).
pub const MAX_FOREIGN_FIELD: usize = 256;
/// Largest `meta` JSON object (bytes).
pub const MAX_FOREIGN_META: usize = 4096;
/// Largest `send_groups.targets` JSON (bytes).
pub const MAX_SEND_TARGETS: usize = 64 * 1024;
/// Largest network id (`meshtastic`, `lxmf`, `app:deltachat`, …).
pub const MAX_NETWORK_ID: usize = 32;

const MAX_META_KEY: usize = 32;

// ---------------------------------------------------------------------------
// Closed enums (stored as CHECKed text)
// ---------------------------------------------------------------------------

macro_rules! text_enum {
    ($(#[$m:meta])* $name:ident, $what:literal { $($(#[$vm:meta])* $v:ident => $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($(#[$vm])* $v),+ }

        impl $name {
            /// The stored (and contract) spelling.
            pub fn as_str(self) -> &'static str {
                match self { $($name::$v => $s),+ }
            }

            /// Parse the stored spelling.
            pub fn parse(s: &str) -> Option<$name> {
                match s { $($s => Some($name::$v),)+ _ => None }
            }

            fn from_col(s: &str) -> Result<$name> {
                $name::parse(s).ok_or(StoreError::Corrupt($what))
            }
        }
    };
}

text_enum!(
    /// `foreign_conversations.kind` (§3).
    ForeignKind, "foreign kind" {
        Channel => "channel",
        Dm => "dm",
        Group => "group",
    }
);

text_enum!(
    /// `foreign_messages.dir` (§3).
    ForeignDir, "foreign dir" {
        In => "in",
        Own => "own",
    }
);

text_enum!(
    /// §4 `ForeignMessageState` (closed; 111's `MsgState` stays at four).
    ForeignState, "foreign state" {
        Queued => "queued",
        Sent => "sent",
        Relayed => "relayed",
        Delivered => "delivered",
        Failed => "failed",
    }
);

text_enum!(
    /// §3 authentication label (closed; copy from the adapter manifest).
    ForeignAuth, "foreign auth" {
        None => "none",
        ChannelKey => "channel-key",
        Pki => "pki",
        Signed => "signed",
        /// Reserved for the second release (§14, §17).
        Attested111 => "attested-111",
    }
);

text_enum!(
    /// §8 per-edge oversize policy (`refuse` default; never truncated).
    EdgeOversize, "edge oversize" {
        Refuse => "refuse",
        Split => "split",
    }
);

impl ForeignState {
    /// The §4 order: `queued` 0 < `sent` 1 < `relayed` 2 < `failed` 3 <
    /// `delivered` 4. `failed` is terminal unless a later `delivered`
    /// arrives; `delivered` never regresses. Which moves are taken is
    /// [`ForeignState::may_become`] (the order with one exception).
    pub fn rank(self) -> u8 {
        match self {
            ForeignState::Queued => 0,
            ForeignState::Sent => 1,
            ForeignState::Relayed => 2,
            ForeignState::Failed => 3,
            ForeignState::Delivered => 4,
        }
    }

    /// Whether a row in `self` takes `next` (§4 monotone states). The same
    /// state is taken (only a differing evidence / reason is then written);
    /// otherwise only a higher [`rank`](ForeignState::rank), except
    /// `relayed` → `failed`: `failed` is only a refusal **before air**, and
    /// a `relayed` row was transmitted (a post-air NAK leaves `relayed` with
    /// "transmitted, not confirmed"). `sent` → `failed` stays: a NAK to a
    /// packet the radio accepted into its queue is still before air.
    pub fn may_become(self, next: ForeignState) -> bool {
        if next == self {
            return true;
        }
        match (self, next) {
            (ForeignState::Relayed, ForeignState::Failed) => false,
            _ => next.rank() > self.rank(),
        }
    }
}

/// The §8 per-edge counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeCounter {
    /// Copies handed to the destination.
    Relayed,
    /// Backlog copies shed by capacity (prune step 2b counts these itself).
    Shed,
    /// Inputs not relayed (too long under `refuse`, `ok_to_mqtt` absent).
    Refused,
}

impl EdgeCounter {
    fn col(self) -> &'static str {
        match self {
            EdgeCounter::Relayed => "relayed",
            EdgeCounter::Shed => "shed",
            EdgeCounter::Refused => "refused",
        }
    }
}

/// [`Store::foreign_set_state`]'s outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateChange {
    /// The state (or its evidence / reason) was written.
    Changed,
    /// Ignored: the state would move back (§4), or nothing differs.
    Unchanged,
}

/// One `meta` value ([`Store::foreign_set_meta`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaValue<'a> {
    Str(&'a str),
    Int(i64),
    Bool(bool),
}

// ---------------------------------------------------------------------------
// Rows and inputs
// ---------------------------------------------------------------------------

/// A foreign conversation with its counts (§3, §10 `ForeignConversation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignConversationRow {
    pub id: i64,
    pub network: String,
    pub account: String,
    pub kind: ForeignKind,
    pub native_id: String,
    pub title: Option<String>,
    /// `local_seq` of the newest row (0 when empty).
    pub last_seq: u64,
    /// Inbound rows since the last [`Store::foreign_conversation_mark_read`].
    pub unread: u64,
    pub pinned: bool,
    /// Rows held.
    pub messages: u64,
    /// Own rows `queued`.
    pub queued: u64,
}

/// A stored foreign message (one row per part of a split send).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignMessageRow {
    pub id: i64,
    pub conv: i64,
    pub dir: ForeignDir,
    pub sender_native: String,
    pub sender_name: Option<String>,
    pub auth: ForeignAuth,
    pub native_msg_id: Option<String>,
    pub body: String,
    pub local_seq: u64,
    /// The sender's claimed Unix seconds (display only, behind "sender's
    /// claim"; never orders anything).
    pub claim_ts: Option<i64>,
    pub state: ForeignState,
    pub evidence: Option<String>,
    pub reason_code: Option<String>,
    pub group_id: Option<i64>,
    pub part_k: u32,
    pub part_n: u32,
    pub origin_msg: Option<i64>,
    pub edge_id: Option<i64>,
    /// The JSON object (snr, rssi, hops, `via_mqtt`, `ok_to_mqtt`,
    /// `truncated`, `handoff`).
    pub meta: String,
    /// `meta.handoff = "pending"`: may have been sent; never re-sent
    /// automatically (§4).
    pub handoff_pending: bool,
}

/// One inbound row for [`Store::foreign_insert_batch`] (after the bridge
/// core's §3 ingest function).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignInsert {
    pub conv: i64,
    pub sender_native: String,
    pub sender_name: Option<String>,
    pub auth: ForeignAuth,
    /// `None`: no native id, never a duplicate.
    pub native_msg_id: Option<String>,
    pub body: String,
    pub claim_ts: Option<i64>,
    /// A JSON object, or `None` for `{}`.
    pub meta: Option<String>,
    /// The id is globally unique (LXMF hash, app-peer id): the key alone
    /// makes a duplicate. `false` (Meshtastic): key **and** body within the
    /// last [`DEDUPE_WINDOW`] rows.
    pub id_globally_unique: bool,
}

/// One own row to queue ([`Store::foreign_queue_send`]). Exactly one of
/// `group_id` (user-composed) or `edge_id` + `origin_msg` (a relay copy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForeignSend<'a> {
    pub conv: i64,
    pub text: &'a str,
    pub group_id: Option<i64>,
    pub part_k: u32,
    pub part_n: u32,
    pub origin_msg: Option<i64>,
    pub edge_id: Option<i64>,
}

/// One target part of a send group ([`Store::send_group_create`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupPart<'a> {
    pub conv: i64,
    pub text: &'a str,
    pub part_k: u32,
    pub part_n: u32,
}

/// A send group (§9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendGroupRow {
    pub id: i64,
    pub local_seq: u64,
    pub text: String,
    /// JSON (§9 targets: 111 contacts and foreign conversations).
    pub targets: String,
}

/// A new relay edge (§8). Created enabled, `cursor_seq` = the current
/// `local_seq` (history is never relayed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeNew<'a> {
    pub src_conv: i64,
    pub dst_conv: i64,
    pub honour_ok_to_mqtt: bool,
    pub oversize: EdgeOversize,
    pub consent: EdgeConsent<'a>,
}

/// The recorded human consent (§8: the warning's hash and who accepted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeConsent<'a> {
    pub consent_hash: &'a str,
    pub enabled_by: &'a str,
}

/// A stored relay edge (§3 `relay_edges`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeRow {
    pub id: i64,
    pub src_conv: i64,
    pub dst_conv: i64,
    pub src_kind: ForeignKind,
    pub dst_kind: ForeignKind,
    pub enabled: bool,
    pub paused: bool,
    pub cursor_seq: u64,
    pub relayed_seq: u64,
    pub honour_ok_to_mqtt: bool,
    pub oversize: EdgeOversize,
    pub consent_hash: String,
    pub enabled_by: String,
    pub relayed: u64,
    pub shed: u64,
    pub refused: u64,
}

// ---------------------------------------------------------------------------
// Column lists and row mapping
// ---------------------------------------------------------------------------

const FM_COLS: &str = "m.id, m.conv, m.dir, m.sender_native, m.sender_name, m.auth, \
     m.native_msg_id, m.body, m.local_seq, m.claim_ts, m.state, m.evidence, m.reason_code, \
     m.group_id, m.part_k, m.part_n, m.origin_msg, m.edge_id, m.meta, \
     coalesce(json_extract(m.meta, '$.handoff') = 'pending', 0)";

struct FmRaw {
    id: i64,
    conv: i64,
    dir: String,
    sender_native: String,
    sender_name: Option<String>,
    auth: String,
    native_msg_id: Option<String>,
    body: String,
    local_seq: i64,
    claim_ts: Option<i64>,
    state: String,
    evidence: Option<String>,
    reason_code: Option<String>,
    group_id: Option<i64>,
    part_k: i64,
    part_n: i64,
    origin_msg: Option<i64>,
    edge_id: Option<i64>,
    meta: String,
    handoff_pending: bool,
}

fn fm_raw(r: &Row<'_>) -> rusqlite::Result<FmRaw> {
    Ok(FmRaw {
        id: r.get(0)?,
        conv: r.get(1)?,
        dir: r.get(2)?,
        sender_native: r.get(3)?,
        sender_name: r.get(4)?,
        auth: r.get(5)?,
        native_msg_id: r.get(6)?,
        body: r.get(7)?,
        local_seq: r.get(8)?,
        claim_ts: r.get(9)?,
        state: r.get(10)?,
        evidence: r.get(11)?,
        reason_code: r.get(12)?,
        group_id: r.get(13)?,
        part_k: r.get(14)?,
        part_n: r.get(15)?,
        origin_msg: r.get(16)?,
        edge_id: r.get(17)?,
        meta: r.get(18)?,
        handoff_pending: r.get(19)?,
    })
}

fn part(v: i64) -> Result<u32> {
    u32::try_from(v).map_err(|_| StoreError::Corrupt("foreign_messages.part"))
}

impl FmRaw {
    fn into_row(self) -> Result<ForeignMessageRow> {
        Ok(ForeignMessageRow {
            id: self.id,
            conv: self.conv,
            dir: ForeignDir::from_col(&self.dir)?,
            sender_native: self.sender_native,
            sender_name: self.sender_name,
            auth: ForeignAuth::from_col(&self.auth)?,
            native_msg_id: self.native_msg_id,
            body: self.body,
            local_seq: to_u64(self.local_seq, "foreign_messages.local_seq")?,
            claim_ts: self.claim_ts,
            state: ForeignState::from_col(&self.state)?,
            evidence: self.evidence,
            reason_code: self.reason_code,
            group_id: self.group_id,
            part_k: part(self.part_k)?,
            part_n: part(self.part_n)?,
            origin_msg: self.origin_msg,
            edge_id: self.edge_id,
            meta: self.meta,
            handoff_pending: self.handoff_pending,
        })
    }
}

fn collect_fm(
    c: &Connection,
    sql: &str,
    p: impl rusqlite::Params,
) -> Result<Vec<ForeignMessageRow>> {
    let mut st = c.prepare(sql)?;
    let rows = st.query_map(p, fm_raw)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?.into_row()?);
    }
    Ok(out)
}

const FC_COLS: &str = "c.id, c.network, c.account, c.kind, c.native_id, c.title, c.last_seq, \
     c.unread, c.pinned, \
     (SELECT count(*) FROM foreign_messages m WHERE m.conv = c.id), \
     (SELECT count(*) FROM foreign_messages m \
      WHERE m.conv = c.id AND m.dir = 'own' AND m.state = 'queued')";

type FcRaw = (
    i64,
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
    i64,
    bool,
    i64,
    i64,
);

fn fc_raw(r: &Row<'_>) -> rusqlite::Result<FcRaw> {
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
    ))
}

fn fc_from_raw(v: FcRaw) -> Result<ForeignConversationRow> {
    Ok(ForeignConversationRow {
        id: v.0,
        network: v.1,
        account: v.2,
        kind: ForeignKind::from_col(&v.3)?,
        native_id: v.4,
        title: v.5,
        last_seq: to_u64(v.6, "foreign_conversations.last_seq")?,
        unread: to_u64(v.7, "foreign_conversations.unread")?,
        pinned: v.8,
        messages: to_u64(v.9, "count")?,
        queued: to_u64(v.10, "count")?,
    })
}

const EDGE_COLS: &str = "id, src_conv, dst_conv, src_kind, dst_kind, enabled, paused, \
     cursor_seq, relayed_seq, honour_ok_to_mqtt, oversize, consent_hash, enabled_by, \
     relayed, shed, refused";

type EdgeRaw = (
    (i64, i64, i64, String, String, bool, bool, i64),
    (i64, bool, String, String, String, i64, i64, i64),
);

fn edge_raw(r: &Row<'_>) -> rusqlite::Result<EdgeRaw> {
    Ok((
        (
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
        ),
        (
            r.get(8)?,
            r.get(9)?,
            r.get(10)?,
            r.get(11)?,
            r.get(12)?,
            r.get(13)?,
            r.get(14)?,
            r.get(15)?,
        ),
    ))
}

fn edge_from_raw(v: EdgeRaw) -> Result<EdgeRow> {
    let ((id, src_conv, dst_conv, src_kind, dst_kind, enabled, paused, cursor), rest) = v;
    let (relayed_seq, honour, oversize, consent_hash, enabled_by, relayed, shed, refused) = rest;
    Ok(EdgeRow {
        id,
        src_conv,
        dst_conv,
        src_kind: ForeignKind::from_col(&src_kind)?,
        dst_kind: ForeignKind::from_col(&dst_kind)?,
        enabled,
        paused,
        cursor_seq: to_u64(cursor, "relay_edges.cursor_seq")?,
        relayed_seq: to_u64(relayed_seq, "relay_edges.relayed_seq")?,
        honour_ok_to_mqtt: honour,
        oversize: EdgeOversize::from_col(&oversize)?,
        consent_hash,
        enabled_by,
        relayed: to_u64(relayed, "relay_edges.relayed")?,
        shed: to_u64(shed, "relay_edges.shed")?,
        refused: to_u64(refused, "relay_edges.refused")?,
    })
}

fn edge_in(c: &Connection, id: i64) -> Result<Option<EdgeRow>> {
    let raw = c
        .query_row(
            &format!("SELECT {EDGE_COLS} FROM relay_edges WHERE id = ?1"),
            [id],
            edge_raw,
        )
        .optional()?;
    raw.map(edge_from_raw).transpose()
}

// ---------------------------------------------------------------------------
// Validation (sizes before anything is written)
// ---------------------------------------------------------------------------

fn field(v: &str, what: &'static str) -> Result<()> {
    if v.len() > MAX_FOREIGN_FIELD {
        return Err(StoreError::Invalid(what));
    }
    Ok(())
}

fn field_opt(v: Option<&str>, what: &'static str) -> Result<()> {
    v.map_or(Ok(()), |v| field(v, what))
}

fn nonempty_field(v: &str, what: &'static str) -> Result<()> {
    if v.is_empty() {
        return Err(StoreError::Invalid(what));
    }
    field(v, what)
}

fn body(v: &str, what: &'static str) -> Result<()> {
    if v.len() > MAX_FOREIGN_BODY {
        return Err(StoreError::Invalid(what));
    }
    Ok(())
}

/// A network id: `[a-z0-9:_-]`, 1..=32 bytes, never `111` (the fabric's).
fn network_id(v: &str) -> Result<()> {
    let ok = !v.is_empty()
        && v.len() <= MAX_NETWORK_ID
        && v != "111"
        && v.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_:".contains(&b));
    if ok {
        Ok(())
    } else {
        Err(StoreError::Invalid("network id"))
    }
}

fn check_parts(k: u32, n: u32) -> Result<()> {
    if n == 0 || k == 0 || k > n {
        return Err(StoreError::Invalid("part numbering"));
    }
    Ok(())
}

/// `meta` must be a JSON object of bounded size (checked by SQLite's JSON
/// parser inside the caller's transaction).
fn check_meta(c: &Connection, meta: &str) -> Result<()> {
    if meta.len() > MAX_FOREIGN_META {
        return Err(StoreError::Invalid("meta too large"));
    }
    let ok: bool = c.query_row(
        "SELECT CASE WHEN json_valid(?1) THEN json_type(?1) = 'object' ELSE 0 END",
        [meta],
        |r| r.get(0),
    )?;
    if ok {
        Ok(())
    } else {
        Err(StoreError::Invalid("meta is not a JSON object"))
    }
}

/// The reserved `meta` key of the §4 at-most-once mark (module doc).
const HANDOFF_KEY: &str = "handoff";

fn meta_key(k: &str) -> Result<()> {
    if k == HANDOFF_KEY {
        return Err(StoreError::Invalid(
            "meta.handoff is set only by foreign_handoff_begin / foreign_accepted / foreign_handoff_abort",
        ));
    }
    let ok = !k.is_empty()
        && k.len() <= MAX_META_KEY
        && k.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(StoreError::Invalid("meta key"))
    }
}

fn limit_i64(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

/// The conversation's `(account, kind)`, or [`StoreError::NotFound`].
fn conv_facts(c: &Connection, conv: i64) -> Result<(String, ForeignKind)> {
    let v: Option<(String, String)> = c
        .query_row(
            "SELECT account, kind FROM foreign_conversations WHERE id = ?1",
            [conv],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (account, kind) = v.ok_or(StoreError::NotFound)?;
    Ok((account, ForeignKind::from_col(&kind)?))
}

fn bump_conv(c: &Connection, conv: i64, local: i64, inbound: bool) -> Result<()> {
    c.execute(
        "UPDATE foreign_conversations
         SET last_seq = max(last_seq, ?2), unread = unread + ?3 WHERE id = ?1",
        params![conv, local, i64::from(inbound)],
    )?;
    Ok(())
}

/// Whether `row` is a §3 duplicate of a stored row (inside the batch's
/// transaction, so earlier rows of the same batch count).
fn is_duplicate(c: &Connection, row: &ForeignInsert) -> Result<bool> {
    let Some(native) = row.native_msg_id.as_deref() else {
        return Ok(false);
    };
    let hit: Option<i64> = if row.id_globally_unique {
        c.query_row(
            "SELECT 1 FROM foreign_messages
             WHERE conv = ?1 AND sender_native = ?2 AND native_msg_id = ?3 LIMIT 1",
            params![row.conv, row.sender_native, native],
            |r| r.get(0),
        )
        .optional()?
    } else {
        c.query_row(
            "SELECT 1 FROM (SELECT sender_native, native_msg_id, body FROM foreign_messages
                            WHERE conv = ?1 AND dir = 'in'
                            ORDER BY local_seq DESC LIMIT ?5)
             WHERE sender_native = ?2 AND native_msg_id = ?3 AND body = ?4 LIMIT 1",
            params![row.conv, row.sender_native, native, row.body, DEDUPE_WINDOW],
            |r| r.get(0),
        )
        .optional()?
    };
    Ok(hit.is_some())
}

fn check_send(s: &ForeignSend<'_>) -> Result<()> {
    body(s.text, "foreign send text too large")?;
    check_parts(s.part_k, s.part_n)?;
    match (s.group_id, s.edge_id, s.origin_msg) {
        (Some(_), None, None) | (None, Some(_), Some(_)) => Ok(()),
        _ => Err(StoreError::Invalid(
            "an own foreign row is user-composed (group) or a relay copy (edge + origin)",
        )),
    }
}

/// A relay copy queued earlier in the current [`Store::foreign_queue_sends`]
/// call: `(edge, origin)` with its `part_n` and the last `part_k` queued.
struct CopyParts {
    edge: i64,
    origin: i64,
    part_n: u32,
    last_k: u32,
}

/// Queue one own row inside the caller's transaction (validated by
/// [`check_send`] first). `copies` holds the copies queued earlier in this
/// transaction: only they may queue a further part of a copy whose origin
/// is already at or below `relayed_seq`, and that part must continue the
/// copy's numbering (same `part_n`, `part_k` = the previous + 1; module
/// doc). A copy's first part in a call is part 1.
fn queue_one(c: &Connection, s: &ForeignSend<'_>, copies: &mut Vec<CopyParts>) -> Result<i64> {
    let (account, _) = conv_facts(c, s.conv)?;
    if let Some(g) = s.group_id {
        let found: Option<i64> = c
            .query_row("SELECT 1 FROM send_groups WHERE id = ?1", [g], |r| r.get(0))
            .optional()?;
        found.ok_or(StoreError::NotFound)?;
    }
    if let (Some(edge), Some(origin)) = (s.edge_id, s.origin_msg) {
        let e = edge_in(c, edge)?.ok_or(StoreError::NotFound)?;
        if e.dst_conv != s.conv {
            return Err(StoreError::Invalid(
                "a relay copy goes to its edge's destination",
            ));
        }
        if !e.enabled || e.paused {
            return Err(StoreError::Invalid("edge is disabled or paused"));
        }
        let o: Option<(i64, String, i64)> = c
            .query_row(
                "SELECT conv, dir, local_seq FROM foreign_messages WHERE id = ?1",
                [origin],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (oconv, odir, oseq) = o.ok_or(StoreError::NotFound)?;
        if oconv != e.src_conv || ForeignDir::from_col(&odir)? != ForeignDir::In {
            return Err(StoreError::Invalid(
                "a relay origin is inbound on the edge's source",
            ));
        }
        let oseq_u = to_u64(oseq, "foreign_messages.local_seq")?;
        if oseq_u <= e.cursor_seq {
            return Err(StoreError::Invalid("history is never relayed"));
        }
        // §8 guard 6: one copy per origin per edge. At or below the mark
        // only the further parts of the copy this transaction queued, in
        // order (B1-bounce F9).
        match copies
            .iter_mut()
            .find(|p| p.edge == edge && p.origin == origin)
        {
            Some(p) => {
                if s.part_n != p.part_n || Some(s.part_k) != p.last_k.checked_add(1) {
                    return Err(StoreError::Invalid(
                        "the parts of one relay copy are numbered 1..n in order in one call",
                    ));
                }
                p.last_k = s.part_k;
            }
            None => {
                if oseq_u <= e.relayed_seq {
                    return Err(StoreError::Invalid(
                        "this origin was already relayed (or shed) on this edge",
                    ));
                }
                if s.part_k != 1 {
                    return Err(StoreError::Invalid(
                        "a relay copy starts at part 1 in the call that queues it",
                    ));
                }
                copies.push(CopyParts {
                    edge,
                    origin,
                    part_n: s.part_n,
                    last_k: 1,
                });
            }
        }
        // §8: the copy and the high-water mark commit together.
        c.execute(
            "UPDATE relay_edges SET relayed_seq = max(relayed_seq, ?2) WHERE id = ?1",
            params![edge, oseq],
        )?;
    }
    let local = to_i64(next_local_seq(c)?, "local_seq")?;
    c.execute(
        "INSERT INTO foreign_messages (conv, dir, sender_native, auth, body, local_seq, state,
                                       group_id, part_k, part_n, origin_msg, edge_id)
         VALUES (?1, 'own', ?2, 'none', ?3, ?4, 'queued', ?5, ?6, ?7, ?8, ?9)",
        params![
            s.conv,
            account,
            s.text,
            local,
            s.group_id,
            i64::from(s.part_k),
            i64::from(s.part_n),
            s.origin_msg,
            s.edge_id,
        ],
    )?;
    let id = c.last_insert_rowid();
    bump_conv(c, s.conv, local, false)?;
    Ok(id)
}

/// Delete the edge's copies never handed to a carrier (`queued`, no
/// pending handoff); a copy with a pending handoff may have been sent and
/// is kept (prune treats it as handed, step 2a).
fn drop_backlog(c: &Connection, edge: i64) -> Result<()> {
    c.execute(
        "DELETE FROM foreign_messages
         WHERE edge_id = ?1 AND dir = 'own' AND state = 'queued'
           AND json_extract(meta, '$.handoff') IS NOT 'pending'",
        [edge],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Store methods
// ---------------------------------------------------------------------------

impl Store {
    // -- conversations ------------------------------------------------------

    /// Insert or find a conversation by `(network, account, kind,
    /// native_id)` (§3 conversation identity) and return its id. A `title`
    /// replaces the stored one; `None` keeps it.
    pub fn foreign_conversation_upsert(
        &mut self,
        network: &str,
        account: &str,
        kind: ForeignKind,
        native_id: &str,
        title: Option<&str>,
    ) -> Result<i64> {
        self.foreign_conversation_upsert_changed(network, account, kind, native_id, title)
            .map(|(id, _)| id)
    }

    /// [`Store::foreign_conversation_upsert`], also telling whether
    /// anything changed: `true` for a new row or a differing `title`,
    /// `false` when the row exists and the requested `title` is `None` or
    /// equal (nothing is written then). The fabric's §10 `Conversation`
    /// event follows this answer, not its cache.
    pub fn foreign_conversation_upsert_changed(
        &mut self,
        network: &str,
        account: &str,
        kind: ForeignKind,
        native_id: &str,
        title: Option<&str>,
    ) -> Result<(i64, bool)> {
        network_id(network)?;
        field(account, "account too long")?;
        nonempty_field(native_id, "conversation native id")?;
        field_opt(title, "title too long")?;
        self.with_tx(Durability::Normal, |tx| {
            let found: Option<(i64, Option<String>)> = tx
                .query_row(
                    "SELECT id, title FROM foreign_conversations
                     WHERE network = ?1 AND account = ?2 AND kind = ?3 AND native_id = ?4",
                    params![network, account, kind.as_str(), native_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((id, stored)) = &found {
                if title.is_none() || title == stored.as_deref() {
                    return Ok((*id, false));
                }
            }
            let id = tx.query_row(
                "INSERT INTO foreign_conversations (network, account, kind, native_id, title)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (network, account, kind, native_id)
                 DO UPDATE SET title = coalesce(excluded.title, title)
                 RETURNING id",
                params![network, account, kind.as_str(), native_id, title],
                |r| r.get(0),
            )?;
            Ok((id, true))
        })
    }

    /// One conversation with its counts.
    pub fn foreign_conversation(&self, id: i64) -> Result<Option<ForeignConversationRow>> {
        let raw = self
            .conn
            .query_row(
                &format!("SELECT {FC_COLS} FROM foreign_conversations c WHERE c.id = ?1"),
                [id],
                fc_raw,
            )
            .optional()?;
        raw.map(fc_from_raw).transpose()
    }

    /// The conversations of `network` (every network when `None`), most
    /// recent first.
    pub fn foreign_conversations(
        &self,
        network: Option<&str>,
    ) -> Result<Vec<ForeignConversationRow>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {FC_COLS} FROM foreign_conversations c
             WHERE ?1 IS NULL OR c.network = ?1
             ORDER BY c.last_seq DESC, c.id DESC"
        ))?;
        let rows = st.query_map([network], fc_raw)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(fc_from_raw(r?)?);
        }
        Ok(out)
    }

    /// Pin or unpin a conversation: a pinned conversation's rows are never
    /// pruned (§3 step 4).
    pub fn foreign_conversation_set_pinned(&mut self, id: i64, pinned: bool) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE foreign_conversations SET pinned = ?2 WHERE id = ?1",
            params![id, pinned],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    /// Clear a conversation's unread counter.
    pub fn foreign_conversation_mark_read(&mut self, id: i64) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE foreign_conversations SET unread = 0 WHERE id = ?1",
            [id],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    // -- inbound ------------------------------------------------------------

    /// §3 ingest write: insert up to [`MAX_FOREIGN_BATCH`] inbound rows in
    /// **one** transaction, each drawing `local_seq` from the store-wide
    /// counter (contiguous across the inserted rows). Returns, per input,
    /// the new row id or `None` for a duplicate (§3 dedupe). Any invalid
    /// row (size, unknown conversation, `meta` not a JSON object) writes
    /// nothing.
    pub fn foreign_insert_batch(&mut self, rows: Vec<ForeignInsert>) -> Result<Vec<Option<i64>>> {
        if rows.len() > MAX_FOREIGN_BATCH {
            return Err(StoreError::Invalid("foreign batch over 256 rows"));
        }
        for r in &rows {
            nonempty_field(&r.sender_native, "sender native id")?;
            field_opt(r.sender_name.as_deref(), "sender name too long")?;
            field_opt(r.native_msg_id.as_deref(), "native message id too long")?;
            body(&r.body, "foreign body over 64 KiB")?;
        }
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        self.with_tx(Durability::Normal, |tx| {
            let mut out = Vec::with_capacity(rows.len());
            for r in &rows {
                conv_facts(tx, r.conv)?;
                let meta = r.meta.as_deref().unwrap_or("{}");
                check_meta(tx, meta)?;
                if is_duplicate(tx, r)? {
                    out.push(None);
                    continue;
                }
                let local = to_i64(next_local_seq(tx)?, "local_seq")?;
                tx.execute(
                    "INSERT INTO foreign_messages (conv, dir, sender_native, sender_name, auth,
                                                   native_msg_id, body, local_seq, claim_ts,
                                                   state, meta)
                     VALUES (?1, 'in', ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'delivered', ?9)",
                    params![
                        r.conv,
                        r.sender_native,
                        r.sender_name,
                        r.auth.as_str(),
                        r.native_msg_id,
                        r.body,
                        local,
                        r.claim_ts,
                        meta,
                    ],
                )?;
                out.push(Some(tx.last_insert_rowid()));
                bump_conv(tx, r.conv, local, true)?;
            }
            Ok(out)
        })
    }

    // -- own rows -----------------------------------------------------------

    /// Queue one own row (`dir own`, `queued`, `local_seq` drawn). A relay
    /// copy must go to its edge's destination on a live edge, from an
    /// inbound origin on the edge's source newer than `cursor_seq` and not
    /// yet relayed on the edge (newer than `relayed_seq`); the edge's
    /// `relayed_seq` advances past the origin in the same transaction (§8).
    pub fn foreign_queue_send(&mut self, send: &ForeignSend<'_>) -> Result<i64> {
        self.foreign_queue_sends(std::slice::from_ref(send))?
            .into_iter()
            .next()
            .ok_or(StoreError::Corrupt("queued row id"))
    }

    /// Queue up to [`MAX_FOREIGN_BATCH`] own rows in one transaction (every
    /// part of a split copy together, so a crash never leaves half a copy;
    /// the parts after the first are the only rows whose origin may already
    /// be at `relayed_seq`, and each `part_k` at most once).
    ///
    /// The parts of one copy go `1..=part_n` in order with one `part_n`, and
    /// the copy is complete within the call; the copies of one edge go in
    /// **ascending origin `local_seq` order** (a descending batch is
    /// refused as a whole: [`StoreError::Invalid`], nothing written). See
    /// the module doc, "One copy per origin per edge".
    pub fn foreign_queue_sends(&mut self, sends: &[ForeignSend<'_>]) -> Result<Vec<i64>> {
        if sends.is_empty() {
            return Err(StoreError::Invalid("nothing to queue"));
        }
        if sends.len() > MAX_FOREIGN_BATCH {
            return Err(StoreError::Invalid("foreign batch over 256 rows"));
        }
        for s in sends {
            check_send(s)?;
        }
        self.with_tx(Durability::Full, |tx| {
            let mut copies = Vec::new();
            let ids = sends
                .iter()
                .map(|s| queue_one(tx, s, &mut copies))
                .collect::<Result<Vec<i64>>>()?;
            if copies.iter().any(|p| p.last_k != p.part_n) {
                // the missing parts could never be queued later (the mark
                // moved past the origin): fail closed
                return Err(StoreError::Invalid(
                    "a split relay copy is queued with all its parts in one call",
                ));
            }
            Ok(ids)
        })
    }

    /// §9: create a send group and queue its foreign parts in one
    /// transaction (validation is atomic: any bad part queues nothing).
    /// `targets` is the group's JSON; `parts` may be empty (111-only
    /// targets). Returns the group id and the row ids in `parts` order.
    pub fn send_group_create(
        &mut self,
        text: &str,
        targets: &str,
        parts: &[GroupPart<'_>],
    ) -> Result<(i64, Vec<i64>)> {
        body(text, "send text over 64 KiB")?;
        if targets.len() > MAX_SEND_TARGETS {
            return Err(StoreError::Invalid("send targets too large"));
        }
        if parts.len() > MAX_FOREIGN_BATCH {
            return Err(StoreError::Invalid("foreign batch over 256 rows"));
        }
        for p in parts {
            body(p.text, "foreign send text too large")?;
            check_parts(p.part_k, p.part_n)?;
        }
        self.with_tx(Durability::Full, |tx| {
            let ok: bool = tx.query_row("SELECT coalesce(json_valid(?1), 0)", [targets], |r| {
                r.get(0)
            })?;
            if !ok {
                return Err(StoreError::Invalid("send targets are not JSON"));
            }
            let local = to_i64(next_local_seq(tx)?, "local_seq")?;
            tx.execute(
                "INSERT INTO send_groups (local_seq, text, targets) VALUES (?1, ?2, ?3)",
                params![local, text, targets],
            )?;
            let group = tx.last_insert_rowid();
            let mut ids = Vec::with_capacity(parts.len());
            let mut copies = Vec::new();
            for p in parts {
                ids.push(queue_one(
                    tx,
                    &ForeignSend {
                        conv: p.conv,
                        text: p.text,
                        group_id: Some(group),
                        part_k: p.part_k,
                        part_n: p.part_n,
                        origin_msg: None,
                        edge_id: None,
                    },
                    &mut copies,
                )?);
            }
            Ok((group, ids))
        })
    }

    /// One send group.
    pub fn send_group(&self, id: i64) -> Result<Option<SendGroupRow>> {
        let v: Option<(i64, i64, String, String)> = self
            .conn
            .query_row(
                "SELECT id, local_seq, text, targets FROM send_groups WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        v.map(|(id, local, text, targets)| {
            Ok(SendGroupRow {
                id,
                local_seq: to_u64(local, "send_groups.local_seq")?,
                text,
                targets,
            })
        })
        .transpose()
    }

    /// The foreign rows of a send group (every part of every target), in
    /// `local_seq` order.
    pub fn send_group_rows(&self, group_id: i64) -> Result<Vec<ForeignMessageRow>> {
        collect_fm(
            &self.conn,
            &format!(
                "SELECT {FM_COLS} FROM foreign_messages m WHERE m.group_id = ?1
                 ORDER BY m.local_seq, m.id"
            ),
            [group_id],
        )
    }

    /// §4 monotone state update of an own row. A move
    /// [`ForeignState::may_become`] refuses is ignored
    /// ([`StateChange::Unchanged`]): `delivered` never regresses, `failed`
    /// is left only by `delivered`, a `relayed` row never becomes `failed`
    /// (it was transmitted). At the same state only a differing
    /// evidence / reason is written. An inbound row has no send state
    /// ([`StoreError::Invalid`]).
    pub fn foreign_set_state(
        &mut self,
        id: i64,
        state: ForeignState,
        evidence: Option<&str>,
        reason: Option<&str>,
    ) -> Result<StateChange> {
        field_opt(evidence, "evidence too long")?;
        field_opt(reason, "reason code too long")?;
        self.with_tx(Durability::Normal, |tx| {
            let cur: Option<(String, String, Option<String>, Option<String>)> = tx
                .query_row(
                    "SELECT dir, state, evidence, reason_code FROM foreign_messages WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            let (dir, cur, cur_ev, cur_reason) = cur.ok_or(StoreError::NotFound)?;
            if ForeignDir::from_col(&dir)? != ForeignDir::Own {
                return Err(StoreError::Invalid(
                    "an inbound foreign row has no send state",
                ));
            }
            let cur = ForeignState::from_col(&cur)?;
            if !cur.may_become(state)
                || (state == cur
                    && evidence == cur_ev.as_deref()
                    && reason == cur_reason.as_deref())
            {
                return Ok(StateChange::Unchanged);
            }
            tx.execute(
                "UPDATE foreign_messages SET state = ?2, evidence = ?3, reason_code = ?4
                 WHERE id = ?1",
                params![id, state.as_str(), evidence, reason],
            )?;
            Ok(StateChange::Changed)
        })
    }

    /// Set (`Some`) or remove (`None`) one `meta` field (`[a-z_]`, ≤ 32
    /// bytes). Returns whether the row exists. The reserved key `handoff`
    /// is [`StoreError::Invalid`] here: the marker has its own durable calls
    /// ([`Store::foreign_handoff_begin`], [`Store::foreign_accepted`],
    /// [`Store::foreign_handoff_abort`]).
    pub fn foreign_set_meta(
        &mut self,
        id: i64,
        key: &str,
        value: Option<MetaValue<'_>>,
    ) -> Result<bool> {
        meta_key(key)?;
        if let Some(MetaValue::Str(s)) = value {
            field(s, "meta value too long")?;
        }
        let path = format!("$.{key}");
        self.with_tx(Durability::Normal, |tx| {
            let n = match value {
                None => tx.execute(
                    "UPDATE foreign_messages SET meta = json_remove(meta, ?2) WHERE id = ?1",
                    params![id, path],
                )?,
                Some(MetaValue::Str(s)) => tx.execute(
                    "UPDATE foreign_messages SET meta = json_set(meta, ?2, ?3) WHERE id = ?1",
                    params![id, path, s],
                )?,
                Some(MetaValue::Int(i)) => tx.execute(
                    "UPDATE foreign_messages SET meta = json_set(meta, ?2, ?3) WHERE id = ?1",
                    params![id, path, i],
                )?,
                Some(MetaValue::Bool(b)) => tx.execute(
                    "UPDATE foreign_messages SET meta = json_set(meta, ?2, json(?3)) WHERE id = ?1",
                    params![id, path, if b { "true" } else { "false" }],
                )?,
            };
            if n == 1 {
                let len: i64 = tx.query_row(
                    "SELECT octet_length(meta) FROM foreign_messages WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                )?;
                if to_u64(len, "meta length")? > MAX_FOREIGN_META as u64 {
                    return Err(StoreError::Invalid("meta too large"));
                }
            }
            Ok(n == 1)
        })
    }

    /// §4 at most once: record `meta.handoff = "pending"` durably (FULL)
    /// before the row is handed to a carrier. Only an own `queued` row.
    pub fn foreign_handoff_begin(&mut self, id: i64) -> Result<()> {
        self.with_tx(Durability::Full, |tx| {
            let n = tx.execute(
                "UPDATE foreign_messages SET meta = json_set(meta, '$.handoff', 'pending')
                 WHERE id = ?1 AND dir = 'own' AND state = 'queued'",
                [id],
            )?;
            if n == 0 {
                return Err(StoreError::Invalid("handoff of a row that is not queued"));
            }
            Ok(())
        })
    }

    /// §4 `Accepted { native_id }`: the carrier took the row — record its
    /// native id (own-echo suppression, §8 guard 2) and clear the handoff
    /// marker, in one transaction. FULL: the recorded native id is own-echo
    /// evidence that must survive a crash.
    pub fn foreign_accepted(&mut self, id: i64, native_msg_id: &str) -> Result<()> {
        nonempty_field(native_msg_id, "native message id")?;
        self.with_tx(Durability::Full, |tx| {
            let n = tx.execute(
                "UPDATE foreign_messages
                 SET native_msg_id = ?2, meta = json_remove(meta, '$.handoff')
                 WHERE id = ?1 AND dir = 'own'",
                params![id, native_msg_id],
            )?;
            if n == 0 {
                return Err(StoreError::NotFound);
            }
            Ok(())
        })
    }

    /// §4 at most once, the pre-air refusal: withdraw the `handoff =
    /// "pending"` mark of a `queued` own row, so [`Store::foreign_queued`]
    /// offers it again (and it is cancellable and droppable again). The
    /// caller uses it **only** when the carrier reported a refusal before
    /// air (`Retry::Later`: duty cycle, rate limit, a full queue) and no
    /// `Accepted` for the row — nothing was transmitted, so a later retry
    /// is still the one send of the user act. After a crash the mark stays
    /// (never re-sent automatically). An unknown id is
    /// [`StoreError::NotFound`]; an inbound row, a row past `queued` or a
    /// row without a pending mark is [`StoreError::Invalid`]. FULL, like
    /// the mark itself.
    pub fn foreign_handoff_abort(&mut self, id: i64) -> Result<()> {
        self.with_tx(Durability::Full, |tx| {
            let row: Option<(String, String, bool)> = tx
                .query_row(
                    "SELECT dir, state,
                            coalesce(json_extract(meta, '$.handoff') = 'pending', 0)
                     FROM foreign_messages WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let (dir, state, pending) = row.ok_or(StoreError::NotFound)?;
            if ForeignDir::from_col(&dir)? != ForeignDir::Own
                || ForeignState::from_col(&state)? != ForeignState::Queued
                || !pending
            {
                return Err(StoreError::Invalid(
                    "only a queued own row with a pending handoff is aborted",
                ));
            }
            tx.execute(
                "UPDATE foreign_messages SET meta = json_remove(meta, '$.handoff') WHERE id = ?1",
                [id],
            )?;
            Ok(())
        })
    }

    /// §9 cancel: delete a user-composed own `queued` row (`group_id` set)
    /// with no pending handoff (nothing of it was handed to a carrier). One
    /// row per call, so a split target's unsent parts are cancelled one by
    /// one and its sent parts stay ("cancel drops unsent parts only"). An
    /// unknown id is [`StoreError::NotFound`]; an inbound row, a relay copy
    /// (copies leave only by shed, disable or delete — module doc), a row
    /// past `queued` or one with `meta.handoff = "pending"` (may have been
    /// sent) is [`StoreError::Invalid`]. FULL: a cancelled row must not come
    /// back after a crash and be sent.
    pub fn foreign_cancel(&mut self, id: i64) -> Result<()> {
        self.with_tx(Durability::Full, |tx| {
            let row: Option<(String, String, bool, Option<i64>)> = tx
                .query_row(
                    "SELECT dir, state,
                            coalesce(json_extract(meta, '$.handoff') = 'pending', 0),
                            group_id
                     FROM foreign_messages WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            let (dir, state, pending, group) = row.ok_or(StoreError::NotFound)?;
            if ForeignDir::from_col(&dir)? != ForeignDir::Own {
                return Err(StoreError::Invalid(
                    "an inbound foreign row is not cancelled",
                ));
            }
            if group.is_none() {
                return Err(StoreError::Invalid(
                    "a relay copy is not cancelled (it leaves by shed, disable or delete)",
                ));
            }
            if ForeignState::from_col(&state)? != ForeignState::Queued || pending {
                return Err(StoreError::Invalid(
                    "only a queued row with nothing handed is cancelled",
                ));
            }
            tx.execute("DELETE FROM foreign_messages WHERE id = ?1", [id])?;
            Ok(())
        })
    }

    // -- reads --------------------------------------------------------------

    /// One foreign row.
    pub fn foreign_message(&self, id: i64) -> Result<Option<ForeignMessageRow>> {
        Ok(collect_fm(
            &self.conn,
            &format!("SELECT {FM_COLS} FROM foreign_messages m WHERE m.id = ?1"),
            [id],
        )?
        .pop())
    }

    /// Rows of `conv` with `local_seq > since_seq`, oldest first, at most
    /// `limit` (the `?since_seq` read, §10).
    pub fn foreign_messages(
        &self,
        conv: i64,
        since_seq: u64,
        limit: usize,
    ) -> Result<Vec<ForeignMessageRow>> {
        collect_fm(
            &self.conn,
            &format!(
                "SELECT {FM_COLS} FROM foreign_messages m
                 WHERE m.conv = ?1 AND m.local_seq > ?2 ORDER BY m.local_seq, m.id LIMIT ?3"
            ),
            params![conv, to_i64(since_seq, "since_seq")?, limit_i64(limit)],
        )
    }

    /// The newest `limit` rows of `conv`, returned oldest first (a thread
    /// page never loads a whole history).
    pub fn foreign_thread_page(&self, conv: i64, limit: usize) -> Result<Vec<ForeignMessageRow>> {
        let mut v = collect_fm(
            &self.conn,
            &format!(
                "SELECT {FM_COLS} FROM foreign_messages m
                 WHERE m.conv = ?1 ORDER BY m.local_seq DESC, m.id DESC LIMIT ?2"
            ),
            params![conv, limit_i64(limit)],
        )?;
        v.reverse();
        Ok(v)
    }

    /// What an adapter tick may hand to `network` (§4 queue order):
    /// own `queued` rows without a pending handoff, user-composed rows
    /// (`group_id` set) ahead of relay copies, then `local_seq`. A relay
    /// copy of an edge that is disabled, paused or gone is held back.
    pub fn foreign_queued(&self, network: &str, limit: usize) -> Result<Vec<ForeignMessageRow>> {
        collect_fm(
            &self.conn,
            &format!(
                "SELECT {FM_COLS} FROM foreign_messages m
                 JOIN foreign_conversations c ON c.id = m.conv
                 WHERE m.dir = 'own' AND m.state = 'queued' AND c.network = ?1
                   AND json_extract(m.meta, '$.handoff') IS NOT 'pending'
                   AND (m.edge_id IS NULL OR EXISTS (
                        SELECT 1 FROM relay_edges e
                        WHERE e.id = m.edge_id AND e.enabled = 1 AND e.paused = 0))
                 ORDER BY (m.group_id IS NULL), m.local_seq, m.id
                 LIMIT ?2"
            ),
            params![network, limit_i64(limit)],
        )
    }

    // -- relay edges --------------------------------------------------------

    /// Create an enabled edge (§8) with the recorded consent; `cursor_seq`
    /// is the current `local_seq`, so nothing already stored is relayed.
    /// Both endpoints must be `channel` or `group` (a DM is refused before
    /// the table's `CHECK`), distinct, and not already joined by an edge.
    pub fn edge_create(&mut self, e: &EdgeNew<'_>) -> Result<i64> {
        nonempty_field(e.consent.consent_hash, "consent hash")?;
        nonempty_field(e.consent.enabled_by, "enabled_by")?;
        if e.src_conv == e.dst_conv {
            return Err(StoreError::Invalid("an edge joins two conversations"));
        }
        self.with_tx(Durability::Full, |tx| {
            let (_, src_kind) = conv_facts(tx, e.src_conv)?;
            let (_, dst_kind) = conv_facts(tx, e.dst_conv)?;
            if src_kind == ForeignKind::Dm || dst_kind == ForeignKind::Dm {
                return Err(StoreError::Invalid("a DM is never an edge endpoint"));
            }
            let exists: Option<i64> = tx
                .query_row(
                    "SELECT id FROM relay_edges WHERE src_conv = ?1 AND dst_conv = ?2",
                    params![e.src_conv, e.dst_conv],
                    |r| r.get(0),
                )
                .optional()?;
            if exists.is_some() {
                return Err(StoreError::Invalid(
                    "an edge between these conversations exists",
                ));
            }
            let cursor = to_i64(current_local_seq(tx)?, "local_seq")?;
            tx.execute(
                "INSERT INTO relay_edges (src_conv, dst_conv, src_kind, dst_kind, cursor_seq,
                                          honour_ok_to_mqtt, oversize, consent_hash, enabled_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    e.src_conv,
                    e.dst_conv,
                    src_kind.as_str(),
                    dst_kind.as_str(),
                    cursor,
                    e.honour_ok_to_mqtt,
                    e.oversize.as_str(),
                    e.consent.consent_hash,
                    e.consent.enabled_by,
                ],
            )?;
            Ok(tx.last_insert_rowid())
        })
    }

    /// One edge.
    pub fn edge(&self, id: i64) -> Result<Option<EdgeRow>> {
        edge_in(&self.conn, id)
    }

    /// Every edge, by id.
    pub fn edge_list(&self) -> Result<Vec<EdgeRow>> {
        let mut st = self
            .conn
            .prepare(&format!("SELECT {EDGE_COLS} FROM relay_edges ORDER BY id"))?;
        let rows = st.query_map([], edge_raw)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(edge_from_raw(r?)?);
        }
        Ok(out)
    }

    /// Enable (with a new recorded consent; `cursor_seq` moves to the
    /// current `local_seq`, so what arrived while off is history) or
    /// disable an edge. Disabling **drops** the edge's never-handed copies
    /// (`queued`, no pending handoff; module doc): a disabled edge has no
    /// backlog. Returns whether it changed.
    pub fn edge_set_enabled(
        &mut self,
        id: i64,
        enabled: bool,
        consent: Option<&EdgeConsent<'_>>,
    ) -> Result<bool> {
        if enabled {
            let c = consent.ok_or(StoreError::Invalid("enabling an edge records the consent"))?;
            nonempty_field(c.consent_hash, "consent hash")?;
            nonempty_field(c.enabled_by, "enabled_by")?;
        }
        self.with_tx(Durability::Full, |tx| {
            let e = edge_in(tx, id)?.ok_or(StoreError::NotFound)?;
            if e.enabled == enabled {
                return Ok(false);
            }
            match consent.filter(|_| enabled) {
                Some(c) => {
                    let cursor = to_i64(current_local_seq(tx)?, "local_seq")?;
                    tx.execute(
                        "UPDATE relay_edges SET enabled = 1, paused = 0, cursor_seq = ?2,
                                consent_hash = ?3, enabled_by = ?4 WHERE id = ?1",
                        params![id, cursor, c.consent_hash, c.enabled_by],
                    )?;
                }
                None => {
                    tx.execute("UPDATE relay_edges SET enabled = 0 WHERE id = ?1", [id])?;
                    drop_backlog(tx, id)?;
                }
            }
            Ok(true)
        })
    }

    /// Pause (`consent_paused`, §8: consent facts changed) or resume an
    /// edge. Resuming records the new human confirmation. Returns whether it
    /// changed.
    pub fn edge_set_paused(
        &mut self,
        id: i64,
        paused: bool,
        consent: Option<&EdgeConsent<'_>>,
    ) -> Result<bool> {
        if !paused {
            let c = consent.ok_or(StoreError::Invalid("resuming an edge records the consent"))?;
            nonempty_field(c.consent_hash, "consent hash")?;
            nonempty_field(c.enabled_by, "enabled_by")?;
        }
        self.with_tx(Durability::Full, |tx| {
            let e = edge_in(tx, id)?.ok_or(StoreError::NotFound)?;
            if e.paused == paused {
                return Ok(false);
            }
            match consent.filter(|_| !paused) {
                Some(c) => {
                    tx.execute(
                        "UPDATE relay_edges SET paused = 0, consent_hash = ?2, enabled_by = ?3
                         WHERE id = ?1",
                        params![id, c.consent_hash, c.enabled_by],
                    )?;
                }
                None => {
                    tx.execute("UPDATE relay_edges SET paused = 1 WHERE id = ?1", [id])?;
                }
            }
            Ok(true)
        })
    }

    /// Advance `relayed_seq` to at least `seq` (never backwards; FULL: the
    /// mark is what keeps history unrelayable). Returns the stored value.
    pub fn edge_advance_relayed_seq(&mut self, id: i64, seq: u64) -> Result<u64> {
        let seq = to_i64(seq, "relayed_seq")?;
        self.with_tx(Durability::Full, |tx| {
            let v: Option<i64> = tx
                .query_row(
                    "UPDATE relay_edges SET relayed_seq = max(relayed_seq, ?2) WHERE id = ?1
                     RETURNING relayed_seq",
                    params![id, seq],
                    |r| r.get(0),
                )
                .optional()?;
            to_u64(v.ok_or(StoreError::NotFound)?, "relay_edges.relayed_seq")
        })
    }

    /// Add `n` to one of the edge's counters.
    pub fn edge_count(&mut self, id: i64, counter: EdgeCounter, n: u64) -> Result<()> {
        let n = to_i64(n, "counter")?;
        let col = counter.col();
        let changed = self.conn.execute(
            &format!("UPDATE relay_edges SET {col} = {col} + ?2 WHERE id = ?1"),
            params![id, n],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    /// Delete an edge, its loop-guard keys and its copies still `queued`
    /// without a pending handoff (never handed to a carrier). Copies already
    /// handed keep their `edge_id`/`origin_msg` (never edge inputs).
    pub fn edge_delete(&mut self, id: i64) -> Result<bool> {
        self.with_tx(Durability::Full, |tx| {
            let n = tx.execute("DELETE FROM relay_edges WHERE id = ?1", [id])?;
            if n == 0 {
                return Ok(false);
            }
            drop_backlog(tx, id)?;
            Ok(true)
        })
    }

    /// §8 "what it relays": inbound rows of the edge's source with
    /// `local_seq > max(cursor_seq, relayed_seq)` that are not relayed
    /// copies, oldest first, at most `limit`. Whether a text parses as
    /// already relayed (the marker) is the bridge core's check.
    pub fn edge_inputs(&self, id: i64, limit: usize) -> Result<Vec<ForeignMessageRow>> {
        let e = self.edge(id)?.ok_or(StoreError::NotFound)?;
        let floor = to_i64(e.cursor_seq.max(e.relayed_seq), "edge floor")?;
        collect_fm(
            &self.conn,
            &format!(
                "SELECT {FM_COLS} FROM foreign_messages m
                 WHERE m.conv = ?1 AND m.local_seq > ?2 AND m.dir = 'in'
                   AND m.origin_msg IS NULL AND m.edge_id IS NULL
                 ORDER BY m.local_seq, m.id LIMIT ?3"
            ),
            params![e.src_conv, floor, limit_i64(limit)],
        )
    }

    // -- loop-guard keys ----------------------------------------------------

    /// Record a loop-guard key on `edge` (§3 `foreign_seen`, §8 guard 6);
    /// beyond [`FOREIGN_SEEN_CAP`] keys on the edge the oldest (by
    /// `local_seq`) are evicted. A repeated key keeps the newer `local_seq`.
    pub fn foreign_seen_insert(
        &mut self,
        edge: i64,
        conv: i64,
        sender_native: &str,
        native_msg_id: &str,
        local_seq: u64,
    ) -> Result<()> {
        field(sender_native, "sender native id")?;
        nonempty_field(native_msg_id, "native message id")?;
        let local = to_i64(local_seq, "local_seq")?;
        self.with_tx(Durability::Normal, |tx| {
            edge_in(tx, edge)?.ok_or(StoreError::NotFound)?;
            tx.execute(
                "INSERT INTO foreign_seen (edge, conv, sender_native, native_msg_id, local_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (edge, conv, sender_native, native_msg_id)
                 DO UPDATE SET local_seq = max(local_seq, excluded.local_seq)",
                params![edge, conv, sender_native, native_msg_id, local],
            )?;
            let n: i64 = tx.query_row(
                "SELECT count(*) FROM foreign_seen WHERE edge = ?1",
                [edge],
                |r| r.get(0),
            )?;
            let over = to_u64(n, "count")?.saturating_sub(FOREIGN_SEEN_CAP);
            if over > 0 {
                tx.execute(
                    "DELETE FROM foreign_seen
                     WHERE (edge, conv, sender_native, native_msg_id) IN (
                         SELECT edge, conv, sender_native, native_msg_id FROM foreign_seen
                         WHERE edge = ?1
                         ORDER BY local_seq, conv, sender_native, native_msg_id LIMIT ?2)",
                    params![edge, to_i64(over, "count")?],
                )?;
            }
            Ok(())
        })
    }

    /// Whether a loop-guard key is held on `edge` (any edge when `None`).
    pub fn foreign_seen_contains(
        &self,
        edge: Option<i64>,
        conv: i64,
        sender_native: &str,
        native_msg_id: &str,
    ) -> Result<bool> {
        let hit: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM foreign_seen
                 WHERE (?1 IS NULL OR edge = ?1) AND conv = ?2
                   AND sender_native = ?3 AND native_msg_id = ?4 LIMIT 1",
                params![edge, conv, sender_native, native_msg_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(hit.is_some())
    }

    /// Loop-guard keys held on `edge` (gauge, tests).
    pub fn foreign_seen_count(&self, edge: i64) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM foreign_seen WHERE edge = ?1",
            [edge],
            |r| r.get(0),
        )?;
        to_u64(n, "count")
    }
}
