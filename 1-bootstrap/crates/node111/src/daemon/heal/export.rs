//! Layer 9 (§9 items 1, 3 and 4; §14.5): T0 stub; filled by row D9.
//!
//! The pure renderer of a draft's export: `fix-<id>.mbox` (git's fixed `From 0000…` first line,
//! `From: <node label> <node@111.invalid>`, `Subject: [PATCH] <title>`, the body, `Assisted-by:`
//! trailers, the diff, `base-commit:`), `PR.md` and the phone's issue URL, built from typed fields
//! only. Model and peer text never shapes the artefact: the subject is one line of at most 72 B, a
//! body line starting `From ` is written `>From `, a trailer-shaped line is indented, untrusted
//! text sits in a fence longer than any backtick run inside it, and no `Signed-off-by:` is ever
//! ours. The node is named by its short label unless the human ticks the NodeId.
//!
//! T0: the name only.
#![allow(dead_code)]

/// The rendered export of one draft: the mbox and `PR.md` (§9). T0: the name only.
pub struct Export;
