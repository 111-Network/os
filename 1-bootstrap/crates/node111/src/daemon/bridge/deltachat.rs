//! The Delta Chat adapter and its Madmail relay (Layer 6 `6-transport/README.md` §7 whole,
//! §2.1, §4 "Delivery states per network", §15.2, §15.4; D26 #6). Built after the netns
//! spike passed: `plan/layers/6-transport/research/08-spike-deltachat.md` (S5) is the
//! as-run evidence, and its §6 (states), §7 (the stdio protocol) and §8 (residuals) are the
//! facts this file follows.
//!
//! **Shape (§7 "Shape, for every app peer").** One headless peer, `deltachat-rpc-server`
//! (the chatmail core, MPL-2.0; optional component `deltachat`), owns a dedicated **bridge
//! account** on this hub's own relay, Madmail (AGPL-3.0; optional component `madmail`).
//! Neither is part of the base: with the network off there is no process, task, listener
//! or open file of either (§2.1 rule 2). [`enable`] is the human's enable: Madmail first
//! (111 writes its configuration itself, never `madmail install` or `upgrade`), then the
//! core, then [`DeltaChatAdapter::attach`] on the core's piped stdio; the daemon hands the
//! adapter to `Bridge::enable` (the registration call is in [`enable`]'s doc).
//!
//! **The stdio protocol (spike §7) and the trust boundary (§15.9).** JSON-RPC 2.0, one
//! object per line both ways, positional params. Responses come **out of order** and are
//! matched by id ([`parse_response`] validates every line: size, the four keys, an id this
//! generation is waiting for; anything else is counted as noise, never interpreted).
//! `get_next_event_batch` long-polls; one is **always outstanding** and a second waits
//! behind it on the core's event lock, so a batch line the supervisor drops as oversize
//! (> 64 KiB) cannot stall the stream: lost lines are counted and the oldest polls are
//! replaced. `Info` events (~200 per session) are dropped at the reader; an unknown kind
//! or a malformed event is noise.
//!
//! **Every generation is checked (S4: the runner checks only at enable).** On each new
//! [`PipedIo`] generation the adapter runs its handshake: `get_system_info` (health:
//! `deltachat_core_version`) → the account (`add_account` if none) → `set_config` `bot=1`,
//! `mdns_enabled=1` (the bot flag turns read receipts off otherwise, spike row 11),
//! `displayname`, `delete_device_after` → `add_transport_from_qr` with a `dclogin:` from
//! Madmail's `create-user` (first enable only: `is_configured`) → the account's address →
//! `start_io` → `wait_for_all_work_done` (bounded, non-fatal) → the Secure-Join QR. A
//! respawn re-attaches the same way; requests in flight fail with `adapter_down` and the
//! core re-queues them (§7 "Supervisor extension").
//!
//! **Consent (§7 (2), §15.3).** The account-level warning ([`ACCOUNT_WARNING`], verbatim)
//! is part of the `deltachat` component's licence notice, so the component enable's `409
//! consent_required` round trip records it. Contacts join by **Secure-Join**
//! (`SecurejoinInviterProgress` 1000: the contact's own act, remembered in
//! `bridge/deltachat/joined.json`); bridging for that contact stays **off** until a human
//! turns it on (`Bridge::set_contact_bridging`, read live through [`OptIn`] per message).
//! Only a joined **and** switched-on contact's texts become foreign rows (kinds `dm` and
//! `group`, only groups can be relay endpoints); everything else is received and decrypted
//! by the core but never copied into Messages, shown or relayed ([`DeltaChatAdapter::unbridged`]
//! counts it). Retention: `delete_device_after` is account-wide (spike §7), set to
//! [`DELETE_DEVICE_AFTER_SECS`] (7 days); bridged texts are in Messages before that. The
//! residual (every contact's plaintext sits in the account store for up to 7 days, §15.2)
//! is named in the component notice.
//!
//! **States (§4; spike §6).** `misc_send_text_message` returns the msg id the adapter keys
//! by (the hidden Secure-Join messages' `MsgDelivered` are ignored): 20 ⇒ `queued`; 26
//! (`MsgDelivered`) ⇒ `Accepted` + `sent` "the relay accepted it" (the core has no separate
//! hand-off step, so `relayed` is never reported); 28 (`MsgRead`) ⇒ `delivered` "the
//! recipient's app sent a read receipt" (it may never come); 24 (`MsgFailed`) before 26 ⇒
//! `failed` (`got_nak`, pre-air). `Accepted` waits for 26 so that a refusal by the relay can
//! still read `failed`.
//!
//! **Receive.** `IncomingMsg` → `get_message` → the sender (`get_contact`'s address is the
//! native id, `displayName` the untrusted name) → `InboundText` with `auth = none` (the hub
//! terminates the app's end-to-end encryption; the banner says so). Relay copies carry the
//! marker as the visible prefix only (no structured field, §8). Own messages, info and
//! device messages are not texts.
//!
//! **Madmail ([`render_madmail_config`], [`MadmailCli`]).** 111 renders the relay's TOML:
//! IP-only, `tls_mode = "self_signed"`, listeners on the Layer 6 block 11240 SMTP, 11241
//! submission (TLS), 11242 IMAP (TLS), 11243 HTTPS ([`RelayPorts`]; ports ≥ 1024 need no
//! `CAP_NET_BIND_SERVICE`, which 111 does not request), bound to loopback, or to the LAN
//! address only while LAN sharing is on ([`relan`] re-renders); no plain IMAP, plain
//! submission or plain HTTP, no OpenMetrics, TURN off, no Shadowsocks or Iroh (the TOML
//! form has neither). The runtime switches are set with the relay's own CLI while it is
//! stopped: `push off` (no `notifications.delta.chat`), `federation policy reject` (nothing
//! leaves the hub), `registration open` only while LAN sharing is on. Health is the IMAP
//! listener. The state lives in `bridge/deltachat/madmail/` (removed by "remove files").
//!
//! **Egress (§15.10).** Nothing here contacts anything but loopback or the LAN address;
//! the component gate (V-wave) logs `connect()`.

use super::{
    AccountFacts, Adapter, AdapterFacts, AdapterState, Auth, Caps, ConvKind, ConvRef,
    ForeignMessageState as S, Honest, Inbound, InboundText, LinkState, PerKind, Reason, Retry,
    SendEvent, SendHandle, SendOpts, NETWORK_DELTACHAT,
};
use crate::daemon::components::{Components, EnableError};
use crate::daemon::fabric::json as fjson;
use crate::daemon::supervise::{PipedIo, PIPED_LINE_MAX};
use crate::util::run::Runner;
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch, Notify};
use tokio::task::JoinHandle;

// ---------------------------------------------------------------- facts and copy

/// The network id (§3).
pub const NETWORK: &str = NETWORK_DELTACHAT;
/// The optional component that runs the core (`assets/components/deltachat.toml`).
pub const RPC_COMPONENT: &str = "deltachat";
/// The optional component that runs the relay (`assets/components/madmail.toml`).
pub const RELAY_COMPONENT: &str = "madmail";
/// The account-level warning of §7 (2), verbatim: shown once, as part of the `deltachat`
/// component's licence notice (the enable's consent text).
pub const ACCOUNT_WARNING: &str = "This hub decrypts everything this account receives, \
including messages from contacts you do not bridge. The plaintext exists on this hub.";
/// The link-out card (§7 "The native UI, offered separately").
pub const CARD_TITLE: &str = "Open Delta Chat on your phone or desktop";
pub const CARD_NOTE: &str = "this is the native app; Messages shows the same conversations as text";
/// What the card says about joining (Secure-Join is the contact's act; the switch is yours).
pub const CARD_JOIN: &str = "A contact who scans this code joins by Secure-Join. Bridging \
stays off for that contact until you turn it on.";
/// The bridge account's display name unless the daemon passes another.
pub const DEFAULT_DISPLAYNAME: &str = "111 bridge";
/// `delete_device_after` (account-wide, spike §7): 7 days. Bridged texts are copied into
/// Messages on receipt, so this bounds only the core's own copy (§15.2 residual).
pub const DELETE_DEVICE_AFTER_SECS: u64 = 7 * 24 * 60 * 60;
/// Largest text sent or relayed (bytes): the JSON-escaped request must fit one 64 KiB
/// protocol line with room to spare; Delta Chat itself states no smaller limit.
pub const MAX_TEXT_BYTES: usize = 16 * 1024;
/// §4 evidence strings (spike §6).
pub const EVIDENCE_QUEUED: &str = "queued in this hub's Delta Chat core";
pub const EVIDENCE_SENT: &str = "the relay accepted it";
pub const EVIDENCE_DELIVERED: &str = "the recipient's app sent a read receipt";
/// The honest copy when the manifest is missing (the manifest wins, §4).
const BANNER: &str = "This hub decrypts everything its Delta Chat bridge account receives; \
the hub terminates Delta Chat's end-to-end encryption.";
const SEND_WARNING: &str = "111 sends from this hub's own Delta Chat bridge account, flagged \
as a bot. What you relay onto another network is readable there.";

/// Core `MessageState` values (spike §6, `deltachat-rpc-client` `const.py`).
const OUT_PREPARING: u64 = 18;
const OUT_DRAFT: u64 = 19;
const OUT_PENDING: u64 = 20;
const OUT_FAILED: u64 = 24;
const OUT_DELIVERED: u64 = 26;
const OUT_MDN_RCVD: u64 = 28;
/// Contact and chat ids up to this are the core's specials (self, device, info).
const LAST_SPECIAL_ID: u32 = 9;
/// Events looked at per batch (the core sends at most 101).
const MAX_BATCH: usize = 256;
/// Inbound items held for the bridge core; full ⇒ the oldest goes and is counted (§4).
const INBOUND_CAP: usize = 1024;
/// Sends tracked for evidence; full ⇒ the oldest is forgotten (its row keeps its state).
const MAX_TRACKED: usize = 4096;
/// Joined contacts remembered.
const MAX_JOINED: usize = 1024;
/// Conversation → chat id entries cached.
const MAX_CHATS: usize = 4096;
/// Event polls kept outstanding (module doc) and the most ever in flight.
const EVENT_POLLS: usize = 2;
const MAX_POLLS: usize = 8;
/// Longest address, QR text and `dclogin:` URI accepted.
const MAX_ADDR: usize = 254;
const MAX_QR: usize = 2048;
const MAX_DCLOGIN: usize = 4096;
/// Largest `joined.json` read back.
const MAX_JOINED_FILE: u64 = 256 * 1024;

/// Every event kind of the pinned core's JSON-RPC (`deltachat-jsonrpc` v2.63.0
/// `api/types/events.rs`): a kind outside this list is noise.
const KNOWN_KINDS: &[&str] = &[
    "AccountsBackgroundFetchDone",
    "AccountsChanged",
    "AccountsItemChanged",
    "CallEnded",
    "ChatDeleted",
    "ChatEphemeralTimerModified",
    "ChatModified",
    "ChatlistChanged",
    "ChatlistItemChanged",
    "ConfigSynced",
    "ConfigureProgress",
    "ConnectivityChanged",
    "ContactsChanged",
    "DeletedBlobFile",
    "Error",
    "ErrorSelfNotInGroup",
    "EventChannelOverflow",
    "ImapConnected",
    "ImapInboxIdle",
    "ImapMessageDeleted",
    "ImexFileWritten",
    "ImexProgress",
    "IncomingCall",
    "IncomingCallAccepted",
    "IncomingMsg",
    "IncomingMsgBunch",
    "IncomingReaction",
    "IncomingWebxdcNotify",
    "Info",
    "LocationChanged",
    "MsgDeleted",
    "MsgDelivered",
    "MsgFailed",
    "MsgRead",
    "MsgReadCountChanged",
    "MsgsChanged",
    "MsgsNoticed",
    "NewBlobFile",
    "OutgoingCallAccepted",
    "PinnedMessagesChanged",
    "ReactionsChanged",
    "SecurejoinInviterProgress",
    "SecurejoinJoinerProgress",
    "SelfavatarChanged",
    "SmtpConnected",
    "SmtpMessageSent",
    "TransportsModified",
    "Warning",
    "WebxdcInstanceDeleted",
    "WebxdcRealtimeAdvertisementReceived",
    "WebxdcRealtimeData",
    "WebxdcStatusUpdate",
];

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A string the core sent, for a log line or an error: control and format characters
/// dropped, cut to `max` bytes at a UTF-8 boundary.
fn plain(s: &str, max: usize) -> String {
    let c: String = s
        .chars()
        .filter(|c| !c.is_control() && !super::ingest::is_cf(*c))
        .collect();
    super::ingest::cut(&c, max).to_string()
}

/// An e-mail-shaped native id (`user@[192.168.1.5]`): at most 254 bytes, one `@` with both
/// sides non-empty, no whitespace, control, format, quote, angle or slash characters.
pub fn address_ok(s: &str) -> bool {
    let Some((local, domain)) = s.rsplit_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && s.len() <= MAX_ADDR
        && s.chars().all(|c| {
            !c.is_whitespace()
                && !c.is_control()
                && !super::ingest::is_cf(c)
                && !matches!(c, '/' | '\\' | '"' | '<' | '>' | '`')
        })
}

/// The host part of an address with the IP-literal brackets removed.
fn address_host(addr: &str) -> Option<&str> {
    let (_, d) = addr.rsplit_once('@')?;
    Some(d.trim_start_matches('[').trim_end_matches(']'))
}

fn u32_of(v: &Value) -> Option<u32> {
    v.as_u64().and_then(|n| u32::try_from(n).ok())
}

// ---------------------------------------------------------------- configuration

/// How the adapter runs (timeouts bound every call; none is a resident timer).
#[derive(Debug, Clone)]
pub struct DcConfig {
    /// The bridge account's display name (`set_config displayname`).
    pub displayname: String,
    /// `delete_device_after` in seconds (account-wide).
    pub delete_device_after_secs: u64,
    /// One ordinary call.
    pub call_timeout: Duration,
    /// `add_transport_from_qr` (the core configures against the relay).
    pub configure_timeout: Duration,
    /// `wait_for_all_work_done` (non-fatal when it runs out).
    pub settle_timeout: Duration,
    /// One request line written to the core.
    pub write_timeout: Duration,
    /// The relay host the account's address must carry (`127.0.0.1`, the LAN address);
    /// another host reads `relay_moved` (the relay's address changed with LAN sharing).
    pub relay_host: Option<String>,
    /// Where joined contacts are remembered (`bridge/deltachat/joined.json`; `None`: in
    /// memory only).
    pub joined_file: Option<PathBuf>,
}

impl Default for DcConfig {
    fn default() -> Self {
        DcConfig {
            displayname: DEFAULT_DISPLAYNAME.into(),
            delete_device_after_secs: DELETE_DEVICE_AFTER_SECS,
            call_timeout: Duration::from_secs(30),
            configure_timeout: Duration::from_secs(180),
            settle_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(5),
            relay_host: None,
            joined_file: None,
        }
    }
}

/// Where the bridge account comes from on first enable: the relay's `create-user`
/// ([`MadmailCli`]). Blocking.
pub trait RelayAccounts: Send + Sync {
    /// Create one account on the relay; its `dclogin:` URI (holds the password: never
    /// logged).
    fn create_user(&self) -> Result<String, String>;
}

/// The per-contact switch, read live per message: `(our account, contact address) ⇒
/// bridged`. Production reads `Bridge::contact_bridged` ([`opt_in_from`]).
pub type OptIn = Arc<dyn Fn(&str, &str) -> bool + Send + Sync>;

/// The production [`OptIn`]: the bridge settings' per-contact switch (off once the bridge
/// is gone).
pub fn opt_in_from(bridge: &Arc<super::Bridge>) -> OptIn {
    let weak = Arc::downgrade(bridge);
    Arc::new(move |account: &str, contact: &str| {
        weak.upgrade()
            .is_some_and(|b| b.contact_bridged(NETWORK, account, contact))
    })
}

// ---------------------------------------------------------------- the line protocol

/// Why a call did not answer with a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcError {
    /// The child is gone (or never took the line): `adapter_down`.
    Down,
    /// No answer within the call's timeout.
    Timeout,
    /// The core answered with an error (its message, cleaned and capped).
    Remote(String),
    /// The request could not be formed.
    Invalid(&'static str),
}

/// Validate one stdout line as a JSON-RPC 2.0 response (§15.9: size first, then shape;
/// nothing in it is interpreted): `(id, result or the error's message)`. `None` is noise.
pub fn parse_response(line: &str) -> Option<(u64, Result<Value, RpcError>)> {
    if line.len() > PIPED_LINE_MAX {
        return None;
    }
    let Value::Object(mut m) = serde_json::from_str::<Value>(line).ok()? else {
        return None;
    };
    if m.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || m.keys()
            .any(|k| !matches!(k.as_str(), "jsonrpc" | "id" | "result" | "error"))
    {
        return None;
    }
    let id = m.get("id")?.as_u64()?;
    match (m.remove("result"), m.remove("error")) {
        (Some(r), None) => Some((id, Ok(r))),
        (None, Some(Value::Object(e))) => {
            let msg = e
                .get("message")
                .and_then(Value::as_str)
                .map(|s| plain(s, 200))
                .unwrap_or_default();
            Some((id, Err(RpcError::Remote(msg))))
        }
        _ => None,
    }
}

/// One event of a batch (spike §7): the kinds the adapter acts on, the rest by class.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Event {
    Info,
    Incoming { msg: u32 },
    State { msg: u32, state: u64 },
    Joined { contact: u32, progress: u64 },
    Overflow { n: u64 },
    Error,
    Warning,
    Other,
}

/// `{contextId, event: {kind, …}}` ⇒ `(account, event)`; `None` is noise (a bad shape or a
/// kind the pinned core does not have).
fn parse_event(v: &Value) -> Option<(u32, Event)> {
    let o = v.as_object()?;
    if o.keys().any(|k| k != "contextId" && k != "event") {
        return None;
    }
    let ctx = u32_of(o.get("contextId")?)?;
    let e = o.get("event")?.as_object()?;
    let kind = e.get("kind")?.as_str()?;
    let field = |k: &str| e.get(k).and_then(u32_of);
    let ev = match kind {
        "Info" => Event::Info,
        "IncomingMsg" => {
            field("chatId")?;
            Event::Incoming {
                msg: field("msgId")?,
            }
        }
        "MsgDelivered" => Event::State {
            msg: field("msgId")?,
            state: OUT_DELIVERED,
        },
        "MsgRead" => Event::State {
            msg: field("msgId")?,
            state: OUT_MDN_RCVD,
        },
        "MsgFailed" => Event::State {
            msg: field("msgId")?,
            state: OUT_FAILED,
        },
        "SecurejoinInviterProgress" => Event::Joined {
            contact: field("contactId")?,
            progress: e.get("progress")?.as_u64()?,
        },
        "EventChannelOverflow" => Event::Overflow {
            n: e.get("n")?.as_u64()?,
        },
        "Error" => Event::Error,
        "Warning" => Event::Warning,
        k if KNOWN_KINDS.contains(&k) => Event::Other,
        _ => return None,
    };
    Some((ctx, ev))
}

/// A contact object's `(address, display name)`; the name is untrusted (the core's ingest
/// cleans and caps it), the address must be [`address_ok`].
fn parse_contact(v: &Value) -> Option<(String, String)> {
    let o = v.as_object()?;
    let addr = o.get("address")?.as_str()?;
    if !address_ok(addr) {
        return None;
    }
    let name = o
        .get("displayName")
        .and_then(Value::as_str)
        .or_else(|| o.get("authName").and_then(Value::as_str))
        .unwrap_or("");
    Some((addr.to_string(), super::ingest::cut(name, 4096).to_string()))
}

/// The conversation kind of a chat object (`chatType`): `Single` ⇒ dm, `Group` ⇒ group;
/// anything else (mailing lists, broadcasts, unknown) is not bridged (fail closed).
fn chat_kind(v: &Value) -> Option<ConvKind> {
    match v.get("chatType") {
        Some(Value::String(t)) if t == "Single" => Some(ConvKind::Dm),
        Some(Value::String(t)) if t == "Group" => Some(ConvKind::Group),
        Some(Value::Number(n)) if n.as_u64() == Some(100) => Some(ConvKind::Dm),
        Some(Value::Number(n)) if n.as_u64() == Some(120) => Some(ConvKind::Group),
        _ => None,
    }
}

/// One message object, the fields the adapter reads.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Msg {
    from: u32,
    chat: u32,
    text: String,
    is_info: bool,
    file_bytes: u64,
    timestamp: Option<i64>,
    sender: Option<(String, String)>,
}

fn parse_message(v: &Value) -> Option<Msg> {
    let o = v.as_object()?;
    Some(Msg {
        from: o.get("fromId").and_then(u32_of)?,
        chat: o.get("chatId").and_then(u32_of)?,
        text: o.get("text")?.as_str()?.to_string(),
        is_info: o.get("isInfo")?.as_bool()?,
        file_bytes: o.get("fileBytes").and_then(Value::as_u64).unwrap_or(0),
        timestamp: o.get("timestamp").and_then(Value::as_i64),
        sender: o.get("sender").and_then(parse_contact),
    })
}

/// The body of an inbound message: its text; an attachment is not ingested, and says so
/// (§3 "attachment not shown (N bytes)").
fn body_of(m: &Msg) -> Option<String> {
    match (m.text.is_empty(), m.file_bytes) {
        (true, 0) => None,
        (true, n) => Some(format!("attachment not shown ({n} bytes)")),
        (false, 0) => Some(m.text.clone()),
        (false, n) => Some(format!("{}\n\nattachment not shown ({n} bytes)", m.text)),
    }
}

/// A `dclogin:` URI as the relay's `create-user` prints it, validated (never logged).
fn dclogin_ok(s: &str) -> bool {
    s.starts_with("dclogin:") && s.len() <= MAX_DCLOGIN && s.bytes().all(|b| b.is_ascii_graphic())
}

/// The Secure-Join QR text (`https://i.delta.chat/#…`, or the older `OPENPGP4FPR:` form).
fn qr_ok(s: &str) -> bool {
    (s.starts_with("https://i.delta.chat/#") || s.starts_with("OPENPGP4FPR:"))
        && s.len() <= MAX_QR
        && s.bytes().all(|b| b.is_ascii_graphic())
}

// ---------------------------------------------------------------- one generation's RPC

/// One answer: the request id and its result (or the core's error).
type Answer = (u64, Result<Value, RpcError>);

/// What waits for an answer.
enum Waiter {
    Call(oneshot::Sender<Result<Value, RpcError>>),
    Poll,
}

/// The JSON-RPC client of one [`PipedIo`] generation: ids, waiters, the poll channel.
struct Rpc {
    io: PipedIo,
    next: AtomicU64,
    pending: Mutex<HashMap<u64, Waiter>>,
    /// Lines the supervisor dropped (oversize, not UTF-8, queue overflow) since the
    /// poller last looked: any could have been an answer.
    lost: AtomicU64,
    closed: AtomicBool,
    /// Answers to `get_next_event_batch` go to the poller; taken on close.
    polls: Mutex<Option<mpsc::Sender<Answer>>>,
    write_timeout: Duration,
}

impl Rpc {
    fn new(io: PipedIo, polls: mpsc::Sender<Answer>, write_timeout: Duration) -> Self {
        Rpc {
            io,
            next: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            lost: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            polls: Mutex::new(Some(polls)),
            write_timeout,
        }
    }

    /// Register `w` under a fresh id, then write the request; a refused or timed-out
    /// write is `Down` (`PipedIo::write_line` closed the child's stdin: it restarts).
    async fn request(&self, method: &str, params: Value, w: Waiter) -> Result<u64, RpcError> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let line =
            json!({"jsonrpc": "2.0", "method": method, "params": params, "id": id}).to_string();
        if line.len() > PIPED_LINE_MAX {
            return Err(RpcError::Invalid("the request is larger than one line"));
        }
        lock(&self.pending).insert(id, w);
        if self.closed.load(Ordering::SeqCst) {
            lock(&self.pending).remove(&id);
            return Err(RpcError::Down);
        }
        match tokio::time::timeout(self.write_timeout, self.io.write_line(&line)).await {
            Ok(Ok(())) => Ok(id),
            _ => {
                lock(&self.pending).remove(&id);
                Err(RpcError::Down)
            }
        }
    }

    async fn call(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, RpcError> {
        let (tx, rx) = oneshot::channel();
        let id = self.request(method, params, Waiter::Call(tx)).await?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(RpcError::Down),
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err(RpcError::Timeout)
            }
        }
    }

    async fn poll(&self) -> Result<u64, RpcError> {
        self.request("get_next_event_batch", json!([]), Waiter::Poll)
            .await
    }

    fn forget(&self, id: u64) {
        lock(&self.pending).remove(&id);
    }

    /// The generation ended: every waiter fails `Down`, the poller's channel closes.
    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        lock(&self.pending).clear();
        lock(&self.polls).take();
    }

    /// Route one answer (the reader).
    fn answer(&self, id: u64, result: Result<Value, RpcError>) -> bool {
        let w = lock(&self.pending).remove(&id);
        match w {
            Some(Waiter::Call(tx)) => {
                let _ = tx.send(result);
                true
            }
            Some(Waiter::Poll) => lock(&self.polls)
                .as_ref()
                .is_some_and(|tx| tx.try_send((id, result)).is_ok()),
            None => false,
        }
    }
}

fn dropped_lines(io: &PipedIo) -> u64 {
    io.dropped_invalid().saturating_add(io.dropped_overflow())
}

// ---------------------------------------------------------------- adapter state

/// What the adapter knows about the attached core.
#[derive(Debug, Clone)]
struct Live {
    generation: Option<u64>,
    rpc: Option<Arc<Rpc>>,
    account: Option<u32>,
    addr: Option<String>,
    qr: Option<String>,
    version: Option<String>,
    state: AdapterState,
    relay_host: Option<String>,
}

impl std::fmt::Debug for Rpc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rpc")
            .field("generation", &self.io.generation())
            .finish_non_exhaustive()
    }
}

/// One send whose evidence is still awaited, keyed by the core's msg id.
struct Tracker {
    tx: mpsc::Sender<SendEvent>,
    accepted: bool,
    /// 0 queued, 1 sent, 2 delivered.
    rank: u8,
}

#[derive(Default)]
struct Sends {
    map: HashMap<u32, Tracker>,
    order: VecDeque<u32>,
}

#[derive(Default)]
struct InQueue {
    items: VecDeque<Inbound>,
    dropped: u64,
}

#[derive(Default)]
struct Chats {
    /// Conversation native id → the core's chat id.
    ids: HashMap<String, u32>,
    /// Conversations announced to the core this generation.
    announced: HashSet<String>,
}

/// A contact that joined by Secure-Join (`SecurejoinInviterProgress` 1000): the contact's
/// own act. Only joined contacts can be bridged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinedContact {
    /// The native id (the address).
    pub address: String,
    /// Self-declared; untrusted, text only.
    pub name: String,
    /// The core's contact id (account-local).
    pub contact_id: u32,
}

/// A joined contact as the UI lists it, with the per-contact switch read live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContactView {
    pub address: String,
    pub name: String,
    pub bridged: bool,
}

/// The link-out card (§7 "The native UI"): the native app is offered separately, with the
/// bridge account's Secure-Join QR as text and as the contract's `QrMatrix` (no image is
/// served; the core's SVG variant is deprecated upstream).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LinkOutCard {
    pub title: &'static str,
    pub note: &'static str,
    pub join: &'static str,
    pub warning: &'static str,
    /// The bridge account's address.
    pub account: String,
    pub qr_text: String,
    pub qr: Value,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JoinedFile {
    schema: u32,
    contacts: Vec<JoinedContact>,
}

struct Inner {
    cfg: DcConfig,
    relay: Arc<dyn RelayAccounts>,
    optin: OptIn,
    live: Mutex<Live>,
    inbound: Mutex<InQueue>,
    notify: Notify,
    sends: Mutex<Sends>,
    joined: Mutex<Vec<JoinedContact>>,
    chats: Mutex<Chats>,
    noise: AtomicU64,
    unbridged: AtomicU64,
    warnings: AtomicU64,
    on_change: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// The driver and the current generation's tasks (aborted on shutdown).
    tasks: Mutex<Vec<JoinHandle<()>>>,
    gen_tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl Inner {
    fn push(&self, item: Inbound) {
        {
            let mut q = lock(&self.inbound);
            if q.items.len() >= INBOUND_CAP {
                if let Some(Inbound::Text(_)) = q.items.pop_front() {
                    q.dropped += 1;
                }
            }
            q.items.push_back(item);
        }
        self.notify.notify_waiters();
    }

    fn pop(&self) -> Option<Inbound> {
        let mut q = lock(&self.inbound);
        if q.dropped > 0 {
            let n = std::mem::take(&mut q.dropped);
            return Some(Inbound::Dropped { n });
        }
        q.items.pop_front()
    }

    fn noise(&self, n: u64) {
        self.noise.fetch_add(n, Ordering::SeqCst);
    }

    fn changed(&self) {
        let f = lock(&self.on_change).clone();
        if let Some(f) = f {
            f();
        }
    }

    fn set_state(&self, s: AdapterState) {
        let changed = {
            let mut l = lock(&self.live);
            let c = l.state != s;
            l.state = s;
            c
        };
        if changed {
            self.changed();
        }
    }

    fn fail(&self, code: &str, why: &str) {
        eprintln!("111 bridge: {NETWORK}: {why}");
        self.set_state(AdapterState::Error(code.to_string()));
    }

    /// The attached generation's client, account and address, when live.
    fn session(&self) -> Option<(Arc<Rpc>, u32, String)> {
        let l = lock(&self.live);
        if l.state != AdapterState::Live {
            return None;
        }
        Some((Arc::clone(l.rpc.as_ref()?), l.account?, l.addr.clone()?))
    }

    fn addr(&self) -> Option<String> {
        lock(&self.live).addr.clone()
    }

    fn joined(&self, addr: &str) -> Option<JoinedContact> {
        lock(&self.joined)
            .iter()
            .find(|c| c.address == addr)
            .cloned()
    }

    /// Bridged = joined by Secure-Join **and** switched on by a human (module doc).
    fn bridged(&self, account: &str, addr: &str) -> bool {
        self.joined(addr).is_some() && (self.optin)(account, addr)
    }

    fn remember_joined(&self, c: JoinedContact) {
        let snapshot = {
            let mut list = lock(&self.joined);
            list.retain(|o| o.address != c.address);
            if list.len() >= MAX_JOINED {
                list.remove(0);
            }
            list.push(c);
            list.clone()
        };
        if let Some(path) = &self.cfg.joined_file {
            let file = JoinedFile {
                schema: 1,
                contacts: snapshot,
            };
            let written = serde_json::to_vec_pretty(&file)
                .map_err(anyhow::Error::from)
                .and_then(|b| crate::util::fs::atomic_write(path, &b, 0o600));
            if let Err(e) = written {
                eprintln!("111 bridge: {NETWORK}: joined contacts not saved: {e:#}");
            }
        }
        self.changed();
    }

    fn track(&self, msg: u32, tx: mpsc::Sender<SendEvent>) {
        let mut s = lock(&self.sends);
        if s.order.len() >= 2 * MAX_TRACKED {
            // ids of finished sends: compact, so the order stays bounded too
            let Sends { map, order } = &mut *s;
            order.retain(|id| map.contains_key(id));
        }
        if s.map.len() >= MAX_TRACKED {
            while let Some(old) = s.order.pop_front() {
                if s.map.remove(&old).is_some() {
                    break;
                }
            }
        }
        s.map.insert(
            msg,
            Tracker {
                tx,
                accepted: false,
                rank: 0,
            },
        );
        s.order.push_back(msg);
    }

    /// Map one core state of a tracked send onto §4 events (module doc, monotone).
    fn apply_state(&self, msg: u32, state: u64) {
        let mut s = lock(&self.sends);
        let Some(t) = s.map.get_mut(&msg) else {
            return;
        };
        let mut events = Vec::new();
        let mut done = false;
        match state {
            OUT_DELIVERED if t.rank < 1 => {
                if !t.accepted {
                    t.accepted = true;
                    events.push(SendEvent::Accepted {
                        native_id: msg.to_string(),
                    });
                }
                events.push(SendEvent::Evidence {
                    state: S::Sent,
                    evidence: EVIDENCE_SENT.into(),
                });
                t.rank = 1;
            }
            OUT_MDN_RCVD if t.rank < 2 => {
                if !t.accepted {
                    t.accepted = true;
                    events.push(SendEvent::Accepted {
                        native_id: msg.to_string(),
                    });
                }
                events.push(SendEvent::Evidence {
                    state: S::Delivered,
                    evidence: EVIDENCE_DELIVERED.into(),
                });
                t.rank = 2;
                done = true;
            }
            OUT_FAILED if t.rank < 2 => {
                // before 26: refused by the relay or the core (pre-air); after: the core
                // keeps `sent` with "transmitted, not confirmed"
                events.push(SendEvent::Failed {
                    reason: Reason::GotNak,
                    retry: Retry::Never,
                });
                done = true;
            }
            OUT_PREPARING | OUT_DRAFT | OUT_PENDING => {}
            _ => {}
        }
        let mut gone = false;
        for e in events {
            if t.tx.try_send(e).is_err() {
                gone = true;
            }
        }
        if done || gone {
            s.map.remove(&msg);
        }
    }

    fn chat_of(&self, native: &str) -> Option<u32> {
        lock(&self.chats).ids.get(native).copied()
    }

    fn remember_chat(&self, native: &str, chat: u32) -> bool {
        let mut c = lock(&self.chats);
        if c.ids.len() >= MAX_CHATS {
            c.ids.clear();
            c.announced.clear();
        }
        c.ids.insert(native.to_string(), chat);
        c.announced.insert(native.to_string())
    }

    fn shutdown(&self) {
        for t in lock(&self.tasks).drain(..) {
            t.abort();
        }
        for t in lock(&self.gen_tasks).drain(..) {
            t.abort();
        }
        if let Some(rpc) = lock(&self.live).rpc.take() {
            rpc.close();
        }
    }
}

/// Read `joined.json` (an absent, unreadable or invalid file reads as nobody: fail closed).
fn load_joined(path: &Path) -> Vec<JoinedContact> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Vec::new();
    };
    if !meta.file_type().is_file() || meta.len() > MAX_JOINED_FILE {
        eprintln!("111 bridge: {NETWORK}: {} ignored", path.display());
        return Vec::new();
    }
    let parsed = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<JoinedFile>(&b).ok())
        .filter(|f| f.schema == 1);
    match parsed {
        Some(f) => f
            .contacts
            .into_iter()
            .filter(|c| address_ok(&c.address) && c.contact_id > LAST_SPECIAL_ID)
            .take(MAX_JOINED)
            .collect(),
        None => {
            eprintln!("111 bridge: {NETWORK}: {} invalid; ignored", path.display());
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------- the adapter

/// The Delta Chat adapter (module doc). Cheap to share; dropping the last handle stops
/// its tasks (the core keeps running until its component is disabled).
pub struct DeltaChatAdapter {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for DeltaChatAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let l = lock(&self.inner.live);
        f.debug_struct("DeltaChatAdapter")
            .field("generation", &l.generation)
            .field("state", &l.state)
            .finish_non_exhaustive()
    }
}

impl Drop for DeltaChatAdapter {
    fn drop(&mut self) {
        self.inner.shutdown();
    }
}

impl DeltaChatAdapter {
    /// Attach to the core's stdio watch (`Components::stdio("deltachat")`): one driver task
    /// that runs the handshake on every new generation. Call inside the runtime.
    pub fn attach(
        cfg: DcConfig,
        stdio: watch::Receiver<Option<PipedIo>>,
        relay: Arc<dyn RelayAccounts>,
        optin: OptIn,
    ) -> Arc<Self> {
        let joined = cfg
            .joined_file
            .as_deref()
            .map(load_joined)
            .unwrap_or_default();
        let relay_host = cfg.relay_host.clone();
        let inner = Arc::new(Inner {
            cfg,
            relay,
            optin,
            live: Mutex::new(Live {
                generation: None,
                rpc: None,
                account: None,
                addr: None,
                qr: None,
                version: None,
                state: AdapterState::Error("adapter_down".into()),
                relay_host,
            }),
            inbound: Mutex::new(InQueue::default()),
            notify: Notify::new(),
            sends: Mutex::new(Sends::default()),
            joined: Mutex::new(joined),
            chats: Mutex::new(Chats::default()),
            noise: AtomicU64::new(0),
            unbridged: AtomicU64::new(0),
            warnings: AtomicU64::new(0),
            on_change: Mutex::new(None),
            tasks: Mutex::new(Vec::new()),
            gen_tasks: Mutex::new(Vec::new()),
        });
        let driver = tokio::spawn(drive(Arc::clone(&inner), stdio));
        lock(&inner.tasks).push(driver);
        Arc::new(DeltaChatAdapter { inner })
    }

    /// Called whenever the card, the joined contacts or the state change (the daemon
    /// publishes a `bridge` SSE frame).
    pub fn set_on_change(&self, f: Arc<dyn Fn() + Send + Sync>) {
        *lock(&self.inner.on_change) = Some(f);
    }

    /// The live state (`Live` once the handshake passed).
    pub fn state(&self) -> AdapterState {
        lock(&self.inner.live).state.clone()
    }

    /// The attached generation (`PipedIo::generation`).
    pub fn generation(&self) -> Option<u64> {
        lock(&self.inner.live).generation
    }

    /// The bridge account's address, once configured.
    pub fn account(&self) -> Option<String> {
        self.inner.addr()
    }

    /// The core's version (`get_system_info`), once attached.
    pub fn core_version(&self) -> Option<String> {
        lock(&self.inner.live).version.clone()
    }

    /// The link-out card with the Secure-Join QR (`None` unless live: a stale code or a
    /// moved account is never offered).
    pub fn card(&self) -> Option<LinkOutCard> {
        let (addr, qr) = {
            let l = lock(&self.inner.live);
            if l.state != AdapterState::Live {
                return None;
            }
            (l.addr.clone()?, l.qr.clone()?)
        };
        let matrix = fjson::qr(&qr).ok()?;
        Some(LinkOutCard {
            title: CARD_TITLE,
            note: CARD_NOTE,
            join: CARD_JOIN,
            warning: ACCOUNT_WARNING,
            account: addr,
            qr_text: qr,
            qr: matrix,
        })
    }

    /// Contacts that joined by Secure-Join, with the per-contact switch read live.
    pub fn contacts(&self) -> Vec<ContactView> {
        let account = self.inner.addr().unwrap_or_default();
        // copied out first: the switch is read without holding the adapter's lock
        let joined = lock(&self.inner.joined).clone();
        joined
            .iter()
            .map(|c| ContactView {
                address: c.address.clone(),
                name: c.name.clone(),
                bridged: (self.inner.optin)(&account, &c.address),
            })
            .collect()
    }

    /// Lines and events refused as malformed, oversize or of an unknown kind (§15.9).
    pub fn noise(&self) -> u64 {
        self.inner.noise.load(Ordering::SeqCst)
    }

    /// Texts received from contacts that are not bridged (never copied, shown or relayed).
    pub fn unbridged(&self) -> u64 {
        self.inner.unbridged.load(Ordering::SeqCst)
    }

    /// `Warning` and `Error` events the core reported (the IMAP `UIDNEXT` warning is one,
    /// spike §8.6); counted, never logged.
    pub fn warnings(&self) -> u64 {
        self.inner.warnings.load(Ordering::SeqCst)
    }

    /// The relay's host changed (LAN sharing, [`relan`]): an account on another host reads
    /// `relay_moved` from the next handshake on.
    pub fn set_relay_host(&self, host: Option<String>) {
        let moved = {
            let mut l = lock(&self.inner.live);
            l.relay_host = host.clone();
            matches!((&host, l.addr.as_deref().and_then(address_host)), (Some(h), Some(a)) if h != a)
        };
        if moved {
            self.inner
                .set_state(AdapterState::Error("relay_moved".into()));
        }
    }

    /// Mark messages seen (the hub then sends read receipts, `mdns_enabled=1`). The
    /// adapter never does this on its own: copying a text into Messages is not reading it.
    pub async fn mark_seen(&self, msg_ids: &[u32]) -> Result<(), RpcError> {
        let (rpc, acct, _) = self.inner.session().ok_or(RpcError::Down)?;
        let ids: Vec<u32> = msg_ids.iter().copied().take(256).collect();
        rpc.call(
            "markseen_msgs",
            json!([acct, ids]),
            self.inner.cfg.call_timeout,
        )
        .await
        .map(|_| ())
    }
}

impl Adapter for DeltaChatAdapter {
    fn id(&self) -> &'static str {
        NETWORK
    }

    fn caps(&self) -> Caps {
        Caps {
            kinds: vec![ConvKind::Dm, ConvKind::Group],
            max_text_bytes: PerKind {
                channel: None,
                dm: Some(MAX_TEXT_BYTES),
                group: Some(MAX_TEXT_BYTES),
            },
            evidence: vec![S::Queued, S::Sent, S::Delivered],
            airtime: None,
            auth_levels: vec![Auth::None],
            copy: Honest {
                label: "Delta Chat".into(),
                chip: "DC".into(),
                tag: "DC".into(),
                banner: BANNER.into(),
                send_warning: SEND_WARNING.into(),
            },
            recipient_syntax: None,
            splittable: false,
            state: self.state(),
        }
    }

    fn facts(&self) -> AdapterFacts {
        AdapterFacts {
            accounts: self
                .inner
                .addr()
                .map(|id| AccountFacts {
                    id,
                    label: Some(self.inner.cfg.displayname.clone()),
                    licensed: false,
                })
                .into_iter()
                .collect(),
            ids_globally_unique: true,
            ..AdapterFacts::default()
        }
    }

    fn recv(&self) -> BoxFuture<'_, Inbound> {
        Box::pin(async move {
            loop {
                let notified = self.inner.notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if let Some(item) = self.inner.pop() {
                    return item;
                }
                notified.await;
            }
        })
    }

    fn send(&self, conv: &ConvRef, text: &str, _o: SendOpts) -> BoxFuture<'_, SendHandle> {
        let (tx, rx) = mpsc::channel(8);
        let inner = Arc::clone(&self.inner);
        let conv = conv.clone();
        let text = text.to_string();
        Box::pin(async move {
            tokio::spawn(send_task(inner, conv, text, tx));
            SendHandle { events: rx }
        })
    }
}

// ---------------------------------------------------------------- the driver

/// Attach to every new generation the watch publishes (module doc).
async fn drive(inner: Arc<Inner>, mut stdio: watch::Receiver<Option<PipedIo>>) {
    let mut last: Option<u64> = None;
    loop {
        let io = stdio.borrow_and_update().clone();
        if let Some(io) = io {
            if last != Some(io.generation()) && !io.is_closed() {
                last = Some(io.generation());
                run_generation(&inner, io).await;
                continue;
            }
        }
        if stdio.changed().await.is_err() {
            return;
        }
    }
}

/// One generation: the reader, the poller and the handshake; it ends when the child's
/// stdout closes (the child exited or was stopped).
async fn run_generation(inner: &Arc<Inner>, io: PipedIo) {
    let generation = io.generation();
    let (ptx, prx) = mpsc::channel(MAX_POLLS + 2);
    let rpc = Arc::new(Rpc::new(io, ptx, inner.cfg.write_timeout));
    {
        let mut l = lock(&inner.live);
        l.generation = Some(generation);
        l.rpc = Some(Arc::clone(&rpc));
        l.state = AdapterState::Error("adapter_down".into());
    }
    {
        let mut c = lock(&inner.chats);
        c.announced.clear();
    }
    let reader = tokio::spawn(read_loop(Arc::clone(inner), Arc::clone(&rpc)));
    let poller = tokio::spawn(poll_loop(Arc::clone(inner), Arc::clone(&rpc), prx));
    let session = tokio::spawn(handshake(Arc::clone(inner), Arc::clone(&rpc)));
    {
        let mut g = lock(&inner.gen_tasks);
        g.retain(|t| !t.is_finished());
        g.push(poller);
        g.push(session);
    }
    let _ = reader.await;
    rpc.close();
    for t in lock(&inner.gen_tasks).drain(..) {
        t.abort();
    }
    let was_live = {
        let mut l = lock(&inner.live);
        let was = l.state == AdapterState::Live;
        if l.generation == Some(generation) {
            l.rpc = None;
            l.state = AdapterState::Error("adapter_down".into());
        }
        was
    };
    if was_live {
        inner.push(Inbound::Link(LinkState::Down));
    }
    inner.changed();
}

/// Every stdout line of the generation, validated and routed by id (§15.9).
async fn read_loop(inner: Arc<Inner>, rpc: Arc<Rpc>) {
    let mut seen = dropped_lines(&rpc.io);
    while let Some(line) = rpc.io.recv_line().await {
        let now = dropped_lines(&rpc.io);
        if now > seen {
            inner.noise(now - seen);
            rpc.lost.fetch_add(now - seen, Ordering::SeqCst);
            seen = now;
        }
        match parse_response(&line) {
            Some((id, result)) => {
                if !rpc.answer(id, result) {
                    inner.noise(1);
                }
            }
            None => inner.noise(1),
        }
    }
    rpc.close();
}

/// Keep [`EVENT_POLLS`] polls outstanding and handle each batch in order (module doc).
async fn poll_loop(inner: Arc<Inner>, rpc: Arc<Rpc>, mut rx: mpsc::Receiver<Answer>) {
    // (poll id, suspected lost)
    let mut out: VecDeque<(u64, bool)> = VecDeque::new();
    loop {
        while out.len() >= MAX_POLLS {
            match out.iter().position(|(_, suspect)| *suspect) {
                Some(i) => {
                    if let Some((id, _)) = out.remove(i) {
                        rpc.forget(id);
                    }
                }
                None => break,
            }
        }
        while out.iter().filter(|(_, s)| !*s).count() < EVENT_POLLS && out.len() < MAX_POLLS {
            match rpc.poll().await {
                Ok(id) => out.push_back((id, false)),
                Err(_) => return,
            }
        }
        let Some((id, result)) = rx.recv().await else {
            return;
        };
        out.retain(|(i, _)| *i != id);
        let lost = rpc.lost.swap(0, Ordering::SeqCst);
        let lost = usize::try_from(lost).unwrap_or(usize::MAX);
        for o in out.iter_mut().filter(|(_, s)| !*s).take(lost) {
            o.1 = true;
        }
        match result {
            Ok(Value::Array(events)) if !events.is_empty() => {
                handle_batch(&inner, &rpc, events).await;
            }
            // an empty batch: the core's event channel is closed; an error: it refuses
            // to deliver events. Either way this generation has no events: stop polling.
            Ok(Value::Array(_)) | Err(_) => {
                inner.noise(1);
                inner.fail("adapter_down", "the core stopped delivering events");
                return;
            }
            Ok(_) => inner.noise(1),
        }
    }
}

async fn handle_batch(inner: &Arc<Inner>, rpc: &Arc<Rpc>, events: Vec<Value>) {
    if events.len() > MAX_BATCH {
        inner.noise((events.len() - MAX_BATCH) as u64);
    }
    for v in events.into_iter().take(MAX_BATCH) {
        let Some((ctx, ev)) = parse_event(&v) else {
            inner.noise(1);
            continue;
        };
        let ours = lock(&inner.live).account.is_none_or(|a| a == ctx);
        if !ours {
            continue;
        }
        match ev {
            Event::Info | Event::Other => {}
            // counted, never logged: their text can carry what others sent (§15.1)
            Event::Warning | Event::Error => {
                inner.warnings.fetch_add(1, Ordering::SeqCst);
            }
            Event::Overflow { n } => inner.push(Inbound::Dropped { n }),
            Event::State { msg, state } => inner.apply_state(msg, state),
            Event::Joined { contact, progress } => {
                if progress == 1000 && contact > LAST_SPECIAL_ID {
                    on_joined(inner, rpc, ctx, contact).await;
                }
            }
            Event::Incoming { msg } => on_incoming(inner, rpc, ctx, msg).await,
        }
    }
}

/// A contact finished Secure-Join with us: remembered as joined (bridging stays off).
async fn on_joined(inner: &Arc<Inner>, rpc: &Arc<Rpc>, acct: u32, contact: u32) {
    let t = inner.cfg.call_timeout;
    let Ok(v) = rpc.call("get_contact", json!([acct, contact]), t).await else {
        inner.noise(1);
        return;
    };
    let Some((address, name)) = parse_contact(&v) else {
        inner.noise(1);
        return;
    };
    inner.remember_joined(JoinedContact {
        address,
        name: super::ingest::cut(&name, 256).to_string(),
        contact_id: contact,
    });
}

/// One incoming message (module doc "Receive"): copied only for a bridged contact.
async fn on_incoming(inner: &Arc<Inner>, rpc: &Arc<Rpc>, acct: u32, msg_id: u32) {
    let t = inner.cfg.call_timeout;
    let m = match rpc.call("get_message", json!([acct, msg_id]), t).await {
        Ok(v) => match parse_message(&v) {
            Some(m) => m,
            None => return inner.noise(1),
        },
        // gone before it was read
        Err(RpcError::Remote(_)) | Err(RpcError::Invalid(_)) => return inner.noise(1),
        // the answer did not come (dropped as oversize, or the core is gone): one text
        // this hub cannot show, counted, never silent (§4)
        Err(RpcError::Timeout) | Err(RpcError::Down) => {
            return inner.push(Inbound::Dropped { n: 1 });
        }
    };
    if m.is_info || m.from <= LAST_SPECIAL_ID || m.chat <= LAST_SPECIAL_ID {
        return;
    }
    let (addr, name) = match m.sender.clone() {
        Some(s) => s,
        None => match rpc.call("get_contact", json!([acct, m.from]), t).await {
            Ok(v) => match parse_contact(&v) {
                Some(s) => s,
                None => return inner.noise(1),
            },
            Err(_) => return inner.noise(1),
        },
    };
    let Some(account) = inner.addr() else {
        // not configured yet: nothing can be bridged (fail closed)
        inner.unbridged.fetch_add(1, Ordering::SeqCst);
        return;
    };
    if addr == account || !inner.bridged(&account, &addr) {
        inner.unbridged.fetch_add(1, Ordering::SeqCst);
        return;
    }
    let Some(body) = body_of(&m) else {
        return;
    };
    let chat = match rpc
        .call("get_basic_chat_info", json!([acct, m.chat]), t)
        .await
    {
        Ok(v) => v,
        Err(_) => return inner.noise(1),
    };
    let (kind, native, title) = match chat_kind(&chat) {
        Some(ConvKind::Dm) => (ConvKind::Dm, format!("dm:{addr}"), name.clone()),
        Some(ConvKind::Group) => (
            ConvKind::Group,
            format!("group:{}", m.chat),
            chat.get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        ),
        _ => {
            inner.unbridged.fetch_add(1, Ordering::SeqCst);
            return;
        }
    };
    let conv = ConvRef {
        network: NETWORK.to_string(),
        account,
        kind,
        native_id: native.clone(),
    };
    if inner.remember_chat(&native, m.chat) {
        inner.push(Inbound::Conversation {
            conv: conv.clone(),
            title: super::ingest::cut(&title, 4096).to_string(),
        });
    }
    inner.push(Inbound::Text(InboundText {
        conv,
        sender_native: addr,
        sender_name: name,
        auth: Auth::None,
        native_msg_id: msg_id.to_string(),
        body,
        claim_ts: m.timestamp,
        meta: Value::Object(Map::new()),
    }));
}

/// The handshake of one generation (module doc, "Every generation is checked").
async fn handshake(inner: Arc<Inner>, rpc: Arc<Rpc>) {
    let t = inner.cfg.call_timeout;
    let call = |method: &'static str, params: Value| {
        let rpc = Arc::clone(&rpc);
        async move { rpc.call(method, params, t).await }
    };
    // 1. health
    let version = match call("get_system_info", json!([])).await {
        Ok(Value::Object(m)) => m
            .get("deltachat_core_version")
            .and_then(Value::as_str)
            .map(|v| plain(v, 64))
            .filter(|v| !v.is_empty()),
        _ => None,
    };
    let Some(version) = version else {
        return inner.fail("adapter_down", "the core did not answer get_system_info");
    };
    lock(&inner.live).version = Some(version);
    // 2. the bridge account (the lowest id; created once)
    let acct = match call("get_all_account_ids", json!([])).await {
        Ok(Value::Array(ids)) => {
            let mut ids: Vec<u32> = ids.iter().filter_map(u32_of).collect();
            ids.sort_unstable();
            ids.first().copied()
        }
        _ => return inner.fail("adapter_down", "the core did not list its accounts"),
    };
    let acct = match acct {
        Some(a) => a,
        None => match call("add_account", json!([]))
            .await
            .ok()
            .as_ref()
            .and_then(u32_of)
        {
            Some(a) => a,
            None => return inner.fail("adapter_down", "the core did not add the bridge account"),
        },
    };
    lock(&inner.live).account = Some(acct);
    // 3. the account's settings, in this order (bot first; mdns_enabled or a bot sends
    //    and requests no read receipts, spike row 11)
    let settings = [
        ("bot", "1".to_string()),
        ("mdns_enabled", "1".to_string()),
        ("displayname", inner.cfg.displayname.clone()),
        (
            "delete_device_after",
            inner.cfg.delete_device_after_secs.to_string(),
        ),
    ];
    for (key, value) in settings {
        if call("set_config", json!([acct, key, value])).await.is_err() {
            return inner.fail(
                "adapter_down",
                &format!("the core refused set_config {key}"),
            );
        }
    }
    // 4. the transport: the relay account on first enable only
    let configured = match call("is_configured", json!([acct])).await {
        Ok(Value::Bool(b)) => b,
        _ => return inner.fail("adapter_down", "the core did not answer is_configured"),
    };
    if !configured {
        let relay = Arc::clone(&inner.relay);
        let dclogin = tokio::task::spawn_blocking(move || relay.create_user())
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
        let dclogin = match dclogin {
            Ok(d) if dclogin_ok(&d) => d,
            Ok(_) => return inner.fail("relay_unreachable", "the relay printed no dclogin: URI"),
            Err(e) => {
                return inner.fail(
                    "relay_unreachable",
                    &format!(
                        "the relay did not create the bridge account: {}",
                        plain(&e, 200)
                    ),
                )
            }
        };
        let r = rpc
            .call(
                "add_transport_from_qr",
                json!([acct, dclogin]),
                inner.cfg.configure_timeout,
            )
            .await;
        if let Err(e) = r {
            let why = match e {
                RpcError::Remote(m) => m,
                other => format!("{other:?}"),
            };
            return inner.fail(
                "relay_unreachable",
                &format!("the core could not configure against the relay: {why}"),
            );
        }
    }
    // 5. the account's address (before IO, so the first fetched message can be bridged)
    let addr = match call("get_config", json!([acct, "configured_addr"])).await {
        Ok(Value::String(a)) if address_ok(&a) => a,
        _ => return inner.fail("adapter_down", "the bridge account has no address"),
    };
    let moved = {
        let mut l = lock(&inner.live);
        l.addr = Some(addr.clone());
        matches!((&l.relay_host, address_host(&addr)), (Some(h), Some(a)) if h != a)
    };
    // 6. IO, then let it settle (bounded; upstream marks the call test-only, so its
    //    timing out is not a failure)
    if call("start_io", json!([acct])).await.is_err() {
        return inner.fail("adapter_down", "the core did not start IO");
    }
    match rpc
        .call(
            "wait_for_all_work_done",
            json!([acct]),
            inner.cfg.settle_timeout,
        )
        .await
    {
        Ok(_) | Err(RpcError::Timeout) | Err(RpcError::Remote(_)) => {}
        Err(_) => return inner.fail("adapter_down", "the core went away while settling"),
    }
    // 7. the Secure-Join QR for the card
    let qr = match call("get_chat_securejoin_qr_code", json!([acct, Value::Null])).await {
        Ok(Value::String(q)) if qr_ok(&q) => q,
        _ => return inner.fail("adapter_down", "the core gave no Secure-Join QR"),
    };
    lock(&inner.live).qr = Some(qr);
    if moved {
        return inner.fail(
            "relay_moved",
            "the bridge account lives on another relay address (LAN sharing changed); contacts join again after remove files",
        );
    }
    inner.set_state(AdapterState::Live);
    inner.push(Inbound::Link(LinkState::Up));
}

// ---------------------------------------------------------------- sending

enum Resolve {
    Refused,
    Down,
}

fn down_or_refused(e: RpcError) -> Resolve {
    match e {
        RpcError::Remote(_) | RpcError::Invalid(_) => Resolve::Refused,
        RpcError::Down | RpcError::Timeout => Resolve::Down,
    }
}

/// The core's chat for a conversation: a group by its chat id (it must still be a group);
/// a DM only for a joined, bridged contact (its key contact's chat).
async fn resolve_chat(
    inner: &Inner,
    rpc: &Rpc,
    acct: u32,
    account: &str,
    conv: &ConvRef,
) -> Result<u32, Resolve> {
    let t = inner.cfg.call_timeout;
    match conv.kind {
        ConvKind::Group => {
            let id = conv
                .native_id
                .strip_prefix("group:")
                .and_then(|s| s.parse::<u32>().ok())
                .filter(|n| *n > LAST_SPECIAL_ID)
                .ok_or(Resolve::Refused)?;
            let info = rpc
                .call("get_basic_chat_info", json!([acct, id]), t)
                .await
                .map_err(down_or_refused)?;
            (chat_kind(&info) == Some(ConvKind::Group))
                .then_some(id)
                .ok_or(Resolve::Refused)
        }
        ConvKind::Dm => {
            let addr = conv.native_id.strip_prefix("dm:").ok_or(Resolve::Refused)?;
            if !inner.bridged(account, addr) {
                return Err(Resolve::Refused);
            }
            if let Some(chat) = inner.chat_of(&conv.native_id) {
                return Ok(chat);
            }
            let contact = inner.joined(addr).ok_or(Resolve::Refused)?.contact_id;
            let found = rpc
                .call("get_chat_id_by_contact_id", json!([acct, contact]), t)
                .await
                .map_err(down_or_refused)?;
            let chat = match found {
                Value::Null => u32_of(
                    &rpc.call("create_chat_by_contact_id", json!([acct, contact]), t)
                        .await
                        .map_err(down_or_refused)?,
                ),
                v => u32_of(&v),
            }
            .filter(|c| *c > LAST_SPECIAL_ID)
            .ok_or(Resolve::Refused)?;
            inner.remember_chat(&conv.native_id, chat);
            Ok(chat)
        }
        ConvKind::Channel => Err(Resolve::Refused),
    }
}

/// One send (§4: `send` returned at once; this task reports the events).
async fn send_task(inner: Arc<Inner>, conv: ConvRef, text: String, tx: mpsc::Sender<SendEvent>) {
    let fail = |reason, retry| {
        let _ = tx.try_send(SendEvent::Failed { reason, retry });
    };
    if text.len() > MAX_TEXT_BYTES {
        return fail(Reason::TooLarge, Retry::Never);
    }
    let Some((rpc, acct, account)) = inner.session() else {
        return fail(Reason::AdapterDown, Retry::Later);
    };
    if conv.network != NETWORK || conv.account != account {
        return fail(Reason::NotAuthorized, Retry::Never);
    }
    let chat = match resolve_chat(&inner, &rpc, acct, &account, &conv).await {
        Ok(c) => c,
        Err(Resolve::Refused) => return fail(Reason::NotAuthorized, Retry::Never),
        Err(Resolve::Down) => return fail(Reason::AdapterDown, Retry::Later),
    };
    let t = inner.cfg.call_timeout;
    let msg = match rpc
        .call("misc_send_text_message", json!([acct, chat, text]), t)
        .await
    {
        Ok(v) => match u32_of(&v).filter(|m| *m > LAST_SPECIAL_ID) {
            Some(m) => m,
            // an answer that is not a msg id: whether it was queued is unknown, so it is
            // never retried on its own
            None => return fail(Reason::AdapterDown, Retry::Never),
        },
        Err(RpcError::Remote(_)) | Err(RpcError::Invalid(_)) => {
            return fail(Reason::NotAuthorized, Retry::Never)
        }
        // §7 "Supervisor extension": requests in flight fail with adapter_down and re-queue
        Err(RpcError::Down) | Err(RpcError::Timeout) => {
            return fail(Reason::AdapterDown, Retry::Later)
        }
    };
    let _ = tx.try_send(SendEvent::Evidence {
        state: S::Queued,
        evidence: EVIDENCE_QUEUED.into(),
    });
    inner.track(msg, tx);
    // the state now (an event may have come before the id was tracked)
    if let Ok(v) = rpc.call("get_message", json!([acct, msg]), t).await {
        if let Some(state) = v.get("state").and_then(Value::as_u64) {
            inner.apply_state(msg, state);
        }
    }
}

// ---------------------------------------------------------------- the Madmail relay

/// The relay's listeners: the Layer 6 block beside meshmud 11230 and SimpleX 11232 (spike
/// §4). All ≥ 1024, so no `CAP_NET_BIND_SERVICE`; the standard chatmail ports only with
/// that capability, which 111 does not request (§18). Configurable; the component's
/// `[run] ports` claims and pre-binds this default block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayPorts {
    /// SMTP (MX, inbound delivery between accounts of this relay).
    pub smtp: u16,
    /// Submission with implicit TLS (the `dclogin:` `sp`).
    pub submission: u16,
    /// IMAP with implicit TLS (the `dclogin:` `ip`; the health check).
    pub imap: u16,
    /// HTTPS (`/new` registration for phones while LAN sharing is on).
    pub https: u16,
}

impl Default for RelayPorts {
    fn default() -> Self {
        RelayPorts {
            smtp: 11240,
            submission: 11241,
            imap: 11242,
            https: 11243,
        }
    }
}

impl RelayPorts {
    pub fn all(&self) -> [u16; 4] {
        [self.smtp, self.submission, self.imap, self.https]
    }
}

/// Where the relay listens: loopback (the bridge account only), or this hub's LAN address
/// while LAN sharing is on (phones join; addresses like `user@[192.168.1.5]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayBind {
    Loopback,
    Lan(Ipv4Addr),
}

impl RelayBind {
    pub fn ip(&self) -> Ipv4Addr {
        match self {
            RelayBind::Loopback => Ipv4Addr::LOCALHOST,
            RelayBind::Lan(ip) => *ip,
        }
    }

    /// The host the relay's addresses and `dclogin:` URIs carry.
    pub fn host(&self) -> String {
        self.ip().to_string()
    }

    pub fn is_lan(&self) -> bool {
        matches!(self, RelayBind::Lan(_))
    }
}

/// The relay's and the account store's paths under `bridge/deltachat/` (0700; removed by
/// "remove files" with the account store, §2.1 rule 5, §15.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayPaths {
    pub dir: PathBuf,
    /// The TOML 111 writes (0600).
    pub config: PathBuf,
    /// Madmail's state (`--state-dir`: its database, certificate, admin token).
    pub state: PathBuf,
    /// Madmail's runtime directory.
    pub runtime: PathBuf,
    /// The core's account store (`DC_ACCOUNTS_PATH`).
    pub accounts: PathBuf,
    /// Contacts that joined by Secure-Join.
    pub joined: PathBuf,
}

impl RelayPaths {
    pub fn of(root: &Path) -> Self {
        let dir = root.join("bridge/deltachat");
        RelayPaths {
            config: dir.join("madmail.toml"),
            state: dir.join("madmail"),
            runtime: dir.join("madmail-run"),
            accounts: dir.join("accounts"),
            joined: dir.join("joined.json"),
            dir,
        }
    }

    /// Create every directory 0700 (a link is refused, never followed).
    pub fn prepare(&self) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        for d in [&self.dir, &self.state, &self.runtime, &self.accounts] {
            match std::fs::symlink_metadata(d) {
                Ok(m) if m.file_type().is_dir() => {}
                Ok(_) => {
                    return Err(std::io::Error::other(format!(
                        "{} is a link or not a directory",
                        d.display()
                    )))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::create_dir_all(d)?;
                }
                Err(e) => return Err(e),
            }
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
}

/// The relay's TOML (`chatmail.toml` keys of Madmail v2.30.5, `chatmail-config` `parse.rs`):
/// only these keys, so everything the TOML form can switch on stays off.
#[derive(Serialize)]
struct MadmailToml {
    hostname: String,
    primary_domain: String,
    state_dir: String,
    runtime_dir: String,
    tls_mode: &'static str,
    debug: bool,
    auth_auto_create: bool,
    smtp_listen: String,
    submission_tls_listen: String,
    imap_tls_listen: String,
    http_tls_listen: String,
    turn_enable: bool,
}

/// Render the relay's configuration (module doc "Madmail"): IP-only, self-signed, the port
/// block on `bind`'s address, nothing else listening. Deterministic (golden-tested).
pub fn render_madmail_config(bind: RelayBind, ports: RelayPorts, paths: &RelayPaths) -> String {
    let host = bind.host();
    let at = |p: u16| format!("{host}:{p}");
    let t = MadmailToml {
        hostname: host.clone(),
        primary_domain: host.clone(),
        state_dir: paths.state.to_string_lossy().into_owned(),
        runtime_dir: paths.runtime.to_string_lossy().into_owned(),
        tls_mode: "self_signed",
        debug: false,
        auth_auto_create: false,
        smtp_listen: at(ports.smtp),
        submission_tls_listen: at(ports.submission),
        imap_tls_listen: at(ports.imap),
        http_tls_listen: at(ports.https),
        turn_enable: false,
    };
    let body = toml::to_string(&t).unwrap_or_default();
    let scope = if bind.is_lan() {
        "LAN-bound while LAN sharing is on"
    } else {
        "loopback only"
    };
    format!(
        "# Written by 111 (Layer 6 §7): the Madmail relay of the Delta Chat bridge, {scope}.\n\
         # Never `madmail install` or `upgrade`. Not here, so off: plain IMAP, plain submission,\n\
         # plain HTTP, OpenMetrics, TURN listeners, Shadowsocks, Iroh. Rewritten on every enable\n\
         # and LAN-sharing change; edits are lost.\n{body}"
    )
}

/// Write the relay's configuration (0600, atomically).
pub fn write_madmail_config(
    paths: &RelayPaths,
    bind: RelayBind,
    ports: RelayPorts,
) -> anyhow::Result<()> {
    paths.prepare()?;
    let text = render_madmail_config(bind, ports, paths);
    crate::util::fs::atomic_write(&paths.config, text.as_bytes(), 0o600)
}

/// The relay's own CLI, run one-shot against 111's configuration (blocking): the runtime
/// switches and `create-user`. Never `install`, `upgrade`, `update` or `push on`.
pub struct MadmailCli {
    program: PathBuf,
    paths: RelayPaths,
    runner: Arc<dyn Runner>,
}

impl MadmailCli {
    /// `program` is the installed file (`components/madmail/<[run] program>`).
    pub fn new(root: &Path, program: &str, runner: Arc<dyn Runner>) -> Self {
        MadmailCli {
            program: root.join("components").join(RELAY_COMPONENT).join(program),
            paths: RelayPaths::of(root),
            runner,
        }
    }

    fn run(&self, args: &[&str]) -> Result<String, String> {
        let config = self.paths.config.to_string_lossy().into_owned();
        let state = self.paths.state.to_string_lossy().into_owned();
        let mut all: Vec<&str> = vec!["--config", &config, "--state-dir", &state];
        all.extend_from_slice(args);
        let program = self.program.to_string_lossy().into_owned();
        let out = self
            .runner
            .run(&program, &all)
            .map_err(|e| plain(&format!("{e:#}"), 200))?;
        if out.ok() {
            Ok(out.stdout)
        } else {
            Err(format!(
                "madmail {} exited {}: {}",
                args.first().copied().unwrap_or(""),
                out.status,
                plain(out.stderr.trim(), 200)
            ))
        }
    }

    /// The runtime switches 111 decides (spike §4, §5): push off (no
    /// `notifications.delta.chat`), federation rejected (nothing leaves the hub),
    /// registration open only while LAN sharing is on. Run while the relay is stopped.
    pub fn settle(&self, lan: bool) -> Result<(), String> {
        self.run(&["push", "off"])?;
        self.run(&["federation", "policy", "reject"])?;
        self.run(&["registration", if lan { "open" } else { "close" }])?;
        Ok(())
    }
}

impl RelayAccounts for MadmailCli {
    fn create_user(&self) -> Result<String, String> {
        parse_create_user(&self.run(&["--json", "create-user"])?)
    }
}

/// The `dclogin:` URI of `madmail --json create-user` (`{"dclogin": …, "email", "password",
/// "username"}`, spike row 4); validated, never logged.
pub fn parse_create_user(stdout: &str) -> Result<String, String> {
    if stdout.len() > 16 * 1024 {
        return Err("create-user printed more than 16 KiB".into());
    }
    let v: Value = serde_json::from_str(stdout.trim())
        .map_err(|_| "create-user did not print one JSON object".to_string())?;
    let uri = v
        .get("dclogin")
        .or_else(|| v.get("dclogin_url"))
        .and_then(Value::as_str)
        .ok_or_else(|| "create-user printed no dclogin".to_string())?;
    if dclogin_ok(uri) {
        Ok(uri.to_string())
    } else {
        Err("create-user printed an invalid dclogin URI".into())
    }
}

/// The relay's health: its IMAP listener accepts a connection within `timeout` (a bounded
/// wait during enable only, never a resident probe).
pub async fn relay_listening(bind: RelayBind, ports: RelayPorts, timeout: Duration) -> bool {
    let addr = std::net::SocketAddr::from((bind.ip(), ports.imap));
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(Ok(_)) = tokio::time::timeout(
            Duration::from_millis(500),
            tokio::net::TcpStream::connect(addr),
        )
        .await
        {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Pre-bind the block on a LAN address (§2: only a bind detects a foreign holder; the
/// component's claim pre-binds loopback).
fn lan_ports_free(bind: RelayBind, ports: RelayPorts) -> Result<(), EnableError> {
    if let RelayBind::Lan(ip) = bind {
        for p in ports.all() {
            if std::net::TcpListener::bind((ip, p)).is_err() {
                return Err(EnableError::StartFailed(format!(
                    "{ip}:{p} is held by another program; the relay cannot listen there"
                )));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- enable, disable

/// What [`enable`] needs from the daemon.
#[derive(Clone)]
pub struct Setup {
    /// The daemon root.
    pub root: PathBuf,
    pub bind: RelayBind,
    pub ports: RelayPorts,
    pub cfg: DcConfig,
    /// Runs the relay's one-shot CLI (production: `SystemRunner`).
    pub cli: Arc<dyn Runner>,
    pub optin: OptIn,
    /// How long the relay may take to open its IMAP listener.
    pub listen_timeout: Duration,
}

impl Setup {
    /// Production defaults on `root` (loopback, the default block, a 20 s listen wait).
    pub fn new(root: &Path, bind: RelayBind, cli: Arc<dyn Runner>, optin: OptIn) -> Self {
        Setup {
            root: root.to_path_buf(),
            bind,
            ports: RelayPorts::default(),
            cfg: DcConfig::default(),
            cli,
            optin,
            listen_timeout: Duration::from_secs(20),
        }
    }

    fn relay_cli(&self, components: &Components) -> MadmailCli {
        let program = components
            .get(RELAY_COMPONENT)
            .map(|c| c.run.program.clone())
            .unwrap_or_else(|| RELAY_COMPONENT.to_string());
        MadmailCli::new(&self.root, &program, Arc::clone(&self.cli))
    }

    fn adapter_cfg(&self) -> DcConfig {
        let mut cfg = self.cfg.clone();
        cfg.relay_host = Some(self.bind.host());
        cfg.joined_file = Some(RelayPaths::of(&self.root).joined);
        cfg
    }
}

/// Stop the relay, set its switches, render `bind`, start it again (the relay's CLI writes
/// its database while nothing else does).
async fn restart_relay(
    components: &Components,
    s: &Setup,
    bind: RelayBind,
) -> Result<(), EnableError> {
    let paths = RelayPaths::of(&s.root);
    components.stop_before(RELAY_COMPONENT).await;
    let cli = s.relay_cli(components);
    let lan = bind.is_lan();
    let settled = tokio::task::spawn_blocking(move || cli.settle(lan))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
    let rendered = write_madmail_config(&paths, bind, s.ports).map_err(|e| format!("{e:#}"));
    let free = lan_ports_free(bind, s.ports);
    components.start_after(RELAY_COMPONENT).await;
    match (settled, rendered, free) {
        (Ok(()), Ok(()), Ok(())) => Ok(()),
        (Err(e), _, _) | (_, Err(e), _) => Err(EnableError::StartFailed(format!(
            "the Madmail relay could not be configured: {e}"
        ))),
        (_, _, Err(e)) => Err(e),
    }
}

/// Enable the Delta Chat network (§7, §2.1 rule 4), each component through its own licence
/// round trip (`relay_ack`, `core_ack`: the hashes the user echoes; a missing one answers
/// `409 consent_required` with that notice and nothing past it runs):
///
/// 1. `bridge/deltachat/` and its directories (0700); the relay's first configuration is
///    loopback-only whatever the LAN setting (its runtime switches are not set yet);
/// 2. `Components::enable("madmail")` (glibc ≥ 2.38, the cache against the pin, the port
///    block claimed and pre-bound), then stopped once to set its switches (`push off`,
///    federation rejected, registration per LAN sharing) and render `bind`, then started;
/// 3. health: the IMAP listener (else the relay is disabled again);
/// 4. `Components::enable("deltachat")` (needs `madmail` enabled; `DC_ACCOUNTS_PATH` =
///    `bridge/deltachat/accounts`);
/// 5. [`DeltaChatAdapter::attach`] on its stdio. The daemon registers it:
///    `bridge.enable(adapter.clone() as Arc<dyn Adapter>).await` (C1's seam).
///
/// Idempotent: an enabled relay is not restarted. When the core's acknowledgement is
/// missing, the relay stays enabled and the call answers the core's `409`.
pub async fn enable(
    components: &Components,
    s: &Setup,
    relay_ack: Option<&str>,
    core_ack: Option<&str>,
) -> Result<Arc<DeltaChatAdapter>, EnableError> {
    let paths = RelayPaths::of(&s.root);
    paths.prepare().map_err(|e| {
        EnableError::InstallFailed(format!("cannot create {}: {e}", paths.dir.display()))
    })?;
    let relay_enabled = components.stdio(RELAY_COMPONENT).await.is_some();
    if !relay_enabled {
        write_madmail_config(&paths, RelayBind::Loopback, s.ports)
            .map_err(|e| EnableError::InstallFailed(format!("{e:#}")))?;
    }
    components.enable(RELAY_COMPONENT, relay_ack).await?;
    if !relay_enabled {
        if let Err(e) = restart_relay(components, s, s.bind).await {
            components.disable(RELAY_COMPONENT).await;
            return Err(e);
        }
    }
    if !relay_listening(s.bind, s.ports, s.listen_timeout).await {
        components.disable(RELAY_COMPONENT).await;
        return Err(EnableError::StartFailed(format!(
            "Madmail started, but its IMAP listener {}:{} did not open",
            s.bind.host(),
            s.ports.imap
        )));
    }
    components.enable(RPC_COMPONENT, core_ack).await?;
    let stdio = components
        .stdio(RPC_COMPONENT)
        .await
        .ok_or_else(|| EnableError::StartFailed("the Delta Chat core is not running".into()))?;
    Ok(DeltaChatAdapter::attach(
        s.adapter_cfg(),
        stdio,
        Arc::new(s.relay_cli(components)),
        Arc::clone(&s.optin),
    ))
}

/// The adapter of a network the daemon resumed (`Components::resume_enabled` started both
/// components); `None` when the core is not enabled. The relay's configuration on disk is
/// the last one rendered.
pub async fn resume(components: &Components, s: &Setup) -> Option<Arc<DeltaChatAdapter>> {
    let stdio = components.stdio(RPC_COMPONENT).await?;
    Some(DeltaChatAdapter::attach(
        s.adapter_cfg(),
        stdio,
        Arc::new(s.relay_cli(components)),
        Arc::clone(&s.optin),
    ))
}

/// Disable the network: the adapter leaves the bridge, then the core, then the relay stop
/// and release their ports. Installed files and the account store stay until "remove
/// files".
pub async fn disable(components: &Components, bridge: &super::Bridge) {
    bridge.disable(NETWORK).await;
    components.disable(RPC_COMPONENT).await;
    components.disable(RELAY_COMPONENT).await;
}

/// LAN sharing changed: render the relay for `bind` and, when it is enabled, restart it
/// with registration open or closed. The relay's addresses carry its IP, so a bridge
/// account made on another address reads `relay_moved`
/// ([`DeltaChatAdapter::set_relay_host`]); contacts join again after "remove files".
pub async fn relan(components: &Components, s: &Setup, bind: RelayBind) -> Result<(), EnableError> {
    let paths = RelayPaths::of(&s.root);
    if components.stdio(RELAY_COMPONENT).await.is_none() {
        return write_madmail_config(&paths, bind, s.ports)
            .map_err(|e| EnableError::InstallFailed(format!("{e:#}")));
    }
    restart_relay(components, s, bind).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_are_validated_and_matched_by_id_only() {
        assert_eq!(
            parse_response(r#"{"jsonrpc":"2.0","id":7,"result":null}"#),
            Some((7, Ok(Value::Null)))
        );
        assert_eq!(
            parse_response("{\"jsonrpc\":\"2.0\",\"id\":2,\"error\":{\"code\":-1,\"message\":\"no\\u202echat\"}}"),
            Some((2, Err(RpcError::Remote("nochat".into())))),
            "the core's error text is cleaned (U+202E dropped)"
        );
        for bad in [
            "",
            "not json",
            "[1,2]",
            r#"{"jsonrpc":"1.0","id":1,"result":1}"#,
            r#"{"jsonrpc":"2.0","result":1}"#,
            r#"{"jsonrpc":"2.0","id":"1","result":1}"#,
            r#"{"jsonrpc":"2.0","id":1}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":1,"error":{}}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":1,"extra":true}"#,
            r#"{"jsonrpc":"2.0","method":"notify","params":[]}"#,
        ] {
            assert_eq!(parse_response(bad), None, "{bad:?}");
        }
        let huge = format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":"{}"}}"#,
            "x".repeat(PIPED_LINE_MAX)
        );
        assert_eq!(parse_response(&huge), None, "oversize lines are noise");
    }

    #[test]
    fn events_parse_by_kind_and_unknown_kinds_are_noise() {
        let ev = |kind: &str, extra: Value| {
            let mut e = json!({ "kind": kind });
            if let (Value::Object(m), Value::Object(x)) = (&mut e, extra) {
                m.extend(x);
            }
            json!({ "contextId": 1, "event": e })
        };
        assert_eq!(
            parse_event(&ev("Info", json!({"msg": "IMAP connected"}))),
            Some((1, Event::Info))
        );
        assert_eq!(
            parse_event(&ev("IncomingMsg", json!({"chatId": 12, "msgId": 40}))),
            Some((1, Event::Incoming { msg: 40 }))
        );
        assert_eq!(
            parse_event(&ev("MsgDelivered", json!({"chatId": 12, "msgId": 41}))),
            Some((
                1,
                Event::State {
                    msg: 41,
                    state: OUT_DELIVERED
                }
            ))
        );
        assert_eq!(
            parse_event(&ev("MsgRead", json!({"chatId": 12, "msgId": 41}))),
            Some((
                1,
                Event::State {
                    msg: 41,
                    state: OUT_MDN_RCVD
                }
            ))
        );
        assert_eq!(
            parse_event(&ev("MsgFailed", json!({"chatId": 12, "msgId": 41}))),
            Some((
                1,
                Event::State {
                    msg: 41,
                    state: OUT_FAILED
                }
            ))
        );
        assert_eq!(
            parse_event(&ev(
                "SecurejoinInviterProgress",
                json!({"contactId": 10, "chatType": "Single", "chatId": 12, "progress": 1000})
            )),
            Some((
                1,
                Event::Joined {
                    contact: 10,
                    progress: 1000
                }
            ))
        );
        assert_eq!(
            parse_event(&ev("ConnectivityChanged", json!({}))),
            Some((1, Event::Other))
        );
        assert_eq!(parse_event(&ev("Telepathy", json!({}))), None);
        assert_eq!(parse_event(&ev("IncomingMsg", json!({"chatId": 1}))), None);
        assert_eq!(
            parse_event(&json!({"contextId": -1, "event": {"kind": "Info"}})),
            None
        );
        assert_eq!(
            parse_event(&json!({"contextId": 1, "event": {"kind": "Info"}, "x": 1})),
            None
        );
    }

    #[test]
    fn addresses_chats_bodies_and_uris_are_checked() {
        assert!(address_ok("2f6zraev6xd4@[127.0.0.1]"));
        assert!(address_ok("alice@[192.168.1.5]"));
        for bad in [
            "",
            "alice",
            "@x",
            "a@",
            "a b@x",
            "a@x\u{202e}",
            "<a>@x",
            "a@x/y",
        ] {
            assert!(!address_ok(bad), "{bad:?}");
        }
        assert_eq!(address_host("a@[192.168.1.5]"), Some("192.168.1.5"));
        assert_eq!(
            chat_kind(&json!({"chatType": "Single"})),
            Some(ConvKind::Dm)
        );
        assert_eq!(
            chat_kind(&json!({"chatType": "Group"})),
            Some(ConvKind::Group)
        );
        assert_eq!(chat_kind(&json!({"chatType": "Mailinglist"})), None);
        assert_eq!(chat_kind(&json!({"chatType": "InBroadcast"})), None);
        assert_eq!(chat_kind(&json!({})), None);
        let m = |text: &str, file_bytes: u64| Msg {
            from: 10,
            chat: 12,
            text: text.into(),
            is_info: false,
            file_bytes,
            timestamp: None,
            sender: None,
        };
        assert_eq!(body_of(&m("hi", 0)).as_deref(), Some("hi"));
        assert_eq!(
            body_of(&m("", 2048)).as_deref(),
            Some("attachment not shown (2048 bytes)")
        );
        assert_eq!(
            body_of(&m("look", 5)).as_deref(),
            Some("look\n\nattachment not shown (5 bytes)")
        );
        assert_eq!(body_of(&m("", 0)), None);
        assert!(dclogin_ok(
            "dclogin:2f6zraev6xd4@[127.0.0.1]/?p=x&v=1&ih=127.0.0.1&ip=11242&is=ssl&sh=127.0.0.1&sp=11241&ss=ssl&ic=3"
        ));
        assert!(!dclogin_ok("https://evil/"));
        assert!(!dclogin_ok("dclogin:a b"));
        assert!(qr_ok(
            "https://i.delta.chat/#EDDC2648&v=3&i=a&s=b&a=x%40%5B127.0.0.1%5D&n=111+bridge"
        ));
        assert!(!qr_ok("https://example.org/#x"));
    }

    #[test]
    fn create_user_output_yields_the_dclogin_uri() {
        let out = r#"{"dclogin":"dclogin:u@[127.0.0.1]/?p=s&v=1&ih=127.0.0.1&ip=11242&is=ssl&sh=127.0.0.1&sp=11241&ss=ssl&ic=3","email":"u@[127.0.0.1]","password":"s","username":"u"}"#;
        assert!(parse_create_user(out)
            .unwrap()
            .starts_with("dclogin:u@[127.0.0.1]/"));
        assert!(parse_create_user("Created!\n").is_err());
        assert!(parse_create_user(r#"{"dclogin":"https://x/"}"#).is_err());
        assert!(parse_create_user(r#"{"email":"u@x"}"#).is_err());
    }

    fn paths() -> RelayPaths {
        RelayPaths::of(Path::new("/var/lib/111"))
    }

    #[test]
    fn madmail_config_golden_loopback() {
        let got = render_madmail_config(RelayBind::Loopback, RelayPorts::default(), &paths());
        let want = "\
# Written by 111 (Layer 6 §7): the Madmail relay of the Delta Chat bridge, loopback only.
# Never `madmail install` or `upgrade`. Not here, so off: plain IMAP, plain submission,
# plain HTTP, OpenMetrics, TURN listeners, Shadowsocks, Iroh. Rewritten on every enable
# and LAN-sharing change; edits are lost.
hostname = \"127.0.0.1\"
primary_domain = \"127.0.0.1\"
state_dir = \"/var/lib/111/bridge/deltachat/madmail\"
runtime_dir = \"/var/lib/111/bridge/deltachat/madmail-run\"
tls_mode = \"self_signed\"
debug = false
auth_auto_create = false
smtp_listen = \"127.0.0.1:11240\"
submission_tls_listen = \"127.0.0.1:11241\"
imap_tls_listen = \"127.0.0.1:11242\"
http_tls_listen = \"127.0.0.1:11243\"
turn_enable = false
";
        assert_eq!(got, want);
    }

    #[test]
    fn madmail_config_golden_lan_and_the_forbidden_features_stay_off() {
        let lan = RelayBind::Lan(Ipv4Addr::new(192, 168, 1, 5));
        let got = render_madmail_config(lan, RelayPorts::default(), &paths());
        let want = "\
# Written by 111 (Layer 6 §7): the Madmail relay of the Delta Chat bridge, LAN-bound while LAN sharing is on.
# Never `madmail install` or `upgrade`. Not here, so off: plain IMAP, plain submission,
# plain HTTP, OpenMetrics, TURN listeners, Shadowsocks, Iroh. Rewritten on every enable
# and LAN-sharing change; edits are lost.
hostname = \"192.168.1.5\"
primary_domain = \"192.168.1.5\"
state_dir = \"/var/lib/111/bridge/deltachat/madmail\"
runtime_dir = \"/var/lib/111/bridge/deltachat/madmail-run\"
tls_mode = \"self_signed\"
debug = false
auth_auto_create = false
smtp_listen = \"192.168.1.5:11240\"
submission_tls_listen = \"192.168.1.5:11241\"
imap_tls_listen = \"192.168.1.5:11242\"
http_tls_listen = \"192.168.1.5:11243\"
turn_enable = false
";
        assert_eq!(got, want);
        // the TOML parses, and no key that would open another listener or reach out
        let body: String = got
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        let t: toml::Table = toml::from_str(&body).unwrap();
        for key in [
            "imap_listen",
            "submission_listen",
            "http_listen",
            "openmetrics_listen",
            "turn_listen_udp",
            "turn_server",
            "ss_addr",
            "acme_email",
            "public_ip",
            "www_dir",
            "admin_web_path",
        ] {
            assert!(!t.contains_key(key), "{key} must not be set");
        }
        assert_eq!(t["tls_mode"].as_str(), Some("self_signed"));
        assert_eq!(t["turn_enable"].as_bool(), Some(false));
        assert_eq!(t["auth_auto_create"].as_bool(), Some(false));
        // every listener is on the bound address and in the Layer 6 block
        for key in [
            "smtp_listen",
            "submission_tls_listen",
            "imap_tls_listen",
            "http_tls_listen",
        ] {
            let v = t[key].as_str().unwrap();
            let (host, port) = v.rsplit_once(':').unwrap();
            assert_eq!(host, "192.168.1.5");
            assert!((11240..=11243).contains(&port.parse::<u16>().unwrap()));
        }
        assert_eq!(RelayPorts::default().all(), [11240, 11241, 11242, 11243]);
    }

    #[test]
    fn the_card_carries_the_qr_text_and_a_module_matrix() {
        let m = fjson::qr("https://i.delta.chat/#EDDC&v=3&i=a&s=b&a=u%40%5B127.0.0.1%5D").unwrap();
        assert!(m.get("size").is_some() || m.is_object(), "{m}");
        assert!(ACCOUNT_WARNING.starts_with("This hub decrypts everything this account receives"));
        assert!(CARD_TITLE.starts_with("Open Delta Chat"));
    }
}
