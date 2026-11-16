//! Layer 9 (§12 delegation): T0 stub; the module root of delegation, its submodules filled by rows
//! D8 (`cbor`), D42 (`member`) and D43 and D54 (`hub`).
//!
//! A member with `heal.ask_hub` on asks its enrolled home hub to diagnose an incident it could not,
//! over `AGENT_OP` (0x10): `diagnose` from the member, `diagnosis` or `refuse` from the hub, each a
//! deterministic CBOR map, sent only on a link attributed to the home hub and never put in custody
//! or a mailbox. The hub serves only under `heal.serve_members`; the reply is a proposal the
//! member's human accepts, never applied by itself.
//!
//! T0: the submodules are names only.
#![allow(dead_code)]

pub mod cbor;
pub mod hub;
pub mod member;
