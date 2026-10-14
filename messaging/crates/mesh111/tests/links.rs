//! §8.1 reconnect with back-off: a dial answered by another node than the
//! one expected at that address (a stale hub address, a mistyped enrollment
//! address) is a failed attempt — the back-off grows, the connection is never
//! reported, and the dialer does not spin with full handshakes.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::*;
use mesh111::fabric::FabricSettings;
use mesh111::link::tcp_lan::{
    encode_frame, read_frame, read_hello, TcpLan, TcpLanConfig, HELLO, T_ANNOUNCE,
};
use mesh111::link::{Dialer, LinkId};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_claim_mismatch_backs_off_like_a_failed_dial() {
    // Carol answers at the address where Alice expects Bob.
    let carol = Ident::new("carol", 0);
    let bob = Ident::new("bob", 0);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let dials = Arc::new(AtomicUsize::new(0));
    let counted = dials.clone();
    let announce = carol.announce_bytes();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else {
                return;
            };
            counted.fetch_add(1, Ordering::SeqCst);
            let announce = announce.clone();
            tokio::spawn(async move {
                let _ = s.write_all(&HELLO).await;
                let _ = s
                    .write_all(&encode_frame(T_ANNOUNCE, &announce).unwrap())
                    .await;
                if read_hello(&mut s).await.is_err() {
                    return;
                }
                while read_frame(&mut s).await.is_ok() {}
            });
        }
    });

    let d = Dirs::new();
    let ai = Ident::new("alice", 0);
    let a = start(&ai, &d.node("a"), FabricSettings::default());
    let mut cfg = TcpLanConfig::new(ai.announce_bytes());
    cfg.refuse_loopback = false;
    cfg.backoff_min = Duration::from_millis(50);
    cfg.backoff_max = Duration::from_secs(10);
    let t = TcpLan::new(LinkId::new("tcp"), cfg, a.link_events(), a.gate());
    add_tcp(&a, &t, true).await;
    t.maintain(addr, Some(bob.id()));

    // Back-off 50, 100, 200, 400, 800 ms: at most six attempts in 1.4 s
    // (a reset back-off would redial every ~50 ms, ~25 times).
    tokio::time::sleep(Duration::from_millis(1400)).await;
    let n = dials.load(Ordering::SeqCst);
    assert!((2..=6).contains(&n), "{n} dials in 1.4 s");
    assert_eq!(t.connection_count(), 0, "a misdial is never a connection");
    let st = a.status().await.unwrap();
    assert_eq!(st.links[0].peers, 0);
    t.release(addr);
    server.abort();
}
