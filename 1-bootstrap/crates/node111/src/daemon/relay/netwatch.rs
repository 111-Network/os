//! Layer 7 (§3.2–§3.4; S15; §19 D8): the address and route watch.
//!
//! The spine's netlink socket binds `RTMGRP_LINK` only and its `debounced` instance re-renders
//! rnsd (`transport::LINK_WATCH_GROUPS`, `transport/mod.rs`); it is untouched. This module owns a
//! **second** route-netlink socket bound to [`GROUPS`] (`RTMGRP_LINK | RTMGRP_IPV4_IFADDR |
//! RTMGRP_IPV4_ROUTE | RTMGRP_IPV6_IFADDR | RTMGRP_IPV6_ROUTE`, `0x551`, §3.3), fed through a
//! second debounce instance ([`DEBOUNCE`], 2 s, bounded by [`DEBOUNCE_MAX`]:
//! `netlink::debounced_within`), so a burst of route updates is one [`NetEvent::Changed`] and a
//! DHCP renew never reaches the spine.
//!
//! **Only a real change is delivered.** The kernel sends `RTM_NEWADDR` for an address whose
//! lifetimes alone were refreshed (a DHCP renew; on an IPv6 LAN every Router Advertisement that
//! refreshes a SLAAC address): measured in a network namespace (row D4b), `ip addr change` with
//! the same address and prefix and new `valid_lft`/`preferred_lft` sends one `RTM_NEWADDR` per
//! refresh. Counting each datagram would wake the carrier check (D11) and the mdns-sd re-arm
//! (D8) all day with nothing changed (against the Pi floor's "idle outbound 0"). So after each
//! debounced burst the watch reads one fingerprint ([`change_fingerprint`]) of what its
//! consumers care about: the set of (interface index, name, family, address, prefix length),
//! each interface's `IFF_UP | IFF_RUNNING`, and the routing tables (the §3.4 read, counters
//! ignored); lifetimes are not in it. [`NetEvent::Changed`] is delivered only when it moved
//! since the last delivery (or the open), or when it cannot be read (then every burst counts,
//! as before). The read is one `getifaddrs(3)` (the libc the crate links) plus the two capped
//! `/proc` reads, once per burst, in the reader task that exists anyway: lighter than parsing
//! every datagram, which needs a dump request at open to know what an `RTM_NEWADDR` refreshes,
//! and it adds no task, timer or socket. A burst that never goes quiet for [`DEBOUNCE`] is still
//! checked [`DEBOUNCE_MAX`] after its first datagram, so the event is never starved.
//!
//! **What the fingerprint cannot see (row D4c).** Its route part is `/proc/net/route` (the main
//! table) and `/proc/net/ipv6_route`, while the socket delivers route messages of every table:
//! a default route changed in a policy table (a VPN, Tailscale's exit node, NetworkManager,
//! Android's per-network tables) with the addresses unchanged would read as no change. So the
//! reader also looks at each datagram's message headers (`netlink::classify`, sizes checked
//! before every read): a burst that carried a route message of a table other than main or
//! local, a deleted address, any link message, or a short, unknown or unparsable message is
//! delivered without asking the fingerprint (which is still read, as the next baseline). A
//! burst of `RTM_NEWADDR` and main / local table route messages only is settled by the
//! fingerprint as above, so a lifetime-only refresh still delivers nothing. What remains: a
//! change made and reverted inside one burst of that second kind only is not seen.
//!
//! **Consumers subscribe** ([`NetWatch::subscribe`]): a configured carrier (row D11, its check
//! of §3.2 (a)) and the running mdns-sd daemon (row D8, the §3.3 re-arm). The first subscriber
//! opens the socket and spawns its one reader task; the last [`Subscription`] dropped aborts the
//! task, which closes the socket. With no subscriber there is no socket, task or timer here, so
//! a node with neither a carrier nor the mdns-sd daemon keeps the Layer 6 socket set exactly
//! (§3.3; §1 rule 2). Netlink sockets per configuration: none ⇒ the spine's one (Layer 6,
//! unchanged); mdns-sd on ⇒ two; a carrier on ⇒ two; both ⇒ two (one shared second socket); a
//! failed bind ⇒ the spine's one.
//!
//! **A failed bind** is logged once per process and reported to every subscriber through
//! [`Subscription::watching`] ([`Watching::Failed`]); a socket that later stops answering is
//! reported as [`NetEvent::Failed`]. No retry runs while subscribed (no timer); the next first
//! subscriber tries again. For mdns-sd a failure keeps its default 5 s IP check (§3.3, D8's
//! rule); for a carrier it selects the Termux fallback.
//!
//! **The Termux fallback (§3.4).** Only on a Termux node ([`crate::config::on_termux`]), only
//! while a [`Consumer::Carrier`] is subscribed (a carrier is hub-only, §3.1) and only while the
//! bind failed: a coarse poll every `gateway.termux_poll_min` minutes (default 15, floor 9,
//! [`super::settings`]). The poll reads the same [`change_fingerprint`] the netlink path uses
//! (row D4c): the interfaces, their `IFF_UP | IFF_RUNNING` and their addresses through one
//! `getifaddrs(3)`, plus the routing tables `/proc/net/route` and `/proc/net/ipv6_route` (small
//! text reads, no socket; lighter than a netlink dump, which needs a socket of the very kind
//! whose bind failed, a request and a multipart parse). On Android the IPv4 main table is
//! mostly empty (per-network policy tables), so a Wi-Fi / cellular switch may not move the
//! routes; it moves the addresses. It delivers [`NetEvent::Poll`] to the carrier when that
//! fingerprint changed (the counter columns and address lifetimes ignored) or when it cannot
//! be read (then every tick: the check is the only way left to learn the network; an Android
//! that hides `/proc/net/route` from apps is such a host). One bounded read per tick, nothing
//! new resident. No other node, and no other consumer, polls. The last
//! fingerprint the poll read lives in the watch's state, so a poll restarted for a new period
//! (a changed `termux_poll_min`, a second carrier) compares against it and a change that
//! happened since the old task's last tick is still delivered; it is forgotten only when the
//! poll stops.
//!
//! Every wait is a monotonic `tokio::time` sleep; no wall clock decides anything here.

use crate::daemon::relay::settings::{MAX_TERMUX_POLL_MIN, MIN_TERMUX_POLL_MIN};
use crate::daemon::transport::netlink::{self, LinkEvents, ADDRESS_ROUTE_GROUPS};
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// The second socket's multicast groups (§3.3; `LC/:2321-2331`).
pub const GROUPS: u32 = ADDRESS_ROUTE_GROUPS;
/// The second `debounced` instance's quiet window (§3.3: 2 s, as the spine's).
pub const DEBOUNCE: Duration = Duration::from_secs(2);
/// The bound on one burst (row D4b): a burst that never goes quiet for [`DEBOUNCE`] is checked
/// this long after its first datagram.
pub const DEBOUNCE_MAX: Duration = Duration::from_secs(10);
/// Most `getifaddrs` entries walked; a longer list reads as unknown (every burst counts).
pub const IFADDRS_MAX: usize = 4_096;
/// The Termux poll's unit (`gateway.termux_poll_min` is in minutes).
pub const POLL_UNIT: Duration = Duration::from_secs(60);
/// The IPv4 routing table the Termux poll reads (§3.4).
pub const PROC_ROUTE: &str = "/proc/net/route";
/// The IPv6 routing table the Termux poll reads (§3.4).
pub const PROC_ROUTE6: &str = "/proc/net/ipv6_route";
/// Largest routing table read (bytes); the rest of a larger file is not read.
pub const ROUTE_READ_MAX: u64 = 64 * 1024;

const CHANGED: u8 = 1;
const POLL: u8 = 2;
const FAILED: u8 = 4;

/// Who subscribes (§3.3). Only these two consumers exist in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consumer {
    /// A configured carrier (row D11): each event is one carrier check (§3.2 (a)).
    /// `termux_poll_min` is `gateway.termux_poll_min`; it matters only for the §3.4 fallback
    /// (clamped to 9..=1440 minutes). A changed setting is a new subscription.
    Carrier { termux_poll_min: u32 },
    /// The running mdns-sd daemon's IP-check re-arm (row D8, §3.3 S2).
    Mdns,
}

/// One event to a subscriber. Pending events coalesce: a subscriber that has not yet called
/// [`Subscription::next`] sees each kind at most once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetEvent {
    /// A debounced address, route or link change (§3.3): at most one per burst, and only when
    /// [`change_fingerprint`] moved (or cannot be read; row D4b).
    Changed,
    /// A Termux fallback tick that saw the routing tables change, or could not read them (§3.4).
    Poll,
    /// The bound socket stopped answering; [`Subscription::watching`] now reads
    /// [`Watching::Failed`] (mdns-sd: restore the 5 s check; carrier: §3.4 applies).
    Failed,
}

/// What the watch rests on, as a subscriber sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Watching {
    /// The second socket is bound: events arrive as [`NetEvent::Changed`].
    Netlink,
    /// The bind failed (logged once). `poll` is the §3.4 period when this subscriber is a
    /// carrier on a Termux node, else `None` (no event source: the consumer keeps its default).
    Failed {
        error: String,
        poll: Option<Duration>,
    },
}

/// Why [`NetWatch::subscribe`] refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetWatchError {
    /// Called outside a tokio runtime (the reader task and the socket's readiness need one).
    NoRuntime,
}

impl std::fmt::Display for NetWatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetWatchError::NoRuntime => f.write_str("the address watch needs a tokio runtime"),
        }
    }
}

impl std::error::Error for NetWatchError {}

/// Opens the second socket (production: [`NetlinkOpener`]; tests inject a fake).
pub trait Opener: Send + Sync {
    /// Open a source bound to `groups`; an error is a failed bind.
    fn open(&self, groups: u32) -> anyhow::Result<Box<dyn LinkEvents>>;
}

/// The production opener: `netlink::NetlinkEvents::new(groups)`.
pub struct NetlinkOpener;

impl Opener for NetlinkOpener {
    fn open(&self, groups: u32) -> anyhow::Result<Box<dyn LinkEvents>> {
        Ok(Box::new(netlink::NetlinkEvents::new(groups)?))
    }
}

/// Reads the host's network fingerprints (production: [`HostNet`]; tests inject a fake).
pub trait NetRead: Send + Sync {
    /// The routing tables' fingerprint ([`route_fingerprint`]; the §3.4 poll reads only this).
    /// `None` when no table could be read.
    fn routes(&self) -> Option<u64>;
    /// The interfaces' and addresses' fingerprint ([`address_fingerprint`]). `None` when they
    /// could not be read.
    fn addresses(&self) -> Option<u64>;
}

/// What a netlink burst is checked against (row D4b): the addresses and the routes together.
/// `None` when either cannot be read (the burst then counts as a change).
pub fn change_fingerprint(net: &dyn NetRead) -> Option<u64> {
    let (a, r) = (net.addresses()?, net.routes()?);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (a, r).hash(&mut h);
    Some(h.finish())
}

/// The production reader: [`PROC_ROUTE`] and [`PROC_ROUTE6`], each capped at
/// [`ROUTE_READ_MAX`], and one `getifaddrs(3)` walk capped at [`IFADDRS_MAX`] entries.
pub struct HostNet;

impl NetRead for HostNet {
    fn routes(&self) -> Option<u64> {
        route_fingerprint(
            read_capped(PROC_ROUTE).as_deref(),
            read_capped(PROC_ROUTE6).as_deref(),
        )
    }

    fn addresses(&self) -> Option<u64> {
        read_interfaces().map(|mut v| address_fingerprint(&mut v))
    }
}

/// One row of the address fingerprint (row D4b). Lifetimes and address flags are not here: a
/// refresh that changes only them is no change.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NetEntry {
    /// An interface: its index, name and `IFF_UP | IFF_RUNNING` bits (a link that goes down or
    /// loses its carrier is a change).
    Link {
        index: u32,
        name: String,
        flags: u32,
    },
    /// An address: its interface's index (0 when the walk named none) and name, the family
    /// (`AF_INET`, `AF_INET6`), the address bytes and the prefix length.
    Addr {
        index: u32,
        name: String,
        family: u16,
        addr: Vec<u8>,
        prefix: u8,
    },
}

/// The order-free fingerprint of `entries` (sorted in place, then hashed).
pub fn address_fingerprint(entries: &mut [NetEntry]) -> u64 {
    entries.sort_unstable();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    entries.hash(&mut h);
    h.finish()
}

/// The `IFF_*` bits an interface row keeps.
const LINK_FLAGS: u32 = (libc::IFF_UP | libc::IFF_RUNNING) as u32;

/// Frees a `getifaddrs` list on every path.
struct IfAddrs(*mut libc::ifaddrs);

impl Drop for IfAddrs {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the list came from getifaddrs and is freed once, here.
            unsafe { libc::freeifaddrs(self.0) };
        }
    }
}

/// One `getifaddrs(3)` walk into [`NetEntry`] rows. `None` when the call fails or the list is
/// longer than [`IFADDRS_MAX`]. Entries of another family are skipped.
fn read_interfaces() -> Option<Vec<NetEntry>> {
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs writes a list head we own and free through `IfAddrs`.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return None;
    }
    let list = IfAddrs(head);
    let mut links: Vec<NetEntry> = Vec::new();
    let mut addrs: Vec<(String, u16, Vec<u8>, u8)> = Vec::new();
    let mut cur = list.0;
    let mut walked = 0usize;
    while !cur.is_null() {
        walked += 1;
        if walked > IFADDRS_MAX {
            return None;
        }
        // SAFETY: `cur` is a non-null node of the list getifaddrs returned, alive until drop.
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_name.is_null() || ifa.ifa_addr.is_null() {
            continue;
        }
        // SAFETY: a non-null, NUL-terminated name owned by the list.
        let name = unsafe { std::ffi::CStr::from_ptr(ifa.ifa_name) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: ifa_addr is non-null; every sockaddr starts with its family.
        let family = i32::from(unsafe { (*ifa.ifa_addr).sa_family });
        match family {
            libc::AF_PACKET => {
                // SAFETY: an AF_PACKET entry's address is a sockaddr_ll (packet(7)).
                let ll = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_ll) };
                links.push(NetEntry::Link {
                    index: u32::try_from(ll.sll_ifindex).unwrap_or(0),
                    name,
                    flags: ifa.ifa_flags & LINK_FLAGS,
                });
            }
            libc::AF_INET => {
                // SAFETY: an AF_INET entry's address (and netmask, when set) is a sockaddr_in.
                let a = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
                let prefix = if ifa.ifa_netmask.is_null() {
                    u8::MAX
                } else {
                    let m = unsafe { &*(ifa.ifa_netmask as *const libc::sockaddr_in) };
                    m.sin_addr.s_addr.count_ones() as u8
                };
                let bytes = a.sin_addr.s_addr.to_ne_bytes().to_vec();
                addrs.push((name, libc::AF_INET as u16, bytes, prefix));
            }
            libc::AF_INET6 => {
                // SAFETY: an AF_INET6 entry's address (and netmask, when set) is a sockaddr_in6.
                let a = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in6) };
                let prefix = if ifa.ifa_netmask.is_null() {
                    u8::MAX
                } else {
                    let m = unsafe { &*(ifa.ifa_netmask as *const libc::sockaddr_in6) };
                    m.sin6_addr
                        .s6_addr
                        .iter()
                        .map(|b| b.count_ones())
                        .sum::<u32>() as u8
                };
                addrs.push((
                    name,
                    libc::AF_INET6 as u16,
                    a.sin6_addr.s6_addr.to_vec(),
                    prefix,
                ));
            }
            _ => {}
        }
    }
    drop(list);
    let index_of: std::collections::HashMap<&str, u32> = links
        .iter()
        .filter_map(|l| match l {
            NetEntry::Link { index, name, .. } => Some((name.as_str(), *index)),
            NetEntry::Addr { .. } => None,
        })
        .collect();
    let mut out: Vec<NetEntry> = addrs
        .into_iter()
        .map(|(name, family, addr, prefix)| NetEntry::Addr {
            index: index_of.get(name.as_str()).copied().unwrap_or(0),
            name,
            family,
            addr,
            prefix,
        })
        .collect();
    out.append(&mut links);
    Some(out)
}

fn read_capped(path: &str) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(ROUTE_READ_MAX).read_to_string(&mut text).ok()?;
    Some(text)
}

/// The fingerprint of the two routing tables (§3.4): every row's columns except the counters
/// (`/proc/net/route`'s RefCnt and Use, `/proc/net/ipv6_route`'s refcnt and use), the header
/// dropped. `None` only when neither table was read (an absent IPv6 table is not a failure).
pub fn route_fingerprint(v4: Option<&str>, v6: Option<&str>) -> Option<u64> {
    if v4.is_none() && v6.is_none() {
        return None;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Some(t) = v4 {
        0u8.hash(&mut h);
        // Iface Destination Gateway Flags RefCnt Use Metric Mask MTU Window IRTT
        for line in t.lines().skip(1) {
            for (i, col) in line.split_whitespace().enumerate() {
                if i != 4 && i != 5 {
                    col.hash(&mut h);
                }
            }
            b'\n'.hash(&mut h);
        }
    }
    if let Some(t) = v6 {
        1u8.hash(&mut h);
        // dest plen src plen nexthop metric refcnt use flags dev (no header)
        for line in t.lines() {
            for (i, col) in line.split_whitespace().enumerate() {
                if i != 6 && i != 7 {
                    col.hash(&mut h);
                }
            }
            b'\n'.hash(&mut h);
        }
    }
    Some(h.finish())
}

/// The watch's timings and host facts (production: [`Config::production`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// The debounce quiet window ([`DEBOUNCE`]).
    pub quiet: Duration,
    /// The bound on one burst ([`DEBOUNCE_MAX`]).
    pub max_wait: Duration,
    /// One `termux_poll_min` minute ([`POLL_UNIT`]; tests shrink it).
    pub poll_unit: Duration,
    /// This node runs under Termux (§3.4).
    pub termux: bool,
}

impl Config {
    /// 2 s debounce bounded at 10 s, minutes of 60 s, and this host's Termux detection
    /// ([`crate::config::on_termux`]).
    pub fn production() -> Self {
        Config {
            quiet: DEBOUNCE,
            max_wait: DEBOUNCE_MAX,
            poll_unit: POLL_UNIT,
            termux: crate::config::on_termux(),
        }
    }
}

/// The §3.4 period for `gateway.termux_poll_min` (clamped to the settings' bounds).
pub fn poll_period(termux_poll_min: u32, unit: Duration) -> Duration {
    unit * termux_poll_min.clamp(MIN_TERMUX_POLL_MIN, MAX_TERMUX_POLL_MIN)
}

/// The address and route watch (§3.3). One per process ([`global`]); constructing it opens
/// nothing.
pub struct NetWatch {
    inner: Arc<Inner>,
}

/// One consumer's hold on the watch. Dropping the last one closes the socket.
pub struct Subscription {
    slot: Arc<Slot>,
    inner: Arc<Inner>,
}

struct Slot {
    id: u64,
    consumer: Consumer,
    pending: AtomicU8,
    notify: Notify,
}

struct Inner {
    cfg: Config,
    opener: Box<dyn Opener>,
    net: Arc<dyn NetRead>,
    failures_logged: AtomicU32,
    state: Mutex<State>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Link {
    Idle,
    Bound,
    Failed(String),
}

struct State {
    next_id: u64,
    slots: Vec<Arc<Slot>>,
    /// Bumped at every open, so a reader that ends after a close and a reopen changes nothing.
    epoch: u64,
    link: Link,
    reader: Option<JoinHandle<()>>,
    /// The netlink path's last delivered fingerprint ([`change_fingerprint`], read at the open):
    /// a burst is delivered only when it moved, or when it is `None` (unreadable).
    seen: Option<u64>,
    poll: Option<(Duration, JoinHandle<()>)>,
    /// Bumped at every poll start and wherever a poll task is aborted (a restart, a stop, the
    /// close), so a task that already woke and read changes nothing after it (row D4c).
    poll_gen: u64,
    /// The §3.4 poll's last [`change_fingerprint`] reading (`Some(None)`: unreadable), kept
    /// across a poll restart and forgotten when the poll stops.
    poll_seen: Option<Option<u64>>,
    rt: Option<Handle>,
}

static GLOBAL: OnceLock<NetWatch> = OnceLock::new();

/// The process's watch (production opener, [`HostNet`], [`Config::production`]). Built on
/// first use, by the first consumer; building it opens nothing.
pub fn global() -> &'static NetWatch {
    GLOBAL.get_or_init(|| {
        NetWatch::new(
            Config::production(),
            Box::new(NetlinkOpener),
            Arc::new(HostNet),
        )
    })
}

impl NetWatch {
    /// A watch over `opener` and `net`. Opens nothing until the first subscriber.
    pub fn new(cfg: Config, opener: Box<dyn Opener>, net: Arc<dyn NetRead>) -> Self {
        NetWatch {
            inner: Arc::new(Inner {
                cfg,
                opener,
                net,
                failures_logged: AtomicU32::new(0),
                state: Mutex::new(State {
                    next_id: 0,
                    slots: Vec::new(),
                    epoch: 0,
                    link: Link::Idle,
                    reader: None,
                    seen: None,
                    poll: None,
                    poll_gen: 0,
                    poll_seen: None,
                    rt: None,
                }),
            }),
        }
    }

    /// Subscribe `consumer` (§3.3). The first subscriber opens the socket (a failed bind is
    /// logged once and reported by [`Subscription::watching`]); a carrier on a Termux node
    /// whose bind failed starts the §3.4 poll. Must run inside a tokio runtime.
    pub fn subscribe(&self, consumer: Consumer) -> Result<Subscription, NetWatchError> {
        let rt = Handle::try_current().map_err(|_| NetWatchError::NoRuntime)?;
        let mut st = self.inner.lock();
        st.rt = Some(rt);
        st.next_id += 1;
        let slot = Arc::new(Slot {
            id: st.next_id,
            consumer,
            pending: AtomicU8::new(0),
            notify: Notify::new(),
        });
        st.slots.push(Arc::clone(&slot));
        if st.slots.len() == 1 {
            self.inner.open(&mut st);
        }
        self.inner.reconcile_poll(&mut st);
        drop(st);
        Ok(Subscription {
            slot,
            inner: Arc::clone(&self.inner),
        })
    }

    /// Subscribers now.
    pub fn subscribers(&self) -> usize {
        self.inner.lock().slots.len()
    }

    /// The second socket is open (a reader task holds it).
    pub fn is_open(&self) -> bool {
        self.inner.lock().reader.is_some()
    }

    /// The §3.4 poll runs, and its period.
    pub fn polling(&self) -> Option<Duration> {
        self.inner.lock().poll.as_ref().map(|(p, _)| *p)
    }

    /// Failed binds logged so far (at most one per process: "logged once").
    pub fn failures_logged(&self) -> u32 {
        self.inner.failures_logged.load(Ordering::Relaxed)
    }
}

impl Subscription {
    /// The consumer this subscription was made for.
    pub fn consumer(&self) -> Consumer {
        self.slot.consumer
    }

    /// What the watch rests on now (§3.3: the bind failure is reported here).
    pub fn watching(&self) -> Watching {
        let st = self.inner.lock();
        match &st.link {
            Link::Bound => Watching::Netlink,
            Link::Failed(error) => Watching::Failed {
                error: error.clone(),
                poll: match self.slot.consumer {
                    Consumer::Carrier { .. } => st.poll.as_ref().map(|(p, _)| *p),
                    Consumer::Mdns => None,
                },
            },
            // unreachable while a subscription exists; fail closed
            Link::Idle => Watching::Failed {
                error: "the address watch is closed".into(),
                poll: None,
            },
        }
    }

    /// The next event (cancel-safe: a pending event stays pending if the future is dropped).
    /// [`NetEvent::Failed`] comes first; a pending change and poll coalesce into one
    /// [`NetEvent::Changed`].
    pub async fn next(&self) -> NetEvent {
        loop {
            let bits = self.slot.pending.swap(0, Ordering::AcqRel);
            if bits & FAILED != 0 {
                let rest = bits & !FAILED;
                if rest != 0 {
                    self.slot.pending.fetch_or(rest, Ordering::AcqRel);
                }
                return NetEvent::Failed;
            }
            if bits & CHANGED != 0 {
                return NetEvent::Changed;
            }
            if bits & POLL != 0 {
                return NetEvent::Poll;
            }
            self.slot.notify.notified().await;
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut st = self.inner.lock();
        st.slots.retain(|s| s.id != self.slot.id);
        if st.slots.is_empty() {
            self.inner.close(&mut st);
        } else {
            self.inner.reconcile_poll(&mut st);
        }
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// First subscriber: bind the socket and spawn its reader (the second `debounced`).
    fn open(self: &Arc<Self>, st: &mut State) {
        st.epoch += 1;
        let epoch = st.epoch;
        let Some(rt) = st.rt.clone() else {
            st.link = Link::Failed("no runtime".into());
            return;
        };
        match self.opener.open(GROUPS) {
            Ok(source) => {
                st.link = Link::Bound;
                // the baseline a burst is compared with (row D4b)
                st.seen = change_fingerprint(&*self.net);
                let weak: Weak<Inner> = Arc::downgrade(self);
                let (quiet, max) = (self.cfg.quiet, self.cfg.max_wait);
                st.reader = Some(rt.spawn(async move {
                    let fire_to = weak.clone();
                    netlink::debounced_within(source, quiet, max, move |forced| {
                        if let Some(inner) = fire_to.upgrade() {
                            inner.settle(epoch, forced);
                        }
                    })
                    .await;
                    if let Some(inner) = weak.upgrade() {
                        inner.source_ended(epoch);
                    }
                }));
            }
            Err(e) => {
                let error = format!("{e:#}");
                self.log_failure(&error);
                st.link = Link::Failed(error);
            }
        }
    }

    /// Last subscriber gone: abort the reader (dropping the source closes the socket) and the
    /// poll.
    fn close(&self, st: &mut State) {
        if let Some(h) = st.reader.take() {
            h.abort();
        }
        if let Some((_, h)) = st.poll.take() {
            h.abort();
            st.poll_gen += 1;
        }
        st.link = Link::Idle;
        st.seen = None;
        st.poll_seen = None;
        st.rt = None;
    }

    /// A debounced burst ended (row D4b): deliver [`NetEvent::Changed`] when the burst was
    /// `forced` (a message in it counts as a change on its own, row D4c), or when the
    /// fingerprint moved since the last delivery (or the open), or cannot be read. The
    /// fingerprint is read either way (the next burst's baseline), before the lock is taken.
    fn settle(&self, epoch: u64, forced: bool) {
        let now = change_fingerprint(&*self.net);
        let mut st = self.lock();
        if st.epoch != epoch || st.link != Link::Bound {
            return;
        }
        if forced || now.is_none() || now != st.seen {
            deliver(&st, CHANGED, |_| true);
        }
        st.seen = now;
    }

    /// The reader's stream ended (the socket errored): report it and fall back.
    fn source_ended(self: &Arc<Self>, epoch: u64) {
        let mut st = self.lock();
        if st.epoch != epoch || st.link != Link::Bound {
            return;
        }
        st.reader = None;
        let error = "the netlink stream ended".to_string();
        self.log_failure(&error);
        st.link = Link::Failed(error);
        deliver(&st, FAILED, |_| true);
        self.reconcile_poll(&mut st);
    }

    fn log_failure(&self, error: &str) {
        if self
            .failures_logged
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            eprintln!(
                "111 relay: the address and route watch cannot use netlink ({error}); mdns-sd \
                 keeps its 5 s address check{}",
                if self.cfg.termux {
                    ", a carrier polls every gateway.termux_poll_min"
                } else {
                    ""
                }
            );
        }
    }

    /// Run the §3.4 poll exactly when this is a Termux node, the bind failed and a carrier is
    /// subscribed (the shortest subscribed period wins).
    fn reconcile_poll(self: &Arc<Self>, st: &mut State) {
        let want = if self.cfg.termux && matches!(st.link, Link::Failed(_)) {
            st.slots
                .iter()
                .filter_map(|s| match s.consumer {
                    Consumer::Carrier { termux_poll_min } => {
                        Some(poll_period(termux_poll_min, self.cfg.poll_unit))
                    }
                    Consumer::Mdns => None,
                })
                .min()
        } else {
            None
        };
        if st.poll.as_ref().map(|(p, _)| *p) == want {
            return;
        }
        if let Some((_, h)) = st.poll.take() {
            h.abort();
            // a tick already past its read must not write its reading back (row D4c)
            st.poll_gen += 1;
        }
        let (Some(period), Some(rt)) = (want, st.rt.clone()) else {
            // the poll stops: its reading is forgotten (the next start reads a new baseline)
            st.poll_seen = None;
            return;
        };
        // a restart (a new period) keeps the last reading as its baseline: a change since the
        // old task's last tick is delivered at the new task's first tick (row D4b)
        if st.poll_seen.is_none() {
            st.poll_seen = Some(change_fingerprint(&*self.net));
        }
        st.poll_gen += 1;
        let gen = st.poll_gen;
        let weak: Weak<Inner> = Arc::downgrade(self);
        let net = Arc::clone(&self.net);
        st.poll = Some((
            period,
            rt.spawn(async move {
                loop {
                    tokio::time::sleep(period).await;
                    let now = change_fingerprint(&*net);
                    let Some(inner) = weak.upgrade() else { return };
                    let mut st = inner.lock();
                    if st.poll_gen != gen {
                        return; // replaced while this tick read the tables
                    }
                    if now.is_none() || st.poll_seen != Some(now) {
                        deliver(&st, POLL, |c| matches!(c, Consumer::Carrier { .. }));
                    }
                    st.poll_seen = Some(now);
                }
            }),
        ));
    }
}

/// Mark `bit` pending on every matching subscriber and wake it.
fn deliver(st: &State, bit: u8, to: impl Fn(&Consumer) -> bool) {
    for s in st.slots.iter().filter(|s| to(&s.consumer)) {
        s.pending.fetch_or(bit, Ordering::AcqRel);
        s.notify.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::transport::netlink::ScriptedEvents;
    use std::sync::atomic::AtomicUsize;

    /// A fake opener: hands out scripted sources (or fails), counting opens and live sources.
    struct Fake {
        fail: bool,
        opens: Arc<AtomicUsize>,
        live: Arc<AtomicUsize>,
        feed: Arc<Mutex<Vec<tokio::sync::mpsc::Sender<()>>>>,
        forced: Arc<std::sync::atomic::AtomicBool>,
    }

    struct Counted {
        inner: ScriptedEvents,
        live: Arc<AtomicUsize>,
        /// The test marks the burst in flight forced (`netlink::classify` said so).
        forced: Arc<std::sync::atomic::AtomicBool>,
    }

    impl LinkEvents for Counted {
        fn next_change(
            &mut self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<()>> + Send + '_>> {
            self.inner.next_change()
        }

        fn take_forced(&mut self) -> bool {
            self.forced.swap(false, Ordering::SeqCst)
        }
    }

    impl Drop for Counted {
        fn drop(&mut self) {
            self.live.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl Opener for Fake {
        fn open(&self, groups: u32) -> anyhow::Result<Box<dyn LinkEvents>> {
            assert_eq!(groups, 0x551, "§3.3: the second socket's mask");
            self.opens.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                anyhow::bail!("netlink bind (groups {groups:#x}): Permission denied");
            }
            let (tx, rx) = tokio::sync::mpsc::channel(64);
            self.feed.lock().unwrap().push(tx);
            self.live.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Counted {
                inner: ScriptedEvents { rx },
                live: Arc::clone(&self.live),
                forced: Arc::clone(&self.forced),
            }))
        }
    }

    struct Handles {
        opens: Arc<AtomicUsize>,
        live: Arc<AtomicUsize>,
        feed: Arc<Mutex<Vec<tokio::sync::mpsc::Sender<()>>>>,
        forced: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Handles {
        fn sender(&self) -> tokio::sync::mpsc::Sender<()> {
            self.feed.lock().unwrap().last().unwrap().clone()
        }
    }

    /// A host whose routes fingerprint the test sets (`None` = unreadable; route reads are
    /// counted) and whose interface rows it edits (`None` = getifaddrs failed).
    struct Routes {
        value: Mutex<Option<u64>>,
        reads: AtomicUsize,
        addrs: Mutex<Option<Vec<NetEntry>>>,
    }

    impl NetRead for Routes {
        fn routes(&self) -> Option<u64> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            *self.value.lock().unwrap()
        }

        fn addresses(&self) -> Option<u64> {
            let mut rows = self.addrs.lock().unwrap().clone()?;
            Some(address_fingerprint(&mut rows))
        }
    }

    fn addr(name: &str, index: u32, last: u8, prefix: u8) -> NetEntry {
        NetEntry::Addr {
            index,
            name: name.into(),
            family: libc::AF_INET as u16,
            addr: vec![192, 168, 1, last],
            prefix,
        }
    }

    fn link(name: &str, index: u32, flags: u32) -> NetEntry {
        NetEntry::Link {
            index,
            name: name.into(),
            flags,
        }
    }

    fn routes(v: Option<u64>) -> Arc<Routes> {
        Arc::new(Routes {
            value: Mutex::new(v),
            reads: AtomicUsize::new(0),
            addrs: Mutex::new(Some(vec![
                link("eth0", 2, LINK_FLAGS),
                addr("eth0", 2, 20, 24),
            ])),
        })
    }

    fn test_cfg(termux: bool) -> Config {
        Config {
            quiet: Duration::from_millis(50),
            max_wait: Duration::from_secs(10),
            poll_unit: Duration::from_millis(5),
            termux,
        }
    }

    fn watch(fail: bool, termux: bool, routes: Arc<Routes>) -> (NetWatch, Handles) {
        watch_cfg(fail, test_cfg(termux), routes)
    }

    fn watch_cfg(fail: bool, cfg: Config, routes: Arc<Routes>) -> (NetWatch, Handles) {
        watch_net(fail, cfg, routes)
    }

    fn watch_net(fail: bool, cfg: Config, net: Arc<dyn NetRead>) -> (NetWatch, Handles) {
        let h = Handles {
            opens: Arc::new(AtomicUsize::new(0)),
            live: Arc::new(AtomicUsize::new(0)),
            feed: Arc::new(Mutex::new(Vec::new())),
            forced: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let fake = Fake {
            fail,
            opens: Arc::clone(&h.opens),
            live: Arc::clone(&h.live),
            feed: Arc::clone(&h.feed),
            forced: Arc::clone(&h.forced),
        };
        (NetWatch::new(cfg, Box::new(fake), net), h)
    }

    async fn eventually(what: &str, f: impl Fn() -> bool) {
        for _ in 0..200 {
            if f() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for: {what}");
    }

    async fn quiet_for(sub: &Subscription, ms: u64) -> Option<NetEvent> {
        tokio::time::timeout(Duration::from_millis(ms), sub.next())
            .await
            .ok()
    }

    #[tokio::test]
    async fn nothing_opens_without_a_subscriber() {
        let (w, h) = watch(false, true, routes(Some(1)));
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(h.opens.load(Ordering::SeqCst), 0);
        assert!(!w.is_open());
        assert_eq!(w.polling(), None);
        assert_eq!(w.subscribers(), 0);
    }

    #[tokio::test]
    async fn the_first_subscriber_opens_and_the_last_closes() {
        let (w, h) = watch(false, false, routes(Some(1)));
        let carrier = w
            .subscribe(Consumer::Carrier {
                termux_poll_min: 15,
            })
            .unwrap();
        assert_eq!(h.opens.load(Ordering::SeqCst), 1);
        assert_eq!(h.live.load(Ordering::SeqCst), 1);
        assert!(w.is_open());
        assert_eq!(carrier.watching(), Watching::Netlink);
        // a second consumer shares the one socket
        let mdns = w.subscribe(Consumer::Mdns).unwrap();
        assert_eq!(
            h.opens.load(Ordering::SeqCst),
            1,
            "one second socket, shared"
        );
        assert_eq!(w.subscribers(), 2);
        drop(carrier);
        assert!(w.is_open(), "mdns-sd still holds it");
        assert_eq!(h.live.load(Ordering::SeqCst), 1);
        drop(mdns);
        assert!(!w.is_open());
        eventually("the socket closed", || h.live.load(Ordering::SeqCst) == 0).await;
        assert_eq!(w.polling(), None, "no timer without a consumer");
        // and a new first subscriber opens again
        let again = w.subscribe(Consumer::Mdns).unwrap();
        assert_eq!(h.opens.load(Ordering::SeqCst), 2);
        assert_eq!(h.live.load(Ordering::SeqCst), 1);
        drop(again);
        eventually("closed again", || h.live.load(Ordering::SeqCst) == 0).await;
    }

    #[tokio::test]
    async fn a_route_burst_is_one_event_to_every_subscriber() {
        let r = routes(Some(1));
        let (w, h) = watch(false, false, Arc::clone(&r));
        let carrier = w
            .subscribe(Consumer::Carrier {
                termux_poll_min: 15,
            })
            .unwrap();
        let mdns = w.subscribe(Consumer::Mdns).unwrap();
        let tx = h.sender();
        *r.value.lock().unwrap() = Some(2); // the default route moved
        for _ in 0..20 {
            tx.send(()).await.unwrap();
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert_eq!(quiet_for(&carrier, 1_000).await, Some(NetEvent::Changed));
        assert_eq!(quiet_for(&mdns, 1_000).await, Some(NetEvent::Changed));
        assert_eq!(quiet_for(&carrier, 200).await, None, "one event per burst");
        assert_eq!(quiet_for(&mdns, 200).await, None);
        // a later, separate change is a second event
        *r.value.lock().unwrap() = Some(3);
        tx.send(()).await.unwrap();
        assert_eq!(quiet_for(&carrier, 1_000).await, Some(NetEvent::Changed));
        assert_eq!(quiet_for(&mdns, 1_000).await, Some(NetEvent::Changed));
    }

    /// Row D4b: a datagram is a change only when what the consumers care about moved. Each
    /// step is one burst; the fingerprint is the real one over the fake host's rows.
    #[tokio::test]
    async fn only_a_real_address_or_route_change_is_delivered() {
        let r = routes(Some(1));
        let (w, h) = watch(false, false, Arc::clone(&r));
        let mdns = w.subscribe(Consumer::Mdns).unwrap();
        let tx = h.sender();
        let burst = |n: usize| {
            let tx = tx.clone();
            async move {
                for _ in 0..n {
                    tx.send(()).await.unwrap();
                }
            }
        };
        // a lifetime-only refresh (a DHCP renew, a Router Advertisement): RTM_NEWADDR for
        // the same rows delivers nothing
        burst(3).await;
        assert_eq!(quiet_for(&mdns, 300).await, None, "a refresh is no change");
        // an added address: one Changed
        r.addrs
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .push(addr("eth0", 2, 21, 24));
        burst(2).await;
        assert_eq!(quiet_for(&mdns, 1_000).await, Some(NetEvent::Changed));
        assert_eq!(quiet_for(&mdns, 200).await, None);
        // its refresh again: nothing
        burst(1).await;
        assert_eq!(quiet_for(&mdns, 300).await, None);
        // the address removed: one Changed
        r.addrs.lock().unwrap().as_mut().unwrap().pop();
        burst(2).await;
        assert_eq!(quiet_for(&mdns, 1_000).await, Some(NetEvent::Changed));
        assert_eq!(quiet_for(&mdns, 200).await, None);
        // a changed prefix length is a change too
        *r.addrs
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .last_mut()
            .unwrap() = addr("eth0", 2, 20, 16);
        burst(1).await;
        assert_eq!(quiet_for(&mdns, 1_000).await, Some(NetEvent::Changed));
        // the default route changed (the real route fingerprint over two tables)
        let v4 = |gw: &str| {
            format!(
                "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                 eth0\t00000000\t{gw}\t0003\t0\t0\t100\t00000000\t0\t0\t0\n"
            )
        };
        *r.value.lock().unwrap() = route_fingerprint(Some(&v4("0101A8C0")), None);
        burst(1).await;
        assert_eq!(quiet_for(&mdns, 1_000).await, Some(NetEvent::Changed));
        *r.value.lock().unwrap() = route_fingerprint(Some(&v4("FE01A8C0")), None);
        burst(2).await;
        assert_eq!(quiet_for(&mdns, 1_000).await, Some(NetEvent::Changed));
        assert_eq!(quiet_for(&mdns, 200).await, None);
        // the link lost its carrier (IFF_RUNNING gone): a change
        *r.addrs
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .first_mut()
            .unwrap() = link("eth0", 2, libc::IFF_UP as u32);
        burst(1).await;
        assert_eq!(quiet_for(&mdns, 1_000).await, Some(NetEvent::Changed));
        // unreadable addresses: every burst counts (as before D4b), twice in a row
        *r.addrs.lock().unwrap() = None;
        burst(1).await;
        assert_eq!(quiet_for(&mdns, 1_000).await, Some(NetEvent::Changed));
        burst(1).await;
        assert_eq!(quiet_for(&mdns, 1_000).await, Some(NetEvent::Changed));
    }

    /// Row D4b: a burst that never goes quiet for the debounce window is still checked within
    /// the bound, and delivers when the fingerprint moved.
    #[tokio::test]
    async fn a_burst_that_never_goes_quiet_delivers_within_the_bound() {
        let r = routes(Some(1));
        let cfg = Config {
            max_wait: Duration::from_millis(250),
            ..test_cfg(false)
        };
        let (w, h) = watch_cfg(false, cfg, Arc::clone(&r));
        let carrier = w
            .subscribe(Consumer::Carrier {
                termux_poll_min: 15,
            })
            .unwrap();
        let tx = h.sender();
        *r.value.lock().unwrap() = Some(2);
        // a datagram every 10 ms (never 50 ms of quiet) for 800 ms
        let feeder = tokio::spawn(async move {
            for _ in 0..80 {
                if tx.send(()).await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let start = tokio::time::Instant::now();
        assert_eq!(quiet_for(&carrier, 600).await, Some(NetEvent::Changed));
        assert!(start.elapsed() < Duration::from_millis(600));
        assert!(!feeder.is_finished(), "delivered while the burst went on");
        // the burst goes on with nothing moved: no second event
        assert_eq!(quiet_for(&carrier, 300).await, None);
        feeder.abort();
    }

    #[tokio::test]
    async fn a_failed_bind_is_logged_once_and_reported() {
        let (w, h) = watch(true, false, routes(Some(1)));
        let mdns = w.subscribe(Consumer::Mdns).unwrap();
        match mdns.watching() {
            Watching::Failed { error, poll } => {
                assert!(error.contains("Permission denied"), "{error}");
                assert_eq!(poll, None, "mdns-sd keeps its own 5 s check");
            }
            other => panic!("{other:?}"),
        }
        assert!(!w.is_open(), "no reader task without a socket");
        assert_eq!(w.polling(), None, "not Termux: nothing polls");
        let carrier = w
            .subscribe(Consumer::Carrier {
                termux_poll_min: 15,
            })
            .unwrap();
        assert!(matches!(
            carrier.watching(),
            Watching::Failed { poll: None, .. }
        ));
        assert_eq!(w.polling(), None, "a carrier off Termux never polls");
        drop((mdns, carrier));
        // the next first subscriber tries again; the failure is not logged twice
        let again = w.subscribe(Consumer::Mdns).unwrap();
        assert_eq!(h.opens.load(Ordering::SeqCst), 2);
        assert_eq!(w.failures_logged(), 1, "logged once");
        assert_eq!(quiet_for(&again, 100).await, None);
    }

    #[tokio::test]
    async fn a_stream_that_ends_is_reported_as_failed() {
        let (w, h) = watch(false, false, routes(Some(1)));
        let mdns = w.subscribe(Consumer::Mdns).unwrap();
        h.feed.lock().unwrap().clear(); // drop the only sender: the source ends
        assert_eq!(quiet_for(&mdns, 1_000).await, Some(NetEvent::Failed));
        assert!(matches!(mdns.watching(), Watching::Failed { .. }));
        assert!(!w.is_open());
        assert_eq!(w.failures_logged(), 1);
    }

    #[tokio::test]
    async fn the_termux_poll_runs_only_for_a_carrier_whose_bind_failed() {
        let r = routes(Some(1));
        let (w, _h) = watch(true, true, Arc::clone(&r));
        // mdns-sd alone: no poll
        let mdns = w.subscribe(Consumer::Mdns).unwrap();
        assert_eq!(w.polling(), None);
        // a carrier: the poll at gateway.termux_poll_min (clamped to ≥ 9)
        let carrier = w
            .subscribe(Consumer::Carrier { termux_poll_min: 3 })
            .unwrap();
        let period = Duration::from_millis(5) * 9;
        assert_eq!(w.polling(), Some(period));
        assert_eq!(
            carrier.watching(),
            Watching::Failed {
                error: "netlink bind (groups 0x551): Permission denied".into(),
                poll: Some(period),
            }
        );
        // unchanged tables: no event
        eventually("the poll read the tables", || {
            r.reads.load(Ordering::SeqCst) >= 3
        })
        .await;
        assert_eq!(quiet_for(&carrier, 20).await, None);
        // a changed table: one Poll to the carrier, nothing to mdns-sd
        *r.value.lock().unwrap() = Some(2);
        assert_eq!(quiet_for(&carrier, 1_000).await, Some(NetEvent::Poll));
        assert_eq!(quiet_for(&mdns, 100).await, None);
        // unreadable tables: every tick checks
        *r.value.lock().unwrap() = None;
        assert_eq!(quiet_for(&carrier, 1_000).await, Some(NetEvent::Poll));
        assert_eq!(quiet_for(&carrier, 1_000).await, Some(NetEvent::Poll));
        // the carrier leaves: the poll stops though mdns-sd stays
        drop(carrier);
        assert_eq!(w.polling(), None);
        let before = r.reads.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            r.reads.load(Ordering::SeqCst) <= before + 1,
            "no read after the stop"
        );
        drop(mdns);
    }

    #[tokio::test]
    async fn a_bound_socket_on_termux_never_polls() {
        let (w, _h) = watch(false, true, routes(Some(1)));
        let carrier = w
            .subscribe(Consumer::Carrier {
                termux_poll_min: 15,
            })
            .unwrap();
        assert_eq!(carrier.watching(), Watching::Netlink);
        assert_eq!(w.polling(), None);
    }

    #[tokio::test]
    async fn a_lost_stream_on_termux_starts_the_carrier_poll() {
        let r = routes(None);
        let (w, h) = watch(false, true, Arc::clone(&r));
        let carrier = w
            .subscribe(Consumer::Carrier { termux_poll_min: 9 })
            .unwrap();
        assert_eq!(w.polling(), None);
        h.feed.lock().unwrap().clear();
        assert_eq!(quiet_for(&carrier, 1_000).await, Some(NetEvent::Failed));
        assert!(w.polling().is_some());
        assert_eq!(quiet_for(&carrier, 1_000).await, Some(NetEvent::Poll));
    }

    #[test]
    fn subscribing_outside_a_runtime_is_refused() {
        let (w, h) = watch(false, false, routes(Some(1)));
        assert_eq!(
            w.subscribe(Consumer::Mdns).err(),
            Some(NetWatchError::NoRuntime)
        );
        assert_eq!(h.opens.load(Ordering::SeqCst), 0);
        assert_eq!(w.subscribers(), 0);
    }

    #[test]
    fn the_fingerprint_ignores_counters_and_headers() {
        let v4a =
            "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                   eth0\t00000000\t0102A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n";
        let v4b = v4a.replace("\t0003\t0\t0\t100", "\t0003\t7\t42\t100");
        let v4c = v4a.replace("0102A8C0", "0103A8C0");
        let v6a = "fe800000000000000000000000000000 40 00000000000000000000000000000000 00 \
                   00000000000000000000000000000000 00000100 00000001 00000000 00000001 eth0\n";
        let v6b = v6a.replace("00000001 00000000 00000001", "00000009 00000123 00000001");
        assert_eq!(
            route_fingerprint(Some(v4a), Some(v6a)),
            route_fingerprint(Some(&v4b), Some(&v6b)),
            "counters are ignored"
        );
        assert_ne!(
            route_fingerprint(Some(v4a), Some(v6a)),
            route_fingerprint(Some(&v4c), Some(v6a)),
            "a new gateway is a change"
        );
        assert_ne!(
            route_fingerprint(Some(v4a), None),
            route_fingerprint(Some(v4a), Some(v6a))
        );
        assert_eq!(route_fingerprint(None, None), None);
        assert!(route_fingerprint(None, Some(v6a)).is_some());
        // the real tables read (or not) without a panic
        let _ = HostNet.routes();
    }

    #[test]
    fn the_address_fingerprint_is_order_free_and_ignores_nothing_it_keeps() {
        let a = vec![link("eth0", 2, LINK_FLAGS), addr("eth0", 2, 20, 24)];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(
            address_fingerprint(&mut a.clone()),
            address_fingerprint(&mut b)
        );
        for other in [
            vec![link("eth0", 3, LINK_FLAGS), addr("eth0", 3, 20, 24)],
            vec![link("eth0", 2, LINK_FLAGS), addr("eth0", 2, 21, 24)],
            vec![link("eth0", 2, LINK_FLAGS), addr("eth0", 2, 20, 25)],
            vec![link("eth0", 2, 0), addr("eth0", 2, 20, 24)],
        ] {
            assert_ne!(
                address_fingerprint(&mut a.clone()),
                address_fingerprint(&mut other.clone()),
                "{other:?}"
            );
        }
    }

    #[test]
    fn this_host_s_interfaces_read_stably() {
        // getifaddrs on the build host: two reads with nothing changed agree (a sandbox where
        // it fails reads None, which the watch treats as "every burst counts")
        match (HostNet.addresses(), HostNet.addresses()) {
            (Some(x), Some(y)) => assert_eq!(x, y),
            (None, None) => eprintln!("getifaddrs unavailable here"),
            other => panic!("unstable read: {other:?}"),
        }
        if let Some(rows) = read_interfaces() {
            assert!(rows.len() <= IFADDRS_MAX);
            // loopback is always listed with its index and 127.0.0.1/8 when IPv4 is up
            let lo = rows.iter().find_map(|r| match r {
                NetEntry::Addr {
                    name,
                    addr,
                    prefix,
                    index,
                    ..
                } if name == "lo" && addr == &[127, 0, 0, 1] => Some((*prefix, *index)),
                _ => None,
            });
            if let Some((prefix, index)) = lo {
                assert_eq!(prefix, 8);
                assert!(index > 0);
            }
        }
    }

    /// Row D4b (1): a poll restarted for a new period keeps the last reading as its baseline,
    /// so a route change since the old task's last tick is still delivered, once.
    #[tokio::test]
    async fn a_poll_restart_keeps_the_last_fingerprint() {
        let r = routes(Some(1));
        let cfg = Config {
            poll_unit: Duration::from_millis(20),
            ..test_cfg(true)
        };
        let (w, _h) = watch_cfg(true, cfg, Arc::clone(&r));
        let slow = w
            .subscribe(Consumer::Carrier {
                termux_poll_min: 12,
            })
            .unwrap();
        assert_eq!(w.polling(), Some(Duration::from_millis(240)));
        // the routes change before the old task's first tick...
        *r.value.lock().unwrap() = Some(2);
        // ...and a second carrier with a shorter period restarts the poll
        let fast = w
            .subscribe(Consumer::Carrier { termux_poll_min: 9 })
            .unwrap();
        assert_eq!(w.polling(), Some(Duration::from_millis(180)));
        // the restarted task's first tick delivers one Poll to each carrier
        assert_eq!(quiet_for(&fast, 1_000).await, Some(NetEvent::Poll));
        assert_eq!(quiet_for(&slow, 100).await, Some(NetEvent::Poll));
        assert_eq!(quiet_for(&fast, 400).await, None, "once");
        // the slow carrier leaves: a restart again (back to 240 ms), nothing to deliver
        drop(fast);
        assert_eq!(w.polling(), Some(Duration::from_millis(240)));
        assert_eq!(quiet_for(&slow, 600).await, None);
        // the poll stops with the last carrier; a new one reads a fresh baseline
        drop(slow);
        assert_eq!(w.polling(), None);
        *r.value.lock().unwrap() = Some(3);
        let again = w
            .subscribe(Consumer::Carrier { termux_poll_min: 9 })
            .unwrap();
        assert_eq!(quiet_for(&again, 500).await, None, "no stale baseline");
    }

    #[test]
    fn the_poll_period_is_clamped_to_the_settings_bounds() {
        let m = Duration::from_secs(60);
        assert_eq!(poll_period(15, m), Duration::from_secs(900));
        assert_eq!(poll_period(0, m), Duration::from_secs(9 * 60));
        assert_eq!(poll_period(u32::MAX, m), Duration::from_secs(1440 * 60));
    }

    /// Row D4c (2): a burst whose datagrams carried a message that counts on its own (a policy
    /// table's route, a deleted address, a link message) is delivered though the fingerprint
    /// (main table, addresses) did not move; an unforced burst is still settled by it.
    #[tokio::test]
    async fn a_forced_burst_is_delivered_without_the_fingerprint() {
        let r = routes(Some(1));
        let (w, h) = watch(false, false, Arc::clone(&r));
        let mdns = w.subscribe(Consumer::Mdns).unwrap();
        let tx = h.sender();
        tx.send(()).await.unwrap();
        assert_eq!(quiet_for(&mdns, 300).await, None, "unforced, unchanged");
        // `ip route add default ... table 52`: the fingerprint reads the same
        h.forced.store(true, Ordering::SeqCst);
        tx.send(()).await.unwrap();
        tx.send(()).await.unwrap();
        assert_eq!(quiet_for(&mdns, 1_000).await, Some(NetEvent::Changed));
        assert_eq!(quiet_for(&mdns, 200).await, None, "one event per burst");
        // the mark is the burst's: the next unforced one is settled by the fingerprint
        tx.send(()).await.unwrap();
        assert_eq!(quiet_for(&mdns, 300).await, None);
        // a forced burst still leaves the fingerprint as the next baseline
        *r.value.lock().unwrap() = Some(2);
        h.forced.store(true, Ordering::SeqCst);
        tx.send(()).await.unwrap();
        assert_eq!(quiet_for(&mdns, 1_000).await, Some(NetEvent::Changed));
        tx.send(()).await.unwrap();
        assert_eq!(quiet_for(&mdns, 300).await, None, "baseline taken");
    }

    /// Row D4c (1): the Termux poll compares the full fingerprint, so a network switch that
    /// moves the addresses but not the main routing table (Android) is delivered.
    #[tokio::test]
    async fn the_termux_poll_sees_an_address_change_with_the_routes_unchanged() {
        let r = routes(Some(1));
        let (w, _h) = watch(true, true, Arc::clone(&r));
        let carrier = w
            .subscribe(Consumer::Carrier { termux_poll_min: 9 })
            .unwrap();
        eventually("two ticks", || r.reads.load(Ordering::SeqCst) >= 3).await;
        assert_eq!(quiet_for(&carrier, 20).await, None);
        // Wi-Fi to cellular: the old address gone, a new one on another interface
        *r.addrs.lock().unwrap() = Some(vec![
            link("eth0", 2, libc::IFF_UP as u32),
            link("rmnet0", 5, LINK_FLAGS),
            addr("rmnet0", 5, 77, 30),
        ]);
        assert_eq!(quiet_for(&carrier, 1_000).await, Some(NetEvent::Poll));
        assert_eq!(quiet_for(&carrier, 200).await, None, "once");
        // the interfaces unreadable: every tick (the check is the only way left)
        *r.addrs.lock().unwrap() = None;
        assert_eq!(quiet_for(&carrier, 1_000).await, Some(NetEvent::Poll));
        assert_eq!(quiet_for(&carrier, 1_000).await, Some(NetEvent::Poll));
    }

    /// A host whose next armed read stops the poll from inside that read: it drops the
    /// subscription it holds, as a carrier leaving while a tick is past its sleep would (the
    /// tick reads without the watch's lock, so the stop runs in between). Nothing blocks: a
    /// test that parks a runtime worker in a read starves the timer driver and never returns
    /// (the cause of the hang this test replaced).
    struct Gate {
        armed: std::sync::atomic::AtomicBool,
        stopped: std::sync::atomic::AtomicBool,
        stop: Mutex<Option<Subscription>>,
        value: std::sync::atomic::AtomicU64,
    }

    impl NetRead for Gate {
        fn routes(&self) -> Option<u64> {
            if self.armed.swap(false, Ordering::SeqCst) {
                drop(self.stop.lock().unwrap().take());
                // the network moved meanwhile: a write-back would differ from the baseline
                self.value.store(2, Ordering::SeqCst);
                self.stopped.store(true, Ordering::SeqCst);
            }
            Some(self.value.load(Ordering::SeqCst))
        }

        fn addresses(&self) -> Option<u64> {
            Some(7)
        }
    }

    /// Row D4c (3): a poll stopped (the carrier leaves while mdns-sd stays) or closed (the
    /// last subscriber leaves) while one of its ticks is inside its read: that tick writes
    /// nothing back after the stop, so the next poll reads a fresh baseline (fails without the
    /// fix: the stop cleared `poll_seen` and the stale tick, its generation unchanged, wrote
    /// its reading back).
    #[tokio::test]
    async fn a_stopped_poll_s_tick_writes_nothing_back() {
        for close in [false, true] {
            let gate = Arc::new(Gate {
                armed: false.into(),
                stopped: false.into(),
                stop: Mutex::new(None),
                value: 1.into(),
            });
            let (w, _h) = watch_net(true, test_cfg(true), Arc::clone(&gate) as Arc<dyn NetRead>);
            let mdns = (!close).then(|| w.subscribe(Consumer::Mdns).unwrap());
            let carrier = w
                .subscribe(Consumer::Carrier { termux_poll_min: 9 })
                .unwrap();
            assert!(w.polling().is_some());
            assert!(w.inner.lock().poll_seen.is_some(), "the baseline");
            // the test, the watch and the poll task hold the host
            assert_eq!(Arc::strong_count(&gate), 3);
            *gate.stop.lock().unwrap() = Some(carrier);
            gate.armed.store(true, Ordering::SeqCst);
            eventually("a tick stopped inside its read", || {
                gate.stopped.load(Ordering::SeqCst)
            })
            .await;
            assert_eq!(w.polling(), None);
            // the aborted task ends at its next await (after this tick, written back or not)
            eventually("the stopped task ended", || Arc::strong_count(&gate) == 2).await;
            assert_eq!(
                w.inner.lock().poll_seen,
                None,
                "close = {close}: the stale tick wrote its reading back"
            );
            // a new poll takes a fresh baseline (the network as it is now)
            let again = w
                .subscribe(Consumer::Carrier { termux_poll_min: 9 })
                .unwrap();
            assert_eq!(
                w.inner.lock().poll_seen,
                Some(change_fingerprint(&*gate)),
                "close = {close}"
            );
            drop(again);
            drop(mdns);
        }
    }

    #[test]
    fn the_mask_is_the_spec_s() {
        assert_eq!(GROUPS, 0x1 | 0x10 | 0x40 | 0x100 | 0x400);
        assert_eq!(DEBOUNCE, Duration::from_secs(2));
    }
}
