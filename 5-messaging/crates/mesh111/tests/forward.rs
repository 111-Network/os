//! Layer 7 §5 cross-hub custody in process, over real `tcp-lan` sockets on
//! loopback (the mesh111 side of §15 G9 and G10): the widened gate's table
//! (§5.1, §5.2) answered on raw connections, the carrier bound (§6.1 item
//! 3), the probe order and the hit cache (§5.5), refused rows and the
//! per-origin share (§5.8), `REJECT_FULL` kept for its pair and retried on
//! events only (C16), `REJECT_UNENROLLED` breaking a pair, a half pair
//! forwarding nothing, the own-send limit (§5.1, D31 (3)), rows resuming
//! after a restart, the carrier hand-off (§5.9: posts, re-posts, acks
//! consumed on take, the longest path leaf → gateway → carrier → gateway′ →
//! leaf′ and back), and with no pairing Layer 5's answers and frames
//! (`tests/hub_flow.rs` stays unchanged and green).

mod common;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use id111::keyset::NodeId;
use mesh111::contacts::{ROLE_ALWAYS_ON_HUB, ROLE_MAILBOX};
use mesh111::custody::{pkt_ref, CustodyAck, CustodyStatus};
use mesh111::envelope::{
    open_envelope, seal_envelope, Inner, Kind, PeerKeys, Priority, DEST_HUB_LEN,
};
use mesh111::fabric::{Fabric, FabricEvent, FabricSettings, ForwardDone, PairView, SettingsPatch};
use mesh111::ids::MsgSeq;
use mesh111::link::tcp_lan::{
    encode_frame, read_frame, read_hello, TcpLan, HELLO, T_ANNOUNCE, T_SEALED,
};
use mesh111::link::Dialer;
use mesh111::mailbox::{decode_put, encode_put};
use mesh111::outbox::Reason;
use mesh111::store::{Class, ForwardItem, Fwd, MsgState};
use mesh111::{PairRole, PairVia};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

const HUB_ROLES: u16 = ROLE_MAILBOX | ROLE_ALWAYS_ON_HUB;
const T: Duration = Duration::from_secs(10);

struct Node {
    ident: Arc<Ident>,
    f: Fabric,
    t: TcpLan,
    /// A hub's listener (a member's home hub's).
    addr: SocketAddr,
    dir: PathBuf,
}

impl Node {
    fn id(&self) -> NodeId {
        self.ident.id()
    }

    fn keys(&self) -> PeerKeys {
        PeerKeys {
            node_id: self.ident.id(),
            enc_pubkey: self.ident.enc(),
        }
    }
}

fn prefix(n: &NodeId) -> [u8; DEST_HUB_LEN] {
    let mut h = [0u8; DEST_HUB_LEN];
    h.copy_from_slice(&n.as_bytes()[..DEST_HUB_LEN]);
    h
}

async fn hub(d: &Dirs, name: &str) -> Node {
    let ident = Ident::new(name, HUB_ROLES);
    let dir = d.node(name);
    let f = start(&ident, &dir, hub_settings());
    let t = tcp(&f, &ident, "tcp");
    add_tcp(&f, &t, true).await;
    let addr = listen(&t, 0).await;
    Node {
        ident,
        f,
        t,
        addr,
        dir,
    }
}

/// A member enrolled at `h`; with `hint` its card names `h` as its home hub
/// (the `appdata` hint, A8), so a contact's put to it carries TLV `0x02`.
async fn member(d: &Dirs, name: &str, h: &Node, hint: bool) -> Node {
    let mut app = appdata(0);
    if hint {
        app.extend_from_slice(&prefix(&h.id()));
    }
    let ident = Ident::with_appdata(name, app);
    let dir = d.node(name);
    let f = start(&ident, &dir, FabricSettings::default());
    let t = tcp(&f, &ident, "tcp");
    add_tcp(&f, &t, true).await;
    let offer = h.f.offer(Some(h.addr)).await.expect("offer");
    tokio::time::timeout(T, f.enroll(h.addr, &offer.code, Some(offer.card.as_str())))
        .await
        .expect("enroll in time")
        .expect("enrolled");
    Node {
        ident,
        f,
        t,
        addr: h.addr,
        dir,
    }
}

/// A node that is nobody's member (a contact on no island).
fn stranger(d: &Dirs, name: &str) -> Node {
    let ident = Ident::new(name, 0);
    let dir = d.node(name);
    let f = start(&ident, &dir, FabricSettings::default());
    let t = tcp(&f, &ident, "tcp");
    Node {
        ident,
        f,
        t,
        addr: "127.0.0.1:9".parse().unwrap(),
        dir,
    }
}

/// Pair `a` and `b` (§4.6 step 3): `b` scans `a`'s bundle and enrolls at
/// `a` on its own dial; `a` then enrolls at `b` over that same connection
/// (the second `ENROLL` rides it, S22). `role` is `b`'s role from `a`'s side.
async fn pair(a: &Node, b: &Node, role: PairRole, via: PairVia) {
    let oa = a.f.offer(Some(a.addr)).await.expect("offer a");
    let rec = tokio::time::timeout(
        T,
        b.f.pair(Some(a.addr), &oa.code, &oa.card, role.complement(), via),
    )
    .await
    .expect("b in time")
    .expect("b paired");
    assert_eq!(
        (rec.node_id, rec.role, rec.via),
        (a.id(), role.complement(), via)
    );
    let ob = b.f.offer(None).await.expect("offer b");
    tokio::time::timeout(T, a.f.pair(None, &ob.code, &ob.card, role, via))
        .await
        .expect("a in time")
        .expect("a paired over b's connection");
    for (x, y) in [(a, b), (b, a)] {
        let y = y.id();
        until(10, || async {
            let s = x.f.status().await.ok()?;
            s.paired
                .iter()
                .any(|p| p.hub.node_id == y && p.whole)
                .then_some(())
        })
        .await;
        // A pairing sets no home hub and writes no cursor (am. 10).
        let st = x.f.status().await.unwrap();
        assert!(st.settings.home_hub.is_none() && st.hub.cursor == 0);
    }
}

async fn pair_view(f: &Fabric, of: &Node) -> PairView {
    let id = of.id();
    f.status()
        .await
        .unwrap()
        .paired
        .into_iter()
        .find(|p| p.hub.node_id == id)
        .expect("paired")
}

/// Every `CUSTODY_PUT` `from` sealed to `to`: `(dest, item, hint)`, oldest
/// first (opened with `to`'s keys, as `to` would).
fn puts(from: &Node, to: &Node) -> Vec<(NodeId, Vec<u8>, Option<[u8; DEST_HUB_LEN]>)> {
    let k = from.keys();
    from.ident
        .sealed_to(&to.ident.enc())
        .iter()
        .filter_map(|pkt| {
            let o = open_envelope(&*to.ident, pkt, &|_| vec![k]).ok()?;
            if o.inner.kind != Kind::CustodyPut {
                return None;
            }
            let (dest, item) = decode_put(&o.inner.payload).ok()?;
            Some((dest, item.to_vec(), o.inner.ext.dest_hub))
        })
        .collect()
}

/// The forward row holding `item` at `f` (`None`: a member row, or none).
async fn row(f: &Fabric, item: &[u8]) -> Option<ForwardItem> {
    let r = pkt_ref(item);
    f.with_store(move |s| s.forward_by_ref(&r))
        .await
        .unwrap()
        .unwrap()
}

fn item(tag: u8, len: usize) -> Vec<u8> {
    let mut v = vec![tag; len];
    v[0] = 0x31;
    v
}

/// A raw `tcp-lan` peer speaking as `who` to the hub `hub` at `addr`
/// (hello, its announce, then sealed frames by hand): the gate's answers.
struct Raw {
    s: TcpStream,
    who: Arc<Ident>,
    hub: PeerKeys,
}

impl Raw {
    async fn open(addr: SocketAddr, who: &Arc<Ident>, hub: &Node) -> Raw {
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(&HELLO).await.unwrap();
        s.write_all(&encode_frame(T_ANNOUNCE, &who.announce_bytes()).unwrap())
            .await
            .unwrap();
        read_hello(&mut s).await.unwrap();
        let (ty, _) = read_frame(&mut s).await.unwrap();
        assert_eq!(ty, T_ANNOUNCE, "the hub announces first");
        Raw {
            s,
            who: who.clone(),
            hub: hub.keys(),
        }
    }

    /// Put `item` for `dest` (with the hint TLV when given) and read the
    /// hub's receipt for it.
    async fn put(
        &mut self,
        dest: &NodeId,
        item: &[u8],
        hint: Option<[u8; DEST_HUB_LEN]>,
    ) -> CustodyStatus {
        let mut inner = Inner::new(
            &self.who.id(),
            Kind::CustodyPut,
            MsgSeq::CONTROL,
            encode_put(dest, item),
        );
        inner.ext.dest_hub = hint;
        let pkt = seal_envelope(&*self.who, &self.hub, Priority::Interactive, &inner).unwrap();
        self.s
            .write_all(&encode_frame(T_SEALED, &pkt).unwrap())
            .await
            .unwrap();
        let want = pkt_ref(item);
        let hub = self.hub;
        loop {
            let (ty, body) = tokio::time::timeout(T, read_frame(&mut self.s))
                .await
                .expect("an answer in time")
                .expect("a frame");
            if ty != T_SEALED {
                continue;
            }
            let Ok(o) = open_envelope(&*self.who, &body, &|_| vec![hub]) else {
                continue;
            };
            if o.inner.kind != Kind::CustodyAck {
                continue;
            }
            let ack = CustodyAck::decode(&o.inner.payload).unwrap();
            if ack.receipt.pkt_ref == want {
                return ack.receipt.status;
            }
        }
    }
}

/// Wait until `id` is `relayed`, or `sent` with a custody reason.
async fn settled(f: &Fabric, id: i64) -> mesh111::fabric::MessageView {
    until(10, || async {
        let m = f.message(id).await.ok()?;
        (m.state == MsgState::Relayed || (m.state == MsgState::Sent && m.reason.is_some()))
            .then_some(m)
    })
    .await
}

/// One flush of a fake carrier between the `carrier` pairs `x` and `y`, both
/// ways: every frame owed is taken, handed to the other side, and reported
/// `Taken`. Returns how many frames moved.
async fn carry(x: &Node, y: &Node) -> usize {
    let mut moved = 0;
    for (from, to) in [(x, y), (y, x)] {
        for fr in from.f.forward_take(to.id(), 64).await.unwrap() {
            to.f.carrier_inbound(from.id(), fr.bytes.clone())
                .await
                .unwrap();
            if let Some(seq) = fr.custody_seq {
                from.f.forward_done(seq, ForwardDone::Taken).await.unwrap();
            }
            moved += 1;
        }
    }
    moved
}

use CustodyStatus::{Accepted, RejectUnenrolled, RejectUnknownDest};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_gate_table_with_the_carrier_bound() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let m = member(&d, "mary", &a, true).await;
    let x = NodeId::from([0x77; 32]); // nobody's member
    let elsewhere = Some([0xEE; DEST_HUB_LEN]); // a hint naming no pair
    let mut raw = Raw::open(a.addr, &m.ident, &a).await;

    // No pairing: today's gate and today's answer, no forward row.
    assert_eq!(raw.put(&x, &item(1, 300), None).await, RejectUnknownDest);
    assert_eq!(
        raw.put(&x, &item(2, 300), elsewhere).await,
        RejectUnknownDest
    );

    // A `tcp` peer.
    let b = hub(&d, "hub-b").await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    assert_eq!(
        raw.put(&x, &item(3, 300), None).await,
        Accepted,
        "no hint and a target exists: held, probed after the ack (C2)"
    );
    assert_eq!(
        raw.put(&x, &item(4, 300), Some(prefix(&b.id()))).await,
        Accepted
    );
    assert_eq!(
        raw.put(&x, &item(5, 300), Some(prefix(&a.id()))).await,
        RejectUnknownDest,
        "a stale hint naming this hub (C18)"
    );
    assert_eq!(
        raw.put(&x, &item(6, 300), elsewhere).await,
        RejectUnknownDest,
        "a hint naming no pair, with no gateway or carrier pair"
    );
    assert_eq!(
        raw.put(&m.id(), &item(7, 300), elsewhere).await,
        Accepted,
        "a member dest is today's row whatever the hint"
    );
    let r3 = row(&a.f, &item(3, 300)).await.expect("a forward row");
    assert_eq!(r3.fwd, Fwd::FromMember(m.id().to_bytes()));
    assert!(row(&a.f, &item(7, 300)).await.is_none(), "a member row");

    // A `carrier` peer: no target while the bound is unset.
    let c = hub(&d, "hub-c").await;
    pair(&a, &c, PairRole::Peer, PairVia::Carrier).await;
    assert_eq!(
        raw.put(&x, &item(8, 2_937), elsewhere).await,
        RejectUnknownDest
    );
    a.f.set_carrier_max_frame(Some(3_068)).await.unwrap();
    assert_eq!(
        raw.put(&x, &item(9, 2_937), elsewhere).await,
        Accepted,
        "2,937 B fits the 3,068 B bound"
    );
    let too_large = a.f.status().await.unwrap().counters.forward_too_large;
    assert_eq!(
        raw.put(&x, &item(10, 2_938), elsewhere).await,
        RejectUnknownDest,
        "2,938 B does not (C15)"
    );
    assert_eq!(
        a.f.status().await.unwrap().counters.forward_too_large,
        too_large + 1
    );

    // A `gateway` upstream takes a hint naming no pair.
    let g = hub(&d, "hub-g").await;
    pair(&a, &g, PairRole::Gateway, PairVia::Tcp).await;
    assert_eq!(raw.put(&x, &item(11, 2_938), elsewhere).await, Accepted);

    // Puts from pairs.
    let mut from_b = Raw::open(a.addr, &b.ident, &a).await;
    assert_eq!(
        from_b.put(&x, &item(12, 300), None).await,
        RejectUnknownDest,
        "a tcp peer: today's gate"
    );
    assert_eq!(from_b.put(&m.id(), &item(13, 300), None).await, Accepted);
    assert!(
        row(&a.f, &item(13, 300)).await.is_none(),
        "for a member: a member row"
    );
    let mut from_g = Raw::open(a.addr, &g.ident, &a).await;
    assert_eq!(
        from_g.put(&x, &item(14, 300), None).await,
        RejectUnknownDest,
        "this hub's gateway: today's gate"
    );
    let mut from_c = Raw::open(a.addr, &c.ident, &a).await;
    assert_eq!(
        from_c.put(&x, &item(15, 300), None).await,
        RejectUnknownDest,
        "a carrier pair's put over tcp-lan is against its record"
    );
    let l = hub(&d, "hub-l").await;
    pair(&a, &l, PairRole::Leaf, PairVia::Tcp).await;
    let mut from_l = Raw::open(a.addr, &l.ident, &a).await;
    assert_eq!(
        from_l.put(&x, &item(16, 300), None).await,
        Accepted,
        "a leaf's put rides the carrier"
    );
    assert_eq!(
        row(&a.f, &item(16, 300)).await.expect("held").fwd,
        Fwd::FromLeaf(l.id().to_bytes())
    );
    assert_eq!(
        from_l.put(&x, &item(17, 2_938), None).await,
        RejectUnknownDest,
        "a leaf's put that fits no carrier"
    );
    let st = a.f.status().await.unwrap();
    let roles: Vec<(PairRole, PairVia)> =
        st.paired.iter().map(|p| (p.hub.role, p.hub.via)).collect();
    assert_eq!(
        roles,
        vec![
            (PairRole::Peer, PairVia::Tcp),
            (PairRole::Peer, PairVia::Carrier),
            (PairRole::Gateway, PairVia::Tcp),
            (PairRole::Leaf, PairVia::Tcp),
        ]
    );
    assert_eq!(st.settings.paired_hubs.len(), 4);
    assert_eq!(st.hub.members, 1, "pairs are not members (am. 10)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hinted_message_crosses_to_the_other_island_and_its_ack_comes_back() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    let m = member(&d, "mary", &a, true).await;
    let dd = member(&d, "dave", &b, true).await;
    exchange_cards(&m.f, &dd.f).await;
    // Mary knows hub B (its card): her hint names a hub she knows.
    let bcard = b.f.card().await.unwrap().text;
    m.f.add_contact_card(&bcard).await.unwrap();
    // B is away: A holds the row protected.
    b.t.release(a.addr);
    until(10, || async {
        (!pair_view(&a.f, &b).await.reachable).then_some(())
    })
    .await;
    let id = m.f.send(dd.id(), "across", None).await.unwrap();
    let rel = until_state(&m.f, id, MsgState::Relayed, 10).await;
    assert_eq!(
        rel.reason,
        Some(Reason::Forwarding),
        "§5.6: held for forwarding"
    );
    let sent = puts(&m, &a);
    let (dest, it, hint) = sent.last().cloned().expect("mary's put");
    assert_eq!(dest, dd.id());
    assert_eq!(hint, Some(prefix(&b.id())), "TLV 0x02 names B (§5.4)");
    let held = row(&a.f, &it).await.expect("a forward row");
    assert_eq!(held.fwd, Fwd::FromMember(m.id().to_bytes()));
    assert!(!held.committed, "protected while B is down");
    assert_eq!(pair_view(&a.f, &b).await.pending, 1);
    // B comes back: A forwards with the hint, B accepts, A commits.
    b.t.maintain(a.addr, Some(a.id()));
    until_state(&m.f, id, MsgState::Delivered, 20).await;
    assert!(
        row(&a.f, &it).await.expect("kept").committed,
        "committed on B's ACCEPTED"
    );
    let fwd = puts(&a, &b);
    assert!(
        fwd.iter()
            .any(|(d2, i2, h2)| *d2 == dd.id() && *i2 == it && *h2 == Some(prefix(&b.id()))),
        "A repeats the hint it received"
    );
    let got = inbox(&dd.f, m.id()).await;
    assert_eq!(got.len(), 1, "D receives once");
    assert_eq!(got[0].text.as_deref(), Some("across"));
    let st = a.f.status().await.unwrap();
    assert_eq!(st.counters.forward_refused, 0);
    assert!(pair_view(&a.f, &b).await.reachable);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_hint_probes_in_paired_hubs_order_then_the_cache_answers() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b1 = hub(&d, "hub-b1").await;
    let b2 = hub(&d, "hub-b2").await;
    pair(&a, &b1, PairRole::Peer, PairVia::Tcp).await;
    pair(&a, &b2, PairRole::Peer, PairVia::Tcp).await;
    let m = member(&d, "mary", &a, false).await;
    let dd = member(&d, "dave", &b2, false).await;
    exchange_cards(&m.f, &dd.f).await;
    let id1 = m.f.send(dd.id(), "first", None).await.unwrap();
    let rel = until_state(&m.f, id1, MsgState::Relayed, 10).await;
    assert_eq!(rel.reason, None, "no hint: plain relayed (§5.6)");
    until_state(&m.f, id1, MsgState::Delivered, 20).await;
    assert_eq!(
        puts(&a, &b1).len(),
        1,
        "B1 first, in paired_hubs order; it refused"
    );
    assert_eq!(puts(&a, &b2).len(), 1, "then B2, which accepted");
    let id2 = m.f.send(dd.id(), "second", None).await.unwrap();
    until_state(&m.f, id2, MsgState::Delivered, 20).await;
    assert_eq!(
        puts(&a, &b1).len(),
        1,
        "a cache hit: no probe for the second"
    );
    assert_eq!(puts(&a, &b2).len(), 2, "one put");
    assert_eq!(inbox(&dd.f, m.id()).await.len(), 2);
    assert_eq!(a.f.status().await.unwrap().counters.forward_refused, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_target_refusing_marks_the_row_refused_and_a_down_pair_keeps_it() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    let m = member(&d, "mary", &a, false).await;
    let x = stranger(&d, "xena");
    exchange_cards(&m.f, &x.f).await;
    b.t.release(a.addr);
    until(10, || async {
        (!pair_view(&a.f, &b).await.reachable).then_some(())
    })
    .await;
    let id = m.f.send(x.id(), "to nobody's member", None).await.unwrap();
    until_state(&m.f, id, MsgState::Relayed, 10).await;
    let (_, it, _) = puts(&m, &a).last().cloned().expect("mary's put");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let held = row(&a.f, &it).await.expect("held");
    assert!(
        !held.committed,
        "a target that is down keeps the row protected"
    );
    assert_eq!(a.f.status().await.unwrap().counters.forward_refused, 0);
    // B returns and refuses: every target refused.
    b.t.maintain(a.addr, Some(a.id()));
    until(10, || async {
        let s = a.f.status().await.ok()?;
        (s.counters.forward_refused == 1).then_some(())
    })
    .await;
    assert!(
        row(&a.f, &it).await.expect("kept").committed,
        "the refused mark"
    );
    assert_eq!(pair_view(&a.f, &b).await.pending, 0);
    assert_eq!(
        m.f.message(id).await.unwrap().state,
        MsgState::Relayed,
        "the member keeps relayed (§5.6, the honest limit)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_origin_over_its_share_gets_reject_full_while_another_is_admitted() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    b.t.release(a.addr); // rows stay protected
    let m1 = member(&d, "mary", &a, false).await;
    let m2 = member(&d, "mick", &a, false).await;
    let x = stranger(&d, "xena");
    exchange_cards(&m1.f, &x.f).await;
    exchange_cards(&m2.f, &x.f).await;
    exchange_cards(&m1.f, &m2.f).await;
    // A test-sized Transit quota: each origin's share is 1,000 B.
    a.f.patch_settings(SettingsPatch {
        quotas: vec![(Class::Transit, Some(8_000))],
        ..SettingsPatch::default()
    })
    .await
    .unwrap();
    let mut full = None;
    for i in 0..12 {
        let id = m1.f.send(x.id(), &format!("item {i}"), None).await.unwrap();
        let v = settled(&m1.f, id).await;
        if v.state == MsgState::Sent {
            assert_eq!(v.reason, Some(Reason::MailboxFull));
            full = Some(i);
            break;
        }
    }
    assert!(
        full.is_some_and(|i| i >= 1),
        "M is refused once over its share: {full:?}"
    );
    let id = m2.f.send(x.id(), "another origin", None).await.unwrap();
    assert_eq!(
        settled(&m2.f, id).await.state,
        MsgState::Relayed,
        "M2 is admitted"
    );
    // Over the whole Transit quota with only protected rows: forward puts
    // REJECT_FULL, a member's put for a member is still admitted.
    a.f.patch_settings(SettingsPatch {
        quotas: vec![(Class::Transit, Some(1))],
        ..SettingsPatch::default()
    })
    .await
    .unwrap();
    let id = m2.f.send(x.id(), "over transit", None).await.unwrap();
    let v = settled(&m2.f, id).await;
    assert_eq!(
        (v.state, v.reason),
        (MsgState::Sent, Some(Reason::MailboxFull))
    );
    m1.t.release(a.addr);
    let id = m2.f.send(m1.id(), "to a member", None).await.unwrap();
    assert_eq!(settled(&m2.f, id).await.state, MsgState::Relayed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reject_full_keeps_the_row_for_its_pair_and_retries_on_events_only() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    let m = member(&d, "mary", &a, false).await;
    let dd = member(&d, "dave", &b, true).await;
    exchange_cards(&m.f, &dd.f).await;
    let tight = |q| SettingsPatch {
        quotas: vec![(Class::Custody, q)],
        ..SettingsPatch::default()
    };
    b.f.patch_settings(tight(Some(1))).await.unwrap();
    let id1 = m.f.send(dd.id(), "one", None).await.unwrap();
    until_state(&m.f, id1, MsgState::Relayed, 10).await;
    let (_, it1, _) = puts(&m, &a).last().cloned().unwrap();
    until(10, || async { (!puts(&a, &b).is_empty()).then_some(()) }).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !row(&a.f, &it1).await.unwrap().committed,
        "REJECT_FULL keeps it"
    );
    assert_eq!(pair_view(&a.f, &b).await.pending, 1, "…for that pair");
    assert_eq!(a.f.status().await.unwrap().counters.forward_refused, 0);
    // Room again at B: nothing happens without an event (no timer).
    b.f.patch_settings(tight(None)).await.unwrap();
    let before = puts(&a, &b).len();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(puts(&a, &b).len(), before, "never on a timer");
    assert!(inbox(&dd.f, m.id()).await.is_empty());
    // The pair's next link-up retries it.
    b.t.release(a.addr);
    until(10, || async {
        (!pair_view(&a.f, &b).await.reachable).then_some(())
    })
    .await;
    b.t.maintain(a.addr, Some(a.id()));
    until_state(&m.f, id1, MsgState::Delivered, 20).await;
    // Full again; a row inserted for the pair retries the first.
    b.f.patch_settings(tight(Some(1))).await.unwrap();
    let n = puts(&a, &b).len();
    let id2 = m.f.send(dd.id(), "two", None).await.unwrap();
    until_state(&m.f, id2, MsgState::Relayed, 10).await;
    until(10, || async { (puts(&a, &b).len() > n).then_some(()) }).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    b.f.patch_settings(tight(None)).await.unwrap();
    let id3 = m.f.send(dd.id(), "three", None).await.unwrap();
    until_state(&m.f, id2, MsgState::Delivered, 20).await;
    until_state(&m.f, id3, MsgState::Delivered, 20).await;
    assert_eq!(inbox(&dd.f, m.id()).await.len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reject_unenrolled_breaks_the_pair_and_returns_the_row() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b1 = hub(&d, "hub-b1").await;
    let b2 = hub(&d, "hub-b2").await;
    pair(&a, &b1, PairRole::Peer, PairVia::Tcp).await;
    pair(&a, &b2, PairRole::Peer, PairVia::Tcp).await;
    let m = member(&d, "mary", &a, false).await;
    let dd = member(&d, "dave", &b2, false).await;
    exchange_cards(&m.f, &dd.f).await;
    // B1 drops A (DELETE /relay/paired): A is no longer enrolled there.
    assert!(b1.f.unpair(a.id()).await.unwrap());
    assert!(b1.f.status().await.unwrap().paired.is_empty());
    // B1's dial went with the pair; A dials B1, so its put reaches it.
    a.t.maintain(b1.addr, Some(b1.id()));
    let id = m.f.send(dd.id(), "around B1", None).await.unwrap();
    until_state(&m.f, id, MsgState::Delivered, 20).await;
    let v = pair_view(&a.f, &b1).await;
    assert!(v.broken, "the other hub no longer lists this hub");
    assert!(
        !puts(&a, &b1).is_empty(),
        "B1 was asked and answered REJECT_UNENROLLED"
    );
    assert_eq!(inbox(&dd.f, m.id()).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_half_pair_forwards_nothing() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let h = hub(&d, "hub-h").await;
    // H enrolls at A as a pairing; A never enrolls at H.
    let oa = a.f.offer(Some(a.addr)).await.unwrap();
    tokio::time::timeout(
        T,
        h.f.pair(
            Some(a.addr),
            &oa.code,
            &oa.card,
            PairRole::Peer,
            PairVia::Tcp,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let v = pair_view(&h.f, &a).await;
    assert!(!v.whole, "half: A did not enroll here");
    let mh = member(&d, "mona", &h, false).await;
    let x = NodeId::from([0x77; 32]);
    let mut raw = Raw::open(h.addr, &mh.ident, &h).await;
    assert_eq!(
        raw.put(&x, &item(1, 300), None).await,
        RejectUnknownDest,
        "no whole pair: no target"
    );
    let mut from_a = Raw::open(h.addr, &a.ident, &h).await;
    assert_eq!(
        from_a.put(&x, &item(2, 300), None).await,
        RejectUnenrolled,
        "the half pair's own puts: not enrolled here"
    );
    assert!(puts(&h, &a).is_empty(), "H forwards nothing to A");
    assert!(a.f.status().await.unwrap().paired.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hubs_own_messages_never_enter_its_forward_table() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    let dd = member(&d, "dave", &b, true).await;
    exchange_cards(&a.f, &dd.f).await;
    dd.t.release(b.addr);
    let id =
        a.f.send(dd.id(), "from the hub itself", None)
            .await
            .unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    let v = a.f.message(id).await.unwrap();
    assert_eq!(v.state, MsgState::Queued, "no home hub: queued (D31 (3))");
    assert!(puts(&a, &b).is_empty(), "no Route::Forward");
    let rows =
        a.f.with_store(|s| s.forward_pending(0, 10))
            .await
            .unwrap()
            .unwrap();
    assert!(rows.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn protected_rows_resume_after_a_restart() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    let m = member(&d, "mary", &a, false).await;
    let dd = member(&d, "dave", &b, false).await;
    exchange_cards(&m.f, &dd.f).await;
    b.t.release(a.addr);
    until(10, || async {
        (!pair_view(&a.f, &b).await.reachable).then_some(())
    })
    .await;
    let id =
        m.f.send(dd.id(), "kept across a restart", None)
            .await
            .unwrap();
    until_state(&m.f, id, MsgState::Relayed, 10).await;
    // A dies (no clean close) and comes back on its port with its pairs.
    let settings = a.f.settings().await.unwrap();
    assert_eq!(settings.paired_hubs.len(), 1);
    a.f.abort();
    a.t.shutdown();
    let a2 = start(&a.ident, &a.dir, settings);
    let t2 = tcp(&a2, &a.ident, "tcp");
    add_tcp(&a2, &t2, true).await;
    listen(&t2, a.addr.port()).await;
    b.t.maintain(a.addr, Some(a.id()));
    until_state(&m.f, id, MsgState::Delivered, 25).await;
    assert_eq!(inbox(&dd.f, m.id()).await.len(), 1, "D receives once");
    t2.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_carrier_hand_off_posts_once_reposts_on_cue_and_consumes_acks() {
    let d = Dirs::new();
    let g = hub(&d, "hub-g").await;
    let g2 = hub(&d, "hub-g2").await;
    let l = hub(&d, "hub-l").await;
    pair(&g, &l, PairRole::Leaf, PairVia::Tcp).await;
    pair(&g, &g2, PairRole::Peer, PairVia::Carrier).await;
    // They met once over tcp-lan; afterwards only the carrier.
    g2.t.release(g.addr);
    for h in [&g, &g2] {
        h.f.set_carrier_max_frame(Some(3_068)).await.unwrap();
    }
    let ml = member(&d, "lena", &l, true).await;
    let m2 = member(&d, "otto", &g2, true).await;
    exchange_cards(&ml.f, &m2.f).await;
    let mut gev = g.f.subscribe();
    let id = ml.f.send(m2.id(), "over the carrier", None).await.unwrap();
    until_state(&ml.f, id, MsgState::Relayed, 10).await;
    // G holds it for G′ (fwd = 2, from its leaf): "forward pending".
    let ev = until(10, || {
        let e = drain_events(&mut gev);
        async move {
            e.into_iter().find_map(|e| match e {
                FabricEvent::Forward { target, pending } => Some((target, pending)),
                _ => None,
            })
        }
    })
    .await;
    assert_eq!(ev, (g2.id(), 1));
    let taken = g.f.forward_take(g2.id(), 8).await.unwrap();
    assert_eq!(taken.len(), 1);
    let seq = taken[0].custody_seq.expect("a put");
    let held =
        g.f.with_store(move |s| s.forward_item(seq))
            .await
            .unwrap()
            .unwrap()
            .expect("held");
    assert_eq!(held.fwd, Fwd::FromLeaf(l.id().to_bytes()));
    assert!(
        g.f.forward_take(g2.id(), 8).await.unwrap().is_empty(),
        "taken, not owed"
    );
    // Retry: owed again, not a re-post.
    g.f.forward_done(seq, ForwardDone::Retry).await.unwrap();
    let again = g.f.forward_take(g2.id(), 8).await.unwrap();
    assert_eq!(again.len(), 1);
    g.f.forward_done(seq, ForwardDone::Taken).await.unwrap();
    // Posted while G′ is silent: never posted again on a flush.
    assert!(g.f.forward_take(g2.id(), 8).await.unwrap().is_empty());
    assert_eq!(pair_view(&g.f, &g2).await.reposts, 0);
    // "Flush now" (or a since miss) re-arms it: one re-post, counted.
    assert_eq!(g.f.forward_repost(g2.id()).await.unwrap(), 1);
    let re = g.f.forward_take(g2.id(), 8).await.unwrap();
    assert_eq!(re.len(), 1);
    g.f.forward_done(seq, ForwardDone::Taken).await.unwrap();
    assert_eq!(pair_view(&g.f, &g2).await.reposts, 1);
    // G′ takes it (both copies: the second is a DUPLICATE).
    g2.f.carrier_inbound(g.id(), again[0].bytes.clone())
        .await
        .unwrap();
    g2.f.carrier_inbound(g.id(), re[0].bytes.clone())
        .await
        .unwrap();
    let acks = g2.f.forward_take(g.id(), 8).await.unwrap();
    assert_eq!(acks.len(), 2, "one CUSTODY_ACK per put");
    assert!(acks.iter().all(|a| a.custody_seq.is_none()));
    assert!(
        g2.f.forward_take(g.id(), 8).await.unwrap().is_empty(),
        "acks are consumed on take, never returned again (C7)"
    );
    assert!(
        !row(&g.f, &held.bytes).await.unwrap().committed,
        "not on the carrier's accept (C5)"
    );
    g.f.carrier_inbound(g2.id(), acks[0].bytes.clone())
        .await
        .unwrap();
    assert!(
        row(&g.f, &held.bytes).await.unwrap().committed,
        "on the far hub's CUSTODY_ACK"
    );
    // An already-processed frame is stale; a stranger's frame is dropped;
    // only a paired hub may hand frames in.
    let st0 = g.f.status().await.unwrap().counters;
    g.f.carrier_inbound(g2.id(), acks[0].bytes.clone())
        .await
        .unwrap();
    g.f.carrier_inbound(g2.id(), taken[0].bytes.clone())
        .await
        .unwrap();
    let st1 = g.f.status().await.unwrap().counters;
    assert_eq!(st1.stale, st0.stale + 1);
    assert_eq!(st1.dropped, st0.dropped + 1, "sealed to G′, not to G");
    assert!(g
        .f
        .carrier_inbound(m2.id(), acks[1].bytes.clone())
        .await
        .is_err());
    assert!(
        g.f.forward_take(l.id(), 8).await.is_err(),
        "a tcp pair has no carrier frames"
    );
    assert!(
        pair_view(&g.f, &g2).await.reachable,
        "heard through the carrier"
    );
    // Otto receives it once; his ACK goes back G′ → carrier → G → L → Lena.
    until(30, || async {
        carry(&g, &g2).await;
        let v = ml.f.message(id).await.ok()?;
        (v.state == MsgState::Delivered).then_some(())
    })
    .await;
    assert_eq!(inbox(&m2.f, ml.id()).await.len(), 1);
    // A carrier's permanent refusal with no other target: refused, counted.
    let id2 =
        ml.f.send(m2.id(), "refused by the carrier", None)
            .await
            .unwrap();
    until_state(&ml.f, id2, MsgState::Relayed, 10).await;
    let fr = until(10, || async {
        let v = g.f.forward_take(g2.id(), 8).await.ok()?;
        v.into_iter().find(|f| f.custody_seq.is_some())
    })
    .await;
    let refused = g.f.status().await.unwrap().counters.forward_refused;
    g.f.forward_done(
        fr.custody_seq.unwrap(),
        ForwardDone::Rejected { why: "413" },
    )
    .await
    .unwrap();
    until(10, || async {
        let s = g.f.status().await.ok()?;
        (s.counters.forward_refused == refused + 1).then_some(())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn leaf_to_leaf_across_the_carrier_and_back() {
    let d = Dirs::new();
    let g = hub(&d, "hub-g").await;
    let g2 = hub(&d, "hub-g2").await;
    let l = hub(&d, "hub-l").await;
    let l2 = hub(&d, "hub-l2").await;
    pair(&g, &l, PairRole::Leaf, PairVia::Tcp).await;
    pair(&g2, &l2, PairRole::Leaf, PairVia::Tcp).await;
    pair(&g, &g2, PairRole::Peer, PairVia::Carrier).await;
    g2.t.release(g.addr);
    for h in [&g, &g2] {
        h.f.set_carrier_max_frame(Some(3_068)).await.unwrap();
    }
    let ml = member(&d, "lena", &l, true).await;
    let ml2 = member(&d, "lars", &l2, true).await;
    exchange_cards(&ml.f, &ml2.f).await;
    let id = ml.f.send(ml2.id(), "the longest path", None).await.unwrap();
    // leaf → gateway → carrier → gateway′ → leaf′, and the ACK back.
    until(30, || async {
        carry(&g, &g2).await;
        let v = ml.f.message(id).await.ok()?;
        (v.state == MsgState::Delivered).then_some(())
    })
    .await;
    let got = inbox(&ml2.f, ml.id()).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].text.as_deref(), Some("the longest path"));
    // Each hop held it in its own table row (§5.1).
    let (_, it, hint) = puts(&ml, &l).last().cloned().unwrap();
    assert_eq!(hint, Some(prefix(&l2.id())));
    let at_l = row(&l.f, &it).await.expect("L: from its member");
    assert_eq!(at_l.fwd, Fwd::FromMember(ml.id().to_bytes()));
    let at_g = row(&g.f, &it).await.expect("G: from its leaf");
    assert_eq!(at_g.fwd, Fwd::FromLeaf(l.id().to_bytes()));
    let at_g2 = row(&g2.f, &it).await.expect("G′: from the carrier");
    assert_eq!(at_g2.fwd, Fwd::FromCarrier(g.id().to_bytes()));
    assert!(at_l.committed && at_g.committed && at_g2.committed);
    assert!(
        row(&l2.f, &it).await.is_none(),
        "L′: a member row, the final put"
    );
    for h in [&g, &g2, &l, &l2] {
        assert_eq!(h.f.status().await.unwrap().counters.forward_refused, 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_no_pairing_frames_and_answers_are_layer_5s() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let m = member(&d, "mary", &a, false).await;
    let x = stranger(&d, "xena");
    exchange_cards(&m.f, &x.f).await;
    let id = m.f.send(x.id(), "no pairing", None).await.unwrap();
    let v = settled(&m.f, id).await;
    assert_eq!(
        (v.state, v.reason),
        (MsgState::Sent, Some(Reason::HubNotYours))
    );
    // The put carries no TLV: no `HAS_EXT`, Layer 5's bytes.
    let k = m.keys();
    let pkt = m.ident.sealed_to(&a.ident.enc());
    let opened: Vec<_> = pkt
        .iter()
        .filter_map(|p| open_envelope(&*a.ident, p, &|_| vec![k]).ok())
        .filter(|o| o.inner.kind == Kind::CustodyPut)
        .collect();
    assert!(!opened.is_empty());
    assert!(opened.iter().all(|o| o.inner.ext.is_empty()));
    let st = a.f.status().await.unwrap();
    assert!(st.settings.paired_hubs.is_empty() && st.paired.is_empty());
    assert_eq!(
        (st.counters.forward_refused, st.counters.forward_too_large),
        (0, 0)
    );
    assert!(a
        .f
        .with_store(|s| s.forward_pending(0, 10))
        .await
        .unwrap()
        .unwrap()
        .is_empty());
    // A hint naming another hub rides the put (§5.4) and changes nothing
    // at an unpaired hub: the same answer.
    let y_ident = Ident::with_appdata("yuri", {
        let mut app = appdata(0);
        app.extend_from_slice(&[0xEE; DEST_HUB_LEN]);
        app
    });
    let y = start(&y_ident, &d.node("yuri"), FabricSettings::default());
    exchange_cards(&m.f, &y).await;
    let id = m.f.send(y_ident.id(), "hinted", None).await.unwrap();
    let v = settled(&m.f, id).await;
    assert_eq!(
        (v.state, v.reason),
        (MsgState::Sent, Some(Reason::HubNotYours))
    );
    let last = puts(&m, &a).last().cloned().unwrap();
    assert_eq!(last.2, Some([0xEE; DEST_HUB_LEN]));
    assert_eq!(a.f.status().await.unwrap().counters.forward_too_large, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unpairing_returns_rows_and_the_far_hub_is_no_member_after() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    let m = member(&d, "mary", &a, false).await;
    let x = stranger(&d, "xena");
    exchange_cards(&m.f, &x.f).await;
    b.t.release(a.addr);
    until(10, || async {
        (!pair_view(&a.f, &b).await.reachable).then_some(())
    })
    .await;
    let id = m.f.send(x.id(), "waiting for B", None).await.unwrap();
    until_state(&m.f, id, MsgState::Relayed, 10).await;
    let (_, it, _) = puts(&m, &a).last().cloned().unwrap();
    let mut sev = a.f.subscribe();
    assert!(a.f.unpair(b.id()).await.unwrap());
    // No other target: refused.
    until(10, || async {
        let r = row(&a.f, &it).await?;
        r.committed.then_some(())
    })
    .await;
    let st = a.f.status().await.unwrap();
    assert!(st.settings.paired_hubs.is_empty());
    assert!(
        drain_events(&mut sev)
            .iter()
            .any(|e| matches!(e, FabricEvent::Settings(s) if s.paired_hubs.is_empty())),
        "persisted through FabricEvent::Settings"
    );
    let c = a.f.contact(b.id()).await.unwrap();
    assert_eq!(
        c.trust,
        mesh111::store::Trust::Oob,
        "never a member here afterwards"
    );
    assert!(!a.f.unpair(b.id()).await.unwrap());
}
