//! `111 config` — minimal node config in /etc/111 (env file 0640 root:node111).
//! LAN dashboard binding is an explicit opt-in here with a printed warning until
//! Layer 4 auth exists.

use crate::daemon::load_env;
use std::path::PathBuf;

fn env_path() -> PathBuf {
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
    crate::util::fs::atomic_write(&path, text.as_bytes(), 0o640)
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
                    "WARNING: binding the dashboard to the LAN exposes node state to your \
                     network. It stays GET/read-only with no auth until Layer 4 — enable \
                     only on networks you trust."
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
