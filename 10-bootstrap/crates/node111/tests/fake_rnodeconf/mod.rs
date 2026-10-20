//! A fake `rnodeconf` for the identify tests (Layer 6 §12 step 1, §16 G5):
//! a shell script at `<root>/venvs/rns/bin/rnodeconf` that logs its argv and
//! prints one golden from `tests/fixtures/rnodeconf/` (exit 0 always, like
//! the real tool's `graceful_exit()`), or sleeps to play a hung tool. It
//! knows nothing of RNodes: no byte of it is modelled on Reticulum source
//! (R06 G6); the goldens are the tool's *text*.
//!
//! - [`FakeRnodeconf::answer`] picks the golden (`info-provisioned.txt`, …).
//! - [`FakeRnodeconf::hang`] makes it hang that many seconds, answering nothing.
//! - [`FakeRnodeconf::calls`] — one line per run: the arguments.

#![allow(dead_code)] // each test crate uses a different subset

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub struct FakeRnodeconf {
    /// The script (`<root>/venvs/rns/bin/rnodeconf`).
    pub program: PathBuf,
    dir: PathBuf,
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rnodeconf")
}

impl FakeRnodeconf {
    /// Install the script under `root` (its state lives in `<root>/fake-rnodeconf/`).
    pub fn install(root: &Path) -> Self {
        let dir = root.join("fake-rnodeconf");
        std::fs::create_dir_all(&dir).unwrap();
        let program = root.join("venvs/rns/bin/rnodeconf");
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        let d = dir.to_string_lossy().into_owned();
        let f = fixtures().to_string_lossy().into_owned();
        assert!(
            !d.contains('\'') && !f.contains('\''),
            "paths must not hold a quote"
        );
        let script = format!(
            r#"#!/bin/sh
D='{d}'
printf '%s\n' "$*" >> "$D/calls.log"
# a hung tool never answers (exec: the killed pid is the sleeper)
if [ -f "$D/hang" ]; then exec sleep "$(cat "$D/hang")"; fi
cat '{f}'/"$(cat "$D/answer" 2>/dev/null || echo no-rnode.txt)"
exit 0
"#
        );
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        FakeRnodeconf { program, dir }
    }

    /// Print `fixture` (a file name in `tests/fixtures/rnodeconf/`).
    pub fn answer(&self, fixture: &str) {
        assert!(fixtures().join(fixture).is_file(), "no golden {fixture}");
        std::fs::write(self.dir.join("answer"), fixture).unwrap();
    }

    /// Hang `secs` and answer nothing (0 removes the hang).
    pub fn hang(&self, secs: u32) {
        let p = self.dir.join("hang");
        if secs == 0 {
            let _ = std::fs::remove_file(p);
        } else {
            std::fs::write(p, secs.to_string()).unwrap();
        }
    }

    /// The argument lines of every run so far.
    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}
