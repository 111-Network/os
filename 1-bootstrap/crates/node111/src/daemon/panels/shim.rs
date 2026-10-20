//! Built-in panel shims (Layer 5 §13 `[source] shim`, §14).
//!
//! A shim answers a panel's API paths **in-process** instead of proxying to
//! an HTTP upstream: e.g. `meshtastic-http` serves `PUT /api/v1/toradio` and
//! `GET /api/v1/fromradio` on the Meshtastic panel origin by reframing bodies
//! onto a per-panel-session meshmud TCP downlink (task M1). This file is only
//! the seam — the [`Shim`] trait, its request/response/error types and the
//! [`ShimRegistry`] the panel listener (task P1) dispatches through by the
//! manifest's `shim` name. No shim is implemented here.
//!
//! Contract for implementations:
//! - Every request reaching a shim already passed the panel guards (§13: Host,
//!   `Sec-Fetch-Site`, Origin authority, ticket/cookie session) and the body
//!   cap; the shim never sees cookies or the ticket.
//! - Per-session upstream state is keyed by [`ShimSession::session`] and must
//!   be dropped when the panel listener reports the session revoked or idle.
//! - An unavailable upstream is [`ShimError::Upstream`] (`502 upstream_error`):
//!   the SPA fallback never answers an API path with `index.html`.
//! - No body rewriting beyond the shim's own reframing; no CORS headers
//!   (the listener strips every `Access-Control-*` anyway).

use crate::daemon::dashboard::api_v1::Problem;
use axum::body::Bytes;
use axum::http::{Method, StatusCode};
use futures_util::future::BoxFuture;
use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::sync::Arc;

/// The guarded panel session a request belongs to (§13 ticket → cookie).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShimSession {
    /// Panel id (= its `ServiceId`, e.g. `meshtastic`).
    pub panel: String,
    /// Opaque per-session key, stable for the session's lifetime (the panel
    /// listener derives it from the `p111_<id>` cookie; never the cookie).
    pub session: String,
    /// The browser's peer address (the session is bound to it, §13).
    pub peer: IpAddr,
}

/// One request on a shim path.
#[derive(Debug, Clone)]
pub struct ShimRequest {
    pub method: Method,
    /// Path on the panel origin, e.g. `/api/v1/fromradio`.
    pub path: String,
    /// Raw query string without the `?` (the ticket parameter is never here).
    pub query: Option<String>,
    pub content_type: Option<String>,
    /// Request body, already within the panel's size cap.
    pub body: Bytes,
}

/// A shim's answer (the listener applies the §13 response policy on top).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShimResponse {
    pub status: StatusCode,
    pub content_type: Option<String>,
    pub body: Bytes,
}

impl ShimResponse {
    /// `200` with a body of `content_type`.
    pub fn ok(content_type: &str, body: impl Into<Bytes>) -> Self {
        ShimResponse {
            status: StatusCode::OK,
            content_type: Some(content_type.to_string()),
            body: body.into(),
        }
    }

    /// An empty answer with `status` (e.g. `204` after a `PUT /toradio`).
    pub fn empty(status: StatusCode) -> Self {
        ShimResponse {
            status,
            content_type: None,
            body: Bytes::new(),
        }
    }
}

/// Typed shim failures, each with its contract status and `Problem.code`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShimError {
    /// The upstream is down, refused or broke mid-exchange → `502
    /// upstream_error` (§13).
    Upstream(String),
    /// The request is malformed for this shim → `400 bad_request`.
    BadRequest(String),
    /// No such path under this shim → `404 not_found`.
    NotFound,
    /// The path exists, the method does not → `405 bad_request`.
    MethodNotAllowed,
    /// The body exceeds what the shim accepts → `413 bad_request`.
    TooLarge { limit: usize },
}

impl ShimError {
    pub fn status(&self) -> StatusCode {
        match self {
            ShimError::Upstream(_) => StatusCode::BAD_GATEWAY,
            ShimError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ShimError::NotFound => StatusCode::NOT_FOUND,
            ShimError::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            ShimError::TooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
        }
    }

    /// `Problem.code` (contract enum; `upstream_error` per §15).
    pub fn code(&self) -> &'static str {
        match self {
            ShimError::Upstream(_) => "upstream_error",
            ShimError::NotFound => "not_found",
            ShimError::BadRequest(_) | ShimError::MethodNotAllowed | ShimError::TooLarge { .. } => {
                "bad_request"
            }
        }
    }

    /// The problem+json body the panel listener answers with.
    pub fn problem(&self) -> Problem {
        let title = match self {
            ShimError::Upstream(_) => "the panel's upstream is not available",
            ShimError::BadRequest(_) => "bad request",
            ShimError::NotFound => "no such panel API path",
            ShimError::MethodNotAllowed => "method not allowed",
            ShimError::TooLarge { .. } => "request body too large",
        };
        let p = Problem::new(self.status(), self.code(), title);
        match self {
            ShimError::Upstream(d) | ShimError::BadRequest(d) => p.detail(d.clone()),
            ShimError::TooLarge { limit } => p.detail(format!("at most {limit} bytes")),
            ShimError::NotFound | ShimError::MethodNotAllowed => p,
        }
    }
}

impl fmt::Display for ShimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShimError::Upstream(d) => write!(f, "upstream unavailable: {d}"),
            ShimError::BadRequest(d) => write!(f, "bad request: {d}"),
            ShimError::NotFound => f.write_str("no such shim path"),
            ShimError::MethodNotAllowed => f.write_str("method not allowed"),
            ShimError::TooLarge { limit } => write!(f, "body over {limit} bytes"),
        }
    }
}

impl std::error::Error for ShimError {}

/// A built-in panel shim, named by a manifest's `[source] shim` (§13).
pub trait Shim: Send + Sync {
    /// The manifest name (`meshtastic-http`).
    fn name(&self) -> &'static str;

    /// Answer one guarded request of `session`.
    fn handle<'a>(
        &'a self,
        session: &'a ShimSession,
        req: ShimRequest,
    ) -> BoxFuture<'a, Result<ShimResponse, ShimError>>;
}

/// A second shim registered under a name already taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateShim(pub &'static str);

impl fmt::Display for DuplicateShim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a shim named {:?} is already registered", self.0)
    }
}

impl std::error::Error for DuplicateShim {}

/// The built-in shims by name. Built once at daemon start; a manifest whose
/// `shim` names nothing here is not registered (§13: invalid ⇒ unregistered).
#[derive(Clone, Default)]
pub struct ShimRegistry {
    shims: HashMap<&'static str, Arc<dyn Shim>>,
}

impl ShimRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `shim`; names are unique.
    pub fn register(&mut self, shim: Arc<dyn Shim>) -> Result<(), DuplicateShim> {
        let name = shim.name();
        if self.shims.contains_key(name) {
            return Err(DuplicateShim(name));
        }
        self.shims.insert(name, shim);
        Ok(())
    }

    /// The shim a manifest names (exact match).
    pub fn get(&self, name: &str) -> Option<Arc<dyn Shim>> {
        self.shims.get(name).cloned()
    }

    /// Registered names, sorted.
    pub fn names(&self) -> Vec<&'static str> {
        let mut names: Vec<_> = self.shims.keys().copied().collect();
        names.sort_unstable();
        names
    }
}

impl fmt::Debug for ShimRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShimRegistry")
            .field("shims", &self.names())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Echo shim: `PUT /api/v1/echo` returns the body; anything else fails.
    struct Echo;

    impl Shim for Echo {
        fn name(&self) -> &'static str {
            "echo"
        }

        fn handle<'a>(
            &'a self,
            session: &'a ShimSession,
            req: ShimRequest,
        ) -> BoxFuture<'a, Result<ShimResponse, ShimError>> {
            Box::pin(async move {
                match (req.method, req.path.as_str()) {
                    (Method::PUT, "/api/v1/echo") => Ok(ShimResponse::ok(
                        "application/octet-stream",
                        [session.session.as_bytes(), b":", &req.body[..]].concat(),
                    )),
                    (_, "/api/v1/echo") => Err(ShimError::MethodNotAllowed),
                    (_, "/api/v1/down") => Err(ShimError::Upstream("refused".into())),
                    _ => Err(ShimError::NotFound),
                }
            })
        }
    }

    fn req(method: Method, path: &str, body: &'static [u8]) -> ShimRequest {
        ShimRequest {
            method,
            path: path.into(),
            query: None,
            content_type: None,
            body: Bytes::from_static(body),
        }
    }

    #[tokio::test]
    async fn registry_dispatches_by_name() {
        let mut reg = ShimRegistry::new();
        assert!(reg.get("echo").is_none());
        reg.register(Arc::new(Echo)).unwrap();
        assert_eq!(reg.register(Arc::new(Echo)), Err(DuplicateShim("echo")));
        assert_eq!(reg.names(), ["echo"]);
        assert!(reg.get("Echo").is_none(), "exact names only");
        assert!(format!("{reg:?}").contains("echo"));

        let session = ShimSession {
            panel: "meshtastic".into(),
            session: "s1".into(),
            peer: "127.0.0.1".parse().unwrap(),
        };
        let shim = reg.clone().get("echo").unwrap();
        let ok = shim
            .handle(
                &session,
                req(Method::PUT, "/api/v1/echo", b"\x94\xc3\x00\x01x"),
            )
            .await
            .unwrap();
        assert_eq!(ok.status, StatusCode::OK);
        assert_eq!(&ok.body[..], b"s1:\x94\xc3\x00\x01x");
        let e = shim
            .handle(&session, req(Method::GET, "/api/v1/echo", b""))
            .await
            .unwrap_err();
        assert_eq!(
            (e.status(), e.code()),
            (StatusCode::METHOD_NOT_ALLOWED, "bad_request")
        );
        let e = shim
            .handle(&session, req(Method::GET, "/api/v1/down", b""))
            .await
            .unwrap_err();
        let p = e.problem();
        assert_eq!((p.status, p.code), (502, "upstream_error"));
        assert_eq!(p.detail.as_deref(), Some("refused"));
    }

    #[test]
    fn error_statuses_and_codes() {
        for (e, status, code) in [
            (ShimError::Upstream("x".into()), 502, "upstream_error"),
            (ShimError::BadRequest("x".into()), 400, "bad_request"),
            (ShimError::NotFound, 404, "not_found"),
            (ShimError::MethodNotAllowed, 405, "bad_request"),
            (ShimError::TooLarge { limit: 8 }, 413, "bad_request"),
        ] {
            assert_eq!(e.status().as_u16(), status, "{e}");
            assert_eq!(e.code(), code, "{e}");
            let p = e.problem();
            assert_eq!((p.status, p.code), (status, code));
        }
        assert_eq!(
            ShimResponse::empty(StatusCode::NO_CONTENT).body,
            Bytes::new()
        );
    }
}
