//! End-to-end tests for the KB retrieval ritual (os/0-knowledge/AGENTS.md):
//! open the snapshot, grep by keyword or filename, then read one entry at a
//! time — with `_meta/` invisible and no path ever escaping the KB dir.

use agent111::kb::Kb;
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kb")
}

fn open_fixture() -> Kb {
    Kb::open(&fixture_dir())
}

#[test]
fn open_fixture_present_with_three_index_entries() {
    let kb = open_fixture();
    assert!(kb.is_present());
    // Only the `- [` lines of the root llms.txt count; the fixture has 3.
    assert_eq!(kb.entries(), Some(3));
    assert!(kb.index_prompt().contains("Alpha Project"));
}

#[test]
fn grep_by_content_keyword_returns_relative_path() {
    let kb = open_fixture();
    let hits = kb.grep("ALPHA-MARKER");
    assert_eq!(hits, vec!["projects/alpha-project.md".to_string()]);
}

#[test]
fn grep_never_surfaces_meta_decoy() {
    let kb = open_fixture();
    // Neither by content nor by filename.
    assert!(kb.grep("HIDDEN-MARKER").is_empty());
    assert!(kb.grep("hidden").is_empty());
    // Even a match-everything query keeps `_meta/` out at any depth.
    let hits = kb.grep("marker");
    assert_eq!(hits.len(), 5); // alpha, beta x2, gamma, delta — not hidden
    for hit in &hits {
        assert!(!hit.starts_with("_meta/"), "decoy leaked: {hit}");
    }
}

#[test]
fn grep_by_filename() {
    let kb = open_fixture();
    assert_eq!(
        kb.grep("gamma-device"),
        vec!["devices/gamma-device.md".to_string()]
    );
}

#[test]
fn grep_is_case_insensitive_on_path_and_content() {
    let kb = open_fixture();
    let want = vec!["devices/gamma-device.md".to_string()];
    assert_eq!(kb.grep("GAMMA-DEVICE"), want); // path match, upper query
    assert_eq!(kb.grep("gamma-marker"), want); // content match, lower query
}

#[test]
fn grep_reaches_nested_entries() {
    let kb = open_fixture();
    assert_eq!(
        kb.grep("DELTA-MARKER"),
        vec!["projects/nested/delta-tool.md".to_string()]
    );
    assert_eq!(
        kb.read_one("projects/nested/delta-tool.md")
            .expect("nested relative path resolves"),
        kb.read_one("delta-tool").expect("nested basename resolves")
    );
}

#[test]
fn grep_results_are_sorted_relative_paths() {
    let kb = open_fixture();
    assert_eq!(
        kb.grep("beta-thing"),
        vec![
            "devices/beta-thing.md".to_string(),
            "projects/beta-thing.md".to_string()
        ]
    );
}

#[test]
fn grep_empty_query_is_empty() {
    let kb = open_fixture();
    assert!(kb.grep("").is_empty());
}

#[test]
fn read_one_by_relative_path() {
    let kb = open_fixture();
    let text = kb
        .read_one("projects/alpha-project.md")
        .expect("relative path resolves");
    assert!(text.contains("ALPHA-MARKER"));
}

#[test]
fn read_one_by_unique_basename() {
    let kb = open_fixture();
    for query in ["alpha-project.md", "alpha-project"] {
        let text = kb
            .read_one(query)
            .unwrap_or_else(|| panic!("{query} should resolve uniquely"));
        assert!(text.contains("ALPHA-MARKER"));
    }
    // Unknown basenames resolve to nothing, not to a guess.
    assert!(kb.read_one("no-such-entry").is_none());
}

#[test]
fn read_one_ambiguous_basename_is_none() {
    let kb = open_fixture();
    // beta-thing.md exists in both projects/ and devices/: refuse, never guess.
    assert!(kb.read_one("beta-thing.md").is_none());
    assert!(kb.read_one("beta-thing").is_none());
}

#[test]
fn read_one_refuses_traversal_and_absolute_paths() {
    let kb = open_fixture();
    assert!(kb.read_one("../../etc/passwd").is_none());
    assert!(kb.read_one("../kb/llms.txt").is_none());
    assert!(kb.read_one("/etc/passwd").is_none());
    // Absolute even when it points straight at a real entry inside the KB.
    let abs = fixture_dir().join("projects/alpha-project.md");
    assert!(kb.read_one(abs.to_str().unwrap()).is_none());
}

#[test]
fn read_one_refuses_non_md_and_meta_targets() {
    let kb = open_fixture();
    assert!(kb.read_one("llms.txt").is_none()); // exists, but not .md
    assert!(kb.read_one("notes.txt").is_none());
    // The decoy never surfaces, even named explicitly by path or basename.
    assert!(kb.read_one("_meta/hidden.md").is_none());
    assert!(kb.read_one("hidden.md").is_none());
}

#[cfg(unix)]
fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let target = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[cfg(unix)]
#[test]
fn symlink_escape_is_refused() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let kb_root = tmp.path().join("kb");
    copy_dir(&fixture_dir(), &kb_root);

    // A secret .md outside the KB, linked in as projects/leak.md.
    let secret = tmp.path().join("secret.md");
    std::fs::write(&secret, "TOP-SECRET-MARKER").unwrap();
    symlink(&secret, kb_root.join("projects").join("leak.md")).unwrap();

    // A whole directory outside the KB, linked in as projects/escape.
    let outside = tmp.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("deep.md"), "DEEP-SECRET-MARKER").unwrap();
    symlink(&outside, kb_root.join("projects").join("escape")).unwrap();

    let kb = Kb::open(&kb_root);
    assert!(kb.is_present());

    // read_one: canonicalize catches both escapes.
    assert!(kb.read_one("projects/leak.md").is_none());
    assert!(kb.read_one("projects/escape/deep.md").is_none());

    // The walk never follows symlinks: grep is blind, basename can't resolve.
    assert!(kb.grep("TOP-SECRET-MARKER").is_empty());
    assert!(kb.grep("DEEP-SECRET-MARKER").is_empty());
    assert!(kb.grep("leak").is_empty());
    assert!(kb.read_one("leak.md").is_none());
}

#[test]
fn read_one_truncates_over_64kib() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("llms.txt"), "- [Big](big.md): big entry\n").unwrap();
    std::fs::write(root.join("big.md"), "x".repeat(128 * 1024)).unwrap();

    let kb = Kb::open(root);
    assert_eq!(kb.entries(), Some(1));
    let text = kb.read_one("big.md").expect("big entry reads");
    const MARK: &str = "\n\n[... truncated ...]";
    assert!(text.ends_with(MARK));
    assert_eq!(text.len(), 64 * 1024 + MARK.len());
    assert!(text.starts_with('x'));
}

#[test]
fn absent_dir_is_honest_and_never_panics() {
    let kb = Kb::open(Path::new("/nonexistent/111/kb/definitely-not-here"));
    assert!(!kb.is_present());
    assert_eq!(kb.entries(), None);
    assert!(kb.grep("anything").is_empty());
    assert_eq!(kb.read_one("alpha-project.md"), None);
    assert!(kb.index_prompt().contains("KNOWLEDGE BASE ABSENT"));
}

#[test]
fn dir_without_index_is_absent() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("stray.md"), "STRAY-MARKER").unwrap();
    let kb = Kb::open(tmp.path());
    assert!(!kb.is_present());
    assert!(kb.grep("STRAY-MARKER").is_empty());
    assert_eq!(kb.read_one("stray.md"), None);
}
