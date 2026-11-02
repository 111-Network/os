//! Layer 7 (§6.3; §2.4 item 4; §12): T0 stub; filled by row D1.
//!
//! `state/relay.json`: every Layer 7 setting at its default, all off; 0600, schema 1, invalid moved
//! aside; the consent hashes, the drain's last id per topic and the metered window beside them;
//! secrets write-only (§6.3, §13.1). No file exists until the first write.
#![allow(dead_code)]

/// The `state/relay.json` document (§6.3). T0: the name only.
pub struct RelaySettings;
