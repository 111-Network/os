//! Relay edges (Layer 6 `6-transport/README.md` §8; §15.3): explicit, directed,
//! non-transitive copies from one `channel`/`group` conversation to another, enabled only
//! by a human who confirmed the exact warning text.
//!
//! - **Endpoints** ([`Bridge::create_edge`]): both `channel` or `group`, distinct, both on
//!   enabled networks; a DM is `400` (the store's `CHECK` backs it); a recipient list is a
//!   destination only. Without `accept_warning` the answer is `409 consent_required` with
//!   the text ([`edge_warning`]) and its SHA-256; with the hash of the text as it reads now
//!   the edge is created with that hash as `consent_hash`.
//! - **What it relays** ([`relay_pass`], once per tick per edge, at most
//!   `relay_per_tick` inputs): the store's `foreign_edge_inputs` (inbound rows of the
//!   source above `max(cursor_seq, relayed_seq)` that are not relayed copies: history is
//!   never relayed, guard 1), minus what parses as already relayed (guard 3, the marker:
//!   a human-typed marker gets no relay either; the row records `meta.not_relayed =
//!   "marker"`, the reason the UI shows), minus a key already in the edge's
//!   `foreign_seen` (guard 6, the ID map). Own echoes never become inputs (guard 2: they
//!   are dropped at ingest). Guards 4 (foreign bridges' "looks relayed" hint, default off)
//!   and 5 (two 111 bridges on one pair duplicate but cannot loop) need no code in R-B.
//! - **Consent is read live, per copy**: for every input the add-ons ([`Addon`]: the
//!   source is a default-key channel; the destination channel is uplinked; the destination
//!   account is licensed) are re-evaluated from the adapters' facts, the warning rebuilt,
//!   and its hash compared with `consent_hash`; a difference **pauses** the edge
//!   (`consent_paused`) and it resumes only on a new confirmation of the new text
//!   ([`Bridge::patch_edge`]). It never relays on with a flag.
//! - **The copy**: `[<n>/<TAG>] ` + the body, `<TAG>` the source network's. Longer than the
//!   destination's cap: `oversize = refuse` counts it (`refused`, "N too long to relay"),
//!   `split` queues numbered parts with the marker first on every part. With
//!   `honour_ok_to_mqtt` on, an origin whose `meta.ok_to_mqtt` is false is refused and
//!   counted (fail closed: whatever the destination). A copy is queued with
//!   `foreign_queue_sends` (all its parts in one call: the store advances `relayed_seq` in
//!   that transaction and refuses a second copy of the origin), then the origin's key goes
//!   into `foreign_seen`. Inputs not relayed advance `relayed_seq` past them; a shed (store
//!   prune 2b) advances it too, so nothing skipped or shed becomes relayable again.
//! - The HAM confirmation and the uplink switch gate every send on the destination, relay
//!   copies included, at handoff ([`super::state`]); edge-only changes publish the SSE
//!   `bridge` snapshot ([`Bridge::publish_edges`]).

use super::core::{is_list_conv, Bridge, BridgeError};
use super::{ingest, marker, AdapterFacts, SizeMode};
use crate::daemon::components::consent::ConsentRequired;
use crate::daemon::fabric::json::{self as fjson, EdgeLive};
use mesh111::fabric::{FabricError, ForeignConsent, ForeignEdgeNew, ForeignMeta, ForeignQueue};
use mesh111::store::{
    EdgeCounter, EdgeOversize, EdgeRow, ForeignConversationRow, ForeignKind, ForeignMessageRow,
};
use serde::Deserialize;
use serde_json::{json, Value};

/// A live consent add-on (§8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Addon {
    DefaultKeySource,
    UplinkedDestination,
    LicensedDestination,
}

impl Addon {
    /// The contract's add-on code (`ConsentRequired.addons`).
    pub fn code(self) -> &'static str {
        match self {
            Addon::DefaultKeySource => "default_key_source",
            Addon::UplinkedDestination => "uplinked_destination",
            Addon::LicensedDestination => "licensed_destination",
        }
    }

    fn text(self) -> &'static str {
        match self {
            Addon::DefaultKeySource => {
                "The source is a default-key channel: anyone can read it; this copies public traffic."
            }
            Addon::UplinkedDestination => {
                "The destination channel is published to the internet by its board (MQTT uplink): the copies are published there too."
            }
            Addon::LicensedDestination => {
                "The destination account is amateur-licensed: third-party traffic is the licensee's fact."
            }
        }
    }
}

fn kind_word(k: ForeignKind) -> &'static str {
    match k {
        ForeignKind::Channel => "channel",
        ForeignKind::Dm => "DM",
        ForeignKind::Group => "group",
    }
}

/// How an endpoint is named in the warning: network label, kind and native id (stable for
/// the conversation's life; a title can be renamed, so it is not in the hashed text).
fn describe(label: &str, c: &ForeignConversationRow) -> String {
    if is_list_conv(c) {
        let id = c.native_id.trim_start_matches(super::core::LIST_PREFIX);
        return format!("the {label} recipient list {id}");
    }
    format!("{label} {} {}", kind_word(c.kind), c.native_id)
}

/// The §8 warning: "Everyone in `<src>` will have their messages copied to `<dst>`. They
/// see the `[name/TAG]` prefix only if they read `<dst>`." plus the add-ons in order.
pub fn edge_warning(src: &str, dst: &str, tag: &str, addons: &[Addon]) -> String {
    let mut t = format!(
        "Everyone in {src} will have their messages copied to {dst}. They see the [name/{tag}] \
         prefix only if they read {dst}."
    );
    for a in addons {
        t.push(' ');
        t.push_str(a.text());
    }
    t
}

/// The add-ons true now for an edge from `src` to `dst` (facts read live).
fn addons(
    src: &ForeignConversationRow,
    src_facts: &AdapterFacts,
    dst: &ForeignConversationRow,
    dst_facts: &AdapterFacts,
) -> Vec<Addon> {
    let mut out = Vec::new();
    if src.kind == ForeignKind::Channel
        && src_facts
            .channel(&src.account, &src.native_id)
            .is_some_and(|c| c.default_key)
    {
        out.push(Addon::DefaultKeySource);
    }
    if dst.kind == ForeignKind::Channel
        && dst_facts
            .channel(&dst.account, &dst.native_id)
            .is_some_and(|c| c.uplink)
    {
        out.push(Addon::UplinkedDestination);
    }
    if dst_facts.account(&dst.account).is_some_and(|a| a.licensed) {
        out.push(Addon::LicensedDestination);
    }
    out
}

/// `POST /bridge/edges` options (`BridgeEdgeCreate` without the endpoints).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct EdgeOpts {
    #[serde(default)]
    pub honour_ok_to_mqtt: bool,
    #[serde(default)]
    pub oversize: SizeMode,
}

/// `PATCH /bridge/edges/{id}` (`BridgeEdgePatch`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgePatch {
    #[serde(default)]
    pub paused: Option<bool>,
    #[serde(default)]
    pub honour_ok_to_mqtt: Option<bool>,
    #[serde(default)]
    pub oversize: Option<SizeMode>,
    #[serde(default)]
    pub accept_warning: Option<String>,
}

/// The live warning of an edge: `(text, hash, addons)`.
struct Warning {
    text: String,
    hash: String,
    addons: Vec<Addon>,
}

impl Warning {
    fn consent(&self, action: &'static str, subject: String) -> ConsentRequired {
        ConsentRequired {
            action,
            subject,
            warning_text: self.text.clone(),
            hash: self.hash.clone(),
            addons: self.addons.iter().map(|a| a.code().to_string()).collect(),
        }
    }
}

impl Bridge {
    /// The warning of an edge from `src` to `dst` as it reads now; `None` while either
    /// network is not enabled (its facts cannot be read).
    fn live_warning(
        &self,
        src: &ForeignConversationRow,
        dst: &ForeignConversationRow,
    ) -> Option<Warning> {
        let sf = self.facts_of(&src.network)?;
        let df = self.facts_of(&dst.network)?;
        let tag = self.tag_of(&src.network)?;
        let addons = addons(src, &sf, dst, &df);
        let text = edge_warning(
            &describe(&self.label_of(&src.network), src),
            &describe(&self.label_of(&dst.network), dst),
            &tag,
            &addons,
        );
        let hash = super::settings::sha256_hex(&text);
        Some(Warning { text, hash, addons })
    }

    fn edge_json(
        &self,
        e: &EdgeRow,
        src: Option<&ForeignConversationRow>,
        dst: Option<&ForeignConversationRow>,
    ) -> Value {
        let adapters_enabled = match (src, dst) {
            (Some(s), Some(d)) => {
                self.attached(&s.network).is_some() && self.attached(&d.network).is_some()
            }
            _ => false,
        };
        let consent_paused = e.paused
            && (super::core::lock_set(&self.consent_paused).contains(&e.id)
                || match (src, dst) {
                    (Some(s), Some(d)) => self
                        .live_warning(s, d)
                        .is_some_and(|w| w.hash != e.consent_hash),
                    _ => false,
                });
        fjson::edge(
            e,
            &EdgeLive {
                adapters_enabled,
                pause_reason: e.paused.then_some(if consent_paused {
                    "consent_paused"
                } else {
                    "user"
                }),
            },
        )
    }

    /// `GET /bridge/edges`: `{schema, edges: [BridgeEdge]}`.
    pub async fn edges(&self) -> Result<Value, BridgeError> {
        let f = self.fab()?;
        let edges = f.foreign_edge_list().await?;
        let convs = f.foreign_conversations(None).await?;
        let find = |id: i64| convs.iter().find(|c| c.id == id);
        let list: Vec<Value> = edges
            .iter()
            .map(|e| self.edge_json(e, find(e.src_conv), find(e.dst_conv)))
            .collect();
        Ok(json!({ "schema": 1, "edges": list }))
    }

    /// `GET /bridge/edges/{id}`.
    pub async fn edge(&self, id: i64) -> Result<Value, BridgeError> {
        let f = self.fab()?;
        let e = match f.foreign_edge_get(id).await {
            Ok(e) => e,
            Err(FabricError::NotFound) => return Err(BridgeError::NotFound("edge")),
            Err(e) => return Err(e.into()),
        };
        let src = self.conv_row(e.src_conv).await.ok();
        let dst = self.conv_row(e.dst_conv).await.ok();
        Ok(self.edge_json(&e, src.as_ref(), dst.as_ref()))
    }

    /// Publish the `bridge` frame with the edges snapshot (edge-only changes emit no
    /// fabric event; this is the core's).
    pub async fn publish_edges(&self) {
        if let Ok(v) = self.edges().await {
            self.publish("bridge", &json!({ "edges": v["edges"] }));
        }
    }

    /// The consent text of the edge `id` as it reads now (the UI shows it on a
    /// `consent_paused` edge); `None` while an endpoint's network is not enabled.
    pub async fn edge_consent(&self, id: i64) -> Result<Option<ConsentRequired>, BridgeError> {
        let e = self.fab()?.foreign_edge_get(id).await?;
        let src = self.conv_row(e.src_conv).await?;
        let dst = self.conv_row(e.dst_conv).await?;
        Ok(self
            .live_warning(&src, &dst)
            .map(|w| w.consent("edge_resume", fjson::be_id(id))))
    }

    /// `POST /bridge/edges` (module doc). `by` is the session that confirms (`loopback` or
    /// a paired session's label; never the agent's bearer — the route's gate).
    pub async fn create_edge(
        &self,
        src: i64,
        dst: i64,
        opts: EdgeOpts,
        accept_hash: Option<&str>,
        by: &str,
    ) -> Result<Value, BridgeError> {
        if src == dst {
            return Err(BridgeError::BadRequest(
                "an edge joins two different conversations".into(),
            ));
        }
        let s = self.conv_row(src).await?;
        let d = self.conv_row(dst).await?;
        if s.kind == ForeignKind::Dm || d.kind == ForeignKind::Dm {
            return Err(BridgeError::BadRequest(
                "a DM is never an edge endpoint".into(),
            ));
        }
        if is_list_conv(&s) {
            return Err(BridgeError::BadRequest(
                "a recipient list is a destination only".into(),
            ));
        }
        if is_list_conv(&d) && self.list_of(&d).is_none() {
            return Err(BridgeError::NotFound("recipient list"));
        }
        for c in [&s, &d] {
            if self.attached(&c.network).is_none() {
                return Err(BridgeError::Unavailable(format!(
                    "Enable {} first.",
                    self.label_of(&c.network)
                )));
            }
        }
        let w = self.live_warning(&s, &d).ok_or_else(|| {
            BridgeError::Unavailable("an endpoint's network is not enabled".into())
        })?;
        if accept_hash != Some(w.hash.as_str()) {
            return Err(BridgeError::Consent(w.consent(
                "edge_enable",
                format!("{}>{}", fjson::fc_id(src), fjson::fc_id(dst)),
            )));
        }
        let row = self
            .fab()?
            .foreign_edge_create(ForeignEdgeNew {
                src_conv: src,
                dst_conv: dst,
                honour_ok_to_mqtt: opts.honour_ok_to_mqtt,
                oversize: match opts.oversize {
                    SizeMode::Refuse => EdgeOversize::Refuse,
                    SizeMode::Split => EdgeOversize::Split,
                },
                consent: ForeignConsent {
                    consent_hash: w.hash,
                    enabled_by: by.to_string(),
                },
            })
            .await?;
        self.publish_edges().await;
        Ok(self.edge_json(&row, Some(&s), Some(&d)))
    }

    /// `PATCH /bridge/edges/{id}`: pause (always allowed) or resume (a `consent_paused`
    /// edge needs the hash of the **new** text; a user pause resumes on the recorded
    /// text), and the two options `honour_ok_to_mqtt` and `oversize` (neither is part of
    /// the consent text, so neither needs a confirmation; C1: `Store::edge_set_options`).
    /// A resume that needs a confirmation is checked before any option is written, so a
    /// `409 consent_required` changes nothing.
    pub async fn patch_edge(&self, id: i64, p: &EdgePatch, by: &str) -> Result<Value, BridgeError> {
        let f = self.fab()?;
        let e = match f.foreign_edge_get(id).await {
            Ok(e) => e,
            Err(FabricError::NotFound) => return Err(BridgeError::NotFound("edge")),
            Err(e) => return Err(e.into()),
        };
        let stored_mode = match e.oversize {
            EdgeOversize::Refuse => SizeMode::Refuse,
            EdgeOversize::Split => SizeMode::Split,
        };
        let src = self.conv_row(e.src_conv).await?;
        let dst = self.conv_row(e.dst_conv).await?;
        // the resume's confirmation first: a refused resume writes nothing
        let resume = match p.paused {
            Some(false) if e.paused => {
                let w = self.live_warning(&src, &dst).ok_or_else(|| {
                    BridgeError::Unavailable("an endpoint's network is not enabled".into())
                })?;
                let ok = match p.accept_warning.as_deref() {
                    Some(h) => h == w.hash,
                    None => {
                        w.hash == e.consent_hash
                            && !super::core::lock_set(&self.consent_paused).contains(&id)
                    }
                };
                if !ok {
                    return Err(BridgeError::Consent(
                        w.consent("edge_resume", fjson::be_id(id)),
                    ));
                }
                Some(w)
            }
            _ => None,
        };
        let honour = p.honour_ok_to_mqtt.unwrap_or(e.honour_ok_to_mqtt);
        let mode = p.oversize.unwrap_or(stored_mode);
        if honour != e.honour_ok_to_mqtt || mode != stored_mode {
            let oversize = match mode {
                SizeMode::Refuse => EdgeOversize::Refuse,
                SizeMode::Split => EdgeOversize::Split,
            };
            f.foreign_edge_set_options(id, honour, oversize).await?;
        }
        match p.paused {
            Some(true) => {
                f.foreign_edge_set_paused(id, true, None).await?;
            }
            Some(false) if e.paused => {
                let Some(w) = resume else {
                    return Err(BridgeError::Unavailable(
                        "an endpoint's network is not enabled".into(),
                    ));
                };
                f.foreign_edge_set_paused(
                    id,
                    false,
                    Some(ForeignConsent {
                        consent_hash: w.hash,
                        enabled_by: by.to_string(),
                    }),
                )
                .await?;
                super::core::lock_set(&self.consent_paused).remove(&id);
            }
            _ => {}
        }
        self.publish_edges().await;
        let e = f.foreign_edge_get(id).await?;
        Ok(self.edge_json(&e, Some(&src), Some(&dst)))
    }

    /// `DELETE /bridge/edges/{id}` (the store drops its copies never handed to a carrier).
    pub async fn delete_edge(&self, id: i64) -> Result<bool, BridgeError> {
        let gone = self.fab()?.foreign_edge_delete(id).await?;
        super::core::lock_set(&self.consent_paused).remove(&id);
        if gone {
            self.publish_edges().await;
        }
        Ok(gone)
    }

    /// Relay the inputs of one edge (module doc). Returns the copies queued.
    async fn relay_edge(&self, e: &EdgeRow, tags: &[&str]) -> Result<usize, BridgeError> {
        let f = self.fab()?;
        let src = self.conv_row(e.src_conv).await?;
        let dst = self.conv_row(e.dst_conv).await?;
        if self.attached(&src.network).is_none() || self.attached(&dst.network).is_none() {
            return Ok(0);
        }
        let inputs = f.foreign_edge_inputs(e.id, self.cfg.relay_per_tick).await?;
        if inputs.is_empty() {
            return Ok(0);
        }
        let src_tag = self.tag_of(&src.network).unwrap_or_else(|| "?".into());
        let mode = match e.oversize {
            EdgeOversize::Refuse => SizeMode::Refuse,
            EdgeOversize::Split => SizeMode::Split,
        };
        let mut skip_to: Option<u64> = None;
        let mut queued = 0usize;
        let mut refused = 0u64;
        for input in &inputs {
            // guard 3: already relayed (by any 111 node), or a human-typed marker; the
            // reason is recorded on the row once (`meta.not_relayed = "marker"`) so the UI
            // can say why it was not relayed
            if marker::is_relayed(&input.body, tags) {
                if !input.meta.contains("\"not_relayed\"") {
                    let _ = f
                        .foreign_set_meta(
                            input.id,
                            "not_relayed",
                            Some(ForeignMeta::Str("marker".into())),
                        )
                        .await;
                }
                skip_to = Some(input.local_seq);
                continue;
            }
            // guard 6: the ID map
            if let Some(native) = &input.native_msg_id {
                if f.foreign_seen_contains(Some(e.id), e.src_conv, &input.sender_native, native)
                    .await?
                {
                    skip_to = Some(input.local_seq);
                    continue;
                }
            }
            // consent, live per copy
            let Some(w) = self.live_warning(&src, &dst) else {
                break;
            };
            if w.hash != e.consent_hash {
                if let Some(s) = skip_to.take() {
                    f.foreign_edge_advance_relayed(e.id, s).await?;
                }
                if refused > 0 {
                    f.foreign_edge_count(e.id, EdgeCounter::Refused, refused)
                        .await?;
                }
                f.foreign_edge_set_paused(e.id, true, None).await?;
                super::core::lock_set(&self.consent_paused).insert(e.id);
                self.publish_edges().await;
                return Ok(queued);
            }
            if e.honour_ok_to_mqtt && ok_to_mqtt_absent(input) {
                refused += 1;
                skip_to = Some(input.local_seq);
                continue;
            }
            let name = input.sender_name.as_deref().unwrap_or("");
            let lead = marker::marker(name, &src_tag);
            let plan = self.plan_copy(&dst, &input.body, mode, &lead);
            if plan.parts.is_empty() {
                refused += 1;
                skip_to = Some(input.local_seq);
                continue;
            }
            let n = plan.parts.len() as u32;
            let sends: Vec<ForeignQueue> = plan
                .parts
                .into_iter()
                .enumerate()
                .map(|(i, text)| {
                    ForeignQueue::relay_copy(dst.id, text, e.id, input.id, i as u32 + 1, n)
                })
                .collect();
            match self.handle.foreign_queue_sends(sends).await {
                Ok(_) => {
                    queued += 1;
                    skip_to = None;
                    if let Some(native) = &input.native_msg_id {
                        f.foreign_seen_insert(
                            e.id,
                            e.src_conv,
                            &input.sender_native,
                            native,
                            input.local_seq,
                        )
                        .await?;
                    }
                }
                Err(FabricError::Invalid(why)) => {
                    // the edge changed under us (paused, disabled) or the origin is
                    // already at the mark: stop this edge for this tick
                    eprintln!(
                        "111 bridge: edge {}: copy not queued: {why}",
                        fjson::be_id(e.id)
                    );
                    break;
                }
                Err(err) => return Err(err.into()),
            }
        }
        if let Some(s) = skip_to {
            f.foreign_edge_advance_relayed(e.id, s).await?;
        }
        if refused > 0 {
            f.foreign_edge_count(e.id, EdgeCounter::Refused, refused)
                .await?;
        }
        Ok(queued)
    }
}

/// `honour_ok_to_mqtt`: the origin says it may not leave the mesh (`meta.ok_to_mqtt` is
/// `false`). Absent means the adapter reported nothing: relayed.
fn ok_to_mqtt_absent(m: &ForeignMessageRow) -> bool {
    serde_json::from_str::<Value>(&m.meta)
        .ok()
        .and_then(|v| v.get("ok_to_mqtt").and_then(Value::as_bool))
        == Some(false)
}

/// One relay pass over every enabled, unpaused edge whose two networks are enabled.
pub(super) async fn relay_pass(b: &Bridge) {
    let Ok(f) = b.fab() else { return };
    let Ok(edges) = f.foreign_edge_list().await else {
        return;
    };
    if edges.is_empty() {
        return;
    }
    let tags = b.tags();
    let tag_refs: Vec<&str> = tags.iter().map(String::as_str).collect();
    for e in edges.iter().filter(|e| e.enabled && !e.paused) {
        if let Err(err) = b.relay_edge(e, &tag_refs).await {
            eprintln!("111 bridge: edge {}: {err}", fjson::be_id(e.id));
        }
    }
}

/// The marker name an inbound row's copy would carry (tests, routes).
pub fn copy_marker(sender_name: Option<&str>, tag: &str) -> String {
    marker::marker(
        ingest::name(sender_name.unwrap_or(""))
            .as_deref()
            .unwrap_or(""),
        tag,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_warning_is_the_spec_text_plus_the_live_add_ons() {
        let t = edge_warning("Meshtastic channel ch:A:1", "Stub group g", "M", &[]);
        assert_eq!(
            t,
            "Everyone in Meshtastic channel ch:A:1 will have their messages copied to Stub group g. \
             They see the [name/M] prefix only if they read Stub group g."
        );
        let t2 = edge_warning(
            "a",
            "b",
            "M",
            &[Addon::DefaultKeySource, Addon::LicensedDestination],
        );
        assert!(t2.contains("anyone can read it; this copies public traffic"));
        assert!(t2.ends_with("third-party traffic is the licensee's fact."));
        assert_ne!(
            super::super::settings::sha256_hex(&t2),
            super::super::settings::sha256_hex(&edge_warning(
                "a",
                "b",
                "M",
                &[Addon::DefaultKeySource]
            )),
            "a changed add-on set changes the hash"
        );
        assert_eq!(Addon::UplinkedDestination.code(), "uplinked_destination");
    }

    #[test]
    fn ok_to_mqtt_is_read_from_meta() {
        let mut m = ForeignMessageRow {
            id: 1,
            conv: 1,
            dir: mesh111::store::ForeignDir::In,
            sender_native: "!a".into(),
            sender_name: None,
            auth: mesh111::store::ForeignAuth::ChannelKey,
            native_msg_id: None,
            body: "x".into(),
            local_seq: 1,
            claim_ts: None,
            state: mesh111::store::ForeignState::Delivered,
            evidence: None,
            reason_code: None,
            group_id: None,
            part_k: 1,
            part_n: 1,
            origin_msg: None,
            edge_id: None,
            meta: r#"{"ok_to_mqtt":false}"#.into(),
            handoff_pending: false,
        };
        assert!(ok_to_mqtt_absent(&m));
        m.meta = r#"{"ok_to_mqtt":true}"#.into();
        assert!(!ok_to_mqtt_absent(&m));
        m.meta = "{}".into();
        assert!(!ok_to_mqtt_absent(&m));
        assert_eq!(copy_marker(Some("W1AW/7"), "M"), "[W1AW7/M] ");
    }
}
