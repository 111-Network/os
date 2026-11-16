//! Layer 7 row D8 (§7.1, §7.4, §7.6, §7.7; §13.1): the Service Discovery Registry.
//!
//! - the `_n111._tcp` record's presence rule and TXT over real fabrics on loopback (a recording
//!   fake daemon; never on Termux; an idle sync reads nothing; a refused register is not retried
//!   until an input changes; the shutdown goodbye withdraws a record a sync in flight
//!   registers), and the record as a second real mdns-sd daemon resolves it (skipped where the
//!   host has no multicast);
//! - an update of the registered record (a re-minted role bit, a renamed host) renames nothing
//!   on a real daemon in a network namespace of its own: no goodbye before an update (G8 phase
//!   12's busy loop); a withdrawn record comes back only after its goodbye's guard (the fake and
//!   the real daemon);
//! - a rename of the dashboard's host on its real announcer with the record (a real conflict,
//!   then a forged answer inside the probe the record's sync opens) stays one rename: no
//!   instance renamed on the wire; after a re-number the dashboard answers under the name the
//!   URL list shows (row D8 follow-up, review findings 1, 4, 6, 8);
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
use node111::daemon::housekeeping::{self, Housekeeping};
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

/// A started fabric with the daemon's one housekeeping interval beside it (Layer 8 row D1),
/// spawned where `daemon/mod.rs` spawns it and alive as long as the fabric; it derefs to the
/// handle, and [`Fab::shutdown`] stops the interval first, as the daemon does.
struct Fab {
    h: Arc<FabricHandle>,
    house: Mutex<Option<Housekeeping>>,
}

impl std::ops::Deref for Fab {
    type Target = Arc<FabricHandle>;
    fn deref(&self) -> &Arc<FabricHandle> {
        &self.h
    }
}

impl Fab {
    /// Stop as the daemon stops (`daemon/mod.rs`): the housekeeping interval, so no pass races
    /// the close, then the fabric.
    async fn shutdown(&self) {
        let house = self.house.lock().unwrap().take();
        if let Some(house) = house {
            house.stop().await;
        }
        self.h.shutdown().await;
    }
}

async fn start_fabric(dir: &Path, api: &Arc<ApiState>, hub: bool) -> Fab {
    start_fabric_with(dir, api, hub, Duration::from_millis(400)).await
}

async fn start_fabric_with(
    dir: &Path,
    api: &Arc<ApiState>,
    hub: bool,
    backoff_max: Duration,
) -> Fab {
    let id = node111::identity::generate(&dir.join("identity")).unwrap();
    let h = fabric::start(fabric_cfg(dir, hub, backoff_max), Some(id), api).await;
    assert!(h.running(), "{:?}", h.down_reason());
    // the daemon's one housekeeping interval, spawned where `daemon/mod.rs` spawns it
    let house = housekeeping::spawn(api, &h);
    Fab {
        h,
        house: Mutex::new(Some(house)),
    }
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
    // an unchanged sync registers nothing again; a rename registers once more, in place: no
    // goodbye first (the daemon would read it back as a conflict: the G8 phase 12 busy loop)
    reg.sync_record().await;
    assert_eq!(fake.log.lock().unwrap().len(), 1);
    reg.set_host("111-2.local.");
    reg.sync_record().await;
    assert_eq!(reg.record().await.unwrap().host, "111-2.local");
    assert_eq!(
        fake.log.lock().unwrap().as_slice(),
        ["register", "register"],
        "an update is a register alone"
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

    // a refused register is tried once, not on every tick (once the withdrawal's goodbye is
    // past its guard: before that no sync registers the instance at all)
    reg.set_lan_record(false);
    reg.sync_record().await;
    assert!(reg.record().await.is_none());
    tokio::time::sleep(registry::GOODBYE_GUARD + Duration::from_millis(50)).await;
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

/// Review finding 5 (row D8 follow-up): a sync never registers an instance whose goodbye left
/// less than `GOODBYE_GUARD` ago (mdns-sd resends the goodbye 120 ms later and reads it back as
/// a conflict with the new registration's probe); the first sync after the guard does. The
/// guard holds for a withdrawal (the switch off), not for an update of the registered record.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_withdrawn_record_comes_back_only_after_its_goodbye() {
    let hd = tempfile::tempdir().unwrap();
    let ha = api_in(hd.path(), true);
    let hub = start_fabric(hd.path(), &ha, true).await;
    let fake = Arc::new(Fake::default());
    let reg = Registry::new(Some(fake.clone()), "111.local", false);
    reg.bind_fabric(&hub);
    reg.set_lan_record(true);
    reg.sync_record().await;
    let rec = reg.record().await.expect("present");
    // an update right after the registration is not held back (no goodbye involved)
    reg.set_host("111-2.local.");
    reg.sync_record().await;
    assert_eq!(reg.record().await.unwrap().host, "111-2.local");
    // off, then on again at once: the goodbye, and no registration while it is in flight
    reg.set_lan_record(false);
    reg.sync_record().await;
    let off = Instant::now();
    assert!(reg.record().await.is_none());
    reg.set_lan_record(true);
    for _ in 0..3 {
        reg.sync_record().await;
    }
    let log = fake.log.lock().unwrap().clone();
    assert!(
        off.elapsed() < registry::GOODBYE_GUARD,
        "the syncs ran inside the guard"
    );
    assert!(reg.record().await.is_none(), "{log:?}");
    assert_eq!(
        log,
        [
            "register".to_string(),
            "register".to_string(),
            format!("unregister {}.{SERVICE_111}", rec.instance)
        ],
        "nothing registered while the goodbye is in flight"
    );
    // the guard has passed: the next sync registers it
    tokio::time::sleep(
        registry::GOODBYE_GUARD.saturating_sub(off.elapsed()) + Duration::from_millis(20),
    )
    .await;
    reg.sync_record().await;
    assert_eq!(reg.record().await.unwrap().host, "111-2.local");
    assert_eq!(fake.log.lock().unwrap().last().unwrap(), "register");
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

// ---------------------------------------------------------------- real daemons in a namespace (row D8 follow-up)

/// One `ip` (iproute2) command inside the namespace.
#[cfg(feature = "mdns")]
fn netns_ip(args: &str) -> Result<(), String> {
    let st = std::process::Command::new("ip")
        .args(args.split(' '))
        .status()
        .map_err(|e| format!("no `ip` (iproute2): {e}"))?;
    st.success()
        .then_some(())
        .ok_or_else(|| format!("`ip {args}` failed: {st}"))
}

/// Run `body` on a thread of its own in a fresh network namespace (it dies with the thread and
/// the threads it spawned, the mdns-sd daemons among them): `lo` up and a veth pair `veth` /
/// `peer`, `veth` holding `cidr`, `peer` up with no address. `Err(why)` = the box cannot run it
/// (no namespace, no `ip`, no multicast path inside it); the caller prints `SKIPPED: <why>`.
#[cfg(feature = "mdns")]
fn in_netns<F, Fut>(
    veth: &'static str,
    peer: &'static str,
    cidr: &'static str,
    body: F,
) -> Result<(), String>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let run = move || -> Result<(), String> {
        // SAFETY: unshare(2) with CLONE_NEWNET moves this thread (and what it spawns) only.
        if unsafe { libc::unshare(libc::CLONE_NEWNET) } != 0 {
            return Err(format!(
                "no network namespace: {}",
                std::io::Error::last_os_error()
            ));
        }
        netns_ip("link set lo up")?;
        netns_ip(&format!("link add {veth} type veth peer name {peer}"))?;
        netns_ip(&format!("addr add {cidr} dev {veth}"))?;
        netns_ip(&format!("link set {veth} up"))?;
        netns_ip(&format!("link set {peer} up"))?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let out = rt.block_on(body());
        rt.shutdown_timeout(Duration::from_secs(2));
        out
    };
    match std::thread::spawn(run).join() {
        Ok(out) => out,
        Err(p) => std::panic::resume_unwind(p),
    }
}

/// The `NameChange`s `monitor` reports within `within` (a probe takes 750 ms; a rename's
/// `NameChange` comes when the probe of the new name ends).
#[cfg(feature = "mdns")]
async fn renames(
    monitor: &mdns_sd::Receiver<mdns_sd::DaemonEvent>,
    within: Duration,
) -> Vec<String> {
    let end = Instant::now() + within;
    let mut seen = Vec::new();
    while Instant::now() < end {
        while let Ok(ev) = monitor.try_recv() {
            if let mdns_sd::DaemonEvent::NameChange(c) = ev {
                seen.push(format!(
                    "{} -> {} ({:?} on {})",
                    c.original, c.new_name, c.rr_type, c.intf_name
                ));
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    seen
}

/// Stop a daemon and wait (bounded) for its thread to confirm.
#[cfg(feature = "mdns")]
fn stop_daemon(d: &mdns_sd::ServiceDaemon) {
    if let Ok(rx) = d.shutdown() {
        let _ = rx.recv_timeout(Duration::from_secs(2));
    }
}

/// A fresh mdns-sd daemon (IPv4 only), its monitor, and a registry over it carrying `hub`'s
/// record under `host`; the first registration's probes are waited out and rename nothing.
#[cfg(feature = "mdns")]
async fn fresh_record(
    hub: &Arc<FabricHandle>,
    host: &str,
) -> Result<
    (
        mdns_sd::ServiceDaemon,
        mdns_sd::Receiver<mdns_sd::DaemonEvent>,
        Registry,
    ),
    String,
> {
    let daemon = mdns_sd::ServiceDaemon::new().map_err(|e| e.to_string())?;
    let _ = daemon.disable_interface(mdns_sd::IfKind::IPv6);
    let monitor = daemon.monitor().map_err(|e| e.to_string())?;
    let reg = Registry::new(Some(Arc::new(daemon.clone())), host, false);
    reg.bind_fabric(hub);
    reg.set_lan_record(true);
    reg.sync_record().await;
    assert!(reg.record().await.is_some(), "the record registers");
    let seen = renames(&monitor, Duration::from_millis(1_500)).await;
    assert!(
        seen.is_empty(),
        "the first registration probes clean: {seen:?}"
    );
    Ok((daemon, monitor, reg))
}

/// G8 phase 12's busy loop (row D8 follow-up): an update of the registered record renames
/// nothing on a real daemon, and neither does a withdrawn record that comes back (review
/// finding 5). In a network namespace of its own (`lo` and a veth with an address), each step on
/// a fresh mdns-sd daemon and registry over one real hub, so each step's renames are its own:
/// - a re-minted role bit (D10's path): the TXT changes in place;
/// - a renamed host (the dashboard's monitor follows a conflict): SRV and A records in place;
/// - the roles re-minted, then the switch off and on again at once (the goodbye carries the old
///   TXT, the record the new one): no registration while the goodbye is in flight
///   (`GOODBYE_GUARD`), the next sync after it registers.
///
/// An unregister before each update sent a goodbye that the daemon read back (multicast loop,
/// `lo`) as a conflict with the update's probe: the instance became `<label> (2)` within a
/// second, and each host rename such a conflict caused came back as one more update; the same
/// goodbye's resend (120 ms later) does it to a registration that follows a withdrawal at once.
/// Every step's renames are collected, so a failure names each step that renamed. The positive
/// control: a second daemon in the namespace resolves the instance under its own name with the
/// last roles. Skipped (printed) only where the box offers no network namespace, no `ip`
/// (iproute2) or no multicast path inside it.
#[cfg(feature = "mdns")]
#[test]
fn an_update_of_the_record_renames_nothing_on_a_real_daemon() {
    if let Err(skip) = in_netns("d8u", "d8v", "10.78.0.1/24", update_renames_nothing) {
        eprintln!("SKIPPED: {skip}");
    }
}

#[cfg(feature = "mdns")]
async fn update_renames_nothing() -> Result<(), String> {
    use mdns_sd::{IfKind, ServiceDaemon};
    let hd = tempfile::tempdir().unwrap();
    let ha = api_in(hd.path(), true);
    let hub = start_fabric(hd.path(), &ha, true).await;
    let mut failed: Vec<String> = Vec::new();

    // 1. a re-minted role bit (D10's path): the TXT changes in place
    let (d1, m1, r1) = fresh_record(&hub, "111.local").await?;
    let roles0 = r1.record().await.unwrap().roles;
    assert!(hub
        .set_relay_roles(fabric::RelayRoles {
            offload_bit: true,
            ..fabric::RelayRoles::default()
        })
        .await
        .unwrap());
    r1.sync_record().await;
    assert_ne!(
        r1.record().await.unwrap().roles,
        roles0,
        "the record carries the re-minted roles"
    );
    let seen = renames(&m1, Duration::from_millis(2_500)).await;
    if !seen.is_empty() {
        failed.push(format!("1. a role change renamed: {seen:?}"));
    }
    stop_daemon(&d1);
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 2. a renamed host (the dashboard's monitor follows a conflict), on a fresh daemon
    let (d2, m2, r2) = fresh_record(&hub, "111.local").await?;
    r2.set_host("111-2.local.");
    r2.sync_record().await;
    assert_eq!(r2.record().await.unwrap().host, "111-2.local");
    let seen = renames(&m2, Duration::from_millis(2_500)).await;
    if !seen.is_empty() {
        failed.push(format!("2. a host rename renamed again: {seen:?}"));
    }

    // 3. the roles re-minted (no sync yet), the switch off, then on again at once: the goodbye
    // carries the old TXT, a registration in its 120 ms would carry the new one
    assert!(hub
        .set_relay_roles(fabric::RelayRoles::default())
        .await
        .unwrap());
    r2.set_lan_record(false);
    r2.sync_record().await;
    assert!(r2.record().await.is_none(), "withdrawn");
    let off = Instant::now();
    r2.set_lan_record(true);
    r2.sync_record().await;
    if r2.record().await.is_some() {
        failed.push(format!(
            "3. registered again {:?} after its goodbye (GOODBYE_GUARD {:?})",
            off.elapsed(),
            registry::GOODBYE_GUARD
        ));
    }
    let seen = renames(&m2, Duration::from_millis(2_500)).await;
    if !seen.is_empty() {
        failed.push(format!(
            "3. a record registered again at once renamed: {seen:?}"
        ));
    }
    // the guard has passed: the next sync registers it, cleanly
    tokio::time::sleep(registry::GOODBYE_GUARD.saturating_sub(off.elapsed())).await;
    r2.sync_record().await;
    let last = r2.record().await.expect("registered once the guard passed");
    let full = format!("{}.{SERVICE_111}", last.instance);
    let seen = renames(&m2, Duration::from_millis(1_500)).await;
    if !seen.is_empty() {
        failed.push(format!(
            "3. the registration after the guard renamed: {seen:?}"
        ));
    }

    // a second daemon in the namespace resolves the instance under its own name, last roles
    let b = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let _ = b.disable_interface(IfKind::IPv6);
    let hits = tokio::task::spawn_blocking({
        let b = b.clone();
        move || MdnsOps::browse(&b, SERVICE_111, Duration::from_secs(3))
    })
    .await
    .unwrap();
    for d in [&d2, &b] {
        stop_daemon(d);
    }
    hub.shutdown().await;
    let hits = hits?;
    if hits.is_empty() {
        return Err("the second daemon heard nothing in the namespace (no multicast path)".into());
    }
    assert!(failed.is_empty(), "{failed:#?}");
    assert!(
        hits.iter().all(|h| h.fullname == full),
        "one instance, its own name: {hits:?}"
    );
    assert!(
        hits[0]
            .txt
            .contains(&("r".to_string(), format!("{:04x}", last.roles))),
        "the last roles reached the LAN: {hits:?}"
    );
    Ok(())
}

/// The datagrams a [`Wire`] heard, with their arrival times.
#[cfg(feature = "mdns")]
type Heard = Vec<(Instant, Vec<u8>)>;

/// Every mDNS datagram the namespace's sockets see, with its arrival time (a listener on
/// 224.0.0.251:5353, the group joined on one address; `SO_REUSEPORT` like mdns-sd's own
/// socket), and the sender of a forged answer (multicast on that address's interface, looped
/// back to every local listener).
#[cfg(feature = "mdns")]
struct Wire {
    sock: std::net::UdpSocket,
    seen: Arc<Mutex<Heard>>,
    stop: Arc<AtomicBool>,
    reader: Option<std::thread::JoinHandle<()>>,
}

#[cfg(feature = "mdns")]
impl Wire {
    fn listen(on: Ipv4Addr) -> Result<Wire, String> {
        use std::os::fd::FromRawFd;
        let os = |what: &str| {
            format!(
                "the wire listener: {what}: {}",
                std::io::Error::last_os_error()
            )
        };
        // SAFETY: socket(2) returns a new descriptor (or -1), owned by `sock` from here on.
        let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
        if fd < 0 {
            return Err(os("socket"));
        }
        // SAFETY: `fd` is a fresh, open UDP socket nothing else owns.
        let sock = unsafe { std::net::UdpSocket::from_raw_fd(fd) };
        let int = |level: libc::c_int, name: libc::c_int, v: libc::c_int| {
            // SAFETY: a c_int option value of the right size on an open socket.
            unsafe {
                libc::setsockopt(
                    fd,
                    level,
                    name,
                    (&v as *const libc::c_int).cast(),
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                )
            }
        };
        if int(libc::SOL_SOCKET, libc::SO_REUSEADDR, 1) != 0
            || int(libc::SOL_SOCKET, libc::SO_REUSEPORT, 1) != 0
        {
            return Err(os("SO_REUSEADDR/SO_REUSEPORT"));
        }
        let any = libc::sockaddr_in {
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: 5353u16.to_be(),
            sin_addr: libc::in_addr { s_addr: 0 },
            sin_zero: [0; 8],
        };
        // SAFETY: a sockaddr_in of the right size on an open socket.
        let bound = unsafe {
            libc::bind(
                fd,
                (&any as *const libc::sockaddr_in).cast(),
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            )
        };
        if bound != 0 {
            return Err(os("bind 0.0.0.0:5353"));
        }
        let ifaddr = libc::in_addr {
            s_addr: u32::from(on).to_be(),
        };
        // SAFETY: an in_addr of the right size on an open socket.
        let set_if = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_IP,
                libc::IP_MULTICAST_IF,
                (&ifaddr as *const libc::in_addr).cast(),
                std::mem::size_of::<libc::in_addr>() as libc::socklen_t,
            )
        };
        if set_if != 0 {
            return Err(os("IP_MULTICAST_IF"));
        }
        sock.join_multicast_v4(&Ipv4Addr::new(224, 0, 0, 251), &on)
            .map_err(|e| format!("the wire listener: join 224.0.0.251 on {on}: {e}"))?;
        sock.set_multicast_loop_v4(true)
            .map_err(|e| e.to_string())?;
        let reader_sock = sock.try_clone().map_err(|e| e.to_string())?;
        reader_sock
            .set_read_timeout(Some(Duration::from_millis(50)))
            .map_err(|e| e.to_string())?;
        let seen: Arc<Mutex<Heard>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let reader = std::thread::spawn({
            let (seen, stop) = (Arc::clone(&seen), Arc::clone(&stop));
            move || {
                let mut buf = vec![0u8; 9_000];
                while !stop.load(Ordering::SeqCst) {
                    if let Ok((n, _)) = reader_sock.recv_from(&mut buf) {
                        seen.lock()
                            .unwrap()
                            .push((Instant::now(), buf[..n].to_vec()));
                    }
                }
            }
        });
        Ok(Wire {
            sock,
            seen,
            stop,
            reader: Some(reader),
        })
    }

    /// Multicast `packet` to 224.0.0.251:5353 on the listener's interface.
    fn send(&self, packet: &[u8]) {
        self.sock
            .send_to(packet, (Ipv4Addr::new(224, 0, 0, 251), 5353))
            .expect("the forged answer is sent");
    }

    /// The datagrams that arrived in `[from, to)` and hold `pat` (an empty `pat`: all of them).
    fn count(&self, from: Instant, to: Instant, pat: &[u8]) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, p)| {
                *t >= from && *t < to && (pat.is_empty() || p.windows(pat.len()).any(|w| w == pat))
            })
            .count()
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
    }
}

/// A forged mDNS answer: `name` (no trailing dot) has the A record `ip` (a cache-flush answer,
/// TTL 120), as another host on the segment would send it.
#[cfg(feature = "mdns")]
fn forged_a(name: &str, ip: Ipv4Addr) -> Vec<u8> {
    // id 0, QR + AA, one answer
    let mut p = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0];
    for label in name.split('.') {
        p.push(u8::try_from(label.len()).unwrap());
        p.extend_from_slice(label.as_bytes());
    }
    p.push(0);
    p.extend_from_slice(&[0, 1, 0x80, 1]); // type A, class IN with the cache-flush bit
    p.extend_from_slice(&120u32.to_be_bytes());
    p.extend_from_slice(&[0, 4]);
    p.extend_from_slice(&ip.octets());
    p
}

/// A plain mDNS query (QU bit clear): `name`, type PTR.
#[cfg(feature = "mdns")]
fn ptr_query(name: &str) -> Vec<u8> {
    let mut p = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.split('.') {
        p.push(u8::try_from(label.len()).unwrap());
        p.extend_from_slice(label.as_bytes());
    }
    p.push(0);
    p.extend_from_slice(&[0, 12, 0, 1]);
    p
}

/// Review findings 1, 4, 6 and 8 (row D8 follow-up): a rename of the dashboard's mDNS host
/// stays bounded and the name the URL list shows stays the one the node holds, on the
/// dashboard's real announcer (its `_http._tcp` service `111` on `111.local`, its monitor
/// thread) with the registry's real `_n111._tcp` record over a real hub and the callback
/// `dashboard::start` installs (the record follows the renamed host: `set_host`, then a sync).
/// In a network namespace of its own (`lo` and a veth with 10.78.1.1/24), a wire listener
/// counting every mDNS packet:
/// 1. a peer on the segment holds `111.local` (another node's address, 10.78.1.2), the record's
///    switch off: the node is renamed `111-2.local` by a real conflict;
/// 2. right after that rename, inside the probe of `111-2.local` the announcer's re-registration
///    opens, a forged answer gives the name to another host (10.78.1.3; a peer holding the same
///    name after a lost packet answers the same way): at most one more rename, no instance
///    renamed (the service is not announced while it probes);
/// 3. the switch on afterwards (D10's PATCH; G8 phase 8 does this after the start-up renames):
///    the record registers under the name the dashboard holds and must open no probe of it
///    beside the announced service, so a forged answer for it in the next 600 ms renames
///    nothing. Under its first name the announced service had the record's A records probe the
///    host alone; the forged answer renamed the host, the rename re-probed the service's SRV,
///    the daemon answered its own probe with `111.local` and renamed the instance (`111 (2)`,
///    `111 (3)` …) every ~100 ms with no end and no `NameChange`: G8 phase 12's 230-290
///    wakeups per 10 s, until a re-number dropped the names;
/// 4. the peer leaves, the record is withdrawn (only the dashboard's own service answers), and
///    the node is re-numbered (10.78.1.1 → .9) and queried at once from the new address: the
///    daemon's answer from the vanished address fails and it drops the interface's names (G8
///    phase 12's path), its 5 s check brings the interface back: the dashboard's SRV must name
///    what the URL list shows, that name must resolve to the new address, and the segment is
///    quiet afterwards.
///
/// Every step's failure is collected, so a run names each one. Skipped (printed) only where the
/// box offers no network namespace, no `ip` or no multicast path inside it.
#[cfg(feature = "mdns")]
#[test]
fn a_renamed_host_stays_bounded_and_truthful_on_the_real_announcer() {
    if let Err(skip) = in_netns("d8r", "d8s", "10.78.1.1/24", renamed_host_stays_bounded) {
        eprintln!("SKIPPED: {skip}");
    }
}

#[cfg(feature = "mdns")]
async fn renamed_host_stays_bounded() -> Result<(), String> {
    use mdns_sd::{HostnameResolutionEvent, IfKind, ServiceDaemon, ServiceEvent, ServiceInfo};
    use node111::daemon::dashboard::mdns::Announcer;
    let first = Ipv4Addr::new(10, 78, 1, 1);
    let wire = Wire::listen(first)?;
    let mut failed: Vec<String> = Vec::new();
    let mut figures: Vec<String> = Vec::new();

    // a peer on the segment holds 111.local (another node's address)
    let peer = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let _ = peer.disable_interface(IfKind::IPv6);
    let none: [(&str, &str); 0] = [];
    let held = ServiceInfo::new(
        "_http._tcp.local.",
        "peer",
        "111.local.",
        std::net::IpAddr::V4(Ipv4Addr::new(10, 78, 1, 2)),
        8080,
        &none[..],
    )
    .map_err(|e| e.to_string())?;
    peer.register(held).map_err(|e| e.to_string())?;
    tokio::time::sleep(Duration::from_millis(1_500)).await;

    // the node: the dashboard's announcer, the registry (its switch off), dashboard::start's
    // callback
    let hd = tempfile::tempdir().unwrap();
    let ha = api_in(hd.path(), true);
    let hub = start_fabric(hd.path(), &ha, true).await;
    let slot: Arc<std::sync::OnceLock<Arc<Registry>>> = Arc::default();
    let hosts: Arc<Mutex<Vec<String>>> = Arc::default();
    let rt = tokio::runtime::Handle::current();
    let announcer = Announcer::start(
        8080,
        Box::new({
            let (slot, hosts) = (Arc::clone(&slot), Arc::clone(&hosts));
            move |host: String| {
                hosts.lock().unwrap().push(host.clone());
                if let Some(reg) = slot.get() {
                    reg.set_host(&host);
                    let reg = Arc::clone(reg);
                    rt.spawn(async move { reg.sync_record().await });
                }
            }
        }),
    )
    .ok_or("the announcer did not start (no mDNS daemon here)")?;
    let shown = |hosts: &Mutex<Vec<String>>| {
        hosts
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_else(|| "111.local".to_string())
    };
    let renamed = |hosts: &Mutex<Vec<String>>| hosts.lock().unwrap().clone();
    let reg = Arc::new(Registry::new(announcer.ops(), &shown(&hosts), false));
    reg.bind_fabric(&hub);
    let _ = slot.set(Arc::clone(&reg));

    // forged answers for `name` at +150, +350 and +550 ms, then 4 s on the wire from the first:
    // (packets, packets with an instance rename)
    let forge = |name: String| {
        let wire = &wire;
        async move {
            let from = Instant::now();
            for wait in [150, 200, 200] {
                tokio::time::sleep(Duration::from_millis(wait)).await;
                wire.send(&forged_a(&name, Ipv4Addr::new(10, 78, 1, 3)));
            }
            tokio::time::sleep(Duration::from_millis(3_450)).await;
            let to = from + Duration::from_secs(4);
            (wire.count(from, to, b""), wire.count(from, to, b"111 ("))
        }
    };

    // 1. the real conflict: the peer defends 111.local
    let t1 = Instant::now();
    while renamed(&hosts).is_empty() {
        if t1.elapsed() > Duration::from_secs(8) {
            if wire.count(t1, Instant::now(), b"") == 0 {
                return Err("the wire heard nothing in the namespace (no multicast path)".into());
            }
            panic!("no rename within 8 s of the announcer's start");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let after1 = renamed(&hosts);
    if after1 != ["111-2.local"] {
        failed.push(format!("1. the first conflict renamed {after1:?}"));
    }

    // 2. inside the announcer's own probe of the new name, a forged answer for it
    let (packets, instance) = forge("111-2.local".to_string()).await;
    let after2 = renamed(&hosts);
    figures.push(format!(
        "2. {packets} packets in 4 s, {instance} with an instance rename, hosts {after2:?}"
    ));
    if instance > 0 || after2.len() > 2 {
        failed.push(format!(
            "2. a forged answer inside the announcer's probe: {instance} packet(s) renamed the \
             instance, the hosts {after2:?} (at most one more rename)"
        ));
    }

    // 3. the switch on afterwards: the record joins the name the dashboard holds
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let before3 = renamed(&hosts);
    reg.set_lan_record(true);
    reg.sync_record().await;
    let rec = reg.record().await;
    if rec.as_ref().map(|r| r.host.as_str()) != Some(shown(&hosts).as_str()) {
        failed.push(format!(
            "3. the record {rec:?} does not carry the dashboard's host {}",
            shown(&hosts)
        ));
    }
    let (packets, instance) = forge(shown(&hosts)).await;
    let after3 = renamed(&hosts);
    figures.push(format!(
        "3. {packets} packets in 4 s, {instance} with an instance rename, hosts {after3:?}"
    ));
    if instance > 0 || after3 != before3 {
        failed.push(format!(
            "3. the record's registration opened a probe of the host beside the announced \
             service: {instance} packet(s) renamed the instance within 4 s, the hosts {before3:?} \
             became {after3:?}"
        ));
    }

    // 4. the peer leaves, the record goes; a re-number, and a query from the new address
    stop_daemon(&peer);
    reg.set_lan_record(false);
    reg.sync_record().await;
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let moved = Ipv4Addr::new(10, 78, 1, 9);
    netns_ip("addr del 10.78.1.1/24 dev d8r").unwrap_or_else(|e| panic!("{e}"));
    netns_ip("addr add 10.78.1.9/24 dev d8r").unwrap_or_else(|e| panic!("{e}"));
    let asker = std::net::UdpSocket::bind((moved, 0)).map_err(|e| e.to_string())?;
    asker
        .send_to(
            &ptr_query("_http._tcp.local"),
            (Ipv4Addr::new(224, 0, 0, 251), 5353),
        )
        .map_err(|e| e.to_string())?;
    tokio::time::sleep(Duration::from_secs(7)).await; // the 5 s check, then the probe
    let quiet_from = Instant::now();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let quiet = wire.count(quiet_from, Instant::now(), b"");
    if quiet > 2 {
        failed.push(format!(
            "4. the segment is not quiet: {quiet} packets in 2 s after the re-number"
        ));
    }
    let url_host = shown(&hosts);
    let c = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let _ = c.disable_interface(IfKind::IPv6);
    let srv_host = tokio::task::spawn_blocking({
        let c = c.clone();
        move || {
            let rx = c.browse("_http._tcp.local.").ok()?;
            let end = std::time::Instant::now() + Duration::from_secs(3);
            let mut host = None;
            while let Ok(ev) =
                rx.recv_timeout(end.saturating_duration_since(std::time::Instant::now()))
            {
                if let ServiceEvent::ServiceResolved(r) = ev {
                    if r.get_fullname().starts_with("111") {
                        host = Some(r.get_hostname().to_string());
                        break;
                    }
                }
            }
            let _ = c.stop_browse("_http._tcp.local.");
            host
        }
    })
    .await
    .unwrap();
    let addrs = tokio::task::spawn_blocking({
        let (c, name) = (c.clone(), format!("{url_host}."));
        move || {
            // every address heard (the node answers on `lo` too) until the segment's or the end
            let mut got = Vec::new();
            if let Ok(rx) = c.resolve_hostname(&name, Some(3_000)) {
                while let Ok(ev) = rx.recv_timeout(Duration::from_secs(4)) {
                    match ev {
                        HostnameResolutionEvent::AddressesFound(_, a) => {
                            got.extend(a.iter().map(|s| s.to_ip_addr()));
                            if got.contains(&std::net::IpAddr::V4(moved)) {
                                break;
                            }
                        }
                        HostnameResolutionEvent::SearchTimeout(_)
                        | HostnameResolutionEvent::SearchStopped(_) => break,
                        _ => {}
                    }
                }
            }
            let _ = c.stop_resolve_hostname(&name);
            got
        }
    })
    .await
    .unwrap();
    stop_daemon(&c);
    announcer.stop();
    hub.shutdown().await;
    wire.stop();
    figures.push(format!(
        "4. {quiet} packets in 2 s after the re-number; the SRV {srv_host:?}, the URL list \
         {url_host}, resolved {addrs:?}"
    ));
    if srv_host.as_deref() != Some(format!("{url_host}.").as_str()) {
        failed.push(format!(
            "4. after the re-number the dashboard's SRV names {srv_host:?}, the URL list {url_host}"
        ));
    }
    if !addrs.contains(&std::net::IpAddr::V4(moved)) {
        failed.push(format!(
            "4. the URL list's name {url_host} resolves to {addrs:?}, not {moved}"
        ));
    }
    eprintln!("figures: {figures:#?}");
    assert!(failed.is_empty(), "{failed:#?}");
    Ok(())
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
    hub: Fab,
    m: Fab,
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
