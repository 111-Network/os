//! §8.1 — the BIP-39 root secret.
//!
//! Implements (owner: T2): mnemonic generation and validation over the
//! rust-bitcoin `bip39` crate (2048-word English list):
//!
//! - 12 words (128-bit) default; 24 words (256-bit) for hubs (Cat 4 SHOULD);
//! - generation is `from_entropy` over **caller-supplied entropy** — the crate
//!   is used with NO internal RNG feature; entropy comes from `keyset.rs`'s
//!   §4.2 `getrandom()` path;
//! - restore input is NFKC-normalized (case/whitespace folded) and the BIP-39
//!   checksum MUST be validated before derivation (§8.4 hygiene);
//! - **no passphrase in v1** (the silent-wrong-wallet trap, §8.1);
//! - the 64-byte BIP-39 seed feeds `slip0010` (§2).
//!
//! Known limit, stated per §8.1: the checksum is 4 bits at 12 words (8 at
//! 24) — the §8.4 NodeId confirmation exists because of it.

use std::borrow::Cow;
use std::fmt;

use zeroize::Zeroizing;

/// §8.1 — the v1-legal mnemonic lengths: **12 words (128-bit entropy) is the
/// fleet default**; 24 words (256-bit) for hubs. Other BIP-39 counts (15/18/21)
/// are outside the v1 profile and rejected (§8.2's descriptor `encoding` field
/// defines only 0x01 = 12 words and 0x02 = 24 words).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WordCount {
    /// 12 words / 128-bit entropy — the default (§8.1).
    #[default]
    Twelve,
    /// 24 words / 256-bit entropy — Cat 4 hubs SHOULD use this (§8.1).
    TwentyFour,
}

impl WordCount {
    /// Number of words (12 or 24).
    pub fn word_count(self) -> usize {
        match self {
            WordCount::Twelve => 12,
            WordCount::TwentyFour => 24,
        }
    }

    /// Entropy length in bytes (16 or 32).
    pub fn entropy_bytes(self) -> usize {
        match self {
            WordCount::Twelve => 16,
            WordCount::TwentyFour => 32,
        }
    }
}

/// Errors from mnemonic wrapping/parsing (§8.1, §8.4 input hygiene).
#[derive(Debug)]
pub enum Bip39Error {
    /// v1 accepts exactly 16 or 32 bytes of entropy (12/24 words, §8.1).
    BadEntropyLength(usize),
    /// The BIP-39 layer rejected the words: unknown word, bad word count, or —
    /// the §8.4 MUST — an invalid checksum.
    Mnemonic(::bip39::Error),
    /// Checksum-valid but not 12/24 words — outside the v1 profile (§8.1/§8.2).
    UnsupportedWordCount(usize),
}

impl fmt::Display for Bip39Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Bip39Error::BadEntropyLength(n) => write!(
                f,
                "§8.1: entropy must be 16 or 32 bytes (12/24 words), got {n}"
            ),
            Bip39Error::Mnemonic(e) => write!(f, "§8.4: mnemonic rejected: {e}"),
            Bip39Error::UnsupportedWordCount(n) => {
                write!(f, "§8.1: v1 profile accepts only 12/24 words, got {n}")
            }
        }
    }
}

impl std::error::Error for Bip39Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Bip39Error::Mnemonic(e) => Some(e),
            _ => None,
        }
    }
}

/// A checksum-validated BIP-39 root secret in the v1 profile (12/24 words,
/// English wordlist, no passphrase — §8.1/§8.2).
///
/// The wrapped value holds word *indices* into the compiled-in wordlist. Seed
/// material derived from it is always returned zeroizing (§12: the mnemonic is
/// the master of both keys).
pub struct RootMnemonic(::bip39::Mnemonic);

impl RootMnemonic {
    /// Compose a mnemonic from caller-supplied entropy (`Mnemonic::from_entropy`
    /// — no crate RNG anywhere in this crate; entropy comes from `keyset.rs`'s
    /// §4.2 `getrandom()` path, or from the §8.3 QR payload's raw-entropy
    /// field). v1 profile: exactly 16 or 32 bytes.
    pub fn from_entropy(entropy: &[u8]) -> Result<Self, Bip39Error> {
        match entropy.len() {
            16 | 32 => {}
            n => return Err(Bip39Error::BadEntropyLength(n)),
        }
        let m = ::bip39::Mnemonic::from_entropy(entropy).map_err(Bip39Error::Mnemonic)?;
        Ok(Self(m))
    }

    /// §8.4 restore parse. Input hygiene (normative): case/whitespace folded,
    /// NFKC-normalized, then the checksum-validating `parse_in_normalized`
    /// (English pinned — §8.2 `encoding` 0x01/0x02). The BIP-39 checksum MUST
    /// validate before any derivation. Only the v1 word counts (12/24) are
    /// accepted — a checksum-valid 15/18/21-word phrase is rejected as
    /// out-of-profile.
    pub fn parse(words: &str) -> Result<Self, Bip39Error> {
        // Case/whitespace folding per §8.4; split_whitespace inside
        // parse_in_normalized collapses internal runs.
        let folded = words.trim().to_lowercase();
        let mut cow = Cow::Borrowed(folded.as_str());
        ::bip39::Mnemonic::normalize_utf8_cow(&mut cow);
        let m = ::bip39::Mnemonic::parse_in_normalized(::bip39::Language::English, cow.as_ref())
            .map_err(Bip39Error::Mnemonic)?;
        match m.word_count() {
            12 | 24 => Ok(Self(m)),
            n => Err(Bip39Error::UnsupportedWordCount(n)),
        }
    }

    /// Word count (12 or 24 in the v1 profile).
    pub fn word_count(&self) -> usize {
        self.0.word_count()
    }

    /// The canonical space-joined lowercase phrase — the §13 `mnemonic` file
    /// body (the file adds a trailing newline; `parse` trims it back).
    pub fn phrase(&self) -> String {
        self.0.to_string()
    }

    /// The raw entropy (16 or 32 bytes) — the payload of the §8.3 step-2
    /// project QR (4B descriptor ‖ raw entropy). Zeroizing.
    pub fn entropy(&self) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(self.0.to_entropy())
    }

    /// §8.1 — derive the 64-byte BIP-39 seed (PBKDF2-HMAC-SHA512, 2048 rounds,
    /// salt `"mnemonic" ‖ passphrase`). **No passphrase in v1** (§8.1: a typo'd
    /// passphrase silently derives a valid-but-different identity — the
    /// silent-wrong-wallet trap; an expert option, if ever, requires
    /// confirm-at-creation + NFKC per §15). The passphrase is therefore always
    /// the empty string, which is trivially NFKC-normalized, so the
    /// normalized-seed path is called directly. The seed is secret and
    /// returned zeroizing; it feeds `slip0010` (§2).
    pub fn to_seed(&self) -> Zeroizing<[u8; 64]> {
        Zeroizing::new(self.0.to_seed_normalized(""))
    }
}

impl fmt::Debug for RootMnemonic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // §5/§12: the mnemonic is secret — never Debug-print the words.
        write!(f, "RootMnemonic({} words, redacted)", self.word_count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// Official BIP-39 vectors: entropy → words. (Seeds in the official table
    /// use passphrase "TREZOR"; v1 pins the empty passphrase, so seed checks
    /// below use empty-passphrase values.)
    #[test]
    fn bip39_entropy_to_words_vectors() {
        // 00000000000000000000000000000000
        let m = RootMnemonic::from_entropy(&[0u8; 16]).unwrap();
        assert_eq!(m.word_count(), 12);
        assert_eq!(
            m.phrase(),
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
        );

        // 7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f
        let m = RootMnemonic::from_entropy(&[0x7f; 16]).unwrap();
        assert_eq!(
            m.phrase(),
            "legal winner thank year wave sausage worth useful legal winner thank yellow"
        );

        // 24-word: ff…ff
        let m = RootMnemonic::from_entropy(&[0xff; 32]).unwrap();
        assert_eq!(m.word_count(), 24);
        assert_eq!(
            m.phrase(),
            "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote"
        );

        // 24-word: 00…00
        let m = RootMnemonic::from_entropy(&[0u8; 32]).unwrap();
        assert_eq!(
            m.phrase(),
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art"
        );
    }

    /// Empty-passphrase seed for the canonical mnemonic — cross-checked
    /// against independent PBKDF2-HMAC-SHA512 implementations.
    #[test]
    fn bip39_seed_derivation_empty_passphrase() {
        let m = RootMnemonic::parse(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        let seed = m.to_seed();
        assert_eq!(
            to_hex(&seed[..]),
            "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc19a\
             5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4"
                .replace(' ', "")
        );
    }

    /// Words → entropy round-trip (§8.3 QR payload fidelity).
    #[test]
    fn bip39_entropy_round_trip() {
        for bytes in [[0x7fu8; 16].as_slice(), [0xffu8; 32].as_slice()] {
            let m = RootMnemonic::from_entropy(bytes).unwrap();
            let ent = m.entropy();
            assert_eq!(&ent[..bytes.len()], bytes);
            let m2 = RootMnemonic::parse(&m.phrase()).unwrap();
            assert_eq!(m2.phrase(), m.phrase());
        }
    }

    /// §8.4 input hygiene: case folded, whitespace collapsed, NFKC applied.
    #[test]
    fn bip39_parse_hygiene() {
        let messy = "  ABANDON\tAbandon aBaNdOn abandon ABANDON abandon abandon\nabandon abandon abandon  abandon ABOUT \n";
        let m = RootMnemonic::parse(messy).unwrap();
        assert_eq!(m.word_count(), 12);
        assert_eq!(
            m.phrase(),
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
        );
    }

    /// §8.4: the checksum MUST be validated — a single-word substitution that
    /// breaks the checksum is rejected (the stated 4-bit weakness means ~1/16
    /// of such errors slip through; the §8.4 NodeId confirmation covers those).
    #[test]
    fn bip39_parse_rejects_bad_checksum() {
        // "abandon"×12: last word wrong (valid word, wrong checksum).
        let r = RootMnemonic::parse(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon",
        );
        assert!(matches!(
            r,
            Err(Bip39Error::Mnemonic(::bip39::Error::InvalidChecksum))
        ));
    }

    #[test]
    fn bip39_parse_rejects_unknown_word_and_count() {
        // Unknown word.
        let r = RootMnemonic::parse(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon zzz",
        );
        assert!(matches!(
            r,
            Err(Bip39Error::Mnemonic(::bip39::Error::UnknownWord(11)))
        ));
        // Word count not a multiple of 6 (rejected by the crate layer).
        let r = RootMnemonic::parse("abandon abandon abandon");
        assert!(matches!(
            r,
            Err(Bip39Error::Mnemonic(::bip39::Error::BadWordCount(3)))
        ));
    }

    /// §8.1/§8.2: a checksum-VALID 15-word phrase is BIP-39-legal but outside
    /// the v1 profile — rejected.
    #[test]
    fn bip39_parse_rejects_out_of_profile_word_count() {
        let fifteen = ::bip39::Mnemonic::from_entropy(&[0u8; 20]).unwrap();
        let r = RootMnemonic::parse(&fifteen.to_string());
        assert!(matches!(r, Err(Bip39Error::UnsupportedWordCount(15))));
    }

    #[test]
    fn bip39_from_entropy_rejects_bad_lengths() {
        for n in [0usize, 15, 17, 20, 33, 64] {
            let buf = vec![0u8; n];
            assert!(matches!(
                RootMnemonic::from_entropy(&buf),
                Err(Bip39Error::BadEntropyLength(_))
            ));
        }
    }

    /// Display/Debug hygiene: the phrase is only produced explicitly; Debug is
    /// redacted (§12 — the mnemonic is the master of both keys).
    #[test]
    fn bip39_debug_is_redacted() {
        let m = RootMnemonic::from_entropy(&[0u8; 16]).unwrap();
        let dbg = format!("{m:?}");
        assert!(!dbg.contains("abandon"));
        assert!(dbg.contains("redacted"));
    }

    #[test]
    fn bip39_word_count_profile() {
        assert_eq!(WordCount::Twelve.word_count(), 12);
        assert_eq!(WordCount::Twelve.entropy_bytes(), 16);
        assert_eq!(WordCount::TwentyFour.word_count(), 24);
        assert_eq!(WordCount::TwentyFour.entropy_bytes(), 32);
        assert_eq!(WordCount::default(), WordCount::Twelve);
        // from_entropy accepts exactly the two profile lengths.
        assert!(RootMnemonic::from_entropy(&[0u8; 16]).is_ok());
        assert!(RootMnemonic::from_entropy(&[0u8; 32]).is_ok());
        // Seed derivation sanity at 24 words too.
        let m = RootMnemonic::from_entropy(&[0u8; 32]).unwrap();
        let seed = m.to_seed();
        assert_eq!(seed.len(), 64);
    }
}
