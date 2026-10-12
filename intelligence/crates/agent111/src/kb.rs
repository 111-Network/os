//! Knowledge Layer access (Layer 0 connection). Read-only, absent-safe.
//! The agent grounds in the local KB snapshot via the AGENTS.md ritual:
//! index first, grep by name, one file at a time. grep/read_one land in T3.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Kb {
    dir: Option<PathBuf>,
    index: String,
    entries: usize,
}

impl Kb {
    /// Open a KB snapshot dir. A missing/unreadable dir or llms.txt yields an
    /// absent KB — never a panic; the agent must stay honest about it.
    pub fn open(dir: &Path) -> Self {
        let index_path = dir.join("llms.txt");
        match std::fs::read_to_string(&index_path) {
            Ok(index) => {
                let entries = index.lines().filter(|l| l.starts_with("- [")).count();
                Self {
                    dir: Some(dir.to_path_buf()),
                    index,
                    entries,
                }
            }
            Err(_) => Self::absent(),
        }
    }

    pub fn absent() -> Self {
        Self {
            dir: None,
            index: String::new(),
            entries: 0,
        }
    }

    pub fn is_present(&self) -> bool {
        self.dir.is_some()
    }

    pub fn entries(&self) -> Option<usize> {
        self.dir.as_ref().map(|_| self.entries)
    }

    /// Root index for the system prompt; an honesty line when absent.
    pub fn index_prompt(&self) -> String {
        if self.is_present() {
            self.index.clone()
        } else {
            "KNOWLEDGE BASE ABSENT: no local knowledge snapshot is installed on this node. \
             You must say your project knowledge is absent rather than inventing facts \
             about projects, devices, or protocols."
                .to_string()
        }
    }

    /// Filenames matching a name keyword (ritual step 2). T3 implements:
    /// filenames only, `_meta/` excluded, case-insensitive.
    pub fn grep(&self, _name: &str) -> Vec<String> {
        Vec::new()
    }

    /// Read one entry by unique basename (ritual step 3). T3 implements:
    /// basename resolution, no path escapes outside kb_dir.
    pub fn read_one(&self, _path: &str) -> Option<String> {
        None
    }
}
