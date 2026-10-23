//! Optional third-party components (Layer 6 §2.1, the optional-component
//! seam; D26 #1 #5): declared in the release, cached at install, installed
//! and started only when a human enables them after acknowledging the licence
//! notice, stopped by disable, deleted only by "remove files".
//!
//! - [`manifest`] — `assets/components/<id>.toml` (+ its lock), embedded.
//! - [`registry`] — disk facts and state, cache verification, install, removal.
//! - [`consent`] — `state/bridge.json`: acknowledgements and the enable switch;
//!   the `409 consent_required` problem with the exact text and its hash.
//! - [`runner`] — child ordering (§6 "Start order") and the post-spawn check.
//!
//! [`Components`] ties them together for the API (C1) and the adapters (C3,
//! C5). **Enable** (§2.1 rule 4) runs, in this order and failing closed at
//! each step: licence notice (a recorded or supplied acknowledgement of the
//! exact notice, else [`EnableError::ConsentRequired`] and nothing happens) →
//! `[requires]` → verify the cache against the pin → install (offline, from
//! the cache) → claim and pre-bind its ports → start (piped stdio, ordered
//! after its dependency) → health. **Disable** stops the child
//! (`stop_and_wait`) and releases its claims; installed files and the cache
//! stay. **Remove files** deletes the installed tree and `bridge/<adapter>/`;
//! **remove cache** deletes `cache/packages/<id>/`, only on that call.
//!
//! Zero weight (§2.1 rule 2): a component that is not enabled has no runner,
//! so no process, task, timer, probe or open file; [`Components`] itself only
//! holds the parsed declarations.
//!
//! **C5 additions (additive):** `[requires] glibc` is checked with
//! [`probe_libc`] (through the wiring's command runner, so tests fake it) and
//! refuses with honest copy on an older glibc, on Termux and on musl;
//! `[requires] component` needs that component enabled first (and
//! [`Components::resume_enabled`] resumes dependencies before dependants); an
//! [`Payload::Unavailable`] payload (no build for this machine, or not in this
//! release) refuses with its copy after `[requires]`; `[run] env` is set on the
//! child with the placeholders expanded.
//!
//! **No runtime fetch (§2.1 rule 3; `[install] prefetch = false`):** a payload
//! missing from the cache answers `409 component_unavailable` (reason
//! `payload_missing`) whose detail names
//! [`manifest::fetch_command`]; enable never downloads.

pub mod consent;
pub mod manifest;
pub mod registry;
pub mod runner;

use crate::daemon::dashboard::api_v1::Problem;
use crate::daemon::ports::{Owner, PortBusy, PortTable};
use crate::daemon::services::ServiceId;
use crate::daemon::supervise::{BackoffPolicy, ChildControl, ChildSpec, PipedIo};
use crate::util::run::Runner;
use axum::http::StatusCode;
use consent::{ConsentCheck, ConsentRequired, ProblemWith};
use manifest::{Component, Health, Payload};
use registry::{ComponentState, Paths, VerifyError};
use runner::{ComponentRunner, Gate, RunnerConfig, RunnerStatus};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The services a component may start after (§6 "Start order").
pub trait Dependencies: Send + Sync {
    /// The supervised child of `service` on this node, if it runs one.
    fn child(&self, service: ServiceId) -> Option<ChildControl>;
    /// `service` serves its dependants now (for `transport`: `rnstatus`
    /// shows the shared instance). Blocking.
    fn serving(&self, service: ServiceId) -> bool;
}

/// Production [`Dependencies`]: children from the daemon's supervised set,
/// `transport` served when [`runner::rnstatus_serving`] says so, any other
/// service when its child runs.
pub struct ServiceDeps {
    pub root: PathBuf,
    pub child_of: Arc<dyn Fn(ServiceId) -> Option<ChildControl> + Send + Sync>,
}

impl Dependencies for ServiceDeps {
    fn child(&self, service: ServiceId) -> Option<ChildControl> {
        (self.child_of)(service)
    }

    fn serving(&self, service: ServiceId) -> bool {
        match service {
            ServiceId::Transport => runner::rnstatus_serving(&self.root),
            other => self
                .child(other)
                .is_some_and(|c| c.state() == crate::daemon::supervise::ChildState::Running),
        }
    }
}

/// Where and how components run.
#[derive(Debug, Clone)]
pub struct ComponentsConfig {
    /// The daemon root (`/var/lib/111`).
    pub root: PathBuf,
    /// The release tree a pip component's script is read from (`current/`).
    pub release: PathBuf,
    pub runner: RunnerConfig,
}

impl ComponentsConfig {
    pub fn new(root: &Path) -> Self {
        ComponentsConfig {
            root: root.to_path_buf(),
            release: root.join("current"),
            runner: RunnerConfig::default(),
        }
    }
}

/// What [`Components`] runs things with.
#[derive(Clone)]
pub struct Wiring {
    /// `python3` and a venv's pip (blocking).
    pub cmd: Arc<dyn Runner>,
    pub ports: Arc<PortTable>,
    pub deps: Arc<dyn Dependencies>,
    /// The daemon tick stamped on port claims (never wall clock).
    pub tick: Arc<dyn Fn() -> u64 + Send + Sync>,
}

/// One component's facts, shaped like the contract's `Component`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub id: String,
    pub adapter: String,
    pub title: String,
    pub state: ComponentState,
    pub cached_bytes: Option<u64>,
    pub installed_bytes: Option<u64>,
    /// Declared resident size while running; `None` when unmeasured.
    pub rss_bytes: Option<u64>,
    pub license: StatusLicense,
    pub notice_hash: String,
    pub acknowledged: bool,
    pub reason_code: Option<&'static str>,
    pub reason: Option<String>,
}

/// The contract's `ComponentLicense`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusLicense {
    pub spdx: String,
    pub read_at: String,
    pub notice: String,
}

/// What [`Components::enable`] achieved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enabled {
    pub status: Status,
    /// `Some` when the child is not spawned yet because its dependency does
    /// not serve (`dependency_down`, `not_serving`); it starts once it does.
    pub waiting: Option<&'static str>,
}

/// Why enable did not complete (fail closed: nothing past the failed step ran).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnableError {
    /// No such registered component (`404 not_found`).
    Unknown,
    /// No valid acknowledgement of this exact notice: nothing installed or
    /// started (`409 consent_required`).
    ConsentRequired { text: String, hash: String },
    /// A `[requires]` check failed (`409 component_unavailable`, `requires_unmet`).
    RequiresUnmet(String),
    /// Nothing (or not everything) cached (`payload_missing`).
    NotCached(String),
    /// A cached file is not the pinned one (`pin_mismatch`); nothing installed.
    HashMismatch(String),
    /// The install from the cache failed; the tree in place is untouched.
    InstallFailed(String),
    /// A port it listens on is held (`409 port_busy`).
    PortBusy(PortBusy),
    /// It started but failed its health check; it was stopped again.
    StartFailed(String),
}

impl EnableError {
    /// The stable reason code (the contract's `Component.reason_code`).
    pub fn reason_code(&self) -> &'static str {
        match self {
            EnableError::Unknown => "unknown_component",
            EnableError::ConsentRequired { .. } => "consent_required",
            EnableError::RequiresUnmet(_) => "requires_unmet",
            EnableError::NotCached(_) => "payload_missing",
            EnableError::HashMismatch(_) => "pin_mismatch",
            EnableError::InstallFailed(_) => "install_failed",
            EnableError::PortBusy(_) => "port_busy",
            EnableError::StartFailed(_) => "health_failed",
        }
    }

    /// The API answer (problem+json; `consent` member on `consent_required`).
    pub fn problem(&self, c: Option<&Component>) -> ProblemWith {
        let unavailable = |detail: &str| {
            ProblemWith::new(
                Problem::new(
                    StatusCode::CONFLICT,
                    "component_unavailable",
                    "This component cannot be enabled.",
                )
                .detail(detail.to_string()),
            )
            .with("reason_code", serde_json::Value::from(self.reason_code()))
        };
        match self {
            EnableError::Unknown => ProblemWith::new(Problem::new(
                StatusCode::NOT_FOUND,
                "not_found",
                "No such component.",
            )),
            EnableError::ConsentRequired { text, hash } => match c {
                Some(c) => ConsentRequired::for_component(c).problem(),
                None => ConsentRequired {
                    action: "component_enable",
                    subject: String::new(),
                    warning_text: text.clone(),
                    hash: hash.clone(),
                    addons: Vec::new(),
                }
                .problem(),
            },
            EnableError::PortBusy(b) => ProblemWith::new(b.problem()),
            EnableError::RequiresUnmet(d)
            | EnableError::NotCached(d)
            | EnableError::HashMismatch(d)
            | EnableError::InstallFailed(d)
            | EnableError::StartFailed(d) => unavailable(d),
        }
    }
}

impl std::fmt::Display for EnableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnableError::Unknown => write!(f, "no such component"),
            EnableError::ConsentRequired { .. } => {
                write!(f, "the licence notice must be acknowledged first")
            }
            EnableError::RequiresUnmet(d)
            | EnableError::NotCached(d)
            | EnableError::HashMismatch(d)
            | EnableError::InstallFailed(d)
            | EnableError::StartFailed(d) => write!(f, "{d}"),
            EnableError::PortBusy(b) => write!(f, "{b}"),
        }
    }
}

impl std::error::Error for EnableError {}

/// Why "remove files" / "remove cache" did not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoveError {
    Unknown,
    /// Disable it first (`409 bad_request`).
    Running,
    Io(String),
}

impl RemoveError {
    pub fn problem(&self) -> Problem {
        match self {
            RemoveError::Unknown => {
                Problem::new(StatusCode::NOT_FOUND, "not_found", "No such component.")
            }
            RemoveError::Running => Problem::new(
                StatusCode::CONFLICT,
                "bad_request",
                "This component is running.",
            )
            .detail("Disable it first, then remove its files."),
            RemoveError::Io(e) => Problem::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "The files could not be removed.",
            )
            .detail(e.clone()),
        }
    }
}

/// Longest an extra argument from an adapter may be.
const EXTRA_ARG_MAX: usize = 256;

struct Inner {
    cfg: ComponentsConfig,
    list: Vec<Component>,
    wiring: Wiring,
    /// The enabled components' runners; the lock also serialises every
    /// enable/disable/remove.
    runners: tokio::sync::Mutex<HashMap<String, ComponentRunner>>,
    /// The last failure per id, for the status' reason.
    last_error: std::sync::Mutex<HashMap<String, (&'static str, String)>>,
}

/// The optional components of this node (module doc). Cheap to clone.
#[derive(Clone)]
pub struct Components {
    inner: Arc<Inner>,
}

impl Components {
    pub fn new(cfg: ComponentsConfig, list: Vec<Component>, wiring: Wiring) -> Self {
        Components {
            inner: Arc::new(Inner {
                cfg,
                list,
                wiring,
                runners: tokio::sync::Mutex::new(HashMap::new()),
                last_error: std::sync::Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Every registered declaration.
    pub fn declared(&self) -> &[Component] {
        &self.inner.list
    }

    pub fn get(&self, id: &str) -> Option<&Component> {
        self.inner.list.iter().find(|c| c.id == id)
    }

    fn root(&self) -> &Path {
        &self.inner.cfg.root
    }

    fn remember(&self, id: &str, e: &EnableError) {
        let mut m = self
            .inner
            .last_error
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        match e {
            EnableError::ConsentRequired { .. } | EnableError::Unknown => {}
            other => {
                m.insert(id.to_string(), (other.reason_code(), other.to_string()));
            }
        }
    }

    fn forget(&self, id: &str) {
        self.inner
            .last_error
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }

    fn status_with(&self, c: &Component, runner: Option<&ComponentRunner>) -> Status {
        let facts = registry::facts(self.root(), c);
        let file = consent::load(self.root());
        let rec = file.record(&c.id);
        let running = runner.is_some_and(ComponentRunner::is_running);
        let state = registry::state_of(&facts, rec.enabled && runner.is_some(), running);
        let (reason_code, reason) = match runner.map(ComponentRunner::status) {
            Some(RunnerStatus::Waiting { reason }) => (
                Some(reason),
                Some(format!(
                    "It starts once {} serves.",
                    c.requires
                        .service
                        .map(|s| s.as_str())
                        .unwrap_or("its dependency")
                )),
            ),
            Some(RunnerStatus::AdapterDown { reason }) => (
                Some(reason),
                Some("It was stopped: its dependency did not serve after it started.".into()),
            ),
            Some(RunnerStatus::Running { .. }) if !running => (
                Some("child_degraded"),
                Some("It keeps exiting; the supervisor backs off.".into()),
            ),
            _ => match self
                .inner
                .last_error
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&c.id)
            {
                Some((code, why)) => (Some(*code), Some(why.clone())),
                None => (None, None),
            },
        };
        let nonzero = |n: u64| (n > 0).then_some(n);
        Status {
            id: c.id.clone(),
            adapter: c.adapter.clone(),
            title: c.title.clone(),
            state,
            cached_bytes: facts.cached_bytes,
            installed_bytes: facts.installed_bytes,
            rss_bytes: nonzero(c.weight.rss_bytes),
            license: StatusLicense {
                spdx: c.license.spdx.clone(),
                read_at: c.license.read_at.clone(),
                notice: c.license.notice.clone(),
            },
            notice_hash: c.notice_hash(),
            acknowledged: file.acknowledged(c),
            reason_code,
            reason,
        }
    }

    /// `id`'s facts (`None`: not registered).
    pub async fn status(&self, id: &str) -> Option<Status> {
        let c = self.get(id)?;
        let runners = self.inner.runners.lock().await;
        Some(self.status_with(c, runners.get(id)))
    }

    /// Every component's facts, by id.
    pub async fn list(&self) -> Vec<Status> {
        let runners = self.inner.runners.lock().await;
        self.inner
            .list
            .iter()
            .map(|c| self.status_with(c, runners.get(&c.id)))
            .collect()
    }

    /// The running component's stdio watch (the adapter attaches here; it is
    /// re-published on every respawn). `None` while not enabled.
    pub async fn stdio(&self, id: &str) -> Option<tokio::sync::watch::Receiver<Option<PipedIo>>> {
        self.inner
            .runners
            .lock()
            .await
            .get(id)
            .map(ComponentRunner::stdio)
    }

    /// Hold `id`'s child for a planned restart of its dependency (§6:
    /// stopped before, started after); `false` when it is not enabled.
    pub async fn stop_before(&self, id: &str) -> bool {
        match self.inner.runners.lock().await.get(id) {
            Some(r) => {
                r.stop_before().await;
                true
            }
            None => false,
        }
    }

    /// Resume `id` after its dependency restarted.
    pub async fn start_after(&self, id: &str) {
        if let Some(r) = self.inner.runners.lock().await.get(id) {
            r.start_after().await;
        }
    }

    /// The ids whose dependency is `service` and that are enabled now (the
    /// transport spine stops these before an rnsd restart).
    pub async fn dependants(&self, service: ServiceId) -> Vec<String> {
        let runners = self.inner.runners.lock().await;
        self.inner
            .list
            .iter()
            .filter(|c| c.requires.service == Some(service) && runners.contains_key(&c.id))
            .map(|c| c.id.clone())
            .collect()
    }

    /// Enable `id` (module doc, the order of steps). `ack` is the hash the
    /// user echoes (`accept_notice_hash`).
    pub async fn enable(&self, id: &str, ack: Option<&str>) -> Result<Enabled, EnableError> {
        self.enable_with(id, ack, Vec::new()).await
    }

    /// [`Self::enable`] with arguments the adapter appends to `[run] args`
    /// (the LXMF helper's `--name <display name>`).
    pub async fn enable_with(
        &self,
        id: &str,
        ack: Option<&str>,
        extra_args: Vec<String>,
    ) -> Result<Enabled, EnableError> {
        let c = self.get(id).cloned().ok_or(EnableError::Unknown)?;
        let mut runners = self.inner.runners.lock().await;
        let result = self.enable_locked(&c, ack, extra_args, &mut runners).await;
        match &result {
            Ok(_) => self.forget(&c.id),
            Err(e) => self.remember(&c.id, e),
        }
        result.map(|waiting| Enabled {
            status: self.status_with(&c, runners.get(&c.id)),
            waiting,
        })
    }

    async fn enable_locked(
        &self,
        c: &Component,
        ack: Option<&str>,
        extra_args: Vec<String>,
        runners: &mut HashMap<String, ComponentRunner>,
    ) -> Result<Option<&'static str>, EnableError> {
        let root = self.root().to_path_buf();
        // 1. the licence notice (§2.1 rule 4): nothing happens without it
        match consent::require(&root, c, ack) {
            Ok(()) => {}
            Err(ConsentCheck::Required(req)) => {
                return Err(EnableError::ConsentRequired {
                    text: req.warning_text,
                    hash: req.hash,
                })
            }
            Err(ConsentCheck::Io(e)) => {
                return Err(EnableError::InstallFailed(format!(
                    "the acknowledgement could not be recorded: {e}"
                )))
            }
        }
        if let Some(r) = runners.get(&c.id) {
            // already enabled: idempotent
            return Ok(match r.status() {
                RunnerStatus::Waiting { reason } => Some(reason),
                _ => None,
            });
        }
        if extra_args.len() > 8
            || extra_args
                .iter()
                .any(|a| a.len() > EXTRA_ARG_MAX || a.chars().any(char::is_control))
        {
            return Err(EnableError::StartFailed("invalid adapter arguments".into()));
        }
        // 2. [requires]
        self.check_requires(c, runners).await?;
        if let Payload::Unavailable { reason } = &c.payload {
            return Err(EnableError::RequiresUnmet(reason.clone()));
        }
        // 3. the cache against the pin, 4. install from it (blocking work)
        let cmd = Arc::clone(&self.inner.wiring.cmd);
        let comp = c.clone();
        let root_b = root.clone();
        tokio::task::spawn_blocking(move || -> Result<(), EnableError> {
            if !registry::installed_from_pin(&root_b, &comp) {
                registry::verify_cache(&root_b, &comp).map_err(|e| match e {
                    // nothing downloads at runtime (§2.1 rule 3): name the
                    // command that fills the cache (`[install] prefetch`)
                    VerifyError::NotCached(d) => EnableError::NotCached(format!(
                        "{d}; the payload is not cached: fetch it with `{}` (or put the pinned file in cache/packages/{}/), then enable again",
                        manifest::fetch_command(&[comp.id.as_str()]),
                        comp.id
                    )),
                    VerifyError::Mismatch(d) => EnableError::HashMismatch(d),
                })?;
            }
            registry::install(&root_b, &comp, cmd.as_ref())
                .map(|_| ())
                .map_err(|e| EnableError::InstallFailed(e.0))
        })
        .await
        .map_err(|e| EnableError::InstallFailed(format!("the install task failed: {e}")))??;
        if let Payload::Binary { .. } = c.payload {
            // the program the declaration names is part of the installed tree
            if !Paths::of(&root, c).install.join(&c.run.program).is_file() {
                return Err(EnableError::InstallFailed(format!(
                    "the installed payload carries no {}",
                    c.run.program
                )));
            }
        }
        // 5. ports: claimed, and pre-bound to catch a holder outside 111
        self.claim_ports(c)?;
        // 6. start, ordered after its dependency
        let spec = match self.child_spec(c, &extra_args) {
            Ok(s) => s,
            Err(e) => {
                self.release_ports(c);
                return Err(e);
            }
        };
        if let Err(e) = consent::update(&root, &c.id, |r| r.enabled = true) {
            self.release_ports(c);
            return Err(EnableError::InstallFailed(format!(
                "the enable switch could not be recorded: {e:#}"
            )));
        }
        let gate = c.requires.service.map(|service| {
            let deps = Arc::clone(&self.inner.wiring.deps);
            let deps2 = Arc::clone(&self.inner.wiring.deps);
            Gate {
                dependency: Arc::new(move || deps.child(service)),
                serving: Arc::new(move || deps2.serving(service)),
            }
        });
        let runner = ComponentRunner::start(spec, gate, self.inner.cfg.runner.clone());
        // 7. health
        match self.health(c, &runner).await {
            Ok(waiting) => {
                runners.insert(c.id.clone(), runner);
                Ok(waiting)
            }
            Err(e) => {
                runner.shutdown().await;
                self.release_ports(c);
                let _ = consent::update(&root, &c.id, |r| r.enabled = false);
                Err(e)
            }
        }
    }

    async fn check_requires(
        &self,
        c: &Component,
        runners: &HashMap<String, ComponentRunner>,
    ) -> Result<(), EnableError> {
        if let Some(need) = c.requires.glibc {
            let cmd = Arc::clone(&self.inner.wiring.cmd);
            let have = tokio::task::spawn_blocking(move || probe_libc(cmd.as_ref()))
                .await
                .unwrap_or(LibC::Other);
            if let Some(copy) = glibc_refusal(c, need, &have) {
                return Err(EnableError::RequiresUnmet(copy));
            }
        }
        if let Some(dep) = &c.requires.component {
            if !runners.contains_key(dep) {
                let title = self.get(dep).map_or(dep.as_str(), |d| d.title.as_str());
                return Err(EnableError::RequiresUnmet(format!(
                    "it needs {title} enabled first"
                )));
            }
        }
        if let Some((major, minor)) = c.requires.python {
            let cmd = Arc::clone(&self.inner.wiring.cmd);
            let out = tokio::task::spawn_blocking(move || cmd.run("python3", &["--version"]))
                .await
                .ok()
                .and_then(Result::ok);
            let have = out
                .filter(|o| o.ok())
                .and_then(|o| parse_python_version(&format!("{}\n{}", o.stdout, o.stderr)));
            match have {
                None => {
                    return Err(EnableError::RequiresUnmet(
                        "python3 is not installed on this machine".into(),
                    ))
                }
                Some(v) if v < (major, minor) => {
                    return Err(EnableError::RequiresUnmet(format!(
                        "it needs python {major}.{minor} or newer; this machine has {}.{}",
                        v.0, v.1
                    )))
                }
                Some(_) => {}
            }
        }
        if let Some(service) = c.requires.service {
            if self.inner.wiring.deps.child(service).is_none() {
                return Err(EnableError::RequiresUnmet(format!(
                    "it needs {} running on this node",
                    service.as_str()
                )));
            }
        }
        if let Payload::Pip { .. } = c.payload {
            let script = self.inner.cfg.release.join(&c.run.program);
            if !script.is_file() {
                return Err(EnableError::RequiresUnmet(format!(
                    "this release carries no {}",
                    c.run.program
                )));
            }
        }
        Ok(())
    }

    fn port_task(c: &Component) -> String {
        format!("component:{}", c.id)
    }

    fn claim_ports(&self, c: &Component) -> Result<(), EnableError> {
        let task = Self::port_task(c);
        let tick = (self.inner.wiring.tick)();
        let mut claimed: Vec<String> = Vec::new();
        for port in &c.run.ports {
            let key = format!("127.0.0.1:{port}");
            let fail = |this: &Self, claimed: &[String], b: PortBusy| {
                for k in claimed {
                    let _ = this.inner.wiring.ports.release(k, &task);
                }
                Err(EnableError::PortBusy(b))
            };
            if let Err(b) = self.inner.wiring.ports.claim(&key, &task, tick) {
                return fail(self, &claimed, b);
            }
            claimed.push(key.clone());
            // only a bind detects a holder outside 111 (§2)
            if std::net::TcpListener::bind(("127.0.0.1", *port)).is_err() {
                let b = PortBusy {
                    port: key,
                    owner: Owner {
                        task: "a program outside 111".into(),
                        since_tick: tick,
                    },
                };
                return fail(self, &claimed, b);
            }
        }
        Ok(())
    }

    fn release_ports(&self, c: &Component) {
        let task = Self::port_task(c);
        for port in &c.run.ports {
            let _ = self
                .inner
                .wiring
                .ports
                .release(&format!("127.0.0.1:{port}"), &task);
        }
    }

    /// The child: `venvs/<id>/bin/python -I <release>/<program> <args>` or
    /// `components/<id>/<program> <args>`, placeholders expanded,
    /// `HOME=<root>`, stderr to `bridge/<adapter dir>/<id>.log`.
    fn child_spec(&self, c: &Component, extra: &[String]) -> Result<ChildSpec, EnableError> {
        let root = self.root();
        let p = Paths::of(root, c);
        if let Err(e) = ensure_private_dir(&p.bridge) {
            return Err(EnableError::InstallFailed(format!(
                "cannot create {}: {e}",
                p.bridge.display()
            )));
        }
        let expand = |a: &str| {
            a.replace("{root}", &root.to_string_lossy())
                .replace("{bridge}", &p.bridge.to_string_lossy())
                .replace("{install}", &p.install.to_string_lossy())
                .replace("{release}", &self.inner.cfg.release.to_string_lossy())
        };
        let mut args: Vec<String> = Vec::new();
        let program = match &c.payload {
            Payload::Unavailable { reason } => {
                return Err(EnableError::RequiresUnmet(reason.clone()));
            }
            Payload::Pip { .. } => {
                args.push("-I".into());
                args.push(
                    self.inner
                        .cfg
                        .release
                        .join(&c.run.program)
                        .to_string_lossy()
                        .into_owned(),
                );
                p.venv.join("bin/python")
            }
            Payload::Binary { .. } => p.install.join(&c.run.program),
        };
        args.extend(c.run.args.iter().map(|a| expand(a)));
        args.extend(extra.iter().cloned());
        let mut env = vec![("HOME".into(), root.to_string_lossy().into_owned())];
        env.extend(c.run.env.iter().map(|(k, v)| (k.clone(), expand(v))));
        Ok(ChildSpec {
            name: c.id.clone(),
            program,
            args,
            env,
            backoff: BackoffPolicy::default(),
            stdout_log: Some(p.bridge.join(format!("{}.log", c.id))),
        })
    }

    /// The health step: `ready` waits for the first line of a spawn (not
    /// consumed: the adapter still reads it) within the timeout; a child
    /// still waiting for its dependency at the deadline is enabled and
    /// waiting.
    async fn health(
        &self,
        c: &Component,
        runner: &ComponentRunner,
    ) -> Result<Option<&'static str>, EnableError> {
        let timeout = match c.run.health {
            Health::Ready { timeout } => timeout,
            Health::None => Duration::from_millis(500),
        };
        let deadline = tokio::time::Instant::now() + timeout;
        let mut stdio = runner.stdio();
        loop {
            let io = stdio.borrow_and_update().clone();
            if let Some(io) = io {
                if c.run.health == Health::None {
                    return Ok(None);
                }
                match tokio::time::timeout_at(deadline, io.first_line()).await {
                    Ok(Some(line)) if is_ready_line(&line) => return Ok(None),
                    Ok(Some(_)) => {
                        return Err(EnableError::StartFailed(format!(
                            "{} started, but its first line was not `ready`",
                            c.title
                        )))
                    }
                    // exited without a line: a respawn may still answer
                    Ok(None) => {}
                    Err(_) => {
                        return Err(EnableError::StartFailed(format!(
                            "{} started, but did not report ready within {}s",
                            c.title,
                            timeout.as_secs()
                        )))
                    }
                }
            }
            match tokio::time::timeout_at(deadline, stdio.changed()).await {
                Ok(Ok(())) => continue,
                Ok(Err(_)) => return Err(EnableError::StartFailed(format!("{} stopped", c.title))),
                Err(_) => {
                    return match runner.status() {
                        RunnerStatus::Waiting { reason } => Ok(Some(reason)),
                        RunnerStatus::AdapterDown { reason } => Ok(Some(reason)),
                        _ if c.run.health == Health::None => Ok(None),
                        _ => Err(EnableError::StartFailed(format!(
                            "{} did not start within {}s",
                            c.title,
                            timeout.as_secs()
                        ))),
                    };
                }
            }
        }
    }

    /// Disable `id` (§2.1 rule 5): stop the child, release its claims, clear
    /// the switch. Idempotent; installed files and the cache stay.
    pub async fn disable(&self, id: &str) -> Option<Status> {
        let c = self.get(id)?.clone();
        let mut runners = self.inner.runners.lock().await;
        if let Some(r) = runners.remove(&c.id) {
            r.shutdown().await;
        }
        self.release_ports(&c);
        if let Err(e) = consent::update(self.root(), &c.id, |r| r.enabled = false) {
            eprintln!(
                "111 components: {}: the switch was not recorded: {e:#}",
                c.id
            );
        }
        self.forget(&c.id);
        Some(self.status_with(&c, None))
    }

    /// Delete the installed tree and the adapter's state (never the cache).
    pub async fn remove_files(&self, id: &str) -> Result<Status, RemoveError> {
        let c = self.get(id).cloned().ok_or(RemoveError::Unknown)?;
        let runners = self.inner.runners.lock().await;
        if runners.contains_key(&c.id) {
            return Err(RemoveError::Running);
        }
        let root = self.root().to_path_buf();
        let comp = c.clone();
        tokio::task::spawn_blocking(move || registry::remove_files(&root, &comp))
            .await
            .map_err(|e| RemoveError::Io(e.to_string()))?
            .map_err(|e| RemoveError::Io(e.to_string()))?;
        Ok(self.status_with(&c, None))
    }

    /// Delete `cache/packages/<id>/` (only on this call).
    pub async fn remove_cache(&self, id: &str) -> Result<Status, RemoveError> {
        let c = self.get(id).cloned().ok_or(RemoveError::Unknown)?;
        let runners = self.inner.runners.lock().await;
        if runners.contains_key(&c.id) {
            return Err(RemoveError::Running);
        }
        registry::remove_cache(self.root(), &c).map_err(|e| {
            RemoveError::Io(if e.kind() == std::io::ErrorKind::PermissionDenied {
                format!("{e}: cache/packages/ is root-owned; remove it as root",)
            } else {
                e.to_string()
            })
        })?;
        Ok(self.status_with(&c, None))
    }

    /// Daemon start: enable again every component whose switch is on (the
    /// recorded acknowledgement must still match its notice). Failures are
    /// logged and leave it off.
    pub async fn resume_enabled(&self) {
        self.resume_enabled_with(|id| async move { self.enable(&id, None).await })
            .await;
    }

    /// [`Self::resume_enabled`] with the per-component enable supplied (W3: the daemon's
    /// `Layer6::resume` passes `api_v1::bridge::enable_component`, so a prepare hook runs
    /// before the child starts, and the ordering lives here only): `enable(id)` is called
    /// for every component whose switch is on, a dependency (`[requires] component`)
    /// before its dependants, declaration order otherwise; an error is logged and leaves
    /// that component off, and the loop goes on. Answers every declared id in that order
    /// (the caller attaches adapters in it).
    pub async fn resume_enabled_with<F, Fut>(&self, mut enable: F) -> Vec<String>
    where
        F: FnMut(String) -> Fut,
        Fut: std::future::Future<Output = Result<Enabled, EnableError>>,
    {
        let file = consent::load(self.root());
        // a dependency (`[requires] component`) before its dependants (a stable sort)
        let mut order: Vec<&Component> = self.inner.list.iter().collect();
        order.sort_by_key(|c| c.requires.component.is_some());
        for c in &order {
            if !file.record(&c.id).enabled {
                continue;
            }
            if let Err(e) = enable(c.id.clone()).await {
                eprintln!("111 components: {} not resumed: {e}", c.id);
            }
        }
        order.into_iter().map(|c| c.id.clone()).collect()
    }

    /// Daemon stop: every runner shut down.
    pub async fn shutdown(&self) {
        let mut runners = self.inner.runners.lock().await;
        for (_, r) in runners.drain() {
            r.shutdown().await;
        }
    }
}

/// The host's C library as far as `[requires] glibc` cares (C5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibC {
    /// glibc (major, minor).
    Glibc(u32, u32),
    /// Termux on Android (bionic; no glibc).
    Termux,
    /// Another C library (musl) or nothing the probes recognise.
    Other,
}

/// Probe the host's C library (blocking): this binary built for Android ⇒
/// Termux; else `getconf GNU_LIBC_VERSION` ("glibc 2.39"), else the first
/// line of `ldd --version` ("ldd (Ubuntu GLIBC 2.39-0ubuntu8) 2.39"), else
/// `uname -o` ("Android" ⇒ Termux). Nothing recognised ⇒ [`LibC::Other`]
/// (fail closed: a glibc requirement refuses).
pub fn probe_libc(cmd: &dyn Runner) -> LibC {
    if cfg!(target_os = "android") {
        return LibC::Termux;
    }
    let out = |program: &str, args: &[&str]| {
        cmd.run(program, args)
            .ok()
            .filter(|o| o.ok())
            .map(|o| format!("{}\n{}", o.stdout, o.stderr))
    };
    if let Some(v) = out("getconf", &["GNU_LIBC_VERSION"]).and_then(|t| {
        let rest = t.trim().strip_prefix("glibc ")?;
        parse_glibc(rest)
    }) {
        return LibC::Glibc(v.0, v.1);
    }
    if let Some(v) = out("ldd", &["--version"]).and_then(|t| {
        let first = t.lines().find(|l| !l.trim().is_empty())?.to_string();
        let lower = first.to_ascii_lowercase();
        if !(lower.contains("glibc") || lower.contains("gnu libc")) {
            return None;
        }
        parse_glibc(first.split_whitespace().last()?)
    }) {
        return LibC::Glibc(v.0, v.1);
    }
    if out("uname", &["-o"]).is_some_and(|t| t.contains("Android")) {
        return LibC::Termux;
    }
    LibC::Other
}

/// `2.39` or `2.39-0ubuntu8` ⇒ (2, 39).
fn parse_glibc(s: &str) -> Option<(u32, u32)> {
    let mut it = s.trim().split(|c: char| !c.is_ascii_digit());
    let major: u32 = it.next()?.parse().ok()?;
    let minor: u32 = it.next()?.parse().ok()?;
    (major == 2).then_some((major, minor))
}

/// The refusal copy of a glibc requirement on `have`, or `None` when met.
pub fn glibc_refusal(c: &Component, need: (u32, u32), have: &LibC) -> Option<String> {
    let distros = c
        .requires
        .glibc_distros
        .as_deref()
        .map(|d| format!(" ({d})"))
        .unwrap_or_default();
    let (title, ma, mi) = (&c.title, need.0, need.1);
    match *have {
        LibC::Glibc(a, b) if (a, b) >= need => None,
        LibC::Glibc(a, b) => Some(format!(
            "this release of {title} needs glibc {ma}.{mi} or newer{distros}; this node has glibc {a}.{b}"
        )),
        LibC::Termux => Some(format!(
            "{title} does not run on Termux: this release needs glibc {ma}.{mi} or newer, which Android does not have; enable it on a Linux hub{distros}"
        )),
        LibC::Other => Some(format!(
            "this release of {title} needs glibc {ma}.{mi} or newer{distros}; this node has another C library (musl, or none the probe recognises)"
        )),
    }
}

/// `Python 3.10.12` ⇒ (3, 10).
fn parse_python_version(text: &str) -> Option<(u32, u32)> {
    let rest = text.split("Python ").nth(1)?;
    let mut it = rest.split(|c: char| !c.is_ascii_digit());
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    Some((major, minor))
}

/// A health line: a JSON object whose `type` is `ready` (§6 `ready {dest,
/// pubkey, instance}`).
fn is_ready_line(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .is_some_and(|v| v.get("type").and_then(|t| t.as_str()) == Some("ready"))
}

/// Create `dir` (and parents) with the leaf 0700; refuse a link.
fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_dir() => return Ok(()),
        Ok(_) => {
            return Err(std::io::Error::other("a link or not a directory"));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_versions_and_ready_lines_parse() {
        assert_eq!(parse_python_version("Python 3.10.12\n"), Some((3, 10)));
        assert_eq!(parse_python_version("\nPython 3.7.3"), Some((3, 7)));
        assert_eq!(parse_python_version("python"), None);
        assert!(is_ready_line(r#"{"type":"ready","dest":"ab"}"#));
        assert!(!is_ready_line(r#"{"type":"msg"}"#));
        assert!(!is_ready_line("ready"));
        assert!(!is_ready_line(r#"["ready"]"#));
    }

    #[test]
    fn the_libc_probe_reads_getconf_then_ldd_then_uname() {
        use crate::testkit::MockRunner;
        use crate::util::run::Output;
        let ok = |s: &str| Output {
            status: 0,
            stdout: s.into(),
            stderr: String::new(),
        };
        let fail = || Output {
            status: 1,
            stdout: String::new(),
            stderr: "nope".into(),
        };
        let r = MockRunner::new(vec![("getconf GNU_LIBC_VERSION", ok("glibc 2.39\n"))]);
        assert_eq!(probe_libc(&r), LibC::Glibc(2, 39));
        let r = MockRunner::new(vec![
            ("getconf GNU_LIBC_VERSION", fail()),
            (
                "ldd --version",
                ok("ldd (Debian GLIBC 2.36-9+deb12u7) 2.36\nCopyright\n"),
            ),
        ]);
        assert_eq!(probe_libc(&r), LibC::Glibc(2, 36));
        let r = MockRunner::new(vec![
            ("getconf GNU_LIBC_VERSION", fail()),
            ("ldd --version", fail()),
            ("uname -o", ok("Android\n")),
        ]);
        assert_eq!(probe_libc(&r), LibC::Termux);
        let r = MockRunner::new(vec![
            ("getconf GNU_LIBC_VERSION", fail()),
            ("ldd --version", ok("musl libc (x86_64)\nVersion 1.2.4\n")),
            ("uname -o", ok("GNU/Linux\n")),
        ]);
        assert_eq!(probe_libc(&r), LibC::Other);
        assert_eq!(parse_glibc("2.41"), Some((2, 41)));
        assert_eq!(parse_glibc("1.2"), None);
    }

    #[test]
    fn enable_errors_map_to_their_problems() {
        let c = manifest::load_embedded()
            .into_iter()
            .find(|c| c.id == "lxmf")
            .unwrap();
        let e = EnableError::ConsentRequired {
            text: c.license.notice.clone(),
            hash: c.notice_hash(),
        };
        let b = e.problem(Some(&c)).body();
        assert_eq!(
            (b["status"].as_u64(), b["code"].as_str()),
            (Some(409), Some("consent_required"))
        );
        assert_eq!(b["consent"]["hash"], c.notice_hash().as_str());
        let b = EnableError::HashMismatch("x.whl is not pinned by the lock".into())
            .problem(Some(&c))
            .body();
        assert_eq!(b["code"], "component_unavailable");
        assert_eq!(b["reason_code"], "pin_mismatch");
        assert_eq!(b["detail"], "x.whl is not pinned by the lock");
        assert_eq!(EnableError::Unknown.problem(None).body()["status"], 404);
        assert_eq!(RemoveError::Running.problem().status, 409);
    }

    /// W3: one resume loop, the enable supplied — dependency first (whatever the
    /// declaration order), a component whose switch is off skipped, a failure logged and
    /// the loop going on; every declared id answered in that order.
    #[tokio::test]
    async fn resume_enabled_with_orders_dependencies_first_and_survives_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let decl = |id: &str, requires: &str| {
            let toml = format!(
                "id = \"{id}\"\nadapter = \"app:deltachat\"\ntitle = \"T {id}\"\n\
                 [payload]\nkind = \"binary\"\nurl = \"https://example.invalid/dl/{id}\"\n\
                 sha256 = \"{sha}\"\nsize = 1\n[requires]\n{requires}\n[run]\n\
                 program = \"{id}\"\nports = []\nhealth = \"none\"\n[license]\n\
                 spdx = \"MPL-2.0\"\nread_at = \"https://example.invalid/L\"\n\
                 notice = \"Test notice for {id}. You install it.\"\n",
                sha = "0".repeat(64)
            );
            (format!("{id}.toml"), toml.into_bytes())
        };
        // the dependant sorts first in the loader's order
        let list = manifest::load(&[
            decl("core", "component = \"relay\""),
            decl("relay", ""),
            decl("off", ""),
        ]);
        assert_eq!(list.len(), 3, "the test declarations register");
        struct NoDeps;
        impl Dependencies for NoDeps {
            fn child(&self, _: ServiceId) -> Option<ChildControl> {
                None
            }
            fn serving(&self, _: ServiceId) -> bool {
                false
            }
        }
        let comps = Components::new(
            ComponentsConfig::new(dir.path()),
            list,
            Wiring {
                cmd: Arc::new(crate::testkit::MockRunner::new(Vec::<(
                    String,
                    crate::util::run::Output,
                )>::new())),
                ports: Arc::new(PortTable::new()),
                deps: Arc::new(NoDeps),
                tick: Arc::new(|| 0),
            },
        );
        for id in ["core", "relay"] {
            consent::update(dir.path(), id, |r| r.enabled = true).unwrap();
        }
        let calls = std::sync::Mutex::new(Vec::new());
        let order = comps
            .resume_enabled_with(|id| {
                calls.lock().unwrap().push(id.clone());
                async move { Err(EnableError::StartFailed(format!("{id} refused"))) }
            })
            .await;
        assert_eq!(
            *calls.lock().unwrap(),
            ["relay", "core"],
            "the dependency first, the switched-off one skipped, a failure does not stop it"
        );
        assert_eq!(
            order,
            ["off", "relay", "core"],
            "the loader's order, dependants last"
        );
        // the default enable is the same loop: nothing cached, so both stay off
        comps.resume_enabled().await;
        for id in ["relay", "core", "off"] {
            assert!(comps.stdio(id).await.is_none(), "{id} runs nothing");
        }
    }
}
