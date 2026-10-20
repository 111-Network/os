//! Knowledge Layer access (Layer 0 connection). Read-only, absent-safe.
//! The agent grounds in the local KB snapshot via the AGENTS.md ritual:
//! index first, grep by name, one file at a time. `_meta/` never surfaces.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

/// Directory-nesting cap for the recursive walk: a pathological or circular
/// tree cannot make grep/read_one unbounded.
const MAX_DEPTH: usize = 8;
/// grep returns at most this many relative paths (ritual is filenames-first).
const MAX_HITS: usize = 50;
/// read_one returns at most this much entry content before truncating.
const MAX_BYTES: usize = 64 * 1024;
/// Appended to read_one output when the entry exceeded `MAX_BYTES`.
const TRUNC_MARK: &str = "\n\n[... truncated ...]";

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

    /// Ritual step 2: entries whose relative path or content contains `name`
    /// (case-insensitive). Returns sorted relative paths — filenames only,
    /// never content dumps. `_meta/` is excluded at any depth, symlinks are
    /// never followed, and the result is capped at `MAX_HITS`. An absent KB
    /// or an empty query yields an empty vec, never a panic.
    pub fn grep(&self, name: &str) -> Vec<String> {
        let Some(dir) = &self.dir else {
            return Vec::new();
        };
        let needle = name.to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for rel in walk_md(dir) {
            if rel.to_lowercase().contains(&needle) {
                hits.push(rel);
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(dir.join(&rel)) {
                if content.to_lowercase().contains(&needle) {
                    hits.push(rel);
                }
            }
        }
        hits.truncate(MAX_HITS);
        hits
    }

    /// Ritual step 3: read one entry, by exact relative path under the KB dir
    /// or by unique basename (`alpha-project.md` or `alpha-project`). Returns
    /// None — never guesses — when the basename matches more than one entry.
    /// Refusals also return None: absolute paths, `..` components, non-`.md`
    /// targets, anything under `_meta/`, and (via canonicalize) any path or
    /// symlink escaping the KB dir. Content is capped at 64 KiB; truncation
    /// appends `[... truncated ...]`. Absent KB ⇒ None.
    pub fn read_one(&self, path: &str) -> Option<String> {
        let dir = self.dir.as_ref()?;
        let rel = Path::new(path);
        if rel.is_absolute() {
            return None;
        }
        for c in rel.components() {
            match c {
                Component::Normal(_) | Component::CurDir => {}
                _ => return None, // ParentDir (`..`), RootDir, Prefix
            }
        }
        // `_meta/` is tooling, not knowledge: it never surfaces, even when
        // named explicitly.
        if rel
            .components()
            .any(|c| matches!(c, Component::Normal(s) if s == OsStr::new("_meta")))
        {
            return None;
        }

        let candidate = if rel.components().count() > 1 {
            // Exact relative path under the KB dir.
            let p = dir.join(rel);
            if p.extension() != Some(OsStr::new("md")) {
                return None;
            }
            p
        } else {
            // Unique-basename resolution against the walked entry set: zero
            // hits or an ambiguous basename (more than one) both refuse.
            let want = rel.file_name()?.to_str()?.to_owned();
            let stem = match want.rsplit_once('.') {
                Some((s, "md")) if !s.is_empty() => None, // match full name
                Some((_, _)) => return None,              // non-.md target refused
                None => Some(want.as_str()),              // extensionless: match stem
            };
            let mut found: Option<PathBuf> = None;
            for rel_entry in walk_md(dir) {
                let p = Path::new(&rel_entry);
                let hit = match stem {
                    None => p.file_name().and_then(|f| f.to_str()) == Some(want.as_str()),
                    Some(stem) => p.file_stem().and_then(|s| s.to_str()) == Some(stem),
                };
                if hit {
                    if found.is_some() {
                        return None; // ambiguous basename
                    }
                    found = Some(dir.join(&rel_entry));
                }
            }
            found?
        };

        // Canonicalize both sides: anything escaping the KB dir (a symlink, a
        // bind mount, …) is refused.
        let dir_canon = dir.canonicalize().ok()?;
        let canon = candidate.canonicalize().ok()?;
        if !canon.starts_with(&dir_canon) {
            return None;
        }
        let bytes = std::fs::read(&canon).ok()?;
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        if text.len() > MAX_BYTES {
            let mut end = MAX_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push_str(TRUNC_MARK);
        }
        Some(text)
    }
}

/// Sorted relative `.md` paths (forward slashes) under `root`: `_meta/`
/// excluded at any depth, symlinks never followed, depth-capped at
/// `MAX_DEPTH`. Hand-rolled on std::fs::read_dir; unreadable dirs are skipped.
fn walk_md(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_name() == OsStr::new("_meta") {
                continue;
            }
            let Ok(ftype) = entry.file_type() else {
                continue;
            };
            if ftype.is_symlink() {
                continue; // never follow: escapes are impossible by construction
            }
            let path = entry.path();
            if ftype.is_dir() {
                if depth + 1 < MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if ftype.is_file() && path.extension() == Some(OsStr::new("md")) {
                if let Ok(rel) = path.strip_prefix(root) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    out.sort();
    out
}
