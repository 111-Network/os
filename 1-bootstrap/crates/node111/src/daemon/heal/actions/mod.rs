//! Layer 9 (§7.1, §7.2): T0 stub; filled by row D19 (the dispatch of the action type to its
//! implementation) and rows D27 to D30 (one arm each).
//!
//! The typed actions of §7.2's closed vocabulary behind the engine's trait: the four device actions
//! ported from the Layer 6 applier byte for byte (`device`), `service.restart` (`service`),
//! `settings.set` on the fabric keys (`settings_set`), `component.enable` and `component.disable`
//! (`component`) and the three bridge actions, human or `device.propose` only (`bridge`).
//!
//! T0: the submodules are names only.
#![allow(dead_code)]

pub mod bridge;
pub mod component;
pub mod device;
pub mod service;
pub mod settings_set;
