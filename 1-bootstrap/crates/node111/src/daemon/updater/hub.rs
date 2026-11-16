//! The home hub as an update origin, the member side (Layer 7 §9.1 S4, §9.3; row L4).
//!
//! - **When (§9.3, M1).** The home hub's origin is checked once when the fabric link to it
//!   attributes: on the rising edge of [`Fabric::attributed_on`] for the enrolled hub, read once
//!   when the watch starts (an attribution that completed before it subscribed counts) and then
//!   on each `FabricEvent::Fabric` (or `Settings`) batch. The edge falls when the link stops
//!   attributing, the hub is forgotten or another hub is enrolled, so the next attribution checks
//!   again. No gossip, no timer, no retry: a check that fails waits for the next edge (or the
//!   hourly internet loop, or `111 update --from`). A re-attribution while the hub's release is
//!   already requested fetches the manifest only (`fetch::check_origins_with`).
//! - **Only while it serves (§9.3, D9).** The hub counts only while its announce carries
//!   `ROLE_ORIGIN` (bit 5, `sealer::ROLE_ORIGIN`), read from its contact's signed role bits at the
//!   edge: a hub that serves nothing gets no request. A role learned after the edge waits for the
//!   next one; a role read that failed leaves the edge for the next look.
//! - **Where (§9.1 S4).** `http://<home_hub.address host>:11180/origin/v1/`
//!   (`fetch::Origin::home_hub`), fetched under the typed `HomeHubOrigin` exception (T6); no
//!   address, no check.
//!
//! Weight (§1 rule 2): one task blocked on the fabric's event channel (no timer, no poll, no
//! socket); per `Fabric` batch while enrolled one `attributed_on` round trip to the actor that
//! runs anyway, and one `contact` read per rising edge (per batch while that read fails). The
//! check runs on the blocking pool and is awaited, so at most one runs, and events that arrive
//! meanwhile coalesce.

use crate::daemon::fabric::sealer::ROLE_ORIGIN;
use crate::daemon::fabric::FabricHandle;
use crate::daemon::updater::fetch::Origin;
use id111::keyset::NodeId;
use mesh111::fabric::{FabricEvent, HomeHub};
use std::future::Future;
use std::sync::{Arc, Weak};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::broadcast::Receiver;

/// The check the watch runs for the origin it names (blocking; `updater::hub_check`).
pub type Check = Arc<dyn Fn(Origin) + Send + Sync>;

/// The fabric reads the watch makes; a trait so the tests drive the edge without a store.
pub trait HubReads: Sync {
    /// The enrolled home hub, if any (the handle's persisted settings; no actor call).
    fn home_hub(&self) -> Option<HomeHub>;
    /// Whether a connection is attributed to `hub` now ([`Fabric::attributed_on`]); `None` when
    /// the read failed (the edge then stays as it was).
    fn attributed(&self, hub: NodeId) -> impl Future<Output = Option<bool>> + Send;
    /// The signed role bits of `hub`'s announce (its contact); `None` when unknown.
    fn roles(&self, hub: NodeId) -> impl Future<Output = Option<u16>> + Send;
}

impl HubReads for FabricHandle {
    fn home_hub(&self) -> Option<HomeHub> {
        self.persisted().fabric.home_hub
    }

    async fn attributed(&self, hub: NodeId) -> Option<bool> {
        let f = self.fabric()?;
        f.attributed_on(hub).await.ok().map(|k| k.is_some())
    }

    async fn roles(&self, hub: NodeId) -> Option<u16> {
        let f = self.fabric()?;
        f.contact(hub).await.ok().map(|c| c.roles)
    }
}

/// The attribution edge (§9.3): which hub was seen attributed at the last look.
#[derive(Debug, Default)]
pub struct HubWatch {
    seen: Option<NodeId>,
}

impl HubWatch {
    /// Look once (on a `Fabric` batch): the origin to check when the home hub's link has just
    /// begun to attribute and its announce carries `ROLE_ORIGIN`; otherwise `None` (no request).
    pub async fn on_fabric<R: HubReads>(&mut self, r: &R) -> Option<Origin> {
        let Some(hub) = r.home_hub() else {
            self.seen = None;
            return None;
        };
        match r.attributed(hub.node_id).await {
            None => return None,
            Some(false) => {
                self.seen = None;
                return None;
            }
            Some(true) => {}
        }
        if self.seen == Some(hub.node_id) {
            return None;
        }
        // a failed read leaves the edge for the next look, as a failed `attributed` read does
        let roles = r.roles(hub.node_id).await?;
        self.seen = Some(hub.node_id);
        if roles & ROLE_ORIGIN == 0 {
            return None;
        }
        Origin::home_hub(&hub.address?.ip().to_string())
    }
}

/// The home hub's origin as `GET /api/v1/update/origins` lists it (§9.3): the enrolled hub with
/// an address whose announce carries `ROLE_ORIGIN` now; attribution is not needed to list it.
/// One `contact` read; no request to the hub.
pub async fn listed<R: HubReads>(r: &R) -> Option<Origin> {
    let hub = r.home_hub()?;
    let address = hub.address?;
    let roles = r.roles(hub.node_id).await?;
    if roles & ROLE_ORIGIN == 0 {
        return None;
    }
    Origin::home_hub(&address.ip().to_string())
}

/// Watch the daemon's fabric (row L4): [`watch`] on its event channel, subscribed before the
/// first look. `None` while the fabric is not running (nothing to watch).
pub fn spawn(fabric: &Arc<FabricHandle>, check: Check) -> Option<tokio::task::JoinHandle<()>> {
    let rx = fabric.fabric()?.subscribe();
    Some(watch(rx, Arc::downgrade(fabric), check))
}

/// The watch's task (row L4): one look at once, so an attribution that completed before `rx`
/// was subscribed is checked without waiting for another event, then one look per batch of
/// events that may move the link to the home hub (`Fabric`, `Settings`, or a lag), the batch
/// coalesced; `check` runs (blocking, on the blocking pool, awaited) for the origin
/// [`HubWatch::on_fabric`] names. The task ends with the channel or with `reads`.
pub fn watch<R>(
    mut rx: Receiver<FabricEvent>,
    reads: Weak<R>,
    check: Check,
) -> tokio::task::JoinHandle<()>
where
    R: HubReads + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let mut watch = HubWatch::default();
        loop {
            let Some(handle) = reads.upgrade() else {
                return;
            };
            let origin = watch.on_fabric(&*handle).await;
            drop(handle);
            if let Some(origin) = origin {
                let check = Arc::clone(&check);
                if let Err(e) = tokio::task::spawn_blocking(move || check(origin)).await {
                    eprintln!("111 updater: the home hub's check ended early: {e}");
                }
            }
            loop {
                match rx.recv().await {
                    Ok(FabricEvent::Fabric | FabricEvent::Settings(_))
                    | Err(RecvError::Lagged(_)) => break,
                    Ok(_) => continue,
                    Err(RecvError::Closed) => return,
                }
            }
            // coalesce what is queued: the look reads the state now
            loop {
                match rx.try_recv() {
                    Ok(_) | Err(TryRecvError::Lagged(_)) => {}
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Closed) => return,
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fake {
        hub: Mutex<Option<HomeHub>>,
        attributed: Mutex<Option<bool>>,
        roles: Mutex<Option<u16>>,
        reads: Mutex<Vec<&'static str>>,
    }

    impl Fake {
        fn new() -> Fake {
            Fake {
                hub: Mutex::new(Some(HomeHub {
                    node_id: NodeId::from([7u8; 32]),
                    address: Some("192.168.1.10:11183".parse().unwrap()),
                })),
                attributed: Mutex::new(Some(false)),
                roles: Mutex::new(Some(ROLE_ORIGIN | 1)),
                reads: Mutex::new(Vec::new()),
            }
        }
    }

    impl HubReads for Fake {
        fn home_hub(&self) -> Option<HomeHub> {
            *self.hub.lock().unwrap()
        }
        fn attributed(&self, _hub: NodeId) -> impl Future<Output = Option<bool>> + Send {
            self.reads.lock().unwrap().push("attributed");
            let v = *self.attributed.lock().unwrap();
            async move { v }
        }
        fn roles(&self, _hub: NodeId) -> impl Future<Output = Option<u16>> + Send {
            self.reads.lock().unwrap().push("roles");
            let v = *self.roles.lock().unwrap();
            async move { v }
        }
    }

    fn hub_origin() -> Option<Origin> {
        Some(Origin::HomeHub {
            base: "http://192.168.1.10:11180/origin/v1/".into(),
        })
    }

    #[tokio::test]
    async fn one_check_per_rising_edge() {
        let f = Fake::new();
        let mut w = HubWatch::default();
        assert_eq!(w.on_fabric(&f).await, None, "not attributed");
        *f.attributed.lock().unwrap() = Some(true);
        assert_eq!(w.on_fabric(&f).await, hub_origin(), "the edge");
        assert_eq!(w.on_fabric(&f).await, None, "still attributed: once");
        // a failed read changes nothing
        *f.attributed.lock().unwrap() = None;
        assert_eq!(w.on_fabric(&f).await, None);
        *f.attributed.lock().unwrap() = Some(true);
        assert_eq!(w.on_fabric(&f).await, None, "the same edge");
        // down, then up again: a new edge
        *f.attributed.lock().unwrap() = Some(false);
        assert_eq!(w.on_fabric(&f).await, None);
        *f.attributed.lock().unwrap() = Some(true);
        assert_eq!(w.on_fabric(&f).await, hub_origin());
    }

    #[tokio::test]
    async fn the_role_bit_gates_the_request() {
        let f = Fake::new();
        *f.attributed.lock().unwrap() = Some(true);
        *f.roles.lock().unwrap() = Some(1);
        let mut w = HubWatch::default();
        assert_eq!(w.on_fabric(&f).await, None, "no ROLE_ORIGIN: nothing");
        // the bit learned later counts at the next edge only
        *f.roles.lock().unwrap() = Some(ROLE_ORIGIN);
        assert_eq!(w.on_fabric(&f).await, None);
        *f.attributed.lock().unwrap() = Some(false);
        w.on_fabric(&f).await;
        *f.attributed.lock().unwrap() = Some(true);
        assert_eq!(w.on_fabric(&f).await, hub_origin());
        // unknown roles: nothing
        let g = Fake::new();
        *g.attributed.lock().unwrap() = Some(true);
        *g.roles.lock().unwrap() = None;
        assert_eq!(HubWatch::default().on_fabric(&g).await, None);
    }

    #[tokio::test]
    async fn no_hub_or_no_address_reads_nothing_more() {
        let f = Fake::new();
        *f.hub.lock().unwrap() = None;
        let mut w = HubWatch::default();
        assert_eq!(w.on_fabric(&f).await, None);
        assert!(f.reads.lock().unwrap().is_empty(), "no actor read");
        let g = Fake::new();
        *g.attributed.lock().unwrap() = Some(true);
        g.hub.lock().unwrap().as_mut().unwrap().address = None;
        assert_eq!(HubWatch::default().on_fabric(&g).await, None);
        // an IPv6 hub is bracketed
        let h = Fake::new();
        *h.attributed.lock().unwrap() = Some(true);
        h.hub.lock().unwrap().as_mut().unwrap().address = Some("[fd00::7]:11183".parse().unwrap());
        assert_eq!(
            HubWatch::default().on_fabric(&h).await,
            Some(Origin::HomeHub {
                base: "http://[fd00::7]:11180/origin/v1/".into()
            })
        );
    }

    #[tokio::test]
    async fn the_listed_hub_needs_the_role_not_the_link() {
        let f = Fake::new();
        assert_eq!(
            listed(&f).await,
            hub_origin(),
            "listed while not attributed"
        );
        assert!(!f.reads.lock().unwrap().contains(&"attributed"));
        *f.roles.lock().unwrap() = Some(1);
        assert_eq!(listed(&f).await, None);
        *f.hub.lock().unwrap() = None;
        assert_eq!(listed(&f).await, None);
    }

    #[tokio::test]
    async fn another_hub_is_a_new_edge() {
        let f = Fake::new();
        *f.attributed.lock().unwrap() = Some(true);
        let mut w = HubWatch::default();
        assert!(w.on_fabric(&f).await.is_some());
        f.hub.lock().unwrap().as_mut().unwrap().node_id = NodeId::from([8u8; 32]);
        assert!(w.on_fabric(&f).await.is_some(), "a newly enrolled hub");
    }

    /// Review of row L4: a role read that fails at the edge does not consume it.
    #[tokio::test]
    async fn a_failed_role_read_leaves_the_edge() {
        let f = Fake::new();
        *f.attributed.lock().unwrap() = Some(true);
        *f.roles.lock().unwrap() = None;
        let mut w = HubWatch::default();
        assert_eq!(w.on_fabric(&f).await, None, "the read failed");
        *f.roles.lock().unwrap() = Some(ROLE_ORIGIN);
        assert_eq!(
            w.on_fabric(&f).await,
            hub_origin(),
            "the same edge, read again"
        );
        assert_eq!(w.on_fabric(&f).await, None, "then once");
    }

    async fn until(what: &str, f: impl Fn() -> bool) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while !f() {
            assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    fn reads_of(f: &Fake, what: &str) -> usize {
        f.reads
            .lock()
            .unwrap()
            .iter()
            .filter(|r| **r == what)
            .count()
    }

    /// Review of row L4: the task looks once at start (an attribution that completed before it
    /// subscribed is checked with no event), then once per `Fabric`/`Settings`/lag batch; other
    /// events make no look; it ends with the channel.
    #[tokio::test]
    async fn the_watch_looks_at_once_then_per_batch() {
        use mesh111::fabric::ContactChange;
        let f = Arc::new(Fake::new());
        *f.attributed.lock().unwrap() = Some(true);
        let (tx, rx) = tokio::sync::broadcast::channel(2);
        let calls: Arc<Mutex<Vec<Origin>>> = Arc::default();
        let check: Check = {
            let calls = Arc::clone(&calls);
            Arc::new(move |o| calls.lock().unwrap().push(o))
        };
        let task = watch(rx, Arc::downgrade(&f), check);
        let n = || calls.lock().unwrap().len();
        until("the look at start", || n() == 1).await;
        assert_eq!(calls.lock().unwrap()[0], hub_origin().unwrap());

        // an event that cannot move the link: no look
        let looks = reads_of(&f, "attributed");
        tx.send(FabricEvent::Contact {
            node_id: NodeId::from([7u8; 32]),
            change: ContactChange::Updated,
        })
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            reads_of(&f, "attributed"),
            looks,
            "a contact event: no look"
        );

        // still attributed: a look, no check
        tx.send(FabricEvent::Fabric).unwrap();
        until("a look", || reads_of(&f, "attributed") > looks).await;
        assert_eq!(n(), 1);

        // down, then up again: the next edge
        *f.attributed.lock().unwrap() = Some(false);
        let looks = reads_of(&f, "attributed");
        tx.send(FabricEvent::Fabric).unwrap();
        until("the fall", || reads_of(&f, "attributed") > looks).await;
        *f.attributed.lock().unwrap() = Some(true);
        tx.send(FabricEvent::Settings(Default::default())).unwrap();
        until("the second edge", || n() == 2).await;

        // a lag is a batch too
        *f.attributed.lock().unwrap() = Some(false);
        let looks = reads_of(&f, "attributed");
        for _ in 0..5 {
            let _ = tx.send(FabricEvent::Contact {
                node_id: NodeId::from([7u8; 32]),
                change: ContactChange::Updated,
            });
        }
        until("a look after a lag", || reads_of(&f, "attributed") > looks).await;

        drop(tx);
        tokio::time::timeout(std::time::Duration::from_secs(10), task)
            .await
            .expect("the task ends with its channel")
            .unwrap();
        assert_eq!(n(), 2);
    }
}
