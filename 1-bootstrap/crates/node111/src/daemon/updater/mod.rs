//! Update client: fetch → verify (minisig + floor) → stage into `state/staging/` →
//! drop `state/update-request.json`. The daemon CANNOT write `releases/` or flip
//! `current` — the privileged root helper is the only flip actor (D12).
//!
//! Layer 7 §9.2–§9.3: every check runs under `state/staging.lock`
//! ([`stage::StagingLock`]); the hourly loop waits for it inside its
//! `spawn_blocking`, and checks the internet bases only while
//! [`fetch::UpdaterConfig::check_internet`] is on (the loop and its hourly sleep
//! stay as they are: no timer is added, D30 Q2).
//!
//! Layer 7 row L4 (§9.1–§9.3, T4, T5):
//! - **`updates.check_internet`** is read from the relay settings each round
//!   ([`Hooks::check_internet`], the daemon's `relay7::cell`): off ⇒ the round
//!   contacts no internet base.
//! - **The home hub's origin** is checked once per rising edge of the fabric link
//!   to it attributing, and only while its announce carries `ROLE_ORIGIN`
//!   ([`hub`]); no gossip, no timer.
//! - **`111 update --from <dir|url>`** ([`from_origin`], [`update_from`]): one
//!   check against that origin alone under a non-blocking take of the staging
//!   lock (held ⇒ exit 75); it writes only `state/staging/<ver>/` and
//!   `state/update-request.json` (T5), plus `state/origin/` on a serving hub. It
//!   runs only as the node user, the owner of `state/` ([`runs_as_node_user`]:
//!   any other user, root included, exits 2 before anything is written).
//! - **Foreign text** never reaches a log, the terminal or the per-origin record
//!   raw: a refused manifest is recorded and shown as one fixed class
//!   ([`fetch::UNVERIFIED`]), other details are [`fetch::display_safe`].
//! - **A serving hub** admits each release it stages into `state/origin/` (D7's
//!   `Origin::admit`, [`admit_staged`]) and then calls `relay::changed()`.

pub mod fetch;
pub mod hub;
pub mod stage;

use crate::daemon::dashboard::{ApiState, SharedState};
use crate::daemon::origin::{AdmitError, Admitted, Origin as OriginStore};
use crate::daemon::watchdog;
use crate::daemon::DaemonLayout;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

/// The minisign verify key compiled into the binary (keys/manifest-pub.key —
/// DEV key in this tree; release machinery substitutes the real one, T15).
pub const MANIFEST_PUBKEY: &str = include_str!("../../../../../keys/manifest-pub.key");

/// Poll interval and failure backoff (exp + jitter, capped).
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(60 * 60); // 1h
pub const MAX_BACKOFF: Duration = Duration::from_secs(24 * 60 * 60); // 24h cap

/// Result of the last check this daemon lifetime (`/api/v1/status`
/// `update.last_result`; Layer 4 R2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LastResult {
    UpToDate,
    Staged,
    Refused,
    /// Fetch/clock problems and check errors.
    Degraded,
}

/// Live updater facts the dashboard reads (Layer 4 R2): whether a check is
/// running right now, the last outcome, and a completed-check counter.
#[derive(Debug, Default)]
pub struct UpdaterStatus {
    checking: AtomicBool,
    last_result: Mutex<Option<LastResult>>,
    checks: AtomicU64,
}

impl UpdaterStatus {
    pub fn checking(&self) -> bool {
        self.checking.load(Ordering::SeqCst)
    }

    /// Mark a check as running; returns the previous value. `POST
    /// /api/v1/update/check` uses it so a request during a running check
    /// coalesces with it instead of queueing a second one.
    pub fn begin_check(&self) -> bool {
        self.checking.swap(true, Ordering::SeqCst)
    }

    pub fn last_result(&self) -> Option<LastResult> {
        *self.last_result.lock().unwrap()
    }

    /// Checks completed this lifetime.
    pub fn checks(&self) -> u64 {
        self.checks.load(Ordering::SeqCst)
    }

    /// A woken loop with no origin to check (`check_internet` off, Layer 7
    /// §9.3): the dashboard's `begin_check` is cleared without an outcome, and
    /// nothing was contacted.
    pub fn settle(&self) {
        self.checking.store(false, Ordering::SeqCst);
    }

    /// Record a finished check (the loop calls it; public so tests can set
    /// an outcome without a network round).
    pub fn finish(&self, result: LastResult) {
        *self.last_result.lock().unwrap() = Some(result);
        self.checks.fetch_add(1, Ordering::SeqCst);
        self.checking.store(false, Ordering::SeqCst);
    }
}

/// `updates.check_internet` as the relay settings hold it now (§9.3).
pub type InternetSwitch = Arc<dyn Fn() -> bool + Send + Sync>;

/// What runs after a stage ([`fetch::AfterStage`]), owned.
pub type AfterStageHook = Arc<dyn Fn(&fetch::Staged<'_>) + Send + Sync>;

/// Dashboard hooks into the loop: `wake` (= `ApiState::update_check`) cuts the
/// current sleep short; `status` is what `/api/v1/status.update` reads.
#[derive(Clone, Default)]
pub struct Hooks {
    pub wake: Arc<Notify>,
    pub status: Arc<UpdaterStatus>,
}

/// Layer 7's hooks into the loop (row L4; [`run_with_relay`]).
#[derive(Clone, Default)]
pub struct RelayHooks {
    /// Read at the start of every round (§9.3): `false` ⇒ the round contacts no
    /// internet base. `None` ⇒ `UpdaterConfig::check_internet` as given.
    pub check_internet: Option<InternetSwitch>,
    /// Run under the staging lock after a stage (§9.1: a serving hub's admission).
    pub after_stage: Option<AfterStageHook>,
}

impl RelayHooks {
    fn internet(&self, cfg: &fetch::UpdaterConfig) -> bool {
        self.check_internet
            .as_ref()
            .map_or(cfg.check_internet, |f| f())
    }
}

/// The borrowed form of an owned after-stage hook.
pub fn after_ref(hook: &Option<AfterStageHook>) -> Option<fetch::AfterStage<'_>> {
    match hook {
        Some(f) => Some(&**f),
        None => None,
    }
}

/// Updater loop. Owns the dashboard's update fields (last-successful-check,
/// pending, clock-fault). Runs until the daemon exits; never panics on
/// transient failures.
pub async fn run(layout: DaemonLayout, state: SharedState, cfg: fetch::UpdaterConfig) {
    run_with_hooks(layout, state, cfg, Hooks::default()).await
}

/// [`run`] with the dashboard hooks: sleeps between checks end early when
/// `hooks.wake` is notified (`POST /api/v1/update/check`).
pub async fn run_with_hooks(
    layout: DaemonLayout,
    state: SharedState,
    cfg: fetch::UpdaterConfig,
    hooks: Hooks,
) {
    run_with_relay(layout, state, cfg, hooks, RelayHooks::default()).await
}

/// [`run_with_hooks`] with Layer 7's hooks (row L4): `updates.check_internet` read
/// each round, and the after-stage hand-over of a serving hub.
pub async fn run_with_relay(
    layout: DaemonLayout,
    state: SharedState,
    cfg: fetch::UpdaterConfig,
    hooks: Hooks,
    relay: RelayHooks,
) {
    let mut backoff = Duration::from_secs(60);
    loop {
        // `check_internet` off (§9.3): the internet bases are not contacted; the
        // loop sleeps as it always does, and a dashboard wake settles at once.
        // Read each round (row L4: the relay settings, `state/relay.json`).
        let internet = relay.internet(&cfg);
        if !internet {
            hooks.status.settle();
            tokio::select! {
                _ = tokio::time::sleep(DEFAULT_INTERVAL) => {}
                _ = hooks.wake.notified() => {
                    eprintln!("111 updater: check requested; internet checks are off");
                }
            }
            continue;
        }
        hooks.status.begin_check();
        // Extend the watchdog during the fetch window; restore right after.
        let _ = watchdog::extend(10 * 60 * 1_000_000);
        // The check waits for `state/staging.lock` here, inside the blocking task
        // (a `111 update --from` run may hold it, §9.2).
        let outcome = tokio::task::spawn_blocking({
            let layout = layout.clone();
            let cfg = fetch::UpdaterConfig {
                check_internet: internet,
                ..cfg.clone()
            };
            let after = relay.after_stage.clone();
            move || fetch::check_once_with(&cfg, &layout, after_ref(&after))
        })
        .await;
        let _ = watchdog::restore(30 * 1_000_000);
        hooks.status.finish(match &outcome {
            Ok(Ok(fetch::FetchOutcome::UpToDate)) => LastResult::UpToDate,
            Ok(Ok(
                fetch::FetchOutcome::Staged { .. } | fetch::FetchOutcome::AlreadyStaged { .. },
            )) => LastResult::Staged,
            Ok(Ok(fetch::FetchOutcome::Refused { .. })) => LastResult::Refused,
            _ => LastResult::Degraded,
        });

        let sleep_for = match outcome {
            Ok(Ok(fetch::FetchOutcome::UpToDate)) => {
                state.write().unwrap().clock_fault = false;
                backoff = Duration::from_secs(60);
                DEFAULT_INTERVAL
            }
            Ok(Ok(
                fetch::FetchOutcome::Staged { version }
                | fetch::FetchOutcome::AlreadyStaged { version },
            )) => {
                eprintln!("111 updater: staged {version}, root helper notified");
                state.write().unwrap().update_pending = true;
                backoff = Duration::from_secs(60);
                DEFAULT_INTERVAL
            }
            Ok(Ok(fetch::FetchOutcome::Refused { reason })) => {
                eprintln!("111 updater: update refused: {reason}");
                backoff = jittered(backoff.saturating_mul(2).min(MAX_BACKOFF));
                backoff
            }
            Ok(Ok(fetch::FetchOutcome::Degraded {
                reason,
                clock_fault,
            })) => {
                eprintln!("111 updater: degraded: {reason}");
                state.write().unwrap().clock_fault = clock_fault;
                backoff = jittered(backoff.saturating_mul(2).min(MAX_BACKOFF));
                backoff
            }
            // A waiting take never answers Held, and the list is not empty while
            // `check_internet` is on; both read as "nothing done this round".
            Ok(Ok(fetch::FetchOutcome::Held | fetch::FetchOutcome::NoOrigin)) => DEFAULT_INTERVAL,
            Ok(Err(e)) => {
                eprintln!("111 updater: check failed: {e:#}");
                backoff = jittered(backoff.saturating_mul(2).min(MAX_BACKOFF));
                backoff
            }
            Err(join_err) => {
                eprintln!("111 updater: task join error: {join_err}");
                jittered(Duration::from_secs(300))
            }
        };

        tokio::select! {
            _ = tokio::time::sleep(sleep_for) => {}
            _ = hooks.wake.notified() => {
                eprintln!("111 updater: check requested from the dashboard");
            }
        }
    }
}

fn jittered(d: Duration) -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.subsec_nanos() as u64)
        .unwrap_or(0);
    let pct = 10 + (nanos % 21); // +10–30% jitter
    d.saturating_mul(100 + pct as u32) / 100
}

// ---------------------------------------------------------------- row L4

/// A serving hub's hand-over after a stage (§9.1, row L4): the verified manifest, its signature
/// and the pin-checked payload go into `state/origin/` (D7's [`OriginStore::admit`]), with the
/// blobs a directory origin holds for the other targets of `origin.targets`. Blocking (hashing).
pub fn admit_staged(
    store: &OriginStore,
    state_dir: &Path,
    staged: &fetch::Staged<'_>,
) -> Result<Admitted, AdmitError> {
    let targets = crate::daemon::relay::settings::load(state_dir)
        .settings
        .origin_targets()
        .to_vec();
    let mut blobs: Vec<PathBuf> = vec![staged.payload.to_path_buf()];
    blobs.extend(staged.other_blobs(&targets));
    let refs: Vec<&Path> = blobs.iter().map(PathBuf::as_path).collect();
    store.admit(staged.manifest_bytes, staged.signature, &refs)
}

fn report_admission(result: Result<Admitted, AdmitError>) {
    match result {
        Ok(a) => {
            let missing = if a.missing.is_empty() {
                String::new()
            } else {
                format!("; not held: {}", a.missing.join(", "))
            };
            eprintln!(
                "111 updater: the origin serves {} ({}){missing}",
                a.release,
                a.held.join(", ")
            );
        }
        // log-safe by construction (D7: fixed reasons, a calver, a path)
        Err(e) => eprintln!("111 updater: the origin keeps what it held: {e}"),
    }
}

/// The daemon's after-stage hook (row L4): while this hub serves the origin (§9.1: hub role, LAN
/// sharing, `origin.serve` consented; `origin::serving`), admit the staged release, then
/// publish the `relay` frame (`origin.bytes` moved). Nothing while it does not serve.
pub fn daemon_after_stage(api: Arc<ApiState>) -> AfterStageHook {
    Arc::new(move |staged: &fetch::Staged<'_>| {
        if !crate::daemon::origin::serving(&api) {
            return;
        }
        let store = OriginStore::shared(&api);
        report_admission(admit_staged(&store, &api.layout.state_dir(), staged));
        crate::daemon::relay::changed();
    })
}

/// `111 update --from`'s after-stage hook (row L4): a second process, so it cannot read the
/// daemon's live gate; it admits only while `state/relay.json` has `origin.serve` on with the
/// consent owed now (the settings route turns it on only for a LAN-shared hub, D10). The
/// daemon's `Origin::bytes` sees the write by its directory stamps; no frame is sent from here.
pub fn verb_after_stage(state_dir: PathBuf) -> AfterStageHook {
    Arc::new(move |staged: &fetch::Staged<'_>| {
        let s = crate::daemon::relay::settings::load(&state_dir).settings;
        let roles =
            crate::daemon::fabric::RelayRoles::from_settings(&s, crate::config::on_termux());
        if !roles.origin_serve {
            return;
        }
        report_admission(admit_staged(
            &OriginStore::new(&state_dir),
            &state_dir,
            staged,
        ));
    })
}

/// The home hub's check (row L4, §9.3), run by [`hub::spawn`] on a rising edge of the link to a
/// hub whose announce carries `ROLE_ORIGIN`: the one origin, waiting for the staging lock; a
/// stage marks the update pending as the hourly loop does. Blocking.
pub fn hub_check(
    cfg: fetch::UpdaterConfig,
    layout: DaemonLayout,
    state: SharedState,
    after: Option<AfterStageHook>,
) -> Arc<dyn Fn(fetch::Origin) + Send + Sync> {
    Arc::new(move |origin: fetch::Origin| {
        let label = origin.label();
        let pending = || {
            state
                .write()
                .unwrap_or_else(|p| p.into_inner())
                .update_pending = true;
        };
        // Every text logged here is log-safe: a refused manifest's reason is the fixed class
        // (`fetch::UNVERIFIED`), the others are this node's own, escaped and capped.
        match fetch::check_origins_with(
            &cfg,
            &layout,
            &[origin],
            stage::LockWait::Wait,
            after_ref(&after),
        ) {
            Ok(fetch::FetchOutcome::Staged { version }) => {
                eprintln!("111 updater: staged {version} from the home hub, root helper notified");
                pending();
            }
            // requested already (a re-attribution): nothing fetched, written or logged again
            Ok(fetch::FetchOutcome::AlreadyStaged { .. }) => pending(),
            Ok(fetch::FetchOutcome::UpToDate) => {}
            Ok(fetch::FetchOutcome::Refused { reason }) => {
                eprintln!("111 updater: the home hub's release refused: {reason}")
            }
            Ok(fetch::FetchOutcome::Degraded { reason, .. }) => {
                eprintln!("111 updater: the home hub's origin: {reason}")
            }
            Ok(fetch::FetchOutcome::Held | fetch::FetchOutcome::NoOrigin) => {}
            Err(e) => eprintln!(
                "111 updater: the home hub's origin {label}: {}",
                fetch::error_detail(&e)
            ),
        }
    })
}

/// Longest `--from` argument read.
const FROM_MAX: usize = 2048;

/// The origin of `111 update --from <dir|url>` (§9.2), read from the argument alone (a usage
/// error is the argument's): an `http://` or `https://` URL the user typed (fetched under
/// `HttpException::UserTypedFrom` when plain), or a directory path (made absolute; whether it
/// can be read now is [`update_from`]'s question, not a usage error). A URL that carries a user
/// or password is refused: the origin is shown by `GET /api/v1/update/origins`, and a secret is
/// never echoed.
pub fn from_origin(arg: &str) -> anyhow::Result<fetch::Origin> {
    anyhow::ensure!(
        !arg.is_empty() && arg.len() <= FROM_MAX,
        "--from needs a directory or a URL (at most {FROM_MAX} bytes)"
    );
    anyhow::ensure!(
        !arg.chars().any(|c| c.is_control() || c.is_whitespace()),
        "--from: no spaces or control characters"
    );
    let lower = arg.to_ascii_lowercase();
    if let Some(rest) = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
    {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        anyhow::ensure!(!authority.is_empty(), "--from: the URL names no host");
        anyhow::ensure!(
            !authority.contains('@'),
            "--from: a URL with a user or password is refused (it would be shown and stored)"
        );
        anyhow::ensure!(
            !rest.contains(['?', '#']),
            "--from: the URL is the origin's base (no query or fragment)"
        );
        return Ok(fetch::Origin::Url {
            base: arg.to_string(),
        });
    }
    anyhow::ensure!(
        !lower.contains("://"),
        "--from: only http:// and https:// URLs, or a directory"
    );
    let path = std::path::absolute(arg).map_err(|e| anyhow::anyhow!("--from {arg}: {e}"))?;
    Ok(fetch::Origin::Dir { path })
}

/// `111 update --from` run by a user other than the owner of `state/` (§9.2: the verb is a
/// oneshot run as the node user). The verb exits 2 on it, before anything is read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotNodeUser {
    /// The node's `state/`.
    pub state_dir: PathBuf,
    /// Its owner (the node user).
    pub owner: u32,
    /// The effective uid of this run.
    pub euid: u32,
}

impl std::fmt::Display for NotNodeUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "run this as the node user, the owner of {} (uid {}), not as uid {}: \
             sudo -u node111 111 update --from <directory|URL>",
            self.state_dir.display(),
            self.owner,
            self.euid
        )
    }
}

impl std::error::Error for NotNodeUser {}

/// §9.2: `111 update --from` runs as the node user, the owner of `state/` (the `node111` user
/// on a host; the invoking user under Termux or `$NODE111_ROOT`), so every file it writes (the
/// staging tree, the lock, the request, the per-origin record, `state/origin/`) stays one the
/// daemon can replace. Any other effective uid `euid`, root above all, is refused
/// ([`NotNodeUser`]); an unreadable `state/` is an error of its own.
pub fn runs_as_node_user(state_dir: &Path, euid: u32) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(state_dir).map_err(|e| {
        anyhow::anyhow!(
            "the node's state directory {} cannot be read: {e}",
            state_dir.display()
        )
    })?;
    let owner = meta.uid();
    if owner != euid {
        return Err(NotNodeUser {
            state_dir: state_dir.to_path_buf(),
            owner,
            euid,
        }
        .into());
    }
    Ok(())
}

/// A `--from` directory as it is now (§9.2): it must exist and be a directory. Otherwise this
/// origin cannot be read, which is an answer, not a usage error: the verb exits 1, as for a URL
/// that does not answer (an unmounted stick, a mount the node user cannot traverse).
pub fn readable_dir(path: &Path) -> anyhow::Result<()> {
    let meta = std::fs::metadata(path).map_err(|e| {
        anyhow::anyhow!(
            "--from {}: this directory cannot be read: {e}",
            path.display()
        )
    })?;
    anyhow::ensure!(meta.is_dir(), "--from {}: not a directory", path.display());
    Ok(())
}

/// `111 update --from` (§9.2, T5): one check against `origin` alone, the staging lock taken
/// without waiting ([`fetch::FetchOutcome::Held`] when the daemon holds it), the running release
/// read from `current`. Refused first, before anything is read or written, unless this process
/// runs as the node user ([`runs_as_node_user`]); a directory that cannot be read is an error
/// ([`readable_dir`]). Writes only `state/staging/<ver>/`, `state/update-request.json`, the
/// per-origin record and, on a serving hub, `state/origin/`.
pub fn update_from(
    layout: &DaemonLayout,
    origin: fetch::Origin,
) -> anyhow::Result<fetch::FetchOutcome> {
    // SAFETY: geteuid(2) cannot fail and touches no memory.
    runs_as_node_user(&layout.state_dir(), unsafe { libc::geteuid() })?;
    if let fetch::Origin::Dir { path } = &origin {
        readable_dir(path)?;
    }
    let running = crate::update_helper::current_version(layout)
        .ok_or_else(|| anyhow::anyhow!("the installed release is unknown (no `current` link)"))?;
    let running = running
        .parse()
        .map_err(|_| anyhow::anyhow!("the installed release {running:?} is not a calver"))?;
    let cfg = fetch::UpdaterConfig::production(running);
    let after = Some(verb_after_stage(layout.state_dir()));
    fetch::check_origins_with(
        &cfg,
        layout,
        &[origin],
        stage::LockWait::NoWait,
        after_ref(&after),
    )
}

/// `EX_TEMPFAIL`: `111 update --from` while another run holds the staging lock (§9.2).
pub const EXIT_HELD: i32 = 75;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_origin_reads_a_url_or_a_directory() {
        assert_eq!(
            from_origin("http://10.0.0.2:11180/origin/v1/").unwrap(),
            fetch::Origin::Url {
                base: "http://10.0.0.2:11180/origin/v1/".into()
            }
        );
        assert!(matches!(
            from_origin("HTTPS://example.org/r").unwrap(),
            fetch::Origin::Url { .. }
        ));
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            from_origin(dir.path().to_str().unwrap()).unwrap(),
            fetch::Origin::Dir {
                path: dir.path().to_path_buf()
            }
        );
        for bad in [
            "",
            "http://",
            "http://user:secret@10.0.0.2/",
            "https://a@b/",
            "http://h/x?y",
            "ftp://h/",
            "file:///tmp",
            "a b",
        ] {
            let e = from_origin(bad).unwrap_err();
            assert!(!format!("{e:#}").contains("secret"), "{bad:?}: {e:#}");
        }
        assert!(from_origin(&"a".repeat(FROM_MAX + 1)).is_err());
        // the argument alone: a path that cannot be read now is no usage error
        assert_eq!(
            from_origin("/no/such/dir/111").unwrap(),
            fetch::Origin::Dir {
                path: "/no/such/dir/111".into()
            }
        );
    }

    /// Review of row L4: a directory that cannot be read is this origin's answer (exit 1).
    #[test]
    fn a_directory_is_read_when_the_verb_runs() {
        let dir = tempfile::tempdir().unwrap();
        readable_dir(dir.path()).unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        let e = readable_dir(&file).unwrap_err();
        assert!(format!("{e:#}").contains("not a directory"), "{e:#}");
        let e = readable_dir(Path::new("/no/such/dir/111")).unwrap_err();
        assert!(format!("{e:#}").contains("cannot be read"), "{e:#}");
    }

    /// Review of row L4 (§9.2): only the owner of `state/` runs the verb; root is no exception.
    #[test]
    fn only_the_owner_of_state_runs_the_verb() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let owner = std::fs::metadata(dir.path()).unwrap().uid();
        runs_as_node_user(dir.path(), owner).unwrap();
        let other = owner.wrapping_add(1);
        let e = runs_as_node_user(dir.path(), other).unwrap_err();
        let refused = e.downcast_ref::<NotNodeUser>().expect("a typed refusal");
        assert_eq!(
            refused,
            &NotNodeUser {
                state_dir: dir.path().to_path_buf(),
                owner,
                euid: other
            }
        );
        assert!(e.to_string().contains("sudo -u node111"), "{e}");
        // root runs it against a node111-owned state/: refused (chown needs root)
        if owner == 0 {
            std::os::unix::fs::chown(dir.path(), Some(4242), Some(4242)).unwrap();
            let e = runs_as_node_user(dir.path(), 0).unwrap_err();
            assert!(e.downcast_ref::<NotNodeUser>().is_some(), "{e:#}");
        }
        // no state/: an error, not a refusal of the user
        let e = runs_as_node_user(&dir.path().join("missing"), owner).unwrap_err();
        assert!(e.downcast_ref::<NotNodeUser>().is_none(), "{e:#}");
    }

    #[test]
    fn the_switch_is_read_each_round() {
        let on = Arc::new(AtomicBool::new(false));
        let hooks = RelayHooks {
            check_internet: Some({
                let on = Arc::clone(&on);
                Arc::new(move || on.load(Ordering::SeqCst))
            }),
            ..RelayHooks::default()
        };
        let cfg = fetch::UpdaterConfig::production("2026.10.01".parse().unwrap());
        assert!(!hooks.internet(&cfg));
        on.store(true, Ordering::SeqCst);
        assert!(hooks.internet(&cfg));
        assert!(
            RelayHooks::default().internet(&cfg),
            "no switch: the config"
        );
    }
}
