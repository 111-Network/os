//! CLI surface: install · daemon · identity backup|export|recover · config ·
//! uninstall · update-helper · update-guard.
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
    /// Shim-forwarded floor (the binary also refuses below the compiled-in
    /// MIN_INSTALLER_VERSION).
    pub min_installer_version: Option<crate::calver::Calver>,
}

#[derive(Debug)]
enum Cmd {
    Install(InstallArgs),
    Daemon,
    IdentityBackup,
    IdentityExport,
    IdentityRecover,
    Config(Vec<String>),
    Uninstall { phone: bool, serial: Option<String> },
    UpdateHelper,
    UpdateGuard,
    Version,
    Help,
}

fn parse_install_flags(args: &[String]) -> anyhow::Result<InstallArgs> {
    let mut ia = InstallArgs::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--phone" => ia.phone = true,
            "--keep-adb" => ia.keep_adb = true,
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
            Some("export") => Cmd::IdentityExport,
            Some("recover") => Cmd::IdentityRecover,
            other => anyhow::bail!("usage: 111 identity backup|export|recover (got {other:?})"),
        },
        "config" => Cmd::Config(args[1..].to_vec()),
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
  111 identity backup|export|recover
  111 config [show | dashboard.lan true|false | offer accept|dismiss]
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
        Cmd::IdentityExport => identity_export(),
        Cmd::IdentityRecover => identity_recover(),
        Cmd::Config(rest) => crate::config::run(&rest),
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

fn identity_dir() -> std::path::PathBuf {
    crate::util::paths::root().join("identity")
}

fn identity_backup() -> anyhow::Result<()> {
    let words = crate::identity::backup(&identity_dir())?;
    println!("Your 111 node identity (12 words — write them down, keep them secret):\n");
    for (i, w) in words.split_whitespace().enumerate() {
        println!("  {:2}. {w}", i + 1);
    }
    println!("\nRecover with: 111 identity recover");
    Ok(())
}

fn identity_export() -> anyhow::Result<()> {
    let ncryptsec = crate::identity::export_ncryptsec(&identity_dir())?;
    println!("{ncryptsec}");
    Ok(())
}

fn identity_recover() -> anyhow::Result<()> {
    anyhow::ensure!(
        unsafe { libc::isatty(0) } == 1,
        "identity recover requires an interactive terminal"
    );
    eprint!("enter your 12 words: ");
    use std::io::Write;
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let id = crate::identity::recover(&identity_dir(), line.trim())?;
    println!("identity recovered: {}", id.npub);
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
        assert!(parse(&["identity".into(), "export".into()]).is_ok());
        assert!(parse(&["identity".into(), "wut".into()]).is_err());
        assert!(parse(&["bogus".into()]).is_err());
        assert!(matches!(
            parse(&["uninstall".into(), "--phone".into()]).unwrap(),
            Cmd::Uninstall { phone: true, .. }
        ));
    }
}
