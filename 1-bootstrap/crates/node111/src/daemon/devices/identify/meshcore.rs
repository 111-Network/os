//! Identify step 3: MeshCore (`os/6-transport/README.md` §12; R05 §3.1).
//!
//! The documented MIT exchange of MeshCore's KISS modem firmware
//! (`docs/kiss_modem_protocol.md`, MeshCore, MIT; read 2026-10-19), and
//! nothing else — no byte here comes from Reticulum's RNode driver:
//!
//! - KISS framing: `FEND 0xC0` … `FEND`, escapes `FESC 0xDB` + `TFEND 0xDC` /
//!   `TFESC 0xDD`; the type byte's low nibble is the command, the high nibble
//!   the port (0). Maximum unescaped frame 512 B. 115200 8N1.
//! - Host → modem: `SetHardware 0x06` + sub-command `GetVersion 0x11`, then
//!   `0x06` + `GetIdentity 0x01` (no data).
//! - Modem → host: `0x06` + `Version 0x91` + version (1) + reserved (1), and
//!   `0x06` + `Identity 0x81` + the modem's Ed25519 public key (32).
//!   Responses are `command | 0x80`; `0xF1 Error`, `0xF8 TxDone`,
//!   `0xF9 RxMeta` and data frames (`0x00`) may interleave and are skipped.
//!
//! Outcomes: both answers well-formed ⇒ `Match {family: meshcore, version:
//! "<n>", source: probe}`; no SetHardware answer at all within [`TIMEOUT`]
//! (silence, other firmware, noise) ⇒ `NoMatch`; a `Version` answer without
//! a well-formed `Identity` ⇒ `Error` (a partial answer is never taken as
//! "not MeshCore"); the port cannot be opened, is not a terminal, or a write
//! fails ⇒ `Error`.
//!
//! The port is opened `O_RDWR | O_NOCTTY | O_NONBLOCK | O_CLOEXEC`, set raw
//! 115200 8N1 with `HUPCL` cleared (closing it does not drop DTR a second
//! time), pending input flushed, and closed when the probe ends. Opening a
//! tty raises DTR/RTS, which resets many ESP32 boards: that is why this runs
//! only at the consented step. **Unverified on hardware** (external gate
//! G1); tested against a pty stub modelled on the protocol document.
//! Companion-firmware identify is deferred (§12).

use super::{serial_runnable, Abort, BoxFut, Outcome, Probe, Step, StepKind, UplinkKind};
use crate::daemon::devices::{DeviceFirmware, FirmwareFamily, FirmwareSource};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long the modem has to answer both requests.
pub const TIMEOUT: Duration = Duration::from_secs(3);
pub const FEND: u8 = 0xC0;
pub const FESC: u8 = 0xDB;
pub const TFEND: u8 = 0xDC;
pub const TFESC: u8 = 0xDD;
/// `SetHardware` (the MeshCore extension command).
pub const CMD_SET_HARDWARE: u8 = 0x06;
pub const SUB_GET_IDENTITY: u8 = 0x01;
pub const SUB_GET_VERSION: u8 = 0x11;
pub const RESP_IDENTITY: u8 = 0x81;
pub const RESP_VERSION: u8 = 0x91;
pub const RESP_ERROR: u8 = 0xF1;
/// Largest unescaped frame (the document's limit).
pub const MAX_FRAME: usize = 512;
/// Most bytes read during one probe (bounded work on a chatty port).
pub const MAX_READ: usize = 16 * 1024;

/// One request frame: `FEND 0x06 <sub> FEND` (no data, nothing to escape).
pub fn request(sub: u8) -> [u8; 4] {
    [FEND, CMD_SET_HARDWARE, sub, FEND]
}

/// KISS frame decoder (bounded; a frame over [`MAX_FRAME`] is dropped).
#[derive(Debug, Default)]
pub struct Deframer {
    buf: Vec<u8>,
    in_frame: bool,
    esc: bool,
    overflow: bool,
}

impl Deframer {
    /// Feed bytes; returns the complete frames (type byte first).
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for &b in bytes {
            if b == FEND {
                if self.in_frame && !self.buf.is_empty() && !self.overflow {
                    out.push(std::mem::take(&mut self.buf));
                }
                self.buf.clear();
                self.in_frame = true;
                self.esc = false;
                self.overflow = false;
                continue;
            }
            if !self.in_frame {
                continue;
            }
            let byte = if self.esc {
                self.esc = false;
                match b {
                    TFEND => FEND,
                    TFESC => FESC,
                    // a bad escape: the frame is not valid KISS
                    _ => {
                        self.overflow = true;
                        continue;
                    }
                }
            } else if b == FESC {
                self.esc = true;
                continue;
            } else {
                b
            };
            if self.buf.len() >= MAX_FRAME {
                self.overflow = true;
            } else {
                self.buf.push(byte);
            }
        }
        out
    }
}

/// What the modem answered so far.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Answers {
    pub version: Option<u8>,
    pub identity: Option<[u8; 32]>,
    /// Any SetHardware frame from the port (a KISS modem speaking MeshCore's
    /// extension, even an error).
    pub spoke: bool,
}

impl Answers {
    /// Take one frame (type byte first).
    pub fn take(&mut self, frame: &[u8]) {
        let Some((&ty, rest)) = frame.split_first() else {
            return;
        };
        if ty & 0x0F != CMD_SET_HARDWARE || ty >> 4 != 0 {
            return;
        }
        let Some((&sub, data)) = rest.split_first() else {
            return;
        };
        match sub {
            RESP_VERSION if data.len() >= 2 => {
                self.spoke = true;
                self.version = Some(data[0]);
            }
            RESP_IDENTITY if data.len() == 32 => {
                self.spoke = true;
                let mut k = [0u8; 32];
                k.copy_from_slice(data);
                self.identity = Some(k);
            }
            RESP_ERROR => self.spoke = true,
            _ => {}
        }
    }

    /// The outcome these answers make (module doc).
    pub fn outcome(&self) -> Outcome {
        match (self.version, self.identity) {
            (Some(v), Some(_)) => Outcome::Match(DeviceFirmware {
                family: FirmwareFamily::Meshcore,
                version: Some(v.to_string()),
                source: FirmwareSource::Probe,
            }),
            (Some(_), None) => {
                Outcome::Error("the MeshCore modem answered GetVersion but not GetIdentity".into())
            }
            (None, Some(_)) => {
                Outcome::Error("the MeshCore modem answered GetIdentity but not GetVersion".into())
            }
            (None, None) if self.spoke => {
                Outcome::Error("a KISS modem answered with an error to GetVersion".into())
            }
            (None, None) => Outcome::NoMatch,
        }
    }
}

/// An owned descriptor closed on drop.
struct Fd(libc::c_int);

impl Drop for Fd {
    fn drop(&mut self) {
        // SAFETY: the fd is owned by this guard and closed once.
        unsafe { libc::close(self.0) };
    }
}

fn os_err(what: &str) -> String {
    format!("{what}: {}", std::io::Error::last_os_error())
}

/// Open `path` as the probe's serial port (module doc).
fn open_port(path: &Path) -> Result<Fd, String> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| "the port path holds a NUL".to_string())?;
    // SAFETY: a valid NUL-terminated path; the fd is owned by `Fd`.
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(os_err("cannot open the port"));
    }
    let fd = Fd(fd);
    // SAFETY: zeroed termios is a valid out-parameter for tcgetattr.
    let mut t: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: fd is open; t is a valid termios.
    if unsafe { libc::tcgetattr(fd.0, &mut t) } != 0 {
        return Err(os_err("the port is not a terminal"));
    }
    // SAFETY: t is a valid termios.
    unsafe { libc::cfmakeraw(&mut t) };
    t.c_cflag |= libc::CLOCAL | libc::CREAD;
    t.c_cflag &= !(libc::HUPCL | libc::CSTOPB | libc::PARENB | libc::CRTSCTS);
    // SAFETY: t is a valid termios; B115200 is a valid speed.
    let speed_ok = unsafe { libc::cfsetspeed(&mut t, libc::B115200) } == 0;
    // SAFETY: fd is open; t is valid.
    if !speed_ok || unsafe { libc::tcsetattr(fd.0, libc::TCSANOW, &t) } != 0 {
        return Err(os_err("cannot set the port to 115200 8N1"));
    }
    // SAFETY: fd is open.
    unsafe { libc::tcflush(fd.0, libc::TCIFLUSH) };
    Ok(fd)
}

fn write_all(fd: &Fd, mut bytes: &[u8], deadline: Instant) -> Result<(), String> {
    while !bytes.is_empty() {
        // SAFETY: bytes is a valid buffer of its length.
        let n = unsafe { libc::write(fd.0, bytes.as_ptr() as *const libc::c_void, bytes.len()) };
        if n > 0 {
            bytes = &bytes[n as usize..];
            continue;
        }
        let err = std::io::Error::last_os_error();
        if n < 0 && err.raw_os_error() == Some(libc::EAGAIN) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        return Err(format!("cannot write to the port: {err}"));
    }
    Ok(())
}

/// Read until `done` or the deadline; `Err` only for a hard read error.
fn read_until(
    fd: &Fd,
    de: &mut Deframer,
    answers: &mut Answers,
    total: &mut usize,
    deadline: Instant,
    done: impl Fn(&Answers) -> bool,
) -> Result<(), String> {
    let mut buf = [0u8; 512];
    while !done(answers) && *total < MAX_READ {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let mut pfd = libc::pollfd {
            fd: fd.0,
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = left.as_millis().clamp(1, 1000) as libc::c_int;
        // SAFETY: one valid pollfd.
        let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
        if rc < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(format!("cannot read the port: {e}"));
        }
        if rc == 0 {
            continue;
        }
        if pfd.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err("the port reported an error".into());
        }
        // SAFETY: buf is a valid buffer of its length.
        let n = unsafe { libc::read(fd.0, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n > 0 {
            *total += n as usize;
            for f in de.push(&buf[..n as usize]) {
                answers.take(&f);
            }
        } else if n == 0 || pfd.revents & libc::POLLHUP != 0 {
            // the other end hung up: nothing more will come
            break;
        } else {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::EAGAIN) {
                return Err(format!("cannot read the port: {e}"));
            }
        }
    }
    Ok(())
}

/// The whole blocking probe of `path` within `timeout` (module doc).
pub fn probe_port(path: &Path, timeout: Duration) -> Outcome {
    let deadline = Instant::now() + timeout;
    let fd = match open_port(path) {
        Ok(fd) => fd,
        Err(e) => return Outcome::Error(e),
    };
    let mut de = Deframer::default();
    let mut answers = Answers::default();
    let mut total = 0usize;
    if let Err(e) = write_all(&fd, &request(SUB_GET_VERSION), deadline) {
        return Outcome::Error(e);
    }
    if let Err(e) = read_until(&fd, &mut de, &mut answers, &mut total, deadline, |a| {
        a.version.is_some() || a.spoke
    }) {
        return Outcome::Error(e);
    }
    if answers.version.is_some() {
        if let Err(e) = write_all(&fd, &request(SUB_GET_IDENTITY), deadline) {
            return Outcome::Error(e);
        }
        if let Err(e) = read_until(&fd, &mut de, &mut answers, &mut total, deadline, |a| {
            a.identity.is_some()
        }) {
            return Outcome::Error(e);
        }
    }
    answers.outcome()
}

/// The MeshCore step (module doc).
pub struct MeshcoreStep {
    timeout: Duration,
    phone_like: bool,
}

impl MeshcoreStep {
    pub fn new(timeout: Duration, phone_like: bool) -> Self {
        MeshcoreStep {
            timeout,
            phone_like,
        }
    }
}

impl Step for MeshcoreStep {
    fn kind(&self) -> StepKind {
        StepKind::Meshcore
    }

    fn tool(&self) -> &'static str {
        "the MeshCore serial probe"
    }

    fn available(&self, kind: UplinkKind) -> bool {
        kind == UplinkKind::Serial && serial_runnable(self.phone_like)
    }

    fn run<'a>(&'a self, probe: &'a Probe) -> BoxFut<'a, Result<Outcome, Abort>> {
        Box::pin(async move {
            let path = probe.path.clone();
            let timeout = self.timeout;
            Ok(
                tokio::task::spawn_blocking(move || probe_port(&path, timeout))
                    .await
                    .unwrap_or_else(|e| Outcome::Error(format!("the probe failed: {e}"))),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deframer_unescapes_and_bounds() {
        let mut d = Deframer::default();
        // noise before the first FEND is ignored; escapes decode
        let frames = d.push(&[0x41, FEND, 0x06, 0x91, FESC, TFEND, 0x00, FEND]);
        assert_eq!(frames, vec![vec![0x06, 0x91, FEND, 0x00]]);
        // back-to-back FENDs make no empty frames; split input joins
        assert!(d.push(&[FEND, FEND, 0x06]).is_empty());
        assert_eq!(d.push(&[0x81, FEND]), vec![vec![0x06, 0x81]]);
        // an oversize frame is dropped whole
        let mut big = vec![FEND, 0x00];
        big.extend(std::iter::repeat_n(0x55, MAX_FRAME + 5));
        big.push(FEND);
        assert!(d.push(&big).is_empty());
        // a bad escape spoils its frame only
        assert!(d.push(&[FEND, 0x06, FESC, 0x01, FEND]).is_empty());
        assert_eq!(
            d.push(&[0x06, 0x91, 1, 0, FEND]),
            vec![vec![0x06, 0x91, 1, 0]]
        );
    }

    #[test]
    fn answers_decide_the_outcome() {
        let mut a = Answers::default();
        assert_eq!(a.outcome(), Outcome::NoMatch);
        a.take(&[0x00, 1, 2, 3]); // a data frame
        a.take(&[0x16, 0x91, 7, 0]); // port 1: not ours
        assert_eq!(a.outcome(), Outcome::NoMatch);
        a.take(&[0x06, 0x91, 7, 0]);
        assert!(matches!(a.outcome(), Outcome::Error(_)), "version alone");
        a.take(&[0x06, 0x81, 1, 2]); // short identity
        assert!(matches!(a.outcome(), Outcome::Error(_)));
        let mut id = vec![0x06, 0x81];
        id.extend([9u8; 32]);
        a.take(&id);
        match a.outcome() {
            Outcome::Match(fw) => {
                assert_eq!(fw.family, FirmwareFamily::Meshcore);
                assert_eq!(fw.version.as_deref(), Some("7"));
                assert_eq!(fw.source, FirmwareSource::Probe);
            }
            o => panic!("{o:?}"),
        }
        let mut err = Answers::default();
        err.take(&[0x06, RESP_ERROR, 0x05]);
        assert!(matches!(err.outcome(), Outcome::Error(_)));
        assert_eq!(request(SUB_GET_VERSION), [0xC0, 0x06, 0x11, 0xC0]);
        assert_eq!(request(SUB_GET_IDENTITY), [0xC0, 0x06, 0x01, 0xC0]);
    }

    #[test]
    fn a_missing_or_non_tty_port_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            probe_port(&dir.path().join("ttyNOPE"), Duration::from_millis(50)),
            Outcome::Error(_)
        ));
        let file = dir.path().join("plain");
        std::fs::write(&file, b"").unwrap();
        match probe_port(&file, Duration::from_millis(50)) {
            Outcome::Error(e) => assert!(e.contains("not a terminal"), "{e}"),
            o => panic!("{o:?}"),
        }
    }
}
