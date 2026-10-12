//! Install context: every host-install step runs against it — production is
//! `/`, tests point the whole tree at a temp dir. Command execution goes
//! through `Runner` (MockRunner in tests).

use crate::daemon::DaemonLayout;
use crate::install::probe::Env;
use crate::util::run::Runner;

pub struct InstallContext<'a> {
    /// Node root (/var/lib/111 in production).
    pub layout: DaemonLayout,
    /// Filesystem root prefix for /etc, /usr, /run ("/" in production).
    pub fs_root: std::path::PathBuf,
    pub runner: &'a dyn Runner,
    pub env: &'a dyn Env,
    pub allow_degraded: bool,
    /// Compiled-in minimum installer version (stale-shim redirects can't
    /// downgrade below it).
    pub min_installer_version: crate::calver::Calver,
    /// Manifest channel base (production: install.111.network; tests: fixture).
    pub manifest_base: String,
    /// Optional fallback base.
    pub manifest_fallback: Option<String>,
    /// Health endpoint base (production http://127.0.0.1:11180; tests: fixture).
    pub health_base: String,
    /// HTTP-204 connectivity probe URLs (production: first-party; tests: fixture).
    pub probe_204: Vec<String>,
    /// Enforce the root privilege contract (production true; tests false).
    pub check_root: bool,
}

impl<'a> InstallContext<'a> {
    /// Map an absolute system path (/etc/systemd/...) into the fs root.
    pub fn sys(&self, absolute: &str) -> std::path::PathBuf {
        self.fs_root.join(absolute.trim_start_matches('/'))
    }

    pub fn state_dir(&self) -> std::path::PathBuf {
        self.layout.state_dir()
    }
}
