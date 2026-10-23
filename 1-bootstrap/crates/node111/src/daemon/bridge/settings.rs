//! The bridge core's part of `state/bridge.json` (Layer 6 `6-transport/README.md` §2
//! "State", §5 "HAM and MQTT: live consent", §6 "Send to Reticulum", §10 `/bridge/settings`,
//! §15.3): HAM confirmations, per-channel send switches, recipient lists, the LXMF announce
//! name and interval, compiled-in adapters on/off. The file and its fail-closed rules are
//! `components::consent`'s (S4); these sections are typed fields of its `BridgeFile`, read
//! once at start ([`Sections::load`]) and rewritten through `consent::update_file`.
//!
//! - **Text + hash, never a bare switch.** Confirming readable sends through a licensed
//!   account ([`ham_text`]) and turning send on for an uplinked channel ([`uplink_text`])
//!   both answer `409 consent_required` with the exact text and its lowercase hex SHA-256
//!   ([`sha256_hex`]) until the client echoes that hash; the record keeps the hash. A gate
//!   passes only while the recorded hash equals the hash of the text as it reads **now**, so
//!   a changed text asks again. Texts name accounts and channels by their native ids (stable
//!   for the conversation's life), never by a renameable title.
//! - **The gates** ([`Bridge::gate_for`], read live per send and per relay copy): a licensed
//!   account needs its confirmation (`ham_unconfirmed`); an uplinked channel needs its send
//!   switch on with the uplink text confirmed, and a switch turned off gates every send on
//!   that channel, relay copies included (`uplink_off`). A channel that is not uplinked is on
//!   by default.
//! - **Recipient lists** (§6: destination-only, one DM per member): each list is a `group`
//!   conversation `list:<id>` under the account `local`, created when the list is saved;
//!   members are validated against the network's recipient syntax (none ⇒ refused).
//! - **A patch is all or nothing**: every entry is validated (and every consent gate
//!   checked) before anything is written.
//! - **Per-contact bridging for app peers** (§7 (2), §15.3; C5, additive): off for every
//!   contact until a human turns it on for that one with the hash of [`contact_text`]
//!   ([`Bridge::set_contact_bridging`]); the app-peer adapter reads the switch live per
//!   received message ([`Bridge::contact_bridged`]) and copies only those contacts' texts
//!   into Messages. The agent can neither propose nor confirm it (§15.3).

use super::core::{compiled_in, is_list_conv, Bridge, BridgeError, LIST_ACCOUNT, LIST_PREFIX};
use super::{ingest, AdapterFacts, ConvKind};
use crate::daemon::components::consent::{
    self, AnnounceRecord, ChannelRecord, ConsentRequired, ContactRecord, HamRecord,
    RecipientListRecord,
};
use crate::daemon::fabric::json::{self as fjson, ConvLive};
use mesh111::store::{ForeignConversationRow, ForeignKind};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// The announce interval when none was set (§6 (3): 60 minutes; `0` = events only).
pub const DEFAULT_ANNOUNCE_MIN: u32 = 60;
/// Members of one recipient list at most (the contract's `maxItems`).
pub const MAX_MEMBERS: usize = 256;

/// Lowercase hex SHA-256 of the exact UTF-8 bytes of `s` (the consent hash).
pub fn sha256_hex(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    let mut out = String::with_capacity(64);
    for b in d {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// The licensed-mode confirmation text for `account` on the network labelled `label`.
pub fn ham_text(label: &str, account: &str) -> String {
    format!(
        "The {label} account {account} is in amateur-licensed mode. Readable sends and relays \
         through it follow the licensee's rules: its content and any third-party traffic are \
         the licensee's fact. 111 never sends sealed frames through it."
    )
}

/// The uplinked-channel send confirmation text (§5: "this channel is published to the
/// internet by your board").
pub fn uplink_text(label: &str, native_id: &str, uplink_copy: &str) -> String {
    format!(
        "{label} channel {native_id}: {uplink_copy}. What you send or relay on it is \
         published there too."
    )
}

/// The per-contact bridging warning of an app peer (§7 (2), verbatim after the subject:
/// the network label and the contact's native id, stable for the contact's life).
pub fn contact_text(label: &str, contact: &str) -> String {
    format!(
        "{label} contact {contact}: This hub decrypts this contact's messages to bridge them. \
         What you relay onto another network is readable there."
    )
}

/// App-peer contacts with bridging on, per network (more is refused).
pub const MAX_BRIDGED_CONTACTS: usize = 256;

/// The bridge-core sections as loaded (module doc).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sections {
    pub adapters: BTreeMap<String, bool>,
    pub ham: Vec<HamRecord>,
    pub channels: Vec<ChannelRecord>,
    pub lists: Vec<RecipientListRecord>,
    pub announce: Option<AnnounceRecord>,
    /// App-peer contacts with bridging on (C5).
    pub contacts: Vec<ContactRecord>,
}

impl Sections {
    /// Read the sections (an unreadable or invalid file reads as empty, fail closed).
    pub fn load(root: &Path) -> Self {
        let f = consent::load(root);
        Sections {
            adapters: f.adapters,
            ham: f.ham,
            channels: f.channels,
            lists: f.recipient_lists,
            announce: f.announce,
            contacts: f.contacts,
        }
    }

    /// Bridging is on for `contact` of `account` on `network`: a record whose hash is
    /// the hash of `text` as it reads now (a changed text asks again).
    pub fn contact_bridged(&self, network: &str, account: &str, contact: &str, text: &str) -> bool {
        let h = sha256_hex(text);
        self.contacts.iter().any(|r| {
            r.network == network && r.account == account && r.contact == contact && r.text_hash == h
        })
    }

    /// A human confirmed `text` for `account` (its hash is the recorded one).
    pub fn ham_confirmed(&self, network: &str, account: &str, text: &str) -> bool {
        let h = sha256_hex(text);
        self.ham
            .iter()
            .any(|r| r.network == network && r.account == account && r.text_hash == h)
    }

    /// The send switch record of a channel.
    pub fn channel(&self, network: &str, account: &str, native_id: &str) -> Option<&ChannelRecord> {
        self.channels
            .iter()
            .find(|r| r.network == network && r.account == account && r.native_id == native_id)
    }

    /// Whether sends on a channel are allowed now: a record decides (an uplinked channel
    /// needs the current uplink text's hash), otherwise on unless uplinked.
    pub fn send_allowed(
        &self,
        network: &str,
        account: &str,
        native_id: &str,
        uplink: bool,
        uplink_text: &str,
    ) -> bool {
        match self.channel(network, account, native_id) {
            Some(r) => {
                r.send && (!uplink || r.text_hash.as_deref() == Some(&sha256_hex(uplink_text)))
            }
            None => !uplink,
        }
    }

    /// The recipient list `id` of `network`.
    pub fn list(&self, network: &str, id: &str) -> Option<&RecipientListRecord> {
        self.lists
            .iter()
            .find(|l| l.network == network && l.id == id)
    }

    /// The announce settings (defaults when none was saved).
    pub fn announce(&self) -> AnnounceRecord {
        self.announce.clone().unwrap_or(AnnounceRecord {
            name: None,
            interval_min: DEFAULT_ANNOUNCE_MIN,
        })
    }
}

/// A consent gate that holds a send (§5, §9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    /// `ham_unconfirmed`: the licensed account's confirmation is missing.
    Ham(ConsentRequired),
    /// `uplink_off`: the channel's send switch is off (with the text to confirm when the
    /// channel is uplinked; `None` when the user switched it off).
    Uplink(Option<ConsentRequired>),
}

impl Gate {
    /// The §4 reason code.
    pub fn reason(&self) -> &'static str {
        match self {
            Gate::Ham(_) => "ham_unconfirmed",
            Gate::Uplink(_) => "uplink_off",
        }
    }

    /// The consent round-trip body, when there is one.
    pub fn consent(&self) -> Option<&ConsentRequired> {
        match self {
            Gate::Ham(c) => Some(c),
            Gate::Uplink(c) => c.as_ref(),
        }
    }

    /// The plain sentence of a plan refusal.
    pub fn detail(&self) -> String {
        match self {
            Gate::Ham(_) => "the licensed account's confirmation is missing".into(),
            Gate::Uplink(Some(_)) => {
                "this channel is published to the internet and sending on it is off".into()
            }
            Gate::Uplink(None) => "sending on this channel is switched off".into(),
        }
    }
}

// ---------------------------------------------------------------- patch bodies

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

/// The contract's `RecipientList`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipientListIn {
    pub id: String,
    pub network: String,
    pub name: String,
    pub members: Vec<String>,
}

/// The contract's `BridgeChannelPatch`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelPatch {
    pub conv: String,
    pub send: bool,
    #[serde(default)]
    pub accept_warning: Option<String>,
}

/// The contract's `BridgeHamPatch`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HamPatch {
    pub account: String,
    pub confirmed: bool,
    #[serde(default)]
    pub accept_warning: Option<String>,
}

/// The contract's `BridgeAnnouncePatch` (`name: null` resets to the default).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnouncePatch {
    #[serde(default, deserialize_with = "present")]
    pub name: Option<Option<String>>,
    #[serde(default)]
    pub interval_min: Option<u32>,
}

/// The contract's `BridgeSettingsPatch`. `order` is `interfaces.toml`'s (B4): this core
/// refuses it, the route hands it to the transport spine.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    #[serde(default)]
    pub order: Option<Vec<String>>,
    #[serde(default)]
    pub adapters: Option<BTreeMap<String, bool>>,
    #[serde(default)]
    pub recipient_lists: Option<Vec<RecipientListIn>>,
    #[serde(default)]
    pub channels: Vec<ChannelPatch>,
    #[serde(default)]
    pub ham: Vec<HamPatch>,
    #[serde(default)]
    pub announce: Option<AnnouncePatch>,
}

fn list_id_ok(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl Bridge {
    /// The network's uplink copy (manifest), or the spec's sentence.
    fn uplink_copy(&self, network: &str) -> String {
        self.manifest(network)
            .and_then(|m| m.uplink_warning.clone())
            .unwrap_or_else(|| "this channel is published to the internet by your board".into())
    }

    /// The uplink text of a channel conversation.
    pub(super) fn uplink_text_of(&self, c: &ForeignConversationRow) -> String {
        uplink_text(
            &self.label_of(&c.network),
            &c.native_id,
            &self.uplink_copy(&c.network),
        )
    }

    /// The consent gate holding a send into `c` now, if any (module doc; facts live).
    pub(super) fn gate_for(
        &self,
        c: &ForeignConversationRow,
        facts: &AdapterFacts,
    ) -> Option<Gate> {
        if is_list_conv(c) {
            return None;
        }
        let label = self.label_of(&c.network);
        let s = super::core::lock_sections(self);
        if facts.account(&c.account).is_some_and(|a| a.licensed) {
            let text = ham_text(&label, &c.account);
            if !s.ham_confirmed(&c.network, &c.account, &text) {
                return Some(Gate::Ham(ConsentRequired {
                    action: "ham_confirm",
                    subject: c.account.clone(),
                    hash: sha256_hex(&text),
                    warning_text: text,
                    addons: vec!["licensed_account".into()],
                }));
            }
        }
        if c.kind == ForeignKind::Channel {
            let uplink = facts
                .channel(&c.account, &c.native_id)
                .is_some_and(|ch| ch.uplink);
            let text = self.uplink_text_of(c);
            if !s.send_allowed(&c.network, &c.account, &c.native_id, uplink, &text) {
                return Some(Gate::Uplink(uplink.then(|| ConsentRequired {
                    action: "uplink_send",
                    subject: fjson::fc_id(c.id),
                    hash: sha256_hex(&text),
                    warning_text: text,
                    addons: vec!["uplinked_destination".into()],
                })));
            }
        }
        None
    }

    /// The live facts of a conversation the store does not hold (§10 `ConvLive`).
    pub(super) fn conv_live(
        &self,
        c: &ForeignConversationRow,
        facts: Option<&AdapterFacts>,
    ) -> ConvLive {
        if c.kind != ForeignKind::Channel {
            return ConvLive::default();
        }
        let uplink = facts.map(|f| {
            f.channel(&c.account, &c.native_id)
                .is_some_and(|ch| ch.uplink)
        });
        let text = self.uplink_text_of(c);
        let send = super::core::lock_sections(self).send_allowed(
            &c.network,
            &c.account,
            &c.native_id,
            uplink.unwrap_or(false),
            &text,
        );
        ConvLive {
            uplink,
            send_enabled: Some(send),
        }
    }

    /// The recipient list a conversation stands for (§6), if it is one.
    pub(super) fn list_of(&self, c: &ForeignConversationRow) -> Option<RecipientListRecord> {
        if !is_list_conv(c) {
            return None;
        }
        let id = c.native_id.strip_prefix(LIST_PREFIX)?;
        super::core::lock_sections(self)
            .list(&c.network, id)
            .cloned()
    }

    /// Write the sections through `consent::update_file` and refresh the cache, under the
    /// cache's lock (concurrent writers are serialised; a failed write changes nothing).
    fn write_sections(&self, f: impl FnOnce(&mut Sections)) -> Result<(), BridgeError> {
        let root = self.cfg.root.clone();
        let mut cache = super::core::lock_sections(self);
        let mut next = cache.clone();
        f(&mut next);
        let written = next.clone();
        consent::update_file(&root, move |file| {
            file.adapters = written.adapters;
            file.ham = written.ham;
            file.channels = written.channels;
            file.recipient_lists = written.lists;
            file.announce = written.announce;
            file.contacts = written.contacts;
            Ok(())
        })
        .map_err(|e| BridgeError::Io(format!("{e:#}")))?;
        *cache = next;
        Ok(())
    }

    /// The network of an account of this node (the first enabled network that has it).
    fn network_of_account(&self, account: &str) -> Option<(String, AdapterFacts)> {
        self.attached_all().into_iter().find_map(|(net, a)| {
            let f = a.adapter.facts();
            f.account(account).is_some().then_some((net, f))
        })
    }

    /// Validate one HAM entry (the consent gate included): the record to write, `None` to
    /// withdraw.
    fn ham_entry(
        &self,
        account: &str,
        confirmed: bool,
        accept: Option<&str>,
        by: &str,
    ) -> Result<(String, Option<HamRecord>), BridgeError> {
        let (network, _) = self
            .network_of_account(account)
            .ok_or(BridgeError::NotFound("account"))?;
        if !confirmed {
            return Ok((network, None));
        }
        let text = ham_text(&self.label_of(&network), account);
        let hash = sha256_hex(&text);
        if accept != Some(hash.as_str()) {
            return Err(BridgeError::Consent(ConsentRequired {
                action: "ham_confirm",
                subject: account.to_string(),
                warning_text: text,
                hash,
                addons: vec!["licensed_account".into()],
            }));
        }
        Ok((
            network.clone(),
            Some(HamRecord {
                network,
                account: account.to_string(),
                text_hash: hash,
                by: by.to_string(),
            }),
        ))
    }

    /// Validate one channel switch (the consent gate included).
    async fn channel_entry(
        &self,
        conv: i64,
        send: bool,
        accept: Option<&str>,
        by: &str,
    ) -> Result<ChannelRecord, BridgeError> {
        let c = self.conv_row(conv).await?;
        if c.kind != ForeignKind::Channel {
            return Err(BridgeError::BadRequest(
                "the send switch is per channel".into(),
            ));
        }
        let facts = self
            .facts_of(&c.network)
            .ok_or_else(|| BridgeError::Unavailable(format!("Enable {} first.", c.network)))?;
        let uplink = facts
            .channel(&c.account, &c.native_id)
            .is_some_and(|ch| ch.uplink);
        let mut rec = ChannelRecord {
            network: c.network.clone(),
            account: c.account.clone(),
            native_id: c.native_id.clone(),
            send,
            text_hash: None,
            by: by.to_string(),
        };
        if send && uplink {
            let text = self.uplink_text_of(&c);
            let hash = sha256_hex(&text);
            if accept != Some(hash.as_str()) {
                return Err(BridgeError::Consent(ConsentRequired {
                    action: "uplink_send",
                    subject: fjson::fc_id(c.id),
                    warning_text: text,
                    hash,
                    addons: vec!["uplinked_destination".into()],
                }));
            }
            rec.text_hash = Some(hash);
        }
        Ok(rec)
    }

    /// Validate a recipient list (module doc).
    fn list_entry(&self, l: &RecipientListIn) -> Result<RecipientListRecord, BridgeError> {
        if !list_id_ok(&l.id) {
            return Err(BridgeError::BadRequest(format!(
                "list id {:?} must be 1..=64 of A-Z a-z 0-9 _ -",
                l.id
            )));
        }
        if !self.known(&l.network) {
            return Err(BridgeError::NotFound("network"));
        }
        let kinds_have_group = match self.adapter(&l.network) {
            Some(a) => a.caps().kinds.contains(&ConvKind::Group),
            None => self
                .manifest(&l.network)
                .is_some_and(|m| m.kinds.contains(&ConvKind::Group)),
        };
        if !kinds_have_group {
            return Err(BridgeError::BadRequest(format!(
                "{} has no recipient lists",
                self.label_of(&l.network)
            )));
        }
        let pattern = self
            .manifest(&l.network)
            .and_then(|m| m.recipient.as_ref().map(|r| r.pattern.clone()))
            .or_else(|| {
                self.adapter(&l.network)
                    .and_then(|a| a.caps().recipient_syntax.map(|s| s.pattern))
            })
            .ok_or_else(|| {
                BridgeError::BadRequest("this network states no recipient syntax".into())
            })?;
        if l.members.len() > MAX_MEMBERS {
            return Err(BridgeError::BadRequest(format!(
                "at most {MAX_MEMBERS} members"
            )));
        }
        let mut members: Vec<String> = Vec::with_capacity(l.members.len());
        for m in &l.members {
            if !ingest::syntax_match(&pattern, m) {
                return Err(BridgeError::BadRequest(format!(
                    "{m:?} is not a {} address",
                    self.label_of(&l.network)
                )));
            }
            if !members.contains(m) {
                members.push(m.clone());
            }
        }
        let name = ingest::name(&l.name).unwrap_or_else(|| l.id.clone());
        Ok(RecipientListRecord {
            id: l.id.clone(),
            network: l.network.clone(),
            name,
            members,
        })
    }

    /// Confirm (with the hash of the text) or withdraw readable sends through a licensed
    /// account (§5).
    pub async fn confirm_ham(
        &self,
        account: &str,
        confirmed: bool,
        accept_hash: Option<&str>,
        by: &str,
    ) -> Result<(), BridgeError> {
        let (network, rec) = self.ham_entry(account, confirmed, accept_hash, by)?;
        self.write_sections(|s| {
            s.ham
                .retain(|r| !(r.network == network && r.account == account));
            s.ham.extend(rec);
        })
    }

    /// Set a channel's send switch (§5): turning it on for an uplinked channel needs the
    /// hash of its uplink text.
    pub async fn set_uplink_send(
        &self,
        conv: i64,
        send: bool,
        accept_hash: Option<&str>,
        by: &str,
    ) -> Result<(), BridgeError> {
        let rec = self.channel_entry(conv, send, accept_hash, by).await?;
        self.write_sections(|s| {
            s.channels.retain(|r| {
                !(r.network == rec.network
                    && r.account == rec.account
                    && r.native_id == rec.native_id)
            });
            s.channels.push(rec);
        })
    }

    /// The recipient lists.
    pub fn recipient_lists(&self) -> Vec<RecipientListRecord> {
        super::core::lock_sections(self).lists.clone()
    }

    /// Replace every recipient list (the contract's patch semantics); each list's
    /// conversation is created. Returns the lists as stored.
    pub async fn set_recipient_lists(
        &self,
        lists: &[RecipientListIn],
    ) -> Result<Vec<RecipientListRecord>, BridgeError> {
        let mut recs = Vec::with_capacity(lists.len());
        for l in lists {
            let r = self.list_entry(l)?;
            if recs
                .iter()
                .any(|o: &RecipientListRecord| o.network == r.network && o.id == r.id)
            {
                return Err(BridgeError::BadRequest(format!("list {} twice", r.id)));
            }
            recs.push(r);
        }
        self.ensure_list_convs(&recs).await?;
        let out = recs.clone();
        self.write_sections(|s| s.lists = recs)?;
        Ok(out)
    }

    /// Add or replace one recipient list.
    pub async fn put_recipient_list(
        &self,
        list: &RecipientListIn,
    ) -> Result<RecipientListRecord, BridgeError> {
        let r = self.list_entry(list)?;
        self.ensure_list_convs(std::slice::from_ref(&r)).await?;
        let out = r.clone();
        self.write_sections(|s| {
            s.lists
                .retain(|o| !(o.network == r.network && o.id == r.id));
            s.lists.push(r);
        })?;
        Ok(out)
    }

    /// Delete one recipient list (its conversation stays, with nothing to send to).
    pub async fn delete_recipient_list(
        &self,
        network: &str,
        id: &str,
    ) -> Result<bool, BridgeError> {
        let had = super::core::lock_sections(self).list(network, id).is_some();
        if had {
            self.write_sections(|s| s.lists.retain(|o| !(o.network == network && o.id == id)))?;
        }
        Ok(had)
    }

    /// The conversation id of a recipient list (created on save).
    pub async fn list_conv(&self, network: &str, id: &str) -> Result<i64, BridgeError> {
        let name = super::core::lock_sections(self)
            .list(network, id)
            .map(|l| l.name.clone())
            .ok_or(BridgeError::NotFound("recipient list"))?;
        Ok(self
            .fab()?
            .foreign_conversation_upsert(
                network,
                LIST_ACCOUNT,
                ForeignKind::Group,
                &format!("{LIST_PREFIX}{id}"),
                Some(&name),
            )
            .await?)
    }

    async fn ensure_list_convs(&self, lists: &[RecipientListRecord]) -> Result<(), BridgeError> {
        let f = self.fab()?;
        for l in lists {
            f.foreign_conversation_upsert(
                &l.network,
                LIST_ACCOUNT,
                ForeignKind::Group,
                &format!("{LIST_PREFIX}{}", l.id),
                Some(&l.name),
            )
            .await?;
        }
        Ok(())
    }

    /// Is bridging on for app-peer `contact` (its native id) of `account` on `network`
    /// (§7 (2))? Read live; off unless a human turned it on with the current text.
    pub fn contact_bridged(&self, network: &str, account: &str, contact: &str) -> bool {
        let text = contact_text(&self.label_of(network), contact);
        super::core::lock_sections(self).contact_bridged(network, account, contact, &text)
    }

    /// The contacts with bridging on for `network`.
    pub fn bridged_contacts(&self, network: &str) -> Vec<ContactRecord> {
        super::core::lock_sections(self)
            .contacts
            .iter()
            .filter(|r| r.network == network)
            .cloned()
            .collect()
    }

    /// Turn bridging on (with the hash of [`contact_text`]; else `409 consent_required`
    /// with the exact text) or off for app-peer `contact` of `account` (§7 (2)). Only an
    /// app-peer network (`app:…`) has the switch; the address is a native id (no
    /// whitespace, control or format characters, at most 254 bytes).
    pub async fn set_contact_bridging(
        &self,
        network: &str,
        account: &str,
        contact: &str,
        on: bool,
        accept_hash: Option<&str>,
        by: &str,
    ) -> Result<(), BridgeError> {
        if !self.known(network) {
            return Err(BridgeError::NotFound("network"));
        }
        if !network.starts_with("app:") {
            return Err(BridgeError::BadRequest(format!(
                "{} has no per-contact bridging",
                self.label_of(network)
            )));
        }
        let native_ok = |s: &str| {
            !s.is_empty()
                && s.len() <= 254
                && ingest::clean(s) == s
                && !s.chars().any(char::is_whitespace)
        };
        if !native_ok(contact) || !native_ok(account) {
            return Err(BridgeError::BadRequest(
                "that is not a contact address".into(),
            ));
        }
        if !on {
            return self.write_sections(|s| {
                s.contacts.retain(|r| {
                    !(r.network == network && r.account == account && r.contact == contact)
                })
            });
        }
        let text = contact_text(&self.label_of(network), contact);
        let hash = sha256_hex(&text);
        if accept_hash != Some(hash.as_str()) {
            return Err(BridgeError::Consent(ConsentRequired {
                action: "contact_bridge",
                subject: contact.to_string(),
                warning_text: text,
                hash,
                addons: Vec::new(),
            }));
        }
        let count = super::core::lock_sections(self)
            .contacts
            .iter()
            .filter(|r| r.network == network && !(r.account == account && r.contact == contact))
            .count();
        if count >= MAX_BRIDGED_CONTACTS {
            return Err(BridgeError::BadRequest(format!(
                "at most {MAX_BRIDGED_CONTACTS} bridged contacts per network"
            )));
        }
        let rec = ContactRecord {
            network: network.to_string(),
            account: account.to_string(),
            contact: contact.to_string(),
            text_hash: hash,
            by: by.to_string(),
        };
        self.write_sections(|s| {
            s.contacts.retain(|r| {
                !(r.network == rec.network && r.account == rec.account && r.contact == rec.contact)
            });
            s.contacts.push(rec);
        })
    }

    /// The LXMF announce name and interval (§6 (3)).
    pub fn announce(&self) -> AnnounceRecord {
        super::core::lock_sections(self).announce()
    }

    /// A compiled-in adapter's switch (`None`: never set).
    pub fn adapter_switch(&self, network: &str) -> Option<bool> {
        super::core::lock_sections(self)
            .adapters
            .get(network)
            .copied()
    }

    /// `GET /bridge/settings` (`BridgeSettings`).
    pub async fn settings(&self) -> Result<Value, BridgeError> {
        let convs = self.fab()?.foreign_conversations(None).await?;
        let s = super::core::lock_sections(self).clone();
        let mut adapters = serde_json::Map::new();
        for m in &self.manifests {
            if !compiled_in(&m.id) {
                continue;
            }
            let on = s
                .adapters
                .get(&m.id)
                .copied()
                .unwrap_or_else(|| self.attached(&m.id).is_some());
            adapters.insert(m.id.clone(), json!(on));
        }
        let mut channels = Vec::new();
        for c in convs.iter().filter(|c| c.kind == ForeignKind::Channel) {
            let facts = self.facts_of(&c.network);
            let live = self.conv_live(c, facts.as_ref());
            channels.push(json!({
                "conv": fjson::fc_id(c.id),
                "network": c.network,
                "send": live.send_enabled.unwrap_or(false),
                "uplink": live.uplink,
            }));
        }
        let mut ham = Vec::new();
        for (net, a) in self.attached_all() {
            let label = self.label_of(&net);
            for acct in a.adapter.facts().accounts {
                let confirmed = s.ham_confirmed(&net, &acct.id, &ham_text(&label, &acct.id));
                ham.push(json!({
                    "account": acct.id,
                    "licensed": acct.licensed,
                    "confirmed": confirmed,
                }));
            }
        }
        let lists: Vec<Value> = s
            .lists
            .iter()
            .map(|l| json!({ "id": l.id, "network": l.network, "name": l.name, "members": l.members }))
            .collect();
        let announce = s.announce();
        Ok(json!({
            "schema": 1,
            "order": self.network_ids(),
            "adapters": adapters,
            "recipient_lists": lists,
            "channels": channels,
            "ham": ham,
            "announce": { "name": announce.name, "interval_min": announce.interval_min },
        }))
    }

    /// `PATCH /bridge/settings` (module doc: all or nothing; consent gates answer 409 with
    /// the text and hash). Returns the settings as they now read.
    pub async fn patch_settings(&self, p: &SettingsPatch, by: &str) -> Result<Value, BridgeError> {
        if p.order.is_some() {
            return Err(BridgeError::BadRequest(
                "the preferred order is set through PATCH /transport (interfaces.toml)".into(),
            ));
        }
        if let Some(a) = &p.adapters {
            for net in a.keys() {
                if self.manifest(net).is_none() {
                    return Err(BridgeError::NotFound("network"));
                }
                if !compiled_in(net) {
                    return Err(BridgeError::BadRequest(format!(
                        "{net} is an optional component: POST /bridge/components/{{id}}/enable"
                    )));
                }
            }
        }
        let mut lists = None;
        if let Some(ls) = &p.recipient_lists {
            let mut recs: Vec<RecipientListRecord> = Vec::with_capacity(ls.len());
            for l in ls {
                let r = self.list_entry(l)?;
                if recs.iter().any(|o| o.network == r.network && o.id == r.id) {
                    return Err(BridgeError::BadRequest(format!("list {} twice", r.id)));
                }
                recs.push(r);
            }
            lists = Some(recs);
        }
        let mut chans = Vec::with_capacity(p.channels.len());
        for c in &p.channels {
            let id = fjson::parse_fc_id(&c.conv).ok_or(BridgeError::NotFound("conversation"))?;
            chans.push(
                self.channel_entry(id, c.send, c.accept_warning.as_deref(), by)
                    .await?,
            );
        }
        let mut hams = Vec::with_capacity(p.ham.len());
        for h in &p.ham {
            let (net, rec) =
                self.ham_entry(&h.account, h.confirmed, h.accept_warning.as_deref(), by)?;
            hams.push((net, h.account.clone(), rec));
        }
        let announce = match &p.announce {
            None => None,
            Some(a) => {
                let cur = self.announce();
                let name = match &a.name {
                    None => cur.name,
                    Some(None) => None,
                    Some(Some(n)) => Some(
                        ingest::name(n)
                            .ok_or_else(|| BridgeError::BadRequest("empty announce name".into()))?,
                    ),
                };
                Some(AnnounceRecord {
                    name,
                    interval_min: a.interval_min.unwrap_or(cur.interval_min),
                })
            }
        };
        if let Some(recs) = &lists {
            self.ensure_list_convs(recs).await?;
        }
        let adapters = p.adapters.clone();
        self.write_sections(move |s| {
            if let Some(a) = adapters {
                s.adapters.extend(a);
            }
            if let Some(l) = lists {
                s.lists = l;
            }
            for rec in chans {
                s.channels.retain(|r| {
                    !(r.network == rec.network
                        && r.account == rec.account
                        && r.native_id == rec.native_id)
                });
                s.channels.push(rec);
            }
            for (net, account, rec) in hams {
                s.ham
                    .retain(|r| !(r.network == net && r.account == account));
                s.ham.extend(rec);
            }
            if let Some(a) = announce {
                s.announce = Some(a);
            }
        })?;
        self.settings().await
    }
}

/// The consent subject of a channel (its `fc-` id) — for routes building their own copy.
pub fn channel_subject(c: &ForeignConversationRow) -> String {
    fjson::fc_id(c.id)
}

/// The native coordinates of a list's destination conversation (tests, routes).
pub fn list_conv_ref(network: &str, id: &str) -> super::ConvRef {
    super::ConvRef {
        network: network.to_string(),
        account: LIST_ACCOUNT.to_string(),
        kind: ConvKind::Group,
        native_id: format!("{LIST_PREFIX}{id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_lowercase_sha256_of_the_exact_text() {
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_ne!(sha256_hex("abc "), sha256_hex("abc"));
    }

    #[test]
    fn the_send_switch_needs_the_current_uplink_hash() {
        let mut s = Sections::default();
        let t = uplink_text("Meshtastic", "ch:LongFast:9f3c", "published");
        assert!(
            s.send_allowed("meshtastic", "!a", "ch:LongFast:9f3c", false, &t),
            "on by default"
        );
        assert!(
            !s.send_allowed("meshtastic", "!a", "ch:LongFast:9f3c", true, &t),
            "uplinked: off"
        );
        s.channels.push(ChannelRecord {
            network: "meshtastic".into(),
            account: "!a".into(),
            native_id: "ch:LongFast:9f3c".into(),
            send: true,
            text_hash: Some(sha256_hex(&t)),
            by: "loopback".into(),
        });
        assert!(s.send_allowed("meshtastic", "!a", "ch:LongFast:9f3c", true, &t));
        let changed = uplink_text("Meshtastic", "ch:LongFast:9f3c", "published (new copy)");
        assert!(
            !s.send_allowed("meshtastic", "!a", "ch:LongFast:9f3c", true, &changed),
            "a changed text asks again"
        );
        s.channels[0].send = false;
        assert!(
            !s.send_allowed("meshtastic", "!a", "ch:LongFast:9f3c", false, &t),
            "off gates even a channel that is not uplinked"
        );
    }

    #[test]
    fn patches_parse_with_absent_and_null_announce_names() {
        let p: SettingsPatch =
            serde_json::from_value(json!({"announce": {"interval_min": 0}})).unwrap();
        assert_eq!(p.announce.unwrap().name, None);
        let p: SettingsPatch = serde_json::from_value(json!({"announce": {"name": null}})).unwrap();
        assert_eq!(p.announce.unwrap().name, Some(None));
        let p: SettingsPatch = serde_json::from_value(
            json!({"announce": {"name": "hub"}, "ham": [{"account": "!a", "confirmed": true}]}),
        )
        .unwrap();
        assert_eq!(p.announce.unwrap().name, Some(Some("hub".into())));
        assert_eq!(p.ham.len(), 1);
        assert!(serde_json::from_value::<SettingsPatch>(json!({"surprise": 1})).is_err());
        assert!(list_id_ok("friends-1_x") && !list_id_ok("") && !list_id_ok("a b"));
    }

    #[test]
    fn a_contact_is_bridged_only_with_the_current_text_hash() {
        let mut s = Sections::default();
        let t = contact_text("Delta Chat", "alice@[192.168.1.5]");
        assert!(t.ends_with(
            "This hub decrypts this contact's messages to bridge them. What you relay onto \
             another network is readable there."
        ));
        assert!(!s.contact_bridged(
            "app:deltachat",
            "hub@[192.168.1.5]",
            "alice@[192.168.1.5]",
            &t
        ));
        s.contacts.push(ContactRecord {
            network: "app:deltachat".into(),
            account: "hub@[192.168.1.5]".into(),
            contact: "alice@[192.168.1.5]".into(),
            text_hash: sha256_hex(&t),
            by: "loopback".into(),
        });
        assert!(s.contact_bridged(
            "app:deltachat",
            "hub@[192.168.1.5]",
            "alice@[192.168.1.5]",
            &t
        ));
        assert!(!s.contact_bridged(
            "app:deltachat",
            "hub@[192.168.1.5]",
            "bob@[192.168.1.5]",
            &t
        ));
        let changed = format!("{t} (new copy)");
        assert!(
            !s.contact_bridged(
                "app:deltachat",
                "hub@[192.168.1.5]",
                "alice@[192.168.1.5]",
                &changed
            ),
            "a changed text asks again"
        );
    }
}
