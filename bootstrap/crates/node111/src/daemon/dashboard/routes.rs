//! Routes + handlers.
//!
//! Layer 1 contracts (pinned in os/bootstrap/TASKS.md), unchanged:
//! - `GET /health`  → 200 iff all components green, always the component JSON
//! - `GET /api/status` → the Layer 4/9 seam schema (additive-only)
//! - both GET-only (405 otherwise); no identity material on any endpoint, ever
//!
//! Layer 4 (D15 amendments, `os/interface/README.md` §1/§11/§13):
//! - `/` serves the embedded UI (`interface/ui/dist`), any other dist path is
//!   served by extension with `cache-control` (hashed `/assets/*` immutable, the
//!   rest `no-cache` + ETag). An empty/missing dist (fresh checkout, UI not
//!   built) serves a short notice instead of `/`.
//! - The Layer 1 minimal page moved to `GET /v1`.
//! - `/api/v1/*` — see `api_v1/mod.rs` (auth, CSRF, events).
//! - Host guard (DNS-rebinding defense) on everything, widened per §11.

use super::api_v1::{self, ApiState, Problem};
use crate::daemon::health::{evaluate, HealthStatus, TransportHealth};
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::net::IpAddr;
use std::sync::Arc;

/// Layer 1 assets (legacy page + install tables). Only `dashboard.html` is
/// ever served (at `/v1`); `lora-table.toml` / `rns/` are never reachable.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets/"]
struct DashboardAssets;

/// The Layer 4 UI build (`pnpm build` in `os/interface/ui`). Path is relative to
/// this crate's manifest dir. `allow_missing`: a fresh checkout without a UI
/// build still compiles (and serves the notice). Release builds embed at compile
/// time; debug builds read the folder from disk at request time.
#[derive(rust_embed::RustEmbed)]
#[folder = "../../../interface/ui/dist"]
#[allow_missing = true]
struct UiDist;

const NOT_BUILT: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>111 node</title></head>
<body style="margin:0;background:#000;color:#fff;font:14px/1.6 ui-monospace,Menlo,Consolas,monospace">
<main style="max-width:40em;padding:24px 16px">
<h1 style="font-size:16px;margin:0 0 12px">111 node</h1>
<p>dashboard UI not built — run <code>pnpm build</code> in <code>interface/ui</code>, then rebuild <code>111</code>.</p>
<p>The minimal node page works: <a href="/v1" style="color:#fff">/v1</a></p>
</main></body></html>
"#;

// ---------------------------------------------------------------- host guard

/// Split a Host header into (lowercased host without brackets or trailing
/// dot, port). `None` for anything malformed (bare IPv6, junk port, empty).
pub fn parse_host(raw: &str) -> Option<(String, Option<u16>)> {
    let raw = raw.trim();
    let (host, port) = if let Some(rest) = raw.strip_prefix('[') {
        let (inner, after) = rest.split_once(']')?;
        let port = match after {
            "" => None,
            p => Some(p.strip_prefix(':')?),
        };
        inner.parse::<std::net::Ipv6Addr>().ok()?;
        (inner, port)
    } else {
        match raw.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (raw, None),
        }
    };
    let port = match port {
        None => None,
        Some(p) if !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()) => {
            Some(p.parse::<u16>().ok()?)
        }
        Some(_) => return None,
    };
    let host = host.to_ascii_lowercase();
    let host = host.strip_suffix('.').unwrap_or(&host).to_string();
    (!host.is_empty()).then_some((host, port))
}

/// `111-N.local` (mDNS conflict renames, N = digits).
fn is_renamed_mdns(host: &str) -> bool {
    host.strip_prefix("111-")
        .and_then(|r| r.strip_suffix(".local"))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The §11 allowlist. `own` = the node's addresses, consulted only when `lan`.
pub fn host_allowed(raw: &str, lan: bool, own: &[IpAddr]) -> bool {
    let Some((host, _port)) = parse_host(raw) else {
        return false;
    };
    if matches!(
        host.as_str(),
        "127.0.0.1" | "localhost" | "::1" | "111.localhost" | "111.local"
    ) || is_renamed_mdns(&host)
    {
        return true;
    }
    lan && host.parse::<IpAddr>().is_ok_and(|ip| own.contains(&ip))
}

/// Host allowlist middleware. A missing Host (HTTP/1.0 health probes) passes.
async fn host_guard(State(api): State<Arc<ApiState>>, req: Request, next: Next) -> Response {
    let ok = match req.headers().get(header::HOST) {
        None => true,
        Some(v) => v
            .to_str()
            .is_ok_and(|h| host_allowed(h, api.lan_enabled, &api.own_addrs())),
    };
    if ok {
        next.run(req).await
    } else {
        Problem::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "refused Host header",
        )
        .into_response()
    }
}

// ---------------------------------------------------------------- static UI

/// Content type by extension (the dist only holds these kinds).
pub fn content_type_for(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "txt" | "md" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// Reject traversal / dotfiles / odd separators; returns the clean relative path.
fn clean_rel(path: &str) -> Option<&str> {
    let rel = path.trim_start_matches('/');
    if rel.is_empty() || rel.contains('\\') || rel.contains('\0') {
        return None;
    }
    rel.split('/')
        .all(|seg| !seg.is_empty() && !seg.starts_with('.'))
        .then_some(rel)
}

struct UiFile {
    data: Cow<'static, [u8]>,
    sha256: [u8; 32],
}

fn ui_file(api: &ApiState, rel: &str) -> Option<UiFile> {
    let rel = clean_rel(rel)?;
    match &api.ui_dir {
        Some(dir) => {
            let path = dir.join(rel);
            // canonical containment (symlinks out of the fixture dir refused)
            let canon = path.canonicalize().ok()?;
            if !canon.starts_with(dir.canonicalize().ok()?) || !canon.is_file() {
                return None;
            }
            let bytes = std::fs::read(&canon).ok()?;
            let sha256 = Sha256::digest(&bytes).into();
            Some(UiFile {
                data: Cow::Owned(bytes),
                sha256,
            })
        }
        None => UiDist::get(rel).map(|f| UiFile {
            sha256: f.metadata.sha256_hash(),
            data: f.data,
        }),
    }
}

fn serve_ui_file(rel: &str, file: UiFile, req_headers: &HeaderMap) -> Response {
    let etag = format!(
        "\"{}\"",
        file.sha256[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let cache = if rel.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let ctype = content_type_for(rel);
    let not_modified = req_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"));
    let mut resp = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let body = match file.data {
            Cow::Borrowed(b) => Body::from(b),
            Cow::Owned(v) => Body::from(v),
        };
        let mut r = Response::new(body);
        r.headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static(ctype));
        r
    };
    let h = resp.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, v);
    }
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if ctype.starts_with("text/html") {
        h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    }
    resp
}

/// `GET /` — the UI's index.html, or the not-built notice.
async fn ui_index(State(api): State<Arc<ApiState>>, headers: HeaderMap) -> Response {
    match ui_file(&api, "index.html") {
        Some(f) => serve_ui_file("index.html", f, &headers),
        None => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            NOT_BUILT,
        )
            .into_response(),
    }
}

/// `GET /{*path}` — any other dist file; unknown `/api/*` → problem+json 404.
async fn ui_path(
    State(api): State<Arc<ApiState>>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> Response {
    if path == "api" || path.starts_with("api/") {
        return Problem::new(StatusCode::NOT_FOUND, "not_found", "no such API route")
            .into_response();
    }
    match ui_file(&api, &path) {
        Some(f) => serve_ui_file(&path, f, &headers),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// Did this binary embed a UI build? (used by the daemon start log)
pub fn ui_embedded() -> bool {
    UiDist::get("index.html").is_some()
}

// ---------------------------------------------------------------- app

/// The whole dashboard app (state applied). Serve with
/// `into_make_service_with_connect_info::<SocketAddr>()` — auth needs the peer.
pub fn app(api: Arc<ApiState>) -> axum::Router {
    let legacy = axum::Router::new()
        .route("/", get(ui_index))
        .route("/v1", get(legacy_index))
        .route("/health", get(health))
        .route("/api/status", get(api_status))
        .route("/{*path}", get(ui_path))
        .with_state(Arc::clone(&api));
    legacy
        .merge(api_v1::router(Arc::clone(&api)))
        .layer(middleware::from_fn_with_state(api, host_guard))
}

/// `GET /v1` — the Layer 1 minimal page (kept for old browsers, spec §2).
async fn legacy_index() -> impl IntoResponse {
    match DashboardAssets::get("dashboard.html") {
        Some(page) => {
            Html(String::from_utf8_lossy(page.data.as_ref()).into_owned()).into_response()
        }
        None => (StatusCode::INTERNAL_SERVER_ERROR, "dashboard asset missing").into_response(),
    }
}

#[derive(Serialize)]
struct HealthBody {
    schema: u32,
    status: &'static str,
    components: HealthComponents,
}

#[derive(Serialize)]
struct HealthComponents {
    identity: bool,
    kb: bool,
    transport: &'static str,
    updater_idle: bool,
}

fn transport_label(t: TransportHealth) -> &'static str {
    match t {
        TransportHealth::Alive => "alive",
        TransportHealth::DeliberatelyAbsent => "absent",
        TransportHealth::Degraded => "degraded",
        TransportHealth::Down => "down",
    }
}

async fn health(State(api): State<Arc<ApiState>>) -> impl IntoResponse {
    let s = api.dash.read().unwrap();
    let status = evaluate(&s.components);
    let body = HealthBody {
        schema: 1,
        status: match status {
            HealthStatus::Ok => "ok",
            HealthStatus::Error => "error",
        },
        components: HealthComponents {
            identity: s.components.identity,
            kb: s.components.kb,
            transport: transport_label(s.components.transport),
            updater_idle: s.components.updater_idle,
        },
    };
    let code = match status {
        HealthStatus::Ok => StatusCode::OK,
        HealthStatus::Error => StatusCode::SERVICE_UNAVAILABLE,
    };
    (code, Json(body))
}

#[derive(Serialize)]
struct StatusBody {
    schema: u32,
    node: NodeBody,
    kb: Option<KbBody>,
    transport_state: super::TransportState,
    health: HealthBody,
    updates: UpdatesBody,
    clock_fault: bool,
    offer: Option<super::OfferState>,
    error_state: Option<super::ErrorState>,
    /// Additive (Layer 3): agent + model server states, agent API URL.
    intelligence: super::IntelligenceState,
}

#[derive(Serialize)]
struct NodeBody {
    /// §7 machine string — lowercase hex (L2 §13). Replaced the provisional
    /// identity handle key at L1 v2.2 (Layer 4 codes against this name).
    node_id: Option<String>,
    category: Option<String>,
    version: String,
    uptime_sec: u64,
}

#[derive(Serialize)]
struct KbBody {
    sha256: String,
    entries: u32,
}

#[derive(Serialize)]
struct UpdatesBody {
    last_successful_check: Option<String>,
    pending: bool,
    floor: Option<String>,
}

async fn api_status(State(api): State<Arc<ApiState>>) -> impl IntoResponse {
    let s = api.dash.read().unwrap();
    let status = evaluate(&s.components);
    let body = StatusBody {
        schema: 1,
        node: NodeBody {
            node_id: s.node_id.clone(),
            category: s.category.clone(),
            version: s.version.clone(),
            uptime_sec: s.uptime_sec(),
        },
        kb: s.kb.as_ref().map(|k| KbBody {
            sha256: k.sha256.clone(),
            entries: k.entries,
        }),
        transport_state: s.transport_state,
        health: HealthBody {
            schema: 1,
            status: match status {
                HealthStatus::Ok => "ok",
                HealthStatus::Error => "error",
            },
            components: HealthComponents {
                identity: s.components.identity,
                kb: s.components.kb,
                transport: transport_label(s.components.transport),
                updater_idle: s.components.updater_idle,
            },
        },
        updates: UpdatesBody {
            last_successful_check: s.last_successful_update_check.clone(),
            pending: s.update_pending,
            floor: s.floor_release.clone(),
        },
        clock_fault: s.clock_fault,
        offer: s.offer.clone(),
        error_state: s.error_state.clone(),
        intelligence: s.intelligence.clone(),
    };
    Json(body)
}
