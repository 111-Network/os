//! Layer 8 (§2.2 rules 4 and 5): T0 stub; filled by row D2.
//!
//! The node-state store's schema v1, with its own `user_version`, migrated append-only through
//! mesh111's `migrate_with`: the STRICT tables `meta` (`next_seq`, the export offers per class, the
//! prune markers; no clean record), `records` (documents keyed by `ns` and `id`, with the (`ns`,
//! `state`, `seq`) index), `entries` (append streams) and `ns_totals` (rows and bytes per
//! namespace, kept by triggers inside the writing transaction). `seq` comes from one allocator and
//! every write takes a new one; `boot_id` and `mono_ms` sit beside it and `wall` orders nothing.
//! The v1 namespaces: `agent/conversations`, `agent/proposals`, `agent/tasks`, `agent/decisions`,
//! `heal/` and `storage/`.
#![allow(dead_code)]

/// The node-state store's migration list for `migrate_with` (schema v1 first). T0: the name only.
pub struct Migrations;
