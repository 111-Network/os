//! Layer 7 (§3.2–§3.4): T0 stub; filled by row D4.
//!
//! The second netlink socket (`RTMGRP_LINK`, `IPV4_IFADDR`, `IPV4_ROUTE`, `IPV6_IFADDR`,
//! `IPV6_ROUTE`), opened only while a consumer is subscribed (a configured carrier, the running
//! mdns-sd daemon) and closed with the last, debounced 2 s (§3.3; §19 D8); the Termux fallback poll
//! only on a Termux hub whose bind failed (§3.4). No socket, task or timer without a consumer.
#![allow(dead_code)]

/// The address and route event source (§3.3). T0: the name only.
pub struct NetWatch;
