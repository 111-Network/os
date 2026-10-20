//! Hotplug (`os/6-transport/README.md` §12 "Hotplug", §18 Layer 1
//! "`AF_NETLINK` kept if the unit restricts address families"; DEV §3.1:
//! "udev/netlink uevents (fallback: 5 s poll)").
//!
//! A [`HotplugSource`] yields kernel uevents; the registry's tick loop
//! ([`super::run_ticks`]) treats a relevant one as a **kick**: it runs a tick
//! at once instead of waiting for the next 5 s one. Nothing about the event
//! is trusted or used beyond "something on the tty or usb bus changed" — the
//! tick re-reads sysfs as always, so the 5 s poll stays the authoritative
//! fallback and the 2-tick debounce (`registry::DEBOUNCE_TICKS`) is unchanged.
//!
//! - **Production: [`NetlinkUevent`]**, a raw `AF_NETLINK` /
//!   `NETLINK_KOBJECT_UEVENT` socket bound to multicast group 1 (the kernel's
//!   uevents; receiving needs no privilege) through libc, readiness through
//!   tokio's `AsyncFd` — the same shape as `transport::netlink`'s
//!   `NetlinkEvents`. One file descriptor and no timer: an idle node pays
//!   nothing (the Pi floor). A datagram whose sender is not the kernel
//!   (`nl_pid != 0`) is ignored.
//! - **Tests: [`FixtureSource`]**, raw uevent buffers (the bytes the kernel
//!   sends, `tests/fixtures/uevent/*.uevent`) pushed through a channel and
//!   parsed exactly like the socket's.
//!
//! [`parse`] reads `ACTION@DEVPATH\0KEY=VALUE\0…` (size checked before the
//! parse, [`MAX_UEVENT_BYTES`]) and keeps only `ACTION=add|remove|bind` on
//! `SUBSYSTEM=tty|usb` — the events a board plug or unplug produces. A
//! udev-format datagram (`libudev\0` magic, group 2) is never parsed.

use super::{clean, MAX_PATH_FACT};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// `NETLINK_KOBJECT_UEVENT` (`linux/netlink.h`).
pub const NETLINK_KOBJECT_UEVENT: libc::c_int = 15;
/// The kernel's uevent multicast group (udev's is 2).
pub const UEVENT_KERNEL_GROUP: u32 = 1;
/// Largest uevent datagram parsed (the kernel caps one at 2048 B of env;
/// anything larger is not a uevent).
pub const MAX_UEVENT_BYTES: usize = 8192;

/// A boxed `Send` future (object-safe async sources without a macro crate).
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The uevent actions that kick a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Add,
    Remove,
    Bind,
}

/// The subsystems a radio board appears on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subsystem {
    Tty,
    Usb,
}

/// One relevant kernel uevent (display facts only: cleaned, capped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uevent {
    pub action: Action,
    pub subsystem: Subsystem,
    /// `DEVPATH` (sysfs path below `/sys`).
    pub devpath: String,
    /// `DEVNAME` (`ttyACM0`, `bus/usb/001/004`), when the event has one.
    pub devname: Option<String>,
}

/// Parse one kernel uevent datagram; `None` for anything that is not a
/// relevant kernel uevent (module doc). Never panics on input.
pub fn parse(buf: &[u8]) -> Option<Uevent> {
    if buf.is_empty() || buf.len() > MAX_UEVENT_BYTES || buf.starts_with(b"libudev\0") {
        return None;
    }
    let mut fields = buf.split(|b| *b == 0).filter(|f| !f.is_empty());
    // the header `ACTION@DEVPATH` comes first in a kernel uevent
    let header = std::str::from_utf8(fields.next()?).ok()?;
    let (head_action, head_path) = header.split_once('@')?;
    let (mut action, mut subsystem, mut devpath, mut devname) = (None, None, None, None);
    for f in fields {
        let Ok(kv) = std::str::from_utf8(f) else {
            continue;
        };
        let Some((k, v)) = kv.split_once('=') else {
            continue;
        };
        match k {
            "ACTION" => action = Some(v),
            "SUBSYSTEM" => subsystem = Some(v),
            "DEVPATH" => devpath = Some(v),
            "DEVNAME" => devname = Some(v),
            _ => {}
        }
    }
    let action = action.unwrap_or(head_action);
    if action != head_action {
        return None;
    }
    let action = match action {
        "add" => Action::Add,
        "remove" => Action::Remove,
        "bind" => Action::Bind,
        _ => return None,
    };
    let subsystem = match subsystem? {
        "tty" => Subsystem::Tty,
        "usb" => Subsystem::Usb,
        _ => return None,
    };
    let devpath = devpath.unwrap_or(head_path);
    Some(Uevent {
        action,
        subsystem,
        devpath: clean(devpath, MAX_PATH_FACT)?,
        devname: devname.and_then(|n| clean(n, MAX_PATH_FACT)),
    })
}

/// A source of relevant uevents (injectable, like `transport::netlink`'s
/// `LinkEvents`). `None` = the source closed.
pub trait HotplugSource: Send {
    fn next_event(&mut self) -> BoxFut<'_, Option<Uevent>>;
}

/// Production source (module doc).
pub struct NetlinkUevent {
    fd: tokio::io::unix::AsyncFd<std::fs::File>,
}

impl NetlinkUevent {
    /// Open and bind the uevent socket (no privilege needed for group 1).
    pub fn new() -> anyhow::Result<Self> {
        use std::os::unix::io::FromRawFd;
        // SAFETY: plain socket(2); the fd is owned by the File below.
        let fd = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_DGRAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                NETLINK_KOBJECT_UEVENT,
            )
        };
        anyhow::ensure!(
            fd >= 0,
            "uevent socket: {}",
            std::io::Error::last_os_error()
        );
        // SAFETY: zeroed sockaddr_nl is a valid value of the C struct.
        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as u16;
        addr.nl_groups = UEVENT_KERNEL_GROUP;
        // SAFETY: addr is a valid sockaddr_nl of the size passed.
        let rc = unsafe {
            libc::bind(
                fd,
                &addr as *const libc::sockaddr_nl as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_nl>() as u32,
            )
        };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            // SAFETY: fd is ours and not wrapped yet.
            unsafe { libc::close(fd) };
            anyhow::bail!("uevent bind (group {UEVENT_KERNEL_GROUP}): {err}");
        }
        // SAFETY: fd is a fresh, owned descriptor.
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        Ok(NetlinkUevent {
            fd: tokio::io::unix::AsyncFd::new(file)?,
        })
    }
}

/// One `recvfrom` into `buf`: `(len, sender nl_pid)`.
fn recv_from(fd: libc::c_int, buf: &mut [u8]) -> std::io::Result<(usize, u32)> {
    // SAFETY: zeroed sockaddr_nl is valid; recvfrom writes at most its size.
    let mut from: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t;
    // SAFETY: buf and from are valid for the lengths passed.
    let rc = unsafe {
        libc::recvfrom(
            fd,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len(),
            libc::MSG_TRUNC,
            &mut from as *mut libc::sockaddr_nl as *mut libc::sockaddr,
            &mut len,
        )
    };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((rc as usize, from.nl_pid))
}

impl HotplugSource for NetlinkUevent {
    fn next_event(&mut self) -> BoxFut<'_, Option<Uevent>> {
        Box::pin(async move {
            let mut buf = vec![0u8; MAX_UEVENT_BYTES];
            loop {
                let mut guard = self.fd.readable().await.ok()?;
                let got = guard.try_io(|inner| {
                    use std::os::unix::io::AsRawFd;
                    recv_from(inner.get_ref().as_raw_fd(), &mut buf)
                });
                match got {
                    // a truncated datagram (MSG_TRUNC reports the full size)
                    // or one not from the kernel is not a uevent
                    Ok(Ok((n, 0))) if n <= buf.len() => {
                        if let Some(e) = parse(&buf[..n]) {
                            return Some(e);
                        }
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) if e.raw_os_error() == Some(libc::ENOBUFS) => {
                        // the kernel dropped events: the tick re-reads sysfs
                        return Some(Uevent {
                            action: Action::Add,
                            subsystem: Subsystem::Usb,
                            devpath: "(overrun)".into(),
                            devname: None,
                        });
                    }
                    Ok(Err(_)) => return None,
                    Err(_would_block) => continue,
                }
            }
        })
    }
}

/// Test source: raw datagrams pushed through a channel, parsed like the
/// socket's (irrelevant ones are skipped).
pub struct FixtureSource {
    pub rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
}

impl FixtureSource {
    /// A source and the sender that feeds it.
    pub fn channel(cap: usize) -> (tokio::sync::mpsc::Sender<Vec<u8>>, Self) {
        let (tx, rx) = tokio::sync::mpsc::channel(cap.max(1));
        (tx, FixtureSource { rx })
    }
}

impl HotplugSource for FixtureSource {
    fn next_event(&mut self) -> BoxFut<'_, Option<Uevent>> {
        Box::pin(async move {
            loop {
                let buf = self.rx.recv().await?;
                if let Some(e) = parse(&buf) {
                    return Some(e);
                }
            }
        })
    }
}

/// Forward every relevant event of `source` as a kick (`notify_one`: a
/// burst while a tick runs leaves one pending kick, never a queue). Returns
/// when the source closes.
pub async fn forward<S: HotplugSource>(mut source: S, kick: Arc<tokio::sync::Notify>) {
    while source.next_event().await.is_some() {
        kick.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(parts: &[&str]) -> Vec<u8> {
        let mut v = Vec::new();
        for p in parts {
            v.extend_from_slice(p.as_bytes());
            v.push(0);
        }
        v
    }

    #[test]
    fn kernel_uevents_parse_and_only_tty_usb_add_remove_bind_count() {
        let add = ev(&[
            "add@/devices/pci0000:00/usb1/1-1/1-1:1.0/tty/ttyACM0",
            "ACTION=add",
            "DEVPATH=/devices/pci0000:00/usb1/1-1/1-1:1.0/tty/ttyACM0",
            "SUBSYSTEM=tty",
            "MAJOR=166",
            "MINOR=0",
            "DEVNAME=ttyACM0",
            "SEQNUM=4242",
        ]);
        let e = parse(&add).unwrap();
        assert_eq!((e.action, e.subsystem), (Action::Add, Subsystem::Tty));
        assert_eq!(e.devname.as_deref(), Some("ttyACM0"));
        assert!(e.devpath.ends_with("tty/ttyACM0"));
        let bind = ev(&[
            "bind@/devices/usb1/1-1",
            "ACTION=bind",
            "DEVPATH=/devices/usb1/1-1",
            "SUBSYSTEM=usb",
            "DEVTYPE=usb_device",
        ]);
        assert_eq!(parse(&bind).unwrap().action, Action::Bind);
        let remove = ev(&["remove@/devices/usb1/1-1", "ACTION=remove", "SUBSYSTEM=usb"]);
        assert_eq!(parse(&remove).unwrap().action, Action::Remove);
        // other subsystems, actions and shapes are not kicks
        for bad in [
            ev(&[
                "add@/devices/virtual/net/veth0",
                "ACTION=add",
                "SUBSYSTEM=net",
            ]),
            ev(&["change@/devices/usb1/1-1", "ACTION=change", "SUBSYSTEM=usb"]),
            ev(&["unbind@/devices/usb1/1-1", "ACTION=unbind", "SUBSYSTEM=usb"]),
            // header and ACTION disagree
            ev(&["add@/devices/usb1/1-1", "ACTION=remove", "SUBSYSTEM=usb"]),
            ev(&["no header here", "ACTION=add", "SUBSYSTEM=usb"]),
            ev(&["add@/devices/usb1/1-1", "ACTION=add"]),
            b"libudev\0\xfe\xed\xca\xfeadd@/x\0SUBSYSTEM=usb\0".to_vec(),
            Vec::new(),
            vec![0xff; 64],
            vec![b'a'; MAX_UEVENT_BYTES + 1],
        ] {
            assert_eq!(parse(&bad), None, "{:?}", String::from_utf8_lossy(&bad));
        }
        // untrusted text is cleaned
        let evil = ev(&[
            "add@/devices/x",
            "ACTION=add",
            "SUBSYSTEM=tty",
            "DEVNAME=tty\u{202e}ACM\u{7}9",
        ]);
        assert_eq!(parse(&evil).unwrap().devname.as_deref(), Some("ttyACM9"));
    }

    #[tokio::test]
    async fn fixture_source_skips_noise_and_forward_kicks_once_per_event() {
        let (tx, src) = FixtureSource::channel(8);
        let kick = Arc::new(tokio::sync::Notify::new());
        let task = tokio::spawn(forward(src, Arc::clone(&kick)));
        tx.send(ev(&["add@/x", "ACTION=add", "SUBSYSTEM=net"]))
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), kick.notified())
                .await
                .is_err(),
            "noise kicked"
        );
        tx.send(ev(&["add@/x", "ACTION=add", "SUBSYSTEM=tty"]))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), kick.notified())
            .await
            .expect("a tty add kicks");
        drop(tx);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn the_uevent_socket_binds_unprivileged() {
        // a real socket (skipped where netlink is not available)
        match NetlinkUevent::new() {
            Ok(_) => {}
            Err(e) => eprintln!("uevent netlink unavailable here ({e:#})"),
        }
    }
}
