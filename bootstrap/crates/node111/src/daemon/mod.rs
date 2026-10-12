//! `111 daemon` — the node kernel: tokio supervision tree (systemd/runit are thin
//! Restart=always wrappers, never the supervisor of record), watchdog heartbeat,
//! dashboard, updater, KB server, conditional rnsd child.

pub mod dashboard;
pub mod health;
pub mod kb;
pub mod supervise;
pub mod transport;
pub mod updater;
pub mod watchdog;

pub fn run() -> anyhow::Result<()> {
    todo!("T7")
}
