//! CLI surface (T14 owns the final wiring): install · daemon · identity
//! backup|export|recover · config · uninstall · update-helper · update-guard.
//! Flags: --phone --serial --keep-adb --allow-degraded --version (calver).
//! Host install and --phone are mutually exclusive.

#[derive(Debug, Clone, Default)]
pub struct InstallArgs {
    pub phone: bool,
    pub serial: Option<String>,
    pub keep_adb: bool,
    pub allow_degraded: bool,
    /// Shim-forwarded floor (the binary also refuses below the compiled-in
    /// MIN_INSTALLER_VERSION).
    pub min_installer_version: Option<crate::calver::Calver>,
}

pub fn main_entry(args: Vec<String>) -> i32 {
    let _ = args;
    eprintln!("111 stub — workspace scaffold (T0). Subcommands land per os/bootstrap/TASKS.md.");
    2
}
