//! Deterministic category table (spec "Detection & category rules"): first match
//! wins, rules 0–7 + gateway overlay. A `None` (unknown) fact makes its rule NOT
//! match — unknown is never "satisfies". The recorded category is provisional and
//! re-probed on each daemon start + udev trigger.

use super::probe::Facts;
use serde::{Deserialize, Serialize};

const GB: u64 = 1 << 30;

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

impl Refusal {
    pub fn message(self) -> &'static str {
        match self {
            Refusal::SystemdLess => {
                "systemd not found in a WSL/container environment — systemd-less containers are refused; on WSL enable systemd or pass --allow-degraded for a degraded relay"
            }
            Refusal::Armv7lKernel => {
                "armv7l (32-bit ARM kernel) hosts are not supported — a 64-bit (aarch64) OS is required"
            }
            Refusal::ThirtyTwoBitUserspaceKernel => {
                "32-bit userspace on a 32-bit kernel is not supported"
            }
        }
    }
}

fn ram_gb(f: &Facts) -> Option<u64> {
    f.ram_bytes.map(|b| b / GB)
}

/// Deterministic first-match table (spec rules 0–7 + gateway overlay).
pub fn categorize(f: &Facts) -> Result<Classification, Refusal> {
    // arch refusals (spec Refusals: armv7l hosts; 32-bit userspace is the
    // shim's warning path, not a category concern)
    if f.arch.as_deref() == Some("armv7l") {
        return Err(Refusal::Armv7lKernel);
    }

    // Rule 0: container/WSL
    let virtualized = f.in_container == Some(true) || f.is_wsl == Some(true);
    if virtualized {
        return match f.systemd {
            Some(true) => Ok(Classification {
                category: Category::Relay1,
                gateway: f.internet == Some(true),
                degraded: true,
                rule: 0,
            }),
            _ => Err(Refusal::SystemdLess),
        };
    }

    let gateway = f.internet == Some(true);
    let classify = |category: Category, rule: u8| Classification {
        category,
        gateway,
        degraded: false,
        rule,
    };

    // Rule 1: discrete GPU (10de/1002) OR (≥8 cores AND ≥32GB RAM) → 4B
    let has_discrete = f
        .gpu
        .as_ref()
        .is_some_and(|gs| gs.iter().any(|g| g.is_discrete()));
    let cores = f.cores.unwrap_or(0);
    let ram = ram_gb(f).unwrap_or(0);
    if has_discrete || (cores >= 8 && ram >= 32) {
        return Ok(classify(Category::Hub4B, 1));
    }

    // Rule 2: ≥512GB free AND ≥8GB RAM → 4A
    let free_gb = f.free_storage_bytes.map(|b| b / GB).unwrap_or(0);
    if free_gb >= 512 && ram >= 8 {
        return Ok(classify(Category::Hub4A, 2));
    }

    // Rule 3: LoRa AND (AP-WiFi OR ethernet); or AP-WiFi + ethernet → bridge
    let has_lora = f.lora.as_ref().is_some_and(|l| !l.is_empty());
    let ap = f.wifi_ap_capable == Some(true);
    let eth = f.ethernet == Some(true);
    if (has_lora && (ap || eth)) || (ap && eth) {
        return Ok(classify(Category::Bridge3, 3));
    }

    // Rule 4: Pi 5 ≥8GB → 2B
    let pi = f.pi_model.as_deref().unwrap_or("");
    if pi.contains("Raspberry Pi 5") && ram >= 8 {
        return Ok(classify(Category::Node2B, 4));
    }

    // Rule 5: battery AND (≥16GB RAM OR any GPU) → 2B
    let battery = f.battery == Some(true);
    let any_gpu = f.gpu.as_ref().is_some_and(|gs| !gs.is_empty());
    if battery && (ram >= 16 || any_gpu) {
        return Ok(classify(Category::Node2B, 5));
    }

    // Rule 6: battery present → 2A (phones: RAM via /proc/meminfo; ≥8GB ⇒ 2B)
    if battery {
        if f.is_phone && ram >= 8 {
            return Ok(classify(Category::Node2B, 6));
        }
        return Ok(classify(Category::Node2A, 6));
    }

    // Rule 7: Pi 3/4, or fallback ≥1GB RAM → 1 relay / 2A
    if pi.contains("Raspberry Pi 3") || pi.contains("Raspberry Pi 4") {
        return Ok(classify(Category::Relay1, 7));
    }
    if ram >= 1 {
        return Ok(classify(Category::Node2A, 7));
    }
    // Below 1GB on a non-Pi: smallest honest assignment.
    Ok(classify(Category::Relay1, 7))
}

#[cfg(test)]
mod tests {
    // Table-driven coverage lives in tests/categorize_table.rs (integration).
}
