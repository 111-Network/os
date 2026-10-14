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
fn a_v1_store_migrates_to_v2_in_place() {
    let (t, s) = ready();
    s.conn
        .execute_batch("DROP TABLE pending_node_ids; PRAGMA user_version = 1;")
        .unwrap();
    s.close(1).unwrap();
    let (mut s, _) = open(t.path(), 1);
    assert_eq!(
        pragma_int(&s.conn, "user_version").unwrap(),
        schema::SCHEMA_VERSION
    );
    assert_eq!(schema::SCHEMA_VERSION, 2);
    assert!(s.pending_node_id_put(&PEER).unwrap());
    assert_eq!(s.pending_node_ids().unwrap(), vec![PEER]);
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
