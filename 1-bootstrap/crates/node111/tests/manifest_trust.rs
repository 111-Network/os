//! The Layer 1 trust amendments T1 (channel) and T2 (keychain walk) of
//! `os/7-relay/README.md` §9.5, as G12 asserts them: a validly signed `beta`
//! manifest under the stable name is refused; a generation-1 release passes only
//! through a valid chain from the compiled key; a broken `signature_by_previous` is
//! refused; the floor generation holds. Fixtures: `tests/fixtures/l7-L1/` (how they
//! were made: its README). Every manifest goes through `parse_verified`, the one
//! function the daemon's updater, the root helper and the installer call.

use node111::floor::Floor;
use node111::manifest::{ChannelManifest, STABLE_CHANNEL};

fn fixture(rel: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{rel}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_else(|e| panic!("fixture {rel}: {e}"))
}

fn text(rel: &str) -> String {
    String::from_utf8(fixture(rel)).unwrap()
}

/// The base64 key line of a `.pub` file.
fn key(rel: &str) -> String {
    text(rel).lines().last().unwrap().trim().to_string()
}

/// The compiled-in DEV key (`keys/manifest-pub.key`) is the fixtures' test key.
fn compiled() -> String {
    std::fs::read_to_string(format!(
        "{}/../../keys/manifest-pub.key",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
    .trim()
    .to_string()
}

/// `parse_verified` of `<dir>/channel-111-stable.toml{,.minisig}`.
fn verify(dir: &str, pubkey: &str, floor: &Floor) -> anyhow::Result<ChannelManifest> {
    ChannelManifest::parse_verified(
        &fixture(&format!("{dir}/channel-111-stable.toml")),
        &text(&format!("{dir}/channel-111-stable.toml.minisig")),
        pubkey,
        floor,
    )
}

fn floor_gen(generation: u32) -> Floor {
    let mut f = Floor::initial();
    f.highest_keychain_generation = generation;
    f
}

fn err(r: anyhow::Result<ChannelManifest>) -> String {
    format!("{:#}", r.expect_err("must be refused"))
}

#[test]
fn compiled_key_is_the_test_key() {
    assert_eq!(compiled(), key("minisign/test.pub"));
}

// ---------- every existing fixture still verifies ----------

#[test]
fn canonical_fixture_still_verifies_at_generation_zero() {
    let m = verify("manifest", &compiled(), &Floor::initial()).unwrap();
    assert_eq!(m.channel(), STABLE_CHANNEL);
    // Its generation-1 entry is a placeholder: the walk ends at the anchor, and the
    // verified keychain holds only what the walk reached.
    assert_eq!(m.trusted_generation(), Some(0));
    assert_eq!(m.keychain().len(), 1);
    assert_eq!(m.keychain()[0].pubkey, compiled());
    // The bare parse still lists both entries, unverified.
    let raw = ChannelManifest::parse(&text("manifest/channel-111-stable.toml")).unwrap();
    assert_eq!(raw.keychain().len(), 2);
    assert_eq!(raw.trusted_generation(), None);
}

#[test]
fn a_keychain_without_the_compiled_key_verifies_at_generation_zero() {
    // The updater, helper and installer tests list a placeholder root and sign with
    // the compiled key: the compiled key alone signs, as generation 0.
    let m = verify("l7-L1/unlisted-root", &compiled(), &Floor::initial()).unwrap();
    assert_eq!(m.trusted_generation(), Some(0));
    assert!(m.keychain().is_empty());
    assert!(err(verify("l7-L1/unlisted-root", &compiled(), &floor_gen(1))).contains("floor"));
}

// ---------- T1: the channel ----------

#[test]
fn t1_signed_beta_under_the_stable_name_is_refused() {
    // The signature, the chain and the floor are all fine: only the channel is not.
    let raw = ChannelManifest::parse(&text("l7-L1/beta/channel-111-stable.toml")).unwrap();
    assert_eq!(raw.channel(), "beta");
    let e = err(verify("l7-L1/beta", &compiled(), &Floor::initial()));
    assert!(e.contains("channel") && e.contains("beta"), "{e}");
}

// ---------- T2: the keychain walk ----------

#[test]
fn t2_generation_one_passes_through_a_valid_chain() {
    let m = verify("l7-L1/gen1", &compiled(), &Floor::initial()).unwrap();
    assert_eq!(m.trusted_generation(), Some(1));
    assert_eq!(m.keychain().len(), 2);
    assert_eq!(m.keychain()[1].pubkey, key("l7-L1/gen1.pub"));
    // A newer binary compiled with the generation-1 key anchors there.
    let m = verify("l7-L1/gen1", &key("l7-L1/gen1.pub"), &Floor::initial()).unwrap();
    assert_eq!(m.trusted_generation(), Some(1));
}

#[test]
fn t2_generation_one_without_a_chain_from_the_compiled_key_is_refused() {
    // An unrelated compiled key anchors nothing: generation 1 is never reached.
    let e = err(verify(
        "l7-L1/gen1",
        &key("minisign/other.pub"),
        &Floor::initial(),
    ));
    assert!(e.contains("generation 0"), "{e}");
}

#[test]
fn t2_the_manifest_must_verify_under_the_highest_generation_reached() {
    // A valid chain to generation 1, but the manifest is signed by generation 0 (a
    // rotated-out key).
    let e = err(verify(
        "l7-L1/gen1-root-signed",
        &compiled(),
        &Floor::initial(),
    ));
    assert!(e.contains("generation 1"), "{e}");
}

#[test]
fn t2_a_broken_signature_by_previous_is_refused() {
    // Generation 1's link is a valid minisig over its pubkey, made by the wrong
    // key: the chain ends at 0 and the generation-1 signature fails there.
    let e = err(verify("l7-L1/broken-link", &compiled(), &Floor::initial()));
    assert!(
        e.contains("generation 0") && e.contains("signature_by_previous"),
        "{e}"
    );
}

#[test]
fn t2_a_broken_link_ends_the_chain() {
    // 0 → 1 valid, 1 → 2 broken (signed by generation 0): signed by 1, it passes at
    // generation 1 and the verified keychain stops there; signed by 2, it is refused.
    let m = verify("l7-L1/gen2-broken", &compiled(), &Floor::initial()).unwrap();
    assert_eq!(m.trusted_generation(), Some(1));
    assert_eq!(m.keychain().len(), 2);
    let raw = ChannelManifest::parse(&text("l7-L1/gen2-broken/channel-111-stable.toml")).unwrap();
    assert_eq!(raw.keychain().len(), 3);
    let e = err(verify(
        "l7-L1/gen2-broken-gen2-signed",
        &compiled(),
        &Floor::initial(),
    ));
    assert!(e.contains("generation 1"), "{e}");
}

#[test]
fn t2_the_floor_generation_holds() {
    // Generation reached ≥ the floor's generation passes; below it is refused.
    assert!(verify("l7-L1/gen1", &compiled(), &floor_gen(1)).is_ok());
    assert!(err(verify("l7-L1/gen1", &compiled(), &floor_gen(2))).contains("floor"));
    // The canonical fixture lists generation 1 but reaches only 0: a floor at 1
    // refuses it (the floor no longer trusts the manifest's own maximum).
    assert!(err(verify("manifest", &compiled(), &floor_gen(1))).contains("floor"));
    // A manifest that lists generation 2 behind a broken link reaches only 1.
    assert!(verify("l7-L1/gen2-broken", &compiled(), &floor_gen(1)).is_ok());
    assert!(err(verify("l7-L1/gen2-broken", &compiled(), &floor_gen(2))).contains("floor"));
}

#[test]
fn t2_a_tampered_link_or_manifest_is_refused() {
    // Any byte change in the signed manifest (here: generation 1's link) fails the
    // manifest signature.
    let toml = text("l7-L1/gen1/channel-111-stable.toml");
    let sig = text("l7-L1/gen1/channel-111-stable.toml.minisig");
    let tampered = toml.replacen("trusted comment: l7-L1 fixture", "trusted comment: x", 1);
    assert_ne!(tampered, toml);
    assert!(ChannelManifest::parse_verified(
        tampered.as_bytes(),
        &sig,
        &compiled(),
        &Floor::initial()
    )
    .is_err());
    // A truncated or garbage signature never panics.
    for bad in ["", "garbage", &sig[..sig.len() / 2]] {
        assert!(ChannelManifest::parse_verified(
            toml.as_bytes(),
            bad,
            &compiled(),
            &Floor::initial()
        )
        .is_err());
    }
}

#[test]
fn ratcheting_the_floor_from_the_verified_keychain_never_bricks() {
    // The helper and the installer ratchet the floor to the verified keychain's
    // maximum; that is the generation reached, so the same manifest passes again.
    for dir in [
        "manifest",
        "l7-L1/gen1",
        "l7-L1/gen2-broken",
        "l7-L1/unlisted-root",
    ] {
        let m = verify(dir, &compiled(), &Floor::initial()).unwrap();
        let max = m.keychain().iter().map(|k| k.generation).max().unwrap_or(0);
        assert_eq!(Some(max), m.trusted_generation(), "{dir}");
        assert!(verify(dir, &compiled(), &floor_gen(max)).is_ok(), "{dir}");
    }
}
