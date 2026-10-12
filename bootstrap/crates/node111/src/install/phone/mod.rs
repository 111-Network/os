//! Phone provisioning over USB-C (`111 install --phone`). Runs as the invoking
//! user (adb authorization is per-user — never root).

pub mod adb;
pub mod apk;
pub mod harden;
pub mod preflight;
pub mod runas;
pub mod start;
pub mod teardown;
pub mod verify;

pub fn run(args: crate::cli::InstallArgs) -> anyhow::Result<()> {
    let _ = args;
    todo!("T12")
}
