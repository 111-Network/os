//! The full identify sequence (`os/6-transport/README.md` §12 "Identify,
//! once per device, consented (D17 #8), through adopted tools"; DEV §3.2;
//! §16 G5). Orchestration moved here from `meshtastic::identify`, which
//! keeps only the Meshtastic step's child mechanics.
//!
//! [`Identify::run`] probes one device:
//!
//! 1. **Consent.** The route body (`{"consent": true}`) or the accepted
//!    `identify` proposal **is** the consent ([`Consent::Given`]); it is
//!    remembered per device (`DeviceRegistry::remember_consent`, persisted)
//!    so a later run may say [`Consent::Remembered`]. Without either:
//!    [`IdentifyError::ConsentRequired`] and nothing is opened.
//! 2. **Rate limit.** One probe per [`RATE_LIMIT`] per device, measured on a
//!    monotonic [`Clock`] (never the wall clock): [`IdentifyError::RateLimited`].
//! 3. **Target.** `DeviceRegistry::identify_target` (offered, present, its
//!    port owned by nothing else: `interfaces.toml` and the `PortTable`).
//! 4. **Port claim.** The board port is claimed for `identify:<id>` before
//!    anything opens it; a held port is `409 port_busy` and nothing runs.
//! 5. **Steps, in DEV §3.2 order** — (1) RNode (`rnodeconf --info`,
//!    [`rnode`]), (2) Meshtastic (meshmud with the identify profile,
//!    [`meshtastic`]), (3) MeshCore (the MIT `kiss_modem_protocol.md`
//!    `GetVersion`/`GetIdentity` exchange, [`meshcore`]). Each answers an
//!    [`Outcome`]: `Match`, `NoMatch` or `Error`; **a step whose tool is not
//!    installed here answers `Error`, never `NoMatch`**; each has its own
//!    timeout. The first `Match` ends the sequence; an `Error` does not (a
//!    missing `rnodeconf` must not stop a Meshtastic board being identified).
//! 6. **Result.** A `Match` is persisted (`firmware {family, version,
//!    source: probe}`; the Meshtastic step also binds the board and its
//!    identify child becomes the service child, as in Layer 5).
//!    **`family: unknown` is persisted only when every step answered
//!    `NoMatch`** (Layer 5 persisted it after the Meshtastic step alone).
//!    Otherwise nothing is persisted and the run is
//!    [`IdentifyError::Inconclusive`] (`502`, each step's answer named).
//! 7. **Release.** The claim is released at the end, unless the Meshtastic
//!    step transferred it to meshmud (`PortTable::transfer`).
//!
//! One identify runs at a time on a node (the probes share the PortTable and
//! a board's serial line). Daemon wiring: `devices::start` builds the
//! production [`Identify`] ([`install`]) and inserts it into `ApiState`
//! ext; the identify route and the `identify` proposal action call
//! `api.ext::<Arc<Identify>>()` then [`Identify::run`] with
//! [`Consent::Given`].

pub mod meshcore;
pub mod meshtastic;
pub mod rnode;

use super::registry::{IdentifyTarget, IdentifyView, TargetError};
use super::{DeviceFirmware, DeviceRecord, DeviceRegistry, FirmwareFamily, FirmwareSource};
use crate::daemon::dashboard::api_v1::Problem;
use crate::daemon::dashboard::ApiState;
use crate::daemon::meshtastic::meshmud::Uplink;
use crate::daemon::ports::{port_key, Owner, PortBusy, PortTable};
use axum::http::StatusCode;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One probe per device per this long (§12, DEV §3.2 "≤ 1 probe/30 s").
pub const RATE_LIMIT: Duration = Duration::from_secs(30);
/// `PortTable` task prefix of a probe (`identify:<device id>`).
pub const TASK_PREFIX: &str = "identify:";

/// A boxed `Send` future (object-safe async steps without a macro crate).
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The three steps (DEV §3.2 order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StepKind {
    Rnode,
    Meshtastic,
    Meshcore,
}

impl StepKind {
    pub fn as_str(self) -> &'static str {
        match self {
            StepKind::Rnode => "rnode",
            StepKind::Meshtastic => "meshtastic",
            StepKind::Meshcore => "meshcore",
        }
    }
}

/// What one step found (§12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The board runs this family's firmware (`source: probe`).
    Match(DeviceFirmware),
    /// The tool ran and the board is not this family.
    NoMatch,
    /// The step could not answer: tool missing, port refused, timeout,
    /// malformed output. Never counts as `NoMatch`.
    Error(String),
}

/// Why a step ends the whole sequence (nothing persisted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Abort {
    /// The device was forgotten while it was probed.
    NotFound,
    /// The daemon is shutting down (or the module stopped).
    Stopped(String),
}

/// The kind of uplink a step would open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UplinkKind {
    Serial,
    Tcp,
}

impl UplinkKind {
    pub fn of(u: &Uplink) -> Self {
        match u {
            Uplink::Serial { .. } => UplinkKind::Serial,
            Uplink::Tcp { .. } => UplinkKind::Tcp,
        }
    }
}

/// What a step probes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub device_id: String,
    /// The uplink as the registry names it (`/dev/serial/by-id/…`).
    pub uplink: Uplink,
    /// The file a serial step opens: the uplink path under
    /// [`IdentifyConfig::dev_root`] (`/` in production; a fake tree in tests).
    pub path: PathBuf,
    /// The `PortTable` task holding the port (`identify:<id>`).
    pub task: String,
}

/// One identify step (§12).
pub trait Step: Send + Sync {
    fn kind(&self) -> StepKind;
    /// What must be installed, for the `Error` a missing tool answers.
    fn tool(&self) -> &'static str;
    /// The step's tool is installed and it can open this kind of uplink here.
    fn available(&self, kind: UplinkKind) -> bool;
    /// Run the step (only called when [`Step::available`]).
    fn run<'a>(&'a self, probe: &'a Probe) -> BoxFut<'a, Result<Outcome, Abort>>;
}

/// A monotonic clock (tests inject a fake one).
pub trait Clock: Send + Sync {
    fn now(&self) -> Duration;
}

/// Production clock: `Instant` since construction.
pub struct MonotonicClock(Instant);

impl Default for MonotonicClock {
    fn default() -> Self {
        MonotonicClock(Instant::now())
    }
}

impl Clock for MonotonicClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

/// How a run was consented to (§12, D17 #8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consent {
    /// The user consented now (the route body, an accepted proposal).
    Given,
    /// Run only if a consent for this device was remembered.
    Remembered,
}

/// One step's answer in a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepReport {
    pub step: StepKind,
    pub outcome: Outcome,
}

impl StepReport {
    /// `rnode: no match` / `meshcore: error: …` (display, one line).
    pub fn line(&self) -> String {
        match &self.outcome {
            Outcome::Match(fw) => format!(
                "{}: match{}",
                self.step.as_str(),
                fw.version
                    .as_deref()
                    .map(|v| format!(" (version {v})"))
                    .unwrap_or_default()
            ),
            Outcome::NoMatch => format!("{}: no match", self.step.as_str()),
            Outcome::Error(e) => format!("{}: error: {e}", self.step.as_str()),
        }
    }
}

/// A finished run.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// The updated record (what the route answers).
    pub record: DeviceRecord,
    pub steps: Vec<StepReport>,
}

/// Why a run produced no record (`problem()` gives the contract answer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentifyError {
    NotFound,
    /// The device does not offer `identify` (`400`).
    NotOffered(&'static str),
    /// Its port has another owner (`409 port_busy`; nothing opened).
    Busy(PortBusy),
    /// Neither a consent now nor a remembered one (`400`; nothing opened).
    ConsentRequired,
    /// Probed less than [`RATE_LIMIT`] ago (`429 rate_limited`).
    RateLimited {
        retry_after_secs: u64,
    },
    /// No step matched and not every step answered `NoMatch`: nothing
    /// persisted (`502 upstream_error`, each step's answer named).
    Inconclusive(Vec<StepReport>),
    /// The daemon is shutting down (`502`).
    Stopped(String),
}

impl fmt::Display for IdentifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdentifyError::NotFound => f.write_str("no such device"),
            IdentifyError::NotOffered(w) => write!(f, "identify is not offered: {w}"),
            IdentifyError::Busy(b) => b.fmt(f),
            IdentifyError::ConsentRequired => f.write_str("identify needs the user's consent"),
            IdentifyError::RateLimited { retry_after_secs } => {
                write!(f, "probed recently; try again in {retry_after_secs} s")
            }
            IdentifyError::Inconclusive(steps) => write!(
                f,
                "not identified: {}",
                steps
                    .iter()
                    .map(StepReport::line)
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            IdentifyError::Stopped(e) => write!(f, "identify stopped: {e}"),
        }
    }
}

impl std::error::Error for IdentifyError {}

impl IdentifyError {
    /// The contract problem (openapi `identifyDevice`).
    pub fn problem(&self) -> Problem {
        match self {
            IdentifyError::NotFound => {
                Problem::new(StatusCode::NOT_FOUND, "not_found", "no such device")
            }
            IdentifyError::NotOffered(why) => Problem::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "This device does not offer identify.",
            )
            .detail(format!("{why}.")),
            IdentifyError::Busy(b) => b.problem(),
            IdentifyError::ConsentRequired => Problem::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "identify needs the user's consent",
            )
            .detail(
                "Send {\"consent\": true} once the user has agreed: opening the board's port can \
                 reset it.",
            ),
            IdentifyError::RateLimited { retry_after_secs } => Problem::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "This board was probed moments ago.",
            )
            .detail(format!(
                "One probe per board every {} s; try again in {retry_after_secs} s.",
                RATE_LIMIT.as_secs()
            )),
            IdentifyError::Inconclusive(_) => Problem::new(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                "The board could not be identified.",
            )
            .detail(self.to_string()),
            IdentifyError::Stopped(e) => Problem::new(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                "The board could not be identified.",
            )
            .detail(e.clone()),
        }
    }
}

/// Production settings ([`IdentifyConfig::production`]).
#[derive(Debug, Clone)]
pub struct IdentifyConfig {
    /// Root a serial uplink is opened under (`/`; tests: a fake tree).
    pub dev_root: PathBuf,
    /// A phone-class node: USB serial cannot be opened (Termux).
    pub phone_like: bool,
    /// `<root>/venvs/rns/bin/rnodeconf`.
    pub rnodeconf: PathBuf,
    /// `HOME` of the rnodeconf process (`<root>`: it creates
    /// `.config/rnodeconf/` there).
    pub home: PathBuf,
    pub rnode_timeout: Duration,
    pub meshcore_timeout: Duration,
    pub rate_limit: Duration,
}

impl IdentifyConfig {
    pub fn production(root: &Path, phone_like: bool) -> Self {
        IdentifyConfig {
            dev_root: PathBuf::from("/"),
            phone_like,
            rnodeconf: rnode::program(root),
            home: root.to_path_buf(),
            rnode_timeout: rnode::TIMEOUT,
            meshcore_timeout: meshcore::TIMEOUT,
            rate_limit: RATE_LIMIT,
        }
    }
}

/// The orchestrator (module doc). Shared as `Arc<Identify>` in `ApiState` ext.
pub struct Identify {
    reg: Arc<DeviceRegistry>,
    ports: Arc<PortTable>,
    steps: Vec<Arc<dyn Step>>,
    clock: Arc<dyn Clock>,
    dev_root: PathBuf,
    rate_limit: Duration,
    last: Mutex<HashMap<String, Duration>>,
    one: tokio::sync::Mutex<()>,
    ticks: AtomicU64,
}

/// A serial uplink's path under `dev_root`.
pub fn mapped_path(dev_root: &Path, uplink: &Uplink) -> PathBuf {
    match uplink {
        Uplink::Serial { path } => dev_root.join(path.trim_start_matches('/')),
        Uplink::Tcp { .. } => PathBuf::from(uplink.port_name()),
    }
}

impl Identify {
    /// An orchestrator over `steps` (in their run order).
    pub fn new(
        reg: Arc<DeviceRegistry>,
        ports: Arc<PortTable>,
        steps: Vec<Arc<dyn Step>>,
        clock: Arc<dyn Clock>,
        dev_root: PathBuf,
        rate_limit: Duration,
    ) -> Arc<Self> {
        Arc::new(Identify {
            reg,
            ports,
            steps,
            clock,
            dev_root,
            rate_limit,
            last: Mutex::new(HashMap::new()),
            one: tokio::sync::Mutex::new(()),
            ticks: AtomicU64::new(0),
        })
    }

    /// The steps in run order.
    pub fn steps(&self) -> Vec<StepKind> {
        self.steps.iter().map(|s| s.kind()).collect()
    }

    /// What some step can probe here (the registry's offer, §12: "whenever
    /// any step can run").
    pub fn view(&self) -> IdentifyView {
        IdentifyView {
            serial: self.steps.iter().any(|s| s.available(UplinkKind::Serial)),
            tcp: self.steps.iter().any(|s| s.available(UplinkKind::Tcp)),
        }
    }

    fn lock_last(&self) -> std::sync::MutexGuard<'_, HashMap<String, Duration>> {
        self.last
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Run the sequence for `device_id` (module doc).
    pub async fn run(
        &self,
        api: &ApiState,
        device_id: &str,
        consent: Consent,
    ) -> Result<Report, IdentifyError> {
        if !super::valid_id(device_id) {
            return Err(IdentifyError::NotFound);
        }
        if consent == Consent::Remembered && !self.reg.has_consent(device_id) {
            return Err(IdentifyError::ConsentRequired);
        }
        let _one = self.one.lock().await;
        let now = self.clock.now();
        if let Some(prev) = self.lock_last().get(device_id).copied() {
            let since = now.saturating_sub(prev);
            if since < self.rate_limit {
                let left = self.rate_limit - since;
                return Err(IdentifyError::RateLimited {
                    retry_after_secs: left.as_secs() + u64::from(left.subsec_nanos() > 0),
                });
            }
        }
        let target: IdentifyTarget = self.reg.identify_target(device_id).map_err(|e| match e {
            TargetError::NotFound => IdentifyError::NotFound,
            TargetError::NotOffered(why) => IdentifyError::NotOffered(why),
            TargetError::Busy { port, owner } => IdentifyError::Busy(PortBusy {
                port: port_key(&port),
                owner: Owner {
                    task: owner,
                    since_tick: 0,
                },
            }),
        })?;
        // the acceptance is the consent, remembered for this device (§12)
        if consent == Consent::Given && !self.reg.remember_consent(device_id) {
            eprintln!(
                "111 identify: the consent for {device_id} could not be remembered (cap \
                 reached); this probe runs on the consent just given"
            );
        }
        let task = format!("{TASK_PREFIX}{device_id}");
        let port = target.uplink.port_name();
        let tick = self.ticks.fetch_add(1, Ordering::SeqCst) + 1;
        self.ports
            .claim(&port, &task, tick)
            .map_err(IdentifyError::Busy)?;
        // from here the port is ours: the probe counts against the limit
        self.lock_last().insert(device_id.to_string(), now);
        let probe = Probe {
            device_id: device_id.to_string(),
            path: mapped_path(&self.dev_root, &target.uplink),
            uplink: target.uplink.clone(),
            task: task.clone(),
        };
        let kind = UplinkKind::of(&target.uplink);
        let mut reports: Vec<StepReport> = Vec::new();
        let mut aborted = None;
        for step in &self.steps {
            let outcome = if step.available(kind) {
                match step.run(&probe).await {
                    Ok(o) => o,
                    Err(a) => {
                        aborted = Some(a);
                        break;
                    }
                }
            } else {
                Outcome::Error(format!(
                    "{} is not installed or cannot run here",
                    step.tool()
                ))
            };
            let matched = matches!(outcome, Outcome::Match(_));
            reports.push(StepReport {
                step: step.kind(),
                outcome,
            });
            if matched {
                break;
            }
        }
        // released unless a step handed it over (Meshtastic: to meshmud)
        let _ = self.ports.release(&port, &task);
        if let Some(a) = aborted {
            return Err(match a {
                Abort::NotFound => IdentifyError::NotFound,
                Abort::Stopped(e) => IdentifyError::Stopped(e),
            });
        }
        let found = reports.iter().find_map(|r| match &r.outcome {
            Outcome::Match(fw) => Some(fw.clone()),
            _ => None,
        });
        let all_no_match = !self.steps.is_empty()
            && reports.len() == self.steps.len()
            && reports.iter().all(|r| r.outcome == Outcome::NoMatch);
        let firmware = match found {
            Some(fw) => fw,
            None if all_no_match => DeviceFirmware {
                family: FirmwareFamily::Unknown,
                version: None,
                source: FirmwareSource::Probe,
            },
            None => return Err(IdentifyError::Inconclusive(reports)),
        };
        let summary = reports
            .iter()
            .map(StepReport::line)
            .collect::<Vec<_>>()
            .join("; ");
        let record = self
            .reg
            .record_firmware(api, device_id, firmware)
            .map_err(|_| IdentifyError::NotFound)?;
        eprintln!("111 identify: {device_id}: {summary}");
        Ok(Report {
            record,
            steps: reports,
        })
    }
}

/// Build the production orchestrator (RNode, Meshtastic, MeshCore), insert
/// it into `api` ext and install its availability as the registry's
/// identify view (module doc). The Meshtastic step uses the module in `api`
/// ext (absent ⇒ that step answers "not installed").
pub fn install(api: &ApiState, reg: &Arc<DeviceRegistry>, cfg: IdentifyConfig) -> Arc<Identify> {
    let ports = api
        .ext::<Arc<PortTable>>()
        .unwrap_or_else(|| Arc::new(PortTable::new()));
    let m = api.ext::<Arc<crate::daemon::meshtastic::Meshtastic>>();
    let steps: Vec<Arc<dyn Step>> = vec![
        Arc::new(rnode::RnodeStep::production(&cfg)),
        Arc::new(meshtastic::MeshtasticStep::new(m)),
        Arc::new(meshcore::MeshcoreStep::new(
            cfg.meshcore_timeout,
            cfg.phone_like,
        )),
    ];
    let ident = Identify::new(
        Arc::clone(reg),
        ports,
        steps,
        Arc::new(MonotonicClock::default()),
        cfg.dev_root.clone(),
        cfg.rate_limit,
    );
    let weak = Arc::downgrade(&ident);
    reg.set_identify_view(move || weak.upgrade().map(|i| i.view()).unwrap_or_default());
    api.insert_ext(Arc::clone(&ident));
    ident
}

/// A serial probe can run on this host (Linux, not a phone-class node).
pub fn serial_runnable(phone_like: bool) -> bool {
    !phone_like && cfg!(target_os = "linux")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problems_follow_the_contract() {
        let busy = PortBusy {
            port: "/dev/ttyACM0".into(),
            owner: Owner {
                task: "ModemManager".into(),
                since_tick: 0,
            },
        };
        let inconclusive = IdentifyError::Inconclusive(vec![
            StepReport {
                step: StepKind::Rnode,
                outcome: Outcome::Error("rnodeconf is not installed".into()),
            },
            StepReport {
                step: StepKind::Meshtastic,
                outcome: Outcome::NoMatch,
            },
        ]);
        for (e, status, code) in [
            (IdentifyError::NotFound, 404, "not_found"),
            (IdentifyError::NotOffered("x"), 400, "bad_request"),
            (IdentifyError::ConsentRequired, 400, "bad_request"),
            (IdentifyError::Busy(busy), 409, "port_busy"),
            (
                IdentifyError::RateLimited {
                    retry_after_secs: 3,
                },
                429,
                "rate_limited",
            ),
            (inconclusive.clone(), 502, "upstream_error"),
            (IdentifyError::Stopped("x".into()), 502, "upstream_error"),
        ] {
            let p = e.problem();
            assert_eq!((p.status, p.code), (status, code), "{e}");
        }
        let d = inconclusive.problem().detail.unwrap();
        assert!(
            d.contains("rnode: error: rnodeconf is not installed"),
            "{d}"
        );
        assert!(d.contains("meshtastic: no match"), "{d}");
    }

    #[test]
    fn serial_paths_map_under_the_dev_root() {
        let up = Uplink::serial("/dev/serial/by-id/usb-X-if00").unwrap();
        assert_eq!(
            mapped_path(Path::new("/tmp/t"), &up),
            PathBuf::from("/tmp/t/dev/serial/by-id/usb-X-if00")
        );
        assert_eq!(
            mapped_path(Path::new("/"), &up),
            PathBuf::from("/dev/serial/by-id/usb-X-if00")
        );
    }
}
