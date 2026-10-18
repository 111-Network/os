//! Step 8 — verify: `/health` 200 within 60s, dashboard reachable on
//! 127.0.0.1:11180, socket checked loopback-bound (unless LAN sharing is on),
//! `/` checked to serve the app (a loud warning when the binary carries no UI);
//! then the final print (NodeId + the dashboard URLs that actually bound + LAN
//! line + identity-backup hint) and a best-effort browser open as the invoking
//! user (spec §12, Layer 4). Without a display (SSH/headless install) the print
//! says how to reach the dashboard: an SSH tunnel, or LAN sharing.

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
                check_ui(&fetcher, &ctx.health_base);
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

/// What `/` serves when the binary was built without `interface/ui/dist`
/// (`daemon::dashboard::routes` NOT_BUILT notice).
pub const UI_NOT_BUILT_MARKER: &str = "dashboard UI not built";

/// Does this `/` body carry the "dashboard UI not built" notice?
pub fn ui_missing(body: &[u8]) -> bool {
    String::from_utf8_lossy(body).contains(UI_NOT_BUILT_MARKER)
}

/// GET `/`: warn loudly when the notice is served instead of the app (a release
/// built without the UI). Never fails the install — the node itself works.
fn check_ui(fetcher: &crate::util::http::Fetcher, base: &str) {
    let base = base.trim_end_matches('/');
    match fetcher.get(&format!("{base}/")) {
        Ok(body) if ui_missing(&body) => eprintln!(
            "
111 install: ================================================================
111 install: WARNING: this 111 build has NO dashboard UI.
111 install:   {base}/ shows the \"dashboard UI not built\" notice, not the app.
111 install:   The node runs; the minimal status page is at {base}/v1.
111 install:   The release was built without interface/ui/dist: report it.
111 install: ================================================================"
        ),
        Ok(_) => {}
        Err(e) => eprintln!(
            "111 install: WARNING: the dashboard page {base}/ did not load ({e:#}); /health is fine — see journalctl --namespace=111node -u 111node"
        ),
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
        t.push_str(&format!(
            "LAN sharing: off — enable with: {}\n",
            crate::config::lan_enable_hint(false)
        ));
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
    open_dashboard(&d);
}

/// How (if at all) to open the browser after install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenPlan {
    /// `program args...` (spawned; its exit status decides what is printed).
    Spawn(String, Vec<String>),
    /// No graphical session / no invoker / no xdg-open: just tell the user.
    Print,
}

/// What `open_plan` reads about the session (injected so tests need no host).
pub struct SessionProbe<'a> {
    /// Environment lookup.
    pub env: &'a dyn Fn(&str) -> Option<String>,
    /// Is this program on PATH?
    pub has: &'a dyn Fn(&str) -> bool,
    /// uid of a user name (`getpwnam`).
    pub uid_of: &'a dyn Fn(&str) -> Option<u32>,
    /// A Wayland socket name (`wayland-0`) inside a runtime dir.
    pub wayland_in: &'a dyn Fn(&str) -> Option<String>,
}

fn nonempty(v: Option<String>) -> Option<String> {
    v.filter(|v| !v.is_empty())
}

/// The invoking (non-root) user behind sudo or doas.
fn invoker(env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    nonempty(env("SUDO_USER"))
        .or_else(|| nonempty(env("DOAS_USER")))
        .filter(|u| u != "root")
}

/// The display a browser could open on: `(DISPLAY, WAYLAND_DISPLAY,
/// runtime dir)`. `WAYLAND_DISPLAY` falls back to a `wayland-*` socket in the
/// runtime dir (sudo strips `WAYLAND_DISPLAY`); the runtime dir is
/// `XDG_RUNTIME_DIR`, else `/run/user/<uid>` (`SUDO_UID`, else the user's uid).
/// `None` = no graphical session.
fn session_display(
    p: &SessionProbe,
    user: Option<&str>,
) -> Option<(Option<String>, Option<String>, Option<String>)> {
    let uid = nonempty((p.env)("SUDO_UID"))
        .and_then(|u| u.parse::<u32>().ok())
        .or_else(|| user.and_then(|u| (p.uid_of)(u)));
    let runtime =
        nonempty((p.env)("XDG_RUNTIME_DIR")).or_else(|| uid.map(|uid| format!("/run/user/{uid}")));
    let display = nonempty((p.env)("DISPLAY"));
    let wayland = nonempty((p.env)("WAYLAND_DISPLAY"))
        .or_else(|| runtime.as_deref().and_then(|d| (p.wayland_in)(d)));
    if display.is_none() && wayland.is_none() {
        return None;
    }
    Some((display, wayland, runtime))
}

/// Is there a graphical session for the invoker (or, without one, for us)?
pub fn graphical_session(p: &SessionProbe) -> bool {
    let user = invoker(p.env).or_else(|| nonempty((p.env)("USER")));
    session_display(p, user.as_deref()).is_some()
}

/// Decide the browser-open command. Opens only as the invoking user
/// (`SUDO_USER` or `DOAS_USER`, never root) on a graphical session (`DISPLAY`,
/// `WAYLAND_DISPLAY`, or a `wayland-*` socket in the user's runtime dir) with
/// `xdg-open` present; `runuser` preferred, `sudo -u` fallback. Session
/// variables sudo usually strips are passed explicitly.
pub fn open_plan(p: &SessionProbe, url: &str) -> OpenPlan {
    let Some(user) = invoker(p.env) else {
        return OpenPlan::Print;
    };
    let Some((display, wayland, runtime)) = session_display(p, Some(&user)) else {
        return OpenPlan::Print;
    };
    if !(p.has)("xdg-open") {
        return OpenPlan::Print;
    }
    let mut vars: Vec<String> = Vec::new();
    for (k, v) in [
        ("DISPLAY", display),
        ("WAYLAND_DISPLAY", wayland),
        ("XAUTHORITY", (p.env)("XAUTHORITY")),
    ] {
        if let Some(v) = nonempty(v) {
            vars.push(format!("{k}={v}"));
        }
    }
    if let Some(dir) = runtime {
        vars.push(format!("DBUS_SESSION_BUS_ADDRESS=unix:path={dir}/bus"));
        vars.push(format!("XDG_RUNTIME_DIR={dir}"));
    }
    let mut tail: Vec<String> = vec!["env".into()];
    tail.extend(vars);
    tail.push("xdg-open".into());
    tail.push(url.to_string());
    let prog = if (p.has)("runuser") {
        "runuser"
    } else if (p.has)("sudo") {
        "sudo"
    } else {
        return OpenPlan::Print;
    };
    let mut args = vec!["-u".to_string(), user, "--".to_string()];
    args.extend(tail);
    OpenPlan::Spawn(prog.into(), args)
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

/// uid of `user` via `getpwnam`.
fn uid_of(user: &str) -> Option<u32> {
    let name = std::ffi::CString::new(user).ok()?;
    let pw = unsafe { libc::getpwnam(name.as_ptr()) };
    (!pw.is_null()).then(|| unsafe { (*pw).pw_uid })
}

/// The first `wayland-*` socket (not its `.lock`) in `dir`.
pub fn wayland_socket_in(dir: &str) -> Option<String> {
    use std::os::unix::fs::FileTypeExt;
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_socket()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("wayland-") && !n.ends_with(".lock"))
        .collect();
    names.sort();
    names.into_iter().next()
}

/// How a spawned opener ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opened {
    /// Exited 0 within the wait, or still running (a browser launching).
    Yes,
    /// Could not spawn, or exited non-zero.
    No,
}

/// Spawn `prog args` and judge it by its exit status within `wait`.
pub fn run_opener(prog: &str, args: &[String], wait: std::time::Duration) -> Opened {
    let child = std::process::Command::new(prog)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return Opened::No;
    };
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Opened::Yes
                } else {
                    Opened::No
                }
            }
            Ok(None) if start.elapsed() < wait => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            // still running after the wait: the browser is coming up
            Ok(None) => return Opened::Yes,
            Err(_) => return Opened::No,
        }
    }
}

/// The remote port an SSH tunnel should reach: the `127.0.0.1:<port>` URL that
/// bound, else 80 when only `:80` bound, else the default port.
pub fn tunnel_port(urls: &[String]) -> u16 {
    for u in urls {
        if let Some(rest) = u.strip_prefix("http://127.0.0.1:") {
            if let Ok(p) = rest.trim_end_matches('/').parse() {
                return p;
            }
        }
    }
    if urls
        .iter()
        .any(|u| u == "http://127.0.0.1" || u == "http://111.localhost")
    {
        80
    } else {
        crate::daemon::dashboard::DEFAULT_PORT
    }
}

/// The no-display print: an SSH tunnel to the loopback dashboard, and the LAN
/// path (how to turn it on, or where it already is).
pub fn headless_text(d: &DashboardSummary, user: &str, host: &str) -> String {
    let local = crate::daemon::dashboard::DEFAULT_PORT;
    let remote = tunnel_port(&d.urls);
    let mut t = format!(
        "No display on this machine. Open the dashboard from your own computer:
  over SSH:  ssh -L {local}:127.0.0.1:{remote} {user}@{host}
             then browse to http://127.0.0.1:{local} on that computer
"
    );
    let lan_urls: Vec<&String> = d
        .urls
        .iter()
        .filter(|u| !u.contains("127.0.0.1") && !u.contains("localhost"))
        .collect();
    if d.lan && !lan_urls.is_empty() {
        t.push_str(&format!(
            "  over LAN:  {}   (enter the pair code; `sudo 111 dashboard` shows it)\n",
            lan_urls
                .iter()
                .find(|u| !u.contains(".local"))
                .unwrap_or(&lan_urls[0])
        ));
    } else {
        t.push_str(
            "  over LAN:  sudo 111 config dashboard.lan true
             sudo systemctl restart 111node
             sudo 111 dashboard   (prints the LAN address and the pair code)
",
        );
    }
    t
}

fn hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    let r = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if r != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    nonempty(Some(String::from_utf8_lossy(&buf[..end]).into_owned()))
}

/// Best-effort browser open; never fails the install. "Opening …" is printed
/// only once the opener's exit status (≤ 2 s) says it worked.
pub fn open_dashboard(d: &DashboardSummary) {
    let url = d
        .urls
        .first()
        .cloned()
        .unwrap_or_else(|| crate::daemon::dashboard::bind::FALLBACK_URL.to_string());
    let env = |k: &str| std::env::var(k).ok();
    let probe = SessionProbe {
        env: &env,
        has: &on_path,
        uid_of: &uid_of,
        wayland_in: &|dir: &str| wayland_socket_in(dir),
    };
    if let OpenPlan::Spawn(prog, args) = open_plan(&probe, &url) {
        if run_opener(&prog, &args, std::time::Duration::from_secs(2)) == Opened::Yes {
            println!("Opening the dashboard in your browser: {url}");
            return;
        }
    }
    if graphical_session(&probe) {
        println!("Open the dashboard on this machine: {url}");
        return;
    }
    let user = invoker(&env)
        .or_else(|| nonempty(env("USER")))
        .or_else(|| nonempty(env("LOGNAME")))
        .unwrap_or_else(|| "<user>".into());
    let host = crate::daemon::dashboard::bind::lan_ipv4s()
        .first()
        .map(|ip| ip.to_string())
        .or_else(hostname)
        .unwrap_or_else(|| "<this-machine>".into());
    print!("{}", headless_text(d, &user, &host));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn probe_with(
        vars: &[(&str, &str)],
        progs: &[&str],
        uids: &[(&str, u32)],
        sockets: &[(&str, &str)],
        f: impl FnOnce(&SessionProbe) -> OpenPlan,
    ) -> OpenPlan {
        let m: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let env = move |k: &str| m.get(k).cloned();
        let has = |p: &str| progs.contains(&p);
        let uid_of = |u: &str| uids.iter().find(|(n, _)| *n == u).map(|(_, id)| *id);
        let wayland_in = |d: &str| {
            sockets
                .iter()
                .find(|(dir, _)| *dir == d)
                .map(|(_, s)| s.to_string())
        };
        f(&SessionProbe {
            env: &env,
            has: &has,
            uid_of: &uid_of,
            wayland_in: &wayland_in,
        })
    }

    fn plan(vars: &[(&str, &str)], progs: &[&str]) -> OpenPlan {
        probe_with(vars, progs, &[], &[], |p| {
            open_plan(p, "http://111.localhost")
        })
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
    fn doas_invoker_and_wayland_socket_hint() {
        let all = ["xdg-open", "runuser"];
        // doas sets DOAS_USER (no uid var); sudo stripped WAYLAND_DISPLAY but the
        // user's runtime dir holds the socket → open there
        let p = probe_with(
            &[("DOAS_USER", "kia")],
            &all,
            &[("kia", 1000)],
            &[("/run/user/1000", "wayland-1")],
            |p| {
                assert!(graphical_session(p));
                open_plan(p, "http://111.localhost")
            },
        );
        match p {
            OpenPlan::Spawn(prog, args) => {
                assert_eq!(prog, "runuser");
                assert_eq!(&args[..3], ["-u", "kia", "--"]);
                assert!(args.contains(&"WAYLAND_DISPLAY=wayland-1".to_string()));
                assert!(args.contains(&"XDG_RUNTIME_DIR=/run/user/1000".to_string()));
            }
            other => panic!("{other:?}"),
        }
        // SUDO_UID wins for the runtime dir
        let p = probe_with(
            &[("SUDO_USER", "kia"), ("SUDO_UID", "1001")],
            &all,
            &[("kia", 1000)],
            &[("/run/user/1001", "wayland-0")],
            |p| open_plan(p, "u"),
        );
        assert!(matches!(p, OpenPlan::Spawn(..)), "{p:?}");
        // no socket, no display: headless
        let p = probe_with(&[("DOAS_USER", "kia")], &all, &[("kia", 1000)], &[], |p| {
            assert!(!graphical_session(p));
            open_plan(p, "u")
        });
        assert_eq!(p, OpenPlan::Print);
        // DOAS_USER=root is never an invoker
        let p = probe_with(
            &[("DOAS_USER", "root"), ("DISPLAY", ":0")],
            &all,
            &[],
            &[],
            |p| open_plan(p, "u"),
        );
        assert_eq!(p, OpenPlan::Print);
    }

    #[test]
    fn wayland_socket_scan_skips_locks_and_plain_files() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().to_str().unwrap();
        assert_eq!(wayland_socket_in(d), None);
        std::fs::write(dir.path().join("wayland-0.lock"), "").unwrap();
        std::fs::write(dir.path().join("wayland-9"), "").unwrap(); // not a socket
        assert_eq!(wayland_socket_in(d), None);
        let _l = std::os::unix::net::UnixListener::bind(dir.path().join("wayland-0")).unwrap();
        assert_eq!(wayland_socket_in(d).as_deref(), Some("wayland-0"));
        assert_eq!(wayland_socket_in("/nonexistent/111"), None);
    }

    #[test]
    fn opener_is_judged_by_its_exit_status() {
        let short = std::time::Duration::from_millis(300);
        assert_eq!(run_opener("true", &[], short), Opened::Yes);
        assert_eq!(run_opener("false", &[], short), Opened::No);
        assert_eq!(run_opener("/nonexistent/xdg-open", &[], short), Opened::No);
        // still running at the deadline = a browser launching
        assert_eq!(run_opener("sleep", &["5".to_string()], short), Opened::Yes);
    }

    #[test]
    fn headless_print_has_the_tunnel_and_the_lan_path() {
        let d = DashboardSummary {
            urls: vec![
                "http://111.localhost".into(),
                "http://127.0.0.1:11180".into(),
            ],
            lan: false,
            pair_code: None,
        };
        let t = headless_text(&d, "kia", "192.168.1.5");
        assert!(
            t.contains("ssh -L 11180:127.0.0.1:11180 kia@192.168.1.5\n"),
            "{t}"
        );
        assert!(t.contains("browse to http://127.0.0.1:11180"));
        assert!(t.contains("sudo 111 config dashboard.lan true\n"));
        assert!(t.contains("sudo systemctl restart 111node\n"));
        assert!(t.contains("sudo 111 dashboard"));
        // only :80 bound: the tunnel reaches port 80
        let only80 = DashboardSummary {
            urls: vec!["http://111.localhost".into(), "http://127.0.0.1".into()],
            ..d.clone()
        };
        assert!(headless_text(&only80, "kia", "h").contains("ssh -L 11180:127.0.0.1:80 kia@h\n"));
        // LAN already on: point at the LAN-IP URL, not the enable steps
        let lan = DashboardSummary {
            urls: vec![
                "http://127.0.0.1:11180".into(),
                "http://111.local:11180".into(),
                "http://192.168.1.5:11180".into(),
            ],
            lan: true,
            pair_code: Some("7KQ2-XM9D".into()),
        };
        let t = headless_text(&lan, "kia", "192.168.1.5");
        assert!(t.contains("over LAN:  http://192.168.1.5:11180"), "{t}");
        assert!(!t.contains("dashboard.lan true"));
        assert_eq!(tunnel_port(&[]), 11180);
    }

    #[test]
    fn ui_notice_is_detected() {
        assert!(ui_missing(
            b"<p>dashboard UI not built \xe2\x80\x94 run <code>pnpm build</code></p>"
        ));
        assert!(!ui_missing(b"<!doctype html><div id=\"app\"></div>"));
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
        assert!(t.contains(
            "LAN sharing: off — enable with: sudo 111 config dashboard.lan true && sudo systemctl restart 111node"
        ));
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
