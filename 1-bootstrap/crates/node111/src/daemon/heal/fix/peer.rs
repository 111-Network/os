//! Layer 9 (§11.4 steps 6 and 7; §12.5's subset; §14.5): T0 stub; filled by row D38.
//!
//! The peer subset: of a received fix's actions only a `service.restart` of the open incident's own
//! subject, an `advise` shown as this node's own playbook text for the same `cause_id`, and an
//! action byte-equal to this node's own playbook action for that cause (its targets bound by
//! `@subject` here) reach a button; anything else is "their words, not checked", preformatted. A
//! fix files a `fix` proposal only while an incident with its fingerprint is open; otherwise it
//! waits as "does not apply here yet" and is listed once when one opens.
//!
//! T0: the name only.
#![allow(dead_code)]

/// How one peer action is shown: `action` or `their_words` (§11.4 step 6, §18.4 `Fix`). T0: the
/// name only.
pub struct ShownAs;
