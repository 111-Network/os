//! `111 uninstall --phone` — teardown over sshd (post-lockdown) or adb (pre):
//! `pm uninstall` ×3, deviceidle whitelist removal, appops resets.
//! Implemented in T12; the signature is frozen for the CLI wiring (T14).

/// Entry point for `111 uninstall --phone [--serial S]`.
pub fn run(serial: Option<String>) -> anyhow::Result<()> {
    let _ = serial;
    todo!("T12")
}
