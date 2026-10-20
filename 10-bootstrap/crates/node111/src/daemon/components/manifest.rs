//! Component declarations (Layer 6 §2.1 rule 1): `assets/components/<id>.toml`,
//! embedded in the release binary (so hash-pinned with it, like
//! `assets/panels/<id>.toml`), parsed at daemon start and by `111 install`
//! (the same binary: the installer reads the embedded declarations, so the
//! cache intake needs nothing staged).
//!
//! Rules:
//! - **Unknown keys or an invalid file ⇒ not registered (logged).** Every
//!   table is `deny_unknown_fields`; every value is validated before use; the
//!   raw text is size-checked before it is parsed.
//! - `id` is a `ComponentId` (`^[a-z0-9][a-z0-9_-]{0,39}$`, the contract
//!   pattern), equals the file stem and is not a name the base tree already
//!   uses under `cache/packages/` or `venvs/` (`python`, `rns`, `presence`).
//! - `[payload] kind = "pip"`: `lock` names a hash-locked requirements file
//!   that rides **beside the declaration** in `assets/components/` (a bare
//!   file name ending in `.txt`; hashes only, no third-party bytes), so it is
//!   pinned by the binary and present at install step 5 before any payload is
//!   staged; `venv` equals `id` (`venvs/<id>`, a link into `venvs/<id>.d/`).
//!   A lock the release does not carry leaves the component declared with no
//!   payload (enable answers `component_unavailable`), like a release without
//!   the presence reader. `kind = "binary"`: `url` (https), `sha256` (64
//!   lowercase hex) and `size` pin one file, cached under
//!   `cache/packages/<id>/<file name of the URL>`.
//! - `[run] program`: for `pip`, a path relative to the release tree
//!   (`share/<dir>/<script>.py`, run as `venvs/<id>/bin/python -I <current>/<program>`);
//!   for `binary`, a path relative to `components/<id>/`. `args` may use the
//!   placeholders `{root}`, `{bridge}` (`bridge/<adapter dir>/`), `{install}`
//!   (`components/<id>/`) and `{release}` (`current/`), each as a whole
//!   argument or inside one; any other `{…}` is invalid.
//! - `[run] health`: `ready` (the first stdout line must be a JSON object with
//!   `"type": "ready"`, within `health_timeout_secs`, default 20) or `none`.
//! - `[license] notice` is the exact text the user acknowledges; its hash
//!   ([`Component::notice_hash`]) is what `POST …/enable` must echo.

use crate::daemon::services::ServiceId;
use serde::Deserialize;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

/// Largest declaration accepted (checked before parsing).
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Largest lock file accepted.
pub const MAX_LOCK_BYTES: usize = 1024 * 1024;
/// Largest binary payload a declaration may pin.
pub const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;
/// Health timeout when `[run] health_timeout_secs` is absent.
pub const DEFAULT_HEALTH_TIMEOUT: Duration = Duration::from_secs(20);
/// Names the base tree already uses under `cache/packages/` and `venvs/`.
const RESERVED_IDS: &[&str] = &["python", "rns", "presence"];

/// The embedded folder (`assets/components/`: declarations and their locks).
/// Debug builds read the folder from disk; release builds embed it.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets/components/"]
struct EmbeddedFiles;

/// Every embedded file (declarations `*.toml` and locks `*.txt`) as
/// `(file name, bytes)`, sorted by name.
pub fn embedded() -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = EmbeddedFiles::iter()
        .filter(|name| !name.contains('/'))
        .filter_map(|name| {
            EmbeddedFiles::get(&name).map(|f| (name.to_string(), f.data.into_owned()))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The embedded declarations, invalid ones logged and left out.
pub fn load_embedded() -> Vec<Component> {
    load(&embedded())
}

/// Parse every `*.toml` in `files` (locks are looked up among the same
/// files); an invalid declaration or a duplicate id is logged and not
/// registered. Sorted by id.
pub fn load(files: &[(String, Vec<u8>)]) -> Vec<Component> {
    let mut out: Vec<Component> = Vec::new();
    for (name, bytes) in files.iter().filter(|(n, _)| n.ends_with(".toml")) {
        let lock_of = |lock: &str| {
            files
                .iter()
                .find(|(n, _)| n == lock)
                .map(|(_, b)| b.as_slice())
        };
        match parse(name, bytes, lock_of) {
            Ok(c) if out.iter().any(|o| o.id == c.id) => {
                eprintln!("111 daemon: component {name}: not registered: duplicate id");
            }
            Ok(c) => out.push(c),
            Err(e) => eprintln!("111 daemon: component {name}: not registered: {e}"),
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

// ---------------------------------------------------------------- raw (serde)

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    id: String,
    adapter: String,
    title: String,
    #[serde(default)]
    kb_entry: Option<String>,
    payload: RawPayload,
    #[serde(default)]
    requires: RawRequires,
    run: RawRun,
    #[serde(default)]
    weight: RawWeight,
    license: RawLicense,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPayload {
    kind: String,
    #[serde(default)]
    lock: Option<String>,
    #[serde(default)]
    venv: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequires {
    #[serde(default)]
    python: Option<String>,
    #[serde(default)]
    service: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRun {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    ports: Vec<i64>,
    health: String,
    #[serde(default)]
    health_timeout_secs: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWeight {
    #[serde(default)]
    cached_bytes: u64,
    #[serde(default)]
    installed_bytes: u64,
    #[serde(default)]
    rss_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLicense {
    spdx: String,
    read_at: String,
    notice: String,
}

// ---------------------------------------------------------------- validated

/// One registered optional component (§2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub id: String,
    /// The network this component carries (`NetworkId`: `lxmf`, `app:deltachat`).
    pub adapter: String,
    pub title: String,
    pub kb_entry: Option<String>,
    pub payload: Payload,
    pub requires: Requires,
    pub run: Run,
    pub weight: Weight,
    pub license: License,
}

/// The payload pin (§2.1 rule 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// A hash-locked pip closure installed into `venvs/<venv>`.
    Pip {
        /// The lock's file name beside the declaration.
        lock: String,
        /// The lock's bytes; `None` when the release does not carry it.
        lock_bytes: Option<Arc<[u8]>>,
        venv: String,
    },
    /// One pinned file (a binary or a `.tar.gz` of one).
    Binary {
        url: String,
        sha256: String,
        size: u64,
    },
}

/// `[requires]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Requires {
    /// Lowest `python3` (major, minor).
    pub python: Option<(u32, u32)>,
    /// A supervised service that must serve before the child starts (§6
    /// "Start order"; `transport` = rnsd's shared instance).
    pub service: Option<ServiceId>,
}

/// How the child is health-checked after it starts (§2.1 rule 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// The first stdout line is a JSON object with `"type": "ready"`.
    Ready { timeout: Duration },
    /// Healthy once it runs.
    None,
}

/// `[run]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub program: String,
    pub args: Vec<String>,
    /// Loopback ports the child listens on (claimed in the `PortTable` and
    /// pre-bound before it starts, §2).
    pub ports: Vec<u16>,
    pub health: Health,
}

/// `[weight]` (0 = not measured; shown before enable).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Weight {
    pub cached_bytes: u64,
    pub installed_bytes: u64,
    pub rss_bytes: u64,
}

/// `[license]` (§15: the user installs; 111 is not the distributor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct License {
    pub spdx: String,
    pub read_at: String,
    pub notice: String,
}

impl Component {
    /// Lowercase hex SHA-256 of the exact notice text (the contract's
    /// `Sha256Hex`: what the user acknowledges and `enable` must echo).
    pub fn notice_hash(&self) -> String {
        crate::util::hash::sha256_bytes(self.license.notice.as_bytes())
    }

    /// The `bridge/<dir>/` name of this component's adapter: the part after
    /// the last `:` (`app:deltachat` ⇒ `deltachat`, §2 "State").
    pub fn adapter_dir(&self) -> &str {
        self.adapter.rsplit(':').next().unwrap_or(&self.adapter)
    }
}

/// Why a declaration was not registered.
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
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::TooLarge(n) => write!(f, "{n} bytes (at most {MAX_MANIFEST_BYTES})"),
            ManifestError::Parse(e) => write!(f, "not a valid declaration: {e}"),
            ManifestError::Invalid { field, why } => write!(f, "{field}: {why}"),
            ManifestError::IdMismatch { id, file } => {
                write!(f, "id {id:?} does not match the file name {file:?}")
            }
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

/// `ComponentId` (contract pattern `^[a-z0-9][a-z0-9_-]{0,39}$`).
pub fn is_component_id(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 40
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

/// `NetworkId` (contract pattern `^[a-z0-9][a-z0-9:_-]{0,39}$`) whose last
/// `:` segment is a non-empty directory name.
fn is_network_id(s: &str) -> bool {
    let b = s.as_bytes();
    let ok = !b.is_empty()
        && b.len() <= 40
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(*c, b':' | b'_' | b'-')
        });
    ok && s
        .rsplit(':')
        .next()
        .is_some_and(|d| !d.is_empty() && !d.starts_with('-') && !d.starts_with('_'))
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
            || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
    }) {
        return Err(invalid(field, "control, format or bidi characters"));
    }
    Ok(t.to_string())
}

/// `projects/lxmf`-style KB path: lowercase segments, no dot segments.
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

/// A relative path of ASCII segments (`[A-Za-z0-9._-]`, none empty, none
/// starting with `.`), at most 256 bytes.
fn rel_path(field: &'static str, s: &str) -> Result<String, ManifestError> {
    let ok = !s.is_empty()
        && s.len() <= 256
        && s.split('/').all(|seg| {
            !seg.is_empty()
                && !seg.starts_with('.')
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        });
    if ok {
        Ok(s.to_string())
    } else {
        Err(invalid(
            field,
            "a relative path of plain segments (no `..`, no leading `/`)",
        ))
    }
}

/// An `https://` URL without whitespace, quotes or control characters.
fn https_url(field: &'static str, s: &str) -> Result<String, ManifestError> {
    let ok = s.len() <= 512
        && s.strip_prefix("https://")
            .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/') && rest.contains('/'))
        && s.bytes()
            .all(|b| b.is_ascii_graphic() && !b"\"'<>\\`{}|^".contains(&b));
    if ok {
        Ok(s.to_string())
    } else {
        Err(invalid(field, "an https URL"))
    }
}

/// An SPDX-shaped expression: identifiers, `LicenseRef-…`, `AND`/`OR`/`WITH`
/// and parentheses.
fn spdx(s: &str) -> Result<String, ManifestError> {
    let ok = !s.trim().is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b" .+-()".contains(&b));
    if ok {
        Ok(s.trim().to_string())
    } else {
        Err(invalid("license.spdx", "not an SPDX expression"))
    }
}

/// Where to read the licence: an https URL or a release-relative path.
fn read_at(s: &str) -> Result<String, ManifestError> {
    if s.starts_with("https://") {
        https_url("license.read_at", s)
    } else {
        rel_path("license.read_at", s)
    }
}

/// `MAJOR.MINOR`.
fn python_version(s: &str) -> Result<(u32, u32), ManifestError> {
    let bad = || invalid("requires.python", "MAJOR.MINOR, e.g. 3.7");
    let (ma, mi) = s.trim().split_once('.').ok_or_else(bad)?;
    let num = |p: &str| {
        (!p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse::<u32>().ok())
            .flatten()
    };
    match (num(ma), num(mi)) {
        (Some(a), Some(b)) if a >= 3 => Ok((a, b)),
        _ => Err(bad()),
    }
}

/// The known `[run] args` placeholders.
pub const PLACEHOLDERS: &[&str] = &["{root}", "{bridge}", "{install}", "{release}"];

fn run_arg(s: &str) -> Result<String, ManifestError> {
    if s.len() > 256 || s.chars().any(char::is_control) {
        return Err(invalid(
            "run.args",
            "an argument is too long or holds control characters",
        ));
    }
    let rest = PLACEHOLDERS
        .iter()
        .fold(s.to_string(), |acc, p| acc.replace(p, ""));
    if rest.contains(['{', '}']) {
        return Err(invalid("run.args", format!("unknown placeholder in {s:?}")));
    }
    Ok(s.to_string())
}

/// The last path segment of a pinned URL as a cache file name.
pub fn url_file_name(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let name = path.rsplit('/').next()?;
    let ok = !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b));
    ok.then(|| name.to_string())
}

/// Parse + validate one declaration. `file` is its file name (`<id>.toml`);
/// `lock_of` finds a lock beside it by file name.
pub fn parse<'a>(
    file: &str,
    bytes: &[u8],
    lock_of: impl Fn(&str) -> Option<&'a [u8]>,
) -> Result<Component, ManifestError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::TooLarge(bytes.len()));
    }
    let text = std::str::from_utf8(bytes).map_err(|e| ManifestError::Parse(e.to_string()))?;
    let raw: Raw = toml::from_str(text).map_err(|e| ManifestError::Parse(e.message().into()))?;

    if !is_component_id(&raw.id) {
        return Err(invalid("id", "not a component id"));
    }
    if RESERVED_IDS.contains(&raw.id.as_str()) {
        return Err(invalid("id", "a name the base tree already uses"));
    }
    let stem = file.strip_suffix(".toml").unwrap_or(file);
    if stem != raw.id {
        return Err(ManifestError::IdMismatch {
            id: raw.id,
            file: file.to_string(),
        });
    }
    if !is_network_id(&raw.adapter) {
        return Err(invalid("adapter", "not a network id"));
    }
    let title = plain_text("title", &raw.title, 64)?;
    let kb_entry = raw.kb_entry.as_deref().map(kb_path).transpose()?;

    let p = raw.payload;
    let payload = match p.kind.as_str() {
        "pip" => {
            if p.url.is_some() || p.sha256.is_some() || p.size.is_some() {
                return Err(invalid("payload", "a pip payload has no url/sha256/size"));
            }
            let lock = p
                .lock
                .ok_or_else(|| invalid("payload.lock", "a pip payload names its lock"))?;
            let lock_ok = lock.len() <= 64
                && lock.ends_with(".txt")
                && !lock.starts_with('.')
                && lock
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
            if !lock_ok {
                return Err(invalid(
                    "payload.lock",
                    "the file name of a .txt lock beside the declaration",
                ));
            }
            let venv = p
                .venv
                .ok_or_else(|| invalid("payload.venv", "a pip payload names its venv"))?;
            if venv != raw.id {
                return Err(invalid("payload.venv", "equals the component id"));
            }
            let lock_bytes = match lock_of(&lock) {
                Some(b) if b.len() > MAX_LOCK_BYTES => {
                    return Err(invalid("payload.lock", "the lock is larger than 1 MiB"))
                }
                Some(b) if lock_hashes(b).is_empty() => {
                    return Err(invalid("payload.lock", "the lock pins no --hash=sha256"))
                }
                Some(b) => Some(Arc::<[u8]>::from(b)),
                None => None,
            };
            Payload::Pip {
                lock,
                lock_bytes,
                venv,
            }
        }
        "binary" => {
            if p.lock.is_some() || p.venv.is_some() {
                return Err(invalid("payload", "a binary payload has no lock/venv"));
            }
            let url = https_url(
                "payload.url",
                p.url
                    .as_deref()
                    .ok_or_else(|| invalid("payload.url", "a binary payload pins its url"))?,
            )?;
            if url_file_name(&url).is_none() {
                return Err(invalid("payload.url", "ends in a plain file name"));
            }
            let sha256 = p
                .sha256
                .filter(|h| {
                    h.len() == 64
                        && h.bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
                .ok_or_else(|| invalid("payload.sha256", "64 lowercase hex"))?;
            let size = p
                .size
                .filter(|s| (1..=MAX_BINARY_BYTES).contains(s))
                .ok_or_else(|| invalid("payload.size", "1 byte to 512 MiB"))?;
            Payload::Binary { url, sha256, size }
        }
        other => return Err(invalid("payload.kind", format!("unknown kind {other:?}"))),
    };

    let requires = Requires {
        python: raw
            .requires
            .python
            .as_deref()
            .map(python_version)
            .transpose()?,
        service: match raw.requires.service.as_deref() {
            None => None,
            Some(s) => Some(
                ServiceId::parse(s)
                    .ok_or_else(|| invalid("requires.service", "not a service id"))?,
            ),
        },
    };
    if matches!(payload, Payload::Pip { .. }) && requires.python.is_none() {
        return Err(invalid("requires.python", "a pip payload names its python"));
    }

    let r = raw.run;
    let program = rel_path("run.program", &r.program)?;
    if matches!(payload, Payload::Pip { .. }) && !program.ends_with(".py") {
        return Err(invalid("run.program", "a pip component runs a .py script"));
    }
    if r.args.len() > 32 {
        return Err(invalid("run.args", "more than 32 arguments"));
    }
    let args = r
        .args
        .iter()
        .map(|a| run_arg(a))
        .collect::<Result<Vec<_>, _>>()?;
    if r.ports.len() > 8 {
        return Err(invalid("run.ports", "more than 8 ports"));
    }
    let mut ports: Vec<u16> = Vec::with_capacity(r.ports.len());
    for p in r.ports {
        let port = u16::try_from(p)
            .ok()
            .filter(|p| *p != 0)
            .ok_or_else(|| invalid("run.ports", "not a TCP port"))?;
        if base_port(port) {
            return Err(invalid("run.ports", format!("{port} is a base 111 port")));
        }
        if ports.contains(&port) {
            return Err(invalid("run.ports", format!("{port} listed twice")));
        }
        ports.push(port);
    }
    let timeout = match r.health_timeout_secs {
        None => DEFAULT_HEALTH_TIMEOUT,
        Some(s) if (1..=300).contains(&s) => Duration::from_secs(s),
        Some(_) => return Err(invalid("run.health_timeout_secs", "1 to 300")),
    };
    let health = match r.health.as_str() {
        "ready" => Health::Ready { timeout },
        "none" => {
            if r.health_timeout_secs.is_some() {
                return Err(invalid(
                    "run.health_timeout_secs",
                    "only with health = \"ready\"",
                ));
            }
            Health::None
        }
        other => {
            return Err(invalid(
                "run.health",
                format!("ready or none, not {other:?}"),
            ))
        }
    };

    let w = raw.weight;
    let license = License {
        spdx: spdx(&raw.license.spdx)?,
        read_at: read_at(&raw.license.read_at)?,
        notice: plain_text("license.notice", &raw.license.notice, 1000)?,
    };

    Ok(Component {
        id: raw.id,
        adapter: raw.adapter,
        title,
        kb_entry,
        payload,
        requires,
        run: Run {
            program,
            args,
            ports,
            health,
        },
        weight: Weight {
            cached_bytes: w.cached_bytes,
            installed_bytes: w.installed_bytes,
            rss_bytes: w.rss_bytes,
        },
        license,
    })
}

/// Ports the base tree owns (L5 §2 and §2 here): dashboard, model, agent,
/// fabric, panels, the meshmud downlink and the reserved SimpleX socket.
fn base_port(p: u16) -> bool {
    matches!(p, 11180..=11183 | 11190..=11230)
}

/// Every `--hash=sha256:<hex>` a lock pins (lowercase), in order of
/// appearance, deduplicated.
pub fn lock_hashes(lock: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(lock);
    let mut out: Vec<String> = Vec::new();
    for token in text.split_whitespace() {
        let Some(hex) = token.strip_prefix("--hash=sha256:") else {
            continue;
        };
        let hex = hex.to_ascii_lowercase();
        if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) && !out.contains(&hex) {
            out.push(hex);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = "rns==1.5.7 \\\n    --hash=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n";

    fn pip_toml(extra: &str) -> String {
        format!(
            r#"id = "demo"
adapter = "lxmf"
title = "Demo helper"
[payload]
kind = "pip"
lock = "demo.lock.txt"
venv = "demo"
[requires]
python = "3.7"
service = "transport"
[run]
program = "share/demo/demo.py"
args = ["--dir", "{{bridge}}/rns", "--root={{root}}"]
health = "ready"
[license]
spdx = "LicenseRef-Reticulum"
read_at = "https://example.org/LICENSE"
notice = "This installs demo. You install it."
{extra}"#
        )
    }

    fn parse_str(file: &str, text: &str) -> Result<Component, ManifestError> {
        parse(file, text.as_bytes(), |n| {
            (n == "demo.lock.txt").then_some(LOCK.as_bytes())
        })
    }

    #[test]
    fn every_embedded_declaration_parses() {
        let files = embedded();
        let tomls: Vec<_> = files.iter().filter(|(n, _)| n.ends_with(".toml")).collect();
        assert!(!tomls.is_empty(), "assets/components/ carries declarations");
        let loaded = load(&files);
        assert_eq!(
            loaded.len(),
            tomls.len(),
            "every embedded declaration registers"
        );
        let lxmf = loaded
            .iter()
            .find(|c| c.id == "lxmf")
            .expect("lxmf declared");
        assert_eq!(lxmf.adapter, "lxmf");
        assert_eq!(lxmf.requires.service, Some(ServiceId::Transport));
        assert!(matches!(lxmf.run.health, Health::Ready { .. }));
        assert!(
            lxmf.run.ports.is_empty(),
            "the helper listens on no port (§15.7)"
        );
        assert_eq!(lxmf.notice_hash().len(), 64);
    }

    #[test]
    fn a_pip_declaration_parses_with_its_lock() {
        let c = parse_str("demo.toml", &pip_toml("")).unwrap();
        assert_eq!(c.adapter_dir(), "lxmf");
        let Payload::Pip {
            lock_bytes, venv, ..
        } = &c.payload
        else {
            panic!("pip")
        };
        assert_eq!(venv, "demo");
        assert_eq!(lock_bytes.as_deref(), Some(LOCK.as_bytes()));
        assert_eq!(c.requires.python, Some((3, 7)));
        assert_eq!(c.run.args[1], "{bridge}/rns");
        assert_eq!(
            c.notice_hash(),
            crate::util::hash::sha256_bytes(b"This installs demo. You install it.")
        );
        // a lock the release does not carry: declared, no payload
        let c = parse("demo.toml", pip_toml("").as_bytes(), |_| None).unwrap();
        assert!(matches!(
            c.payload,
            Payload::Pip {
                lock_bytes: None,
                ..
            }
        ));
    }

    #[test]
    fn unknown_keys_and_bad_values_are_refused() {
        // an unknown key anywhere
        let e = parse_str("demo.toml", &pip_toml("[weight]\nfoo = 1\n")).unwrap_err();
        assert!(matches!(e, ManifestError::Parse(_)), "{e}");
        let e = parse_str("demo.toml", &format!("surprise = true\n{}", pip_toml(""))).unwrap_err();
        assert!(matches!(e, ManifestError::Parse(_)), "{e}");
        // the id must be the file stem
        assert!(matches!(
            parse_str("other.toml", &pip_toml("")),
            Err(ManifestError::IdMismatch { .. })
        ));
        // reserved ids
        let t = pip_toml("")
            .replace("id = \"demo\"", "id = \"rns\"")
            .replace("venv = \"demo\"", "venv = \"rns\"");
        assert!(parse_str("rns.toml", &t).is_err());
        // a lock path, not a file name beside the declaration
        let t = pip_toml("").replace("demo.lock.txt", "../x.txt");
        assert!(parse_str("demo.toml", &t).is_err());
        // an unknown placeholder
        let t = pip_toml("").replace("{bridge}", "{home}");
        assert!(parse_str("demo.toml", &t).is_err());
        // a base port
        let t = pip_toml("").replace("health = \"ready\"", "health = \"ready\"\nports = [11183]");
        assert!(parse_str("demo.toml", &t).is_err());
        // a bidi override in the notice
        let t = pip_toml("").replace("You install it.", "You \u{202e}install it.");
        assert!(parse_str("demo.toml", &t).is_err());
        // too large, before parsing
        let big = vec![b'#'; MAX_MANIFEST_BYTES + 1];
        assert!(matches!(
            parse("demo.toml", &big, |_| None),
            Err(ManifestError::TooLarge(_))
        ));
    }

    #[test]
    fn a_binary_declaration_pins_url_hash_and_size() {
        let t = format!(
            r#"id = "rpc"
adapter = "app:deltachat"
title = "Delta Chat core"
[payload]
kind = "binary"
url = "https://example.org/dl/deltachat-rpc-server"
sha256 = "{}"
size = 1234
[run]
program = "deltachat-rpc-server"
health = "none"
[license]
spdx = "MPL-2.0"
read_at = "https://example.org/LICENSE"
notice = "This installs deltachat-rpc-server (MPL-2.0)."
"#,
            "b".repeat(64)
        );
        let c = parse("rpc.toml", t.as_bytes(), |_| None).unwrap();
        assert_eq!(c.adapter_dir(), "deltachat");
        assert!(matches!(c.payload, Payload::Binary { size: 1234, .. }));
        assert_eq!(c.run.health, Health::None);
        // http is refused; an upper-case hash is refused
        assert!(parse(
            "rpc.toml",
            t.replace("https://", "http://").as_bytes(),
            |_| None
        )
        .is_err());
        assert!(parse(
            "rpc.toml",
            t.replace(&"b".repeat(64), &"B".repeat(64)).as_bytes(),
            |_| None
        )
        .is_err());
    }

    #[test]
    fn invalid_files_are_not_registered() {
        let files = vec![
            ("demo.toml".to_string(), pip_toml("").into_bytes()),
            ("demo.lock.txt".to_string(), LOCK.as_bytes().to_vec()),
            ("broken.toml".to_string(), b"id = ".to_vec()),
            (
                "extra.toml".to_string(),
                pip_toml("unknown = 1")
                    .replace("demo", "extra")
                    .into_bytes(),
            ),
        ];
        let loaded = load(&files);
        assert_eq!(
            loaded.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["demo"]
        );
    }

    #[test]
    fn lock_hashes_are_collected() {
        let lock = format!(
            "a==1 \\\n  --hash=sha256:{a} \\\n  --hash=sha256:{b}\nb==2 --hash=sha256:{A}\n# --hash=sha256:short\n",
            a = "a".repeat(64),
            b = "b".repeat(64),
            A = "A".repeat(64)
        );
        assert_eq!(
            lock_hashes(lock.as_bytes()),
            vec!["a".repeat(64), "b".repeat(64)]
        );
        assert_eq!(
            url_file_name("https://h/x/tool-1.2.tar.gz?x=1").as_deref(),
            Some("tool-1.2.tar.gz")
        );
        assert_eq!(url_file_name("https://h/x/"), None);
        assert_eq!(url_file_name("https://h/x/.hidden"), None);
    }
}
