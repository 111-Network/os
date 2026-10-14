//! Static panel bundles (Layer 5 §13 `[source] kind = "static"`): the
//! adopted project's own build, served **unmodified** from its directory
//! under the release payload (`<root>/current/<bundle>`, e.g.
//! `panels/meshtastic-web` — never embedded in our binary, GPL boundary §14).
//!
//! - Plain files by extension ([`routes::content_type_for`]); a directory path
//!   serves its `index.html`. `ETag` + `If-None-Match` → `304`,
//!   `Cache-Control: no-cache` (the bundle's own file names may or may not be
//!   hashed; we do not guess).
//! - **SPA fallback for non-API paths only**: a missing path whose last segment
//!   has no extension (a client route) or that a browser navigates to
//!   (`Accept: text/html`) serves the bundle's root `index.html`. Under an API
//!   path ([`is_api_path`]) it never does — a missing file there is a
//!   problem+json `404` (shim panels answer API paths through the shim).
//! - Paths are percent-decoded once; NUL, backslash, an encoded `/`, empty and
//!   dot segments are refused; the resolved file must stay inside the bundle
//!   directory after canonicalization (no symlink escapes).
//! - GET/HEAD only.

use crate::daemon::dashboard::api_v1::Problem;
use crate::daemon::dashboard::routes::content_type_for;
use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Largest bundle file served (a larger one is a packaging error).
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// `/api` and everything under `/api/` (never the SPA fallback).
pub fn is_api_path(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

/// Percent-decode a request path once and validate it as a relative bundle
/// path (`""` = the bundle root). `None` for anything unsafe.
pub fn decode_rel(path: &str) -> Option<String> {
    let raw = path.strip_prefix('/')?;
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let h = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
                let b = u8::from_str_radix(h, 16).ok()?;
                if b == b'/' || b == b'\\' || b == 0 {
                    return None;
                }
                out.push(b);
                i += 3;
            }
            b'\\' | 0 => return None,
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    let rel = String::from_utf8(out).ok()?;
    if rel.is_empty() {
        return Some(rel);
    }
    let trimmed = rel.strip_suffix('/').unwrap_or(&rel);
    let ok = trimmed
        .split('/')
        .all(|seg| !seg.is_empty() && !seg.starts_with('.') && !seg.chars().any(char::is_control));
    ok.then_some(rel)
}

/// Resolve `rel` inside `bundle` (canonical containment; directories map to
/// their `index.html`). `None` when absent or outside.
fn resolve(bundle: &Path, rel: &str) -> Option<(PathBuf, String)> {
    let root = bundle.canonicalize().ok()?;
    let mut rel = rel.to_string();
    let mut path = root.join(&rel);
    if rel.is_empty() || rel.ends_with('/') || path.is_dir() {
        if !rel.is_empty() && !rel.ends_with('/') {
            rel.push('/');
        }
        rel.push_str("index.html");
        path = root.join(&rel);
    }
    let canon = path.canonicalize().ok()?;
    (canon.starts_with(&root) && canon.is_file()).then_some((canon, rel))
}

fn wants_html(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.to_ascii_lowercase().contains("text/html"))
}

/// A client route (last segment without an extension)?
fn looks_like_route(rel: &str) -> bool {
    let last = rel.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    !last.contains('.')
}

fn not_found(api: bool) -> Response {
    if api {
        Problem::new(StatusCode::NOT_FOUND, "not_found", "no such panel API path").into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "not found",
        )
            .into_response()
    }
}

async fn file_response(path: &Path, rel: &str, req_headers: &HeaderMap) -> Response {
    let unavailable = || {
        Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "panel file unavailable",
        )
        .into_response()
    };
    if !tokio::fs::metadata(path)
        .await
        .is_ok_and(|m| m.len() <= MAX_FILE_BYTES)
    {
        return unavailable();
    }
    let Ok(bytes) = tokio::fs::read(path).await else {
        return unavailable();
    };
    let digest = Sha256::digest(&bytes);
    let etag = format!(
        "\"{}\"",
        digest[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let not_modified = req_headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"));
    let mut resp = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let mut r = Response::new(Body::from(bytes));
        r.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(content_type_for(rel)),
        );
        r
    };
    let h = resp.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, v);
    }
    resp
}

/// Serve `path` (the request path, still percent-encoded) from `bundle`.
pub async fn serve(bundle: &Path, method: &Method, path: &str, headers: &HeaderMap) -> Response {
    if *method != Method::GET && *method != Method::HEAD {
        let mut r = Problem::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "bad_request",
            "method not allowed",
        )
        .into_response();
        r.headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return r;
    }
    let api = is_api_path(path);
    let Some(rel) = decode_rel(path) else {
        return not_found(api);
    };
    let bundle = bundle.to_path_buf();
    let found = {
        let (b, r) = (bundle.clone(), rel.clone());
        tokio::task::spawn_blocking(move || resolve(&b, &r))
            .await
            .ok()
            .flatten()
    };
    if let Some((file, rel)) = found {
        return file_response(&file, &rel, headers).await;
    }
    if api || !(looks_like_route(&rel) || wants_html(headers)) {
        return not_found(api);
    }
    let index = tokio::task::spawn_blocking(move || resolve(&bundle, ""))
        .await
        .ok()
        .flatten();
    match index {
        Some((file, rel)) => file_response(&file, &rel, headers).await,
        None => not_found(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_decoded_and_confined() {
        assert_eq!(decode_rel("/").as_deref(), Some(""));
        assert_eq!(decode_rel("/a/b.js").as_deref(), Some("a/b.js"));
        assert_eq!(decode_rel("/devices/").as_deref(), Some("devices/"));
        assert_eq!(
            decode_rel("/i18n/en%20US.json").as_deref(),
            Some("i18n/en US.json")
        );
        for bad in [
            "",
            "a",
            "/../x",
            "/a/../x",
            "/.env",
            "/a//b",
            "/a%2Fb",
            "/a%2fb",
            "/a%5Cb",
            "/a%00b",
            "/a\\b",
            "/%2e%2e/x",
            "/%zz",
            "/%e2",
            "/%ff",
        ] {
            assert_eq!(decode_rel(bad), None, "{bad}");
        }
        assert!(is_api_path("/api") && is_api_path("/api/v1/x"));
        assert!(!is_api_path("/apix") && !is_api_path("/x/api/"));
        assert!(looks_like_route("messages/1") && !looks_like_route("a/b.js"));
    }

    #[tokio::test]
    async fn spa_fallback_never_under_api() {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path();
        std::fs::write(b.join("index.html"), "<html>root</html>").unwrap();
        std::fs::create_dir_all(b.join("devices")).unwrap();
        std::fs::write(b.join("devices/list.json"), "{}").unwrap();
        std::fs::create_dir_all(b.join("sub")).unwrap();
        std::fs::write(b.join("sub/index.html"), "<html>sub</html>").unwrap();
        std::os::unix::fs::symlink("/etc/hostname", b.join("escape")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "s").unwrap();
        std::os::unix::fs::symlink(outside.path(), b.join("linkdir")).unwrap();

        let h = HeaderMap::new();
        let body = |r: Response| async move {
            let st = r.status();
            let ct = r
                .headers()
                .get(header::CONTENT_TYPE)
                .map(|v| v.to_str().unwrap().to_string());
            let b = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
            (st, ct, String::from_utf8_lossy(&b).into_owned())
        };
        let (s, ct, t) = body(serve(b, &Method::GET, "/", &h).await).await;
        assert_eq!((s, t.as_str()), (StatusCode::OK, "<html>root</html>"));
        assert_eq!(ct.as_deref(), Some("text/html; charset=utf-8"));
        let (s, ct, _) = body(serve(b, &Method::GET, "/devices/list.json", &h).await).await;
        assert_eq!(
            (s, ct.as_deref()),
            (StatusCode::OK, Some("application/json"))
        );
        let (_, _, t) = body(serve(b, &Method::GET, "/sub", &h).await).await;
        assert_eq!(t, "<html>sub</html>");
        // client route → index.html
        let (s, _, t) = body(serve(b, &Method::GET, "/messages/7", &h).await).await;
        assert_eq!((s, t.as_str()), (StatusCode::OK, "<html>root</html>"));
        // a missing asset is a 404, not HTML
        let (s, _, _) = body(serve(b, &Method::GET, "/assets/missing.js", &h).await).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        // API paths never fall back
        let (s, ct, _) = body(serve(b, &Method::GET, "/api/v1/fromradio", &h).await).await;
        assert_eq!(
            (s, ct.as_deref()),
            (StatusCode::NOT_FOUND, Some("application/problem+json"))
        );
        // symlink escapes are not served (they fall back like any missing route)
        for p in ["/escape", "/linkdir/secret"] {
            let (_, _, t) = body(serve(b, &Method::GET, p, &h).await).await;
            assert_eq!(t, "<html>root</html>", "{p}");
        }
        // method
        let r = serve(b, &Method::POST, "/", &h).await;
        assert_eq!(r.status(), StatusCode::METHOD_NOT_ALLOWED);
        // ETag round trip
        let r = serve(b, &Method::GET, "/devices/list.json", &h).await;
        let etag = r.headers().get(header::ETAG).unwrap().clone();
        let mut h2 = HeaderMap::new();
        h2.insert(header::IF_NONE_MATCH, etag);
        let r = serve(b, &Method::GET, "/devices/list.json", &h2).await;
        assert_eq!(r.status(), StatusCode::NOT_MODIFIED);
    }
}
