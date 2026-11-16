//! Layer 8 (§5, §6, §7, §9, §16.1, §16.2): T0 stub; filled by row D29 (`GET /api/v1/datasets`:
//! `Publisher`, `DatasetEntry`, `Holding`; the drafts, publish and removal routes' state).
//!
//! Datasets: the public files a community shares (encyclopedias as ZIM files, regional maps as
//! PMTiles files), each named by its sha256 in Layer 7's object tree `state/origin/blob/` with a
//! bao-tree sidecar beside it, listed in a catalogue the publisher's node key signs, held in the
//! holdings index `state/datasets.json`, and moved by one job at a time on a named thread that
//! exists only while a job runs. A dataset is never pruned automatically (§4.6). Nothing here adds
//! a resident process, a listener, a timer or a crate.
//!
//! T0: the submodules are names only.
#![allow(dead_code)]

pub mod adopt;
pub mod catalogue;
pub mod check;
pub mod firstcopy;
pub mod index;
pub mod job;
pub mod kiwix;
pub mod metered;
pub mod publish;
pub mod pull;
pub mod search;
pub mod sidecar;

/// `GET /api/v1/datasets`'s answer: `{schema, publishers, holdings, jobs}` (§16.1, §16.2). T0: the
/// name only.
pub struct DatasetsListing;
