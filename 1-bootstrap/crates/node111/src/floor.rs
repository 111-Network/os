//! Monotonic anti-replay floor (`state/floor.json`, root-written 0644). Daemon AND
//! helper refuse anything lower, regardless of what an incoming manifest claims
//! (replayed old-but-valid signatures included).

use crate::calver::Calver;
use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Floor {
    pub schema: u32,
    /// Empty string = unset (never refuses on that dimension).
    pub highest_release: String,
    pub highest_manifest_date: String,
    pub highest_keychain_generation: u32,
}

impl Floor {
    pub fn initial() -> Self {
        Floor {
            schema: 1,
            highest_release: String::new(),
            highest_manifest_date: String::new(),
            highest_keychain_generation: 0,
        }
    }

    /// True iff the candidate is below the floor in any dimension (i.e. must be refused).
    ///
    /// The stored floor is root-written but still treated as untrusted input: a
    /// non-empty stored calver that fails to parse refuses everything on that
    /// dimension (fail closed, never silently permissive).
    pub fn refuses(
        &self,
        release: &Calver,
        manifest_date: &Calver,
        keychain_generation: u32,
    ) -> bool {
        let below_calver = |stored: &str, candidate: &Calver| -> bool {
            if stored.is_empty() {
                return false; // unset dimension
            }
            match stored.parse::<Calver>() {
                Ok(floor_cv) => *candidate < floor_cv,
                Err(_) => true, // corrupt stored value: fail closed
            }
        };
        below_calver(&self.highest_release, release)
            || below_calver(&self.highest_manifest_date, manifest_date)
            || keychain_generation < self.highest_keychain_generation
    }

    /// Missing file → `Ok(None)` (fresh install has no floor yet); corrupt → Err.
    pub fn read(path: &std::path::Path) -> anyhow::Result<Option<Self>> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).context(format!("read floor {}", path.display())),
        };
        let floor: Floor = serde_json::from_slice(&bytes)
            .with_context(|| format!("corrupt floor file {}", path.display()))?;
        anyhow::ensure!(
            floor.schema == 1,
            "unsupported floor schema {} in {}",
            floor.schema,
            path.display()
        );
        Ok(Some(floor))
    }

    /// Root-only writer; atomic (tmp + fsync + rename), mode 0644.
    pub fn write(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let json = serde_json::to_vec_pretty(self).context("serialize floor")?;
        crate::util::fs::atomic_write(path, &json, 0o644)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn cv(s: &str) -> Calver {
        Calver::from_str(s).unwrap()
    }

    #[test]
    fn refuses_matrix() {
        let mut f = Floor::initial();
        // Empty floor refuses nothing.
        assert!(!f.refuses(&cv("2020.1.1"), &cv("2020.1.1"), 0));

        f.highest_release = "2026.10.12".into();
        f.highest_manifest_date = "2026.10.12".into();
        f.highest_keychain_generation = 2;

        // Below in each dimension.
        assert!(f.refuses(&cv("2026.10.11"), &cv("2026.10.12"), 2));
        assert!(f.refuses(&cv("2026.10.12"), &cv("2026.10.11"), 2));
        assert!(f.refuses(&cv("2026.10.12"), &cv("2026.10.12"), 1)); // replayed old generation
                                                                     // Equal is not below (re-issuing the current state is allowed here; the
                                                                     // installer's <= installed-version check lives elsewhere).
        assert!(!f.refuses(&cv("2026.10.12"), &cv("2026.10.12"), 2));
        // Above on every dimension.
        assert!(!f.refuses(&cv("2026.10.13"), &cv("2026.10.13"), 3));
    }

    #[test]
    fn corrupt_stored_calver_fails_closed() {
        let mut f = Floor::initial();
        f.highest_release = "not-a-calver".into();
        assert!(f.refuses(&cv("2999.1.1"), &cv("2999.1.1"), 99));
    }

    #[test]
    fn read_missing_is_none_corrupt_is_err() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("floor.json");
        assert!(Floor::read(&p).unwrap().is_none());
        std::fs::write(&p, b"{not json").unwrap();
        assert!(Floor::read(&p).is_err());
        // schema mismatch
        std::fs::write(&p, br#"{"schema":2,"highest_release":"","highest_manifest_date":"","highest_keychain_generation":0}"#).unwrap();
        assert!(Floor::read(&p).is_err());
    }

    #[test]
    fn write_roundtrip_mode_0644() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state/floor.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let mut f = Floor::initial();
        f.highest_release = "2026.10.12".into();
        f.highest_keychain_generation = 1;
        f.write(&p).unwrap();
        assert_eq!(p.metadata().unwrap().permissions().mode() & 0o777, 0o644);
        assert_eq!(Floor::read(&p).unwrap().unwrap(), f);
    }
}
