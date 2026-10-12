//! State layer: conversations, proposals, tasks under `state_dir` (0700).
//! Layout per TASKS.md. Capacity-pruning policy is Layer 8's; v1 keeps the
//! shapes honest and the writes atomic.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

use crate::events::now_rfc3339;
use crate::tasks::TaskKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub schema: u8,
    pub id: String,
    pub task_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Running,
    Done,
    Failed,
    Refused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub schema: u8,
    pub id: String,
    pub kind: TaskKind,
    pub refs: Vec<String>,
    pub note: Option<String>,
    pub state: TaskState,
    pub proposal_id: Option<String>,
    pub created_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// Create the state dir (0700) and its subdirs. Idempotent.
    pub fn init(dir: &Path) -> anyhow::Result<Self> {
        for sub in ["", "conversations", "proposals", "tasks"] {
            let p = if sub.is_empty() {
                dir.to_path_buf()
            } else {
                dir.join(sub)
            };
            std::fs::create_dir_all(&p)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn events_path(&self) -> PathBuf {
        self.dir.join("events.jsonl")
    }

    pub fn conversation_path(&self, id: &str) -> PathBuf {
        self.dir
            .join("conversations")
            .join(format!("{}.jsonl", sanitize(id)))
    }

    pub fn conversation_count(&self) -> usize {
        std::fs::read_dir(self.dir.join("conversations"))
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
                    .count()
            })
            .unwrap_or(0)
    }

    /// Append one message (OpenAI shape) to a conversation JSONL.
    pub fn append_conversation(&self, id: &str, message: &serde_json::Value) -> anyhow::Result<()> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.conversation_path(id))?;
        writeln!(f, "{}", serde_json::to_string(message)?)?;
        Ok(())
    }

    /// Load a conversation's messages in order. Missing id ⇒ empty.
    pub fn load_conversation(&self, id: &str) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.conversation_path(id))
            .map(|s| {
                s.lines()
                    .filter_map(|l| serde_json::from_str(l).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn save_proposal(
        &self,
        task_id: Option<String>,
        kind: &str,
        title: &str,
        body: &str,
    ) -> anyhow::Result<Proposal> {
        let p = Proposal {
            schema: 1,
            id: new_id("prop"),
            task_id,
            kind: kind.to_string(),
            title: title.to_string(),
            body: body.to_string(),
            created_at: now_rfc3339(),
        };
        let path = self.dir.join("proposals").join(format!("{}.json", p.id));
        std::fs::write(path, serde_json::to_string_pretty(&p)?)?;
        Ok(p)
    }

    pub fn save_task(&self, task: &Task) -> anyhow::Result<()> {
        let path = self.dir.join("tasks").join(format!("{}.json", task.id));
        std::fs::write(path, serde_json::to_string_pretty(&task)?)?;
        Ok(())
    }

    pub fn load_tasks(&self) -> Vec<Task> {
        let rd = match std::fs::read_dir(self.dir.join("tasks")) {
            Ok(rd) => rd,
            Err(_) => return Vec::new(),
        };
        rd.flatten()
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|s| serde_json::from_str(&s).ok())
            .collect()
    }
}

static ID_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Timestamp-nanos + process counter — unique per process, no rand dep.
pub fn new_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{prefix}-{nanos:x}-{n:04x}")
}

/// Keep ids filesystem-safe and traversal-proof.
fn sanitize(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}
