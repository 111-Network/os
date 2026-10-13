//! `111 config` — minimal node config in /etc/111 (env file 0640 root:node111).
//! LAN dashboard sharing is an explicit opt-in here with a printed warning; LAN
//! visitors must pair with the code `111 dashboard` prints (Layer 4, D15).

use crate::daemon::load_env;
use std::path::PathBuf;

pub(crate) fn env_path() -> PathBuf {
    if std::env::var("NODE111_ROOT").is_ok() {
        crate::util::paths::root().join("etc/env")
    } else {
        PathBuf::from("/etc/111/env")
    }
}

fn write_env(pairs: &[(String, String)]) -> anyhow::Result<()> {
    let mut text = String::from("# 111 node config (minimal; `111 config` manages)\n");
    for (k, v) in pairs {
        text.push_str(&format!("{k}={v}\n"));
    }
    let path = env_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::util::fs::atomic_write(&path, text.as_bytes(), 0o640)?;
    // atomic_write replaces the inode (root:root under sudo) — hand the group
    // back to node111 or the daemon cannot read its own config (D15 defect fix).
    chown_root_node111(&path);
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
        [] => {
            let pairs = load_env(env_path().parent().unwrap_or(&PathBuf::from("/etc/111")))?;
            if pairs.is_empty() {
                println!("(no config set)");
            } else {
                for (k, v) in pairs {
                    println!("{k}={v}");
                }
            }
            Ok(())
        }
        [cmd] if cmd == "show" => {
            let pairs = load_env(env_path().parent().unwrap_or(&PathBuf::from("/etc/111")))?;
            if pairs.is_empty() {
                println!("(no config set)");
            } else {
                for (k, v) in pairs {
                    println!("{k}={v}");
                }
            }
            Ok(())
        }
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
            let mut pairs = load_env(env_path().parent().unwrap_or(&PathBuf::from("/etc/111")))?;
            pairs.retain(|(k, _)| k != "DASHBOARD_LAN");
            pairs.push(("DASHBOARD_LAN".into(), on.to_string()));
            write_env(&pairs)?;
            println!("dashboard.lan={on} (takes effect on next daemon restart)");
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
    #[test]
    fn parse_env_roundtrip() {
        let parsed = crate::daemon::parse_env_file("A=1\nB= two\n");
        assert_eq!(parsed.len(), 2);
    }
}
