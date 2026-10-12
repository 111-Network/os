//! adb wrapper — always runs as the INVOKING user (adb authorization is
//! per-user; running adb as root silently targets a different authorization
//! set, so `--phone` refuses root outright).

use crate::util::run::{Output, Runner};

/// Minimum adb version (spec preflight; Android 12+ reliability floor).
pub const MIN_ADB_VERSION: (u32, u32, u32) = (1, 0, 41);

pub struct Adb<'a> {
    pub runner: &'a dyn Runner,
    pub serial: Option<String>,
    pub adb_path: String,
}

impl<'a> Adb<'a> {
    pub fn new(runner: &'a dyn Runner, serial: Option<String>, adb_path: &str) -> Self {
        Adb {
            runner,
            serial,
            adb_path: adb_path.to_string(),
        }
    }

    /// The phone flow must never run as root (per-user adb authorization).
    pub fn refuse_root() -> anyhow::Result<()> {
        anyhow::ensure!(
            unsafe { libc::geteuid() } != 0,
            "`111 install --phone` must run as the invoking user, NOT root — adb authorization is per-user. Run it as your normal user."
        );
        Ok(())
    }

    fn argv<'b>(&'b self, extra: &[&'b str]) -> Vec<&'b str> {
        let mut v: Vec<&str> = Vec::new();
        if let Some(s) = &self.serial {
            v.push("-s");
            v.push(s.as_str());
        }
        v.extend_from_slice(extra);
        v
    }

    pub fn cmd(&self, args: &[&str]) -> anyhow::Result<Output> {
        self.runner.run(&self.adb_path, &self.argv(args))
    }

    pub fn cmd_stdin(&self, args: &[&str], input: &[u8]) -> anyhow::Result<Output> {
        self.runner
            .run_stdin(&self.adb_path, &self.argv(args), input)
    }

    pub fn shell(&self, cmd: &str) -> anyhow::Result<Output> {
        self.cmd(&["shell", cmd])
    }

    pub fn shell_stdin(&self, cmd: &str, input: &[u8]) -> anyhow::Result<Output> {
        self.cmd_stdin(&["shell", cmd], input)
    }

    /// (serial, state) pairs from `adb devices`.
    pub fn devices(&self) -> anyhow::Result<Vec<(String, String)>> {
        let out = self.cmd(&["devices"])?;
        anyhow::ensure!(out.ok(), "adb devices failed: {}", out.stderr.trim());
        Ok(out
            .stdout
            .lines()
            .skip(1)
            .filter_map(|l| {
                let mut parts = l.split_whitespace();
                let (Some(serial), Some(state)) = (parts.next(), parts.next()) else {
                    return None;
                };
                if serial.is_empty() {
                    None
                } else {
                    Some((serial.to_string(), state.to_string()))
                }
            })
            .collect())
    }

    pub fn getprop(&self, key: &str) -> anyhow::Result<String> {
        let out = self.shell(&format!("getprop {key}"))?;
        if !out.ok() {
            anyhow::bail!("getprop {key} failed: {}", out.stderr.trim());
        }
        Ok(out.stdout.trim().to_string())
    }

    /// getprop that may legitimately be empty/absent.
    pub fn getprop_opt(&self, key: &str) -> Option<String> {
        self.getprop(key).ok().filter(|s| !s.is_empty())
    }

    pub fn install(&self, apk: &str) -> anyhow::Result<Output> {
        self.cmd(&["install", "-r", apk])
    }

    pub fn push(&self, local: &str, remote: &str) -> anyhow::Result<Output> {
        self.cmd(&["push", local, remote])
    }

    pub fn forward(&self, spec: &str) -> anyhow::Result<Output> {
        self.cmd(&["forward", spec])
    }

    pub fn reverse(&self, spec: &str) -> anyhow::Result<Output> {
        self.cmd(&["reverse", spec])
    }
}

/// Parse `adb version` stdout into (major, minor, patch-ish).
pub fn parse_adb_version(text: &str) -> Option<(u32, u32, u32)> {
    // "Android Debug Bridge version 1.0.41"
    let line = text.lines().next()?;
    let ver = line.split_whitespace().nth(4)?;
    let mut parts = ver.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing() {
        assert_eq!(
            parse_adb_version("Android Debug Bridge version 1.0.41\n"),
            Some((1, 0, 41))
        );
        assert_eq!(
            parse_adb_version("Android Debug Bridge version 34.0.4\n"),
            Some((34, 0, 4))
        );
        assert!(parse_adb_version("garbage").is_none());
        assert!(MIN_ADB_VERSION <= (1, 0, 41));
    }
}
