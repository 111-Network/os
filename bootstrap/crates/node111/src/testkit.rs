//! Test doubles shared by unit and integration tests (integration tests are separate
//! crates, so this lives in the library). Not part of the shipped behavior surface.
#![doc(hidden)]

use crate::util::run::{Output, Runner};
use std::collections::VecDeque;
use std::sync::Mutex;

/// Script-driven command runner: programs are matched by `program args...` string,
/// responses replayed in order per script entry. Panics on unexpected invocations
/// so tests can't silently pass on the wrong command.
pub struct MockRunner {
    pub script: Mutex<VecDeque<(String, Output)>>,
    pub calls: Mutex<Vec<String>>,
}

impl MockRunner {
    pub fn new(entries: Vec<(impl Into<String>, Output)>) -> Self {
        MockRunner {
            script: Mutex::new(entries.into_iter().map(|(k, v)| (k.into(), v)).collect()),
            calls: Mutex::new(Vec::new()),
        }
    }
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Runner for MockRunner {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        let cmd = format!("{program} {}", args.join(" "));
        self.calls.lock().unwrap().push(cmd.clone());
        todo!("T2: pop matching script entry for {cmd}")
    }
    fn run_as(&self, user: &str, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        let cmd = format!("sudo -u {user} {program} {}", args.join(" "));
        self.calls.lock().unwrap().push(cmd.clone());
        todo!("T2: pop matching script entry for {cmd}")
    }
}

/// An `install::probe::Env` backed by fixture files on disk (tests point it at a
/// fixture /proc+/sysfs tree).
pub struct FixtureEnv {
    pub root: std::path::PathBuf,
}

impl FixtureEnv {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        FixtureEnv { root: root.into() }
    }
}
