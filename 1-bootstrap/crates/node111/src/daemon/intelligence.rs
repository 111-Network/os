//! Layer 3 children (Local Intelligence): the daemon supervises `llama-server`
//! (model) and `111-agent` (agent API) exactly like it supervises rnsd, through
//! the existing [`supervise::spawn_child`]. Both are conditional on files the
//! release/model install put on disk; a missing piece is an honest `absent`
//! (relay and Tier-2A nodes carry no model), never an error.
//!
//! Wiring contract (os/3-intelligence/TASKS.md "Cross-layer wiring"):
//! - `llama-server` runs iff `current/bin/llama-server` exists AND a `*.gguf`
//!   exists directly under `<root>/models/current/` (first by file name).
//! - `111-agent` runs iff `current/bin/111-agent` exists. Its environment
//!   carries category, KB dir, state dir, env file, and the PUBLIC node id.
//!   No key material ever reaches it.
//! - Layer 5 §17: the agent reaches `111d`'s own `/api/v1` on loopback
//!   ([`DAEMON_API_URL`], env `AGENT111_DAEMON_API_URL`) with a bearer token
//!   minted when the agent is spawned ([`DaemonToken::mint`], env
//!   `AGENT111_DAEMON_TOKEN`). The token is a session credential for that API,
//!   not key material: it never touches disk, and `require_session` accepts it
//!   from a loopback peer only. The supervisor reuses the spawn's environment
//!   for crash restarts, so one token lives as long as this handle; the daemon
//!   start hook puts the same token into `ApiState` ext
//!   ([`IntelligenceHandle::daemon_token`]).
//! - Both log to `<root>/agent/`, which is created 0700.

use crate::daemon::dashboard::api_v1::auth::{DaemonToken, DAEMON_TOKEN_ENV};
use crate::daemon::supervise::{
    self, BackoffPolicy, ChildControl, ChildHandle, ChildSpec, ChildState,
};
use crate::daemon::DaemonLayout;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Loopback port `llama-server` listens on (the agent's default model base).
pub const MODEL_PORT: u16 = 11181;

/// Loopback port `111-agent` listens on (see [`AGENT_URL`]).
pub const AGENT_PORT: u16 = 11182;

/// Where the agent API listens (loopback only); published in `/api/status`
/// while the agent is running so Layer 4 can relay to it.
pub const AGENT_URL: &str = "http://127.0.0.1:11182";

/// `111d`'s own `/api/v1` base as the agent reaches it (loopback, the
/// dashboard's [`crate::daemon::dashboard::DEFAULT_PORT`]; Layer 5 §17).
pub const DAEMON_API_URL: &str = "http://127.0.0.1:11180/api/v1";

/// Child-environment name of [`DAEMON_API_URL`].
pub const DAEMON_API_URL_ENV: &str = "AGENT111_DAEMON_API_URL";

/// Display state of one Layer 3 child (pinned strings in `/api/status`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChildDisplay {
    /// Not started: the binary (or model) is not installed on this node.
    #[default]
    Absent,
    Running,
    Backoff,
    Degraded,
    /// Stopped by the daemon (shutdown path).
    Stopped,
}

impl From<Option<ChildState>> for ChildDisplay {
    fn from(state: Option<ChildState>) -> Self {
        match state {
            None => ChildDisplay::Absent,
            Some(ChildState::Running) => ChildDisplay::Running,
            Some(ChildState::Backoff) => ChildDisplay::Backoff,
            Some(ChildState::Degraded) => ChildDisplay::Degraded,
            Some(ChildState::Stopped) => ChildDisplay::Stopped,
        }
    }
}

/// The additive `intelligence` object of `/api/status`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IntelligenceState {
    pub agent: ChildDisplay,
    pub model_server: ChildDisplay,
    /// `Some(AGENT_URL)` iff the agent is running.
    pub agent_url: Option<String>,
}

/// Owner of the two Layer 3 supervisors (either may be absent).
pub struct IntelligenceHandle {
    agent: Option<ChildHandle>,
    model: Option<ChildHandle>,
    /// The agent's bearer token (Layer 5 §17); `None` when no agent was
    /// spawned or the OS random source failed (then no bearer is accepted).
    daemon_token: Option<DaemonToken>,
}

impl IntelligenceHandle {
    /// Current display state (cheap; the daemon mirrors it into the dashboard).
    pub fn state(&self) -> IntelligenceState {
        let agent = ChildDisplay::from(self.agent.as_ref().map(|c| c.state()));
        let model_server = ChildDisplay::from(self.model.as_ref().map(|c| c.state()));
        let agent_url = (agent == ChildDisplay::Running).then(|| AGENT_URL.to_string());
        IntelligenceState {
            agent,
            model_server,
            agent_url,
        }
    }

    /// Current `111-agent` PID while running (Layer 4 services registry uses
    /// PID changes to time its `starting` grace after each spawn).
    pub fn agent_pid(&self) -> Option<u32> {
        self.agent.as_ref().and_then(|c| c.pid())
    }

    /// Current `llama-server` PID while running (see [`Self::agent_pid`]).
    pub fn model_pid(&self) -> Option<u32> {
        self.model.as_ref().and_then(|c| c.pid())
    }

    /// Control of the `111-agent` child (`None` when absent).
    pub fn agent_control(&self) -> Option<ChildControl> {
        self.agent.as_ref().map(ChildHandle::control)
    }

    /// Control of the `llama-server` child (`None` when absent).
    pub fn model_control(&self) -> Option<ChildControl> {
        self.model.as_ref().map(ChildHandle::control)
    }

    /// The bearer token handed to the agent at spawn (Layer 5 §17). The daemon
    /// start hook inserts it into `ApiState` ext so `require_session` can
    /// check it; `None` ⇒ every bearer is refused.
    pub fn daemon_token(&self) -> Option<DaemonToken> {
        self.daemon_token.clone()
    }

    /// SIGTERM both children and stop restarting them, without waiting.
    pub fn stop(&self) {
        for child in [&self.agent, &self.model].into_iter().flatten() {
            child.stop();
        }
    }

    /// Daemon shutdown: stop both children and join them (SIGTERM, `grace`,
    /// SIGKILL — [`ChildControl::stop_and_wait`]), concurrently.
    pub async fn stop_and_wait(&self, grace: std::time::Duration) {
        let agent = async {
            if let Some(c) = &self.agent {
                c.stop_and_wait(grace).await;
            }
        };
        let model = async {
            if let Some(c) = &self.model {
                c.stop_and_wait(grace).await;
            }
        };
        tokio::join!(agent, model);
    }
}

/// First `*.gguf` (by file name) directly under `<root>/models/current/`.
fn first_gguf(layout: &DaemonLayout) -> Option<PathBuf> {
    let dir = layout.root.join("models/current");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "gguf") && p.is_file())
        .filter_map(|p| p.file_name().map(|n| n.to_owned()))
        .collect();
    names.sort();
    names.into_iter().next().map(|n| dir.join(n))
}

fn llama_server_spec(layout: &DaemonLayout) -> Option<ChildSpec> {
    let program = layout.current_release().join("bin/llama-server");
    if !program.is_file() {
        eprintln!("111 intelligence: no llama-server in current/bin — model server absent");
        return None;
    }
    let Some(model) = first_gguf(layout) else {
        eprintln!("111 intelligence: no .gguf under models/current — model server absent");
        return None;
    };
    Some(ChildSpec {
        name: "llama-server".into(),
        program,
        args: vec![
            "--model".into(),
            model.to_string_lossy().into_owned(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            MODEL_PORT.to_string(),
        ],
        env: vec![],
        backoff: BackoffPolicy::default(),
        stdout_log: Some(agent_dir(layout).join("llama-server.log")),
    })
}

fn agent_spec(
    layout: &DaemonLayout,
    category: Option<&str>,
    node_id: Option<&str>,
    daemon_token: Option<&DaemonToken>,
) -> Option<ChildSpec> {
    let program = layout.current_release().join("bin/111-agent");
    if !program.is_file() {
        eprintln!("111 intelligence: no 111-agent in current/bin — agent absent");
        return None;
    }
    let path_str = |p: PathBuf| p.to_string_lossy().into_owned();
    let mut env = vec![
        (
            "AGENT111_CATEGORY".to_string(),
            category
                .filter(|c| !c.is_empty())
                .unwrap_or("unknown")
                .to_string(),
        ),
        (
            "AGENT111_KB_DIR".to_string(),
            path_str(layout.current_release().join("share/kb")),
        ),
        (
            "AGENT111_STATE_DIR".to_string(),
            path_str(agent_dir(layout)),
        ),
        (
            "AGENT111_ENV_FILE".to_string(),
            path_str(layout.etc.join("agent.env")),
        ),
        (DAEMON_API_URL_ENV.to_string(), DAEMON_API_URL.to_string()),
    ];
    if let Some(token) = daemon_token {
        env.push((DAEMON_TOKEN_ENV.to_string(), token.as_str().to_string()));
    }
    if let Some(id) = node_id.filter(|i| !i.is_empty()) {
        env.push(("AGENT111_NODE_ID".to_string(), id.to_string()));
    }
    Some(ChildSpec {
        name: "111-agent".into(),
        program,
        args: vec![],
        env,
        backoff: BackoffPolicy::default(),
        stdout_log: Some(agent_dir(layout).join("111-agent.log")),
    })
}

/// Agent state + log directory (`<root>/agent`).
fn agent_dir(layout: &DaemonLayout) -> PathBuf {
    layout.root.join("agent")
}

/// Paths the Layer 4 services registry reports (one source of truth with the
/// child specs above).
pub mod paths {
    use super::*;

    /// `current/bin/111-agent` (its presence = Layer 3 installed).
    pub fn agent_binary(layout: &DaemonLayout) -> PathBuf {
        layout.current_release().join("bin/111-agent")
    }

    /// `current/bin/llama-server`.
    pub fn model_binary(layout: &DaemonLayout) -> PathBuf {
        layout.current_release().join("bin/llama-server")
    }

    /// The model file `llama-server` is (or would be) started with.
    pub fn model_file(layout: &DaemonLayout) -> Option<PathBuf> {
        first_gguf(layout)
    }

    /// `<root>/agent/111-agent.log` (child stdout+stderr).
    pub fn agent_log(layout: &DaemonLayout) -> PathBuf {
        agent_dir(layout).join("111-agent.log")
    }

    /// `<root>/agent/llama-server.log` (child stdout+stderr).
    pub fn model_log(layout: &DaemonLayout) -> PathBuf {
        agent_dir(layout).join("llama-server.log")
    }
}

/// Create the agent dir 0700 (and tighten it if it pre-existed wider). A
/// failure is logged, not fatal: the supervisor falls back to inheriting
/// stdio and the agent creates its own state dir.
fn ensure_private_dir(dir: &Path) {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let made = std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .and_then(|()| std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)));
    if let Err(e) = made {
        eprintln!("111 intelligence: cannot prepare {}: {e}", dir.display());
    }
}

/// Start the Layer 3 children that this node can run. `category` and `node_id`
/// come from the daemon's already-resolved state (`node_id` is the public
/// NodeId hex). Must be called within a tokio runtime (the supervisors are
/// tasks). Never fails: whatever cannot run is reported `absent`.
pub fn start(
    layout: &DaemonLayout,
    category: Option<&str>,
    node_id: Option<&str>,
) -> IntelligenceHandle {
    let llama = llama_server_spec(layout);
    // Minted only when an agent will be spawned (Layer 5 §17): no agent, no
    // token, no bearer accepted.
    let daemon_token = if paths::agent_binary(layout).is_file() {
        DaemonToken::mint()
            .inspect_err(|e| {
                eprintln!("111 intelligence: {e} — the agent starts without daemon API access")
            })
            .ok()
    } else {
        None
    };
    let agent = agent_spec(layout, category, node_id, daemon_token.as_ref());
    if llama.is_some() || agent.is_some() {
        ensure_private_dir(&agent_dir(layout));
    }
    let daemon_token = daemon_token.filter(|_| agent.is_some());
    IntelligenceHandle {
        model: llama.map(supervise::spawn_child),
        agent: agent.map(supervise::spawn_child),
        daemon_token,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn layout(root: &Path) -> DaemonLayout {
        DaemonLayout {
            root: root.to_path_buf(),
            etc: root.join("etc"),
        }
    }

    fn touch_exec(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn display_mapping_and_json_shape() {
        assert_eq!(ChildDisplay::from(None), ChildDisplay::Absent);
        assert_eq!(
            ChildDisplay::from(Some(ChildState::Running)),
            ChildDisplay::Running
        );
        assert_eq!(
            ChildDisplay::from(Some(ChildState::Backoff)),
            ChildDisplay::Backoff
        );
        assert_eq!(
            ChildDisplay::from(Some(ChildState::Degraded)),
            ChildDisplay::Degraded
        );
        assert_eq!(
            ChildDisplay::from(Some(ChildState::Stopped)),
            ChildDisplay::Stopped
        );

        let absent = serde_json::to_value(IntelligenceState::default()).unwrap();
        assert_eq!(
            absent,
            serde_json::json!({"agent": "absent", "model_server": "absent", "agent_url": null})
        );
        let up = serde_json::to_value(IntelligenceState {
            agent: ChildDisplay::Running,
            model_server: ChildDisplay::Degraded,
            agent_url: Some(AGENT_URL.into()),
        })
        .unwrap();
        assert_eq!(
            up,
            serde_json::json!({
                "agent": "running",
                "model_server": "degraded",
                "agent_url": "http://127.0.0.1:11182"
            })
        );
    }

    #[test]
    fn llama_spec_needs_binary_and_model() {
        let dir = tempfile::tempdir().unwrap();
        let l = layout(dir.path());
        assert!(llama_server_spec(&l).is_none(), "nothing installed");

        touch_exec(&dir.path().join("current/bin/llama-server"));
        assert!(llama_server_spec(&l).is_none(), "binary but no model");

        let models = dir.path().join("models/current");
        std::fs::create_dir_all(&models).unwrap();
        std::fs::write(models.join("readme.txt"), "not a model").unwrap();
        assert!(llama_server_spec(&l).is_none(), "no .gguf");

        std::fs::write(models.join("zz.gguf"), "x").unwrap();
        std::fs::write(models.join("aa.gguf"), "x").unwrap();
        let spec = llama_server_spec(&l).expect("binary + model");
        assert_eq!(spec.name, "llama-server");
        assert_eq!(spec.program, dir.path().join("current/bin/llama-server"));
        assert_eq!(
            spec.args,
            [
                "--model",
                models.join("aa.gguf").to_str().unwrap(),
                "--host",
                "127.0.0.1",
                "--port",
                "11181"
            ]
        );
        assert!(spec.env.is_empty());
        assert_eq!(
            spec.stdout_log,
            Some(dir.path().join("agent/llama-server.log"))
        );

        // A model with no binary is equally absent.
        std::fs::remove_file(dir.path().join("current/bin/llama-server")).unwrap();
        assert!(llama_server_spec(&l).is_none(), "model but no binary");
    }

    #[test]
    fn agent_spec_env_contract() {
        let dir = tempfile::tempdir().unwrap();
        let l = layout(dir.path());
        assert!(agent_spec(&l, Some("Node2A"), None, None).is_none());

        touch_exec(&dir.path().join("current/bin/111-agent"));
        let root = dir.path().display();
        let token = DaemonToken::mint().unwrap();
        let spec = agent_spec(&l, Some("Node2A"), Some("abc123"), Some(&token)).unwrap();
        assert_eq!(spec.name, "111-agent");
        assert!(spec.args.is_empty());
        assert_eq!(
            spec.env,
            [
                ("AGENT111_CATEGORY".to_string(), "Node2A".to_string()),
                (
                    "AGENT111_KB_DIR".to_string(),
                    format!("{root}/current/share/kb")
                ),
                ("AGENT111_STATE_DIR".to_string(), format!("{root}/agent")),
                (
                    "AGENT111_ENV_FILE".to_string(),
                    format!("{root}/etc/agent.env")
                ),
                (
                    "AGENT111_DAEMON_API_URL".to_string(),
                    "http://127.0.0.1:11180/api/v1".to_string()
                ),
                (
                    "AGENT111_DAEMON_TOKEN".to_string(),
                    token.as_str().to_string()
                ),
                ("AGENT111_NODE_ID".to_string(), "abc123".to_string()),
            ]
        );
        assert_eq!(
            spec.stdout_log,
            Some(dir.path().join("agent/111-agent.log"))
        );

        // Unknown category, no node id, no token: category defaults, node id
        // and token omitted (the API URL is always there).
        for (cat, id) in [(None, None), (Some(""), Some(""))] {
            let spec = agent_spec(&l, cat, id, None).unwrap();
            assert_eq!(spec.env.len(), 5, "{:?}", spec.env);
            assert_eq!(spec.env[0].1, "unknown");
            assert!(spec.env.iter().all(|(k, _)| k != "AGENT111_NODE_ID"));
            assert!(spec.env.iter().all(|(k, _)| k != DAEMON_TOKEN_ENV));
        }
    }

    #[test]
    fn daemon_api_url_is_the_dashboard_port_on_loopback() {
        assert_eq!(
            DAEMON_API_URL,
            format!(
                "http://127.0.0.1:{}/api/v1",
                crate::daemon::dashboard::DEFAULT_PORT
            )
        );
    }

    #[test]
    fn private_dir_is_0700_even_if_it_existed_wider() {
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        std::fs::create_dir(&agent).unwrap();
        std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).unwrap();
        ensure_private_dir(&agent);
        assert_eq!(
            agent.metadata().unwrap().permissions().mode() & 0o777,
            0o700
        );
        let fresh = dir.path().join("a/b");
        ensure_private_dir(&fresh);
        assert_eq!(
            fresh.metadata().unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
