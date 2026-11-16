//! Layer 8 (§13.2, §13.1): T0 stub; filled by row L3.
//!
//! `111 store export --out <file>` and `111 store import --from <file>`: run as the node user on a
//! local terminal, never over HTTP, with the daemon stopped (both take `node/lock` without waiting
//! and refuse while `111d` holds it: "Stop the node first: sudo systemctl stop 111node"). Export
//! opens each store with its key through mesh111's `open_encrypted`, checkpoints its WAL, reads
//! `user_version` and tars `manifest.toml`, both stores and the fabric's attachment files, sealed
//! and signed in `format.rs`'s layout; a store that does not open is refused, never skipped. Import
//! verifies before decrypting, refuses another NodeId and a store newer than the build writes, sets
//! each existing store aside as `<file>.replaced-<n>` and places the export's files.
//!
//! T0: usage text only; `cli.rs` does not route the verbs yet (row L3 does).
#![allow(dead_code)]

pub mod format;

/// The verbs' usage lines (§13.2), for `cli.rs`'s USAGE once row L3 routes them. T0: text only.
pub const USAGE: &str = "  111 store export --out <file>
  111 store import --from <file>   (run as the node user with the node stopped: sudo systemctl stop 111node)";
