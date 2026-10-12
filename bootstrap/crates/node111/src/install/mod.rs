//! Linux host installer state machine + phone provisioning. Every step probes
//! content (hashes, markers), never mere existence, and resumes at the failed step.

pub mod categorize;
pub mod offer;
pub mod packages;
pub mod phone;
pub mod preflight;
pub mod probe;
pub mod receipt;
pub mod stage;
pub mod supervise;
pub mod verify;

/// Entry point for `111 install` / `111 install --phone`.
pub fn run(args: crate::cli::InstallArgs) -> anyhow::Result<()> {
    let _ = args;
    todo!("T5/T12")
}
