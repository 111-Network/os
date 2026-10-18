//! Accepting a `kind: message` proposal (`os/messaging/README.md` §17,
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

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f";

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
