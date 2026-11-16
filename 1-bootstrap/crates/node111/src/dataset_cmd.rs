//! Layer 8 (§7.6; §5.1; §4.6): T0 stub; filled by row L6 (`111 dataset add` and the verbs' shared
//! module), row L6b (`import --from <dir>`) and row L8 (`import --from <url>`).
//!
//! The dataset verbs, run once as the node user: each takes an `flock` on
//! `state/origin/import.lock`, checks the §4.6 fit before the first byte, verifies and copies into
//! `state/origin/incoming/`, then hands the result to the running daemon on loopback (`POST
//! /api/v1/datasets/adopt`), which alone writes the tree, the catalogue, the drafts and the two
//! JSON files; with the daemon stopped (`node/lock` free) the verb writes them itself under both
//! locks. `add` (hubs only) computes sha256, BLAKE3 and the sidecar in one pass and writes a draft;
//! `import --from <dir|url>` verifies the catalogue (an unfollowed publisher is refused without
//! `--follow`), each sidecar and every block; `--in-place` (hubs only) registers a file on an
//! attached disk with one hash pass and copies no data.
//!
//! T0: usage text only; `cli.rs` does not route the verbs yet (rows L6, L6b and L8 do).
#![allow(dead_code)]

/// The verbs' usage lines (§7.6), for `cli.rs`'s USAGE once row L6 routes them. T0: text only.
pub const USAGE: &str =
    "  111 dataset add <file> --id --kind --name --version --licence --attribution --source-url
                  [--sha256 HEX] [--ob-log N] [--in-place]   (hubs only)
  111 dataset import --from <dir|url> [--in-place] [--follow [<publisher hex>]]
                                   (run as the node user; --in-place on hubs only)";
