//! Layer 9 (§8.3, §8.4 code drafts; §9 the export; §13 item 4 `kb_draft`): T0 stub; filled by rows
//! D46 (code drafts on an opted-in hub), D47 (the drafts routes and the export) and D49
//! (`kb_draft`, the daemon's half).
//!
//! Accepting a `patch` proposal on a hub with `heal.code_drafts` on creates a `heal/drafts` record
//! of class `code`; accepting a `kb_draft` one creates class `kb`, its `target` validated again
//! first. Nothing is applied anywhere: a draft reads "drafted and applies cleanly, not built" or
//! "drafted, does not apply" and is exported as `fix-<id>.mbox` and `PR.md` for the human to send.
//! A `patch` task is submitted only when a source tarball whose sha256 is the running release's
//! `[source]` pin is present; otherwise the incident gets the `human_step` naming `sudo 111 install
//! --components opencode`.
//!
//! T0: the name only.
#![allow(dead_code)]

/// One `heal/drafts` record (§18.4 `Draft`). T0: the name only.
pub struct Draft;
