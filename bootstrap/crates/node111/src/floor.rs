//! Monotonic anti-replay floor (`state/floor.json`, root-written 0644). Daemon AND
//! helper refuse anything lower, regardless of what an incoming manifest claims
//! (replayed old-but-valid signatures included).

use crate::calver::Calver;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Floor {
    pub schema: u32,
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
    pub fn refuses(
        &self,
        _release: &Calver,
        _manifest_date: &Calver,
        _keychain_generation: u32,
    ) -> bool {
        todo!("T2")
    }

    pub fn read(path: &std::path::Path) -> anyhow::Result<Option<Self>> {
        let _ = path;
        todo!("T2")
    }

    /// Root-only writer; atomic (tmp + fsync + rename), mode 0644.
    pub fn write(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let _ = path;
        todo!("T2")
    }
}
