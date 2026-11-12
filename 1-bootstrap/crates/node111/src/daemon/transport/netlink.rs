//! Interface-change watch with debounce — rnsd config regeneration trigger
//! (spec §Transport) and, through a second instance, the Layer 7 address and
//! route watch (`os/7-relay/README.md` §3.3; `daemon::relay::netwatch`). The
//! event source is a trait so tests inject synthetic changes.
//!
//! The production source is a raw netlink socket via libc (no crate-API
//! surface to drift): bind AF_NETLINK/NETLINK_ROUTE with the multicast groups
//! the caller names ([`NetlinkEvents::new`] takes the mask, §3.3), tokio
//! AsyncFd for readiness. The spine passes [`RTMGRP_LINK`] only
//! (`transport::LINK_WATCH_GROUPS`), so an address or route change (a DHCP
//! renew) never reaches its `debounced` instance (S15; §19 D8). rtnetlink
//! remains in the dep set for Layer 6's richer routing work.
//!
//! **What a burst carried (Layer 7 §3.3, row D4c).** Each datagram's netlink
//! message headers are read ([`classify`]: every `nlmsghdr` length and type,
//! sizes checked before each read, never a panic). A message that counts as a
//! change on its own marks the burst *forced* ([`LinkEvents::take_forced`]):
//! a route message (`RTM_NEWROUTE`, `RTM_DELROUTE`) of a table that is neither
//! main (254) nor local (255) (a VPN's or Tailscale's policy table, Android's
//! per-network tables: the address watch's fingerprint reads the main table
//! only), a deleted address (`RTM_DELADDR`), any link message (`RTM_NEWLINK`,
//! `RTM_DELLINK`), and a short, truncated, unknown or unparsable message or a
//! failed read (messages were lost). A burst of `RTM_NEWADDR` and main / local
//! table route messages only is left to the caller's fingerprint (so a
//! lifetime-only address refresh still delivers nothing). The trade-off that
//! remains: a change made and reverted inside one burst whose messages are
//! only of that second kind is not seen (the fingerprint reads the same
//! before and after).

/// One "something changed about interfaces" event (details don't matter — the
/// reaction is always: debounce, maybe regenerate).
pub trait LinkEvents: Send {
    fn next_change(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<()>> + Send + '_>>;

    /// Whether the events since the last call carried a message that counts as a change
    /// without asking any fingerprint ([`classify`]; Layer 7 row D4c), and reset it. A source
    /// that cannot tell answers `false` (its caller's fingerprint decides).
    fn take_forced(&mut self) -> bool {
        false
    }
}

/// A boxed source is a source (`relay::netwatch` holds its opener's result as one).
impl<T: LinkEvents + ?Sized> LinkEvents for Box<T> {
    fn next_change(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<()>> + Send + '_>> {
        (**self).next_change()
    }

    fn take_forced(&mut self) -> bool {
        (**self).take_forced()
    }
}

/// `RTMGRP_LINK` multicast group bit (`linux/rtnetlink.h`; `LC/:2321`).
pub const RTMGRP_LINK: u32 = 0x1;
/// `RTMGRP_IPV4_IFADDR` (`LC/:2325`): IPv4 address added or removed.
pub const RTMGRP_IPV4_IFADDR: u32 = 0x10;
/// `RTMGRP_IPV4_ROUTE` (`LC/:2327`): IPv4 route added or removed.
pub const RTMGRP_IPV4_ROUTE: u32 = 0x40;
/// `RTMGRP_IPV6_IFADDR` (`LC/:2329`): IPv6 address added or removed.
pub const RTMGRP_IPV6_IFADDR: u32 = 0x100;
/// `RTMGRP_IPV6_ROUTE` (`LC/:2331`): IPv6 route added or removed.
pub const RTMGRP_IPV6_ROUTE: u32 = 0x400;
/// The Layer 7 address and route mask (§3.3): `0x1 | 0x10 | 0x40 | 0x100 | 0x400`.
/// Only `relay::netwatch`'s second socket binds it; the spine never does.
pub const ADDRESS_ROUTE_GROUPS: u32 =
    RTMGRP_LINK | RTMGRP_IPV4_IFADDR | RTMGRP_IPV4_ROUTE | RTMGRP_IPV6_IFADDR | RTMGRP_IPV6_ROUTE;

/// `NLMSG_HDRLEN`: a `struct nlmsghdr` (len u32, type u16, flags u16, seq u32, pid u32).
pub const NLMSG_HDRLEN: usize = 16;
/// `sizeof(struct rtmsg)`: family, dst_len, src_len, tos, table, protocol, scope, type (u8
/// each), flags (u32).
pub const RTMSG_LEN: usize = 12;
/// `rtm_table`'s offset inside a `struct rtmsg`.
const RTM_TABLE_AT: usize = 4;
/// `RTM_NEWLINK` (`linux/rtnetlink.h`).
pub const RTM_NEWLINK: u16 = 16;
/// `RTM_DELLINK`.
pub const RTM_DELLINK: u16 = 17;
/// `RTM_NEWADDR`.
pub const RTM_NEWADDR: u16 = 20;
/// `RTM_DELADDR`.
pub const RTM_DELADDR: u16 = 21;
/// `RTM_NEWROUTE`.
pub const RTM_NEWROUTE: u16 = 24;
/// `RTM_DELROUTE`.
pub const RTM_DELROUTE: u16 = 25;
/// `RT_TABLE_MAIN`.
pub const RT_TABLE_MAIN: u8 = 254;
/// `RT_TABLE_LOCAL`.
pub const RT_TABLE_LOCAL: u8 = 255;
/// The receive buffer: one page-sized datagram holds a burst's worth of messages; a larger
/// one is truncated and counts as a change ([`classify`]).
pub const RECV_MAX: usize = 8192;

/// Does this datagram carry a message that counts as a change on its own (Layer 7 row D4c; the
/// module doc)? Each `nlmsghdr` is read with its size checked first: `true` for a link message
/// (`RTM_NEWLINK`, `RTM_DELLINK`), a deleted address, a route message whose `rtm_table` is
/// neither main nor local, and for anything short, unknown or unparsable (an empty datagram,
/// a length below the header or past the datagram's end, a route message shorter than its
/// `rtmsg`). `false` only when every message is an `RTM_NEWADDR` or a main / local table route
/// message. Never panics.
pub fn classify(datagram: &[u8]) -> bool {
    if datagram.is_empty() {
        return true;
    }
    let mut rest = datagram;
    while !rest.is_empty() {
        let (Some(len), Some(kind)) = (rest.get(0..4), rest.get(4..6)) else {
            return true;
        };
        let len = u32::from_ne_bytes([len[0], len[1], len[2], len[3]]) as usize;
        let kind = u16::from_ne_bytes([kind[0], kind[1]]);
        if len < NLMSG_HDRLEN || len > rest.len() {
            return true;
        }
        let msg = &rest[..len];
        match kind {
            RTM_NEWADDR => {}
            RTM_NEWROUTE | RTM_DELROUTE => match msg.get(NLMSG_HDRLEN + RTM_TABLE_AT) {
                Some(&t) if len >= NLMSG_HDRLEN + RTMSG_LEN => {
                    if t != RT_TABLE_MAIN && t != RT_TABLE_LOCAL {
                        return true;
                    }
                }
                _ => return true,
            },
            // RTM_NEWLINK, RTM_DELLINK, RTM_DELADDR, and every other type
            _ => return true,
        }
        // NLMSG_ALIGN: the next header starts on a 4-byte boundary
        let next = len.checked_add(3).map(|n| n & !3).unwrap_or(usize::MAX);
        rest = rest.get(next..).unwrap_or(&[]);
    }
    false
}

/// Production source: raw netlink socket, readiness via tokio AsyncFd.
pub struct NetlinkEvents {
    fd: tokio::io::unix::AsyncFd<std::fs::File>,
    /// A datagram since the last [`LinkEvents::take_forced`] counted as a change on its own.
    forced: bool,
}

impl NetlinkEvents {
    /// Open and bind a route-netlink socket subscribed to `groups` (a mask of the
    /// `RTMGRP_*` bits above; §3.3: the caller names the mask). Close-on-exec and
    /// non-blocking; the error names the mask so a failed bind is attributable.
    pub fn new(groups: u32) -> anyhow::Result<Self> {
        use std::os::unix::io::FromRawFd;
        let fd = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_DGRAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                libc::NETLINK_ROUTE,
            )
        };
        anyhow::ensure!(
            fd >= 0,
            "netlink socket: {}",
            std::io::Error::last_os_error()
        );
        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as u16;
        addr.nl_groups = groups;
        let rc = unsafe {
            libc::bind(
                fd,
                &addr as *const libc::sockaddr_nl as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_nl>() as u32,
            )
        };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            anyhow::bail!("netlink bind (groups {groups:#x}): {err}");
        }
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        Ok(NetlinkEvents {
            fd: tokio::io::unix::AsyncFd::new(file)?,
            forced: false,
        })
    }

    /// The multicast groups the kernel holds for this socket (`getsockname`'s
    /// `nl_groups`): the proof of the bound mask (the spine's is `RTMGRP_LINK` only).
    pub fn groups(&self) -> std::io::Result<u32> {
        use std::os::unix::io::AsRawFd;
        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockname(
                self.fd.get_ref().as_raw_fd(),
                &mut addr as *mut libc::sockaddr_nl as *mut libc::sockaddr,
                &mut len,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(addr.nl_groups)
    }

    /// Receive one datagram into a stack buffer and hand `see` its full length and the bytes
    /// kept (`None` when the socket fails). `MSG_TRUNC` makes the kernel report a datagram's
    /// full length, so a truncated one is known as such (the length is then larger than
    /// [`RECV_MAX`], the bytes kept).
    async fn recv<R>(
        &mut self,
        mut see: impl FnMut(usize, &[u8]) -> R,
    ) -> Option<std::io::Result<R>> {
        loop {
            let mut guard = self.fd.readable().await.ok()?;
            let result = guard.try_io(|inner| {
                use std::os::unix::io::AsRawFd;
                let mut buf = [0u8; RECV_MAX];
                // SAFETY: `buf` is a live, writable buffer of `buf.len()` bytes.
                let rc = unsafe {
                    libc::recv(
                        inner.get_ref().as_raw_fd(),
                        buf.as_mut_ptr() as *mut libc::c_void,
                        buf.len(),
                        libc::MSG_TRUNC,
                    )
                };
                if rc < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    let n = rc as usize;
                    Ok(see(n, &buf[..n.min(RECV_MAX)]))
                }
            });
            match result {
                Ok(r) => return Some(r),
                Err(_would_block) => continue,
            }
        }
    }

    /// Tests (the namespace check, row D4c): the next datagram's bytes.
    #[cfg(test)]
    async fn next_datagram(&mut self) -> Option<Vec<u8>> {
        self.recv(|_, b| b.to_vec()).await?.ok()
    }
}

impl LinkEvents for NetlinkEvents {
    fn next_change(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<()>> + Send + '_>> {
        Box::pin(async move {
            // truncated (the kernel reported more than was kept): unparsable
            // a failed read (ENOBUFS: the kernel dropped messages): a change
            let forced = self
                .recv(|n, b| n > RECV_MAX || classify(b))
                .await?
                .unwrap_or(true);
            self.forced |= forced;
            Some(())
        })
    }

    fn take_forced(&mut self) -> bool {
        std::mem::take(&mut self.forced)
    }
}

/// Scripted source for tests: pops events from a channel.
pub struct ScriptedEvents {
    pub rx: tokio::sync::mpsc::Receiver<()>,
}

impl LinkEvents for ScriptedEvents {
    fn next_change(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<()>> + Send + '_>> {
        Box::pin(async move { self.rx.recv().await })
    }
}

/// Scripted source for tests that also scripts [`LinkEvents::take_forced`]: each `true` sent
/// is an event whose datagram counted as a change on its own.
pub struct ScriptedBursts {
    pub rx: tokio::sync::mpsc::Receiver<bool>,
    forced: bool,
}

impl ScriptedBursts {
    pub fn new(rx: tokio::sync::mpsc::Receiver<bool>) -> Self {
        ScriptedBursts { rx, forced: false }
    }
}

impl LinkEvents for ScriptedBursts {
    fn next_change(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<()>> + Send + '_>> {
        Box::pin(async move {
            let forced = self.rx.recv().await?;
            self.forced |= forced;
            Some(())
        })
    }

    fn take_forced(&mut self) -> bool {
        std::mem::take(&mut self.forced)
    }
}

/// Debounce loop: collapses bursts into single callbacks after `quiet` of silence.
pub async fn debounced<S: LinkEvents, F: FnMut() + Send>(
    mut source: S,
    quiet: std::time::Duration,
    mut fire: F,
) {
    while source.next_change().await.is_some() {
        loop {
            match tokio::time::timeout(quiet, source.next_change()).await {
                Ok(Some(())) => continue, // burst continues
                Ok(None) => return,       // source closed
                Err(_) => break,          // quiet elapsed → fire once
            }
        }
        fire();
    }
}

/// [`debounced`] with a bound (Layer 7 §3.3; `relay::netwatch`): a burst
/// fires after `quiet` of silence, or `max` after its first event when it
/// never goes quiet, so a source that never pauses for `quiet` still fires
/// at most once per `max` and never starves its consumer. The deadline is a
/// monotonic `tokio::time::Instant`; no timer exists between bursts. The
/// spine keeps the plain [`debounced`] (its behaviour is Layer 6's).
///
/// `fire` receives the burst's [`LinkEvents::take_forced`] (row D4c): `true`
/// when a message in it counts as a change without any fingerprint.
pub async fn debounced_within<S: LinkEvents, F: FnMut(bool) + Send>(
    mut source: S,
    quiet: std::time::Duration,
    max: std::time::Duration,
    mut fire: F,
) {
    while source.next_change().await.is_some() {
        let deadline = tokio::time::Instant::now() + max;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                break; // the bound: fire although the burst goes on
            }
            match tokio::time::timeout(quiet.min(left), source.next_change()).await {
                Ok(Some(())) => continue, // burst continues
                Ok(None) => return,       // source closed
                Err(_) => break,          // quiet elapsed (or the bound) → fire once
            }
        }
        fire(source.take_forced());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn debounce_collapses_bursts() {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let source = ScriptedEvents { rx };
        let fires = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let fires2 = std::sync::Arc::clone(&fires);
        let task = tokio::spawn(debounced(
            source,
            std::time::Duration::from_millis(50),
            move || {
                fires2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
        ));
        // burst of 5 within the quiet window → one fire
        for _ in 0..5 {
            tx.send(()).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert_eq!(fires.load(std::sync::atomic::Ordering::SeqCst), 1);
        // a second isolated event → second fire
        tx.send(()).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert_eq!(fires.load(std::sync::atomic::Ordering::SeqCst), 2);
        drop(tx);
        let _ = task.await;
    }

    /// Row D4c (5): asserts only what load cannot break. The source never pauses (each event is
    /// ready at once, after a yield), the quiet window is an hour, so only the bound can fire:
    /// it fires at all, and every fire is at least `max` after the previous one (each burst's
    /// deadline is its first event plus `max`; a slow box only lengthens the gaps).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_burst_that_never_goes_quiet_still_fires_within_the_bound() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, Instant};
        struct Endless(Arc<AtomicBool>);
        impl LinkEvents for Endless {
            fn next_change(
                &mut self,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<()>> + Send + '_>>
            {
                Box::pin(async move {
                    tokio::task::yield_now().await;
                    (!self.0.load(Ordering::SeqCst)).then_some(())
                })
            }
        }
        let max = Duration::from_millis(40);
        let stop = Arc::new(AtomicBool::new(false));
        let fires: Arc<Mutex<Vec<Instant>>> = Arc::default();
        let f2 = Arc::clone(&fires);
        let started = Instant::now();
        let task = tokio::spawn(debounced_within(
            Endless(Arc::clone(&stop)),
            Duration::from_secs(3_600),
            max,
            move |_| f2.lock().unwrap().push(Instant::now()),
        ));
        let deadline = Instant::now() + Duration::from_secs(30);
        while fires.lock().unwrap().len() < 3 {
            assert!(Instant::now() < deadline, "the bound never fired");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        stop.store(true, Ordering::SeqCst);
        task.await.unwrap();
        let elapsed = started.elapsed();
        let at = fires.lock().unwrap().clone();
        assert!(at[0] >= started + max, "not before the bound");
        for w in at.windows(2) {
            assert!(w[1] >= w[0] + max, "at most one fire per bound");
        }
        let most = (elapsed.as_millis() / max.as_millis()) as usize + 1;
        assert!(at.len() <= most, "{} fires in {elapsed:?}", at.len());
    }

    /// Row D4c (2): a burst's forced mark reaches `fire`, once, and is reset for the next. Each
    /// burst is queued whole before the debounce reads it (a ready event never times out), so
    /// load cannot split it.
    #[tokio::test]
    async fn the_burst_s_forced_mark_reaches_fire_once() {
        use std::time::Duration;
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s2 = std::sync::Arc::clone(&seen);
        let task = tokio::spawn(debounced_within(
            ScriptedBursts::new(rx),
            Duration::from_millis(30),
            Duration::from_secs(10),
            move |forced| s2.lock().unwrap().push(forced),
        ));
        for (i, burst) in [&[false, false][..], &[false, true, false], &[false]]
            .into_iter()
            .enumerate()
        {
            for &f in burst {
                tx.send(f).await.unwrap();
            }
            let until = std::time::Instant::now() + Duration::from_secs(30);
            while seen.lock().unwrap().len() <= i {
                assert!(std::time::Instant::now() < until, "burst {i} never fired");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
        assert_eq!(*seen.lock().unwrap(), [false, true, false]);
        drop(tx);
        let _ = task.await;
    }

    /// A netlink message: header (`len`, `kind`) and `body`, padded to 4 bytes.
    fn msg(kind: u16, body: &[u8]) -> Vec<u8> {
        let len = (NLMSG_HDRLEN + body.len()) as u32;
        let mut m = Vec::new();
        m.extend_from_slice(&len.to_ne_bytes());
        m.extend_from_slice(&kind.to_ne_bytes());
        m.extend_from_slice(&[0u8; 10]); // flags, seq, pid
        m.extend_from_slice(body);
        while m.len() % 4 != 0 {
            m.push(0);
        }
        m
    }

    /// A route message of `table` (an `rtmsg` and one attribute's worth of bytes).
    fn route(kind: u16, table: u8) -> Vec<u8> {
        let mut rtm = [0u8; RTMSG_LEN + 8];
        rtm[0] = libc::AF_INET as u8;
        rtm[RTM_TABLE_AT] = table;
        msg(kind, &rtm)
    }

    #[test]
    fn the_message_types_are_the_kernel_s() {
        assert_eq!(RTM_NEWLINK, libc::RTM_NEWLINK);
        assert_eq!(RTM_DELLINK, libc::RTM_DELLINK);
        assert_eq!(RTM_NEWADDR, libc::RTM_NEWADDR);
        assert_eq!(RTM_DELADDR, libc::RTM_DELADDR);
        assert_eq!(RTM_NEWROUTE, libc::RTM_NEWROUTE);
        assert_eq!(RTM_DELROUTE, libc::RTM_DELROUTE);
        assert_eq!(RT_TABLE_MAIN, libc::RT_TABLE_MAIN);
        assert_eq!(RT_TABLE_LOCAL, libc::RT_TABLE_LOCAL);
        assert_eq!(NLMSG_HDRLEN, std::mem::size_of::<libc::nlmsghdr>());
    }

    /// Row D4c (2): which datagrams count as a change on their own.
    #[test]
    fn a_datagram_is_classified_by_its_message_headers() {
        let addr = msg(RTM_NEWADDR, &[0u8; 8]);
        // settled by the fingerprint: new addresses, main / local table routes
        assert!(!classify(&addr));
        assert!(!classify(&route(RTM_NEWROUTE, RT_TABLE_MAIN)));
        assert!(!classify(&route(RTM_DELROUTE, RT_TABLE_LOCAL)));
        assert!(!classify(
            &[
                addr.clone(),
                route(RTM_NEWROUTE, RT_TABLE_MAIN),
                addr.clone()
            ]
            .concat()
        ));
        // a change on its own
        for (what, d) in [
            ("policy table add", route(RTM_NEWROUTE, 52)),
            ("policy table del", route(RTM_DELROUTE, 52)),
            ("compat table (id > 255)", route(RTM_NEWROUTE, 252)),
            ("unspec table", route(RTM_NEWROUTE, 0)),
            ("address deleted", msg(RTM_DELADDR, &[0u8; 8])),
            ("link new", msg(RTM_NEWLINK, &[0u8; 16])),
            ("link del", msg(RTM_DELLINK, &[0u8; 16])),
            ("unknown type", msg(3, &[0u8; 4])),
            (
                "a policy route after settled ones",
                [addr.clone(), route(RTM_NEWROUTE, 52)].concat(),
            ),
        ] {
            assert!(classify(&d), "{what}");
        }
        // short or unparsable: a change, never a panic
        let mut long = msg(RTM_NEWADDR, &[0u8; 8]);
        long[0..4].copy_from_slice(&1000u32.to_ne_bytes());
        let mut tiny = msg(RTM_NEWADDR, &[0u8; 8]);
        tiny[0..4].copy_from_slice(&3u32.to_ne_bytes());
        let short_route = msg(RTM_NEWROUTE, &[0u8; 4]);
        for (what, d) in [
            ("empty", Vec::new()),
            ("half a header", addr[..7].to_vec()),
            ("length past the end", long),
            ("length below the header", tiny),
            ("route shorter than its rtmsg", short_route),
            (
                "a trailing fragment",
                [addr.clone(), vec![1, 2, 3]].concat(),
            ),
        ] {
            assert!(classify(&d), "{what}");
        }
        // a length that is not a multiple of 4 is padded to the next message
        let mut odd = msg(RTM_NEWADDR, &[0u8; 5]);
        assert_eq!(odd.len(), 24);
        odd.extend(route(RTM_NEWROUTE, RT_TABLE_MAIN));
        assert!(!classify(&odd));
        // arbitrary bytes never panic
        let mut x: u32 = 0x9e37_79b9;
        for n in 0..2_000usize {
            let bytes: Vec<u8> = (0..(n % 97))
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    x as u8
                })
                .collect();
            let _ = classify(&bytes);
        }
    }

    /// Row D4c (2), the namespace check: run by hand inside a private network namespace
    /// (`unshare -Urn <test binary> --ignored --nocapture`); prints what the kernel sent for
    /// each step and asserts the classification. Skips outside such a namespace.
    #[tokio::test]
    #[ignore = "needs a private network namespace: unshare -Urn"]
    async fn the_kernel_s_messages_classify_as_documented() {
        use std::time::Duration;
        if std::env::var_os("NODE111_NETNS_CHECK").is_none() {
            eprintln!("set NODE111_NETNS_CHECK=1 inside `unshare -Urn` to run");
            return;
        }
        let mut ev = NetlinkEvents::new(ADDRESS_ROUTE_GROUPS).expect("bind");
        let ip = |args: &str| {
            let st = std::process::Command::new("ip")
                .args(args.split_whitespace())
                .status()
                .expect("ip");
            assert!(st.success(), "ip {args}");
        };
        async fn drain(ev: &mut NetlinkEvents, what: &str) -> bool {
            let mut forced = false;
            let mut n = 0;
            while let Ok(Some(d)) =
                tokio::time::timeout(Duration::from_millis(300), ev.next_datagram()).await
            {
                n += 1;
                let mut rest = &d[..];
                while rest.len() >= NLMSG_HDRLEN {
                    let len = u32::from_ne_bytes(rest[0..4].try_into().unwrap()) as usize;
                    let kind = u16::from_ne_bytes(rest[4..6].try_into().unwrap());
                    let table = rest.get(NLMSG_HDRLEN + RTM_TABLE_AT).copied();
                    println!("  {what}: type {kind} len {len} table-byte {table:?}");
                    if len < NLMSG_HDRLEN {
                        break;
                    }
                    rest = rest.get((len + 3) & !3..).unwrap_or(&[]);
                }
                forced |= classify(&d);
            }
            println!("{what}: {n} datagram(s), forced = {forced}");
            forced
        }
        ip("link set lo up");
        drain(&mut ev, "lo up").await;
        ip("link add d0 type dummy");
        ip("link set d0 up");
        drain(&mut ev, "dummy up").await;
        ip("addr add 10.9.0.2/24 dev d0");
        assert!(!drain(&mut ev, "address added (main/local routes)").await);
        ip("addr change 10.9.0.2/24 dev d0 valid_lft 300 preferred_lft 200");
        assert!(!drain(&mut ev, "lifetime-only refresh").await);
        ip("route add default via 10.9.0.1 dev d0 table 52");
        assert!(drain(&mut ev, "policy table 52 default route added").await);
        ip("route del default via 10.9.0.1 dev d0 table 52");
        assert!(drain(&mut ev, "policy table 52 default route deleted").await);
        ip("route add 10.20.0.0/16 via 10.9.0.1 dev d0");
        assert!(!drain(&mut ev, "main table route added").await);
        ip("link set d0 down");
        assert!(drain(&mut ev, "link down").await);
        ip("link set d0 up");
        assert!(drain(&mut ev, "link up").await);
        ip("addr del 10.9.0.2/24 dev d0");
        assert!(drain(&mut ev, "address deleted").await);
    }

    #[tokio::test]
    async fn the_bounded_debounce_still_collapses_a_short_burst() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::time::Duration;
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let fires = std::sync::Arc::new(AtomicU32::new(0));
        let f2 = std::sync::Arc::clone(&fires);
        let task = tokio::spawn(debounced_within(
            ScriptedEvents { rx },
            Duration::from_millis(50),
            Duration::from_secs(10),
            move |_| {
                f2.fetch_add(1, Ordering::SeqCst);
            },
        ));
        for _ in 0..5 {
            tx.send(()).await.unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(fires.load(Ordering::SeqCst), 1);
        drop(tx);
        let _ = task.await;
        assert_eq!(
            fires.load(Ordering::SeqCst),
            1,
            "a closed source fires nothing more"
        );
    }

    #[tokio::test]
    async fn netlink_socket_binds() {
        // Real socket smoke test (skipped in restricted sandboxes).
        match NetlinkEvents::new(RTMGRP_LINK) {
            Ok(_) => {}
            Err(e) => eprintln!("netlink unavailable here ({e:#}) — acceptable in sandboxes"),
        }
    }

    #[test]
    fn the_group_bits_are_the_kernel_s() {
        // §3.3: `0x1 | 0x10 | 0x40 | 0x100 | 0x400` (LC/:2321-2331).
        assert_eq!(RTMGRP_LINK, libc::RTMGRP_LINK as u32);
        assert_eq!(RTMGRP_IPV4_IFADDR, libc::RTMGRP_IPV4_IFADDR as u32);
        assert_eq!(RTMGRP_IPV4_ROUTE, libc::RTMGRP_IPV4_ROUTE as u32);
        assert_eq!(RTMGRP_IPV6_IFADDR, libc::RTMGRP_IPV6_IFADDR as u32);
        assert_eq!(RTMGRP_IPV6_ROUTE, libc::RTMGRP_IPV6_ROUTE as u32);
        assert_eq!(ADDRESS_ROUTE_GROUPS, 0x551);
        // the spine's mask carries no address or route bit (S15)
        assert_eq!(
            crate::daemon::transport::LINK_WATCH_GROUPS & !RTMGRP_LINK,
            0,
            "the spine subscribes RTMGRP_LINK only"
        );
    }

    #[tokio::test]
    async fn the_socket_holds_the_mask_it_was_given() {
        // Each mask the kernel accepts reads back exactly; a sandbox that refuses the
        // socket skips (the bind failure path is relay::netwatch's fake-opener test).
        for mask in [RTMGRP_LINK, ADDRESS_ROUTE_GROUPS] {
            match NetlinkEvents::new(mask) {
                Ok(ev) => assert_eq!(ev.groups().unwrap(), mask, "mask {mask:#x}"),
                Err(e) => {
                    let text = format!("{e:#}");
                    assert!(text.contains(&format!("{mask:#x}")) || text.contains("socket"));
                    eprintln!("netlink unavailable here ({text}) — acceptable in sandboxes");
                }
            }
        }
    }

    #[tokio::test]
    async fn a_boxed_source_forwards() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let mut boxed: Box<dyn LinkEvents> = Box::new(ScriptedEvents { rx });
        tx.send(()).await.unwrap();
        assert_eq!(boxed.next_change().await, Some(()));
        drop(tx);
        assert_eq!(boxed.next_change().await, None);
    }
}
