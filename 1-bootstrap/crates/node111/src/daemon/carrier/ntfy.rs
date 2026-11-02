//! Layer 7 (§6.2): the ntfy carrier prototype (row P1; §6.1, §6.2, §6.4; D30 F3, Q15–Q16), wired
//! to the forwarder by row D11.
//!
//! [`NtfyCarrier`] implements [`super::Carrier`] over its own `ureq::Agent`, built the way
//! `wan_probe` builds one (`N/services/probe.rs:259-268`), never the updater's `Fetcher` (S6):
//!
//! - **5A** (§6.2 "5A flush"): every request runs in `spawn_blocking` on an agent with connect and
//!   global timeouts and no idle pool, so no connection outlives the call that opened it.
//! - **5B** (§6.2 "Held"): [`Carrier::hold`] runs the held `GET` on one named std thread,
//!   [`THREAD_5B`], on an agent with no global timeout and a read timeout of 3 × the server's
//!   keepalive interval, reconnecting with back-off (0.5 s doubling to 60 s, the `tcp-lan` dial
//!   bounds). The thread exists only while a held stream is open (§16).
//! - **Topics** (§6.2 "Topic"): [`topic_key`] and [`topic`], one topic per direction, 35 chars.
//! - **Batches** (§6.2 "Batch"): `0x01 ‖ n ‖ (len u16 BE ‖ frame)…`, base64url without padding,
//!   ≤ 4,096 B, so [`MAX_FRAME`] = 3,068 B; frames go out in custody order, packed into as few
//!   messages as fit, never one message per fragment (§6.1 rule 2).
//! - **Requests** (§6.2 "Requests"): publish `POST <server>/<topic(me→peer)>` `text/plain` (+ the
//!   token as `Authorization: Bearer`, *unverified from this box*); drain `GET
//!   <server>/<topic(peer→me)>/json?poll=1&since=<id>` (NDJSON); a `since` miss is
//!   [`CarrierError::SinceMiss`] and the next read is `since=all` (*unverified from this box*).
//! - **Outcomes** (§5.9): 2xx ⇒ `Taken`; 413, 401, 403 ⇒ `Rejected`; 429, 5xx, no route ⇒ `Retry`;
//!   a status §5.9 does not name ⇒ `Rejected` (fail closed: the row returns, never dropped).
//!
//! The §5.9 re-post rule, the consent record, the metered budget and the licensed-board rule
//! (§6.1 rule 6, §12) are D11's; [`NtfyCarrier::flush_5a`] is the §6.2 5A order (drain, post,
//! drain, stop) over a [`HandOff`], the prototype's smoke sequence. Nothing here is built while
//! `carrier.kind = "none"` (§6.3): this module holds no static state, starts nothing at load, and
//! an [`NtfyCarrier`] opens no socket, thread or file until one of its calls runs.

use std::collections::{HashMap, VecDeque};
use std::io::{self, BufRead, BufReader};
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::{FutureExt, StreamExt};
use id111::keyset::NodeId;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, DefaultConnector, NextTimeout, Transport,
};
use zeroize::{Zeroize, Zeroizing};

use super::{Carrier, CarrierError, CarrierFrame, ForwardDone, HandOff, Pair};

/// [`Carrier::id`] (§5.9).
pub const ID: &str = "ntfy";
/// The L2 domain tag of the topic key (§6.2, §18).
pub const TOPIC_TAG: &[u8] = b"111/carrier-topic/v1";
/// Every topic starts with this (§6.2).
pub const TOPIC_PREFIX: &str = "111";
/// `"111"` + 32 base32 characters of a 20-byte digest (§6.2).
pub const TOPIC_LEN: usize = 35;
/// The batch format version (§6.2 "Batch").
pub const BATCH_VERSION: u8 = 0x01;
/// The largest ntfy message body this carrier posts: "longer messages are treated as attachments"
/// (§6.2; R07g1 §3 item 2).
pub const MAX_BODY: usize = 4096;
/// The raw bytes 4,096 base64 characters carry.
pub const MAX_RAW: usize = MAX_BODY / 4 * 3;
/// The largest sealed frame one batch holds: 3,072 − 1 (version) − 1 (n) − 2 (len) (§6.2).
pub const MAX_FRAME: usize = MAX_RAW - 4;
/// The largest item a forward put through ntfy carries: 3,068 − 120 (wrap) − 11 (hint) (§6.2).
pub const MAX_ITEM: usize = MAX_FRAME - 120 - 11;
/// `n` is one byte.
pub const MAX_FRAMES_PER_BATCH: usize = 255;
/// The 5B thread's name (§6.2, §16): the only thread this module ever starts.
pub const THREAD_5B: &str = "111d-ntfy-5b";
/// ntfy's default `keepalive-interval` (upstream `server/server.yml`; *unverified from this box*:
/// the smoke log measures the server's own, and the setting overrides this).
pub const DEFAULT_KEEPALIVE: Duration = Duration::from_secs(45);
/// 5A and 5B: TCP connect plus the TLS handshake.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 5A: one whole request, from the DNS lookup to the end of the response body.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// One NDJSON line: a message event is ≈ 4,096 B of body plus ≈ 200 B of JSON.
const MAX_LINE: usize = 16 * 1024;
/// A line longer than [`MAX_LINE`] is skipped; past this many discarded bytes the read stops.
const MAX_DISCARD: usize = 1024 * 1024;
/// One 5A drain reads at most this many messages; the rest come with the next drain.
const MAX_DRAIN_MESSAGES: usize = 256;
/// One 5A drain reads at most this many response bytes.
const MAX_DRAIN_BYTES: u64 = 2 * 1024 * 1024;
/// A publish answer is one JSON object; more is read no further.
const MAX_PUBLISH_ANSWER: u64 = 64 * 1024;
/// ntfy message ids are 12 alphanumerics upstream; anything else is never put in a URL.
const MAX_ID: usize = 64;
/// An access token is printable ASCII.
const MAX_TOKEN: usize = 512;
const MAX_SERVER_URL: usize = 2048;
/// 5B reconnect back-off bounds (the `tcp-lan` dial bounds, `M/link/tcp_lan.rs:128-129`).
const BACKOFF_MIN: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// Deliveries the 5B thread queues ahead of the consumer (backpressure beyond: the socket).
const HOLD_QUEUE: usize = 16;
/// Request records kept when [`NtfyConfig::trace`] is on (the smoke log's instrument).
const TRACE_CAP: usize = 64;

// ---------------------------------------------------------------- topics (§6.2 "Topic")

/// `topic_key = BLAKE2b-256(key, "111/carrier-topic/v1" ‖ lo(enc_pk_a, enc_pk_b) ‖ hi(…))` (§6.2).
///
/// `key` is the X25519 output of the pairing (`X25519(own_enc_sk, peer_enc_pk)`, computed behind
/// the identity handle, row M5) or `carrier.ntfy.topic_secret` when that is set; `lo`/`hi` order
/// the two encryption public keys bytewise, so both hubs derive the same key. An all-zero `key`
/// (a low-order peer key, or an unset secret) is refused: `None` (fail closed). The key names a
/// mailbox; it protects no content (L5 §18 rule 5).
pub fn topic_key(
    key: &[u8; 32],
    enc_pk_a: &[u8; 32],
    enc_pk_b: &[u8; 32],
) -> Option<Zeroizing<[u8; 32]>> {
    use blake2::digest::consts::U32;
    use blake2::digest::{FixedOutput, KeyInit, Update};
    if key.iter().all(|&b| b == 0) {
        return None;
    }
    let (lo, hi) = if enc_pk_a <= enc_pk_b {
        (enc_pk_a, enc_pk_b)
    } else {
        (enc_pk_b, enc_pk_a)
    };
    let mut mac = <blake2::Blake2bMac<U32> as KeyInit>::new_from_slice(key).ok()?;
    Update::update(&mut mac, TOPIC_TAG);
    Update::update(&mut mac, lo);
    Update::update(&mut mac, hi);
    let mut out = mac.finalize_fixed();
    let mut k = Zeroizing::new([0u8; 32]);
    k.copy_from_slice(&out);
    out.zeroize();
    Some(k)
}

/// `topic(X→Y) = "111" ‖ base32-lower(BLAKE2b-160(key = topic_key, NodeId_X ‖ NodeId_Y))`, 35
/// characters, one topic per direction (§6.2). ntfy's topic alphabet and length limit are
/// *unverified from this box* (upstream's `[-_A-Za-z0-9]{1,64}` as read by R07g1).
pub fn topic(topic_key: &[u8; 32], from: &NodeId, to: &NodeId) -> String {
    use blake2::digest::consts::U20;
    use blake2::digest::{FixedOutput, KeyInit, Update};
    let mut out = String::with_capacity(TOPIC_LEN);
    out.push_str(TOPIC_PREFIX);
    // A 32-byte key always fits BLAKE2b's 64-byte key limit.
    if let Ok(mut mac) = <blake2::Blake2bMac<U20> as KeyInit>::new_from_slice(topic_key) {
        Update::update(&mut mac, from.as_bytes());
        Update::update(&mut mac, to.as_bytes());
        out.push_str(&base32_lower(&mac.finalize_fixed()));
    }
    out
}

/// This hub's outbound topic of `pair`: `topic(own→peer)`, where it publishes.
pub fn outbound_topic(pair: &Pair) -> String {
    topic(&pair.topic_key, &pair.own, &pair.peer)
}

/// This hub's inbound topic of `pair`: `topic(peer→own)`, where it drains and holds.
pub fn inbound_topic(pair: &Pair) -> String {
    topic(&pair.topic_key, &pair.peer, &pair.own)
}

/// RFC 4648 base32, lower case, unpadded (the topic suffix; 20 bytes → 32 characters).
fn base32_lower(data: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &b in data {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ALPHABET[((acc >> bits) & 31) as usize]));
        }
        acc &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(char::from(ALPHABET[((acc << (5 - bits)) & 31) as usize]));
    }
    out
}

// ---------------------------------------------------------------- base64url (RFC 4648 §5)

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64url without padding (the batch body, §6.2).
pub fn b64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() * 4).div_ceil(3));
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let v = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let symbols = chunk.len() + 1;
        for i in 0..symbols {
            out.push(char::from(B64URL[((v >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

fn b64url_value(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some(u32::from(c - b'A')),
        b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    }
}

/// Strict base64url without padding: the url alphabet only, no `=`, no whitespace, a length that
/// is not 1 mod 4, and zero trailing bits (one canonical text per byte string). `None` otherwise.
pub fn b64url_decode(text: &str) -> Option<Vec<u8>> {
    let s = text.as_bytes();
    if s.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3 + 2);
    for chunk in s.chunks(4) {
        let mut v: u32 = 0;
        for (i, &c) in chunk.iter().enumerate() {
            v |= b64url_value(c)? << (18 - 6 * i);
        }
        let bytes = [(v >> 16) as u8, (v >> 8) as u8, v as u8];
        let n = chunk.len() - 1;
        // the bits below the last whole byte must be zero (canonical form)
        if n < 3 && (v & ((1u32 << (8 * (3 - n))) - 1)) != 0 {
            return None;
        }
        out.extend_from_slice(&bytes[..n]);
    }
    Some(out)
}

// ---------------------------------------------------------------- batches (§6.2 "Batch")

/// Why a received ntfy message is not a batch (foreign bytes: counted, never trusted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchError {
    /// The body is longer than [`MAX_BODY`] (checked before decoding).
    TooLong,
    /// Not strict unpadded base64url.
    NotBase64,
    /// A version other than [`BATCH_VERSION`].
    Version,
    /// `n = 0`, or no bytes at all.
    Empty,
    /// A length runs past the end.
    Truncated,
    /// A frame of 0 B or over [`MAX_FRAME`].
    FrameLen,
    /// Bytes after the `n`th frame.
    Trailing,
}

impl std::fmt::Display for BatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            BatchError::TooLong => "batch: body over 4,096 B",
            BatchError::NotBase64 => "batch: not base64url",
            BatchError::Version => "batch: unknown version",
            BatchError::Empty => "batch: no frames",
            BatchError::Truncated => "batch: truncated",
            BatchError::FrameLen => "batch: frame length out of range",
            BatchError::Trailing => "batch: trailing bytes",
        })
    }
}

impl std::error::Error for BatchError {}

/// One batch body: `0x01 ‖ n ‖ (len u16 BE ‖ frame)…` in base64url (§6.2). `None` when `frames`
/// is empty, holds more than [`MAX_FRAMES_PER_BATCH`], holds an empty or over-[`MAX_FRAME`]
/// frame, or would exceed [`MAX_BODY`].
pub fn encode_batch(frames: &[&[u8]]) -> Option<String> {
    if frames.is_empty() || frames.len() > MAX_FRAMES_PER_BATCH {
        return None;
    }
    let raw_len = 2 + frames.iter().map(|f| 2 + f.len()).sum::<usize>();
    if raw_len > MAX_RAW || frames.iter().any(|f| f.is_empty() || f.len() > MAX_FRAME) {
        return None;
    }
    let mut raw = Vec::with_capacity(raw_len);
    raw.push(BATCH_VERSION);
    raw.push(frames.len() as u8);
    for f in frames {
        raw.extend_from_slice(&(f.len() as u16).to_be_bytes());
        raw.extend_from_slice(f);
    }
    Some(b64url_encode(&raw))
}

/// Parse one received batch body into its frames, in order. The size is checked before decoding.
pub fn decode_batch(body: &str) -> Result<Vec<Vec<u8>>, BatchError> {
    if body.len() > MAX_BODY {
        return Err(BatchError::TooLong);
    }
    let raw = b64url_decode(body).ok_or(BatchError::NotBase64)?;
    let (&version, rest) = raw.split_first().ok_or(BatchError::Empty)?;
    if version != BATCH_VERSION {
        return Err(BatchError::Version);
    }
    let (&n, mut rest) = rest.split_first().ok_or(BatchError::Empty)?;
    if n == 0 {
        return Err(BatchError::Empty);
    }
    let mut frames = Vec::with_capacity(usize::from(n));
    for _ in 0..n {
        if rest.len() < 2 {
            return Err(BatchError::Truncated);
        }
        let len = usize::from(u16::from_be_bytes([rest[0], rest[1]]));
        if len == 0 || len > MAX_FRAME {
            return Err(BatchError::FrameLen);
        }
        let body = rest.get(2..2 + len).ok_or(BatchError::Truncated)?;
        frames.push(body.to_vec());
        rest = &rest[2 + len..];
    }
    if !rest.is_empty() {
        return Err(BatchError::Trailing);
    }
    Ok(frames)
}

/// Where one frame of a flush goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// Into batch `i`.
    Batch(usize),
    /// Never posted (§6.1 rule 3: the fabric gate refuses such a put; this is the carrier's own
    /// guard).
    Refused(&'static str),
}

/// Pack frames greedily in custody order: a batch closes when the next frame would push it over
/// [`MAX_RAW`] or [`MAX_FRAMES_PER_BATCH`] (§6.1 rule 2, §6.2). Returns one slot per frame and
/// the batch bodies.
fn plan_batches(frames: &[CarrierFrame]) -> (Vec<Slot>, Vec<String>) {
    let mut slots = Vec::with_capacity(frames.len());
    let mut bodies = Vec::new();
    let mut cur: Vec<&[u8]> = Vec::new();
    let mut cur_raw = 2usize;
    for f in frames {
        let len = f.bytes.len();
        if len == 0 {
            slots.push(Slot::Refused("ntfy: empty frame"));
            continue;
        }
        if len > MAX_FRAME {
            slots.push(Slot::Refused("ntfy: frame over max_frame (3,068 B)"));
            continue;
        }
        if cur.len() == MAX_FRAMES_PER_BATCH || cur_raw + 2 + len > MAX_RAW {
            if let Some(b) = encode_batch(&cur) {
                bodies.push(b);
            }
            cur.clear();
            cur_raw = 2;
        }
        cur.push(&f.bytes);
        cur_raw += 2 + len;
        slots.push(Slot::Batch(bodies.len()));
    }
    if let Some(b) = encode_batch(&cur) {
        bodies.push(b);
    }
    (slots, bodies)
}

// ---------------------------------------------------------------- HTTP outcomes (§5.9)

/// One request's outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    /// 2xx: the mailbox accepted (publish) or answered (drain).
    Ok,
    /// Transient: no route, 429, 5xx.
    Retry(String),
    /// Permanent: 413, 401, 403, and every other status (fail closed).
    Rejected(&'static str),
}

/// The §5.9 status mapping. A status the spec does not name (a redirect, which the agent never
/// follows, or another 4xx) is permanent: the row returns to the other targets, never dropped.
fn status_outcome(status: u16) -> Outcome {
    match status {
        200..=299 => Outcome::Ok,
        413 => Outcome::Rejected("ntfy: 413 message too large"),
        401 => Outcome::Rejected("ntfy: 401 unauthorized"),
        403 => Outcome::Rejected("ntfy: 403 forbidden"),
        429 => Outcome::Retry("ntfy: 429 too many requests".into()),
        500..=599 => Outcome::Retry(format!("ntfy: {status} server error")),
        _ => Outcome::Rejected("ntfy: unexpected status"),
    }
}

impl Outcome {
    fn done(&self) -> ForwardDone {
        match self {
            Outcome::Ok => ForwardDone::Taken,
            Outcome::Retry(_) => ForwardDone::Retry,
            Outcome::Rejected(why) => ForwardDone::Rejected { why },
        }
    }

    fn error(self) -> Option<CarrierError> {
        match self {
            Outcome::Ok => None,
            Outcome::Retry(why) => Some(CarrierError::Retry(why)),
            Outcome::Rejected(why) => Some(CarrierError::Rejected(why)),
        }
    }

    /// A publish answer after which the rest of a flush is not posted: a transient failure (the
    /// limiter or the route would refuse the next one too) or an auth refusal. A 413 concerns its
    /// own batch only.
    fn stops_flush(&self) -> bool {
        match self {
            Outcome::Ok => false,
            Outcome::Retry(_) => true,
            Outcome::Rejected(why) => !why.starts_with("ntfy: 413"),
        }
    }
}

// ---------------------------------------------------------------- the server URL (§6.3)

/// `carrier.ntfy.server` (§6.3): an `https://` URL; `http://` only for a private address (§9.5
/// T6, S12: the token then crosses that LAN in clear), which here means an IP literal in a
/// private, loopback or link-local range, or `localhost`; a host name over `http://` is refused
/// (it could resolve anywhere). Returns the base without a trailing `/`.
fn check_server(url: &str) -> Result<String, CarrierError> {
    const BAD: &str = "ntfy: the server must be https:// (http:// only for a private address)";
    if url.is_empty()
        || url.len() > MAX_SERVER_URL
        || url.bytes().any(|b| !(0x21..=0x7e).contains(&b))
        || url.contains(['?', '#'])
    {
        return Err(CarrierError::Rejected(BAD));
    }
    let uri: ureq::http::Uri = url.parse().map_err(|_| CarrierError::Rejected(BAD))?;
    let host = uri.host().ok_or(CarrierError::Rejected(BAD))?;
    match uri.scheme_str() {
        Some("https") => {}
        Some("http") if is_private_host(host) => {}
        _ => return Err(CarrierError::Rejected(BAD)),
    }
    Ok(url.trim_end_matches('/').to_string())
}

fn is_private_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    match bare.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        Ok(IpAddr::V6(v6)) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.is_private() || v4.is_loopback() || v4.is_link_local();
            }
            let s0 = v6.segments()[0];
            v6.is_loopback() || (s0 & 0xfe00) == 0xfc00 || (s0 & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

// ---------------------------------------------------------------- NDJSON (§6.2 "Drain", "Held")

/// What one line read gave.
#[derive(Debug, PartialEq, Eq)]
enum Line {
    /// A line is in the buffer (without its `\n` or `\r\n`).
    Complete,
    /// A line over the cap was read through and dropped.
    TooLong,
    /// The body ended.
    Eof,
}

/// Read one `\n`-terminated line of at most `cap` bytes into `out`, never allocating past `cap`;
/// a longer line is consumed and dropped ([`Line::TooLong`]); past [`MAX_DISCARD`] dropped bytes
/// the read fails (a hostile stream never ends a line).
fn read_line_capped<R: BufRead>(r: &mut R, out: &mut Vec<u8>, cap: usize) -> io::Result<Line> {
    out.clear();
    let mut too_long = false;
    let mut discarded = 0usize;
    loop {
        let avail = r.fill_buf()?;
        if avail.is_empty() {
            return Ok(if too_long {
                Line::TooLong
            } else if out.is_empty() {
                Line::Eof
            } else {
                Line::Complete
            });
        }
        let (take, ended) = match avail.iter().position(|&b| b == b'\n') {
            Some(i) => (i, true),
            None => (avail.len(), false),
        };
        if !too_long && out.len() + take > cap {
            too_long = true;
            discarded = out.len();
            out.clear();
        }
        if too_long {
            discarded += take;
        } else {
            out.extend_from_slice(&avail[..take]);
        }
        r.consume(take + usize::from(ended));
        if ended {
            if too_long {
                return Ok(Line::TooLong);
            }
            if out.last() == Some(&b'\r') {
                out.pop();
            }
            return Ok(Line::Complete);
        }
        if discarded > MAX_DISCARD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ntfy: line too long",
            ));
        }
    }
}

#[derive(serde::Deserialize)]
struct WireEvent {
    #[serde(default)]
    id: String,
    #[serde(default)]
    event: String,
    #[serde(default)]
    topic: String,
    #[serde(default)]
    message: Option<String>,
}

/// One NDJSON event as this carrier reads it (foreign bytes: every field checked).
#[derive(Debug, PartialEq, Eq)]
enum Event {
    /// `open`, `poll_request` or an event this carrier does not use.
    Other,
    /// The server's keepalive (5B).
    Keepalive,
    /// A message: its id, and its frames or why it is no batch.
    Message {
        id: String,
        frames: Result<Vec<Vec<u8>>, BatchError>,
    },
    /// Not JSON, a bad id, or another topic's event: dropped and counted.
    Malformed,
}

fn parse_event(line: &[u8], topic: &str) -> Event {
    if line.iter().all(u8::is_ascii_whitespace) {
        return Event::Other;
    }
    let Ok(ev) = serde_json::from_slice::<WireEvent>(line) else {
        return Event::Malformed;
    };
    if !ev.topic.is_empty() && ev.topic != topic {
        return Event::Malformed;
    }
    match ev.event.as_str() {
        "keepalive" => Event::Keepalive,
        "message" => {
            if !valid_id(&ev.id) {
                return Event::Malformed;
            }
            let frames = match ev.message.as_deref() {
                Some(m) => decode_batch(m),
                None => Err(BatchError::Empty),
            };
            Event::Message { id: ev.id, frames }
        }
        _ => Event::Other,
    }
}

/// Where a read starts (§6.2 "Drain").
#[derive(Debug, Clone, PartialEq, Eq)]
enum Since {
    /// `since=all`: no saved id, or after a `since` miss (*unverified from this box*).
    All,
    /// `since=<id>`: the last id the fabric took.
    Id(String),
}

impl Since {
    fn query(&self) -> &str {
        match self {
            Since::All => "all",
            Since::Id(id) => id,
        }
    }
}

// ---------------------------------------------------------------- the carrier

/// The ntfy carrier's configuration (§6.3 `carrier.ntfy.*`; D11 reads it from `state/relay.json`).
pub struct NtfyConfig {
    /// `carrier.ntfy.server`: `https://…`, or `http://` for a private address only.
    pub server: String,
    /// `carrier.ntfy.token`: the pair's access token on an `auth-file` server; a secret, never on
    /// any API.
    pub token: Option<Zeroizing<String>>,
    /// The server's keepalive interval (the smoke log's measure); the 5B read timeout is 3 × it.
    pub keepalive: Duration,
    /// 5A and 5B connect timeout (TCP and TLS).
    pub connect_timeout: Duration,
    /// 5A global timeout per request.
    pub request_timeout: Duration,
    /// Keep the last [`TRACE_CAP`] request records ([`NtfyCarrier::take_trace`]): the smoke log's
    /// per-request method, status, bytes and latency. Off in the daemon.
    pub trace: bool,
}

impl NtfyConfig {
    /// The defaults for `server`: no token, [`DEFAULT_KEEPALIVE`], [`DEFAULT_CONNECT_TIMEOUT`],
    /// [`DEFAULT_REQUEST_TIMEOUT`], no trace.
    pub fn new(server: impl Into<String>) -> Self {
        NtfyConfig {
            server: server.into(),
            token: None,
            keepalive: DEFAULT_KEEPALIVE,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            trace: false,
        }
    }
}

/// One request as the smoke log records it (§6.4): method, status, body bytes, latency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRecord {
    /// `POST` or `GET`.
    pub method: &'static str,
    /// `publish`, `poll` (5A drain) or `stream` (5B; latency to the response head).
    pub kind: &'static str,
    /// The HTTP status, `None` when no answer came (no route, timeout).
    pub status: Option<u16>,
    /// Request body bytes.
    pub sent: usize,
    /// Response body bytes read (for `stream`: 0, the stream's bytes are not one answer).
    pub received: usize,
    /// From the call to the end of the answer this carrier read.
    pub latency: Duration,
    /// The transport error, when there was no answer.
    pub error: Option<String>,
}

/// Counters since the carrier was built (for `RelayStatus`, D11, and the smoke log).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NtfyStats {
    /// Publish requests answered 2xx.
    pub posts: u64,
    /// Body bytes of those posts.
    pub post_bytes: u64,
    /// Frames in those posts.
    pub frames_out: u64,
    /// 5A drains answered 2xx.
    pub drains: u64,
    /// Frames read (5A and 5B).
    pub frames_in: u64,
    /// Received events dropped: not JSON, a bad id, another topic, not a batch, an over-long line.
    pub malformed: u64,
    /// `since` misses (§5.9 re-post trigger 3).
    pub since_misses: u64,
    /// 5B keepalive events.
    pub keepalives: u64,
    /// The last gap between two 5B keepalives, in ms (the server's interval, measured).
    pub keepalive_gap_ms: u64,
    /// 5B connections opened after the first.
    pub reconnects: u64,
}

#[derive(Default)]
struct Counters {
    posts: AtomicU64,
    post_bytes: AtomicU64,
    frames_out: AtomicU64,
    drains: AtomicU64,
    frames_in: AtomicU64,
    malformed: AtomicU64,
    since_misses: AtomicU64,
    keepalives: AtomicU64,
    keepalive_gap_ms: AtomicU64,
    reconnects: AtomicU64,
}

fn bump(c: &AtomicU64, by: u64) {
    c.fetch_add(by, Ordering::Relaxed);
}

/// The drain position of one inbound topic (§6.2 "Drain", C13).
#[derive(Default)]
struct Cursor {
    /// The last id the fabric took: what `state/relay.json` saves (D11) and where reads start.
    committed: Option<String>,
    /// The last id a read returned since the last commit.
    seen: Option<String>,
}

struct Inner {
    server: String,
    /// `Bearer <token>`, when a token is set.
    auth: Option<Zeroizing<String>>,
    /// The 5A agent: connect and global timeouts, no idle pool.
    agent: ureq::Agent,
    keepalive: Duration,
    connect_timeout: Duration,
    cursors: Mutex<HashMap<String, Cursor>>,
    counters: Counters,
    trace: Option<Mutex<VecDeque<RequestRecord>>>,
    /// The 5B thread is alive.
    holding: AtomicBool,
}

/// The ntfy carrier (§6.2). One per configured server; it serves every `carrier` pair, each by its
/// own two topics.
pub struct NtfyCarrier {
    inner: Arc<Inner>,
}

/// What [`NtfyCarrier::drain_into`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Drained {
    /// Frames handed to [`HandOff::carrier_inbound`].
    pub frames: usize,
    /// The first read was a `since` miss and the frames come from `since=all` (§5.9 re-post
    /// trigger 3).
    pub since_miss: bool,
    /// Every frame was taken, so the cursor moved.
    pub all_taken: bool,
    /// The cursor after the call (what D11 saves in `state/relay.json`).
    pub committed: Option<String>,
}

/// What [`NtfyCarrier::flush_5a`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flushed {
    /// The first drain (acks first; the §3.2 check).
    pub first: Drained,
    /// Frames `forward_take` returned.
    pub taken_from_fabric: usize,
    /// Of those: posted (`Taken`), permanent (`Rejected`), transient (`Retry`).
    pub posted: usize,
    pub rejected: usize,
    pub retry: usize,
    /// Of those: acks (consumed on take, C7).
    pub acks: usize,
    /// The drain after the post.
    pub second: Drained,
}

impl NtfyCarrier {
    /// Build the carrier: checks the server URL (§6.3) and the token, builds the 5A agent. Opens
    /// nothing.
    pub fn new(cfg: NtfyConfig) -> Result<Self, CarrierError> {
        let server = check_server(&cfg.server)?;
        let auth = match cfg.token {
            None => None,
            Some(t) => {
                if t.is_empty()
                    || t.len() > MAX_TOKEN
                    || t.bytes().any(|b| !(0x21..=0x7e).contains(&b))
                {
                    return Err(CarrierError::Rejected(
                        "ntfy: the token is not printable ASCII",
                    ));
                }
                Some(Zeroizing::new(format!("Bearer {}", t.as_str())))
            }
        };
        if cfg.keepalive.is_zero() || cfg.connect_timeout.is_zero() || cfg.request_timeout.is_zero()
        {
            return Err(CarrierError::Rejected("ntfy: a zero interval"));
        }
        let config = base_config()
            .timeout_connect(Some(cfg.connect_timeout))
            .timeout_global(Some(cfg.request_timeout))
            .build();
        Ok(NtfyCarrier {
            inner: Arc::new(Inner {
                server,
                auth,
                agent: ureq::Agent::new_with_config(config),
                keepalive: cfg.keepalive,
                connect_timeout: cfg.connect_timeout,
                cursors: Mutex::new(HashMap::new()),
                counters: Counters::default(),
                trace: cfg.trace.then(|| Mutex::new(VecDeque::new())),
                holding: AtomicBool::new(false),
            }),
        })
    }

    /// The configured server base.
    pub fn server(&self) -> &str {
        &self.inner.server
    }

    /// The 5B read timeout: 3 × the keepalive interval (§6.2).
    pub fn read_timeout(&self) -> Duration {
        self.inner.keepalive.saturating_mul(3)
    }

    /// The saved drain position of `pair`'s inbound topic (the id the fabric last took).
    pub fn cursor(&self, pair: &Pair) -> Option<String> {
        let topic = inbound_topic(pair);
        self.inner.with_cursor(&topic, |c| c.committed.clone())
    }

    /// Restore the drain position read back from `state/relay.json` (D11). An id that is not a
    /// plausible ntfy id is refused (it would go into a URL).
    pub fn restore_cursor(&self, pair: &Pair, id: Option<String>) -> Result<(), CarrierError> {
        if let Some(i) = &id {
            if !valid_id(i) {
                return Err(CarrierError::Rejected("ntfy: not a message id"));
            }
        }
        let topic = inbound_topic(pair);
        self.inner.with_cursor(&topic, |c| {
            c.committed = id;
            c.seen = None;
        });
        Ok(())
    }

    /// The fabric took every frame the reads since the last commit returned: move the cursor to
    /// the last id they read and return it, for `state/relay.json` (C13: the id is saved only
    /// after the fabric took the batch).
    pub fn commit_drain(&self, pair: &Pair) -> Option<String> {
        let topic = inbound_topic(pair);
        self.inner.with_cursor(&topic, |c| {
            if let Some(seen) = c.seen.take() {
                c.committed = Some(seen);
            }
            c.committed.clone()
        })
    }

    /// The counters.
    pub fn stats(&self) -> NtfyStats {
        let c = &self.inner.counters;
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        NtfyStats {
            posts: g(&c.posts),
            post_bytes: g(&c.post_bytes),
            frames_out: g(&c.frames_out),
            drains: g(&c.drains),
            frames_in: g(&c.frames_in),
            malformed: g(&c.malformed),
            since_misses: g(&c.since_misses),
            keepalives: g(&c.keepalives),
            keepalive_gap_ms: g(&c.keepalive_gap_ms),
            reconnects: g(&c.reconnects),
        }
    }

    /// The request records kept since the last call (only with [`NtfyConfig::trace`]).
    pub fn take_trace(&self) -> Vec<RequestRecord> {
        match &self.inner.trace {
            Some(t) => t
                .lock()
                .map(|mut q| q.drain(..).collect())
                .unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// The 5B thread is alive (a held stream is open, or its thread is still closing: it ends
    /// within one keepalive interval of the stream's drop, at once while it waits to reconnect).
    pub fn holding(&self) -> bool {
        self.inner.holding.load(Ordering::Acquire)
    }

    /// One 5A drain of `pair` handed to the fabric (§6.2 "Drain"): a `since` miss reads again
    /// with `since=all`; every frame goes to [`HandOff::carrier_inbound`]; the cursor moves only
    /// when every frame was taken (C13). `carrier_inbound` answers `Ok` for a frame it verified
    /// and dropped (a frame from anyone but the pair fails `pair_mac` and is counted there); an
    /// `Err` means the fabric could not process it, so the same frames are read again next time
    /// (the L2 window drops the ones already processed, `L5:375-377`).
    pub async fn drain_into(
        &self,
        pair: &Pair,
        handoff: &dyn HandOff,
    ) -> Result<Drained, CarrierError> {
        let mut since_miss = false;
        let frames = match self.drain(pair).await {
            Ok(f) => f,
            Err(CarrierError::SinceMiss) => {
                since_miss = true;
                self.drain(pair).await?
            }
            Err(e) => return Err(e),
        };
        let n = frames.len();
        let mut all_taken = true;
        for f in frames {
            if handoff.carrier_inbound(pair.peer, f).await.is_err() {
                all_taken = false;
            }
        }
        let committed = if all_taken {
            self.commit_drain(pair)
        } else {
            self.cursor(pair)
        };
        Ok(Drained {
            frames: n,
            since_miss,
            all_taken,
            committed,
        })
    }

    /// The §6.2 5A flush over `handoff`: drain (acks first, so acked rows are not posted again),
    /// post what `forward_take(peer, max)` returns (in custody order, batched), report each put's
    /// outcome through `forward_done`, drain once, stop. Between flushes nothing remains: no
    /// connection, thread or timer. The first drain is the §3.2 check: when it fails, nothing is
    /// taken or posted. The §5.9 re-post rule, consent, the metered budget and the licensed-board
    /// rule are the caller's (D11).
    pub async fn flush_5a(
        &self,
        pair: &Pair,
        handoff: &dyn HandOff,
        max: usize,
    ) -> Result<Flushed, CarrierError> {
        let first = self.drain_into(pair, handoff).await?;
        let out = handoff.forward_take(pair.peer, max).await?;
        let mut report = Flushed {
            first,
            taken_from_fabric: out.len(),
            ..Flushed::default()
        };
        let mut handoff_err = None;
        for (seq, done) in self.flush(pair, out).await {
            match &done {
                ForwardDone::Taken => report.posted += 1,
                ForwardDone::Rejected { .. } => report.rejected += 1,
                ForwardDone::Retry => report.retry += 1,
            }
            match seq {
                Some(s) => {
                    if let Err(e) = handoff.forward_done(s, done).await {
                        handoff_err.get_or_insert(e);
                    }
                }
                None => report.acks += 1,
            }
        }
        if let Some(e) = handoff_err {
            return Err(e);
        }
        report.second = self.drain_into(pair, handoff).await?;
        Ok(report)
    }
}

/// The agent settings 5A and 5B share: the WebPKI roots as `wan_probe` uses them, statuses as
/// answers, no redirect followed (a redirect is an unexpected status), no idle connection kept.
fn base_config() -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::WebPki)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .http_status_as_error(false)
        .max_redirects(0)
        .max_idle_connections(0)
        .max_idle_connections_per_host(0)
}

impl Inner {
    fn with_cursor<T>(&self, topic: &str, f: impl FnOnce(&mut Cursor) -> T) -> T {
        let mut map = self.cursors.lock().unwrap_or_else(|p| p.into_inner());
        f(map.entry(topic.to_string()).or_default())
    }

    fn since(&self, topic: &str) -> Since {
        self.with_cursor(topic, |c| match &c.committed {
            Some(id) => Since::Id(id.clone()),
            None => Since::All,
        })
    }

    fn set_seen(&self, topic: &str, id: String) {
        self.with_cursor(topic, |c| c.seen = Some(id));
    }

    /// A `since` miss: the saved id is gone, so the next read is `since=all`.
    fn since_missed(&self, topic: &str) {
        bump(&self.counters.since_misses, 1);
        self.with_cursor(topic, |c| {
            c.committed = None;
            c.seen = None;
        });
    }

    fn record(&self, r: RequestRecord) {
        if let Some(t) = &self.trace {
            if let Ok(mut q) = t.lock() {
                if q.len() == TRACE_CAP {
                    q.pop_front();
                }
                q.push_back(r);
            }
        }
    }

    fn get(
        &self,
        agent: &ureq::Agent,
        url: &str,
    ) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
        let mut req = agent.get(url);
        if let Some(a) = &self.auth {
            req = req.header("Authorization", a.as_str());
        }
        req.call()
    }

    /// One publish (blocking).
    fn post_blocking(&self, topic: &str, body: &str, frames: usize) -> Outcome {
        let url = format!("{}/{}", self.server, topic);
        let started = Instant::now();
        let mut req = self.agent.post(&url).content_type("text/plain");
        if let Some(a) = &self.auth {
            req = req.header("Authorization", a.as_str());
        }
        match req.send(body) {
            Ok(mut resp) => {
                let status = resp.status().as_u16();
                let received = resp
                    .body_mut()
                    .with_config()
                    .limit(MAX_PUBLISH_ANSWER)
                    .read_to_vec()
                    .map(|v| v.len())
                    .unwrap_or(0);
                self.record(RequestRecord {
                    method: "POST",
                    kind: "publish",
                    status: Some(status),
                    sent: body.len(),
                    received,
                    latency: started.elapsed(),
                    error: None,
                });
                let outcome = status_outcome(status);
                if outcome == Outcome::Ok {
                    bump(&self.counters.posts, 1);
                    bump(&self.counters.post_bytes, body.len() as u64);
                    bump(&self.counters.frames_out, frames as u64);
                }
                outcome
            }
            Err(e) => {
                self.record(RequestRecord {
                    method: "POST",
                    kind: "publish",
                    status: None,
                    sent: body.len(),
                    received: 0,
                    latency: started.elapsed(),
                    error: Some(e.to_string()),
                });
                Outcome::Retry(format!("ntfy: publish: {e}"))
            }
        }
    }

    /// Post every batch in order; after a batch that stops the flush, the rest are not posted and
    /// stay (`Retry`).
    fn post_all(&self, topic: &str, bodies: &[(String, usize)]) -> Vec<Outcome> {
        let mut out = Vec::with_capacity(bodies.len());
        let mut stopped = false;
        for (body, frames) in bodies {
            if stopped {
                out.push(Outcome::Retry(
                    "ntfy: not posted after an earlier refusal".into(),
                ));
                continue;
            }
            let o = self.post_blocking(topic, body, *frames);
            stopped = o.stops_flush();
            out.push(o);
        }
        out
    }

    /// One 5A poll (blocking): `GET …/json?poll=1&since=<id|all>`, NDJSON, bounded.
    fn poll_blocking(&self, topic: &str, since: &Since) -> Result<Polled, CarrierError> {
        let url = format!(
            "{}/{}/json?poll=1&since={}",
            self.server,
            topic,
            since.query()
        );
        let started = Instant::now();
        let resp = match self.get(&self.agent, &url) {
            Ok(r) => r,
            Err(e) => {
                self.record(RequestRecord {
                    method: "GET",
                    kind: "poll",
                    status: None,
                    sent: 0,
                    received: 0,
                    latency: started.elapsed(),
                    error: Some(e.to_string()),
                });
                return Err(CarrierError::Retry(format!("ntfy: poll: {e}")));
            }
        };
        let status = resp.status().as_u16();
        if let Some(err) = status_outcome(status).error() {
            self.record(RequestRecord {
                method: "GET",
                kind: "poll",
                status: Some(status),
                sent: 0,
                received: 0,
                latency: started.elapsed(),
                error: None,
            });
            // A refused `since=<id>` is the miss (§6.2): the next read is `since=all`.
            if status == 400 && matches!(since, Since::Id(_)) {
                return Err(CarrierError::SinceMiss);
            }
            return Err(err);
        }
        let mut reader = BufReader::new(
            resp.into_body()
                .into_with_config()
                .limit(MAX_DRAIN_BYTES)
                .reader(),
        );
        let mut polled = Polled::default();
        let mut line = Vec::new();
        let mut received = 0usize;
        let mut cut = None;
        while polled.messages < MAX_DRAIN_MESSAGES {
            match read_line_capped(&mut reader, &mut line, MAX_LINE) {
                Ok(Line::Eof) => break,
                Ok(Line::TooLong) => polled.malformed += 1,
                Ok(Line::Complete) => {
                    received += line.len() + 1;
                    match parse_event(&line, topic) {
                        Event::Message { id, frames } => {
                            polled.messages += 1;
                            match frames {
                                Ok(f) => polled.frames.extend(f),
                                Err(_) => polled.malformed += 1,
                            }
                            polled.last_id = Some(id);
                        }
                        Event::Malformed => polled.malformed += 1,
                        Event::Keepalive | Event::Other => {}
                    }
                }
                Err(e) => {
                    cut = Some(e.to_string());
                    break;
                }
            }
        }
        self.record(RequestRecord {
            method: "GET",
            kind: "poll",
            status: Some(status),
            sent: 0,
            received,
            latency: started.elapsed(),
            error: cut.clone(),
        });
        bump(&self.counters.malformed, polled.malformed as u64);
        if let (Some(why), None) = (&cut, &polled.last_id) {
            return Err(CarrierError::Retry(format!("ntfy: poll: {why}")));
        }
        bump(&self.counters.drains, 1);
        bump(&self.counters.frames_in, polled.frames.len() as u64);
        Ok(polled)
    }
}

/// What one poll read.
#[derive(Default)]
struct Polled {
    frames: Vec<Vec<u8>>,
    messages: usize,
    malformed: usize,
    last_id: Option<String>,
}

fn join_error(e: tokio::task::JoinError) -> CarrierError {
    CarrierError::Retry(format!("ntfy: request task: {e}"))
}

impl Carrier for NtfyCarrier {
    fn id(&self) -> &'static str {
        ID
    }

    fn max_frame(&self) -> usize {
        MAX_FRAME
    }

    /// Post `out` to `topic(own→peer)` in custody order, packed into as few batches as fit, in one
    /// `spawn_blocking` (§6.2 "Publish"). One outcome per frame, in `out`'s order: a frame over
    /// [`MAX_FRAME`] or empty is `Rejected` and never posted; a batch's answer applies to each of
    /// its frames; after a 429, a 5xx, no route, a 401 or a 403 the rest stay (`Retry`).
    fn flush<'a>(
        &'a self,
        pair: &'a Pair,
        out: Vec<CarrierFrame>,
    ) -> BoxFuture<'a, Vec<(Option<u64>, ForwardDone)>> {
        async move {
            if out.is_empty() {
                return Vec::new();
            }
            let topic = outbound_topic(pair);
            let (slots, bodies) = plan_batches(&out);
            let mut sizes = vec![0usize; bodies.len()];
            for s in &slots {
                if let Slot::Batch(i) = s {
                    sizes[*i] += 1;
                }
            }
            let batches: Vec<(String, usize)> = bodies.into_iter().zip(sizes).collect();
            let n = batches.len();
            let inner = self.inner.clone();
            let outcomes = if n == 0 {
                Vec::new()
            } else {
                tokio::task::spawn_blocking(move || inner.post_all(&topic, &batches))
                    .await
                    .unwrap_or_else(|e| vec![Outcome::Retry(join_error(e).to_string()); n])
            };
            out.iter()
                .zip(slots)
                .map(|(f, slot)| {
                    let done = match slot {
                        Slot::Refused(why) => ForwardDone::Rejected { why },
                        Slot::Batch(i) => outcomes
                            .get(i)
                            .map(Outcome::done)
                            .unwrap_or(ForwardDone::Retry),
                    };
                    (f.custody_seq, done)
                })
                .collect()
        }
        .boxed()
    }

    /// One 5A poll of `topic(peer→own)` from the saved id (or `since=all`), in `spawn_blocking`
    /// (§6.2 "Drain"); the frames in order, at most 256 messages a call. A refused `since=<id>`
    /// (the server lost the id: a restart or an eviction) is [`CarrierError::SinceMiss`]: the
    /// saved id is dropped and the next call reads `since=all`. The cursor moves only through
    /// [`NtfyCarrier::commit_drain`].
    fn drain<'a>(&'a self, pair: &'a Pair) -> BoxFuture<'a, Result<Vec<Vec<u8>>, CarrierError>> {
        async move {
            let topic = inbound_topic(pair);
            let since = self.inner.since(&topic);
            let inner = self.inner.clone();
            let t = topic.clone();
            let polled = tokio::task::spawn_blocking(move || inner.poll_blocking(&t, &since))
                .await
                .map_err(join_error)?;
            match polled {
                Ok(p) => {
                    if let Some(id) = p.last_id {
                        self.inner.set_seen(&topic, id);
                    }
                    Ok(p.frames)
                }
                Err(CarrierError::SinceMiss) => {
                    self.inner.since_missed(&topic);
                    Err(CarrierError::SinceMiss)
                }
                Err(e) => Err(e),
            }
        }
        .boxed()
    }

    /// 5B (§6.2 "Held"): the held `GET …/json?since=<id|all>` of `topic(peer→own)` on the one
    /// thread [`THREAD_5B`]; frames as they arrive. The stream yields `Err(Retry)` when the
    /// connection drops (it reconnects with back-off), `Err(SinceMiss)` when the saved id is gone
    /// (it reconnects at once with `since=all`), and ends after `Err(Rejected)` (401, 403, …).
    /// One held stream per carrier: a second `hold` while the thread lives yields one
    /// `Err(Retry)` and ends. Dropping the stream ends the thread (within one keepalive interval
    /// while it reads, at once while it waits to reconnect). The cursor moves only through
    /// [`NtfyCarrier::commit_drain`].
    fn hold<'a>(&'a self, pair: &'a Pair) -> BoxStream<'a, Result<Vec<u8>, CarrierError>> {
        hold_stream(self.inner.clone(), inbound_topic(pair))
    }
}

// ---------------------------------------------------------------- 5B (§6.2 "Held")

/// What the 5B thread hands the stream: one message's frames and id.
struct Delivery {
    id: String,
    frames: Vec<Vec<u8>>,
}

type HoldItem = Result<Delivery, CarrierError>;

/// Clears [`Inner::holding`] when the 5B thread ends (or when its closure is dropped unrun).
struct HoldFlag(Arc<Inner>);

impl Drop for HoldFlag {
    fn drop(&mut self) {
        self.0.holding.store(false, Ordering::Release);
    }
}

struct HoldState {
    rx: tokio::sync::mpsc::Receiver<HoldItem>,
    /// Dropped with the stream: wakes the thread out of its back-off wait.
    _cancel: std::sync::mpsc::Sender<()>,
    buf: VecDeque<Vec<u8>>,
    pending_id: Option<String>,
    inner: Arc<Inner>,
    topic: String,
}

fn hold_stream(
    inner: Arc<Inner>,
    topic: String,
) -> BoxStream<'static, Result<Vec<u8>, CarrierError>> {
    let refused = |why: String| {
        futures_util::stream::once(async move { Err(CarrierError::Retry(why)) }).boxed()
    };
    if inner
        .holding
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return refused("ntfy: 5B: a held stream is already open".into());
    }
    let (tx, rx) = tokio::sync::mpsc::channel::<HoldItem>(HOLD_QUEUE);
    let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
    let flag = HoldFlag(inner.clone());
    let thread_inner = inner.clone();
    let thread_topic = topic.clone();
    let spawned = std::thread::Builder::new()
        .name(THREAD_5B.into())
        .spawn(move || {
            let _flag = flag;
            hold_thread(&thread_inner, &thread_topic, &tx, &cancel_rx);
        });
    if let Err(e) = spawned {
        return refused(format!("ntfy: 5B: thread: {e}"));
    }
    let state = HoldState {
        rx,
        _cancel: cancel_tx,
        buf: VecDeque::new(),
        pending_id: None,
        inner,
        topic,
    };
    futures_util::stream::unfold(state, |mut st| async move {
        loop {
            if let Some(f) = st.buf.pop_front() {
                if st.buf.is_empty() {
                    if let Some(id) = st.pending_id.take() {
                        st.inner.set_seen(&st.topic, id);
                    }
                }
                return Some((Ok(f), st));
            }
            match st.rx.recv().await? {
                Ok(d) if d.frames.is_empty() => st.inner.set_seen(&st.topic, d.id),
                Ok(d) => {
                    st.buf.extend(d.frames);
                    st.pending_id = Some(d.id);
                }
                Err(e) => return Some((Err(e), st)),
            }
        }
    })
    .boxed()
}

/// How one held connection ended.
enum StreamEnd {
    /// The stream was dropped.
    Closed,
    /// A permanent refusal: the stream ends.
    Rejected(&'static str),
    /// The saved id is gone: reconnect at once with `since=all`.
    SinceMiss,
    /// A drop or a transient refusal: reconnect after the back-off. `progress`: an event arrived
    /// after the open, so the back-off starts over.
    Dropped { progress: bool, why: String },
}

/// The 5B thread: hold, read, reconnect with back-off, until the stream is dropped or refused.
fn hold_thread(
    inner: &Arc<Inner>,
    topic: &str,
    tx: &tokio::sync::mpsc::Sender<HoldItem>,
    cancel: &std::sync::mpsc::Receiver<()>,
) {
    let config = base_config()
        .timeout_connect(Some(inner.connect_timeout))
        .build();
    let agent = ureq::Agent::with_parts(
        config,
        DefaultConnector::new().chain(ReadCap(inner.keepalive.saturating_mul(3))),
        ureq::unversioned::resolver::DefaultResolver::default(),
    );
    let mut since = inner.since(topic);
    let mut backoff = BACKOFF_MIN;
    let mut first = true;
    loop {
        if tx.is_closed() {
            return;
        }
        if !first {
            bump(&inner.counters.reconnects, 1);
        }
        first = false;
        match stream_once(inner, &agent, topic, &mut since, tx) {
            StreamEnd::Closed => return,
            StreamEnd::Rejected(why) => {
                let _ = tx.blocking_send(Err(CarrierError::Rejected(why)));
                return;
            }
            StreamEnd::SinceMiss => {
                inner.since_missed(topic);
                since = Since::All;
                backoff = BACKOFF_MIN;
                if tx.blocking_send(Err(CarrierError::SinceMiss)).is_err() {
                    return;
                }
            }
            StreamEnd::Dropped { progress, why } => {
                if progress {
                    backoff = BACKOFF_MIN;
                }
                if tx.blocking_send(Err(CarrierError::Retry(why))).is_err() {
                    return;
                }
                match cancel.recv_timeout(backoff) {
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    _ => return,
                }
                backoff = backoff.saturating_mul(2).min(BACKOFF_MAX);
            }
        }
    }
}

fn is_timeout(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::TimedOut
        || e.get_ref()
            .and_then(|x| x.downcast_ref::<ureq::Error>())
            .is_some_and(|x| matches!(x, ureq::Error::Timeout(_)))
}

/// One held connection: connect, read events until it ends; `since` follows the deliveries.
fn stream_once(
    inner: &Inner,
    agent: &ureq::Agent,
    topic: &str,
    since: &mut Since,
    tx: &tokio::sync::mpsc::Sender<HoldItem>,
) -> StreamEnd {
    let url = format!("{}/{}/json?since={}", inner.server, topic, since.query());
    let started = Instant::now();
    let record = |status: Option<u16>, error: Option<String>| RequestRecord {
        method: "GET",
        kind: "stream",
        status,
        sent: 0,
        received: 0,
        latency: started.elapsed(),
        error,
    };
    let resp = match inner.get(agent, &url) {
        Ok(r) => r,
        Err(e) => {
            inner.record(record(None, Some(e.to_string())));
            return StreamEnd::Dropped {
                progress: false,
                why: format!("ntfy: 5B: {e}"),
            };
        }
    };
    let status = resp.status().as_u16();
    inner.record(record(Some(status), None));
    match status_outcome(status) {
        Outcome::Ok => {}
        _ if status == 400 && matches!(since, Since::Id(_)) => return StreamEnd::SinceMiss,
        Outcome::Retry(why) => {
            return StreamEnd::Dropped {
                progress: false,
                why,
            }
        }
        Outcome::Rejected(why) => return StreamEnd::Rejected(why),
    }
    let mut reader = BufReader::new(resp.into_body().into_reader());
    let mut line = Vec::new();
    let mut progress = false;
    let mut last_keepalive: Option<Instant> = None;
    loop {
        if tx.is_closed() {
            return StreamEnd::Closed;
        }
        match read_line_capped(&mut reader, &mut line, MAX_LINE) {
            Ok(Line::Eof) => {
                return StreamEnd::Dropped {
                    progress,
                    why: "ntfy: 5B: the stream ended".into(),
                }
            }
            Ok(Line::TooLong) => bump(&inner.counters.malformed, 1),
            Ok(Line::Complete) => match parse_event(&line, topic) {
                Event::Keepalive => {
                    progress = true;
                    bump(&inner.counters.keepalives, 1);
                    let now = Instant::now();
                    if let Some(prev) = last_keepalive {
                        let gap = now.duration_since(prev).as_millis();
                        inner
                            .counters
                            .keepalive_gap_ms
                            .store(u64::try_from(gap).unwrap_or(u64::MAX), Ordering::Relaxed);
                    }
                    last_keepalive = Some(now);
                }
                Event::Message { id, frames } => {
                    progress = true;
                    let frames = frames.unwrap_or_else(|_| {
                        bump(&inner.counters.malformed, 1);
                        Vec::new()
                    });
                    bump(&inner.counters.frames_in, frames.len() as u64);
                    *since = Since::Id(id.clone());
                    if tx.blocking_send(Ok(Delivery { id, frames })).is_err() {
                        return StreamEnd::Closed;
                    }
                }
                Event::Malformed => bump(&inner.counters.malformed, 1),
                Event::Other => {}
            },
            Err(e) => {
                let why = if is_timeout(&e) {
                    "ntfy: 5B: read timeout (3 × keepalive)".to_string()
                } else {
                    format!("ntfy: 5B: {e}")
                };
                return StreamEnd::Dropped { progress, why };
            }
        }
    }
}

/// The 5B read timeout as a ureq connector (§6.2): ureq 3's own body timeout is one budget for
/// the whole body, so the held stream instead caps every single wait for input (and every write)
/// at 3 × the keepalive interval, last in the default chain (TCP, then TLS). It uses ureq's
/// `unversioned` transport API (`Agent::with_parts`), which ureq may change in a minor release;
/// the lockfile pins 3.4.2, and a bump that breaks it fails to compile here, never silently.
#[derive(Debug)]
struct ReadCap(Duration);

impl Connector<Box<dyn Transport>> for ReadCap {
    type Out = CappedTransport;

    fn connect(
        &self,
        _details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        Ok(chained.map(|inner| CappedTransport { inner, cap: self.0 }))
    }
}

#[derive(Debug)]
struct CappedTransport {
    inner: Box<dyn Transport>,
    cap: Duration,
}

impl CappedTransport {
    fn clamp(&self, t: NextTimeout) -> NextTimeout {
        if *t.after > self.cap {
            NextTimeout {
                after: ureq::unversioned::transport::time::Duration::Exact(self.cap),
                reason: ureq::Timeout::RecvBody,
            }
        } else {
            t
        }
    }
}

impl Transport for CappedTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        let t = self.clamp(timeout);
        self.inner.transmit_output(amount, t)
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let t = self.clamp(timeout);
        self.inner.await_input(t)
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nid(start: u8) -> NodeId {
        let mut b = [0u8; 32];
        for (i, x) in b.iter_mut().enumerate() {
            *x = start.wrapping_add(i as u8);
        }
        NodeId::from(b)
    }

    fn seq32(start: u8) -> [u8; 32] {
        nid(start).to_bytes()
    }

    /// Fixed vectors (§6.2 "Topic"), cross-checked with Python's hashlib (RFC 7693 keyed mode):
    ///
    /// ```text
    /// k = hashlib.blake2b(b"111/carrier-topic/v1" + lo + hi, key=key, digest_size=32).digest()
    /// t = "111" + base64.b32encode(hashlib.blake2b(x + y, key=k, digest_size=20).digest())
    ///             .decode().lower().rstrip("=")
    /// ```
    ///
    /// Vector 1: key = bytes(range(0, 32)) (the X25519 output stand-in), enc_pk_a =
    /// bytes(range(32, 64)), enc_pk_b = bytes(range(64, 96)), X = bytes(range(96, 128)), Y =
    /// bytes(range(128, 160)). Vector 2: key = b"\x42" * 32 (a `topic_secret`), enc_pk_a =
    /// b"\xff" * 32, enc_pk_b = b"\x01" * 32 (so lo = b, hi = a), X = b"\x5a" * 32, Y = b"\xa5" * 32.
    #[test]
    fn topic_known_answers() {
        let k = topic_key(&seq32(0), &seq32(32), &seq32(64)).unwrap();
        assert_eq!(
            id111::encoding::hex_encode(&k),
            "450910c1856aa7ddcff702fadc8f0bd57d04c4f0107e14b021df4195871d1c51"
        );
        assert_eq!(
            topic(&k, &nid(96), &nid(128)),
            "111w5juayuaplobelufg5pv5cjnakf2srhx"
        );
        assert_eq!(
            topic(&k, &nid(128), &nid(96)),
            "111bw2fqnkrwgvw2bhapn2g4mh642ciejoj"
        );

        let k2 = topic_key(&[0x42; 32], &[0xff; 32], &[0x01; 32]).unwrap();
        assert_eq!(
            id111::encoding::hex_encode(&k2),
            "db967e2d201e4d5544356e3470c39efdb4bfdcab3afa7d3e42bfe915cfa3ab7e"
        );
        let (x, y) = (NodeId::from([0x5a; 32]), NodeId::from([0xa5; 32]));
        assert_eq!(topic(&k2, &x, &y), "111lpjjwfgu3jxxcnmwn5lshf652v4xhimu");
        assert_eq!(topic(&k2, &y, &x), "111utuhudbw5w6lu5pdfqb4xo76l2fbdxey");
    }

    #[test]
    fn topic_key_is_symmetric_and_topics_are_directional() {
        let a = topic_key(&seq32(7), &seq32(1), &seq32(200)).unwrap();
        let b = topic_key(&seq32(7), &seq32(200), &seq32(1)).unwrap();
        assert_eq!(*a, *b, "both hubs derive one key");
        let t1 = topic(&a, &nid(3), &nid(9));
        let t2 = topic(&a, &nid(9), &nid(3));
        assert_ne!(t1, t2, "one topic per direction");
        for t in [&t1, &t2] {
            assert_eq!(t.len(), TOPIC_LEN);
            assert!(t.starts_with("111"));
            assert!(t
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
        }
        // a different key (another pairing, or a topic_secret) names other topics
        let c = topic_key(&seq32(8), &seq32(1), &seq32(200)).unwrap();
        assert_ne!(topic(&c, &nid(3), &nid(9)), t1);
    }

    #[test]
    fn topic_key_refuses_an_all_zero_key() {
        assert!(topic_key(&[0; 32], &seq32(1), &seq32(2)).is_none());
    }

    #[test]
    fn pair_topics_mirror() {
        let key = topic_key(&seq32(5), &seq32(6), &seq32(7)).unwrap();
        let a = Pair {
            own: nid(1),
            peer: nid(2),
            topic_key: key.clone(),
        };
        let b = Pair {
            own: nid(2),
            peer: nid(1),
            topic_key: key,
        };
        assert_eq!(outbound_topic(&a), inbound_topic(&b));
        assert_eq!(outbound_topic(&b), inbound_topic(&a));
    }

    #[test]
    fn base32_rfc4648_lower() {
        for (input, want) in [
            (&b""[..], ""),
            (b"f", "my"),
            (b"fo", "mzxq"),
            (b"foo", "mzxw6"),
            (b"foob", "mzxw6yq"),
            (b"fooba", "mzxw6ytb"),
            (b"foobar", "mzxw6ytboi"),
        ] {
            assert_eq!(base32_lower(input), want);
        }
        assert_eq!(base32_lower(&[0u8; 20]).len(), 32);
    }

    #[test]
    fn base64url_rfc4648_vectors_and_strictness() {
        for (input, want) in [
            (&b""[..], ""),
            (b"f", "Zg"),
            (b"fo", "Zm8"),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg"),
            (b"fooba", "Zm9vYmE"),
            (b"foobar", "Zm9vYmFy"),
            (&[0xfb, 0xff, 0xbf][..], "-_-_"),
        ] {
            assert_eq!(b64url_encode(input), want);
            assert_eq!(b64url_decode(want).as_deref(), Some(input));
        }
        // padding, the standard alphabet, a 1-mod-4 length, non-zero trailing bits, whitespace
        for bad in [
            "Zg==", "Zm9v+", "Zm9/", "Z", "Zm9vY", "Zh", "Zm9vYmF", "Zm 9v", "Zm9v\n",
        ] {
            assert!(b64url_decode(bad).is_none(), "{bad:?} must be refused");
        }
        // every length round-trips
        let data: Vec<u8> = (0..=255u8).collect();
        for n in 0..data.len() {
            let e = b64url_encode(&data[..n]);
            assert_eq!(e.len(), (n * 4).div_ceil(3));
            assert_eq!(b64url_decode(&e).unwrap(), &data[..n]);
        }
    }

    #[test]
    fn batch_round_trip_and_the_4096_boundary() {
        let a = vec![1u8; 100];
        let b = vec![2u8; 313];
        let body = encode_batch(&[&a, &b]).unwrap();
        assert_eq!(decode_batch(&body).unwrap(), vec![a, b]);
        // one max frame fills a batch to exactly 4,096 B
        let max = vec![9u8; MAX_FRAME];
        let body = encode_batch(&[&max]).unwrap();
        assert_eq!(body.len(), MAX_BODY);
        assert_eq!(decode_batch(&body).unwrap(), vec![max]);
        assert!(encode_batch(&[&vec![9u8; MAX_FRAME + 1]]).is_none());
        assert!(encode_batch(&[]).is_none());
        assert!(encode_batch(&[&[][..]]).is_none());
        assert_eq!(MAX_FRAME, 3068);
        assert_eq!(MAX_ITEM, 2937);
    }

    #[test]
    fn batch_decode_refuses_malformed_bodies() {
        let enc = |raw: &[u8]| b64url_encode(raw);
        assert_eq!(
            decode_batch(&"A".repeat(MAX_BODY + 1)),
            Err(BatchError::TooLong)
        );
        assert_eq!(decode_batch("not base64!"), Err(BatchError::NotBase64));
        assert_eq!(decode_batch(""), Err(BatchError::Empty));
        assert_eq!(
            decode_batch(&enc(&[2, 1, 0, 1, 7])),
            Err(BatchError::Version)
        );
        assert_eq!(decode_batch(&enc(&[1, 0])), Err(BatchError::Empty));
        assert_eq!(decode_batch(&enc(&[1])), Err(BatchError::Empty));
        assert_eq!(decode_batch(&enc(&[1, 1, 0])), Err(BatchError::Truncated));
        assert_eq!(
            decode_batch(&enc(&[1, 1, 0, 5, 1, 2])),
            Err(BatchError::Truncated)
        );
        assert_eq!(decode_batch(&enc(&[1, 1, 0, 0])), Err(BatchError::FrameLen));
        assert_eq!(
            decode_batch(&enc(&[1, 2, 0, 1, 7])),
            Err(BatchError::Truncated)
        );
        assert_eq!(
            decode_batch(&enc(&[1, 1, 0, 1, 7, 8])),
            Err(BatchError::Trailing)
        );
        assert_eq!(decode_batch(&enc(&[1, 1, 0, 1, 7])), Ok(vec![vec![7]]));
        // a length over max_frame (a hostile 0x0c 0xfd = 3,325)
        let mut raw = vec![1, 1, 0x0c, 0xfd];
        raw.extend(std::iter::repeat_n(0u8, 1000));
        assert_eq!(decode_batch(&enc(&raw)), Err(BatchError::FrameLen));
    }

    fn frames(lens: &[usize]) -> Vec<CarrierFrame> {
        lens.iter()
            .enumerate()
            .map(|(i, &n)| CarrierFrame {
                custody_seq: Some(i as u64),
                bytes: vec![i as u8; n],
            })
            .collect()
    }

    #[test]
    fn packing_is_greedy_in_custody_order() {
        // 1,500 + 1,500 + 4 + 4 + 2 = 3,010 ≤ 3,072; the third frame opens a second batch
        let f = frames(&[1500, 1500, 100]);
        let (slots, bodies) = plan_batches(&f);
        assert_eq!(slots, vec![Slot::Batch(0), Slot::Batch(0), Slot::Batch(1)]);
        assert_eq!(bodies.len(), 2);
        assert_eq!(decode_batch(&bodies[0]).unwrap().len(), 2);
        // never one message per fragment: 20 small frames go in one batch
        let (slots, bodies) = plan_batches(&frames(&[110; 20]));
        assert_eq!(bodies.len(), 1);
        assert!(slots.iter().all(|s| *s == Slot::Batch(0)));
        // n ≤ 255
        let (_, bodies) = plan_batches(&frames(&[1; 300]));
        assert_eq!(bodies.len(), 2);
        assert_eq!(decode_batch(&bodies[0]).unwrap().len(), 255);
        // an over-size or empty frame is refused and never posted; the rest still pack
        let (slots, bodies) = plan_batches(&frames(&[10, MAX_FRAME + 1, 0, 10]));
        assert_eq!(slots[0], Slot::Batch(0));
        assert!(matches!(slots[1], Slot::Refused(_)));
        assert!(matches!(slots[2], Slot::Refused(_)));
        assert_eq!(slots[3], Slot::Batch(0));
        assert_eq!(bodies.len(), 1);
        let (slots, bodies) = plan_batches(&frames(&[MAX_FRAME + 1]));
        assert!(matches!(slots[0], Slot::Refused(_)));
        assert!(bodies.is_empty());
        // every batch body stays ≤ 4,096 B
        let (_, bodies) = plan_batches(&frames(&[3000, 70, 3068, 1, 2000, 1070, 1068]));
        assert!(bodies.iter().all(|b| b.len() <= MAX_BODY));
    }

    #[test]
    fn status_mapping_follows_5_9() {
        assert_eq!(status_outcome(200), Outcome::Ok);
        for s in [413, 401, 403] {
            assert!(matches!(status_outcome(s), Outcome::Rejected(_)), "{s}");
        }
        for s in [429, 500, 502, 503, 507] {
            assert!(matches!(status_outcome(s), Outcome::Retry(_)), "{s}");
        }
        // statuses §5.9 does not name fail closed (permanent; the row is returned, never lost)
        for s in [301, 400, 404, 418] {
            assert!(matches!(status_outcome(s), Outcome::Rejected(_)), "{s}");
        }
        assert!(matches!(
            status_outcome(413).done(),
            ForwardDone::Rejected { .. }
        ));
        assert!(matches!(status_outcome(429).done(), ForwardDone::Retry));
        assert!(matches!(status_outcome(201).done(), ForwardDone::Taken));
        assert!(!status_outcome(413).stops_flush());
        assert!(status_outcome(403).stops_flush());
        assert!(status_outcome(429).stops_flush());
    }

    #[test]
    fn server_url_rule() {
        for ok in [
            "https://ntfy.example.org",
            "https://ntfy.example.org/",
            "https://ntfy.example.org:8443/prefix",
            "http://127.0.0.1:8080",
            "http://10.1.2.3",
            "http://192.168.1.4:80",
            "http://172.16.0.9",
            "http://169.254.1.1",
            "http://localhost:2586",
            "http://[::1]:2586",
            "http://[fd00::1]",
            "http://[fe80::1]",
        ] {
            assert!(check_server(ok).is_ok(), "{ok}");
        }
        assert_eq!(
            check_server("https://a.example/").unwrap(),
            "https://a.example"
        );
        for bad in [
            "",
            "ntfy.example.org",
            "ftp://ntfy.example.org",
            "http://ntfy.example.org",
            "http://8.8.8.8",
            "http://[2001:db8::1]",
            "http://172.32.0.1",
            "https://a.example/?x=1",
            "https://a.example/#x",
            "https://a.example/ x",
        ] {
            assert!(check_server(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn token_and_interval_checks() {
        let mut cfg = NtfyConfig::new("https://ntfy.example.org");
        cfg.token = Some(Zeroizing::new("tk_abc123".into()));
        assert!(NtfyCarrier::new(cfg).is_ok());
        let mut cfg = NtfyConfig::new("https://ntfy.example.org");
        cfg.token = Some(Zeroizing::new("tk with space".into()));
        assert!(NtfyCarrier::new(cfg).is_err());
        let mut cfg = NtfyConfig::new("https://ntfy.example.org");
        cfg.token = Some(Zeroizing::new("tk\r\nX-Injected: 1".into()));
        assert!(NtfyCarrier::new(cfg).is_err());
        let mut cfg = NtfyConfig::new("https://ntfy.example.org");
        cfg.keepalive = Duration::ZERO;
        assert!(NtfyCarrier::new(cfg).is_err());
        let c = NtfyCarrier::new(NtfyConfig::new("https://ntfy.example.org")).unwrap();
        assert_eq!(c.read_timeout(), Duration::from_secs(135));
        assert_eq!(c.id(), "ntfy");
        assert_eq!(c.max_frame(), 3068);
        assert!(!c.holding());
    }

    #[test]
    fn cursor_moves_only_on_commit() {
        let c = NtfyCarrier::new(NtfyConfig::new("https://ntfy.example.org")).unwrap();
        let key = topic_key(&seq32(5), &seq32(6), &seq32(7)).unwrap();
        let p = Pair {
            own: nid(1),
            peer: nid(2),
            topic_key: key,
        };
        assert_eq!(c.cursor(&p), None);
        let topic = inbound_topic(&p);
        assert_eq!(c.inner.since(&topic), Since::All);
        c.inner.set_seen(&topic, "AbC123xyz789".into());
        assert_eq!(c.cursor(&p), None, "a read alone does not move it");
        assert_eq!(c.commit_drain(&p).as_deref(), Some("AbC123xyz789"));
        assert_eq!(c.inner.since(&topic), Since::Id("AbC123xyz789".into()));
        assert_eq!(c.commit_drain(&p).as_deref(), Some("AbC123xyz789"));
        c.inner.since_missed(&topic);
        assert_eq!(c.cursor(&p), None);
        assert_eq!(c.stats().since_misses, 1);
        assert!(c.restore_cursor(&p, Some("bad id&x=1".into())).is_err());
        c.restore_cursor(&p, Some("Zz9".into())).unwrap();
        assert_eq!(c.cursor(&p).as_deref(), Some("Zz9"));
    }

    #[test]
    fn line_reader_caps_and_skips() {
        let data = b"one\r\ntwo\n\nthree".to_vec();
        let mut r = BufReader::with_capacity(4, &data[..]);
        let mut buf = Vec::new();
        assert_eq!(
            read_line_capped(&mut r, &mut buf, 10).unwrap(),
            Line::Complete
        );
        assert_eq!(buf, b"one");
        assert_eq!(
            read_line_capped(&mut r, &mut buf, 10).unwrap(),
            Line::Complete
        );
        assert_eq!(buf, b"two");
        assert_eq!(
            read_line_capped(&mut r, &mut buf, 10).unwrap(),
            Line::Complete
        );
        assert_eq!(buf, b"");
        assert_eq!(
            read_line_capped(&mut r, &mut buf, 10).unwrap(),
            Line::Complete
        );
        assert_eq!(buf, b"three");
        assert_eq!(read_line_capped(&mut r, &mut buf, 10).unwrap(), Line::Eof);

        let mut data = vec![b'x'; 40];
        data.extend_from_slice(b"\nok\n");
        let mut r = BufReader::with_capacity(8, &data[..]);
        assert_eq!(
            read_line_capped(&mut r, &mut buf, 10).unwrap(),
            Line::TooLong
        );
        assert_eq!(
            read_line_capped(&mut r, &mut buf, 10).unwrap(),
            Line::Complete
        );
        assert_eq!(buf, b"ok");

        let endless = vec![b'y'; MAX_DISCARD + 64 * 1024];
        let mut r = BufReader::new(&endless[..]);
        assert!(read_line_capped(&mut r, &mut buf, 10).is_err());
    }

    #[test]
    fn events_are_checked_field_by_field() {
        let t = "111abc";
        let batch = encode_batch(&[&[1, 2, 3][..]]).unwrap();
        let msg = |id: &str, topic: &str, m: &str| {
            serde_json::json!({"id": id, "time": 1, "event": "message", "topic": topic, "message": m})
                .to_string()
        };
        assert_eq!(
            parse_event(msg("AbC123xyz789", t, &batch).as_bytes(), t),
            Event::Message {
                id: "AbC123xyz789".into(),
                frames: Ok(vec![vec![1, 2, 3]])
            }
        );
        assert_eq!(
            parse_event(br#"{"id":"k1","event":"keepalive","topic":"111abc"}"#, t),
            Event::Keepalive
        );
        assert_eq!(
            parse_event(br#"{"id":"o1","event":"open","topic":"111abc"}"#, t),
            Event::Other
        );
        assert_eq!(parse_event(b"", t), Event::Other);
        assert_eq!(parse_event(b"not json", t), Event::Malformed);
        assert_eq!(
            parse_event(msg("AbC123xyz789", "111other", &batch).as_bytes(), t),
            Event::Malformed
        );
        assert_eq!(
            parse_event(msg("bad id&since=", t, &batch).as_bytes(), t),
            Event::Malformed
        );
        assert_eq!(
            parse_event(msg("", t, &batch).as_bytes(), t),
            Event::Malformed
        );
        assert!(matches!(
            parse_event(msg("AbC123xyz789", t, "hello, world").as_bytes(), t),
            Event::Message {
                frames: Err(BatchError::NotBase64),
                ..
            }
        ));
        assert!(matches!(
            parse_event(
                br#"{"id":"AbC123xyz789","event":"message","topic":"111abc"}"#,
                t
            ),
            Event::Message {
                frames: Err(BatchError::Empty),
                ..
            }
        ));
        assert_eq!(
            parse_event(br#"{"id":5,"event":"message"}"#, t),
            Event::Malformed
        );
    }

    #[test]
    fn the_read_cap_clamps_only_longer_waits() {
        use ureq::unversioned::transport::time::Duration as UDuration;
        #[derive(Debug)]
        struct Nothing;
        impl Transport for Nothing {
            fn buffers(&mut self) -> &mut dyn Buffers {
                unreachable!("not used")
            }
            fn transmit_output(&mut self, _: usize, _: NextTimeout) -> Result<(), ureq::Error> {
                Ok(())
            }
            fn await_input(&mut self, _: NextTimeout) -> Result<bool, ureq::Error> {
                Ok(false)
            }
            fn is_open(&mut self) -> bool {
                false
            }
        }
        let c = CappedTransport {
            inner: Box::new(Nothing),
            cap: Duration::from_secs(3),
        };
        let never = NextTimeout {
            after: UDuration::NotHappening,
            reason: ureq::Timeout::Global,
        };
        assert_eq!(*c.clamp(never).after, Duration::from_secs(3));
        let short = NextTimeout {
            after: UDuration::Exact(Duration::from_secs(1)),
            reason: ureq::Timeout::Connect,
        };
        assert_eq!(c.clamp(short), short);
    }
}
