//! 111-agent — the thin always-on agent loop of Layer 3 (Local Intelligence).
//!
//! Capability only: chat backend with KB grounding, a fixed tool set, a task
//! API for Layer 9, and an offload client mode. Behavior (monitor cadence,
//! diagnosis playbooks, sandbox, PR gate) is Layer 9's, per D10.

pub mod chat;
pub mod config;
pub mod events;
pub mod health;
pub mod http;
pub mod kb;
pub mod model;
pub mod status;
pub mod store;
pub mod tasks;
pub mod tools;

use std::net::Ipv4Addr;
use std::sync::Arc;

/// Boot the agent: load config, open state, serve the HTTP contract.
/// This is the process 111d spawns and supervises as a child (rnsd pattern).
pub async fn run() -> anyhow::Result<()> {
    let cfg = Arc::new(config::Config::load()?);
    log_line(&format!(
        "starting 111-agent v{} (category={}, kb={}, model_base={})",
        env!("CARGO_PKG_VERSION"),
        cfg.category,
        cfg.kb_dir.display(),
        cfg.model_base
    ));

    let store = Arc::new(store::Store::init(&cfg.state_dir)?);
    let kb = Arc::new(kb::Kb::open(&cfg.kb_dir));
    let model = Arc::new(model::Client::new(&cfg.model_base, &cfg.model_id));
    let journal = events::Journal::new(store.events_path());
    let tasks = Arc::new(tasks::TaskEngine::new(
        cfg.clone(),
        store.clone(),
        journal.clone(),
        model.clone(),
    ));

    let state = Arc::new(http::AppState {
        cfg: cfg.clone(),
        model,
        kb,
        store,
        journal,
        tasks,
        started: std::time::Instant::now(),
    });

    let app = http::router(state);
    let bind_ip = if cfg.lan {
        log_line("WARNING: LAN binding enabled — agent endpoints exposed off-loopback");
        Ipv4Addr::UNSPECIFIED
    } else {
        Ipv4Addr::LOCALHOST
    };
    let listener = tokio::net::TcpListener::bind((bind_ip, cfg.port)).await?;
    log_line(&format!(
        "listening on http://{}:{} ({})",
        bind_ip,
        cfg.port,
        if cfg.lan { "lan" } else { "loopback" }
    ));
    axum::serve(listener, app).await?;
    Ok(())
}

/// One stderr line, RFC3339-stamped (journald does the rest).
pub(crate) fn log_line(msg: &str) {
    eprintln!("[{}] 111-agent: {msg}", events::now_rfc3339());
}
