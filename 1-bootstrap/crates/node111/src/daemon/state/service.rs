//! Layer 8 (§3 J9, J3, J4; §2.2 rule 4): T0 stub; filled by row D3.
//!
//! The state service API, in process only (Layer 9's modules call it; no route; the agent never
//! does): `put`, `get`, `list`, `append`, `read_stream`, `set_state`, `delete`, `delete_stream`,
//! `trim_stream`, `usage`, `reclaim`, `decision_begin` (`FULL`), `decision_complete` (`FULL`) and
//! `decision_reopen`. Each call is one transaction; every write takes a new `seq` and returns it;
//! `list` and `read_stream` return at most `limit` rows after the `seq` cursor; `reclaim` calls the
//! reclaimer the owning module registered per namespace, writes the J3 marker for `heal` and ends
//! with `PRAGMA incremental_vacuum`; a write to a `full` class returns `Full`, except the three
//! decision calls and a spool ingest.
#![allow(dead_code)]

/// The J9 state service over the node-state store (§3 J9). T0: the name only.
pub struct StateService;
