//! Licence acknowledgements and the enable switch of optional components in
//! `state/bridge.json` (Layer 6 §2 "State", §2.1 rule 4, §15 "every
//! third-party program"; D26 #5: the user installs, 111 is not the
//! distributor).
//!
//! **File.** `state/bridge.json` (0600) is shared with the bridge core
//! (recipient lists, send switches, HAM confirmations, announce settings land
//! there too): this module owns only the `components` object and keeps every
//! other top-level key as it found it. `components.<id>` =
//! `{acknowledged: {notice_hash, notice_text_version}, enabled}`.
//!
//! **Failures (fail closed).** An absent file is an empty one. A file that is
//! not a small regular file, not a JSON object, has another `schema`, or whose
//! `components` object is malformed is **moved aside**
//! (`bridge.json.invalid-<n>`, never deleted, a link moved itself, never
//! followed, the `fabric/settings` pattern) and treated as empty: no
//! acknowledgement survives, so every component asks again.
//!
//! **Consent.** An acknowledgement is valid only for the exact notice text the
//! component's declaration carries now: its lowercase hex SHA-256
//! (`Component::notice_hash`) and [`NOTICE_TEXT_VERSION`]. A changed notice
//! (an update that changes the text) has a new hash, so the user is asked
//! again. Without a valid recorded or supplied acknowledgement nothing is
//! installed or started ([`ConsentRequired`] → `409 consent_required`).

use super::manifest::Component;
use crate::daemon::dashboard::api_v1::Problem;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The file, under the daemon root.
pub const BRIDGE_FILE: &str = "state/bridge.json";
/// Largest file read back.
pub const MAX_FILE: u64 = 64 * 1024;
/// How the acknowledged text is composed: the declaration's `[license]
/// notice`, verbatim. Bumped if that composition ever changes, which
/// invalidates every recorded acknowledgement.
pub const NOTICE_TEXT_VERSION: u32 = 1;
/// File schema.
const SCHEMA: u64 = 1;
/// Attempts at a free `bridge.json.invalid-<n>` name.
const MOVE_ASIDE_TRIES: u32 = 1000;

/// Serialises every read-modify-write of the file in this process.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// One recorded acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Acknowledged {
    pub notice_hash: String,
    pub notice_text_version: u32,
}

/// `components.<id>`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged: Option<Acknowledged>,
    /// The user enabled it and has not disabled it since.
    #[serde(default)]
    pub enabled: bool,
}

/// The `components` part of `state/bridge.json` plus the keys this module
/// does not own (kept verbatim on write).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BridgeFile {
    pub components: BTreeMap<String, ComponentRecord>,
    others: serde_json::Map<String, serde_json::Value>,
}

impl BridgeFile {
    /// The record of `id` (default: never acknowledged, not enabled).
    pub fn record(&self, id: &str) -> ComponentRecord {
        self.components.get(id).cloned().unwrap_or_default()
    }

    /// The recorded acknowledgement matches `c`'s current notice.
    pub fn acknowledged(&self, c: &Component) -> bool {
        self.record(&c.id)
            .acknowledged
            .is_some_and(|a| ack_matches(&a, c))
    }
}

fn ack_matches(a: &Acknowledged, c: &Component) -> bool {
    a.notice_text_version == NOTICE_TEXT_VERSION && a.notice_hash == c.notice_hash()
}

/// `<root>/state/bridge.json`.
pub fn path(root: &Path) -> PathBuf {
    root.join(BRIDGE_FILE)
}

/// Move the invalid file aside (never deleted; a link is moved itself).
fn move_aside(p: &Path) -> Option<PathBuf> {
    let dir = p.parent()?;
    for n in 1..=MOVE_ASIDE_TRIES {
        let to = dir.join(format!("bridge.json.invalid-{n}"));
        if std::fs::symlink_metadata(&to).is_ok() {
            continue;
        }
        return match std::fs::rename(p, &to) {
            Ok(()) => Some(to),
            Err(e) => {
                eprintln!("111 bridge: cannot move {} aside: {e}", p.display());
                None
            }
        };
    }
    eprintln!(
        "111 bridge: cannot move {} aside: no free name",
        p.display()
    );
    None
}

fn decode(bytes: &[u8]) -> Result<BridgeFile, String> {
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let serde_json::Value::Object(mut map) = v else {
        return Err("not a JSON object".into());
    };
    match map.get("schema") {
        None => {}
        Some(s) if s.as_u64() == Some(SCHEMA) => {}
        Some(s) => return Err(format!("schema {s} unsupported")),
    }
    let components = match map.remove("components") {
        None => BTreeMap::new(),
        Some(c) => serde_json::from_value::<BTreeMap<String, ComponentRecord>>(c)
            .map_err(|e| format!("components: {e}"))?,
    };
    if components
        .keys()
        .any(|k| !super::manifest::is_component_id(k))
    {
        return Err("components: a key is not a component id".into());
    }
    map.remove("schema");
    Ok(BridgeFile {
        components,
        others: map,
    })
}

/// Read the file (see the module doc for the failure rules). Never fails:
/// what cannot be read is treated as empty.
pub fn load(root: &Path) -> BridgeFile {
    let p = path(root);
    let meta = match std::fs::symlink_metadata(&p) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return BridgeFile::default(),
        Err(e) => {
            eprintln!(
                "111 bridge: {} unreadable ({e}); no acknowledgement",
                p.display()
            );
            return BridgeFile::default();
        }
    };
    let invalid = |why: String| {
        match move_aside(&p) {
            Some(to) => eprintln!(
                "111 bridge: {} invalid ({why}); moved to {}; every component asks again",
                p.display(),
                to.display()
            ),
            None => eprintln!("111 bridge: {} invalid ({why})", p.display()),
        }
        BridgeFile::default()
    };
    if !meta.file_type().is_file() || meta.len() > MAX_FILE {
        return invalid("not a small regular file".into());
    }
    let bytes = match read_capped(&p) {
        Ok(b) => b,
        Err(e) => return invalid(e.to_string()),
    };
    match decode(&bytes) {
        Ok(f) => f,
        Err(why) => invalid(why),
    }
}

fn read_capped(p: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(p)?;
    let mut bytes = Vec::new();
    f.take(MAX_FILE + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(std::io::Error::other("larger than 64 KiB"));
    }
    Ok(bytes)
}

fn save(root: &Path, f: &BridgeFile) -> anyhow::Result<()> {
    let mut map = f.others.clone();
    map.insert("schema".into(), serde_json::Value::from(SCHEMA));
    map.insert("components".into(), serde_json::to_value(&f.components)?);
    let mut text = serde_json::to_string_pretty(&serde_json::Value::Object(map))?;
    text.push('\n');
    let p = path(root);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::util::fs::atomic_write(&p, text.as_bytes(), 0o600)
}

/// Read-modify-write `components.<id>` under the process-wide lock.
pub fn update(
    root: &Path,
    id: &str,
    f: impl FnOnce(&mut ComponentRecord),
) -> anyhow::Result<ComponentRecord> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut file = load(root);
    let rec = file.components.entry(id.to_string()).or_default();
    f(rec);
    let out = rec.clone();
    save(root, &file)?;
    Ok(out)
}

/// The consent round-trip body (contract `ConsentRequired`, the Problem's
/// `consent` member): the exact text and its hash to echo back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConsentRequired {
    pub action: &'static str,
    pub subject: String,
    pub warning_text: String,
    pub hash: String,
    pub addons: Vec<String>,
}

impl ConsentRequired {
    /// The notice `c` asks the user to acknowledge before enabling.
    pub fn for_component(c: &Component) -> Self {
        ConsentRequired {
            action: "component_enable",
            subject: c.id.clone(),
            warning_text: c.license.notice.clone(),
            hash: c.notice_hash(),
            addons: Vec::new(),
        }
    }

    /// `409 consent_required` as problem+json with the `consent` member.
    pub fn problem(&self) -> ProblemWith {
        ProblemWith::new(
            Problem::new(
                StatusCode::CONFLICT,
                "consent_required",
                "Read and acknowledge the licence notice first.",
            )
            .detail("Show the notice, then repeat the request with accept_notice_hash."),
        )
        .with("consent", serde_json::to_value(self).unwrap_or_default())
    }
}

/// Check (and record) consent for enabling `c` (§2.1 rule 4). `ack` is the
/// hash the client echoes (`accept_notice_hash`): the right one is recorded
/// and passes; a wrong one is refused even when an older acknowledgement is
/// on file; none passes only on a recorded acknowledgement of this exact
/// notice.
pub fn require(root: &Path, c: &Component, ack: Option<&str>) -> Result<(), ConsentCheck> {
    let want = c.notice_hash();
    match ack {
        Some(h) if h == want => {
            update(root, &c.id, |r| {
                r.acknowledged = Some(Acknowledged {
                    notice_hash: want.clone(),
                    notice_text_version: NOTICE_TEXT_VERSION,
                })
            })
            .map_err(|e| ConsentCheck::Io(format!("{e:#}")))?;
            Ok(())
        }
        Some(_) => Err(ConsentCheck::Required(ConsentRequired::for_component(c))),
        None if load(root).acknowledged(c) => Ok(()),
        None => Err(ConsentCheck::Required(ConsentRequired::for_component(c))),
    }
}

/// Why [`require`] did not pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentCheck {
    Required(ConsentRequired),
    /// The acknowledgement could not be recorded (nothing proceeds).
    Io(String),
}

/// A [`Problem`] plus RFC 9457 extension members (`consent`, …), as the
/// contract's `Problem` allows. Answers `application/problem+json`.
#[derive(Debug, Clone)]
pub struct ProblemWith {
    pub problem: Problem,
    pub members: serde_json::Map<String, serde_json::Value>,
}

impl ProblemWith {
    pub fn new(problem: Problem) -> Self {
        ProblemWith {
            problem,
            members: serde_json::Map::new(),
        }
    }

    pub fn with(mut self, key: &str, value: serde_json::Value) -> Self {
        self.members.insert(key.to_string(), value);
        self
    }

    /// The body as JSON.
    pub fn body(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(&self.problem).unwrap_or_default();
        if let serde_json::Value::Object(map) = &mut v {
            for (k, m) in &self.members {
                map.insert(k.clone(), m.clone());
            }
        }
        v
    }
}

impl axum::response::IntoResponse for ProblemWith {
    fn into_response(self) -> axum::response::Response {
        let status =
            StatusCode::from_u16(self.problem.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (
            status,
            [(axum::http::header::CONTENT_TYPE, "application/problem+json")],
            self.body().to_string(),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn component() -> Component {
        let files = super::super::manifest::embedded();
        super::super::manifest::load(&files)
            .into_iter()
            .find(|c| c.id == "lxmf")
            .unwrap()
    }

    #[test]
    fn acknowledgement_round_trips_in_a_0600_file_and_keeps_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("state")).unwrap();
        std::fs::write(
            path(root),
            r#"{"schema":1,"recipients":{"lxmf":["ab"]},"components":{}}"#,
        )
        .unwrap();
        let c = component();
        assert!(!load(root).acknowledged(&c));
        // without a hash, or with a wrong one: the exact notice and its hash
        let Err(ConsentCheck::Required(req)) = require(root, &c, None) else {
            panic!("consent required")
        };
        assert_eq!(req.warning_text, c.license.notice);
        assert_eq!(req.hash, c.notice_hash());
        assert_eq!(req.action, "component_enable");
        assert!(matches!(
            require(root, &c, Some(&"0".repeat(64))),
            Err(ConsentCheck::Required(_))
        ));
        // the right hash is recorded
        require(root, &c, Some(&c.notice_hash())).unwrap();
        let f = load(root);
        assert!(f.acknowledged(&c));
        assert_eq!(
            f.record("lxmf").acknowledged.unwrap().notice_text_version,
            NOTICE_TEXT_VERSION
        );
        // now no hash passes; a wrong hash still does not
        require(root, &c, None).unwrap();
        assert!(require(root, &c, Some("nope")).is_err());
        let mode = std::fs::metadata(path(root)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path(root)).unwrap()).unwrap();
        assert_eq!(v["recipients"]["lxmf"][0], "ab", "keys of others kept");
        // a changed notice asks again
        let mut changed = c.clone();
        changed.license.notice.push_str(" Changed.");
        assert!(!load(root).acknowledged(&changed));
        assert!(require(root, &changed, None).is_err());
    }

    #[test]
    fn an_invalid_file_is_moved_aside_and_nothing_is_acknowledged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("state")).unwrap();
        let c = component();
        for (n, bad) in [
            "{nope",
            "[1]",
            r#"{"schema":2}"#,
            r#"{"components":{"lxmf":{"acknowledged":true}}}"#,
            r#"{"components":{"Bad Id":{}}}"#,
        ]
        .iter()
        .enumerate()
        {
            std::fs::write(path(root), bad).unwrap();
            let f = load(root);
            assert!(!f.acknowledged(&c));
            let aside = root.join(format!("state/bridge.json.invalid-{}", n + 1));
            assert_eq!(std::fs::read_to_string(&aside).unwrap(), *bad, "kept");
            assert!(!path(root).exists());
        }
        // a link is moved itself, never followed
        let real = root.join("elsewhere.json");
        std::fs::write(&real, r#"{"components":{}}"#).unwrap();
        std::os::unix::fs::symlink(&real, path(root)).unwrap();
        let _ = load(root);
        assert!(real.exists());
        assert!(
            std::fs::symlink_metadata(root.join("state/bridge.json.invalid-6"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn the_consent_problem_carries_text_and_hash() {
        let c = component();
        let body = ConsentRequired::for_component(&c).problem().body();
        assert_eq!(body["status"], 409);
        assert_eq!(body["code"], "consent_required");
        assert_eq!(body["consent"]["action"], "component_enable");
        assert_eq!(body["consent"]["subject"], "lxmf");
        assert_eq!(body["consent"]["warning_text"], c.license.notice.as_str());
        assert_eq!(body["consent"]["hash"], c.notice_hash().as_str());
        assert!(body["consent"]["addons"].as_array().unwrap().is_empty());
    }
}
