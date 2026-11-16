//! Layer 9 (§12.5 the member side; §12.2 the re-send; §17.2 `heal_delegate`): T0 stub; filled by
//! row D42.
//!
//! Under `heal.ask_hub` (off by default), when an incident opens with no T0 cause and the home
//! hub's card carries `ROLE_HEAL`, `111d` builds the `diagnose` request from the incident record
//! (no log line, nothing from a bridge adapter or the presence log) and sends it with `send_op`,
//! `direct_only`, to the home hub; an `open` delegation record keeps it. A request with no reply is
//! sent again, with the same `req_id`, on the next link-attribution event to the home hub: no
//! timer. A reply counts only from the home hub for an open `req_id` and becomes a proposal shown
//! through the peer subset; closing the incident cancels the request.
//!
//! T0: the name only.
#![allow(dead_code)]

/// The member's `heal/delegations` record (§12.5). T0: the name only.
pub struct Delegation;
