//! `111 daemon` — the node kernel: tokio supervision tree (systemd/runit are thin
//! Restart=always wrappers, never the supervisor of record), watchdog heartbeat,
//! dashboard, updater, KB server, conditional rnsd child.
//!
//! Final wiring of `run()` lands in T14 once dashboard/updater/transport exist;
//! this module provides the shared pieces.

pub mod dashboard;
pub mod health;
pub mod kb;
pub mod supervise;
pub mod transport;
pub mod updater;
pub mod watchdog;

use crate::install::categorize::{categorize, Classification, Refusal};
use crate::install::probe::{self, Env, Facts};
use std::path::{Path, PathBuf};

/// Runtime layout resolved once at daemon start.
#[derive(Debug, Clone)]
pub struct DaemonLayout {
    /// Node root: $NODE111_ROOT or /var/lib/111.
    pub root: PathBuf,
    /// Config dir: /etc/111 (host) — under Termux a sibling of root.
    pub etc: PathBuf,
}

impl DaemonLayout {
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }
    pub fn identity_dir(&self) -> PathBuf {
        self.root.join("identity")
    }
    pub fn current_release(&self) -> PathBuf {
        self.root.join("current")
    }
}

/// Parse `/etc/111/env`-style KEY=VALUE files (comments + blanks skipped).
pub fn parse_env_file(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Load node config from `<etc>/env` (missing file = empty config, not an error).
pub fn load_env(etc: &Path) -> anyhow::Result<Vec<(String, String)>> {
    match std::fs::read_to_string(etc.join("env")) {
        Ok(text) => Ok(parse_env_file(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// Category re-probe (daemon start + udev trigger, spec §Detection): collect
/// facts, reclassify, persist facts.json (best-effort write — probing must not
/// kill the daemon).
pub fn reprobe(env: &dyn Env, layout: &DaemonLayout) -> (Facts, Result<Classification, Refusal>) {
    let facts = probe::collect(env);
    let class = categorize(&facts);
    if let Ok(class) = &class {
        let _ = probe::write_facts_json(&layout.state_dir().join("facts.json"), &facts, class);
    }
    (facts, class)
}

pub fn run() -> anyhow::Result<()> {
    todo!("T14 final wiring: supervise children, dashboard, updater, transport")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_file_parsing() {
        let parsed = parse_env_file("# comment\n\nDASHBOARD_LAN=true\nNODE111_X= y \n");
        assert_eq!(
            parsed,
            vec![
                ("DASHBOARD_LAN".to_string(), "true".to_string()),
                ("NODE111_X".to_string(), "y".to_string())
            ]
        );
    }
}
