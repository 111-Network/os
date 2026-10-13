//! mDNS announce of `111.local` while the dashboard is LAN-bound (D15, spec §1).
//!
//! Service `_http._tcp`, instance `111`, host `111.local.`, addresses tracked
//! automatically by `mdns-sd` as interfaces come and go. On a name conflict
//! `mdns-sd` renames the host to `111-N.local.` and reports it; the rename is
//! logged and handed to `on_rename` so the URL list stays truthful (the Host
//! guard already accepts `111-N.local`). Stopped on daemon shutdown.
//!
//! Behind the default-on cargo feature `mdns`; without it [`Announcer::start`]
//! logs once and returns `None`.

/// Hostname announced (FQDN form for mdns-sd; printed without the trailing dot).
pub const HOSTNAME: &str = "111.local.";

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
            let props = std::collections::HashMap::from([("path".to_string(), "/".to_string())]);
            let info =
                mdns_sd::ServiceInfo::new("_http._tcp.local.", "111", HOSTNAME, (), port, props)?
                    .enable_addr_auto();
            let fullname = info.get_fullname().to_string();
            let monitor = daemon.monitor()?;
            daemon.register(info)?;
            std::thread::Builder::new()
                .name("111d-mdns-mon".into())
                .spawn(move || {
                    while let Ok(ev) = monitor.recv() {
                        if let mdns_sd::DaemonEvent::NameChange(c) = ev {
                            if c.original == HOSTNAME {
                                let host = c.new_name.trim_end_matches('.').to_string();
                                eprintln!(
                                    "111 daemon: mDNS name conflict — announcing as {host} instead of 111.local"
                                );
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

    /// Withdraw the record (goodbye packet) and stop the mDNS thread.
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
