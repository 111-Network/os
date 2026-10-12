//! OpenAI-compatible model client (blocking ureq; callers use spawn_blocking).
//! Talks to llama-server locally, or to a hub's endpoint when offloaded.
//! Plain HTTP only — endpoints are loopback/private fabric; encryption is
//! Layer 5's job. Full implementation with SSE streaming lands in T2.

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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// What a completed chat turn returned.
#[derive(Debug, Default)]
pub struct Reply {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Clone)]
pub struct Client {
    base: String,
    model_id: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(base: impl Into<String>, model_id: impl Into<String>) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(120)))
            .build();
        Self {
            base: base.into(),
            model_id: model_id.into(),
            agent: config.into(),
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
    pub fn reachable(&self) -> bool {
        let url = format!("{}/health", self.base.trim_end_matches('/'));
        match self.agent.get(&url).call() {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// Streaming chat completion with the OpenAI tool-calling protocol.
    /// `on_delta` receives each text delta as it arrives. T2 implements.
    pub fn chat_stream(
        &self,
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
        _on_delta: &mut dyn FnMut(&str),
    ) -> anyhow::Result<Reply> {
        anyhow::bail!("model client lands in T2")
    }
}
