//! HTTPS fetching (ureq + rustls + webpki-roots — no native cert store dependency,
//! deterministic on musl). Used by installer, updater, and prefetch steps.
//!
//! Production callers pass **https** first-party endpoints only (GitHub Releases +
//! the own-domain mirror, per spec v2.1 "Security & trust model"). `http://` URLs
//! are not rejected by this wrapper so tests can use loopback fixture servers;
//! nothing in production code may pass one.

use anyhow::Context;
use std::time::Duration;

pub struct Fetcher {
    agent: ureq::Agent,
    max_bytes: u64,
}

impl Fetcher {
    /// `timeout` bounds connect AND the whole call; bodies are capped at `max_bytes`
    /// (a larger body is a hard error, not a truncation).
    pub fn new(timeout: Duration, max_bytes: u64) -> anyhow::Result<Self> {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::WebPki)
            .build();
        let config = ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_connect(Some(timeout))
            .timeout_global(Some(timeout))
            .build();
        Ok(Fetcher {
            agent: ureq::Agent::new_with_config(config),
            max_bytes,
        })
    }

    /// GET `url`, return body (capped). Errors carry the URL.
    pub fn get(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        let mut resp = self
            .agent
            .get(url)
            .call()
            .with_context(|| format!("GET {url}"))?;
        let body = resp
            .body_mut()
            .with_config()
            .limit(self.max_bytes)
            .read_to_vec()
            .with_context(|| format!("GET {url}: reading body (cap {} bytes)", self.max_bytes))?;
        Ok(body)
    }

    /// HEAD `url`, returning the HTTP Date header value when present (clock sanity).
    pub fn head_date(&self, url: &str) -> anyhow::Result<Option<String>> {
        let resp = self
            .agent
            .head(url)
            .call()
            .with_context(|| format!("HEAD {url}"))?;
        Ok(resp
            .headers()
            .get("date")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned))
    }
}
