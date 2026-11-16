//! Layer 9 (§10, §11): T0 stub; the module root of fix sharing, its submodules filled by rows D7
//! (`body`), D36 and D39 (`send`), D37 and D39 (`receive`) and D38 (`peer`).
//!
//! A proven typed fix, signed with this node's key through `111d` (`111/fix/v1`), sent by a human
//! to the home hub or a contact in one sealed `PATCH` (0x13) and taken only from the senders the
//! receiving human set; there it becomes a proposal only while the same incident is open, and only
//! for the peer subset. Accepting one sends its author one signed vouch (`111/fix-vouch/v1`).
//!
//! T0: the submodules are names only.
#![allow(dead_code)]

pub mod body;
pub mod peer;
pub mod receive;
pub mod send;
