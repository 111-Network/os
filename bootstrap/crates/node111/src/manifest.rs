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

use crate::calver::Calver;
use crate::floor::Floor;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::str::FromStr;

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
    rns_lock: PinnedArtifact,
    kb: PinnedArtifact,
    keychain: Vec<KeyEntry>,
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
    #[serde(default, rename = "deb")]
    debs: Vec<RawDeb>,
    #[serde(rename = "rns-lock")]
    rns_lock: RawArtifact,
    kb: RawArtifact,
    #[serde(default)]
    keychain: Vec<RawKeyEntry>,
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
            rns_lock,
            kb,
            keychain,
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

#[cfg(test)]
mod tests {
    // Known-answer + tamper tests live in tests/manifest_parse.rs against the
    // committed fixture (tests/fixtures/manifest/channel-111-stable.toml).
}
