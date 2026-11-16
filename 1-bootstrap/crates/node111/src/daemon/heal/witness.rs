//! Layer 9 (§3.3 rules 1, 5 and 7; §4.1 rule 1; Layer 8's J2): T0 stub; filled by row D12 (the run
//! marker and the start facts) and row D16 (the J2 fold and the open incidents at start).
//!
//! `state/heal/run` = `{schema: 1, boot_id, release, clean, starts}`, atomic 0600: written at start
//! before serving (`clean: false`, `starts` the old value plus one) and rewritten `clean: true` at
//! a graceful stop. At start, after `node/lock`, the marker is read without rewriting it, with the
//! inbox and the update tombstone; the undo file is reconciled (row D21); then the marker is
//! rewritten. Each inbox line is classified alone, oldest first (`watchdog_kill`, `killed_by_os`,
//! `unclean_exit`; a `code=75` line opens nothing and is never counted as a crash), and the marker
//! and the tombstone by their own table (`unclean_exit`, `unclean_boot`, `update_reverted`,
//! `release_reverted_at_start`). Without the witness (the unit's `ExecStopPost=` line, the runit
//! `finish` hook) the start classifies from the marker alone and opens `witness_absent`. Layer 9's
//! folder writes the start facts and the lines into `heal/`; Layer 8 clears the files only after
//! that write committed.
//!
//! T0: the names only; nothing is read or written.
#![allow(dead_code)]

/// The run marker `state/heal/run` (§3.3 rule 1). T0: the name only.
pub struct RunMarker;

/// One line of `state/heal/inbox` (§3.3 rule 4). T0: the name only.
pub struct InboxLine;

/// What the start step read: the marker, the inbox lines and the update tombstone (§3.3 rule 5).
/// T0: the name only.
pub struct StartFacts;
