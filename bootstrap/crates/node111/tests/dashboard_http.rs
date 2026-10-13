//! T8 acceptance: dashboard HTTP behavior on an ephemeral loopback port —
//! GET-only, Host allowlist, /health + /api/status contracts, loopback assert,
//! and a rendered-page identity-material grep.

use node111::daemon::dashboard::{serve, DashboardState, SharedState, TransportState};
use node111::daemon::health::TransportHealth;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::{Arc, RwLock};

fn http_get(port: u16, path: &str, host: &str) -> (u16, String) {
    http_req(port, "GET", path, host)
}

fn http_req(port: u16, method: &str, path: &str, host: &str) -> (u16, String) {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    conn.write_all(req.as_bytes()).unwrap();
    let mut buf = String::new();
    conn.read_to_string(&mut buf).unwrap();
    let status: u16 = buf
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

async fn start_server(state: SharedState) -> u16 {
    // Bind an ephemeral port first to learn a free one, drop it, then serve.
    let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let bind = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    tokio::spawn(async move {
        let _ = serve(state, bind, false).await;
    });
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return port;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("server never came up");
}

/// A §7 machine-string NodeId (lowercase hex of a 32B Ed25519 public key).
const NODE_ID_HEX: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

fn fresh_state() -> SharedState {
    Arc::new(RwLock::new(DashboardState::new("2026.10.12")))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn page_and_api_contracts() {
    let state = fresh_state();
    {
        let mut s = state.write().unwrap();
        s.node_id = Some(NODE_ID_HEX.into());
        s.category = Some("Node2A".into());
        s.transport_state = TransportState::NoTransport;
        s.components.transport = TransportHealth::DeliberatelyAbsent;
        s.components.identity = true;
        s.components.kb = true;
        s.kb = Some(node111::daemon::kb::KbStatus {
            sha256: "a".repeat(64),
            entries: 9,
        });
    }
    let port = start_server(state).await;

    // index page
    let (code, body) = http_get(port, "/", "127.0.0.1");
    assert_eq!(code, 200);
    assert!(body.contains("111 node"));

    // identity-material grep over the rendered page: no key-bearing strings.
    for banned in ["mnemonic", "keyset", "BIP-39"] {
        assert!(!body.contains(banned), "page leaks {banned}");
    }

    // /api/status schema
    let (code, body) = http_get(port, "/api/status", "localhost:11180");
    assert_eq!(code, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["node"]["version"], "2026.10.12");
    // L1 v2.2: `node.node_id` = the §7 machine string (lowercase hex). The node
    // object is pinned to exactly these keys — the provisional-identity key is
    // gone from the seam Layer 4 codes against, and nothing else was added.
    assert_eq!(v["node"]["node_id"], NODE_ID_HEX);
    let mut keys: Vec<&str> = v["node"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["category", "node_id", "uptime_sec", "version"]);
    assert_eq!(v["transport_state"], "no transport");
    assert_eq!(v["health"]["status"], "ok");
    assert_eq!(v["health"]["components"]["transport"], "absent");
    assert_eq!(v["kb"]["entries"], 9);
    assert_eq!(v["clock_fault"], false);
    assert!(v["offer"].is_null());
    assert!(v["error_state"].is_null());
    // identity material must not appear anywhere in the JSON
    let raw = v.to_string();
    for banned in ["secret", "mnemonic", "keyset"] {
        assert!(!raw.contains(banned), "api leaks {banned}: {raw}");
    }

    // /health green → 200 with checklist
    let (code, body) = http_get(port, "/health", "127.0.0.1");
    assert_eq!(code, 200);
    let h: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(h["status"], "ok");
    assert!(h["components"]["identity"].as_bool().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_503_when_red() {
    let port = start_server(fresh_state()).await; // default: everything red
    let (code, body) = http_get(port, "/health", "127.0.0.1");
    assert_eq!(code, 503);
    let h: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(h["status"], "error");
    assert_eq!(h["components"]["transport"], "down");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_only_and_host_allowlist() {
    let port = start_server(fresh_state()).await;

    let (code, _) = http_req(port, "POST", "/api/status", "127.0.0.1");
    assert_eq!(code, 405);
    let (code, _) = http_req(port, "DELETE", "/", "127.0.0.1");
    assert_eq!(code, 405);

    for bad_host in ["evil.example.com", "127.0.0.1.evil.com", "192.168.1.5"] {
        let (code, _) = http_get(port, "/api/status", bad_host);
        assert_eq!(code, 400, "host {bad_host} must be refused");
    }
    let (code, _) = http_get(port, "/api/status", "localhost:11180");
    assert_eq!(code, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_loopback_bind_refused_without_optin() {
    let state = fresh_state();
    let bind = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0));
    let err = serve(state, bind, false).await.unwrap_err();
    assert!(err.to_string().contains("LAN opt-in"), "{err}");
}
