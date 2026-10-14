//! `lossy-loopback` — a **test-only** in-process bearer (§20 G1: "Tier-1
//! fragments over a test-only `lossy-loopback` link at a forced 160 B
//! profile with loss injection and SACK retransmit"). Owner: F1.
//!
//! Two ends of one pipe ([`pair`]). Each end advertises a configurable
//! [`PathProfile`] (default: the 160 B unknown-path profile, datagram,
//! unreliable), refuses frames above its MTU (`LinkError::TooLarge`), and on
//! the way across drops frames (deterministic PRNG, `loss_permille`),
//! reorders them (`reorder_permille`: a frame is held back and released
//! after the next one) and runs an optional per-frame filter (tests use it
//! to drop exactly the frames a scenario needs). Each end's own announce is
//! delivered to the other end first, losslessly — the same "announce first"
//! shape as `tcp-lan`, so the fabric learns which NodeId sits at the far
//! end — and both ends report [`LinkEvent::Connected`] (outbound, so the
//! fabric may send on the claim, like a dial to a configured address).
//!
//! Never constructed by the daemon. No timers, no threads: frames move
//! inside `send`. Its in-process queues are unbounded (both ends are driven
//! by the tests' own fabrics); the bounded-inbound rule of
//! [`Link::recv`] is for real bearers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use tokio::sync::mpsc;

use super::{
    EventSink, Frame, Link, LinkError, LinkEvent, LinkId, LinkKind, LinkState, PathProfile,
    PeerAddr,
};

/// A per-frame drop filter: `true` drops the frame.
pub type DropFilter = Arc<dyn Fn(&Frame) -> bool + Send + Sync>;

/// Loss and reorder settings for one direction (the sending end's).
#[derive(Clone)]
pub struct LossyConfig {
    /// The advertised profile (MTU enforced on send).
    pub profile: PathProfile,
    /// The adapter kind this test bearer stands in for.
    pub kind: LinkKind,
    /// Frames dropped per thousand.
    pub loss_permille: u16,
    /// Frames held back (released after the next frame) per thousand.
    pub reorder_permille: u16,
    /// PRNG seed (xorshift64*; deterministic per seed).
    pub seed: u64,
    /// Optional per-frame filter (`true` = drop).
    pub filter: Option<DropFilter>,
}

impl std::fmt::Debug for LossyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LossyConfig")
            .field("profile", &self.profile)
            .field("kind", &self.kind)
            .field("loss_permille", &self.loss_permille)
            .field("reorder_permille", &self.reorder_permille)
            .field("seed", &self.seed)
            .field("filter", &self.filter.is_some())
            .finish()
    }
}

impl Default for LossyConfig {
    fn default() -> Self {
        LossyConfig {
            profile: PathProfile::unknown(),
            kind: LinkKind::KissModem,
            loss_permille: 0,
            reorder_permille: 0,
            seed: 0x1111_5eed,
            filter: None,
        }
    }
}

/// Counters for one end's outbound direction.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LossyStats {
    /// Frames handed to `send` (within the MTU).
    pub sent: u64,
    /// Frames dropped by loss or filter.
    pub dropped: u64,
    /// Frames held back for reordering (each released later).
    pub reordered: u64,
    /// Frames refused as above the MTU.
    pub too_large: u64,
}

struct Dir {
    cfg: LossyConfig,
    rng: u64,
    held: Option<Frame>,
    stats: LossyStats,
}

impl Dir {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn roll(&mut self, permille: u16) -> bool {
        permille > 0 && self.next() % 1000 < u64::from(permille)
    }
}

/// One end of a lossy pipe.
pub struct LossyEnd {
    id: LinkId,
    /// How the far end addresses this end.
    me: PeerAddr,
    /// How this end addresses the far end.
    far: PeerAddr,
    dir: Mutex<Dir>,
    to_far: mpsc::UnboundedSender<(PeerAddr, Frame)>,
    rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<(PeerAddr, Frame)>>,
    delivered: AtomicU64,
}

impl std::fmt::Debug for LossyEnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LossyEnd")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// One side's construction inputs.
#[derive(Debug, Clone)]
pub struct EndSpec {
    /// Link id for this end.
    pub id: LinkId,
    /// This end's own announce (delivered to the far end first).
    pub announce: Vec<u8>,
    /// Where this end reports its `Connected` event.
    pub events: EventSink,
    /// Loss/reorder applied to frames this end sends.
    pub cfg: LossyConfig,
}

/// Build a connected pair. Each end's announce reaches the other first
/// (lossless), then both report `Connected { outbound: true }`.
pub fn pair(a: EndSpec, b: EndSpec) -> (Arc<LossyEnd>, Arc<LossyEnd>) {
    let (tx_a, rx_a) = mpsc::unbounded_channel(); // frames for A
    let (tx_b, rx_b) = mpsc::unbounded_channel(); // frames for B
    let addr_a = PeerAddr::Opaque(b"lossy-a".to_vec());
    let addr_b = PeerAddr::Opaque(b"lossy-b".to_vec());
    let _ = tx_b.send((addr_a.clone(), Frame::announce(a.announce.clone())));
    let _ = tx_a.send((addr_b.clone(), Frame::announce(b.announce.clone())));
    let end_a = Arc::new(LossyEnd {
        id: a.id.clone(),
        me: addr_a.clone(),
        far: addr_b.clone(),
        dir: Mutex::new(Dir {
            rng: a.cfg.seed.max(1),
            cfg: a.cfg,
            held: None,
            stats: LossyStats::default(),
        }),
        to_far: tx_b,
        rx: tokio::sync::Mutex::new(rx_a),
        delivered: AtomicU64::new(0),
    });
    let end_b = Arc::new(LossyEnd {
        id: b.id.clone(),
        me: addr_b.clone(),
        far: addr_a.clone(),
        dir: Mutex::new(Dir {
            rng: b.cfg.seed.max(1) ^ 0x9e37_79b9,
            cfg: b.cfg,
            held: None,
            stats: LossyStats::default(),
        }),
        to_far: tx_a,
        rx: tokio::sync::Mutex::new(rx_b),
        delivered: AtomicU64::new(0),
    });
    let _ = a.events.send(LinkEvent::Connected {
        link: a.id,
        peer: addr_b,
        outbound: true,
        expected: None,
    });
    let _ = b.events.send(LinkEvent::Connected {
        link: b.id,
        peer: addr_a,
        outbound: true,
        expected: None,
    });
    (end_a, end_b)
}

impl LossyEnd {
    /// Replace the loss/reorder settings of this end's outbound direction.
    pub fn set_loss(&self, loss_permille: u16, reorder_permille: u16) {
        if let Ok(mut d) = self.dir.lock() {
            d.cfg.loss_permille = loss_permille;
            d.cfg.reorder_permille = reorder_permille;
        }
    }

    /// Replace the drop filter.
    pub fn set_filter(&self, filter: Option<DropFilter>) {
        if let Ok(mut d) = self.dir.lock() {
            d.cfg.filter = filter;
        }
    }

    /// Release a held (reordered) frame now.
    pub fn flush_held(&self) {
        let held = self.dir.lock().ok().and_then(|mut d| d.held.take());
        if let Some(f) = held {
            self.deliver(f);
        }
    }

    /// Outbound counters.
    pub fn stats(&self) -> LossyStats {
        self.dir.lock().map(|d| d.stats).unwrap_or_default()
    }

    /// Frames this end has handed to the far end.
    pub fn delivered(&self) -> u64 {
        self.delivered.load(Ordering::SeqCst)
    }

    fn deliver(&self, f: Frame) {
        self.delivered.fetch_add(1, Ordering::SeqCst);
        let _ = self.to_far.send((self.me.clone(), f));
    }
}

impl Link for LossyEnd {
    fn id(&self) -> LinkId {
        self.id.clone()
    }

    fn kind(&self) -> LinkKind {
        self.dir
            .lock()
            .map(|d| d.cfg.kind)
            .unwrap_or(LinkKind::KissModem)
    }

    fn profile(&self) -> PathProfile {
        self.dir
            .lock()
            .map(|d| d.cfg.profile)
            .unwrap_or_else(|_| PathProfile::unknown())
    }

    fn peers(&self) -> Vec<PeerAddr> {
        vec![self.far.clone()]
    }

    fn send(&self, to: PeerAddr, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            if to != self.far {
                return Err(LinkError::Closed);
            }
            let mut out: Vec<Frame> = Vec::with_capacity(2);
            {
                let mut d = self.dir.lock().map_err(|_| LinkError::Closed)?;
                let mtu = usize::from(d.cfg.profile.mtu_bytes);
                let wire = frame.bytes.len() + d.cfg.profile.shim_len();
                if wire > mtu {
                    d.stats.too_large += 1;
                    return Err(LinkError::TooLarge(wire));
                }
                d.stats.sent += 1;
                let filtered = d.cfg.filter.as_ref().is_some_and(|f| f(&frame));
                let loss = d.cfg.loss_permille;
                if filtered || d.roll(loss) {
                    d.stats.dropped += 1;
                    return Ok(()); // "written to the link"; lost in flight
                }
                let reorder = d.cfg.reorder_permille;
                if d.held.is_none() && d.roll(reorder) {
                    d.stats.reordered += 1;
                    d.held = Some(frame);
                    return Ok(());
                }
                out.push(frame);
                if let Some(h) = d.held.take() {
                    out.push(h); // the held frame arrives after a later one
                }
            }
            for f in out {
                self.deliver(f);
            }
            Ok(())
        })
    }

    fn recv(&self) -> BoxFuture<'_, (PeerAddr, Frame)> {
        Box::pin(async move {
            let mut rx = self.rx.lock().await;
            match rx.recv().await {
                Some(x) => x,
                None => futures::future::pending().await,
            }
        })
    }

    fn state(&self) -> LinkState {
        LinkState::Up { peers: 1 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::FrameType;

    fn spec(name: &str, cfg: LossyConfig) -> (EndSpec, mpsc::UnboundedReceiver<LinkEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            EndSpec {
                id: LinkId::new(name),
                announce: vec![1; 137],
                events: tx,
                cfg,
            },
            rx,
        )
    }

    #[tokio::test]
    async fn announce_first_then_mtu_loss_reorder() {
        let (sa, mut ea) = spec("a", LossyConfig::default());
        let (sb, _eb) = spec(
            "b",
            LossyConfig {
                reorder_permille: 1000,
                ..LossyConfig::default()
            },
        );
        let (a, b) = pair(sa, sb);
        assert!(matches!(
            ea.recv().await,
            Some(LinkEvent::Connected { outbound: true, .. })
        ));
        let (from, f) = b.recv().await;
        assert_eq!(f.frame_type, FrameType::Announce);
        assert_eq!(from, PeerAddr::Opaque(b"lossy-a".to_vec()));
        // MTU 160 enforced
        assert!(matches!(
            a.send(b.me.clone(), Frame::sealed(vec![0; 161])).await,
            Err(LinkError::TooLarge(161))
        ));
        // b reorders every frame it can: 1 is held, 2 then 1 arrive.
        b.send(a.me.clone(), Frame::sealed(vec![1])).await.unwrap();
        b.send(a.me.clone(), Frame::sealed(vec![2])).await.unwrap();
        let _announce = a.recv().await;
        assert_eq!(a.recv().await.1.bytes, vec![2]);
        assert_eq!(a.recv().await.1.bytes, vec![1]);
        // full loss
        a.set_loss(1000, 0);
        a.send(b.me.clone(), Frame::sealed(vec![3])).await.unwrap();
        assert_eq!(a.stats().dropped, 1);
        // filter
        a.set_loss(0, 0);
        a.set_filter(Some(Arc::new(|f: &Frame| f.bytes == vec![4])));
        a.send(b.me.clone(), Frame::sealed(vec![4])).await.unwrap();
        a.send(b.me.clone(), Frame::sealed(vec![5])).await.unwrap();
        assert_eq!(b.recv().await.1.bytes, vec![5]);
    }
}
