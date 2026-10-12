//! Step 7 — supervise: node111 system user (getent-detected dialout/uucp), unit
//! + degraded drop-in + udev rule + journald namespace, daemon-reload, udevadm
//!   reload+trigger, enable --now. No sudoers, no polkit (spec §Supervision).

use super::context::InstallContext;

const UNIT: &str = include_str!("../../../../systemd/111node.service");
const UPDATE_PATH: &str = include_str!("../../../../systemd/111node-update.path");
const UPDATE_SERVICE: &str = include_str!("../../../../systemd/111node-update.service");
const JOURNALD_NS: &str = include_str!("../../../../systemd/journald.111node.conf");
const DEGRADED_DROPIN: &str = include_str!("../../../../systemd/degraded.d/10-container.conf");
const UDEV_RULE: &str = include_str!("../../../../systemd/99-111-lora.rules");

/// Present supplementary groups, getent-detected (dialout/uucp per spec).
fn present_groups(ctx: &InstallContext) -> Vec<String> {
    ["dialout", "uucp"]
        .iter()
        .filter(|g| {
            ctx.runner
                .run("getent", &["group", g])
                .map(|o| o.ok())
                .unwrap_or(false)
        })
        .map(|g| g.to_string())
        .collect()
}

fn write(ctx: &InstallContext, absolute: &str, content: &str, mode: u32) -> anyhow::Result<()> {
    let path = ctx.sys(absolute);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::util::fs::atomic_write(&path, content.as_bytes(), mode)?;
    Ok(())
}

pub fn run(ctx: &InstallContext, degraded: bool) -> anyhow::Result<()> {
    // --- node111 system user ---
    let exists = ctx
        .runner
        .run("id", &["node111"])
        .map(|o| o.ok())
        .unwrap_or(false);
    if !exists {
        let out = ctx.runner.run(
            "useradd",
            &[
                "--system",
                "--home-dir",
                "/var/lib/111",
                "--shell",
                "/usr/sbin/nologin",
                "node111",
            ],
        )?;
        anyhow::ensure!(out.ok(), "useradd node111 failed: {}", out.stderr.trim());
    }
    let groups = present_groups(ctx);
    for g in &groups {
        let _ = ctx.runner.run("usermod", &["-aG", g, "node111"]);
    }

    // --- unit render: SupplementaryGroups from present groups only ---
    let unit = UNIT.replace(
        "SupplementaryGroups=dialout uucp",
        &format!("SupplementaryGroups={}", groups.join(" ")),
    );
    write(ctx, "/etc/systemd/system/111node.service", &unit, 0o644)?;
    write(
        ctx,
        "/etc/systemd/system/111node-update.path",
        UPDATE_PATH,
        0o644,
    )?;
    write(
        ctx,
        "/etc/systemd/system/111node-update.service",
        UPDATE_SERVICE,
        0o644,
    )?;
    write(
        ctx,
        "/etc/systemd/journald.111node.conf",
        JOURNALD_NS,
        0o644,
    )?;
    write(ctx, "/etc/udev/rules.d/99-111-lora.rules", UDEV_RULE, 0o644)?;
    if degraded {
        write(
            ctx,
            "/etc/systemd/system/111node.service.d/10-container.conf",
            DEGRADED_DROPIN,
            0o644,
        )?;
    }

    // --- minimal config env (/etc/111/env, 0640 root:node111) ---
    let env_content = "# 111 node config (minimal; `111 config` manages)\n";
    write(ctx, "/etc/111/env", env_content, 0o640)?;

    // --- ownership: node111 trees vs root trees (the security boundary) ---
    // node111: state/, state/staging/, identity/, rns/, venvs/
    // root:    releases/, current, cache/, receipt.json, state/update.json, state/floor.json
    let node_dirs = [
        ctx.layout.state_dir(),
        ctx.layout.state_dir().join("staging"),
        ctx.layout.identity_dir(),
        ctx.layout.root.join("rns"),
        ctx.layout.root.join("venvs"),
    ];
    for dir in &node_dirs {
        std::fs::create_dir_all(dir)?;
        chown_tree(ctx, dir)?;
    }
    let _ = ctx.runner.run(
        "chmod",
        &["0700", &ctx.layout.identity_dir().to_string_lossy()],
    );

    // --- /usr/local/bin/111-uninstall wrapper (spec layout) ---
    write(
        ctx,
        "/usr/local/bin/111-uninstall",
        "#!/bin/sh\n# 111 node uninstall wrapper — see 111 uninstall\nexec /usr/local/bin/111 uninstall \"$@\"\n",
        0o755,
    )?;

    // --- reload + enable ---
    must(ctx, "systemctl", &["daemon-reload"])?;
    let _ = ctx.runner.run("udevadm", &["control", "--reload"]);
    let _ = ctx.runner.run("udevadm", &["trigger"]);
    must(
        ctx,
        "systemctl",
        &["enable", "--now", "111node.service", "111node-update.path"],
    )?;
    Ok(())
}

fn must(ctx: &InstallContext, program: &str, args: &[&str]) -> anyhow::Result<()> {
    let out = ctx.runner.run(program, args)?;
    anyhow::ensure!(
        out.ok(),
        "{program} {} failed: {}",
        args.join(" "),
        out.stderr.trim()
    );
    Ok(())
}

/// chown -R node111:node111 (test environments without the user just log).
fn chown_tree(ctx: &InstallContext, dir: &std::path::Path) -> anyhow::Result<()> {
    let out = ctx
        .runner
        .run("chown", &["-R", "node111:node111", &dir.to_string_lossy()])?;
    if !out.ok() {
        eprintln!(
            "111 install: chown {} failed ({}); continuing — dev/rootless environments lack node111",
            dir.display(),
            out.stderr.trim()
        );
    }
    Ok(())
}
