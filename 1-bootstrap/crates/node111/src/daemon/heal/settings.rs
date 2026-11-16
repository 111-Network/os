//! Layer 9 (§17.1 every Layer 9 setting; §2.3 "an unknown value of any setting reads as that key's
//! default"; §4's path table, the `state/heal.json` row; §18.4 `RepairsSettings`): the settings
//! reader and writer (row D14). The settings route and its consents (`GET` and the loopback-only
//! `PATCH /api/v1/repairs/settings`, the §17.2 texts, the `409 consent_required` round trip) are
//! row D35's; they read and write through [`Handle`] (or [`load`] and [`update`]).
//!
//! **The keys** ([`Key`], §17.1's table in its order; [`Setting`] holds one value) and what each
//! reads with no file: `heal.autonomy` `propose` ([`Autonomy`], §2.1), `heal.fixes.receive`
//! `home_hub` ([`FixesReceive`]), `heal.fixes.senders` `[]` (contact NodeIds as 64 lowercase hex,
//! at most [`MAX_SENDERS`]; read only under `named_contacts`, §11.4 step 3), `heal.fixes.send`
//! `off` ([`FixesSend`]), `heal.ask_hub` `false`, `heal.serve_members` `false`,
//! `heal.code_drafts` `false`, `heal.notify` `off` ([`Notify`]).
//!
//! **The file** (§17.1, §4): JSON `{"schema": 1, …}`, each key under its dotted name (§18.4's
//! form), on one line (v1's own values compact, every other value as it was read), mode 0600,
//! written atomically (`util::fs::atomic_write`: a temp file, fsync, rename, the directory
//! fsynced) and **only on a change**: [`update`] writes nothing when the settings after the
//! caller's change read as they read before it, so a key set to the value it already reads, a
//! default set while the file is absent, a value set and set back, or a change the caller refused
//! touches nothing. **Absent ⇒ every default**, and no file exists until the first change. Only a
//! key a human set is written: one never set stays absent and follows the running release's
//! default (`heal.notify`'s default is the owner's to move, §17.1).
//!
//! **Kept, never dropped** (§17.1, Layer 8 §15.1's rule; Layer 8 §22 C1; §24 D9). Every entry is
//! held as its bytes ([`RawValue`]) and written back byte for byte until a human sets it. An
//! unknown key (a later release's setting or consent) is kept and ignored whatever its value holds
//! (a number past 64 bits or past `f64`, an exponent, a long decimal, an escape, whitespace,
//! nesting); a key given twice keeps its last occurrence, which is what serde_json, so a later
//! release, reads (v1 never writes one). A value that is not one of its key's values reads as that
//! key's default (§2.3: `heal.autonomy: "auto"`, left by a later release, reads `propose` after a
//! rollback) and stays stored until a human sets that key to another value. A rollback therefore
//! never drops a consent.
//!
//! **Consents** (§17.1, §17.2). The text hash of the consent a switch asks when it widens is stored
//! beside its key, under `<key>.consent_hash` ([`CONSENT_SUFFIX`]), as 64 lowercase hex (the
//! contract's `Sha256Hex`): `heal.ask_hub.consent_hash` (`heal_delegate`) and
//! `heal.serve_members.consent_hash` (`heal_serve`). `fix_send` is asked for each send with its
//! recipient in the text (§11.3), and `heal.code_drafts` rides the `opencode` component's own
//! licence notice, which that component's consent records: neither keeps a hash here
//! ([`Key::consent`]). A stored hash that is not 64 lowercase hex reads as none.
//! **A consent-gated switch is read only through [`Consents`]**, what the route (row D35) finds
//! owed now: [`HealSettings::ask_hub`], [`HealSettings::serve_members`],
//! [`HealSettings::code_drafts`], [`HealSettings::get`] and [`HealSettings::view`] read a switch
//! on only while its consent holds ([`HealSettings::consent_holds`]: its recorded hash is the hash
//! of the text owed now, so a changed text asks again; for `heal.code_drafts`, the `opencode`
//! component enabled on a hub, §8.3). So every consumer and the view get the node's answer, never
//! an optimistic one (§17.1), and nothing public reads a switch's stored value alone. The switch
//! and its hash are the route's to set together (row D35). No view here carries a hash (§17.3: the
//! agent never sees one).
//!
//! **An invalid file**: not a small regular file (a symlink, a directory, a FIFO, a socket, a
//! device, over [`MAX_FILE`]), not JSON, not an object or not `"schema": 1`. It reads as every
//! default, as an absent file does (§17.1 "Absent ⇒ every default"; Layer 8 §15.1's rule for an
//! invalid file, which §17.1 cites), and [`Loaded::invalid`] says why (log-safe: a category and a
//! position, never a value from the file). The defaults are inert as §17.1 and §24 D15 read it
//! (nothing changes, leaves or loads without a human; every consent-gated switch off, its consent
//! left with the file), but they are not the narrowest values: `propose` files proposals and
//! `home_hub` takes the home hub's fixes as proposals, each waiting for a human (§2.1, §2.2 item
//! 4), where a human may have chosen `observe` or `off`. The read writes nothing (§17.1: written
//! only on a change); the first change moves the file aside to `heal.json.invalid-<n>` (never
//! deleted; a symlink is moved itself, never followed), logs that once, and then writes. A file
//! that cannot be moved aside is never replaced: the change fails ([`UpdateError::MoveAside`]).
//!
//! **A file that cannot be read** is not an invalid one: an I/O error other than "not found" on
//! opening, inspecting or reading a regular file (`EACCES`, `EMFILE`, `EIO`, …) says nothing of
//! its bytes, which may be valid, so nothing is concluded from it ([`ReadError`]). [`load`]
//! returns the error; [`update`] fails with [`UpdateError::Read`] and neither moves nor writes
//! anything; [`Handle::reload`] keeps the settings it holds. Only at start, with nothing held
//! yet, does [`Handle::open`] hold every default (as for an invalid file) until a route call
//! reads the file.
//!
//! **No `auto` rung, no grant** (§2.3). [`Autonomy`] has §2.1's three rungs and no `auto`; v1
//! parses no grant and reads no `heal/grants` record: nothing here opens the store.
//!
//! **Reads and writes** (§21; the row's "never adds"). Nothing runs on its own: no task, thread,
//! timer or watcher. The daemon holds one [`Handle`], and every reader in it (the ladder, row
//! D13; the fix, delegation, draft and notification rows) a clone: [`Handle::open`] is the one
//! read at start; [`Handle::reload`] and [`Handle::update`] read once per route call; everything
//! else ([`Handle::current`], [`Handle::autonomy`]) answers from the copy in memory, which those
//! two refresh. [`load`] and [`update`] are the handle's halves, public for a caller that holds
//! none (a test); the free [`update`] refreshes no copy. A write
//! happens only on a change, so an idle node writes nothing here (§21 "writes"). [`update`] and
//! the handle's calls block on the file: a route calls them off the async runtime
//! (`spawn_blocking`), as the relay route does.

use id111::keyset::NodeId;
use serde_json::value::RawValue;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

/// File name inside the daemon's `state/` directory (§4's path table).
pub const HEAL_FILE: &str = "heal.json";
/// File schema (§17.1).
pub const SCHEMA: u64 = 1;
/// Largest file read back, checked before it is read: room for a later release's keys beside the
/// at most 1 KiB v1 writes (§4's path table). A larger file is invalid, and [`update`] never
/// writes what it could not read back.
pub const MAX_FILE: u64 = 64 * 1024;
/// Most `heal.fixes.senders` entries: with every key at its longest value and both consent
/// hashes, the largest file v1 writes stays within §4's 1 KiB (963 B).
pub const MAX_SENDERS: usize = 8;
/// What a consent's text hash is stored under, after its setting's dotted name (§17.1).
pub const CONSENT_SUFFIX: &str = ".consent_hash";
/// Attempts at a free `heal.json.invalid-<n>` name.
const MOVE_ASIDE_TRIES: u32 = 1000;
/// [`Loaded::invalid`] for anything but a small regular file.
const NOT_SMALL_FILE: &str = "not a small regular file";

/// Serialises every read-modify-write of the file in this process ([`update`], [`Handle`]).
static WRITE_LOCK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------- errors

/// Why a value was refused (a setter, or [`Setting::from_json`]). The route (row D35) answers it
/// `400 bad_request` and writes nothing ("a value outside §17.1's values"); the reader instead
/// reads the key's default (§2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsError {
    /// Not one of the key's values (§17.1): `heal.autonomy: "auto"` is one such (§2.3).
    BadValue(Key),
    /// More than [`MAX_SENDERS`] entries in `heal.fixes.senders`.
    TooManySenders,
    /// The key keeps no consent hash ([`Key::consent`]).
    NoConsent(Key),
    /// A consent hash that is not 64 lowercase hex.
    BadHash,
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::BadValue(k) => write!(f, "{} takes {}", k.name(), k.values()),
            SettingsError::TooManySenders => {
                write!(f, "heal.fixes.senders holds at most {MAX_SENDERS} NodeIds")
            }
            SettingsError::NoConsent(k) => write!(f, "{} keeps no consent", k.name()),
            SettingsError::BadHash => f.write_str("a consent hash is 64 lowercase hex"),
        }
    }
}

impl std::error::Error for SettingsError {}

// ---------------------------------------------------------------- keys and values

/// A Layer 9 setting (§17.1's table, in its order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    /// `heal.autonomy`: the ladder (§2.1).
    Autonomy,
    /// `heal.fixes.receive`: whose fixes this node takes (§11.4 step 3).
    FixesReceive,
    /// `heal.fixes.senders`: the named contacts, read only under `named_contacts` (§11.4 step 3).
    FixesSenders,
    /// `heal.fixes.send`: to whom a human may send a fix (§11.3).
    FixesSend,
    /// `heal.ask_hub`: a member asks its home hub (§12.5).
    AskHub,
    /// `heal.serve_members`: a hub answers its members (§12.4).
    ServeMembers,
    /// `heal.code_drafts`: code drafts on an opted-in hub (§8).
    CodeDrafts,
    /// `heal.notify`: the phone notification (§14.4).
    Notify,
}

impl Key {
    /// Every key, in §17.1's order.
    pub const ALL: [Key; 8] = [
        Key::Autonomy,
        Key::FixesReceive,
        Key::FixesSenders,
        Key::FixesSend,
        Key::AskHub,
        Key::ServeMembers,
        Key::CodeDrafts,
        Key::Notify,
    ];

    /// The dotted name: the file's key and §18.4's.
    pub fn name(self) -> &'static str {
        match self {
            Key::Autonomy => "heal.autonomy",
            Key::FixesReceive => "heal.fixes.receive",
            Key::FixesSenders => "heal.fixes.senders",
            Key::FixesSend => "heal.fixes.send",
            Key::AskHub => "heal.ask_hub",
            Key::ServeMembers => "heal.serve_members",
            Key::CodeDrafts => "heal.code_drafts",
            Key::Notify => "heal.notify",
        }
    }

    /// The key a dotted name names (a `RepairsSettingsPatch` member, an argument of
    /// `111 heal settings`); `None` for anything else, `schema` and `accept_warning` included.
    pub fn from_name(name: &str) -> Option<Key> {
        Key::ALL.into_iter().find(|k| k.name() == name)
    }

    /// The consent asked when this switch widens, whose text hash is stored beside it (§17.1,
    /// §17.2): `heal_delegate` for `heal.ask_hub`, `heal_serve` for `heal.serve_members`; `None`
    /// for the others (`fix_send` is asked per send, §11.3; `heal.code_drafts` rides the
    /// component's own licence notice).
    pub fn consent(self) -> Option<&'static str> {
        match self {
            Key::AskHub => Some("heal_delegate"),
            Key::ServeMembers => Some("heal_serve"),
            _ => None,
        }
    }

    /// The file's key the consent's text hash is stored under: the setting's name and
    /// [`CONSENT_SUFFIX`].
    pub fn consent_name(self) -> Option<&'static str> {
        match self {
            Key::AskHub => Some("heal.ask_hub.consent_hash"),
            Key::ServeMembers => Some("heal.serve_members.consent_hash"),
            _ => None,
        }
    }

    /// The key's values, as §17.1's table states them (a refusal's text).
    fn values(self) -> &'static str {
        match self {
            Key::Autonomy => "observe, diagnose or propose",
            Key::FixesReceive => "off, home_hub, paired_hubs or named_contacts",
            Key::FixesSenders => "a list of contact NodeIds, each 64 lowercase hex",
            Key::FixesSend => "off, home_hub or contacts",
            Key::AskHub | Key::ServeMembers | Key::CodeDrafts => "false or true",
            Key::Notify => "transitions or off",
        }
    }
}

/// `heal.autonomy` (§2.1): the ladder's three rungs, in order. v1 has no `auto` rung (§2.3): a
/// stored `"auto"` is not one of these values and reads as the default, `propose`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Autonomy {
    /// Records incidents; evaluates nothing.
    Observe,
    /// And T0, T1 and T2 (§5); the diagnosis is shown; no action is proposed.
    Diagnose,
    /// And a `repair` proposal for a diagnosed cause (§7); the default (D35 F1 A).
    #[default]
    Propose,
}

impl Autonomy {
    /// Every rung, in order.
    pub const ALL: [Autonomy; 3] = [Autonomy::Observe, Autonomy::Diagnose, Autonomy::Propose];

    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            Autonomy::Observe => "observe",
            Autonomy::Diagnose => "diagnose",
            Autonomy::Propose => "propose",
        }
    }

    /// The rung a stored word names (exact, lowercase); `None` for anything else.
    pub fn parse(word: &str) -> Option<Autonomy> {
        Autonomy::ALL.into_iter().find(|a| a.as_str() == word)
    }

    /// Whether this rung evaluates playbooks (§2.1): `diagnose` and `propose` do, `observe` does
    /// not.
    pub fn diagnoses(self) -> bool {
        self != Autonomy::Observe
    }

    /// Whether this rung files a `repair` proposal (§2.1): `propose` only.
    pub fn proposes(self) -> bool {
        self == Autonomy::Propose
    }
}

/// `heal.fixes.receive` (§17.1, §11.4 step 3): each value an explicit set of senders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FixesReceive {
    /// No fix is taken.
    Off,
    /// The enrolled home hub (the default).
    #[default]
    HomeHub,
    /// The home hub and the Layer 7 paired hubs (offered on a hub only: the route's `409
    /// needs_hub`).
    PairedHubs,
    /// The home hub and each NodeId of `heal.fixes.senders`.
    NamedContacts,
}

impl FixesReceive {
    /// Every value, in §17.1's order.
    pub const ALL: [FixesReceive; 4] = [
        FixesReceive::Off,
        FixesReceive::HomeHub,
        FixesReceive::PairedHubs,
        FixesReceive::NamedContacts,
    ];

    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            FixesReceive::Off => "off",
            FixesReceive::HomeHub => "home_hub",
            FixesReceive::PairedHubs => "paired_hubs",
            FixesReceive::NamedContacts => "named_contacts",
        }
    }

    /// The value a stored word names; `None` for anything else.
    pub fn parse(word: &str) -> Option<FixesReceive> {
        FixesReceive::ALL.into_iter().find(|v| v.as_str() == word)
    }
}

/// `heal.fixes.send` (§17.1, §11.3): each value an explicit set of recipients, each send under its
/// own `fix_send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FixesSend {
    /// No fix is sent (the default).
    #[default]
    Off,
    /// The enrolled home hub.
    HomeHub,
    /// The home hub or any Layer 5 contact the human picks for that send.
    Contacts,
}

impl FixesSend {
    /// Every value, in §17.1's order.
    pub const ALL: [FixesSend; 3] = [FixesSend::Off, FixesSend::HomeHub, FixesSend::Contacts];

    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            FixesSend::Off => "off",
            FixesSend::HomeHub => "home_hub",
            FixesSend::Contacts => "contacts",
        }
    }

    /// The value a stored word names; `None` for anything else.
    pub fn parse(word: &str) -> Option<FixesSend> {
        FixesSend::ALL.into_iter().find(|v| v.as_str() == word)
    }
}

/// `heal.notify` (§17.1, §14.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Notify {
    /// One notification when the Needs-you set goes from empty to non-empty (it does nothing on
    /// Linux or without the `termux-api` helper).
    Transitions,
    /// None (the default, until the `termux-api` deb is in the pinned closure).
    #[default]
    Off,
}

impl Notify {
    /// Every value, in §17.1's order.
    pub const ALL: [Notify; 2] = [Notify::Transitions, Notify::Off];

    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            Notify::Transitions => "transitions",
            Notify::Off => "off",
        }
    }

    /// The value a stored word names; `None` for anything else.
    pub fn parse(word: &str) -> Option<Notify> {
        Notify::ALL.into_iter().find(|v| v.as_str() == word)
    }
}

/// One setting's value (§17.1). [`Setting::from_json`] is the one table of values: the reader
/// takes a value it refuses as the key's default (§2.3), the route answers it `400` (row D35).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setting {
    /// `heal.autonomy`.
    Autonomy(Autonomy),
    /// `heal.fixes.receive`.
    FixesReceive(FixesReceive),
    /// `heal.fixes.senders`.
    FixesSenders(Vec<NodeId>),
    /// `heal.fixes.send`.
    FixesSend(FixesSend),
    /// `heal.ask_hub`.
    AskHub(bool),
    /// `heal.serve_members`.
    ServeMembers(bool),
    /// `heal.code_drafts`.
    CodeDrafts(bool),
    /// `heal.notify`.
    Notify(Notify),
}

impl Setting {
    /// The key this value is of.
    pub fn key(&self) -> Key {
        match self {
            Setting::Autonomy(_) => Key::Autonomy,
            Setting::FixesReceive(_) => Key::FixesReceive,
            Setting::FixesSenders(_) => Key::FixesSenders,
            Setting::FixesSend(_) => Key::FixesSend,
            Setting::AskHub(_) => Key::AskHub,
            Setting::ServeMembers(_) => Key::ServeMembers,
            Setting::CodeDrafts(_) => Key::CodeDrafts,
            Setting::Notify(_) => Key::Notify,
        }
    }

    /// The key's default (§17.1's Default column): what it reads with no file.
    pub fn default_of(key: Key) -> Setting {
        HealSettings::default().stored(key)
    }

    /// A JSON value of `key`, strictly: one of §17.1's values, else [`SettingsError::BadValue`]
    /// (or [`SettingsError::TooManySenders`]). Words are exact and lowercase, switches JSON
    /// booleans, `heal.fixes.senders` an array of at most [`MAX_SENDERS`] NodeIds as 64 lowercase
    /// hex, its length checked before an entry is parsed and only first occurrences kept.
    pub fn from_json(key: Key, v: &Value) -> Result<Setting, SettingsError> {
        let word = v.as_str();
        let value = match key {
            Key::Autonomy => word.and_then(Autonomy::parse).map(Setting::Autonomy),
            Key::FixesReceive => word
                .and_then(FixesReceive::parse)
                .map(Setting::FixesReceive),
            Key::FixesSenders => return parse_senders(v).map(Setting::FixesSenders),
            Key::FixesSend => word.and_then(FixesSend::parse).map(Setting::FixesSend),
            Key::AskHub => v.as_bool().map(Setting::AskHub),
            Key::ServeMembers => v.as_bool().map(Setting::ServeMembers),
            Key::CodeDrafts => v.as_bool().map(Setting::CodeDrafts),
            Key::Notify => word.and_then(Notify::parse).map(Setting::Notify),
        };
        value.ok_or(SettingsError::BadValue(key))
    }

    /// The stored JSON value.
    pub fn to_json(&self) -> Value {
        match self {
            Setting::Autonomy(a) => Value::from(a.as_str()),
            Setting::FixesReceive(r) => Value::from(r.as_str()),
            Setting::FixesSenders(ids) => Value::Array(
                ids.iter()
                    .map(|id| Value::from(id111::encoding::hex_encode(id.as_bytes())))
                    .collect(),
            ),
            Setting::FixesSend(s) => Value::from(s.as_str()),
            Setting::AskHub(on) | Setting::ServeMembers(on) | Setting::CodeDrafts(on) => {
                Value::Bool(*on)
            }
            Setting::Notify(n) => Value::from(n.as_str()),
        }
    }
}

/// A contract NodeId: exactly 64 lowercase hex (id111's machine string, which fails closed).
fn parse_node_id(s: &str) -> Option<NodeId> {
    id111::encoding::hex_decode(s).ok().map(NodeId::from)
}

/// `heal.fixes.senders` from JSON (the length checked before any entry is parsed).
fn parse_senders(v: &Value) -> Result<Vec<NodeId>, SettingsError> {
    let list = v
        .as_array()
        .ok_or(SettingsError::BadValue(Key::FixesSenders))?;
    if list.len() > MAX_SENDERS {
        return Err(SettingsError::TooManySenders);
    }
    let ids = list
        .iter()
        .map(|e| {
            e.as_str()
                .and_then(parse_node_id)
                .ok_or(SettingsError::BadValue(Key::FixesSenders))
        })
        .collect::<Result<Vec<_>, _>>()?;
    senders(ids)
}

/// At most [`MAX_SENDERS`] NodeIds given, first occurrences kept in order.
fn senders(ids: Vec<NodeId>) -> Result<Vec<NodeId>, SettingsError> {
    if ids.len() > MAX_SENDERS {
        return Err(SettingsError::TooManySenders);
    }
    let mut out: Vec<NodeId> = Vec::with_capacity(ids.len());
    for id in ids {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(out)
}

/// The contract's `Sha256Hex`: 64 lowercase hex.
fn is_sha256_hex(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

// ---------------------------------------------------------------- the settings

/// What the consent-gated switches need now (§17.1: "each consent records its text hash; a changed
/// text asks again"; "a switch shows the node's answer, never an optimistic one"). Row D35 builds
/// it: it renders §17.2's texts with their subjects as it would show them now and hashes them, and
/// reads the `opencode` component's state. A switch reads on only while its consent holds;
/// [`Consents::default`] holds none, so with it every consent-gated switch reads off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Consents<'a> {
    /// The hash of the `heal_delegate` text owed now (its subject the home hub's short id), or
    /// `None` where it is not offered (no home hub): `heal.ask_hub` reads on only while the hash
    /// recorded beside it is this one.
    pub heal_delegate: Option<&'a str>,
    /// The hash of the `heal_serve` text owed now, or `None` where it is not offered (not a hub,
    /// the route's `409 needs_hub`): `heal.serve_members` reads on only while the hash recorded
    /// beside it is this one.
    pub heal_serve: Option<&'a str>,
    /// Whether code drafts are offered now: `hub_role`, and the `opencode` component enabled
    /// under its current licence notice (that consent is the component's own record,
    /// `N/components/consent.rs`; §8.3): `heal.code_drafts` reads on only then.
    pub opencode: bool,
}

/// The settings of `state/heal.json` (§17.1): every entry of the file but `schema`, unknown ones
/// included, each held as its bytes (module doc, "Kept, never dropped"); each typed value is read
/// from them (an unknown value reads as its key's default). Equality compares the stored bytes;
/// [`update`] writes when what the settings read changed.
#[derive(Debug, Clone, Default)]
pub struct HealSettings {
    entries: BTreeMap<String, Box<RawValue>>,
}

impl PartialEq for HealSettings {
    fn eq(&self, other: &HealSettings) -> bool {
        self.entries.len() == other.entries.len()
            && self
                .entries
                .iter()
                .zip(&other.entries)
                .all(|((k, v), (ok, ov))| k == ok && v.get() == ov.get())
    }
}

impl Eq for HealSettings {}

/// `v` as stored bytes, compact. Serialising a [`Value`] never fails (its numbers are finite, its
/// keys strings): `None` is unreachable, and a caller reads it as a refusal, never a panic.
fn to_raw(v: &Value) -> Option<Box<RawValue>> {
    serde_json::value::to_raw_value(v).ok()
}

impl HealSettings {
    /// What `key` reads, as [`HealSettings::view`] shows it: its stored value when that is one of
    /// its values, else its default; a consent-gated switch on only while its consent holds
    /// ([`Consents`]).
    pub fn get(&self, key: Key, consents: &Consents<'_>) -> Setting {
        match key {
            Key::AskHub => Setting::AskHub(self.ask_hub(consents)),
            Key::ServeMembers => Setting::ServeMembers(self.serve_members(consents)),
            Key::CodeDrafts => Setting::CodeDrafts(self.code_drafts(consents)),
            _ => self.stored(key),
        }
    }

    /// What `key` stores, read: its stored value when that is one of its values, else its default;
    /// a switch as stored, whatever its consent. Private: a set and a change compare it, and every
    /// public read of a switch goes through [`Consents`].
    fn stored(&self, key: Key) -> Setting {
        match key {
            Key::Autonomy => Setting::Autonomy(self.autonomy()),
            Key::FixesReceive => Setting::FixesReceive(self.fixes_receive()),
            Key::FixesSenders => Setting::FixesSenders(self.fixes_senders()),
            Key::FixesSend => Setting::FixesSend(self.fixes_send()),
            Key::AskHub => Setting::AskHub(self.switch(key)),
            Key::ServeMembers => Setting::ServeMembers(self.switch(key)),
            Key::CodeDrafts => Setting::CodeDrafts(self.switch(key)),
            Key::Notify => Setting::Notify(self.notify()),
        }
    }

    /// The value stored under the file's key `name`, parsed; `None` when absent or when its bytes
    /// do not parse into a [`Value`] (a number past `f64`, nesting past serde_json's limit): such a
    /// value reads as its key's default and stays stored as it is.
    fn value(&self, name: &str) -> Option<Value> {
        serde_json::from_str(self.entries.get(name)?.get()).ok()
    }

    /// The stored value of `key` read through `parse`, or the type's default (§17.1's Default
    /// column: each value type's `#[default]`, `false`, `[]`).
    fn read<T: Default>(&self, key: Key, parse: impl FnOnce(&Value) -> Option<T>) -> T {
        self.value(key.name())
            .as_ref()
            .and_then(parse)
            .unwrap_or_default()
    }

    /// A switch as stored (`false` by default), whatever its consent.
    fn switch(&self, key: Key) -> bool {
        self.read(key, Value::as_bool)
    }

    /// `heal.autonomy` (§2.1; `propose` by default, and for `"auto"`, §2.3).
    pub fn autonomy(&self) -> Autonomy {
        self.read(Key::Autonomy, |v| v.as_str().and_then(Autonomy::parse))
    }

    /// `heal.fixes.receive` (`home_hub` by default).
    pub fn fixes_receive(&self) -> FixesReceive {
        self.read(Key::FixesReceive, |v| {
            v.as_str().and_then(FixesReceive::parse)
        })
    }

    /// `heal.fixes.senders` (`[]` by default; the receiver reads it only under `named_contacts`,
    /// §11.4 step 3).
    pub fn fixes_senders(&self) -> Vec<NodeId> {
        self.read(Key::FixesSenders, |v| parse_senders(v).ok())
    }

    /// `heal.fixes.send` (`off` by default).
    pub fn fixes_send(&self) -> FixesSend {
        self.read(Key::FixesSend, |v| v.as_str().and_then(FixesSend::parse))
    }

    /// `heal.ask_hub` (`false` by default): on only while its `heal_delegate` consent holds for
    /// the text owed now ([`Consents::heal_delegate`]; §12.5).
    pub fn ask_hub(&self, consents: &Consents<'_>) -> bool {
        self.switch(Key::AskHub)
            && consents
                .heal_delegate
                .is_some_and(|owed| self.consent_holds(Key::AskHub, owed))
    }

    /// `heal.serve_members` (`false` by default): on only while its `heal_serve` consent holds for
    /// the text owed now ([`Consents::heal_serve`]; §12.4).
    pub fn serve_members(&self, consents: &Consents<'_>) -> bool {
        self.switch(Key::ServeMembers)
            && consents
                .heal_serve
                .is_some_and(|owed| self.consent_holds(Key::ServeMembers, owed))
    }

    /// `heal.code_drafts` (`false` by default): on only while code drafts are offered
    /// ([`Consents::opencode`]; §8.3 "with `heal.code_drafts` on and the component enabled").
    pub fn code_drafts(&self, consents: &Consents<'_>) -> bool {
        self.switch(Key::CodeDrafts) && consents.opencode
    }

    /// `heal.notify` (`off` by default).
    pub fn notify(&self) -> Notify {
        self.read(Key::Notify, |v| v.as_str().and_then(Notify::parse))
    }

    /// Set one key (§17.1). A value equal to what the key stores changes nothing: its stored
    /// bytes, an unknown value included, stay as they are (a rollback keeps them, §2.3). Another
    /// value replaces them. A switch is set as stored: it reads on only with its consent recorded
    /// beside it ([`HealSettings::set_consent_hash`]; the route sets both, row D35) or, for
    /// `heal.code_drafts`, while offered ([`Consents`]). `heal.fixes.senders` takes at most
    /// [`MAX_SENDERS`] NodeIds, first occurrences kept. Returns whether the stored settings
    /// changed.
    pub fn set(&mut self, value: Setting) -> Result<bool, SettingsError> {
        let value = match value {
            Setting::FixesSenders(ids) => Setting::FixesSenders(senders(ids)?),
            other => other,
        };
        let key = value.key();
        if self.stored(key) == value {
            return Ok(false);
        }
        let raw = to_raw(&value.to_json()).ok_or(SettingsError::BadValue(key))?;
        self.entries.insert(key.name().to_string(), raw);
        Ok(true)
    }

    /// The text hash stored beside `key` (§17.1), when the key keeps one and it is 64 lowercase
    /// hex.
    pub fn consent_hash(&self, key: Key) -> Option<String> {
        match self.value(key.consent_name()?)? {
            Value::String(h) if is_sha256_hex(&h) => Some(h),
            _ => None,
        }
    }

    /// Store (`Some`) or clear (`None`) the text hash beside `key`: the hash of the consent text
    /// the human accepted (§17.1 "each consent records its text hash"). Refused for a key that
    /// keeps none and for a hash that is not 64 lowercase hex. Returns whether the stored settings
    /// changed.
    pub fn set_consent_hash(
        &mut self,
        key: Key,
        hash: Option<&str>,
    ) -> Result<bool, SettingsError> {
        let name = key.consent_name().ok_or(SettingsError::NoConsent(key))?;
        if hash.is_some_and(|h| !is_sha256_hex(h)) {
            return Err(SettingsError::BadHash);
        }
        if self.consent_hash(key).as_deref() == hash {
            return Ok(false);
        }
        match hash {
            Some(h) => {
                let raw = to_raw(&Value::from(h)).ok_or(SettingsError::BadHash)?;
                self.entries.insert(name.to_string(), raw);
            }
            None => {
                self.entries.remove(name);
            }
        }
        Ok(true)
    }

    /// The one test of a recorded consent (§17.1): `key` keeps a hash and it is `owed`, the hash
    /// of the text owed now. A text that changed since has another hash, so it asks again.
    pub fn consent_holds(&self, key: Key, owed: &str) -> bool {
        self.consent_hash(key).is_some_and(|h| h == owed)
    }

    /// §18.4's `RepairsSettings`: `{schema}` plus each key of §17.1 under its dotted name with the
    /// value it reads ([`HealSettings::get`]: a consent-gated switch on only while its consent
    /// holds, the node's answer, §17.1). Never a consent hash, never an unknown key.
    pub fn view(&self, consents: &Consents<'_>) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("schema".to_string(), Value::from(SCHEMA));
        for key in Key::ALL {
            m.insert(key.name().to_string(), self.get(key, consents).to_json());
        }
        Value::Object(m)
    }

    /// Whether `self` reads as `other` does: every key's stored value and every consent hash. This
    /// is what a change is (§17.1 "written only on a change"): settings that read the same are not
    /// written, whatever their stored bytes differ in (an unknown key, which nothing here
    /// changes; an unknown value and a value set back to what it read, which leave the file as it
    /// was).
    fn reads_as(&self, other: &HealSettings) -> bool {
        Key::ALL.into_iter().all(|k| {
            self.stored(k) == other.stored(k) && self.consent_hash(k) == other.consent_hash(k)
        })
    }
}

// ---------------------------------------------------------------- the file

/// The file path under the daemon's `state/` directory.
pub fn path(state_dir: &Path) -> PathBuf {
    state_dir.join(HEAL_FILE)
}

/// What [`load`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// The settings to run with: the file's, or every default when it is absent or invalid.
    pub settings: HealSettings,
    /// `Some(why)` when a file is present but invalid (module doc): log-safe, never a value read
    /// from the file. The caller logs it once; the first change moves the file aside.
    pub invalid: Option<String>,
}

/// Why `state/heal.json` could not be read (module doc, "A file that cannot be read"): an I/O
/// error other than "not found" on opening, inspecting or reading what is, as far as could be
/// told, a regular file. Its bytes may be valid, so nothing is concluded from it: [`load`] and
/// [`update`] return this and a [`Handle`] keeps what it holds. Log-safe: the error's text, never
/// a byte of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadError(String);

impl From<std::io::Error> for ReadError {
    fn from(e: std::io::Error) -> ReadError {
        ReadError(e.to_string())
    }
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot read {HEAL_FILE}: {}", self.0)
    }
}

impl std::error::Error for ReadError {}

/// What one read of the path found.
enum Found {
    Absent,
    Bytes(Vec<u8>),
    /// Not a small regular file: invalid ([`NOT_SMALL_FILE`]).
    NotSmallFile,
    /// An I/O error on a regular file, or on a path that could not be inspected: it may be valid.
    Unreadable(std::io::Error),
}

/// Opens `p` to read it: never follows a symlink (`ELOOP`), never blocks on a FIFO. A test can
/// make it fail with a chosen `errno` (`tests::fail_opens`), since a test running as root cannot
/// make a regular file unreadable.
fn open_file(p: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    #[cfg(test)]
    if let Some(e) = tests::injected_open_error() {
        return Err(e);
    }
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(p)
}

/// One read of the file: checks the open file is a regular file within [`MAX_FILE`] before
/// reading it, and reads at most that. An open that fails is classed by what the path is, never
/// followed: gone (absent), not a regular file (a symlink's `ELOOP`, a socket's `ENXIO`: invalid),
/// or a regular file, or a path that cannot be inspected (unreadable). An error inspecting or
/// reading the open file is unreadable.
fn read_file(p: &Path) -> Found {
    use std::io::{ErrorKind, Read};
    #[cfg(test)]
    tests::count_read();
    let file = match open_file(p) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return Found::Absent,
        Err(e) => {
            return match std::fs::symlink_metadata(p) {
                Ok(m) if !m.file_type().is_file() => Found::NotSmallFile,
                Err(m) if m.kind() == ErrorKind::NotFound => Found::Absent,
                _ => Found::Unreadable(e),
            }
        }
    };
    match file.metadata() {
        Ok(m) if m.file_type().is_file() && m.len() <= MAX_FILE => {}
        Ok(_) => return Found::NotSmallFile,
        Err(e) => return Found::Unreadable(e),
    }
    let mut buf = Vec::new();
    if let Err(e) = file.take(MAX_FILE + 1).read_to_end(&mut buf) {
        return Found::Unreadable(e);
    }
    if buf.len() as u64 > MAX_FILE {
        return Found::NotSmallFile;
    }
    Found::Bytes(buf)
}

/// A serde failure as a log-safe text: its category and position, never the bytes it saw.
fn parse_error(e: &serde_json::Error) -> String {
    use serde_json::error::Category;
    let what = match e.classify() {
        Category::Io => "io",
        Category::Syntax => "syntax",
        Category::Data => "a value out of shape",
        Category::Eof => "truncated",
    };
    format!(
        "not JSON: {what} at line {} column {}",
        e.line(),
        e.column()
    )
}

/// The file's bytes: one JSON object whose `schema` is 1; every other entry is kept as its bytes
/// ([`RawValue`]: a value is scanned, never converted, so no number is rounded or refused). A key
/// given twice keeps its last occurrence, as serde_json reads it.
fn decode(bytes: &[u8]) -> Result<HealSettings, String> {
    let mut entries: BTreeMap<String, Box<RawValue>> = match serde_json::from_slice(bytes) {
        Ok(entries) => entries,
        // a raw value takes any JSON value, so the one data error is a document that is not an
        // object
        Err(e) if e.classify() == serde_json::error::Category::Data => {
            return Err("not a JSON object".into())
        }
        Err(e) => return Err(parse_error(&e)),
    };
    let schema = entries.remove("schema");
    if schema.and_then(|raw| serde_json::from_str::<u64>(raw.get()).ok()) != Some(SCHEMA) {
        return Err("schema is not 1".into());
    }
    Ok(HealSettings { entries })
}

/// The file's bytes: `{"schema":1` first, then every entry in key order, each value's bytes as
/// held, on one line (a value read with whitespace inside keeps it).
fn encode(s: &HealSettings) -> Result<Vec<u8>, serde_json::Error> {
    let mut out = format!("{{\"schema\":{SCHEMA}").into_bytes();
    for (k, v) in &s.entries {
        out.push(b',');
        serde_json::to_writer(&mut out, k)?;
        out.push(b':');
        out.extend_from_slice(v.get().as_bytes());
    }
    out.extend_from_slice(b"}\n");
    Ok(out)
}

/// Read `state/heal.json` once: absent ⇒ every default; invalid ⇒ every default and why (module
/// doc); a file that cannot be read ⇒ [`ReadError`], nothing concluded. Never writes, never
/// creates the file, never moves it.
pub fn load(state_dir: &Path) -> Result<Loaded, ReadError> {
    let (settings, invalid) = match read_file(&path(state_dir)) {
        Found::Absent => (HealSettings::default(), None),
        Found::NotSmallFile => (HealSettings::default(), Some(NOT_SMALL_FILE.to_string())),
        Found::Unreadable(e) => return Err(e.into()),
        Found::Bytes(bytes) => match decode(&bytes) {
            Ok(s) => (s, None),
            Err(why) => (HealSettings::default(), Some(why)),
        },
    };
    Ok(Loaded { settings, invalid })
}

/// Move an invalid file aside before the first change replaces it (never deleted; a symlink is
/// moved itself, never followed), logged once. On failure it stays where it is and the caller
/// writes nothing ([`UpdateError::MoveAside`]): an invalid file is never replaced unkept (a later
/// release's schema keeps its consents).
fn move_aside(state_dir: &Path, why: &str) -> Result<(), String> {
    let from = path(state_dir);
    for n in 1..=MOVE_ASIDE_TRIES {
        let to = state_dir.join(format!("{HEAL_FILE}.invalid-{n}"));
        if std::fs::symlink_metadata(&to).is_ok() {
            continue;
        }
        return match std::fs::rename(&from, &to) {
            Ok(()) => {
                eprintln!(
                    "111 heal: {} invalid ({why}); moved to {}",
                    from.display(),
                    to.display()
                );
                Ok(())
            }
            Err(e) => Err(format!("cannot move {} aside: {e}", from.display())),
        };
    }
    Err(format!(
        "cannot move {} aside: no free {HEAL_FILE}.invalid-<n> name",
        from.display()
    ))
}

/// Read-modify-write under the process's write lock: one read ([`load`]), the caller's `change`,
/// and a write only when the settings then read differently (atomic, 0600; an invalid file moved
/// aside first). Returns the settings as the file now holds them and whether a write happened.
/// A file that cannot be read, a `change` that fails, or one that changes nothing leaves the file
/// untouched; `change` must not call back into this module's writers (the lock is not
/// re-entrant).
pub fn update<E>(
    state_dir: &Path,
    change: impl FnOnce(&mut HealSettings) -> Result<(), E>,
) -> Result<(HealSettings, bool), UpdateError<E>> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    update_locked(state_dir, change)
}

/// [`update`] with [`WRITE_LOCK`] held by the caller.
fn update_locked<E>(
    state_dir: &Path,
    change: impl FnOnce(&mut HealSettings) -> Result<(), E>,
) -> Result<(HealSettings, bool), UpdateError<E>> {
    let Loaded { settings, invalid } = load(state_dir).map_err(UpdateError::Read)?;
    let mut after = settings.clone();
    change(&mut after).map_err(UpdateError::Change)?;
    if after.reads_as(&settings) {
        return Ok((settings, false));
    }
    let bytes = encode(&after).map_err(|e| UpdateError::Write(e.to_string()))?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(UpdateError::TooLarge);
    }
    if let Some(why) = invalid {
        move_aside(state_dir, &why).map_err(UpdateError::MoveAside)?;
    }
    crate::util::fs::atomic_write(&path(state_dir), &bytes, 0o600)
        .map_err(|e| UpdateError::Write(format!("{e:#}")))?;
    Ok((after, true))
}

/// Why [`update`] failed.
#[derive(Debug)]
pub enum UpdateError<E> {
    /// The file could not be read ([`ReadError`]): nothing was moved or written, and a
    /// [`Handle`] keeps the settings it holds.
    Read(ReadError),
    /// The change refused itself; nothing was written.
    Change(E),
    /// The settings would pass [`MAX_FILE`]; nothing was written.
    TooLarge,
    /// An invalid file could not be moved aside: nothing was written, and it stays where it is.
    MoveAside(String),
    /// The write failed; the file is as it was (an invalid one moved aside).
    Write(String),
}

impl<E: std::fmt::Display> std::fmt::Display for UpdateError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::Read(e) => write!(f, "{e}"),
            UpdateError::Change(e) => e.fmt(f),
            UpdateError::TooLarge => write!(f, "{HEAL_FILE} would pass {MAX_FILE} bytes"),
            UpdateError::MoveAside(e) => write!(f, "{e}; nothing written"),
            UpdateError::Write(e) => write!(f, "cannot write {HEAL_FILE}: {e}"),
        }
    }
}

impl<E: std::fmt::Debug + std::fmt::Display> std::error::Error for UpdateError<E> {}

// ---------------------------------------------------------------- the copy in memory

/// The settings held in memory for the daemon (module doc, "Reads and writes"): read once at
/// start, read again only by a route call ([`Handle::reload`], [`Handle::update`]), and answered
/// from memory everywhere else. Clones share one copy.
#[derive(Debug, Clone)]
pub struct Handle {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    state_dir: PathBuf,
    current: RwLock<HealSettings>,
}

impl Handle {
    /// The one read at start. Returns the handle and, when the file is invalid or cannot be read,
    /// why (the start logs it once). With nothing held yet, a file that cannot be read holds every
    /// default, as an invalid one does, until a route call reads it ([`Handle::reload`],
    /// [`Handle::update`]).
    pub fn open(state_dir: &Path) -> (Handle, Option<String>) {
        let (settings, why) = match load(state_dir) {
            Ok(Loaded { settings, invalid }) => (settings, invalid),
            Err(e) => (HealSettings::default(), Some(e.to_string())),
        };
        let inner = Inner {
            state_dir: state_dir.to_path_buf(),
            current: RwLock::new(settings),
        };
        (
            Handle {
                inner: Arc::new(inner),
            },
            why,
        )
    }

    /// The settings held now (a copy; no read).
    pub fn current(&self) -> HealSettings {
        self.inner
            .current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// `heal.autonomy` held now (no read): the ladder's rung (§2.1).
    pub fn autonomy(&self) -> Autonomy {
        self.inner
            .current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .autonomy()
    }

    /// A route's read (`GET /api/v1/repairs/settings`): one read of the file, held from now on. A
    /// file that cannot be read answers [`ReadError`] and the settings held stay as they are.
    pub fn reload(&self) -> Result<Loaded, ReadError> {
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let loaded = load(&self.inner.state_dir)?;
        self.hold(loaded.settings.clone());
        Ok(loaded)
    }

    /// A route's write (`PATCH /api/v1/repairs/settings`): [`update`], and the settings it
    /// returns are held from now on; on any error the settings held stay as they are.
    pub fn update<E>(
        &self,
        change: impl FnOnce(&mut HealSettings) -> Result<(), E>,
    ) -> Result<(HealSettings, bool), UpdateError<E>> {
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (settings, wrote) = update_locked(&self.inner.state_dir, change)?;
        self.hold(settings.clone());
        Ok((settings, wrote))
    }

    /// Hold `s` (with [`WRITE_LOCK`] held, so the copy never goes back to an older read).
    fn hold(&self, s: HealSettings) {
        *self
            .inner
            .current
            .write()
            .unwrap_or_else(|e| e.into_inner()) = s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

    thread_local! {
        /// Reads of the file on this test's thread (each test runs on its own thread).
        static READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        /// The `errno` every open of the file fails with on this test's thread, while set.
        static FAIL_OPEN: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
    }

    /// Counts one read ([`read_file`], test builds only).
    pub(super) fn count_read() {
        READS.with(|c| c.set(c.get() + 1));
    }

    fn reads() -> usize {
        READS.with(|c| c.get())
    }

    /// The error an open of the file fails with on this test's thread ([`open_file`], test builds
    /// only): a regular file a test running as root cannot otherwise make unreadable.
    pub(super) fn injected_open_error() -> Option<std::io::Error> {
        FAIL_OPEN
            .with(|c| c.get())
            .map(std::io::Error::from_raw_os_error)
    }

    /// Every open of the file on this thread fails with `errno` from now on (`None`: opens again).
    fn fail_opens(errno: Option<i32>) {
        FAIL_OPEN.with(|c| c.set(errno));
    }

    /// The hash of the `heal_delegate` text owed in these tests (`hash('a')`).
    const OWED_A: &str = concat!(
        "aaaaaaaaaaaaaaaa",
        "aaaaaaaaaaaaaaaa",
        "aaaaaaaaaaaaaaaa",
        "aaaaaaaaaaaaaaaa"
    );
    /// The hash of the `heal_serve` text owed in these tests (`hash('b')`).
    const OWED_B: &str = concat!(
        "bbbbbbbbbbbbbbbb",
        "bbbbbbbbbbbbbbbb",
        "bbbbbbbbbbbbbbbb",
        "bbbbbbbbbbbbbbbb"
    );

    /// Both consents owed as `OWED_A` and `OWED_B`, and code drafts offered: a switch reads as
    /// stored where its consent is recorded as owed.
    fn owed() -> Consents<'static> {
        Consents {
            heal_delegate: Some(OWED_A),
            heal_serve: Some(OWED_B),
            opencode: true,
        }
    }

    fn node(b: u8) -> NodeId {
        NodeId::from([b; 32])
    }

    fn hex_id(b: u8) -> String {
        id111::encoding::hex_encode(&[b; 32])
    }

    fn hash(b: char) -> String {
        std::iter::repeat_n(b, 64).collect()
    }

    fn write_raw(dir: &Path, bytes: impl AsRef<[u8]>) {
        std::fs::write(path(dir), bytes).unwrap();
    }

    fn read_raw(dir: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path(dir)).unwrap()).unwrap()
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// The file as the filesystem holds it: inode, change and modification times, bytes.
    fn stamp(dir: &Path) -> (u64, i64, i64, i64, i64, Vec<u8>) {
        let p = path(dir);
        let m = std::fs::symlink_metadata(&p).unwrap();
        (
            m.ino(),
            m.ctime(),
            m.ctime_nsec(),
            m.mtime(),
            m.mtime_nsec(),
            std::fs::read(&p).unwrap(),
        )
    }

    /// One `set` as an [`update`] change.
    fn set_one(value: Setting) -> impl FnOnce(&mut HealSettings) -> Result<(), SettingsError> {
        move |s| s.set(value).map(|_| ())
    }

    #[test]
    fn the_defaults_with_no_file() {
        assert_eq!((OWED_A, OWED_B), (hash('a').as_str(), hash('b').as_str()));
        let dir = tempfile::tempdir().unwrap();
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded.invalid, None);
        let s = loaded.settings;
        assert_eq!(s.autonomy(), Autonomy::Propose);
        assert_eq!(s.fixes_receive(), FixesReceive::HomeHub);
        assert_eq!(s.fixes_senders(), Vec::<NodeId>::new());
        assert_eq!(s.fixes_send(), FixesSend::Off);
        assert!(!s.ask_hub(&owed()));
        assert!(!s.serve_members(&owed()));
        assert!(!s.code_drafts(&owed()));
        assert_eq!(s.notify(), Notify::Off);
        for k in Key::ALL {
            assert_eq!(s.consent_hash(k), None, "{}", k.name());
            assert_eq!(s.get(k, &owed()), Setting::default_of(k), "{}", k.name());
        }
        assert_eq!(
            s.view(&owed()),
            json!({
                "schema": 1,
                "heal.autonomy": "propose",
                "heal.fixes.receive": "home_hub",
                "heal.fixes.senders": [],
                "heal.fixes.send": "off",
                "heal.ask_hub": false,
                "heal.serve_members": false,
                "heal.code_drafts": false,
                "heal.notify": "off"
            })
        );
        assert!(listing(dir.path()).is_empty(), "a read creates nothing");
    }

    #[test]
    fn an_unknown_key_survives_a_write() {
        let dir = tempfile::tempdir().unwrap();
        let later = json!({
            "schema": 1,
            "heal.autonomy": "diagnose",
            "heal.later": {"on": true, "list": [1, "two", null, {"z": 0.5}]},
            "heal.later.consent_hash": hash('c'),
            "heal.code_drafts.consent_hash": hash('d'),
            "zz.other": 7
        });
        write_raw(dir.path(), later.to_string());
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded.invalid, None, "an unknown key leaves the file valid");
        assert_eq!(loaded.settings.autonomy(), Autonomy::Diagnose);
        assert_eq!(
            loaded.settings.view(&owed()).as_object().unwrap().len(),
            1 + Key::ALL.len(),
            "ignored: the view holds §17.1's keys only"
        );

        let (s, wrote) = update(dir.path(), set_one(Setting::Notify(Notify::Transitions))).unwrap();
        assert!(wrote);
        assert_eq!(s.notify(), Notify::Transitions);
        let mut want = later.clone();
        want["heal.notify"] = json!("transitions");
        assert_eq!(
            read_raw(dir.path()),
            want,
            "kept and written back untouched"
        );
        let mode = std::fs::metadata(path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);

        // and through a second change, and a change of the key read beside them
        update(dir.path(), |s: &mut HealSettings| {
            s.set(Setting::Autonomy(Autonomy::Observe))?;
            s.set(Setting::AskHub(true))?;
            s.set_consent_hash(Key::AskHub, Some(&hash('a')))?;
            Ok::<(), SettingsError>(())
        })
        .unwrap();
        want["heal.autonomy"] = json!("observe");
        want["heal.ask_hub"] = json!(true);
        want["heal.ask_hub.consent_hash"] = json!(hash('a'));
        assert_eq!(read_raw(dir.path()), want);
    }

    #[test]
    fn auto_and_a_misspelt_value_read_the_default() {
        for bad in [
            json!("auto"),
            json!("Propose"),
            json!("obsrve"),
            json!(" propose"),
            json!(""),
            json!(null),
            json!(2),
            json!(["observe"]),
        ] {
            let dir = tempfile::tempdir().unwrap();
            write_raw(
                dir.path(),
                json!({"schema": 1, "heal.autonomy": bad, "heal.notify": "transitions"})
                    .to_string(),
            );
            let l = load(dir.path()).unwrap();
            assert_eq!(
                l.invalid, None,
                "{bad}: an unknown value leaves the file valid"
            );
            assert_eq!(l.settings.autonomy(), Autonomy::Propose, "{bad}");
            assert_eq!(
                l.settings.notify(),
                Notify::Transitions,
                "{bad}: the others read"
            );
        }

        // every key: an unknown value reads as that key's default
        let unknown = json!({
            "schema": 1,
            "heal.autonomy": "auto",
            "heal.fixes.receive": "everyone",
            "heal.fixes.senders": [hex_id(0xab).to_uppercase()],
            "heal.fixes.send": "all",
            "heal.ask_hub": "true",
            "heal.ask_hub.consent_hash": hash('A'),
            "heal.serve_members": 1,
            "heal.serve_members.consent_hash": "ab",
            "heal.code_drafts": "yes",
            "heal.notify": "on"
        });
        let dir = tempfile::tempdir().unwrap();
        write_raw(dir.path(), unknown.to_string());
        let l = load(dir.path()).unwrap();
        assert_eq!(l.invalid, None);
        for k in Key::ALL {
            assert_eq!(l.settings.stored(k), Setting::default_of(k), "{}", k.name());
            assert_eq!(l.settings.consent_hash(k), None, "{}", k.name());
        }
        assert_eq!(
            l.settings.view(&owed()),
            HealSettings::default().view(&owed())
        );

        // a word in another case is misspelt too, whichever value it would name
        for (k, word) in [
            (Key::Autonomy, "OBSERVE"),
            (Key::FixesReceive, "Off"),
            (Key::FixesSend, "Contacts"),
            (Key::Notify, "Transitions"),
        ] {
            let mut doc = json!({"schema": 1});
            doc[k.name()] = json!(word);
            write_raw(dir.path(), doc.to_string());
            let s = load(dir.path()).unwrap().settings;
            assert_eq!(s.get(k, &owed()), Setting::default_of(k), "{word}");
        }
        write_raw(dir.path(), unknown.to_string());

        // the route's 400: the same table refuses each of them
        for k in Key::ALL {
            assert_eq!(
                Setting::from_json(k, &unknown[k.name()]),
                Err(SettingsError::BadValue(k)),
                "{}",
                k.name()
            );
        }

        // a rollback keeps them: a change of another key writes them back, a key set to what it
        // reads keeps its stored value, and a human's other value replaces it
        update(dir.path(), set_one(Setting::Notify(Notify::Transitions))).unwrap();
        let mut want = unknown.clone();
        want["heal.notify"] = json!("transitions");
        assert_eq!(read_raw(dir.path()), want);
        let (_, wrote) = update(dir.path(), set_one(Setting::Autonomy(Autonomy::Propose))).unwrap();
        assert!(!wrote, "auto already reads propose");
        assert_eq!(read_raw(dir.path())["heal.autonomy"], json!("auto"));
        update(dir.path(), set_one(Setting::Autonomy(Autonomy::Observe))).unwrap();
        assert_eq!(read_raw(dir.path())["heal.autonomy"], json!("observe"));
    }

    #[test]
    fn a_write_with_no_change_touches_nothing() {
        // absent: every key set to its default, every hash cleared, creates nothing
        let dir = tempfile::tempdir().unwrap();
        let (s, wrote) = update(dir.path(), |s: &mut HealSettings| {
            for k in Key::ALL {
                s.set(Setting::default_of(k))?;
                if k.consent().is_some() {
                    s.set_consent_hash(k, None)?;
                }
            }
            Ok::<(), SettingsError>(())
        })
        .unwrap();
        assert!(!wrote);
        assert_eq!(s, HealSettings::default());
        assert!(listing(dir.path()).is_empty(), "no file, no temp file");

        // present: the same values again leave its inode, times and bytes as they were
        let same = |s: &mut HealSettings| {
            s.set(Setting::Autonomy(Autonomy::Diagnose))?;
            s.set(Setting::FixesSenders(vec![node(1), node(2)]))?;
            s.set(Setting::AskHub(true))?;
            s.set_consent_hash(Key::AskHub, Some(&hash('a')))?;
            Ok::<(), SettingsError>(())
        };
        let (_, wrote) = update(dir.path(), same).unwrap();
        assert!(wrote);
        let before = stamp(dir.path());
        let names = listing(dir.path());
        let (_, wrote) = update(dir.path(), same).unwrap();
        assert!(!wrote);
        assert_eq!(stamp(dir.path()), before);
        assert_eq!(listing(dir.path()), names);

        // a value set and set back in one change
        let (_, wrote) = update(dir.path(), |s: &mut HealSettings| {
            s.set(Setting::Notify(Notify::Transitions))?;
            s.set(Setting::Notify(Notify::Off))?;
            s.set_consent_hash(Key::AskHub, None)?;
            s.set_consent_hash(Key::AskHub, Some(&hash('a')))?;
            Ok::<(), SettingsError>(())
        })
        .unwrap();
        assert!(!wrote);
        assert_eq!(stamp(dir.path()), before);

        // a change the caller refuses (after a set) writes nothing either
        let r = update(dir.path(), |s: &mut HealSettings| {
            s.set(Setting::Autonomy(Autonomy::Observe))
                .map_err(|_| "set")?;
            Err::<(), _>("refused")
        });
        assert!(matches!(r, Err(UpdateError::Change("refused"))));
        assert_eq!(stamp(dir.path()), before);
        assert_eq!(listing(dir.path()), names);
        assert_eq!(
            load(dir.path()).unwrap().settings.autonomy(),
            Autonomy::Diagnose
        );
    }

    #[test]
    fn the_first_change_writes_that_key_only_0600_and_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let (s, wrote) = update(dir.path(), set_one(Setting::Autonomy(Autonomy::Observe))).unwrap();
        assert!(wrote);
        assert_eq!(s.autonomy(), Autonomy::Observe);
        assert_eq!(
            std::fs::read_to_string(path(dir.path())).unwrap(),
            "{\"schema\":1,\"heal.autonomy\":\"observe\"}\n",
            "the keys never set stay absent and follow the release's defaults"
        );
        let m = std::fs::symlink_metadata(path(dir.path())).unwrap();
        assert!(m.file_type().is_file());
        assert_eq!(m.permissions().mode() & 0o777, 0o600);
        assert_eq!(listing(dir.path()), [HEAL_FILE], "no temp file left");
        // a new inode each write: written through a temp file and a rename, never in place
        let ino = m.ino();
        update(dir.path(), set_one(Setting::Notify(Notify::Transitions))).unwrap();
        assert_ne!(std::fs::metadata(path(dir.path())).unwrap().ino(), ino);
        assert_eq!(
            load(dir.path()).unwrap().settings.notify(),
            Notify::Transitions
        );
    }

    #[test]
    fn each_consent_hash_is_stored_beside_its_key() {
        assert_eq!(Key::AskHub.consent(), Some("heal_delegate"));
        assert_eq!(Key::ServeMembers.consent(), Some("heal_serve"));
        for k in Key::ALL {
            match k.consent_name() {
                Some(n) => assert_eq!(n, format!("{}{CONSENT_SUFFIX}", k.name())),
                None => assert_eq!(k.consent(), None),
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (hash('a'), hash('b'));
        update(dir.path(), |s: &mut HealSettings| {
            s.set(Setting::AskHub(true))?;
            s.set_consent_hash(Key::AskHub, Some(&a))?;
            s.set(Setting::ServeMembers(true))?;
            s.set_consent_hash(Key::ServeMembers, Some(&b))?;
            Ok::<(), SettingsError>(())
        })
        .unwrap();
        let text = std::fs::read_to_string(path(dir.path())).unwrap();
        assert!(
            text.contains(&format!(
                "\"heal.ask_hub\":true,\"heal.ask_hub.consent_hash\":\"{a}\""
            )),
            "beside its key: {text}"
        );
        assert!(text.contains(&format!(
            "\"heal.serve_members\":true,\"heal.serve_members.consent_hash\":\"{b}\""
        )));
        let s = load(dir.path()).unwrap().settings;
        assert_eq!(s.consent_hash(Key::AskHub).as_deref(), Some(a.as_str()));
        assert!(s.consent_holds(Key::AskHub, &a));
        assert!(
            !s.consent_holds(Key::AskHub, &b),
            "a changed text asks again"
        );
        assert!(!s.consent_holds(Key::Autonomy, &a));
        let view = s.view(&owed()).to_string();
        assert!(
            !view.contains(&a) && !view.contains(&b),
            "no view carries a consent hash (§17.3)"
        );

        // the refusals, each changing nothing
        let mut t = s.clone();
        for k in [Key::Autonomy, Key::FixesSend, Key::CodeDrafts, Key::Notify] {
            assert_eq!(
                t.set_consent_hash(k, Some(&a)),
                Err(SettingsError::NoConsent(k))
            );
        }
        let upper = a.to_uppercase();
        for bad in ["ab", upper.as_str(), &a[1..], "g".repeat(64).as_str()] {
            assert_eq!(
                t.set_consent_hash(Key::AskHub, Some(bad)),
                Err(SettingsError::BadHash)
            );
        }
        assert_eq!(t, s);

        // cleared: the entry goes; a stored hash that is not hex reads as none
        let (_, wrote) = update(dir.path(), |s: &mut HealSettings| {
            s.set_consent_hash(Key::AskHub, None).map(|_| ())
        })
        .unwrap();
        assert!(wrote);
        assert!(read_raw(dir.path())
            .get("heal.ask_hub.consent_hash")
            .is_none());
        write_raw(
            dir.path(),
            json!({"schema": 1, "heal.serve_members.consent_hash": "not hex"}).to_string(),
        );
        assert_eq!(
            load(dir.path())
                .unwrap()
                .settings
                .consent_hash(Key::ServeMembers),
            None
        );
    }

    #[test]
    fn an_invalid_file_reads_the_defaults_and_moves_aside_at_the_first_change() {
        let too_large = {
            let mut t = String::from("{\"schema\":1,\"heal.pad\":\"");
            t.push_str(&"x".repeat(MAX_FILE as usize));
            t.push_str("\"}");
            t.into_bytes()
        };
        let cases: Vec<(&str, Vec<u8>)> = vec![
            (
                "truncated",
                b"{\"schema\":1,\"heal.autonomy\":\"observe\"".to_vec(),
            ),
            ("syntax", b"heal.autonomy=observe".to_vec()),
            ("an array", b"[1]".to_vec()),
            ("no schema", b"{\"heal.autonomy\":\"observe\"}".to_vec()),
            (
                "schema 2",
                b"{\"schema\":2,\"heal.autonomy\":\"observe\"}".to_vec(),
            ),
            (
                "schema \"1\"",
                b"{\"schema\":\"1\",\"heal.autonomy\":\"observe\"}".to_vec(),
            ),
            (
                "not UTF-8",
                b"{\"schema\":1,\"heal.autonomy\":\"\xff\"}".to_vec(),
            ),
            ("too large", too_large),
            ("empty", Vec::new()),
        ];
        for (what, bytes) in cases {
            let dir = tempfile::tempdir().unwrap();
            write_raw(dir.path(), &bytes);
            let l = load(dir.path()).unwrap();
            let why = l.invalid.unwrap_or_else(|| panic!("{what}: read as valid"));
            assert!(
                !why.contains("observe") && !why.contains("xxx"),
                "{what}: {why}"
            );
            assert_eq!(l.settings, HealSettings::default(), "{what}");
            // neither the read nor a change of nothing touches it
            let (_, wrote) =
                update(dir.path(), set_one(Setting::Autonomy(Autonomy::Propose))).unwrap();
            assert!(!wrote, "{what}");
            assert_eq!(std::fs::read(path(dir.path())).unwrap(), bytes, "{what}");
            // the first change moves it aside, then writes
            let (_, wrote) =
                update(dir.path(), set_one(Setting::Autonomy(Autonomy::Observe))).unwrap();
            assert!(wrote, "{what}");
            let aside = dir.path().join(format!("{HEAL_FILE}.invalid-1"));
            assert_eq!(std::fs::read(&aside).unwrap(), bytes, "{what}: kept aside");
            let l = load(dir.path()).unwrap();
            assert_eq!(l.invalid, None, "{what}");
            assert_eq!(l.settings.autonomy(), Autonomy::Observe, "{what}");
        }

        // a symlink: never followed, moved aside as itself, its target untouched
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("elsewhere.json");
        let target_text = "{\"schema\":1,\"heal.autonomy\":\"observe\"}";
        std::fs::write(&target, target_text).unwrap();
        std::os::unix::fs::symlink(&target, path(dir.path())).unwrap();
        let l = load(dir.path()).unwrap();
        assert_eq!(l.invalid.as_deref(), Some(NOT_SMALL_FILE));
        assert_eq!(l.settings.autonomy(), Autonomy::Propose, "not followed");
        update(dir.path(), set_one(Setting::Notify(Notify::Transitions))).unwrap();
        let aside = dir.path().join(format!("{HEAL_FILE}.invalid-1"));
        assert!(std::fs::symlink_metadata(&aside)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(std::fs::symlink_metadata(path(dir.path()))
            .unwrap()
            .file_type()
            .is_file());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), target_text);

        // a directory, and a FIFO (the read never blocks on it)
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(path(dir.path())).unwrap();
        assert_eq!(
            load(dir.path()).unwrap().invalid.as_deref(),
            Some(NOT_SMALL_FILE)
        );
        let (_, wrote) = update(dir.path(), set_one(Setting::CodeDrafts(true))).unwrap();
        assert!(wrote);
        assert!(dir.path().join(format!("{HEAL_FILE}.invalid-1")).is_dir());
        assert!(load(dir.path()).unwrap().settings.code_drafts(&owed()));

        let dir = tempfile::tempdir().unwrap();
        let fifo =
            std::ffi::CString::new(path(dir.path()).into_os_string().into_encoded_bytes()).unwrap();
        // SAFETY: a valid NUL-terminated path; mkfifo reads it only.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert_eq!(
            load(dir.path()).unwrap().invalid.as_deref(),
            Some(NOT_SMALL_FILE)
        );

        // a socket, whose open fails (ENXIO): not a regular file, so invalid, never unreadable
        let dir = tempfile::tempdir().unwrap();
        let socket = std::os::unix::net::UnixListener::bind(path(dir.path())).unwrap();
        assert_eq!(
            load(dir.path()).unwrap().invalid.as_deref(),
            Some(NOT_SMALL_FILE)
        );
        let (_, wrote) = update(dir.path(), set_one(Setting::Notify(Notify::Transitions))).unwrap();
        assert!(wrote);
        let aside = dir.path().join(format!("{HEAL_FILE}.invalid-1"));
        assert!(std::fs::symlink_metadata(aside)
            .unwrap()
            .file_type()
            .is_socket());
        drop(socket);

        // a second invalid file takes the next free name
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(format!("{HEAL_FILE}.invalid-1")), "old").unwrap();
        write_raw(dir.path(), "nope");
        update(dir.path(), set_one(Setting::AskHub(true))).unwrap();
        let second = dir.path().join(format!("{HEAL_FILE}.invalid-2"));
        assert_eq!(std::fs::read(second).unwrap(), b"nope");
    }

    #[test]
    fn the_handle_reads_once_at_start_and_once_per_route_call() {
        let dir = tempfile::tempdir().unwrap();
        write_raw(dir.path(), "{\"schema\":1,\"heal.autonomy\":\"diagnose\"}");
        let r0 = reads();
        let (h, invalid) = Handle::open(dir.path());
        assert_eq!(invalid, None);
        assert_eq!(reads() - r0, 1, "the one read at start");
        for _ in 0..100 {
            assert_eq!(h.autonomy(), Autonomy::Diagnose);
            assert_eq!(h.current().autonomy(), Autonomy::Diagnose);
        }
        assert_eq!(reads() - r0, 1, "the copy answers: no read");

        // a hand edit is seen at the next route call, not before
        write_raw(dir.path(), "{\"schema\":1,\"heal.autonomy\":\"observe\"}");
        assert_eq!(h.autonomy(), Autonomy::Diagnose);
        let l = h.reload().unwrap();
        assert_eq!(reads() - r0, 2, "one read per route call");
        assert_eq!(l.settings.autonomy(), Autonomy::Observe);
        assert_eq!(h.autonomy(), Autonomy::Observe);

        // a PATCH: one read, the write, and every clone holds the result
        let h2 = h.clone();
        let (s, wrote) = h2
            .update(set_one(Setting::Autonomy(Autonomy::Propose)))
            .unwrap();
        assert!(wrote);
        assert_eq!(reads() - r0, 3, "one read per route call");
        assert_eq!(s.autonomy(), Autonomy::Propose);
        assert_eq!(h.autonomy(), Autonomy::Propose);
        assert_eq!(h.current(), load(dir.path()).unwrap().settings);

        // a refused change keeps the copy as it was
        let r = h.update(|_: &mut HealSettings| Err::<(), _>("no"));
        assert!(matches!(r, Err(UpdateError::Change("no"))));
        assert_eq!(h.autonomy(), Autonomy::Propose);

        // an invalid file at start: every default and why, nothing moved
        let dir = tempfile::tempdir().unwrap();
        write_raw(dir.path(), "{");
        let (h, invalid) = Handle::open(dir.path());
        assert!(invalid.is_some());
        assert_eq!(h.current(), HealSettings::default());
        assert_eq!(listing(dir.path()), [HEAL_FILE]);
    }

    #[test]
    fn the_largest_file_v1_writes_fits_one_kib() {
        let mut s = HealSettings::default();
        // the longest word of each key; each switch turned on and off again ("false" is longer
        // than "true"), its consent hash left in place; every sender
        s.set(Setting::Autonomy(Autonomy::Diagnose)).unwrap();
        s.set(Setting::FixesReceive(FixesReceive::NamedContacts))
            .unwrap();
        let all: Vec<NodeId> = (1..=MAX_SENDERS as u8).map(node).collect();
        s.set(Setting::FixesSenders(all.clone())).unwrap();
        s.set(Setting::FixesSend(FixesSend::Contacts)).unwrap();
        for on in [true, false] {
            s.set(Setting::AskHub(on)).unwrap();
            s.set(Setting::ServeMembers(on)).unwrap();
            s.set(Setting::CodeDrafts(on)).unwrap();
        }
        s.set_consent_hash(Key::AskHub, Some(&hash('a'))).unwrap();
        s.set_consent_hash(Key::ServeMembers, Some(&hash('b')))
            .unwrap();
        s.set(Setting::Notify(Notify::Transitions)).unwrap();
        let bytes = encode(&s).unwrap();
        assert_eq!(bytes.len(), 963);
        assert!(bytes.len() <= 1024, "§4's path table: ≤ 1 KiB");
        assert_eq!(decode(&bytes).unwrap(), s, "read back as written");
        assert_eq!(s.fixes_senders(), all);
    }

    #[test]
    fn fix_senders_are_contact_node_ids() {
        let k = Key::FixesSenders;
        assert_eq!(
            Setting::from_json(k, &json!([hex_id(0xab), hex_id(2), hex_id(0xab)])),
            Ok(Setting::FixesSenders(vec![node(0xab), node(2)])),
            "first occurrences, in order"
        );
        for bad in [
            json!([hex_id(0xab).to_uppercase()]),
            json!([&hex_id(1)[2..]]),
            json!([format!("{}0", hex_id(1))]),
            json!([1]),
            json!([null]),
            json!(hex_id(1)),
            json!({}),
            json!(null),
        ] {
            assert_eq!(
                Setting::from_json(k, &bad),
                Err(SettingsError::BadValue(k)),
                "{bad}"
            );
        }
        let nine: Vec<String> = (0..=MAX_SENDERS as u8).map(hex_id).collect();
        assert_eq!(
            Setting::from_json(k, &json!(nine)),
            Err(SettingsError::TooManySenders)
        );
        // the length is checked before an entry is parsed
        let nine_junk = vec![json!(0); MAX_SENDERS + 1];
        assert_eq!(
            Setting::from_json(k, &json!(nine_junk)),
            Err(SettingsError::TooManySenders)
        );

        let mut s = HealSettings::default();
        assert_eq!(
            s.set(Setting::FixesSenders(
                (0..=MAX_SENDERS as u8).map(node).collect()
            )),
            Err(SettingsError::TooManySenders)
        );
        assert_eq!(
            s.set(Setting::FixesSenders(vec![node(3), node(3), node(4)])),
            Ok(true)
        );
        assert_eq!(s.fixes_senders(), [node(3), node(4)]);
        assert_eq!(
            s.view(&Consents::default())["heal.fixes.senders"],
            json!([hex_id(3), hex_id(4)])
        );
        assert_eq!(
            s.set(Setting::FixesSenders(vec![node(3), node(4)])),
            Ok(false)
        );
        // a stored list with a ninth entry reads as the default
        let dir = tempfile::tempdir().unwrap();
        write_raw(
            dir.path(),
            json!({"schema": 1, "heal.fixes.senders": nine}).to_string(),
        );
        assert_eq!(
            load(dir.path()).unwrap().settings.fixes_senders(),
            Vec::<NodeId>::new()
        );
    }

    #[test]
    fn the_ladder_has_three_rungs_and_no_auto() {
        assert_eq!(
            Autonomy::ALL.map(Autonomy::as_str),
            ["observe", "diagnose", "propose"]
        );
        assert_eq!(Autonomy::parse("auto"), None);
        assert_eq!(Autonomy::default(), Autonomy::Propose);
        assert!(!Autonomy::Observe.diagnoses());
        assert!(Autonomy::Diagnose.diagnoses() && Autonomy::Propose.diagnoses());
        assert!(!Autonomy::Observe.proposes() && !Autonomy::Diagnose.proposes());
        assert!(Autonomy::Propose.proposes());

        // every name and every word round-trips; nothing else is a key or a value
        for k in Key::ALL {
            assert_eq!(Key::from_name(k.name()), Some(k));
            assert_eq!(Setting::default_of(k).key(), k);
        }
        for other in [
            "schema",
            "accept_warning",
            "heal.auto",
            "heal.ask_hub.consent_hash",
            "",
        ] {
            assert_eq!(Key::from_name(other), None, "{other}");
        }
        assert!(Autonomy::ALL
            .iter()
            .all(|v| Autonomy::parse(v.as_str()) == Some(*v)));
        assert!(FixesReceive::ALL
            .iter()
            .all(|v| FixesReceive::parse(v.as_str()) == Some(*v)));
        assert!(FixesSend::ALL
            .iter()
            .all(|v| FixesSend::parse(v.as_str()) == Some(*v)));
        assert!(Notify::ALL
            .iter()
            .all(|v| Notify::parse(v.as_str()) == Some(*v)));
        for k in Key::ALL {
            let v = Setting::default_of(k);
            assert_eq!(Setting::from_json(k, &v.to_json()), Ok(v), "{}", k.name());
        }
        assert_eq!(
            SettingsError::BadValue(Key::Autonomy).to_string(),
            "heal.autonomy takes observe, diagnose or propose"
        );
    }

    #[test]
    fn a_file_that_cannot_be_read_is_never_replaced_and_the_held_copy_stays() {
        let dir = tempfile::tempdir().unwrap();
        // a human chose the narrow values and a consent: none of them may go
        update(dir.path(), |s: &mut HealSettings| {
            s.set(Setting::Autonomy(Autonomy::Observe))?;
            s.set(Setting::FixesReceive(FixesReceive::Off))?;
            s.set(Setting::AskHub(true))?;
            s.set_consent_hash(Key::AskHub, Some(OWED_A))?;
            Ok::<(), SettingsError>(())
        })
        .unwrap();
        let bytes = std::fs::read(path(dir.path())).unwrap();
        let chosen = |s: &HealSettings| {
            s.autonomy() == Autonomy::Observe
                && s.fixes_receive() == FixesReceive::Off
                && s.ask_hub(&owed())
        };
        let (h, why) = Handle::open(dir.path());
        assert_eq!(why, None);
        let held = h.current();
        assert!(chosen(&held));

        for errno in [
            libc::EMFILE,
            libc::ENFILE,
            libc::EACCES,
            libc::EIO,
            libc::EINTR,
        ] {
            fail_opens(Some(errno));
            let want = ReadError::from(std::io::Error::from_raw_os_error(errno));
            assert_eq!(load(dir.path()), Err(want.clone()), "errno {errno}");
            // a route's read answers the error, and the copy held stays
            assert_eq!(h.reload(), Err(want.clone()), "errno {errno}");
            assert_eq!(h.current(), held, "errno {errno}: the copy held stays");
            // a route's write, and the free one: nothing moved, nothing written
            let r = h.update(set_one(Setting::Notify(Notify::Transitions)));
            assert!(
                matches!(&r, Err(UpdateError::Read(e)) if *e == want),
                "errno {errno}: {r:?}"
            );
            let r = update(dir.path(), set_one(Setting::Notify(Notify::Transitions)));
            assert!(
                matches!(&r, Err(UpdateError::Read(e)) if *e == want),
                "errno {errno}: {r:?}"
            );
            assert_eq!(h.current(), held, "errno {errno}");
            assert_eq!(
                std::fs::read(path(dir.path())).unwrap(),
                bytes,
                "errno {errno}"
            );
            assert_eq!(
                listing(dir.path()),
                [HEAL_FILE],
                "errno {errno}: nothing moved aside"
            );
            // at start nothing is held yet: every default and why, nothing moved
            let (fresh, why) = Handle::open(dir.path());
            assert_eq!(fresh.current(), HealSettings::default(), "errno {errno}");
            assert_eq!(why, Some(want.to_string()), "errno {errno}");
            assert_eq!(listing(dir.path()), [HEAL_FILE], "errno {errno}");
        }
        let e = ReadError::from(std::io::Error::from_raw_os_error(libc::EACCES)).to_string();
        assert!(e.starts_with("cannot read heal.json: "), "{e}");

        // read again, the next change builds on the file: every choice and the consent kept
        fail_opens(None);
        let (s, wrote) = h
            .update(set_one(Setting::Notify(Notify::Transitions)))
            .unwrap();
        assert!(wrote);
        assert_eq!(s.notify(), Notify::Transitions);
        assert!(chosen(&s) && chosen(&h.current()));
        assert!(chosen(&load(dir.path()).unwrap().settings));
        assert_eq!(listing(dir.path()), [HEAL_FILE]);
    }

    #[test]
    fn an_invalid_file_that_cannot_be_moved_aside_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        for n in 1..=MOVE_ASIDE_TRIES {
            std::fs::write(dir.path().join(format!("{HEAL_FILE}.invalid-{n}")), "").unwrap();
        }
        // a later release's schema: invalid here, its consents kept only while the file is
        let later = b"{\"schema\":2,\"heal.later.consent_hash\":\"cc\"}";
        write_raw(dir.path(), later);
        assert_eq!(
            load(dir.path()).unwrap().invalid.as_deref(),
            Some("schema is not 1")
        );
        let r = update(dir.path(), set_one(Setting::Notify(Notify::Transitions)));
        assert!(matches!(&r, Err(UpdateError::MoveAside(_))), "{r:?}");
        let e = r.unwrap_err().to_string();
        assert!(
            e.ends_with("no free heal.json.invalid-<n> name; nothing written"),
            "{e}"
        );
        assert_eq!(std::fs::read(path(dir.path())).unwrap(), later);
        assert_eq!(
            listing(dir.path()).len(),
            1 + MOVE_ASIDE_TRIES as usize,
            "no temp file left"
        );

        // a free name lets the change through, the file kept under it
        let free = dir.path().join(format!("{HEAL_FILE}.invalid-7"));
        std::fs::remove_file(&free).unwrap();
        let (_, wrote) = update(dir.path(), set_one(Setting::Notify(Notify::Transitions))).unwrap();
        assert!(wrote);
        assert_eq!(std::fs::read(free).unwrap(), later);
    }

    #[test]
    fn a_consent_gated_switch_reads_off_without_its_consent() {
        // the three switches as the node answers them; the view, `get` and each accessor agree
        let answer = |s: &HealSettings, c: &Consents<'_>| -> [bool; 3] {
            let view = s.view(c);
            let each = [s.ask_hub(c), s.serve_members(c), s.code_drafts(c)];
            for (k, on) in [Key::AskHub, Key::ServeMembers, Key::CodeDrafts]
                .into_iter()
                .zip(each)
            {
                assert_eq!(view[k.name()], json!(on), "{}", k.name());
                assert_eq!(s.get(k, c), Setting::from_json(k, &json!(on)).unwrap());
            }
            each
        };

        // a hand-edited or restored file: every switch on, no hash beside either
        let dir = tempfile::tempdir().unwrap();
        write_raw(
            dir.path(),
            json!({
                "schema": 1,
                "heal.ask_hub": true,
                "heal.serve_members": true,
                "heal.code_drafts": true
            })
            .to_string(),
        );
        let s = load(dir.path()).unwrap().settings;
        assert_eq!(
            answer(&s, &owed()),
            [false, false, true],
            "no consent recorded: off"
        );

        // each consent recorded as owed now: on
        let (s, wrote) = update(dir.path(), |s: &mut HealSettings| {
            s.set_consent_hash(Key::AskHub, Some(OWED_A))?;
            s.set_consent_hash(Key::ServeMembers, Some(OWED_B))?;
            Ok::<(), SettingsError>(())
        })
        .unwrap();
        assert!(wrote);
        assert_eq!(answer(&s, &owed()), [true, true, true]);

        // a changed text (another hash owed now) asks again: off, the old hash still on file
        let changed = Consents {
            heal_delegate: Some(OWED_B),
            heal_serve: Some(OWED_A),
            opencode: true,
        };
        assert_eq!(answer(&s, &changed), [false, false, true]);
        assert_eq!(s.consent_hash(Key::AskHub).as_deref(), Some(OWED_A));
        // not offered now (no home hub, not a hub, the component off): off
        assert_eq!(answer(&s, &Consents::default()), [false, false, false]);
        // the handle's copy answers the same
        let (h, _) = Handle::open(dir.path());
        assert_eq!(answer(&h.current(), &owed()), [true, true, true]);
        assert_eq!(answer(&h.current(), &changed), [false, false, true]);

        // a recorded hash that is not 64 lowercase hex holds nothing, even owed as written
        let (upper, short) = (OWED_A.to_uppercase(), &OWED_B[1..]);
        write_raw(
            dir.path(),
            json!({
                "schema": 1,
                "heal.ask_hub": true,
                "heal.ask_hub.consent_hash": upper,
                "heal.serve_members": true,
                "heal.serve_members.consent_hash": short
            })
            .to_string(),
        );
        let s = load(dir.path()).unwrap().settings;
        let as_written = Consents {
            heal_delegate: Some(&upper),
            heal_serve: Some(short),
            opencode: false,
        };
        assert_eq!(answer(&s, &as_written), [false, false, false]);

        // stored off with each consent recorded: off, whatever is owed
        let mut t = HealSettings::default();
        t.set_consent_hash(Key::AskHub, Some(OWED_A)).unwrap();
        t.set_consent_hash(Key::ServeMembers, Some(OWED_B)).unwrap();
        assert_eq!(answer(&t, &owed()), [false, false, false]);
    }

    #[test]
    fn an_unknown_value_is_written_back_byte_for_byte() {
        // values a `serde_json::Value` would round, reformat, collapse or refuse, as a later
        // release may write them (one under a key v1 knows), in no particular order
        let later = concat!(
            "{\"heal.later.neg0\":-0,\"schema\":1,\"heal.autonomy\":1E400,",
            "\"heal.later.big\":18446744073709551616,\"heal.later.dup\":1,",
            "\"heal.later.deep\":{ \"a\" : [ 1 ,\n 2 ] },\"heal.later.exp\":1e2,",
            "\"heal.later.long\":0.1000000000000000055511151231257827021181583404541015625,",
            "\"heal.later.text\":\"caf\\u00e9\\n\",\"heal.later.dup\":2}"
        );
        let dir = tempfile::tempdir().unwrap();
        write_raw(dir.path(), later);
        let l = load(dir.path()).unwrap();
        assert_eq!(
            l.invalid, None,
            "a later release's number leaves the file valid"
        );
        assert_eq!(
            l.settings.autonomy(),
            Autonomy::Propose,
            "a value v1 cannot read reads the default"
        );

        // a change of another key writes each back byte for byte, in key order; a key given
        // twice keeps its last occurrence, as serde_json (so a later release) reads it
        update(dir.path(), set_one(Setting::Notify(Notify::Transitions))).unwrap();
        let want = concat!(
            "{\"schema\":1,\"heal.autonomy\":1E400,\"heal.later.big\":18446744073709551616,",
            "\"heal.later.deep\":{ \"a\" : [ 1 ,\n 2 ] },\"heal.later.dup\":2,",
            "\"heal.later.exp\":1e2,",
            "\"heal.later.long\":0.1000000000000000055511151231257827021181583404541015625,",
            "\"heal.later.neg0\":-0,\"heal.later.text\":\"caf\\u00e9\\n\",",
            "\"heal.notify\":\"transitions\"}\n"
        );
        assert_eq!(std::fs::read_to_string(path(dir.path())).unwrap(), want);
        assert_eq!(load(dir.path()).unwrap().invalid, None);

        // a human's value replaces the one v1 could not read; the others stay as they were
        update(dir.path(), set_one(Setting::Autonomy(Autonomy::Observe))).unwrap();
        assert_eq!(
            std::fs::read_to_string(path(dir.path())).unwrap(),
            want.replace("\"heal.autonomy\":1E400", "\"heal.autonomy\":\"observe\"")
        );
    }
}
