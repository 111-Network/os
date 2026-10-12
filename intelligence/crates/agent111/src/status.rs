//! GET /v1/agent/status body and the local|offloaded|unavailable derivation.

use serde::Serialize;

use crate::config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Local,
    Offloaded,
    Unavailable,
}

impl AgentState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Offloaded => "offloaded",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Honest state: unreachable model ⇒ unavailable; loopback base ⇒ local;
/// anything else (a private hub address) ⇒ offloaded.
pub fn derive_state(model_base: &str, reachable: bool) -> AgentState {
    if !reachable {
        return AgentState::Unavailable;
    }
    if config::host_is_loopback(model_base) {
        AgentState::Local
    } else {
        AgentState::Offloaded
    }
}

#[derive(Debug, Serialize)]
pub struct ModelBody {
    pub base: String,
    pub reachable: bool,
    pub id: String,
}

#[derive(Debug, Serialize)]
pub struct KbBody {
    pub present: bool,
    pub entries: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct StatusBody {
    pub schema: u8,
    pub state: &'static str,
    pub model: ModelBody,
    pub kb: KbBody,
    pub conversations: usize,
    pub tasks_open: usize,
    pub uptime_sec: u64,
    pub version: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_derivation() {
        assert_eq!(
            derive_state("http://127.0.0.1:11181", false),
            AgentState::Unavailable
        );
        assert_eq!(
            derive_state("http://127.0.0.1:11181", true),
            AgentState::Local
        );
        assert_eq!(
            derive_state("http://10.0.0.5:11181", true),
            AgentState::Offloaded
        );
    }
}
