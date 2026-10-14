//! §5 / §6.1 over the test-only `lossy-loopback` link at a forced 160 B
//! profile (§20 G1): Tier-1 fragments with loss and reorder (SACK
//! retransmit, ≤ 16 outstanding), a single loss inferred from later SACKs
//! and resent with no event at all, recovery after the receiver evicts more
//! than 16 partials, duplicates re-ACKed and never redelivered, the bounded
//! range list (deferral) with every retransmitted packet acknowledged, and
//! `restore_pending`.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::*;
use mesh111::fabric::{Fabric, FabricSettings};
use mesh111::frag::plan;
use mesh111::ids::MsgSeq;
use mesh111::link::lossy::{DropFilter, LossyConfig, LossyEnd};
use mesh111::link::{LinkKind, PathProfile};
use mesh111::outbox::{MessageBody, Reason};
use mesh111::store::MsgState;

async fn pair_nodes(
    d: &Dirs,
    cfg_a: LossyConfig,
    cfg_b: LossyConfig,
) -> (
    Arc<Ident>,
    Fabric,
    Arc<Ident>,
    Fabric,
    Arc<LossyEnd>,
    Arc<LossyEnd>,
) {
    let ai = Ident::new("alice", 0);
    let bi = Ident::new("bob", 0);
    let a = start(&ai, &d.node("a"), FabricSettings::default());
    let b = start(&bi, &d.node("b"), FabricSettings::default());
    exchange_cards(&a, &b).await;
    let (ea, eb) = lossy_pair((&a, &ai), (&b, &bi), cfg_a, cfg_b).await;
    (ai, a, bi, b, ea, eb)
}

/// Drive until every id is delivered. Retries happen only on events: when
/// nothing moved for a while the harness plays the link event (held frames
/// released, `retry_all`), as the daemon does on fabric events. `retry_all`
/// presumes everything lost but never counts an inferred loss
/// (`Counters::inferred_lost`), so the tests can tell the two apart.
async fn drive(a: &Fabric, ends: &[&Arc<LossyEnd>], ids: &[i64], secs: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    let mut last = usize::MAX;
    let mut idle = 0;
    loop {
        let mut done = 0;
        for id in ids {
            if a.message(*id).await.unwrap().state == MsgState::Delivered {
                done += 1;
            }
        }
        if done == ids.len() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{done}/{} delivered",
            ids.len()
        );
        if done == last {
            idle += 1;
        } else {
            idle = 0;
            last = done;
        }
        if idle >= 3 {
            for e in ends {
                e.flush_held();
            }
            a.retry_all().await.unwrap();
            idle = 0;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

fn lossy(loss: u16, reorder: u16, seed: u64) -> LossyConfig {
    LossyConfig {
        loss_permille: loss,
        reorder_permille: reorder,
        seed,
        ..LossyConfig::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tier1_over_lossy_loopback_with_loss_and_reorder() {
    let d = Dirs::new();
    let (_ai, a, bi, b, ea, eb) = pair_nodes(&d, lossy(150, 150, 7), lossy(150, 150, 11)).await;
    let texts: Vec<String> = (0..4)
        .map(|i| format!("{i}:{}", "x".repeat(880 + i)))
        .collect();
    // 160 B profile: ~70 B per fragment ⇒ 13 fragments each (Tier 1).
    let body_len = MessageBody::text(texts[0].clone()).encode().len();
    assert_eq!(plan(body_len, 160, false, MsgSeq::new(2, 1)), Ok(13));
    let mut ids = Vec::new();
    for t in &texts {
        ids.push(a.send(bi.id(), t, None).await.unwrap());
    }
    drive(&a, &[&ea, &eb], &ids, 90).await;
    let mut got: Vec<String> = inbox(&b, a.node_id())
        .await
        .into_iter()
        .filter_map(|m| m.text)
        .collect();
    got.sort();
    assert_eq!(got, texts, "each delivered once, intact");
    let st = a.status().await.unwrap();
    assert!(
        (1..=16).contains(&st.counters.max_outstanding),
        "≤ 16 outstanding fragments per peer: {}",
        st.counters.max_outstanding
    );
    assert!(st.counters.retransmits > 0, "retransmissions happened");
    // Losses were inferred from SACK evidence, which the harness's
    // `retry_all` never counts: a broken `infer_losses` cannot hide behind
    // the hammer. (The next test pins an inferred loss resent with no event
    // at all.)
    assert!(
        st.counters.inferred_lost > 0,
        "SACK-inferred losses: {:?}",
        st.counters
    );
    assert!(ea.stats().dropped > 0 && ea.stats().reordered > 0);
    assert!(eb.stats().sent > 0, "ACKs crossed the same lossy path");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lost_fragment_is_inferred_from_later_sacks_and_resent_without_events() {
    let d = Dirs::new();
    // The third frame Alice writes (fragment 2 of 13) is lost once; nothing
    // else is. Bob's SACKs of the later fragments are the only evidence.
    let sent = Arc::new(AtomicUsize::new(0));
    let seen = sent.clone();
    let lose_third: DropFilter = Arc::new(move |_| seen.fetch_add(1, Ordering::SeqCst) == 2);
    let cfg_a = LossyConfig {
        filter: Some(lose_third),
        ..LossyConfig::default()
    };
    let (_ai, a, bi, b, _ea, _eb) = pair_nodes(&d, cfg_a, LossyConfig::default()).await;
    let text = format!("x:{}", "x".repeat(880));
    let id = a.send(bi.id(), &text, None).await.unwrap();
    // No harness event at all (no `retry`, no `retry_all`, no link change):
    // only the SACK evidence can get the lost fragment resent.
    until_state(&a, id, MsgState::Delivered, 30).await;
    let c = a.status().await.unwrap().counters;
    assert_eq!(
        (c.inferred_lost, c.retransmits),
        (1, 1),
        "one loss inferred from SACK evidence, one fragment resent"
    );
    assert_eq!(sent.load(Ordering::SeqCst), 14, "13 fragments + 1 resend");
    let got = inbox(&b, a.node_id()).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].text.as_deref(), Some(text.as_str()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn more_than_16_partials_evicted_and_recovered() {
    let d = Dirs::new();
    // A 97-char text is a 101 B CBOR body: two fragments, 51 + 50 B. With a
    // 2-byte msg_seq the sealed fragments are 141 and 140 B; drop every 140.
    let text = |i: usize| format!("{i:02}{}", "y".repeat(95));
    assert_eq!(MessageBody::text(text(0)).encode().len(), 101);
    let f1_len = 64 + 4 + 1 + 1 + 2 + 2 + 50 + 16;
    let drop_f1: DropFilter = Arc::new(move |f| f.bytes.len() == f1_len);
    let cfg_a = LossyConfig {
        filter: Some(drop_f1),
        ..LossyConfig::default()
    };
    let (ai, a, bi, b, ea, eb) = pair_nodes(&d, cfg_a, LossyConfig::default()).await;
    let mut ids = Vec::new();
    for i in 0..20 {
        ids.push(a.send(bi.id(), &text(i), None).await.unwrap());
    }
    // Fresh fragments first: every first fragment reaches Bob, every second
    // is lost, and Bob's store evicts the four oldest of 20 partials.
    let a_id = ai.id();
    until(20, || async {
        let ns: Vec<u32> = b
            .with_store(move |s| s.rx_snapshot(a_id.as_bytes(), 0))
            .await
            .ok()?
            .ok()?
            .first()?
            .partials
            .iter()
            .map(|p| p.n)
            .collect();
        (ns == (5..=20).collect::<Vec<u32>>()).then_some(())
    })
    .await;
    assert!(inbox(&b, ai.id()).await.is_empty());

    // The path heals. Bob's ACKs stop claiming the evicted bits; Alice
    // regresses them and resends the first fragments.
    ea.set_filter(None);
    drive(&a, &[&ea, &eb], &ids, 60).await;
    let mut got: Vec<String> = inbox(&b, ai.id())
        .await
        .into_iter()
        .filter_map(|m| m.text)
        .collect();
    got.sort();
    assert_eq!(got, (0..20).map(text).collect::<Vec<_>>());
    let st = a.status().await.unwrap();
    assert!(
        st.counters.regressed >= 4,
        "evicted partials regressed: {}",
        st.counters.regressed
    );
    assert!(st.counters.max_outstanding <= 16);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn duplicates_are_reacked_never_redelivered() {
    let d = Dirs::new();
    let drop_all: DropFilter = Arc::new(|_| true);
    let cfg_b = LossyConfig {
        filter: Some(drop_all),
        ..LossyConfig::default()
    };
    let (ai, a, bi, b, _ea, eb) = pair_nodes(&d, LossyConfig::default(), cfg_b).await;
    let id = a.send(bi.id(), "say it twice", None).await.unwrap();
    until(10, || async {
        (inbox(&b, ai.id()).await.len() == 1).then_some(())
    })
    .await;
    // Bob's ACKs are lost: Alice's copy stays `sent`.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(a.message(id).await.unwrap().state, MsgState::Sent);
    for _ in 0..3 {
        a.retry(id).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(inbox(&b, ai.id()).await.len(), 1, "never redelivered");
    eb.set_filter(None);
    a.retry(id).await.unwrap();
    until_state(&a, id, MsgState::Delivered, 10).await;
    assert_eq!(inbox(&b, ai.id()).await.len(), 1);
    // `retry` is refused once delivered.
    assert!(a.retry(id).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bounded_ranges_defer_and_retransmits_are_always_acknowledged() {
    let d = Dirs::new();
    // Short (30-char) and long (31-char) texts seal to 122 / 123 B frames.
    let short = |i: usize| format!("{i:02}{}", "s".repeat(28));
    let long = |i: usize| format!("{i:02}{}", "l".repeat(29));
    let drop_long: DropFilter = Arc::new(|f| f.bytes.len() == 123);
    let drop_all: DropFilter = Arc::new(|_| true);
    let cfg_a = LossyConfig {
        filter: Some(drop_long),
        ..LossyConfig::default()
    };
    let cfg_b = LossyConfig {
        filter: Some(drop_all),
        ..LossyConfig::default()
    };
    let (ai, a, bi, b, ea, eb) = pair_nodes(&d, cfg_a, cfg_b).await;
    let mut ids = Vec::new();
    for n in 1..=19usize {
        let t = if n % 2 == 1 { short(n) } else { long(n) };
        ids.push(a.send(bi.id(), &t, None).await.unwrap());
    }
    // ≤ 16 outstanding: n = 1..16 go out, the odd ones arrive (no ACK
    // evidence comes back, so the window stays full until an event).
    until(10, || async {
        (inbox(&b, ai.id()).await.len() == 8).then_some(())
    })
    .await;
    assert_eq!(a.status().await.unwrap().counters.max_outstanding, 16);
    // A fabric event: fresh packets first (n = 17, 18, 19), then resends.
    a.retry_all().await.unwrap();
    // Bob holds n = 1 (prefix) and 3, 5, …, 17 (8 isolated ranges); n = 19
    // would open a ninth and is deferred: not recorded, not delivered.
    until(10, || async {
        (inbox(&b, ai.id()).await.len() == 9).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(inbox(&b, ai.id()).await.len(), 9);

    // ACKs flow again; Alice retransmits. Every duplicate is acknowledged
    // (its range travels first), the deferred one is not.
    eb.set_filter(None);
    a.retry_all().await.unwrap();
    for id in ids.iter().take(17).step_by(2) {
        until_state(&a, *id, MsgState::Delivered, 10).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_ne!(a.message(ids[18]).await.unwrap().state, MsgState::Delivered);
    assert_ne!(a.message(ids[1]).await.unwrap().state, MsgState::Delivered);

    // The gaps fill; everything is delivered exactly once.
    ea.set_filter(None);
    drive(&a, &[&ea, &eb], &ids, 30).await;
    let got = inbox(&b, ai.id()).await;
    assert_eq!(got.len(), 19);
    let mut ns: Vec<u32> = got.iter().map(|m| m.seq.n).collect();
    ns.sort();
    assert_eq!(ns, (1..=19).collect::<Vec<u32>>());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restore_pending_seals_nothing_until_it_clears() {
    let d = Dirs::new();
    let ai = Ident::new("alice", 0);
    let bi = Ident::new("bob", 0);
    let a = start(&ai, &d.node("a"), FabricSettings::default());
    let b = start(&bi, &d.node("b"), FabricSettings::default());
    exchange_cards(&a, &b).await;
    let id = a.send(bi.id(), "after the restore", None).await.unwrap();
    // A crash, and the identity holder now reports a restore floor.
    a.abort();
    ai.set_restore_pending(true);
    let a = start(&ai, &d.node("a"), FabricSettings::default());
    let st = a.status().await.unwrap();
    assert!(st.store.restore_pending && st.store.epoch.is_none());
    let m = until(10, || async {
        let m = a.message(id).await.ok()?;
        (m.reason == Some(Reason::RestorePending)).then_some(m)
    })
    .await;
    assert_eq!(m.state, MsgState::Queued);
    assert!(matches!(
        a.send(bi.id(), "refused", None).await,
        Err(mesh111::fabric::FabricError::RestorePending)
    ));
    let (_ea, _eb) = lossy_pair(
        (&a, &ai),
        (&b, &bi),
        LossyConfig::default(),
        LossyConfig::default(),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(a.message(id).await.unwrap().state, MsgState::Queued);
    assert!(inbox(&b, ai.id()).await.is_empty(), "nothing was sealed");
    // The floor is set: the next fabric event mints and sends.
    ai.set_restore_pending(false);
    a.retry_all().await.unwrap();
    until_state(&a, id, MsgState::Delivered, 10).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn intro_is_honoured_only_over_tcp_lan() {
    let d = Dirs::new();
    let ai = Ident::new("alice", 0);
    let bi = Ident::new("bob", 0);
    let a = start(&ai, &d.node("a"), FabricSettings::default());
    let b = start(&bi, &d.node("b"), FabricSettings::default());
    // Alice holds Bob's card; Bob has never heard of Alice.
    a.add_contact_card(&b.card().await.unwrap().text)
        .await
        .unwrap();
    // Alice's end looks like tcp-lan (so she rides INTRO); Bob's end is a
    // LoRa-class bearer, where v1 never honours INTRO (§7.2 (3)).
    let cfg_a = LossyConfig {
        kind: LinkKind::TcpLan,
        profile: PathProfile::tcp_lan(),
        ..LossyConfig::default()
    };
    let (_ea, _eb) = lossy_pair((&a, &ai), (&b, &bi), cfg_a, LossyConfig::default()).await;
    let id = a.send(bi.id(), "hello?", None).await.unwrap();
    until(10, || async {
        let s = b.status().await.ok()?;
        (s.counters.dropped >= 1).then_some(())
    })
    .await;
    assert!(b.contact(ai.id()).await.is_err(), "no request filed");
    assert!(
        bi.forgotten().contains(&ai.enc()),
        "provisional key dropped"
    );
    assert_eq!(a.message(id).await.unwrap().state, MsgState::Sent);
}
