//! Layer 8 (§13.6 (1); §17 "A store swapped at rollback"): T0 stub; filled by row M6.
//!
//! The migration copy (amends L5 §10.3, additive): before any migration of a store whose
//! `user_version` is above 0 and below the build's, `wal_checkpoint(TRUNCATE)`, a copy to
//! `<file>.pre-v<N>.tmp` (same key, owner, 0600) and an `fsync`; for the fabric store the copy
//! opened through `open_encrypted` and its `settings.clean` deleted, so it carries no clean record;
//! then the rename to `<file>.pre-v<N>`, an `fsync` of the directory, and the migration. A copy cut
//! short is only ever a `.tmp`, never renamed in. Wired before `migrate` in this store's open and
//! exposed for the node-state store (node111's row D17); a caller flag skips it under an accepted
//! `update_without_copy`.
//!
//! T0: the name only; nothing is copied.
#![allow(dead_code)]

/// The pre-migration copy of one store (§13.6 (1)). T0: the name only.
pub struct MigrationCopy;
