//! Layer 7 row D8 (§7.1, §7.4, §7.6, §7.7; §13.1): the Service Discovery Registry.
//!
//! - the `_n111._tcp` record's presence rule and TXT over real fabrics on loopback (a recording
//!   fake daemon; never on Termux; an idle sync reads nothing; a refused register is not retried
//!   until an input changes; the shutdown goodbye withdraws a record a sync in flight
//!   registers), and the record as a second real mdns-sd daemon resolves it (skipped where the
//!   host has no multicast);
//! - a real browse stopped within 5 s;
//! - `POST /registry/browse` through the real dashboard app: `409 lan_off` without an mDNS
//!   daemon, the agent's bearer 401, 202 and the SSE `registry` frame with one;
//! - a member's browse matching its home hub's `k`: a forged record naming the hub at another
//!   node's address is refused by the handshake, one naming it at the dead address it holds is
//!   never taken for success; the hub's real new address is kept only after its handshake named
//!   the hub; an address event while the hub's redial back-off is below its maximum browses
//!   nothing, once it sits there it starts the same browse with no call; the hub back at the
//!   address it was enrolled at (the fabric's own copy, released at the first rehome) is tried;
//! - a forged flood naming the hub: at most `REHOME_MAX` tries per browse, a human's call during
//!   the lost-hub phase queues one more browse (its frame arrives), and the real record is
//!   reached by the next browse.

use mesh111::sealer::Sealer;
use node111::daemon::dashboard::api_v1::auth;
use node111::daemon::dashboard::mdns::{BrowseHit, MdnsOps, Record111, SERVICE_111};
use node111::daemon::dashboard::{routes, serve_listener, ApiConfig, ApiState, DashboardState};
use node111::daemon::fabric::{self, FabricConfig, FabricHandle, TcpTuning, FABRIC_DIR};
use node111::daemon::registry::{self, Registry};
use node111::daemon::DaemonLayout;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// ---------------------------------------------------------------- harness

fn api_in(dir: &Path, lan: bool) -> Arc<ApiState> {
    std::fs::create_dir_all(dir.join("state")).unwrap();
    let layout = DaemonLayout {
        root: dir.to_path_buf(),
        etc: dir.join("etc"),
    };
    let mut cfg = ApiConfig::new(layout, lan);
    cfg.persist_auth = false;
    ApiState::new(Arc::new(RwLock::new(DashboardState::new("t"))), cfg)
}

fn fabric_cfg(dir: &Path, hub: bool, backoff_max: Duration) -> FabricConfig {
    FabricConfig {
        fabric_dir: dir.join(FABRIC_DIR),
        identity_dir: dir.join("identity"),
        state_dir: dir.join("state"),
        listen: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        tcp: TcpTuning {
            refuse_loopback: false,
            backoff_min: Duration::from_millis(50),
            backoff_max,
            ..TcpTuning::default()
        },
        hub_default: hub,
        enroll_timeout: Duration::from_secs(5),
        housekeeping: Duration::from_secs(3600),
        shutdown_grace: Duration::from_secs(5),
        offer_host: None,
    }
}

async fn start_fabric(dir: &Path, api: &Arc<ApiState>, hub: bool) -> Arc<FabricHandle> {
    start_fabric_with(dir, api, hub, Duration::from_millis(400)).await
}

async fn start_fabric_with(
    dir: &Path,
    api: &Arc<ApiState>,
    hub: bool,
    backoff_max: Duration,
) -> Arc<FabricHandle> {
    let id = node111::identity::generate(&dir.join("identity")).unwrap();
    let h = fabric::start(fabric_cfg(dir, hub, backoff_max), Some(id), api).await;
    assert!(h.running(), "{:?}", h.down_reason());
    h
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

async fn until(what: &str, within: Duration, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + within;
    while !f() {
        assert!(Instant::now() < end, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A recording daemon: registrations, withdrawals and browses; a browse answers `hits`. With
/// `refuse` a register fails; `delay_ms` holds a register that long (a sync in flight).
#[derive(Default)]
struct Fake {
    hits: Mutex<Vec<BrowseHit>>,
    log: Mutex<Vec<String>>,
    records: Mutex<Vec<Record111>>,
    refuse: AtomicBool,
    delay_ms: AtomicUsize,
}

impl MdnsOps for Fake {
    fn register(&self, rec: &Record111) -> Result<String, String> {
        let ms = self.delay_ms.load(Ordering::SeqCst);
        if ms > 0 {
            std::thread::sleep(Duration::from_millis(ms as u64));
        }
        if self.refuse.load(Ordering::SeqCst) {
            self.log.lock().unwrap().push("refused".into());
            return Err("the daemon refused it".into());
        }
        self.records.lock().unwrap().push(rec.clone());
        self.log.lock().unwrap().push("register".into());
        Ok(format!("{}.{SERVICE_111}", rec.instance))
    }
    fn unregister(&self, fullname: &str, _wait: Duration) {
        self.log
            .lock()
            .unwrap()
            .push(format!("unregister {fullname}"));
    }
    fn browse(&self, service: &str, limit: Duration) -> Result<Vec<BrowseHit>, String> {
        assert_eq!(service, SERVICE_111);
        assert!(limit <= registry::BROWSE_FOR, "§7.1: at most 5 s");
        self.log.lock().unwrap().push("browse".into());
        // a browse takes its time (bounded here so the route test stays quick)
        std::thread::sleep(limit.min(Duration::from_millis(200)));
        Ok(self.hits.lock().unwrap().clone())
    }
}

fn hint(instance: &str, k: &str, at: SocketAddr) -> BrowseHit {
    let SocketAddr::V4(v4) = at else {
        panic!("v4 only")
    };
    BrowseHit {
        fullname: format!("{instance}.{SERVICE_111}"),
        port: at.port(),
        v4: vec![*v4.ip()],
        txt: vec![
            ("v".into(), "1".into()),
            ("r".into(), "0002".into()),
            ("k".into(), k.into()),
        ],
    }
}

/// One HTTP/1.1 request to the app on `port` from loopback.
async fn request(port: u16, method: &str, path: &str, headers: &[(&str, &str)]) -> (u16, String) {
    let mut conn = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if method != "GET" {
        req.push_str("Content-Type: application/json\r\nContent-Length: 2\r\n\r\n{}");
    } else {
        req.push_str("\r\n");
    }
    conn.write_all(req.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), conn.read_to_end(&mut raw))
        .await
        .unwrap()
        .unwrap();
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        let mut out = String::new();
        let mut rest = body;
        while let Some((size, after)) = rest.split_once("\r\n") {
            let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
            if n == 0 || after.len() < n {
                break;
            }
            out.push_str(&after[..n]);
            rest = after[n..].trim_start_matches("\r\n");
        }
        out
    } else {
        body.to_string()
    };
    (status, body)
}

async fn serve(api: &Arc<ApiState>) -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve_listener(listener, routes::app(Arc::clone(api))));
    port
}

fn registry_frames(api: &ApiState) -> Vec<serde_json::Value> {
    api.events
        .subscribe(Some(api.events.base()))
        .replay
        .iter()
        .filter(|f| f.event == "registry")
        .map(|f| serde_json::from_str(&f.data).unwrap())
        .collect()
}

// ---------------------------------------------------------------- the record (§7.1)

/// Present only while `discovery.lan_record`, an mDNS daemon (a LAN-bound dashboard) and a
/// LAN-bound fabric listener; TXT `v=1`, `r`, `k` (78 B); the instance is the L2 label, the
/// SRV port the listener's. Never registered without the daemon (the registry starts none).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_record_follows_its_presence_rule_and_carries_the_txt() {
    let (hd, md) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (ha, ma) = (api_in(hd.path(), true), api_in(md.path(), true));
    let hub = start_fabric(hd.path(), &ha, true).await;
    let member = start_fabric(md.path(), &ma, false).await;
    let hub_id = hex(hub.sealer().unwrap().node_id().as_bytes());

    let fake = Arc::new(Fake::default());
    let reg = Registry::new(Some(fake.clone()), "111.local", false);
    reg.bind_fabric(&hub);
    // switch off: nothing
    reg.sync_record().await;
    assert!(reg.record().await.is_none());
    // switch on, a hub's listener: the record
    reg.set_lan_record(true);
    reg.sync_record().await;
    let rec = reg.record().await.expect("present");
    let port = hub.listen_addr().await.unwrap().port();
    assert_eq!(rec.port, port, "SRV = the fabric listener");
    assert_eq!(rec.host, "111.local");
    assert_eq!(
        rec.instance,
        id111::encoding::NodeIdDisplay::new(hub.sealer().unwrap().node_id().as_bytes()).label()
    );
    let txt = rec.txt();
    assert_eq!(txt[0].1, "1");
    assert_eq!(txt[2].1, hub_id);
    let roles = u16::from_str_radix(&txt[1].1, 16).unwrap();
    assert_ne!(roles & 0x0002, 0, "the signed hub bit");
    assert_eq!(rec.txt_wire_len(), 78);
    // an unchanged sync registers nothing again; a rename re-registers once
    reg.sync_record().await;
    assert_eq!(fake.log.lock().unwrap().len(), 1);
    reg.set_host("111-2.local.");
    reg.sync_record().await;
    assert_eq!(reg.record().await.unwrap().host, "111-2.local");
    assert_eq!(
        fake.log.lock().unwrap().len(),
        3,
        "{:?}",
        fake.log.lock().unwrap()
    );
    // switch off again: withdrawn
    reg.set_lan_record(false);
    reg.sync_record().await;
    assert!(reg.record().await.is_none());
    assert!(fake
        .log
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .starts_with("unregister"));

    // a member without a LAN listener: no record, whatever the switch says
    let fake_m = Arc::new(Fake::default());
    let reg_m = Registry::new(Some(fake_m.clone()), "111.local", false);
    reg_m.bind_fabric(&member);
    reg_m.set_lan_record(true);
    reg_m.sync_record().await;
    assert!(reg_m.record().await.is_none());
    assert!(fake_m.log.lock().unwrap().is_empty());

    // no daemon (the dashboard is not LAN-bound): never a record, nothing started
    let reg_n = Registry::new(None, "111.local", false);
    reg_n.bind_fabric(&hub);
    reg_n.set_lan_record(true);
    reg_n.sync_record().await;
    assert!(reg_n.record().await.is_none());

    // Termux (ruling 9; §7.1: no browse or answer there): a hub with the switch on and a
    // daemon carries no record, the daemon is never asked
    let fake_t = Arc::new(Fake::default());
    let reg_t = Registry::new(Some(fake_t.clone()), "111.local", true);
    reg_t.bind_fabric(&hub);
    reg_t.set_lan_record(true);
    reg_t.sync_record().await;
    assert!(reg_t.record().await.is_none());
    assert!(
        fake_t.log.lock().unwrap().is_empty(),
        "no register on Termux"
    );
    hub.shutdown().await;
    member.shutdown().await;
}

/// Ruling 8: an idle sync (the 30 s tick) clones no settings and verifies no announce; a
/// settings change reads them once (the announce, unchanged, is not verified again); a
/// register the daemon refused is not retried until an input changes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_idle_sync_reads_nothing_and_a_refused_record_waits_for_a_change() {
    let hd = tempfile::tempdir().unwrap();
    let ha = api_in(hd.path(), true);
    let hub = start_fabric(hd.path(), &ha, true).await;
    let fake = Arc::new(Fake::default());
    let reg = Registry::new(Some(fake.clone()), "111.local", false);
    reg.bind_fabric(&hub);
    reg.set_lan_record(true);
    reg.sync_record().await;
    assert!(reg.record().await.is_some());
    assert_eq!(reg.fabric_reads(), 2, "one settings read, one verify");
    let mut idle_checked = false;
    for _ in 0..5 {
        let (g0, r0) = (hub.pair_generation(), reg.fabric_reads());
        for _ in 0..3 {
            reg.sync_record().await;
        }
        if hub.pair_generation() == g0 {
            assert_eq!(reg.fabric_reads(), r0, "an idle sync reads nothing");
            idle_checked = true;
            break;
        }
    }
    assert!(idle_checked, "the pair generation never held still");
    let r0 = reg.fabric_reads();
    hub.sync_settings().await.unwrap();
    reg.sync_record().await;
    assert_eq!(
        reg.fabric_reads(),
        r0 + 1,
        "a settings change: one settings read, the same announce not verified again"
    );
    // a re-mint with no generation bump (the relay switches, D10's path): the record follows
    let (g0, r0) = (hub.pair_generation(), reg.fabric_reads());
    let roles0 = reg.record().await.unwrap().roles;
    assert!(hub
        .set_relay_roles(fabric::RelayRoles {
            offload_bit: true,
            ..fabric::RelayRoles::default()
        })
        .await
        .unwrap());
    assert_eq!(hub.pair_generation(), g0, "the re-mint bumps no generation");
    reg.sync_record().await;
    let roles1 = reg.record().await.unwrap().roles;
    assert_ne!(roles1, roles0, "the record carries the re-minted roles");
    assert_eq!(
        roles1 & fabric::sealer::ROLE_OFFLOAD,
        fabric::sealer::ROLE_OFFLOAD
    );
    assert_eq!(reg.fabric_reads(), r0 + 1, "only the new announce verified");

    // a refused register is tried once, not on every tick
    reg.set_lan_record(false);
    reg.sync_record().await;
    assert!(reg.record().await.is_none());
    fake.refuse.store(true, Ordering::SeqCst);
    reg.set_lan_record(true);
    for _ in 0..4 {
        reg.sync_record().await;
    }
    let refused = |f: &Fake| {
        f.log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| *l == "refused")
            .count()
    };
    assert_eq!(refused(&fake), 1, "{:?}", fake.log.lock().unwrap());
    // an input changes (a rename): tried again, once
    reg.set_host("111-2.local.");
    reg.sync_record().await;
    reg.sync_record().await;
    assert_eq!(refused(&fake), 2);
    // the daemon takes it once the inputs change again
    fake.refuse.store(false, Ordering::SeqCst);
    reg.set_host("111-3.local.");
    reg.sync_record().await;
    assert_eq!(reg.record().await.unwrap().host, "111-3.local");
    hub.shutdown().await;
}

/// Ruling 10: the shutdown goodbye is reliable: a sync in flight (its register still running)
/// finishes first and its record is the one withdrawn; no sync registers after it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_goodbye_withdraws_a_record_a_sync_in_flight_registers() {
    let hd = tempfile::tempdir().unwrap();
    let ha = api_in(hd.path(), true);
    let hub = start_fabric(hd.path(), &ha, true).await;
    let fake = Arc::new(Fake::default());
    let reg = Arc::new(Registry::new(Some(fake.clone()), "111.local", false));
    reg.bind_fabric(&hub);
    reg.set_lan_record(true);
    reg.sync_record().await;
    assert!(reg.record().await.is_some());
    // a rename's sync, its register slow
    fake.delay_ms.store(400, Ordering::SeqCst);
    reg.set_host("111-2.local.");
    let in_flight = tokio::spawn({
        let reg = Arc::clone(&reg);
        async move { reg.sync_record().await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let started = Instant::now();
    tokio::task::spawn_blocking({
        let reg = Arc::clone(&reg);
        move || reg.withdraw()
    })
    .await
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(2), "a bounded wait");
    in_flight.await.unwrap();
    assert!(
        reg.record().await.is_none(),
        "{:?}",
        fake.log.lock().unwrap()
    );
    let log = fake.log.lock().unwrap().clone();
    let last = fake.records.lock().unwrap().last().unwrap().clone();
    assert_eq!(
        last.host, "111-2.local",
        "the sync in flight registered the rename"
    );
    assert_eq!(
        log.last(),
        Some(&format!("unregister {}.{SERVICE_111}", last.instance)),
        "the record registered in flight got its goodbye: {log:?}"
    );
    // nothing registers after the goodbye
    fake.delay_ms.store(0, Ordering::SeqCst);
    reg.set_host("111-3.local.");
    reg.sync_record().await;
    assert!(reg.record().await.is_none());
    assert_eq!(fake.log.lock().unwrap().len(), log.len());
    hub.shutdown().await;
}

/// The record on a real daemon, under the real type `_n111._tcp` (the owner's ruling: mdns-sd
/// refuses `_111._tcp`, RFC 6335 §5.1), resolved by a second daemon: A records only, the three
/// TXT keys. The resolution part is skipped where the host offers no multicast path between
/// two daemons.
#[cfg(feature = "mdns")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_real_daemon_answers_the_record_with_a_records_only() {
    let (Ok(a), Ok(b)) = (mdns_sd::ServiceDaemon::new(), mdns_sd::ServiceDaemon::new()) else {
        eprintln!("SKIPPED: no mDNS daemon on this host");
        return;
    };
    for d in [&a, &b] {
        let _ = d.disable_interface(mdns_sd::IfKind::IPv6);
    }
    let node_id = [0x5a; 32];
    let rec = Record111 {
        instance: id111::encoding::NodeIdDisplay::new(&node_id).label(),
        host: format!("d8-test-{}.local", std::process::id()),
        port: 11183,
        roles: 0x0003,
        node_id,
    };
    assert_eq!(SERVICE_111, "_n111._tcp.local.");
    let full = MdnsOps::register(&a, &rec).expect("the real type registers");
    assert_eq!(full, format!("{}.{SERVICE_111}", rec.instance));
    let hits = tokio::task::spawn_blocking({
        let b = b.clone();
        move || MdnsOps::browse(&b, SERVICE_111, Duration::from_secs(3)).unwrap()
    })
    .await
    .unwrap();
    MdnsOps::unregister(&a, &full, Duration::from_secs(1));
    let _ = (a.shutdown(), b.shutdown());
    let Some(hit) = hits.iter().find(|h| h.fullname == full) else {
        eprintln!("SKIPPED: the second daemon heard nothing (no multicast here): {hits:?}");
        return;
    };
    assert_eq!(hit.port, 11183);
    assert!(!hit.v4.is_empty(), "A records");
    let mut txt = hit.txt.clone();
    txt.sort();
    assert_eq!(
        txt,
        vec![
            ("k".to_string(), "5a".repeat(32)),
            ("r".to_string(), "0003".to_string()),
            ("v".to_string(), "1".to_string()),
        ]
    );
}

// ---------------------------------------------------------------- the browse (§7.1)

/// A real browse returns and has sent `stop_browse` within 5 s.
#[cfg(feature = "mdns")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_real_browse_is_stopped_within_five_seconds() {
    let Ok(d) = mdns_sd::ServiceDaemon::new() else {
        eprintln!("SKIPPED: no mDNS daemon on this host");
        return;
    };
    let started = Instant::now();
    let r = tokio::task::spawn_blocking({
        let d = d.clone();
        move || MdnsOps::browse(&d, SERVICE_111, registry::BROWSE_FOR)
    })
    .await
    .unwrap();
    let took = started.elapsed();
    assert!(r.is_ok(), "{r:?}");
    assert!(
        took >= registry::BROWSE_FOR && took < registry::BROWSE_FOR + Duration::from_millis(700),
        "browse 5 s, then stop_browse (confirmed): {took:?}"
    );
    // the stop was taken: a new browse of the same type is accepted at once
    let again = tokio::task::spawn_blocking({
        let d = d.clone();
        move || MdnsOps::browse(&d, SERVICE_111, Duration::from_millis(100))
    })
    .await
    .unwrap();
    assert!(again.is_ok());
    let _ = d.shutdown();
}

/// `POST /registry/browse` through the real app: `409 lan_off` with no mDNS daemon (and no
/// registry at all), the agent's bearer 401, 202 with a daemon and the rows on the `registry`
/// frame and on `GET /registry`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_browse_route_answers_lan_off_bearer_401_and_202() {
    let dir = tempfile::tempdir().unwrap();
    // no registry in ext: lan_off; GET is an empty list
    let bare = api_in(dir.path(), false);
    let port = serve(&bare).await;
    let (s, body) = request(port, "POST", "/api/v1/registry/browse", &[]).await;
    assert_eq!(s, 409, "{body}");
    assert!(body.contains("\"lan_off\""), "{body}");
    let (s, body) = request(port, "GET", "/api/v1/registry", &[]).await;
    assert_eq!(s, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v, serde_json::json!({"schema": 1, "rows": []}));

    // a registry with no daemon (LAN sharing off): lan_off, nothing browsed
    let dir2 = tempfile::tempdir().unwrap();
    let off = api_in(dir2.path(), false);
    off.insert_ext(Arc::new(Registry::new(None, "111.local", false)));
    let port = serve(&off).await;
    let (s, body) = request(port, "POST", "/api/v1/registry/browse", &[]).await;
    assert_eq!(s, 409, "{body}");
    assert!(body.contains("\"lan_off\""), "{body}");

    // a daemon: the bearer is refused, a person gets 202 and the frame
    let dir3 = tempfile::tempdir().unwrap();
    let on = api_in(dir3.path(), true);
    let fake = Arc::new(Fake::default());
    *fake.hits.lock().unwrap() = vec![hint(
        "abcd-efgh-ij",
        &"cd".repeat(32),
        "192.168.1.9:11183".parse().unwrap(),
    )];
    let reg = Arc::new(Registry::new(Some(fake.clone()), "111.local", false));
    reg.bind_api(&on);
    on.insert_ext(Arc::clone(&reg));
    let token = auth::DaemonToken::mint().unwrap();
    on.insert_ext(token.clone());
    let port = serve(&on).await;
    let bearer = format!("Bearer {}", token.as_str());
    let (s, body) = request(
        port,
        "POST",
        "/api/v1/registry/browse",
        &[("Authorization", &bearer)],
    )
    .await;
    assert_eq!(s, 401, "{body}");
    assert!(body.contains("unauthenticated"), "{body}");
    assert!(
        fake.log.lock().unwrap().is_empty(),
        "the bearer started nothing"
    );
    // the bearer reads
    let (s, _) = request(
        port,
        "GET",
        "/api/v1/registry",
        &[("Authorization", &bearer)],
    )
    .await;
    assert_eq!(s, 200);
    let (s, body) = request(port, "POST", "/api/v1/registry/browse", &[]).await;
    assert_eq!(s, 202, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["schema"], 1);
    assert!(v["started"].is_boolean());
    until("the registry frame", Duration::from_secs(5), || {
        !registry_frames(&on).is_empty()
    })
    .await;
    let frame = &registry_frames(&on)[0];
    assert_eq!(frame["rows"][0]["answered_by"], "hint");
    assert_eq!(frame["rows"][0]["address"], "192.168.1.9:11183");
    assert_eq!(frame["rows"][0]["roles"], serde_json::json!(["hub"]));
    let (s, body) = request(port, "GET", "/api/v1/registry", &[]).await;
    assert_eq!(s, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["rows"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["rows"][0]["node_id"], "cd".repeat(32));
}

// ---------------------------------------------------------------- lost hub (§7.7)

/// A TCP relay to `to` whose accept loop and live connections can be killed.
struct Relay {
    at: SocketAddr,
    tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl Relay {
    async fn new(to: SocketAddr) -> Relay {
        Relay::on("127.0.0.1:0".parse().unwrap(), to).await
    }

    /// A relay listening at `bind` (an address an earlier relay used: the hub comes back).
    async fn on(bind: SocketAddr, to: SocketAddr) -> Relay {
        let l = tokio::net::TcpListener::bind(bind).await.unwrap();
        let at = l.local_addr().unwrap();
        let tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> = Arc::default();
        let t2 = Arc::clone(&tasks);
        let accept = tokio::spawn(async move {
            while let Ok((mut inbound, _)) = l.accept().await {
                let h = tokio::spawn(async move {
                    if let Ok(mut out) = tokio::net::TcpStream::connect(to).await {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut out).await;
                    }
                });
                t2.lock().unwrap().push(h);
            }
        });
        tasks.lock().unwrap().push(accept);
        Relay { at, tasks }
    }

    fn kill(&self) {
        for t in self.tasks.lock().unwrap().drain(..) {
            t.abort();
        }
    }
}

/// Loopback is on-link here (the tests' fabrics allow loopback peers; production refuses it).
fn lo_nets() -> registry::NetsFn {
    Arc::new(|| vec![(std::net::IpAddr::from([127, 0, 0, 1]), 8)])
}

/// A hub, a member enrolled through a first path to it (a relay), and the member's registry
/// over a recording daemon. The member's redial back-off tops out at `backoff_max`.
struct Lost {
    _dirs: Vec<tempfile::TempDir>,
    hub: Arc<FabricHandle>,
    m: Arc<FabricHandle>,
    ma: Arc<ApiState>,
    at: SocketAddr,
    hub_k: String,
    old: Relay,
    fake: Arc<Fake>,
    reg: Arc<Registry>,
}

async fn lost_hub_setup(backoff_max: Duration, rehome_wait: Duration) -> Lost {
    let (hd, md) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (ha, ma) = (api_in(hd.path(), true), api_in(md.path(), true));
    let hub = start_fabric(hd.path(), &ha, true).await;
    let m = start_fabric_with(md.path(), &ma, false, backoff_max).await;
    let at = hub.listen_addr().await.unwrap();
    let hub_k = hex(hub.sealer().unwrap().node_id().as_bytes());
    let old = Relay::new(at).await;
    let offer = hub.fabric().unwrap().offer(Some(old.at)).await.unwrap();
    m.fabric()
        .unwrap()
        .enroll(old.at, &offer.code, Some(&offer.card))
        .await
        .unwrap();
    m.sync_settings().await.unwrap();
    assert_eq!(
        m.persisted().fabric.home_hub.and_then(|h| h.address),
        Some(old.at)
    );
    let fake = Arc::new(Fake::default());
    let reg = Arc::new(
        Registry::new(Some(fake.clone()), "111.local", false)
            .with_timings(Duration::from_millis(200), rehome_wait)
            .with_nets(lo_nets()),
    );
    reg.bind_api(&ma);
    reg.bind_fabric(&m);
    hub_is(&m, true, "the hub connected").await;
    Lost {
        _dirs: vec![hd, md],
        hub,
        m,
        ma,
        at,
        hub_k,
        old,
        fake,
        reg,
    }
}

/// "Find my hub" on a member: every hint is matched against the home hub's `k`; a forged
/// record naming the hub at another node's address is refused by the handshake there, one
/// naming it at the dead address the member holds is skipped (never "found" without a
/// handshake: ruling 1), one named like the hub's NodeId never collides with the hub's signed
/// row (ruling 6); the hub's real new address is kept only once its handshake named the hub.
/// Then, the hub lost again: an address event while the redial back-off is below its maximum
/// browses nothing (ruling 5), once it sits there the same browse starts with no call. Last,
/// the hub back at its first address (A → B → C → A): the fabric's stale copy is no held
/// address, so it is tried and the hub connects there (second review).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_member_s_browse_keeps_its_hub_s_address_only_after_the_handshake() {
    let od = tempfile::tempdir().unwrap();
    let oa = api_in(od.path(), true);
    let other = start_fabric(od.path(), &oa, true).await;
    let Lost {
        _dirs,
        hub,
        m,
        ma,
        at,
        hub_k,
        old,
        fake,
        reg,
    } = lost_hub_setup(Duration::from_secs(2), Duration::from_secs(3)).await;
    // the hub up: an address event costs one status read and browses nothing
    assert!(!reg.hub_lost().await);
    reg.on_address_event();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        reg.browses(),
        0,
        "S23: a renew with the hub up costs nothing"
    );

    old.kill();
    hub_is(&m, false, "the hub disconnected").await;
    // ruling 1: a record naming the hub at the dead address the member holds is no candidate
    // (trying it "succeeds" with no handshake); the try goes to the next record
    let sink = Sink::new().await;
    *fake.hits.lock().unwrap() = vec![hint("stale", &hub_k, old.at), hint("next", &hub_k, sink.at)];
    assert_eq!(reg.browse(registry::Why::Asked), Ok(true));
    until("the browse ended", Duration::from_secs(20), || {
        !reg.browsing()
    })
    .await;
    assert_eq!(reg.rehome_tries(), 1, "the held address is never tried");
    assert!(
        sink.hits.load(Ordering::SeqCst) > 0,
        "the next record was tried"
    );
    assert_eq!(
        m.persisted().fabric.home_hub.and_then(|h| h.address),
        Some(old.at),
        "nothing found, nothing kept"
    );
    sink.task.abort();

    // the hub "moved": reachable through a new path; forged records name it at the other
    // node, at the dead address the member holds (first: the order a flood would use), and
    // under an instance spelled like the hub's NodeId
    let moved = Relay::new(at).await;
    let wrong = other.listen_addr().await.unwrap();
    *fake.hits.lock().unwrap() = vec![
        hint("stale", &hub_k, old.at),
        hint("forged", &hub_k, wrong),
        hint(&hub_k, &hub_k, "8.8.8.8:11183".parse().unwrap()),
        hint("real", &hub_k, moved.at),
        hint("someone", &"ee".repeat(32), "127.0.0.1:9".parse().unwrap()),
    ];
    hub_is(&m, false, "the hub disconnected").await;

    // "Find my hub" = the browse route's call on a member with a home hub
    assert_eq!(reg.browse(registry::Why::Asked), Ok(true));
    until("the browse ended", Duration::from_secs(20), || {
        !reg.browsing()
    })
    .await;
    assert_eq!(
        m.persisted().fabric.home_hub.and_then(|h| h.address),
        Some(moved.at),
        "kept only after the handshake there named the hub"
    );
    let st = m.status_json().await.unwrap();
    assert_eq!(st["hub"]["address"], moved.at.to_string());
    assert!(
        reg.rehome_tries() <= 1 + 2,
        "the held address and the public one are never tried: {}",
        reg.rehome_tries()
    );
    let rows = reg.rows_json();
    assert_eq!(
        rows.len(),
        5 + 1,
        "five hints and the hub's signed row: {rows:?}"
    );
    let mut ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), rows.len(), "ids are unique: {rows:?}");
    let signed: Vec<_> = rows
        .iter()
        .filter(|r| r["answered_by"] == "signed")
        .collect();
    assert_eq!(signed.len(), 1);
    assert_eq!(signed[0]["node_id"], hub_k);
    assert_eq!(signed[0]["id"], hub_k, "the NodeId when known");
    assert!(signed[0]["address"].is_null());
    assert!(signed[0]["roles"]
        .as_array()
        .unwrap()
        .contains(&"hub".into()));
    // one DNS label holds 63 characters: the lookalike's label is the NodeId less its last digit
    let lookalike: Vec<_> = rows
        .iter()
        .filter(|r| r["label"] == hub_k[..registry::LABEL_MAX])
        .collect();
    assert_eq!(lookalike.len(), 1);
    assert_eq!(
        lookalike[0]["answered_by"], "hint",
        "never passes for the signed row"
    );
    assert_ne!(lookalike[0]["id"], hub_k);

    // the hub lost again: below the maximum back-off an address event browses nothing
    moved.kill();
    hub_is(&m, false, "the hub disconnected again").await;
    let back = Relay::new(at).await;
    *fake.hits.lock().unwrap() = vec![hint("real", &hub_k, back.at)];
    let before = reg.browses();
    assert!(!reg.hub_lost().await, "the back-off is still growing");
    reg.on_address_event();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        reg.browses(),
        before,
        "no browse below the maximum back-off"
    );
    assert!(!reg.hub_lost().await, "still below the 2 s maximum");
    // once it sits at its maximum: the same browse with no call
    let end = Instant::now() + Duration::from_secs(10);
    while !reg.hub_lost().await {
        assert!(
            Instant::now() < end,
            "the back-off never reached its maximum"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    reg.on_address_event();
    until("a browse with no call", Duration::from_secs(5), || {
        reg.browses() > before
    })
    .await;
    until("the browse ended", Duration::from_secs(15), || {
        !reg.browsing()
    })
    .await;
    assert_eq!(
        m.persisted().fabric.home_hub.and_then(|h| h.address),
        Some(back.at)
    );
    // the hub comes back to the address it was enrolled at: the fabric's own copy, released
    // at the first rehome, is no held address and stays a candidate (A to B to A)
    back.kill();
    hub_is(&m, false, "the hub disconnected a third time").await;
    let home = Relay::on(old.at, at).await;
    *fake.hits.lock().unwrap() = vec![hint("real", &hub_k, old.at)];
    let tries = reg.rehome_tries();
    assert_eq!(reg.browse(registry::Why::Asked), Ok(true));
    until("the browse ended", Duration::from_secs(20), || {
        !reg.browsing()
    })
    .await;
    assert_eq!(reg.rehome_tries(), tries + 1, "the old address was tried");
    // found: the handshake there named the hub; the fabric's own copy is that address, so the
    // override at the last address is dropped (the shown address is the first one again)
    hub_is(&m, true, "the hub connected at its first address").await;
    assert_eq!(
        m.persisted().fabric.home_hub.and_then(|h| h.address),
        Some(old.at),
        "the override at the address before is dropped"
    );
    home.kill();
    drop(ma);
    for h in [hub, other, m] {
        h.shutdown().await;
    }
}

/// A listener that counts the connections it is offered and closes each at once (no
/// handshake ever names anyone there).
struct Sink {
    at: SocketAddr,
    hits: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Sink {
    async fn new() -> Sink {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let at = l.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = Arc::clone(&hits);
        let task = tokio::spawn(async move {
            while let Ok((c, _)) = l.accept().await {
                h.fetch_add(1, Ordering::SeqCst);
                drop(c);
            }
        });
        Sink { at, hits, task }
    }
}

/// Rulings 1, 4 and 5 (`REHOME_MAX`): six forged records name the hub at addresses where no
/// handshake names it. One browse tries at most `REHOME_MAX` of them; a human's call during
/// that lost-hub phase answers `started: false` and queues one more browse, whose `registry`
/// frame does arrive; that browse hears the real record after the same six (arrival order
/// would spend every try on the flood again) and tries the addresses not yet refused first,
/// so the real one is reached.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_forged_flood_spends_at_most_rehome_max_tries_and_cannot_starve_the_hub() {
    let Lost {
        _dirs,
        hub,
        m,
        ma,
        at,
        hub_k,
        old,
        fake,
        reg,
    } = lost_hub_setup(Duration::from_millis(400), Duration::from_millis(700)).await;
    old.kill();
    hub_is(&m, false, "the hub disconnected").await;
    let mut sinks = Vec::new();
    for _ in 0..6 {
        sinks.push(Sink::new().await);
    }
    let moved = Relay::new(at).await;
    let flood: Vec<BrowseHit> = sinks
        .iter()
        .enumerate()
        .map(|(i, s)| hint(&format!("forged-{i}"), &hub_k, s.at))
        .collect();
    *fake.hits.lock().unwrap() = flood.clone();

    assert_eq!(reg.browse(registry::Why::Asked), Ok(true));
    until("the first frame", Duration::from_secs(5), || {
        registry_frames(&ma).len() == 1
    })
    .await;
    // a human asks while the lost-hub phase runs
    until("the lost-hub phase", Duration::from_secs(5), || {
        reg.rehoming()
    })
    .await;
    let mut hits = flood;
    hits.push(hint("real", &hub_k, moved.at)); // the hub answers now, heard after the flood
    *fake.hits.lock().unwrap() = hits;
    assert_eq!(reg.browse(registry::Why::Asked), Ok(false));
    until("the queued browse", Duration::from_secs(10), || {
        reg.browses() == 2
    })
    .await;
    assert_eq!(
        reg.rehome_tries() as usize,
        registry::REHOME_MAX,
        "one browse tries REHOME_MAX of six"
    );
    until("the queued browse's frame", Duration::from_secs(10), || {
        registry_frames(&ma).len() == 2
    })
    .await;
    until("the task ended", Duration::from_secs(20), || {
        !reg.browsing()
    })
    .await;
    assert!(reg.rehome_tries() as usize <= 2 * registry::REHOME_MAX);
    assert_eq!(
        m.persisted().fabric.home_hub.and_then(|h| h.address),
        Some(moved.at),
        "the real record was reached by the second browse"
    );
    let contacted = sinks
        .iter()
        .filter(|s| s.hits.load(Ordering::SeqCst) > 0)
        .count();
    assert!(
        contacted <= reg.rehome_tries() as usize,
        "only tried addresses were dialled"
    );
    for s in &sinks {
        s.task.abort();
    }
    moved.kill();
    for h in [hub, m] {
        h.shutdown().await;
    }
}

/// Wait until the member's hub connection (the fabric's status) reads `up`.
async fn hub_is(m: &FabricHandle, up: bool, what: &str) {
    let end = Instant::now() + Duration::from_secs(10);
    while m.fabric().unwrap().status().await.unwrap().hub.connected != up {
        assert!(Instant::now() < end, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// ---------------------------------------------------------------- the Pi-floor figure (§16)

/// The mdns-sd daemon thread's context switches over 30 s with the default 5 s IP check and
/// with the check at 0 (the re-arm's resting state): the row's "−12 wakeups/min where the bind
/// succeeds". A measurement, not a gate: `cargo test -p node111 --test registry_mdns -- --ignored
/// --nocapture`.
#[cfg(feature = "mdns")]
#[test]
#[ignore = "a 60 s measurement"]
fn measure_the_mdns_daemon_s_wakeups_with_and_without_its_ip_check() {
    fn switches() -> Option<u64> {
        for t in std::fs::read_dir("/proc/self/task").ok()?.flatten() {
            let comm = std::fs::read_to_string(t.path().join("comm")).unwrap_or_default();
            if comm.trim() == "mDNS_daemon" {
                let status = std::fs::read_to_string(t.path().join("status")).ok()?;
                return Some(
                    status
                        .lines()
                        .filter(|l| l.contains("ctxt_switches"))
                        .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
                        .sum(),
                );
            }
        }
        None
    }
    for interval in [node111::daemon::dashboard::mdns::DEFAULT_IP_CHECK_SECS, 0] {
        let d = mdns_sd::ServiceDaemon::new().unwrap();
        let _ = d.disable_interface(mdns_sd::IfKind::IPv6);
        d.set_ip_check_interval(interval).unwrap();
        std::thread::sleep(Duration::from_secs(2));
        let a = switches().unwrap();
        std::thread::sleep(Duration::from_secs(30));
        let b = switches().unwrap();
        let rx = d.shutdown().unwrap();
        let _ = rx.recv_timeout(Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(300));
        println!(
            "MEASURE ip_check_interval={interval}s: {} context switches / min",
            (b - a) * 2
        );
    }
}
