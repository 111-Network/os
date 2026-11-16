//! Layer 9 (§3.4 the subject id space; §6.3 the one vocabulary; §13 item 1): T0 stub; filled by
//! row D3 (the table) and row D48 (the `agent` row's `prompts_invalid`).
//!
//! One compiled table of `(layer, subject, reason code)` rows: each service id with the reason
//! codes its row publishes in state `error`; `child_exit` for each supervised service id and for
//! any component; the fixed daemon subjects `111d`, `updater`, `identity`, `heal` and `storage`
//! with theirs. Nothing else is a subject, and the child codes, the `unset` codes and Layer 7's
//! `gateway` codes are not rows. A node111 test checks that every reason literal the daemon
//! publishes in `error` is in the table and writes `0-knowledge/_meta/playbook-vocabulary.toml`,
//! which `build.py` reads, so the loader and the lint cannot disagree on a subject or a code.
//! Static data only.
//!
//! T0: the name only; no table yet.
#![allow(dead_code)]

/// One row of the compiled vocabulary: a fingerprint's `(layer, subject, reason code)` (§3.4). T0:
/// the name only.
pub struct VocabRow;
