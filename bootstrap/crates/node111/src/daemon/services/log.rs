//! Child log tails for `GET /api/v1/services/{id}/log?n=` (spec §6.8, §7).
//!
//! Only the last [`WINDOW`] bytes of a file are read (a partial first line is
//! dropped), lines are capped at [`MAX_LINE`] chars, and every control
//! character except `\t` is stripped (ANSI CSI colour sequences are dropped
//! whole, so `ESC[31m` does not leave `[31m` behind). Log lines are untrusted
//! data — the UI renders them as text and the agent prompt quotes them as such.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Default and bounds of `n` (contract: 1..=500, default 100).
pub const DEFAULT_LINES: usize = 100;
pub const MAX_LINES: usize = 500;
/// Bytes read from the end of the file.
pub const WINDOW: u64 = 256 * 1024;
/// Characters kept per line.
pub const MAX_LINE: usize = 2000;

/// Clamp a requested line count into 1..=500 (`None` → default 100).
pub fn clamp_lines(n: Option<i64>) -> usize {
    match n {
        None => DEFAULT_LINES,
        Some(n) => n.clamp(1, MAX_LINES as i64) as usize,
    }
}

/// Strip control characters except `\t`; drop ANSI CSI sequences entirely.
pub fn sanitize(line: &str) -> String {
    let mut out = String::with_capacity(line.len().min(MAX_LINE * 4));
    let mut kept = 0usize;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // parameters/intermediates until a final byte in @..~
                for d in chars.by_ref() {
                    if ('@'..='~').contains(&d) {
                        break;
                    }
                }
            }
            continue;
        }
        if c == '\t' || !c.is_control() {
            out.push(c);
            kept += 1;
            if kept >= MAX_LINE {
                break;
            }
        }
    }
    out
}

/// The last `n` lines of `path` (sanitized). `Ok(None)` when the file does
/// not exist.
pub fn tail(path: &Path, n: usize) -> std::io::Result<Option<Vec<String>>> {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let len = f.metadata()?.len();
    let start = len.saturating_sub(WINDOW);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    f.take(WINDOW).read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.split('\n').collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // partial line at the window edge
    }
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop(); // trailing newline
    }
    let skip = lines.len().saturating_sub(n);
    Ok(Some(lines[skip..].iter().map(|l| sanitize(l)).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_bounds() {
        assert_eq!(clamp_lines(None), 100);
        assert_eq!(clamp_lines(Some(0)), 1);
        assert_eq!(clamp_lines(Some(-5)), 1);
        assert_eq!(clamp_lines(Some(7)), 7);
        assert_eq!(clamp_lines(Some(501)), 500);
        assert_eq!(clamp_lines(Some(i64::MAX)), 500);
    }

    #[test]
    fn sanitize_strips_controls_keeps_tabs() {
        assert_eq!(sanitize("a\tb\rc\u{7}d\u{0}e"), "a\tbcde");
        assert_eq!(sanitize("\u{1b}[31mred\u{1b}[0m ok"), "red ok");
        assert_eq!(sanitize("bare \u{1b} esc"), "bare  esc");
        assert_eq!(sanitize("héllo ✓"), "héllo ✓");
        assert_eq!(sanitize(&"x".repeat(5000)).len(), MAX_LINE);
    }

    #[test]
    fn tail_window_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(tail(&dir.path().join("nope.log"), 10).unwrap(), None);
        let p = dir.path().join("a.log");
        std::fs::write(&p, "one\ntwo\r\nthree\n").unwrap();
        assert_eq!(tail(&p, 2).unwrap().unwrap(), ["two", "three"]);
        assert_eq!(tail(&p, 100).unwrap().unwrap(), ["one", "two", "three"]);
        std::fs::write(&p, "").unwrap();
        assert!(tail(&p, 5).unwrap().unwrap().is_empty());
        // big file: only the window is read, the cut first line is dropped
        let mut big = String::new();
        for i in 0..40_000 {
            big.push_str(&format!("line {i:06}\n"));
        }
        std::fs::write(&p, &big).unwrap();
        let t = tail(&p, 500).unwrap().unwrap();
        assert_eq!(t.len(), 500);
        assert_eq!(t.last().unwrap(), "line 039999");
        assert_eq!(t[0], "line 039500");
    }
}
