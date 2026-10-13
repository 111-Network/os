//! Step 8 — verify: `/health` 200 within 60s, dashboard reachable on
//! 127.0.0.1:11180, socket checked loopback-bound (unless LAN sharing is on);
//! then the final print (NodeId + the dashboard URLs that actually bound + LAN
//! line + identity-backup hint) and a best-effort browser open as the invoking
//! user (spec §12, Layer 4).

use super::context::InstallContext;

/// Poll /health until 200 or the deadline. Returns the health JSON on success.
pub fn wait_healthy(ctx: &InstallContext, deadline_secs: u64) -> anyhow::Result<String> {
    let fetcher = crate::util::http::Fetcher::new(std::time::Duration::from_secs(3), 1 << 20)?;
    let health_url = format!("{}/health", ctx.health_base.trim_end_matches('/'));
    let start = std::time::Instant::now();
    loop {
        if let Ok(body) = fetcher.get(&health_url) {
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
            if v["status"] == "ok" {
                // Loopback assert: the same port must NOT answer on a
                // non-loopback address of this host.
                assert_loopback_only(ctx);
                return Ok(String::from_utf8_lossy(&body).into_owned());
            }
        }
        if start.elapsed().as_secs() >= deadline_secs {
            anyhow::bail!(
                "node did not become healthy within {deadline_secs}s — check `journalctl --namespace=111node -u 111node -e`"
            );
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}

/// Check the dashboard is not listening on any non-loopback address: every
/// non-loopback IPv4 of this host must refuse the connection — unless LAN
/// sharing is on (then answering there is the point). A hit is a logged
/// warning, never a panic (release builds used to skip this entirely).
fn assert_loopback_only(ctx: &InstallContext) {
    let lan =
        crate::daemon::dashboard::bind::read_state(&ctx.layout.state_dir()).is_some_and(|f| f.lan);
    if lan {
        return;
    }
    let out = ctx.runner.run("hostname", &["-I"]);
    let Ok(out) = out else { return };
    for ip in out.stdout.split_whitespace() {
        if ip == "127.0.0.1" {
            continue;
        }
        if let Ok(addr) = ip.parse::<std::net::Ipv4Addr>() {
            if addr.is_loopback() {
                continue;
            }
            let hit = std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::new(std::net::IpAddr::V4(addr), 11180),
                std::time::Duration::from_millis(500),
            );
            if hit.is_ok() {
                eprintln!(
                    "111 install: WARNING: the dashboard answers on {addr}:11180 although LAN sharing is off — check for another service on that port"
                );
            }
        }
    }
}

/// What the success block says about the dashboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardSummary {
    /// URLs that bound (nicest first); the constant fallback when unknown.
    pub urls: Vec<String>,
    pub lan: bool,
    /// Pair code (only read when LAN is on).
    pub pair_code: Option<String>,
}

/// Read `state/dashboard.json` (+ the pair code when LAN is on).
pub fn dashboard_summary(state_dir: &std::path::Path) -> DashboardSummary {
    use crate::daemon::dashboard::{api_v1::auth, bind};
    let file = bind::read_state(state_dir).filter(|f| !f.urls.is_empty());
    let lan = file.as_ref().is_some_and(|f| f.lan);
    DashboardSummary {
        urls: file
            .map(|f| f.urls)
            .unwrap_or_else(|| vec![bind::FALLBACK_URL.to_string()]),
        lan,
        pair_code: if lan {
            auth::read_pair_code(state_dir).ok().flatten()
        } else {
            None
        },
    }
}

/// The success block (pure — printed by [`print_success`], asserted in tests).
pub fn success_text(node_id: &str, category: &str, version: &str, d: &DashboardSummary) -> String {
    let human = crate::identity::node_id_bech32(node_id).unwrap_or_else(|| node_id.to_string());
    let mut t = format!(
        "
111 Network node installed.
  node id:   {human}
  hex:       {node_id}
  category:  {category}
  version:   {version}
  dashboard: {}
",
        d.urls
            .first()
            .map(String::as_str)
            .unwrap_or(crate::daemon::dashboard::bind::FALLBACK_URL)
    );
    for u in d.urls.iter().skip(1) {
        t.push_str(&format!("             {u}\n"));
    }
    if d.lan {
        match &d.pair_code {
            Some(code) => t.push_str(&format!(
                "LAN sharing: on — visitors pair with code {code} (`111 dashboard` shows it again)\n"
            )),
            None => t.push_str("LAN sharing: on — `sudo 111 dashboard` shows the pair code\n"),
        }
    } else {
        t.push_str("LAN sharing: off — enable with: sudo 111 config dashboard.lan true\n");
    }
    t.push_str(
        "
Back up your identity (12 words) with:
  sudo -u node111 111 identity backup",
    );
    t
}

/// Final print per spec step 8 / Layer 4 §12. `node_id` is the §7 machine
/// string (hex); the human line carries the bech32 `node1…` transfer form.
pub fn print_success(
    layout: &crate::daemon::DaemonLayout,
    node_id: &str,
    category: &str,
    version: &str,
) {
    let d = dashboard_summary(&layout.state_dir());
    println!("{}", success_text(node_id, category, version, &d));
    let url = d
        .urls
        .first()
        .cloned()
        .unwrap_or_else(|| crate::daemon::dashboard::bind::FALLBACK_URL.to_string());
    open_dashboard(&url);
}

/// How (if at all) to open the browser after install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenPlan {
    /// `program args...` (spawned detached, result ignored).
    Spawn(String, Vec<String>),
    /// No graphical session / no invoker / no xdg-open: just tell the user.
    Print,
}

/// Decide the browser-open command. Opens only as the invoking user
/// (`SUDO_USER`, never root) on a graphical session (`DISPLAY` or
/// `WAYLAND_DISPLAY`) with `xdg-open` present; `runuser` preferred, `sudo -u`
/// fallback. Session variables sudo usually strips are passed explicitly
/// (`XDG_RUNTIME_DIR` / `DBUS_SESSION_BUS_ADDRESS` derived from `SUDO_UID`).
pub fn open_plan(
    env: &dyn Fn(&str) -> Option<String>,
    has: &dyn Fn(&str) -> bool,
    url: &str,
) -> OpenPlan {
    let Some(user) = env("SUDO_USER").filter(|u| !u.is_empty() && u != "root") else {
        return OpenPlan::Print;
    };
    let display = env("DISPLAY").filter(|v| !v.is_empty());
    let wayland = env("WAYLAND_DISPLAY").filter(|v| !v.is_empty());
    if (display.is_none() && wayland.is_none()) || !has("xdg-open") {
        return OpenPlan::Print;
    }
    let mut vars: Vec<String> = Vec::new();
    for (k, v) in [
        ("DISPLAY", display),
        ("WAYLAND_DISPLAY", wayland),
        ("XAUTHORITY", env("XAUTHORITY")),
    ] {
        if let Some(v) = v.filter(|v| !v.is_empty()) {
            vars.push(format!("{k}={v}"));
        }
    }
    let runtime = env("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .or_else(|| env("SUDO_UID").map(|uid| format!("/run/user/{uid}")));
    if let Some(dir) = runtime {
        vars.push(format!("DBUS_SESSION_BUS_ADDRESS=unix:path={dir}/bus"));
        vars.push(format!("XDG_RUNTIME_DIR={dir}"));
    }
    let mut tail: Vec<String> = vec!["env".into()];
    tail.extend(vars);
    tail.push("xdg-open".into());
    tail.push(url.to_string());
    if has("runuser") {
        let mut args = vec!["-u".to_string(), user, "--".to_string()];
        args.extend(tail);
        OpenPlan::Spawn("runuser".into(), args)
    } else if has("sudo") {
        let mut args = vec!["-u".to_string(), user, "--".to_string()];
        args.extend(tail);
        OpenPlan::Spawn("sudo".into(), args)
    } else {
        OpenPlan::Print
    }
}

fn on_path(prog: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| {
            let f = d.join(prog);
            f.is_file()
                && std::fs::metadata(&f)
                    .map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o111 != 0)
                    .unwrap_or(false)
        })
    })
}

/// Best-effort browser open; never fails the install.
pub fn open_dashboard(url: &str) {
    let env = |k: &str| std::env::var(k).ok();
    match open_plan(&env, &on_path, url) {
        OpenPlan::Spawn(prog, args) => {
            let spawned = std::process::Command::new(&prog)
                .args(&args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            match spawned {
                Ok(_) => println!("Opening the dashboard in your browser: {url}"),
                Err(_) => println!("Open the dashboard on this machine: {url}"),
            }
        }
        OpenPlan::Print => println!("Open the dashboard on this machine: {url}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn plan(vars: &[(&str, &str)], progs: &[&str]) -> OpenPlan {
        let m: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let env = move |k: &str| m.get(k).cloned();
        let has = |p: &str| progs.contains(&p);
        open_plan(&env, &has, "http://111.localhost")
    }

    #[test]
    fn opens_only_as_invoker_on_a_graphical_session() {
        let all = ["xdg-open", "runuser", "sudo"];
        assert_eq!(plan(&[("DISPLAY", ":0")], &all), OpenPlan::Print); // no SUDO_USER
        assert_eq!(
            plan(&[("SUDO_USER", "root"), ("DISPLAY", ":0")], &all),
            OpenPlan::Print
        );
        assert_eq!(plan(&[("SUDO_USER", "kia")], &all), OpenPlan::Print); // headless
        assert_eq!(
            plan(&[("SUDO_USER", "kia"), ("DISPLAY", ":0")], &["runuser"]),
            OpenPlan::Print
        ); // no xdg-open
        match plan(
            &[
                ("SUDO_USER", "kia"),
                ("SUDO_UID", "1000"),
                ("WAYLAND_DISPLAY", "wayland-0"),
            ],
            &all,
        ) {
            OpenPlan::Spawn(prog, args) => {
                assert_eq!(prog, "runuser");
                assert_eq!(&args[..4], ["-u", "kia", "--", "env"]);
                assert!(args.contains(&"WAYLAND_DISPLAY=wayland-0".to_string()));
                assert!(args.contains(&"XDG_RUNTIME_DIR=/run/user/1000".to_string()));
                assert_eq!(
                    &args[args.len() - 2..],
                    ["xdg-open", "http://111.localhost"]
                );
            }
            other => panic!("{other:?}"),
        }
        match plan(
            &[("SUDO_USER", "kia"), ("DISPLAY", ":0")],
            &["xdg-open", "sudo"],
        ) {
            OpenPlan::Spawn(prog, _) => assert_eq!(prog, "sudo"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn success_block_lists_urls_lan_and_backup() {
        let d = DashboardSummary {
            urls: vec![
                "http://111.localhost".into(),
                "http://127.0.0.1:11180".into(),
            ],
            lan: false,
            pair_code: None,
        };
        let t = success_text("ab", "Node2A", "2026.10.13", &d);
        assert!(t.contains("dashboard: http://111.localhost\n"));
        assert!(t.contains("             http://127.0.0.1:11180\n"));
        assert!(t.contains("LAN sharing: off — enable with: sudo 111 config dashboard.lan true"));
        assert!(t.contains("sudo -u node111 111 identity backup"));
        let d = DashboardSummary {
            urls: vec!["http://127.0.0.1:11180".into()],
            lan: true,
            pair_code: Some("7KQ2-XM9D".into()),
        };
        assert!(success_text("ab", "Node2A", "v", &d).contains("pair with code 7KQ2-XM9D"));
    }

    #[test]
    fn summary_falls_back_without_state() {
        let dir = tempfile::tempdir().unwrap();
        let d = dashboard_summary(dir.path());
        assert_eq!(d.urls, ["http://127.0.0.1:11180"]);
        assert!(!d.lan);
    }
}
