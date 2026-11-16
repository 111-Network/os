//! The Layer 7 consents (§12, §6.2 consent copy, §4.6 step 6; §13.1, §13.2; I2): the texts and
//! the hash rule in one place.
//!
//! Every Layer 7 consent is the pinned round trip: `409 consent_required` whose `consent` member is
//! the contract's `ConsentRequired {action, subject, warning_text, hash, addons}`; the client shows
//! `warning_text` verbatim and repeats the call with `accept_warning` = `hash` (on `POST
//! /proposals/{id}/decision` too). The four actions are `carrier_enable`, `pair_plain`,
//! `gateway_bit` and `origin_serve`.
//!
//! **The hash rule.** `hash` is the SHA-256 of the exact UTF-8 bytes of `warning_text`, as 64
//! lowercase hex characters (the contract's `Sha256Hex`); [`digest`] is the one function and
//! [`hash`] its hex form. The hash of the text the human accepted is recorded (`state/relay.json`
//! for `carrier_enable`, `gateway_bit` and `origin_serve`, row D1, as hex; the pair's row in
//! `fabric/settings` for `pair_plain`, row D6, as the 32 bytes of [`digest`]), so a changed text
//! has another hash and asks again.
//!
//! **The texts.** [`GATEWAY_BIT_TEXT`] and [`ORIGIN_SERVE_TEXT`] are §12's copies verbatim;
//! [`carrier_enable_text`] is §6.2's copy with its two placeholders filled (`<far hub>` and
//! `<server>`) and then one cap sentence per metered link, naming its match and its cap, in a
//! stable order (interfaces first, then Wi-Fi networks, each by name): §12 asks for the cap in the
//! text ("setting or raising `daily_cap_bytes` while the carrier is on re-asks `carrier_enable`
//! with the cap in `warning_text` and `metered` in `addons`"), and naming every link makes any
//! cap set or raised, on any link, another text with another hash; [`PAIR_PLAIN_WARNING`] is
//! §4.6 step 6's verbatim, with its consent [`pair_plain`] (defined here since row D6d, the one
//! place for every Layer 7 text; `relay::pair` re-exports the constant and asks it on the scan).
//!
//! **Current, not merely recorded.** [`holds`] is the one test of a recorded consent: its hash
//! is the hash of the text owed now. Role bits 3 and 5 (row D9) read it, so a consent recorded
//! under a text an update has since changed stops the bit until the human consents again.
//!
//! **`<far hub>`.** The carrier's consent is one per carrier (its subject is `<kind>:<server>`,
//! §12, never a hub): one consent covers every pair that reaches its far hub through it, so the
//! placeholder is filled with [`CARRIER_FAR_HUBS`], never with one pair's name. A text that
//! named the pairs would change with every pairing and unpairing, and the recorded hash, which
//! the `gateway` row (row D3) and role bit 3 (row D9) read, would go stale under them.
//! With this filling the recorded hash is the hash of the text owed now whenever it is set (the
//! settings route keeps it so, row D10).

use crate::daemon::dashboard::api_v1::Problem;
use crate::daemon::fabric::json;
use crate::daemon::relay::settings::{
    CarrierKind, ConsentKind, MeteredLink, MeteredMatch, RelaySettings,
};
use axum::http::StatusCode;
use id111::keyset::NodeId;
use sha2::{Digest, Sha256};

pub use crate::daemon::components::consent::{ConsentRequired, ProblemWith};

/// `ConsentRequired.action` of the carrier's switch (ntfy on, or its metered cap set or raised).
pub const ACTION_CARRIER_ENABLE: &str = "carrier_enable";
/// `ConsentRequired.action` of the pairing warning (§4.6 step 6; row D6 answers it).
pub const ACTION_PAIR_PLAIN: &str = "pair_plain";
/// `ConsentRequired.action` of the public gateway role bit (§7.2, §12).
pub const ACTION_GATEWAY_BIT: &str = "gateway_bit";
/// `ConsentRequired.action` of serving update files to the LAN (§9.1, §12).
pub const ACTION_ORIGIN_SERVE: &str = "origin_serve";

/// The `addons` entry of a `carrier_enable` text that carries a metered cap (§12).
pub const ADDON_METERED: &str = "metered";

/// The `subject` of the `gateway_bit` consent (the setting it guards).
pub const SUBJECT_GATEWAY_BIT: &str = "discovery.public_gateway_bit";
/// The `subject` of the `origin_serve` consent (the setting it guards).
pub const SUBJECT_ORIGIN_SERVE: &str = "origin.serve";

/// §6.2's `<far hub>`, filled for the carrier as a whole (module doc).
pub const CARRIER_FAR_HUBS: &str = "the hubs it is paired with";

/// §12, the public gateway bit, verbatim.
pub const GATEWAY_BIT_TEXT: &str = "This adds 'willing to act as a gateway' to this node's signed \
card. Everyone who receives the card, on any network, learns that this node has internet. Your \
hub's members learn it without this.";

/// §12, origin serving, verbatim.
pub const ORIGIN_SERVE_TEXT: &str = "Any device on this network can download this hub's verified \
update files without signing in. They are public files signed by the 111 release key; nothing \
about you is in them.";

/// §4.6 step 6 (D30 Q21), the pairing warning, verbatim: asked by the scan (row D6) when a pair
/// would run plain over an address that is not private (`relay::pair` re-exports it).
pub const PAIR_PLAIN_WARNING: &str = "This hub is on the open internet and this link cannot be \
encrypted. Anyone on the path between the two hubs can see which hubs talk, when, and how much. \
Message contents stay sealed. Pair anyway?";

/// SHA-256 of the exact UTF-8 bytes of `text`, the hash rule (module doc): the 32 bytes a pair's
/// row keeps for `pair_plain` (`PairLink::warning_hash`, §4.6 step 6).
pub fn digest(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

/// [`digest`] as 64 lowercase hex characters (the contract's `Sha256Hex`, the form
/// `accept_warning` echoes and `state/relay.json` records).
pub fn hash(text: &str) -> String {
    digest(text).iter().map(|b| format!("{b:02x}")).collect()
}

/// §6.2's copy with `<far hub>` = [`CARRIER_FAR_HUBS`] and `<server>` = `server` (the URL as
/// configured), then one cap sentence of §12 per link of `metered`, in the order of [`by_match`]
/// (module doc).
pub fn carrier_enable_text(server: &str, metered: &[MeteredLink]) -> String {
    let mut t = format!(
        "To carry messages between this hub and {CARRIER_FAR_HUBS}, 111 will post sealed packets \
         to the ntfy server at {server} and read packets back from it. The server's operator can \
         see this hub's internet address, the topic names and a short tag of the two hubs' ids, \
         how many packets are posted, their sizes and when. The operator cannot read them or tell \
         which person they are for. Packets stay only as long as the server keeps them (12 hours \
         on a default server); a lost packet is sent again. Nothing is sent while this switch is \
         off."
    );
    for l in by_match(metered) {
        let place = match &l.matcher {
            MeteredMatch::Iface(n) => format!("the network interface {n}"),
            MeteredMatch::Ssid(n) => format!("the Wi-Fi network \"{n}\""),
        };
        t.push_str(&format!(
            " A daily cap of {} bytes applies on {place}, which you marked metered.",
            l.daily_cap_bytes
        ));
    }
    t
}

/// `metered` in the text's stable order: interfaces first, then Wi-Fi networks, each by name
/// (the order the links were written in never changes the text).
pub fn by_match(metered: &[MeteredLink]) -> Vec<&MeteredLink> {
    let mut v: Vec<&MeteredLink> = metered.iter().collect();
    v.sort_by(|a, b| {
        let key = |l: &MeteredLink| match &l.matcher {
            MeteredMatch::Iface(n) => (0u8, n.clone()),
            MeteredMatch::Ssid(n) => (1u8, n.clone()),
        };
        key(a).cmp(&key(b))
    });
    v
}

/// The `carrier_enable` consent for the carrier `kind` at `server` (subject `<kind>:<server>`,
/// §12), with `metered` in `addons` when the text carries a cap.
pub fn carrier_enable(kind: &str, server: &str, metered: &[MeteredLink]) -> ConsentRequired {
    let text = carrier_enable_text(server, metered);
    ConsentRequired {
        action: ACTION_CARRIER_ENABLE,
        subject: format!("{kind}:{server}"),
        hash: hash(&text),
        warning_text: text,
        addons: if metered.is_empty() {
            Vec::new()
        } else {
            vec![ADDON_METERED.to_string()]
        },
    }
}

/// The `carrier_enable` consent the carrier `s` holds owes now (§6.2, §12): `None` while no
/// carrier is configured (`carrier.kind` `none`, or no server).
pub fn carrier_enable_for(s: &RelaySettings) -> Option<ConsentRequired> {
    match s.carrier_kind() {
        CarrierKind::None => None,
        CarrierKind::Ntfy => s
            .ntfy_server()
            .map(|server| carrier_enable("ntfy", server, s.metered())),
    }
}

/// The consent `kind` owed now for the settings `s` (`None`: a carrier consent with no carrier
/// configured).
pub fn owed(s: &RelaySettings, kind: ConsentKind) -> Option<ConsentRequired> {
    match kind {
        ConsentKind::GatewayBit => Some(gateway_bit()),
        ConsentKind::OriginServe => Some(origin_serve()),
        ConsentKind::CarrierEnable => carrier_enable_for(s),
    }
}

/// `s` holds the consent `kind`: a hash is recorded and it is the hash of the text owed now
/// (module doc). A consent recorded under another text (a changed copy, another server, another
/// cap) does not hold.
pub fn holds(s: &RelaySettings, kind: ConsentKind) -> bool {
    owed(s, kind).is_some_and(|c| s.consent_hash(kind) == Some(c.hash.as_str()))
}

/// The `gateway_bit` consent (§12).
pub fn gateway_bit() -> ConsentRequired {
    ConsentRequired {
        action: ACTION_GATEWAY_BIT,
        subject: SUBJECT_GATEWAY_BIT.to_string(),
        warning_text: GATEWAY_BIT_TEXT.to_string(),
        hash: hash(GATEWAY_BIT_TEXT),
        addons: Vec::new(),
    }
}

/// The `origin_serve` consent (§12).
pub fn origin_serve() -> ConsentRequired {
    ConsentRequired {
        action: ACTION_ORIGIN_SERVE,
        subject: SUBJECT_ORIGIN_SERVE.to_string(),
        warning_text: ORIGIN_SERVE_TEXT.to_string(),
        hash: hash(ORIGIN_SERVE_TEXT),
        addons: Vec::new(),
    }
}

/// The `pair_plain` consent (§4.6 step 6, §12): [`PAIR_PLAIN_WARNING`], subject the far hub's
/// NodeId as 64 lowercase hex.
pub fn pair_plain(far: &NodeId) -> ConsentRequired {
    ConsentRequired {
        action: ACTION_PAIR_PLAIN,
        subject: json::hex(far),
        warning_text: PAIR_PLAIN_WARNING.to_string(),
        hash: hash(PAIR_PLAIN_WARNING),
        addons: Vec::new(),
    }
}

/// `409 consent_required` with `c` as the `consent` member (§12, I2): nothing was written for
/// the switch it guards.
pub fn problem(c: &ConsentRequired) -> ProblemWith {
    ProblemWith::new(
        Problem::new(
            StatusCode::CONFLICT,
            "consent_required",
            "Read this first, then confirm.",
        )
        .detail(
            "Show warning_text as it is, then repeat the request with accept_warning = its hash.",
        ),
    )
    .with("consent", serde_json::to_value(c).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(n: &str, cap: u64) -> MeteredLink {
        MeteredLink {
            matcher: MeteredMatch::Iface(n.to_string()),
            daily_cap_bytes: cap,
        }
    }

    fn ssid(n: &str, cap: u64) -> MeteredLink {
        MeteredLink {
            matcher: MeteredMatch::Ssid(n.to_string()),
            daily_cap_bytes: cap,
        }
    }

    #[test]
    fn a_consent_holds_only_under_the_text_owed_now() {
        let mut s = RelaySettings::default();
        for k in [ConsentKind::GatewayBit, ConsentKind::OriginServe] {
            assert!(!holds(&s, k), "nothing recorded");
            s.set_consent_hash(k, Some(&"ab".repeat(32))).unwrap();
            assert!(!holds(&s, k), "a hash of another text (an older copy)");
        }
        s.set_consent_hash(ConsentKind::GatewayBit, Some(&hash(GATEWAY_BIT_TEXT)))
            .unwrap();
        s.set_consent_hash(ConsentKind::OriginServe, Some(&hash(ORIGIN_SERVE_TEXT)))
            .unwrap();
        assert!(holds(&s, ConsentKind::GatewayBit) && holds(&s, ConsentKind::OriginServe));
        // the carrier: none configured holds nothing; its text follows the server and the caps
        assert!(!holds(&s, ConsentKind::CarrierEnable));
        s.set_carrier_kind(CarrierKind::Ntfy);
        s.set_ntfy_server(Some("https://n.example")).unwrap();
        let h = carrier_enable("ntfy", "https://n.example", &[]).hash;
        s.set_consent_hash(ConsentKind::CarrierEnable, Some(&h))
            .unwrap();
        assert!(holds(&s, ConsentKind::CarrierEnable));
        s.set_metered(vec![iface("wwan0", 1_000)]).unwrap();
        assert!(!holds(&s, ConsentKind::CarrierEnable));
    }

    #[test]
    fn the_hash_is_sha256_hex_of_the_exact_text() {
        // FIPS 180-2 test vector
        assert_eq!(
            hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let h = hash(GATEWAY_BIT_TEXT);
        assert_eq!(h.len(), 64);
        assert!(h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        // one character more is another hash: a changed text asks again
        assert_ne!(hash(&format!("{GATEWAY_BIT_TEXT} ")), h);
        // the same rule as Layer 6's consents
        assert_eq!(
            h,
            crate::daemon::bridge::settings::sha256_hex(GATEWAY_BIT_TEXT)
        );
        // row D6d: one rule for the pairing warning too; the pair row keeps the 32 bytes whose
        // hex the route asks for (the digest of the verbatim text, pinned: sha256sum of it)
        let warned = "e32303b0492bd0208597558793644763413feb2fe37302ec0b05ff5769e6a80e";
        assert_eq!(hash(PAIR_PLAIN_WARNING), warned);
        let bytes = digest(PAIR_PLAIN_WARNING);
        assert_eq!(
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            warned
        );
        assert_eq!(crate::daemon::relay::pair::pair_plain_hash(), bytes);
        assert_eq!(crate::daemon::relay::pair::pair_plain_hash_hex(), warned);
    }

    #[test]
    fn the_texts_are_the_spec_copies() {
        assert_eq!(
            GATEWAY_BIT_TEXT,
            "This adds 'willing to act as a gateway' to this node's signed card. Everyone who \
             receives the card, on any network, learns that this node has internet. Your hub's \
             members learn it without this."
        );
        assert_eq!(
            ORIGIN_SERVE_TEXT,
            "Any device on this network can download this hub's verified update files without \
             signing in. They are public files signed by the 111 release key; nothing about you \
             is in them."
        );
        // §4.6 step 6 (D30 Q21)
        assert_eq!(
            PAIR_PLAIN_WARNING,
            "This hub is on the open internet and this link cannot be encrypted. Anyone on the \
             path between the two hubs can see which hubs talk, when, and how much. Message \
             contents stay sealed. Pair anyway?"
        );
        let t = carrier_enable_text("https://ntfy.example.org", &[]);
        assert_eq!(
            t,
            "To carry messages between this hub and the hubs it is paired with, 111 will post \
             sealed packets to the ntfy server at https://ntfy.example.org and read packets back \
             from it. The server's operator can see this hub's internet address, the topic names \
             and a short tag of the two hubs' ids, how many packets are posted, their sizes and \
             when. The operator cannot read them or tell which person they are for. Packets stay \
             only as long as the server keeps them (12 hours on a default server); a lost packet \
             is sent again. Nothing is sent while this switch is off."
        );
        assert!(!t.contains("  "), "no doubled space from the line breaks");
    }

    #[test]
    fn the_cap_is_in_the_text_and_metered_in_the_addons() {
        let plain = carrier_enable("ntfy", "https://n.example", &[]);
        assert_eq!(plain.action, "carrier_enable");
        assert_eq!(plain.subject, "ntfy:https://n.example");
        assert!(plain.addons.is_empty());
        assert_eq!(plain.hash, hash(&plain.warning_text));
        let capped = carrier_enable("ntfy", "https://n.example", &[iface("wwan0", 5_000_000)]);
        assert!(capped.warning_text.ends_with(
            " A daily cap of 5000000 bytes applies on the network interface wwan0, which you \
             marked metered."
        ));
        assert_eq!(capped.addons, vec!["metered".to_string()]);
        assert_ne!(capped.hash, plain.hash);
        assert_ne!(
            carrier_enable("ntfy", "https://n.example", &[iface("wwan0", 6_000_000)]).hash,
            capped.hash,
            "a raised cap is another text"
        );
        assert_ne!(
            carrier_enable("ntfy", "https://n.example", &[iface("wwan1", 5_000_000)]).hash,
            capped.hash,
            "another link is another text"
        );
        // every link is named, so a second cap at or under the largest is another text too
        let two = [iface("wwan0", 5_000_000), ssid("Phone", 1_000)];
        let both = carrier_enable("ntfy", "https://n.example", &two);
        assert_ne!(both.hash, capped.hash, "a second link is another text");
        assert!(both.warning_text.ends_with(
            " A daily cap of 5000000 bytes applies on the network interface wwan0, which you \
             marked metered. A daily cap of 1000 bytes applies on the Wi-Fi network \"Phone\", \
             which you marked metered."
        ));
        let raised = [iface("wwan0", 5_000_000), ssid("Phone", 2_000)];
        assert_ne!(
            carrier_enable("ntfy", "https://n.example", &raised).hash,
            both.hash,
            "a raised cap under the largest is another text"
        );
        // the order the links were written in never changes the text
        let swapped = [ssid("Phone", 1_000), iface("wwan0", 5_000_000)];
        assert_eq!(
            carrier_enable("ntfy", "https://n.example", &swapped).hash,
            both.hash
        );
        assert_ne!(
            carrier_enable("ntfy", "https://m.example", &[]).hash,
            plain.hash,
            "another server is another text"
        );
    }

    #[test]
    fn the_problem_is_the_pinned_round_trip() {
        let far = NodeId::from([9; 32]);
        for c in [
            gateway_bit(),
            origin_serve(),
            carrier_enable("ntfy", "https://n.example", &[]),
            pair_plain(&far),
        ] {
            let b = problem(&c).body();
            assert_eq!(b["status"], 409);
            assert_eq!(b["code"], "consent_required");
            assert_eq!(b["consent"]["action"], c.action);
            assert_eq!(b["consent"]["subject"], c.subject.as_str());
            assert_eq!(b["consent"]["warning_text"], c.warning_text.as_str());
            assert_eq!(b["consent"]["hash"], hash(&c.warning_text).as_str());
            assert!(b["consent"]["addons"].is_array());
        }
        assert_eq!(gateway_bit().subject, "discovery.public_gateway_bit");
        assert_eq!(origin_serve().subject, "origin.serve");
        // §4.6 step 6: the far hub's NodeId, the verbatim warning, no addons
        let c = pair_plain(&far);
        assert_eq!(c.action, "pair_plain");
        assert_eq!(c.subject, "09".repeat(32));
        assert_eq!(c.warning_text, PAIR_PLAIN_WARNING);
        assert_eq!(c.hash, hash(PAIR_PLAIN_WARNING));
        assert!(c.addons.is_empty());
    }
}
