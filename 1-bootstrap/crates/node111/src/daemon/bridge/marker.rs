//! The relay marker (Layer 6 `60-transport/README.md` §8 "The marker (normative; bridge
//! B1)"): every relayed copy, and every part of a split copy, starts with `[<n>/<TAG>] `.
//!
//! - **Build** ([`marker`]): `<n>` is the origin sender's name after the §3 ingest function,
//!   reduced to the marker alphabet ([`marker_name`]: no `]`, `/`, Cc or Cf), cut to 10
//!   bytes at a UTF-8 boundary, `?` when nothing is left (`W1AW/7` → `W1AW7`). `<TAG>` is
//!   one to three ASCII letters registered per adapter manifest (`M`, `LX`, `DC`, `SX`).
//!   A split copy reads `[n/TAG] (k/n) text`: the marker first, on every part.
//! - **Parse** ([`parse`]): an optional `(k/n) ` first (a part number added by another tool
//!   cannot hide a marker), then `[`, one to ten bytes without `]`, `/`, Cc or Cf, `/`, a
//!   **registered** TAG, `]` and a space. A text that parses is "already relayed" and is
//!   relayed nowhere — by any 111 node, which stops loops between nodes; a human who types
//!   a marker gets no relay either (the UI copy says so).

use super::ingest;

/// Bytes of `<n>` at most (§8).
pub const NAME_MAX: usize = 10;

/// Is `c` outside the marker alphabet (`]`, `/`, Cc, Cf)?
fn excluded(c: char) -> bool {
    c == ']' || c == '/' || c.is_control() || ingest::is_cf(c)
}

/// `<n>` for a sender name: the §3 ingest function, the marker alphabet, 10 bytes at a
/// UTF-8 boundary, `?` when nothing is left.
pub fn marker_name(name: &str) -> String {
    let cleaned = ingest::clean(ingest::cut(name, 4 * ingest::MAX_NAME_BYTES));
    let reduced: String = cleaned.chars().filter(|c| !excluded(*c)).collect();
    let n = ingest::cut(&reduced, NAME_MAX);
    if n.is_empty() {
        "?".to_string()
    } else {
        n.to_string()
    }
}

/// A TAG: one to three ASCII letters.
pub fn valid_tag(tag: &str) -> bool {
    (1..=3).contains(&tag.len()) && tag.bytes().all(|b| b.is_ascii_alphabetic())
}

/// The prefix `[<n>/<TAG>] ` (the trailing space included).
pub fn marker(name: &str, tag: &str) -> String {
    format!("[{}/{}] ", marker_name(name), tag)
}

/// The part prefix `(k/n) ` a split adds after the marker (§9).
pub fn part_prefix(k: usize, n: usize) -> String {
    format!("({k}/{n}) ")
}

/// A parsed marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed<'a> {
    /// A `(k/n) ` found before the marker.
    pub part: Option<(u32, u32)>,
    pub name: &'a str,
    pub tag: &'a str,
    /// The text after the marker's space.
    pub rest: &'a str,
}

/// `(k/n) ` at the start of `s`: `(`, digits, `/`, digits, `)`, a space.
fn leading_part(s: &str) -> Option<((u32, u32), &str)> {
    let r = s.strip_prefix('(')?;
    let slash = r.find('/')?;
    let (k, r) = r.split_at(slash);
    let r = &r[1..];
    let close = r.find(')')?;
    let (n, r) = r.split_at(close);
    let r = r.strip_prefix(") ")?;
    let digits = |d: &str| {
        (!d.is_empty() && d.len() <= 9 && d.bytes().all(|b| b.is_ascii_digit()))
            .then(|| d.parse::<u32>().ok())
            .flatten()
    };
    Some(((digits(k)?, digits(n)?), r))
}

/// Parse a marker at the start of `text` against the registered `tags` (§8).
pub fn parse<'a>(text: &'a str, tags: &[&str]) -> Option<Parsed<'a>> {
    let (part, s) = match leading_part(text) {
        Some((p, rest)) => (Some(p), rest),
        None => (None, text),
    };
    let s = s.strip_prefix('[')?;
    // `<n>`: 1..=10 bytes without `]`, `/`, Cc, Cf — so the first `/` ends it
    let slash = s.find('/')?;
    let name = &s[..slash];
    if name.is_empty() || name.len() > NAME_MAX || name.chars().any(excluded) {
        return None;
    }
    let s = &s[slash + 1..];
    let close = s.find(']')?;
    let tag = &s[..close];
    if !valid_tag(tag) || !tags.contains(&tag) {
        return None;
    }
    let rest = s[close + 1..].strip_prefix(' ')?;
    Some(Parsed {
        part,
        name,
        tag,
        rest,
    })
}

/// Does `text` parse as already relayed?
pub fn is_relayed(text: &str, tags: &[&str]) -> bool {
    parse(text, tags).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAGS: &[&str] = &["M", "LX", "DC", "SX"];

    #[test]
    fn builds_the_marker_from_the_ingested_name() {
        assert_eq!(marker("W1AW", "M"), "[W1AW/M] ");
        assert_eq!(marker("W1AW/7", "M"), "[W1AW7/M] ", "the slash is dropped");
        assert_eq!(marker("]", "M"), "[?/M] ", "a ]-named board reads ?");
        assert_eq!(marker("", "LX"), "[?/LX] ");
        assert_eq!(marker("a]b/c\u{202E}d\u{200D}e", "M"), "[abcde/M] ");
        assert_eq!(
            marker("abcdefghijklmnop", "M"),
            "[abcdefghij/M] ",
            "10 bytes"
        );
        // 4 × "€" = 12 B ⇒ 3 whole euros (9 B), never half a char
        assert_eq!(marker_name(&"\u{20AC}".repeat(4)), "\u{20AC}".repeat(3));
        assert!(valid_tag("M") && valid_tag("LX") && valid_tag("abc"));
        assert!(!valid_tag("") && !valid_tag("ABCD") && !valid_tag("M1"));
    }

    #[test]
    fn parses_markers_with_an_optional_leading_part_number() {
        let p = parse("[W1AW/M] hello", TAGS).unwrap();
        assert_eq!(
            (p.part, p.name, p.tag, p.rest),
            (None, "W1AW", "M", "hello")
        );
        let p = parse("(1/2) [W1AW7/M] part one", TAGS).unwrap();
        assert_eq!(p.part, Some((1, 2)));
        assert_eq!(p.rest, "part one");
        // our own split format: marker first, then the part number
        let p = parse("[?/M] (2/3) tail", TAGS).unwrap();
        assert_eq!((p.name, p.rest), ("?", "(2/3) tail"));
        assert!(is_relayed("[?/LX] x", TAGS), "the ]-named board's copy");
        assert!(is_relayed("[a/DC] ", TAGS));
    }

    #[test]
    fn refuses_what_is_not_a_marker() {
        for t in [
            "hello [W1AW/M] there", // not at the start
            "[W1AW/Q] hi",          // TAG not registered
            "[W1AW/M]hi",           // no space
            "[/M] hi",              // empty name
            "[abcdefghijk/M] hi",   // 11 bytes
            "[a]b/M] hi",           // `]` in the name
            "[W1AW/7/M] hi",        // `/` in the name (the TAG would be `7/M`)
            "[a\u{202E}/M] hi",     // Cf in the name
            "(1/2)[W1AW/M] hi",     // the part number needs its space
            "(x/2) [W1AW/M] hi",    // not a part number, and the marker is not first
            "[W1AW/M",
            "",
        ] {
            assert!(!is_relayed(t, TAGS), "{t:?}");
        }
        assert!(!is_relayed("[W1AW/M] hi", &[]), "nothing registered");
    }

    #[test]
    fn every_built_marker_parses() {
        for name in [
            "W1AW/7",
            "]",
            "",
            "José",
            "a\u{200D}b",
            "x".repeat(40).as_str(),
        ] {
            let m = marker(name, "LX");
            let text = format!("{m}body");
            let p = parse(&text, TAGS).unwrap_or_else(|| panic!("{text:?}"));
            assert_eq!(p.rest, "body");
            let split = format!("{m}{}tail", part_prefix(2, 3));
            assert!(is_relayed(&split, TAGS));
        }
    }
}
