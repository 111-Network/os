//! Shared mock-HTTP-server helpers for agent111 integration tests.
//!
//! std-only per the dependency policy (no HTTP-mock crates): a hand-rolled
//! HTTP/1.1 server on `std::net::TcpListener`, bound to `127.0.0.1` port 0
//! (ephemeral — tests never touch fixed ports or the real network).
//!
//! One `MockServer` per test. The accept loop runs on its own thread and
//! shuts down on drop; each connection is handled on a detached thread, so
//! a handler that blocks forever (timeout tests) can never stall the accept
//! loop or test teardown.
//!
//! Not every test binary uses every helper — allow dead code here rather
//! than trimming a shared module per-consumer.
#![allow(dead_code)]

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// One parsed HTTP/1.1 request, handed to the handler.
#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// First header value with this (case-insensitive) name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Parse the body as JSON; panics (fails the test) if it is not JSON.
    pub fn body_json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("request body should be JSON")
    }
}

pub struct MockServer {
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
}

impl MockServer {
    /// Start a server whose handler receives each parsed request and the
    /// open stream to write the response into. The connection closes when
    /// the handler returns (drop of the stream), so write everything first.
    pub fn start<F>(handler: F) -> MockServer
    where
        F: Fn(Request, &mut TcpStream) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind mock server");
        let addr = listener.local_addr().expect("mock server local addr");
        // Nonblocking accept + a poll on the shutdown flag, so drop can
        // always stop the loop without a self-connect wakeup.
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&shutdown);
        let handler = Arc::new(handler);
        let accept_thread = thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let handler = Arc::clone(&handler);
                        // Detached: a handler may legitimately block (e.g.
                        // the never-respond timeout test); it must not stall
                        // the accept loop or server shutdown.
                        thread::spawn(move || {
                            if let Some(req) = read_request(&mut stream) {
                                handler(req, &mut stream);
                            }
                        });
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        MockServer {
            addr,
            shutdown,
            accept_thread: Some(accept_thread),
        }
    }

    /// `http://127.0.0.1:<port>` base URL for building a `Client`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
    }
}

/// Read one request: head up to `\r\n\r\n`, then exactly Content-Length
/// body bytes (absent header ⇒ no body). Returns `None` on EOF, timeout,
/// or garbage — the connection is then closed without a response.
fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 8192];
    let header_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > 1024 * 1024 {
            return None;
        }
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut headers = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_string();
            let value = value.trim().to_string();
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
        }
    }

    let mut body = buf[header_end..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(content_length);

    Some(Request {
        method,
        path,
        headers,
        body,
    })
}

/// Write a complete response with a Content-Length body. The connection
/// closes when the handler returns.
pub fn respond(stream: &mut TcpStream, status: u16, reason: &str, content_type: &str, body: &str) {
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

/// An open SSE response. Each `data()` call writes and flushes one
/// `data: <payload>\n\n` event immediately, and the connection stays open
/// until the sink (and then the handler) drops — events genuinely stream
/// instead of arriving as one buffer, and nothing closes early.
pub struct SseSink<'a> {
    stream: &'a mut TcpStream,
}

/// Write the SSE response headers (200, `text/event-stream`, no
/// Content-Length: the body is delimited by connection close, HTTP/1.1).
pub fn begin_sse(stream: &mut TcpStream) -> std::io::Result<SseSink<'_>> {
    stream.write_all(
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n",
    )?;
    stream.flush()?;
    Ok(SseSink { stream })
}

impl SseSink<'_> {
    pub fn data(&mut self, payload: &str) -> std::io::Result<()> {
        self.stream.write_all(b"data: ")?;
        self.stream.write_all(payload.as_bytes())?;
        self.stream.write_all(b"\n\n")?;
        self.stream.flush()
    }

    /// The OpenAI stream terminator: `data: [DONE]`.
    pub fn done(&mut self) -> std::io::Result<()> {
        self.data("[DONE]")
    }
}
