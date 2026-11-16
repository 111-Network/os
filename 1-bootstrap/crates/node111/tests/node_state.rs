//! Layer 8 row D2 (`os/8-storage/README.md` §2.2 rules 1 to 9; G13 (1) and (13); §19): the
//! node-state store `<root>/node/state.db` on a scratch root, through its public handle, as the
//! daemon uses it: the modes and the key through `KeyProvider::key_for`, a plaintext file and a
//! wrong key refused, the store opened on use and closed on the idle pass (a turn spanning passes
//! keeps it open), a read-only open that changes neither the file nor its WAL (a first write cut
//! short included: a read never migrates), a non-empty WAL as the unclean signal, a newer
//! `user_version` unavailable with its reason, a damaged page closing it for the run, and the
//! daemon's start keying it from the identity it holds, with no fabric, closed by the real
//! housekeeping pass.
//!
//! A "turn" here is the write row D5 makes on `done`: the turn's messages appended to the
//! conversation's stream and its head re-put, each with a new `seq`, in one NORMAL transaction.

use mesh111::keys::{
    derive_tagged_key, KeyError, KeyMode, KeyProvider, ModeS, ModeT, StoreKey, NODE_STATE_KDF_TAG,
    STORE_KDF_TAG,
};
use mesh111::store::{open_encrypted, StoreError};
use node111::daemon::housekeeping::{self, PassCtx};
use node111::daemon::state::{
    self, companion, db_path, Durability, NodeState, Reason, StateError, StoreState,
};
use node111::daemon::DaemonLayout;
use rusqlite::params;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use zeroize::Zeroizing;

/// `5-messaging/test-vectors/layer8/node-state-kdf.json`, case 1: the Mode S secret, the NodeId
/// and the node-state key they derive under `111/node-state/v1`.
const VECTOR_SECRET: [u8; 32] = {
    let mut s = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        s[i] = i as u8;
        i += 1;
    }
    s
};
const VECTOR_NODE: [u8; 32] = {
    let mut n = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        n[i] = 0xa0 + (i % 16) as u8;
        i += 1;
    }
    n
};
const VECTOR_KEY: &str = "f7baa8403a789e49454168ac665980ade729156bea6c819310c509f8d240fb7e";

fn unhex(s: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

/// A Mode S custody that records every tag it is asked for.
struct Recording {
    inner: ModeS,
    asked: Mutex<Vec<Vec<u8>>>,
}

impl Recording {
    fn vector() -> Recording {
        Recording {
            inner: ModeS::new(Zeroizing::new(VECTOR_SECRET), VECTOR_NODE),
            asked: Mutex::new(Vec::new()),
        }
    }
}

impl KeyProvider for Recording {
    fn mode(&self) -> KeyMode {
        self.inner.mode()
    }
    fn store_key(&self) -> Result<StoreKey, KeyError> {
        self.asked.lock().unwrap().push(STORE_KDF_TAG.to_vec());
        self.inner.store_key()
    }
    fn key_for(&self, tag: &[u8]) -> Result<StoreKey, KeyError> {
        self.asked.lock().unwrap().push(tag.to_vec());
        self.inner.key_for(tag)
    }
}

fn custody(seed: u8) -> ModeS {
    ModeS::new(Zeroizing::new([seed; 32]), [seed.wrapping_add(1); 32])
}

fn store(root: &Path, seed: u8) -> Arc<NodeState> {
    Arc::new(NodeState::new(root, &custody(seed)))
}

/// One turn (row D5's write on `done`): `msgs` appended to the conversation's stream and its head
/// re-put, each with a new `seq`, in one NORMAL transaction. Returns the head's `seq`.
fn turn(s: &NodeState, conv: &str, msgs: &[&[u8]]) -> i64 {
    s.write(Durability::Normal, |w| -> Result<i64, StateError> {
        let stream = format!("agent/conversations/{conv}");
        for m in msgs {
            let seq = w.next_seq()?;
            w.tx().execute(
                "INSERT INTO entries (ns, stream, seq, body)
                     VALUES ('agent/conversations', ?1, ?2, ?3)",
                params![stream, seq, m],
            )?;
        }
        let h = w.stamp()?;
        w.tx().execute(
            "INSERT INTO records (ns, id, seq, boot_id, mono_ms, wall, state, body)
                 VALUES ('agent/conversations', ?1, ?2, ?3, ?4, ?5, 'open', ?6)
                 ON CONFLICT (ns, id) DO UPDATE SET seq = excluded.seq,
                     boot_id = excluded.boot_id, mono_ms = excluded.mono_ms,
                     wall = excluded.wall, body = excluded.body",
            params![
                conv,
                h.seq,
                h.boot_id,
                h.mono_ms,
                h.wall,
                br#"{"title":"a conversation"}"#.as_slice()
            ],
        )?;
        Ok(h.seq)
    })
    .unwrap()
}

/// A history read (row D5's `GET /api/v1/chat/history`): the stream's messages, oldest first.
fn history(s: &NodeState, conv: &str) -> Result<Vec<Vec<u8>>, StateError> {
    s.read(|c| {
        let mut st =
            c.prepare("SELECT body FROM entries WHERE stream = ?1 ORDER BY seq LIMIT 1000")?;
        let rows = st.query_map([format!("agent/conversations/{conv}")], |r| r.get(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    })
}

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o7777
}

/// Size and modification time (to the nanosecond), or `None` when the file is absent.
fn stamp_of(p: &Path) -> Option<(u64, i64, i64)> {
    std::fs::metadata(p)
        .ok()
        .map(|m| (m.len(), m.mtime(), m.mtime_nsec()))
}

/// The descriptors this process holds on files under `dir` (§19 "open files").
fn open_fds_under(dir: &Path) -> Vec<PathBuf> {
    let dir = dir.canonicalize().unwrap();
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|e| std::fs::read_link(e.path()).ok())
        .filter(|p| p.starts_with(&dir))
        .collect()
}

fn wal(root: &Path) -> PathBuf {
    companion(&db_path(root), "-wal")
}

fn shm(root: &Path) -> PathBuf {
    companion(&db_path(root), "-shm")
}

/// This process's resident set (`VmRSS`, kB), 0 when unreadable.
fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("VmRSS:"))
                .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        })
        .unwrap_or(0)
}

/// Two passes with no use between them: the store is closed after them.
fn idle_passes(s: &NodeState) {
    s.on_pass();
    s.on_pass();
}

#[test]
fn g13_1_modes_and_the_key_through_key_for() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let keys = Recording::vector();
    let s = Arc::new(NodeState::new(root, &keys));
    assert_eq!(
        *keys.asked.lock().unwrap(),
        vec![NODE_STATE_KDF_TAG.to_vec()],
        "the key comes from key_for(\"111/node-state/v1\") alone, never store_key()"
    );
    assert!(
        !root.join(state::DIR).exists(),
        "nothing is created before a use"
    );
    turn(&s, "c1", &[b"hello", b"hi there"]);
    // §2.2 rule 1: node/ 0700, the file and its companions 0600, owned by the daemon's user
    assert_eq!(mode(&root.join(state::DIR)), 0o700);
    for p in [db_path(root), wal(root), shm(root)] {
        assert!(p.exists(), "{p:?} while open");
        assert_eq!(mode(&p), 0o600, "{p:?}");
        assert_eq!(
            std::fs::metadata(&p).unwrap().uid(),
            std::fs::metadata(root).unwrap().uid(),
            "{p:?} is the daemon user's"
        );
    }
    // §19: +3 open files while it is open (state.db, -wal, -shm), none once closed
    assert_eq!(open_fds_under(root).len(), 3, "{:?}", open_fds_under(root));
    idle_passes(&s);
    assert!(!s.is_open());
    assert!(
        open_fds_under(root).is_empty(),
        "{:?}",
        open_fds_under(root)
    );
    // the file opens with the vector's key: KDF(store_secret, "111/node-state/v1" ‖ NodeId)
    let key = StoreKey::from_bytes(unhex(VECTOR_KEY));
    let conn = open_encrypted(&db_path(root), &key).unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2);
    drop(conn);
    let same = derive_tagged_key(NODE_STATE_KDF_TAG, &VECTOR_SECRET, &VECTOR_NODE).unwrap();
    assert_eq!(same.as_bytes(), key.as_bytes());
    // and not with the fabric store's key of the same custody
    let fabric = keys.inner.store_key().unwrap();
    assert!(matches!(
        open_encrypted(&db_path(root), &fabric),
        Err(StoreError::WrongKey)
    ));
}

#[test]
fn g13_1_a_plaintext_file_is_refused_and_left_as_it_is() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    std::fs::create_dir(root.join(state::DIR)).unwrap();
    std::fs::set_permissions(
        root.join(state::DIR),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    {
        let plain = rusqlite::Connection::open(db_path(root)).unwrap();
        plain
            .execute_batch(
                "CREATE TABLE records (x); INSERT INTO records VALUES ('a plaintext row');",
            )
            .unwrap();
    }
    let before = std::fs::read(db_path(root)).unwrap();
    let when = stamp_of(&db_path(root));
    let s = store(root, 1);
    for _ in 0..2 {
        match history(&s, "c1") {
            Err(StateError::Unavailable(Reason::Refused(why))) => {
                assert!(why.contains("this node's key"), "{why}");
            }
            other => panic!("a plaintext file must be refused, got {other:?}"),
        }
        assert!(matches!(
            s.write(Durability::Full, |_| -> Result<(), StateError> { Ok(()) }),
            Err(StateError::Unavailable(Reason::Refused(_)))
        ));
    }
    let st = s.status();
    assert_eq!(st.state, StoreState::Unavailable);
    assert!(st
        .reason
        .unwrap()
        .starts_with("node/state.db does not open: "));
    assert_eq!(
        std::fs::read(db_path(root)).unwrap(),
        before,
        "never overwritten"
    );
    assert_eq!(stamp_of(&db_path(root)), when);
    assert!(!wal(root).exists());
}

#[test]
fn g13_1_a_wrong_key_fails_on_page_1_and_writes_nothing() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let a = store(root, 1);
    turn(&a, "c1", &[b"mine"]);
    a.stop();
    let before = std::fs::read(db_path(root)).unwrap();
    let when = stamp_of(&db_path(root));
    // another node's key: refused at the first read, which decrypts page 1
    assert!(matches!(
        open_encrypted(
            &db_path(root),
            &custody(2).key_for(NODE_STATE_KDF_TAG).unwrap()
        ),
        Err(StoreError::WrongKey)
    ));
    let b = store(root, 2);
    assert!(matches!(
        history(&b, "c1"),
        Err(StateError::Unavailable(Reason::Refused(_)))
    ));
    assert!(matches!(
        b.load_totals(),
        Err(StateError::Unavailable(Reason::Refused(_)))
    ));
    assert_eq!(b.status().state, StoreState::Unavailable);
    assert_eq!(std::fs::read(db_path(root)).unwrap(), before);
    assert_eq!(stamp_of(&db_path(root)), when);
    assert!(!wal(root).exists() && !shm(root).exists());
    // the right key still reads it
    let a = store(root, 1);
    assert_eq!(history(&a, "c1").unwrap(), vec![b"mine".to_vec()]);
}

#[test]
fn mode_t_and_no_identity_leave_it_unavailable_with_no_key() {
    let t = tempfile::tempdir().unwrap();
    for s in [
        NodeState::new(t.path(), &ModeT),
        NodeState::without_key(t.path(), "no identity is loaded"),
    ] {
        assert!(matches!(
            history(&s, "c1"),
            Err(StateError::Unavailable(Reason::NoKey(_)))
        ));
        let st = s.status();
        assert_eq!(st.state, StoreState::Unavailable);
        assert!(st.reason.unwrap().contains("has no key"));
    }
    assert!(!t.path().join(state::DIR).exists());
}

#[test]
fn opened_on_use_and_closed_on_the_first_idle_pass() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let s = store(root, 3);
    assert_eq!(s.status().state, StoreState::Closed);
    s.on_pass();
    assert!(!s.is_open(), "a pass opens nothing");
    turn(&s, "c1", &[b"a"]);
    assert_eq!(s.status().state, StoreState::Open);
    s.on_pass();
    assert!(s.is_open(), "used since the previous pass");
    s.on_pass();
    assert!(
        !s.is_open(),
        "the first pass with no use since the previous one"
    );
    assert_eq!(s.status().state, StoreState::Closed);
    assert!(
        !wal(root).exists(),
        "the close's TRUNCATE checkpoint leaves no WAL"
    );
    assert!(!shm(root).exists());
    assert!(open_fds_under(root).is_empty());
    // a read is a use too
    assert_eq!(history(&s, "c1").unwrap().len(), 1);
    assert!(s.is_open());
    idle_passes(&s);
    assert!(!s.is_open());
}

#[test]
fn a_turn_spanning_two_passes_keeps_it_open() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let s = store(root, 4);
    turn(&s, "c1", &[b"first"]);
    idle_passes(&s);
    assert!(!s.is_open());
    // the turn starts: its history read, then a model answer that takes passes
    let in_flight = s.hold();
    assert_eq!(history(&s, "c1").unwrap().len(), 1);
    for _ in 0..3 {
        s.on_pass();
        assert!(s.is_open(), "a turn in flight is a use on every pass");
    }
    // `done`: the turn's messages appended, then the turn ends
    turn(&s, "c1", &[b"question", b"answer"]);
    drop(in_flight);
    s.on_pass();
    assert!(s.is_open(), "the turn's end is a use");
    s.on_pass();
    assert!(!s.is_open());
    assert_eq!(history(&s, "c1").unwrap().len(), 3);
}

#[test]
fn g13_13_a_read_only_open_leaves_state_db_and_its_wal_unchanged() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let s = store(root, 5);
    turn(&s, "c1", &[b"kept", b"also kept"]);
    s.stop();
    drop(s);
    let db_before = stamp_of(&db_path(root)).expect("state.db");
    assert_eq!(stamp_of(&wal(root)), None, "a clean close leaves no WAL");
    // a daemon start: the status read (the totals not known yet), then a history read
    let s = store(root, 5);
    assert_eq!(s.totals(), None);
    let totals = s.load_totals().unwrap();
    assert_eq!(totals["agent/conversations"].rows, 3);
    assert!(s.is_open());
    assert_eq!(history(&s, "c1").unwrap().len(), 2);
    assert_eq!(
        stamp_of(&db_path(root)),
        Some(db_before),
        "nothing written while open"
    );
    assert!(
        stamp_of(&wal(root)).is_none_or(|(len, _, _)| len == 0),
        "nothing in the WAL"
    );
    idle_passes(&s);
    assert!(!s.is_open());
    assert_eq!(
        stamp_of(&db_path(root)),
        Some(db_before),
        "nor at the close"
    );
    assert_eq!(stamp_of(&wal(root)), None);
    assert!(!s.take_unclean());
    assert_eq!(s.status().file_bytes, Some(db_before.0));
}

#[test]
fn a_non_empty_wal_is_the_unclean_signal_and_a_clean_close_leaves_none() {
    let t = tempfile::tempdir().unwrap();
    let (live, crashed) = (t.path().join("live"), t.path().join("crashed"));
    std::fs::create_dir(&live).unwrap();
    let s = store(&live, 6);
    turn(&s, "c1", &[b"written before the power cut"]);
    // the power cut: the files as the disk holds them while the store is open
    std::fs::create_dir_all(crashed.join(state::DIR)).unwrap();
    std::fs::copy(db_path(&live), db_path(&crashed)).unwrap();
    std::fs::copy(wal(&live), wal(&crashed)).unwrap();
    assert!(std::fs::metadata(wal(&crashed)).unwrap().len() > 0);
    let after = store(&crashed, 6);
    assert!(!after.take_unclean(), "no signal before an open");
    assert_eq!(
        history(&after, "c1").unwrap(),
        vec![b"written before the power cut".to_vec()],
        "the WAL's commit is kept"
    );
    assert!(
        after.take_unclean(),
        "a non-empty state.db-wal before the open"
    );
    assert!(!after.take_unclean(), "handed over once");
    // its clean close leaves no WAL, so the next open finds no signal
    idle_passes(&after);
    assert!(!wal(&crashed).exists());
    history(&after, "c1").unwrap();
    assert!(!after.take_unclean());
    idle_passes(&after);
    // and the live store's own clean close leaves none either
    idle_passes(&s);
    assert!(!wal(&live).exists());
    let again = store(&live, 6);
    history(&again, "c1").unwrap();
    assert!(!again.take_unclean());
}

#[test]
fn a_newer_user_version_is_unavailable_with_the_reason_and_the_file_kept() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let s = store(root, 7);
    turn(&s, "c1", &[b"a"]);
    s.stop();
    {
        // a later release migrated this file to schema 2
        let key = custody(7).key_for(NODE_STATE_KDF_TAG).unwrap();
        let conn = open_encrypted(&db_path(root), &key).unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
    }
    let before = std::fs::read(db_path(root)).unwrap();
    let when = stamp_of(&db_path(root));
    let s = store(root, 7);
    match history(&s, "c1") {
        Err(StateError::Unavailable(Reason::NewerSchema { found, supported })) => {
            assert_eq!((found, supported), (2, 1));
        }
        other => panic!("a newer user_version must be refused, got {other:?}"),
    }
    let st = s.status();
    assert_eq!(st.state, StoreState::Unavailable);
    assert_eq!(
        st.reason.as_deref(),
        Some(
            "node/state.db was written by a newer release (schema 2; this release reads up \
             to 1)"
        )
    );
    // every caller reads it, and none touches the file again
    assert!(matches!(
        s.write(Durability::Full, |_| -> Result<(), StateError> { Ok(()) }),
        Err(StateError::Unavailable(Reason::NewerSchema { .. }))
    ));
    assert!(matches!(
        s.load_totals(),
        Err(StateError::Unavailable(Reason::NewerSchema { .. }))
    ));
    assert!(!s.is_open());
    assert_eq!(
        std::fs::read(db_path(root)).unwrap(),
        before,
        "never downgraded"
    );
    assert_eq!(stamp_of(&db_path(root)), when);
    assert!(
        !wal(root).exists() && !shm(root).exists(),
        "the open's own -wal and -shm gone"
    );
    // a newer release's crash: its last commit only in the WAL, which the refusal keeps as it is
    let crashed = t.path().join("crashed");
    std::fs::create_dir_all(crashed.join(state::DIR)).unwrap();
    {
        let key = custody(7).key_for(NODE_STATE_KDF_TAG).unwrap();
        let conn = open_encrypted(&db_path(root), &key).unwrap();
        conn.execute("INSERT INTO meta (key, value) VALUES ('newer', 1)", [])
            .unwrap();
        std::fs::copy(db_path(root), db_path(&crashed)).unwrap();
        std::fs::copy(wal(root), wal(&crashed)).unwrap();
    }
    let db = std::fs::read(db_path(&crashed)).unwrap();
    let log = std::fs::read(wal(&crashed)).unwrap();
    assert!(!log.is_empty());
    let s = store(&crashed, 7);
    assert!(matches!(
        history(&s, "c1"),
        Err(StateError::Unavailable(Reason::NewerSchema { .. }))
    ));
    assert!(s.take_unclean(), "the signal was seen");
    assert_eq!(
        std::fs::read(db_path(&crashed)).unwrap(),
        db,
        "nothing checkpointed in"
    );
    assert_eq!(
        std::fs::read(wal(&crashed)).unwrap(),
        log,
        "the WAL kept for its release"
    );
}

/// `bytes` in a new file `p`, 0600 as the store makes its files.
fn write_0600(p: &Path, bytes: &[u8]) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(p)
        .unwrap();
    f.write_all(bytes).unwrap();
}

/// A first write cut short (§2.2 rules 7 and 9): mesh111's open made `state.db` and the v1
/// migration did not commit, a power cut or a full disk. The file is empty (created, then nothing),
/// holds the one header page that open writes, or holds that page with the migration's frames in
/// its WAL and their commit frame cut off. A status read and a history read see an empty store and
/// leave the file and its WAL as they were (a cut WAL is still the unclean signal); the next write
/// migrates the file.
#[test]
fn g13_13_a_store_whose_first_write_did_not_commit_reads_empty_and_is_left_as_it_is() {
    let t = tempfile::tempdir().unwrap();
    let seed = 9;
    let key = custody(seed).key_for(NODE_STATE_KDF_TAG).unwrap();
    let roots: Vec<PathBuf> = ["empty", "header", "cut"]
        .iter()
        .map(|n| t.path().join(n))
        .collect();
    for root in &roots {
        std::fs::create_dir_all(root.join(state::DIR)).unwrap();
        std::fs::set_permissions(
            root.join(state::DIR),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
    }
    // mesh111's open creates the file 0600 before SQLite touches it
    write_0600(&db_path(&roots[0]), b"");
    // then SQLite writes its header page (auto_vacuum, WAL), and no migration ran
    drop(open_encrypted(&db_path(&roots[1]), &key).unwrap());
    assert_eq!(std::fs::metadata(db_path(&roots[1])).unwrap().len(), 4096);
    // that page on disk, the migration's frames in the WAL without their commit frame
    {
        let whole = t.path().join("whole");
        let conn = open_encrypted(&db_path(&whole), &key).unwrap();
        write_0600(
            &db_path(&roots[2]),
            &std::fs::read(db_path(&whole)).unwrap(),
        );
        state::schema::migrate(&conn).unwrap();
        let log = std::fs::read(wal(&whole)).unwrap();
        const FRAME: usize = 24 + 4096;
        assert!(
            log.len() >= 32 + 2 * FRAME && (log.len() - 32).is_multiple_of(FRAME),
            "{}",
            log.len()
        );
        write_0600(&wal(&roots[2]), &log[..log.len() - FRAME]);
    }
    for root in &roots {
        let db = std::fs::read(db_path(root)).unwrap();
        let when = stamp_of(&db_path(root));
        let log = std::fs::read(wal(root)).ok();
        let s = store(root, seed);
        // a daemon start: the status read (the totals not known yet), then a history read
        assert!(s.load_totals().unwrap().is_empty(), "{root:?}");
        assert!(history(&s, "c1").unwrap().is_empty(), "{root:?}");
        assert!(!s.is_open(), "{root:?}: nothing kept open");
        let st = s.status();
        assert_eq!(
            (st.state, st.reason),
            (StoreState::Closed, None),
            "{root:?}"
        );
        assert_eq!(
            std::fs::read(db_path(root)).unwrap(),
            db,
            "{root:?}: not migrated by a read"
        );
        assert_eq!(stamp_of(&db_path(root)), when, "{root:?}");
        assert_eq!(
            std::fs::read(wal(root)).ok(),
            log,
            "{root:?}: its WAL as it was"
        );
        assert_eq!(
            s.take_unclean(),
            log.is_some(),
            "{root:?}: a cut WAL is the unclean signal"
        );
        if log.is_none() {
            assert!(!shm(root).exists(), "{root:?}");
        }
        assert!(open_fds_under(root).is_empty(), "{root:?}");
        // the next write migrates it
        turn(&s, "c1", &[b"the first turn that commits"]);
        assert_eq!(
            history(&s, "c1").unwrap(),
            vec![b"the first turn that commits".to_vec()],
            "{root:?}"
        );
        idle_passes(&s);
        assert!(!s.is_open());
        assert!(!wal(root).exists(), "{root:?}");
    }
}

/// A damaged page the open reads (§2.2 rule 8): `ns_totals`' page, read at the first open after a
/// start, fails its HMAC. The store is refused for the run, with the reason every caller reads, and
/// nothing is written; it is not opened again, even once the file reads whole (row D18 moves a
/// damaged file aside, §13.7). The next start reads the whole file.
#[test]
fn a_damaged_page_the_open_reads_closes_the_store_for_the_run() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let seed = 10;
    let s = store(root, seed);
    turn(&s, "c1", &[b"a message"]);
    s.stop();
    drop(s);
    let page: i64 = {
        let key = custody(seed).key_for(NODE_STATE_KDF_TAG).unwrap();
        let conn = open_encrypted(&db_path(root), &key).unwrap();
        conn.query_row(
            "SELECT rootpage FROM sqlite_master WHERE name = 'ns_totals'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    let whole = std::fs::read(db_path(root)).unwrap();
    let mut damaged = whole.clone();
    damaged[usize::try_from(page - 1).unwrap() * 4096 + 2048] ^= 0xff;
    std::fs::write(db_path(root), &damaged).unwrap();
    let when = stamp_of(&db_path(root));
    let s = store(root, seed);
    match s.load_totals() {
        Err(StateError::Unavailable(r @ Reason::Refused(_))) => assert!(r.is_sticky()),
        other => panic!("a damaged page must close the store for the run, got {other:?}"),
    }
    let st = s.status();
    assert_eq!(st.state, StoreState::Unavailable);
    assert!(st
        .reason
        .unwrap()
        .starts_with("node/state.db does not open: "));
    assert_eq!(
        std::fs::read(db_path(root)).unwrap(),
        damaged,
        "nothing written"
    );
    assert_eq!(stamp_of(&db_path(root)), when);
    assert!(!wal(root).exists() && !shm(root).exists());
    // closed for the run: every later use reads the reason and opens nothing
    std::fs::write(db_path(root), &whole).unwrap();
    assert!(matches!(
        history(&s, "c1"),
        Err(StateError::Unavailable(Reason::Refused(_)))
    ));
    assert!(matches!(
        s.write(Durability::Normal, |_| -> Result<(), StateError> { Ok(()) }),
        Err(StateError::Unavailable(Reason::Refused(_)))
    ));
    assert!(matches!(
        s.load_totals(),
        Err(StateError::Unavailable(Reason::Refused(_)))
    ));
    assert!(!s.is_open() && !wal(root).exists());
    // the next start opens the whole file
    let again = store(root, seed);
    assert_eq!(history(&again, "c1").unwrap(), vec![b"a message".to_vec()]);
}

/// The daemon's start (`daemon/mod.rs`): the key derives from the identity the daemon holds, with
/// no fabric started; the store is current, the real housekeeping pass (§4.3 step 7) closes it,
/// and the start's end closes it too. No identity: no key. (The one test of this file that makes
/// a store current.)
#[test]
fn the_daemon_start_keys_it_from_its_identity_and_the_housekeeping_pass_closes_it() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let layout = DaemonLayout {
        root: root.to_path_buf(),
        etc: root.join("etc"),
    };
    let id = node111::identity::generate(&root.join("identity")).unwrap();
    let started = state::start(&layout, Some(&id));
    let s = Arc::clone(started.store());
    assert!(state::current().is_some_and(|c| Arc::ptr_eq(&c, &s)));
    assert!(!root.join(state::DIR).exists(), "the start opens nothing");
    turn(&s, "c1", &[b"through the daemon's store"]);
    let mut ctx = PassCtx::default();
    housekeeping::pass(&mut ctx);
    assert!(s.is_open());
    housekeeping::pass(&mut ctx);
    assert!(!s.is_open(), "the housekeeping pass's step 7 closed it");
    // the key: KDF(store_secret of the identity, "111/node-state/v1" ‖ NodeId)
    let secret =
        node111::daemon::fabric::sealer::store_secret_from(&id.keyset().to_bytes()).unwrap();
    let key = derive_tagged_key(NODE_STATE_KDF_TAG, &secret, &id.node_id.to_bytes()).unwrap();
    let conn = open_encrypted(&db_path(root), &key).unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
    drop(conn);
    // the daemon's end closes it and leaves no WAL
    history(&s, "c1").unwrap();
    assert!(s.is_open());
    drop(started);
    assert!(!s.is_open());
    assert!(!wal(root).exists());
    assert!(state::current().is_none());
    // no identity loaded: no key, the store unavailable with the reason
    let keyless = state::start(&layout, None);
    let st = keyless.store().status();
    assert_eq!(st.state, StoreState::Unavailable);
    assert_eq!(
        st.reason.as_deref(),
        Some("the node-state store has no key: no identity is loaded")
    );
}

/// §19 figures for row D2 (printed; run alone with `--nocapture` for clean numbers): the open
/// files while open, the WAL one turn writes, the file's size, the open and the close on this
/// box. The asserted bounds are loose regression bounds, not the figures.
#[test]
fn the_figures_of_a_turn() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let s = store(root, 8);
    // a conversation of 20 turns, then the store closed (a clean file, no WAL)
    for _ in 0..20 {
        turn(&s, "c1", &[&[b'q'; 300], &[b'a'; 1500]]);
    }
    idle_passes(&s);
    let file = std::fs::metadata(db_path(root)).unwrap().len();
    // the open (a history read) and one more turn
    let rss_closed = rss_kib();
    let t0 = Instant::now();
    history(&s, "c1").unwrap();
    let open_ms = t0.elapsed().as_secs_f64() * 1e3;
    let rss_open = rss_kib();
    let fds = open_fds_under(root).len();
    let wal_open = std::fs::metadata(wal(root)).map(|m| m.len()).unwrap_or(0);
    turn(&s, "c1", &[&[b'q'; 300], &[b'a'; 1500]]);
    let wal_turn = std::fs::metadata(wal(root)).unwrap().len() - wal_open;
    s.on_pass();
    let t1 = Instant::now();
    s.on_pass();
    let close_ms = t1.elapsed().as_secs_f64() * 1e3;
    assert!(!s.is_open());
    // WAL frames: a 32-byte header, then 24 + 4096 bytes per page written
    let frames = wal_turn.saturating_sub(if wal_open == 0 { 32 } else { 0 }) / (24 + 4096);
    eprintln!(
        "D2 figures: open files while open {fds}; state.db after 20 turns {file} B; \
         one turn's WAL {wal_turn} B ({frames} page frames); open {open_ms:.2} ms; \
         close {close_ms:.2} ms (this box, not an SD card); VmRSS closed {rss_closed} kB, \
         open after a history read {rss_open} kB"
    );
    assert_eq!(fds, 3);
    assert!(wal_turn > 0 && wal_turn < 128 * 1024, "{wal_turn}");
}
