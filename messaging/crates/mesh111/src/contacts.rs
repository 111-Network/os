//! §7 — contacts, cards and announces (pure parts). Owner: F1.
//!
//! - **Cards (§7.2 (1))**: a card is the node's own genesis announce (stored
//!   bytes, never minted) as base32 text ([`card_text`]: RFC 4648 alphabet,
//!   upper case, no padding — ≈ 220 chars for a 137 B announce, and inside
//!   the QR alphanumeric set). The QR module bitmap is drawn by the UI from
//!   this text. Scanning or pasting a card adds the contact with
//!   `trust = oob` and works with zero connectivity.
//! - **`node1…` (§7.2 (2))**: a NodeId typed or scanned ([`parse_node_id`]:
//!   64 hex or bech32 `node1…`). The fabric then asks its hub for the card
//!   (frame 0x04) and pins the reply only after it verifies **against the
//!   expected NodeId**.
//! - **Stale rule (§7, A6)**: an announce with the same key material and a
//!   counter ≤ the stored one is accepted with no state change; only a key
//!   conflict or a bad signature drops ([`classify_announce`]).
//! - **Trust is monotone** (`tofu` < `oob` < `enrolled`): the store enforces
//!   it on every upsert.
//! - **A7 cache semantics**: the contacts table is authoritative; the
//!   fabric's per-`sender_ref` key lookup ([`KeyCache`]) is a cache rebuilt
//!   from it and dropped on forget (the handle's pair key is zeroized then).
//! - **`appdata` v1 (§7.3, A8)**: `[ver=1][roles u16 LE][reserved]` +
//!   optional 8 B truncated home-hub id — a locator, never an authenticator
//!   ([`AppData`]).

use std::collections::HashMap;

use id111::announce::Announce;
use id111::encoding::{bech32_decode, hex_decode};
use id111::keyset::NodeId;

use crate::envelope::{node_prefix, PeerKeys};
use crate::store::{ContactRow, Trust};

/// RFC 4648 base32 alphabet (upper case).
const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
/// Longest card text accepted (a 169 B announce is 271 chars; slack for
/// separators a person or scanner adds).
pub const MAX_CARD_TEXT: usize = 512;
/// Longest NodeId text accepted (64 hex or 63 bech32, plus whitespace).
pub const MAX_NODE_ID_TEXT: usize = 128;

/// Card / NodeId parsing failures (typed; nothing panics on input).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CardError {
    /// Text longer than the cap (checked before decoding).
    #[error("card text too long")]
    TooLong,
    /// A character outside base32 (after separators are dropped).
    #[error("card text is not base32")]
    NotBase32,
    /// The bytes are not a valid announce (length, version, low-order key,
    /// or signature).
    #[error("card is not a valid signed announce")]
    BadAnnounce,
    /// The card is this node's own.
    #[error("card is this node's own")]
    OwnCard,
    /// Neither 64 hex nor `node1…`.
    #[error("not a NodeId")]
    BadNodeId,
}

/// §7.2 (1): base32 text of announce bytes (upper case, unpadded).
pub fn card_text(announce: &[u8]) -> String {
    let mut out = String::with_capacity(announce.len().div_ceil(5) * 8);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for b in announce {
        acc = (acc << 8) | u32::from(*b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(B32[((acc >> bits) & 31) as usize]));
        }
        acc &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(char::from(B32[((acc << (5 - bits)) & 31) as usize]));
    }
    out
}

/// Decode card text: case-insensitive; whitespace, `-` and trailing `=`
/// are ignored; length capped before decoding; non-zero trailing bits are
/// refused (one text per card).
pub fn parse_card_text(text: &str) -> Result<Vec<u8>, CardError> {
    if text.len() > MAX_CARD_TEXT {
        return Err(CardError::TooLong);
    }
    let mut out = Vec::with_capacity(text.len() * 5 / 8);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for c in text.bytes() {
        if c.is_ascii_whitespace() || c == b'-' || c == b'=' {
            continue;
        }
        let up = c.to_ascii_uppercase();
        let v = B32
            .iter()
            .position(|x| *x == up)
            .ok_or(CardError::NotBase32)? as u32;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
        acc &= (1 << bits) - 1;
    }
    if bits >= 5 || acc != 0 {
        return Err(CardError::NotBase32);
    }
    Ok(out)
}

/// Parse and verify announce bytes (length, version, low-order key,
/// signature under the strict predicate). Pure.
pub fn verify_card(bytes: &[u8]) -> Result<Announce, CardError> {
    let a = Announce::parse(bytes).map_err(|_| CardError::BadAnnounce)?;
    a.verify().map_err(|_| CardError::BadAnnounce)?;
    Ok(a)
}

/// §7.2 (2): a NodeId as 64 lowercase/uppercase hex or bech32 `node1…`.
pub fn parse_node_id(text: &str) -> Result<NodeId, CardError> {
    if text.len() > MAX_NODE_ID_TEXT {
        return Err(CardError::BadNodeId);
    }
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("node1") {
        return bech32_decode(&lower)
            .map(NodeId::from)
            .map_err(|_| CardError::BadNodeId);
    }
    hex_decode(&lower)
        .map(NodeId::from)
        .map_err(|_| CardError::BadNodeId)
}

/// `appdata` role bit 0: mailbox (accepts custody).
pub const ROLE_MAILBOX: u16 = 1 << 0;
/// `appdata` role bit 1: always-on hub.
pub const ROLE_ALWAYS_ON_HUB: u16 = 1 << 1;
/// `appdata` role bit 2: LoRa-attached.
pub const ROLE_LORA_ATTACHED: u16 = 1 << 2;

/// §7.3 `appdata` v1 (L2 amendment A8). Anything that does not start with
/// `ver = 1` reads as "no roles, no hint".
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AppData {
    /// Role bits.
    pub roles: u16,
    /// Truncated home-hub id (locator only, never an authenticator).
    pub home_hub: Option<[u8; 8]>,
}

impl AppData {
    /// Parse announce `appdata`.
    pub fn parse(appdata: &[u8]) -> AppData {
        if appdata.len() < 4 || appdata[0] != 1 {
            return AppData::default();
        }
        let roles = u16::from_le_bytes([appdata[1], appdata[2]]);
        let home_hub = appdata.get(4..12).map(|h| {
            let mut a = [0u8; 8];
            a.copy_from_slice(h);
            a
        });
        AppData { roles, home_hub }
    }

    /// Role names for the contract (`Contact.roles`).
    pub fn role_names(roles: u16) -> Vec<&'static str> {
        let mut v = Vec::new();
        if roles & ROLE_MAILBOX != 0 {
            v.push("mailbox");
        }
        if roles & ROLE_ALWAYS_ON_HUB != 0 {
            v.push("always_on_hub");
        }
        if roles & ROLE_LORA_ATTACHED != 0 {
            v.push("lora_attached");
        }
        v
    }
}

/// A contact row for a verified announce (`announce` = its exact bytes).
/// Alias and flags are the caller's; the store assigns the seen sequences.
pub fn contact_row(ann: &Announce, announce: &[u8], trust: Trust) -> ContactRow {
    let app = AppData::parse(ann.appdata());
    ContactRow {
        node_id: ann.node_id().to_bytes(),
        enc_pubkey: *ann.enc_pubkey(),
        announce: announce.to_vec(),
        announce_counter: ann.counter(),
        trust,
        alias: None,
        roles: app.roles,
        home_hub: app.home_hub.map(|h| h.to_vec()),
        first_seen_seq: 0,
        last_seen_seq: 0,
        request: false,
        pinned: false,
    }
}

/// How a verified announce relates to what is pinned (§7 stale rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnounceVerdict {
    /// This node's own NodeId — never pinned (identity collision / loop).
    Own,
    /// Not a contact.
    Unknown,
    /// Same keys, higher counter: refresh the stored announce.
    Refresh,
    /// Same keys, counter ≤ stored: accepted, no state change.
    Stale,
    /// Different key material for a pinned NodeId: reject and surface.
    Conflict,
}

/// Classify `ann` against the pinned contact (if any).
pub fn classify_announce(
    own: &NodeId,
    pinned: Option<&ContactRow>,
    ann: &Announce,
) -> AnnounceVerdict {
    if ann.node_id() == *own {
        return AnnounceVerdict::Own;
    }
    match pinned {
        None => AnnounceVerdict::Unknown,
        Some(c) if c.enc_pubkey != *ann.enc_pubkey() => AnnounceVerdict::Conflict,
        Some(c) if ann.counter() > c.announce_counter => AnnounceVerdict::Refresh,
        Some(_) => AnnounceVerdict::Stale,
    }
}

/// The `sender_ref` → keys lookup `open_envelope` asks for: a cache of the
/// contacts table (A7), rebuilt on every contact change.
#[derive(Debug, Default, Clone)]
pub struct KeyCache {
    by_prefix: HashMap<[u8; 4], Vec<PeerKeys>>,
}

impl KeyCache {
    /// Rebuild from contact rows.
    pub fn rebuild(rows: &[ContactRow]) -> KeyCache {
        let mut by_prefix: HashMap<[u8; 4], Vec<PeerKeys>> = HashMap::new();
        for r in rows {
            let k = PeerKeys {
                node_id: NodeId::from(r.node_id),
                enc_pubkey: r.enc_pubkey,
            };
            by_prefix
                .entry(node_prefix(&k.node_id))
                .or_default()
                .push(k);
        }
        KeyCache { by_prefix }
    }

    /// Candidates whose NodeId starts with `prefix` (collisions ⇒ several).
    pub fn lookup(&self, prefix: &[u8; 4]) -> Vec<PeerKeys> {
        self.by_prefix.get(prefix).cloned().unwrap_or_default()
    }

    /// The keys pinned for `node`.
    pub fn get(&self, node: &NodeId) -> Option<PeerKeys> {
        self.by_prefix
            .get(&node_prefix(node))
            .and_then(|v| v.iter().find(|k| k.node_id == *node).copied())
    }

    /// Number of cached contacts.
    pub fn len(&self) -> usize {
        self.by_prefix.values().map(Vec::len).sum()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.by_prefix.is_empty()
    }
}

/// Prefix of a hub-offer bundle (§7.3: card, address and code in one QR).
pub const BUNDLE_PREFIX: &str = "111HUB:";

/// §7.3 offer bundle: the hub's card, its one-time code and its `tcp-lan`
/// address in one text (`111HUB:<card>:<code>:<address>`; the address goes
/// last because it may itself contain colons). Upper-case card and code keep
/// it inside the QR alphanumeric set for IPv4 addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubBundle {
    /// Card text ([`card_text`]).
    pub card: String,
    /// Canonical 8-symbol code.
    pub code: String,
    /// `host:port`.
    pub address: String,
}

impl HubBundle {
    /// The bundle text.
    pub fn text(&self) -> String {
        format!(
            "{BUNDLE_PREFIX}{}:{}:{}",
            self.card, self.code, self.address
        )
    }

    /// Parse a bundle text (case of the prefix ignored; bounded).
    pub fn parse(text: &str) -> Option<HubBundle> {
        if text.len() > MAX_CARD_TEXT + 128 {
            return None;
        }
        let t = text.trim();
        let head = t.get(..BUNDLE_PREFIX.len())?;
        if !head.eq_ignore_ascii_case(BUNDLE_PREFIX) {
            return None;
        }
        let rest = &t[BUNDLE_PREFIX.len()..];
        let mut it = rest.splitn(3, ':');
        let card = it.next()?.to_string();
        let code = it.next()?.to_string();
        let address = it.next()?.to_string();
        if card.is_empty() || code.is_empty() || address.is_empty() {
            return None;
        }
        Some(HubBundle {
            card,
            code,
            address,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use id111::keyset::Keyset;

    fn keyset(b: u8) -> Keyset {
        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(&crate::envelope::blake2b_256(&[b"c-sign", &[b]]));
        bytes[32..].copy_from_slice(&crate::envelope::blake2b_256(&[b"c-ecdh", &[b]]));
        Keyset::from_bytes(&bytes).unwrap()
    }

    #[test]
    fn base32_round_trip_and_rfc4648_vectors() {
        // RFC 4648 §10 (unpadded): "foobar" → MZXW6YTBOI
        assert_eq!(card_text(b"foobar"), "MZXW6YTBOI");
        assert_eq!(card_text(b"f"), "MY");
        assert_eq!(parse_card_text("mzxw6ytboi").unwrap(), b"foobar");
        assert_eq!(parse_card_text("MZXW-6YTB OI").unwrap(), b"foobar");
        assert_eq!(parse_card_text("MY======").unwrap(), b"f");
        assert_eq!(parse_card_text("M1"), Err(CardError::NotBase32));
        assert_eq!(parse_card_text("MZ"), Err(CardError::NotBase32)); // stray bits
        assert_eq!(
            parse_card_text(&"A".repeat(MAX_CARD_TEXT + 1)),
            Err(CardError::TooLong)
        );
        for len in 0..40usize {
            let b: Vec<u8> = (0..len as u8).collect();
            assert_eq!(parse_card_text(&card_text(&b)).unwrap(), b);
        }
    }

    #[test]
    fn cards_verify_and_classify() {
        let a = keyset(1);
        let ann = Announce::create(&a, 3, &[1, 1, 0, 0, 9, 9, 9, 9, 9, 9, 9, 9]).unwrap();
        let bytes = ann.encode();
        let text = card_text(&bytes);
        assert!(text.len() < 300);
        let parsed = verify_card(&parse_card_text(&text).unwrap()).unwrap();
        assert_eq!(parsed.node_id(), a.node_id());
        let mut bad = bytes.clone();
        bad[100] ^= 1;
        assert_eq!(verify_card(&bad).unwrap_err(), CardError::BadAnnounce);

        let row = contact_row(&parsed, &bytes, Trust::Oob);
        assert_eq!(row.roles, ROLE_MAILBOX);
        assert_eq!(row.home_hub, Some(vec![9; 8]));
        let own = keyset(2).node_id();
        assert_eq!(
            classify_announce(&own, None, &parsed),
            AnnounceVerdict::Unknown
        );
        assert_eq!(
            classify_announce(&own, Some(&row), &parsed),
            AnnounceVerdict::Stale
        );
        let newer = Announce::create(&a, 4, &[]).unwrap();
        assert_eq!(
            classify_announce(&own, Some(&row), &newer),
            AnnounceVerdict::Refresh
        );
        let mut other = row.clone();
        other.enc_pubkey = [5; 32];
        assert_eq!(
            classify_announce(&own, Some(&other), &parsed),
            AnnounceVerdict::Conflict
        );
        assert_eq!(
            classify_announce(&a.node_id(), None, &parsed),
            AnnounceVerdict::Own
        );
        let cache = KeyCache::rebuild(&[row]);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.lookup(&node_prefix(&a.node_id())).len(), 1);
        assert!(cache.get(&a.node_id()).is_some());
        assert!(cache.get(&own).is_none());
    }

    #[test]
    fn node_ids_parse_hex_and_bech32() {
        let id = keyset(3).node_id();
        let hex = id111::encoding::hex_encode(id.as_bytes());
        let b32 = id111::encoding::bech32_encode(id.as_bytes());
        assert_eq!(parse_node_id(&hex).unwrap(), id);
        assert_eq!(parse_node_id(&hex.to_uppercase()).unwrap(), id);
        assert_eq!(parse_node_id(&format!(" {b32} ")).unwrap(), id);
        assert_eq!(parse_node_id("node1xyz"), Err(CardError::BadNodeId));
        assert_eq!(parse_node_id("zz"), Err(CardError::BadNodeId));
    }

    #[test]
    fn appdata_and_bundle() {
        assert_eq!(AppData::parse(&[]), AppData::default());
        assert_eq!(AppData::parse(&[2, 1, 0, 0]), AppData::default());
        assert_eq!(AppData::parse(&[1, 7, 0, 0]).roles, 7);
        assert_eq!(
            AppData::role_names(7),
            vec!["mailbox", "always_on_hub", "lora_attached"]
        );
        let b = HubBundle {
            card: "ABC".into(),
            code: "0123BCDF".into(),
            address: "[fe80::1]:11183".into(),
        };
        assert_eq!(HubBundle::parse(&b.text()), Some(b.clone()));
        assert_eq!(
            HubBundle::parse("111hub:ABC:0123BCDF:[fe80::1]:11183"),
            Some(b)
        );
        assert_eq!(HubBundle::parse("nope"), None);
        assert_eq!(HubBundle::parse("111HUB::x:y"), None);
    }
}
