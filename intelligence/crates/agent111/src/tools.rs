//! The fixed tool set — no shell, no plugins, ever. Schemas are contract
//! (frozen here at T0); dispatch lands in T4.

use std::sync::Arc;

use crate::config::Config;
use crate::events::Journal;
use crate::kb::Kb;
use crate::store::Store;
use crate::tasks::TaskEngine;

/// Everything a tool is allowed to touch.
pub struct ToolCtx {
    pub kb: Arc<Kb>,
    pub store: Arc<Store>,
    pub journal: Journal,
    pub cfg: Arc<Config>,
    pub tasks: Arc<TaskEngine>,
}

/// OpenAI tool-calling schemas for the five tools.
pub fn schemas() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "kb.read",
                "description": "Read from the local knowledge base. Follow the retrieval ritual: grep a keyword first (filenames come back), then read one file. Never dump the whole base.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Keyword to grep for (returns matching filenames), or a filename from a previous grep to read in full." }
                    },
                    "required": ["query"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "health.read",
                "description": "Read the node's health: this agent's components and the 111d daemon's /health verdict.",
                "parameters": { "type": "object", "properties": {} }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "logs.read",
                "description": "Read the agent's own recent activity journal (tail).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "n": { "type": "integer", "description": "Number of recent events (default 20, max 100)." }
                    }
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "proposal.write",
                "description": "Write a proposal document (a diagnosis, a suggested fix, a plan). Proposals are the only write path the agent has; humans review them.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string" },
                        "body": { "type": "string", "description": "Markdown body of the proposal." }
                    },
                    "required": ["title", "body"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "task.submit",
                "description": "Submit a background task (Layer 9 seam). patch tasks need a hub (opencode) and are refused otherwise.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string", "enum": ["diagnose", "propose", "patch"] },
                        "refs": { "type": "array", "items": { "type": "string" } },
                        "note": { "type": "string" }
                    },
                    "required": ["kind", "refs"]
                }
            }
        }),
    ]
}

/// Execute one tool call. T4 implements; errors are strings the model can read.
pub async fn dispatch(
    _name: &str,
    _args: serde_json::Value,
    _ctx: &ToolCtx,
) -> Result<serde_json::Value, String> {
    Err("tools land in T4".to_string())
}
