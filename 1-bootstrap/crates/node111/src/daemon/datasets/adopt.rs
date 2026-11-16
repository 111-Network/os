//! Layer 8 (§7.6 "One writer"; §16.1; §5.1): T0 stub; filled by row D30.
//!
//! The adopt hand-off: `POST /api/v1/datasets/adopt` (loopback only, never a session, never the
//! bearer) takes what a dataset verb staged under `state/origin/import.lock` (the `incoming/`
//! copies, the catalogue copies, the drafts, the in-place files and the request file
//! `adopt-<n>.json`), checks each item again (each staged catalogue's signature and `seq`, each
//! in-place file's tuple), and only then writes the tree, `state/origin/catalogue/`, the drafts,
//! `state/datasets.json` and `state/storage.json` and rebuilds the kiwix library: `111d` is the
//! only writer of these while it runs. A failing item writes nothing.
#![allow(dead_code)]

/// One adopt request (§16.1). T0: the name only.
pub struct AdoptRequest;
