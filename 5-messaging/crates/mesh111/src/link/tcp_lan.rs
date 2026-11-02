//! §8.1 — the `tcp-lan` wire (normative) and its adapter. Owner: F1.
//!
//! ```text
//! both sides:  hello = "111F" ‖ 0x01
//!              then frames: type(1) ‖ len(2 BE) ‖ body
//! types:       0x01 sealed packet · 0x02 announce (cleartext) · 0x03 keepalive (empty)
//!              0x04 card request (32 B NodeId) · 0x05 chunk (reserved, §5 Tier 2)
//! ```
//!
//! Rules implemented here:
//! - **Announce first.** Each side writes its hello and then its own
//!   announce (stored bytes, never minted) before anything else; the first
//!   frame read must be the peer's announce. It is parsed and its signature
//!   verified (`id111` strict predicate) and then offered to the fabric's
//!   [`AnnounceGate`] (pinned-keys conflict check, §7 stale rule: identical
//!   keys at any counter are admitted). A bad signature, a conflict, or any
//!   other first frame drops the connection. Later announce frames (card
//!   answers, §7.2 (2)) are signature-checked here too.
//! - **Hello and announce authenticate nothing.** This adapter only reports
//!   the connection ([`LinkEvent::Connected`]) and the announce; attribution
//!   to a NodeId is the fabric's, after a `pair_mac`-verified packet the L2
//!   window judged fresh (§8.1, §4).
//! - **Length caps** are checked from the 3-byte header **before** the body
//!   is read: sealed ≤ 65,535, announce 137–169, keepalive 0, card request
//!   32; an unknown type or a cap violation drops the connection. Chunk
//!   frames (reserved for v1.1) are read and discarded, never processed.
//! - **Keepalive** is a transport detail: an empty 0x03 frame every
//!   `keepalive`; a connection silent for three intervals is closed. The
//!   protocol above (fabric) has no timers.
//! - **Reconnect** on link events with exponential back-off for addresses
//!   the fabric asks to [`Dialer::maintain`]. A dial's `expected` NodeId is
//!   checked against the peer's first announce inside the handshake: a
//!   mismatch (a stale hub address, a mistyped enrollment address) is a
//!   failed attempt — the back-off grows and no connection is reported —
//!   so a misdial never becomes a redial loop with full handshakes.
//! - **Back-pressure, not buffering:** frames read from every connection
//!   wait in one bounded queue (`inbound_queue`) for [`Link::recv`]; a full
//!   queue stops the readers, so TCP flow control pushes back on a peer that
//!   sends faster than the fabric opens (the fabric's own frame queue is
//!   bounded too). Memory held per adapter is at most `inbound_queue` frames
//!   plus one frame per connection reader.
//! - **Loopback is never a peer** (§2, §8.1): with `refuse_loopback` (the
//!   default) inbound connections from, and dials to, loopback addresses are
//!   refused. The listener is bound only where the caller says (the fabric
//!   settings: hub role or LAN sharing) — [`TcpLan::listen`].
//! - **Capacity**, not policy: at most `max_connections` connections (file
//!   descriptors are finite); a full table refuses new ones.
//! - **The stream seam** (Layer 7 §4.4; Layer 7 §19 code seam 1):
//!   [`TcpLanConfig::wrap`] is `None` by default, and then every byte on the
//!   wire is what this module always wrote. With a [`StreamWrap`] the host
//!   wraps each accepted or dialed `TcpStream` before the hello (node111
//!   runs paired-hub links inside TLS 1.3 with raw public keys there; this
//!   crate gains no crypto dependency). The 111 wire inside the wrapped
//!   stream is unchanged. An inbound wrap may name the pair whose pinned key
//!   the peer presented (S7): a first announce naming another NodeId ends
//!   the connection inside the handshake, before any byte beyond the hello
//!   and the own announce, rather than waiting for `pair_mac` attribution.
//!   The wrap's own handshake counts against `handshake_timeout` and the
//!   connection table, like the hello.
//!
//! The stream leaks `dest_hint`, counters and sizes to an on-path observer
//! (§18 rule 4): no transport encryption in this crate (a host wrap may add
//! it, Layer 7 §4.1); content, sender and kind are sealed.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use futures::future::BoxFuture;
use id111::announce::{self, Announce};
use id111::keyset::NodeId;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinHandle;

use super::{
    AnnounceGate, Dialer, EventSink, Frame, FrameType, Link, LinkError, LinkEvent, LinkId,
    LinkKind, LinkState, PathProfile, PeerAddr,
};

/// §2: the fabric `tcp-lan` port.
pub const DEFAULT_PORT: u16 = 11183;
/// §8.1 hello: `"111F" ‖ 0x01`.
pub const HELLO: [u8; 5] = [b'1', b'1', b'1', b'F', 0x01];
/// Frame type 0x01 — sealed packet.
pub const T_SEALED: u8 = 0x01;
/// Frame type 0x02 — announce (cleartext).
pub const T_ANNOUNCE: u8 = 0x02;
/// Frame type 0x03 — keepalive (empty).
pub const T_KEEPALIVE: u8 = 0x03;
/// Frame type 0x04 — card request (32 B NodeId).
pub const T_CARD_REQUEST: u8 = 0x04;
/// Frame type 0x05 — chunk (reserved, §5 Tier 2, v1.1).
pub const T_CHUNK: u8 = 0x05;
/// `type(1) ‖ len(2 BE)`.
pub const FRAME_HEADER_LEN: usize = 3;
/// Largest frame body (the 2-byte length field).
pub const MAX_BODY: usize = 65_535;
/// Card request body: one NodeId.
pub const CARD_REQUEST_LEN: usize = 32;

/// A duplex byte stream a [`StreamWrap`] hands back (Layer 7 §4.4): any
/// `AsyncRead + AsyncWrite + Send + Unpin` type, behind a box so the trait
/// stays object-safe.
pub trait LinkStream: AsyncRead + AsyncWrite + Send + Unpin {}

impl<T: AsyncRead + AsyncWrite + Send + Unpin + ?Sized> LinkStream for T {}

/// A boxed [`LinkStream`].
pub type BoxedStream = Box<dyn LinkStream>;

/// The `tcp-lan` stream seam (Layer 7 §4.4; §19 code seam 1): the host
/// wraps each connection's `TcpStream` before the §8.1 hello is written.
///
/// Both calls run inside the adapter's handshake: they count against
/// `handshake_timeout` and the connection table. An `Err` ends the
/// connection before any 111 byte (a dial counts it as a failed attempt).
/// A wrap may hand the stream back unwrapped (a member, a LAN pair: Layer 7
/// §4.1 rules 1 and 3); the choice is the host's.
pub trait StreamWrap: Send + Sync {
    /// A dialed connection to `addr`, where the dialer expects `expected`
    /// ([`Dialer::maintain`]; `None` when the dial names no NodeId).
    fn outbound(
        &self,
        addr: SocketAddr,
        expected: Option<NodeId>,
        stream: TcpStream,
    ) -> BoxFuture<'_, io::Result<BoxedStream>>;

    /// An accepted connection from `remote`. The `NodeId` is the pair whose
    /// pinned key the peer presented (S7), `None` for a plain peer; when
    /// `Some`, a first announce naming another NodeId is refused.
    fn inbound(
        &self,
        remote: SocketAddr,
        stream: TcpStream,
    ) -> BoxFuture<'_, io::Result<(BoxedStream, Option<NodeId>)>>;
}

/// Adapter settings. Every limit is capacity or transport, never policy.
#[derive(Clone)]
pub struct TcpLanConfig {
    /// This node's own announce (stored bytes, §7), sent first on every
    /// connection.
    pub own_announce: Vec<u8>,
    /// Connection table bound (capacity).
    pub max_connections: usize,
    /// Keepalive interval (transport detail, §8.1).
    pub keepalive: Duration,
    /// Hello + first announce must complete within this.
    pub handshake_timeout: Duration,
    /// §2/§8.1: refuse loopback peers (inbound and dials). Tests that run
    /// several nodes on one host turn it off.
    pub refuse_loopback: bool,
    /// First reconnect delay.
    pub backoff_min: Duration,
    /// Reconnect delay cap.
    pub backoff_max: Duration,
    /// Frames queued per connection before `send` reports back-pressure.
    pub write_queue: usize,
    /// Frames read from all connections and not yet taken by `recv`
    /// (capacity: a full queue stops the readers — module doc).
    pub inbound_queue: usize,
    /// The stream seam (Layer 7 §4.4): `None` (the default) leaves every
    /// connection a plain `TcpStream`, byte-identical with the adapter
    /// before the seam.
    pub wrap: Option<Arc<dyn StreamWrap>>,
}

impl std::fmt::Debug for TcpLanConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TcpLanConfig")
            .field("own_announce_len", &self.own_announce.len())
            .field("max_connections", &self.max_connections)
            .field("keepalive", &self.keepalive)
            .field("handshake_timeout", &self.handshake_timeout)
            .field("refuse_loopback", &self.refuse_loopback)
            .field("backoff_min", &self.backoff_min)
            .field("backoff_max", &self.backoff_max)
            .field("write_queue", &self.write_queue)
            .field("inbound_queue", &self.inbound_queue)
            .field("wrap", &self.wrap.is_some())
            .finish()
    }
}

impl TcpLanConfig {
    /// Defaults around `own_announce`.
    pub fn new(own_announce: Vec<u8>) -> Self {
        TcpLanConfig {
            own_announce,
            max_connections: 256,
            keepalive: Duration::from_secs(25),
            handshake_timeout: Duration::from_secs(10),
            refuse_loopback: true,
            backoff_min: Duration::from_millis(500),
            backoff_max: Duration::from_secs(60),
            write_queue: 1024,
            inbound_queue: 256,
            wrap: None,
        }
    }
}

/// Wire-level failures (each drops the connection).
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    /// Transport.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Not `"111F" ‖ 0x01`.
    #[error("bad hello")]
    BadHello,
    /// A frame type outside 0x01–0x05.
    #[error("unknown frame type 0x{0:02x}")]
    UnknownType(u8),
    /// A length outside the type's cap.
    #[error("frame type 0x{ty:02x} with length {len}")]
    Length {
        /// Type.
        ty: u8,
        /// Declared length.
        len: usize,
    },
    /// The first frame was not an announce.
    #[error("first frame is not an announce")]
    NotAnnounceFirst,
    /// The announce failed to parse or verify, or the gate refused it.
    #[error("announce refused")]
    AnnounceRefused,
    /// A dial was answered by another node than the one expected there, or
    /// a wrapped peer's first announce names another NodeId than the pair
    /// its pinned key matched (Layer 7 §4.4, S7).
    #[error("the peer is not the node this connection was opened for")]
    ClaimMismatch,
}

/// Check a declared frame length against its type's cap (before the body
/// is read).
pub fn check_frame_len(ty: u8, len: usize) -> Result<(), WireError> {
    let ok = match ty {
        T_SEALED => (1..=MAX_BODY).contains(&len),
        T_ANNOUNCE => (announce::BASE_LEN..=announce::MAX_LEN).contains(&len),
        T_KEEPALIVE => len == 0,
        T_CARD_REQUEST => len == CARD_REQUEST_LEN,
        T_CHUNK => len <= MAX_BODY,
        other => return Err(WireError::UnknownType(other)),
    };
    if ok {
        Ok(())
    } else {
        Err(WireError::Length { ty, len })
    }
}

/// Encode one frame: `type ‖ len(2 BE) ‖ body`, caps checked.
pub fn encode_frame(ty: u8, body: &[u8]) -> Result<Vec<u8>, WireError> {
    check_frame_len(ty, body.len())?;
    let mut out = Vec::with_capacity(FRAME_HEADER_LEN + body.len());
    out.push(ty);
    out.extend_from_slice(&(body.len() as u16).to_be_bytes()); // ≤ 65,535, checked
    out.extend_from_slice(body);
    Ok(out)
}

/// Read one frame; the length cap is enforced before the body is read.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<(u8, Vec<u8>), WireError> {
    let mut h = [0u8; FRAME_HEADER_LEN];
    r.read_exact(&mut h).await?;
    let ty = h[0];
    let len = usize::from(u16::from_be_bytes([h[1], h[2]]));
    check_frame_len(ty, len)?;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    Ok((ty, body))
}

/// Read and check the peer's hello.
pub async fn read_hello<R: AsyncRead + Unpin>(r: &mut R) -> Result<(), WireError> {
    let mut h = [0u8; HELLO.len()];
    r.read_exact(&mut h).await?;
    if h != HELLO {
        return Err(WireError::BadHello);
    }
    Ok(())
}

/// Parse and verify an announce body (signature under the strict
/// predicate). Pure: no pin state is touched here.
pub fn verify_announce(body: &[u8]) -> Option<Announce> {
    let a = Announce::parse(body).ok()?;
    a.verify().ok()?;
    Some(a)
}

fn frame_wire_type(f: &Frame) -> Result<u8, LinkError> {
    match f.frame_type {
        FrameType::Sealed => Ok(T_SEALED),
        FrameType::Announce => Ok(T_ANNOUNCE),
        FrameType::CardRequest => Ok(T_CARD_REQUEST),
        FrameType::Chunk | FrameType::Foreign => Err(LinkError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "frame type not carried by tcp-lan v1",
        ))),
    }
}

fn is_loopback(addr: &SocketAddr) -> bool {
    addr.ip().to_canonical().is_loopback()
}

struct ConnHandle {
    tx: mpsc::Sender<Vec<u8>>,
    close: Arc<Notify>,
}

struct Target {
    released: AtomicBool,
    wake: Notify,
}

struct Shared {
    id: LinkId,
    cfg: TcpLanConfig,
    /// The announce sent first on each new connection: `cfg.own_announce`
    /// until [`TcpLan::set_announce`] replaces it.
    own: RwLock<Vec<u8>>,
    events: EventSink,
    gate: Arc<dyn AnnounceGate>,
    conns: Mutex<HashMap<SocketAddr, ConnHandle>>,
    handshaking: AtomicUsize,
    inbound_tx: mpsc::Sender<(PeerAddr, Frame)>,
    inbound_rx: tokio::sync::Mutex<mpsc::Receiver<(PeerAddr, Frame)>>,
    targets: Mutex<HashMap<SocketAddr, Arc<Target>>>,
    listener: Mutex<Option<(SocketAddr, JoinHandle<()>)>>,
}

impl Shared {
    fn conn_count(&self) -> usize {
        self.conns.lock().map(|c| c.len()).unwrap_or(usize::MAX)
    }

    fn at_capacity(&self) -> bool {
        self.conn_count()
            .saturating_add(self.handshaking.load(Ordering::SeqCst))
            >= self.cfg.max_connections
    }
}

/// One handshake in flight (the wrap's and the hello's): counted against
/// `max_connections`, decremented on every exit path.
struct HandshakeSlot(Arc<Shared>);

impl HandshakeSlot {
    fn take(shared: &Arc<Shared>) -> Self {
        shared.handshaking.fetch_add(1, Ordering::SeqCst);
        HandshakeSlot(shared.clone())
    }
}

impl Drop for HandshakeSlot {
    fn drop(&mut self) {
        self.0.handshaking.fetch_sub(1, Ordering::SeqCst);
    }
}

/// How a connection was opened.
#[derive(Debug, Clone, Copy)]
enum Opened {
    /// Accepted by the listener.
    Accepted,
    /// Dialed to `addr` for `expected` ([`Dialer::maintain`]).
    Dialed {
        addr: SocketAddr,
        expected: Option<NodeId>,
    },
}

/// The `tcp-lan` adapter (§8.1). Cheap to clone (shared state).
#[derive(Clone)]
pub struct TcpLan {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for TcpLan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TcpLan")
            .field("id", &self.shared.id)
            .field("conns", &self.shared.conn_count())
            .finish_non_exhaustive()
    }
}

impl TcpLan {
    /// A new adapter. Nothing listens until [`TcpLan::listen`]; nothing is
    /// dialed until [`Dialer::maintain`].
    pub fn new(
        id: LinkId,
        cfg: TcpLanConfig,
        events: EventSink,
        gate: Arc<dyn AnnounceGate>,
    ) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::channel(cfg.inbound_queue.max(1));
        TcpLan {
            shared: Arc::new(Shared {
                id,
                own: RwLock::new(cfg.own_announce.clone()),
                cfg,
                events,
                gate,
                conns: Mutex::new(HashMap::new()),
                handshaking: AtomicUsize::new(0),
                inbound_tx,
                inbound_rx: tokio::sync::Mutex::new(inbound_rx),
                targets: Mutex::new(HashMap::new()),
                listener: Mutex::new(None),
            }),
        }
    }

    /// Bind the listener at `addr` (the caller chooses: LAN-bound only when
    /// the node is a hub or LAN sharing is on, §8.1). Replaces any previous
    /// listener. Returns the bound address. Must run inside a Tokio runtime.
    pub async fn listen(&self, addr: SocketAddr) -> std::io::Result<SocketAddr> {
        self.stop_listening();
        let listener = TcpListener::bind(addr).await?;
        let local = listener.local_addr()?;
        let shared = self.shared.clone();
        let task = tokio::spawn(async move {
            loop {
                let (stream, remote) = match listener.accept().await {
                    Ok(x) => x,
                    Err(_) => {
                        // A persistent accept error (EMFILE): pause instead
                        // of spinning (transport detail, like keepalive).
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                if shared.cfg.refuse_loopback && is_loopback(&remote) {
                    continue; // §2: loopback is never a peer
                }
                if shared.at_capacity() {
                    continue;
                }
                let s = shared.clone();
                tokio::spawn(async move {
                    open(s, stream, remote, Opened::Accepted).await;
                });
            }
        });
        if let Ok(mut l) = self.shared.listener.lock() {
            *l = Some((local, task));
        }
        Ok(local)
    }

    /// The bound listener address, if listening.
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.shared
            .listener
            .lock()
            .ok()
            .and_then(|l| l.as_ref().map(|(a, _)| *a))
    }

    /// Stop listening (open connections stay).
    pub fn stop_listening(&self) {
        if let Ok(mut l) = self.shared.listener.lock() {
            if let Some((_, task)) = l.take() {
                task.abort();
            }
        }
    }

    /// Stop everything: listener, maintained dials, connections.
    pub fn shutdown(&self) {
        self.stop_listening();
        let targets: Vec<SocketAddr> = self
            .shared
            .targets
            .lock()
            .map(|t| t.keys().copied().collect())
            .unwrap_or_default();
        for a in targets {
            self.release(a);
        }
        if let Ok(conns) = self.shared.conns.lock() {
            for c in conns.values() {
                c.close.notify_one();
            }
        }
    }

    /// Connections currently up.
    pub fn connection_count(&self) -> usize {
        self.shared.conn_count()
    }

    /// Replace the own announce sent first on every **new** connection
    /// (accepts, dials, reconnects) — the host re-minted it (§7.3 A8: its
    /// roles or home-hub hint changed). The caller passes stored, verified
    /// bytes (§7: never minted here); connections already up keep the one
    /// they opened with.
    pub fn set_announce(&self, announce: Vec<u8>) {
        if let Ok(mut a) = self.shared.own.write() {
            *a = announce;
        }
    }
}

/// One TCP connection, start to end: the host's [`StreamWrap`] if any
/// (Layer 7 §4.4), then [`serve`]. Returns whether the handshake completed.
async fn open(shared: Arc<Shared>, stream: TcpStream, remote: SocketAddr, how: Opened) -> bool {
    let slot = HandshakeSlot::take(&shared);
    // One deadline for the wrap's handshake and the hello (monotonic clock).
    let deadline = tokio::time::Instant::now() + shared.cfg.handshake_timeout;
    let _ = stream.set_nodelay(true);
    let Some(wrap) = shared.cfg.wrap.clone() else {
        // `wrap = None`: the plain stream, byte-identical.
        return serve(shared, slot, stream, remote, how, None, deadline).await;
    };
    let wrapped = async {
        match how {
            Opened::Accepted => wrap.inbound(remote, stream).await,
            Opened::Dialed { addr, expected } => wrap
                .outbound(addr, expected, stream)
                .await
                .map(|s| (s, None)),
        }
    };
    let (stream, matched) = match tokio::time::timeout_at(deadline, wrapped).await {
        Ok(Ok(x)) => x,
        _ => return false, // refused, failed or slow: no 111 byte was written
    };
    serve(shared, slot, stream, remote, how, matched, deadline).await
}

/// One connection's 111 wire over any duplex stream (§8.1; generic since
/// Layer 7 §4.4). `matched` is the pair an inbound wrap named (S7).
async fn serve<S>(
    shared: Arc<Shared>,
    slot: HandshakeSlot,
    stream: S,
    remote: SocketAddr,
    how: Opened,
    matched: Option<NodeId>,
    deadline: tokio::time::Instant,
) -> bool
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let (outbound, expected) = match how {
        Opened::Accepted => (false, None),
        Opened::Dialed { expected, .. } => (true, expected),
    };
    let (mut rd, mut wr) = tokio::io::split(stream);
    let own = shared
        .own
        .read()
        .map(|a| a.clone())
        .unwrap_or_else(|_| shared.cfg.own_announce.clone());
    let gate = shared.gate.clone();
    let handshake = async {
        wr.write_all(&HELLO).await?;
        wr.write_all(&encode_frame(T_ANNOUNCE, &own)?).await?;
        read_hello(&mut rd).await?;
        let (ty, body) = read_frame(&mut rd).await?;
        if ty != T_ANNOUNCE {
            return Err(WireError::NotAnnounceFirst);
        }
        let ann = verify_announce(&body).ok_or(WireError::AnnounceRefused)?;
        // S7: the wrapped peer is the pair its pinned key matched; another
        // NodeId ends the connection here, before any other 111 byte and
        // before the fabric's gate sees it.
        if matched.is_some_and(|m| m != ann.node_id()) {
            return Err(WireError::ClaimMismatch);
        }
        if !gate.admit(&ann) {
            return Err(WireError::AnnounceRefused);
        }
        // A misdial is a failed attempt, never a connection (module doc).
        if expected.is_some_and(|e| e != ann.node_id()) {
            return Err(WireError::ClaimMismatch);
        }
        Ok::<Vec<u8>, WireError>(body)
    };
    let first = match tokio::time::timeout_at(deadline, handshake).await {
        Ok(Ok(body)) => body,
        _ => return false,
    };

    let (tx, rx) = mpsc::channel::<Vec<u8>>(shared.cfg.write_queue.max(1));
    let close = Arc::new(Notify::new());
    {
        let Ok(mut conns) = shared.conns.lock() else {
            return false;
        };
        if conns.len() >= shared.cfg.max_connections || conns.contains_key(&remote) {
            return false;
        }
        conns.insert(
            remote,
            ConnHandle {
                tx,
                close: close.clone(),
            },
        );
    }
    drop(slot);
    let peer = PeerAddr::Tcp(remote);
    let _ = shared.events.send(LinkEvent::Connected {
        link: shared.id.clone(),
        peer: peer.clone(),
        outbound,
        expected,
    });

    let keepalive = shared.cfg.keepalive;
    let writer = tokio::spawn(writer_loop(wr, rx, keepalive));
    if shared
        .inbound_tx
        .send((peer.clone(), Frame::announce(first)))
        .await
        .is_ok()
    {
        reader_loop(&shared, &mut rd, &peer, &close, keepalive.saturating_mul(3)).await;
    }
    writer.abort();
    if let Ok(mut conns) = shared.conns.lock() {
        if conns
            .get(&remote)
            .is_some_and(|c| Arc::ptr_eq(&c.close, &close))
        {
            conns.remove(&remote);
        }
    }
    let _ = shared.events.send(LinkEvent::Disconnected {
        link: shared.id.clone(),
        peer,
    });
    true
}

async fn reader_loop<R: AsyncRead + Unpin>(
    shared: &Shared,
    rd: &mut R,
    peer: &PeerAddr,
    close: &Notify,
    idle: Duration,
) {
    loop {
        let frame = tokio::select! {
            _ = close.notified() => return,
            r = tokio::time::timeout(idle, read_frame(rd)) => match r {
                Ok(Ok(f)) => f,
                _ => return, // idle, io, or a cap/type violation
            },
        };
        let out = match frame {
            (T_SEALED, body) => Frame::sealed(body),
            (T_ANNOUNCE, body) => {
                if verify_announce(&body).is_none() {
                    return; // §8.1: a bad announce drops the peer
                }
                Frame::announce(body)
            }
            (T_CARD_REQUEST, body) => Frame {
                frame_type: FrameType::CardRequest,
                hop_limit: None,
                bytes: body,
            },
            (T_KEEPALIVE, _) | (T_CHUNK, _) => continue, // transport detail / reserved
            _ => return,
        };
        if shared.inbound_tx.send((peer.clone(), out)).await.is_err() {
            return;
        }
    }
}

async fn writer_loop<W: AsyncWrite + Unpin>(
    mut wr: W,
    mut rx: mpsc::Receiver<Vec<u8>>,
    keepalive: Duration,
) {
    let ka = [T_KEEPALIVE, 0, 0];
    let mut tick = tokio::time::interval(keepalive);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick.tick().await; // the first tick is immediate
    loop {
        tokio::select! {
            m = rx.recv() => match m {
                Some(bytes) => {
                    if wr.write_all(&bytes).await.is_err() {
                        return;
                    }
                }
                None => return,
            },
            _ = tick.tick() => {
                if wr.write_all(&ka).await.is_err() {
                    return;
                }
            }
        }
    }
}

impl Link for TcpLan {
    fn id(&self) -> LinkId {
        self.shared.id.clone()
    }

    fn kind(&self) -> LinkKind {
        LinkKind::TcpLan
    }

    fn profile(&self) -> PathProfile {
        PathProfile::tcp_lan()
    }

    fn peers(&self) -> Vec<PeerAddr> {
        self.shared
            .conns
            .lock()
            .map(|c| c.keys().map(|a| PeerAddr::Tcp(*a)).collect())
            .unwrap_or_default()
    }

    fn send(&self, to: PeerAddr, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            let PeerAddr::Tcp(addr) = to else {
                return Err(LinkError::Closed);
            };
            let ty = frame_wire_type(&frame)?;
            if frame.bytes.len() > MAX_BODY {
                return Err(LinkError::TooLarge(frame.bytes.len()));
            }
            let bytes = encode_frame(ty, &frame.bytes).map_err(|_| {
                LinkError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "frame length outside its type's cap",
                ))
            })?;
            let tx = {
                let conns = self.shared.conns.lock().map_err(|_| LinkError::Closed)?;
                conns.get(&addr).map(|c| c.tx.clone())
            };
            let tx = tx.ok_or(LinkError::Closed)?;
            tx.try_send(bytes).map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => LinkError::Io(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "write queue full",
                )),
                mpsc::error::TrySendError::Closed(_) => LinkError::Closed,
            })
        })
    }

    fn recv(&self) -> BoxFuture<'_, (PeerAddr, Frame)> {
        Box::pin(async move {
            let mut rx = self.shared.inbound_rx.lock().await;
            match rx.recv().await {
                Some(x) => x,
                // Unreachable while `Shared` holds a sender; pend, never spin.
                None => futures::future::pending().await,
            }
        })
    }

    fn state(&self) -> LinkState {
        let peers = self.shared.conn_count();
        let listening = self.local_addr().is_some();
        let dialing = self
            .shared
            .targets
            .lock()
            .map(|t| !t.is_empty())
            .unwrap_or(false);
        if peers > 0 || listening || dialing {
            LinkState::Up { peers }
        } else {
            LinkState::Down
        }
    }
}

impl Dialer for TcpLan {
    fn maintain(&self, addr: SocketAddr, expected: Option<NodeId>) {
        let target = Arc::new(Target {
            released: AtomicBool::new(false),
            wake: Notify::new(),
        });
        {
            let Ok(mut t) = self.shared.targets.lock() else {
                return;
            };
            if t.contains_key(&addr) {
                return;
            }
            t.insert(addr, target.clone());
        }
        let shared = self.shared.clone();
        tokio::spawn(async move {
            let mut backoff = shared.cfg.backoff_min;
            loop {
                if target.released.load(Ordering::SeqCst) {
                    return;
                }
                if shared.cfg.refuse_loopback && is_loopback(&addr) {
                    return; // §2: loopback is never a peer
                }
                if !shared.at_capacity() {
                    let connect = TcpStream::connect(addr);
                    if let Ok(Ok(stream)) =
                        tokio::time::timeout(shared.cfg.handshake_timeout, connect).await
                    {
                        let remote = stream.peer_addr().unwrap_or(addr);
                        let how = Opened::Dialed { addr, expected };
                        if open(shared.clone(), stream, remote, how).await {
                            backoff = shared.cfg.backoff_min;
                        }
                    }
                }
                if target.released.load(Ordering::SeqCst) {
                    return;
                }
                tokio::select! {
                    _ = target.wake.notified() => {}
                    _ = tokio::time::sleep(backoff) => {}
                }
                backoff = backoff.saturating_mul(2).min(shared.cfg.backoff_max);
            }
        });
    }

    fn release(&self, addr: SocketAddr) {
        let target = self
            .shared
            .targets
            .lock()
            .ok()
            .and_then(|mut t| t.remove(&addr));
        if let Some(t) = target {
            t.released.store(true, Ordering::SeqCst);
            t.wake.notify_one();
        }
        self.hang_up(&PeerAddr::Tcp(addr));
    }

    fn hang_up(&self, peer: &PeerAddr) {
        let PeerAddr::Tcp(addr) = peer else {
            return;
        };
        if let Ok(conns) = self.shared.conns.lock() {
            if let Some(c) = conns.get(addr) {
                c.close.notify_one();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_are_checked_before_the_body() {
        assert!(check_frame_len(T_SEALED, 0).is_err());
        assert!(check_frame_len(T_SEALED, 65_535).is_ok());
        assert!(check_frame_len(T_ANNOUNCE, 136).is_err());
        assert!(check_frame_len(T_ANNOUNCE, 137).is_ok());
        assert!(check_frame_len(T_ANNOUNCE, 169).is_ok());
        assert!(check_frame_len(T_ANNOUNCE, 170).is_err());
        assert!(check_frame_len(T_KEEPALIVE, 0).is_ok());
        assert!(check_frame_len(T_KEEPALIVE, 1).is_err());
        assert!(check_frame_len(T_CARD_REQUEST, 32).is_ok());
        assert!(check_frame_len(T_CARD_REQUEST, 31).is_err());
        assert!(check_frame_len(T_CHUNK, 10).is_ok());
        assert!(matches!(
            check_frame_len(0x06, 0),
            Err(WireError::UnknownType(6))
        ));
        assert!(matches!(
            check_frame_len(0x00, 0),
            Err(WireError::UnknownType(0))
        ));
    }

    #[test]
    fn frame_encoding_is_type_len_be_body() {
        let f = encode_frame(T_SEALED, &[9; 300]).unwrap();
        assert_eq!(&f[..3], &[0x01, 0x01, 0x2c]);
        assert_eq!(f.len(), 303);
        assert!(encode_frame(T_CARD_REQUEST, &[0; 33]).is_err());
    }

    #[tokio::test]
    async fn read_frame_refuses_oversize_without_reading_body() {
        // A keepalive that claims a body: refused from the header alone.
        let bytes = [T_KEEPALIVE, 0x00, 0x05];
        let mut r: &[u8] = &bytes;
        assert!(matches!(
            read_frame(&mut r).await,
            Err(WireError::Length { ty: 3, len: 5 })
        ));
        let mut ok: &[u8] = &[T_CARD_REQUEST, 0, 32, 1, 2];
        assert!(matches!(read_frame(&mut ok).await, Err(WireError::Io(_))));
        let mut h: &[u8] = b"111G\x01";
        assert!(matches!(read_hello(&mut h).await, Err(WireError::BadHello)));
    }

    /// `set_announce`: a new connection opens with the replaced announce
    /// (hello, then the announce frame, before anything is read).
    #[tokio::test]
    async fn set_announce_applies_to_new_connections() {
        let first = vec![0xa1u8; announce::BASE_LEN];
        let second = vec![0xb2u8; announce::BASE_LEN + 8];
        let mut cfg = TcpLanConfig::new(first.clone());
        cfg.refuse_loopback = false; // the test client is on loopback
        let (events, _rx) = mpsc::unbounded_channel();
        let t = TcpLan::new(
            LinkId::new("tcp-lan"),
            cfg,
            events,
            Arc::new(super::super::AdmitAll),
        );
        let at = t.listen("127.0.0.1:0".parse().unwrap()).await.unwrap();
        async fn opening(at: SocketAddr) -> Vec<u8> {
            let mut s = TcpStream::connect(at).await.unwrap();
            read_hello(&mut s).await.unwrap();
            let (ty, body) = read_frame(&mut s).await.unwrap();
            assert_eq!(ty, T_ANNOUNCE);
            body
        }
        assert_eq!(opening(at).await, first);
        t.set_announce(second.clone());
        assert_eq!(opening(at).await, second);
        t.shutdown();
    }

    /// A test identity: its NodeId and stored announce bytes.
    fn ident(label: u8) -> (NodeId, Vec<u8>) {
        let mut b = [0u8; 64];
        b[..32].copy_from_slice(&crate::envelope::blake2b_256(&[b"m6-sign", &[label]]));
        b[32..].copy_from_slice(&crate::envelope::blake2b_256(&[b"m6-ecdh", &[label]]));
        let k = id111::keyset::Keyset::from_bytes(&b).unwrap();
        let ann = Announce::create(&k, 1, &[1, 0, 0, 0]).unwrap().encode();
        (k.node_id(), ann)
    }

    /// A fake [`StreamWrap`] (Layer 7 §4.4): each connection is pumped
    /// through a `tokio::io::duplex` pair, so the adapter speaks over a
    /// stream that is not a `TcpStream`; every call is recorded.
    #[derive(Default)]
    struct DuplexWrap {
        matched: Option<NodeId>,
        refuse_outbound: bool,
        calls: Mutex<Vec<(bool, SocketAddr, Option<NodeId>)>>,
    }

    fn pump(tcp: TcpStream) -> BoxedStream {
        let (near, mut far) = tokio::io::duplex(1 << 17);
        tokio::spawn(async move {
            let mut tcp = tcp;
            let _ = tokio::io::copy_bidirectional(&mut tcp, &mut far).await;
        });
        Box::new(near)
    }

    impl StreamWrap for DuplexWrap {
        fn outbound(
            &self,
            addr: SocketAddr,
            expected: Option<NodeId>,
            stream: TcpStream,
        ) -> BoxFuture<'_, io::Result<BoxedStream>> {
            self.calls.lock().unwrap().push((true, addr, expected));
            let refuse = self.refuse_outbound;
            Box::pin(async move {
                if refuse {
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, "refused"));
                }
                Ok(pump(stream))
            })
        }

        fn inbound(
            &self,
            remote: SocketAddr,
            stream: TcpStream,
        ) -> BoxFuture<'_, io::Result<(BoxedStream, Option<NodeId>)>> {
            self.calls.lock().unwrap().push((false, remote, None));
            let matched = self.matched;
            Box::pin(async move { Ok((pump(stream), matched)) })
        }
    }

    fn wrapped(
        own: Vec<u8>,
        wrap: Arc<DuplexWrap>,
    ) -> (TcpLan, mpsc::UnboundedReceiver<LinkEvent>) {
        let mut cfg = TcpLanConfig::new(own);
        cfg.refuse_loopback = false; // the test peers are on loopback
        cfg.wrap = Some(wrap);
        let (events, rx) = mpsc::unbounded_channel();
        let t = TcpLan::new(
            LinkId::new("tcp-lan"),
            cfg,
            events,
            Arc::new(super::super::AdmitAll),
        );
        (t, rx)
    }

    async fn next_event(rx: &mut mpsc::UnboundedReceiver<LinkEvent>) -> LinkEvent {
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("an event")
            .expect("the sink is open")
    }

    /// Everything the peer still sends until it closes.
    async fn rest(s: &mut TcpStream) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf = [0u8; 512];
        loop {
            match tokio::time::timeout(Duration::from_secs(5), s.read(&mut buf)).await {
                Ok(Ok(0)) | Ok(Err(_)) => return out,
                Ok(Ok(n)) => out.extend_from_slice(&buf[..n]),
                Err(_) => panic!("the connection stayed open"),
            }
        }
    }

    #[test]
    fn the_default_is_no_wrap() {
        let cfg = TcpLanConfig::new(Vec::new());
        assert!(cfg.wrap.is_none());
        assert!(format!("{cfg:?}").contains("wrap: false"));
    }

    /// Layer 7 §4.4: through a wrapped inbound stream the 111 wire is
    /// unchanged in both directions, and a first announce from the matched
    /// pair is served.
    #[tokio::test]
    async fn a_wrapped_inbound_stream_carries_the_wire_unchanged() {
        let (_, own) = ident(1);
        let (peer_id, peer_ann) = ident(2);
        let wrap = Arc::new(DuplexWrap {
            matched: Some(peer_id),
            ..DuplexWrap::default()
        });
        let (t, mut events) = wrapped(own.clone(), wrap.clone());
        let at = t.listen("127.0.0.1:0".parse().unwrap()).await.unwrap();

        let mut c = TcpStream::connect(at).await.unwrap();
        read_hello(&mut c).await.unwrap();
        assert_eq!(read_frame(&mut c).await.unwrap(), (T_ANNOUNCE, own));
        c.write_all(&HELLO).await.unwrap();
        c.write_all(&encode_frame(T_ANNOUNCE, &peer_ann).unwrap())
            .await
            .unwrap();

        let LinkEvent::Connected {
            peer,
            outbound,
            expected,
            ..
        } = next_event(&mut events).await
        else {
            panic!("not a connection");
        };
        assert!(!outbound);
        assert_eq!(expected, None);
        assert_eq!(t.connection_count(), 1);
        assert_eq!(t.recv().await, (peer.clone(), Frame::announce(peer_ann)));

        let body: Vec<u8> = (0..300u16).map(|i| (i % 251) as u8).collect();
        c.write_all(&encode_frame(T_SEALED, &body).unwrap())
            .await
            .unwrap();
        assert_eq!(t.recv().await, (peer.clone(), Frame::sealed(body)));

        t.send(peer, Frame::sealed(vec![7; 40])).await.unwrap();
        assert_eq!(read_frame(&mut c).await.unwrap(), (T_SEALED, vec![7; 40]));

        let local = c.local_addr().unwrap();
        assert_eq!(*wrap.calls.lock().unwrap(), vec![(false, local, None)]);
        t.shutdown();
    }

    /// S7: a wrapped peer whose pinned key matched one pair and whose first
    /// announce names another NodeId is refused: no byte follows the hello
    /// and the own announce, and no connection is reported.
    #[tokio::test]
    async fn a_mismatched_first_announce_ends_the_connection() {
        let (_, own) = ident(1);
        let (pair_id, _) = ident(2);
        let (_, other_ann) = ident(3);
        let wrap = Arc::new(DuplexWrap {
            matched: Some(pair_id),
            ..DuplexWrap::default()
        });
        let (t, mut events) = wrapped(own.clone(), wrap);
        let at = t.listen("127.0.0.1:0".parse().unwrap()).await.unwrap();

        let mut c = TcpStream::connect(at).await.unwrap();
        read_hello(&mut c).await.unwrap();
        assert_eq!(read_frame(&mut c).await.unwrap(), (T_ANNOUNCE, own));
        c.write_all(&HELLO).await.unwrap();
        c.write_all(&encode_frame(T_ANNOUNCE, &other_ann).unwrap())
            .await
            .unwrap();
        assert_eq!(rest(&mut c).await, Vec::<u8>::new(), "no other 111 byte");
        assert!(events.try_recv().is_err(), "never a connection");
        assert_eq!(t.connection_count(), 0);
        t.shutdown();
    }

    /// Layer 7 §4.4: a dial hands the wrap its address and expected NodeId,
    /// and the wire inside is unchanged.
    #[tokio::test]
    async fn a_wrapped_dial_names_the_pair_and_carries_the_wire() {
        let (_, own) = ident(1);
        let (peer_id, peer_ann) = ident(2);
        let wrap = Arc::new(DuplexWrap::default());
        let (t, mut events) = wrapped(own.clone(), wrap.clone());
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        t.maintain(addr, Some(peer_id));

        let (mut s, _) = l.accept().await.unwrap();
        read_hello(&mut s).await.unwrap();
        assert_eq!(read_frame(&mut s).await.unwrap(), (T_ANNOUNCE, own));
        s.write_all(&HELLO).await.unwrap();
        s.write_all(&encode_frame(T_ANNOUNCE, &peer_ann).unwrap())
            .await
            .unwrap();
        let LinkEvent::Connected {
            outbound, expected, ..
        } = next_event(&mut events).await
        else {
            panic!("not a connection");
        };
        assert!(outbound);
        assert_eq!(expected, Some(peer_id));
        assert_eq!(
            *wrap.calls.lock().unwrap(),
            vec![(true, addr, Some(peer_id))]
        );
        t.release(addr);
        t.shutdown();
    }

    /// A wrap that refuses a dial ends it before any 111 byte; the dial is a
    /// failed attempt, never a connection.
    #[tokio::test]
    async fn a_refused_wrap_writes_no_111_byte() {
        let (_, own) = ident(1);
        let wrap = Arc::new(DuplexWrap {
            refuse_outbound: true,
            ..DuplexWrap::default()
        });
        let (t, mut events) = wrapped(own, wrap);
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        t.maintain(addr, None);
        let (mut s, _) = l.accept().await.unwrap();
        assert_eq!(rest(&mut s).await, Vec::<u8>::new());
        assert!(events.try_recv().is_err());
        assert_eq!(t.connection_count(), 0);
        t.release(addr);
        t.shutdown();
    }

    #[test]
    fn loopback_detection_covers_mapped_v6() {
        assert!(is_loopback(&"127.0.0.1:1".parse().unwrap()));
        assert!(is_loopback(&"[::1]:1".parse().unwrap()));
        assert!(is_loopback(&"[::ffff:127.0.0.1]:1".parse().unwrap()));
        assert!(!is_loopback(&"192.168.1.2:1".parse().unwrap()));
    }
}
