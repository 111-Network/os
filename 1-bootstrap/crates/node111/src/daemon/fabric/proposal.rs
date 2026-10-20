//! Accepting a `kind: message` proposal (`os/50-messaging/README.md` §17,
//! §18 rule 9: the agent never sends — it proposes, the human accepts in the
//! dashboard, and `111d` sends). The proposal file is untrusted agent output;
//! acceptance checks, in order:
//!
//! 1. the body is exactly the A1 contract
//!    `"to: " + <64 lowercase hex> + "\n\n" + <text>` — the text is taken
//!    verbatim (never trimmed or rewritten) and must not be empty;
//! 2. the proposal's node-id stamp equals this node (a proposal written for
//!    another node — a copied agent dir — is never applied here);
//! 3. the recipient is an accepted contact (a pending message request is
//!    not; a `node1…` contact still waiting for its card is — the message
//!    then waits `queued`, `waiting_for_key`, exactly as a dashboard send);
//! 4. the fabric queues the message (its store commits it, `synchronous =
//!    FULL`, before the call returns — §4).
//!
//! The caller (`proposals.rs`) runs checks 1–3 ([`prepare`]), records the
//! acceptance as `applied: {pending: true}`, then sends ([`send`]) and
//! finalises the record with the message id — so a crash between the send
//! and the record leaves a visibly half-applied proposal, never a second
//! send after a restart; a refused send removes the pending record (the
//! proposal stays open).
//!
//! **Layer 6 kinds** (`os/60-transport/README.md` §9, §8, §12, §13; the body
//! contracts B5 writes byte for byte, `30-intelligence/README.md` "Appendix —
//! Layer 6 tools"). Each parser is strict and total over untrusted text: a
//! body that does not follow its contract is a [`Refusal::Body`] (`502
//! upstream_error`, the proposal stays open, the user may dismiss it):
//!
//! - `bridge_message` ([`parse_bridge_message`]): one `to: <fc-id>` line per
//!   target (a 111 contact as `to: <64 lowercase hex>`), 1..=16, no target
//!   twice, then a blank line, then the text **verbatim** (never trimmed),
//!   non-empty. Conversations are named by `fc-` id, never by native id.
//! - `bridge_edge` ([`parse_bridge_edge`]): `from: <fc-id>` and `to: <fc-id>`,
//!   then (optionally) a blank line and the agent's reason, which is shown,
//!   never used.
//! - `device` bodies are the device core's (`devices::applier::ApplierRequest::
//!   from_body`, C4): one parser for the closed vocabulary, beside its applier.

use super::FabricHandle;
use crate::daemon::dashboard::api_v1::Problem;
use axum::http::StatusCode;
use id111::keyset::NodeId;
use mesh111::fabric::FabricError;

/// The A1 body prefix.
pub const TO_PREFIX: &str = "to: ";

/// Why a message proposal cannot be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The body does not follow the A1 contract.
    Body(&'static str),
    /// The node-id stamp is missing or names another node.
    Stamp,
    /// The recipient is not a contact.
    NotContact,
    /// The recipient is a pending message request.
    Request,
}

/// Parse the A1 body: `(to, text)`.
pub fn parse_body(body: &str) -> Result<(NodeId, &str), Refusal> {
    let rest = body
        .strip_prefix(TO_PREFIX)
        .ok_or(Refusal::Body("the body does not start with \"to: \""))?;
    let hex = rest
        .get(..64)
        .ok_or(Refusal::Body("the recipient is not 64 lowercase hex"))?;
    let to = super::json::parse_node_id(hex)
        .ok_or(Refusal::Body("the recipient is not 64 lowercase hex"))?;
    let text = rest[64..].strip_prefix("\n\n").ok_or(Refusal::Body(
        "the recipient line is not followed by a blank line",
    ))?;
    if text.is_empty() {
        return Err(Refusal::Body("the message text is empty"));
    }
    Ok((to, text))
}

/// Check the stamp against this node.
pub fn check_stamp(stamp: Option<&str>, own: &NodeId) -> Result<(), Refusal> {
    match stamp {
        Some(s) if s == super::json::hex(own) => Ok(()),
        _ => Err(Refusal::Stamp),
    }
}

impl Refusal {
    /// The problem the decision route answers (the proposal stays open; the
    /// user may dismiss it).
    pub fn problem(&self) -> Problem {
        match self {
            Refusal::Body(why) => Problem::new(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                "The local AI agent wrote a message proposal that cannot be sent.",
            )
            .detail(format!("{why}; dismiss it and ask the agent again.")),
            Refusal::Stamp => Problem::new(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                "This message proposal was written for another node.",
            )
            .detail("Its node-id stamp does not name this node; it is never sent from here."),
            Refusal::NotContact => Problem::new(
                StatusCode::CONFLICT,
                "bad_request",
                "The recipient is not a contact.",
            )
            .detail("Add them in Contacts first, then accept the proposal again."),
            Refusal::Request => Problem::new(
                StatusCode::CONFLICT,
                "bad_request",
                "The recipient is a pending message request.",
            )
            .detail("Accept their request in Messages first, then accept the proposal again."),
        }
    }
}

/// Checks 1–3 of an accepted message proposal (nothing is written): the
/// recipient and the verbatim text to send.
pub async fn prepare(
    handle: &FabricHandle,
    node_id_stamp: Option<&str>,
    body: &str,
) -> Result<(NodeId, String), Problem> {
    let (to, text) = parse_body(body).map_err(|r| r.problem())?;
    let fabric = handle.fabric().ok_or_else(super::not_running)?;
    check_stamp(node_id_stamp, &fabric.node_id()).map_err(|r| r.problem())?;
    match fabric.contact(to).await {
        Ok(c) if c.request => return Err(Refusal::Request.problem()),
        Ok(_) => {}
        Err(FabricError::NotFound) => return Err(Refusal::NotContact.problem()),
        Err(e) => return Err(super::problem(&e)),
    }
    Ok((to, text.to_string()))
}

/// Check 4: queue the prepared message. Returns its store id (its store
/// commit is done).
pub async fn send(handle: &FabricHandle, to: NodeId, text: &str) -> Result<i64, Problem> {
    handle.send(to, text).await.map_err(|e| match e {
        FabricError::NotFound => Refusal::NotContact.problem(),
        FabricError::TooLarge(_) | FabricError::Invalid(_) => Problem::new(
            StatusCode::BAD_GATEWAY,
            "upstream_error",
            "The proposed message cannot be sent as written.",
        )
        .detail(e.to_string()),
        other => super::problem(&other),
    })
}

// ---------------------------------------------------------------- Layer 6 bodies

/// Targets a `bridge_message` names at most (B5's `targets` bound).
pub const MAX_BRIDGE_TARGETS: usize = 16;
/// One target of a `bridge_message` (`to:` line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeTarget {
    /// A foreign conversation (`fc-<n>`, the row id).
    Conv(i64),
    /// A 111 contact (an ordinary sealed message).
    Contact(NodeId),
}

/// Parse a `bridge_message` body: the targets in order and the verbatim text.
pub fn parse_bridge_message(body: &str) -> Result<(Vec<BridgeTarget>, &str), Refusal> {
    let mut targets: Vec<BridgeTarget> = Vec::new();
    let mut rest = body;
    loop {
        let (line, after) = rest.split_once('\n').ok_or(Refusal::Body(
            "the target lines are not followed by a blank line and the text",
        ))?;
        if line.is_empty() {
            if targets.is_empty() {
                return Err(Refusal::Body("the body names no target"));
            }
            rest = after;
            break;
        }
        let v = line
            .strip_prefix(TO_PREFIX)
            .ok_or(Refusal::Body("a target line does not read \"to: <fc-id>\""))?;
        let t = if let Some(id) = super::json::parse_fc_id(v) {
            BridgeTarget::Conv(id)
        } else if let Some(n) = super::json::parse_node_id(v) {
            BridgeTarget::Contact(n)
        } else {
            return Err(Refusal::Body(
                "a target is neither a conversation (fc-…) nor a 111 contact (64 hex)",
            ));
        };
        if targets.contains(&t) {
            return Err(Refusal::Body("a target is named twice"));
        }
        targets.push(t);
        if targets.len() > MAX_BRIDGE_TARGETS {
            return Err(Refusal::Body("more than 16 targets"));
        }
        rest = after;
    }
    if rest.is_empty() {
        return Err(Refusal::Body("the message text is empty"));
    }
    Ok((targets, rest))
}

/// Parse a `bridge_edge` body: `(from, to, why)` (row ids; `why` may be empty).
pub fn parse_bridge_edge(body: &str) -> Result<(i64, i64, &str), Refusal> {
    let bad = Refusal::Body("the body does not read \"from: <fc-id>\" then \"to: <fc-id>\"");
    let rest = body.strip_prefix("from: ").ok_or(bad.clone())?;
    let (from, rest) = rest.split_once('\n').ok_or(bad.clone())?;
    let rest = rest.strip_prefix(TO_PREFIX).ok_or(bad.clone())?;
    let (to, why) = match rest.split_once('\n') {
        None => (rest, ""),
        Some((to, after)) => (
            to,
            after.strip_prefix('\n').ok_or(Refusal::Body(
                "the edge lines are not followed by a blank line before the reason",
            ))?,
        ),
    };
    let from = super::json::parse_fc_id(from).ok_or(bad.clone())?;
    let to = super::json::parse_fc_id(to).ok_or(bad)?;
    Ok((from, to, why))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f";

    #[test]
    fn bridge_message_body_contract() {
        let (t, text) = parse_bridge_message("to: fc-3\nto: fc-12\n\nhello\n  there ").unwrap();
        assert_eq!(t, [BridgeTarget::Conv(3), BridgeTarget::Conv(12)]);
        assert_eq!(text, "hello\n  there ", "verbatim, never trimmed");
        let body = format!("to: {HEX}\nto: fc-1\n\n\n");
        let (t, text) = parse_bridge_message(&body).unwrap();
        assert_eq!(t[0], BridgeTarget::Contact(NodeId::from([0x0f; 32])));
        assert_eq!(text, "\n");
        let many: String = (1..=17).map(|i| format!("to: fc-{i}\n")).collect();
        for bad in [
            String::new(),
            "\n\nx".into(),
            "to: fc-1\n\n".into(),
            "to: fc-1\nx".into(),
            "to: fc-1\r\n\r\nx".into(),
            "to:fc-1\n\nx".into(),
            "To: fc-1\n\nx".into(),
            "to: !a1b2c3d4\n\nx".into(),
            "to: ch:LongFast\n\nx".into(),
            "to: fc-0\n\nx".into(),
            "to: fc-1\nto: fc-1\n\nx".into(),
            format!("{many}\nx"),
        ] {
            assert!(parse_bridge_message(&bad).is_err(), "{bad:?}");
        }
        let sixteen = &many[..many.len() - "to: fc-17\n".len()];
        assert!(parse_bridge_message(&format!("{sixteen}\nx")).is_ok());
    }

    #[test]
    fn bridge_edge_body_contract() {
        assert_eq!(
            parse_bridge_edge("from: fc-1\nto: fc-2\n\nbecause").unwrap(),
            (1, 2, "because")
        );
        assert_eq!(
            parse_bridge_edge("from: fc-1\nto: fc-2").unwrap(),
            (1, 2, "")
        );
        for bad in [
            "",
            "from: fc-1",
            "from: fc-1\nto: fc-2\nwhy",
            "to: fc-2\nfrom: fc-1",
            "from: !a1\nto: fc-2",
            "from: fc-1\nto: dm:!a1\n\nx",
            " from: fc-1\nto: fc-2",
        ] {
            assert!(parse_bridge_edge(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a1_body_contract() {
        let body = format!("to: {HEX}\n\nhello\n  there ");
        let (to, text) = parse_body(&body).unwrap();
        assert_eq!(to, NodeId::from([0x0f; 32]));
        assert_eq!(text, "hello\n  there ", "verbatim, never trimmed");
        let body = format!("to: {HEX}\n\n\n");
        let (_, t) = parse_body(&body).unwrap();
        assert_eq!(t, "\n");
        for bad in [
            format!("to:{HEX}\n\nx"),
            format!("To: {HEX}\n\nx"),
            format!(" to: {HEX}\n\nx"),
            format!("to: {}\n\nx", HEX.to_uppercase()),
            format!("to: {}\n\nx", &HEX[..63]),
            format!("to: {HEX}0\n\nx"),
            format!("to: {HEX}\nx"),
            format!("to: {HEX}\r\n\r\nx"),
            format!("to: {HEX}\n\n"),
            "to: node1qqqq\n\nx".to_string(),
            String::new(),
        ] {
            assert!(parse_body(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn stamp() {
        let own = NodeId::from([0x0f; 32]);
        assert!(check_stamp(Some(HEX), &own).is_ok());
        assert_eq!(check_stamp(None, &own), Err(Refusal::Stamp));
        assert_eq!(
            check_stamp(Some(&HEX.to_uppercase()), &own),
            Err(Refusal::Stamp)
        );
        assert_eq!(
            check_stamp(Some(&"1".repeat(64)), &own),
            Err(Refusal::Stamp)
        );
    }
}
