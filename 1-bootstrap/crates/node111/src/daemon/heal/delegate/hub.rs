//! Layer 9 (§12.3 trust; §12.4 the hub side): T0 stub; filled by row D43 (the consumer, the queue
//! and the stored replies) and row D54 (the run).
//!
//! On a hub with `heal.serve_members` on, a `diagnose` is accepted only from an `enrolled` member
//! on an attributed connection, one open request per member and at most 4 queued in total, in
//! memory only (`refuse` 3 beyond, and while `heal` is `full` or the store closed). One delegated
//! diagnosis runs at a time, only when no local task or chat turn waits, on the live model or from
//! T0 alone; one stored reply per member answers a re-sent `req_id`, and the same `req_id` with
//! another request hash gets `refuse` 6 `request_changed`.
//!
//! T0: the name only.
#![allow(dead_code)]

/// The hub's in-memory queue of delegated requests, at most 4 (§12.3). T0: the name only.
pub struct DelegationQueue;
