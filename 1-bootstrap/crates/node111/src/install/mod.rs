//! Linux host installer state machine + phone provisioning. Every step probes
//! content (hashes, markers), never mere existence, and resumes at the failed step.

pub mod categorize;
pub mod context;
pub mod offer;
pub mod packages;
pub mod phone;
pub mod preflight;
pub mod probe;
pub mod receipt;
pub mod stage;
pub mod supervise;
pub mod verify;

use context::InstallContext;
use receipt::Receipt;

/// Entry point for `111 install` / `111 install --phone` (production).
pub fn run(args: crate::cli::InstallArgs) -> anyhow::Result<()> {
    if args.phone {
        return phone::run(args);
    }
    let layout = crate::daemon::DaemonLayout {
        root: crate::util::paths::root(),
        etc: std::path::PathBuf::from("/etc/111"),
    };
    let runner = crate::util::run::SystemRunner;
    let sysenv = probe::SystemEnv;
    let ctx = InstallContext {
        layout,
        fs_root: std::path::PathBuf::from("/"),
        runner: &runner,
        env: &sysenv,
        allow_degraded: args.allow_degraded,
        min_installer_version: args.min_installer_version.unwrap_or_else(|| {
            crate::MIN_INSTALLER_VERSION
                .parse()
                .expect("compiled-in MIN_INSTALLER_VERSION is a valid calver")
        }),
        // Operator override (mirrors, air-gapped channels, e2e rigs):
        // NODE111_MANIFEST_BASE (+ NODE111_PROBE_204, comma-separated).
        manifest_base: std::env::var("NODE111_MANIFEST_BASE")
            .unwrap_or_else(|_| crate::daemon::updater::fetch::DEFAULT_MANIFEST_BASE.to_string()),
        manifest_fallback: Some(crate::daemon::updater::fetch::FALLBACK_MANIFEST_BASE.to_string()),
        health_base: "http://127.0.0.1:11180".to_string(),
        probe_204: std::env::var("NODE111_PROBE_204")
            .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_else(|_| {
                vec![
                    preflight::PROBE_204_PRIMARY.to_string(),
                    preflight::PROBE_204_FALLBACK.to_string(),
                ]
            }),
        check_root: true,
    };
    let fetcher =
        crate::util::http::Fetcher::new(std::time::Duration::from_secs(60), 512 * 1024 * 1024)?;
    if let Some(ids) = &args.components {
        anyhow::ensure!(
            unsafe { libc::geteuid() } == 0,
            "111 install --components needs root (the cache is root-owned) — re-run it with sudo"
        );
        fetch_components_with(
            &ctx,
            &fetcher,
            &crate::daemon::components::manifest::load_embedded(),
            ids,
            &mut std::io::stderr(),
        )?;
        eprintln!(
            "111 install: cached {}; enable it from the dashboard (nothing is installed or started until you do)",
            ids.join(", ")
        );
        return Ok(());
    }
    let receipt = run_with(&ctx, &fetcher)?;
    verify::print_success(
        &ctx.layout,
        &receipt.node_id,
        &receipt.category,
        &receipt.version,
    );
    Ok(())
}

/// The state machine (spec §Install flow Linux, steps 1–9). Testable: every
/// system touch goes through ctx (Runner/Env/paths) and fetcher. The optional
/// components are this binary's embedded declarations
/// ([`run_with_components`]).
pub fn run_with(
    ctx: &InstallContext,
    fetcher: &crate::util::http::Fetcher,
) -> anyhow::Result<Receipt> {
    run_with_components(
        ctx,
        fetcher,
        &crate::daemon::components::manifest::load_embedded(),
    )
}

/// [`run_with`] with the optional-component declarations step 5 caches
/// (Layer 6 §2.1 rule 3) passed in, so a test harness decides which payloads
/// its fetcher is asked for. A declaration with `[install] prefetch = false`
/// is never downloaded here (one line names the command that fetches it).
pub fn run_with_components(
    ctx: &InstallContext,
    fetcher: &crate::util::http::Fetcher,
    components: &[crate::daemon::components::manifest::Component],
) -> anyhow::Result<Receipt> {
    // 0. base tree + existing receipt (untrusted hint) + version gate
    for d in [
        ctx.layout.state_dir(),
        ctx.layout.root.join("releases"),
        ctx.layout.root.join("cache/packages"),
    ] {
        std::fs::create_dir_all(&d)?;
    }
    let prior = Receipt::read(&ctx.layout.root.join("receipt.json"))?;

    // 1. preflight (fetches + verifies the channel manifest)
    let pre = preflight::run(ctx, fetcher)?;
    stage::version_gate(prior.as_ref(), &pre.manifest)?;

    // 2+3. probe + categorize (+ gateway overlay: internet proven by preflight)
    let (_facts, class) = {
        let mut facts = probe::collect(ctx.env);
        facts.internet = Some(true);
        let class = categorize::categorize(&facts)
            .map_err(|r| anyhow::anyhow!("install refused: {}", r.message()))?;
        (facts, class)
    };

    // 4. identity (never overwrite; a D9-format dir migrates on load; corruption
    //    quarantines + instructs restore — silent keygen only when Missing)
    let identity_dir = ctx.layout.identity_dir();
    let identity = match crate::identity::load(&identity_dir) {
        Ok(id) => id,
        Err(e) if crate::identity::is_missing(&e) => crate::identity::generate(&identity_dir)?,
        Err(e) => {
            return Err(e.context(
                "identity failed integrity check (quarantined where corrupt) — run `111 identity restore` explicitly",
            ));
        }
    };

    // 5. packages + caches (receipt accumulates hashes, notices + degrade branches)
    let mut receipt = Receipt::new(&pre.manifest.release().to_string());
    let family = packages::Family::of(&pre.distro);
    packages::run(ctx, family, &mut receipt)?;
    let lockfile = stage::fetch_rns_lock(ctx, fetcher, &pre.manifest)?;
    // Layer 6 §2.1 rule 3, D26 #5: one notice line per third-party program,
    // printed and recorded in the receipt before its download
    packages::announce(
        &packages::rns_notice(),
        &mut receipt,
        &mut std::io::stderr(),
    );
    packages::pip_closure(ctx, &lockfile, &mut receipt)?;
    // optional components declared in this binary (assets/components/):
    // notices, then their payloads into cache/packages/<id>/ (never fatal;
    // nothing is installed until a human enables one)
    packages::cache_components(
        ctx,
        components,
        fetcher,
        &mut receipt,
        &mut std::io::stderr(),
    );

    // 6. stage (payload → releases/, flip current, initial floor, symlink,
    //    presence venv — never fatal; a venv it could not build is a degrade
    //    branch)
    let release_dir = stage::run(ctx, fetcher, &pre.manifest)?;
    if let Some(branch) = presence_degrade(ctx, &pre.manifest, &release_dir) {
        receipt.degrade_branches.push(branch);
    }

    // 7. supervise (user, units, udev, ownership, enable --now)
    supervise::run(ctx, pre.degraded)?;

    // 8. verify (/health ≤60s + loopback assert)
    verify::wait_healthy(ctx, 60)?;

    // 9. receipt + offer (intent stub; downloads nothing)
    let cache_root = ctx.layout.root.join("cache");
    packages::record_cache_hashes(&cache_root, &mut receipt)?;
    receipt.category = format!("{:?}", class.category);
    receipt.gateway = class.gateway;
    receipt.node_id = identity.node_id_hex();
    receipt.write(&ctx.layout.root.join("receipt.json"))?;
    if offer::offer_standard(ctx)? {
        offer::print_offer();
    }

    Ok(receipt)
}

/// `111 install --components <ids>` (Layer 6 §2.1 rule 3; the payloads
/// `[install] prefetch = false` leaves out of the base install): on an
/// installed node, print each named component's licence notice and record it,
/// then download its pinned payload into `cache/packages/<id>/` (size and
/// SHA-256 against the declaration in this signed binary), and rewrite the
/// receipt with the new notices and cache hashes. Nothing is installed or
/// started. Refuses before any download: a node without a receipt (run
/// `111 install` first), an id that is not declared, or one with nothing to
/// download on this machine. A failed download (or a file failing its pin)
/// is an error after the receipt is written (the notices printed stay
/// recorded); nothing of it lands in the cache.
pub fn fetch_components_with(
    ctx: &InstallContext,
    fetcher: &dyn packages::PayloadFetcher,
    components: &[crate::daemon::components::manifest::Component],
    ids: &[String],
    out: &mut dyn std::io::Write,
) -> anyhow::Result<Receipt> {
    let receipt_path = ctx.layout.root.join("receipt.json");
    let mut receipt = Receipt::read(&receipt_path)?.ok_or_else(|| {
        anyhow::anyhow!("111 is not installed on this machine — run `111 install` first")
    })?;
    anyhow::ensure!(!ids.is_empty(), "--components names at least one component");
    let mut chosen = Vec::new();
    for id in ids {
        let c = components.iter().find(|c| &c.id == id).ok_or_else(|| {
            let known: Vec<&str> = components.iter().map(|c| c.id.as_str()).collect();
            anyhow::anyhow!(
                "no component {id:?} is declared in this release (declared: {})",
                known.join(", ")
            )
        })?;
        if let crate::daemon::components::manifest::Payload::Unavailable { reason } = &c.payload {
            anyhow::bail!("nothing to download for {id}: {reason}");
        }
        anyhow::ensure!(
            c.has_payload(),
            "nothing to download for {id}: this release carries no lock for it"
        );
        if !chosen
            .iter()
            .any(|x: &crate::daemon::components::manifest::Component| x.id == c.id)
        {
            chosen.push(c.clone());
        }
    }
    std::fs::create_dir_all(ctx.layout.root.join("cache/packages"))?;
    let before = receipt.degrade_branches.len();
    packages::fetch_components(ctx, &chosen, fetcher, &mut receipt, out);
    // a failed fetch is this command's error, not an install degrade branch
    let failed: Vec<String> = receipt.degrade_branches.split_off(before);
    packages::record_cache_hashes(&ctx.layout.root.join("cache"), &mut receipt)?;
    receipt.write(&receipt_path)?;
    anyhow::ensure!(
        failed.is_empty(),
        "not every payload was fetched: {}",
        failed.join("; ")
    );
    Ok(receipt)
}

/// Step 6's presence venv (os/5-messaging/README.md §14, §19 L1) as a receipt
/// degrade branch: the release pins a presence lock, yet the venv in place was
/// not built from it — exactly when `stage::presence_venv` returned
/// `Outcome::Unavailable` (`Built`/`Unchanged` leave a venv whose marker is the
/// lock's sha256, `NotShipped` pins no lock). A content probe of that marker,
/// never the log; the reason is the one `venvs/presence.status.json` records.
fn presence_degrade(
    ctx: &InstallContext,
    manifest: &crate::manifest::ChannelManifest,
    release_dir: &std::path::Path,
) -> Option<String> {
    use crate::daemon::transport::venv::presence;
    manifest.presence_lock()?;
    let root = &ctx.layout.root;
    if presence::integrity_ok(root, &presence::lockfile(release_dir)) {
        return None;
    }
    let reason = presence::unavailable_reason(root)
        .unwrap_or_else(|| "no venv was built from the release's presence lock".into());
    Some(format!("presence venv unavailable: {reason}"))
}
