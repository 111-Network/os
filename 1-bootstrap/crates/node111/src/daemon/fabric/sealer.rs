//! The identity holder's handle (`os/5-messaging/README.md` §1, §3, §4, §10.2,
//! §18.3): `mesh111`'s [`Sealer`] implemented over the live `id111` keyset
//! that `111d` keeps for its lifetime (L2 §13 signer pattern — the keyset
//! never enters `mesh111`, and no key material reaches any HTTP route).
//!
//! What crosses into `mesh111` and what does not (§18 rule 3):
//!
//! - **Sealing / opening** run here, over I1's caller-owned-inner variants
//!   (`id111::seal::{seal_payload, open_payload}`, L2 amendment A2).
//! - **Pair keys** (§3: `BLAKE2b-256("111/pair/v1" ‖ ss ‖ sender_enc_pk ‖
//!   recipient_enc_pk)`, the static-static ECDH behind it) are derived here
//!   and cached per peer in zeroizing memory; the 32-byte MAC key itself
//!   crosses (the MAC runs in `mesh111`). [`Sealer::forget_pair_key`] drops
//!   (and so zeroizes) both directions for that peer. The cache is bounded
//!   ([`PAIR_CACHE_MAX`]): a full cache is cleared — every entry is
//!   recomputable — so INTRO churn cannot grow it.
//! - **Signatures** prepend the ASCII domain tag (`Keyset::sign_with_tag`,
//!   L2 §6.1).
//! - **Counters** are `id111::counters::CounterStore` over the identity
//!   dir's `counters` file (the daemon is its single runtime writer):
//!   `next_send_counter` (§6.5, batch-committed), `observe_counter` (A11: the
//!   window's verdict, high-water advanced either way — a stale counter
//!   changes nothing), `next_announce_counter` (§6.2 sync-commit; the §4
//!   epoch source, A3) and `announce_last`. A missing `counters` file is the
//!   L2 §8.4 state-absent case: the store opens in restore-pending mode and
//!   every announce/epoch request answers `RestoreFloorRequired` — nothing is
//!   sealed until a floor is set. **Nothing is written either**:
//!   [`IdentitySealer::shutdown_counters`] skips the clean-shutdown commit
//!   while pending (a file with `announce_last = 0` would read as state
//!   present at the next start — a counter reset, not a floor), the stored
//!   announce is not presented ([`AnnounceOutcome::Missing`]), and the host
//!   registers no link (no inbound packet can tick the window). In `111d`
//!   the state is rare: a `counters` file missing at daemon start already
//!   fails the identity integrity check (`crate::identity::load`
//!   quarantines the dir and asks for `111 identity restore`, which writes
//!   a provisional floor), so the handle meets it only when the file goes
//!   missing between that check and the store open. No Layer 2 command sets
//!   a floor on a live identity dir (`111 identity restore` refuses one):
//!   the way out is the node's own `identity/counters` put back (state
//!   present), or a restart into the restore flow — the `fabric` service
//!   row says so.
//! - **Randomness** is the §4 named source, `getrandom()` (typed failure,
//!   never a weak fallback).
//! - **`store_secret`** (§10.2 Mode S, L2 amendment A9) =
//!   `BLAKE2b-256(key = X25519 private key = keyset bytes 32..64, data =
//!   "111/store-secret/v1")`, RFC 7693 keyed mode, computed here; the private
//!   key bytes are copied only into zeroizing buffers.
//!
//! The node's **own announce** (§7: "card requests are answered from stored
//! announce bytes, never minted") is kept in `fabric/announce` (0600): read at
//! start, re-minted only when it is missing, does not verify, names other
//! keys, or carries other `appdata` (§7.3 A8: roles + home-hub hint) than the
//! settings call for — and only when the counter store can issue an announce
//! counter (committed before the announce exists, §6.2).

use id111::announce::Announce;
use id111::counters::{CounterError, CounterStore, Freshness as L2Freshness};
use id111::keyset::{Keyset, NodeId};
use id111::seal::{self, OpenError, SealHeader};
use mesh111::envelope::derive_pair_key;
use mesh111::sealer::{Direction, Freshness, Key32, Sealer, SealerError};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, RwLock};
use zeroize::{Zeroize, Zeroizing};

use crate::identity::Identity;

/// §10.2 / §3.1 domain tag of the Mode S store secret.
pub const STORE_SECRET_TAG: &[u8] = b"111/store-secret/v1";
/// Pair-key cache bound (peers); a full cache is cleared and recomputed.
pub const PAIR_CACHE_MAX: usize = 4096;
/// The stored own announce inside the fabric directory.
pub const ANNOUNCE_FILE: &str = "announce";

/// Both directions of one peer's pair key, zeroized on drop.
#[derive(Default)]
struct PairKeys {
    outbound: Option<Zeroizing<Key32>>,
    inbound: Option<Zeroizing<Key32>>,
}

/// The daemon's [`Sealer`]: the live identity plus its counter store.
pub struct IdentitySealer {
    identity: Identity,
    node_id: NodeId,
    enc_pubkey: Key32,
    counters: Mutex<CounterStore>,
    pairs: Mutex<HashMap<Key32, PairKeys>>,
    announce: RwLock<Vec<u8>>,
}

impl std::fmt::Debug for IdentitySealer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentitySealer")
            .field("node_id", &self.identity.node_id_hex())
            .finish_non_exhaustive()
    }
}

/// How the counter store was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CounterOpen {
    /// State present (L2 §8.4): reused as-is.
    Present,
    /// State absent: no announce, no epoch, nothing sealed until a floor.
    RestorePending,
}

/// Open the identity dir's counter store: present ⇒ `open` (the §6.5
/// unclean-reboot jump applies inside), absent ⇒ restore-pending (nothing is
/// written). Corruption is an error (fail closed, L2 §12 rule 1).
pub fn open_counters(identity_dir: &Path) -> Result<(CounterStore, CounterOpen), CounterError> {
    match CounterStore::open(identity_dir) {
        Ok(c) => Ok((c, CounterOpen::Present)),
        Err(CounterError::NotFound) => Ok((
            CounterStore::restore_pending(identity_dir)?,
            CounterOpen::RestorePending,
        )),
        Err(e) => Err(e),
    }
}

/// §10.2 Mode S over raw private-keyset bytes (`sign seed ‖ X25519
/// scalar`): keyed BLAKE2b-256 with the X25519 half as the key.
pub fn store_secret_from(private_keyset: &[u8; 64]) -> Result<Zeroizing<Key32>, SealerError> {
    use blake2::digest::consts::U32;
    use blake2::digest::{FixedOutput, KeyInit, Update};
    let mut mac = <blake2::Blake2bMac<U32> as KeyInit>::new_from_slice(&private_keyset[32..64])
        .map_err(|_| SealerError::Counters("store-secret KDF refused its key".into()))?;
    Update::update(&mut mac, STORE_SECRET_TAG);
    let mut out = mac.finalize_fixed();
    let mut secret = Zeroizing::new([0u8; 32]);
    secret.copy_from_slice(&out);
    out.zeroize();
    Ok(secret)
}

/// `appdata` v1 (§7.3, L2 amendment A8): `[ver=1][roles u16 LE][reserved]`
/// plus the optional 8-byte truncated home-hub id (a locator only).
pub fn appdata(roles: u16, home_hub: Option<&NodeId>) -> Vec<u8> {
    let r = roles.to_le_bytes();
    let mut out = vec![1, r[0], r[1], 0];
    if let Some(h) = home_hub {
        out.extend_from_slice(&h.as_bytes()[..8]);
    }
    out
}

/// What [`IdentitySealer::ensure_announce`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnounceOutcome {
    /// The stored announce is current.
    Kept,
    /// A new announce was minted (counter committed) and stored.
    Minted,
    /// The stored announce is valid but its `appdata` is out of date and no
    /// counter could be issued (the counter commit failed, or the counter
    /// space is exhausted) — it is used as is. Never while restore-pending.
    Stale,
    /// No usable announce and none could be minted, or the identity holder
    /// is restore-pending (§8.4: even a valid stored announce is not
    /// presented): the node presents no card and no `tcp-lan` link is
    /// registered (§4 restore pending).
    Missing,
}

impl IdentitySealer {
    /// The handle over a loaded identity and its opened counter store.
    pub fn new(identity: Identity, counters: CounterStore) -> IdentitySealer {
        let node_id = identity.node_id;
        let enc_pubkey = identity.keyset().x25519_public();
        IdentitySealer {
            identity,
            node_id,
            enc_pubkey,
            counters: Mutex::new(counters),
            pairs: Mutex::new(HashMap::new()),
            announce: RwLock::new(Vec::new()),
        }
    }

    fn keyset(&self) -> &Keyset {
        self.identity.keyset()
    }

    /// This node's X25519 public key.
    pub fn enc_pubkey(&self) -> Key32 {
        self.enc_pubkey
    }

    /// The identity holder reports `RestoreFloorRequired` (§4).
    pub fn restore_pending(&self) -> bool {
        self.counters
            .lock()
            .map(|c| c.is_restore_pending())
            .unwrap_or(true)
    }

    /// Peers with a cached pair key (diagnostics, tests).
    pub fn cached_pair_keys(&self) -> usize {
        self.pairs.lock().map(|p| p.len()).unwrap_or(0)
    }

    /// Clean-shutdown commit of the counter store (L2 §6.5: the next open
    /// applies no forward jump). Call after the fabric has stopped. While
    /// restore-pending nothing is written (module doc; L2 §8.4 — id111
    /// refuses too) and this returns `Ok`.
    pub fn shutdown_counters(&self) -> Result<(), CounterError> {
        match self.counters.lock() {
            Ok(c) if c.is_restore_pending() => Ok(()),
            Ok(mut c) => c.shutdown(),
            Err(_) => Err(CounterError::Corrupt("counter store lock poisoned")),
        }
    }

    /// Make sure `fabric_dir/announce` holds a verified announce of this
    /// node carrying `appdata` (see the module doc), and keep it as the
    /// bytes [`Sealer::own_announce`] returns. While restore-pending the node
    /// presents no announce at all — not even a valid stored one (§8.4: it
    /// MUST NOT announce until the floor is set) — and answers `Missing`.
    pub fn ensure_announce(&self, fabric_dir: &Path, appdata: &[u8]) -> AnnounceOutcome {
        if self.restore_pending() {
            self.set_announce(Vec::new());
            return AnnounceOutcome::Missing;
        }
        let path = fabric_dir.join(ANNOUNCE_FILE);
        let stored = read_small(&path, id111::announce::MAX_LEN as u64)
            .and_then(|b| self.own_valid(&b).then_some(b));
        if let Some(b) = &stored {
            if Announce::parse(b).is_ok_and(|a| a.appdata() == appdata) {
                self.set_announce(b.clone());
                return AnnounceOutcome::Kept;
            }
        }
        let counter = match self.counters.lock() {
            Ok(mut c) => c.next_announce_counter().ok(),
            Err(_) => None,
        };
        let minted = counter.and_then(|n| Announce::create(self.keyset(), n, appdata).ok());
        match (minted, stored) {
            (Some(a), _) => {
                let bytes = a.encode();
                if let Err(e) = crate::util::fs::atomic_write(&path, &bytes, 0o600) {
                    // Still usable this run (the counter is committed); the
                    // next start mints again.
                    eprintln!("111 fabric: cannot store the announce: {e:#}");
                }
                self.set_announce(bytes);
                AnnounceOutcome::Minted
            }
            (None, Some(old)) => {
                self.set_announce(old);
                AnnounceOutcome::Stale
            }
            (None, None) => AnnounceOutcome::Missing,
        }
    }

    fn own_valid(&self, bytes: &[u8]) -> bool {
        match Announce::parse(bytes) {
            Ok(a) => {
                a.verify().is_ok()
                    && a.node_id() == self.node_id
                    && *a.enc_pubkey() == self.enc_pubkey
            }
            Err(_) => false,
        }
    }

    fn set_announce(&self, bytes: Vec<u8>) {
        if let Ok(mut a) = self.announce.write() {
            *a = bytes;
        }
    }

    fn counters_err(e: CounterError) -> SealerError {
        SealerError::Counters(e.to_string())
    }
}

/// A small regular file (symlinks and oversize refused), or `None`.
fn read_small(path: &Path, max: u64) -> Option<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() || meta.len() > max {
        return None;
    }
    std::fs::read(path).ok()
}

impl Sealer for IdentitySealer {
    fn node_id(&self) -> NodeId {
        self.node_id
    }

    fn own_announce(&self) -> Vec<u8> {
        self.announce.read().map(|a| a.clone()).unwrap_or_default()
    }

    fn seal_payload(
        &self,
        recipient_enc_pubkey: &Key32,
        header: &SealHeader,
        inner: &mut dyn FnMut(&Key32) -> Vec<u8>,
    ) -> Result<Vec<u8>, SealerError> {
        seal::seal_payload(recipient_enc_pubkey, header, |epk| inner(epk))
            .map_err(SealerError::Seal)
    }

    fn open_payload(&self, packet: &[u8]) -> Result<(SealHeader, Key32, Vec<u8>), SealerError> {
        seal::open_payload(self.keyset(), packet).map_err(SealerError::Open)
    }

    fn pair_key(
        &self,
        peer_enc_pubkey: &Key32,
        direction: Direction,
    ) -> Result<Key32, SealerError> {
        let mut cache = self
            .pairs
            .lock()
            .map_err(|_| SealerError::Counters("pair-key cache lock poisoned".into()))?;
        if let Some(k) = cache.get(peer_enc_pubkey).and_then(|p| match direction {
            Direction::Outbound => p.outbound.as_ref(),
            Direction::Inbound => p.inbound.as_ref(),
        }) {
            return Ok(**k);
        }
        let ss = self
            .keyset()
            .ecdh(peer_enc_pubkey)
            .map_err(|e| SealerError::Open(OpenError::Ecdh(e)))?;
        let key = match direction {
            Direction::Outbound => derive_pair_key(&ss, &self.enc_pubkey, peer_enc_pubkey),
            Direction::Inbound => derive_pair_key(&ss, peer_enc_pubkey, &self.enc_pubkey),
        };
        if cache.len() >= PAIR_CACHE_MAX && !cache.contains_key(peer_enc_pubkey) {
            cache.clear(); // every entry is recomputable; dropping zeroizes
        }
        let slot = cache.entry(*peer_enc_pubkey).or_default();
        let out = *key;
        match direction {
            Direction::Outbound => slot.outbound = Some(key),
            Direction::Inbound => slot.inbound = Some(key),
        }
        Ok(out)
    }

    fn forget_pair_key(&self, peer_enc_pubkey: &Key32) {
        if let Ok(mut cache) = self.pairs.lock() {
            // Dropping the entry zeroizes both directions (Zeroizing).
            cache.remove(peer_enc_pubkey);
        }
    }

    fn sign(&self, tag: &[u8], msg: &[u8]) -> [u8; 64] {
        self.keyset().sign_with_tag(tag, msg)
    }

    fn next_send_counter(&self, peer: &NodeId) -> Result<u64, SealerError> {
        let mut c = self
            .counters
            .lock()
            .map_err(|_| SealerError::Counters("counter store lock poisoned".into()))?;
        c.next_send_counter(peer).map_err(Self::counters_err)
    }

    fn observe_counter(&self, peer: &NodeId, counter: u64) -> Result<Freshness, SealerError> {
        let mut c = self
            .counters
            .lock()
            .map_err(|_| SealerError::Counters("counter store lock poisoned".into()))?;
        match c
            .observe_counter(peer, counter)
            .map_err(Self::counters_err)?
        {
            L2Freshness::Fresh => Ok(Freshness::Fresh),
            L2Freshness::Stale => Ok(Freshness::Stale),
        }
    }

    fn next_announce_counter(&self) -> Result<u64, SealerError> {
        let mut c = self
            .counters
            .lock()
            .map_err(|_| SealerError::Counters("counter store lock poisoned".into()))?;
        c.next_announce_counter().map_err(Self::counters_err)
    }

    fn announce_last(&self) -> u64 {
        self.counters.lock().map(|c| c.announce_last()).unwrap_or(0)
    }

    fn random(&self, out: &mut [u8]) -> Result<(), SealerError> {
        getrandom::getrandom(out).map_err(|_| SealerError::Entropy)
    }

    fn store_secret(&self) -> Result<Key32, SealerError> {
        let private = self.keyset().to_bytes();
        let secret = store_secret_from(&private)?;
        Ok(*secret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(dir: &Path) -> Identity {
        crate::identity::generate(dir).expect("fresh identity")
    }

    fn sealer(dir: &Path) -> IdentitySealer {
        let id = identity(&dir.join("identity"));
        let (c, how) = open_counters(&dir.join("identity")).unwrap();
        assert_eq!(how, CounterOpen::Present);
        IdentitySealer::new(id, c)
    }

    #[test]
    fn store_secret_known_answers() {
        // Independent cross-check (Python hashlib, RFC 7693 keyed mode):
        // hashlib.blake2b(b"111/store-secret/v1", key=<keyset[32..64]>,
        //                 digest_size=32).hexdigest()
        let mut ks = [0u8; 64];
        for (i, b) in ks.iter_mut().enumerate() {
            *b = i as u8;
        }
        let s = store_secret_from(&ks).unwrap();
        assert_eq!(
            id111::encoding::hex_encode(&s),
            "19e6ca2b1deb14a764c04f9308690ed936c73a4e2293936fd36aa08908d3cc33"
        );
        let mut ks = [0x42u8; 64];
        ks[..32].fill(0); // only the X25519 half keys the KDF
        let s = store_secret_from(&ks).unwrap();
        assert_eq!(
            id111::encoding::hex_encode(&s),
            "ab6766e4796631710cc96cc1e54d3fdaa3640e04bfcedc8c02f15cd935626f87"
        );
        // and the handle computes exactly this over its own keyset
        let t = tempfile::tempdir().unwrap();
        let a = sealer(t.path());
        let want = store_secret_from(&a.keyset().to_bytes()).unwrap();
        assert_eq!(a.store_secret().unwrap(), *want);
    }

    #[test]
    fn pair_keys_match_both_ways_and_forget_drops_them() {
        let t = tempfile::tempdir().unwrap();
        let a = sealer(&t.path().join("a"));
        let b = sealer(&t.path().join("b"));
        let ab = a.pair_key(&b.enc_pubkey(), Direction::Outbound).unwrap();
        let ba = b.pair_key(&a.enc_pubkey(), Direction::Inbound).unwrap();
        assert_eq!(ab, ba, "A→B outbound = B's inbound from A");
        let ab_in = a.pair_key(&b.enc_pubkey(), Direction::Inbound).unwrap();
        assert_ne!(ab, ab_in, "direction-bound");
        assert_eq!(a.cached_pair_keys(), 1);
        a.forget_pair_key(&b.enc_pubkey());
        assert_eq!(a.cached_pair_keys(), 0);
        // recomputed on demand, same value
        assert_eq!(
            a.pair_key(&b.enc_pubkey(), Direction::Outbound).unwrap(),
            ab
        );
    }

    #[test]
    fn seal_open_round_trip_and_counters() {
        let t = tempfile::tempdir().unwrap();
        let a = sealer(&t.path().join("a"));
        let b = sealer(&t.path().join("b"));
        let header = SealHeader::v1(7);
        let mut seen = None;
        let packet = a
            .seal_payload(&b.enc_pubkey(), &header, &mut |epk| {
                seen = Some(*epk);
                b"inner".to_vec()
            })
            .unwrap();
        let (h, epk, inner) = b.open_payload(&packet).unwrap();
        assert_eq!(h.counter, 7);
        assert_eq!(Some(epk), seen);
        assert_eq!(inner, b"inner");
        assert!(a.open_payload(&packet).is_err(), "not sealed to A");

        let peer = b.node_id();
        assert_eq!(a.next_send_counter(&peer).unwrap(), 1);
        assert_eq!(a.next_send_counter(&peer).unwrap(), 2);
        assert_eq!(a.observe_counter(&peer, 5).unwrap(), Freshness::Fresh);
        assert_eq!(a.observe_counter(&peer, 5).unwrap(), Freshness::Stale);
        assert_eq!(a.observe_counter(&peer, 6).unwrap(), Freshness::Fresh);
        let before = a.announce_last();
        let e = a.next_announce_counter().unwrap();
        assert_eq!(e, before + 1);
        assert_eq!(a.announce_last(), e);
        let sig = a.sign(b"111/test/v1", b"m");
        assert_eq!(sig.len(), 64);
        a.shutdown_counters().unwrap();
    }

    /// F2 review M1: a restore-pending handle presents no stored announce
    /// and its clean stop writes no `counters` file (the floor stays
    /// settable).
    #[test]
    fn restore_pending_hides_a_stored_announce_and_writes_no_counters() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("identity");
        let fabric = t.path().join("fabric");
        std::fs::create_dir_all(&fabric).unwrap();
        let id = identity(&dir);
        let again = identity_load(&dir); // while `counters` is still there
                                         // a first run mints and stores a valid announce
        let (c, _) = open_counters(&dir).unwrap();
        let s = IdentitySealer::new(id, c);
        let plain = appdata(0, None);
        assert_eq!(s.ensure_announce(&fabric, &plain), AnnounceOutcome::Minted);
        s.shutdown_counters().unwrap();
        drop(s);
        // then the counters file goes missing (a partial identity copy)
        let counters = dir.join(id111::counters::COUNTERS_FILE);
        std::fs::remove_file(&counters).unwrap();
        let (c, how) = open_counters(&dir).unwrap();
        assert_eq!(how, CounterOpen::RestorePending);
        let s = IdentitySealer::new(again, c);
        assert_eq!(
            s.ensure_announce(&fabric, &plain),
            AnnounceOutcome::Missing,
            "a valid stored announce is not presented while pending"
        );
        assert!(s.own_announce().is_empty());
        assert_eq!(s.announce_last(), 0);
        s.shutdown_counters().unwrap();
        assert!(!counters.exists(), "no counter reset on a clean stop");
        assert!(id111::counters::CounterStore::restore_pending(&dir).is_ok());
    }

    fn identity_load(dir: &Path) -> Identity {
        crate::identity::load(dir).expect("identity loads")
    }

    #[test]
    fn restore_pending_refuses_announces_and_epochs() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("identity");
        let id = identity(&dir);
        std::fs::remove_file(dir.join(id111::counters::COUNTERS_FILE)).unwrap();
        let (c, how) = open_counters(&dir).unwrap();
        assert_eq!(how, CounterOpen::RestorePending);
        let s = IdentitySealer::new(id, c);
        assert!(s.restore_pending());
        assert!(s.next_announce_counter().is_err());
        let fabric = t.path().join("fabric");
        std::fs::create_dir_all(&fabric).unwrap();
        assert_eq!(
            s.ensure_announce(&fabric, &appdata(0, None)),
            AnnounceOutcome::Missing
        );
        assert!(s.own_announce().is_empty());
    }

    #[test]
    fn announce_is_stored_kept_and_reminted_on_new_appdata() {
        let t = tempfile::tempdir().unwrap();
        let s = sealer(t.path());
        let fabric = t.path().join("fabric");
        std::fs::create_dir_all(&fabric).unwrap();
        let plain = appdata(0, None);
        assert_eq!(s.ensure_announce(&fabric, &plain), AnnounceOutcome::Minted);
        let first = s.own_announce();
        let a = Announce::parse(&first).unwrap();
        a.verify().unwrap();
        assert_eq!(a.node_id(), s.node_id());
        assert_eq!(a.counter(), s.announce_last());
        assert_eq!(std::fs::read(fabric.join(ANNOUNCE_FILE)).unwrap(), first);
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(fabric.join(ANNOUNCE_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        // same appdata ⇒ kept (stored bytes, never minted)
        assert_eq!(s.ensure_announce(&fabric, &plain), AnnounceOutcome::Kept);
        assert_eq!(s.own_announce(), first);
        // new roles ⇒ a new announce under a higher, committed counter
        let hub = appdata(
            mesh111::contacts::ROLE_MAILBOX | mesh111::contacts::ROLE_ALWAYS_ON_HUB,
            None,
        );
        assert_eq!(s.ensure_announce(&fabric, &hub), AnnounceOutcome::Minted);
        let second = Announce::parse(&s.own_announce()).unwrap();
        assert!(second.counter() > a.counter());
        assert_eq!(second.appdata(), &hub[..]);
        // a foreign or corrupt file is never presented
        std::fs::write(fabric.join(ANNOUNCE_FILE), b"junk").unwrap();
        assert_eq!(s.ensure_announce(&fabric, &hub), AnnounceOutcome::Minted);
    }

    #[test]
    fn appdata_layout() {
        assert_eq!(appdata(3, None), vec![1, 3, 0, 0]);
        let h = NodeId::from([9u8; 32]);
        assert_eq!(
            appdata(1, Some(&h)),
            vec![1, 1, 0, 0, 9, 9, 9, 9, 9, 9, 9, 9]
        );
    }
}
