//! `Message.at` (`4-interface/api/openapi.yaml`): "local wall-clock time this
//! node stored the message (own: queued; in: received). Display only".
//!
//! The store carries no wall clock (§4: nothing is ordered or decided by a
//! clock), so the daemon keeps this display fact beside it: the first time it
//! sees a message id (its own send, or the fabric's `New`/`Request` event)
//! it records `(id, unix seconds)` in `fabric/display-times` (0600, append-
//! only 16-byte records, little-endian `i64 id ‖ i64 secs`). A message the
//! daemon never saw arrive (an event lost to a lagging subscriber, or rows
//! from before this file existed) gets the time it is first shown. Bounded:
//! at most [`CAP`] entries are kept (newest ids); the file is compacted when
//! it grows past twice that, and a file larger than [`MAX_FILE`] is ignored.
//! Never used for ordering, freshness or anything cryptographic.
//!
//! **A second instance (Layer 6 §3):** foreign rows (`foreign_messages`) have
//! their own row-id space, so their "received" display time lives in
//! `fabric/display-times.foreign` ([`FOREIGN_TIMES_FILE`]): the same record
//! format, API and bounds, opened with [`DisplayTimes::open`] and
//! [`TimesFile::Foreign`], so a foreign id can never collide with a 111
//! message id. The file is chosen by a closed enum, never by a caller's
//! string.

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// File name inside the fabric directory (111 `messages` row ids).
pub const TIMES_FILE: &str = "display-times";
/// The second instance's file (Layer 6 §3: `foreign_messages` row ids).
pub const FOREIGN_TIMES_FILE: &str = "display-times.foreign";
/// Entries kept.
pub const CAP: usize = 100_000;
/// One record.
const REC: usize = 16;
/// Largest file read (compaction keeps it far below).
pub const MAX_FILE: u64 = (4 * CAP * REC) as u64;

/// Which display-times instance (module doc): the file is one of two fixed
/// names, never caller input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimesFile {
    /// [`TIMES_FILE`]: 111 `messages` row ids.
    Ours,
    /// [`FOREIGN_TIMES_FILE`]: `foreign_messages` row ids (Layer 6 §3).
    Foreign,
}

impl TimesFile {
    /// The file name inside the fabric directory.
    pub fn name(self) -> &'static str {
        match self {
            TimesFile::Ours => TIMES_FILE,
            TimesFile::Foreign => FOREIGN_TIMES_FILE,
        }
    }
}

/// Wall-clock now, Unix seconds (display only).
pub fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

/// RFC 3339 UTC of Unix seconds.
pub fn rfc3339(secs: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(secs)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".into())
}

/// The id → first-seen time map (see the module doc).
#[derive(Debug)]
pub struct DisplayTimes {
    path: Option<PathBuf>,
    map: HashMap<i64, i64>,
    records: usize,
}

impl DisplayTimes {
    /// In memory only (tests, a fabric that never started).
    pub fn memory() -> DisplayTimes {
        DisplayTimes {
            path: None,
            map: HashMap::new(),
            records: 0,
        }
    }

    /// Load `fabric_dir/display-times` (a missing, oversize or symlinked
    /// file starts empty).
    pub fn load(fabric_dir: &Path) -> DisplayTimes {
        DisplayTimes::open(fabric_dir, TimesFile::Ours)
    }

    /// Load `fabric_dir/<file>` — [`TIMES_FILE`] or [`FOREIGN_TIMES_FILE`]
    /// (a missing, oversize or symlinked file starts empty).
    pub fn open(fabric_dir: &Path, file: TimesFile) -> DisplayTimes {
        let path = fabric_dir.join(file.name());
        let mut t = DisplayTimes {
            path: Some(path.clone()),
            map: HashMap::new(),
            records: 0,
        };
        let ok = std::fs::symlink_metadata(&path)
            .is_ok_and(|m| m.file_type().is_file() && m.len() <= MAX_FILE);
        if ok {
            if let Ok(bytes) = std::fs::read(&path) {
                for r in bytes.as_chunks::<REC>().0 {
                    let id = i64::from_le_bytes(r[..8].try_into().unwrap_or([0; 8]));
                    let at = i64::from_le_bytes(r[8..].try_into().unwrap_or([0; 8]));
                    t.map.entry(id).or_insert(at);
                    t.records += 1;
                }
            }
        }
        t.trim();
        t
    }

    /// The recorded time of `id`, recording `now` when it has none.
    pub fn get_or_record(&mut self, id: i64, now: i64) -> i64 {
        if let Some(at) = self.map.get(&id) {
            return *at;
        }
        self.record(id, now);
        now
    }

    /// Record `id` at `at` unless it already has a time.
    pub fn record(&mut self, id: i64, at: i64) {
        if self.map.contains_key(&id) {
            return;
        }
        self.map.insert(id, at);
        self.records += 1;
        if let Some(p) = &self.path {
            let mut rec = [0u8; REC];
            rec[..8].copy_from_slice(&id.to_le_bytes());
            rec[8..].copy_from_slice(&at.to_le_bytes());
            let w = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(p)
                .and_then(|mut f| f.write_all(&rec));
            if let Err(e) = w {
                eprintln!("111 fabric: display time not saved: {e}");
            }
        }
        if self.map.len() > CAP || self.records > 2 * CAP {
            self.trim();
            self.compact();
        }
    }

    /// Entries held (tests).
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Nothing recorded.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn trim(&mut self) {
        if self.map.len() <= CAP {
            return;
        }
        let mut ids: Vec<i64> = self.map.keys().copied().collect();
        ids.sort_unstable();
        let drop = ids.len() - CAP * 3 / 4;
        for id in &ids[..drop] {
            self.map.remove(id);
        }
    }

    fn compact(&mut self) {
        let Some(p) = &self.path else { return };
        let mut ids: Vec<(&i64, &i64)> = self.map.iter().collect();
        ids.sort_unstable();
        let mut out = Vec::with_capacity(ids.len() * REC);
        for (id, at) in ids {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&at.to_le_bytes());
        }
        match crate::util::fs::atomic_write(p, &out, 0o600) {
            Ok(()) => self.records = self.map.len(),
            Err(e) => eprintln!("111 fabric: display times not compacted: {e:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_survive_a_reload_and_first_time_wins() {
        let t = tempfile::tempdir().unwrap();
        let mut a = DisplayTimes::load(t.path());
        assert!(a.is_empty());
        assert_eq!(a.get_or_record(7, 1_700_000_000), 1_700_000_000);
        assert_eq!(a.get_or_record(7, 1_800_000_000), 1_700_000_000);
        a.record(8, 1_700_000_100);
        let mut b = DisplayTimes::load(t.path());
        assert_eq!(b.len(), 2);
        assert_eq!(b.get_or_record(8, 0), 1_700_000_100);
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn the_foreign_instance_is_a_separate_file_with_the_same_format() {
        let t = tempfile::tempdir().unwrap();
        let mut ours = DisplayTimes::load(t.path());
        let mut foreign = DisplayTimes::open(t.path(), TimesFile::Foreign);
        // The same row id in both spaces never collides.
        assert_eq!(ours.get_or_record(7, 1_700_000_000), 1_700_000_000);
        assert_eq!(foreign.get_or_record(7, 1_800_000_000), 1_800_000_000);
        foreign.record(9, 1_800_000_009);
        let mut ours = DisplayTimes::open(t.path(), TimesFile::Ours);
        let mut foreign = DisplayTimes::open(t.path(), TimesFile::Foreign);
        assert_eq!((ours.len(), foreign.len()), (1, 2));
        assert_eq!(ours.get_or_record(7, 0), 1_700_000_000);
        assert_eq!(foreign.get_or_record(7, 0), 1_800_000_000);
        // Same record format: 16-byte little-endian `id ‖ secs` records.
        let bytes = std::fs::read(t.path().join(FOREIGN_TIMES_FILE)).unwrap();
        assert_eq!(bytes.len(), 2 * REC);
        assert_eq!(&bytes[..8], &7i64.to_le_bytes());
        assert_eq!(&bytes[8..16], &1_800_000_000i64.to_le_bytes());
        // A symlinked file is ignored (starts empty), as for the 111 file.
        let other = t.path().join("elsewhere");
        std::fs::write(&other, &bytes).unwrap();
        let d = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(&other, d.path().join(FOREIGN_TIMES_FILE)).unwrap();
        assert!(DisplayTimes::open(d.path(), TimesFile::Foreign).is_empty());
    }

    #[test]
    fn bounded() {
        let mut a = DisplayTimes::memory();
        for id in 0..(CAP as i64 + 10) {
            a.record(id, id);
        }
        assert!(a.len() <= CAP);
        assert_eq!(
            a.get_or_record(CAP as i64 + 9, -1),
            CAP as i64 + 9,
            "newest kept"
        );
    }
}
