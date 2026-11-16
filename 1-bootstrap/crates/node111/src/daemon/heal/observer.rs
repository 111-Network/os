//! Layer 9 (§3.1, §3.4, §3.6; §4.1 rules 1, 2 and 4): T0 stub; filled by rows D10 (children and
//! their exits), D11 (rows, health and the `incident` frame), D13 (the two T0 calls), D16 (the open
//! incidents at start), D18 (Layer 8's sources and the degraded modes), D25 (a `repair` withdrawn
//! when its incident closes) and D38 (the stored fixes listed once at an incident's open).
//!
//! One task in `111d`, spawned once the supervisor's children exist and stopped before
//! `layer6.shutdown()`. It wakes only on channels other tasks already feed: each supervised child's
//! state and exit watches, the generation watches, the services registry's change signal and the
//! `EventBus`, a wake-up only (it never parses a frame; it reads the typed state `111d` holds).
//! Only an exit with `requested: false` and a service row in state `error` open or bump an
//! incident; one closes `self_recovered` after a one-shot sleep armed by a transition. No interval,
//! no poll, no socket and no heartbeat of its own.
//!
//! T0: the name only; nothing is spawned (row D10 adds the spawn).
#![allow(dead_code)]

/// The observer task (§3.1). T0: the name only.
pub struct Observer;
