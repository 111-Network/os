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
//!   `cache/packages/<id>/<file name of the URL>`. **Per-target binaries
//!   (Layer 6 C5, additive):** instead of one pin, `[payload.targets.<arch>-<os>]`
//!   tables (`x86_64-linux`, `aarch64-linux`, `aarch64-android`; the Rust
//!   `std::env::consts` spelling) each pin a `url`/`sha256`/`size`; every
//!   table is validated, and the one for this binary's own target
//!   ([`host_triple`]) becomes [`Payload::Binary`]. No table for this target ⇒
//!   [`Payload::Unavailable`] ("there is no build of … for this machine"):
//!   declared, nothing cached, enable refused with that copy. `kind = "none"`
//!   (optional `note`) declares a component the release does not ship
//!   (`Payload::Unavailable` with the note as its copy).
//! - `[run] program`: for `pip`, a path relative to the release tree
//!   (`share/<dir>/<script>.py`, run as `venvs/<id>/bin/python -I <current>/<program>`);
//!   for `binary`, a path relative to `components/<id>/`. `args` may use the
//!   placeholders `{root}`, `{bridge}` (`bridge/<adapter dir>/`), `{install}`
//!   (`components/<id>/`) and `{release}` (`current/`), each as a whole
//!   argument or inside one; any other `{…}` is invalid. `program =
//!   "{payload}"` names the installed file of a per-target binary (its URL's
//!   file name differs per target). `env` (C5, additive) is a table of at most
//!   8 variables set on the child (`[A-Z][A-Z0-9_]*`; never `HOME`, `PATH` or
//!   `LD_*`), values with the same placeholders.
//! - `[requires]` (C5, additive): `glibc = "2.38"` (the host's C library must
//!   be glibc of at least that version: probed with `getconf
//!   GNU_LIBC_VERSION`, else `ldd --version`; Termux and musl hosts refuse with
//!   their own copy), `glibc_distros` (the copy's hint, e.g. "Debian 13 /
//!   Ubuntu 24.04"), `component = "<id>"` (another component that must be
//!   enabled first).
//! - `[run] health`: `ready` (the first stdout line must be a JSON object with
//!   `"type": "ready"`, within `health_timeout_secs`, default 20) or `none`.
//! - `[license] notice` is the exact text the user acknowledges; its hash
//!   ([`Component::notice_hash`]) is what `POST …/enable` must echo.
//! - `[install] prefetch` (C5, additive; default `true`): whether `111 install`
//!   pre-downloads the payload into `cache/packages/<id>/` (§2.1 rule 3).
//!   `false` keeps a large payload off the base install (the Raspberry Pi
//!   floor, AGENTS.md: the Delta Chat core and Madmail are ~104 MB): the
//!   installer prints one line naming [`fetch_command`] instead, and the
//!   payload is fetched only when a human runs that command. Enable never
//!   downloads (§2.1 rule 3, "no runtime fetch"): without the cached file it
//!   fails closed with `component_unavailable` naming the same command.

use crate::daemon::services::ServiceId;
use serde::Deserialize;
use std::collections::BTreeMap;
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
    #[serde(default)]
    install: RawInstall,
}

/// `[install]` (C5, additive; module doc).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInstall {
    #[serde(default)]
    prefetch: Option<bool>,
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
    /// Per-target pins (`[payload.targets.<arch>-<os>]`, C5).
    #[serde(default)]
    targets: Option<BTreeMap<String, RawTarget>>,
    /// The copy of a `kind = "none"` declaration.
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTarget {
    url: String,
    sha256: String,
    size: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequires {
    #[serde(default)]
    python: Option<String>,
    #[serde(default)]
    service: Option<String>,
    #[serde(default)]
    glibc: Option<String>,
    #[serde(default)]
    glibc_distros: Option<String>,
    #[serde(default)]
    component: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRun {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    ports: Vec<i64>,
    #[serde(default)]
    env: BTreeMap<String, String>,
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
    /// `[install] prefetch` (default `true`): `111 install` pre-downloads the
    /// payload; `false` leaves it to [`fetch_command`] (module doc).
    pub prefetch: bool,
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
    /// One pinned file (a binary or a `.tar.gz` of one); for a per-target
    /// declaration, this machine's target.
    Binary {
        url: String,
        sha256: String,
        size: u64,
    },
    /// Declared, but nothing to install on this machine: no build for its
    /// target, or `kind = "none"` (not shipped in this release). Nothing is
    /// cached; enable answers `component_unavailable` with `reason`.
    Unavailable { reason: String },
}

/// `[requires]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Requires {
    /// Lowest `python3` (major, minor).
    pub python: Option<(u32, u32)>,
    /// A supervised service that must serve before the child starts (§6
    /// "Start order"; `transport` = rnsd's shared instance).
    pub service: Option<ServiceId>,
    /// Lowest glibc (major, minor) the payload links against (C5: the
    /// Madmail release needs 2.38).
    pub glibc: Option<(u32, u32)>,
    /// The copy's hint for [`Requires::glibc`] ("Debian 13 / Ubuntu 24.04").
    pub glibc_distros: Option<String>,
    /// Another component that must be enabled first (C5: the Delta Chat core
    /// needs the Madmail relay).
    pub component: Option<String>,
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
    /// Extra environment (placeholders unexpanded; `HOME` is always set).
    pub env: Vec<(String, String)>,
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

    /// Whether this machine has anything to download for it (a pip lock the
    /// release carries, or a binary pin for this target).
    pub fn has_payload(&self) -> bool {
        match &self.payload {
            Payload::Pip { lock_bytes, .. } => lock_bytes.is_some(),
            Payload::Binary { .. } => true,
            Payload::Unavailable { .. } => false,
        }
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

/// The command that downloads the named components' payloads into the
/// install-time cache later (`[install] prefetch = false`, §2.1 rule 3): run
/// as root, online; nothing else fetches a component payload.
pub fn fetch_command(ids: &[&str]) -> String {
    format!("sudo 111 install --components {}", ids.join(","))
}

/// The `[run] program` token naming a per-target binary's installed file.
pub const PAYLOAD_PROGRAM: &str = "{payload}";

/// This binary's own target in the `[payload.targets.<arch>-<os>]` spelling
/// (`x86_64-linux`, `aarch64-linux`, `aarch64-android`, `arm-linux` for a
/// 32-bit ARM build): the payload a per-target declaration installs here.
pub fn host_triple() -> String {
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

/// `<arch>-<os>`: lowercase ASCII, digits and `_` in the arch, one `-`.
fn is_target_triple(s: &str) -> bool {
    let Some((arch, os)) = s.split_once('-') else {
        return false;
    };
    s.len() <= 32
        && !arch.is_empty()
        && !os.is_empty()
        && arch
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && os.bytes().all(|b| b.is_ascii_lowercase())
}

/// One validated binary pin `(url, sha256, size)`.
fn binary_pin(
    url: Option<&str>,
    sha256: Option<String>,
    size: Option<u64>,
) -> Result<(String, String, u64), ManifestError> {
    let url = https_url(
        "payload.url",
        url.ok_or_else(|| invalid("payload.url", "a binary payload pins its url"))?,
    )?;
    if url_file_name(&url).is_none() {
        return Err(invalid("payload.url", "ends in a plain file name"));
    }
    let sha256 = sha256
        .filter(|h| {
            h.len() == 64
                && h.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .ok_or_else(|| invalid("payload.sha256", "64 lowercase hex"))?;
    let size = size
        .filter(|s| (1..=MAX_BINARY_BYTES).contains(s))
        .ok_or_else(|| invalid("payload.size", "1 byte to 512 MiB"))?;
    Ok((url, sha256, size))
}

/// `2.MINOR` (glibc's major is 2).
fn glibc_version(s: &str) -> Result<(u32, u32), ManifestError> {
    let bad = || invalid("requires.glibc", "2.MINOR, e.g. 2.38");
    let (ma, mi) = s.trim().split_once('.').ok_or_else(bad)?;
    let num = |p: &str| {
        (!p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse::<u32>().ok())
            .flatten()
    };
    match (num(ma), num(mi)) {
        (Some(2), Some(b)) => Ok((2, b)),
        _ => Err(bad()),
    }
}

/// An environment name a declaration may set (never one the runner owns or
/// one that changes how the dynamic linker loads code).
fn env_key_ok(k: &str) -> bool {
    let b = k.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_uppercase()
        && b.iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'_')
        && !matches!(k, "HOME" | "PATH" | "IFS" | "SHELL")
        && !k.starts_with("LD_")
}

/// Parse + validate one declaration. `file` is its file name (`<id>.toml`);
/// `lock_of` finds a lock beside it by file name. A per-target payload picks
/// this binary's own target ([`host_triple`]).
pub fn parse<'a>(
    file: &str,
    bytes: &[u8],
    lock_of: impl Fn(&str) -> Option<&'a [u8]>,
) -> Result<Component, ManifestError> {
    parse_for(file, bytes, lock_of, &host_triple())
}

/// [`parse`] as a machine of `target` would (tests; the installer of another
/// architecture).
pub fn parse_for<'a>(
    file: &str,
    bytes: &[u8],
    lock_of: impl Fn(&str) -> Option<&'a [u8]>,
    target: &str,
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
    let mut per_target = false;
    let payload = match p.kind.as_str() {
        "pip" => {
            if p.url.is_some()
                || p.sha256.is_some()
                || p.size.is_some()
                || p.targets.is_some()
                || p.note.is_some()
            {
                return Err(invalid(
                    "payload",
                    "a pip payload has no url/sha256/size/targets/note",
                ));
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
            if p.lock.is_some() || p.venv.is_some() || p.note.is_some() {
                return Err(invalid("payload", "a binary payload has no lock/venv/note"));
            }
            match p.targets {
                None => {
                    let (url, sha256, size) = binary_pin(p.url.as_deref(), p.sha256, p.size)?;
                    Payload::Binary { url, sha256, size }
                }
                Some(targets) => {
                    if p.url.is_some() || p.sha256.is_some() || p.size.is_some() {
                        return Err(invalid("payload", "one pin or per-target pins, not both"));
                    }
                    if targets.is_empty() || targets.len() > 16 {
                        return Err(invalid("payload.targets", "1 to 16 targets"));
                    }
                    per_target = true;
                    let mut chosen = None;
                    for (triple, t) in targets {
                        if !is_target_triple(&triple) {
                            return Err(invalid(
                                "payload.targets",
                                format!("{triple:?} is not an <arch>-<os> target"),
                            ));
                        }
                        // every target is validated, not only this machine's
                        let pin = binary_pin(Some(&t.url), Some(t.sha256), Some(t.size))?;
                        if triple == target {
                            chosen = Some(pin);
                        }
                    }
                    match chosen {
                        Some((url, sha256, size)) => Payload::Binary { url, sha256, size },
                        None => Payload::Unavailable {
                            reason: format!(
                                "there is no build of {title} for this machine ({target})"
                            ),
                        },
                    }
                }
            }
        }
        "none" => {
            if p.lock.is_some()
                || p.venv.is_some()
                || p.url.is_some()
                || p.sha256.is_some()
                || p.size.is_some()
                || p.targets.is_some()
            {
                return Err(invalid(
                    "payload",
                    "a declaration without a payload pins nothing",
                ));
            }
            let reason = match p.note.as_deref() {
                Some(n) => plain_text("payload.note", n, 200)?,
                None => format!("{title} is not part of this release"),
            };
            Payload::Unavailable { reason }
        }
        other => return Err(invalid("payload.kind", format!("unknown kind {other:?}"))),
    };

    let mut requires = Requires {
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
        glibc: raw
            .requires
            .glibc
            .as_deref()
            .map(glibc_version)
            .transpose()?,
        glibc_distros: None,
        component: match raw.requires.component {
            None => None,
            Some(c) if is_component_id(&c) && c != raw.id => Some(c),
            Some(_) => return Err(invalid("requires.component", "another component's id")),
        },
    };
    if let Some(d) = raw.requires.glibc_distros.as_deref() {
        if requires.glibc.is_none() {
            return Err(invalid(
                "requires.glibc_distros",
                "only with requires.glibc",
            ));
        }
        requires.glibc_distros = Some(plain_text("requires.glibc_distros", d, 64)?);
    }
    if matches!(payload, Payload::Pip { .. }) && requires.python.is_none() {
        return Err(invalid("requires.python", "a pip payload names its python"));
    }

    let r = raw.run;
    let program = if r.program == PAYLOAD_PROGRAM {
        match &payload {
            Payload::Binary { url, .. } if per_target => url_file_name(url).unwrap_or_default(),
            // never run: enable refuses an unavailable payload first
            Payload::Unavailable { .. } => raw.id.clone(),
            _ => {
                return Err(invalid(
                    "run.program",
                    "{payload} names the file of a per-target binary",
                ))
            }
        }
    } else {
        r.program.clone()
    };
    let program = rel_path("run.program", &program)?;
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
    if r.env.len() > 8 {
        return Err(invalid("run.env", "more than 8 variables"));
    }
    let mut env: Vec<(String, String)> = Vec::with_capacity(r.env.len());
    for (k, v) in &r.env {
        if !env_key_ok(k) {
            return Err(invalid(
                "run.env",
                format!("{k:?} is not a name a declaration may set"),
            ));
        }
        env.push((k.clone(), run_arg(v)?));
    }
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
            env,
            health,
        },
        weight: Weight {
            cached_bytes: w.cached_bytes,
            installed_bytes: w.installed_bytes,
            rss_bytes: w.rss_bytes,
        },
        license,
        prefetch: raw.install.prefetch.unwrap_or(true),
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
    fn install_prefetch_defaults_to_true_and_the_delta_chat_pair_opts_out() {
        // default: absent [install] ⇒ pre-downloaded by `111 install`
        assert!(parse_str("demo.toml", &pip_toml("")).unwrap().prefetch);
        let off = parse_str("demo.toml", &pip_toml("[install]\nprefetch = false\n")).unwrap();
        assert!(!off.prefetch);
        // an unknown [install] key is refused like every other table
        assert!(matches!(
            parse_str("demo.toml", &pip_toml("[install]\neager = true\n")),
            Err(ManifestError::Parse(_))
        ));
        assert!(matches!(
            parse_str("demo.toml", &pip_toml("[install]\nprefetch = \"no\"\n")),
            Err(ManifestError::Parse(_))
        ));
        // the Raspberry Pi floor: the ~104 MB Delta Chat + Madmail payloads
        // are never pre-downloaded; the LXMF closure (small) still is
        let loaded = load(&embedded());
        let by = |id: &str| loaded.iter().find(|c| c.id == id).unwrap();
        assert!(by("lxmf").prefetch);
        assert!(!by("deltachat").prefetch);
        assert!(!by("madmail").prefetch);
        assert_eq!(
            fetch_command(&["deltachat", "madmail"]),
            "sudo 111 install --components deltachat,madmail"
        );
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

    fn per_target_toml(extra_run: &str, requires: &str) -> String {
        format!(
            r#"id = "relay"
adapter = "app:deltachat"
title = "Relay"
[payload]
kind = "binary"
[payload.targets.x86_64-linux]
url = "https://example.org/dl/relay-linux-amd64"
sha256 = "{a}"
size = 10
[payload.targets.aarch64-linux]
url = "https://example.org/dl/relay-linux-arm64"
sha256 = "{b}"
size = 20
[requires]
{requires}
[run]
program = "{{payload}}"
args = ["--config", "{{bridge}}/relay.toml"]
{extra_run}
health = "none"
[license]
spdx = "AGPL-3.0-or-later"
read_at = "https://example.org/LICENCE"
notice = "This installs the relay. You install it."
"#,
            a = "a".repeat(64),
            b = "b".repeat(64),
        )
    }

    #[test]
    fn a_per_target_binary_picks_this_machines_pin_and_names_its_file() {
        let t = per_target_toml("", "");
        let c = parse_for("relay.toml", t.as_bytes(), |_| None, "x86_64-linux").unwrap();
        assert_eq!(
            c.payload,
            Payload::Binary {
                url: "https://example.org/dl/relay-linux-amd64".into(),
                sha256: "a".repeat(64),
                size: 10
            }
        );
        assert_eq!(
            c.run.program, "relay-linux-amd64",
            "{{payload}} is the file"
        );
        let c = parse_for("relay.toml", t.as_bytes(), |_| None, "aarch64-linux").unwrap();
        assert_eq!(c.run.program, "relay-linux-arm64");
        assert!(matches!(c.payload, Payload::Binary { size: 20, .. }));
        // no build for a 32-bit ARM machine: declared, with the honest copy
        let c = parse_for("relay.toml", t.as_bytes(), |_| None, "arm-linux").unwrap();
        assert_eq!(
            c.payload,
            Payload::Unavailable {
                reason: "there is no build of Relay for this machine (arm-linux)".into()
            }
        );
        // the host's own parse is one of these
        assert!(parse("relay.toml", t.as_bytes(), |_| None).is_ok());
        assert!(host_triple().contains('-'));
        // every target is validated, not only this machine's
        let bad = t.replace(&"b".repeat(64), "nothex");
        assert!(parse_for("relay.toml", bad.as_bytes(), |_| None, "x86_64-linux").is_err());
        let bad = t.replace("[payload.targets.aarch64-linux]", "[payload.targets.Arm64]");
        assert!(parse_for("relay.toml", bad.as_bytes(), |_| None, "x86_64-linux").is_err());
        // one pin or per-target pins, not both
        let both = t.replace("kind = \"binary\"", "kind = \"binary\"\nsize = 3");
        assert!(parse_for("relay.toml", both.as_bytes(), |_| None, "x86_64-linux").is_err());
        // {payload} only names a per-target binary
        let single = t
            .replace("[payload.targets.x86_64-linux]\n", "")
            .replace(
                &format!(
                    "[payload.targets.aarch64-linux]\nurl = \"https://example.org/dl/relay-linux-arm64\"\nsha256 = \"{}\"\nsize = 20\n",
                    "b".repeat(64)
                ),
                "",
            );
        assert!(parse_for("relay.toml", single.as_bytes(), |_| None, "x86_64-linux").is_err());
    }

    #[test]
    fn requires_glibc_component_and_env_are_validated() {
        let t = per_target_toml(
            "env = { DC_ACCOUNTS_PATH = \"{bridge}/accounts\" }",
            "glibc = \"2.38\"\nglibc_distros = \"Debian 13 / Ubuntu 24.04\"\ncomponent = \"other\"",
        );
        let c = parse_for("relay.toml", t.as_bytes(), |_| None, "x86_64-linux").unwrap();
        assert_eq!(c.requires.glibc, Some((2, 38)));
        assert_eq!(
            c.requires.glibc_distros.as_deref(),
            Some("Debian 13 / Ubuntu 24.04")
        );
        assert_eq!(c.requires.component.as_deref(), Some("other"));
        assert_eq!(
            c.run.env,
            vec![(
                "DC_ACCOUNTS_PATH".to_string(),
                "{bridge}/accounts".to_string()
            )]
        );
        for bad in [
            ("glibc = \"2.38\"", "glibc = \"3.1\""),
            ("glibc = \"2.38\"", "glibc = \"2\""),
            ("glibc = \"2.38\"\n", ""),
            ("component = \"other\"", "component = \"relay\""),
            ("component = \"other\"", "component = \"Not An Id\""),
            ("DC_ACCOUNTS_PATH", "LD_PRELOAD"),
            ("DC_ACCOUNTS_PATH", "HOME"),
            ("DC_ACCOUNTS_PATH", "dc_lower"),
            ("{bridge}/accounts", "{home}/accounts"),
        ] {
            let b = t.replace(bad.0, bad.1);
            assert!(
                parse_for("relay.toml", b.as_bytes(), |_| None, "x86_64-linux").is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_declaration_without_a_payload_carries_its_copy() {
        let t = r#"id = "sx"
adapter = "app:simplex"
title = "SimpleX"
[payload]
kind = "none"
note = "SimpleX is not enabled in this release"
[run]
program = "simplex-chat"
ports = [11232]
health = "none"
[license]
spdx = "AGPL-3.0-only"
read_at = "https://example.org/LICENSE"
notice = "SimpleX is not part of this release."
"#;
        let c = parse("sx.toml", t.as_bytes(), |_| None).unwrap();
        assert_eq!(
            c.payload,
            Payload::Unavailable {
                reason: "SimpleX is not enabled in this release".into()
            }
        );
        let pinned = t.replace("kind = \"none\"", "kind = \"none\"\nsize = 1");
        assert!(parse("sx.toml", pinned.as_bytes(), |_| None).is_err());
        let default = t.replace("note = \"SimpleX is not enabled in this release\"\n", "");
        let c = parse("sx.toml", default.as_bytes(), |_| None).unwrap();
        assert_eq!(
            c.payload,
            Payload::Unavailable {
                reason: "SimpleX is not part of this release".into()
            }
        );
    }
}
