//! `111 daemon` — the node kernel: tokio supervision tree (systemd/runit are thin
//! Restart=always wrappers, never the supervisor of record), watchdog heartbeat,
//! dashboard, updater, KB server, conditional rnsd child.
//!
//! Layer 6 (`os/6-transport/README.md` §2, §4, §5, §11; task C1) is wired by
//! [`start_layer6`]: the bridge core with the compiled-in Meshtastic adapter
//! registered (enabled in the core only while its client runs, i.e. while
//! service `meshtastic` is LIVE — B3's rule), the optional components and
//! their runner, the transport spine's routes, all in `ApiState` ext. Nothing
//! of it spawns a task, timer or listener: the bridge has no recv task until
//! an adapter is enabled, and the daemon's existing 2 s tick drives
//! [`Layer6::tick_once`] (`Bridge::tick_once` returns at once with nothing
//! enabled; AGENTS.md: the Raspberry Pi is the floor).
//!
//! W1 (the open wiring of C1): the LXMF helper's seam is registered
//! ([`register_lxmf`]: before its child starts, the helper's Reticulum client
//! config is written with rnsd's `rpc_key` and its run arguments computed;
//! once it runs, `LxmfAdapter` attaches to its piped stdio and is enabled in
//! the core), and the core's Nearby sink feeds `GET /nearby` (the
//! `reticulum` sources, `dashboard::api_v1::nearby::BridgeNearby`).
//! Registration spawns nothing either: with the component disabled no task,
//! child, listener or file of the helper exists.
//!
//! W3 (§7, §14): the Delta Chat network's two components are registered the
//! same way ([`register_deltachat`]: Madmail's prepare hook writes the relay's
//! loopback-only first config; the core's settles and renders the relay for
//! its bind and requires its IMAP listener; the factory attaches
//! `DeltaChatAdapter` to the running core), and [`Layer6::resume`] runs
//! `Components::resume_enabled_with`'s one dependency-ordered loop. Nothing of
//! either component exists while both are off.

pub mod bridge;
pub mod components;
pub mod dashboard;
pub mod devices;
pub mod fabric;
pub mod health;
pub mod intelligence;
pub mod kb;
pub mod meshtastic;
pub mod panels;
pub mod ports;
pub mod services;
pub mod supervise;
pub mod transport;
pub mod updater;
pub mod watchdog;

use crate::install::categorize::{categorize, Classification, Refusal};
use crate::install::probe::{self, Env, Facts};
use dashboard::{ApiState, DashboardState, ErrorState, OfferState, SharedState};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

/// Runtime layout resolved once at daemon start.
#[derive(Debug, Clone)]
pub struct DaemonLayout {
    /// Node root: $NODE111_ROOT or /var/lib/111.
    pub root: PathBuf,
    /// Config dir: /etc/111 (host) — under Termux a sibling of root.
    pub etc: PathBuf,
}

impl DaemonLayout {
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }
    pub fn identity_dir(&self) -> PathBuf {
        self.root.join("identity")
    }
    pub fn current_release(&self) -> PathBuf {
        self.root.join("current")
    }
}

/// Parse `/etc/111/env`-style KEY=VALUE files (comments + blanks skipped).
pub fn parse_env_file(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Load node config from `<etc>/env` (missing file = empty config, not an error).
pub fn load_env(etc: &Path) -> anyhow::Result<Vec<(String, String)>> {
    match std::fs::read_to_string(etc.join("env")) {
        Ok(text) => Ok(parse_env_file(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// Category re-probe (daemon start + udev trigger, spec §Detection): collect
/// facts, reclassify, persist facts.json (best-effort write — probing must not
/// kill the daemon).
pub fn reprobe(env: &dyn Env, layout: &DaemonLayout) -> (Facts, Result<Classification, Refusal>) {
    let facts = probe::collect(env);
    let class = categorize(&facts);
    if let Ok(class) = &class {
        let _ = probe::write_facts_json(&layout.state_dir().join("facts.json"), &facts, class);
    }
    (facts, class)
}

/// SIGTERM → SIGKILL grace for each supervised child on daemon shutdown (and
/// for a dashboard per-service restart).
pub const CHILD_STOP_GRACE: Duration = Duration::from_secs(5);

/// Release version currently linked (basename of `current`).
fn current_version(layout: &DaemonLayout) -> String {
    crate::update_helper::current_version(layout).unwrap_or_else(|| "unknown".into())
}

/// Read the offer stub (state/offer.json) into dashboard shape.
fn read_offer(layout: &DaemonLayout) -> Option<OfferState> {
    let text = std::fs::read_to_string(layout.state_dir().join("offer.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(OfferState {
        tier: v["tier"].as_str()?.to_string(),
        state: v["state"].as_str()?.to_string(),
    })
}

/// Probation monitor (daemon side, spec §Updates): while a journal is pending,
/// sustained-green (60s) ⇒ drop `health-confirmed.json`; sustained-red ⇒
/// drop `revert-request.json` (READY-but-503). The root helper acts on both.
async fn probation_monitor(layout: DaemonLayout, state: SharedState) {
    let mut green_since: Option<std::time::Instant> = None;
    let mut red_since: Option<std::time::Instant> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    loop {
        tick.tick().await;
        let journal = match crate::probation::Journal::read(&layout.state_dir().join("update.json"))
        {
            Ok(Some(j)) if j.pending => j,
            _ => {
                green_since = None;
                red_since = None;
                continue;
            }
        };
        let _ = journal;
        let green = health::evaluate(&state.read().unwrap().components) == health::HealthStatus::Ok;
        if green {
            red_since = None;
            let since = green_since.get_or_insert_with(std::time::Instant::now);
            if since.elapsed() >= Duration::from_secs(60) {
                let body = serde_json::json!({"schema": 1, "at": rfc3339_now()});
                let _ = crate::util::fs::atomic_write(
                    &layout.state_dir().join("health-confirmed.json"),
                    body.to_string().as_bytes(),
                    0o644,
                );
                green_since = None;
            }
        } else {
            green_since = None;
            let since = red_since.get_or_insert_with(std::time::Instant::now);
            if since.elapsed() >= Duration::from_secs(60) {
                let body = serde_json::json!({"schema": 1, "reason": "ready-but-unhealthy", "at": rfc3339_now()});
                let _ = crate::util::fs::atomic_write(
                    &layout.state_dir().join("revert-request.json"),
                    body.to_string().as_bytes(),
                    0o644,
                );
                red_since = None;
            }
        }
    }
}

fn rfc3339_now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}

// ---------------------------------------------------------------- Layer 6 wiring

/// §3 domain tag of the 32-byte key the bridge derives conversation ids under (the
/// Meshtastic channel ids: a keyed BLAKE2b over the name and a keyed PSK digest).
pub const BRIDGE_CONV_TAG: &[u8] = b"111/bridge-conv/v1";

/// The bridge's conversation-id key from the identity's private keyset bytes (`sign seed
/// ‖ X25519 scalar`), derived the way the Mode S store secret is
/// (`fabric::sealer::store_secret_from`): keyed BLAKE2b-256 with the X25519 half as the
/// key over [`BRIDGE_CONV_TAG`]. Nothing new is persisted; the identity is the only
/// secret. `None` only if the MAC refuses its key (it never does for 32 bytes).
pub fn bridge_conv_key(private_keyset: &[u8; 64]) -> Option<zeroize::Zeroizing<[u8; 32]>> {
    use blake2::digest::consts::U32;
    use blake2::digest::{FixedOutput, KeyInit, Update};
    use zeroize::Zeroize;
    let mut mac =
        <blake2::Blake2bMac<U32> as KeyInit>::new_from_slice(&private_keyset[32..64]).ok()?;
    Update::update(&mut mac, BRIDGE_CONV_TAG);
    let mut out = mac.finalize_fixed();
    let mut key = zeroize::Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&out);
    out.zeroize();
    Some(key)
}

/// What [`start_layer6`] wires.
pub struct Layer6Parts {
    pub api: Arc<ApiState>,
    pub fabric: Arc<fabric::FabricHandle>,
    pub ports: Arc<ports::PortTable>,
    /// The daemon tick stamped on port claims (never wall clock).
    pub tick: Arc<dyn Fn() -> u64 + Send + Sync>,
    /// The transport spine and its `rnstatus` facts cache (`None`: tests without one).
    pub transport: Option<(
        Arc<transport::posture::Spine>,
        Arc<transport::rnstatus::FactsCache>,
    )>,
    /// The Meshtastic bridge client (`None` without an identity: no conversation key).
    pub mesh_client: Option<Arc<meshtastic::client::MeshtasticAdapter>>,
    /// `python3` and a venv's pip for the components (blocking).
    pub cmd: Arc<dyn crate::util::run::Runner>,
}

/// The Layer 6 parts of a running daemon (module doc).
pub struct Layer6 {
    pub bridge: Arc<bridge::Bridge>,
    pub components: components::Components,
    pub mesh_client: Option<Arc<meshtastic::client::MeshtasticAdapter>>,
}

/// Build the bridge core, the optional components and the transport routes' state, and
/// insert them into `api` ext (`Arc<Bridge>`, `Components`, `Arc<TransportApi>`,
/// `Arc<ComponentAdapters>`). Spawns nothing (module doc); the supervised children the
/// components depend on are read from `api`'s `Supervised` ext when they are asked for.
pub fn start_layer6(p: Layer6Parts) -> Layer6 {
    use dashboard::api_v1::{bridge::ComponentAdapters, transport::TransportApi};
    let root = p.api.layout.root.clone();
    let deps = {
        let api = Arc::downgrade(&p.api);
        let child_of = move |sid: services::ServiceId| {
            api.upgrade()?
                .ext::<Arc<services::Supervised>>()?
                .get(sid)
                .cloned()
        };
        components::ServiceDeps {
            root: root.clone(),
            child_of: Arc::new(child_of),
        }
    };
    let components = components::Components::new(
        components::ComponentsConfig::new(&root),
        components::manifest::load_embedded(),
        components::Wiring {
            cmd: p.cmd,
            ports: Arc::clone(&p.ports),
            deps: Arc::new(deps),
            tick: p.tick,
        },
    );
    let bridge = bridge::Bridge::new(bridge::BridgeConfig::new(&root), p.fabric, &p.api);
    bridge.set_components(Arc::new(components.clone()));
    if let Some((spine, facts)) = p.transport {
        // an rnsd restart stops and resumes the components that depend on it (§6)
        spine.set_restart_hooks(Arc::new(components.clone()));
        p.api.insert_ext(Arc::new(TransportApi { spine, facts }));
    }
    wire_bridge_nearby(&p.api, &bridge);
    p.api.insert_ext(Arc::clone(&bridge));
    p.api.insert_ext(components.clone());
    let adapters = Arc::new(ComponentAdapters::default());
    register_lxmf(&adapters, &p.api, &bridge);
    register_deltachat(&adapters, &p.api, &bridge);
    p.api.insert_ext(adapters);
    Layer6 {
        bridge,
        components,
        mesh_client: p.mesh_client,
    }
}

impl Layer6 {
    /// One pass, from the daemon's existing tick (no timer of its own): the Meshtastic
    /// adapter is enabled in the bridge core while its client runs (service `meshtastic`
    /// LIVE) and the user has not switched it off, disabled otherwise; then
    /// `Bridge::tick_once`, which returns at once while nothing is enabled.
    pub async fn tick_once(&self) {
        if let Some(c) = &self.mesh_client {
            let net = bridge::NETWORK_MESHTASTIC;
            let want = c.running() && self.bridge.adapter_switch(net) != Some(false);
            let have = self.bridge.adapter(net).is_some();
            if want && !have {
                let adapter: Arc<dyn bridge::Adapter> = Arc::clone(c) as Arc<dyn bridge::Adapter>;
                if let Err(e) = self.bridge.enable(adapter).await {
                    eprintln!("111 bridge: meshtastic: {e}");
                }
            } else if !want && have {
                self.bridge.disable(net).await;
            }
        }
        self.bridge.tick_once().await;
    }

    /// Daemon start, after the supervised children exist: resume the components whose
    /// switch is on, and attach the adapters registered for them. The loop is
    /// `Components::resume_enabled_with`'s (W3: one place for the order — a dependency
    /// before its dependants — and the rule that the recorded acknowledgement must still
    /// match), with each enable going through `api_v1::bridge::enable_component` (the
    /// route's path) so a component's prepare hook runs before its child starts; a
    /// failure is logged and leaves that component off (its switch stays as it was).
    pub async fn resume(&self, api: &ApiState) {
        let comps = &self.components;
        let order = comps
            .resume_enabled_with(|id| async move {
                dashboard::api_v1::bridge::enable_component(api, comps, &id, None).await
            })
            .await;
        for id in order {
            if comps.stdio(&id).await.is_some() {
                dashboard::api_v1::bridge::attach_adapter(api, comps, &id).await;
            }
        }
    }

    /// Daemon stop: the adapters, then the components' children.
    pub async fn shutdown(&self) {
        for net in self.bridge.enabled() {
            self.bridge.disable(&net).await;
        }
        self.components.shutdown().await;
    }
}

/// The bridge core's Nearby sink (W1): the adapters' Nearby items (LXMF heards, the
/// `reticulum` sources) are merged into a `BridgeNearby` in `api` ext, which `GET
/// /nearby` lists after the Meshtastic module's sources; a change publishes the SSE
/// `nearby` frame. No timer, no task.
pub fn wire_bridge_nearby(api: &Arc<ApiState>, bridge: &bridge::Bridge) {
    use dashboard::api_v1::nearby;
    let heard = Arc::new(nearby::BridgeNearby::default());
    bridge.set_nearby_sink(nearby::sink(api, Arc::clone(&heard)));
    api.insert_ext(heard);
}

/// Register the LXMF helper's seam (§6, C3rs; W1) in `adapters`: a prepare hook that,
/// once the licence notice passes and before the helper starts, writes its Reticulum client
/// config ([`lxmf_client_config`]) and answers `bridge::lxmf::run_args` of the bridge's
/// announce settings (fail closed: no spine, no shared instance or no `rpc_key` refuses
/// the enable and nothing starts); and the adapter factory that attaches `LxmfAdapter` to
/// the running component's stdio (`attach_adapter` then enables it in the core; the route
/// disables it before the child stops). Captures the API state and the bridge weakly;
/// spawns nothing.
pub fn register_lxmf(
    adapters: &dashboard::api_v1::bridge::ComponentAdapters,
    api: &Arc<ApiState>,
    bridge: &Arc<bridge::Bridge>,
) {
    use bridge::lxmf::{self, LxmfAdapter, LxmfConfig};
    use components::EnableError;
    let (api_w, bridge_w) = (Arc::downgrade(api), Arc::downgrade(bridge));
    let prepare: dashboard::api_v1::bridge::PrepareHook = Arc::new(move || {
        let (api, bridge) = (api_w.upgrade(), bridge_w.upgrade());
        Box::pin(async move {
            let stopping = || EnableError::StartFailed("the daemon is stopping".into());
            let api = api.ok_or_else(stopping)?;
            let bridge = bridge.ok_or_else(stopping)?;
            let spine = api
                .ext::<Arc<dashboard::api_v1::transport::TransportApi>>()
                .map(|t| Arc::clone(&t.spine))
                .ok_or_else(|| {
                    EnableError::RequiresUnmet(
                        "this node runs no transport spine, so rnsd's shared instance is not \
                         available to attach to"
                            .into(),
                    )
                })?;
            let root = api.layout.root.clone();
            tokio::task::spawn_blocking(move || lxmf_client_config(&spine, &root))
                .await
                .map_err(|e| {
                    EnableError::InstallFailed(format!("the config task failed: {e}"))
                })??;
            Ok(lxmf::run_args(&bridge.announce()))
        })
    });
    adapters.register_prepare(lxmf::COMPONENT_ID, prepare);
    let bridge_w = Arc::downgrade(bridge);
    let factory: dashboard::api_v1::bridge::AdapterFactory =
        Arc::new(move |components: components::Components| {
            let bridge = bridge_w.upgrade();
            Box::pin(async move {
                let bridge = bridge.ok_or("the bridge is stopping")?;
                let stdio = components
                    .stdio(lxmf::COMPONENT_ID)
                    .await
                    .ok_or("the lxmf component is not running")?;
                let adapter =
                    LxmfAdapter::start(stdio, LxmfConfig::from_manifests(bridge.manifests()));
                Ok(adapter as Arc<dyn bridge::Adapter>)
            })
        });
    adapters.register(lxmf::COMPONENT_ID, factory);
}

/// The LXMF helper's client config (§6 (1), §11): `share_instance = Yes`, the spine's
/// `instance_name` and rnsd's `rpc_key` under `<root>/bridge/lxmf/rns/config` (0700 / 0600,
/// `transport::config::write_client_config`; unchanged bytes are not rewritten). The key
/// is the one the spine rendered rnsd's config with: it must exist already — this never
/// mints one (a fresh key would not be rnsd's). Fails closed with the enable's problem:
/// `requires_unmet` when rnsd does not share its instance or has no key yet,
/// `install_failed` when the key cannot be read or the file written. Blocking.
pub fn lxmf_client_config(
    spine: &transport::posture::Spine,
    root: &Path,
) -> Result<bool, components::EnableError> {
    use components::EnableError;
    use transport::config;
    let f = spine.file();
    if !f.posture.share_instance {
        return Err(EnableError::RequiresUnmet(
            "rnsd does not share its instance (share_instance is off), so the helper has \
             nothing to attach to"
                .into(),
        ));
    }
    let rnsd_root = &spine.layout().root;
    let key = config::secrets_dir(rnsd_root).join(config::RPC_KEY_FILE);
    if !std::fs::symlink_metadata(&key).is_ok_and(|m| m.is_file()) {
        return Err(EnableError::RequiresUnmet(
            "rnsd has no rpc_key yet (its config was never rendered); start the transport \
             service first"
                .into(),
        ));
    }
    let secrets = config::Secrets::for_file(rnsd_root, &f).map_err(|e| {
        EnableError::InstallFailed(format!("rnsd's rpc_key could not be read: {e:#}"))
    })?;
    config::write_client_config(root, &secrets, &f.posture.instance_name).map_err(|e| {
        EnableError::InstallFailed(format!(
            "the helper's client config could not be written: {e:#}"
        ))
    })
}

/// How [`register_deltachat_with`] runs the Madmail relay's one-shot CLI and how long it
/// waits (§7; `bridge::deltachat::Setup`'s knobs). [`DeltaChatWiring::production`] is
/// `Setup::new`'s defaults (the 11240–11243 block the components claim, a 20 s IMAP
/// wait, `SystemRunner`); tests move the ports and shorten the waits.
#[derive(Clone)]
pub struct DeltaChatWiring {
    /// Runs `madmail --config … --state-dir … <switch>` (blocking).
    pub cli: Arc<dyn crate::util::run::Runner>,
    pub ports: bridge::deltachat::RelayPorts,
    pub cfg: bridge::deltachat::DcConfig,
    pub listen_timeout: Duration,
}

impl DeltaChatWiring {
    /// The daemon's wiring (`Setup::new`'s defaults; nothing runs until an enable).
    pub fn production() -> Self {
        let cli: Arc<dyn crate::util::run::Runner> = Arc::new(crate::util::run::SystemRunner);
        let s = bridge::deltachat::Setup::new(
            Path::new("/"),
            bridge::deltachat::RelayBind::Loopback,
            Arc::clone(&cli),
            Arc::new(|_: &str, _: &str| false),
        );
        DeltaChatWiring {
            cli,
            ports: s.ports,
            cfg: s.cfg,
            listen_timeout: s.listen_timeout,
        }
    }

    fn setup(
        &self,
        root: &Path,
        bind: bridge::deltachat::RelayBind,
        optin: bridge::deltachat::OptIn,
    ) -> bridge::deltachat::Setup {
        let mut s = bridge::deltachat::Setup::new(root, bind, Arc::clone(&self.cli), optin);
        s.ports = self.ports;
        s.cfg = self.cfg.clone();
        s.listen_timeout = self.listen_timeout;
        s
    }
}

/// Where the Madmail relay listens (§7: "LAN-bound only while LAN sharing is on"): this
/// hub's first LAN IPv4 (`dashboard::bind::lan_ipv4s`, the order the dashboard prints)
/// while LAN sharing is effective this lifetime (`ApiState::lan_enabled`, the contract's
/// single source of truth, `settings.lan_enabled`), loopback otherwise — and loopback
/// when LAN sharing is on but no LAN address exists (fail closed).
pub fn deltachat_relay_bind(api: &ApiState) -> bridge::deltachat::RelayBind {
    use bridge::deltachat::RelayBind;
    if !api.lan_enabled {
        return RelayBind::Loopback;
    }
    dashboard::bind::lan_ipv4s()
        .first()
        .map_or(RelayBind::Loopback, |ip| RelayBind::Lan(*ip))
}

/// [`register_deltachat_with`] the daemon's wiring.
pub fn register_deltachat(
    adapters: &dashboard::api_v1::bridge::ComponentAdapters,
    api: &Arc<ApiState>,
    bridge: &Arc<bridge::Bridge>,
) {
    register_deltachat_with(adapters, api, bridge, DeltaChatWiring::production());
}

/// Register the Delta Chat network's two components (§7, §2.1; C5, W3) in `adapters`, each
/// enabled through the component route (`enable_component`, its own licence round trip):
///
/// - `madmail`'s prepare hook writes the relay's **first configuration, loopback-only**
///   whatever the LAN setting (`RelayPaths::prepare` + `write_madmail_config`, 0700 / 0600)
///   before its child starts; a relay already running keeps the configuration it runs with
///   (the idempotent enable rewrites nothing).
/// - `deltachat`'s prepare hook needs the relay running (else `requires_unmet`, the
///   components' own copy), then settles and renders it for its bind (`deltachat::relan`:
///   stopped, `push off`, federation rejected, registration open only on the LAN,
///   re-rendered, started) and requires its IMAP listener (`relay_listening`, the bounded
///   wait of `listen_timeout`) — else `start_failed` (`health_failed`) and the core does
///   not start. A core already attached in the bridge changes nothing.
/// - `deltachat`'s factory attaches `DeltaChatAdapter` to the running core's stdio
///   (`deltachat::resume`, on the bind the hook rendered), with the per-contact switch read
///   live from the bridge settings (`opt_in_from`); `attach_adapter` enables it in the
///   core and the route's disable of either component detaches it first.
///
/// The bind is [`deltachat_relay_bind`]. Captures the API state and the bridge weakly;
/// spawns nothing, writes nothing (with both components off: no task, child, listener or
/// file of either).
pub fn register_deltachat_with(
    adapters: &dashboard::api_v1::bridge::ComponentAdapters,
    api: &Arc<ApiState>,
    bridge: &Arc<bridge::Bridge>,
    wiring: DeltaChatWiring,
) {
    use bridge::deltachat::{self as dc, RelayBind, RelayPaths};
    use components::EnableError;
    use dashboard::api_v1::bridge::{AdapterFactory, PrepareHook};
    fn stopping() -> EnableError {
        EnableError::StartFailed("the daemon is stopping".into())
    }
    // the bind the core's hook last rendered the relay for (the factory's relay host)
    let bound: Arc<std::sync::Mutex<Option<RelayBind>>> = Arc::default();

    let (api_w, w) = (Arc::downgrade(api), wiring.clone());
    let relay: PrepareHook = Arc::new(move || {
        let (api, ports) = (api_w.upgrade(), w.ports);
        Box::pin(async move {
            let api = api.ok_or_else(stopping)?;
            let comps = api.ext::<components::Components>().ok_or_else(stopping)?;
            if comps.stdio(dc::RELAY_COMPONENT).await.is_some() {
                return Ok(Vec::new());
            }
            let paths = RelayPaths::of(&api.layout.root);
            tokio::task::spawn_blocking(move || {
                dc::write_madmail_config(&paths, RelayBind::Loopback, ports)
            })
            .await
            .map_err(|e| EnableError::InstallFailed(format!("the config task failed: {e}")))?
            .map_err(|e| {
                EnableError::InstallFailed(format!(
                    "the relay's configuration could not be written: {e:#}"
                ))
            })?;
            Ok(Vec::new())
        })
    });
    adapters.register_prepare(dc::RELAY_COMPONENT, relay);

    let (api_w, bridge_w, w, b) = (
        Arc::downgrade(api),
        Arc::downgrade(bridge),
        wiring.clone(),
        Arc::clone(&bound),
    );
    let core: PrepareHook = Arc::new(move || {
        let (api, bridge, w, bound) = (api_w.upgrade(), bridge_w.upgrade(), w.clone(), b.clone());
        Box::pin(async move {
            let api = api.ok_or_else(stopping)?;
            let bridge = bridge.ok_or_else(stopping)?;
            let comps = api.ext::<components::Components>().ok_or_else(stopping)?;
            if comps.stdio(dc::RPC_COMPONENT).await.is_some()
                && bridge.enabled().iter().any(|n| n == dc::NETWORK)
            {
                return Ok(Vec::new());
            }
            if comps.stdio(dc::RELAY_COMPONENT).await.is_none() {
                let title = comps
                    .get(dc::RELAY_COMPONENT)
                    .map_or("Madmail", |c| c.title.as_str());
                return Err(EnableError::RequiresUnmet(format!(
                    "it needs {title} enabled first"
                )));
            }
            let bind = deltachat_relay_bind(&api);
            let setup = w.setup(&api.layout.root, bind, dc::opt_in_from(&bridge));
            dc::relan(&comps, &setup, bind).await?;
            if !dc::relay_listening(bind, setup.ports, setup.listen_timeout).await {
                return Err(EnableError::StartFailed(format!(
                    "Madmail runs, but its IMAP listener {}:{} did not open; the Delta Chat \
                     core was not started",
                    bind.host(),
                    setup.ports.imap
                )));
            }
            *bound.lock().unwrap_or_else(|e| e.into_inner()) = Some(bind);
            Ok(Vec::new())
        })
    });
    adapters.register_prepare(dc::RPC_COMPONENT, core);

    let (api_w, bridge_w) = (Arc::downgrade(api), Arc::downgrade(bridge));
    let factory: AdapterFactory = Arc::new(move |comps: components::Components| {
        let (api, bridge, w, bound) = (
            api_w.upgrade(),
            bridge_w.upgrade(),
            wiring.clone(),
            Arc::clone(&bound),
        );
        Box::pin(async move {
            let api = api.ok_or("the daemon is stopping")?;
            let bridge = bridge.ok_or("the bridge is stopping")?;
            let last = *bound.lock().unwrap_or_else(|e| e.into_inner());
            let bind = last.unwrap_or_else(|| deltachat_relay_bind(&api));
            let setup = w.setup(&api.layout.root, bind, dc::opt_in_from(&bridge));
            let adapter = dc::resume(&comps, &setup)
                .await
                .ok_or("the Delta Chat core is not running")?;
            Ok(adapter as Arc<dyn bridge::Adapter>)
        })
    });
    adapters.register(dc::RPC_COMPONENT, factory);
}

async fn daemon_main() -> anyhow::Result<()> {
    let layout = DaemonLayout {
        root: crate::util::paths::root(),
        etc: if cfg!(target_os = "android") || std::env::var("NODE111_ROOT").is_ok() {
            crate::util::paths::root().join("etc")
        } else {
            PathBuf::from("/etc/111")
        },
    };
    let env_pairs = load_env(&layout.etc)?;
    for (k, v) in &env_pairs {
        if std::env::var(k).is_err() {
            unsafe { std::env::set_var(k, v) };
        }
    }

    let version = current_version(&layout);
    let state: SharedState = Arc::new(RwLock::new(DashboardState::new(&version)));

    // identity (corruption ⇒ distinct error state, never silent re-key).
    // Layer 5 F2: the loaded identity is kept (not dropped) and handed to
    // the fabric below — the identity handle `mesh111` seals through (§1,
    // L2 §13 signer pattern); nothing else holds the keyset.
    let identity = match crate::identity::load(&layout.identity_dir()) {
        Ok(id) => {
            state.write().unwrap().node_id = Some(id.node_id_hex());
            state.write().unwrap().components.identity = true;
            Some(id)
        }
        Err(e) => {
            eprintln!("111 daemon: identity error: {e:#}");
            state.write().unwrap().error_state = Some(ErrorState {
                kind: "identity".into(),
                detail: "identity failed integrity check — `111 identity restore` required".into(),
            });
            None
        }
    };

    // Layer 6 §3: the bridge's conversation-id key (nothing new persisted)
    let conv_key = identity
        .as_ref()
        .and_then(|id| bridge_conv_key(&id.keyset().to_bytes()));

    // KB snapshot (integrity-verify at start)
    match kb::verify_current(&layout.root) {
        Ok(status) => {
            state.write().unwrap().kb = Some(status);
            state.write().unwrap().components.kb = true;
        }
        Err(e) => eprintln!("111 daemon: KB integrity: {e:#}"),
    }

    // category re-probe + floor + offer stub
    let sysenv = probe::SystemEnv;
    let (_facts, class) = reprobe(&sysenv, &layout);
    if let Ok(class) = &class {
        state.write().unwrap().category = Some(format!("{:?}", class.category));
    }
    if let Ok(Some(floor)) = crate::floor::Floor::read(&layout.state_dir().join("floor.json")) {
        state.write().unwrap().floor_release = Some(floor.highest_release);
    }
    state.write().unwrap().offer = read_offer(&layout);

    // unconditional watchdog heartbeat FIRST (health gates the API, never this)
    let _heartbeat = watchdog::start_heartbeat();

    // transport (conditional child; stub when no python3)
    let runner = crate::util::run::SystemRunner;
    let transport = transport::start(transport::TransportArgs {
        layout: layout.clone(),
        runner: &runner,
        lockfile: layout.root.join("cache/packages/python/rns-lock.txt"),
        cache_py: layout.root.join("cache/packages/python"),
        allow_index: false,
    })
    .await;

    let transport = Arc::new(transport);
    // mirror transport → dashboard components/display state
    {
        let state = Arc::clone(&state);
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        let transport = Arc::clone(&transport);
        tokio::spawn(async move {
            loop {
                tick.tick().await;
                let mut s = state.write().unwrap();
                s.components.transport = transport.health();
                s.transport_state = transport.display_state();
            }
        });
    }

    // Layer 3 children (llama-server + 111-agent; each `absent` unless installed)
    let intelligence = {
        let (category, node_id) = {
            let s = state.read().unwrap();
            (s.category.clone(), s.node_id.clone())
        };
        Arc::new(intelligence::start(
            &layout,
            category.as_deref(),
            node_id.as_deref(),
        ))
    };
    // the daemon tick (2 s): the intelligence mirror, and — once Layer 6 is wired
    // below — the bridge core's pass (no timer of its own, §4)
    let daemon_tick = Arc::new(AtomicU64::new(0));
    let layer6_slot: Arc<OnceLock<Arc<Layer6>>> = Arc::new(OnceLock::new());
    {
        let state = Arc::clone(&state);
        let intelligence = Arc::clone(&intelligence);
        let daemon_tick = Arc::clone(&daemon_tick);
        let layer6_slot = Arc::clone(&layer6_slot);
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        tokio::spawn(async move {
            loop {
                tick.tick().await;
                daemon_tick.fetch_add(1, Ordering::SeqCst);
                state.write().unwrap().intelligence = intelligence.state();
                if let Some(l6) = layer6_slot.get() {
                    l6.tick_once().await;
                }
            }
        });
    }
    let tick_fn: Arc<dyn Fn() -> u64 + Send + Sync> = {
        let t = Arc::clone(&daemon_tick);
        Arc::new(move || t.load(Ordering::SeqCst))
    };

    // probation monitor (health-confirmed / revert-request signals)
    tokio::spawn(probation_monitor(layout.clone(), Arc::clone(&state)));

    // dashboard (Layer 4, D15): loopback unless LAN sharing is on (settings.json
    // lan_enabled, unless DASHBOARD_LAN in the env file overrides it —
    // bind::lan_config); :11180 + :80
    // best-effort; pairing auth for non-loopback peers; mDNS when LAN.
    let lan = dashboard::bind::lan_configured(&env_pairs, &layout.state_dir());
    let dashboard = dashboard::start(Arc::clone(&state), layout.clone(), lan).await;
    // Layer 5 §17: the agent's per-spawn bearer token, checked by require_session
    if let Some(token) = intelligence.daemon_token() {
        dashboard.api.insert_ext(token);
    }

    // updater loop (fetch/verify/stage; never flips — root helper owns that).
    // Started after the dashboard so `POST /api/v1/update/check` can wake it
    // (`ApiState::update_check`) and `/api/v1/status.update` can read it.
    {
        let cfg = updater::fetch::UpdaterConfig::production(
            version
                .parse()
                .unwrap_or_else(|_| "2026.01.01".parse().unwrap()),
        );
        let hooks = updater::Hooks {
            wake: Arc::clone(&dashboard.api.update_check),
            status: Arc::new(updater::UpdaterStatus::default()),
        };
        dashboard.api.insert_ext(Arc::clone(&hooks.status));
        tokio::spawn(updater::run_with_hooks(
            layout.clone(),
            Arc::clone(&state),
            cfg,
            hooks,
        ));
    }

    // Layer 4 R2: services registry (§7 probes, `services` frames), the
    // `status` heartbeat, and the dashboard's restart path (POST
    // /api/v1/node/restart → graceful exit 0 below; the supervisor restarts).
    // Layer 5 §13: hosted panels — embedded declarations, slots, listeners of
    // enabled panels, tickets/sessions; `PanelInfo` facts feed the registry.
    // Built-in shims register here (`meshtastic-http`: the Meshtastic module).
    let mut panel_shims = panels::ShimRegistry::new();
    // Layer 5 §14: the Meshtastic module (meshmud + presence reader children,
    // consented identify, Nearby, the `meshtastic-http` shim) and the
    // daemon-wide port-owner table it claims board ports in.
    let port_table = Arc::new(ports::PortTable::new());
    dashboard.api.insert_ext(Arc::clone(&port_table));
    // Layer 6 §12: the spine claims 111d's and rnsd's radio ports in the one table
    if let Err(e) = transport
        .spine()
        .adopt_ports(Arc::clone(&port_table), Arc::clone(&tick_fn))
        .await
    {
        eprintln!("111 transport: {e}");
    }
    // §14: the presence reader's venv for the lock `current/` carries, before
    // the module exists (its reader start checks `reader_installed()`)
    ensure_presence_venv(&layout, Arc::new(crate::util::run::SystemRunner)).await;
    let meshtastic = meshtastic::Meshtastic::new(
        meshtastic::MeshtasticConfig::production(
            &layout,
            cfg!(target_os = "android")
                || state.read().unwrap().category.as_deref() == Some("Node2A"),
        ),
        Arc::clone(&port_table),
    );
    dashboard.api.insert_ext(Arc::clone(&meshtastic));
    if let Err(e) = panel_shims.register(Arc::new(meshtastic.shim())) {
        eprintln!("111 daemon: {e}");
    }
    let panel_host = Arc::new(panels::PanelHost::load(
        panels::PanelsConfig::new(&layout),
        &panel_shims,
        dashboard.api.lan_enabled,
    ));
    dashboard.api.insert_ext(Arc::clone(&panel_host));
    // Layer 5 F2: the messaging fabric (store, identity handle, tcp-lan,
    // routes' state, SSE `message`/`contact`/`fabric`, `fabric` service facts).
    let fabric = {
        let category = state.read().unwrap().category.clone();
        fabric::start(
            fabric::FabricConfig::production(&layout, category.as_deref()),
            identity,
            &dashboard.api,
        )
        .await
    };
    dashboard.api.insert_ext(Arc::clone(&fabric));
    // Layer 6: the bridge core (inert until an adapter is enabled), the Meshtastic client
    // registered (started by the module while service `meshtastic` is LIVE), the optional
    // components, the transport routes
    let mesh_client = conv_key.map(|k| {
        let c = meshtastic::client::MeshtasticAdapter::new(
            meshtastic::client::ClientConfig::production(meshtastic::meshmud::DOWNLINK),
            *k,
        );
        meshtastic.attach_client(Arc::clone(&c));
        c
    });
    let layer6 = Arc::new(start_layer6(Layer6Parts {
        api: Arc::clone(&dashboard.api),
        fabric: Arc::clone(&fabric),
        ports: Arc::clone(&port_table),
        tick: Arc::clone(&tick_fn),
        transport: Some((transport.spine(), transport.facts_cache())),
        mesh_client,
        cmd: Arc::new(crate::util::run::SystemRunner),
    }));
    let mut sources =
        services::probe::Sources::new(Arc::clone(&intelligence), Arc::clone(&transport));
    sources.panels = Some(panels::provider(&panel_host));
    sources.fabric = Some(fabric.provider());
    meshtastic.install_providers(&mut sources);
    services::probe::spawn(Arc::clone(&dashboard.api), sources);
    let _panels = panels::spawn(Arc::clone(&dashboard.api), Arc::clone(&panel_host));
    // revoked Meshtastic panel sessions close their shim downlink; drop count
    let _mesh_panels = meshtastic.attach_panels(&panel_host);
    // POST /api/v1/services/{id}/restart: the children this daemon supervises
    // (the Meshtastic module re-inserts it with meshmud + the reader added)
    let supervised = services::Supervised::default()
        .with(services::ServiceId::Agent, intelligence.agent_control())
        .with(services::ServiceId::Model, intelligence.model_control())
        .with(services::ServiceId::Transport, transport.control());
    dashboard.api.insert_ext(Arc::new(supervised.clone()));
    dashboard::api_v1::status::spawn_heartbeat(Arc::clone(&dashboard.api));
    let restart = Arc::new(tokio::sync::Notify::new());
    let restart_signal = dashboard::api_v1::actions::RestartSignal(Arc::clone(&restart));
    dashboard.api.insert_ext(restart_signal);
    // device registry (Layer 4 R4): detection + paired sessions → /api/v1/devices
    let devices = devices::start(Arc::clone(&dashboard.api), {
        let transport = Arc::clone(&transport);
        move || transport.child_state() == Some(supervise::ChildState::Running)
    });
    // Layer 6 §11: `interfaces.toml` is the only home of the Meshtastic binding — the
    // registry reads and writes it through the spine, and the spine's write-only
    // `devices.json` mirror goes through the registry's in-memory store (C4)
    devices.attach_spine(transport.spine());
    // Layer 6 §12: the device core's apply → probe → keep-or-revert loop for accepted
    // `kind: device` proposals (the registry is in ext now)
    match devices::applier::Applier::production(
        &dashboard.api,
        transport.spine(),
        transport
            .control()
            .map(|c| Arc::new(c) as Arc<dyn transport::posture::RnsdChild>),
        Some(Arc::new(layer6.components.clone()) as Arc<dyn devices::applier::HelperCheck>),
    ) {
        Some(applier) => dashboard.api.insert_ext(applier),
        None => eprintln!("111 daemon: devices: no applier (the registry is not running)"),
    }
    // meshmud + the presence reader from the binding (Layer 5 §14; Layer 6 §11)
    meshtastic
        .start(Arc::clone(&dashboard.api), devices, supervised)
        .await;
    // Layer 6: the components whose switch is on, now that their dependencies exist;
    // then the bridge pass joins the daemon tick
    layer6.resume(&dashboard.api).await;
    let _ = layer6_slot.set(Arc::clone(&layer6));
    // agent activity (Layer 4 R3): 111-agent /v1/agent/events → `agent` + `proposal` frames
    let _agent_events = dashboard::api_v1::relay::start_events(&dashboard.api);

    // READY once serving (components may still be warming — /health reflects)
    watchdog::ready();

    // signals: SIGTERM/SIGINT → graceful; voluntary exit(0) = Layer 9 primitive
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = sigterm.recv() => {},
        _ = restart.notified() => {
            eprintln!("111 daemon: restart requested from the dashboard — exiting 0 for the supervisor");
        },
    }
    dashboard.stop();
    // Layer 6: adapters off, component children joined (before rnsd stops below)
    layer6.shutdown().await;
    // Layer 5: close the fabric cleanly (store clean record + L2 counters)
    fabric.shutdown().await;
    // join every child (SIGTERM, grace, SIGKILL) before exiting: an orphaned
    // llama-server would keep the port and the RAM the next daemon needs
    tokio::join!(
        transport.stop_and_wait(CHILD_STOP_GRACE),
        intelligence.stop_and_wait(CHILD_STOP_GRACE),
        meshtastic.stop_and_wait(CHILD_STOP_GRACE),
    );
    eprintln!("111 daemon: shutting down");
    Ok(())
}

/// Daemon start, before the Meshtastic module is built (os/5-messaging/
/// README.md §14, §19 L1; every promotion restarts the daemon): ensure
/// `venvs/presence` was built from the lock the release in `current/` carries
/// — rebuilt offline from `cache/packages/presence/` when an update changed
/// the lock, nothing at all when it did not, nothing when the release ships no
/// reader (`transport::venv::presence::ensure_current`). Runs on a blocking
/// thread and completes before the reader can start, so the reader starts on
/// the venv this release needs. Never fatal: an unavailable venv is logged
/// here and recorded in `venvs/presence.status.json`, whose reason the
/// `meshtastic` service row shows.
pub async fn ensure_presence_venv(
    layout: &DaemonLayout,
    runner: Arc<dyn crate::util::run::Runner>,
) -> transport::venv::presence::Outcome {
    use transport::venv::presence::{self, Outcome, PresenceVenvError};
    let at = layout.clone();
    let outcome = tokio::task::spawn_blocking(move || presence::ensure_current(&at, &*runner))
        .await
        .unwrap_or_else(|e| Outcome::Unavailable {
            error: PresenceVenvError::Io(format!("the build task failed: {e}")),
            previous_kept: presence::python(&layout.root).is_file(),
        });
    match &outcome {
        Outcome::Built => eprintln!("111 daemon: presence venv rebuilt for this release's lock"),
        Outcome::Unavailable { error, .. } => eprintln!("111 daemon: presence venv: {error}"),
        Outcome::NotShipped | Outcome::Unchanged => {}
    }
    outcome
}

/// `111 daemon` entry (called by systemd/runit).
pub fn run() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("111d")
        .build()?;
    rt.block_on(daemon_main())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bridge_conv_key_is_keyed_by_the_x25519_half_and_its_own_tag() {
        let mut ks = [0u8; 64];
        for (i, b) in ks.iter_mut().enumerate() {
            *b = i as u8;
        }
        let k = bridge_conv_key(&ks).unwrap();
        assert_eq!(*k, *bridge_conv_key(&ks).unwrap(), "deterministic");
        let store = fabric::sealer::store_secret_from(&ks).unwrap();
        assert_ne!(*k, *store, "a different tag, a different key");
        let mut other = ks;
        other[0] ^= 0xff; // the signing half does not key it
        assert_eq!(*bridge_conv_key(&other).unwrap(), *k);
        other[40] ^= 0xff;
        assert_ne!(*bridge_conv_key(&other).unwrap(), *k);
    }

    #[test]
    fn env_file_parsing() {
        let parsed = parse_env_file("# comment\n\nDASHBOARD_LAN=true\nNODE111_X= y \n");
        assert_eq!(
            parsed,
            vec![
                ("DASHBOARD_LAN".to_string(), "true".to_string()),
                ("NODE111_X".to_string(), "y".to_string())
            ]
        );
    }
}
