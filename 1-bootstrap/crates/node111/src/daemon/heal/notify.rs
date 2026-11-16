//! Layer 9 (§14.4 phone notifications; §17.1 `heal.notify`): T0 stub; filled by row D51.
//!
//! With `heal.notify` at `transitions` (default `off`), `111d` posts one content-free notification,
//! "111: your node needs you", through `termux-notification` when the Needs-you set goes from empty
//! to non-empty, however many items join it, and none while it stays non-empty: one transient
//! process then, none otherwise. On Linux, and wherever the helper is absent, the setting does
//! nothing and its text says so.
//!
//! T0: the name only; nothing is spawned.
#![allow(dead_code)]

/// The Needs-you transition that posts the one notification (§14.4). T0: the name only.
pub struct Notifier;
