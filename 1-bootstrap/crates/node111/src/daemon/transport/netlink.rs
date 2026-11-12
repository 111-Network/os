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

/// One "something changed about interfaces" event (details don't matter — the
/// reaction is always: debounce, maybe regenerate).
pub trait LinkEvents: Send {
    fn next_change(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<()>> + Send + '_>>;
}

/// A boxed source is a source (`relay::netwatch` holds its opener's result as one).
impl<T: LinkEvents + ?Sized> LinkEvents for Box<T> {
    fn next_change(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<()>> + Send + '_>> {
        (**self).next_change()
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

/// Production source: raw netlink socket, readiness via tokio AsyncFd.
pub struct NetlinkEvents {
    fd: tokio::io::unix::AsyncFd<std::fs::File>,
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
}

impl LinkEvents for NetlinkEvents {
    fn next_change(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<()>> + Send + '_>> {
        Box::pin(async move {
            loop {
                let mut guard = self.fd.readable().await.ok()?;
                let result = guard.try_io(|inner| {
                    use std::os::unix::io::AsRawFd;
                    let mut buf = [0u8; 8192];
                    let rc = unsafe {
                        libc::recv(
                            inner.get_ref().as_raw_fd(),
                            buf.as_mut_ptr() as *mut libc::c_void,
                            buf.len(),
                            0,
                        )
                    };
                    if rc < 0 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(rc)
                    }
                });
                match result {
                    Ok(_) => return Some(()),
                    Err(_would_block) => continue,
                }
            }
        })
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
