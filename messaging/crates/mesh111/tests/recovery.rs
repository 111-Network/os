//! §4 / §5 after a crash: an own message that was numbered and stored but
//! whose outbox rows were never written is planned again (never abandoned)
//! — for the destination path's profile (§5: 160 B when unknown), so it can
//! leave on that path.

mod common;

use common::*;
use mesh111::contacts::card_text;
use mesh111::envelope::Kind;
use mesh111::fabric::FabricSettings;
use mesh111::frag::plan;
use mesh111::ids::MsgSeq;
use mesh111::link::lossy::LossyConfig;
use mesh111::outbox::MessageBody;
use mesh111::store::{Class, MsgState, NewMessage};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_orphan_is_planned_for_its_path_and_delivered() {
    let d = Dirs::new();
    let ai = Ident::new("alice", 0);
    let bi = Ident::new("bob", 0);
    let dir = d.node("a");
    let a = start(&ai, &dir, FabricSettings::default());
    a.add_contact_card(&card_text(&bi.announce_bytes()))
        .await
        .unwrap();

    // The crash window: `n` committed with the content, no outbox rows yet.
    let body = MessageBody::text("o".repeat(400)).encode();
    let len = body.len();
    let (me, peer) = (ai.id().to_bytes(), bi.id().to_bytes());
    let (id, seq) = a
        .with_store(move |s| {
            s.queue_own_message(&NewMessage {
                peer,
                sender: me,
                stream: 0,
                kind: Kind::Msg.byte(),
                class: Class::Human,
                body: &body,
                blob_refs: None,
                reason: None,
                pinned: false,
            })
        })
        .await
        .unwrap()
        .unwrap();
    a.abort();

    let a = start(&ai, &dir, FabricSettings::default());
    let frags = a
        .with_store(move |s| s.outbox_for(&peer))
        .await
        .unwrap()
        .unwrap();
    let n = u32::try_from(seq.n).unwrap();
    let want = plan(len, 160, false, MsgSeq::new(seq.epoch, n)).unwrap();
    assert!(want > 1);
    assert_eq!(frags.len(), usize::from(want));
    assert!(frags
        .iter()
        .all(|f| f.frag_count == want && f.message_id == id));

    // It leaves on a 160 B path and is delivered.
    let b = start(&bi, &d.node("b"), FabricSettings::default());
    b.add_contact_card(&a.card().await.unwrap().text)
        .await
        .unwrap();
    let (_ea, _eb) = lossy_pair(
        (&a, &ai),
        (&b, &bi),
        LossyConfig::default(),
        LossyConfig::default(),
    )
    .await;
    until_state(&a, id, MsgState::Delivered, 15).await;
    let got = inbox(&b, ai.id()).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].text.as_deref(), Some("o".repeat(400).as_str()));
}
