//! Task engine — the Layer 9 seam. diagnose/propose run the model loop and
//! land in a proposal; patch delegates to opencode on hubs. Engine lands in
//! T6; the refusal semantics (`needs hub`) are contract and live here already.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::events::Journal;
use crate::model::Client;
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskKind {
    Diagnose,
    Propose,
    Patch,
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

pub struct TaskEngine {
    cfg: Arc<Config>,
    store: Arc<Store>,
    journal: Journal,
    model: Arc<Client>,
}

impl TaskEngine {
    pub fn new(cfg: Arc<Config>, store: Arc<Store>, journal: Journal, model: Arc<Client>) -> Self {
        Self {
            cfg,
            store,
            journal,
            model,
        }
    }

    /// Submit a task; returns the task id. Synchronous validation, async work
    /// (T6 spawns the run; T0 only models the honest refusals).
    pub fn submit(
        &self,
        kind: TaskKind,
        _refs: Vec<String>,
        _note: Option<String>,
    ) -> Result<String, SubmitError> {
        match kind {
            TaskKind::Patch => Err(SubmitError::NeedsHub), // opencode delegation lands in T6
            _ => Err(SubmitError::Other(anyhow::anyhow!(
                "task engine lands in T6"
            ))),
        }
    }

    pub fn open_count(&self) -> usize {
        0 // T6: count running tasks from tasks/*.json
    }

    pub fn cfg(&self) -> &Config {
        &self.cfg
    }
    pub fn store(&self) -> &Store {
        &self.store
    }
    pub fn journal(&self) -> &Journal {
        &self.journal
    }
    pub fn model(&self) -> &Client {
        &self.model
    }
}
