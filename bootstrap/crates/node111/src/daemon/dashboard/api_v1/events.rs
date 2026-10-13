//! `GET /api/v1/events` — the one live stream (spec §10.1, api/README.md "SSE").
//!
//! - `id:` is one global monotonic sequence assigned here at publish time. The
//!   sequence starts at `boot_ms × 1000` (not 0), so ids are still monotonic
//!   across daemon restarts (JS-safe: < 2^53 until ~2255) and a `Last-Event-ID`
//!   from a previous lifetime is recognisable.
//! - Ring buffer of [`RING`] frames. `Last-Event-ID: n` replays every frame with
//!   id > n when n is still covered by the ring. Otherwise the first frame is
//!   `event: reset` with `{"reason": ...}` and the client refetches snapshots:
//!   - `restart` — n belongs to another daemon lifetime (below this lifetime's
//!     base, or above the current head);
//!   - `expired` — n fell out of the ring, or no `Last-Event-ID` at all (a fresh
//!     client has nothing to resume).
//!
//!   The reset frame carries `id: <head>` so the client's next reconnect resumes
//!   from here.
//! - `: keep-alive` comment every [`super::KEEPALIVE`] (15 s).
//! - A subscriber that lags more than the broadcast capacity is disconnected; the
//!   browser reconnects with its `Last-Event-ID` and gets replay-or-reset.
//!
//! Producers (R2 `status`/`services`, R3 `proposal`/`agent`) call
//! [`EventBus::publish`] with an event name from `x-sse-events.events`.

use super::{auth, ApiRouter, ApiState};
use axum::extract::State;
use axum::http::{Extensions, HeaderMap};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use futures_util::stream::{self, Stream, StreamExt};
use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// Replay ring size (contract: 256).
pub const RING: usize = 256;

/// One published frame. `data` is compact JSON (never contains a newline).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub id: u64,
    pub event: String,
    pub data: String,
}

/// Why a subscriber starts with `event: reset` instead of a replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetReason {
    Expired,
    Restart,
}

impl ResetReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ResetReason::Expired => "expired",
            ResetReason::Restart => "restart",
        }
    }
}

struct Ring {
    frames: VecDeque<Arc<Frame>>,
    /// Ids of this lifetime are > base.
    base: u64,
    /// Last assigned id (== base before the first publish).
    head: u64,
}

/// The daemon-wide event bus (ring + broadcast fan-out).
pub struct EventBus {
    ring: Mutex<Ring>,
    tx: broadcast::Sender<Arc<Frame>>,
}

/// What a new subscriber gets: frames to replay first (or a reset), then live
/// frames from `rx` with id > `head`.
pub struct Subscription {
    pub replay: Vec<Arc<Frame>>,
    pub reset: Option<ResetReason>,
    pub head: u64,
    pub rx: broadcast::Receiver<Arc<Frame>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

fn boot_base() -> u64 {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    ms.saturating_mul(1000)
}

impl EventBus {
    pub fn new() -> Self {
        Self::with_base(boot_base())
    }

    /// Bus whose first id is `base + 1` (tests pin the base).
    pub fn with_base(base: u64) -> Self {
        let (tx, _) = broadcast::channel(RING);
        EventBus {
            ring: Mutex::new(Ring {
                frames: VecDeque::with_capacity(RING),
                base,
                head: base,
            }),
            tx,
        }
    }

    /// Publish `data` as `event: <name>`; returns the assigned id. Event names
    /// come from the contract's `x-sse-events.events` map.
    pub fn publish(&self, name: &str, data: &serde_json::Value) -> u64 {
        let event: String = name.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        let data = data.to_string(); // compact: JSON escapes newlines inside strings
        let mut ring = self.ring.lock().unwrap();
        ring.head += 1;
        let frame = Arc::new(Frame {
            id: ring.head,
            event,
            data,
        });
        if ring.frames.len() == RING {
            ring.frames.pop_front();
        }
        ring.frames.push_back(Arc::clone(&frame));
        // Sent under the ring lock so subscribe() sees ring + live consistently.
        let _ = self.tx.send(frame);
        ring.head
    }

    /// Last assigned id (== [`Self::base`] before the first publish).
    pub fn head(&self) -> u64 {
        self.ring.lock().unwrap().head
    }

    pub fn base(&self) -> u64 {
        self.ring.lock().unwrap().base
    }

    /// Subscribe resuming after `last_event_id` (see module docs for the rules).
    pub fn subscribe(&self, last_event_id: Option<u64>) -> Subscription {
        let ring = self.ring.lock().unwrap();
        let rx = self.tx.subscribe();
        let head = ring.head;
        let (replay, reset) = match last_event_id {
            None => (Vec::new(), Some(ResetReason::Expired)),
            Some(n) if n < ring.base || n > head => (Vec::new(), Some(ResetReason::Restart)),
            Some(n) if n == head => (Vec::new(), None),
            Some(n) => match ring.frames.front() {
                Some(oldest) if n + 1 >= oldest.id => (
                    ring.frames.iter().filter(|f| f.id > n).cloned().collect(),
                    None,
                ),
                _ => (Vec::new(), Some(ResetReason::Expired)),
            },
        };
        Subscription {
            replay,
            reset,
            head,
            rx,
        }
    }
}

fn to_event(f: &Frame) -> Event {
    Event::default()
        .id(f.id.to_string())
        .event(&f.event)
        .data(&f.data)
}

/// The SSE body for one subscriber: reset-or-replay, then live frames.
pub fn stream_for(sub: Subscription) -> impl Stream<Item = Result<Event, Infallible>> {
    let mut first: Vec<Result<Event, Infallible>> = Vec::new();
    if let Some(reason) = sub.reset {
        first.push(Ok(Event::default()
            .id(sub.head.to_string())
            .event("reset")
            .data(
                serde_json::json!({ "reason": reason.as_str() }).to_string(),
            )));
    }
    first.extend(sub.replay.iter().map(|f| Ok(to_event(f))));
    let live = stream::unfold((sub.rx, sub.head), |(mut rx, head)| async move {
        loop {
            match rx.recv().await {
                Ok(f) if f.id <= head => continue,
                Ok(f) => return Some((Ok(to_event(&f)), (rx, head))),
                // Lagged: drop the connection; the client resumes via
                // Last-Event-ID (replay or reset). Closed: daemon shutting down.
                Err(_) => return None,
            }
        }
    });
    stream::iter(first).chain(live)
}

fn last_event_id(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse().ok())
}

async fn events(
    State(api): State<Arc<ApiState>>,
    ext: Extensions,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let sub = api.events.subscribe(last_event_id(&headers));
    let keepalive = api.keepalive;
    // A paired (non-loopback) stream ends at the first frame after its session
    // is revoked (logout, `111 dashboard rotate`, expiry); the reconnect then
    // gets 401 and the UI shows /pair. Loopback streams are never cut.
    let token = if auth::peer_is_loopback(&ext) {
        None
    } else {
        auth::session_cookie(&headers)
    };
    let stream = stream_for(sub).take_while(move |_| {
        let ok = token
            .as_deref()
            .is_none_or(|t| api.auth.session_for(t).is_some());
        std::future::ready(ok)
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(keepalive).text("keep-alive"))
}

pub fn router() -> ApiRouter {
    axum::Router::new().route("/api/v1/events", get(events))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(sub: &Subscription) -> Vec<u64> {
        sub.replay.iter().map(|f| f.id).collect()
    }

    #[test]
    fn replay_reset_rules() {
        let bus = EventBus::with_base(1000);
        assert_eq!(bus.head(), 1000);
        // fresh client → reset expired, nothing to replay
        let s = bus.subscribe(None);
        assert_eq!(s.reset, Some(ResetReason::Expired));
        // resume at head before any publish → live only
        assert_eq!(bus.subscribe(Some(1000)).reset, None);
        for i in 0..3 {
            bus.publish("status", &serde_json::json!({ "i": i }));
        }
        assert_eq!(bus.head(), 1003);
        let s = bus.subscribe(Some(1001));
        assert_eq!(s.reset, None);
        assert_eq!(ids(&s), vec![1002, 1003]);
        // from the base itself (seen nothing of this lifetime yet) → full replay
        assert_eq!(ids(&bus.subscribe(Some(1000))), vec![1001, 1002, 1003]);
        // previous lifetime / future id → restart
        assert_eq!(bus.subscribe(Some(5)).reset, Some(ResetReason::Restart));
        assert_eq!(bus.subscribe(Some(2000)).reset, Some(ResetReason::Restart));
    }

    #[test]
    fn ring_overflow_expires_old_positions() {
        let bus = EventBus::with_base(0);
        for _ in 0..(RING + 10) {
            bus.publish("services", &serde_json::json!({}));
        }
        // oldest kept = 11; resuming from 10 is still contiguous, 9 is not
        assert_eq!(bus.subscribe(Some(10)).replay.len(), RING);
        assert_eq!(bus.subscribe(Some(9)).reset, Some(ResetReason::Expired));
    }

    #[test]
    fn event_names_cannot_inject_lines() {
        let bus = EventBus::with_base(0);
        bus.publish("status\ndata: x", &serde_json::json!({"t": "a\nb"}));
        let s = bus.subscribe(Some(0));
        assert_eq!(s.replay[0].event, "statusdata: x");
        assert!(!s.replay[0].data.contains('\n'));
    }
}
