//! `GET /api/v1/proposals?status=open|decided|all` · `GET /proposals/{id}` ·
//! `POST /proposals/{id}/decision` (spec §10, `Proposal` / `Decision` schemas).
//!
//! Storage (all under the agent state dir `<root>/agent/`, 0700):
//! - `proposals/<id>.json` — written by `111-agent` (`{schema,id,node_id,task_id,
//!   kind,title,body,created_at}`, no status). Untrusted input: read-only here,
//!   size-capped, a file whose `id` differs from its name is skipped, an
//!   unparsable one is skipped with one stderr line. `kind` must be one of
//!   the contract's [`KINDS`]: an unknown kind is skipped in lists and
//!   events, and `GET /proposals/{id}` / a decision on it answer `502
//!   upstream_error` naming the value (the agent wrote something the
//!   contract does not allow).
//! - `decisions/<id>.json` — `{schema:1, decision, at}`, written here atomically
//!   ([`crate::util::fs::atomic_write`]); Layer 9 consumes it. A proposal is
//!   `decided` iff a valid decision file exists (a corrupt one is ignored and
//!   may be overwritten).
//!
//! Every `{id}` is checked against `^[A-Za-z0-9_-]{1,128}$` before any path is
//! built; an id that fails can name no proposal, so it answers `404`.
//!
//! **`kind: message`** (Layer 5 §17, task F2): accepting a message proposal
//! is applied, not just recorded — the A1 body contract (`"to: " + 64
//! lowercase hex + "\n\n" + text`) is parsed, the node-id stamp must name
//! this node, the recipient must be an accepted contact, then the fabric
//! queues the message ([`crate::daemon::fabric::proposal`]). The decision
//! file carries an additive `applied` field (the API's `decision` object
//! keeps its contract shape) and is written **twice**: first
//! `applied: {pending: true}` before the send, then `applied: {message_id}`
//! after the store committed the message. The record on disk is the guard:
//! a proposal whose file exists reads decided, so after a crash, a failed
//! finalise or a restart a retried accept answers `409 already_decided` — a
//! half-applied proposal stays visibly `pending` on disk, it is never sent a
//! second time. A refused send removes the pending record (the proposal
//! stays open; the user may dismiss it); the cheap checks run before
//! anything is written. Every decision on a message proposal (accept or
//! dismiss) is serialized under one async lock and refused to the agent's
//! bearer (`401`: deciding is the human's act, §17), so a proposal is never
//! sent twice and never sent after being dismissed.
//!
//! **Layer 6 kinds** (`os/6-transport/README.md` §8, §9, §12, §13; bodies
//! parsed by [`crate::daemon::fabric::proposal`]). `KINDS` grows to eight.
//! Accepting one is applied like `message`, under the same lock, with the
//! same record discipline (`applied: {pending: true}` first, the result
//! after, a refusal before anything ran removes the pending record and the
//! proposal stays open), and only after the proposal's node-id stamp names
//! this node:
//! - `bridge_message`: the targets (`fc-` ids, or 111 contacts by node id)
//!   and the verbatim text run the **same validation as `POST
//!   /messages/fanout`** (a preview first, nothing written). A consent gate
//!   (`ham_unconfirmed`, `uplink_off` on an uplinked channel) answers `409
//!   consent_required` with `consent` {text, hash} and the per-target report;
//!   the UI shows the text and repeats the decision with `accept_warning` =
//!   the hash, which records that confirmation (by this session) and the
//!   validation runs again (one gate per round). An oversize target, or a
//!   channel the user switched off, is `409` with the report. Then the send
//!   group is queued through the bridge core: `applied: {group_id, items}`.
//! - `bridge_edge`: the edge-enable round-trip of `POST /bridge/edges` (the
//!   warning as it reads now; `accept_warning` must echo its hash); `applied:
//!   {edge_id}`.
//! - `device`: the device core's applier (`devices::applier`, C4) takes the
//!   whole proposal — `ApplierRequest::from_body` (the closed vocabulary; a
//!   body that is not the one JSON object is `502 upstream_error`, an action
//!   outside it `409 unsupported_action`) and `devices::applier::apply`
//!   (every action dry-run first; the pending record, then per action
//!   snapshot → apply → probe → keep or revert; the one-shots `identify` —
//!   the acceptance is the consent — and `service.restart`, `409
//!   not_supervised` for a child 111d does not supervise). The applier writes
//!   the decision record itself; this file does not also decide it. Without
//!   an applier in ext (the device core not wired) a device proposal is `409
//!   unsupported_action` and stays open.
//!
//! The agent's bearer never decides (the global write gate, `401`).
//!
//! Events: a recorded decision publishes `event: proposal` with the
//! `SseProposal` shape plus the new `decision` (`{id,title,kind,created_at,
//! decision:{decision,at}}`) so other open tabs refetch. The relay's events
//! subscriber publishes the same shape with `decision: null` for a new file
//! ([`summary`]).

use super::{ApiRouter, ApiState, Problem};
use crate::daemon::components::consent::{ConsentRequired, ProblemWith};
use crate::daemon::DaemonLayout;
use axum::body::Bytes;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use time::OffsetDateTime;

/// List cap (newest first).
pub const MAX_LIST: usize = 200;
/// `Proposal.kind` (contract enum; Layer 6 adds three, §10).
pub const KINDS: [&str; 8] = [
    "diagnose",
    "propose",
    "patch",
    "chat",
    "message",
    "bridge_message",
    "bridge_edge",
    "device",
];
/// `Proposal.kind` of a message the agent proposes (§17).
pub const MESSAGE_KIND: &str = "message";
/// Layer 6 §9: a text into foreign conversations (and 111 contacts).
pub const BRIDGE_MESSAGE_KIND: &str = "bridge_message";
/// Layer 6 §8: a relay edge.
pub const BRIDGE_EDGE_KIND: &str = "bridge_edge";
/// Layer 6 §12: device actions (incl. the `identify` / `service.restart` one-shots).
pub const DEVICE_KIND: &str = "device";
/// Largest proposal/decision file read (bodies are model-written markdown).
const MAX_FILE: u64 = 1 << 20;

/// `^[A-Za-z0-9_-]{1,128}$` — proposal ids (`prop-…`), conversation ids.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn proposals_dir(layout: &DaemonLayout) -> PathBuf {
    layout.root.join("agent/proposals")
}

pub fn decisions_dir(layout: &DaemonLayout) -> PathBuf {
    layout.root.join("agent/decisions")
}

/// What `111-agent` writes (extra fields ignored; additive evolution).
#[derive(Debug, Clone, Deserialize)]
struct ProposalFile {
    #[serde(default = "schema_one")]
    schema: u32,
    id: String,
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    task_id: Option<String>,
    kind: String,
    title: String,
    body: String,
    created_at: String,
}

fn schema_one() -> u32 {
    1
}

/// A recorded decision (`decision` is `accepted` | `dismissed`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRec {
    pub decision: String,
    pub at: String,
}

/// The `Proposal` contract shape.
#[derive(Debug, Clone, Serialize)]
pub struct Proposal {
    pub schema: u32,
    pub id: String,
    pub node_id: Option<String>,
    pub task_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub created_at: String,
    pub decision: Option<DecisionRec>,
}

impl Proposal {
    /// The `event: proposal` payload (`SseProposal` + `decision`).
    pub fn event(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "kind": self.kind,
            "created_at": self.created_at,
            "decision": self.decision,
        })
    }

    fn sort_key(&self) -> (OffsetDateTime, &str) {
        let at = OffsetDateTime::parse(
            &self.created_at,
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap_or(OffsetDateTime::UNIX_EPOCH);
        (at, self.id.as_str())
    }
}

fn is_decision(d: &str) -> bool {
    matches!(d, "accepted" | "dismissed")
}

/// Read a small regular file (symlinks and oversize files refused).
fn read_capped(path: &std::path::Path) -> std::io::Result<Option<String>> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !meta.file_type().is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    if meta.len() > MAX_FILE {
        return Err(std::io::Error::other(format!(
            "larger than {MAX_FILE} bytes"
        )));
    }
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(MAX_FILE)
        .read_to_string(&mut text)?;
    Ok(Some(text))
}

/// The decision for `id`, if a valid one is on disk.
fn read_decision(layout: &DaemonLayout, id: &str) -> Option<DecisionRec> {
    let path = decisions_dir(layout).join(format!("{id}.json"));
    let text = match read_capped(&path) {
        Ok(t) => t?,
        Err(e) => {
            eprintln!("111 proposals: ignoring decision {}: {e}", path.display());
            return None;
        }
    };
    match serde_json::from_str::<DecisionRec>(&text) {
        Ok(d) if is_decision(&d.decision) => Some(d),
        Ok(d) => {
            eprintln!(
                "111 proposals: ignoring decision {}: unknown decision {:?}",
                path.display(),
                d.decision
            );
            None
        }
        Err(e) => {
            eprintln!("111 proposals: ignoring decision {}: {e}", path.display());
            None
        }
    }
}

/// Why a present proposal file cannot be served.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Unusable {
    /// Unreadable, unparsable, or misnamed.
    Broken(String),
    /// Parsed, but `kind` is not in [`KINDS`] (the value, verbatim).
    Kind(String),
}

impl std::fmt::Display for Unusable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unusable::Broken(e) => f.write_str(e),
            Unusable::Kind(k) => write!(f, "unknown kind {k:?}"),
        }
    }
}

/// `Ok(None)` = no such file; `Err` = present but unusable (logged by callers).
fn read_proposal(layout: &DaemonLayout, id: &str) -> Result<Option<Proposal>, Unusable> {
    if !valid_id(id) {
        return Ok(None);
    }
    let path = proposals_dir(layout).join(format!("{id}.json"));
    let Some(text) = read_capped(&path).map_err(|e| Unusable::Broken(e.to_string()))? else {
        return Ok(None);
    };
    let f: ProposalFile =
        serde_json::from_str(&text).map_err(|e| Unusable::Broken(e.to_string()))?;
    if f.id != id {
        return Err(Unusable::Broken(format!(
            "its id {:?} does not match the file name",
            f.id
        )));
    }
    if !KINDS.contains(&f.kind.as_str()) {
        return Err(Unusable::Kind(f.kind));
    }
    Ok(Some(Proposal {
        decision: read_decision(layout, id),
        schema: f.schema,
        id: f.id,
        node_id: f.node_id,
        task_id: f.task_id,
        kind: f.kind,
        title: f.title,
        body: f.body,
        created_at: f.created_at,
    }))
}

/// One proposal by id (unusable files are logged and read as absent).
pub fn get_one(layout: &DaemonLayout, id: &str) -> Option<Proposal> {
    match read_proposal(layout, id) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("111 proposals: skipping {id}.json: {e}");
            None
        }
    }
}

/// One proposal for a route: unknown kind → `502 upstream_error` naming it;
/// otherwise as [`get_one`] (`None` = 404).
fn get_checked(layout: &DaemonLayout, id: &str) -> Result<Option<Proposal>, Problem> {
    match read_proposal(layout, id) {
        Ok(p) => Ok(p),
        Err(Unusable::Kind(kind)) => {
            eprintln!("111 proposals: {id}.json has unknown kind {kind:?}");
            Err(bad_kind(id, &kind))
        }
        Err(e) => {
            eprintln!("111 proposals: skipping {id}.json: {e}");
            Ok(None)
        }
    }
}

fn bad_kind(id: &str, kind: &str) -> Problem {
    Problem::new(
        StatusCode::BAD_GATEWAY,
        "upstream_error",
        "The local AI agent wrote a proposal this dashboard cannot show.",
    )
    .detail(format!(
        "proposal {id} has kind {kind:?}; expected one of {}",
        KINDS.join(", ")
    ))
}

/// The `event: proposal` payload for `id` (events subscriber), if readable.
pub fn summary(layout: &DaemonLayout, id: &str) -> Option<Value> {
    get_one(layout, id).map(|p| p.event())
}

/// List filter (`status` query).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusFilter {
    Open,
    Decided,
    All,
}

impl StatusFilter {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(StatusFilter::Open),
            "decided" => Some(StatusFilter::Decided),
            "all" => Some(StatusFilter::All),
            _ => None,
        }
    }

    fn keeps(self, p: &Proposal) -> bool {
        match self {
            StatusFilter::Open => p.decision.is_none(),
            StatusFilter::Decided => p.decision.is_some(),
            StatusFilter::All => true,
        }
    }
}

/// Proposals on disk matching `filter`, newest first by `created_at` (then id),
/// at most [`MAX_LIST`]. A missing directory is an empty list.
pub fn list(layout: &DaemonLayout, filter: StatusFilter) -> Vec<Proposal> {
    let Ok(rd) = std::fs::read_dir(proposals_dir(layout)) else {
        return Vec::new();
    };
    let mut out: Vec<Proposal> = rd
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let id = name.strip_suffix(".json")?;
            // Temp files (`.x.tmp-…`) and anything else that is not an id.
            valid_id(id).then(|| id.to_string())
        })
        .filter_map(|id| get_one(layout, &id))
        .filter(|p| filter.keeps(p))
        .collect();
    out.sort_by(|a, b| b.sort_key().cmp(&a.sort_key()));
    out.truncate(MAX_LIST);
    out
}

fn rfc3339_now() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

/// Serializes check-then-write so two racing decisions cannot both win.
static DECIDE_LOCK: Mutex<()> = Mutex::new(());

/// Record `decision` for `id` (`accepted` | `dismissed`). 404 when no proposal,
/// 409 `already_decided` when a valid decision exists.
pub fn decide(layout: &DaemonLayout, id: &str, decision: &str) -> Result<Proposal, Problem> {
    decide_applied(layout, id, decision, None)
}

/// [`decide`], with an `applied` record written into the decision file
/// (message proposals: `{"message_id": "msg-…"}`).
pub fn decide_applied(
    layout: &DaemonLayout,
    id: &str,
    decision: &str,
    applied: Option<Value>,
) -> Result<Proposal, Problem> {
    let _guard = DECIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut p) = get_checked(layout, id)? else {
        return Err(not_found());
    };
    if let Some(d) = &p.decision {
        return Err(Problem::new(
            StatusCode::CONFLICT,
            "already_decided",
            "This proposal was already decided.",
        )
        .detail(format!("It was marked {} at {}.", d.decision, d.at)));
    }
    let rec = DecisionRec {
        decision: decision.to_string(),
        at: rfc3339_now(),
    };
    if let Err(e) = write_decision(layout, id, &rec, applied.as_ref()) {
        eprintln!("111 proposals: cannot record decision for {id}: {e:#}");
        return Err(Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "The decision could not be saved.",
        )
        .detail(format!("{e:#}")));
    }
    p.decision = Some(rec);
    Ok(p)
}

/// Write `decisions/<id>.json` (`{schema, decision, at, applied?}`; the dir
/// 0700, the file atomic 0600).
fn write_decision(
    layout: &DaemonLayout,
    id: &str,
    rec: &DecisionRec,
    applied: Option<&Value>,
) -> anyhow::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let dir = decisions_dir(layout);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    let mut body = json!({"schema": 1, "decision": rec.decision, "at": rec.at});
    if let Some(a) = applied {
        body["applied"] = a.clone();
    }
    crate::util::fs::atomic_write(
        &dir.join(format!("{id}.json")),
        format!("{body}\n").as_bytes(),
        0o600,
    )
}

fn not_found() -> Problem {
    Problem::new(StatusCode::NOT_FOUND, "not_found", "No such proposal.")
}

fn bad_request(detail: &str) -> Problem {
    Problem::new(
        StatusCode::BAD_REQUEST,
        "bad_request",
        "The request was refused.",
    )
    .detail(detail)
}

fn internal(e: impl std::fmt::Display) -> Problem {
    Problem::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
        "The proposals could not be read.",
    )
    .detail(e.to_string())
}

/// `status` from a raw query string (`None` = absent).
fn status_param(q: Option<&str>) -> Result<StatusFilter, Problem> {
    let mut status = StatusFilter::Open;
    for pair in q.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == "status" {
            status = StatusFilter::parse(v)
                .ok_or_else(|| bad_request("status must be open, decided or all."))?;
        }
    }
    Ok(status)
}

async fn list_handler(State(api): State<Arc<ApiState>>, RawQuery(q): RawQuery) -> Response {
    let filter = match status_param(q.as_deref()) {
        Ok(f) => f,
        Err(p) => return p.into_response(),
    };
    let layout = api.layout.clone();
    match tokio::task::spawn_blocking(move || list(&layout, filter)).await {
        Ok(proposals) => Json(json!({"schema": 1, "proposals": proposals})).into_response(),
        Err(e) => internal(e).into_response(),
    }
}

async fn get_handler(
    State(api): State<Arc<ApiState>>,
    id: Result<Path<String>, PathRejection>,
) -> Response {
    let Ok(Path(id)) = id else {
        return not_found().into_response();
    };
    if !valid_id(&id) {
        return not_found().into_response();
    }
    let layout = api.layout.clone();
    match tokio::task::spawn_blocking(move || get_checked(&layout, &id)).await {
        Ok(Ok(Some(p))) => Json(p).into_response(),
        Ok(Ok(None)) => not_found().into_response(),
        Ok(Err(problem)) => problem.into_response(),
        Err(e) => internal(e).into_response(),
    }
}

/// One async lock for every decision on an applied proposal (`message` and the Layer 6
/// kinds; module doc).
static MESSAGE_DECIDE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// `applied` while a message acceptance is in flight (module doc).
pub fn applied_pending() -> Value {
    json!({ "pending": true })
}

/// A decision on a `kind: message` proposal (module doc).
async fn decide_message(
    api: &ApiState,
    p: Proposal,
    decision: String,
) -> Result<Proposal, Problem> {
    let _one = MESSAGE_DECIDE.lock().await;
    let layout = api.layout.clone();
    let id = p.id.clone();
    // Re-read under the lock: another request may have decided it.
    let current = {
        let (layout, id) = (layout.clone(), id.clone());
        tokio::task::spawn_blocking(move || get_checked(&layout, &id))
            .await
            .map_err(internal)??
    };
    let Some(p) = current else {
        return Err(not_found());
    };
    if p.decision.is_some() || decision != "accepted" {
        let d = decision.clone();
        return tokio::task::spawn_blocking(move || decide(&layout, &id, &d))
            .await
            .map_err(internal)?;
    }
    use crate::daemon::fabric::proposal;
    // checks 1–3 first: nothing is written for a proposal that cannot go
    let handle = super::messages::handle(api)?;
    let (to, text) = proposal::prepare(&handle, p.node_id.as_deref(), &p.body).await?;
    // the pending record: from here a retry is `already_decided`
    let accepted = {
        let (layout, id) = (layout.clone(), id.clone());
        tokio::task::spawn_blocking(move || {
            decide_applied(&layout, &id, "accepted", Some(applied_pending()))
        })
        .await
        .map_err(internal)??
    };
    let Some(rec) = accepted.decision.clone() else {
        return Err(internal("the acceptance was not recorded"));
    };
    let n = match proposal::send(&handle, to, &text).await {
        Ok(n) => n,
        Err(problem) => {
            // refused: nothing was sent, so the proposal reopens
            let path = decisions_dir(&layout).join(format!("{id}.json"));
            if let Err(e) = std::fs::remove_file(&path) {
                eprintln!(
                    "111 proposals: {id} was refused ({}) but its pending record {} stays: {e}",
                    problem.code,
                    path.display()
                );
            }
            return Err(problem);
        }
    };
    let message_id = crate::daemon::fabric::json::msg_id(n);
    let applied = json!({ "message_id": message_id });
    let (l2, id2) = (layout.clone(), id.clone());
    let finalised =
        tokio::task::spawn_blocking(move || write_decision(&l2, &id2, &rec, Some(&applied)))
            .await
            .map_err(anyhow::Error::from)
            .and_then(|r| r);
    if let Err(e) = finalised {
        // sent once and recorded as accepted: the record keeps saying
        // `pending`, and a retry is still `already_decided`
        eprintln!(
            "111 proposals: {id} was sent as {message_id}; its decision record still reads pending: {e:#}"
        );
    }
    Ok(accepted)
}

#[derive(Deserialize)]
struct DecisionReq {
    decision: Value,
    /// Layer 6: the hash of the consent text the user was shown (absent on the first
    /// call).
    #[serde(default)]
    accept_warning: Option<Value>,
}

async fn decision_handler(
    State(api): State<Arc<ApiState>>,
    id: Result<Path<String>, PathRejection>,
    headers: axum::http::HeaderMap,
    ext: axum::http::Extensions,
    body: Bytes,
) -> Response {
    let Ok(Path(id)) = id else {
        return not_found().into_response();
    };
    if !valid_id(&id) {
        return not_found().into_response();
    }
    let (decision, accept) = match serde_json::from_slice::<DecisionReq>(&body) {
        Ok(DecisionReq {
            decision: Value::String(d),
            accept_warning,
        }) if is_decision(&d) => match accept_warning {
            None | Some(Value::Null) => (d, None),
            Some(Value::String(h)) if super::bridge::check_hash(Some(&h)).is_ok() => (d, Some(h)),
            Some(_) => {
                return bad_request(
                    "accept_warning must be the 64-hex SHA-256 the consent carried.",
                )
                .into_response()
            }
        },
        _ => {
            return bad_request("Body must be {\"decision\": \"accepted\" | \"dismissed\"}.")
                .into_response()
        }
    };
    let layout = api.layout.clone();
    // An applied proposal is applied on acceptance (module doc). The agent's bearer
    // never reaches this handler on a POST (the global write gate, §13).
    let peek = {
        let (layout, id) = (layout.clone(), id.clone());
        tokio::task::spawn_blocking(move || get_checked(&layout, &id)).await
    };
    if let Ok(Ok(Some(p))) = peek {
        if p.kind == MESSAGE_KIND {
            return match decide_message(&api, p, decision).await {
                Ok(p) => {
                    api.events.publish("proposal", &p.event());
                    Json(p).into_response()
                }
                Err(problem) => problem.into_response(),
            };
        }
        if [BRIDGE_MESSAGE_KIND, BRIDGE_EDGE_KIND, DEVICE_KIND].contains(&p.kind.as_str()) {
            let by = super::auth::session_label(&api, &headers, &ext);
            return match decide_layer6(&api, p, decision, accept, by).await {
                Ok(p) => {
                    api.events.publish("proposal", &p.event());
                    Json(p).into_response()
                }
                Err(problem) => problem.into_response(),
            };
        }
    }
    match tokio::task::spawn_blocking(move || decide(&layout, &id, &decision)).await {
        Ok(Ok(p)) => {
            api.events.publish("proposal", &p.event());
            Json(p).into_response()
        }
        Ok(Err(problem)) => problem.into_response(),
        Err(e) => internal(e).into_response(),
    }
}

// ---------------------------------------------------------------- Layer 6 kinds

/// `409 unsupported_action` (the proposal stays open).
fn unsupported(action: &str, why: &str) -> ProblemWith {
    ProblemWith::new(
        Problem::new(
            StatusCode::CONFLICT,
            "unsupported_action",
            "This node does not apply that action.",
        )
        .detail(format!("{action:?}: {why}")),
    )
}

fn upstream(r: &crate::daemon::fabric::proposal::Refusal) -> ProblemWith {
    ProblemWith::new(r.problem())
}

/// The stamp rule of the applied kinds (a proposal written for another node — a copied
/// agent dir — is never applied here).
fn check_own_stamp(api: &ApiState, p: &Proposal) -> Result<(), ProblemWith> {
    let own = api
        .dash
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .node_id
        .clone();
    match (own, p.node_id.as_deref()) {
        (Some(own), Some(stamp)) if own == stamp => Ok(()),
        _ => Err(upstream(&crate::daemon::fabric::proposal::Refusal::Stamp)),
    }
}

/// Write the pending record (from here a retry is `already_decided`).
async fn record_pending(layout: &DaemonLayout, id: &str) -> Result<Proposal, ProblemWith> {
    let (layout, id) = (layout.clone(), id.to_string());
    let p = tokio::task::spawn_blocking(move || {
        decide_applied(&layout, &id, "accepted", Some(applied_pending()))
    })
    .await
    .map_err(|e| ProblemWith::new(internal(e)))?
    .map_err(ProblemWith::new)?;
    Ok(p)
}

/// A refusal before anything ran: the proposal reopens.
fn reopen(layout: &DaemonLayout, id: &str, why: &str) {
    let path = decisions_dir(layout).join(format!("{id}.json"));
    if let Err(e) = std::fs::remove_file(&path) {
        eprintln!(
            "111 proposals: {id} was refused ({why}) but its pending record {} stays: {e}",
            path.display()
        );
    }
}

/// Replace the pending record with the outcome.
async fn finalise(layout: &DaemonLayout, accepted: &Proposal, applied: Value) {
    let Some(rec) = accepted.decision.clone() else {
        return;
    };
    let (l2, id2) = (layout.clone(), accepted.id.clone());
    let done = tokio::task::spawn_blocking(move || write_decision(&l2, &id2, &rec, Some(&applied)))
        .await
        .map_err(anyhow::Error::from)
        .and_then(|r| r);
    if let Err(e) = done {
        eprintln!(
            "111 proposals: {} was applied; its decision record still reads pending: {e:#}",
            accepted.id
        );
    }
}

/// A decision on a Layer 6 kind (module doc).
async fn decide_layer6(
    api: &ApiState,
    p: Proposal,
    decision: String,
    accept: Option<String>,
    by: String,
) -> Result<Proposal, ProblemWith> {
    let _one = MESSAGE_DECIDE.lock().await;
    let layout = api.layout.clone();
    let id = p.id.clone();
    let current = {
        let (layout, id) = (layout.clone(), id.clone());
        tokio::task::spawn_blocking(move || get_checked(&layout, &id))
            .await
            .map_err(|e| ProblemWith::new(internal(e)))?
            .map_err(ProblemWith::new)?
    };
    let Some(p) = current else {
        return Err(ProblemWith::new(not_found()));
    };
    if p.decision.is_some() || decision != "accepted" {
        let d = decision.clone();
        return tokio::task::spawn_blocking(move || decide(&layout, &id, &d))
            .await
            .map_err(|e| ProblemWith::new(internal(e)))?
            .map_err(ProblemWith::new);
    }
    check_own_stamp(api, &p)?;
    match p.kind.as_str() {
        BRIDGE_MESSAGE_KIND => accept_bridge_message(api, &p, accept, &by).await,
        BRIDGE_EDGE_KIND => accept_bridge_edge(api, &p, accept, &by).await,
        _ => accept_device(api, &p).await,
    }
}

/// The 409 of a preview whose targets are refused (`targets` report; `consent` = the
/// first gate that can be confirmed).
fn refused_preview(preview: &Value) -> Option<ProblemWith> {
    let mut targets = Vec::new();
    let mut first_consent: Option<Value> = None;
    let mut all_oversize = true;
    for item in preview["items"].as_array().into_iter().flatten() {
        let r = &item["plan"]["refusal"];
        if r.is_null() {
            continue;
        }
        let code = if r["code"] == "oversize" {
            "oversize"
        } else {
            all_oversize = false;
            "consent_required"
        };
        if first_consent.is_none() && !r["consent"].is_null() {
            first_consent = Some(r["consent"].clone());
        }
        targets.push(json!({
            "target": item["target"],
            "code": code,
            "detail": r["detail"],
            "consent": r["consent"],
        }));
    }
    if targets.is_empty() {
        return None;
    }
    let (code, title, detail) = match (&first_consent, all_oversize) {
        (Some(_), _) => (
            "consent_required",
            "Read and confirm the warning first.",
            "Show the text, then repeat the decision with accept_warning = its hash; nothing was queued.",
        ),
        (None, true) => (
            "oversize",
            "The text is longer than a target carries.",
            "See targets; nothing was queued. Dismiss the proposal or ask for a shorter text.",
        ),
        (None, false) => (
            "consent_required",
            "Sending is switched off for a target.",
            "See targets: turn sending on in Settings, then accept again; nothing was queued.",
        ),
    };
    let mut pw = ProblemWith::new(Problem::new(StatusCode::CONFLICT, code, title).detail(detail))
        .with("targets", Value::Array(targets));
    if let Some(c) = first_consent {
        pw = pw.with("consent", c);
    }
    Some(pw)
}

/// Record the confirmation `c` (a gate of a `bridge_message`) as this session's.
async fn confirm_gate(
    bridge: &crate::daemon::bridge::Bridge,
    c: &Value,
    by: &str,
) -> Result<(), ProblemWith> {
    let hash = c["hash"].as_str().unwrap_or_default();
    let subject = c["subject"].as_str().unwrap_or_default();
    let done = match c["action"].as_str() {
        Some("ham_confirm") => bridge.confirm_ham(subject, true, Some(hash), by).await,
        Some("uplink_send") => {
            let Some(conv) = crate::daemon::fabric::json::parse_fc_id(subject) else {
                return Err(ProblemWith::new(not_found()));
            };
            bridge.set_uplink_send(conv, true, Some(hash), by).await
        }
        _ => return Err(ProblemWith::new(internal("an unknown consent gate"))),
    };
    done.map_err(|e| e.problem())
}

async fn accept_bridge_message(
    api: &ApiState,
    p: &Proposal,
    accept: Option<String>,
    by: &str,
) -> Result<Proposal, ProblemWith> {
    use crate::daemon::bridge::fanout::{
        ContactTarget, FanoutRequest, FanoutTarget, ForeignTarget,
    };
    use crate::daemon::fabric::proposal::{parse_bridge_message, BridgeTarget};
    let (targets, text) = parse_bridge_message(&p.body).map_err(|r| upstream(&r))?;
    let bridge = super::bridge::bridge_of(api).map_err(ProblemWith::new)?;
    let handle = super::messages::handle(api).map_err(ProblemWith::new)?;
    let fab = handle
        .fabric()
        .cloned()
        .ok_or_else(|| ProblemWith::new(crate::daemon::fabric::not_running()))?;
    let mut list = Vec::with_capacity(targets.len());
    for t in targets {
        list.push(match t {
            BridgeTarget::Conv(id) => {
                let c = fab.foreign_conversation(id).await.map_err(|e| match e {
                    mesh111::fabric::FabricError::NotFound => ProblemWith::new(
                        Problem::new(StatusCode::NOT_FOUND, "not_found", "No such conversation.")
                            .detail(format!(
                                "{} names no conversation of this node.",
                                crate::daemon::fabric::json::fc_id(id)
                            )),
                    ),
                    other => ProblemWith::new(crate::daemon::fabric::problem(&other)),
                })?;
                FanoutTarget::Foreign(ForeignTarget {
                    network: c.network,
                    conv: crate::daemon::fabric::json::fc_id(id),
                    size_mode: None,
                })
            }
            BridgeTarget::Contact(n) => FanoutTarget::Contact(ContactTarget {
                node_id: crate::daemon::fabric::json::hex(&n),
            }),
        });
    }
    let req = FanoutRequest {
        text: text.to_string(),
        targets: list,
        size_mode: None,
    };
    // the fan-out validation, nothing written; one confirmable gate per round
    let mut accept = accept;
    loop {
        let preview = bridge.fanout_preview(&req).await.map_err(|e| e.problem())?;
        let Some(refused) = refused_preview(&preview) else {
            break;
        };
        let gate = accept.take().and_then(|h| {
            preview["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| &i["plan"]["refusal"]["consent"])
                .find(|c| c["hash"].as_str() == Some(h.as_str()))
                .cloned()
        });
        match gate {
            Some(c) => confirm_gate(&bridge, &c, by).await?,
            None => return Err(refused),
        }
    }
    let accepted = record_pending(&api.layout, &p.id).await?;
    let sent = match bridge.fanout(&req).await {
        Ok(v) => v,
        Err(e) => {
            reopen(&api.layout, &p.id, "the send was refused");
            return Err(e.problem());
        }
    };
    let items: Vec<Value> = sent["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|i| json!({ "target": i["target"], "id": i["id"] }))
        .collect();
    finalise(
        &api.layout,
        &accepted,
        json!({ "group_id": sent["group_id"], "items": items }),
    )
    .await;
    Ok(accepted)
}

async fn accept_bridge_edge(
    api: &ApiState,
    p: &Proposal,
    accept: Option<String>,
    by: &str,
) -> Result<Proposal, ProblemWith> {
    use crate::daemon::bridge::edges::EdgeOpts;
    use crate::daemon::bridge::BridgeError;
    let (src, dst, _why) =
        crate::daemon::fabric::proposal::parse_bridge_edge(&p.body).map_err(|r| upstream(&r))?;
    let bridge = super::bridge::bridge_of(api).map_err(ProblemWith::new)?;
    // the warning as it reads now (nothing is created without its hash)
    let consent: ConsentRequired = match bridge
        .create_edge(src, dst, EdgeOpts::default(), None, by)
        .await
    {
        Err(BridgeError::Consent(c)) => c,
        Err(e) => return Err(e.problem()),
        Ok(v) => {
            // cannot happen (no hash matches `None`); record it as applied
            let accepted = record_pending(&api.layout, &p.id).await?;
            finalise(&api.layout, &accepted, json!({ "edge_id": v["id"] })).await;
            return Ok(accepted);
        }
    };
    if accept.as_deref() != Some(consent.hash.as_str()) {
        return Err(BridgeError::Consent(consent).problem());
    }
    let accepted = record_pending(&api.layout, &p.id).await?;
    match bridge
        .create_edge(src, dst, EdgeOpts::default(), Some(&consent.hash), by)
        .await
    {
        Ok(v) => {
            finalise(&api.layout, &accepted, json!({ "edge_id": v["id"] })).await;
            Ok(accepted)
        }
        Err(e) => {
            reopen(&api.layout, &p.id, "the edge was refused");
            Err(e.problem())
        }
    }
}

/// A `device` proposal: the device core's applier decides and records it (module doc).
async fn accept_device(api: &ApiState, p: &Proposal) -> Result<Proposal, ProblemWith> {
    use crate::daemon::devices::applier::{self, Applier, ApplierRequest};
    let Some(ap) = api.ext::<Arc<Applier>>() else {
        return Err(unsupported(
            "device",
            "the device core's apply, probe and revert loop is not wired on this node",
        ));
    };
    let req = ApplierRequest::from_body(&p.id, &p.body).map_err(|e| e.problem())?;
    applier::apply(&ap, req).await.map_err(|e| e.problem())?;
    let (layout, id) = (api.layout.clone(), p.id.clone());
    tokio::task::spawn_blocking(move || get_checked(&layout, &id))
        .await
        .map_err(|e| ProblemWith::new(internal(e)))?
        .map_err(ProblemWith::new)?
        .ok_or_else(|| ProblemWith::new(not_found()))
}

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/proposals", get(list_handler))
        .route("/api/v1/proposals/{id}", get(get_handler))
        .route("/api/v1/proposals/{id}/decision", post(decision_handler))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(root: &std::path::Path) -> DaemonLayout {
        DaemonLayout {
            root: root.to_path_buf(),
            etc: root.join("etc"),
        }
    }

    fn write_prop(l: &DaemonLayout, id: &str, created_at: &str) {
        let dir = proposals_dir(l);
        std::fs::create_dir_all(&dir).unwrap();
        let body = json!({
            "schema": 1, "id": id, "node_id": null, "task_id": null,
            "kind": "chat", "title": format!("t-{id}"), "body": "b", "created_at": created_at
        });
        std::fs::write(dir.join(format!("{id}.json")), body.to_string()).unwrap();
    }

    #[test]
    fn id_rule() {
        assert!(valid_id("prop-18f2a-0001"));
        assert!(valid_id("a_B-9"));
        assert!(valid_id(&"x".repeat(128)));
        for bad in ["", "../x", "a.b", "a/b", "a b", "é", &"x".repeat(129)] {
            assert!(!valid_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn list_sorts_filters_and_skips_junk() {
        let dir = tempfile::tempdir().unwrap();
        let l = layout(dir.path());
        assert!(list(&l, StatusFilter::All).is_empty(), "missing dir");
        write_prop(&l, "prop-a", "2026-10-01T10:00:00Z");
        write_prop(&l, "prop-b", "2026-10-02T10:00:00.5Z");
        write_prop(&l, "prop-c", "not a date");
        let pdir = proposals_dir(&l);
        std::fs::write(pdir.join("broken.json"), "{nope").unwrap();
        std::fs::write(pdir.join(".prop-z.json.tmp-1-0"), "{}").unwrap();
        std::fs::write(pdir.join("notes.txt"), "x").unwrap();
        // id inside does not match the file name → skipped
        std::fs::write(
            pdir.join("prop-x.json"),
            json!({"id": "prop-y", "kind": "chat", "title": "t", "body": "b", "created_at": "2026-10-03T00:00:00Z"}).to_string(),
        )
        .unwrap();
        let ids: Vec<_> = list(&l, StatusFilter::All)
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, ["prop-b", "prop-a", "prop-c"]);

        decide(&l, "prop-a", "dismissed").unwrap();
        let open: Vec<_> = list(&l, StatusFilter::Open)
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(open, ["prop-b", "prop-c"]);
        let decided = list(&l, StatusFilter::Decided);
        assert_eq!(decided.len(), 1);
        assert_eq!(decided[0].decision.as_ref().unwrap().decision, "dismissed");

        // second decision conflicts; unknown proposal is 404
        assert_eq!(
            decide(&l, "prop-a", "accepted").unwrap_err().code,
            "already_decided"
        );
        assert_eq!(
            decide(&l, "prop-nope", "accepted").unwrap_err().code,
            "not_found"
        );
        assert_eq!(
            decide(&l, "../prop-a", "accepted").unwrap_err().code,
            "not_found"
        );

        // a corrupt decision file reads as open and may be overwritten
        std::fs::write(decisions_dir(&l).join("prop-b.json"), "garbage").unwrap();
        assert!(get_one(&l, "prop-b").unwrap().decision.is_none());
        assert_eq!(
            decide(&l, "prop-b", "accepted")
                .unwrap()
                .decision
                .unwrap()
                .decision,
            "accepted"
        );
    }

    #[test]
    fn unknown_kind_is_an_upstream_error() {
        let dir = tempfile::tempdir().unwrap();
        let l = layout(dir.path());
        let pdir = proposals_dir(&l);
        std::fs::create_dir_all(&pdir).unwrap();
        for kind in KINDS {
            let id = format!("prop-{kind}");
            std::fs::write(
                pdir.join(format!("{id}.json")),
                json!({"id": id, "kind": kind, "title": "t", "body": "b",
                       "created_at": "2026-10-01T10:00:00Z"})
                .to_string(),
            )
            .unwrap();
            assert_eq!(get_checked(&l, &id).unwrap().unwrap().kind, kind);
        }
        std::fs::write(
            pdir.join("prop-evil.json"),
            json!({"id": "prop-evil", "kind": "rm -rf /", "title": "t", "body": "b",
                   "created_at": "2026-10-02T10:00:00Z"})
            .to_string(),
        )
        .unwrap();
        let p = get_checked(&l, "prop-evil").unwrap_err();
        assert_eq!((p.status, p.code), (502, "upstream_error"));
        assert!(
            p.detail.as_deref().unwrap().contains("\"rm -rf /\""),
            "{p:?}"
        );
        // a decision on it is refused the same way; nothing is written
        let p = decide(&l, "prop-evil", "accepted").unwrap_err();
        assert_eq!((p.status, p.code), (502, "upstream_error"));
        assert!(!decisions_dir(&l).join("prop-evil.json").exists());
        // lists and events skip it; the valid ones still show
        let ids: Vec<_> = list(&l, StatusFilter::All)
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids.len(), KINDS.len());
        assert!(!ids.contains(&"prop-evil".to_string()));
        assert!(summary(&l, "prop-evil").is_none());
    }

    #[test]
    fn list_caps_at_max() {
        let dir = tempfile::tempdir().unwrap();
        let l = layout(dir.path());
        for i in 0..(MAX_LIST + 5) {
            write_prop(&l, &format!("prop-{i:04}"), "2026-10-01T10:00:00Z");
        }
        let all = list(&l, StatusFilter::All);
        assert_eq!(all.len(), MAX_LIST);
        // same timestamp → id descending
        assert_eq!(all[0].id, format!("prop-{:04}", MAX_LIST + 4));
    }

    #[test]
    fn status_query_parsing() {
        assert_eq!(status_param(None).unwrap(), StatusFilter::Open);
        assert_eq!(status_param(Some("status=all")).unwrap(), StatusFilter::All);
        assert_eq!(
            status_param(Some("x=1&status=decided")).unwrap(),
            StatusFilter::Decided
        );
        assert_eq!(
            status_param(Some("status=bogus")).unwrap_err().code,
            "bad_request"
        );
    }
}
