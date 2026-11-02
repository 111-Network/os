//! Layer 7 (§4.1, §4.3–§4.5, §4.7): T0 stub; filled by row D5.
//!
//! TLS 1.3 with RFC 7250 raw public keys for non-private paired-hub links, through `tokio-rustls`
//! on the `ring` provider: M6's `StreamWrap` (§4.4) implemented here, inbound peeking `0x16` (TLS)
//! or `0x31` (plain), the SPKI pinned from the pairing bundle (§4.2), TLS 1.3 and Ed25519 only,
//! client auth mandatory, no X.509, CA, SNI or name check (§4.3). M5 reads `relay::tls::wrap()`
//! into `TcpLanConfig.wrap` (`None` until D5, and always `None` without a non-private pair). No
//! runtime weight before a non-LAN pair.
#![allow(dead_code)]

/// The pinned raw-public-key stream wrap (§4.3, §4.4). T0: the name only.
pub struct RpkWrap;
