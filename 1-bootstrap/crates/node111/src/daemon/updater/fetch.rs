//! Manifest fetch + verification + staging decision (one poll cycle).
//!
//! Contract order (spec §Updates): fetch manifest → verify minisig → check
//! floor → only then trust any pin. Notice is internet-only in v1 (gossip is
//! Layer 7); clock faults surface as a class (minisign verification is
//! time-free — fetches degrade, verification doesn't).

use super::stage;
use crate::calver::Calver;
use crate::daemon::DaemonLayout;
use crate::floor::Floor;
use crate::manifest::ChannelManifest;
use crate::util::http::Fetcher;
use std::time::Duration;

/// Default channel endpoints (first-party only): own domain primary, GitHub
/// Releases page fallback (canonical artifact store per spec).
pub const DEFAULT_MANIFEST_BASE: &str = "https://install.111.network/stable";
pub const FALLBACK_MANIFEST_BASE: &str =
    "https://github.com/111-Network/os/releases/download/channel";

/// Target triple for payload selection (compile-time, matches the binary).
#[cfg(target_arch = "x86_64")]
pub const TARGET_TRIPLE: &str = "x86_64-unknown-linux-musl";
#[cfg(target_arch = "aarch64")]
pub const TARGET_TRIPLE: &str = "aarch64-unknown-linux-musl";

#[derive(Debug, Clone)]
pub struct UpdaterConfig {
    pub manifest_base: String,
    pub fallback_base: Option<String>,
    /// Running release calver.
    pub current_version: Calver,
    /// Fetch budget per call.
    pub timeout: Duration,
}

impl UpdaterConfig {
    pub fn production(current_version: Calver) -> Self {
        UpdaterConfig {
            manifest_base: DEFAULT_MANIFEST_BASE.to_string(),
            fallback_base: Some(FALLBACK_MANIFEST_BASE.to_string()),
            current_version,
            timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
pub enum FetchOutcome {
    UpToDate,
    Staged {
        version: Calver,
    },
    /// Manifest/sig/floor refusal — valid-looking but rejected (log-worthy).
    Refused {
        reason: String,
    },
    /// Fetch/clock problems — degraded, not refused.
    Degraded {
        reason: String,
        clock_fault: bool,
    },
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}

/// Compare the HTTP Date of a fetched response against local time. A skew
/// beyond the threshold is a clock-fault class (Pi has no RTC).
fn clock_skewed(date_header: Option<&str>) -> bool {
    let Some(h) = date_header else { return false };
    let Ok(parsed) = time::OffsetDateTime::parse(h, &time::format_description::well_known::Rfc2822)
    else {
        return false;
    };
    let skew = (time::OffsetDateTime::now_utc() - parsed).abs();
    skew > time::Duration::hours(1)
}

/// Fetch manifest+minisig from a base, verify, and return the manifest with
/// the raw bytes' sha256 and the Date header seen.
fn fetch_manifest(
    fetcher: &Fetcher,
    base: &str,
) -> anyhow::Result<(Vec<u8>, String, Option<String>)> {
    let toml_url = format!("{}/channel-111-stable.toml", base.trim_end_matches('/'));
    let sig_url = format!("{toml_url}.minisig");
    let toml = fetcher.get(&toml_url)?;
    let sig = fetcher.get(&sig_url)?;
    let sig_text = String::from_utf8(sig).map_err(|_| anyhow::anyhow!("minisig not UTF-8"))?;
    let date = fetcher.head_date(&toml_url)?;
    Ok((toml, sig_text, date))
}

/// One poll cycle. Pure-ish: all network via `Fetcher`, all disk under `layout`.
pub fn check_once(cfg: &UpdaterConfig, layout: &DaemonLayout) -> anyhow::Result<FetchOutcome> {
    let fetcher = Fetcher::new(cfg.timeout, 512 * 1024 * 1024)?;

    // 1. Fetch (primary then fallback).
    let (manifest_bytes, sig_text, date_header) = match fetch_manifest(&fetcher, &cfg.manifest_base)
    {
        Ok(v) => v,
        Err(primary_err) => {
            let Some(fallback) = &cfg.fallback_base else {
                return Ok(FetchOutcome::Degraded {
                    reason: format!("manifest fetch failed: {primary_err:#}"),
                    clock_fault: false,
                });
            };
            match fetch_manifest(&fetcher, fallback) {
                Ok(v) => v,
                Err(fallback_err) => {
                    return Ok(FetchOutcome::Degraded {
                        reason: format!(
                            "manifest fetch failed on both endpoints ({}; {})",
                            primary_err, fallback_err
                        ),
                        clock_fault: false,
                    });
                }
            }
        }
    };
    let clock_fault = clock_skewed(date_header.as_deref());
    // Clock-fault is a surfaced class regardless of outcome (spec §Updates):
    // a marker file the dashboard reads; written when skewed, cleared when not.
    set_clock_fault_marker(layout, clock_fault, date_header.as_deref())?;

    // 2. Verify (minisig + floor), then trust.
    let floor = Floor::read(&layout.state_dir().join("floor.json"))?.unwrap_or_else(Floor::initial);
    let manifest = match ChannelManifest::parse_verified(
        &manifest_bytes,
        &sig_text,
        super::MANIFEST_PUBKEY.trim(),
        &floor,
    ) {
        Ok(m) => m,
        Err(e) => {
            return Ok(FetchOutcome::Refused {
                reason: format!("{e:#}"),
            });
        }
    };

    // 3. Newer than what runs?
    if manifest.release() <= cfg.current_version {
        record_success(layout)?;
        return Ok(FetchOutcome::UpToDate);
    }

    // 4. Stage the payload for our triple.
    let Some(payload) = manifest.payload_for(TARGET_TRIPLE) else {
        return Ok(FetchOutcome::Refused {
            reason: format!("manifest ships no payload for {TARGET_TRIPLE}"),
        });
    };
    let tarball = fetcher.get(&payload.url)?;
    let manifest_sha = crate::util::hash::sha256_bytes(&manifest_bytes);
    let staged = stage::stage_payload(
        &tarball,
        payload,
        &layout.state_dir().join("staging"),
        &manifest.release(),
    )?;
    // Layer 5 cross-check (os/5-messaging/README.md §19 L1): the staged tree's
    // bin/meshmud, panel bundles and presence lock against the manifest's pins.
    // Advisory here — the root helper repeats it on the tree it extracts itself
    // (D12) — but a release whose payload disagrees with its own manifest is
    // refused now, its staging bundle removed and the helper never asked.
    if let Err(e) = manifest.verify_layer5_tree(&staged.join("tree"), TARGET_TRIPLE) {
        let _ = std::fs::remove_dir_all(&staged);
        return Ok(FetchOutcome::Refused {
            reason: format!("payload disagrees with the manifest's Layer 5 pins: {e:#}"),
        });
    }
    // The root helper re-verifies from the staging bundle alone (D12): the
    // signed channel manifest + its minisig ride alongside the tarball.
    crate::util::fs::atomic_write(
        &staged.join("channel-111-stable.toml"),
        &manifest_bytes,
        0o644,
    )?;
    crate::util::fs::atomic_write(
        &staged.join("channel-111-stable.toml.minisig"),
        sig_text.as_bytes(),
        0o644,
    )?;
    stage::write_update_request(
        &layout.state_dir(),
        &manifest.release(),
        &manifest_sha,
        &staged,
    )?;
    record_success(layout)?;
    Ok(FetchOutcome::Staged {
        version: manifest.release(),
    })
}

/// Persist last-successful-check (dashboard field + receipt of liveness).
fn record_success(layout: &DaemonLayout) -> anyhow::Result<()> {
    let body = serde_json::json!({
        "schema": 1,
        "at": now_rfc3339(),
    });
    crate::util::fs::atomic_write(
        &layout.state_dir().join("last-update-check.json"),
        body.to_string().as_bytes(),
        0o644,
    )
}

/// `state/clock-fault.json`: present ⇔ the last manifest response's Date
/// header skewed >1h from local time. Fetches degrade, verification (minisign,
/// time-free) doesn't (spec §Updates).
fn set_clock_fault_marker(
    layout: &DaemonLayout,
    skewed: bool,
    date_header: Option<&str>,
) -> anyhow::Result<()> {
    let path = layout.state_dir().join("clock-fault.json");
    if skewed {
        let body = serde_json::json!({
            "schema": 1,
            "kind": "clock-skew",
            "date_header": date_header.unwrap_or(""),
        });
        crate::util::fs::atomic_write(&path, body.to_string().as_bytes(), 0o644)
    } else {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    // Integration coverage in tests/updater_stage.rs (fixture HTTP server).
}
