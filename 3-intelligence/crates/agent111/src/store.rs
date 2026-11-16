//! State layer: conversations, proposals, tasks under `state_dir` (0700).
//! Layout per TASKS.md. Capacity-pruning policy is Layer 8's; v1 keeps the
//! shapes honest and the writes atomic.
//!
//! **The spool (Layer 8 §2.4 rule 1).** `proposals/` and `tasks/` are where
//! `111d` takes proposals and finished tasks from: it ingests each into its
//! encrypted `node/state.db`, then unlinks the file. Every file there is
//! written atomically (a `.tmp-` file in the same directory, fsync, rename),
//! so `111d` and [`Store::load_tasks`] read the old file, the new one or none,
//! never a part; a `.tmp-` file is never read as a task, and the ones a crash
//! left are removed at the agent's start ([`Store::init`]). A write waits on
//! the disk without holding a runtime worker.
//!
//! **Conversations (Layer 8 §2.3 rule 2, §2.5).** They live in `111d`'s store,
//! which sends them in the chat body's `history`; nothing is written under
//! `conversations/` for such a turn. Only a chat body without `history` (an
//! older `111d`) still appends to `conversations/<id>.jsonl`.

use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

use crate::events::now_rfc3339;
use crate::tasks::TaskKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub schema: u8,
    pub id: String,
    /// Layer 2 NodeId hex of the authoring node (None when 111d passed none).
    pub node_id: Option<String>,
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
    /// Create the state dir (0700) and its subdirs. Idempotent. Run once at
    /// the agent's start, it also removes every `.tmp-*` file of the two spool
    /// directories (§2.4 rule 1): a write a crash cut short is never renamed
    /// in, so it is never a proposal or a task.
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
        let removed: usize = SPOOL_DIRS
            .iter()
            .map(|sub| remove_temp_files(&dir.join(sub)))
            .sum();
        if removed > 0 {
            crate::log_line(&format!(
                "removed {removed} unfinished spool file(s) a stop mid-write left"
            ));
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

    /// Append one message (OpenAI shape) to a conversation JSONL. Only a chat
    /// body without `history` (an older `111d`) writes here (§2.3 rule 2).
    pub fn append_conversation(&self, id: &str, message: &serde_json::Value) -> anyhow::Result<()> {
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

    /// Write `proposals/<id>.json` atomically (§2.4 rule 1) for `111d` to
    /// ingest; the file appears whole or not at all.
    pub fn save_proposal(
        &self,
        node_id: Option<String>,
        task_id: Option<String>,
        kind: &str,
        title: &str,
        body: &str,
    ) -> anyhow::Result<Proposal> {
        let p = Proposal {
            schema: 1,
            id: new_id("prop"),
            node_id,
            task_id,
            kind: kind.to_string(),
            title: title.to_string(),
            body: body.to_string(),
            created_at: now_rfc3339(),
        };
        write_atomic(
            &self.dir.join("proposals"),
            &format!("{}.json", p.id),
            serde_json::to_string_pretty(&p)?.as_bytes(),
        )?;
        Ok(p)
    }

    /// Write `tasks/<id>.json` atomically (§2.4 rule 1): a rewrite replaces
    /// the file whole, so `111d` never ingests half a task.
    pub fn save_task(&self, task: &Task) -> anyhow::Result<()> {
        write_atomic(
            &self.dir.join("tasks"),
            &format!("{}.json", task.id),
            serde_json::to_string_pretty(&task)?.as_bytes(),
        )?;
        Ok(())
    }

    /// Every task file that parses; a `.tmp-` file still being written (or
    /// left by a crash) is not a task.
    pub fn load_tasks(&self) -> Vec<Task> {
        let rd = match std::fs::read_dir(self.dir.join("tasks")) {
            Ok(rd) => rd,
            Err(_) => return Vec::new(),
        };
        rd.flatten()
            .filter(|e| !is_temp_name(&e.file_name()))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|s| serde_json::from_str(&s).ok())
            .collect()
    }
}

/// The spool directories `111d` ingests from (§2.4).
const SPOOL_DIRS: [&str; 2] = ["proposals", "tasks"];

/// What every temp file of a spool directory is named with; `111d`'s sweep
/// takes only `<id>.json` and never one of these (§2.4 rule 2).
const TMP_PREFIX: &str = ".tmp-";

/// Attempts at a fresh temp name before a write gives up.
const TMP_ATTEMPTS: u32 = 16;

static TMP_COUNTER: AtomicU32 = AtomicU32::new(0);

fn is_temp_name(name: &OsStr) -> bool {
    name.as_encoded_bytes().starts_with(TMP_PREFIX.as_bytes())
}

/// Write `dir/name` atomically (§2.4 rule 1): the bytes go to a new `.tmp-`
/// file in `dir`, which is fsync'd, closed and renamed over `name`; then `dir`
/// itself is fsync'd so the rename outlasts a power cut. A reader of `dir`
/// finds the old file, the new one or none, never a part. On an error the
/// temp file is removed and `name` is as it was. The write waits on the disk
/// off the runtime's workers ([`off_the_workers`]).
fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    off_the_workers(|| write_atomic_here(dir, name, bytes))
}

/// Runs `f`, which waits on the disk, without holding a runtime worker: the
/// two fsyncs of a spool write (§2.4 rule 1) can take tens of milliseconds on
/// an SD card, and the proposal tools and the task route write from async
/// code. On a worker of a multi-thread runtime `f` runs under
/// `block_in_place`, which hands the worker's other tasks (`/health`, the
/// deltas of another turn) to a fresh thread meanwhile; anywhere else (a task
/// thread, the blocking pool, a current-thread runtime, where
/// `block_in_place` would panic) it runs in place.
fn off_the_workers<R>(f: impl FnOnce() -> R) -> R {
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current() {
        Ok(h) if h.runtime_flavor() == RuntimeFlavor::MultiThread => tokio::task::block_in_place(f),
        _ => f(),
    }
}

/// [`write_atomic`] on the calling thread.
fn write_atomic_here(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    let (tmp, file) = create_temp(dir, name)?;
    let written = fill(file, bytes).and_then(|()| std::fs::rename(&tmp, dir.join(name)));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // The file is in place whatever this answers; a filesystem that cannot
    // sync a directory still has it.
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// `bytes` into `file`, on disk before the file closes.
fn fill(mut file: std::fs::File, bytes: &[u8]) -> std::io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()
}

/// A temp file of `dir` that did not exist before (`create_new`): two writers
/// never share one, whatever else is in the directory.
fn create_temp(dir: &Path, name: &str) -> std::io::Result<(PathBuf, std::fs::File)> {
    let mut last = None;
    for _ in 0..TMP_ATTEMPTS {
        let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!("{TMP_PREFIX}{name}.{n}"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(file) => return Ok((tmp, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no free temp name")))
}

/// Remove every `.tmp-*` file of `dir`; returns how many went.
fn remove_temp_files(dir: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .filter(|e| is_temp_name(&e.file_name()))
        .filter(|e| std::fs::remove_file(e.path()).is_ok())
        .count()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// With one worker, a task waiting inside [`off_the_workers`] for another
    /// task of the same runtime still gets its answer: the worker went on
    /// running tasks while it waited. Held on the worker, it would wait out
    /// its timeout: the other task never runs.
    #[test]
    fn a_wait_on_the_disk_holds_no_runtime_worker() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let answered = rt.block_on(async {
            let (inside_tx, inside_rx) = tokio::sync::oneshot::channel::<()>();
            let (answer_tx, answer_rx) = std::sync::mpsc::channel::<()>();
            // Runs only once the waiting task is inside its wait.
            let other = tokio::spawn(async move {
                if inside_rx.await.is_ok() {
                    let _ = answer_tx.send(());
                }
            });
            let waiting = tokio::spawn(async move {
                off_the_workers(move || {
                    let _ = inside_tx.send(());
                    answer_rx.recv_timeout(Duration::from_secs(5)).is_ok()
                })
            });
            let answered = waiting.await.unwrap();
            other.await.unwrap();
            answered
        });
        assert!(
            answered,
            "the other task did not run while the write waited"
        );
    }
}
