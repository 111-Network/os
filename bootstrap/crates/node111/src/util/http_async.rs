//! Minimal async HTTP/1.1 client for **loopback** upstreams (Layer 4 R3: the
//! `111d` → `111-agent` relay, `daemon/dashboard/api_v1/relay.rs`).
//!
//! One TCP connection per request: no pool, no TLS, no redirects, no proxy —
//! the only peer is a child process on this node. Built on hyper's connection
//! API (hyper is already in the graph via axum; see Cargo.toml).
//!
//! - [`Target::parse`] refuses anything but `http://<loopback>[:port]`, so a
//!   misconfigured base can never turn the relay into an open proxy.
//! - [`send`] bounds the connect phase (`connect_timeout`) and the wait for
//!   the response head (`head_deadline`). Body reads are bounded by the caller
//!   ([`Response::read_to_end`] takes a deadline; streams wrap [`Response::chunk`]).
//! - Dropping a [`Response`] tears the TCP connection down at once. That is
//!   how a dashboard client disconnect cancels an upstream chat turn.

use axum::body::Bytes;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::time::Instant;

/// A loopback upstream: the socket to dial and the `Host` header to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    addr: SocketAddr,
    authority: String,
}

impl Target {
    /// Parse `http://<127.x.y.z | [::1] | localhost>[:port][/]` (default port 80).
    /// Anything else (https, a non-loopback host, a path, userinfo) is refused.
    pub fn parse(base: &str) -> Result<Target, String> {
        let base = base.trim();
        let authority = base
            .strip_prefix("http://")
            .ok_or_else(|| format!("not an http:// URL: {base}"))?
            .trim_end_matches('/');
        if authority.is_empty() || authority.contains(['/', '?', '#', '@']) {
            return Err(format!("not a bare http://host:port URL: {base}"));
        }
        let (host, port) = split_host_port(authority)?;
        let ip: IpAddr = if host.eq_ignore_ascii_case("localhost") {
            Ipv4Addr::LOCALHOST.into()
        } else {
            host.parse()
                .map_err(|_| format!("host is not an IP literal or localhost: {base}"))?
        };
        if !ip.is_loopback() {
            return Err(format!("refusing a non-loopback upstream: {base}"));
        }
        Ok(Target {
            addr: SocketAddr::new(ip, port),
            authority: authority.to_ascii_lowercase(),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `host[:port]` as sent in the `Host` header.
    pub fn authority(&self) -> &str {
        &self.authority
    }
}

fn split_host_port(authority: &str) -> Result<(&str, u16), String> {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest
            .split_once(']')
            .ok_or_else(|| format!("unterminated IPv6 literal: {authority}"))?;
        match after {
            "" => (host, None),
            _ => match after.strip_prefix(':') {
                Some(p) => (host, Some(p)),
                None => return Err(format!("bad authority: {authority}")),
            },
        }
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    let port = match port {
        None => 80,
        Some(p) => p.parse().map_err(|_| format!("bad port in {authority}"))?,
    };
    Ok((host, port))
}

/// Why an upstream exchange failed (the relay maps these to problem codes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Nothing accepted the connection (refused, unreachable, connect timeout).
    Connect(String),
    /// A time budget ran out (response head or body).
    Timeout,
    /// The peer broke the exchange (closed early, malformed HTTP, body over cap).
    Protocol(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Connect(d) => write!(f, "connect failed: {d}"),
            Error::Timeout => f.write_str("timed out"),
            Error::Protocol(d) => write!(f, "protocol error: {d}"),
        }
    }
}

impl std::error::Error for Error {}

/// Aborts the connection task (closing the socket) when dropped.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// An upstream response whose head has arrived; the body is read on demand.
pub struct Response {
    pub status: StatusCode,
    pub headers: HeaderMap,
    body: hyper::body::Incoming,
    // Kept alive while the body is read; dropping the Response drops both.
    _sender: http1::SendRequest<Full<Bytes>>,
    _conn: AbortOnDrop,
}

impl Response {
    /// `Content-Type` essence, lowercased, without parameters
    /// (`text/plain; charset=utf-8` → `text/plain`).
    pub fn media_type(&self) -> Option<String> {
        let v = self.headers.get(header::CONTENT_TYPE)?.to_str().ok()?;
        let essence = v.split(';').next().unwrap_or("").trim();
        (!essence.is_empty()).then(|| essence.to_ascii_lowercase())
    }

    /// Next non-empty body data chunk, exactly as it arrived; `None` at the
    /// clean end of the body. Trailers are skipped.
    pub async fn chunk(&mut self) -> Option<Result<Bytes, Error>> {
        loop {
            match self.body.frame().await? {
                Err(e) => return Some(Err(Error::Protocol(e.to_string()))),
                Ok(frame) => {
                    if let Ok(data) = frame.into_data() {
                        if !data.is_empty() {
                            return Some(Ok(data));
                        }
                    }
                }
            }
        }
    }

    /// The whole body; more than `cap` bytes or running past `deadline` is an error.
    pub async fn read_to_end(mut self, cap: usize, deadline: Instant) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        loop {
            match tokio::time::timeout_at(deadline, self.chunk()).await {
                Err(_) => return Err(Error::Timeout),
                Ok(None) => return Ok(out),
                Ok(Some(Err(e))) => return Err(e),
                Ok(Some(Ok(b))) => {
                    if out.len() + b.len() > cap {
                        return Err(Error::Protocol(format!("body larger than {cap} bytes")));
                    }
                    out.extend_from_slice(&b);
                }
            }
        }
    }
}

fn protocol(e: hyper::Error) -> Error {
    Error::Protocol(e.to_string())
}

/// One request to `target`. `path` is origin-form (`/v1/chat?x=y`); a `json`
/// body is sent with `Content-Type: application/json`. Connect is bounded by
/// `connect_timeout`; connect + request + response head by `head_deadline`.
pub async fn send(
    target: &Target,
    method: Method,
    path: &str,
    json: Option<Vec<u8>>,
    connect_timeout: Duration,
    head_deadline: Instant,
) -> Result<Response, Error> {
    let connect =
        tokio::time::timeout(connect_timeout, tokio::net::TcpStream::connect(target.addr));
    let stream = match tokio::time::timeout_at(head_deadline, connect).await {
        Err(_) | Ok(Err(_)) => {
            return Err(Error::Connect(format!(
                "no answer from {} (connect timed out)",
                target.authority
            )))
        }
        Ok(Ok(Err(e))) => return Err(Error::Connect(format!("{}: {e}", target.authority))),
        Ok(Ok(Ok(s))) => s,
    };
    let _ = stream.set_nodelay(true);

    let exchange = async {
        let (mut sender, conn) = http1::handshake(hyper_util::rt::TokioIo::new(stream))
            .await
            .map_err(protocol)?;
        let guard = AbortOnDrop(
            tokio::spawn(async move {
                let _ = conn.await;
            })
            .abort_handle(),
        );
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, &target.authority)
            .header(header::USER_AGENT, "111d-relay");
        let body = match json {
            Some(b) => {
                req = req
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::CONTENT_LENGTH, b.len());
                Full::new(Bytes::from(b))
            }
            None => Full::new(Bytes::new()),
        };
        let req = req.body(body).map_err(|e| Error::Protocol(e.to_string()))?;
        let resp = sender.send_request(req).await.map_err(protocol)?;
        let (parts, body) = resp.into_parts();
        Ok(Response {
            status: parts.status,
            headers: parts.headers,
            body,
            _sender: sender,
            _conn: guard,
        })
    };
    tokio::time::timeout_at(head_deadline, exchange)
        .await
        .unwrap_or(Err(Error::Timeout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_parse_loopback_only() {
        let t = Target::parse("http://127.0.0.1:11182").unwrap();
        assert_eq!(t.addr(), "127.0.0.1:11182".parse().unwrap());
        assert_eq!(t.authority(), "127.0.0.1:11182");
        assert_eq!(
            Target::parse("http://LOCALHOST:9/").unwrap().addr(),
            "127.0.0.1:9".parse().unwrap()
        );
        let v6 = Target::parse("http://[::1]:11182").unwrap();
        assert_eq!(v6.addr(), "[::1]:11182".parse().unwrap());
        assert_eq!(v6.authority(), "[::1]:11182");
        assert_eq!(Target::parse("http://127.0.0.2").unwrap().addr().port(), 80);
        for bad in [
            "https://127.0.0.1:11182",
            "http://192.168.1.10:11182",
            "http://example.com:11182",
            "http://127.0.0.1:11182/v1",
            "http://user@127.0.0.1:1",
            "http://127.0.0.1:port",
            "http://[::1",
            "http://[::1]x",
            "http://",
            "127.0.0.1:11182",
        ] {
            assert!(Target::parse(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn refused_connection_is_connect_error() {
        // A just-released ephemeral port can be re-bound by a parallel test before we connect
        // (seen once in CI-style runs): retry on a fresh port instead of asserting on a race.
        let mut last = None;
        for _ in 0..5 {
            let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = l.local_addr().unwrap().port();
            drop(l);
            let t = Target::parse(&format!("http://127.0.0.1:{port}")).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            match send(&t, Method::GET, "/", None, Duration::from_secs(2), deadline).await {
                Err(Error::Connect(_)) => return,
                Err(e) => last = Some(e.to_string()),
                Ok(r) => last = Some(format!("unexpected response {}", r.status)),
            }
        }
        panic!("never saw a connect error: {last:?}");
    }
}
