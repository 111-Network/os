//! rnsd config: rendered into `<root>/rns/config`, regenerated on netlink
//! interface-change events. Carries a `schema-version` field Layer 6 bumps on
//! takeover. AutoInterface on. rns identity is independent of node identity in
//! v1 (spec §Transport).

use crate::daemon::DaemonLayout;
use std::path::PathBuf;

const TEMPLATE: &str = include_str!("../../../assets/rns/config.tmpl");

pub const SCHEMA_VERSION: u32 = 1;

pub fn config_dir(layout: &DaemonLayout) -> PathBuf {
    layout.root.join("rns")
}

pub fn config_path(layout: &DaemonLayout) -> PathBuf {
    config_dir(layout).join("config")
}

/// Render the current config (deterministic; interface data lands with Layer 6).
pub fn render() -> String {
    TEMPLATE.replace("@@SCHEMA_VERSION@@", &SCHEMA_VERSION.to_string())
}

/// Write the config atomically. Returns false when the file already had this
/// exact content (no rewrite → no needless rnsd restart).
pub fn write_if_changed(layout: &DaemonLayout) -> anyhow::Result<bool> {
    let dir = config_dir(layout);
    std::fs::create_dir_all(&dir)?;
    let content = render();
    let path = config_path(layout);
    if std::fs::read_to_string(&path)
        .map(|c| c == content)
        .unwrap_or(false)
    {
        return Ok(false);
    }
    crate::util::fs::atomic_write(&path, content.as_bytes(), 0o644)?;
    Ok(true)
}

/// Read the schema-version from an existing config (Layer 6 seam: a bumped
/// version means someone newer owns the file — stop regenerating).
pub fn existing_schema_version(layout: &DaemonLayout) -> Option<u32> {
    let text = std::fs::read_to_string(config_path(layout)).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("schema-version") {
            let rest = rest.trim_start_matches(['=', ' ']);
            if let Ok(v) = rest.parse::<u32>() {
                return Some(v);
            }
        }
    }
    None
}
