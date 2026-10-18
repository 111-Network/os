//! 111 — the 111 Network node binary: installer, daemon, identity, config, uninstall.
//! Module tree frozen at T0 per os/bootstrap/TASKS.md; stubs are filled by their
//! owning tasks. Cross-module contracts live in the pinned types (see TASKS.md).

/// Compiled-in minimum installer version (calver). Stale-shim redirects cannot
/// downgrade an install below this floor; release machinery bumps it (T15).
pub const MIN_INSTALLER_VERSION: &str = "2026.10.01";

pub mod calver;
pub mod cli;
pub mod config;
pub mod daemon;
pub mod floor;
pub mod identity;
pub mod install;
pub mod manifest;
pub mod minisign;
pub mod probation;
pub mod testkit;
pub mod uninstall;
pub mod update_guard;
pub mod update_helper;
pub mod util;
