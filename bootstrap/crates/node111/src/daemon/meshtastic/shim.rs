//! The `meshtastic-http` panel shim (Layer 5 §13 `[source] shim`, §14 "One API
//! client").
//!
//! Meshtastic's web client speaks the node HTTP API: `PUT /api/v1/toradio`
//! with one `ToRadio` protobuf as the body, and `GET /api/v1/fromradio` for the
//! next `FromRadio`. meshmud has no HTTP downlink here (§14); instead `111d`
//! answers those two paths on the guarded panel origin and **reframes** the
//! bodies onto meshmud's TCP downlink, which carries the same protobuf bytes
//! behind the 4-byte stream header `0x94 0xC3 <len hi> <len lo>`. No protobuf
//! is ever decoded here (and no GPL code enters this binary).
//!
//! - **Per panel session one TCP connection** to the downlink, opened on the
//!   session's first `toradio`/`fromradio` call, closed when the panel listener
//!   reports the session revoked ([`MeshtasticShim::close_session`]) or after
//!   [`ShimConfig::idle`] without a call. Several viewers each own a downlink.
//! - `PUT /api/v1/toradio`: a body of 1..=512 bytes (Meshtastic's
//!   `MAX_TO_FROM_RADIO_SIZE`) → exactly one framed `ToRadio`; `200`, empty.
//! - `GET /api/v1/fromradio[?all=…]`: the **next** queued `FromRadio` payload
//!   (`application/x-protobuf`), or an empty `200` when none is queued. One
//!   frame per call whatever `all` says — the client polls until empty.
//! - The per-session `FromRadio` queue is bounded ([`ShimConfig::capacity`]
//!   frames) and **drops the oldest** when full; every drop is counted, and the
//!   total is surfaced in `panel.state.dropped` ([`MeshtasticShim::dropped`]).
//! - The deframer skips anything that is not a frame (the board's debug
//!   output) and resyncs on a declared length over 512.
//! - A closed or failing upstream answers `502 upstream_error` and drops the
//!   session's connection; the next call opens a fresh one.

use crate::daemon::panels::shim::{Shim, ShimError, ShimRequest, ShimResponse, ShimSession};
use axum::body::Bytes;
use axum::http::{Method, StatusCode};
use futures_util::future::BoxFuture;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::Instant;

/// The manifest's shim name (`assets/panels/meshtastic.toml`).
pub const NAME: &str = "meshtastic-http";
/// Stream header bytes.
pub const START1: u8 = 0x94;
pub const START2: u8 = 0xC3;
/// Largest ToRadio/FromRadio payload (Meshtastic `MAX_TO_FROM_RADIO_SIZE`).
pub const MAX_FRAME: usize = 512;
/// The protobuf media type the node HTTP API uses.
pub const PROTOBUF: &str = "application/x-protobuf";

/// Frame one payload: `0x94 0xC3 len_hi len_lo ‖ payload`.
pub fn frame(payload: &[u8]) -> Result<Vec<u8>, ShimError> {
    if payload.is_empty() {
        return Err(ShimError::BadRequest("empty ToRadio body".into()));
    }
    if payload.len() > MAX_FRAME {
        return Err(ShimError::TooLarge { limit: MAX_FRAME });
    }
    let len = payload.len() as u16;
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&[START1, START2, (len >> 8) as u8, len as u8]);
    out.extend_from_slice(payload);
    Ok(out)
}

/// Incremental deframer for the downlink byte stream (bounded: it never
/// holds more than one header + [`MAX_FRAME`] bytes of a partial frame).
#[derive(Debug, Default)]
pub struct Deframer {
    buf: Vec<u8>,
}

impl Deframer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed bytes; returns every complete payload, in order.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            // drop everything before the next START1 START2 (debug text, noise)
            match self.buf.windows(2).position(|w| w == [START1, START2]) {
                Some(0) => {}
                Some(i) => {
                    self.buf.drain(..i);
                }
                None => {
                    // keep a trailing START1: its START2 may be next
                    let keep = usize::from(self.buf.last() == Some(&START1));
                    let cut = self.buf.len() - keep;
                    self.buf.drain(..cut);
                    return out;
                }
            }
            if self.buf.len() < 4 {
                return out;
            }
            let len = usize::from(u16::from_be_bytes([self.buf[2], self.buf[3]]));
            if len > MAX_FRAME {
                // corrupt header: skip this START1 and resync
                self.buf.drain(..1);
                continue;
            }
            if self.buf.len() < 4 + len {
                return out;
            }
            out.push(self.buf[4..4 + len].to_vec());
            self.buf.drain(..4 + len);
        }
    }

    /// Bytes held for a partial frame.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }
}

/// Bounded drop-oldest queue of `FromRadio` payloads.
#[derive(Debug)]
pub struct FrameQueue {
    frames: VecDeque<Vec<u8>>,
    capacity: usize,
    dropped: u64,
}

impl FrameQueue {
    pub fn new(capacity: usize) -> Self {
        FrameQueue {
            frames: VecDeque::new(),
            capacity: capacity.max(1),
            dropped: 0,
        }
    }

    /// Append; when full the oldest frame goes. Returns how many were dropped.
    pub fn push(&mut self, f: Vec<u8>) -> u64 {
        let mut d = 0;
        while self.frames.len() >= self.capacity {
            self.frames.pop_front();
            d += 1;
        }
        self.frames.push_back(f);
        self.dropped += d;
        d
    }

    pub fn pop(&mut self) -> Option<Vec<u8>> {
        self.frames.pop_front()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Frames this queue dropped.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// Shim tuning (production: [`ShimConfig::production`]).
#[derive(Debug, Clone)]
pub struct ShimConfig {
    /// meshmud's TCP downlink (`127.0.0.1:11230`).
    pub downlink: SocketAddr,
    /// FromRadio frames queued per session before the oldest is dropped.
    pub capacity: usize,
    /// A session without a call this long loses its connection.
    pub idle: Duration,
    /// Sessions with an open connection at once (more ⇒ `502`).
    pub max_sessions: usize,
    pub connect_timeout: Duration,
    pub write_timeout: Duration,
}

impl ShimConfig {
    pub fn production(downlink: SocketAddr) -> Self {
        ShimConfig {
            downlink,
            // a full node DB download is one FromRadio per node; 1024 × ≤ 512 B
            // = ≤ 512 KiB per viewer
            capacity: 1024,
            idle: Duration::from_secs(60),
            max_sessions: 16,
            connect_timeout: Duration::from_secs(3),
            write_timeout: Duration::from_secs(5),
        }
    }
}

/// One panel session's downlink.
struct Session {
    stream: Arc<TcpStream>,
    queue: Mutex<FrameQueue>,
    /// Serializes writers so two PUTs never interleave partial frames.
    write: tokio::sync::Mutex<()>,
    last_used: Mutex<Instant>,
    closed: AtomicBool,
    close: tokio::sync::Notify,
}

impl Session {
    fn touch(&self) {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn last_used(&self) -> Instant {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn shut(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.close.notify_one();
    }
}

struct Inner {
    cfg: ShimConfig,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    dropped: AtomicU64,
}

/// The `meshtastic-http` shim (register it in the `ShimRegistry`).
#[derive(Clone)]
pub struct MeshtasticShim {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for MeshtasticShim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeshtasticShim")
            .field("downlink", &self.inner.cfg.downlink)
            .field("sessions", &self.sessions())
            .field("dropped", &self.dropped())
            .finish()
    }
}

fn upstream(e: impl std::fmt::Display) -> ShimError {
    ShimError::Upstream(format!("meshmud downlink: {e}"))
}

impl MeshtasticShim {
    pub fn new(cfg: ShimConfig) -> Self {
        MeshtasticShim {
            inner: Arc::new(Inner {
                cfg,
                sessions: Mutex::new(HashMap::new()),
                dropped: AtomicU64::new(0),
            }),
        }
    }

    pub fn config(&self) -> &ShimConfig {
        &self.inner.cfg
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Session>>> {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// FromRadio frames dropped by full session queues since start
    /// (`PanelState.dropped`).
    pub fn dropped(&self) -> u64 {
        self.inner.dropped.load(Ordering::Relaxed)
    }

    /// Sessions holding a downlink connection now.
    pub fn sessions(&self) -> usize {
        self.map().len()
    }

    /// Close one session's connection (cookie revoked, panel disabled,
    /// LAN off — the panel listener calls this). Unknown sessions are a no-op.
    pub fn close_session(&self, session: &str) -> bool {
        match self.map().remove(session) {
            Some(s) => {
                s.shut();
                true
            }
            None => false,
        }
    }

    /// Close every session (meshmud restarted or unbound; daemon shutdown).
    pub fn close_all(&self) {
        for (_, s) in self.map().drain() {
            s.shut();
        }
    }

    /// The live session for `key`, or a new connection (first call).
    async fn session(&self, key: &str) -> Result<Arc<Session>, ShimError> {
        if let Some(s) = self.map().get(key) {
            if !s.closed.load(Ordering::SeqCst) {
                return Ok(Arc::clone(s));
            }
        }
        let cfg = &self.inner.cfg;
        if self.map().len() >= cfg.max_sessions {
            // stale entries are removed by their tasks; count only live ones
            self.map().retain(|_, s| !s.closed.load(Ordering::SeqCst));
            if self.map().len() >= cfg.max_sessions {
                return Err(ShimError::Upstream(format!(
                    "too many open panel sessions (at most {})",
                    cfg.max_sessions
                )));
            }
        }
        let stream = tokio::time::timeout(cfg.connect_timeout, TcpStream::connect(cfg.downlink))
            .await
            .map_err(|_| upstream("connect timed out"))?
            .map_err(upstream)?;
        let _ = stream.set_nodelay(true);
        let s = Arc::new(Session {
            stream: Arc::new(stream),
            queue: Mutex::new(FrameQueue::new(cfg.capacity)),
            write: tokio::sync::Mutex::new(()),
            last_used: Mutex::new(Instant::now()),
            closed: AtomicBool::new(false),
            close: tokio::sync::Notify::new(),
        });
        // a concurrent first call may have raced us: keep one connection
        {
            let mut map = self.map();
            if let Some(existing) = map.get(key) {
                if !existing.closed.load(Ordering::SeqCst) {
                    return Ok(Arc::clone(existing));
                }
            }
            map.insert(key.to_string(), Arc::clone(&s));
        }
        tokio::spawn(read_loop(
            Arc::clone(&self.inner),
            key.to_string(),
            Arc::clone(&s),
        ));
        Ok(s)
    }

    fn drop_session(&self, key: &str, s: &Arc<Session>) {
        s.shut();
        let mut map = self.map();
        if map.get(key).is_some_and(|cur| Arc::ptr_eq(cur, s)) {
            map.remove(key);
        }
    }

    async fn toradio(&self, key: &str, body: &[u8]) -> Result<ShimResponse, ShimError> {
        let framed = frame(body)?;
        let s = self.session(key).await?;
        s.touch();
        let _w = s.write.lock().await;
        if s.closed.load(Ordering::SeqCst) {
            self.drop_session(key, &s);
            return Err(upstream("connection closed"));
        }
        let wrote = tokio::time::timeout(
            self.inner.cfg.write_timeout,
            crate::daemon::services::probe::write_all(&s.stream, &framed),
        )
        .await;
        match wrote {
            Ok(Ok(())) => Ok(ShimResponse::empty(StatusCode::OK)),
            Ok(Err(e)) => {
                self.drop_session(key, &s);
                Err(upstream(e))
            }
            Err(_) => {
                self.drop_session(key, &s);
                Err(upstream("write timed out"))
            }
        }
    }

    async fn fromradio(&self, key: &str) -> Result<ShimResponse, ShimError> {
        let s = self.session(key).await?;
        s.touch();
        let next = s.queue.lock().unwrap_or_else(|e| e.into_inner()).pop();
        match next {
            Some(f) => Ok(ShimResponse::ok(PROTOBUF, f)),
            None if s.closed.load(Ordering::SeqCst) => {
                self.drop_session(key, &s);
                Err(upstream("connection closed"))
            }
            None => Ok(ShimResponse::ok(PROTOBUF, Bytes::new())),
        }
    }
}

/// Per-session reader: deframes the downlink into the bounded queue until
/// EOF, an error, a close request or the idle timeout.
async fn read_loop(inner: Arc<Inner>, key: String, s: Arc<Session>) {
    let mut deframer = Deframer::new();
    let mut chunk = vec![0u8; 4096];
    let idle = inner.cfg.idle;
    loop {
        if s.closed.load(Ordering::SeqCst) {
            break;
        }
        let deadline = s.last_used() + idle;
        tokio::select! {
            r = s.stream.readable() => {
                if r.is_err() {
                    break;
                }
                match s.stream.try_read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let frames = deframer.push(&chunk[..n]);
                        if !frames.is_empty() {
                            let mut q = s.queue.lock().unwrap_or_else(|e| e.into_inner());
                            for f in frames {
                                let d = q.push(f);
                                if d > 0 {
                                    inner.dropped.fetch_add(d, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => break,
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                if Instant::now() >= s.last_used() + idle {
                    break; // idle: the viewer went away
                }
            }
            _ = s.close.notified() => {}
        }
    }
    s.closed.store(true, Ordering::SeqCst);
    let mut map = inner
        .sessions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if map.get(&key).is_some_and(|cur| Arc::ptr_eq(cur, &s)) {
        map.remove(&key);
    }
    // dropping the last Arc<TcpStream> closes the connection
}

impl Shim for MeshtasticShim {
    fn name(&self) -> &'static str {
        NAME
    }

    fn handle<'a>(
        &'a self,
        session: &'a ShimSession,
        req: ShimRequest,
    ) -> BoxFuture<'a, Result<ShimResponse, ShimError>> {
        Box::pin(async move {
            if session.panel != "meshtastic" {
                return Err(ShimError::NotFound);
            }
            let key = session.session.as_str();
            match req.path.as_str() {
                "/api/v1/toradio" => match req.method {
                    Method::PUT => self.toradio(key, &req.body).await,
                    _ => Err(ShimError::MethodNotAllowed),
                },
                "/api/v1/fromradio" => match req.method {
                    Method::GET => self.fromradio(key).await,
                    _ => Err(ShimError::MethodNotAllowed),
                },
                _ => Err(ShimError::NotFound),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_round_trip_and_bounds() {
        let f = frame(b"\x08\x01").unwrap();
        assert_eq!(f, [0x94, 0xc3, 0x00, 0x02, 0x08, 0x01]);
        let max = vec![7u8; MAX_FRAME];
        assert_eq!(frame(&max).unwrap()[2..4], [0x02, 0x00]);
        assert_eq!(
            frame(&[0u8; MAX_FRAME + 1]),
            Err(ShimError::TooLarge { limit: MAX_FRAME })
        );
        assert!(matches!(frame(b""), Err(ShimError::BadRequest(_))));
        let mut d = Deframer::new();
        assert_eq!(d.push(&f), vec![b"\x08\x01".to_vec()]);
        assert_eq!(d.pending(), 0);
    }

    #[test]
    fn deframer_skips_noise_split_reads_and_bad_lengths() {
        let mut d = Deframer::new();
        let a = frame(b"one").unwrap();
        let b = frame(b"two").unwrap();
        // debug text, then a frame split across reads, then a header claiming
        // 0xFFFF bytes (corrupt) followed by a real frame
        let mut stream = b"INFO | boot\r\n".to_vec();
        stream.extend_from_slice(&a);
        stream.extend_from_slice(&[0x94, 0xc3, 0xff, 0xff]);
        stream.extend_from_slice(&b);
        let mut got = Vec::new();
        for chunk in stream.chunks(3) {
            got.extend(d.push(chunk));
        }
        assert_eq!(got, vec![b"one".to_vec(), b"two".to_vec()]);
        // a lone START1 at the end is kept for the next read
        assert!(d.push(&[b'x', 0x94]).is_empty());
        assert_eq!(d.pending(), 1);
        assert_eq!(d.push(&[0xc3, 0, 1, 9]), vec![vec![9u8]]);
        // garbage alone never accumulates
        assert!(d.push(&[0u8; 10_000]).is_empty());
        assert!(d.pending() <= 1);
        // a zero-length frame is a frame
        assert_eq!(d.push(&[0x94, 0xc3, 0, 0]), vec![Vec::<u8>::new()]);
    }

    #[test]
    fn queue_drops_oldest_and_counts() {
        let mut q = FrameQueue::new(3);
        for i in 0..5u8 {
            q.push(vec![i]);
        }
        assert_eq!(q.len(), 3);
        assert_eq!(q.dropped(), 2);
        assert_eq!(q.pop(), Some(vec![2]));
        assert_eq!(q.pop(), Some(vec![3]));
        assert_eq!(q.pop(), Some(vec![4]));
        assert!(q.is_empty());
        assert_eq!(q.pop(), None);
    }
}
