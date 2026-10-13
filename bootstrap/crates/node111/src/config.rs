//! `111 config` — minimal node config in /etc/111 (env file 0640 root:node111).
//! LAN dashboard sharing is an explicit opt-in here with a printed warning; LAN
//! visitors must pair with the code `111 dashboard` prints (Layer 4, D15).
//!
//! `dashboard.lan` lives in `state/settings.json` `lan_enabled` — the single
//! source of truth the dashboard's Settings toggle also writes. Setting it here
//! also deletes any `DASHBOARD_LAN` line from the env file (that line is an
//! override which would otherwise pin the value and lock the toggle).

use crate::daemon::dashboard::api_v1::settings;
use crate::daemon::dashboard::bind;
use crate::daemon::load_env;
use std::path::{Path, PathBuf};

pub(crate) fn env_path() -> PathBuf {
    if std::env::var("NODE111_ROOT").is_ok() {
        crate::util::paths::root().join("etc/env")
    } else {
        PathBuf::from("/etc/111/env")
    }
}

fn write_env(path: &Path, pairs: &[(String, String)]) -> anyhow::Result<()> {
    let mut text = String::from("# 111 node config (minimal; `111 config` manages)\n");
    for (k, v) in pairs {
        text.push_str(&format!("{k}={v}\n"));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::util::fs::atomic_write(path, text.as_bytes(), 0o640)?;
    // atomic_write replaces the inode (root:root under sudo) — hand the group
    // back to node111 or the daemon cannot read its own config (D15 defect fix).
    chown_root_node111(path);
    Ok(())
}

/// Running on Termux (the phone node)? Same detection as the daemon's
/// loopback-trust policy: `TERMUX_VERSION`, or the Termux prefix exists (also
/// a `PREFIX` pointing into it).
pub fn on_termux() -> bool {
    std::env::var_os("TERMUX_VERSION").is_some()
        || std::path::Path::new("/data/data/com.termux/files/usr").is_dir()
        || std::env::var("PREFIX").is_ok_and(|p| p.contains("com.termux"))
}

/// Restart command on a Termux node (runit service dir; explicit `SVDIR` so it
/// also works in a non-login `ssh … 'cmd'` session).
pub const TERMUX_RESTART: &str = "SVDIR=$PREFIX/var/service sv restart 111node";
/// Restart command on a systemd host.
pub const HOST_RESTART: &str = "sudo systemctl restart 111node";

/// The command that restarts the node daemon here.
pub fn restart_cmd() -> &'static str {
    if on_termux() {
        TERMUX_RESTART
    } else {
        HOST_RESTART
    }
}

/// How to turn LAN sharing on (the setting, then the restart that applies it).
pub fn lan_enable_hint(termux: bool) -> String {
    if termux {
        format!("111 config dashboard.lan true && {TERMUX_RESTART}")
    } else {
        format!("sudo 111 config dashboard.lan true && {HOST_RESTART}")
    }
}

/// `chown node111:node111 <path>` when running as root (no-op otherwise, and
/// when the user does not exist). Best-effort, logged.
pub fn chown_node111(path: &Path) {
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    let Ok(name) = std::ffi::CString::new("node111") else {
        return;
    };
    let pw = unsafe { libc::getpwnam(name.as_ptr()) };
    if pw.is_null() {
        return;
    }
    let (uid, gid) = unsafe { ((*pw).pw_uid, (*pw).pw_gid) };
    if let Err(e) = std::os::unix::fs::chown(path, Some(uid), Some(gid)) {
        eprintln!("111: chown node111 {} failed: {e}", path.display());
    }
}

/// What [`set_dashboard_lan`] changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanChange {
    /// A `DASHBOARD_LAN` line was deleted from the env file.
    pub env_line_removed: bool,
}

/// `111 config dashboard.lan <on>`: write `lan_enabled` into
/// `<state_dir>/settings.json` (merge; 0644, handed to node111 when run as root
/// — the same file and mode the daemon's PATCH writes) and delete every
/// `DASHBOARD_LAN` line from `env_file`, which would otherwise override it.
pub fn set_dashboard_lan(state_dir: &Path, env_file: &Path, on: bool) -> anyhow::Result<LanChange> {
    // read the env file first: an unreadable one fails before anything is written
    let etc = env_file.parent().unwrap_or(Path::new("/etc/111"));
    let mut pairs = load_env(etc)?;
    let before = pairs.len();
    pairs.retain(|(k, _)| k != bind::LAN_ENV_KEY);
    let env_line_removed = pairs.len() != before;

    std::fs::create_dir_all(state_dir)?;
    let mut f = settings::read_file(state_dir);
    f.schema = 1;
    f.lan_enabled = on;
    let path = state_dir.join(bind::SETTINGS_FILE);
    crate::util::fs::atomic_write(&path, &serde_json::to_vec_pretty(&f)?, 0o644)?;
    chown_node111(&path);

    if env_line_removed {
        write_env(env_file, &pairs)?;
    }
    Ok(LanChange { env_line_removed })
}

/// The line printed after `dashboard.lan` was saved. `running` = the LAN state
/// of a live daemon (`None` = not running).
pub fn lan_saved_text(on: bool, running: Option<bool>, restart: &str) -> String {
    match running {
        Some(now) if now == on => format!("dashboard.lan={on} (saved; already in effect)"),
        Some(_) => format!("dashboard.lan={on} (saved; takes effect after: {restart})"),
        None => format!("dashboard.lan={on} (saved; applies when the node starts)"),
    }
}

/// Map a permission error to the sudo hint.
fn needs_root(e: anyhow::Error, hint: &str) -> anyhow::Error {
    let denied = e
        .chain()
        .filter_map(|c| c.downcast_ref::<std::io::Error>())
        .any(|io| io.kind() == std::io::ErrorKind::PermissionDenied);
    if denied {
        anyhow::anyhow!("permission denied — run: {hint}")
    } else {
        e
    }
}

fn state_dir() -> PathBuf {
    crate::util::paths::root().join("state")
}

/// `111 config` / `111 config show`.
fn show() -> anyhow::Result<()> {
    let env_file = env_path();
    let pairs = load_env(env_file.parent().unwrap_or(Path::new("/etc/111")))?;
    let lan = bind::lan_config(&pairs, &state_dir());
    if lan.locked {
        println!(
            "dashboard.lan={} (pinned by {} in {})",
            lan.enabled,
            bind::LAN_ENV_KEY,
            env_file.display()
        );
    } else {
        println!("dashboard.lan={}", lan.enabled);
    }
    for (k, v) in pairs {
        println!("{k}={v}");
    }
    Ok(())
}

/// The `node111` group id (`getgrnam`), if the group exists.
pub fn node111_gid() -> Option<u32> {
    let name = std::ffi::CString::new("node111").ok()?;
    let gr = unsafe { libc::getgrnam(name.as_ptr()) };
    (!gr.is_null()).then(|| unsafe { (*gr).gr_gid })
}

/// `chown root:node111 <path>` when running as root (no-op otherwise: Termux /
/// `NODE111_ROOT` trees are owned by the invoking user). Best-effort, logged.
pub fn chown_root_node111(path: &std::path::Path) {
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    match node111_gid() {
        Some(gid) => {
            if let Err(e) = std::os::unix::fs::chown(path, Some(0), Some(gid)) {
                eprintln!("111: chown root:node111 {} failed: {e}", path.display());
            }
        }
        None => eprintln!(
            "111: group node111 not found — {} stays root-owned (the daemon may not read it)",
            path.display()
        ),
    }
}

pub fn run(args: &[String]) -> anyhow::Result<()> {
    match args {
        [] => show(),
        [cmd] if cmd == "show" => show(),
        [key, value] if key == "dashboard.lan" => {
            let on = match value.as_str() {
                "true" => true,
                "false" => false,
                other => anyhow::bail!("dashboard.lan takes true|false (got {other:?})"),
            };
            if on {
                println!(
                    "WARNING: LAN visitors must enter the pair code shown by `111 dashboard`; \
                     traffic is unencrypted on shared Wi-Fi."
                );
            }
            let env_file = env_path();
            let state = state_dir();
            let change = set_dashboard_lan(&state, &env_file, on)
                .map_err(|e| needs_root(e, &format!("sudo 111 config dashboard.lan {on}")))?;
            if change.env_line_removed {
                println!(
                    "removed {} from {} (it overrode this setting)",
                    bind::LAN_ENV_KEY,
                    env_file.display()
                );
            }
            let running = bind::read_state(&state)
                .filter(bind::daemon_alive)
                .map(|f| f.lan);
            println!("{}", lan_saved_text(on, running, restart_cmd()));
            Ok(())
        }
        [action, target] if action == "offer" => {
            let layout = crate::daemon::DaemonLayout {
                root: crate::util::paths::root(),
                etc: PathBuf::from("/etc/111"),
            };
            let ctx_root = crate::util::paths::root();
            let _ = layout;
            match target.as_str() {
                "accept" | "dismiss" => {
                    let accept = target == "accept";
                    // offer.json lives in state/ (node111-owned); write via the
                    // install::offer helper semantics.
                    let path = ctx_root.join("state/offer.json");
                    let body = serde_json::json!({
                        "schema": 1,
                        "tier": "standard",
                        "state": if accept { "accepted" } else { "dismissed" },
                        "recorded_at": time::OffsetDateTime::now_utc()
                            .format(&time::format_description::well_known::Rfc3339)
                            .unwrap_or_else(|_| "unknown".into()),
                    });
                    crate::util::fs::atomic_write(&path, body.to_string().as_bytes(), 0o644)?;
                    println!(
                        "standard tier offer: {}",
                        if accept {
                            "accepted (intent recorded; nothing downloaded yet — Layer 3 lands it)"
                        } else {
                            "dismissed"
                        }
                    );
                    Ok(())
                }
                other => anyhow::bail!("offer takes accept|dismiss (got {other:?})"),
            }
        }
        _ => {
            eprintln!("usage: 111 config [show | dashboard.lan true|false | offer accept|dismiss]");
            std::process::exit(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_roundtrip() {
        let parsed = crate::daemon::parse_env_file("A=1\nB= two\n");
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn saved_text_is_honest_about_restarts() {
        let r = "sudo systemctl restart 111node";
        assert_eq!(
            lan_saved_text(true, Some(true), r),
            "dashboard.lan=true (saved; already in effect)"
        );
        assert_eq!(
            lan_saved_text(false, Some(true), r),
            "dashboard.lan=false (saved; takes effect after: sudo systemctl restart 111node)"
        );
        assert_eq!(
            lan_saved_text(true, None, r),
            "dashboard.lan=true (saved; applies when the node starts)"
        );
    }
}
