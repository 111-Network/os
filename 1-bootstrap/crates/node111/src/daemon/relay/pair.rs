//! Layer 7 (§4.6; §4.1 rule 1): T0 stub; filled by row D6.
//!
//! Pairing: the `111PAIR:<card b32>:<code>:<link SPKI b32>:<address or empty>:<role>:<via>` bundle
//! printed and parsed (§4.6 steps 1–2), the role complement and `via` rules, the `pair_plain`
//! warning, the pin and the two `ENROLL`s (steps 3–6), unpair. No dial or record on a non-hub.
#![allow(dead_code)]

/// A parsed `111PAIR:` bundle (§4.6 step 1). T0: the name only.
pub struct PairBundle;
