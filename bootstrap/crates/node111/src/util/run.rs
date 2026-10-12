//! Command execution behind a trait so installer/helper paths are testable
//! without touching the host.

use anyhow::Context;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.status == 0
    }
}

pub trait Runner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output>;
    /// Run as another user (`sudo -u`); the installer is root when it uses this.
    fn run_as(&self, user: &str, program: &str, args: &[&str]) -> anyhow::Result<Output>;
    /// Run with bytes on stdin (file pushes over `sh -c 'cat > …'` etc.).
    fn run_stdin(&self, program: &str, args: &[&str], input: &[u8]) -> anyhow::Result<Output> {
        let _ = (program, args, input);
        anyhow::bail!("run_stdin not supported by this Runner")
    }
}

pub struct SystemRunner;

impl SystemRunner {
    fn spawn(mut cmd: std::process::Command) -> anyhow::Result<Output> {
        let display = format!("{:?}", cmd);
        let out = cmd.output().with_context(|| format!("spawn {display}"))?;
        let status = match out.status.code() {
            Some(code) => code,
            None => {
                // Killed by a signal: report as -signal (e.g. SIGKILL => -9).
                use std::os::unix::process::ExitStatusExt;
                -out.status.signal().unwrap_or(0)
            }
        };
        Ok(Output {
            status,
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        Self::spawn(cmd)
    }

    fn run_as(&self, user: &str, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        let mut cmd = std::process::Command::new("sudo");
        cmd.arg("-u").arg(user).arg("--").arg(program).args(args);
        Self::spawn(cmd)
    }

    fn run_stdin(&self, program: &str, args: &[&str], input: &[u8]) -> anyhow::Result<Output> {
        use std::io::Write;
        let mut child = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("spawn {program}"))?;
        child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("stdin pipe"))?
            .write_all(input)?;
        let display = format!("{program} {}", args.join(" "));
        let out = child
            .wait_with_output()
            .with_context(|| format!("wait {display}"))?;
        let status = match out.status.code() {
            Some(code) => code,
            None => {
                use std::os::unix::process::ExitStatusExt;
                -out.status.signal().unwrap_or(0)
            }
        };
        Ok(Output {
            status,
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_stdout_and_status() {
        let out = SystemRunner.run("sh", &["-c", "echo hi; exit 3"]).unwrap();
        assert_eq!(out.status, 3);
        assert_eq!(out.stdout.trim(), "hi");
        assert!(!out.ok());
    }

    #[test]
    fn signal_is_negative_status() {
        let out = SystemRunner.run("sh", &["-c", "kill -TERM $$"]).unwrap();
        assert_eq!(out.status, -15);
    }

    #[test]
    fn spawn_failure_is_err() {
        assert!(SystemRunner
            .run("definitely-not-a-real-binary-111", &[])
            .is_err());
    }
}
