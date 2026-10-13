//! §6.3 — key distribution: the TOFU pinning store.
//!
//! Implements (owner: T4): keys distribute exclusively via signed announces;
//! there is no registry. The store holds `{sign_pubkey, enc_pubkey,
//! highest_announce_counter, receive_window}` per peer (≈80–130B; no
//! redundant `node_id` field — it IS `sign_pubkey` under B2, so it is the
//! map key, not a stored field).
//!
//! First-seen pins. **Pinned key material is immutable while held**: any
//! announce presenting different key material for a known NodeId is rejected
//! (Meshtastic CVE-2025-55293). The store is bounded — v1 default cap 256
//! peers (≈33KB flash, trivial on ESP32-S3) with LRU eviction permitted.
//! Eviction is safe under B2 (§6.3): the NodeId IS the public key itself, so
//! no attacker can impersonate an evicted NodeId, and a re-appearance
//! re-pins only the true key holder's material — immutability binds key
//! material, not retention (panel seams#6). No wall-clock fields (§12 rule
//! 15).
//!
//! Persistence: the caller passes the identity dir; writes are atomic
//! (temp + fsync + rename + dir fsync — no partial state, §13) and the file
//! is 0600 protocol state, not secret (no zeroize here). Announces are rare,
//! so every accepted announce (pin or counter update) is persisted
//! immediately; per-packet receive-window state is the §6.5 `counters`
//! file's job (batch-committed), and the [`PeerEntry::receive_window`] field
//! mirrors the §6.3 entry shape for hosts that keep one unit — both use
//! [`crate::counters::ReceiveWindow`] with identical semantics.
//!
//! File format v1 (fixed-field binary, fail-closed parse per §12 rules 1/7):
//! `magic "111p"(4) ‖ version=0x01(1) ‖ count LE(4)` then per entry, in LRU
//! order (least-recent first): `sign_pubkey(32) ‖ enc_pubkey(32) ‖
//! highest_announce_counter LE(8) ‖ recv_high LE(8) ‖ recv_bitmap LE(4)`
//! = 84B per peer (inside §6.3's ≈80–130B estimate).

use std::collections::HashMap;
use std::fs;
use std::io;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use crate::counters::{atomic_write_private, ReceiveWindow};
use crate::keyset::{validate_x25519_public_key, NodeId};

/// §6.3 — the v1 default bound: 256 peers (≈33KB flash — trivial on ESP32-S3).
pub const DEFAULT_CAP: usize = 256;

/// Parse guard (§12 rule 7): the peers file never holds more entries than
/// this — far above any configured cap.
pub const MAX_STORE_ENTRIES: usize = 4096;

/// The identity-dir file name for the pinning store (0600 protocol state,
/// like `counters`; §13's file list pins the identity-critical set, and this
/// store follows its conventions).
pub const PEERS_FILE: &str = "peers";

const MAGIC: [u8; 4] = *b"111p";
const FORMAT_VERSION: u8 = 0x01;
/// Header: magic(4) ‖ version(1) ‖ count(4).
const HEADER_LEN: usize = 4 + 1 + 4;
/// Entry: sign_pubkey(32) ‖ enc_pubkey(32) ‖ highest_announce_counter(8) ‖
/// recv_high(8) ‖ recv_bitmap(4).
const ENTRY_LEN: usize = 32 + 32 + 8 + 8 + 4;

// ---------------------------------------------------------------------------
// §6.3 — the pinned-peer entry
// ---------------------------------------------------------------------------

/// §6.3 — one pinned peer: `{sign_pubkey, enc_pubkey,
/// highest_announce_counter, receive_window}`. The `sign_pubkey` half is the
/// map key (under B2 it IS the NodeId — no redundant stored field); this
/// struct carries the remaining three fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerEntry {
    enc_pubkey: [u8; 32],
    highest_announce_counter: u64,
    receive_window: ReceiveWindow,
}

impl PeerEntry {
    /// The peer's X25519 public key (§6.2's `enc_pubkey`; §3 low-order
    /// checked at announce parse time and again at store load).
    pub fn enc_pubkey(&self) -> &[u8; 32] {
        &self.enc_pubkey
    }

    /// §6.2 — the greatest announce counter seen from this peer. A later
    /// announce MUST exceed it (stale ⇒ drop).
    pub fn highest_announce_counter(&self) -> u64 {
        self.highest_announce_counter
    }

    /// §6.3/§6.5 — the per-peer traffic receive window.
    pub fn receive_window(&self) -> ReceiveWindow {
        self.receive_window
    }

    /// Mutable window access for the host's packet path. Window mutations do
    /// NOT auto-persist (per-packet fsync is exactly what §6.5's batch-commit
    /// rule forbids); the §6.5 `counters` file is the traffic-state
    /// authority, and a host mirroring into this entry flushes with
    /// [`PinStore::persist`] at its own batch points.
    pub fn receive_window_mut(&mut self) -> &mut ReceiveWindow {
        &mut self.receive_window
    }
}

// ---------------------------------------------------------------------------
// Errors & outcomes
// ---------------------------------------------------------------------------

/// Pinning-store errors. Key-material conflicts and stale counters are
/// rejections, not mutations: the held pin is never changed by them.
#[derive(Debug)]
pub enum StoreError {
    /// §6.3/§12 rule 3 — an announce presented different key material for a
    /// pinned NodeId (CVE-2025-55293 class). Rejected; the pin is unchanged.
    KeyConflict,
    /// §6.2 — the announce counter did not exceed the last-seen value
    /// (stale ⇒ drop).
    StaleCounter { last_seen: u64, got: u64 },
    /// Fail-closed parse rejection (§12 rules 1/7).
    Corrupt(&'static str),
    /// Filesystem failure on the atomic persist.
    Io(io::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::KeyConflict => write!(
                f,
                "§6.3: conflicting key material for a pinned NodeId (immutable while held)"
            ),
            StoreError::StaleCounter { last_seen, got } => write!(
                f,
                "§6.2: stale announce counter {got} (last-seen {last_seen}) — dropped"
            ),
            StoreError::Corrupt(why) => write!(f, "§12: corrupt pinning store: {why}"),
            StoreError::Io(e) => write!(f, "pinning-store persist failed: {e}"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        StoreError::Io(e)
    }
}

/// The result of accepting a verified announce into the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinOutcome {
    /// First-seen pin (TOFU). `evicted` names the LRU victim dropped to stay
    /// within the cap, if the store was full.
    Pinned { evicted: Option<NodeId> },
    /// Known peer, same key material, fresher counter: the stored
    /// highest-announce-counter advanced from `previous`.
    CounterUpdated { previous: u64 },
}

// ---------------------------------------------------------------------------
// The pinning store
// ---------------------------------------------------------------------------

/// §6.3 — the bounded TOFU pinning store. See the module docs for the
/// immutability rule, the LRU-eviction safety rationale, and the file format.
///
/// Concurrency: single-writer (same integration contract as
/// [`crate::counters::CounterStore`]).
pub struct PinStore {
    dir: PathBuf,
    cap: usize,
    /// LRU via a monotonic use tick per entry; the smallest tick is the
    /// eviction victim. Ticks persist implicitly: the file is written in
    /// LRU order and reload assigns ticks in that order.
    entries: HashMap<NodeId, (PeerEntry, u64)>,
    next_tick: u64,
}

impl PinStore {
    /// Open (or start empty) the identity-dir pinning store with the §6.3
    /// default cap of 256 peers.
    pub fn open(dir: &Path) -> Result<Self, StoreError> {
        Self::open_with_cap(dir, NonZeroUsize::new(DEFAULT_CAP).expect("256 > 0"))
    }

    /// Open with an explicit cap (tests, constrained devices tuning the §6.3
    /// bound). A missing file is an empty store, not an error — the pins are
    /// a TOFU cache: rebuilt from announces, never identity-critical.
    pub fn open_with_cap(dir: &Path, cap: NonZeroUsize) -> Result<Self, StoreError> {
        let path = dir.join(PEERS_FILE);
        let mut store = PinStore {
            dir: dir.to_path_buf(),
            cap: cap.get(),
            entries: HashMap::new(),
            next_tick: 0,
        };
        if path.exists() {
            let bytes = fs::read(&path)?;
            store.load(&bytes)?;
        }
        Ok(store)
    }

    /// Number of pinned peers.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no peer is pinned.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The configured peer cap (§6.3; v1 default 256).
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// A pinned peer's entry.
    pub fn get(&self, id: &NodeId) -> Option<&PeerEntry> {
        self.entries.get(id).map(|(e, _)| e)
    }

    /// Mutable entry access (the window, per its docs). Does not affect LRU
    /// order — use [`Self::touch`] for that.
    pub fn get_mut(&mut self, id: &NodeId) -> Option<&mut PeerEntry> {
        self.entries.get_mut(id).map(|(e, _)| e)
    }

    /// Mark a peer most-recently-used without changing its entry.
    pub fn touch(&mut self, id: &NodeId) {
        let tick = self.alloc_tick();
        if let Some((_, slot)) = self.entries.get_mut(id) {
            *slot = tick;
        }
    }

    /// §6.2/§6.3 — the announce state transition, applied ONLY after the
    /// announce's signature has verified (§6.2: verification before any
    /// state change — `announce::process_announce` orders it so):
    ///
    /// - known NodeId, different `enc_pubkey` ⇒ [`StoreError::KeyConflict`]
    ///   (pinned key material is immutable while held, §12 rule 3);
    /// - known NodeId, `counter` ≤ last-seen ⇒ [`StoreError::StaleCounter`]
    ///   (stale ⇒ drop);
    /// - known NodeId, fresher counter ⇒ counter update (+ persist);
    /// - unknown NodeId ⇒ first-seen pin (+ LRU eviction at the cap +
    ///   persist).
    ///
    /// A first-seen counter of any value is accepted as the pin's starting
    /// high-water — the MUST-exceed rule needs a last-seen value, and a
    /// first-seen pin has none (the genesis announce carries 1, §6.2).
    pub fn apply_announce(
        &mut self,
        node_id: NodeId,
        enc_pubkey: [u8; 32],
        counter: u64,
    ) -> Result<PinOutcome, StoreError> {
        if let Some((entry, _)) = self.entries.get_mut(&node_id) {
            if entry.enc_pubkey != enc_pubkey {
                return Err(StoreError::KeyConflict);
            }
            if counter <= entry.highest_announce_counter {
                return Err(StoreError::StaleCounter {
                    last_seen: entry.highest_announce_counter,
                    got: counter,
                });
            }
            let previous = entry.highest_announce_counter;
            entry.highest_announce_counter = counter;
            self.touch(&node_id);
            self.persist()?;
            return Ok(PinOutcome::CounterUpdated { previous });
        }

        // First-seen pin. Bounded: evict least-recently-used until there is
        // room (safe under B2 — see module docs). The `while` also drains a
        // store loaded from a file written under a larger cap.
        let mut evicted = None;
        while self.entries.len() >= self.cap {
            let victim = self.lru_victim();
            self.entries.remove(&victim);
            evicted = Some(victim);
        }
        let entry = PeerEntry {
            enc_pubkey,
            highest_announce_counter: counter,
            receive_window: ReceiveWindow::new(),
        };
        let tick = self.alloc_tick();
        self.entries.insert(node_id, (entry, tick));
        self.persist()?;
        Ok(PinOutcome::Pinned { evicted })
    }

    /// §13 — atomic persist (temp + fsync + rename + dir fsync), 0600.
    /// Announces auto-persist on every accept; hosts mutating entry windows
    /// per packet call this at their own batch points (§6.5 discipline).
    pub fn persist(&self) -> Result<(), StoreError> {
        let mut buf = Vec::with_capacity(HEADER_LEN + self.entries.len() * ENTRY_LEN);
        buf.extend_from_slice(&MAGIC);
        buf.push(FORMAT_VERSION);
        // Bounded by MAX_STORE_ENTRIES at parse and cap ≤ guard at runtime.
        buf.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        // LRU order, least-recent first — reload rebuilds ticks from it.
        let mut entries: Vec<(&NodeId, &(PeerEntry, u64))> = self.entries.iter().collect();
        entries.sort_by_key(|(_, (_, tick))| *tick);
        for (id, (e, _)) in entries {
            buf.extend_from_slice(id.as_bytes());
            buf.extend_from_slice(&e.enc_pubkey);
            buf.extend_from_slice(&e.highest_announce_counter.to_le_bytes());
            buf.extend_from_slice(&e.receive_window.high().to_le_bytes());
            buf.extend_from_slice(&e.receive_window.bitmap().to_le_bytes());
        }
        atomic_write_private(&self.dir, PEERS_FILE, &buf)?;
        Ok(())
    }

    fn alloc_tick(&mut self) -> u64 {
        // Saturating: after 2^64 uses ties in LRU order are acceptable.
        self.next_tick = self.next_tick.saturating_add(1);
        self.next_tick
    }

    fn lru_victim(&self) -> NodeId {
        *self
            .entries
            .iter()
            .min_by_key(|(_, (_, tick))| *tick)
            .map(|(id, _)| id)
            .expect("eviction runs only on a full, non-empty store")
    }

    /// Fail-closed parse (§12 rules 1/7): exact layout, exact length,
    /// bounded count, no duplicate NodeIds, and every stored `enc_pubkey`
    /// re-checked against the §3 low-order rule at load.
    fn load(&mut self, bytes: &[u8]) -> Result<(), StoreError> {
        if bytes.len() < HEADER_LEN {
            return Err(StoreError::Corrupt("truncated header"));
        }
        if !bytes.starts_with(&MAGIC) {
            return Err(StoreError::Corrupt("bad magic"));
        }
        if bytes[4] != FORMAT_VERSION {
            return Err(StoreError::Corrupt("unknown format version"));
        }
        let count = u32::from_le_bytes(bytes[5..9].try_into().expect("4-byte slice")) as usize;
        if count > MAX_STORE_ENTRIES {
            return Err(StoreError::Corrupt("entry count over guard"));
        }
        if bytes.len() != HEADER_LEN + count * ENTRY_LEN {
            return Err(StoreError::Corrupt("length mismatch"));
        }
        for i in 0..count {
            let off = HEADER_LEN + i * ENTRY_LEN;
            let rec = &bytes[off..off + ENTRY_LEN];
            let id = NodeId::from(<[u8; 32]>::try_from(&rec[..32]).expect("32-byte slice"));
            let enc = <[u8; 32]>::try_from(&rec[32..64]).expect("32-byte slice");
            validate_x25519_public_key(&enc)
                .map_err(|_| StoreError::Corrupt("low-order enc_pubkey at load"))?;
            let highest = u64::from_le_bytes(rec[64..72].try_into().expect("8-byte slice"));
            let high = u64::from_le_bytes(rec[72..80].try_into().expect("8-byte slice"));
            let bitmap = u32::from_le_bytes(rec[80..84].try_into().expect("4-byte slice"));
            let entry = PeerEntry {
                enc_pubkey: enc,
                highest_announce_counter: highest,
                receive_window: ReceiveWindow::from_raw(high, bitmap),
            };
            // File order is LRU order (least-recent first): rebuild ticks.
            let tick = self.alloc_tick();
            if self.entries.insert(id, (entry, tick)).is_some() {
                return Err(StoreError::Corrupt("duplicate node id"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroUsize;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Minimal unique tempdir (deps are frozen — no tempfile crate).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "id111-t4-store-{tag}-{}-{seq}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn cap(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    /// A deterministic, non-low-order X25519 public key for tests: the RFC
    /// 7748 basepoint multiplied by a small scalar via dalek.
    fn enc_key(n: u8) -> [u8; 32] {
        let mut scalar = [0u8; 32];
        // RFC 7748 clamping clears the low 3 bits — << 3 keeps test keys
        // distinct after clamping (tests use small n).
        scalar[0] = n << 3;
        let secret = x25519_dalek::StaticSecret::from(scalar);
        x25519_dalek::PublicKey::from(&secret).to_bytes()
    }

    fn id(n: u8) -> NodeId {
        NodeId::from([n; 32])
    }

    // ------------------------------------------------------------------
    // §6.3 — first-seen pin, counter update, immutability, staleness.
    // ------------------------------------------------------------------

    #[test]
    fn first_seen_pins_and_fresher_counter_updates() {
        let tmp = TempDir::new("pin");
        let mut store = PinStore::open(tmp.path()).unwrap();
        let a = id(0xa1);
        let enc_a = enc_key(1);
        assert_eq!(
            store.apply_announce(a, enc_a, 1).unwrap(),
            PinOutcome::Pinned { evicted: None }
        );
        assert_eq!(store.len(), 1);
        let e = *store.get(&a).unwrap();
        assert_eq!(e.enc_pubkey(), &enc_a);
        assert_eq!(e.highest_announce_counter(), 1);

        assert_eq!(
            store.apply_announce(a, enc_a, 5).unwrap(),
            PinOutcome::CounterUpdated { previous: 1 }
        );
        assert_eq!(store.get(&a).unwrap().highest_announce_counter(), 5);
    }

    #[test]
    fn stale_counter_dropped() {
        let tmp = TempDir::new("stale");
        let mut store = PinStore::open(tmp.path()).unwrap();
        let a = id(0xa2);
        let enc_a = enc_key(1);
        store.apply_announce(a, enc_a, 3).unwrap();
        // Equal is stale (the rule is MUST EXCEED)…
        assert!(matches!(
            store.apply_announce(a, enc_a, 3),
            Err(StoreError::StaleCounter {
                last_seen: 3,
                got: 3
            })
        ));
        // …and so is lower.
        assert!(matches!(
            store.apply_announce(a, enc_a, 2),
            Err(StoreError::StaleCounter {
                last_seen: 3,
                got: 2
            })
        ));
        // The entry is untouched.
        assert_eq!(store.get(&a).unwrap().highest_announce_counter(), 3);
    }

    #[test]
    fn conflicting_key_material_rejected_pin_immutable() {
        let tmp = TempDir::new("conflict");
        let mut store = PinStore::open(tmp.path()).unwrap();
        let a = id(0xa3);
        let enc_a = enc_key(1);
        let enc_b = enc_key(2);
        store.apply_announce(a, enc_a, 1).unwrap();
        // §12 rule 3 (CVE-2025-55293 class): different enc_pubkey for a
        // pinned NodeId is rejected even with a fresher counter…
        assert!(matches!(
            store.apply_announce(a, enc_b, 2),
            Err(StoreError::KeyConflict)
        ));
        // …and the held pin is unchanged by the attempt.
        let e = store.get(&a).unwrap();
        assert_eq!(e.enc_pubkey(), &enc_a);
        assert_eq!(e.highest_announce_counter(), 1);
    }

    // ------------------------------------------------------------------
    // §6.3 — bounded store, LRU eviction, re-pin after eviction.
    // ------------------------------------------------------------------

    #[test]
    fn lru_eviction_at_cap_and_repin_after_eviction() {
        let tmp = TempDir::new("lru");
        let mut store = PinStore::open_with_cap(tmp.path(), cap(3)).unwrap();
        let (a, b, c, d) = (id(0xb1), id(0xb2), id(0xb3), id(0xb4));
        store.apply_announce(a, enc_key(1), 1).unwrap();
        store.apply_announce(b, enc_key(2), 1).unwrap();
        store.apply_announce(c, enc_key(3), 1).unwrap();
        // Refresh A: B is now the least-recently-used.
        store.apply_announce(a, enc_key(1), 2).unwrap();
        // Pinning D evicts B, and ONLY B.
        assert_eq!(
            store.apply_announce(d, enc_key(4), 1).unwrap(),
            PinOutcome::Pinned { evicted: Some(b) }
        );
        assert!(store.get(&b).is_none());
        for still in [a, c, d] {
            assert!(store.get(&still).is_some());
        }
        assert_eq!(store.len(), 3);

        // §6.3's eviction-safety rationale: an evicted NodeId's re-appearance
        // is a fresh first-seen pin — immutability binds key material while
        // HELD, never retention.
        assert_eq!(
            store.apply_announce(b, enc_key(9), 1).unwrap(),
            PinOutcome::Pinned { evicted: Some(c) }
        );
        assert_eq!(store.get(&b).unwrap().enc_pubkey(), &enc_key(9));
    }

    #[test]
    fn touch_refreshes_lru_order() {
        let tmp = TempDir::new("touch");
        let mut store = PinStore::open_with_cap(tmp.path(), cap(2)).unwrap();
        let (a, b, c) = (id(0xc1), id(0xc2), id(0xc3));
        store.apply_announce(a, enc_key(1), 1).unwrap();
        store.apply_announce(b, enc_key(2), 1).unwrap();
        store.touch(&a); // B is now LRU
        assert_eq!(
            store.apply_announce(c, enc_key(3), 1).unwrap(),
            PinOutcome::Pinned { evicted: Some(b) }
        );
    }

    // ------------------------------------------------------------------
    // §13 — persistence: round-trip, LRU order survives, fail-closed parse.
    // ------------------------------------------------------------------

    #[test]
    fn persistence_round_trip_including_window() {
        let tmp = TempDir::new("persist");
        let (a, b) = (id(0xd1), id(0xd2));
        {
            let mut store = PinStore::open(tmp.path()).unwrap();
            store.apply_announce(a, enc_key(1), 7).unwrap();
            store.apply_announce(b, enc_key(2), 3).unwrap();
            // Host packet path mirrors a window into the entry…
            let w = store.get_mut(&a).unwrap().receive_window_mut();
            w.offer(41).unwrap();
            w.offer(43).unwrap();
            // …and flushes at its own batch point.
            store.persist().unwrap();
        }
        let store = PinStore::open(tmp.path()).unwrap();
        assert_eq!(store.len(), 2);
        let ea = store.get(&a).unwrap();
        assert_eq!(ea.enc_pubkey(), &enc_key(1));
        assert_eq!(ea.highest_announce_counter(), 7);
        assert_eq!(ea.receive_window().high(), 43);
        assert_eq!(ea.receive_window().bitmap(), 0b101); // 43 and 41
        assert_eq!(store.get(&b).unwrap().highest_announce_counter(), 3);

        // LRU order survives the round-trip: A was pinned first, B second;
        // evicting at cap 2 must take A.
        let tmp2 = TempDir::new("persist-lru");
        std::fs::copy(
            tmp.path().join(PEERS_FILE),
            tmp2.path().join(PEERS_FILE),
        )
        .unwrap();
        let mut store = PinStore::open_with_cap(tmp2.path(), cap(2)).unwrap();
        assert_eq!(
            store.apply_announce(id(0xd3), enc_key(3), 1).unwrap(),
            PinOutcome::Pinned { evicted: Some(a) }
        );
    }

    #[test]
    fn missing_file_is_an_empty_store_not_an_error() {
        let tmp = TempDir::new("missing");
        let store = PinStore::open(tmp.path()).unwrap();
        assert!(store.is_empty());
        assert_eq!(store.cap(), DEFAULT_CAP);
    }

    #[test]
    fn corrupt_file_rejected_fail_closed() {
        let good = {
            let tmp = TempDir::new("corrupt-src");
            let mut store = PinStore::open(tmp.path()).unwrap();
            store.apply_announce(id(0xe1), enc_key(1), 1).unwrap();
            fs::read(tmp.path().join(PEERS_FILE)).unwrap()
        };
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty", vec![]),
            ("truncated header", good[..HEADER_LEN - 1].to_vec()),
            ("bad magic", {
                let mut b = good.clone();
                b[0] = b'X';
                b
            }),
            ("unknown version", {
                let mut b = good.clone();
                b[4] = 0x02;
                b
            }),
            ("count over guard", {
                let mut b = good.clone();
                b[5..9].copy_from_slice(&(u32::MAX).to_le_bytes());
                b
            }),
            ("length mismatch", {
                let mut b = good.clone();
                b.push(0x00);
                b
            }),
            ("truncated entry", good[..good.len() - 10].to_vec()),
            ("low-order enc at load", {
                let mut b = good.clone();
                for x in &mut b[HEADER_LEN + 32..HEADER_LEN + 64] {
                    *x = 0;
                }
                b
            }),
            ("duplicate node id", {
                let entry = good[HEADER_LEN..].to_vec();
                let mut b = good[..HEADER_LEN].to_vec();
                b[5..9].copy_from_slice(&2u32.to_le_bytes());
                b.extend_from_slice(&entry);
                b.extend_from_slice(&entry);
                b
            }),
        ];
        for (name, bytes) in cases {
            let tmp = TempDir::new(name);
            fs::write(tmp.path().join(PEERS_FILE), &bytes).unwrap();
            assert!(
                matches!(PinStore::open(tmp.path()), Err(StoreError::Corrupt(_))),
                "case {name} must be rejected as corrupt"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn peers_file_is_0600_and_write_is_atomic() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new("perms");
        let mut store = PinStore::open(tmp.path()).unwrap();
        store.apply_announce(id(0xf1), enc_key(1), 1).unwrap();
        let mode = std::fs::metadata(tmp.path().join(PEERS_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        // No temp file lingers after an atomic write (§13: no partial state).
        assert!(!tmp.path().join(format!("{PEERS_FILE}.tmp")).exists());
    }
}
