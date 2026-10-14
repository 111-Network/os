//! Meshtastic in `111d` (Layer 5 §14, D19 g): service `meshtastic`.
//!
//! A Meshtastic board is **identified** (consented, [`identify`]), gets a
//! service row whose page hosts Meshtastic's own web client (through the
//! [`shim`]), and its nodes-heard feed **Nearby** ([`presence`]). 111 itself
//! never transmits on the Meshtastic mesh and never changes node
//! configuration.
//!
//! Two supervised children, exactly like rnsd and `111-agent`:
//! - **meshmud** ([`meshmud`]) holds the board's one API-client slot: uplink =
//!   the binding's serial port (Linux only), a Wi-Fi board's `host:4403`, or
//!   meshtasticd on `127.0.0.1:4403`; downlink = TCP `127.0.0.1:11230` only.
//!   Its binary is `current/bin/meshmud`; absent ⇒ honestly absent.
//! - **meshtastic-presence** (GPL-3.0 Python sidecar, own hash-locked venv
//!   under `venvs/presence`, script under `current/share/meshtastic-presence/`)
//!   reads the downlink and writes `fabric/presence/meshtastic.json`.
//!
//! [`Meshtastic`] owns both: it starts them from the binding in
//! `state/devices.json` at daemon start, reconciles every tick (a binding
//! removed by *forget* stops them; a port another owner held is retried),
//! claims the board port and the downlink in the daemon's port-owner table
//! (`daemon::ports`, task `meshmud`), registers both children for
//! `POST /services/meshtastic/restart` (meshmud first, then the reader —
//! `services::Supervised`), and joins both on shutdown.
//!
//! The identify child is spawned with the identify profile
//! (`meshmud::identify_backoff`: its first exit is final) and becomes the
//! service child in place; should it exit later, the monitor swaps it for a
//! child with the service policy on its next tick — the port stays claimed by
//! `meshmud` throughout, and that respawn is the same one the service policy
//! would have made.
//!
//! [`Meshtastic::attach_panels`] follows the panel host: a revoked
//! `meshtastic` panel session closes its shim downlink at once, and the
//! shim's drop count is reported for `panel.state.dropped`.
//!
//! Facts for the registry (`Facts.meshmud`, `Facts.radio.meshmud_bound`) come
//! from cached snapshots ([`Meshtastic::facts`], [`Meshtastic::bound_ports`]):
//! the monitor refreshes them every [`MeshtasticConfig::tick`] — the downlink
//! LISTEN check reads `/proc/net/tcp{,6}` and never connects; the uplink is
//! "lost" while a serial device node is gone, or (TCP uplink to an IP
//! literal) while no ESTABLISHED socket to it exists in `/proc/net/tcp{,6}`.
//! Board-reported channel utilization is not known to the daemon (the reader
//! excludes the local board from its file), so it stays `null`.

pub mod identify;
pub mod meshmud;
pub mod presence;
pub mod shim;

use crate::daemon::dashboard::ApiState;
use crate::daemon::devices::registry::MeshtasticView;
use crate::daemon::devices::DeviceRegistry;
use crate::daemon::intelligence::ChildDisplay;
use crate::daemon::panels::PanelHost;
use crate::daemon::ports::{PortBusy, PortTable};
use crate::daemon::services::probe::{Provider, Sources};
use crate::daemon::services::{MeshmudFacts, PanelInfo, ServiceId, Supervised};
use crate::daemon::supervise::{self, BackoffPolicy, ChildHandle, ChildState};
use crate::daemon::DaemonLayout;
use meshmud::Uplink;
use presence::{NearbySource, Presence, Refresh};
use shim::{MeshtasticShim, ShimConfig};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// Port-owner table task of the service child.
pub const MESHMUD_TASK: &str = "meshmud";

/// Where everything lives (production: [`MeshtasticConfig::production`]).
#[derive(Debug, Clone)]
pub struct MeshtasticConfig {
    pub root: PathBuf,
    /// meshmud's TCP downlink (`127.0.0.1:11230`).
    pub downlink: SocketAddr,
    /// `current/bin/meshmud`.
    pub meshmud_bin: PathBuf,
    /// `venvs/presence/bin/python` (built by the installer from the lock).
    pub reader_python: PathBuf,
    /// `current/share/meshtastic-presence/meshtastic_presence.py`.
    pub reader_script: PathBuf,
    /// `fabric/presence` (0700).
    pub presence_dir: PathBuf,
    /// `meshtastic/meshmud.toml` (0600).
    pub config_path: PathBuf,
    /// `meshtastic/meshmud.log` (`services::probe::meshmud_log`).
    pub log: PathBuf,
    /// `meshtastic/presence.log` (the reader's stderr).
    pub reader_log: PathBuf,
    /// A phone-class node: USB serial cannot be opened (Termux).
    pub phone_like: bool,
    /// procfs `net` dir for LISTEN / ESTABLISHED checks (`/proc/net`).
    pub proc_net: PathBuf,
    /// Root a serial uplink's `/dev/…` path is checked under for presence
    /// (`/`; tests point it at a fake tree). meshmud itself always opens the
    /// real path.
    pub dev_root: PathBuf,
    /// Monitor cadence (2 s like the services registry).
    pub tick: Duration,
    /// Identify: how long the reader's `--once` may take.
    pub identify_timeout: Duration,
    /// Identify: how long to wait for meshmud's downlink to LISTEN.
    pub downlink_wait: Duration,
    /// SIGTERM → SIGKILL grace when identify or reconcile stops a child.
    pub stop_grace: Duration,
    pub meshmud_backoff: BackoffPolicy,
    pub shim: ShimConfig,
}

impl MeshtasticConfig {
    pub fn production(layout: &DaemonLayout, phone_like: bool) -> Self {
        let state = meshmud::state_dir(&layout.root);
        MeshtasticConfig {
            root: layout.root.clone(),
            downlink: meshmud::DOWNLINK,
            meshmud_bin: layout.current_release().join("bin/meshmud"),
            reader_python: layout.root.join("venvs/presence/bin/python"),
            reader_script: layout
                .current_release()
                .join("share/meshtastic-presence/meshtastic_presence.py"),
            presence_dir: presence::presence_dir(&layout.root),
            config_path: state.join("meshmud.toml"),
            log: crate::daemon::services::probe::meshmud_log(layout),
            reader_log: state.join("presence.log"),
            phone_like,
            proc_net: PathBuf::from("/proc/net"),
            dev_root: PathBuf::from("/"),
            tick: Duration::from_secs(2),
            identify_timeout: Duration::from_secs(90),
            downlink_wait: Duration::from_secs(15),
            stop_grace: crate::daemon::CHILD_STOP_GRACE,
            meshmud_backoff: meshmud::backoff(),
            shim: ShimConfig::production(meshmud::DOWNLINK),
        }
    }

    /// meshmud is installed (`current/bin/meshmud` is a file).
    pub fn meshmud_installed(&self) -> bool {
        self.meshmud_bin.is_file()
    }

    /// The presence reader's venv and script are installed.
    pub fn reader_installed(&self) -> bool {
        self.reader_python.is_file() && self.reader_script.is_file()
    }
}

/// The running (or wanted) service for the bound board.
struct Service {
    device_id: String,
    uplink: Uplink,
    meshmud: Option<ChildHandle>,
    reader: Option<ChildHandle>,
    /// Another owner held the board port or the downlink at the last try.
    port_held_by: Option<String>,
    /// `meshmud` is the identify child (identify profile: its first exit is
    /// final); swapped for a service-policy child once it exits.
    identify_born: bool,
}

/// Cached facts the monitor refreshes (read by providers without blocking).
#[derive(Default)]
struct Status {
    downlink_listening: bool,
    /// meshmud running and its uplink absent since then.
    uplink_down_since: Option<Instant>,
    board: Option<String>,
    firmware_version: Option<String>,
    usb_uplink: bool,
    log: Option<String>,
    identify_usb: bool,
    identify_tcp: bool,
    bound_ports: Vec<String>,
}

struct Ctx {
    api: Weak<ApiState>,
    reg: Arc<DeviceRegistry>,
    base: Supervised,
}

/// The Meshtastic module (one per daemon; shared as `Arc<Meshtastic>` in
/// `ApiState` ext).
pub struct Meshtastic {
    cfg: MeshtasticConfig,
    ports: Arc<PortTable>,
    shim: MeshtasticShim,
    presence: Arc<Presence>,
    ticks: AtomicU64,
    /// Serializes identify, reconcile and shutdown.
    transition: tokio::sync::Mutex<()>,
    service: Mutex<Option<Service>>,
    status: Mutex<Status>,
    ctx: Mutex<Option<Ctx>>,
    stopping: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Decode one `/proc/net/tcp{,6}` address (`HEXIP:HEXPORT`, each 32-bit word
/// in host — little-endian — order).
fn proc_addr(s: &str) -> Option<SocketAddr> {
    let (ip, port) = s.rsplit_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let word = |h: &str| u32::from_str_radix(h, 16).ok().map(u32::to_le_bytes);
    let ip = match ip.len() {
        8 => IpAddr::from(word(ip)?),
        32 => {
            let mut b = [0u8; 16];
            for i in 0..4 {
                b[i * 4..i * 4 + 4].copy_from_slice(&word(&ip[i * 8..i * 8 + 8])?);
            }
            IpAddr::from(b)
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip.to_canonical(), port))
}

/// Is there an ESTABLISHED (`01`) TCP socket to `remote` in a procfs table?
pub fn established_in_table(table: &str, remote: SocketAddr) -> bool {
    let want = SocketAddr::new(remote.ip().to_canonical(), remote.port());
    table.lines().skip(1).any(|line| {
        let mut f = line.split_whitespace();
        let (_sl, _local, rem, state) = (f.next(), f.next(), f.next(), f.next());
        state == Some("01") && rem.and_then(proc_addr) == Some(want)
    })
}

/// [`established_in_table`] over `<net_dir>/tcp` and `tcp6`; `None` when
/// procfs is unreadable.
pub fn established_under(net_dir: &Path, remote: SocketAddr) -> Option<bool> {
    let v4 = std::fs::read_to_string(net_dir.join("tcp")).ok()?;
    let v6 = std::fs::read_to_string(net_dir.join("tcp6")).unwrap_or_default();
    Some(established_in_table(&v4, remote) || established_in_table(&v6, remote))
}

/// Is the uplink there? Serial: the device node exists under `dev_root`
/// (`/` in production); TCP to an IP literal: an ESTABLISHED socket to it;
/// anything else: unknown (`None`).
pub fn uplink_present(uplink: &Uplink, net_dir: &Path, dev_root: &Path) -> Option<bool> {
    match uplink {
        Uplink::Serial { path } => Some(dev_root.join(path.trim_start_matches('/')).exists()),
        Uplink::Tcp { host, port } => {
            let ip: IpAddr = host.parse().ok()?;
            established_under(net_dir, SocketAddr::new(ip, *port))
        }
    }
}

/// Wrap the panels provider so the Meshtastic panel's `state.dropped` carries
/// the shim's drop count (§14 "drop count surfaced in `panel.state`").
pub fn with_dropped(
    panels: Option<Provider<Vec<PanelInfo>>>,
    shim: MeshtasticShim,
) -> Option<Provider<Vec<PanelInfo>>> {
    let inner = panels?;
    Some(Arc::new(move || {
        let mut v = inner()?;
        for p in v.iter_mut().filter(|p| p.id == ServiceId::Meshtastic) {
            p.state.dropped = Some(shim.dropped());
        }
        Some(v)
    }))
}

/// One revocation-stream event for the shim; `false` = the stream ended.
pub fn on_revocation(
    shim: &MeshtasticShim,
    r: Result<crate::daemon::panels::ticket::Revoked, tokio::sync::broadcast::error::RecvError>,
) -> bool {
    use tokio::sync::broadcast::error::RecvError;
    match r {
        Ok(rev) => {
            if rev.panel == ServiceId::Meshtastic.as_str() {
                shim.close_session(&rev.session);
            }
            true
        }
        Err(RecvError::Lagged(_)) => {
            shim.close_all();
            true
        }
        Err(RecvError::Closed) => false,
    }
}

impl Meshtastic {
    pub fn new(cfg: MeshtasticConfig, ports: Arc<PortTable>) -> Arc<Self> {
        let presence = Arc::new(Presence::new(cfg.presence_dir.join(presence::FILE)));
        let shim = MeshtasticShim::new(cfg.shim.clone());
        Arc::new(Meshtastic {
            cfg,
            ports,
            shim,
            presence,
            ticks: AtomicU64::new(0),
            transition: tokio::sync::Mutex::new(()),
            service: Mutex::new(None),
            status: Mutex::new(Status::default()),
            ctx: Mutex::new(None),
            stopping: AtomicBool::new(false),
        })
    }

    pub fn config(&self) -> &MeshtasticConfig {
        &self.cfg
    }

    /// The `meshtastic-http` shim (register it in the panels' `ShimRegistry`).
    pub fn shim(&self) -> MeshtasticShim {
        self.shim.clone()
    }

    pub fn presence(&self) -> &Presence {
        &self.presence
    }

    pub fn ports(&self) -> &Arc<PortTable> {
        &self.ports
    }

    fn next_tick(&self) -> u64 {
        self.ticks.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Install the registry providers (`Facts.meshmud`,
    /// `Facts.radio.meshmud_bound`) and wrap a panels provider with the drop
    /// count. Call before `services::probe::spawn`.
    pub fn install_providers(self: &Arc<Self>, sources: &mut Sources) {
        let me = Arc::clone(self);
        sources.meshmud = Some(Arc::new(move || Some(me.facts())));
        let me = Arc::clone(self);
        sources.meshmud_bound = Some(Arc::new(move || Some(me.bound_ports())));
        sources.panels = with_dropped(sources.panels.take(), self.shim());
    }

    /// Follow the panel host (§13/§14): a revoked `meshtastic` panel session
    /// closes its shim downlink at once (cookie revoked, panel disabled, LAN
    /// off), and every tick the shim's drop count is reported for
    /// `panel.state.dropped`. Falling behind the revocation stream closes
    /// every session (fail closed: a viewer reconnects on its next call).
    pub fn attach_panels(self: &Arc<Self>, host: &Arc<PanelHost>) -> tokio::task::JoinHandle<()> {
        let shim = self.shim();
        let mut rx = host.revocations();
        let host = Arc::downgrade(host);
        let me = Arc::downgrade(self);
        let every = self.cfg.tick;
        tokio::spawn(async move {
            let mut t = tokio::time::interval(every);
            t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    r = rx.recv() => {
                        if !on_revocation(&shim, r) {
                            break;
                        }
                    }
                    _ = t.tick() => {
                        let (Some(h), Some(m)) = (host.upgrade(), me.upgrade()) else { break };
                        if m.stopping.load(Ordering::SeqCst) {
                            break;
                        }
                        h.report_dropped(ServiceId::Meshtastic, shim.dropped());
                    }
                }
            }
        })
    }

    // ------------------------------------------------------------ facts

    /// `Facts.meshmud` (cheap: cached snapshots and child states).
    pub fn facts(&self) -> MeshmudFacts {
        let (child_state, running, port_held_by) = {
            let svc = lock(&self.service);
            match svc.as_ref() {
                Some(s) => match &s.meshmud {
                    Some(h) => (
                        ChildDisplay::from(Some(h.state())),
                        h.pid().is_some(),
                        s.port_held_by.clone(),
                    ),
                    None => (ChildDisplay::Absent, false, s.port_held_by.clone()),
                },
                None => (ChildDisplay::Absent, false, None),
            }
        };
        let st = lock(&self.status);
        MeshmudFacts {
            running,
            child_state,
            downlink_listening: st.downlink_listening,
            node_db_present: self.presence.present(),
            board: st.board.clone(),
            firmware_version: st.firmware_version.clone(),
            nodes_heard: self.presence.count(),
            uplink_lost_secs: st.uplink_down_since.map(|t| t.elapsed().as_secs()),
            channel_utilization: None,
            usb_uplink: st.usb_uplink,
            port_held_by,
            log: st.log.clone(),
        }
    }

    /// `Facts.radio.meshmud_bound`: the bound board's port while meshmud can
    /// run here (a USB binding on a phone never runs, so it binds nothing).
    pub fn bound_ports(&self) -> Vec<String> {
        lock(&self.status).bound_ports.clone()
    }

    /// meshmud's process is up now.
    pub fn meshmud_running(&self) -> bool {
        lock(&self.service)
            .as_ref()
            .and_then(|s| s.meshmud.as_ref())
            .is_some_and(|h| h.state() == ChildState::Running && h.pid().is_some())
    }

    /// The registry's view (identify offer, meshmud liveness, held ports).
    pub fn view(&self) -> MeshtasticView {
        let meshmud_running = self.meshmud_running();
        let st = lock(&self.status);
        MeshtasticView {
            identify_usb: st.identify_usb,
            identify_tcp: st.identify_tcp,
            meshmud_running,
            held: self
                .ports
                .held()
                .into_iter()
                .map(|(k, o)| (k, o.task))
                .collect(),
        }
    }

    /// `GET /api/v1/nearby` sources: the bound board's node DB, once the
    /// reader produced one.
    pub fn nearby_sources(&self) -> Vec<NearbySource> {
        let device = lock(&self.service).as_ref().map(|s| s.device_id.clone());
        let (Some(id), Some(nodes)) = (device, self.presence.nodes()) else {
            return Vec::new();
        };
        let label = lock(&self.status)
            .board
            .clone()
            .unwrap_or_else(|| "the bound board".into());
        vec![NearbySource::meshtastic(&id, &label, nodes)]
    }

    fn publish_nearby(&self, api: &ApiState) {
        api.events.publish(
            "nearby",
            &serde_json::json!({ "sources": self.nearby_sources() }),
        );
    }

    // ------------------------------------------------------------ lifecycle

    fn ctx(&self) -> Option<(Arc<ApiState>, Arc<DeviceRegistry>)> {
        let c = lock(&self.ctx);
        let c = c.as_ref()?;
        Some((c.api.upgrade()?, Arc::clone(&c.reg)))
    }

    /// Start: hook the device registry, reconcile once (meshmud + reader from
    /// the persisted binding), install the restart wiring, spawn the monitor.
    /// `base` = the other services' children (`Supervised` is re-inserted
    /// into `api` ext whenever meshtastic's children change).
    pub async fn start(
        self: &Arc<Self>,
        api: Arc<ApiState>,
        reg: Arc<DeviceRegistry>,
        base: Supervised,
    ) {
        {
            let me = Arc::downgrade(self);
            reg.set_meshtastic_view(move || me.upgrade().map(|m| m.view()).unwrap_or_default());
        }
        *lock(&self.ctx) = Some(Ctx {
            api: Arc::downgrade(&api),
            reg: Arc::clone(&reg),
            base,
        });
        self.install_supervised();
        self.tick_once().await;
        let me = Arc::downgrade(self);
        let every = self.cfg.tick;
        tokio::spawn(async move {
            let mut t = tokio::time::interval(every);
            t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            t.tick().await;
            loop {
                t.tick().await;
                let Some(m) = me.upgrade() else { break };
                if m.stopping.load(Ordering::SeqCst) {
                    break;
                }
                m.tick_once().await;
            }
        });
    }

    /// One monitor pass: reconcile (skipped while an identify runs), then
    /// refresh the cached facts and the presence snapshot.
    pub async fn tick_once(&self) {
        if self.stopping.load(Ordering::SeqCst) {
            return;
        }
        if let Ok(_t) = self.transition.try_lock() {
            self.reconcile().await;
        }
        self.refresh().await;
    }

    /// Re-insert `Arc<Supervised>` = base + meshmud, then the reader.
    fn install_supervised(&self) {
        let Some((api, _)) = self.ctx() else { return };
        let base = match lock(&self.ctx).as_ref() {
            Some(c) => c.base.clone(),
            None => return,
        };
        let controls: Vec<_> = lock(&self.service)
            .as_ref()
            .map(|s| {
                s.meshmud
                    .iter()
                    .chain(s.reader.iter())
                    .map(ChildHandle::control)
                    .collect()
            })
            .unwrap_or_default();
        let sup = if controls.is_empty() {
            base
        } else {
            base.with_children(ServiceId::Meshtastic, controls)
        };
        api.insert_ext(Arc::new(sup));
    }

    /// Bring the children in line with the binding (caller holds the
    /// transition lock).
    async fn reconcile(&self) {
        let Some((api, reg)) = self.ctx() else { return };
        let desired = reg.meshtastic_binding();
        let current = lock(&self.service)
            .as_ref()
            .map(|s| (s.device_id.clone(), s.uplink.clone(), s.meshmud.is_some()));
        let same = match (&desired, &current) {
            (Some((id, b)), Some((cid, cup, _))) => id == cid && b.uplink == *cup,
            (None, None) => true,
            _ => false,
        };
        if !same {
            if current.is_some() {
                self.stop_service().await;
                // the old board's node DB must never stand for the new one
                self.remove_presence_file();
                if self.presence.clear() {
                    self.publish_nearby(&api);
                }
            }
            if let Some((id, b)) = desired {
                self.start_service(&id, b.uplink);
            }
            self.install_supervised();
        } else if let Some((id, up, false)) = current {
            // wanted but not running (port busy before): try again
            let retry = lock(&self.service)
                .as_ref()
                .is_some_and(|s| s.port_held_by.is_some());
            if retry {
                *lock(&self.service) = None;
                self.start_service(&id, up);
                self.install_supervised();
            }
        } else if current.is_some() {
            self.swap_identify_child().await;
        }
    }

    /// The identify child exited (its profile never respawns): replace it
    /// with a meshmud under the service policy. The claims stay with
    /// `meshmud`; the config on disk is re-rendered for the same uplink.
    async fn swap_identify_child(&self) {
        let (old, uplink) = {
            let mut svc = lock(&self.service);
            match svc.as_mut() {
                Some(s)
                    if s.identify_born
                        && s.meshmud
                            .as_ref()
                            .is_some_and(|h| h.state() == ChildState::Degraded) =>
                {
                    s.identify_born = false;
                    (s.meshmud.take(), s.uplink.clone())
                }
                _ => return,
            }
        };
        if let Some(old) = old {
            old.stop_and_wait(self.cfg.stop_grace).await;
        }
        eprintln!(
            "111 meshtastic: meshmud (started by identify) exited; restarting it as the service"
        );
        if let Err(e) = meshmud::write_config(&self.cfg.config_path, &uplink, self.cfg.downlink) {
            eprintln!("111 meshtastic: cannot render meshmud's config: {e:#}");
            return;
        }
        let child = supervise::spawn_child(meshmud::spec(
            &self.cfg.meshmud_bin,
            &self.cfg.config_path,
            &self.cfg.log,
            self.cfg.meshmud_backoff.clone(),
        ));
        if let Some(s) = lock(&self.service).as_mut() {
            s.meshmud = Some(child);
        }
        self.install_supervised();
    }

    /// Delete the reader's output file (best effort; never follows a link).
    fn remove_presence_file(&self) {
        match std::fs::remove_file(self.presence.path()) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!(
                "111 meshtastic: cannot remove {}: {e}",
                self.presence.path().display()
            ),
        }
    }

    /// Claim the board port and the downlink for meshmud. On a refusal
    /// nothing stays claimed.
    fn claim_for(&self, task: &str, uplink: &Uplink, tick: u64) -> Result<(), PortBusy> {
        self.ports.claim(&uplink.port_name(), task, tick)?;
        if let Err(b) = self.ports.claim(&self.cfg.downlink.to_string(), task, tick) {
            let _ = self.ports.release(&uplink.port_name(), task);
            return Err(b);
        }
        Ok(())
    }

    fn release_for(&self, task: &str, uplink: &Uplink) {
        let _ = self.ports.release(&uplink.port_name(), task);
        let _ = self.ports.release(&self.cfg.downlink.to_string(), task);
    }

    /// meshmud may run here for this uplink.
    fn runnable(&self, uplink: &Uplink) -> bool {
        !(uplink.is_usb() && (self.cfg.phone_like || !cfg!(target_os = "linux")))
    }

    fn spawn_reader(&self) -> Option<ChildHandle> {
        if !self.cfg.reader_installed() {
            eprintln!("111 meshtastic: presence reader not installed — Nearby stays empty");
            return None;
        }
        if let Err(e) = meshmud::ensure_private_dir(&self.cfg.presence_dir) {
            eprintln!(
                "111 meshtastic: cannot prepare {}: {e}",
                self.cfg.presence_dir.display()
            );
        }
        // a file left from an earlier run (or another board) must never make
        // this one look LIVE before the reader has written its own
        self.remove_presence_file();
        self.presence.clear();
        Some(supervise::spawn_child(meshmud::reader_spec(
            &self.cfg.reader_python,
            &self.cfg.reader_script,
            self.cfg.downlink,
            &self.cfg.presence_dir,
            &self.cfg.reader_log,
        )))
    }

    /// Start meshmud then the reader for the binding (or record why not).
    fn start_service(&self, device_id: &str, uplink: Uplink) {
        let mut svc = Service {
            device_id: device_id.to_string(),
            uplink: uplink.clone(),
            meshmud: None,
            reader: None,
            port_held_by: None,
            identify_born: false,
        };
        if !self.runnable(&uplink) || !self.cfg.meshmud_installed() {
            if !self.cfg.meshmud_installed() {
                eprintln!("111 meshtastic: no meshmud in current/bin — meshtastic absent");
            }
            *lock(&self.service) = Some(svc);
            return;
        }
        let tick = self.next_tick();
        if let Err(busy) = self.claim_for(MESHMUD_TASK, &uplink, tick) {
            eprintln!("111 meshtastic: {busy}; meshmud waits");
            svc.port_held_by = Some(busy.owner.task);
            *lock(&self.service) = Some(svc);
            return;
        }
        if let Err(e) = meshmud::write_config(&self.cfg.config_path, &uplink, self.cfg.downlink) {
            eprintln!("111 meshtastic: cannot render meshmud's config: {e:#}");
            self.release_for(MESHMUD_TASK, &uplink);
            *lock(&self.service) = Some(svc);
            return;
        }
        svc.meshmud = Some(supervise::spawn_child(meshmud::spec(
            &self.cfg.meshmud_bin,
            &self.cfg.config_path,
            &self.cfg.log,
            self.cfg.meshmud_backoff.clone(),
        )));
        svc.reader = self.spawn_reader();
        *lock(&self.service) = Some(svc);
    }

    /// Stop both children (joined), release meshmud's claims, drop shim
    /// sessions.
    async fn stop_service(&self) {
        let svc = lock(&self.service).take();
        let Some(svc) = svc else { return };
        let grace = self.cfg.stop_grace;
        let reader = async {
            if let Some(r) = &svc.reader {
                r.stop_and_wait(grace).await;
            }
        };
        let mm = async {
            if let Some(m) = &svc.meshmud {
                m.stop_and_wait(grace).await;
            }
        };
        tokio::join!(reader, mm);
        if svc.meshmud.is_some() {
            self.release_for(MESHMUD_TASK, &svc.uplink);
        }
        self.shim.close_all();
    }

    /// Refresh the cached facts and the presence snapshot (publishes
    /// `nearby` on a change).
    async fn refresh(&self) {
        let Some((api, reg)) = self.ctx() else { return };
        let binding = reg.meshtastic_binding();
        let unbound = if binding.is_none() {
            reg.identified_unbound_meshtastic()
        } else {
            None
        };
        let (board, version, usb) = match (&binding, &unbound) {
            (Some((id, b)), _) => (
                reg.label(id).or_else(|| b.hw_model.clone()),
                reg.stored_firmware(id).and_then(|f| f.version),
                b.uplink.is_usb(),
            ),
            (None, Some((id, usb))) => (
                reg.label(id),
                reg.stored_firmware(id).and_then(|f| f.version),
                *usb,
            ),
            (None, None) => (None, None, false),
        };
        let running = self.meshmud_running();
        let uplink = binding.as_ref().map(|(_, b)| b.uplink.clone());
        let cfg = self.cfg.clone();
        let observed = tokio::task::spawn_blocking(move || {
            let listening =
                crate::daemon::services::probe::listening_under(&cfg.proc_net, cfg.downlink.port())
                    .unwrap_or(false);
            let present = uplink
                .as_ref()
                .and_then(|u| uplink_present(u, &cfg.proc_net, &cfg.dev_root));
            let log = cfg
                .log
                .is_file()
                .then(|| cfg.log.to_string_lossy().into_owned());
            let identify = cfg.meshmud_installed() && cfg.reader_installed();
            (listening, present, log, identify)
        })
        .await
        .unwrap_or((false, None, None, false));
        let (listening, present, log, identify) = observed;
        // the node DB counts only while a board is bound (the file of an
        // unbound board is never read)
        let has_service = lock(&self.service).is_some();
        let presence = Arc::clone(&self.presence);
        let changed = tokio::task::spawn_blocking(move || {
            if has_service {
                presence.refresh()
            } else if presence.clear() {
                Refresh::Changed
            } else {
                Refresh::Unchanged
            }
        })
        .await
        .unwrap_or(Refresh::Unchanged);
        let bound_ports = binding
            .as_ref()
            .filter(|(_, b)| self.runnable(&b.uplink))
            .map(|(_, b)| vec![b.uplink.display()])
            .unwrap_or_default();
        {
            let mut st = lock(&self.status);
            st.downlink_listening = listening;
            st.uplink_down_since = if running && present == Some(false) {
                Some(st.uplink_down_since.unwrap_or_else(Instant::now))
            } else {
                None
            };
            st.board = board;
            st.firmware_version = version;
            st.usb_uplink = usb;
            st.log = log;
            st.identify_usb = identify && !self.cfg.phone_like && cfg!(target_os = "linux");
            st.identify_tcp = identify;
            st.bound_ports = bound_ports;
        }
        if changed == Refresh::Changed {
            self.publish_nearby(&api);
        }
    }

    /// Daemon shutdown: stop the monitor, stop and join both children,
    /// release the claims.
    pub async fn stop_and_wait(&self, grace: Duration) {
        self.stopping.store(true, Ordering::SeqCst);
        let _t = self.transition.lock().await;
        let svc = lock(&self.service).take();
        if let Some(svc) = svc {
            let reader = async {
                if let Some(r) = &svc.reader {
                    r.stop_and_wait(grace).await;
                }
            };
            let mm = async {
                if let Some(m) = &svc.meshmud {
                    m.stop_and_wait(grace).await;
                }
            };
            tokio::join!(reader, mm);
            if svc.meshmud.is_some() {
                self.release_for(MESHMUD_TASK, &svc.uplink);
            }
        }
        self.shim.close_all();
    }

    /// The bound device id and its meshmud / reader PIDs (tests, debugging).
    pub fn service_pids(&self) -> Option<(String, Option<u32>, Option<u32>)> {
        lock(&self.service).as_ref().map(|s| {
            (
                s.device_id.clone(),
                s.meshmud.as_ref().and_then(|h| h.pid()),
                s.reader.as_ref().and_then(|h| h.pid()),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_addresses_decode() {
        assert_eq!(
            proc_addr("0100007F:2AF8"),
            Some("127.0.0.1:11000".parse().unwrap())
        );
        assert_eq!(
            proc_addr("0700000A:1133"),
            Some("10.0.0.7:4403".parse().unwrap())
        );
        // ::ffff:10.0.0.7 → canonical v4
        assert_eq!(
            proc_addr("0000000000000000FFFF00000700000A:1133"),
            Some("10.0.0.7:4403".parse().unwrap())
        );
        assert_eq!(
            proc_addr("00000000000000000000000001000000:1133"),
            Some("[::1]:4403".parse().unwrap())
        );
        assert_eq!(proc_addr("xyz:1"), None);
        assert_eq!(proc_addr("0100007F"), None);
    }

    #[test]
    fn established_rows_only() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
   0: 0100007F:D431 0700000A:1133 01 00000000:00000000 00:00000000 00000000  1000        0 1\n\
   1: 0100007F:D432 0800000A:1133 06 00000000:00000000 00:00000000 00000000  1000        0 2\n";
        assert!(established_in_table(
            table,
            "10.0.0.7:4403".parse().unwrap()
        ));
        assert!(
            !established_in_table(table, "10.0.0.8:4403".parse().unwrap()),
            "TIME_WAIT"
        );
        assert!(!established_in_table(
            table,
            "10.0.0.7:4404".parse().unwrap()
        ));
    }

    #[test]
    fn uplink_presence() {
        let dir = tempfile::tempdir().unwrap();
        // a serial node that does not exist is absent
        let gone = Uplink::serial("/dev/serial/by-id/usb-not-plugged-in-111").unwrap();
        assert_eq!(
            uplink_present(&gone, dir.path(), Path::new("/")),
            Some(false)
        );
        // … and present once it exists under the dev root
        let fake = dir.path().join("dev/serial/by-id");
        std::fs::create_dir_all(&fake).unwrap();
        std::fs::write(fake.join("usb-not-plugged-in-111"), "").unwrap();
        assert_eq!(uplink_present(&gone, dir.path(), dir.path()), Some(true));
        // a TCP uplink with unreadable procfs or a host name is unknown
        let wifi = Uplink::tcp("10.0.0.7:4403").unwrap();
        assert_eq!(uplink_present(&wifi, dir.path(), Path::new("/")), None);
        let named = Uplink::tcp("board.lan:4403").unwrap();
        assert_eq!(
            uplink_present(&named, Path::new("/proc/net"), Path::new("/")),
            None
        );
        std::fs::write(
            dir.path().join("tcp"),
            "header\n 0: 0100007F:D431 0700000A:1133 01 x\n",
        )
        .unwrap();
        assert_eq!(
            uplink_present(&wifi, dir.path(), Path::new("/")),
            Some(true)
        );
    }
}
