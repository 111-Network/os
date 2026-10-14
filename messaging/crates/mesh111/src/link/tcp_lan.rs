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
//!   the fabric asks to [`Dialer::maintain`].
//! - **Loopback is never a peer** (§2, §8.1): with `refuse_loopback` (the
//!   default) inbound connections from, and dials to, loopback addresses are
//!   refused. The listener is bound only where the caller says (the fabric
//!   settings: hub role or LAN sharing) — [`TcpLan::listen`].
//! - **Capacity**, not policy: at most `max_connections` connections (file
//!   descriptors are finite); a full table refuses new ones.
//!
//! The stream leaks `dest_hint`, counters and sizes to an on-path observer
//! (§18 rule 4): no transport encryption in v1; content, sender and kind are
//! sealed.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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

/// Adapter settings. Every limit is capacity or transport, never policy.
#[derive(Debug, Clone)]
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

/// Decrements the handshake counter on every exit path.
struct HandshakeSlot<'a>(&'a AtomicUsize);

impl Drop for HandshakeSlot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
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
        let (inbound_tx, inbound_rx) = mpsc::channel(1024);
        TcpLan {
            shared: Arc::new(Shared {
                id,
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
                    serve(s, stream, remote, false, None).await;
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
}

/// One connection, start to end. Returns whether the handshake completed.
async fn serve(
    shared: Arc<Shared>,
    stream: TcpStream,
    remote: SocketAddr,
    outbound: bool,
    expected: Option<NodeId>,
) -> bool {
    shared.handshaking.fetch_add(1, Ordering::SeqCst);
    let slot = HandshakeSlot(&shared.handshaking);
    let _ = stream.set_nodelay(true);
    let (mut rd, mut wr) = stream.into_split();
    let own = shared.cfg.own_announce.clone();
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
        if !gate.admit(&ann) {
            return Err(WireError::AnnounceRefused);
        }
        Ok::<Vec<u8>, WireError>(body)
    };
    let first = match tokio::time::timeout(shared.cfg.handshake_timeout, handshake).await {
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
                        if serve(shared.clone(), stream, remote, true, expected).await {
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

    #[test]
    fn loopback_detection_covers_mapped_v6() {
        assert!(is_loopback(&"127.0.0.1:1".parse().unwrap()));
        assert!(is_loopback(&"[::1]:1".parse().unwrap()));
        assert!(is_loopback(&"[::ffff:127.0.0.1]:1".parse().unwrap()));
        assert!(!is_loopback(&"192.168.1.2:1".parse().unwrap()));
    }
}
