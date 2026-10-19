//! The scripted [`StubAdapter`] (Layer 6 `60-transport/README.md` §4 "`StubAdapter`"): an
//! in-process [`Adapter`] with scripted accept, evidence, failures, queue-full answers,
//! airtime, an inbound feed and link flaps — the G1 fixture (§16), modelled on
//! `mesh111::link::lossy` (a test bearer with scripted behaviour, no sockets).
//!
//! - **Caps and facts per instance** ([`StubAdapter::update_caps`],
//!   [`StubAdapter::update_facts`]): kinds, per-kind caps, the airtime profile, this node's
//!   accounts, licensed mode and channel uplink can flip at runtime, so the live-consent
//!   rules (§8) are exercised the way a board flips them.
//! - **Sends** answer from a script ([`StubOutcome`], in order); with no script a send is
//!   accepted with a fresh native id and reaches `sent` ("accepted by the radio"). A full
//!   queue ([`StubAdapter::set_queue_full`]) answers `Failed { rate_limited, Later }`
//!   without consuming the script. [`StubOutcome::Hold`] keeps the handle open so a test
//!   emits later evidence by hand ([`StubAdapter::emit_held`]).
//! - **Inbound** is bounded ([`StubAdapter::with_inbound_cap`]): an overflow drops the
//!   oldest item and the next `recv` reports `Dropped { n }` first (§4: never silent).
//! - **The air** ([`StubAdapter::wire`]): a send on one stub is heard as an inbound text on
//!   its wired peers (the two-node relay cases), from the sending account, with the
//!   accepted native id.
//!
//! `recv` is cancel-safe (an item is taken only when it is returned), as the core requires
//! of every adapter: the recv task drains with `now_or_never`.

use super::{
    Adapter, AdapterFacts, AdapterState, Caps, ConvKind, ConvRef, ForeignMessageState, Honest,
    Inbound, InboundText, LinkState, PerKind, Reason, Retry, SendEvent, SendHandle, SendOpts,
};
use futures_util::future::BoxFuture;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::{mpsc, Notify};

/// Events buffered per send handle.
const EVENTS: usize = 64;
/// Default inbound queue bound.
const INBOUND_CAP: usize = 1 << 16;

/// What the stub answers for a send (scripted per test, §4 "StubAdapter").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StubOutcome {
    /// `Accepted {native_id}` then the listed evidence states in order.
    Accept {
        native_id: String,
        evidence: Vec<(ForeignMessageState, String)>,
    },
    /// Accepted (with the listed evidence), then a failure: a post-air NAK.
    AcceptThenFail {
        native_id: String,
        evidence: Vec<(ForeignMessageState, String)>,
        reason: Reason,
        retry: Retry,
    },
    Fail {
        reason: Reason,
        retry: Retry,
    },
    /// No event at all and the handle closes: the carrier vanished between the handoff and
    /// its answer (the crash case of §4 "At most once").
    Silent,
    /// No event yet; the handle stays open for [`StubAdapter::emit_held`].
    Hold,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// In-process adapter with scripted accept, evidence, failures, airtime and inbound feed:
/// the G1 fixture.
pub struct StubAdapter {
    id: &'static str,
    caps: Mutex<Caps>,
    facts: Mutex<AdapterFacts>,
    inbound: Mutex<VecDeque<Inbound>>,
    inbound_cap: usize,
    dropped: AtomicU64,
    wake: Notify,
    outcomes: Mutex<VecDeque<StubOutcome>>,
    sent: Mutex<Vec<(ConvRef, String)>>,
    next_native_id: AtomicU64,
    queue_full: AtomicBool,
    held: Mutex<Vec<mpsc::Sender<SendEvent>>>,
    peers: Mutex<Vec<Weak<StubAdapter>>>,
}

impl StubAdapter {
    pub fn new(id: &'static str, caps: Caps) -> Self {
        Self {
            id,
            caps: Mutex::new(caps),
            facts: Mutex::new(AdapterFacts::default()),
            inbound: Mutex::new(VecDeque::new()),
            inbound_cap: INBOUND_CAP,
            dropped: AtomicU64::new(0),
            wake: Notify::new(),
            outcomes: Mutex::new(VecDeque::new()),
            sent: Mutex::new(Vec::new()),
            next_native_id: AtomicU64::new(1),
            queue_full: AtomicBool::new(false),
            held: Mutex::new(Vec::new()),
            peers: Mutex::new(Vec::new()),
        }
    }

    /// The same stub with an inbound queue of at most `cap` items (overflow drops the
    /// oldest and is reported as `Dropped`).
    pub fn with_inbound_cap(mut self, cap: usize) -> Self {
        self.inbound_cap = cap.max(1);
        self
    }

    /// The same stub with these facts.
    pub fn with_facts(self, facts: AdapterFacts) -> Self {
        *lock(&self.facts) = facts;
        self
    }

    /// Default caps for a stub network: channel + DM, 232/220 B, every evidence state.
    pub fn default_caps(label: &str, tag: &str) -> Caps {
        Caps {
            kinds: vec![ConvKind::Channel, ConvKind::Dm],
            max_text_bytes: PerKind {
                channel: Some(232),
                dm: Some(220),
                group: None,
            },
            evidence: vec![
                ForeignMessageState::Queued,
                ForeignMessageState::Sent,
                ForeignMessageState::Relayed,
                ForeignMessageState::Delivered,
            ],
            airtime: None,
            auth_levels: vec![super::Auth::None, super::Auth::ChannelKey],
            copy: Honest {
                label: label.to_string(),
                chip: tag.to_string(),
                tag: tag.to_string(),
                banner: format!("{label}: a stub network (tests only)"),
                send_warning: String::new(),
            },
            recipient_syntax: None,
            splittable: true,
            state: AdapterState::Live,
        }
    }

    /// Feed one inbound item (text, upsert, nearby, link state); wakes a pending `recv`.
    pub fn feed(&self, item: Inbound) {
        self.feed_many(std::iter::once(item));
    }

    /// Feed many items at once (one wake-up: a burst the recv task drains in batches).
    pub fn feed_many(&self, items: impl IntoIterator<Item = Inbound>) {
        {
            let mut q = lock(&self.inbound);
            for item in items {
                if q.len() >= self.inbound_cap {
                    q.pop_front();
                    self.dropped.fetch_add(1, Ordering::SeqCst);
                }
                q.push_back(item);
            }
        }
        self.wake.notify_one();
    }

    /// Script the outcome of the next `send`s, in order. With no script, a send is
    /// accepted and reaches `sent` ("accepted by the radio").
    pub fn script(&self, outcome: StubOutcome) {
        lock(&self.outcomes).push_back(outcome);
    }

    pub fn set_state(&self, state: AdapterState) {
        lock(&self.caps).state = state;
    }

    /// Change the caps (kinds, limits, airtime, copy) at runtime.
    pub fn update_caps(&self, f: impl FnOnce(&mut Caps)) {
        f(&mut lock(&self.caps));
    }

    /// Change the live facts (accounts, licensed, uplink, region) at runtime.
    pub fn update_facts(&self, f: impl FnOnce(&mut AdapterFacts)) {
        f(&mut lock(&self.facts));
    }

    /// While `true`, every send answers `Failed { rate_limited, Later }` ("waiting for the
    /// radio": the board's queue has no free slot).
    pub fn set_queue_full(&self, full: bool) {
        self.queue_full.store(full, Ordering::SeqCst);
    }

    /// A link flap: `Down` then `Up` on the inbound feed.
    pub fn flap(&self) {
        self.feed_many([Inbound::Link(LinkState::Down), Inbound::Link(LinkState::Up)]);
    }

    /// Every `(conv, text)` handed to `send`, in order.
    pub fn sent(&self) -> Vec<(ConvRef, String)> {
        lock(&self.sent).clone()
    }

    /// How many sends were handed to this stub.
    pub fn sent_count(&self) -> usize {
        lock(&self.sent).len()
    }

    /// Handles kept open by [`StubOutcome::Hold`], in send order.
    pub fn held_count(&self) -> usize {
        lock(&self.held).len()
    }

    /// Emit `ev` on the `i`-th held handle; `false` when it is gone or closed.
    pub fn emit_held(&self, i: usize, ev: SendEvent) -> bool {
        let tx = lock(&self.held).get(i).cloned();
        tx.is_some_and(|tx| tx.try_send(ev).is_ok())
    }

    /// Wire two stubs as one radio channel: a send on either is heard by the other.
    pub fn wire(a: &Arc<StubAdapter>, b: &Arc<StubAdapter>) {
        lock(&a.peers).push(Arc::downgrade(b));
        lock(&b.peers).push(Arc::downgrade(a));
    }

    fn own_account(&self) -> Option<String> {
        lock(&self.facts).accounts.first().map(|a| a.id.clone())
    }

    /// The air: every wired peer hears `text` on the same conversation, from the account
    /// that sent it, carrying the accepted native id.
    fn transmit(&self, conv: &ConvRef, text: &str, native_id: &str) {
        let peers: Vec<Arc<StubAdapter>> =
            lock(&self.peers).iter().filter_map(Weak::upgrade).collect();
        for p in peers {
            let account = p.own_account().unwrap_or_else(|| conv.account.clone());
            p.feed(Inbound::Text(InboundText {
                conv: ConvRef {
                    network: p.id.to_string(),
                    account,
                    kind: conv.kind,
                    native_id: conv.native_id.clone(),
                },
                sender_native: conv.account.clone(),
                sender_name: format!("node {}", conv.account),
                auth: super::Auth::ChannelKey,
                native_msg_id: native_id.to_string(),
                body: text.to_string(),
                claim_ts: None,
                meta: serde_json::json!({}),
            }));
        }
    }

    fn fresh_native_id(&self) -> String {
        let n = self.next_native_id.fetch_add(1, Ordering::SeqCst);
        format!("{}-{n}", self.id)
    }
}

impl Adapter for StubAdapter {
    fn id(&self) -> &'static str {
        self.id
    }

    fn caps(&self) -> Caps {
        lock(&self.caps).clone()
    }

    fn facts(&self) -> AdapterFacts {
        lock(&self.facts).clone()
    }

    fn recv(&self) -> BoxFuture<'_, Inbound> {
        Box::pin(async move {
            loop {
                let n = self.dropped.swap(0, Ordering::SeqCst);
                if n > 0 {
                    return Inbound::Dropped { n };
                }
                if let Some(item) = lock(&self.inbound).pop_front() {
                    return item;
                }
                self.wake.notified().await;
            }
        })
    }

    fn send(&self, conv: &ConvRef, text: &str, _o: SendOpts) -> BoxFuture<'_, SendHandle> {
        lock(&self.sent).push((conv.clone(), text.to_string()));
        let (tx, rx) = mpsc::channel(EVENTS);
        let outcome = if self.queue_full.load(Ordering::SeqCst) {
            StubOutcome::Fail {
                reason: Reason::RateLimited,
                retry: Retry::Later,
            }
        } else {
            lock(&self.outcomes).pop_front().unwrap_or_else(|| {
                let native_id = self.fresh_native_id();
                StubOutcome::Accept {
                    native_id,
                    evidence: vec![(
                        ForeignMessageState::Sent,
                        "accepted by the radio".to_string(),
                    )],
                }
            })
        };
        let mut events = Vec::new();
        match outcome {
            StubOutcome::Accept {
                native_id,
                evidence,
            } => {
                self.transmit(conv, text, &native_id);
                events.push(SendEvent::Accepted { native_id });
                for (state, evidence) in evidence {
                    events.push(SendEvent::Evidence { state, evidence });
                }
            }
            StubOutcome::AcceptThenFail {
                native_id,
                evidence,
                reason,
                retry,
            } => {
                self.transmit(conv, text, &native_id);
                events.push(SendEvent::Accepted { native_id });
                for (state, evidence) in evidence {
                    events.push(SendEvent::Evidence { state, evidence });
                }
                events.push(SendEvent::Failed { reason, retry });
            }
            StubOutcome::Fail { reason, retry } => {
                events.push(SendEvent::Failed { reason, retry });
            }
            StubOutcome::Silent => {}
            StubOutcome::Hold => lock(&self.held).push(tx.clone()),
        }
        for ev in events {
            // EVENTS is far above any script; a full buffer only drops test evidence.
            let _ = tx.try_send(ev);
        }
        drop(tx);
        Box::pin(async move { SendHandle { events: rx } })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv() -> ConvRef {
        ConvRef {
            network: "stub-a".into(),
            account: "board-1".into(),
            kind: ConvKind::Channel,
            native_id: "ch:LongFast:abcd".into(),
        }
    }

    #[tokio::test]
    async fn stub_send_returns_at_once_and_streams_events() {
        let stub = StubAdapter::new("stub-a", StubAdapter::default_caps("Stub A", "A"));
        let mut h = stub.send(&conv(), "hello", SendOpts::default()).await;
        assert_eq!(
            h.events.recv().await,
            Some(SendEvent::Accepted {
                native_id: "stub-a-1".into()
            })
        );
        assert!(matches!(
            h.events.recv().await,
            Some(SendEvent::Evidence {
                state: ForeignMessageState::Sent,
                ..
            })
        ));
        assert_eq!(h.events.recv().await, None, "the handle closes");
        stub.script(StubOutcome::Fail {
            reason: Reason::RegionUnset,
            retry: Retry::Later,
        });
        let mut h = stub.send(&conv(), "again", SendOpts::default()).await;
        assert_eq!(
            h.events.recv().await,
            Some(SendEvent::Failed {
                reason: Reason::RegionUnset,
                retry: Retry::Later
            })
        );
        stub.set_queue_full(true);
        let mut h = stub.send(&conv(), "full", SendOpts::default()).await;
        assert!(matches!(
            h.events.recv().await,
            Some(SendEvent::Failed {
                reason: Reason::RateLimited,
                retry: Retry::Later
            })
        ));
        assert_eq!(stub.sent_count(), 3);
    }

    #[tokio::test]
    async fn stub_recv_waits_for_feed_and_reports_overflow() {
        let stub = Arc::new(
            StubAdapter::new("stub-b", StubAdapter::default_caps("Stub B", "B"))
                .with_inbound_cap(2),
        );
        let s2 = stub.clone();
        let waiter = tokio::spawn(async move { s2.recv().await });
        tokio::task::yield_now().await;
        stub.feed(Inbound::Link(LinkState::Up));
        assert_eq!(waiter.await.unwrap(), Inbound::Link(LinkState::Up));
        stub.feed_many([
            Inbound::Link(LinkState::Down),
            Inbound::Link(LinkState::Up),
            Inbound::Link(LinkState::Partitioned),
        ]);
        assert_eq!(stub.recv().await, Inbound::Dropped { n: 1 }, "never silent");
        assert_eq!(stub.recv().await, Inbound::Link(LinkState::Up));
        assert_eq!(stub.recv().await, Inbound::Link(LinkState::Partitioned));
    }

    #[tokio::test]
    async fn wired_stubs_hear_each_other_and_held_handles_emit_later() {
        let a = Arc::new(StubAdapter::new(
            "stub-a",
            StubAdapter::default_caps("A", "A"),
        ));
        let b = Arc::new(StubAdapter::new(
            "stub-a",
            StubAdapter::default_caps("A", "A"),
        ));
        StubAdapter::wire(&a, &b);
        let _ = a.send(&conv(), "over the air", SendOpts::default()).await;
        match b.recv().await {
            Inbound::Text(t) => {
                assert_eq!(t.body, "over the air");
                assert_eq!(t.sender_native, "board-1");
                assert_eq!(t.native_msg_id, "stub-a-1");
            }
            other => panic!("{other:?}"),
        }
        a.script(StubOutcome::Hold);
        let mut h = a.send(&conv(), "later", SendOpts::default()).await;
        assert!(h.events.try_recv().is_err());
        assert!(a.emit_held(
            0,
            SendEvent::Accepted {
                native_id: "x".into()
            }
        ));
        assert_eq!(
            h.events.recv().await,
            Some(SendEvent::Accepted {
                native_id: "x".into()
            })
        );
    }
}
