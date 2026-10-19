//! Layer 6 §3 "Typed writes, batched" (integration M3; G1 carries the same
//! shape): a 1,000-row foreign burst, ingested on one side in batches of
//! 256 through the actor (one SQLCipher transaction per call, each call
//! awaited before the next — the bridge core's back-pressure), must not
//! starve 111 traffic. Two in-process fabrics exchange 111 messages over
//! the `lossy-loopback` link while the burst runs: each reaches `delivered`
//! within the bound, and every foreign row is stored once, in contiguous
//! `local_seq` order per batch. Foreign events stay coalesced per
//! conversation per call for an unpinned channel.
//!
//! The overlap is proven, not assumed (B1-bounce F1): the burst task fires
//! a oneshot once its first call returned, the two 111 sends are issued
//! only then, and the burst's second call waits for the "sends issued"
//! signal, so at least three of the four calls (768 rows) run while both
//! 111 messages are in flight. The test asserts that the last ingest
//! returned after the sends were issued.

mod common;

use std::time::{Duration, Instant};

use common::*;
use mesh111::fabric::{FabricEvent, FabricSettings, ForeignChange, MAX_FOREIGN_INGEST};
use mesh111::link::lossy::LossyConfig;
use mesh111::store::{ForeignAuth, ForeignInsert, ForeignKind, MsgState};

/// The assertion bound on 111 delivery latency under the burst (wall time
/// on the shared build box; the measured value is printed).
const BOUND: Duration = Duration::from_secs(10);
const ROWS: usize = 1_000;

fn burst_row(conv: i64, i: usize) -> ForeignInsert {
    ForeignInsert {
        conv,
        sender_native: format!("!{:08x}", 0xa1b2_0000_u32 + (i % 37) as u32),
        sender_name: Some(format!("node {}", i % 37)),
        auth: ForeignAuth::ChannelKey,
        // 32-bit Meshtastic packet ids: unique here, so nothing dedupes
        native_msg_id: Some(format!("{:08x}", 0x1000_0000_u32 + i as u32)),
        body: format!("{i:04} {}", "m".repeat(180)),
        claim_ts: None,
        meta: Some(r#"{"snr":6.5,"rssi":-97,"hops":2,"via_mqtt":false}"#.into()),
        id_globally_unique: false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_thousand_row_burst_does_not_starve_111_delivery() {
    let d = Dirs::new();
    let ai = Ident::new("alice", 0);
    let bi = Ident::new("bob", 0);
    let a = start(&ai, &d.node("a"), FabricSettings::default());
    let b = start(&bi, &d.node("b"), FabricSettings::default());
    exchange_cards(&a, &b).await;
    let _ends = lossy_pair(
        (&a, &ai),
        (&b, &bi),
        LossyConfig::default(),
        LossyConfig::default(),
    )
    .await;
    // a warm-up message: both sides hold a connection attributed to the other
    let warm = a.send(bi.id(), "warm-up", None).await.unwrap();
    until_state(&a, warm, MsgState::Delivered, 20).await;

    let conv = a
        .foreign_conversation_upsert(
            "meshtastic",
            "!board",
            ForeignKind::Channel,
            "ch:LongFast:9f3c",
            Some("LongFast"),
        )
        .await
        .unwrap();
    // a barrier call first: the upsert's tick has emitted its event
    a.foreign_conversations(None).await.unwrap();
    let mut events = a.subscribe();

    // The burst: four awaited calls (256, 256, 256, 232), as an adapter
    // tick would issue them. F1 gates: `first_done` fires when the first
    // call returned; the second call waits for `sends_issued`.
    let (first_done_tx, first_done) = tokio::sync::oneshot::channel::<()>();
    let (sends_issued_tx, sends_issued) = tokio::sync::oneshot::channel::<()>();
    let fa = a.clone();
    let burst = tokio::spawn(async move {
        let all: Vec<ForeignInsert> = (0..ROWS).map(|i| burst_row(conv, i)).collect();
        let started = Instant::now();
        let mut batches = Vec::new();
        let mut first_done_tx = Some(first_done_tx);
        let mut sends_issued = Some(sends_issued);
        for (i, chunk) in all.chunks(MAX_FOREIGN_INGEST).enumerate() {
            if i == 1 {
                if let Some(gate) = sends_issued.take() {
                    gate.await.expect("the sends were issued");
                }
            }
            let ids = fa.foreign_ingest(chunk.to_vec()).await.expect("ingest");
            batches.push(ids);
            if let Some(tx) = first_done_tx.take() {
                let _ = tx.send(());
            }
        }
        // when the LAST ingest returned
        let ended = Instant::now();
        (batches, started.elapsed(), ended)
    });

    // 111 traffic both ways while the burst runs: issued only once the
    // burst's first call has returned.
    first_done.await.expect("the burst started");
    let t0 = Instant::now();
    let ab = a
        .send(bi.id(), "alice → bob during the burst", None)
        .await
        .unwrap();
    let ba = b
        .send(ai.id(), "bob → alice during the burst", None)
        .await
        .unwrap();
    let issued = Instant::now();
    sends_issued_tx.send(()).expect("the burst waits");
    until_state(&a, ab, MsgState::Delivered, BOUND.as_secs()).await;
    let lat_ab = t0.elapsed();
    until_state(&b, ba, MsgState::Delivered, BOUND.as_secs()).await;
    let lat_ba = t0.elapsed();
    let (batches, burst_time, burst_ended) = burst.await.unwrap();
    let overlap = burst_ended.saturating_duration_since(issued);
    println!(
        "foreign burst: {ROWS} rows in {} calls took {burst_time:?}; \
         111 latency under it: alice→bob delivered after {lat_ab:?}, \
         bob→alice after {lat_ba:?} (bound {BOUND:?}); the burst ran \
         {overlap:?} past the sends being issued",
        batches.len()
    );
    // F1: the bound was measured during the burst — its last call returned
    // after both 111 sends were issued.
    assert!(
        burst_ended > issued,
        "the burst ended before the 111 sends were issued"
    );
    assert!(lat_ab < BOUND && lat_ba < BOUND);
    assert_eq!(batches.len(), ROWS.div_ceil(MAX_FOREIGN_INGEST));

    // Every row is stored once; per batch the local_seq run is contiguous
    // (one transaction), and the whole burst is in ingest order.
    let ids: Vec<i64> = batches
        .iter()
        .flatten()
        .map(|id| id.expect("new row"))
        .collect();
    assert_eq!(ids.len(), ROWS);
    let page = a.foreign_thread(conv, Some(0), ROWS + 10).await.unwrap();
    assert!(!page.more);
    assert_eq!(page.messages.len(), ROWS, "every foreign row is present");
    assert_eq!(page.conversation.messages, ROWS as u64);
    assert_eq!(page.conversation.unread, ROWS as u64);
    let stored: Vec<i64> = page.messages.iter().map(|m| m.id).collect();
    assert_eq!(stored, ids, "stored in ingest order");
    for (i, m) in page.messages.iter().enumerate() {
        assert!(m.body.starts_with(&format!("{i:04} ")), "row {i} in order");
    }
    let mut at = 0;
    for b in &batches {
        let run = &page.messages[at..at + b.len()];
        assert!(
            run.windows(2).all(|w| w[1].local_seq == w[0].local_seq + 1),
            "one batch draws a contiguous local_seq run"
        );
        at += b.len();
    }
    assert!(page
        .messages
        .windows(2)
        .all(|w| w[1].local_seq > w[0].local_seq));
    assert_eq!(
        page.conversation.last_seq,
        page.messages[ROWS - 1].local_seq
    );

    // Coalesced: one event per call for the unpinned channel, never 1,000.
    a.foreign_conversations(None).await.unwrap(); // barrier: earlier ticks emitted
    let mut foreign = 0;
    while let Ok(e) = events.try_recv() {
        if let FabricEvent::Foreign {
            conv: c,
            id,
            change,
        } = e
        {
            assert_eq!((c, id, change), (conv, None, ForeignChange::New));
            foreign += 1;
        }
    }
    assert_eq!(foreign, batches.len(), "one coalesced event per call");

    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}
