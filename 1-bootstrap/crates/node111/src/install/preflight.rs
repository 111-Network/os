//! Step 1 — preflight (spec §Install flow Linux): systemd present, privilege
//! contract, free space ≥ (payload + packages) ×2, clock sanity vs HTTP Date,
//! connectivity (HTTPS HEAD on real artifact endpoints + HTTP-204 ×2 + DNS +
//! default route), WSL/container policy. Fetches + verifies the channel
//! manifest (network is guaranteed at install time) — later steps reuse it.

use super::context::InstallContext;
use super::probe;
use crate::floor::Floor;
use crate::manifest::ChannelManifest;

/// First-party probe endpoints (spec §Security: probes hit first-party only;
/// zero telemetry = no data beyond standard HTTP access logs).
pub const PROBE_204_PRIMARY: &str = "https://install.111.network/204";
pub const PROBE_204_FALLBACK: &str = "https://static.111.network/204";

/// Space headroom beyond the pinned payload: packages (~700MB) ×2 safety.
const PACKAGES_BUDGET_BYTES: u64 = 2 * 700 * 1024 * 1024;

#[derive(Debug)]
pub struct PreflightReport {
    pub manifest: ChannelManifest,
    pub manifest_bytes: Vec<u8>,
    pub manifest_sig: String,
    pub degraded: bool,
    pub distro: String,
}

fn refuse(msg: impl Into<String>) -> anyhow::Error {
    anyhow::anyhow!(
        "install refused: {} — see https://111.network/docs/install",
        msg.into()
    )
}

pub fn run(
    ctx: &InstallContext,
    fetcher: &crate::util::http::Fetcher,
) -> anyhow::Result<PreflightReport> {
    // --- systemd present (hard refusal for non-systemd distros) ---
    let systemd_dir = ctx.env.read_dir("/run/systemd/system").is_ok();
    let init = ctx.env.read_file("/proc/1/comm").unwrap_or_default();
    anyhow::ensure!(
        systemd_dir && init.trim() == "systemd",
        refuse("systemd not found (/run/systemd/system + PID 1) — non-systemd distros are refused per spec")
    );

    // --- privilege contract: root (the shim escalates; a bare `111 install`
    //      must fail loudly and point at the shim/sudo) ---
    if ctx.check_root {
        let uid = unsafe { libc::geteuid() };
        anyhow::ensure!(
            uid == 0,
            refuse("the installer needs root — re-run via the shim (curl … | sh) or sudo")
        );
    }

    // --- environment facts needed for WSL/container policy ---
    let facts = probe::collect(ctx.env);

    // --- connectivity: HTTPS HEAD on the real artifact endpoint + 204×2 + DNS + route ---
    let artifact_probe = format!(
        "{}/channel-111-stable.toml",
        ctx.manifest_base.trim_end_matches('/')
    );
    fetcher
        .head_date(&artifact_probe)
        .map_err(|e| refuse(format!("no connectivity to artifact endpoint ({e:#})")))?;
    let mut ok_204 = 0;
    for probe_url in &ctx.probe_204 {
        match fetcher.head_date(probe_url) {
            Ok(_) => ok_204 += 1,
            Err(e) => eprintln!("111 install: 204 probe {probe_url} failed: {e:#}"),
        }
    }
    anyhow::ensure!(
        ok_204 >= 1,
        refuse("HTTP-204 probes failed on all endpoints")
    );
    // DNS resolution evidence: the HTTPS fetches above already prove DNS; default route:
    let route = ctx.env.read_file("/proc/net/route").unwrap_or_default();
    anyhow::ensure!(
        route
            .lines()
            .skip(1)
            .any(|l| l.split_whitespace().nth(1) == Some("00000000")),
        refuse("no default route")
    );

    // --- clock sanity: HTTP Date vs local (loud refusal; Pi has no RTC) ---
    if let Ok(Some(date)) = fetcher.head_date(&artifact_probe) {
        if let Ok(parsed) =
            time::OffsetDateTime::parse(&date, &time::format_description::well_known::Rfc2822)
        {
            let skew = (time::OffsetDateTime::now_utc() - parsed).abs();
            anyhow::ensure!(
                skew <= time::Duration::hours(1),
                refuse(format!(
                    "system clock skewed {}s vs HTTP Date — fix the clock (no RTC?)",
                    skew.whole_seconds()
                ))
            );
        }
    }

    // --- fetch + verify the channel manifest (order: minisig → floor → trust) ---
    let (manifest_bytes, manifest_sig) = fetch_signed_manifest(ctx, fetcher)?;
    let floor = Floor::read(&ctx.state_dir().join("floor.json"))?.unwrap_or_else(Floor::initial);
    let manifest = ChannelManifest::parse_verified(
        &manifest_bytes,
        &manifest_sig,
        crate::daemon::updater::MANIFEST_PUBKEY.trim(),
        &floor,
    )
    .map_err(|e| refuse(format!("channel manifest failed verification: {e:#}")))?;
    anyhow::ensure!(
        manifest.release() >= ctx.min_installer_version,
        refuse(format!(
            "manifest release {} below compiled-in minimum {} (stale shim?)",
            manifest.release(),
            ctx.min_installer_version
        ))
    );

    // --- free space ≥ (payload + packages) ×2 ---
    let payload_size = manifest
        .payload_for(crate::daemon::updater::fetch::TARGET_TRIPLE)
        .map(|p| p.size)
        .unwrap_or(0);
    let need = payload_size * 2 + PACKAGES_BUDGET_BYTES;
    let free = probe::free_bytes(&ctx.layout.root).unwrap_or(u64::MAX);
    anyhow::ensure!(
        free >= need,
        refuse(format!(
            "free space {}MB < required {}MB (payload+packages)×2",
            free / (1 << 20),
            need / (1 << 20)
        ))
    );

    // --- WSL/container policy ---
    let virtualized = facts.in_container == Some(true) || facts.is_wsl == Some(true);
    let degraded = if virtualized {
        if ctx.allow_degraded {
            true
        } else {
            eprintln!(
                "111 install: WARNING: {} detected — installing as a DEGRADED relay (capabilities stripped). \
                 Pass --allow-degraded to silence, or install on bare metal for a full node.",
                if facts.is_wsl == Some(true) { "WSL" } else { "container" }
            );
            true
        }
    } else {
        false
    };

    let distro = facts.distro.clone().unwrap_or_else(|| "unknown".into());
    Ok(PreflightReport {
        manifest,
        manifest_bytes,
        manifest_sig,
        degraded,
        distro,
    })
}

/// Fetch manifest + minisig (primary then fallback).
fn fetch_signed_manifest(
    ctx: &InstallContext,
    fetcher: &crate::util::http::Fetcher,
) -> anyhow::Result<(Vec<u8>, String)> {
    let bases: Vec<&str> = std::iter::once(ctx.manifest_base.as_str())
        .chain(ctx.manifest_fallback.as_deref())
        .collect();
    for base in bases {
        let toml_url = format!("{}/channel-111-stable.toml", base.trim_end_matches('/'));
        match (
            fetcher.get(&toml_url),
            fetcher.get(&format!("{toml_url}.minisig")),
        ) {
            (Ok(toml), Ok(sig)) => {
                let sig_text =
                    String::from_utf8(sig).map_err(|_| anyhow::anyhow!("minisig not UTF-8"))?;
                return Ok((toml, sig_text));
            }
            (Err(e), _) | (_, Err(e)) => eprintln!("111 install: manifest fetch {toml_url}: {e:#}"),
        }
    }
    Err(refuse("channel manifest unreachable on all endpoints"))
}
