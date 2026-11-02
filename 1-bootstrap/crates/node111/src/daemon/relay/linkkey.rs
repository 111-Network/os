//! Layer 7 (§4.2): T0 stub; filled by row D5.
//!
//! The link key: a fresh Ed25519 key from the OS CSPRNG at the first pairing, `secrets/link-key`
//! 0600 in the 0700 `secrets/`, never seed-derived, never the identity key, on no API and in no
//! backup, re-minted after a restore; its 44 B SPKI rides the `111PAIR:` bundle (§4.2, §4.6; §19
//! C9).
#![allow(dead_code)]

/// This hub's link key and its SPKI (§4.2). T0: the name only.
pub struct LinkKey;
