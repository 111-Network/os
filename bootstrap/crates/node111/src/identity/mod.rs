//! Provisional identity (D9/D1): BIP-39 mnemonic → secp256k1 at NIP-06 path
//! `m/44'/1237'/0'/0/0` (k256 schnorr). Final crypto is Layer 2's call; the swap
//! seam is this directory + its readers.
//!
//! On disk (`identity/`, mode 0700, owner node111 — the caller chowns):
//!   `secret.hex` 0600 · `npub` 0644 · `mnemonic` 0600 — written as an atomic
//!   trio (built in a temp sibling dir, then renamed into place). Never
//!   overwritten: a second `generate`/`recover` over a live trio refuses.
//!
//! Boot integrity: `load` re-derives and cross-checks the whole trio. Any
//! inconsistency quarantines the dir (`identity.quarantine-<epoch>`) and
//! returns `IdentityError::Corrupt` — never a silent re-key; explicit
//! `111 identity recover` is the only way forward.

pub mod bip32;
pub mod nip49;

use bip39::{Language, Mnemonic};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

/// NIP-06 derivation path (account 0).
pub const NIP06_PATH: &str = "m/44'/1237'/0'/0/0";

const SECRET_FILE: &str = "secret.hex";
const NPUB_FILE: &str = "npub";
const MNEMONIC_FILE: &str = "mnemonic";

/// Public handle: the npub (bech32). Secret material never leaves the trio files.
#[derive(Debug, Clone)]
pub struct Identity {
    pub npub: String,
}

/// Distinguishable failure classes — the daemon maps these to distinct
/// dashboard/`/api/status` error states.
#[derive(Debug)]
pub enum IdentityError {
    /// No identity directory (fresh machine before install finishes).
    Missing,
    /// Trio unreadable/inconsistent → quarantined. Recovery = `recover`.
    Corrupt(String),
    /// Refused to overwrite an existing identity.
    Exists,
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdentityError::Missing => write!(f, "identity directory missing"),
            IdentityError::Corrupt(why) => write!(f, "identity corrupt (quarantined): {why}"),
            IdentityError::Exists => write!(f, "identity exists — never overwritten"),
        }
    }
}

impl std::error::Error for IdentityError {}

/// OS entropy via getrandom(2), falling back to /dev/urandom (old kernels).
/// Loops until the buffer is full; errors only if both sources fail.
pub(crate) fn os_entropy(buf: &mut [u8]) -> anyhow::Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        let ret = unsafe {
            libc::getrandom(
                buf[filled..].as_mut_ptr() as *mut libc::c_void,
                buf.len() - filled,
                0,
            )
        };
        if ret < 0 {
            break; // fall back below
        }
        filled += ret as usize;
    }
    if filled == buf.len() {
        return Ok(());
    }
    let mut f = std::fs::File::open("/dev/urandom")?;
    f.read_exact(&mut buf[filled..])?;
    Ok(())
}

/// Full derivation chain: words → (NFKD-normalized) seed → NIP-06 key → npub.
fn derive(words: &str) -> anyhow::Result<(Mnemonic, [u8; 32], String)> {
    let normalized: String = unicode_normalization::UnicodeNormalization::nfkd(words).collect();
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, &normalized)
        .map_err(|e| anyhow::anyhow!("invalid BIP-39 mnemonic: {e}"))?;
    let seed = mnemonic.to_seed_normalized("");
    let secret = bip32::derive_private_key(&seed, NIP06_PATH)?;
    let npub = npub_of(&secret)?;
    Ok((mnemonic, secret, npub))
}

/// bech32 `npub` of the x-only (BIP-340 schnorr) public key.
fn npub_of(secret: &[u8; 32]) -> anyhow::Result<String> {
    let signing = k256::schnorr::SigningKey::from_slice(secret)
        .map_err(|e| anyhow::anyhow!("invalid secp256k1 secret: {e}"))?;
    let x_only = signing.verifying_key().to_bytes();
    let hrp = bech32::Hrp::parse("npub").expect("constant hrp is valid");
    bech32::encode::<bech32::Bech32>(hrp, &x_only).map_err(|e| anyhow::anyhow!("bech32: {e}"))
}

fn hex_of(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn trio_exists(dir: &Path) -> bool {
    dir.join(SECRET_FILE).exists()
        || dir.join(NPUB_FILE).exists()
        || dir.join(MNEMONIC_FILE).exists()
}

/// Write the trio atomically: build in `identity.tmp-<pid>` (0700), then rename.
/// Refuses to overwrite any existing trio (checked before AND after via the
/// rename target rule: a non-empty existing dir is never replaced).
fn write_trio(dir: &Path, words: &str, secret: &[u8; 32], npub: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if trio_exists(dir) {
        return Err(IdentityError::Exists.into());
    }
    let parent = dir.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let tmp: PathBuf = parent.join(format!(
        ".{}.tmp-{}",
        dir.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir(&tmp)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o700))?;

    let result = (|| -> anyhow::Result<()> {
        crate::util::fs::atomic_write(
            &tmp.join(SECRET_FILE),
            format!("{}\n", hex_of(secret)).as_bytes(),
            0o600,
        )?;
        crate::util::fs::atomic_write(&tmp.join(NPUB_FILE), format!("{npub}\n").as_bytes(), 0o644)?;
        crate::util::fs::atomic_write(
            &tmp.join(MNEMONIC_FILE),
            format!("{}\n", words).as_bytes(),
            0o600,
        )?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }

    // Rename into place. If `dir` exists it must be empty (fresh installer mkdir)
    // or absent; rename(2) only replaces an EMPTY dir, so a live trio survives.
    if let Err(e) = std::fs::rename(&tmp, dir) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e).map_err(|e2| anyhow::anyhow!("identity rename failed: {e2}"));
    }
    crate::util::fs::fsync_dir(parent)?;
    Ok(())
}

/// Generate a fresh identity (12-word BIP-39, NIP-06 path, atomic trio).
/// Never overwrites. Caller chowns the dir to node111 after generation.
pub fn generate(dir: &Path) -> anyhow::Result<Identity> {
    let mut entropy = [0u8; 16]; // 128 bits → 12 words
    os_entropy(&mut entropy)?;
    let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy)
        .map_err(|e| anyhow::anyhow!("mnemonic from entropy: {e}"))?;
    let words = mnemonic.to_string();
    let (_m, secret, npub) = derive(&words)?;
    write_trio(dir, &words, &secret, &npub)?;
    Ok(Identity { npub })
}

/// Read + verify the trio. Returns (words, secret, npub) or an IdentityError.
fn read_verified(dir: &Path) -> Result<(String, [u8; 32], String), IdentityError> {
    let read = |name: &str| -> Result<String, IdentityError> {
        std::fs::read_to_string(dir.join(name))
            .map(|s| s.trim().to_string())
            .map_err(|e| IdentityError::Corrupt(format!("{name} unreadable: {e}")))
    };
    if !dir.exists() {
        return Err(IdentityError::Missing);
    }
    if !trio_exists(dir) {
        return Err(IdentityError::Missing);
    }
    let words = read(MNEMONIC_FILE)?;
    let secret_hex = read(SECRET_FILE)?;
    let npub_file = read(NPUB_FILE)?;

    let (mnemonic, secret, npub) =
        derive(&words).map_err(|e| IdentityError::Corrupt(format!("mnemonic chain: {e}")))?;
    let _ = mnemonic;
    if hex_of(&secret) != secret_hex {
        return Err(IdentityError::Corrupt(
            "secret.hex does not match mnemonic".into(),
        ));
    }
    if npub != npub_file {
        return Err(IdentityError::Corrupt("npub does not match secret".into()));
    }
    Ok((words, secret, npub))
}

/// Load + integrity-check. Corruption ⇒ quarantine dir + `Corrupt` (never
/// silent re-key; `recover` is the explicit path).
pub fn load(dir: &Path) -> anyhow::Result<Identity> {
    match read_verified(dir) {
        Ok((_, _, npub)) => Ok(Identity { npub }),
        Err(IdentityError::Corrupt(why)) => {
            let quarantine = dir.with_file_name(format!(
                "identity.quarantine-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            ));
            let _ = std::fs::rename(dir, &quarantine);
            Err(IdentityError::Corrupt(why).into())
        }
        Err(e) => Err(e.into()),
    }
}

/// Regenerate from user-supplied BIP-39 words (explicit recovery). Refuses to
/// overwrite a live trio; a quarantined or absent identity is fine.
pub fn recover(dir: &Path, words: &str) -> anyhow::Result<Identity> {
    let (_m, secret, npub) = derive(words)?;
    write_trio(dir, words, &secret, &npub)?;
    Ok(Identity { npub })
}

/// `identity backup`: return the mnemonic words (local CLI only, never HTTP).
/// Refuses to back up a corrupt trio.
pub fn backup(dir: &Path) -> anyhow::Result<String> {
    let (words, _, _) = read_verified(dir)?;
    Ok(words)
}

/// `identity export`: NIP-49 ncryptsec with an interactive confirmed passphrase.
/// Refuses non-tty stdin (a piped passphrase would be scriptable key exfil).
pub fn export_ncryptsec(dir: &Path) -> anyhow::Result<String> {
    anyhow::ensure!(
        unsafe { libc::isatty(0) } == 1,
        "identity export requires an interactive terminal (passphrase is never read from a pipe)"
    );
    let (_, secret, _) = read_verified(dir)?;
    let pass = read_secret_interactive("ncryptsec passphrase: ")?;
    let pass2 = read_secret_interactive("confirm passphrase: ")?;
    anyhow::ensure!(pass == pass2, "passphrases do not match");
    anyhow::ensure!(!pass.is_empty(), "empty passphrase refused");
    nip49::encrypt(&secret, &pass)
}

/// Read a line from the tty with echo disabled (termios ECHO toggle, restored
/// on every path).
fn read_secret_interactive(prompt: &str) -> anyhow::Result<String> {
    use std::io::Write;
    eprint!("{prompt}");
    std::io::stderr().flush()?;

    let mut old: libc::termios = unsafe { std::mem::zeroed() };
    anyhow::ensure!(
        unsafe { libc::tcgetattr(0, &mut old) } == 0,
        "tcgetattr failed"
    );
    let mut new = old;
    new.c_lflag &= !libc::ECHO;
    unsafe { libc::tcsetattr(0, libc::TCSANOW, &new) };

    let mut line = String::new();
    let read_result = std::io::stdin().read_line(&mut line);

    unsafe { libc::tcsetattr(0, libc::TCSANOW, &old) };
    eprintln!(); // newline after the hidden input
    read_result?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    // Published NIP-06 vectors (nostr-protocol/nips 06.md).
    const V1_WORDS: &str =
        "leader monkey parrot ring guide accident before fence cannon height naive bean";
    const V1_SECRET: &str = "7f7ff03d123792d6ac594bfa67bf6d0c0ab55b6b1fdb6249303fe861f1ccba9a";
    const V1_NPUB: &str = "npub1zutzeysacnf9rru6zqwmxd54mud0k44tst6l70ja5mhv8jjumytsd2x7nu";
    const V2_WORDS: &str = "what bleak badge arrange retreat wolf trade produce cricket blur garlic valid proud rude strong choose busy staff weather area salt hollow arm fade";
    const V2_SECRET: &str = "c15d739894c81a2fcfd3a2df85a0d2c0dbc47a280d092799f144d73d7ae78add";
    const V2_NPUB: &str = "npub16sdj9zv4f8sl85e45vgq9n7nsgt5qphpvmf7vk8r5hhvmdjxx4es8rq74h";

    #[test]
    fn nip06_vectors() {
        let (_, s1, n1) = derive(V1_WORDS).unwrap();
        assert_eq!(hex_of(&s1), V1_SECRET);
        assert_eq!(n1, V1_NPUB);
        let (_, s2, n2) = derive(V2_WORDS).unwrap();
        assert_eq!(hex_of(&s2), V2_SECRET);
        assert_eq!(n2, V2_NPUB);
    }

    fn mode(p: &Path) -> u32 {
        p.metadata().unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn recover_writes_trio_with_perms() {
        let dir = tempfile::tempdir().unwrap();
        let id_dir = dir.path().join("identity");
        let id = recover(&id_dir, V1_WORDS).unwrap();
        assert_eq!(id.npub, V1_NPUB);
        assert_eq!(mode(&id_dir), 0o700);
        assert_eq!(mode(&id_dir.join(SECRET_FILE)), 0o600);
        assert_eq!(mode(&id_dir.join(NPUB_FILE)), 0o644);
        assert_eq!(mode(&id_dir.join(MNEMONIC_FILE)), 0o600);
        // load round-trip
        let loaded = load(&id_dir).unwrap();
        assert_eq!(loaded.npub, V1_NPUB);
        // backup returns the words
        assert_eq!(backup(&id_dir).unwrap(), V1_WORDS);
    }

    #[test]
    fn never_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let id_dir = dir.path().join("identity");
        recover(&id_dir, V1_WORDS).unwrap();
        let err = recover(&id_dir, V2_WORDS).unwrap_err();
        assert!(err.to_string().contains("never overwritten"), "{err}");
        // trio untouched
        assert_eq!(load(&id_dir).unwrap().npub, V1_NPUB);
    }

    #[test]
    fn corrupt_trio_quarantines_and_never_rekeys() {
        let dir = tempfile::tempdir().unwrap();
        let id_dir = dir.path().join("identity");
        recover(&id_dir, V1_WORDS).unwrap();
        // corrupt the secret
        std::fs::write(id_dir.join(SECRET_FILE), "00dead\n").unwrap();
        let err = load(&id_dir).unwrap_err();
        assert!(err.to_string().contains("corrupt"), "{err}");
        // dir quarantined aside, no silent replacement
        assert!(!id_dir.exists());
        let quarantined: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("identity.quarantine-")
            })
            .collect();
        assert_eq!(quarantined.len(), 1);
        // recover is possible after quarantine
        let id = recover(&id_dir, V1_WORDS).unwrap();
        assert_eq!(id.npub, V1_NPUB);
    }

    #[test]
    fn missing_identity_is_missing_not_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let err = load(&dir.path().join("identity")).unwrap_err();
        assert!(err.to_string().contains("missing"), "{err}");
    }

    #[test]
    fn export_refuses_pipes() {
        // Tests run without a tty on stdin → export must refuse.
        let dir = tempfile::tempdir().unwrap();
        let id_dir = dir.path().join("identity");
        recover(&id_dir, V1_WORDS).unwrap();
        if unsafe { libc::isatty(0) } == 1 {
            return; // interactive dev shell: pipe-refusal not exercisable here
        }
        let err = export_ncryptsec(&id_dir).unwrap_err();
        assert!(err.to_string().contains("interactive terminal"), "{err}");
    }
}
