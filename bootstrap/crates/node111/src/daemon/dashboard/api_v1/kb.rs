//! `GET /api/v1/kb` · `GET /api/v1/kb/entry?path=` — the release's knowledge
//! base (`<root>/current/share/kb/`), read-only. Owned by Layer 4 task R2.
//!
//! - `/kb` → `llms.txt` as `text/plain` (404 problem when absent).
//! - `/kb/entry?path=projects/meshtastic.md` → that file as `text/markdown`.
//!   Refused with `400 bad_request`: a missing/empty path, absolute paths,
//!   backslashes or NULs, any `.`/`..`/empty/dot-file segment, non-`.md`
//!   files, and anything whose canonical path (symlinks resolved) leaves the
//!   canonical KB root. A clean path that does not exist is `404 not_found`.
//!   Entries above [`MAX_ENTRY`] bytes are refused (`400`).

use super::{ApiRouter, ApiState, Problem};
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Largest entry served (KB entries are a few KB).
pub const MAX_ENTRY: u64 = 1024 * 1024;

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/kb", get(index))
        .route("/api/v1/kb/entry", get(entry))
}

/// `<root>/current/share/kb`.
pub fn kb_root(api: &ApiState) -> PathBuf {
    api.layout.current_release().join("share/kb")
}

fn bad(detail: &str) -> Response {
    Problem::new(StatusCode::BAD_REQUEST, "bad_request", "invalid KB path")
        .detail(detail)
        .into_response()
}

fn not_found(detail: &str) -> Response {
    Problem::new(StatusCode::NOT_FOUND, "not_found", "no such KB entry")
        .detail(detail)
        .into_response()
}

async fn index(State(api): State<Arc<ApiState>>) -> Response {
    match std::fs::read(kb_root(&api).join("llms.txt")) {
        Ok(bytes) => ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], bytes).into_response(),
        Err(_) => not_found("this release has no knowledge base index (share/kb/llms.txt)"),
    }
}

/// Lexical checks on the requested relative path.
pub fn clean_entry_path(rel: &str) -> Result<&str, &'static str> {
    if rel.is_empty() {
        return Err("path is required");
    }
    if rel.starts_with('/') {
        return Err("absolute paths are refused");
    }
    if rel.contains('\\') || rel.contains('\0') {
        return Err("path contains forbidden characters");
    }
    if rel
        .split('/')
        .any(|seg| seg.is_empty() || seg.starts_with('.'))
    {
        return Err("path segments must be plain names (no '.', '..', or dot files)");
    }
    if !rel.ends_with(".md") {
        return Err("only .md entries are served");
    }
    Ok(rel)
}

/// Resolve `rel` under `root`, refusing symlink escapes. `Ok(None)` = missing.
pub fn resolve(root: &Path, rel: &str) -> Result<Option<PathBuf>, &'static str> {
    let rel = clean_entry_path(rel)?;
    let Ok(root) = root.canonicalize() else {
        return Ok(None);
    };
    let file = match root.join(rel).canonicalize() {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("path cannot be resolved"),
    };
    if !file.starts_with(&root) {
        return Err("path leaves the knowledge base");
    }
    if file.extension().is_none_or(|e| e != "md") {
        return Err("only .md entries are served");
    }
    Ok(file.is_file().then_some(file))
}

/// `?path=` (decoded by axum's `Query`; a malformed query string is a 400).
#[derive(Debug, serde::Deserialize)]
pub struct EntryQuery {
    pub path: Option<String>,
}

async fn entry(
    State(api): State<Arc<ApiState>>,
    query: Result<Query<EntryQuery>, QueryRejection>,
) -> Response {
    let Some(rel) = query.ok().and_then(|Query(q)| q.path) else {
        return bad("path is required, e.g. ?path=projects/meshtastic.md");
    };
    let file = match resolve(&kb_root(&api), &rel) {
        Ok(Some(f)) => f,
        Ok(None) => return not_found(&format!("no entry at {rel}")),
        Err(why) => return bad(why),
    };
    match std::fs::metadata(&file) {
        Ok(m) if m.len() > MAX_ENTRY => return bad("entry is too large"),
        Ok(_) => {}
        Err(_) => return not_found(&format!("no entry at {rel}")),
    }
    match std::fs::read(&file) {
        Ok(bytes) => (
            [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
            bytes,
        )
            .into_response(),
        Err(_) => not_found(&format!("no entry at {rel}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_refusals() {
        assert_eq!(
            clean_entry_path("projects/meshtastic.md"),
            Ok("projects/meshtastic.md")
        );
        for bad in [
            "",
            "/etc/passwd.md",
            "../secret.md",
            "projects/../../x.md",
            "./a.md",
            "projects//a.md",
            "projects/.hidden.md",
            "a\\b.md",
            "a\0.md",
            "projects/meshtastic.txt",
            "llms.txt",
            "projects/",
        ] {
            assert!(clean_entry_path(bad).is_err(), "{bad:?}");
        }
    }
}
