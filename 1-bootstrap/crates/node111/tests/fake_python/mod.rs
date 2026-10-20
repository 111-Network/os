//! Fake `python3` and venv `pip` for the presence-venv tests
//! (`daemon::transport::venv::presence`): real shell scripts in a temp dir,
//! run by the real `SystemRunner`. `python3` resolves to the fake through
//! [`FakePython`]'s `Runner` (as if it came first on PATH, without touching
//! the process environment that other tests share); a venv's `bin/pip` is the
//! file the fake `python3 -m venv` wrote, run by its path.
//!
//! - `python3 --version` · `python3 -m venv DIR` (writes `DIR/bin/python` and
//!   `DIR/bin/pip`) · `python3 -m pip ...` (the fake pip; with
//!   [`FakePython::no_host_pip`], "No module named pip" like a fresh Debian).
//! - `pip --version` · `pip download ...` · `pip install ... -r LOCK ...`
//!   (copies LOCK to `<venv>/installed-lock.txt`, so a test sees which lock a
//!   venv was built from).
//! - Switches: [`FakePython::set_missing`] (no `python3` at all: spawning it
//!   fails), [`FakePython::fail_install`] (pip reports a hash mismatch),
//!   [`FakePython::fail_download`], [`FakePython::fail_venv`],
//!   [`FakePython::no_host_pip`].
//! - Every call is appended to `calls.log` ([`FakePython::calls`]):
//!   `python3 <args>` or `<pip path> <args>`.

#![allow(dead_code)] // each test crate uses a different subset

use node111::testkit::MockRunner;
use node111::util::run::{Output, Runner, SystemRunner};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct FakePython {
    dir: tempfile::TempDir,
}

impl FakePython {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path().to_string_lossy().into_owned();
        assert!(!t.contains('\''), "temp dir must not contain a quote: {t}");
        let python3 = format!(
            r#"#!/bin/sh
T='{t}'
printf 'python3 %s\n' "$*" >> "$T/calls.log"
case "$1" in
  --version) echo "Python 3.10.12"; exit 0 ;;
  -m)
    case "$2" in
      venv)
        if [ -e "$T/venv-fail" ]; then
          echo "Error: Command '-m ensurepip' returned non-zero exit status 1." >&2
          exit 1
        fi
        mkdir -p "$3/bin" || exit 1
        printf '#!/bin/sh\nexit 0\n' > "$3/bin/python"
        chmod 755 "$3/bin/python"
        cp "$T/pip" "$3/bin/pip" && chmod 755 "$3/bin/pip"
        exit 0 ;;
      pip)
        if [ -e "$T/no-host-pip" ]; then
          echo "/usr/bin/python3: No module named pip" >&2
          exit 1
        fi
        shift 2; exec "$T/pip" "$@" ;;
    esac ;;
esac
echo "fake python3: unexpected arguments: $*" >&2
exit 2
"#
        );
        let pip = format!(
            r#"#!/bin/sh
T='{t}'
printf '%s %s\n' "$0" "$*" >> "$T/calls.log"
lock=""
prev=""
for a in "$@"; do
  if [ "$prev" = "-r" ]; then lock="$a"; fi
  prev="$a"
done
case "$1" in
  --version) echo "pip 24.0"; exit 0 ;;
  download)
    if [ -e "$T/download-fail" ]; then
      echo "ERROR: Could not find a version that satisfies the requirement meshtastic==2.7.11" >&2
      exit 1
    fi
    exit 0 ;;
  install)
    if [ -e "$T/install-fail" ]; then
      printf 'ERROR: THESE PACKAGES DO NOT MATCH THE HASHES FROM THE REQUIREMENTS FILE.\n    meshtastic==2.7.11:\n        Expected sha256 aa\n             Got        bb\n' >&2
      exit 1
    fi
    venv="$(dirname "$(dirname "$0")")"
    cp "$lock" "$venv/installed-lock.txt" || exit 1
    exit 0 ;;
esac
echo "fake pip: unexpected arguments: $*" >&2
exit 2
"#
        );
        for (name, body) in [("python3", python3), ("pip", pip)] {
            let path = dir.path().join(name);
            std::fs::write(&path, body).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        FakePython { dir }
    }

    fn flag(&self, name: &str, on: bool) {
        let path = self.dir.path().join(name);
        if on {
            std::fs::write(path, b"").unwrap();
        } else {
            let _ = std::fs::remove_file(path);
        }
    }

    /// No `python3` on this machine: spawning it fails.
    pub fn set_missing(&self, on: bool) {
        self.flag("missing", on);
    }

    /// `pip install` fails like pip on a hash mismatch.
    pub fn fail_install(&self, on: bool) {
        self.flag("install-fail", on);
    }

    /// `pip download` fails (no network, no such version).
    pub fn fail_download(&self, on: bool) {
        self.flag("download-fail", on);
    }

    /// `python3 -m venv` fails (no `ensurepip`).
    pub fn fail_venv(&self, on: bool) {
        self.flag("venv-fail", on);
    }

    /// The host interpreter has no pip module (`python3 -m pip` fails as on a
    /// fresh Debian/Ubuntu with `python3-venv` but not `python3-pip`); a
    /// venv's own `bin/pip` still works.
    pub fn no_host_pip(&self, on: bool) {
        self.flag("no-host-pip", on);
    }

    /// Every call so far, in order.
    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("calls.log"))
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    fn program(&self, program: &str) -> PathBuf {
        if program != "python3" {
            return PathBuf::from(program);
        }
        if self.dir.path().join("missing").exists() {
            self.dir.path().join("absent/python3")
        } else {
            self.dir.path().join("python3")
        }
    }
}

impl Runner for FakePython {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        SystemRunner.run(&self.program(program).to_string_lossy(), args)
    }

    fn run_as(&self, user: &str, program: &str, _args: &[&str]) -> anyhow::Result<Output> {
        anyhow::bail!("fake python: run_as {user} {program} is not supported")
    }
}

/// `python3` and every `*/bin/pip` go to the [`FakePython`]; everything else
/// to the scripted [`MockRunner`]. `order` records both, so a test can assert
/// the order across them.
pub struct Routed<'a> {
    pub mock: &'a MockRunner,
    pub py: &'a FakePython,
    pub order: Mutex<Vec<String>>,
}

impl<'a> Routed<'a> {
    pub fn new(mock: &'a MockRunner, py: &'a FakePython) -> Self {
        Routed {
            mock,
            py,
            order: Mutex::new(Vec::new()),
        }
    }

    fn is_python(program: &str) -> bool {
        program == "python3" || Path::new(program).ends_with("bin/pip")
    }

    pub fn order(&self) -> Vec<String> {
        self.order.lock().unwrap().clone()
    }

    /// Index of the first recorded call starting with `prefix`.
    pub fn position(&self, prefix: &str) -> Option<usize> {
        self.order().iter().position(|c| c.starts_with(prefix))
    }
}

impl Runner for Routed<'_> {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        self.order
            .lock()
            .unwrap()
            .push(format!("{program} {}", args.join(" ")));
        if Self::is_python(program) {
            self.py.run(program, args)
        } else {
            self.mock.run(program, args)
        }
    }

    fn run_as(&self, user: &str, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        self.mock.run_as(user, program, args)
    }
}
