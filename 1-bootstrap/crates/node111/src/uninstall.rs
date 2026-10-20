//! `111 uninstall` (host): full teardown — units, udev rule, journald namespace,
//! node111 user, node trees, symlinks. Receipt-driven where possible, root
//! required. Phone teardown lives in `install::phone::teardown`.

use crate::util::run::Runner;

const UNIT_FILES: &[&str] = &[
    "/etc/systemd/system/111node.service",
    "/etc/systemd/system/111node-update.path",
    "/etc/systemd/system/111node-update.service",
    "/etc/systemd/journald.111node.conf",
    "/etc/udev/rules.d/99-111-lora.rules",
    "/etc/systemd/system/111node.service.d/10-container.conf",
];

/// The full pass. `runner` executes system actions (MockRunner in tests);
/// `fs_root` confines file removal ("/" in production).
pub fn run_with(runner: &dyn Runner, fs_root: &std::path::Path) -> anyhow::Result<()> {
    // stop + disable first (best-effort: units may not exist)
    let _ = runner.run(
        "systemctl",
        &["stop", "111node.service", "111node-update.path"],
    );
    let _ = runner.run(
        "systemctl",
        &[
            "disable",
            "111node.service",
            "111node-update.path",
            "111node-update.service",
        ],
    );

    for f in UNIT_FILES {
        let path = fs_root.join(f.trim_start_matches('/'));
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
    }
    let dropin_dir = fs_root.join("etc/systemd/system/111node.service.d");
    if dropin_dir.exists() {
        let _ = std::fs::remove_dir(&dropin_dir); // only when empty
    }

    let _ = runner.run("systemctl", &["daemon-reload"]);
    let _ = runner.run("udevadm", &["control", "--reload"]);
    let _ = runner.run("udevadm", &["trigger"]);

    // trees
    for dir in ["var/lib/111", "etc/111"] {
        let path = fs_root.join(dir);
        if path.exists() {
            std::fs::remove_dir_all(&path)?;
        }
    }
    for link in ["usr/local/bin/111", "usr/local/bin/111-uninstall"] {
        let path = fs_root.join(link);
        if path.symlink_metadata().is_ok() {
            std::fs::remove_file(&path)?;
        }
    }

    // user (best-effort)
    let _ = runner.run("userdel", &["node111"]);

    Ok(())
}

/// Production entry (`111 uninstall`, root).
pub fn run_host() -> anyhow::Result<()> {
    anyhow::ensure!(
        unsafe { libc::geteuid() } == 0,
        "uninstall needs root — run: sudo 111 uninstall"
    );
    run_with(&crate::util::run::SystemRunner, std::path::Path::new("/"))?;
    println!("111 node removed (units, udev rule, journald namespace, node111 user, /var/lib/111, /etc/111, symlinks).");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teardown_on_temp_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // plant the world
        for f in UNIT_FILES {
            let p = root.join(f.trim_start_matches('/'));
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "x").unwrap();
        }
        std::fs::create_dir_all(root.join("var/lib/111/state")).unwrap();
        std::fs::create_dir_all(root.join("etc/111")).unwrap();
        let link_dir = root.join("usr/local/bin");
        std::fs::create_dir_all(&link_dir).unwrap();
        std::os::unix::fs::symlink("/var/lib/111/current/bin/111", link_dir.join("111")).unwrap();
        let runner = crate::testkit::MockRunner::new(vec![
            (
                "systemctl stop 111node.service 111node-update.path".to_string(),
                crate::util::run::Output {
                    status: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            ),
            (
                "systemctl disable 111node.service 111node-update.path 111node-update.service"
                    .to_string(),
                crate::util::run::Output {
                    status: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            ),
            (
                "systemctl daemon-reload".to_string(),
                crate::util::run::Output {
                    status: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            ),
            (
                "udevadm control --reload".to_string(),
                crate::util::run::Output {
                    status: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            ),
            (
                "udevadm trigger".to_string(),
                crate::util::run::Output {
                    status: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            ),
            (
                "userdel node111".to_string(),
                crate::util::run::Output {
                    status: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            ),
        ]);
        run_with(&runner, root).unwrap();
        for f in UNIT_FILES {
            assert!(!root.join(f.trim_start_matches('/')).exists(), "{f} left");
        }
        assert!(!root.join("var/lib/111").exists());
        assert!(!root.join("etc/111").exists());
        assert!(!root.join("usr/local/bin/111").exists());
        runner.assert_exhausted();
    }
}
