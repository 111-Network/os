//! Provisional identity (D9/D1): BIP-39 mnemonic → secp256k1 at NIP-06 path
//! `m/44'/1237'/0'/0/0` (k256 schnorr). Final crypto is Layer 2's call; the swap
//! seam is this directory + its readers.

pub mod bip32;
pub mod nip49;

use std::path::Path;

/// Public handle: the npub (bech32). Secret material never leaves the trio files.
#[derive(Debug, Clone)]
pub struct Identity {
    pub npub: String,
}

/// Generate a fresh identity: 12-word BIP-39 mnemonic, NIP-06 derivation, atomic
/// trio write (`secret.hex` 0600, `npub` 0644, `mnemonic` 0600) into `dir` (0700).
/// Never overwrites an existing trio.
pub fn generate(dir: &Path) -> anyhow::Result<Identity> {
    let _ = dir;
    todo!("T3")
}

/// Load + integrity-check an existing identity. Corruption ⇒ quarantine dir,
/// distinct error state, never silent re-key (`recover` is the explicit path).
pub fn load(dir: &Path) -> anyhow::Result<Identity> {
    let _ = dir;
    todo!("T3")
}

/// Regenerate from user-supplied BIP-39 words (explicit recovery).
pub fn recover(dir: &Path, words: &str) -> anyhow::Result<Identity> {
    let _ = (dir, words);
    todo!("T3")
}

/// `identity backup`: print the mnemonic words (local CLI only, never HTTP).
pub fn backup(dir: &Path) -> anyhow::Result<String> {
    let _ = dir;
    todo!("T3")
}

/// `identity export`: NIP-49 ncryptsec with interactive confirmed passphrase;
/// refuses non-tty stdin.
pub fn export_ncryptsec(dir: &Path) -> anyhow::Result<String> {
    let _ = dir;
    todo!("T3")
}
