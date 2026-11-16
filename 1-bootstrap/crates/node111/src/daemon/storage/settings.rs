//! Layer 8 (§15.1): T0 stub; filled by row D9.
//!
//! `state/storage.json`: JSON `{"schema": 1, …}`, 0600, written atomically and only on a change;
//! absent ⇒ every default; not a small regular file or not JSON ⇒ moved aside to
//! `storage.json.invalid-<n>` and every default (the `state/relay.json` rule); an unknown key kept,
//! ignored and written back, so a rollback never drops a consent. Keys: `keep_bytes`,
//! `keep_default`, `caps.agent`, `caps.heal`, `logs.max_file_bytes`, `follow`, `sources`, the
//! `origin.*` switches with their consent hashes, `update_without_copy`. No file until the first
//! real change.
#![allow(dead_code)]

/// Layer 8's settings (§15.1). T0: the name only.
pub struct StorageSettings;
