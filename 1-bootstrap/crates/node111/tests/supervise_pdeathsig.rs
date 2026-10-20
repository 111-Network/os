//! A supervised child does not outlive `111d` killed with SIGKILL (Layer 5 integrated
//! review m4; `daemon/supervise.rs` module doc, "Parent death"). This test binary
//! re-runs itself as the "daemon": a helper process on a multi-thread runtime, like
//! `111d`, that supervises one `sleep`, writes its pid and waits. The test SIGKILLs the
//! helper (no stop path runs) and expects the child gone within 5 s. Hermetic: every
//! process it starts is killed and reaped on the way out, pass or fail.

use node111::daemon::supervise::{spawn_child, BackoffPolicy, ChildSpec};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Set (to the pid file path) only in the helper process.
const HELPER_ENV: &str = "NODE111_TEST_PDEATHSIG_PIDFILE";
const HELPER_TEST: &str = "pdeathsig_helper_daemon";

/// The helper "daemon". Ignored in a normal run; the test below starts it as a separate
/// process with the env var set (without it, it returns at once).
#[test]
#[ignore = "helper process for a_supervised_child_dies_with_its_sigkilled_parent"]
fn pdeathsig_helper_daemon() {
    let Some(pid_file) = std::env::var_os(HELPER_ENV).map(PathBuf::from) else {
        return;
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("111d")
        .build()
        .unwrap();
    rt.block_on(async move {
        let h = spawn_child(ChildSpec {
            name: "sleeper".into(),
            program: PathBuf::from("/bin/sleep"),
            args: vec!["300".into()],
            env: vec![],
            backoff: BackoffPolicy::default(),
            stdout_log: None,
        });
        let t0 = Instant::now();
        let pid = loop {
            if let Some(pid) = h.pid() {
                break pid;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(10),
                "the child never started"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let tmp = pid_file.with_extension("tmp");
        std::fs::write(&tmp, pid.to_string()).unwrap();
        std::fs::rename(&tmp, &pid_file).unwrap();
        // Live like a daemon until the test SIGKILLs this process; bounded, so a
        // helper whose test died first does not linger.
        tokio::time::sleep(Duration::from_secs(120)).await;
        h.stop_and_wait(Duration::from_secs(5)).await;
    });
}

/// /proc/<pid>/stat as (comm, state, ppid); None when the pid is gone.
fn stat(pid: u32) -> Option<(String, char, u32)> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (head, rest) = s.rsplit_once(')')?;
    let comm = head.split_once('(')?.1.to_string();
    let mut f = rest.split_whitespace();
    let state = f.next()?.chars().next()?;
    let ppid = f.next()?.parse().ok()?;
    Some((comm, state, ppid))
}

/// The helper's `sleep` is still running (not gone, not a zombie awaiting its reaper).
fn sleeper_running(pid: u32) -> bool {
    matches!(stat(pid), Some((comm, state, _)) if comm == "sleep" && !matches!(state, 'Z' | 'X'))
}

/// Kills (SIGKILL) and reaps the helper when dropped.
struct Helper(Child);

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// SIGKILLs the helper's child when dropped, if it is still the helper's `sleep`.
struct Sleeper(u32);

impl Drop for Sleeper {
    fn drop(&mut self) {
        if sleeper_running(self.0) {
            unsafe { libc::kill(self.0 as libc::pid_t, libc::SIGKILL) };
        }
    }
}

fn wait_pid_file(path: &Path, helper: &mut Helper) -> u32 {
    let t0 = Instant::now();
    loop {
        if let Some(pid) = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
        {
            return pid;
        }
        if let Some(status) = helper.0.try_wait().unwrap() {
            panic!("the helper exited ({status}) before its child started");
        }
        assert!(
            t0.elapsed() < Duration::from_secs(20),
            "no pid file from the helper"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_supervised_child_dies_with_its_sigkilled_parent() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("child.pid");
    let mut helper = Helper(
        Command::new(std::env::current_exe().unwrap())
            .args([
                HELPER_TEST,
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(HELPER_ENV, &pid_file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let pid = wait_pid_file(&pid_file, &mut helper);
    let _sleeper = Sleeper(pid);
    let helper_pid = helper.0.id();
    assert!(
        matches!(stat(pid), Some((ref comm, _, ppid)) if comm == "sleep" && ppid == helper_pid),
        "the supervised child is the helper's sleep: {:?}",
        stat(pid)
    );
    // no early signal while the parent lives (a worker thread that retired would cause one)
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        sleeper_running(pid),
        "the child died while its parent lived"
    );

    helper.0.kill().unwrap(); // SIGKILL: the daemon's stop path never runs
    helper.0.wait().unwrap();
    let t0 = Instant::now();
    while sleeper_running(pid) && t0.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !sleeper_running(pid),
        "the supervised child outlived its SIGKILLed parent by 5 s: {:?}",
        stat(pid)
    );
}
