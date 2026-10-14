//! §8.1 / §18 rule 1: an unauthenticated LAN peer (hello + any self-signed
//! announce) that streams sealed frames faster than the fabric can open them
//! is back-pressured end to end — the hub's link reader, the adapter's
//! inbound queue and the fabric's frame queue are all bounded, so TCP flow
//! control stops the sender — instead of being buffered without bound in
//! the daemon's memory.

mod common;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::*;
use mesh111::contacts::ROLE_MAILBOX;
use mesh111::fabric::FRAME_QUEUE;
use mesh111::link::tcp_lan::{
    encode_frame, read_frame, read_hello, TcpLanConfig, HELLO, MAX_BODY, T_ANNOUNCE, T_SEALED,
};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

/// The kernel's socket-buffer ceiling on both ends of a loopback TCP
/// connection (`tcp_rmem` + `tcp_wmem` maxima; 16 MB when unreadable).
fn kernel_slack() -> u64 {
    let max = |path: &str| -> Option<u64> {
        std::fs::read_to_string(path)
            .ok()?
            .split_whitespace()
            .nth(2)?
            .parse()
            .ok()
    };
    match (
        max("/proc/sys/net/ipv4/tcp_rmem"),
        max("/proc/sys/net/ipv4/tcp_wmem"),
    ) {
        (Some(r), Some(w)) => r + w + (1 << 20),
        _ => 16 << 20,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_flooding_peer_is_back_pressured_not_buffered() {
    let d = Dirs::new();
    let hi = Ident::new("hub", ROLE_MAILBOX);
    let h = start(&hi, &d.node("hub"), hub_settings());
    let t = tcp(&h, &hi, "tcp");
    add_tcp(&h, &t, false).await;
    let haddr = listen(&t, 0).await;

    // Any LAN host: hello and a self-signed announce are all it takes.
    let mallory = Ident::new("mallory", 0);
    let mut s = TcpStream::connect(haddr).await.unwrap();
    s.write_all(&HELLO).await.unwrap();
    s.write_all(&encode_frame(T_ANNOUNCE, &mallory.announce_bytes()).unwrap())
        .await
        .unwrap();
    read_hello(&mut s).await.unwrap();
    assert_eq!(read_frame(&mut s).await.unwrap().0, T_ANNOUNCE);

    // The hub's actor is busy (a long store pass) while the peer streams
    // full-size sealed frames at line rate.
    let stalled = Arc::new(AtomicBool::new(false));
    let busy = {
        let (h, st) = (h.clone(), stalled.clone());
        tokio::spawn(async move {
            h.with_store(move |_| {
                st.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(3500));
            })
            .await
        })
    };
    until(10, || async {
        stalled.load(Ordering::SeqCst).then_some(())
    })
    .await;

    // Everything the hub may hold for this peer: the fabric's frame queue,
    // the adapter's inbound queue, one frame in each forwarding hand, plus
    // the kernel's socket buffers on both ends.
    let frame = encode_frame(T_SEALED, &vec![0x5a; MAX_BODY]).unwrap();
    let queued = FRAME_QUEUE + TcpLanConfig::new(Vec::new()).inbound_queue + 4;
    // (Kernel buffer accounting is approximate: twice its ceiling, plus
    // 8 MB, still sits far below what an unbounded queue swallows.)
    let bound = (queued * frame.len()) as u64 + 2 * kernel_slack() + (8 << 20);
    // The flooder tries far more than that (an unbounded queue takes it all).
    let target = (4 * bound).max(192 << 20);
    let written = AtomicU64::new(0);
    let flood = async {
        while written.load(Ordering::SeqCst) < target {
            if s.write_all(&frame).await.is_err() {
                return;
            }
            written.fetch_add(frame.len() as u64, Ordering::SeqCst);
        }
    };
    let finished = tokio::time::timeout(Duration::from_millis(2000), flood)
        .await
        .is_ok();
    let accepted = written.load(Ordering::SeqCst);
    eprintln!("flood: accepted {accepted} B of {target} B; bound {bound} B");
    assert!(
        !finished && accepted <= bound,
        "the hub took {accepted} B while its actor was busy (bound {bound} B)"
    );

    // The actor catches up: it opens what was queued (garbage: dropped and
    // counted, §18 rule 1) and keeps answering the API.
    busy.await.unwrap().unwrap();
    until(20, || async {
        let st = h.status().await.ok()?;
        (st.counters.dropped > 0).then_some(())
    })
    .await;
    drop(s);
    let st = h.status().await.unwrap();
    assert_eq!(
        st.links[0].peers, 0,
        "garbage never attributes a connection"
    );
}
