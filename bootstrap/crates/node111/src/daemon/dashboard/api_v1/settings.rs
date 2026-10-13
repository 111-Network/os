//! `GET/PATCH /api/v1/settings` — `<root>/state/settings.json`
//! `{schema:1, alias, lan_enabled, offer}` (spec §6.7, §10). Owned by Layer 4
//! task R2.
//!
//! - `PATCH` is a partial merge of `SettingsPatch`; the whole patch is
//!   validated first and applied atomically (all or nothing). `schema` in a
//!   patch is ignored; any other unknown key is `400 bad_request` (typos must
//!   not silently succeed).
//! - `alias`: string ≤ 64 chars after trimming, no control characters; empty or
//!   `null` clears it.
//! - `lan_enabled` and `offer` are loopback-only, checked per field: a patch
//!   that carries either from a non-loopback peer is `403 loopback_only` and
//!   nothing is written.
//! - `offer: accepted|dismissed` is also written to `state/offer.json` in the
//!   exact format `111 config offer` writes, and mirrored into `/status.offer`,
//!   so Layer 1 stays consistent.
//! - Reads: `alias` from the file; `lan_enabled` is the *configured* value —
//!   env `DASHBOARD_LAN=true` (`<etc>/env`) OR the file's `lan_enabled` (the
//!   daemon reads the same pair at start via `bind::lan_configured`; the env
//!   file is root-owned, so while it says `true` a `false` here cannot turn LAN
//!   off and the response keeps answering `true`); `offer` from
//!   `state/offer.json` (Layer 1's record), else the file's last write.
//! - Every successful PATCH publishes a `status` frame (alias shows in status).

use super::{auth, status, ApiRouter, ApiState, Problem};
use crate::daemon::dashboard::bind::{self, SETTINGS_FILE};
use crate::daemon::dashboard::OfferState;
use axum::extract::State;
use axum::http::{Extensions, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Alias length cap (characters, after trimming).
pub const ALIAS_MAX: usize = 64;
pub const OFFER_FILE: &str = "offer.json";

pub fn router() -> ApiRouter {
    axum::Router::new().route("/api/v1/settings", get(get_settings).patch(patch_settings))
}

/// `state/settings.json` on disk (missing keys default; garbled file = defaults).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsFile {
    #[serde(default = "schema_one")]
    pub schema: u32,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub lan_enabled: bool,
    #[serde(default)]
    pub offer: Option<String>,
}

fn schema_one() -> u32 {
    1
}

impl Default for SettingsFile {
    fn default() -> Self {
        SettingsFile {
            schema: 1,
            alias: None,
            lan_enabled: false,
            offer: None,
        }
    }
}

/// The `Settings` response body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Settings {
    pub schema: u32,
    pub alias: Option<String>,
    pub lan_enabled: bool,
    pub offer: Option<String>,
}

/// Read `state/settings.json` (absent/garbled → defaults).
pub fn read_file(state_dir: &Path) -> SettingsFile {
    std::fs::read(state_dir.join(SETTINGS_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Configured LAN sharing (env file OR settings), as the next daemon start
/// will see it.
pub fn lan_configured(api: &ApiState) -> bool {
    let env = crate::daemon::load_env(&api.layout.etc).unwrap_or_default();
    bind::lan_configured(&env, &api.layout.state_dir())
}

fn valid_offer_state(s: &str) -> bool {
    matches!(s, "offered" | "accepted" | "dismissed")
}

/// `state/offer.json` `state` (Layer 1's offer record).
pub fn read_offer_state(state_dir: &Path) -> Option<String> {
    let v: Value = serde_json::from_slice(&std::fs::read(state_dir.join(OFFER_FILE)).ok()?).ok()?;
    v["state"]
        .as_str()
        .filter(|s| valid_offer_state(s))
        .map(str::to_owned)
}

/// Current settings (GET body and PATCH response).
pub fn current(api: &ApiState) -> Settings {
    let state_dir = api.layout.state_dir();
    let f = read_file(&state_dir);
    let offer = read_offer_state(&state_dir).or(f.offer.filter(|s| valid_offer_state(s)));
    Settings {
        schema: 1,
        alias: f.alias,
        lan_enabled: lan_configured(api),
        offer,
    }
}

async fn get_settings(State(api): State<Arc<ApiState>>) -> Json<Settings> {
    Json(current(&api))
}

/// A validated patch.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Patch {
    /// `Some(None)` clears the alias.
    pub alias: Option<Option<String>>,
    pub lan_enabled: Option<bool>,
    pub offer: Option<&'static str>,
}

impl Patch {
    /// Does this patch touch a loopback-only field?
    pub fn needs_loopback(&self) -> bool {
        self.lan_enabled.is_some() || self.offer.is_some()
    }
}

fn bad(detail: impl Into<String>) -> Problem {
    Problem::new(
        StatusCode::BAD_REQUEST,
        "bad_request",
        "invalid settings patch",
    )
    .detail(detail)
}

/// Validate a `SettingsPatch` body.
pub fn parse_patch(body: &[u8]) -> Result<Patch, Problem> {
    let v: Value = serde_json::from_slice(body).map_err(|_| bad("body must be a JSON object"))?;
    let obj = v
        .as_object()
        .ok_or_else(|| bad("body must be a JSON object"))?;
    let mut p = Patch::default();
    for (k, val) in obj {
        match k.as_str() {
            "alias" => {
                p.alias = Some(match val {
                    Value::Null => None,
                    Value::String(s) => {
                        let t = s.trim();
                        if t.chars().count() > ALIAS_MAX {
                            return Err(bad(format!(
                                "alias is longer than {ALIAS_MAX} characters"
                            )));
                        }
                        if t.chars().any(char::is_control) {
                            return Err(bad("alias contains control characters"));
                        }
                        (!t.is_empty()).then(|| t.to_string())
                    }
                    _ => return Err(bad("alias must be a string or null")),
                })
            }
            "lan_enabled" => {
                p.lan_enabled = Some(
                    val.as_bool()
                        .ok_or_else(|| bad("lan_enabled must be a boolean"))?,
                )
            }
            "offer" => {
                p.offer = Some(match val.as_str() {
                    Some("accepted") => "accepted",
                    Some("dismissed") => "dismissed",
                    _ => return Err(bad("offer must be \"accepted\" or \"dismissed\"")),
                })
            }
            "schema" => {}
            other => return Err(bad(format!("unknown setting {other:?}"))),
        }
    }
    Ok(p)
}

/// Serializes read-merge-write of the settings + offer files.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn rfc3339_now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}

/// `state/offer.json` exactly as `111 config offer accept|dismiss` writes it.
pub fn write_offer(state_dir: &Path, state: &str) -> anyhow::Result<()> {
    let body = serde_json::json!({
        "schema": 1,
        "tier": "standard",
        "state": state,
        "recorded_at": rfc3339_now(),
    });
    crate::util::fs::atomic_write(
        &state_dir.join(OFFER_FILE),
        body.to_string().as_bytes(),
        0o644,
    )
}

/// Apply a validated patch to disk (+ the in-memory offer mirror).
pub fn apply(api: &ApiState, p: &Patch) -> anyhow::Result<()> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let state_dir = api.layout.state_dir();
    std::fs::create_dir_all(&state_dir)?;
    let mut f = read_file(&state_dir);
    f.schema = 1;
    if let Some(alias) = &p.alias {
        f.alias = alias.clone();
    }
    if let Some(lan) = p.lan_enabled {
        f.lan_enabled = lan;
    }
    if let Some(offer) = p.offer {
        f.offer = Some(offer.to_string());
    }
    let json = serde_json::to_vec_pretty(&f)?;
    crate::util::fs::atomic_write(&state_dir.join(SETTINGS_FILE), &json, 0o644)?;
    if let Some(offer) = p.offer {
        write_offer(&state_dir, offer)?;
        api.dash.write().unwrap().offer = Some(OfferState {
            tier: "standard".into(),
            state: offer.into(),
        });
    }
    Ok(())
}

async fn patch_settings(
    State(api): State<Arc<ApiState>>,
    ext: Extensions,
    body: axum::body::Bytes,
) -> Response {
    let patch = match parse_patch(&body) {
        Ok(p) => p,
        Err(problem) => return problem.into_response(),
    };
    if patch.needs_loopback() && !auth::peer_is_loopback(&ext) {
        return auth::loopback_only().into_response();
    }
    if let Err(e) = apply(&api, &patch) {
        eprintln!("111 daemon: settings write failed: {e:#}");
        return Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "could not save settings",
        )
        .detail(format!("{e:#}"))
        .into_response();
    }
    status::publish_now(&api);
    Json(current(&api)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_validation() {
        assert_eq!(parse_patch(b"{}").unwrap(), Patch::default());
        let p = parse_patch(br#"{"alias":"  kitchen node  ","schema":1}"#).unwrap();
        assert_eq!(p.alias, Some(Some("kitchen node".into())));
        assert!(!p.needs_loopback());
        assert_eq!(
            parse_patch(br#"{"alias":"   "}"#).unwrap().alias,
            Some(None)
        );
        assert_eq!(parse_patch(br#"{"alias":null}"#).unwrap().alias, Some(None));
        let ok64 = format!(r#"{{"alias":"{}"}}"#, "é".repeat(64));
        assert!(parse_patch(ok64.as_bytes()).is_ok());
        let long = format!(r#"{{"alias":"{}"}}"#, "a".repeat(65));
        assert_eq!(
            parse_patch(long.as_bytes()).unwrap_err().code,
            "bad_request"
        );
        for bad_body in [
            &br#"{"alias":5}"#[..],
            br#"{"alias":"a\u0007b"}"#,
            br#"{"lan_enabled":"yes"}"#,
            br#"{"offer":"offered"}"#,
            br#"{"alais":"x"}"#,
            br#"[1]"#,
            b"not json",
        ] {
            assert!(
                parse_patch(bad_body).is_err(),
                "{}",
                String::from_utf8_lossy(bad_body)
            );
        }
        let p = parse_patch(br#"{"lan_enabled":true,"offer":"dismissed"}"#).unwrap();
        assert!(p.needs_loopback());
        assert_eq!(p.offer, Some("dismissed"));
    }

    #[test]
    fn settings_file_defaults_and_garbage() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_file(dir.path()), SettingsFile::default());
        std::fs::write(dir.path().join(SETTINGS_FILE), "garbage").unwrap();
        assert_eq!(read_file(dir.path()), SettingsFile::default());
        std::fs::write(dir.path().join(SETTINGS_FILE), r#"{"alias":"x"}"#).unwrap();
        assert_eq!(read_file(dir.path()).alias.as_deref(), Some("x"));
        assert_eq!(read_file(dir.path()).schema, 1);
    }
}
