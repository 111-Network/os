//! `111 update-guard` — root ExecStartPre before every daemon start. If a probation
//! journal is pending and its deadline passed without confirmed health: flip
//! `current` back, clear the failed state. The node can never be permanently dead
//! on a bad release (crash-before-READY, READY-but-503, torn payload).

use crate::daemon::DaemonLayout;
use crate::probation::Journal;
use crate::update_helper::{current_version, flip_current};
use crate::util::run::Runner;
use std::path::PathBuf;

#[derive(Debug, Default)]
pub struct GuardReport {
    /// Calver we reverted to, when a revert happened.
    pub reverted: Option<String>,
    /// Torn current payload detected (hash-check failed) without a journal.
    pub torn_payload: bool,
}

/// Revert to a previous release: flip, clear the journal, reset-failed so the
/// service starts on the known-good release.
fn revert_to(
    layout: &DaemonLayout,
    runner: &dyn Runner,
    previous: &str,
    report: &mut GuardReport,
) -> anyhow::Result<()> {
    flip_current(layout, previous)?;
    Journal::clear(&layout.state_dir().join("update.json"))?;
    let _ = runner.run("systemctl", &["reset-failed", "111node.service"]);
    report.reverted = Some(previous.to_string());
    Ok(())
}

pub fn run_with(
    layout: &DaemonLayout,
    runner: &dyn Runner,
    now_epoch: u64,
) -> anyhow::Result<GuardReport> {
    let mut report = GuardReport::default();
    let journal = Journal::read(&layout.state_dir().join("update.json"))?;

    if let Some(j) = &journal {
        if j.pending
            && !j.health_confirmed
            && now_epoch > j.deadline_epoch
            && !j.previous.is_empty()
        {
            // Crash-before-READY: probation expired unconfirmed.
            revert_to(layout, runner, &j.previous, &mut report)?;
            return Ok(report);
        }
        // Probation still open (deadline in the future) or confirmed: nothing to do.
        return Ok(report);
    }

    // No journal: torn-payload check on the current release (startup
    // hash-check → flip to N-1, spec §Updates).
    if let Some(current) = current_version(layout) {
        let tree = layout.current_release();
        if tree.join("share/payload.manifest").exists()
            && crate::daemon::updater::stage::verify_tree(&tree).is_err()
        {
            report.torn_payload = true;
            // newest release that is NOT the current one
            let mut others: Vec<String> = std::fs::read_dir(layout.root.join("releases"))
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .filter(|n| n != &current)
                        .collect()
                })
                .unwrap_or_default();
            others.sort();
            others.reverse();
            if let Some(previous) = others.first() {
                revert_to(layout, runner, previous, &mut report)?;
            }
        }
    }
    Ok(report)
}

/// Production entry (`111 update-guard`, root ExecStartPre).
pub fn run() -> anyhow::Result<()> {
    let layout = DaemonLayout {
        root: crate::util::paths::root(),
        etc: PathBuf::from("/etc/111"),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let report = run_with(&layout, &crate::util::run::SystemRunner, now)?;
    if let Some(to) = &report.reverted {
        eprintln!("111 update-guard: probation expired unconfirmed — reverted to {to}");
    }
    Ok(())
}
