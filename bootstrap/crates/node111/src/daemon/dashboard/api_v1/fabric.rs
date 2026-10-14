//! `/api/v1/fabric*` (`os/messaging/README.md` §7.3, §9, §15; contract
//! `getFabric`, `enrollHub`, `offerHub`, `dismissHubOffer`,
//! `patchFabricSettings`).
//!
//! - `GET /fabric` → `FabricStatus` (links, home hub, custody, store,
//!   settings incl. per-class quotas). While the fabric is not running the
//!   same shape answers with `store.open: false`.
//! - `POST /fabric/hub/enroll {address, code, card?}` (member side): `409`
//!   while this node already has a home hub (one per node in v1); waits for
//!   `ENROLL_OK` up to the configured bound (the fabric has no timers — the
//!   route bounds the wait); unreachable or refused ⇒ `502 upstream_error`,
//!   the hub's lockout ⇒ `429 rate_limited`; `200 FabricHub` on success.
//!   `address` is the contract's `host:port`: an IP literal (`ip:port`,
//!   `[v6]:port`) is used as is; a host name (the Settings page and the CLI
//!   accept one) is resolved once, here, bounded by [`RESOLVE_TIMEOUT`],
//!   and its first IPv4 answer (else the first answer) is dialed and
//!   persisted (a name that does not resolve ⇒ `502 upstream_error`). The
//!   home hub is then kept by address, not by name (v1).
//!   A wait that times out leaves the enrollment pending inside the fabric
//!   (its dial stays up): if the hub answers later the node still enrolls
//!   and a `fabric` frame reports it.
//! - `POST /fabric/hub/offer` → `HubOffer` (`Cache-Control: no-store`; the
//!   code is shown once) — loopback-only (`403 loopback_only`); `409
//!   bad_request` when this node holds no custody (turn on the hub role
//!   first), `409 restore_pending` while it has no card to put in the bundle.
//! - `DELETE /fabric/hub/offer` → `204` (idempotent) — loopback-only.
//! - `PATCH /fabric/settings` → `FabricSettings` — loopback-only; unknown
//!   keys, unknown quota classes and wrong types are `400`. The listener is
//!   reconciled before the answer (`listen_lan` takes effect immediately).
//!
//! The agent's bearer reads `GET /fabric` (§17 `fabric.read`) and nothing
//! else here: enroll, offer, dismiss and settings are `401` for it.

use super::auth::LoopbackOnly;
use super::messages::{bad_request, handle, refuse_bearer};
use super::{ApiRouter, ApiState, Problem};
use crate::daemon::fabric::{self, json, FabricHandle};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::Json;
use mesh111::fabric::{FabricError, SettingsPatch};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/fabric", get(status))
        .route("/api/v1/fabric/hub/enroll", post(enroll))
        .route("/api/v1/fabric/hub/offer", post(offer).delete(dismiss))
        .route("/api/v1/fabric/settings", patch(settings))
}

async fn status(State(api): State<Arc<ApiState>>) -> Response {
    let Some(h) = api.ext::<Arc<FabricHandle>>() else {
        return fabric::not_running().into_response();
    };
    match h.status_json().await {
        Ok(v) => Json(v).into_response(),
        Err(e) => fabric::problem(&e).into_response(),
    }
}

/// How long a host name in `address` may take to resolve.
pub const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest host name accepted (RFC 1035).
const HOST_MAX: usize = 253;

/// The hub address of an enrollment request (contract `host:port`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum HubAddr {
    /// An IP literal with its port.
    Ip(SocketAddr),
    /// A host name and port, resolved by the route.
    Name(String, u16),
}

impl std::fmt::Display for HubAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HubAddr::Ip(a) => write!(f, "{a}"),
            HubAddr::Name(h, p) => write!(f, "{h}:{p}"),
        }
    }
}

/// `host:port`: an IP literal, or a DNS-style name (letters, digits, `-`
/// and `.`, starting and ending with a letter or digit, ≤ 253, a last label
/// that is not all digits) and a port 1–65535. Anything else (a scheme, a
/// path, a bare host, a malformed IP) is `None`.
fn parse_hub_addr(raw: &str) -> Option<HubAddr> {
    let t = raw.trim();
    if let Ok(a) = t.parse::<SocketAddr>() {
        return (a.port() != 0).then_some(HubAddr::Ip(a));
    }
    let (host, port) = t.rsplit_once(':')?;
    let port = port
        .parse::<u16>()
        .ok()
        .filter(|p| *p != 0 && port.bytes().all(|b| b.is_ascii_digit()))?;
    let edge = |b: Option<&u8>| b.is_some_and(u8::is_ascii_alphanumeric);
    let ok = !host.is_empty()
        && host.len() <= HOST_MAX
        && edge(host.as_bytes().first())
        && edge(host.as_bytes().last())
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        && !host.contains("..")
        // RFC 3696 §2: a top-level label is never all-numeric — and a
        // numeric-looking host (`1.2.3`) would reach `inet_aton`'s legacy
        // forms in the resolver.
        && host
            .rsplit('.')
            .next()
            .is_some_and(|l| l.bytes().any(|b| b.is_ascii_alphabetic()));
    ok.then(|| HubAddr::Name(host.to_ascii_lowercase(), port))
}

/// The socket address to dial for `a` (a name: resolved within
/// [`RESOLVE_TIMEOUT`]; the first IPv4 answer is preferred — a hub's
/// listener binds `0.0.0.0:11183` in v1 — else the resolver's first).
async fn resolve(a: &HubAddr) -> Result<SocketAddr, Problem> {
    let (host, port) = match a {
        HubAddr::Ip(s) => return Ok(*s),
        HubAddr::Name(h, p) => (h.clone(), *p),
    };
    let unreachable = |why: String| {
        Problem::new(
            StatusCode::BAD_GATEWAY,
            "upstream_error",
            "The hub's name did not resolve.",
        )
        .detail(why)
    };
    let answer = tokio::time::timeout(
        RESOLVE_TIMEOUT,
        tokio::net::lookup_host((host.as_str(), port)),
    )
    .await;
    match answer {
        Ok(Ok(addrs)) => {
            let all: Vec<SocketAddr> = addrs.collect();
            all.iter()
                .find(|a| a.is_ipv4())
                .or_else(|| all.first())
                .copied()
                .ok_or_else(|| {
                    unreachable(format!(
                        "{host} has no address; type the hub's IP address instead."
                    ))
                })
        }
        Ok(Err(e)) => Err(unreachable(format!(
            "{host}: {e}; type the hub's IP address instead."
        ))),
        Err(_) => Err(unreachable(format!(
            "{host} did not resolve within {} s; type the hub's IP address instead.",
            RESOLVE_TIMEOUT.as_secs()
        ))),
    }
}

struct EnrollReq {
    address: HubAddr,
    code: String,
    card: Option<String>,
}

fn parse_enroll(body: &[u8]) -> Result<EnrollReq, Problem> {
    let shape = "Body must be {\"address\": \"host:port\", \"code\": <8 characters>, \"card\"?: <the hub's card>}.";
    let obj = match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(o)) => o,
        _ => return Err(bad_request(shape)),
    };
    let mut address = None;
    let mut code = None;
    let mut card = None;
    for (k, v) in obj {
        match (k.as_str(), v) {
            ("address", Value::String(s)) => address = Some(s),
            ("code", Value::String(s)) => code = Some(s),
            ("card", Value::String(s)) => card = Some(s),
            _ => return Err(bad_request(shape)),
        }
    }
    let (Some(address), Some(code)) = (address, code) else {
        return Err(bad_request(shape));
    };
    let address = parse_hub_addr(&address).ok_or_else(|| {
        bad_request(
            "address must be the hub's host:port, like 192.168.1.20:11183 (no scheme, no path).",
        )
    })?;
    Ok(EnrollReq {
        address,
        code,
        card: card.filter(|c| !c.trim().is_empty()),
    })
}

async fn enroll(State(api): State<Arc<ApiState>>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(p) = refuse_bearer(&headers) {
        return p.into_response();
    }
    let req = match parse_enroll(&body) {
        Ok(r) => r,
        Err(p) => return p.into_response(),
    };
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    let Some(f) = h.fabric() else {
        return fabric::not_running().into_response();
    };
    match f.settings().await {
        Ok(s) if s.home_hub.is_some() => {
            return Problem::new(
                StatusCode::CONFLICT,
                "bad_request",
                "This node already has a home hub.",
            )
            .detail("A node has one home hub in v1; forget the current hub's contact first.")
            .into_response()
        }
        Ok(_) => {}
        Err(e) => return fabric::problem(&e).into_response(),
    }
    let address = match resolve(&req.address).await {
        Ok(a) => a,
        Err(p) => return p.into_response(),
    };
    let wait = h.enroll_timeout();
    let r = tokio::time::timeout(wait, f.enroll(address, &req.code, req.card.as_deref())).await;
    match r {
        Ok(Ok(())) => match f.status().await {
            Ok(st) => {
                h.refresh_facts().await;
                Json(json::hub(&st.hub)).into_response()
            }
            Err(e) => fabric::problem(&e).into_response(),
        },
        Ok(Err(e)) => fabric::problem(&e).into_response(),
        Err(_) => Problem::new(
            StatusCode::BAD_GATEWAY,
            "upstream_error",
            "The hub did not answer.",
        )
        .detail(format!(
            "No enrollment answer from {} within {} s; check the address and that the hub is on.",
            req.address,
            wait.as_secs()
        ))
        .into_response(),
    }
}

async fn offer(
    _lo: LoopbackOnly,
    State(api): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> Response {
    if let Err(p) = refuse_bearer(&headers) {
        return p.into_response();
    }
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    let Some(f) = h.fabric() else {
        return fabric::not_running().into_response();
    };
    if h.own_announce().is_empty() {
        // §4: no announce yet ⇒ no card for the bundle; a member could not
        // seal its ENROLL to this hub.
        return Problem::new(
            StatusCode::CONFLICT,
            "restore_pending",
            "This node has no card yet, so it cannot offer.",
        )
        .detail("Nothing is announced until the identity restore completes (§4).")
        .into_response();
    }
    let address = h.offer_address().await;
    let view = match f.offer(address).await {
        Ok(v) => v,
        Err(FabricError::Invalid(_)) => return Problem::new(
            StatusCode::CONFLICT,
            "bad_request",
            "This node is not a hub.",
        )
        .detail(
            "It holds no mail for other nodes: turn on \"Let other nodes use this one as their \
                 home hub\" (or custody opt-in) first, then offer again.",
        )
        .into_response(),
        Err(e) => return fabric::problem(&e).into_response(),
    };
    match json::offer(&view) {
        Ok(v) => {
            let mut resp = Json(v).into_response();
            resp.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            resp
        }
        Err(e) => Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "The offer could not be drawn.",
        )
        .detail(e)
        .into_response(),
    }
}

async fn dismiss(
    _lo: LoopbackOnly,
    State(api): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> Response {
    if let Err(p) = refuse_bearer(&headers) {
        return p.into_response();
    }
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    let Some(f) = h.fabric() else {
        return fabric::not_running().into_response();
    };
    match f.dismiss_offer().await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => fabric::problem(&e).into_response(),
    }
}

/// `FabricSettingsPatch` from the body.
fn parse_settings(body: &[u8]) -> Result<SettingsPatch, Problem> {
    let obj = match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(o)) => o,
        _ => return Err(bad_request("Body must be a JSON object.")),
    };
    let mut p = SettingsPatch::default();
    for (k, v) in obj {
        match (k.as_str(), v) {
            ("hub_role", Value::Bool(b)) => p.hub_role = Some(b),
            ("listen_lan", Value::Bool(b)) => p.listen_lan = Some(b),
            ("custody_opt_in", Value::Bool(b)) => p.custody_opt_in = Some(b),
            ("quotas", Value::Object(q)) => {
                for (name, v) in q {
                    let class = json::quota_class(&name).ok_or_else(|| {
                        bad_request(format!(
                            "{name:?} is not a quota class (kb, update, patch, sync, object, custody)."
                        ))
                    })?;
                    let quota = match v {
                        Value::Null => None,
                        Value::Number(n) if n.as_u64().is_some() => n.as_u64(),
                        _ => {
                            return Err(bad_request(format!(
                                "quota {name:?} must be a non-negative integer or null."
                            )))
                        }
                    };
                    p.quotas.push((class, quota));
                }
            }
            (other, _) => {
                return Err(bad_request(format!(
                    "{other:?} is not a fabric setting (hub_role, listen_lan, custody_opt_in, quotas) or has the wrong type."
                )))
            }
        }
    }
    Ok(p)
}

async fn settings(
    _lo: LoopbackOnly,
    State(api): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(p) = refuse_bearer(&headers) {
        return p.into_response();
    }
    let patch = match parse_settings(&body) {
        Ok(p) => p,
        Err(p) => return p.into_response(),
    };
    let h = match handle(&api) {
        Ok(h) => h,
        Err(p) => return p.into_response(),
    };
    match h.patch_settings(patch).await {
        Ok(s) => {
            let q = h.quotas().await;
            h.refresh_facts().await;
            Json(json::settings(&s, &q)).into_response()
        }
        Err(e) => fabric::problem(&e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_body() {
        let p = parse_settings(
            br#"{"hub_role":true,"listen_lan":false,"quotas":{"kb":1000,"custody":null}}"#,
        )
        .unwrap();
        assert_eq!(p.hub_role, Some(true));
        assert_eq!(p.listen_lan, Some(false));
        assert_eq!(p.custody_opt_in, None);
        assert_eq!(p.quotas.len(), 2);
        for bad in [
            &br#"{"hub_role":"on"}"#[..],
            br#"{"quotas":{"human":5}}"#,
            br#"{"quotas":{"kb":-1}}"#,
            br#"{"quotas":{"msg":5}}"#,
            br#"{"request_quota":5}"#,
            br#"[]"#,
        ] {
            assert!(
                parse_settings(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn enroll_body() {
        let r = parse_enroll(br#"{"address":"192.168.1.4:11183","code":"ABCD-EFGH"}"#).unwrap();
        assert_eq!(r.address, HubAddr::Ip("192.168.1.4:11183".parse().unwrap()));
        assert!(r.card.is_none());
        let r = parse_enroll(br#"{"address":"[fe80::1]:11183","code":"x","card":"C"}"#).unwrap();
        assert_eq!(r.card.as_deref(), Some("C"));
        // a host name is accepted (U2: the Settings page sends host:port)
        let r = parse_enroll(br#"{"address":" Hub-1.local:11183 ","code":"x"}"#).unwrap();
        assert_eq!(r.address, HubAddr::Name("hub-1.local".into(), 11183));
        assert_eq!(r.address.to_string(), "hub-1.local:11183");
        for bad in [
            &br#"{"address":"10.0.0.1","code":"x"}"#[..],
            br#"{"address":"hub.local","code":"x"}"#,
            br#"{"address":"hub.local:0","code":"x"}"#,
            br#"{"address":"hub.local:70000","code":"x"}"#,
            br#"{"address":"hub.local:+80","code":"x"}"#,
            br#"{"address":"http://hub.local:11183","code":"x"}"#,
            br#"{"address":"hub.local:11183/x","code":"x"}"#,
            br#"{"address":"-hub:11183","code":"x"}"#,
            br#"{"address":"hub..local:11183","code":"x"}"#,
            br#"{"address":"hub_1:11183","code":"x"}"#,
            br#"{"address":"10.0.0.1:0","code":"x"}"#,
            br#"{"address":"1.2.3:11183","code":"x"}"#,
            br#"{"address":"999.1.1.1:11183","code":"x"}"#,
            br#"{"address":"123:11183","code":"x"}"#,
            br#"{"code":"x"}"#,
            br#"{"address":"10.0.0.1:1","code":5}"#,
            br#"{"address":"10.0.0.1:1","code":"x","extra":1}"#,
        ] {
            assert!(
                parse_enroll(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        let long = format!("{}:1", "a".repeat(254));
        assert!(parse_hub_addr(&long).is_none());
        assert!(parse_hub_addr(&format!("{}:1", "a".repeat(253))).is_some());
    }

    #[tokio::test]
    async fn names_resolve_and_literals_pass_through() {
        let lit = HubAddr::Ip("10.1.2.3:11183".parse().unwrap());
        assert_eq!(
            resolve(&lit).await.unwrap(),
            "10.1.2.3:11183".parse::<SocketAddr>().unwrap()
        );
        // `localhost` resolves from /etc/hosts on every Linux host
        let a = resolve(&HubAddr::Name("localhost".into(), 11183))
            .await
            .unwrap();
        assert!(a.ip().is_loopback() && a.port() == 11183, "{a}");
        // the reserved `.invalid` TLD never resolves (RFC 6761)
        let p = resolve(&HubAddr::Name("hub.invalid".into(), 11183))
            .await
            .unwrap_err();
        assert_eq!(p.status, 502);
        assert_eq!(p.code, "upstream_error");
    }
}
