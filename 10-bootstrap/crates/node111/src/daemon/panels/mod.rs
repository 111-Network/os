//! Hosted panels (Layer 5 §13; D19 d, amends D15): an adopted project's own
//! web UI, framed by the dashboard on the service's LIVE page, each on its own
//! origin. **Only registry-declared panels are ever framed.** v1 = static
//! bundles plus built-in shims; the generic reverse proxy is reserved (P7).
//!
//! - [`manifest`] — `assets/panels/<id>.toml` (embedded), validated; unknown
//!   keys or an invalid file ⇒ not registered (logged).
//! - [`ticket`] — ticket → `p111_<id>` cookie, `state/panel-sessions.json`,
//!   bindings and revocation.
//! - [`guard`] — Host / `Sec-Fetch-Site` / Origin / `OPTIONS` guards.
//! - [`headers`] — the response policy.
//! - [`static_files`] (`static.rs`) — bundle serving with the SPA fallback.
//! - [`listener`] — the per-panel app, binding and the serve loop.
//! - [`shim`] — built-in shims (`meshtastic-http` is registered by the
//!   Meshtastic module, task M1).
//!
//! [`PanelHost`] ties them together: registered panels, their **slots** in
//! `state/panels.json` (from [`PANEL_PORTS`], stable across restarts), the
//! listeners of **enabled** panels, health, and the `PanelInfo` facts the
//! services registry attaches to `ServiceRecord.panel` ([`provider`]).
//!
//! **Enabled** (v1 reading, fail closed): the panel is registered, its bundle
//! directory exists, and its service is set up — the services registry's
//! published state for the panel's id is anything but `unset` (the contract's
//! "unknown panel, or its service is not set up" ⇒ `404` on mint). Only enabled
//! panels hold a listener; a panel that is not enabled loses its sessions and
//! tickets (§13 "revoked on … panel disable").

pub mod guard;
pub mod headers;
pub mod listener;
pub mod manifest;
pub mod shim;
#[path = "static.rs"]
pub mod static_files;
pub mod ticket;

pub use shim::{Shim, ShimError, ShimRegistry, ShimRequest, ShimResponse, ShimSession};

use crate::daemon::dashboard::ApiState;
use crate::daemon::services::{PanelHealth, PanelInfo, PanelState, ServiceId, ServiceState};
use crate::daemon::DaemonLayout;
use guard::HostPolicy;
use manifest::{AuthMode, HealthSpec, Manifest};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ticket::{Binding, PanelSessionRec, PeerClass, SessionStore, Tickets};

/// Panel listener ports (§13: one per *enabled* panel, slot stable in
/// `state/panels.json`).
pub const PANEL_PORTS: std::ops::RangeInclusive<u16> = 11190..=11229;

/// `state/panels.json` (slot assignments; no secrets, 0644).
pub const SLOTS_FILE: &str = "panels.json";

/// The reconcile cadence (same 2 s tick as the services registry).
pub const RECONCILE_EVERY: Duration = Duration::from_secs(2);

/// A failed listener bind is retried after this.
pub const BIND_RETRY: Duration = Duration::from_secs(30);

/// Where panels live and how they are served.
#[derive(Debug, Clone)]
pub struct PanelsConfig {
    /// `<root>/state` (slots, sessions).
    pub state_dir: PathBuf,
    /// Bundles resolve against this: the release payload, `<root>/current`
    /// (`bundle = "panels/meshtastic-web"` ⇒ `<root>/current/panels/meshtastic-web`).
    pub bundle_root: PathBuf,
    /// The slot block ([`PANEL_PORTS`]; tests narrow it).
    pub ports: RangeInclusive<u16>,
    pub ticket_ttl: Duration,
    pub timeouts: listener::Timeouts,
    /// `/proc/net` (the LISTEN health check reads `tcp`/`tcp6` here).
    pub proc_net: PathBuf,
    /// Persist slots and sessions under `state_dir` (tests may keep them in
    /// memory).
    pub persist: bool,
    /// Bind loopback even when LAN sharing is on (tests exercise LAN host
    /// rules without opening a LAN socket). Never set by the daemon; it can
    /// only narrow the bind.
    pub force_loopback: bool,
}

impl PanelsConfig {
    /// The daemon's configuration for `layout`.
    pub fn new(layout: &DaemonLayout) -> Self {
        PanelsConfig {
            state_dir: layout.state_dir(),
            bundle_root: layout.current_release(),
            ports: PANEL_PORTS,
            ticket_ttl: ticket::TICKET_TTL,
            timeouts: listener::Timeouts::default(),
            proc_net: PathBuf::from("/proc/net"),
            persist: true,
            force_loopback: false,
        }
    }
}

/// A registered panel.
pub struct Panel {
    pub manifest: Manifest,
    /// Its slot.
    pub port: u16,
    /// `<bundle_root>/<manifest.bundle>`.
    pub bundle_dir: PathBuf,
    /// The built-in shim named by the manifest.
    pub shim: Option<Arc<dyn Shim>>,
}

impl Panel {
    pub fn id(&self) -> ServiceId {
        self.manifest.id
    }

    pub fn loopback_only(&self) -> bool {
        self.manifest.auth == AuthMode::LoopbackOnly
    }
}

impl std::fmt::Debug for Panel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Panel")
            .field("id", &self.id())
            .field("port", &self.port)
            .field("bundle_dir", &self.bundle_dir)
            .field("shim", &self.manifest.shim)
            .finish()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SlotsFile {
    schema: u32,
    #[serde(default)]
    slots: BTreeMap<String, u16>,
}

/// Keep valid existing slots (id-shaped key, port in the block, first holder of
/// a port wins), then give every id without one the lowest free port. Unknown
/// ids keep their slots (a panel that comes back gets its port back). Returns
/// the map and whether it changed.
pub fn assign_slots(
    existing: &BTreeMap<String, u16>,
    ids: &[ServiceId],
    block: &RangeInclusive<u16>,
) -> (BTreeMap<String, u16>, bool) {
    let mut out = BTreeMap::new();
    let mut taken = std::collections::HashSet::new();
    for (id, port) in existing {
        let id_ok = !id.is_empty()
            && id.len() <= 32
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if id_ok && block.contains(port) && taken.insert(*port) {
            out.insert(id.clone(), *port);
        }
    }
    for id in ids {
        if out.contains_key(id.as_str()) {
            continue;
        }
        if let Some(port) = block.clone().find(|p| !taken.contains(p)) {
            taken.insert(port);
            out.insert(id.as_str().to_string(), port);
        }
    }
    let changed = out != *existing;
    (out, changed)
}

fn read_slots(path: &std::path::Path) -> BTreeMap<String, u16> {
    match std::fs::read(path) {
        Ok(bytes) if bytes.len() <= 64 * 1024 => serde_json::from_slice::<SlotsFile>(&bytes)
            .map(|f| f.slots)
            .unwrap_or_else(|e| {
                eprintln!(
                    "111 daemon: panels: {} is not valid ({e}); slots reassigned",
                    path.display()
                );
                BTreeMap::new()
            }),
        Ok(_) => BTreeMap::new(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(e) => {
            eprintln!("111 daemon: panels: cannot read {}: {e}", path.display());
            BTreeMap::new()
        }
    }
}

/// A running (or retrying) listener set of one enabled panel.
struct Live {
    tasks: Vec<tokio::task::JoinHandle<()>>,
    addrs: Vec<SocketAddr>,
    /// Bind failed: retry at this instant.
    retry_at: Option<Instant>,
}

impl Drop for Live {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

/// 2-observation debounce of a panel's health.
#[derive(Debug, Clone, Copy)]
struct HealthTrack {
    published: PanelHealth,
    pending: Option<PanelHealth>,
}

impl HealthTrack {
    fn observe(&mut self, h: PanelHealth) {
        if h == self.published {
            self.pending = None;
        } else if self.pending == Some(h) {
            self.published = h;
            self.pending = None;
        } else {
            self.pending = Some(h);
        }
    }
}

#[derive(Default)]
struct HostState {
    enabled: std::collections::HashSet<ServiceId>,
    live: HashMap<ServiceId, Live>,
    health: HashMap<ServiceId, HealthTrack>,
    dropped: HashMap<ServiceId, u64>,
    last_published: Option<Vec<PanelInfo>>,
}

/// Why a ticket was not minted.
#[derive(Debug)]
pub enum MintError {
    /// No such registered panel, or it is not enabled (service not set up).
    NotFound,
    /// A loopback-only panel asked for from a non-loopback dashboard host.
    LoopbackOnly,
    Random(ticket::RandomError),
}

/// The panels of this daemon lifetime (one per daemon; `Arc` in `ApiState`
/// ext).
pub struct PanelHost {
    cfg: PanelsConfig,
    panels: Vec<Arc<Panel>>,
    pub tickets: Tickets,
    pub sessions: SessionStore,
    state: Mutex<HostState>,
}

impl std::fmt::Debug for PanelHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PanelHost")
            .field("panels", &self.panels)
            .finish()
    }
}

impl PanelHost {
    /// Register `manifests` (`(file name, bytes)`), assign slots, open the
    /// session table. `lan` is this lifetime's LAN sharing (LAN-off revokes
    /// sessions minted while it was on). Invalid declarations are logged and
    /// skipped.
    pub fn new(
        cfg: PanelsConfig,
        manifests: Vec<(String, Vec<u8>)>,
        shims: &ShimRegistry,
        lan: bool,
    ) -> Self {
        let mut valid: Vec<Manifest> = Vec::new();
        for (file, bytes) in &manifests {
            match manifest::parse(file, bytes, shims) {
                Ok(m) if valid.iter().any(|v| v.id == m.id) => {
                    eprintln!("111 daemon: panel {file}: not registered: duplicate id");
                }
                Ok(m) => valid.push(m),
                Err(e) => eprintln!("111 daemon: panel {file}: not registered: {e}"),
            }
        }
        let slots_path = cfg.state_dir.join(SLOTS_FILE);
        let existing = if cfg.persist {
            read_slots(&slots_path)
        } else {
            BTreeMap::new()
        };
        let ids: Vec<ServiceId> = valid.iter().map(|m| m.id).collect();
        let (slots, changed) = assign_slots(&existing, &ids, &cfg.ports);
        if changed && cfg.persist {
            let file = SlotsFile {
                schema: 1,
                slots: slots.clone(),
            };
            let res = serde_json::to_vec_pretty(&file)
                .map_err(anyhow::Error::from)
                .and_then(|b| crate::util::fs::atomic_write(&slots_path, &b, 0o644));
            if let Err(e) = res {
                eprintln!(
                    "111 daemon: panels: cannot write {}: {e:#} (slots kept in memory)",
                    slots_path.display()
                );
            }
        }
        let mut panels = Vec::new();
        for m in valid {
            let Some(port) = slots.get(m.id.as_str()).copied() else {
                eprintln!(
                    "111 daemon: panel {}: not registered: no free port in {}..={}",
                    m.id.as_str(),
                    cfg.ports.start(),
                    cfg.ports.end()
                );
                continue;
            };
            let shim = m.shim.as_deref().and_then(|n| shims.get(n));
            panels.push(Arc::new(Panel {
                bundle_dir: cfg.bundle_root.join(&m.bundle),
                port,
                shim,
                manifest: m,
            }));
        }
        let sessions_path = cfg
            .persist
            .then(|| cfg.state_dir.join(ticket::SESSIONS_FILE));
        PanelHost {
            tickets: Tickets::new(cfg.ticket_ttl),
            sessions: SessionStore::open(sessions_path, lan),
            panels,
            state: Mutex::new(HostState::default()),
            cfg,
        }
    }

    /// The daemon's host: the embedded declarations.
    pub fn load(cfg: PanelsConfig, shims: &ShimRegistry, lan: bool) -> Self {
        Self::new(cfg, manifest::embedded(), shims, lan)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HostState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The registered panel `id`.
    pub fn panel(&self, id: ServiceId) -> Option<Arc<Panel>> {
        self.panels.iter().find(|p| p.id() == id).cloned()
    }

    /// Every registered panel.
    pub fn registered(&self) -> Vec<Arc<Panel>> {
        self.panels.clone()
    }

    pub fn is_enabled(&self, id: ServiceId) -> bool {
        self.lock().enabled.contains(&id)
    }

    /// Addresses `id`'s listener is bound to (empty when not listening).
    pub fn bound_addrs(&self, id: ServiceId) -> Vec<SocketAddr> {
        self.lock()
            .live
            .get(&id)
            .map(|l| l.addrs.clone())
            .unwrap_or_default()
    }

    /// Shim seam (§14): the shim reports its drop-oldest count for
    /// `panel.state.dropped`.
    pub fn report_dropped(&self, id: ServiceId, total: u64) {
        self.lock().dropped.insert(id, total);
    }

    /// Session revocations as they happen (shims drop per-session upstream
    /// state, §14).
    pub fn revocations(&self) -> tokio::sync::broadcast::Receiver<ticket::Revoked> {
        self.sessions.subscribe()
    }

    /// The `Panel` facts of every registered panel (contract `Panel`; the
    /// services registry attaches each to its service's record).
    pub fn panels(&self) -> Vec<PanelInfo> {
        let st = self.lock();
        self.panels.iter().map(|p| info(p, &st)).collect()
    }

    /// The valid session of `id` behind any of `cookies` for `peer`. A session
    /// whose minting session is gone is revoked here.
    pub fn session_for(
        &self,
        api: &ApiState,
        id: ServiceId,
        cookies: &[String],
        peer: PeerClass,
    ) -> Option<PanelSessionRec> {
        if cookies.is_empty() {
            return None;
        }
        let rec = self.sessions.find(id, cookies, peer)?;
        if ticket::binding_alive(api, &rec.binding) {
            Some(rec)
        } else {
            let gone = rec.id.clone();
            self.sessions.revoke(|r| r.id == gone);
            None
        }
    }

    /// Mint a ticket for `id` (registered and enabled) bound to `binding` and
    /// `peer`. `dashboard_host` is the host the dashboard was reached on (the
    /// ticket URL's host); a loopback-only panel refuses non-loopback hosts.
    pub fn mint(
        &self,
        id: ServiceId,
        dashboard_host: &str,
        peer: PeerClass,
        binding: Binding,
    ) -> Result<(Arc<Panel>, String), MintError> {
        let panel = self.panel(id).ok_or(MintError::NotFound)?;
        if !self.is_enabled(id) {
            return Err(MintError::NotFound);
        }
        if panel.loopback_only()
            && !guard::host_ok(
                dashboard_host,
                &HostPolicy {
                    lan: false,
                    own: &[],
                    loopback_only: true,
                },
            )
        {
            return Err(MintError::LoopbackOnly);
        }
        let t = self
            .tickets
            .mint(id, peer, binding)
            .map_err(MintError::Random)?;
        Ok((panel, t))
    }

    /// Origins of the enabled, listening panels on the dashboard host `host`
    /// (parsed, lowercased) — the dashboard CSP's `frame-src` (§13 "Dashboard
    /// side").
    pub fn frame_origins(&self, host: &str, lan: bool, own: &[IpAddr]) -> Vec<String> {
        let st = self.lock();
        self.panels
            .iter()
            .filter(|p| st.live.get(&p.id()).is_some_and(|l| !l.addrs.is_empty()))
            .filter(|p| {
                guard::host_ok(
                    host,
                    &HostPolicy {
                        lan,
                        own,
                        loopback_only: p.loopback_only(),
                    },
                )
            })
            .map(|p| format!("http://{}:{}", guard::url_host(host), p.port))
            .collect()
    }

    /// Revoke sessions (and drop tickets) whose minting session is gone
    /// (`111 dashboard rotate`, logout).
    pub fn sweep(&self, api: &ApiState) -> usize {
        self.tickets.retain_bound(|b| ticket::binding_alive(api, b));
        self.sessions
            .revoke(|r| !ticket::binding_alive(api, &r.binding))
    }

    /// Bring listeners in line with `setup` (the services registry's view:
    /// `Some(f)` once it has published, `None` before — then nothing changes),
    /// sweep dead sessions, refresh health, and publish a `panels` frame when
    /// the facts changed. Returns whether they changed.
    pub async fn reconcile(
        self: &Arc<Self>,
        api: &Arc<ApiState>,
        setup: Option<&(dyn Fn(ServiceId) -> bool + Sync)>,
    ) -> bool {
        if let Some(setup) = setup {
            let now = Instant::now();
            let mut to_start = Vec::new();
            {
                let mut st = self.lock();
                for p in &self.panels {
                    let want = setup(p.id()) && p.bundle_dir.is_dir();
                    if want {
                        st.enabled.insert(p.id());
                        let retry = match st.live.get(&p.id()) {
                            None => true,
                            Some(l) => l.retry_at.is_some_and(|t| now >= t),
                        };
                        if retry {
                            to_start.push(Arc::clone(p));
                        }
                    } else {
                        st.enabled.remove(&p.id());
                        st.live.remove(&p.id()); // aborts its tasks
                    }
                }
            }
            // §13 panel disable: a panel that is not enabled keeps no session
            for p in &self.panels {
                if !self.is_enabled(p.id()) {
                    self.tickets.drop_panel(p.id());
                    let id = p.id().as_str();
                    self.sessions.revoke(|r| r.panel == id);
                }
            }
            for p in to_start {
                let live = self.start_listener(api, &p).await;
                let mut st = self.lock();
                if st.enabled.contains(&p.id()) {
                    st.live.insert(p.id(), live);
                }
            }
        }
        self.sweep(api);
        let observed: Vec<(ServiceId, PanelHealth)> = self
            .panels
            .iter()
            .map(|p| (p.id(), self.observe_health(p)))
            .collect();
        let snapshot = {
            let mut st = self.lock();
            for (id, h) in observed {
                st.health
                    .entry(id)
                    .or_insert(HealthTrack {
                        published: PanelHealth::Unknown,
                        pending: None,
                    })
                    .observe(h);
            }
            let snap: Vec<PanelInfo> = self.panels.iter().map(|p| info(p, &st)).collect();
            if st.last_published.as_ref() == Some(&snap) {
                return false;
            }
            st.last_published = Some(snap.clone());
            snap
        };
        api.events
            .publish("panels", &serde_json::json!({ "panels": snapshot }));
        true
    }

    fn observe_health(&self, p: &Panel) -> PanelHealth {
        match &p.manifest.health {
            Some(HealthSpec::Listen(port)) => {
                match crate::daemon::services::probe::listening_under(&self.cfg.proc_net, *port) {
                    Some(true) => PanelHealth::Up,
                    Some(false) => PanelHealth::Down,
                    None => PanelHealth::Unknown,
                }
            }
            Some(HealthSpec::Url(_)) | None => PanelHealth::Unknown,
        }
    }

    async fn start_listener(self: &Arc<Self>, api: &Arc<ApiState>, p: &Arc<Panel>) -> Live {
        let lan = api.lan_enabled && !self.cfg.force_loopback;
        let listeners = listener::bind(p.port, lan, p.manifest.auth).await;
        if listeners.is_empty() {
            return Live {
                tasks: Vec::new(),
                addrs: Vec::new(),
                retry_at: Some(Instant::now() + BIND_RETRY),
            };
        }
        let app = listener::app(listener::PanelCtx {
            api: Arc::clone(api),
            host: Arc::clone(self),
            panel: Arc::clone(p),
        });
        let mut tasks = Vec::new();
        let mut addrs = Vec::new();
        for l in listeners {
            if let Ok(a) = l.local_addr() {
                addrs.push(a);
            }
            tasks.push(tokio::spawn(listener::serve(
                l,
                app.clone(),
                self.cfg.timeouts,
            )));
        }
        eprintln!(
            "111 daemon: panel {} listening on {}",
            p.id().as_str(),
            addrs
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" · ")
        );
        Live {
            tasks,
            addrs,
            retry_at: None,
        }
    }
}

fn info(p: &Panel, st: &HostState) -> PanelInfo {
    let m = &p.manifest;
    PanelInfo {
        id: m.id,
        port: p.port,
        entry: m.entry.clone(),
        needs_secure_context: m.needs_secure_context,
        allow: m.allow.clone(),
        optional_hosts: m.optional_hosts.clone(),
        state: PanelState {
            enabled: st.enabled.contains(&m.id),
            listening: st.live.get(&m.id).is_some_and(|l| !l.addrs.is_empty()),
            health: st
                .health
                .get(&m.id)
                .map(|h| h.published)
                .unwrap_or(PanelHealth::Unknown),
            dropped: st.dropped.get(&m.id).copied(),
        },
        title: Some(m.title.clone()),
        secure_note: m.secure_note.clone(),
    }
}

/// The services-registry provider (`Sources.panels`): a cached, non-blocking
/// snapshot per tick.
pub fn provider(host: &Arc<PanelHost>) -> crate::daemon::services::probe::Provider<Vec<PanelInfo>> {
    let host = Arc::clone(host);
    Arc::new(move || Some(host.panels()))
}

/// The daemon task: every [`RECONCILE_EVERY`], read the services registry
/// (`Arc<Registry>` in `api` ext) and reconcile. A panel's service counts as
/// set up when its published state is not `unset`.
pub fn spawn(api: Arc<ApiState>, host: Arc<PanelHost>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(RECONCILE_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let snapshot = api
                .ext::<Arc<crate::daemon::services::Registry>>()
                .map(|r| r.snapshot())
                .unwrap_or_default();
            if snapshot.is_empty() {
                host.reconcile(&api, None).await;
                continue;
            }
            let setup = move |id: ServiceId| {
                snapshot
                    .iter()
                    .any(|r| r.id == id && r.state != ServiceState::Unset)
            };
            host.reconcile(&api, Some(&setup)).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_block_is_the_spec_block() {
        assert_eq!(super::PANEL_PORTS.clone().count(), 40);
        assert!(!super::PANEL_PORTS.contains(&crate::daemon::dashboard::DEFAULT_PORT));
    }

    #[test]
    fn slots_are_stable_and_validated() {
        let block = 11190..=11192;
        let (first, changed) = assign_slots(
            &BTreeMap::new(),
            &[ServiceId::Meshtastic, ServiceId::Fabric],
            &block,
        );
        assert!(changed);
        assert_eq!(first["meshtastic"], 11190);
        assert_eq!(first["fabric"], 11191);
        // stable: same input, no change, regardless of id order
        let (again, changed) =
            assign_slots(&first, &[ServiceId::Fabric, ServiceId::Meshtastic], &block);
        assert!(!changed);
        assert_eq!(again, first);
        // a departed panel keeps its slot; a newcomer takes the next free one
        let (more, _) = assign_slots(&first, &[ServiceId::Meshtastic, ServiceId::Kb], &block);
        assert_eq!(more["fabric"], 11191);
        assert_eq!(more["kb"], 11192);
        // junk entries are dropped: out of block, duplicate port, bad id
        let mut junk = BTreeMap::new();
        junk.insert("meshtastic".to_string(), 80u16);
        junk.insert("a".to_string(), 11190);
        junk.insert("b".to_string(), 11190);
        junk.insert("Bad Id".to_string(), 11191);
        let (fixed, changed) = assign_slots(&junk, &[ServiceId::Meshtastic], &block);
        assert!(changed);
        assert_eq!(fixed.get("a"), Some(&11190));
        assert!(!fixed.contains_key("b") && !fixed.contains_key("Bad Id"));
        assert_eq!(fixed["meshtastic"], 11191);
        // block exhausted: no slot
        let (none, _) = assign_slots(&fixed, &[ServiceId::Kb, ServiceId::Agent], &(11190..=11192));
        assert_eq!(none.len(), 3);
    }

    #[test]
    fn health_debounce_needs_two_ticks() {
        let mut h = HealthTrack {
            published: PanelHealth::Unknown,
            pending: None,
        };
        h.observe(PanelHealth::Up);
        assert_eq!(h.published, PanelHealth::Unknown);
        h.observe(PanelHealth::Up);
        assert_eq!(h.published, PanelHealth::Up);
        h.observe(PanelHealth::Down);
        h.observe(PanelHealth::Up);
        assert_eq!(h.published, PanelHealth::Up);
        h.observe(PanelHealth::Down);
        h.observe(PanelHealth::Down);
        assert_eq!(h.published, PanelHealth::Down);
    }
}
