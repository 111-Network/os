//! Layer 7 row L4 (`os/7-relay/README.md` §9.1–§9.3, §9.5 T4, T5): the hub-origin client and
//! `111 update --from`.
//!
//! - The home hub's origin is asked only on a rising edge of the link to it attributing, and
//!   only while its announce carries `ROLE_ORIGIN` (`updater::hub::HubWatch`, driven here by a
//!   fake of the fabric reads; the check is `updater::hub_check` against a logging server).
//! - `111 update --from <dir>` (the real binary, `$NODE111_ROOT`): stages from the directory and
//!   writes only `state/staging/<ver>/`, `state/update-request.json`, the per-origin record and
//!   the lock file (T5); a held `state/staging.lock` exits 75 with "the daemon is checking now"
//!   and stages nothing; a URL with a user or password is refused before anything runs.
//! - A serving hub admits what it stages into `state/origin/` (D7's `Origin::admit`): the
//!   daemon's hook (`origin::serving`) and the verb's (`origin.serve` consented in
//!   `state/relay.json`); nothing while it does not serve.
//! - `updates.check_internet = false` (read each round through `RelayHooks`): the hourly loop
//!   contacts no internet base, a dashboard wake included; on again, the next wake does.
//! - `GET /api/v1/update/origins`: the order, each origin's last answer and newest seen.
//!
//! The independent review's fixes:
//! - the watch on a real fabric (`updater::hub::spawn`, a member enrolled at a hub over loopback
//!   `tcp-lan` through a proxy that can cut the link): one check per attribution while the hub
//!   announces `ROLE_ORIGIN`, none while it does not, and the route's `hub` row;
//! - the verb runs only as the owner of `state/` (exit 2, nothing written); a directory that
//!   cannot be read exits 1;
//! - a refused manifest leaves the fixed class `fetch::UNVERIFIED` in the record, the route, the
//!   log and the terminal, never the parser's text (which quotes the offered bytes);
//! - a re-attribution while the hub's release is requested fetches the manifest only.

mod common;

use id111::keyset::NodeId;
use mesh111::fabric::HomeHub;
use node111::daemon::dashboard::{
    routes, serve_listener, ApiConfig, ApiState, DashboardState, DEFAULT_PORT,
};
use node111::daemon::fabric::{self, sealer::ROLE_ORIGIN, FabricConfig, FabricHandle};
use node111::daemon::origin::Origin as OriginStore;
use node111::daemon::relay::settings::{self as rs, ConsentKind, RelaySettings};
use node111::daemon::updater::fetch::{
    check_origins_with, origin_records, FetchOutcome, Origin, UpdaterConfig, UNVERIFIED,
};
use node111::daemon::updater::hub::{self, HubReads, HubWatch};
use node111::daemon::updater::stage::{LockTake, LockWait, StagingLock};
use node111::daemon::updater::{self, Hooks, RelayHooks, UpdaterStatus};
use node111::daemon::DaemonLayout;
use std::future::Future;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

const RUNNING: &str = "2026.10.01";
const RELEASE: &str = "2026.10.12";

fn sha(b: &[u8]) -> String {
    node111::util::hash::sha256_bytes(b)
}

// ---------------------------------------------------------------- fixtures

/// A logging HTTP server: `GET <path>` answers the file under `dir`, else 404.
struct LogServer {
    port: u16,
    log: Arc<Mutex<Vec<String>>>,
}

impl LogServer {
    fn serve(dir: Option<PathBuf>) -> LogServer {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&log);
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { break };
                let dir = dir.clone();
                let seen = Arc::clone(&seen);
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let Ok(n) = conn.read(&mut buf) else { return };
                    let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let mut words = req.split_whitespace();
                    let method = words.next().unwrap_or("").to_string();
                    let path = words.next().unwrap_or("/").to_string();
                    seen.lock().unwrap().push(format!("{method} {path}"));
                    let file = dir
                        .as_ref()
                        .map(|d| d.join(path.trim_start_matches('/')))
                        .and_then(|f| std::fs::read(f).ok());
                    let (status, body) = match file {
                        Some(b) => ("200 OK", b),
                        None => ("404 Not Found", b"not found".to_vec()),
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = conn.write_all(head.as_bytes());
                    if method != "HEAD" {
                        let _ = conn.write_all(&body);
                    }
                });
            }
        });
        LogServer { port, log }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn requests(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn exact(&self, line: &str) -> usize {
        self.requests().iter().filter(|r| *r == line).count()
    }

    fn count(&self, prefix: &str) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.starts_with(prefix))
            .count()
    }
}

/// An origin directory: the signed manifest of `release` (its payload pinned for this build's
/// target, `payload.url` a decoy) and `blob/<sha256>`.
fn origin_dir(release: &str, payload: &[u8]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let triple = node111::daemon::updater::fetch::TARGET_TRIPLE;
    let manifest = format!(
        r#"manifest-version = 1
date = "{release}"
channel = "stable"
release = "{release}"
min-upgrade-from = "2026.10.01"

[pkg.111.target.{triple}]
url = "https://example.invalid/111"
sha256 = "{a64}"
size = 100

[payload.{triple}]
url = "http://127.0.0.1:9/never-fetched.tar.gz"
sha256 = "{psha}"
size = {plen}

[rns-lock]
url = "https://example.invalid/rns-lock.txt"
sha256 = "{a64}"
size = 100

[kb]
url = "https://example.invalid/kb.tar.gz"
sha256 = "{a64}"
size = 100

[[keychain]]
generation = 0
pubkey = "RWRoot00000000000000000000000000000000000000000="
"#,
        a64 = "a".repeat(64),
        psha = sha(payload),
        plen = payload.len(),
    );
    let path = dir.path().join("channel-111-stable.toml");
    std::fs::write(&path, manifest).unwrap();
    common::sign_file(&common::minisign_fixture_key(), &path);
    std::fs::create_dir_all(dir.path().join("blob")).unwrap();
    std::fs::write(dir.path().join("blob").join(sha(payload)), payload).unwrap();
    dir
}

/// A hub serving `dir` under `/origin/v1/` (§9.1).
fn hub_server(dir: &Path) -> (LogServer, tempfile::TempDir) {
    let served = tempfile::tempdir().unwrap();
    std::fs::create_dir(served.path().join("origin")).unwrap();
    std::os::unix::fs::symlink(dir, served.path().join("origin/v1")).unwrap();
    (LogServer::serve(Some(served.path().to_path_buf())), served)
}

fn layout_at(root: &Path) -> DaemonLayout {
    std::fs::create_dir_all(root.join("state")).unwrap();
    DaemonLayout {
        root: root.to_path_buf(),
        etc: root.join("etc"),
    }
}

fn cfg(base: &str, check_internet: bool) -> UpdaterConfig {
    UpdaterConfig {
        manifest_base: base.to_string(),
        fallback_base: None,
        current_version: RUNNING.parse().unwrap(),
        timeout: Duration::from_secs(5),
        check_internet,
    }
}

fn dash() -> node111::daemon::dashboard::SharedState {
    Arc::new(RwLock::new(DashboardState::new(RUNNING)))
}

/// `state/relay.json` with `origin.serve` on and the consent owed now recorded.
fn serving_relay() -> RelaySettings {
    let mut r = RelaySettings::default();
    r.set_origin_serve(true);
    let hash = node111::daemon::relay::consent::origin_serve().hash;
    r.set_consent_hash(ConsentKind::OriginServe, Some(&hash))
        .unwrap();
    r
}

// ---------------------------------------------------------------- the fabric reads

struct Reads {
    hub: Option<HomeHub>,
    attributed: Mutex<bool>,
    roles: Mutex<u16>,
}

impl Reads {
    fn new(roles: u16) -> Reads {
        Reads {
            hub: Some(HomeHub {
                node_id: NodeId::from([0x42; 32]),
                address: Some("192.168.1.10:11183".parse().unwrap()),
            }),
            attributed: Mutex::new(false),
            roles: Mutex::new(roles),
        }
    }
    fn link(&self, up: bool) {
        *self.attributed.lock().unwrap() = up;
    }
}

impl HubReads for Reads {
    fn home_hub(&self) -> Option<HomeHub> {
        self.hub
    }
    fn attributed(&self, _hub: NodeId) -> impl Future<Output = Option<bool>> + Send {
        let v = *self.attributed.lock().unwrap();
        async move { Some(v) }
    }
    fn roles(&self, _hub: NodeId) -> impl Future<Output = Option<u16>> + Send {
        let v = *self.roles.lock().unwrap();
        async move { Some(v) }
    }
}

/// One `Fabric` batch: the watch looks; an origin it names is checked at `hub` (the test's
/// stand-in for `http://192.168.1.10:11180/origin/v1/`, which it asserts).
async fn batch(
    watch: &mut HubWatch,
    reads: &Reads,
    check: &Arc<dyn Fn(Origin) + Send + Sync>,
    hub: &LogServer,
) {
    if let Some(o) = watch.on_fabric(reads).await {
        assert_eq!(
            o,
            Origin::HomeHub {
                base: "http://192.168.1.10:11180/origin/v1/".into()
            }
        );
        let check = Arc::clone(check);
        let at = Origin::HomeHub {
            base: format!("{}/origin/v1/", hub.base()),
        };
        tokio::task::spawn_blocking(move || check(at))
            .await
            .unwrap();
    }
}

// ---------------------------------------------------------------- the hub origin

/// §9.3: checked once when the link attributes (with `check_internet` off too: that switch is
/// the internet bases'), never while it stays up, again on the next rising edge.
#[tokio::test]
async fn the_hub_origin_is_asked_on_attribution_only() {
    let payload = common::build_payload();
    let dir = origin_dir(RELEASE, &payload);
    let (hub, _served) = hub_server(dir.path());
    let decoy = LogServer::serve(None);
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let state = dash();
    let check = updater::hub_check(
        cfg(&decoy.base(), false),
        layout.clone(),
        Arc::clone(&state),
        None,
    );
    let reads = Reads::new(ROLE_ORIGIN | 1);
    let mut watch = HubWatch::default();

    for _ in 0..3 {
        batch(&mut watch, &reads, &check, &hub).await;
    }
    assert!(hub.requests().is_empty(), "no link, no request");

    reads.link(true);
    batch(&mut watch, &reads, &check, &hub).await;
    assert_eq!(hub.exact("GET /origin/v1/channel-111-stable.toml"), 1);
    assert_eq!(
        hub.count(&format!("GET /origin/v1/blob/{}", sha(&payload))),
        1
    );
    assert!(layout.state_dir().join("update-request.json").exists());
    assert!(
        state.read().unwrap().update_pending,
        "a stage marks the update pending"
    );
    assert_eq!(origin_records(&layout.state_dir())[0].kind, "home_hub");
    let request = std::fs::read(layout.state_dir().join("update-request.json")).unwrap();
    let before = hub.requests().len();
    for _ in 0..5 {
        batch(&mut watch, &reads, &check, &hub).await;
    }
    assert_eq!(
        hub.requests().len(),
        before,
        "still attributed: no new request"
    );

    reads.link(false);
    batch(&mut watch, &reads, &check, &hub).await;
    reads.link(true);
    batch(&mut watch, &reads, &check, &hub).await;
    assert_eq!(
        hub.exact("GET /origin/v1/channel-111-stable.toml"),
        2,
        "a new attribution asks again"
    );
    // review of row L4: the release is requested already, so a re-attribution (a flapping
    // link) costs the manifest only: no payload, no staging rewrite, no new helper run
    assert_eq!(
        hub.count(&format!("GET /origin/v1/blob/{}", sha(&payload))),
        1,
        "the payload is fetched once"
    );
    assert_eq!(
        std::fs::read(layout.state_dir().join("update-request.json")).unwrap(),
        request,
        "the request is not rewritten"
    );
    assert!(state.read().unwrap().update_pending);
    assert!(
        decoy.requests().is_empty(),
        "no internet base, no payload.url"
    );
}

/// D9: a hub whose announce lacks `ROLE_ORIGIN` is never asked, whatever its link does.
#[tokio::test]
async fn a_hub_without_the_origin_role_gets_no_request() {
    let payload = common::build_payload();
    let dir = origin_dir(RELEASE, &payload);
    let (hub, _served) = hub_server(dir.path());
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let check = updater::hub_check(
        cfg("http://127.0.0.1:9", true),
        layout.clone(),
        dash(),
        None,
    );
    let reads = Reads::new(0b11);
    let mut watch = HubWatch::default();
    for up in [true, true, false, true, false, true] {
        reads.link(up);
        batch(&mut watch, &reads, &check, &hub).await;
    }
    assert!(hub.requests().is_empty(), "{:?}", hub.requests());
    assert!(!layout.state_dir().join("staging").exists());
    assert!(origin_records(&layout.state_dir()).is_empty());
}

// ---------------------------------------------------------------- 111 update --from

/// A node root with `current` → `releases/<RUNNING>` (the release the verb reads).
fn node_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let rel = root.path().join("releases").join(RUNNING);
    std::fs::create_dir_all(&rel).unwrap();
    std::fs::create_dir_all(root.path().join("state")).unwrap();
    std::os::unix::fs::symlink(&rel, root.path().join("current")).unwrap();
    root
}

fn verb(root: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_111"))
        .args(args)
        .env("NODE111_ROOT", root)
        .output()
        .unwrap()
}

fn state_names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join("state"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// §9.2, T5: `--from <dir>` stages from the directory and writes nothing else.
#[test]
fn update_from_a_directory_stages_and_writes_only_the_staging() {
    let payload = common::build_payload();
    let dir = origin_dir(RELEASE, &payload);
    let root = node_root();
    let out = verb(
        root.path(),
        &["update", "--from", dir.path().to_str().unwrap()],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains(&format!("staged {RELEASE}")));
    let staged = root.path().join("state/staging").join(RELEASE);
    for f in [
        "payload.tar.gz",
        "channel-111-stable.toml",
        "channel-111-stable.toml.minisig",
        "tree/bin/111",
    ] {
        assert!(staged.join(f).exists(), "{f}");
    }
    assert_eq!(
        state_names(root.path()),
        [
            "staging",
            "staging.lock",
            "update-origins.json",
            "update-request.json"
        ],
        "T5: the staging, the request, the record and the lock; no relay.json, no origin/"
    );
    let recs = origin_records(&root.path().join("state"));
    assert_eq!(recs[0].kind, "dir");
    assert_eq!(recs[0].newest_seen.as_deref(), Some(RELEASE));
    assert!(
        !root.path().join("releases").join(RELEASE).exists(),
        "no flip"
    );
    // the same release again: up to date only once installed; staged again here
    let out = verb(
        root.path(),
        &["update", "--from", dir.path().to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0));
}

/// §9.2: a held staging lock ⇒ "the daemon is checking now", exit 75, nothing staged.
#[test]
fn a_held_lock_exits_75_and_stages_nothing() {
    let payload = common::build_payload();
    let dir = origin_dir(RELEASE, &payload);
    let root = node_root();
    let held = match StagingLock::acquire(&root.path().join("state"), LockWait::NoWait).unwrap() {
        LockTake::Taken(l) => l,
        LockTake::Held => panic!("free"),
    };
    let out = verb(
        root.path(),
        &["update", "--from", dir.path().to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(75));
    assert!(String::from_utf8_lossy(&out.stderr).contains("the daemon is checking now"));
    assert!(!root.path().join("state/staging").exists());
    assert!(!root.path().join("state/update-request.json").exists());
    assert!(origin_records(&root.path().join("state")).is_empty());
    drop(held);
    let out = verb(
        root.path(),
        &["update", "--from", dir.path().to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "free again");
}

/// Usage errors exit 2 before anything runs; a URL's password is never echoed.
#[test]
fn the_verb_refuses_bad_arguments() {
    let root = node_root();
    for (args, says) in [
        (&["update"][..], "usage: 111 update --from"),
        (&["update", "--from"], "usage: 111 update --from"),
        (
            &["update", "--from", "/tmp", "extra"],
            "usage: 111 update --from",
        ),
        (
            &["update", "--from", "http://user:hunter2@10.0.0.2/r"],
            "user or password",
        ),
        (
            &["update", "--from", "ftp://10.0.0.2/r"],
            "only http:// and https://",
        ),
    ] {
        let out = verb(root.path(), args);
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(err.contains(says), "{args:?}: {err}");
        assert!(!err.contains("hunter2"));
    }
    assert_eq!(
        state_names(root.path()),
        Vec::<String>::new(),
        "nothing written"
    );
}

/// Review of row L4: a directory that cannot be read now (an unmounted stick) is this origin's
/// answer, exit 1 as for a URL that does not answer; usage errors keep exit 2.
#[test]
fn a_directory_that_cannot_be_read_exits_1() {
    let root = node_root();
    let out = verb(root.path(), &["update", "--from", "/no/such/dir/111"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(
        err.contains("--from /no/such/dir/111: this directory cannot be read"),
        "{err}"
    );
    assert_eq!(
        state_names(root.path()),
        Vec::<String>::new(),
        "nothing written"
    );
}

/// Review of row L4 (§9.2): the verb runs as the node user, the owner of `state/`. Root running
/// it against a `state/` it does not own (`sudo 111 update --from …` on a host) exits 2 before
/// anything is read or written, so no root-owned lock, staging tree or record is left behind.
#[test]
fn the_verb_runs_only_as_the_owner_of_state() {
    // SAFETY: geteuid(2) cannot fail and touches no memory.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("skipped: handing state/ to another user needs root");
        return;
    }
    let payload = common::build_payload();
    let dir = origin_dir(RELEASE, &payload);
    let root = node_root();
    let state = root.path().join("state");
    std::os::unix::fs::chown(&state, Some(4242), Some(4242)).unwrap();
    let out = verb(
        root.path(),
        &["update", "--from", dir.path().to_str().unwrap()],
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(err.contains("run this as the node user"), "{err}");
    assert!(err.contains("sudo -u node111"), "{err}");
    assert_eq!(
        state_names(root.path()),
        Vec::<String>::new(),
        "nothing written"
    );
    // its owner runs it: the same tree stages
    std::os::unix::fs::chown(&state, Some(0), Some(0)).unwrap();
    let out = verb(
        root.path(),
        &["update", "--from", dir.path().to_str().unwrap()],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// An origin directory whose manifest the parser refuses with a quote of the offered line: an
/// escape sequence, a bidirectional override and a marker.
fn forged_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("channel-111-stable.toml");
    std::fs::write(
        &path,
        "manifest-version = 1\nFORGED-LINE \u{1b}[2J\u{202e} = \"x\"\n",
    )
    .unwrap();
    common::sign_file(&common::minisign_fixture_key(), &path);
    dir
}

/// `GET /api/v1/update/origins` on `api` (a loopback dashboard), as JSON.
async fn origins_json(api: &Arc<ApiState>) -> serde_json::Value {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(serve_listener(listener, routes::app(Arc::clone(api))));
    let body = tokio::task::spawn_blocking(move || {
        let mut conn = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            conn,
            "GET /api/v1/update/origins HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut raw = String::new();
        conn.read_to_string(&mut raw).unwrap();
        raw
    })
    .await
    .unwrap();
    server.abort();
    assert!(body.starts_with("HTTP/1.1 200"), "{body}");
    serde_json::from_str(body.split("\r\n\r\n").nth(1).unwrap()).unwrap()
}

/// Review of row L4 (§9.5 T6): a manifest refused before anything in it verified leaves one
/// fixed reason (`fetch::UNVERIFIED`) in the outcome, the record, the route, the log and the
/// terminal; the parser's text, which quotes the offered bytes, goes nowhere.
#[tokio::test]
async fn a_refused_manifest_leaves_only_a_fixed_reason() {
    let dir = forged_dir();
    // the home hub over plain http (T6)
    let (hub, _served) = hub_server(dir.path());
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let at = Origin::HomeHub {
        base: format!("{}/origin/v1/", hub.base()),
    };
    let l = layout.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        check_origins_with(
            &cfg("http://127.0.0.1:9", true),
            &l,
            &[at],
            LockWait::Wait,
            None,
        )
        .unwrap()
    })
    .await
    .unwrap();
    match outcome {
        FetchOutcome::Refused { reason } => assert_eq!(reason, UNVERIFIED),
        other => panic!("expected Refused, got {other:?}"),
    }
    let recs = origin_records(&layout.state_dir());
    assert_eq!(recs[0].detail.as_deref(), Some(UNVERIFIED));

    // `111 update --from <stick>`: the terminal, the record and the route
    let root = node_root();
    let out = verb(
        root.path(),
        &["update", "--from", dir.path().to_str().unwrap()],
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(
        err.contains(&format!("111: update refused: {UNVERIFIED}")),
        "{err}"
    );
    assert!(
        !err.contains(['\u{1b}', '\u{202e}']),
        "no raw escape reaches the terminal: {err:?}"
    );
    for line in err.lines() {
        assert!(line.starts_with("111"), "a line not ours: {line:?}");
    }
    let state_dir = root.path().join("state");
    assert_eq!(
        origin_records(&state_dir)[0].detail.as_deref(),
        Some(UNVERIFIED)
    );
    let mut c = ApiConfig::new(
        DaemonLayout {
            root: root.path().to_path_buf(),
            etc: root.path().join("etc"),
        },
        false,
    );
    c.persist_auth = false;
    let api = ApiState::new(dash(), c);
    let json = origins_json(&api).await;
    let row = json["origins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "directory")
        .unwrap()
        .clone();
    assert_eq!(row["last_answer"]["ok"], false);
    assert_eq!(row["last_answer"]["detail"], UNVERIFIED);
    assert!(!json.to_string().contains("FORGED"), "{json}");
}

/// §9.1 (D7's `admit`): `--from` on a hub whose `origin.serve` is on admits the verified files.
#[test]
fn the_verb_admits_into_the_origin_of_a_serving_hub() {
    let payload = common::build_payload();
    let dir = origin_dir(RELEASE, &payload);
    let root = node_root();
    rs::save(&root.path().join("state"), &serving_relay()).unwrap();
    let out = verb(
        root.path(),
        &["update", "--from", dir.path().to_str().unwrap()],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let origin = root.path().join("state/origin");
    assert_eq!(
        std::fs::read(origin.join("channel-111-stable.toml")).unwrap(),
        std::fs::read(dir.path().join("channel-111-stable.toml")).unwrap()
    );
    assert!(origin.join("channel-111-stable.toml.minisig").exists());
    assert_eq!(
        std::fs::read(origin.join("blob").join(sha(&payload))).unwrap(),
        payload
    );
    assert!(OriginStore::new(&root.path().join("state")).bytes() > payload.len() as u64);
}

// ---------------------------------------------------------------- the daemon's hook

/// A hub with LAN sharing, the hub role and `origin.serve` per `serve`; its fabric is down (no
/// identity), as in `origin_serve.rs`.
async fn hub_api(serve: bool) -> (Arc<ApiState>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let layout = layout_at(dir.path());
    let mut c = ApiConfig::new(layout.clone(), true);
    c.persist_auth = false;
    let api = ApiState::new(dash(), c);
    let f = fabric::start(FabricConfig::production(&layout, Some("Hub4")), None, &api).await;
    let relay = if serve {
        serving_relay()
    } else {
        RelaySettings::default()
    };
    rs::save(&layout.state_dir(), &relay).unwrap();
    f.set_relay_settings(&relay).await.unwrap();
    api.insert_ext(f);
    (api, dir)
}

/// §9.1: the daemon's after-stage hook admits only while `origin::serving` holds.
#[tokio::test]
async fn a_serving_hub_admits_what_it_stages_and_no_other_does() {
    let payload = common::build_payload();
    let dir = origin_dir(RELEASE, &payload);
    for serve in [true, false] {
        let (api, _root) = hub_api(serve).await;
        assert_eq!(node111::daemon::origin::serving(&api), serve);
        let layout = api.layout.clone();
        let hook = updater::daemon_after_stage(Arc::clone(&api));
        let from = Origin::Dir {
            path: dir.path().to_path_buf(),
        };
        let outcome = tokio::task::spawn_blocking(move || {
            check_origins_with(
                &cfg("http://127.0.0.1:9", true),
                &layout,
                &[from],
                LockWait::Wait,
                Some(&*hook),
            )
            .unwrap()
        })
        .await
        .unwrap();
        assert!(
            matches!(outcome, FetchOutcome::Staged { .. }),
            "{outcome:?}"
        );
        let held = api
            .layout
            .state_dir()
            .join("origin/blob")
            .join(sha(&payload));
        assert_eq!(held.exists(), serve, "serve = {serve}");
        let bytes = OriginStore::shared(&api).bytes();
        assert_eq!(bytes > 0, serve, "origin.bytes moves only on a serving hub");
    }
}

// ---------------------------------------------------------------- check_internet

/// §9.3: `updates.check_internet = false` ⇒ the loop contacts no internet base, a wake
/// included; the switch is read each round, so on again the next wake checks.
#[tokio::test]
async fn check_internet_off_contacts_no_internet_base() {
    let net = LogServer::serve(None);
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let on = Arc::new(AtomicBool::new(false));
    let hooks = Hooks {
        wake: Arc::new(tokio::sync::Notify::new()),
        status: Arc::new(UpdaterStatus::default()),
    };
    let relay = RelayHooks {
        check_internet: Some({
            let on = Arc::clone(&on);
            Arc::new(move || on.load(Ordering::SeqCst))
        }),
        after_stage: None,
    };
    let (wake, status) = (Arc::clone(&hooks.wake), Arc::clone(&hooks.status));
    // the config says on: the switch decides
    let task = tokio::spawn(updater::run_with_relay(
        layout.clone(),
        dash(),
        cfg(&net.base(), true),
        hooks,
        relay,
    ));
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        wake.notify_one();
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(net.requests().is_empty(), "{:?}", net.requests());
    assert_eq!(status.checks(), 0);
    assert!(!layout.state_dir().join("staging.lock").exists());

    on.store(true, Ordering::SeqCst);
    wake.notify_one();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while status.checks() == 0 && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    task.abort();
    assert_eq!(status.checks(), 1);
    assert_eq!(
        net.count("GET /channel-111-stable.toml"),
        1,
        "on: the base is asked"
    );
}

// ---------------------------------------------------------------- GET /api/v1/update/origins

/// §9.3, T4: the order (internet bases, then each `--from` origin), each last answer with its
/// monotonic age, and the newest release seen; a GET asks no origin.
#[tokio::test]
async fn the_origins_route_lists_the_order_and_the_answers() {
    let net = LogServer::serve(None);
    let fallback = LogServer::serve(None);
    let node = tempfile::tempdir().unwrap();
    let layout = layout_at(node.path());
    let mut c = ApiConfig::new(layout.clone(), false);
    c.persist_auth = false;
    let api = ApiState::new(dash(), c);
    let mut ucfg = cfg(&net.base(), true);
    ucfg.fallback_base = Some(fallback.base());
    api.insert_ext(Arc::new(ucfg.clone()));

    // one --from directory staged, then the internet base answers 404 (both bases tried)
    let payload = common::build_payload();
    let dir = origin_dir(RELEASE, &payload);
    let l = layout.clone();
    let u = ucfg.clone();
    let from = Origin::Dir {
        path: dir.path().to_path_buf(),
    };
    tokio::task::spawn_blocking(move || {
        check_origins_with(&u, &l, &[from], LockWait::NoWait, None).unwrap();
        node111::daemon::updater::fetch::check_once(&u, &l).unwrap();
    })
    .await
    .unwrap();
    let asked = net.requests().len() + fallback.requests().len();

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve_listener(listener, routes::app(Arc::clone(&api))));
    let body = tokio::task::spawn_blocking(move || {
        let mut conn = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            conn,
            "GET /api/v1/update/origins HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut raw = String::new();
        conn.read_to_string(&mut raw).unwrap();
        raw
    })
    .await
    .unwrap();
    assert!(body.starts_with("HTTP/1.1 200"), "{body}");
    let json: serde_json::Value =
        serde_json::from_str(body.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(json["schema"], 1);
    let rows = json["origins"].as_array().unwrap();
    assert_eq!(rows.len(), 3, "{json}");
    let kinds: Vec<&str> = rows.iter().map(|r| r["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["internet", "internet", "directory"]);
    assert_eq!(rows[0]["url"], net.base());
    assert_eq!(rows[1]["url"], fallback.base());
    for r in rows {
        for key in ["kind", "url", "last_answer", "newest_seen"] {
            assert!(r.get(key).is_some(), "{key} in {r}");
        }
        let a = &r["last_answer"];
        assert!(
            a["ok"].is_boolean() && a["ago_s"].as_u64().is_some_and(|s| s < 60),
            "{r}"
        );
    }
    assert_eq!(rows[0]["last_answer"]["ok"], false);
    assert!(rows[0]["last_answer"]["detail"].is_string());
    assert_eq!(rows[0]["newest_seen"], serde_json::Value::Null);
    assert_eq!(rows[2]["url"], dir.path().to_str().unwrap());
    assert_eq!(rows[2]["last_answer"]["ok"], true);
    assert_eq!(rows[2]["last_answer"]["detail"], serde_json::Value::Null);
    assert_eq!(rows[2]["newest_seen"], RELEASE);
    assert_eq!(
        net.requests().len() + fallback.requests().len(),
        asked,
        "a GET asks no origin"
    );
}

// ---------------------------------------------------------------- the watch on a real fabric

/// A node with its fabric running on loopback: `hub` holds the hub role, `lan` is LAN sharing
/// (role bit 5 needs it), `relay` is written to `state/relay.json` before the fabric starts.
struct Peer {
    api: Arc<ApiState>,
    fabric: Arc<FabricHandle>,
    id: NodeId,
    _dir: tempfile::TempDir,
}

fn fabric_config(layout: &DaemonLayout, hub: bool) -> FabricConfig {
    let mut c = FabricConfig::production(layout, None);
    c.listen = "127.0.0.1:0".parse().unwrap();
    c.tcp.refuse_loopback = false;
    c.tcp.backoff_min = Duration::from_millis(30);
    c.tcp.backoff_max = Duration::from_millis(200);
    c.tcp.handshake_timeout = Duration::from_secs(3);
    c.hub_default = hub;
    c.enroll_timeout = Duration::from_secs(10);
    c.housekeeping = Duration::from_millis(200);
    c
}

async fn peer(hub: bool, lan: bool, relay: Option<RelaySettings>) -> Peer {
    let dir = tempfile::tempdir().unwrap();
    let layout = layout_at(dir.path());
    if let Some(r) = &relay {
        rs::save(&layout.state_dir(), r).unwrap();
    }
    let identity = node111::identity::generate(&layout.identity_dir()).unwrap();
    let hex = identity.node_id_hex();
    let id = NodeId::from(id111::encoding::hex_decode(&hex).unwrap());
    let state = dash();
    state.write().unwrap().node_id = Some(hex);
    let mut c = ApiConfig::new(layout.clone(), lan);
    c.persist_auth = false;
    let api = ApiState::new(state, c);
    let fabric = fabric::start(fabric_config(&layout, hub), Some(identity), &api).await;
    assert!(fabric.running(), "{:?}", fabric.down_reason());
    api.insert_ext(Arc::clone(&fabric));
    Peer {
        api,
        fabric,
        id,
        _dir: dir,
    }
}

/// `member` enrols at `hub`, dialling `at` (the hub's listener, or a proxy to it).
async fn enrol(member: &Peer, hub: &Peer, at: SocketAddr) {
    let offer = hub.fabric.fabric().unwrap().offer(Some(at)).await.unwrap();
    member
        .fabric
        .fabric()
        .unwrap()
        .enroll(at, &offer.code, Some(&offer.card))
        .await
        .unwrap();
    member.fabric.sync_settings().await.unwrap();
}

/// Whether a connection of `member` is attributed to `hub` now.
async fn attributed(member: &Peer, hub: &Peer) -> bool {
    matches!(
        member.fabric.fabric().unwrap().attributed_on(hub.id).await,
        Ok(Some(_))
    )
}

/// The role bits of `hub`'s announce as `member` holds them.
async fn roles_of(member: &Peer, hub: &Peer) -> u16 {
    member
        .fabric
        .fabric()
        .unwrap()
        .contact(hub.id)
        .await
        .unwrap()
        .roles
}

async fn until<F: Future<Output = bool>>(secs: u64, what: &str, mut f: impl FnMut() -> F) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    while !f().await {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A loopback TCP proxy to `to` whose connections can be cut: the member's link to its hub
/// drops, stays down while [`Flap::cut`] holds (new connections are closed at once), and comes
/// back by the member's own redial once [`Flap::reopen`]ed.
struct Flap {
    at: SocketAddr,
    open: Arc<AtomicBool>,
    tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    accept: tokio::task::JoinHandle<()>,
}

impl Flap {
    async fn start(to: SocketAddr) -> Flap {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let at = listener.local_addr().unwrap();
        let open = Arc::new(AtomicBool::new(true));
        let tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> = Arc::default();
        let accept = tokio::spawn({
            let (open, tasks) = (Arc::clone(&open), Arc::clone(&tasks));
            async move {
                while let Ok((mut inbound, _)) = listener.accept().await {
                    if !open.load(Ordering::SeqCst) {
                        continue;
                    }
                    let h = tokio::spawn(async move {
                        if let Ok(mut out) = tokio::net::TcpStream::connect(to).await {
                            let _ = tokio::io::copy_bidirectional(&mut inbound, &mut out).await;
                        }
                    });
                    tasks.lock().unwrap().push(h);
                }
            }
        });
        Flap {
            at,
            open,
            tasks,
            accept,
        }
    }

    fn cut(&self) {
        self.open.store(false, Ordering::SeqCst);
        for t in self.tasks.lock().unwrap().drain(..) {
            t.abort();
        }
    }

    fn reopen(&self) {
        self.open.store(true, Ordering::SeqCst);
    }
}

impl Drop for Flap {
    fn drop(&mut self) {
        self.accept.abort();
        for t in self.tasks.lock().unwrap().drain(..) {
            t.abort();
        }
    }
}

/// A check that only counts the origins it is asked for.
fn counting() -> (hub::Check, Arc<Mutex<Vec<Origin>>>) {
    let calls: Arc<Mutex<Vec<Origin>>> = Arc::default();
    let check: hub::Check = {
        let calls = Arc::clone(&calls);
        Arc::new(move |o| calls.lock().unwrap().push(o))
    };
    (check, calls)
}

/// Review of row L4 (§9.3, D9): the daemon's watch (`hub::spawn`) on a member's real fabric,
/// enrolled at a hub that announces `ROLE_ORIGIN`: the attribution that completed before the
/// watch started is checked at once, nothing more while the link stays up, and the next
/// attribution after the link drops is checked again; the route lists the hub first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_watch_on_a_real_fabric_asks_its_hub_once_per_attribution() {
    let hub = peer(true, true, Some(serving_relay())).await;
    let proxy = Flap::start(hub.fabric.listen_addr().await.unwrap()).await;
    let member = peer(false, false, None).await;
    enrol(&member, &hub, proxy.at).await;
    until(30, "the link to the hub attributes", || {
        attributed(&member, &hub)
    })
    .await;
    assert_ne!(
        roles_of(&member, &hub).await & ROLE_ORIGIN,
        0,
        "the hub's announce carries ROLE_ORIGIN"
    );

    let (check, calls) = counting();
    let n = || calls.lock().unwrap().len();
    let watch = hub::spawn(&member.fabric, check).expect("the member's fabric runs");
    until(30, "the attribution is checked", || {
        let k = n();
        async move { k == 1 }
    })
    .await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(n(), 1, "still attributed: once");

    // the link drops and stays down until the watch has seen it fall
    proxy.cut();
    until(30, "the link falls", || async {
        !attributed(&member, &hub).await
    })
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(n(), 1, "no check while down");
    proxy.reopen();
    until(30, "the next attribution is checked", || {
        let k = n();
        async move { k == 2 }
    })
    .await;
    let want = Origin::HomeHub {
        base: format!("http://127.0.0.1:{DEFAULT_PORT}/origin/v1/"),
    };
    assert!(
        calls.lock().unwrap().iter().all(|o| *o == want),
        "{:?}",
        calls.lock().unwrap()
    );

    // `GET /api/v1/update/origins` through the member's real fabric handle: the hub first
    let json = origins_json(&member.api).await;
    let rows = json["origins"].as_array().unwrap();
    assert_eq!(rows[0]["kind"], "hub", "{json}");
    assert_eq!(rows[0]["url"], want.label());
    assert_eq!(rows[0]["last_answer"], serde_json::Value::Null);
    assert_eq!(rows[1]["kind"], "internet");

    watch.abort();
    member.fabric.shutdown().await;
    hub.fabric.shutdown().await;
}

/// Review of row L4 (D9): a member of a real hub that does not announce `ROLE_ORIGIN` (LAN
/// sharing and the hub role on, `origin.serve` off) attributes its link and is never asked to
/// check; the route lists no hub.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_real_hub_without_the_origin_role_is_never_asked() {
    let hub = peer(true, true, None).await;
    let member = peer(false, false, None).await;
    let (check, calls) = counting();
    // watching before the enrolment: the enrolment and the attribution arrive as events
    let watch = hub::spawn(&member.fabric, check).expect("the member's fabric runs");
    enrol(&member, &hub, hub.fabric.listen_addr().await.unwrap()).await;
    until(30, "the link to the hub attributes", || {
        attributed(&member, &hub)
    })
    .await;
    assert_eq!(roles_of(&member, &hub).await & ROLE_ORIGIN, 0);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(calls.lock().unwrap().is_empty(), "no ROLE_ORIGIN: no check");
    let json = origins_json(&member.api).await;
    let kinds: Vec<&str> = json["origins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["kind"].as_str().unwrap())
        .collect();
    assert!(!kinds.contains(&"hub"), "{json}");

    watch.abort();
    member.fabric.shutdown().await;
    hub.fabric.shutdown().await;
}
