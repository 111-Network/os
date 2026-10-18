//! §7.2 (3) over `tcp-lan`: an INTRO from a never-contacted sender becomes a
//! message request — held, never ACKed while pending (ignore), ACKed for
//! exactly what is shown on accept — and capacity refusals (the pending
//! quota and `store_full`) drop the INTRO without an ACK and count it.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use mesh111::contacts::card_text;
use mesh111::fabric::{ContactPatch, Fabric, FabricSettings, SettingsPatch};
use mesh111::link::tcp_lan::TcpLan;
use mesh111::link::Dialer;
use mesh111::store::{MsgState, Trust, Watermarks};

struct Peer {
    ident: Arc<Ident>,
    f: Fabric,
    t: TcpLan,
}

async fn peer(d: &Dirs, name: &str) -> Peer {
    let ident = Ident::new(name, 0);
    let f = start(&ident, &d.node(name), FabricSettings::default());
    let t = tcp(&f, &ident, "tcp");
    add_tcp(&f, &t, true).await;
    Peer { ident, f, t }
}

/// `from` holds `to`'s card and dials it; `to` has never heard of `from`.
async fn introduce(from: &Peer, to: &Peer, to_addr: SocketAddr) {
    from.f
        .add_contact_card(&card_text(&to.ident.announce_bytes()))
        .await
        .unwrap();
    from.t.maintain(to_addr, Some(to.ident.id()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn intro_request_ignore_then_accept() {
    let d = Dirs::new();
    let alice = peer(&d, "alice").await;
    let bob = peer(&d, "bob").await;
    let baddr = listen(&bob.t, 0).await;
    introduce(&alice, &bob, baddr).await;

    let id = alice
        .f
        .send(bob.ident.id(), "hi, it's alice", None)
        .await
        .unwrap();
    // Bob files a message request; the message is held, nothing is ACKed.
    until(10, || async {
        let c = bob.f.contact(alice.ident.id()).await.ok()?;
        c.request.then_some(())
    })
    .await;
    let c = bob.f.contact(alice.ident.id()).await.unwrap();
    assert_eq!(c.trust, Trust::Tofu);
    let t = bob.f.thread(alice.ident.id(), 10).await.unwrap();
    assert!(t.request);
    assert_eq!(t.messages.len(), 1);
    let threads = bob.f.threads().await.unwrap();
    assert!(threads
        .iter()
        .any(|t| t.peer == alice.ident.id() && t.request));
    until_state(&alice.f, id, MsgState::Sent, 10).await;

    // Ignoring it: a retransmission is a duplicate — still no ACK, no copy.
    alice.f.retry(id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(alice.f.message(id).await.unwrap().state, MsgState::Sent);
    assert_eq!(inbox(&bob.f, alice.ident.id()).await.len(), 1);

    // Accepting: the contact is created and the shown message is ACKed.
    let c = bob
        .f
        .patch_contact(
            alice.ident.id(),
            ContactPatch {
                accept: true,
                alias: Some(Some("Alice".into())),
                ..ContactPatch::default()
            },
        )
        .await
        .unwrap();
    assert!(!c.request && c.has_key);
    assert_eq!(c.alias.as_deref(), Some("Alice"));
    until_state(&alice.f, id, MsgState::Delivered, 10).await;

    // And Bob can answer on the same connection.
    let r = bob
        .f
        .send(alice.ident.id(), "hello alice", None)
        .await
        .unwrap();
    until_state(&bob.f, r, MsgState::Delivered, 10).await;
    let got = inbox(&alice.f, bob.ident.id()).await;
    assert_eq!(got[0].text.as_deref(), Some("hello alice"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn request_quota_and_store_full_refuse_without_ack() {
    let d = Dirs::new();
    let bob = peer(&d, "bob").await;
    let baddr = listen(&bob.t, 0).await;
    let carol = peer(&d, "carol").await;
    let dan = peer(&d, "dan").await;
    introduce(&carol, &bob, baddr).await;
    introduce(&dan, &bob, baddr).await;

    // The pending-request quota is 0: Carol's INTRO is dropped and counted.
    bob.f
        .patch_settings(SettingsPatch {
            request_quota: Some(Some(0)),
            ..SettingsPatch::default()
        })
        .await
        .unwrap();
    let id = carol
        .f
        .send(bob.ident.id(), "knock knock", None)
        .await
        .unwrap();
    until(10, || async {
        let s = bob.f.status().await.ok()?;
        (s.counters.refused_requests == 1).then_some(())
    })
    .await;
    assert!(bob.f.contact(carol.ident.id()).await.is_err());
    assert!(inbox(&bob.f, carol.ident.id()).await.is_empty());
    assert_eq!(carol.f.message(id).await.unwrap().state, MsgState::Sent);
    assert!(
        bob.ident.forgotten().contains(&carol.ident.enc()),
        "the provisional pair key is zeroized"
    );

    // The quota is restored: Carol's next try is filed.
    bob.f
        .patch_settings(SettingsPatch {
            request_quota: Some(None),
            ..SettingsPatch::default()
        })
        .await
        .unwrap();
    carol.f.retry(id).await.unwrap();
    until(10, || async {
        let c = bob.f.contact(carol.ident.id()).await.ok()?;
        c.request.then_some(())
    })
    .await;

    // store_full: pending requests are never pruned, so a new one is refused.
    bob.f
        .with_store(|s| s.set_watermarks(Watermarks { high: 1, low: 1 }))
        .await
        .unwrap()
        .unwrap();
    let id2 = dan.f.send(bob.ident.id(), "me too", None).await.unwrap();
    until(10, || async {
        let s = bob.f.status().await.ok()?;
        (s.counters.refused_requests == 2).then_some(())
    })
    .await;
    assert!(bob.f.contact(dan.ident.id()).await.is_err());
    assert_eq!(dan.f.message(id2).await.unwrap().state, MsgState::Sent);
    assert!(bob.f.status().await.unwrap().store.store_full);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ack_piggybacks_on_queued_data() {
    let d = Dirs::new();
    let alice = peer(&d, "alice").await;
    let bob = peer(&d, "bob").await;
    let aaddr = listen(&alice.t, 0).await;
    exchange_cards(&alice.f, &bob.f).await;
    // Both queue before any connection exists.
    let pong = alice.f.send(bob.ident.id(), "pong", None).await.unwrap();
    let ping = bob.f.send(alice.ident.id(), "ping", None).await.unwrap();
    // Bob dials Alice. Her side is inbound: she sends nothing until Bob's
    // fresh packet attributes it — the very packet she now owes an ACK for,
    // so that ACK rides her queued message (§6.1 piggyback).
    bob.t.maintain(aaddr, Some(alice.ident.id()));
    until_state(&bob.f, ping, MsgState::Delivered, 10).await;
    until_state(&alice.f, pong, MsgState::Delivered, 10).await;
    assert!(alice.f.status().await.unwrap().counters.piggybacked >= 1);
}
