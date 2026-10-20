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

use rusqlite::Connection;

use super::{Result, StoreError};

/// The schema version this build writes (v3: Layer 6 §3).
pub const SCHEMA_VERSION: i64 = 3;

/// Migration `i` takes `user_version` from `i` to `i + 1`. Append only: a
/// released migration is never edited (crate-visible so the tests can build
/// an older store from the first entries).
pub(crate) const MIGRATIONS: &[&str] = &[V1, V2, V3];

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

/// Bring the schema to [`SCHEMA_VERSION`].
pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    let found = super::pragma_int(conn, "user_version")?;
    let supported = MIGRATIONS.len() as i64;
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
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
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
