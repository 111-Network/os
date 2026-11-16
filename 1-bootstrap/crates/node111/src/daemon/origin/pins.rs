//! Layer 8 (§11.1, §16.7, §7.2, §9.5): T0 stub; filled by row D27 (the admission rule, the
//! catalogue routes, the stream cap), row D45 (`origin.components` and the notice route), row L11
//! (the release's source tarball) and row D46 (the Termux pins).
//!
//! One admission rule for `GET /origin/v1/blob/{sha256}` (amends Layer 7 §9.1, additive): a hub
//! answers only a hash in a pin list it holds, whose bytes it holds and that match it; everything
//! else answers 404. The pin sources, each behind its own switch and all behind `hub_role` and LAN
//! sharing: the verified release manifest and its `[source]` pin (`origin.serve`, Layer 7's), the
//! embedded component declarations served in place from `cache/packages/` (`origin.components`), a
//! verified catalogue's entries and sidecars (`origin.datasets`), and the manifest's `[[deb]]` and
//! `[apk.*]` pins (`origin.components` and the "Termux install files" tick, off in v1:
//! `termux.rs`). The catalogue routes, the notice route and the stream cap (`origin.max_streams`)
//! sit beside it. No listener, timer or request log.
#![allow(dead_code)]

/// The pin lists the admission rule reads (§11.1). T0: the name only.
pub struct Pins;
