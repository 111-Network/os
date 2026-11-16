//! Layer 9 (§6.2 the predicates' reads; §5.1 T0): T0 stub; filled by row D13.
//!
//! The evaluator's facts (`playbook::eval::Facts`, row D5) over the state `111d` already holds: a
//! child's state watch and its last exit facts, `Registry::snapshot()`, the open incident's
//! `count`, the health struct and the services probe's last code for the agent, one `metadata()`
//! under `state`, `run` or `data`, the sanitised log tail of an own child, the fabric's `statvfs`,
//! the run marker, `Components::status` and the running release. Reading a fact makes no request,
//! opens no socket and spawns nothing.
//!
//! T0: the name only.
#![allow(dead_code)]

/// The daemon's facts for the T0 evaluator (§6.2). T0: the name only.
pub struct DaemonFacts;
