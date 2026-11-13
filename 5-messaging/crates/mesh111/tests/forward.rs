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
//! (`tests/hub_flow.rs` stays unchanged and green). The M4b fixes: wholeness
//! recorded (a former member pairs, a member code records no pair), no
//! pairing dial kept once whole, the hub role switch, re-post rule (1)'s
//! bound, a replayed or misdirected pair ack, the resident window and
//! the forwarder's linear work. The M4c fixes: the window shared fairly (a
//! down pair's backlog never holds back an up pair's rows), `Forward` on a
//! far frame with a post outstanding, rule (1) once per far-hub round. The
//! M4d fixes: the swap probe reads a deep store on its own (the self-wake),
//! an unpairing restarts it, a full pair counts as down for the window. The
//! M4e fixes: a hit-cache entry restarts the probe only for a dest whose
//! rows wait in the store (commits for many dests never sweep it again, a
//! dest's first hit moves its stored rows first).

mod common;

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use id111::keyset::NodeId;
use mesh111::contacts::{ROLE_ALWAYS_ON_HUB, ROLE_MAILBOX};
use mesh111::custody::CustodyReceipt;
use mesh111::custody::{pkt_ref, CustodyAck, CustodyStatus};
use mesh111::envelope::{
    open_envelope, seal_envelope, Inner, Kind, PeerKeys, Priority, DEST_HUB_LEN,
};
use mesh111::fabric::{
    Fabric, FabricError, FabricEvent, FabricSettings, ForwardDone, PairView, SettingsPatch,
    FORWARD_CACHE, FORWARD_RESIDENT,
};
use mesh111::ids::MsgSeq;
use mesh111::link::tcp_lan::{
    encode_frame, read_frame, read_hello, TcpLan, HELLO, T_ANNOUNCE, T_SEALED,
};
use mesh111::link::{Dialer, Link, PeerAddr};
use mesh111::mailbox::{decode_put, encode_cursor, encode_put, EnrollStatus};
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

/// Pair `a` and `b` (§4.6 steps 1–4) without the daemon: `a` shows a
/// pairing code and `b` enrolls at `a` on its own dial (`b` records `ours`,
/// `a` records `theirs`); `b` shows one and `a` enrolls at `b` over that
/// same connection (the second `ENROLL` rides it, S22): whole on both
/// sides. `role` is `b`'s role from `a`'s side. `hold`: `b` holds `a`'s
/// address and `a` none for it, so the daemon's §4.6 step 5 rule has `b`
/// dial (`dial_peer`, as the daemon does); without it the fabric keeps no
/// pairing dial once the pair is whole (M4b).
async fn pair_with(a: &Node, b: &Node, role: PairRole, via: PairVia, hold: bool) {
    let oa =
        a.f.offer_pair(Some(a.addr), role, via)
            .await
            .expect("offer a");
    assert!(oa.bundle.is_none(), "the 111PAIR: bundle is the daemon's");
    let rec = tokio::time::timeout(
        T,
        b.f.pair(Some(a.addr), &oa.code, &oa.card, role.complement(), via),
    )
    .await
    .expect("b in time")
    .expect("b paired");
    assert_eq!(
        (rec.node_id, rec.role, rec.via, rec.ours, rec.theirs),
        (a.id(), role.complement(), via, true, false),
        "half: b's ENROLL_OK only"
    );
    assert!(!pair_view(&b.f, a).await.whole, "a half pair");
    if hold {
        b.f.dial_peer(a.addr, a.id())
            .await
            .expect("b holds the dial");
    }
    let ob =
        b.f.offer_pair(None, role.complement(), via)
            .await
            .expect("offer b");
    let rec = tokio::time::timeout(T, a.f.pair(None, &ob.code, &ob.card, role, via))
        .await
        .expect("a in time")
        .expect("a paired over b's connection");
    assert!(rec.whole(), "b redeemed a's code, then a's ENROLL_OK");
    for (x, y) in [(a, b), (b, a)] {
        let y = y.id();
        until(10, || async {
            let s = x.f.status().await.ok()?;
            s.paired
                .iter()
                .any(|p| p.hub.node_id == y && p.whole && p.hub.ours && p.hub.theirs)
                .then_some(())
        })
        .await;
        // A pairing sets no home hub and writes no cursor (am. 10).
        let st = x.f.status().await.unwrap();
        assert!(st.settings.home_hub.is_none() && st.hub.cursor == 0);
    }
    if hold {
        for (x, y) in [(a, b), (b, a)] {
            until(10, || async {
                pair_view(&x.f, y).await.reachable.then_some(())
            })
            .await;
        }
    } else {
        // Whole: the pairing dial went with the connection it held.
        let far = PeerAddr::Tcp(a.addr);
        until(10, || async { (!b.t.peers().contains(&far)).then_some(()) }).await;
    }
}

/// [`pair_with`] as the daemon would run it: a `tcp` pair keeps the one
/// connection `b` opened, a `carrier` pair none.
async fn pair(a: &Node, b: &Node, role: PairRole, via: PairVia) {
    pair_with(a, b, role, via, via == PairVia::Tcp).await;
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

/// The `i`-th of many distinct items.
fn numbered(i: u32) -> Vec<u8> {
    let mut v = vec![0x5a; 200];
    v[0] = 0x31;
    v[1..5].copy_from_slice(&i.to_le_bytes());
    v
}

/// `(count, bytes)` of protected forward rows at `f` (the totals).
async fn protected(f: &Fabric) -> u64 {
    f.with_store(|s| s.forward_stats())
        .await
        .unwrap()
        .unwrap()
        .0
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

    /// Seal `payload` as `kind` from `who` to the hub and write it;
    /// returns the packet (for a replay).
    async fn send(&mut self, kind: Kind, payload: Vec<u8>) -> Vec<u8> {
        let inner = Inner::new(&self.who.id(), kind, MsgSeq::CONTROL, payload);
        let pkt = seal_envelope(&*self.who, &self.hub, Priority::Control, &inner).unwrap();
        self.write(&pkt).await;
        pkt
    }

    /// Write one sealed packet as it is.
    async fn write(&mut self, pkt: &[u8]) {
        self.s
            .write_all(&encode_frame(T_SEALED, pkt).unwrap())
            .await
            .unwrap();
    }

    /// The next sealed packet of `kind` from the hub, within `wait`.
    async fn next(&mut self, kind: Kind, wait: Duration) -> Option<Vec<u8>> {
        let hub = self.hub;
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let (ty, body) = tokio::time::timeout_at(deadline, read_frame(&mut self.s))
                .await
                .ok()?
                .ok()?;
            if ty != T_SEALED {
                continue;
            }
            let Ok(o) = open_envelope(&*self.who, &body, &|_| vec![hub]) else {
                continue;
            };
            if o.inner.kind == kind {
                return Some(o.inner.payload);
            }
        }
    }

    /// A signed `CUSTODY_ACK` from `who` for `item` held for `dest`.
    async fn ack(&mut self, dest: &NodeId, item: &[u8], status: CustodyStatus) -> Vec<u8> {
        let receipt = CustodyReceipt {
            status,
            custody_seq: u64::from(status.is_held()),
            pkt_ref: pkt_ref(item),
        };
        let ack = CustodyAck::build(&*self.who, receipt, dest, false).unwrap();
        self.send(Kind::CustodyAck, ack.encode().unwrap()).await
    }

    /// Put every item for `dest` back to back while a second task reads
    /// the receipts; returns how many came back `ACCEPTED`.
    async fn blast(self, dest: &NodeId, items: &[Vec<u8>]) -> usize {
        let Raw { s, who, hub } = self;
        let (mut rd, mut wr) = s.into_split();
        let want = items.len();
        let reader_who = who.clone();
        let reader = tokio::spawn(async move {
            let mut seen = 0usize;
            let mut accepted = 0usize;
            while seen < want {
                let Ok(Ok((ty, body))) =
                    tokio::time::timeout(Duration::from_secs(60), read_frame(&mut rd)).await
                else {
                    break;
                };
                if ty != T_SEALED {
                    continue;
                }
                let Ok(o) = open_envelope(&*reader_who, &body, &|_| vec![hub]) else {
                    continue;
                };
                if o.inner.kind != Kind::CustodyAck {
                    continue;
                }
                seen += 1;
                if CustodyAck::decode(&o.inner.payload).unwrap().receipt.status == Accepted {
                    accepted += 1;
                }
            }
            accepted
        });
        for it in items {
            let inner = Inner::new(
                &who.id(),
                Kind::CustodyPut,
                MsgSeq::CONTROL,
                encode_put(dest, it),
            );
            let pkt = seal_envelope(&*who, &hub, Priority::Interactive, &inner).unwrap();
            wr.write_all(&encode_frame(T_SEALED, &pkt).unwrap())
                .await
                .unwrap();
        }
        reader.await.unwrap()
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
    // The daemon releases B1's dial with the pair (M5); A still lists B1
    // and dials it, so its put reaches it.
    b1.t.release(a.addr);
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
    // H redeems A's pairing code; A never enrolls at H: each side holds one
    // half (H `ours`, A `theirs`).
    let oa =
        a.f.offer_pair(Some(a.addr), PairRole::Peer, PairVia::Tcp)
            .await
            .unwrap();
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
    assert!(!v.whole && v.hub.ours && !v.hub.theirs, "half at H: {v:?}");
    let v = pair_view(&a.f, &h).await;
    assert!(!v.whole && !v.hub.ours && v.hub.theirs, "half at A: {v:?}");
    let c = a.f.contact(h.id()).await.unwrap();
    assert_eq!(c.trust, mesh111::store::Trust::Oob, "never a member");
    assert_eq!(a.f.status().await.unwrap().hub.members, 0);
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
        "the half pair's own puts: not admitted here"
    );
    let ma = member(&d, "milo", &a, false).await;
    let mut from_h = Raw::open(a.addr, &h.ident, &a).await;
    assert_eq!(
        from_h.put(&ma.id(), &item(3, 300), None).await,
        RejectUnenrolled,
        "nor there, even for a member dest"
    );
    // The only puts between them are the raw ones above.
    assert_eq!(puts(&h, &a).len(), 1, "H forwards nothing to A");
    assert_eq!(puts(&a, &h).len(), 1, "nor A to H");
    // A member code records no pair: J enrolls at A with one through
    // `pair`, and A takes it for a member, never a pair.
    let j = hub(&d, "hub-j").await;
    let om = a.f.offer(Some(a.addr)).await.unwrap();
    tokio::time::timeout(
        T,
        j.f.pair(
            Some(a.addr),
            &om.code,
            &om.card,
            PairRole::Peer,
            PairVia::Tcp,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let st = a.f.status().await.unwrap();
    assert_eq!(st.paired.len(), 1, "only H's half is recorded at A");
    assert_eq!(st.hub.members, 2, "J is a member here (and Milo)");
    assert!(!pair_view(&j.f, &a).await.whole);
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
    // They met once over tcp-lan; afterwards only the carrier (the
    // pairing dial went once the pair was whole, `pair_with`).
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
        g.f.carrier_inbound(l.id(), acks[1].bytes.clone())
            .await
            .is_err(),
        "and hands none in"
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

/// M4b: wholeness is recorded, never inferred from a contact's trust. A hub
/// that is another hub's member stays its member until both pairing
/// enrollments are recorded; its home hub cannot pair with it (amendment 12,
/// both sides); a former member pairs both ways and is then a pair, not a
/// member; a recorded pair presenting a member code is refused without
/// spending it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_former_member_pairs_only_by_two_recorded_enrollments() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    let m = member(&d, "mary", &a, false).await;
    // B is A's member (its home hub is A).
    let om = a.f.offer(Some(a.addr)).await.unwrap();
    tokio::time::timeout(T, b.f.enroll(a.addr, &om.code, Some(om.card.as_str())))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(a.f.status().await.unwrap().hub.members, 2);
    // B cannot pair with its home hub, and A's pairing code at B is refused
    // there: nothing is recorded on either side.
    let oa =
        a.f.offer_pair(Some(a.addr), PairRole::Peer, PairVia::Tcp)
            .await
            .unwrap();
    let r =
        b.f.pair(
            Some(a.addr),
            &oa.code,
            &oa.card,
            PairRole::Peer,
            PairVia::Tcp,
        )
        .await;
    assert!(matches!(r, Err(FabricError::Invalid(_))), "{r:?}");
    let ob =
        b.f.offer_pair(None, PairRole::Peer, PairVia::Tcp)
            .await
            .unwrap();
    let r = tokio::time::timeout(
        T,
        a.f.pair(None, &ob.code, &ob.card, PairRole::Peer, PairVia::Tcp),
    )
    .await
    .unwrap();
    assert!(
        matches!(r, Err(FabricError::Enroll(EnrollStatus::Refused))),
        "{r:?}"
    );
    for h in [&a, &b] {
        assert!(h.f.status().await.unwrap().paired.is_empty());
    }
    // Still A's member: its fetch is served, its puts admitted.
    let st = a.f.status().await.unwrap();
    assert_eq!(st.hub.members, 2, "B is still a member at A");
    let mut from_b = Raw::open(a.addr, &b.ident, &a).await;
    assert_eq!(
        from_b.put(&m.id(), &item(1, 300), None).await,
        Accepted,
        "a member's put"
    );
    // (closed, so A's ENROLL below rides B's own connection)
    let raw_at = PeerAddr::Tcp(from_b.s.local_addr().unwrap());
    drop(from_b);
    until(10, || async {
        (!a.t.peers().contains(&raw_at)).then_some(())
    })
    .await;
    // B leaves A (L5 has no un-enrol at A: B stays `enrolled` there) and the
    // two pair both ways: then B is a pair at A, not a member.
    b.f.forget_contact(a.id()).await.unwrap();
    // (its old connection gone first: an ENROLL that rode a closing one
    // would be lost, and an attempt that dialed nothing never resends it)
    let at_a = PeerAddr::Tcp(a.addr);
    until(10, || async {
        let s = b.f.status().await.ok()?;
        (!b.t.peers().contains(&at_a) && s.links.iter().all(|l| l.peers == 0)).then_some(())
    })
    .await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    let st = a.f.status().await.unwrap();
    assert_eq!(st.hub.members, 1, "a hub both member and pair is a pair");
    assert!(pair_view(&a.f, &b).await.whole);
    let mut from_b = Raw::open(a.addr, &b.ident, &a).await;
    assert_eq!(
        from_b
            .put(&NodeId::from([0x77; 32]), &item(2, 300), None)
            .await,
        RejectUnknownDest,
        "a tcp peer pair: today's gate, never a forward origin"
    );
    assert_eq!(
        from_b.put(&m.id(), &item(3, 300), None).await,
        Accepted,
        "for a member: a member row"
    );
    assert!(row(&a.f, &item(3, 300)).await.is_none());
    // B drops its half; a member code from B (still recorded at A) is
    // refused before it is checked, so the code still serves a member.
    assert!(b.f.unpair(a.id()).await.unwrap());
    let om = a.f.offer(Some(a.addr)).await.unwrap();
    let r = tokio::time::timeout(T, b.f.enroll(a.addr, &om.code, Some(om.card.as_str())))
        .await
        .unwrap();
    assert!(
        matches!(r, Err(FabricError::Enroll(EnrollStatus::Refused))),
        "{r:?}"
    );
    assert!(
        pair_view(&a.f, &b).await.hub.whole(),
        "A's record unchanged"
    );
    let c = stranger(&d, "carl");
    add_tcp(&c.f, &c.t, true).await;
    tokio::time::timeout(T, c.f.enroll(a.addr, &om.code, Some(om.card.as_str())))
        .await
        .unwrap()
        .expect("the code was not spent by the refusal");
    assert_eq!(a.f.status().await.unwrap().hub.members, 2);
}

/// M4b: without the daemon's dial, a whole `tcp` pair keeps no pairing dial
/// either (the daemon's §4.6 step 5 rule holds its one connection); a half
/// pair keeps the dial its attempt started (the far `ENROLL` rides it).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_whole_pair_keeps_no_pairing_dial() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    pair_with(&a, &b, PairRole::Peer, PairVia::Tcp, false).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !b.t.peers().contains(&PeerAddr::Tcp(a.addr)),
        "not redialed"
    );
    // A half pair: H's dial stays up while A has not enrolled there.
    let h = hub(&d, "hub-h").await;
    let oa =
        a.f.offer_pair(Some(a.addr), PairRole::Peer, PairVia::Carrier)
            .await
            .unwrap();
    tokio::time::timeout(
        T,
        h.f.pair(
            Some(a.addr),
            &oa.code,
            &oa.card,
            PairRole::Peer,
            PairVia::Carrier,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        h.t.peers().contains(&PeerAddr::Tcp(a.addr)),
        "kept while half"
    );
    // Unpairing releases it.
    assert!(h.f.unpair(a.id()).await.unwrap());
    until(10, || async {
        (!h.t.peers().contains(&PeerAddr::Tcp(a.addr))).then_some(())
    })
    .await;
}

/// M4b: with the hub role off (custody opt-in keeps member custody) the
/// records stay, nothing is forwarded or answered, the held row waits in
/// the store (never refused) and the view shows each pair not whole, not
/// reachable, nothing pending; back on, the row is forwarded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_the_hub_role_off_pairs_wait_and_nothing_is_forwarded() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    a.f.patch_settings(SettingsPatch {
        custody_opt_in: Some(true),
        ..SettingsPatch::default()
    })
    .await
    .unwrap();
    let m = member(&d, "mary", &a, false).await;
    let dd = member(&d, "dave", &b, false).await;
    exchange_cards(&m.f, &dd.f).await;
    b.t.release(a.addr);
    until(10, || async {
        (!pair_view(&a.f, &b).await.reachable).then_some(())
    })
    .await;
    let id =
        m.f.send(dd.id(), "waits with the role", None)
            .await
            .unwrap();
    until_state(&m.f, id, MsgState::Relayed, 10).await;
    assert_eq!(pair_view(&a.f, &b).await.pending, 1);
    a.f.patch_settings(SettingsPatch {
        hub_role: Some(false),
        ..SettingsPatch::default()
    })
    .await
    .unwrap();
    let v = pair_view(&a.f, &b).await;
    assert!(
        !v.whole && !v.reachable && v.pending == 0,
        "the view with the role off: {v:?}"
    );
    assert!(v.hub.whole(), "the record is kept");
    assert_eq!(protected(&a.f).await, 1, "the row waits in the store");
    // No pair answers: B's fetch gets no item-less answer from A.
    let mut from_b = Raw::open(a.addr, &b.ident, &a).await;
    from_b.send(Kind::MailboxFetch, encode_cursor(0)).await;
    assert!(
        from_b
            .next(Kind::MailboxItem, Duration::from_millis(500))
            .await
            .is_none(),
        "no pair is answered"
    );
    let raw_at = PeerAddr::Tcp(from_b.s.local_addr().unwrap());
    drop(from_b);
    until(10, || async {
        (!a.t.peers().contains(&raw_at)).then_some(())
    })
    .await;
    // B is back on its own connection: still nothing is forwarded.
    let before = puts(&a, &b).len();
    b.t.maintain(a.addr, Some(a.id()));
    let at_b = PeerAddr::Tcp(a.addr);
    until(10, || async { b.t.peers().contains(&at_b).then_some(()) }).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(puts(&a, &b).len(), before, "nothing forwarded");
    assert!(a.f.forward_take(b.id(), 8).await.is_err());
    assert_eq!(a.f.status().await.unwrap().counters.forward_refused, 0);
    // Back on: the row pages in from the store and goes (the switch is a
    // forwarder run).
    a.f.patch_settings(SettingsPatch {
        hub_role: Some(true),
        ..SettingsPatch::default()
    })
    .await
    .unwrap();
    until_state(&m.f, id, MsgState::Delivered, 20).await;
    assert_eq!(protected(&a.f).await, 0);
}

/// The `FabricEvent::Forward`s for `target` among `events`.
fn forwards(events: &[FabricEvent], target: &Node) -> Vec<u32> {
    let id = target.id();
    events
        .iter()
        .filter_map(|e| match e {
            FabricEvent::Forward { target, pending } if *target == id => Some(*pending),
            _ => None,
        })
        .collect()
}

/// The `Forward`s for `target` that `f` emits once its ticks settled: an
/// API reply can precede the end of the tick that emits (the events of a
/// tick go out after its batch), so this waits for the first one (up to
/// 5 s) and then collects what follows over a few more ticks.
async fn forwards_settled(
    f: &Fabric,
    rx: &mut tokio::sync::broadcast::Receiver<FabricEvent>,
    target: &Node,
) -> Vec<u32> {
    let mut got = Vec::new();
    for _ in 0..250 {
        f.status().await.unwrap();
        got.extend(forwards(&drain_events(rx), target));
        if !got.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        f.status().await.unwrap();
        got.extend(forwards(&drain_events(rx), target));
    }
    got
}

/// Take what G owes G′ and report every put `Taken`.
async fn take_posted(g: &Node, g2: &Node) -> Vec<mesh111::fabric::CarrierFrame> {
    let out = g.f.forward_take(g2.id(), 8).await.unwrap();
    for p in &out {
        if let Some(seq) = p.custody_seq {
            g.f.forward_done(seq, ForwardDone::Taken).await.unwrap();
        }
    }
    out
}

/// §5.9 re-post rule (1) (M4b, M4c): a verified far frame after a post
/// marks it; the re-arm waits for the take after the drain (a drain's own
/// acks commit first); one re-post per row per far-hub round, never two
/// within one round, and a new round (a far frame after the re-post with
/// no ack for it) allows one more.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rule_1_re_arms_a_post_once_per_far_hub_round() {
    let d = Dirs::new();
    let g = hub(&d, "hub-g").await;
    let g2 = hub(&d, "hub-g2").await;
    pair(&g, &g2, PairRole::Peer, PairVia::Carrier).await;
    for h in [&g, &g2] {
        h.f.set_carrier_max_frame(Some(3_068)).await.unwrap();
    }
    let m = member(&d, "mary", &g, false).await;
    let y = member(&d, "yuri", &g2, false).await;
    y.t.release(g2.addr);
    let mut raw = Raw::open(g.addr, &m.ident, &g).await;
    let hint = Some([0xEE; DEST_HUB_LEN]); // upstream: the carrier pair
    for i in 1..=4u8 {
        assert_eq!(raw.put(&y.id(), &item(i, 300), hint).await, Accepted);
    }
    let posts = take_posted(&g, &g2).await;
    assert_eq!(posts.len(), 4);
    // G′ takes the first three; their acks come back in one drain.
    for p in &posts[..3] {
        g2.f.carrier_inbound(g.id(), p.bytes.clone()).await.unwrap();
    }
    let acks = g2.f.forward_take(g.id(), 8).await.unwrap();
    assert_eq!(acks.len(), 3);
    let mut gev = g.f.subscribe();
    for a in &acks {
        g.f.carrier_inbound(g2.id(), a.bytes.clone()).await.unwrap();
    }
    assert_eq!(
        forwards_settled(&g.f, &mut gev, &g2).await.len(),
        1,
        "the far frame tells the carrier once that a post is owed again"
    );
    assert_eq!(
        pair_view(&g.f, &g2).await.reposts,
        0,
        "nothing re-armed while the drain is handed in"
    );
    for i in 1..=3u8 {
        assert!(row(&g.f, &item(i, 300)).await.unwrap().committed);
    }
    // Round 1: the take re-arms the fourth post, unacknowledged, once.
    let re = take_posted(&g, &g2).await;
    assert_eq!(re.len(), 1);
    assert_eq!(re[0].custody_seq, posts[3].custody_seq);
    assert_eq!(pair_view(&g.f, &g2).await.reposts, 1);
    // Never twice within the round: flushes while G′ is silent, and a
    // replayed (stale) far frame, post nothing again.
    assert!(take_posted(&g, &g2).await.is_empty(), "never twice");
    g.f.carrier_inbound(g2.id(), acks[0].bytes.clone())
        .await
        .unwrap();
    assert!(
        take_posted(&g, &g2).await.is_empty(),
        "a stale frame is no round"
    );
    assert_eq!(pair_view(&g.f, &g2).await.reposts, 1);
    // Round 2: G′ flushes again after the re-post (the ack of a fifth
    // put, none for the fourth): the fourth may be re-posted once more.
    assert_eq!(raw.put(&y.id(), &item(5, 300), hint).await, Accepted);
    let fifth = take_posted(&g, &g2).await;
    assert_eq!(fifth.len(), 1, "only the new put: the fourth waits");
    g2.f.carrier_inbound(g.id(), fifth[0].bytes.clone())
        .await
        .unwrap();
    for a in g2.f.forward_take(g.id(), 8).await.unwrap() {
        g.f.carrier_inbound(g2.id(), a.bytes).await.unwrap();
    }
    assert!(row(&g.f, &item(5, 300)).await.unwrap().committed);
    let re2 = take_posted(&g, &g2).await;
    assert_eq!(re2.len(), 1, "one re-post in the second round");
    assert_eq!(re2[0].custody_seq, posts[3].custody_seq);
    assert_eq!(pair_view(&g.f, &g2).await.reposts, 2);
    assert!(
        take_posted(&g, &g2).await.is_empty(),
        "never twice in it either"
    );
    assert_eq!(pair_view(&g.f, &g2).await.reposts, 2);
    // "Flush now" (rule (2)) still re-arms it.
    assert_eq!(g.f.forward_repost(g2.id()).await.unwrap(), 1);
}

/// M4c (§5.9 rule (1)): a drain carrying only acks for other rows emits a
/// `Forward` for the pair when a post is still unacknowledged after it,
/// so the re-post does not wait for an unrelated trigger.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_far_frame_with_a_post_outstanding_emits_forward() {
    let d = Dirs::new();
    let g = hub(&d, "hub-g").await;
    let g2 = hub(&d, "hub-g2").await;
    pair(&g, &g2, PairRole::Peer, PairVia::Carrier).await;
    for h in [&g, &g2] {
        h.f.set_carrier_max_frame(Some(3_068)).await.unwrap();
    }
    let m = member(&d, "mary", &g, false).await;
    let y = member(&d, "yuri", &g2, false).await;
    y.t.release(g2.addr);
    let mut raw = Raw::open(g.addr, &m.ident, &g).await;
    let hint = Some([0xEE; DEST_HUB_LEN]);
    for i in 1..=3u8 {
        assert_eq!(raw.put(&y.id(), &item(i, 300), hint).await, Accepted);
    }
    let posts = take_posted(&g, &g2).await;
    assert_eq!(posts.len(), 3);
    for p in &posts[..2] {
        g2.f.carrier_inbound(g.id(), p.bytes.clone()).await.unwrap();
    }
    let acks = g2.f.forward_take(g.id(), 8).await.unwrap();
    assert_eq!(acks.len(), 2, "acks for R1 and R2 only");
    let mut gev = g.f.subscribe();
    for a in &acks {
        g.f.carrier_inbound(g2.id(), a.bytes.clone()).await.unwrap();
    }
    let ev = forwards_settled(&g.f, &mut gev, &g2).await;
    assert_eq!(ev.len(), 1, "one Forward for the pair: {ev:?}");
    assert!(ev[0] >= 1);
    let re = take_posted(&g, &g2).await;
    assert_eq!(re.len(), 1);
    assert_eq!(re[0].custody_seq, posts[2].custody_seq, "R3 re-armed");
    // Nothing owed after it: a silent far hub gets no further event.
    let mut gev = g.f.subscribe();
    assert!(take_posted(&g, &g2).await.is_empty());
    tokio::time::sleep(Duration::from_millis(100)).await;
    g.f.status().await.unwrap();
    g.f.status().await.unwrap();
    assert!(forwards(&drain_events(&mut gev), &g2).is_empty());
}

/// M4b: a pair's `CUSTODY_ACK` is read only fresh, on a connection
/// attributed to the pair the put went to: a replayed `REJECT_FULL` and an
/// `ACCEPTED` from another pair change nothing; the real answer commits.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_replayed_or_misdirected_pair_ack_changes_nothing() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    let c = hub(&d, "hub-c").await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    pair(&a, &c, PairRole::Peer, PairVia::Tcp).await;
    let m = member(&d, "mary", &a, false).await;
    // B and C go away; raw connections speak for them.
    for h in [&b, &c] {
        h.f.abort();
        h.t.shutdown();
    }
    for h in [&b, &c] {
        until(10, || async {
            (!pair_view(&a.f, h).await.reachable).then_some(())
        })
        .await;
    }
    let x = NodeId::from([0x77; 32]);
    let r = item(9, 300);
    let mut raw = Raw::open(a.addr, &m.ident, &a).await;
    assert_eq!(raw.put(&x, &r, None).await, Accepted, "held for B first");
    let mut rb = Raw::open(a.addr, &b.ident, &a).await;
    rb.send(Kind::MailboxFetch, encode_cursor(0)).await;
    let put = rb.next(Kind::CustodyPut, T).await.expect("A puts R to B");
    assert_eq!(decode_put(&put).unwrap().1, &r[..]);
    let full = rb.ack(&x, &r, CustodyStatus::RejectFull).await;
    // A retry trigger: B's next fetch re-offers R.
    tokio::time::sleep(Duration::from_millis(200)).await;
    rb.send(Kind::MailboxFetch, encode_cursor(0)).await;
    rb.next(Kind::CustodyPut, T).await.expect("R again");
    let stale = a.f.status().await.unwrap().counters.stale;
    rb.write(&full).await; // the replay
    let mut rc = Raw::open(a.addr, &c.ident, &a).await;
    rc.send(Kind::MailboxFetch, encode_cursor(0)).await;
    rc.ack(&x, &r, Accepted).await; // C was never sent R
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!row(&a.f, &r).await.unwrap().committed, "neither counted");
    assert_eq!(a.f.status().await.unwrap().counters.stale, stale + 1);
    rb.ack(&x, &r, Accepted).await;
    until(10, || async {
        row(&a.f, &r).await?.committed.then_some(())
    })
    .await;
    assert_eq!(a.f.status().await.unwrap().counters.forward_refused, 0);
}

/// M4b: a backlog beyond the resident window drains completely: at most
/// [`FORWARD_RESIDENT`] rows live in memory (the view counts them), the
/// rest wait in the store and page in as the window drains.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_6000_row_backlog_drains_through_the_resident_window() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    let m = member(&d, "mary", &a, false).await;
    let dd = member(&d, "dave", &b, false).await;
    dd.t.release(b.addr);
    b.t.release(a.addr);
    until(10, || async {
        (!pair_view(&a.f, &b).await.reachable).then_some(())
    })
    .await;
    const N: u32 = 6_000;
    let items: Vec<Vec<u8>> = (0..N).map(numbered).collect();
    let raw = Raw::open(a.addr, &m.ident, &a).await;
    assert_eq!(raw.blast(&dd.id(), &items).await, N as usize);
    assert_eq!(protected(&a.f).await, u64::from(N));
    assert_eq!(
        pair_view(&a.f, &b).await.pending as usize,
        FORWARD_RESIDENT,
        "the resident window; the rest is in the store"
    );
    b.t.maintain(a.addr, Some(a.id()));
    until(300, || async { (protected(&a.f).await == 0).then_some(()) }).await;
    let held = b.f.status().await.unwrap().custody.0;
    assert!(held >= u64::from(N), "B holds them all: {held}");
    let st = a.f.status().await.unwrap();
    assert_eq!(st.counters.forward_refused, 0);
    assert_eq!(pair_view(&a.f, &b).await.pending, 0);
}

/// M4b: the forwarder recomputes a row's targets only when its inputs
/// changed: a 1,000-row backlog drained to a pair that comes back costs at
/// most 3 computations per row (the gate's and the forwarder's), never one
/// per row per trigger.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn target_computations_stay_linear_in_a_1000_row_drain() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    let m = member(&d, "mary", &a, false).await;
    let dd = member(&d, "dave", &b, false).await;
    dd.t.release(b.addr);
    b.t.release(a.addr);
    until(10, || async {
        (!pair_view(&a.f, &b).await.reachable).then_some(())
    })
    .await;
    const N: u32 = 1_000;
    let before = a.f.forward_target_computations().await.unwrap();
    let items: Vec<Vec<u8>> = (0..N).map(numbered).collect();
    let raw = Raw::open(a.addr, &m.ident, &a).await;
    assert_eq!(raw.blast(&dd.id(), &items).await, N as usize);
    b.t.maintain(a.addr, Some(a.id()));
    until(120, || async { (protected(&a.f).await == 0).then_some(()) }).await;
    let spent = a.f.forward_target_computations().await.unwrap() - before;
    assert!(
        spent <= 3 * u64::from(N),
        "{spent} target computations for {N} rows"
    );
}

/// The `numbered` index of every distinct item `from` put to `to` for
/// `dest`, in the order of their first put.
fn first_puts(from: &Node, to: &Node, dest: &NodeId) -> Vec<u32> {
    let mut seen = HashSet::new();
    puts(from, to)
        .into_iter()
        .filter(|(d, _, _)| d == dest)
        .filter_map(|(_, it, _)| {
            let i = u32::from_le_bytes(it.get(1..5)?.try_into().ok()?);
            seen.insert(i).then_some(i)
        })
        .collect()
}

/// M4c (§5.5): the resident window is shared fairly. With B away, 5,000
/// rows whose turn is B fill the window and the store; rows for a dest
/// behind C (up) are still delivered while B is down, each in place of
/// one of B's newest resident rows (evicted to the store, never refused);
/// when B comes back every row for B is delivered too, each target's in
/// custody order, none lost or duplicated.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_down_pairs_backlog_never_holds_back_an_up_pairs_rows() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    let c = hub(&d, "hub-c").await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    pair(&a, &c, PairRole::Peer, PairVia::Tcp).await;
    let m = member(&d, "mary", &a, false).await;
    let dd = member(&d, "dave", &b, false).await;
    let cc = member(&d, "carl", &c, false).await;
    dd.t.release(b.addr);
    cc.t.release(c.addr);
    // One row for Carl while both are up: B (first in the probe order)
    // refuses it and C takes it, so the hit cache names C for Carl (the
    // stored rows are judged without their hint).
    let warm = numbered(1_000_000);
    let mut raw = Raw::open(a.addr, &m.ident, &a).await;
    assert_eq!(raw.put(&cc.id(), &warm, None).await, Accepted);
    until(10, || async {
        row(&a.f, &warm).await?.committed.then_some(())
    })
    .await;
    b.t.release(a.addr);
    until(10, || async {
        (!pair_view(&a.f, &b).await.reachable).then_some(())
    })
    .await;
    const N: u32 = 5_000;
    const K: u32 = 200;
    let for_b: Vec<Vec<u8>> = (0..N).map(numbered).collect();
    assert_eq!(raw.blast(&dd.id(), &for_b).await, N as usize);
    assert_eq!(protected(&a.f).await, u64::from(N));
    assert_eq!(
        pair_view(&a.f, &b).await.pending as usize,
        FORWARD_RESIDENT,
        "B's rows fill the window; the rest is in the store"
    );
    let for_c: Vec<Vec<u8>> = (N..N + K).map(numbered).collect();
    let raw = Raw::open(a.addr, &m.ident, &a).await;
    assert_eq!(raw.blast(&cc.id(), &for_c).await, K as usize);
    // Carl's rows reach C while B is still away.
    until(60, || async {
        (protected(&a.f).await == u64::from(N)).then_some(())
    })
    .await;
    assert!(!pair_view(&a.f, &b).await.reachable, "B is still away");
    assert_eq!(
        c.f.status().await.unwrap().custody.0,
        u64::from(K) + 1,
        "C holds each of Carl's rows once"
    );
    // B's evicted rows page back in as C's commits free room (page-in
    // skips nothing), so B's resident count is bounded by the window only.
    let at_b = pair_view(&a.f, &b).await.pending as usize;
    assert!(
        at_b <= FORWARD_RESIDENT,
        "B's rows stay within the window: {at_b}"
    );
    assert_eq!(
        first_puts(&a, &c, &cc.id()),
        [1_000_000].into_iter().chain(N..N + K).collect::<Vec<_>>(),
        "C got Carl's rows in custody order, once each"
    );
    // B comes back: everything for B, in custody order, once each.
    b.t.maintain(a.addr, Some(a.id()));
    until(300, || async { (protected(&a.f).await == 0).then_some(()) }).await;
    assert_eq!(
        b.f.status().await.unwrap().custody.0,
        u64::from(N),
        "B holds each of Dave's rows once"
    );
    assert_eq!(
        first_puts(&a, &b, &dd.id()),
        (0..N).collect::<Vec<_>>(),
        "B got Dave's rows in custody order"
    );
    let st = a.f.status().await.unwrap();
    assert_eq!(st.counters.forward_refused, 0, "evicted, never refused");
    assert_eq!(pair_view(&a.f, &b).await.pending, 0);
    assert_eq!(pair_view(&a.f, &c).await.pending, 0);
}

/// Rows held at `h` (its custody count): how a test watches a hub without
/// calling the hub under test.
async fn held(h: &Node) -> u64 {
    h.f.status().await.unwrap().custody.0
}

/// Wait until `h` reports `.reachable` for `of` false.
async fn until_down(h: &Node, of: &Node) {
    until(10, || async {
        (!pair_view(&h.f, of).await.reachable).then_some(())
    })
    .await;
}

/// M4d (§5.5, module doc "The resident window"): the swap probe reads a
/// deep store on its own. B's rows (B down) fill the window and 1,536 more
/// (six pages) wait in the store ahead of one row for Yuri, whose turn is
/// the carrier pair G′ (the hit cache) while the carrier bound is unset.
/// The one trigger, setting the bound, is a tick of four passes (four
/// pages); after it nothing calls the hub: each tick that leaves the probe
/// more to read queues one self-wake, so the probe reaches Yuri's row and
/// the hub tells the carrier it is owed. Watched only through the hub's
/// event stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_swap_probe_reads_a_deep_store_on_its_own() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    let g2 = hub(&d, "hub-g2").await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    pair(&a, &g2, PairRole::Peer, PairVia::Carrier).await;
    for h in [&a, &g2] {
        h.f.set_carrier_max_frame(Some(3_068)).await.unwrap();
    }
    let m = member(&d, "mary", &a, false).await;
    let dd = member(&d, "dave", &b, false).await;
    let y = member(&d, "yuri", &g2, false).await;
    dd.t.release(b.addr);
    y.t.release(g2.addr);
    // Yuri's first row crosses the carrier and G′ accepts it: the cache
    // names G′ for Yuri.
    let warm = numbered(1_000_000);
    let mut raw = Raw::open(a.addr, &m.ident, &a).await;
    let upstream = Some([0xEE; DEST_HUB_LEN]);
    assert_eq!(raw.put(&y.id(), &warm, upstream).await, Accepted);
    carry(&a, &g2).await;
    carry(&a, &g2).await;
    assert!(row(&a.f, &warm).await.unwrap().committed);
    // The carrier goes away (a held row for G′ waits) and B too.
    a.f.set_carrier_max_frame(None).await.unwrap();
    b.t.release(a.addr);
    until_down(&a, &b).await;
    const N: u32 = FORWARD_RESIDENT as u32 + 1_536;
    let for_b: Vec<Vec<u8>> = (0..N).map(numbered).collect();
    let blast = Raw::open(a.addr, &m.ident, &a).await;
    assert_eq!(blast.blast(&dd.id(), &for_b).await, N as usize);
    let yuri = numbered(2_000_000);
    assert_eq!(raw.put(&y.id(), &yuri, None).await, Accepted);
    assert_eq!(protected(&a.f).await, u64::from(N) + 1);
    assert_eq!(
        pair_view(&a.f, &g2).await.pending,
        0,
        "Yuri's row waits in the store behind B's"
    );
    let mut ev = a.f.subscribe();
    a.f.set_carrier_max_frame(Some(3_068)).await.unwrap();
    // From here on nothing calls A: only its own self-wakes move it on.
    let owed = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match ev.recv().await {
                Ok(FabricEvent::Forward { target, pending }) if target == g2.id() => {
                    return pending;
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("{e}"),
            }
        }
    })
    .await
    .expect("the probe reached Yuri's row with no call into the hub");
    assert_eq!(owed, 1, "Yuri's row only");
    let out = a.f.forward_take(g2.id(), 64).await.unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(first_puts(&a, &g2, &y.id()), [1_000_000, 2_000_000]);
    assert!(puts(&a, &g2).iter().all(|(dest, _, _)| *dest == y.id()));
    let st = a.f.status().await.unwrap();
    assert_eq!(st.counters.forward_refused, 0, "evicted, never refused");
    assert_eq!(protected(&a.f).await, u64::from(N) + 1);
}

/// M4d (§5.5): a change that turns stored rows' turn up restarts the
/// probe. Pairs [B, C, D]: D's rows (D down) fill the window; Carl's rows
/// wait in the store with their turn at B (down, first in the probe
/// order) and the probe found nothing to move. The owner unpairs B: those
/// rows now go to C, before newer puts for Carl, in custody order.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unpairing_moves_stored_rows_before_newer_ones() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let b = hub(&d, "hub-b").await;
    let c = hub(&d, "hub-c").await;
    let dh = hub(&d, "hub-d").await;
    pair(&a, &b, PairRole::Peer, PairVia::Tcp).await;
    pair(&a, &c, PairRole::Peer, PairVia::Tcp).await;
    pair(&a, &dh, PairRole::Peer, PairVia::Tcp).await;
    let m = member(&d, "mary", &a, false).await;
    let cc = member(&d, "carl", &c, false).await;
    let dd = member(&d, "dave", &dh, false).await;
    cc.t.release(c.addr);
    dd.t.release(dh.addr);
    // B and C refuse Dave's first row and D takes it: the cache names D.
    let warm = numbered(1_000_000);
    let mut raw = Raw::open(a.addr, &m.ident, &a).await;
    assert_eq!(raw.put(&dd.id(), &warm, None).await, Accepted);
    until(10, || async {
        row(&a.f, &warm).await?.committed.then_some(())
    })
    .await;
    b.t.release(a.addr);
    dh.t.release(a.addr);
    until_down(&a, &b).await;
    until_down(&a, &dh).await;
    const W: u32 = FORWARD_RESIDENT as u32;
    const S: u32 = 100;
    const P: u32 = 20;
    let for_d: Vec<Vec<u8>> = (0..W).map(numbered).collect();
    let blast = Raw::open(a.addr, &m.ident, &a).await;
    assert_eq!(blast.blast(&dd.id(), &for_d).await, W as usize);
    let for_carl: Vec<Vec<u8>> = (W..W + S).map(numbered).collect();
    let blast = Raw::open(a.addr, &m.ident, &a).await;
    assert_eq!(blast.blast(&cc.id(), &for_carl).await, S as usize);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(protected(&a.f).await, u64::from(W + S));
    assert_eq!(
        pair_view(&a.f, &c).await.pending,
        0,
        "Carl's rows wait for B in the store"
    );
    assert!(first_puts(&a, &c, &cc.id()).is_empty());
    assert!(a.f.unpair(b.id()).await.unwrap());
    let newer: Vec<Vec<u8>> = (W + S..W + S + P).map(numbered).collect();
    let blast = Raw::open(a.addr, &m.ident, &a).await;
    assert_eq!(blast.blast(&cc.id(), &newer).await, P as usize);
    until(60, || async {
        (held(&c).await == u64::from(S + P)).then_some(())
    })
    .await;
    assert_eq!(
        first_puts(&a, &c, &cc.id()),
        (W..W + S + P).collect::<Vec<_>>(),
        "the ex-B rows first, then the newer ones, in custody order"
    );
    let st = a.f.status().await.unwrap();
    assert_eq!(st.counters.forward_refused, 0, "evicted, never refused");
    assert_eq!(protected(&a.f).await, u64::from(W), "D's rows wait for D");
}

/// M4d (§5.5, C16): a pair whose `REJECT_FULL` rows wait counts as down
/// for the window. C's rows fill it and C answers every put `REJECT_FULL`
/// (its custody quota is full); Dora's rows (turn D, the hit cache) wait
/// in the store. D comes up: Dora's rows enter in place of C's newest
/// (full or idle, evicted to the store, never refused) and reach D in
/// custody order while C stays full.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_full_pair_never_holds_back_another_pairs_stored_rows() {
    let d = Dirs::new();
    let a = hub(&d, "hub-a").await;
    let c = hub(&d, "hub-c").await;
    let dh = hub(&d, "hub-d").await;
    pair(&a, &c, PairRole::Peer, PairVia::Tcp).await;
    pair(&a, &dh, PairRole::Peer, PairVia::Tcp).await;
    let m = member(&d, "mary", &a, false).await;
    let cc = member(&d, "carl", &c, false).await;
    let dd = member(&d, "dora", &dh, false).await;
    cc.t.release(c.addr);
    dd.t.release(dh.addr);
    // C refuses Dora's first row and D takes it: the cache names D.
    let warm = numbered(1_000_000);
    let mut raw = Raw::open(a.addr, &m.ident, &a).await;
    assert_eq!(raw.put(&dd.id(), &warm, None).await, Accepted);
    until(10, || async {
        row(&a.f, &warm).await?.committed.then_some(())
    })
    .await;
    c.t.release(a.addr);
    dh.t.release(a.addr);
    until_down(&a, &c).await;
    until_down(&a, &dh).await;
    c.f.patch_settings(SettingsPatch {
        quotas: vec![(Class::Custody, Some(1))],
        ..SettingsPatch::default()
    })
    .await
    .unwrap();
    const W: u32 = FORWARD_RESIDENT as u32;
    const K: u32 = 100;
    let for_c: Vec<Vec<u8>> = (0..W).map(numbered).collect();
    let blast = Raw::open(a.addr, &m.ident, &a).await;
    assert_eq!(blast.blast(&cc.id(), &for_c).await, W as usize);
    let for_d: Vec<Vec<u8>> = (W..W + K).map(numbered).collect();
    let blast = Raw::open(a.addr, &m.ident, &a).await;
    assert_eq!(blast.blast(&dd.id(), &for_d).await, K as usize);
    // C comes back full: its link-up releases one window of puts, each
    // answered REJECT_FULL, and nothing retries them.
    c.t.maintain(a.addr, Some(a.id()));
    until(10, || async { (puts(&a, &c).len() >= 32).then_some(()) }).await;
    let mut seen = 0;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let now = puts(&a, &c).len();
        if now == seen {
            break;
        }
        seen = now;
    }
    assert_eq!(held(&c).await, 0, "C is full");
    assert_eq!(held(&dh).await, 1);
    dh.t.maintain(a.addr, Some(a.id()));
    until(60, || async {
        (held(&dh).await == u64::from(K) + 1).then_some(())
    })
    .await;
    assert_eq!(
        first_puts(&a, &dh, &dd.id()),
        [1_000_000].into_iter().chain(W..W + K).collect::<Vec<_>>(),
        "D got Dora's rows in custody order, once each"
    );
    let st = a.f.status().await.unwrap();
    assert_eq!(st.counters.forward_refused, 0, "evicted, never refused");
    assert_eq!(protected(&a.f).await, u64::from(W), "C's rows wait for C");
}

/// A raw connection speaking for the pair `p` (its own fabric is gone): one
/// fetch attributes it, so `a` puts to `p` on it.
async fn raw_pair(a: &Node, p: &Node) -> Raw {
    let mut r = Raw::open(a.addr, &p.ident, a).await;
    r.send(Kind::MailboxFetch, encode_cursor(0)).await;
    until(10, || async {
        pair_view(&a.f, p).await.reachable.then_some(())
    })
    .await;
    r
}

/// The next put written to a raw pair, answered `ACCEPTED`: `(dest, the
/// numbered index of its item)`.
async fn take_put(r: &mut Raw) -> (NodeId, u32) {
    let put = r.next(Kind::CustodyPut, T).await.expect("a put in time");
    let (dest, it) = decode_put(&put).unwrap();
    let it = it.to_vec();
    r.ack(&dest, &it, Accepted).await;
    (dest, u32::from_le_bytes(it[1..5].try_into().unwrap()))
}

/// The `i`-th of many distinct dests no hub holds as a member.
fn far_dest(i: u32) -> NodeId {
    let mut b = [0x7D; 32];
    b[..4].copy_from_slice(&i.to_le_bytes());
    NodeId::from(b)
}

/// Pairs [D, C] at hub A: D away, C a raw connection that takes every put.
/// `held` rows for `x` (turn D: first in the probe order, no hint, no cache
/// entry) fill the window and wait behind it, then `zoe` rows for `zoe`
/// (numbered on). C comes up and the probe sweeps the store once, moving
/// nothing (one run: at most four pages). Returns A, Mary (A's member),
/// the raw C and the hubs to keep alive.
async fn behind_a_down_pair(
    d: &Dirs,
    x: NodeId,
    held: u32,
    zoe: NodeId,
    zoes: u32,
) -> (Node, Node, Node, Raw, Node) {
    let a = hub(d, "hub-a").await;
    let dh = hub(d, "hub-d").await;
    let c = hub(d, "hub-c").await;
    pair(&a, &dh, PairRole::Peer, PairVia::Tcp).await;
    pair(&a, &c, PairRole::Peer, PairVia::Tcp).await;
    let m = member(d, "mary", &a, false).await;
    c.f.abort();
    c.t.shutdown();
    dh.t.release(a.addr);
    until_down(&a, &c).await;
    until_down(&a, &dh).await;
    let for_x: Vec<Vec<u8>> = (0..held).map(numbered).collect();
    let blast = Raw::open(a.addr, &m.ident, &a).await;
    assert_eq!(blast.blast(&x, &for_x).await, held as usize);
    if zoes > 0 {
        let for_zoe: Vec<Vec<u8>> = (held..held + zoes).map(numbered).collect();
        let blast = Raw::open(a.addr, &m.ident, &a).await;
        assert_eq!(blast.blast(&zoe, &for_zoe).await, zoes as usize);
    }
    let behind = u64::from(held + zoes) - FORWARD_RESIDENT as u64;
    assert!(behind <= 4 * 256, "one run sweeps it");
    let before = a.f.forward_target_computations().await.unwrap();
    let rc = raw_pair(&a, &c).await;
    // C's coming up restarts the probe: one sweep, one target computation
    // per stored row, nothing moves (every stored row's turn is D).
    until(10, || async {
        let now = a.f.forward_target_computations().await.ok()?;
        (now >= before + behind).then_some(())
    })
    .await;
    assert_eq!(pair_view(&a.f, &c).await.pending, 0, "nothing moved");
    (a, m, c, rc, dh)
}

/// M4e (§5.5, the re-review's F1): a hit-cache entry added for a dest with
/// no row in the store restarts nothing. D's rows fill the window and 512
/// more wait behind it; C is up and the probe found nothing to move. Rows
/// hinted to C for more dests than the hit cache holds: each commit adds
/// an entry (the cache churns), none restarts the probe, so each row is
/// admitted at once and no page of the store is read again: three target
/// computations a row (the gate's, the admission's, the pass's), where a
/// sweep would cost one per stored row at every commit.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cache_entry_for_a_dest_with_no_stored_row_never_restarts_the_probe() {
    let d = Dirs::new();
    const W: u32 = FORWARD_RESIDENT as u32;
    const S: u32 = 512;
    let x = NodeId::from([0x71; 32]);
    let (a, m, c, mut rc, _dh) = behind_a_down_pair(&d, x, W + S, x, 0).await;
    let hint = Some(prefix(&c.id()));
    let mut raw = Raw::open(a.addr, &m.ident, &a).await;
    let before = a.f.forward_target_computations().await.unwrap();
    const N: u32 = FORWARD_CACHE as u32 + 44;
    for i in 0..N {
        let it = numbered(1_000_000 + i);
        assert_eq!(raw.put(&far_dest(i), &it, hint).await, Accepted);
        assert_eq!(take_put(&mut rc).await, (far_dest(i), 1_000_000 + i));
    }
    let last = numbered(1_000_000 + N - 1);
    until(10, || async {
        row(&a.f, &last).await?.committed.then_some(())
    })
    .await;
    let spent = a.f.forward_target_computations().await.unwrap() - before;
    // What the test means: no commit restarts the probe, so the store is
    // not swept again at each commit. A row costs three target computations
    // (the gate's, the admission's, the pass's: 3N = 900), and a sweep one
    // per stored row above the floor (S + 1 = 513: the first admission
    // evicted one of x's rows from the full window). The bound allows twice
    // the per-row cost and four whole sweeps, 6N + 4(S + 1) = 3,852, so a
    // computation more per row or a stray legitimate restart (or the hit
    // cache's own dropped entries, M4f: those dests hold no row anywhere)
    // passes; a restart on every changed entry (M4d's rule) sweeps the S + 1
    // rows after each of the N commits, about N(S + 1) = 153,900, and fails
    // it forty times over.
    let bound = 6 * u64::from(N) + 4 * (u64::from(S) + 1);
    assert!(
        spent < bound,
        "{spent} target computations for {N} rows (bound {bound}): the store was swept again"
    );
    assert_eq!(
        protected(&a.f).await,
        u64::from(W + S),
        "D's rows wait for D"
    );
    let st = a.f.status().await.unwrap();
    assert_eq!(st.counters.forward_refused, 0);
}

/// M4e (§5.5): a dest's first hit at C restarts the probe when rows of that
/// dest wait in the store. D's rows fill the window and 100 more wait,
/// then 40 rows for Zoe (turn D: no hint, no cache entry); C is up and the
/// probe found nothing to move. A row for Zoe hinted to C is admitted at
/// once and C takes it: the new cache entry finds Zoe's rows in the store,
/// the probe restarts, and they reach C in custody order before a newer
/// row for Zoe put after that commit.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dests_first_hit_moves_its_stored_rows_before_a_newer_one() {
    let d = Dirs::new();
    const W: u32 = FORWARD_RESIDENT as u32;
    const S: u32 = 100;
    const Z: u32 = 40;
    let (x, zoe) = (NodeId::from([0x71; 32]), NodeId::from([0x2E; 32]));
    let (a, m, c, mut rc, _dh) = behind_a_down_pair(&d, x, W + S, zoe, Z).await;
    let mut raw = Raw::open(a.addr, &m.ident, &a).await;
    let first = numbered(2_000_000);
    let hint = Some(prefix(&c.id()));
    assert_eq!(raw.put(&zoe, &first, hint).await, Accepted);
    assert_eq!(take_put(&mut rc).await, (zoe, 2_000_000));
    until(10, || async {
        row(&a.f, &first).await?.committed.then_some(())
    })
    .await;
    let newer = numbered(2_000_001);
    assert_eq!(raw.put(&zoe, &newer, None).await, Accepted);
    let mut got = Vec::new();
    for _ in 0..=Z {
        let (dest, i) = take_put(&mut rc).await;
        assert_eq!(dest, zoe);
        got.push(i);
    }
    assert_eq!(
        got,
        (W + S..W + S + Z).chain([2_000_001]).collect::<Vec<_>>(),
        "Zoe's stored rows first, in custody order, then the newer one"
    );
    until(10, || async {
        row(&a.f, &newer).await?.committed.then_some(())
    })
    .await;
    assert_eq!(
        protected(&a.f).await,
        u64::from(W + S),
        "D's rows wait for D"
    );
    let st = a.f.status().await.unwrap();
    assert_eq!(st.counters.forward_refused, 0, "evicted, never refused");
}
