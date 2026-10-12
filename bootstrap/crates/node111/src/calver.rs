//! Calendar versioning `YYYY.MM.DD[.N]` (spec v2.1, D12). Fields are zero-padded in
//! the canonical string form; ordering equals lexicographic order on that form.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Calver {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    /// Same-day respin (`.2`, `.3`, ...). None == first release of the day.
    pub respin: Option<u32>,
}

impl Calver {
    /// Canonical zero-padded form: `2026.10.12` or `2026.10.12.2`.
    pub fn canonical(&self) -> String {
        match self.respin {
            Some(n) => format!("{:04}.{:02}.{:02}.{}", self.year, self.month, self.day, n),
            None => format!("{:04}.{:02}.{:02}", self.year, self.month, self.day),
        }
    }
}

impl fmt::Display for Calver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}

impl FromStr for Calver {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('.').collect();
        anyhow::ensure!(
            parts.len() == 3 || parts.len() == 4,
            "calver must be YYYY.MM.DD[.N]: {s:?}"
        );
        let parse = |p: &str| -> anyhow::Result<u32> {
            anyhow::ensure!(
                !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()),
                "non-numeric calver field in {s:?}"
            );
            Ok(p.parse()?)
        };
        let (year, month, day) = (parse(parts[0])?, parse(parts[1])?, parse(parts[2])?);
        anyhow::ensure!((1..=12).contains(&month), "month out of range in {s:?}");
        anyhow::ensure!((1..=31).contains(&day), "day out of range in {s:?}");
        let respin = if parts.len() == 4 {
            let n = parse(parts[3])?;
            anyhow::ensure!(n >= 2, "respin starts at .2 in {s:?}");
            Some(n)
        } else {
            None
        };
        Ok(Calver {
            year,
            month,
            day,
            respin,
        })
    }
}

impl PartialOrd for Calver {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Calver {
    fn cmp(&self, other: &Self) -> Ordering {
        // Equivalent to lexicographic order on the canonical zero-padded string.
        (self.year, self.month, self.day, self.respin.unwrap_or(1)).cmp(&(
            other.year,
            other.month,
            other.day,
            other.respin.unwrap_or(1),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_order() {
        let a = Calver::from_str("2026.10.12").unwrap();
        assert_eq!(a.to_string(), "2026.10.12");
        let b = Calver::from_str("2026.10.12.2").unwrap();
        assert!(b > a);
        assert!(Calver::from_str("2026.11.1").unwrap() > b);
        assert!(Calver::from_str("2027.1.1").unwrap() > Calver::from_str("2026.12.31.9").unwrap());
        assert!(Calver::from_str("2026.13.1").is_err());
        assert!(Calver::from_str("2026.10.12.1").is_err());
        assert!(Calver::from_str("v2026.10.12").is_err());
    }
}
