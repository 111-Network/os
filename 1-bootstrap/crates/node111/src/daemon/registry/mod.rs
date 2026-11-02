//! Layer 7 (§7.1, §7.4, §7.6, §7.7): T0 stub; filled by row D8 (the DNS-SD record, the mdns-sd
//! re-arm, lost-hub recovery).
//!
//! The Service Discovery Registry: the `_111._tcp.local.` record (TXT `v`, `r`, `k`; only with
//! `discovery.lan_record`, a LAN-bound dashboard and a LAN-bound fabric listener), the ≤ 5 s browse
//! behind `POST /api/v1/registry/browse`, the rows of `GET /api/v1/registry` and the SSE `registry`
//! frame, and "Find my hub" (§7.7). Every string is untrusted text. No listener (the record rides
//! mdns-sd's socket), no browse timer; the sealed query stays unbuilt (§7.3).
#![allow(dead_code)]

/// The registry's rows and browse state (§7.1, §7.4). T0: the name only.
pub struct Registry;
