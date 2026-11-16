//! Layer 8 (`os/8-storage/README.md` §2.2 rules 4 and 5; §3 J4; row D2): the node-state store's
//! schema.
//!
//! **Schema v1**, with its own `user_version` ([`SCHEMA_VERSION`]), migrated append-only through
//! mesh111's `migrate_with` (§2.7 S2, `M/store/schema.rs`): entry `i` of [`MIGRATIONS`] runs in one
//! transaction at `synchronous = FULL` and sets `user_version` to `i + 1`; a file whose
//! `user_version` is above [`SCHEMA_VERSION`] is refused, never downgraded (§2.2 rule 8); a file
//! already at it is only read. The four STRICT tables of §2.2 rule 4, exactly:
//!
//! - `meta` (`key`, `value`): [`NEXT_SEQ`], the one allocator of `seq`, and later the export offers
//!   per class (§4.5) and the prune markers (J3). No clean record (rule 9).
//! - `records` (`ns`, `id`, `seq`, `boot_id`, `mono_ms`, `wall`, `state`, `body`), primary key
//!   (`ns`, `id`), `seq` unique, and the (`ns`, `state`, `seq`) index that the reclaim and the J9
//!   `list` cursor read: the documents (a conversation's head, a proposal, a task, a decision, an
//!   incident).
//! - `entries` (`ns`, `stream`, `seq`, `body`), primary key (`stream`, `seq`), `seq` unique: the
//!   append streams (a conversation's messages, Layer 9's audit trail). `ns` is the stream's
//!   namespace, so a trigger keys `ns_totals` without parsing `stream`.
//! - `ns_totals` (`ns`, `rows`, `bytes`), kept by `AFTER INSERT/DELETE/UPDATE` triggers on
//!   `records` and `entries`, inside the writing transaction (the M1b pattern of the fabric's
//!   `custody_totals`): `bytes` is Σ `length(body)` plus [`ROW_OVERHEAD`] per row, the fabric's
//!   `ROW_OVERHEAD` (`M/store/prune.rs:135`). A namespace's row goes with its last row.
//!
//! Every column but `wall` is `NOT NULL`: `wall TEXT NULL` is the one column rule 4 marks nullable,
//! a display claim that orders nothing (J4). `seq` is the store's monotonic sequence and the only
//! order (rule 4): [`next_seq`] takes it from `meta` inside the writing transaction, a new one for
//! every write. `boot_id` and `mono_ms` sit beside it. The namespaces of v1 (rule 5) are their
//! writers' (`agent/conversations`, `agent/proposals`, `agent/tasks`, `agent/decisions`, `heal/…`,
//! `storage/…`): the schema keys nothing on their names.

use super::{NsTotal, StateError, Totals};
use mesh111::store::schema::migrate_with;
use mesh111::store::StoreError;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension};

/// The node-state store's schema version, its own `user_version` (§2.2 rule 4): the length of
/// [`MIGRATIONS`].
pub const SCHEMA_VERSION: i64 = 1;

/// The fixed per-row overhead `ns_totals.bytes` counts beside `length(body)` (§2.2 rule 4): the
/// fabric's `ROW_OVERHEAD` (`M/store/prune.rs:135`, 64 B), written as a literal in [`V1`]'s
/// triggers.
pub const ROW_OVERHEAD: u64 = 64;

/// The `meta` key of the `seq` allocator (§2.2 rule 4): the next `seq` to hand out.
pub const NEXT_SEQ: &str = "next_seq";

/// Schema v1 (§2.2 rule 4): the four STRICT tables, the (`ns`, `state`, `seq`) index, the six
/// `ns_totals` triggers and the allocator's first value.
pub const V1: &str = "
CREATE TABLE meta (
    key   TEXT NOT NULL PRIMARY KEY,
    value ANY NOT NULL
) STRICT;
INSERT INTO meta (key, value) VALUES ('next_seq', 1);
CREATE TABLE records (
    ns      TEXT NOT NULL,
    id      TEXT NOT NULL,
    seq     INTEGER NOT NULL UNIQUE,
    boot_id BLOB NOT NULL,
    mono_ms INTEGER NOT NULL,
    wall    TEXT NULL,
    state   TEXT NOT NULL,
    body    BLOB NOT NULL,
    PRIMARY KEY (ns, id)
) STRICT;
CREATE INDEX records_ns_state_seq ON records (ns, state, seq);
CREATE TABLE entries (
    ns     TEXT NOT NULL,
    stream TEXT NOT NULL,
    seq    INTEGER NOT NULL UNIQUE,
    body   BLOB NOT NULL,
    PRIMARY KEY (stream, seq)
) STRICT;
CREATE TABLE ns_totals (
    ns    TEXT NOT NULL PRIMARY KEY,
    rows  INTEGER NOT NULL,
    bytes INTEGER NOT NULL
) STRICT;
CREATE TRIGGER records_totals_insert AFTER INSERT ON records
BEGIN
    INSERT INTO ns_totals (ns, rows, bytes)
        SELECT NEW.ns, 1, length(NEW.body) + 64 WHERE true
        ON CONFLICT (ns) DO UPDATE SET rows = rows + 1, bytes = bytes + excluded.bytes;
END;
CREATE TRIGGER records_totals_delete AFTER DELETE ON records
BEGIN
    UPDATE ns_totals SET rows = rows - 1, bytes = bytes - (length(OLD.body) + 64)
        WHERE ns = OLD.ns;
    DELETE FROM ns_totals WHERE ns = OLD.ns AND rows = 0;
END;
CREATE TRIGGER records_totals_update AFTER UPDATE OF ns, body ON records
BEGIN
    UPDATE ns_totals SET rows = rows - 1, bytes = bytes - (length(OLD.body) + 64)
        WHERE ns = OLD.ns;
    INSERT INTO ns_totals (ns, rows, bytes)
        SELECT NEW.ns, 1, length(NEW.body) + 64 WHERE true
        ON CONFLICT (ns) DO UPDATE SET rows = rows + 1, bytes = bytes + excluded.bytes;
    DELETE FROM ns_totals WHERE ns = OLD.ns AND rows = 0;
END;
CREATE TRIGGER entries_totals_insert AFTER INSERT ON entries
BEGIN
    INSERT INTO ns_totals (ns, rows, bytes)
        SELECT NEW.ns, 1, length(NEW.body) + 64 WHERE true
        ON CONFLICT (ns) DO UPDATE SET rows = rows + 1, bytes = bytes + excluded.bytes;
END;
CREATE TRIGGER entries_totals_delete AFTER DELETE ON entries
BEGIN
    UPDATE ns_totals SET rows = rows - 1, bytes = bytes - (length(OLD.body) + 64)
        WHERE ns = OLD.ns;
    DELETE FROM ns_totals WHERE ns = OLD.ns AND rows = 0;
END;
CREATE TRIGGER entries_totals_update AFTER UPDATE OF ns, body ON entries
BEGIN
    UPDATE ns_totals SET rows = rows - 1, bytes = bytes - (length(OLD.body) + 64)
        WHERE ns = OLD.ns;
    INSERT INTO ns_totals (ns, rows, bytes)
        SELECT NEW.ns, 1, length(NEW.body) + 64 WHERE true
        ON CONFLICT (ns) DO UPDATE SET rows = rows + 1, bytes = bytes + excluded.bytes;
    DELETE FROM ns_totals WHERE ns = OLD.ns AND rows = 0;
END;
";

/// The node-state store's migration list for `migrate_with` (§2.2 rule 4): append-only, entry `i`
/// brings a file from version `i` to `i + 1`.
pub const MIGRATIONS: &[&str] = &[V1];

/// Bring an opened store to [`SCHEMA_VERSION`] through mesh111's `migrate_with` (§2.7 S2): a new
/// file gets v1 in one `FULL` transaction; a file at v1 is only read (nothing written, §2.2 rule
/// 9); a newer one is [`StoreError::SchemaTooNew`] (§2.2 rule 8). Row D17 takes the migration copy
/// (§13.6 (1)) before a file between 0 and this version migrates; v1 has no such file (a version-0
/// file is a new one: no copy). The store calls it after [`held`], and a use that only reads never
/// migrates a file [`Held::Unwritten`] (rule 9).
pub fn migrate(conn: &Connection) -> Result<(), StoreError> {
    migrate_with(conn, MIGRATIONS)
}

/// What an opened file holds before [`migrate`] runs (§2.2 rules 4, 8 and 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// `user_version` 0 and no schema object: a store never written. A first write's open made
    /// the file (mesh111's `open_encrypted` gives a new file its header page and no table) and its
    /// migration did not commit (a power cut, a full disk). A read sees an empty store and writes
    /// nothing; the next write migrates it.
    Unwritten,
    /// `user_version` 0 with schema objects: not a file this schema wrote (the v1 migration sets
    /// `user_version` in the transaction that creates the tables), refused as a damaged one
    /// (§2.2 rule 8), never migrated over.
    Unversioned,
    /// Any other `user_version`: [`migrate`] reads a file at this version, refuses a newer or a
    /// negative one.
    Versioned,
}

/// What `conn`'s file holds (its `user_version` and, at 0, whether `sqlite_master` lists
/// anything), read before [`migrate`] (§2.2 rule 9: a read never migrates a file never written).
/// Reads only.
pub fn held(conn: &Connection) -> Result<Held, StoreError> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version != 0 {
        return Ok(Held::Versioned);
    }
    let objects: i64 = conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
    Ok(if objects == 0 {
        Held::Unwritten
    } else {
        Held::Unversioned
    })
}

/// Take the next `seq` from `meta.next_seq` inside the writing transaction `tx` (§2.2 rule 4):
/// the store's monotonic sequence, a new value on every call, so every write of a record (a `put`
/// or a `set_state` on an existing one included) and every appended entry takes its own. The
/// allocator's value is checked (an integer of at least 1); a file whose `meta` lost the key
/// resumes above the highest `seq` held, so a value is never handed out twice. Its work does not
/// grow with the rows held (one primary-key read and one write; the recovery reads two unique
/// indexes' last keys).
pub fn next_seq(tx: &Connection) -> Result<i64, StateError> {
    let held: Option<Value> = tx
        .query_row("SELECT value FROM meta WHERE key = 'next_seq'", [], |r| {
            r.get(0)
        })
        .optional()?;
    let seq = match held {
        Some(Value::Integer(n)) if n >= 1 => n,
        Some(_) => return Err(StateError::Corrupt("meta.next_seq")),
        None => tx.query_row(
            "SELECT max(coalesce((SELECT max(seq) FROM records), 0),
                        coalesce((SELECT max(seq) FROM entries), 0)) + 1",
            [],
            |r| r.get::<_, i64>(0),
        )?,
    };
    let following = seq
        .checked_add(1)
        .ok_or(StateError::Corrupt("meta.next_seq"))?;
    tx.execute(
        "INSERT INTO meta (key, value) VALUES ('next_seq', ?1)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [following],
    )?;
    Ok(seq)
}

/// Every namespace's `ns_totals` row (§2.2 rule 4): read at the first open after a daemon start
/// and inside every writing transaction, before its commit, so the daemon keeps the totals in
/// memory from its own writes (§2.2 rule 7). One row per namespace that holds rows; a negative
/// figure (a damaged row) reads as 0.
pub fn read_totals(conn: &Connection) -> Result<Totals, StateError> {
    let mut stmt = conn.prepare_cached("SELECT ns, rows, bytes FROM ns_totals")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    let mut totals = Totals::new();
    for row in rows {
        let (ns, n, bytes) = row?;
        totals.insert(
            ns,
            NsTotal {
                rows: u64::try_from(n).unwrap_or(0),
                bytes: u64::try_from(bytes).unwrap_or(0),
            },
        );
    }
    Ok(totals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    /// A v1 store in memory (the schema alone: no key, no file).
    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")
            .unwrap();
        migrate(&c).unwrap();
        c
    }

    fn put(c: &Connection, ns: &str, id: &str, state: &str, body: &[u8]) -> i64 {
        let seq = next_seq(c).unwrap();
        c.execute(
            "INSERT INTO records (ns, id, seq, boot_id, mono_ms, wall, state, body)
                 VALUES (?1, ?2, ?3, x'00', 0, NULL, ?4, ?5)
                 ON CONFLICT (ns, id) DO UPDATE SET seq = excluded.seq,
                     state = excluded.state, body = excluded.body",
            params![ns, id, seq, state, body],
        )
        .unwrap();
        seq
    }

    fn append(c: &Connection, ns: &str, stream: &str, body: &[u8]) -> i64 {
        let seq = next_seq(c).unwrap();
        c.execute(
            "INSERT INTO entries (ns, stream, seq, body) VALUES (?1, ?2, ?3, ?4)",
            params![ns, stream, seq, body],
        )
        .unwrap();
        seq
    }

    fn total(c: &Connection, ns: &str) -> Option<NsTotal> {
        read_totals(c).unwrap().get(ns).copied()
    }

    /// The figures `ns_totals` must hold for `ns`: its rows counted from the tables themselves.
    fn recount(c: &Connection, ns: &str) -> Option<NsTotal> {
        let (rows, bytes): (i64, i64) = c
            .query_row(
                "SELECT count(*), coalesce(sum(length(body) + ?2), 0) FROM (
                     SELECT body FROM records WHERE ns = ?1
                     UNION ALL SELECT body FROM entries WHERE ns = ?1)",
                params![ns, ROW_OVERHEAD as i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        (rows > 0).then_some(NsTotal {
            rows: rows as u64,
            bytes: bytes as u64,
        })
    }

    #[test]
    fn v1_is_exactly_the_four_strict_tables_the_index_and_the_triggers() {
        let c = mem();
        let v: i64 = c
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        assert_eq!(MIGRATIONS.len() as i64, SCHEMA_VERSION);
        let names = |ty: &str| -> Vec<String> {
            let mut s = c
                .prepare(
                    "SELECT name FROM sqlite_master WHERE type = ?1 AND name NOT LIKE 'sqlite_%'
                     ORDER BY name",
                )
                .unwrap();
            s.query_map([ty], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        assert_eq!(
            names("table"),
            ["entries", "meta", "ns_totals", "records"],
            "§2.2 rule 4's four tables, no fifth (no clean record)"
        );
        assert_eq!(names("index"), ["records_ns_state_seq"]);
        assert_eq!(
            names("trigger"),
            [
                "entries_totals_delete",
                "entries_totals_insert",
                "entries_totals_update",
                "records_totals_delete",
                "records_totals_insert",
                "records_totals_update",
            ]
        );
        for t in ["meta", "records", "entries", "ns_totals"] {
            let strict: i64 = c
                .query_row(
                    "SELECT strict FROM pragma_table_list WHERE name = ?1",
                    [t],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(strict, 1, "{t} is STRICT");
        }
        let cols: Vec<(String, String, bool)> = c
            .prepare(
                "SELECT name, type, \"notnull\" FROM pragma_table_info('records') ORDER BY cid",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let want = [
            ("ns", "TEXT", true),
            ("id", "TEXT", true),
            ("seq", "INTEGER", true),
            ("boot_id", "BLOB", true),
            ("mono_ms", "INTEGER", true),
            ("wall", "TEXT", false),
            ("state", "TEXT", true),
            ("body", "BLOB", true),
        ];
        assert_eq!(cols.len(), want.len());
        for ((n, t, nn), (wn, wt, wnn)) in cols.iter().zip(want) {
            assert_eq!((n.as_str(), t.as_str(), *nn), (wn, wt, wnn));
        }
        let idx: Vec<String> = c
            .prepare("SELECT name FROM pragma_index_info('records_ns_state_seq') ORDER BY seqno")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(idx, ["ns", "state", "seq"]);
        // `entries` carries `ns` (the trigger's key) and its primary key is (stream, seq)
        let pk: Vec<String> = c
            .prepare("SELECT name FROM pragma_table_info('entries') WHERE pk > 0 ORDER BY pk")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(pk, ["stream", "seq"]);
    }

    #[test]
    fn a_v1_file_is_only_read_and_a_newer_one_is_refused() {
        let c = mem();
        let changes = c.total_changes();
        migrate(&c).unwrap();
        assert_eq!(c.total_changes(), changes, "a v1 file migrates nothing");
        c.pragma_update(None, "user_version", 2).unwrap();
        match migrate(&c) {
            Err(StoreError::SchemaTooNew { found, supported }) => {
                assert_eq!((found, supported), (2, SCHEMA_VERSION));
            }
            other => panic!("a newer user_version must be refused, got {other:?}"),
        }
    }

    #[test]
    fn held_tells_a_store_never_written_from_a_versioned_and_an_unversioned_one() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")
            .unwrap();
        assert_eq!(held(&c).unwrap(), Held::Unwritten, "version 0, no object");
        migrate(&c).unwrap();
        assert_eq!(held(&c).unwrap(), Held::Versioned);
        c.pragma_update(None, "user_version", -1).unwrap();
        assert_eq!(held(&c).unwrap(), Held::Versioned, "migrate refuses it");
        let foreign = Connection::open_in_memory().unwrap();
        foreign
            .execute_batch("CREATE TABLE notes (x TEXT) STRICT;")
            .unwrap();
        assert_eq!(
            held(&foreign).unwrap(),
            Held::Unversioned,
            "version 0 with a table"
        );
    }

    #[test]
    fn every_write_takes_a_new_seq_from_meta() {
        let c = mem();
        let a = put(&c, "agent/proposals", "p1", "open", b"x");
        let b = put(&c, "agent/proposals", "p1", "decided", b"x");
        let e = append(&c, "agent/conversations", "agent/conversations/c1", b"m");
        assert_eq!((a, b, e), (1, 2, 3), "one allocator, a re-put included");
        let held: i64 = c
            .query_row("SELECT seq FROM records WHERE id = 'p1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(held, 2, "the record carries the seq of its last write");
        let next: i64 = c
            .query_row("SELECT value FROM meta WHERE key = ?1", [NEXT_SEQ], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(next, 4);
    }

    #[test]
    fn the_allocator_refuses_a_damaged_value_and_resumes_above_the_rows_when_lost() {
        let c = mem();
        put(&c, "heal/incidents", "i1", "open", b"a");
        append(&c, "heal/audit", "heal/audit", b"b");
        c.execute("UPDATE meta SET value = 'x' WHERE key = 'next_seq'", [])
            .unwrap();
        assert!(matches!(
            next_seq(&c),
            Err(StateError::Corrupt("meta.next_seq"))
        ));
        c.execute("UPDATE meta SET value = 0 WHERE key = 'next_seq'", [])
            .unwrap();
        assert!(matches!(next_seq(&c), Err(StateError::Corrupt(_))));
        c.execute(
            "UPDATE meta SET value = 9223372036854775807 WHERE key = 'next_seq'",
            [],
        )
        .unwrap();
        assert!(
            matches!(next_seq(&c), Err(StateError::Corrupt(_))),
            "no wrap"
        );
        c.execute("DELETE FROM meta WHERE key = 'next_seq'", [])
            .unwrap();
        assert_eq!(next_seq(&c).unwrap(), 3, "above the highest seq held");
        assert_eq!(next_seq(&c).unwrap(), 4);
    }

    #[test]
    fn ns_totals_count_length_of_body_plus_64_per_row_through_every_kind_of_write() {
        let c = mem();
        let ns = "agent/conversations";
        put(&c, ns, "c1", "open", &[7u8; 100]);
        append(&c, ns, "agent/conversations/c1", &[1u8; 30]);
        append(&c, ns, "agent/conversations/c1", b"");
        let want = NsTotal {
            rows: 3,
            bytes: 100 + 30 + 3 * ROW_OVERHEAD,
        };
        assert_eq!(total(&c, ns), Some(want));
        assert_eq!(recount(&c, ns), Some(want));
        // an upsert on the existing head (the UPDATE triggers)
        put(&c, ns, "c1", "open", &[7u8; 10]);
        assert_eq!(total(&c, ns), recount(&c, ns));
        assert_eq!(total(&c, ns).unwrap().bytes, 10 + 30 + 3 * ROW_OVERHEAD);
        // a state change alone counts nothing new
        let seq = next_seq(&c).unwrap();
        c.execute(
            "UPDATE records SET state = 'closed', seq = ?1 WHERE ns = ?2 AND id = 'c1'",
            params![seq, ns],
        )
        .unwrap();
        assert_eq!(total(&c, ns), recount(&c, ns));
        // a row moved to another namespace leaves one and joins the other
        c.execute("UPDATE entries SET ns = 'heal/audit' WHERE body = x''", [])
            .unwrap();
        assert_eq!(total(&c, ns), recount(&c, ns));
        assert_eq!(
            total(&c, "heal/audit"),
            Some(NsTotal {
                rows: 1,
                bytes: ROW_OVERHEAD
            })
        );
        // INSERT OR REPLACE deletes the old row through the DELETE trigger only with
        // recursive_triggers on, as the store's connection sets it
        c.pragma_update(None, "recursive_triggers", true).unwrap();
        let seq = next_seq(&c).unwrap();
        c.execute(
            "INSERT OR REPLACE INTO records (ns, id, seq, boot_id, mono_ms, wall, state, body)
                 VALUES (?1, 'c1', ?2, x'', 0, NULL, 'open', x'0102')",
            params![ns, seq],
        )
        .unwrap();
        assert_eq!(total(&c, ns), recount(&c, ns));
        // deletes, down to the namespace's row going with its last row
        c.execute("DELETE FROM entries WHERE ns = ?1", [ns])
            .unwrap();
        assert_eq!(total(&c, ns), recount(&c, ns));
        c.execute("DELETE FROM records WHERE ns = ?1", [ns])
            .unwrap();
        assert_eq!(total(&c, ns), None);
        c.execute("DELETE FROM entries", []).unwrap();
        assert!(read_totals(&c).unwrap().is_empty());
    }

    #[test]
    fn the_triggers_run_inside_the_writing_transaction() {
        let c = mem();
        let tx = c.unchecked_transaction().unwrap();
        put(&tx, "heal/incidents", "i1", "open", b"abc");
        assert_eq!(total(&tx, "heal/incidents").unwrap().rows, 1);
        tx.rollback().unwrap();
        assert_eq!(total(&c, "heal/incidents"), None, "rolled back with it");
        let next: i64 = c
            .query_row("SELECT value FROM meta WHERE key = 'next_seq'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(next, 1, "the seq it took went back too");
    }

    #[test]
    fn strict_tables_refuse_a_text_body_and_a_null_column() {
        let c = mem();
        let err = c
            .execute(
                "INSERT INTO records (ns, id, seq, boot_id, mono_ms, wall, state, body)
                     VALUES ('n', 'i', 1, x'', 0, NULL, 's', 'text')",
                [],
            )
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("cannot store TEXT value in BLOB column"));
        assert!(c
            .execute(
                "INSERT INTO entries (ns, stream, seq, body) VALUES (NULL, 's', 1, x'')",
                [],
            )
            .is_err());
        assert!(c
            .execute(
                "INSERT INTO records (ns, id, seq, boot_id, mono_ms, wall, state, body)
                     VALUES ('n', 'i', 2, x'', 0, NULL, 's', x'')",
                [],
            )
            .is_ok());
        assert!(
            c.execute(
                "INSERT INTO records (ns, id, seq, boot_id, mono_ms, wall, state, body)
                     VALUES ('n', 'j', 2, x'', 0, NULL, 's', x'')",
                [],
            )
            .is_err(),
            "a seq is held by one record only"
        );
    }
}
