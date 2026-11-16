//! Layer 8 (§13.7; §2.2 rule 9; §3 J4): T0 stub; filled by row D18.
//!
//! The integrity checks after an unclean shutdown: `PRAGMA quick_check` and `PRAGMA
//! cipher_integrity_check` run once when a store's unclean signal is present at its open (the
//! fabric store's absent clean record; the node-state store's non-empty `state.db-wal`), never on a
//! clean boot, on a second, read-only connection on a blocking thread while the store already
//! serves; `integrity.<store>` reads `checking`, then `ok` or `failed`. On failure the store is
//! closed and moved aside to `<file>.damaged-<n>`; never a silent re-creation.
//!
//! T0: [`status`] is empty: no check runs.
#![allow(dead_code)]

/// One store's check (§16.2 `integrity`: `checking`, `ok` or `failed`; a store with no check due has
/// none). T0: the name only.
pub struct StoreIntegrity;

/// The checks due or run since this start (§13.7). T0: none.
pub fn status() -> Vec<StoreIntegrity> {
    Vec::new()
}
