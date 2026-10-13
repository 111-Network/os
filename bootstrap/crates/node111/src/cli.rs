//! CLI surface: install · daemon · identity backup|restore|commission ·
//! config · dashboard [rotate] · uninstall · update-helper · update-guard.
//! Flags: --phone --serial --keep-adb --allow-degraded --min-installer-version
//! --version (calver). Host install and --phone are mutually exclusive.

/// Binary version: stamped by release builds (NODE111_BUILD_VERSION), "dev" otherwise.
pub const BUILD_VERSION: &str = match option_env!("NODE111_BUILD_VERSION") {
    Some(v) => v,
    None => "dev",
};

#[derive(Debug, Clone, Default)]
pub struct InstallArgs {
    pub phone: bool,
    pub serial: Option<String>,
    pub keep_adb: bool,
    pub allow_degraded: bool,
    /// Optional adb-reboot verification test before lockdown.
    pub reboot_test: bool,
    /// Shim-forwarded floor (the binary also refuses below the compiled-in
    /// MIN_INSTALLER_VERSION).
    pub min_installer_version: Option<crate::calver::Calver>,
}

#[derive(Debug)]
enum Cmd {
    Install(InstallArgs),
    Daemon,
    IdentityBackup,
    IdentityRestore,
    /// Cat-1 commissioning ceremony, commissioner side (L2 §8.3, T11).
    /// `--loopback-test` runs step 4 against the in-process test relay.
    IdentityCommission {
        loopback_test: bool,
    },
    /// Internal (not in USAGE): idempotent keygen for the phone flow's
    /// on-device step — prints the NodeId hex; never overwrites.
    IdentityGenerate {
        burn: bool,
    },
    Config(Vec<String>),
    /// `111 dashboard [rotate]` (Layer 4 §11): URLs, LAN state, pair code, QR.
    Dashboard {
        rotate: bool,
    },
    Uninstall {
        phone: bool,
        serial: Option<String>,
    },
    UpdateHelper,
    UpdateGuard,
    Version,
    Help,
}

fn parse_install_flags(args: &[String]) -> anyhow::Result<InstallArgs> {
    let mut ia = InstallArgs::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        // `--flag value` and `--flag=value` both accepted (the shim uses =).
        if let Some(v) = arg.strip_prefix("--min-installer-version=") {
            ia.min_installer_version = Some(v.parse()?);
            i += 1;
            continue;
        }
        if let Some(v) = arg.strip_prefix("--serial=") {
            ia.serial = Some(v.to_string());
            i += 1;
            continue;
        }
        match arg {
            "--phone" => ia.phone = true,
            "--keep-adb" => ia.keep_adb = true,
            "--reboot-test" => ia.reboot_test = true,
            "--allow-degraded" => ia.allow_degraded = true,
            "--serial" => {
                i += 1;
                ia.serial = Some(
                    args.get(i)
                        .ok_or_else(|| anyhow::anyhow!("--serial needs a value"))?
                        .clone(),
                );
            }
            "--min-installer-version" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("--min-installer-version needs a value"))?;
                ia.min_installer_version = Some(v.parse()?);
            }
            other => anyhow::bail!("unknown install flag {other:?}"),
        }
        i += 1;
    }
    Ok(ia)
}

fn parse(args: &[String]) -> anyhow::Result<Cmd> {
    let Some(first) = args.first() else {
        return Ok(Cmd::Help);
    };
    Ok(match first.as_str() {
        "install" => Cmd::Install(parse_install_flags(&args[1..])?),
        "daemon" => Cmd::Daemon,
        "identity" => match args.get(1).map(String::as_str) {
            Some("backup") => Cmd::IdentityBackup,
            Some("restore") => Cmd::IdentityRestore,
            Some("commission") => {
                let mut loopback_test = false;
                for flag in &args[2..] {
                    match flag.as_str() {
                        "--loopback-test" => loopback_test = true,
                        other => anyhow::bail!("unknown identity commission flag {other:?}"),
                    }
                }
                Cmd::IdentityCommission { loopback_test }
            }
            Some("generate") => Cmd::IdentityGenerate {
                burn: args.get(2).map(String::as_str) == Some("--burn"),
            },
            other => anyhow::bail!("usage: 111 identity backup|restore|commission (got {other:?})"),
        },
        "config" => Cmd::Config(args[1..].to_vec()),
        "dashboard" => match (args.get(1).map(String::as_str), args.len()) {
            (None, _) => Cmd::Dashboard { rotate: false },
            (Some("rotate"), 2) => Cmd::Dashboard { rotate: true },
            (other, _) => anyhow::bail!("usage: 111 dashboard [rotate] (got {other:?})"),
        },
        "uninstall" => {
            let phone = args[1..].iter().any(|a| a == "--phone");
            let serial = args[1..]
                .iter()
                .position(|a| a == "--serial")
                .and_then(|i| args.get(i + 2).cloned());
            Cmd::Uninstall { phone, serial }
        }
        "update-helper" => Cmd::UpdateHelper,
        "update-guard" => Cmd::UpdateGuard,
        "--version" | "-V" | "version" => Cmd::Version,
        "--help" | "-h" | "help" => Cmd::Help,
        other => anyhow::bail!("unknown subcommand {other:?} (try `111 help`)"),
    })
}

const USAGE: &str = "111 — the 111 Network node

USAGE:
  111 install [--allow-degraded] [--min-installer-version=CALVER]
  111 install --phone [--serial SERIAL] [--keep-adb]
  111 daemon
  111 identity backup|restore      (run as the node user: sudo -u node111 111 identity backup)
  111 identity commission [--loopback-test]   (Cat-1 relay ceremony; your own computer, needs a terminal)
  111 config [show | dashboard.lan true|false | offer accept|dismiss]
  111 dashboard [rotate]           (dashboard URLs + pair code; rotate = new code, sign everyone out)
  111 uninstall [--phone] [--serial SERIAL]
  111 update-helper | 111 update-guard     (root oneshots via systemd)
  111 --version";

pub fn main_entry(args: Vec<String>) -> i32 {
    let cmd = match parse(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("111: {e:#}");
            return 2;
        }
    };
    let result: anyhow::Result<()> = match cmd {
        Cmd::Help => {
            println!("{USAGE}");
            Ok(())
        }
        Cmd::Version => {
            println!("{BUILD_VERSION}");
            Ok(())
        }
        Cmd::Install(ia) => crate::install::run(ia),
        Cmd::Daemon => crate::daemon::run(),
        Cmd::IdentityBackup => identity_backup(),
        Cmd::IdentityRestore => identity_restore(),
        Cmd::IdentityCommission { loopback_test } => identity_commission(loopback_test),
        Cmd::IdentityGenerate { burn } => identity_generate(burn),
        Cmd::Config(rest) => crate::config::run(&rest),
        Cmd::Dashboard { rotate } => dashboard_cmd(rotate),
        Cmd::Uninstall { phone, serial } => {
            if phone {
                crate::install::phone::teardown::run(serial)
            } else {
                crate::uninstall::run_host()
            }
        }
        Cmd::UpdateHelper => crate::update_helper::run(),
        Cmd::UpdateGuard => crate::update_guard::run(),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("111: {e:#}");
            1
        }
    }
}

/// LAN sharing as configured on disk (env file OR settings.json). An
/// unreadable env file (non-root caller) counts as "not set there".
fn lan_configured_now(state_dir: &std::path::Path) -> bool {
    let etc = crate::config::env_path();
    let pairs = etc
        .parent()
        .and_then(|d| crate::daemon::load_env(d).ok())
        .unwrap_or_default();
    crate::daemon::dashboard::bind::lan_configured(&pairs, state_dir)
}

/// The pair URL encoded in the QR: the announced `.local` name if the daemon
/// bound one, else its first LAN IP URL, else `http://111.local`.
pub fn pair_url(urls: &[String], code: &str) -> String {
    let local = urls
        .iter()
        .find(|u| u.contains(".local") && !u.contains(".localhost"));
    let lan_ip = urls
        .iter()
        .find(|u| !u.contains("127.0.0.1") && !u.contains("localhost") && !u.contains(".local"));
    let base = local
        .or(lan_ip)
        .map(String::as_str)
        .unwrap_or("http://111.local");
    format!("{}/#/pair?code={code}", base.trim_end_matches('/'))
}

/// Terminal QR (unicode half blocks, inverted for dark terminals).
pub fn render_qr(data: &str) -> anyhow::Result<String> {
    use qrcode::render::unicode::Dense1x2;
    let code = qrcode::QrCode::new(data.as_bytes())?;
    Ok(code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .quiet_zone(true)
        .build())
}

/// `111 dashboard` / `111 dashboard rotate`.
fn dashboard_cmd(rotate: bool) -> anyhow::Result<()> {
    use crate::daemon::dashboard::{api_v1::auth, bind};
    let state_dir = crate::util::paths::root().join("state");
    if rotate {
        let code = auth::rotate_files(&state_dir).map_err(|e| {
            let denied = e
                .chain()
                .filter_map(|c| c.downcast_ref::<std::io::Error>())
                .any(|io| io.kind() == std::io::ErrorKind::PermissionDenied);
            if denied {
                anyhow::anyhow!("permission denied — run: sudo 111 dashboard rotate")
            } else {
                e
            }
        })?;
        println!("New pair code: {code}");
        println!("Every paired browser was signed out; LAN visitors pair again with the new code.");
        return Ok(());
    }

    let file = bind::read_state(&state_dir).filter(|f| bind::pid_alive(f.pid));
    let configured = lan_configured_now(&state_dir);
    let (urls, lan_running) = match &file {
        Some(f) => {
            println!(
                "111 dashboard — daemon running (pid {}, since {})",
                f.pid, f.started_at
            );
            if f.urls.is_empty() {
                println!("  (the daemon could not bind its dashboard port — see journalctl --namespace=111node -u 111node)");
            }
            (f.urls.clone(), Some(f.lan))
        }
        None => {
            println!("111 dashboard — daemon not running; once it starts the dashboard is at:");
            (vec![bind::FALLBACK_URL.to_string()], None)
        }
    };
    for u in &urls {
        println!("  {u}");
    }
    let lan_now = lan_running.unwrap_or(configured);
    match (lan_running, configured) {
        (Some(true), false) => println!(
            "LAN sharing: on (turned off in config — takes effect after: sudo systemctl restart 111node)"
        ),
        (Some(false), true) => println!(
            "LAN sharing: off (turned on in config — takes effect after: sudo systemctl restart 111node)"
        ),
        (_, _) if lan_now => println!("LAN sharing: on — visitors on your network must pair"),
        _ => println!("LAN sharing: off — enable with: sudo 111 config dashboard.lan true"),
    }
    match auth::read_pair_code(&state_dir) {
        Ok(Some(code)) => {
            println!("Pair code:   {code}   (LAN visitors enter it; this computer never needs it)");
            if lan_now {
                let url = pair_url(&urls, &code);
                match render_qr(&url) {
                    Ok(qr) => println!("\nScan to pair: {url}\n{qr}"),
                    Err(_) => println!("Pair link: {url}"),
                }
            }
        }
        Ok(None) => {
            println!("Pair code:   not generated yet — the daemon creates it at first start")
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            println!(
                "Pair code:   readable by root or the node user only — run: sudo 111 dashboard"
            )
        }
        Err(e) => println!("Pair code:   unreadable ({e})"),
    }
    Ok(())
}

fn identity_dir() -> std::path::PathBuf {
    crate::util::paths::root().join("identity")
}

fn print_node_id(d: &id111::encoding::NodeIdDisplay) {
    println!("  label   {}", d.label());
    println!("  node1…  {}", d.bech32());
    println!("  hex     {}", d.hex());
}

/// `111 identity backup` — the §8.3 local-TTY ceremony display (the TTY-only
/// refusal lives in `identity::backup`, so no caller can bypass it). The QR
/// frame and the verify-on-write-down gate arrive with the shared ceremony
/// driver (T11).
fn identity_backup() -> anyhow::Result<()> {
    let view = crate::identity::backup(&identity_dir())?;
    let words = view.artifacts.words();
    println!(
        "Your 111 node identity ({} words — write them down, keep them secret):\n",
        words.split_whitespace().count()
    );
    for (i, w) in words.split_whitespace().enumerate() {
        println!("  {:2}. {w}", i + 1);
    }
    println!(
        "\nBackup descriptor (write it under the words): {}",
        view.artifacts.descriptor
    );
    println!("\nNodeId (record it — `111 identity restore` asks you to confirm it):");
    print_node_id(&view.display());
    println!("\nRestore with: 111 identity restore");
    Ok(())
}

/// Read a line from the tty with echo disabled (termios ECHO toggle, restored
/// on every path). Used for the recovery words — the master secret.
fn read_hidden(prompt: &str) -> anyhow::Result<String> {
    use std::io::Write;
    eprint!("{prompt}");
    std::io::stderr().flush()?;

    let mut old: libc::termios = unsafe { std::mem::zeroed() };
    anyhow::ensure!(
        unsafe { libc::tcgetattr(0, &mut old) } == 0,
        "tcgetattr failed"
    );
    let mut new = old;
    new.c_lflag &= !libc::ECHO;
    unsafe { libc::tcsetattr(0, libc::TCSANOW, &new) };

    let mut line = String::new();
    let read_result = std::io::stdin().read_line(&mut line);

    unsafe { libc::tcsetattr(0, libc::TCSANOW, &old) };
    eprintln!(); // newline after the hidden input
    read_result?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

fn read_visible(prompt: &str) -> anyhow::Result<String> {
    use std::io::Write;
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn confirm(prompt: &str) -> anyhow::Result<bool> {
    let a = read_visible(&format!("{prompt} [y/N] "))?;
    Ok(a.eq_ignore_ascii_case("y") || a.eq_ignore_ascii_case("yes"))
}

/// `111 identity restore` (L2 §8.4) — words on a local TTY only. Order matters:
/// refuse a live dir BEFORE asking for words → hygiene + checksum + derive
/// (writes nothing) → show the NodeId and require the operator's confirmation
/// against their recorded reference → require old-instance retirement → only
/// then the atomic write. Declining at any prompt leaves the disk untouched.
fn identity_restore() -> anyhow::Result<()> {
    anyhow::ensure!(
        unsafe { libc::isatty(0) } == 1,
        "identity restore requires an interactive terminal (words are never read from a pipe)"
    );
    let dir = identity_dir();
    if crate::identity::is_live(&dir) {
        return Err(crate::identity::IdentityError::Exists.into());
    }
    let words = zeroize::Zeroizing::new(read_hidden(
        "enter your recovery words (12 or 24, input hidden): ",
    )?);
    let descriptor = crate::identity::parse_descriptor_hex(&read_visible(
        "descriptor printed under the words (8 hex digits; Enter = v1 defaults): ",
    )?)?;
    let node_id = crate::identity::preview_restore(&words)?;
    println!("These words derive NodeId:");
    print_node_id(&id111::encoding::NodeIdDisplay::new(node_id.as_bytes()));
    anyhow::ensure!(
        confirm("Does this match the NodeId you recorded for this node?")?,
        "restore aborted: NodeId not confirmed (nothing written)"
    );
    anyhow::ensure!(
        confirm(
            "Is the OLD instance of this identity retired (powered off / wiped)? \
             Two live instances on one identity flap announces fleet-wide."
        )?,
        "restore aborted: old instance not confirmed retired (nothing written)"
    );
    let outcome = crate::identity::restore(&dir, &words, descriptor, true)?;
    println!("identity restored:");
    print_node_id(&outcome.display());
    if outcome.counter_floor_pending {
        println!(
            "note: words-only recovery — the announce-counter floor is unknown (provisional {}); \
             this node must not announce until the floor is learned from the mesh (v1 nodes do not announce yet).",
            outcome.announce_counter_init
        );
    }
    println!("Restart the node daemon to pick the identity up.");
    Ok(())
}

/// `111 identity commission` (L2 §8.3, Cat-1 ceremony, commissioner side).
/// Runs on the operator's own computer — it touches no identity dir and no
/// disk at all. TTY on stdin AND stdout is required BEFORE any keygen (the
/// words are printed, the type-back is read hidden); the ceremony itself is
/// `id111::commission`'s state machine, driven by `identity::commission`.
///
/// Exit status 0 means "a relay was provisioned and challenge-verified". Until
/// the relay transport exists (§15) only `--loopback-test` can say that; the
/// real path ends in a dry run and reports failure rather than letting a
/// script mistake it for a commissioned relay.
fn identity_commission(loopback_test: bool) -> anyhow::Result<()> {
    use crate::identity::commission::{self as cer, Outcome, Transport};
    cer::require_stdio_tty()?;
    let transport = if loopback_test {
        Transport::Loopback
    } else {
        Transport::Pending
    };
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut hidden = |p: &str| read_hidden(p);
    let mut visible = |p: &str| read_visible(p);
    let mut log = |line: &str| eprintln!("111 identity commission: {line}");
    let mut io = cer::Io {
        out: &mut out,
        read_hidden: &mut hidden,
        read_visible: &mut visible,
        log: &mut log,
    };
    match cer::run(transport, &mut io)? {
        Outcome::Commissioned(_) => Ok(()),
        Outcome::Pending(_) => Err(anyhow::anyhow!(
            "relay transport pending (L2 §15): no relay was provisioned (dry run; commissioner copies wiped)"
        )),
    }
}

/// Internal idempotent keygen (phone flow step 6): load; generate only when
/// `Missing`; a corrupt identity is quarantined by `load` and surfaces as an
/// error — never a silent re-key. Prints exactly one line: the NodeId hex.
fn identity_generate(burn: bool) -> anyhow::Result<()> {
    let dir = identity_dir();
    let id = match crate::identity::load(&dir) {
        Ok(id) => id,
        Err(e) if crate::identity::is_missing(&e) => {
            if burn {
                crate::identity::generate_burn(&dir)?
            } else {
                crate::identity::generate(&dir)?
            }
        }
        Err(e) => return Err(e),
    };
    println!("{}", id.node_id_hex());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_matrix() {
        assert!(matches!(
            parse(&["install".into()]).unwrap(),
            Cmd::Install(_)
        ));
        let ia = match parse(&[
            "install".into(),
            "--phone".into(),
            "--serial".into(),
            "abc123".into(),
            "--keep-adb".into(),
        ])
        .unwrap()
        {
            Cmd::Install(ia) => ia,
            other => panic!("{other:?}"),
        };
        assert!(ia.phone && ia.keep_adb);
        assert_eq!(ia.serial.as_deref(), Some("abc123"));
        assert!(matches!(
            parse(&["identity".into(), "backup".into()]).unwrap(),
            Cmd::IdentityBackup
        ));
        assert!(matches!(
            parse(&["identity".into(), "restore".into()]).unwrap(),
            Cmd::IdentityRestore
        ));
        assert!(matches!(
            parse(&["identity".into(), "commission".into()]).unwrap(),
            Cmd::IdentityCommission {
                loopback_test: false
            }
        ));
        assert!(matches!(
            parse(&[
                "identity".into(),
                "commission".into(),
                "--loopback-test".into()
            ])
            .unwrap(),
            Cmd::IdentityCommission {
                loopback_test: true
            }
        ));
        // unknown flags are refused, never ignored (a typo must not silently
        // select the other transport mode)
        assert!(parse(&[
            "identity".into(),
            "commission".into(),
            "--loopbak-test".into()
        ])
        .is_err());
        assert!(parse(&[
            "identity".into(),
            "commission".into(),
            "--loopback-test".into(),
            "extra".into()
        ])
        .is_err());
        // L2 §13: `recover` is subsumed by `restore`; NIP-49 `export` is deleted.
        assert!(parse(&["identity".into(), "export".into()]).is_err());
        assert!(parse(&["identity".into(), "recover".into()]).is_err());
        // shim passes --min-installer-version=X (with =) — must parse
        let ia2 = match parse(&[
            "install".into(),
            "--min-installer-version=2026.10.01".into(),
        ])
        .unwrap()
        {
            Cmd::Install(ia) => ia,
            other => panic!("{other:?}"),
        };
        assert_eq!(ia2.min_installer_version.unwrap().to_string(), "2026.10.01");
        assert!(parse(&["identity".into(), "wut".into()]).is_err());
        assert!(matches!(
            parse(&["dashboard".into()]).unwrap(),
            Cmd::Dashboard { rotate: false }
        ));
        assert!(matches!(
            parse(&["dashboard".into(), "rotate".into()]).unwrap(),
            Cmd::Dashboard { rotate: true }
        ));
        assert!(parse(&["dashboard".into(), "rotat".into()]).is_err());
        assert!(parse(&["dashboard".into(), "rotate".into(), "x".into()]).is_err());
        assert!(parse(&["bogus".into()]).is_err());
        assert!(matches!(
            parse(&["uninstall".into(), "--phone".into()]).unwrap(),
            Cmd::Uninstall { phone: true, .. }
        ));
    }

    #[test]
    fn pair_url_prefers_mdns_then_lan_ip() {
        let urls = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            pair_url(
                &urls(&[
                    "http://111.localhost",
                    "http://127.0.0.1:11180",
                    "http://111.local",
                    "http://192.168.1.5"
                ]),
                "7KQ2-XM9D"
            ),
            "http://111.local/#/pair?code=7KQ2-XM9D"
        );
        assert_eq!(
            pair_url(
                &urls(&["http://127.0.0.1:11180", "http://192.168.1.5:11180"]),
                "X"
            ),
            "http://192.168.1.5:11180/#/pair?code=X"
        );
        assert_eq!(pair_url(&[], "X"), "http://111.local/#/pair?code=X");
        let qr = render_qr("http://111.local/#/pair?code=7KQ2-XM9D").unwrap();
        assert!(qr.lines().count() > 10);
    }

    #[test]
    fn commission_refuses_without_a_terminal_before_any_keygen() {
        if unsafe { libc::isatty(0) } == 1 && unsafe { libc::isatty(1) } == 1 {
            return; // `--nocapture` on a real terminal: the refusal is not exercisable here
        }
        for loopback in [false, true] {
            let err = identity_commission(loopback).unwrap_err().to_string();
            assert!(err.contains("interactive terminal"), "{err}");
        }
    }
}
