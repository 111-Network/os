//! Layer 9 (§6.2 a cause's `action`; §7.2 the v1 vocabulary): T0 stub; filled by row D4.
//!
//! §7.2's closed vocabulary as data: each typed action by its name with its fields, tagged on
//! `action` (the Layer 6 applier's `{"action": name, …args}` form), plus `advise {text}`,
//! `human_step {steps}` and `escalate {task}` (`task` `diagnose` or `patch`); every table
//! `deny_unknown_fields`; `"@subject"` the one non-literal, bound to the incident's subject from
//! the compiled subject table. `firmware.flash` is never typed; the three bridge actions and the
//! two interface actions are marked human or `device.propose` only, so a playbook cause naming one
//! is refused. Playbooks, `repair` and `fix` proposals and received fixes parse into this one type.
//!
//! T0: the name only.
#![allow(dead_code)]

/// One action of §7.2's closed vocabulary, or `advise`, `human_step` or `escalate` (§6.2). T0: the
/// name only.
pub struct Action;
