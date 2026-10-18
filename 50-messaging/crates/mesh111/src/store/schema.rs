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

use rusqlite::Connection;

use super::{Result, StoreError};

/// The schema version this build writes.
pub const SCHEMA_VERSION: i64 = 2;

/// Migration `i` takes `user_version` from `i` to `i + 1`.
const MIGRATIONS: &[&str] = &[V1, V2];

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
