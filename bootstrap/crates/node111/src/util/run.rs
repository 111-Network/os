//! Command execution behind a trait so installer/helper paths are testable
//! without touching the host.

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
}

pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        let _ = (program, args);
        todo!("T2")
    }
    fn run_as(&self, user: &str, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        let _ = (user, program, args);
        todo!("T2")
    }
}
