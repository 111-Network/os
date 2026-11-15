//! §10.3 — schema v1 and migrations via `PRAGMA user_version`.
//!
//! Each migration runs in one transaction with `synchronous = FULL` and sets
//! `user_version` to its index + 1. A store whose `user_version` is newer
//! than this build is refused (fail closed, never downgraded). Tables are
//! `STRICT` so a wrong type is an error, not a coercion.
//!
//! Column notes (spec names kept):
//! - `messages.dir` 0 own · 1 in · 2 transit; `state` 0 queued · 1 sent ·
//!   2 relayed · 3 delivered (§9 states); `class` = [`super::Class`].
//! - `tx_state` (peer, stream) → (epoch, next_n) (§4: never a global counter).
//! - `rx_state` (peer, stream, epoch) → prefix + ranges + partials (§4);
//!   `rx_floor` (peer, stream) → the newest epoch ever dropped, so an evicted
//!   epoch can never be re-opened and redelivered ("below a prefix nothing is
//!   ever forgotten").
//! - `rx_frags` holds the bytes of every fragment a partial bitmap records,
//!   written in the **same** FULL transaction as the bit (a SACKed fragment
//!   is never resent, so a bit without its bytes would lose the message).
//!   Bounded by the Tier-1 cap (≤ 16 fragments × 65,535 B per message) and
//!   by the partial bound per epoch; rows go with their partial (delivery,
//!   oldest-first eviction, epoch eviction). A rowid table: rows are large.
//! - `outbox` one row per fragment: sealed bytes awaiting a path, the
//!   outstanding flag (outstanding count = rows set) and the acked flag
//!   (SACK bitmap = rows set).
//! - `custody.custody_seq` is `AUTOINCREMENT`: the hub's local insertion
//!   sequence, never reused (cursors compare against it, §9).
//! - `blobs` adds the pruning facts §10.4 needs (`disk_bytes`, `class`,
//!   `local_seq`, `pinned`, `refetchable`, `superseded`, `present`). A public
//!   object never carries an `object_key`; a private one may lack it (a relay
//!   or hub holds the ciphertext keyless, §10.3 / §11).
//! - v2 (F1 bounce): `pending_node_ids` — §7.2 (2) "the node **stores** the
//!   NodeId": a contact added by `node1…` only, waiting for its card (no key
//!   material yet, so it cannot be a `contacts` row). A row leaves when the
//!   card is pinned or the user forgets the contact; never pruned (§10.4 (4):
//!   contacts are never pruned). Read marks stay in memory in v1 (recorded in
//!   the fabric module doc).
//! - v3 (Layer 6 §3, D25 F1; additive amendment of §10.3): the foreign
//!   message model — `foreign_conversations`, `foreign_messages` (one row per
//!   part of a split send), `relay_edges`, the bounded loop-guard keys
//!   `foreign_seen` and `send_groups`. Foreign rows are never NodeId-keyed
//!   (no pseudo-NodeIds, D25) and draw `local_seq` from the one store-wide
//!   counter. Enumerated columns are `CHECK`ed text (`kind`, `dir`, `state`,
//!   `auth`, `oversize`); a DM endpoint is unrepresentable in `relay_edges`
//!   (`src_kind`/`dst_kind` are `channel` or `group`). `meta` and
//!   `send_groups.targets` are JSON text (`json_valid`). Typed access:
//!   [`super::foreign`].
//! - v4 (Layer 7 §5.3 amendment 5, §5.1, §5.8; additive amendment of
//!   §10.3): `custody.fwd` (0 member · 1 from a member · 2 from a leaf hub ·
//!   3 from the carrier) and `custody.fwd_from` (the 32 B NodeId of the
//!   member or pair that put a forward row, NULL for `fwd = 0`; the
//!   per-origin share needs it). Two `ADD COLUMN`s: a row written before v4
//!   reads `fwd = 0`, `fwd_from` NULL, and a hub with no pairing keeps
//!   writing exactly that, so its rows and its pruning are unchanged. Typed
//!   access: [`super::rows::Fwd`].
//! - v5 (Layer 7 §5.8, the Pi floor; additive amendment of §10.3):
//!   `custody_totals` — the running row count and byte sums of `custody`
//!   under four kinds of key (member rows · forward rows · protected
//!   forward rows · the protected forward rows of one origin), kept by
//!   triggers on `custody` inside the writing transaction and backfilled by
//!   the migration. The §10.4 class totals of Custody and Transit, the
//!   §5.8 per-origin share, the forward gauge and the custody gauge read
//!   one or two rows by primary key instead of summing `custody`. No
//!   `custody` column changes; a store with no forward rows holds three
//!   fixed rows only. Typed access: [`super::prune::custody_total`]. Four
//!   partial indexes on `custody` (a dest's uncommitted rows, the
//!   protected forward rows, the committed rows, a dest's protected
//!   forward rows) make the commit, floor, forwarder-page, probe-restart
//!   and step-1 reads bounded searches ([`V5`]); [`heal`] re-creates a
//!   missing v5 trigger, index or fixed totals key at open.

use rusqlite::Connection;

use super::{Result, StoreError};

/// The schema version this build writes (v5: the custody totals).
pub const SCHEMA_VERSION: i64 = 5;

/// Migration `i` takes `user_version` from `i` to `i + 1`. Append only: a
/// released migration is never edited (crate-visible so the tests can build
/// an older store from the first entries).
pub(crate) const MIGRATIONS: &[&str] = &[V1, V2, V3, V4, V5];

const V1: &str = "
CREATE TABLE settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value ANY
) STRICT, WITHOUT ROWID;

CREATE TABLE contacts (
    node_id          BLOB PRIMARY KEY NOT NULL CHECK (length(node_id) = 32),
    enc_pubkey       BLOB NOT NULL CHECK (length(enc_pubkey) = 32),
    announce         BLOB NOT NULL,
    announce_counter INTEGER NOT NULL CHECK (announce_counter >= 0),
    trust            INTEGER NOT NULL CHECK (trust IN (1, 2, 3)),
    alias            TEXT,
    roles            INTEGER NOT NULL DEFAULT 0,
    home_hub         BLOB,
    first_seen_seq   INTEGER NOT NULL,
    last_seen_seq    INTEGER NOT NULL,
    request          INTEGER NOT NULL DEFAULT 0 CHECK (request IN (0, 1)),
    pinned           INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1))
) STRICT, WITHOUT ROWID;

CREATE TABLE messages (
    id                 INTEGER PRIMARY KEY,
    peer               BLOB NOT NULL CHECK (length(peer) = 32),
    sender             BLOB NOT NULL CHECK (length(sender) = 32),
    stream             INTEGER NOT NULL CHECK (stream >= 0),
    epoch              INTEGER NOT NULL CHECK (epoch >= 0),
    n                  INTEGER NOT NULL CHECK (n >= 0),
    dir                INTEGER NOT NULL CHECK (dir IN (0, 1, 2)),
    kind               INTEGER NOT NULL CHECK (kind BETWEEN 0 AND 255),
    class              INTEGER NOT NULL,
    size               INTEGER NOT NULL CHECK (size >= 0),
    local_seq          INTEGER NOT NULL,
    state              INTEGER NOT NULL CHECK (state IN (0, 1, 2, 3)),
    reason             TEXT,
    received_local_seq INTEGER,
    body               BLOB,
    blob_refs          BLOB,
    pinned             INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    UNIQUE (peer, sender, stream, epoch, n)
) STRICT;
CREATE INDEX messages_by_peer ON messages (peer, local_seq);
CREATE INDEX messages_by_age ON messages (local_seq);

CREATE TABLE tx_state (
    peer   BLOB NOT NULL CHECK (length(peer) = 32),
    stream INTEGER NOT NULL CHECK (stream >= 0),
    epoch  INTEGER NOT NULL CHECK (epoch >= 0),
    next_n INTEGER NOT NULL CHECK (next_n >= 1),
    PRIMARY KEY (peer, stream)
) STRICT, WITHOUT ROWID;

CREATE TABLE outbox (
    message_id  INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    frag_idx    INTEGER NOT NULL CHECK (frag_idx >= 0),
    frag_count  INTEGER NOT NULL CHECK (frag_count BETWEEN 1 AND 16),
    peer        BLOB NOT NULL CHECK (length(peer) = 32),
    sealed      BLOB,
    outstanding INTEGER NOT NULL DEFAULT 0 CHECK (outstanding IN (0, 1)),
    acked       INTEGER NOT NULL DEFAULT 0 CHECK (acked IN (0, 1)),
    local_seq   INTEGER NOT NULL,
    PRIMARY KEY (message_id, frag_idx),
    CHECK (frag_idx < frag_count)
) STRICT, WITHOUT ROWID;
CREATE INDEX outbox_by_peer ON outbox (peer, local_seq);

CREATE TABLE rx_state (
    peer     BLOB NOT NULL CHECK (length(peer) = 32),
    stream   INTEGER NOT NULL CHECK (stream >= 0),
    epoch    INTEGER NOT NULL CHECK (epoch >= 0),
    prefix   INTEGER NOT NULL CHECK (prefix >= 0),
    ranges   BLOB NOT NULL,
    partials BLOB NOT NULL,
    PRIMARY KEY (peer, stream, epoch)
) STRICT, WITHOUT ROWID;

CREATE TABLE rx_frags (
    peer       BLOB NOT NULL CHECK (length(peer) = 32),
    stream     INTEGER NOT NULL CHECK (stream >= 0),
    epoch      INTEGER NOT NULL CHECK (epoch >= 0),
    n          INTEGER NOT NULL CHECK (n >= 1),
    frag_idx   INTEGER NOT NULL CHECK (frag_idx >= 0),
    frag_count INTEGER NOT NULL CHECK (frag_count BETWEEN 1 AND 16),
    bytes      BLOB NOT NULL CHECK (length(bytes) <= 65535),
    UNIQUE (peer, stream, epoch, n, frag_idx),
    CHECK (frag_idx < frag_count)
) STRICT;

CREATE TABLE rx_floor (
    peer        BLOB NOT NULL CHECK (length(peer) = 32),
    stream      INTEGER NOT NULL CHECK (stream >= 0),
    floor_epoch INTEGER NOT NULL CHECK (floor_epoch >= 0),
    PRIMARY KEY (peer, stream)
) STRICT, WITHOUT ROWID;

CREATE TABLE custody (
    custody_seq INTEGER PRIMARY KEY AUTOINCREMENT,
    dest        BLOB NOT NULL CHECK (length(dest) = 32),
    pkt_ref     BLOB NOT NULL UNIQUE CHECK (length(pkt_ref) = 16),
    bytes       BLOB NOT NULL,
    committed   INTEGER NOT NULL DEFAULT 0 CHECK (committed IN (0, 1)),
    local_seq   INTEGER NOT NULL
) STRICT;
CREATE INDEX custody_by_dest ON custody (dest, custody_seq);

CREATE TABLE cursors (
    hub    BLOB PRIMARY KEY NOT NULL CHECK (length(hub) = 32),
    cursor INTEGER NOT NULL CHECK (cursor >= 0)
) STRICT, WITHOUT ROWID;

CREATE TABLE blobs (
    hash        BLOB PRIMARY KEY NOT NULL CHECK (length(hash) = 32),
    size        INTEGER NOT NULL CHECK (size >= 0),
    disk_bytes  INTEGER NOT NULL CHECK (disk_bytes >= 0),
    refcount    INTEGER NOT NULL CHECK (refcount >= 0),
    public      INTEGER NOT NULL CHECK (public IN (0, 1)),
    object_key  BLOB CHECK (object_key IS NULL OR length(object_key) = 32),
    class       INTEGER NOT NULL,
    local_seq   INTEGER NOT NULL,
    pinned      INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    refetchable INTEGER NOT NULL DEFAULT 0 CHECK (refetchable IN (0, 1)),
    superseded  INTEGER NOT NULL DEFAULT 0 CHECK (superseded IN (0, 1)),
    present     INTEGER NOT NULL DEFAULT 1 CHECK (present IN (0, 1)),
    CHECK (public = 0 OR object_key IS NULL)
) STRICT, WITHOUT ROWID;

CREATE TABLE requests (
    node_id   BLOB PRIMARY KEY NOT NULL CHECK (length(node_id) = 32),
    announce  BLOB NOT NULL,
    local_seq INTEGER NOT NULL
) STRICT, WITHOUT ROWID;
";

/// v2: `node1…` contacts waiting for their card (§7.2 (2), module doc).
const V2: &str = "
CREATE TABLE pending_node_ids (
    node_id   BLOB PRIMARY KEY NOT NULL CHECK (length(node_id) = 32),
    local_seq INTEGER NOT NULL
) STRICT, WITHOUT ROWID;
";

/// v3 (Layer 6 §3, D25 F1): the foreign message model (module doc).
///
/// - `foreign_messages.dir` `in` · `own`; `state` per Layer 6 §4
///   (`ForeignMessageState`); inbound rows carry `delivered`. Own rows are
///   either user-composed (`group_id` set) or relay copies (`edge_id` and
///   `origin_msg` set), never both, never neither.
/// - The dedupe key `(conv, sender_native, native_msg_id)` is an index, not a
///   `UNIQUE`: a Meshtastic packet id repeats (§3 dedupe).
/// - A relay copy part is unique per `(edge_id, origin_msg, part_k)` (partial
///   `UNIQUE` index over copies): a second copy of one origin on one edge is
///   unrepresentable (§8 guard 6; the store is the ID map and fails closed).
/// - `foreign_messages.id` and `relay_edges.id` are `AUTOINCREMENT`: a
///   deleted row's id (a cancelled, shed or pruned row, a deleted edge) is
///   never reused, so an `fm-`/`be-` id the UI holds never names another
///   row, and a handed copy's dangling `edge_id`/`origin_msg` never matches
///   a later edge or origin (the copy-part index above stays exact).
/// - `foreign_seen` holds keys only (bodies are prunable), bounded per edge
///   by the writer ([`super::foreign::FOREIGN_SEEN_CAP`]).
const V3: &str = "
CREATE TABLE foreign_conversations (
    id        INTEGER PRIMARY KEY,
    network   TEXT NOT NULL CHECK (network <> '111'),
    account   TEXT NOT NULL,
    kind      TEXT NOT NULL CHECK (kind IN ('channel', 'dm', 'group')),
    native_id TEXT NOT NULL,
    title     TEXT,
    last_seq  INTEGER NOT NULL DEFAULT 0 CHECK (last_seq >= 0),
    unread    INTEGER NOT NULL DEFAULT 0 CHECK (unread >= 0),
    pinned    INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    UNIQUE (network, account, kind, native_id)
) STRICT;

CREATE TABLE send_groups (
    id        INTEGER PRIMARY KEY,
    local_seq INTEGER NOT NULL CHECK (local_seq >= 1),
    text      TEXT NOT NULL,
    targets   TEXT NOT NULL CHECK (json_valid(targets))
) STRICT;

CREATE TABLE foreign_messages (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    conv          INTEGER NOT NULL REFERENCES foreign_conversations (id) ON DELETE CASCADE,
    dir           TEXT NOT NULL CHECK (dir IN ('in', 'own')),
    sender_native TEXT NOT NULL,
    sender_name   TEXT,
    auth          TEXT NOT NULL
                  CHECK (auth IN ('none', 'channel-key', 'pki', 'signed', 'attested-111')),
    native_msg_id TEXT,
    body          TEXT NOT NULL,
    local_seq     INTEGER NOT NULL CHECK (local_seq >= 1),
    claim_ts      INTEGER,
    state         TEXT NOT NULL
                  CHECK (state IN ('queued', 'sent', 'relayed', 'delivered', 'failed')),
    evidence      TEXT,
    reason_code   TEXT,
    group_id      INTEGER REFERENCES send_groups (id),
    part_k        INTEGER NOT NULL DEFAULT 1,
    part_n        INTEGER NOT NULL DEFAULT 1,
    origin_msg    INTEGER,
    edge_id       INTEGER,
    meta          TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(meta)),
    CHECK (part_n >= 1 AND part_k BETWEEN 1 AND part_n),
    CHECK ((origin_msg IS NULL) = (edge_id IS NULL)),
    CHECK (CASE dir
             WHEN 'in' THEN group_id IS NULL AND edge_id IS NULL
             ELSE (group_id IS NULL) <> (edge_id IS NULL)
           END)
) STRICT;
CREATE INDEX foreign_messages_by_key ON foreign_messages (conv, sender_native, native_msg_id);
CREATE INDEX foreign_messages_by_conv ON foreign_messages (conv, local_seq);
CREATE INDEX foreign_messages_queued ON foreign_messages (local_seq)
    WHERE dir = 'own' AND state = 'queued';
CREATE INDEX foreign_messages_by_group ON foreign_messages (group_id)
    WHERE group_id IS NOT NULL;
CREATE UNIQUE INDEX foreign_messages_copy_part ON foreign_messages (edge_id, origin_msg, part_k)
    WHERE edge_id IS NOT NULL;

CREATE TABLE relay_edges (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    src_conv          INTEGER NOT NULL REFERENCES foreign_conversations (id) ON DELETE CASCADE,
    dst_conv          INTEGER NOT NULL REFERENCES foreign_conversations (id) ON DELETE CASCADE,
    src_kind          TEXT NOT NULL CHECK (src_kind IN ('channel', 'group')),
    dst_kind          TEXT NOT NULL CHECK (dst_kind IN ('channel', 'group')),
    enabled           INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    paused            INTEGER NOT NULL DEFAULT 0 CHECK (paused IN (0, 1)),
    cursor_seq        INTEGER NOT NULL CHECK (cursor_seq >= 0),
    relayed_seq       INTEGER NOT NULL DEFAULT 0 CHECK (relayed_seq >= 0),
    honour_ok_to_mqtt INTEGER NOT NULL DEFAULT 0 CHECK (honour_ok_to_mqtt IN (0, 1)),
    oversize          TEXT NOT NULL DEFAULT 'refuse' CHECK (oversize IN ('refuse', 'split')),
    consent_hash      TEXT NOT NULL,
    enabled_by        TEXT NOT NULL,
    relayed           INTEGER NOT NULL DEFAULT 0 CHECK (relayed >= 0),
    shed              INTEGER NOT NULL DEFAULT 0 CHECK (shed >= 0),
    refused           INTEGER NOT NULL DEFAULT 0 CHECK (refused >= 0),
    CHECK (src_conv <> dst_conv),
    UNIQUE (src_conv, dst_conv)
) STRICT;

CREATE TABLE foreign_seen (
    edge          INTEGER NOT NULL REFERENCES relay_edges (id) ON DELETE CASCADE,
    conv          INTEGER NOT NULL,
    sender_native TEXT NOT NULL,
    native_msg_id TEXT NOT NULL,
    local_seq     INTEGER NOT NULL CHECK (local_seq >= 0),
    PRIMARY KEY (edge, conv, sender_native, native_msg_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX foreign_seen_by_age ON foreign_seen (edge, local_seq);
CREATE INDEX foreign_seen_by_key ON foreign_seen (conv, sender_native, native_msg_id);
";

/// v4 (Layer 7 §5.3 amendment 5, §5.1, §5.8): the forward-table columns of
/// `custody` (module doc).
///
/// The two statements are amendment 5's; `fwd_from` also carries a
/// fail-closed `CHECK` (an as-built addition, the house rule for every
/// NodeId column): it is exactly 32 B, and it is NULL exactly when
/// `fwd = 0` (a forward row always names its origin, a member row never
/// does). Every row a v3 store already has passes both checks (`fwd`
/// defaults to 0, `fwd_from` to NULL), so the migration never fails on old
/// rows.
const V4: &str = "
ALTER TABLE custody ADD COLUMN fwd INTEGER NOT NULL DEFAULT 0 CHECK (fwd IN (0, 1, 2, 3));
ALTER TABLE custody ADD COLUMN fwd_from BLOB
    CHECK ((fwd_from IS NULL OR length(fwd_from) = 32) AND ((fwd = 0) = (fwd_from IS NULL)));
";

// v5 is written as pieces (each a string literal, so `V5` stays one
// literal through `concat!`): the migration runs them all, and [`heal`]
// re-runs a missing trigger or index and the backfill from the same text.

/// v5: the totals table (key shapes and fail-closed counters, [`V5`]).
macro_rules! v5_table {
    () => {
        "CREATE TABLE custody_totals (
    key        BLOB PRIMARY KEY NOT NULL
               CHECK (key IN (x'00', x'01', x'02')
                      OR (length(key) = 33 AND substr(key, 1, 1) = x'03')),
    rows       INTEGER NOT NULL CHECK (rows >= 0),
    item_bytes INTEGER NOT NULL CHECK (item_bytes >= 0),
    from_bytes INTEGER NOT NULL CHECK (from_bytes >= 0)
) STRICT, WITHOUT ROWID;
"
    };
}

/// v5: the backfill, one grouped pass over `custody` into an empty
/// `custody_totals` ([`V5`]; [`rebuild_custody_totals`]).
macro_rules! v5_backfill {
    () => {
        "WITH g AS MATERIALIZED (
    SELECT fwd != 0 AS f,
           fwd != 0 AND committed = 0 AS p,
           fwd_from AS o,
           count(*) AS n,
           sum(length(bytes)) AS ib,
           sum(coalesce(length(fwd_from), 0)) AS fb
    FROM custody
    GROUP BY 1, 2, 3
)
INSERT INTO custody_totals (key, rows, item_bytes, from_bytes)
    SELECT x'00', coalesce(sum(n), 0), coalesce(sum(ib), 0), coalesce(sum(fb), 0)
    FROM g WHERE NOT f
    UNION ALL
    SELECT x'01', coalesce(sum(n), 0), coalesce(sum(ib), 0), coalesce(sum(fb), 0)
    FROM g WHERE f
    UNION ALL
    SELECT x'02', coalesce(sum(n), 0), coalesce(sum(ib), 0), coalesce(sum(fb), 0)
    FROM g WHERE p
    UNION ALL
    SELECT CAST(x'03' || o AS BLOB), n, ib, fb
    FROM g WHERE p;
"
    };
}

/// v5: a new row joins every key it counts under ([`V5`]).
macro_rules! v5_trigger_insert {
    () => {
        "CREATE TRIGGER custody_totals_insert AFTER INSERT ON custody
BEGIN
    INSERT INTO custody_totals (key, rows, item_bytes, from_bytes)
        SELECT CASE WHEN NEW.fwd = 0 THEN x'00' ELSE x'01' END,
               1, length(NEW.bytes), coalesce(length(NEW.fwd_from), 0)
        WHERE true
        ON CONFLICT (key) DO UPDATE SET rows = rows + 1,
            item_bytes = item_bytes + excluded.item_bytes,
            from_bytes = from_bytes + excluded.from_bytes;
    INSERT INTO custody_totals (key, rows, item_bytes, from_bytes)
        SELECT x'02', 1, length(NEW.bytes), length(NEW.fwd_from)
        WHERE NEW.fwd != 0 AND NEW.committed = 0
        ON CONFLICT (key) DO UPDATE SET rows = rows + 1,
            item_bytes = item_bytes + excluded.item_bytes,
            from_bytes = from_bytes + excluded.from_bytes;
    INSERT INTO custody_totals (key, rows, item_bytes, from_bytes)
        SELECT CAST(x'03' || NEW.fwd_from AS BLOB), 1, length(NEW.bytes), length(NEW.fwd_from)
        WHERE NEW.fwd != 0 AND NEW.committed = 0
        ON CONFLICT (key) DO UPDATE SET rows = rows + 1,
            item_bytes = item_bytes + excluded.item_bytes,
            from_bytes = from_bytes + excluded.from_bytes;
END;
"
    };
}

/// v5: a deleted row leaves every key it counted under ([`V5`]).
macro_rules! v5_trigger_delete {
    () => {
        "CREATE TRIGGER custody_totals_delete AFTER DELETE ON custody
BEGIN
    UPDATE custody_totals SET rows = rows - 1,
        item_bytes = item_bytes - length(OLD.bytes),
        from_bytes = from_bytes - coalesce(length(OLD.fwd_from), 0)
        WHERE key = CASE WHEN OLD.fwd = 0 THEN x'00' ELSE x'01' END;
    UPDATE custody_totals SET rows = rows - 1,
        item_bytes = item_bytes - length(OLD.bytes),
        from_bytes = from_bytes - length(OLD.fwd_from)
        WHERE OLD.fwd != 0 AND OLD.committed = 0
          AND key IN (x'02', CAST(x'03' || OLD.fwd_from AS BLOB));
    DELETE FROM custody_totals
        WHERE OLD.fwd != 0 AND OLD.committed = 0
          AND key = CAST(x'03' || OLD.fwd_from AS BLOB) AND rows = 0;
END;
"
    };
}

/// v5: an updated row leaves its old keys and joins its new ones ([`V5`]).
macro_rules! v5_trigger_update {
    () => {
        "CREATE TRIGGER custody_totals_update AFTER UPDATE OF fwd, fwd_from, committed, bytes ON custody
BEGIN
    UPDATE custody_totals SET rows = rows - 1,
        item_bytes = item_bytes - length(OLD.bytes),
        from_bytes = from_bytes - coalesce(length(OLD.fwd_from), 0)
        WHERE key = CASE WHEN OLD.fwd = 0 THEN x'00' ELSE x'01' END;
    UPDATE custody_totals SET rows = rows - 1,
        item_bytes = item_bytes - length(OLD.bytes),
        from_bytes = from_bytes - length(OLD.fwd_from)
        WHERE OLD.fwd != 0 AND OLD.committed = 0
          AND key IN (x'02', CAST(x'03' || OLD.fwd_from AS BLOB));
    INSERT INTO custody_totals (key, rows, item_bytes, from_bytes)
        SELECT CASE WHEN NEW.fwd = 0 THEN x'00' ELSE x'01' END,
               1, length(NEW.bytes), coalesce(length(NEW.fwd_from), 0)
        WHERE true
        ON CONFLICT (key) DO UPDATE SET rows = rows + 1,
            item_bytes = item_bytes + excluded.item_bytes,
            from_bytes = from_bytes + excluded.from_bytes;
    INSERT INTO custody_totals (key, rows, item_bytes, from_bytes)
        SELECT x'02', 1, length(NEW.bytes), length(NEW.fwd_from)
        WHERE NEW.fwd != 0 AND NEW.committed = 0
        ON CONFLICT (key) DO UPDATE SET rows = rows + 1,
            item_bytes = item_bytes + excluded.item_bytes,
            from_bytes = from_bytes + excluded.from_bytes;
    INSERT INTO custody_totals (key, rows, item_bytes, from_bytes)
        SELECT CAST(x'03' || NEW.fwd_from AS BLOB), 1, length(NEW.bytes), length(NEW.fwd_from)
        WHERE NEW.fwd != 0 AND NEW.committed = 0
        ON CONFLICT (key) DO UPDATE SET rows = rows + 1,
            item_bytes = item_bytes + excluded.item_bytes,
            from_bytes = from_bytes + excluded.from_bytes;
    DELETE FROM custody_totals
        WHERE OLD.fwd != 0 AND OLD.committed = 0
          AND key = CAST(x'03' || OLD.fwd_from AS BLOB) AND rows = 0;
END;
"
    };
}

/// v5: a member's uncommitted rows by `custody_seq` ([`V5`]).
macro_rules! v5_index_open {
    () => {
        "CREATE INDEX custody_open_by_dest ON custody (dest, custody_seq) WHERE committed = 0;
"
    };
}

/// v5: the protected forward rows by `custody_seq` ([`V5`]).
macro_rules! v5_index_forward {
    () => {
        "CREATE INDEX custody_forward_open ON custody (custody_seq) WHERE fwd != 0 AND committed = 0;
"
    };
}

/// v5: the committed (step-1 evictable) rows by `fwd` ([`V5`]).
macro_rules! v5_index_committed {
    () => {
        "CREATE INDEX custody_committed ON custody (fwd) WHERE committed = 1;
"
    };
}

/// v5 (Layer 7 M4g): the protected forward rows of one dest by
/// `custody_seq` ([`V5`]; `fwd` rides along so the read it serves is
/// covered).
macro_rules! v5_index_forward_dest {
    () => {
        "CREATE INDEX custody_forward_open_by_dest ON custody (dest, custody_seq, fwd)
    WHERE fwd != 0 AND committed = 0;
"
    };
}

/// v5 (Layer 7 §5.8; L5 §10.3, §10.4; the Pi floor): `custody_totals`
/// (module doc), so admission reads one row per total whatever the rows a
/// hub holds, and four partial indexes on `custody`, so a member's
/// commit and floor, the forwarder's pages, its probe-restart read and
/// the §10.4 step-1 read search only the rows they want.
///
/// - **Keys.** `x'00'` the member rows (`fwd = 0`, [`super::Class::Custody`])
///   · `x'01'` every forward row (`fwd != 0`, [`super::Class::Transit`]) ·
///   `x'02'` the protected forward rows (`fwd != 0 AND committed = 0`) ·
///   `x'03' ‖ fwd_from` (33 B) the protected forward rows of one origin. A
///   row counts under its class key and, while it is a protected forward
///   row, under `x'02'` and its origin's key.
/// - **Values.** `rows` counts rows, `item_bytes` sums `length(bytes)`,
///   `from_bytes` sums `coalesce(length(fwd_from), 0)`. The per-row
///   overhead is never stored: the reader multiplies `rows` by it
///   ([`super::prune::CustodyTotal::bytes`]), so the totals are the same
///   facts the §10.4 sums read.
/// - **Triggers.** After an insert, a delete, or an update of `fwd`,
///   `fwd_from`, `committed` or `bytes`, the old row's contribution leaves
///   every key it counted under and the new row's joins every key it
///   counts under (an upsert by primary key); an origin key whose `rows`
///   reach 0 is deleted, so the table never grows with the origins ever
///   seen. They run inside the writing statement's transaction: a
///   rollback takes the totals back with the rows. No other trigger,
///   foreign key or `REPLACE` writes `custody` (every writer is an
///   `INSERT`, an `UPDATE … SET committed` or a `DELETE` by `custody_seq`).
/// - **Fail closed.** The counters carry `CHECK (… >= 0)` and the key its
///   shape, so a total that would go negative fails the write, never
///   reads as room. The three fixed keys are written by the backfill and
///   never deleted; an origin key exists exactly while it has rows.
/// - **Backfill.** One grouped pass over `custody` in the migration's
///   transaction (`g`, at most 2 + 2 × origins groups), then the keys
///   from `g`: a v4 store opens with exact totals.
/// - **Partial indexes** (each holds only the rows its readers want, so
///   each read is a bounded `SEARCH`): `custody_open_by_dest (dest,
///   custody_seq) WHERE committed = 0` — a member's `MAILBOX_COMMIT`
///   ([`super::Store::custody_commit`]) and its `ENROLL_OK` floor
///   ([`super::Store::custody_floor`]) read its uncommitted rows only,
///   never the committed ones it already acknowledged ·
///   `custody_forward_open (custody_seq) WHERE fwd != 0 AND committed = 0`
///   — the forwarder's pages ([`super::Store::forward_pending`],
///   [`super::Store::forward_pending_lens`]) read protected forward rows
///   only, never the member or committed rows between them ·
///   `custody_committed (fwd) WHERE committed = 1` — the §10.4 step-1 read
///   ([`super::prune`]) reads the committed rows of the class it prunes
///   only, never a protected row ·
///   `custody_forward_open_by_dest (dest, custody_seq, fwd) WHERE fwd != 0
///   AND committed = 0` (Layer 7 M4g) — the forwarder's probe-restart read
///   ([`super::Store::forward_waits_for_dest`]) visits a dest's protected
///   forward rows only, never its committed rows or its member rows
///   (`custody_open_by_dest` holds the latter, which a dest that is or was
///   a member here keeps uncommitted up to a quota); `fwd` rides along so
///   the index covers that read and the planner prefers it to
///   `custody_open_by_dest` in any creation order. A put writes one entry
///   (a member row) or three (a forward row) in these partial indexes; a
///   commit moves a row from the open indexes (one for a member row, three
///   for a forward row) to `custody_committed`; a delete removes it.
/// - **Self-heal** ([`heal`]): at every open the store checks that these
///   triggers and indexes exist (one read of `sqlite_master`) and that the
///   three fixed keys do (three primary-key lookups), and re-creates what
///   is missing; a missing trigger, table or fixed key also rebuilds the
///   totals from the rows ([`rebuild_custody_totals`]); an index that
///   cannot be re-created is counted, never fatal.
const V5: &str = concat!(
    v5_table!(),
    "\n",
    v5_backfill!(),
    "\n",
    v5_trigger_insert!(),
    "\n",
    v5_trigger_delete!(),
    "\n",
    v5_trigger_update!(),
    "\n",
    v5_index_open!(),
    v5_index_forward!(),
    v5_index_committed!(),
    v5_index_forward_dest!(),
);

/// The v5 objects [`heal`] checks at open: `(type, name, DDL)` as
/// `sqlite_master` names them, the DDL the same text [`V5`] runs.
const V5_OBJECTS: [(&str, &str, &str); 8] = [
    ("table", "custody_totals", v5_table!()),
    ("trigger", "custody_totals_insert", v5_trigger_insert!()),
    ("trigger", "custody_totals_delete", v5_trigger_delete!()),
    ("trigger", "custody_totals_update", v5_trigger_update!()),
    ("index", "custody_open_by_dest", v5_index_open!()),
    ("index", "custody_forward_open", v5_index_forward!()),
    ("index", "custody_committed", v5_index_committed!()),
    (
        "index",
        "custody_forward_open_by_dest",
        v5_index_forward_dest!(),
    ),
];

/// The read [`heal`] makes of `sqlite_master`: which [`V5_OBJECTS`] it
/// holds, one `?` per object, bound to their names (a handful of schema
/// rows; no `custody` row is read). Built from [`V5_OBJECTS`], so the two
/// cannot disagree.
fn v5_present(conn: &Connection) -> Result<Vec<(String, String)>> {
    let marks = vec!["?"; V5_OBJECTS.len()].join(", ");
    let mut st = conn.prepare(&format!(
        "SELECT type, name FROM sqlite_master WHERE name IN ({marks})"
    ))?;
    let rows = st.query_map(
        rusqlite::params_from_iter(V5_OBJECTS.iter().map(|(_, name, _)| *name)),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )?;
    let mut present = Vec::with_capacity(V5_OBJECTS.len());
    for r in rows {
        present.push(r?);
    }
    Ok(present)
}

/// How many of the three fixed `custody_totals` keys (`x'00'`, `x'01'`,
/// `x'02'`, [`V5`]) are present: three primary-key lookups.
const FIXED_KEYS_SQL: &str =
    "SELECT count(*) FROM custody_totals WHERE key IN (x'00', x'01', x'02')";

/// Recompute `custody_totals` from the `custody` rows: empty it, then the
/// v5 backfill ([`V5`]'s own text). Runs inside the caller's transaction,
/// so the rows and their totals change together (the migration's, or
/// [`heal`]'s); one grouped pass over `custody`, used only when the
/// totals cannot be trusted (a trigger or a fixed key was missing).
pub(crate) fn rebuild_custody_totals(c: &Connection) -> Result<()> {
    c.execute_batch(concat!("DELETE FROM custody_totals;\n", v5_backfill!()))?;
    Ok(())
}

/// Self-heal at open (v5, the Pi floor): after [`migrate`], read which v5
/// objects exist ([`v5_present`]) and whether the three fixed totals keys
/// do ([`FIXED_KEYS_SQL`]), then re-create what is missing from its v5
/// text. Returns how many missing indexes could not be re-created.
///
/// - **Fatal: the totals.** When the totals table, a trigger or a fixed
///   key was missing, writes may have bypassed the totals (or a reader
///   would take a missing key for zero), so in one transaction the
///   missing table and triggers are re-created and the totals rebuilt from
///   the rows ([`rebuild_custody_totals`]): a drifted total would read as
///   room or fail a later write on its `CHECK` (wedging eviction). Any
///   error here fails the open: the totals guard correctness.
/// - **Not fatal: an index.** A missing index changes no fact, so each is
///   re-created on its own, after the totals, as one autocommit statement:
///   a failure (`SQLITE_FULL` on a nearly full disk, the very moment the
///   store is under pressure) rolls that statement back alone, is counted
///   in the return value (the fabric counts it as an error at start) and
///   the open carries on; the statements it served run on the wider index
///   or the table until a later open re-creates it.
///
/// A whole store pays the two reads and nothing else. Drift with every
/// trigger and key present (a hand edit of `custody_totals`) is not
/// detected here: that needs a pass over the rows.
pub(crate) fn heal(conn: &Connection) -> Result<u32> {
    let present = v5_present(conn)?;
    let missing: Vec<&(&str, &str, &str)> = V5_OBJECTS
        .iter()
        .filter(|(ty, name, _)| !present.iter().any(|(t, n)| t == ty && n == name))
        .collect();
    let table_present = !missing.iter().any(|(ty, _, _)| *ty == "table");
    let keys_lost =
        table_present && conn.query_row(FIXED_KEYS_SQL, [], |r| r.get::<_, i64>(0))? < 3;
    let (indexes, totals): (Vec<_>, Vec<_>) =
        missing.into_iter().partition(|(ty, _, _)| *ty == "index");
    if !totals.is_empty() || keys_lost {
        let tx = conn.unchecked_transaction()?;
        for (_, _, ddl) in totals {
            tx.execute_batch(ddl)?;
        }
        rebuild_custody_totals(&tx)?;
        tx.commit()?;
    }
    let mut failed = 0u32;
    for (_, _, ddl) in indexes {
        if conn.execute_batch(ddl).is_err() {
            failed += 1;
            // An autocommit statement rolls back alone; were SQLite to
            // leave a transaction open, it goes too (nothing else is in it).
            if !conn.is_autocommit() {
                conn.execute_batch("ROLLBACK;")?;
            }
        }
    }
    Ok(failed)
}

/// Bring the schema to [`SCHEMA_VERSION`].
pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    migrate_with(conn, MIGRATIONS)
}

/// [`migrate`] over an explicit migration list: `migrations.len()` is the
/// newest version that list writes. The tests pass a prefix of
/// [`MIGRATIONS`] to stand in for an older build (a v3 build is this code
/// with `[V1, V2, V3]`), so "a v4 store is refused by a v3 build" is
/// checked against the exact `SchemaTooNew` path such a build runs.
pub(crate) fn migrate_with(conn: &Connection, migrations: &[&str]) -> Result<()> {
    let found = super::pragma_int(conn, "user_version")?;
    let supported = migrations.len() as i64;
    if found > supported {
        return Err(StoreError::SchemaTooNew { found, supported });
    }
    if found < 0 {
        return Err(StoreError::Corrupt("user_version"));
    }
    if found == supported {
        // An existing store must already carry the §10.1 auto_vacuum mode.
        let av = super::pragma_int(conn, "auto_vacuum")?;
        if av != 2 {
            return Err(StoreError::Pragma("auto_vacuum"));
        }
        return Ok(());
    }
    conn.pragma_update(None, "synchronous", "FULL")?;
    let res = (|| -> Result<()> {
        for (i, sql) in migrations.iter().enumerate().skip(found as usize) {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i as i64 + 1)?;
            tx.commit()?;
        }
        Ok(())
    })();
    let reset = conn.pragma_update(None, "synchronous", "NORMAL");
    res?;
    reset?;
    let av = super::pragma_int(conn, "auto_vacuum")?;
    if av != 2 {
        return Err(StoreError::Pragma("auto_vacuum"));
    }
    Ok(())
}
