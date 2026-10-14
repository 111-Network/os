//! §10.3 / §11 — the `blobs` table: refcounts and pruning facts for the
//! content-addressed files in `fabric/blobs/` ([`crate::blobs`]).
//!
//! Ordering keeps the DB and the directory consistent across crashes:
//! files are written (and fsynced) **before** their row commits; rows are
//! deleted (or marked `present = 0`) **before** their files are removed.
//! A crash therefore leaves at most an orphan directory, which
//! [`Store::open`] sweeps — never a row that points at a missing file.
//! New bulk passes §10.4 admission first (`StoreFull` when refused).
//!
//! Keys: a private object's `object_key` is generated here
//! ([`Store::blob_put_private`]) and never accepted from a caller for
//! encryption (key reuse would be nonce reuse). A private object may be held
//! **without** its key — a relay or hub carrying ciphertext it cannot read
//! (§10.3, §11); its row has `public = 0` and no `object_key`, and
//! [`Store::blob_read_private`] answers [`StoreError::NoObjectKey`].

use rusqlite::{params, OptionalExtension};
use zeroize::Zeroizing;

use crate::blobs::{
    ciphertext_len, BlobError, BlobHash, ChunkGroup, ObjectKey, Written, TMP_PREFIX,
};

use super::rows::Class;
use super::{id32, next_local_seq, to_i64, to_u64, Durability, Result, Store, StoreError};

/// A `blobs` row (the object key is never part of it; see
/// [`Store::blob_object_key`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobRow {
    pub hash: BlobHash,
    /// Addressed size (ciphertext for private objects).
    pub size: u64,
    /// Data + outboards on disk.
    pub disk_bytes: u64,
    pub refcount: u64,
    pub public: bool,
    pub class: Class,
    pub local_seq: u64,
    pub pinned: bool,
    pub refetchable: bool,
    pub superseded: bool,
    /// `false` once pruned (re-fetchable/superseded objects keep their row).
    pub present: bool,
}

/// Disk estimate for admission: data + both outboards.
fn disk_estimate(size: u64) -> u64 {
    size + ChunkGroup::K1.outboard_size(size) + ChunkGroup::K16.outboard_size(size)
}

impl Store {
    /// Store a public object (stored plain, §10.3) with one reference.
    /// `refetchable` marks public chunks pruning step 1 may evict.
    pub fn blob_put_public(
        &mut self,
        bytes: &[u8],
        class: Class,
        refetchable: bool,
    ) -> Result<BlobHash> {
        self.admit_bulk(class, disk_estimate(bytes.len() as u64))?;
        let w = self.blobs.write(bytes)?;
        self.blob_record(&w, true, None, class, refetchable)
    }

    /// Encrypt and store a private object under a **fresh random**
    /// `object_key` generated here (§10.3: XChaCha20-Poly1305, 64 KiB chunks;
    /// addressed by ciphertext hash). Returns the address and the key (the
    /// key also stays in the row for [`Store::blob_read_private`]).
    pub fn blob_put_private(
        &mut self,
        plaintext: &[u8],
        class: Class,
    ) -> Result<(BlobHash, ObjectKey)> {
        self.blob_put_private_rng(plaintext, class, |k| {
            getrandom::getrandom(k).map_err(|_| BlobError::Entropy)
        })
    }

    /// [`Store::blob_put_private`] with the entropy source as a closure that
    /// fills the 32 key bytes (getrandom in production; tests inject
    /// failure). The key is still minted here, never handed in.
    pub(super) fn blob_put_private_rng(
        &mut self,
        plaintext: &[u8],
        class: Class,
        fill: impl FnOnce(&mut [u8]) -> Result<(), BlobError>,
    ) -> Result<(BlobHash, ObjectKey)> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        fill(&mut bytes[..])?;
        let key = ObjectKey::from_bytes(*bytes);
        self.admit_bulk(class, disk_estimate(ciphertext_len(plaintext.len() as u64)))?;
        let w = self
            .blobs
            .write_private_with_key(plaintext, u64::MAX, &key)?;
        let hash = self.blob_record(&w, false, Some(&key), class, false)?;
        Ok((hash, key))
    }

    /// Store an object received under a known address (custody of an
    /// already-encrypted object, a fetched public object): refused unless
    /// `BLAKE3(bytes) == expected`. A private object may come with its key
    /// (the recipient, from the `OBJ_ADVERT` / attachment ref) or without it
    /// (a relay or hub holding ciphertext keyless); a key arriving later for
    /// a keyless row is kept. A public object never stores a key.
    pub fn blob_put_received(
        &mut self,
        expected: &BlobHash,
        bytes: &[u8],
        public: bool,
        key: Option<&ObjectKey>,
        class: Class,
        refetchable: bool,
    ) -> Result<BlobHash> {
        self.admit_bulk(class, disk_estimate(bytes.len() as u64))?;
        let w = self.blobs.write_verified(bytes, u64::MAX, expected)?;
        self.blob_record(
            &w,
            public,
            if public { None } else { key },
            class,
            refetchable,
        )
    }

    fn admit_bulk(&mut self, class: Class, bytes: u64) -> Result<()> {
        if self.admit(class, bytes)? {
            Ok(())
        } else {
            Err(StoreError::StoreFull)
        }
    }

    fn blob_record(
        &mut self,
        w: &Written,
        public: bool,
        key: Option<&ObjectKey>,
        class: Class,
        refetchable: bool,
    ) -> Result<BlobHash> {
        let size = to_i64(w.size, "blob size")?;
        let disk = to_i64(w.disk_bytes, "blob disk bytes")?;
        self.with_tx(Durability::Full, |tx| {
            let seq = to_i64(next_local_seq(tx)?, "local_seq")?;
            // A key learned later fills a keyless private row; a stored key
            // is never replaced, and a public row never gains one.
            let updated = tx.execute(
                "UPDATE blobs SET refcount = refcount + 1, present = 1, disk_bytes = ?2,
                                  local_seq = ?3,
                                  object_key = CASE WHEN public = 0
                                                    THEN coalesce(object_key, ?4)
                                                    ELSE object_key END
                 WHERE hash = ?1",
                params![&w.hash.0[..], disk, seq, key.map(|k| k.as_bytes().to_vec())],
            )?;
            if updated == 0 {
                tx.execute(
                    "INSERT INTO blobs (hash, size, disk_bytes, refcount, public, object_key,
                                        class, local_seq, refetchable)
                     VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        &w.hash.0[..],
                        size,
                        disk,
                        public,
                        key.map(|k| k.as_bytes().to_vec()),
                        class as i64,
                        seq,
                        refetchable
                    ],
                )?;
            }
            Ok(())
        })?;
        Ok(w.hash)
    }

    /// One blob row.
    pub fn blob(&self, hash: &BlobHash) -> Result<Option<BlobRow>> {
        let raw = self
            .conn
            .query_row(
                "SELECT hash, size, disk_bytes, refcount, public, class, local_seq, pinned,
                        refetchable, superseded, present
                 FROM blobs WHERE hash = ?1",
                [&hash.0[..]],
                |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, bool>(4)?,
                        r.get::<_, i64>(5)?,
                        r.get::<_, i64>(6)?,
                        r.get::<_, bool>(7)?,
                        r.get::<_, bool>(8)?,
                        r.get::<_, bool>(9)?,
                        r.get::<_, bool>(10)?,
                    ))
                },
            )
            .optional()?;
        let Some((h, size, disk, rc, public, class, local, pinned, refetch, sup, present)) = raw
        else {
            return Ok(None);
        };
        Ok(Some(BlobRow {
            hash: BlobHash(id32(h, "blobs.hash")?),
            size: to_u64(size, "blobs.size")?,
            disk_bytes: to_u64(disk, "blobs.disk_bytes")?,
            refcount: to_u64(rc, "blobs.refcount")?,
            public,
            class: Class::from_i64(class)?,
            local_seq: to_u64(local, "blobs.local_seq")?,
            pinned,
            refetchable: refetch,
            superseded: sup,
            present,
        }))
    }

    /// The `object_key` of a private object (zeroized on drop); `None` for a
    /// public object or a ciphertext-only (keyless) one.
    pub fn blob_object_key(&self, hash: &BlobHash) -> Result<Option<ObjectKey>> {
        let k: Option<Option<Vec<u8>>> = self
            .conn
            .query_row(
                "SELECT object_key FROM blobs WHERE hash = ?1",
                [&hash.0[..]],
                |r| r.get(0),
            )
            .optional()?;
        match k.flatten() {
            None => Ok(None),
            Some(v) => {
                let v = Zeroizing::new(v);
                let arr = <[u8; 32]>::try_from(v.as_slice())
                    .map_err(|_| StoreError::Corrupt("blobs.object_key"))?;
                Ok(Some(ObjectKey::from_bytes(arr)))
            }
        }
    }

    fn require_present(&self, hash: &BlobHash) -> Result<BlobRow> {
        let row = self.blob(hash)?.ok_or(StoreError::NotFound)?;
        if !row.present {
            return Err(StoreError::Blob(crate::blobs::BlobError::Missing));
        }
        Ok(row)
    }

    /// The addressed bytes (plaintext of a public object, ciphertext of a
    /// private one), verified against the address; `max_len` bounds the read.
    pub fn blob_read(&self, hash: &BlobHash, max_len: u64) -> Result<Vec<u8>> {
        self.require_present(hash)?;
        Ok(self.blobs.read(hash, max_len)?)
    }

    /// The plaintext of a private object (address verified, every chunk
    /// authenticated).
    pub fn blob_read_private(&self, hash: &BlobHash, max_len: u64) -> Result<Zeroizing<Vec<u8>>> {
        let row = self.require_present(hash)?;
        if row.public {
            return Err(StoreError::Invalid("object is public"));
        }
        let key = self.blob_object_key(hash)?.ok_or(StoreError::NoObjectKey)?;
        Ok(self.blobs.read_private(hash, &key, max_len)?)
    }

    /// Add a reference; returns the new count.
    pub fn blob_incref(&mut self, hash: &BlobHash) -> Result<u64> {
        self.with_tx(Durability::Normal, |tx| {
            let n = tx.execute(
                "UPDATE blobs SET refcount = refcount + 1 WHERE hash = ?1",
                [&hash.0[..]],
            )?;
            if n == 0 {
                return Err(StoreError::NotFound);
            }
            let rc: i64 = tx.query_row(
                "SELECT refcount FROM blobs WHERE hash = ?1",
                [&hash.0[..]],
                |r| r.get(0),
            )?;
            to_u64(rc, "blobs.refcount")
        })
    }

    /// Drop a reference; at zero the row goes, then the files. Returns the
    /// new count.
    pub fn blob_decref(&mut self, hash: &BlobHash) -> Result<u64> {
        let rc = self.with_tx(Durability::Normal, |tx| {
            let rc: Option<i64> = tx
                .query_row(
                    "SELECT refcount FROM blobs WHERE hash = ?1",
                    [&hash.0[..]],
                    |r| r.get(0),
                )
                .optional()?;
            let rc = to_u64(rc.ok_or(StoreError::NotFound)?, "blobs.refcount")?;
            let new = rc.saturating_sub(1);
            if new == 0 {
                tx.execute("DELETE FROM blobs WHERE hash = ?1", [&hash.0[..]])?;
            } else {
                tx.execute(
                    "UPDATE blobs SET refcount = ?2 WHERE hash = ?1",
                    params![&hash.0[..], to_i64(new, "refcount")?],
                )?;
            }
            Ok(new)
        })?;
        if rc == 0 {
            self.blobs.remove(hash)?;
        }
        Ok(rc)
    }

    /// Pin or unpin an object (pinned items are never pruned).
    pub fn blob_set_pinned(&mut self, hash: &BlobHash, pinned: bool) -> Result<()> {
        self.blob_flag(hash, "pinned", pinned)
    }

    /// Mark an object superseded (below the Layer 1 floor, or a fully
    /// fetched delta): pruning step 2 may evict it.
    pub fn blob_mark_superseded(&mut self, hash: &BlobHash) -> Result<()> {
        self.blob_flag(hash, "superseded", true)
    }

    fn blob_flag(&mut self, hash: &BlobHash, col: &'static str, v: bool) -> Result<()> {
        let n = self.conn.execute(
            &format!("UPDATE blobs SET {col} = ?2 WHERE hash = ?1"),
            params![&hash.0[..], v],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    /// At open: remove temp directories, directories without a row, and the
    /// files of rows marked `present = 0`. Unknown names are left alone.
    pub(crate) fn sweep_blob_orphans(&mut self) -> Result<()> {
        for (name, parsed) in self.blobs.entries()? {
            if name.starts_with(TMP_PREFIX) {
                self.blobs.remove_entry(&name)?;
                continue;
            }
            let Some(hash) = parsed else { continue };
            let present: Option<bool> = self
                .conn
                .query_row(
                    "SELECT present FROM blobs WHERE hash = ?1",
                    [&hash.0[..]],
                    |r| r.get(0),
                )
                .optional()?;
            if present != Some(true) {
                self.blobs.remove(&hash)?;
            }
        }
        Ok(())
    }
}
