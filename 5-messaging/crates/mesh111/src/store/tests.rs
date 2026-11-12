//! Store tests (§4, §10, §11). In-module because `crates/mesh111/tests/**`
//! belongs to task F1; these exercise the public `Store` API end to end on a
//! real SQLCipher file in a temp directory.

use std::path::Path;

use super::*;
use crate::blobs::{BlobError, BlobHash, ObjectKey};
use crate::keys::{derive_store_key, KeyError, ModeS, ModeT};

const ME: [u8; 32] = [0xA0; 32];
const PEER: [u8; 32] = [0xB0; 32];
const PEER2: [u8; 32] = [0xB1; 32];

fn key() -> StoreKey {
    derive_store_key(&[1; 32], &ME).unwrap()
}

fn params(announce_last: u64) -> OpenParams {
    OpenParams {
        announce_last,
        restore_floor_set: false,
    }
}

fn open(dir: &Path, announce_last: u64) -> (Store, EpochDecision) {
    Store::open(dir, &key(), &params(announce_last)).unwrap()
}

/// A fresh store with epoch 1 recorded.
fn ready() -> (tempfile::TempDir, Store) {
    let t = tempfile::tempdir().unwrap();
    let (mut s, d) = open(t.path(), 1);
    assert!(matches!(d, EpochDecision::Mint(r) if r.unclean));
    s.record_epoch(1).unwrap();
    (t, s)
}

fn own<'a>(peer: [u8; 32], stream: u64, body: &'a [u8]) -> NewMessage<'a> {
    NewMessage {
        peer,
        sender: ME,
        stream,
        kind: 0x01,
        class: Class::Human,
        body,
        blob_refs: None,
        reason: None,
        pinned: false,
    }
}

fn inbound(peer: [u8; 32], body: &[u8]) -> NewMessage<'_> {
    NewMessage {
        sender: peer,
        ..own(peer, 0, body)
    }
}

fn mint(r: MintReasons) -> EpochDecision {
    EpochDecision::Mint(r)
}

/// Allocate through the only allocation path (n is committed with content).
fn alloc(s: &mut Store, peer: &[u8; 32], stream: u64) -> Result<Seq> {
    s.queue_own_message(&own(*peer, stream, b"x"))
        .map(|(_, seq)| seq)
}

// ---------------------------------------------------------------------------
// §10.1 engine
// ---------------------------------------------------------------------------

#[test]
fn open_applies_pragmas_and_encrypts_the_file() {
    let (t, s) = ready();
    let q = |p: &str| -> String {
        s.conn
            .query_row(&format!("PRAGMA {p}"), [], |r| {
                r.get::<_, rusqlite::types::Value>(0)
            })
            .map(|v| match v {
                rusqlite::types::Value::Integer(i) => i.to_string(),
                rusqlite::types::Value::Text(t) => t,
                other => format!("{other:?}"),
            })
            .unwrap()
    };
    assert_eq!(q("journal_mode"), "wal");
    assert_eq!(q("auto_vacuum"), "2"); // INCREMENTAL
    assert_eq!(q("secure_delete"), "1");
    assert_eq!(q("temp_store"), "2"); // MEMORY
    assert_eq!(q("cipher_page_size"), "4096"); // SQLCipher 4 format
    assert_eq!(q("cipher_hmac_algorithm"), "HMAC_SHA512");
    assert_eq!(q("foreign_keys"), "1");
    assert_eq!(s.synchronous_level().unwrap(), 1); // NORMAL between FULL txs
    assert!(!q("cipher_version").is_empty());
    let db = t.path().join(DB_FILE);
    let mode = fs::metadata(&db).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    s.close(1).unwrap();
    let head = fs::read(&db).unwrap();
    assert!(head.len() >= 4096);
    assert_ne!(
        &head[..16],
        b"SQLite format 3\0",
        "store.db must be encrypted"
    );
}

#[test]
fn sequence_transactions_run_at_synchronous_full() {
    let (_t, mut s) = ready();
    let inside = s
        .with_tx(Durability::Full, |tx| pragma_int(tx, "synchronous"))
        .unwrap();
    assert_eq!(inside, 2); // FULL while allocating / promising
    let routine = s
        .with_tx(Durability::Normal, |tx| pragma_int(tx, "synchronous"))
        .unwrap();
    assert_eq!(routine, 1);
    // A failing FULL transaction still restores NORMAL.
    let err: Result<()> = s.with_tx(Durability::Full, |_| Err(StoreError::NotFound));
    assert!(err.is_err());
    assert_eq!(s.synchronous_level().unwrap(), 1);
}

#[test]
fn wrong_key_and_mode_t_are_refused() {
    let (t, s) = ready();
    s.close(1).unwrap();
    let other = derive_store_key(&[2; 32], &ME).unwrap();
    assert!(matches!(
        Store::open(t.path(), &other, &params(1)),
        Err(StoreError::WrongKey)
    ));
    assert!(matches!(
        Store::open_with(t.path(), &ModeT, &params(1)),
        Err(StoreError::Key(KeyError::Unsupported(_)))
    ));
    // Mode S over the same secret + NodeId opens it.
    let p = ModeS::new(zeroize::Zeroizing::new([1; 32]), ME);
    let (s, d) = Store::open_with(t.path(), &p, &params(1)).unwrap();
    assert_eq!(d, EpochDecision::Keep { epoch: 1 });
    s.close(1).unwrap();
}

#[test]
fn schema_newer_than_build_is_refused() {
    let (t, s) = ready();
    s.conn.pragma_update(None, "user_version", 99).unwrap();
    s.close(1).unwrap();
    assert!(matches!(
        Store::open(t.path(), &key(), &params(1)),
        Err(StoreError::SchemaTooNew { found: 99, .. })
    ));
}

// ---------------------------------------------------------------------------
// §4 epochs, clean record, restore_pending
// ---------------------------------------------------------------------------

#[test]
fn fresh_store_requires_a_mint_before_any_allocation() {
    let t = tempfile::tempdir().unwrap();
    let (mut s, d) = open(t.path(), 1);
    assert_eq!(
        d,
        mint(MintReasons {
            unclean: true,
            announce_mismatch: true,
            ..Default::default()
        })
    );
    assert!(matches!(
        alloc(&mut s, &PEER, 0),
        Err(StoreError::EpochNotMinted)
    ));
    assert!(matches!(
        s.queue_own_message(&own(PEER, 0, b"hi")),
        Err(StoreError::EpochNotMinted)
    ));
    assert_eq!(s.message_count().unwrap(), 0);
}

#[test]
fn clean_close_and_reopen_keeps_the_epoch_and_n() {
    let (t, mut s) = ready();
    assert_eq!(alloc(&mut s, &PEER, 0).unwrap(), Seq { epoch: 1, n: 1 });
    assert_eq!(alloc(&mut s, &PEER, 0).unwrap(), Seq { epoch: 1, n: 2 });
    assert_eq!(s.synchronous_level().unwrap(), 1);
    s.close(1).unwrap();
    let (mut s, d) = open(t.path(), 1);
    assert_eq!(d, EpochDecision::Keep { epoch: 1 });
    assert_eq!(s.epoch_state(), EpochState::Ready { epoch: 1 });
    assert_eq!(alloc(&mut s, &PEER, 0).unwrap(), Seq { epoch: 1, n: 3 });
    s.close(1).unwrap();
}

#[test]
fn unclean_shutdown_forces_an_epoch_jump() {
    let (t, s) = ready();
    s.close(1).unwrap();
    let (mut s, d) = open(t.path(), 1);
    assert_eq!(d, EpochDecision::Keep { epoch: 1 });
    alloc(&mut s, &PEER, 0).unwrap();
    alloc(&mut s, &PEER, 0).unwrap();
    drop(s); // crash: no clean record
    let (mut s, d) = open(t.path(), 1);
    assert_eq!(
        d,
        mint(MintReasons {
            unclean: true,
            ..Default::default()
        })
    );
    assert!(alloc(&mut s, &PEER, 0).is_err());
    // The caller mints (announce counter 2) and records it.
    s.record_epoch(2).unwrap();
    assert_eq!(alloc(&mut s, &PEER, 0).unwrap(), Seq { epoch: 2, n: 1 });
    // Epochs only grow.
    assert!(matches!(
        s.record_epoch(2),
        Err(StoreError::EpochNotIncreasing {
            new: 2,
            recorded: 2
        })
    ));
    assert!(s.record_epoch(1).is_err());
    s.close(2).unwrap();
    let (_, d) = open(t.path(), 2);
    assert_eq!(d, EpochDecision::Keep { epoch: 2 });
}

#[test]
fn backup_restored_onto_a_live_identity_mints_although_closed_cleanly() {
    let (t, s) = ready();
    s.close(1).unwrap();
    // Back up the cleanly closed store.
    let backup = tempfile::tempdir().unwrap();
    for e in fs::read_dir(t.path()).unwrap() {
        let e = e.unwrap();
        if e.file_name().to_string_lossy().starts_with(DB_FILE) {
            fs::copy(e.path(), backup.path().join(e.file_name())).unwrap();
        }
    }
    // The live node carries on: a later run mints epoch 2 (announce_last 2).
    let (mut s, _) = open(t.path(), 1);
    s.record_epoch(2).unwrap();
    s.close(2).unwrap();
    // Restore the backup onto the live identity (announce_last = 2).
    let (_, d) = open(backup.path(), 2);
    assert_eq!(
        d,
        mint(MintReasons {
            announce_mismatch: true,
            ..Default::default()
        })
    );
}

#[test]
fn restore_floor_set_since_last_run_mints() {
    let (t, s) = ready();
    s.close(1).unwrap();
    let p = OpenParams {
        announce_last: 1,
        restore_floor_set: true,
    };
    let (_, d) = Store::open(t.path(), &key(), &p).unwrap();
    assert_eq!(
        d,
        mint(MintReasons {
            restore_floor: true,
            ..Default::default()
        })
    );
}

#[test]
fn restore_pending_refuses_allocation_until_an_epoch_is_recorded() {
    let (t, mut s) = ready();
    s.set_restore_pending();
    assert_eq!(s.epoch_state(), EpochState::RestorePending);
    assert!(matches!(
        alloc(&mut s, &PEER, 0),
        Err(StoreError::RestorePending)
    ));
    assert!(matches!(
        s.queue_own_message(&own(PEER, 0, b"x")),
        Err(StoreError::RestorePending)
    ));
    // A clean close while pending leaves no clean record (fail closed).
    s.close(1).unwrap();
    let (mut s, d) = open(t.path(), 1);
    assert!(matches!(d, EpochDecision::Mint(r) if r.unclean));
    s.record_epoch(3).unwrap();
    assert_eq!(alloc(&mut s, &PEER, 0).unwrap(), Seq { epoch: 3, n: 1 });
}

#[test]
fn a_pending_mint_survives_a_clean_close() {
    let t = tempfile::tempdir().unwrap();
    let (s, _) = open(t.path(), 1);
    s.close(1).unwrap(); // no epoch recorded this run
    let (_, d) = open(t.path(), 1);
    assert!(matches!(d, EpochDecision::Mint(r) if r.unclean));
}

#[test]
fn n_space_exhaustion_refuses_then_mints() {
    let (t, mut s) = ready();
    alloc(&mut s, &PEER, 0).unwrap();
    s.conn
        .execute("UPDATE tx_state SET next_n = ?1", [N_MAX as i64])
        .unwrap();
    assert_eq!(alloc(&mut s, &PEER, 0).unwrap(), Seq { epoch: 1, n: N_MAX });
    assert!(matches!(
        alloc(&mut s, &PEER, 0),
        Err(StoreError::EpochExhausted)
    ));
    // Other streams and peers are independent.
    assert_eq!(alloc(&mut s, &PEER, 1).unwrap(), Seq { epoch: 1, n: 1 });
    assert_eq!(alloc(&mut s, &PEER2, 0).unwrap(), Seq { epoch: 1, n: 1 });
    s.close(1).unwrap();
    let (mut s, d) = open(t.path(), 1);
    assert_eq!(
        d,
        mint(MintReasons {
            n_exhausted: true,
            ..Default::default()
        })
    );
    s.record_epoch(2).unwrap();
    assert_eq!(alloc(&mut s, &PEER, 0).unwrap(), Seq { epoch: 2, n: 1 });
    assert_eq!(alloc(&mut s, &PEER, 1).unwrap(), Seq { epoch: 2, n: 1 });
}

#[test]
fn queue_own_message_commits_n_with_content() {
    let (_t, mut s) = ready();
    let (id1, q1) = s.queue_own_message(&own(PEER, 0, b"one")).unwrap();
    let (_, q2) = s.queue_own_message(&own(PEER, 0, b"two")).unwrap();
    assert_eq!((q1.n, q2.n), (1, 2));
    assert_eq!(alloc(&mut s, &PEER, 0).unwrap().n, 3);
    let m = s.message(id1).unwrap().unwrap();
    assert_eq!(m.state, MsgState::Queued);
    assert_eq!(m.dir, Dir::Own);
    assert_eq!(m.seq, q1);
    assert_eq!(m.body.as_deref(), Some(&b"one"[..]));
    assert_eq!(m.received_local_seq, None);
    let big = vec![0u8; rows::MAX_BODY + 1];
    assert!(matches!(
        s.queue_own_message(&own(PEER, 0, &big)),
        Err(StoreError::Invalid(_))
    ));
    // The failed insert rolled back its allocation too.
    assert_eq!(s.peek_next_n(&PEER, 0).unwrap().unwrap().n, 4);
}

// ---------------------------------------------------------------------------
// §4 receive dedup
// ---------------------------------------------------------------------------

#[test]
fn dedup_matrix_persists_across_reopen() {
    let (t, mut s) = ready();
    assert_eq!(s.offer(&PEER, 0, 7, 1).unwrap(), Offer::New);
    assert_eq!(s.offer(&PEER, 0, 7, 1).unwrap(), Offer::BelowPrefix);
    assert_eq!(s.offer(&PEER, 0, 7, 3).unwrap(), Offer::New);
    assert_eq!(s.offer(&PEER, 0, 7, 3).unwrap(), Offer::Duplicate);
    assert_eq!(s.offer(&PEER, 0, 7, 5).unwrap(), Offer::New);
    // Streams and peers are independent.
    assert_eq!(s.offer(&PEER, 1, 7, 1).unwrap(), Offer::New);
    assert_eq!(s.offer(&PEER2, 0, 7, 1).unwrap(), Offer::New);
    assert!(matches!(
        s.offer(&PEER, 0, 7, 0),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        s.offer(&PEER, 0, 7, N_MAX + 1),
        Err(StoreError::Invalid(_))
    ));
    drop(s); // even an unclean stop never forgets
    let (mut s, _) = open(t.path(), 1);
    assert_eq!(s.offer(&PEER, 0, 7, 3).unwrap(), Offer::Duplicate);
    assert_eq!(s.offer(&PEER, 0, 7, 1).unwrap(), Offer::BelowPrefix);
    assert_eq!(s.offer(&PEER, 0, 7, 2).unwrap(), Offer::New);
    let snap = s.rx_snapshot(&PEER, 0).unwrap();
    assert_eq!(snap.len(), 1);
    assert_eq!((snap[0].prefix, snap[0].ranges.clone()), (3, vec![(5, 1)]));
    assert_eq!(s.offer(&PEER, 0, 7, 4).unwrap(), Offer::New);
    assert_eq!(s.rx_snapshot(&PEER, 0).unwrap()[0].prefix, 5);
}

#[test]
fn bounded_ranges_defer_and_never_forget() {
    let (_t, mut s) = ready();
    for k in 0..rx::MAX_RANGES as u64 {
        assert_eq!(s.offer(&PEER, 0, 1, 2 + 2 * k).unwrap(), Offer::New);
    }
    let far = 100;
    assert_eq!(s.offer(&PEER, 0, 1, far).unwrap(), Offer::Deferred);
    // Not recorded: offered again after the gaps close, it is New.
    for n in [1, 3, 5, 7, 9, 11, 13, 15] {
        assert_eq!(s.offer(&PEER, 0, 1, n).unwrap(), Offer::New);
    }
    assert_eq!(s.offer(&PEER, 0, 1, far).unwrap(), Offer::New);
    let snap = s.rx_snapshot(&PEER, 0).unwrap();
    assert_eq!(
        (snap[0].prefix, snap[0].ranges.clone()),
        (16, vec![(far as u32, 1)])
    );
}

#[test]
fn receive_message_dedups_and_re_acks_never_redelivers() {
    let (t, mut s) = ready();
    let seq = |n| Seq { epoch: 9, n };
    let (o, id) = s.receive_message(&inbound(PEER, b"hello"), seq(1)).unwrap();
    assert_eq!(o, Offer::New);
    let id = id.unwrap();
    let (o, id2) = s.receive_message(&inbound(PEER, b"hello"), seq(1)).unwrap();
    assert!(o.re_ack() && !o.deliver() && id2.is_none());
    assert_eq!(
        s.receive_message(&inbound(PEER, b"c"), seq(3)).unwrap().0,
        Offer::New
    );
    assert_eq!(
        s.receive_message(&inbound(PEER, b"c"), seq(3)).unwrap().0,
        Offer::Duplicate
    );
    assert_eq!(s.message_count().unwrap(), 2);
    let m = s.message(id).unwrap().unwrap();
    assert_eq!((m.dir, m.state), (Dir::In, MsgState::Delivered));
    assert_eq!(m.received_local_seq, Some(m.local_seq));
    // Crash and come back: still a duplicate, still one copy.
    drop(s);
    let (mut s, _) = open(t.path(), 1);
    assert_eq!(
        s.receive_message(&inbound(PEER, b"hello"), seq(1))
            .unwrap()
            .0,
        Offer::BelowPrefix
    );
    assert_eq!(s.message_count().unwrap(), 2);
    assert_eq!(s.messages_with(&PEER, 10).unwrap().len(), 2);
}

#[test]
fn frozen_epochs_keep_delivering_and_evicted_epochs_stay_dead() {
    let (_t, mut s) = ready();
    let o = |s: &mut Store, e, n| s.offer(&PEER, 0, e, n).unwrap();
    assert_eq!(o(&mut s, 5, 1), Offer::New);
    assert_eq!(o(&mut s, 5, 2), Offer::New);
    assert_eq!(o(&mut s, 6, 1), Offer::New); // new epoch; 5 frozen
    assert_eq!(o(&mut s, 5, 3), Offer::New); // custody delivers the old epoch late
    assert_eq!(o(&mut s, 5, 2), Offer::BelowPrefix);
    let snap = s.rx_snapshot(&PEER, 0).unwrap();
    assert_eq!(
        snap.iter().map(|e| (e.epoch, e.prefix)).collect::<Vec<_>>(),
        vec![(6, 1), (5, 3)]
    );
    assert_eq!(o(&mut s, 3, 1), Offer::New); // older, unseen, room left
    assert_eq!(o(&mut s, 7, 1), Offer::New); // 4 kept: 7, 6, 5, 3
    assert_eq!(s.rx_floor(&PEER, 0).unwrap(), None);
    assert_eq!(o(&mut s, 8, 1), Offer::New); // evicts 3
    assert_eq!(s.rx_floor(&PEER, 0).unwrap(), Some(3));
    assert_eq!(o(&mut s, 3, 1), Offer::StaleEpoch); // never redelivered
    assert_eq!(o(&mut s, 3, 2), Offer::StaleEpoch);
    assert_eq!(o(&mut s, 4, 1), Offer::StaleEpoch); // older than every kept epoch
    assert_eq!(o(&mut s, 5, 4), Offer::New); // frozen 5 still kept
    assert_eq!(o(&mut s, 9, 1), Offer::New); // evicts 5
    assert_eq!(o(&mut s, 5, 4), Offer::StaleEpoch);
    assert_eq!(o(&mut s, 5, 9), Offer::StaleEpoch);
    assert_eq!(s.rx_floor(&PEER, 0).unwrap(), Some(5));
    let epochs: Vec<u64> = s
        .rx_snapshot(&PEER, 0)
        .unwrap()
        .iter()
        .map(|e| e.epoch)
        .collect();
    assert_eq!(epochs, vec![9, 8, 7, 6]);
    // Fragments obey the same epoch rules.
    assert_eq!(
        s.offer_fragment(&PEER, 0, 5, 10, 0, 2, b"f").unwrap(),
        FragOffer::StaleEpoch
    );
}

#[test]
fn fragments_partials_and_completion() {
    let (_t, mut s) = ready();
    let f =
        |s: &mut Store, n, i: u8, c| s.offer_fragment(&PEER, 0, 1, n, i, c, &[b'a' + i]).unwrap();
    assert_eq!(f(&mut s, 1, 0, 3), FragOffer::Stored { complete: false });
    assert_eq!(f(&mut s, 1, 0, 3), FragOffer::DuplicateFragment);
    assert_eq!(f(&mut s, 1, 2, 3), FragOffer::Stored { complete: false });
    assert_eq!(f(&mut s, 1, 1, 4), FragOffer::Mismatch);
    let p = s.rx_snapshot(&PEER, 0).unwrap()[0].partials.clone();
    assert_eq!(
        p,
        vec![Partial {
            n: 1,
            frag_count: 3,
            bitmap: 0b101
        }]
    );
    // Incomplete: nothing to reassemble yet.
    assert_eq!(s.reassemble(&PEER, 0, 1, 1).unwrap(), None);
    assert_eq!(f(&mut s, 1, 1, 3), FragOffer::Stored { complete: true });
    // Reassembled from the stored bytes, in frag_idx order.
    let body = s.reassemble(&PEER, 0, 1, 1).unwrap().unwrap();
    assert_eq!(body, b"abc");
    let (o, _) = s
        .receive_message(&inbound(PEER, &body), Seq { epoch: 1, n: 1 })
        .unwrap();
    assert_eq!(o, Offer::New);
    let e = &s.rx_snapshot(&PEER, 0).unwrap()[0];
    assert!(e.partials.is_empty() && e.prefix == 1);
    assert_eq!(frag_rows(&s, 1), 0); // the bytes went with the partial
    assert_eq!(s.reassemble(&PEER, 0, 1, 1).unwrap(), None);
    assert_eq!(f(&mut s, 1, 0, 3), FragOffer::Delivered(Offer::BelowPrefix));
    // Bounds are input errors.
    assert!(matches!(
        s.offer_fragment(&PEER, 0, 1, 2, 3, 3, b""),
        Err(StoreError::Invalid(_))
    ));
    assert!(s.offer_fragment(&PEER, 0, 1, 2, 0, 17, b"").is_err());
    assert!(s.offer_fragment(&PEER, 0, 1, 2, 0, 0, b"").is_err());
    assert!(matches!(
        s.offer_fragment(&PEER, 0, 1, 2, 0, 2, &vec![0; rx::MAX_FRAG_BYTES + 1]),
        Err(StoreError::Invalid(_))
    ));
    // Single-fragment messages complete at once; 16-fragment bitmaps fill.
    assert_eq!(f(&mut s, 2, 0, 1), FragOffer::Stored { complete: true });
    for i in 0..15 {
        assert_eq!(f(&mut s, 3, i, 16), FragOffer::Stored { complete: false });
    }
    assert_eq!(f(&mut s, 3, 15, 16), FragOffer::Stored { complete: true });
    assert_eq!(frag_rows(&s, 3), 16);
    // Partials are bounded; the oldest is evicted first, bytes included.
    for n in 4..4 + rx::MAX_PARTIALS as u64 {
        f(&mut s, n, 0, 2);
    }
    let parts = &s.rx_snapshot(&PEER, 0).unwrap()[0].partials;
    assert_eq!(parts.len(), rx::MAX_PARTIALS);
    assert!(parts.iter().all(|p| p.n != 2 && p.n != 3));
    assert_eq!((frag_rows(&s, 2), frag_rows(&s, 3)), (0, 0));
    assert_eq!(s.reassemble(&PEER, 0, 1, 3).unwrap(), None);
    let held: i64 = s
        .conn
        .query_row("SELECT count(*) FROM rx_frags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(held, rx::MAX_PARTIALS as i64);
}

/// `rx_frags` rows held for `(PEER, 0, epoch 1, n)`.
fn frag_rows(s: &Store, n: i64) -> i64 {
    s.conn
        .query_row(
            "SELECT count(*) FROM rx_frags WHERE peer = ?1 AND n = ?2",
            rusqlite::params![&PEER[..], n],
            |r| r.get(0),
        )
        .unwrap()
}

// ---------------------------------------------------------------------------
// rows: messages, contacts, outbox, custody, cursors
// ---------------------------------------------------------------------------

#[test]
fn message_states_never_regress_and_delivery_releases_the_outbox() {
    let (_t, mut s) = ready();
    let (id, _) = s.queue_own_message(&own(PEER, 0, b"m")).unwrap();
    s.outbox_put(id, 0, 2, &PEER, Some(b"sealed-0")).unwrap();
    s.outbox_put(id, 1, 2, &PEER, None).unwrap();
    assert!(s.outbox_put(id, 2, 2, &PEER, None).is_err());
    s.outbox_set_outstanding(id, 0, true).unwrap();
    assert_eq!(s.outbox_outstanding(&PEER).unwrap(), 1);
    assert_eq!(s.outbox_peers().unwrap(), vec![PEER]);
    assert!(!s.outbox_ack(id, 0b01).unwrap());
    assert_eq!(s.outbox_for(&PEER).unwrap().len(), 1);
    assert!(s.set_message_state(id, MsgState::Sent, None).unwrap());
    assert!(s
        .set_message_state(id, MsgState::Relayed, Some("mailbox"))
        .unwrap());
    assert!(s.set_message_state(id, MsgState::Delivered, None).unwrap());
    assert!(!s.set_message_state(id, MsgState::Sent, None).unwrap());
    assert_eq!(s.message(id).unwrap().unwrap().state, MsgState::Delivered);
    assert!(s.outbox_for(&PEER).unwrap().is_empty());
    assert!(matches!(
        s.set_message_state(999, MsgState::Sent, None),
        Err(StoreError::NotFound)
    ));
}

#[test]
fn contacts_keys_are_immutable_and_stale_announces_change_nothing() {
    let (_t, mut s) = ready();
    let mut c = ContactRow {
        node_id: PEER,
        enc_pubkey: [7; 32],
        announce: vec![1; 140],
        announce_counter: 5,
        trust: Trust::Oob,
        alias: Some("bee".into()),
        roles: 1,
        home_hub: None,
        first_seen_seq: 0,
        last_seen_seq: 0,
        request: false,
        pinned: false,
    };
    s.contact_upsert(&c).unwrap();
    let first = s.contact(&PEER).unwrap().unwrap().first_seen_seq;
    c.announce = vec![2; 140];
    c.announce_counter = 3; // stale: same keys, lower counter
    s.contact_upsert(&c).unwrap();
    let got = s.contact(&PEER).unwrap().unwrap();
    assert_eq!((got.announce[0], got.announce_counter), (1, 5));
    assert_eq!(got.first_seen_seq, first);
    assert!(got.last_seen_seq > first);
    c.announce_counter = 6;
    s.contact_upsert(&c).unwrap();
    assert_eq!(s.contact(&PEER).unwrap().unwrap().announce[0], 2);
    c.enc_pubkey = [8; 32];
    assert!(matches!(s.contact_upsert(&c), Err(StoreError::KeyConflict)));
    assert_eq!(s.contacts().unwrap().len(), 1);
    assert!(s.contact_forget(&PEER).unwrap());
    assert!(s.contact(&PEER).unwrap().is_none());
}

#[test]
fn custody_dedup_fetch_commit_and_monotonic_sequence() {
    let (_t, mut s) = ready();
    assert_eq!(
        s.custody_put(&PEER, b"item-a").unwrap(),
        CustodyPut::Accepted(1)
    );
    assert_eq!(
        s.custody_put(&PEER, b"item-a").unwrap(),
        CustodyPut::Duplicate(1)
    );
    assert_eq!(
        s.custody_put(&PEER2, b"item-b").unwrap(),
        CustodyPut::Accepted(2)
    );
    assert_eq!(
        s.custody_put(&PEER, b"item-c").unwrap(),
        CustodyPut::Accepted(3)
    );
    let got = s.custody_fetch(&PEER, 0, 10).unwrap();
    assert_eq!(
        got.iter().map(|i| i.custody_seq).collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_eq!(s.custody_fetch(&PEER, 1, 10).unwrap()[0].bytes, b"item-c");
    assert_eq!(s.custody_commit(&PEER, 1).unwrap(), 1);
    assert_eq!(s.custody_commit(&PEER, 1).unwrap(), 0);
    assert_eq!(s.custody_stats().unwrap().0, 3);
    assert!(s
        .custody_put(&PEER, &vec![0; rows::MAX_CUSTODY_ITEM + 1])
        .is_err());
    // Pruning removes the committed item; the sequence never goes back.
    s.prune_with(DiskSpace::UNKNOWN, Watermarks { high: 0, low: 0 })
        .unwrap();
    assert_eq!(s.custody_fetch(&PEER, 0, 10).unwrap().len(), 1);
    assert_eq!(s.custody_high().unwrap(), 3);
    s.prune_with(DiskSpace::UNKNOWN, Watermarks::with_high(u64::MAX / 2))
        .unwrap();
    assert_eq!(
        s.custody_put(&PEER, b"item-d").unwrap(),
        CustodyPut::Accepted(4)
    );
    // Member-side cursor is max-monotonic.
    assert_eq!(s.advance_cursor(&PEER2, 5).unwrap(), 5);
    assert_eq!(s.advance_cursor(&PEER2, 2).unwrap(), 5);
    assert_eq!(s.cursor(&PEER2).unwrap(), 5);
    assert_eq!(s.cursor(&PEER).unwrap(), 0);
}

// ---------------------------------------------------------------------------
// §10.4 pruning
// ---------------------------------------------------------------------------

struct Fixture {
    custody_big: u64,
    custody_small: u64,
    custody_open: u64,
    blob_refetch: BlobHash,
    blob_superseded: BlobHash,
    blob_pinned: BlobHash,
    blob_private: BlobHash,
    hist_own: i64,
    hist_in: i64,
    undelivered: i64,
    pinned_msg: i64,
    request_msg: i64,
}

fn sample(len: usize, salt: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31) ^ salt)
        .collect()
}

/// Every §10.4 category, inserted so that the step-1 weights are
/// unambiguous: the big custody item is the oldest of the three evictables.
fn fixture(s: &mut Store) -> Fixture {
    const REQ: [u8; 32] = [0xC0; 32];
    // Step 3 history: an own delivered message, then an inbound one.
    let (hist_own, _) = s.queue_own_message(&own(PEER, 0, &[1; 300])).unwrap();
    s.set_message_state(hist_own, MsgState::Delivered, None)
        .unwrap();
    let (_, hist_in) = s
        .receive_message(&inbound(PEER2, &[2; 300]), Seq { epoch: 4, n: 1 })
        .unwrap();
    // Protected: undelivered outbound (with its outbox fragment), pinned,
    // a pending request's message, uncommitted custody, a contact.
    let (undelivered, _) = s.queue_own_message(&own(PEER, 0, &[3; 300])).unwrap();
    s.outbox_put(undelivered, 0, 1, &PEER, Some(&[9; 100]))
        .unwrap();
    let (pinned_msg, _) = s.queue_own_message(&own(PEER2, 0, &[4; 300])).unwrap();
    s.set_message_state(pinned_msg, MsgState::Delivered, None)
        .unwrap();
    s.set_message_pinned(pinned_msg, true).unwrap();
    assert_eq!(s.request_put(&REQ, &[5; 140]).unwrap(), RequestPut::Filed);
    let (_, request_msg) = s
        .receive_message(&inbound(REQ, &[6; 300]), Seq { epoch: 4, n: 1 })
        .unwrap();
    let custody_open = match s.custody_put(&PEER2, &[7; 3000]).unwrap() {
        CustodyPut::Accepted(q) => q,
        other => panic!("{other:?}"),
    };
    s.contact_upsert(&ContactRow {
        node_id: PEER,
        enc_pubkey: [7; 32],
        announce: vec![1; 140],
        announce_counter: 1,
        trust: Trust::Enrolled,
        alias: None,
        roles: 0,
        home_hub: None,
        first_seen_seq: 0,
        last_seen_seq: 0,
        request: false,
        pinned: false,
    })
    .unwrap();
    let blob_pinned = s
        .blob_put_public(&sample(4000, 1), Class::Object, true)
        .unwrap();
    s.blob_set_pinned(&blob_pinned, true).unwrap();
    let (blob_private, _) = s.blob_put_private(&sample(4000, 2), Class::Object).unwrap();
    // Step 2: a superseded update payload.
    let blob_superseded = s
        .blob_put_public(&sample(8000, 3), Class::Update, false)
        .unwrap();
    s.blob_mark_superseded(&blob_superseded).unwrap();
    // Step 1, oldest first: big committed custody, small committed custody,
    // then a re-fetchable public chunk.
    let custody_big = match s.custody_put(&PEER, &[8; 60_000]).unwrap() {
        CustodyPut::Accepted(q) => q,
        other => panic!("{other:?}"),
    };
    let custody_small = match s.custody_put(&PEER, &[9; 10]).unwrap() {
        CustodyPut::Accepted(q) => q,
        other => panic!("{other:?}"),
    };
    s.custody_commit(&PEER, custody_small).unwrap();
    let blob_refetch = s
        .blob_put_public(&sample(5000, 4), Class::Object, true)
        .unwrap();
    Fixture {
        custody_big,
        custody_small,
        custody_open,
        blob_refetch,
        blob_superseded,
        blob_pinned,
        blob_private,
        hist_own,
        hist_in: hist_in.unwrap(),
        undelivered,
        pinned_msg,
        request_msg: request_msg.unwrap(),
    }
}

#[test]
fn pruning_follows_the_normative_order_and_raises_store_full() {
    let (_t, mut s) = ready();
    let fx = fixture(&mut s);
    // A tiny watermark: everything prunable must go.
    s.set_watermarks(Watermarks { high: 0, low: 0 }).unwrap();
    let r = s.prune(DiskSpace::UNKNOWN).unwrap();
    assert!(r.triggered);
    let order: Vec<(PruneStep, PrunedRef)> = r.pruned.iter().map(|p| (p.step, p.item)).collect();
    assert_eq!(
        order,
        vec![
            (PruneStep::Evictable, PrunedRef::Custody(fx.custody_big)),
            (PruneStep::Evictable, PrunedRef::Blob(fx.blob_refetch)),
            (PruneStep::Evictable, PrunedRef::Custody(fx.custody_small)),
            (PruneStep::Superseded, PrunedRef::Blob(fx.blob_superseded)),
        ]
    );
    // History waits for an export offer; only protected items remain.
    assert!(r.export_offer_needed);
    assert!(r.store_full && s.store_full());
    assert!(s.message(fx.hist_own).unwrap().is_some());
    // Pruned blob files are gone; their rows stay (re-fetchable / superseded).
    assert!(!s.blob_dir().exists(&fx.blob_refetch));
    assert!(!s.blob(&fx.blob_refetch).unwrap().unwrap().present);
    assert!(matches!(
        s.blob_read(&fx.blob_refetch, u64::MAX),
        Err(StoreError::Blob(BlobError::Missing))
    ));

    // store_full refuses new custody and new bulk — never messages.
    assert_eq!(
        s.custody_put(&PEER, b"new").unwrap(),
        CustodyPut::RejectFull
    );
    assert!(matches!(
        s.blob_put_public(b"bulk", Class::Kb, false),
        Err(StoreError::StoreFull)
    ));
    let (late, _) = s.queue_own_message(&own(PEER, 0, b"still ok")).unwrap();
    assert!(s
        .receive_message(&inbound(PEER2, b"in ok"), Seq { epoch: 4, n: 2 })
        .unwrap()
        .1
        .is_some());

    // After the export offer, history goes oldest-first.
    s.set_export_offered(true).unwrap();
    let r = s.prune(DiskSpace::UNKNOWN).unwrap();
    let hist: Vec<PrunedRef> = r
        .pruned
        .iter()
        .filter(|p| p.step == PruneStep::History)
        .map(|p| p.item)
        .collect();
    assert_eq!(
        hist[..2],
        [
            PrunedRef::Message(fx.hist_own),
            PrunedRef::Message(fx.hist_in)
        ]
    );
    assert!(r.pruned.iter().all(|p| p.step == PruneStep::History));
    assert!(r.store_full && !r.export_offer_needed);

    // Never pruned: undelivered outbound, pinned, pending request, own
    // queued, uncommitted custody, pinned/private blobs, contacts.
    for id in [fx.undelivered, fx.pinned_msg, fx.request_msg, late] {
        assert!(s.message(id).unwrap().is_some(), "message {id}");
    }
    assert_eq!(
        s.custody_fetch(&PEER2, 0, 10).unwrap()[0].custody_seq,
        fx.custody_open
    );
    assert!(s.blob_dir().exists(&fx.blob_pinned));
    assert!(s.blob_dir().exists(&fx.blob_private));
    assert!(s.contact(&PEER).unwrap().is_some());

    // Room again: store_full clears and custody is accepted.
    s.set_watermarks(Watermarks::with_high(u64::MAX / 2))
        .unwrap();
    let r = s.prune(DiskSpace::UNKNOWN).unwrap();
    assert!(!r.triggered && !r.store_full && r.pruned.is_empty());
    assert!(matches!(
        s.custody_put(&PEER, b"new").unwrap(),
        CustodyPut::Accepted(_)
    ));
}

#[test]
fn just_over_the_watermark_prunes_exactly_the_heaviest_item() {
    let (_t, mut s) = ready();
    let fx = fixture(&mut s);
    let live = s.live_bytes().unwrap();
    let r = s
        .prune_with(
            DiskSpace::UNKNOWN,
            Watermarks {
                high: live - 1,
                low: live - 1,
            },
        )
        .unwrap();
    assert_eq!(r.pruned.len(), 1);
    assert_eq!(r.pruned[0].item, PrunedRef::Custody(fx.custody_big));
    assert!(!r.store_full);
}

#[test]
fn free_disk_below_one_percent_triggers_pruning() {
    let (_t, mut s) = ready();
    let fx = fixture(&mut s);
    // Live bytes are far below the default watermark; the disk is not.
    let r = s
        .prune(DiskSpace {
            free: 5,
            total: 1000,
        })
        .unwrap();
    assert!(r.triggered);
    assert_eq!(r.pruned[0].item, PrunedRef::Custody(fx.custody_big));
    let r = s
        .prune(DiskSpace {
            free: 500,
            total: 1000,
        })
        .unwrap();
    assert!(!r.triggered && r.pruned.is_empty());
}

#[test]
fn quotas_shape_refusable_classes_only() {
    let (_t, mut s) = ready();
    assert!(matches!(
        s.set_quota(Class::Human, Some(1)),
        Err(StoreError::Invalid(_))
    ));
    for (i, len) in [(1u8, 2000usize), (2, 2000), (3, 2000)] {
        s.custody_put(&PEER, &vec![i; len]).unwrap();
    }
    s.custody_put(&PEER2, &[4; 2000]).unwrap(); // uncommitted
    s.custody_commit(&PEER, 3).unwrap();
    s.set_quota(Class::Custody, Some(4500)).unwrap();
    assert_eq!(s.quota(Class::Custody).unwrap(), Some(4500));
    let r = s.prune(DiskSpace::UNKNOWN).unwrap();
    assert!(!r.triggered); // global capacity is fine; the quota shaped it
    assert!(r
        .pruned
        .iter()
        .all(|p| p.step == PruneStep::Quota(Class::Custody)));
    assert_eq!(r.pruned.len(), 2);
    assert!(s.class_bytes(Class::Custody).unwrap() <= 4500);
    assert!(r.full_classes.is_empty());
    // Over quota with only uncommitted items left: the class refuses.
    s.set_quota(Class::Custody, Some(1000)).unwrap();
    s.custody_commit(&PEER, 0).unwrap();
    let r = s.prune(DiskSpace::UNKNOWN).unwrap();
    assert_eq!(s.custody_stats().unwrap().0, 1); // the uncommitted one
    assert_eq!(r.full_classes, vec![Class::Custody]);
    assert_eq!(s.custody_put(&PEER, b"x").unwrap(), CustodyPut::RejectFull);
    // Other classes are unaffected.
    s.blob_put_public(b"kb delta", Class::Kb, false).unwrap();
    s.set_quota(Class::Custody, None).unwrap();
    assert!(matches!(
        s.custody_put(&PEER, b"x").unwrap(),
        CustodyPut::Accepted(_)
    ));
}

// ---------------------------------------------------------------------------
// §11 blobs through the store
// ---------------------------------------------------------------------------

#[test]
fn blob_round_trips_refcounts_and_sweep() {
    let (t, mut s) = ready();
    let data = sample(50_000, 9);
    let h = s.blob_put_public(&data, Class::Kb, false).unwrap();
    assert_eq!(h, BlobHash::of(&data));
    assert_eq!(s.blob_put_public(&data, Class::Kb, false).unwrap(), h);
    assert_eq!(s.blob(&h).unwrap().unwrap().refcount, 2);
    assert_eq!(s.blob_read(&h, u64::MAX).unwrap(), data);
    assert_eq!(s.blob_decref(&h).unwrap(), 1);
    assert!(s.blob_dir().exists(&h));
    assert_eq!(s.blob_incref(&h).unwrap(), 2);
    assert_eq!(s.blob_decref(&h).unwrap(), 1);
    assert_eq!(s.blob_decref(&h).unwrap(), 0);
    assert!(s.blob(&h).unwrap().is_none());
    assert!(!s.blob_dir().path(&h).exists());
    assert!(matches!(s.blob_decref(&h), Err(StoreError::NotFound)));

    // Private: encrypted once under a key minted here, addressed by
    // ciphertext, key kept in the DB.
    let secret = sample(70_000, 7);
    let (hp, key) = s.blob_put_private(&secret, Class::Object).unwrap();
    let ct = s.blob_read(&hp, u64::MAX).unwrap();
    assert_eq!(BlobHash::of(&ct), hp);
    assert_ne!(&ct[..secret.len().min(ct.len())], &secret[..]);
    assert_eq!(
        &s.blob_read_private(&hp, u64::MAX).unwrap()[..],
        &secret[..]
    );
    assert_eq!(
        s.blob_object_key(&hp).unwrap().unwrap().as_bytes(),
        key.as_bytes()
    );
    assert!(!s.blob(&hp).unwrap().unwrap().public);
    assert!(matches!(
        s.blob_read_private(&BlobHash::of(b"nope"), u64::MAX),
        Err(StoreError::NotFound)
    ));

    // Received objects must match their address; nothing lands otherwise.
    let other = sample(1234, 1);
    assert!(matches!(
        s.blob_put_received(&BlobHash([0; 32]), &other, true, None, Class::Object, true),
        Err(StoreError::Blob(BlobError::HashMismatch))
    ));
    let hr = s
        .blob_put_received(
            &BlobHash::of(&other),
            &other,
            true,
            None,
            Class::Object,
            true,
        )
        .unwrap();
    assert!(s.blob(&hr).unwrap().unwrap().refetchable);

    // Orphans (a temp dir, a dir without a row) are swept at open; rows'
    // directories survive.
    fs::create_dir(s.blob_dir().root().join(".tmp-123")).unwrap();
    let orphan = BlobHash::of(b"orphan");
    fs::create_dir(s.blob_dir().path(&orphan)).unwrap();
    fs::write(s.blob_dir().data_path(&orphan), b"orphan").unwrap();
    s.close(1).unwrap();
    let (s, _) = open(t.path(), 1);
    let names: Vec<String> = s
        .blob_dir()
        .entries()
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert!(!names.iter().any(|n| n.starts_with(".tmp-")));
    assert!(!s.blob_dir().path(&orphan).exists());
    assert!(s.blob_dir().exists(&hp) && s.blob_dir().exists(&hr));
    assert_eq!(
        &s.blob_read_private(&hp, u64::MAX).unwrap()[..],
        &secret[..]
    );
}

#[test]
fn a_full_class_never_refuses_another_class() {
    let (_t, mut s) = ready();
    s.custody_put(&PEER, &[1; 3000]).unwrap(); // uncommitted: protected
    s.set_quota(Class::Custody, Some(100)).unwrap();
    let r = s.prune(DiskSpace::UNKNOWN).unwrap();
    assert_eq!(r.full_classes, vec![Class::Custody]);
    // Force admission through a full pruning pass: Kb still fits.
    s.store_full = true;
    assert!(s.admit(Class::Kb, 10).unwrap());
    assert!(!s.admit(Class::Custody, 10).unwrap());
}

#[test]
fn incoming_items_never_displace_own_history() {
    let (_t, mut s) = ready();
    let (id, _) = s.queue_own_message(&own(PEER, 0, &[1; 500])).unwrap();
    s.set_message_state(id, MsgState::Delivered, None).unwrap();
    s.set_export_offered(true).unwrap();
    // The store itself is under its watermark; an item that would overflow
    // it is refused rather than paid for with history.
    let huge = DEFAULT_TEST_HIGH + 1;
    s.set_watermarks(Watermarks::with_high(DEFAULT_TEST_HIGH))
        .unwrap();
    assert!(!s.admit(Class::Object, huge).unwrap());
    assert!(!s.store_full());
    assert!(s.message(id).unwrap().is_some());
}

const DEFAULT_TEST_HIGH: u64 = 64 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Review fixes (W0-A): fragment bytes, request admission, pkt_ref, trust,
// keyless blobs, transit eviction, store_full persistence
// ---------------------------------------------------------------------------

/// Major 1: a fragment's bytes commit with its bit. Crash between the last
/// fragment and reassembly: after reopen the message is listed as complete,
/// reassembles from the stored bytes and finishes; the SACKed fragments are
/// never needed again.
#[test]
fn fragment_bytes_survive_a_crash_and_the_message_finishes() {
    let (t, mut s) = ready();
    let parts: [&[u8]; 3] = [b"frag-zero|", b"frag-one|", b"frag-two"];
    for (i, p) in parts.iter().enumerate() {
        let o = s.offer_fragment(&PEER, 0, 1, 7, i as u8, 3, p).unwrap();
        assert_eq!(o, FragOffer::Stored { complete: i == 2 });
    }
    // A second message, still partial at the crash.
    s.offer_fragment(&PEER, 0, 1, 8, 1, 2, b"tail").unwrap();
    drop(s); // crash: no close, no reassembly

    let (mut s, _) = open(t.path(), 1);
    s.record_epoch(2).unwrap();
    assert_eq!(
        s.complete_partials().unwrap(),
        vec![PartialRef {
            peer: PEER,
            stream: 0,
            epoch: 1,
            n: 7
        }]
    );
    // A retransmit of a SACKed fragment is still only a re-ACK.
    assert_eq!(
        s.offer_fragment(&PEER, 0, 1, 7, 0, 3, b"frag-zero|")
            .unwrap(),
        FragOffer::DuplicateFragment
    );
    let frags = s.fragments(&PEER, 0, 1, 7).unwrap().unwrap();
    assert_eq!(frags, parts.map(<[u8]>::to_vec).to_vec());
    let body = s.reassemble(&PEER, 0, 1, 7).unwrap().unwrap();
    assert_eq!(body, b"frag-zero|frag-one|frag-two");
    let (o, id) = s
        .receive_message(&inbound(PEER, &body), Seq { epoch: 1, n: 7 })
        .unwrap();
    assert_eq!(o, Offer::New);
    assert_eq!(
        s.message(id.unwrap()).unwrap().unwrap().body.as_deref(),
        Some(&body[..])
    );
    assert!(s.complete_partials().unwrap().is_empty());
    assert_eq!(s.fragments(&PEER, 0, 1, 7).unwrap(), None);
    // The other partial is untouched and still resumable.
    assert_eq!(
        s.offer_fragment(&PEER, 0, 1, 8, 0, 2, b"head-").unwrap(),
        FragOffer::Stored { complete: true }
    );
    assert_eq!(s.reassemble(&PEER, 0, 1, 8).unwrap().unwrap(), b"head-tail");
    // A stored row that disagrees with the bitmap is never trusted.
    s.conn
        .execute("DELETE FROM rx_frags WHERE n = 8 AND frag_idx = 0", [])
        .unwrap();
    assert!(matches!(
        s.reassemble(&PEER, 0, 1, 8),
        Err(StoreError::Corrupt("rx_frags"))
    ));
}

/// Evicting a whole epoch drops its stored fragments with it; a maximal
/// fragment (one `tcp-lan` frame) is accepted.
#[test]
fn epoch_eviction_drops_fragment_bytes() {
    let (_t, mut s) = ready();
    let big = vec![0x5A; rx::MAX_FRAG_BYTES];
    assert_eq!(
        s.offer_fragment(&PEER, 0, 1, 1, 0, 2, &big).unwrap(),
        FragOffer::Stored { complete: false }
    );
    for e in 2..=4 {
        s.offer_fragment(&PEER, 0, e, 1, 0, 2, b"x").unwrap();
    }
    let count = |s: &Store, e: i64| -> i64 {
        s.conn
            .query_row("SELECT count(*) FROM rx_frags WHERE epoch = ?1", [e], |r| {
                r.get(0)
            })
            .unwrap()
    };
    assert_eq!(count(&s, 1), 1);
    // A fifth epoch evicts epoch 1, bytes included.
    s.offer_fragment(&PEER, 0, 5, 1, 0, 2, b"x").unwrap();
    assert_eq!(s.rx_floor(&PEER, 0).unwrap(), Some(1));
    assert_eq!(count(&s, 1), 0);
    assert_eq!(
        s.offer_fragment(&PEER, 0, 1, 1, 1, 2, b"late").unwrap(),
        FragOffer::StaleEpoch
    );
}

/// Major 2: pending requests are admitted against a count quota and
/// capacity; refusals are typed; an already pending NodeId is never refused;
/// request rows count in live bytes.
#[test]
fn request_admission_quota_store_full_and_live_bytes() {
    let (_t, mut s) = ready();
    assert_eq!(s.request_quota().unwrap(), prune::DEFAULT_REQUEST_QUOTA);
    assert_eq!(prune::DEFAULT_REQUEST_QUOTA, 256);
    let live0 = s.live_bytes().unwrap();
    let id = |i: u32| -> [u8; 32] {
        let mut v = [0xD0; 32];
        v[..4].copy_from_slice(&i.to_le_bytes());
        v
    };
    for i in 0..200 {
        assert_eq!(
            s.request_put(&id(i), &[i as u8; 169]).unwrap(),
            RequestPut::Filed
        );
    }
    assert_eq!(s.request_count().unwrap(), 200);
    // ~200 × (32 + 169) B of rows: live bytes see them (DB pages).
    assert!(s.live_bytes().unwrap() >= live0 + 200 * 201);
    // The quota (settable) refuses new NodeIds, never a pending one.
    s.set_request_quota(Some(200)).unwrap();
    assert_eq!(s.request_quota().unwrap(), 200);
    assert_eq!(
        s.request_put(&id(999), &[1; 140]).unwrap(),
        RequestPut::Refused(RequestRefusal::Quota)
    );
    assert_eq!(
        s.request_put(&id(5), &[5; 169]).unwrap(),
        RequestPut::AlreadyPending
    );
    assert!(!s.request_pending(&id(999)).unwrap());
    // Resolving one makes room again; None restores the default.
    assert!(s.request_remove(&id(0)).unwrap());
    assert_eq!(
        s.request_put(&id(999), &[1; 140]).unwrap(),
        RequestPut::Filed
    );
    s.set_request_quota(None).unwrap();
    assert_eq!(s.request_quota().unwrap(), 256);

    // store_full (only protected rows over the watermark): refused, never
    // pruned — the pending ones all stay.
    s.set_watermarks(Watermarks { high: 0, low: 0 }).unwrap();
    assert!(s.prune(DiskSpace::UNKNOWN).unwrap().store_full);
    assert_eq!(
        s.request_put(&id(1000), &[1; 140]).unwrap(),
        RequestPut::Refused(RequestRefusal::StoreFull)
    );
    assert_eq!(s.request_count().unwrap(), 200);
    assert_eq!(
        s.request_put(&id(7), &[7; 169]).unwrap(),
        RequestPut::AlreadyPending
    );
    // Room again: admitted.
    s.set_watermarks(Watermarks::with_high(u64::MAX / 2))
        .unwrap();
    assert_eq!(
        s.request_put(&id(1000), &[1; 140]).unwrap(),
        RequestPut::Filed
    );
    assert!(matches!(
        s.request_put(&id(1001), &[0; rows::MAX_ANNOUNCE + 1]),
        Err(StoreError::Invalid(_))
    ));
}

/// Minor: `pkt_ref` is computed inside from the exact bytes (BLAKE2b-128 over
/// "111/custody/v1" ‖ bytes), re-derived here independently.
#[test]
fn custody_pkt_ref_is_computed_from_the_item_bytes() {
    use blake2::digest::consts::U16;
    use blake2::{Blake2b, Digest};
    let (_t, mut s) = ready();
    let item = b"sealed custody item bytes";
    assert_eq!(s.custody_put(&PEER, item).unwrap(), CustodyPut::Accepted(1));
    let stored: Vec<u8> = s
        .conn
        .query_row(
            "SELECT pkt_ref FROM custody WHERE custody_seq = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut h = Blake2b::<U16>::new();
    h.update(b"111/custody/v1");
    h.update(item);
    assert_eq!(stored, h.finalize().to_vec());
    assert_eq!(stored, crate::custody::pkt_ref(item).to_vec());
    // Same bytes for another member: still the same item (dedup by bytes).
    assert_eq!(
        s.custody_put(&PEER2, item).unwrap(),
        CustodyPut::Duplicate(1)
    );
}

fn contact(trust: Trust, request: bool, pinned: bool) -> ContactRow {
    ContactRow {
        node_id: PEER,
        enc_pubkey: [7; 32],
        announce: vec![1; 140],
        announce_counter: 1,
        trust,
        alias: None,
        roles: 0,
        home_hub: None,
        first_seen_seq: 0,
        last_seen_seq: 0,
        request,
        pinned,
    }
}

/// Minor: trust is monotone (tofu < oob < enrolled); a refresh never
/// touches `pinned` or `request`.
#[test]
fn contact_trust_is_monotone_and_flags_survive_refresh() {
    let (_t, mut s) = ready();
    assert!(Trust::Tofu.rank() < Trust::Oob.rank());
    assert!(Trust::Oob.rank() < Trust::Enrolled.rank());
    s.contact_upsert(&contact(Trust::Tofu, true, true)).unwrap();
    let trust = |s: &Store| s.contact(&PEER).unwrap().unwrap().trust;
    // Upgrades take.
    s.contact_upsert(&contact(Trust::Oob, false, false))
        .unwrap();
    assert_eq!(trust(&s), Trust::Oob);
    s.contact_upsert(&contact(Trust::Enrolled, false, false))
        .unwrap();
    assert_eq!(trust(&s), Trust::Enrolled);
    // An INTRO or card refresh never downgrades.
    s.contact_upsert(&contact(Trust::Tofu, false, false))
        .unwrap();
    assert_eq!(trust(&s), Trust::Enrolled);
    s.contact_upsert(&contact(Trust::Oob, false, false))
        .unwrap();
    assert_eq!(trust(&s), Trust::Enrolled);
    // The refreshes (all with request = pinned = false) left both flags.
    let c = s.contact(&PEER).unwrap().unwrap();
    assert!(c.request && c.pinned);
    // Only the explicit setters change them.
    s.contact_set_request(&PEER, false).unwrap();
    s.contact_set_pinned(&PEER, false).unwrap();
    let c = s.contact(&PEER).unwrap().unwrap();
    assert!(!c.request && !c.pinned);
    s.contact_upsert(&contact(Trust::Tofu, true, true)).unwrap();
    let c = s.contact(&PEER).unwrap().unwrap();
    assert!(!c.request && !c.pinned);
    assert!(matches!(
        s.contact_set_pinned(&PEER2, true),
        Err(StoreError::NotFound)
    ));
}

/// Minor: a ciphertext-only private object (a relay/hub holding it keyless)
/// is representable; a key learned later is kept; a store-minted key is the
/// only encryption key; an entropy failure is typed and leaves nothing.
#[test]
fn keyless_private_blobs_and_store_minted_keys() {
    let (t, mut s) = ready();
    // The sender's node encrypts (key minted inside) …
    let (sender, _) = {
        let other = tempfile::tempdir().unwrap();
        let (mut o, _) = open(other.path(), 1);
        o.record_epoch(1).unwrap();
        let (h, k) = o
            .blob_put_private(b"private payload", Class::Object)
            .unwrap();
        let ct = o.blob_read(&h, u64::MAX).unwrap();
        ((h, ct, k), other)
    };
    let (h, ct, k) = sender;
    // … this hub holds the ciphertext without the key.
    let got = s
        .blob_put_received(&h, &ct, false, None, Class::Object, false)
        .unwrap();
    assert_eq!(got, h);
    let row = s.blob(&h).unwrap().unwrap();
    assert!(!row.public && row.present);
    assert!(s.blob_object_key(&h).unwrap().is_none());
    assert_eq!(s.blob_read(&h, u64::MAX).unwrap(), ct); // verifiable keyless
    assert!(matches!(
        s.blob_read_private(&h, u64::MAX),
        Err(StoreError::NoObjectKey)
    ));
    // The recipient later learns the key (OBJ_ADVERT): kept, readable.
    s.blob_put_received(&h, &ct, false, Some(&k), Class::Object, false)
        .unwrap();
    assert_eq!(s.blob(&h).unwrap().unwrap().refcount, 2);
    assert_eq!(
        &s.blob_read_private(&h, u64::MAX).unwrap()[..],
        b"private payload"
    );
    // A stored key is never replaced by another.
    s.blob_put_received(
        &h,
        &ct,
        false,
        Some(&ObjectKey::from_bytes([9; 32])),
        Class::Object,
        false,
    )
    .unwrap();
    assert_eq!(
        s.blob_object_key(&h).unwrap().unwrap().as_bytes(),
        k.as_bytes()
    );
    // Same plaintext twice through the store: two fresh keys, two objects.
    let (a, ka) = s.blob_put_private(b"same", Class::Object).unwrap();
    let (b, kb) = s.blob_put_private(b"same", Class::Object).unwrap();
    assert_ne!(a, b);
    assert_ne!(ka.as_bytes(), kb.as_bytes());
    // Entropy failure: typed, no row, no file.
    let before = s.blob_dir().entries().unwrap().len();
    assert!(matches!(
        s.blob_put_private_rng(b"x", Class::Object, |_| Err(BlobError::Entropy)),
        Err(StoreError::Blob(BlobError::Entropy))
    ));
    assert_eq!(s.blob_dir().entries().unwrap().len(), before);
    // The schema still refuses a public row with a key.
    assert!(s
        .conn
        .execute("UPDATE blobs SET public = 1 WHERE hash = ?1", [&h.0[..]],)
        .is_err());
    drop(t);
}

/// §10.4 step 1 includes delivered transit copies (dir = 2, state =
/// delivered), weighted by size × age; undelivered transit stays.
#[test]
fn step_one_evicts_delivered_transit_messages() {
    let (_t, mut s) = ready();
    let transit = |s: &mut Store, n: u64, body: &[u8], state: MsgState| -> i64 {
        let m = NewMessage {
            sender: PEER2,
            ..own(PEER, 0, body)
        };
        s.with_tx(Durability::Normal, |tx| {
            rows::insert_message(tx, &m, Seq { epoch: 1, n }, Dir::Transit, state)
        })
        .unwrap()
    };
    let old_big = transit(&mut s, 1, &[1; 4000], MsgState::Delivered);
    let pending = transit(&mut s, 2, &[2; 4000], MsgState::Sent);
    let new_small = transit(&mut s, 3, &[3; 100], MsgState::Delivered);
    let (hist, _) = s.queue_own_message(&own(PEER, 0, &[4; 300])).unwrap();
    s.set_message_state(hist, MsgState::Delivered, None)
        .unwrap();
    let r = s
        .prune_with(DiskSpace::UNKNOWN, Watermarks { high: 0, low: 0 })
        .unwrap();
    let step1: Vec<PrunedRef> = r
        .pruned
        .iter()
        .filter(|p| p.step == PruneStep::Evictable)
        .map(|p| p.item)
        .collect();
    assert_eq!(
        step1,
        vec![PrunedRef::Message(old_big), PrunedRef::Message(new_small)]
    );
    assert!(s.message(old_big).unwrap().is_none());
    assert!(s.message(new_small).unwrap().is_none());
    // Undelivered transit is never evicted; own history waits for export.
    assert!(s.message(pending).unwrap().is_some());
    assert!(s.message(hist).unwrap().is_some());
    assert!(r.export_offer_needed);
}

/// Item 7: `store_full` is persisted by every pass and read at open, so a
/// full store still refuses new custody, bulk and requests after a restart
/// (clean or not) until a pass with room clears it.
#[test]
fn store_full_survives_a_reopen() {
    let (t, mut s) = ready();
    let fx = fixture(&mut s);
    s.set_watermarks(Watermarks { high: 0, low: 0 }).unwrap();
    s.set_quota(Class::Custody, Some(1)).unwrap();
    let r = s
        .prune(DiskSpace {
            free: 1,
            total: 1000,
        })
        .unwrap();
    assert!(r.store_full);
    assert_eq!(r.full_classes, vec![Class::Custody]);
    drop(s); // unclean
    let (mut s, _) = open(t.path(), 1);
    assert!(s.store_full());
    assert_eq!(s.full_classes(), &[Class::Custody]);
    assert_eq!(
        s.custody_put(&PEER, b"after restart").unwrap(),
        CustodyPut::RejectFull
    );
    assert!(matches!(
        s.blob_put_public(b"bulk", Class::Kb, false),
        Err(StoreError::StoreFull)
    ));
    assert_eq!(
        s.request_put(&[0xC1; 32], &[1; 140]).unwrap(),
        RequestPut::Refused(RequestRefusal::StoreFull)
    );
    s.record_epoch(2).unwrap();
    s.close(1).unwrap();
    let (mut s, _) = open(t.path(), 1);
    assert!(s.store_full());
    // Room and a healthy disk: the next pass clears it, persistently.
    s.set_watermarks(Watermarks::with_high(u64::MAX / 2))
        .unwrap();
    s.set_quota(Class::Custody, None).unwrap();
    assert!(!s.prune(DiskSpace::UNKNOWN).unwrap().store_full);
    drop(s);
    let (mut s, _) = open(t.path(), 1);
    assert!(!s.store_full() && s.full_classes().is_empty());
    assert!(matches!(
        s.custody_put(&PEER, b"after restart").unwrap(),
        CustodyPut::Accepted(_)
    ));
    assert!(s.message(fx.undelivered).unwrap().is_some());
}

/// Item 7: `n` is allocated only together with its content — every number
/// handed out names a stored message (none abandoned), and a failed insert
/// hands out nothing. (`allocate_seq` is not public: see the compile-fail
/// doctest on `Store::queue_own_message`.)
#[test]
fn every_allocated_n_names_a_stored_message() {
    let (_t, mut s) = ready();
    let mut seqs = Vec::new();
    for i in 0..5u8 {
        let (id, q) = s.queue_own_message(&own(PEER, 0, &[i])).unwrap();
        assert_eq!(s.message(id).unwrap().unwrap().seq, q);
        seqs.push(q.n);
    }
    let big = vec![0u8; rows::MAX_BODY + 1];
    assert!(s.queue_own_message(&own(PEER, 0, &big)).is_err());
    let (_, q) = s.queue_own_message(&own(PEER, 0, b"next")).unwrap();
    seqs.push(q.n);
    assert_eq!(seqs, vec![1, 2, 3, 4, 5, 6]); // no hole
    let stored: Vec<u64> = s
        .messages_with(&PEER, 100)
        .unwrap()
        .iter()
        .map(|m| m.seq.n)
        .collect();
    assert_eq!(stored, seqs);
}

// ---------------------------------------------------------------------------
// F1 bounce: §7.3 floor, §7.2 (2) pending NodeIds (schema v2), thread queries
// ---------------------------------------------------------------------------

#[test]
fn custody_floor_sits_below_what_is_still_held_uncommitted() {
    let (_t, mut s) = ready();
    assert_eq!(s.custody_floor(&PEER).unwrap(), 0, "nothing ever issued");
    for item in [&b"p-1"[..], b"p-2", b"p-3"] {
        s.custody_put(&PEER, item).unwrap();
    }
    s.custody_put(&PEER2, b"q-4").unwrap();
    // Nothing committed: a re-enrolling member starts below seq 1.
    assert_eq!(s.custody_floor(&PEER).unwrap(), 0);
    assert_eq!(s.custody_floor(&PEER2).unwrap(), 3);
    // Committed items are below the floor; held ones stay above it.
    s.custody_commit(&PEER, 2).unwrap();
    assert_eq!(s.custody_floor(&PEER).unwrap(), 2);
    s.custody_commit(&PEER, 3).unwrap();
    assert_eq!(s.custody_floor(&PEER).unwrap(), s.custody_high().unwrap());
    // A NodeId with nothing held: the high-water mark (a first enrollment).
    assert_eq!(s.custody_floor(&[0xC0; 32]).unwrap(), 4);
}

#[test]
fn pending_node_ids_persist_until_removed() {
    let t = tempfile::tempdir().unwrap();
    {
        let (mut s, _) = open(t.path(), 1);
        s.record_epoch(1).unwrap();
        assert!(s.pending_node_id_put(&PEER).unwrap());
        assert!(!s.pending_node_id_put(&PEER).unwrap(), "idempotent");
        assert!(s.pending_node_id_put(&PEER2).unwrap());
        // No clean close: the rows were committed FULL when added.
        drop(s);
    }
    let (mut s, _) = open(t.path(), 1);
    assert_eq!(s.pending_node_ids().unwrap(), vec![PEER, PEER2]);
    assert!(s.pending_node_id_remove(&PEER).unwrap());
    assert!(!s.pending_node_id_remove(&PEER).unwrap());
    assert_eq!(s.pending_node_ids().unwrap(), vec![PEER2]);
    s.close(1).unwrap();
}

#[test]
fn a_v1_store_migrates_to_the_current_schema_in_place() {
    // A V1 store as a V1 build leaves it: the first migration alone (v4
    // adds columns, so the store cannot be faked by dropping tables).
    let t = tempfile::tempdir().unwrap();
    old_store(t.path(), 1).close().unwrap();
    let (mut s, _) = open(t.path(), 1);
    assert_eq!(
        pragma_int(&s.conn, "user_version").unwrap(),
        schema::SCHEMA_VERSION
    );
    assert_eq!(schema::SCHEMA_VERSION, 5);
    assert_totals_exact(&s);
    s.record_epoch(1).unwrap();
    assert!(s.pending_node_id_put(&PEER).unwrap());
    assert_eq!(s.pending_node_ids().unwrap(), vec![PEER]);
    s.foreign_conversation_upsert("lxmf", "me", ForeignKind::Dm, "dm:ab", None)
        .unwrap();
    let q = accepted(
        s.custody_put_fwd(&PEER2, b"v4", Fwd::FromMember(PEER))
            .unwrap(),
    );
    assert_eq!(raw_fwd(&s, q), (1, Some(PEER.to_vec())));
    assert_totals_exact(&s);
}

#[test]
fn thread_queries_read_the_newest_page_and_counts_only() {
    let (_t, mut s) = ready();
    let mut own_ids = Vec::new();
    for i in 0..3u8 {
        own_ids.push(s.queue_own_message(&own(PEER, 0, &[i])).unwrap().0);
    }
    let mut received = Vec::new();
    for n in 1..=4u64 {
        let (_, id) = s
            .receive_message(&inbound(PEER, &[0x40 + n as u8]), Seq { epoch: 1, n })
            .unwrap();
        received.push(s.message(id.unwrap()).unwrap().unwrap().local_seq);
    }
    s.queue_own_message(&own(PEER2, 0, b"elsewhere")).unwrap();
    s.set_message_state(own_ids[0], MsgState::Delivered, None)
        .unwrap();
    s.set_message_state(own_ids[1], MsgState::Relayed, None)
        .unwrap();

    let all = s.messages_with(&PEER, 100).unwrap();
    assert_eq!(all.len(), 7);
    let page = s.messages_newest(&PEER, 3).unwrap();
    assert_eq!(page, all[4..].to_vec(), "newest three, oldest first");
    assert_eq!(s.messages_newest(&PEER, 100).unwrap(), all);
    assert!(s.messages_newest(&[0xC0; 32], 5).unwrap().is_empty());

    let c = s.thread_counts(&PEER, received[1]).unwrap();
    assert_eq!(
        c,
        ThreadCounts {
            queued: 1,
            sent: 0,
            relayed: 1,
            delivered: 1,
            unread: 2,
        }
    );
    assert_eq!(s.thread_counts(&PEER, 0).unwrap().unread, 4);
    assert_eq!(s.thread_counts(&PEER2, 0).unwrap().queued, 1);
}

#[test]
fn own_unplanned_lists_numbered_messages_without_outbox_rows() {
    let (_t, mut s) = ready();
    let (planned, _) = s.queue_own_message(&own(PEER, 0, b"planned")).unwrap();
    s.outbox_put(planned, 0, 1, &PEER, None).unwrap();
    let (orphan, _) = s.queue_own_message(&own(PEER2, 0, b"orphan")).unwrap();
    let (done, _) = s.queue_own_message(&own(PEER, 0, b"done")).unwrap();
    s.set_message_state(done, MsgState::Delivered, None)
        .unwrap();
    s.receive_message(&inbound(PEER, b"in"), Seq { epoch: 1, n: 1 })
        .unwrap();
    let ids: Vec<i64> = s.own_unplanned().unwrap().iter().map(|m| m.id).collect();
    assert_eq!(ids, vec![orphan]);
}

// ---------------------------------------------------------------------------
// Layer 6 §3 — schema v3, the foreign message model
// ---------------------------------------------------------------------------

const BOARD: &str = "board-1";

fn fconv(s: &mut Store, net: &str, kind: ForeignKind, native: &str) -> i64 {
    s.foreign_conversation_upsert(net, BOARD, kind, native, None)
        .unwrap()
}

fn fin(conv: i64, sender: &str, id: Option<&str>, body: &str, unique: bool) -> ForeignInsert {
    ForeignInsert {
        conv,
        sender_native: sender.into(),
        sender_name: Some("W1AW".into()),
        auth: ForeignAuth::ChannelKey,
        native_msg_id: id.map(Into::into),
        body: body.into(),
        claim_ts: None,
        meta: None,
        id_globally_unique: unique,
    }
}

/// One inbound row; its id.
fn ingest1(s: &mut Store, row: ForeignInsert) -> i64 {
    s.foreign_insert_batch(vec![row]).unwrap()[0].unwrap()
}

fn consent() -> EdgeConsent<'static> {
    EdgeConsent {
        consent_hash: "sha256:warning",
        enabled_by: "loopback",
    }
}

fn mk_edge(s: &mut Store, src: i64, dst: i64) -> i64 {
    s.edge_create(&EdgeNew {
        src_conv: src,
        dst_conv: dst,
        honour_ok_to_mqtt: false,
        oversize: EdgeOversize::Refuse,
        consent: consent(),
    })
    .unwrap()
}

fn copy_send<'a>(
    edge: i64,
    origin: i64,
    dst: i64,
    text: &'a str,
    k: u32,
    n: u32,
) -> ForeignSend<'a> {
    ForeignSend {
        conv: dst,
        text,
        group_id: None,
        part_k: k,
        part_n: n,
        origin_msg: Some(origin),
        edge_id: Some(edge),
    }
}

/// A user-composed one-target send; the row id.
fn user_send(s: &mut Store, conv: i64, text: &str) -> i64 {
    let (_, ids) = s
        .send_group_create(
            text,
            "{\"targets\":[]}",
            &[GroupPart {
                conv,
                text,
                part_k: 1,
                part_n: 1,
            }],
        )
        .unwrap();
    ids[0]
}

fn fseq(s: &Store, id: i64) -> u64 {
    s.foreign_message(id).unwrap().unwrap().local_seq
}

#[test]
fn class_foreign_is_pinned_never_from_a_kind_and_never_refusable() {
    assert_eq!(Class::ALL.len(), 11);
    assert_eq!(Class::Foreign as u8, 9);
    assert_eq!(Class::ALL[9], Class::Foreign);
    assert!((0..=255u8).all(|k| Class::from_kind(k) != Class::Foreign));
    assert!(!Class::Foreign.is_refusable());
    assert!(Class::Foreign.has_quota());
    assert!(!Class::Human.has_quota() && !Class::Control.has_quota());
    assert_eq!(Class::Foreign.name(), "foreign");
    assert_eq!(Class::from_i64(9).unwrap(), Class::Foreign);
    let (_t, mut s) = ready();
    s.set_quota(Class::Foreign, Some(1000)).unwrap();
    assert_eq!(s.quota(Class::Foreign).unwrap(), Some(1000));
    assert!(matches!(
        s.set_quota(Class::Agent, Some(1)),
        Err(StoreError::Invalid(_))
    ));
}

/// A store at `version`, written by the first `version` migrations alone
/// (the shape a build of that version leaves on disk); open for seeding.
fn old_store(dir: &Path, version: usize) -> rusqlite::Connection {
    fs::create_dir_all(dir).unwrap();
    let conn = rusqlite::Connection::open(dir.join(DB_FILE)).unwrap();
    apply_key_and_pragmas(&conn, &key()).unwrap();
    for (i, sql) in schema::MIGRATIONS[..version].iter().enumerate() {
        conn.execute_batch(sql).unwrap();
        conn.pragma_update(None, "user_version", i as i64 + 1)
            .unwrap();
    }
    conn
}

/// A V2 store written by the V2 migrations alone (the shape a V2 build
/// leaves on disk).
fn write_v2_fixture(dir: &Path) {
    let conn = old_store(dir, 2);
    conn.execute(
        "INSERT INTO pending_node_ids (node_id, local_seq) VALUES (?1, 1)",
        [&PEER2[..]],
    )
    .unwrap();
    assert_eq!(pragma_int(&conn, "user_version").unwrap(), 2);
    conn.close().unwrap();
}

#[test]
fn a_v2_store_opens_at_the_current_schema_and_keeps_its_rows() {
    let t = tempfile::tempdir().unwrap();
    write_v2_fixture(t.path());
    let (mut s, _) = open(t.path(), 1);
    assert_eq!(
        pragma_int(&s.conn, "user_version").unwrap(),
        schema::SCHEMA_VERSION
    );
    assert_eq!(s.pending_node_ids().unwrap(), vec![PEER2]);
    s.record_epoch(1).unwrap();
    let c = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:LongFast:00");
    ingest1(&mut s, fin(c, "!a1b2c3d4", Some("1"), "hello", false));
    s.close(1).unwrap();
    let (s, _) = open(t.path(), 1);
    assert_eq!(s.foreign_conversation(c).unwrap().unwrap().messages, 1);
}

#[test]
fn a_store_newer_than_this_build_is_refused() {
    // The "V5 refused by a V4 build" rule is the same check one version up:
    // this build refuses user_version 6.
    let (t, s) = ready();
    s.conn.pragma_update(None, "user_version", 6).unwrap();
    s.close(1).unwrap();
    assert!(matches!(
        Store::open(t.path(), &key(), &params(1)),
        Err(StoreError::SchemaTooNew {
            found: 6,
            supported: 5
        })
    ));
}

#[test]
fn conversations_upsert_by_identity_and_refuse_bad_ids() {
    let (_t, mut s) = ready();
    let a = s
        .foreign_conversation_upsert(
            "meshtastic",
            BOARD,
            ForeignKind::Channel,
            "ch:a:1",
            Some("A"),
        )
        .unwrap();
    let again = s
        .foreign_conversation_upsert("meshtastic", BOARD, ForeignKind::Channel, "ch:a:1", None)
        .unwrap();
    assert_eq!(a, again);
    assert_eq!(
        s.foreign_conversation(a).unwrap().unwrap().title.as_deref(),
        Some("A")
    );
    let other_board = s
        .foreign_conversation_upsert(
            "meshtastic",
            "board-2",
            ForeignKind::Channel,
            "ch:a:1",
            None,
        )
        .unwrap();
    assert_ne!(a, other_board);
    for bad in ["", "111", "Mesh", "a b", &"x".repeat(33)] {
        assert!(
            matches!(
                s.foreign_conversation_upsert(bad, BOARD, ForeignKind::Dm, "dm:1", None),
                Err(StoreError::Invalid(_))
            ),
            "{bad:?}"
        );
    }
    assert!(matches!(
        s.foreign_conversation_upsert("lxmf", BOARD, ForeignKind::Dm, "", None),
        Err(StoreError::Invalid(_))
    ));
    let lx = fconv(&mut s, "lxmf", ForeignKind::Dm, "dm:ab");
    let mesh: Vec<i64> = s
        .foreign_conversations(Some("meshtastic"))
        .unwrap()
        .iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(mesh.len(), 2);
    assert!(!mesh.contains(&lx));
    assert_eq!(s.foreign_conversations(None).unwrap().len(), 3);
}

#[test]
fn ingest_batches_are_bounded_one_transaction_with_contiguous_seqs() {
    let (_t, mut s) = ready();
    let c = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:LongFast:00");
    let before = s.local_seq().unwrap();
    let rows = |n: usize| -> Vec<ForeignInsert> {
        (0..n)
            .map(|i| fin(c, "!a1", Some(&i.to_string()), "x", false))
            .collect()
    };
    assert!(matches!(
        s.foreign_insert_batch(rows(300)),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(s.local_seq().unwrap(), before);
    // One bad row (meta not an object) writes nothing.
    for meta in ["[1]", "{", "not json", "\"str\""] {
        let mut bad = rows(5);
        bad[3].meta = Some(meta.into());
        assert!(
            matches!(s.foreign_insert_batch(bad), Err(StoreError::Invalid(_))),
            "{meta}"
        );
    }
    let mut big = rows(1);
    big[0].body = "y".repeat(MAX_FOREIGN_BODY + 1);
    assert!(matches!(
        s.foreign_insert_batch(big),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(s.local_seq().unwrap(), before);
    assert_eq!(s.foreign_conversation(c).unwrap().unwrap().messages, 0);

    let ids = s.foreign_insert_batch(rows(256)).unwrap();
    assert_eq!(ids.len(), 256);
    let seqs: Vec<u64> = ids.iter().map(|id| fseq(&s, id.unwrap())).collect();
    assert_eq!(seqs, (before + 1..=before + 256).collect::<Vec<_>>());
    let conv = s.foreign_conversation(c).unwrap().unwrap();
    assert_eq!(
        (conv.messages, conv.unread, conv.last_seq),
        (256, 256, before + 256)
    );
    let mut max = rows(1);
    max[0].body = "y".repeat(MAX_FOREIGN_BODY);
    max[0].native_msg_id = Some("max".into());
    max[0].meta = Some("{\"truncated\":true,\"snr\":-7}".into());
    let id = s.foreign_insert_batch(max).unwrap()[0].unwrap();
    let row = s.foreign_message(id).unwrap().unwrap();
    assert_eq!(row.body.len(), MAX_FOREIGN_BODY);
    assert_eq!(row.state, ForeignState::Delivered);
    assert_eq!(row.dir, ForeignDir::In);
    // Pages and the since_seq read.
    assert_eq!(s.foreign_thread_page(c, 3).unwrap().last().unwrap().id, id);
    let since = s.foreign_messages(c, before + 250, 100).unwrap();
    assert_eq!(since.len(), 7);
    assert!(since.windows(2).all(|w| w[0].local_seq < w[1].local_seq));
    s.foreign_conversation_mark_read(c).unwrap();
    assert_eq!(s.foreign_conversation(c).unwrap().unwrap().unread, 0);
    assert!(matches!(
        s.foreign_insert_batch(vec![fin(c + 99, "!a1", None, "x", false)]),
        Err(StoreError::NotFound)
    ));
}

#[test]
fn dedupe_matrix_repeating_ids_need_the_body_in_the_window() {
    let (_t, mut s) = ready();
    let mesh = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:LongFast:00");
    let lx = fconv(&mut s, "lxmf", ForeignKind::Dm, "dm:ab");
    // Meshtastic: key + body within the last 32 rows.
    let first = s
        .foreign_insert_batch(vec![
            fin(mesh, "!a1", Some("7"), "hi", false),
            fin(mesh, "!a1", Some("7"), "hi", false), // same batch: duplicate
            fin(mesh, "!a1", Some("7"), "other", false), // id reused: new text
            fin(mesh, "!b2", Some("7"), "hi", false), // another sender
            fin(mesh, "!a1", None, "hi", false),      // no id: never a duplicate
            fin(mesh, "!a1", None, "hi", false),
        ])
        .unwrap();
    let pattern: Vec<bool> = first.iter().map(Option::is_some).collect();
    assert_eq!(pattern, [true, false, true, true, true, true]);
    assert_eq!(
        s.foreign_insert_batch(vec![fin(mesh, "!a1", Some("7"), "hi", false)])
            .unwrap(),
        [None]
    );
    // 32 newer rows push the original out of the window: a repeat is new.
    let filler: Vec<ForeignInsert> = (0..32)
        .map(|i| fin(mesh, "!c3", Some(&format!("f{i}")), "fill", false))
        .collect();
    s.foreign_insert_batch(filler).unwrap();
    assert!(s
        .foreign_insert_batch(vec![fin(mesh, "!a1", Some("7"), "hi", false)])
        .unwrap()[0]
        .is_some());
    // LXMF: the hash is globally unique — the key alone, any distance.
    assert!(s
        .foreign_insert_batch(vec![fin(lx, "ab", Some("h1"), "one", true)])
        .unwrap()[0]
        .is_some());
    let filler: Vec<ForeignInsert> = (0..40)
        .map(|i| fin(lx, "ab", Some(&format!("g{i}")), "fill", true))
        .collect();
    s.foreign_insert_batch(filler).unwrap();
    assert_eq!(
        s.foreign_insert_batch(vec![fin(lx, "ab", Some("h1"), "changed", true)])
            .unwrap(),
        [None]
    );
    // The key is per conversation.
    assert!(s
        .foreign_insert_batch(vec![fin(mesh, "ab", Some("h1"), "one", true)])
        .unwrap()[0]
        .is_some());
}

#[test]
fn the_dedupe_window_counts_inbound_rows_only() {
    let (_t, mut s) = ready();
    let a = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:a:1");
    let b = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:b:1");
    let e = mk_edge(&mut s, b, a);
    ingest1(&mut s, fin(a, "!a1", Some("7"), "hi", false));
    // 20 inbound rows interleaved with 40 own rows (user sends and relay
    // copies into `a`): 60 rows, but only 20 inbound ones.
    for i in 0..20 {
        ingest1(&mut s, fin(a, "!c3", Some(&format!("f{i}")), "fill", false));
        user_send(&mut s, a, "mine");
        let o = ingest1(
            &mut s,
            fin(b, "!b2", Some(&format!("o{i}")), "relay me", false),
        );
        s.foreign_queue_send(&copy_send(e, o, a, "[!b2/M] relay me", 1, 1))
            .unwrap();
    }
    assert_eq!(s.foreign_conversation(a).unwrap().unwrap().messages, 61);
    // Still within the last 32 inbound rows: a duplicate.
    assert_eq!(
        s.foreign_insert_batch(vec![fin(a, "!a1", Some("7"), "hi", false)])
            .unwrap(),
        [None]
    );
    // An own row never matches as an inbound duplicate.
    let own = user_send(&mut s, a, "echo");
    s.foreign_handoff_begin(own).unwrap();
    s.foreign_accepted(own, "99").unwrap();
    assert!(s
        .foreign_insert_batch(vec![fin(a, BOARD, Some("99"), "echo", false)])
        .unwrap()[0]
        .is_some());
    // 11 more inbound rows (32 newer than the original) push it out.
    let filler: Vec<ForeignInsert> = (0..11)
        .map(|i| fin(a, "!c3", Some(&format!("g{i}")), "fill", false))
        .collect();
    s.foreign_insert_batch(filler).unwrap();
    assert!(s
        .foreign_insert_batch(vec![fin(a, "!a1", Some("7"), "hi", false)])
        .unwrap()[0]
        .is_some());
}

#[test]
fn foreign_states_are_monotone() {
    let (_t, mut s) = ready();
    let dm = fconv(&mut s, "meshtastic", ForeignKind::Dm, "dm:!a1");
    let id = user_send(&mut s, dm, "hello");
    use ForeignState::*;
    let st = |s: &Store| s.foreign_message(id).unwrap().unwrap().state;
    assert_eq!(st(&s), Queued);
    let steps: [(ForeignState, Option<&str>, StateChange, ForeignState); 8] = [
        (
            Sent,
            Some("accepted by the radio"),
            StateChange::Changed,
            Sent,
        ),
        (Queued, None, StateChange::Unchanged, Sent),
        (Relayed, Some("implicit ack"), StateChange::Changed, Relayed),
        (Sent, None, StateChange::Unchanged, Relayed),
        // §4: failed is only a refusal before air; a relayed row was
        // transmitted, so a late NAK leaves it relayed.
        (Failed, Some("NO_CHANNEL"), StateChange::Unchanged, Relayed),
        (
            Delivered,
            Some("the recipient confirmed"),
            StateChange::Changed,
            Delivered,
        ),
        // Delivered never regresses.
        (Failed, Some("late NAK"), StateChange::Unchanged, Delivered),
        (Sent, None, StateChange::Unchanged, Delivered),
    ];
    for (to, ev, want, now) in steps {
        assert_eq!(
            s.foreign_set_state(id, to, ev, None).unwrap(),
            want,
            "{to:?}"
        );
        assert_eq!(st(&s), now);
    }
    // sent → failed stays (a NAK to a packet the radio queued is still
    // before air); failed is left only by a later delivered.
    let nak = user_send(&mut s, dm, "nak");
    let nst = |s: &Store| s.foreign_message(nak).unwrap().unwrap().state;
    let steps: [(ForeignState, Option<&str>, StateChange, ForeignState); 4] = [
        (Sent, None, StateChange::Changed, Sent),
        (Failed, Some("GOT_NAK"), StateChange::Changed, Failed),
        (Relayed, None, StateChange::Unchanged, Failed),
        (Delivered, None, StateChange::Changed, Delivered),
    ];
    for (to, reason, want, now) in steps {
        assert_eq!(
            s.foreign_set_state(nak, to, None, reason).unwrap(),
            want,
            "{to:?}"
        );
        assert_eq!(nst(&s), now);
    }
    assert!(ForeignState::Sent.may_become(Failed));
    assert!(!ForeignState::Relayed.may_become(Failed));
    assert!(!ForeignState::Failed.may_become(Relayed));
    assert!(ForeignState::Failed.may_become(Delivered));
    // Same state: only a different evidence is a change.
    assert_eq!(
        s.foreign_set_state(id, Delivered, Some("the recipient confirmed"), None)
            .unwrap(),
        StateChange::Unchanged
    );
    assert_eq!(
        s.foreign_set_state(id, Delivered, Some("read receipt"), None)
            .unwrap(),
        StateChange::Changed
    );
    // A reason travels with the state.
    let q = user_send(&mut s, dm, "two");
    assert_eq!(
        s.foreign_set_state(
            q,
            Queued,
            Some("waiting for the radio"),
            Some("region_unset")
        )
        .unwrap(),
        StateChange::Changed
    );
    let row = s.foreign_message(q).unwrap().unwrap();
    assert_eq!(row.reason_code.as_deref(), Some("region_unset"));
    // Inbound rows have no send state; unknown ids are NotFound.
    let inbound = ingest1(&mut s, fin(dm, "!a1", Some("9"), "hey", false));
    assert!(matches!(
        s.foreign_set_state(inbound, Sent, None, None),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        s.foreign_set_state(9999, Sent, None, None),
        Err(StoreError::NotFound)
    ));
}

#[test]
fn queue_order_puts_user_composed_rows_first_and_holds_pending_handoffs() {
    let (_t, mut s) = ready();
    let src = fconv(&mut s, "lxmf", ForeignKind::Group, "list:ops");
    let dst = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:LongFast:00");
    let dm = fconv(&mut s, "meshtastic", ForeignKind::Dm, "dm:!a1");
    let lx_dm = fconv(&mut s, "lxmf", ForeignKind::Dm, "dm:ab");
    let e = mk_edge(&mut s, src, dst);
    let o1 = ingest1(&mut s, fin(src, "ab", Some("h1"), "one", true));
    let o2 = ingest1(&mut s, fin(src, "ab", Some("h2"), "two", true));
    let c1 = s
        .foreign_queue_send(&copy_send(e, o1, dst, "[ab/LX] one", 1, 1))
        .unwrap();
    let c2 = s
        .foreign_queue_sends(&[
            copy_send(e, o2, dst, "[ab/LX] (1/2) tw", 1, 2),
            copy_send(e, o2, dst, "[ab/LX] (2/2) o", 2, 2),
        ])
        .unwrap();
    let u1 = user_send(&mut s, dm, "mine");
    let u2 = user_send(&mut s, dst, "mine too");
    user_send(&mut s, lx_dm, "other network");
    let ids = |s: &Store| -> Vec<i64> {
        s.foreign_queued("meshtastic", 100)
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect()
    };
    assert_eq!(ids(&s), vec![u1, u2, c1, c2[0], c2[1]]);
    assert_eq!(ids(&s)[..2], [u1, u2]);
    assert_eq!(s.foreign_queued("meshtastic", 2).unwrap().len(), 2);
    // A pending handoff is never offered again (§4 at most once) …
    s.foreign_handoff_begin(u1).unwrap();
    assert!(s.foreign_message(u1).unwrap().unwrap().handoff_pending);
    assert_eq!(ids(&s), vec![u2, c1, c2[0], c2[1]]);
    // … and Accepted clears it and records the native id.
    s.foreign_accepted(u1, "0x1234").unwrap();
    let row = s.foreign_message(u1).unwrap().unwrap();
    assert!(!row.handoff_pending);
    assert_eq!(row.native_msg_id.as_deref(), Some("0x1234"));
    s.foreign_set_state(u1, ForeignState::Sent, None, None)
        .unwrap();
    // A paused or disabled edge holds its copies back.
    assert!(s.edge_set_paused(e, true, None).unwrap());
    assert_eq!(ids(&s), vec![u2]);
    assert!(matches!(
        s.edge_set_paused(e, false, None),
        Err(StoreError::Invalid(_))
    ));
    assert!(s.edge_set_paused(e, false, Some(&consent())).unwrap());
    assert_eq!(ids(&s), vec![u2, c1, c2[0], c2[1]]);
    // A handoff only of a queued own row.
    assert!(matches!(
        s.foreign_handoff_begin(u1),
        Err(StoreError::Invalid(_))
    ));
    // meta fields round-trip; keys are restricted.
    assert!(s
        .foreign_set_meta(u2, "via_mqtt", Some(MetaValue::Bool(true)))
        .unwrap());
    assert!(s
        .foreign_set_meta(u2, "hops", Some(MetaValue::Int(3)))
        .unwrap());
    assert!(s.foreign_set_meta(u2, "hops", None).unwrap());
    assert_eq!(
        s.foreign_message(u2).unwrap().unwrap().meta,
        "{\"via_mqtt\":true}"
    );
    assert!(matches!(
        s.foreign_set_meta(u2, "a.b", Some(MetaValue::Str("x"))),
        Err(StoreError::Invalid(_))
    ));
    assert!(!s
        .foreign_set_meta(9999, "hops", Some(MetaValue::Int(1)))
        .unwrap());
}

#[test]
fn own_rows_are_either_user_composed_or_a_copy_on_a_live_edge() {
    let (_t, mut s) = ready();
    let a = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:a:1");
    let b = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:b:1");
    let dm = fconv(&mut s, "meshtastic", ForeignKind::Dm, "dm:!a1");
    let e = mk_edge(&mut s, a, b);
    let o = ingest1(&mut s, fin(a, "!a1", Some("1"), "x", false));
    let neither = ForeignSend {
        conv: b,
        text: "x",
        group_id: None,
        part_k: 1,
        part_n: 1,
        origin_msg: None,
        edge_id: None,
    };
    let both = ForeignSend {
        group_id: Some(1),
        ..copy_send(e, o, b, "x", 1, 1)
    };
    for bad in [
        neither,
        both,
        copy_send(e, o, b, "x", 2, 1),  // k > n
        copy_send(e, o, b, "x", 0, 1),  // k = 0
        copy_send(e, o, a, "x", 1, 1),  // not the edge's destination
        copy_send(e, o, dm, "x", 1, 1), // not the edge's destination
    ] {
        assert!(
            matches!(s.foreign_queue_send(&bad), Err(StoreError::Invalid(_))),
            "{bad:?}"
        );
    }
    let text = "y".repeat(MAX_FOREIGN_BODY + 1);
    assert!(matches!(
        s.foreign_queue_send(&copy_send(e, o, b, &text, 1, 1)),
        Err(StoreError::Invalid(_))
    ));
    // A copy's origin must be inbound on the edge's source.
    let own = user_send(&mut s, a, "mine");
    assert!(matches!(
        s.foreign_queue_send(&copy_send(e, own, b, "x", 1, 1)),
        Err(StoreError::Invalid(_))
    ));
    // Send groups validate atomically: one bad part queues nothing.
    let before = s.local_seq().unwrap();
    assert!(matches!(
        s.send_group_create(
            "t",
            "{}",
            &[
                GroupPart {
                    conv: dm,
                    text: "t",
                    part_k: 1,
                    part_n: 1
                },
                GroupPart {
                    conv: 9999,
                    text: "t",
                    part_k: 1,
                    part_n: 1
                },
            ],
        ),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        s.send_group_create("t", "not json", &[]),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(s.local_seq().unwrap(), before);
    let (g, rows) = s
        .send_group_create(
            "long text",
            "{\"targets\":[\"fc-1\",\"msg\"]}",
            &[
                GroupPart {
                    conv: dm,
                    text: "(1/2) long",
                    part_k: 1,
                    part_n: 2,
                },
                GroupPart {
                    conv: dm,
                    text: "(2/2) text",
                    part_k: 2,
                    part_n: 2,
                },
                GroupPart {
                    conv: b,
                    text: "long text",
                    part_k: 1,
                    part_n: 1,
                },
            ],
        )
        .unwrap();
    let group = s.send_group(g).unwrap().unwrap();
    assert_eq!(group.text, "long text");
    assert_eq!(group.local_seq, before + 1);
    let held: Vec<i64> = s.send_group_rows(g).unwrap().iter().map(|m| m.id).collect();
    assert_eq!(held, rows);
    let r0 = s.foreign_message(rows[0]).unwrap().unwrap();
    assert_eq!(
        (
            r0.dir,
            r0.state,
            r0.group_id,
            r0.part_k,
            r0.part_n,
            r0.sender_native.as_str()
        ),
        (ForeignDir::Own, ForeignState::Queued, Some(g), 1, 2, BOARD)
    );
    // A single send into an existing group.
    let extra = s
        .foreign_queue_send(&ForeignSend {
            group_id: Some(g),
            ..neither
        })
        .unwrap();
    assert_eq!(s.send_group_rows(g).unwrap().len(), 4);
    assert!(s.send_group_rows(g).unwrap().iter().any(|m| m.id == extra));
    assert!(matches!(
        s.foreign_queue_send(&ForeignSend {
            group_id: Some(g + 50),
            ..neither
        }),
        Err(StoreError::NotFound)
    ));
}

#[test]
fn edge_inputs_honour_cursor_and_relayed_seq_and_skip_copies() {
    let (_t, mut s) = ready();
    let a = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:a:1");
    let b = fconv(&mut s, "app:deltachat", ForeignKind::Group, "grp:9");
    let dm = fconv(&mut s, "meshtastic", ForeignKind::Dm, "dm:!a1");
    let history = ingest1(&mut s, fin(a, "!a1", Some("1"), "old", false));
    // Endpoints: never a DM, never one conversation, never twice.
    for (src, dst) in [(a, dm), (dm, b), (a, a), (a, 9999)] {
        assert!(s
            .edge_create(&EdgeNew {
                src_conv: src,
                dst_conv: dst,
                honour_ok_to_mqtt: false,
                oversize: EdgeOversize::Split,
                consent: consent(),
            })
            .is_err());
    }
    let e = mk_edge(&mut s, a, b);
    assert!(matches!(
        s.edge_create(&EdgeNew {
            src_conv: a,
            dst_conv: b,
            honour_ok_to_mqtt: true,
            oversize: EdgeOversize::Split,
            consent: consent(),
        }),
        Err(StoreError::Invalid(_))
    ));
    let back = mk_edge(&mut s, b, a); // the reverse direction is its own edge
    let edge = s.edge(e).unwrap().unwrap();
    assert_eq!(edge.cursor_seq, fseq(&s, history));
    assert_eq!(
        (
            edge.src_kind,
            edge.dst_kind,
            edge.oversize,
            edge.enabled,
            edge.paused
        ),
        (
            ForeignKind::Channel,
            ForeignKind::Group,
            EdgeOversize::Refuse,
            true,
            false
        )
    );
    let i1 = ingest1(&mut s, fin(a, "!a1", Some("2"), "new 1", false));
    let i2 = ingest1(&mut s, fin(a, "!b2", Some("3"), "new 2", false));
    let i3 = ingest1(&mut s, fin(a, "!b2", Some("4"), "new 3", false));
    let inputs = |s: &Store, e: i64| -> Vec<i64> {
        s.edge_inputs(e, 100)
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect()
    };
    // History (at or below cursor_seq) is never an input, never a copy.
    assert_eq!(inputs(&s, e), vec![i1, i2, i3]);
    assert!(matches!(
        s.foreign_queue_send(&copy_send(e, history, b, "x", 1, 1)),
        Err(StoreError::Invalid(_))
    ));
    // Queueing a copy advances relayed_seq past its origin in the same
    // transaction.
    let c1 = s
        .foreign_queue_send(&copy_send(e, i1, b, "[!a1/M] new 1", 1, 1))
        .unwrap();
    assert_eq!(s.edge(e).unwrap().unwrap().relayed_seq, fseq(&s, i1));
    assert_eq!(inputs(&s, e), vec![i2, i3]);
    // A refused input is skipped by advancing the mark explicitly.
    assert_eq!(
        s.edge_advance_relayed_seq(e, fseq(&s, i2)).unwrap(),
        fseq(&s, i2)
    );
    s.edge_count(e, EdgeCounter::Refused, 1).unwrap();
    assert_eq!(inputs(&s, e), vec![i3]);
    // Never backwards.
    assert_eq!(s.edge_advance_relayed_seq(e, 1).unwrap(), fseq(&s, i2));
    // The copy (own, in b) is never an input of the reverse edge; inbound
    // traffic in b is.
    let rb = ingest1(&mut s, fin(b, "bob@x", Some("m1"), "from dc", true));
    assert_eq!(inputs(&s, back), vec![rb]);
    let copy = s.foreign_message(c1).unwrap().unwrap();
    assert_eq!((copy.origin_msg, copy.edge_id), (Some(i1), Some(e)));
    // Disabled: no copies; re-enable needs consent and moves the cursor,
    // so what arrived while off is history.
    assert!(s.edge_set_enabled(e, false, None).unwrap());
    let off = ingest1(&mut s, fin(a, "!a1", Some("5"), "while off", false));
    assert!(matches!(
        s.foreign_queue_send(&copy_send(e, i3, b, "x", 1, 1)),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        s.edge_set_enabled(e, true, None),
        Err(StoreError::Invalid(_))
    ));
    assert!(s.edge_set_enabled(e, true, Some(&consent())).unwrap());
    assert!(!s.edge_set_enabled(e, true, Some(&consent())).unwrap());
    assert!(s.edge(e).unwrap().unwrap().cursor_seq >= fseq(&s, off));
    assert!(inputs(&s, e).is_empty());
    s.edge_count(e, EdgeCounter::Relayed, 2).unwrap();
    assert_eq!(s.edge(e).unwrap().unwrap().relayed, 2);
    assert_eq!(s.edge_list().unwrap().len(), 2);
    // Delete: the edge, its keys and its never-handed copies go.
    s.foreign_seen_insert(e, a, "!a1", "2", fseq(&s, i1))
        .unwrap();
    assert!(s.edge_delete(e).unwrap());
    assert!(!s.edge_delete(e).unwrap());
    assert!(s.foreign_message(c1).unwrap().is_none());
    assert_eq!(s.foreign_seen_count(e).unwrap(), 0);
    assert!(matches!(
        s.edge_count(e, EdgeCounter::Shed, 1),
        Err(StoreError::NotFound)
    ));
}

#[test]
fn foreign_seen_is_bounded_per_edge_and_evicts_the_oldest() {
    let (_t, mut s) = ready();
    let a = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:a:1");
    let b = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:b:1");
    let e = mk_edge(&mut s, a, b);
    let f = mk_edge(&mut s, b, a);
    s.foreign_seen_insert(f, b, "!z", "keep", 1).unwrap();
    let total = FOREIGN_SEEN_CAP + 4;
    for i in 0..total {
        s.foreign_seen_insert(e, a, "!a1", &i.to_string(), 100 + i)
            .unwrap();
    }
    assert_eq!(s.foreign_seen_count(e).unwrap(), FOREIGN_SEEN_CAP);
    for i in 0..4u64 {
        assert!(!s
            .foreign_seen_contains(Some(e), a, "!a1", &i.to_string())
            .unwrap());
    }
    assert!(s.foreign_seen_contains(Some(e), a, "!a1", "4").unwrap());
    assert!(s
        .foreign_seen_contains(Some(e), a, "!a1", &(total - 1).to_string())
        .unwrap());
    // Bounded per edge: the other edge keeps its key.
    assert_eq!(s.foreign_seen_count(f).unwrap(), 1);
    assert!(s.foreign_seen_contains(None, b, "!z", "keep").unwrap());
    assert!(!s.foreign_seen_contains(Some(e), b, "!z", "keep").unwrap());
    // A repeated key keeps the newer local_seq (it is not evicted next).
    s.foreign_seen_insert(e, a, "!a1", "4", 1_000_000).unwrap();
    s.foreign_seen_insert(e, a, "!a1", "new", 1_000_001)
        .unwrap();
    assert!(s.foreign_seen_contains(Some(e), a, "!a1", "4").unwrap());
    assert!(!s.foreign_seen_contains(Some(e), a, "!a1", "5").unwrap());
    assert!(matches!(
        s.foreign_seen_insert(9999, a, "!a1", "x", 1),
        Err(StoreError::NotFound)
    ));
}

/// The Layer 6 prune fixture: every 2a/2b category and every "never" item.
struct ForeignFixture {
    conv_a: i64,
    custody: u64,
    blob_superseded: BlobHash,
    edge: i64,
    edge_pinned_src: i64,
    a1: i64,
    a2: i64,
    a3: i64,
    c1: i64,
    c2: [i64; 2],
    c3_sent: i64,
    cp1: i64,
    p1: i64,
    user_queued: i64,
    user_failed: i64,
}

fn foreign_fixture(s: &mut Store) -> ForeignFixture {
    let custody = match s.custody_put(&PEER, &[8; 1000]).unwrap() {
        CustodyPut::Accepted(q) => q,
        other => panic!("{other:?}"),
    };
    s.custody_commit(&PEER, custody).unwrap();
    let blob_superseded = s
        .blob_put_public(&sample(3000, 3), Class::Update, false)
        .unwrap();
    s.blob_mark_superseded(&blob_superseded).unwrap();

    let a = fconv(s, "meshtastic", ForeignKind::Channel, "ch:a:1");
    let b = fconv(s, "meshtastic", ForeignKind::Channel, "ch:b:1");
    let p = fconv(s, "meshtastic", ForeignKind::Channel, "ch:pinned:1");
    let dm = fconv(s, "meshtastic", ForeignKind::Dm, "dm:!a1");
    s.foreign_conversation_set_pinned(p, true).unwrap();
    let edge = mk_edge(s, a, b);
    let edge_pinned_src = mk_edge(s, p, b);
    let a1 = ingest1(s, fin(a, "!a1", Some("1"), &"s".repeat(100), false));
    let a2 = ingest1(s, fin(a, "!a1", Some("2"), &"L".repeat(5000), false));
    let a3 = ingest1(s, fin(a, "!a1", Some("3"), &"m".repeat(300), false));
    let p1 = ingest1(s, fin(p, "!p1", Some("1"), "pinned", false));
    let c1 = s
        .foreign_queue_send(&copy_send(edge, a1, b, "[a1/M] s", 1, 1))
        .unwrap();
    let c2 = s
        .foreign_queue_sends(&[
            copy_send(edge, a2, b, "[a1/M] (1/2) L", 1, 2),
            copy_send(edge, a2, b, "[a1/M] (2/2) L", 2, 2),
        ])
        .unwrap();
    let c3_sent = s
        .foreign_queue_send(&copy_send(edge, a3, b, "[a1/M] m", 1, 1))
        .unwrap();
    s.foreign_set_state(c3_sent, ForeignState::Sent, None, None)
        .unwrap();
    let cp1 = s
        .foreign_queue_send(&copy_send(edge_pinned_src, p1, b, "[p1/M] pinned", 1, 1))
        .unwrap();
    let user_queued = user_send(s, dm, "never pruned while undelivered");
    let user_failed = user_send(s, dm, "failed is undelivered too");
    s.foreign_set_state(user_failed, ForeignState::Failed, None, Some("NO_CHANNEL"))
        .unwrap();
    s.foreign_seen_insert(edge, a, "!a1", "1", fseq(s, a1))
        .unwrap();
    ForeignFixture {
        conv_a: a,
        custody,
        blob_superseded,
        edge,
        edge_pinned_src,
        a1,
        a2,
        a3,
        c1,
        c2: [c2[0], c2[1]],
        c3_sent,
        cp1,
        p1,
        user_queued,
        user_failed,
    }
}

#[test]
fn pruning_runs_2a_then_2b_between_steps_2_and_3_and_keeps_the_never_list() {
    let (_t, mut s) = ready();
    let fx = foreign_fixture(&mut s);
    assert!(s.class_bytes(Class::Foreign).unwrap() > 5000);
    // Simulate a mark that lags the copy (the shed must advance it).
    s.conn
        .execute(
            "UPDATE relay_edges SET relayed_seq = 0 WHERE id = ?1",
            [fx.edge_pinned_src],
        )
        .unwrap();
    s.set_watermarks(Watermarks { high: 0, low: 0 }).unwrap();
    let r = s.prune(DiskSpace::UNKNOWN).unwrap();
    assert!(r.triggered && r.store_full);
    let steps: Vec<PruneStep> = r.pruned.iter().map(|p| p.step).collect();
    assert_eq!(
        steps,
        [
            PruneStep::Evictable,
            PruneStep::Superseded,
            PruneStep::ForeignInbound,
            PruneStep::ForeignInbound,
            PruneStep::ForeignInbound,
            PruneStep::ForeignInbound,
            PruneStep::RelayBacklog,
            PruneStep::RelayBacklog,
            PruneStep::RelayBacklog,
        ]
    );
    assert_eq!(r.pruned[0].item, PrunedRef::Custody(fx.custody));
    assert_eq!(r.pruned[1].item, PrunedRef::Blob(fx.blob_superseded));
    // 2a: size × age, the 5 KB row first; handed copies go with 2a.
    assert_eq!(r.pruned[2].item, PrunedRef::Foreign(fx.a2));
    let mut two_a: Vec<i64> = r.pruned[2..6]
        .iter()
        .map(|p| match p.item {
            PrunedRef::Foreign(id) => id,
            other => panic!("{other:?}"),
        })
        .collect();
    two_a.sort_unstable();
    let mut want = vec![fx.a1, fx.a2, fx.a3, fx.c3_sent];
    want.sort_unstable();
    assert_eq!(two_a, want);
    // 2b: oldest copy first; both parts of the split copy go together.
    assert_eq!(
        r.pruned[6..].iter().map(|p| p.item).collect::<Vec<_>>(),
        [
            PrunedRef::RelayCopy(fx.c1),
            PrunedRef::RelayCopy(fx.c2[0]),
            PrunedRef::RelayCopy(fx.cp1),
        ]
    );
    for id in [fx.c1, fx.c2[0], fx.c2[1], fx.cp1] {
        assert!(s.foreign_message(id).unwrap().is_none(), "{id}");
    }
    // Each shed is counted on its edge; relayed_seq moves past the origin.
    let edge = s.edge(fx.edge).unwrap().unwrap();
    assert_eq!(edge.shed, 2);
    let pinned_src = s.edge(fx.edge_pinned_src).unwrap().unwrap();
    assert_eq!(pinned_src.shed, 1);
    assert_eq!(pinned_src.relayed_seq, fseq(&s, fx.p1));
    assert!(s.edge_inputs(fx.edge_pinned_src, 10).unwrap().is_empty());
    // Never: undelivered user sends, pinned conversations, foreign_seen.
    for id in [fx.user_queued, fx.user_failed, fx.p1] {
        assert!(s.foreign_message(id).unwrap().is_some(), "{id}");
    }
    assert!(s
        .foreign_seen_contains(Some(fx.edge), fx.conv_a, "!a1", "1")
        .unwrap());
    assert_eq!(s.foreign_seen_count(fx.edge).unwrap(), 1);
    // Counters follow what is left (every inbound row of `a` is gone).
    let a = s.foreign_conversation(fx.conv_a).unwrap().unwrap();
    assert_eq!((a.messages, a.unread), (0, 0));
    let all = s.foreign_conversations(Some("meshtastic")).unwrap();
    assert!(all.iter().all(|c| c.unread <= c.messages));
    // A second pass finds nothing more of ours to shed.
    let r = s.prune(DiskSpace::UNKNOWN).unwrap();
    assert!(r
        .pruned
        .iter()
        .all(|p| !matches!(p.item, PrunedRef::Foreign(_) | PrunedRef::RelayCopy(_))));
    // Foreign rows are never refused, even with the store full.
    let c = s.foreign_message(fx.p1).unwrap().unwrap().conv;
    assert!(s
        .foreign_insert_batch(vec![fin(c, "!p1", Some("2"), "still ok", false)])
        .unwrap()[0]
        .is_some());
}

#[test]
fn the_foreign_shaping_quota_evicts_first_and_never_refuses() {
    let (_t, mut s) = ready();
    let c = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:mqtt:1");
    let p = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:pin:1");
    s.foreign_conversation_set_pinned(p, true).unwrap();
    // Committed custody over its own quota as well.
    for i in 0..3u8 {
        s.custody_put(&PEER, &[i; 2000]).unwrap();
    }
    s.custody_commit(&PEER, 3).unwrap();
    s.set_quota(Class::Custody, Some(2500)).unwrap();
    let rows: Vec<ForeignInsert> = (0..20)
        .map(|i| fin(c, "!a1", Some(&i.to_string()), &"q".repeat(1000), false))
        .collect();
    s.foreign_insert_batch(rows).unwrap();
    ingest1(&mut s, fin(p, "!p", Some("1"), &"z".repeat(3000), false));
    s.set_quota(Class::Foreign, Some(8000)).unwrap();
    let r = s.prune(DiskSpace::UNKNOWN).unwrap();
    assert!(!r.triggered, "global capacity is fine; quotas shaped it");
    let first_custody = r
        .pruned
        .iter()
        .position(|p| p.step == PruneStep::Quota(Class::Custody))
        .unwrap();
    assert!(first_custody > 0);
    assert!(r.pruned[..first_custody]
        .iter()
        .all(|p| p.step == PruneStep::Quota(Class::Foreign)
            && matches!(p.item, PrunedRef::Foreign(_))));
    assert!(s.class_bytes(Class::Foreign).unwrap() <= 8000);
    // Only the pinned row is left over a tiny quota: still never refused,
    // never a full class.
    s.set_quota(Class::Foreign, Some(1)).unwrap();
    let r = s.prune(DiskSpace::UNKNOWN).unwrap();
    assert!(!r.full_classes.contains(&Class::Foreign));
    assert!(!s.full_classes().contains(&Class::Foreign));
    assert_eq!(s.foreign_conversation(c).unwrap().unwrap().messages, 0);
    assert_eq!(s.foreign_conversation(p).unwrap().unwrap().messages, 1);
    assert_eq!(s.foreign_conversation(c).unwrap().unwrap().unread, 0);
    assert!(s
        .foreign_insert_batch(vec![fin(c, "!a1", Some("new"), "fits anyway", false)])
        .unwrap()[0]
        .is_some());
    s.set_quota(Class::Foreign, None).unwrap();
    assert_eq!(s.quota(Class::Foreign).unwrap(), None);
}

#[test]
fn foreign_rows_share_the_one_local_seq_with_111_rows() {
    let (_t, mut s) = ready();
    let c = fconv(&mut s, "lxmf", ForeignKind::Dm, "dm:ab");
    let (m1, _) = s.queue_own_message(&own(PEER, 0, b"111")).unwrap();
    let f1 = ingest1(&mut s, fin(c, "ab", Some("h1"), "lx", true));
    let (m2, _) = s.queue_own_message(&own(PEER, 0, b"111 again")).unwrap();
    let f2 = user_send(&mut s, c, "reply");
    let l = |id: i64| s.message(id).unwrap().unwrap().local_seq;
    let (l1, l2) = (l(m1), l(m2));
    let (fl1, fl2) = (fseq(&s, f1), fseq(&s, f2));
    assert!(l1 < fl1 && fl1 < l2 && l2 < fl2);
    // The send group drew one too, just before its row.
    let g = s.foreign_message(f2).unwrap().unwrap().group_id.unwrap();
    assert_eq!(s.send_group(g).unwrap().unwrap().local_seq + 1, fl2);
    assert_eq!(s.local_seq().unwrap(), fl2);
}

#[test]
fn a_second_copy_of_one_origin_on_one_edge_is_refused() {
    let (_t, mut s) = ready();
    let a = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:a:1");
    let b = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:b:1");
    let e = mk_edge(&mut s, a, b);
    let o1 = ingest1(&mut s, fin(a, "!a1", Some("1"), "one", false));
    let o2 = ingest1(&mut s, fin(a, "!a1", Some("2"), "two", false));
    let c1 = s
        .foreign_queue_send(&copy_send(e, o1, b, "[!a1/M] one", 1, 1))
        .unwrap();
    // §8 guard 6: the same origin again on the same edge fails closed,
    // alone or as a fresh split.
    let before = s.local_seq().unwrap();
    assert!(matches!(
        s.foreign_queue_send(&copy_send(e, o1, b, "[!a1/M] one", 1, 1)),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        s.foreign_queue_sends(&[
            copy_send(e, o1, b, "(1/2) o", 1, 2),
            copy_send(e, o1, b, "(2/2) ne", 2, 2),
        ]),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(s.local_seq().unwrap(), before);
    // The split parts of one copy still go together in one call.
    let parts = s
        .foreign_queue_sends(&[
            copy_send(e, o2, b, "(1/2) t", 1, 2),
            copy_send(e, o2, b, "(2/2) wo", 2, 2),
        ])
        .unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(s.edge(e).unwrap().unwrap().relayed_seq, fseq(&s, o2));
    // A further part of that copy in a later call is refused.
    assert!(matches!(
        s.foreign_queue_send(&copy_send(e, o2, b, "(3/3) x", 3, 3)),
        Err(StoreError::Invalid(_))
    ));
    // A part twice is unrepresentable (the UNIQUE index) and the batch
    // writes nothing.
    let o3 = ingest1(&mut s, fin(a, "!a1", Some("3"), "three", false));
    let before = s.local_seq().unwrap();
    assert!(s
        .foreign_queue_sends(&[
            copy_send(e, o3, b, "(1/2) th", 1, 2),
            copy_send(e, o3, b, "(1/2) th", 1, 2),
        ])
        .is_err());
    assert_eq!(s.local_seq().unwrap(), before);
    assert_eq!(s.edge(e).unwrap().unwrap().relayed_seq, fseq(&s, o2));
    // Several origins in ascending order in one call are fine.
    let o4 = ingest1(&mut s, fin(a, "!a1", Some("4"), "four", false));
    s.foreign_queue_sends(&[
        copy_send(e, o3, b, "[!a1/M] three", 1, 1),
        copy_send(e, o4, b, "[!a1/M] four", 1, 1),
    ])
    .unwrap();
    // B1-bounce F10: a relay copy is not cancelled (copies leave by shed,
    // disable or delete); the copy stays and its origin is never queued
    // again (the mark holds).
    assert!(matches!(s.foreign_cancel(c1), Err(StoreError::Invalid(_))));
    assert!(s.foreign_message(c1).unwrap().is_some());
    assert!(matches!(
        s.foreign_queue_send(&copy_send(e, o1, b, "[!a1/M] one", 1, 1)),
        Err(StoreError::Invalid(_))
    ));
    // Below the API the index refuses a duplicate part too.
    let raw = s.conn.execute(
        "INSERT INTO foreign_messages (conv, dir, sender_native, auth, body, local_seq, state,
                                       part_k, part_n, origin_msg, edge_id)
         VALUES (?1, 'own', 'x', 'none', 'dup', 999999, 'queued', 1, 2, ?2, ?3)",
        rusqlite::params![b, o2, e],
    );
    assert!(raw.is_err());
}

#[test]
fn cancel_drops_unsent_parts_only_one_by_one() {
    let (_t, mut s) = ready();
    let dm = fconv(&mut s, "meshtastic", ForeignKind::Dm, "dm:!a1");
    let parts: Vec<GroupPart<'_>> = (1..=4)
        .map(|k| GroupPart {
            conv: dm,
            text: "part",
            part_k: k,
            part_n: 4,
        })
        .collect();
    let (g, ids) = s.send_group_create("long", "{}", &parts).unwrap();
    // Part 1 was handed and accepted; part 2's handoff is pending (may have
    // been sent).
    s.foreign_handoff_begin(ids[0]).unwrap();
    s.foreign_accepted(ids[0], "0x1").unwrap();
    s.foreign_set_state(ids[0], ForeignState::Sent, None, None)
        .unwrap();
    s.foreign_handoff_begin(ids[1]).unwrap();
    for sent in [ids[0], ids[1]] {
        assert!(matches!(
            s.foreign_cancel(sent),
            Err(StoreError::Invalid(_))
        ));
    }
    // The unsent parts go, one per call.
    s.foreign_cancel(ids[2]).unwrap();
    let held: Vec<i64> = s.send_group_rows(g).unwrap().iter().map(|m| m.id).collect();
    assert_eq!(held, [ids[0], ids[1], ids[3]]);
    s.foreign_cancel(ids[3]).unwrap();
    let held: Vec<i64> = s.send_group_rows(g).unwrap().iter().map(|m| m.id).collect();
    assert_eq!(held, [ids[0], ids[1]]);
    assert!(matches!(
        s.foreign_cancel(ids[3]),
        Err(StoreError::NotFound)
    ));
    assert_eq!(s.foreign_conversation(dm).unwrap().unwrap().queued, 1);
    // An inbound row is not cancelled; an unknown id is NotFound.
    let inbound = ingest1(&mut s, fin(dm, "!a1", Some("9"), "hey", false));
    assert!(matches!(
        s.foreign_cancel(inbound),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(s.foreign_cancel(9999), Err(StoreError::NotFound)));
    // A cancelled row's id is never reused (AUTOINCREMENT): the newest
    // row was cancelled, and the next row still draws a fresh id.
    let last = user_send(&mut s, dm, "last");
    s.foreign_cancel(last).unwrap();
    let next = ingest1(&mut s, fin(dm, "!a1", Some("10"), "next", false));
    assert!(next > last);
    assert!(s.foreign_message(last).unwrap().is_none());
}

#[test]
fn edge_options_change_alone_and_report_whether_they_changed() {
    let (_t, mut s) = ready();
    let a = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:a:1");
    let b = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:b:1");
    let e = mk_edge(&mut s, a, b);
    let o1 = ingest1(&mut s, fin(a, "!a1", Some("1"), "one", false));
    s.foreign_queue_send(&copy_send(e, o1, b, "[!a1/M] one", 1, 1))
        .unwrap();
    let before = s.edge(e).unwrap().unwrap();
    assert!(!s.edge_set_options(e, false, EdgeOversize::Refuse).unwrap());
    assert!(s.edge_set_options(e, true, EdgeOversize::Split).unwrap());
    let after = s.edge(e).unwrap().unwrap();
    assert!(after.honour_ok_to_mqtt);
    assert_eq!(after.oversize, EdgeOversize::Split);
    assert_eq!(after.relayed_seq, before.relayed_seq, "the mark stays");
    assert_eq!(after.cursor_seq, before.cursor_seq, "history stays history");
    assert_eq!(after.consent_hash, before.consent_hash);
    assert_eq!(
        (after.enabled, after.paused),
        (before.enabled, before.paused)
    );
    assert!(s.edge_set_options(e, false, EdgeOversize::Split).unwrap());
    assert!(!s.edge(e).unwrap().unwrap().honour_ok_to_mqtt);
    assert!(matches!(
        s.edge_set_options(e + 100, true, EdgeOversize::Refuse),
        Err(StoreError::NotFound)
    ));
}

#[test]
fn disabling_an_edge_drops_its_never_handed_copies() {
    let (_t, mut s) = ready();
    let a = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:a:1");
    let b = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:b:1");
    let e = mk_edge(&mut s, a, b);
    let o1 = ingest1(&mut s, fin(a, "!a1", Some("1"), "one", false));
    let o2 = ingest1(&mut s, fin(a, "!a1", Some("2"), "two", false));
    let o3 = ingest1(&mut s, fin(a, "!a1", Some("3"), "three", false));
    let backlog = s
        .foreign_queue_send(&copy_send(e, o1, b, "[!a1/M] one", 1, 1))
        .unwrap();
    let pending = s
        .foreign_queue_send(&copy_send(e, o2, b, "[!a1/M] two", 1, 1))
        .unwrap();
    s.foreign_handoff_begin(pending).unwrap();
    let sent = s
        .foreign_queue_send(&copy_send(e, o3, b, "[!a1/M] three", 1, 1))
        .unwrap();
    s.foreign_handoff_begin(sent).unwrap();
    s.foreign_accepted(sent, "0x3").unwrap();
    s.foreign_set_state(sent, ForeignState::Sent, None, None)
        .unwrap();
    assert!(s.edge_set_enabled(e, false, None).unwrap());
    // A disabled edge has no backlog; a pending or handed copy is kept.
    assert!(s.foreign_message(backlog).unwrap().is_none());
    assert!(s.foreign_message(pending).unwrap().unwrap().handoff_pending);
    assert!(s.foreign_message(sent).unwrap().is_some());
    // Nothing dropped comes back: re-enabling moves the cursor.
    assert!(s.edge_set_enabled(e, true, Some(&consent())).unwrap());
    assert!(s.edge_inputs(e, 10).unwrap().is_empty());
    assert!(matches!(
        s.foreign_queue_send(&copy_send(e, o1, b, "[!a1/M] one", 1, 1)),
        Err(StoreError::Invalid(_))
    ));
    // edge_delete draws the same line.
    assert!(s.edge_delete(e).unwrap());
    assert!(s.foreign_message(pending).unwrap().is_some());
    assert!(s.foreign_message(sent).unwrap().is_some());
}

#[test]
fn a_copy_with_a_pending_handoff_is_pruned_as_handed_never_shed() {
    let (_t, mut s) = ready();
    let a = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:a:1");
    let b = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:b:1");
    let e = mk_edge(&mut s, a, b);
    let o1 = ingest1(&mut s, fin(a, "!a1", Some("1"), "one", false));
    let o2 = ingest1(&mut s, fin(a, "!a1", Some("2"), "two", false));
    let pending = s
        .foreign_queue_send(&copy_send(e, o1, b, "[!a1/M] one", 1, 1))
        .unwrap();
    s.foreign_handoff_begin(pending).unwrap();
    let backlog = s
        .foreign_queue_send(&copy_send(e, o2, b, "[!a1/M] two", 1, 1))
        .unwrap();
    s.set_watermarks(Watermarks { high: 0, low: 0 }).unwrap();
    let r = s.prune(DiskSpace::UNKNOWN).unwrap();
    let step_of = |item: PrunedRef| r.pruned.iter().find(|p| p.item == item).map(|p| p.step);
    assert_eq!(
        step_of(PrunedRef::Foreign(pending)),
        Some(PruneStep::ForeignInbound)
    );
    assert_eq!(step_of(PrunedRef::RelayCopy(pending)), None);
    assert_eq!(
        step_of(PrunedRef::RelayCopy(backlog)),
        Some(PruneStep::RelayBacklog)
    );
    // Only the never-handed copy counts as a shed.
    assert_eq!(s.edge(e).unwrap().unwrap().shed, 1);
    assert!(s.foreign_message(pending).unwrap().is_none());
    assert!(s.foreign_message(backlog).unwrap().is_none());
}

/// B1-bounce F7: `handoff` is a reserved meta key — the at-most-once mark
/// is representable only through its three calls.
#[test]
fn the_handoff_meta_key_is_reserved() {
    let (_t, mut s) = ready();
    let dm = fconv(&mut s, "meshtastic", ForeignKind::Dm, "dm:!a1");
    let u = user_send(&mut s, dm, "hello");
    for v in [Some(MetaValue::Str("pending")), None] {
        assert!(matches!(
            s.foreign_set_meta(u, "handoff", v),
            Err(StoreError::Invalid(_))
        ));
    }
    s.foreign_handoff_begin(u).unwrap();
    assert!(matches!(
        s.foreign_set_meta(u, "handoff", None),
        Err(StoreError::Invalid(_))
    ));
    assert!(s.foreign_message(u).unwrap().unwrap().handoff_pending);
}

/// B1-bounce F8: a pre-air refusal (no `Accepted`) withdraws the pending
/// mark; the row is offered, cancellable and droppable again. Anything
/// else is refused.
#[test]
fn a_pending_handoff_is_aborted_only_on_a_queued_own_row() {
    let (_t, mut s) = ready();
    let dm = fconv(&mut s, "meshtastic", ForeignKind::Dm, "dm:!a1");
    let a = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:a:1");
    let b = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:b:1");
    let e = mk_edge(&mut s, a, b);
    let offered = |s: &Store, id: i64| {
        s.foreign_queued("meshtastic", 50)
            .unwrap()
            .iter()
            .any(|m| m.id == id)
    };
    let u = user_send(&mut s, dm, "hello");
    // no mark yet
    assert!(matches!(
        s.foreign_handoff_abort(u),
        Err(StoreError::Invalid(_))
    ));
    s.foreign_handoff_begin(u).unwrap();
    assert!(!offered(&s, u));
    s.foreign_handoff_abort(u).unwrap();
    let row = s.foreign_message(u).unwrap().unwrap();
    assert!(!row.handoff_pending);
    assert_eq!(row.state, ForeignState::Queued);
    assert!(offered(&s, u), "offered again");
    // a second handoff after the abort is the one send of the act
    s.foreign_handoff_begin(u).unwrap();
    s.foreign_accepted(u, "0x1").unwrap();
    s.foreign_set_state(u, ForeignState::Sent, None, None)
        .unwrap();
    assert!(matches!(
        s.foreign_handoff_abort(u),
        Err(StoreError::Invalid(_))
    ));
    // a row past queued with a (stale) mark is not aborted
    let v = user_send(&mut s, dm, "two");
    s.foreign_handoff_begin(v).unwrap();
    s.foreign_set_state(v, ForeignState::Failed, None, Some("NO_CHANNEL"))
        .unwrap();
    assert!(matches!(
        s.foreign_handoff_abort(v),
        Err(StoreError::Invalid(_))
    ));
    // inbound and unknown rows
    let inbound = ingest1(&mut s, fin(dm, "!a1", Some("9"), "hey", false));
    assert!(matches!(
        s.foreign_handoff_abort(inbound),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        s.foreign_handoff_abort(9999),
        Err(StoreError::NotFound)
    ));
    // after an abort the row is cancellable again
    let w = user_send(&mut s, dm, "three");
    s.foreign_handoff_begin(w).unwrap();
    assert!(matches!(s.foreign_cancel(w), Err(StoreError::Invalid(_))));
    s.foreign_handoff_abort(w).unwrap();
    s.foreign_cancel(w).unwrap();
    // a relay copy: after an abort it is never-handed backlog again, so a
    // disable drops it
    let o = ingest1(&mut s, fin(a, "!a1", Some("1"), "one", false));
    let c = s
        .foreign_queue_send(&copy_send(e, o, b, "[!a1/M] one", 1, 1))
        .unwrap();
    s.foreign_handoff_begin(c).unwrap();
    s.foreign_handoff_abort(c).unwrap();
    assert!(offered(&s, c));
    assert!(s.edge_set_enabled(e, false, None).unwrap());
    assert!(s.foreign_message(c).unwrap().is_none());
}

/// B1-bounce F9: the parts of one copy in one call are cross-checked
/// (part 1 first, `part_k` = previous + 1, one `part_n`, complete by the
/// end of the call), and origins go in ascending order: any violation
/// fails the whole batch and writes nothing.
#[test]
fn one_batch_numbers_a_copys_parts_in_order_and_origins_ascending() {
    let (_t, mut s) = ready();
    let a = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:a:1");
    let b = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:b:1");
    let e = mk_edge(&mut s, a, b);
    let o1 = ingest1(&mut s, fin(a, "!a1", Some("1"), "one", false));
    let o2 = ingest1(&mut s, fin(a, "!a1", Some("2"), "two", false));
    let before = (
        s.local_seq().unwrap(),
        s.edge(e).unwrap().unwrap().relayed_seq,
    );
    let bad: [&[ForeignSend<'_>]; 6] = [
        // part_n differs between the parts of one copy
        &[
            copy_send(e, o1, b, "p", 1, 2),
            copy_send(e, o1, b, "p", 2, 3),
        ],
        // a part skipped
        &[
            copy_send(e, o1, b, "p", 1, 3),
            copy_send(e, o1, b, "p", 3, 3),
        ],
        // out of order
        &[
            copy_send(e, o1, b, "p", 2, 2),
            copy_send(e, o1, b, "p", 1, 2),
        ],
        // a copy that does not start at part 1
        &[copy_send(e, o1, b, "p", 2, 2)],
        // incomplete: part 2 of 2 could never be queued later
        &[copy_send(e, o1, b, "p", 1, 2)],
        // origins descending: o1 is below the mark o2 just moved
        &[
            copy_send(e, o2, b, "[!a1/M] two", 1, 1),
            copy_send(e, o1, b, "[!a1/M] one", 1, 1),
        ],
    ];
    for batch in bad {
        assert!(
            matches!(s.foreign_queue_sends(batch), Err(StoreError::Invalid(_))),
            "{batch:?}"
        );
        assert_eq!(
            (
                s.local_seq().unwrap(),
                s.edge(e).unwrap().unwrap().relayed_seq
            ),
            before,
            "nothing written"
        );
    }
    // the same copies in order: interleaving two origins' parts is fine as
    // long as each copy's own numbering holds and origins ascend
    let ids = s
        .foreign_queue_sends(&[
            copy_send(e, o1, b, "(1/2) o", 1, 2),
            copy_send(e, o1, b, "(2/2) ne", 2, 2),
            copy_send(e, o2, b, "(1/3) t", 1, 3),
            copy_send(e, o2, b, "(2/3) w", 2, 3),
            copy_send(e, o2, b, "(3/3) o", 3, 3),
        ])
        .unwrap();
    assert_eq!(ids.len(), 5);
    assert_eq!(s.edge(e).unwrap().unwrap().relayed_seq, fseq(&s, o2));
}

// ---------------------------------------------------------------------------
// Layer 7 §5.3 amendments 5 and 6, §5.8: schema v4, forward rows, Transit
// ---------------------------------------------------------------------------

const LEAF: [u8; 32] = [0xD0; 32];
const CARRIER: [u8; 32] = [0xD1; 32];
const STRANGER: [u8; 32] = [0xE0; 32];

fn accepted(p: CustodyPut) -> u64 {
    match p {
        CustodyPut::Accepted(q) => q,
        other => panic!("{other:?}"),
    }
}

/// The raw v4 columns of one `custody` row.
fn raw_fwd(s: &Store, seq: u64) -> (i64, Option<Vec<u8>>) {
    s.conn
        .query_row(
            "SELECT fwd, fwd_from FROM custody WHERE custody_seq = ?1",
            [seq as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

/// What one forward row of an `len`-byte item counts (item, `fwd_from`,
/// row overhead).
fn fwd_bytes(len: usize) -> u64 {
    (len + FWD_FROM_LEN) as u64 + prune::ROW_OVERHEAD
}

#[test]
fn a_v3_store_opens_at_v4_and_its_custody_rows_read_as_member_rows() {
    let t = tempfile::tempdir().unwrap();
    {
        let conn = old_store(t.path(), 3);
        for (item, committed) in [(&b"v3-open"[..], 0), (b"v3-done", 1)] {
            conn.execute(
                "INSERT INTO custody (dest, pkt_ref, bytes, committed, local_seq)
                 VALUES (?1, ?2, ?3, ?4, 1)",
                rusqlite::params![
                    &PEER[..],
                    &crate::custody::pkt_ref(item)[..],
                    item,
                    committed
                ],
            )
            .unwrap();
        }
        assert_eq!(pragma_int(&conn, "user_version").unwrap(), 3);
        conn.close().unwrap();
    }
    let (mut s, _) = open(t.path(), 1);
    s.record_epoch(1).unwrap();
    // Through v4 to the current schema (v5 adds a table, no custody column).
    assert_eq!(
        pragma_int(&s.conn, "user_version").unwrap(),
        schema::SCHEMA_VERSION
    );
    assert_totals_exact(&s);
    let cols: Vec<String> = s
        .conn
        .prepare("SELECT name FROM pragma_table_info('custody')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(
        cols,
        [
            "custody_seq",
            "dest",
            "pkt_ref",
            "bytes",
            "committed",
            "local_seq",
            "fwd",
            "fwd_from"
        ]
    );
    // Old rows are member rows: fwd 0, fwd_from NULL, fetched as before.
    assert_eq!(raw_fwd(&s, 1), (0, None));
    assert_eq!(raw_fwd(&s, 2), (0, None));
    let got = s.custody_fetch(&PEER, 0, 10).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!((got[0].committed, got[1].committed), (false, true));
    assert!(s.forward_pending(0, 10).unwrap().is_empty());
    assert_eq!(s.forward_item(1).unwrap(), None);
    assert_eq!(
        s.class_bytes(Class::Custody).unwrap(),
        (7 + 7) + 2 * prune::ROW_OVERHEAD
    );
    assert_eq!(s.class_bytes(Class::Transit).unwrap(), 0);
    // Dedup spans member and forward rows; the sequence continues.
    assert_eq!(
        s.custody_put(&PEER, b"v3-open").unwrap(),
        CustodyPut::Duplicate(1)
    );
    assert_eq!(
        s.custody_put_fwd(&STRANGER, b"v3-done", Fwd::FromLeaf(LEAF))
            .unwrap(),
        CustodyPut::Duplicate(2)
    );
    assert_eq!(accepted(s.custody_put(&PEER, b"v4-item").unwrap()), 3);
    s.close(1).unwrap();
}

#[test]
fn a_v4_store_is_refused_by_a_v3_build() {
    // A v4 store as a v4 build leaves it (this build would write v5).
    let t = tempfile::tempdir().unwrap();
    {
        let conn = old_store(t.path(), 4);
        conn.execute(
            "INSERT INTO custody (dest, pkt_ref, bytes, local_seq, fwd, fwd_from)
             VALUES (?1, ?2, x'667764', 1, 1, ?3)",
            rusqlite::params![
                &STRANGER[..],
                &crate::custody::pkt_ref(b"fwd")[..],
                &PEER[..]
            ],
        )
        .unwrap();
        conn.close().unwrap();
    }
    // A v3 build runs this `migrate` with its own list, `[V1, V2, V3]`.
    let conn = rusqlite::Connection::open(t.path().join(DB_FILE)).unwrap();
    apply_key_and_pragmas(&conn, &key()).unwrap();
    assert!(matches!(
        schema::migrate_with(&conn, &schema::MIGRATIONS[..3]),
        Err(StoreError::SchemaTooNew {
            found: 4,
            supported: 3
        })
    ));
    // Refused, never downgraded: the file is still v4 and this build opens it.
    assert_eq!(pragma_int(&conn, "user_version").unwrap(), 4);
    conn.close().unwrap();
    let (s, _) = open(t.path(), 1);
    assert_eq!(s.forward_pending(0, 10).unwrap().len(), 1);
    assert_totals_exact(&s);
}

#[test]
fn the_v4_checks_refuse_a_forward_row_without_its_origin() {
    let (_t, s) = ready();
    let ins = |fwd: i64, from: Option<&[u8]>, salt: u8| {
        s.conn.execute(
            "INSERT INTO custody (dest, pkt_ref, bytes, local_seq, fwd, fwd_from)
             VALUES (?1, ?2, x'00', 1, ?3, ?4)",
            rusqlite::params![&STRANGER[..], &[salt; 16][..], fwd, from],
        )
    };
    assert!(ins(4, Some(&PEER[..]), 1).is_err(), "fwd outside 0..=3");
    assert!(ins(1, None, 2).is_err(), "a forward row names its origin");
    assert!(ins(0, Some(&PEER[..]), 3).is_err(), "a member row has none");
    assert!(ins(2, Some(&[1u8; 31][..]), 4).is_err(), "a NodeId is 32 B");
    assert!(ins(3, Some(&CARRIER[..]), 5).is_ok());
    assert!(ins(0, None, 6).is_ok());
    // The typed reader agrees and fails closed on a pair it cannot hold.
    assert_eq!(Fwd::from_parts(0, None).unwrap(), Fwd::Member);
    assert_eq!(
        Fwd::from_parts(3, Some(CARRIER.to_vec())).unwrap(),
        Fwd::FromCarrier(CARRIER)
    );
    for (fwd, from) in [
        (1, None),
        (0, Some(PEER.to_vec())),
        (4, Some(PEER.to_vec())),
        (2, Some(vec![1; 31])),
    ] {
        assert!(matches!(
            Fwd::from_parts(fwd, from),
            Err(StoreError::Corrupt(_))
        ));
    }
}

// ---------------------------------------------------------------------------
// Layer 7 §5.8 (M1b): schema v5, the custody totals in O(1)
// ---------------------------------------------------------------------------

/// An origin whose NodeId is no text (NULs, 0xFF, a cut UTF-8 lead, a lone
/// continuation byte): the `x'03' ‖ fwd_from` key the v5 triggers build
/// must be the exact bytes.
const ODD: [u8; 32] = {
    let mut a = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        a[i] = [0x00, 0xFF, 0xC3, 0x28, 0x80][i % 5];
        i += 1;
    }
    a
};
/// An all-zero origin.
const NUL: [u8; 32] = [0; 32];

/// The pre-v5 readers' queries, kept here only as the oracle (never on a
/// production path): `class_bytes` of Custody / Transit (`{rows}` is the
/// `fwd` filter), `forward_origin_bytes`, `forward_stats`, `custody_stats`.
const OLD_CLASS_SQL: &str =
    "SELECT coalesce(sum(length(bytes) + coalesce(length(fwd_from), 0) + ?1), 0) FROM custody WHERE {rows}";
const OLD_ORIGIN_SQL: &str =
    "SELECT coalesce(sum(length(bytes) + length(fwd_from) + ?2), 0) FROM custody
     WHERE fwd != 0 AND committed = 0 AND fwd_from = ?1";
const OLD_STATS_SQL: &str = "SELECT count(*), coalesce(sum(length(bytes)), 0) FROM custody
     WHERE fwd != 0 AND committed = 0";
const OLD_CUSTODY_STATS_SQL: &str = "SELECT count(*), coalesce(sum(length(bytes)), 0) FROM custody";

/// What `custody_put_fwd` runs on its admission path besides the totals
/// read ([`prune::CUSTODY_TOTAL_SQL`]), verbatim from `rows.rs` (the O(1)
/// test checks they are still there).
const DEDUP_SQL: &str = "SELECT custody_seq FROM custody WHERE pkt_ref = ?1";
const INSERT_MEMBER_SQL: &str =
    "INSERT INTO custody (dest, pkt_ref, bytes, local_seq) VALUES (?1, ?2, ?3, ?4)";
const INSERT_FWD_SQL: &str = concat!(
    "INSERT INTO custody (dest, pkt_ref, bytes, local_seq, fwd, fwd_from)\n",
    "                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
);

type TotalsRow = (Vec<u8>, i64, i64, i64);

/// Every `custody_totals` row, in key order.
fn totals_rows(s: &Store) -> Vec<TotalsRow> {
    let mut st = s
        .conn
        .prepare("SELECT key, rows, item_bytes, from_bytes FROM custody_totals ORDER BY key")
        .unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

/// The v5 oracle: recompute every `custody_totals` row from the `custody`
/// rows with `SUM` / `GROUP BY` and require the table to be exactly that —
/// the three fixed keys present, one key per origin that holds protected
/// forward rows, no other key, no origin key at zero rows — then require
/// the five readers to return exactly what the pre-v5 sums return.
fn assert_totals_exact(s: &Store) {
    let sums = |filter: &str| -> (i64, i64, i64) {
        s.conn
            .query_row(
                &format!(
                    "SELECT count(*), coalesce(sum(length(bytes)), 0),
                            coalesce(sum(coalesce(length(fwd_from), 0)), 0)
                     FROM custody WHERE {filter}"
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    };
    let mut want: Vec<TotalsRow> = Vec::new();
    for (key, filter) in [
        (0u8, "fwd = 0"),
        (1, "fwd != 0"),
        (2, "fwd != 0 AND committed = 0"),
    ] {
        let (n, item, from) = sums(filter);
        want.push((vec![key], n, item, from));
    }
    let mut st = s
        .conn
        .prepare(
            "SELECT fwd_from, count(*), sum(length(bytes)), sum(length(fwd_from)) FROM custody
             WHERE fwd != 0 AND committed = 0 GROUP BY fwd_from",
        )
        .unwrap();
    let origins: Vec<TotalsRow> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    for (origin, n, item, from) in origins {
        let mut k = vec![0x03];
        k.extend_from_slice(&origin);
        want.push((k, n, item, from));
    }
    want.sort();
    let got = totals_rows(s);
    assert_eq!(got, want, "custody_totals differs from the rows' sums");
    assert!(
        got.iter().all(|(k, n, _, _)| k.len() == 1 || *n > 0),
        "an origin key at zero rows"
    );

    // The readers answer what the pre-v5 sums answer, to the byte.
    let old_class = |rows: &str| -> u64 {
        let v: i64 = s
            .conn
            .query_row(
                &OLD_CLASS_SQL.replace("{rows}", rows),
                [prune::ROW_OVERHEAD as i64],
                |r| r.get(0),
            )
            .unwrap();
        v as u64
    };
    assert_eq!(s.class_bytes(Class::Custody).unwrap(), old_class("fwd = 0"));
    assert_eq!(
        s.class_bytes(Class::Transit).unwrap(),
        old_class("fwd != 0")
    );
    let (n, b): (i64, i64) = s
        .conn
        .query_row(OLD_STATS_SQL, [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(s.forward_stats().unwrap(), (n as u64, b as u64));
    let (n, b): (i64, i64) = s
        .conn
        .query_row(OLD_CUSTODY_STATS_SQL, [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(s.custody_stats().unwrap(), (n as u64, b as u64));
    // Every origin ever stored (one with committed rows only reads 0) and
    // every NodeId these tests name.
    let mut ids: Vec<[u8; 32]> = s
        .conn
        .prepare("SELECT DISTINCT fwd_from FROM custody WHERE fwd_from IS NOT NULL")
        .unwrap()
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|r| id32(r.unwrap(), "fwd_from").unwrap())
        .collect();
    ids.extend([PEER, PEER2, LEAF, CARRIER, STRANGER, ODD, NUL, ME]);
    for o in ids {
        let old: i64 = s
            .conn
            .query_row(
                OLD_ORIGIN_SQL,
                rusqlite::params![&o[..], prune::ROW_OVERHEAD as i64],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(s.forward_origin_bytes(&o).unwrap(), old as u64, "{o:?}");
    }
}

/// The key of `origin`'s total.
fn origin_key(origin: &[u8; 32]) -> Vec<u8> {
    let mut k = vec![0x03];
    k.extend_from_slice(origin);
    k
}

#[test]
fn custody_totals_follow_every_write_path() {
    let (t, mut s) = ready();
    // An empty store: the three fixed keys, at zero.
    assert_eq!(
        totals_rows(&s),
        vec![(vec![0], 0, 0, 0), (vec![1], 0, 0, 0), (vec![2], 0, 0, 0)]
    );
    assert_totals_exact(&s);

    // A member put; forward puts from three origins (two NodeIds that are
    // no text).
    let m1 = accepted(s.custody_put(&PEER, &[1; 700]).unwrap());
    assert_totals_exact(&s);
    let a1 = accepted(
        s.custody_put_fwd(&STRANGER, &[2; 1000], Fwd::FromMember(PEER))
            .unwrap(),
    );
    assert_totals_exact(&s);
    let a2 = accepted(
        s.custody_put_fwd(&STRANGER, &[3; 1000], Fwd::FromMember(PEER))
            .unwrap(),
    );
    let b1 = accepted(
        s.custody_put_fwd(&PEER2, &[4; 500], Fwd::FromLeaf(ODD))
            .unwrap(),
    );
    assert_totals_exact(&s);
    let c1 = accepted(
        s.custody_put_fwd(&PEER2, &[5; 300], Fwd::FromCarrier(NUL))
            .unwrap(),
    );
    assert_totals_exact(&s);
    assert_eq!(s.forward_origin_bytes(&PEER).unwrap(), 2 * fwd_bytes(1000));
    assert_eq!(s.forward_origin_bytes(&ODD).unwrap(), fwd_bytes(500));
    assert_eq!(s.forward_origin_bytes(&NUL).unwrap(), fwd_bytes(300));

    // A duplicate, a share rejection and a quota rejection change nothing.
    let before = totals_rows(&s);
    assert_eq!(
        s.custody_put_fwd(&STRANGER, &[2; 1000], Fwd::FromLeaf(LEAF))
            .unwrap(),
        CustodyPut::Duplicate(a1)
    );
    assert_eq!(
        s.custody_put(&PEER, &[1; 700]).unwrap(),
        CustodyPut::Duplicate(m1)
    );
    assert_eq!(totals_rows(&s), before);
    s.set_quota(Class::Transit, Some(8 * (2 * fwd_bytes(1000) + 10)))
        .unwrap();
    assert_eq!(
        s.custody_put_fwd(&STRANGER, &[6; 1000], Fwd::FromMember(PEER))
            .unwrap(),
        CustodyPut::RejectFull,
        "over its share"
    );
    assert_eq!(totals_rows(&s), before);
    assert_totals_exact(&s);
    // Custody holds 700 + 64 B of protected rows: 100 B more are over the
    // quota, and the pruning pass admission runs finds nothing to evict.
    s.set_quota(Class::Custody, Some(800)).unwrap();
    assert_eq!(
        s.custody_put(&PEER, &[7; 100]).unwrap(),
        CustodyPut::RejectFull,
        "over the custody quota"
    );
    assert_eq!(totals_rows(&s), before);
    assert_totals_exact(&s);
    s.set_quota(Class::Custody, None).unwrap();

    // The commit and the refused mark move a row out of its origin's key;
    // an origin's last protected row takes its key with it.
    assert!(s.forward_commit(a1).unwrap());
    assert_totals_exact(&s);
    assert!(s.forward_commit(c1).unwrap());
    assert_totals_exact(&s);
    assert!(!totals_rows(&s).iter().any(|r| r.0 == origin_key(&NUL)));
    assert!(s.forward_refuse(b1).unwrap());
    assert_totals_exact(&s);
    assert!(!totals_rows(&s).iter().any(|r| r.0 == origin_key(&ODD)));
    let before = totals_rows(&s);
    assert!(!s.forward_refuse(b1).unwrap());
    assert!(!s.forward_commit(m1).unwrap());
    assert_eq!(
        totals_rows(&s),
        before,
        "an idempotent repeat moves nothing"
    );

    // The member's ack (MAILBOX_COMMIT) marks; pruning deletes.
    let m2 = accepted(s.custody_put(&PEER, &[8; 200]).unwrap());
    assert_eq!(s.custody_commit(&PEER, m1).unwrap(), 1);
    assert_totals_exact(&s);
    // Transit's quota evicts the three committed forward rows (step 1).
    s.set_quota(Class::Transit, Some(fwd_bytes(1000))).unwrap();
    let r = s.prune(DiskSpace::UNKNOWN).unwrap();
    let mut gone: Vec<PrunedRef> = r.pruned.iter().map(|p| p.item).collect();
    gone.sort_by_key(|p| match p {
        PrunedRef::Custody(q) => *q,
        _ => u64::MAX,
    });
    assert_eq!(
        gone,
        vec![
            PrunedRef::Custody(a1),
            PrunedRef::Custody(b1),
            PrunedRef::Custody(c1)
        ]
    );
    assert!(r
        .pruned
        .iter()
        .all(|p| p.step == PruneStep::Quota(Class::Transit)));
    assert_totals_exact(&s);
    s.set_quota(Class::Transit, None).unwrap();
    // The watermark evicts the committed member row.
    let r = s
        .prune_with(DiskSpace::UNKNOWN, Watermarks { high: 0, low: 0 })
        .unwrap();
    assert_eq!(
        r.pruned.iter().map(|p| p.item).collect::<Vec<_>>(),
        vec![PrunedRef::Custody(m1)]
    );
    assert_totals_exact(&s);
    s.prune_with(DiskSpace::UNKNOWN, Watermarks::with_high(u64::MAX / 2))
        .unwrap();
    assert!(!s.store_full());

    // A failed transaction: the rows and their totals roll back together.
    let before = totals_rows(&s);
    let res = s.with_tx(Durability::Normal, |tx| {
        tx.execute(
            "INSERT INTO custody (dest, pkt_ref, bytes, local_seq, fwd, fwd_from)
             VALUES (?1, ?2, x'0102', 1, 2, ?3)",
            rusqlite::params![&STRANGER[..], &[0x77u8; 16][..], &LEAF[..]],
        )?;
        tx.execute(
            "UPDATE custody SET committed = 1 WHERE custody_seq = ?1",
            [a2 as i64],
        )?;
        tx.execute("DELETE FROM custody WHERE custody_seq = ?1", [m2 as i64])?;
        let inside: i64 = tx.query_row(
            "SELECT count(*) FROM custody_totals WHERE key = ?1",
            [&origin_key(&LEAF)[..]],
            |r| r.get(0),
        )?;
        assert_eq!(inside, 1, "the triggers ran inside the transaction");
        Err::<(), _>(StoreError::Invalid("roll back"))
    });
    assert!(matches!(res, Err(StoreError::Invalid("roll back"))));
    assert_eq!(totals_rows(&s), before);
    assert_totals_exact(&s);
    // A statement that fails on its second row: the first row's trigger
    // work leaves with it.
    let dup = crate::custody::pkt_ref(&[3; 1000]);
    assert!(s
        .conn
        .execute(
            "INSERT INTO custody (dest, pkt_ref, bytes, local_seq, fwd, fwd_from)
             VALUES (?1, ?2, x'03', 1, 1, ?3), (?1, ?4, x'04', 1, 1, ?3)",
            rusqlite::params![&STRANGER[..], &[0x78u8; 16][..], &CARRIER[..], &dup[..]],
        )
        .is_err());
    assert_eq!(totals_rows(&s), before);
    assert_totals_exact(&s);

    // A reopen: the totals are on disk with the rows, and keep counting.
    s.close(1).unwrap();
    let (mut s, _) = open(t.path(), 1);
    assert_eq!(totals_rows(&s), before);
    assert_totals_exact(&s);
    accepted(
        s.custody_put_fwd(&STRANGER, &[9; 64], Fwd::FromLeaf(ODD))
            .unwrap(),
    );
    assert_totals_exact(&s);
    s.close(1).unwrap();
}

#[test]
fn a_v4_store_opens_at_v5_with_exact_custody_totals() {
    assert_eq!(schema::MIGRATIONS.len() as i64, schema::SCHEMA_VERSION);
    let t = tempfile::tempdir().unwrap();
    // Every kind of row a v4 hub holds: member rows open and committed;
    // protected forward rows of three origins (one with two rows, two
    // whose NodeIds are no text); a handed-over row of an origin that keeps
    // a protected one; an origin with committed rows only. Each: the item,
    // `committed`, `fwd`, `fwd_from`.
    type V4Row = (Vec<u8>, i64, i64, Option<[u8; 32]>);
    let rows: [V4Row; 8] = [
        (b"m-open".to_vec(), 0, 0, None),
        (b"m-done".to_vec(), 1, 0, None),
        (vec![1; 900], 0, 1, Some(PEER)),
        (vec![2; 1100], 0, 1, Some(PEER)),
        (vec![3; 70], 1, 1, Some(PEER)),
        (vec![4; 40], 0, 2, Some(ODD)),
        (vec![5; 33], 0, 3, Some(NUL)),
        (vec![6; 500], 1, 3, Some(CARRIER)),
    ];
    {
        let conn = old_store(t.path(), 4);
        for (i, (item, committed, fwd, from)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO custody (dest, pkt_ref, bytes, committed, local_seq, fwd, fwd_from)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    &STRANGER[..],
                    &crate::custody::pkt_ref(item)[..],
                    item,
                    committed,
                    i as i64 + 1,
                    fwd,
                    from.as_ref().map(|f| &f[..])
                ],
            )
            .unwrap();
        }
        let tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name LIKE 'custody_totals%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0, "a v4 store has no totals");
        assert_eq!(pragma_int(&conn, "user_version").unwrap(), 4);
        conn.close().unwrap();
    }
    let (mut s, _) = open(t.path(), 1);
    assert_eq!(pragma_int(&s.conn, "user_version").unwrap(), 5);
    assert_totals_exact(&s);
    let fwd_from = 32;
    let mut want: Vec<TotalsRow> = vec![
        (vec![0], 2, 12, 0),
        (vec![1], 6, 900 + 1100 + 70 + 40 + 33 + 500, 6 * fwd_from),
        (vec![2], 4, 900 + 1100 + 40 + 33, 4 * fwd_from),
        (origin_key(&PEER), 2, 2000, 2 * fwd_from),
        (origin_key(&ODD), 1, 40, fwd_from),
        (origin_key(&NUL), 1, 33, fwd_from),
    ];
    want.sort();
    assert_eq!(totals_rows(&s), want, "no key for CARRIER: committed only");
    assert_eq!(
        s.forward_origin_bytes(&PEER).unwrap(),
        fwd_bytes(900) + fwd_bytes(1100)
    );
    assert_eq!(s.forward_origin_bytes(&CARRIER).unwrap(), 0);
    // The triggers carry on from the backfill.
    let q = accepted(
        s.custody_put_fwd(&STRANGER, &[7; 10], Fwd::FromCarrier(CARRIER))
            .unwrap(),
    );
    assert_totals_exact(&s);
    assert!(s.forward_commit(q).unwrap());
    assert_eq!(s.custody_commit(&STRANGER, 1).unwrap(), 1);
    assert_totals_exact(&s);
    s.close(1).unwrap();
}

#[test]
fn a_v5_store_is_refused_by_a_v4_build() {
    let (t, mut s) = ready();
    accepted(
        s.custody_put_fwd(&STRANGER, b"fwd", Fwd::FromMember(PEER))
            .unwrap(),
    );
    s.close(1).unwrap();
    // A v4 build runs this `migrate` with its own list, `[V1, V2, V3, V4]`.
    let conn = rusqlite::Connection::open(t.path().join(DB_FILE)).unwrap();
    apply_key_and_pragmas(&conn, &key()).unwrap();
    assert!(matches!(
        schema::migrate_with(&conn, &schema::MIGRATIONS[..4]),
        Err(StoreError::SchemaTooNew {
            found: 5,
            supported: 4
        })
    ));
    // Refused, never downgraded: the file is still v5 and this build opens it.
    assert_eq!(pragma_int(&conn, "user_version").unwrap(), 5);
    conn.close().unwrap();
    let (s, _) = open(t.path(), 1);
    assert_eq!(s.forward_pending(0, 10).unwrap().len(), 1);
    assert_totals_exact(&s);
}

/// `EXPLAIN QUERY PLAN` of `sql` (parameters bound to NULL): one detail
/// line per plan step.
fn plan(s: &Store, sql: &str) -> Vec<String> {
    let mut st = s
        .conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap();
    let n = st.parameter_count();
    st.query_map(
        rusqlite::params_from_iter(std::iter::repeat_n(rusqlite::types::Null, n)),
        |r| r.get::<_, String>(3),
    )
    .unwrap()
    .map(|r| r.unwrap())
    .collect()
}

/// A plan step that walks all of `custody` (not `custody_totals`).
fn scans_custody(plan: &[String]) -> bool {
    plan.iter().any(|d| {
        let mut w = d.split_whitespace();
        w.next() == Some("SCAN") && w.next() == Some("custody")
    })
}

/// Run the four statements of a forward put's admission path exactly as
/// `custody_put_fwd` runs them (the dedup lookup, the origin's total, the
/// class total, the insert with its triggers) for `item` from `origin`;
/// SQLite's own counters summed over the four: (full-scan steps, VM steps).
/// A trigger's subprogram counts toward the statement that fired it.
fn put_counters(s: &Store, item: &[u8], origin: &[u8; 32]) -> (i32, i32) {
    use rusqlite::StatementStatus::{FullscanStep, VmStep};
    let pkt_ref = crate::custody::pkt_ref(item);
    let mut out = (0, 0);
    let mut count = |st: &rusqlite::Statement<'_>| {
        out.0 += st.get_status(FullscanStep);
        out.1 += st.get_status(VmStep);
    };
    let mut st = s.conn.prepare(DEDUP_SQL).unwrap();
    let dup: Option<i64> = st
        .query_row([&pkt_ref[..]], |r| r.get(0))
        .optional()
        .unwrap();
    assert_eq!(dup, None);
    count(&st);
    for key in [origin_key(origin), vec![0x01]] {
        let mut st = s.conn.prepare(prune::CUSTODY_TOTAL_SQL).unwrap();
        let n: i64 = st.query_row([&key[..]], |r| r.get(0)).unwrap();
        assert!(n > 0);
        count(&st);
    }
    let mut st = s.conn.prepare(INSERT_FWD_SQL).unwrap();
    st.execute(rusqlite::params![
        &STRANGER[..],
        &pkt_ref[..],
        item,
        1,
        1,
        &origin[..]
    ])
    .unwrap();
    count(&st);
    out
}

#[test]
fn the_admission_path_never_scans_custody() {
    // The statements are the ones `custody_put_fwd` runs (rows.rs).
    let rows_rs = include_str!("rows.rs");
    for sql in [DEDUP_SQL, INSERT_MEMBER_SQL, INSERT_FWD_SQL] {
        assert!(rows_rs.contains(sql), "rows.rs no longer runs {sql}");
    }

    // A hub holding 5,000 forward rows (8 origins, a third committed)
    // and 50 member rows, seeded in one transaction through the triggers.
    let origins: Vec<[u8; 32]> = (0..8u8).map(|i| [0x40 + i; 32]).collect();
    let (_t, mut s) = ready();
    s.with_tx(Durability::Normal, |tx| {
        let mut st = tx.prepare(
            "INSERT INTO custody (dest, pkt_ref, bytes, committed, local_seq, fwd, fwd_from)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6)",
        )?;
        for i in 0..5_050u32 {
            let mut item = i.to_le_bytes().to_vec();
            item.extend_from_slice(&[i as u8; 60]);
            let member = i >= 5_000;
            st.execute(rusqlite::params![
                &STRANGER[..],
                &crate::custody::pkt_ref(&item)[..],
                &item,
                i % 3 == 0,
                if member { 0 } else { 1 + i % 3 },
                (!member).then_some(&origins[i as usize % 8][..])
            ])?;
        }
        Ok(())
    })
    .unwrap();
    assert_totals_exact(&s);
    assert_eq!(s.forward_stats().unwrap().0, 5_000 - 1_667);

    // Plans: the reads on the admission path search, never walk `custody`.
    // (The two INSERTs have no plan of their own; what they cost, their
    // trigger programs and index writes included, is in the counters
    // below.)
    for (what, sql) in [("dedup", DEDUP_SQL), ("totals", prune::CUSTODY_TOTAL_SQL)] {
        let p = plan(&s, sql);
        assert!(!scans_custody(&p), "{what}: {p:?}");
    }
    assert!(plan(&s, DEDUP_SQL)
        .iter()
        .any(|d| d.starts_with("SEARCH custody USING")));
    assert!(plan(&s, prune::CUSTODY_TOTAL_SQL)
        .iter()
        .any(|d| d.starts_with("SEARCH custody_totals USING PRIMARY KEY")));
    // The check has teeth: the pre-v5 class sum walks every row.
    assert!(scans_custody(&plan(
        &s,
        &OLD_CLASS_SQL.replace("{rows}", "fwd != 0")
    )));
    let mut st = s
        .conn
        .prepare(&OLD_CLASS_SQL.replace("{rows}", "fwd != 0"))
        .unwrap();
    let _: i64 = st
        .query_row([prune::ROW_OVERHEAD as i64], |r| r.get(0))
        .unwrap();
    assert!(st.get_status(rusqlite::StatementStatus::FullscanStep) >= 5_000);
    drop(st);

    // Counters: the put into 5,050 rows takes no full-scan step and the
    // same VM steps as into a store of two rows (a B-tree seek is one
    // step at any depth), so its work does not grow with the rows held.
    let big = put_counters(&s, b"probe into a full hub", &origins[0]);
    let (_t2, mut small) = ready();
    accepted(small.custody_put(&PEER, &[1; 64]).unwrap());
    accepted(
        small
            .custody_put_fwd(&STRANGER, &[2; 64], Fwd::FromMember(origins[0]))
            .unwrap(),
    );
    let little = put_counters(&small, b"probe into a full hub", &origins[0]);
    assert_eq!(big.0, 0, "full-scan steps");
    assert_eq!(big, little, "(full-scan, VM) steps: 5,052 rows vs 3");
    assert_totals_exact(&s);
    assert_totals_exact(&small);

    // And the real put, through admission, into the full hub.
    accepted(
        s.custody_put_fwd(&STRANGER, b"a real put", Fwd::FromLeaf(origins[1]))
            .unwrap(),
    );
    assert_totals_exact(&s);
}

/// The readers and the admission path answer from `custody_totals`, not
/// from the rows (so the O(1) plans above are the plans the put runs), and
/// a total that would go negative fails the write instead of reading as
/// room.
#[test]
fn admission_reads_the_totals_and_a_drifted_total_fails_closed() {
    let (_t, mut s) = ready();
    accepted(s.custody_put(&PEER, &[1; 100]).unwrap());
    let q = accepted(
        s.custody_put_fwd(&STRANGER, &[2; 100], Fwd::FromMember(PEER))
            .unwrap(),
    );
    assert_totals_exact(&s);
    // Shift every stored total by 1,000 item bytes: the readers follow.
    s.conn
        .execute(
            "UPDATE custody_totals SET item_bytes = item_bytes + 1000",
            [],
        )
        .unwrap();
    assert_eq!(
        s.class_bytes(Class::Custody).unwrap(),
        1100 + prune::ROW_OVERHEAD
    );
    assert_eq!(
        s.class_bytes(Class::Transit).unwrap(),
        fwd_bytes(100) + 1000
    );
    assert_eq!(s.forward_stats().unwrap(), (1, 1100));
    assert_eq!(
        s.forward_origin_bytes(&PEER).unwrap(),
        fwd_bytes(100) + 1000
    );
    // So does admission: an origin whose stored total fills its share is
    // refused although its rows leave room; another origin is admitted.
    let share = s.transit_share().unwrap();
    s.conn
        .execute(
            "UPDATE custody_totals SET item_bytes = ?2 WHERE key = ?1",
            rusqlite::params![origin_key(&PEER), share as i64],
        )
        .unwrap();
    assert_eq!(
        s.custody_put_fwd(&STRANGER, &[3; 10], Fwd::FromMember(PEER))
            .unwrap(),
        CustodyPut::RejectFull
    );
    accepted(
        s.custody_put_fwd(&STRANGER, &[3; 10], Fwd::FromMember(PEER2))
            .unwrap(),
    );
    // A total that would go below zero fails the write (the counters'
    // CHECK) and the row is left as it was.
    s.conn
        .execute("UPDATE custody_totals SET rows = 0 WHERE key = x'01'", [])
        .unwrap();
    assert!(matches!(s.forward_commit(q), Err(StoreError::Sql(_))));
    assert!(!s.forward_item(q).unwrap().unwrap().committed);
}

// ---------------------------------------------------------------------------
// Layer 7 §5.8 (M1c): no pass over the rows held on a per-item path
// ---------------------------------------------------------------------------

/// The per-item statements on `custody`, verbatim from `rows.rs` and
/// `prune.rs` (the tests check they are still there): a member's
/// `MAILBOX_COMMIT`, its `ENROLL_OK` floor, the forwarder's two pages, a
/// forward commit, a prune delete.
const COMMIT_SQL: &str = "UPDATE custody SET committed = 1
             WHERE dest = ?1 AND custody_seq <= ?2 AND committed = 0";
const FLOOR_SQL: &str = "SELECT min(custody_seq) FROM custody WHERE dest = ?1 AND committed = 0";
const PENDING_TAIL: &str = "WHERE fwd != 0 AND committed = 0 AND custody_seq > ?1
             ORDER BY custody_seq LIMIT ?2";
const PENDING_SQL: &str =
    "SELECT custody_seq, dest, pkt_ref, bytes, fwd, fwd_from, committed FROM custody
             WHERE fwd != 0 AND committed = 0 AND custody_seq > ?1
             ORDER BY custody_seq LIMIT ?2";
const LENS_SQL: &str =
    "SELECT custody_seq, dest, pkt_ref, length(bytes), fwd, fwd_from FROM custody
             WHERE fwd != 0 AND committed = 0 AND custody_seq > ?1
             ORDER BY custody_seq LIMIT ?2";
const FWD_COMMIT_SQL: &str = "UPDATE custody SET committed = 1
             WHERE custody_seq = ?1 AND fwd != 0 AND committed = 0";
const PRUNE_DELETE_SQL: &str = "DELETE FROM custody WHERE custody_seq = ?1";
/// [`Store::forward_waits_for_dest`] (Layer 7 M4e): it names
/// `committed = 0`, so it searches `custody_open_by_dest` too.
const WAITS_SQL: &str = "SELECT custody_seq FROM custody
             WHERE dest = ?1 AND custody_seq > ?2 AND fwd != 0 AND committed = 0
             LIMIT ?3";

/// A forward origin no seeded row uses.
const PROBE: [u8; 32] = [0x77; 32];

/// A hub's `custody` for the M1c checks, seeded in one transaction through
/// the triggers and the indexes: `fwd` forward rows to STRANGER over
/// `origins` origins (row `i` committed when `every != 0` and `i % every
/// == 0`), then `member` rows for PEER, every one committed (acknowledged
/// by its member's `MAILBOX_COMMIT`, waiting for step 1).
fn seeded_hub(fwd: u32, origins: u32, every: u32, member: u32) -> (tempfile::TempDir, Store) {
    let (t, mut s) = ready();
    let ids: Vec<[u8; 32]> = (0..origins.max(1))
        .map(|i| {
            let mut a = [0x40; 32];
            a[..4].copy_from_slice(&i.to_le_bytes());
            a
        })
        .collect();
    s.with_tx(Durability::Normal, |tx| {
        let mut st = tx.prepare(
            "INSERT INTO custody (dest, pkt_ref, bytes, committed, local_seq, fwd, fwd_from)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for i in 0..fwd + member {
            let mut item = i.to_le_bytes().to_vec();
            item.extend_from_slice(&[i as u8; 60]);
            let m = i >= fwd;
            st.execute(rusqlite::params![
                if m { &PEER[..] } else { &STRANGER[..] },
                &crate::custody::pkt_ref(&item)[..],
                &item,
                m || (every != 0 && i % every == 0),
                i64::from(i) + 1,
                if m { 0 } else { 1 + i % 3 },
                (!m).then_some(&ids[(i % origins.max(1)) as usize][..])
            ])?;
        }
        set_u64(tx, K_LOCAL_SEQ, u64::from(fwd + member))
    })
    .unwrap();
    (t, s)
}

/// SQLite's own counters of one statement: (full-scan steps, VM steps). A
/// trigger's program and the index writes count toward the statement that
/// fired them.
fn status(st: &rusqlite::Statement<'_>) -> (i32, i32) {
    use rusqlite::StatementStatus::{FullscanStep, VmStep};
    (st.get_status(FullscanStep), st.get_status(VmStep))
}

/// Run a writing statement once: the rows it changed and its counters.
fn write_counted(s: &Store, sql: &str, p: impl rusqlite::Params) -> (usize, (i32, i32)) {
    let mut st = s.conn.prepare(sql).unwrap();
    let n = st.execute(p).unwrap();
    (n, status(&st))
}

/// Run a reading statement once: its first column per row and its
/// counters.
fn read_counted(s: &Store, sql: &str, p: impl rusqlite::Params) -> (Vec<Option<i64>>, (i32, i32)) {
    let mut st = s.conn.prepare(sql).unwrap();
    let v = st
        .query_map(p, |r| r.get::<_, Option<i64>>(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    (v, status(&st))
}

#[test]
fn the_per_item_reads_search_a_partial_index() {
    let rows_rs = include_str!("rows.rs");
    for sql in [COMMIT_SQL, FLOOR_SQL, LENS_SQL, FWD_COMMIT_SQL, WAITS_SQL] {
        assert!(rows_rs.contains(sql), "rows.rs no longer runs {sql}");
    }
    assert!(rows_rs.contains(&format!(
        "SELECT {{FORWARD_COLS}} FROM custody\n             {PENDING_TAIL}"
    )));
    assert!(include_str!("prune.rs").contains(PRUNE_DELETE_SQL));
    assert!(
        !rows_rs.contains(OLD_CUSTODY_STATS_SQL),
        "custody_stats reads the totals"
    );
    // A hub of 7,000 rows: the plans are bounded searches of the v5
    // partial indexes (before v5: a member's committed rows walked by
    // `custody_by_dest`, the rowid range walked across member and
    // committed rows, a `SCAN custody` for step 1).
    let (_t, s) = seeded_hub(5_000, 8, 3, 2_000);
    for (sql, want) in [
        (
            COMMIT_SQL,
            "SEARCH custody USING COVERING INDEX custody_open_by_dest (dest=? AND custody_seq<?)",
        ),
        (
            FLOOR_SQL,
            "SEARCH custody USING COVERING INDEX custody_open_by_dest (dest=?)",
        ),
        (
            WAITS_SQL,
            "SEARCH custody USING INDEX custody_open_by_dest (dest=? AND custody_seq>?)",
        ),
        (
            PENDING_SQL,
            "SEARCH custody USING INDEX custody_forward_open (custody_seq>?)",
        ),
        (
            LENS_SQL,
            "SEARCH custody USING INDEX custody_forward_open (custody_seq>?)",
        ),
        (
            prune::EVICTABLE_CUSTODY_SQL,
            "SEARCH custody USING INDEX custody_committed (fwd>? AND fwd<?)",
        ),
        (
            FWD_COMMIT_SQL,
            "SEARCH custody USING INTEGER PRIMARY KEY (rowid=?)",
        ),
        (
            PRUNE_DELETE_SQL,
            "SEARCH custody USING INTEGER PRIMARY KEY (rowid=?)",
        ),
    ] {
        assert_eq!(plan(&s, sql), vec![want.to_string()], "{sql}");
    }
    assert_totals_exact(&s);
}

/// The per-item writes and reads cost the same VM steps, and no full-scan
/// step, in a hub of 10 rows, of 7,000 rows over 8 origins and of 7,000
/// rows over 2,500 origins: a member's commit (2,000 of its rows already
/// committed in the big hubs), its floor, the forwarder's page past the
/// last forward row (2,000 member rows after it), a forward commit, a
/// prune delete, the custody totals read a pass at rest makes, and the
/// step-1 read of a class with no committed row (5,000 protected forward
/// rows and 2,000 committed member rows beside it).
#[test]
fn per_item_statements_cost_the_same_at_any_size() {
    type Costs = Vec<(&'static str, (i32, i32))>;
    let mut seen: Vec<Costs> = Vec::new();
    for (fwd, origins, member) in [(6, 2, 4), (5_000, 8, 2_000), (5_000, 2_500, 2_000)] {
        let (_t, mut s) = seeded_hub(fwd, origins, 3, member);
        let q = accepted(
            s.custody_put_fwd(&STRANGER, b"m1c forward probe", Fwd::FromCarrier(PROBE))
                .unwrap(),
        );
        let m = accepted(s.custody_put(&PEER, b"m1c member probe").unwrap());
        let done = accepted(s.custody_put(&PEER2, b"m1c committed probe").unwrap());
        assert_eq!(s.custody_commit(&PEER2, done).unwrap(), 1);
        let mut c: Costs = Vec::new();
        let (page, k) = read_counted(&s, LENS_SQL, rusqlite::params![fwd, 256]);
        assert_eq!(page, vec![Some(q as i64)]);
        c.push(("forward page", k));
        let (floor, k) = read_counted(&s, FLOOR_SQL, [&PEER[..]]);
        assert_eq!(floor, vec![Some(m as i64)]);
        c.push(("floor", k));
        let (n, k) = write_counted(&s, COMMIT_SQL, rusqlite::params![&PEER[..], m as i64]);
        assert_eq!(n, 1);
        c.push(("custody_commit", k));
        let (n, k) = write_counted(&s, FWD_COMMIT_SQL, [q as i64]);
        assert_eq!(n, 1);
        c.push(("forward_commit", k));
        let (n, k) = write_counted(&s, PRUNE_DELETE_SQL, [done as i64]);
        assert_eq!(n, 1);
        c.push(("prune delete", k));
        let (_, k) = read_counted(&s, prune::CUSTODY_TOTAL_SQL, [&[0x01u8][..]]);
        c.push(("transit total", k));
        assert_totals_exact(&s);
        seen.push(c);
    }
    for c in &seen {
        assert!(c.iter().all(|(_, (full, _))| *full == 0), "{c:?}");
    }
    assert_eq!(seen[0], seen[1], "10 rows vs 7,000");
    assert_eq!(seen[1], seen[2], "8 origins vs 2,500");

    // Step 1 for a class with nothing committed reads none of the rows
    // beside it: the same steps over 4 committed member rows and 6
    // protected forward rows as over 2,000 and 5,000.
    let mut step1 = Vec::new();
    for (fwd, member) in [(6, 4), (5_000, 2_000)] {
        let (_t, s) = seeded_hub(fwd, 8, 0, member);
        let (rows, k) = read_counted(&s, prune::EVICTABLE_CUSTODY_SQL, [1, 3]);
        assert!(rows.is_empty());
        step1.push(k);
    }
    assert_eq!(step1[0], step1[1]);
    assert_eq!(step1[0].0, 0);
}

/// §10.4: a pass with nothing over (the 60 s capacity pass of every node
/// with a fabric, and admission while the store is full or a quota hit
/// clears) reads no candidate list and writes nothing while the persisted
/// outcome is unchanged; it writes the outcome when it changes, as every
/// pass did.
#[test]
fn a_pass_at_rest_reads_no_candidate_and_writes_nothing() {
    for (fwd, member) in [(6, 4), (5_000, 2_000)] {
        let (_t, mut s) = seeded_hub(fwd, 8, 3, member);
        // A committed row no list can hold (an age below zero is a corrupt
        // row): only a pass that reads step 1 meets it.
        s.conn
            .execute(
                "INSERT INTO custody (dest, pkt_ref, bytes, committed, local_seq)
                 VALUES (?1, ?2, x'00', 1, -1)",
                rusqlite::params![&PEER2[..], &[0x99u8; 16][..]],
            )
            .unwrap();
        // The first pass writes the outcome a fresh store has never had.
        let first = s.prune(DiskSpace::UNKNOWN).unwrap();
        assert!(!first.triggered && first.pruned.is_empty() && !first.store_full);
        let changes = s.conn.total_changes();
        let reps = 20u32;
        let t0 = std::time::Instant::now();
        for _ in 0..reps {
            let r = s.prune(DiskSpace::UNKNOWN).unwrap();
            assert_eq!(
                r,
                PruneReport {
                    triggered: false,
                    live_before: r.live_before,
                    live_after: r.live_before,
                    pruned: Vec::new(),
                    store_full: false,
                    export_offer_needed: false,
                    full_classes: Vec::new(),
                }
            );
        }
        let per_pass = t0.elapsed() / reps;
        assert_eq!(
            s.conn.total_changes(),
            changes,
            "a pass at rest writes nothing"
        );
        eprintln!(
            "a pass at rest over {} rows: {per_pass:?} per pass (debug build)",
            fwd + member + 1
        );
        // New disk figures change the outcome: written, as before.
        let disk = DiskSpace {
            free: 1 << 40,
            total: 1 << 41,
        };
        assert!(s.prune(disk).unwrap().pruned.is_empty());
        assert!(s.conn.total_changes() > changes);
        assert_eq!(
            get_u64(&s.conn, "capacity.disk_free").unwrap(),
            Some(1 << 40)
        );
        // A store_full left by the last pass is cleared by a pass at rest,
        // in memory and on disk.
        let r = s
            .prune_with(DiskSpace::UNKNOWN, Watermarks { high: 0, low: 0 })
            .unwrap_err();
        assert!(matches!(r, StoreError::Corrupt("custody.local_seq")), "{r}");
        s.conn
            .execute("DELETE FROM custody WHERE local_seq = -1", [])
            .unwrap();
        s.set_quota(Class::Transit, Some(1 << 40)).unwrap();
        let full = s
            .prune_with(DiskSpace::UNKNOWN, Watermarks { high: 0, low: 0 })
            .unwrap();
        assert!(full.store_full && s.store_full());
        let r = s.prune(DiskSpace::UNKNOWN).unwrap();
        assert!(!r.store_full && !s.store_full() && r.pruned.is_empty());
        assert_eq!(get_i64(&s.conn, "capacity.store_full").unwrap(), Some(0));
        assert_totals_exact(&s);
    }
}

/// The v5 objects present in `sqlite_master`, by name.
fn v5_objects(s: &Store) -> Vec<String> {
    let mut st = s
        .conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE name LIKE 'custody_totals%'
             OR name IN ('custody_open_by_dest', 'custody_forward_open', 'custody_committed')
             ORDER BY name",
        )
        .unwrap();
    st.query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

#[test]
fn open_heals_a_missing_v5_trigger_index_or_table() {
    let all = [
        "custody_committed",
        "custody_forward_open",
        "custody_open_by_dest",
        "custody_totals",
        "custody_totals_delete",
        "custody_totals_insert",
        "custody_totals_update",
    ];
    let (t, mut s) = ready();
    assert_eq!(v5_objects(&s), all);
    accepted(s.custody_put(&PEER, &[1; 100]).unwrap());
    accepted(
        s.custody_put_fwd(&STRANGER, &[2; 100], Fwd::FromMember(PEER))
            .unwrap(),
    );
    // A whole store: one read of sqlite_master, no write.
    let changes = s.conn.total_changes();
    schema::heal(&s.conn).unwrap();
    assert_eq!(s.conn.total_changes(), changes);

    // A lost insert trigger: rows written without it leave the totals
    // behind (Transit still reads one row).
    s.conn
        .execute_batch("DROP TRIGGER custody_totals_insert;")
        .unwrap();
    let q = accepted(
        s.custody_put_fwd(&STRANGER, &[3; 200], Fwd::FromLeaf(LEAF))
            .unwrap(),
    );
    accepted(s.custody_put(&PEER2, &[4; 300]).unwrap());
    assert_eq!(s.class_bytes(Class::Transit).unwrap(), fwd_bytes(100));
    s.close(1).unwrap();
    // The open puts it back and rebuilds the totals from the rows.
    let (mut s, _) = open(t.path(), 1);
    assert_eq!(v5_objects(&s), all);
    assert_totals_exact(&s);
    accepted(s.custody_put(&PEER, &[5; 10]).unwrap());
    assert!(s.forward_commit(q).unwrap());
    assert_totals_exact(&s);

    // Lost indexes: the open re-creates them and the reads use them again.
    s.conn
        .execute_batch("DROP INDEX custody_committed; DROP INDEX custody_open_by_dest;")
        .unwrap();
    assert_ne!(
        plan(&s, FLOOR_SQL),
        vec!["SEARCH custody USING COVERING INDEX custody_open_by_dest (dest=?)".to_string()]
    );
    s.close(1).unwrap();
    let (mut s, _) = open(t.path(), 1);
    assert_eq!(v5_objects(&s), all);
    assert_eq!(
        plan(&s, FLOOR_SQL),
        vec!["SEARCH custody USING COVERING INDEX custody_open_by_dest (dest=?)".to_string()]
    );
    assert_totals_exact(&s);

    // A lost totals table: every custody write fails (fail closed) until
    // the open re-creates it and counts the rows again.
    s.conn.execute_batch("DROP TABLE custody_totals;").unwrap();
    assert!(s.custody_put(&PEER, &[6; 10]).is_err());
    s.close(1).unwrap();
    let (mut s, _) = open(t.path(), 1);
    assert_eq!(v5_objects(&s), all);
    assert_totals_exact(&s);
    accepted(s.custody_put(&PEER, &[6; 10]).unwrap());
    assert_totals_exact(&s);
    s.close(1).unwrap();
}

/// M1c question (Layer 7 §5.1-§5.2; L5 §7.3, §9): a forward row is held
/// only for a `dest` that is not a member then, but that `dest` may enroll
/// here later. Its mailbox serves every row of its `dest`: the `ENROLL_OK`
/// floor counts the forward row (so its cursor starts below it) and
/// `MAILBOX_FETCH` (`custody_fetch`, the fabric's push) streams it. The
/// member is given the row, so its `MAILBOX_COMMIT` committing it is right
/// (the item reached its dest) and the row leaves the forwarder's pages;
/// a commit below it commits nothing.
#[test]
fn a_dest_that_enrolls_later_is_given_its_forward_rows_and_commits_them() {
    let (_t, mut s) = ready();
    accepted(s.custody_put(&PEER, b"another member's item").unwrap());
    let q = accepted(
        s.custody_put_fwd(&STRANGER, b"held for forwarding", Fwd::FromMember(PEER))
            .unwrap(),
    );
    let floor = s.custody_floor(&STRANGER).unwrap();
    assert_eq!(floor, q - 1);
    assert_eq!(
        s.custody_fetch(&STRANGER, floor, 10).unwrap(),
        vec![CustodyItem {
            custody_seq: q,
            dest: STRANGER,
            bytes: b"held for forwarding".to_vec(),
            committed: false,
        }]
    );
    assert_eq!(s.custody_commit(&STRANGER, floor).unwrap(), 0);
    assert_eq!(s.forward_pending(0, 10).unwrap().len(), 1);
    assert_eq!(s.custody_commit(&STRANGER, q).unwrap(), 1);
    assert!(s.forward_item(q).unwrap().unwrap().committed);
    assert!(s.forward_pending(0, 10).unwrap().is_empty());
    assert!(s.forward_pending_lens(0, 10).unwrap().is_empty());
    assert_eq!(s.forward_stats().unwrap(), (0, 0));
    assert_eq!(s.forward_origin_bytes(&PEER).unwrap(), 0);
    assert_eq!(
        s.custody_floor(&STRANGER).unwrap(),
        s.custody_high().unwrap()
    );
    assert!(!s.custody_fetch(&PEER, 0, 10).unwrap()[0].committed);
    assert_totals_exact(&s);
}

/// A small deterministic generator (xorshift64*): a failing seed replays.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.below(v.len() as u64) as usize]
    }
}

/// A store holding every kind of §10.4 item — member and forward custody
/// open and committed, re-fetchable and superseded blobs (pinned or not),
/// own and inbound history, transit messages, foreign inbound rows and
/// relay copies queued and handed — with sizes, ages and classes drawn
/// from `seed`: the same seed builds the same store, row for row.
fn random_store(seed: u64) -> (tempfile::TempDir, Store) {
    let (t, mut s) = ready();
    let mut r = Rng::new(seed);
    let classes = [
        Class::Human,
        Class::Kb,
        Class::Update,
        Class::Object,
        Class::Custody,
        Class::Transit,
        Class::Foreign,
    ];
    let a = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:a:1");
    let b = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:b:1");
    let p = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:p:1");
    s.foreign_conversation_set_pinned(p, true).unwrap();
    let edge = mk_edge(&mut s, a, b);
    let mut n = 0u32;
    let mut item = |len: u64| -> Vec<u8> {
        n += 1;
        let mut v = n.to_le_bytes().to_vec();
        v.resize(4 + len as usize, n as u8);
        v
    };
    let mut seq = 0u64;
    for _ in 0..(30 + r.below(50)) {
        match r.below(11) {
            0 | 1 => {
                let dest = r.pick(&[PEER, PEER2]);
                let q = accepted(s.custody_put(&dest, &item(r.below(3000))).unwrap());
                if r.chance(60) {
                    s.custody_commit(&dest, q).unwrap();
                }
            }
            2 | 3 => {
                let fwd = match r.below(3) {
                    0 => Fwd::FromMember(PEER),
                    1 => Fwd::FromLeaf(LEAF),
                    _ => Fwd::FromCarrier(CARRIER),
                };
                let q = accepted(
                    s.custody_put_fwd(&STRANGER, &item(r.below(3000)), fwd)
                        .unwrap(),
                );
                if r.chance(50) {
                    s.forward_commit(q).unwrap();
                }
            }
            4 | 5 => {
                let h = s
                    .blob_put_public(&item(r.below(6000)), r.pick(&classes), r.chance(60))
                    .unwrap();
                if r.chance(40) {
                    s.blob_mark_superseded(&h).unwrap();
                }
                if r.chance(15) {
                    s.blob_set_pinned(&h, true).unwrap();
                }
            }
            6 => {
                let body = item(r.below(800));
                let m = NewMessage {
                    class: r.pick(&classes),
                    ..own(PEER, 0, &body)
                };
                let (id, _) = s.queue_own_message(&m).unwrap();
                if r.chance(80) {
                    s.set_message_state(id, MsgState::Delivered, None).unwrap();
                }
                if r.chance(10) {
                    s.set_message_pinned(id, true).unwrap();
                }
            }
            7 => {
                seq += 1;
                let body = item(r.below(800));
                let m = NewMessage {
                    sender: PEER2,
                    class: r.pick(&classes),
                    ..own(PEER, 0, &body)
                };
                let state = if r.chance(70) {
                    MsgState::Delivered
                } else {
                    MsgState::Sent
                };
                s.with_tx(Durability::Normal, |tx| {
                    rows::insert_message(tx, &m, Seq { epoch: 1, n: seq }, Dir::Transit, state)
                })
                .unwrap();
            }
            8 | 9 => {
                let conv = if r.chance(80) { a } else { p };
                let body = "x".repeat(1 + r.below(900) as usize);
                let id = ingest1(&mut s, fin(conv, "!a1", None, &body, false));
                if conv == a && r.chance(60) {
                    let text = "c".repeat(1 + r.below(300) as usize);
                    let c = s
                        .foreign_queue_send(&copy_send(edge, id, b, &text, 1, 1))
                        .unwrap();
                    if r.chance(40) {
                        s.foreign_set_state(c, ForeignState::Sent, None, None)
                            .unwrap();
                    }
                }
            }
            _ => {
                seq += 1;
                let body = item(r.below(800));
                s.receive_message(&inbound(PEER2, &body), Seq { epoch: 4, n: seq })
                    .unwrap();
            }
        }
    }
    if r.chance(50) {
        s.set_export_offered(true).unwrap();
    }
    (t, s)
}

/// What a pass may change, as text: the custody rows and their commit
/// marks, the blobs and their presence, the message, foreign and copy ids,
/// the edges' shed counts and marks, the unread counts and the persisted
/// capacity outcome.
fn prune_state(s: &Store) -> Vec<String> {
    [
        "SELECT group_concat(custody_seq || ':' || committed, ',')
         FROM (SELECT * FROM custody ORDER BY custody_seq)",
        "SELECT group_concat(hex(hash) || ':' || present, ',') FROM (SELECT * FROM blobs ORDER BY hash)",
        "SELECT group_concat(id, ',') FROM (SELECT id FROM messages ORDER BY id)",
        "SELECT group_concat(id || ':' || state, ',') FROM (SELECT * FROM foreign_messages ORDER BY id)",
        "SELECT group_concat(id || ':' || shed || ':' || relayed_seq, ',')
         FROM (SELECT * FROM relay_edges ORDER BY id)",
        "SELECT group_concat(id || ':' || unread, ',') FROM (SELECT * FROM foreign_conversations ORDER BY id)",
        "SELECT group_concat(key || '=' || quote(value), ',')
         FROM (SELECT * FROM settings WHERE key LIKE 'capacity.%' ORDER BY key)",
    ]
    .iter()
    .map(|sql| {
        s.conn
            .query_row(sql, [], |r| r.get::<_, Option<String>>(0))
            .unwrap()
            .unwrap_or_default()
    })
    .collect()
}

/// §10.4, M1c: the pass with its at-rest path and its lazy, class-filtered,
/// bounded step lists removes the same items in the same order, reports
/// the same flags and persists the same outcome as the pass that read
/// every list up front ([`Store::prune_eager`], the pre-M1c code) — over
/// random stores and random quotas, watermarks, disk figures and incoming
/// items, four passes in a row on each pair of identical stores.
#[test]
fn the_lazy_pass_equals_the_eager_pass_on_random_stores() {
    let quota_classes = [
        Class::Custody,
        Class::Transit,
        Class::Kb,
        Class::Update,
        Class::Object,
        Class::Foreign,
    ];
    let refusable: Vec<Class> = Class::ALL
        .into_iter()
        .filter(|c| c.is_refusable())
        .collect();
    let (mut at_rest, mut pruning, mut taken) = (0, 0, 0);
    for seed in 0..40u64 {
        let (_ta, mut a) = random_store(seed);
        let (_tb, mut b) = random_store(seed);
        assert_eq!(
            prune_state(&a),
            prune_state(&b),
            "seed {seed}: the same store"
        );
        let mut r = Rng::new(seed ^ 0xA5A5);
        for pass in 0..4 {
            for c in quota_classes {
                let q = if r.chance(35) {
                    Some(a.class_bytes(c).unwrap() * r.below(130) / 100)
                } else {
                    None
                };
                a.set_quota(c, q).unwrap();
                b.set_quota(c, q).unwrap();
            }
            let live = a.live_bytes().unwrap();
            let w = if r.chance(30) {
                Watermarks::with_high(live * 4)
            } else {
                let high = live * (40 + r.below(80)) / 100;
                Watermarks {
                    high,
                    low: high - high * r.below(40) / 100,
                }
            };
            let disk = match r.below(10) {
                0 => DiskSpace {
                    free: r.below(9_000),
                    total: 1_000_000,
                },
                1 => DiskSpace {
                    free: 1 << 30,
                    total: 1 << 31,
                },
                _ => DiskSpace::UNKNOWN,
            };
            let extra = match r.below(5) {
                0 | 1 => None,
                2 | 3 => Some((Some(r.pick(&refusable)), 1 + r.below(20_000))),
                _ => Some((None, 1 + r.below(20_000))),
            };
            let lazy = a.prune_inner(disk, w, extra).unwrap();
            let eager = b.prune_eager(disk, w, extra).unwrap();
            assert_eq!(lazy, eager, "seed {seed}, pass {pass}");
            assert_eq!(prune_state(&a), prune_state(&b), "seed {seed}, pass {pass}");
            assert_eq!(
                (a.store_full(), a.full_classes()),
                (b.store_full(), b.full_classes())
            );
            assert_totals_exact(&a);
            if lazy.0.pruned.is_empty() && !lazy.0.triggered && lazy.0.full_classes.is_empty() {
                at_rest += 1;
            } else {
                pruning += 1;
            }
            taken += lazy.0.pruned.len();
        }
    }
    // The draw covers both paths and real pruning.
    assert!(
        at_rest > 10 && pruning > 40 && taken > 200,
        "{at_rest} {pruning} {taken}"
    );
}

#[test]
fn class_transit_is_pinned_quota_only_and_never_from_a_kind() {
    assert_eq!(Class::ALL.len(), 11);
    assert_eq!(Class::Transit as u8, 10);
    assert_eq!(Class::ALL[10], Class::Transit);
    assert_eq!(Class::ALL[8], Class::Custody);
    assert!((0..=255u8).all(|k| Class::from_kind(k) != Class::Transit));
    assert!(Class::Transit.is_refusable() && Class::Transit.has_quota());
    assert_eq!(Class::Transit.name(), "transit");
    assert_eq!(Class::from_i64(10).unwrap(), Class::Transit);
    assert!(Class::from_i64(11).is_err());
    // The class of a row comes from its `fwd`.
    assert_eq!(Fwd::Member.class(), Class::Custody);
    for (f, code) in [
        (Fwd::FromMember(PEER), 1),
        (Fwd::FromLeaf(LEAF), 2),
        (Fwd::FromCarrier(CARRIER), 3),
    ] {
        assert_eq!(f.class(), Class::Transit);
        assert_eq!(f.code(), code);
        assert!(f.is_forward() && f.origin().is_some());
    }
    assert_eq!((Fwd::Member.code(), Fwd::Member.origin()), (0, None));
    // The capacity mask persists bit 10 like any other class.
    let (t, mut s) = ready();
    s.set_quota(Class::Transit, Some(0)).unwrap();
    accepted(s.custody_put(&PEER, b"member").unwrap());
    s.conn
        .execute(
            "INSERT INTO custody (dest, pkt_ref, bytes, local_seq, fwd, fwd_from)
             VALUES (?1, ?2, x'01', 1, 1, ?3)",
            rusqlite::params![&STRANGER[..], &[9u8; 16][..], &PEER[..]],
        )
        .unwrap();
    let r = s.prune(DiskSpace::UNKNOWN).unwrap();
    assert_eq!(r.full_classes, vec![Class::Transit]);
    s.close(1).unwrap();
    let (s, _) = open(t.path(), 1);
    assert_eq!(s.full_classes(), &[Class::Transit]);
}

#[test]
fn custody_splits_by_fwd_between_custody_and_transit() {
    let (_t, mut s) = ready();
    let member = accepted(s.custody_put(&PEER, &[1; 1000]).unwrap());
    let f1 = accepted(
        s.custody_put_fwd(&STRANGER, &[2; 2000], Fwd::FromMember(PEER))
            .unwrap(),
    );
    let f2 = accepted(
        s.custody_put_fwd(&STRANGER, &[3; 3000], Fwd::FromLeaf(LEAF))
            .unwrap(),
    );
    let f3 = accepted(
        s.custody_put_fwd(&PEER2, &[4; 500], Fwd::FromCarrier(CARRIER))
            .unwrap(),
    );
    assert_eq!(raw_fwd(&s, member), (0, None));
    assert_eq!(raw_fwd(&s, f1), (1, Some(PEER.to_vec())));
    assert_eq!(raw_fwd(&s, f2), (2, Some(LEAF.to_vec())));
    assert_eq!(raw_fwd(&s, f3), (3, Some(CARRIER.to_vec())));
    assert_eq!(
        s.class_bytes(Class::Custody).unwrap(),
        1000 + prune::ROW_OVERHEAD
    );
    assert_eq!(
        s.class_bytes(Class::Transit).unwrap(),
        fwd_bytes(2000) + fwd_bytes(3000) + fwd_bytes(500)
    );
    // Strangers' items never count against members' custody quota: the
    // member class has 100 B of room although Transit holds 5.6 KB.
    s.set_quota(Class::Custody, Some(1000 + prune::ROW_OVERHEAD + 100))
        .unwrap();
    s.set_quota(Class::Transit, Some(1 << 20)).unwrap();
    let m2 = accepted(s.custody_put(&PEER, &[5; 100]).unwrap());
    assert_eq!(
        s.custody_put(&PEER, &[6; 100]).unwrap(),
        CustodyPut::RejectFull
    );
    // A forward row still fits Transit.
    let f4 = accepted(
        s.custody_put_fwd(&STRANGER, &[7; 100], Fwd::FromMember(PEER2))
            .unwrap(),
    );
    // The forwarder's view: forward rows only, ascending, typed.
    let pending = s.forward_pending(0, 10).unwrap();
    assert_eq!(
        pending
            .iter()
            .map(|i| (i.custody_seq, i.fwd))
            .collect::<Vec<_>>(),
        vec![
            (f1, Fwd::FromMember(PEER)),
            (f2, Fwd::FromLeaf(LEAF)),
            (f3, Fwd::FromCarrier(CARRIER)),
            (f4, Fwd::FromMember(PEER2)),
        ]
    );
    assert_eq!(pending[1].bytes, vec![3; 3000]);
    assert_eq!(pending[1].dest, STRANGER);
    assert_eq!(pending[1].pkt_ref, crate::custody::pkt_ref(&[3; 3000]));
    assert!(!pending[1].committed);
    assert_eq!(
        s.forward_pending(f2, 1).unwrap()[0].custody_seq,
        f3,
        "after and limit page"
    );
    assert_eq!(s.forward_item(member).unwrap(), None);
    assert_eq!(s.forward_item(f3).unwrap().unwrap().fwd.code(), 3);
    assert_eq!(
        s.forward_by_ref(&crate::custody::pkt_ref(&[2; 2000]))
            .unwrap()
            .unwrap()
            .custody_seq,
        f1
    );
    assert_eq!(
        s.forward_by_ref(&crate::custody::pkt_ref(&[1; 1000]))
            .unwrap(),
        None,
        "a member row is not a forward row"
    );
    assert_eq!(s.forward_stats().unwrap(), (4, 2000 + 3000 + 500 + 100));
    // A member's fetch reads its own dest only; the totals stay whole.
    assert_eq!(
        s.custody_fetch(&PEER, 0, 10)
            .unwrap()
            .iter()
            .map(|i| i.custody_seq)
            .collect::<Vec<_>>(),
        vec![member, m2]
    );
    assert_eq!(s.custody_stats().unwrap().0, 6);
}

/// Layer 7 §5.9 (M4c): the forwarder's page query reads the rows'
/// heads, the same rows in the same order as `forward_pending`, with the
/// item's length and never its bytes.
#[test]
fn forward_heads_page_like_forward_pending_without_the_bytes() {
    let (_t, mut s) = ready();
    let member = accepted(s.custody_put(&PEER, &[1; 1000]).unwrap());
    let f1 = accepted(
        s.custody_put_fwd(&STRANGER, &[2; 2000], Fwd::FromMember(PEER))
            .unwrap(),
    );
    let f2 = accepted(
        s.custody_put_fwd(&PEER2, &[3; 3000], Fwd::FromLeaf(LEAF))
            .unwrap(),
    );
    let f3 = accepted(
        s.custody_put_fwd(&STRANGER, &[4; 500], Fwd::FromCarrier(CARRIER))
            .unwrap(),
    );
    assert!(s.forward_commit(f2).unwrap());
    let heads = s.forward_pending_lens(0, 10).unwrap();
    assert_eq!(
        heads,
        vec![
            ForwardHead {
                custody_seq: f1,
                dest: STRANGER,
                pkt_ref: crate::custody::pkt_ref(&[2; 2000]),
                len: 2000,
                fwd: Fwd::FromMember(PEER),
            },
            ForwardHead {
                custody_seq: f3,
                dest: STRANGER,
                pkt_ref: crate::custody::pkt_ref(&[4; 500]),
                len: 500,
                fwd: Fwd::FromCarrier(CARRIER),
            },
        ],
        "protected forward rows only: no member row, no committed one"
    );
    let full = s.forward_pending(0, 10).unwrap();
    assert_eq!(
        full.iter()
            .map(|i| (
                i.custody_seq,
                i.dest,
                i.pkt_ref,
                i.bytes.len() as u64,
                i.fwd
            ))
            .collect::<Vec<_>>(),
        heads
            .iter()
            .map(|h| (h.custody_seq, h.dest, h.pkt_ref, h.len, h.fwd))
            .collect::<Vec<_>>(),
        "the same rows as forward_pending"
    );
    // `after` is exclusive and `limit` pages.
    assert_eq!(s.forward_pending_lens(f1, 1).unwrap()[0].custody_seq, f3);
    assert_eq!(s.forward_pending_lens(0, 1).unwrap().len(), 1);
    assert!(s.forward_pending_lens(f3, 10).unwrap().is_empty());
    assert!(s.forward_pending_lens(member, 0).unwrap().is_empty());
}

#[test]
fn the_transit_quota_defaults_to_a_quarter_and_the_share_to_an_eighth() {
    let (_t, mut s) = ready();
    assert_eq!(s.quota(Class::Custody).unwrap(), None);
    assert_eq!(
        s.quota(Class::Transit).unwrap(),
        Some(prune::DEFAULT_HIGH_WATERMARK / 4)
    );
    assert_eq!(s.quota(Class::Transit).unwrap(), Some(128 << 20));
    assert_eq!(s.transit_share().unwrap(), 16 << 20);
    // The base is the store's (configured) high watermark ...
    s.set_watermarks(Watermarks::with_high(64 << 20)).unwrap();
    assert_eq!(s.quota(Class::Transit).unwrap(), Some(16 << 20));
    // ... or a quarter of the custody quota when one is set ...
    s.set_quota(Class::Custody, Some(40_000)).unwrap();
    assert_eq!(s.quota(Class::Transit).unwrap(), Some(10_000));
    assert_eq!(s.transit_share().unwrap(), 1_250);
    // ... and an explicit `quota.transit` wins; clearing it restores the
    // derived default, never "no cap".
    s.set_quota(Class::Transit, Some(800)).unwrap();
    assert_eq!(s.quota(Class::Transit).unwrap(), Some(800));
    assert_eq!(s.transit_share().unwrap(), 100);
    s.set_quota(Class::Transit, None).unwrap();
    assert_eq!(s.quota(Class::Transit).unwrap(), Some(10_000));
    assert_eq!(s.quota(Class::Custody).unwrap(), Some(40_000));
    assert_eq!(s.quota(Class::Kb).unwrap(), None);
}

#[test]
fn an_origin_over_its_share_is_refused_while_others_are_admitted() {
    let (_t, mut s) = ready();
    // Share = quota / 8 = 2.5 rows of 1,000 B.
    s.set_quota(Class::Transit, Some(8 * (2 * fwd_bytes(1000) + 500)))
        .unwrap();
    let share = s.transit_share().unwrap();
    assert_eq!(share, 2 * fwd_bytes(1000) + 500);
    let m = Fwd::FromMember(PEER);
    let a = accepted(s.custody_put_fwd(&STRANGER, &[1; 1000], m).unwrap());
    let b = accepted(s.custody_put_fwd(&STRANGER, &[2; 1000], m).unwrap());
    assert_eq!(s.forward_origin_bytes(&PEER).unwrap(), 2 * fwd_bytes(1000));
    assert_eq!(
        s.custody_put_fwd(&STRANGER, &[3; 1000], m).unwrap(),
        CustodyPut::RejectFull,
        "over its share"
    );
    // Dedup comes first: a repeat of a held item is still a duplicate.
    assert_eq!(
        s.custody_put_fwd(&STRANGER, &[1; 1000], m).unwrap(),
        CustodyPut::Duplicate(a)
    );
    // Other origins are admitted: another member, a leaf hub, a carrier.
    accepted(
        s.custody_put_fwd(&STRANGER, &[3; 1000], Fwd::FromMember(PEER2))
            .unwrap(),
    );
    accepted(
        s.custody_put_fwd(&STRANGER, &[4; 1000], Fwd::FromLeaf(LEAF))
            .unwrap(),
    );
    accepted(
        s.custody_put_fwd(&STRANGER, &[5; 1000], Fwd::FromCarrier(CARRIER))
            .unwrap(),
    );
    // Member rows never count in a share, even for the same NodeId.
    accepted(s.custody_put(&PEER, &[6; 1000]).unwrap());
    assert_eq!(s.forward_origin_bytes(&PEER).unwrap(), 2 * fwd_bytes(1000));
    // A committed row (handed over) or a refused one pins nothing: the
    // origin is admitted again.
    assert!(s.forward_commit(a).unwrap());
    assert_eq!(s.forward_origin_bytes(&PEER).unwrap(), fwd_bytes(1000));
    let c = accepted(s.custody_put_fwd(&STRANGER, &[7; 1000], m).unwrap());
    assert_eq!(
        s.custody_put_fwd(&STRANGER, &[8; 1000], m).unwrap(),
        CustodyPut::RejectFull
    );
    assert!(s.forward_refuse(b).unwrap());
    accepted(s.custody_put_fwd(&STRANGER, &[8; 1000], m).unwrap());
    assert!(s.forward_item(c).unwrap().is_some());
}

#[test]
fn refused_forward_rows_are_step_one_evictable_and_protected_ones_never() {
    let (_t, mut s) = ready();
    let refused = accepted(
        s.custody_put_fwd(&STRANGER, &[1; 4000], Fwd::FromMember(PEER))
            .unwrap(),
    );
    let handed = accepted(
        s.custody_put_fwd(&STRANGER, &[2; 100], Fwd::FromLeaf(LEAF))
            .unwrap(),
    );
    let protected = accepted(
        s.custody_put_fwd(&PEER2, &[3; 3000], Fwd::FromCarrier(CARRIER))
            .unwrap(),
    );
    let member = accepted(s.custody_put(&PEER, &[4; 2000]).unwrap());
    // The refused mark and the commit are idempotent, forward rows only.
    assert!(s.forward_refuse(refused).unwrap());
    assert!(!s.forward_refuse(refused).unwrap());
    assert!(s.forward_commit(handed).unwrap());
    assert!(!s.forward_commit(member).unwrap(), "never a member row");
    assert!(!s.forward_commit(999).unwrap());
    assert!(s.forward_item(refused).unwrap().unwrap().committed);
    assert_eq!(
        s.forward_pending(0, 10)
            .unwrap()
            .iter()
            .map(|i| i.custody_seq)
            .collect::<Vec<_>>(),
        vec![protected]
    );
    assert_eq!(s.forward_stats().unwrap(), (1, 3000));
    let r = s
        .prune_with(DiskSpace::UNKNOWN, Watermarks { high: 0, low: 0 })
        .unwrap();
    assert_eq!(
        r.pruned
            .iter()
            .map(|p| (p.step, p.item))
            .collect::<Vec<_>>(),
        vec![
            (PruneStep::Evictable, PrunedRef::Custody(refused)),
            (PruneStep::Evictable, PrunedRef::Custody(handed)),
        ]
    );
    assert_eq!(r.pruned[0].bytes, fwd_bytes(4000));
    // Step 4: uncommitted custody, forward rows included, stays.
    assert!(r.store_full);
    assert!(s.forward_item(refused).unwrap().is_none());
    assert!(!s.forward_item(protected).unwrap().unwrap().committed);
    assert_eq!(
        s.custody_fetch(&PEER, 0, 10).unwrap()[0].custody_seq,
        member
    );
    // `pkt_ref` stays until the row is evicted, then a re-put is new.
    s.set_watermarks(Watermarks::with_high(u64::MAX / 2))
        .unwrap();
    s.prune(DiskSpace::UNKNOWN).unwrap();
    assert!(
        s.custody_put_fwd(&STRANGER, &[1; 4000], Fwd::FromMember(PEER))
            .unwrap()
            != CustodyPut::Duplicate(refused)
    );
}

#[test]
fn over_transit_with_only_protected_rows_forward_puts_are_refused_member_puts_admitted() {
    let (_t, mut s) = ready();
    // Eight origins at their full share fill Transit exactly.
    let row = fwd_bytes(100);
    s.set_quota(Class::Transit, Some(8 * row)).unwrap();
    assert_eq!(s.transit_share().unwrap(), row);
    let origins: Vec<[u8; 32]> = (0..9u8).map(|i| [0x40 + i; 32]).collect();
    let mut seqs = Vec::new();
    for (i, o) in origins[..8].iter().enumerate() {
        seqs.push(accepted(
            s.custody_put_fwd(&STRANGER, &[i as u8; 100], Fwd::FromMember(*o))
                .unwrap(),
        ));
    }
    assert_eq!(s.class_bytes(Class::Transit).unwrap(), 8 * row);
    // A committed member row the Transit quota must never touch.
    let member_done = accepted(s.custody_put(&PEER, &[0xAA; 500]).unwrap());
    s.custody_commit(&PEER, member_done).unwrap();
    // The ninth origin is inside its share but Transit is full of
    // protected rows: refused. A member put is admitted.
    assert_eq!(
        s.custody_put_fwd(&STRANGER, &[0xF0; 100], Fwd::FromLeaf(origins[8]))
            .unwrap(),
        CustodyPut::RejectFull
    );
    accepted(s.custody_put(&PEER, &[0xAB; 100]).unwrap());
    // One forward row handed over: the quota evicts it (Transit only) and
    // admits the new origin.
    assert!(s.forward_commit(seqs[0]).unwrap());
    accepted(
        s.custody_put_fwd(&STRANGER, &[0xF0; 100], Fwd::FromLeaf(origins[8]))
            .unwrap(),
    );
    assert!(s.forward_item(seqs[0]).unwrap().is_none());
    assert_eq!(
        s.custody_fetch(&PEER, 0, 10).unwrap()[0].custody_seq,
        member_done,
        "the committed member row is not Transit's to evict"
    );
}

#[test]
fn protected_transit_counts_toward_the_high_watermark() {
    let (_t, mut s) = ready();
    let before = s.live_bytes().unwrap();
    let mut fwd = Vec::new();
    for i in 0..4u8 {
        fwd.push(accepted(
            s.custody_put_fwd(&STRANGER, &[i; 60_000], Fwd::FromMember(PEER))
                .unwrap(),
        ));
    }
    let live = s.live_bytes().unwrap();
    assert!(live - before >= 4 * 60_000, "{before} → {live}");
    // The store is at its high watermark with only protected rows: a member
    // item that needs room is refused (REJECT_FULL), one that fits is
    // admitted up to the watermark.
    s.set_watermarks(Watermarks {
        high: live + 6_000,
        low: live + 6_000,
    })
    .unwrap();
    assert_eq!(
        s.custody_put(&PEER, &[0xA0; 20_000]).unwrap(),
        CustodyPut::RejectFull
    );
    assert!(!s.store_full(), "the store itself is not over");
    accepted(s.custody_put(&PEER, &[0xA1; 500]).unwrap());
    assert_eq!(
        s.custody_put_fwd(&STRANGER, &[0xA2; 20_000], Fwd::FromMember(PEER2))
            .unwrap(),
        CustodyPut::RejectFull
    );
    // Refused rows release the watermark: step 1 evicts them for the item.
    for q in &fwd {
        assert!(s.forward_refuse(*q).unwrap());
    }
    accepted(s.custody_put(&PEER, &[0xA0; 20_000]).unwrap());
    assert!(fwd.iter().any(|q| s.forward_item(*q).unwrap().is_none()));
}

#[test]
fn the_prune_order_places_forward_rows_by_their_commit_state() {
    let (_t, mut s) = ready();
    let fx = fixture(&mut s);
    // After the fixture: a handed-over forward row (newest, mid-sized) and
    // a protected one (larger than every evictable item but the first).
    let handed = accepted(
        s.custody_put_fwd(&STRANGER, &[0x11; 20_000], Fwd::FromLeaf(LEAF))
            .unwrap(),
    );
    s.forward_commit(handed).unwrap();
    let protected = accepted(
        s.custody_put_fwd(&STRANGER, &[0x22; 30_000], Fwd::FromMember(PEER2))
            .unwrap(),
    );
    // A zero watermark would make the derived Transit quota zero too (a
    // quarter of it): keep the quota out of this pass, and see it below.
    s.set_quota(Class::Transit, Some(1 << 30)).unwrap();
    s.set_watermarks(Watermarks { high: 0, low: 0 }).unwrap();
    let r = s.prune(DiskSpace::UNKNOWN).unwrap();
    let order: Vec<(PruneStep, PrunedRef)> = r.pruned.iter().map(|p| (p.step, p.item)).collect();
    // Step 1 by size × age (the forward row among the member rows and the
    // re-fetchable chunk), then step 2; the protected forward row is in
    // step 4's "never" tier.
    assert_eq!(
        order,
        vec![
            (PruneStep::Evictable, PrunedRef::Custody(fx.custody_big)),
            (PruneStep::Evictable, PrunedRef::Custody(handed)),
            (PruneStep::Evictable, PrunedRef::Blob(fx.blob_refetch)),
            (PruneStep::Evictable, PrunedRef::Custody(fx.custody_small)),
            (PruneStep::Superseded, PrunedRef::Blob(fx.blob_superseded)),
        ]
    );
    assert!(r.store_full && r.export_offer_needed);
    assert!(s.forward_item(protected).unwrap().is_some());
    assert_eq!(
        s.custody_fetch(&PEER2, 0, 10).unwrap()[0].custody_seq,
        fx.custody_open
    );
    // With the derived quota (a quarter of a zero watermark, so 0) a
    // committed forward row goes first, as Transit's quota step.
    s.set_watermarks(Watermarks::with_high(u64::MAX / 2))
        .unwrap();
    let late = accepted(
        s.custody_put_fwd(&STRANGER, &[0x33; 10], Fwd::FromLeaf(LEAF))
            .unwrap(),
    );
    s.forward_commit(late).unwrap();
    s.set_quota(Class::Transit, None).unwrap();
    s.set_watermarks(Watermarks { high: 0, low: 0 }).unwrap();
    assert_eq!(s.quota(Class::Transit).unwrap(), Some(0));
    let r = s.prune(DiskSpace::UNKNOWN).unwrap();
    assert_eq!(
        (r.pruned[0].step, r.pruned[0].item),
        (PruneStep::Quota(Class::Transit), PrunedRef::Custody(late))
    );
    assert!(
        r.full_classes.contains(&Class::Transit),
        "protected rows remain"
    );
}

#[test]
fn no_pairing_writes_exactly_the_member_row() {
    let (_t, mut s) = ready();
    let a = accepted(s.custody_put(&PEER, b"plain").unwrap());
    let b = accepted(s.custody_put_fwd(&PEER, b"also", Fwd::Member).unwrap());
    for q in [a, b] {
        assert_eq!(raw_fwd(&s, q), (0, None));
    }
    assert_eq!(s.class_bytes(Class::Transit).unwrap(), 0);
    assert_eq!(s.forward_stats().unwrap(), (0, 0));
    assert!(s.forward_pending(0, 10).unwrap().is_empty());
    assert_eq!(
        s.class_bytes(Class::Custody).unwrap(),
        5 + 4 + 2 * prune::ROW_OVERHEAD
    );
    // A pass with no forward rows prunes and reports what v3 did.
    s.custody_commit(&PEER, a).unwrap();
    let r = s
        .prune_with(DiskSpace::UNKNOWN, Watermarks { high: 0, low: 0 })
        .unwrap();
    assert_eq!(
        r.pruned
            .iter()
            .map(|p| (p.step, p.item, p.bytes))
            .collect::<Vec<_>>(),
        vec![(
            PruneStep::Evictable,
            PrunedRef::Custody(a),
            5 + prune::ROW_OVERHEAD
        )]
    );
    assert!(r.full_classes.is_empty());
}

/// Layer 7 §5.5 (M4e): whether a protected forward row of a dest waits
/// above the paging floor outside the forwarder's window. Member rows of
/// the same dest and committed rows never count; `above` is exclusive; the
/// answer is "more than `resident`"; the read goes through the v5 partial
/// index `custody_open_by_dest` and never scans the table.
#[test]
fn forward_waits_for_dest_counts_protected_rows_of_that_dest_only() {
    let (_t, mut s) = ready();
    let put = |s: &mut Store, dest: &[u8; 32], tag: u8, fwd: Fwd| {
        accepted(s.custody_put_fwd(dest, &[tag; 64], fwd).unwrap())
    };
    let m1 = put(&mut s, &STRANGER, 1, Fwd::Member);
    let f1 = put(&mut s, &STRANGER, 2, Fwd::FromMember(PEER));
    let other = put(&mut s, &PEER2, 3, Fwd::FromLeaf(LEAF));
    let f2 = put(&mut s, &STRANGER, 4, Fwd::FromCarrier(CARRIER));
    let c1 = put(&mut s, &STRANGER, 5, Fwd::FromMember(PEER));
    let m2 = put(&mut s, &STRANGER, 6, Fwd::Member);
    assert!(s.forward_commit(c1).unwrap());
    assert!(m1 < f1 && f1 < other && other < f2 && f2 < c1 && c1 < m2);
    let waits = |s: &Store, dest: &[u8; 32], above: u64, resident: usize| {
        s.forward_waits_for_dest(dest, above, resident).unwrap()
    };
    // STRANGER: two protected forward rows (f1, f2); its member rows and
    // the committed c1 never count.
    assert!(waits(&s, &STRANGER, 0, 0));
    assert!(waits(&s, &STRANGER, 0, 1), "two rows, one resident");
    assert!(!waits(&s, &STRANGER, 0, 2), "both resident");
    assert!(!waits(&s, &STRANGER, 0, 5), "n resident, fewer stored");
    assert!(waits(&s, &STRANGER, f1 - 1, 1), "f1 is above f1 - 1");
    assert!(!waits(&s, &STRANGER, f1, 1), "above is exclusive: f2 only");
    assert!(waits(&s, &STRANGER, f1, 0));
    assert!(!waits(&s, &STRANGER, f2, 0), "only c1 and m2 above f2");
    assert!(!waits(&s, &STRANGER, c1, 0));
    // The other dest: its own row only.
    assert!(waits(&s, &PEER2, 0, 0));
    assert!(!waits(&s, &PEER2, 0, 1));
    assert!(!waits(&s, &PEER2, other, 0));
    assert!(!waits(&s, &PEER, 0, 0), "a dest with no row");
    // Committing f2 leaves f1 alone above the floor.
    assert!(s.forward_commit(f2).unwrap());
    assert!(waits(&s, &STRANGER, 0, 0));
    assert!(!waits(&s, &STRANGER, 0, 1));
    // The same statement's plan: a search of `custody_open_by_dest` (v5,
    // uncommitted rows only) on the dest and the floor, no table scan.
    let plan: Vec<String> = {
        let mut st = s
            .conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT custody_seq FROM custody
                 WHERE dest = ?1 AND custody_seq > ?2 AND fwd != 0 AND committed = 0
                 LIMIT ?3",
            )
            .unwrap();
        let rows = st
            .query_map(rusqlite::params![&STRANGER[..], 0i64, 1i64], |r| {
                r.get::<_, String>(3)
            })
            .unwrap();
        rows.map(|r| r.unwrap()).collect()
    };
    assert!(
        plan.iter()
            .any(|d| d.starts_with("SEARCH") && d.contains("USING INDEX custody_open_by_dest")),
        "{plan:?}"
    );
    assert!(plan.iter().all(|d| !d.contains("SCAN")), "{plan:?}");
}
