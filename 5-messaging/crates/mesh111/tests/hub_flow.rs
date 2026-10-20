//! §20 G1 shapes, in process (two members + one hub over real `tcp-lan`
//! sockets on loopback): the four delivery states, a non-member answered
//! `REJECT_UNKNOWN_DEST`, `node1…` card requests, `REJECT_FULL`, the hub
//! killed mid-transfer and resumed by cursor, replays on fresh sockets, a
//! sender crash with items in custody followed by a direct new-epoch packet,
//! re-enrollment on a new device fetching custody already held (§7.3 floor),
//! a stale home-hub hint (a locator only), a direct write clearing a custody
//! reject's reason, forgetting the home hub, and enrollment attempts that end
//! without `ENROLL_OK` (§7.3): a spent code leaves no dial and no hub card on
//! the member, a refused re-enrollment keeps a member's dial, and an
//! abandoned wait releases what it started but never a configured peer's dial
//! or a contact that existed before. Lock fixes (integrated review): a lost
//! `CUSTODY_PUT` is re-PUT by the message's retry and a lost push re-fetched
//! by `retry_all` (m1); leaving the hub keeps a configured peer's dial at
//! the same address (m2); `hub.reachable` needs the hub's own answer on the
//! current connection, never just a dialed socket and an announce (m3).

mod common;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::*;
use mesh111::contacts::{card_text, ROLE_ALWAYS_ON_HUB, ROLE_MAILBOX};
use mesh111::envelope::OUTER_OVERHEAD;
use mesh111::fabric::{
    ContactChange, Fabric, FabricError, FabricEvent, FabricSettings, HomeHub, MessageChange,
    SettingsPatch,
};
use mesh111::link::lossy::{DropFilter, LossyConfig};
use mesh111::link::tcp_lan::{
    encode_frame, read_frame, read_hello, TcpLan, HELLO, T_ANNOUNCE, T_SEALED,
};
use mesh111::link::{Dialer, Frame, FrameType, Link, LinkKind, LinkState, PathProfile};
use mesh111::mailbox::EnrollStatus;
use mesh111::outbox::Reason;
use mesh111::store::{Class, MsgState, Trust};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

struct Node {
    ident: Arc<Ident>,
    f: Fabric,
    t: TcpLan,
    dir: PathBuf,
}

async fn hub(d: &Dirs) -> (Node, SocketAddr) {
    let ident = Ident::new("hub", ROLE_MAILBOX | ROLE_ALWAYS_ON_HUB);
    let dir = d.node("hub");
    let f = start(&ident, &dir, hub_settings());
    let t = tcp(&f, &ident, "tcp");
    add_tcp(&f, &t, true).await;
    let addr = listen(&t, 0).await;
    (Node { ident, f, t, dir }, addr)
}

/// A member enrolled at `h` (its fabric dials the hub and keeps it up).
async fn member(d: &Dirs, name: &str, h: &Node, haddr: SocketAddr) -> Node {
    member_as(d, name, Ident::new(name, 0), h, haddr).await
}

/// [`member`] with a given identity.
async fn member_as(d: &Dirs, name: &str, ident: Arc<Ident>, h: &Node, haddr: SocketAddr) -> Node {
    let dir = d.node(name);
    let f = start(&ident, &dir, FabricSettings::default());
    let t = tcp(&f, &ident, "tcp");
    add_tcp(&f, &t, true).await;
    let offer = h.f.offer(Some(haddr)).await.expect("offer");
    tokio::time::timeout(
        Duration::from_secs(10),
        f.enroll(haddr, &offer.code, Some(offer.card.as_str())),
    )
    .await
    .expect("enroll in time")
    .expect("enrolled");
    let s = f.settings().await.unwrap();
    assert_eq!(s.home_hub.map(|h| h.node_id), Some(h.ident.id()));
    Node { ident, f, t, dir }
}

fn member_settings(h: &Node) -> FabricSettings {
    FabricSettings {
        home_hub: Some(HomeHub {
            node_id: h.ident.id(),
            address: None,
        }),
        ..FabricSettings::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_members_and_a_hub_through_all_four_states() {
    let d = Dirs::new();
    let (h, haddr) = hub(&d).await;
    let b = member(&d, "bob", &h, haddr).await;
    // Bob goes away: the hub will hold his mail.
    b.t.release(haddr);
    let a_ident = Ident::new("alice", 0);
    let a = start(&a_ident, &d.node("alice"), FabricSettings::default());
    let at = tcp(&a, &a_ident, "tcp");
    add_tcp(&a, &at, true).await;
    exchange_cards(&a, &b.f).await;

    // queued: no path yet.
    let mut events = a.subscribe();
    let id = a.send(b.ident.id(), "hello bob", None).await.unwrap();
    let m = a.message(id).await.unwrap();
    assert_eq!(m.state, MsgState::Queued);
    assert_eq!(m.text.as_deref(), Some("hello bob"));

    // Alice enrolls: CUSTODY_PUT written (sent), CUSTODY_ACK ACCEPTED (relayed).
    let offer = h.f.offer(Some(haddr)).await.unwrap();
    a.enroll(haddr, &offer.code, Some(offer.card.as_str()))
        .await
        .unwrap();
    until_state(&a, id, MsgState::Relayed, 10).await;
    assert_eq!(h.f.status().await.unwrap().custody.0, 1);

    // Bob comes back: fetch on link-up, delivered, his ACK through the hub.
    b.t.maintain(haddr, Some(h.ident.id()));
    until_state(&a, id, MsgState::Delivered, 10).await;
    let got = inbox(&b.f, a_ident.id()).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].text.as_deref(), Some("hello bob"));
    assert_eq!(got[0].state, MsgState::Delivered);

    // Coalesced per message per tick: New first, then one State frame per
    // tick that changed it (sent, relayed, delivered).
    let mine: Vec<MessageChange> = drain_events(&mut events)
        .into_iter()
        .filter_map(|e| match e {
            FabricEvent::Message { id: i, change, .. } if i == id => Some(change),
            _ => None,
        })
        .collect();
    assert_eq!(mine.first(), Some(&MessageChange::New));
    assert!(
        mine.iter().filter(|c| **c == MessageChange::State).count() >= 3,
        "{mine:?}"
    );

    // A non-member (a card Alice holds, never enrolled at this hub):
    // REJECT_UNKNOWN_DEST keeps `sent` with the reason.
    let carol = Ident::new("carol", 0);
    a.add_contact_card(&card_text(&carol.announce_bytes()))
        .await
        .unwrap();
    let id2 = a.send(carol.id(), "hi carol", None).await.unwrap();
    let m2 = until(10, || async {
        let m = a.message(id2).await.ok()?;
        (m.reason == Some(Reason::HubNotYours)).then_some(m)
    })
    .await;
    assert_eq!(m2.state, MsgState::Sent);

    // `node1…` only: the hub answers the card request for its member Dave.
    let dave = member(&d, "dave", &h, haddr).await;
    let bech = id111::encoding::bech32_encode(dave.ident.id().as_bytes());
    let c = a.add_contact_node_id(&bech).await.unwrap();
    assert!(!c.has_key);
    let id3 = a.send(dave.ident.id(), "hi dave", None).await.unwrap();
    until(10, || async {
        let c = a.contact(dave.ident.id()).await.ok()?;
        c.has_key.then_some(())
    })
    .await;
    // Dave never met Alice: her first message rides INTRO through the
    // mailbox and is held as a message request (no ACK) until he accepts.
    until(10, || async {
        let c = dave.f.contact(a_ident.id()).await.ok()?;
        c.request.then_some(())
    })
    .await;
    until_state(&a, id3, MsgState::Relayed, 10).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        a.message(id3).await.unwrap().state,
        MsgState::Relayed,
        "a held request is never ACKed"
    );
    dave.f
        .patch_contact(
            a_ident.id(),
            mesh111::fabric::ContactPatch {
                accept: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    until_state(&a, id3, MsgState::Delivered, 10).await;
    assert_eq!(inbox(&dave.f, a_ident.id()).await.len(), 1);
    let st = h.f.status().await.unwrap();
    assert_eq!(st.hub.members, 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mailbox_full_keeps_sent_with_its_reason() {
    let d = Dirs::new();
    let (h, haddr) = hub(&d).await;
    let a = member(&d, "alice", &h, haddr).await;
    let b = member(&d, "bob", &h, haddr).await;
    exchange_cards(&a.f, &b.f).await;
    b.t.release(haddr);
    h.f.patch_settings(SettingsPatch {
        quotas: vec![(Class::Custody, Some(1))],
        ..SettingsPatch::default()
    })
    .await
    .unwrap();
    let id = a.f.send(b.ident.id(), "no room", None).await.unwrap();
    let m = until(10, || async {
        let m = a.f.message(id).await.ok()?;
        (m.reason == Some(Reason::MailboxFull)).then_some(m)
    })
    .await;
    assert_eq!(m.state, MsgState::Sent);
    // Capacity frees: the next fabric event retries and the hub holds it.
    h.f.patch_settings(SettingsPatch {
        quotas: vec![(Class::Custody, None)],
        ..SettingsPatch::default()
    })
    .await
    .unwrap();
    a.f.retry(id).await.unwrap();
    until_state(&a.f, id, MsgState::Relayed, 10).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hub_killed_mid_transfer_resumes_by_cursor() {
    let d = Dirs::new();
    let (h, haddr) = hub(&d).await;
    let a = member(&d, "alice", &h, haddr).await;
    let b = member(&d, "bob", &h, haddr).await;
    exchange_cards(&a.f, &b.f).await;

    let m1 = a.f.send(b.ident.id(), "one", None).await.unwrap();
    until_state(&a.f, m1, MsgState::Delivered, 10).await;
    let c1 = b.f.status().await.unwrap().hub.cursor;
    assert!(c1 > 0, "bob committed his cursor");
    let items_before = b.f.status().await.unwrap().counters.items_received;

    // Bob leaves; two more go into custody.
    b.t.release(haddr);
    let m2 = a.f.send(b.ident.id(), "two", None).await.unwrap();
    let m3 = a.f.send(b.ident.id(), "three", None).await.unwrap();
    until_state(&a.f, m2, MsgState::Relayed, 10).await;
    until_state(&a.f, m3, MsgState::Relayed, 10).await;

    // The hub dies (no clean close) and comes back on the same port.
    h.f.abort();
    h.t.shutdown();
    let h2 = start(&h.ident, &h.dir, hub_settings());
    let t2 = tcp(&h2, &h.ident, "tcp");
    add_tcp(&h2, &t2, true).await;
    listen(&t2, haddr.port()).await;

    // Bob returns and fetches from his cursor: only what he has not seen.
    b.t.maintain(haddr, Some(h.ident.id()));
    until_state(&a.f, m2, MsgState::Delivered, 15).await;
    until_state(&a.f, m3, MsgState::Delivered, 15).await;
    let got: Vec<String> = inbox(&b.f, a.ident.id())
        .await
        .into_iter()
        .filter_map(|m| m.text)
        .collect();
    assert_eq!(got, vec!["one", "two", "three"]);
    let st = b.f.status().await.unwrap();
    assert_eq!(
        st.counters.items_received - items_before,
        2,
        "items above the cursor only"
    );
    assert!(st.hub.cursor > c1);
}

/// Read every frame a raw `tcp-lan` peer receives within `ms`.
async fn frames_seen(sock: &mut TcpStream, ms: u64) -> Vec<u8> {
    let mut types = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return types;
        }
        match tokio::time::timeout(left, read_frame(sock)).await {
            Ok(Ok((ty, _))) => types.push(ty),
            _ => return types,
        }
    }
}

async fn raw_peer(haddr: SocketAddr, announce: &[u8], sealed: Option<&[u8]>) -> TcpStream {
    let mut s = TcpStream::connect(haddr).await.unwrap();
    s.write_all(&HELLO).await.unwrap();
    s.write_all(&encode_frame(T_ANNOUNCE, announce).unwrap())
        .await
        .unwrap();
    if let Some(p) = sealed {
        s.write_all(&encode_frame(T_SEALED, p).unwrap())
            .await
            .unwrap();
    }
    read_hello(&mut s).await.unwrap();
    let (ty, _) = read_frame(&mut s).await.unwrap();
    assert_eq!(ty, T_ANNOUNCE, "the hub announces first");
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replays_on_fresh_sockets_never_attribute_or_get_pushes() {
    let d = Dirs::new();
    let (h, haddr) = hub(&d).await;
    let a = member(&d, "alice", &h, haddr).await;
    let b = member(&d, "bob", &h, haddr).await;
    exchange_cards(&a.f, &b.f).await;
    // A packet Bob really sealed to the hub (his last fetch or commit).
    let captured = b
        .ident
        .sealed_to(&h.ident.enc())
        .pop()
        .expect("bob sealed to the hub");
    b.t.release(haddr);
    until(10, || async {
        let s = h.f.status().await.ok()?;
        (s.links[0].peers == 1).then_some(())
    })
    .await;
    let stale_before = h.f.status().await.unwrap().counters.stale;

    // A replayed public announce, and a replayed sealed packet, each on a
    // fresh socket claiming to be Bob.
    let mut ann_sock = raw_peer(haddr, &b.ident.announce_bytes(), None).await;
    let mut seal_sock = raw_peer(haddr, &b.ident.announce_bytes(), Some(&captured)).await;
    until(10, || async {
        let s = h.f.status().await.ok()?;
        (s.counters.stale > stale_before).then_some(())
    })
    .await;

    // Mail for Bob arrives at the hub while only the impostors are there.
    let id = a.f.send(b.ident.id(), "for bob only", None).await.unwrap();
    until_state(&a.f, id, MsgState::Relayed, 10).await;
    let seen_ann = frames_seen(&mut ann_sock, 600).await;
    let seen_seal = frames_seen(&mut seal_sock, 100).await;
    assert!(
        !seen_ann.contains(&T_SEALED),
        "a replayed announce gets no push: {seen_ann:?}"
    );
    assert!(
        !seen_seal.contains(&T_SEALED),
        "a replayed sealed packet never attributes: {seen_seal:?}"
    );
    let st = h.f.status().await.unwrap();
    assert_eq!(st.links[0].peers, 1, "only Alice is attributed");

    // The real Bob, attributed by a fresh packet, gets it.
    b.t.maintain(haddr, Some(h.ident.id()));
    until_state(&a.f, id, MsgState::Delivered, 10).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sender_crash_new_epoch_delivers_every_message_once() {
    let d = Dirs::new();
    let (h, haddr) = hub(&d).await;
    let a = member(&d, "alice", &h, haddr).await;
    let b = member(&d, "bob", &h, haddr).await;
    exchange_cards(&a.f, &b.f).await;
    let epoch1 = a.f.status().await.unwrap().store.epoch.unwrap();

    // Bob stops cleanly; two messages go into custody.
    b.f.shutdown().await.unwrap();
    b.t.shutdown();
    let m1 =
        a.f.send(b.ident.id(), "first-in-custody", None)
            .await
            .unwrap();
    let m2 =
        a.f.send(b.ident.id(), "second-in-custody", None)
            .await
            .unwrap();
    until_state(&a.f, m1, MsgState::Relayed, 10).await;
    until_state(&a.f, m2, MsgState::Relayed, 10).await;

    // Alice crashes and restarts: a new epoch.
    a.f.abort();
    a.t.shutdown();
    let a2 = start(&a.ident, &a.dir, member_settings(&h));
    let epoch2 = a2.status().await.unwrap().store.epoch.unwrap();
    assert!(epoch2 > epoch1, "an unclean open mints a new epoch");
    let b2 = start(&b.ident, &b.dir, member_settings(&h));

    // A direct lossy link where only the new-epoch packet gets through (the
    // custody messages' frames are shorter and dropped).
    let filter: mesh111::link::lossy::DropFilter = Arc::new(|f| f.bytes.len() < 120);
    let cfg_a = LossyConfig {
        filter: Some(filter),
        ..LossyConfig::default()
    };
    let (ea, _eb) = lossy_pair(
        (&a2, &a.ident),
        (&b2, &b.ident),
        cfg_a,
        LossyConfig::default(),
    )
    .await;
    let m3 = a2
        .send(b.ident.id(), "third-direct-new-epoch-xxxxxxxxx", None)
        .await
        .unwrap();
    until(10, || async {
        let got = inbox(&b2, a.ident.id()).await;
        (got.len() == 1).then_some(())
    })
    .await;
    let first = inbox(&b2, a.ident.id()).await;
    assert_eq!(first[0].seq.epoch, epoch2);

    // Bob fetches custody: the older epoch is delivered against a frozen entry.
    let bt = tcp(&b2, &b.ident, "tcp");
    add_tcp(&b2, &bt, false).await;
    bt.maintain(haddr, Some(h.ident.id()));
    for id in [m1, m2] {
        until_state(&a2, id, MsgState::Delivered, 15).await;
    }
    until_state(&a2, m3, MsgState::Delivered, 15).await;

    // The direct path opens up; retransmissions of anything still pending
    // would be re-ACKed, never redelivered.
    ea.set_filter(None);
    a2.retry_all().await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut texts: Vec<String> = inbox(&b2, a.ident.id())
        .await
        .into_iter()
        .filter_map(|m| m.text)
        .collect();
    texts.sort();
    assert_eq!(
        texts,
        vec![
            "first-in-custody",
            "second-in-custody",
            "third-direct-new-epoch-xxxxxxxxx"
        ],
        "every message exactly once"
    );
    let epochs: Vec<u64> = inbox(&b2, a.ident.id())
        .await
        .iter()
        .map(|m| m.seq.epoch)
        .collect();
    assert_eq!(epochs, vec![epoch2, epoch1, epoch1]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reenrolling_on_a_new_device_fetches_custody_already_held() {
    let d = Dirs::new();
    let (h, haddr) = hub(&d).await;
    let a = member(&d, "alice", &h, haddr).await;
    let b = member(&d, "bob", &h, haddr).await;
    exchange_cards(&a.f, &b.f).await;
    // History at the hub: one item streamed, delivered and committed.
    let m0 = a.f.send(b.ident.id(), "zero", None).await.unwrap();
    until_state(&a.f, m0, MsgState::Delivered, 10).await;

    // Bob's device is gone for good; two items wait in custody for him.
    b.f.shutdown().await.unwrap();
    b.t.shutdown();
    let m1 = a.f.send(b.ident.id(), "one", None).await.unwrap();
    let m2 = a.f.send(b.ident.id(), "two", None).await.unwrap();
    until_state(&a.f, m1, MsgState::Relayed, 10).await;
    until_state(&a.f, m2, MsgState::Relayed, 10).await;

    // Bob restores his identity on a new device (an empty store), re-adds
    // Alice and re-enrolls with a fresh offer: the hub's floor sits below
    // what it still holds for him, so his first fetch streams it (§7.3).
    let b2 = start(&b.ident, &d.node("bob-new"), FabricSettings::default());
    let t2 = tcp(&b2, &b.ident, "tcp");
    add_tcp(&b2, &t2, true).await;
    b2.add_contact_card(&a.f.card().await.unwrap().text)
        .await
        .unwrap();
    let offer = h.f.offer(Some(haddr)).await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(10),
        b2.enroll(haddr, &offer.code, Some(offer.card.as_str())),
    )
    .await
    .expect("enroll in time")
    .expect("re-enrolled");
    until_state(&a.f, m1, MsgState::Delivered, 15).await;
    until_state(&a.f, m2, MsgState::Delivered, 15).await;
    let got: Vec<String> = inbox(&b2, a.ident.id())
        .await
        .into_iter()
        .filter_map(|m| m.text)
        .collect();
    assert_eq!(got, vec!["one", "two"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stale_home_hub_hint_never_preempts_the_hub() {
    let d = Dirs::new();
    let (h, haddr) = hub(&d).await;
    let a = member(&d, "alice", &h, haddr).await;
    // Bob's card still hints at the hub he left; he is a member here now.
    let mut app = appdata(0);
    app.extend_from_slice(&[0xEE; 8]);
    let b = member_as(&d, "bob", Ident::with_appdata("bob", app), &h, haddr).await;
    exchange_cards(&a.f, &b.f).await;
    assert_eq!(
        a.f.contact(b.ident.id()).await.unwrap().home_hub,
        Some(vec![0xEE; 8])
    );
    b.t.release(haddr);
    // The hint is a locator only (§7.3): the hub's verdict decides.
    let id = a.f.send(b.ident.id(), "you moved", None).await.unwrap();
    let m = until_state(&a.f, id, MsgState::Relayed, 10).await;
    assert_eq!(m.reason, None);
    b.t.maintain(haddr, Some(h.ident.id()));
    until_state(&a.f, id, MsgState::Delivered, 10).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_direct_write_clears_a_custody_reject_reason() {
    let d = Dirs::new();
    let (h, haddr) = hub(&d).await;
    let a = member(&d, "alice", &h, haddr).await;
    // Carol is not a member: the hub answers REJECT_UNKNOWN_DEST.
    let ci = Ident::new("carol", 0);
    let carol = start(&ci, &d.node("carol"), FabricSettings::default());
    exchange_cards(&a.f, &carol).await;
    let id = a.f.send(ci.id(), "hi carol", None).await.unwrap();
    let m = until(10, || async {
        let m = a.f.message(id).await.ok()?;
        (m.reason == Some(Reason::HubNotYours)).then_some(m)
    })
    .await;
    assert_eq!(m.state, MsgState::Sent);
    // A direct path appears; Carol's ACKs are lost, so the message stays
    // `sent` — but it was written to her directly, so the hub's reject no
    // longer explains it.
    let drop_all: DropFilter = Arc::new(|_| true);
    let (_ea, _eb) = lossy_pair(
        (&a.f, &a.ident),
        (&carol, &ci),
        LossyConfig::default(),
        LossyConfig {
            filter: Some(drop_all),
            ..LossyConfig::default()
        },
    )
    .await;
    let m = until(10, || async {
        let m = a.f.message(id).await.ok()?;
        m.reason.is_none().then_some(m)
    })
    .await;
    assert_eq!(m.state, MsgState::Sent);
    assert_eq!(inbox(&carol, a.ident.id()).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forgetting_the_home_hub_ends_the_membership_here() {
    let d = Dirs::new();
    let (h, haddr) = hub(&d).await;
    let a = member(&d, "alice", &h, haddr).await;
    assert_eq!(a.t.connection_count(), 1);
    let mut events = a.f.subscribe();
    a.f.forget_contact(h.ident.id()).await.unwrap();
    assert_eq!(a.f.settings().await.unwrap().home_hub, None);
    let st = a.f.status().await.unwrap();
    assert!(!st.hub.enrolled && !st.hub.reachable);
    assert!(drain_events(&mut events)
        .iter()
        .any(|e| matches!(e, FabricEvent::Settings(s) if s.home_hub.is_none())));
    // The maintained dial is released: the connection closes and stays
    // closed (no redial loop against a forgotten hub).
    until(10, || async { (a.t.connection_count() == 0).then_some(()) }).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(a.t.connection_count(), 0);
}

/// A node with a `tcp-lan` dialer and nothing else: no listener, no hub.
async fn bare(d: &Dirs, name: &str) -> Node {
    let ident = Ident::new(name, 0);
    let dir = d.node(name);
    let f = start(&ident, &dir, FabricSettings::default());
    let t = tcp(&f, &ident, "tcp");
    add_tcp(&f, &t, true).await;
    Node { ident, f, t, dir }
}

/// Attributed peers on the hub's `tcp-lan` link.
async fn hub_peers(h: &Node) -> usize {
    h.f.status().await.unwrap().links[0].peers
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_spent_code_leaves_no_dial_and_no_hub_card() {
    let d = Dirs::new();
    let (h, haddr) = hub(&d).await;
    // Alice spends the code.
    let offer = h.f.offer(Some(haddr)).await.unwrap();
    let a = bare(&d, "alice").await;
    tokio::time::timeout(
        Duration::from_secs(10),
        a.f.enroll(haddr, &offer.code, Some(offer.card.as_str())),
    )
    .await
    .expect("enroll in time")
    .expect("alice enrolled");
    until(10, || async { (hub_peers(&h).await == 1).then_some(()) }).await;

    // Carol presents the same code with the hub's card: the hub refuses.
    let c = bare(&d, "carol").await;
    let mut events = c.f.subscribe();
    let r = tokio::time::timeout(
        Duration::from_secs(10),
        c.f.enroll(haddr, &offer.code, Some(offer.card.as_str())),
    )
    .await
    .expect("answered in time");
    assert!(
        matches!(r, Err(FabricError::Enroll(EnrollStatus::Refused))),
        "{r:?}"
    );
    // No new pin: the card pinned for the attempt is gone again.
    assert!(matches!(
        c.f.contact(h.ident.id()).await,
        Err(FabricError::NotFound)
    ));
    assert!(c.f.contacts().await.unwrap().is_empty());
    assert_eq!(c.f.settings().await.unwrap().home_hub, None);
    let seen = drain_events(&mut events);
    assert!(
        seen.iter().any(|e| matches!(e, FabricEvent::Contact { node_id, change: ContactChange::Removed } if *node_id == h.ident.id())),
        "{seen:?}"
    );
    // No maintained dial: the connection closes and stays closed, and the
    // adapter has no dial target left.
    until(10, || async { (c.t.connection_count() == 0).then_some(()) }).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(c.t.connection_count(), 0, "no redial");
    assert_eq!(c.t.state(), LinkState::Down, "no dial target");
    // The hub counts Alice alone again.
    until(10, || async { (hub_peers(&h).await == 1).then_some(()) }).await;
    assert_eq!(h.f.status().await.unwrap().hub.members, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_reenrollment_keeps_the_members_dial() {
    let d = Dirs::new();
    let (h, haddr) = hub(&d).await;
    let a = member(&d, "alice", &h, haddr).await;
    assert_eq!(a.t.connection_count(), 1);
    // A code that no longer opens anything.
    let offer = h.f.offer(Some(haddr)).await.unwrap();
    assert!(h.f.dismiss_offer().await.unwrap());
    for card in [None, Some(offer.card.as_str())] {
        let r = tokio::time::timeout(
            Duration::from_secs(10),
            a.f.enroll(haddr, &offer.code, card),
        )
        .await
        .expect("answered in time");
        assert!(matches!(r, Err(FabricError::Enroll(_))), "{r:?}");
    }
    // The membership and its dial are untouched.
    let s = a.f.settings().await.unwrap();
    assert_eq!(s.home_hub.map(|x| x.node_id), Some(h.ident.id()));
    assert_eq!(s.home_hub.and_then(|x| x.address), Some(haddr));
    assert_eq!(
        a.f.contact(h.ident.id()).await.unwrap().trust,
        Trust::Enrolled
    );
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(a.t.connection_count(), 1, "the home hub's dial stays");
    assert!(a.f.status().await.unwrap().hub.reachable);
    assert_eq!(hub_peers(&h).await, 1);
    // ... and still carries mail.
    let b = member(&d, "bob", &h, haddr).await;
    exchange_cards(&a.f, &b.f).await;
    b.t.release(haddr);
    let id =
        a.f.send(b.ident.id(), "still a member", None)
            .await
            .unwrap();
    until_state(&a.f, id, MsgState::Relayed, 10).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_abandoned_wait_releases_what_it_started_only() {
    let d = Dirs::new();
    let (h, _haddr) = hub(&d).await;
    let offer = h.f.offer(None).await.unwrap();
    // Accepts TCP (the kernel backlog) but never speaks: the attempt waits.
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let saddr = silent.local_addr().unwrap();
    let c = bare(&d, "carol").await;

    // An attempt whose caller goes away (a route's bound elapsing drops its
    // future the same way): the dial it started and the card it pinned go.
    let wait = {
        let (f, code, card) = (c.f.clone(), offer.code.clone(), offer.card.clone());
        tokio::spawn(async move { f.enroll(saddr, &code, Some(&card)).await })
    };
    until(10, || async {
        matches!(c.t.state(), LinkState::Up { .. }).then_some(())
    })
    .await;
    assert!(c.f.contact(h.ident.id()).await.is_ok(), "pinned meanwhile");
    wait.abort();
    until(10, || async {
        (c.t.state() == LinkState::Down).then_some(())
    })
    .await;
    until(10, || async {
        matches!(c.f.contact(h.ident.id()).await, Err(FabricError::NotFound)).then_some(())
    })
    .await;

    // The same with a timeout around the wait: a contact that existed
    // before the attempt is kept.
    c.f.add_contact_card(&offer.card).await.unwrap();
    let r = tokio::time::timeout(
        Duration::from_millis(300),
        c.f.enroll(saddr, &offer.code, Some(offer.card.as_str())),
    )
    .await;
    assert!(r.is_err(), "nothing answers there");
    until(10, || async {
        (c.t.state() == LinkState::Down).then_some(())
    })
    .await;
    assert_eq!(
        c.f.contact(h.ident.id()).await.unwrap().trust,
        Trust::Oob,
        "an existing contact is kept"
    );

    // A configured peer's dial at that address is never released.
    c.f.dial_peer(saddr, h.ident.id()).await.unwrap();
    let r = tokio::time::timeout(
        Duration::from_millis(300),
        c.f.enroll(saddr, &offer.code, Some(offer.card.as_str())),
    )
    .await;
    assert!(r.is_err());
    // The dropped wait queued its abandonment first: one round trip later
    // the attempt has ended.
    c.f.status().await.unwrap();
    assert!(
        matches!(c.t.state(), LinkState::Up { .. }),
        "the configured peer's dial stays"
    );
    drop(silent);
}

/// A `tcp-lan` test bearer with stream sizes (no fragmenting), optionally
/// losing frames in its sending direction.
fn tcp_like(filter: Option<DropFilter>) -> LossyConfig {
    LossyConfig {
        profile: PathProfile::tcp_lan(),
        kind: LinkKind::TcpLan,
        filter,
        ..LossyConfig::default()
    }
}

/// Drops the first sealed frame big enough to carry a sealed item inside
/// (a `CUSTODY_PUT` or a `MAILBOX_ITEM`; a fetch or a commit is far
/// smaller), then nothing.
fn lose_first_item() -> DropFilter {
    let seen = Arc::new(AtomicUsize::new(0));
    Arc::new(move |f: &Frame| {
        f.frame_type == FrameType::Sealed
            && f.bytes.len() > 2 * OUTER_OVERHEAD + 32
            && seen.fetch_add(1, Ordering::SeqCst) == 0
    })
}

/// m1: a `CUSTODY_PUT` lost on a live connection keeps its fragment in
/// flight (no ACK, no disconnect, no timer). The message's retry forgets
/// that, and a second PUT goes out and is held.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lost_custody_put_is_put_again_by_the_messages_retry() {
    let d = Dirs::new();
    let (h, haddr) = hub(&d).await;
    let a = member(&d, "alice", &h, haddr).await;
    let b = member(&d, "bob", &h, haddr).await;
    exchange_cards(&a.f, &b.f).await;
    b.t.release(haddr);
    a.t.release(haddr);
    until(10, || async { (hub_peers(&h).await == 0).then_some(()) }).await;
    // Alice's only path to the hub now loses her first item-sized frame.
    let (ea, _eh) = lossy_pair(
        (&a.f, &a.ident),
        (&h.f, &h.ident),
        tcp_like(Some(lose_first_item())),
        tcp_like(None),
    )
    .await;
    let id =
        a.f.send(b.ident.id(), "lost on the way", None)
            .await
            .unwrap();
    until_state(&a.f, id, MsgState::Sent, 10).await;
    until(10, || async { (ea.stats().dropped == 1).then_some(()) }).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        a.f.message(id).await.unwrap().state,
        MsgState::Sent,
        "nothing answers a lost PUT"
    );
    assert_eq!(h.f.status().await.unwrap().custody.0, 0);
    let sent = ea.stats().sent;
    a.f.retry(id).await.unwrap();
    until_state(&a.f, id, MsgState::Relayed, 10).await;
    assert!(ea.stats().sent > sent, "a second CUSTODY_PUT went out");
    assert_eq!(ea.stats().dropped, 1);
    assert_eq!(h.f.status().await.unwrap().custody.0, 1);
}

/// m1: a pushed `MAILBOX_ITEM` lost on a live connection is streamed again
/// only when the member fetches again: `retry_all` (what the daemon calls on
/// a `fabric` event with new connectivity) does, after link-up.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retry_all_after_link_up_fetches_a_lost_push_again() {
    let d = Dirs::new();
    let (h, haddr) = hub(&d).await;
    let a = member(&d, "alice", &h, haddr).await;
    let b = member(&d, "bob", &h, haddr).await;
    exchange_cards(&a.f, &b.f).await;
    b.t.release(haddr);
    until(10, || async { (hub_peers(&h).await == 1).then_some(()) }).await;
    let id =
        a.f.send(b.ident.id(), "held, then lost once", None)
            .await
            .unwrap();
    until_state(&a.f, id, MsgState::Relayed, 10).await;
    // Bob's way back to the hub loses the hub's first item to him.
    let (_eb, eh) = lossy_pair(
        (&b.f, &b.ident),
        (&h.f, &h.ident),
        tcp_like(None),
        tcp_like(Some(lose_first_item())),
    )
    .await;
    until(10, || async { (eh.stats().dropped == 1).then_some(()) }).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(inbox(&b.f, a.ident.id()).await.is_empty());
    assert_eq!(a.f.message(id).await.unwrap().state, MsgState::Relayed);
    b.f.retry_all().await.unwrap();
    until_state(&a.f, id, MsgState::Delivered, 10).await;
    let got = inbox(&b.f, a.ident.id()).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].text.as_deref(), Some("held, then lost once"));
}

/// m2: leaving the hub (forgetting its contact) keeps the dial when a
/// configured direct peer sits at the same address.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn leaving_the_hub_keeps_a_configured_peers_dial_at_its_address() {
    let d = Dirs::new();
    let (h, haddr) = hub(&d).await;
    let a = member(&d, "alice", &h, haddr).await;
    assert_eq!(a.t.connection_count(), 1);
    a.f.dial_peer(haddr, h.ident.id()).await.unwrap();
    a.f.forget_contact(h.ident.id()).await.unwrap();
    assert_eq!(a.f.settings().await.unwrap().home_hub, None);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        a.t.connection_count(),
        1,
        "the configured peer's dial stays"
    );
    assert!(matches!(a.t.state(), LinkState::Up { .. }));
}

/// A listener that answers the `tcp-lan` handshake with `announce` (public
/// bytes anyone can replay) and then reads everything and answers nothing.
async fn silent_hub(announce: Vec<u8>) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut socks = Vec::new();
        while let Ok((mut s, _)) = l.accept().await {
            let ann = announce.clone();
            socks.push(tokio::spawn(async move {
                if s.write_all(&HELLO).await.is_err()
                    || s.write_all(&encode_frame(T_ANNOUNCE, &ann).unwrap())
                        .await
                        .is_err()
                    || read_hello(&mut s).await.is_err()
                {
                    return;
                }
                while read_frame(&mut s).await.is_ok() {}
            }));
        }
    });
    (addr, task)
}

/// m3: a dialed connection whose announce names the hub is `connected`,
/// but `reachable` stays false until the hub itself answers on it — an
/// announce authenticates nothing (§8.1), and the fetch sent on link-up
/// gets no answer from a hub that is not there.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hub_that_never_answers_is_connected_but_not_reachable() {
    let d = Dirs::new();
    let (h, _haddr) = hub(&d).await;
    let (fake, task) = silent_hub(h.ident.announce_bytes()).await;
    let ai = Ident::new("alice", 0);
    let settings = FabricSettings {
        home_hub: Some(HomeHub {
            node_id: h.ident.id(),
            address: Some(fake),
        }),
        ..FabricSettings::default()
    };
    let a = start(&ai, &d.node("alice"), settings);
    a.add_contact_card(&h.f.card().await.unwrap().text)
        .await
        .unwrap();
    let at = tcp(&a, &ai, "tcp");
    add_tcp(&a, &at, true).await;
    until(10, || async {
        let s = a.status().await.ok()?;
        s.hub.connected.then_some(())
    })
    .await;
    // The fetch went out on link-up (sealed to the hub) and nothing came back.
    until(10, || async {
        (!ai.sealed_to(&h.ident.enc()).is_empty()).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let st = a.status().await.unwrap();
    assert!(st.hub.enrolled && st.hub.connected);
    assert!(!st.hub.reachable, "self-judged from a dialed socket");
    assert_eq!(st.links[0].peers, 0, "nothing attributed");
    task.abort();
}

/// m3: every link-up fetch is answered, an empty mailbox too (an item-less
/// `MAILBOX_ITEM`), so `reachable` comes back after a reconnect and after
/// the hub's restart — from the hub's answer on the new connection, never
/// from the one before.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_empty_mailbox_fetch_is_answered_so_reachable_returns_after_link_up() {
    let d = Dirs::new();
    let (h, haddr) = hub(&d).await;
    let a = member(&d, "alice", &h, haddr).await;
    // Enrolled: ENROLL_OK answered on this connection.
    let st = a.f.status().await.unwrap();
    assert!(st.hub.connected && st.hub.reachable);
    let before = st.counters;
    let cursor = st.hub.cursor;
    let gone = || async {
        let s = a.f.status().await.ok()?;
        (!s.hub.connected && !s.hub.reachable).then_some(())
    };

    // The link drops, then comes back with nothing held for Alice.
    a.t.release(haddr);
    until(10, gone).await;
    assert_eq!(h.f.status().await.unwrap().custody.0, 0);
    a.t.maintain(haddr, Some(h.ident.id()));
    until(10, || async {
        a.f.status().await.ok()?.hub.reachable.then_some(())
    })
    .await;
    let st = a.f.status().await.unwrap();
    assert!(st.hub.connected);
    assert_eq!(st.links[0].peers, 1, "the hub's connection is attributed");
    assert_eq!(st.counters.items_received, before.items_received, "no item");
    assert_eq!(st.counters.dropped, before.dropped, "a marker, not junk");
    assert_eq!(st.hub.cursor, cursor, "the cursor stays");

    // The hub dies and comes back on the same port: the old connection's
    // answer does not carry over; the new one's does.
    h.f.abort();
    h.t.shutdown();
    until(10, gone).await;
    let h2 = start(&h.ident, &h.dir, hub_settings());
    let t2 = tcp(&h2, &h.ident, "tcp");
    add_tcp(&h2, &t2, true).await;
    listen(&t2, haddr.port()).await;
    until(10, || async {
        a.f.status().await.ok()?.hub.reachable.then_some(())
    })
    .await;
    assert_eq!(a.f.status().await.unwrap().hub.cursor, cursor);
}
