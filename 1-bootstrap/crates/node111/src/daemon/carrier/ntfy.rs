//! Layer 7 (§6.2): T0 stub; filled by row P1 (the ntfy carrier prototype and its smoke log; §6.1,
//! §6.2, §6.4; D30 F3, Q15–Q16), wired to the forwarder by row D11.
//!
//! The ntfy carrier: [`super::Carrier`] over its own `ureq::Agent` (as `wan_probe` builds one,
//! never the updater's `Fetcher`; S6); 5A requests in `spawn_blocking` with connect and global
//! timeouts; 5B on the one named std thread `111d-ntfy-5b`; topics `topic(X→Y)` from the pairing's
//! `topic_key`; batches `0x01 ‖ n ‖ (len u16 BE ‖ frame)…`, base64url unpadded ≤ 4,096 B,
//! `max_frame` 3,068 B; publish `POST <server>/<topic>`, drain `GET …/json?poll=1&since=<id>`
//! (§6.2). Nothing is built while `carrier.kind = "none"` (§6.3).
#![allow(dead_code)]

/// The ntfy carrier (§6.2). T0: an empty type; P1 gives it its fields and its `Carrier` impl.
pub struct NtfyCarrier;
