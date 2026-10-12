//! Step 9 — offer (dismissible): standard tier = small local model. v1 semantics
//! (spec + user amendment): records intent in `state/offer.json` + the
//! `/api/status` field; downloads NOTHING. Layer 3 replaces the hook. The offer
//! is written once — an existing user choice is never overwritten.

use super::context::InstallContext;

/// Write the offer stub if absent. Returns true when newly offered.
pub fn offer_standard(ctx: &InstallContext) -> anyhow::Result<bool> {
    let path = ctx.state_dir().join("offer.json");
    if path.exists() {
        return Ok(false);
    }
    let body = serde_json::json!({
        "schema": 1,
        "tier": "standard",
        "state": "offered",
        "recorded_at": time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "unknown".into()),
    });
    crate::util::fs::atomic_write(&path, body.to_string().as_bytes(), 0o644)?;
    Ok(true)
}

/// Record the user's choice (`111 config offer accept|dismiss` — T14 wiring).
pub fn record_choice(ctx: &InstallContext, accept: bool) -> anyhow::Result<()> {
    let path = ctx.state_dir().join("offer.json");
    let body = serde_json::json!({
        "schema": 1,
        "tier": "standard",
        "state": if accept { "accepted" } else { "dismissed" },
        "recorded_at": time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "unknown".into()),
    });
    crate::util::fs::atomic_write(&path, body.to_string().as_bytes(), 0o644)?;
    Ok(())
}

pub fn print_offer() {
    println!(
        "
Standard tier available: a small local model on this node (offline-first,
adds local intelligence to your 111 node). Nothing is downloaded yet —
accept later with:  sudo 111 config offer accept   (or dismiss: offer dismiss)"
    );
}
