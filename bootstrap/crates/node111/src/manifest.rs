//! `channel-111-stable.toml` — rustup-v2-style signed channel manifest.
//!
//! Order of operations is contractual: **parse → verify minisig → check floor →
//! only then may any pin inside be trusted.** Use `parse_verified` for the full
//! chain; `parse` alone is for tooling/tests that handle signature verification
//! separately.
//!
//! ## TOML layout (canonical example: tests/fixtures/manifest/channel-111-stable.toml)
//!
//! ```toml
//! manifest-version = 1
//! date = "2026.10.12"            # manifest issue calver
//! channel = "stable"
//! release = "2026.10.12"         # calver of the release this manifest ships
//! min-upgrade-from = "2026.10.01"
//!
//! [pkg.111.target.x86_64-unknown-linux-musl]   # the `111` binary per target
//! url = "https://..."
//! sha256 = "<64 lowercase hex>"
//! size = 1234567
//!
//! [payload.x86_64-unknown-linux-musl]          # release payload tarball per target
//! url = "https://..."
//! sha256 = "..."
//! size = 123
//!
//! [apk.termux]        # termux-app
//! url = "..."
//! sha256 = "..."
//! size = 123
//! [apk.termux-boot]   # Termux:Boot
//! ...
//! [apk.termux-api]    # Termux:API
//! ...
//!
//! [[deb]]             # Termux .deb closure (ordered)
//! name = "openssh"
//! url = "..."
//! sha256 = "..."
//! size = 123
//!
//! [rns-lock]          # hash-locked pip requirements (rns==1.5.6 closure)
//! url = "..."
//! sha256 = "..."
//! size = 123
//!
//! [kb]                # KB snapshot (also inside the payload; pinned here for prefetch)
//! url = "..."
//! sha256 = "..."
//! size = 123
//!
//! [[keychain]]        # rotation chain: new keys signed by old, monotonic generations
//! generation = 0      # root generation (cold custody) has no signature_by_previous
//! pubkey = "RW..."
//! [[keychain]]
//! generation = 1
//! pubkey = "RW..."
//! signature_by_previous = "<detached minisig of the new pubkey, made by generation-0>"
//! ```
//!
//! Validation at parse time: sha256 fields must be 64-char lowercase hex, sizes > 0,
//! URLs must be http(s) (https in production; http is tolerated for loopback test
//! fixtures), keychain generations strictly consecutive starting at 0, unknown
//! fields rejected (`deny_unknown_fields` — evolution bumps `manifest-version`).
//!
//! ## Layer 5 entries (D20; `os/messaging/README.md` §2, §13, §14, §19 L1)
//!
//! Optional and additive. Each one rides **inside** the release payload as well, so
//! the updater, the root helper and the installer receive it only through the
//! pinned payload tarball and its `share/payload.manifest` tree check, exactly like
//! `bin/111` and `share/kb/` — nothing is fetched separately and nothing downloads
//! at runtime. The pins here name the CI-built bytes in the signed manifest, and
//! [`ChannelManifest::verify_layer5_tree`] cross-checks an extracted payload tree
//! against them (paths in [`layer5`]).
//!
//! ```toml
//! [pkg.meshmud.target.x86_64-unknown-linux-musl]  # meshmud, built in CI from the pinned
//! url = "https://..."                             # upstream tarball (scripts/meshmud-build.sh;
//! sha256 = "..."                                  # static, conveyed per its .verdict); in that
//! size = 123                                      # target's payload as `bin/meshmud`
//!
//! [bundle.meshtastic-web]   # the Meshtastic web client (scripts/meshtastic-web-build.sh)
//! url = "https://..."       # its deterministic tar (meshtastic-web-<tag>.tar)
//! sha256 = "..."
//! size = 123
//! tree-sha256 = "..."       # the bundle hash: sha256 of the `sha256␠␠path` listing
//!                           # (WEB_TREE_SHA256); unpacked in every payload at
//!                           # `panels/meshtastic-web/`
//!
//! [presence-lock]           # the meshtastic-presence venv's hash-locked pip closure
//! url = "https://..."       # (meshtastic-presence-lock.txt); in every payload under
//! sha256 = "..."            # `share/meshtastic-presence/`; the node builds `venvs/presence`
//! size = 123                # from it with `pip install --require-hashes`, like `rns-lock`
//! ```
//!
//! Package names are closed (`111`, `meshmud`) and so are bundle names
//! (`meshtastic-web`, the bundle the embedded panel declaration points at): an
//! unknown one is refused like an unknown field. A `meshmud` target needs a `111`
//! binary and a payload for the same triple (it rides in that payload).
//! `manifest-version` stays 1: no signed release has shipped yet, and the first
//! one carries these sections; any section added after it bumps the version.

use crate::calver::Calver;
use crate::floor::Floor;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Package names a manifest may pin under `[pkg.<name>.target.<triple>]`.
pub const KNOWN_PACKAGES: [&str; 2] = ["111", MESHMUD_PACKAGE];

/// The `meshmud` package (Layer 5 §14): the Meshtastic multiplexer, per target.
pub const MESHMUD_PACKAGE: &str = "meshmud";

/// Bundle names a manifest may pin under `[bundle.<name>]`: the static panel
/// bundles the embedded panel declarations point at (`assets/panels/*.toml`
/// `bundle = "panels/<name>"`, Layer 5 §13).
pub const KNOWN_BUNDLES: [&str; 1] = ["meshtastic-web"];

/// Where the Layer 5 assets sit inside a payload tree (= `releases/<ver>/`, i.e.
/// `current/`), and where the node builds the presence venv (Layer 5 §2, §13, §14;
/// `os/bootstrap/README.md` "Layer 5 amendments").
pub mod layer5 {
    /// meshmud's binary (`current/bin/meshmud`, Layer 5 §14).
    pub const MESHMUD_BIN: &str = "bin/meshmud";
    /// Parent of every static panel bundle (`current/panels/<name>/`, §13).
    pub const PANELS_DIR: &str = "panels";
    /// The presence reader's directory (script, lock, license; §14).
    pub const PRESENCE_DIR: &str = "share/meshtastic-presence";
    /// The presence venv lock inside the payload (pinned as `[presence-lock]`).
    pub const PRESENCE_LOCK: &str = "share/meshtastic-presence/meshtastic-presence-lock.txt";
    /// The presence venv, under the state root (`/var/lib/111/venvs/presence`),
    /// built on the node from [`PRESENCE_LOCK`] — never shipped prebuilt.
    pub const PRESENCE_VENV: &str = "venvs/presence";
}

/// Upper bounds for walking a bundle tree (a packaging error, not a bundle,
/// beyond these).
const BUNDLE_MAX_FILES: usize = 65_536;
const BUNDLE_MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedArtifact {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub target: String,
    pub artifact: PinnedArtifact,
}

/// Keychain rotation entry: new keys signed by old, monotonic generations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEntry {
    pub generation: u32,
    pub pubkey: String,
    /// Detached minisig of this pubkey made by the previous-generation key.
    /// None only on generation 0 (the cold-custody root).
    pub signature_by_previous: Option<String>,
}

/// A static panel bundle (Layer 5 §13): the deterministic tar CI built, and the
/// tree hash of its unpacked files as they sit at `panels/<name>/` in every payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// `artifact.name` is the bundle name (`meshtastic-web`).
    pub artifact: PinnedArtifact,
    /// sha256 of the `sha256␠␠relpath\n` listing of every file, sorted bytewise —
    /// `meshtastic-web-build.sh`'s `.tree` file hash ([`bundle_tree_sha256`]).
    pub tree_sha256: String,
}

#[derive(Debug, Clone)]
pub struct ChannelManifest {
    pub manifest_version: u32,
    date: Calver,
    channel: String,
    release: Calver,
    min_upgrade_from: Calver,
    packages: Vec<Package>,
    payloads: Vec<PinnedArtifact>, // name = target triple
    termux_apks: Vec<PinnedArtifact>,
    termux_debs: Vec<PinnedArtifact>,
    platform_tools: Option<PinnedArtifact>,
    rns_lock: PinnedArtifact,
    kb: PinnedArtifact,
    keychain: Vec<KeyEntry>,
    bundles: Vec<Bundle>,
    presence_lock: Option<PinnedArtifact>,
}

// ---------- raw serde layer ----------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    #[serde(rename = "manifest-version")]
    manifest_version: u32,
    date: String,
    channel: String,
    release: String,
    #[serde(rename = "min-upgrade-from")]
    min_upgrade_from: String,
    #[serde(default)]
    pkg: BTreeMap<String, RawPkg>,
    #[serde(default)]
    payload: BTreeMap<String, RawArtifact>,
    #[serde(default)]
    apk: BTreeMap<String, RawArtifact>,
    #[serde(default, rename = "platform-tools")]
    platform_tools: Option<RawArtifact>,
    #[serde(default, rename = "deb")]
    debs: Vec<RawDeb>,
    #[serde(rename = "rns-lock")]
    rns_lock: RawArtifact,
    kb: RawArtifact,
    #[serde(default)]
    keychain: Vec<RawKeyEntry>,
    #[serde(default)]
    bundle: BTreeMap<String, RawBundle>,
    #[serde(default, rename = "presence-lock")]
    presence_lock: Option<RawArtifact>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPkg {
    target: BTreeMap<String, RawArtifact>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct RawArtifact {
    url: String,
    sha256: String,
    size: u64,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct RawBundle {
    url: String,
    sha256: String,
    size: u64,
    #[serde(rename = "tree-sha256")]
    tree_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDeb {
    name: String,
    url: String,
    sha256: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeyEntry {
    generation: u32,
    pubkey: String,
    signature_by_previous: Option<String>,
}

// ---------- validation ----------

fn validate_sha256(field: &str, sha: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        sha.len() == 64
            && sha
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "{field}: sha256 must be 64 lowercase hex chars, got {sha:?}"
    );
    Ok(())
}

fn validate_url(field: &str, url: &str) -> anyhow::Result<()> {
    // Production pins are https (GitHub Releases + own-domain mirror); http is
    // tolerated so loopback fixture servers can be used in tests.
    anyhow::ensure!(
        url.starts_with("https://") || url.starts_with("http://"),
        "{field}: url must be http(s), got {url:?}"
    );
    Ok(())
}

fn artifact(name: String, field: &str, raw: RawArtifact) -> anyhow::Result<PinnedArtifact> {
    validate_url(field, &raw.url)?;
    validate_sha256(field, &raw.sha256)?;
    anyhow::ensure!(raw.size > 0, "{field}: size must be > 0");
    Ok(PinnedArtifact {
        name,
        url: raw.url,
        sha256: raw.sha256,
        size: raw.size,
    })
}

impl ChannelManifest {
    /// Parse and structurally validate. Does NOT verify the signature — see
    /// `parse_verified` for the contractual chain.
    pub fn parse(toml_str: &str) -> anyhow::Result<Self> {
        let raw: RawManifest =
            toml::from_str(toml_str).map_err(|e| anyhow::anyhow!("manifest TOML: {e}"))?;
        anyhow::ensure!(
            raw.manifest_version == 1,
            "unsupported manifest-version {}",
            raw.manifest_version
        );
        anyhow::ensure!(!raw.channel.is_empty(), "channel must not be empty");

        let date =
            Calver::from_str(&raw.date).map_err(|e| anyhow::anyhow!("manifest date: {e}"))?;
        let release =
            Calver::from_str(&raw.release).map_err(|e| anyhow::anyhow!("manifest release: {e}"))?;
        let min_upgrade_from = Calver::from_str(&raw.min_upgrade_from)
            .map_err(|e| anyhow::anyhow!("manifest min-upgrade-from: {e}"))?;

        let mut packages = Vec::new();
        for (pkg_name, pkg) in &raw.pkg {
            anyhow::ensure!(
                KNOWN_PACKAGES.contains(&pkg_name.as_str()),
                "pkg.{pkg_name}: unknown package (known: {})",
                KNOWN_PACKAGES.join(", ")
            );
            for (target, art) in &pkg.target {
                let field = format!("pkg.{pkg_name}.target.{target}");
                let artifact = artifact(pkg_name.clone(), &field, art.clone())?;
                packages.push(Package {
                    name: pkg_name.clone(),
                    target: target.clone(),
                    artifact,
                });
            }
        }
        anyhow::ensure!(!packages.is_empty(), "manifest ships no pkg entries");

        let mut payloads = Vec::new();
        for (triple, art) in &raw.payload {
            payloads.push(artifact(
                triple.clone(),
                &format!("payload.{triple}"),
                art.clone(),
            )?);
        }
        anyhow::ensure!(!payloads.is_empty(), "manifest ships no payload entries");

        // Layer 5 §14: meshmud rides inside its target's payload next to `bin/111`.
        for p in packages.iter().filter(|p| p.name == MESHMUD_PACKAGE) {
            anyhow::ensure!(
                packages
                    .iter()
                    .any(|q| q.name == "111" && q.target == p.target),
                "pkg.meshmud.target.{}: no `111` binary for that target",
                p.target
            );
            anyhow::ensure!(
                payloads.iter().any(|q| q.name == p.target),
                "pkg.meshmud.target.{}: no payload for that target (meshmud rides inside it)",
                p.target
            );
        }

        let mut bundles = Vec::new();
        for (name, raw_bundle) in &raw.bundle {
            let field = format!("bundle.{name}");
            anyhow::ensure!(
                KNOWN_BUNDLES.contains(&name.as_str()),
                "{field}: unknown bundle (known: {})",
                KNOWN_BUNDLES.join(", ")
            );
            validate_sha256(&format!("{field}.tree-sha256"), &raw_bundle.tree_sha256)?;
            let artifact = artifact(
                name.clone(),
                &field,
                RawArtifact {
                    url: raw_bundle.url.clone(),
                    sha256: raw_bundle.sha256.clone(),
                    size: raw_bundle.size,
                },
            )?;
            bundles.push(Bundle {
                artifact,
                tree_sha256: raw_bundle.tree_sha256.clone(),
            });
        }
        let presence_lock = raw
            .presence_lock
            .map(|r| artifact("meshtastic-presence-lock.txt".into(), "presence-lock", r))
            .transpose()?;

        let mut termux_apks = Vec::new();
        for (name, art) in &raw.apk {
            termux_apks.push(artifact(name.clone(), &format!("apk.{name}"), art.clone())?);
        }

        let mut termux_debs = Vec::new();
        for deb in &raw.debs {
            anyhow::ensure!(!deb.name.is_empty(), "deb entry with empty name");
            termux_debs.push(artifact(
                deb.name.clone(),
                &format!("deb {}", deb.name),
                RawArtifact {
                    url: deb.url.clone(),
                    sha256: deb.sha256.clone(),
                    size: deb.size,
                },
            )?);
        }

        let rns_lock = artifact("rns-lock.txt".into(), "rns-lock", raw.rns_lock.clone())?;
        let kb = artifact("kb".into(), "kb", raw.kb.clone())?;
        let platform_tools = raw
            .platform_tools
            .map(|r| artifact("platform-tools".into(), "platform-tools", r))
            .transpose()?;

        let mut keychain = Vec::new();
        for (i, entry) in raw.keychain.iter().enumerate() {
            anyhow::ensure!(
                entry.generation as usize == i,
                "keychain generations must be consecutive from 0: entry {i} claims {}",
                entry.generation
            );
            if i == 0 {
                anyhow::ensure!(
                    entry.signature_by_previous.is_none(),
                    "keychain generation 0 (root) must not carry signature_by_previous"
                );
            } else {
                anyhow::ensure!(
                    entry.signature_by_previous.is_some(),
                    "keychain generation {} lacks signature_by_previous",
                    entry.generation
                );
            }
            keychain.push(KeyEntry {
                generation: entry.generation,
                pubkey: entry.pubkey.clone(),
                signature_by_previous: entry.signature_by_previous.clone(),
            });
        }

        Ok(ChannelManifest {
            manifest_version: raw.manifest_version,
            date,
            channel: raw.channel,
            release,
            min_upgrade_from,
            packages,
            payloads,
            termux_apks,
            termux_debs,
            platform_tools,
            rns_lock,
            kb,
            keychain,
            bundles,
            presence_lock,
        })
    }

    /// The contractual chain: verify the detached minisig over the raw bytes,
    /// parse, then check the monotonic floor. Only the Ok result may be trusted.
    pub fn parse_verified(
        toml_bytes: &[u8],
        signature_b64: &str,
        pubkey_b64: &str,
        floor: &Floor,
    ) -> anyhow::Result<Self> {
        crate::minisign::verify_detached(toml_bytes, signature_b64, pubkey_b64)?;
        let text =
            std::str::from_utf8(toml_bytes).map_err(|_| anyhow::anyhow!("manifest not UTF-8"))?;
        let m = Self::parse(text)?;
        m.check_floor(floor)?;
        Ok(m)
    }

    pub fn date(&self) -> Calver {
        self.date
    }
    pub fn channel(&self) -> &str {
        &self.channel
    }
    pub fn release(&self) -> Calver {
        self.release
    }
    pub fn min_upgrade_from(&self) -> Calver {
        self.min_upgrade_from
    }
    pub fn packages(&self) -> &[Package] {
        &self.packages
    }
    pub fn payloads(&self) -> &[PinnedArtifact] {
        &self.payloads
    }
    pub fn termux_apks(&self) -> &[PinnedArtifact] {
        &self.termux_apks
    }
    pub fn termux_debs(&self) -> &[PinnedArtifact] {
        &self.termux_debs
    }
    /// Google platform-tools zip pin for the adb fallback (absent ⇒ fallback
    /// unavailable, phone flow requires a distro adb).
    pub fn platform_tools(&self) -> Option<&PinnedArtifact> {
        self.platform_tools.as_ref()
    }
    pub fn rns_lock(&self) -> &PinnedArtifact {
        &self.rns_lock
    }
    pub fn kb(&self) -> &PinnedArtifact {
        &self.kb
    }
    pub fn keychain(&self) -> &[KeyEntry] {
        &self.keychain
    }

    /// Release payload tarball for a target triple, if shipped.
    pub fn payload_for(&self, target_triple: &str) -> Option<&PinnedArtifact> {
        self.payloads.iter().find(|p| p.name == target_triple)
    }

    /// The `111` binary package for a target triple, if shipped.
    pub fn binary_for(&self, target_triple: &str) -> Option<&Package> {
        self.packages
            .iter()
            .find(|p| p.name == "111" && p.target == target_triple)
    }

    /// The `meshmud` binary for a target triple, if this release ships one
    /// (Layer 5 §14; absent ⇒ the node reports meshmud honestly absent).
    pub fn meshmud_for(&self, target_triple: &str) -> Option<&Package> {
        self.packages
            .iter()
            .find(|p| p.name == MESHMUD_PACKAGE && p.target == target_triple)
    }

    /// Static panel bundles this release ships (Layer 5 §13).
    pub fn bundles(&self) -> &[Bundle] {
        &self.bundles
    }

    /// One bundle by name (`meshtastic-web`).
    pub fn bundle(&self, name: &str) -> Option<&Bundle> {
        self.bundles.iter().find(|b| b.artifact.name == name)
    }

    /// The presence venv lock (Layer 5 §14), if this release ships the reader.
    pub fn presence_lock(&self) -> Option<&PinnedArtifact> {
        self.presence_lock.as_ref()
    }

    /// Cross-check an extracted payload tree (`releases/<ver>/` or a staging
    /// `tree/`) for `target_triple` against this manifest's Layer 5 pins
    /// (`os/messaging/README.md` §19 L1). Fails closed both ways: a pinned asset
    /// must be present with exactly the pinned bytes, and an asset the manifest
    /// does not pin must not be in the tree.
    ///
    /// - `bin/meshmud`: a regular file (never a link) whose size and sha256 equal
    ///   `pkg.meshmud.target.<triple>`; absent when that pin is absent.
    /// - `panels/<name>/`: a real directory whose [`bundle_tree_sha256`] equals
    ///   `bundle.<name>.tree-sha256` (every file is covered, so an extra, missing
    ///   or altered file fails); nothing under `panels/` that is not pinned.
    /// - `share/meshtastic-presence/meshtastic-presence-lock.txt`: size and sha256
    ///   equal `presence-lock`; absent when that pin is absent.
    ///
    /// Call it right after `daemon::updater::stage::verify_tree` (which checks the
    /// tree against the payload's own `share/payload.manifest`).
    pub fn verify_layer5_tree(&self, tree: &Path, target_triple: &str) -> anyhow::Result<()> {
        let bin = tree.join(layer5::MESHMUD_BIN);
        match self.meshmud_for(target_triple) {
            Some(pkg) => check_pinned_file(&bin, &pkg.artifact, layer5::MESHMUD_BIN)?,
            None => ensure_absent(
                &bin,
                &format!(
                    "payload carries {} but the manifest pins no meshmud for {target_triple}",
                    layer5::MESHMUD_BIN
                ),
            )?,
        }

        let panels = tree.join(layer5::PANELS_DIR);
        for b in &self.bundles {
            let dir = panels.join(&b.artifact.name);
            let rel = format!("{}/{}", layer5::PANELS_DIR, b.artifact.name);
            let meta = std::fs::symlink_metadata(&dir)
                .map_err(|e| anyhow::anyhow!("payload lacks pinned bundle {rel}/: {e}"))?;
            anyhow::ensure!(
                meta.file_type().is_dir(),
                "payload {rel} is not a directory (links are refused)"
            );
            let got = bundle_tree_sha256(&dir)?;
            anyhow::ensure!(
                got == b.tree_sha256,
                "bundle {rel}/ tree hash {got} != manifest tree-sha256 {}",
                b.tree_sha256
            );
        }
        match std::fs::symlink_metadata(&panels) {
            Ok(meta) => {
                anyhow::ensure!(
                    meta.file_type().is_dir(),
                    "payload {} is not a directory",
                    layer5::PANELS_DIR
                );
                for entry in std::fs::read_dir(&panels)? {
                    let entry = entry?;
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    anyhow::ensure!(
                        self.bundle(&name).is_some(),
                        "payload carries {}/{name} but the manifest pins no such bundle",
                        layer5::PANELS_DIR
                    );
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => anyhow::bail!("payload {} unreadable: {e}", layer5::PANELS_DIR),
        }

        let lock = tree.join(layer5::PRESENCE_LOCK);
        match &self.presence_lock {
            Some(pin) => check_pinned_file(&lock, pin, layer5::PRESENCE_LOCK)?,
            None => ensure_absent(
                &lock,
                &format!(
                    "payload carries {} but the manifest pins no presence-lock",
                    layer5::PRESENCE_LOCK
                ),
            )?,
        }
        Ok(())
    }

    /// Refuse anything at/below the floor (anti-replay; replayed old-but-valid
    /// signatures included). Compares release, manifest date, and the keychain's
    /// maximum generation against the stored floor.
    pub fn check_floor(&self, floor: &Floor) -> anyhow::Result<()> {
        let max_gen = self
            .keychain
            .iter()
            .map(|k| k.generation)
            .max()
            .unwrap_or(0);
        anyhow::ensure!(
            !floor.refuses(&self.release, &self.date, max_gen),
            "manifest at/below monotonic floor (release {} date {} keychain gen {} vs floor {:?})",
            self.release,
            self.date,
            max_gen,
            floor
        );
        Ok(())
    }
}

/// A pinned regular file in a payload tree: never a link, exact size, exact sha256.
fn check_pinned_file(path: &Path, pin: &PinnedArtifact, rel: &str) -> anyhow::Result<()> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| anyhow::anyhow!("payload lacks pinned {rel}: {e}"))?;
    anyhow::ensure!(
        meta.file_type().is_file(),
        "payload {rel} is not a regular file (links are refused)"
    );
    anyhow::ensure!(
        meta.len() == pin.size,
        "payload {rel} size {} != manifest {}",
        meta.len(),
        pin.size
    );
    let got = crate::util::hash::sha256_file(path)?;
    anyhow::ensure!(
        got == pin.sha256,
        "payload {rel} sha256 {got} != manifest {}",
        pin.sha256
    );
    Ok(())
}

/// Nothing (not even a dangling link) at `path`.
fn ensure_absent(path: &Path, why: &str) -> anyhow::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => anyhow::bail!("{why}"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => anyhow::bail!("{}: {e}", path.display()),
    }
}

/// The bundle hash (Layer 5 §13): sha256 of the listing `"<sha256>␠␠<relpath>\n"`
/// for every regular file under `dir`, relpaths `/`-separated and sorted bytewise —
/// byte-identical to `meshtastic-web-build.sh`'s `.tree` file
/// (`find . -type f | LC_ALL=C sort | xargs sha256sum`, `./` stripped), whose hash
/// is `WEB_TREE_SHA256`. Fails closed on links and special files (the build script
/// refuses them too), on names that are not UTF-8 or that `sha256sum` would escape
/// (`\`, newline, carriage return), and beyond 65 536 files or 32 levels.
pub fn bundle_tree_sha256(dir: &Path) -> anyhow::Result<String> {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    walk_bundle(dir, "", 0, &mut files)?;
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut listing = String::new();
    for (rel, path) in &files {
        listing.push_str(&crate::util::hash::sha256_file(path)?);
        listing.push_str("  ");
        listing.push_str(rel);
        listing.push('\n');
    }
    Ok(crate::util::hash::sha256_bytes(listing.as_bytes()))
}

fn walk_bundle(
    dir: &Path,
    prefix: &str,
    depth: usize,
    out: &mut Vec<(String, PathBuf)>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        depth <= BUNDLE_MAX_DEPTH,
        "bundle nests deeper than {BUNDLE_MAX_DEPTH} levels at {prefix}"
    );
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let os_name = entry.file_name();
        let name = os_name
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("bundle file name is not UTF-8 under {prefix:?}"))?;
        anyhow::ensure!(
            !name.contains(['\\', '\n', '\r']),
            "bundle file name {name:?} would be escaped by sha256sum"
        );
        let rel = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        let kind = entry.file_type()?; // never follows links
        if kind.is_dir() {
            walk_bundle(&entry.path(), &rel, depth + 1, out)?;
        } else if kind.is_file() {
            anyhow::ensure!(
                out.len() < BUNDLE_MAX_FILES,
                "bundle holds more than {BUNDLE_MAX_FILES} files"
            );
            out.push((rel, entry.path()));
        } else {
            anyhow::bail!("bundle entry {rel} is a link or special file");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // Known-answer + tamper tests live in tests/manifest_parse.rs against the
    // committed fixture (tests/fixtures/manifest/channel-111-stable.toml); the
    // Layer 5 entries in tests/manifest_layer5.rs.
}
