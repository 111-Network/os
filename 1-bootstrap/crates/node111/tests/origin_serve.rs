//! Layer 7 row D7 (`os/7-relay/README.md` §9.1; S4, S8, I7; §19 C6, D7): the hub as update
//! origin, the serving side, through the real dashboard app on loopback sockets.
//!
//! - `GET /origin/v1/{channel-111-stable.toml, .minisig, blob/<sha256>}` answer the admitted
//!   bytes byte-exact, without a session (an untrusted loopback peer and a LAN peer both get
//!   401 on `/api/status` and 200 here), with the dashboard's CSP;
//! - 404 while any of the three switches is off (LAN sharing, `hub_role`, `origin.serve` with
//!   its consent), and as soon as the relay switch goes off at run time;
//! - 405 on every other method (no CSRF problem: the route is outside `/api/v1`);
//! - the Host guard admits the hub's own address while LAN sharing is on and refuses another;
//!   `Host: localhost` is not redirected (the machine-contract exemption);
//! - a missing, unpinned, planted or misnamed blob 404s; another target's payload is held and
//!   served only when `origin.targets` lists it;
//! - `Origin::admit` refuses unverified bytes and serves nothing then; `Origin::bytes` is the
//!   size of `state/origin/`;
//! - one `Origin` per process: first callers racing on `Origin::shared` share one store (row
//!   D5b);
//! - a torn pair (a crash between the signature's rename and the manifest's) is not served:
//!   all three names answer 404, as when no release is held, until the next admission (row
//!   D5c item 4).
//!
//! Fixtures: `tests/fixtures/l7-D7/` (two releases signed by the compiled DEV key; its README).

use axum::extract::connect_info::MockConnectInfo;
use node111::daemon::dashboard::routes::{self, CSP};
use node111::daemon::dashboard::{serve_listener, ApiConfig, ApiState, DashboardState};
use node111::daemon::fabric::{self, FabricConfig, FabricHandle};
use node111::daemon::origin::{AdmitError, Origin, MANIFEST_NAME, SIGNATURE_NAME};
use node111::daemon::relay::settings::{self as relay_settings, ConsentKind, RelaySettings};
use node111::daemon::updater::fetch::TARGET_TRIPLE;
use node111::daemon::DaemonLayout;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

const R1: &str = "2026.10.12";
const OWN_ADDR: &str = "192.0.2.7";

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/l7-D7/{rel}"))
}

fn release(rel: &str) -> (Vec<u8>, String) {
    (
        std::fs::read(fixture(&format!("{rel}/{MANIFEST_NAME}"))).unwrap(),
        std::fs::read_to_string(fixture(&format!("{rel}/{SIGNATURE_NAME}"))).unwrap(),
    )
}

/// The other target of the fixture (this build's own is `TARGET_TRIPLE`).
fn other_target() -> &'static str {
    if TARGET_TRIPLE.starts_with("x86_64") {
        "aarch64-unknown-linux-musl"
    } else {
        "x86_64-unknown-linux-musl"
    }
}

fn payload(rel: &str, triple: &str) -> PathBuf {
    fixture(&format!(
        "payload/{rel}-{}.bin",
        triple.split('-').next().unwrap()
    ))
}

fn sha(p: &std::path::Path) -> String {
    node111::util::hash::sha256_file(p).unwrap()
}

// ---------------------------------------------------------------- http

struct Reply {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_string())
        })
    }
}

fn http(port: u16, method: &str, path: &str, host: &str) -> Reply {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    conn.write_all(req.as_bytes()).unwrap();
    let mut buf = Vec::new();
    conn.read_to_end(&mut buf).unwrap();
    let split = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a response head");
    let head = String::from_utf8_lossy(&buf[..split]).into_owned();
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Reply {
        status,
        head,
        body: buf[split + 4..].to_vec(),
    }
}

fn get(port: u16, path: &str) -> Reply {
    http(port, "GET", path, &format!("127.0.0.1:{port}"))
}

// ---------------------------------------------------------------- a hub

/// The switches of one test node.
#[derive(Clone, Copy)]
struct Switches {
    lan: bool,
    hub_role: bool,
    serve: bool,
    consent: bool,
}

const ALL_ON: Switches = Switches {
    lan: true,
    hub_role: true,
    serve: true,
    consent: true,
};

struct Node {
    api: Arc<ApiState>,
    fabric: Arc<FabricHandle>,
    state_dir: PathBuf,
    _dir: tempfile::TempDir,
}

fn relay(serve: bool, consent: bool) -> RelaySettings {
    let mut r = RelaySettings::default();
    r.set_origin_serve(serve);
    if consent {
        r.set_consent_hash(ConsentKind::OriginServe, Some(&"ab".repeat(32)))
            .unwrap();
    }
    r
}

/// A node with `sw`, its fabric down (no identity) but holding the hub role per `sw`.
async fn node(sw: Switches) -> Node {
    let dir = tempfile::tempdir().unwrap();
    let layout = DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let state_dir = layout.state_dir();
    std::fs::create_dir_all(&state_dir).unwrap();
    let mut cfg = ApiConfig::new(layout.clone(), sw.lan);
    cfg.persist_auth = false;
    // Loopback is not trusted here: `/api/status` must answer 401 to these sockets.
    cfg.trust_loopback = false;
    let api = ApiState::new(
        Arc::new(RwLock::new(DashboardState::new("2026.10.14"))),
        cfg,
    );
    api.set_own_addrs(vec![OWN_ADDR.parse::<IpAddr>().unwrap()]);
    let category = sw.hub_role.then_some("Hub4");
    let fabric = fabric::start(FabricConfig::production(&layout, category), None, &api).await;
    assert_eq!(fabric.persisted().fabric.hub_role, sw.hub_role);
    fabric
        .set_relay_settings(&relay(sw.serve, sw.consent))
        .await
        .expect("the relay switches are applied");
    api.insert_ext(Arc::clone(&fabric));
    Node {
        api,
        fabric,
        state_dir,
        _dir: dir,
    }
}

impl Node {
    /// Admit release `rel` with the payloads of `triples`.
    fn admit(&self, rel: &str, triples: &[&str]) -> Result<(), AdmitError> {
        let (m, s) = release(rel);
        let paths: Vec<PathBuf> = triples.iter().map(|t| payload(rel, t)).collect();
        let refs: Vec<&std::path::Path> = paths.iter().map(|p| p.as_path()).collect();
        Origin::shared(&self.api).admit(&m, &s, &refs).map(|_| ())
    }

    /// Serve the dashboard app; `peer` fakes the connection's remote address.
    async fn serve(&self, peer: Option<&str>) -> u16 {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = routes::app(Arc::clone(&self.api));
        match peer {
            None => {
                tokio::spawn(serve_listener(listener, app));
            }
            Some(p) => {
                let addr: SocketAddr = p.parse().unwrap();
                let app = app.layer(MockConnectInfo(addr));
                tokio::spawn(async move { axum::serve(listener, app).await });
            }
        }
        port
    }
}

fn paths() -> [String; 3] {
    let p = payload(R1, TARGET_TRIPLE);
    [
        format!("/origin/v1/{MANIFEST_NAME}"),
        format!("/origin/v1/{SIGNATURE_NAME}"),
        format!("/origin/v1/blob/{}", sha(&p)),
    ]
}

// ---------------------------------------------------------------- tests

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn served_byte_exact_without_a_session() {
    let n = node(ALL_ON).await;
    n.admit(R1, &[TARGET_TRIPLE]).unwrap();
    let (m, s) = release(R1);
    let blob = std::fs::read(payload(R1, TARGET_TRIPLE)).unwrap();
    for peer in [None, Some("192.168.1.20:40000")] {
        let port = n.serve(peer).await;
        // the session gate holds for the gated routes on this very socket
        assert_eq!(get(port, "/api/status").status, 401, "peer {peer:?}");
        let [mp, sp, bp] = paths();
        for (path, want, ctype) in [
            (&mp, m.as_slice(), "application/toml"),
            (&sp, s.as_bytes(), "text/plain; charset=utf-8"),
            (&bp, blob.as_slice(), "application/octet-stream"),
        ] {
            let r = get(port, path);
            assert_eq!(r.status, 200, "{path} from {peer:?}: {}", r.head);
            assert_eq!(r.body, want, "{path} byte-exact");
            assert_eq!(r.header("content-length"), Some(want.len().to_string()));
            assert_eq!(r.header("content-type").as_deref(), Some(ctype));
            assert_eq!(r.header("content-security-policy").as_deref(), Some(CSP));
            assert_eq!(
                r.header("x-content-type-options").as_deref(),
                Some("nosniff")
            );
            assert!(r.header("set-cookie").is_none());
        }
    }
    // §16 record: what the origin holds for one served target.
    let bytes = Origin::shared(&n.api).bytes();
    assert_eq!(bytes, (m.len() + s.len() + blob.len()) as u64);
    eprintln!("origin disk, one target (fixture): {bytes} B");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_switch_off_answers_404() {
    let off = [
        (
            "LAN sharing",
            Switches {
                lan: false,
                ..ALL_ON
            },
        ),
        (
            "hub_role",
            Switches {
                hub_role: false,
                ..ALL_ON
            },
        ),
        (
            "origin.serve",
            Switches {
                serve: false,
                ..ALL_ON
            },
        ),
        (
            "origin_serve consent",
            Switches {
                consent: false,
                ..ALL_ON
            },
        ),
    ];
    for (what, sw) in off {
        let n = node(sw).await;
        n.admit(R1, &[TARGET_TRIPLE]).unwrap();
        let port = n.serve(None).await;
        for path in paths() {
            let r = get(port, &path);
            assert_eq!(r.status, 404, "{path} with {what} off");
            assert_ne!(r.body, release(R1).0, "no file bytes with {what} off");
        }
    }
    // The relay switch is read per request: off at run time ⇒ 404 at once.
    let n = node(ALL_ON).await;
    n.admit(R1, &[TARGET_TRIPLE]).unwrap();
    let port = n.serve(None).await;
    let [mp, ..] = paths();
    assert_eq!(get(port, &mp).status, 200);
    n.fabric
        .set_relay_settings(&relay(false, true))
        .await
        .expect("the relay switches are applied");
    assert_eq!(get(port, &mp).status, 404);
    n.fabric
        .set_relay_settings(&relay(true, true))
        .await
        .expect("the relay switches are applied");
    assert_eq!(get(port, &mp).status, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_other_method_is_405() {
    let n = node(ALL_ON).await;
    n.admit(R1, &[TARGET_TRIPLE]).unwrap();
    let port = n.serve(Some("192.168.1.20:40000")).await;
    let host = format!("127.0.0.1:{port}");
    for path in paths() {
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            let r = http(port, method, &path, &host);
            assert_eq!(r.status, 405, "{method} {path}");
            assert!(
                r.header("allow").is_some_and(|a| a.contains("GET")),
                "{}",
                r.head
            );
        }
    }
    // the same while not serving: the method is refused before anything is looked up
    let n = node(Switches {
        serve: false,
        ..ALL_ON
    })
    .await;
    let port = n.serve(None).await;
    let [mp, ..] = paths();
    assert_eq!(
        http(port, "POST", &mp, &format!("127.0.0.1:{port}")).status,
        405
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_host_guard_admits_the_hubs_own_address() {
    let n = node(ALL_ON).await;
    n.admit(R1, &[TARGET_TRIPLE]).unwrap();
    let port = n.serve(Some("192.168.1.20:40000")).await;
    let [mp, _, bp] = paths();
    // a member fetches http://<home hub host>:11180/origin/v1/ (S4): an IP-literal Host
    for path in [&mp, &bp] {
        assert_eq!(
            http(port, "GET", path, &format!("{OWN_ADDR}:11180")).status,
            200
        );
        assert_eq!(http(port, "GET", path, "198.51.100.9:11180").status, 400);
        assert_eq!(http(port, "GET", path, "evil.example").status, 400);
    }
    // the machine-contract exemption: no 308 to 127.0.0.1 for Host: localhost
    let r = http(port, "GET", &mp, &format!("localhost:{port}"));
    assert_eq!(r.status, 200, "{}", r.head);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_unpinned_and_misnamed_blobs_404() {
    let n = node(ALL_ON).await;
    n.admit(R1, &[TARGET_TRIPLE]).unwrap();
    let port = n.serve(None).await;
    let blob_dir = n.state_dir.join("origin/blob");
    // a well-formed name nothing pins
    let nothing = "0123456789abcdef".repeat(4);
    assert_eq!(get(port, &format!("/origin/v1/blob/{nothing}")).status, 404);
    // a file planted under blob/ that no pin names is never served
    std::fs::write(blob_dir.join(&nothing), b"planted").unwrap();
    assert_eq!(get(port, &format!("/origin/v1/blob/{nothing}")).status, 404);
    // pinned for the other target, which origin.targets does not list: not held
    let other = sha(&payload(R1, other_target()));
    assert!(!blob_dir.join(&other).exists());
    assert_eq!(get(port, &format!("/origin/v1/blob/{other}")).status, 404);
    // pinned, but the held file was removed
    let own = sha(&payload(R1, TARGET_TRIPLE));
    std::fs::rename(blob_dir.join(&own), n.state_dir.join("aside")).unwrap();
    assert_eq!(get(port, &format!("/origin/v1/blob/{own}")).status, 404);
    // a symlink at the pinned name is not followed
    std::os::unix::fs::symlink(n.state_dir.join("aside"), blob_dir.join(&own)).unwrap();
    assert_eq!(get(port, &format!("/origin/v1/blob/{own}")).status, 404);
    // names that are not 64 lowercase hex digits
    for bad in [own.to_uppercase(), own[..63].to_string(), "x".repeat(64)] {
        assert_eq!(
            get(port, &format!("/origin/v1/blob/{bad}")).status,
            404,
            "{bad}"
        );
    }
    assert_eq!(get(port, "/origin/v1/blob/..%2Ffloor.json").status, 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_target_only_from_origin_targets() {
    let n = node(ALL_ON).await;
    let mut r = relay(true, true);
    r.set_origin_targets(&[TARGET_TRIPLE, other_target()])
        .unwrap();
    relay_settings::save(&n.state_dir, &r).unwrap();
    n.admit(R1, &[TARGET_TRIPLE, other_target()]).unwrap();
    let port = n.serve(None).await;
    for t in [TARGET_TRIPLE, other_target()] {
        let p = payload(R1, t);
        let reply = get(port, &format!("/origin/v1/blob/{}", sha(&p)));
        assert_eq!(reply.status, 200, "{t}");
        assert_eq!(reply.body, std::fs::read(&p).unwrap());
    }
    // the target is dropped again: its payload goes, the own one stays
    let origin = Origin::shared(&n.api);
    assert!(origin.retain_targets(&[TARGET_TRIPLE.to_string()]) > 0);
    let gone = sha(&payload(R1, other_target()));
    assert_eq!(get(port, &format!("/origin/v1/blob/{gone}")).status, 404);
    let [_, _, bp] = paths();
    assert_eq!(get(port, &bp).status, 200);
}

/// Row D5b: `Origin::shared` registers one store per process even when its first callers
/// race (the look-up and the insert under one lock): every caller gets the same `Arc`, so one
/// write mutex serializes every admission. Many rounds of eight threads released together.
#[test]
fn first_callers_share_one_origin() {
    const THREADS: usize = 8;
    for round in 0..200 {
        let dir = tempfile::tempdir().unwrap();
        let layout = DaemonLayout {
            root: dir.path().to_path_buf(),
            etc: dir.path().join("etc"),
        };
        let mut cfg = ApiConfig::new(layout, false);
        cfg.persist_auth = false;
        let api = ApiState::new(
            Arc::new(RwLock::new(DashboardState::new("2026.10.14"))),
            cfg,
        );
        let gate = Arc::new(std::sync::Barrier::new(THREADS));
        let got: Vec<Arc<Origin>> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..THREADS)
                .map(|_| {
                    let (api, gate) = (Arc::clone(&api), Arc::clone(&gate));
                    s.spawn(move || {
                        gate.wait();
                        Origin::shared(&api)
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let kept = Origin::shared(&api);
        for o in &got {
            assert!(Arc::ptr_eq(o, &kept), "round {round}: two stores");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unverified_bytes_are_refused_by_admit() {
    let n = node(ALL_ON).await;
    let (mut m, s) = release(R1);
    let p = payload(R1, TARGET_TRIPLE);
    // a pin edited after signing
    let text = String::from_utf8(m.clone()).unwrap();
    let at = text.find("size = ").unwrap() + "size = ".len();
    m[at] = if m[at] == b'9' { b'8' } else { b'9' };
    let origin = Origin::shared(&n.api);
    let e = origin.admit(&m, &s, &[p.as_path()]).unwrap_err();
    assert!(matches!(e, AdmitError::Unverified(_)), "{e}");
    // a signature over other bytes
    let (m1, _) = release(R1);
    let (_, s2) = release("2026.11.02");
    let e = origin.admit(&m1, &s2, &[p.as_path()]).unwrap_err();
    assert!(matches!(e, AdmitError::Unverified(_)), "{e}");
    // nothing was stored, nothing is served
    assert!(!n.state_dir.join("origin").exists());
    assert_eq!(origin.bytes(), 0);
    let port = n.serve(None).await;
    for path in paths() {
        assert_eq!(get(port, &path).status, 404, "{path}");
    }
}

/// Row D5c item 4: R2's signature beside R1's manifest (what a crash between the two renames
/// leaves) is never served: the manifest, the signature and the held blob answer 404, as when
/// no release is held. The next admission repairs it and the pair is served again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_torn_pair_answers_404_until_the_next_admission() {
    const R2: &str = "2026.11.02";
    let n = node(ALL_ON).await;
    n.admit(R1, &[TARGET_TRIPLE]).unwrap();
    let port = n.serve(None).await;
    for path in paths() {
        assert_eq!(get(port, &path).status, 200, "{path}");
    }
    // the crash: the new signature renamed into place, the manifest's rename never ran
    let origin_dir = n.state_dir.join("origin");
    let (_, s2) = release(R2);
    let tmp = origin_dir.join(".tmp-crash-0");
    std::fs::write(&tmp, &s2).unwrap();
    std::fs::rename(&tmp, origin_dir.join(SIGNATURE_NAME)).unwrap();
    for path in paths() {
        assert_eq!(get(port, &path).status, 404, "torn: {path}");
    }
    // the next admission repairs it
    n.admit(R2, &[TARGET_TRIPLE]).unwrap();
    let (m2, s2) = release(R2);
    let p2 = payload(R2, TARGET_TRIPLE);
    for (path, want) in [
        (format!("/origin/v1/{MANIFEST_NAME}"), m2),
        (format!("/origin/v1/{SIGNATURE_NAME}"), s2.into_bytes()),
        (
            format!("/origin/v1/blob/{}", sha(&p2)),
            std::fs::read(&p2).unwrap(),
        ),
    ] {
        let r = get(port, &path);
        assert_eq!(r.status, 200, "repaired: {path}");
        assert_eq!(r.body, want, "{path}");
    }
}
