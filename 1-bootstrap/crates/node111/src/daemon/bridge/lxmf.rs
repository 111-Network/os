//! The LXMF adapter (Layer 6 `6-transport/README.md` §6 "LXMF adapter: our own helper";
//! D26 #4): the Rust side of the `lxmf-sidecar` helper, attached to the component runner's
//! child (`assets/components/lxmf.toml`, §2.1) through its piped stdio, JSON Lines both
//! ways (§6 "Process contract"; the helper's README "Protocol"). `111d` imports nothing of
//! Reticulum: the helper is the one program of ours that does, a separate process (§15).
//!
//! - **Attachment (§6 "Start order", §4 states).** [`LxmfAdapter::start`] takes the runner's
//!   stdio watch ([`crate::daemon::components::Components::stdio`]). Every [`PipedIo`]
//!   generation is attached afresh: its **first line** ([`PipedIo::first_line`]) must be
//!   `ready {dest, pubkey, instance}`, else the generation is refused (`adapter_down`,
//!   logged, counted in [`LxmfAdapter::refused`]) and asked to quit so the supervisor
//!   respawns it. A ready line makes the adapter `Live`, the delivery destination the one
//!   account ([`Adapter::facts`]), and its 64 B public key the only key of a **fresh**
//!   [`BridgeVerifier`] (§14: `ready.pubkey` feeds the `0x03` hash binding; carried, never
//!   derived). When a generation ends — the child exited, a respawn replaced it, the helper
//!   reported `partitioned` — every send in flight fails `adapter_down` with
//!   [`Retry::Later`]: a send never on air stays `queued` and the bridge core re-hands it
//!   (§6 (4): a FAILED while OUTBOUND is `queued`); one already on air keeps `sent`
//!   "transmitted, not confirmed" and is never re-sent automatically (§4).
//! - **The trust boundary (§15.9).** Every line is validated before anything is read from
//!   it ([`parse_line`]): at most 64 KiB, one JSON object, a closed `type` set, a closed key
//!   set per type, every field type-checked and capped (destination hashes 32 lowercase hex,
//!   LXMF message hashes 64, names ≤ 128 B, `fields` a bounded object nested at most
//!   [`FIELDS_DEPTH`] deep with capped strings, `reason` from the closed set). A malformed
//!   line is counted as noise ([`LxmfAdapter::noise`], with the supervisor's own oversize /
//!   non-UTF-8 drops) and dropped; no field of it is interpreted. Log lines carry fixed text
//!   and counters only, never foreign text.
//! - **Inbound (§3, §6 (5)).** `msg` ⇒ [`Inbound::Text`] in `dm:<src>` of our account
//!   (`auth` `signed` exactly when the library validated the signature, else `none` with
//!   `why_unsigned` in `meta`; the hash is the dedupe key, ids are globally unique), plus a
//!   conversation upsert when the sender's announced name is new; the received relay custom
//!   field is surfaced in `meta.relay` only in the exact shape this adapter writes (`net`
//!   registered, `id` a §8 marker name of 1..=10 bytes, `mid` 32 lowercase hex; otherwise
//!   the field alone is dropped and counted, [`LxmfAdapter::relay_refused`]); a field
//!   nested deeper than [`FIELDS_DEPTH`] is dropped alone with `meta.fields_truncated`
//!   ([`LxmfAdapter::fields_dropped`]); attachments are counted in `meta.attachments`,
//!   never ingested. `heard` ⇒

//!   [`Inbound::Nearby`] (a `reticulum` `NearbySource`), which also wakes the core's `Later`
//!   rows (§4 "earlier on an adapter event"); heards are coalesced and rate-limited per
//!   destination ([`LxmfConfig::heard_interval`], monotonic clock) because the helper
//!   reports every announce. `link` ⇒ [`Inbound::Link`]. The inbound queue is bounded; an
//!   overflow drops the oldest item and reports [`Inbound::Dropped`] (§4: never silent).
//! - **Sends (§6 (4), (6)).** [`Adapter::send`] writes one `send {id, to, text, fields?}`
//!   line and returns its handle at once. A relay copy is recognised by its visible marker
//!   (§8; the core's `SendOpts` carries no field hook) and the custom field rides as
//!   `fields.relay = {v, net, id, mid}` (the helper sets `FIELD_CUSTOM_TYPE "111.relay"` and
//!   `FIELD_CUSTOM_DATA`). `state` lines map onto [`SendEvent`]s under the §4 monotone rule,
//!   applied here as well as in the helper: `queued` "no path yet" / "waiting for their
//!   announce" (the helper parks it and resumes on the peer's announce), `sent`, `relayed`,
//!   `delivered`; a post-air `FAILED` keeps `sent` "transmitted, not confirmed" and the same
//!   `send` line is written again on that peer's next surfaced `heard` (§6 (4): `Later` on
//!   the next heard; the helper starts a new library attempt, its emitted states stay
//!   monotone); `failed` only for `rejected` (stamp, blackhole) and `cancelled`.
//!   `Accepted` carries the LXMF message hash, and is sent **only once the send is on air**
//!   (`sent`/`relayed`/`delivered`): the helper reports the hash as soon as the library
//!   packed the message, possibly still `queued` (OUTBOUND, nothing transmitted), and the
//!   core reads any failure after `Accepted` as post-air (§4 forbids that evidence for a
//!   text never transmitted). A `queued` line's hash is kept adapter-side from its first
//!   line and an inbound `msg` carrying one of our hashes is dropped as our own echo
//!   (§8 guard 2; [`LxmfAdapter::echoes`]).
//!
//! - **Announce on request (§6 (3)).** The adapter writes the helper's `announce {}` line
//!   (i) once right after the `ready` of every new generation (the component's start or a
//!   restart: the peers relearn our address) and (ii) when a send parks as
//!   `queued`/`peer_unannounced` (so the peer can learn our address and reply; the helper
//!   also asks its rnsd for the peer's path). Both are bounded by one announce per
//!   [`LxmfConfig::announce_interval`] (60 s) per helper run, read on the monotonic clock
//!   where the adapter already handles the lines — no timer, no task. The helper's own
//!   start, link-up and interval announces are unchanged.
//!
//! Weight: the adapter exists only while the `lxmf` component is enabled (§2.1 rule 2). Its
//! one task waits on the stdio watch, the helper's lines and its own command queue; it owns
//! no timer and polls nothing (a write is bounded by a timeout only while it runs).

use super::attest::BridgeVerifier;
use super::core::AdapterManifest;
use super::{
    ingest, marker, AccountFacts, Adapter, AdapterFacts, AdapterState, Auth, Caps, ConvKind,
    ConvRef, ForeignMessageState as S, Honest, Inbound, InboundText, LinkState, PerKind, Reason,
    Retry, SendEvent, SendHandle, SendOpts, Syntax, NETWORK_LXMF,
};
use crate::daemon::components::consent::AnnounceRecord;
use crate::daemon::supervise::{PipedIo, PIPED_LINE_MAX};
use futures_util::future::BoxFuture;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, watch, Notify};
use tokio::task::JoinHandle;

/// The component id of the helper (`assets/components/lxmf.toml`).
pub const COMPONENT_ID: &str = "lxmf";
/// One protocol line at most, either direction (§6: 64 KiB).
pub const LINE_MAX: usize = PIPED_LINE_MAX;
/// A display name from the helper at most (its protocol cap, `NAME_MAX_BYTES`).
pub const HELPER_NAME_MAX: usize = 128;
/// A sender name handed to the core's ingest at most (§3: 64 B at a UTF-8 boundary).
pub const NAME_MAX: usize = ingest::MAX_NAME_BYTES;
/// Nesting of a received `fields` object at most (the object itself is level 1).
pub const FIELDS_DEPTH: usize = 3;
/// Entries of one `fields` container at most (the helper renders 64 ids + `relay`).
pub const FIELDS_ENTRIES: usize = 65;
/// One string (or key) inside `fields` at most, in bytes (the helper cuts at 1024 chars).
pub const FIELD_STR_MAX: usize = 4 * 1024;
/// A send id (the helper accepts 1..=128 characters).
pub const SEND_ID_MAX: usize = 128;
/// Sends tracked at once (the helper's own table holds 1,024).
pub const MAX_SENDS: usize = 1024;
/// Items the inbound queue holds; an overflow drops the oldest (§4).
pub const INBOUND_CAP: usize = 4096;
/// Distinct destinations with a `heard` waiting in the queue at most.
pub const HEARD_PENDING_MAX: usize = 256;
/// Destinations whose last surfaced `heard` is remembered (rate limit).
pub const HEARD_MEMORY: usize = 4096;
/// Sender titles remembered (a conversation upsert only when one changes).
const TITLE_MEMORY: usize = 1024;
/// Events buffered per send handle.
const EVENTS: usize = 64;
/// Commands queued for the helper at most (a full queue answers `adapter_down`, `Later`).
const CMD_CAP: usize = 256;
/// Nearby node names (Layer 5 §14 `NearbyNode`: names truncated to 40 bytes).
const NEARBY_NAME_MAX: usize = 40;
/// The helper's `announce {}` command (§6 "Process contract").
const ANNOUNCE_LINE: &str = "{\"type\":\"announce\"}";

/// The evidence strings of the §4 table (LXMF rows).
pub const EV_NO_PATH: &str = "no path yet";
pub const EV_UNANNOUNCED: &str = "waiting for their announce";
pub const EV_SENT: &str = "sent; waiting for the recipient's node to confirm";
pub const EV_RELAYED: &str = "a propagation node holds it";
pub const EV_DELIVERED: &str = "the recipient's node confirmed";
pub const EV_UNCONFIRMED: &str = super::state::TRANSMITTED_NOT_CONFIRMED;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One log line: fixed text and numbers only (§15.1: never foreign text, never key
/// material). Every call site passes a literal or numbers.
fn log(text: std::fmt::Arguments<'_>) {
    eprintln!("111 bridge: lxmf: {text}");
}

// ---------------------------------------------------------------- validation (§15.9)

/// Why a helper line was dropped as noise: a fixed token, never the line's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Noise {
    Oversize,
    BadJson,
    NotObject,
    UnknownType,
    /// A key outside the type's closed set, or a required key missing.
    Keys,
    /// A field of the wrong type, out of range or over its cap.
    Field(&'static str),
    /// `fields` deeper than [`FIELDS_DEPTH`], too many entries or an over-long string.
    Fields,
}

impl Noise {
    pub fn token(self) -> &'static str {
        match self {
            Noise::Oversize => "oversize",
            Noise::BadJson => "bad_json",
            Noise::NotObject => "not_object",
            Noise::UnknownType => "unknown_type",
            Noise::Keys => "keys",
            Noise::Field(f) => f,
            Noise::Fields => "fields",
        }
    }
}

/// `ready {dest, pubkey, instance}` (§6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyLine {
    pub dest: String,
    pub pubkey: [u8; 64],
    pub instance: String,
}

/// What the adapter knows of the current helper run (handed to [`ReadyHook`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyFacts {
    /// The [`PipedIo`] generation that reported it.
    pub generation: u64,
    /// The LXMF delivery destination hash (32 hex): carried, never derived (§14).
    pub dest: String,
    /// The identity's 64 B public key (`X25519 ‖ Ed25519`) as the helper reported it.
    pub pubkey: [u8; 64],
    /// The shared-instance name of the helper's client config.
    pub instance: String,
}

/// The relay custom field `{v, net, id, mid}` (§6 "Relay marker and fields").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayField {
    pub v: u8,
    pub net: String,
    pub id: String,
    pub mid: String,
}

impl RelayField {
    fn json(&self) -> Value {
        json!({ "v": self.v, "net": self.net, "id": self.id, "mid": self.mid })
    }

    /// The exact JSON text in key order `v, net, id, mid`.
    fn line(&self) -> String {
        format!(
            "{{\"v\":{},\"net\":{},\"id\":{},\"mid\":{}}}",
            self.v,
            jstr(&self.net),
            jstr(&self.id),
            jstr(&self.mid)
        )
    }
}

/// `msg {hash, src, name, text, claim_ts, signed, why_unsigned, fields}` (§6), validated.
#[derive(Debug, Clone, PartialEq)]
pub struct MsgLine {
    pub hash: String,
    pub src: String,
    pub name: Option<String>,
    pub text: String,
    pub claim_ts: Option<i64>,
    pub signed: bool,
    pub why_unsigned: Option<&'static str>,
    /// The received relay field; `None` when absent or when it failed the §8 alphabet.
    pub relay: Option<RelayField>,
    /// A relay field was present but refused by the §8 alphabet.
    pub relay_refused: bool,
    /// Attachments (`FIELD_FILE_ATTACHMENTS`, `FIELD_IMAGE`, `FIELD_AUDIO`): count, bytes.
    pub attachments: (u64, u64),
    pub truncated: bool,
    /// The helper cut a field, or this adapter dropped one nested too deep.
    pub fields_truncated: bool,
    /// Fields this adapter dropped for nesting deeper than [`FIELDS_DEPTH`].
    pub fields_dropped: u64,
}

/// `state {id, hash?, state, reason?}` (§6), validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateLine {
    pub id: String,
    pub hash: Option<String>,
    pub state: S,
    pub reason: Option<HelperReason>,
}

/// The helper's closed reason set (its README "Send states").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperReason {
    PeerUnannounced,
    Unconfirmed,
    Rejected,
    Cancelled,
    HelperBusy,
    SendError,
}

impl HelperReason {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "peer_unannounced" => HelperReason::PeerUnannounced,
            "transmitted, not confirmed" => HelperReason::Unconfirmed,
            "rejected" => HelperReason::Rejected,
            "cancelled" => HelperReason::Cancelled,
            "helper_busy" => HelperReason::HelperBusy,
            "send_error" => HelperReason::SendError,
            _ => return None,
        })
    }

    /// Which states may carry it (a mismatch is noise).
    fn fits(self, s: S) -> bool {
        match self {
            HelperReason::PeerUnannounced => s == S::Queued,
            HelperReason::Unconfirmed => s == S::Sent,
            _ => s == S::Failed,
        }
    }
}

/// One validated helper line.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Ready(ReadyLine),
    Link(LinkState),
    Msg(Box<MsgLine>),
    State(StateLine),
    Heard { dest: String, name: Option<String> },
}

/// `n` lowercase hex digits exactly.
pub fn is_hex(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn unhex64(s: &str) -> Option<[u8; 64]> {
    if !is_hex(s, 128) {
        return None;
    }
    let digit = |b: u8| match b {
        b'0'..=b'9' => b - b'0',
        _ => b - b'a' + 10,
    };
    let mut out = [0u8; 64];
    for (i, pair) in s.as_bytes().chunks_exact(2).enumerate() {
        out[i] = (digit(pair[0]) << 4) | digit(pair[1]);
    }
    Some(out)
}

/// A JSON string literal (never fails: a `Value` always serialises).
fn jstr(s: &str) -> String {
    Value::String(s.to_owned()).to_string()
}

/// The object's keys are exactly `required` plus any of `optional`.
fn keys_ok(o: &Map<String, Value>, required: &[&str], optional: &[&str]) -> bool {
    required.iter().all(|k| o.contains_key(*k))
        && o.keys()
            .all(|k| required.contains(&k.as_str()) || optional.contains(&k.as_str()))
}

fn hex_field(o: &Map<String, Value>, key: &'static str, n: usize) -> Result<String, Noise> {
    match o.get(key) {
        Some(Value::String(s)) if is_hex(s, n) => Ok(s.clone()),
        _ => Err(Noise::Field(key)),
    }
}

/// `null` or a string of at most `max` bytes.
fn opt_str(o: &Map<String, Value>, key: &'static str, max: usize) -> Result<Option<String>, Noise> {
    match o.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.len() <= max => Ok(Some(s.clone())),
        _ => Err(Noise::Field(key)),
    }
}

/// An optional flag the helper sets only to `true`.
fn flag(o: &Map<String, Value>, key: &'static str) -> Result<bool, Noise> {
    match o.get(key) {
        None => Ok(false),
        Some(Value::Bool(true)) => Ok(true),
        _ => Err(Noise::Field(key)),
    }
}

/// The §8 marker alphabet: no `]`, `/`, Cc or Cf.
fn marker_alphabet(s: &str) -> bool {
    !s.chars()
        .any(|c| c == ']' || c == '/' || c.is_control() || ingest::is_cf(c))
}

/// The registered network ids (§3, defined once).
fn known_network(s: &str) -> bool {
    [
        super::NETWORK_MESHTASTIC,
        super::NETWORK_LXMF,
        super::NETWORK_DELTACHAT,
        super::NETWORK_SIMPLEX,
    ]
    .contains(&s)
}

/// Why one received field value was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldErr {
    /// A container deeper than [`FIELDS_DEPTH`]: foreign content (the helper renders five
    /// levels), so only that field is dropped and the message kept (`fields_truncated`).
    Deep,
    /// Over the helper's own caps (entries, string bytes): the line breaks the protocol.
    Noise,
}

/// A field value inside `fields` at level `depth` (a container there must be ≤
/// [`FIELDS_DEPTH`]); strings and keys capped, containers bounded. Every container is
/// checked for its size before its children, so a protocol breach anywhere in the value
/// is reported as noise even when it also nests too deep.
fn field_value(v: &Value, depth: usize) -> Result<(), FieldErr> {
    let deep = |r: Result<(), FieldErr>, d: &mut bool| match r {
        Err(FieldErr::Deep) => {
            *d = true;
            Ok(())
        }
        other => other,
    };
    let mut too_deep = depth > FIELDS_DEPTH && matches!(v, Value::Array(_) | Value::Object(_));
    match v {
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
        Value::String(s) if s.len() <= FIELD_STR_MAX => {}
        Value::String(_) => return Err(FieldErr::Noise),
        Value::Array(a) => {
            if a.len() > FIELDS_ENTRIES {
                return Err(FieldErr::Noise);
            }
            for x in a {
                deep(field_value(x, depth + 1), &mut too_deep)?;
            }
        }
        Value::Object(o) => {
            if o.len() > FIELDS_ENTRIES {
                return Err(FieldErr::Noise);
            }
            for (k, x) in o {
                if k.len() > FIELD_STR_MAX {
                    return Err(FieldErr::Noise);
                }
                deep(field_value(x, depth + 1), &mut too_deep)?;
            }
        }
    }
    if too_deep {
        Err(FieldErr::Deep)
    } else {
        Ok(())
    }
}

/// A received field id: decimal (the helper renders integer ids as decimal strings).
fn field_key(k: &str) -> bool {
    let digits = k.strip_prefix('-').unwrap_or(k);
    (1..=20).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
}

/// `{"attachment": {"bytes": N}}` under ids 5, 6, 7: the byte count.
fn attachment_bytes(k: &str, v: &Value) -> Option<u64> {
    if !matches!(k, "5" | "6" | "7") {
        return None;
    }
    let o = v.as_object()?;
    if o.len() != 1 {
        return None;
    }
    let inner = o.get("attachment")?.as_object()?;
    if inner.len() != 1 {
        return None;
    }
    inner.get("bytes")?.as_u64()
}

/// The relay entry's shape (protocol: a wrong shape is noise) and its content (foreign: a
/// refusal drops the entry, not the message, and is counted in
/// [`LxmfAdapter::relay_refused`]). The accepted content is exactly what this adapter
/// writes ([`relay_for`]): `v` 1, `net` a registered network, `id` a §8 marker name (1..=10
/// bytes of the marker alphabet: no `]`, `/`, Cc or Cf), `mid` 32 lowercase hex (16 bytes).
fn relay_entry(v: &Value) -> Result<Option<RelayField>, Noise> {
    let Some(o) = v.as_object() else {
        return Err(Noise::Field("relay"));
    };
    if !keys_ok(o, &["v", "net", "id", "mid"], &[]) {
        return Err(Noise::Field("relay"));
    }
    let ver = o
        .get("v")
        .and_then(Value::as_u64)
        .filter(|v| *v <= 255)
        .ok_or(Noise::Field("relay"))?;
    let s = |key: &str, cap: usize| -> Result<String, Noise> {
        match o.get(key) {
            Some(Value::String(s)) if !s.is_empty() && s.len() <= cap => Ok(s.clone()),
            _ => Err(Noise::Field("relay")),
        }
    };
    let (net, id, mid) = (s("net", 64)?, s("id", 256)?, s("mid", 256)?);
    let ok = ver == 1
        && known_network(&net)
        && id.len() <= marker::NAME_MAX
        && marker_alphabet(&id)
        && is_hex(&mid, 32);
    Ok(ok.then_some(RelayField { v: 1, net, id, mid }))
}

/// What the received `fields` object yields.
#[derive(Debug, Default)]
struct FieldFacts {
    /// The relay entry (`Some(None)` = present but refused).
    relay: Option<Option<RelayField>>,
    /// Attachment count and bytes.
    attachments: (u64, u64),
    /// Fields dropped for nesting deeper than [`FIELDS_DEPTH`] (the message is kept).
    deep: u64,
}

/// The received `fields` object: bounded, nested at most [`FIELDS_DEPTH`]. A field nested
/// deeper is dropped alone (counted in [`FieldFacts::deep`]); a breach of the helper's own
/// caps is noise.
fn fields(v: &Value) -> Result<FieldFacts, Noise> {
    let Some(o) = v.as_object() else {
        return Err(Noise::Field("fields"));
    };
    if o.len() > FIELDS_ENTRIES {
        return Err(Noise::Fields);
    }
    let mut out = FieldFacts::default();
    let (mut n, mut bytes) = (0u64, 0u64);
    for (k, x) in o {
        if k == "relay" {
            out.relay = Some(relay_entry(x)?);
            continue;
        }
        if !field_key(k) {
            return Err(Noise::Field("fields"));
        }
        match field_value(x, 2) {
            Ok(()) => {}
            Err(FieldErr::Deep) => {
                out.deep += 1;
                continue;
            }
            Err(FieldErr::Noise) => return Err(Noise::Fields),
        }
        if let Some(b) = attachment_bytes(k, x) {
            n += 1;
            bytes = bytes.saturating_add(b);
        }
    }
    out.attachments = (n, bytes);
    Ok(out)
}

/// `claim_ts`: the sender's unverified clock (display only, §3). A JSON number; seconds
/// outside `0..=2^53` read as unknown (the value is a foreign hint, never trusted).
fn claim_ts(v: Option<&Value>) -> Result<Option<i64>, Noise> {
    match v {
        Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => {
            const MAX: f64 = 9_007_199_254_740_992.0;
            let secs = if let Some(i) = n.as_i64() {
                Some(i).filter(|i| (0..=MAX as i64).contains(i))
            } else {
                n.as_f64()
                    .filter(|f| f.is_finite() && (0.0..=MAX).contains(f))
                    .map(|f| f.floor() as i64)
            };
            Ok(secs)
        }
        _ => Err(Noise::Field("claim_ts")),
    }
}

/// Validate one helper line (§6, §15.9). Pure: the adapter acts on the result only.
pub fn parse_line(line: &str) -> Result<Line, Noise> {
    if line.len() > LINE_MAX {
        return Err(Noise::Oversize);
    }
    let v: Value = serde_json::from_str(line).map_err(|_| Noise::BadJson)?;
    let Value::Object(o) = v else {
        return Err(Noise::NotObject);
    };
    let kind = match o.get("type") {
        Some(Value::String(t)) => t.as_str(),
        _ => return Err(Noise::UnknownType),
    };
    match kind {
        "ready" => {
            if !keys_ok(&o, &["type", "dest", "pubkey", "instance"], &[]) {
                return Err(Noise::Keys);
            }
            let dest = hex_field(&o, "dest", 32)?;
            let pubkey = match o.get("pubkey") {
                Some(Value::String(s)) => unhex64(s).ok_or(Noise::Field("pubkey"))?,
                _ => return Err(Noise::Field("pubkey")),
            };
            let instance = match o.get("instance") {
                Some(Value::String(s))
                    if !s.is_empty()
                        && s.len() <= 64
                        && s.bytes().all(|b| b.is_ascii_graphic()) =>
                {
                    s.clone()
                }
                _ => return Err(Noise::Field("instance")),
            };
            Ok(Line::Ready(ReadyLine {
                dest,
                pubkey,
                instance,
            }))
        }
        "link" => {
            if !keys_ok(&o, &["type", "state"], &[]) {
                return Err(Noise::Keys);
            }
            match o.get("state").and_then(Value::as_str) {
                Some("up") => Ok(Line::Link(LinkState::Up)),
                Some("down") => Ok(Line::Link(LinkState::Down)),
                Some("partitioned") => Ok(Line::Link(LinkState::Partitioned)),
                _ => Err(Noise::Field("state")),
            }
        }
        "msg" => {
            let required = [
                "type",
                "hash",
                "src",
                "name",
                "text",
                "claim_ts",
                "signed",
                "why_unsigned",
                "fields",
            ];
            if !keys_ok(&o, &required, &["truncated", "fields_truncated"]) {
                return Err(Noise::Keys);
            }
            let hash = hex_field(&o, "hash", 64)?;
            let src = hex_field(&o, "src", 32)?;
            let name = opt_str(&o, "name", HELPER_NAME_MAX)?;
            let text = match o.get("text") {
                Some(Value::String(s)) if s.len() <= LINE_MAX => s.clone(),
                _ => return Err(Noise::Field("text")),
            };
            let claim_ts = claim_ts(o.get("claim_ts"))?;
            let signed = o
                .get("signed")
                .and_then(Value::as_bool)
                .ok_or(Noise::Field("signed"))?;
            let why_unsigned = match o.get("why_unsigned") {
                Some(Value::Null) => None,
                Some(Value::String(s)) => Some(match s.as_str() {
                    "source_unknown" => "source_unknown",
                    "signature_invalid" => "signature_invalid",
                    "unverified" => "unverified",
                    _ => return Err(Noise::Field("why_unsigned")),
                }),
                _ => return Err(Noise::Field("why_unsigned")),
            };
            if signed && why_unsigned.is_some() {
                return Err(Noise::Field("why_unsigned"));
            }
            let facts = fields(o.get("fields").unwrap_or(&Value::Null))?;
            Ok(Line::Msg(Box::new(MsgLine {
                hash,
                src,
                name,
                text,
                claim_ts,
                signed,
                why_unsigned: if signed {
                    None
                } else {
                    why_unsigned.or(Some("unverified"))
                },
                relay_refused: matches!(facts.relay, Some(None)),
                relay: facts.relay.flatten(),
                attachments: facts.attachments,
                truncated: flag(&o, "truncated")?,
                fields_truncated: flag(&o, "fields_truncated")? || facts.deep > 0,
                fields_dropped: facts.deep,
            })))
        }
        "state" => {
            if !keys_ok(&o, &["type", "id", "state"], &["hash", "reason"]) {
                return Err(Noise::Keys);
            }
            let id = match o.get("id") {
                Some(Value::String(s))
                    if !s.is_empty()
                        && s.chars().count() <= SEND_ID_MAX
                        && !s.chars().any(char::is_control) =>
                {
                    s.clone()
                }
                _ => return Err(Noise::Field("id")),
            };
            let hash = match o.get("hash") {
                None => None,
                Some(Value::String(s)) if is_hex(s, 64) => Some(s.clone()),
                _ => return Err(Noise::Field("hash")),
            };
            let state = match o.get("state").and_then(Value::as_str) {
                Some("queued") => S::Queued,
                Some("sent") => S::Sent,
                Some("relayed") => S::Relayed,
                Some("delivered") => S::Delivered,
                Some("failed") => S::Failed,
                _ => return Err(Noise::Field("state")),
            };
            let reason = match o.get("reason") {
                None => None,
                Some(Value::String(s)) => {
                    Some(HelperReason::parse(s).ok_or(Noise::Field("reason"))?)
                }
                _ => return Err(Noise::Field("reason")),
            };
            match reason {
                Some(r) if !r.fits(state) => return Err(Noise::Field("reason")),
                None if state == S::Failed => return Err(Noise::Field("reason")),
                _ => {}
            }
            Ok(Line::State(StateLine {
                id,
                hash,
                state,
                reason,
            }))
        }
        "heard" => {
            if !keys_ok(&o, &["type", "dest", "name"], &[]) {
                return Err(Noise::Keys);
            }
            Ok(Line::Heard {
                dest: hex_field(&o, "dest", 32)?,
                name: opt_str(&o, "name", HELPER_NAME_MAX)?,
            })
        }
        _ => Err(Noise::UnknownType),
    }
}

// ---------------------------------------------------------------- sends

/// The §4 monotone rule on the adapter side (the helper's per-id guarantee can lapse once it
/// evicted a concluded entry): `delivered` is final; `failed` only from `queued`/`sent` and
/// then final unless a later `delivered` arrives; a state only moves up; an equal
/// `queued`/`sent` row may only add a new reason.
pub fn admit(
    prior: Option<(S, Option<HelperReason>)>,
    next: S,
    reason: Option<HelperReason>,
) -> bool {
    let Some((p, pr)) = prior else { return true };
    match (p, next) {
        (S::Delivered, _) => false,
        (S::Failed, n) => n == S::Delivered,
        (p, S::Failed) => matches!(p, S::Queued | S::Sent),
        (p, n) if n.rank() > p.rank() => true,
        (p, n) => p == n && matches!(n, S::Queued | S::Sent) && reason.is_some() && reason != pr,
    }
}

/// What one admitted `state` line means for the core (§4, §6 (4), (6)).
///
/// A parked send (`queued`/`peer_unannounced`) is reported as evidence only ("waiting for
/// their announce"), so its row's `reason_code` stays null for now although the contract's
/// `ForeignReasonCode` lists `peer_unannounced` as a queued reason: the core's
/// [`SendEvent::Evidence`] carries no reason (and clears one), and the only queued reason it
/// takes is a pre-air `Failed { .., Retry::Later }`, on which it withdraws the handoff and
/// hands the row again — while the helper still holds the send parked and resumes it on the
/// peer's announce, so the text would go twice (§4 "at most once per user act"). Fail-closed:
/// the evidence alone until the core takes a reason with a queued state (a core follow-up).
fn events_for(s: &StateLine) -> SendEvent {
    match (s.state, s.reason) {
        (S::Queued, Some(HelperReason::PeerUnannounced)) => SendEvent::Evidence {
            state: S::Queued,
            evidence: EV_UNANNOUNCED.into(),
        },
        (S::Queued, _) => SendEvent::Evidence {
            state: S::Queued,
            evidence: EV_NO_PATH.into(),
        },
        (S::Sent, Some(HelperReason::Unconfirmed)) => SendEvent::Evidence {
            state: S::Sent,
            evidence: EV_UNCONFIRMED.into(),
        },
        (S::Sent, _) => SendEvent::Evidence {
            state: S::Sent,
            evidence: EV_SENT.into(),
        },
        (S::Relayed, _) => SendEvent::Evidence {
            state: S::Relayed,
            evidence: EV_RELAYED.into(),
        },
        (S::Delivered, _) => SendEvent::Evidence {
            state: S::Delivered,
            evidence: EV_DELIVERED.into(),
        },
        // REJECTED (stamp, blackhole): the recipient refused it
        (S::Failed, Some(HelperReason::Rejected)) => SendEvent::Failed {
            reason: Reason::NotAuthorized,
            retry: Retry::Never,
        },
        (S::Failed, Some(HelperReason::Cancelled)) => SendEvent::Failed {
            reason: Reason::Cancelled,
            retry: Retry::Never,
        },
        // the helper's table was full: a free slot is an adapter event, retry later
        (S::Failed, Some(HelperReason::HelperBusy)) => SendEvent::Failed {
            reason: Reason::AdapterDown,
            retry: Retry::Later,
        },
        // the library could not build or take the message (before air)
        (S::Failed, _) => SendEvent::Failed {
            reason: Reason::AdapterDown,
            retry: Retry::Never,
        },
    }
}

/// The exact `send` line (§6), keys in the order `type, id, to, text, fields`; `fields`
/// only with a relay field.
pub fn send_line(id: &str, to: &str, text: &str, relay: Option<&RelayField>) -> String {
    let mut s = format!(
        "{{\"type\":\"send\",\"id\":{},\"to\":{},\"text\":{}",
        jstr(id),
        jstr(to),
        jstr(text)
    );
    if let Some(r) = relay {
        s.push_str(",\"fields\":{\"relay\":");
        s.push_str(&r.line());
        s.push('}');
    }
    s.push('}');
    s
}

/// The relay field for a copy whose text leads with a registered marker (§8): `net` is the
/// network of the marker's TAG, `id` the marker name, `mid` the first 16 bytes (hex) of
/// SHA-256 over the text after the marker (the same origin relayed twice carries the same
/// `mid`). `None` for a text without a marker.
pub fn relay_for(text: &str, tags: &[(String, String)]) -> Option<RelayField> {
    let names: Vec<&str> = tags.iter().map(|(t, _)| t.as_str()).collect();
    let p = marker::parse(text, &names)?;
    let net = tags.iter().find(|(t, _)| t == p.tag)?.1.clone();
    let digest = Sha256::digest(p.rest.as_bytes());
    let mid: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    Some(RelayField {
        v: 1,
        net,
        id: p.name.to_string(),
        mid,
    })
}

/// The helper's `[run]` arguments the adapter adds (§6 (3)): `--announce-interval` and,
/// when one is set, `--name`; pass to `Components::enable_with`.
pub fn run_args(announce: &AnnounceRecord) -> Vec<String> {
    let mut args = vec![
        "--announce-interval".to_string(),
        announce.interval_min.to_string(),
    ];
    if let Some(name) = announce.name.as_deref().and_then(ingest::name) {
        args.push("--name".into());
        args.push(name);
    }
    args
}

// ---------------------------------------------------------------- the adapter

/// Called with the facts of every helper run that passed the ready check.
pub type ReadyHook = Arc<dyn Fn(&ReadyFacts) + Send + Sync>;

/// How the adapter is built.
#[derive(Clone)]
pub struct LxmfConfig {
    /// The honest copy (`assets/adapters/lxmf.toml`).
    pub copy: Honest,
    pub recipient: Option<Syntax>,
    /// Registered marker TAGs and their networks (`(TAG, network)`, §8).
    pub tags: Vec<(String, String)>,
    /// A destination's `heard` is surfaced at most once per this interval (monotonic).
    pub heard_interval: Duration,
    /// A write to the helper's stdin waits at most this long (then stdin closes, §7).
    pub write_timeout: Duration,
    /// Announce on request (§6 (3)): at most one `announce {}` per this interval per helper
    /// run (monotonic; the one after `ready` counts).
    pub announce_interval: Duration,
    pub on_ready: Option<ReadyHook>,
}

impl LxmfConfig {
    /// From the bridge's registered adapter manifests.
    pub fn from_manifests(manifests: &[AdapterManifest]) -> Self {
        let m = manifests.iter().find(|m| m.id == NETWORK_LXMF);
        LxmfConfig {
            copy: Honest {
                label: m.map_or_else(|| "Reticulum (LXMF)".into(), |m| m.label.clone()),
                chip: m.map_or_else(|| "LX".into(), |m| m.chip.clone()),
                tag: m.map_or_else(|| "LX".into(), |m| m.tag.clone()),
                banner: m.map(|m| m.banner.clone()).unwrap_or_default(),
                send_warning: m.map(|m| m.send_warning.clone()).unwrap_or_default(),
            },
            recipient: Some(match m.and_then(|m| m.recipient.as_ref()) {
                Some(r) => Syntax {
                    pattern: r.pattern.clone(),
                    hint: r.label.clone(),
                },
                None => Syntax {
                    pattern: "^[0-9a-f]{32}$".into(),
                    hint: "LXMF address (32 hex)".into(),
                },
            }),
            tags: manifests
                .iter()
                .filter(|m| marker::valid_tag(&m.tag))
                .map(|m| (m.tag.clone(), m.id.clone()))
                .collect(),
            heard_interval: Duration::from_secs(30),
            write_timeout: Duration::from_secs(5),
            announce_interval: Duration::from_secs(60),
            on_ready: None,
        }
    }
}

impl Default for LxmfConfig {
    /// From the embedded manifests.
    fn default() -> Self {
        Self::from_manifests(&super::core::embedded_manifests())
    }
}

enum Cmd {
    /// One protocol line; `send` names the tracked send it belongs to.
    Line {
        line: String,
        send: Option<String>,
    },
    Quit(oneshot::Sender<()>),
}

/// One tracked send (§4: its events go to the core's handle).
struct Entry {
    tx: mpsc::Sender<SendEvent>,
    to: String,
    line: String,
    generation: u64,
    last: Option<(S, Option<HelperReason>)>,
    /// The LXMF message hash the helper last reported (also while still `queued`).
    hash: Option<String>,
    /// The hash last handed to the core as `Accepted` — only once the send was on air.
    accepted: Option<String>,
    /// A post-air FAILED: written again on the peer's next surfaced `heard` (§6 (4)).
    unconfirmed: bool,
}

/// Was a send in state `s` on air (§4 table, LXMF rows: `SENDING`/`SENT` and above)?
fn on_air(s: S) -> bool {
    matches!(s, S::Sent | S::Relayed | S::Delivered)
}

impl Entry {
    /// The event that ends this send when its helper run ends (exited, replaced,
    /// partitioned, quit) or its line could not be written: `adapter_down`, `Later`. A send
    /// never on air stays a pre-air `Later` (the core keeps the row `queued` and re-hands
    /// it, §6 (4): a FAILED while OUTBOUND is `queued`); one on air was `Accepted`, so the
    /// core keeps `sent` "transmitted, not confirmed". Fail-closed for a send on air whose
    /// hash never came (the helper always reports one once packed): a post-air reason, so
    /// the core never re-sends what may have been transmitted (§4 "at most once").
    fn down_event(&self) -> SendEvent {
        let aired = self.last.is_some_and(|(p, _)| on_air(p));
        SendEvent::Failed {
            reason: if aired && self.accepted.is_none() {
                Reason::NoResponse
            } else {
                Reason::AdapterDown
            },
            retry: Retry::Later,
        }
    }
}

/// The LXMF hashes of our own sends this adapter saw, `queued` ones included (§8 guard 2):
/// the core records a native id only on `Accepted` (on air), so an inbound `msg` carrying a
/// hash of ours that was never accepted is dropped here. Bounded (oldest forgotten).
#[derive(Default)]
struct OwnHashes {
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl OwnHashes {
    fn record(&mut self, h: &str) {
        if self.set.contains(h) {
            return;
        }
        if self.order.len() >= MAX_SENDS {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        self.order.push_back(h.to_string());
        self.set.insert(h.to_string());
    }

    fn contains(&self, h: &str) -> bool {
        self.set.contains(h)
    }
}

#[derive(Default)]
struct Queue {
    items: VecDeque<Item>,
    heard: HashMap<String, Value>,
    dropped: u64,
}

enum Item {
    In(Box<Inbound>),
    /// The latest `heard` of this destination (coalesced in `Queue::heard`).
    Heard(String),
}

struct St {
    /// The generation that passed the ready check and has not ended.
    live: Option<u64>,
    ready: Option<ReadyFacts>,
    verifier: BridgeVerifier,
    state: AdapterState,
    link: Option<LinkState>,
    quit: bool,
    sends: HashMap<String, Entry>,
    order: VecDeque<String>,
    titles: HashMap<String, String>,
    heard_at: HashMap<String, Instant>,
    heard_order: VecDeque<String>,
    own: OwnHashes,
    /// The last announce this adapter asked of the live run (§6 (3); reset per run).
    announced_at: Option<Instant>,
    /// Supervisor drops (oversize / non-UTF-8) of ended generations.
    io_noise: u64,
}

struct Inner {
    caps: Caps,
    tags: Vec<(String, String)>,
    heard_interval: Duration,
    write_timeout: Duration,
    announce_interval: Duration,
    on_ready: Option<ReadyHook>,
    cmds: mpsc::Sender<Cmd>,
    st: Mutex<St>,
    queue: Mutex<Queue>,
    wake: Notify,
    noise: AtomicU64,
    refused: AtomicU64,
    coalesced: AtomicU64,
    relay_refused: AtomicU64,
    fields_dropped: AtomicU64,
    echoes: AtomicU64,
    next_id: AtomicU64,
    /// The current generation's io (for the supervisor's noise counter).
    io: Mutex<Option<PipedIo>>,
}

/// The LXMF adapter (module doc). Built with [`LxmfAdapter::start`]; registered with
/// `Bridge::enable(adapter)` after the `lxmf` component's enable.
pub struct LxmfAdapter {
    inner: Arc<Inner>,
    driver: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for LxmfAdapter {
    fn drop(&mut self) {
        if let Some(t) = lock(&self.driver).take() {
            t.abort();
        }
    }
}

impl LxmfAdapter {
    /// Attach to the runner's stdio watch (`Components::stdio("lxmf")`) and start the one
    /// driver task. Must be called within a tokio runtime.
    pub fn start(stdio: watch::Receiver<Option<PipedIo>>, cfg: LxmfConfig) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(CMD_CAP);
        let caps = Caps {
            kinds: vec![ConvKind::Dm, ConvKind::Group],
            // §4: 287 B opportunistic; the library picks a link for larger texts
            max_text_bytes: PerKind {
                channel: None,
                dm: Some(287),
                group: Some(287),
            },
            evidence: vec![S::Queued, S::Sent, S::Relayed, S::Delivered],
            // "depends on the path": Reticulum picks the interface
            airtime: None,
            auth_levels: vec![Auth::None, Auth::Signed],
            copy: cfg.copy.clone(),
            recipient_syntax: cfg.recipient.clone(),
            splittable: true,
            state: AdapterState::Error("adapter_down".into()),
        };
        let inner = Arc::new(Inner {
            caps,
            tags: cfg.tags,
            heard_interval: cfg.heard_interval,
            write_timeout: cfg.write_timeout,
            announce_interval: cfg.announce_interval,
            on_ready: cfg.on_ready,
            cmds: tx,
            st: Mutex::new(St {
                live: None,
                ready: None,
                verifier: BridgeVerifier::new(),
                state: AdapterState::Error("adapter_down".into()),
                link: None,
                quit: false,
                sends: HashMap::new(),
                order: VecDeque::new(),
                titles: HashMap::new(),
                heard_at: HashMap::new(),
                heard_order: VecDeque::new(),
                own: OwnHashes::default(),
                announced_at: None,
                io_noise: 0,
            }),
            queue: Mutex::new(Queue::default()),
            wake: Notify::new(),
            noise: AtomicU64::new(0),
            refused: AtomicU64::new(0),
            coalesced: AtomicU64::new(0),
            relay_refused: AtomicU64::new(0),
            fields_dropped: AtomicU64::new(0),
            echoes: AtomicU64::new(0),
            next_id: AtomicU64::new(1),
            io: Mutex::new(None),
        });
        let task = tokio::spawn(drive(Arc::clone(&inner), stdio, rx));
        Arc::new(LxmfAdapter {
            inner,
            driver: Mutex::new(Some(task)),
        })
    }

    /// The facts of the current helper run (`None` before the first ready line).
    pub fn ready(&self) -> Option<ReadyFacts> {
        lock(&self.inner.st).ready.clone()
    }

    /// The verifier of the current helper run: a fresh one per run, holding that run's
    /// `ready.pubkey` only (§14); empty while no run is live.
    pub fn verifier(&self) -> BridgeVerifier {
        lock(&self.inner.st).verifier.clone()
    }

    /// Lines dropped as noise (§15.9): ours plus the supervisor's oversize / non-UTF-8 drops.
    pub fn noise(&self) -> u64 {
        let io = lock(&self.inner.io)
            .as_ref()
            .map_or(0, PipedIo::dropped_invalid);
        self.inner.noise.load(Ordering::SeqCst) + lock(&self.inner.st).io_noise + io
    }

    /// Helper runs refused because their first line was not `ready`.
    pub fn refused(&self) -> u64 {
        self.inner.refused.load(Ordering::SeqCst)
    }

    /// `heard` lines coalesced or rate-limited (not surfaced).
    pub fn coalesced(&self) -> u64 {
        self.inner.coalesced.load(Ordering::SeqCst)
    }

    /// Received relay fields refused by the §8 alphabet (the message was kept).
    pub fn relay_refused(&self) -> u64 {
        self.inner.relay_refused.load(Ordering::SeqCst)
    }

    /// Received fields dropped for nesting deeper than [`FIELDS_DEPTH`] (the message was
    /// kept with `fields_truncated`).
    pub fn fields_dropped(&self) -> u64 {
        self.inner.fields_dropped.load(Ordering::SeqCst)
    }

    /// Inbound `msg`s dropped because their hash is one of our own sends' that the core
    /// never saw accepted (§8 guard 2; the core counts the rest in `Bridge::echoes`).
    pub fn echoes(&self) -> u64 {
        self.inner.echoes.load(Ordering::SeqCst)
    }

    /// Sends tracked now.
    pub fn in_flight(&self) -> usize {
        lock(&self.inner.st).sends.len()
    }

    /// Set the announced display name (`name {display}`, §6 (3)); the helper announces.
    /// `false` when the queue is full or nothing runs.
    pub fn set_name(&self, display: &str) -> bool {
        let Some(name) = ingest::name(display) else {
            return false;
        };
        self.command(format!("{{\"type\":\"name\",\"display\":{}}}", jstr(&name)))
    }

    /// Announce now (`announce {}`, §6 (3)).
    pub fn announce(&self) -> bool {
        self.command("{\"type\":\"announce\"}".into())
    }

    fn command(&self, line: String) -> bool {
        lock(&self.inner.st).live.is_some()
            && self
                .inner
                .cmds
                .try_send(Cmd::Line { line, send: None })
                .is_ok()
    }

    /// Disable (§2.1 rule 5): `quit {}` to the helper, every send in flight fails
    /// `adapter_down` (`Later`), the state becomes `Disabled`, and any later run is asked
    /// to quit at once. Call before `Components::disable("lxmf")`.
    pub async fn quit(&self) {
        let (tx, rx) = oneshot::channel();
        if self.inner.cmds.send(Cmd::Quit(tx)).await.is_ok() {
            let _ = rx.await;
        }
    }
}

impl Inner {
    fn set_state(&self, s: AdapterState) {
        lock(&self.st).state = s;
    }

    fn push(&self, item: Inbound) {
        {
            let mut q = lock(&self.queue);
            push_item(&mut q, Item::In(Box::new(item)));
        }
        self.wake.notify_one();
    }

    fn push_heard(&self, dest: &str, v: Value) {
        {
            let mut q = lock(&self.queue);
            if let Some(slot) = q.heard.get_mut(dest) {
                *slot = v;
                self.coalesced.fetch_add(1, Ordering::SeqCst);
                return;
            }
            if q.heard.len() >= HEARD_PENDING_MAX {
                self.coalesced.fetch_add(1, Ordering::SeqCst);
                return;
            }
            q.heard.insert(dest.to_string(), v);
            push_item(&mut q, Item::Heard(dest.to_string()));
        }
        self.wake.notify_one();
    }

    /// Fail every tracked send `adapter_down` (`Later`): the core re-queues them (§4).
    fn fail_all(&self) {
        let sends: Vec<Entry> = {
            let mut st = lock(&self.st);
            st.order.clear();
            st.sends.drain().map(|(_, e)| e).collect()
        };
        for e in sends {
            let _ = e.tx.try_send(e.down_event());
        }
    }

    /// End the live generation (exited, replaced, partitioned): sends fail, the link reads
    /// down, the verifier empties (a fresh one per run, §14).
    fn end_live(&self, state: AdapterState) {
        let was_up = {
            let mut st = lock(&self.st);
            st.live = None;
            st.verifier = BridgeVerifier::new();
            if !st.quit {
                st.state = state;
            }
            let up = st.link == Some(LinkState::Up);
            if up {
                st.link = Some(LinkState::Down);
            }
            up
        };
        self.fail_all();
        if was_up {
            self.push(Inbound::Link(LinkState::Down));
        }
    }

    async fn write(&self, io: &PipedIo, line: &str) -> bool {
        matches!(
            tokio::time::timeout(self.write_timeout, io.write_line(line)).await,
            Ok(Ok(()))
        )
    }

    /// Announce on request (§6 (3); module doc): `true` when no announce was asked of this
    /// run within [`LxmfConfig::announce_interval`] (monotonic), and the slot is taken.
    fn announce_due(&self, now: Instant) -> bool {
        let mut st = lock(&self.st);
        if st
            .announced_at
            .is_some_and(|t| now.saturating_duration_since(t) < self.announce_interval)
        {
            return false;
        }
        st.announced_at = Some(now);
        true
    }

    /// Write `announce {}` to the live run when [`Inner::announce_due`] allows it.
    async fn announce_on_request(&self, io: &PipedIo, why: &'static str) {
        if self.announce_due(Instant::now()) && !self.write(io, ANNOUNCE_LINE).await {
            log(format_args!(
                "an announce ({why}) could not be written to the helper"
            ));
        }
    }

    /// Fail one tracked send with `adapter_down` (`Later`; [`Entry::down_event`]).
    fn fail_one(&self, id: &str) {
        let e = {
            let mut st = lock(&self.st);
            st.order.retain(|x| x != id);
            st.sends.remove(id)
        };
        if let Some(e) = e {
            let _ = e.tx.try_send(e.down_event());
        }
    }

    fn count_noise(&self, why: Noise) {
        let n = self.noise.fetch_add(1, Ordering::SeqCst) + 1;
        if n.is_power_of_two() {
            log(format_args!(
                "dropped a malformed helper line ({}; {n} so far)",
                why.token()
            ));
        }
    }

    /// One validated line of the live generation `generation`.
    async fn line(&self, io: &PipedIo, generation: u64, line: Line) -> Phase {
        match line {
            // a second ready within one run
            Line::Ready(_) => self.count_noise(Noise::Field("ready")),
            Line::Link(s) => {
                lock(&self.st).link = Some(s);
                self.push(Inbound::Link(s));
                if s == LinkState::Partitioned {
                    // §6 "Start order": the helper would have become the instance; it exits
                    log(format_args!(
                        "the helper reported partitioned; sends fail adapter_down"
                    ));
                    self.end_live(AdapterState::Error("partitioned".into()));
                    return Phase::Ended;
                }
            }
            Line::Msg(m) => self.msg(*m),
            Line::State(s) => {
                if self.state(s) {
                    // §6 (3): a send parked for the peer's announce; let the peer learn us
                    self.announce_on_request(io, "a parked send").await;
                }
            }
            Line::Heard { dest, name } => self.heard(io, generation, dest, name).await,
        }
        Phase::Live
    }

    fn msg(&self, m: MsgLine) {
        let (account, ours) = {
            let st = lock(&self.st);
            let Some(account) = st.ready.as_ref().map(|r| r.dest.clone()) else {
                return;
            };
            (account, st.own.contains(&m.hash))
        };
        if ours {
            // §8 guard 2: one of our own sends (its hash seen before the core accepted it)
            self.echoes.fetch_add(1, Ordering::SeqCst);
            return;
        }
        if m.relay_refused {
            self.relay_refused.fetch_add(1, Ordering::SeqCst);
        }
        if m.fields_dropped > 0 {
            self.fields_dropped
                .fetch_add(m.fields_dropped, Ordering::SeqCst);
        }
        let conv = ConvRef {
            network: NETWORK_LXMF.into(),
            account,
            kind: ConvKind::Dm,
            native_id: format!("dm:{}", m.src),
        };
        let name = m
            .name
            .as_deref()
            .map(|n| ingest::cut(n, NAME_MAX).to_string())
            .filter(|n| !n.is_empty());
        if let Some(title) = &name {
            let fresh = {
                let mut st = lock(&self.st);
                if st.titles.get(&m.src) == Some(title) {
                    false
                } else {
                    if st.titles.len() >= TITLE_MEMORY {
                        st.titles.clear();
                    }
                    st.titles.insert(m.src.clone(), title.clone());
                    true
                }
            };
            if fresh {
                self.push(Inbound::Conversation {
                    conv: conv.clone(),
                    title: title.clone(),
                });
            }
        }
        let mut meta = Map::new();
        if let Some(why) = m.why_unsigned {
            meta.insert("why_unsigned".into(), Value::from(why));
        }
        if let Some(r) = &m.relay {
            meta.insert("relay".into(), r.json());
        }
        if m.attachments.0 > 0 {
            meta.insert(
                "attachments".into(),
                json!({ "count": m.attachments.0, "bytes": m.attachments.1 }),
            );
        }
        if m.truncated {
            meta.insert("helper_truncated".into(), Value::Bool(true));
        }
        if m.fields_truncated {
            meta.insert("fields_truncated".into(), Value::Bool(true));
        }
        self.push(Inbound::Text(InboundText {
            conv,
            sender_native: m.src,
            sender_name: name.unwrap_or_default(),
            auth: if m.signed { Auth::Signed } else { Auth::None },
            native_msg_id: m.hash,
            body: m.text,
            claim_ts: m.claim_ts,
            meta: Value::Object(meta),
        }));
    }

    /// One `state` line (§6 (4), (6); §4). The hash is kept from the first line that
    /// carries it (the helper reports it once the library packed the message, while the
    /// send may still be `queued`: OUTBOUND, nothing on air) and recorded for own-echo
    /// suppression at once; `Accepted` goes to the core only once the send is on air
    /// (`sent`/`relayed`/`delivered`), because the core reads any later failure of an
    /// accepted send as post-air and never re-sends it. A new hash after that (the
    /// helper's new library attempt of the same id) is accepted again once on air.
    /// Returns `true` when the admitted line parks the send (`queued`/`peer_unannounced`).
    fn state(&self, s: StateLine) -> bool {
        let mut st = lock(&self.st);
        let St { sends, own, .. } = &mut *st;
        let Some(e) = sends.get_mut(&s.id) else {
            return false; // not ours (an evicted or older run's id): nothing to report
        };
        if let Some(h) = &s.hash {
            own.record(h);
            if e.hash.as_deref() != Some(h) {
                e.hash = Some(h.clone());
            }
        }
        let admitted = admit(e.last, s.state, s.reason);
        // on air now: an admitted on-air state, or a repeated one of a send already on air
        // (a new attempt's `sent` after "transmitted, not confirmed")
        let aired = on_air(s.state) && (admitted || e.last.is_some_and(|(p, _)| on_air(p)));
        if aired && e.hash.is_some() && e.accepted != e.hash {
            e.accepted = e.hash.clone();
            if let Some(h) = &e.hash {
                let _ = e.tx.try_send(SendEvent::Accepted {
                    native_id: h.clone(),
                });
            }
        }
        if !admitted {
            return false;
        }
        let parked = s.state == S::Queued && s.reason == Some(HelperReason::PeerUnannounced);
        e.last = Some((s.state, s.reason));
        e.unconfirmed = s.reason == Some(HelperReason::Unconfirmed);
        let ev = events_for(&s);
        let _ = e.tx.try_send(ev.clone());
        let done = match ev {
            SendEvent::Failed { .. } => true,
            SendEvent::Evidence { state, .. } => state == S::Delivered,
            SendEvent::Accepted { .. } => false,
        };
        if done {
            st.sends.remove(&s.id);
            st.order.retain(|x| x != &s.id);
        }
        parked
    }

    async fn heard(&self, io: &PipedIo, generation: u64, dest: String, name: Option<String>) {
        let now = Instant::now();
        let (account, resend) = {
            let mut st = lock(&self.st);
            let Some(account) = st.ready.as_ref().map(|r| r.dest.clone()) else {
                return;
            };
            if dest == account {
                return;
            }
            if let Some(t) = st.heard_at.get(&dest) {
                if now.duration_since(*t) < self.heard_interval {
                    self.coalesced.fetch_add(1, Ordering::SeqCst);
                    return;
                }
            } else {
                st.heard_order.push_back(dest.clone());
                while st.heard_order.len() > HEARD_MEMORY {
                    if let Some(old) = st.heard_order.pop_front() {
                        st.heard_at.remove(&old);
                    }
                }
            }
            st.heard_at.insert(dest.clone(), now);
            let mut resend = Vec::new();
            for (id, e) in st.sends.iter_mut() {
                // kept set: a later heard writes it again (the helper ignores a send whose
                // attempt is still in flight and starts a new one once it concluded)
                if e.to == dest && e.unconfirmed && e.generation == generation && !e.tx.is_closed()
                {
                    resend.push((id.clone(), e.line.clone()));
                }
            }
            (account, resend)
        };
        let long = name
            .as_deref()
            .map(|n| ingest::cut(&ingest::clean(n), NEARBY_NAME_MAX).to_string())
            .unwrap_or_default();
        self.push_heard(&dest, nearby_source(&account, &dest, &long));
        // §6 (4): a post-air FAILED is `Later` on that peer's next heard
        for (id, line) in resend {
            if !self.write(io, &line).await {
                self.fail_one(&id);
            }
        }
    }
}

fn push_item(q: &mut Queue, item: Item) {
    if q.items.len() >= INBOUND_CAP {
        match q.items.pop_front() {
            Some(Item::Heard(d)) => {
                q.heard.remove(&d);
            }
            Some(Item::In(i)) if matches!(*i, Inbound::Text(_)) => q.dropped += 1,
            _ => {}
        }
    }
    q.items.push_back(item);
}

/// A `reticulum` `NearbySource` (Layer 5 §14 schema, Layer 6 kind): one node, the heard
/// destination; its name is untrusted text.
pub fn nearby_source(account: &str, dest: &str, long_name: &str) -> Value {
    json!({
        "id": format!("reticulum:{account}"),
        "kind": "reticulum",
        "label": "heard via Reticulum",
        "device_id": null,
        "nodes": [{
            "id": dest,
            "num": 0,
            "long_name": long_name,
            "short_name": &dest[..dest.len().min(8)],
            "pubkey": null,
            "snr": null,
            "rssi": null,
            "hops_away": null,
            "last_heard_hint": null,
            "position": null,
            "channel_utilization": null,
        }],
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    AwaitReady,
    Live,
    /// The first line was not `ready`, or the adapter was told to quit: lines are ignored.
    Refused,
    Ended,
}

struct Gen {
    io: PipedIo,
    phase: Phase,
    /// The ready line, skipped once when `recv_line` returns it.
    skip: Option<String>,
}

enum Ev {
    First(Option<String>),
    Line(Option<String>),
}

/// The driver (module doc): one task per enabled adapter.
async fn drive(
    inner: Arc<Inner>,
    mut stdio: watch::Receiver<Option<PipedIo>>,
    mut cmds: mpsc::Receiver<Cmd>,
) {
    let mut cur: Option<Gen> = None;
    let mut watching = true;
    loop {
        if watching {
            let latest = stdio.borrow_and_update().clone();
            reattach(&inner, &mut cur, latest).await;
        }
        let next = cur.as_ref().map(|g| (g.io.clone(), g.phase));
        let next = async move {
            match next {
                Some((io, Phase::AwaitReady)) => Ev::First(io.first_line().await),
                Some((io, Phase::Live)) => Ev::Line(io.recv_line().await),
                _ => std::future::pending().await,
            }
        };
        tokio::select! {
            r = stdio.changed(), if watching => {
                if r.is_err() {
                    // the runner is gone (disabled): nothing will run again
                    watching = false;
                    reattach(&inner, &mut cur, None).await;
                }
            }
            cmd = cmds.recv() => match cmd {
                Some(c) => command(&inner, &mut cur, c).await,
                None => break,
            },
            ev = next => event(&inner, &mut cur, ev).await,
        }
    }
    reattach(&inner, &mut cur, None).await;
}

/// Attach the watch's current io if it is a new generation (module doc).
async fn reattach(inner: &Inner, cur: &mut Option<Gen>, latest: Option<PipedIo>) {
    let new = latest.as_ref().map(PipedIo::generation);
    if cur.as_ref().map(|g| g.io.generation()) == new {
        return;
    }
    if let Some(mut old) = cur.take() {
        // what the ended run still wrote (a final state, `partitioned`) counts before it
        // ends: a send the helper already took is never failed as if it had not
        while old.phase == Phase::Live {
            let Some(line) = old.io.try_recv_line() else {
                break;
            };
            on_line(inner, &mut old, line).await;
        }
        *lock(&inner.io) = None;
        lock(&inner.st).io_noise += old.io.dropped_invalid();
        if old.phase == Phase::Live {
            inner.end_live(AdapterState::Error("adapter_down".into()));
        }
    }
    *lock(&inner.io) = latest.clone();
    let quitting = lock(&inner.st).quit;
    let Some(io) = latest else {
        inner.set_state(if quitting {
            AdapterState::Disabled
        } else {
            AdapterState::Error("adapter_down".into())
        });
        return;
    };
    if quitting {
        let _ = inner.write(&io, "{\"type\":\"quit\"}").await;
        *cur = Some(Gen {
            io,
            phase: Phase::Refused,
            skip: None,
        });
        return;
    }
    inner.set_state(AdapterState::Error("adapter_down".into()));
    *cur = Some(Gen {
        io,
        phase: Phase::AwaitReady,
        skip: None,
    });
}

async fn event(inner: &Inner, cur: &mut Option<Gen>, ev: Ev) {
    let Some(g) = cur.as_mut() else { return };
    let generation = g.io.generation();
    match ev {
        Ev::First(None) => g.phase = Phase::Ended,
        Ev::First(Some(first)) => match parse_line(&first) {
            Ok(Line::Ready(r)) => {
                let facts = ReadyFacts {
                    generation,
                    dest: r.dest,
                    pubkey: r.pubkey,
                    instance: r.instance,
                };
                {
                    let mut st = lock(&inner.st);
                    let mut v = BridgeVerifier::new();
                    // a fresh table holds one key: never full
                    let _ = v.add_reticulum_key(facts.pubkey);
                    st.verifier = v;
                    st.ready = Some(facts.clone());
                    st.live = Some(generation);
                    st.link = None;
                    st.state = AdapterState::Live;
                    // the announce bound is per helper run
                    st.announced_at = None;
                }
                g.phase = Phase::Live;
                g.skip = Some(first);
                log(format_args!("helper run {generation} is ready"));
                if let Some(hook) = &inner.on_ready {
                    hook(&facts);
                }
                // §6 (3): a new run (start or restart) announces so the peers relearn us
                inner.announce_on_request(&g.io, "a new run").await;
            }
            _ => {
                let n = inner.refused.fetch_add(1, Ordering::SeqCst) + 1;
                log(format_args!(
                    "helper run {generation}: the first line was not ready; refused ({n} so far)"
                ));
                g.phase = Phase::Refused;
                inner.set_state(AdapterState::Error("adapter_down".into()));
                // ask it to go: the supervisor respawns it and the next run is checked again
                let _ = inner.write(&g.io, "{\"type\":\"quit\"}").await;
            }
        },
        Ev::Line(None) => {
            g.phase = Phase::Ended;
            inner.end_live(AdapterState::Error("adapter_down".into()));
        }
        Ev::Line(Some(line)) => on_line(inner, g, line).await,
    }
}

/// One line of a live generation (the ready line itself is skipped once).
async fn on_line(inner: &Inner, g: &mut Gen, line: String) {
    if g.skip.take().as_deref() == Some(line.as_str()) {
        return;
    }
    match parse_line(&line) {
        Ok(l) => {
            let io = g.io.clone();
            g.phase = inner.line(&io, io.generation(), l).await;
        }
        Err(why) => inner.count_noise(why),
    }
}

async fn command(inner: &Inner, cur: &mut Option<Gen>, cmd: Cmd) {
    match cmd {
        Cmd::Line { line, send } => {
            let live = cur
                .as_ref()
                .filter(|g| g.phase == Phase::Live)
                .map(|g| (g.io.clone(), g.io.generation()));
            match (live, send) {
                (Some((io, generation)), Some(id)) => {
                    let ours = lock(&inner.st)
                        .sends
                        .get(&id)
                        .is_some_and(|e| e.generation == generation);
                    if ours && !inner.write(&io, &line).await {
                        log(format_args!("a send could not be written to the helper"));
                        inner.fail_one(&id);
                    }
                }
                (Some((io, _)), None) => {
                    if !inner.write(&io, &line).await {
                        log(format_args!("a command could not be written to the helper"));
                    }
                }
                (None, Some(id)) => inner.fail_one(&id),
                (None, None) => {}
            }
        }
        Cmd::Quit(ack) => {
            lock(&inner.st).quit = true;
            if let Some(g) = cur.as_mut() {
                if matches!(g.phase, Phase::Live | Phase::AwaitReady) {
                    let _ = inner.write(&g.io, "{\"type\":\"quit\"}").await;
                }
                if g.phase == Phase::Live {
                    inner.end_live(AdapterState::Disabled);
                }
                g.phase = Phase::Refused;
            }
            inner.fail_all();
            inner.set_state(AdapterState::Disabled);
            let _ = ack.send(());
        }
    }
}

impl Adapter for LxmfAdapter {
    fn id(&self) -> &'static str {
        NETWORK_LXMF
    }

    fn caps(&self) -> Caps {
        let mut c = self.inner.caps.clone();
        c.state = lock(&self.inner.st).state.clone();
        c
    }

    /// One account: this node's LXMF delivery destination (§8 guard 2); LXMF hashes are
    /// globally unique (§3 dedupe on the key alone); separate messages may arrive out of
    /// order, so the parts of a split send may too.
    fn facts(&self) -> AdapterFacts {
        let st = lock(&self.inner.st);
        AdapterFacts {
            accounts: st
                .ready
                .iter()
                .map(|r| AccountFacts {
                    id: r.dest.clone(),
                    label: None,
                    licensed: false,
                })
                .collect(),
            channels: Vec::new(),
            region_set: None,
            ids_globally_unique: true,
            parts_may_reorder: true,
            licensed_caps: None,
        }
    }

    /// Cancel-safe: an item leaves the queue only when it is returned.
    fn recv(&self) -> BoxFuture<'_, Inbound> {
        Box::pin(async move {
            loop {
                let notified = self.inner.wake.notified();
                {
                    let mut q = lock(&self.inner.queue);
                    let n = std::mem::take(&mut q.dropped);
                    if n > 0 {
                        return Inbound::Dropped { n };
                    }
                    while let Some(item) = q.items.pop_front() {
                        match item {
                            Item::In(i) => return *i,
                            Item::Heard(d) => {
                                if let Some(v) = q.heard.remove(&d) {
                                    return Inbound::Nearby(v);
                                }
                            }
                        }
                    }
                }
                notified.await;
            }
        })
    }

    /// One `send` line per call (a recipient list is the core's: one DM per member); the
    /// handle returns at once (§4).
    fn send(&self, conv: &ConvRef, text: &str, _o: SendOpts) -> BoxFuture<'_, SendHandle> {
        let (tx, rx) = mpsc::channel(EVENTS);
        let fail = |reason, retry| {
            let _ = tx.try_send(SendEvent::Failed { reason, retry });
        };
        let to = conv
            .native_id
            .strip_prefix("dm:")
            .filter(|d| is_hex(d, 32))
            .map(str::to_string);
        let Some(to) = to.filter(|_| conv.network == NETWORK_LXMF && conv.kind == ConvKind::Dm)
        else {
            fail(Reason::NoChannel, Retry::Never);
            return Box::pin(std::future::ready(SendHandle { events: rx }));
        };
        let n = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let id = format!("m{n}");
        let relay = relay_for(text, &self.inner.tags);
        let line = send_line(&id, &to, text, relay.as_ref());
        if line.len() > LINE_MAX {
            fail(Reason::TooLarge, Retry::Never);
            return Box::pin(std::future::ready(SendHandle { events: rx }));
        }
        let registered = {
            let mut st = lock(&self.inner.st);
            match (st.live, st.ready.as_ref().map(|r| r.dest.clone())) {
                (Some(generation), Some(account)) if !st.quit => {
                    if conv.account != account {
                        // a conversation of another identity of this node: never sent as this one
                        Err((Reason::NoChannel, Retry::Never))
                    } else if !make_room(&mut st) {
                        Err((Reason::AdapterDown, Retry::Later))
                    } else {
                        st.sends.insert(
                            id.clone(),
                            Entry {
                                tx: tx.clone(),
                                to,
                                line: line.clone(),
                                generation,
                                last: None,
                                hash: None,
                                accepted: None,
                                unconfirmed: false,
                            },
                        );
                        st.order.push_back(id.clone());
                        Ok(())
                    }
                }
                _ => Err((Reason::AdapterDown, Retry::Later)),
            }
        };
        match registered {
            Err((reason, retry)) => fail(reason, retry),
            Ok(()) => {
                let cmd = Cmd::Line {
                    line,
                    send: Some(id.clone()),
                };
                if self.inner.cmds.try_send(cmd).is_err() {
                    self.inner.fail_one(&id);
                }
            }
        }
        drop(tx);
        Box::pin(std::future::ready(SendHandle { events: rx }))
    }
}

/// A slot for one more tracked send: drop sends the core no longer listens to, then the
/// oldest "transmitted, not confirmed" one (its row keeps that state).
fn make_room(st: &mut St) -> bool {
    if st.sends.len() < MAX_SENDS {
        return true;
    }
    st.sends.retain(|_, e| !e.tx.is_closed());
    if st.sends.len() >= MAX_SENDS {
        if let Some(old) = st
            .order
            .iter()
            .find(|id| st.sends.get(*id).is_some_and(|e| e.unconfirmed))
            .cloned()
        {
            st.sends.remove(&old);
        }
    }
    let St { order, sends, .. } = &mut *st;
    order.retain(|id| sends.contains_key(id));
    st.sends.len() < MAX_SENDS
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEST: &str = "0123456789abcdef0123456789abcdef";
    const SRC: &str = "fedcba9876543210fedcba9876543210";

    fn h64(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    fn msg(fields: &str, extra: &str) -> String {
        format!(
            r#"{{"type":"msg","hash":"{}","src":"{SRC}","name":"Ann","text":"hi","claim_ts":1700000000.75,"signed":true,"why_unsigned":null,"fields":{fields}{extra}}}"#,
            h64('a')
        )
    }

    #[test]
    fn ready_lines_validate_every_field() {
        let pk = "ab".repeat(64);
        let ok = format!(r#"{{"type":"ready","dest":"{DEST}","pubkey":"{pk}","instance":"111"}}"#);
        match parse_line(&ok).unwrap() {
            Line::Ready(r) => {
                assert_eq!(r.dest, DEST);
                assert_eq!(r.pubkey, [0xab; 64]);
                assert_eq!(r.instance, "111");
            }
            other => panic!("{other:?}"),
        }
        for bad in [
            ok.replace(DEST, &DEST[..31]),
            ok.replace(DEST, &format!("{DEST}0")),
            ok.replace(DEST, &DEST.to_uppercase()),
            ok.replace(&pk, &pk[..126]),
            ok.replace("\"111\"", "\"\""),
            ok.replace("\"111\"", "\"a b\""),
            ok.replace("}", ",\"extra\":1}"),
            ok.replace(",\"instance\":\"111\"", ""),
        ] {
            assert!(parse_line(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn closed_types_and_framing() {
        assert_eq!(parse_line("not json"), Err(Noise::BadJson));
        assert_eq!(parse_line("[1]"), Err(Noise::NotObject));
        assert_eq!(parse_line(r#"{"type":"hello"}"#), Err(Noise::UnknownType));
        assert_eq!(parse_line(r#"{"type":7}"#), Err(Noise::UnknownType));
        assert_eq!(parse_line(&"x".repeat(LINE_MAX + 1)), Err(Noise::Oversize));
        let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
        assert!(parse_line(&deep).is_err(), "deep nesting never overflows");
        assert_eq!(
            parse_line(r#"{"type":"link","state":"up"}"#),
            Ok(Line::Link(LinkState::Up))
        );
        assert!(parse_line(r#"{"type":"link","state":"sideways"}"#).is_err());
        assert!(parse_line(&format!(r#"{{"type":"heard","dest":"{DEST}"}}"#)).is_err());
        assert_eq!(
            parse_line(&format!(
                r#"{{"type":"heard","dest":"{DEST}","name":null}}"#
            )),
            Ok(Line::Heard {
                dest: DEST.into(),
                name: None
            })
        );
    }

    #[test]
    fn msg_fields_relay_attachments_and_caps() {
        let f = r#"{"5":{"attachment":{"bytes":1200}},"9":[{"1":"x"}],"251":"111.relay","252":{"v":1},"relay":{"v":1,"net":"meshtastic","id":"W1AW7","mid":"00ff00ff00ff00ff00ff00ff00ff00ff"}}"#;
        let Line::Msg(m) = parse_line(&msg(f, "")).unwrap() else {
            panic!()
        };
        assert_eq!(m.attachments, (1, 1200));
        assert_eq!(
            m.relay,
            Some(RelayField {
                v: 1,
                net: "meshtastic".into(),
                id: "W1AW7".into(),
                mid: "00ff00ff00ff00ff00ff00ff00ff00ff".into()
            })
        );
        assert!(!m.fields_truncated && m.fields_dropped == 0);
        assert_eq!(m.claim_ts, Some(1_700_000_000));
        assert!(m.signed && m.why_unsigned.is_none());
        // the §8 alphabet refuses the entry, the message stays
        let f = r#"{"relay":{"v":1,"net":"meshtastic","id":"a/b","mid":"1"}}"#;
        let Line::Msg(m) = parse_line(&msg(f, "")).unwrap() else {
            panic!()
        };
        assert!(m.relay.is_none() && m.relay_refused);
        let f = r#"{"relay":{"v":1,"net":"elsewhere","id":"a","mid":"1"}}"#;
        let Line::Msg(m) = parse_line(&msg(f, "")).unwrap() else {
            panic!()
        };
        assert!(m.relay_refused);
        // shape errors are noise
        for f in [
            r#"{"relay":{"v":1,"net":"lxmf","id":"a"}}"#,
            r#"{"relay":"x"}"#,
            r#"{"x":1}"#,
            r#"[]"#,
        ] {
            assert!(parse_line(&msg(f, "")).is_err(), "{f}");
        }
        // depth 3 is the bound: fields, a container, a container
        let Line::Msg(m) = parse_line(&msg(r#"{"9":[[1]]}"#, "")).unwrap() else {
            panic!()
        };
        assert!(!m.fields_truncated && m.fields_dropped == 0);
        // a protocol breach inside a too-deep field is still noise
        let breach = format!(r#"{{"9":[[["{}"]]]}}"#, "y".repeat(FIELD_STR_MAX + 1));
        assert_eq!(parse_line(&msg(&breach, "")), Err(Noise::Fields));
        let long = format!(r#"{{"9":"{}"}}"#, "y".repeat(FIELD_STR_MAX + 1));
        assert_eq!(parse_line(&msg(&long, "")), Err(Noise::Fields));
        assert!(parse_line(&msg("{}", r#","truncated":true"#)).is_ok());
        assert!(parse_line(&msg("{}", r#","truncated":false"#)).is_err());
        let unsigned = msg("{}", "")
            .replace("\"signed\":true", "\"signed\":false")
            .replace(
                "\"why_unsigned\":null",
                "\"why_unsigned\":\"source_unknown\"",
            );
        let Line::Msg(m) = parse_line(&unsigned).unwrap() else {
            panic!()
        };
        assert_eq!(m.why_unsigned, Some("source_unknown"));
        assert!(parse_line(&unsigned.replace("source_unknown", "because")).is_err());
        let contradictory =
            msg("{}", "").replace("\"why_unsigned\":null", "\"why_unsigned\":\"unverified\"");
        assert!(parse_line(&contradictory).is_err());
        // a foreign timestamp out of range is unknown, a non-number is noise
        let far = msg("{}", "").replace("1700000000.75", "1e300");
        let Line::Msg(m) = parse_line(&far).unwrap() else {
            panic!()
        };
        assert_eq!(m.claim_ts, None);
        assert!(parse_line(&msg("{}", "").replace("1700000000.75", "\"now\"")).is_err());
        // a 33-hex source and a 32-hex message hash are noise
        assert!(parse_line(&msg("{}", "").replace(SRC, &format!("{SRC}0"))).is_err());
        assert!(parse_line(&msg("{}", "").replace(&h64('a'), &h64('a')[..32])).is_err());
    }

    /// F2 (review of C3rs): a received relay field has exactly the shape this adapter
    /// writes — `id` a §8 marker name (1..=10 bytes of the alphabet), `mid` 32 lowercase
    /// hex — or it is refused alone and the message kept.
    #[test]
    fn f2_received_relay_field_id_is_a_marker_name_and_mid_is_32_hex() {
        let mid = "0123456789abcdef0123456789abcdef";
        let relay = |id: &str, mid: &str| {
            let f = format!(
                r#"{{"relay":{{"v":1,"net":"meshtastic","id":{},"mid":{}}}}}"#,
                jstr(id),
                jstr(mid)
            );
            match parse_line(&msg(&f, "")) {
                Ok(Line::Msg(m)) => m,
                other => panic!("{f}: {other:?}"),
            }
        };
        let ok = relay("0123456789", mid);
        assert_eq!(ok.relay.as_ref().map(|r| r.id.as_str()), Some("0123456789"));
        assert!(!ok.relay_refused);
        // ten bytes at a UTF-8 boundary: five two-byte characters
        assert!(!relay("ééééé", mid).relay_refused);
        // the adapter's own field round-trips
        let tags = vec![("M".to_string(), "meshtastic".to_string())];
        let mine = relay_for("[W1AW7/M] hello", &tags).unwrap();
        assert_eq!(relay(&mine.id, &mine.mid).relay, Some(mine));
        for (id, m) in [
            ("01234567890", mid),                          // 11 bytes
            ("éééééé", mid),                               // 12 bytes
            ("a]b", mid),                                  // outside the alphabet
            ("a\u{200b}b", mid),                           // Cf
            ("W1AW7", "00ff"),                             // short
            ("W1AW7", &format!("{mid}0")),                 // 33 hex
            ("W1AW7", &mid.to_uppercase()),                // not lowercase
            ("W1AW7", "0123456789abcdef0123456789abcdeg"), // not hex
            ("W1AW7", &"x".repeat(64)),                    // the old 64-byte cap
        ] {
            let m = relay(id, m);
            assert!(m.relay.is_none() && m.relay_refused, "{id:?} {m:?}");
            assert_eq!(m.text, "hi", "the message is kept");
        }
    }

    /// F3 (review of C3rs, the owner question answered fail-safe): `FIELDS_DEPTH` stays 3;
    /// a field nested deeper is dropped alone, the message kept with `fields_truncated`.
    #[test]
    fn f3_a_field_nested_too_deep_is_dropped_alone_and_the_message_kept() {
        let f = r#"{"9":[[[1]]],"10":"kept","5":{"attachment":{"bytes":7}}}"#;
        let Line::Msg(m) = parse_line(&msg(f, "")).unwrap() else {
            panic!()
        };
        assert!(m.fields_truncated);
        assert_eq!(m.fields_dropped, 1);
        assert_eq!(
            (m.text.as_str(), m.hash.as_str()),
            ("hi", h64('a').as_str())
        );
        assert!(m.signed);
        assert_eq!(m.attachments, (1, 7), "the other fields still count");
        // a too-deep attachment-shaped field is dropped, not counted
        let f = r#"{"5":{"attachment":{"bytes":{"n":[1]}}}}"#;
        let Line::Msg(m) = parse_line(&msg(f, "")).unwrap() else {
            panic!()
        };
        assert_eq!((m.attachments, m.fields_dropped), ((0, 0), 1));
        // two offending fields, and the helper's own flag is kept
        let f = r#"{"9":{"a":{"b":{"c":1}}},"11":[[[[]]]]}"#;
        let Line::Msg(m) = parse_line(&msg(f, r#","fields_truncated":true"#)).unwrap() else {
            panic!()
        };
        assert!(m.fields_truncated);
        assert_eq!(m.fields_dropped, 2);
    }

    /// F1 (review of C3rs): an on-air state, never a `queued` one, makes the send accepted.
    #[test]
    fn f1_on_air_states_and_the_down_event() {
        assert!(!on_air(S::Queued) && !on_air(S::Failed));
        assert!(on_air(S::Sent) && on_air(S::Relayed) && on_air(S::Delivered));
        let (tx, _rx) = mpsc::channel(1);
        let mut e = Entry {
            tx,
            to: SRC.into(),
            line: String::new(),
            generation: 1,
            last: Some((S::Queued, None)),
            hash: Some(h64('c')),
            accepted: None,
            unconfirmed: false,
        };
        let later = |reason| SendEvent::Failed {
            reason,
            retry: Retry::Later,
        };
        assert_eq!(
            e.down_event(),
            later(Reason::AdapterDown),
            "pre-air: queued"
        );
        e.last = Some((S::Sent, None));
        e.accepted = e.hash.clone();
        assert_eq!(
            e.down_event(),
            later(Reason::AdapterDown),
            "accepted: post-air"
        );
        e.accepted = None;
        assert_eq!(
            e.down_event(),
            later(Reason::NoResponse),
            "on air without a hash: fail closed, never re-sent"
        );
        assert!(super::super::state::post_air(Reason::NoResponse));
        let mut own = OwnHashes::default();
        for i in 0..=MAX_SENDS {
            own.record(&format!("{i:064}"));
        }
        assert!(!own.contains(&format!("{:064}", 0)), "oldest forgotten");
        assert!(own.contains(&format!("{MAX_SENDS:064}")));
        assert_eq!(own.order.len(), MAX_SENDS);
    }

    #[test]
    fn state_lines_carry_closed_reasons_that_fit_their_state() {
        let ok = |s: &str| parse_line(s).unwrap();
        let h = h64('b');
        assert_eq!(
            ok(&format!(
                r#"{{"type":"state","id":"m1","hash":"{h}","state":"sent","reason":"transmitted, not confirmed"}}"#
            )),
            Line::State(StateLine {
                id: "m1".into(),
                hash: Some(h.clone()),
                state: S::Sent,
                reason: Some(HelperReason::Unconfirmed)
            })
        );
        for bad in [
            r#"{"type":"state","id":"m1","state":"failed"}"#,
            r#"{"type":"state","id":"m1","state":"sent","reason":"rejected"}"#,
            r#"{"type":"state","id":"m1","state":"queued","reason":"why not"}"#,
            r#"{"type":"state","id":"","state":"queued"}"#,
            r#"{"type":"state","id":"m1","state":"lost"}"#,
            r#"{"type":"state","id":"m1","hash":"00","state":"sent"}"#,
        ] {
            assert!(parse_line(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_monotone_rule_matrix() {
        use HelperReason::*;
        assert!(admit(None, S::Queued, None));
        assert!(admit(Some((S::Queued, None)), S::Sent, None));
        assert!(admit(
            Some((S::Queued, None)),
            S::Queued,
            Some(PeerUnannounced)
        ));
        assert!(!admit(
            Some((S::Queued, Some(PeerUnannounced))),
            S::Queued,
            Some(PeerUnannounced)
        ));
        assert!(!admit(Some((S::Sent, None)), S::Queued, None));
        assert!(admit(Some((S::Sent, None)), S::Sent, Some(Unconfirmed)));
        assert!(!admit(Some((S::Sent, Some(Unconfirmed))), S::Sent, None));
        assert!(admit(
            Some((S::Sent, Some(Unconfirmed))),
            S::Delivered,
            None
        ));
        assert!(admit(Some((S::Sent, None)), S::Failed, Some(Rejected)));
        assert!(!admit(Some((S::Relayed, None)), S::Failed, Some(Rejected)));
        assert!(!admit(
            Some((S::Delivered, None)),
            S::Failed,
            Some(Cancelled)
        ));
        assert!(!admit(Some((S::Delivered, None)), S::Sent, None));
        assert!(admit(Some((S::Failed, Some(Rejected))), S::Delivered, None));
        assert!(!admit(Some((S::Failed, Some(Rejected))), S::Sent, None));
    }

    #[test]
    fn state_mapping_to_send_events() {
        let st = |state, reason| StateLine {
            id: "m".into(),
            hash: None,
            state,
            reason,
        };
        assert_eq!(
            events_for(&st(S::Sent, Some(HelperReason::Unconfirmed))),
            SendEvent::Evidence {
                state: S::Sent,
                evidence: "transmitted, not confirmed".into()
            }
        );
        assert_eq!(
            events_for(&st(S::Queued, Some(HelperReason::PeerUnannounced))),
            SendEvent::Evidence {
                state: S::Queued,
                evidence: EV_UNANNOUNCED.into()
            }
        );
        assert_eq!(
            events_for(&st(S::Failed, Some(HelperReason::Rejected))),
            SendEvent::Failed {
                reason: Reason::NotAuthorized,
                retry: Retry::Never
            }
        );
        assert_eq!(
            events_for(&st(S::Failed, Some(HelperReason::HelperBusy))),
            SendEvent::Failed {
                reason: Reason::AdapterDown,
                retry: Retry::Later
            }
        );
    }

    #[test]
    fn send_lines_are_exact_and_relay_copies_carry_the_custom_field() {
        assert_eq!(
            send_line("m1", DEST, "a \"b\"\n", None),
            format!(r#"{{"type":"send","id":"m1","to":"{DEST}","text":"a \"b\"\n"}}"#)
        );
        let tags = vec![
            ("M".to_string(), "meshtastic".to_string()),
            ("LX".into(), "lxmf".into()),
        ];
        assert_eq!(relay_for("hello", &tags), None);
        assert_eq!(relay_for("[ann/XX] hello", &tags), None, "unregistered TAG");
        let r = relay_for("[W1AW7/M] (1/2) hello", &tags).unwrap();
        assert_eq!(
            (r.v, r.net.as_str(), r.id.as_str()),
            (1, "meshtastic", "W1AW7")
        );
        assert_eq!(r.mid.len(), 32);
        assert!(is_hex(&r.mid, 32));
        assert_eq!(
            r.mid,
            relay_for("[W1AW7/M] (1/2) hello", &tags).unwrap().mid
        );
        let line = send_line("m2", DEST, "[W1AW7/M] (1/2) hello", Some(&r));
        assert_eq!(
            line,
            format!(
                r#"{{"type":"send","id":"m2","to":"{DEST}","text":"[W1AW7/M] (1/2) hello","fields":{{"relay":{{"v":1,"net":"meshtastic","id":"W1AW7","mid":"{}"}}}}}}"#,
                r.mid
            )
        );
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["fields"]["relay"]["net"], "meshtastic");
    }

    #[test]
    fn run_args_carry_the_interval_and_a_cleaned_name() {
        let a = AnnounceRecord {
            name: Some("hub\u{202e}".into()),
            interval_min: 0,
        };
        assert_eq!(run_args(&a), ["--announce-interval", "0", "--name", "hub"]);
        let a = AnnounceRecord {
            name: None,
            interval_min: 60,
        };
        assert_eq!(run_args(&a), ["--announce-interval", "60"]);
    }

    #[test]
    fn nearby_sources_are_reticulum_with_untrusted_names() {
        let v = nearby_source(DEST, SRC, "Bob");
        assert_eq!(v["kind"], "reticulum");
        assert_eq!(v["device_id"], Value::Null);
        assert_eq!(v["nodes"][0]["id"], SRC);
        assert_eq!(v["nodes"][0]["num"], 0);
        assert_eq!(v["nodes"][0]["short_name"], &SRC[..8]);
        assert_eq!(v["nodes"][0]["last_heard_hint"], Value::Null);
    }

    #[test]
    fn config_reads_the_lxmf_manifest() {
        let c = LxmfConfig::default();
        assert_eq!(c.copy.tag, "LX");
        assert_eq!(c.recipient.as_ref().unwrap().pattern, "^[0-9a-f]{32}$");
        assert!(c.tags.iter().any(|(t, n)| t == "M" && n == "meshtastic"));
        assert!(c.tags.iter().any(|(t, n)| t == "LX" && n == "lxmf"));
    }
}
