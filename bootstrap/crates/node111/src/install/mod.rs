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
    let receipt = run_with(&ctx, &fetcher)?;
    verify::print_success(&receipt.node_id, &receipt.category, &receipt.version);
    Ok(())
}

/// The state machine (spec §Install flow Linux, steps 1–9). Testable: every
/// system touch goes through ctx (Runner/Env/paths) and fetcher.
pub fn run_with(
    ctx: &InstallContext,
    fetcher: &crate::util::http::Fetcher,
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

    // 5. packages + caches (receipt accumulates hashes + degrade branches)
    let mut receipt = Receipt::new(&pre.manifest.release().to_string());
    let family = packages::Family::of(&pre.distro);
    packages::run(ctx, family, &mut receipt)?;
    let lockfile = stage::fetch_rns_lock(ctx, fetcher, &pre.manifest)?;
    packages::pip_closure(ctx, &lockfile, &mut receipt)?;

    // 6. stage (payload → releases/, flip current, initial floor, symlink)
    stage::run(ctx, fetcher, &pre.manifest)?;

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
