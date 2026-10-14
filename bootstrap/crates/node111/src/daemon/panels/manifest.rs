//! Panel declarations (Layer 5 §13 "Declaration"): `assets/panels/<id>.toml`,
//! embedded in the release binary (so hash-pinned with it), parsed with the
//! `toml` crate at build (the unit test below parses every embedded file) and
//! at daemon start.
//!
//! Rules:
//! - **Unknown keys or an invalid file ⇒ not registered (logged).** Every table
//!   is `deny_unknown_fields`; every value is validated before use; the raw
//!   text is size-checked before it is parsed.
//! - `id` must be a `ServiceId` and equal the file stem (`meshtastic.toml` ⇒
//!   `meshtastic`).
//! - v1 implements `[source] kind = "static"` only; `child` / `detected` are the
//!   reserved generic reverse proxy (P7) and are not registered.
//! - `[proxy] websocket = true` or `sse = true` ⇒ not registered in v1 (§13:
//!   "true here fails adoption in v1"). `max_body` is the request-body cap the
//!   listener applies before a shim sees a body.
//! - `[source] shim` must name a built-in shim in the [`ShimRegistry`];
//!   otherwise the panel is not registered.
//! - `bundle` is a relative path confined under `panels/` (resolved against the
//!   release payload by the panel host).

use super::shim::ShimRegistry;
use super::PANEL_PORTS;
use crate::daemon::services::ServiceId;
use serde::Deserialize;
use std::fmt;

/// Largest manifest accepted (checked before parsing).
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Largest `[proxy] max_body` accepted.
pub const MAX_BODY_CAP: usize = 64 * 1024 * 1024;
/// `[proxy] max_body` when the table or key is absent.
pub const DEFAULT_MAX_BODY: usize = 8 * 1024 * 1024;

/// The embedded declarations (`assets/panels/*.toml`). Debug builds read the
/// folder from disk; release builds embed it at compile time.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets/panels/"]
struct EmbeddedManifests;

/// Every embedded `*.toml` declaration as `(file name, bytes)`, sorted by name.
pub fn embedded() -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = EmbeddedManifests::iter()
        .filter(|name| name.ends_with(".toml") && !name.contains('/'))
        .filter_map(|name| {
            EmbeddedManifests::get(&name).map(|f| (name.to_string(), f.data.into_owned()))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

// ---------------------------------------------------------------- raw (serde)

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    id: String,
    title: String,
    #[serde(default)]
    kb_entry: Option<String>,
    source: RawSource,
    entry: RawEntry,
    #[serde(default)]
    health: Option<RawHealth>,
    frame: RawFrame,
    #[serde(default)]
    proxy: Option<RawProxy>,
    #[serde(default)]
    offline: Option<RawOffline>,
    auth: RawAuth,
    license: RawLicense,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSource {
    kind: String,
    #[serde(default)]
    bundle: Option<String>,
    #[serde(default)]
    shim: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHealth {
    #[serde(default)]
    listen: Option<i64>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    multi_client: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFrame {
    #[serde(default)]
    allow: Vec<String>,
    needs_secure_context: bool,
    #[serde(default)]
    sandbox: Option<String>,
    #[serde(default)]
    secure_note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProxy {
    #[serde(default)]
    websocket: bool,
    #[serde(default)]
    sse: bool,
    #[serde(default)]
    max_body: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOffline {
    #[serde(default)]
    optional_hosts: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAuth {
    mode: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLicense {
    spdx: String,
    served: String,
    source_offer: String,
}

// ---------------------------------------------------------------- validated

/// `[source] kind` (§13: v1 implements `static`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Static,
}

/// `[auth] mode` (§13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    /// The ticket → cookie flow; the listener binds where the dashboard binds.
    Session111,
    /// Same ticket flow, but the listener binds loopback regardless of LAN
    /// sharing.
    LoopbackOnly,
}

/// `[health]` (§13 "Health": a LISTEN check, never a connect; `url` only for
/// multi-client upstreams).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthSpec {
    /// Something LISTENs on local TCP `port` (read from `/proc/net/tcp*`).
    Listen(u16),
    /// A multi-client HTTP health URL. Accepted and validated; v1 does not
    /// probe it (the panel's health reads `unknown`).
    Url(String),
}

/// `[license]` (recorded; the source offer lives in `THIRD-PARTY-NOTICES.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct License {
    pub spdx: String,
    pub served: String,
    pub source_offer: String,
}

/// A validated declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub id: ServiceId,
    pub title: String,
    pub kb_entry: Option<String>,
    pub kind: SourceKind,
    /// Relative bundle directory under `panels/` (e.g. `panels/meshtastic-web`).
    pub bundle: String,
    /// Built-in shim name (`meshtastic-http`), dispatched for `/api/*` paths.
    pub shim: Option<String>,
    /// `[entry] path` (always starts with `/`, never under `/api`).
    pub entry: String,
    pub health: Option<HealthSpec>,
    pub allow: Vec<String>,
    pub needs_secure_context: bool,
    pub secure_note: Option<String>,
    /// `[proxy] max_body` in bytes (the request-body cap).
    pub max_body: usize,
    pub optional_hosts: Vec<String>,
    pub auth: AuthMode,
    pub license: License,
}

/// Why a declaration is not registered (logged by the panel host).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// Over [`MAX_MANIFEST_BYTES`] (checked before parsing).
    TooLarge(usize),
    /// Not UTF-8 / not TOML / unknown key / wrong type.
    Parse(String),
    /// A value failed validation.
    Invalid { field: &'static str, why: String },
    /// `id` does not equal the file stem.
    IdMismatch { id: String, file: String },
    /// A reserved feature v1 does not implement (child/detected sources,
    /// WebSocket/SSE proxying).
    Unsupported(&'static str),
    /// `[source] shim` names no built-in shim.
    UnknownShim(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::TooLarge(n) => {
                write!(f, "{n} bytes (at most {MAX_MANIFEST_BYTES})")
            }
            ManifestError::Parse(e) => write!(f, "not a valid declaration: {e}"),
            ManifestError::Invalid { field, why } => write!(f, "{field}: {why}"),
            ManifestError::IdMismatch { id, file } => {
                write!(f, "id {id:?} does not match the file name {file:?}")
            }
            ManifestError::Unsupported(what) => write!(f, "{what} is reserved (not built in v1)"),
            ManifestError::UnknownShim(name) => write!(f, "no built-in shim named {name:?}"),
        }
    }
}

impl std::error::Error for ManifestError {}

fn invalid(field: &'static str, why: impl Into<String>) -> ManifestError {
    ManifestError::Invalid {
        field,
        why: why.into(),
    }
}

fn plain_text(field: &'static str, s: &str, max: usize) -> Result<String, ManifestError> {
    let t = s.trim();
    if t.is_empty() {
        return Err(invalid(field, "empty"));
    }
    if t.chars().count() > max {
        return Err(invalid(field, format!("longer than {max} characters")));
    }
    if t.chars().any(|c| {
        c.is_control()
            || matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    }) {
        return Err(invalid(field, "control or bidi characters"));
    }
    Ok(t.to_string())
}

/// `projects/meshtastic-web`-style KB path: lowercase segments, no dot segments.
fn kb_path(s: &str) -> Result<String, ManifestError> {
    let ok = !s.is_empty()
        && s.len() <= 128
        && s.split('/').all(|seg| {
            !seg.is_empty()
                && !seg.starts_with('.')
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b))
        });
    if ok {
        Ok(s.to_string())
    } else {
        Err(invalid("kb_entry", "not a KB entry path"))
    }
}

/// Relative bundle dir confined under `panels/`: ASCII segments, no dot
/// segments, no empty segments.
fn bundle_path(s: &str) -> Result<String, ManifestError> {
    let ok = s.len() <= 128
        && s.starts_with("panels/")
        && s.split('/').all(|seg| {
            !seg.is_empty()
                && !seg.starts_with('.')
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        });
    if ok && s.split('/').count() >= 2 {
        Ok(s.to_string())
    } else {
        Err(invalid(
            "source.bundle",
            "must be a relative path under panels/ without dot segments",
        ))
    }
}

/// `[entry] path`: absolute path, plain characters, no dot segments, not under
/// `/api` (API paths never serve the SPA).
fn entry_path(s: &str) -> Result<String, ManifestError> {
    let ok = s.starts_with('/')
        && s.len() <= 256
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/-_.~".contains(&b))
        && s.split('/').all(|seg| seg != "." && seg != "..")
        && !s.contains("//");
    if !ok {
        return Err(invalid(
            "entry.path",
            "must be an absolute path of plain characters",
        ));
    }
    if super::static_files::is_api_path(s) {
        return Err(invalid("entry.path", "must not be under /api"));
    }
    Ok(s.to_string())
}

/// A DNS host name (lowercase labels of `[a-z0-9-]`, ≤ 253 bytes).
fn host_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// `8MiB` / `512KiB` / `4096B` → bytes (≤ [`MAX_BODY_CAP`], > 0).
pub fn parse_size(s: &str) -> Option<usize> {
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit())?;
    let (num, unit) = s.split_at(split);
    if num.is_empty() || num.len() > 12 {
        return None;
    }
    let n: u64 = num.parse().ok()?;
    let mult: u64 = match unit {
        "B" => 1,
        "KiB" => 1024,
        "MiB" => 1024 * 1024,
        _ => return None,
    };
    let bytes = n.checked_mul(mult)?;
    (bytes > 0 && bytes <= MAX_BODY_CAP as u64).then_some(bytes as usize)
}

/// Is `url` a loopback HTTP health URL (`http://127.0.0.1:<port>/path`)?
fn loopback_health_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://127.0.0.1:") else {
        return false;
    };
    let (port, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    port.parse::<u16>().is_ok_and(|p| p != 0)
        && path.len() <= 256
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/-_.~".contains(&b))
}

/// Parse + validate one declaration. `file` is its file name (`<id>.toml`);
/// `shims` the built-in shims a `[source] shim` may name.
pub fn parse(file: &str, bytes: &[u8], shims: &ShimRegistry) -> Result<Manifest, ManifestError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::TooLarge(bytes.len()));
    }
    let text = std::str::from_utf8(bytes).map_err(|e| ManifestError::Parse(e.to_string()))?;
    let raw: Raw = toml::from_str(text).map_err(|e| ManifestError::Parse(e.message().into()))?;

    let id = ServiceId::parse(&raw.id).ok_or_else(|| invalid("id", "not a service id"))?;
    let stem = file.strip_suffix(".toml").unwrap_or(file);
    if stem != raw.id {
        return Err(ManifestError::IdMismatch {
            id: raw.id,
            file: file.to_string(),
        });
    }
    let title = plain_text("title", &raw.title, 64)?;
    let kb_entry = raw.kb_entry.as_deref().map(kb_path).transpose()?;

    let kind = match raw.source.kind.as_str() {
        "static" => SourceKind::Static,
        "child" | "detected" => {
            return Err(ManifestError::Unsupported(
                "a child/detected source (generic reverse proxy)",
            ))
        }
        other => return Err(invalid("source.kind", format!("unknown kind {other:?}"))),
    };
    let bundle = bundle_path(
        raw.source
            .bundle
            .as_deref()
            .ok_or_else(|| invalid("source.bundle", "a static panel needs a bundle"))?,
    )?;
    let shim = match raw.source.shim {
        None => None,
        Some(name) => {
            if shims.get(&name).is_none() {
                return Err(ManifestError::UnknownShim(name));
            }
            Some(name)
        }
    };
    let entry = entry_path(&raw.entry.path)?;

    let health = match raw.health {
        None => None,
        Some(RawHealth {
            listen: None,
            url: None,
            multi_client: None,
        }) => None,
        Some(RawHealth {
            listen: Some(port),
            url: None,
            multi_client: None,
        }) => {
            let port = u16::try_from(port)
                .ok()
                .filter(|p| *p != 0)
                .ok_or_else(|| invalid("health.listen", "not a TCP port"))?;
            if PANEL_PORTS.contains(&port) || port == crate::daemon::dashboard::DEFAULT_PORT {
                return Err(invalid("health.listen", "a dashboard or panel port"));
            }
            Some(HealthSpec::Listen(port))
        }
        Some(RawHealth {
            listen: None,
            url: Some(url),
            multi_client: Some(true),
        }) if loopback_health_url(&url) => Some(HealthSpec::Url(url)),
        Some(RawHealth { url: Some(_), .. }) => {
            return Err(invalid(
                "health.url",
                "only a loopback http URL, only with multi_client = true, never with listen",
            ))
        }
        Some(_) => {
            return Err(invalid(
                "health",
                "listen or url (with multi_client = true), not both",
            ))
        }
    };

    if raw.frame.allow.len() > 16 {
        return Err(invalid("frame.allow", "more than 16 features"));
    }
    let mut allow: Vec<String> = Vec::with_capacity(raw.frame.allow.len());
    for feature in raw.frame.allow {
        let ok = !feature.is_empty()
            && feature.len() <= 32
            && feature.as_bytes()[0].is_ascii_lowercase()
            && feature
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !ok || allow.contains(&feature) {
            return Err(invalid("frame.allow", format!("bad feature {feature:?}")));
        }
        allow.push(feature);
    }
    match raw.frame.sandbox.as_deref() {
        None | Some("default") => {}
        Some(other) => {
            return Err(invalid(
                "frame.sandbox",
                format!("unknown sandbox {other:?} (v1 knows \"default\")"),
            ))
        }
    }
    let secure_note = raw
        .frame
        .secure_note
        .as_deref()
        .map(|n| plain_text("frame.secure_note", n, 512))
        .transpose()?;

    let (websocket, sse, max_body) = match raw.proxy {
        None => (false, false, DEFAULT_MAX_BODY),
        Some(p) => (
            p.websocket,
            p.sse,
            match p.max_body {
                None => DEFAULT_MAX_BODY,
                Some(s) => parse_size(&s).ok_or_else(|| {
                    invalid("proxy.max_body", format!("not a size up to 64MiB: {s:?}"))
                })?,
            },
        ),
    };
    if websocket || sse {
        return Err(ManifestError::Unsupported("WebSocket/SSE proxying"));
    }

    let hosts = raw.offline.map(|o| o.optional_hosts).unwrap_or_default();
    if hosts.len() > 32 {
        return Err(invalid("offline.optional_hosts", "more than 32 hosts"));
    }
    let mut optional_hosts: Vec<String> = Vec::with_capacity(hosts.len());
    for h in hosts {
        if !host_name(&h) || optional_hosts.contains(&h) {
            return Err(invalid(
                "offline.optional_hosts",
                format!("bad host name {h:?}"),
            ));
        }
        optional_hosts.push(h);
    }

    let auth = match raw.auth.mode.as_str() {
        "111-session" => AuthMode::Session111,
        "loopback-only" => AuthMode::LoopbackOnly,
        other => return Err(invalid("auth.mode", format!("unknown mode {other:?}"))),
    };

    let spdx = plain_text("license.spdx", &raw.license.spdx, 64)?;
    if !spdx
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-.+ ()".contains(&b))
    {
        return Err(invalid("license.spdx", "not an SPDX expression"));
    }
    if raw.license.served != "unmodified" {
        return Err(invalid(
            "license.served",
            "v1 serves bundles unmodified only (\"unmodified\")",
        ));
    }
    let source_offer = plain_text("license.source_offer", &raw.license.source_offer, 128)?;

    Ok(Manifest {
        id,
        title,
        kb_entry,
        kind,
        bundle,
        shim,
        entry,
        health,
        allow,
        needs_secure_context: raw.frame.needs_secure_context,
        secure_note,
        max_body,
        optional_hosts,
        auth,
        license: License {
            spdx,
            served: raw.license.served,
            source_offer,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::panels::shim::{Shim, ShimError, ShimRequest, ShimResponse, ShimSession};
    use futures_util::future::BoxFuture;
    use std::sync::Arc;

    struct Named;
    impl Shim for Named {
        fn name(&self) -> &'static str {
            "meshtastic-http"
        }
        fn handle<'a>(
            &'a self,
            _s: &'a ShimSession,
            _r: ShimRequest,
        ) -> BoxFuture<'a, Result<ShimResponse, ShimError>> {
            Box::pin(async { Err(ShimError::NotFound) })
        }
    }

    fn shims() -> ShimRegistry {
        let mut r = ShimRegistry::new();
        r.register(Arc::new(Named)).unwrap();
        r
    }

    fn shipped() -> String {
        let all = embedded();
        let (_, bytes) = all
            .iter()
            .find(|(n, _)| n == "meshtastic.toml")
            .expect("meshtastic.toml embedded");
        String::from_utf8(bytes.clone()).unwrap()
    }

    /// The "parsed at build" gate: every embedded declaration is valid with
    /// the shims it names registered.
    #[test]
    fn every_embedded_declaration_parses() {
        let all = embedded();
        assert!(!all.is_empty());
        for (name, bytes) in &all {
            let m = parse(name, bytes, &shims()).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(format!("{}.toml", m.id.as_str()), *name);
        }
        let m = parse("meshtastic.toml", shipped().as_bytes(), &shims()).unwrap();
        assert_eq!(m.id, ServiceId::Meshtastic);
        assert_eq!(m.bundle, "panels/meshtastic-web");
        assert_eq!(m.shim.as_deref(), Some("meshtastic-http"));
        assert_eq!(m.entry, "/");
        assert_eq!(m.health, Some(HealthSpec::Listen(11230)));
        assert_eq!(m.allow, ["serial", "bluetooth"]);
        assert!(!m.needs_secure_context);
        assert_eq!(m.max_body, 8 * 1024 * 1024);
        assert_eq!(
            m.optional_hosts,
            ["tiles.openfreemap.org", "api.meshtastic.org"]
        );
        assert_eq!(m.auth, AuthMode::Session111);
        assert_eq!(m.license.spdx, "GPL-3.0-only");
    }

    #[test]
    fn unregistered_shim_fails_registration() {
        let e = parse(
            "meshtastic.toml",
            shipped().as_bytes(),
            &ShimRegistry::new(),
        )
        .unwrap_err();
        assert_eq!(e, ManifestError::UnknownShim("meshtastic-http".into()));
    }

    fn with(edit: impl Fn(String) -> String) -> Result<Manifest, ManifestError> {
        parse("meshtastic.toml", edit(shipped()).as_bytes(), &shims())
    }

    #[test]
    fn unknown_keys_and_reserved_features_are_refused() {
        // unknown keys at the top and in a table
        assert!(matches!(
            with(|t| format!("extra = 1\n{t}")),
            Err(ManifestError::Parse(_))
        ));
        assert!(matches!(
            with(|t| t.replace(
                "[entry]\npath = \"/\"",
                "[entry]\npath = \"/\"\nquery = \"x\""
            )),
            Err(ManifestError::Parse(_))
        ));
        // websocket / sse proxying is reserved
        for key in ["websocket", "sse"] {
            let e = with(|t| t.replace(&format!("{key} = false"), &format!("{key} = true")))
                .unwrap_err();
            assert!(matches!(e, ManifestError::Unsupported(_)), "{key}: {e}");
        }
        // child / detected sources are the reserved proxy
        let e = with(|t| t.replace("kind = \"static\"", "kind = \"child\"")).unwrap_err();
        assert!(matches!(e, ManifestError::Unsupported(_)), "{e}");
        let e = with(|t| t.replace("kind = \"static\"", "kind = \"remote\"")).unwrap_err();
        assert!(matches!(e, ManifestError::Invalid { .. }), "{e}");
    }

    #[test]
    fn values_are_validated() {
        let cases: [(&str, &str); 14] = [
            ("id = \"meshtastic\"", "id = \"nope\""),
            ("id = \"meshtastic\"", "id = \"fabric\""),
            ("panels/meshtastic-web", "../etc"),
            ("panels/meshtastic-web", "panels/../x"),
            ("panels/meshtastic-web", "/abs/panels"),
            ("[entry]\npath = \"/\"", "[entry]\npath = \"/api/v1/x\""),
            ("[entry]\npath = \"/\"", "[entry]\npath = \"relative\""),
            ("[entry]\npath = \"/\"", "[entry]\npath = \"/a/../b\""),
            ("listen = 11230", "listen = 11190"),
            ("listen = 11230", "listen = 70000"),
            ("max_body = \"8MiB\"", "max_body = \"1GiB\""),
            ("mode = \"111-session\"", "mode = \"none\""),
            ("served = \"unmodified\"", "served = \"patched\""),
            ("\"tiles.openfreemap.org\"", "\"Tiles.Example\""),
        ];
        for (from, to) in cases {
            let r = with(|t| {
                assert!(t.contains(from), "fixture lacks {from}");
                t.replacen(from, to, 1)
            });
            assert!(r.is_err(), "{from} -> {to} accepted");
        }
        // a duplicate allow feature, a control char in the title
        assert!(
            with(|t| t.replace("[\"serial\", \"bluetooth\"]", "[\"serial\", \"serial\"]")).is_err()
        );
        assert!(with(|t| t.replace("title = \"Meshtastic\"", "title = \"Mesh\\u0007\"")).is_err());
        // loopback-only is a valid mode
        let m = with(|t| t.replace("mode = \"111-session\"", "mode = \"loopback-only\"")).unwrap();
        assert_eq!(m.auth, AuthMode::LoopbackOnly);
        // health url only with multi_client = true, loopback only
        let m = with(|t| {
            t.replace(
                "listen = 11230",
                "url = \"http://127.0.0.1:8080/health\"\nmulti_client = true",
            )
        })
        .unwrap();
        assert_eq!(
            m.health,
            Some(HealthSpec::Url("http://127.0.0.1:8080/health".into()))
        );
        assert!(with(|t| t.replace("listen = 11230", "url = \"http://127.0.0.1:8080/\"")).is_err());
        assert!(with(|t| t.replace(
            "listen = 11230",
            "url = \"http://example.com:80/\"\nmulti_client = true"
        ))
        .is_err());
        // file name must match
        assert!(matches!(
            parse("other.toml", shipped().as_bytes(), &shims()),
            Err(ManifestError::IdMismatch { .. })
        ));
        // size checked before parsing
        let big = vec![b'#'; MAX_MANIFEST_BYTES + 1];
        assert!(matches!(
            parse("meshtastic.toml", &big, &shims()),
            Err(ManifestError::TooLarge(_))
        ));
        assert!(matches!(
            parse("meshtastic.toml", b"\xff\xfe", &shims()),
            Err(ManifestError::Parse(_))
        ));
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("8MiB"), Some(8 << 20));
        assert_eq!(parse_size("512KiB"), Some(512 << 10));
        assert_eq!(parse_size("4096B"), Some(4096));
        assert_eq!(parse_size("64MiB"), Some(64 << 20));
        for bad in [
            "",
            "MiB",
            "0B",
            "65MiB",
            "8mib",
            "8 MiB",
            "-1B",
            "99999999999999B",
        ] {
            assert_eq!(parse_size(bad), None, "{bad}");
        }
    }
}
