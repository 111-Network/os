//! Compose fan-out (Layer 6 `60-transport/README.md` §9): one text into several targets as
//! one fresh send group, never a 111 message exported.
//!
//! - **Targets** ([`FanoutTarget`]): a foreign conversation of an enabled network, a
//!   recipient list (§6: one DM per member, each DM conversation created from this node's
//!   first account), or a 111 contact (an ordinary sealed message through
//!   `FabricHandle::send`, with its Layer 5 states).
//! - **Plan per target** ([`Plan`], the contract's `SendPlan`): UTF-8 bytes (with the part
//!   overhead), the cap for the target's kind and mode (licensed accounts have their own),
//!   the parts of a split, an airtime estimate from the adapter's profile (the LoRa
//!   time-on-air formula for a modem preset, KB `lora-airtime-budget`; bytes ÷ bit rate
//!   otherwise; "depends on the path" for LXMF — always labelled an estimate), warnings
//!   (adapter down, region unset, uplinked channel, parts out of order, duty cycle) and a
//!   refusal (`oversize` in refuse mode — "312 B > 232 B per packet" — or a consent gate:
//!   `ham_unconfirmed`, `uplink_off`, with the text and hash).
//! - **Split** ([`split`]): cuts at UTF-8 boundaries, prefers whitespace, numbers parts
//!   `(k/n) ` and counts that overhead (and, for a relay copy, the marker that leads every
//!   part). Truncation never happens silently. One row per part.
//! - **Send** ([`Bridge::fanout`]): validation is atomic — any refused target answers
//!   `409` with the per-target report and **nothing** is queued; adapter state (region
//!   unset, board away, queue full, no path) is not a call error: rows queue with the
//!   reason. Each row has its own state, retry and cancel ([`Bridge::cancel`]: only a row
//!   never handed to a carrier; [`Bridge::cancel_unsent`] drops a send's unsent parts and
//!   leaves the sent ones); one target's failure never cancels another; pacing is the
//!   adapter's. A direct reply ([`Bridge::send_text`]) is a one-target send group (S1).

use super::core::{is_list_conv, Bridge, BridgeError};
use super::settings::Gate;
use super::{ingest, marker, AdapterFacts, AirtimeProfile, Caps, ConvKind, SizeMode};
use crate::daemon::components::consent::ConsentRequired;
use crate::daemon::fabric::json::{self as fjson, GroupTarget};
use mesh111::fabric::{FabricError, ForeignGroupPart};
use mesh111::store::{ForeignConversationRow, ForeignKind, ForeignState, MAX_FOREIGN_BATCH};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Targets per fan-out (the contract's `maxItems`).
pub const MAX_TARGETS: usize = 64;
/// Rows one send group queues at most (the store's batch).
pub const MAX_PARTS: usize = MAX_FOREIGN_BATCH;
/// Bytes a Meshtastic packet adds around the text (16 B radio header, the `Data` envelope),
/// for the time-on-air estimate only.
const LORA_OVERHEAD: usize = 23;
/// The largest LoRa frame.
const LORA_MAX: usize = 255;
/// Meshtastic's preamble, in symbols.
const LORA_PREAMBLE: f64 = 16.0;

// ---------------------------------------------------------------- request bodies

/// A foreign target (`FanoutForeignTarget`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForeignTarget {
    pub network: String,
    pub conv: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_mode: Option<SizeMode>,
}

/// A 111 contact target (`FanoutContactTarget`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContactTarget {
    pub node_id: String,
}

/// One target of a fan-out (`FanoutTarget`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FanoutTarget {
    Foreign(ForeignTarget),
    Contact(ContactTarget),
}

/// `POST /messages/fanout[/preview]` (`FanoutRequest`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FanoutRequest {
    pub text: String,
    pub targets: Vec<FanoutTarget>,
    #[serde(default)]
    pub size_mode: Option<SizeMode>,
}

// ---------------------------------------------------------------- split

/// Cut `text` into chunks for `n` parts (the overhead of part `k` is `lead` + `(k/n) `).
fn cut_into<'a>(text: &'a str, cap: usize, lead: &str, n: usize) -> Option<Vec<&'a str>> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let k = out.len() + 1;
        let overhead = lead.len() + marker::part_prefix(k, n.max(k)).len();
        let budget = cap.checked_sub(overhead).filter(|b| *b > 0)?;
        let chunk = if rest.len() <= budget {
            rest
        } else {
            let hard = ingest::cut(rest, budget);
            if hard.is_empty() {
                return None;
            }
            match hard.char_indices().rev().find(|(_, c)| c.is_whitespace()) {
                Some((i, c)) if i + c.len_utf8() >= hard.len() / 2 => &rest[..i + c.len_utf8()],
                _ => hard,
            }
        };
        out.push(chunk);
        rest = &rest[chunk.len()..];
        if out.len() > MAX_PARTS {
            return None;
        }
    }
    Some(out)
}

/// Split `text` into numbered parts of at most `cap` bytes each, every part led by `lead`
/// (the relay marker; empty for a compose send) then `(k/n) `. Cuts at UTF-8 boundaries
/// and prefers whitespace; the chunks joined are exactly `text`. `None`: it cannot be done
/// within `cap` (the overhead leaves no room) or in at most [`MAX_PARTS`] parts.
pub fn split(text: &str, cap: usize, lead: &str) -> Option<Vec<String>> {
    let mut n = 2usize;
    loop {
        if n > MAX_PARTS {
            return None;
        }
        let chunks = cut_into(text, cap, lead, n)?;
        let m = chunks.len();
        if m <= n {
            return Some(
                chunks
                    .into_iter()
                    .enumerate()
                    .map(|(i, c)| format!("{lead}{}{c}", marker::part_prefix(i + 1, m.max(1))))
                    .collect(),
            );
        }
        n = m;
    }
}

// ---------------------------------------------------------------- airtime

/// `(spreading factor, bandwidth Hz, coding-rate denominator − 4)` of a Meshtastic modem
/// preset (the firmware's table).
pub fn lora_preset(name: &str) -> Option<(u32, u32, u32)> {
    Some(match name {
        "LongFast" | "LONG_FAST" => (11, 250_000, 1),
        "LongSlow" | "LONG_SLOW" => (12, 125_000, 4),
        "LongModerate" | "LONG_MODERATE" => (11, 125_000, 4),
        "VeryLongSlow" | "VERY_LONG_SLOW" => (12, 62_500, 4),
        "MediumSlow" | "MEDIUM_SLOW" => (10, 250_000, 1),
        "MediumFast" | "MEDIUM_FAST" => (9, 250_000, 1),
        "ShortSlow" | "SHORT_SLOW" => (8, 250_000, 1),
        "ShortFast" | "SHORT_FAST" => (7, 250_000, 1),
        "ShortTurbo" | "SHORT_TURBO" => (7, 500_000, 1),
        _ => return None,
    })
}

/// Time on air of one LoRa frame of `payload` bytes (the Semtech formula: explicit header,
/// CRC on, low-data-rate optimisation above 16 ms symbols), in milliseconds.
pub fn lora_toa_ms(sf: u32, bw_hz: u32, cr: u32, payload: usize) -> f64 {
    let ts = f64::from(1u32 << sf) / f64::from(bw_hz);
    let de = if ts > 0.016 { 1.0 } else { 0.0 };
    let sf_f = f64::from(sf);
    let t_pre = (LORA_PREAMBLE + 4.25) * ts;
    let num = 8.0 * payload as f64 - 4.0 * sf_f + 28.0 + 16.0;
    let den = 4.0 * (sf_f - 2.0 * de);
    let n_payload = 8.0 + ((num / den).ceil() * (f64::from(cr) + 4.0)).max(0.0);
    (t_pre + n_payload * ts) * 1000.0
}

/// The estimate of one part of `bytes` on `profile`, in milliseconds.
pub fn airtime_ms(profile: &AirtimeProfile, bytes: usize) -> Option<u64> {
    if let Some((sf, bw, cr)) = profile.preset.as_deref().and_then(lora_preset) {
        let pl = (bytes + LORA_OVERHEAD).min(LORA_MAX);
        return Some(lora_toa_ms(sf, bw, cr, pl).round() as u64);
    }
    (profile.bits_per_s > 0)
        .then(|| (bytes as u64 * 8 * 1000).div_ceil(u64::from(profile.bits_per_s)))
}

/// The estimate rule in words (`NetworkCaps.airtime.rule`).
pub fn airtime_rule(profile: &AirtimeProfile) -> String {
    match profile.preset.as_deref().filter(|p| lora_preset(p).is_some()) {
        Some(p) => format!(
            "the LoRa time-on-air formula for the {p} preset, per packet (an estimate: retransmissions and other traffic add airtime)"
        ),
        None => format!(
            "bytes × 8 ÷ {} bit/s per part (an estimate)",
            profile.bits_per_s
        ),
    }
}

fn airtime_label(ms: u64) -> String {
    format!("about {:.1} s (estimate)", ms as f64 / 1000.0)
}

// ---------------------------------------------------------------- plans

/// A refused target (`PlanRefusal`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// `oversize`, `ham_unconfirmed` or `uplink_off`.
    pub code: &'static str,
    pub detail: String,
    pub consent: Option<ConsentRequired>,
}

/// What a send will do to one target (`SendPlan`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The texts as they will be queued, one per part (empty when refused).
    pub parts: Vec<String>,
    pub bytes: usize,
    pub cap: Option<usize>,
    pub size_mode: SizeMode,
    /// `(millis, label)`; `None` when the carrier states nothing.
    pub airtime: Option<(Option<u64>, String)>,
    pub refusal: Option<Refusal>,
    /// `(code, text)`.
    pub warnings: Vec<(&'static str, String)>,
}

impl Plan {
    /// The contract's `SendPlan`.
    pub fn json(&self) -> Value {
        json!({
            "bytes": self.bytes,
            "cap": self.cap,
            "parts": self.parts.len().max(1),
            "size_mode": match self.size_mode { SizeMode::Refuse => "refuse", SizeMode::Split => "split" },
            "airtime_estimate": self.airtime.as_ref().map(|(ms, label)| json!({ "millis": ms, "label": label })),
            "refusal": self.refusal.as_ref().map(|r| json!({
                "code": r.code,
                "detail": r.detail,
                "consent": r.consent.as_ref().and_then(|c| serde_json::to_value(c).ok()),
            })),
            "warnings": self.warnings.iter().map(|(c, t)| json!({ "code": c, "text": t })).collect::<Vec<_>>(),
        })
    }

    fn plan_111(text: &str) -> Plan {
        Plan {
            parts: vec![text.to_string()],
            bytes: text.len(),
            cap: None,
            size_mode: SizeMode::Refuse,
            airtime: None,
            refusal: None,
            warnings: Vec::new(),
        }
    }
}

/// The oversize detail ("312 B > 232 B per packet").
pub fn oversize_detail(bytes: usize, cap: usize) -> String {
    format!("{bytes} B > {cap} B per packet")
}

/// The live context of one target conversation.
struct Live {
    caps: Option<Caps>,
    facts: Option<AdapterFacts>,
    link_down: bool,
}

/// Where a plan goes: the network, the kind and account the cap is read for, and the
/// stored conversation (`None` for a recipient list's DMs: no channel gates).
#[derive(Clone, Copy)]
struct Dest<'a> {
    network: &'a str,
    kind: ConvKind,
    account: &'a str,
    conv: Option<&'a ForeignConversationRow>,
}

/// A resolved target.
enum Resolved {
    Foreign {
        target: Value,
        conv: ForeignConversationRow,
        mode: SizeMode,
    },
    List {
        target: Value,
        members: Vec<String>,
        network: String,
        mode: SizeMode,
    },
    Contact {
        target: Value,
        node: id111::keyset::NodeId,
    },
}

impl Bridge {
    fn live_of(&self, network: &str) -> Live {
        match self.attached(network) {
            Some(a) => Live {
                caps: Some(a.adapter.caps()),
                facts: Some(a.adapter.facts()),
                link_down: a.link_down(),
            },
            None => Live {
                caps: None,
                facts: None,
                link_down: true,
            },
        }
    }

    /// The plan of `text` into a conversation of `kind` on `network` under `account`
    /// (module doc). `conv` is `None` for a recipient list (no channel gates).
    fn plan_into(&self, to: Dest<'_>, text: &str, mode: SizeMode, lead: &str) -> Plan {
        let Dest {
            network,
            kind,
            account,
            conv,
        } = to;
        let live = self.live_of(network);
        let cap = self
            .cap_for(
                network,
                kind,
                account,
                live.caps.as_ref(),
                live.facts.as_ref(),
            )
            .map(|c| c.min(ingest::MAX_BODY_BYTES));
        let splittable = live
            .caps
            .as_ref()
            .map(|c| c.splittable)
            .or(self.manifest(network).map(|m| m.splittable))
            .unwrap_or(false);
        let whole = format!("{lead}{text}");
        let mut refusal = None;
        let parts: Vec<String> = match cap {
            None if whole.len() <= ingest::MAX_BODY_BYTES => vec![whole.clone()],
            Some(c) if whole.len() <= c => vec![whole.clone()],
            _ => {
                let c = cap.unwrap_or(ingest::MAX_BODY_BYTES);
                match (mode, splittable) {
                    (SizeMode::Split, true) => match split(text, c, lead) {
                        Some(p) => p,
                        None => {
                            refusal = Some(Refusal {
                                code: "oversize",
                                detail: format!(
                                    "{} cannot be split into at most {MAX_PARTS} parts of {c} B",
                                    oversize_detail(whole.len(), c)
                                ),
                                consent: None,
                            });
                            Vec::new()
                        }
                    },
                    _ => {
                        refusal = Some(Refusal {
                            code: "oversize",
                            detail: oversize_detail(whole.len(), c),
                            consent: None,
                        });
                        Vec::new()
                    }
                }
            }
        };
        if let (Some(c), Some(f)) = (conv, live.facts.as_ref()) {
            if let Some(g) = self.gate_for(c, f) {
                refusal = Some(Refusal {
                    code: g.reason(),
                    detail: g.detail(),
                    consent: g.consent().cloned(),
                });
            }
        }
        let bytes = if parts.is_empty() {
            whole.len()
        } else {
            parts.iter().map(String::len).sum()
        };
        let mut warnings = Vec::new();
        match (&live.caps, live.link_down) {
            (None, _) => warnings.push((
                "adapter_down",
                "This network's adapter is not running; the text waits.".to_string(),
            )),
            (Some(_), true) => warnings.push((
                "adapter_down",
                "The adapter's link is down; the text waits for it.".to_string(),
            )),
            (Some(c), false) if c.state != super::AdapterState::Live => warnings.push((
                "adapter_down",
                "The adapter is not live; the text waits.".to_string(),
            )),
            _ => {}
        }
        if live.facts.as_ref().and_then(|f| f.region_set) == Some(false) {
            warnings.push((
                "region_unset",
                "The radio's region is not set: nothing is transmitted until it is.".to_string(),
            ));
        }
        if let (Some(c), Some(f)) = (conv, live.facts.as_ref()) {
            if c.kind == ForeignKind::Channel
                && f.channel(&c.account, &c.native_id)
                    .is_some_and(|ch| ch.uplink)
            {
                warnings.push(("uplink_channel", self.uplink_text_of(c)));
            }
        }
        if parts.len() > 1 && live.facts.as_ref().is_some_and(|f| f.parts_may_reorder) {
            warnings.push((
                "parts_out_of_order",
                "The parts can arrive out of order; each is numbered.".to_string(),
            ));
        }
        let profile = live.caps.as_ref().and_then(|c| c.airtime.clone());
        if let Some(p) = profile.as_ref().filter(|p| p.duty_cycle_permille > 0) {
            warnings.push((
                "duty_cycle",
                format!(
                    "The region allows {}.{} % airtime; a long send waits for it.",
                    p.duty_cycle_permille / 10,
                    p.duty_cycle_permille % 10
                ),
            ));
        }
        let airtime = match &profile {
            Some(p) => {
                let total: Option<u64> = if parts.is_empty() {
                    airtime_ms(p, whole.len().min(cap.unwrap_or(usize::MAX)))
                } else {
                    parts.iter().map(|t| airtime_ms(p, t.len())).sum()
                };
                total.map(|ms| (Some(ms), airtime_label(ms)))
            }
            None if self
                .manifest(network)
                .is_some_and(|m| m.airtime_rule.is_some()) =>
            {
                Some((None, "depends on the path".to_string()))
            }
            None => None,
        };
        Plan {
            parts,
            bytes,
            cap,
            size_mode: mode,
            airtime,
            refusal,
            warnings,
        }
    }

    /// The plan of a compose send into the stored conversation `c` (a list plans its DMs).
    pub(super) fn plan_conv(&self, c: &ForeignConversationRow, text: &str, mode: SizeMode) -> Plan {
        if is_list_conv(c) {
            let account = self
                .facts_of(&c.network)
                .and_then(|f| f.accounts.first().map(|a| a.id.clone()))
                .unwrap_or_default();
            let to = Dest {
                network: &c.network,
                kind: ConvKind::Dm,
                account: &account,
                conv: None,
            };
            return self.plan_into(to, text, mode, "");
        }
        let to = Dest {
            network: &c.network,
            kind: super::core::conv_kind(c.kind),
            account: &c.account,
            conv: Some(c),
        };
        self.plan_into(to, text, mode, "")
    }

    /// The plan of a relay copy (`lead` = the marker) into `c`.
    pub(super) fn plan_copy(
        &self,
        c: &ForeignConversationRow,
        text: &str,
        mode: SizeMode,
        lead: &str,
    ) -> Plan {
        let kind = if is_list_conv(c) {
            ConvKind::Dm
        } else {
            super::core::conv_kind(c.kind)
        };
        let account = if is_list_conv(c) {
            self.facts_of(&c.network)
                .and_then(|f| f.accounts.first().map(|a| a.id.clone()))
                .unwrap_or_default()
        } else {
            c.account.clone()
        };
        let to = Dest {
            network: &c.network,
            kind,
            account: &account,
            conv: None,
        };
        self.plan_into(to, text, mode, lead)
    }

    async fn resolve(
        &self,
        t: &FanoutTarget,
        default_mode: SizeMode,
    ) -> Result<Resolved, BridgeError> {
        let target = serde_json::to_value(t).unwrap_or_default();
        match t {
            FanoutTarget::Contact(c) => {
                let node = fjson::parse_node_id(&c.node_id).ok_or_else(|| {
                    BridgeError::BadRequest(format!("{:?} is not a NodeId", c.node_id))
                })?;
                Ok(Resolved::Contact { target, node })
            }
            FanoutTarget::Foreign(f) => {
                let id =
                    fjson::parse_fc_id(&f.conv).ok_or(BridgeError::NotFound("conversation"))?;
                let conv = self.conv_row(id).await?;
                if conv.network != f.network {
                    return Err(BridgeError::NotFound("conversation"));
                }
                if self.attached(&conv.network).is_none() {
                    return Err(BridgeError::Unavailable(format!(
                        "Enable {} first.",
                        self.label_of(&conv.network)
                    )));
                }
                let mode = f.size_mode.unwrap_or(default_mode);
                if is_list_conv(&conv) {
                    let list = self
                        .list_of(&conv)
                        .ok_or(BridgeError::NotFound("recipient list"))?;
                    if list.members.is_empty() {
                        return Err(BridgeError::BadRequest(format!(
                            "the recipient list {} has no members",
                            list.id
                        )));
                    }
                    return Ok(Resolved::List {
                        target,
                        members: list.members,
                        network: conv.network.clone(),
                        mode,
                    });
                }
                Ok(Resolved::Foreign { target, conv, mode })
            }
        }
    }

    fn check_text(text: &str) -> Result<(), BridgeError> {
        if text.is_empty() {
            return Err(BridgeError::BadRequest("the text is empty".into()));
        }
        if text.len() > ingest::MAX_BODY_BYTES {
            return Err(BridgeError::BadRequest("the text is over 64 KiB".into()));
        }
        Ok(())
    }

    async fn resolve_all(&self, req: &FanoutRequest) -> Result<Vec<(Resolved, Plan)>, BridgeError> {
        Self::check_text(&req.text)?;
        if req.targets.is_empty() || req.targets.len() > MAX_TARGETS {
            return Err(BridgeError::BadRequest(format!(
                "1..={MAX_TARGETS} targets"
            )));
        }
        let mode = req.size_mode.unwrap_or_default();
        let mut out = Vec::with_capacity(req.targets.len());
        for t in &req.targets {
            let r = self.resolve(t, mode).await?;
            let plan = match &r {
                Resolved::Foreign { conv, mode, .. } => self.plan_conv(conv, &req.text, *mode),
                Resolved::List { network, mode, .. } => {
                    let account = self
                        .facts_of(network)
                        .and_then(|f| f.accounts.first().map(|a| a.id.clone()))
                        .unwrap_or_default();
                    let to = Dest {
                        network,
                        kind: ConvKind::Dm,
                        account: &account,
                        conv: None,
                    };
                    self.plan_into(to, &req.text, *mode, "")
                }
                Resolved::Contact { .. } => Plan::plan_111(&req.text),
            };
            out.push((r, plan));
        }
        Ok(out)
    }

    /// `POST /messages/fanout/preview`: a plan per target; writes, transmits and logs
    /// nothing.
    pub async fn fanout_preview(&self, req: &FanoutRequest) -> Result<Value, BridgeError> {
        let resolved = self.resolve_all(req).await?;
        let ok = resolved.iter().all(|(_, p)| p.refusal.is_none());
        let items: Vec<Value> = resolved
            .iter()
            .map(|(r, p)| {
                let target = match r {
                    Resolved::Foreign { target, .. }
                    | Resolved::List { target, .. }
                    | Resolved::Contact { target, .. } => target.clone(),
                };
                json!({ "target": target, "plan": p.json() })
            })
            .collect();
        Ok(json!({ "schema": 1, "text_bytes": req.text.len(), "ok": ok, "items": items }))
    }

    /// The per-target 409 report, when any target is refused (nothing is queued).
    fn refusals(resolved: &[(Resolved, Plan)]) -> Option<BridgeError> {
        let mut targets = Vec::new();
        let mut all_oversize = true;
        for (r, p) in resolved {
            let Some(refusal) = &p.refusal else { continue };
            let target = match r {
                Resolved::Foreign { target, .. }
                | Resolved::List { target, .. }
                | Resolved::Contact { target, .. } => target.clone(),
            };
            let code = if refusal.code == "oversize" {
                "oversize"
            } else {
                all_oversize = false;
                "consent_required"
            };
            targets.push(json!({
                "target": target,
                "code": code,
                "detail": refusal.detail,
                "consent": refusal.consent.as_ref().and_then(|c| serde_json::to_value(c).ok()),
            }));
        }
        (!targets.is_empty()).then(|| BridgeError::Refused {
            code: if all_oversize {
                "oversize"
            } else {
                "consent_required"
            },
            targets,
        })
    }

    /// `POST /messages/fanout` (module doc). Returns `FanoutResult`.
    pub async fn fanout(&self, req: &FanoutRequest) -> Result<Value, BridgeError> {
        let resolved = self.resolve_all(req).await?;
        if let Some(e) = Self::refusals(&resolved) {
            return Err(e);
        }
        let f = self.fab()?;
        // 111 targets must be accepted contacts (validated before anything is sent)
        for (r, _) in &resolved {
            if let Resolved::Contact { node, .. } = r {
                match f.contact(*node).await {
                    Ok(c) if c.request => {
                        return Err(BridgeError::BadRequest(
                            "that contact is a pending message request".into(),
                        ))
                    }
                    Ok(_) => {}
                    Err(FabricError::NotFound) => return Err(BridgeError::NotFound("contact")),
                    Err(e) => return Err(e.into()),
                }
            }
        }
        // expand recipient lists into one DM conversation per member
        struct Row {
            target: Value,
            plan: Value,
            conv: Option<i64>,
            msg: Option<i64>,
            parts: Vec<String>,
            network: Option<String>,
        }
        let mut rows: Vec<Row> = Vec::new();
        for (r, p) in &resolved {
            match r {
                Resolved::Foreign { target, conv, .. } => rows.push(Row {
                    target: target.clone(),
                    plan: p.json(),
                    conv: Some(conv.id),
                    msg: None,
                    parts: p.parts.clone(),
                    network: Some(conv.network.clone()),
                }),
                Resolved::List {
                    members, network, ..
                } => {
                    let account = self
                        .facts_of(network)
                        .and_then(|f| f.accounts.first().map(|a| a.id.clone()))
                        .ok_or_else(|| {
                            BridgeError::Unavailable(
                                "This network has no account of this node yet.".into(),
                            )
                        })?;
                    for m in members {
                        let dm = f
                            .foreign_conversation_upsert(
                                network,
                                &account,
                                ForeignKind::Dm,
                                &format!("dm:{m}"),
                                None,
                            )
                            .await?;
                        rows.push(Row {
                            target: json!({ "network": network, "conv": fjson::fc_id(dm) }),
                            plan: p.json(),
                            conv: Some(dm),
                            msg: None,
                            parts: p.parts.clone(),
                            network: Some(network.clone()),
                        });
                    }
                }
                Resolved::Contact { target, .. } => rows.push(Row {
                    target: target.clone(),
                    plan: p.json(),
                    conv: None,
                    msg: None,
                    parts: Vec::new(),
                    network: None,
                }),
            }
        }
        let total: usize = rows.iter().map(|r| r.parts.len()).sum();
        if total > MAX_PARTS {
            return Err(BridgeError::BadRequest(format!(
                "{total} rows: one send queues at most {MAX_PARTS}"
            )));
        }
        // 111 targets: ordinary sealed messages with their Layer 5 states
        for (row, (r, _)) in rows.iter_mut().filter(|r| r.conv.is_none()).zip(
            resolved
                .iter()
                .filter(|(r, _)| matches!(r, Resolved::Contact { .. })),
        ) {
            if let Resolved::Contact { node, .. } = r {
                row.msg = Some(self.handle.send(*node, &req.text).await?);
            }
        }
        let targets: Vec<GroupTarget> = rows
            .iter()
            .map(|r| GroupTarget {
                target: r.target.clone(),
                plan: r.plan.clone(),
                msg: r.msg,
            })
            .collect();
        let mut parts = Vec::with_capacity(total);
        for r in &rows {
            if let Some(conv) = r.conv {
                let n = r.parts.len() as u32;
                for (i, text) in r.parts.iter().enumerate() {
                    parts.push(ForeignGroupPart {
                        conv,
                        text: text.clone(),
                        part_k: i as u32 + 1,
                        part_n: n,
                    });
                }
            }
        }
        let (group, ids) = self
            .handle
            .foreign_group_create(&req.text, &fjson::send_group_targets(&targets), parts)
            .await?;
        let mut ids = ids.into_iter();
        let mut items = Vec::with_capacity(rows.len());
        for r in &rows {
            match (r.conv, r.msg) {
                (Some(_), _) => {
                    let mine: Vec<i64> = ids.by_ref().take(r.parts.len()).collect();
                    if let Some(net) = &r.network {
                        self.mark_adapter_state(net, &mine).await;
                    }
                    let mut item = json!({
                        "target": r.target,
                        "id": mine.first().map(|i| fjson::fm_id(*i)),
                        "plan": r.plan,
                    });
                    if mine.len() > 1 {
                        item["ids"] = mine.iter().map(|i| json!(fjson::fm_id(*i))).collect();
                    }
                    items.push(item);
                }
                (None, Some(m)) => items.push(json!({
                    "target": r.target,
                    "id": fjson::msg_id(m),
                    "plan": r.plan,
                })),
                (None, None) => {}
            }
        }
        Ok(json!({ "schema": 1, "group_id": fjson::sg_id(group), "items": items }))
    }

    /// Rows queued on a network whose adapter cannot take them now carry the reason (§9:
    /// adapter state is not a call error).
    async fn mark_adapter_state(&self, network: &str, ids: &[i64]) {
        let reason = match self.attached(network) {
            None => Some("adapter_down"),
            Some(a) if a.link_down() || a.adapter.caps().state != super::AdapterState::Live => {
                Some("adapter_down")
            }
            Some(a) if a.adapter.facts().region_set == Some(false) => Some("region_unset"),
            Some(_) => None,
        };
        if let (Some(reason), Ok(f)) = (reason, self.fab()) {
            for id in ids {
                let _ = f
                    .foreign_set_state(*id, ForeignState::Queued, None, Some(reason))
                    .await;
            }
        }
    }

    /// `POST /networks/{net}/conversations/{conv}/messages` `{text, size_mode?}`: a direct
    /// send is a one-target send group. `409 oversize` in refuse mode, `409
    /// consent_required` for a gate; otherwise `ForeignSendAccepted` `{id, ids?, plan}`.
    pub async fn send_text(
        &self,
        network: &str,
        conv: i64,
        text: &str,
        size_mode: Option<SizeMode>,
    ) -> Result<Value, BridgeError> {
        let req = FanoutRequest {
            text: text.to_string(),
            targets: vec![FanoutTarget::Foreign(ForeignTarget {
                network: network.to_string(),
                conv: fjson::fc_id(conv),
                size_mode,
            })],
            size_mode: None,
        };
        let resolved = self.resolve_all(&req).await?;
        if let Some((r, p)) = resolved.first() {
            if let Some(refusal) = &p.refusal {
                return Err(match &refusal.consent {
                    _ if refusal.code == "oversize" => {
                        BridgeError::Oversize(refusal.detail.clone())
                    }
                    Some(c) => BridgeError::Consent(c.clone()),
                    None => {
                        let subject = match r {
                            Resolved::Foreign { conv, .. } => fjson::fc_id(conv.id),
                            _ => String::new(),
                        };
                        let text = format!(
                            "Sending on this channel is switched off on this node ({subject}). Turn it on to send."
                        );
                        BridgeError::Consent(ConsentRequired {
                            action: "uplink_send",
                            subject,
                            hash: super::settings::sha256_hex(&text),
                            warning_text: text,
                            addons: Vec::new(),
                        })
                    }
                });
            }
        }
        let v = self.fanout(&req).await?;
        let item = v["items"].get(0).cloned().unwrap_or_default();
        let ids: Vec<Value> = v["items"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|i| match i.get("ids").and_then(Value::as_array) {
                Some(a) => a.clone(),
                None => vec![i["id"].clone()],
            })
            .collect();
        let mut out = json!({ "schema": 1, "id": item["id"], "plan": item["plan"] });
        if ids.len() > 1 {
            out["ids"] = Value::Array(ids);
        }
        Ok(out)
    }

    /// Cancel one own row (§9): only while `queued` and never handed to a carrier.
    pub async fn cancel(&self, id: i64) -> Result<(), BridgeError> {
        Ok(self.fab()?.foreign_cancel(id).await?)
    }

    /// Cancel every unsent row of send group `group` (a split send's unsent parts; the
    /// parts already handed stay). Returns the cancelled row ids.
    pub async fn cancel_unsent(&self, group: i64) -> Result<Vec<i64>, BridgeError> {
        let f = self.fab()?;
        let view = f.foreign_group(group).await?;
        let mut out = Vec::new();
        for r in view.rows {
            let unsent =
                r.state == ForeignState::Queued && !r.handoff_pending && r.native_msg_id.is_none();
            if unsent && f.foreign_cancel(r.id).await.is_ok() {
                out.push(r.id);
            }
        }
        Ok(out)
    }
}

/// The detail of a gate (re-exported for routes building a report).
pub fn gate_refusal(g: &Gate) -> Refusal {
    Refusal {
        code: g.reason(),
        detail: g.detail(),
        consent: g.consent().cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_counts_the_part_prefix_prefers_whitespace_and_joins_back() {
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
        let parts = split(text, 24, "").unwrap();
        for p in &parts {
            assert!(p.len() <= 24, "{p:?}");
        }
        let n = parts.len();
        let mut joined = String::new();
        for (i, p) in parts.iter().enumerate() {
            let prefix = format!("({}/{n}) ", i + 1);
            assert!(p.starts_with(&prefix), "{p:?}");
            joined.push_str(&p[prefix.len()..]);
        }
        assert_eq!(joined, text, "nothing lost, nothing added");
        assert!(parts[0].ends_with(' '), "cut after a space: {:?}", parts[0]);
    }

    #[test]
    fn split_never_cuts_inside_a_character_and_leads_with_the_marker() {
        let text = "\u{20AC}".repeat(100); // 300 B, no whitespace
        let lead = "[W1AW/M] ";
        let parts = split(&text, 50, lead).unwrap();
        let n = parts.len();
        let mut joined = String::new();
        for (i, p) in parts.iter().enumerate() {
            assert!(p.len() <= 50);
            let prefix = format!("{lead}({}/{n}) ", i + 1);
            assert!(p.starts_with(&prefix), "marker first on every part: {p:?}");
            joined.push_str(&p[prefix.len()..]);
        }
        assert_eq!(joined, text);
        assert!(split("x", 5, "[W1AW/M] ").is_none(), "no room for any text");
        assert!(split(&"x".repeat(5000), 10, "").is_none(), "over 256 parts");
    }

    #[test]
    fn split_grows_the_prefix_width_with_the_part_count() {
        let text = "y".repeat(3000);
        let parts = split(&text, 30, "").unwrap();
        assert!(
            parts.len() >= 100,
            "three-digit part numbers: {}",
            parts.len()
        );
        assert!(parts.iter().all(|p| p.len() <= 30));
        assert!(parts[0].starts_with(&format!("(1/{}) ", parts.len())));
    }

    #[test]
    fn the_longfast_estimate_matches_the_kb() {
        let (sf, bw, cr) = lora_preset("LongFast").unwrap();
        let ms = lora_toa_ms(sf, bw, cr, 255);
        assert!(
            (2050.0..2200.0).contains(&ms),
            "255 B on LongFast is about 2.1 s: {ms}"
        );
        let p = AirtimeProfile {
            duty_cycle_permille: 100,
            bits_per_s: 1070,
            preset: Some("LongFast".into()),
        };
        assert_eq!(
            airtime_ms(&p, 232).unwrap(),
            lora_toa_ms(sf, bw, cr, 255).round() as u64
        );
        let plain = AirtimeProfile {
            duty_cycle_permille: 0,
            bits_per_s: 1000,
            preset: None,
        };
        assert_eq!(airtime_ms(&plain, 125), Some(1000));
        assert!(airtime_rule(&p).contains("estimate"));
        assert_eq!(airtime_label(2156), "about 2.2 s (estimate)");
    }

    #[test]
    fn requests_parse_both_target_shapes() {
        let r: FanoutRequest = serde_json::from_value(json!({
            "text": "hi",
            "targets": [
                {"network": "meshtastic", "conv": "fc-1", "size_mode": "split"},
                {"node_id": "ab".repeat(32)},
            ],
            "size_mode": "refuse",
        }))
        .unwrap();
        assert!(matches!(r.targets[0], FanoutTarget::Foreign(_)));
        assert!(matches!(r.targets[1], FanoutTarget::Contact(_)));
        assert!(serde_json::from_value::<FanoutRequest>(
            json!({"text": "x", "targets": [{"conv": "fc-1"}]})
        )
        .is_err());
        assert_eq!(oversize_detail(312, 232), "312 B > 232 B per packet");
    }
}
