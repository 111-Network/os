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
//! Behind the default-on cargo feature `mdns`; without it [`Announcer::start`]
//! logs once and returns `None`.

/// Hostname announced (FQDN form for mdns-sd; printed without the trailing dot).
pub const HOSTNAME: &str = "111.local.";

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

pub struct Announcer {
    #[cfg(feature = "mdns")]
    daemon: mdns_sd::ServiceDaemon,
    #[cfg(feature = "mdns")]
    fullname: String,
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
            Ok(Announcer { daemon, fullname })
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

    /// Withdraw the record (goodbye packet) and stop the mDNS daemon thread
    /// (the monitor thread ends when its channel closes).
    pub fn stop(&self) {
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
