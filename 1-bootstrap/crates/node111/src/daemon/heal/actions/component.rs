//! Layer 9 (§7.2 `component.enable` and `component.disable`; §2.2 item 5): T0 stub; filled by row
//! D29.
//!
//! `component.enable {id}` through `enable_component`, addressed by component id, with the
//! component's own consent and cached payload (one not cached becomes a `human_step` naming `sudo
//! 111 install --components <id>`), probed `running` and healthy; `component.disable {id}` through
//! `stop_and_wait`. Each is the other's undo; a licence notice that is not current stops the revert
//! `revert_blocked`.
//!
//! T0: the name only.
#![allow(dead_code)]

/// `component.enable` and `component.disable` (§7.2). T0: the name only.
pub struct ComponentToggle;
