//! Layer 9 (§17.1 every Layer 9 setting; §17.2 the consent copy; §2.3): T0 stub; filled by row D14
//! (the reader and writer) and row D35 (the settings route's consents).
//!
//! `state/heal.json` `{"schema": 1, …}`, 0600, written atomically and only on a change; absent,
//! every key reads its default (`heal.autonomy` `propose`, `heal.fixes.receive` `home_hub`,
//! `heal.fixes.senders` `[]`, `heal.fixes.send` `off`, `heal.ask_hub` `false`, `heal.serve_members`
//! `false`, `heal.code_drafts` `false`, `heal.notify` `off`). Unknown keys are kept and written
//! back untouched, an unknown value reads as its key's default (`heal.autonomy: "auto"` reads
//! `propose`), and each consent's text hash is stored beside its key. Changed only through the
//! loopback `PATCH /api/v1/repairs/settings`; v1 reads no `heal/grants` record and offers no `auto`
//! rung.
//!
//! T0: the names only; nothing is read.
#![allow(dead_code)]

/// The settings of `state/heal.json` (§17.1). T0: the name only.
pub struct HealSettings;

/// `heal.autonomy`: `observe`, `diagnose` or `propose` (§2.1). T0: the name only.
pub struct Autonomy;
