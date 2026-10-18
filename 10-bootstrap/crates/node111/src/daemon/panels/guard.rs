//! Request guards on every panel request, every method (Layer 5 §13 "Guards"):
//!
//! - **Host:** exact match against the dashboard's accepted host list
//!   ([`routes::host_allowed`]) **minus `localhost`** (the main listener 308s
//!   `localhost` → `127.0.0.1`, so panels never see it), and the port must be
//!   the panel's own. `[auth] mode = "loopback-only"` panels accept loopback
//!   names only (`127.0.0.1`, `[::1]`, `111.localhost`). A missing or repeated
//!   Host is refused.
//! - **`Sec-Fetch-Site`:** `cross-site` is refused for any method and mode;
//!   values other than `same-origin` / `same-site` / `none` are refused too
//!   (fail closed). **On an API path** ([`static_files::is_api_request`], raw
//!   or decoded) only `same-origin` passes when the header is present: every
//!   other origin on the node's host (the dashboard, another panel, any other
//!   local service) is *same-site* with the panel and `SameSite=Strict` still
//!   attaches the cookie, so a same-site `GET` would otherwise drive the API
//!   (e.g. dequeue the shim's FromRadio frames) with the victim's cookie. A
//!   panel's own JS is same-origin with it; the frame / tab navigation that is
//!   legitimately same-site never targets `/api` (P1 review m1).
//! - **Origin authority:** for non-GET/HEAD requests, WebSocket upgrades and
//!   SSE opens, `Origin` (else `Referer`) must name exactly the request
//!   authority (host **and** port). §13 asks for this when `Sec-Fetch-*` is
//!   absent; it is enforced whenever the header is present as well (a panel's
//!   own JS is same-origin with it, so nothing legitimate is refused), and a
//!   guarded request carrying neither header is refused.
//! - **`OPTIONS`** (and `TRACE` / `CONNECT`) are refused: there is no
//!   legitimate preflight against a panel origin.

use super::static_files;
use crate::daemon::dashboard::api_v1::Problem;
use crate::daemon::dashboard::routes;
use axum::http::{header, HeaderMap, Method, StatusCode};
use std::net::IpAddr;

/// The request's accepted authority (`Host` after the guard).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedHost {
    /// Lowercased, no brackets, no trailing dot (e.g. `127.0.0.1`, `::1`).
    pub host: String,
    pub port: u16,
}

impl AcceptedHost {
    /// The host as it appears in a URL (IPv6 bracketed).
    pub fn url_host(&self) -> String {
        url_host(&self.host)
    }

    /// `host:port` (IPv6 bracketed).
    pub fn authority(&self) -> String {
        format!("{}:{}", self.url_host(), self.port)
    }

    /// `http://host:port`.
    pub fn origin(&self) -> String {
        format!("http://{}", self.authority())
    }
}

/// `host` as written in a URL authority (`::1` → `[::1]`).
pub fn url_host(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

/// What the Host check needs from the dashboard.
#[derive(Debug, Clone, Copy)]
pub struct HostPolicy<'a> {
    /// LAN sharing effective this daemon lifetime.
    pub lan: bool,
    /// The node's own addresses (accepted only when `lan`).
    pub own: &'a [IpAddr],
    /// `[auth] mode = "loopback-only"`.
    pub loopback_only: bool,
}

/// May a panel be addressed by `host` (parsed, lowercased, unbracketed)?
/// The dashboard's accepted list minus `localhost`; loopback names only for
/// loopback-only panels.
pub fn host_ok(host: &str, policy: &HostPolicy<'_>) -> bool {
    if host == "localhost" {
        return false;
    }
    if policy.loopback_only {
        return matches!(host, "127.0.0.1" | "::1" | "111.localhost");
    }
    routes::host_allowed(&url_host(host), policy.lan, policy.own)
}

/// Why a request was refused (each maps to a problem+json answer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Host missing, repeated, malformed, not accepted, or wrong port.
    Host,
    /// `Sec-Fetch-Site: cross-site` (or an unknown value).
    CrossSite,
    /// An API path fetched from anything but the panel's own origin
    /// (`Sec-Fetch-Site` present and not `same-origin`).
    NotSameOrigin,
    /// Origin/Referer authority missing or not the request authority.
    Origin,
    /// `OPTIONS` / `TRACE` / `CONNECT`.
    Method,
}

impl Refusal {
    pub fn problem(self) -> Problem {
        match self {
            Refusal::Host => Problem::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "refused Host header",
            ),
            Refusal::CrossSite => Problem::new(
                StatusCode::FORBIDDEN,
                "bad_request",
                "cross-site request refused",
            ),
            Refusal::NotSameOrigin => Problem::new(
                StatusCode::FORBIDDEN,
                "bad_request",
                "panel API refused to other origins",
            )
            .detail("Only the panel's own pages call its API."),
            Refusal::Origin => Problem::new(
                StatusCode::FORBIDDEN,
                "bad_request",
                "cross-origin request refused",
            ),
            Refusal::Method => Problem::new(
                StatusCode::FORBIDDEN,
                "bad_request",
                "method refused on a panel origin",
            )
            .detail("Panels answer no preflight; their own pages are same-origin."),
        }
    }
}

/// The exactly-one Host header, accepted for a panel listening on `port`.
pub fn accept_host(
    headers: &HeaderMap,
    port: u16,
    policy: &HostPolicy<'_>,
) -> Result<AcceptedHost, Refusal> {
    let mut all = headers.get_all(header::HOST).iter();
    let (Some(raw), None) = (all.next(), all.next()) else {
        return Err(Refusal::Host);
    };
    let raw = raw.to_str().map_err(|_| Refusal::Host)?;
    let (host, got_port) = routes::parse_host(raw).ok_or(Refusal::Host)?;
    if got_port != Some(port) || !host_ok(&host, policy) {
        return Err(Refusal::Host);
    }
    Ok(AcceptedHost { host, port })
}

/// `scheme://authority[/...]` → the normalized authority (`http` only;
/// lowercased host, explicit port, IPv6 bracketed). `None` for `null`, other
/// schemes, userinfo, or junk.
pub fn http_authority(url: &str) -> Option<String> {
    let rest = url.trim().strip_prefix("http://")?;
    let auth = rest.split(['/', '?', '#']).next()?;
    if auth.is_empty() || auth.contains('@') {
        return None;
    }
    let (host, port) = routes::parse_host(auth)?;
    Some(format!("{}:{}", url_host(&host), port.unwrap_or(80)))
}

/// Does this request need the Origin/Referer authority check? Non-GET/HEAD,
/// WebSocket upgrades, SSE opens.
fn needs_origin(method: &Method, headers: &HeaderMap) -> bool {
    if *method != Method::GET && *method != Method::HEAD {
        return true;
    }
    let upgrade = headers.contains_key(header::UPGRADE);
    let sse = headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.to_ascii_lowercase().contains("text/event-stream"));
    upgrade || sse
}

/// Run every guard on a request for `path` (the raw request path, still
/// percent-encoded). `Ok` carries the accepted authority (used for the
/// response's `frame-ancestors`).
pub fn check(
    method: &Method,
    path: &str,
    headers: &HeaderMap,
    port: u16,
    policy: &HostPolicy<'_>,
) -> Result<AcceptedHost, Refusal> {
    let accepted = accept_host(headers, port, policy)?;
    if matches!(*method, Method::OPTIONS | Method::TRACE | Method::CONNECT) {
        return Err(Refusal::Method);
    }
    let api = static_files::is_api_request(path);
    for v in headers.get_all("sec-fetch-site") {
        let v = v.to_str().map_err(|_| Refusal::CrossSite)?;
        match v.trim().to_ascii_lowercase().as_str() {
            "same-origin" => {}
            "same-site" | "none" if !api => {}
            "same-site" | "none" => return Err(Refusal::NotSameOrigin),
            _ => return Err(Refusal::CrossSite),
        }
    }
    if needs_origin(method, headers) {
        let want = accepted.authority();
        let mut origins = headers.get_all(header::ORIGIN).iter();
        let presented = match (origins.next(), origins.next()) {
            (Some(o), None) => o.to_str().ok().and_then(http_authority),
            (Some(_), Some(_)) => None,
            (None, _) => {
                let mut refs = headers.get_all(header::REFERER).iter();
                match (refs.next(), refs.next()) {
                    (Some(r), None) => r.to_str().ok().and_then(http_authority),
                    _ => None,
                }
            }
        };
        if presented.as_deref() != Some(want.as_str()) {
            return Err(Refusal::Origin);
        }
    }
    Ok(accepted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn hm(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    const P: u16 = 11190;

    fn pol(lan: bool, own: &[IpAddr], loopback_only: bool) -> HostPolicy<'_> {
        HostPolicy {
            lan,
            own,
            loopback_only,
        }
    }

    #[test]
    fn host_matrix() {
        let own: Vec<IpAddr> = vec!["192.168.1.5".parse().unwrap()];
        let off = pol(false, &own, false);
        let on = pol(true, &own, false);
        let lo = pol(true, &own, true);
        for h in [
            "127.0.0.1:11190",
            "[::1]:11190",
            "111.localhost:11190",
            "111.local:11190",
            "111.LOCAL.:11190",
            "111-2.local:11190",
        ] {
            assert!(accept_host(&hm(&[("host", h)]), P, &off).is_ok(), "{h}");
        }
        for h in [
            "localhost:11190",
            "LOCALHOST:11190",
            "localhost.:11190",
            "127.0.0.1",
            "127.0.0.1:11180",
            "127.0.0.1:80",
            "evil.com:11190",
            "192.168.1.5:11190",
            "::1:11190",
            "[::1]",
        ] {
            assert_eq!(
                accept_host(&hm(&[("host", h)]), P, &off),
                Err(Refusal::Host),
                "{h}"
            );
        }
        assert!(accept_host(&hm(&[("host", "192.168.1.5:11190")]), P, &on).is_ok());
        assert!(accept_host(&hm(&[("host", "192.168.1.9:11190")]), P, &on).is_err());
        // loopback-only: loopback names only, even with LAN sharing on
        for h in ["127.0.0.1:11190", "[::1]:11190", "111.localhost:11190"] {
            assert!(accept_host(&hm(&[("host", h)]), P, &lo).is_ok(), "{h}");
        }
        for h in ["192.168.1.5:11190", "111.local:11190", "localhost:11190"] {
            assert!(accept_host(&hm(&[("host", h)]), P, &lo).is_err(), "{h}");
        }
        // missing or repeated Host
        assert_eq!(accept_host(&hm(&[]), P, &off), Err(Refusal::Host));
        assert_eq!(
            accept_host(
                &hm(&[("host", "127.0.0.1:11190"), ("host", "127.0.0.1:11190")]),
                P,
                &off
            ),
            Err(Refusal::Host)
        );
        let a = accept_host(&hm(&[("host", "[::1]:11190")]), P, &off).unwrap();
        assert_eq!(a.origin(), "http://[::1]:11190");
    }

    #[test]
    fn fetch_metadata_and_origin() {
        let own = [];
        let p = pol(false, &own, false);
        let host = ("host", "127.0.0.1:11190");
        let ok = |m: Method, extra: &[(&str, &str)]| {
            let mut v = vec![host];
            v.extend_from_slice(extra);
            check(&m, "/", &hm(&v), P, &p)
        };
        // GET: fetch metadata only
        assert!(ok(Method::GET, &[]).is_ok());
        assert!(ok(Method::GET, &[("sec-fetch-site", "same-site")]).is_ok());
        assert!(ok(Method::GET, &[("sec-fetch-site", "none")]).is_ok());
        for v in ["cross-site", "Cross-Site", "weird"] {
            assert_eq!(
                ok(Method::GET, &[("sec-fetch-site", v)]),
                Err(Refusal::CrossSite),
                "{v}"
            );
        }
        assert_eq!(
            ok(
                Method::POST,
                &[
                    ("sec-fetch-site", "cross-site"),
                    ("origin", "http://127.0.0.1:11190")
                ]
            ),
            Err(Refusal::CrossSite)
        );
        // non-GET: Origin (else Referer) must be the request authority
        assert!(ok(Method::PUT, &[("origin", "http://127.0.0.1:11190")]).is_ok());
        assert!(ok(Method::PUT, &[("referer", "http://127.0.0.1:11190/x?y")]).is_ok());
        for bad in [
            "http://127.0.0.1:11180",
            "http://127.0.0.1",
            "null",
            "https://127.0.0.1:11190",
            "http://evil.com:11190",
            "http://u@127.0.0.1:11190",
        ] {
            assert_eq!(
                ok(Method::PUT, &[("origin", bad)]),
                Err(Refusal::Origin),
                "{bad}"
            );
        }
        assert_eq!(ok(Method::DELETE, &[]), Err(Refusal::Origin));
        // a matching Referer does not rescue a wrong Origin
        assert_eq!(
            ok(
                Method::POST,
                &[
                    ("origin", "http://127.0.0.1:11180"),
                    ("referer", "http://127.0.0.1:11190/")
                ]
            ),
            Err(Refusal::Origin)
        );
        // WebSocket upgrade and SSE opens are guarded like non-GET
        assert_eq!(
            ok(Method::GET, &[("upgrade", "websocket")]),
            Err(Refusal::Origin)
        );
        assert_eq!(
            ok(Method::GET, &[("accept", "text/event-stream")]),
            Err(Refusal::Origin)
        );
        assert!(ok(
            Method::GET,
            &[
                ("accept", "text/event-stream"),
                ("origin", "http://127.0.0.1:11190")
            ]
        )
        .is_ok());
        // preflight and friends
        for m in [Method::OPTIONS, Method::TRACE, Method::CONNECT] {
            assert_eq!(ok(m.clone(), &[]), Err(Refusal::Method), "{m}");
        }
        // the Host guard runs first
        assert_eq!(
            check(
                &Method::GET,
                "/",
                &hm(&[("host", "localhost:11190")]),
                P,
                &p
            ),
            Err(Refusal::Host)
        );
    }

    #[test]
    fn api_paths_are_same_origin_only() {
        let own = [];
        let p = pol(false, &own, false);
        let at = |path: &str, site: Option<&str>| {
            let mut v = vec![("host", "127.0.0.1:11190")];
            if let Some(s) = site {
                v.push(("sec-fetch-site", s));
            }
            check(&Method::GET, path, &hm(&v), P, &p)
        };
        for path in [
            "/api",
            "/api/",
            "/api/v1/fromradio",
            "/%61pi/v1/fromradio",
            "/api%2Fv1",
        ] {
            assert!(at(path, Some("same-origin")).is_ok(), "{path}");
            // no Fetch Metadata (older browsers, curl): SameSite=Strict and
            // the Origin rule for non-GET still apply
            assert!(at(path, None).is_ok(), "{path}");
            for site in ["same-site", "none", "Same-Site"] {
                assert_eq!(
                    at(path, Some(site)),
                    Err(Refusal::NotSameOrigin),
                    "{path} {site}"
                );
            }
            assert_eq!(at(path, Some("cross-site")), Err(Refusal::CrossSite));
        }
        // the frame / tab navigation stays same-site on non-API paths
        for path in ["/", "/messages/7", "/apix", "/x/api/v1", "/%2561pi/v1"] {
            assert!(at(path, Some("same-site")).is_ok(), "{path}");
            assert!(at(path, Some("none")).is_ok(), "{path}");
        }
        // a non-GET API call from the panel's own page passes
        let v = [
            ("host", "127.0.0.1:11190"),
            ("sec-fetch-site", "same-origin"),
            ("origin", "http://127.0.0.1:11190"),
        ];
        assert!(check(&Method::PUT, "/api/v1/toradio", &hm(&v), P, &p).is_ok());
    }

    #[test]
    fn authorities() {
        assert_eq!(
            http_authority("http://127.0.0.1:11190/a").as_deref(),
            Some("127.0.0.1:11190")
        );
        assert_eq!(
            http_authority("http://111.LOCAL").as_deref(),
            Some("111.local:80")
        );
        assert_eq!(http_authority("http://[::1]:5").as_deref(), Some("[::1]:5"));
        for bad in [
            "",
            "null",
            "file:///x",
            "http://",
            "http://a@b:1",
            "http://[::1",
        ] {
            assert_eq!(http_authority(bad), None, "{bad}");
        }
        for r in [
            Refusal::Host,
            Refusal::CrossSite,
            Refusal::NotSameOrigin,
            Refusal::Origin,
            Refusal::Method,
        ] {
            assert_eq!(r.problem().code, "bad_request");
        }
    }
}
