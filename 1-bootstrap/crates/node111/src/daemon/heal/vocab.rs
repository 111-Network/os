//! Layer 9 (§3.4 "One subject id space, one source per failure"; §6.3 "One vocabulary, one
//! source"; §16 row 1): the one compiled table of `(layer, subject, reason code)` rows, filled by
//! row D3; row D48 adds the `agent` row's `prompts_invalid` (§13 item 1).
//!
//! An incident's fingerprint (§3.4) is a row of [`ROWS`], and a playbook's `[match]` names one
//! (§6.2's block layout): nothing else is a subject or a reason code. The rows are:
//! - each service id with the reason codes its row publishes in state `error`, the row named by
//!   [`VocabRow::service_row`]: the `kb` row's `verify_failed`; the `agent` and `model` rows' codes
//!   that are not child codes; the `fabric` and `meshtastic` rows' codes (`store_full`, `port_busy`
//!   among them); the `radio` row's `stack_down` (the `transport` row publishes child codes only).
//!   The `fabric` row's `no_identity` and `restore_pending` are the `identity` subject's, layer 2,
//!   never a Layer 5 fingerprint; the health struct's `identity` is their other source (§3.4's
//!   Layer 2 row);
//! - [`CHILD_EXIT`] for each supervised service id (`agent`, `model`, `transport`, `meshtastic`)
//!   and for [`ANY`] component: an exit fact with `requested: false` (§3.2; §3.4 rule 2), its code
//!   or signal kept in the `exit` evidence. A component's fingerprint takes the `any` row's layer,
//!   6: the optional-component seam every component is declared, enabled and run under (L6 §2.1);
//! - the fixed daemon subjects: `111d` and `updater`, from §3.3's start facts, rule 7 and §12.6's
//!   APK check; `storage`, from Layer 8's typed storage state; `heal`, Layer 9's own (a revert that
//!   failed, the node-state store unopenable).
//!
//! Not rows (§3.4): the child codes `child_backoff`, `child_degraded`, `child_stopped`,
//! `spawn_pending` and `transport_degraded` (the child's state watch and exit facts are the
//! source); the `unset` codes (`kb_absent`, `carrier_pending`, the `codeagent` row's); Layer 7's
//! `gateway` codes (no `error` code in v1, §0). A row in `error` with a code outside the table
//! opens nothing ([`of_service_row`] answers `None`).
//!
//! `tests/heal_playbooks.rs` (`vocabulary`) scans `services/mod.rs` for every reason literal a row
//! constructor publishes in state `error` and fails on one this table lacks, on a row no
//! constructor publishes any more, or on a row state it cannot follow back to a literal; it
//! renders the table into `0-knowledge/_meta/playbook-vocabulary.toml` (one `[[row]]` per entry,
//! sorted; `UPDATE_PLAYBOOK_VOCABULARY=1` rewrites it) and fails while the committed file
//! differs, in CI too. The playbook lint in `build.py` reads that file (row K1), so the loader and
//! the lint cannot disagree on a subject or a code.
//!
//! Static data only (§1 rule 1): one table in the binary's read-only data and two lookups over it;
//! nothing is constructed, read, opened or spawned.

use crate::daemon::services::ServiceId::{self, Agent, Fabric, Kb, Meshtastic, Model, Radio};

/// The subject wildcard of a `[match]`: every component (§6.1 "`any` for a subject wildcard";
/// `repair-any-child-exit.md`).
pub const ANY: &str = "any";

/// The reason of an unrequested exit of a supervised child or a component runner: one reason
/// whatever the code or signal, which stay in the `exit` evidence for `exit_is` (§3.4; O4).
pub const CHILD_EXIT: &str = "child_exit";

/// One row of the compiled vocabulary: a fingerprint `(layer, subject, reason code)` (§3.4, §3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VocabRow {
    /// The layer, 0 to 9, a number (§6.2): the subject's; a supervised child's is its service's
    /// (§3.4), so a subject has one layer.
    pub layer: u8,
    /// A service id, [`ANY`] component, or a fixed daemon subject: `111d`, `updater`, `identity`,
    /// `storage`, `heal` (§3.4).
    pub subject: &'static str,
    /// The reason code.
    pub reason: &'static str,
    /// The service row whose state `error` with this reason code opens the fingerprint (§3.4 rule
    /// 1); `None` when no service row is its source: an exit fact, a start fact, Layer 8's storage
    /// state or Layer 9's own.
    pub service_row: Option<ServiceId>,
}

const fn row(
    layer: u8,
    subject: &'static str,
    reason: &'static str,
    service_row: Option<ServiceId>,
) -> VocabRow {
    VocabRow {
        layer,
        subject,
        reason,
        service_row,
    }
}

/// The table (§3.4's "Subject · reason codes" column), sorted by `(layer, subject, reason)` in
/// byte order, one row per fingerprint.
pub static ROWS: &[VocabRow] = &[
    // 0: the knowledge base row in `error` (`kb_absent` is `unset`)
    row(0, "kb", "verify_failed", Some(Kb)),
    // 1: §3.3's start facts and rule 7; §12.6's APK check
    row(1, "111d", "apk_outdated", None),
    row(1, "111d", "killed_by_os", None),
    row(1, "111d", "unclean_boot", None),
    row(1, "111d", "unclean_exit", None),
    row(1, "111d", "watchdog_kill", None),
    row(1, "111d", "witness_absent", None),
    row(1, "updater", "release_reverted_at_start", None),
    row(1, "updater", "update_reverted", None),
    // 2: the `fabric` row's identity codes (and `health.identity`), never a Layer 5 source
    row(2, "identity", "no_identity", Some(Fabric)),
    row(2, "identity", "restore_pending", Some(Fabric)),
    // 3: the agent and the model: their exits and the row codes that are not child codes
    row(3, "agent", CHILD_EXIT, None),
    row(3, "agent", "health_503", Some(Agent)),
    row(3, "agent", "health_error", Some(Agent)),
    row(3, "agent", "health_unreachable", Some(Agent)),
    row(3, "model", CHILD_EXIT, None),
    row(3, "model", "health_503", Some(Model)),
    row(3, "model", "health_error", Some(Model)),
    row(3, "model", "health_unreachable", Some(Model)),
    // 5: the fabric (a full store is this row's only, L8 §13.4) and meshmud
    row(5, "fabric", "store_failed", Some(Fabric)),
    row(5, "fabric", "store_full", Some(Fabric)),
    row(5, "meshtastic", "binary_absent", Some(Meshtastic)),
    row(5, "meshtastic", CHILD_EXIT, None),
    row(5, "meshtastic", "port_busy", Some(Meshtastic)),
    row(5, "meshtastic", "presence_unavailable", Some(Meshtastic)),
    row(5, "meshtastic", "uplink_lost", Some(Meshtastic)),
    // 6: every component's exits, the radio stack, rnsd's exits
    row(6, ANY, CHILD_EXIT, None),
    row(6, "radio", "stack_down", Some(Radio)),
    row(6, "transport", CHILD_EXIT, None),
    // 8: Layer 8's typed storage state
    row(8, "storage", "disk_low", None),
    // 9: a revert that failed; the node-state store unopenable (J2)
    row(9, "heal", "revert_failed", None),
    row(9, "heal", "store_unavailable", None),
];

/// The row of `(subject, reason)`, if the table holds it; [`ANY`] is a subject here. A subject has
/// one layer, so the pair is the key: a `[match]` names a row when this finds it at its `layer`
/// (§6.2).
pub fn lookup(subject: &str, reason: &str) -> Option<&'static VocabRow> {
    ROWS.iter()
        .find(|r| r.subject == subject && r.reason == reason)
}

/// The fingerprint a service row opens while it is in state `error` with `reason` (§3.4 rule 1;
/// the observer's key): `fabric` with `no_identity` opens `(2, identity, no_identity)`. `None` for
/// a child code, an `unset` code or any code outside the table: those open nothing. One row at
/// most matches: the unit tests keep `(service_row, reason)` unique (§3.4 "one source per
/// failure"; the `fabric` row sources two subjects, `fabric` and `identity`).
pub fn of_service_row(id: ServiceId, reason: &str) -> Option<&'static VocabRow> {
    ROWS.iter()
        .find(|r| r.service_row == Some(id) && r.reason == reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    /// §6.2: a subject or a reason is lower case, digits and `_`.
    fn token_ok(s: &str) -> bool {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    }

    /// §12.1 caps a fingerprint's subject and reason at 24 B each on the wire (`AGENT_OP` key 1).
    const WIRE_CAP: usize = 24;

    /// The reasons §3.3 and §3.4 name that are longer than [`WIRE_CAP`]:
    /// `release_reverted_at_start` (25 B). A start fact, so never delegated by itself (§3.3 rule
    /// 5); the conflict with §12.1 is open for the lead (a lock-amendment candidate), pinned here
    /// so no other code joins it.
    const OVER_WIRE_CAP: &[&str] = &["release_reverted_at_start"];

    /// The table's rules, one message per break: sorted, one row per fingerprint; layers 0 to 9;
    /// tokens (§6.2) within §12.1's cap; one layer per subject (§3.4); one rendered filename per
    /// row (§6.1); a service row sources its own subject (`identity` the `fabric` row's) and never
    /// `child_exit`; and a service row's code opens one fingerprint (§3.4 "one source per
    /// failure"): `fabric` sources two subjects, and [`of_service_row`] answers the first of two
    /// rows, so a second would leave the other unreachable with every other rule kept.
    fn problems(rows: &[VocabRow]) -> Vec<String> {
        let mut out = Vec::new();
        for pair in rows.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            if (a.layer, a.subject, a.reason) >= (b.layer, b.subject, b.reason) {
                out.push(format!("not sorted or duplicated: {a:?} then {b:?}"));
            }
        }
        let mut layer: BTreeMap<&str, u8> = BTreeMap::new();
        let mut names = BTreeSet::new();
        let mut sources: BTreeMap<(&str, &str), &VocabRow> = BTreeMap::new();
        for r in rows {
            if r.layer > 9 {
                out.push(format!("layer out of 0-9: {r:?}"));
            }
            if !token_ok(r.subject) || r.subject.len() > WIRE_CAP {
                out.push(format!("subject not a token within §12.1's cap: {r:?}"));
            }
            if !token_ok(r.reason)
                || (r.reason.len() > WIRE_CAP && !OVER_WIRE_CAP.contains(&r.reason))
            {
                out.push(format!("reason not a token within §12.1's cap: {r:?}"));
            }
            if *layer.entry(r.subject).or_insert(r.layer) != r.layer {
                out.push(format!("{} has two layers", r.subject));
            }
            // §6.1: `repair-<subject>-<reason>.md`, `_` written as `-`; N06 renders, never splits
            let name = format!(
                "repair-{}-{}.md",
                r.subject.replace('_', "-"),
                r.reason.replace('_', "-")
            );
            if !names.insert(name.clone()) {
                out.push(format!("two rows render {name}"));
            }
            let Some(id) = r.service_row else { continue };
            let own = if r.subject == "identity" {
                Some(ServiceId::Fabric)
            } else {
                ServiceId::parse(r.subject)
            };
            if own != Some(id) || r.reason == CHILD_EXIT {
                out.push(format!(
                    "a service row sources another subject or an exit: {r:?}"
                ));
            }
            if let Some(first) = sources.insert((id.as_str(), r.reason), r) {
                out.push(format!(
                    "the `{}` row's `{}` opens two fingerprints: {first:?} and {r:?}",
                    id.as_str(),
                    r.reason
                ));
            }
        }
        out
    }

    #[test]
    fn rows_are_sorted_unique_and_well_formed() {
        let p = problems(ROWS);
        assert!(p.is_empty(), "{}", p.join("\n"));
        for reason in OVER_WIRE_CAP {
            assert!(
                reason.len() > WIRE_CAP,
                "{reason} fits: drop it from the list"
            );
            assert!(
                ROWS.iter().any(|r| r.reason == *reason),
                "{reason} is no row"
            );
        }
    }

    #[test]
    fn a_second_row_for_a_service_row_code_is_refused() {
        // `(2, identity, store_failed)` beside `(5, fabric, store_failed)`: sorted, one filename
        // each, both subjects the `fabric` row's, so only the one-source rule refuses it;
        // `of_service_row(Fabric, "store_failed")` would answer the first and never the second
        let mut rows = ROWS.to_vec();
        rows.push(row(2, "identity", "store_failed", Some(Fabric)));
        rows.sort_by_key(|r| (r.layer, r.subject, r.reason));
        let p = problems(&rows);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(
            p[0].starts_with("the `fabric` row's `store_failed` opens two fingerprints"),
            "{p:?}"
        );
    }

    #[test]
    fn lookups() {
        let kb = lookup("kb", "verify_failed").unwrap();
        assert_eq!((kb.layer, kb.service_row), (0, Some(ServiceId::Kb)));
        assert_eq!(lookup(ANY, CHILD_EXIT).unwrap().layer, 6);
        assert_eq!(lookup("model", CHILD_EXIT).unwrap().layer, 3);
        assert_eq!(lookup("kb", "kb_absent"), None);
        assert_eq!(lookup("fabric", "no_identity"), None);
        assert_eq!(lookup("gateway", "carrier_pending"), None);

        let id = of_service_row(ServiceId::Fabric, "no_identity").unwrap();
        assert_eq!((id.layer, id.subject), (2, "identity"));
        let full = of_service_row(ServiceId::Fabric, "store_full").unwrap();
        assert_eq!((full.layer, full.subject), (5, "fabric"));
        assert_eq!(of_service_row(ServiceId::Model, "child_degraded"), None);
        assert_eq!(
            of_service_row(ServiceId::Transport, "transport_degraded"),
            None
        );
        assert_eq!(of_service_row(ServiceId::Model, CHILD_EXIT), None);
        assert_eq!(of_service_row(ServiceId::Kb, "kb_absent"), None);
    }
}
