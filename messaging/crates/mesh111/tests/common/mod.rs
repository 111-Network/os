//! Shared harness for the in-process multi-node tests (§20 G1 shapes, no
//! netns): a real-crypto identity handle over `id111` and node helpers.
//!
//! [`Ident`] implements [`Sealer`] with the real L2 `seal_payload` /
//! `open_payload`, the real static-static pair key, Ed25519 signing, a
//! per-peer send counter, a 32-entry receive window per peer for the A11
//! freshness verdict, and an announce counter that survives "restarts" (the
//! identity dir outlives the store). Every sealed packet is logged so replay
//! tests can resend one byte-for-byte.

#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use id111::announce::Announce;
use id111::counters::ReceiveWindow;
use id111::keyset::{Keyset, NodeId};
use id111::seal::{OpenError, SealHeader};
use mesh111::envelope::{blake2b_256, derive_pair_key};
use mesh111::fabric::{Fabric, FabricEvent, FabricSettings, MessageView};
use mesh111::keys::StoreKey;
use mesh111::link::lossy::{self, EndSpec, LossyConfig, LossyEnd};
use mesh111::link::tcp_lan::{TcpLan, TcpLanConfig};
use mesh111::link::{Dialer, LinkId};
use mesh111::sealer::{Direction, Freshness, Key32, Sealer, SealerError};
use mesh111::store::{MsgState, OpenParams, Store};

/// `appdata` v1: `[ver=1][roles u16 LE][reserved]` (A8).
pub fn appdata(roles: u16) -> Vec<u8> {
    let r = roles.to_le_bytes();
    vec![1, r[0], r[1], 0]
}

/// A test identity holder with real crypto.
pub struct Ident {
    keyset: Keyset,
    announce: Vec<u8>,
    send: Mutex<HashMap<NodeId, u64>>,
    windows: Mutex<HashMap<NodeId, ReceiveWindow>>,
    announce_counter: Mutex<u64>,
    restore_pending: AtomicBool,
    log: Mutex<Vec<(Key32, Vec<u8>)>>,
    forgotten: Mutex<Vec<Key32>>,
}

impl Ident {
    pub fn new(label: &str, roles: u16) -> Arc<Ident> {
        Ident::with_appdata(label, appdata(roles))
    }

    /// An identity whose announce carries `appdata` verbatim (e.g. a
    /// home-hub hint after the roles).
    pub fn with_appdata(label: &str, appdata: Vec<u8>) -> Arc<Ident> {
        let mut b = [0u8; 64];
        b[..32].copy_from_slice(&blake2b_256(&[b"f1-sign", label.as_bytes()]));
        b[32..].copy_from_slice(&blake2b_256(&[b"f1-ecdh", label.as_bytes()]));
        let keyset = Keyset::from_bytes(&b).expect("test keyset");
        let announce = Announce::create(&keyset, 1, &appdata)
            .expect("announce")
            .encode();
        Arc::new(Ident {
            keyset,
            announce,
            send: Mutex::new(HashMap::new()),
            windows: Mutex::new(HashMap::new()),
            announce_counter: Mutex::new(1),
            restore_pending: AtomicBool::new(false),
            log: Mutex::new(Vec::new()),
            forgotten: Mutex::new(Vec::new()),
        })
    }

    pub fn id(&self) -> NodeId {
        self.keyset.node_id()
    }

    pub fn enc(&self) -> Key32 {
        self.keyset.x25519_public()
    }

    pub fn announce_bytes(&self) -> Vec<u8> {
        self.announce.clone()
    }

    /// Simulate the identity holder's `RestoreFloorRequired`.
    pub fn set_restore_pending(&self, v: bool) {
        self.restore_pending.store(v, Ordering::SeqCst);
    }

    /// Every packet this identity sealed to `recipient` (oldest first).
    pub fn sealed_to(&self, recipient: &Key32) -> Vec<Vec<u8>> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|(r, _)| r == recipient)
            .map(|(_, p)| p.clone())
            .collect()
    }

    pub fn forgotten(&self) -> Vec<Key32> {
        self.forgotten.lock().unwrap().clone()
    }
}

impl Sealer for Ident {
    fn node_id(&self) -> NodeId {
        self.keyset.node_id()
    }

    fn own_announce(&self) -> Vec<u8> {
        self.announce.clone()
    }

    fn seal_payload(
        &self,
        recipient_enc_pubkey: &Key32,
        header: &SealHeader,
        inner: &mut dyn FnMut(&Key32) -> Vec<u8>,
    ) -> Result<Vec<u8>, SealerError> {
        let p = id111::seal::seal_payload(recipient_enc_pubkey, header, |epk| inner(epk))
            .map_err(SealerError::Seal)?;
        self.log
            .lock()
            .unwrap()
            .push((*recipient_enc_pubkey, p.clone()));
        Ok(p)
    }

    fn open_payload(&self, packet: &[u8]) -> Result<(SealHeader, Key32, Vec<u8>), SealerError> {
        id111::seal::open_payload(&self.keyset, packet).map_err(SealerError::Open)
    }

    fn pair_key(&self, peer: &Key32, direction: Direction) -> Result<Key32, SealerError> {
        let ss = self
            .keyset
            .ecdh(peer)
            .map_err(|e| SealerError::Open(OpenError::Ecdh(e)))?;
        let own = self.enc();
        let key = match direction {
            Direction::Outbound => derive_pair_key(&ss, &own, peer),
            Direction::Inbound => derive_pair_key(&ss, peer, &own),
        };
        Ok(*key)
    }

    fn forget_pair_key(&self, peer: &Key32) {
        self.forgotten.lock().unwrap().push(*peer);
    }

    fn sign(&self, tag: &[u8], msg: &[u8]) -> [u8; 64] {
        self.keyset.sign_with_tag(tag, msg)
    }

    fn next_send_counter(&self, peer: &NodeId) -> Result<u64, SealerError> {
        let mut m = self.send.lock().unwrap();
        let c = m.entry(*peer).or_insert(0);
        *c += 1;
        Ok(*c)
    }

    fn observe_counter(&self, peer: &NodeId, counter: u64) -> Result<Freshness, SealerError> {
        let mut m = self.windows.lock().unwrap();
        let w = m.entry(*peer).or_default();
        Ok(match w.offer(counter) {
            Ok(()) => Freshness::Fresh,
            Err(_) => Freshness::Stale,
        })
    }

    fn next_announce_counter(&self) -> Result<u64, SealerError> {
        if self.restore_pending.load(Ordering::SeqCst) {
            return Err(SealerError::Counters("RestoreFloorRequired".into()));
        }
        let mut a = self.announce_counter.lock().unwrap();
        *a += 1;
        Ok(*a)
    }

    fn announce_last(&self) -> u64 {
        *self.announce_counter.lock().unwrap()
    }

    fn random(&self, out: &mut [u8]) -> Result<(), SealerError> {
        getrandom::getrandom(out).map_err(|_| SealerError::Entropy)
    }

    fn store_secret(&self) -> Result<Key32, SealerError> {
        Ok(blake2b_256(&[b"f1-store", self.node_id().as_bytes()]))
    }
}

/// Serializes store opens within one test process: a rare SQLCipher
/// failure was seen once when several tests opened their first stores at
/// the same instant under heavy load (reported to the lead; the daemon
/// opens one store per process).
static OPEN: Mutex<()> = Mutex::new(());

/// Open (or reopen) a node's store and start its fabric.
pub fn start(ident: &Arc<Ident>, dir: &Path, settings: FabricSettings) -> Fabric {
    let key = StoreKey::from_bytes(blake2b_256(&[b"f1-key", ident.id().as_bytes()]));
    let _one_at_a_time = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    let (store, decision) = Store::open(
        dir,
        &key,
        &OpenParams {
            announce_last: ident.announce_last(),
            restore_floor_set: false,
        },
    )
    .expect("store opens");
    Fabric::start(ident.clone() as Arc<dyn Sealer>, store, decision, settings).expect("fabric")
}

/// A `tcp-lan` adapter for tests: loopback allowed, fast back-off.
pub fn tcp(fabric: &Fabric, ident: &Ident, name: &str) -> TcpLan {
    let mut cfg = TcpLanConfig::new(ident.announce_bytes());
    cfg.refuse_loopback = false;
    cfg.backoff_min = Duration::from_millis(30);
    cfg.backoff_max = Duration::from_millis(150);
    cfg.handshake_timeout = Duration::from_secs(3);
    TcpLan::new(LinkId::new(name), cfg, fabric.link_events(), fabric.gate())
}

/// Register a `tcp-lan` adapter; `with_dialer` lets the fabric dial its hub.
pub async fn add_tcp(fabric: &Fabric, t: &TcpLan, with_dialer: bool) {
    let link = Arc::new(t.clone());
    let dialer: Option<Arc<dyn Dialer>> = if with_dialer {
        Some(Arc::new(t.clone()))
    } else {
        None
    };
    fabric.add_link(link, dialer).await.expect("add link");
}

/// Listen on `127.0.0.1:port` (0 = ephemeral), retrying a busy port briefly
/// (a restarted hub rebinds its old port).
pub async fn listen(t: &TcpLan, port: u16) -> SocketAddr {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    for _ in 0..100 {
        match t.listen(addr).await {
            Ok(a) => return a,
            Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    panic!("could not bind {addr}");
}

/// Pair two fabrics over a lossy in-process link.
pub async fn lossy_pair(
    a: (&Fabric, &Ident),
    b: (&Fabric, &Ident),
    cfg_a: LossyConfig,
    cfg_b: LossyConfig,
) -> (Arc<LossyEnd>, Arc<LossyEnd>) {
    let (ea, eb) = lossy::pair(
        EndSpec {
            id: LinkId::new("lossy-a"),
            announce: a.1.announce_bytes(),
            events: a.0.link_events(),
            cfg: cfg_a,
        },
        EndSpec {
            id: LinkId::new("lossy-b"),
            announce: b.1.announce_bytes(),
            events: b.0.link_events(),
            cfg: cfg_b,
        },
    );
    a.0.add_link(ea.clone(), None).await.expect("add a");
    b.0.add_link(eb.clone(), None).await.expect("add b");
    (ea, eb)
}

/// Exchange cards (both sides `oob`).
pub async fn exchange_cards(a: &Fabric, b: &Fabric) {
    let ca = a.card().await.unwrap().text;
    let cb = b.card().await.unwrap().text;
    a.add_contact_card(&cb).await.expect("a adds b");
    b.add_contact_card(&ca).await.expect("b adds a");
}

/// Poll `f` until it returns `Some`, up to `secs`.
pub async fn until<T, F, Fut>(secs: u64, mut f: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("condition not reached within {secs}s");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

/// Wait until message `id` reaches `state`.
pub async fn until_state(f: &Fabric, id: i64, state: MsgState, secs: u64) -> MessageView {
    until(secs, || async {
        let m = f.message(id).await.ok()?;
        (m.state == state).then_some(m)
    })
    .await
}

/// Incoming messages from `peer`, in thread order.
pub async fn inbox(f: &Fabric, peer: NodeId) -> Vec<MessageView> {
    match f.thread(peer, 10_000).await {
        Ok(t) => t
            .messages
            .into_iter()
            .filter(|m| m.dir == mesh111::store::Dir::In)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Collect every message event for `id` seen on `rx` so far (non-blocking).
pub fn drain_events(rx: &mut tokio::sync::broadcast::Receiver<FabricEvent>) -> Vec<FabricEvent> {
    let mut out = Vec::new();
    while let Ok(e) = rx.try_recv() {
        out.push(e);
    }
    out
}

/// A scratch directory per node.
pub struct Dirs {
    pub root: tempfile::TempDir,
}

impl Dirs {
    pub fn new() -> Dirs {
        Dirs {
            root: tempfile::tempdir().expect("tempdir"),
        }
    }

    pub fn node(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }
}

/// A hub's settings.
pub fn hub_settings() -> FabricSettings {
    FabricSettings {
        hub_role: true,
        ..FabricSettings::default()
    }
}
