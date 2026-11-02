//! HTTPS fetching (ureq + rustls + webpki-roots — no native cert store dependency,
//! deterministic on musl). Used by installer, updater, and prefetch steps.
//!
//! Production callers pass **https** first-party endpoints only (GitHub Releases +
//! the own-domain mirror, per spec v2.1 "Security & trust model"). The scheme rule
//! is enforced here ([`check_url`], Layer 7 §9.5 T6):
//!
//! - `https://` passes, and a redirect may only go to another `https://` URL;
//! - `http://` to a loopback IP literal (`127.0.0.0/8`, `[::1]`) passes: the
//!   installer's own `/health` check of the local daemon and the loopback fixture
//!   servers of the tests; those bytes never leave the host;
//! - any other `http://` URL passes **only** as a typed [`HttpException`]: the home
//!   hub's LAN `origin_base` learned at enrolment, or a URL the user typed for
//!   `111 update --from`. Every byte fetched that way still passes minisign, the
//!   floor, the pins and the tree check (the updater); the transport is never
//!   trusted;
//! - every other scheme is refused, and no `http://` request follows a redirect.

use anyhow::Context;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

/// The two typed exceptions under which an `http://` URL to a non-loopback host
/// passes the fetcher (Layer 7 §9.5 T6). Nothing else may pass one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpException {
    /// The home hub's LAN `origin_base`, `http://<hub host>:11180/origin/v1/`
    /// (Layer 7 §9.1, S4).
    HomeHubOrigin,
    /// A URL the user typed for `111 update --from <url>` (Layer 7 §9.2).
    UserTypedFrom,
}

/// Why a URL was refused before any request was made (§9.5 T6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlRefused {
    /// `http://` to a host that is not a loopback IP literal, without a typed
    /// [`HttpException`].
    PlainHttp { url: String },
    /// Neither `https://` nor `http://`.
    Scheme { url: String },
}

impl std::fmt::Display for UrlRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UrlRefused::PlainHttp { url } => write!(
                f,
                "refusing plain http:// to a non-loopback host ({url}): only the home hub's \
                 origin and a URL typed for `111 update --from` may use http://"
            ),
            UrlRefused::Scheme { url } => {
                write!(
                    f,
                    "refusing a URL that is neither https:// nor http:// ({url})"
                )
            }
        }
    }
}

impl std::error::Error for UrlRefused {}

/// The scheme of a URL that passed [`check_url`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Https,
    Http,
}

/// The §9.5 T6 rule: `https://` passes; `http://` passes to a loopback IP literal,
/// or to any host under a typed `exception`; everything else is refused. The scheme
/// is matched case-insensitively (RFC 3986 §3.1).
pub fn check_url(url: &str, exception: Option<HttpException>) -> Result<Scheme, UrlRefused> {
    if strip_prefix_ci(url, "https://").is_some() {
        return Ok(Scheme::Https);
    }
    let Some(rest) = strip_prefix_ci(url, "http://") else {
        return Err(UrlRefused::Scheme {
            url: url.to_string(),
        });
    };
    if exception.is_some() || authority_is_loopback(rest) {
        Ok(Scheme::Http)
    } else {
        Err(UrlRefused::PlainHttp {
            url: url.to_string(),
        })
    }
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

/// Whether the authority at the start of `rest` (what follows `http://`) names a
/// loopback IP literal. Fails closed: userinfo (`@`), host names (`localhost`
/// included: a resolver decides where it points) and anything unparsable are not
/// loopback.
fn authority_is_loopback(rest: &str) -> bool {
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.contains('@') {
        return false;
    }
    if let Some(v6) = authority.strip_prefix('[') {
        let Some((host, after)) = v6.split_once(']') else {
            return false;
        };
        let port_ok = after.is_empty()
            || after
                .strip_prefix(':')
                .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        return port_ok && host.parse::<Ipv6Addr>().is_ok_and(|ip| ip.is_loopback());
    }
    let host = match authority.split_once(':') {
        Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => h,
        Some(_) => return false,
        None => authority,
    };
    host.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
}

pub struct Fetcher {
    agent: ureq::Agent,
    max_bytes: u64,
    http_exception: Option<HttpException>,
}

impl Fetcher {
    /// `timeout` bounds connect AND the whole call; bodies are capped at `max_bytes`
    /// (a larger body is a hard error, not a truncation). No [`HttpException`]:
    /// `http://` passes only to a loopback IP literal.
    pub fn new(timeout: Duration, max_bytes: u64) -> anyhow::Result<Self> {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::WebPki)
            .build();
        let config = ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_connect(Some(timeout))
            .timeout_global(Some(timeout))
            .build();
        Ok(Fetcher {
            agent: ureq::Agent::new_with_config(config),
            max_bytes,
            http_exception: None,
        })
    }

    /// This fetcher with the typed `http://` exception (§9.5 T6): the updater builds
    /// one per home-hub or `--from <url>` origin, never for an internet base.
    pub fn allowing_http(mut self, exception: HttpException) -> Self {
        self.http_exception = Some(exception);
        self
    }

    /// The typed exception this fetcher carries, if any.
    pub fn http_exception(&self) -> Option<HttpException> {
        self.http_exception
    }

    /// The scheme rule (§9.5 T6) before any request: an `https://` request then
    /// follows redirects to `https://` only; an `http://` request follows none
    /// (a 3xx answer is an error).
    fn checked(&self, url: &str) -> anyhow::Result<Scheme> {
        Ok(check_url(url, self.http_exception)?)
    }

    /// GET `url`, return body (capped). Errors carry the URL.
    pub fn get(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        self.get_capped(url, self.max_bytes)
    }

    /// [`Fetcher::get`] with a tighter body cap (`max_bytes`, never above the
    /// fetcher's own): a manifest is read at most at its parse limit.
    pub fn get_capped(&self, url: &str, max_bytes: u64) -> anyhow::Result<Vec<u8>> {
        let cap = max_bytes.min(self.max_bytes);
        let scheme = self.checked(url)?;
        let req = self.agent.get(url).config();
        let req = match scheme {
            Scheme::Https => req.https_only(true),
            Scheme::Http => req.max_redirects(0),
        }
        .build();
        let mut resp = req.call().with_context(|| format!("GET {url}"))?;
        anyhow::ensure!(
            !resp.status().is_redirection(),
            "GET {url}: refusing to follow a redirect from plain http:// ({})",
            resp.status()
        );
        let body = resp
            .body_mut()
            .with_config()
            // ureq's limit refuses a read once `limit` bytes are in, EOF included,
            // so a body of exactly `cap` bytes needs one byte of room.
            .limit(cap.saturating_add(1))
            .read_to_vec()
            .with_context(|| format!("GET {url}: reading body (cap {cap} bytes)"))?;
        anyhow::ensure!(
            body.len() as u64 <= cap,
            "GET {url}: body exceeds the cap of {cap} bytes"
        );
        Ok(body)
    }

    /// HEAD `url`, returning the HTTP Date header value when present (clock sanity).
    pub fn head_date(&self, url: &str) -> anyhow::Result<Option<String>> {
        let scheme = self.checked(url)?;
        let req = self.agent.head(url).config();
        let req = match scheme {
            Scheme::Https => req.https_only(true),
            Scheme::Http => req.max_redirects(0),
        }
        .build();
        let resp = req.call().with_context(|| format!("HEAD {url}"))?;
        Ok(resp
            .headers()
            .get("date")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned))
    }
}

/// Whether `err` (from [`Fetcher::get`]) is an HTTP 404 answer: the updater tries
/// the next origin on one (Layer 7 §9.3, G12).
pub fn is_not_found(err: &anyhow::Error) -> bool {
    err.chain().any(|e| {
        matches!(
            e.downcast_ref::<ureq::Error>(),
            Some(ureq::Error::StatusCode(404))
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_always_passes() {
        for url in [
            "https://install.111.network/stable/channel-111-stable.toml",
            "HTTPS://github.com/x",
            "https://10.0.0.1/x",
        ] {
            assert_eq!(check_url(url, None), Ok(Scheme::Https), "{url}");
        }
    }

    #[test]
    fn loopback_http_passes_without_an_exception() {
        for url in [
            "http://127.0.0.1:11180/health",
            "http://127.0.0.1",
            "http://127.1.2.3:9/x?y",
            "http://[::1]:11180/",
            "http://[::1]",
            "HTTP://127.0.0.1:1/",
        ] {
            assert_eq!(check_url(url, None), Ok(Scheme::Http), "{url}");
        }
    }

    #[test]
    fn other_http_needs_a_typed_exception() {
        for url in [
            "http://192.168.1.10:11180/origin/v1/",
            "http://10.0.0.7/",
            "http://example.com/x",
            "http://localhost:11180/",
            "http://user@127.0.0.1:1/",
            "http://127.0.0.1@evil.example/",
            "http://127.0.0.1:port/",
            "http://127.0.0.1:/",
            "http://[::1/",
            "http://[::1]x/",
            "http://[fd00::1]:11180/",
            "http://",
        ] {
            assert_eq!(
                check_url(url, None),
                Err(UrlRefused::PlainHttp { url: url.into() }),
                "{url}"
            );
            for e in [HttpException::HomeHubOrigin, HttpException::UserTypedFrom] {
                assert_eq!(check_url(url, Some(e)), Ok(Scheme::Http), "{url} {e:?}");
            }
        }
    }

    #[test]
    fn other_schemes_are_refused_even_under_an_exception() {
        for url in ["file:///etc/passwd", "ftp://x/", "/media/stick", "", "htt"] {
            for e in [None, Some(HttpException::UserTypedFrom)] {
                assert_eq!(
                    check_url(url, e),
                    Err(UrlRefused::Scheme { url: url.into() }),
                    "{url}"
                );
            }
        }
    }

    #[test]
    fn the_fetcher_refuses_before_any_request() {
        let f = Fetcher::new(Duration::from_secs(1), 1024).unwrap();
        assert_eq!(f.http_exception(), None);
        let err = f.get("http://192.0.2.1:9/x").unwrap_err();
        assert!(err.downcast_ref::<UrlRefused>().is_some(), "{err:#}");
        let err = f.head_date("http://192.0.2.1:9/x").unwrap_err();
        assert!(err.downcast_ref::<UrlRefused>().is_some(), "{err:#}");
        let f = f.allowing_http(HttpException::HomeHubOrigin);
        assert_eq!(f.http_exception(), Some(HttpException::HomeHubOrigin));
        assert!(f.get("file:///etc/passwd").is_err());
    }

    /// A plain-http answer that redirects is an error, never followed.
    #[test]
    fn http_redirects_are_not_followed() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = conn.read(&mut buf);
            let _ = conn.write_all(
                b"HTTP/1.1 302 Found\r\nLocation: http://192.0.2.1/x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        });
        let f = Fetcher::new(Duration::from_secs(5), 1024).unwrap();
        let err = f.get(&format!("http://127.0.0.1:{port}/x")).unwrap_err();
        assert!(format!("{err:#}").contains("redirect"), "{err:#}");
        server.join().unwrap();
    }

    /// A body of exactly the cap passes; one byte more is an error.
    #[test]
    fn the_cap_is_inclusive() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut conn, _) = listener.accept().unwrap();
                let mut buf = [0u8; 1024];
                let _ = conn.read(&mut buf);
                let _ = conn.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd",
                );
            }
        });
        let f = Fetcher::new(Duration::from_secs(5), 1024).unwrap();
        let url = format!("http://127.0.0.1:{port}/x");
        assert_eq!(f.get_capped(&url, 4).unwrap(), b"abcd");
        assert!(f.get_capped(&url, 3).is_err());
        server.join().unwrap();
    }

    #[test]
    fn not_found_is_recognised() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = conn.read(&mut buf);
            let _ = conn.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        });
        let f = Fetcher::new(Duration::from_secs(5), 1024).unwrap();
        let err = f.get(&format!("http://127.0.0.1:{port}/x")).unwrap_err();
        assert!(is_not_found(&err), "{err:#}");
        server.join().unwrap();
        let refused = f.get("http://192.0.2.1/x").unwrap_err();
        assert!(!is_not_found(&refused));
    }
}
