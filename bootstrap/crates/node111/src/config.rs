//! `111 config` — minimal node config in /etc/111 (env file 0640 root:node111).
//! LAN dashboard binding is an explicit opt-in here with a printed warning until
//! Layer 4 auth exists.

pub fn run(args: &[String]) -> anyhow::Result<()> {
    let _ = args;
    todo!("T14")
}
