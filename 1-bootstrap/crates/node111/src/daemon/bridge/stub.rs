//! The scripted [`StubAdapter`] (Layer 6 `6-transport/README.md` §4 "`StubAdapter`"): an
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

/// The debug-build-only G1 scenario hook (§16 G1; `6-transport/verify/bridge.sh`): with
/// `NODE111_BRIDGE_STUBS=<file>` set, a **debug** `111 daemon` enables the [`StubAdapter`]s
/// that file declares in its running bridge core and drives them from a control directory,
/// so the gate script exercises the bridge through the real `/api/v1` of a real daemon.
/// Compiled out of release builds entirely (`cfg(debug_assertions)`): a shipped `111`
/// carries no stub, no hook and no reader of the variable (`AGENTS.md`: the Raspberry Pi is
/// the floor). With the variable unset, [`scenario::start`] returns at once and spawns
/// nothing.
///
/// - **The scenario file** (JSON, ≤ 1 MiB, `deny_unknown_fields`): `control` (a directory),
///   `node` (a short name), optional `air` (a directory shared by the nodes of one run),
///   and `stubs`: per stub `id` (a network id), `label`, `tag`, optional `kinds`,
///   `max_text_bytes`, `airtime`, `recipient_syntax`, `facts` ([`AdapterFacts`]),
///   `inbound_cap` and `air_name` (the name this node's account carries on the air).
/// - **Control** (polled every 100 ms by one task, debug builds only): the script writes
///   `<control>/<seq>.json` (atomically, by rename); the hook runs it, writes
///   `<seq>.done` (`{"ok": true, ...}` or `{"ok": false, "error": ...}`) and removes the
///   command. Operations (`op`): `feed` (`items`: [`Inbound`] values), `burst` (`n` texts
///   in one conversation, one wake-up), `script` (send outcomes, in order; `replace` drops
///   what is still scripted), `facts`
///   (replace the live facts), `state` (the adapter state), `queue_full`, `emit_held`,
///   `pin` (a conversation's pinned flag, which no R-B route sets).
/// - **Sends** are appended to `<control>/<id>.sent.jsonl` (one line per `send`, the
///   conversation, the text and the scripted outcome), so the script counts what reached
///   the stub — across a `kill -9` and a restart (the native ids continue from the log's
///   line count, so a restarted stub never reuses one).
/// - **The air**: an accepted send is appended to `<air>/<node>.jsonl`; every other node's
///   file is read from where it stood at start and fed to the stub of the same network id
///   as an inbound text from the sending account (the cross-node loop cases of §8).
///
/// Errors in a command are answered in its `.done` file; nothing here panics on input.
#[cfg(debug_assertions)]
pub mod scenario {
    use super::{
        lock, Adapter, AdapterFacts, AdapterState, Caps, ConvKind, ConvRef, ForeignMessageState,
        Inbound, InboundText, PerKind, Reason, Retry, SendEvent, SendHandle, SendOpts, StubAdapter,
        StubOutcome,
    };
    use crate::daemon::bridge::{AirtimeProfile, Auth, Bridge, Syntax};
    use futures_util::future::BoxFuture;
    use serde::Deserialize;
    use serde_json::{json, Value};
    use std::collections::{BTreeMap, VecDeque};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// The variable naming the scenario file.
    pub const ENV: &str = "NODE111_BRIDGE_STUBS";
    /// Largest scenario or command file read (checked before parsing).
    const MAX_FILE: u64 = 1 << 20;
    /// Largest `burst`.
    const MAX_BURST: usize = 100_000;
    /// Control and air poll period.
    const POLL: Duration = Duration::from_millis(100);

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ScenarioFile {
        control: PathBuf,
        #[serde(default)]
        node: String,
        #[serde(default)]
        air: Option<PathBuf>,
        stubs: Vec<Spec>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Spec {
        id: String,
        label: String,
        tag: String,
        #[serde(default)]
        kinds: Option<Vec<ConvKind>>,
        #[serde(default)]
        max_text_bytes: Option<PerKind>,
        #[serde(default)]
        airtime: Option<AirtimeProfile>,
        #[serde(default)]
        recipient_syntax: Option<Syntax>,
        #[serde(default)]
        facts: Option<AdapterFacts>,
        #[serde(default)]
        inbound_cap: Option<usize>,
        #[serde(default)]
        air_name: Option<String>,
    }

    /// A scripted send outcome as the control file spells it.
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "snake_case", deny_unknown_fields)]
    enum Outcome {
        Accept {
            #[serde(default)]
            native_id: Option<String>,
            #[serde(default)]
            evidence: Vec<(ForeignMessageState, String)>,
        },
        AcceptThenFail {
            #[serde(default)]
            native_id: Option<String>,
            #[serde(default)]
            evidence: Vec<(ForeignMessageState, String)>,
            reason: Reason,
            retry: Retry,
        },
        Fail {
            reason: Reason,
            retry: Retry,
        },
        Silent,
        Hold,
    }

    /// An event for a held handle (`emit_held`).
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "snake_case", deny_unknown_fields)]
    enum HeldEvent {
        Accepted {
            native_id: String,
        },
        Evidence {
            state: ForeignMessageState,
            evidence: String,
        },
        Failed {
            reason: Reason,
            retry: Retry,
        },
    }

    #[derive(Debug, Deserialize)]
    #[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
    enum Command {
        Feed {
            stub: String,
            items: Vec<Inbound>,
        },
        Burst {
            stub: String,
            conv: ConvRef,
            n: usize,
            sender_native: String,
            sender_name: String,
            prefix: String,
        },
        Script {
            stub: String,
            outcomes: Vec<Outcome>,
            /// Drop what is still scripted first.
            #[serde(default)]
            replace: bool,
        },
        Facts {
            stub: String,
            facts: AdapterFacts,
        },
        State {
            stub: String,
            state: AdapterState,
        },
        QueueFull {
            stub: String,
            on: bool,
        },
        EmitHeld {
            stub: String,
            index: usize,
            event: HeldEvent,
        },
        /// Pin or unpin a foreign conversation (`fc-<conv>`; no route sets it in R-B, and
        /// pinned rows are on the prune "never" list, §3).
        Pin {
            conv: i64,
            pinned: bool,
        },
    }

    /// One line on the air.
    #[derive(Debug, Deserialize)]
    struct AirLine {
        node: String,
        conv: ConvRef,
        native_id: String,
        text: String,
        name: String,
    }

    /// A [`StubAdapter`] with the hook's send log, its own outcome queue (so a restarted
    /// stub never reuses a native id) and the air.
    struct ScenarioStub {
        id: &'static str,
        inner: Arc<StubAdapter>,
        outcomes: Mutex<VecDeque<Outcome>>,
        queue_full: AtomicBool,
        sends: AtomicU64,
        log: PathBuf,
        node: String,
        air: Option<PathBuf>,
        air_name: String,
    }

    fn append(path: &Path, line: &Value) {
        let r = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut f| writeln!(f, "{line}"));
        if let Err(e) = r {
            eprintln!("111 bridge scenario: {}: {e}", path.display());
        }
    }

    fn file_slug(id: &str) -> String {
        id.replace(':', "-")
    }

    impl ScenarioStub {
        fn native_id(&self, n: u64, given: Option<String>) -> String {
            given.unwrap_or_else(|| format!("{}-{}-{n}", self.id, self.node))
        }

        fn outcome(&self, n: u64) -> (StubOutcome, &'static str) {
            if self.queue_full.load(Ordering::SeqCst) {
                return (
                    StubOutcome::Fail {
                        reason: Reason::RateLimited,
                        retry: Retry::Later,
                    },
                    "queue_full",
                );
            }
            match lock(&self.outcomes).pop_front() {
                Some(Outcome::Accept {
                    native_id,
                    evidence,
                }) => (
                    StubOutcome::Accept {
                        native_id: self.native_id(n, native_id),
                        evidence,
                    },
                    "accept",
                ),
                Some(Outcome::AcceptThenFail {
                    native_id,
                    evidence,
                    reason,
                    retry,
                }) => (
                    StubOutcome::AcceptThenFail {
                        native_id: self.native_id(n, native_id),
                        evidence,
                        reason,
                        retry,
                    },
                    "accept_then_fail",
                ),
                Some(Outcome::Fail { reason, retry }) => {
                    (StubOutcome::Fail { reason, retry }, "fail")
                }
                Some(Outcome::Silent) => (StubOutcome::Silent, "silent"),
                Some(Outcome::Hold) => (StubOutcome::Hold, "hold"),
                None => (
                    StubOutcome::Accept {
                        native_id: self.native_id(n, None),
                        evidence: vec![(
                            ForeignMessageState::Sent,
                            "accepted by the radio".to_string(),
                        )],
                    },
                    "accept",
                ),
            }
        }
    }

    impl Adapter for ScenarioStub {
        fn id(&self) -> &'static str {
            self.id
        }

        fn caps(&self) -> Caps {
            self.inner.caps()
        }

        fn facts(&self) -> AdapterFacts {
            self.inner.facts()
        }

        fn recv(&self) -> BoxFuture<'_, Inbound> {
            self.inner.recv()
        }

        fn send(&self, conv: &ConvRef, text: &str, o: SendOpts) -> BoxFuture<'_, SendHandle> {
            let n = self.sends.fetch_add(1, Ordering::SeqCst) + 1;
            let (outcome, tag) = self.outcome(n);
            let native = match &outcome {
                StubOutcome::Accept { native_id, .. }
                | StubOutcome::AcceptThenFail { native_id, .. } => Some(native_id.clone()),
                _ => None,
            };
            append(
                &self.log,
                &json!({"n": n, "conv": conv, "text": text, "outcome": tag, "native_id": native}),
            );
            if let (Some(air), Some(native_id)) = (&self.air, &native) {
                append(
                    &air.join(format!("{}.jsonl", self.node)),
                    &json!({"node": self.node, "conv": conv, "native_id": native_id,
                            "text": text, "name": self.air_name}),
                );
            }
            self.inner.script(outcome);
            self.inner.send(conv, text, o)
        }
    }

    struct Hook {
        bridge: Arc<Bridge>,
        stubs: BTreeMap<String, Arc<ScenarioStub>>,
        control: PathBuf,
        node: String,
        air: Option<PathBuf>,
        /// Read offsets of the other nodes' air files.
        offsets: BTreeMap<PathBuf, u64>,
    }

    fn read_capped(path: &Path) -> Result<Vec<u8>, String> {
        let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
        if len > MAX_FILE {
            return Err(format!("{} is larger than 1 MiB", path.display()));
        }
        std::fs::read(path).map_err(|e| e.to_string())
    }

    fn build_stub(spec: Spec, sc: &ScenarioFile) -> Result<Arc<ScenarioStub>, String> {
        // the network id must outlive the adapter (`Adapter::id` is 'static): leaked once
        // per stub per daemon start, debug builds only
        let id: &'static str = Box::leak(spec.id.clone().into_boxed_str());
        let mut caps = StubAdapter::default_caps(&spec.label, &spec.tag);
        if let Some(k) = spec.kinds {
            caps.kinds = k;
        }
        if let Some(m) = spec.max_text_bytes {
            caps.max_text_bytes = m;
        }
        caps.airtime = spec.airtime;
        caps.recipient_syntax = spec.recipient_syntax;
        let mut inner = StubAdapter::new(id, caps).with_facts(spec.facts.unwrap_or_default());
        if let Some(c) = spec.inbound_cap {
            inner = inner.with_inbound_cap(c);
        }
        let log = sc.control.join(format!("{}.sent.jsonl", file_slug(id)));
        let before = std::fs::read_to_string(&log)
            .map(|t| t.lines().count() as u64)
            .unwrap_or(0);
        let air_name = spec.air_name.unwrap_or_else(|| format!("node {}", sc.node));
        Ok(Arc::new(ScenarioStub {
            id,
            inner: Arc::new(inner),
            outcomes: Mutex::new(VecDeque::new()),
            queue_full: AtomicBool::new(false),
            sends: AtomicU64::new(before),
            log,
            node: sc.node.clone(),
            air: sc.air.clone(),
            air_name,
        }))
    }

    /// Daemon start (debug builds): enable the stubs `$NODE111_BRIDGE_STUBS` declares and
    /// spawn the one control task. Returns at once, spawning nothing, when it is unset.
    pub fn start(bridge: Arc<Bridge>) {
        let Some(path) = std::env::var_os(ENV) else {
            return;
        };
        let path = PathBuf::from(path);
        tokio::spawn(async move {
            if let Err(e) = run(bridge, &path).await {
                eprintln!("111 bridge scenario: {}: {e}", path.display());
            }
        });
    }

    async fn run(bridge: Arc<Bridge>, path: &Path) -> Result<(), String> {
        let raw = read_capped(path)?;
        let sc: ScenarioFile = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&sc.control).map_err(|e| e.to_string())?;
        if let Some(a) = &sc.air {
            std::fs::create_dir_all(a).map_err(|e| e.to_string())?;
        }
        let mut stubs = BTreeMap::new();
        let ScenarioFile {
            control,
            node,
            air,
            stubs: specs,
        } = sc;
        let sc_ref = ScenarioFile {
            control: control.clone(),
            node: node.clone(),
            air: air.clone(),
            stubs: Vec::new(),
        };
        for spec in specs {
            let s = build_stub(spec, &sc_ref)?;
            stubs.insert(s.id.to_string(), s);
        }
        // the bridge needs the fabric open; retry briefly while it starts
        for s in stubs.values() {
            let mut tries = 0;
            loop {
                match bridge.enable(Arc::clone(s) as Arc<dyn Adapter>).await {
                    Ok(()) => break,
                    Err(e) if tries < 100 => {
                        tries += 1;
                        if tries == 1 {
                            eprintln!("111 bridge scenario: enable {}: {e} (retrying)", s.id);
                        }
                        tokio::time::sleep(POLL).await;
                    }
                    Err(e) => return Err(format!("enable {}: {e}", s.id)),
                }
            }
        }
        let mut hook = Hook {
            bridge,
            stubs,
            control,
            node,
            air,
            offsets: BTreeMap::new(),
        };
        hook.init_air();
        let names: Vec<&str> = hook.stubs.keys().map(String::as_str).collect();
        eprintln!(
            "111 bridge scenario: node {} enabled {}",
            hook.node,
            names.join(", ")
        );
        append(
            &hook.control.join("ready.jsonl"),
            &json!({"node": hook.node, "stubs": names}),
        );
        loop {
            hook.control_pass().await;
            hook.air_pass();
            if hook.bridge.enabled().is_empty() {
                // the daemon is stopping (Layer6::shutdown disabled every adapter)
                return Ok(());
            }
            tokio::time::sleep(POLL).await;
        }
    }

    impl Hook {
        fn stub(&self, id: &str) -> Result<&Arc<ScenarioStub>, String> {
            self.stubs.get(id).ok_or_else(|| format!("no stub {id:?}"))
        }

        fn init_air(&mut self) {
            let Some(dir) = &self.air else { return };
            for p in self.air_files(dir) {
                let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                self.offsets.insert(p, len);
            }
        }

        fn air_files(&self, dir: &Path) -> Vec<PathBuf> {
            let own = format!("{}.jsonl", self.node);
            let Ok(rd) = std::fs::read_dir(dir) else {
                return Vec::new();
            };
            let mut v: Vec<PathBuf> = rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.extension().is_some_and(|x| x == "jsonl")
                        && p.file_name().is_some_and(|n| n.to_string_lossy() != own)
                })
                .collect();
            v.sort();
            v
        }

        /// Feed what the other nodes transmitted since the last pass.
        fn air_pass(&mut self) {
            let Some(dir) = self.air.clone() else { return };
            for p in self.air_files(&dir) {
                let from = *self.offsets.get(&p).unwrap_or(&0);
                let Ok(bytes) = std::fs::read(&p) else {
                    continue;
                };
                if (bytes.len() as u64) <= from {
                    continue;
                }
                let new = &bytes[from as usize..];
                // only whole lines; a partial last line waits for the next pass
                let Some(end) = new.iter().rposition(|b| *b == b'\n') else {
                    continue;
                };
                self.offsets.insert(p.clone(), from + end as u64 + 1);
                for line in new[..end].split(|b| *b == b'\n') {
                    let Ok(l) = serde_json::from_slice::<AirLine>(line) else {
                        continue;
                    };
                    let Some(s) = self.stubs.get(&l.conv.network) else {
                        continue;
                    };
                    let account = s
                        .inner
                        .facts()
                        .accounts
                        .first()
                        .map(|a| a.id.clone())
                        .unwrap_or_else(|| l.conv.account.clone());
                    let _ = &l.node;
                    s.inner.feed(Inbound::Text(InboundText {
                        conv: ConvRef {
                            network: l.conv.network.clone(),
                            account,
                            kind: l.conv.kind,
                            native_id: l.conv.native_id.clone(),
                        },
                        sender_native: l.conv.account.clone(),
                        sender_name: l.name,
                        auth: Auth::ChannelKey,
                        native_msg_id: l.native_id,
                        body: l.text,
                        claim_ts: None,
                        meta: json!({}),
                    }));
                }
            }
        }

        async fn control_pass(&mut self) {
            let Ok(rd) = std::fs::read_dir(&self.control) else {
                return;
            };
            let mut cmds: Vec<PathBuf> = rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect();
            cmds.sort();
            for p in cmds {
                let answer = match read_capped(&p)
                    .and_then(|b| serde_json::from_slice::<Command>(&b).map_err(|e| e.to_string()))
                {
                    Ok(c) => match self.apply(c).await {
                        Ok(v) => json!({"ok": true, "result": v}),
                        Err(e) => json!({"ok": false, "error": e}),
                    },
                    Err(e) => json!({"ok": false, "error": e}),
                };
                let done = p.with_extension("done");
                let tmp = p.with_extension("done.tmp");
                let w = std::fs::write(&tmp, answer.to_string())
                    .and_then(|()| std::fs::rename(&tmp, &done));
                if let Err(e) = w {
                    eprintln!("111 bridge scenario: {}: {e}", done.display());
                }
                let _ = std::fs::remove_file(&p);
            }
        }

        async fn apply(&self, c: Command) -> Result<Value, String> {
            match c {
                Command::Feed { stub, items } => {
                    let n = items.len();
                    self.stub(&stub)?.inner.feed_many(items);
                    Ok(json!({"fed": n}))
                }
                Command::Burst {
                    stub,
                    conv,
                    n,
                    sender_native,
                    sender_name,
                    prefix,
                } => {
                    if n > MAX_BURST {
                        return Err(format!("a burst is at most {MAX_BURST} texts"));
                    }
                    let s = self.stub(&stub)?;
                    s.inner.feed_many((1..=n).map(|i| {
                        Inbound::Text(InboundText {
                            conv: conv.clone(),
                            sender_native: sender_native.clone(),
                            sender_name: sender_name.clone(),
                            auth: Auth::ChannelKey,
                            native_msg_id: format!("{prefix}-{i}"),
                            body: format!("{prefix} {i:06}"),
                            claim_ts: None,
                            meta: json!({}),
                        })
                    }));
                    Ok(json!({"fed": n}))
                }
                Command::Script {
                    stub,
                    outcomes,
                    replace,
                } => {
                    let n = outcomes.len();
                    let s = self.stub(&stub)?;
                    let mut q = lock(&s.outcomes);
                    let dropped = if replace { q.drain(..).count() } else { 0 };
                    q.extend(outcomes);
                    Ok(json!({"scripted": n, "dropped": dropped}))
                }
                Command::Facts { stub, facts } => {
                    self.stub(&stub)?.inner.update_facts(|f| *f = facts);
                    Ok(json!({}))
                }
                Command::State { stub, state } => {
                    self.stub(&stub)?.inner.set_state(state);
                    Ok(json!({}))
                }
                Command::QueueFull { stub, on } => {
                    self.stub(&stub)?.queue_full.store(on, Ordering::SeqCst);
                    Ok(json!({}))
                }
                Command::EmitHeld { stub, index, event } => {
                    let ev = match event {
                        HeldEvent::Accepted { native_id } => SendEvent::Accepted { native_id },
                        HeldEvent::Evidence { state, evidence } => {
                            SendEvent::Evidence { state, evidence }
                        }
                        HeldEvent::Failed { reason, retry } => SendEvent::Failed { reason, retry },
                    };
                    let ok = self.stub(&stub)?.inner.emit_held(index, ev);
                    Ok(json!({"emitted": ok}))
                }
                Command::Pin { conv, pinned } => {
                    let f = self.bridge.fab().map_err(|e| e.to_string())?;
                    f.foreign_conversation_pin(conv, pinned)
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(json!({"pinned": pinned}))
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn commands_and_scenario_files_parse_closed() {
            let c: Command = serde_json::from_str(
                r#"{"op":"script","stub":"stub-mesh","outcomes":["hold",{"fail":{"reason":"region_unset","retry":"later"}},{"accept":{}}]}"#,
            )
            .unwrap();
            assert!(matches!(c, Command::Script { ref outcomes, .. } if outcomes.len() == 3));
            assert!(serde_json::from_str::<Command>(r#"{"op":"nope"}"#).is_err());
            assert!(serde_json::from_str::<Command>(
                r#"{"op":"queue_full","stub":"x","on":true,"extra":1}"#
            )
            .is_err());
            let f: Command = serde_json::from_str(
                r#"{"op":"feed","stub":"s","items":[{"Link":"up"},{"Conversation":{"conv":{"network":"s","account":"a","kind":"channel","native_id":"ch:x"},"title":"x"}}]}"#,
            )
            .unwrap();
            assert!(matches!(f, Command::Feed { ref items, .. } if items.len() == 2));
            let s: ScenarioFile = serde_json::from_str(
                r#"{"control":"/c","node":"a","stubs":[{"id":"stub-mesh","label":"M","tag":"SM","max_text_bytes":{"channel":232,"dm":220,"group":null}}]}"#,
            )
            .unwrap();
            assert_eq!(s.stubs[0].max_text_bytes.unwrap().channel, Some(232));
        }

        #[tokio::test]
        async fn a_scenario_stub_logs_sends_continues_native_ids_and_writes_the_air() {
            let dir = std::env::temp_dir().join(format!("g1-hook-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("air")).unwrap();
            let sc = ScenarioFile {
                control: dir.clone(),
                node: "a".into(),
                air: Some(dir.join("air")),
                stubs: Vec::new(),
            };
            let spec = || Spec {
                id: "stub-mesh".into(),
                label: "Mesh".into(),
                tag: "SM".into(),
                kinds: None,
                max_text_bytes: None,
                airtime: None,
                recipient_syntax: None,
                facts: None,
                inbound_cap: None,
                air_name: Some("]".into()),
            };
            let conv = ConvRef {
                network: "stub-mesh".into(),
                account: "!a".into(),
                kind: ConvKind::Channel,
                native_id: "ch:x".into(),
            };
            let s = build_stub(spec(), &sc).unwrap();
            let mut h = s.send(&conv, "one", SendOpts::default()).await;
            assert_eq!(
                h.events.recv().await,
                Some(SendEvent::Accepted {
                    native_id: "stub-mesh-a-1".into()
                })
            );
            s.queue_full.store(true, Ordering::SeqCst);
            let mut h = s.send(&conv, "two", SendOpts::default()).await;
            assert!(matches!(
                h.events.recv().await,
                Some(SendEvent::Failed {
                    reason: Reason::RateLimited,
                    ..
                })
            ));
            // a restart: the ids continue from the log
            let s2 = build_stub(spec(), &sc).unwrap();
            let mut h = s2.send(&conv, "three", SendOpts::default()).await;
            assert_eq!(
                h.events.recv().await,
                Some(SendEvent::Accepted {
                    native_id: "stub-mesh-a-3".into()
                })
            );
            let log = std::fs::read_to_string(dir.join("stub-mesh.sent.jsonl")).unwrap();
            assert_eq!(log.lines().count(), 3);
            let air = std::fs::read_to_string(dir.join("air/a.jsonl")).unwrap();
            assert_eq!(air.lines().count(), 2, "only accepted sends are on the air");
            let l: AirLine = serde_json::from_str(air.lines().next().unwrap()).unwrap();
            assert_eq!((l.name.as_str(), l.text.as_str()), ("]", "one"));
            let _ = std::fs::remove_dir_all(&dir);
        }
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
