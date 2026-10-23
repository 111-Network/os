//! `/api/v1/transport*` (Layer 6 `os/6-transport/README.md` §11, §12; contract
//! `getTransport`, `patchTransportPosture`, `joinTransport`).
//!
//! - `GET /transport` → `TransportStatus`: the interface facts from the tick-driven
//!   `rnstatus` cache ([`FactsCache::snapshot`]: never runs `rnstatus` on the request
//!   path; a stale cache only wakes the tick), the `[posture]`, `[radio]` and `[order]` of
//!   `state/interfaces.toml`. No credential appears here.
//! - `PATCH /transport/posture` → `TransportPostureResult`: a partial merge of the
//!   posture, the radio region (the request is the human's confirmation, §12
//!   `radio.region.set`) and callsign, and the preferred order. The whole merge is
//!   validated against a copy of the file first (an unknown region or a bad value is
//!   `400`, nothing written). Then, **posture through the applier (§10, §12;
//!   C1-bounce F3)**: when the device core's `Arc<Applier>` is in ext, the parts its
//!   closed vocabulary carries — `share_instance` / `enable_transport` as
//!   `rns.posture.set`, the region as `radio.region.set`, in that order — go through its
//!   snapshot → apply → probe → keep-or-revert loop, so a failed rnsd restart or probe
//!   restores the previous `interfaces.toml` byte-exact and answers `applied: false,
//!   reverted: true`. Only when they were kept does the rest (`instance_name`, `lan`,
//!   `stock_autointerface`, `propagation_node`, callsign, order — no applier action
//!   exists for them) follow through `Spine::update` (validate → save 0600 → re-render →
//!   diff → `rnstatus --attach|--detach|--reload` or an rnsd restart with its dependants
//!   in order); a failed step there is `applied: false, reverted: false` with the
//!   `reason`. The PATCH is no proposal: the applier's decision record for it (a
//!   one-off `transport-posture-<random>` id) is removed once it answers. Without an
//!   applier in ext (a node without the device registry, tests) the whole merge goes
//!   through `Spine::update` as before and `reverted` is `false`.
//! - `POST /transport/join` → `TransportJoin` with `Cache-Control: no-store`: the
//!   phone-relay descriptor (address, port, and the IFAC name and passphrase when IFAC is
//!   on). A POST because it carries a credential and the agent's bearer reads every GET:
//!   the global write gate refuses the bearer (`401`), and only a paired or loopback
//!   session reaches it. The address is the one the client used to reach this node when
//!   it is one of this node's own unicast addresses, else this node's first LAN address.
//!   `409 bad_request` while LAN sharing is off, the Backbone listener is off, or no LAN
//!   address exists. The descriptor is never logged.

use super::auth;
use super::{ApiRouter, ApiState, Problem};
use crate::daemon::devices::applier::{self, Action, Applier, ApplierRequest, ApplyReport};
use crate::daemon::transport::join::{JoinError, Session};
use crate::daemon::transport::posture::{Applied, ApplyError, Spine};
use crate::daemon::transport::rnstatus::{FactsCache, FactsSnapshot};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, Extensions, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::IpAddr;
use std::sync::Arc;

/// What the transport routes read (inserted as `Arc<TransportApi>` at daemon start, from
/// the `TransportHandle`; tests build one from a spine and a cache).
pub struct TransportApi {
    pub spine: Arc<Spine>,
    pub facts: Arc<FactsCache>,
}

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/transport", get(status))
        .route("/api/v1/transport/posture", patch(patch_posture))
        .route("/api/v1/transport/join", post(join))
}

fn transport_of(api: &ApiState) -> Result<Arc<TransportApi>, Problem> {
    api.ext::<Arc<TransportApi>>().ok_or_else(|| {
        Problem::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "internal",
            "The transport spine is not running on this node.",
        )
    })
}

/// `TransportStatus` from the file and a facts snapshot.
pub fn status_json(spine: &Spine, facts: &FactsSnapshot) -> Value {
    let f = spine.file();
    let p = &f.posture;
    json!({
        "schema": 1,
        "interfaces": facts.interfaces,
        "interfaces_state": facts.state,
        "posture": {
            "share_instance": p.share_instance,
            "instance_name": p.instance_name,
            "enable_transport": p.enable_transport_for(spine.category()),
            "lan": {
                "backbone_listen": p.lan.backbone.listen,
                "port": p.lan.backbone.port,
                "ifac": p.lan.backbone.ifac,
            },
            "stock_autointerface": p.lan.stock_autointerface,
            "propagation_node": p.lan.propagation_node,
        },
        "region": f.radio.region,
        "callsign": f.radio.callsign(),
        "order": f.order.networks,
    })
}

async fn status(State(api): State<Arc<ApiState>>) -> Response {
    match transport_of(&api) {
        Ok(t) => Json(status_json(&t.spine, &t.facts.snapshot())).into_response(),
        Err(p) => p.into_response(),
    }
}

/// An apply failure as the contract's problem.
pub fn apply_problem(e: &ApplyError) -> Problem {
    match e {
        ApplyError::Invalid(i) => Problem::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "The change was refused.",
        )
        .detail(i.to_string()),
        ApplyError::ReadOnly | ApplyError::NewerConfig => Problem::new(
            StatusCode::CONFLICT,
            "bad_request",
            "This change cannot be made by this release.",
        )
        .detail(e.to_string()),
        ApplyError::Save(_) | ApplyError::Write(_) => Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "The change could not be saved.",
        )
        .detail(e.to_string()),
    }
}

/// `TransportPosturePatch.posture.lan`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LanPatch {
    #[serde(default)]
    backbone_listen: Option<bool>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    ifac: Option<bool>,
}

/// `TransportPosturePatch.posture`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PosturePatch {
    #[serde(default)]
    share_instance: Option<bool>,
    #[serde(default)]
    instance_name: Option<String>,
    #[serde(default)]
    enable_transport: Option<bool>,
    #[serde(default)]
    lan: Option<LanPatch>,
    #[serde(default)]
    stock_autointerface: Option<bool>,
    #[serde(default)]
    propagation_node: Option<bool>,
}

impl PosturePatch {
    fn is_empty(&self) -> bool {
        self.share_instance.is_none()
            && self.instance_name.is_none()
            && self.enable_transport.is_none()
            && self
                .lan
                .as_ref()
                .is_none_or(|l| l.backbone_listen.is_none() && l.port.is_none() && l.ifac.is_none())
            && self.stock_autointerface.is_none()
            && self.propagation_node.is_none()
    }
}

fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

/// `TransportPosturePatch`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportPatch {
    #[serde(default)]
    posture: Option<PosturePatch>,
    #[serde(default)]
    region: Option<String>,
    /// `null` clears the callsign.
    #[serde(default, deserialize_with = "present")]
    callsign: Option<Option<String>>,
    #[serde(default)]
    order: Option<Vec<String>>,
}

impl TransportPatch {
    fn is_empty(&self) -> bool {
        self.posture.is_none()
            && self.region.is_none()
            && self.callsign.is_none()
            && self.order.is_none()
    }

    /// The contract's bounds the file's validation does not state.
    fn check(&self) -> Result<(), Problem> {
        if let Some(p) = &self.posture {
            if let Some(n) = &p.instance_name {
                if n.is_empty() || n.chars().count() > 32 {
                    return Err(super::bridge::bad_request(
                        "instance_name must be 1..=32 characters.",
                    ));
                }
            }
            if let Some(port) = p.lan.as_ref().and_then(|l| l.port) {
                if port < 1024 {
                    return Err(super::bridge::bad_request("port must be 1024..=65535."));
                }
            }
        }
        if let Some(r) = &self.region {
            if r.is_empty() || r.len() > 32 {
                return Err(super::bridge::bad_request("region must be 1..=32 bytes."));
            }
        }
        if let Some(Some(c)) = &self.callsign {
            if c.chars().count() > 16 {
                return Err(super::bridge::bad_request(
                    "callsign is at most 16 characters.",
                ));
            }
        }
        Ok(())
    }

    /// The parts the device core's applier carries (§12 vocabulary, module doc):
    /// `rns.posture.set` first, then `radio.region.set`.
    fn applier_actions(&self) -> Vec<Action> {
        let mut out = Vec::new();
        if let Some(p) = &self.posture {
            if p.share_instance.is_some() || p.enable_transport.is_some() {
                out.push(Action::PostureSet {
                    share_instance: p.share_instance,
                    enable_transport: p.enable_transport,
                });
            }
        }
        if let Some(r) = &self.region {
            out.push(Action::RegionSet { region: r.clone() });
        }
        out
    }

    /// This patch without the applier's parts (what follows through `Spine::update`).
    fn without_applier_parts(mut self) -> Self {
        if let Some(p) = &mut self.posture {
            p.share_instance = None;
            p.enable_transport = None;
        }
        self.region = None;
        if self.posture.as_ref().is_some_and(PosturePatch::is_empty) {
            self.posture = None;
        }
        self
    }

    /// Apply the merge to the file (validation follows in `Spine::update`).
    fn merge(self, f: &mut crate::daemon::transport::interfaces::InterfacesFile) {
        if let Some(p) = self.posture {
            let d = &mut f.posture;
            if let Some(v) = p.share_instance {
                d.share_instance = v;
            }
            if let Some(v) = p.instance_name {
                d.instance_name = v;
            }
            if let Some(v) = p.enable_transport {
                d.enable_transport = Some(v);
            }
            if let Some(l) = p.lan {
                if let Some(v) = l.backbone_listen {
                    d.lan.backbone.listen = v;
                }
                if let Some(v) = l.port {
                    d.lan.backbone.port = v;
                }
                if let Some(v) = l.ifac {
                    d.lan.backbone.ifac = v;
                }
            }
            if let Some(v) = p.stock_autointerface {
                d.lan.stock_autointerface = v;
            }
            if let Some(v) = p.propagation_node {
                d.lan.propagation_node = v;
            }
        }
        if let Some(r) = self.region {
            f.radio.region = Some(r);
        }
        if let Some(c) = self.callsign {
            f.radio.callsign = c.filter(|c| !c.is_empty());
        }
        if let Some(o) = self.order {
            f.order.networks = o;
        }
    }
}

/// `TransportPostureResult` for an apply.
fn result_json(applied: &Applied, t: &TransportApi) -> Value {
    let ok = applied.failures.is_empty();
    json!({
        "schema": 1,
        "applied": ok,
        "reverted": false,
        "reason": (!ok).then(|| applied.failures.join("; ")),
        "status": status_json(&t.spine, &t.facts.snapshot()),
    })
}

/// `TransportPostureResult` for the applier's parts not kept (module doc).
fn report_json(report: &ApplyReport, t: &TransportApi) -> Value {
    json!({
        "schema": 1,
        "applied": false,
        "reverted": report.reverted,
        "reason": report.reason.clone().or_else(|| Some("the change was not applied".into())),
        "status": status_json(&t.spine, &t.facts.snapshot()),
    })
}

/// A one-off applier request id for a PATCH (never a proposal's: proposals are the
/// agent's `prop-…`; 8 random bytes, no clock).
fn patch_request_id() -> Option<String> {
    let mut b = [0u8; 8];
    getrandom::getrandom(&mut b).ok()?;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Some(format!("transport-posture-{hex}"))
}

/// The applier's parts of a PATCH through its loop (module doc). `Ok(None)`: kept;
/// `Ok(Some(body))`: not kept (the answer); `Err`: refused before anything was written.
async fn through_applier(
    api: &ApiState,
    applier: &Applier,
    actions: Vec<Action>,
    t: &TransportApi,
) -> Result<Option<Value>, Response> {
    let internal = |why: &str| {
        Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "The change could not be applied.",
        )
        .detail(why.to_string())
        .into_response()
    };
    let id = patch_request_id().ok_or_else(|| internal("no randomness for the request id"))?;
    let req = ApplierRequest::new(&id, actions).map_err(|r| r.problem().into_response())?;
    let res = applier::apply(applier, req).await;
    if !matches!(res, Err(applier::ApplyError::AlreadyDecided)) {
        // the PATCH is no proposal: its decision record goes (absent is fine)
        let rec = super::proposals::decisions_dir(&api.layout).join(format!("{id}.json"));
        if let Err(e) = std::fs::remove_file(&rec) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("111 transport: {} stays: {e}", rec.display());
            }
        }
    }
    let report = res.map_err(|e| e.problem().into_response())?;
    Ok((!report.applied).then(|| report_json(&report, t)))
}

async fn patch_posture(State(api): State<Arc<ApiState>>, bytes: Bytes) -> Response {
    let t = match transport_of(&api) {
        Ok(t) => t,
        Err(p) => return p.into_response(),
    };
    let p: TransportPatch = match super::bridge::body(&bytes) {
        Ok(p) => p,
        Err(p) => return p.into_response(),
    };
    if let Err(p) = p.check() {
        return p.into_response();
    }
    if p.is_empty() {
        let none = Applied::default();
        return Json(result_json(&none, &t)).into_response();
    }
    // the whole merge validated first: nothing is applied when any part is refused
    let mut sim = t.spine.file();
    p.clone().merge(&mut sim);
    if let Err(i) = sim.validate() {
        return apply_problem(&ApplyError::Invalid(i)).into_response();
    }
    let actions = p.applier_actions();
    let rest = match api.ext::<Arc<Applier>>() {
        Some(ap) if !actions.is_empty() => {
            match through_applier(&api, &ap, actions, &t).await {
                Ok(None) => {}
                Ok(Some(not_kept)) => return Json(not_kept).into_response(),
                Err(resp) => return resp,
            }
            let rest = p.without_applier_parts();
            if rest.is_empty() {
                let none = Applied::default();
                return Json(result_json(&none, &t)).into_response();
            }
            rest
        }
        _ => p,
    };
    match t.spine.update(move |f| rest.merge(f)).await {
        Ok(applied) => Json(result_json(&applied, &t)).into_response(),
        Err(e) => apply_problem(&e).into_response(),
    }
}

/// The session a join is asked on (`require_session` already refused everything else).
fn session_of(api: &ApiState, headers: &HeaderMap, ext: &Extensions) -> Session {
    if api.auth.session_in(headers).is_some() {
        Session::Paired
    } else if auth::trusted_loopback(api, ext) {
        Session::Loopback
    } else {
        Session::Other
    }
}

fn usable(ip: &IpAddr) -> bool {
    !(ip.is_loopback() || ip.is_unspecified() || ip.is_multicast())
}

/// The hub address a phone reaches (module doc).
fn join_address(api: &ApiState, headers: &HeaderMap) -> Option<IpAddr> {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .and_then(crate::daemon::dashboard::routes::parse_host)
        .and_then(|(h, _)| h.trim_matches(['[', ']']).parse::<IpAddr>().ok());
    if let Some(ip) = host.filter(|ip| usable(ip) && api.is_own_addr(ip)) {
        return Some(ip);
    }
    let own = api.own_addrs();
    own.iter()
        .find(|ip| ip.is_ipv4() && usable(ip))
        .or_else(|| own.iter().find(|ip| usable(ip)))
        .copied()
}

fn join_problem(e: &JoinError) -> Problem {
    match e {
        JoinError::LanOff | JoinError::NoListener | JoinError::BadAddress => Problem::new(
            StatusCode::CONFLICT,
            "bad_request",
            "There is no relay to join on this node.",
        )
        .detail(e.to_string()),
        JoinError::Untrusted => Problem::new(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "pairing required",
        )
        .detail(e.to_string()),
        _ => Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "The join descriptor could not be made.",
        )
        .detail(e.to_string()),
    }
}

async fn join(
    State(api): State<Arc<ApiState>>,
    headers: HeaderMap,
    ext: Extensions,
    bytes: Bytes,
) -> Response {
    let t = match transport_of(&api) {
        Ok(t) => t,
        Err(p) => return p.into_response(),
    };
    if let Err(p) = super::bridge::body::<Value>(&bytes) {
        return p.into_response();
    }
    let session = session_of(&api, &headers, &ext);
    let Some(address) = join_address(&api, &headers) else {
        return join_problem(&JoinError::BadAddress).into_response();
    };
    let spine = Arc::clone(&t.spine);
    let made = tokio::task::spawn_blocking(move || spine.join_descriptor(address, session)).await;
    match made {
        Ok(Ok(d)) => (
            [(header::CACHE_CONTROL, "no-store")],
            Json(serde_json::to_value(&d).unwrap_or(Value::Null)),
        )
            .into_response(),
        Ok(Err(e)) => join_problem(&e).into_response(),
        Err(_) => join_problem(&JoinError::IfacSecretMissing).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_patch_merges_only_what_it_names() {
        let mut f = crate::daemon::transport::interfaces::InterfacesFile::default();
        let before = f.clone();
        let p: TransportPatch = serde_json::from_value(json!({})).unwrap();
        assert!(p.is_empty());
        p.merge(&mut f);
        assert_eq!(f, before);
        let p: TransportPatch = serde_json::from_value(json!({
            "posture": {"share_instance": false, "lan": {"ifac": true}},
            "region": "EU_868",
            "callsign": "W1AW",
            "order": ["111", "meshtastic"],
        }))
        .unwrap();
        p.check().unwrap();
        p.merge(&mut f);
        assert!(!f.posture.share_instance && f.posture.lan.backbone.ifac);
        assert_eq!(
            f.posture.lan.backbone.port,
            before.posture.lan.backbone.port
        );
        assert_eq!(f.radio.region.as_deref(), Some("EU_868"));
        assert_eq!(f.radio.callsign(), Some("W1AW"));
        assert_eq!(f.order.networks, ["111", "meshtastic"]);
        let p: TransportPatch = serde_json::from_value(json!({"callsign": null})).unwrap();
        p.merge(&mut f);
        assert_eq!(f.radio.callsign(), None);
        assert!(serde_json::from_value::<TransportPatch>(json!({"surprise": 1})).is_err());
        let bad: TransportPatch =
            serde_json::from_value(json!({"posture": {"lan": {"port": 80}}})).unwrap();
        assert_eq!(bad.check().unwrap_err().status, 400);
        let bad: TransportPatch =
            serde_json::from_value(json!({"posture": {"instance_name": ""}})).unwrap();
        assert_eq!(bad.check().unwrap_err().status, 400);
    }
}
