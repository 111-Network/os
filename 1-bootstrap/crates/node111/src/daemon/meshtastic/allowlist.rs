//! The Meshtastic **send allowlist at the lowest layer** (Layer 6 `6-transport/README.md`
//! §5 "Send allowlist at the lowest layer"; precedent: the presence reader's
//! `ALLOWED_TORADIO`, `meshtastic_presence.py:93`).
//!
//! The client ([`super::client`]) never writes to meshmud's downlink directly: its write
//! half is wrapped in an [`AllowlistWriter`], an [`AsyncWrite`] that deframes every outgoing
//! `0x94 0xC3 <len hi> <len lo>` frame, parses the `ToRadio` with prost and forwards **only**:
//!
//! - `packet` carrying plain text: `decoded.portnum = TEXT_MESSAGE_APP`, `to` = the channel
//!   broadcast `0xFFFFFFFF` or a node, and nothing else set but the channel index, a nonzero
//!   packet id, `want_ack` and a hop limit ≤ 7 (the exact shape [`super::client`] builds);
//! - `want_config_id` **once** per connection (a writer is one connection), never 0, 1,
//!   69420 or 69421;
//! - `heartbeat` with nonce 0 (nonce 1 makes the board broadcast its NodeInfo: a transmit).
//!
//! Everything else is **dropped, counted and logged without content**: `disconnect` (it
//! would close the board's API session for every client), `xmodemPacket`,
//! `mqttClientProxyMessage`, a heartbeat with any other nonce, a second `want_config_id`
//! (re-sending it inside a session can wedge firmware, R01 §3.10), any other portnum (admin,
//! config, position, …), an encrypted or odd-shaped packet, a frame that does not decode,
//! one whose encoding is not the canonical prost encoding (unknown fields, repeated oneofs),
//! and bytes that are not a frame at all. The second release adds `PRIVATE_APP` (§17).
//!
//! Refused variants are labelled from the frame's first tag byte, so this file names no
//! `ToRadio` variant beyond the three allowed ones (the source-scan test in
//! `tests/meshtastic_client.rs` holds every file under `src/` to that).

use super::shim::{MAX_FRAME, START1, START2};
use meshtastic::protobufs as pb;
use meshtastic::Message;
use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::io::AsyncWrite;

/// The channel broadcast address (`to = 0xFFFFFFFF`).
pub const BROADCAST: u32 = 0xFFFF_FFFF;
/// `want_config_id` values a client must never use (§5): 1, and the web client's special
/// nonces 69420 (config only) and 69421 (nodes only). 0 is "no id" and refused too.
pub const RESERVED_CONFIG_IDS: [u32; 3] = [1, 69_420, 69_421];
/// `Data.payload` limit (`mesh.options`: `max_size:233`).
pub const MAX_DATA_PAYLOAD: usize = 233;
/// Highest hop limit the firmware accepts (R01 §3.2).
pub const MAX_HOP_LIMIT: u32 = 7;
/// Highest channel index (0–7, R01 §3.5).
pub const MAX_CHANNEL_INDEX: u32 = 7;
/// Bytes absorbed per `poll_write` (one frame is at most 516).
const ABSORB_MAX: usize = 4096;
/// Recent log lines kept for [`LogRing::recent`].
pub const LOG_LINES: usize = 64;

/// What passed the allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Allowed {
    /// A plain text packet.
    Text,
    /// The connection's one `want_config_id`.
    WantConfig(u32),
    /// The nonce-0 session keep-alive.
    Heartbeat,
}

/// Why a frame (or stray bytes) was dropped. The labels are counted and logged; no content
/// ever is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Refused {
    /// Bytes outside a frame, a declared length over 512, or a partial frame at shutdown.
    NotFramed,
    /// A zero-length frame or a `ToRadio` with no variant.
    Empty,
    /// Not a decodable `ToRadio`.
    Undecodable,
    /// Decodes, but is not the canonical encoding (unknown fields, repeated oneofs, padding).
    NonCanonical,
    /// A packet that is not plain text in the client's shape (any other portnum, an
    /// encrypted payload, a reply, a tapback, a `from`, PKI fields, a hop limit over 7, …).
    Packet,
    /// A second `want_config_id` on this connection.
    WantConfigRepeat,
    /// `want_config_id` 0, 1, 69420 or 69421.
    WantConfigReserved,
    /// `ToRadio.disconnect` (tag byte `0x20`).
    Disconnect,
    /// `ToRadio.xmodemPacket` (tag byte `0x2a`).
    Xmodem,
    /// `ToRadio.mqttClientProxyMessage` (tag byte `0x32`).
    MqttProxy,
    /// A heartbeat whose nonce is not 0.
    HeartbeatNonce,
    /// Any other variant.
    Other,
}

impl Refused {
    pub fn as_str(self) -> &'static str {
        match self {
            Refused::NotFramed => "not_framed",
            Refused::Empty => "empty",
            Refused::Undecodable => "undecodable",
            Refused::NonCanonical => "non_canonical",
            Refused::Packet => "packet_not_text",
            Refused::WantConfigRepeat => "want_config_repeat",
            Refused::WantConfigReserved => "want_config_reserved",
            Refused::Disconnect => "disconnect",
            Refused::Xmodem => "xmodem",
            Refused::MqttProxy => "mqtt_client_proxy",
            Refused::HeartbeatNonce => "heartbeat_nonce",
            Refused::Other => "other",
        }
    }

    /// Label a refused `ToRadio` from its first tag byte (field number << 3 | wire type).
    fn from_tag(first: u8) -> Refused {
        match first {
            0x20 => Refused::Disconnect,
            0x2a => Refused::Xmodem,
            0x32 => Refused::MqttProxy,
            _ => Refused::Other,
        }
    }
}

/// A config id the client may use (§5: random, nonzero, never 1, 69420 or 69421).
pub fn config_id_ok(id: u32) -> bool {
    id != 0 && !RESERVED_CONFIG_IDS.contains(&id)
}

/// The one packet shape that passes: plain text with only the client's fields set.
fn text_packet_ok(p: &pb::MeshPacket) -> bool {
    let Some(pb::mesh_packet::PayloadVariant::Decoded(d)) = &p.payload_variant else {
        return false;
    };
    if d.portnum != pb::PortNum::TextMessageApp as i32
        || d.payload.len() > MAX_DATA_PAYLOAD
        || p.to == 0
        || p.id == 0
        || p.channel > MAX_CHANNEL_INDEX
        || p.hop_limit > MAX_HOP_LIMIT
    {
        return false;
    }
    // every other field at its default: equality with the template proves it
    let template = pb::MeshPacket {
        to: p.to,
        channel: p.channel,
        id: p.id,
        hop_limit: p.hop_limit,
        // §5 fixes `want_ack = true` for channel text and DMs (the local reliable record);
        // a text without it is not ours (B3 review F4)
        want_ack: true,
        payload_variant: Some(pb::mesh_packet::PayloadVariant::Decoded(pb::Data {
            portnum: pb::PortNum::TextMessageApp as i32,
            payload: d.payload.clone(),
            ..Default::default()
        })),
        ..Default::default()
    };
    *p == template
}

/// Classify one frame payload. `want_config_seen` = this connection already let its one
/// `want_config_id` through.
pub fn classify(payload: &[u8], want_config_seen: bool) -> Result<Allowed, Refused> {
    let Some(&first) = payload.first() else {
        return Err(Refused::Empty);
    };
    let msg = pb::ToRadio::decode(payload).map_err(|_| Refused::Undecodable)?;
    let allowed = match &msg.payload_variant {
        None => return Err(Refused::Empty),
        Some(pb::to_radio::PayloadVariant::Packet(p)) => {
            if !text_packet_ok(p) {
                return Err(Refused::Packet);
            }
            Allowed::Text
        }
        Some(pb::to_radio::PayloadVariant::WantConfigId(id)) => {
            if !config_id_ok(*id) {
                return Err(Refused::WantConfigReserved);
            }
            if want_config_seen {
                return Err(Refused::WantConfigRepeat);
            }
            Allowed::WantConfig(*id)
        }
        Some(pb::to_radio::PayloadVariant::Heartbeat(h)) => {
            if h.nonce != 0 {
                return Err(Refused::HeartbeatNonce);
            }
            Allowed::Heartbeat
        }
        Some(_) => return Err(Refused::from_tag(first)),
    };
    if msg.encoded_len() != payload.len() {
        return Err(Refused::NonCanonical);
    }
    Ok(allowed)
}

/// Bounded recent log lines (no content ever reaches one) that also go to stderr: the
/// sentinel tests read [`LogRing::recent`] to prove nothing secret is logged.
#[derive(Default)]
pub struct LogRing {
    lines: Mutex<VecDeque<String>>,
}

impl LogRing {
    pub fn new() -> Self {
        Self::default()
    }

    /// Log one line (stderr, `111 meshtastic client: …`) and keep it.
    pub fn line(&self, s: impl Into<String>) {
        let s = s.into();
        eprintln!("111 meshtastic client: {s}");
        let mut l = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        if l.len() >= LOG_LINES {
            l.pop_front();
        }
        l.push_back(s);
    }

    /// The last [`LOG_LINES`] lines, oldest first.
    pub fn recent(&self) -> Vec<String> {
        self.lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}

impl std::fmt::Debug for LogRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogRing")
            .field("lines", &self.recent().len())
            .finish()
    }
}

/// Counters shared by every connection's writer (`/networks`, the service page, tests).
#[derive(Debug, Default)]
pub struct AllowStats {
    passed: AtomicU64,
    dropped: AtomicU64,
    by_reason: Mutex<BTreeMap<Refused, u64>>,
    log: Option<Arc<LogRing>>,
}

impl AllowStats {
    pub fn new(log: Option<Arc<LogRing>>) -> Self {
        AllowStats {
            log,
            ..Default::default()
        }
    }

    fn pass(&self) {
        self.passed.fetch_add(1, Ordering::Relaxed);
    }

    fn drop_one(&self, why: Refused, len: usize) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
        *self
            .by_reason
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(why)
            .or_insert(0) += 1;
        if let Some(log) = &self.log {
            log.line(format!(
                "allowlist dropped a ToRadio ({}, {len} bytes); nothing was sent",
                why.as_str()
            ));
        }
    }

    /// Frames forwarded to meshmud.
    pub fn passed(&self) -> u64 {
        self.passed.load(Ordering::Relaxed)
    }

    /// Frames (or stray byte runs) dropped.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Drops by reason.
    pub fn by_reason(&self) -> BTreeMap<Refused, u64> {
        self.by_reason
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// The allowlisting write half of one downlink connection (see the module docs).
///
/// Bytes written are absorbed into a parse buffer that never holds more than one partial
/// frame plus one [`ABSORB_MAX`] chunk; complete frames are classified and only allowed
/// frames reach `inner`. Back-pressure: a write first drains what is already allowed.
pub struct AllowlistWriter<W> {
    inner: W,
    stats: Arc<AllowStats>,
    parse: Vec<u8>,
    out: Vec<u8>,
    out_pos: usize,
    want_config_seen: bool,
}

impl<W> std::fmt::Debug for AllowlistWriter<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AllowlistWriter")
            .field("pending", &self.parse.len())
            .field("queued", &(self.out.len() - self.out_pos))
            .field("want_config_seen", &self.want_config_seen)
            .finish()
    }
}

impl<W: AsyncWrite + Unpin> AllowlistWriter<W> {
    pub fn new(inner: W, stats: Arc<AllowStats>) -> Self {
        AllowlistWriter {
            inner,
            stats,
            parse: Vec::new(),
            out: Vec::new(),
            out_pos: 0,
            want_config_seen: false,
        }
    }

    /// This connection's `want_config_id` already went out.
    pub fn want_config_seen(&self) -> bool {
        self.want_config_seen
    }

    pub fn stats(&self) -> &Arc<AllowStats> {
        &self.stats
    }

    /// Move every complete frame out of the parse buffer: allowed ones to `out`, the rest
    /// dropped and counted.
    fn process(&mut self) {
        loop {
            match self.parse.windows(2).position(|w| w == [START1, START2]) {
                Some(0) => {}
                Some(i) => {
                    self.parse.drain(..i);
                    self.stats.drop_one(Refused::NotFramed, i);
                }
                None => {
                    // keep a trailing START1: its START2 may come with the next write
                    let keep = usize::from(self.parse.last() == Some(&START1));
                    let cut = self.parse.len() - keep;
                    if cut > 0 {
                        self.parse.drain(..cut);
                        self.stats.drop_one(Refused::NotFramed, cut);
                    }
                    return;
                }
            }
            if self.parse.len() < 4 {
                return;
            }
            let len = usize::from(u16::from_be_bytes([self.parse[2], self.parse[3]]));
            if len > MAX_FRAME {
                self.parse.drain(..2);
                self.stats.drop_one(Refused::NotFramed, 2);
                continue;
            }
            if self.parse.len() < 4 + len {
                return;
            }
            let frame: Vec<u8> = self.parse.drain(..4 + len).collect();
            match classify(&frame[4..], self.want_config_seen) {
                Ok(kind) => {
                    if matches!(kind, Allowed::WantConfig(_)) {
                        self.want_config_seen = true;
                    }
                    self.stats.pass();
                    self.out.extend_from_slice(&frame);
                }
                Err(why) => self.stats.drop_one(why, len),
            }
        }
    }

    /// Write what is allowed and queued; `Pending` while `inner` is.
    fn poll_drain(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.out_pos < self.out.len() {
            match Pin::new(&mut self.inner).poll_write(cx, &self.out[self.out_pos..]) {
                Poll::Ready(Ok(0)) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "meshmud downlink took no bytes",
                    )))
                }
                Poll::Ready(Ok(n)) => self.out_pos += n,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
        self.out.clear();
        self.out_pos = 0;
        Poll::Ready(Ok(()))
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for AllowlistWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let me = &mut *self;
        match me.poll_drain(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Pending => return Poll::Pending,
        }
        let n = buf.len().min(ABSORB_MAX);
        me.parse.extend_from_slice(&buf[..n]);
        me.process();
        // the bytes are ours now; a pending inner write finishes on the next poll or flush
        if let Poll::Ready(Err(e)) = me.poll_drain(cx) {
            return Poll::Ready(Err(e));
        }
        Poll::Ready(Ok(n))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let me = &mut *self;
        match me.poll_drain(cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut me.inner).poll_flush(cx),
            other => other,
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let me = &mut *self;
        if !me.parse.is_empty() {
            let n = me.parse.len();
            me.parse.clear();
            me.stats.drop_one(Refused::NotFramed, n);
        }
        match me.poll_drain(cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut me.inner).poll_shutdown(cx),
            other => other,
        }
    }
}

/// Write all of `bytes` through `w` and flush (tokio's `io-util` is not enabled in this
/// crate, so this is the poll form of `write_all`).
pub async fn write_all_flush<W: AsyncWrite + Unpin>(w: &mut W, bytes: &[u8]) -> io::Result<()> {
    let mut done = 0;
    while done < bytes.len() {
        let n = std::future::poll_fn(|cx| Pin::new(&mut *w).poll_write(cx, &bytes[done..])).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "write took no bytes",
            ));
        }
        done += n;
    }
    std::future::poll_fn(|cx| Pin::new(&mut *w).poll_flush(cx)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::meshtastic::shim::frame;

    fn enc(m: &pb::ToRadio) -> Vec<u8> {
        m.encode_to_vec()
    }

    fn text(to: u32, payload: &[u8]) -> pb::MeshPacket {
        pb::MeshPacket {
            to,
            channel: 0,
            id: 0x1234_5678,
            hop_limit: 3,
            want_ack: true,
            payload_variant: Some(pb::mesh_packet::PayloadVariant::Decoded(pb::Data {
                portnum: pb::PortNum::TextMessageApp as i32,
                payload: payload.to_vec(),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn packet(p: pb::MeshPacket) -> Vec<u8> {
        enc(&pb::ToRadio {
            payload_variant: Some(pb::to_radio::PayloadVariant::Packet(p)),
        })
    }

    fn with_data(f: impl FnOnce(&mut pb::Data)) -> Vec<u8> {
        let mut p = text(BROADCAST, b"hi");
        if let Some(pb::mesh_packet::PayloadVariant::Decoded(d)) = &mut p.payload_variant {
            f(d);
        }
        packet(p)
    }

    /// The fixed list (§5): exactly these pass, everything else is refused with its label.
    #[test]
    fn the_list_is_fixed() {
        // allowed
        assert_eq!(
            classify(&packet(text(BROADCAST, b"hello")), false),
            Ok(Allowed::Text)
        );
        assert_eq!(
            classify(&packet(text(0x0a0b_0c0d, b"dm")), true),
            Ok(Allowed::Text),
            "a node destination; want_config state does not matter"
        );
        let want = enc(&pb::ToRadio {
            payload_variant: Some(pb::to_radio::PayloadVariant::WantConfigId(0x5eed_1234)),
        });
        assert_eq!(classify(&want, false), Ok(Allowed::WantConfig(0x5eed_1234)));
        let hb = enc(&pb::ToRadio {
            payload_variant: Some(pb::to_radio::PayloadVariant::Heartbeat(pb::Heartbeat {
                nonce: 0,
            })),
        });
        assert_eq!(hb, [0x3a, 0x00], "the nonce-0 keep-alive's exact bytes");
        assert_eq!(classify(&hb, true), Ok(Allowed::Heartbeat));

        // want_config_id: once, never reserved
        assert_eq!(classify(&want, true), Err(Refused::WantConfigRepeat));
        for id in [0u32, 1, 69_420, 69_421] {
            let m = enc(&pb::ToRadio {
                payload_variant: Some(pb::to_radio::PayloadVariant::WantConfigId(id)),
            });
            // a set oneof is always on the wire, 0 included
            assert_eq!(
                classify(&m, false),
                Err(Refused::WantConfigReserved),
                "id {id}"
            );
        }
        // heartbeat nonce 1 (would broadcast our NodeInfo) and any other nonce
        for nonce in [1u32, 2, 0xffff_ffff] {
            let m = enc(&pb::ToRadio {
                payload_variant: Some(pb::to_radio::PayloadVariant::Heartbeat(pb::Heartbeat {
                    nonce,
                })),
            });
            assert_eq!(classify(&m, false), Err(Refused::HeartbeatNonce));
        }
        // the variants we never send, by their tag byte (built from raw bytes so no
        // constructor of them exists in this crate)
        assert_eq!(classify(&[0x20, 0x01], false), Err(Refused::Disconnect));
        assert_eq!(classify(&[0x2a, 0x00], false), Err(Refused::Xmodem));
        assert_eq!(classify(&[0x32, 0x00], false), Err(Refused::MqttProxy));
        assert_eq!(classify(&[], false), Err(Refused::Empty));
        assert_eq!(
            classify(&[0xff, 0xff, 0xff], false),
            Err(Refused::Undecodable)
        );
        // a future field (number 9, varint) only: decodes to no variant
        assert_eq!(classify(&[0x48, 0x01], false), Err(Refused::Empty));
        // unknown field after an allowed variant: not canonical
        let mut hb_extra = hb.clone();
        hb_extra.extend_from_slice(&[0x48, 0x01]);
        assert_eq!(classify(&hb_extra, false), Err(Refused::NonCanonical));
        // heartbeat followed by disconnect: prost keeps the last oneof → refused as such
        assert_eq!(
            classify(&[0x3a, 0x00, 0x20, 0x01], false),
            Err(Refused::Other),
            "labelled by the first tag byte, refused either way"
        );
    }

    #[test]
    fn only_plain_text_packets_pass() {
        // every other portnum
        for port in [
            pb::PortNum::UnknownApp,
            pb::PortNum::PositionApp,
            pb::PortNum::NodeinfoApp,
            pb::PortNum::RoutingApp,
            pb::PortNum::AdminApp,
            pb::PortNum::TelemetryApp,
            pb::PortNum::PrivateApp,
            pb::PortNum::TracerouteApp,
            pb::PortNum::WaypointApp,
        ] {
            assert_eq!(
                classify(&with_data(|d| d.portnum = port as i32), false),
                Err(Refused::Packet),
                "{port:?}"
            );
        }
        // reply, tapback, want_response, request id, bitfield, dest/source
        for f in [
            (|d: &mut pb::Data| d.reply_id = 7) as fn(&mut pb::Data),
            |d| d.emoji = 1,
            |d| d.want_response = true,
            |d| d.request_id = 9,
            |d| d.bitfield = Some(1),
            |d| d.dest = 3,
            |d| d.source = 4,
            |d| d.payload = vec![b'x'; MAX_DATA_PAYLOAD + 1],
        ] {
            assert_eq!(classify(&with_data(f), false), Err(Refused::Packet));
        }
        // a 233-byte payload is the most that passes
        assert_eq!(
            classify(
                &with_data(|d| d.payload = vec![b'x'; MAX_DATA_PAYLOAD]),
                false
            ),
            Ok(Allowed::Text)
        );
        // packet-level fields we never set
        for f in [
            (|p: &mut pb::MeshPacket| p.to = 0) as fn(&mut pb::MeshPacket),
            |p| p.id = 0,
            |p| p.from = 0x0101_0101,
            |p| p.channel = 8,
            |p| p.hop_limit = 8,
            |p| p.pki_encrypted = true,
            |p| p.public_key = vec![1; 32],
            |p| p.via_mqtt = true,
            |p| p.priority = 70,
            |p| p.hop_start = 3,
            |p| p.rx_time = 1,
            |p| p.next_hop = 1,
            |p| p.relay_node = 1,
            // F4: §5 pins want_ack = true; a text without it never passes
            |p| p.want_ack = false,
            |p| p.payload_variant = Some(pb::mesh_packet::PayloadVariant::Encrypted(vec![1, 2])),
            |p| p.payload_variant = None,
        ] {
            let mut p = text(BROADCAST, b"hi");
            f(&mut p);
            assert_eq!(classify(&packet(p), false), Err(Refused::Packet));
        }
    }

    struct Sink(Vec<u8>);

    impl AsyncWrite for Sink {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            b: &[u8],
        ) -> Poll<io::Result<usize>> {
            // one byte at a time: partial writes must be handled
            self.0.push(b[0]);
            Poll::Ready(Ok(1))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn the_writer_forwards_only_allowed_frames() {
        let log = Arc::new(LogRing::new());
        let stats = Arc::new(AllowStats::new(Some(log.clone())));
        let mut w = AllowlistWriter::new(Sink(Vec::new()), stats.clone());
        let want = frame(&enc(&pb::ToRadio {
            payload_variant: Some(pb::to_radio::PayloadVariant::WantConfigId(42)),
        }))
        .unwrap();
        let hb = frame(&[0x3a, 0x00]).unwrap();
        let txt = frame(&packet(text(BROADCAST, b"hello"))).unwrap();
        let disc = frame(&[0x20, 0x01]).unwrap();
        let mut stream = b"garbage".to_vec();
        stream.extend_from_slice(&want);
        stream.extend_from_slice(&want); // a second want_config_id: dropped
        stream.extend_from_slice(&disc);
        stream.extend_from_slice(&hb);
        stream.extend_from_slice(&txt);
        // written in odd chunk sizes
        for chunk in stream.chunks(5) {
            write_all_flush(&mut w, chunk).await.unwrap();
        }
        let mut expect = want.clone();
        expect.extend_from_slice(&hb);
        expect.extend_from_slice(&txt);
        assert_eq!(
            w.inner.0, expect,
            "the second want_config does not wedge it"
        );
        assert!(w.want_config_seen());
        assert_eq!(stats.passed(), 3);
        let by = stats.by_reason();
        // the garbage arrives in two writes: two stray runs
        assert_eq!(by[&Refused::NotFramed], 2);
        assert_eq!(by[&Refused::WantConfigRepeat], 1);
        assert_eq!(by[&Refused::Disconnect], 1);
        assert_eq!(stats.dropped(), 4);
        let lines = log.recent();
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().all(|l| l.contains("nothing was sent")));
        assert!(!lines.iter().any(|l| l.contains("hello")), "no content");
        // a partial frame at shutdown is dropped, never forwarded
        let mut w2 = AllowlistWriter::new(Sink(Vec::new()), stats.clone());
        write_all_flush(&mut w2, &txt[..txt.len() - 1])
            .await
            .unwrap();
        std::future::poll_fn(|cx| Pin::new(&mut w2).poll_shutdown(cx))
            .await
            .unwrap();
        assert!(w2.inner.0.is_empty());
    }

    #[test]
    fn config_ids() {
        assert!(!config_id_ok(0));
        assert!(!config_id_ok(1));
        assert!(!config_id_ok(69_420));
        assert!(!config_id_ok(69_421));
        assert!(config_id_ok(2));
        assert!(config_id_ok(u32::MAX));
    }
}
