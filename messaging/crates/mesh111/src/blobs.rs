//! §11 / §10.3 — the blob store's file layer: content-addressed objects under
//! `<fabric>/blobs/<blake3 hex>/` (0700), each holding
//!
//! - `data` — the addressed bytes: plaintext for public objects, ciphertext
//!   for private ones (§10.3: "addressed by **ciphertext** hash, so relays and
//!   hubs verify what they carry without reading it");
//! - `ob1k` / `ob16k` — `bao-tree` post-order outboards at chunk group 1 KiB
//!   (LoRa/BLE, 6.25 %) and 16 KiB (TCP, 0.39 %). The BLAKE3 root is the same
//!   for both groups and equals `blake3::hash(data)` (§11), so a chunk fetched
//!   over LoRa and one over TCP verify against the same address.
//!
//! Private objects use the `object_key` framing (§10.3): XChaCha20-Poly1305
//! over 64 KiB plaintext chunks, nonce = chunk index (u64 LE) ‖ 16 zero bytes,
//! AD = chunk index (u64 LE) ‖ `last_chunk` (1 B: 1 on the final chunk, else
//! 0), each chunk followed by its 16 B tag. The key is random per object and
//! generated **inside** this layer ([`BlobDir::write_private_from`],
//! `Store::blob_put_private`) — never supplied by a caller — so nonces never
//! repeat under one key. The ciphertext hash is the address, so truncation or
//! reordering changes the address; independently of that, a cut at a 64 KiB
//! boundary fails decryption (the new final chunk was sealed with
//! `last_chunk = 0`).
//!
//! v1 builds the store, addressing and outboards only — no wire transfer
//! (`OBJ_*` kinds are v1.1, §5 Tier 2). Refcounts, classes and pruning live
//! in the store's `blobs` table (`store::blobs`).

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use bao_tree::io::outboard::PostOrderOutboard;
use bao_tree::io::sync::outboard_post_order;
use bao_tree::{BaoTree, BlockSize};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305};
use zeroize::{Zeroize, Zeroizing};

/// §10.3 private-object AEAD plaintext chunk (64 KiB).
pub const AEAD_CHUNK: usize = 64 * 1024;
/// XChaCha20-Poly1305 tag length.
pub const AEAD_TAG: usize = 16;
/// One ciphertext chunk on disk: plaintext chunk ‖ tag.
pub const AEAD_CT_CHUNK: usize = AEAD_CHUNK + AEAD_TAG;

/// Data file name inside a blob directory.
const DATA: &str = "data";
/// Prefix of in-progress blob directories (swept at store open).
pub(crate) const TMP_PREFIX: &str = ".tmp-";

/// A BLAKE3 root (§11): the content address of a blob.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlobHash(pub [u8; 32]);

impl BlobHash {
    /// BLAKE3 of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        BlobHash(*blake3::hash(bytes).as_bytes())
    }

    /// Lower-case hex, the directory name.
    pub fn to_hex(&self) -> String {
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }

    /// Parse a 64-char lower/upper-case hex name (directory sweep).
    pub fn from_hex(s: &str) -> Option<Self> {
        blake3::Hash::from_hex(s)
            .ok()
            .map(|h| BlobHash(*h.as_bytes()))
    }
}

impl std::fmt::Debug for BlobHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BlobHash({})", self.to_hex())
    }
}

impl std::fmt::Display for BlobHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// §11 chunk groups. Runtime choice per link; the root never depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkGroup {
    /// 1 KiB (one BLAKE3 chunk): LoRa / BLE.
    K1,
    /// 16 KiB: TCP.
    K16,
}

impl ChunkGroup {
    /// Both groups, in the order their outboards are written.
    pub const ALL: [ChunkGroup; 2] = [ChunkGroup::K1, ChunkGroup::K16];

    /// The `bao-tree` block size.
    pub const fn block_size(self) -> BlockSize {
        match self {
            ChunkGroup::K1 => BlockSize::from_chunk_log(0),
            ChunkGroup::K16 => BlockSize::from_chunk_log(4),
        }
    }

    /// Outboard file name inside the blob directory.
    pub const fn file_name(self) -> &'static str {
        match self {
            ChunkGroup::K1 => "ob1k",
            ChunkGroup::K16 => "ob16k",
        }
    }

    /// Outboard size for an object of `size` bytes.
    pub fn outboard_size(self, size: u64) -> u64 {
        BaoTree::new(size, self.block_size()).outboard_size()
    }
}

/// Typed blob failures (never a panic on input).
#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("object exceeds the {limit}-byte limit")]
    TooLarge { limit: u64 },
    #[error("content does not match its address")]
    HashMismatch,
    #[error("ciphertext is malformed or fails authentication")]
    Decrypt,
    #[error("blob not present")]
    Missing,
    #[error("entropy unavailable")]
    Entropy,
}

/// The per-object key of a private object (§3.3 attachment ref
/// `object_key(32)`, §10.3). Zeroized on drop; `Debug` hides the bytes.
#[derive(Clone)]
pub struct ObjectKey(Zeroizing<[u8; 32]>);

impl ObjectKey {
    /// Wrap key bytes (from an `OBJ_ADVERT` / attachment ref, or the store).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        let mut b = bytes;
        let k = ObjectKey(Zeroizing::new(b));
        b.zeroize();
        k
    }

    /// A fresh random key (getrandom; typed failure, never a weak fallback).
    pub fn generate() -> Result<Self, BlobError> {
        let mut k = Zeroizing::new([0u8; 32]);
        getrandom::getrandom(&mut k[..]).map_err(|_| BlobError::Entropy)?;
        Ok(ObjectKey(k))
    }

    /// The key bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for ObjectKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ObjectKey(..)")
    }
}

/// §10.3 nonce: chunk index (u64 LE) ‖ 16 zero bytes.
fn chunk_nonce(index: u64) -> [u8; 24] {
    let mut n = [0u8; 24];
    n[..8].copy_from_slice(&index.to_le_bytes());
    n
}

/// Chunk associated data: chunk index (u64 LE) ‖ `last_chunk` (1 B).
fn chunk_ad(index: u64, last: bool) -> [u8; 9] {
    let mut ad = [0u8; 9];
    ad[..8].copy_from_slice(&index.to_le_bytes());
    ad[8] = u8::from(last);
    ad
}

/// Ciphertext length of a `plain_len`-byte object: at least one chunk (an
/// empty object is one empty chunk = one tag).
pub fn ciphertext_len(plain_len: u64) -> u64 {
    let chunk = AEAD_CHUNK as u64;
    let chunks = plain_len.div_ceil(chunk).max(1);
    plain_len + chunks * AEAD_TAG as u64
}

/// Plaintext length of a canonical ciphertext of `ct_len` bytes, or
/// `Decrypt` when no canonical framing yields that length.
pub fn plaintext_len(ct_len: u64) -> Result<u64, BlobError> {
    if ct_len < AEAD_TAG as u64 {
        return Err(BlobError::Decrypt);
    }
    let ct_chunk = AEAD_CT_CHUNK as u64;
    let chunks = ct_len.div_ceil(ct_chunk);
    let last = ct_len - (chunks - 1) * ct_chunk;
    // Every chunk carries a tag; only a single-chunk object may be empty.
    if last < AEAD_TAG as u64 || (chunks > 1 && last == AEAD_TAG as u64) {
        return Err(BlobError::Decrypt);
    }
    Ok(ct_len - chunks * AEAD_TAG as u64)
}

/// Encrypt one plaintext chunk.
fn seal_chunk(
    cipher: &XChaCha20Poly1305,
    index: u64,
    last: bool,
    chunk: &[u8],
) -> Result<Vec<u8>, BlobError> {
    let nonce = chunk_nonce(index);
    let ad = chunk_ad(index, last);
    cipher
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: chunk,
                aad: &ad,
            },
        )
        .map_err(|_| BlobError::Decrypt)
}

/// §10.3 — encrypt a whole private object under `key` (the reference the
/// streaming writer is checked against; production writes go through
/// [`BlobDir::write_private_from`], which generates the key).
#[cfg(test)]
pub(crate) fn encrypt_object(key: &ObjectKey, plaintext: &[u8]) -> Result<Vec<u8>, BlobError> {
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
    let mut out = Vec::with_capacity(
        usize::try_from(ciphertext_len(plaintext.len() as u64)).map_err(|_| {
            BlobError::TooLarge {
                limit: usize::MAX as u64,
            }
        })?,
    );
    if plaintext.is_empty() {
        out.extend_from_slice(&seal_chunk(&cipher, 0, true, &[])?);
        return Ok(out);
    }
    let count = plaintext.len().div_ceil(AEAD_CHUNK);
    for (i, chunk) in plaintext.chunks(AEAD_CHUNK).enumerate() {
        out.extend_from_slice(&seal_chunk(&cipher, i as u64, i + 1 == count, chunk)?);
    }
    Ok(out)
}

/// §10.3 — decrypt a whole private object. Rejects non-canonical framing
/// and any chunk that fails authentication (no partial plaintext escapes).
pub fn decrypt_object(key: &ObjectKey, ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>, BlobError> {
    let plain_len = plaintext_len(ciphertext.len() as u64)?;
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
    let mut out = Zeroizing::new(Vec::with_capacity(
        usize::try_from(plain_len).map_err(|_| BlobError::Decrypt)?,
    ));
    let count = ciphertext.len().div_ceil(AEAD_CT_CHUNK);
    for (i, chunk) in ciphertext.chunks(AEAD_CT_CHUNK).enumerate() {
        let nonce = chunk_nonce(i as u64);
        let ad = chunk_ad(i as u64, i + 1 == count);
        let mut pt = cipher
            .decrypt(
                (&nonce).into(),
                Payload {
                    msg: chunk,
                    aad: &ad,
                },
            )
            .map_err(|_| BlobError::Decrypt)?;
        out.extend_from_slice(&pt);
        pt.zeroize();
    }
    Ok(out)
}

/// What a write produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Written {
    /// The BLAKE3 root of the stored bytes (the address).
    pub hash: BlobHash,
    /// Stored (addressed) size: plaintext for public, ciphertext for private.
    pub size: u64,
    /// Bytes on disk: data + both outboards.
    pub disk_bytes: u64,
}

/// `<fabric>/blobs/` — the directory of content-addressed objects.
#[derive(Debug, Clone)]
pub struct BlobDir {
    root: PathBuf,
}

impl BlobDir {
    /// Open (creating 0700 if needed) the blob directory at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, BlobError> {
        let root = root.into();
        if !root.exists() {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&root)?;
        }
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        Ok(BlobDir { root })
    }

    /// The directory root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<root>/<hex>/`.
    pub fn path(&self, hash: &BlobHash) -> PathBuf {
        self.root.join(hash.to_hex())
    }

    /// `<root>/<hex>/data`.
    pub fn data_path(&self, hash: &BlobHash) -> PathBuf {
        self.path(hash).join(DATA)
    }

    /// `<root>/<hex>/ob1k|ob16k`.
    pub fn outboard_path(&self, hash: &BlobHash, group: ChunkGroup) -> PathBuf {
        self.path(hash).join(group.file_name())
    }

    /// Whether a complete object directory exists for `hash`.
    pub fn exists(&self, hash: &BlobHash) -> bool {
        self.data_path(hash).is_file()
    }

    /// Store `bytes` as-is (a public object, or ciphertext already framed).
    pub fn write(&self, bytes: &[u8]) -> Result<Written, BlobError> {
        self.write_from(bytes, u64::MAX)
    }

    /// Stream `reader` into the store as-is, refusing more than `max_len`
    /// bytes (checked while reading, before anything is published).
    pub fn write_from(&self, reader: impl Read, max_len: u64) -> Result<Written, BlobError> {
        self.write_with(reader, max_len, None, None)
    }

    /// Stream `reader` into the store, requiring the result to hash to
    /// `expected` (a received object; nothing is published on mismatch).
    pub fn write_verified(
        &self,
        reader: impl Read,
        max_len: u64,
        expected: &BlobHash,
    ) -> Result<Written, BlobError> {
        self.write_with(reader, max_len, None, Some(expected))
    }

    /// Encrypt `reader` under a **fresh random** `object_key` (§10.3
    /// framing) while streaming it into the store; the address is the
    /// ciphertext hash. `max_len` bounds the plaintext. Returns the key — the
    /// only way to obtain one, so a key is never reused across objects.
    pub fn write_private_from(
        &self,
        reader: impl Read,
        max_len: u64,
    ) -> Result<(Written, ObjectKey), BlobError> {
        let key = ObjectKey::generate()?;
        let w = self.write_private_with_key(reader, max_len, &key)?;
        Ok((w, key))
    }

    /// Encrypt under a key generated by this crate (never a caller's).
    pub(crate) fn write_private_with_key(
        &self,
        reader: impl Read,
        max_len: u64,
        key: &ObjectKey,
    ) -> Result<Written, BlobError> {
        self.write_with(reader, max_len, Some(key), None)
    }

    fn write_with(
        &self,
        mut reader: impl Read,
        max_len: u64,
        key: Option<&ObjectKey>,
        expected: Option<&BlobHash>,
    ) -> Result<Written, BlobError> {
        let tmp = self.make_tmp_dir()?;
        let res = (|| {
            let data_tmp = tmp.join(DATA);
            let mut hasher = blake3::Hasher::new();
            let mut size = 0u64;
            {
                let f = create_0600(&data_tmp)?;
                let mut w = BufWriter::new(f);
                let cipher = key.map(|k| XChaCha20Poly1305::new(k.as_bytes().into()));
                // Two buffers: a chunk is sealed only once the next read tells
                // whether it is the last (`last_chunk` in the AD). A short read
                // is EOF (`read_full`), so no lookahead follows it; an object
                // of whole chunks ends on a full chunk (canonical framing), an
                // empty object is one empty final chunk.
                let mut cur = Zeroizing::new(vec![0u8; AEAD_CHUNK]);
                let mut next = Zeroizing::new(vec![0u8; AEAD_CHUNK]);
                let mut got = read_full(&mut reader, &mut cur)?;
                let mut plain_total = got as u64;
                let mut index = 0u64;
                loop {
                    if plain_total > max_len {
                        return Err(BlobError::TooLarge { limit: max_len });
                    }
                    let next_got = if got == AEAD_CHUNK {
                        read_full(&mut reader, &mut next)?
                    } else {
                        0
                    };
                    plain_total = plain_total.saturating_add(next_got as u64);
                    if plain_total > max_len {
                        return Err(BlobError::TooLarge { limit: max_len });
                    }
                    let last = next_got == 0;
                    match &cipher {
                        Some(c) => {
                            let ct = seal_chunk(c, index, last, &cur[..got])?;
                            hasher.update(&ct);
                            w.write_all(&ct)?;
                            size += ct.len() as u64;
                        }
                        None => {
                            hasher.update(&cur[..got]);
                            w.write_all(&cur[..got])?;
                            size += got as u64;
                        }
                    }
                    if last {
                        break;
                    }
                    std::mem::swap(&mut cur, &mut next);
                    got = next_got;
                    index += 1;
                }
                let f = w.into_inner().map_err(|e| e.into_error())?;
                f.sync_all()?;
            }
            let hash = BlobHash(*hasher.finalize().as_bytes());
            if let Some(exp) = expected {
                if exp != &hash {
                    return Err(BlobError::HashMismatch);
                }
            }
            // §11 outboards at both chunk groups; each root must equal the
            // streaming BLAKE3 (identical across groups by construction —
            // checked, fail closed).
            let mut disk = size;
            for group in ChunkGroup::ALL {
                let src = BufReader::new(File::open(&data_tmp)?);
                let ob_path = tmp.join(group.file_name());
                let mut w = BufWriter::new(create_0600(&ob_path)?);
                let root =
                    outboard_post_order(src, BaoTree::new(size, group.block_size()), &mut w)?;
                if root.as_bytes() != &hash.0 {
                    return Err(BlobError::HashMismatch);
                }
                let f = w.into_inner().map_err(|e| e.into_error())?;
                f.sync_all()?;
                disk += f.metadata()?.len();
            }
            fsync_dir(&tmp)?;
            Ok(Written {
                hash,
                size,
                disk_bytes: disk,
            })
        })();
        match res {
            Ok(w) => {
                let dst = self.path(&w.hash);
                if self.exists(&w.hash) {
                    // Content-addressed: an existing object is the same bytes.
                    let _ = fs::remove_dir_all(&tmp);
                } else {
                    if dst.exists() {
                        // A stale partial directory (no `data`): replace it.
                        fs::remove_dir_all(&dst)?;
                    }
                    fs::rename(&tmp, &dst)?;
                    fsync_dir(&self.root)?;
                }
                Ok(w)
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&tmp);
                Err(e)
            }
        }
    }

    /// Read a whole object, verifying it against its address. `max_len`
    /// bounds the read (size checked before reading).
    pub fn read(&self, hash: &BlobHash, max_len: u64) -> Result<Vec<u8>, BlobError> {
        let path = self.data_path(hash);
        let f = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(BlobError::Missing),
            Err(e) => return Err(e.into()),
        };
        let len = f.metadata()?.len();
        if len > max_len {
            return Err(BlobError::TooLarge { limit: max_len });
        }
        let mut out = Vec::with_capacity(
            usize::try_from(len).map_err(|_| BlobError::TooLarge { limit: max_len })?,
        );
        f.take(len).read_to_end(&mut out)?;
        if BlobHash::of(&out) != *hash {
            return Err(BlobError::HashMismatch);
        }
        Ok(out)
    }

    /// Read and decrypt a private object (address checked, then every chunk
    /// authenticated). `max_len` bounds the ciphertext read.
    pub fn read_private(
        &self,
        hash: &BlobHash,
        key: &ObjectKey,
        max_len: u64,
    ) -> Result<Zeroizing<Vec<u8>>, BlobError> {
        let ct = self.read(hash, max_len)?;
        decrypt_object(key, &ct)
    }

    /// The stored outboard of `hash` at `group`, ready for `bao-tree`'s
    /// verified streaming (v1.1 transfer). The tree size is the data size.
    pub fn outboard(
        &self,
        hash: &BlobHash,
        group: ChunkGroup,
    ) -> Result<PostOrderOutboard<File>, BlobError> {
        let size = match fs::metadata(self.data_path(hash)) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(BlobError::Missing),
            Err(e) => return Err(e.into()),
        };
        let tree = BaoTree::new(size, group.block_size());
        let data = File::open(self.outboard_path(hash, group))?;
        if data.metadata()?.len() != tree.outboard_size() {
            return Err(BlobError::HashMismatch);
        }
        Ok(PostOrderOutboard {
            root: blake3::Hash::from_bytes(hash.0),
            tree,
            data,
        })
    }

    /// Bytes on disk for `hash` (data + outboards); 0 when absent.
    pub fn disk_bytes(&self, hash: &BlobHash) -> Result<u64, BlobError> {
        let mut total = 0;
        for name in [
            DATA,
            ChunkGroup::K1.file_name(),
            ChunkGroup::K16.file_name(),
        ] {
            match fs::metadata(self.path(hash).join(name)) {
                Ok(m) => total += m.len(),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(total)
    }

    /// Remove the object directory (idempotent).
    pub fn remove(&self, hash: &BlobHash) -> Result<(), BlobError> {
        match fs::remove_dir_all(self.path(hash)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Every entry name under the root: `(name, parsed hash if it is one)`.
    pub(crate) fn entries(&self) -> Result<Vec<(String, Option<BlobHash>)>, BlobError> {
        let mut out = Vec::new();
        for e in fs::read_dir(&self.root)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            let parsed = if name.len() == 64 {
                BlobHash::from_hex(&name)
            } else {
                None
            };
            out.push((name, parsed));
        }
        Ok(out)
    }

    /// Remove an arbitrary entry under the root by name (sweep).
    pub(crate) fn remove_entry(&self, name: &str) -> Result<(), BlobError> {
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            return Ok(());
        }
        let p = self.root.join(name);
        let res = if p.is_dir() {
            fs::remove_dir_all(&p)
        } else {
            fs::remove_file(&p)
        };
        match res {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    fn make_tmp_dir(&self) -> Result<PathBuf, BlobError> {
        let mut r = [0u8; 8];
        getrandom::getrandom(&mut r).map_err(|_| BlobError::Entropy)?;
        let name = format!("{TMP_PREFIX}{}", u64::from_le_bytes(r));
        let p = self.root.join(name);
        DirBuilder::new().mode(0o700).create(&p)?;
        Ok(p)
    }
}

/// Fill `buf` from `r` until full or EOF; returns bytes read.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(got)
}

fn create_0600(p: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(p)
}

fn fsync_dir(p: &Path) -> io::Result<()> {
    File::open(p)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bao_tree::io::outboard::PreOrderMemOutboard;
    use bao_tree::io::round_up_to_chunks;
    use bao_tree::io::sync::{decode_ranges, encode_ranges_validated};
    use bao_tree::ByteRanges;

    fn sample(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    fn dir() -> (tempfile::TempDir, BlobDir) {
        let t = tempfile::tempdir().unwrap();
        let d = BlobDir::open(t.path().join("blobs")).unwrap();
        (t, d)
    }

    /// §11: the BLAKE3 root is identical at 1 KiB and 16 KiB chunk groups
    /// and equals `blake3::hash` — for empty, sub-chunk, exact-group and
    /// ragged sizes.
    #[test]
    fn root_identical_across_chunk_groups() {
        for len in [
            0usize,
            1,
            1023,
            1024,
            1025,
            16 * 1024,
            16 * 1024 + 7,
            200_001,
        ] {
            let data = sample(len);
            let r1 = PreOrderMemOutboard::create(&data, ChunkGroup::K1.block_size()).root;
            let r16 = PreOrderMemOutboard::create(&data, ChunkGroup::K16.block_size()).root;
            assert_eq!(r1, r16, "len {len}");
            assert_eq!(r1, blake3::hash(&data), "len {len}");
        }
    }

    /// Outboard sizes: 64 B per 1 KiB group (6.25 %), 64 B per 16 KiB.
    #[test]
    fn outboard_overheads() {
        let size = 1024 * 1024;
        assert_eq!(ChunkGroup::K1.outboard_size(size), (1024 - 1) * 64);
        assert_eq!(ChunkGroup::K16.outboard_size(size), (64 - 1) * 64);
    }

    #[test]
    fn public_round_trip_and_files() {
        let (_t, d) = dir();
        let data = sample(70_000);
        let w = d.write(&data).unwrap();
        assert_eq!(w.hash, BlobHash::of(&data));
        assert_eq!(w.size, data.len() as u64);
        assert_eq!(
            w.disk_bytes,
            data.len() as u64
                + ChunkGroup::K1.outboard_size(w.size)
                + ChunkGroup::K16.outboard_size(w.size)
        );
        assert_eq!(d.disk_bytes(&w.hash).unwrap(), w.disk_bytes);
        assert_eq!(d.read(&w.hash, u64::MAX).unwrap(), data);
        let mode = fs::metadata(d.path(&w.hash)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        let mode = fs::metadata(d.data_path(&w.hash))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        // Idempotent re-write; no temp directories left behind.
        assert_eq!(d.write(&data).unwrap(), w);
        assert!(d
            .entries()
            .unwrap()
            .iter()
            .all(|(n, _)| !n.starts_with(TMP_PREFIX)));
        // Size limit checked before reading.
        assert!(matches!(
            d.read(&w.hash, 10),
            Err(BlobError::TooLarge { .. })
        ));
        d.remove(&w.hash).unwrap();
        assert!(matches!(d.read(&w.hash, u64::MAX), Err(BlobError::Missing)));
    }

    #[test]
    fn corrupted_data_is_refused() {
        let (_t, d) = dir();
        let w = d.write(&sample(5000)).unwrap();
        let p = d.data_path(&w.hash);
        let mut bytes = fs::read(&p).unwrap();
        bytes[17] ^= 1;
        fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&p, &bytes).unwrap();
        assert!(matches!(
            d.read(&w.hash, u64::MAX),
            Err(BlobError::HashMismatch)
        ));
    }

    #[test]
    fn write_limits_and_expected_hash() {
        let (_t, d) = dir();
        let data = sample(3000);
        assert!(matches!(
            d.write_from(&data[..], 2999),
            Err(BlobError::TooLarge { .. })
        ));
        assert!(matches!(
            d.write_verified(&data[..], u64::MAX, &BlobHash([0; 32])),
            Err(BlobError::HashMismatch)
        ));
        let ok = d
            .write_verified(&data[..], u64::MAX, &BlobHash::of(&data))
            .unwrap();
        assert_eq!(ok.hash, BlobHash::of(&data));
        // Nothing was published by the failed writes.
        let names: Vec<_> = d.entries().unwrap().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec![ok.hash.to_hex()]);
    }

    /// §10.3 framing: 64 KiB chunks, LE-index nonces, ciphertext address.
    #[test]
    fn private_round_trip_and_framing() {
        let (_t, d) = dir();
        let mut keys = Vec::new();
        for len in [
            0usize,
            1,
            AEAD_CHUNK - 1,
            AEAD_CHUNK,
            AEAD_CHUNK + 1,
            2 * AEAD_CHUNK,
            3 * AEAD_CHUNK + 5,
        ] {
            let pt = sample(len);
            let (w, key) = d.write_private_from(&pt[..], u64::MAX).unwrap();
            keys.push(*key.as_bytes());
            assert_eq!(w.size, ciphertext_len(len as u64), "len {len}");
            assert_eq!(plaintext_len(w.size).unwrap(), len as u64);
            let ct = d.read(&w.hash, u64::MAX).unwrap();
            // Streaming and whole-object encryption agree byte for byte.
            assert_eq!(ct, encrypt_object(&key, &pt).unwrap(), "len {len}");
            assert_eq!(w.hash, BlobHash::of(&ct));
            assert_ne!(w.hash, BlobHash::of(&pt));
            assert_eq!(
                &d.read_private(&w.hash, &key, u64::MAX).unwrap()[..],
                &pt[..]
            );
        }
        // A fresh key per object, generated inside.
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 7);
        // The same plaintext twice: two keys, two addresses.
        let (a, _) = d.write_private_from(&b"same"[..], u64::MAX).unwrap();
        let (b, _) = d.write_private_from(&b"same"[..], u64::MAX).unwrap();
        assert_ne!(a.hash, b.hash);
        // The plaintext bound holds on the lookahead read too.
        assert!(matches!(
            d.write_private_from(&sample(AEAD_CHUNK + 1)[..], AEAD_CHUNK as u64),
            Err(BlobError::TooLarge { .. })
        ));
    }

    /// Nonce = LE index ‖ zeros; AD = LE index ‖ last_chunk.
    #[test]
    fn private_nonce_and_ad_layout() {
        let key = ObjectKey::from_bytes([7; 32]);
        let pt = sample(AEAD_CHUNK + 10);
        let ct = encrypt_object(&key, &pt).unwrap();
        let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
        let open = |idx: u8, last: u8, bytes: &[u8]| {
            let mut nonce = [0u8; 24];
            nonce[0] = idx;
            let ad = [idx, 0, 0, 0, 0, 0, 0, 0, last];
            cipher.decrypt(
                (&nonce).into(),
                Payload {
                    msg: bytes,
                    aad: &ad,
                },
            )
        };
        let first = open(0, 0, &ct[..AEAD_CT_CHUNK]).unwrap();
        assert_eq!(first, &pt[..AEAD_CHUNK]);
        let second = open(1, 1, &ct[AEAD_CT_CHUNK..]).unwrap();
        assert_eq!(second, &pt[AEAD_CHUNK..]);
        // The flag is bound: the wrong last_chunk byte fails.
        assert!(open(0, 1, &ct[..AEAD_CT_CHUNK]).is_err());
        assert!(open(1, 0, &ct[AEAD_CT_CHUNK..]).is_err());
        // Empty AD (the pre-fix framing) no longer opens anything.
        let mut nonce = [0u8; 24];
        nonce[0] = 1;
        assert!(cipher
            .decrypt((&nonce).into(), &ct[AEAD_CT_CHUNK..])
            .is_err());
    }

    /// A cut exactly at a 64 KiB chunk boundary is canonical framing (the
    /// length check passes) yet fails decryption: the new final chunk was
    /// sealed with `last_chunk = 0`. No address check is involved.
    #[test]
    fn truncation_at_a_chunk_boundary_fails_decryption() {
        let key = ObjectKey::from_bytes([5; 32]);
        let pt = sample(3 * AEAD_CHUNK);
        let ct = encrypt_object(&key, &pt).unwrap();
        for chunks in [1usize, 2] {
            let cut = &ct[..chunks * AEAD_CT_CHUNK];
            assert_eq!(
                plaintext_len(cut.len() as u64).unwrap(),
                (chunks * AEAD_CHUNK) as u64
            );
            assert!(matches!(decrypt_object(&key, cut), Err(BlobError::Decrypt)));
        }
        assert_eq!(&decrypt_object(&key, &ct).unwrap()[..], &pt[..]);
        // An exact multiple of the chunk ends on a full final chunk.
        assert_eq!(ct.len(), 3 * AEAD_CT_CHUNK);
    }

    #[test]
    fn private_tamper_reorder_truncate_wrong_key() {
        let key = ObjectKey::from_bytes([3; 32]);
        let pt = sample(2 * AEAD_CHUNK + 100);
        let ct = encrypt_object(&key, &pt).unwrap();
        // Bit flip.
        let mut bad = ct.clone();
        bad[5] ^= 0x80;
        assert!(matches!(
            decrypt_object(&key, &bad),
            Err(BlobError::Decrypt)
        ));
        // Chunk swap.
        let mut swapped = ct[AEAD_CT_CHUNK..2 * AEAD_CT_CHUNK].to_vec();
        swapped.extend_from_slice(&ct[..AEAD_CT_CHUNK]);
        swapped.extend_from_slice(&ct[2 * AEAD_CT_CHUNK..]);
        assert!(matches!(
            decrypt_object(&key, &swapped),
            Err(BlobError::Decrypt)
        ));
        // Truncation inside a tag and at a chunk boundary.
        assert!(decrypt_object(&key, &ct[..ct.len() - 1]).is_err());
        assert!(decrypt_object(&key, &ct[..AEAD_CT_CHUNK + AEAD_TAG]).is_err());
        assert!(decrypt_object(&key, &[]).is_err());
        // Wrong key.
        assert!(decrypt_object(&ObjectKey::from_bytes([4; 32]), &ct).is_err());
        assert_eq!(&decrypt_object(&key, &ct).unwrap()[..], &pt[..]);
    }

    #[test]
    fn plaintext_len_rejects_noncanonical() {
        assert!(plaintext_len(0).is_err());
        assert!(plaintext_len(15).is_err());
        assert_eq!(plaintext_len(16).unwrap(), 0);
        assert_eq!(
            plaintext_len(AEAD_CT_CHUNK as u64).unwrap(),
            AEAD_CHUNK as u64
        );
        // A trailing empty chunk after a full one is never produced.
        assert!(plaintext_len(AEAD_CT_CHUNK as u64 + 16).is_err());
        assert!(plaintext_len(AEAD_CT_CHUNK as u64 + 15).is_err());
        assert_eq!(
            plaintext_len(AEAD_CT_CHUNK as u64 + 17).unwrap(),
            AEAD_CHUNK as u64 + 1
        );
    }

    /// §11: a range streamed with the 1 KiB outboard (a LoRa carrier) and
    /// one streamed with the 16 KiB outboard (a TCP carrier) both verify
    /// against the one address; the stored outboards reload with that root.
    /// (The stream format follows the carrier's group; the address does not.)
    #[test]
    fn verified_ranges_across_groups_share_the_address() {
        let (_t, d) = dir();
        let data = sample(100_000);
        let w = d.write(&data).unwrap();
        let ranges = round_up_to_chunks(&ByteRanges::from(20_000..41_000));
        let mut decoded = Vec::new();
        for group in ChunkGroup::ALL {
            let ob = d.outboard(&w.hash, group).unwrap();
            assert_eq!(ob.root.as_bytes(), &w.hash.0);
            let mut wire = Vec::new();
            encode_ranges_validated(&data[..], &ob, &ranges, &mut wire).unwrap();
            // The receiver knows only the address and the size.
            let fresh = || bao_tree::io::outboard::PreOrderOutboard {
                root: blake3::Hash::from_bytes(w.hash.0),
                tree: BaoTree::new(w.size, group.block_size()),
                data: Vec::new(),
            };
            let mut rx_ob = fresh();
            let mut out = vec![0u8; data.len()];
            decode_ranges(&wire[..], &ranges, &mut out[..], &mut rx_ob).unwrap();
            assert_eq!(&out[20_480..40_960], &data[20_480..40_960]);
            decoded.push(out);
            // A corrupted byte fails verification against the address.
            let mut bad = wire.clone();
            let last = bad.len() - 1;
            bad[last] ^= 1;
            let mut rx_bad = fresh();
            let mut out2 = vec![0u8; data.len()];
            assert!(decode_ranges(&bad[..], &ranges, &mut out2[..], &mut rx_bad).is_err());
        }
        assert_eq!(decoded[0][20_480..40_960], decoded[1][20_480..40_960]);
    }
}
