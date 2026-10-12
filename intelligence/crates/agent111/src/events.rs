//! Agent activity journal: JSONL on disk + broadcast for the SSE stream.
//! T0: append + tail + subscribe. T5 adds the 1 MiB FIFO trim and file replay.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

const RECENT_CAP: usize = 1000;

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
    seq: AtomicU64,
    recent: Mutex<VecDeque<Event>>,
    path: PathBuf,
}

impl Journal {
    pub fn new(path: PathBuf) -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            inner: Arc::new(Inner {
                tx,
                seq: AtomicU64::new(0),
                recent: Mutex::new(VecDeque::new()),
                path,
            }),
        }
    }

    pub fn append(&self, kind: &str, detail: serde_json::Value) {
        let ev = Event {
            schema: 1,
            seq: self.inner.seq.fetch_add(1, Ordering::SeqCst) + 1,
            ts: now_rfc3339(),
            kind: kind.to_string(),
            detail,
        };
        if let Ok(line) = serde_json::to_string(&ev) {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.inner.path)
            {
                use std::io::Write;
                let _ = writeln!(f, "{line}");
            }
        }
        {
            let mut recent = self.inner.recent.lock().unwrap();
            recent.push_back(ev.clone());
            while recent.len() > RECENT_CAP {
                recent.pop_front();
            }
        }
        let _ = self.inner.tx.send(ev); // lagging/absent subscribers are fine
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.tx.subscribe()
    }

    pub fn tail(&self, n: usize) -> Vec<Event> {
        let recent = self.inner.recent.lock().unwrap();
        recent.iter().rev().take(n).rev().cloned().collect()
    }
}

/// RFC3339 UTC timestamp; empty string is acceptable on clock fault.
pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}
