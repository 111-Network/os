//! OpenAI-compatible model client (blocking ureq; callers use spawn_blocking).
//! Talks to llama-server locally, or to a hub's endpoint when offloaded.
//! Plain HTTP only — endpoints are loopback/private fabric; encryption is
//! Layer 5's job.
//!
//! Error taxonomy (distinguishable in the message text):
//! `model unreachable: …` for transport failures (refused, timeout, DNS),
//! `model http status <code>: <snippet>` for non-2xx responses (snippet is
//! the response body truncated to 200 chars), and `malformed model
//! response: …` for bad SSE framing or JSON.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// One message in the OpenAI chat-completions shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }
}

/// A tool call requested by the model (OpenAI tool-calling protocol).
///
/// In memory the arguments are parsed JSON; on the wire (an assistant message
/// re-sent to the server) this serializes to the OpenAI shape
/// `{id, type:"function", function:{name, arguments:"<json string>"}}` —
/// a real llama-server rejects anything else.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
struct WireToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: WireFunction,
}

#[derive(Serialize, Deserialize)]
struct WireFunction {
    name: String,
    arguments: String,
}

impl Serialize for ToolCall {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        WireToolCall {
            id: self.id.clone(),
            kind: "function".to_string(),
            function: WireFunction {
                name: self.name.clone(),
                arguments: serde_json::to_string(&self.arguments)
                    .map_err(serde::ser::Error::custom)?,
            },
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for ToolCall {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = WireToolCall::deserialize(d)?;
        let arguments = if w.function.arguments.trim().is_empty() {
            serde_json::Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str(&w.function.arguments).map_err(serde::de::Error::custom)?
        };
        Ok(ToolCall {
            id: w.id,
            name: w.function.name,
            arguments,
        })
    }
}

/// What a completed chat turn returned.
#[derive(Debug, Default)]
pub struct Reply {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

/// Default whole-call timeout; tests shrink it via `Client::with_timeout`.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
/// Health probes stay snappy even when the main timeout is large.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Hard caps on what a model stream may make us buffer. Exceeding any of
/// these is a (malformed-class) error, never a panic.
const MAX_REPLY_TEXT: usize = 4 * 1024 * 1024;
const MAX_SSE_LINE: usize = 1024 * 1024;
const MAX_TOOL_ARGS: usize = 1024 * 1024;

#[derive(Clone)]
pub struct Client {
    base: String,
    model_id: String,
    agent: ureq::Agent,
    probe_agent: ureq::Agent,
}

/// One agent config, shared by chat calls and health probes; only the
/// timeout differs. Status codes are classified by hand (the error needs a
/// body snippet), so ureq must not map non-2xx to `Err` body-less.
fn agent_with_timeout(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .build()
        .into()
}

impl Client {
    pub fn new(base: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self::with_timeout(base, model_id, DEFAULT_TIMEOUT)
    }

    pub fn with_timeout(
        base: impl Into<String>,
        model_id: impl Into<String>,
        timeout: Duration,
    ) -> Self {
        Self {
            base: base.into(),
            model_id: model_id.into(),
            agent: agent_with_timeout(timeout),
            probe_agent: agent_with_timeout(timeout.min(PROBE_TIMEOUT)),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// llama-server `/health`: 200 when a model is loaded and serving.
    /// Any non-2xx or connection error counts as unreachable (honest).
    /// The probe uses its own short (≤2s) timeout, never the chat timeout.
    pub fn reachable(&self) -> bool {
        let url = format!("{}/health", self.base.trim_end_matches('/'));
        match self.probe_agent.get(&url).call() {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// Streaming chat completion with the OpenAI tool-calling protocol.
    /// `on_delta` receives each non-empty text delta as it arrives; the
    /// returned `Reply` holds the full concatenation plus any tool calls.
    pub fn chat_stream(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
        on_delta: &mut dyn FnMut(&str),
    ) -> anyhow::Result<Reply> {
        let url = format!("{}/v1/chat/completions", self.base.trim_end_matches('/'));
        let mut body = serde_json::json!({
            "model": self.model_id,
            "messages": messages,
            "stream": true,
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(tools.to_vec());
        }

        let mut resp = self
            .agent
            .post(&url)
            .send_json(&body)
            .map_err(|e| anyhow::anyhow!("model unreachable: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let raw = resp
                .body_mut()
                .with_config()
                .limit(64 * 1024)
                .read_to_string()
                .unwrap_or_default();
            let snippet: String = raw.trim().chars().take(200).collect();
            anyhow::bail!(
                "model http status {}: {}",
                status.as_u16(),
                if snippet.is_empty() {
                    "<empty body>"
                } else {
                    &snippet
                }
            );
        }

        let mut reader = BufReader::new(resp.body_mut().as_reader());
        let mut reply = Reply::default();
        // Tool calls stream in fragments keyed by `index`; order the map so
        // the finished calls come out in the order the model started them.
        let mut partials: BTreeMap<u64, PartialToolCall> = BTreeMap::new();

        while let Some(line) = read_sse_line(&mut reader, MAX_SSE_LINE)? {
            if line.is_empty() || line.starts_with(':') {
                continue; // event boundary, or a keep-alive comment
            }
            let Some(data) = line.strip_prefix("data:") else {
                continue; // event:/id:/retry: fields are unused
            };
            let data = data.strip_prefix(' ').unwrap_or(data);
            if data.trim() == "[DONE]" {
                break;
            }
            let chunk: Chunk = serde_json::from_str(data)
                .map_err(|e| anyhow::anyhow!("malformed model response: bad SSE data JSON: {e}"))?;
            let Some(choice) = chunk.choices.into_iter().next() else {
                continue; // e.g. a usage-only trailer chunk
            };
            let Some(delta) = choice.delta else { continue };
            if let Some(content) = delta.content.filter(|s| !s.is_empty()) {
                reply.text.push_str(&content);
                if reply.text.len() > MAX_REPLY_TEXT {
                    anyhow::bail!(
                        "malformed model response: accumulated text exceeds {MAX_REPLY_TEXT} bytes"
                    );
                }
                on_delta(&content);
            }
            if let Some(calls) = delta.tool_calls {
                for call in calls {
                    let entry = partials.entry(call.index).or_default();
                    if let Some(id) = call.id.filter(|s| !s.is_empty()) {
                        entry.id = Some(id);
                    }
                    if let Some(function) = call.function {
                        if let Some(name) = function.name.filter(|s| !s.is_empty()) {
                            entry.name = Some(name);
                        }
                        if let Some(arguments) = function.arguments {
                            entry.arguments.push_str(&arguments);
                            if entry.arguments.len() > MAX_TOOL_ARGS {
                                anyhow::bail!(
                                    "malformed model response: tool arguments exceed {MAX_TOOL_ARGS} bytes"
                                );
                            }
                        }
                    }
                }
            }
        }

        // [DONE] or clean EOF ends the turn: parse each arguments string
        // exactly once, now that all its fragments have arrived.
        for partial in partials.into_values() {
            let arguments = if partial.arguments.trim().is_empty() {
                serde_json::Value::Object(serde_json::Map::new())
            } else {
                serde_json::from_str(&partial.arguments).map_err(|e| {
                    anyhow::anyhow!("malformed model response: tool arguments not valid JSON: {e}")
                })?
            };
            reply.tool_calls.push(ToolCall {
                id: partial.id.unwrap_or_default(),
                name: partial.name.unwrap_or_default(),
                arguments,
            });
        }
        Ok(reply)
    }
}

/// Read one SSE line (without the terminator) with a hard size cap.
/// `Ok(None)` is a clean EOF before any bytes; a final unterminated line
/// at EOF is still returned. `\r\n` terminators lose the `\r`.
fn read_sse_line(reader: &mut impl BufRead, cap: usize) -> anyhow::Result<Option<String>> {
    let mut raw: Vec<u8> = Vec::new();
    loop {
        let (consumed, terminated, eof) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                (0, false, true)
            } else {
                match available.iter().position(|&b| b == b'\n') {
                    Some(pos) => {
                        raw.extend_from_slice(&available[..pos]);
                        (pos + 1, true, false)
                    }
                    None => {
                        raw.extend_from_slice(available);
                        (available.len(), false, false)
                    }
                }
            }
        };
        reader.consume(consumed);
        if raw.len() > cap {
            anyhow::bail!("malformed model response: SSE line exceeds {cap} bytes");
        }
        if terminated {
            break;
        }
        if eof {
            if raw.is_empty() {
                return Ok(None);
            }
            break;
        }
    }
    if raw.last() == Some(&b'\r') {
        raw.pop();
    }
    let line = String::from_utf8(raw)
        .map_err(|_| anyhow::anyhow!("malformed model response: SSE line is not UTF-8"))?;
    Ok(Some(line))
}

/// `choices[0]` of one `data:` chunk. Unknown fields (role, finish_reason,
/// usage, …) are ignored so llama-server extras never break parsing.
#[derive(Debug, Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
}

#[derive(Debug, Deserialize)]
struct ChunkChoice {
    #[serde(default)]
    delta: Option<ChunkDelta>,
}

#[derive(Debug, Deserialize)]
struct ChunkDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChunkToolCall>>,
}

#[derive(Debug, Deserialize)]
struct ChunkToolCall {
    #[serde(default)]
    index: u64,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ChunkFunction>,
}

#[derive(Debug, Deserialize)]
struct ChunkFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// One in-flight tool call, reassembled across stream fragments.
#[derive(Default)]
struct PartialToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_uses_openai_wire_shape() {
        let call = ToolCall {
            id: "call_1".into(),
            name: "kb.read".into(),
            arguments: serde_json::json!({"query": "alpha"}),
        };
        let v = serde_json::to_value(&call).unwrap();
        assert_eq!(v["type"], "function");
        assert_eq!(v["function"]["name"], "kb.read");
        // arguments travel as a JSON *string*, not an object
        assert_eq!(v["function"]["arguments"], "{\"query\":\"alpha\"}");
        assert!(v.get("name").is_none() && v.get("arguments").is_none());

        let back: ToolCall = serde_json::from_value(v).unwrap();
        assert_eq!(back.id, "call_1");
        assert_eq!(back.arguments["query"], "alpha");

        let empty: ToolCall = serde_json::from_value(serde_json::json!({
            "id": "c", "type": "function", "function": {"name": "health.read", "arguments": ""}
        }))
        .unwrap();
        assert!(empty.arguments.as_object().is_some_and(|m| m.is_empty()));
    }

    #[test]
    fn assistant_message_with_calls_round_trips() {
        let msg = ChatMessage {
            role: "assistant".into(),
            content: None,
            tool_calls: Some(vec![ToolCall {
                id: "c1".into(),
                name: "logs.read".into(),
                arguments: serde_json::json!({"n": 5}),
            }]),
            tool_call_id: None,
        };
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["tool_calls"][0]["function"]["arguments"], "{\"n\":5}");
        let back: ChatMessage = serde_json::from_value(v).unwrap();
        assert_eq!(back.tool_calls.unwrap()[0].arguments["n"], 5);
    }
}
