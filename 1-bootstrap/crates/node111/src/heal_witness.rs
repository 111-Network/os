//! Layer 9 (§3.3 rules 2 and 4; §16 row 2; §23 L1 item 2; §21 processes; row L1): `111 heal-witness`,
//! the hidden verb the unit's `ExecStopPost=/usr/local/bin/111 heal-witness` runs as `node111` after
//! each daemon exit (the unit's line is row L2's; an additive amendment of locked Layer 1, its text
//! row K5's). Absent from the usage text, as `identity generate` is.
//!
//! **The line (§3.3 rule 4).** One ASCII line appended to `state/heal/inbox`:
//! `v1 boot=<32 hex|-> result=<word ≤16> code=<≤3 digits|-> signal=<≤3 digits|->`, at most 84 B by
//! the types of [`Facts`] (inside the 96 B bound, [`LINE_MAX`]); no free text. The facts are the
//! service manager's (systemd.exec(5), "Environment Variables Set or Propagated by the Service
//! Manager"): `$SERVICE_RESULT` the result word (`success`, `exit-code`, `signal`, `watchdog`, …),
//! `$EXIT_CODE` one of `exited`, `killed`, `dumped`, and `$EXIT_STATUS` the exit status for
//! `exited`, else the signal's name without `SIG` (§3.3 rule 2), mapped to its number through
//! [`SIGNALS`], a table compiled for this target, `-` for a name it lacks (`RTMIN+n` among them: its
//! base differs between C libraries). `boot` is `/proc/sys/kernel/random/boot_id` without its
//! dashes, or `-` when it cannot be read.
//!
//! **The inbox (J2, L8 §3; §4's path table).** The old inbox is read bounded (its tail,
//! [`TAIL_MAX`] bytes), a line outside the 96 B ASCII bound is dropped, the newest 255 lines are
//! kept and the new one appended; the whole goes to a temp file beside it (`.inbox.tmp-<pid>-<n>`,
//! `O_EXCL`, 0600, fsync) renamed over it, so the file keeps the newest [`INBOX_LINES`] lines and a
//! crash loop with the store unopenable cannot grow it. Nor can runs killed between their temp
//! file's create and its rename (a power cut, the stop timeout's kill) grow `state/heal/`: a run
//! that writes first removes the temp files an earlier run left (`sweep_stale_tmp`), so at most
//! one stays.
//!
//! **Exit 0 on every error (§3.3 rule 2).** `state/heal/` missing (`111d` creates it at its first
//! start, never this verb), not a directory, owned by another user than the one running the verb (the
//! unit runs it as `node111`, the owner, so no inbox is ever left unreadable to `111d`), an inbox that
//! cannot be read, a write that fails before its rename: one line on stderr (the unit's journal),
//! nothing written, status 0, so `ExecStopPost=` never fails the unit. A step that fails after the
//! rename landed (the directory's fsync) leaves the line in the inbox, and the stderr line says so
//! ([`Outcome::Unsynced`]). With `$SERVICE_RESULT` unset the verb was not run by the service manager
//! after an exit: nothing to witness, nothing written. Extra arguments are ignored.
//!
//! **Never adds (§1 rule 1, §21 processes):** a resident process (one transient process per daemon
//! exit: no thread, no runtime, no timer), a network call, a write outside `state/heal/inbox` and its
//! temp files (the one a run makes; those an earlier, killed run left, removed).

use std::ffi::{OsStr, OsString};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

/// The inbox line's bound (§3.3 rule 4, J2): ASCII, at most 96 bytes, its newline not counted.
pub const LINE_MAX: usize = 96;

/// The lines the inbox keeps, the newest (§3.3 rule 4, J2).
pub const INBOX_LINES: usize = 256;

/// The longest result word (§3.3 rule 4).
pub const RESULT_MAX: usize = 16;

/// The inbox's file name under `state/heal/` (§4's path table).
pub const INBOX: &str = "inbox";

/// The kernel's boot id (§3.3 rules 1 and 4).
pub const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";

/// The most of an old inbox this verb reads: its tail, 257 lines of 96 B and their newlines, so a
/// file grown by anything else costs a bounded read (sizes before parse) and still yields the
/// newest 256 whole lines of a full inbox.
pub const TAIL_MAX: u64 = ((INBOX_LINES + 1) * (LINE_MAX + 1)) as u64;

/// The inbox's temp files beside it: `util::fs::atomic_write` names them `.inbox.tmp-<pid>-<n>`.
const TMP_PREFIX: &str = ".inbox.tmp-";

/// The most entries of `state/heal/` one sweep looks at (sizes before parse: a directory filled by
/// anything else costs a bounded read; §4's path table puts a handful there).
const SWEEP_MAX: usize = 64;

/// The longest `$EXIT_CODE` (`exited`, `killed`, `dumped`) and `$EXIT_STATUS` (`SIGVTALRM`) read;
/// a longer value is out of range before it is looked at.
const EXIT_CODE_MAX: usize = 6;
const EXIT_STATUS_MAX: usize = 9;

/// The service manager's signal names (systemd's `signal_to_string`: the name without `SIG`) and
/// their numbers for this target, from the C library's own constants, so the table holds on every
/// architecture the binary is built for (§3.3 rule 2). A name absent here maps to `-`.
pub const SIGNALS: [(&str, libc::c_int); 31] = [
    ("HUP", libc::SIGHUP),
    ("INT", libc::SIGINT),
    ("QUIT", libc::SIGQUIT),
    ("ILL", libc::SIGILL),
    ("TRAP", libc::SIGTRAP),
    ("ABRT", libc::SIGABRT),
    ("BUS", libc::SIGBUS),
    ("FPE", libc::SIGFPE),
    ("KILL", libc::SIGKILL),
    ("USR1", libc::SIGUSR1),
    ("SEGV", libc::SIGSEGV),
    ("USR2", libc::SIGUSR2),
    ("PIPE", libc::SIGPIPE),
    ("ALRM", libc::SIGALRM),
    ("TERM", libc::SIGTERM),
    ("STKFLT", libc::SIGSTKFLT),
    ("CHLD", libc::SIGCHLD),
    ("CONT", libc::SIGCONT),
    ("STOP", libc::SIGSTOP),
    ("TSTP", libc::SIGTSTP),
    ("TTIN", libc::SIGTTIN),
    ("TTOU", libc::SIGTTOU),
    ("URG", libc::SIGURG),
    ("XCPU", libc::SIGXCPU),
    ("XFSZ", libc::SIGXFSZ),
    ("VTALRM", libc::SIGVTALRM),
    ("PROF", libc::SIGPROF),
    ("WINCH", libc::SIGWINCH),
    ("IO", libc::SIGIO),
    ("PWR", libc::SIGPWR),
    ("SYS", libc::SIGSYS),
];

/// A signal name of `$EXIT_STATUS` (with or without `SIG`) to its number through [`SIGNALS`];
/// `None` for a name the table lacks (§3.3 rule 2).
pub fn signal_number(name: &[u8]) -> Option<u8> {
    if name.len() > EXIT_STATUS_MAX {
        return None;
    }
    let name = name.strip_prefix(b"SIG").unwrap_or(name);
    SIGNALS
        .iter()
        .find(|(n, _)| n.as_bytes() == name)
        .and_then(|&(_, number)| u8::try_from(number).ok())
}

/// A boot id without its dashes: 32 lowercase hex digits (§3.3 rules 1 and 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootId(String);

impl BootId {
    /// From the bytes of [`BOOT_ID_PATH`]: a UUID (`8-4-4-4-12` hex digits, one trailing newline
    /// allowed) or its 32 digits alone; `None` for anything else.
    pub fn parse(raw: &[u8]) -> Option<BootId> {
        let raw = raw.strip_suffix(b"\n").unwrap_or(raw);
        let dashed = match raw.len() {
            36 => true,
            32 => false,
            _ => return None,
        };
        let mut hex = String::with_capacity(32);
        for (i, &b) in raw.iter().enumerate() {
            if dashed && matches!(i, 8 | 13 | 18 | 23) {
                if b != b'-' {
                    return None;
                }
            } else if b.is_ascii_hexdigit() {
                hex.push(char::from(b.to_ascii_lowercase()));
            } else {
                return None;
            }
        }
        (hex.len() == 32).then_some(BootId(hex))
    }

    /// The 32 digits.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `$SERVICE_RESULT` as the line's word: 1 to 16 of `a-z`, `0-9` and `-` (systemd's result words,
/// `start-limit-hit` the longest at 15); anything else is no word and is written `-` (§3.3 rule 4: no
/// free text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultWord(String);

impl ResultWord {
    /// The word, or `None` outside 1 to [`RESULT_MAX`] bytes of `[a-z0-9-]`.
    pub fn parse(raw: &[u8]) -> Option<ResultWord> {
        if raw.is_empty() || raw.len() > RESULT_MAX {
            return None;
        }
        raw.iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            .then(|| ResultWord(raw.iter().map(|&b| char::from(b)).collect()))
    }

    /// The word.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The four facts of one inbox line, each bounded by its type, so [`Facts::line`] is at most 84 B
/// (§3.3 rule 4's arithmetic: `v1 ` 3 + `boot=` 37 + ` result=` 24 + ` code=` 9 + ` signal=` 11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// The boot id, `-` when it cannot be read.
    pub boot: Option<BootId>,
    /// `$SERVICE_RESULT`, `-` when it is no word.
    pub result: Option<ResultWord>,
    /// The exit status when the daemon exited (0-255).
    pub code: Option<u8>,
    /// The signal's number when a signal ended it.
    pub signal: Option<u8>,
}

impl Facts {
    /// The facts of the service manager's three variables and the boot id file's bytes (§3.3 rule
    /// 2). An exit status of 1 to 3 digits is the exit code (0-255), or the signal's number (1-64)
    /// when `$EXIT_CODE` says `killed` or `dumped` (systemd writes a number for a signal it has no
    /// name for); any other status is a signal name, mapped through [`SIGNALS`]. Each value is
    /// checked for size before it is read; unset or out of range is `-`.
    pub fn from_env(
        result: &OsStr,
        exit_code: Option<&OsStr>,
        exit_status: Option<&OsStr>,
        boot_id: Option<&[u8]>,
    ) -> Facts {
        let exit_code = exit_code
            .map(OsStr::as_bytes)
            .filter(|c| c.len() <= EXIT_CODE_MAX);
        let by_signal = matches!(exit_code, Some(b"killed" | b"dumped"));
        let (code, signal) = match exit_status.map(OsStr::as_bytes) {
            None => (None, None),
            Some(status) => match decimal(status) {
                Some(n) if by_signal => {
                    (None, u8::try_from(n).ok().filter(|s| (1..=64).contains(s)))
                }
                Some(n) => (u8::try_from(n).ok(), None),
                None => (None, signal_number(status)),
            },
        };
        Facts {
            boot: boot_id.and_then(BootId::parse),
            result: ResultWord::parse(result.as_bytes()),
            code,
            signal,
        }
    }

    /// The line, without its newline (§3.3 rule 4).
    pub fn line(&self) -> String {
        fn or_dash(v: Option<String>) -> String {
            v.unwrap_or_else(|| "-".to_string())
        }
        format!(
            "v1 boot={} result={} code={} signal={}",
            or_dash(self.boot.as_ref().map(|b| b.as_str().to_string())),
            or_dash(self.result.as_ref().map(|r| r.as_str().to_string())),
            or_dash(self.code.map(|c| c.to_string())),
            or_dash(self.signal.map(|s| s.to_string())),
        )
    }
}

/// 1 to 3 ASCII digits as a number; `None` otherwise (the size is checked first).
fn decimal(raw: &[u8]) -> Option<u16> {
    if raw.is_empty() || raw.len() > 3 || !raw.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(
        raw.iter()
            .fold(0u16, |n, &d| n * 10 + u16::from(d.wrapping_sub(b'0'))),
    )
}

/// Why nothing was written; every one of them still exits 0 (§3.3 rule 2).
#[derive(Debug)]
pub enum WitnessError {
    /// `state/heal/` is missing (`111d` creates it at its first start).
    NoDir,
    /// `state/heal/` is not a directory (a symlink included).
    NotADir,
    /// `state/heal/` belongs to another user than the one running the verb.
    NotOwner {
        /// The directory's owner (the node user).
        owner: u32,
        /// The effective uid of this run.
        euid: u32,
    },
    /// The directory or the old inbox could not be read (other than missing).
    Read(std::io::Error),
    /// The temp file or the rename failed: the inbox is unchanged (a failure after the rename
    /// landed is [`Outcome::Unsynced`]).
    Write(anyhow::Error),
}

impl std::fmt::Display for WitnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDir => write!(
                f,
                "state/heal/ is missing (111d creates it at its first start): nothing written"
            ),
            Self::NotADir => write!(f, "state/heal/ is not a directory: nothing written"),
            Self::NotOwner { owner, euid } => write!(
                f,
                "state/heal/ belongs to uid {owner}, not to uid {euid} running the witness \
                 (the unit runs it as the node user): nothing written"
            ),
            Self::Read(e) => write!(
                f,
                "state/heal/ or its inbox unreadable ({e}): nothing written"
            ),
            Self::Write(e) => write!(f, "the inbox could not be written ({e:#}): unchanged"),
        }
    }
}

impl std::error::Error for WitnessError {}

/// What a run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The line was appended; the inbox holds `lines` lines.
    Appended {
        /// The inbox's lines after the write (at most [`INBOX_LINES`]).
        lines: usize,
    },
    /// The line was appended (the rename landed), but a step after it failed, the directory's
    /// fsync, so a power cut may still lose it; status 0 all the same (§3.3 rule 2).
    Unsynced {
        /// The inbox's lines after the write (at most [`INBOX_LINES`]).
        lines: usize,
        /// What failed after the rename.
        error: String,
    },
    /// `$SERVICE_RESULT` is unset: not run by the service manager after an exit, nothing written.
    NoServiceResult,
}

/// The verb (§3.3 rule 2): witness this exit and return 0, whatever happened; an error is one line
/// on stderr (never a panic: a closed stderr is ignored).
pub fn run() -> i32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let euid = unsafe { libc::geteuid() };
    let env = |k: &str| std::env::var_os(k);
    match witness(&crate::util::paths::root(), env, read_boot_id, euid) {
        Ok(Outcome::Appended { .. }) => {}
        Ok(Outcome::Unsynced { error, .. }) => note(&format_args!(
            "the line is in state/heal/inbox, but a step after its rename failed ({error}): \
             a power cut may still lose it"
        )),
        Ok(Outcome::NoServiceResult) => {
            note(&"$SERVICE_RESULT is unset (the unit's ExecStopPost= sets it): nothing written")
        }
        Err(e) => note(&e),
    }
    0
}

/// One run under `root` (the node root, `state/heal/` below it): the facts from `env` and the boot
/// id from `boot_id` (read only when `$SERVICE_RESULT` is set), appended as the user `euid`.
pub fn witness(
    root: &Path,
    env: impl Fn(&str) -> Option<OsString>,
    boot_id: impl FnOnce() -> Option<Vec<u8>>,
    euid: u32,
) -> Result<Outcome, WitnessError> {
    let Some(result) = env("SERVICE_RESULT") else {
        return Ok(Outcome::NoServiceResult);
    };
    let facts = Facts::from_env(
        &result,
        env("EXIT_CODE").as_deref(),
        env("EXIT_STATUS").as_deref(),
        boot_id().as_deref(),
    );
    append(&root.join("state").join("heal"), &facts, euid)
}

/// Appends the line of `facts` to `dir/inbox` (J2): the bounded tail of the old file, its lines
/// inside the bound, the newest 255 of them, then the new one, written through a temp file and a
/// rename (0600) once the temp files an earlier run left are gone (`sweep_stale_tmp`). `dir` must
/// be a directory (not a symlink) owned by `euid`; it is never created. A step that fails after the
/// rename landed is [`Outcome::Unsynced`], the line in; any other failure writes nothing.
pub fn append(dir: &Path, facts: &Facts, euid: u32) -> Result<Outcome, WitnessError> {
    let meta = match std::fs::symlink_metadata(dir) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(WitnessError::NoDir),
        Err(e) => return Err(WitnessError::Read(e)),
    };
    if !meta.is_dir() {
        return Err(WitnessError::NotADir);
    }
    if meta.uid() != euid {
        return Err(WitnessError::NotOwner {
            owner: meta.uid(),
            euid,
        });
    }
    let inbox = dir.join(INBOX);
    let old = read_tail(&inbox).map_err(WitnessError::Read)?;
    let mut kept = inbox_lines(&old);
    let drop = kept.len().saturating_sub(INBOX_LINES - 1);
    kept.drain(..drop);
    let line = facts.line();
    let mut out = Vec::with_capacity(old.len() + line.len() + 1);
    for l in &kept {
        out.extend_from_slice(l);
        out.push(b'\n');
    }
    out.extend_from_slice(line.as_bytes());
    out.push(b'\n');
    let lines = kept.len() + 1;
    let before = identity(&inbox);
    sweep_stale_tmp(dir);
    match crate::util::fs::atomic_write(&inbox, &out, 0o600) {
        Ok(()) => Ok(Outcome::Appended { lines }),
        // The rename landed and a later step failed (`atomic_write`'s last, the directory's fsync):
        // the inbox is the temp file now, an inode made while the old one still stood.
        Err(e) if identity(&inbox).is_some_and(|now| Some(now) != before) => {
            Ok(Outcome::Unsynced {
                lines,
                error: format!("{e:#}"),
            })
        }
        Err(e) => Err(WitnessError::Write(e)),
    }
}

/// The file at `path` itself (a symlink not followed): its device and inode, `None` when absent.
fn identity(path: &Path) -> Option<(u64, u64)> {
    std::fs::symlink_metadata(path)
        .ok()
        .map(|m| (m.dev(), m.ino()))
}

/// Removes the temp files an earlier run left in `dir` (killed between its temp file's create and
/// its rename: a power cut, the stop timeout's kill), so with the one this run makes at most one
/// stays (§3.3 rule 4's bound, carried to the inbox's temp files). Only regular files named
/// [`TMP_PREFIX`]`…`,
/// among the first [`SWEEP_MAX`] entries; a symlink, a directory or another name stays. Runs are
/// serial (the unit's `ExecStopPost=`, after the daemon and its control group are gone), so no live
/// writer's temp file is removed. Best effort: an error is ignored.
fn sweep_stale_tmp(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.take(SWEEP_MAX).flatten() {
        if entry
            .file_name()
            .as_bytes()
            .starts_with(TMP_PREFIX.as_bytes())
            && entry.file_type().is_ok_and(|t| t.is_file())
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The old inbox's last [`TAIL_MAX`] bytes, from the start of a whole line; empty when there is no
/// inbox, or something not a regular file stands in its place (a symlink is never followed, a FIFO
/// never waited on: the rename replaces either).
fn read_tail(path: &Path) -> std::io::Result<Vec<u8>> {
    let opened = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path);
    let mut file = match opened {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Ok(Vec::new());
    }
    let start = meta.len().saturating_sub(TAIL_MAX);
    if start > 0 {
        file.seek(SeekFrom::Start(start))?;
    }
    let mut buf = Vec::new();
    file.take(TAIL_MAX).read_to_end(&mut buf)?;
    if start > 0 {
        // The first piece is the tail of an older line: never kept.
        match buf.iter().position(|&b| b == b'\n') {
            Some(i) => {
                buf.drain(..=i);
            }
            None => buf.clear(),
        }
    }
    Ok(buf)
}

/// The lines of an old inbox inside the bound (§3.3 rule 4: 1 to 96 B of printable ASCII), oldest
/// first; anything else is no inbox line and is dropped.
fn inbox_lines(buf: &[u8]) -> Vec<&[u8]> {
    buf.split(|&b| b == b'\n')
        .filter(|l| {
            !l.is_empty() && l.len() <= LINE_MAX && l.iter().all(|b| (0x20..=0x7e).contains(b))
        })
        .collect()
}

/// [`BOOT_ID_PATH`]'s first 64 bytes, or `None` when it cannot be read (§3.3 rule 1: then `-`).
fn read_boot_id() -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    std::fs::File::open(BOOT_ID_PATH)
        .and_then(|f| f.take(64).read_to_end(&mut buf))
        .ok()?;
    Some(buf)
}

/// One line on stderr; a write error (a closed stderr) is ignored, never a panic.
fn note(msg: &dyn std::fmt::Display) {
    let _ = writeln!(std::io::stderr().lock(), "111 heal-witness: {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const BOOT: &[u8] = b"f8737252-14f3-4204-BCA0-aa8a822e4e08\n";

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |k| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| OsString::from(v))
        }
    }

    fn heal_dir() -> (tempfile::TempDir, std::path::PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("state/heal");
        std::fs::create_dir_all(&dir).unwrap();
        (root, dir)
    }

    fn euid() -> u32 {
        unsafe { libc::geteuid() }
    }

    fn appended(lines: usize) -> Outcome {
        Outcome::Appended { lines }
    }

    /// The entries of `dir`, sorted.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn facts(result: &str, code: Option<&str>, status: Option<&str>) -> Facts {
        Facts::from_env(
            OsStr::new(result),
            code.map(OsStr::new),
            status.map(OsStr::new),
            Some(BOOT),
        )
    }

    /// §3.3 rule 2: the names systemd writes map to this target's numbers; anything else is `-`.
    #[test]
    fn signal_names_map_through_the_compiled_table() {
        assert_eq!(signal_number(b"KILL"), Some(9));
        assert_eq!(signal_number(b"TERM"), Some(15));
        assert_eq!(signal_number(b"ABRT"), Some(6));
        assert_eq!(signal_number(b"SEGV"), Some(11));
        assert_eq!(signal_number(b"HUP"), Some(1));
        assert_eq!(signal_number(b"SIGKILL"), Some(9));
        assert_eq!(
            signal_number(b"USR1"),
            u8::try_from(libc::SIGUSR1).ok(),
            "the C library's number for this target"
        );
        for unknown in [
            &b"RTMIN+3"[..],
            b"BOGUS",
            b"kill",
            b"",
            b"SIG",
            b"SIGSIGKILL",
            b"KILL ",
            b"9",
        ] {
            assert_eq!(signal_number(unknown), None, "{unknown:?}");
        }
        let mut numbers: Vec<_> = SIGNALS.iter().map(|&(_, n)| n).collect();
        numbers.sort_unstable();
        numbers.dedup();
        assert_eq!(numbers.len(), SIGNALS.len(), "one number per name");
        assert!(numbers.iter().all(|n| (1..=64).contains(n)));
    }

    /// §3.3 rules 1 and 4: the boot id without its dashes, lowercased; anything else `-`.
    #[test]
    fn the_boot_id_loses_its_dashes() {
        let b = BootId::parse(BOOT).unwrap();
        assert_eq!(b.as_str(), "f873725214f34204bca0aa8a822e4e08");
        assert_eq!(BootId::parse(b"f873725214f34204bca0aa8a822e4e08"), Some(b));
        for bad in [
            &b""[..],
            b"-",
            b"f8737252-14f3-4204-bca0-aa8a822e4e0",
            b"f8737252-14f3-4204-bca0-aa8a822e4e08\n\n",
            b"f873725214f34204bca0aa8a822e4e0g",
            b"f8737252x14f3-4204-bca0-aa8a822e4e08",
            b"f8737252-14f3-4204-bca0-aa8a822e4e08-0000",
            b"-8737252-14f3-4204-bca0-aa8a822e4e08",
        ] {
            assert_eq!(BootId::parse(bad), None, "{bad:?}");
        }
    }

    /// §3.3 rule 4: the result is a word of at most 16 bytes or `-`; no free text.
    #[test]
    fn the_result_is_a_word_or_a_dash() {
        for ok in [
            "success",
            "exit-code",
            "start-limit-hit",
            "abcdefghijklmnop",
            "oom-kill",
        ] {
            assert_eq!(ResultWord::parse(ok.as_bytes()).unwrap().as_str(), ok);
        }
        for bad in [
            "",
            "abcdefghijklmnopq",
            "Success",
            "exit code",
            "success\nv1",
            "x=y",
            "caf\u{e9}",
        ] {
            assert_eq!(ResultWord::parse(bad.as_bytes()), None, "{bad:?}");
        }
    }

    /// §3.3 rule 2: `$EXIT_STATUS` is the code, a signal name, or a signal number under `killed`.
    #[test]
    fn exit_facts_read_code_and_signal() {
        let f = facts("signal", None, Some("KILL"));
        assert_eq!((f.code, f.signal), (None, Some(9)));
        let f = facts("signal", Some("killed"), Some("KILL"));
        assert_eq!((f.code, f.signal), (None, Some(9)));
        let f = facts("watchdog", Some("dumped"), Some("ABRT"));
        assert_eq!((f.code, f.signal), (None, Some(6)));
        let f = facts("exit-code", Some("exited"), Some("75"));
        assert_eq!((f.code, f.signal), (Some(75), None));
        let f = facts("success", Some("exited"), Some("0"));
        assert_eq!((f.code, f.signal), (Some(0), None));
        let f = facts("exit-code", None, Some("255"));
        assert_eq!((f.code, f.signal), (Some(255), None));
        // systemd's number for a signal it cannot name
        let f = facts("signal", Some("killed"), Some("33"));
        assert_eq!((f.code, f.signal), (None, Some(33)));
        for (code, status) in [
            (Some("killed"), Some("RTMIN+3")),
            (Some("killed"), Some("0")),
            (Some("killed"), Some("65")),
            (Some("exited"), Some("256")),
            (Some("exited"), Some("1000")),
            (Some("exited"), Some("-1")),
            (Some("exited"), Some(" 75")),
            (Some("exited"), Some("")),
            (Some("exited"), None),
            (None, None),
            (Some("killed"), Some("SIGVTALRMX")),
        ] {
            let f = facts("signal", code, status);
            assert_eq!((f.code, f.signal), (None, None), "{code:?} {status:?}");
        }
        // an EXIT_CODE out of range is not read: the digits are then an exit code
        let f = facts("signal", Some("killed-by-xyz"), Some("9"));
        assert_eq!((f.code, f.signal), (Some(9), None));
        // non-UTF-8 values are bytes like any other: no word, no number
        let f = Facts::from_env(
            OsStr::from_bytes(b"succ\xffess"),
            Some(OsStr::from_bytes(b"\xff")),
            Some(OsStr::from_bytes(b"K\xffLL")),
            None,
        );
        assert_eq!(f.line(), "v1 boot=- result=- code=- signal=-");
    }

    /// §3.3 rule 4: 84 B at most with every field at its maximum, inside the 96 B bound.
    #[test]
    fn the_line_is_84_bytes_at_most() {
        let max = Facts {
            boot: BootId::parse(BOOT),
            result: ResultWord::parse(b"abcdefghijklmnop"),
            code: Some(255),
            signal: Some(255),
        };
        let line = max.line();
        assert_eq!(
            line,
            "v1 boot=f873725214f34204bca0aa8a822e4e08 result=abcdefghijklmnop code=255 signal=255"
        );
        assert_eq!(line.len(), 84);
        assert!(line.len() <= LINE_MAX);
        let min = Facts {
            boot: None,
            result: None,
            code: None,
            signal: None,
        };
        assert_eq!(min.line(), "v1 boot=- result=- code=- signal=-");
    }

    /// J2: the inbox keeps the newest 256 lines, 0600, through a temp file that does not stay.
    #[test]
    fn append_keeps_the_newest_256_lines() {
        let (_root, dir) = heal_dir();
        for i in 0..300u32 {
            let f = Facts {
                boot: None,
                result: ResultWord::parse(format!("r{i}").as_bytes()),
                code: None,
                signal: None,
            };
            let n = append(&dir, &f, euid()).unwrap();
            assert_eq!(n, appended((i as usize + 1).min(INBOX_LINES)));
        }
        let text = std::fs::read_to_string(dir.join(INBOX)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), INBOX_LINES);
        assert_eq!(lines[0], "v1 boot=- result=r44 code=- signal=-");
        assert_eq!(lines[255], "v1 boot=- result=r299 code=- signal=-");
        let mode = std::fs::metadata(dir.join(INBOX))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [INBOX], "no temp file stays");
    }

    /// Sizes before parse: an old inbox grown by anything else is read by its tail only, and what
    /// is no inbox line is dropped.
    #[test]
    fn an_oversized_inbox_is_read_bounded() {
        let (_root, dir) = heal_dir();
        // lines before the tail are never read, inbox-shaped or not
        let mut junk = b"v1 beyond the tail 1\nv1 beyond the tail 2\n".to_vec();
        junk.extend_from_slice(&[b'x'; 1 << 20]);
        junk.extend_from_slice(b"\nv1 keep me\n\xff\xfe bad\nA line far too long: ");
        junk.extend_from_slice(&[b'y'; 200]);
        junk.extend_from_slice(b"\n\tTab\n\nv1 keep me too\n");
        std::fs::write(dir.join(INBOX), &junk).unwrap();
        let f = facts("success", Some("exited"), Some("0"));
        assert_eq!(append(&dir, &f, euid()).unwrap(), appended(3));
        let text = std::fs::read_to_string(dir.join(INBOX)).unwrap();
        assert_eq!(
            text,
            "v1 keep me\nv1 keep me too\nv1 boot=f873725214f34204bca0aa8a822e4e08 result=success code=0 signal=-\n"
        );
        // a whole file of junk with no newline in its tail keeps nothing of it
        std::fs::write(dir.join(INBOX), vec![b'z'; 1 << 16]).unwrap();
        assert_eq!(append(&dir, &f, euid()).unwrap(), appended(1));
    }

    /// §3.3 rule 2: no `state/heal/`, another type in its place, another owner: nothing written.
    #[test]
    fn append_writes_nothing_without_its_directory() {
        let root = tempfile::tempdir().unwrap();
        let f = facts("signal", None, Some("KILL"));
        let dir = root.path().join("state/heal");
        assert!(matches!(append(&dir, &f, euid()), Err(WitnessError::NoDir)));
        assert!(!root.path().join("state").exists(), "nothing created");

        std::fs::create_dir_all(root.path().join("state")).unwrap();
        std::fs::write(&dir, b"").unwrap();
        assert!(matches!(
            append(&dir, &f, euid()),
            Err(WitnessError::NotADir)
        ));
        std::fs::remove_file(&dir).unwrap();

        let elsewhere = root.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &dir).unwrap();
        assert!(matches!(
            append(&dir, &f, euid()),
            Err(WitnessError::NotADir)
        ));
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
        std::fs::remove_file(&dir).unwrap();

        std::fs::create_dir(&dir).unwrap();
        let other = euid().wrapping_add(1);
        assert!(matches!(
            append(&dir, &f, other),
            Err(WitnessError::NotOwner { .. })
        ));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        assert_eq!(append(&dir, &f, euid()).unwrap(), appended(1));
    }

    /// A symlink in the inbox's place is replaced, its target untouched; a FIFO is not waited on.
    #[test]
    fn a_planted_inbox_is_replaced_never_followed() {
        let (root, dir) = heal_dir();
        let target = root.path().join("target");
        std::fs::write(&target, b"v1 not an inbox of ours\n").unwrap();
        std::os::unix::fs::symlink(&target, dir.join(INBOX)).unwrap();
        let f = facts("signal", Some("killed"), Some("KILL"));
        assert_eq!(append(&dir, &f, euid()).unwrap(), appended(1));
        assert!(std::fs::symlink_metadata(dir.join(INBOX))
            .unwrap()
            .file_type()
            .is_file());
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"v1 not an inbox of ours\n"
        );

        std::fs::remove_file(dir.join(INBOX)).unwrap();
        let fifo = std::ffi::CString::new(dir.join(INBOX).as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert_eq!(append(&dir, &f, euid()).unwrap(), appended(1));
    }

    /// Review of row L1 (§3.3 rule 4): the temp files an earlier, killed run left go before the
    /// write, regular files named `.inbox.tmp-…` only (a directory, a symlink, another file's temp
    /// file stay), the first [`SWEEP_MAX`] entries a run; a run that writes nothing removes nothing.
    #[test]
    fn stale_temp_files_go_before_the_write() {
        assert_eq!(TMP_PREFIX, format!(".{INBOX}.tmp-"));
        let (root, dir) = heal_dir();
        std::fs::write(dir.join(".inbox.tmp-4242-0"), b"v1 boot=- res").unwrap();
        std::fs::write(dir.join(".inbox.tmp-4243-7"), b"").unwrap();
        std::fs::create_dir(dir.join(".inbox.tmp-4244-0")).unwrap();
        let outside = root.path().join("outside");
        std::fs::write(&outside, b"keep").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join(".inbox.tmp-4245-0")).unwrap();
        std::fs::write(dir.join(".run.tmp-4246-0"), b"").unwrap();
        std::fs::write(dir.join("undo"), b"").unwrap();
        let before = names(&dir);
        assert_eq!(before.len(), 6);
        let f = facts("signal", Some("killed"), Some("KILL"));
        assert!(matches!(
            append(&dir, &f, euid().wrapping_add(1)),
            Err(WitnessError::NotOwner { .. })
        ));
        assert_eq!(names(&dir), before, "nothing written, nothing removed");
        assert_eq!(append(&dir, &f, euid()).unwrap(), appended(1));
        assert_eq!(
            names(&dir),
            [
                ".inbox.tmp-4244-0",
                ".inbox.tmp-4245-0",
                ".run.tmp-4246-0",
                INBOX,
                "undo"
            ]
        );
        assert_eq!(std::fs::read(&outside).unwrap(), b"keep");

        // bounded: a run looks at SWEEP_MAX entries at most, the next goes on
        let (_root, dir) = heal_dir();
        for n in 0..SWEEP_MAX + 10 {
            std::fs::write(dir.join(format!(".inbox.tmp-1-{n}")), b"").unwrap();
        }
        assert_eq!(append(&dir, &f, euid()).unwrap(), appended(1));
        assert_eq!(names(&dir).len(), 10 + 1);
        assert_eq!(append(&dir, &f, euid()).unwrap(), appended(2));
        assert_eq!(names(&dir), [INBOX]);
    }

    /// §3.3 rule 2: unset `$SERVICE_RESULT` writes nothing; the boot id is not even read then.
    #[test]
    fn witness_needs_the_service_result() {
        let (root, dir) = heal_dir();
        let none = env(&[("EXIT_STATUS", "KILL")]);
        let out = witness(root.path(), none, || panic!("not read"), euid()).unwrap();
        assert_eq!(out, Outcome::NoServiceResult);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);

        let vars = [("SERVICE_RESULT", "signal"), ("EXIT_STATUS", "KILL")];
        let out = witness(root.path(), env(&vars), || Some(BOOT.to_vec()), euid()).unwrap();
        assert_eq!(out, Outcome::Appended { lines: 1 });
        assert_eq!(
            std::fs::read_to_string(dir.join(INBOX)).unwrap(),
            "v1 boot=f873725214f34204bca0aa8a822e4e08 result=signal code=- signal=9\n"
        );
        let out = witness(root.path(), env(&vars), || None, euid()).unwrap();
        assert_eq!(out, Outcome::Appended { lines: 2 });
        assert!(std::fs::read_to_string(dir.join(INBOX))
            .unwrap()
            .ends_with("\nv1 boot=- result=signal code=- signal=9\n"));
    }
}
