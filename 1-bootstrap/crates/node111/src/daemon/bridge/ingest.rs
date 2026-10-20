//! The one ingest function (Layer 6 `60-transport/README.md` §3 "One ingest function",
//! "Dedupe"; §8 guard 2; §15.1, §15.9): every foreign string — sender names, bodies,
//! conversation titles and native ids, the marker name — passes [`clean`] before it is
//! stored, shown or relayed.
//!
//! - **Strip Cc and Cf** ([`is_stripped`]): control characters except `\n` and `\t`, and
//!   every format character (bidi embeddings and isolates such as U+202E, zero-width
//!   joiners U+200C/U+200D, the BOM, tag characters, …; [`CF`] is the Unicode 15.1 `Cf`
//!   table). Then **NFC** (`unicode-normalization`), so two spellings of one name are one
//!   string. Text is stored as that and rendered as text only.
//! - **Caps** at a UTF-8 boundary: names (sender names, titles) 64 B ([`MAX_NAME_BYTES`]);
//!   bodies 64 KiB ([`MAX_BODY_BYTES`]) — a longer body is stored cut with
//!   `meta.truncated` = its original size, never silently (the UI says "message too long
//!   to show (N bytes)"). Native ids and accounts are cleaned but never cut: one over the
//!   store's field cap is refused (counted as noise), because a cut id could merge two
//!   senders.
//! - **`meta`** is rebuilt from a closed set of keys (`snr`, `rssi`, `hops`, `via_mqtt`,
//!   `ok_to_mqtt`), each type-checked; anything else the adapter sent is dropped (the core
//!   interprets no foreign field).
//! - **Dedupe** is the store's (§3 key `(conv, sender_native, native_msg_id)`): the
//!   adapter's [`super::AdapterFacts::ids_globally_unique`] picks key-alone (LXMF hash,
//!   app ids) or key + body within the last 32 inbound rows (Meshtastic's 32-bit ids).
//! - **Own echo** (§8 guard 2, [`OwnEcho`]): inbound from any account of this node (two
//!   boards of ours hear each other), from our LXMF destination (an account of the `lxmf`
//!   network), or carrying a native id we recorded for one of our own sends is dropped and
//!   counted, never stored.

use super::InboundText;
use mesh111::store::{ForeignAuth, ForeignInsert, MAX_FOREIGN_BODY};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use unicode_normalization::UnicodeNormalization;

/// Names and titles (§3: 64 B at a UTF-8 boundary).
pub const MAX_NAME_BYTES: usize = 64;
/// One stored body (§3: 64 KiB; the store's own cap).
pub const MAX_BODY_BYTES: usize = MAX_FOREIGN_BODY;
/// Native ids and accounts: the store's field cap (refused above, never cut).
pub const MAX_FIELD_BYTES: usize = 256;
/// Raw input looked at before cleaning (a bound on the work per body; the original size
/// is still reported in `truncated`).
const MAX_RAW_BYTES: usize = 4 * MAX_BODY_BYTES;
/// Own native ids remembered per network (§8 guard 2; oldest forgotten).
pub const ECHO_IDS: usize = 4096;

/// Unicode general category `Cf` (format characters), Unicode 15.1, as inclusive ranges.
pub const CF: &[(u32, u32)] = &[
    (0x00AD, 0x00AD),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x180E, 0x180E),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x2066, 0x206F),
    (0xFEFF, 0xFEFF),
    (0xFFF9, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x13430, 0x1343F),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0001, 0xE0001),
    (0xE0020, 0xE007F),
];

/// Is `c` a format character (`Cf`)?
pub fn is_cf(c: char) -> bool {
    let u = u32::from(c);
    CF.iter().any(|&(a, b)| (a..=b).contains(&u))
}

/// Is `c` removed by [`clean`]: `Cc` except `\n` and `\t`, and every `Cf`.
pub fn is_stripped(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t') || is_cf(c)
}

/// The §3 function: strip Cc (keeping `\n`, `\t`) and Cf, then NFC-normalise.
pub fn clean(s: &str) -> String {
    s.chars().filter(|c| !is_stripped(*c)).nfc().collect()
}

/// The longest prefix of `s` of at most `max` bytes that ends at a UTF-8 boundary.
pub fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut i = max;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    &s[..i]
}

/// A name or title: cleaned, cut to 64 B; `None` when nothing is left.
pub fn name(s: &str) -> Option<String> {
    let c = clean(cut(s, MAX_RAW_BYTES));
    let c = cut(&c, MAX_NAME_BYTES);
    (!c.is_empty()).then(|| c.to_string())
}

/// A native id or account: cleaned, never cut — `None` when empty or over the store's
/// field cap (the item is refused, not merged with another id).
pub fn native(s: &str) -> Option<String> {
    if s.len() > 4 * MAX_FIELD_BYTES {
        return None;
    }
    let c = clean(s);
    (!c.is_empty() && c.len() <= MAX_FIELD_BYTES).then_some(c)
}

/// A body: cleaned and cut to 64 KiB. The second value is the original size in bytes when
/// the stored body was cut (`meta.truncated`), `None` otherwise.
pub fn body(s: &str) -> (String, Option<u64>) {
    let raw_cut = s.len() > MAX_RAW_BYTES;
    let c = clean(cut(s, MAX_RAW_BYTES));
    if raw_cut || c.len() > MAX_BODY_BYTES {
        (cut(&c, MAX_BODY_BYTES).to_string(), Some(s.len() as u64))
    } else {
        (c, None)
    }
}

/// The closed `meta` keys an adapter may report (§3), type-checked; plus `truncated`.
fn meta(adapter: &Value, truncated: Option<u64>) -> Option<String> {
    let mut out = Map::new();
    if let Value::Object(m) = adapter {
        for key in ["snr", "rssi"] {
            if let Some(v) = m.get(key).filter(|v| v.is_number()) {
                out.insert(key.into(), v.clone());
            }
        }
        if let Some(v) = m.get("hops").filter(|v| v.is_u64()) {
            out.insert("hops".into(), v.clone());
        }
        for key in ["via_mqtt", "ok_to_mqtt"] {
            if let Some(v) = m.get(key).filter(|v| v.is_boolean()) {
                out.insert(key.into(), v.clone());
            }
        }
    }
    if let Some(n) = truncated {
        out.insert("truncated".into(), Value::from(n));
    }
    (!out.is_empty()).then(|| Value::Object(out).to_string())
}

/// Map the contract's auth label to the store's.
pub fn auth(a: super::Auth) -> ForeignAuth {
    match a {
        super::Auth::None => ForeignAuth::None,
        super::Auth::ChannelKey => ForeignAuth::ChannelKey,
        super::Auth::Pki => ForeignAuth::Pki,
        super::Auth::Signed => ForeignAuth::Signed,
        // reserved for the second release (§14): never produced in v1, fail closed
        super::Auth::Attested111 => ForeignAuth::None,
    }
}

/// One inbound text as the store row it becomes (§3), or `None` when it is noise (no
/// usable sender, an over-long native id). `conv` is the conversation's row id; the
/// adapter's auth label passes through (§3 "Authentication label").
pub fn prepare(t: &InboundText, conv: i64, ids_globally_unique: bool) -> Option<ForeignInsert> {
    let sender_native = native(&t.sender_native)?;
    let native_msg_id = if t.native_msg_id.is_empty() {
        None
    } else {
        Some(native(&t.native_msg_id)?)
    };
    let (body, truncated) = body(&t.body);
    Some(ForeignInsert {
        conv,
        sender_native,
        sender_name: name(&t.sender_name),
        auth: auth(t.auth),
        native_msg_id,
        body,
        claim_ts: t.claim_ts,
        meta: meta(&t.meta, truncated),
        id_globally_unique: ids_globally_unique,
    })
}

/// §8 guard 2: the native ids of our own accepted sends, bounded per network (oldest
/// forgotten). Accounts are read live from the adapter's facts; this holds only ids.
#[derive(Debug, Default)]
pub struct OwnEcho {
    nets: HashMap<String, (HashSet<String>, VecDeque<String>)>,
}

impl OwnEcho {
    /// Record the native id of one of our own sends on `network`.
    pub fn record(&mut self, network: &str, native_id: &str) {
        let (set, order) = self.nets.entry(network.to_string()).or_default();
        if set.insert(native_id.to_string()) {
            order.push_back(native_id.to_string());
            while order.len() > ECHO_IDS {
                if let Some(old) = order.pop_front() {
                    set.remove(&old);
                }
            }
        }
    }

    /// Is `native_id` one of our own recorded sends on `network`?
    pub fn is_ours(&self, network: &str, native_id: &str) -> bool {
        self.nets
            .get(network)
            .is_some_and(|(set, _)| set.contains(native_id))
    }

    /// Ids held for `network` (gauge, tests).
    pub fn len(&self, network: &str) -> usize {
        self.nets.get(network).map_or(0, |(s, _)| s.len())
    }
}

/// Is `t` our own echo (§8 guard 2): from an account of this node on its network, or
/// carrying a native id of one of our own sends?
pub fn is_own_echo(t: &InboundText, accounts: &[super::AccountFacts], echo: &OwnEcho) -> bool {
    accounts.iter().any(|a| a.id == t.sender_native)
        || (!t.native_msg_id.is_empty() && echo.is_ours(&t.conv.network, &t.native_msg_id))
}

/// Does `s` match an anchored recipient-syntax pattern (`caps.recipient_syntax`, the
/// contract's ECMAScript subset: literals, `[…]` classes with ranges, `{n}`, `{n,m}`, `+`,
/// `*`, `?`)? Anything outside that subset fails closed (no match).
pub fn syntax_match(pattern: &str, s: &str) -> bool {
    let Some(p) = pattern.strip_prefix('^').and_then(|p| p.strip_suffix('$')) else {
        return false;
    };
    let p: Vec<char> = p.chars().collect();
    /// A character class with its repeat bounds.
    type Atom = (Vec<(char, char)>, usize, usize);
    let mut atoms: Vec<Atom> = Vec::new();
    let mut i = 0;
    while i < p.len() {
        let mut ranges = Vec::new();
        match p[i] {
            '[' => {
                i += 1;
                while i < p.len() && p[i] != ']' {
                    if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
                        ranges.push((p[i], p[i + 2]));
                        i += 3;
                    } else {
                        ranges.push((p[i], p[i]));
                        i += 1;
                    }
                }
                if i >= p.len() {
                    return false;
                }
            }
            '\\' => {
                i += 1;
                match p.get(i) {
                    Some(c) if !c.is_ascii_alphanumeric() => ranges.push((*c, *c)),
                    _ => return false,
                }
            }
            c if "().|^$".contains(c) => return false,
            c => ranges.push((c, c)),
        }
        i += 1;
        let (min, max) = match p.get(i) {
            Some('+') => {
                i += 1;
                (1, usize::MAX)
            }
            Some('*') => {
                i += 1;
                (0, usize::MAX)
            }
            Some('?') => {
                i += 1;
                (0, 1)
            }
            Some('{') => {
                let Some(end) = p[i..].iter().position(|c| *c == '}').map(|e| e + i) else {
                    return false;
                };
                let inner: String = p[i + 1..end].iter().collect();
                i = end + 1;
                let parsed = match inner.split_once(',') {
                    Some((a, b)) => a.parse().ok().zip(b.parse().ok()),
                    None => inner.parse().ok().map(|n| (n, n)),
                };
                match parsed {
                    Some(v) => v,
                    None => return false,
                }
            }
            _ => (1, 1),
        };
        atoms.push((ranges, min, max));
    }
    // greedy per atom is exact for the contract's patterns (no atom overlaps the next
    // ambiguously); a pattern that would need backtracking simply fails closed
    let s: Vec<char> = s.chars().collect();
    let mut at = 0;
    for (ranges, min, max) in atoms {
        let mut n = 0;
        while n < max && at < s.len() && ranges.iter().any(|(a, b)| s[at] >= *a && s[at] <= *b) {
            n += 1;
            at += 1;
        }
        if n < min {
            return false;
        }
    }
    at == s.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::bridge::{Auth, ConvKind, ConvRef};

    fn text(body: &str, name: &str) -> InboundText {
        InboundText {
            conv: ConvRef {
                network: "meshtastic".into(),
                account: "!0badc0de".into(),
                kind: ConvKind::Channel,
                native_id: "ch:LongFast:9f3c".into(),
            },
            sender_native: "!a1b2c3d4".into(),
            sender_name: name.into(),
            auth: Auth::ChannelKey,
            native_msg_id: "5000000a".into(),
            body: body.into(),
            claim_ts: Some(1_700_000_000),
            meta: serde_json::json!({"snr": 6.5, "rssi": -90, "hops": 2, "via_mqtt": true,
                                      "ok_to_mqtt": false, "evil": "<script>", "handoff": "pending"}),
        }
    }

    #[test]
    fn strips_bidi_and_zero_width_keeps_newline_and_tab_and_nfc_normalises() {
        // U+202E RIGHT-TO-LEFT OVERRIDE, U+200D ZWJ, U+2066 isolate, BEL, CR, BOM
        let s = "a\u{202E}b\u{200D}c\u{2066}d\u{7}e\r\u{FEFF}f\ng\th\u{E0041}";
        assert_eq!(clean(s), "abcdef\ng\th");
        // e + COMBINING ACUTE ⇒ é (NFC); the precomposed form is unchanged
        assert_eq!(clean("Jose\u{301}"), "Jos\u{e9}");
        assert_eq!(clean("Jos\u{e9}"), "Jos\u{e9}");
        assert!(is_cf('\u{202E}') && is_cf('\u{200D}') && is_cf('\u{AD}'));
        assert!(!is_cf('a') && !is_stripped('\n') && !is_stripped('\t'));
        assert!(is_stripped('\u{0}') && is_stripped('\u{7F}') && is_stripped('\u{85}'));
    }

    #[test]
    fn names_cap_at_64_bytes_on_a_utf8_boundary() {
        // 21 × "é" (2 B each) + "x" = 43 B: kept
        let short = format!("{}x", "\u{e9}".repeat(21));
        assert_eq!(name(&short).unwrap(), short);
        // 40 × "€" (3 B each) = 120 B ⇒ 21 whole euros (63 B), never half a char
        let long = "\u{20AC}".repeat(40);
        let n = name(&long).unwrap();
        assert_eq!(n.len(), 63);
        assert_eq!(n, "\u{20AC}".repeat(21));
        assert_eq!(name("\u{202E}\u{200D}"), None, "nothing left");
        assert_eq!(cut("abc", 2), "ab");
        assert_eq!(cut("a\u{e9}", 2), "a");
    }

    #[test]
    fn bodies_over_64_kib_are_cut_and_marked_with_the_original_size() {
        let (b, t) = body("hello\u{202E}");
        assert_eq!((b.as_str(), t), ("hello", None));
        let big = "\u{20AC}".repeat(30_000); // 90,000 B
        let (b, t) = body(&big);
        assert!(b.len() <= MAX_BODY_BYTES && b.len() > MAX_BODY_BYTES - 3);
        assert_eq!(t, Some(90_000));
        let row = prepare(&text(&big, "W1AW"), 7, false).unwrap();
        let m: Value = serde_json::from_str(row.meta.as_deref().unwrap()).unwrap();
        assert_eq!(m["truncated"], 90_000);
    }

    #[test]
    fn prepare_runs_every_string_through_the_function_and_keeps_a_closed_meta() {
        let row = prepare(&text("hi\u{200D} there", "W1\u{202E}AW"), 3, false).unwrap();
        assert_eq!(row.body, "hi there");
        assert_eq!(row.sender_name.as_deref(), Some("W1AW"));
        assert_eq!(row.auth, ForeignAuth::ChannelKey, "auth passes through");
        assert_eq!(row.native_msg_id.as_deref(), Some("5000000a"));
        assert!(!row.id_globally_unique);
        let m: Value = serde_json::from_str(row.meta.as_deref().unwrap()).unwrap();
        assert_eq!(
            m,
            serde_json::json!({"snr": 6.5, "rssi": -90, "hops": 2, "via_mqtt": true, "ok_to_mqtt": false})
        );
        let mut t = text("x", "");
        t.sender_native = "\u{200D}".into();
        assert!(prepare(&t, 3, false).is_none(), "no usable sender: noise");
        let mut t = text("x", "");
        t.native_msg_id = "a".repeat(300);
        assert!(prepare(&t, 3, true).is_none(), "an over-long id is refused");
        let mut t = text("x", "");
        t.native_msg_id.clear();
        assert_eq!(prepare(&t, 3, true).unwrap().native_msg_id, None);
    }

    #[test]
    fn own_echo_by_account_or_recorded_native_id() {
        let accounts = vec![
            crate::daemon::bridge::AccountFacts {
                id: "!0badc0de".into(),
                label: None,
                licensed: false,
            },
            crate::daemon::bridge::AccountFacts {
                id: "!0badbeef".into(),
                label: None,
                licensed: false,
            },
        ];
        let mut echo = OwnEcho::default();
        let mut t = text("hello", "me");
        assert!(!is_own_echo(&t, &accounts, &echo));
        t.sender_native = "!0badbeef".into(); // our second board heard our first
        assert!(is_own_echo(&t, &accounts, &echo));
        t.sender_native = "!a1b2c3d4".into();
        echo.record("meshtastic", "5000000a");
        assert!(is_own_echo(&t, &accounts, &echo), "a recorded native id");
        assert!(!echo.is_ours("lxmf", "5000000a"), "per network");
        for i in 0..(ECHO_IDS + 10) {
            echo.record("meshtastic", &format!("id{i}"));
        }
        assert_eq!(echo.len("meshtastic"), ECHO_IDS, "bounded");
        assert!(!echo.is_ours("meshtastic", "5000000a"), "oldest forgotten");
    }

    #[test]
    fn recipient_syntax_subset() {
        assert!(syntax_match("^[0-9a-f]{32}$", &"ab".repeat(16)));
        assert!(!syntax_match("^[0-9a-f]{32}$", &"AB".repeat(16)));
        assert!(!syntax_match("^[0-9a-f]{32}$", &"ab".repeat(17)));
        assert!(syntax_match("^![0-9a-f]{8}$", "!a1b2c3d4"));
        assert!(!syntax_match("^![0-9a-f]{8}$", "a1b2c3d4"));
        assert!(syntax_match("^[A-Za-z0-9_-]{1,64}$", "friends_1"));
        assert!(
            !syntax_match("^(a|b)$", "a"),
            "outside the subset fails closed"
        );
        assert!(!syntax_match("[0-9]+", "1"), "unanchored fails closed");
    }
}
