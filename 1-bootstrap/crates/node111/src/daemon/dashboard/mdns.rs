//! mDNS announce of `111.local` while the dashboard is LAN-bound (D15, spec §1).
//!
//! Service `_http._tcp`, instance `111`, host `111.local.`, IPv4 addresses
//! tracked automatically by `mdns-sd` as interfaces come and go. Only A records
//! are announced: the LAN listener is `0.0.0.0` (IPv4-only), so an AAAA record
//! would send IPv6-preferring clients to a port nothing answers on. On a name
//! conflict `mdns-sd` renames the host to `111-N.local.` and reports it — again
//! on every further conflict (`111-2` → `111-3` …); each rename is logged and
//! handed to `on_rename` so the URL list stays truthful (the Host guard already
//! accepts `111-N.local`). Stopped on daemon shutdown.
//!
//! **Layer 7 (§3.3, §7.1; row D8).** The same daemon carries the registry's
//! `_n111._tcp.local.` record ([`Record111`], registered by `daemon::registry`
//! through [`MdnsOps`]; this module never decides whether it is present) and
//! answers the on-demand browse ([`MdnsOps::browse`]). It also re-arms its own
//! IP check from the address watch instead of polling ([`rearm`], the pinned
//! S2 procedure): mdns-sd checks the interfaces every 5 s
//! ([`DEFAULT_IP_CHECK_SECS`]) unless told otherwise, so while the watch's
//! second netlink socket is bound the interval is 0 and each debounced address
//! event runs one check (`set_ip_check_interval(1)`, one one-shot 1,500 ms
//! sleep, `set_ip_check_interval(0)`, all in the one task that holds the
//! subscription, so two events serialise and no set-0 comes early). A failed
//! bind keeps the 5 s check (logged once), so the A records never freeze at
//! the start-time addresses (S3); so does a set the daemon refuses while it
//! still runs (one retry, then back to 5 s; never left at 1 s). No listener,
//! timer or socket is added: the
//! record and the browse ride the daemon's own socket, the task sleeps only
//! after an event.
//!
//! Behind the default-on cargo feature `mdns`; without it [`Announcer::start`]
//! logs once and returns `None`.

use crate::daemon::relay::netwatch::{self, Consumer, NetEvent, NetWatch, Watching};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Hostname announced (FQDN form for mdns-sd; printed without the trailing dot).
pub const HOSTNAME: &str = "111.local.";

/// The registry's DNS-SD service type (§7.1). `_n111`, not `_111`: RFC 6335 §5.1 wants a
/// letter in a service name and mdns-sd refuses one without (the owner's ruling on row D8).
pub const SERVICE_111: &str = "_n111._tcp.local.";

/// mdns-sd's own IP check period (`IP_CHECK_INTERVAL_IN_SECS_DEFAULT`, `MD/service_daemon.rs:69`):
/// kept while the address watch has no bound socket (§3.3).
pub const DEFAULT_IP_CHECK_SECS: u32 = 5;

/// The re-arm's one-shot check period (§3.3 S2): the next check runs 1 s after it is set.
pub const REARM_CHECK_SECS: u32 = 1;

/// The re-arm's one-shot sleep before the check is switched off again (§3.3 S2): long enough
/// for the 1 s check to run once, short enough that a second one never does.
pub const REARM_SLEEP: Duration = Duration::from_millis(1_500);

/// A refused restore of the 5 s check is tried again after this, doubling up to
/// [`RESTORE_RETRY_MAX`] (the daemon's command channel was full: it drains on its own).
pub const RESTORE_RETRY: Duration = Duration::from_millis(250);

/// The longest wait between two tries of a refused restore.
pub const RESTORE_RETRY_MAX: Duration = Duration::from_secs(60);

/// The host name to switch to for one `NameChange` event, if it concerns our
/// host: `original` is the name we registered (mdns-sd reports the registered
/// name on every rename) or the name we currently hold; an event that repeats
/// the current name (one per record type / interface) is no change. Names are
/// returned without the trailing dot.
pub fn rename_target(current: &str, original: &str, new_name: &str) -> Option<String> {
    let norm = |s: &str| s.trim_end_matches('.').to_ascii_lowercase();
    let (current, original, new_name) = (norm(current), norm(original), norm(new_name));
    let ours = original == norm(HOSTNAME) || original == current;
    (ours && !new_name.is_empty() && new_name != current).then_some(new_name)
}

// ---------------------------------------------------------------- the `_n111._tcp` record (§7.1)

/// The registry's DNS-SD record (§7.1): instance = the L2 label, host = the dashboard's mDNS
/// host, SRV = the fabric listener's port (11183), A records only, TXT `v=1`, `r=<roles, 4
/// hex>`, `k=<NodeId, 64 hex>` (78 B with the length bytes). Nothing else is advertised (§7.4):
/// no alias, load, battery, position or model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record111 {
    /// The L2 label (`xxxx-xxxx-xx`).
    pub instance: String,
    /// The dashboard's mDNS host without the trailing dot (`111.local` or its rename).
    pub host: String,
    /// The fabric listener's port.
    pub port: u16,
    /// The signed role bits of this node's announce.
    pub roles: u16,
    /// This node's NodeId.
    pub node_id: [u8; 32],
}

impl Record111 {
    /// The TXT keys and values, in record order (`v`, `r`, `k`).
    pub fn txt(&self) -> [(&'static str, String); 3] {
        let k: String = self.node_id.iter().map(|b| format!("{b:02x}")).collect();
        [
            ("v", "1".to_string()),
            ("r", format!("{:04x}", self.roles)),
            ("k", k),
        ]
    }

    /// The TXT RDATA size: one length byte per `key=value` string (RFC 6763 §6).
    pub fn txt_wire_len(&self) -> usize {
        self.txt()
            .iter()
            .map(|(k, v)| 1 + k.len() + 1 + v.len())
            .sum()
    }

    /// The host in mdns-sd's FQDN form (trailing dot).
    pub fn host_fqdn(&self) -> String {
        format!("{}.", self.host.trim_end_matches('.'))
    }
}

/// One resolved `_n111._tcp` instance as a browse saw it. Every field is untrusted (§7.1: a
/// forged record is possible); `daemon::registry` checks and bounds it before it becomes a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseHit {
    /// The full instance name (`<instance>._n111._tcp.local.`).
    pub fullname: String,
    /// The SRV port.
    pub port: u16,
    /// The A records.
    pub v4: Vec<Ipv4Addr>,
    /// The TXT strings as key/value pairs.
    pub txt: Vec<(String, String)>,
}

/// At most this many instances are kept from one browse (a flood of forged records is cut).
/// Past the cap a uniform sample is kept ([`Reservoir`]), so the records that answer first
/// cannot crowd out a later one.
pub const BROWSE_MAX_HITS: usize = 64;

/// The browse's bounded hit list: one entry per instance (a re-resolve replaces it); once
/// [`BROWSE_MAX_HITS`] distinct instances are held, the n-th new one replaces a random held
/// entry with probability `cap / n` (reservoir sampling): every instance heard is kept with the
/// same chance, whatever its arrival order.
#[derive(Debug)]
pub struct Reservoir {
    cap: usize,
    seen: usize,
    hits: Vec<BrowseHit>,
}

impl Reservoir {
    pub fn new(cap: usize) -> Reservoir {
        Reservoir {
            cap,
            seen: 0,
            hits: Vec::new(),
        }
    }

    /// One resolved instance.
    pub fn offer(&mut self, hit: BrowseHit) {
        if let Some(h) = self.hits.iter_mut().find(|h| h.fullname == hit.fullname) {
            *h = hit;
            return;
        }
        self.seen += 1;
        if self.hits.len() < self.cap {
            self.hits.push(hit);
        } else if self.cap > 0 {
            let j = random_below(self.seen);
            if j < self.cap {
                self.hits[j] = hit;
            }
        }
    }

    /// An instance said goodbye.
    pub fn remove(&mut self, fullname: &str) {
        self.hits.retain(|h| h.fullname != fullname);
    }

    pub fn into_hits(self) -> Vec<BrowseHit> {
        self.hits
    }
}

/// A random number below `n` (`n > 0`) from std's per-process random hasher keys (no RNG
/// dependency; a fresh `RandomState` takes new keys).
pub fn random_below(n: usize) -> usize {
    use std::hash::BuildHasher;
    let r = std::collections::hash_map::RandomState::new().hash_one(n);
    usize::try_from(r % u64::try_from(n.max(1)).unwrap_or(u64::MAX)).unwrap_or(0)
}

/// What the registry asks of the running mDNS daemon (§7.1). Production: the announcer's
/// `mdns_sd::ServiceDaemon`; tests pass a fake. The registry never starts a daemon: it only
/// holds one of these while the dashboard's announcer runs.
pub trait MdnsOps: Send + Sync {
    /// Register `rec` (A records follow the interfaces, IPv4 only); returns its full name.
    fn register(&self, rec: &Record111) -> Result<String, String>;
    /// Withdraw a record registered by [`MdnsOps::register`] (a goodbye packet), waiting at
    /// most `wait` for the daemon to confirm it was sent (`ZERO`: no wait).
    fn unregister(&self, fullname: &str, wait: Duration);
    /// Browse `service` for at most `limit`, then `stop_browse`; blocking (the caller runs it
    /// on the blocking pool). Returns the instances resolved and not removed meanwhile.
    fn browse(&self, service: &str, limit: Duration) -> Result<Vec<BrowseHit>, String>;
}

/// Why a `set_ip_check_interval` did not take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpCheckError {
    /// The daemon is gone (its command channel is closed): the re-arm ends.
    Gone,
    /// The daemon runs but the command was not taken or not signalled (a full command
    /// channel, a failed wake-up send): worth one retry.
    Refused(String),
}

/// The daemon's IP-check switch, as [`rearm`] uses it (production: the announcer's daemon;
/// tests pass a fake that records each call).
pub trait IpCheck: Send + Sync {
    /// `set_ip_check_interval(secs)` (non-blocking: a command to the daemon thread).
    fn set_ip_check_interval(&self, secs: u32) -> Result<(), IpCheckError>;
}

static KEEP_DEFAULT_LOGGED: AtomicBool = AtomicBool::new(false);

/// The §3.3 re-arm, one task per running daemon, holding the [`Consumer::Mdns`] subscription:
///
/// - the watch's socket bound ([`Watching::Netlink`]): the interval goes to 0 (only then);
/// - each debounced event: `set_ip_check_interval(1)`, one one-shot [`REARM_SLEEP`],
///   `set_ip_check_interval(0)`, then `on_change` (the registry's lost-hub check, §7.7);
/// - a failed bind ([`Watching::Failed`]) or a socket that stops answering
///   ([`NetEvent::Failed`]): the 5 s check stays or comes back, logged once, and the task ends
///   (dropping the subscription; the next daemon start tries again);
/// - a set the running daemon refuses ([`IpCheckError::Refused`]) is tried once more; a set of
///   0 or 1 that still fails puts the 5 s check back and the task stays on it: events still
///   reach `on_change`, the interval is no longer touched. A refused restore is tried again
///   (backing off, [`restore_default`]) until the daemon takes it, and only then logged (once)
///   as kept: the interval is never left at 1 s, and never at 0 without the watch.
///
/// Ends only when the daemon is gone ([`IpCheckError::Gone`]), on a watch failure, or when the
/// task is aborted ([`Announcer::stop`]).
pub async fn rearm(
    ip: Arc<dyn IpCheck>,
    sub: netwatch::Subscription,
    on_change: Arc<dyn Fn() + Send + Sync>,
) {
    rearm_logged(ip, sub, on_change, &KEEP_DEFAULT_LOGGED).await
}

/// One set, retried once when the running daemon refused it: `Ok(true)` taken, `Ok(false)`
/// refused twice, `Err(())` the daemon is gone.
fn set_retry(ip: &dyn IpCheck, secs: u32) -> Result<bool, ()> {
    for _ in 0..2 {
        match ip.set_ip_check_interval(secs) {
            Ok(()) => return Ok(true),
            Err(IpCheckError::Gone) => return Err(()),
            Err(IpCheckError::Refused(_)) => {}
        }
    }
    Ok(false)
}

/// Put the daemon's own 5 s check back: tried (each try = [`set_retry`]) until the daemon
/// takes it, waiting [`RESTORE_RETRY`] and then twice as long each time (at most
/// [`RESTORE_RETRY_MAX`]), so the interval is never left at 1 s, nor at 0 without the re-arm.
/// `keep_default` is called (and so the log written) only once a set of 5 was taken; `Err` =
/// the daemon is gone.
async fn restore_default(
    ip: &dyn IpCheck,
    why: &str,
    keep_default: &(dyn Fn(&str) + Sync),
) -> Result<(), ()> {
    let mut wait = RESTORE_RETRY;
    let mut told = false;
    loop {
        if set_retry(ip, DEFAULT_IP_CHECK_SECS)? {
            keep_default(why);
            return Ok(());
        }
        if !told {
            told = true;
            eprintln!(
                "111 daemon: mDNS refused its {DEFAULT_IP_CHECK_SECS} s address check back ({why}); trying again"
            );
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(RESTORE_RETRY_MAX);
    }
}

/// [`rearm`] with its log-once flag (tests pass their own).
pub async fn rearm_logged(
    ip: Arc<dyn IpCheck>,
    sub: netwatch::Subscription,
    on_change: Arc<dyn Fn() + Send + Sync>,
    logged: &AtomicBool,
) {
    let keep_default = |why: &str| {
        if !logged.swap(true, Ordering::SeqCst) {
            eprintln!("111 daemon: mDNS keeps its {DEFAULT_IP_CHECK_SECS} s address check ({why})");
        }
    };
    // `true` while the interval rests at 0 and each event re-arms it
    let mut quiet = false;
    match sub.watching() {
        Watching::Netlink => match set_retry(&*ip, 0) {
            Ok(true) => quiet = true,
            Ok(false) => {
                let why = "the daemon refused to switch its check off";
                if restore_default(&*ip, why, &keep_default).await.is_err() {
                    return;
                }
            }
            Err(()) => return,
        },
        Watching::Failed { error, .. } => {
            keep_default(&format!("the address watch could not bind: {error}"));
            return;
        }
    }
    loop {
        match sub.next().await {
            NetEvent::Changed | NetEvent::Poll => {
                if quiet {
                    let refused = match set_retry(&*ip, REARM_CHECK_SECS) {
                        Ok(true) => {
                            tokio::time::sleep(REARM_SLEEP).await;
                            match set_retry(&*ip, 0) {
                                Ok(true) => None,
                                Ok(false) => Some("the daemon refused the check's reset to 0"),
                                Err(()) => return,
                            }
                        }
                        Ok(false) => Some("the daemon refused the one-shot check"),
                        Err(()) => return,
                    };
                    if let Some(why) = refused {
                        quiet = false;
                        if restore_default(&*ip, why, &keep_default).await.is_err() {
                            return;
                        }
                    }
                }
                on_change();
            }
            NetEvent::Failed => {
                let why = "the address watch stopped answering";
                if quiet {
                    let _ = restore_default(&*ip, why, &keep_default).await;
                } else {
                    keep_default(why);
                }
                return;
            }
        }
    }
}

/// Subscribe `watch` for the daemon behind `ip` and spawn [`rearm`]; `None` outside a runtime
/// (then the 5 s check stays).
pub fn spawn_rearm(
    watch: &NetWatch,
    ip: Arc<dyn IpCheck>,
    on_change: Arc<dyn Fn() + Send + Sync>,
) -> Option<tokio::task::JoinHandle<()>> {
    let sub = watch.subscribe(Consumer::Mdns).ok()?;
    Some(tokio::spawn(rearm(ip, sub, on_change)))
}

// ---------------------------------------------------------------- mdns-sd

#[cfg(feature = "mdns")]
impl IpCheck for mdns_sd::ServiceDaemon {
    fn set_ip_check_interval(&self, secs: u32) -> Result<(), IpCheckError> {
        mdns_sd::ServiceDaemon::set_ip_check_interval(self, secs).map_err(|e| match e {
            mdns_sd::Error::DaemonShutdown => IpCheckError::Gone,
            e => IpCheckError::Refused(e.to_string()),
        })
    }
}

/// Register `rec` as [`SERVICE_111`] on `daemon` (A records only, following the interfaces).
#[cfg(feature = "mdns")]
pub fn register_record(daemon: &mdns_sd::ServiceDaemon, rec: &Record111) -> Result<String, String> {
    let txt = rec.txt();
    let props: Vec<(&str, &str)> = txt.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let mut info = mdns_sd::ServiceInfo::new(
        SERVICE_111,
        &rec.instance,
        &rec.host_fqdn(),
        (),
        rec.port,
        &props[..],
    )
    .map_err(|e| e.to_string())?
    .enable_addr_auto();
    info.set_interfaces(vec![mdns_sd::IfKind::IPv4]);
    let fullname = info.get_fullname().to_string();
    daemon.register(info).map_err(|e| e.to_string())?;
    Ok(fullname)
}

#[cfg(feature = "mdns")]
impl MdnsOps for mdns_sd::ServiceDaemon {
    fn register(&self, rec: &Record111) -> Result<String, String> {
        register_record(self, rec)
    }

    fn unregister(&self, fullname: &str, wait: Duration) {
        if let Ok(rx) = mdns_sd::ServiceDaemon::unregister(self, fullname) {
            if !wait.is_zero() {
                let _ = rx.recv_timeout(wait);
            }
        }
    }

    fn browse(&self, service: &str, limit: Duration) -> Result<Vec<BrowseHit>, String> {
        use mdns_sd::ServiceEvent;
        let rx = mdns_sd::ServiceDaemon::browse(self, service).map_err(|e| e.to_string())?;
        let deadline = std::time::Instant::now() + limit;
        let mut hits = Reservoir::new(BROWSE_MAX_HITS);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(left) {
                Ok(ServiceEvent::ServiceResolved(r)) => {
                    let hit = BrowseHit {
                        fullname: r.fullname.clone(),
                        port: r.port,
                        v4: r.get_addresses_v4().into_iter().collect(),
                        txt: r
                            .txt_properties
                            .iter()
                            .map(|p| (p.key().to_string(), p.val_str().to_string()))
                            .collect(),
                    };
                    hits.offer(hit);
                }
                Ok(ServiceEvent::ServiceRemoved(_, full)) => hits.remove(&full),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        let _ = mdns_sd::ServiceDaemon::stop_browse(self, service);
        // wait (bounded) for the daemon to confirm the stop, so no query is sent after this
        let confirm = std::time::Instant::now() + Duration::from_millis(500);
        loop {
            let left = confirm.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left) {
                Ok(ServiceEvent::SearchStopped(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
        Ok(hits.into_hits())
    }
}

// ---------------------------------------------------------------- the announcer

pub struct Announcer {
    #[cfg(feature = "mdns")]
    daemon: mdns_sd::ServiceDaemon,
    #[cfg(feature = "mdns")]
    fullname: String,
    /// The §3.3 re-arm task (holds the address-watch subscription).
    rearm: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Announcer {
    /// Announce `111.local` → this node's addresses, port `port`.
    #[cfg(feature = "mdns")]
    pub fn start(port: u16, on_rename: Box<dyn Fn(String) + Send>) -> Option<Announcer> {
        let started = (|| -> Result<Announcer, mdns_sd::Error> {
            let daemon = mdns_sd::ServiceDaemon::new()?;
            // IPv4 only (the LAN listener is 0.0.0.0): no AAAA, no v6 sockets.
            if let Err(e) = daemon.disable_interface(mdns_sd::IfKind::IPv6) {
                eprintln!("111 daemon: mDNS: cannot disable IPv6 ({e}); announcing A records only");
            }
            let props = std::collections::HashMap::from([("path".to_string(), "/".to_string())]);
            let mut info =
                mdns_sd::ServiceInfo::new("_http._tcp.local.", "111", HOSTNAME, (), port, props)?
                    .enable_addr_auto();
            info.set_interfaces(vec![mdns_sd::IfKind::IPv4]);
            let fullname = info.get_fullname().to_string();
            let monitor = daemon.monitor()?;
            daemon.register(info)?;
            std::thread::Builder::new()
                .name("111d-mdns-mon".into())
                .spawn(move || {
                    let mut current = HOSTNAME.trim_end_matches('.').to_string();
                    while let Ok(ev) = monitor.recv() {
                        if let mdns_sd::DaemonEvent::NameChange(c) = ev {
                            if let Some(host) = rename_target(&current, &c.original, &c.new_name)
                            {
                                eprintln!(
                                    "111 daemon: mDNS name conflict — announcing as {host} instead of {current}"
                                );
                                current = host.clone();
                                on_rename(host);
                            }
                        }
                    }
                })
                .ok();
            Ok(Announcer {
                daemon,
                fullname,
                rearm: std::sync::Mutex::new(None),
            })
        })();
        match started {
            Ok(a) => Some(a),
            Err(e) => {
                eprintln!("111 daemon: mDNS announce of 111.local failed: {e}; LAN visitors use the IP URL");
                None
            }
        }
    }

    #[cfg(not(feature = "mdns"))]
    pub fn start(_port: u16, _on_rename: Box<dyn Fn(String) + Send>) -> Option<Announcer> {
        eprintln!("111 daemon: built without the `mdns` feature — 111.local is not announced");
        None
    }

    /// The running daemon for the registry's record and browse (§7.1).
    pub fn ops(&self) -> Option<Arc<dyn MdnsOps>> {
        #[cfg(feature = "mdns")]
        {
            Some(Arc::new(self.daemon.clone()))
        }
        #[cfg(not(feature = "mdns"))]
        {
            None
        }
    }

    /// Start the §3.3 re-arm on `watch` (production: `netwatch::global()`); `on_change` runs
    /// after each re-armed event. At most one re-arm per announcer.
    pub fn start_rearm(&self, watch: &NetWatch, on_change: Arc<dyn Fn() + Send + Sync>) {
        #[cfg(feature = "mdns")]
        {
            let mut slot = self.rearm.lock().unwrap_or_else(|p| p.into_inner());
            if slot.is_none() {
                *slot = spawn_rearm(watch, Arc::new(self.daemon.clone()), on_change);
            }
        }
        #[cfg(not(feature = "mdns"))]
        {
            let _ = (watch, on_change);
        }
    }

    /// Withdraw the record (goodbye packet) and stop the mDNS daemon thread
    /// (the monitor thread ends when its channel closes). The re-arm task is
    /// aborted first, which drops its address-watch subscription.
    pub fn stop(&self) {
        if let Some(h) = self.rearm.lock().unwrap_or_else(|p| p.into_inner()).take() {
            h.abort();
        }
        #[cfg(feature = "mdns")]
        {
            if let Ok(rx) = self.daemon.unregister(&self.fullname) {
                let _ = rx.recv_timeout(std::time::Duration::from_secs(1));
            }
            let _ = self.daemon.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::relay::netwatch::{Config, NetRead, Opener};
    use crate::daemon::transport::netlink::{LinkEvents, ScriptedEvents};
    use std::sync::Mutex;
    use tokio::time::Instant;

    /// Records every `set_ip_check_interval` with its (tokio) time; answers from `script` in
    /// order (then `Ok`).
    #[derive(Default)]
    struct Calls(
        Mutex<Vec<(u32, Instant)>>,
        Mutex<std::collections::VecDeque<Result<(), IpCheckError>>>,
    );

    impl IpCheck for Calls {
        fn set_ip_check_interval(&self, secs: u32) -> Result<(), IpCheckError> {
            self.0.lock().unwrap().push((secs, Instant::now()));
            self.1.lock().unwrap().pop_front().unwrap_or(Ok(()))
        }
    }

    impl Calls {
        fn values(&self) -> Vec<u32> {
            self.0.lock().unwrap().iter().map(|(v, _)| *v).collect()
        }

        fn scripted(answers: &[Result<(), IpCheckError>]) -> Arc<Calls> {
            let c = Calls::default();
            c.1.lock().unwrap().extend(answers.iter().cloned());
            Arc::new(c)
        }
    }

    fn refused() -> Result<(), IpCheckError> {
        Err(IpCheckError::Refused("the command channel is full".into()))
    }

    /// A watch whose socket is a scripted channel (or a failed bind).
    struct Scripted {
        fail: bool,
        feed: Mutex<Option<tokio::sync::mpsc::Sender<()>>>,
    }

    impl Opener for Arc<Scripted> {
        fn open(&self, _groups: u32) -> anyhow::Result<Box<dyn LinkEvents>> {
            if self.fail {
                anyhow::bail!("netlink bind: Permission denied");
            }
            let (tx, rx) = tokio::sync::mpsc::channel(16);
            *self.feed.lock().unwrap() = Some(tx);
            Ok(Box::new(ScriptedEvents { rx }))
        }
    }

    /// A host whose fingerprint cannot be read: every debounced burst counts as a change.
    struct Unreadable;

    impl NetRead for Unreadable {
        fn routes(&self) -> Option<u64> {
            None
        }
        fn addresses(&self) -> Option<u64> {
            None
        }
    }

    fn watch(fail: bool) -> (NetWatch, Arc<Scripted>) {
        let s = Arc::new(Scripted {
            fail,
            feed: Mutex::new(None),
        });
        let cfg = Config {
            quiet: Duration::from_millis(50),
            max_wait: Duration::from_millis(300),
            poll_unit: Duration::from_secs(60),
            termux: false,
        };
        (
            NetWatch::new(cfg, Box::new(Arc::clone(&s)), Arc::new(Unreadable)),
            s,
        )
    }

    async fn until(f: impl Fn() -> bool, within: Duration) {
        let end = Instant::now() + within;
        while !f() {
            assert!(Instant::now() < end, "timed out");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    fn counter() -> (
        Arc<std::sync::atomic::AtomicUsize>,
        Arc<dyn Fn() + Send + Sync>,
    ) {
        let n = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let m = Arc::clone(&n);
        (
            n,
            Arc::new(move || {
                m.fetch_add(1, Ordering::SeqCst);
            }),
        )
    }

    /// The re-arm's sleep as measured between two recorded calls (real time: the crate has
    /// no tokio `test-util`, so no paused clock; a loaded box adds at most this much).
    fn slept(d: Duration) -> bool {
        d >= REARM_SLEEP && d < REARM_SLEEP + Duration::from_millis(400)
    }

    /// Row D8's first test (§3.3 S2): 0 only after the bind; on each debounced event 1, one
    /// one-shot 1,500 ms sleep, then 0 (within 2 s of the event); two events serialise (no
    /// set-0 lands between another event's set-1 and its own sleep).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_debounced_address_event_re_arms_the_ip_check_once() {
        let (w, s) = watch(false);
        let calls = Arc::new(Calls::default());
        let (changes, on_change) = counter();
        let sub = w.subscribe(Consumer::Mdns).unwrap();
        assert!(w.is_open(), "the second socket is bound");
        let task = tokio::spawn(rearm_logged(
            calls.clone(),
            sub,
            on_change,
            Box::leak(Box::new(AtomicBool::new(false))),
        ));
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(calls.values(), vec![0], "0 once the socket is bound");

        // one address event → after the 50 ms debounce: 1, sleep 1.5 s, 0
        let tx = s.feed.lock().unwrap().clone().unwrap();
        let event_at = Instant::now();
        tx.send(()).await.unwrap();
        until(|| calls.values().len() == 3, Duration::from_secs(2)).await;
        let got = calls.0.lock().unwrap().clone();
        assert_eq!(
            got.iter().map(|(v, _)| *v).collect::<Vec<_>>(),
            vec![0, 1, 0]
        );
        let (one_at, zero_at) = (got[1].1, got[2].1);
        assert!(slept(zero_at - one_at), "one one-shot 1,500 ms sleep");
        assert!(zero_at - event_at <= Duration::from_secs(2), "0 within 2 s");
        assert_eq!(
            changes.load(Ordering::SeqCst),
            1,
            "the registry is told once"
        );

        // two bursts back to back: each its own 1 → sleep → 0, in order
        tx.send(()).await.unwrap();
        until(|| calls.values().len() == 4, Duration::from_secs(1)).await; // burst one fired
        tx.send(()).await.unwrap(); // arrives while the re-arm sleeps
        until(|| calls.values().len() == 7, Duration::from_secs(5)).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(calls.values(), vec![0, 1, 0, 1, 0, 1, 0]);
        let got = calls.0.lock().unwrap().clone();
        for pair in got[1..].chunks(2) {
            assert!(slept(pair[1].1 - pair[0].1), "{got:?}");
        }
        assert_eq!(changes.load(Ordering::SeqCst), 3);

        // the socket stops answering: back to the 5 s check, and the task ends
        drop(tx);
        *s.feed.lock().unwrap() = None;
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the re-arm ends")
            .unwrap();
        assert_eq!(calls.values().last(), Some(&DEFAULT_IP_CHECK_SECS));
        assert_eq!(w.subscribers(), 0, "the subscription is dropped");
    }

    /// §3.3: a failed bind keeps the default 5 s check (no set at all), logged once.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_bind_keeps_the_default_check() {
        let (w, _s) = watch(true);
        let logged: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
        for _ in 0..2 {
            let calls = Arc::new(Calls::default());
            let (changes, on_change) = counter();
            let sub = w.subscribe(Consumer::Mdns).unwrap();
            rearm_logged(calls.clone(), sub, on_change, logged).await;
            assert!(calls.values().is_empty(), "the interval is never touched");
            assert_eq!(changes.load(Ordering::SeqCst), 0);
        }
        assert!(logged.load(Ordering::SeqCst));
        assert_eq!(w.failures_logged(), 1, "the bind failure is logged once");
        assert!(!w.is_open() && w.subscribers() == 0);
    }

    /// Ruling 3 (D8 review): a set the running daemon refuses is retried once and never ends
    /// the re-arm; a set of 0 or 1 refused twice puts the 5 s check back (logged once, only
    /// once a set of 5 was taken: a refused restore is tried again) and the task stays on it
    /// (events still reach the registry, the interval is left alone); the interval never rests
    /// at 1 s, nor at 0 without the re-arm. Only a gone daemon ends the task.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refused_set_retries_once_then_keeps_the_default_check() {
        // (script, the calls after the bind, the calls after one event, the interval it rests at)
        type Case = (Vec<Result<(), IpCheckError>>, Vec<u32>, Vec<u32>);
        let cases: Vec<Case> = vec![
            // set(0) at the bind refused once: retried, then the normal re-arm
            (vec![refused()], vec![0, 0], vec![0, 0, 1, 0]),
            // set(0) at the bind refused twice: back to 5, no more sets
            (vec![refused(), refused()], vec![0, 0, 5], vec![0, 0, 5]),
            // set(1) refused twice: back to 5 (never 0 with no check), no more sets
            (
                vec![Ok(()), refused(), refused()],
                vec![0],
                vec![0, 1, 1, 5],
            ),
            // the set(0) after the sleep refused twice: back to 5, never left at 1
            (
                vec![Ok(()), Ok(()), refused(), refused()],
                vec![0],
                vec![0, 1, 0, 0, 5],
            ),
            // the restore itself refused twice (after a refused set(0) at the bind): tried
            // again after RESTORE_RETRY until taken; logged as kept only then
            (
                vec![refused(), refused(), refused(), refused()],
                vec![0, 0, 5, 5, 5],
                vec![0, 0, 5, 5, 5],
            ),
            // set(1) refused twice, then the restore refused twice: never left at 0
            (
                vec![Ok(()), refused(), refused(), refused(), refused()],
                vec![0],
                vec![0, 1, 1, 5, 5, 5],
            ),
            // the set(0) after the sleep refused twice, then the restore refused twice: never
            // left at 1
            (
                vec![Ok(()), Ok(()), refused(), refused(), refused(), refused()],
                vec![0],
                vec![0, 1, 0, 0, 5, 5, 5],
            ),
        ];
        for (n, (script, after_bind, after_event)) in cases.into_iter().enumerate() {
            let (w, s) = watch(false);
            let calls = Calls::scripted(&script);
            let (changes, on_change) = counter();
            let logged: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
            let sub = w.subscribe(Consumer::Mdns).unwrap();
            let task = tokio::spawn(rearm_logged(calls.clone(), sub, on_change, logged));
            until(|| calls.values() == after_bind, Duration::from_secs(1)).await;
            let tx = s.feed.lock().unwrap().clone().unwrap();
            tx.send(()).await.unwrap();
            until(
                || changes.load(Ordering::SeqCst) == 1,
                Duration::from_secs(3),
            )
            .await;
            assert_eq!(calls.values(), after_event, "case {n}");
            assert!(
                !task.is_finished(),
                "case {n}: a refused set never ends the re-arm"
            );
            assert_eq!(
                logged.load(Ordering::SeqCst),
                after_event.last() == Some(&DEFAULT_IP_CHECK_SECS),
                "case {n}: logged once when back to 5"
            );
            // a second event: re-armed again only while quiet; the registry is told either way
            tx.send(()).await.unwrap();
            until(
                || changes.load(Ordering::SeqCst) == 2,
                Duration::from_secs(3),
            )
            .await;
            let v = calls.values();
            if after_event.last() == Some(&DEFAULT_IP_CHECK_SECS) {
                assert_eq!(v, after_event, "case {n}: the 5 s check is left alone");
            } else {
                assert_eq!(v[after_event.len()..], [1, 0], "case {n}");
            }
            assert_ne!(
                v.last(),
                Some(&REARM_CHECK_SECS),
                "case {n}: never left at 1 s"
            );
            task.abort();
        }
        // a gone daemon ends the task (at the bind, at the set(1))
        for script in [
            vec![Err(IpCheckError::Gone)],
            vec![Ok(()), Err(IpCheckError::Gone)],
        ] {
            let (w, s) = watch(false);
            let calls = Calls::scripted(&script);
            let (_changes, on_change) = counter();
            let sub = w.subscribe(Consumer::Mdns).unwrap();
            let task = tokio::spawn(rearm_logged(
                calls.clone(),
                sub,
                on_change,
                Box::leak(Box::new(AtomicBool::new(false))),
            ));
            if script.len() == 2 {
                until(|| calls.values() == [0], Duration::from_secs(1)).await;
                let tx = s.feed.lock().unwrap().clone().unwrap();
                tx.send(()).await.unwrap();
            }
            tokio::time::timeout(Duration::from_secs(3), task)
                .await
                .expect("a gone daemon ends the re-arm")
                .unwrap();
            assert_eq!(
                calls.values().len(),
                script.len(),
                "no retry of a gone daemon"
            );
        }
    }

    /// The browse's cap keeps a uniform sample (ruling 1: a flood answering first cannot crowd
    /// out a later record): over many runs a record heard after 64 forged ones is kept at about
    /// the rate `cap / n`, never at 0; a re-resolve replaces its entry; a goodbye removes it.
    #[test]
    fn the_browse_cap_keeps_a_uniform_sample() {
        let hit = |i: usize| BrowseHit {
            fullname: format!("x{i}.{SERVICE_111}"),
            port: 1,
            v4: vec![],
            txt: vec![],
        };
        let mut kept = 0;
        for _ in 0..400 {
            let mut r = Reservoir::new(BROWSE_MAX_HITS);
            for i in 0..BROWSE_MAX_HITS * 2 {
                r.offer(hit(i));
            }
            let hits = r.into_hits();
            assert_eq!(hits.len(), BROWSE_MAX_HITS);
            kept += usize::from(
                hits.iter()
                    .any(|h| h.fullname == hit(BROWSE_MAX_HITS * 2 - 1).fullname),
            );
        }
        // expected 200 of 400 (p = 64/128); far from both 0 and 400
        assert!(
            (120..=280).contains(&kept),
            "the last record kept {kept} times of 400"
        );
        let mut r = Reservoir::new(2);
        r.offer(hit(1));
        let mut again = hit(1);
        again.port = 2;
        r.offer(again);
        r.offer(hit(2));
        r.remove(&hit(2).fullname);
        let hits = r.into_hits();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].port, 2);
    }

    /// The real daemon (§3.3, G8's assertion): `get_ip_check_interval()` reads the default
    /// before the re-arm, 0 once the watch is bound, 0 again within 2 s of an event; a failed
    /// bind leaves it at 5.
    #[cfg(feature = "mdns")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_real_daemon_reads_zero_after_the_bind_and_after_each_event() {
        let Ok(daemon) = mdns_sd::ServiceDaemon::new() else {
            eprintln!("SKIPPED: no mDNS daemon on this host");
            return;
        };
        let read = |d: &mdns_sd::ServiceDaemon| d.get_ip_check_interval().unwrap();
        assert_eq!(read(&daemon), DEFAULT_IP_CHECK_SECS);
        // a failed bind: stays 5
        let (failing, _) = watch(true);
        let sub = failing.subscribe(Consumer::Mdns).unwrap();
        rearm_logged(
            Arc::new(daemon.clone()),
            sub,
            Arc::new(|| {}),
            &AtomicBool::new(false),
        )
        .await;
        assert_eq!(read(&daemon), DEFAULT_IP_CHECK_SECS);
        // bound: 0
        let (w, s) = watch(false);
        let (changes, on_change) = counter();
        let sub = w.subscribe(Consumer::Mdns).unwrap();
        let task = tokio::spawn(rearm_logged(
            Arc::new(daemon.clone()),
            sub,
            on_change,
            Box::leak(Box::new(AtomicBool::new(false))),
        ));
        let wait_for = |want: u32, within: Duration| {
            let d = daemon.clone();
            async move {
                let end = std::time::Instant::now() + within;
                loop {
                    let v = tokio::task::spawn_blocking({
                        let d = d.clone();
                        move || d.get_ip_check_interval().unwrap()
                    })
                    .await
                    .unwrap();
                    if v == want {
                        return true;
                    }
                    if std::time::Instant::now() > end {
                        return false;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        };
        assert!(
            wait_for(0, Duration::from_secs(1)).await,
            "0 after the bind"
        );
        let tx = s.feed.lock().unwrap().clone().unwrap();
        tx.send(()).await.unwrap();
        assert!(
            wait_for(REARM_CHECK_SECS, Duration::from_secs(1)).await,
            "1 while re-armed"
        );
        let started = std::time::Instant::now();
        assert!(wait_for(0, Duration::from_secs(2)).await, "0 within 2 s");
        assert!(started.elapsed() <= Duration::from_secs(2));
        assert_eq!(changes.load(Ordering::SeqCst), 1);
        task.abort();
        let _ = daemon.shutdown();
    }

    /// Row D8's first test, its last clause (ruling 5): "the A records changed" on a real
    /// address change. In a network namespace of its own (this thread only; a veth pair, no
    /// dummy module here), with the real netlink watch, the real mdns-sd daemon carrying the
    /// `_n111._tcp` record and the re-arm: with the interval at 0 and no re-arm a new address
    /// is never picked up (S3's frozen A records); with the re-arm the next address event
    /// brings the new address into the daemon (`DaemonEvent::IpAdd`, which adds it to the
    /// auto-address record) within 2 s of the debounced event, and a second daemon resolving
    /// the host hears it. Skipped (printed) only where the box offers no network namespace or
    /// no `ip` (iproute2).
    #[cfg(feature = "mdns")]
    #[test]
    fn a_real_address_change_changes_the_a_records_through_the_re_arm() {
        let out = std::thread::spawn(netns_a_records).join();
        match out {
            Ok(Ok(())) => {}
            Ok(Err(skip)) => eprintln!("SKIPPED: {skip}"),
            Err(p) => std::panic::resume_unwind(p),
        }
    }

    /// The body of the test above, on its own thread (the namespace dies with it and the two
    /// daemon threads it spawned). `Err` = the box cannot run it.
    #[cfg(feature = "mdns")]
    fn netns_a_records() -> Result<(), String> {
        use crate::daemon::relay::netwatch::{HostNet, NetlinkOpener};
        use std::net::IpAddr;
        // SAFETY: unshare(2) with CLONE_NEWNET moves this thread (and what it spawns) only.
        if unsafe { libc::unshare(libc::CLONE_NEWNET) } != 0 {
            return Err(format!(
                "no network namespace: {}",
                std::io::Error::last_os_error()
            ));
        }
        let ip = |args: &str| -> Result<(), String> {
            let st = std::process::Command::new("ip")
                .args(args.split(' '))
                .status()
                .map_err(|e| format!("no `ip` (iproute2): {e}"))?;
            st.success()
                .then_some(())
                .ok_or_else(|| format!("`ip {args}` failed: {st}"))
        };
        ip("link set lo up")?;
        ip("link add d8a type veth peer name d8b")?;
        ip("addr add 10.77.0.1/24 dev d8a")?;
        ip("link set d8a up")?;
        ip("link set d8b up")?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        rt.block_on(async move {
            let daemon = mdns_sd::ServiceDaemon::new().map_err(|e| e.to_string())?;
            let _ = daemon.disable_interface(mdns_sd::IfKind::IPv6);
            let monitor = daemon.monitor().map_err(|e| e.to_string())?;
            let rec = Record111 {
                instance: "d8ns-test-aa".into(),
                host: "d8-netns.local".into(),
                port: 11183,
                roles: 0x0002,
                node_id: [0x42; 32],
            };
            register_record(&daemon, &rec).expect("the _n111._tcp record registers");
            let added = |want: IpAddr, within: Duration| {
                let monitor = monitor.clone();
                async move {
                    let end = std::time::Instant::now() + within;
                    while std::time::Instant::now() < end {
                        while let Ok(ev) = monitor.try_recv() {
                            if matches!(ev, mdns_sd::DaemonEvent::IpAdd(a) if a == want) {
                                return true;
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    false
                }
            };
            // the start-time address (drains the start's events)
            let first: IpAddr = "10.77.0.1".parse().unwrap();
            let _ = added(first, Duration::from_secs(1)).await;

            // the control: interval 0, no re-arm: a new address is never picked up
            daemon.set_ip_check_interval(0).unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
            ip("addr add 10.77.2.1/24 dev d8a")?;
            let frozen: IpAddr = "10.77.2.1".parse().unwrap();
            assert!(
                !added(frozen, Duration::from_millis(3_500)).await,
                "with the check at 0 and no re-arm the A records stay frozen (S3)"
            );

            // the re-arm on the real watch: the next address event brings both in
            let watch = NetWatch::new(
                Config {
                    quiet: Duration::from_millis(100),
                    max_wait: Duration::from_millis(500),
                    poll_unit: Duration::from_secs(60),
                    termux: false,
                },
                Box::new(NetlinkOpener),
                Arc::new(HostNet),
            );
            let sub = watch
                .subscribe(Consumer::Mdns)
                .map_err(|e| format!("{e:?}"))?;
            if !matches!(sub.watching(), Watching::Netlink) {
                return Err("the netlink watch could not bind in the namespace".into());
            }
            let (changes, on_change) = counter();
            let task = tokio::spawn(rearm_logged(
                Arc::new(daemon.clone()),
                sub,
                on_change,
                Box::leak(Box::new(AtomicBool::new(false))),
            ));
            tokio::time::sleep(Duration::from_millis(100)).await;
            let event_at = std::time::Instant::now();
            ip("addr add 10.77.1.1/24 dev d8b")?;
            let new: IpAddr = "10.77.1.1".parse().unwrap();
            assert!(
                added(new, Duration::from_secs(3)).await,
                "the re-armed check picked the new address up"
            );
            let took = event_at.elapsed();
            assert!(
                took <= Duration::from_millis(100 + 500) + Duration::from_secs(2),
                "within 2 s of the debounced event: {took:?}"
            );
            let end = std::time::Instant::now() + Duration::from_secs(3);
            while changes.load(Ordering::SeqCst) == 0 {
                assert!(std::time::Instant::now() < end, "the registry was told");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let rested = tokio::task::spawn_blocking({
                let d = daemon.clone();
                move || d.get_ip_check_interval().unwrap()
            })
            .await
            .unwrap();
            assert_eq!(rested, 0, "the check rests at 0 again");

            // a second daemon resolving the host hears the new A record
            let b = mdns_sd::ServiceDaemon::new().map_err(|e| e.to_string())?;
            let _ = b.disable_interface(mdns_sd::IfKind::IPv6);
            let rx = b
                .resolve_hostname("d8-netns.local.", Some(3_000))
                .map_err(|e| e.to_string())?;
            let heard = tokio::task::spawn_blocking(move || {
                let mut all = std::collections::HashSet::new();
                while let Ok(ev) = rx.recv_timeout(Duration::from_secs(4)) {
                    match ev {
                        mdns_sd::HostnameResolutionEvent::AddressesFound(_, a) => {
                            all.extend(a.into_iter().map(|x| x.to_ip_addr()))
                        }
                        mdns_sd::HostnameResolutionEvent::SearchStopped(_) => break,
                        _ => {}
                    }
                }
                all
            })
            .await
            .unwrap();
            task.abort();
            for d in [&daemon, &b] {
                if let Ok(rx) = d.shutdown() {
                    let _ = rx.recv_timeout(Duration::from_secs(2));
                }
            }
            if heard.is_empty() {
                eprintln!(
                    "note: the second daemon heard no answer in the namespace (no multicast path)"
                );
            } else {
                assert!(heard.contains(&new), "the A records changed: {heard:?}");
            }
            Ok(())
        })
    }

    #[test]
    fn the_record_s_txt_is_78_bytes_and_names_nothing_else() {
        let rec = Record111 {
            instance: "abcd-efgh-ij".into(),
            host: "111.local".into(),
            port: 11183,
            roles: 0x0003,
            node_id: [0xab; 32],
        };
        let txt = rec.txt();
        assert_eq!(txt[0], ("v", "1".to_string()));
        assert_eq!(txt[1], ("r", "0003".to_string()));
        assert_eq!(txt[2].0, "k");
        assert_eq!(txt[2].1, "ab".repeat(32));
        assert_eq!(rec.txt_wire_len(), 78, "§7.1: 78 B with length bytes");
        assert_eq!(rec.host_fqdn(), "111.local.");
        let rec = Record111 {
            host: "111-2.local.".into(),
            ..rec
        };
        assert_eq!(rec.host_fqdn(), "111-2.local.");
    }

    #[test]
    fn first_and_further_renames_are_followed() {
        // first conflict: 111.local → 111-2.local
        assert_eq!(
            rename_target("111.local", "111.local.", "111-2.local."),
            Some("111-2.local".into())
        );
        // the same rename reported again (A record / another interface): no change
        assert_eq!(
            rename_target("111-2.local", "111.local.", "111-2.local."),
            None
        );
        // second conflict, as mdns-sd reports it (original = registered name)
        assert_eq!(
            rename_target("111-2.local", "111.local.", "111-3.local."),
            Some("111-3.local".into())
        );
        // ... or as a chain from the current name
        assert_eq!(
            rename_target("111-2.local", "111-2.local.", "111-3.local."),
            Some("111-3.local".into())
        );
        // someone else's name change is not ours
        assert_eq!(
            rename_target("111.local", "printer.local.", "printer-2.local."),
            None
        );
        assert_eq!(rename_target("111.local", "111.local.", ""), None);
    }
}
