//! `POST /api/v1/chat` · `GET /chat/history` · `GET /agent/status` ·
//! `POST /agent/task` — relay to `111-agent` (spec §10), plus the
//! `/v1/agent/events` subscriber that republishes agent activity on
//! `/api/v1/events` as `agent` + `proposal` frames (§10.1).
//!
//! ## Upstream
//!
//! The agent is reachable iff the daemon's mirror says its child is `running`
//! (`DashboardState.intelligence.agent`, refreshed every 2 s from the
//! `IntelligenceHandle`); any other state answers `503 agent_unavailable`
//! without dialing. The base URL is, in order: [`RelayConfig::base`] (tests point
//! it at a stub), `intelligence.agent_url` (= `/api/status.intelligence.agent_url`),
//! [`AGENT_URL`]. It must be `http://<loopback>:port` ([`http_async::Target`]).
//! Configure by inserting an `Arc<RelayConfig>` with [`ApiState::insert_ext`];
//! absent = [`RelayConfig::default`] (production).
//!
//! Budgets: connect 2 s. `history`, `status`, `task`: whole exchange 10 s.
//! `chat`: stream head 10 s, then the whole stream 10 min; `stream:false` gets
//! the 10 min budget for the whole exchange (a non-streamed turn on a small CPU
//! takes minutes, so the 10 s budget cannot apply to it).
//!
//! ## Error normalization (api/README.md "Errors")
//!
//! | upstream | `/api/v1` problem |
//! |---|---|
//! | connect refused / timeout, child not running | `503 agent_unavailable` |
//! | `503 {"error":"model unavailable"}` | `503 model_unavailable` |
//! | `400` (JSON `{error}`), `415`/`422` (plain text) | `400 bad_request`, upstream text as `detail` |
//! | `409 {"error":"needs hub"}` | `409 needs_hub` |
//! | any other status, non-JSON success, broken body | `502 upstream_error` |
//! | head/body budget exceeded | `504 upstream_error` |
//!
//! ## Chat stream translation
//!
//! Upstream frames are `data:` JSON lines; each is translated as soon as its
//! line is complete (nothing is buffered beyond one partial line):
//! `delta` → `event: token {text}` · `tool_call` → `event: tool {name}` ·
//! `done` → `event: done {conversation_id, reply, tool_trace}` and end ·
//! `error` → `event: error {error, code:"upstream_error"}` and end · `[DONE]` →
//! end · comment lines → `: keep-alive`. An upstream that closes (or exceeds
//! the 10 min budget) before a terminal frame yields `event: error
//! {"error":"stream ended"|"stream timed out","code":"upstream_error"}`. Unknown
//! frame types are skipped (additive upstream evolution). A pre-stream failure
//! (any upstream response that is not `200 text/event-stream`) is answered as
//! problem+json, never SSE. Dropping the response body (client gone) drops the
//! upstream connection, which makes `111-agent` abandon the turn.
//!
//! ## Events subscriber ([`start_events`])
//!
//! While the agent child is `running`, keep `GET /v1/agent/events` open; on
//! close or failure reconnect with backoff 1 s → 30 s (reset after a session
//! that delivered data); while the child is not running, poll its state every
//! 2 s without dialing. A connection that is silent for 45 s (three missed
//! upstream keep-alives) is dropped and redialed. Every new frame is published
//! verbatim as `event: agent`. Upstream replays its last 50 frames on every
//! connect, so frames are deduped on `seq` across reconnects ([`Dedupe`]; a
//! journal reset upstream is detected and resynced). For `proposal.written` —
//! and `task.done` carrying `detail.proposal_id`, since task-made proposals are
//! journaled only as `task.done` — the proposal file is read and published as
//! `event: proposal` (`{id,title,kind,created_at,decision}`), unless the frame
//! predates this subscriber (replayed history after a daemon restart).

use super::proposals;
use super::{ApiRouter, ApiState, Problem};
use crate::daemon::intelligence::{ChildDisplay, AGENT_URL};
use crate::util::http_async::{self, Target};
use axum::body::{Body, Bytes};
use axum::extract::{RawQuery, State};
use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::{Arc, Weak};
use std::time::Duration;
use time::OffsetDateTime;
use tokio::time::Instant;

/// Relay tuning (insert an `Arc<RelayConfig>` via [`ApiState::insert_ext`]).
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// Upstream base override (`http://127.0.0.1:port`); `None` follows
    /// `intelligence.agent_url`, then [`AGENT_URL`].
    pub base: Option<String>,
    pub connect_timeout: Duration,
    /// Whole exchange for non-streaming routes; head wait for `chat` streams.
    pub request_timeout: Duration,
    /// Whole `chat` stream (and the whole `stream:false` exchange).
    pub stream_timeout: Duration,
    /// Events subscriber reconnect backoff (doubling from min to max).
    pub backoff_min: Duration,
    pub backoff_max: Duration,
    /// Events connection silent this long is redialed.
    pub events_idle: Duration,
    /// Poll interval while the agent child is not running.
    pub absent_poll: Duration,
}

impl Default for RelayConfig {
    fn default() -> Self {
        RelayConfig {
            base: None,
            connect_timeout: Duration::from_secs(2),
            request_timeout: Duration::from_secs(10),
            stream_timeout: Duration::from_secs(600),
            backoff_min: Duration::from_secs(1),
            backoff_max: Duration::from_secs(30),
            events_idle: Duration::from_secs(45),
            absent_poll: Duration::from_secs(2),
        }
    }
}

fn config(api: &ApiState) -> Arc<RelayConfig> {
    api.ext::<Arc<RelayConfig>>().unwrap_or_default()
}

/// Largest upstream body read whole (history of a long conversation).
const MAX_BODY: usize = 8 << 20;
/// `ChatRequest.message` bounds (characters).
const MESSAGE_MAX: usize = 16_000;
/// Longest SSE line / event accepted from upstream.
const MAX_SSE: usize = 1 << 20;
/// Upstream replays this many journal frames on connect.
const UPSTREAM_REPLAY: u64 = 50;
/// `proposal` frames are not emitted for events older than subscriber start
/// minus this slack.
const FRESH_SLACK: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------- problems

fn agent_unavailable(detail: impl Into<String>) -> Problem {
    Problem::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "agent_unavailable",
        "The local AI agent is not reachable.",
    )
    .detail(detail)
}

fn model_unavailable(detail: impl Into<String>) -> Problem {
    Problem::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "model_unavailable",
        "The agent's language model is not running.",
    )
    .detail(detail)
}

fn bad_request(detail: impl Into<String>) -> Problem {
    Problem::new(
        StatusCode::BAD_REQUEST,
        "bad_request",
        "The request was refused.",
    )
    .detail(detail)
}

fn upstream_error(status: StatusCode, detail: impl Into<String>) -> Problem {
    Problem::new(status, "upstream_error", "The local AI agent failed.").detail(detail)
}

fn transport_problem(e: http_async::Error) -> Problem {
    match e {
        http_async::Error::Connect(d) => {
            agent_unavailable(format!("Cannot connect to 111-agent: {d}"))
        }
        http_async::Error::Timeout => upstream_error(
            StatusCode::GATEWAY_TIMEOUT,
            "111-agent did not answer in time.",
        ),
        http_async::Error::Protocol(d) => upstream_error(StatusCode::BAD_GATEWAY, d),
    }
}

/// Human text of an upstream error body: JSON `{error}` or the plain text.
fn upstream_text(body: &[u8]) -> String {
    let text = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| String::from_utf8_lossy(body).trim().to_string());
    let mut text: String = text.chars().take(500).collect();
    if text.is_empty() {
        text = "(empty response)".into();
    }
    text
}

/// Map a non-success upstream response to the contract envelope.
pub fn upstream_problem(status: StatusCode, body: &[u8]) -> Problem {
    let text = upstream_text(body);
    match status.as_u16() {
        503 if text == "model unavailable" => model_unavailable(format!("111-agent: {text}")),
        503 => agent_unavailable(format!("111-agent: {text}")),
        400 | 415 | 422 => bad_request(text),
        409 => Problem::new(
            StatusCode::CONFLICT,
            "needs_hub",
            "This task needs a hub node.",
        )
        .detail(text),
        _ => upstream_error(
            StatusCode::BAD_GATEWAY,
            format!("111-agent answered {}: {text}", status.as_u16()),
        ),
    }
}

// ---------------------------------------------------------------- upstream

/// The upstream to dial, or why there is none.
fn upstream(api: &ApiState, cfg: &RelayConfig) -> Result<Target, Problem> {
    let intel = api.dash.read().unwrap().intelligence.clone();
    let why = match intel.agent {
        ChildDisplay::Running => None,
        ChildDisplay::Absent => Some("111-agent is not installed on this node."),
        ChildDisplay::Backoff => Some("111-agent is restarting after a crash."),
        ChildDisplay::Degraded => Some("111-agent keeps crashing; its restarts are backed off."),
        ChildDisplay::Stopped => Some("111-agent was stopped."),
    };
    if let Some(why) = why {
        return Err(agent_unavailable(why));
    }
    let base = cfg
        .base
        .clone()
        .or(intel.agent_url)
        .unwrap_or_else(|| AGENT_URL.to_string());
    Target::parse(&base).map_err(|e| {
        Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "The agent relay is misconfigured.",
        )
        .detail(e)
    })
}

fn json_response(status: StatusCode, body: Vec<u8>) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// One request/response exchange within `budget`; a 2xx JSON body is passed
/// through verbatim (same status), anything else is normalized.
async fn exchange(
    api: &ApiState,
    method: Method,
    path: &str,
    body: Option<Vec<u8>>,
    budget: Duration,
) -> Result<Response, Problem> {
    let cfg = config(api);
    let target = upstream(api, &cfg)?;
    let deadline = Instant::now() + budget;
    let resp = http_async::send(&target, method, path, body, cfg.connect_timeout, deadline)
        .await
        .map_err(transport_problem)?;
    let status = resp.status;
    let media = resp.media_type();
    let bytes = resp
        .read_to_end(MAX_BODY, deadline)
        .await
        .map_err(transport_problem)?;
    if !status.is_success() {
        return Err(upstream_problem(status, &bytes));
    }
    let is_json = media.as_deref() == Some("application/json")
        && serde_json::from_slice::<serde::de::IgnoredAny>(&bytes).is_ok();
    if !is_json {
        return Err(upstream_error(
            StatusCode::BAD_GATEWAY,
            format!("111-agent sent a non-JSON {} reply.", status.as_u16()),
        ));
    }
    Ok(json_response(status, bytes))
}

fn respond(r: Result<Response, Problem>) -> Response {
    r.unwrap_or_else(IntoResponse::into_response)
}

// ---------------------------------------------------------------- validation

fn body_object(body: &[u8]) -> Result<serde_json::Map<String, Value>, Problem> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(bad_request("The body must be a JSON object.")),
        Err(e) => Err(bad_request(format!("The body is not valid JSON: {e}"))),
    }
}

/// A validated `ChatRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatRequest {
    pub conversation_id: String,
    pub message: String,
    pub stream: bool,
}

/// Validate against the contract `ChatRequest` (conversation id pattern,
/// message 1..=16000 characters and not blank, optional boolean `stream`).
pub fn parse_chat(body: &[u8]) -> Result<ChatRequest, Problem> {
    let m = body_object(body)?;
    let conversation_id = match m.get("conversation_id") {
        Some(Value::String(s)) if proposals::valid_id(s) => s.clone(),
        None | Some(Value::Null) => return Err(bad_request("conversation_id is required.")),
        Some(_) => {
            return Err(bad_request(
                "conversation_id must match ^[A-Za-z0-9_-]{1,128}$.",
            ))
        }
    };
    let message = match m.get("message") {
        Some(Value::String(s)) => s.clone(),
        None | Some(Value::Null) => return Err(bad_request("message is required.")),
        Some(_) => return Err(bad_request("message must be a string.")),
    };
    let chars = message.chars().count();
    if chars == 0 || chars > MESSAGE_MAX {
        return Err(bad_request(format!(
            "message must be 1 to {MESSAGE_MAX} characters (got {chars})."
        )));
    }
    if message.trim().is_empty() {
        return Err(bad_request("message must not be blank."));
    }
    let stream = match m.get("stream") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad_request("stream must be a boolean.")),
    };
    Ok(ChatRequest {
        conversation_id,
        message,
        stream,
    })
}

#[derive(Debug, Serialize)]
struct TaskRequest {
    kind: String,
    refs: Vec<String>,
    note: Option<String>,
}

fn parse_task(body: &[u8]) -> Result<TaskRequest, Problem> {
    let m = body_object(body)?;
    let kind = match m.get("kind") {
        Some(Value::String(k)) if matches!(k.as_str(), "diagnose" | "propose" | "patch") => {
            k.clone()
        }
        _ => return Err(bad_request("kind must be diagnose, propose or patch.")),
    };
    let refs = match m.get("refs") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|r| r.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| bad_request("refs must be an array of strings."))?,
        Some(_) => return Err(bad_request("refs must be an array of strings.")),
    };
    let note = match m.get("note") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err(bad_request("note must be a string or null.")),
    };
    Ok(TaskRequest { kind, refs, note })
}

// ---------------------------------------------------------------- SSE reader

/// One item from an upstream `text/event-stream`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseItem {
    /// A dispatched event's `data` (multiple `data:` lines joined by `\n`).
    Data(String),
    /// A comment line (`: keep-alive`).
    Comment,
}

/// Incremental `text/event-stream` reader. Only `data:` and comments matter
/// (`111-agent` sends no `event:`/`id:`); lines end in LF or CRLF. Lines are
/// decoded only once complete, so chunks may split anywhere (inside a JSON
/// value or a UTF-8 sequence).
#[derive(Debug, Default)]
pub struct SseReader {
    buf: Vec<u8>,
    data: Option<String>,
}

impl SseReader {
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<SseItem>, String> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        let mut start = 0;
        while let Some(nl) = self.buf[start..].iter().position(|&b| b == b'\n') {
            let end = start + nl;
            let mut line = &self.buf[start..end];
            start = end + 1;
            if let Some(l) = line.strip_suffix(b"\r") {
                line = l;
            }
            if line.is_empty() {
                if let Some(d) = self.data.take() {
                    out.push(SseItem::Data(d));
                }
                continue;
            }
            if line[0] == b':' {
                out.push(SseItem::Comment);
                continue;
            }
            let line = String::from_utf8_lossy(line);
            let (field, value) = match line.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (line.as_ref(), ""),
            };
            if field == "data" {
                let d = match &mut self.data {
                    Some(d) => {
                        d.push('\n');
                        d
                    }
                    None => self.data.insert(String::new()),
                };
                d.push_str(value);
                if d.len() > MAX_SSE {
                    return Err("upstream event too large".into());
                }
            }
        }
        self.buf.drain(..start);
        if self.buf.len() > MAX_SSE {
            return Err("upstream line too long".into());
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- chat stream

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ToolTrace {
    #[serde(default)]
    name: String,
    #[serde(default)]
    ok: bool,
}

fn sse_frame(event: &str, data: &impl Serialize) -> Bytes {
    let data = serde_json::to_string(data).unwrap_or_else(|_| "{}".into());
    Bytes::from(format!("event: {event}\ndata: {data}\n\n"))
}

/// `event: error` (`SseChatError`) frame.
pub fn chat_error_frame(error: &str) -> Bytes {
    #[derive(Serialize)]
    struct E<'a> {
        error: &'a str,
        code: &'static str,
    }
    sse_frame(
        "error",
        &E {
            error,
            code: "upstream_error",
        },
    )
}

const KEEPALIVE_FRAME: &[u8] = b": keep-alive\n\n";

/// Upstream chat SSE → contract chat SSE, frame by frame.
#[derive(Debug)]
pub struct ChatTranslator {
    sse: SseReader,
    conversation_id: String,
    terminal: bool,
}

impl ChatTranslator {
    /// `conversation_id` fills a `done` frame that lacks one.
    pub fn new(conversation_id: &str) -> Self {
        ChatTranslator {
            sse: SseReader::default(),
            conversation_id: conversation_id.to_string(),
            terminal: false,
        }
    }

    /// Saw `done`, `error`, `[DONE]` or a fatal parse error: nothing follows.
    pub fn is_terminal(&self) -> bool {
        self.terminal
    }

    /// Translate whatever complete frames `chunk` finishes.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Bytes> {
        let mut out = Vec::new();
        if self.terminal {
            return out;
        }
        let items = match self.sse.feed(chunk) {
            Ok(items) => items,
            Err(e) => {
                self.terminal = true;
                out.push(chat_error_frame(&e));
                return out;
            }
        };
        for item in items {
            match item {
                SseItem::Comment => out.push(Bytes::from_static(KEEPALIVE_FRAME)),
                SseItem::Data(d) if d.trim() == "[DONE]" => self.terminal = true,
                SseItem::Data(d) => out.extend(self.translate(&d)),
            }
            if self.terminal {
                break;
            }
        }
        out
    }

    fn translate(&mut self, data: &str) -> Option<Bytes> {
        let v: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("111 relay: skipping unreadable chat frame: {e}");
                return None;
            }
        };
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("");
        match s("type") {
            "delta" => Some(sse_frame("token", &json!({ "text": s("text") }))),
            "tool_call" => Some(sse_frame("tool", &json!({ "name": s("name") }))),
            "done" => {
                #[derive(Serialize)]
                struct Done<'a> {
                    conversation_id: &'a str,
                    reply: &'a str,
                    tool_trace: Vec<ToolTrace>,
                }
                self.terminal = true;
                let conversation_id = match s("conversation_id") {
                    "" => self.conversation_id.as_str(),
                    id => id,
                };
                let tool_trace = v
                    .get("tool_trace")
                    .cloned()
                    .and_then(|t| serde_json::from_value(t).ok())
                    .unwrap_or_default();
                Some(sse_frame(
                    "done",
                    &Done {
                        conversation_id,
                        reply: s("reply"),
                        tool_trace,
                    },
                ))
            }
            "error" => {
                self.terminal = true;
                let e = match s("error") {
                    "" => "the agent reported an error",
                    e => e,
                };
                Some(chat_error_frame(e))
            }
            _ => None, // unknown frame type: forward-compatible skip
        }
    }
}

/// Body stream state: upstream response + translator + ready output.
struct ChatBody {
    upstream: Option<http_async::Response>,
    tr: ChatTranslator,
    pending: VecDeque<Bytes>,
    deadline: Instant,
}

fn chat_stream(st: ChatBody) -> Body {
    let s = futures_util::stream::unfold(st, |mut st| async move {
        loop {
            if let Some(b) = st.pending.pop_front() {
                return Some((Ok::<_, Infallible>(b), st));
            }
            if st.tr.is_terminal() {
                return None;
            }
            let up = st.upstream.as_mut()?;
            match tokio::time::timeout_at(st.deadline, up.chunk()).await {
                Ok(Some(Ok(bytes))) => st.pending.extend(st.tr.feed(&bytes)),
                Ok(None) | Ok(Some(Err(_))) => {
                    st.pending.push_back(chat_error_frame("stream ended"));
                    st.tr.terminal = true;
                }
                Err(_) => {
                    st.pending.push_back(chat_error_frame("stream timed out"));
                    st.tr.terminal = true;
                }
            }
            if st.tr.is_terminal() {
                st.upstream = None; // close the upstream connection now
            }
        }
    });
    Body::from_stream(s)
}

async fn chat(State(api): State<Arc<ApiState>>, body: Bytes) -> Response {
    let req = match parse_chat(&body) {
        Ok(r) => r,
        Err(p) => return p.into_response(),
    };
    let fwd = serde_json::to_vec(&req).unwrap_or_default();
    let cfg = config(&api);
    if !req.stream {
        return respond(
            exchange(
                &api,
                Method::POST,
                "/v1/chat",
                Some(fwd),
                cfg.stream_timeout,
            )
            .await,
        );
    }
    let target = match upstream(&api, &cfg) {
        Ok(t) => t,
        Err(p) => return p.into_response(),
    };
    let start = Instant::now();
    let head_deadline = start + cfg.request_timeout;
    let resp = match http_async::send(
        &target,
        Method::POST,
        "/v1/chat",
        Some(fwd),
        cfg.connect_timeout,
        head_deadline,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return transport_problem(e).into_response(),
    };
    if resp.status != StatusCode::OK || resp.media_type().as_deref() != Some("text/event-stream") {
        let status = resp.status;
        return match resp.read_to_end(MAX_BODY, head_deadline).await {
            Err(e) => transport_problem(e).into_response(),
            Ok(_) if status.is_success() => upstream_error(
                StatusCode::BAD_GATEWAY,
                "111-agent did not stream the reply.",
            )
            .into_response(),
            Ok(b) => upstream_problem(status, &b).into_response(),
        };
    }
    let body = chat_stream(ChatBody {
        upstream: Some(resp),
        tr: ChatTranslator::new(&req.conversation_id),
        pending: VecDeque::new(),
        deadline: start + cfg.stream_timeout,
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("x-accel-buffering", "no")
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn history(State(api): State<Arc<ApiState>>, RawQuery(q): RawQuery) -> Response {
    let id = q
        .as_deref()
        .unwrap_or("")
        .split('&')
        .filter_map(|p| p.split_once('='))
        .find(|(k, _)| *k == "conversation_id")
        .map(|(_, v)| v);
    let id = match id {
        Some(id) if proposals::valid_id(id) => id,
        Some(_) => {
            return bad_request("conversation_id must match ^[A-Za-z0-9_-]{1,128}$.")
                .into_response()
        }
        None => return bad_request("conversation_id is required.").into_response(),
    };
    let cfg = config(&api);
    let path = format!("/v1/chat/history?conversation_id={id}");
    respond(exchange(&api, Method::GET, &path, None, cfg.request_timeout).await)
}

async fn agent_status(State(api): State<Arc<ApiState>>) -> Response {
    let cfg = config(&api);
    respond(
        exchange(
            &api,
            Method::GET,
            "/v1/agent/status",
            None,
            cfg.request_timeout,
        )
        .await,
    )
}

async fn agent_task(State(api): State<Arc<ApiState>>, body: Bytes) -> Response {
    let req = match parse_task(&body) {
        Ok(r) => r,
        Err(p) => return p.into_response(),
    };
    let cfg = config(&api);
    let fwd = serde_json::to_vec(&req).unwrap_or_default();
    respond(
        exchange(
            &api,
            Method::POST,
            "/v1/agent/task",
            Some(fwd),
            cfg.request_timeout,
        )
        .await,
    )
}

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/chat", post(chat))
        .route("/api/v1/chat/history", get(history))
        .route("/api/v1/agent/status", get(agent_status))
        .route("/api/v1/agent/task", post(agent_task))
}

// ---------------------------------------------------------------- events

/// Dedupe of upstream journal frames on `seq` across reconnects.
///
/// Within one connection upstream `seq` strictly increases; each reconnect
/// replays the last [`UPSTREAM_REPLAY`] frames. A frame is new iff its seq is
/// above the last one seen — unless the upstream journal was reset (state
/// wiped): a remembered seq arriving with a different `ts`, or the first frame
/// of a connection more than a replay window below the last seen seq, means
/// the journal restarted, and the dedupe resyncs from it.
#[derive(Debug, Default)]
pub struct Dedupe {
    last: Option<u64>,
    recent: VecDeque<(u64, String)>,
}

const DEDUPE_WINDOW: usize = 64;

impl Dedupe {
    /// Should this frame be republished? `first_on_conn`: it is the first
    /// frame of a fresh upstream connection.
    pub fn accept(&mut self, seq: u64, ts: &str, first_on_conn: bool) -> bool {
        if let Some(last) = self.last {
            if seq <= last {
                let reset = match self.recent.iter().find(|(s, _)| *s == seq) {
                    Some((_, seen)) => seen != ts,
                    None => first_on_conn && seq + UPSTREAM_REPLAY <= last,
                };
                if !reset {
                    return false;
                }
                eprintln!(
                    "111 relay: 111-agent event journal restarted (seq {seq} after {last}); resyncing"
                );
                self.recent.clear();
            }
        }
        self.last = Some(seq);
        self.recent.push_back((seq, ts.to_string()));
        if self.recent.len() > DEDUPE_WINDOW {
            self.recent.pop_front();
        }
        true
    }
}

/// Was this event emitted after the subscriber started (minus slack)?
/// Unparsable timestamps count as fresh.
fn is_fresh(ts: &str, started: OffsetDateTime) -> bool {
    match OffsetDateTime::parse(ts, &time::format_description::well_known::Rfc3339) {
        Ok(at) => at >= started - FRESH_SLACK,
        Err(_) => true,
    }
}

/// Handle one upstream events `data` payload.
fn on_agent_frame(
    api: &ApiState,
    data: &str,
    dedupe: &mut Dedupe,
    first_on_conn: &mut bool,
    started: OffsetDateTime,
) {
    let v: Value = match serde_json::from_str(data) {
        Ok(v @ Value::Object(_)) => v,
        _ => {
            eprintln!("111 relay: skipping unreadable agent event");
            return;
        }
    };
    let Some(seq) = v.get("seq").and_then(Value::as_u64) else {
        eprintln!("111 relay: skipping agent event without seq");
        return;
    };
    let ts = v.get("ts").and_then(Value::as_str).unwrap_or("");
    let first = std::mem::replace(first_on_conn, false);
    if !dedupe.accept(seq, ts, first) {
        return;
    }
    api.events.publish("agent", &v);
    let kind = v.get("kind").and_then(Value::as_str).unwrap_or("");
    let pid = v
        .get("detail")
        .and_then(|d| d.get("proposal_id"))
        .and_then(Value::as_str);
    let names_proposal = matches!(kind, "proposal.written" | "task.done");
    if let Some(pid) = pid.filter(|_| names_proposal && is_fresh(ts, started)) {
        match proposals::summary(&api.layout, pid) {
            Some(frame) => {
                api.events.publish("proposal", &frame);
            }
            None => eprintln!("111 relay: {kind} names proposal {pid:?}, but it is unreadable"),
        }
    }
}

/// One subscription until upstream closes (`Ok`) or fails. `connected` is set
/// once the stream head arrived (the backoff resets after such a session).
async fn events_session(
    api: &ApiState,
    target: &Target,
    cfg: &RelayConfig,
    dedupe: &mut Dedupe,
    started: OffsetDateTime,
    connected: &mut bool,
) -> Result<(), String> {
    let deadline = Instant::now() + cfg.request_timeout;
    let mut resp = http_async::send(
        target,
        Method::GET,
        "/v1/agent/events",
        None,
        cfg.connect_timeout,
        deadline,
    )
    .await
    .map_err(|e| e.to_string())?;
    if !resp.status.is_success() || resp.media_type().as_deref() != Some("text/event-stream") {
        return Err(format!("unexpected answer {}", resp.status.as_u16()));
    }
    *connected = true;
    let mut sse = SseReader::default();
    let mut first = true;
    loop {
        let chunk = match tokio::time::timeout(cfg.events_idle, resp.chunk()).await {
            Err(_) => return Err("silent too long".into()),
            Ok(None) => return Ok(()),
            Ok(Some(Err(e))) => return Err(e.to_string()),
            Ok(Some(Ok(b))) => b,
        };
        for item in sse.feed(&chunk)? {
            if let SseItem::Data(d) = item {
                on_agent_frame(api, &d, dedupe, &mut first, started);
            }
        }
    }
}

async fn events_loop(weak: Weak<ApiState>) {
    let started = OffsetDateTime::now_utc();
    let mut dedupe = Dedupe::default();
    let mut backoff: Option<Duration> = None;
    let mut last_err: Option<String> = None;
    loop {
        let Some(api) = weak.upgrade() else { return };
        let cfg = config(&api);
        let target = match upstream(&api, &cfg) {
            Ok(t) => t,
            Err(_) => {
                drop(api);
                backoff = None;
                tokio::time::sleep(cfg.absent_poll).await;
                continue;
            }
        };
        let mut connected = false;
        let outcome =
            events_session(&api, &target, &cfg, &mut dedupe, started, &mut connected).await;
        drop(api);
        match outcome {
            Err(e) if last_err.as_deref() != Some(e.as_str()) => {
                eprintln!("111 relay: agent events: {e}; reconnecting with backoff");
                last_err = Some(e);
            }
            Err(_) => {}
            Ok(()) => last_err = None,
        }
        let wait = match backoff {
            Some(b) if !connected => (b * 2).min(cfg.backoff_max),
            _ => cfg.backoff_min,
        };
        backoff = Some(wait);
        tokio::time::sleep(wait).await;
    }
}

/// Start the agent events subscriber (daemon hook in `daemon/mod.rs`, tests).
/// Holds only a weak reference between sessions; abort the handle to stop it.
pub fn start_events(api: &Arc<ApiState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(events_loop(Arc::downgrade(api)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(tr: &mut ChatTranslator, chunks: &[&[u8]]) -> String {
        let mut out = Vec::new();
        for c in chunks {
            for b in tr.feed(c) {
                out.extend_from_slice(&b);
            }
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn sse_reader_handles_splits_crlf_comments_and_multiline() {
        let mut r = SseReader::default();
        assert_eq!(r.feed(b"data: {\"a\"").unwrap(), vec![]);
        assert_eq!(
            r.feed(b":1}\r\n\r\n: keep-alive\n\ndata:x\ndata: y\n\n")
                .unwrap(),
            vec![
                SseItem::Data("{\"a\":1}".into()),
                SseItem::Comment,
                SseItem::Data("x\ny".into()),
            ]
        );
        // non-data fields and an empty data line
        assert_eq!(
            r.feed(b"event: e\nid: 3\nretry: 5\ndata\n\n").unwrap(),
            vec![SseItem::Data(String::new())]
        );
        // a UTF-8 sequence split across chunks decodes intact
        let s = "data: na\u{ef}ve\n\n".as_bytes();
        let cut = s.iter().position(|&b| b == 0xc3).unwrap() + 1;
        assert!(r.feed(&s[..cut]).unwrap().is_empty());
        assert_eq!(
            r.feed(&s[cut..]).unwrap(),
            vec![SseItem::Data("na\u{ef}ve".into())]
        );
        // blank lines without data dispatch nothing
        assert!(r.feed(b"\n\n\n").unwrap().is_empty());
    }

    #[test]
    fn sse_reader_caps_lines() {
        let mut r = SseReader::default();
        assert!(r.feed(&vec![b'x'; MAX_SSE + 1]).is_err());
    }

    #[test]
    fn translator_byte_exact() {
        let mut tr = ChatTranslator::new("conv-1");
        let out = feed_all(
            &mut tr,
            &[
                b"data: {\"type\":\"delta\",\"text\":\"Hel\"}\n\ndata: {\"type\":\"del",
                b"ta\",\"text\":\"lo\\n\"}\n\n:\n\n",
                b"data: {\"type\":\"tool_call\",\"name\":\"kb.read\"}\n\n",
                b"data: {\"type\":\"future\",\"x\":1}\n\ndata: not json\n\n",
                b"data: {\"type\":\"done\",\"reply\":\"Hello\",\"tool_trace\":[{\"name\":\"kb.read\",\"ok\":true}]}\n\n",
                b"data: {\"type\":\"delta\",\"text\":\"after\"}\n\n",
            ],
        );
        assert_eq!(
            out,
            "event: token\ndata: {\"text\":\"Hel\"}\n\n\
             event: token\ndata: {\"text\":\"lo\\n\"}\n\n\
             : keep-alive\n\n\
             event: tool\ndata: {\"name\":\"kb.read\"}\n\n\
             event: done\ndata: {\"conversation_id\":\"conv-1\",\"reply\":\"Hello\",\"tool_trace\":[{\"name\":\"kb.read\",\"ok\":true}]}\n\n"
        );
        assert!(tr.is_terminal());
    }

    #[test]
    fn translator_error_and_done_sentinel() {
        let mut tr = ChatTranslator::new("c");
        let out = feed_all(
            &mut tr,
            &[b"data: {\"type\":\"error\",\"error\":\"boom\"}\n\n"],
        );
        assert_eq!(
            out,
            "event: error\ndata: {\"error\":\"boom\",\"code\":\"upstream_error\"}\n\n"
        );
        assert!(tr.is_terminal());
        let mut tr = ChatTranslator::new("c");
        assert_eq!(feed_all(&mut tr, &[b"data: [DONE]\n\n"]), "");
        assert!(tr.is_terminal());
    }

    #[test]
    fn upstream_error_mapping() {
        let p = upstream_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            br#"{"schema":1,"error":"model unavailable"}"#,
        );
        assert_eq!((p.status, p.code), (503, "model_unavailable"));
        let p = upstream_problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            b"Failed to deserialize: missing field `message`",
        );
        assert_eq!((p.status, p.code), (400, "bad_request"));
        assert_eq!(
            p.detail.as_deref(),
            Some("Failed to deserialize: missing field `message`")
        );
        let p = upstream_problem(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            b"Expected request with `Content-Type: application/json`",
        );
        assert_eq!((p.status, p.code), (400, "bad_request"));
        let p = upstream_problem(
            StatusCode::BAD_REQUEST,
            br#"{"schema":1,"error":"invalid conversation_id"}"#,
        );
        assert_eq!(p.detail.as_deref(), Some("invalid conversation_id"));
        let p = upstream_problem(StatusCode::CONFLICT, br#"{"schema":1,"error":"needs hub"}"#);
        assert_eq!((p.status, p.code), (409, "needs_hub"));
        for s in [500u16, 502, 403, 404] {
            let p = upstream_problem(StatusCode::from_u16(s).unwrap(), b"{\"error\":\"x\"}");
            assert_eq!((p.status, p.code), (502, "upstream_error"), "{s}");
        }
        assert_eq!(
            transport_problem(http_async::Error::Connect("refused".into())).code,
            "agent_unavailable"
        );
        assert_eq!(transport_problem(http_async::Error::Timeout).status, 504);
    }

    #[test]
    fn chat_validation() {
        let ok = parse_chat(br#"{"conversation_id":"conv-1","message":"hi"}"#).unwrap();
        assert_eq!(
            ok,
            ChatRequest {
                conversation_id: "conv-1".into(),
                message: "hi".into(),
                stream: true
            }
        );
        assert!(
            !parse_chat(br#"{"conversation_id":"c","message":"hi","stream":false}"#)
                .unwrap()
                .stream
        );
        let max = format!(
            r#"{{"conversation_id":"c","message":"{}"}}"#,
            "\u{e9}".repeat(MESSAGE_MAX)
        );
        assert!(
            parse_chat(max.as_bytes()).is_ok(),
            "limit counts characters, not bytes"
        );
        let over = format!(
            r#"{{"conversation_id":"c","message":"{}"}}"#,
            "x".repeat(MESSAGE_MAX + 1)
        );
        for bad in [
            "not json".to_string(),
            "[1]".into(),
            r#"{"message":"hi"}"#.into(),
            r#"{"conversation_id":"a/b","message":"hi"}"#.into(),
            r#"{"conversation_id":7,"message":"hi"}"#.into(),
            r#"{"conversation_id":"c"}"#.into(),
            r#"{"conversation_id":"c","message":""}"#.into(),
            r#"{"conversation_id":"c","message":"  \n"}"#.into(),
            r#"{"conversation_id":"c","message":1}"#.into(),
            r#"{"conversation_id":"c","message":"hi","stream":"yes"}"#.into(),
            over,
        ] {
            assert_eq!(
                parse_chat(bad.as_bytes()).unwrap_err().code,
                "bad_request",
                "{bad}"
            );
        }
    }

    #[test]
    fn task_validation() {
        let t = parse_task(br#"{"kind":"diagnose","refs":["a"],"note":null,"x":1}"#).unwrap();
        assert_eq!(
            serde_json::to_value(&t).unwrap(),
            json!({"kind":"diagnose","refs":["a"],"note":null})
        );
        for bad in [
            r#"{"kind":"rm -rf"}"#,
            r#"{}"#,
            r#"{"kind":"patch","refs":[1]}"#,
            r#"{"kind":"patch","refs":"a"}"#,
            r#"{"kind":"patch","note":5}"#,
        ] {
            assert_eq!(
                parse_task(bad.as_bytes()).unwrap_err().code,
                "bad_request",
                "{bad}"
            );
        }
    }

    #[test]
    fn dedupe_across_reconnects_and_journal_reset() {
        let mut d = Dedupe::default();
        // first connection: 1, 2, 3
        assert!(d.accept(1, "t1", true));
        assert!(d.accept(2, "t2", false));
        assert!(d.accept(3, "t3", false));
        // reconnect replays 1..3, then 4 is new
        assert!(!d.accept(1, "t1", true));
        assert!(!d.accept(2, "t2", false));
        assert!(!d.accept(3, "t3", false));
        assert!(d.accept(4, "t4", false));
        // journal wiped upstream: seq 1 comes back with a new ts → resync
        assert!(d.accept(1, "t1-new", true));
        assert!(d.accept(2, "t2-new", false));
        assert!(!d.accept(2, "t2-new", true));

        // large journal reset: seq far below the last seen, outside the window
        let mut d = Dedupe::default();
        for s in 1..=500 {
            assert!(d.accept(s, &format!("t{s}"), s == 1));
        }
        assert!(!d.accept(451, "t451", true), "normal replay");
        assert!(!d.accept(420, "t420", false), "in-connection old frame");
        assert!(d.accept(3, "fresh", true), "far below on a new connection");
    }

    #[test]
    fn freshness() {
        let started = OffsetDateTime::parse(
            "2026-10-13T12:00:00Z",
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap();
        assert!(is_fresh("2026-10-13T12:00:01Z", started));
        assert!(is_fresh("2026-10-13T11:59:58Z", started), "within slack");
        assert!(!is_fresh("2026-10-13T11:00:00Z", started));
        assert!(is_fresh("garbage", started));
    }
}
