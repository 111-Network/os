//! Layer 7 §12 (row D11): the metered budget of the carrier.
//!
//! **The rule (§12; §6.1 rule 6).** `metered = [{match: {iface | ssid}, daily_cap_bytes}]`.
//! Carrier bytes (bodies and headers, both ways) count in a rolling 24 h window on
//! `CLOCK_BOOTTIME`, persisted in `state/relay.json` (`MeteredWindow`, row D1), never the wall
//! clock. Past the cap no flush runs and the badge says "metered cap reached". The carrier reads
//! [`reached`] before every flush and [`count`]s what it moved after it.
//!
//! **The window.** Bytes go into 5-minute slots ([`SLOT_S`]; 288 slots are 24 h, the file's
//! limit) tagged with the `CLOCK_BOOTTIME` second their slot starts at; a slot leaves the window
//! 24 h after its start ([`WINDOW_S`]). `CLOCK_BOOTTIME` restarts at a boot, so the window carries
//! the kernel's boot id (`/proc/sys/kernel/random/boot_id`): slots counted in another boot, or
//! whose start lies ahead of the clock now (a boot whose id could not be read), are of unknown
//! age, so they are folded into one slot at second 0 of this boot (fail closed: they count until
//! 24 h into this boot, at least as long as they could still be inside the real window). Nothing
//! here reads the wall clock, and no decision rests on it.
//!
//! **Which links count (§12).** A rule matches the carrier's path when its `iface` carries a
//! default route now (`/proc/net/route`, `/proc/net/ipv6_route`, each read capped), and always
//! for an `ssid` rule: neither Linux nor Termux gives 111 the current SSID (R05 §3.5 item 4), so a
//! marked SSID is counted and enforced whatever network is up (fail closed; the owner removes the
//! rule to lift it). [`active_by_default_route`] is that test; the carrier takes it as a seam so
//! tests choose.
//!
//! Nothing here is constructed, read or written while no metered rule is set: [`reached`] with
//! an empty `metered` list reads no file and no clock, and the carrier counts only when a rule is
//! set (no write without a change, row D1).

use crate::daemon::relay::settings::{
    MeteredLink, MeteredMatch, MeteredSlot, MeteredUsage, MeteredWindow, MAX_BOOT_ID,
    MAX_WINDOW_SLOTS,
};
use std::io::Read;
use std::path::Path;

/// The rolling window (§12): 24 h of `CLOCK_BOOTTIME` seconds.
pub const WINDOW_S: u64 = 24 * 3600;
/// One slot of the window: 5 minutes, so a full window is [`MAX_WINDOW_SLOTS`] (288) slots.
pub const SLOT_S: u64 = WINDOW_S / MAX_WINDOW_SLOTS as u64;
/// The kernel's id of this boot.
pub const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";
/// The IPv4 routing table (default routes).
pub const PROC_ROUTE: &str = "/proc/net/route";
/// The IPv6 routing table (default routes).
pub const PROC_ROUTE6: &str = "/proc/net/ipv6_route";
/// Largest routing table read (bytes); the rest of a larger file is not read.
pub const ROUTE_READ_MAX: u64 = 64 * 1024;

/// A point on the window's clock: the boot and its `CLOCK_BOOTTIME` second (§12; never the wall
/// clock).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clock {
    /// The kernel's boot id (`None` when it could not be read).
    pub boot_id: Option<String>,
    /// `CLOCK_BOOTTIME` seconds.
    pub now_s: u64,
}

impl Clock {
    /// This host's clock now: `CLOCK_BOOTTIME` and the boot id. `None` only when
    /// `clock_gettime(CLOCK_BOOTTIME)` fails.
    pub fn now() -> Option<Clock> {
        Some(Clock {
            boot_id: read_boot_id(Path::new(BOOT_ID_PATH)),
            now_s: boottime_s()?,
        })
    }
}

/// `CLOCK_BOOTTIME` in whole seconds (monotonic, counts suspend; never the wall clock).
pub fn boottime_s() -> Option<u64> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec; CLOCK_BOOTTIME is a valid clock id on Linux.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) };
    if rc != 0 {
        return None;
    }
    u64::try_from(ts.tv_sec).ok()
}

/// The boot id in `path` (36 chars of hex and dashes), size checked before parse; `None` when it
/// cannot be read or is not one.
pub fn read_boot_id(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_BOOT_ID as u64 + 2)
        .read_to_string(&mut text)
        .ok()?;
    let id = text.trim();
    (!id.is_empty()
        && id.len() <= MAX_BOOT_ID
        && id.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-'))
    .then(|| id.to_string())
}

/// Bring `w` to `clock` (module doc): slots of another boot, or ahead of the clock, folded into
/// one slot at second 0; slots whose 24 h have passed dropped; empty links removed. Returns
/// whether `w` changed.
pub fn age(w: &mut MeteredWindow, clock: &Clock) -> bool {
    let before = w.clone();
    let foreign = w.boot_id != clock.boot_id;
    for l in &mut w.links {
        let ahead = l.slots.iter().any(|s| s.at_s > clock.now_s);
        if foreign || ahead {
            let total = l.slots.iter().fold(0u64, |a, s| a.saturating_add(s.bytes));
            l.slots.clear();
            if total > 0 {
                l.slots.push(MeteredSlot {
                    at_s: 0,
                    bytes: total,
                });
            }
        }
        l.slots
            .retain(|s| s.at_s.saturating_add(WINDOW_S) > clock.now_s);
    }
    w.links.retain(|l| !l.slots.is_empty());
    w.boot_id = clock.boot_id.clone();
    if w.links.is_empty() {
        w.boot_id = None;
    }
    *w != before
}

/// The bytes `matcher` counted in the window (call [`age`] first).
pub fn used(w: &MeteredWindow, matcher: &MeteredMatch) -> u64 {
    w.links
        .iter()
        .filter(|l| l.matcher == *matcher)
        .flat_map(|l| l.slots.iter())
        .fold(0u64, |a, s| a.saturating_add(s.bytes))
}

/// The cap of a metered link the carrier's path rides now is reached (§12: no flush runs).
/// `active` says which rules match the path ([`active_by_default_route`] in the daemon). With no
/// rule, `false` at once (no clock, no file).
pub fn reached(
    links: &[MeteredLink],
    window: &MeteredWindow,
    clock: Option<&Clock>,
    active: &dyn Fn(&MeteredMatch) -> bool,
) -> bool {
    remaining(links, window, clock, active, 0) == Some(0)
}

/// The bytes the carrier may still move before the first cap of a metered link its path rides
/// now is reached (§12), with `extra` bytes moved but not yet in `window` counted against every
/// active link; `None` when no rule is active (no limit). With no rule, `None` at once (no clock,
/// no file). The daemon's carrier counts its unsaved bytes as `extra`, so a cap reached by a
/// flush stops the next one in the same round, and a held stream stops at the cap.
pub fn remaining(
    links: &[MeteredLink],
    window: &MeteredWindow,
    clock: Option<&Clock>,
    active: &dyn Fn(&MeteredMatch) -> bool,
    extra: u64,
) -> Option<u64> {
    if links.is_empty() {
        return None;
    }
    let mut w = window.clone();
    if let Some(c) = clock {
        age(&mut w, c);
    }
    links
        .iter()
        .filter(|l| active(&l.matcher))
        .map(|l| {
            let spent = used(&w, &l.matcher).saturating_add(extra);
            l.daily_cap_bytes.saturating_sub(spent)
        })
        .min()
}

/// Count `bytes` of carrier traffic now against every rule the path matches (§12). Returns
/// whether `w` changed. The window keeps at most [`MAX_WINDOW_SLOTS`] slots a link: the two
/// oldest are merged into the newer of them first (bytes moved later, never earlier: fail
/// closed).
pub fn count(
    links: &[MeteredLink],
    w: &mut MeteredWindow,
    clock: &Clock,
    active: &dyn Fn(&MeteredMatch) -> bool,
    bytes: u64,
) -> bool {
    let mut changed = age(w, clock);
    if bytes == 0 {
        return changed;
    }
    let at = clock.now_s - clock.now_s % SLOT_S;
    for l in links.iter().filter(|l| active(&l.matcher)) {
        let pos = match w.links.iter().position(|u| u.matcher == l.matcher) {
            Some(p) => p,
            None => {
                w.links.push(MeteredUsage {
                    matcher: l.matcher.clone(),
                    slots: Vec::new(),
                });
                w.links.len() - 1
            }
        };
        let slots = &mut w.links[pos].slots;
        match slots.last_mut() {
            Some(last) if last.at_s == at => last.bytes = last.bytes.saturating_add(bytes),
            _ => slots.push(MeteredSlot { at_s: at, bytes }),
        }
        while slots.len() > MAX_WINDOW_SLOTS {
            let first = slots.remove(0);
            slots[0].bytes = slots[0].bytes.saturating_add(first.bytes);
        }
        changed = true;
    }
    if changed && !w.links.is_empty() {
        w.boot_id = clock.boot_id.clone();
    }
    changed
}

/// The interfaces holding a default route now, from the two routing tables (each read capped at
/// [`ROUTE_READ_MAX`]). An unreadable table contributes nothing.
pub fn default_route_ifaces() -> Vec<String> {
    let mut out = Vec::new();
    if let Some(t) = read_capped(Path::new(PROC_ROUTE)) {
        out.extend(default_ifaces_v4(&t));
    }
    if let Some(t) = read_capped(Path::new(PROC_ROUTE6)) {
        out.extend(default_ifaces_v6(&t));
    }
    out.sort();
    out.dedup();
    out
}

fn read_capped(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(ROUTE_READ_MAX)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

/// `/proc/net/route`: `Iface Destination Gateway Flags … Mask …` (hex); a default route has
/// destination and mask 0.
pub fn default_ifaces_v4(table: &str) -> Vec<String> {
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            (f.len() >= 8 && f[1] == "00000000" && f[7] == "00000000").then(|| f[0].to_string())
        })
        .collect()
}

/// `/proc/net/ipv6_route`: `dest dest_len src src_len next_hop metric ref use flags iface`; a
/// default route has an all-zero destination of length 0. The loopback's unreachable default
/// (`lo`) is not a path.
pub fn default_ifaces_v6(table: &str) -> Vec<String> {
    table
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            (f.len() >= 10 && f[0].bytes().all(|b| b == b'0') && f[1] == "00" && f[9] != "lo")
                .then(|| f[9].to_string())
        })
        .collect()
}

/// The daemon's rule match (module doc): an `iface` rule when that interface holds a default
/// route now, an `ssid` rule always.
pub fn active_by_default_route(m: &MeteredMatch) -> bool {
    match m {
        MeteredMatch::Iface(name) => default_route_ifaces().iter().any(|i| i == name),
        MeteredMatch::Ssid(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(iface: &str, cap: u64) -> MeteredLink {
        MeteredLink {
            matcher: MeteredMatch::Iface(iface.into()),
            daily_cap_bytes: cap,
        }
    }

    fn clock(boot: &str, now_s: u64) -> Clock {
        Clock {
            boot_id: Some(boot.into()),
            now_s,
        }
    }

    const BOOT: &str = "0f8e2d1c-aaaa-bbbb-cccc-0123456789ab";
    const OTHER: &str = "0f8e2d1c-aaaa-bbbb-cccc-ffffffffffff";

    #[test]
    fn bytes_count_against_the_active_links_and_reach_the_cap() {
        let links = vec![link("wwan0", 1000), link("eth9", 10)];
        let only_wwan = |m: &MeteredMatch| *m == MeteredMatch::Iface("wwan0".into());
        let mut w = MeteredWindow::default();
        let c = clock(BOOT, 10_000);
        assert!(!reached(&links, &w, Some(&c), &only_wwan));
        assert!(count(&links, &mut w, &c, &only_wwan, 600));
        assert_eq!(used(&w, &links[0].matcher), 600);
        assert_eq!(
            used(&w, &links[1].matcher),
            0,
            "eth9 carries no default route"
        );
        assert!(!reached(&links, &w, Some(&c), &only_wwan));
        count(&links, &mut w, &clock(BOOT, 10_100), &only_wwan, 400);
        assert!(reached(&links, &w, Some(&clock(BOOT, 10_100)), &only_wwan));
        // the window persists through the file's own check
        let mut s = crate::daemon::relay::settings::RelaySettings::default();
        s.set_metered(links.clone()).unwrap();
        s.set_metered_window(w.clone()).unwrap();
        // 24 h after the first slot it has left the window
        let later = clock(BOOT, 10_000 - 10_000 % SLOT_S + WINDOW_S);
        assert!(!reached(&links, &w, Some(&later), &only_wwan));
        assert!(!reached(&[], &w, None, &only_wwan), "no rule, no cap");
    }

    #[test]
    fn remaining_counts_the_unsaved_bytes_against_the_active_links() {
        let links = vec![link("wwan0", 1000), link("eth9", 10)];
        let only_wwan = |m: &MeteredMatch| *m == MeteredMatch::Iface("wwan0".into());
        let mut w = MeteredWindow::default();
        let c = clock(BOOT, 10_000);
        assert_eq!(remaining(&links, &w, Some(&c), &only_wwan, 0), Some(1000));
        count(&links, &mut w, &c, &only_wwan, 600);
        assert_eq!(remaining(&links, &w, Some(&c), &only_wwan, 0), Some(400));
        // bytes moved and not yet saved count too: a flush stops the next one in the round
        assert_eq!(remaining(&links, &w, Some(&c), &only_wwan, 399), Some(1));
        assert_eq!(remaining(&links, &w, Some(&c), &only_wwan, 400), Some(0));
        assert_eq!(
            remaining(&links, &w, Some(&c), &only_wwan, u64::MAX),
            Some(0)
        );
        // the smallest cap left of the active links; none active: no limit
        let all = |_: &MeteredMatch| true;
        assert_eq!(remaining(&links, &w, Some(&c), &all, 0), Some(10));
        assert_eq!(remaining(&links, &w, Some(&c), &|_| false, 0), None);
        assert_eq!(remaining(&[], &w, None, &all, 5), None);
    }

    #[test]
    fn another_boot_folds_its_slots_into_second_zero() {
        let links = vec![link("wwan0", 1000)];
        let all = |_: &MeteredMatch| true;
        let mut w = MeteredWindow::default();
        count(&links, &mut w, &clock(OTHER, 80_000), &all, 900);
        // a reboot: CLOCK_BOOTTIME restarted; the old bytes are of unknown age
        let c = clock(BOOT, 100);
        assert!(age(&mut w, &c));
        assert_eq!(w.boot_id.as_deref(), Some(BOOT));
        assert_eq!(
            w.links[0].slots,
            vec![MeteredSlot {
                at_s: 0,
                bytes: 900
            }]
        );
        count(&links, &mut w, &c, &all, 100);
        assert!(reached(&links, &w, Some(&c), &all));
        // they count until 24 h into this boot, then leave
        assert!(!reached(
            &links,
            &w,
            Some(&clock(BOOT, WINDOW_S + 300)),
            &all
        ));
        // an unreadable boot id: a slot ahead of the clock is folded too
        let mut w2 = MeteredWindow::default();
        let none = |n| Clock {
            boot_id: None,
            now_s: n,
        };
        count(&links, &mut w2, &none(50_000), &all, 5);
        age(&mut w2, &none(10));
        assert_eq!(w2.links[0].slots, vec![MeteredSlot { at_s: 0, bytes: 5 }]);
    }

    #[test]
    fn the_window_never_holds_more_than_its_slots() {
        let links = vec![link("wwan0", u64::MAX)];
        let all = |_: &MeteredMatch| true;
        let mut w = MeteredWindow::default();
        for i in 0..(MAX_WINDOW_SLOTS as u64 + 5) {
            count(
                &links,
                &mut w,
                &clock(BOOT, 1_000_000 + i * SLOT_S),
                &all,
                1,
            );
        }
        assert!(w.links[0].slots.len() <= MAX_WINDOW_SLOTS);
        let mut s = crate::daemon::relay::settings::RelaySettings::default();
        s.set_metered(links.clone()).unwrap();
        s.set_metered_window(w).unwrap();
    }

    #[test]
    fn default_routes_are_read_from_both_tables() {
        let v4 =
            "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                  wwan0\t00000000\t0101A8C0\t0003\t0\t0\t0\t00000000\t0\t0\t0\n\
                  eth0\t0001A8C0\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\n";
        assert_eq!(default_ifaces_v4(v4), vec!["wwan0".to_string()]);
        let v6 = "00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000001 00000400 00000001 00000000 00000003 wlan0\n\
                  00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 ffffffff 00000001 00000000 00200200 lo\n\
                  fd000000000000000000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001 eth0\n";
        assert_eq!(default_ifaces_v6(v6), vec!["wlan0".to_string()]);
        assert!(active_by_default_route(&MeteredMatch::Ssid("Phone".into())));
    }

    #[test]
    fn the_boot_clock_reads() {
        let c = Clock::now().expect("CLOCK_BOOTTIME");
        assert!(c.now_s > 0);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("boot_id");
        std::fs::write(&p, "not a boot id!\n").unwrap();
        assert_eq!(read_boot_id(&p), None);
        std::fs::write(&p, format!("{BOOT}\n")).unwrap();
        assert_eq!(read_boot_id(&p).as_deref(), Some(BOOT));
    }
}
