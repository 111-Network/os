//! Layer 9 (§1 rule 1; §3 to §14; §17): T0 stub; the module root, its submodules filled by the rows
//! their own docs name.
//!
//! Layer 9 inside `111d`: the observer, which records incidents from the events other tasks already
//! feed (`observer`, `incident`, `witness`, `facts`, `apk`); diagnosis by playbooks first and the
//! model only to choose (`playbook`, `vocab`, `diagnose`); the change engine, typed actions behind
//! a trait with a persisted undo (`action`, `actions`, `engine`, `undo`, `ledger`); what a human
//! decides (`proposal`, `settings`, `notify`); the `heal/` records in Layer 8's node-state store
//! (`store`); fixes shared between nodes (`fix`), delegation to the home hub (`delegate`) and code
//! drafts exported for a person to send (`drafts`, `export`). With every switch at its default
//! nothing leaves the node, nothing changes without a human and nothing resides: no process,
//! listener, timer or poll of its own.
//!
//! T0: the submodules are names only and nothing is started (row D10 adds the observer's spawn to
//! `daemon/mod.rs`, row D12 the start step).
#![allow(dead_code)]

pub mod action;
pub mod actions;
pub mod apk;
pub mod delegate;
pub mod diagnose;
pub mod drafts;
pub mod engine;
pub mod export;
pub mod facts;
pub mod fix;
pub mod incident;
pub mod ledger;
pub mod notify;
pub mod observer;
pub mod playbook;
pub mod proposal;
pub mod settings;
pub mod store;
pub mod undo;
pub mod vocab;
pub mod witness;
