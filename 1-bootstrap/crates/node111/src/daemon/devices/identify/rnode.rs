//! Identify step 1: RNode (`os/6-transport/README.md` §12; DEV §3.2;
//! KB `projects/rnode-firmware.md`: "Layer 6 adopts `rnodeconf --info` as
//! the identify probe").
//!
//! Runs `<root>/venvs/rns/bin/rnodeconf --info <port>` (the adopted tool from
//! the rns venv — a separate process; no Reticulum code enters this binary)
//! through a [`Runner`] with stdin closed, `HOME=<root>`, a [`TIMEOUT`] and
//! capped output, then reads its **text** ([`parse_info`]):
//!
//! - `Device info:` with a `Firmware version   : <v>` line, or `Device
//!   connected` with `Current firmware version: <v>` (an RNode whose EEPROM
//!   is not provisioned still runs the firmware) ⇒ `Match {family: rnode,
//!   version, source: probe}`;
//! - `Serial port opened, but RNode did not respond` / `Got invalid response
//!   while detecting device` (the tool says it is not an RNode) ⇒ `NoMatch`;
//! - the port could not be opened or found, the tool is missing, it timed
//!   out, or anything unrecognised ⇒ `Error`.
//!
//! rnodeconf exits 0 on most of these paths (`graceful_exit()`), so the exit
//! code decides nothing. **Unverified on hardware**: the phrases are read
//! from rnodeconf's output statements (RNS 1.5.7, `RNS/Utilities/
//! rnodeconf.py`: its compact log format `[YYYY-MM-DD HH:MM:SS] <msg>`), the
//! golden fixtures in `tests/fixtures/rnodeconf/` are written from them, and
//! no stub here is modelled on Reticulum source (R06 G6). External gate G1
//! (serial open without an unwanted reset on real boards) is still open.

use super::UplinkKind;
use super::{serial_runnable, Abort, BoxFut, IdentifyConfig, Outcome, Probe, Step, StepKind};
use crate::daemon::devices::{clean, DeviceFirmware, FirmwareFamily, FirmwareSource, MAX_FACT};
use crate::daemon::transport::rnstatus::TimedRunner;
use crate::util::run::Runner;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Longest one `rnodeconf --info` may take (it waits ~3.3 s for the
/// detect, then reads the EEPROM).
pub const TIMEOUT: Duration = Duration::from_secs(25);
/// Output parsed at most (checked before the parse).
pub const MAX_OUTPUT: usize = 64 * 1024;

/// `<root>/venvs/rns/bin/rnodeconf`.
pub fn program(root: &Path) -> PathBuf {
    root.join("venvs/rns/bin/rnodeconf")
}

/// A version string from untrusted output: `[0-9A-Za-z._+-]`, ≤ 32 chars.
fn version(raw: &str) -> Option<String> {
    let v = clean(raw, MAX_FACT)?;
    let v: String = v
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
        .take(32)
        .collect();
    (!v.is_empty()).then_some(v)
}

/// The message of one rnodeconf line: the `[date time] ` prefix and
/// surrounding whitespace removed.
fn message(line: &str) -> &str {
    let t = line.trim();
    match t.strip_prefix('[').and_then(|r| r.split_once("] ")) {
        Some((stamp, rest)) if stamp.len() <= 32 && stamp.bytes().any(|b| b.is_ascii_digit()) => {
            rest.trim()
        }
        _ => t,
    }
}

/// `key : value` of a message (`Firmware version   : 1.86`).
fn field<'a>(msg: &'a str, key: &str) -> Option<&'a str> {
    let (k, v) = msg.split_once(':')?;
    (k.trim() == key).then(|| v.trim())
}

/// Read `rnodeconf --info` output (stdout and stderr together; module doc).
/// Unverified on hardware.
pub fn parse_info(text: &str) -> Outcome {
    if text.len() > MAX_OUTPUT {
        return Outcome::Error(format!("rnodeconf printed over {MAX_OUTPUT} bytes"));
    }
    let msgs: Vec<&str> = text.lines().map(message).collect();
    let has = |needle: &str| msgs.iter().any(|m| m.contains(needle));
    let info = msgs.iter().position(|m| *m == "Device info:");
    if let Some(at) = info {
        if let Some(v) = msgs[at..]
            .iter()
            .find_map(|m| field(m, "Firmware version"))
            .and_then(version)
        {
            return Outcome::Match(rnode(Some(v)));
        }
    }
    if has("Device connected") {
        let v = msgs.iter().find_map(|m| {
            m.strip_prefix("Current firmware version:")
                .and_then(|v| version(v.trim()))
        });
        if v.is_some() || info.is_some() {
            return Outcome::Match(rnode(v));
        }
    }
    if has("RNode did not respond") || has("Got invalid response while detecting device") {
        return Outcome::NoMatch;
    }
    if has("Could not open the specified serial port") {
        return Outcome::Error("rnodeconf could not open the port".into());
    }
    if has("Could not find specified port") {
        return Outcome::Error("rnodeconf did not find the port".into());
    }
    Outcome::Error("rnodeconf gave no recognisable answer".into())
}

fn rnode(version: Option<String>) -> DeviceFirmware {
    DeviceFirmware {
        family: FirmwareFamily::Rnode,
        version,
        source: FirmwareSource::Probe,
    }
}

/// The RNode step (module doc).
pub struct RnodeStep {
    runner: Arc<dyn Runner>,
    program: PathBuf,
    phone_like: bool,
}

impl RnodeStep {
    /// A step running `program` through `runner` (tests: a fake rnodeconf).
    pub fn new(runner: Arc<dyn Runner>, program: PathBuf, phone_like: bool) -> Self {
        RnodeStep {
            runner,
            program,
            phone_like,
        }
    }

    /// The production step: [`TimedRunner`] with the config's timeout.
    pub fn production(cfg: &IdentifyConfig) -> Self {
        let mut runner = TimedRunner::new(&cfg.home);
        runner.timeout = cfg.rnode_timeout;
        Self::new(Arc::new(runner), cfg.rnodeconf.clone(), cfg.phone_like)
    }
}

impl Step for RnodeStep {
    fn kind(&self) -> StepKind {
        StepKind::Rnode
    }

    fn tool(&self) -> &'static str {
        "rnodeconf (venvs/rns)"
    }

    fn available(&self, kind: UplinkKind) -> bool {
        kind == UplinkKind::Serial && serial_runnable(self.phone_like) && self.program.is_file()
    }

    fn run<'a>(&'a self, probe: &'a Probe) -> BoxFut<'a, Result<Outcome, Abort>> {
        Box::pin(async move {
            let runner = Arc::clone(&self.runner);
            let program = self.program.to_string_lossy().into_owned();
            let port = probe.path.to_string_lossy().into_owned();
            let ran =
                tokio::task::spawn_blocking(move || runner.run(&program, &["--info", &port])).await;
            Ok(match ran {
                Ok(Ok(out)) => {
                    let mut text = out.stdout;
                    text.push('\n');
                    text.push_str(&out.stderr);
                    parse_info(&text)
                }
                Ok(Err(e)) => {
                    let e = format!("{e:#}");
                    Outcome::Error(if e.contains("killed after") {
                        "rnodeconf timed out".into()
                    } else {
                        format!(
                            "rnodeconf did not run: {}",
                            clean(&e, 160).unwrap_or_default()
                        )
                    })
                }
                Err(e) => Outcome::Error(format!("rnodeconf did not run: {e}")),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_text_is_read_by_phrase_not_exit_code() {
        let ok = "[2026-10-19 12:00:00] Opening serial port /dev/ttyACM0...\n\
                  [2026-10-19 12:00:03] Device connected\n\
                  [2026-10-19 12:00:03] Current firmware version: 1.86\n\
                  [2026-10-19 12:00:03] Reading EEPROM...\n\
                  [2026-10-19 12:00:04] EEPROM checksum correct\n\
                  [2026-10-19 12:00:04] \n\
                  [2026-10-19 12:00:04] Device info:\n\
                  [2026-10-19 12:00:04] \tProduct            : RNode 820 - 1020 MHz (10:aa:13)\n\
                  [2026-10-19 12:00:04] \tFirmware version   : 1.86\n";
        assert_eq!(parse_info(ok), Outcome::Match(rnode(Some("1.86".into()))));
        // an RNode whose EEPROM is blank still runs the firmware
        let blank = "[x 1] Device connected\n[x 1] Current firmware version: 1.74\n\
                     [x 1] EEPROM is invalid, no further information available\n";
        assert_eq!(
            parse_info(blank),
            Outcome::Match(rnode(Some("1.74".into())))
        );
        let no = "[2026-10-19 12:00:00] Opening serial port /dev/ttyACM0...\n\
                  [2026-10-19 12:00:03] Serial port opened, but RNode did not respond. Is a valid firmware installed?\n\
                  Got invalid response while detecting device\n";
        assert_eq!(parse_info(no), Outcome::NoMatch);
        assert!(matches!(
            parse_info(
                "[t 1] Could not open the specified serial port. The contained exception was:\n"
            ),
            Outcome::Error(_)
        ));
        assert!(matches!(parse_info(""), Outcome::Error(_)));
        assert!(matches!(
            parse_info("Traceback (most recent call last):\n"),
            Outcome::Error(_)
        ));
        assert!(matches!(
            parse_info(&"x".repeat(MAX_OUTPUT + 1)),
            Outcome::Error(_)
        ));
        // untrusted version text is cut to a version shape
        let evil = "Device info:\n\tFirmware version   : 1.86\u{202e}; rm -rf /\n";
        assert_eq!(parse_info(evil), Outcome::Match(rnode(Some("1.86".into()))));
    }
}
