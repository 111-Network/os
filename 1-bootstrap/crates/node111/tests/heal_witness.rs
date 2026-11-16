//! Layer 9 (`9-evolution/README.md` §3.3 rules 2 and 4; §20 G19 (3), the witness half; row L1):
//! `111 heal-witness`, run as the real binary the unit's `ExecStopPost=` runs, on a temp
//! `NODE111_ROOT`, with the variables systemd sets for `ExecStopPost=` (systemd.exec(5):
//! `SERVICE_RESULT`, `EXIT_CODE`, `EXIT_STATUS`) standing in for the service manager. Row D12 adds
//! the daemon's cases (the run marker and the start facts) to this file.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// `linux/capability.h`: the two capabilities that let root open a directory whatever its mode.
const CAP_DAC_OVERRIDE: libc::c_ulong = 1;
const CAP_DAC_READ_SEARCH: libc::c_ulong = 2;

/// A node root with `state/heal/` as `111d` creates it at its first start (0700).
fn node_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("state/heal");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    root
}

fn inbox(root: &Path) -> PathBuf {
    root.join("state/heal/inbox")
}

/// `111 <args>` with exactly the service-manager variables `vars` (inherited ones removed).
fn command(root: &Path, args: &[&str], vars: &[(&str, &[u8])]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_111"));
    cmd.args(args).env("NODE111_ROOT", root);
    for k in ["SERVICE_RESULT", "EXIT_CODE", "EXIT_STATUS"] {
        cmd.env_remove(k);
    }
    for (k, v) in vars {
        cmd.env(k, OsStr::from_bytes(v));
    }
    cmd
}

fn run(root: &Path, args: &[&str], vars: &[(&str, &[u8])]) -> Output {
    command(root, args, vars).output().unwrap()
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

/// `111 heal-witness` with these variables; it must exit 0, whatever happened (§3.3 rule 2).
fn witness(root: &Path, vars: &[(&str, &[u8])]) -> Output {
    let out = run(root, &["heal-witness"], vars);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    out
}

/// A run that wrote its line: nothing on stderr either.
fn witness_ok(root: &Path, vars: &[(&str, &[u8])]) {
    let out = witness(root, vars);
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn lines(root: &Path) -> Vec<String> {
    std::fs::read_to_string(inbox(root))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

/// The `boot` field every line must carry: the kernel's boot id without its dashes, or `-`.
fn boot() -> String {
    let raw = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default();
    let hex: String = raw.trim_end().chars().filter(|&c| c != '-').collect();
    if raw.trim_end().len() == 36 && hex.len() == 32 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        hex.to_ascii_lowercase()
    } else {
        "-".to_string()
    }
}

/// §3.3 rule 4's grammar: `v1 boot=<32 hex|-> result=<word ≤16> code=<≤3 digits|-> signal=<≤3
/// digits|->`, ASCII, at most 96 B.
fn grammar_ok(line: &str) -> bool {
    let digits =
        |v: &str| v == "-" || ((1..=3).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_digit()));
    let parts: Vec<&str> = line.split(' ').collect();
    line.len() <= 96
        && line.bytes().all(|b| (0x20..=0x7e).contains(&b))
        && parts.len() == 5
        && parts[0] == "v1"
        && parts[1].strip_prefix("boot=").is_some_and(|v| {
            v == "-"
                || (v.len() == 32
                    && v.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        })
        && parts[2].strip_prefix("result=").is_some_and(|v| {
            (1..=16).contains(&v.len())
                && v.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        && parts[3].strip_prefix("code=").is_some_and(digits)
        && parts[4].strip_prefix("signal=").is_some_and(digits)
}

/// G19 (3): `SERVICE_RESULT=signal EXIT_STATUS=KILL` reads `signal=9`, as do systemd's own
/// `EXIT_CODE=killed` and a watchdog's `dumped` `ABRT` (6).
#[test]
fn a_signal_name_is_written_as_its_number() {
    let root = node_root();
    witness_ok(
        root.path(),
        &[("SERVICE_RESULT", b"signal"), ("EXIT_STATUS", b"KILL")],
    );
    witness_ok(
        root.path(),
        &[
            ("SERVICE_RESULT", b"signal"),
            ("EXIT_CODE", b"killed"),
            ("EXIT_STATUS", b"KILL"),
        ],
    );
    witness_ok(
        root.path(),
        &[
            ("SERVICE_RESULT", b"watchdog"),
            ("EXIT_CODE", b"dumped"),
            ("EXIT_STATUS", b"ABRT"),
        ],
    );
    let b = boot();
    assert_eq!(
        lines(root.path()),
        [
            format!("v1 boot={b} result=signal code=- signal=9"),
            format!("v1 boot={b} result=signal code=- signal=9"),
            format!("v1 boot={b} result=watchdog code=- signal=6"),
        ]
    );
    let mode = std::fs::metadata(inbox(root.path()))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "J2: 0600");
}

/// G19 (3): the line a `111d` exit with status 75 leaves (a store verb held `node/lock`) is written
/// as such, `result=exit-code code=75`; a clean exit as `result=success code=0`.
#[test]
fn result_exit_code_75_is_written_as_such() {
    let root = node_root();
    witness_ok(
        root.path(),
        &[
            ("SERVICE_RESULT", b"exit-code"),
            ("EXIT_CODE", b"exited"),
            ("EXIT_STATUS", b"75"),
        ],
    );
    witness_ok(
        root.path(),
        &[
            ("SERVICE_RESULT", b"success"),
            ("EXIT_CODE", b"exited"),
            ("EXIT_STATUS", b"0"),
        ],
    );
    let b = boot();
    assert_eq!(
        lines(root.path()),
        [
            format!("v1 boot={b} result=exit-code code=75 signal=-"),
            format!("v1 boot={b} result=success code=0 signal=-"),
        ]
    );
}

/// §3.3 rule 2: a signal name the compiled table lacks is written `-`, the line still written.
#[test]
fn an_unknown_signal_name_is_a_dash() {
    let root = node_root();
    for status in [&b"RTMIN+3"[..], b"NOSUCHSIG", b"kill"] {
        witness_ok(
            root.path(),
            &[
                ("SERVICE_RESULT", b"signal"),
                ("EXIT_CODE", b"killed"),
                ("EXIT_STATUS", status),
            ],
        );
    }
    let b = boot();
    assert_eq!(
        lines(root.path()),
        vec![format!("v1 boot={b} result=signal code=- signal=-"); 3]
    );
}

/// G19 (3), J2: 300 exits leave the newest 256 lines, in order, 0600, and no temp file.
#[test]
fn three_hundred_runs_leave_the_newest_256_lines() {
    let root = node_root();
    for i in 0..300 {
        let result = format!("r{i}");
        let status = (i % 256).to_string();
        witness_ok(
            root.path(),
            &[
                ("SERVICE_RESULT", result.as_bytes()),
                ("EXIT_CODE", b"exited"),
                ("EXIT_STATUS", status.as_bytes()),
            ],
        );
    }
    let all = lines(root.path());
    assert_eq!(all.len(), 256);
    let b = boot();
    for (n, line) in all.iter().enumerate() {
        let i = n + 44;
        assert_eq!(
            *line,
            format!("v1 boot={b} result=r{i} code={} signal=-", i % 256)
        );
    }
    let mode = std::fs::metadata(inbox(root.path()))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(
        names(&root.path().join("state/heal")),
        ["inbox"],
        "only the inbox: no temp file stays"
    );
}

/// Review of row L1 (§3.3 rule 4's bound, carried to the inbox's temp files): a witness killed
/// between its temp file's create and its rename (here by SIGXFSZ, its `RLIMIT_FSIZE` 16 B, below
/// any line; in the field a power cut or the stop timeout's kill) leaves `.inbox.tmp-<pid>-0` and
/// no line. The next run removes it before it writes, so at most one ever stays, and a whole run
/// leaves the inbox only.
#[test]
fn a_temp_file_a_killed_run_left_goes_with_the_next_run() {
    let root = node_root();
    let dir = root.path().join("state/heal");
    let vars: &[(&str, &[u8])] = &[("SERVICE_RESULT", b"signal"), ("EXIT_STATUS", b"KILL")];
    witness_ok(root.path(), vars);
    for _ in 0..3 {
        let mut cmd = command(root.path(), &["heal-witness"], vars);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        // SAFETY: setrlimit(2) is async-signal-safe; the closure touches only its own stack.
        unsafe {
            cmd.pre_exec(|| {
                for (resource, limit) in [(libc::RLIMIT_CORE, 0), (libc::RLIMIT_FSIZE, 16)] {
                    let rl = libc::rlimit {
                        rlim_cur: limit,
                        rlim_max: limit,
                    };
                    if libc::setrlimit(resource, &rl) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let child = cmd.spawn().unwrap();
        let pid = child.id();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.signal(), Some(libc::SIGXFSZ), "{out:?}");
        assert_eq!(
            names(&dir),
            [format!(".inbox.tmp-{pid}-0"), "inbox".to_string()],
            "the killed run's temp file, the earlier one's gone"
        );
    }
    witness_ok(root.path(), vars);
    assert_eq!(names(&dir), ["inbox"]);
    let line = format!("v1 boot={} result=signal code=- signal=9", boot());
    assert_eq!(lines(root.path()), [line.clone(), line]);
}

/// Review of row L1: a step that fails after the rename landed, the directory's fsync, leaves the
/// line in the inbox, and the stderr line says so, never "unchanged". `state/heal/` 0300: the
/// witness can create and rename in it but not open it; as root the child first drops
/// CAP_DAC_OVERRIDE and CAP_DAC_READ_SEARCH from its bounding set, so the root it execs has neither.
#[test]
fn a_line_whose_directory_cannot_be_synced_is_reported_in() {
    let root = node_root();
    let dir = root.path().join("state/heal");
    let vars: &[(&str, &[u8])] = &[("SERVICE_RESULT", b"signal"), ("EXIT_STATUS", b"KILL")];
    let mut cmd = command(root.path(), &["heal-witness"], vars);
    // SAFETY: geteuid(2) cannot fail and touches no memory.
    if unsafe { libc::geteuid() } == 0 {
        // SAFETY: prctl(2) is async-signal-safe; the closure touches only its own stack.
        unsafe {
            cmd.pre_exec(|| {
                for cap in [CAP_DAC_OVERRIDE, CAP_DAC_READ_SEARCH] {
                    let zero: libc::c_ulong = 0;
                    if libc::prctl(libc::PR_CAPBSET_DROP, cap, zero, zero, zero) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
    }
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o300)).unwrap();
    let out = cmd.output();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let out = match out {
        Err(e) if e.raw_os_error() == Some(libc::EPERM) => {
            eprintln!("skipped: this root cannot drop CAP_DAC_OVERRIDE ({e})");
            return;
        }
        out => out.unwrap(),
    };
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("the line is in state/heal/inbox"), "{err}");
    assert!(!err.contains("unchanged"), "{err}");
    assert_eq!(
        lines(root.path()),
        [format!("v1 boot={} result=signal code=- signal=9", boot())]
    );
}

/// §3.3 rule 4: every line is at most 96 B with each field at its maximum (84 B: the renderer with
/// both numbers at 3 digits; the verb with the longest values systemd can pass), and hostile values
/// (too long, free text, a second line, non-UTF-8 bytes) still write one line of the grammar.
#[test]
fn every_line_is_at_most_96_bytes_with_each_field_at_its_maximum() {
    use node111::heal_witness::{BootId, Facts, ResultWord, LINE_MAX};
    let max = Facts {
        boot: BootId::parse(b"ffffffff-ffff-ffff-ffff-ffffffffffff\n"),
        result: ResultWord::parse(b"abcdefghijklmnop"),
        code: Some(255),
        signal: Some(255),
    };
    assert_eq!(max.line().len(), 84);
    assert!(max.line().len() <= LINE_MAX && grammar_ok(&max.line()));

    let root = node_root();
    let long = vec![b'a'; 4096];
    let cases: [&[(&str, &[u8])]; 9] = [
        &[
            ("SERVICE_RESULT", b"abcdefghijklmnop"),
            ("EXIT_CODE", b"exited"),
            ("EXIT_STATUS", b"255"),
        ],
        &[
            ("SERVICE_RESULT", b"start-limit-hit"),
            ("EXIT_CODE", b"killed"),
            ("EXIT_STATUS", b"VTALRM"),
        ],
        &[
            ("SERVICE_RESULT", b"core-dump"),
            ("EXIT_CODE", b"dumped"),
            ("EXIT_STATUS", b"64"),
        ],
        &[
            ("SERVICE_RESULT", b"abcdefghijklmnopq"),
            ("EXIT_CODE", b"exited"),
            ("EXIT_STATUS", b"2550"),
        ],
        &[
            ("SERVICE_RESULT", &long),
            ("EXIT_CODE", &long),
            ("EXIT_STATUS", &long),
        ],
        &[
            (
                "SERVICE_RESULT",
                b"success\nv1 boot=- result=success code=0 signal=-",
            ),
            ("EXIT_STATUS", b"9\n"),
        ],
        &[
            ("SERVICE_RESULT", b"exit code"),
            ("EXIT_STATUS", b"TERM signal=9"),
        ],
        &[
            ("SERVICE_RESULT", b"succ\xff\xfe"),
            ("EXIT_CODE", b"\xff"),
            ("EXIT_STATUS", b"K\xc3\xa9LL"),
        ],
        &[("SERVICE_RESULT", b"")],
    ];
    for (n, vars) in cases.iter().enumerate() {
        witness_ok(root.path(), vars);
        assert_eq!(lines(root.path()).len(), n + 1, "one line per run");
    }
    let all = lines(root.path());
    for line in &all {
        assert!(line.len() <= 96 && grammar_ok(line), "{line:?}");
    }
    let b = boot();
    assert_eq!(
        all,
        [
            format!("v1 boot={b} result=abcdefghijklmnop code=255 signal=-"),
            format!("v1 boot={b} result=start-limit-hit code=- signal=26"),
            format!("v1 boot={b} result=core-dump code=- signal=64"),
            format!("v1 boot={b} result=- code=- signal=-"),
            format!("v1 boot={b} result=- code=- signal=-"),
            format!("v1 boot={b} result=- code=- signal=-"),
            format!("v1 boot={b} result=- code=- signal=-"),
            format!("v1 boot={b} result=- code=- signal=-"),
            format!("v1 boot={b} result=- code=- signal=-"),
        ]
    );
    let raw = std::fs::read(inbox(root.path())).unwrap();
    assert!(
        raw.iter()
            .all(|&b| b == b'\n' || (0x20..=0x7e).contains(&b)),
        "ASCII only"
    );
}

/// §3.3 rule 2: with `state/heal/` missing the verb exits 0 and writes no file (`111d` creates the
/// directory, never the witness); so with something else in its place, or an inbox it cannot
/// replace.
#[test]
fn state_heal_missing_exits_0_and_writes_no_file() {
    let vars: &[(&str, &[u8])] = &[("SERVICE_RESULT", b"signal"), ("EXIT_STATUS", b"KILL")];
    let empty = tempfile::tempdir().unwrap();
    let out = witness(empty.path(), vars);
    assert!(String::from_utf8_lossy(&out.stderr).contains("state/heal/ is missing"));
    assert_eq!(
        std::fs::read_dir(empty.path()).unwrap().count(),
        0,
        "no state/"
    );

    std::fs::create_dir(empty.path().join("state")).unwrap();
    witness(empty.path(), vars);
    assert_eq!(
        std::fs::read_dir(empty.path().join("state"))
            .unwrap()
            .count(),
        0,
        "no state/heal/"
    );

    std::fs::write(empty.path().join("state/heal"), b"").unwrap();
    let out = witness(empty.path(), vars);
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a directory"));
    assert_eq!(std::fs::read(empty.path().join("state/heal")).unwrap(), b"");

    // an inbox the rename cannot replace (a directory): exit 0, nothing left behind
    let root = node_root();
    std::fs::create_dir(inbox(root.path())).unwrap();
    let out = witness(root.path(), vars);
    assert!(String::from_utf8_lossy(&out.stderr).contains("could not be written"));
    assert_eq!(names(&root.path().join("state/heal")), ["inbox"]);
    assert!(inbox(root.path()).is_dir());
}

/// §3.3 rule 2: not run by the service manager (`$SERVICE_RESULT` unset): exit 0, nothing written;
/// extra arguments are ignored, never a failure of the unit.
#[test]
fn without_service_result_nothing_is_written() {
    let root = node_root();
    let out = witness(root.path(), &[("EXIT_STATUS", b"KILL")]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("SERVICE_RESULT"));
    assert!(!inbox(root.path()).exists());

    let out = run(
        root.path(),
        &["heal-witness", "extra", "--flag"],
        &[("SERVICE_RESULT", b"success")],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        lines(root.path()),
        [format!("v1 boot={} result=success code=- signal=-", boot())]
    );
}

/// Row L1: the verb is hidden, absent from the usage text, and routed all the same: `111
/// heal-witness` runs (status 0, its own note on stderr), never the unknown-subcommand error.
#[test]
fn the_verb_is_absent_from_the_usage_text() {
    let root = tempfile::tempdir().unwrap();
    for args in [&["help"][..], &["--help"], &[]] {
        let out = run(root.path(), args, &[]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("USAGE:"), "{text}");
        assert!(
            !text.contains("heal-witness") && !text.contains("witness"),
            "{text}"
        );
    }
    let out = witness(root.path(), &[]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("heal-witness: $SERVICE_RESULT is unset"),
        "{err}"
    );
    assert!(!err.contains("unknown subcommand"), "{err}");
}
