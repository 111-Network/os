//! Layer 9 (§12.1 the op set): T0 stub; filled by row D8.
//!
//! The `AGENT_OP` payload: `0x01` (the schema) ‖ op ‖ a deterministic CBOR map (RFC 8949 §4.2.1)
//! with unsigned integer keys, encoded and decoded with `ciborium`: `diagnose` (keys 0 to 9, each
//! capped), `diagnosis` (keys 0 to 6) and `refuse` (a reason from 1 to 6). A map not in the
//! deterministic encoding (keys out of order, a duplicate key, an indefinite length, a non-shortest
//! integer) is refused, dropped and counted, never re-encoded; no key carries a log line.
//!
//! T0: the name only.
#![allow(dead_code)]

/// One `AGENT_OP` op: `diagnose`, `diagnosis` or `refuse` (§12.1). T0: the name only.
pub struct AgentOp;
