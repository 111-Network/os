//! Layer 9 (§7.2 the `bridge.*` rows; §13 item 5): T0 stub; filled by row D30.
//!
//! `bridge.edge.pause {edge_id}`, `bridge.edge.set {edge_id, oversize?, honour_ok_to_mqtt?}` (the
//! `PATCH /bridge/edges/{id}` fields) and `bridge.order.set {order}` (`interfaces.toml` `[order]`
//! through the spine) as typed actions: human or `device.propose` only in v1, never in a playbook
//! cause, a `repair` proposal or a peer fix. Resuming a `consent_paused` edge whose consent is not
//! current stops the revert `revert_blocked`.
//!
//! T0: the name only.
#![allow(dead_code)]

/// The three bridge actions (§7.2). T0: the name only.
pub struct BridgeAction;
