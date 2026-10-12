//! Configuration: defaults + `111_AGENT_*` env overrides (T0).
//! T1 adds `/etc/111/agent.env`, CLI flags, precedence, and the model-base guard.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub model_base: String,
    pub model_id: String,
    pub kb_dir: PathBuf,
    pub state_dir: PathBuf,
    pub category: String,
    pub daemon_health_url: String,
    pub opencode_path: PathBuf,
    pub lan: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: 11182,
            model_base: "http://127.0.0.1:11181".to_string(),
            model_id: "qwen3-4b".to_string(),
            kb_dir: PathBuf::from("/var/lib/111/kb/current"),
            state_dir: PathBuf::from("/var/lib/111/agent"),
            category: "unknown".to_string(),
            daemon_health_url: "http://127.0.0.1:11180/health".to_string(),
            opencode_path: PathBuf::from("opencode"),
            lan: false,
        }
    }
}

impl Config {
    /// T0: env overrides only. T1 owns the full precedence chain and guard.
    pub fn load() -> anyhow::Result<Self> {
        let mut cfg = Self::default();
        if let Some(v) = env_opt("111_AGENT_PORT") {
            cfg.port = v
                .parse()
                .map_err(|_| anyhow::anyhow!("bad 111_AGENT_PORT: {v}"))?;
        }
        if let Some(v) = env_opt("111_AGENT_MODEL_BASE") {
            cfg.model_base = v;
        }
        if let Some(v) = env_opt("111_AGENT_MODEL_ID") {
            cfg.model_id = v;
        }
        if let Some(v) = env_opt("111_AGENT_KB_DIR") {
            cfg.kb_dir = PathBuf::from(v);
        }
        if let Some(v) = env_opt("111_AGENT_STATE_DIR") {
            cfg.state_dir = PathBuf::from(v);
        }
        if let Some(v) = env_opt("111_AGENT_CATEGORY") {
            cfg.category = v;
        }
        if let Some(v) = env_opt("111_AGENT_DAEMON_HEALTH_URL") {
            cfg.daemon_health_url = v;
        }
        if let Some(v) = env_opt("111_AGENT_OPENCODE_PATH") {
            cfg.opencode_path = PathBuf::from(v);
        }
        if let Some(v) = env_opt("111_AGENT_LAN") {
            cfg.lan = matches!(v.as_str(), "1" | "true" | "yes");
        }
        Ok(cfg)
    }
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// True when an OpenAI-compatible base URL points at loopback (local model).
pub fn host_is_loopback(base: &str) -> bool {
    match host_part(base) {
        Some(h) => h == "127.0.0.1" || h == "localhost" || h == "::1" || h == "[::1]",
        None => false,
    }
}

/// True when an OpenAI-compatible base URL points at a private/mesh address
/// (hub offload). T1 implements the actual ranges; the guard depends on it.
pub fn host_is_private(_base: &str) -> bool {
    // T1: RFC1918 + ULA + link-local. T0 refuses everything non-loopback.
    false
}

/// Extract the host portion of a URL, without the port or scheme.
fn host_part(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let hostport = after_scheme.split('/').next()?;
    if hostport.is_empty() {
        return None;
    }
    if let Some(rest) = hostport.strip_prefix('[') {
        // [v6]:port
        let end = rest.find(']')?;
        return Some(format!("[{}]", &rest[..end]));
    }
    Some(hostport.split(':').next()?.to_string())
}
