//! Layer 9 (§6.1 the entry; §6.3 the loader): T0 stub; filled by row D4 (the loader), with
//! `grammar.rs` (row D4) and `eval.rs` (row D5).
//!
//! `111d` reads `current/share/kb/playbooks/*.md` once at start, extracts the one `toml` block
//! under `### Checks` and keeps only each playbook's `[match]` key and its load state (`loaded`, or
//! `refused` with "unsupported grammar N, needs release X"), the Repairs page's list. A playbook is
//! parsed again when an incident with its fingerprint opens and dropped when none is open, so an
//! idle node holds no parsed playbook. The filename is the one rendered from `[match]`:
//! `repair-<subject>-<reason>.md`.
//!
//! T0: the names only; nothing is read.
#![allow(dead_code)]

pub mod eval;
pub mod grammar;

/// One playbook's `[match]` key and load state, as the start scan keeps it (§6.3). T0: the name
/// only.
pub struct PlaybookEntry;
