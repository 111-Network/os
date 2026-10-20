//! Piped stdio for supervised children (Layer 6 §7 "Supervisor extension";
//! `daemon/supervise.rs` module doc, "Piped stdio"): shell children like the
//! pdeathsig test uses. Lines flow from the child's stdout, an overlong or
//! non-UTF-8 line is dropped and counted, a full queue drops its oldest lines,
//! a stdin write reaches the child, every respawn publishes a new generation,
//! and the reader ends with the child (no task outlives it). Hermetic: every
//! child is stopped and reaped on the way out (and dies with the test process
//! through the parent-death signal otherwise).

use node111::daemon::supervise::{
    spawn_child, BackoffPolicy, ChildHandle, ChildSpec, ChildState, ChildStdio, PipedIo,
    PIPED_LINE_MAX, PIPED_QUEUE_CAP,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const LIMIT: Duration = Duration::from_secs(5);

fn sh(script: &str, initial_ms: u64) -> ChildSpec {
    ChildSpec {
        name: "piped-test".into(),
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".into(), script.into(), "sh".into()],
        env: vec![],
        backoff: BackoffPolicy {
            initial_ms,
            max_ms: initial_ms.max(20),
            jitter_pct: 0,
            ..BackoffPolicy::default()
        },
        stdout_log: None,
    }
}

fn piped(script: &str, initial_ms: u64) -> ChildHandle {
    spawn_child(sh(script, initial_ms).with_stdio(ChildStdio::Piped))
}

/// The current spawn's stdio once published.
async fn attach(h: &ChildHandle) -> PipedIo {
    let mut rx = h.stdio();
    let got = tokio::time::timeout(LIMIT, rx.wait_for(|io| io.is_some()))
        .await
        .expect("stdio published")
        .expect("supervisor alive");
    got.clone().unwrap()
}

async fn recv(io: &PipedIo) -> Option<String> {
    tokio::time::timeout(LIMIT, io.recv_line())
        .await
        .expect("a line or the end within the limit")
}

async fn until(limit: Duration, f: impl Fn() -> bool) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < limit {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    f()
}

fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| {
        !matches!(
            s.rsplit_once(')')
                .and_then(|(_, rest)| rest.trim_start().chars().next()),
            None | Some('Z') | Some('X')
        )
    })
}

#[tokio::test]
async fn lines_flow_from_the_child() {
    let h = piped("printf 'hello\\nwörld\\r\\nlast'; exec sleep 30", 20);
    let io = attach(&h).await;
    assert!(io.generation() > 0);
    assert_eq!(recv(&io).await.as_deref(), Some("hello"));
    assert_eq!(
        recv(&io).await.as_deref(),
        Some("wörld"),
        "UTF-8 kept, CR stripped"
    );
    // the first line is kept for a health check without being consumed again
    assert_eq!(io.first_line().await.as_deref(), Some("hello"));
    assert_eq!(
        io.try_recv_line(),
        None,
        "an unterminated line waits for EOF"
    );
    assert_eq!(h.state(), ChildState::Running);
    h.stop_and_wait(LIMIT).await;
    // EOF delivers the last unterminated line, then the end
    assert_eq!(recv(&io).await.as_deref(), Some("last"));
    assert_eq!(recv(&io).await, None);
    assert_eq!(io.dropped_invalid(), 0);
    assert!(
        h.stdio().borrow().is_none(),
        "nothing published while stopped"
    );
}

#[tokio::test]
async fn an_overlong_or_invalid_line_is_dropped_and_counted() {
    // 70 KiB of x on one line, then a non-UTF-8 line, then a good one
    let script = "head -c 71680 /dev/zero | tr '\\000' x; echo; \
                  printf 'bad \\377\\n'; echo after; exec sleep 30";
    let h = piped(script, 20);
    let io = attach(&h).await;
    assert_eq!(recv(&io).await.as_deref(), Some("after"));
    assert_eq!(
        io.dropped_invalid(),
        2,
        "the 70 KiB line and the invalid one"
    );
    // exactly at the cap still passes
    h.stop_and_wait(LIMIT).await;
    let at_cap = format!("head -c {PIPED_LINE_MAX} /dev/zero | tr '\\000' y; echo; exec sleep 30");
    let h = piped(&at_cap, 20);
    let io = attach(&h).await;
    assert_eq!(recv(&io).await.map(|l| l.len()), Some(PIPED_LINE_MAX));
    assert_eq!(io.dropped_invalid(), 0);
    h.stop_and_wait(LIMIT).await;
}

#[tokio::test]
async fn a_full_queue_drops_the_oldest_lines() {
    let extra = 76;
    let total = PIPED_QUEUE_CAP + extra;
    // print `total` numbered lines and exit; the slow respawn keeps the run single
    let script = format!("i=0; while [ $i -lt {total} ]; do echo $i; i=$((i+1)); done");
    let h = piped(&script, 60_000);
    let io = attach(&h).await;
    assert!(
        until(LIMIT, || io.is_closed()).await,
        "the reader ends with the child"
    );
    assert_eq!(io.dropped_overflow(), extra as u64);
    let mut got = Vec::new();
    while let Some(line) = recv(&io).await {
        got.push(line);
    }
    assert_eq!(got.len(), PIPED_QUEUE_CAP);
    assert_eq!(
        got.first().map(String::as_str),
        Some("76"),
        "oldest dropped"
    );
    assert_eq!(got.last().map(String::as_str), Some("1099"));
    h.stop_and_wait(LIMIT).await;
}

#[tokio::test]
async fn a_stdin_write_reaches_the_child() {
    // the child echoes each line back with a prefix
    let h = piped("while read -r l; do echo \"got:$l\"; done", 20);
    let io = attach(&h).await;
    io.write_line(r#"{"type":"send","id":1}"#).await.unwrap();
    assert_eq!(
        recv(&io).await.as_deref(),
        Some(r#"got:{"type":"send","id":1}"#)
    );
    // one protocol line, never two
    let err = io.write_line("a\nb").await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let err = io
        .write_line(&"z".repeat(PIPED_LINE_MAX + 1))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    h.stop_and_wait(LIMIT).await;
}

#[tokio::test]
async fn a_respawn_publishes_a_new_generation() {
    let h = piped("echo up; exec sleep 30", 20);
    let first = attach(&h).await;
    assert_eq!(recv(&first).await.as_deref(), Some("up"));
    let old_pid = h.pid().unwrap();
    let mut rx = h.stdio();
    rx.mark_unchanged();
    assert!(h.control().restart(LIMIT).await);
    let second = tokio::time::timeout(
        LIMIT,
        rx.wait_for(|io| {
            io.as_ref()
                .is_some_and(|io| io.generation() != first.generation())
        }),
    )
    .await
    .expect("re-published")
    .unwrap()
    .clone()
    .unwrap();
    assert!(second.generation() > first.generation());
    assert_eq!(recv(&second).await.as_deref(), Some("up"));
    // the old attachment ended with its child
    assert!(first.is_closed());
    assert_eq!(recv(&first).await, None);
    let err = first.write_line("late").await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
    assert!(!alive(old_pid));
    h.stop_and_wait(LIMIT).await;
}

#[tokio::test]
async fn the_reader_ends_when_the_child_exits() {
    let h = piped("echo bye", 60_000);
    let io = attach(&h).await;
    assert_eq!(recv(&io).await.as_deref(), Some("bye"));
    assert_eq!(recv(&io).await, None, "closed once the child exited");
    assert!(io.is_closed());
    // the supervisor dropped its stdin and published that nothing runs
    let mut rx = h.stdio();
    tokio::time::timeout(LIMIT, rx.wait_for(|io| io.is_none()))
        .await
        .expect("unpublished after the exit")
        .unwrap();
    assert_eq!(
        io.write_line("x").await.unwrap_err().kind(),
        std::io::ErrorKind::BrokenPipe
    );
    h.stop_and_wait(LIMIT).await;
    assert_eq!(h.state(), ChildState::Stopped);
}

#[tokio::test]
async fn a_null_child_publishes_no_stdio() {
    let h = spawn_child(sh("exec sleep 30", 20));
    assert!(until(LIMIT, || h.pid().is_some()).await);
    assert!(h.stdio().borrow().is_none());
    h.stop_and_wait(LIMIT).await;
}
