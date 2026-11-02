//! Layer 7 (§12; §6.2 consent copy): T0 stub; filled by row D10.
//!
//! The Layer 7 consents in the pinned round trip (`409 consent_required`, then the repeat with
//! `accept_warning`): `carrier_enable`, `pair_plain`, `gateway_bit`, `origin_serve` (§12, §13.1,
//! §13.2); each text verbatim, its hash recorded in `state/relay.json`, a changed text asks again.
#![allow(dead_code)]

/// A recorded consent (action, subject, text hash; §12). T0: the name only.
pub struct ConsentRecord;
