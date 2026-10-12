//! HTTPS fetching (ureq + rustls + webpki-roots — no native cert store dependency,
//! deterministic on musl). Used by installer, updater, and prefetch steps.

use std::time::Duration;

pub struct Fetcher {
    // ureq agent, timeout, response size cap — construction detail is T2's.
    _priv: (),
}

impl Fetcher {
    pub fn new(timeout: Duration, max_bytes: u64) -> anyhow::Result<Self> {
        let _ = (timeout, max_bytes);
        todo!("T2")
    }

    /// GET `url`, return body (capped). Errors carry the URL.
    pub fn get(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        let _ = url;
        todo!("T2")
    }

    /// HEAD `url`, returning the HTTP Date header value when present (clock sanity).
    pub fn head_date(&self, url: &str) -> anyhow::Result<Option<String>> {
        let _ = url;
        todo!("T2")
    }
}
