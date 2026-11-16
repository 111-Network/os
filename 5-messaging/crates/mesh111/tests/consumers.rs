//! Layer 8 §12.3, G14 (8) `[L5-L8:consumer-registry]`: the consumer
//! registry for 0x10–0x14 (`Fabric::set_consumer`, an additive amendment of
//! L5's dispatch). The fabric under test listens on `tcp-lan`; a raw peer
//! ([`Peer`]) speaks there as its contact `pat`, sealing data frames of
//! every registry kind by hand (no sender API for these kinds exists before
//! Layer 9's `send_op`) and reading the fabric's ACKs back. Asserted:
//!
//! - an inbound `SYNC`, `KB`, `UPDATE`, `PATCH` or `AGENT_OP` from a
//!   contact never becomes a message row (a `MSG` still does);
//! - with no consumer it is acknowledged and counted (`unconsumed_*`);
//! - with a consumer it reaches the consumer once, and the fabric commits
//!   (read on a connection of the store's own) and acknowledges only after
//!   the consumer returned;
//! - a consumer that fails leaves the frame unacknowledged and uncommitted,
//!   and the retransmission reaches it again;
//! - the fault point (a process killed after the consumer's write and
//!   before the fabric's commit) redelivers the frame at the next
//!   connection (through the mailbox, at the next start's fetch), and the
//!   consumer, idempotent by `msg_id`, keeps one record (debug builds: the
//!   fault point is absent from a release build);
//! - a duplicate is re-acknowledged and never reaches the consumer;
//! - a Tier 1 frame reaches the consumer whole, and its completing fragment
//!   is kept only with the delivery: a failure or a kill leaves the partial
//!   incomplete, so no restart finishes it as a message row; with no
//!   consumer it is counted once and its fragments go with it;
//! - a body the consumer refuses, answered `Ok` as the trait asks, is
//!   committed and acknowledged and holds nothing behind it;
//! - through the mailbox (a [`Hub`] holds the items; L5 §9) a frame comes
//!   unattributed, and one the consumer did not take keeps the member's
//!   cursor below it, with every item after it, until the hub streams it
//!   again at the next fetch.

mod common;

use std::collections::BTreeMap;
use std::error::Error;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use id111::keyset::NodeId;
use mesh111::ack::AckPayload;
use mesh111::contacts::{card_text, ROLE_ALWAYS_ON_HUB, ROLE_MAILBOX};
use mesh111::envelope::{
    blake2b_256, open_envelope, seal_envelope, FragPos, Inner, Kind, PeerKeys, Priority,
};
use mesh111::fabric::{Consumer, Fabric, FabricError, FabricSettings};
use mesh111::frag::frag_slice;
use mesh111::ids::{MsgId, MsgSeq};
use mesh111::link::tcp_lan::{
    encode_frame, read_frame, read_hello, TcpLan, HELLO, T_ANNOUNCE, T_SEALED,
};
use mesh111::link::Dialer;
use mesh111::outbox::MessageBody;
use mesh111::store::{CustodyPut, Partial, RxEpoch};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

/// The five registry kinds, in byte order.
const KINDS: [Kind; 5] = [
    Kind::AgentOp,
    Kind::Kb,
    Kind::Update,
    Kind::Patch,
    Kind::Sync,
];
/// How long an ACK that must come is waited for.
const T: Duration = Duration::from_secs(10);
/// How long the socket is watched for an ACK that must not come.
const QUIET: Duration = Duration::from_millis(400);
/// `pat`'s epoch on every frame.
const EPOCH: u64 = 7;

fn seq(n: u32) -> MsgSeq {
    MsgSeq::new(EPOCH, n)
}

/// A distinct body per kind and number.
fn body(kind: Kind, n: u32) -> Vec<u8> {
    format!("{kind:?} body {n}").into_bytes()
}

/// The bodies [`Rec`] refuses (taken with `Ok`, nothing kept), as a
/// consumer does with a body it cannot parse.
const REFUSED: &[u8] = b"refused:";

/// One data frame from `who` sealed to `to` (a mailbox item's bytes).
fn sealed(who: &Ident, to: &PeerKeys, kind: Kind, n: u32, payload: &[u8]) -> Vec<u8> {
    let inner = Inner::new(&who.id(), kind, seq(n), payload.to_vec());
    seal_envelope(who, to, Priority::Interactive, &inner).unwrap()
}

/// One call the consumer saw.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Call {
    kind: Kind,
    msg_id: MsgId,
    attributed: bool,
    body: Vec<u8>,
    /// Whether the frame's `n` was already committed in the fabric store
    /// while the consumer ran (`None` without a probe).
    committed: Option<bool>,
}

/// A consumer that keeps one record per `msg_id` (its durable write: it
/// outlives a fabric restart, as the test holds it) and logs every call. A
/// body that starts with [`REFUSED`] is refused as the trait asks: `Ok`,
/// nothing kept.
#[derive(Default)]
struct Rec {
    calls: Mutex<Vec<Call>>,
    kept: Mutex<BTreeMap<MsgId, Vec<u8>>>,
    /// Refuse the write (an error) while set.
    fail: AtomicBool,
    /// The fabric store (its directory and key) to read at every call.
    probe: Option<(PathBuf, [u8; 32])>,
}

impl Rec {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn kept(&self) -> usize {
        self.kept.lock().unwrap().len()
    }
}

impl Consumer for Rec {
    fn consume(
        &self,
        kind: Kind,
        msg_id: &MsgId,
        attributed: bool,
        body: &[u8],
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let committed = self
            .probe
            .as_ref()
            .map(|(dir, key)| committed_prefix(dir, key, &msg_id.sender) >= i64::from(msg_id.n));
        self.calls.lock().unwrap().push(Call {
            kind,
            msg_id: *msg_id,
            attributed,
            body: body.to_vec(),
            committed,
        });
        if self.fail.load(Ordering::SeqCst) {
            return Err("the consumer's store refused the write".into());
        }
        if body.starts_with(REFUSED) {
            return Ok(());
        }
        self.kept
            .lock()
            .unwrap()
            .entry(*msg_id)
            .or_insert_with(|| body.to_vec());
        Ok(())
    }
}

/// The key `common::start` opens a node's store with.
fn store_key(ident: &Ident) -> [u8; 32] {
    blake2b_256(&[b"f1-key", ident.id().as_bytes()])
}

/// The highest `n` of [`EPOCH`] from `peer` committed in the fabric store
/// at `dir`, read on a connection of its own (what a crash would leave
/// behind); 0 while none is.
fn committed_prefix(dir: &Path, key: &[u8; 32], peer: &NodeId) -> i64 {
    let conn = rusqlite::Connection::open(dir.join("store.db")).expect("open");
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    conn.execute_batch(&format!(
        "PRAGMA key = \"x'{hex}'\"; PRAGMA cipher_compatibility = 4;"
    ))
    .expect("key");
    conn.query_row(
        "SELECT coalesce(max(prefix), 0) FROM rx_state
         WHERE peer = ?1 AND stream = 0 AND epoch = ?2",
        rusqlite::params![peer.as_bytes().as_slice(), i64::try_from(EPOCH).unwrap()],
        |r| r.get(0),
    )
    .expect("rx_state")
}

/// The fabric under test, listening on `tcp-lan`.
struct Node {
    ident: Arc<Ident>,
    f: Fabric,
    t: TcpLan,
    addr: SocketAddr,
}

impl Node {
    /// Start (or restart) on `dir`; `consumer`, when given, is registered
    /// for every registry kind before the link is added (no frame can come
    /// before it).
    async fn start(ident: &Arc<Ident>, dir: &Path, consumer: Option<&Arc<Rec>>) -> Node {
        Node::open(ident, dir, consumer, FabricSettings::default(), false).await
    }

    /// [`Node::start`] with `settings`; `dial` gives the fabric its dialer.
    async fn open(
        ident: &Arc<Ident>,
        dir: &Path,
        consumer: Option<&Arc<Rec>>,
        settings: FabricSettings,
        dial: bool,
    ) -> Node {
        let f = start(ident, dir, settings);
        if let Some(c) = consumer {
            for kind in KINDS {
                f.set_consumer(kind, c.clone()).await.expect("registered");
            }
        }
        let t = tcp(&f, ident, "tcp");
        add_tcp(&f, &t, dial).await;
        let addr = listen(&t, 0).await;
        Node {
            ident: ident.clone(),
            f,
            t,
            addr,
        }
    }

    /// A member enrolled at `hub`, `consumer` registered as in
    /// [`Node::start`]; its fabric dials the hub and keeps that connection
    /// up.
    async fn member(ident: &Arc<Ident>, dir: &Path, consumer: &Arc<Rec>, hub: &Hub) -> Node {
        let settings = FabricSettings::default();
        let node = Node::open(ident, dir, Some(consumer), settings, true).await;
        let offer = hub.f.offer(Some(hub.addr)).await.expect("offer");
        tokio::time::timeout(
            T,
            node.f
                .enroll(hub.addr, &offer.code, Some(offer.card.as_str())),
        )
        .await
        .expect("enrolled in time")
        .expect("enrolled");
        node
    }

    /// A member of `hub` started again on `dir` (its home hub in its
    /// settings, as the daemon keeps them), dialing the hub.
    #[cfg(debug_assertions)]
    async fn member_again(ident: &Arc<Ident>, dir: &Path, consumer: &Arc<Rec>, hub: &Hub) -> Node {
        let settings = FabricSettings {
            home_hub: Some(mesh111::fabric::HomeHub {
                node_id: hub.ident.id(),
                address: None,
            }),
            ..FabricSettings::default()
        };
        let node = Node::open(ident, dir, Some(consumer), settings, false).await;
        node.t.maintain(hub.addr, Some(hub.ident.id()));
        node
    }

    fn keys(&self) -> PeerKeys {
        PeerKeys {
            node_id: self.ident.id(),
            enc_pubkey: self.ident.enc(),
        }
    }

    /// The member's mailbox cursor at `hub` (0 while it has none).
    async fn cursor(&self, hub: NodeId) -> u64 {
        let h = hub.to_bytes();
        self.f
            .with_store(move |s| s.cursor(&h))
            .await
            .unwrap()
            .unwrap()
    }

    /// A killed process: no clean close, its adapter gone with it.
    #[cfg(debug_assertions)]
    fn kill(&self) {
        self.f.abort();
        self.t.shutdown();
    }

    /// Message rows in the store, of any kind and direction.
    async fn rows(&self) -> u64 {
        self.f
            .with_store(|s| s.message_count())
            .await
            .unwrap()
            .unwrap()
    }

    /// The committed receive state of [`EPOCH`] from `peer` (an empty one
    /// while the store holds none).
    async fn rx(&self, peer: NodeId) -> RxEpoch {
        let p = peer.to_bytes();
        let snap = self
            .f
            .with_store(move |s| s.rx_snapshot(&p, 0))
            .await
            .unwrap()
            .unwrap();
        snap.into_iter()
            .find(|e| e.epoch == EPOCH)
            .unwrap_or(RxEpoch {
                epoch: EPOCH,
                prefix: 0,
                ranges: Vec::new(),
                partials: Vec::new(),
            })
    }

    /// The five `unconsumed_*` counters, in byte order.
    async fn unconsumed(&self) -> [u64; 5] {
        let c = self.f.status().await.unwrap().counters;
        [
            c.unconsumed_agent_op,
            c.unconsumed_kb,
            c.unconsumed_update,
            c.unconsumed_patch,
            c.unconsumed_sync,
        ]
    }

    /// Two calls through the actor: the second runs only after the tick
    /// that handled every frame seen so far flushed its ACKs.
    async fn settle(&self) {
        self.f.status().await.unwrap();
        self.f.status().await.unwrap();
    }
}

/// A hub (the mailbox role, L5 §9) listening on `tcp-lan`.
struct Hub {
    ident: Arc<Ident>,
    f: Fabric,
    /// Kept: the adapter lives as long as the hub.
    _t: TcpLan,
    addr: SocketAddr,
}

impl Hub {
    async fn start(dir: &Path) -> Hub {
        let ident = Ident::new("hub", ROLE_MAILBOX | ROLE_ALWAYS_ON_HUB);
        let f = start(&ident, dir, hub_settings());
        let t = tcp(&f, &ident, "tcp");
        add_tcp(&f, &t, true).await;
        let addr = listen(&t, 0).await;
        Hub {
            ident,
            f,
            _t: t,
            addr,
        }
    }

    /// Hold `item` for member `dest`, as a sender's `CUSTODY_PUT` leaves it
    /// (no sender API exists for the registry kinds); its custody sequence.
    async fn hold(&self, dest: NodeId, item: Vec<u8>) -> u64 {
        let d = dest.to_bytes();
        match self
            .f
            .with_store(move |s| s.custody_put(&d, &item))
            .await
            .unwrap()
            .unwrap()
        {
            CustodyPut::Accepted(seq) => seq,
            other => panic!("not held: {other:?}"),
        }
    }

    /// `dest`'s items: each custody sequence and whether it is committed.
    async fn items(&self, dest: NodeId) -> Vec<(u64, bool)> {
        let d = dest.to_bytes();
        self.f
            .with_store(move |s| s.custody_fetch(&d, 0, 64))
            .await
            .unwrap()
            .unwrap()
            .into_iter()
            .map(|i| (i.custody_seq, i.committed))
            .collect()
    }

    /// Peers attributed on the hub's link.
    async fn peers(&self) -> usize {
        self.f.status().await.unwrap().links[0].peers
    }
}

/// A raw `tcp-lan` peer speaking as `who` (hello, its announce, then sealed
/// frames by hand) to the fabric whose keys are `to`.
struct Peer {
    s: TcpStream,
    who: Arc<Ident>,
    to: PeerKeys,
}

impl Peer {
    async fn open(addr: SocketAddr, who: &Arc<Ident>, to: PeerKeys) -> Peer {
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(&HELLO).await.unwrap();
        s.write_all(&encode_frame(T_ANNOUNCE, &who.announce_bytes()).unwrap())
            .await
            .unwrap();
        read_hello(&mut s).await.unwrap();
        let (ty, _) = read_frame(&mut s).await.unwrap();
        assert_eq!(ty, T_ANNOUNCE, "the fabric announces first");
        Peer {
            s,
            who: who.clone(),
            to,
        }
    }

    /// Seal one data frame (sealed afresh on every call, as a
    /// retransmission is) and write it.
    async fn send(&mut self, kind: Kind, n: u32, payload: &[u8], frag: Option<FragPos>) {
        let mut inner = Inner::new(&self.who.id(), kind, seq(n), payload.to_vec());
        inner.frag = frag;
        let pkt = seal_envelope(&*self.who, &self.to, Priority::Interactive, &inner).unwrap();
        self.s
            .write_all(&encode_frame(T_SEALED, &pkt).unwrap())
            .await
            .unwrap();
    }

    /// Read what the fabric sends within `wait`: whether an ACK block (a
    /// standalone `ACK` or a piggybacked one) acknowledges `n` as delivered.
    async fn acked(&mut self, n: u32, wait: Duration) -> bool {
        let to = self.to;
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let Ok(Ok((ty, frame))) =
                tokio::time::timeout_at(deadline, read_frame(&mut self.s)).await
            else {
                return false;
            };
            if ty != T_SEALED {
                continue;
            }
            let Ok(o) = open_envelope(&*self.who, &frame, &|_| vec![to]) else {
                continue;
            };
            let block = if o.inner.kind == Kind::Ack {
                AckPayload::decode(&o.inner.payload).ok().map(|p| p.block)
            } else {
                o.inner.ack.clone()
            };
            if block.is_some_and(|b| b.acks(seq(n))) {
                return true;
            }
        }
    }

    /// Read and drop whatever the fabric sends within `wait`.
    async fn drain(&mut self, wait: Duration) {
        let deadline = tokio::time::Instant::now() + wait;
        while let Ok(Ok(_)) = tokio::time::timeout_at(deadline, read_frame(&mut self.s)).await {}
    }
}

/// A node with `pat` as a contact, and `pat`'s connection to it.
async fn setup(dir: &Path, consumer: Option<&Arc<Rec>>) -> (Node, Arc<Ident>, Peer) {
    let pat = Ident::new("pat", 0);
    let node = Node::start(&Ident::new("fabric", 0), dir, consumer).await;
    node.f
        .add_contact_card(&card_text(&pat.announce_bytes()))
        .await
        .unwrap();
    let peer = Peer::open(node.addr, &pat, node.keys()).await;
    (node, pat, peer)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_no_consumer_each_kind_is_acknowledged_counted_and_never_a_row() {
    let d = Dirs::new();
    let (node, pat, mut p) = setup(&d.node("fabric"), None).await;
    for (n, kind) in (1u32..).zip(KINDS) {
        p.send(kind, n, &body(kind, n), None).await;
        assert!(
            p.acked(n, T).await,
            "{kind:?} acknowledged with no consumer"
        );
    }
    assert_eq!(node.unconsumed().await, [1; 5], "one count per kind");
    assert_eq!(node.rows().await, 0, "no message row for any registry kind");
    assert_eq!(node.rx(pat.id()).await.prefix, 5, "every `n` committed");

    // A duplicate is re-acknowledged and counted once.
    node.settle().await;
    p.drain(QUIET).await;
    p.send(Kind::Kb, 2, &body(Kind::Kb, 2), None).await;
    assert!(p.acked(2, T).await, "a duplicate is re-acknowledged");
    assert_eq!(node.unconsumed().await, [1; 5]);

    // A Tier 1 frame: its fragments are kept until the last completes it,
    // then go with it, uncommitted to anyone; it is counted once.
    let whole: Vec<u8> = (0..300u32).map(|i| (i % 239) as u8).collect();
    for idx in 0..3 {
        let part = frag_slice(&whole, 3, idx).unwrap();
        p.send(Kind::Update, 6, part, Some(FragPos { idx, count: 3 }))
            .await;
    }
    assert!(p.acked(6, T).await, "a Tier 1 frame acknowledged whole");
    assert_eq!(
        node.unconsumed().await,
        [1, 1, 2, 1, 1],
        "counted once, not per fragment"
    );
    let rx = node.rx(pat.id()).await;
    assert_eq!(rx.prefix, 6, "its `n` committed");
    assert!(rx.partials.is_empty(), "its fragments went with it");
    assert_eq!(node.rows().await, 0);

    // `MSG` is unchanged: a row.
    p.send(Kind::Msg, 7, &MessageBody::text("hello").encode(), None)
        .await;
    assert!(p.acked(7, T).await);
    assert_eq!(node.rows().await, 1);
    let texts: Vec<String> = inbox(&node.f, pat.id())
        .await
        .into_iter()
        .filter_map(|m| m.text)
        .collect();
    assert_eq!(texts, ["hello"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_consumer_takes_each_new_frame_once_and_never_a_duplicate() {
    let d = Dirs::new();
    let rec = Arc::new(Rec::default());
    let (node, pat, mut p) = setup(&d.node("fabric"), Some(&rec)).await;
    for (n, kind) in (1u32..).zip(KINDS) {
        p.send(kind, n, &body(kind, n), None).await;
        assert!(p.acked(n, T).await, "{kind:?} acknowledged");
    }
    let calls = rec.calls();
    assert_eq!(calls.len(), 5, "{calls:#?}");
    for ((n, kind), c) in (1u32..).zip(KINDS).zip(&calls) {
        assert_eq!(c.kind, kind);
        assert_eq!(c.msg_id, MsgId::new(pat.id(), node.ident.id(), 0, seq(n)));
        assert!(c.attributed, "it came on a connection attributed to pat");
        assert_eq!(c.body, body(kind, n));
    }
    assert_eq!(rec.kept(), 5);
    assert_eq!(node.unconsumed().await, [0; 5], "consumed, not counted");
    assert_eq!(node.rows().await, 0, "no message row for any registry kind");

    // Duplicates: re-acknowledged, never delivered again.
    node.settle().await;
    p.drain(QUIET).await;
    for (n, kind) in (1u32..).zip(KINDS) {
        p.send(kind, n, &body(kind, n), None).await;
        assert!(p.acked(n, T).await, "{kind:?} duplicate re-acknowledged");
    }
    node.settle().await;
    assert_eq!(
        rec.calls().len(),
        5,
        "a duplicate never reaches the consumer"
    );
    assert_eq!(node.rows().await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_fabric_commits_only_after_the_consumer_returned() {
    let d = Dirs::new();
    let dir = d.node("fabric");
    let fabric = Ident::new("fabric", 0);
    let rec = Arc::new(Rec {
        probe: Some((dir.clone(), store_key(&fabric))),
        ..Rec::default()
    });
    let pat = Ident::new("pat", 0);
    let node = Node::start(&fabric, &dir, Some(&rec)).await;
    node.f
        .add_contact_card(&card_text(&pat.announce_bytes()))
        .await
        .unwrap();
    let mut p = Peer::open(node.addr, &pat, node.keys()).await;
    let key = store_key(&fabric);
    for (n, kind) in (1u32..).zip(KINDS) {
        p.send(kind, n, &body(kind, n), None).await;
        assert!(p.acked(n, T).await, "{kind:?} acknowledged");
        assert_eq!(
            committed_prefix(&dir, &key, &pat.id()),
            i64::from(n),
            "committed once acknowledged"
        );
    }
    let calls = rec.calls();
    assert_eq!(calls.len(), 5);
    assert!(
        calls.iter().all(|c| c.committed == Some(false)),
        "while the consumer ran nothing of its frame was committed: {calls:#?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_consumer_leaves_the_frame_unacknowledged_until_it_comes_again() {
    let d = Dirs::new();
    let rec = Arc::new(Rec::default());
    rec.fail.store(true, Ordering::SeqCst);
    let (node, pat, mut p) = setup(&d.node("fabric"), Some(&rec)).await;
    let errors = node.f.status().await.unwrap().counters.errors;
    p.send(Kind::Patch, 1, b"a fix", None).await;
    until(10, || async { (rec.calls().len() == 1).then_some(()) }).await;
    node.settle().await;
    assert!(!p.acked(1, QUIET).await, "not acknowledged");
    assert_eq!(node.rx(pat.id()).await.prefix, 0, "not committed");
    let c = node.f.status().await.unwrap().counters;
    assert_eq!(c.errors, errors + 1, "the failure is counted");
    assert_eq!(c.unconsumed_patch, 0);
    assert_eq!(rec.kept(), 0);

    // The consumer can write again, and the sender's retransmission comes.
    rec.fail.store(false, Ordering::SeqCst);
    p.send(Kind::Patch, 1, b"a fix", None).await;
    assert!(p.acked(1, T).await, "redelivered and acknowledged");
    assert_eq!(rec.calls().len(), 2, "the retransmission reached it again");
    assert_eq!(rec.kept(), 1);
    assert_eq!(node.rx(pat.id()).await.prefix, 1);
    assert_eq!(node.rows().await, 0);
}

/// The other half of the `Consumer` contract: a body the consumer refuses
/// (one it cannot parse) is answered `Ok` with nothing kept, never with an
/// error (which would bring it back on every retransmission).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_the_consumer_refuses_is_taken_with_ok_and_holds_nothing_behind_it() {
    let d = Dirs::new();
    let rec = Arc::new(Rec::default());
    let (node, pat, mut p) = setup(&d.node("fabric"), Some(&rec)).await;
    let errors = node.f.status().await.unwrap().counters.errors;
    let mut bad = REFUSED.to_vec();
    bad.extend_from_slice(&[0xFF; 9]);
    p.send(Kind::AgentOp, 1, &bad, None).await;
    assert!(p.acked(1, T).await, "refused with Ok: acknowledged");
    p.send(Kind::AgentOp, 2, b"an op", None).await;
    assert!(p.acked(2, T).await, "the frame after it is taken");
    assert_eq!(rec.calls().len(), 2);
    assert_eq!(rec.kept(), 1, "the refused body kept nothing");
    assert_eq!(node.rx(pat.id()).await.prefix, 2, "nothing waits on it");
    let c = node.f.status().await.unwrap().counters;
    assert_eq!((c.errors, c.unconsumed_agent_op), (errors, 0));
    assert_eq!(node.rows().await, 0);
}

/// Through the mailbox (L5 §9: a `MAILBOX_COMMIT` says the member stored
/// everything up to it). Its hub holds two items from `pat` for a member:
/// a `PATCH`, then a `MSG`. The consumer fails on the `PATCH`: the member's
/// cursor stays below it (the `MSG` after it is received but moves nothing),
/// so the hub keeps both uncommitted; at the next link-up's fetch the hub
/// streams the `PATCH` again and the consumer takes it. Each call says the
/// frame came unattributed (a mailbox item).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn through_the_mailbox_a_frame_not_taken_stays_at_the_hub_and_comes_again() {
    let d = Dirs::new();
    let hub = Hub::start(&d.node("hub")).await;
    let rec = Arc::new(Rec::default());
    let m = Node::member(&Ident::new("member", 0), &d.node("member"), &rec, &hub).await;
    let pat = Ident::new("pat", 0);
    m.f.add_contact_card(&card_text(&pat.announce_bytes()))
        .await
        .unwrap();
    // The member goes away; its hub takes the two items for it.
    m.t.release(hub.addr);
    until(10, || async { (hub.peers().await == 0).then_some(()) }).await;
    let me = m.ident.id();
    let s1 = hub
        .hold(me, sealed(&pat, &m.keys(), Kind::Patch, 1, b"a fix"))
        .await;
    let text = MessageBody::text("after it").encode();
    let s2 = hub
        .hold(me, sealed(&pat, &m.keys(), Kind::Msg, 2, &text))
        .await;
    assert!(s1 < s2);

    // Back: the link-up fetch streams both, and the consumer fails.
    rec.fail.store(true, Ordering::SeqCst);
    m.t.maintain(hub.addr, Some(hub.ident.id()));
    until(10, || async { (m.rows().await == 1).then_some(()) }).await;
    m.settle().await;
    let calls = rec.calls();
    assert_eq!(calls.len(), 1, "{calls:#?}");
    assert_eq!(calls[0].kind, Kind::Patch);
    assert_eq!(calls[0].body, b"a fix");
    assert!(!calls[0].attributed, "a mailbox item is not attributed");
    assert_eq!(rec.kept(), 0);
    assert_eq!(m.rx(pat.id()).await.prefix, 0, "the PATCH is not committed");
    assert!(
        m.cursor(hub.ident.id()).await < s1,
        "the cursor stays below the PATCH, and the MSG after it moves nothing"
    );
    assert_eq!(hub.items(me).await, [(s1, false), (s2, false)]);

    // A disconnect, and the next link-up: the hub streams the PATCH again.
    rec.fail.store(false, Ordering::SeqCst);
    m.t.release(hub.addr);
    until(10, || async { (hub.peers().await == 0).then_some(()) }).await;
    m.t.maintain(hub.addr, Some(hub.ident.id()));
    until(10, || async { (rec.kept() == 1).then_some(()) }).await;
    until(10, || async {
        (hub.items(me).await == [(s1, true), (s2, true)]).then_some(())
    })
    .await;
    assert_eq!(m.cursor(hub.ident.id()).await, s2);
    let calls = rec.calls();
    assert_eq!(calls.len(), 2, "the hub's second stream reached it again");
    assert_eq!(calls[1].body, b"a fix");
    assert!(!calls[1].attributed);
    assert_eq!(m.rx(pat.id()).await.prefix, 2);
    assert_eq!(m.rows().await, 1, "the MSG once; the PATCH never a row");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tier1_frame_reaches_the_consumer_whole_and_its_last_fragment_waits_for_it() {
    let d = Dirs::new();
    let rec = Arc::new(Rec::default());
    let (node, pat, mut p) = setup(&d.node("fabric"), Some(&rec)).await;
    let whole: Vec<u8> = (0..300u32).map(|i| (i % 251) as u8).collect();
    let frag = |idx: u8| {
        let part = frag_slice(&whole, 3, idx).unwrap().to_vec();
        (part, Some(FragPos { idx, count: 3 }))
    };
    for idx in 0..2 {
        let (part, pos) = frag(idx);
        p.send(Kind::Kb, 1, &part, pos).await;
    }
    // The consumer fails on the completing fragment.
    rec.fail.store(true, Ordering::SeqCst);
    let (part, pos) = frag(2);
    p.send(Kind::Kb, 1, &part, pos).await;
    until(10, || async { (rec.calls().len() == 1).then_some(()) }).await;
    assert_eq!(rec.calls()[0].body, whole, "the consumer reads it whole");
    node.settle().await;
    assert!(!p.acked(1, QUIET).await, "not acknowledged");
    let rx = node.rx(pat.id()).await;
    assert_eq!(rx.prefix, 0);
    assert_eq!(
        rx.partials,
        [Partial {
            n: 1,
            frag_count: 3,
            bitmap: 0b011
        }],
        "the completing fragment went with the rollback"
    );

    // The sender retransmits the fragment it holds no SACK for.
    rec.fail.store(false, Ordering::SeqCst);
    let (part, pos) = frag(2);
    p.send(Kind::Kb, 1, &part, pos).await;
    assert!(p.acked(1, T).await, "delivered and acknowledged");
    let calls = rec.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].body, whole);
    assert_eq!(rec.kept(), 1);
    let rx = node.rx(pat.id()).await;
    assert_eq!(rx.prefix, 1);
    assert!(rx.partials.is_empty(), "the fragments went with the record");
    assert_eq!(node.rows().await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_registry_kinds_take_a_consumer() {
    let d = Dirs::new();
    let f = start(
        &Ident::new("fabric", 0),
        &d.node("fabric"),
        FabricSettings::default(),
    );
    let rec: Arc<dyn Consumer> = Arc::new(Rec::default());
    for kind in Kind::ALL {
        match f.set_consumer(kind, rec.clone()).await {
            Ok(()) => assert!(KINDS.contains(&kind), "{kind:?} took a consumer"),
            Err(FabricError::Invalid(_)) => {
                assert!(!KINDS.contains(&kind), "{kind:?} refused")
            }
            Err(e) => panic!("{kind:?}: {e}"),
        }
    }
}

/// The fault point (debug builds only, as the hook is): a process killed
/// after the consumer's write and before the fabric's commit.
#[cfg(debug_assertions)]
mod killed {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_kill_after_the_consumer_redelivers_at_the_next_connection_one_record_kept() {
        let d = Dirs::new();
        let dir = d.node("fabric");
        let rec = Arc::new(Rec::default());
        let (node, pat, mut p) = setup(&dir, Some(&rec)).await;
        node.f.fault_after_consume().await.unwrap();
        p.send(Kind::AgentOp, 1, b"an op", None).await;
        until(10, || async { (rec.kept() == 1).then_some(()) }).await;
        node.settle().await;
        assert!(!p.acked(1, QUIET).await, "killed before the commit: no ACK");
        assert_eq!(node.rx(pat.id()).await.prefix, 0, "nothing committed");
        node.kill();
        drop(p);

        // The next start and the next connection: the sender retransmits.
        let node = Node::start(&node.ident, &dir, Some(&rec)).await;
        let mut p = Peer::open(node.addr, &pat, node.keys()).await;
        p.send(Kind::AgentOp, 1, b"an op", None).await;
        assert!(p.acked(1, T).await, "redelivered and acknowledged");
        assert_eq!(rec.calls().len(), 2, "the consumer saw the frame again");
        assert_eq!(rec.kept(), 1, "idempotent by msg_id: one record");
        assert_eq!(node.rx(pat.id()).await.prefix, 1);
        assert_eq!(node.rows().await, 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_kill_after_a_tier1_delivery_leaves_no_row_at_the_restart() {
        let d = Dirs::new();
        let dir = d.node("fabric");
        let rec = Arc::new(Rec::default());
        let (node, pat, mut p) = setup(&dir, Some(&rec)).await;
        let whole: Vec<u8> = (0..300u32).map(|i| (i % 241) as u8).collect();
        let frag = |idx: u8| {
            let part = frag_slice(&whole, 3, idx).unwrap().to_vec();
            (part, Some(FragPos { idx, count: 3 }))
        };
        for idx in 0..2 {
            let (part, pos) = frag(idx);
            p.send(Kind::Patch, 1, &part, pos).await;
        }
        node.f.fault_after_consume().await.unwrap();
        let (part, pos) = frag(2);
        p.send(Kind::Patch, 1, &part, pos).await;
        until(10, || async { (rec.kept() == 1).then_some(()) }).await;
        node.settle().await;
        assert!(!p.acked(1, QUIET).await, "killed before the commit: no ACK");
        node.kill();
        drop(p);

        // The restart finishes complete partials as messages: none is left.
        let node = Node::start(&node.ident, &dir, Some(&rec)).await;
        assert_eq!(node.rows().await, 0, "no row at the restart");
        assert_eq!(node.rx(pat.id()).await.partials.len(), 1);
        let mut p = Peer::open(node.addr, &pat, node.keys()).await;
        let (part, pos) = frag(2);
        p.send(Kind::Patch, 1, &part, pos).await;
        assert!(p.acked(1, T).await, "redelivered whole and acknowledged");
        let calls = rec.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].body, whole);
        assert_eq!(rec.kept(), 1, "one record");
        assert_eq!(node.rows().await, 0);
    }

    /// The same through the mailbox: the member killed after its consumer's
    /// write kept its stored cursor below the item, so its next start's
    /// fetch brings the item again.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_kill_after_the_consumer_on_a_mailbox_item_redelivers_it_at_the_next_start() {
        let d = Dirs::new();
        let hub = Hub::start(&d.node("hub")).await;
        let rec = Arc::new(Rec::default());
        let ident = Ident::new("member", 0);
        let dir = d.node("member");
        let m = Node::member(&ident, &dir, &rec, &hub).await;
        let pat = Ident::new("pat", 0);
        m.f.add_contact_card(&card_text(&pat.announce_bytes()))
            .await
            .unwrap();
        m.t.release(hub.addr);
        until(10, || async { (hub.peers().await == 0).then_some(()) }).await;
        let me = ident.id();
        let s1 = hub
            .hold(me, sealed(&pat, &m.keys(), Kind::Update, 1, b"an update"))
            .await;
        m.f.fault_after_consume().await.unwrap();
        m.t.maintain(hub.addr, Some(hub.ident.id()));
        until(10, || async { (rec.kept() == 1).then_some(()) }).await;
        m.settle().await;
        assert!(
            m.cursor(hub.ident.id()).await < s1,
            "killed before the commit: the stored cursor stays below it"
        );
        m.kill();

        let m = Node::member_again(&ident, &dir, &rec, &hub).await;
        until(10, || async {
            (hub.items(me).await == [(s1, true)]).then_some(())
        })
        .await;
        assert_eq!(rec.calls().len(), 2, "the next start's fetch brought it");
        assert_eq!(rec.kept(), 1, "idempotent by msg_id: one record");
        assert_eq!(m.cursor(hub.ident.id()).await, s1);
        assert_eq!(m.rx(pat.id()).await.prefix, 1);
        assert_eq!(m.rows().await, 0);
    }
}
