//! Layer 7 §4.2: the link key (row D5).
//!
//! A hub's TLS 1.3 raw public key for paired-hub links (§4.1, §4.3):
//!
//! - **Random, per node.** 32 bytes from the OS CSPRNG (`getrandom`), drawn
//!   at the first pairing ([`LinkKey::load_or_mint`], the pairing path's
//!   call, D6). Never seed-derived and never the identity key: nothing here
//!   reads `identity/` or the recovery words (L2 §6.6; L5 §8 "Link keys are
//!   random"; §14 rule 7).
//! - **Stored** in `<root>/secrets/link-key`, 0600, inside the 0700
//!   `secrets/` directory Layer 6 created (`daemon/transport/config.rs`,
//!   `SECRETS_DIR`; created here with the same `ensure_dir` when absent). The
//!   file holds the 32-byte seed as 64 lowercase hex characters and a
//!   newline; it is read through `O_NOFOLLOW`, must be a regular file of at
//!   most [`MAX_FILE`] bytes, and a wrong mode is repaired to 0600, as the
//!   transport's own secrets are.
//! - **On no API.** Only the SPKI ([`LinkKey::spki`], 44 B of DER, and its
//!   71-character base32 form for the `111PAIR:` bundle, §4.6 step 1) leaves
//!   this module; [`LinkKey`]'s `Debug` prints the SPKI only, and nothing
//!   here serialises the seed anywhere but the file.
//! - **In no backup.** The backup set is the recovery words (and the
//!   descriptor) only: `identity::backup` → id111 `BackupArtifacts`
//!   (`NI/identity/mod.rs`, L2 §8.3). `secrets/` is not in it, so a restore
//!   onto a new device has no link key and mints one at its next pairing.
//! - **Re-minted after a restore** (§4.2): [`discard`] removes the key so the
//!   next pairing mints a fresh one; the far hubs then re-pin it from a fresh
//!   bundle. `111 identity restore` (`NI/cli.rs`, row D5b) calls it right
//!   after a successful restore; its error is logged, not fatal (the
//!   identity is already restored).
//!
//! Nothing here runs, opens or allocates until a caller asks: the TLS wrap
//! (`relay::tls`) reads the key at its first TLS handshake and again only
//! when the file changed (one `lstat` per handshake against the stamp it
//! kept, and [`generation`] for a mint or discard made in this process), and
//! the pairing path mints it. No process, timer, thread or open file.

use crate::daemon::fabric::settings::LinkSpki;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use rustls::sign::{CertifiedKey, SigningKey};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

/// The key's file name inside `<root>/secrets/` (§4.2).
pub const LINK_KEY_FILE: &str = "link-key";
/// Largest link-key file read (the file is 65 bytes; anything bigger is not
/// ours and is refused, never read whole).
pub const MAX_FILE: u64 = 256;
/// The Ed25519 seed length.
const SEED_LEN: usize = 32;
/// PKCS#8 v1 (RFC 5958) of an Ed25519 private key (RFC 8410 §7): this fixed
/// 16-byte prefix, then the 32-byte seed. `ring` (through rustls) parses it
/// and derives the public key.
const PKCS8_ED25519_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

/// Mints and discards run one at a time in this process, so two pairings
/// started together never show two different keys (only the daemon mints).
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Per key file (`<root>/secrets/link-key`), bumped by every mint and every
/// [`discard`] of it in this process: the TLS wrap's cache key beside the
/// file's stamp (a change by another process is seen through the stamp
/// alone). One entry per node root (one in the daemon).
static GENERATIONS: Mutex<BTreeMap<PathBuf, u64>> = Mutex::new(BTreeMap::new());

fn bump(root: &Path) {
    let mut g = GENERATIONS.lock().unwrap_or_else(|e| e.into_inner());
    let n = g.entry(path(root)).or_insert(0);
    *n = n.wrapping_add(1);
}

/// The in-process change counter of the link key under `root` (bumped by a
/// mint and by [`discard`] of that file; 0 before either); equality only,
/// never an ordering. A key under another root never moves it.
pub fn generation(root: &Path) -> u64 {
    let key = path(root);
    let g = GENERATIONS.lock().unwrap_or_else(|e| e.into_inner());
    g.get(&key).copied().unwrap_or(0)
}

/// Why the link key could not be read or minted.
#[derive(Debug)]
pub enum LinkKeyError {
    /// The file or `secrets/` could not be read or written: unreadable, a
    /// symlink, not a regular file, too large, or an I/O error. Nothing is
    /// minted over such a file (a silent re-key would break every pair).
    Io(std::io::Error),
    /// The file was read whole and does not hold a 32-byte hex seed.
    Invalid,
    /// The OS CSPRNG failed, or the key was refused by the TLS library.
    Crypto(String),
}

impl std::fmt::Display for LinkKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkKeyError::Io(e) => write!(f, "link key: {e}"),
            LinkKeyError::Invalid => write!(f, "link key: the file does not hold a 32-byte key"),
            LinkKeyError::Crypto(e) => write!(f, "link key: {e}"),
        }
    }
}

impl std::error::Error for LinkKeyError {}

impl From<std::io::Error> for LinkKeyError {
    fn from(e: std::io::Error) -> Self {
        LinkKeyError::Io(e)
    }
}

/// This hub's link key (§4.2): the signing key and its SPKI.
pub struct LinkKey {
    key: Arc<dyn SigningKey>,
    spki: LinkSpki,
}

impl std::fmt::Debug for LinkKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The SPKI is public; the key never prints.
        write!(f, "LinkKey({})", self.spki.text())
    }
}

/// `<root>/secrets`.
fn secrets_dir(root: &Path) -> PathBuf {
    crate::daemon::transport::config::secrets_dir(root)
}

/// `<root>/secrets/link-key`.
pub fn path(root: &Path) -> PathBuf {
    secrets_dir(root).join(LINK_KEY_FILE)
}

impl LinkKey {
    /// The key at `<root>/secrets/link-key`, or `Ok(None)` when there is none
    /// (no pairing yet, or [`discard`]ed). Reads only: nothing is minted or
    /// created; a wrong file mode is repaired to 0600. The TLS wrap's call.
    pub fn load(root: &Path) -> Result<Option<LinkKey>, LinkKeyError> {
        match read_seed(root)? {
            Stored::Absent => Ok(None),
            Stored::Seed(seed) => LinkKey::from_seed(&seed).map(Some),
            Stored::Invalid => Err(LinkKeyError::Invalid),
        }
    }

    /// The key, minted from the OS CSPRNG when none is stored (§4.2: at the
    /// first pairing; the pairing path's call). A file read whole that does
    /// not hold a key is replaced (logged); any other read error is returned
    /// and nothing is minted.
    pub fn load_or_mint(root: &Path) -> Result<LinkKey, LinkKeyError> {
        let _one = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        match read_seed(root)? {
            Stored::Seed(seed) => return LinkKey::from_seed(&seed),
            Stored::Invalid => eprintln!(
                "111 relay: {} does not hold a link key; minting a new one (paired hubs re-pin \
                 it from a fresh bundle)",
                path(root).display()
            ),
            Stored::Absent => {}
        }
        crate::daemon::transport::interfaces::ensure_dir(&secrets_dir(root), true)
            .map_err(|e| LinkKeyError::Io(std::io::Error::other(e.to_string())))?;
        let mut seed = Zeroizing::new([0u8; SEED_LEN]);
        getrandom::getrandom(&mut seed[..])
            .map_err(|e| LinkKeyError::Crypto(format!("getrandom: {e}")))?;
        let key = LinkKey::from_seed(&seed)?;
        let mut text = Zeroizing::new(String::with_capacity(2 * SEED_LEN + 1));
        for b in seed.iter() {
            text.push(hex_digit(b >> 4));
            text.push(hex_digit(b & 0x0f));
        }
        text.push('\n');
        let written = crate::util::fs::atomic_write(&path(root), text.as_bytes(), 0o600);
        // Bumped whatever the outcome: a failed rename may still have moved
        // the file (the wrap then re-reads it, which costs one load).
        bump(root);
        written.map_err(|e| LinkKeyError::Io(std::io::Error::other(format!("{e:#}"))))?;
        Ok(key)
    }

    /// The key from its 32-byte seed (PKCS#8 v1 through rustls's `ring`
    /// provider). Private: a seed only ever comes from the CSPRNG or the file.
    fn from_seed(seed: &[u8; SEED_LEN]) -> Result<LinkKey, LinkKeyError> {
        let mut der = Zeroizing::new(Vec::with_capacity(PKCS8_ED25519_PREFIX.len() + SEED_LEN));
        der.extend_from_slice(&PKCS8_ED25519_PREFIX);
        der.extend_from_slice(seed);
        let key =
            rustls::crypto::ring::sign::any_eddsa_type(&PrivatePkcs8KeyDer::from(der.as_slice()))
                .map_err(|e| LinkKeyError::Crypto(e.to_string()))?;
        let spki = key
            .public_key()
            .and_then(|s| LinkSpki::from_der(s.as_ref()))
            .ok_or_else(|| LinkKeyError::Crypto("no Ed25519 SPKI".into()))?;
        Ok(LinkKey { key, spki })
    }

    /// The SPKI (44 B DER, §4.2): what the far hub pins.
    pub fn spki(&self) -> &LinkSpki {
        &self.spki
    }

    /// The SPKI as the `111PAIR:` bundle carries it (base32, 71 characters,
    /// §4.6 step 1).
    pub fn text(&self) -> String {
        self.spki.text()
    }

    /// The RFC 7250 identity rustls presents: the SPKI as the one
    /// "certificate" and the signing key (§4.3). Crate-only: the TLS wrap's.
    pub(crate) fn certified(&self) -> Arc<CertifiedKey> {
        Arc::new(CertifiedKey::new(
            vec![CertificateDer::from(self.spki.as_der().to_vec())],
            Arc::clone(&self.key),
        ))
    }
}

/// Remove the link key (§4.2: re-minted after a restore; the next pairing
/// mints a fresh one). `Ok(true)` when a key was removed, `Ok(false)` when
/// there was none. A symlink at the path is removed, never followed.
pub fn discard(root: &Path) -> std::io::Result<bool> {
    let _one = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let removed = std::fs::remove_file(path(root));
    bump(root);
    match removed {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn hex_digit(n: u8) -> char {
    char::from(b"0123456789abcdef"[usize::from(n & 0x0f)])
}

fn unhex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// What `secrets/link-key` holds.
enum Stored {
    /// No file (or no `secrets/`).
    Absent,
    /// A file read whole that holds no seed.
    Invalid,
    /// The seed.
    Seed(Zeroizing<[u8; SEED_LEN]>),
}

/// The stored seed, or an error for a file that cannot be read whole (§4.2;
/// the transport's secret-file rules: `O_NOFOLLOW`, regular, size-capped, the
/// mode repaired to 0600, `secrets/` itself no symlink).
fn read_seed(root: &Path) -> Result<Stored, LinkKeyError> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let dir = secrets_dir(root);
    match std::fs::symlink_metadata(&dir) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
            return Err(LinkKeyError::Io(std::io::Error::other(format!(
                "{} is not a directory",
                dir.display()
            ))))
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Stored::Absent),
        Err(e) => return Err(e.into()),
    }
    let p = dir.join(LINK_KEY_FILE);
    let f = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&p)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Stored::Absent),
        Err(e) => return Err(e.into()),
    };
    let meta = f.metadata()?;
    if !meta.is_file() {
        return Err(LinkKeyError::Io(std::io::Error::other(format!(
            "{} is not a regular file",
            p.display()
        ))));
    }
    if meta.len() > MAX_FILE {
        return Err(LinkKeyError::Io(std::io::Error::other(format!(
            "{} is too large",
            p.display()
        ))));
    }
    if meta.permissions().mode() & 0o777 != 0o600 {
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    let mut buf = Zeroizing::new(Vec::with_capacity(MAX_FILE as usize));
    f.take(MAX_FILE).read_to_end(&mut buf)?;
    let text = buf.trim_ascii();
    if text.len() != 2 * SEED_LEN {
        return Ok(Stored::Invalid);
    }
    let mut seed = Zeroizing::new([0u8; SEED_LEN]);
    for (i, pair) in text.chunks_exact(2).enumerate() {
        match (unhex(pair[0]), unhex(pair[1])) {
            (Some(h), Some(l)) => seed[i] = (h << 4) | l,
            _ => return Ok(Stored::Invalid),
        }
    }
    Ok(Stored::Seed(seed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::fabric::settings::{ED25519_SPKI_PREFIX, LINK_SPKI_LEN, LINK_SPKI_TEXT_LEN};
    use std::os::unix::fs::PermissionsExt;

    fn mode(p: &Path) -> u32 {
        std::fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777
    }

    /// §4.2: nothing exists before the first pairing, and reading creates
    /// nothing (no `secrets/`, no file).
    #[test]
    fn load_reads_only() {
        let d = tempfile::tempdir().unwrap();
        assert!(LinkKey::load(d.path()).unwrap().is_none());
        assert!(!secrets_dir(d.path()).exists(), "nothing created");
        std::fs::create_dir(secrets_dir(d.path())).unwrap();
        assert!(LinkKey::load(d.path()).unwrap().is_none());
        assert!(!path(d.path()).exists());
    }

    /// §4.2: the first pairing mints a key into `secrets/link-key` (0600 in
    /// the 0700 `secrets/`); later reads give the same key; its SPKI is 44 B
    /// of Ed25519 DER and its text 71 characters.
    #[test]
    fn mint_once_then_load_the_same_key() {
        let d = tempfile::tempdir().unwrap();
        let k = LinkKey::load_or_mint(d.path()).unwrap();
        assert_eq!(mode(&secrets_dir(d.path())), 0o700);
        assert_eq!(mode(&path(d.path())), 0o600);
        assert_eq!(k.spki().as_der().len(), LINK_SPKI_LEN);
        assert_eq!(k.spki().as_der()[..12], ED25519_SPKI_PREFIX);
        assert_eq!(k.text().len(), LINK_SPKI_TEXT_LEN);
        assert_eq!(LinkSpki::from_text(&k.text()), Some(*k.spki()));
        let again = LinkKey::load(d.path()).unwrap().unwrap();
        assert_eq!(again.spki(), k.spki());
        let minted_again = LinkKey::load_or_mint(d.path()).unwrap();
        assert_eq!(minted_again.spki(), k.spki(), "never re-minted while valid");
        let file = std::fs::read_to_string(path(d.path())).unwrap();
        assert_eq!(file.len(), 65);
        assert!(file.ends_with('\n'));
    }

    /// §4.2: random per node: two nodes never share a key, and the SPKI is
    /// the public half of the stored seed (RFC 8032 test vector 1).
    #[test]
    fn keys_are_random_and_the_spki_is_the_seeds() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let ka = LinkKey::load_or_mint(a.path()).unwrap();
        let kb = LinkKey::load_or_mint(b.path()).unwrap();
        assert_ne!(ka.spki(), kb.spki());
        // RFC 8032 §7.1 TEST 1: secret 9d61b19d…7f60 ⇒ public d75a9801…511a.
        std::fs::write(
            path(a.path()),
            "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60\n",
        )
        .unwrap();
        let k = LinkKey::load(a.path()).unwrap().unwrap();
        let mut want = ED25519_SPKI_PREFIX.to_vec();
        want.extend_from_slice(&[
            0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64,
            0x07, 0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68,
            0xf7, 0x07, 0x51, 0x1a,
        ]);
        assert_eq!(k.spki().as_der().as_slice(), want.as_slice());
    }

    /// The key never prints: `Debug` shows the SPKI text only.
    #[test]
    fn debug_shows_the_spki_only() {
        let d = tempfile::tempdir().unwrap();
        let k = LinkKey::load_or_mint(d.path()).unwrap();
        let seed = std::fs::read_to_string(path(d.path())).unwrap();
        let shown = format!("{k:?}");
        assert_eq!(shown, format!("LinkKey({})", k.text()));
        assert!(!shown.contains(seed.trim()));
    }

    /// A file read whole that holds no key: `load` refuses it, the pairing
    /// path replaces it. A wrong mode is repaired to 0600.
    #[test]
    fn an_invalid_file_is_refused_then_replaced_at_pairing() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(secrets_dir(d.path())).unwrap();
        for bad in ["", "zz", &"0".repeat(63), &"g".repeat(64), &"0".repeat(66)] {
            std::fs::write(path(d.path()), bad).unwrap();
            std::fs::set_permissions(path(d.path()), std::fs::Permissions::from_mode(0o644))
                .unwrap();
            assert!(
                matches!(LinkKey::load(d.path()), Err(LinkKeyError::Invalid)),
                "{bad:?}"
            );
            assert_eq!(mode(&path(d.path())), 0o600, "mode repaired");
        }
        let k = LinkKey::load_or_mint(d.path()).unwrap();
        assert_eq!(LinkKey::load(d.path()).unwrap().unwrap().spki(), k.spki());
    }

    /// The transport's secret-file rules: a symlink (file or `secrets/`), a
    /// directory and an oversized file are errors, and nothing is minted
    /// over them.
    #[test]
    fn symlinks_odd_files_and_oversize_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let s = secrets_dir(d.path());
        std::fs::create_dir(&s).unwrap();
        let elsewhere = d.path().join("elsewhere");
        std::fs::write(&elsewhere, "9d".repeat(32)).unwrap();
        std::os::unix::fs::symlink(&elsewhere, path(d.path())).unwrap();
        assert!(matches!(LinkKey::load(d.path()), Err(LinkKeyError::Io(_))));
        assert!(matches!(
            LinkKey::load_or_mint(d.path()),
            Err(LinkKeyError::Io(_))
        ));
        assert_eq!(
            std::fs::read_to_string(&elsewhere).unwrap(),
            "9d".repeat(32)
        );
        std::fs::remove_file(path(d.path())).unwrap();

        std::fs::create_dir(path(d.path())).unwrap();
        assert!(matches!(LinkKey::load(d.path()), Err(LinkKeyError::Io(_))));
        std::fs::remove_dir(path(d.path())).unwrap();

        std::fs::write(path(d.path()), vec![b'0'; MAX_FILE as usize + 1]).unwrap();
        assert!(matches!(LinkKey::load(d.path()), Err(LinkKeyError::Io(_))));
        assert!(matches!(
            LinkKey::load_or_mint(d.path()),
            Err(LinkKeyError::Io(_))
        ));
        std::fs::remove_file(path(d.path())).unwrap();

        let other = tempfile::tempdir().unwrap();
        std::fs::remove_dir(&s).unwrap();
        std::os::unix::fs::symlink(other.path(), &s).unwrap();
        assert!(matches!(LinkKey::load(d.path()), Err(LinkKeyError::Io(_))));
        assert!(LinkKey::load_or_mint(d.path()).is_err());
        assert_eq!(std::fs::read_dir(other.path()).unwrap().count(), 0);
    }

    /// §4.2: after a restore the key is discarded and the next pairing mints
    /// a new one.
    #[test]
    fn discard_then_the_next_pairing_mints_a_new_key() {
        let d = tempfile::tempdir().unwrap();
        assert!(!discard(d.path()).unwrap());
        let old = LinkKey::load_or_mint(d.path()).unwrap();
        assert!(discard(d.path()).unwrap());
        assert!(LinkKey::load(d.path()).unwrap().is_none());
        let new = LinkKey::load_or_mint(d.path()).unwrap();
        assert_ne!(new.spki(), old.spki());
    }

    /// The in-process change counter moves on every mint and discard (the
    /// TLS wrap's cache key, row D5b) and not on a load.
    #[test]
    fn mint_and_discard_bump_the_generation() {
        let d = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        assert_eq!(generation(d.path()), 0);
        LinkKey::load_or_mint(d.path()).unwrap();
        assert_eq!(generation(d.path()), 1, "a mint bumps it");
        LinkKey::load(d.path()).unwrap().unwrap();
        LinkKey::load_or_mint(d.path()).unwrap(); // a load, no mint
        assert_eq!(generation(d.path()), 1, "a load does not");
        LinkKey::load_or_mint(other.path()).unwrap();
        assert!(discard(other.path()).unwrap());
        assert_eq!(generation(d.path()), 1, "another root's key never moves it");
        assert!(discard(d.path()).unwrap());
        assert_eq!(generation(d.path()), 2, "a discard bumps it");
        assert!(!discard(d.path()).unwrap());
        assert_eq!(generation(d.path()), 3, "a discard of nothing too");
    }

    /// The RFC 7250 identity rustls presents is the SPKI itself.
    #[test]
    fn the_certified_key_is_the_raw_spki() {
        let d = tempfile::tempdir().unwrap();
        let k = LinkKey::load_or_mint(d.path()).unwrap();
        let c = k.certified();
        assert_eq!(c.cert.len(), 1);
        assert_eq!(c.cert[0].as_ref(), k.spki().as_der().as_slice());
        let presented = c.key.public_key().unwrap();
        assert_eq!(presented.as_ref(), k.spki().as_der().as_slice());
    }
}
