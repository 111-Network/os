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
    let (t, s) = ready();
    s.conn
        .execute_batch(
            "DROP TABLE foreign_seen; DROP TABLE relay_edges; DROP TABLE foreign_messages;
             DROP TABLE send_groups; DROP TABLE foreign_conversations;
             DROP TABLE pending_node_ids; PRAGMA user_version = 1;",
        )
        .unwrap();
    s.close(1).unwrap();
    let (mut s, _) = open(t.path(), 1);
    assert_eq!(
        pragma_int(&s.conn, "user_version").unwrap(),
        schema::SCHEMA_VERSION
    );
    assert_eq!(schema::SCHEMA_VERSION, 3);
    assert!(s.pending_node_id_put(&PEER).unwrap());
    assert_eq!(s.pending_node_ids().unwrap(), vec![PEER]);
    s.foreign_conversation_upsert("lxmf", "me", ForeignKind::Dm, "dm:ab", None)
        .unwrap();
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
    assert_eq!(Class::ALL.len(), 10);
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

/// A V2 store written by the V2 migrations alone (the shape a V2 build
/// leaves on disk).
fn write_v2_fixture(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    let conn = rusqlite::Connection::open(dir.join(DB_FILE)).unwrap();
    apply_key_and_pragmas(&conn, &key()).unwrap();
    for (i, sql) in schema::MIGRATIONS[..2].iter().enumerate() {
        conn.execute_batch(sql).unwrap();
        conn.pragma_update(None, "user_version", i as i64 + 1)
            .unwrap();
    }
    conn.execute(
        "INSERT INTO pending_node_ids (node_id, local_seq) VALUES (?1, 1)",
        [&PEER2[..]],
    )
    .unwrap();
    assert_eq!(pragma_int(&conn, "user_version").unwrap(), 2);
    conn.close().unwrap();
}

#[test]
fn a_v2_store_opens_at_v3_and_keeps_its_rows() {
    let t = tempfile::tempdir().unwrap();
    write_v2_fixture(t.path());
    let (mut s, _) = open(t.path(), 1);
    assert_eq!(pragma_int(&s.conn, "user_version").unwrap(), 3);
    assert_eq!(s.pending_node_ids().unwrap(), vec![PEER2]);
    s.record_epoch(1).unwrap();
    let c = fconv(&mut s, "meshtastic", ForeignKind::Channel, "ch:LongFast:00");
    ingest1(&mut s, fin(c, "!a1b2c3d4", Some("1"), "hello", false));
    s.close(1).unwrap();
    let (s, _) = open(t.path(), 1);
    assert_eq!(s.foreign_conversation(c).unwrap().unwrap().messages, 1);
}

#[test]
fn a_store_newer_than_v3_is_refused() {
    // The "V3 refused by a V2 build" rule is the same check one version up:
    // this build refuses user_version 4.
    let (t, s) = ready();
    s.conn.pragma_update(None, "user_version", 4).unwrap();
    s.close(1).unwrap();
    assert!(matches!(
        Store::open(t.path(), &key(), &params(1)),
        Err(StoreError::SchemaTooNew {
            found: 4,
            supported: 3
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
