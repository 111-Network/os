//! Agent activity journal: JSONL on disk + broadcast for the SSE stream.
//!
//! The file is the durable record (`events.jsonl`, 1 MiB FIFO). On start the
//! last [`RECENT_CAP`] valid lines are replayed into memory and `seq`
//! continues from the highest one seen, so a restart never resets the
//! stream. Disk failures degrade to memory-only; they never panic or block
//! the agent.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

const RECENT_CAP: usize = 1000;
/// Trim when the file grows past this.
const MAX_BYTES: u64 = 1024 * 1024;
/// Trim down to this (half, so a trim is amortized over many appends).
const TRIM_TO: u64 = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub schema: u8,
    pub seq: u64,
    pub ts: String,
    pub kind: String,
    pub detail: serde_json::Value,
}

#[derive(Clone)]
pub struct Journal {
    inner: Arc<Inner>,
}

struct Inner {
    tx: broadcast::Sender<Event>,
    path: PathBuf,
    state: Mutex<State>,
}

/// Everything an append mutates, behind one lock so seq order, file order,
/// memory order, and broadcast order are the same order.
struct State {
    seq: u64,
    recent: VecDeque<Event>,
    /// Approximate on-disk size; avoids a stat per append.
    bytes: u64,
}

impl Journal {
    /// Open the journal at `path`, replaying whatever valid history is there.
    pub fn new(path: PathBuf) -> Self {
        let (tx, _) = broadcast::channel(256);
        let state = replay(&path);
        Self {
            inner: Arc::new(Inner {
                tx,
                path,
                state: Mutex::new(state),
            }),
        }
    }

    pub fn append(&self, kind: &str, detail: serde_json::Value) {
        let mut st = self.lock();
        st.seq += 1;
        let ev = Event {
            schema: 1,
            seq: st.seq,
            ts: now_rfc3339(),
            kind: kind.to_string(),
            detail,
        };
        if let Ok(line) = serde_json::to_string(&ev) {
            if append_line(&self.inner.path, &line) {
                st.bytes += line.len() as u64 + 1;
                if st.bytes > MAX_BYTES {
                    // On failure assume we are at the trim target, so the
                    // retry comes after another ~512 KiB, not every append.
                    st.bytes = trim_file(&self.inner.path).unwrap_or(TRIM_TO);
                }
            }
        }
        st.recent.push_back(ev.clone());
        while st.recent.len() > RECENT_CAP {
            st.recent.pop_front();
        }
        let _ = self.inner.tx.send(ev); // lagging/absent subscribers are fine
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.tx.subscribe()
    }

    pub fn tail(&self, n: usize) -> Vec<Event> {
        tail_of(&self.lock().recent, n)
    }

    /// The last `n` events plus a live receiver, taken under one lock: no
    /// event is lost or duplicated between the replay and the live part.
    pub fn replay_and_subscribe(&self, n: usize) -> (Vec<Event>, broadcast::Receiver<Event>) {
        let st = self.lock();
        let rx = self.inner.tx.subscribe();
        (tail_of(&st.recent, n), rx)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic elsewhere must not take the journal down with it.
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn tail_of(recent: &VecDeque<Event>, n: usize) -> Vec<Event> {
    recent.iter().rev().take(n).rev().cloned().collect()
}

/// Load the last [`RECENT_CAP`] valid events and the highest seq. Anything
/// unreadable or malformed is skipped: the journal is advisory history.
fn replay(path: &Path) -> State {
    let mut st = State {
        seq: 0,
        recent: VecDeque::new(),
        bytes: 0,
    };
    let Ok(data) = std::fs::read(path) else {
        return st;
    };
    st.bytes = data.len() as u64;
    // A crash mid-write leaves a line without its newline; terminate it so
    // the next append does not glue itself onto the fragment.
    if data.last().is_some_and(|b| *b != b'\n') && append_line(path, "") {
        st.bytes += 1;
    }
    for line in data.split(|b| *b == b'\n') {
        if let Ok(ev) = serde_json::from_slice::<Event>(line) {
            st.seq = st.seq.max(ev.seq);
            st.recent.push_back(ev);
            if st.recent.len() > RECENT_CAP {
                st.recent.pop_front();
            }
        }
    }
    st
}

/// Append `line` + `\n` in one write. Returns whether it reached the file.
fn append_line(path: &Path, line: &str) -> bool {
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return false;
    };
    f.write_all(format!("{line}\n").as_bytes()).is_ok()
}

/// Rewrite the file (tmp + rename) keeping the newest whole lines that fit in
/// [`TRIM_TO`], and always at least the last one. Returns the new size.
fn trim_file(path: &Path) -> std::io::Result<u64> {
    let data = std::fs::read(path)?;
    let lines: Vec<&[u8]> = data
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    let mut kept = 0u64;
    let mut first = lines.len();
    for (i, l) in lines.iter().enumerate().rev() {
        let cost = l.len() as u64 + 1;
        if first < lines.len() && kept + cost > TRIM_TO {
            break;
        }
        kept += cost;
        first = i;
    }
    let mut out = Vec::with_capacity(kept as usize);
    for l in &lines[first..] {
        out.extend_from_slice(l);
        out.push(b'\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    let res = std::fs::write(&tmp, &out).and_then(|_| std::fs::rename(&tmp, path));
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res.map(|_| out.len() as u64)
}

/// RFC3339 UTC timestamp; empty string is acceptable on clock fault.
pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}
