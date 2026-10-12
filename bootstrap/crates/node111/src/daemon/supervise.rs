//! Supervision tree: restart-in-place per child with exponential backoff + cap,
//! crash-loop detection (→ dashboard "degraded"), the Layer 9 seam.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct BackoffPolicy {
    pub initial_ms: u64,
    pub max_ms: u64,
    pub factor: f64,
    pub jitter_pct: u32,
    /// Restarts within `crash_loop_window_secs` that mark the child Degraded.
    pub crash_loop_threshold: u32,
    pub crash_loop_window_secs: u64,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        BackoffPolicy {
            initial_ms: 250,
            max_ms: 60_000,
            factor: 2.0,
            jitter_pct: 20,
            crash_loop_threshold: 5,
            crash_loop_window_secs: 60,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChildSpec {
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub backoff: BackoffPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildState {
    Running,
    Backoff,
    Degraded,
    Stopped,
}

pub struct ChildHandle {
    // task handle + state watch channel — construction detail is T7's.
    _priv: (),
}

impl ChildHandle {
    pub fn state(&self) -> ChildState {
        todo!("T7")
    }
}

/// Spawn a supervised child: restart-in-place, backoff per policy, sustained
/// crash-loop ⇒ Degraded (surfaced by the dashboard, not fatal to the daemon).
pub fn spawn_child(spec: ChildSpec) -> ChildHandle {
    let _ = spec;
    todo!("T7")
}
