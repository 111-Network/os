//! §4 — transmit sequence state: `tx_state` keyed `(peer, stream)` (stream 0
//! is the pair's 1:1 stream; never a global counter), `n` strictly
//! increasing within the run's epoch and **committed with
//! `synchronous = FULL` before it is returned** — i.e. before the first
//! fragment can be sealed.
//!
//! `n` starts at 1 in every epoch (so `prefix = 0` means "nothing yet" on the
//! receive side and no data `msg_seq` ever equals the control `0 ‖ 0`), and
//! never exceeds 2^32 − 1: past that the allocation fails with
//! [`StoreError::EpochExhausted`] and the caller mints a new epoch.
//! Numbers are handed out only by [`Store::queue_own_message`], together
//! with the content they number.

use rusqlite::{params, Connection, OptionalExtension};

use super::rows::{insert_message, NewMessage};
use super::{to_i64, to_u64, Durability, Result, Store, StoreError, N_MAX};

/// An allocated `msg_seq` (§4: `epoch(varint) ‖ n(varint)` on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Seq {
    pub epoch: u64,
    pub n: u64,
}

/// Whether any stream in `epoch` has exhausted its `n` space (§4 condition
/// "n would wrap 2^32").
pub(crate) fn epoch_exhausted(c: &Connection, epoch: u64) -> Result<bool> {
    let max: Option<i64> = c.query_row(
        "SELECT max(next_n) FROM tx_state WHERE epoch = ?1",
        [to_i64(epoch, "epoch")?],
        |r| r.get(0),
    )?;
    Ok(match max {
        Some(m) => to_u64(m, "tx_state.next_n")? > N_MAX,
        None => false,
    })
}

/// Allocate the next `n` for `(peer, stream)` under `epoch` inside the
/// caller's transaction.
pub(crate) fn alloc_in(c: &Connection, epoch: u64, peer: &[u8; 32], stream: u64) -> Result<Seq> {
    let stream_i = to_i64(stream, "stream")?;
    let epoch_i = to_i64(epoch, "epoch")?;
    let row: Option<(i64, i64)> = c
        .query_row(
            "SELECT epoch, next_n FROM tx_state WHERE peer = ?1 AND stream = ?2",
            params![&peer[..], stream_i],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let n = match row {
        Some((e, next)) if e == epoch_i => to_u64(next, "tx_state.next_n")?,
        // A recorded epoch beyond the run's epoch cannot happen (epochs only
        // grow); refuse rather than risk reusing a number.
        Some((e, _)) if e > epoch_i => return Err(StoreError::Corrupt("tx_state.epoch")),
        _ => 1,
    };
    if n > N_MAX {
        return Err(StoreError::EpochExhausted);
    }
    c.execute(
        "INSERT INTO tx_state (peer, stream, epoch, next_n) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (peer, stream) DO UPDATE SET epoch = excluded.epoch, next_n = excluded.next_n",
        params![&peer[..], stream_i, epoch_i, to_i64(n + 1, "n")?],
    )?;
    Ok(Seq { epoch, n })
}

impl Store {
    /// §4: allocate a `msg_seq` and store the outbound message (state
    /// `queued`) in one FULL transaction, so the number is never reused and
    /// never detached from its content. Returns `(message id, seq)`.
    ///
    /// This is the **only** allocation path: an allocated `n` is never
    /// abandoned (a message cancelled before it leaves is sent as an empty
    /// tombstone by the fabric), so no permanent hole can wedge the
    /// receiver's bounded range list. There is no public bare allocator:
    ///
    /// ```compile_fail
    /// fn burn(s: &mut mesh111::store::Store) {
    ///     let _ = s.allocate_seq(&[0; 32], 0);
    /// }
    /// ```
    ///
    /// ```no_run
    /// fn queue(s: &mut mesh111::store::Store, m: &mesh111::store::NewMessage<'_>) {
    ///     let _ = s.queue_own_message(m);
    /// }
    /// ```
    ///
    /// Refused while a mint is owed ([`StoreError::EpochNotMinted`]) or a
    /// restore is pending ([`StoreError::RestorePending`]).
    pub fn queue_own_message(&mut self, msg: &NewMessage<'_>) -> Result<(i64, Seq)> {
        let epoch = self.ready_epoch()?;
        self.with_tx(Durability::Full, |tx| {
            let seq = alloc_in(tx, epoch, &msg.peer, msg.stream)?;
            let id = insert_message(tx, msg, seq, super::Dir::Own, super::MsgState::Queued)?;
            Ok((id, seq))
        })
    }

    /// The next `n` that would be allocated for `(peer, stream)` in the
    /// current epoch (diagnostics; allocates nothing).
    pub fn peek_next_n(&self, peer: &[u8; 32], stream: u64) -> Result<Option<Seq>> {
        let epoch = match self.epoch_state() {
            super::EpochState::Ready { epoch } => epoch,
            _ => return Ok(None),
        };
        let row: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT epoch, next_n FROM tx_state WHERE peer = ?1 AND stream = ?2",
                params![&peer[..], to_i64(stream, "stream")?],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let n = match row {
            Some((e, next)) if to_u64(e, "tx_state.epoch")? == epoch => {
                to_u64(next, "tx_state.next_n")?
            }
            _ => 1,
        };
        Ok(Some(Seq { epoch, n }))
    }
}
