//! Static panel bundles (Layer 5 §13 `[source] kind = "static"`): the
//! adopted project's own build, served **unmodified** from its directory
//! under the release payload (`<root>/current/<bundle>`, e.g.
//! `panels/meshtastic-web` — never embedded in our binary, GPL boundary §14).
//!
//! - Plain files by extension ([`routes::content_type_for`]); a directory path
//!   serves its `index.html`. `ETag` + `If-None-Match` → `304`,
//!   `Cache-Control: no-cache` (the bundle's own file names may or may not be
//!   hashed; we do not guess). The `ETag` is derived from the opened file's
//!   metadata (inode, mtime, length — [`etag_of`]), never from its bytes, and
//!   the body is streamed in [`CHUNK`]-sized reads: a request costs one
//!   `fstat` and a bounded buffer, not the whole file in memory plus a hash
//!   (P1 review m4).
//! - **SPA fallback for non-API paths only**: a missing path whose last segment
//!   has no extension (a client route) or that a browser navigates to
//!   (`Accept: text/html`) serves the bundle's root `index.html`. Under an API
//!   path it never does — a missing file there is a problem+json `404` (shim
//!   panels answer API paths through the shim). "API path" is judged on the
//!   raw **and** the percent-decoded path ([`is_api_request`]), so
//!   `/%61pi/v1/x` is an API path too.
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
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::Poll;
use tokio::io::{AsyncRead, ReadBuf};

/// Largest bundle file served (a larger one is a packaging error).
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Bytes read per body chunk when streaming a bundle file.
pub const CHUNK: usize = 64 * 1024;

/// `/api` and everything under `/api/` (never the SPA fallback).
pub fn is_api_path(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

/// Every `%XX` decoded once (invalid escapes kept as written, invalid UTF-8
/// replaced). Only for *classifying* a path — never for resolving a file
/// ([`decode_rel`] does that, strictly).
fn decode_lossy(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Is the request path (raw, still percent-encoded) an API path as sent
/// **or** once decoded? Used by the guards (API paths are same-origin only)
/// and by [`serve`] (never the SPA fallback there).
pub fn is_api_request(raw: &str) -> bool {
    is_api_path(raw) || is_api_path(&decode_lossy(raw))
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

/// A strong `ETag` for one version of a bundle file: a digest of (inode,
/// mtime in ns, length) — a release swap or an in-place rewrite changes at
/// least one of them. Hashed so the tag does not publish inode numbers.
pub fn etag_of(meta: &std::fs::Metadata) -> String {
    let mut h = Sha256::new();
    h.update(b"111 panel etag v1\0");
    h.update(meta.ino().to_le_bytes());
    h.update(meta.mtime().to_le_bytes());
    h.update(meta.mtime_nsec().to_le_bytes());
    h.update(meta.len().to_le_bytes());
    let d = h.finalize();
    let hex: String = d[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("\"{hex}\"")
}

/// The file's bytes as a body stream: at most `len` bytes in [`CHUNK`]-sized
/// reads (a file that shrinks mid-stream ends early — the declared length
/// then makes hyper close the connection; one that grows is cut at `len`).
fn file_body(file: tokio::fs::File, len: u64) -> Body {
    let stream = futures_util::stream::unfold((file, len), |(mut file, left)| async move {
        if left == 0 {
            return None;
        }
        let want = usize::try_from(left.min(CHUNK as u64)).unwrap_or(CHUNK);
        let mut buf = vec![0u8; want];
        let read = std::future::poll_fn(|cx| {
            let mut rb = ReadBuf::new(&mut buf);
            match Pin::new(&mut file).poll_read(cx, &mut rb) {
                Poll::Ready(Ok(())) => Poll::Ready(Ok(rb.filled().len())),
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            }
        })
        .await;
        match read {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                let left = left.saturating_sub(n as u64);
                Some((Ok(axum::body::Bytes::from(buf)), (file, left)))
            }
            Err(e) => Some((Err(e), (file, 0))),
        }
    });
    Body::from_stream(stream)
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
    // metadata from the opened handle: the tag and the length describe
    // exactly the bytes that are streamed
    let Ok(file) = tokio::fs::File::open(path).await else {
        return unavailable();
    };
    let Ok(meta) = file.metadata().await else {
        return unavailable();
    };
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return unavailable();
    }
    let etag = etag_of(&meta);
    let not_modified = req_headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"));
    let mut resp = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let mut r = Response::new(file_body(file, meta.len()));
        let h = r.headers_mut();
        h.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(content_type_for(rel)),
        );
        h.insert(header::CONTENT_LENGTH, HeaderValue::from(meta.len()));
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
    let api = is_api_request(path);
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
        // classified on the raw and the decoded path
        for p in [
            "/api",
            "/api/v1/x",
            "/%61pi/v1/x",
            "/%61%70%69",
            "/api%2Fv1",
            "/%61pi/",
        ] {
            assert!(is_api_request(p), "{p}");
        }
        for p in [
            "/",
            "/apix",
            "/%2561pi/v1",
            "/x/%61pi/",
            "/%zz/api/",
            "/%",
            "/%+61pi",
        ] {
            assert!(!is_api_request(p), "{p}");
        }
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
        h2.insert(header::IF_NONE_MATCH, etag.clone());
        let r = serve(b, &Method::GET, "/devices/list.json", &h2).await;
        assert_eq!(r.status(), StatusCode::NOT_MODIFIED);
        // a rewrite of the file is a new version
        std::fs::write(b.join("devices/list.json"), "{\"devices\":[1]}").unwrap();
        let r = serve(b, &Method::GET, "/devices/list.json", &h2).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_ne!(r.headers().get(header::ETAG), Some(&etag));
        // encoded API paths are API paths: never the SPA, problem+json 404
        for p in ["/%61pi/v1/fromradio", "/%61pi"] {
            let (s, ct, t) = body(serve(b, &Method::GET, p, &h).await).await;
            assert_eq!(
                (s, ct.as_deref()),
                (StatusCode::NOT_FOUND, Some("application/problem+json")),
                "{p}"
            );
            assert_ne!(t, "<html>root</html>", "{p}");
        }
    }

    #[tokio::test]
    async fn etag_is_metadata_and_bodies_stream() {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path();
        // a file larger than one chunk, not a multiple of it
        let big: Vec<u8> = (0..(CHUNK * 3 + 17)).map(|i| (i % 251) as u8).collect();
        std::fs::write(b.join("big.bin"), &big).unwrap();
        std::fs::write(b.join("index.html"), "<html>root</html>").unwrap();
        let meta = std::fs::metadata(b.join("big.bin")).unwrap();
        let h = HeaderMap::new();
        let r = serve(b, &Method::GET, "/big.bin", &h).await;
        assert_eq!(r.status(), StatusCode::OK);
        // the tag comes from (inode, mtime, length), never from the bytes
        assert_eq!(
            r.headers().get(header::ETAG).unwrap().to_str().unwrap(),
            etag_of(&meta)
        );
        assert_eq!(
            r.headers().get(header::CONTENT_LENGTH).unwrap(),
            &HeaderValue::from(big.len() as u64)
        );
        // streamed: no exact in-memory body, and every byte arrives
        assert_ne!(
            axum::body::HttpBody::size_hint(r.body()).exact(),
            Some(big.len() as u64),
            "not buffered"
        );
        let got = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
        assert_eq!(got.as_ref(), big.as_slice());
        // a 304 never opens a body
        let mut h2 = HeaderMap::new();
        h2.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&etag_of(&meta)).unwrap(),
        );
        let r = serve(b, &Method::GET, "/big.bin", &h2).await;
        assert_eq!(r.status(), StatusCode::NOT_MODIFIED);
        let got = axum::body::to_bytes(r.into_body(), 16).await.unwrap();
        assert!(got.is_empty());
        // same metadata ⇒ same tag; another file ⇒ another tag
        let other = std::fs::metadata(b.join("index.html")).unwrap();
        assert_eq!(
            etag_of(&meta),
            etag_of(&std::fs::metadata(b.join("big.bin")).unwrap())
        );
        assert_ne!(etag_of(&meta), etag_of(&other));
    }
}
