//! `channel-111-stable.toml` — rustup-v2-style signed channel manifest.
//! Order of operations is contractual: parse → verify minisig → check floor →
//! only then may any pin inside be trusted.

use crate::calver::Calver;
use crate::floor::Floor;
use serde::Deserialize;

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
    /// Detached minisig of this key made by the previous-generation key.
    pub signature_by_previous: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelManifest {
    pub manifest_version: u32,
    // Deserialization detail is T2's; the pinned surface is the typed accessors below.
    #[serde(flatten)]
    pub raw: toml::Value,
}

impl ChannelManifest {
    pub fn parse(toml_str: &str) -> anyhow::Result<Self> {
        let _ = toml_str;
        todo!("T2")
    }
    pub fn date(&self) -> anyhow::Result<Calver> {
        todo!("T2")
    }
    pub fn release(&self) -> anyhow::Result<Calver> {
        todo!("T2")
    }
    pub fn min_upgrade_from(&self) -> anyhow::Result<Calver> {
        todo!("T2")
    }
    pub fn packages(&self) -> anyhow::Result<Vec<Package>> {
        todo!("T2")
    }
    pub fn termux_apks(&self) -> anyhow::Result<Vec<PinnedArtifact>> {
        todo!("T2")
    }
    pub fn termux_debs(&self) -> anyhow::Result<Vec<PinnedArtifact>> {
        todo!("T2")
    }
    pub fn rns_lock(&self) -> anyhow::Result<PinnedArtifact> {
        todo!("T2")
    }
    pub fn kb(&self) -> anyhow::Result<PinnedArtifact> {
        todo!("T2")
    }
    pub fn payload_for(&self, target_triple: &str) -> anyhow::Result<Option<PinnedArtifact>> {
        let _ = target_triple;
        todo!("T2")
    }
    pub fn keychain(&self) -> anyhow::Result<Vec<KeyEntry>> {
        todo!("T2")
    }
    /// Refuse anything at/below the floor or below `min-upgrade-from`.
    pub fn check_floor(&self, floor: &Floor) -> anyhow::Result<()> {
        let _ = floor;
        todo!("T2")
    }
}
