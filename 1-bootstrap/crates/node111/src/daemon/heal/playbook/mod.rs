//! Layer 9 (§6.1 the entry and its filename; §6.3 the loader; §21 memory and start-up work):
//! filled by row D4 (the loader here and `grammar.rs`), with `eval.rs` (row D5).
//!
//! **The start scan** ([`Library::scan`]). `111d` reads `current/share/kb/playbooks/*.md` once at
//! start ([`dir`]; the KB rides the payload, `N/kb.rs`; `*.md` as a shell globs it, so a name
//! that starts with `.` is no match): at most [`MAX_FILES`] files, regular files of at most
//! [`MAX_FILE_BYTES`] each, sized before they are read. Each is parsed whole
//! ([`grammar::parse_document`]) and its name compared with the one its `[match]` renders (§6.1,
//! so one file serves one fingerprint); the parse is then dropped and only the file's
//! [`PlaybookEntry`] is kept: its slug, its `[match]` key (a pointer into the compiled vocabulary),
//! its declared `schema` and its load state, `loaded`, or `refused` with "unsupported grammar N,
//! needs release X" (the Repairs page's list, `GET /api/v1/repairs/playbooks`, row D24). A
//! refused playbook is refused whole and never runs in part (§19 hard rule 8).
//!
//! **One read per incident open** ([`Library::open`]). When an incident opens, its fingerprint
//! finds the loaded entry of its `[match]` (a component's fingerprint, whose subject is no subject
//! of the table, finds the `any` playbook) and that one file is read and parsed again. The caller
//! holds the [`Held`] playbook while the incident is open and drops it when the incident closes,
//! so an idle node holds no parsed playbook: the library never holds one, and [`Library::held`]
//! counts the live ones (G19 (5)'s unit half). No incident, no read.
//!
//! **Never adds:** a parsed playbook held while no incident is open, a dependency, a read beyond
//! the start scan and one read per incident open; no task, timer, watch or open file.

pub mod eval;
pub mod grammar;

use super::vocab::{self, VocabRow, ANY};
use crate::daemon::components::manifest::is_component_id;
use crate::daemon::DaemonLayout;
use grammar::{Playbook, Refusal, Rule};
use std::collections::BTreeSet;
use std::io::Read;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Where the playbooks ride in the release payload (§6.3), under `current/`.
pub const PLAYBOOKS: &str = "share/kb/playbooks";
/// The most `*.md` files the start scan reads, the first in byte order; the rest are counted, not
/// read. The vocabulary has one fingerprint per row, so a release ships far fewer.
pub const MAX_FILES: usize = 256;
/// The largest playbook file read (bytes): a Layer 0 entry's body is 2,000 tokens at most (B09).
pub const MAX_FILE_BYTES: u64 = 64 * 1024;

/// `current/share/kb/playbooks` of the running release.
pub fn dir(layout: &DaemonLayout) -> PathBuf {
    layout.current_release().join(PLAYBOOKS)
}

/// One playbook's load state (§6.3, §18.1's `state: loaded | refused`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    Loaded,
    /// Refused whole: its rule and "unsupported grammar N, needs release X" with the detail.
    Refused {
        rule: Rule,
        reason: String,
    },
}

/// One playbook's `[match]` key and load state, as the start scan keeps it (§6.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybookEntry {
    /// The file name without its one `.md` (`repair-model-child-exit`): distinct files have
    /// distinct slugs, and [`Library::open`] reads `<slug>.md`.
    pub slug: String,
    /// `[match]`, when the block parsed.
    pub key: Option<&'static VocabRow>,
    /// The block's `schema`, when it is an integer.
    pub schema: Option<i64>,
    pub state: LoadState,
}

impl PlaybookEntry {
    pub fn is_loaded(&self) -> bool {
        self.state == LoadState::Loaded
    }

    /// Does it serve the fingerprint row `(layer, subject, reason)`, as loaded?
    fn serves(&self, layer: u8, subject: &str, reason: &str) -> bool {
        self.is_loaded()
            && self
                .key
                .is_some_and(|k| k.layer == layer && k.subject == subject && k.reason == reason)
    }
}

/// The start scan's table: each playbook file's entry, and no parsed playbook.
#[derive(Debug)]
pub struct Library {
    dir: PathBuf,
    release: String,
    entries: Vec<PlaybookEntry>,
    skipped: usize,
    live: Arc<AtomicUsize>,
}

/// What [`Library::open`] finds for an incident.
#[derive(Debug)]
pub enum Opened {
    /// No loaded playbook serves the fingerprint (§5.1: `unable_to_diagnose`, or a delegation).
    NoPlaybook,
    /// The file no longer parses or no longer serves the fingerprint: refused whole.
    Refused(Refusal),
    /// The playbook, parsed again, held while the incident is open.
    Loaded(Held),
}

/// A playbook parsed for an open incident: dropped when the incident closes (§6.3).
#[derive(Debug)]
pub struct Held {
    playbook: Playbook,
    subject: String,
    slug: String,
    live: Arc<AtomicUsize>,
}

impl Held {
    /// The incident's subject, the one `@subject` names (§6.2).
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// The playbook's slug.
    pub fn slug(&self) -> &str {
        &self.slug
    }

    pub fn playbook(&self) -> &Playbook {
        &self.playbook
    }
}

impl Deref for Held {
    type Target = Playbook;
    fn deref(&self) -> &Playbook {
        &self.playbook
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Library {
    /// The start scan of `dir` (module doc). A missing or unreadable directory is an empty
    /// library: no playbook, every incident `unable_to_diagnose`. `release` is the running one,
    /// the `X` of "needs release X".
    pub fn scan(dir: &Path, release: &str) -> Library {
        // the first MAX_FILES names in byte order, whatever the listing's order, holding no
        // more than MAX_FILES + 1 names at once
        let mut names: BTreeSet<String> = BTreeSet::new();
        let mut skipped = 0usize;
        if let Ok(rd) = std::fs::read_dir(dir) {
            // `*.md` as a shell globs it: a name that starts with `.` is no match (`.md`, an
            // editor's `.#name.md` lock or `.name.md` copy)
            for name in rd
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .filter(|n| n.ends_with(".md") && !n.starts_with('.'))
            {
                names.insert(name);
                if names.len() > MAX_FILES {
                    names.pop_last();
                    skipped += 1;
                }
            }
        }
        let entries = names
            .into_iter()
            .map(|name| {
                // one `.md` off: two files never share a slug, and `open` reads `<slug>.md`
                let slug = name.strip_suffix(".md").unwrap_or(&name).to_string();
                match load(dir, &name) {
                    Ok(p) => PlaybookEntry {
                        slug,
                        key: Some(p.key),
                        schema: Some(p.schema),
                        state: LoadState::Loaded,
                    },
                    Err((key, r)) => PlaybookEntry {
                        slug,
                        key,
                        schema: r.schema,
                        state: LoadState::Refused {
                            rule: r.rule,
                            reason: r.reason(release),
                        },
                    },
                }
            })
            .collect();
        Library {
            dir: dir.to_path_buf(),
            release: release.to_string(),
            entries,
            skipped,
            live: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Each scanned file's entry, sorted by slug.
    pub fn entries(&self) -> &[PlaybookEntry] {
        &self.entries
    }

    /// `*.md` files past [`MAX_FILES`], not read.
    pub fn skipped(&self) -> usize {
        self.skipped
    }

    /// The release the reasons name.
    pub fn release(&self) -> &str {
        &self.release
    }

    /// The loaded entry serving an incident's fingerprint `(layer, subject, reason)`: its own
    /// `[match]`, else, for a component's fingerprint (a component id that is no subject of the
    /// table), the `any` playbook of that layer and reason.
    pub fn find(&self, layer: u8, subject: &str, reason: &str) -> Option<&PlaybookEntry> {
        if let Some(e) = self
            .entries
            .iter()
            .find(|e| e.serves(layer, subject, reason))
        {
            return Some(e);
        }
        let component =
            is_component_id(subject) && !vocab::ROWS.iter().any(|r| r.subject == subject);
        if !component {
            return None;
        }
        self.entries.iter().find(|e| e.serves(layer, ANY, reason))
    }

    /// The playbook for an incident that opens with the fingerprint `(layer, subject, reason)`:
    /// one file read and parsed again, held by the caller while the incident is open.
    pub fn open(&self, layer: u8, subject: &str, reason: &str) -> Opened {
        let Some(entry) = self.find(layer, subject, reason) else {
            return Opened::NoPlaybook;
        };
        match load(&self.dir, &format!("{}.md", entry.slug)) {
            Ok(playbook) if Some(playbook.key) == entry.key => {
                self.live.fetch_add(1, Ordering::Relaxed);
                Opened::Loaded(Held {
                    playbook,
                    subject: subject.to_string(),
                    slug: entry.slug.clone(),
                    live: Arc::clone(&self.live),
                })
            }
            Ok(playbook) => Opened::Refused(Refusal::new(
                Some(playbook.schema),
                Rule::Filename,
                "the file serves another [match] than at the start scan",
            )),
            Err((_, r)) => Opened::Refused(r),
        }
    }

    /// Parsed playbooks alive now: one per open incident with a playbook, 0 on an idle node.
    pub fn held(&self) -> usize {
        self.live.load(Ordering::Relaxed)
    }

    /// The table's memory (§21 memory: the held `[match]` table): the library and its entries,
    /// with their heap bytes.
    pub fn table_bytes(&self) -> usize {
        std::mem::size_of::<Library>()
            + self.dir.as_os_str().len()
            + self.release.capacity()
            + self.entries.capacity() * std::mem::size_of::<PlaybookEntry>()
            + self
                .entries
                .iter()
                .map(|e| {
                    e.slug.capacity()
                        + match &e.state {
                            LoadState::Loaded => 0,
                            LoadState::Refused { reason, .. } => reason.capacity(),
                        }
                })
                .sum::<usize>()
    }
}

/// Read `dir/name`, a regular file of at most [`MAX_FILE_BYTES`], parse it and hold it to the
/// filename its `[match]` renders. A refusal carries the key when the block parsed.
fn load(dir: &Path, name: &str) -> Result<Playbook, (Option<&'static VocabRow>, Refusal)> {
    let text = read_bounded(&dir.join(name)).map_err(|r| (None, r))?;
    let playbook = grammar::parse_document(&text).map_err(|r| (None, r))?;
    let want = playbook.filename();
    if want != name {
        return Err((
            Some(playbook.key),
            Refusal::new(
                Some(playbook.schema),
                Rule::Filename,
                format!("its [match] renders {want}"),
            ),
        ));
    }
    Ok(playbook)
}

/// The file's text: a regular file (no symlink) of at most [`MAX_FILE_BYTES`], sized before the
/// read and read no further than the bound, UTF-8.
fn read_bounded(path: &Path) -> Result<String, Refusal> {
    let refuse = |why: String| Refusal::new(None, Rule::File, why);
    let meta = std::fs::symlink_metadata(path).map_err(|e| refuse(format!("unreadable: {e}")))?;
    if !meta.file_type().is_file() {
        return Err(refuse("not a regular file".into()));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(refuse(format!("over {MAX_FILE_BYTES} B")));
    }
    let mut buf = Vec::with_capacity(meta.len() as usize);
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_FILE_BYTES + 1).read_to_end(&mut buf))
        .map_err(|e| refuse(format!("unreadable: {e}")))?;
    if buf.len() as u64 > MAX_FILE_BYTES {
        return Err(refuse(format!("over {MAX_FILE_BYTES} B")));
    }
    String::from_utf8(buf).map_err(|_| refuse("not UTF-8".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE: &str = "2026.11.16";

    /// A playbook for the row `(layer, subject, reason)` whose one cause advises.
    fn playbook(layer: u8, subject: &str, reason: &str) -> String {
        format!(
            "## Details\n\n### Checks\n\n```toml\nschema = 1\n[match]\nlayer = {layer}\n\
             subject = \"{subject}\"\nreason = \"{reason}\"\n[[check]]\nid = \"open\"\n\
             test = {{ p = \"incident_count_at_least\", n = 1 }}\n[[cause]]\nid = \"seen\"\n\
             when = [\"open\"]\naction = {{ action = \"advise\", text = \"Seen.\" }}\n```\n"
        )
    }

    fn write(dir: &Path, name: &str, text: &str) {
        std::fs::write(dir.join(name), text).unwrap();
    }

    #[test]
    fn the_scan_keeps_keys_and_states_and_no_parsed_playbook() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(
            d,
            "repair-model-child-exit.md",
            &playbook(3, "model", "child_exit"),
        );
        write(
            d,
            "repair-any-child-exit.md",
            &playbook(6, "any", "child_exit"),
        );
        // named for another [match]: refused, never a second playbook for one fingerprint
        write(
            d,
            "repair-agent-child-exit.md",
            &playbook(3, "model", "child_exit"),
        );
        write(d, "repair-kb-verify-failed.md", "no checks here\n");
        write(d, "llms.txt", "not a playbook\n");
        std::fs::create_dir(d.join("sub.md")).unwrap();
        let lib = Library::scan(d, RELEASE);
        let states: Vec<(&str, Option<Rule>)> = lib
            .entries()
            .iter()
            .map(|e| {
                (
                    e.slug.as_str(),
                    match &e.state {
                        LoadState::Loaded => None,
                        LoadState::Refused { rule, .. } => Some(*rule),
                    },
                )
            })
            .collect();
        assert_eq!(
            states,
            [
                ("repair-agent-child-exit", Some(Rule::Filename)),
                ("repair-any-child-exit", None),
                ("repair-kb-verify-failed", Some(Rule::ChecksBlock)),
                ("repair-model-child-exit", None),
                ("sub", Some(Rule::File)),
            ]
        );
        let refused = &lib.entries()[0];
        assert_eq!(refused.key.map(|k| k.subject), Some("model"));
        match &refused.state {
            LoadState::Refused { reason, .. } => assert!(
                reason.starts_with("unsupported grammar 1, needs release newer than 2026.11.16"),
                "{reason}"
            ),
            LoadState::Loaded => unreachable!(),
        }
        assert_eq!(lib.held(), 0);
        assert_eq!(lib.skipped(), 0);
    }

    #[test]
    fn open_reads_once_and_held_counts_until_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(
            d,
            "repair-model-child-exit.md",
            &playbook(3, "model", "child_exit"),
        );
        write(
            d,
            "repair-any-child-exit.md",
            &playbook(6, "any", "child_exit"),
        );
        let lib = Library::scan(d, RELEASE);
        let a = match lib.open(3, "model", "child_exit") {
            Opened::Loaded(h) => h,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            (a.subject(), a.slug()),
            ("model", "repair-model-child-exit")
        );
        assert_eq!(a.key.subject, "model");
        // a component's fingerprint finds the `any` playbook; a table subject never does
        let b = match lib.open(6, "deltachat", "child_exit") {
            Opened::Loaded(h) => h,
            other => panic!("{other:?}"),
        };
        assert_eq!((b.subject(), b.key.subject), ("deltachat", ANY));
        assert!(matches!(
            lib.open(6, "transport", "child_exit"),
            Opened::NoPlaybook
        ));
        assert!(matches!(
            lib.open(3, "agent", "child_exit"),
            Opened::NoPlaybook
        ));
        assert!(matches!(
            lib.open(6, "Bad Id", "child_exit"),
            Opened::NoPlaybook
        ));
        assert_eq!(lib.held(), 2);
        drop(a);
        assert_eq!(lib.held(), 1);
        drop(b);
        assert_eq!(lib.held(), 0);

        // the file changed after the scan: refused at open, nothing held
        write(d, "repair-model-child-exit.md", "### Checks\n");
        assert!(matches!(
            lib.open(3, "model", "child_exit"),
            Opened::Refused(Refusal {
                rule: Rule::ChecksBlock,
                ..
            })
        ));
        write(
            d,
            "repair-model-child-exit.md",
            &playbook(3, "agent", "child_exit"),
        );
        assert!(matches!(
            lib.open(3, "model", "child_exit"),
            Opened::Refused(Refusal {
                rule: Rule::Filename,
                ..
            })
        ));
        assert_eq!(lib.held(), 0);
    }

    #[test]
    fn files_are_sized_before_they_are_read() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let mut big = playbook(3, "model", "child_exit");
        big.push_str(&"x".repeat(MAX_FILE_BYTES as usize));
        write(d, "repair-model-child-exit.md", &big);
        std::fs::write(d.join("repair-agent-child-exit.md"), [0xff, 0xfe, b'\n']).unwrap();
        // a link to a playbook that would load, named for its own [match]: a regular file only
        write(d, "kb.txt", &playbook(0, "kb", "verify_failed"));
        std::os::unix::fs::symlink(d.join("kb.txt"), d.join("repair-kb-verify-failed.md")).unwrap();
        let lib = Library::scan(d, "dev");
        for e in lib.entries() {
            match &e.state {
                LoadState::Refused { rule, reason } => {
                    assert_eq!(*rule, Rule::File, "{}", e.slug);
                    assert!(
                        reason.contains("needs release newer than this one"),
                        "{reason}"
                    );
                }
                LoadState::Loaded => panic!("{} loaded", e.slug),
            }
        }
        assert_eq!(lib.entries().len(), 3);
    }

    #[test]
    fn a_slug_drops_one_md_and_a_dot_file_is_no_match() {
        // review finding 2: one `.md` comes off, so two files never share a slug; `*.md` reads
        // as a shell globs it, so `.md` and an editor's `.#name.md` lock are no playbook
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let text = playbook(3, "model", "child_exit");
        write(d, "repair-model-child-exit.md", &text);
        write(d, "repair-model-child-exit.md.md", &text);
        write(d, ".md", &text);
        write(d, ".repair-model-child-exit.md", &text);
        std::os::unix::fs::symlink(
            d.join("repair-model-child-exit.md"),
            d.join(".#repair-model-child-exit.md"),
        )
        .unwrap();
        let lib = Library::scan(d, RELEASE);
        let slugs: Vec<&str> = lib.entries().iter().map(|e| e.slug.as_str()).collect();
        assert_eq!(
            slugs,
            ["repair-model-child-exit", "repair-model-child-exit.md"]
        );
        assert!(lib.entries()[0].is_loaded());
        assert!(matches!(
            lib.entries()[1].state,
            LoadState::Refused {
                rule: Rule::Filename,
                ..
            }
        ));
        match lib.open(3, "model", "child_exit") {
            Opened::Loaded(h) => assert_eq!(h.slug(), "repair-model-child-exit"),
            other => panic!("{other:?}"),
        }
        assert_eq!(lib.skipped(), 0);
    }

    #[test]
    fn a_missing_directory_is_an_empty_library() {
        let lib = Library::scan(Path::new("/nonexistent/111/playbooks"), RELEASE);
        assert!(lib.entries().is_empty());
        assert!(matches!(
            lib.open(3, "model", "child_exit"),
            Opened::NoPlaybook
        ));
        assert_eq!(lib.release(), RELEASE);
    }

    #[test]
    fn the_scan_reads_at_most_max_files() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..MAX_FILES + 3 {
            write(tmp.path(), &format!("x{i:04}.md"), "");
        }
        let lib = Library::scan(tmp.path(), RELEASE);
        assert_eq!((lib.entries().len(), lib.skipped()), (MAX_FILES, 3));
    }

    #[test]
    fn the_playbook_directory_is_the_payload_s() {
        let layout = DaemonLayout {
            root: PathBuf::from("/var/lib/111"),
            etc: PathBuf::from("/etc/111"),
        };
        assert_eq!(
            dir(&layout),
            Path::new("/var/lib/111/current/share/kb/playbooks")
        );
    }
}
