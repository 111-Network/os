//! The consented identify step (Layer 5 §14 "Identify", D17 #8;
//! `POST /api/v1/devices/{id}/identify`).
//!
//! 1. The device must offer `identify` (a LoRa board with a port nothing else
//!    drives; the registry decides — `DeviceRegistry::identify_target`).
//! 2. The board port **and** meshmud's downlink are claimed in the port-owner
//!    table for `identify:<id>`; a port someone else holds answers
//!    `409 port_busy` and nothing is opened (a second board while one is bound
//!    collides on the downlink, which the bound meshmud holds).
//! 3. meshmud is started on the port with the **identify profile**: the same
//!    rendered config the service uses (so it never has to be restarted), the
//!    port owned by `identify:<id>`, no presence reader yet, and a restart
//!    policy whose first exit is final (`meshmud::identify_backoff`) — any
//!    exit during the step ends it and never reopens the port.
//! 4. Once the downlink LISTENs, the reader runs once (`--once`) through the
//!    downlink and prints `my_node_info` (one JSON line, ≤ 64 KiB, validated).
//! 5. A Meshtastic answer: `firmware {family: meshtastic, version, source:
//!    probe}` and the binding are persisted in `devices.json`, the claims are
//!    **transferred** to `meshmud` (`PortTable::transfer` — the port is never
//!    free in between), the presence reader starts, and **the identify child
//!    becomes the service child in place** (the same process: one open, one
//!    possible DTR/RTS reset, at this consented step only).
//! 6. Exit code 3 (the board answered nothing Meshtastic): meshmud is stopped,
//!    the claims released, and the honest result is persisted — `firmware
//!    {family: unknown, source: probe}`, no binding (`200`).
//! 7. Anything else (meshmud did not start or exited, no downlink, the reader
//!    failed or timed out, malformed output, the daemon shutting down):
//!    meshmud stopped, claims released, nothing persisted,
//!    `502 upstream_error`. The probe polls the module's `stopping` flag in
//!    its 50 ms loops, so daemon shutdown never waits for the reader's
//!    `--once` timeout (the reader is killed when its future is dropped).
//!    meshmud's liveness is checked again after the reader answers and before
//!    anything is bound or persisted, because the supervisor reaps and records
//!    an exit asynchronously and a quick answer can beat it: an exited child
//!    (supervisor state, or `/proc/<pid>/stat` gone or a zombie) turns any
//!    answer into this `502`.
//! 8. A forget that lands while the probe runs wins: the registry refuses
//!    the result before writing anything ([`DeviceRegistry::record_identify`]),
//!    the service the identify child was becoming is torn down at once
//!    (children stopped, claims released, node DB removed) and the answer is
//!    `404 not_found`.

use super::{meshmud, Meshtastic, MESHMUD_TASK};
use crate::daemon::dashboard::api_v1::Problem;
use crate::daemon::dashboard::bind::pid_alive;
use crate::daemon::dashboard::ApiState;
use crate::daemon::devices::registry::TargetError;
use crate::daemon::devices::store::{board_name, MeshtasticBinding};
use crate::daemon::devices::{
    clean, DeviceFirmware, DeviceRecord, DeviceRegistry, FirmwareFamily, FirmwareSource, MAX_FACT,
};
use crate::daemon::ports::PortBusy;
use crate::daemon::supervise::{self, ChildControl, ChildHandle, ChildState};
use axum::http::StatusCode;
use serde::Deserialize;
use std::fmt;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// Largest `--once` output accepted (checked before parsing).
pub const MAX_ONCE_BYTES: usize = 64 * 1024;
/// The reader's "could not identify" exit code.
pub const EXIT_NOT_IDENTIFIED: i32 = 3;
const POLL: Duration = Duration::from_millis(50);

/// Why an identify did not produce a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentifyError {
    /// The Meshtastic module or the device registry is not running.
    Unavailable,
    NotFound,
    /// The device does not offer `identify` (`400`).
    NotOffered(&'static str),
    /// A port has another owner (`409 port_busy`).
    Busy(PortBusy),
    /// meshmud or the reader did not start or did not answer (`502`).
    Upstream(String),
}

impl fmt::Display for IdentifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdentifyError::Unavailable => f.write_str("identify is not available on this node"),
            IdentifyError::NotFound => f.write_str("no such device"),
            IdentifyError::NotOffered(why) => write!(f, "identify is not offered: {why}"),
            IdentifyError::Busy(b) => b.fmt(f),
            IdentifyError::Upstream(e) => write!(f, "identify failed: {e}"),
        }
    }
}

impl std::error::Error for IdentifyError {}

impl IdentifyError {
    /// The contract problem (openapi `identifyDevice`).
    pub fn problem(&self) -> Problem {
        match self {
            IdentifyError::Unavailable => Problem::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "This device cannot be identified on this node.",
            )
            .detail("The Meshtastic tools (meshmud and the presence reader) are not installed."),
            IdentifyError::NotFound => {
                Problem::new(StatusCode::NOT_FOUND, "not_found", "no such device")
            }
            IdentifyError::NotOffered(why) => Problem::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "This device does not offer identify.",
            )
            .detail(format!("{why}.")),
            IdentifyError::Busy(b) => b.problem(),
            IdentifyError::Upstream(e) => Problem::new(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                "The board could not be identified.",
            )
            .detail(e.clone()),
        }
    }
}

/// `my_node_info` as the reader prints it with `--once` (README of the
/// sidecar). Unknown keys are ignored; every string is untrusted.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MyNodeInfo {
    pub num: u64,
    pub id: String,
    #[serde(default)]
    pub long_name: String,
    #[serde(default)]
    pub short_name: String,
    #[serde(default)]
    pub hw_model: String,
    #[serde(default)]
    pub firmware_version: String,
}

#[derive(Deserialize)]
struct OnceLine {
    schema: u32,
    my_node_info: MyNodeInfo,
}

/// Parse `--once` stdout: the first non-empty line, schema 1, a unicast node
/// number whose `!xxxxxxxx` id matches.
pub fn parse_once(stdout: &[u8]) -> Result<MyNodeInfo, String> {
    if stdout.len() > MAX_ONCE_BYTES {
        return Err(format!("reader output over {MAX_ONCE_BYTES} bytes"));
    }
    let line = stdout
        .split(|b| *b == b'\n')
        .find(|l| !l.iter().all(u8::is_ascii_whitespace))
        .ok_or("the reader printed nothing")?;
    let parsed: OnceLine =
        serde_json::from_slice(line).map_err(|e| format!("reader output is not valid: {e}"))?;
    if parsed.schema != 1 {
        return Err(format!("reader output schema {}", parsed.schema));
    }
    let info = parsed.my_node_info;
    let num = u32::try_from(info.num)
        .ok()
        .filter(|n| *n > 0 && *n < u32::MAX)
        .ok_or("node number out of range")?;
    if info.id != format!("!{num:08x}") {
        return Err("node id does not match its number".into());
    }
    Ok(info)
}

fn shutting_down() -> IdentifyError {
    IdentifyError::Upstream("the node is shutting down".into())
}

/// How the probe ended.
enum Outcome {
    Meshtastic(MyNodeInfo),
    NotMeshtastic,
}

impl Meshtastic {
    fn identify_ctx(&self) -> Result<(Arc<ApiState>, Arc<DeviceRegistry>), IdentifyError> {
        self.ctx().ok_or(IdentifyError::Unavailable)
    }

    /// Run the identify step for `device_id` (see module docs).
    pub async fn identify(&self, device_id: &str) -> Result<DeviceRecord, IdentifyError> {
        let (api, reg) = self.identify_ctx()?;
        let _t = self.transition.lock().await;
        if self.stopping.load(Ordering::SeqCst) {
            return Err(shutting_down());
        }
        let target = reg.identify_target(device_id).map_err(|e| match e {
            TargetError::NotFound => IdentifyError::NotFound,
            TargetError::NotOffered(why) => IdentifyError::NotOffered(why),
            TargetError::Busy { port, owner } => IdentifyError::Busy(PortBusy {
                port: crate::daemon::ports::port_key(&port),
                owner: crate::daemon::ports::Owner {
                    task: owner,
                    since_tick: 0,
                },
            }),
        })?;
        if !(self.cfg.meshmud_installed() && self.cfg.reader_installed()) {
            return Err(IdentifyError::Unavailable);
        }
        let task = format!("identify:{device_id}");
        let tick = self.next_tick();
        self.claim_for(&task, &target.uplink, tick)
            .map_err(IdentifyError::Busy)?;

        let child = match self.start_identify_child(&target.uplink) {
            Ok(c) => c,
            Err(e) => {
                self.release_for(&task, &target.uplink);
                return Err(e);
            }
        };
        let outcome = self.probe(&child).await;
        match outcome {
            Ok(Outcome::Meshtastic(info)) => {
                let Some(binding) = MeshtasticBinding::new(
                    target.uplink.clone(),
                    info.num as u32,
                    Some(&info.hw_model),
                    Some(&info.long_name),
                ) else {
                    self.abort(&task, &target.uplink, &child).await;
                    return Err(IdentifyError::Upstream("invalid my_node_info".into()));
                };
                let firmware = DeviceFirmware {
                    family: FirmwareFamily::Meshtastic,
                    version: clean(&info.firmware_version, MAX_FACT),
                    source: FirmwareSource::Probe,
                };
                // the identify child becomes the service child in place: the
                // claims move to `meshmud` atomically, the process stays
                let tick = self.next_tick();
                let moved = self
                    .ports
                    .transfer(&target.uplink.port_name(), &task, MESHMUD_TASK, tick)
                    .and_then(|()| {
                        self.ports.transfer(
                            &self.cfg.downlink.to_string(),
                            &task,
                            MESHMUD_TASK,
                            tick,
                        )
                    });
                if let Err(e) = moved {
                    self.abort(&task, &target.uplink, &child).await;
                    let _ = self.ports.release(&target.uplink.port_name(), MESHMUD_TASK);
                    return Err(IdentifyError::Upstream(format!(
                        "port hand-over failed: {e}"
                    )));
                }
                let reader = self.spawn_reader();
                *super::lock(&self.service) = Some(super::Service {
                    device_id: device_id.to_string(),
                    uplink: target.uplink.clone(),
                    meshmud: Some(child),
                    reader,
                    port_held_by: None,
                    awaiting_binary: false,
                    identify_born: true,
                });
                {
                    let mut st = super::lock(&self.status);
                    st.board = reg.label(device_id).or_else(|| binding.hw_model.clone());
                    st.firmware_version = firmware.version.clone();
                    st.usb_uplink = target.uplink.is_usb();
                    st.bound_ports = vec![target.uplink.display()];
                }
                self.install_supervised();
                let Ok(rec) = reg.record_identify(&api, device_id, firmware, Some(binding)) else {
                    // forgotten while the probe ran: nothing was persisted, so
                    // nothing may keep running or holding the port either
                    eprintln!(
                        "111 meshtastic: {device_id} was forgotten during identify; meshmud stopped"
                    );
                    self.drop_service(&api).await;
                    return Err(IdentifyError::NotFound);
                };
                eprintln!(
                    "111 meshtastic: {device_id} identified as Meshtastic ({}); meshmud now serves it",
                    rec.firmware
                        .as_ref()
                        .and_then(|f| f.version.as_deref())
                        .unwrap_or("version unknown")
                );
                Ok(rec)
            }
            Ok(Outcome::NotMeshtastic) => {
                self.abort(&task, &target.uplink, &child).await;
                let firmware = DeviceFirmware {
                    family: FirmwareFamily::Unknown,
                    version: None,
                    source: FirmwareSource::Probe,
                };
                reg.record_identify(&api, device_id, firmware, None)
                    .map_err(|_| IdentifyError::NotFound)
            }
            Err(e) => {
                self.abort(&task, &target.uplink, &child).await;
                Err(e)
            }
        }
    }

    /// Stop the identify child and release the identify claims.
    async fn abort(&self, task: &str, uplink: &meshmud::Uplink, child: &ChildHandle) {
        child.stop_and_wait(self.cfg.stop_grace).await;
        self.release_for(task, uplink);
    }

    /// Render the config and spawn meshmud with the identify profile (its
    /// first exit is final: `meshmud::identify_backoff`).
    fn start_identify_child(&self, uplink: &meshmud::Uplink) -> Result<ChildHandle, IdentifyError> {
        meshmud::write_config(&self.cfg.config_path, uplink, self.cfg.downlink).map_err(|e| {
            IdentifyError::Upstream(format!("cannot render meshmud's config: {e:#}"))
        })?;
        Ok(supervise::spawn_child(meshmud::spec(
            &self.cfg.meshmud_bin,
            &self.cfg.config_path,
            &self.cfg.log,
            meshmud::identify_backoff(),
        )))
    }

    /// Wait for the downlink, then run the reader once — ending as soon as
    /// meshmud exits or the daemon starts shutting down.
    async fn probe(&self, child: &ChildHandle) -> Result<Outcome, IdentifyError> {
        let died = || IdentifyError::Upstream("meshmud exited during identify".into());
        // 1. meshmud up and its downlink listening
        let deadline = tokio::time::Instant::now() + self.cfg.downlink_wait;
        let mut seen_running = false;
        loop {
            if self.stopping.load(Ordering::SeqCst) {
                return Err(shutting_down());
            }
            match child.state() {
                ChildState::Running => seen_running = true,
                ChildState::Degraded | ChildState::Stopped => return Err(died()),
                ChildState::Backoff if seen_running => return Err(died()),
                ChildState::Backoff => {}
            }
            if seen_running {
                match crate::daemon::services::probe::listening_under(
                    &self.cfg.proc_net,
                    self.cfg.downlink.port(),
                ) {
                    Some(true) | None => break,
                    Some(false) => {}
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(IdentifyError::Upstream(if seen_running {
                    "meshmud's downlink did not open".into()
                } else {
                    "meshmud did not start".into()
                }));
            }
            tokio::time::sleep(POLL).await;
        }
        // 2. the reader, raced against meshmud exiting and daemon shutdown
        // (dropping `run_once` kills the reader: `kill_on_drop`)
        let watch = async {
            loop {
                if self.stopping.load(Ordering::SeqCst) {
                    return shutting_down();
                }
                if child.state() != ChildState::Running {
                    return died();
                }
                tokio::time::sleep(POLL).await;
            }
        };
        let answer = tokio::select! {
            r = self.run_once() => r,
            e = watch => Err(e),
        };
        // 3. an answer counts only from a meshmud still running: the
        // supervisor sees an exit late, so the exit wins (module doc, 7.)
        match answer {
            Ok(_) if !child_running(child) => Err(died()),
            other => other,
        }
    }

    /// `<python> -I <script> --endpoint <downlink> --once --timeout <s>`.
    async fn run_once(&self) -> Result<Outcome, IdentifyError> {
        let timeout = self.cfg.identify_timeout;
        let inner = timeout
            .saturating_sub(Duration::from_secs(5))
            .max(Duration::from_secs(1));
        let mut cmd = tokio::process::Command::new(&self.cfg.reader_python);
        cmd.arg("-I")
            .arg(&self.cfg.reader_script)
            .args([
                "--endpoint".to_string(),
                self.cfg.downlink.to_string(),
                "--once".to_string(),
                "--timeout".to_string(),
                inner.as_secs().to_string(),
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true);
        let log =
            meshmud::ensure_private_dir(self.cfg.reader_log.parent().unwrap_or(&self.cfg.root))
                .and_then(|()| {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&self.cfg.reader_log)
                });
        match log {
            Ok(f) => cmd.stderr(std::process::Stdio::from(f)),
            Err(_) => cmd.stderr(std::process::Stdio::null()),
        };
        let out = tokio::time::timeout(timeout, cmd.output())
            .await
            .map_err(|_| {
                IdentifyError::Upstream("the presence reader did not answer in time".into())
            })?
            .map_err(|e| IdentifyError::Upstream(format!("cannot run the presence reader: {e}")))?;
        match out.status.code() {
            Some(0) => parse_once(&out.stdout)
                .map(Outcome::Meshtastic)
                .map_err(IdentifyError::Upstream),
            Some(EXIT_NOT_IDENTIFIED) => Ok(Outcome::NotMeshtastic),
            Some(c) => Err(IdentifyError::Upstream(format!(
                "the presence reader failed (exit {c})"
            ))),
            None => Err(IdentifyError::Upstream(
                "the presence reader was killed".into(),
            )),
        }
    }
}

/// The identify child is still up: the supervisor reports it running and its
/// pid is neither gone nor a zombie. The supervisor reaps and records an exit
/// asynchronously, so its state alone can still say `Running` for a child
/// that has already exited.
fn child_running(child: &ChildControl) -> bool {
    child.state() == ChildState::Running && child.pid().is_some_and(pid_running)
}

/// `pid` exists and is not a zombie: the state field of `/proc/<pid>/stat`
/// (after the parenthesised comm) is neither `Z` nor `X`. Without a readable
/// `/proc/<pid>/stat` (reaped, or no procfs), the kernel answers signal 0.
fn pid_running(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => !matches!(
            stat.rsplit_once(')')
                .and_then(|(_, rest)| rest.trim_start().chars().next()),
            None | Some('Z' | 'X' | 'x')
        ),
        Err(_) => pid_alive(pid),
    }
}

/// Display-safe board name from `my_node_info` (exposed for tests).
pub fn display_board(info: &MyNodeInfo) -> Option<String> {
    board_name(&info.hw_model).or_else(|| board_name(&info.long_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn once_output_is_validated() {
        let ok = br#"{"schema":1,"my_node_info":{"num":168430090,"id":"!0a0a0a0a","long_name":"My Board","short_name":"MB","hw_model":"HELTEC_V3","firmware_version":"2.7.11.abcdef0","pio_env":"heltec-v3"}}
"#;
        let info = parse_once(ok).unwrap();
        assert_eq!(info.num, 0x0a0a0a0a);
        assert_eq!(info.firmware_version, "2.7.11.abcdef0");
        assert_eq!(display_board(&info).as_deref(), Some("HELTEC_V3"));
        // leading blank lines are fine
        let mut padded = b"\n  \n".to_vec();
        padded.extend_from_slice(ok);
        assert!(parse_once(&padded).is_ok());
        for bad in [
            &b""[..],
            b"\n\n",
            b"not json\n",
            br#"{"schema":2,"my_node_info":{"num":1,"id":"!00000001"}}"#,
            br#"{"schema":1,"my_node_info":{"num":0,"id":"!00000000"}}"#,
            br#"{"schema":1,"my_node_info":{"num":4294967295,"id":"!ffffffff"}}"#,
            br#"{"schema":1,"my_node_info":{"num":1,"id":"!deadbeef"}}"#,
            br#"{"schema":1,"my_node_info":{"num":"1","id":"!00000001"}}"#,
            br#"{"schema":1}"#,
        ] {
            assert!(
                parse_once(bad).is_err(),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
        let huge = vec![b' '; MAX_ONCE_BYTES + 1];
        assert!(parse_once(&huge).is_err());
    }

    /// The race the post-answer check closes: the identify child has exited
    /// but its supervisor has not observed that yet. On a current-thread
    /// runtime that does not yield, the supervisor task cannot reap, so the
    /// child stays a zombie while its state still says `Running`.
    #[tokio::test(flavor = "current_thread")]
    async fn an_exit_the_supervisor_has_not_seen_is_not_running() {
        let child = supervise::spawn_child(supervise::ChildSpec {
            name: "identify-liveness".into(),
            program: "sleep".into(),
            args: vec!["30".into()],
            env: Vec::new(),
            backoff: meshmud::identify_backoff(),
            stdout_log: None,
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while child.state() != ChildState::Running || child.pid().is_none() {
            assert!(std::time::Instant::now() < deadline, "the child never ran");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let pid = child.pid().unwrap();
        assert!(child_running(&child), "a live child counted as gone");
        // kill it and wait for the zombie without yielding to the runtime
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        while pid_running(pid) {
            assert!(
                std::time::Instant::now() < deadline,
                "pid {pid} never exited"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        // the supervisor has not seen the exit, the check has
        assert_eq!(child.state(), ChildState::Running);
        assert_eq!(child.pid(), Some(pid));
        assert!(!child_running(&child), "a zombie counted as running");
        // once the supervisor runs it reaps the child; nothing is left behind
        child.stop_and_wait(Duration::from_secs(2)).await;
        assert_eq!(child.state(), ChildState::Stopped);
        assert!(!child_running(&child));
        assert!(!pid_running(pid));
    }

    #[test]
    fn problems_follow_the_contract() {
        let busy = PortBusy {
            port: "/dev/ttyACM0".into(),
            owner: crate::daemon::ports::Owner {
                task: "rnsd".into(),
                since_tick: 1,
            },
        };
        for (e, status, code) in [
            (IdentifyError::NotFound, 404, "not_found"),
            (IdentifyError::NotOffered("x"), 400, "bad_request"),
            (IdentifyError::Unavailable, 400, "bad_request"),
            (IdentifyError::Busy(busy), 409, "port_busy"),
            (IdentifyError::Upstream("x".into()), 502, "upstream_error"),
        ] {
            let p = e.problem();
            assert_eq!((p.status, p.code), (status, code), "{e}");
        }
    }
}
