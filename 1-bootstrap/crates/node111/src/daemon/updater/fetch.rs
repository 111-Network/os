//! Manifest fetch + verification + staging decision (one check).
//!
//! Contract order (spec §Updates): fetch manifest → verify minisig → check
//! floor → only then trust any pin. Clock faults surface as a class (minisign
//! verification is time-free — fetches degrade, verification doesn't).
//!
//! Layer 7 §9.2–§9.3 and §9.5 T3, T6:
//! - **Origins.** A check walks an [`Origin`] list in order; a fetch failure (a 404
//!   included) tries the next one. The visible order is `[home hub,
//!   install.111.network, GitHub Releases]` ([`UpdaterConfig::visible_order`]); the
//!   hourly loop checks the internet bases only, and none while
//!   [`UpdaterConfig::check_internet`] is off. The home hub's origin and
//!   `111 update --from` are checked by their own callers (Layer 7 L4).
//! - **The blob rule (S5).** An internet base fetches the payload at the signed
//!   `payload.url`; a hub, directory or typed-URL origin fetches
//!   `<origin>/blob/<sha256 of the pin>` and ignores `payload.url`. The pin's
//!   sha256 and size are checked either way (`stage::stage_payload`, and again by
//!   the root helper). Only an internet base writes `record_success`
//!   (`last-update-check.json`) and the clock-fault marker; the others write
//!   their per-origin record ([`ORIGINS_RECORD`]).
//! - **The staging lock (S1).** The whole fetch–stage–request sequence runs under
//!   `state/staging.lock` ([`stage::StagingLock`]).
//! - **T3.** A manifest whose `min-upgrade-from` is above the running release is
//!   refused before its payload is fetched.
//! - **T6.** `http://` reaches the fetcher only as the typed exception of a hub or
//!   typed-URL origin (`crate::util::http::HttpException`).

use super::stage::{self, LockTake, LockWait, StagingLock};
use crate::calver::Calver;
use crate::daemon::DaemonLayout;
use crate::floor::Floor;
use crate::manifest::{ChannelManifest, MANIFEST_MAX_BYTES, SIGNATURE_MAX_BYTES};
use crate::util::http::{Fetcher, HttpException};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Default channel endpoints (first-party only): own domain primary, GitHub
/// Releases page fallback (canonical artifact store per spec).
pub const DEFAULT_MANIFEST_BASE: &str = "https://install.111.network/stable";
pub const FALLBACK_MANIFEST_BASE: &str =
    "https://github.com/111-Network/os/releases/download/channel";

/// Target triple for payload selection (compile-time, matches the binary).
#[cfg(target_arch = "x86_64")]
pub const TARGET_TRIPLE: &str = "x86_64-unknown-linux-musl";
#[cfg(target_arch = "aarch64")]
pub const TARGET_TRIPLE: &str = "aarch64-unknown-linux-musl";

/// The three names every origin serves (Layer 7 §9.1).
pub const MANIFEST_NAME: &str = "channel-111-stable.toml";
pub const SIGNATURE_NAME: &str = "channel-111-stable.toml.minisig";

/// The payload cap of every fetch (the payload is read whole, §9.6).
pub const PAYLOAD_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// The per-origin record under `state/` (Layer 7 §9.2 S5, §9.3): each hub,
/// directory or typed-URL origin's last answer and the newest release it showed.
pub const ORIGINS_RECORD: &str = "update-origins.json";

/// Origins kept in [`ORIGINS_RECORD`]; the oldest-added one leaves first.
const ORIGINS_RECORD_MAX: usize = 16;
const ORIGINS_RECORD_MAX_BYTES: u64 = 64 * 1024;
const DETAIL_MAX_CHARS: usize = 300;

#[derive(Debug, Clone)]
pub struct UpdaterConfig {
    pub manifest_base: String,
    pub fallback_base: Option<String>,
    /// Running release calver.
    pub current_version: Calver,
    /// Fetch budget per call.
    pub timeout: Duration,
    /// `updates.check_internet` (Layer 7 §9.3, D30 Q2; default `true`, today's
    /// behaviour): `false` ⇒ no check contacts an internet base. Layer 7 L4 wires
    /// it to `state/relay.json`.
    pub check_internet: bool,
}

impl UpdaterConfig {
    pub fn production(current_version: Calver) -> Self {
        UpdaterConfig {
            manifest_base: DEFAULT_MANIFEST_BASE.to_string(),
            fallback_base: Some(FALLBACK_MANIFEST_BASE.to_string()),
            current_version,
            timeout: Duration::from_secs(30),
            check_internet: true,
        }
    }

    /// The internet bases in order (§9.3): what the hourly loop checks. Empty
    /// while `check_internet` is off.
    pub fn internet_origins(&self) -> Vec<Origin> {
        if !self.check_internet {
            return Vec::new();
        }
        std::iter::once(&self.manifest_base)
            .chain(self.fallback_base.iter())
            .map(|base| Origin::Internet { base: base.clone() })
            .collect()
    }

    /// The visible order `[home hub, install.111.network, GitHub Releases]`
    /// (§9.3): every origin this node knows, whether or not a check contacts it
    /// (`GET /api/v1/update/origins` lists them all; Layer 7 L4).
    pub fn visible_order(&self, home_hub: Option<Origin>) -> Vec<Origin> {
        home_hub
            .into_iter()
            .chain(
                std::iter::once(&self.manifest_base)
                    .chain(self.fallback_base.iter())
                    .map(|base| Origin::Internet { base: base.clone() }),
            )
            .collect()
    }
}

/// One update origin (Layer 7 §9.1–§9.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The home hub's `origin_base` (§9.1, S4): `http://<hub host>:11180/origin/v1/`,
    /// fetched under [`HttpException::HomeHubOrigin`].
    HomeHub { base: String },
    /// An internet base ([`DEFAULT_MANIFEST_BASE`], [`FALLBACK_MANIFEST_BASE`]):
    /// the payload at the signed `payload.url`.
    Internet { base: String },
    /// `111 update --from <dir>` (§9.2): the three names read from the directory.
    Dir { path: PathBuf },
    /// `111 update --from <url>` (§9.2): a URL the user typed, fetched under
    /// [`HttpException::UserTypedFrom`] when it is `http://`.
    Url { base: String },
}

impl Origin {
    /// The home hub's origin from the host of its fabric address (§9.1, S4): the
    /// dashboard's `DEFAULT_PORT`, never `:80`. An IPv6 literal is bracketed. The
    /// host comes from a peer's record, so anything but an IP literal or a plain
    /// DNS name (letters, digits, `-`, `.`) is refused (`None`).
    pub fn home_hub(host: &str) -> Option<Origin> {
        let bracketed = host.strip_prefix('[').and_then(|h| h.strip_suffix(']'));
        let bare = bracketed.unwrap_or(host);
        let host = if let Ok(v6) = bare.parse::<std::net::Ipv6Addr>() {
            format!("[{v6}]")
        } else if bracketed.is_none()
            && !bare.is_empty()
            && bare.len() <= 253
            && bare
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        {
            bare.to_ascii_lowercase()
        } else {
            return None;
        };
        Some(Origin::HomeHub {
            base: format!(
                "http://{host}:{}/origin/v1/",
                crate::daemon::dashboard::DEFAULT_PORT
            ),
        })
    }

    /// An internet base: fetches `payload.url`, writes `record_success`.
    pub fn is_internet(&self) -> bool {
        matches!(self, Origin::Internet { .. })
    }

    /// `home_hub`, `internet`, `dir` or `url`.
    pub fn kind(&self) -> &'static str {
        match self {
            Origin::HomeHub { .. } => "home_hub",
            Origin::Internet { .. } => "internet",
            Origin::Dir { .. } => "dir",
            Origin::Url { .. } => "url",
        }
    }

    /// The base URL or the directory, as shown and recorded.
    pub fn label(&self) -> String {
        match self {
            Origin::HomeHub { base } | Origin::Internet { base } | Origin::Url { base } => {
                base.clone()
            }
            Origin::Dir { path } => path.to_string_lossy().into_owned(),
        }
    }

    fn http_exception(&self) -> Option<HttpException> {
        match self {
            Origin::HomeHub { .. } => Some(HttpException::HomeHubOrigin),
            Origin::Url { .. } => Some(HttpException::UserTypedFrom),
            Origin::Internet { .. } | Origin::Dir { .. } => None,
        }
    }
}

#[derive(Debug)]
pub enum FetchOutcome {
    UpToDate,
    Staged {
        version: Calver,
    },
    /// The run this one waited for on the staging lock already staged this
    /// release with this manifest; nothing was fetched or written again (§9.2).
    AlreadyStaged {
        version: Calver,
    },
    /// Manifest/sig/floor refusal — valid-looking but rejected (log-worthy).
    Refused {
        reason: String,
    },
    /// Fetch/clock problems — degraded, not refused.
    Degraded {
        reason: String,
        clock_fault: bool,
    },
    /// A non-blocking take found `state/staging.lock` held (§9.2): nothing was
    /// fetched or staged (`111 update --from` exits 75; Layer 7 L4).
    Held,
    /// The origin list was empty (`check_internet` off and no other origin):
    /// nothing was contacted.
    NoOrigin,
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}

/// Compare the HTTP Date of a fetched response against local time. A skew
/// beyond the threshold is a clock-fault class (Pi has no RTC).
fn clock_skewed(date_header: Option<&str>) -> bool {
    let Some(h) = date_header else { return false };
    let Ok(parsed) = time::OffsetDateTime::parse(h, &time::format_description::well_known::Rfc2822)
    else {
        return false;
    };
    let skew = (time::OffsetDateTime::now_utc() - parsed).abs();
    skew > time::Duration::hours(1)
}

/// Where an origin's bytes come from.
enum Source {
    Http { fetcher: Fetcher, base: String },
    Dir { path: PathBuf },
}

impl Source {
    fn open(origin: &Origin, timeout: Duration) -> anyhow::Result<Source> {
        Ok(match origin {
            Origin::HomeHub { base } | Origin::Internet { base } | Origin::Url { base } => {
                let mut fetcher = Fetcher::new(timeout, PAYLOAD_MAX_BYTES)?;
                if let Some(e) = origin.http_exception() {
                    fetcher = fetcher.allowing_http(e);
                }
                Source::Http {
                    fetcher,
                    base: base.trim_end_matches('/').to_string(),
                }
            }
            Origin::Dir { path } => Source::Dir { path: path.clone() },
        })
    }

    /// One of the origin's files (`name` is relative: the two manifest names or
    /// `blob/<sha256>`), at most `max` bytes, checked before the read.
    fn get(&self, name: &str, max: u64) -> anyhow::Result<Vec<u8>> {
        match self {
            Source::Http { fetcher, base } => fetcher.get_capped(&format!("{base}/{name}"), max),
            Source::Dir { path } => read_capped(&path.join(name), max),
        }
    }

    /// An absolute URL (an internet base's `payload.url`).
    fn get_url(&self, url: &str, max: u64) -> anyhow::Result<Vec<u8>> {
        match self {
            Source::Http { fetcher, .. } => fetcher.get_capped(url, max),
            Source::Dir { .. } => anyhow::bail!("a directory origin has no payload URL"),
        }
    }

    fn date(&self, name: &str) -> anyhow::Result<Option<String>> {
        match self {
            Source::Http { fetcher, base } => fetcher.head_date(&format!("{base}/{name}")),
            Source::Dir { .. } => Ok(None),
        }
    }
}

/// Read a regular file of at most `max` bytes; the size is checked before the
/// read and again while reading (the file may grow under us).
fn read_capped(path: &Path, max: u64) -> anyhow::Result<Vec<u8>> {
    let file = std::fs::File::open(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let meta = file.metadata()?;
    anyhow::ensure!(meta.is_file(), "{} is not a regular file", path.display());
    anyhow::ensure!(
        meta.len() <= max,
        "{} is {} bytes (at most {max})",
        path.display(),
        meta.len()
    );
    let mut buf = Vec::with_capacity(meta.len() as usize);
    file.take(max + 1).read_to_end(&mut buf)?;
    anyhow::ensure!(
        buf.len() as u64 <= max,
        "{} grew past {max} bytes while read",
        path.display()
    );
    Ok(buf)
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The hourly check (§9.3): the internet bases in order, waiting for the staging
/// lock. With `check_internet` off it contacts nothing ([`FetchOutcome::NoOrigin`]).
pub fn check_once(cfg: &UpdaterConfig, layout: &DaemonLayout) -> anyhow::Result<FetchOutcome> {
    check_origins(cfg, layout, &cfg.internet_origins(), LockWait::Wait)
}

/// One check against `origins` in order, under `state/staging.lock` for the
/// whole fetch–stage–request sequence (§9.2, S1). `LockWait::NoWait` answers
/// [`FetchOutcome::Held`] when another run holds it. The first origin whose
/// manifest arrives decides; one that cannot be fetched (a 404 included), or
/// whose payload cannot be, passes to the next.
pub fn check_origins(
    cfg: &UpdaterConfig,
    layout: &DaemonLayout,
    origins: &[Origin],
    wait: LockWait,
) -> anyhow::Result<FetchOutcome> {
    if origins.is_empty() {
        return Ok(FetchOutcome::NoOrigin);
    }
    let lock = match StagingLock::acquire(&layout.state_dir(), wait)? {
        LockTake::Taken(lock) => lock,
        LockTake::Held => return Ok(FetchOutcome::Held),
    };
    let mut failures = Vec::new();
    for origin in origins {
        match check_origin(cfg, layout, origin, &lock)? {
            Step::Done(outcome) => return Ok(outcome),
            Step::Next(why) => failures.push(format!("{}: {why}", origin.label())),
        }
    }
    Ok(FetchOutcome::Degraded {
        reason: format!("no origin answered ({})", failures.join("; ")),
        clock_fault: false,
    })
}

enum Step {
    Done(FetchOutcome),
    /// This origin could not be fetched from: try the next.
    Next(String),
}

fn check_origin(
    cfg: &UpdaterConfig,
    layout: &DaemonLayout,
    origin: &Origin,
    lock: &StagingLock,
) -> anyhow::Result<Step> {
    let state_dir = layout.state_dir();
    let source = Source::open(origin, cfg.timeout)?;

    // 1. Fetch the manifest and its signature.
    let fetched = source
        .get(MANIFEST_NAME, MANIFEST_MAX_BYTES as u64)
        .and_then(|toml| {
            let sig = source.get(SIGNATURE_NAME, SIGNATURE_MAX_BYTES as u64)?;
            let sig = String::from_utf8(sig).map_err(|_| anyhow::anyhow!("minisig not UTF-8"))?;
            Ok((toml, sig))
        });
    let (manifest_bytes, sig_text) = match fetched {
        Ok(v) => v,
        Err(e) => {
            note_origin(layout, origin, unreachable(&e), None, Some(&e))?;
            return Ok(Step::Next(format!("manifest fetch failed: {e:#}")));
        }
    };
    // Clock-fault is a surfaced class regardless of outcome (spec §Updates): a
    // marker the dashboard reads, from an internet base's Date header only (a
    // hub's clock is no better than ours; no extra request to one).
    if origin.is_internet() {
        let date_header = source.date(MANIFEST_NAME).unwrap_or(None);
        let clock_fault = clock_skewed(date_header.as_deref());
        set_clock_fault_marker(layout, clock_fault, date_header.as_deref())?;
    }

    // 2. Verify (minisig + floor), then trust.
    let floor = Floor::read(&state_dir.join("floor.json"))?.unwrap_or_else(Floor::initial);
    let manifest = match ChannelManifest::parse_verified(
        &manifest_bytes,
        &sig_text,
        super::MANIFEST_PUBKEY.trim(),
        &floor,
    ) {
        Ok(m) => m,
        Err(e) => {
            note_origin(layout, origin, Answer::Refused, None, Some(&e))?;
            return Ok(Step::Done(FetchOutcome::Refused {
                reason: format!("{e:#}"),
            }));
        }
    };
    let release = manifest.release();

    // 3. Newer than what runs?
    if release <= cfg.current_version {
        succeeded(layout, origin, Answer::UpToDate, release)?;
        return Ok(Step::Done(FetchOutcome::UpToDate));
    }

    // 4. T3 (§9.5): never stage a release that refuses an upgrade from ours.
    if manifest.min_upgrade_from() > cfg.current_version {
        let reason = format!(
            "release {release} needs at least {} installed (min-upgrade-from); this node runs {}",
            manifest.min_upgrade_from(),
            cfg.current_version
        );
        note_origin(layout, origin, Answer::Refused, Some(release), None)?;
        return Ok(Step::Done(FetchOutcome::Refused { reason }));
    }

    // 5. The payload for our triple.
    let Some(payload) = manifest.payload_for(TARGET_TRIPLE) else {
        note_origin(layout, origin, Answer::Refused, Some(release), None)?;
        return Ok(Step::Done(FetchOutcome::Refused {
            reason: format!("manifest ships no payload for {TARGET_TRIPLE}"),
        }));
    };
    let manifest_sha = crate::util::hash::sha256_bytes(&manifest_bytes);
    // The run this one waited for may have staged exactly this (§9.2: a
    // concurrent run stages once).
    if lock.waited() && stage::already_requested(&state_dir, &release, &manifest_sha) {
        succeeded(layout, origin, Answer::Staged, release)?;
        return Ok(Step::Done(FetchOutcome::AlreadyStaged { version: release }));
    }
    // The blob rule (S5): an internet base at the signed URL; any other origin
    // at blob/<sha256 of the pin>, never at payload.url.
    let cap = payload.size.min(PAYLOAD_MAX_BYTES);
    let tarball = if origin.is_internet() {
        source.get_url(&payload.url, cap)
    } else if is_sha256_hex(&payload.sha256) {
        source.get(&format!("blob/{}", payload.sha256), cap)
    } else {
        note_origin(layout, origin, Answer::Refused, Some(release), None)?;
        return Ok(Step::Done(FetchOutcome::Refused {
            reason: "the payload pin's sha256 is not 64 lowercase hex digits".into(),
        }));
    };
    let tarball = match tarball {
        Ok(t) => t,
        Err(e) => {
            note_origin(layout, origin, unreachable(&e), Some(release), Some(&e))?;
            return Ok(Step::Next(format!("payload fetch failed: {e:#}")));
        }
    };
    let staged = stage::stage_payload(&tarball, payload, &state_dir.join("staging"), &release)?;
    // Layer 5 cross-check (os/5-messaging/README.md §19 L1): the staged tree's
    // bin/meshmud, panel bundles and presence lock against the manifest's pins.
    // Advisory here — the root helper repeats it on the tree it extracts itself
    // (D12) — but a release whose payload disagrees with its own manifest is
    // refused now, its staging bundle removed and the helper never asked.
    if let Err(e) = manifest.verify_layer5_tree(&staged.join("tree"), TARGET_TRIPLE) {
        let _ = std::fs::remove_dir_all(&staged);
        note_origin(layout, origin, Answer::Refused, Some(release), Some(&e))?;
        return Ok(Step::Done(FetchOutcome::Refused {
            reason: format!("payload disagrees with the manifest's Layer 5 pins: {e:#}"),
        }));
    }
    // The root helper re-verifies from the staging bundle alone (D12): the
    // signed channel manifest + its minisig ride alongside the tarball.
    crate::util::fs::atomic_write(&staged.join(MANIFEST_NAME), &manifest_bytes, 0o644)?;
    crate::util::fs::atomic_write(&staged.join(SIGNATURE_NAME), sig_text.as_bytes(), 0o644)?;
    stage::write_update_request(&state_dir, &release, &manifest_sha, &staged)?;
    succeeded(layout, origin, Answer::Staged, release)?;
    Ok(Step::Done(FetchOutcome::Staged { version: release }))
}

/// A check that answered: `record_success` for an internet base, the per-origin
/// record for any other (S5).
fn succeeded(
    layout: &DaemonLayout,
    origin: &Origin,
    answer: Answer,
    release: Calver,
) -> anyhow::Result<()> {
    if origin.is_internet() {
        record_success(layout)
    } else {
        note_origin(layout, origin, answer, Some(release), None)
    }
}

fn unreachable(e: &anyhow::Error) -> Answer {
    if crate::util::http::is_not_found(e)
        || e.chain().any(|c| {
            c.downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
        })
    {
        Answer::NotFound
    } else {
        Answer::Unreachable
    }
}

/// Persist last-successful-check (dashboard field + receipt of liveness). Only an
/// internet base's check writes it (Layer 7 §9.2, S5).
fn record_success(layout: &DaemonLayout) -> anyhow::Result<()> {
    let body = serde_json::json!({
        "schema": 1,
        "at": now_rfc3339(),
    });
    crate::util::fs::atomic_write(
        &layout.state_dir().join("last-update-check.json"),
        body.to_string().as_bytes(),
        0o644,
    )
}

/// An origin's last answer in [`ORIGINS_RECORD`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    UpToDate,
    Staged,
    Refused,
    /// A 404, or a missing file in a directory origin.
    NotFound,
    Unreachable,
}

/// One origin's entry in [`ORIGINS_RECORD`] (Layer 7 §9.3; T4's "newest release
/// seen: <calver> from <origin>"). `at` is shown, never ordered by; `newest_seen`
/// is the highest release of a manifest that verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginRecord {
    pub kind: String,
    pub origin: String,
    pub answer: Answer,
    /// The release of the verified manifest this answer was about, if any.
    pub release: Option<String>,
    pub newest_seen: Option<String>,
    pub detail: Option<String>,
    pub at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct OriginsFile {
    schema: u32,
    origins: Vec<OriginRecord>,
}

/// The per-origin records (oldest-added first). A missing, oversized or
/// unparsable file reads as empty: it is the daemon's own display state.
pub fn origin_records(state_dir: &Path) -> Vec<OriginRecord> {
    read_capped(&state_dir.join(ORIGINS_RECORD), ORIGINS_RECORD_MAX_BYTES)
        .ok()
        .and_then(|b| serde_json::from_slice::<OriginsFile>(&b).ok())
        .filter(|f| f.schema == 1)
        .map(|f| f.origins)
        .unwrap_or_default()
}

/// Write a hub, directory or typed-URL origin's answer (S5); an internet base
/// writes `record_success` instead. Runs under the staging lock.
fn note_origin(
    layout: &DaemonLayout,
    origin: &Origin,
    answer: Answer,
    release: Option<Calver>,
    error: Option<&anyhow::Error>,
) -> anyhow::Result<()> {
    if origin.is_internet() {
        return Ok(());
    }
    let state_dir = layout.state_dir();
    let mut records = origin_records(&state_dir);
    let (kind, label) = (origin.kind(), origin.label());
    let previous = records
        .iter()
        .position(|r| r.kind == kind && r.origin == label)
        .map(|i| records.remove(i));
    let newest = previous
        .as_ref()
        .and_then(|r| r.newest_seen.as_deref())
        .and_then(|s| s.parse::<Calver>().ok());
    let newest_seen = match (newest, release) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    records.push(OriginRecord {
        kind: kind.to_string(),
        origin: label,
        answer,
        release: release.map(|r| r.to_string()),
        newest_seen: newest_seen.map(|r| r.to_string()),
        detail: error.map(|e| format!("{e:#}").chars().take(DETAIL_MAX_CHARS).collect()),
        at: now_rfc3339(),
    });
    if records.len() > ORIGINS_RECORD_MAX {
        records.drain(..records.len() - ORIGINS_RECORD_MAX);
    }
    let body = serde_json::to_vec(&OriginsFile {
        schema: 1,
        origins: records,
    })?;
    crate::util::fs::atomic_write(&state_dir.join(ORIGINS_RECORD), &body, 0o644)
}

/// `state/clock-fault.json`: present ⇔ the last manifest response's Date
/// header skewed >1h from local time. Fetches degrade, verification (minisign,
/// time-free) doesn't (spec §Updates).
fn set_clock_fault_marker(
    layout: &DaemonLayout,
    skewed: bool,
    date_header: Option<&str>,
) -> anyhow::Result<()> {
    let path = layout.state_dir().join("clock-fault.json");
    if skewed {
        let body = serde_json::json!({
            "schema": 1,
            "kind": "clock-skew",
            "date_header": date_header.unwrap_or(""),
        });
        crate::util::fs::atomic_write(&path, body.to_string().as_bytes(), 0o644)
    } else {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    // Fetch/stage coverage in tests/updater_stage.rs (fixture HTTP server); the
    // origin list's own rules here.
    use super::*;

    fn cfg() -> UpdaterConfig {
        UpdaterConfig::production("2026.10.01".parse().unwrap())
    }

    #[test]
    fn the_visible_order_and_the_internet_switch() {
        let mut c = cfg();
        let hub = Origin::home_hub("192.168.1.10").unwrap();
        assert_eq!(
            hub,
            Origin::HomeHub {
                base: "http://192.168.1.10:11180/origin/v1/".into()
            }
        );
        let order = c.visible_order(Some(hub.clone()));
        assert_eq!(
            order,
            vec![
                hub.clone(),
                Origin::Internet {
                    base: DEFAULT_MANIFEST_BASE.into()
                },
                Origin::Internet {
                    base: FALLBACK_MANIFEST_BASE.into()
                },
            ]
        );
        assert_eq!(c.internet_origins(), order[1..].to_vec());
        c.check_internet = false;
        assert!(c.internet_origins().is_empty());
        assert_eq!(
            c.visible_order(None).len(),
            2,
            "still listed, never contacted"
        );
    }

    #[test]
    fn origin_kinds_and_exceptions() {
        assert_eq!(
            Origin::home_hub("fd00::7"),
            Some(Origin::HomeHub {
                base: "http://[fd00::7]:11180/origin/v1/".into()
            })
        );
        assert_eq!(Origin::home_hub("[fd00::7]"), Origin::home_hub("fd00::7"));
        assert_eq!(
            Origin::home_hub("Hub.local"),
            Some(Origin::HomeHub {
                base: "http://hub.local:11180/origin/v1/".into()
            })
        );
        for bad in [
            "",
            "evil.example/x?",
            "u@10.0.0.1",
            "10.0.0.1:80",
            "[zz]",
            "a b",
        ] {
            assert_eq!(Origin::home_hub(bad), None, "{bad:?}");
        }
        let dir = Origin::Dir {
            path: "/media/stick".into(),
        };
        let url = Origin::Url {
            base: "http://10.0.0.2/r".into(),
        };
        let net = Origin::Internet {
            base: DEFAULT_MANIFEST_BASE.into(),
        };
        assert_eq!(
            Origin::home_hub("h").unwrap().http_exception(),
            Some(HttpException::HomeHubOrigin)
        );
        assert_eq!(url.http_exception(), Some(HttpException::UserTypedFrom));
        assert_eq!(net.http_exception(), None);
        assert_eq!(dir.http_exception(), None);
        assert!(net.is_internet() && !dir.is_internet() && !url.is_internet());
        assert_eq!(dir.label(), "/media/stick");
        assert_eq!(
            [dir.kind(), url.kind(), net.kind()],
            ["dir", "url", "internet"]
        );
    }

    #[test]
    fn an_empty_list_contacts_nothing_and_takes_no_lock() {
        let root = tempfile::tempdir().unwrap();
        let layout = DaemonLayout {
            root: root.path().to_path_buf(),
            etc: root.path().join("etc"),
        };
        let mut c = cfg();
        c.check_internet = false;
        assert!(matches!(
            check_once(&c, &layout).unwrap(),
            FetchOutcome::NoOrigin
        ));
        assert!(!StagingLock::path(&layout.state_dir()).exists());
    }

    #[test]
    fn sha256_hex_is_strict() {
        assert!(is_sha256_hex(&"a".repeat(64)));
        assert!(!is_sha256_hex(&"A".repeat(64)));
        assert!(!is_sha256_hex(&"a".repeat(63)));
        assert!(!is_sha256_hex(&format!("../{}", "a".repeat(61))));
    }

    #[test]
    fn origin_records_keep_the_newest_seen_and_a_bound() {
        let root = tempfile::tempdir().unwrap();
        let layout = DaemonLayout {
            root: root.path().to_path_buf(),
            etc: root.path().join("etc"),
        };
        std::fs::create_dir_all(layout.state_dir()).unwrap();
        let hub = Origin::home_hub("10.0.0.1").unwrap();
        let r = |s: &str| Some(s.parse::<Calver>().unwrap());
        note_origin(&layout, &hub, Answer::Staged, r("2026.10.12"), None).unwrap();
        note_origin(&layout, &hub, Answer::UpToDate, r("2026.10.05"), None).unwrap();
        let recs = origin_records(&layout.state_dir());
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].answer, Answer::UpToDate);
        assert_eq!(recs[0].release.as_deref(), Some("2026.10.05"));
        assert_eq!(recs[0].newest_seen.as_deref(), Some("2026.10.12"));
        // an internet base never writes one
        let net = Origin::Internet {
            base: DEFAULT_MANIFEST_BASE.into(),
        };
        note_origin(&layout, &net, Answer::UpToDate, r("2026.10.20"), None).unwrap();
        assert_eq!(origin_records(&layout.state_dir()).len(), 1);
        for i in 0..20 {
            let o = Origin::Url {
                base: format!("http://10.0.0.{i}/"),
            };
            note_origin(&layout, &o, Answer::Unreachable, None, None).unwrap();
        }
        let recs = origin_records(&layout.state_dir());
        assert_eq!(recs.len(), ORIGINS_RECORD_MAX);
        assert_eq!(recs.last().unwrap().origin, "http://10.0.0.19/");
        assert!(
            !recs.iter().any(|r| r.kind == "home_hub"),
            "oldest left first"
        );
    }
}
