//! §8 — links: the object-safe [`Link`] trait every bearer adapter
//! implements, the path facts it advertises ([`PathProfile`]), and the two
//! adapters this layer builds: [`tcp_lan`] (§8.1, the one real v1 link) and
//! [`lossy`] (a test-only in-process bearer with loss, reorder and an MTU,
//! for the §20 G1 Tier-1 scenario). Owner: F1.
//!
//! The trait is exactly §8's shape (futures boxed so it stays object-safe).
//! Three small seams sit **beside** it rather than in it, so the trait stays
//! the contract Layer 6 drivers implement:
//!
//! - [`LinkEvent`] — connection up/down facts an adapter reports to the
//!   fabric over an [`EventSink`] (§9 "members fetch on link-up"; §9 "retries
//!   happen on link and contact events, never on wall-clock timers");
//! - [`AnnounceGate`] — the §8.1 "a peer whose announce … conflicts with
//!   pinned keys is dropped" check, answered by the fabric from the contacts
//!   table (authoritative, L2 amendment A7);
//! - [`Dialer`] — keep an outbound connection to a known address up
//!   (reconnect with back-off is a transport detail, §8.1).
//!
//! The fabric never rate-limits: airtime and duty cycle are facts an adapter
//! advertises ([`Airtime`]), never limits this layer imposes (§1, §8).

use std::net::SocketAddr;

use futures::future::BoxFuture;
use id111::announce::Announce;
use id111::keyset::NodeId;

pub mod lossy;
pub mod tcp_lan;

/// §5: the default path MTU when the destination path is unknown (the
/// flood floor: MeshCore `GRP_DATA` 163 B ⇒ 160 B bearer payload).
pub const DEFAULT_MTU: u16 = 160;

/// §8 `Framing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Framing {
    /// A byte stream with this layer's own framing (`tcp-lan`).
    Stream,
    /// One frame per bearer datagram (LoRa, BLE).
    Datagram,
}

/// §8 `CostClass`: bulk classes route to `Bulk` links by default (§11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CostClass {
    /// Cheap, fast (`tcp-lan`).
    Bulk,
    /// Scarce airtime (LoRa).
    LowBandwidth,
}

/// §8: advertised physics — never a limit the fabric enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Airtime {
    /// Regulatory duty cycle in permille (10 = 1 %).
    pub duty_cycle_permille: u16,
    /// Raw bearer bit rate.
    pub bits_per_s: u32,
}

/// §8 `PathProfile`: what a link's path can carry. The fabric sizes
/// fragments from the destination path's profile (§5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PathProfile {
    /// Largest frame (bearer payload) in bytes, shim included.
    pub mtu_bytes: u16,
    /// Stream or datagram.
    pub framing: Framing,
    /// The bearer itself retransmits (TCP) — the fabric is reliable end to
    /// end either way (§6.1: only the recipient's ACK releases a copy).
    pub reliable: bool,
    /// A broadcast bearer carries the 1 B L6 shim in front of each frame
    /// (§3); `tcp-lan` never does.
    pub broadcast: bool,
    /// Bulk or low-bandwidth (§11 routing default).
    pub cost_class: CostClass,
    /// Advertised physics, if any.
    pub airtime: Option<Airtime>,
}

impl PathProfile {
    /// §5 "default profile when unknown: 160 B".
    pub fn unknown() -> Self {
        PathProfile {
            mtu_bytes: DEFAULT_MTU,
            framing: Framing::Datagram,
            reliable: false,
            broadcast: false,
            cost_class: CostClass::LowBandwidth,
            airtime: None,
        }
    }

    /// §8.1: `tcp-lan` — frames ≤ 65,535 B, reliable stream, no shim.
    pub fn tcp_lan() -> Self {
        PathProfile {
            mtu_bytes: u16::MAX,
            framing: Framing::Stream,
            reliable: true,
            broadcast: false,
            cost_class: CostClass::Bulk,
            airtime: None,
        }
    }

    /// Bytes of L6 shim in front of every frame on this path (§3).
    pub fn shim_len(&self) -> usize {
        usize::from(self.broadcast)
    }
}

/// §8 `PeerAddr`: where a frame goes on one link.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PeerAddr {
    /// A `tcp-lan` connection, keyed by its remote socket address.
    Tcp(SocketAddr),
    /// Every listener on a broadcast bearer.
    Broadcast,
    /// A bearer-specific address (test links, LoRa node numbers, …).
    Opaque(Vec<u8>),
}

/// What a frame carries (§3 shim `frame_type`; §8.1 `tcp-lan` types). The
/// `tcp-lan` keepalive is a transport detail and never surfaces here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameType {
    /// One §3 sealed packet (`tcp-lan` 0x01; shim type 1).
    Sealed,
    /// A cleartext L2 announce (`tcp-lan` 0x02; shim public sub-type 0).
    Announce,
    /// A cleartext card request: the wanted NodeId, 32 B (`tcp-lan` 0x04;
    /// shim public sub-type 1; §7.2 (2)).
    CardRequest,
    /// An unsealed Tier-2 chunk (`tcp-lan` 0x05; reserved, built v1.1).
    Chunk,
    /// A foreign network's frame (shim type 3; bridges, later).
    Foreign,
}

/// §8 `Frame`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Frame type.
    pub frame_type: FrameType,
    /// L6 shim hop limit on broadcast bearers; `None` elsewhere.
    pub hop_limit: Option<u8>,
    /// The frame body (a sealed packet, an announce, a NodeId, …).
    pub bytes: Vec<u8>,
}

impl Frame {
    /// A sealed packet frame.
    pub fn sealed(bytes: Vec<u8>) -> Frame {
        Frame {
            frame_type: FrameType::Sealed,
            hop_limit: None,
            bytes,
        }
    }

    /// An announce frame (stored announce bytes, never minted here, §7).
    pub fn announce(bytes: Vec<u8>) -> Frame {
        Frame {
            frame_type: FrameType::Announce,
            hop_limit: None,
            bytes,
        }
    }

    /// A card request for `wanted` (§7.2 (2)).
    pub fn card_request(wanted: &NodeId) -> Frame {
        Frame {
            frame_type: FrameType::CardRequest,
            hop_limit: None,
            bytes: wanted.to_bytes().to_vec(),
        }
    }
}

/// §8 `LinkState` (feeds `transport_state` / `DeviceState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    /// Not usable.
    Down,
    /// Usable; `peers` connected.
    Up {
        /// Connected peers.
        peers: usize,
    },
    /// Usable with a problem, one plain sentence.
    Degraded(String),
}

/// §8 `LinkError`.
#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    /// The link or the addressed peer is gone.
    #[error("link closed")]
    Closed,
    /// The frame exceeds what the path carries.
    #[error("frame of {0} B exceeds the link's cap")]
    TooLarge(usize),
    /// Transport failure.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// A link's stable identifier (shown in `GET /fabric`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LinkId(pub String);

impl LinkId {
    /// A link id from a name.
    pub fn new(name: impl Into<String>) -> Self {
        LinkId(name.into())
    }

    /// The id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// §8 adapter kinds (`TcpLan | KissModem | MeshtasticClient | RnsBridge | Ble`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkKind {
    /// §8.1, built with L5.
    TcpLan,
    /// MeshCore KISS modem (seam; Layer 6).
    KissModem,
    /// Meshtastic via meshmud (seam; no bridge this cycle).
    MeshtasticClient,
    /// LXMF carriage (seam; deferred).
    RnsBridge,
    /// Android helper (later).
    Ble,
}

impl LinkKind {
    /// The contract's `LinkKind` string (`openapi.yaml`).
    pub fn as_str(self) -> &'static str {
        match self {
            LinkKind::TcpLan => "tcp-lan",
            LinkKind::KissModem => "kiss-modem",
            LinkKind::MeshtasticClient => "meshtastic-client",
            LinkKind::RnsBridge => "rns-bridge",
            LinkKind::Ble => "ble",
        }
    }
}

/// §8: the bearer contract. Object-safe: futures are boxed.
pub trait Link: Send + Sync {
    /// Stable id.
    fn id(&self) -> LinkId;
    /// Adapter kind.
    fn kind(&self) -> LinkKind;
    /// What the path carries.
    fn profile(&self) -> PathProfile;
    /// Currently reachable peers.
    fn peers(&self) -> Vec<PeerAddr>;
    /// Write one frame to `to`. `Ok` means the frame was handed to the
    /// bearer (§9 `sent`: "frame written to a link"). Must not wait on this
    /// link's `recv` being drained: the fabric awaits `send` from the same
    /// task that takes inbound frames (back-pressure, `fabric` module doc).
    fn send(&self, to: PeerAddr, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>>;
    /// The next inbound frame (pends while none). An adapter holds inbound
    /// frames in a bounded queue and stops reading its bearer while it is
    /// full, so a slow fabric pushes back on the sender instead of growing
    /// memory.
    fn recv(&self) -> BoxFuture<'_, (PeerAddr, Frame)>;
    /// Current state.
    fn state(&self) -> LinkState;
}

/// Connection facts an adapter reports beside the trait (module doc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEvent {
    /// A connection passed the adapter's handshake.
    Connected {
        /// The link.
        link: LinkId,
        /// The connection.
        peer: PeerAddr,
        /// This side opened it (a dial to a configured address).
        outbound: bool,
        /// For a dial: the NodeId the dialer expects at that address. The
        /// peer's announce is checked against it (a mismatch is hung up).
        expected: Option<NodeId>,
    },
    /// The connection is gone.
    Disconnected {
        /// The link.
        link: LinkId,
        /// The connection.
        peer: PeerAddr,
    },
}

/// Where adapters report [`LinkEvent`]s (the fabric owns the receiver).
pub type EventSink = tokio::sync::mpsc::UnboundedSender<LinkEvent>;

/// The §8.1 pinned-keys check on a signature-verified announce: `false` ⇒
/// the adapter drops the connection. Answered from the contacts table (the
/// authority, A7) — never from the L2 pin cache.
pub trait AnnounceGate: Send + Sync {
    /// Admit `announce` (its signature already verified).
    fn admit(&self, announce: &Announce) -> bool;
}

/// A gate that admits every verified announce (tests, tools).
#[derive(Debug, Default, Clone, Copy)]
pub struct AdmitAll;

impl AnnounceGate for AdmitAll {
    fn admit(&self, _announce: &Announce) -> bool {
        true
    }
}

/// Outbound connection management beside the trait (module doc).
pub trait Dialer: Send + Sync {
    /// Keep a connection to `addr` up: dial now, redial with back-off when it
    /// drops. `expected` is the NodeId that should answer there.
    fn maintain(&self, addr: SocketAddr, expected: Option<NodeId>);
    /// Stop maintaining `addr` (an open connection is closed).
    fn release(&self, addr: SocketAddr);
    /// Close one connection (a key conflict or a misdial).
    fn hang_up(&self, peer: &PeerAddr);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_match_the_spec_table() {
        let t = PathProfile::tcp_lan();
        assert_eq!(t.mtu_bytes, 65_535);
        assert_eq!(t.shim_len(), 0);
        assert!(t.reliable);
        let u = PathProfile::unknown();
        assert_eq!(u.mtu_bytes, 160);
        assert_eq!(u.framing, Framing::Datagram);
        assert_eq!(LinkKind::TcpLan.as_str(), "tcp-lan");
        assert_eq!(LinkKind::MeshtasticClient.as_str(), "meshtastic-client");
    }

    #[test]
    fn frames() {
        let id = NodeId::from([7u8; 32]);
        let f = Frame::card_request(&id);
        assert_eq!(f.frame_type, FrameType::CardRequest);
        assert_eq!(f.bytes, vec![7u8; 32]);
        assert_eq!(Frame::sealed(vec![1]).frame_type, FrameType::Sealed);
    }
}
