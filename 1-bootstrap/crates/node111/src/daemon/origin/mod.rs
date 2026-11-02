//! Layer 7 (§9.1): T0 stub; filled by row D7 (the hub as origin: serving).
//!
//! `state/origin/`: only bytes that passed `parse_verified` and blobs whose sha256 and size match a
//! pin, served at `GET /origin/v1/{channel-111-stable.toml, .minisig, blob/<sha256>}` on the
//! existing dashboard listener, only while `hub_role`, LAN sharing and `origin.serve` are on (§9.1;
//! S4, S8); `admit(…)` for row L4, the byte count for `RelayStatus.origin.bytes`. No listener,
//! timer or resident buffer.
#![allow(dead_code)]

/// The served origin directory (§9.1). T0: the name only.
pub struct Origin;
