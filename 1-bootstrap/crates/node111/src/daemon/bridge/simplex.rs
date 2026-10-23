//! SimpleX (Layer 6 `6-transport/README.md` §7 "SimpleX (second; glibc hubs only)"; D27:
//! SimpleX ships only by the owner's hand-selection). **This release does not enable it.**
//!
//! What exists: the component declaration `assets/components/simplex.toml` (AGPL-3.0-only;
//! `[payload] kind = "none"`, so nothing is pinned, cached or downloaded; `[requires] glibc`,
//! so on Termux and musl hosts the enable refuses with its own copy before anything else, and
//! on a glibc hub with "SimpleX is not enabled in this release"), the adapter manifest
//! `assets/adapters/app-simplex.toml` (the honest copy the networks view shows), and the
//! placeholder [`SimplexAdapter`] below, which reports `Disabled` with that copy, yields
//! nothing and refuses every send.
//!
//! What does **not** exist, on purpose: no client of the SimpleX CLI's WebSocket bot API
//! (loopback 11232, `peerType: "bot"`), no `smp-server` runner, no pins. Building them is
//! the second-release work the owner selects (§17); the Delta Chat adapter
//! (`super::deltachat`) is the reference shape it would follow (one bridge account, the
//! warning once, per-contact opt-in, the relay marker as a visible prefix).

use super::{
    Adapter, AdapterState, Auth, Caps, ConvKind, ConvRef, Honest, Inbound, PerKind, Reason, Retry,
    SendEvent, SendHandle, SendOpts, NETWORK_SIMPLEX,
};
use futures_util::future::BoxFuture;
use tokio::sync::mpsc;

/// The copy of every refusal (and of the placeholder's state).
pub const NOT_ENABLED: &str = "SimpleX is not enabled in this release";

/// The placeholder adapter (module doc). Inert: no task, no process, no socket.
#[derive(Debug, Default)]
pub struct SimplexAdapter;

impl SimplexAdapter {
    pub fn new() -> Self {
        SimplexAdapter
    }
}

impl Adapter for SimplexAdapter {
    fn id(&self) -> &'static str {
        NETWORK_SIMPLEX
    }

    fn caps(&self) -> Caps {
        Caps {
            kinds: vec![ConvKind::Dm, ConvKind::Group],
            max_text_bytes: PerKind::default(),
            evidence: Vec::new(),
            airtime: None,
            auth_levels: vec![Auth::None],
            copy: Honest {
                label: "SimpleX".into(),
                chip: "SX".into(),
                tag: "SX".into(),
                banner: NOT_ENABLED.into(),
                send_warning: NOT_ENABLED.into(),
            },
            recipient_syntax: None,
            splittable: false,
            state: AdapterState::Disabled,
        }
    }

    /// Never yields (cancel-safe: nothing is taken).
    fn recv(&self) -> BoxFuture<'_, Inbound> {
        Box::pin(std::future::pending())
    }

    /// Refused at once (`adapter_down`, never retried on its own).
    fn send(&self, _conv: &ConvRef, _text: &str, _o: SendOpts) -> BoxFuture<'_, SendHandle> {
        Box::pin(async move {
            let (tx, rx) = mpsc::channel(1);
            let _ = tx.try_send(SendEvent::Failed {
                reason: Reason::AdapterDown,
                retry: Retry::Never,
            });
            SendHandle { events: rx }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;

    #[tokio::test]
    async fn the_placeholder_is_disabled_yields_nothing_and_refuses_sends() {
        let a = SimplexAdapter::new();
        assert_eq!(a.id(), "app:simplex");
        let caps = a.caps();
        assert_eq!(caps.state, AdapterState::Disabled);
        assert_eq!(caps.copy.banner, NOT_ENABLED);
        assert!(a.recv().now_or_never().is_none(), "nothing to receive");
        let conv = ConvRef {
            network: NETWORK_SIMPLEX.into(),
            account: "x".into(),
            kind: ConvKind::Dm,
            native_id: "dm:x".into(),
        };
        let mut h = a.send(&conv, "hi", SendOpts::default()).await;
        assert_eq!(
            h.events.recv().await,
            Some(SendEvent::Failed {
                reason: Reason::AdapterDown,
                retry: Retry::Never
            })
        );
        assert_eq!(h.events.recv().await, None);
    }
}
