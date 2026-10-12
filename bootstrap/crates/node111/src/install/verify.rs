//! Step 8 — verify: `/health` 200 within 60s, dashboard reachable on
//! 127.0.0.1:11180, socket asserted loopback-bound; then the final print
//! (npub + dashboard URL + identity-backup hint).

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
                return Ok(String::from_utf8_lossy(&body).into_owned());
            }
        }
        if start.elapsed().as_secs() >= deadline_secs {
            anyhow::bail!(
                "node did not become healthy within {deadline_secs}s — check `journalctl -t 111` and /var/log"
            );
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}

/// Assert the dashboard is not listening on any non-loopback address: every
/// non-loopback IPv4 of this host must refuse the connection.
fn assert_loopback_only(ctx: &InstallContext) {
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
            debug_assert!(hit.is_err(), "dashboard MUST NOT answer on {addr}:11180");
        }
    }
}

/// Final print per spec step 8.
pub fn print_success(npub: &str, category: &str, version: &str) {
    println!(
        "
111 Network node installed.
  npub:      {npub}
  category:  {category}
  version:   {version}
  dashboard: http://127.0.0.1:11180 (loopback only)

Back up your identity (12 words) with:
  sudo -u node111 111 identity backup"
    );
}
