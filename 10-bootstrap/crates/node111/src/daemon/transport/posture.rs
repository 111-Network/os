//! The spine's apply path (`os/60-transport/README.md` §11 "Live attach,
//! detach, reload", §12 "Port ownership" and the `rns.posture.set` row, §6
//! "Start order"; §18 Layer 1: child ordering is orchestration over
//! `restart` and fresh spawns, not a supervisor feature).
//!
//! [`Spine`] holds the parsed `state/interfaces.toml` and applies every change
//! the same way: **re-render → diff → act**.
//! - The new config is rendered from the in-memory file (`config::render_with`)
//!   after the RNode ports are **claimed in the `PortTable` for `rnsd`**
//!   (a refused claim leaves that stanza out, with the holder named), and
//!   written only when it changed (0600, `rns/` 0700).
//! - [`diff`] compares the old and new file: a change outside the interface
//!   stanzas (`[reticulum]` keys: share_instance, instance_name, rpc_key,
//!   enable_transport) needs an **rnsd restart**; otherwise each removed
//!   stanza is `rnstatus --detach`ed, each changed one `--reload`ed and each
//!   new one `--attach`ed. Comments never count.
//! - The restart goes through `ChildControl::restart` (never `pkill`) with
//!   the child ordering of §6: every enabled component that depends on
//!   `transport` is stopped first (`Components::stop_before`), rnsd restarts,
//!   then each is resumed (`Components::start_after`), whose runner gates on
//!   rnsd serving again and runs the post-spawn partition check.
//! - Ports rnsd no longer drives are released after the detach.
//! - The netlink path ([`Spine::on_link_change`]) is the same apply plus a
//!   `--reload` of the AutoInterface(s), so new links are picked up without a
//!   restart.
//! - With `share_instance` off there is no control RPC (rnstatus talks to
//!   the shared instance), so any interface change restarts rnsd instead and
//!   a link change reloads nothing (v1's behaviour).
//!
//! Applies are serialised by one async lock. Nothing here runs on a timer.

use super::config::{self, LeftOut, Rendered, Secrets};
use super::interfaces::{
    self, IfKind, IfOwner, InterfacesFile, Invalid, Startup, AUTO_NAME, STOCK_AUTO_NAME, TASK_RNSD,
};
use super::rnstatus::Rnstatus;
use crate::daemon::components::Components;
use crate::daemon::devices::store::MeshtasticBinding;
use crate::daemon::ports::{port_key, PortTable};
use crate::daemon::services::ServiceId;
use crate::daemon::supervise::{ChildControl, ChildState};
use crate::daemon::DaemonLayout;
use crate::install::categorize::Category;
use std::collections::BTreeSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// A boxed `Send` future (object-safe async hooks without a macro crate).
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The dependants of rnsd and how to hold them across a restart (§6).
/// Production: [`Components`].
pub trait RestartHooks: Send + Sync {
    /// The enabled components whose dependency is `transport`.
    fn dependants(&self) -> BoxFut<'_, Vec<String>>;
    /// Stop `id` and hold it (returns once stopped).
    fn stop_before<'a>(&'a self, id: &'a str) -> BoxFut<'a, bool>;
    /// Resume `id` (its runner gates on rnsd serving, then checks again).
    fn start_after<'a>(&'a self, id: &'a str) -> BoxFut<'a, ()>;
}

impl RestartHooks for Components {
    fn dependants(&self) -> BoxFut<'_, Vec<String>> {
        Box::pin(Components::dependants(self, ServiceId::Transport))
    }

    fn stop_before<'a>(&'a self, id: &'a str) -> BoxFut<'a, bool> {
        Box::pin(Components::stop_before(self, id))
    }

    fn start_after<'a>(&'a self, id: &'a str) -> BoxFut<'a, ()> {
        Box::pin(Components::start_after(self, id))
    }
}

/// The supervised rnsd child. Production: [`ChildControl`].
pub trait RnsdChild: Send + Sync {
    fn running(&self) -> bool;
    /// Restart in place (SIGTERM, grace, SIGKILL, respawn); false when the
    /// child is stopped for good.
    fn restart(&self, grace: Duration) -> BoxFut<'_, bool>;
}

impl RnsdChild for ChildControl {
    fn running(&self) -> bool {
        self.state() == ChildState::Running
    }

    fn restart(&self, grace: Duration) -> BoxFut<'_, bool> {
        Box::pin(ChildControl::restart(self, grace))
    }
}

/// Copies the binding into `devices.json` after a save (the write-only
/// mirror, §11); returns a divergence line.
pub type BindingMirror =
    Arc<dyn Fn(Option<(&str, &MeshtasticBinding)>) -> Option<String> + Send + Sync>;

/// The daemon tick stamped on port claims (never wall clock).
pub type Tick = Arc<dyn Fn() -> u64 + Send + Sync>;

// ---------------------------------------------------------------- diff

/// What changed between two rendered configs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigDiff {
    /// A key outside the interface stanzas changed: rnsd restarts.
    pub restart: bool,
    pub attach: Vec<String>,
    pub detach: Vec<String>,
    pub reload: Vec<String>,
}

impl ConfigDiff {
    pub fn is_empty(&self) -> bool {
        !self.restart && self.attach.is_empty() && self.detach.is_empty() && self.reload.is_empty()
    }
}

/// A config split into the lines outside the interface stanzas and the
/// stanzas (name, body), comments and blank lines dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sections {
    pub head: Vec<String>,
    pub stanzas: Vec<(String, Vec<String>)>,
}

/// Split a Reticulum config (the ConfigObj form 111 renders).
pub fn split(text: &str) -> Sections {
    let mut s = Sections::default();
    let mut in_interfaces = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix("[[").and_then(|l| l.strip_suffix("]]")) {
            if in_interfaces {
                s.stanzas.push((name.trim().to_string(), Vec::new()));
                continue;
            }
        } else if line.starts_with('[') && line.ends_with(']') {
            in_interfaces = line.eq_ignore_ascii_case("[interfaces]");
            s.head.push(line.to_string());
            continue;
        }
        match (in_interfaces, s.stanzas.last_mut()) {
            (true, Some((_, body))) => body.push(line.to_string()),
            _ => s.head.push(line.to_string()),
        }
    }
    s
}

/// The actions that take `old` to `new` (module doc).
pub fn diff(old: &str, new: &str) -> ConfigDiff {
    let a = split(old);
    let b = split(new);
    if a.head != b.head {
        return ConfigDiff {
            restart: true,
            ..ConfigDiff::default()
        };
    }
    let find = |s: &Sections, n: &str| {
        s.stanzas
            .iter()
            .find(|(m, _)| m == n)
            .map(|(_, body)| body.clone())
    };
    let mut d = ConfigDiff::default();
    for (name, body) in &a.stanzas {
        match find(&b, name) {
            None => d.detach.push(name.clone()),
            Some(nb) if nb != *body => d.reload.push(name.clone()),
            Some(_) => {}
        }
    }
    for (name, _) in &b.stanzas {
        if find(&a, name).is_none() {
            d.attach.push(name.clone());
        }
    }
    d
}

// ---------------------------------------------------------------- spine

/// Why a change was not applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// A newer schema owns `interfaces.toml`: this binary never saves it.
    ReadOnly,
    /// A newer `schema-version` owns `rns/config`: never regenerated.
    NewerConfig,
    Invalid(Invalid),
    Save(String),
    Write(String),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::ReadOnly => {
                f.write_str("interfaces.toml belongs to a newer 111 release; not changed")
            }
            ApplyError::NewerConfig => {
                f.write_str("rns/config belongs to a newer 111 release; not regenerated")
            }
            ApplyError::Invalid(e) => write!(f, "refused: {e}"),
            ApplyError::Save(e) => write!(f, "not saved: {e}"),
            ApplyError::Write(e) => write!(f, "rns/config not written: {e}"),
        }
    }
}

impl std::error::Error for ApplyError {}

/// What an apply did (for the applier's probe and the logs).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// `rns/config` was rewritten.
    pub written: bool,
    pub diff: ConfigDiff,
    /// rnsd was restarted (with its dependants in order).
    pub restarted: bool,
    /// What ran, in order (`detach X`, `reload X`, `attach X`,
    /// `stop <component>`, `restart rnsd`, `start <component>`).
    pub actions: Vec<String>,
    /// What failed, one line each.
    pub failures: Vec<String>,
    /// Stanzas not rendered, with reasons.
    pub left_out: Vec<LeftOut>,
}

#[derive(Debug, Clone, Copy)]
enum Op {
    Attach,
    Detach,
    Reload,
}

impl Op {
    fn as_str(self) -> &'static str {
        match self {
            Op::Attach => "attach",
            Op::Detach => "detach",
            Op::Reload => "reload",
        }
    }
}

/// The transport spine of one node (module doc). Shared as `Arc<Spine>`.
pub struct Spine {
    layout: DaemonLayout,
    file: RwLock<InterfacesFile>,
    read_only: bool,
    lan_sharing: AtomicBool,
    category: Option<Category>,
    rnstatus: Arc<Rnstatus>,
    apply_lock: tokio::sync::Mutex<()>,
    rnsd: RwLock<Option<Arc<dyn RnsdChild>>>,
    hooks: RwLock<Option<Arc<dyn RestartHooks>>>,
    ports: RwLock<Option<(Arc<PortTable>, Tick)>>,
    mirror: RwLock<BindingMirror>,
    left_out: RwLock<Vec<LeftOut>>,
    grace: RwLock<Duration>,
}

fn read<T: Clone>(l: &RwLock<T>) -> T {
    l.read().unwrap_or_else(|p| p.into_inner()).clone()
}

fn write<T>(l: &RwLock<T>, v: T) {
    *l.write().unwrap_or_else(|p| p.into_inner()) = v;
}

impl Spine {
    /// A spine over `startup`'s file. The binding mirror defaults to
    /// `state/devices.json` on disk; [`Spine::set_binding_mirror`] swaps in
    /// the registry's in-memory store.
    pub fn new(
        layout: DaemonLayout,
        startup: Startup,
        rnstatus: Arc<Rnstatus>,
        lan_sharing: bool,
        category: Option<Category>,
    ) -> Arc<Self> {
        let devices_json: PathBuf = layout.state_dir().join(crate::daemon::devices::store::FILE);
        let mirror: BindingMirror = Arc::new(move |want| {
            crate::daemon::devices::store::mirror_meshtastic_binding(&devices_json, want)
        });
        Arc::new(Spine {
            layout,
            file: RwLock::new(startup.file),
            read_only: startup.read_only,
            lan_sharing: AtomicBool::new(lan_sharing),
            category,
            rnstatus,
            apply_lock: tokio::sync::Mutex::new(()),
            rnsd: RwLock::new(None),
            hooks: RwLock::new(None),
            ports: RwLock::new(None),
            mirror: RwLock::new(mirror),
            left_out: RwLock::new(Vec::new()),
            grace: RwLock::new(crate::daemon::CHILD_STOP_GRACE),
        })
    }

    /// The parsed file (a copy).
    pub fn file(&self) -> InterfacesFile {
        read(&self.file)
    }

    pub fn read_only(&self) -> bool {
        self.read_only
    }

    pub fn lan_sharing(&self) -> bool {
        self.lan_sharing.load(Ordering::SeqCst)
    }

    pub fn category(&self) -> Option<Category> {
        self.category
    }

    pub fn layout(&self) -> &DaemonLayout {
        &self.layout
    }

    /// rnstatus for this node (the applier's probe runs it on a blocking
    /// thread).
    pub fn rnstatus(&self) -> Arc<Rnstatus> {
        Arc::clone(&self.rnstatus)
    }

    /// The stanzas the last render left out, with reasons.
    pub fn left_out(&self) -> Vec<LeftOut> {
        read(&self.left_out)
    }

    /// The Meshtastic binding (§11: this file wins; the Layer 5 readers
    /// switch to this).
    pub fn meshtastic_binding(&self) -> Option<(String, MeshtasticBinding)> {
        self.file.read().ok()?.meshtastic_binding()
    }

    pub fn set_rnsd(&self, c: Arc<dyn RnsdChild>) {
        write(&self.rnsd, Some(c));
    }

    pub fn set_restart_hooks(&self, h: Arc<dyn RestartHooks>) {
        write(&self.hooks, Some(h));
    }

    pub fn set_binding_mirror(&self, m: BindingMirror) {
        write(&self.mirror, m);
    }

    /// SIGTERM → SIGKILL grace of an rnsd restart (tests shorten it).
    pub fn set_grace(&self, grace: Duration) {
        write(&self.grace, grace);
    }

    fn rnsd(&self) -> Option<Arc<dyn RnsdChild>> {
        read(&self.rnsd)
    }

    fn secrets(&self, f: &InterfacesFile) -> Result<Secrets, ApplyError> {
        Secrets::for_file(&self.layout.root, f).map_err(|e| ApplyError::Write(format!("{e:#}")))
    }

    /// Render `f`, claiming the RNode ports for rnsd first when a
    /// `PortTable` is adopted; returns the render and the port keys rnsd
    /// now holds.
    fn render_claimed(
        &self,
        f: &InterfacesFile,
    ) -> Result<(Rendered, Option<BTreeSet<String>>), ApplyError> {
        let secrets = self.secrets(f)?;
        let lan = self.lan_sharing();
        let first = config::render_with(f, &secrets, lan, self.category, &[]);
        let Some((table, tick)) = read(&self.ports) else {
            return Ok((first, None));
        };
        let left: BTreeSet<String> = first
            .left_out
            .iter()
            .map(|l| l.name.to_ascii_lowercase())
            .collect();
        let mut exclude = Vec::new();
        let mut wanted = BTreeSet::new();
        for i in f.interfaces.iter().filter(|i| {
            i.owner == IfOwner::Rnsd
                && i.kind == IfKind::RNode
                && !left.contains(&i.name.to_ascii_lowercase())
        }) {
            let Some(port) = i.port.as_deref() else {
                continue;
            };
            match table.claim(port, TASK_RNSD, tick()) {
                Ok(_) => {
                    wanted.insert(port_key(port));
                }
                Err(busy) => exclude.push(LeftOut {
                    name: i.name.clone(),
                    reason: format!("its port is in use by {}", busy.owner.task),
                }),
            }
        }
        let rendered = if exclude.is_empty() {
            first
        } else {
            config::render_with(f, &secrets, lan, self.category, &exclude)
        };
        Ok((rendered, Some(wanted)))
    }

    fn release_unwanted(&self, wanted: Option<&BTreeSet<String>>) {
        let (Some((table, _)), Some(wanted)) = (read(&self.ports), wanted) else {
            return;
        };
        for (key, owner) in table.held() {
            if owner.task == TASK_RNSD && !wanted.contains(&key) {
                let _ = table.release(&key, TASK_RNSD);
            }
        }
    }

    fn log_left_out(left: &[LeftOut]) {
        for l in left {
            eprintln!(
                "111 transport: interface {:?} is not rendered: {}",
                l.name, l.reason
            );
        }
    }

    /// The daemon-start render: write `rns/config` from the file (claims
    /// included when a table is adopted), no rnsd action (rnsd is spawned
    /// after this, or reads it at its next spawn).
    pub fn write_config(&self) -> Result<Rendered, ApplyError> {
        if !config::should_regenerate(&self.layout) {
            return Err(ApplyError::NewerConfig);
        }
        let f = self.file();
        let (rendered, wanted) = self.render_claimed(&f)?;
        config::write_rendered(&self.layout, &rendered.text)
            .map_err(|e| ApplyError::Write(format!("{e:#}")))?;
        write(&self.left_out, rendered.left_out.clone());
        Self::log_left_out(&rendered.left_out);
        self.release_unwanted(wanted.as_ref());
        Ok(rendered)
    }

    async fn control(&self, op: Op, name: &str, applied: &mut Applied) {
        let rn = Arc::clone(&self.rnstatus);
        let n = name.to_string();
        let r = tokio::task::spawn_blocking(move || match op {
            Op::Attach => rn.attach(&n),
            Op::Detach => rn.detach(&n),
            Op::Reload => rn.reload(&n),
        })
        .await;
        match r {
            Ok(Ok(())) => applied.actions.push(format!("{} {name}", op.as_str())),
            Ok(Err(e)) => applied
                .failures
                .push(format!("{} {name}: {e}", op.as_str())),
            Err(e) => applied
                .failures
                .push(format!("{} {name}: {e}", op.as_str())),
        }
    }

    /// Restart rnsd with the child ordering of §6 (module doc).
    async fn restart(&self, applied: &mut Applied) {
        let hooks = read(&self.hooks);
        let deps = match &hooks {
            Some(h) => h.dependants().await,
            None => Vec::new(),
        };
        if let Some(h) = &hooks {
            for d in &deps {
                h.stop_before(d).await;
                applied.actions.push(format!("stop {d}"));
            }
        }
        let ok = match self.rnsd() {
            Some(c) => c.restart(read(&self.grace)).await,
            None => false,
        };
        applied.restarted = ok;
        if ok {
            applied.actions.push("restart rnsd".into());
        } else {
            applied
                .failures
                .push("restart rnsd: the child is not supervised".into());
        }
        if let Some(h) = &hooks {
            for d in &deps {
                h.start_after(d).await;
                applied.actions.push(format!("start {d}"));
            }
        }
    }

    async fn apply_locked(&self, reload_auto: bool) -> Result<Applied, ApplyError> {
        if !config::should_regenerate(&self.layout) {
            return Err(ApplyError::NewerConfig);
        }
        let f = self.file();
        let old = config::current_text(&self.layout).unwrap_or_default();
        let (rendered, wanted) = self.render_claimed(&f)?;
        let written = config::write_rendered(&self.layout, &rendered.text)
            .map_err(|e| ApplyError::Write(format!("{e:#}")))?;
        write(&self.left_out, rendered.left_out.clone());
        if written {
            Self::log_left_out(&rendered.left_out);
        }
        let d = diff(&old, &rendered.text);
        let mut applied = Applied {
            written,
            diff: d.clone(),
            left_out: rendered.left_out.clone(),
            ..Applied::default()
        };
        if self.rnsd().is_some_and(|c| c.running()) {
            // without a shared instance there is no control RPC: an
            // interface change can only be applied by a restart
            let live = f.posture.share_instance;
            if d.restart || (!live && !d.is_empty()) {
                self.restart(&mut applied).await;
            } else if !live {
                // a link change with nothing to apply: nothing to reload through
            } else {
                for n in &d.detach {
                    self.control(Op::Detach, n, &mut applied).await;
                }
                for n in &d.reload {
                    self.control(Op::Reload, n, &mut applied).await;
                }
                for n in &d.attach {
                    self.control(Op::Attach, n, &mut applied).await;
                }
                if reload_auto {
                    let present = split(&rendered.text);
                    for auto in [AUTO_NAME, STOCK_AUTO_NAME] {
                        let rendered_here = present.stanzas.iter().any(|(n, _)| n == auto);
                        let touched = d.attach.iter().chain(&d.reload).any(|n| n == auto);
                        if rendered_here && !touched {
                            self.control(Op::Reload, auto, &mut applied).await;
                        }
                    }
                }
            }
        }
        self.release_unwanted(wanted.as_ref());
        for line in &applied.failures {
            eprintln!("111 transport: {line}");
        }
        Ok(applied)
    }

    /// Re-render and apply the current file (module doc).
    pub async fn apply(&self) -> Result<Applied, ApplyError> {
        let _g = self.apply_lock.lock().await;
        self.apply_locked(false).await
    }

    /// The netlink path: re-render, diff, act, and `--reload` the
    /// AutoInterface(s) so new links are used (§11).
    pub async fn on_link_change(&self) -> Result<Applied, ApplyError> {
        let _g = self.apply_lock.lock().await;
        self.apply_locked(true).await
    }

    /// Change the file through `f`, validate, save (0600, atomic), mirror
    /// the binding into `devices.json`, then apply.
    pub async fn update<F>(&self, f: F) -> Result<Applied, ApplyError>
    where
        F: FnOnce(&mut InterfacesFile) + Send,
    {
        let _g = self.apply_lock.lock().await;
        if self.read_only {
            return Err(ApplyError::ReadOnly);
        }
        let mut next = self.file();
        f(&mut next);
        next.validate().map_err(ApplyError::Invalid)?;
        interfaces::save(&self.layout.state_dir(), &next).map_err(|e| match e {
            interfaces::SaveError::Invalid(i) => ApplyError::Invalid(i),
            other => ApplyError::Save(other.to_string()),
        })?;
        let binding = next.meshtastic_binding();
        write(&self.file, next);
        let mirror = read(&self.mirror);
        if let Some(line) = mirror(binding.as_ref().map(|(id, b)| (id.as_str(), b))) {
            eprintln!("111 transport: {line}");
        }
        self.apply_locked(false).await
    }

    /// Bind (or with `None`, unbind) the Meshtastic board in the file (the
    /// write the Layer 5 identify path switches to, §11).
    pub async fn set_meshtastic_binding(
        &self,
        binding: Option<(String, MeshtasticBinding)>,
    ) -> Result<Applied, ApplyError> {
        self.update(move |f| {
            f.set_meshtastic_binding(binding.as_ref().map(|(id, b)| (id.as_str(), b)))
        })
        .await
    }

    /// LAN sharing changed (the Backbone listener and the `[posture.lan]`
    /// items follow it).
    pub async fn set_lan_sharing(&self, on: bool) -> Result<Applied, ApplyError> {
        let _g = self.apply_lock.lock().await;
        self.lan_sharing.store(on, Ordering::SeqCst);
        self.apply_locked(false).await
    }

    /// Adopt the daemon's `PortTable` (§12): claim the 111d-owned uplink for
    /// its task (logged when held), then apply, which claims rnsd's RNode
    /// ports and detaches a stanza whose port another task holds.
    pub async fn adopt_ports(
        &self,
        table: Arc<PortTable>,
        tick: Tick,
    ) -> Result<Applied, ApplyError> {
        for i in self
            .file()
            .interfaces
            .iter()
            .filter(|i| i.owner == IfOwner::D111)
        {
            if let Some(port) = i.owned_port() {
                if let Err(busy) = table.claim(&port, i.claim_task(), tick()) {
                    eprintln!("111 transport: interface {:?}: {busy}", i.name);
                }
            }
        }
        write(&self.ports, Some((table, tick)));
        self.apply().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "# c\nschema-version = 2\n\n[reticulum]\n  share_instance = Yes\n  rpc_key = aa\n\n\
[interfaces]\n  [[Auto Interface]]\n    type = AutoInterface\n  [[R1]]\n    type = RNodeInterface\n    \
frequency = 1\n  [[R2]]\n    type = RNodeInterface\n";

    #[test]
    fn diff_stanzas_and_head() {
        assert!(diff(A, A).is_empty());
        // a comment change is nothing
        assert!(diff(A, &A.replace("# c", "# other")).is_empty());
        // stanza changes
        let b = A
            .replace("    frequency = 1\n", "    frequency = 2\n")
            .replace(
                "  [[R2]]\n    type = RNodeInterface\n",
                "  [[R3]]\n    type = RNodeInterface\n",
            );
        let d = diff(A, &b);
        assert!(!d.restart);
        assert_eq!(d.reload, ["R1"]);
        assert_eq!(d.detach, ["R2"]);
        assert_eq!(d.attach, ["R3"]);
        // a [reticulum] key: restart only
        let c = A.replace("rpc_key = aa", "rpc_key = bb");
        assert_eq!(
            diff(A, &c),
            ConfigDiff {
                restart: true,
                ..ConfigDiff::default()
            }
        );
        // no old file: restart
        assert!(diff("", A).restart);
    }
}
