//! Layer 8 (§15.2; §7.7; §16.2 `ConsentRequired.facts`): T0 stub; filled by row D19.
//!
//! Layer 8's six consent texts, verbatim, with their subjects and addons: `dataset_fetch`,
//! `dataset_override`, `datasets_serve`, `components_serve` (addon `termux`), `update_without_copy`
//! and `store_start_empty`, in the pinned `ConsentRequired` round trip (`409 consent_required`,
//! then the repeat with `accept_warning`). The hash covers the text alone, which holds stable facts
//! only; the live figures ride `facts` beside it and are never hashed. Publisher strings are
//! inserted as text; the agent's bearer never sees a hash and never accepts one.
#![allow(dead_code)]

/// One Layer 8 consent: its action, subject, text and `facts` (§15.2). T0: the name only.
pub struct ConsentText;
