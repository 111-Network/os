//! The fixed tool set — no shell, no plugins, ever. Schemas are contract
//! (frozen at T0); `dispatch` runs one call against the node's own state.
//! Every tool is a plain function over `ToolCtx`: nothing here spawns a
//! subprocess.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::config::Config;
use crate::events::Journal;
use crate::kb::Kb;
use crate::model::Client;
use crate::store::Store;
use crate::tasks::{SubmitError, TaskEngine, TaskKind};

/// Everything a tool is allowed to touch.
pub struct ToolCtx {
    pub kb: Arc<Kb>,
    pub store: Arc<Store>,
    pub journal: Journal,
    pub cfg: Arc<Config>,
    pub tasks: Arc<TaskEngine>,
    /// Probed live by `health.read`; never a cached answer.
    pub model: Arc<Client>,
}

const LOGS_DEFAULT: usize = 20;
const LOGS_MAX: usize = 100;
const DAEMON_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const DAEMON_BODY_CAP: u64 = 64 * 1024;

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

/// Execute one tool call. Errors are plain strings the model can read; the
/// chat loop hands them back as `{"error": …}` tool results.
pub async fn dispatch(name: &str, args: Value, ctx: &ToolCtx) -> Result<Value, String> {
    match name {
        "kb.read" => kb_read(&args, ctx).await,
        "health.read" => Ok(health_read(ctx).await),
        "logs.read" => Ok(logs_read(&args, ctx)),
        "proposal.write" => proposal_write(&args, ctx),
        "task.submit" => task_submit(&args, ctx),
        _ => Err("unknown tool".to_string()),
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing or non-string argument '{key}'"))
}

/// A name that resolves to one entry returns its content; anything else is a
/// grep, which returns filenames only (the retrieval ritual).
async fn kb_read(args: &Value, ctx: &ToolCtx) -> Result<Value, String> {
    let query = str_arg(args, "query")?.trim().to_string();
    if query.is_empty() {
        return Err("query must not be empty".to_string());
    }
    if !ctx.kb.is_present() {
        return Err("knowledge base absent on this node".to_string());
    }
    let kb = ctx.kb.clone();
    tokio::task::spawn_blocking(move || match kb.read_one(&query) {
        Some(content) => json!({"path": query, "content": content}),
        None => {
            let matches = kb.grep(&query);
            if matches.is_empty() {
                json!({
                    "matches": matches,
                    "hint": "no entry matches; say the knowledge base has no entry for this",
                })
            } else {
                json!({"matches": matches})
            }
        }
    })
    .await
    .map_err(|e| format!("knowledge base lookup failed: {e}"))
}

/// This agent's own components plus the daemon's `/health` body, verbatim.
/// Both probes are bounded to 2s and run concurrently.
async fn health_read(ctx: &ToolCtx) -> Value {
    let model = ctx.model.clone();
    let url = ctx.cfg.daemon_health_url.clone();
    let (model_reachable, daemon) = tokio::join!(
        tokio::task::spawn_blocking(move || model.reachable()),
        tokio::task::spawn_blocking(move || probe_daemon(&url)),
    );
    json!({
        "agent": {
            "model_reachable": model_reachable.unwrap_or(false),
            "kb_present": ctx.kb.is_present(),
        },
        "daemon": daemon.unwrap_or_else(|e| json!({"error": format!("daemon probe failed: {e}")})),
    })
}

fn probe_daemon(url: &str) -> Value {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(DAEMON_PROBE_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let mut resp = match agent.get(url).call() {
        Ok(resp) => resp,
        Err(e) => return json!({"error": format!("daemon unreachable: {e}")}),
    };
    let status = resp.status().as_u16();
    let text = match resp
        .body_mut()
        .with_config()
        .limit(DAEMON_BODY_CAP)
        .read_to_string()
    {
        Ok(text) => text,
        Err(e) => return json!({"error": format!("daemon health unreadable: {e}")}),
    };
    serde_json::from_str(&text).unwrap_or_else(
        |_| json!({"error": format!("daemon health was not JSON (http status {status})")}),
    )
}

fn logs_read(args: &Value, ctx: &ToolCtx) -> Value {
    let n = args
        .get("n")
        .and_then(Value::as_f64)
        .map_or(LOGS_DEFAULT, |f| (f as usize).clamp(1, LOGS_MAX));
    json!({"events": ctx.journal.tail(n)})
}

fn proposal_write(args: &Value, ctx: &ToolCtx) -> Result<Value, String> {
    let title = str_arg(args, "title")?;
    let body = str_arg(args, "body")?;
    if title.trim().is_empty() {
        return Err("title must not be empty".to_string());
    }
    let proposal = ctx
        .store
        .save_proposal(ctx.cfg.node_id.clone(), None, "chat", title, body)
        .map_err(|e| format!("could not save proposal: {e}"))?;
    ctx.journal.append(
        "proposal.written",
        json!({"proposal_id": proposal.id, "kind": "chat", "title": title}),
    );
    Ok(json!({"proposal_id": proposal.id}))
}

fn task_submit(args: &Value, ctx: &ToolCtx) -> Result<Value, String> {
    let kind_text = str_arg(args, "kind")?;
    let kind: TaskKind =
        serde_json::from_value(Value::String(kind_text.to_string())).map_err(|_| {
            format!("invalid task kind {kind_text:?} (expected diagnose, propose or patch)")
        })?;
    let refs: Vec<String> = args
        .get("refs")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let note = args
        .get("note")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    match ctx.tasks.submit(kind, refs, note) {
        Ok(id) => Ok(json!({"task_id": id})),
        Err(SubmitError::NeedsHub) => {
            Err("needs hub: patch tasks require a hub node with opencode".to_string())
        }
        Err(SubmitError::Other(e)) => Err(e.to_string()),
    }
}
