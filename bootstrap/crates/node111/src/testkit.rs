//! Test doubles shared by unit and integration tests (integration tests are separate
//! crates, so this lives in the library). Not part of the shipped behavior surface.
#![doc(hidden)]

use crate::util::run::{Output, Runner};
use std::collections::VecDeque;
use std::sync::Mutex;

/// Script-driven command runner: programs are matched by `program args...` string,
/// responses replayed in order per script entry. Panics on unexpected invocations
/// so tests can't silently pass on the wrong command.
///
/// Script keys are matched strictly in FIFO order against the exact
/// `"prog arg1 arg2"` invocation string (trailing whitespace ignored, so a
/// zero-arg entry is keyed plainly as `"prog"`). `run_as` invocations are keyed
/// with the full `"sudo -u <user> prog args..."` prefix.
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

    /// Assert the whole script was consumed (call at the end of a test).
    pub fn assert_exhausted(&self) {
        let remaining = self.script.lock().unwrap();
        assert!(
            remaining.is_empty(),
            "MockRunner script not exhausted, {} entr(ies) left: {:?}",
            remaining.len(),
            remaining.iter().map(|(k, _)| k).collect::<Vec<_>>()
        );
    }

    fn format_cmd(program: &str, args: &[&str]) -> String {
        if args.is_empty() {
            program.to_string()
        } else {
            format!("{program} {}", args.join(" "))
        }
    }

    fn pop_matching(&self, cmd: &str) -> anyhow::Result<Output> {
        let mut script = self.script.lock().unwrap();
        let front_matches = script
            .front()
            .is_some_and(|(key, _)| key.trim_end() == cmd.trim_end());
        if front_matches {
            return Ok(script.pop_front().unwrap().1);
        }
        panic!(
            "MockRunner: unexpected invocation {cmd:?}\n  remaining script: {:?}",
            script.iter().map(|(k, _)| k).collect::<Vec<_>>()
        );
    }
}

impl Runner for MockRunner {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        let cmd = Self::format_cmd(program, args);
        self.calls.lock().unwrap().push(cmd.clone());
        self.pop_matching(&cmd)
    }
    fn run_as(&self, user: &str, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        let cmd = Self::format_cmd(&format!("sudo -u {user} {program}"), args);
        self.calls.lock().unwrap().push(cmd.clone());
        self.pop_matching(&cmd)
    }
}

/// In-memory `install::probe::Env` for probe/categorize tests: fake files, fake
/// dir listings, fake command outputs. Unlisted paths behave as absent (Err),
/// so every probe degrades to "unknown" unless the test arranges otherwise.
#[derive(Default)]
pub struct MapEnv {
    pub files: std::collections::HashMap<String, String>,
    pub dirs: std::collections::HashMap<String, Vec<String>>,
    /// Exact "prog arg1 arg2" → stdout. Failing commands = absent key.
    pub cmds: std::collections::HashMap<String, String>,
}

impl MapEnv {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn file(mut self, path: &str, content: &str) -> Self {
        self.files.insert(path.to_string(), content.to_string());
        self
    }
    pub fn dir(mut self, path: &str, entries: &[&str]) -> Self {
        self.dirs.insert(
            path.to_string(),
            entries.iter().map(|s| s.to_string()).collect(),
        );
        self
    }
    pub fn cmd(mut self, cmdline: &str, stdout: &str) -> Self {
        self.cmds.insert(cmdline.to_string(), stdout.to_string());
        self
    }
}

impl crate::install::probe::Env for MapEnv {
    fn read_file(&self, path: &str) -> std::io::Result<String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, path.to_string()))
    }
    fn read_dir(&self, path: &str) -> std::io::Result<Vec<String>> {
        self.dirs
            .get(path)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, path.to_string()))
    }
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<String> {
        let key = if args.is_empty() {
            program.to_string()
        } else {
            format!("{program} {}", args.join(" "))
        };
        self.cmds
            .get(&key)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(status: i32, stdout: &str) -> Output {
        Output {
            status,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    #[test]
    fn replays_script_in_order() {
        let r = MockRunner::new(vec![
            ("probe --all", out(0, "facts")),
            ("install pkg", out(0, "")),
        ]);
        assert_eq!(r.run("probe", &["--all"]).unwrap().stdout, "facts");
        assert!(r.run("install", &["pkg"]).unwrap().ok());
        assert_eq!(r.calls(), vec!["probe --all", "install pkg"]);
        r.assert_exhausted();
    }

    #[test]
    fn run_as_keys_on_sudo_prefix_and_zero_arg_calls_match() {
        let r = MockRunner::new(vec![
            ("sudo -u node111 id -u", out(0, "999")),
            ("id", out(0, "0")),
        ]);
        assert_eq!(r.run_as("node111", "id", &["-u"]).unwrap().stdout, "999");
        assert_eq!(r.run("id", &[]).unwrap().stdout, "0");
        assert_eq!(
            r.calls(),
            vec!["sudo -u node111 id -u", "id"] // recorded calls have no trailing space issue
        );
        r.assert_exhausted();
    }

    #[test]
    #[should_panic(expected = "MockRunner: unexpected invocation")]
    fn panics_on_unscripted_call_and_lists_remaining() {
        let r = MockRunner::new(vec![("good cmd", out(0, ""))]);
        let _ = r.run("bad", &["cmd"]);
    }

    #[test]
    #[should_panic(expected = "MockRunner: unexpected invocation")]
    fn panics_on_out_of_order_call() {
        let r = MockRunner::new(vec![("first", out(0, "")), ("second", out(0, ""))]);
        let _ = r.run("second", &[]);
    }
}
