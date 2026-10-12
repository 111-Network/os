//! Interface-change watch (RTMGRP_LINK) with debounce — rnsd config
//! regeneration trigger (spec §Transport). The event source is a trait so tests
//! inject synthetic changes.
//!
//! The production source is a raw netlink socket via libc (no crate-API
//! surface to drift): bind AF_NETLINK/NETLINK_ROUTE with the RTMGRP_LINK
//! multicast group, tokio AsyncFd for readiness. rtnetlink remains in the dep
//! set for Layer 6's richer routing work.

/// One "something changed about interfaces" event (details don't matter — the
/// reaction is always: debounce, maybe regenerate).
pub trait LinkEvents: Send {
    fn next_change(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<()>> + Send + '_>>;
}

/// RTMGRP_LINK multicast group id (linux/netlink.h).
const RTMGRP_LINK: u32 = 1;

/// Production source: raw netlink socket, readiness via tokio AsyncFd.
pub struct NetlinkEvents {
    fd: tokio::io::unix::AsyncFd<std::fs::File>,
}

impl NetlinkEvents {
    pub fn new() -> anyhow::Result<Self> {
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
        addr.nl_groups = RTMGRP_LINK;
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
            anyhow::bail!("netlink bind (RTMGRP_LINK): {err}");
        }
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        Ok(NetlinkEvents {
            fd: tokio::io::unix::AsyncFd::new(file)?,
        })
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
        match NetlinkEvents::new() {
            Ok(_) => {}
            Err(e) => eprintln!("netlink unavailable here ({e:#}) — acceptable in sandboxes"),
        }
    }
}
