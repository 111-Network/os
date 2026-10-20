//! §7.2 (2): a contact added by `node1…` only is stored — it survives
//! restarts (clean or not) until its card arrives or the user forgets it.

mod common;

use common::*;
use mesh111::contacts::card_text;
use mesh111::fabric::FabricSettings;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn node_id_contacts_survive_restarts_until_forgotten() {
    let d = Dirs::new();
    let ai = Ident::new("alice", 0);
    let dave = Ident::new("dave", 0);
    let erin = Ident::new("erin", 0);
    let dir = d.node("a");
    let a = start(&ai, &dir, FabricSettings::default());
    for who in [&dave, &erin] {
        let bech = id111::encoding::bech32_encode(who.id().as_bytes());
        let c = a.add_contact_node_id(&bech).await.unwrap();
        assert!(!c.has_key);
    }

    // A clean restart: both are still listed, key-less.
    a.shutdown().await.unwrap();
    let a = start(&ai, &dir, FabricSettings::default());
    for who in [&dave, &erin] {
        let c = a.contact(who.id()).await.expect("still a contact");
        assert!(!c.has_key);
    }

    // Forgotten, then a crash: Erin stays forgotten, Dave stays.
    a.forget_contact(erin.id()).await.unwrap();
    a.abort();
    let a = start(&ai, &dir, FabricSettings::default());
    assert!(a.contact(erin.id()).await.is_err());
    assert!(!a.contact(dave.id()).await.unwrap().has_key);

    // Dave's card is scanned: one contact, with a key, across a restart.
    a.add_contact_card(&card_text(&dave.announce_bytes()))
        .await
        .unwrap();
    a.shutdown().await.unwrap();
    let a = start(&ai, &dir, FabricSettings::default());
    let all = a.contacts().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].node_id, dave.id());
    assert!(all[0].has_key);
}
