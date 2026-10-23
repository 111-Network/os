//! `/api/v1/bridge/*` (Layer 6 `os/6-transport/README.md` §8, §2.1, §10; contract
//! `listBridgeEdges`, `createBridgeEdge`, `getBridgeEdge`, `patchBridgeEdge`,
//! `deleteBridgeEdge`, `getBridgeSettings`, `patchBridgeSettings`, `listBridgeComponents`,
//! `enableBridgeComponent`, `disableBridgeComponent`, `removeBridgeComponentFiles`).
//!
//! - **Edges** (§8): `POST /bridge/edges {src, dst, honour_ok_to_mqtt?, oversize?,
//!   accept_warning?}` — without the hash of the warning as it reads now the answer is
//!   `409 consent_required` carrying the exact text and its hash (`consent`); with it,
//!   `201 BridgeEdge` (`enabled_by` = the session's label, [`auth::session_label`]). A DM
//!   endpoint or `src = dst` is `400`; an id that names no conversation `404`. `PATCH`
//!   pauses (always), resumes (a `consent_paused` edge needs the NEW text's hash: the same
//!   409 round-trip) and sets `honour_ok_to_mqtt` / `oversize`; `DELETE` is `204`.
//! - **Settings** (§5, §6, §11): `GET` mirrors `state/bridge.json` with the preferred order
//!   from `interfaces.toml` (the spine owns `[order]`); `PATCH` hands `order` to the spine
//!   (validated before anything is written) and the rest to the bridge core, where a HAM
//!   confirmation or an uplinked channel's send switch is never a bare patch — the 409
//!   round-trip with the text hash.
//! - **Components** (§2.1): `GET` lists the optional components with their weight facts
//!   (the facts walk the installed tree, so they run on a blocking thread, S4 review
//!   minor); `enable {accept_notice_hash?}` answers `409 consent_required` with the licence
//!   notice until the hash is echoed, `409 component_unavailable` when the cache misses or
//!   fails its pin or `[requires]` is unmet (nothing installed), else `200 Component` (the
//!   contract's success; the child may still wait for its dependency, `reason_code`
//!   says so). A component whose network has an adapter factory registered
//!   ([`ComponentAdapters`], the C3/C5 seam) gets its adapter enabled in the bridge core
//!   after enable and disabled before disable (its Nearby source cleared with it); a
//!   component with a [`PrepareHook`] (the LXMF helper's client config and run arguments,
//!   W1) has it run once the licence notice passes and before its child starts — a
//!   failing hook answers the enable's problem and nothing starts ([`enable_component`]). `disable` is idempotent; `remove-files
//!   {cache?}` deletes the installed tree and `bridge/<adapter>/` (and with `cache: true`
//!   the cached payload — root-owned, so the daemon user's `EACCES` is a clear `500`).
//!
//! Every write here is a human act: the global write gate refuses the agent's bearer
//! (`401`, `auth::require_session`, §13); CSRF/JSON rules apply.

use super::auth;
use super::{ApiRouter, ApiState, Problem};
use crate::daemon::bridge::edges::{EdgeOpts, EdgePatch};
use crate::daemon::bridge::settings::SettingsPatch;
use crate::daemon::bridge::{Adapter, Bridge, BridgeError, SizeMode};
use crate::daemon::components::{Components, EnableError, Enabled, Status};
use crate::daemon::fabric::json as fjson;
use axum::body::Bytes;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{Extensions, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use futures_util::future::BoxFuture;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub fn router() -> ApiRouter {
    axum::Router::new()
        .route("/api/v1/bridge/edges", get(edges).post(create_edge))
        .route(
            "/api/v1/bridge/edges/{id}",
            get(edge).patch(patch_edge).delete(delete_edge),
        )
        .route(
            "/api/v1/bridge/settings",
            get(settings).patch(patch_settings),
        )
        .route("/api/v1/bridge/components", get(components))
        .route("/api/v1/bridge/components/{id}/enable", post(enable))
        .route("/api/v1/bridge/components/{id}/disable", post(disable))
        .route(
            "/api/v1/bridge/components/{id}/remove-files",
            post(remove_files),
        )
}

// ---------------------------------------------------------------- shared helpers

/// The bridge core, or `503` (no bridge in this daemon: the compat entry, tools).
pub(crate) fn bridge_of(api: &ApiState) -> Result<Arc<Bridge>, Problem> {
    api.ext::<Arc<Bridge>>().ok_or_else(|| {
        Problem::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "internal",
            "The bridge is not running on this node.",
        )
        .detail("The bridge core starts with the message store; see the Messaging service page.")
    })
}

/// `400 bad_request` with a detail.
pub(crate) fn bad_request(detail: impl Into<String>) -> Problem {
    Problem::new(
        StatusCode::BAD_REQUEST,
        "bad_request",
        "The request was refused.",
    )
    .detail(detail)
}

/// `404 not_found` naming `what`.
pub(crate) fn not_found(what: &str) -> Problem {
    Problem::new(
        StatusCode::NOT_FOUND,
        "not_found",
        format!("No such {what}."),
    )
}

/// A JSON body (size-capped by the framework before this runs; unknown fields are refused
/// where the type says so). An empty body reads as `{}` (the optional bodies).
pub(crate) fn body<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Problem> {
    let text: &[u8] = if bytes.iter().all(u8::is_ascii_whitespace) {
        b"{}"
    } else {
        bytes
    };
    serde_json::from_slice(text).map_err(|e| bad_request(format!("The body does not fit: {e}.")))
}

/// Lowercase SHA-256 hex (`Sha256Hex`), or `400`.
pub(crate) fn check_hash(h: Option<&str>) -> Result<(), Problem> {
    match h {
        Some(h)
            if !(h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))) =>
        {
            Err(bad_request(
                "accept_warning must be the 64-hex SHA-256 the consent carried.",
            ))
        }
        _ => Ok(()),
    }
}

/// A bridge error as the contract's problem (with its extension members).
pub(crate) fn bridge_problem(e: BridgeError) -> Response {
    e.problem().into_response()
}

/// A component's facts as the contract's `Component` (no `title`).
pub(crate) fn component_json(s: &Status) -> Value {
    let mut v = serde_json::to_value(s).unwrap_or(Value::Null);
    if let Value::Object(o) = &mut v {
        o.remove("title");
    }
    v
}

/// Every component's facts, read on a blocking thread (the facts walk the installed tree
/// under the runners' lock: never on a runtime worker).
pub(crate) async fn components_list(c: Components) -> Vec<Status> {
    let h = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || h.block_on(c.list()))
        .await
        .unwrap_or_default()
}

// ---------------------------------------------------------------- the adapter seam

/// Builds the bridge adapter of an enabled component (C3: the LXMF helper's adapter
/// attaches to the component's piped stdio; C5: the app peers). Registered once at daemon
/// start, keyed by component id.
pub type AdapterFactory =
    Arc<dyn Fn(Components) -> BoxFuture<'static, Result<Arc<dyn Adapter>, String>> + Send + Sync>;

/// Runs before a component's child is started, once its licence notice is acknowledged
/// (W1, the C3rs seam: the LXMF helper's Reticulum client config written with rnsd's
/// `rpc_key`, §6 (1), §11): prepares what the child needs on disk and answers the
/// arguments the adapter appends to `[run] args` (`lxmf::run_args`). An error refuses the
/// enable with the contract's problem and nothing starts (fail closed).
pub type PrepareHook =
    Arc<dyn Fn() -> BoxFuture<'static, Result<Vec<String>, EnableError>> + Send + Sync>;

/// Undoes what a component's [`PrepareHook`] wrote when the enable it prepared is refused
/// afterwards (C1-bounce F4: the hook runs before `Components::enable_with`'s
/// `[requires]`, cache and pin checks; the LXMF helper's client config — a 0600 copy of
/// rnsd's `rpc_key` — must not stay on disk for a component that did not start). Best
/// effort; runs only while the component has no running child.
pub type UnprepareHook = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

/// The registration seam between optional components and the bridge core (module doc).
/// With nothing registered, enabling a component starts its child only.
#[derive(Default)]
pub struct ComponentAdapters {
    map: RwLock<HashMap<String, AdapterFactory>>,
    prepare: RwLock<HashMap<String, PrepareHook>>,
    unprepare: RwLock<HashMap<String, UnprepareHook>>,
}

impl ComponentAdapters {
    /// Register the factory of `component` (a second registration replaces the first).
    pub fn register(&self, component: &str, f: AdapterFactory) {
        self.map
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(component.to_string(), f);
    }

    /// Register the hook that runs before `component`'s child starts ([`PrepareHook`]; a
    /// second registration replaces the first).
    pub fn register_prepare(&self, component: &str, f: PrepareHook) {
        self.prepare
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(component.to_string(), f);
    }

    /// Register the undo of `component`'s [`PrepareHook`] ([`UnprepareHook`]; a second
    /// registration replaces the first).
    pub fn register_unprepare(&self, component: &str, f: UnprepareHook) {
        self.unprepare
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(component.to_string(), f);
    }

    /// Whether `component` has an adapter factory registered.
    pub fn registered(&self, component: &str) -> bool {
        self.get(component).is_some()
    }

    /// Whether `component` has a [`PrepareHook`] registered (W3: Madmail has a hook and no
    /// factory of its own).
    pub fn prepares(&self, component: &str) -> bool {
        self.prepare_of(component).is_some()
    }

    fn get(&self, component: &str) -> Option<AdapterFactory> {
        self.map
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(component)
            .cloned()
    }

    fn prepare_of(&self, component: &str) -> Option<PrepareHook> {
        self.prepare
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(component)
            .cloned()
    }

    fn unprepare_of(&self, component: &str) -> Option<UnprepareHook> {
        self.unprepare
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(component)
            .cloned()
    }
}

/// Enable `component` (the route's and `Layer6::resume`'s one path): when a
/// [`PrepareHook`] is registered and the licence notice passes (`ack` is the notice's
/// hash, or none and the acknowledgement is recorded), the hook runs first and its
/// arguments go to `Components::enable_with`; a failing hook refuses the enable (nothing
/// installed past what was there, nothing started). Without a valid acknowledgement the
/// hook does not run and the enable answers `consent_required` as usual (nothing is
/// written for a component the user has not accepted, §2.1 rule 2). When the hook ran
/// and `enable_with` then refuses (its `[requires]`, cache or pin checks) while the
/// component has no running child, its [`UnprepareHook`] removes what the hook wrote
/// (C1-bounce F4).
pub async fn enable_component(
    api: &ApiState,
    components: &Components,
    component: &str,
    ack: Option<&str>,
) -> Result<Enabled, EnableError> {
    let reg = api.ext::<Arc<ComponentAdapters>>();
    let hook = reg.as_ref().and_then(|r| r.prepare_of(component));
    let mut args = Vec::new();
    let mut prepared = false;
    if let Some(hook) = hook {
        let accepted = components
            .status(component)
            .await
            .is_some_and(|st| match ack {
                Some(h) => h == st.notice_hash,
                None => st.acknowledged,
            });
        if accepted {
            args = hook().await?;
            prepared = true;
        }
    }
    let res = components.enable_with(component, ack, args).await;
    if res.is_err() && prepared && components.stdio(component).await.is_none() {
        if let Some(undo) = reg.as_ref().and_then(|r| r.unprepare_of(component)) {
            undo().await;
        }
    }
    res
}

/// After an enable: build and enable the component's adapter in the bridge core, when a
/// factory is registered and its network is not enabled yet. Failures are logged; the
/// component keeps running (its network reads `error` until the adapter attaches).
pub async fn attach_adapter(api: &ApiState, components: &Components, component: &str) {
    let (Some(reg), Some(bridge)) = (
        api.ext::<Arc<ComponentAdapters>>(),
        api.ext::<Arc<Bridge>>(),
    ) else {
        return;
    };
    let Some(f) = reg.get(component) else { return };
    let Some(network) = components.get(component).map(|c| c.adapter.clone()) else {
        return;
    };
    if bridge.enabled().contains(&network) {
        return;
    }
    match f(components.clone()).await {
        Ok(adapter) => {
            if let Err(e) = bridge.enable(adapter).await {
                eprintln!("111 bridge: {component}: the adapter was not enabled: {e}");
            }
        }
        Err(e) => eprintln!("111 bridge: {component}: the adapter was not built: {e}"),
    }
}

/// Before a disable: disable the component's network in the bridge core (whoever enabled
/// its adapter: nothing reads a child that is about to stop).
pub async fn detach_adapter(api: &ApiState, components: &Components, component: &str) {
    let Some(bridge) = api.ext::<Arc<Bridge>>() else {
        return;
    };
    if let Some(network) = components.get(component).map(|c| c.adapter.clone()) {
        bridge.disable(&network).await;
        // its Nearby source goes with it (W1: a stopped helper hears nothing)
        if let Some(nearby) = api.ext::<Arc<super::nearby::BridgeNearby>>() {
            if nearby.clear(&network) {
                super::nearby::publish(api);
            }
        }
    }
}

// ---------------------------------------------------------------- edges

/// `be-<n>` → the edge row id.
fn edge_id(id: Result<Path<String>, PathRejection>) -> Result<i64, Problem> {
    let Ok(Path(id)) = id else {
        return Err(not_found("edge"));
    };
    fjson::parse_be_id(&id).ok_or_else(|| not_found("edge"))
}

/// `fc-<n>` → the conversation row id (`404`: an id that names nothing).
pub(crate) fn conv_id(id: &str) -> Result<i64, Problem> {
    fjson::parse_fc_id(id).ok_or_else(|| not_found("conversation"))
}

async fn edges(State(api): State<Arc<ApiState>>) -> Response {
    let b = match bridge_of(&api) {
        Ok(b) => b,
        Err(p) => return p.into_response(),
    };
    match b.edges().await {
        Ok(v) => Json(v).into_response(),
        Err(e) => bridge_problem(e),
    }
}

async fn edge(
    State(api): State<Arc<ApiState>>,
    id: Result<Path<String>, PathRejection>,
) -> Response {
    let (id, b) = match edge_id(id).and_then(|id| Ok((id, bridge_of(&api)?))) {
        Ok(x) => x,
        Err(p) => return p.into_response(),
    };
    match b.edge(id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => bridge_problem(e),
    }
}

/// `BridgeEdgeCreate`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EdgeCreate {
    src: String,
    dst: String,
    #[serde(default)]
    honour_ok_to_mqtt: bool,
    #[serde(default)]
    oversize: SizeMode,
    #[serde(default)]
    accept_warning: Option<String>,
}

async fn create_edge(
    State(api): State<Arc<ApiState>>,
    headers: HeaderMap,
    ext: Extensions,
    bytes: Bytes,
) -> Response {
    let req: EdgeCreate = match body(&bytes) {
        Ok(r) => r,
        Err(p) => return p.into_response(),
    };
    let (src, dst) = match (conv_id(&req.src), conv_id(&req.dst)) {
        (Ok(s), Ok(d)) => (s, d),
        (Err(p), _) | (_, Err(p)) => return p.into_response(),
    };
    if let Err(p) = check_hash(req.accept_warning.as_deref()) {
        return p.into_response();
    }
    let b = match bridge_of(&api) {
        Ok(b) => b,
        Err(p) => return p.into_response(),
    };
    let by = auth::session_label(&api, &headers, &ext);
    let opts = EdgeOpts {
        honour_ok_to_mqtt: req.honour_ok_to_mqtt,
        oversize: req.oversize,
    };
    match b
        .create_edge(src, dst, opts, req.accept_warning.as_deref(), &by)
        .await
    {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => bridge_problem(e),
    }
}

async fn patch_edge(
    State(api): State<Arc<ApiState>>,
    id: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
    ext: Extensions,
    bytes: Bytes,
) -> Response {
    let id = match edge_id(id) {
        Ok(i) => i,
        Err(p) => return p.into_response(),
    };
    let p: EdgePatch = match body(&bytes) {
        Ok(p) => p,
        Err(p) => return p.into_response(),
    };
    if let Err(p) = check_hash(p.accept_warning.as_deref()) {
        return p.into_response();
    }
    let b = match bridge_of(&api) {
        Ok(b) => b,
        Err(p) => return p.into_response(),
    };
    let by = auth::session_label(&api, &headers, &ext);
    match b.patch_edge(id, &p, &by).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => bridge_problem(e),
    }
}

async fn delete_edge(
    State(api): State<Arc<ApiState>>,
    id: Result<Path<String>, PathRejection>,
) -> Response {
    let id = match edge_id(id) {
        Ok(i) => i,
        Err(p) => return p.into_response(),
    };
    let b = match bridge_of(&api) {
        Ok(b) => b,
        Err(p) => return p.into_response(),
    };
    match b.delete_edge(id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => not_found("edge").into_response(),
        Err(e) => bridge_problem(e),
    }
}

// ---------------------------------------------------------------- settings

/// The preferred order from the spine (`interfaces.toml` `[order]`), when one is wired.
fn spine_order(api: &ApiState) -> Option<Vec<String>> {
    api.ext::<Arc<super::transport::TransportApi>>()
        .map(|t| t.spine.file().order.networks)
}

async fn settings(State(api): State<Arc<ApiState>>) -> Response {
    let b = match bridge_of(&api) {
        Ok(b) => b,
        Err(p) => return p.into_response(),
    };
    match b.settings().await {
        Ok(mut v) => {
            if let Some(order) = spine_order(&api) {
                v["order"] = json!(order);
            }
            Json(v).into_response()
        }
        Err(e) => bridge_problem(e),
    }
}

async fn patch_settings(
    State(api): State<Arc<ApiState>>,
    headers: HeaderMap,
    ext: Extensions,
    bytes: Bytes,
) -> Response {
    let mut p: SettingsPatch = match body(&bytes) {
        Ok(p) => p,
        Err(p) => return p.into_response(),
    };
    for h in p
        .channels
        .iter()
        .map(|c| c.accept_warning.as_deref())
        .chain(p.ham.iter().map(|h| h.accept_warning.as_deref()))
    {
        if let Err(pr) = check_hash(h) {
            return pr.into_response();
        }
    }
    let b = match bridge_of(&api) {
        Ok(b) => b,
        Err(pr) => return pr.into_response(),
    };
    // the order belongs to the spine: validated first, written after the rest succeeded
    let order = p.order.take();
    let spine = api
        .ext::<Arc<super::transport::TransportApi>>()
        .map(|t| Arc::clone(&t.spine));
    if let Some(o) = &order {
        let Some(spine) = &spine else {
            return bad_request("The preferred order is not settable on this node.")
                .into_response();
        };
        let mut f = spine.file();
        f.order.networks = o.clone();
        if let Err(e) = f.validate() {
            return bad_request(format!("order: {e}")).into_response();
        }
    }
    let by = auth::session_label(&api, &headers, &ext);
    let mut v = match b.patch_settings(&p, &by).await {
        Ok(v) => v,
        Err(e) => return bridge_problem(e),
    };
    if let (Some(o), Some(spine)) = (order, spine) {
        if let Err(e) = spine.update(move |f| f.order.networks = o).await {
            return super::transport::apply_problem(&e).into_response();
        }
    }
    if let Some(order) = spine_order(&api) {
        v["order"] = json!(order);
    }
    Json(v).into_response()
}

// ---------------------------------------------------------------- components

fn components_of(api: &ApiState) -> Result<Components, Problem> {
    api.ext::<Components>().ok_or_else(|| {
        Problem::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "internal",
            "Optional components are not available on this node.",
        )
    })
}

fn component_id(id: Result<Path<String>, PathRejection>) -> Result<String, Problem> {
    match id {
        Ok(Path(id)) if crate::daemon::components::manifest::is_component_id(&id) => Ok(id),
        _ => Err(not_found("component")),
    }
}

async fn components(State(api): State<Arc<ApiState>>) -> Response {
    let c = match components_of(&api) {
        Ok(c) => c,
        Err(p) => return p.into_response(),
    };
    let list: Vec<Value> = components_list(c)
        .await
        .iter()
        .map(component_json)
        .collect();
    Json(json!({ "schema": 1, "components": list })).into_response()
}

/// `ComponentEnableRequest`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnableReq {
    #[serde(default)]
    accept_notice_hash: Option<String>,
}

/// `ComponentRemoveRequest`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveReq {
    #[serde(default)]
    cache: bool,
}

async fn enable(
    State(api): State<Arc<ApiState>>,
    id: Result<Path<String>, PathRejection>,
    bytes: Bytes,
) -> Response {
    let id = match component_id(id) {
        Ok(i) => i,
        Err(p) => return p.into_response(),
    };
    let req: EnableReq = match body(&bytes) {
        Ok(r) => r,
        Err(p) => return p.into_response(),
    };
    if let Err(p) = check_hash(req.accept_notice_hash.as_deref()) {
        return p.into_response();
    }
    let c = match components_of(&api) {
        Ok(c) => c,
        Err(p) => return p.into_response(),
    };
    if c.get(&id).is_none() {
        return not_found("component").into_response();
    }
    match enable_component(&api, &c, &id, req.accept_notice_hash.as_deref()).await {
        Ok(done) => {
            attach_adapter(&api, &c, &id).await;
            let status = c.status(&id).await.unwrap_or(done.status);
            if let Some(b) = api.ext::<Arc<Bridge>>() {
                b.publish_snapshot().await;
            }
            Json(component_json(&status)).into_response()
        }
        Err(e) => e.problem(c.get(&id)).into_response(),
    }
}

async fn disable(
    State(api): State<Arc<ApiState>>,
    id: Result<Path<String>, PathRejection>,
    bytes: Bytes,
) -> Response {
    let id = match component_id(id) {
        Ok(i) => i,
        Err(p) => return p.into_response(),
    };
    if let Err(p) = body::<Value>(&bytes) {
        return p.into_response();
    }
    let c = match components_of(&api) {
        Ok(c) => c,
        Err(p) => return p.into_response(),
    };
    if c.get(&id).is_none() {
        return not_found("component").into_response();
    }
    detach_adapter(&api, &c, &id).await;
    match c.disable(&id).await {
        Some(s) => {
            if let Some(b) = api.ext::<Arc<Bridge>>() {
                b.publish_snapshot().await;
            }
            Json(component_json(&s)).into_response()
        }
        None => not_found("component").into_response(),
    }
}

async fn remove_files(
    State(api): State<Arc<ApiState>>,
    id: Result<Path<String>, PathRejection>,
    bytes: Bytes,
) -> Response {
    let id = match component_id(id) {
        Ok(i) => i,
        Err(p) => return p.into_response(),
    };
    let req: RemoveReq = match body(&bytes) {
        Ok(r) => r,
        Err(p) => return p.into_response(),
    };
    let c = match components_of(&api) {
        Ok(c) => c,
        Err(p) => return p.into_response(),
    };
    let mut status = match c.remove_files(&id).await {
        Ok(s) => s,
        Err(e) => return e.problem().into_response(),
    };
    if req.cache {
        status = match c.remove_cache(&id).await {
            Ok(s) => s,
            Err(e) => return e.problem().into_response(),
        };
    }
    if let Some(b) = api.ext::<Arc<Bridge>>() {
        b.publish_snapshot().await;
    }
    Json(component_json(&status)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_hashes_and_ids() {
        assert!(body::<EnableReq>(b"").unwrap().accept_notice_hash.is_none());
        assert!(body::<EnableReq>(b"  \n").is_ok());
        assert_eq!(
            body::<EnableReq>(br#"{"surprise":1}"#).unwrap_err().code,
            "bad_request"
        );
        assert!(body::<RemoveReq>(br#"{"cache":true}"#).unwrap().cache);
        assert!(check_hash(None).is_ok());
        assert!(check_hash(Some(&"a".repeat(64))).is_ok());
        for bad in ["A".repeat(64), "a".repeat(63), "g".repeat(64)] {
            assert_eq!(check_hash(Some(&bad)).unwrap_err().status, 400);
        }
        assert_eq!(conv_id("fc-12").unwrap(), 12);
        for bad in ["fc-", "fc-0", "fc-x", "fm-1", "12"] {
            assert_eq!(conv_id(bad).unwrap_err().status, 404, "{bad}");
        }
    }
}
