//! Layer 9 (§17.1 the verbs; §23 L1 item 19): T0 stub; filled by row L6.
//!
//! `111 heal`, run on the node like `111 fabric`: `111 heal settings [<key> <value>]` calls the
//! loopback `PATCH /api/v1/repairs/settings`, prints a setting's consent text on `409
//! consent_required`, asks y/N and sends `accept_warning` only on `y`; with no argument it prints
//! every setting. `111 heal status`, `incidents`, `fixes`, `playbooks` and `audit` make the
//! loopback GETs of §18.1 and print §18.4's shapes, so a section the Repairs page holds back stays
//! reachable on the node. On Termux, where loopback is not trusted, each reports the `401` with the
//! way forward.
//!
//! T0: usage text only; `cli.rs` does not route the verb yet (row L6 does).
#![allow(dead_code)]

/// The verb's usage lines (§17.1), for `cli.rs`'s USAGE once row L6 routes it. T0: text only.
pub const USAGE: &str = "  111 heal [status | incidents | fixes | playbooks | audit]
                                   (repairs: incidents, fixes, playbooks and the audit, via the running daemon)
  111 heal settings [<key> <value>]
                                   (a repairs setting: prints its consent text and asks y/N; no key lists them all)";
