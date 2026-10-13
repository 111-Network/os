//! Task engine — the Layer 9 seam. diagnose/propose run ONE tool-less model
//! call over gathered context; patch delegates to opencode (headless JSONL)
//! on hubs. Every outcome lands as a proposal document under
//! `proposals/`; task state lives in `tasks/<id>.json`.
//!
//! Deliberately minimal: monitoring cadence, playbooks, sandboxing and PR
//! packaging are Layer 9's. Runs execute on plain OS threads (the work is
//! blocking ureq / child-process I/O), so the engine needs no runtime and
//! never holds up runtime shutdown.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::Config;
use crate::events::{now_rfc3339, Journal};
use crate::kb::Kb;
use crate::model::{ChatMessage, Client};
use crate::store::{new_id, Store, Task, TaskState};

/// opencode's headless JSONL invocation: `<opencode_path> <OPENCODE_ARGV…>
/// <prompt>`. See knowledge/projects/opencode.md (headless JSONL is the
/// delegation seam). The exact subcommand/flags drift across opencode
/// releases (the 2.x line moves fast), so pinning a version is one edit here.
const OPENCODE_ARGV: &[&str] = &["run", "--format", "json"];

const DEFAULT_PATCH_TIMEOUT: Duration = Duration::from_secs(600);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
/// Journal events handed to the model/opencode as context.
const JOURNAL_CONTEXT: usize = 30;
/// KB refs resolved per task, and bytes kept per resolved entry.
const MAX_REFS: usize = 8;
const MAX_REF_BYTES: usize = 8 * 1024;
/// A single argv element tops out at 128 KiB on Linux; stay well under it.
const MAX_PATCH_PROMPT: usize = 96 * 1024;
/// Text collected from opencode's JSONL, and the longest line we buffer.
const MAX_COLLECTED: usize = 256 * 1024;
const MAX_STDERR_TAIL: usize = 2 * 1024;
const TITLE_MAX: usize = 80;
/// After the child exits, how long to wait for its pipes to drain.
const EXIT_GRACE: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskKind {
    Diagnose,
    Propose,
    Patch,
}

impl TaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskKind::Diagnose => "diagnose",
            TaskKind::Propose => "propose",
            TaskKind::Patch => "patch",
        }
    }
}

#[derive(Debug)]
pub enum SubmitError {
    /// patch was requested but no working opencode is available on this node.
    NeedsHub,
    Other(anyhow::Error),
}

impl std::fmt::Display for SubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubmitError::NeedsHub => write!(f, "needs hub"),
            SubmitError::Other(e) => write!(f, "{e}"),
        }
    }
}

/// Everything a run needs; cloned onto the worker thread.
#[derive(Clone)]
struct Ctx {
    cfg: Arc<Config>,
    store: Arc<Store>,
    journal: Journal,
    model: Arc<Client>,
    kb: Arc<Kb>,
    patch_timeout: Duration,
    open: Arc<AtomicUsize>,
}

pub struct TaskEngine {
    ctx: Ctx,
}

impl TaskEngine {
    /// Build the engine and reload persisted tasks: anything still `running`
    /// belongs to a previous process, so it is marked `failed` and journaled
    /// as `task.orphaned` — a restarted agent never pretends old work lives.
    pub fn new(cfg: Arc<Config>, store: Arc<Store>, journal: Journal, model: Arc<Client>) -> Self {
        // The KB snapshot is read-only, so the engine opens its own handle
        // rather than widening this signature.
        let kb = Arc::new(Kb::open(&cfg.kb_dir));
        let ctx = Ctx {
            cfg,
            store,
            journal,
            model,
            kb,
            patch_timeout: DEFAULT_PATCH_TIMEOUT,
            open: Arc::new(AtomicUsize::new(0)),
        };
        ctx.orphan_running();
        Self { ctx }
    }

    /// Override the opencode run timeout (default 10 min); tests shrink it.
    pub fn with_patch_timeout(mut self, timeout: Duration) -> Self {
        self.ctx.patch_timeout = timeout;
        self
    }

    /// Submit a task; returns the task id. Validation and persistence are
    /// synchronous, the run itself is asynchronous (worker thread).
    /// `patch` without a working opencode is refused before anything is
    /// persisted: a refused submit is not a task.
    pub fn submit(
        &self,
        kind: TaskKind,
        refs: Vec<String>,
        note: Option<String>,
    ) -> Result<String, SubmitError> {
        let ctx = &self.ctx;
        if kind == TaskKind::Patch && !opencode_available(&ctx.cfg) {
            ctx.journal.append(
                "task.refused",
                json!({"kind": kind.as_str(), "reason": "needs hub"}),
            );
            return Err(SubmitError::NeedsHub);
        }

        let task = Task {
            schema: 1,
            id: new_id("task"),
            kind,
            refs,
            note,
            state: TaskState::Running,
            proposal_id: None,
            created_at: now_rfc3339(),
            finished_at: None,
        };
        ctx.store
            .save_task(&task)
            .map_err(|e| SubmitError::Other(e.context("cannot persist task")))?;
        ctx.open.fetch_add(1, Ordering::SeqCst);
        ctx.journal.append(
            "task.submitted",
            json!({"task_id": task.id, "kind": kind.as_str()}),
        );

        let id = task.id.clone();
        let worker = ctx.clone();
        let run_task = task.clone();
        if let Err(e) = std::thread::Builder::new()
            .name(format!("agent111-{id}"))
            .spawn(move || worker.run(run_task))
        {
            let msg = format!("cannot start task thread: {e}");
            ctx.finalize(task, Err(msg.clone()));
            return Err(SubmitError::Other(anyhow::anyhow!(msg)));
        }
        Ok(id)
    }

    /// Tasks currently running in this process.
    pub fn open_count(&self) -> usize {
        self.ctx.open.load(Ordering::SeqCst)
    }

    pub fn cfg(&self) -> &Config {
        &self.ctx.cfg
    }
    pub fn store(&self) -> &Store {
        &self.ctx.store
    }
    pub fn journal(&self) -> &Journal {
        &self.ctx.journal
    }
    pub fn model(&self) -> &Client {
        &self.ctx.model
    }
}

/// True when `cfg.opencode_path` resolves to an executable file: a path that
/// exists and has an exec bit, or a bare name found by walking `PATH`.
pub fn opencode_available(cfg: &Config) -> bool {
    resolve_executable(&cfg.opencode_path).is_some()
}

/// Absolute path of the executable, without resolving symlinks (a symlinked
/// launcher must keep its own name).
fn resolve_executable(p: &Path) -> Option<PathBuf> {
    let found = if p.is_absolute() || p.components().count() > 1 {
        is_executable_file(p).then(|| p.to_path_buf())?
    } else {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .filter(|d| !d.as_os_str().is_empty())
            .map(|d| d.join(p))
            .find(|c| is_executable_file(c))?
    };
    std::path::absolute(found).ok()
}

fn is_executable_file(p: &Path) -> bool {
    let Ok(md) = std::fs::metadata(p) else {
        return false;
    };
    if !md.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        md.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The result of a successful run, before it becomes a proposal.
struct Draft {
    title: String,
    body: String,
}

impl Ctx {
    fn orphan_running(&self) {
        for mut task in self.store.load_tasks() {
            if task.state != TaskState::Running {
                continue;
            }
            task.state = TaskState::Failed;
            task.finished_at = Some(now_rfc3339());
            let mut detail = json!({"task_id": task.id, "kind": task.kind.as_str()});
            if let Err(e) = self.store.save_task(&task) {
                detail["persist_error"] = json!(format!("{e:#}"));
            }
            self.journal.append("task.orphaned", detail);
        }
    }

    /// Worker-thread entry: run, turn the draft into a proposal, finalize.
    fn run(&self, task: Task) {
        let drafted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match task.kind {
            TaskKind::Diagnose | TaskKind::Propose => self.run_model(&task),
            TaskKind::Patch => self.run_patch(&task),
        }))
        .unwrap_or_else(|_| Err("task run panicked".to_string()));

        let outcome = drafted.and_then(|draft| {
            self.store
                .save_proposal(
                    self.cfg.node_id.clone(),
                    Some(task.id.clone()),
                    task.kind.as_str(),
                    &draft.title,
                    &draft.body,
                )
                .map(|p| p.id)
                .map_err(|e| format!("cannot save proposal: {e:#}"))
        });
        self.finalize(task, outcome);
    }

    /// Persist the terminal state, then journal it. The counter drops before
    /// the journal event so an observer who saw the event sees `open_count`
    /// already settled. The error text of a failed run lives in the journal
    /// (`Task` has no error field).
    fn finalize(&self, mut task: Task, outcome: Result<String, String>) {
        let (kind, mut detail) = match &outcome {
            Ok(pid) => (
                "task.done",
                json!({"task_id": task.id, "kind": task.kind.as_str(), "proposal_id": pid}),
            ),
            Err(e) => (
                "task.failed",
                json!({"task_id": task.id, "kind": task.kind.as_str(), "error": e}),
            ),
        };
        task.state = if outcome.is_ok() {
            TaskState::Done
        } else {
            TaskState::Failed
        };
        task.proposal_id = outcome.ok();
        task.finished_at = Some(now_rfc3339());
        if let Err(e) = self.store.save_task(&task) {
            detail["persist_error"] = json!(format!("{e:#}"));
        }
        self.open.fetch_sub(1, Ordering::SeqCst);
        self.journal.append(kind, detail);
    }

    // --- diagnose / propose -------------------------------------------------

    fn run_model(&self, task: &Task) -> Result<Draft, String> {
        let word = if task.kind == TaskKind::Diagnose {
            "diagnosis"
        } else {
            "proposal"
        };
        let system = format!(
            "You are the maintenance agent of a 111network mesh node. Produce a {word} in \
             Markdown with exactly these sections: Summary, Evidence, Suggested action. \
             Never invent facts: rely only on the context you are given. If the context is \
             insufficient to conclude, say so plainly in the Summary and list what is missing. \
             You have no tools and cannot run commands; Suggested action is advice for a \
             human or an automated playbook, not something you execute."
        );
        let user = format!(
            "Task kind: {}\nNote: {}\n\nContext (JSON):\n{}",
            task.kind.as_str(),
            task.note.as_deref().unwrap_or("(none)"),
            self.gather_context(task)
        );
        let reply = self
            .model
            .chat_stream(
                &[ChatMessage::system(system), ChatMessage::user(user)],
                &[],
                &mut |_| {},
            )
            .map_err(|e| format!("{e:#}"))?;
        let body = reply.text.trim();
        if body.is_empty() {
            return Err("model returned an empty reply".to_string());
        }
        Ok(Draft {
            title: title_of(body).unwrap_or_else(|| format!("{} {}", task.kind.as_str(), task.id)),
            body: body.to_string(),
        })
    }

    /// Context shared by every task kind: daemon health, recent journal,
    /// and each ref resolved against the KB (else the literal ref string).
    fn gather_context(&self, task: &Task) -> Value {
        let refs: Vec<Value> = task
            .refs
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let entry = if i < MAX_REFS {
                    self.kb.read_one(r)
                } else {
                    None
                };
                json!({
                    "ref": r,
                    "resolved": entry.is_some(),
                    "content": entry.map(|c| cap_str(&c, MAX_REF_BYTES)),
                })
            })
            .collect();
        let mut ctx = json!({
            "node_id": self.cfg.node_id,
            "category": self.cfg.category,
            "kb_present": self.kb.is_present(),
            "daemon_health": daemon_health(&self.cfg.daemon_health_url),
            "recent_events": self.journal.tail(JOURNAL_CONTEXT),
            "refs": refs,
        });
        if task.refs.len() > MAX_REFS {
            ctx["refs_not_resolved_beyond"] = json!(MAX_REFS);
        }
        ctx
    }

    // --- patch (opencode) ---------------------------------------------------

    fn run_patch(&self, task: &Task) -> Result<Draft, String> {
        let exe = resolve_executable(&self.cfg.opencode_path)
            .ok_or_else(|| "opencode is no longer available".to_string())?;
        let work = self.store.dir().join("work").join(&task.id);
        std::fs::create_dir_all(&work)
            .map_err(|e| format!("cannot create work dir {}: {e}", work.display()))?;

        let prompt = cap_str(
            &format!(
                "You are the maintenance agent of a 111network mesh node, running headless. \
                 Produce a patch (a unified diff plus a short rationale) for the task below, \
                 working only inside the current directory. Never invent facts: if the context \
                 is insufficient, say so instead of guessing.\n\n\
                 Note: {}\n\nContext (JSON):\n{}",
                task.note.as_deref().unwrap_or("(none)"),
                self.gather_context(task)
            ),
            MAX_PATCH_PROMPT,
        );

        // No WAN/cloud model, no API keys: the child sees only PATH, HOME and
        // the one permitted provider, the local OpenAI-compatible endpoint.
        let mut cmd = Command::new(&exe);
        cmd.args(OPENCODE_ARGV)
            .arg(prompt)
            .current_dir(&work)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for key in ["PATH", "HOME"] {
            if let Some(v) = std::env::var_os(key) {
                cmd.env(key, v);
            }
        }
        cmd.env(
            "OPENAI_BASE_URL",
            format!("{}/v1", self.cfg.model_base.trim_end_matches('/')),
        )
        .env("OPENAI_API_KEY", "local");

        let child = cmd
            .spawn()
            .map_err(|e| format!("cannot start opencode ({}): {e}", exe.display()))?;
        let run = supervise(child, self.patch_timeout);
        let tail = tail_of(&run.stderr, MAX_STDERR_TAIL).trim();

        if run.timed_out {
            return Err(format!(
                "opencode timed out after {}s: {tail}",
                self.patch_timeout.as_secs()
            ));
        }
        match run.status.and_then(|s| s.code()) {
            Some(0) => {}
            Some(n) => return Err(format!("opencode exited with exit code {n}: {tail}")),
            None => return Err(format!("opencode terminated by a signal: {tail}")),
        }

        let mut body = if run.text.is_empty() {
            "(opencode produced no text output)".to_string()
        } else {
            run.text
        };
        if run.truncated {
            body.push_str("\n\n[... opencode output truncated ...]");
        }
        body.push_str(&format!("\n\n---\nopencode work dir: {}\n", work.display()));
        let subject = task
            .note
            .as_deref()
            .and_then(first_line)
            .or_else(|| task.refs.first().map(String::as_str))
            .unwrap_or(&task.id);
        Ok(Draft {
            title: take_chars(&format!("patch: {subject}"), TITLE_MAX),
            body,
        })
    }
}

/// What the supervised child produced.
#[derive(Default)]
struct Supervised {
    status: Option<std::process::ExitStatus>,
    timed_out: bool,
    text: String,
    truncated: bool,
    stderr: String,
}

enum Msg {
    Out(String),
    Err(String),
}

/// Drive a piped child to completion or the deadline. Reader threads feed a
/// channel (lines capped at `MAX_COLLECTED`); on timeout the child is killed
/// and the readers are simply abandoned — they end when the pipes close.
fn supervise(mut child: Child, timeout: Duration) -> Supervised {
    let (tx, rx) = mpsc::channel::<Msg>();
    if let Some(out) = child.stdout.take() {
        let tx = tx.clone();
        std::thread::spawn(move || pump(out, &tx, Msg::Out));
    }
    if let Some(err) = child.stderr.take() {
        let tx = tx.clone();
        std::thread::spawn(move || pump(err, &tx, Msg::Err));
    }
    drop(tx);

    let deadline = Instant::now() + timeout;
    let mut run = Supervised::default();
    let mut exited_at: Option<Instant> = None;
    let mut drained = false;
    loop {
        if drained {
            std::thread::sleep(POLL);
        } else {
            match rx.recv_timeout(POLL) {
                Ok(Msg::Out(line)) => run.take_stdout(&line),
                Ok(Msg::Err(line)) => run.take_stderr(&line),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => drained = true,
            }
        }
        if exited_at.is_none() {
            if let Ok(Some(status)) = child.try_wait() {
                run.status = Some(status);
                exited_at = Some(Instant::now());
            }
        }
        match exited_at {
            Some(at) if drained || at.elapsed() >= EXIT_GRACE => break,
            Some(_) => {}
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                run.timed_out = true;
                break;
            }
            None => {}
        }
    }
    run
}

fn pump(stream: impl Read, tx: &mpsc::Sender<Msg>, wrap: fn(String) -> Msg) {
    let mut reader = BufReader::new(stream);
    loop {
        let mut buf = Vec::new();
        match reader
            .by_ref()
            .take(MAX_COLLECTED as u64)
            .read_until(b'\n', &mut buf)
        {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = String::from_utf8_lossy(&buf).trim_end().to_string();
        if tx.send(wrap(line)).is_err() {
            break;
        }
    }
}

impl Supervised {
    /// One stdout line of opencode's JSONL: collect every string under a
    /// `text`/`content`/`message` key; tolerate lines that are not JSON by
    /// keeping them verbatim.
    fn take_stdout(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let mut pieces = Vec::new();
        match serde_json::from_str::<Value>(line) {
            Ok(v) if line.starts_with(['{', '[']) => collect_text(&v, &mut pieces),
            _ => pieces.push(line.to_string()),
        }
        for piece in pieces {
            let piece = piece.trim();
            if piece.is_empty() {
                continue;
            }
            let sep = if self.text.is_empty() { 0 } else { 2 };
            if self.text.len() + sep + piece.len() > MAX_COLLECTED {
                self.truncated = true;
                return;
            }
            if sep > 0 {
                self.text.push_str("\n\n");
            }
            self.text.push_str(piece);
        }
    }

    /// Keep only the recent stderr (bounded memory for a chatty child).
    fn take_stderr(&mut self, line: &str) {
        self.stderr.push_str(line);
        self.stderr.push('\n');
        if self.stderr.len() > MAX_STDERR_TAIL * 4 {
            let keep = tail_of(&self.stderr, MAX_STDERR_TAIL).len();
            let cut = self.stderr.len() - keep;
            self.stderr.drain(..cut);
        }
    }
}

fn collect_text(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (key, val) in map {
                match val {
                    Value::String(s) if matches!(key.as_str(), "text" | "content" | "message") => {
                        out.push(s.clone())
                    }
                    Value::Object(_) | Value::Array(_) => collect_text(val, out),
                    _ => {}
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|i| collect_text(i, out)),
        _ => {}
    }
}

// --- small helpers ---------------------------------------------------------

/// GET the daemon's health JSON (2 s timeout). Never fails: an unreachable or
/// unreadable daemon is itself evidence, reported as `{"error": …}`.
fn daemon_health(url: &str) -> Value {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(HEALTH_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let mut resp = match agent.get(url).call() {
        Ok(r) => r,
        Err(e) => return json!({"error": format!("daemon health unreachable: {e}")}),
    };
    let status = resp.status();
    let text = match resp
        .body_mut()
        .with_config()
        .limit(32 * 1024)
        .read_to_string()
    {
        Ok(t) => t,
        Err(e) => return json!({"error": format!("daemon health unreadable: {e}")}),
    };
    if !status.is_success() {
        let snippet: String = text.trim().chars().take(200).collect();
        return json!({"error": format!("daemon health http status {}: {snippet}", status.as_u16())});
    }
    serde_json::from_str(&text)
        .unwrap_or_else(|_| json!({"error": "daemon health response is not JSON"}))
}

/// First non-empty line, markdown heading marks stripped, capped for a title.
fn title_of(text: &str) -> Option<String> {
    text.lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty())
        .map(|l| take_chars(l, TITLE_MAX))
}

fn first_line(s: &str) -> Option<&str> {
    s.lines().map(str::trim).find(|l| !l.is_empty())
}

fn take_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `s` cut to at most `max` bytes on a char boundary, marked when cut.
fn cap_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[... truncated ...]", &s[..end])
}

/// The last `max` bytes of `s`, on a char boundary.
fn tail_of(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_text_keys_at_any_depth_and_ignores_the_rest() {
        let v: Value = serde_json::from_str(
            r#"{"type":"x","part":{"text":"deep"},"items":[{"content":"in-array"}],"n":1,"message":"top"}"#,
        )
        .unwrap();
        let mut out = Vec::new();
        collect_text(&v, &mut out);
        out.sort();
        assert_eq!(out, ["deep", "in-array", "top"]);
    }

    #[test]
    fn title_strips_heading_marks_and_caps() {
        assert_eq!(title_of("\n\n## Summary\nbody").as_deref(), Some("Summary"));
        assert_eq!(title_of(&"x".repeat(200)).unwrap().len(), TITLE_MAX);
        assert_eq!(title_of(" \n#\n"), None);
    }

    #[test]
    fn executable_resolution_needs_the_exec_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("fake");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(resolve_executable(&file).is_none());
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(resolve_executable(&file).as_deref(), Some(file.as_path()));
        assert!(resolve_executable(dir.path()).is_none(), "dirs never count");
    }

    #[test]
    fn caps_respect_char_boundaries() {
        assert_eq!(tail_of("aé", 2), "é");
        assert!(cap_str("ééé", 3).starts_with('é'));
    }
}
