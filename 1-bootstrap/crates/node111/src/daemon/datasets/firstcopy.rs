//! Layer 8 (§7.5 (1); §7.3 (1); §7.11): T0 stub; filled by row D33.
//!
//! The first copy from upstream, a job kind on the `111d-dataset` thread: the upstream file in
//! consecutive 16 MiB windows, one `get_to_file` call each sending `If-Range`; then the supplied
//! sha256 checked, and BLAKE3 and the sidecar computed in the same pass; both placed in the tree
//! (never replacing) and a draft written. Only then may the owner publish.
#![allow(dead_code)]

/// One first copy from an upstream URL (§7.5). T0: the name only.
pub struct FirstCopy;
