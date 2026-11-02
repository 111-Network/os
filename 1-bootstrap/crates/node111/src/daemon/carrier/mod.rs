//! Layer 7 (§5.9, §6.1): T0 stub; filled by row D11 (the carrier on the forwarder). Row P1
//! implements [`Carrier`] for ntfy in `ntfy.rs` behind a fake [`HandOff`].
//!
//! The carrier contract (§6.1): a carrier moves opaque sealed hub-to-hub frames (the same
//! `CUSTODY_PUT` and `CUSTODY_ACK` packets a pair exchanges on `tcp-lan`) between two paired hubs,
//! batched in custody order, never one message per fragment; it declares `max_frame`; it is not a
//! `Link` in v1; its accept is a "posted" mark and only the far hub's `CUSTODY_ACK` commits; before
//! every flush it consults the consent record, the metered budget and the licensed-board rule. In
//! v1 it runs in-process behind the hand-off API (§5.9; §19 D12).
//!
//! The fabric side of the hand-off (amendment 11) is mesh111's, built by row M4:
//! `Fabric::forward_take`, `Fabric::forward_done`, `Fabric::carrier_inbound`,
//! `mesh111::CarrierFrame`, `mesh111::ForwardDone` and `FabricEvent::Forward { target, pending }`.
//! Until M4 lands, this module mirrors their §5.9 shapes ([`HandOff`], [`CarrierFrame`],
//! [`ForwardDone`], [`ForwardPending`]) so P1 builds against them; D11 adapts the mesh111 types to
//! them (or replaces the mirrors with re-exports) when it wires the carrier to the real fabric.
//!
//! Nothing here is constructed while `carrier.kind = "none"` or `gateway.enabled = false` (§6.3):
//! no socket, task, thread, file write or netlink subscription.
#![allow(dead_code)]

pub mod ntfy;

use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use id111::keyset::NodeId;
use zeroize::Zeroizing;

/// One sealed frame owed to a `carrier` pair (§5.9; mirrors `mesh111::CarrierFrame`): a forward put
/// (`custody_seq = Some`) or a `CUSTODY_ACK` owed to it (`None`, consumed on take, C7).
pub struct CarrierFrame {
    pub custody_seq: Option<u64>,
    pub bytes: Vec<u8>,
}

/// What the carrier did with a put it took (§5.9; mirrors `mesh111::ForwardDone`).
pub enum ForwardDone {
    /// The carrier's mailbox accepted the message: an in-memory "posted" mark; the row stays
    /// `committed = 0` until the far hub's `CUSTODY_ACK` arrives through `carrier_inbound` (§5.6;
    /// §19 C5).
    Taken,
    /// Permanent (HTTP 413, 401, 403 for ntfy): the row returns to the other targets; with none
    /// left it is refused and counted (§5.8).
    Rejected { why: &'static str },
    /// Transient (no route, HTTP 429 or 5xx): the row stays for the next flush.
    Retry,
}

/// "Forward pending / target up" for one `carrier` pair (§5.9; mirrors `FabricEvent::Forward {
/// target, pending }`, coalesced per target per tick; no timer). A 5A flush trigger (§6.2), and one
/// post per event in 5B.
pub struct ForwardPending {
    pub target: NodeId,
    pub pending: u32,
}

/// The fabric side of the hand-off API as the carrier sees it (§5.9, amendment 11). D11 implements
/// it over `FabricHandle::fabric()`; P1's tests implement it with a fake.
pub trait HandOff: Send + Sync {
    /// Sealed frames owed to `target`, a `carrier` pair: forward puts and the `CUSTODY_ACK`s owed
    /// to it. An ack is consumed by this call and never returned again (C7).
    fn forward_take(
        &self,
        target: NodeId,
        max: usize,
    ) -> BoxFuture<'_, Result<Vec<CarrierFrame>, CarrierError>>;
    /// What the carrier did with a put it took.
    fn forward_done(
        &self,
        custody_seq: u64,
        done: ForwardDone,
    ) -> BoxFuture<'_, Result<(), CarrierError>>;
    /// A sealed frame from `from` (a `carrier` pair) that arrived through the carrier; verified
    /// like any sealed packet, processed with no connection the way a mailbox item is (INTRO
    /// allowed).
    fn carrier_inbound(
        &self,
        from: NodeId,
        frame: Vec<u8>,
    ) -> BoxFuture<'_, Result<(), CarrierError>>;
}

/// One `carrier` pair as a carrier sees it (§4.6, §6.2): this hub, the far hub, and the pairing's
/// carrier key.
pub struct Pair {
    /// This hub's NodeId (`NodeId_X` of `topic(X→Y)`, §6.2).
    pub own: NodeId,
    /// The far hub's NodeId.
    pub peer: NodeId,
    /// The ntfy `topic_key` (§6.2): BLAKE2b-256 keyed by the X25519 output behind the identity
    /// handle (M5), or keyed by `carrier.ntfy.topic_secret` when that is set. It names a mailbox;
    /// it protects no content. Never on any API.
    pub topic_key: Zeroizing<[u8; 32]>,
}

/// A carrier request that did not complete (§5.9, §6.2).
#[derive(Debug)]
pub enum CarrierError {
    /// Transient: no route, HTTP 429 or 5xx; the rows stay (`ForwardDone::Retry`).
    Retry(String),
    /// Permanent: HTTP 413, 401 or 403 (`ForwardDone::Rejected`).
    Rejected(&'static str),
    /// The saved `since` id is gone (the server lost its cache): read again with `since=all`, and
    /// re-post trigger (3) of §5.9.
    SinceMiss,
    /// The fabric side of the hand-off refused or failed.
    HandOff(String),
}

impl std::fmt::Display for CarrierError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CarrierError::Retry(why) => write!(f, "carrier: transient: {why}"),
            CarrierError::Rejected(why) => write!(f, "carrier: rejected: {why}"),
            CarrierError::SinceMiss => f.write_str("carrier: the saved since id is gone"),
            CarrierError::HandOff(why) => write!(f, "carrier: hand-off: {why}"),
        }
    }
}

impl std::error::Error for CarrierError {}

/// A carrier (§5.9, the node111 side): ntfy in v1 (`ntfy.rs`), chatmail after the G10 log (§6.5,
/// row D12).
pub trait Carrier: Send + Sync {
    /// `"ntfy"`.
    fn id(&self) -> &'static str;
    /// The largest sealed frame one carrier message holds (§6.1 rule 3; ntfy: 3,068 B, §6.2).
    fn max_frame(&self) -> usize;
    /// Post `out` in custody order, batched; one outcome per frame (`None`: an ack, logged only,
    /// C7).
    fn flush<'a>(
        &'a self,
        pair: &'a Pair,
        out: Vec<CarrierFrame>,
    ) -> BoxFuture<'a, Vec<(Option<u64>, ForwardDone)>>;
    /// 5A: one poll of the pair's inbound topic; the frames go to [`HandOff::carrier_inbound`].
    fn drain<'a>(&'a self, pair: &'a Pair) -> BoxFuture<'a, Result<Vec<Vec<u8>>, CarrierError>>;
    /// 5B: the held stream of the pair's inbound topic.
    fn hold<'a>(&'a self, pair: &'a Pair) -> BoxStream<'a, Result<Vec<u8>, CarrierError>>;
}
