//! Layer 8 (§5.3, §5.4, §6.1 to §6.3): T0 stub; filled by row D24.
//!
//! The catalogue: TOML in the release manifest's style (`deny_unknown_fields`), at most 1,000
//! entries and 1 MiB checked before parsing, every (`id`, `version`) unique; an entry of an unknown
//! `kind` skipped whole and counted, any other unknown key refusing the file. The signature:
//! Ed25519 by the publisher's node key over `"111/catalogue/v1" ‖ publisher NodeId ‖ the file's
//! bytes`, a detached `<publisher hex>.toml.sig`. Trust: listed only for the pinned home hub and
//! the publishers in `follow`. Freeze: the highest verified `seq` per publisher kept, a lower one
//! refused; no clock expires a catalogue.
#![allow(dead_code)]

/// One verified catalogue of one publisher (§5.4, §6). T0: the name only.
pub struct Catalogue;
