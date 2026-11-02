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

/// The `tcp-lan` stream wrap (§4.1 rule 1, §4.4), read once by the fabric when it builds `tcp-lan`
/// (`daemon/fabric/mod.rs`, row M5). `fabric_dir` is `<root>/fabric` (the link key lives in
/// `<root>/secrets/`, §4.2); `pairs` is the daemon's live pairing table (§4.6: the pins, `lan`, the
/// warning hash), updated whenever a pin is recorded or a pair goes. `None` keeps every connection
/// plain and byte-identical: until D5, and always without a non-private pair. Stub: `None`.
pub fn wrap(
    _fabric_dir: &std::path::Path,
    _pairs: &crate::daemon::fabric::settings::PairTable,
) -> Option<std::sync::Arc<dyn mesh111::link::tcp_lan::StreamWrap>> {
    None
}
