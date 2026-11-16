//! Layer 8 (`os/8-storage/README.md` §4.3 "The owner" and its closing paragraph, §1 rule 2, §18
//! "The pass knob"; row D1, an additive amendment of locked Layer 1, §21 L1): the daemon-level
//! owner of the one housekeeping interval.
//!
//! **The owner.** Until row D1 the 5 s interval (`FabricConfig.housekeeping`) was spawned inside
//! `fabric::start`, and only while the fabric was live. [`spawn`] now starts it once, from
//! `daemon/mod.rs`, with or without a live fabric, and the fabric spawns none, so the daemon holds
//! exactly one interval either way: it changed owner, not count (§1 rule 2: no timer is added).
//! [`Housekeeping::stop`] ends it before the fabric closes, where the fabric's own was aborted.
//!
//! **A tick.** Every [`PRUNE_EVERY`] = 12 ticks (≈ 1 min), the first at `n = 0`, i.e. at once, a
//! capacity pass runs. Step 1 is the fabric's `capacity_pass` as built while the fabric is live
//! (its `statvfs` of `fabric/` handed to the store's `prune`; the pass's outcome unchanged), else
//! the pass's own `statvfs` of the root. [`PassCtx`] carries that figure, `disk_low`, the fabric's
//! live bytes (none while it is down) and the tick count to steps 2 to 9, which [`pass`] runs in
//! §4.3's order whether or not the fabric is live; step 3 (`set_watermarks`, inside the budget's
//! step) runs only while the fabric is live, which the budget reads from
//! [`PassCtx::fabric_live_bytes`]. Then, on every tick while the fabric is live, `refresh_facts`,
//! as before.
//!
//! **The pass knob** (§18). A debug build reads `NODE111_TEST_HOUSEKEEPING_MS` (whole
//! milliseconds) once, at [`spawn`], and shortens the interval to it (never lengthens it; at
//! least 10 ms); the pass still runs every [`PRUNE_EVERY`] ticks. A release build compiles the
//! read out (the `NODE111_BRIDGE_STUBS` gating: its `knob` never asks the environment): it has no
//! such setting and carries no weight from it.
//!
//! **Never adds** (§1 rule 2): a timer, a second task, a store open, a file read or a write on a
//! pass with nothing configured. With the fabric live a tick does what the fabric's own did; with
//! it down a pass is one `statvfs` and the steps, each of which returns at once until its row
//! lands.

use crate::daemon::dashboard::api_v1::ApiState;
use crate::daemon::fabric::{self, FabricHandle};
use crate::daemon::{datasets, state, storage};
use mesh111::store::DiskSpace;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

/// Housekeeping ticks between capacity passes (≈ 1 min at 5 s; §4.3: `PRUNE_EVERY` = 12, the
/// first pass on the first tick).
pub const PRUNE_EVERY: u64 = 12;

/// The pass knob (§18 "The pass knob"): read by a debug build only, so a release build has no
/// such setting.
#[cfg(debug_assertions)]
pub const PASS_KNOB: &str = "NODE111_TEST_HOUSEKEEPING_MS";

/// The shortest interval the pass knob sets (a stray `1` never spins the loop).
const KNOB_FLOOR: Duration = Duration::from_millis(10);

/// The shortest interval at all (`tokio::time::interval` refuses a zero period).
const MIN_PERIOD: Duration = Duration::from_millis(1);

/// What one capacity pass knows (§4.3): the figures step 1 takes, handed to every later step so
/// that no step takes its own. Plain fields; nothing here is read from disk.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PassCtx {
    /// The `statvfs` figure of step 1: the fabric's (of `fabric/`, the one its `prune` was given)
    /// while it is live, else the pass's own of the root. `None` when `statvfs` failed: the figure
    /// is unknown and, as for the fabric's `DiskSpace::UNKNOWN`, the floor never triggers.
    pub disk: Option<DiskSpace>,
    /// Free space is below the fabric's 1 % floor (`DiskSpace::floor`; §4.2 item 5); false while
    /// the figure is unknown.
    pub disk_low: bool,
    /// The fabric's live bytes after its pass (the `fabric` class, §4.1; `PruneReport::live_after`).
    /// `None` with the fabric down, or when its pass failed: step 3 (`set_watermarks`, which writes
    /// the fabric store's settings) runs only while this is `Some`.
    pub fabric_live_bytes: Option<u64>,
    /// The housekeeping tick count `n` of this pass (the first pass runs at `n = 0`).
    pub tick: u64,
}

/// One call of the step list after step 1 (§4.3), in the order [`pass`] runs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Steps 2 to 4: the budget's figures, `set_watermarks` on a change (step 3, only while the
    /// fabric is live), the reclaim (`storage::budget::on_pass`; row D10).
    Budget,
    /// Step 5: a log over its cap rotates (`storage::logs::on_pass`; row D12).
    Logs,
    /// Step 6: a paused dataset job is checked against its condition (`datasets::job::on_pass`;
    /// row D32).
    Jobs,
    /// Step 7: the node-state store closes after a pass with no use (`state::on_pass`; row D2).
    NodeState,
    /// Step 8: one `storage` frame on a change §16.3 lists (`storage::on_pass_end`; row D14).
    Frame,
    /// Step 9: a kiwix-serve no request reached for 30 passes stops (`datasets::kiwix::on_pass`;
    /// row D42).
    Kiwix,
}

/// A step's call: its module's `on_pass`, handed the pass's figures.
type StepFn = fn(&PassCtx);

/// §4.3 steps 2 to 9, in order, each its module's call (the T0 step list).
const STEPS: [(Step, StepFn); 6] = [
    (Step::Budget, storage::budget::on_pass),
    (Step::Logs, storage::logs::on_pass),
    (Step::Jobs, datasets::job::on_pass),
    (Step::NodeState, state::on_pass),
    (Step::Frame, storage::on_pass_end),
    (Step::Kiwix, datasets::kiwix::on_pass),
];

/// One capacity pass after step 1: §4.3 steps 2 to 9, in order, each handed `ctx` as step 1 left
/// it, whether or not the fabric is live (the budget runs step 3 only while
/// [`PassCtx::fabric_live_bytes`] is `Some`). Each step returns at once until its row lands.
pub fn pass(ctx: &mut PassCtx) {
    for (step, run) in STEPS {
        trace(step, ctx);
        run(ctx);
    }
}

/// The running housekeeping interval: one task ([`spawn`]). Dropping the handle ends it too.
#[must_use = "dropping the handle stops the housekeeping interval"]
#[derive(Debug)]
pub struct Housekeeping {
    task: JoinHandle<()>,
}

impl Housekeeping {
    /// End the interval and wait for its task to be gone, so no tick runs after this returns. The
    /// daemon calls it before the fabric closes, where the fabric's own interval was aborted.
    pub async fn stop(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}

impl Drop for Housekeeping {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Start the one housekeeping interval (§4.3 "The owner"; row D1): called once, from
/// `daemon/mod.rs`, after the fabric's handle exists, live or down. Its period is
/// `FabricConfig.housekeeping` (5 s; [`FabricHandle::housekeeping_period`]), shortened in a debug
/// build by the pass knob; the first tick, at once, runs a pass. The task holds the API state and
/// the fabric handle weakly and ends when the API state is gone or at [`Housekeeping::stop`]; a
/// fabric handle that is gone counts as a fabric that is down.
pub fn spawn(api: &Arc<ApiState>, fabric: &Arc<FabricHandle>) -> Housekeeping {
    let configured = fabric.housekeeping_period();
    let every = period(
        configured,
        knob(std::env::var_os).as_deref(),
        cfg!(debug_assertions),
    );
    #[cfg(debug_assertions)]
    if every != configured.max(MIN_PERIOD) {
        eprintln!(
            "111 daemon: housekeeping every {} ms ({PASS_KNOB}, a debug build)",
            every.as_millis()
        );
    }
    let root = api.layout.root.clone();
    let task = tokio::spawn(run(
        Arc::downgrade(api),
        Arc::downgrade(fabric),
        root,
        every,
    ));
    Housekeeping { task }
}

/// The interval: one `tokio::time::interval`, whose first tick completes at once (`n = 0`).
async fn run(api: Weak<ApiState>, fabric: Weak<FabricHandle>, root: PathBuf, every: Duration) {
    let mut tick = tokio::time::interval(every);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut n: u64 = 0;
    loop {
        tick.tick().await;
        let Some(api_now) = api.upgrade() else {
            return;
        };
        let fabric_now = fabric.upgrade();
        on_tick(n, &api_now, fabric_now.as_deref(), &root).await;
        n = n.wrapping_add(1);
    }
}

/// Tick `n`: every [`PRUNE_EVERY`] ticks, step 1 then [`pass`]; then `refresh_facts` while the
/// fabric is live, on every tick, as the fabric's own interval did. Returns the pass's figures
/// when this tick ran one.
async fn on_tick(
    n: u64,
    api: &ApiState,
    fabric: Option<&FabricHandle>,
    root: &Path,
) -> Option<PassCtx> {
    let live = fabric.filter(|f| f.running());
    let ran = if n.is_multiple_of(PRUNE_EVERY) {
        let mut ctx = step_one(n, api, live, root).await;
        pass(&mut ctx);
        Some(ctx)
    } else {
        None
    };
    if let Some(f) = live {
        f.refresh_facts().await;
    }
    ran
}

/// §4.3 step 1: while the fabric is live its capacity pass as built (`prune` with its `statvfs`
/// figure of `fabric/`), whose figure and live bytes the pass reuses; with it down the pass's own
/// `statvfs` of the root, and no live bytes.
async fn step_one(tick: u64, api: &ApiState, live: Option<&FabricHandle>, root: &Path) -> PassCtx {
    let figures = match live {
        Some(f) => f.capacity_pass(api).await,
        None => None,
    };
    let (disk, fabric_live_bytes) = match figures {
        Some(c) => (c.disk, c.live_bytes),
        None => (fabric::disk_space(root), None),
    };
    PassCtx {
        disk,
        disk_low: disk.is_some_and(|d| d.below_floor()),
        fabric_live_bytes,
        tick,
    }
}

/// The pass knob's raw value as `env` holds it (the process environment, at [`spawn`]): a debug
/// build asks `env` for [`PASS_KNOB`] and nothing else (§18).
#[cfg(debug_assertions)]
fn knob(env: impl FnOnce(&'static str) -> Option<OsString>) -> Option<OsString> {
    env(PASS_KNOB)
}

/// A release build has no pass knob: `env` is never asked, so nothing is read (§18).
#[cfg(not(debug_assertions))]
fn knob(_env: impl FnOnce(&'static str) -> Option<OsString>) -> Option<OsString> {
    None
}

/// The interval's period: `configured` (at least [`MIN_PERIOD`]), shortened to the pass knob's
/// whole milliseconds (at least [`KNOB_FLOOR`]) only in a debug build and only when the knob is a
/// positive number below it. Any other value, or none, leaves `configured`.
fn period(configured: Duration, knob: Option<&OsStr>, debug_build: bool) -> Duration {
    let configured = configured.max(MIN_PERIOD);
    if !debug_build {
        return configured;
    }
    let ms = knob
        .and_then(OsStr::to_str)
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&ms| ms > 0);
    match ms {
        Some(ms) => configured.min(Duration::from_millis(ms).max(KNOB_FLOOR)),
        None => configured,
    }
}

/// What [`pass`] ran: nothing outside a test build.
#[cfg(not(test))]
fn trace(_step: Step, _ctx: &PassCtx) {}

#[cfg(test)]
thread_local! {
    /// Each step [`pass`] ran on this thread, with the figures it was handed (tests only).
    static TRACE: std::cell::RefCell<Vec<(Step, PassCtx)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// What [`pass`] ran, recorded on this thread (tests only).
#[cfg(test)]
fn trace(step: Step, ctx: &PassCtx) {
    TRACE.with(|t| t.borrow_mut().push((step, *ctx)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::dashboard::api_v1::ApiConfig;
    use crate::daemon::dashboard::DashboardState;
    use crate::daemon::fabric::{FabricConfig, TcpTuning, FABRIC_DIR};
    use crate::daemon::DaemonLayout;
    use std::net::{Ipv4Addr, SocketAddr};
    use std::sync::RwLock;

    /// §4.3's order of the steps after step 1.
    const ORDER: [Step; 6] = [
        Step::Budget,
        Step::Logs,
        Step::Jobs,
        Step::NodeState,
        Step::Frame,
        Step::Kiwix,
    ];

    fn test_api(dir: &Path) -> Arc<ApiState> {
        std::fs::create_dir_all(dir.join("state")).unwrap();
        let layout = DaemonLayout {
            root: dir.to_path_buf(),
            etc: dir.join("etc"),
        };
        let mut cfg = ApiConfig::new(layout, false);
        cfg.persist_auth = false;
        ApiState::new(Arc::new(RwLock::new(DashboardState::new("t"))), cfg)
    }

    fn test_cfg(dir: &Path, every: Duration) -> FabricConfig {
        FabricConfig {
            fabric_dir: dir.join(FABRIC_DIR),
            identity_dir: dir.join("identity"),
            state_dir: dir.join("state"),
            listen: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            tcp: TcpTuning::default(),
            hub_default: false,
            enroll_timeout: Duration::from_secs(5),
            housekeeping: every,
            shutdown_grace: Duration::from_secs(5),
            offer_host: None,
        }
    }

    /// A fabric handle that is down (no identity: `fabric::start` boots nothing).
    async fn down(dir: &Path, api: &Arc<ApiState>, every: Duration) -> Arc<FabricHandle> {
        let h = fabric::start(test_cfg(dir, every), None, api).await;
        assert!(!h.running(), "no identity, no fabric");
        h
    }

    /// A live fabric handle.
    async fn live(dir: &Path, api: &Arc<ApiState>, every: Duration) -> Arc<FabricHandle> {
        let identity = crate::identity::generate(&dir.join("identity")).unwrap();
        let h = fabric::start(test_cfg(dir, every), Some(identity), api).await;
        assert!(h.running(), "{:?}", h.down_reason());
        h
    }

    /// The trace this thread recorded since the last take.
    fn take_trace() -> Vec<(Step, PassCtx)> {
        TRACE.with(|t| std::mem::take(&mut *t.borrow_mut()))
    }

    fn steps(t: &[(Step, PassCtx)]) -> Vec<Step> {
        t.iter().map(|(s, _)| *s).collect()
    }

    /// The tick of each pass in `t` (one per `Budget` step: a pass's first call).
    fn pass_ticks(t: &[(Step, PassCtx)]) -> Vec<u64> {
        t.iter()
            .filter(|(s, _)| *s == Step::Budget)
            .map(|(_, c)| c.tick)
            .collect()
    }

    /// Wait (yielding to this thread's tasks) until the trace holds `passes` passes.
    async fn until_passes(passes: usize) -> Vec<(Step, PassCtx)> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        let mut got = Vec::new();
        loop {
            got.extend(take_trace());
            if pass_ticks(&got).len() >= passes {
                return got;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{} of {passes} passes: {got:?}",
                pass_ticks(&got).len()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Frames named `event` published since `head`.
    fn frames(api: &ApiState, head: u64, event: &str) -> usize {
        api.events
            .subscribe(Some(head))
            .replay
            .iter()
            .filter(|f| f.event == event)
            .count()
    }

    #[test]
    fn the_steps_run_in_section_4_3_order_each_handed_the_same_figures() {
        assert_eq!(
            STEPS.map(|(s, _)| s),
            ORDER,
            "the T0 step list, §4.3 steps 2 to 9"
        );
        let mut ctx = PassCtx {
            disk: Some(DiskSpace {
                free: 5,
                total: 1000,
            }),
            disk_low: true,
            fabric_live_bytes: Some(7),
            tick: 24,
        };
        let before = ctx;
        pass(&mut ctx);
        let t = take_trace();
        assert_eq!(steps(&t), ORDER, "every step, once, in order");
        assert!(t.iter().all(|(_, c)| *c == before), "{t:?}");
        assert_eq!(ctx, before, "no step changes the figures today");
    }

    /// Fabric down: step 1 is the pass's own `statvfs` of the root, no live bytes (so the budget
    /// skips step 3), and every other step runs; a pass every `PRUNE_EVERY` ticks from `n = 0`,
    /// nothing in between (no facts to refresh).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn with_the_fabric_down_a_pass_takes_its_own_statvfs_and_runs_every_other_step() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let api = test_api(root);
        let h = down(root, &api, Duration::from_secs(3600)).await;
        let own = fabric::disk_space(root).expect("statvfs of the root");

        let ctx = on_tick(0, &api, Some(&h), root)
            .await
            .expect("the first pass at n = 0");
        assert_eq!(ctx.tick, 0);
        assert_eq!(ctx.disk.map(|d| d.total), Some(own.total), "the root's");
        assert_eq!(ctx.disk_low, ctx.disk.unwrap().below_floor());
        assert_eq!(ctx.fabric_live_bytes, None, "down: step 3 waits");
        let t = take_trace();
        assert_eq!(steps(&t), ORDER, "steps 2 and 4 to 9 run while down");
        assert!(t.iter().all(|(_, c)| *c == ctx));

        for n in 1..PRUNE_EVERY {
            assert_eq!(on_tick(n, &api, Some(&h), root).await, None, "tick {n}");
        }
        assert!(take_trace().is_empty(), "no pass between the twelfth ticks");
        let ctx = on_tick(PRUNE_EVERY, &api, Some(&h), root).await.unwrap();
        assert_eq!(ctx.tick, PRUNE_EVERY);
        assert_eq!(ctx.fabric_live_bytes, None);
        assert_eq!(steps(&take_trace()), ORDER);

        // no fabric handle at all reads as a fabric that is down
        let ctx = on_tick(2 * PRUNE_EVERY, &api, None, root).await.unwrap();
        assert_eq!(ctx.disk.map(|d| d.total), Some(own.total));
        assert_eq!(ctx.fabric_live_bytes, None);
        assert_eq!(steps(&take_trace()), ORDER);
        // a root `statvfs` cannot read: unknown, and the floor never triggers
        let gone = root.join("not-here");
        let ctx = on_tick(0, &api, None, &gone).await.unwrap();
        assert_eq!((ctx.disk, ctx.disk_low), (None, false));
        take_trace();
    }

    /// Fabric live: step 1 is its capacity pass as built (the figure of `fabric/`, the live bytes
    /// after it, so the budget may run step 3), the steps follow in order, and `refresh_facts`
    /// runs on every tick, a pass's or not.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn with_the_fabric_live_step_one_is_its_capacity_pass_and_the_facts_refresh_every_tick() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let api = test_api(root);
        let h = live(root, &api, Duration::from_secs(3600)).await;
        let fabric_disk = fabric::disk_space(&root.join(FABRIC_DIR)).unwrap();

        let head = api.events.subscribe(None).head;
        let at0 = h.pair_views().at.expect("start read the status once");
        let ctx = on_tick(0, &api, Some(&h), root)
            .await
            .expect("the first pass at n = 0");
        let at1 = h.pair_views().at.unwrap();
        assert_eq!(ctx.tick, 0);
        assert_eq!(ctx.disk.map(|d| d.total), Some(fabric_disk.total));
        assert_eq!(ctx.disk_low, ctx.disk.unwrap().below_floor());
        let held = h
            .fabric()
            .unwrap()
            .with_store(|s| s.live_bytes().unwrap())
            .await
            .unwrap();
        assert_eq!(ctx.fabric_live_bytes, Some(held), "live: step 3 may run");
        let t = take_trace();
        assert_eq!(steps(&t), ORDER);
        assert!(t.iter().all(|(_, c)| *c == ctx));
        assert_eq!(frames(&api, head, "fabric"), 0, "no frame read the status");
        assert!(at1 > at0, "refresh_facts on the pass's tick");

        // a tick between passes: no pass, the facts refreshed as before
        assert_eq!(on_tick(1, &api, Some(&h), root).await, None);
        assert!(take_trace().is_empty());
        let at2 = h.pair_views().at.unwrap();
        assert!(at2 > at1, "refresh_facts on every tick while live");
        assert_eq!(frames(&api, head, "fabric"), 0);
        h.shutdown().await;
    }

    /// The interval itself, with the fabric down: one task whose first tick runs a pass at once
    /// (`n = 0`, long before the period), and `stop` ends it. Current-thread runtime: the task
    /// runs on this thread, so the trace sees it.
    #[tokio::test]
    async fn the_interval_runs_with_the_fabric_down_its_first_pass_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let api = test_api(dir.path());
        let h = down(dir.path(), &api, Duration::from_secs(3600)).await;
        let house = spawn(&api, &h);
        let t = until_passes(1).await;
        assert_eq!(pass_ticks(&t), [0], "at once, not a period later");
        assert_eq!(steps(&t), ORDER);
        house.stop().await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(take_trace().is_empty());
    }

    /// The interval at a short period, with the fabric down and with it live: one pass every
    /// `PRUNE_EVERY` ticks from `n = 0`, the live one carrying the fabric's figures; after `stop`
    /// no tick runs.
    #[tokio::test]
    async fn the_interval_passes_every_twelfth_tick_with_the_fabric_down_and_live() {
        for is_live in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let api = test_api(dir.path());
            let every = Duration::from_millis(10);
            let h = if is_live {
                live(dir.path(), &api, every).await
            } else {
                down(dir.path(), &api, every).await
            };
            let house = spawn(&api, &h);
            let t = until_passes(3).await;
            assert_eq!(pass_ticks(&t)[..3], [0, 12, 24], "live: {is_live}");
            assert!(
                t.iter()
                    .all(|(_, c)| c.fabric_live_bytes.is_some() == is_live),
                "live: {is_live}: {t:?}"
            );
            house.stop().await;
            take_trace();
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(
                take_trace().is_empty(),
                "stopped: no tick (live: {is_live})"
            );
            h.shutdown().await;
        }
    }

    /// The interval ends with the API state (the daemon's lifetime), whatever the fabric.
    #[tokio::test]
    async fn the_interval_ends_with_the_api_state() {
        let dir = tempfile::tempdir().unwrap();
        let api = test_api(dir.path());
        let h = down(dir.path(), &api, Duration::from_millis(10)).await;
        let house = spawn(&api, &h);
        until_passes(1).await;
        drop(api);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !house.task.is_finished() {
            assert!(tokio::time::Instant::now() < deadline, "still ticking");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        take_trace();
    }

    /// §18 "The pass knob": a debug build shortens the interval to the knob's milliseconds (never
    /// below 10 ms, never longer than configured); a release build has no such setting.
    #[test]
    fn the_pass_knob_shortens_the_interval_in_a_debug_build_only() {
        let five = Duration::from_secs(5);
        let knob = |v: &'static str| Some(OsStr::new(v));
        assert_eq!(period(five, knob("200"), true), Duration::from_millis(200));
        assert_eq!(
            period(five, knob(" 200\n"), true),
            Duration::from_millis(200)
        );
        assert_eq!(
            period(five, knob("200"), false),
            five,
            "ignored outside a debug build"
        );
        assert_eq!(period(five, None, true), five);
        assert_eq!(period(five, knob("60000"), true), five, "it only shortens");
        assert_eq!(period(five, knob("1"), true), KNOB_FLOOR);
        for bad in ["", " ", "0", "-5", "1.5", "200ms", "0x10"] {
            assert_eq!(period(five, knob(bad), true), five, "{bad:?} is ignored");
        }
        let not_utf8 = std::os::unix::ffi::OsStrExt::from_bytes(b"2\xff0");
        assert_eq!(period(five, Some(not_utf8), true), five);
        assert_eq!(
            period(Duration::ZERO, None, false),
            MIN_PERIOD,
            "never zero"
        );
    }

    /// §18 "The pass knob", the read itself: a debug build asks the environment for
    /// `NODE111_TEST_HOUSEKEEPING_MS` and nothing else; a release build asks it nothing, whatever
    /// it holds, and keeps the configured period. Compiled with debug assertions on and off, each
    /// build checking its own half (the name is spelt out: a release build has no `PASS_KNOB`).
    #[test]
    fn the_pass_knob_is_read_by_a_debug_build_only() {
        let mut asked = Vec::new();
        let got = knob(|name| {
            asked.push(name);
            Some(OsString::from("200"))
        });
        let five = Duration::from_secs(5);
        let every = period(five, got.as_deref(), cfg!(debug_assertions));
        if cfg!(debug_assertions) {
            assert_eq!(asked, ["NODE111_TEST_HOUSEKEEPING_MS"], "the knob, once");
            assert_eq!(got.as_deref(), Some(OsStr::new("200")));
            assert_eq!(every, Duration::from_millis(200));
        } else {
            assert!(asked.is_empty(), "a release build asks nothing: {asked:?}");
            assert_eq!(got, None, "a release build has no such setting");
            assert_eq!(every, five);
        }
    }
}
