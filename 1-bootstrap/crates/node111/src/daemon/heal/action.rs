//! Layer 9 (§6.2 a cause's `action` and "the one non-literal"; §7.2 the v1 vocabulary, closed;
//! §19 hard rule 8; §24 D19, D21; S19): the one action type, filled by row D4.
//!
//! [`Action`] is §7.2's closed vocabulary as data, in the Layer 6 applier's `{"action": name,
//! …args}` form (`N/devices/applier.rs` `Action::parse`): a TOML table or a JSON object tagged on
//! `action` and nothing else (serde's derive alone would also read an array, as the tag and then
//! the fields in order), each key once, every variant `deny_unknown_fields`, an enum field read
//! from its name only, and every field checked as it is read (`params` too: [`Params`]), so a
//! value the vocabulary cannot carry never becomes an `Action`. A playbook cause (§6.2), a
//! `repair` proposal (§7.3) and a received fix (§10.1, row D7) parse into this one type.
//!
//! - **Typed** ([`Kind::Typed`]): `service.restart {service_id}`, `rns.posture.set
//!   {share_instance?, enable_transport?}` (one at least), `radio.region.set {region}`,
//!   `settings.set {key, value}` on the keys `fabric.listen_lan` and `fabric.hub_role` (a
//!   boolean) and `fabric.quota.<class>` (bytes, a class of `PATCH /fabric/settings`' `quotas`;
//!   `lan_enabled` is refused, Q6), `component.enable {id}` and `component.disable {id}`.
//! - **Human or `device.propose` only** (†, [`Kind::HumanOnly`]; §7.2, §24 D19):
//!   `rns.interface.upsert`, `rns.interface.remove`, `bridge.edge.pause`, `bridge.edge.set` and
//!   `bridge.order.set`, typed here so a playbook cause naming one is refused by name (the
//!   grammar's `human_only`), never read as an unknown action.
//! - **`device` proposals only** ([`Kind::DeviceOnly`]): `identify {device_id}`.
//! - **Nothing runs** ([`Kind::Advice`]): `advise {text}` and `human_step {steps}`.
//! - [`Kind::Escalate`]: `escalate {task}`, `task` `diagnose` or `patch`.
//! - Never typed: `firmware.flash`, and every other name: an unknown action.
//!
//! **The one non-literal (§6.2).** A subject field holds a literal of its own set or `"@subject"`
//! ([`Ref`]), the incident's subject: `service.restart`'s `service_id` here, a predicate's `id` in
//! the grammar. `@subject` is a member of the compiled subject table (`heal::vocab`) and nothing
//! else: the grammar refuses it in an `any` playbook, whose subject is a component id, and where
//! the playbook's `[match]` subject is not a value the field takes; `plan()` resolves it to an
//! existing object. A component's `id` takes a literal only. Any other value that starts with `@`
//! is refused wherever it stands, so nothing else is a substitution; a text that holds `@subject`
//! further in is plain text (no templating, no command, no URL, no environment). A value derived
//! from a channel, device or peer name never reaches an action parameter.
//!
//! Static data and parsing only: nothing is applied, read, opened or spawned here (the engine's
//! `Action` trait is row D25's, `heal::engine`).

use crate::daemon::components::manifest::is_component_id;
use crate::daemon::devices::valid_id;
use crate::daemon::fabric::json::quota_class;
use crate::daemon::services::{catalogue, Kind as ServiceKind, ServiceId};
use crate::daemon::transport::interfaces::{MAX_NAME_BYTES, MAX_ORDER};
use serde::de::{self, Deserializer};
use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;

/// The one substitution (§6.2): a subject field's value bound to the incident's subject.
pub const SUBJECT: &str = "@subject";

/// How a refusal for a misplaced `@` value starts: the grammar files it under its `subject` rule.
pub const SUBJECT_MARK: &str = "@subject: ";

/// How a refusal for a service id outside a field's set starts: the grammar's `unknown_service`.
pub const SERVICE_MARK: &str = "unknown service id: ";

/// The longest `advise` text (bytes): a card's paragraph.
pub const MAX_ADVICE: usize = 512;
/// The most steps of one `human_step`.
pub const MAX_STEPS: usize = 8;
/// The longest step (bytes).
pub const MAX_STEP: usize = 256;
/// The longest radio region code (bytes; the applier's bound).
pub const MAX_REGION: usize = 32;
/// The longest interface `type` (bytes; the applier's bound).
pub const MAX_IF_TYPE: usize = 64;
/// The most `params` keys of one `rns.interface.upsert` (the widest stanza allowlist,
/// RNodeInterface's, has 10: `N/transport/interfaces.rs` `RNODE_PARAMS`).
pub const MAX_PARAMS: usize = 16;
/// The longest `params` key (bytes; the allowlists' keys are at most 19).
pub const MAX_PARAM_KEY: usize = 32;
/// The longest `params` text (bytes; a host name's 253 fits).
pub const MAX_PARAM_TEXT: usize = 255;
/// The key prefix of a class quota (§7.2 `fabric.quota.<class>`).
pub const QUOTA_PREFIX: &str = "fabric.quota.";

/// The action names, in §7.2's order: the grammar's pre-pass names an unknown one before serde
/// reads the table.
pub const NAMES: [&str; 15] = [
    "service.restart",
    "identify",
    "rns.interface.upsert",
    "rns.interface.remove",
    "rns.posture.set",
    "radio.region.set",
    "settings.set",
    "component.enable",
    "component.disable",
    "bridge.edge.pause",
    "bridge.edge.set",
    "bridge.order.set",
    "advise",
    "human_step",
    "escalate",
];

// ---------------------------------------------------------------- the action

/// One action of §7.2's closed vocabulary, or `advise`, `human_step` or `escalate` (§6.2), in the
/// applier's `{"action": name, …args}` form: a table or an object, never an array, each key once
/// ([`Tagged`] spells the names and fields). Unknown fields, unknown names (`firmware.flash`) and
/// values outside a field's set are refused as the table is read.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// `service.restart {service_id}`: a supervised child (agent, model, transport, meshtastic)
    /// or `"@subject"`.
    ServiceRestart { service_id: Ref<Supervised> },
    /// `identify {device_id}`: `device` proposals only.
    Identify { device_id: DeviceId },
    /// `rns.interface.upsert {device_id, type, params}` †.
    InterfaceUpsert {
        device_id: DeviceId,
        kind: Text<MAX_IF_TYPE>,
        params: Params,
    },
    /// `rns.interface.remove {name}` †.
    InterfaceRemove { name: Text<MAX_NAME_BYTES> },
    /// `rns.posture.set {share_instance?, enable_transport?}`, one at least.
    PostureSet(Posture),
    /// `radio.region.set {region}`.
    RegionSet { region: Text<MAX_REGION> },
    /// `settings.set {key, value}`.
    SettingsSet(Setting),
    /// `component.enable {id}`, addressed by component id (a literal).
    ComponentEnable { id: ComponentId },
    /// `component.disable {id}`.
    ComponentDisable { id: ComponentId },
    /// `bridge.edge.pause {edge_id}` †.
    EdgePause { edge_id: EdgeId },
    /// `bridge.edge.set {edge_id, oversize?, honour_ok_to_mqtt?}` †, one setting at least.
    EdgeSet(EdgeSettings),
    /// `bridge.order.set {order}` †.
    OrderSet { order: Order },
    /// `advise {text}`: the card says it; nothing runs.
    Advise { text: Text<MAX_ADVICE> },
    /// `human_step {steps}`: the human does it; nothing runs.
    HumanStep { steps: Steps },
    /// `escalate {task}`: a `diagnose` or `patch` task; nothing applied.
    Escalate { task: EscalateTask },
}

impl Serialize for Action {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Tagged::serialize(self, s)
    }
}

impl<'de> Deserialize<'de> for Action {
    /// A table (TOML) or an object (JSON) and nothing else: serde's derive alone also reads a
    /// sequence, as the tag and then the fields in order (`["service.restart", "model"]`), which
    /// is not the applier's form. The table's keys then go to [`Tagged`] as they come, so a key
    /// twice is still refused.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct OneTable;
        impl<'de> de::Visitor<'de> for OneTable {
            type Value = Action;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an action table, {\"action\": name, …args}")
            }
            fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<Action, A::Error> {
                Tagged::deserialize(de::value::MapAccessDeserializer::new(map))
            }
        }
        d.deserialize_map(OneTable)
    }
}

/// [`Action`]'s wire form, the one place its names and fields are spelled: serde's `remote`
/// derive reads into and writes from an `Action` (a variant or a field of `Action` missing here
/// does not compile). Never constructed, and read only through [`Action`]'s `Deserialize`.
#[derive(Serialize, Deserialize)]
#[serde(remote = "Action", tag = "action", deny_unknown_fields)]
#[allow(dead_code)] // the remote derive builds `Action`s, never a `Tagged`
enum Tagged {
    #[serde(rename = "service.restart")]
    ServiceRestart { service_id: Ref<Supervised> },
    #[serde(rename = "identify")]
    Identify { device_id: DeviceId },
    #[serde(rename = "rns.interface.upsert")]
    InterfaceUpsert {
        device_id: DeviceId,
        #[serde(rename = "type")]
        kind: Text<MAX_IF_TYPE>,
        params: Params,
    },
    #[serde(rename = "rns.interface.remove")]
    InterfaceRemove { name: Text<MAX_NAME_BYTES> },
    #[serde(rename = "rns.posture.set")]
    PostureSet(Posture),
    #[serde(rename = "radio.region.set")]
    RegionSet { region: Text<MAX_REGION> },
    #[serde(rename = "settings.set")]
    SettingsSet(Setting),
    #[serde(rename = "component.enable")]
    ComponentEnable { id: ComponentId },
    #[serde(rename = "component.disable")]
    ComponentDisable { id: ComponentId },
    #[serde(rename = "bridge.edge.pause")]
    EdgePause { edge_id: EdgeId },
    #[serde(rename = "bridge.edge.set")]
    EdgeSet(EdgeSettings),
    #[serde(rename = "bridge.order.set")]
    OrderSet { order: Order },
    #[serde(rename = "advise")]
    Advise { text: Text<MAX_ADVICE> },
    #[serde(rename = "human_step")]
    HumanStep { steps: Steps },
    #[serde(rename = "escalate")]
    Escalate { task: EscalateTask },
}

/// Where an action may stand (§7.2's table and its † note).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A typed action a playbook cause, a `repair` proposal or a peer's fix may carry.
    Typed,
    /// † Human or `device.propose` only in v1: no playbook cause, `repair` proposal or peer fix
    /// carries it (§24 D19).
    HumanOnly,
    /// `identify`: `device` proposals only.
    DeviceOnly,
    /// `advise`, `human_step`: nothing runs.
    Advice,
    /// `escalate`: a task, nothing applied.
    Escalate,
}

impl Action {
    /// The vocabulary name (the `action` tag).
    pub fn name(&self) -> &'static str {
        match self {
            Action::ServiceRestart { .. } => "service.restart",
            Action::Identify { .. } => "identify",
            Action::InterfaceUpsert { .. } => "rns.interface.upsert",
            Action::InterfaceRemove { .. } => "rns.interface.remove",
            Action::PostureSet(_) => "rns.posture.set",
            Action::RegionSet { .. } => "radio.region.set",
            Action::SettingsSet(_) => "settings.set",
            Action::ComponentEnable { .. } => "component.enable",
            Action::ComponentDisable { .. } => "component.disable",
            Action::EdgePause { .. } => "bridge.edge.pause",
            Action::EdgeSet(_) => "bridge.edge.set",
            Action::OrderSet { .. } => "bridge.order.set",
            Action::Advise { .. } => "advise",
            Action::HumanStep { .. } => "human_step",
            Action::Escalate { .. } => "escalate",
        }
    }

    /// Where the action may stand (§7.2).
    pub fn kind(&self) -> Kind {
        match self {
            Action::ServiceRestart { .. }
            | Action::PostureSet(_)
            | Action::RegionSet { .. }
            | Action::SettingsSet(_)
            | Action::ComponentEnable { .. }
            | Action::ComponentDisable { .. } => Kind::Typed,
            Action::InterfaceUpsert { .. }
            | Action::InterfaceRemove { .. }
            | Action::EdgePause { .. }
            | Action::EdgeSet(_)
            | Action::OrderSet { .. } => Kind::HumanOnly,
            Action::Identify { .. } => Kind::DeviceOnly,
            Action::Advise { .. } | Action::HumanStep { .. } => Kind::Advice,
            Action::Escalate { .. } => Kind::Escalate,
        }
    }

    /// May a playbook cause, a `repair` proposal or a peer's fix carry it (§7.2, §24 D19)? Not a
    /// † action and not `identify`.
    pub fn playbook_may_carry(&self) -> bool {
        !matches!(self.kind(), Kind::HumanOnly | Kind::DeviceOnly)
    }

    /// Must a cause naming it name in `when` a check with a leaf that is not `log_contains`
    /// (§6.2, §24 D21)? Every action but `advise` and `escalate`.
    pub fn needs_a_non_log_check(&self) -> bool {
        !matches!(self, Action::Advise { .. } | Action::Escalate { .. })
    }

    /// The action's subject field, when it has one (`service.restart`'s `service_id`).
    pub fn subject_ref(&self) -> Option<&Ref<Supervised>> {
        match self {
            Action::ServiceRestart { service_id } => Some(service_id),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- the one substitution

/// A subject field's value: `"@subject"`, bound to the incident's subject (§6.2), or a literal of
/// the field's own set `T`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ref<T> {
    /// `"@subject"`.
    Subject,
    /// A literal.
    Id(T),
}

/// The literal set of a subject field.
pub trait SubjectField: Sized {
    /// What the field names, for a refusal.
    const WHAT: &'static str;
    /// The literal, if `s` is one of the set.
    fn parse(s: &str) -> Option<Self>;
    /// The literal as written.
    fn as_str(&self) -> &str;
}

impl<T: SubjectField> Ref<T> {
    /// Is this `"@subject"`?
    pub fn is_subject(&self) -> bool {
        matches!(self, Ref::Subject)
    }

    /// The value with `"@subject"` replaced by `subject`.
    pub fn resolve<'a>(&'a self, subject: &'a str) -> &'a str {
        match self {
            Ref::Subject => subject,
            Ref::Id(t) => t.as_str(),
        }
    }

    /// Can `subject`, a playbook's `[match]` subject, bind `"@subject"` in this field (§6.2: a
    /// member of the compiled subject table the field takes)?
    pub fn binds(subject: &str) -> bool {
        T::parse(subject).is_some()
    }
}

impl<'de, T: SubjectField> Deserialize<'de> for Ref<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s == SUBJECT {
            return Ok(Ref::Subject);
        }
        if s.starts_with('@') {
            return Err(de::Error::custom(at_refusal(T::WHAT)));
        }
        T::parse(&s)
            .map(Ref::Id)
            .ok_or_else(|| de::Error::custom(format_args!("{SERVICE_MARK}not {}", T::WHAT)))
    }
}

impl<T: SubjectField> Serialize for Ref<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.resolve(SUBJECT))
    }
}

/// A supervised child's service id: a `child` row of the services catalogue (agent, model,
/// transport, meshtastic), the children whose state watch, exit facts and log `111d` holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Supervised(pub ServiceId);

impl SubjectField for Supervised {
    const WHAT: &'static str = "a supervised service (agent, model, transport, meshtastic)";
    fn parse(s: &str) -> Option<Self> {
        ServiceId::parse(s)
            .filter(|id| catalogue(*id).kind == ServiceKind::Child)
            .map(Supervised)
    }
    fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// A service row's id: any `ServiceId` (`N/services/mod.rs`), the rows `Registry::snapshot()`
/// publishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceRow(pub ServiceId);

impl SubjectField for ServiceRow {
    const WHAT: &'static str = "a service id";
    fn parse(s: &str) -> Option<Self> {
        ServiceId::parse(s).map(ServiceRow)
    }
    fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// The refusal of a value that starts with `@` where it is no substitution.
fn at_refusal(field: &str) -> String {
    format!("{SUBJECT_MARK}{field}: only \"@subject\", in a subject field, is a substitution")
}

// ---------------------------------------------------------------- checked values

/// Plain text of 1 to `MAX` bytes (§6.2: no templating): no control character, and no `@` first
/// (only a subject field takes `"@subject"`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Text<const MAX: usize>(String);

impl<const MAX: usize> Text<MAX> {
    /// The text, checked.
    pub fn new(s: &str) -> Result<Self, String> {
        check_text("a text", s, MAX)?;
        Ok(Text(s.to_string()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de, const MAX: usize> Deserialize<'de> for Text<MAX> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Text::new(&s).map_err(de::Error::custom)
    }
}

/// `what` is 1 to `max` bytes with no control character and no `@` first.
pub(crate) fn check_text(what: &str, s: &str, max: usize) -> Result<(), String> {
    if s.starts_with('@') {
        return Err(at_refusal(what));
    }
    if s.is_empty() || s.len() > max {
        return Err(format!("{what} of 1 to {max} bytes"));
    }
    if s.chars().any(char::is_control) {
        return Err(format!("{what} holds a control character"));
    }
    Ok(())
}

/// A device id (`N/devices/mod.rs` `valid_id`: 4 to 64 of `[A-Za-z0-9_-]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct DeviceId(String);

impl DeviceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s.starts_with('@') {
            return Err(de::Error::custom(at_refusal("a device id")));
        }
        if !valid_id(&s) {
            return Err(de::Error::custom("not a device id"));
        }
        Ok(DeviceId(s))
    }
}

/// A component id (the contract's `ComponentId`, `is_component_id`): a literal only, never
/// `"@subject"`, whose binding is a member of the compiled subject table and so never a
/// component (§6.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ComponentId(String);

impl ComponentId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ComponentId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s.starts_with('@') {
            return Err(de::Error::custom(at_refusal("a component id")));
        }
        if !is_component_id(&s) {
            return Err(de::Error::custom("not a component id"));
        }
        Ok(ComponentId(s))
    }
}

/// A bridge edge id (the contract's `BridgeEdgeId`, `^be-[A-Za-z0-9_-]{1,125}$`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct EdgeId(String);

impl EdgeId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for EdgeId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s.starts_with('@') {
            return Err(de::Error::custom(at_refusal("an edge id")));
        }
        let ok = s.strip_prefix("be-").is_some_and(|rest| {
            (1..=125).contains(&rest.len())
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        });
        if !ok {
            return Err(de::Error::custom("not a bridge edge id"));
        }
        Ok(EdgeId(s))
    }
}

/// The preferred network order (`interfaces.toml` `[order]`): 1 to 16 distinct network ids (the
/// contract's `NetworkId`, `^[a-z0-9][a-z0-9:_-]{0,39}$`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Order(Vec<String>);

impl Order {
    pub fn ids(&self) -> &[String] {
        &self.0
    }
}

fn is_network_id(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=40).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b':' | b'_' | b'-')
        })
}

impl<'de> Deserialize<'de> for Order {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let ids = Vec::<String>::deserialize(d)?;
        if ids.iter().any(|s| s.starts_with('@')) {
            return Err(de::Error::custom(at_refusal("a network id")));
        }
        if ids.is_empty() || ids.len() > MAX_ORDER {
            return Err(de::Error::custom(format_args!(
                "an order of 1 to {MAX_ORDER} network ids"
            )));
        }
        if !ids.iter().all(|s| is_network_id(s)) {
            return Err(de::Error::custom("not a network id"));
        }
        if (1..ids.len()).any(|i| ids[..i].contains(&ids[i])) {
            return Err(de::Error::custom("a network id twice in one order"));
        }
        Ok(Order(ids))
    }
}

/// A `human_step`'s steps: 1 to [`MAX_STEPS`] texts of 1 to [`MAX_STEP`] bytes each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Steps(Vec<String>);

impl Steps {
    pub fn steps(&self) -> &[String] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Steps {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let steps = Vec::<String>::deserialize(d)?;
        if steps.is_empty() || steps.len() > MAX_STEPS {
            return Err(de::Error::custom(format_args!("1 to {MAX_STEPS} steps")));
        }
        for s in &steps {
            check_text("a step", s, MAX_STEP).map_err(de::Error::custom)?;
        }
        Ok(Steps(steps))
    }
}

/// `rns.interface.upsert`'s `params` (†), the applier's flat form (`N/devices/applier.rs`
/// `toml_params`): a table of at most [`MAX_PARAMS`] keys of 1 to [`MAX_PARAM_KEY`] B of
/// `[a-z0-9_]`, each key once, each value a boolean, a number or a text of 1 to
/// [`MAX_PARAM_TEXT`] B with no control character and no `@` first ([`check_text`]); never a
/// table, an array or null. The stanza type's allowlist and each key's range stay the spine's,
/// checked when the applier renders the entry (`N/transport/interfaces.rs` `param_specs`,
/// `check_value`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Params(Map<String, Value>);

impl Params {
    pub fn as_map(&self) -> &Map<String, Value> {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Params {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Flat;
        impl<'de> de::Visitor<'de> for Flat {
            type Value = Params;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a flat table of parameters")
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Params, A::Error> {
                let mut out = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    let shaped = (1..=MAX_PARAM_KEY).contains(&key.len())
                        && key
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
                    if !shaped {
                        return Err(de::Error::custom(format_args!(
                            "params: a key is 1 to {MAX_PARAM_KEY} B of a-z, 0-9 and _"
                        )));
                    }
                    let value: Value = map.next_value()?;
                    match &value {
                        Value::Bool(_) | Value::Number(_) => {}
                        Value::String(s) => check_text("a parameter", s, MAX_PARAM_TEXT)
                            .map_err(de::Error::custom)?,
                        _ => {
                            return Err(de::Error::custom(
                                "params: a value is a boolean, a number or a text",
                            ))
                        }
                    }
                    if out.insert(key, value).is_some() {
                        return Err(de::Error::custom("params: a key twice"));
                    }
                    if out.len() > MAX_PARAMS {
                        return Err(de::Error::custom(format_args!(
                            "params: at most {MAX_PARAMS} keys"
                        )));
                    }
                }
                Ok(Params(out))
            }
        }
        d.deserialize_map(Flat)
    }
}

/// `escalate`'s task (§6.2), read from its name: a string, never the one-key table
/// (`{ patch = {} }`) serde's derive would also read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EscalateTask {
    Diagnose,
    Patch,
}

impl<'de> Deserialize<'de> for EscalateTask {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match String::deserialize(d)?.as_str() {
            "diagnose" => Ok(EscalateTask::Diagnose),
            "patch" => Ok(EscalateTask::Patch),
            _ => Err(de::Error::custom("escalate's task is diagnose or patch")),
        }
    }
}

/// `rns.posture.set`'s two switches, one at least (the applier's rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawPosture")]
pub struct Posture {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_instance: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_transport: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPosture {
    share_instance: Option<bool>,
    enable_transport: Option<bool>,
}

impl TryFrom<RawPosture> for Posture {
    type Error = &'static str;
    fn try_from(r: RawPosture) -> Result<Self, Self::Error> {
        if r.share_instance.is_none() && r.enable_transport.is_none() {
            return Err("rns.posture.set needs share_instance or enable_transport");
        }
        Ok(Posture {
            share_instance: r.share_instance,
            enable_transport: r.enable_transport,
        })
    }
}

/// `bridge.edge.set`'s edge and its settings, one at least (`PATCH /bridge/edges/{id}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawEdgeSettings")]
pub struct EdgeSettings {
    pub edge_id: EdgeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oversize: Option<Oversize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub honour_ok_to_mqtt: Option<bool>,
}

/// An edge's `oversize` (the contract's `BridgeEdgePatch.oversize`), read from its name only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Oversize {
    Refuse,
    Split,
}

impl<'de> Deserialize<'de> for Oversize {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match String::deserialize(d)?.as_str() {
            "refuse" => Ok(Oversize::Refuse),
            "split" => Ok(Oversize::Split),
            _ => Err(de::Error::custom("oversize is refuse or split")),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEdgeSettings {
    edge_id: EdgeId,
    oversize: Option<Oversize>,
    honour_ok_to_mqtt: Option<bool>,
}

impl TryFrom<RawEdgeSettings> for EdgeSettings {
    type Error = &'static str;
    fn try_from(r: RawEdgeSettings) -> Result<Self, Self::Error> {
        if r.oversize.is_none() && r.honour_ok_to_mqtt.is_none() {
            return Err("bridge.edge.set needs oversize or honour_ok_to_mqtt");
        }
        Ok(EdgeSettings {
            edge_id: r.edge_id,
            oversize: r.oversize,
            honour_ok_to_mqtt: r.honour_ok_to_mqtt,
        })
    }
}

/// `settings.set {key, value}` (§7.2): the only keys are `fabric.listen_lan` and
/// `fabric.hub_role` (each has a probe; a boolean) and `fabric.quota.<class>` (bytes, `class` a
/// quota class of `PATCH /fabric/settings`); `lan_enabled` takes effect only at a daemon restart
/// and is refused (Q6), and no consent-gated setting is a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setting {
    ListenLan(bool),
    HubRole(bool),
    /// A class quota in bytes, through Layer 8's `quota.set`.
    Quota {
        class: String,
        bytes: u64,
    },
}

impl Setting {
    /// The dotted key.
    pub fn key(&self) -> String {
        match self {
            Setting::ListenLan(_) => "fabric.listen_lan".into(),
            Setting::HubRole(_) => "fabric.hub_role".into(),
            Setting::Quota { class, .. } => format!("{QUOTA_PREFIX}{class}"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSetting {
    key: String,
    value: SettingValue,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SettingValue {
    Flag(bool),
    Bytes(u64),
}

impl<'de> Deserialize<'de> for Setting {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = RawSetting::deserialize(d)?;
        if r.key.starts_with('@') {
            return Err(de::Error::custom(at_refusal("a settings key")));
        }
        match (r.key.as_str(), r.value) {
            ("fabric.listen_lan", SettingValue::Flag(b)) => Ok(Setting::ListenLan(b)),
            ("fabric.hub_role", SettingValue::Flag(b)) => Ok(Setting::HubRole(b)),
            (key, SettingValue::Bytes(bytes)) if key.starts_with(QUOTA_PREFIX) => {
                let class = &key[QUOTA_PREFIX.len()..];
                if quota_class(class).is_none() {
                    return Err(de::Error::custom("settings.set: not a quota class"));
                }
                Ok(Setting::Quota {
                    class: class.to_string(),
                    bytes,
                })
            }
            ("fabric.listen_lan" | "fabric.hub_role", _) => Err(de::Error::custom(
                "settings.set: fabric.listen_lan and fabric.hub_role take a boolean",
            )),
            (key, _) if key.starts_with(QUOTA_PREFIX) => Err(de::Error::custom(
                "settings.set: a quota takes a number of bytes",
            )),
            _ => Err(de::Error::custom(
                "settings.set: the keys are fabric.listen_lan, fabric.hub_role and \
                 fabric.quota.<class>",
            )),
        }
    }
}

impl Serialize for Setting {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("Setting", 2)?;
        st.serialize_field("key", &self.key())?;
        match self {
            Setting::ListenLan(b) | Setting::HubRole(b) => st.serialize_field("value", b)?,
            Setting::Quota { bytes, .. } => st.serialize_field("value", bytes)?,
        }
        st.end()
    }
}

impl fmt::Display for Action {
    /// The name and its subject field, for a log line or a refusal (never a free text).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::ServiceRestart { service_id } => {
                write!(f, "{} {}", self.name(), service_id.resolve(SUBJECT))
            }
            Action::ComponentEnable { id } | Action::ComponentDisable { id } => {
                write!(f, "{} {}", self.name(), id.as_str())
            }
            _ => f.write_str(self.name()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: Value) -> Result<Action, String> {
        serde_json::from_value::<Action>(v).map_err(|e| e.to_string())
    }

    fn toml_action(text: &str) -> Result<Action, String> {
        #[derive(Deserialize)]
        struct W {
            action: Action,
        }
        toml::from_str::<W>(text)
            .map(|w| w.action)
            .map_err(|e| e.message().to_string())
    }

    #[test]
    fn every_action_parses_in_the_applier_form_and_round_trips() {
        let cases = [
            json!({"action": "service.restart", "service_id": "model"}),
            json!({"action": "service.restart", "service_id": "@subject"}),
            json!({"action": "identify", "device_id": "dev-0001"}),
            json!({"action": "rns.interface.upsert", "device_id": "dev-0001",
                   "type": "RNodeInterface", "params": {"frequency": 869525000}}),
            json!({"action": "rns.interface.remove", "name": "RNode dev-0001"}),
            json!({"action": "rns.posture.set", "share_instance": true}),
            json!({"action": "rns.posture.set", "share_instance": false, "enable_transport": true}),
            json!({"action": "radio.region.set", "region": "EU_868"}),
            json!({"action": "settings.set", "key": "fabric.listen_lan", "value": true}),
            json!({"action": "settings.set", "key": "fabric.hub_role", "value": false}),
            json!({"action": "settings.set", "key": "fabric.quota.custody", "value": 1048576}),
            json!({"action": "component.enable", "id": "deltachat"}),
            json!({"action": "component.disable", "id": "lxmf"}),
            json!({"action": "bridge.edge.pause", "edge_id": "be-1"}),
            json!({"action": "bridge.edge.set", "edge_id": "be-1", "oversize": "split"}),
            json!({"action": "bridge.order.set", "order": ["lxmf", "app:deltachat"]}),
            json!({"action": "advise", "text": "More free memory is needed."}),
            json!({"action": "human_step", "steps": ["On Linux: re-run `sudo 111 install`."]}),
            json!({"action": "escalate", "task": "diagnose"}),
            json!({"action": "escalate", "task": "patch"}),
        ];
        let mut named = std::collections::BTreeSet::new();
        for v in cases {
            let a = parse(v.clone()).unwrap_or_else(|e| panic!("{v}: {e}"));
            assert_eq!(a.name(), v["action"], "{v}");
            assert!(NAMES.contains(&a.name()), "{v}");
            named.insert(a.name());
            assert_eq!(
                serde_json::to_value(&a).unwrap(),
                v,
                "the round trip of {v}"
            );
            // every variant denies an unknown field, a null one too
            for extra in [json!(1), Value::Null] {
                let mut w = v.clone();
                w["zz"] = extra;
                let e = parse(w.clone()).unwrap_err();
                assert!(e.contains("unknown field"), "{w}: {e}");
            }
        }
        // NAMES is the enum's names, every one of them (the grammar's pre-pass reads it)
        assert_eq!(named.len(), NAMES.len());
        for name in NAMES {
            let e = parse(json!({ "action": name })).unwrap_err();
            assert!(!e.contains("unknown variant"), "{name}: {e}");
        }
    }

    #[test]
    fn the_vocabulary_is_closed() {
        for name in [
            "firmware.flash",
            "meshtastic.set",
            "ble.pair",
            "ble.forget",
            "update.rollback",
            "shell",
            "Service.Restart",
        ] {
            let e = parse(json!({"action": name})).unwrap_err();
            assert!(e.contains("unknown variant"), "{name}: {e}");
            assert!(!NAMES.contains(&name), "{name}");
        }
        // no tag, a tag that is not a string, a tag twice
        assert!(parse(json!({"service_id": "model"})).is_err());
        assert!(parse(json!({"action": 3})).is_err());
        let twice = r#"{"action": "advise", "action": "advise", "text": "x"}"#;
        assert!(serde_json::from_str::<Action>(twice).is_err());
    }

    #[test]
    fn every_table_denies_unknown_fields() {
        for v in [
            json!({"action": "service.restart", "service_id": "model", "force": true}),
            json!({"action": "rns.posture.set", "share_instance": true, "restart": true}),
            json!({"action": "settings.set", "key": "fabric.hub_role", "value": true, "x": 1}),
            json!({"action": "bridge.edge.set", "edge_id": "be-1", "paused": true}),
            json!({"action": "advise", "text": "x", "url": "https://example.com"}),
            json!({"action": "human_step", "steps": ["x"], "command": "rm"}),
            json!({"action": "escalate", "task": "diagnose", "model": "big"}),
            // a null unknown field too (the applier lets one through; a fix body never does)
            json!({"action": "component.enable", "id": "lxmf", "ack": null}),
        ] {
            let e = parse(v.clone()).unwrap_err();
            assert!(e.contains("unknown field"), "{v}: {e}");
        }
        let e =
            toml_action("action = { action = \"service.restart\", service_id = \"model\", n = 1 }")
                .unwrap_err();
        assert!(e.starts_with("unknown field"), "{e}");
    }

    #[test]
    fn subject_is_the_one_substitution() {
        let a = parse(json!({"action": "service.restart", "service_id": "@subject"})).unwrap();
        let r = a.subject_ref().unwrap();
        assert!(r.is_subject());
        assert_eq!(r.resolve("agent"), "agent");
        assert_eq!(a.to_string(), "service.restart @subject");
        // any other `@` value, in a subject field or in any other field, is refused
        for v in [
            json!({"action": "service.restart", "service_id": "@device"}),
            json!({"action": "service.restart", "service_id": "@SUBJECT"}),
            json!({"action": "service.restart", "service_id": "@subject "}),
            json!({"action": "component.enable", "id": "@subject"}),
            json!({"action": "radio.region.set", "region": "@subject"}),
            json!({"action": "advise", "text": "@subject"}),
            json!({"action": "human_step", "steps": ["ok", "@channel"]}),
            json!({"action": "settings.set", "key": "@subject", "value": true}),
            json!({"action": "identify", "device_id": "@subject"}),
            json!({"action": "bridge.edge.pause", "edge_id": "@subject"}),
            json!({"action": "bridge.order.set", "order": ["@subject"]}),
        ] {
            let e = parse(v.clone()).unwrap_err();
            assert!(e.starts_with(SUBJECT_MARK), "{v}: {e}");
        }
        // a text holding `@subject` further in is plain text: no templating
        let a = parse(json!({"action": "advise", "text": "restart @subject by hand"})).unwrap();
        match a {
            Action::Advise { text } => assert_eq!(text.as_str(), "restart @subject by hand"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn service_ids_are_the_supervised_children() {
        for id in ["agent", "model", "transport", "meshtastic"] {
            let a = parse(json!({"action": "service.restart", "service_id": id})).unwrap();
            assert_eq!(a.subject_ref().unwrap().resolve("x"), id);
            assert!(Ref::<Supervised>::binds(id));
        }
        for id in [
            "kb",
            "radio",
            "fabric",
            "gateway",
            "codeagent",
            "111d",
            "llama-server",
            "",
        ] {
            let e = parse(json!({"action": "service.restart", "service_id": id})).unwrap_err();
            assert!(e.starts_with(SERVICE_MARK), "{id}: {e}");
            assert!(!Ref::<Supervised>::binds(id), "{id}");
        }
        assert!(Ref::<ServiceRow>::binds("kb"));
        assert!(!Ref::<ServiceRow>::binds("identity"));
        assert!(!Ref::<ServiceRow>::binds("any"));
    }

    #[test]
    fn kinds_follow_the_dagger_note() {
        let kind = |v: Value| parse(v).unwrap().kind();
        for v in [
            json!({"action": "rns.interface.upsert", "device_id": "dev-0001",
                   "type": "RNodeInterface", "params": {}}),
            json!({"action": "rns.interface.remove", "name": "RNode"}),
            json!({"action": "bridge.edge.pause", "edge_id": "be-1"}),
            json!({"action": "bridge.edge.set", "edge_id": "be-1", "honour_ok_to_mqtt": true}),
            json!({"action": "bridge.order.set", "order": ["lxmf"]}),
        ] {
            assert_eq!(kind(v.clone()), Kind::HumanOnly, "{v}");
            assert!(!parse(v).unwrap().playbook_may_carry());
        }
        let identify = parse(json!({"action": "identify", "device_id": "dev-0001"})).unwrap();
        assert_eq!(identify.kind(), Kind::DeviceOnly);
        assert!(!identify.playbook_may_carry());
        for v in [
            json!({"action": "service.restart", "service_id": "model"}),
            json!({"action": "rns.posture.set", "enable_transport": true}),
            json!({"action": "radio.region.set", "region": "US"}),
            json!({"action": "settings.set", "key": "fabric.quota.transit", "value": 0}),
            json!({"action": "component.enable", "id": "simplex"}),
            json!({"action": "component.disable", "id": "simplex"}),
        ] {
            assert_eq!(kind(v.clone()), Kind::Typed, "{v}");
            let a = parse(v).unwrap();
            assert!(a.playbook_may_carry() && a.needs_a_non_log_check());
        }
        let advise = parse(json!({"action": "advise", "text": "x"})).unwrap();
        let step = parse(json!({"action": "human_step", "steps": ["x"]})).unwrap();
        let escalate = parse(json!({"action": "escalate", "task": "patch"})).unwrap();
        assert_eq!(
            (advise.kind(), step.kind(), escalate.kind()),
            (Kind::Advice, Kind::Advice, Kind::Escalate)
        );
        // §6.2: only advise and escalate may stand on log text alone
        assert!(!advise.needs_a_non_log_check() && !escalate.needs_a_non_log_check());
        assert!(step.needs_a_non_log_check());
    }

    #[test]
    fn values_outside_their_sets_are_refused() {
        for v in [
            // settings: the three keys only, each with its type
            json!({"action": "settings.set", "key": "lan_enabled", "value": true}),
            json!({"action": "settings.set", "key": "fabric.custody_opt_in", "value": true}),
            json!({"action": "settings.set", "key": "fabric.quota.chat", "value": 1}),
            json!({"action": "settings.set", "key": "fabric.quota.", "value": 1}),
            json!({"action": "settings.set", "key": "fabric.listen_lan", "value": 1}),
            json!({"action": "settings.set", "key": "fabric.quota.custody", "value": true}),
            json!({"action": "settings.set", "key": "fabric.quota.custody", "value": -1}),
            json!({"action": "settings.set", "key": "fabric.quota.custody", "value": null}),
            json!({"action": "settings.set", "key": "fabric.hub_role"}),
            // posture and edge settings need one setting
            json!({"action": "rns.posture.set"}),
            json!({"action": "rns.posture.set", "share_instance": null}),
            json!({"action": "rns.posture.set", "share_instance": "yes"}),
            json!({"action": "bridge.edge.set", "edge_id": "be-1"}),
            json!({"action": "bridge.edge.set", "edge_id": "be-1", "oversize": "drop"}),
            // ids by their own shapes
            json!({"action": "identify", "device_id": "abc"}),
            json!({"action": "component.enable", "id": "Delta Chat"}),
            json!({"action": "component.enable", "id": ""}),
            json!({"action": "bridge.edge.pause", "edge_id": "edge-1"}),
            json!({"action": "bridge.edge.pause", "edge_id": "be-"}),
            json!({"action": "bridge.order.set", "order": []}),
            json!({"action": "bridge.order.set", "order": ["lxmf", "lxmf"]}),
            json!({"action": "bridge.order.set", "order": ["LXMF"]}),
            // texts: bounded, no control character
            json!({"action": "radio.region.set", "region": ""}),
            json!({"action": "radio.region.set", "region": "x".repeat(MAX_REGION + 1)}),
            json!({"action": "advise", "text": ""}),
            json!({"action": "advise", "text": "x".repeat(MAX_ADVICE + 1)}),
            json!({"action": "advise", "text": "one\ntwo"}),
            json!({"action": "human_step", "steps": []}),
            json!({"action": "human_step", "steps": vec!["x"; MAX_STEPS + 1]}),
            json!({"action": "human_step", "steps": ["x".repeat(MAX_STEP + 1)]}),
            json!({"action": "human_step", "steps": ["a\u{7}b"]}),
            json!({"action": "escalate", "task": "fix"}),
            json!({"action": "service.restart", "service_id": 3}),
            json!({"action": "service.restart"}),
        ] {
            assert!(parse(v.clone()).is_err(), "{v}");
        }
        // the bounds themselves are taken
        for v in [
            json!({"action": "advise", "text": "x".repeat(MAX_ADVICE)}),
            json!({"action": "human_step", "steps": vec!["x".repeat(MAX_STEP); MAX_STEPS]}),
            json!({"action": "radio.region.set", "region": "x".repeat(MAX_REGION)}),
            json!({"action": "settings.set", "key": "fabric.quota.kb", "value": u64::MAX}),
        ] {
            assert!(parse(v.clone()).is_ok(), "{v}");
        }
    }

    #[test]
    fn an_action_is_a_table_and_a_name_is_a_string() {
        // serde's derive alone reads a sequence as the tag and then the fields in order, and a
        // one-key table as an enum's name: neither is the applier's form (review finding 1)
        for v in [
            json!(["service.restart", "model"]),
            json!(["advise", "x"]),
            json!(["escalate", "patch"]),
            json!(["rns.posture.set", true]),
            json!({"action": "escalate", "task": {"patch": {}}}),
            json!({"action": "escalate", "task": {"patch": null}}),
            json!({"action": "bridge.edge.set", "edge_id": "be-1", "oversize": {"split": {}}}),
            json!("advise"),
            Value::Null,
        ] {
            assert!(parse(v.clone()).is_err(), "{v}");
        }
        assert!(serde_json::from_str::<Action>(r#"["service.restart", "model"]"#).is_err());
        assert!(toml_action("action = [\"service.restart\", \"model\"]").is_err());
        assert!(toml_action("action = { action = \"escalate\", task = { patch = {} } }").is_err());
        // a key twice is refused, the tag or a field
        for text in [
            r#"{"action": "advise", "action": "advise", "text": "x"}"#,
            r#"{"action": "advise", "text": "x", "text": "y"}"#,
            r#"{"action": "service.restart", "service_id": "model", "service_id": "agent"}"#,
        ] {
            assert!(serde_json::from_str::<Action>(text).is_err(), "{text}");
        }
        // the same actions written as tables parse
        for v in [
            json!({"action": "escalate", "task": "patch"}),
            json!({"action": "bridge.edge.set", "edge_id": "be-1", "oversize": "split"}),
        ] {
            assert!(parse(v.clone()).is_ok(), "{v}");
        }
        let a = serde_json::from_str::<Action>(r#"{"action": "advise", "text": "x"}"#).unwrap();
        assert_eq!(a.name(), "advise");
    }

    #[test]
    fn params_are_flat_bounded_and_checked() {
        // `rns.interface.upsert`'s params: the applier's flat form (review finding 4)
        let upsert = |params: Value| {
            json!({"action": "rns.interface.upsert", "device_id": "dev-0001",
                   "type": "RNodeInterface", "params": params})
        };
        let keys = |n: usize| -> Value {
            Value::Object((0..n).map(|i| (format!("k{i}"), json!(i))).collect())
        };
        // taken: a boolean, an integer, a number, a text; the bounds themselves; none at all
        for p in [
            json!({"frequency": 869525000, "flow_control": true, "airtime_limit_long": 1.5,
                   "id_callsign": "N0CALL"}),
            keys(MAX_PARAMS),
            json!({"target_host": "h".repeat(MAX_PARAM_TEXT)}),
            Value::Object(
                [("k".repeat(MAX_PARAM_KEY), json!(1))]
                    .into_iter()
                    .collect(),
            ),
            json!({}),
        ] {
            let a = parse(upsert(p.clone())).unwrap_or_else(|e| panic!("{p}: {e}"));
            assert_eq!(
                serde_json::to_value(&a).unwrap(),
                upsert(p),
                "the round trip"
            );
        }
        // refused: nested, an array, null, empty or long text, a control character, too many
        // keys, a key out of shape, params that are no table
        for p in [
            json!({"a": {"b": 1}}),
            json!({"a": [1]}),
            json!({"a": null}),
            json!({"a": ""}),
            json!({"a": "x".repeat(MAX_PARAM_TEXT + 1)}),
            json!({"a": "x\ny"}),
            keys(MAX_PARAMS + 1),
            json!({"Bad Key": 1}),
            json!({"": 1}),
            Value::Object(
                [("k".repeat(MAX_PARAM_KEY + 1), json!(1))]
                    .into_iter()
                    .collect(),
            ),
            json!([1, 2]),
            json!("frequency=1"),
        ] {
            assert!(parse(upsert(p.clone())).is_err(), "{p}");
        }
        // `@subject` is no substitution here either
        let e = parse(upsert(json!({"port": "@subject"}))).unwrap_err();
        assert!(e.starts_with(SUBJECT_MARK), "{e}");
        // a key twice
        let twice = r#"{"action": "rns.interface.upsert", "device_id": "dev-0001",
                        "type": "RNodeInterface", "params": {"txpower": 1, "txpower": 37}}"#;
        assert!(serde_json::from_str::<Action>(twice).is_err());
        // TOML reads the same
        assert!(toml_action(
            "action = { action = \"rns.interface.upsert\", device_id = \"dev-0001\", \
             type = \"RNodeInterface\", params = { a = { b = 1 } } }"
        )
        .is_err());
    }

    #[test]
    fn a_toml_cause_action_reads_the_same_as_json() {
        let t = toml_action(
            "action = { action = \"settings.set\", key = \"fabric.quota.custody\", value = 4096 }",
        )
        .unwrap();
        let j =
            parse(json!({"action": "settings.set", "key": "fabric.quota.custody", "value": 4096}))
                .unwrap();
        assert_eq!(t, j);
        assert_eq!(
            t,
            Action::SettingsSet(Setting::Quota {
                class: "custody".into(),
                bytes: 4096
            })
        );
        // a TOML date is no text, a float is no count
        assert!(
            toml_action("action = { action = \"radio.region.set\", region = 2026-11-16 }").is_err()
        );
        assert!(toml_action(
            "action = { action = \"settings.set\", key = \"fabric.quota.kb\", value = 1.5 }"
        )
        .is_err());
    }
}
