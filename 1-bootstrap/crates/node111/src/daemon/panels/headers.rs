//! Panel response policy (Layer 5 §13 "Response policy"), applied to every
//! answer a panel listener sends — static files, shim answers, redirects and
//! refusals alike:
//!
//! - drop `X-Frame-Options`;
//! - parse every `Content-Security-Policy[-Report-Only]`, remove its
//!   `frame-ancestors`, then append `Content-Security-Policy: frame-ancestors
//!   <dashboard origins on the request's accepted host: :11180 and :80>`
//!   (`'none'` when the request had no accepted host);
//! - `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`;
//! - strip hop-by-hop headers and **every `Access-Control-*`** header (no other
//!   origin can read a panel response);
//! - never rewrite bodies; never apply the dashboard's own CSP.
//!
//! The generic reverse proxy is reserved (P7); its two cookie rules live here
//! so the seam is complete: [`filter_upstream_set_cookie`] (drop `s111` /
//! `p111_*`, strip `Domain=`) and [`strip_request_cookies`].
//!
//! [`redact_ticket`] keeps the ticket query out of every log line.

use super::guard::AcceptedHost;
use super::ticket::TICKET_PARAM;
use crate::daemon::dashboard::DEFAULT_PORT;
use axum::http::{header, HeaderMap, HeaderName, HeaderValue};

/// Hop-by-hop headers (RFC 9110 §7.6.1 plus the legacy `Proxy-Connection`).
pub const HOP_BY_HOP: [&str; 9] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "proxy-authenticate",
    "proxy-authorization",
];

/// `frame-ancestors` naming the dashboard origins on `host` (`:11180` and
/// `:80`), or `'none'` without an accepted host.
pub fn frame_ancestors(host: Option<&AcceptedHost>) -> String {
    match host {
        Some(h) => {
            let u = h.url_host();
            format!("frame-ancestors http://{u}:{DEFAULT_PORT} http://{u}:80")
        }
        None => "frame-ancestors 'none'".to_string(),
    }
}

/// One CSP header value with every `frame-ancestors` directive removed (a
/// value may hold several comma-separated policies). `None` when nothing is
/// left.
pub fn strip_frame_ancestors(value: &str) -> Option<String> {
    let policies: Vec<String> = value
        .split(',')
        .filter_map(|policy| {
            let kept: Vec<&str> = policy
                .split(';')
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .filter(|d| {
                    let name = d.split_ascii_whitespace().next().unwrap_or("");
                    !name.eq_ignore_ascii_case("frame-ancestors")
                })
                .collect();
            (!kept.is_empty()).then(|| kept.join("; "))
        })
        .collect();
    (!policies.is_empty()).then(|| policies.join(", "))
}

/// Rewrite every CSP header of `name` in place (non-UTF-8 values are dropped:
/// they cannot be checked for `frame-ancestors`).
fn rewrite_csp(h: &mut HeaderMap, name: &HeaderName) {
    let values: Vec<HeaderValue> = h.get_all(name).iter().cloned().collect();
    if values.is_empty() {
        return;
    }
    h.remove(name);
    for v in values {
        let Ok(s) = v.to_str() else { continue };
        if let Some(kept) = strip_frame_ancestors(s) {
            if let Ok(v) = HeaderValue::from_str(&kept) {
                h.append(name.clone(), v);
            }
        }
    }
}

/// Apply the §13 response policy to `h` for a request whose accepted host is
/// `host`.
pub fn apply(h: &mut HeaderMap, host: Option<&AcceptedHost>) {
    h.remove(header::X_FRAME_OPTIONS);
    for name in HOP_BY_HOP {
        h.remove(name);
    }
    let cors: Vec<HeaderName> = h
        .keys()
        .filter(|k| k.as_str().starts_with("access-control-"))
        .cloned()
        .collect();
    for k in cors {
        h.remove(k);
    }
    rewrite_csp(h, &header::CONTENT_SECURITY_POLICY);
    rewrite_csp(h, &header::CONTENT_SECURITY_POLICY_REPORT_ONLY);
    if let Ok(v) = HeaderValue::from_str(&frame_ancestors(host)) {
        h.append(header::CONTENT_SECURITY_POLICY, v);
    }
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
}

/// Is `name` one of ours (`s111`, `p111_*`)? Exact, case-sensitive (cookie
/// names are).
fn is_ours(name: &str) -> bool {
    name == crate::daemon::dashboard::api_v1::auth::COOKIE || name.starts_with("p111_")
}

/// Reserved proxy seam (§13): drop upstream `Set-Cookie`s named `s111` or
/// `p111_*`, strip `Domain=` from the rest.
pub fn filter_upstream_set_cookie(h: &mut HeaderMap) {
    let values: Vec<HeaderValue> = h.get_all(header::SET_COOKIE).iter().cloned().collect();
    h.remove(header::SET_COOKIE);
    for v in values {
        let Ok(s) = v.to_str() else { continue };
        let mut parts = s.split(';');
        let first = parts.next().unwrap_or("").trim();
        let name = first.split('=').next().unwrap_or("").trim();
        if name.is_empty() || is_ours(name) {
            continue;
        }
        let mut out = first.to_string();
        for attr in parts {
            let a = attr.trim();
            let key = a.split('=').next().unwrap_or("").trim();
            if a.is_empty() || key.eq_ignore_ascii_case("domain") {
                continue;
            }
            out.push_str("; ");
            out.push_str(a);
        }
        if let Ok(v) = HeaderValue::from_str(&out) {
            h.append(header::SET_COOKIE, v);
        }
    }
}

/// Reserved proxy seam (§13): remove `s111` / `p111_*` from the request's
/// `Cookie` headers before anything is forwarded upstream.
pub fn strip_request_cookies(h: &mut HeaderMap) {
    let values: Vec<HeaderValue> = h.get_all(header::COOKIE).iter().cloned().collect();
    h.remove(header::COOKIE);
    let kept: Vec<String> = values
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .map(str::trim)
        .filter(|kv| !kv.is_empty())
        .filter(|kv| !is_ours(kv.split('=').next().unwrap_or("").trim()))
        .map(str::to_string)
        .collect();
    if !kept.is_empty() {
        if let Ok(v) = HeaderValue::from_str(&kept.join("; ")) {
            h.insert(header::COOKIE, v);
        }
    }
}

/// `uri` with every ticket parameter value replaced by `<redacted>` (for log
/// lines; §13 "the ticket query is redacted from logs").
pub fn redact_ticket(uri: &str) -> String {
    let Some((path, query)) = uri.split_once('?') else {
        return uri.to_string();
    };
    let (query, frag) = match query.split_once('#') {
        Some((q, f)) => (q, Some(f)),
        None => (query, None),
    };
    let q: Vec<String> = query
        .split('&')
        .map(|kv| match kv.split_once('=') {
            Some((k, _)) if k == TICKET_PARAM => format!("{k}=<redacted>"),
            _ => kv.to_string(),
        })
        .collect();
    let mut out = format!("{path}?{}", q.join("&"));
    if let Some(f) = frag {
        out.push('#');
        out.push_str(f);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(h: &str) -> AcceptedHost {
        AcceptedHost {
            host: h.into(),
            port: 11190,
        }
    }

    #[test]
    fn csp_frame_ancestors_are_replaced() {
        assert_eq!(
            strip_frame_ancestors("default-src 'self'; frame-ancestors 'none'; img-src *"),
            Some("default-src 'self'; img-src *".into())
        );
        assert_eq!(strip_frame_ancestors("FRAME-ANCESTORS 'self'"), None);
        assert_eq!(
            strip_frame_ancestors("frame-ancestors x, script-src 'self'"),
            Some("script-src 'self'".into())
        );
        let mut h = HeaderMap::new();
        h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        h.append(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'; frame-ancestors 'none'"),
        );
        h.append(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("frame-ancestors 'self'"),
        );
        h.insert(
            header::CONTENT_SECURITY_POLICY_REPORT_ONLY,
            HeaderValue::from_static("frame-ancestors 'none'; report-uri /r"),
        );
        h.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        );
        h.insert(
            "access-control-allow-credentials",
            HeaderValue::from_static("true"),
        );
        h.insert(header::CONNECTION, HeaderValue::from_static("close"));
        h.insert("keep-alive", HeaderValue::from_static("timeout=5"));
        h.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("unsafe-url"),
        );
        apply(&mut h, Some(&host("127.0.0.1")));
        assert!(h.get(header::X_FRAME_OPTIONS).is_none());
        assert!(h.keys().all(|k| !k.as_str().starts_with("access-control-")));
        assert!(h.get(header::CONNECTION).is_none() && h.get("keep-alive").is_none());
        let csp: Vec<&str> = h
            .get_all(header::CONTENT_SECURITY_POLICY)
            .iter()
            .map(|v| v.to_str().unwrap())
            .collect();
        assert_eq!(
            csp,
            [
                "default-src 'self'",
                "frame-ancestors http://127.0.0.1:11180 http://127.0.0.1:80"
            ]
        );
        assert_eq!(
            h.get(header::CONTENT_SECURITY_POLICY_REPORT_ONLY).unwrap(),
            "report-uri /r"
        );
        assert_eq!(h.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");
        assert_eq!(h.get(header::REFERRER_POLICY).unwrap(), "no-referrer");

        let mut h = HeaderMap::new();
        apply(&mut h, Some(&host("::1")));
        assert_eq!(
            h.get(header::CONTENT_SECURITY_POLICY).unwrap(),
            "frame-ancestors http://[::1]:11180 http://[::1]:80"
        );
        let mut h = HeaderMap::new();
        apply(&mut h, None);
        assert_eq!(
            h.get(header::CONTENT_SECURITY_POLICY).unwrap(),
            "frame-ancestors 'none'"
        );
    }

    #[test]
    fn proxy_cookie_rules() {
        let mut h = HeaderMap::new();
        for c in [
            "s111=abc; Path=/",
            "p111_meshtastic=x; HttpOnly",
            "app=1; Domain=evil.com; Path=/; HttpOnly",
            "other=2",
        ] {
            h.append(header::SET_COOKIE, HeaderValue::from_str(c).unwrap());
        }
        filter_upstream_set_cookie(&mut h);
        let got: Vec<&str> = h
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap())
            .collect();
        assert_eq!(got, ["app=1; Path=/; HttpOnly", "other=2"]);

        let mut h = HeaderMap::new();
        h.append(
            header::COOKIE,
            HeaderValue::from_static("s111=a; app=1; p111_meshtastic=b"),
        );
        h.append(header::COOKIE, HeaderValue::from_static("p111_x=c"));
        strip_request_cookies(&mut h);
        assert_eq!(h.get(header::COOKIE).unwrap(), "app=1");
        let mut h = HeaderMap::new();
        h.append(header::COOKIE, HeaderValue::from_static("s111=a"));
        strip_request_cookies(&mut h);
        assert!(h.get(header::COOKIE).is_none());
    }

    #[test]
    fn ticket_is_redacted() {
        assert_eq!(
            redact_ticket("/?p111_ticket=00112233445566778899aabbccddeeff"),
            "/?p111_ticket=<redacted>"
        );
        assert_eq!(
            redact_ticket("/a?x=1&p111_ticket=abc&y=2#f"),
            "/a?x=1&p111_ticket=<redacted>&y=2#f"
        );
        assert_eq!(redact_ticket("/a?x=1"), "/a?x=1");
        assert_eq!(redact_ticket("/a"), "/a");
    }
}
