//! Deterministic category table (spec "Detection & category rules"): first match
//! wins, rules 0–7 + gateway overlay. The recorded category is provisional and
//! re-probed on each daemon start + udev trigger.

use super::probe::Facts;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Category {
    Relay1,
    Node2A,
    Node2B,
    Bridge3,
    Hub4A,
    Hub4B,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Classification {
    pub category: Category,
    /// Overlay 5: internet up at detect time (recorded intent for later layers).
    pub gateway: bool,
    /// True when running as a degraded relay (systemd-ful WSL/container with
    /// --allow-degraded).
    pub degraded: bool,
    /// Which spec rule matched (0–7), for receipt + dashboard transparency.
    pub rule: u8,
}

/// Hard refusals from rule 0 / arch rules (spec Refusals section).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    SystemdLess,
    Armv7lKernel,
    ThirtyTwoBitUserspaceKernel,
}

pub fn categorize(f: &Facts) -> Result<Classification, Refusal> {
    let _ = f;
    todo!("T4")
}
