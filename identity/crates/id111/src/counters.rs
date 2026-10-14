//! §6.5 — replay protection: counters, not clocks (plus §6.2's announce
//! counter and §8.4's restore semantics).
//!
//! Implements (owner: T4): per-peer persistent 64-bit counters carried in
//! AEAD associated data (§6.4) + a 32-entry sliding acceptance window. No
//! wall-clock anywhere (§12 rule 15).
//!
//! Commit discipline: receive windows and per-peer send counters are
//! batch-committed every N packets and on clean shutdown (the §6.2 announce
//! counter instead commits synchronously — fsync — BEFORE the announce it
//! authorizes is transmitted; announces are rare, and the flash-wear argument
//! never applies to them — panel seams#1). After an unclean reboot BOTH sides
//! apply a conservative forward jump with **J ≥ N** (send-side counter and
//! receive-side high-water alike); the receive side accepts the resulting gap
//! harmlessly — a jump only advances the window. The §6.5 residual is stated
//! honestly: a receiver's stale-by-≤N persisted window can re-accept
//! already-consumed frames after an unclean reboot; content-level idempotency
//! is Layer 5's (§13).
//!
//! Unclean-vs-clean detection: the `counters` file carries a clean-shutdown
//! flag byte. On disk the file is ALWAYS marked dirty/running except by
//! [`CounterStore::shutdown`]; [`CounterStore::open`] treats any flag ≠ 0x01
//! as an unclean previous run and applies the jump (fail-safe: a torn flag
//! byte costs only a harmless forward jump).
//!
//! Restore (§8.4): state present ⇒ reuse as-is (`open`). State absent ⇒
//! [`CounterStore::restore_pending`]: the node MUST NOT announce
//! ([`CounterStore::next_announce_counter`] errors) until
//! [`CounterStore::set_restore_floor`] sets `counter = floor + 1024` — the
//! floor learned from mesh-gossiped copies of its own prior announces, else
//! operator-supplied, else 0 only for a never-before-deployed identity — and
//! applies the same jump to per-peer traffic send-counters on new sessions.
//!
//! Layer 5 amendments (`os/messaging/README.md` §4, §19):
//!
//! - **A11 — verdict, not rejection.** [`CounterStore::observe_counter`]
//!   offers a verified packet's counter to the same 32-entry window as
//!   [`CounterStore::on_receive`], advancing it exactly as `on_receive` would
//!   for an accepted counter, but reports a counter the window would reject
//!   as [`Freshness::Stale`] (no state change) instead of an error. For
//!   `msg_seq` kinds replay authority is Layer 5's own receive state; the
//!   verdict only gates connection attribution, card answers and proactive
//!   pushes (L5 §4, §8.1). `on_receive` keeps the §6.5 rejecting contract.
//! - **A3 — the `msg_seq` epoch source.** [`CounterStore::next_announce_counter`]
//!   is the §4 epoch source: each call returns a fresh, strictly greater
//!   value that is already durable (fsync) when returned, and refuses with
//!   [`CounterError::RestoreFloorRequired`] while a state-absent restore has
//!   no floor — during which **nothing is sealed** (L5 queues outbound work
//!   as `restore_pending`). [`CounterStore::announce_last`] is the read-only
//!   value L5 records at clean close and compares at the next open.
//!
//! Counter state is protocol state, not secret (§13 `counters` file, 0600 /
//! the Cat-1 NVS map) — no zeroize handling in this module.
//!
//! File format v1 (fixed-field binary, fail-closed parse per §12 rules 1/7):
//! `magic "111c"(4) ‖ version=0x01(1) ‖ clean_flag(1) ‖ announce_last LE(8) ‖
//! traffic_base LE(8) ‖ batch_n LE(8) ‖ peer_count LE(4)` then per peer
//! `node_id(32) ‖ send_last LE(8) ‖ recv_high LE(8) ‖ recv_bitmap LE(4)`.
//! `announce_last`/`send_last` hold the LAST committed/emitted value; the
//! next announce/packet uses +1 (§6.2: initial value 0, genesis announce
//! uses 1). Writes are atomic (temp + fsync + rename + dir fsync, §13).

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

use crate::keyset::NodeId;

/// §8.4 — the state-absent restore safety margin: `counter = floor + 1024`,
/// and the same jump applied to per-peer traffic send-counters on new
/// sessions. Over-jumping is harmless — gaps only advance windows.
pub const RESTORE_JUMP: u64 = 1024;

/// §6.5 — the sliding acceptance window width (LoRaWAN/IPsec class; Briar
/// confirms 32-entry windows at mesh scale).
pub const WINDOW_SIZE: u32 = 32;

/// §6.5 — default batch-commit interval N (packets) for per-peer traffic
/// counters/windows. The unclean-reboot forward jump J equals the persisted
/// N (satisfying J ≥ N at the spec's minimum; the ≤N stale-window
/// re-acceptance residual is §6.5's own, handed to Layer 5 idempotency).
pub const DEFAULT_BATCH_N: u64 = 64;

/// Parse/insert guard (§12 rule 7): the counters file never holds more peer
/// records than this. Traffic peers are expected ⊆ the §6.3 pinning set
/// (default cap 256); this guard exists only to bound hostile/corrupt input.
pub const MAX_COUNTER_PEERS: usize = 1024;

/// §13 — the identity-dir file name for counter state (0600; protocol state,
/// not secret).
pub const COUNTERS_FILE: &str = "counters";

const MAGIC: [u8; 4] = *b"111c";
const FORMAT_VERSION: u8 = 0x01;
/// Header: magic(4) ‖ version(1) ‖ clean_flag(1) ‖ announce_last(8) ‖
/// traffic_base(8) ‖ batch_n(8) ‖ peer_count(4).
const HEADER_LEN: usize = 4 + 1 + 1 + 8 + 8 + 8 + 4;
/// Peer record: node_id(32) ‖ send_last(8) ‖ recv_high(8) ‖ recv_bitmap(4).
const PEER_RECORD_LEN: usize = 32 + 8 + 8 + 4;

const CLEAN_SHUTDOWN: u8 = 0x01;
const DIRTY_RUNNING: u8 = 0x00;

// ---------------------------------------------------------------------------
// §6.5 — the 32-entry sliding acceptance window
// ---------------------------------------------------------------------------

/// §6.5 — the sliding acceptance window over a peer's traffic counters:
/// `high` is the greatest counter accepted so far; bit `i` of `bitmap`
/// records whether counter `high - i` was accepted (bit 0 ⇔ `high` itself).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiveWindow {
    high: u64,
    bitmap: u32,
}

/// §6.5 window rejection reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowReject {
    /// The counter was already consumed (replay).
    Replayed,
    /// The counter fell behind the trailing edge of the 32-entry window.
    OutsideWindow,
}

impl std::fmt::Display for WindowReject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WindowReject::Replayed => write!(f, "§6.5: counter already consumed (replay)"),
            WindowReject::OutsideWindow => {
                write!(f, "§6.5: counter behind the 32-entry acceptance window")
            }
        }
    }
}

impl std::error::Error for WindowReject {}

/// L5 amendment A11 — the window's verdict on a verified packet's counter
/// ([`CounterStore::observe_counter`]). Bookkeeping, never rejection: the
/// caller processes `msg_seq` content whatever the verdict (its own §4
/// receive state is the replay authority), and only a `Fresh` packet may
/// attribute a connection, answer a card request or trigger a push.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// The window accepted the counter (above high-water, or an unseen slot
    /// inside the window); its state advanced exactly as
    /// [`CounterStore::on_receive`] would have.
    Fresh,
    /// The window would reject the counter (already seen, or 32 or more
    /// behind high-water); nothing changed.
    Stale,
}

impl ReceiveWindow {
    /// A fresh window (nothing received yet).
    pub fn new() -> Self {
        ReceiveWindow { high: 0, bitmap: 0 }
    }

    /// Rebuild a window from its persisted parts (the `counters` and `peers`
    /// files both serialize `high ‖ bitmap`).
    pub(crate) fn from_raw(high: u64, bitmap: u32) -> Self {
        ReceiveWindow { high, bitmap }
    }

    /// The greatest counter accepted so far.
    pub fn high(&self) -> u64 {
        self.high
    }

    /// The raw acceptance bitmap (bit `i` ⇔ counter `high - i` accepted).
    pub fn bitmap(&self) -> u32 {
        self.bitmap
    }

    /// Offer a received counter: accept it (advancing the window) or reject
    /// it as a replay / too-old frame. Counter values above `high` slide the
    /// window forward (gaps are harmless — they only advance the window).
    pub fn offer(&mut self, counter: u64) -> Result<(), WindowReject> {
        if counter > self.high {
            let shift = counter - self.high;
            self.bitmap = if shift >= u64::from(WINDOW_SIZE) {
                1
            } else {
                (self.bitmap << shift) | 1
            };
            self.high = counter;
            return Ok(());
        }
        let d = self.high - counter;
        if d >= u64::from(WINDOW_SIZE) {
            return Err(WindowReject::OutsideWindow);
        }
        let bit = 1u32 << d;
        if self.bitmap & bit != 0 {
            return Err(WindowReject::Replayed);
        }
        self.bitmap |= bit;
        Ok(())
    }

    /// §6.5 unclean-reboot receive-side forward jump: the high-water advances
    /// by `jump` and the bitmap travels with it (a jump only advances the
    /// window). Saturates at u64::MAX — 2^64 packets is not a reachable state.
    pub fn jump_forward(&mut self, jump: u64) {
        self.bitmap = if jump >= u64::from(WINDOW_SIZE) {
            0
        } else {
            self.bitmap << jump
        };
        self.high = self.high.saturating_add(jump);
    }
}

impl Default for ReceiveWindow {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Counter-store errors. The store fails closed: any corrupt persisted state
/// is an error, never a guess (§12 rule 1; §13 boot integrity).
#[derive(Debug)]
pub enum CounterError {
    /// §13 — the `counters` file does not exist on the `open` path; the
    /// caller distinguishes fresh keygen ([`CounterStore::create`]) from
    /// state-absent restore ([`CounterStore::restore_pending`]).
    NotFound,
    /// The file exists where a constructor requires absence (§13
    /// never-overwrite / boot-integrity discipline).
    AlreadyExists,
    /// Fail-closed parse rejection (§12 rules 1/7).
    Corrupt(&'static str),
    /// §8.4 — a state-absent restore MUST NOT announce before the counter
    /// floor is set (`set_restore_floor`).
    RestoreFloorRequired,
    /// `set_restore_floor` called on a store not in the restore-pending state.
    FloorAlreadySet,
    /// A counter would exceed u64::MAX — fail closed. (A §8.4 floor is
    /// gossip/operator input, i.e. untrusted; the addition is checked.)
    Overflow,
    /// §12 rule 7 guard — more than [`MAX_COUNTER_PEERS`] traffic peers.
    TooManyPeers,
    /// §6.5 — the acceptance window rejected the packet.
    Window(WindowReject),
    /// Filesystem failure on commit.
    Io(io::Error),
}

impl std::fmt::Display for CounterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CounterError::NotFound => write!(f, "§13: counters file not found"),
            CounterError::AlreadyExists => {
                write!(f, "§13: counters file already exists (never overwrite)")
            }
            CounterError::Corrupt(why) => write!(f, "§12: corrupt counters file: {why}"),
            CounterError::RestoreFloorRequired => {
                write!(f, "§8.4: no announce before the restore floor is set")
            }
            CounterError::FloorAlreadySet => {
                write!(f, "§8.4: restore floor already set / not restore-pending")
            }
            CounterError::Overflow => write!(f, "counter would exceed u64::MAX"),
            CounterError::TooManyPeers => write!(f, "§12: traffic-peer guard exceeded"),
            CounterError::Window(e) => write!(f, "{e}"),
            CounterError::Io(e) => write!(f, "counters commit failed: {e}"),
        }
    }
}

impl std::error::Error for CounterError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CounterError::Window(e) => Some(e),
            CounterError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for CounterError {
    fn from(e: io::Error) -> Self {
        CounterError::Io(e)
    }
}

// ---------------------------------------------------------------------------
// §6.5 — per-peer traffic state
// ---------------------------------------------------------------------------

/// §6.5 per-peer traffic state: `send_last` is the last counter this node
/// emitted to the peer (the next packet uses +1) plus the receive window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerTraffic {
    send_last: u64,
    window: ReceiveWindow,
}

impl PeerTraffic {
    /// The last send counter emitted to this peer (next packet uses +1).
    pub fn send_last(&self) -> u64 {
        self.send_last
    }

    /// The per-peer receive window (§6.5).
    pub fn receive_window(&self) -> ReceiveWindow {
        self.window
    }
}

// ---------------------------------------------------------------------------
// The counter store (§6.2 announce counter + §6.5 traffic counters)
// ---------------------------------------------------------------------------

/// §6.2/§6.5/§8.4 — the persistent counter store bound to the identity dir's
/// `counters` file (0600). See the module docs for the commit discipline,
/// the unclean-reboot jump, and the restore floor rules.
///
/// Concurrency: single-writer. The host serializes access (one `111d`
/// identity owner); the temp file name is fixed, so concurrent writers would
/// be an integration bug, not a handled case.
pub struct CounterStore {
    dir: PathBuf,
    announce_last: u64,
    traffic_base: u64,
    restore_pending: bool,
    peers: HashMap<NodeId, PeerTraffic>,
    dirty_packets: u64,
    batch_n: u64,
    /// True while the persisted flag says CLEAN (fresh create, restore floor
    /// set, or clean shutdown — no emission since). The first traffic
    /// operation flips the file to DIRTY_RUNNING before proceeding, so a
    /// crash after any emission is always seen as unclean at the next open.
    clean_on_disk: bool,
}

impl CounterStore {
    /// §13 — create a fresh counter file for a never-before-deployed
    /// identity: announce counter 0 committed (the genesis announce will use
    /// 1, §6.2). Fails if the file already exists (never overwrite).
    pub fn create(dir: &Path) -> Result<Self, CounterError> {
        let path = dir.join(COUNTERS_FILE);
        if path.exists() {
            return Err(CounterError::AlreadyExists);
        }
        let mut store = CounterStore {
            dir: dir.to_path_buf(),
            announce_last: 0,
            traffic_base: 0,
            restore_pending: false,
            peers: HashMap::new(),
            dirty_packets: 0,
            batch_n: DEFAULT_BATCH_N,
            clean_on_disk: false,
        };
        // Initial commit: counter 0 durable. Persisted CLEAN — nothing has been
        // emitted yet, so the first open() must not read this as a crash
        // (reviewer fix: the §6.5 unclean jump is for real crashes only).
        store.persist(CLEAN_SHUTDOWN)?;
        Ok(store)
    }

    /// §13/§8.4 — open an existing counter file (state-present case: reused
    /// as-is). Fails [`CounterError::NotFound`] when absent.
    ///
    /// Unless the previous run marked a clean shutdown, this applies the §6.5
    /// unclean-reboot forward jump J = N (≥ N) to every per-peer send counter
    /// and receive high-water — the announce counter is NOT jumped (§6.2
    /// sync-commits it before transmit, so it is never uncommitted). The file
    /// is then re-marked dirty/running so a crash from here reads as unclean
    /// at the next open.
    pub fn open(dir: &Path) -> Result<Self, CounterError> {
        let path = dir.join(COUNTERS_FILE);
        if !path.exists() {
            return Err(CounterError::NotFound);
        }
        let bytes = fs::read(&path)?;
        let parsed = parse_file(&bytes)?;
        let mut store = CounterStore {
            dir: dir.to_path_buf(),
            announce_last: parsed.announce_last,
            traffic_base: parsed.traffic_base,
            restore_pending: false,
            peers: parsed.peers,
            dirty_packets: 0,
            batch_n: parsed.batch_n,
            clean_on_disk: false,
        };
        if !parsed.clean {
            store.apply_unclean_jump();
        }
        // Mark dirty/running (and commit the jump, if any) immediately.
        store.persist(DIRTY_RUNNING)?;
        Ok(store)
    }

    /// §8.4 — the state-absent restore entry point: a store that refuses to
    /// issue announce counters until [`Self::set_restore_floor`] commits the
    /// floor + 1024 jump. No file is written until then. Fails
    /// [`CounterError::AlreadyExists`] if a counters file is present (that is
    /// the state-present case — use [`Self::open`]).
    pub fn restore_pending(dir: &Path) -> Result<Self, CounterError> {
        let path = dir.join(COUNTERS_FILE);
        if path.exists() {
            return Err(CounterError::AlreadyExists);
        }
        Ok(CounterStore {
            dir: dir.to_path_buf(),
            announce_last: 0,
            traffic_base: 0,
            restore_pending: true,
            peers: HashMap::new(),
            dirty_packets: 0,
            batch_n: DEFAULT_BATCH_N,
            clean_on_disk: false,
        })
    }

    /// §8.4 (normative) — set the restore counter floor for a state-absent
    /// recovery: commits `announce_last = floor + 1024` (safety margin;
    /// over-jumping is harmless) and `traffic_base = 1024` so per-peer send
    /// counters on new sessions take the same jump. The floor is learned from
    /// mesh-gossiped copies of the node's own prior announces where a mesh
    /// exists, else operator-supplied, else 0 only for a never-before-deployed
    /// identity. Committed (fsync) BEFORE returning — this state authorizes
    /// every future announce, so it takes the §6.2 sync-commit discipline.
    pub fn set_restore_floor(&mut self, floor: u64) -> Result<(), CounterError> {
        if !self.restore_pending {
            return Err(CounterError::FloorAlreadySet);
        }
        self.announce_last = floor
            .checked_add(RESTORE_JUMP)
            .ok_or(CounterError::Overflow)?;
        self.traffic_base = RESTORE_JUMP;
        self.restore_pending = false;
        // Quiescent state (no emissions yet) — persisted CLEAN, see create().
        self.persist(CLEAN_SHUTDOWN)?;
        Ok(())
    }

    /// §6.2 (normative) — the announce sequence counter. The next value is
    /// committed synchronously (fsync) BEFORE this call returns it, so the
    /// announce it authorizes is never transmitted ahead of its own counter
    /// state (panel seams#1). On any commit failure the in-memory value rolls
    /// back and no value is issued. Errors [`CounterError::RestoreFloorRequired`]
    /// while a state-absent restore has no floor set (§8.4).
    ///
    /// **Also the Layer 5 `msg_seq` epoch source (L5 §4, amendment A3).** A
    /// value returned here is fresh (strictly greater than every value issued
    /// before it, across clean and unclean restarts and across a floored
    /// restore — floor + 1024 + 1 at least) and already durable, so an epoch
    /// minted from it can never repeat for new content. L5 mints one per store
    /// open under its §4 conditions; the call costs one fsync. Callers MUST
    /// treat [`CounterError::RestoreFloorRequired`] as "seal nothing": no
    /// announce, no epoch, no traffic until [`Self::set_restore_floor`] has
    /// committed (L5 keeps outbound items queued, reason `restore_pending`).
    pub fn next_announce_counter(&mut self) -> Result<u64, CounterError> {
        self.mark_running()?;
        if self.restore_pending {
            return Err(CounterError::RestoreFloorRequired);
        }
        let next = self
            .announce_last
            .checked_add(1)
            .ok_or(CounterError::Overflow)?;
        self.announce_last = next;
        if let Err(e) = self.persist(DIRTY_RUNNING) {
            self.announce_last = next - 1; // next ≥ 1: this is the prior value
            return Err(e);
        }
        Ok(next)
    }

    /// §6.5 — the per-peer send counter for the next protected packet (the
    /// value carried in the AEAD associated data, §6.4). Batch-committed every
    /// N packets; a commit failure is reported and the caller MUST NOT
    /// transmit the packet (the in-memory counter stays advanced — a gap is
    /// harmless, a reused counter is not).
    pub fn next_send_counter(&mut self, peer: &NodeId) -> Result<u64, CounterError> {
        self.mark_running()?;
        let base = self.traffic_base;
        let entry = self.peer_entry(*peer, base)?;
        let next = entry
            .send_last
            .checked_add(1)
            .ok_or(CounterError::Overflow)?;
        entry.send_last = next;
        self.tick_and_maybe_commit()?;
        Ok(next)
    }

    /// §6.5 — offer a received packet counter: accepted (window advanced) or
    /// rejected as replay/too-old. Batch-committed with the send side.
    pub fn on_receive(&mut self, peer: &NodeId, counter: u64) -> Result<(), CounterError> {
        self.mark_running()?;
        let base = self.traffic_base;
        let entry = self.peer_entry(*peer, base)?;
        entry.window.offer(counter).map_err(CounterError::Window)?;
        self.tick_and_maybe_commit()?;
        Ok(())
    }

    /// L5 amendment A11 — offer a verified packet's counter to the §6.5
    /// window and report the verdict instead of rejecting.
    ///
    /// A counter the window accepts advances it exactly as
    /// [`Self::on_receive`] does (same window logic, same dirty-mark, same
    /// batch-commit tick) and returns [`Freshness::Fresh`]. A counter the
    /// window would reject — already seen, or [`WINDOW_SIZE`] or more behind
    /// high-water — returns `Ok(`[`Freshness::Stale`]`)` and changes NOTHING
    /// (not the window, not the batch tick, not the on-disk clean flag).
    ///
    /// Errors are the store's own, never a replay verdict:
    /// [`CounterError::TooManyPeers`] for a new peer past the guard, and
    /// commit/I-O failures — from the dirty-mark (nothing advanced) or from
    /// the batch commit (the in-memory window has already advanced, exactly
    /// as with `on_receive`).
    ///
    /// Call it only AFTER the packet's sender authentication has verified
    /// (L5 §3: `pair_mac` first — a forged sender must never move a peer's
    /// window).
    pub fn observe_counter(
        &mut self,
        peer: &NodeId,
        counter: u64,
    ) -> Result<Freshness, CounterError> {
        // Read-only verdict first: a stale counter touches no state at all. A
        // peer with no entry yet has an empty window, which accepts any
        // counter (§6.5), so only known peers can be stale.
        if let Some(t) = self.peers.get(peer) {
            let mut probe = t.window;
            if probe.offer(counter).is_err() {
                return Ok(Freshness::Stale);
            }
        }
        // Fresh: exactly the on_receive path.
        self.mark_running()?;
        let base = self.traffic_base;
        let entry = self.peer_entry(*peer, base)?;
        if entry.window.offer(counter).is_err() {
            // Unreachable after the probe above (single-writer store); kept
            // typed so no input can panic.
            return Ok(Freshness::Stale);
        }
        self.tick_and_maybe_commit()?;
        Ok(Freshness::Fresh)
    }

    /// §6.5 — clean-shutdown commit: uncommitted state is flushed and the
    /// file is marked clean, so the next [`Self::open`] applies NO forward
    /// jump. The host MUST call this on every orderly stop.
    pub fn shutdown(&mut self) -> Result<(), CounterError> {
        self.persist(CLEAN_SHUTDOWN)?;
        self.dirty_packets = 0;
        Ok(())
    }

    /// Force a batch commit now (the file stays marked dirty/running).
    pub fn commit(&mut self) -> Result<(), CounterError> {
        self.persist(DIRTY_RUNNING)?;
        self.dirty_packets = 0;
        Ok(())
    }

    /// The last committed announce counter (next announce uses +1).
    /// Read-only: never mints or commits. L5 §4 records this value at clean
    /// close and compares it at the next store open — a mismatch (e.g. a
    /// `store.db` restored onto a live identity) mints a new epoch (A3).
    pub fn announce_last(&self) -> u64 {
        self.announce_last
    }

    /// The init value new per-peer send counters start from (0, or
    /// [`RESTORE_JUMP`] after a state-absent restore, §8.4).
    pub fn traffic_base(&self) -> u64 {
        self.traffic_base
    }

    /// True while a state-absent restore has no floor set (§8.4).
    pub fn is_restore_pending(&self) -> bool {
        self.restore_pending
    }

    /// Per-peer traffic state, if the peer is known.
    pub fn peer_traffic(&self, peer: &NodeId) -> Option<PeerTraffic> {
        self.peers.get(peer).copied()
    }

    /// Number of tracked traffic peers.
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// The batch-commit interval N in packets (§6.5).
    pub fn batch_n(&self) -> u64 {
        self.batch_n
    }

    /// Set the batch-commit interval N. Takes effect immediately in memory
    /// and is persisted at the next commit — call [`Self::commit`] to make a
    /// changed N durable now (the unclean-reboot jump J reads the persisted
    /// N, keeping J ≥ N true for the state actually on disk).
    pub fn set_batch_n(&mut self, n: NonZeroU64) {
        self.batch_n = n.get();
    }

    fn peer_entry(&mut self, id: NodeId, base: u64) -> Result<&mut PeerTraffic, CounterError> {
        if !self.peers.contains_key(&id) && self.peers.len() >= MAX_COUNTER_PEERS {
            return Err(CounterError::TooManyPeers);
        }
        Ok(self.peers.entry(id).or_insert(PeerTraffic {
            send_last: base,
            window: ReceiveWindow::new(),
        }))
    }

    fn tick_and_maybe_commit(&mut self) -> Result<(), CounterError> {
        self.dirty_packets += 1;
        if self.dirty_packets >= self.batch_n {
            self.commit()?;
        }
        Ok(())
    }

    /// §6.5 — the unclean-reboot conservative forward jump, J = N (≥ N), on
    /// send-side counters and receive-side high-waters alike. Never touches
    /// the announce counter (§6.2 sync-commit makes that unnecessary).
    fn apply_unclean_jump(&mut self) {
        let jump = self.batch_n;
        for entry in self.peers.values_mut() {
            entry.send_last = entry.send_last.saturating_add(jump);
            entry.window.jump_forward(jump);
        }
        // §6.5 — the jump MUST cover every uncommitted emission, including
        // sends to a peer first contacted fewer than N operations before the
        // crash (that peer is ABSENT from the persisted file, so the per-peer
        // loop above cannot reach it). Raising the base by J makes any peer
        // created after this reopen start above those lost counters
        // (reviewer fix, wave-3 finding T8#1: without it such a peer's send
        // counter restarted at the old base and its first N packets were
        // dropped as replays).
        self.traffic_base = self.traffic_base.saturating_add(jump);
    }

    /// §13 — atomic write (temp + fsync + rename + dir fsync), 0600.
    fn persist(&mut self, clean_flag: u8) -> Result<(), CounterError> {
        self.clean_on_disk = clean_flag == CLEAN_SHUTDOWN;
        let mut buf = Vec::with_capacity(HEADER_LEN + self.peers.len() * PEER_RECORD_LEN);
        buf.extend_from_slice(&MAGIC);
        buf.push(FORMAT_VERSION);
        buf.push(clean_flag);
        buf.extend_from_slice(&self.announce_last.to_le_bytes());
        buf.extend_from_slice(&self.traffic_base.to_le_bytes());
        buf.extend_from_slice(&self.batch_n.to_le_bytes());
        // Bounded by MAX_COUNTER_PEERS at insert/parse.
        buf.extend_from_slice(&(self.peers.len() as u32).to_le_bytes());
        // Deterministic serialization: peers sorted by NodeId.
        let mut entries: Vec<(&NodeId, &PeerTraffic)> = self.peers.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (id, t) in entries {
            buf.extend_from_slice(id.as_bytes());
            buf.extend_from_slice(&t.send_last.to_le_bytes());
            buf.extend_from_slice(&t.window.high.to_le_bytes());
            buf.extend_from_slice(&t.window.bitmap.to_le_bytes());
        }
        atomic_write_private(&self.dir, COUNTERS_FILE, &buf)?;
        Ok(())
    }

    /// Flip the persisted flag to DIRTY_RUNNING before the first emission
    /// after a clean state (one fsync per session start). Without this, sends
    /// to a never-persisted peer before a crash would be invisible to
    /// `open()`'s unclean detection (wave-3 reviewer fix, T8 finding).
    fn mark_running(&mut self) -> Result<(), CounterError> {
        if self.clean_on_disk {
            self.persist(DIRTY_RUNNING)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Persistence helpers
// ---------------------------------------------------------------------------

struct ParsedFile {
    clean: bool,
    announce_last: u64,
    traffic_base: u64,
    batch_n: u64,
    peers: HashMap<NodeId, PeerTraffic>,
}

/// Fail-closed parse of the `counters` file (§12 rules 1/7): exact layout,
/// exact length, bounded peer count. The one deliberate exception is the
/// clean flag, which fails SAFE: any value ≠ 0x01 reads as unclean (cost: a
/// harmless forward jump), never as corrupt.
fn parse_file(bytes: &[u8]) -> Result<ParsedFile, CounterError> {
    if bytes.len() < HEADER_LEN {
        return Err(CounterError::Corrupt("truncated header"));
    }
    if !bytes.starts_with(&MAGIC) {
        return Err(CounterError::Corrupt("bad magic"));
    }
    if bytes[4] != FORMAT_VERSION {
        return Err(CounterError::Corrupt("unknown format version"));
    }
    let clean = bytes[5] == CLEAN_SHUTDOWN;
    let announce_last = u64::from_le_bytes(bytes[6..14].try_into().expect("8-byte slice"));
    let traffic_base = u64::from_le_bytes(bytes[14..22].try_into().expect("8-byte slice"));
    let batch_n = u64::from_le_bytes(bytes[22..30].try_into().expect("8-byte slice"));
    if batch_n == 0 {
        return Err(CounterError::Corrupt("zero batch interval"));
    }
    let count = u32::from_le_bytes(bytes[30..34].try_into().expect("4-byte slice")) as usize;
    if count > MAX_COUNTER_PEERS {
        return Err(CounterError::Corrupt("peer count over guard"));
    }
    if bytes.len() != HEADER_LEN + count * PEER_RECORD_LEN {
        return Err(CounterError::Corrupt("length mismatch"));
    }
    let mut peers = HashMap::with_capacity(count);
    for i in 0..count {
        let off = HEADER_LEN + i * PEER_RECORD_LEN;
        let rec = &bytes[off..off + PEER_RECORD_LEN];
        let id = NodeId::from(<[u8; 32]>::try_from(&rec[..32]).expect("32-byte slice"));
        let send_last = u64::from_le_bytes(rec[32..40].try_into().expect("8-byte slice"));
        let high = u64::from_le_bytes(rec[40..48].try_into().expect("8-byte slice"));
        let bitmap = u32::from_le_bytes(rec[48..52].try_into().expect("4-byte slice"));
        peers.insert(
            id,
            PeerTraffic {
                send_last,
                window: ReceiveWindow { high, bitmap },
            },
        );
    }
    Ok(ParsedFile {
        clean,
        announce_last,
        traffic_base,
        batch_n,
        peers,
    })
}

/// §13 — atomic identity-dir write: temp file (0600) + fsync + rename + dir
/// fsync, so no partial state is ever visible at the target path and the
/// rename itself is durable. Shared by the counter and pinning stores
/// (crate-internal).
pub(crate) fn atomic_write_private(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let tmp = dir.join(format!("{name}.tmp"));
    let target = dir.join(name);
    {
        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &target)?;
    // Durability of the rename itself (the directory entry).
    let d = File::open(dir)?;
    d.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU64;
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
                "id111-t4-counters-{tag}-{}-{seq}-{nanos}",
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

    fn nz(n: u64) -> NonZeroU64 {
        NonZeroU64::new(n).unwrap()
    }

    fn peer(n: u8) -> NodeId {
        NodeId::from([n; 32])
    }

    // ------------------------------------------------------------------
    // §6.5 — window semantics.
    // ------------------------------------------------------------------

    #[test]
    fn window_accepts_slides_and_rejects() {
        let mut w = ReceiveWindow::new();
        w.offer(1).unwrap();
        assert_eq!(w.high(), 1);
        assert_eq!(w.bitmap(), 0b1);
        assert_eq!(w.offer(1), Err(WindowReject::Replayed));
        // Jump ahead: window slides, gap harmless.
        w.offer(3).unwrap();
        assert_eq!(w.high(), 3);
        assert_eq!(w.bitmap(), 0b101); // counters 3 and 1
                                       // In-window gap fill.
        w.offer(2).unwrap();
        assert_eq!(w.bitmap(), 0b111);
        assert_eq!(w.offer(2), Err(WindowReject::Replayed));
        // Far ahead: shift ≥ 32 resets the bitmap to just the new high.
        w.offer(40).unwrap();
        assert_eq!(w.high(), 40);
        assert_eq!(w.bitmap(), 0b1);
        // 40 − 8 = 32: exactly behind the window.
        assert_eq!(w.offer(8), Err(WindowReject::OutsideWindow));
        // 40 − 9 = 31: the trailing edge, still inside.
        w.offer(9).unwrap();
        assert_eq!(w.bitmap() & (1 << 31), 1 << 31);
        // Counter 0 is a legal value on a fresh window.
        let mut w0 = ReceiveWindow::new();
        w0.offer(0).unwrap();
        assert_eq!(w0.offer(0), Err(WindowReject::Replayed));
    }

    #[test]
    fn window_jump_forward_carries_bits() {
        let mut w = ReceiveWindow::new();
        w.offer(8).unwrap();
        w.offer(10).unwrap(); // high = 10, bitmap 0b101 (counters 10, 8)
        w.jump_forward(4);
        assert_eq!(w.high(), 14);
        assert_eq!(w.bitmap(), 0b1010000); // bit4 ⇔ 10, bit6 ⇔ 8
        assert_eq!(w.offer(10), Err(WindowReject::Replayed));
        assert_eq!(w.offer(8), Err(WindowReject::Replayed));
        w.offer(13).unwrap(); // unset bit inside the window: accepted
                              // A jump ≥ 32 clears the bitmap entirely.
        w.jump_forward(32);
        assert_eq!(w.bitmap(), 0);
        assert_eq!(w.high(), 46);
        assert_eq!(w.offer(13), Err(WindowReject::OutsideWindow));
    }

    // ------------------------------------------------------------------
    // §6.2 — announce counter: genesis, durability, sync-commit rule (§14).
    // ------------------------------------------------------------------

    #[test]
    fn announce_counter_genesis_and_durability() {
        let tmp = TempDir::new("genesis");
        let mut store = CounterStore::create(tmp.path()).unwrap();
        // §6.2: initial value 0; the genesis announce uses 1.
        assert_eq!(store.announce_last(), 0);
        assert_eq!(store.next_announce_counter().unwrap(), 1);
        assert_eq!(store.next_announce_counter().unwrap(), 2);
        // Simulate a crash: drop WITHOUT shutdown. The returned values must
        // already be durable — §14's sync-commit rule test: an announce
        // counter value is durable before its announce is transmitted.
        drop(store);
        let mut store = CounterStore::open(tmp.path()).unwrap();
        assert_eq!(store.announce_last(), 2);
        assert_eq!(store.next_announce_counter().unwrap(), 3);
    }

    #[test]
    fn announce_counter_survives_clean_shutdown() {
        let tmp = TempDir::new("clean-announce");
        let mut store = CounterStore::create(tmp.path()).unwrap();
        assert_eq!(store.next_announce_counter().unwrap(), 1);
        store.shutdown().unwrap();
        let mut store = CounterStore::open(tmp.path()).unwrap();
        assert_eq!(store.announce_last(), 1);
        assert_eq!(store.next_announce_counter().unwrap(), 2);
    }

    // ------------------------------------------------------------------
    // §6.5 — batch commit, clean/unclean restart, the J ≥ N forward jump.
    // ------------------------------------------------------------------

    #[test]
    fn unclean_reboot_jumps_send_and_receive_sides() {
        let tmp = TempDir::new("unclean");
        let a = peer(0xa1);
        {
            let mut store = CounterStore::create(tmp.path()).unwrap();
            store.set_batch_n(nz(8));
            // One send + one receive, committed: the jump below starts from
            // this on-disk state (uncommitted state is lost on a crash — the
            // §6.5 stale-window residual, handed to Layer 5 idempotency).
            assert_eq!(store.next_send_counter(&a).unwrap(), 1);
            store.on_receive(&a, 7).unwrap();
            store.commit().unwrap(); // also makes N = 8 durable for the jump
                                     // Three more sends, uncommitted (3 < N = 8 after the commit).
            assert_eq!(store.next_send_counter(&a).unwrap(), 2);
            assert_eq!(store.next_send_counter(&a).unwrap(), 3);
            assert_eq!(store.next_send_counter(&a).unwrap(), 4);
            // Dropped without shutdown ⇒ unclean at next open.
        }
        let mut store = CounterStore::open(tmp.path()).unwrap();
        // Send side jumped past any uncommitted emission: 1 + J(8) → next 10,
        // safely past the uncommitted 2, 3, 4.
        let t = store.peer_traffic(&a).unwrap();
        assert_eq!(t.send_last(), 9);
        assert_eq!(store.next_send_counter(&a).unwrap(), 10);
        // Receive side: high-water 7 + J(8) = 15, bitmap carried (bit8 ⇔ 7).
        let w = store.peer_traffic(&a).unwrap().receive_window();
        assert_eq!(w.high(), 15);
        assert_eq!(w.bitmap(), 1 << 8);
        // The committed-and-consumed counter 7 stays rejected after the jump.
        assert!(matches!(
            store.on_receive(&a, 7),
            Err(CounterError::Window(WindowReject::Replayed))
        ));
        // The gap is harmless: 16 is accepted, sliding the window.
        store.on_receive(&a, 16).unwrap();
        assert_eq!(store.peer_traffic(&a).unwrap().receive_window().high(), 16);
    }

    #[test]
    fn batch_commit_fires_at_n_packets() {
        let tmp = TempDir::new("batchn");
        let a = peer(0xa2);
        {
            let mut store = CounterStore::create(tmp.path()).unwrap();
            store.set_batch_n(nz(4));
            store.commit().unwrap();
            for want in 1..=4u64 {
                assert_eq!(store.next_send_counter(&a).unwrap(), want);
            }
            // The 4th packet hit N: send_last = 4 is on disk. Drop unclean.
        }
        let mut store = CounterStore::open(tmp.path()).unwrap();
        // Unclean jump J = 4 from the committed 4: next = 4 + 4 + 1 = 9.
        assert_eq!(store.peer_traffic(&a).unwrap().send_last(), 8);
        assert_eq!(store.next_send_counter(&a).unwrap(), 9);
    }

    #[test]
    fn clean_shutdown_commits_and_skips_the_jump() {
        let tmp = TempDir::new("clean");
        let a = peer(0xa3);
        {
            let mut store = CounterStore::create(tmp.path()).unwrap();
            assert_eq!(store.next_send_counter(&a).unwrap(), 1);
            assert_eq!(store.next_send_counter(&a).unwrap(), 2);
            assert_eq!(store.next_send_counter(&a).unwrap(), 3);
            store.on_receive(&a, 5).unwrap();
            store.shutdown().unwrap(); // §6.5: batch-commit on clean shutdown
        }
        let store = CounterStore::open(tmp.path()).unwrap();
        let t = store.peer_traffic(&a).unwrap();
        // No jump after a clean shutdown: exactly the committed state.
        assert_eq!(t.send_last(), 3);
        assert_eq!(t.receive_window().high(), 5);
    }

    #[test]
    fn clean_flag_garbage_reads_unclean_not_corrupt() {
        let tmp = TempDir::new("flagbyte");
        let a = peer(0xa4);
        {
            let mut store = CounterStore::create(tmp.path()).unwrap();
            assert_eq!(store.next_send_counter(&a).unwrap(), 1);
            store.shutdown().unwrap();
        }
        // A torn flag byte: 0x77 is neither clean nor a parse error — the
        // fail-safe reading is "unclean", costing one harmless jump.
        let path = tmp.path().join(COUNTERS_FILE);
        let mut bytes = fs::read(&path).unwrap();
        bytes[5] = 0x77;
        fs::write(&path, &bytes).unwrap();
        let mut store = CounterStore::open(tmp.path()).unwrap();
        // J = DEFAULT_BATCH_N = 64: 1 + 64 → next 66.
        assert_eq!(store.next_send_counter(&a).unwrap(), 66);
    }

    // ------------------------------------------------------------------
    // §8.4 — restore: state-absent floor + jump, no-announce-before-floor.
    // ------------------------------------------------------------------

    #[test]
    fn restore_pending_floor_and_jump() {
        let tmp = TempDir::new("restore");
        let mut store = CounterStore::restore_pending(tmp.path()).unwrap();
        // §8.4: the node MUST NOT announce until a counter floor is set.
        assert!(matches!(
            store.next_announce_counter(),
            Err(CounterError::RestoreFloorRequired)
        ));
        // Gossip learned the node's prior announces topped out at 100.
        store.set_restore_floor(100).unwrap();
        // §8.4: counter = floor + 1024, committed before the call returns.
        assert_eq!(store.announce_last(), 100 + RESTORE_JUMP);
        assert_eq!(
            store.next_announce_counter().unwrap(),
            100 + RESTORE_JUMP + 1
        );
        // §8.4: the same jump applies to per-peer traffic send-counters on
        // new sessions — a fresh peer's first packet uses 1024 + 1.
        let a = peer(0xb1);
        assert_eq!(store.traffic_base(), RESTORE_JUMP);
        assert_eq!(store.next_send_counter(&a).unwrap(), RESTORE_JUMP + 1);
        store.shutdown().unwrap();

        // State present after that: reused as-is (§8.4).
        let mut store = CounterStore::open(tmp.path()).unwrap();
        assert!(!store.is_restore_pending());
        assert_eq!(store.announce_last(), 100 + RESTORE_JUMP + 1);
        assert_eq!(
            store.next_announce_counter().unwrap(),
            100 + RESTORE_JUMP + 2
        );
        // traffic_base persists: another new peer still starts past the jump.
        let b = peer(0xb2);
        assert_eq!(store.next_send_counter(&b).unwrap(), RESTORE_JUMP + 1);
        // The floor is set exactly once.
        assert!(matches!(
            store.set_restore_floor(50),
            Err(CounterError::FloorAlreadySet)
        ));
    }

    #[test]
    fn restore_floor_zero_only_for_never_deployed() {
        let tmp = TempDir::new("floor0");
        let mut store = CounterStore::restore_pending(tmp.path()).unwrap();
        store.set_restore_floor(0).unwrap();
        assert_eq!(store.announce_last(), RESTORE_JUMP);
        assert_eq!(store.next_announce_counter().unwrap(), RESTORE_JUMP + 1);
    }

    #[test]
    fn restore_floor_overflow_fails_closed() {
        let tmp = TempDir::new("floor-overflow");
        let mut store = CounterStore::restore_pending(tmp.path()).unwrap();
        // The floor is gossip/operator input — untrusted; the add is checked.
        assert!(matches!(
            store.set_restore_floor(u64::MAX),
            Err(CounterError::Overflow)
        ));
        // Still pending: no announce authorized.
        assert!(matches!(
            store.next_announce_counter(),
            Err(CounterError::RestoreFloorRequired)
        ));
    }

    // ------------------------------------------------------------------
    // §13/§12 — file hygiene: never-overwrite, fail-closed parse, 0600.
    // ------------------------------------------------------------------

    #[test]
    fn open_missing_create_twice_restore_over_existing() {
        let tmp = TempDir::new("triad");
        assert!(matches!(
            CounterStore::open(tmp.path()),
            Err(CounterError::NotFound)
        ));
        CounterStore::create(tmp.path()).unwrap();
        assert!(matches!(
            CounterStore::create(tmp.path()),
            Err(CounterError::AlreadyExists)
        ));
        assert!(matches!(
            CounterStore::restore_pending(tmp.path()),
            Err(CounterError::AlreadyExists)
        ));
    }

    #[test]
    fn corrupt_file_rejected_fail_closed() {
        let good = {
            let tmp = TempDir::new("corrupt-src");
            CounterStore::create(tmp.path()).unwrap();
            fs::read(tmp.path().join(COUNTERS_FILE)).unwrap()
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
            ("zero batch interval", {
                let mut b = good.clone();
                for x in &mut b[22..30] {
                    *x = 0;
                }
                b
            }),
            ("peer count over guard", {
                let mut b = good.clone();
                b[30..34].copy_from_slice(&(u32::MAX).to_le_bytes());
                b
            }),
            ("length mismatch (trailing garbage)", {
                let mut b = good.clone();
                b.push(0x00);
                b
            }),
        ];
        for (name, bytes) in cases {
            let tmp = TempDir::new(name);
            fs::write(tmp.path().join(COUNTERS_FILE), &bytes).unwrap();
            assert!(
                matches!(
                    CounterStore::open(tmp.path()),
                    Err(CounterError::Corrupt(_))
                ),
                "case {name} must be rejected as corrupt"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn counters_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new("perms");
        CounterStore::create(tmp.path()).unwrap();
        let mode = std::fs::metadata(tmp.path().join(COUNTERS_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        // No temp file lingers after an atomic write.
        assert!(!tmp.path().join(format!("{COUNTERS_FILE}.tmp")).exists());
    }

    #[test]
    fn peer_guard_fails_closed() {
        let tmp = TempDir::new("toomany");
        let mut store = CounterStore::create(tmp.path()).unwrap();
        // Distinct NodeIds from a counter pattern in the first two bytes.
        for i in 0..MAX_COUNTER_PEERS {
            let mut b = [0u8; 32];
            b[0..2].copy_from_slice(&(i as u16).to_le_bytes());
            b[31] = 0xff;
            store.next_send_counter(&NodeId::from(b)).unwrap();
        }
        let mut b = [0u8; 32];
        b[31] = 0xaa;
        assert!(matches!(
            store.next_send_counter(&NodeId::from(b)),
            Err(CounterError::TooManyPeers)
        ));
    }
    // ------------------------------------------------------------------
    // L5 amendment A11 — observe_counter: verdict, never rejection.
    // ------------------------------------------------------------------

    #[test]
    fn observe_counter_mirrors_on_receive_without_rejecting() {
        // Twin stores fed the same counters: observe_counter's verdict is
        // Fresh exactly where on_receive accepts, Stale exactly where it
        // rejects, and the windows stay identical after every step.
        let (ta, tb) = (TempDir::new("obs-a"), TempDir::new("obs-b"));
        let mut obs = CounterStore::create(ta.path()).unwrap();
        let mut rcv = CounterStore::create(tb.path()).unwrap();
        let a = peer(0xc1);
        for c in [1u64, 1, 3, 2, 2, 40, 8, 9, 9, 41, 0, 1000, 700, 999, 999] {
            let verdict = obs.observe_counter(&a, c).unwrap();
            let accepted = rcv.on_receive(&a, c);
            match verdict {
                Freshness::Fresh => assert!(accepted.is_ok(), "counter {c}"),
                Freshness::Stale => assert!(
                    matches!(accepted, Err(CounterError::Window(_))),
                    "counter {c}"
                ),
            }
            assert_eq!(
                obs.peer_traffic(&a).unwrap().receive_window(),
                rcv.peer_traffic(&a).unwrap().receive_window(),
                "window parity after {c}"
            );
        }
    }

    #[test]
    fn observe_counter_custody_late_arrival_is_stale_not_an_error() {
        // L5 §4: a custody item can arrive hundreds of counters behind a
        // packet that took the direct path — reported, never rejected.
        let tmp = TempDir::new("obs-late");
        let mut store = CounterStore::create(tmp.path()).unwrap();
        let a = peer(0xc2);
        assert_eq!(store.observe_counter(&a, 1000).unwrap(), Freshness::Fresh);
        let before = store.peer_traffic(&a).unwrap();
        assert_eq!(store.observe_counter(&a, 700).unwrap(), Freshness::Stale);
        assert_eq!(store.observe_counter(&a, 1000).unwrap(), Freshness::Stale);
        assert_eq!(store.peer_traffic(&a).unwrap(), before, "no state change");
        // An unseen slot inside the window is still fresh.
        assert_eq!(store.observe_counter(&a, 999).unwrap(), Freshness::Fresh);
        let w = store.peer_traffic(&a).unwrap().receive_window();
        assert_eq!((w.high(), w.bitmap()), (1000, 0b11));
        // A first counter from an unknown peer is always fresh (empty window).
        assert_eq!(
            store.observe_counter(&peer(0xc3), 0).unwrap(),
            Freshness::Fresh
        );
    }

    #[test]
    fn observe_counter_stale_touches_neither_disk_flag_nor_batch_tick() {
        let tmp = TempDir::new("obs-nostate");
        let path = tmp.path().join(COUNTERS_FILE);
        let a = peer(0xc4);
        let mut store = CounterStore::create(tmp.path()).unwrap();
        store.set_batch_n(nz(2));
        assert_eq!(store.observe_counter(&a, 100).unwrap(), Freshness::Fresh);
        store.shutdown().unwrap(); // flag CLEAN, batch tick reset
        let clean = fs::read(&path).unwrap();
        assert_eq!(clean[5], CLEAN_SHUTDOWN);
        // Stale observations: the file is not rewritten (still CLEAN — a
        // crash now is correctly a clean state) and no batch tick accrues.
        for _ in 0..5 {
            assert_eq!(store.observe_counter(&a, 100).unwrap(), Freshness::Stale);
            assert_eq!(store.observe_counter(&a, 50).unwrap(), Freshness::Stale);
        }
        assert_eq!(fs::read(&path).unwrap(), clean);
        // The first fresh one marks the file running, exactly as on_receive.
        assert_eq!(store.observe_counter(&a, 101).unwrap(), Freshness::Fresh);
        assert_eq!(fs::read(&path).unwrap()[5], DIRTY_RUNNING);
        // One fresh tick so far (N = 2): the 2nd fresh one batch-commits.
        let persisted = |p: &Path| parse_file(&fs::read(p).unwrap()).unwrap().peers[&a];
        assert_eq!(persisted(&path).window.high(), 100);
        assert_eq!(store.observe_counter(&a, 102).unwrap(), Freshness::Fresh);
        assert_eq!(persisted(&path).window.high(), 102);
    }

    #[test]
    fn observe_counter_peer_guard_is_a_store_error_not_a_verdict() {
        let tmp = TempDir::new("obs-guard");
        let mut store = CounterStore::create(tmp.path()).unwrap();
        for i in 0..MAX_COUNTER_PEERS {
            let mut b = [0u8; 32];
            b[0..2].copy_from_slice(&(i as u16).to_le_bytes());
            b[31] = 0xee;
            store.observe_counter(&NodeId::from(b), 1).unwrap();
        }
        assert!(matches!(
            store.observe_counter(&peer(0x01), 1),
            Err(CounterError::TooManyPeers)
        ));
    }

    // ------------------------------------------------------------------
    // L5 amendment A3 — next_announce_counter as the msg_seq epoch source.
    // ------------------------------------------------------------------

    #[test]
    fn epoch_source_is_fresh_and_durable_across_restarts() {
        let tmp = TempDir::new("epoch");
        let mut store = CounterStore::create(tmp.path()).unwrap();
        let e1 = store.next_announce_counter().unwrap();
        assert_eq!(store.announce_last(), e1);
        // Unclean stop: the minted epoch was already durable.
        drop(store);
        let mut store = CounterStore::open(tmp.path()).unwrap();
        assert_eq!(store.announce_last(), e1);
        let e2 = store.next_announce_counter().unwrap();
        assert!(e2 > e1);
        // Clean stop: announce_last is what L5 compares at the next open.
        store.shutdown().unwrap();
        let mut store = CounterStore::open(tmp.path()).unwrap();
        assert_eq!(store.announce_last(), e2);
        assert!(store.next_announce_counter().unwrap() > e2);
    }

    #[test]
    fn epoch_source_refuses_while_restore_pending_and_restores_never_collide() {
        // First words-only restore: nothing may be sealed until the floor
        // commits — the epoch source refuses and announce_last stays put.
        let t1 = TempDir::new("epoch-r1");
        let mut r1 = CounterStore::restore_pending(t1.path()).unwrap();
        for _ in 0..3 {
            assert!(matches!(
                r1.next_announce_counter(),
                Err(CounterError::RestoreFloorRequired)
            ));
        }
        assert_eq!(r1.announce_last(), 0);
        assert!(!t1.path().join(COUNTERS_FILE).exists(), "nothing committed");
        r1.set_restore_floor(500).unwrap();
        let first_epoch = r1.next_announce_counter().unwrap();
        assert_eq!(first_epoch, 500 + RESTORE_JUMP + 1);
        let last_of_r1 = r1.next_announce_counter().unwrap();

        // A second words-only restore whose floor is learned from the first
        // run's announces (the §8.4 gossip floor) mints strictly above every
        // epoch the first restore ever used (L2 amendment A3).
        let t2 = TempDir::new("epoch-r2");
        let mut r2 = CounterStore::restore_pending(t2.path()).unwrap();
        r2.set_restore_floor(last_of_r1).unwrap();
        let e = r2.next_announce_counter().unwrap();
        assert!(e > last_of_r1 && e > first_epoch);
    }

    /// Wave-3 reviewer regression (T8 finding): a peer first contacted fewer
    /// than N operations before an UNCLEAN reboot is absent from the persisted
    /// file; its send counter MUST still resume above every uncommitted
    /// emission (§6.5 "J ≥ N covers any uncommitted emission").
    #[test]
    fn unclean_reboot_covers_peers_absent_from_persisted_file() {
        let tmp = TempDir::new("unknown-peer-jump");
        let dir = tmp.0.clone();
        let peer = NodeId::from([0x42u8; 32]);
        let mut store = CounterStore::create(&dir).unwrap();
        let n = store.batch_n();
        // Three sends to a brand-new peer, no batch commit reached, no clean
        // shutdown: the peer never hits disk.
        let c1 = store.next_send_counter(&peer).unwrap();
        let c2 = store.next_send_counter(&peer).unwrap();
        let c3 = store.next_send_counter(&peer).unwrap();
        assert_eq!((c1, c2, c3), (1, 2, 3));
        drop(store); // simulated crash — no shutdown()

        let mut reopened = CounterStore::open(&dir).unwrap();
        assert!(
            reopened.peer_traffic(&peer).is_none(),
            "peer was never persisted"
        );
        let next = reopened.next_send_counter(&peer).unwrap();
        assert!(
            next > c3 && next > n,
            "first post-crash send ({next}) must exceed every uncommitted counter (≤{c3}) and the jump N={n}"
        );
    }
}
