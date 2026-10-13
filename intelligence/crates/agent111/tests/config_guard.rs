//! T1 integration: config precedence matrix, env-file errors, flag errors,
//! the model-base guard table, and `Store::init` permissions/idempotence.
//!
//! Everything goes through the pure `Config::resolve` with in-memory env and
//! filesystem closures — no test touches the real process env.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

use agent111::config::{host_is_loopback, host_is_private, Config, DEFAULT_ENV_FILE};
use agent111::store::Store;

fn resolve(env: &[(&str, &str)], argv: &[&str], files: &[(&str, &str)]) -> anyhow::Result<Config> {
    let env_map: HashMap<String, String> = env
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let file_map: HashMap<PathBuf, String> = files
        .iter()
        .map(|(k, v)| (PathBuf::from(k), v.to_string()))
        .collect();
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    Config::resolve(&move |k| env_map.get(k).cloned(), &argv, &move |p| {
        file_map
            .get(p)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "absent"))
    })
}

fn resolve_with_base(base: &str, extra_argv: &[&str]) -> anyhow::Result<Config> {
    let mut argv = vec!["--model-base", base];
    argv.extend_from_slice(extra_argv);
    resolve(&[], &argv, &[])
}

#[test]
fn defaults_when_nothing_set() {
    // Env file at the default path is absent: defaults continue.
    let cfg = resolve(&[], &[], &[]).unwrap();
    assert_eq!(cfg.port, 11182);
    assert_eq!(cfg.model_base, "http://127.0.0.1:11181");
    assert_eq!(cfg.model_id, "qwen3-4b");
    assert_eq!(cfg.kb_dir, PathBuf::from("/var/lib/111/kb/current"));
    assert_eq!(cfg.state_dir, PathBuf::from("/var/lib/111/agent"));
    assert_eq!(cfg.category, "unknown");
    assert_eq!(cfg.daemon_health_url, "http://127.0.0.1:11180/health");
    assert_eq!(cfg.opencode_path, PathBuf::from("opencode"));
    assert!(!cfg.lan);
}

#[test]
fn env_file_overrides_defaults() {
    let file = "# comment\n\n  111_AGENT_PORT=12001 \n111_AGENT_MODEL_ID=phi-4\n111_AGENT_LAN=yes\n111_AGENT_STATE_DIR=/tmp/state\n";
    let cfg = resolve(&[], &[], &[(DEFAULT_ENV_FILE, file)]).unwrap();
    assert_eq!(cfg.port, 12001);
    assert_eq!(cfg.model_id, "phi-4");
    assert!(cfg.lan);
    assert_eq!(cfg.state_dir, PathBuf::from("/tmp/state"));
    // Untouched fields keep defaults.
    assert_eq!(cfg.model_base, "http://127.0.0.1:11181");
}

#[test]
fn precedence_flag_over_env_var_over_env_file_over_default() {
    let file = "111_AGENT_PORT=12001\n111_AGENT_MODEL_BASE=http://10.9.9.9:11181\n";
    let env = [
        ("111_AGENT_PORT", "13001"),
        ("111_AGENT_MODEL_BASE", "http://192.168.0.5:11181"),
    ];
    let flag_base = "http://172.16.0.7:11181";

    // Full chain: flag wins.
    let cfg = resolve(
        &env,
        &["--port", "14001", "--model-base", flag_base],
        &[(DEFAULT_ENV_FILE, file)],
    )
    .unwrap();
    assert_eq!(cfg.port, 14001);
    assert_eq!(cfg.model_base, flag_base);

    // No flag: env var wins over file.
    let cfg = resolve(&env, &[], &[(DEFAULT_ENV_FILE, file)]).unwrap();
    assert_eq!(cfg.port, 13001);
    assert_eq!(cfg.model_base, "http://192.168.0.5:11181");

    // No env var: file wins over default.
    let cfg = resolve(&[], &[], &[(DEFAULT_ENV_FILE, file)]).unwrap();
    assert_eq!(cfg.port, 12001);
    assert_eq!(cfg.model_base, "http://10.9.9.9:11181");

    // Empty env var counts as unset, so the file value shows through.
    let cfg = resolve(&[("111_AGENT_PORT", "")], &[], &[(DEFAULT_ENV_FILE, file)]).unwrap();
    assert_eq!(cfg.port, 12001);

    // --flag=value form parses identically.
    let cfg = resolve(&[], &["--port=14002"], &[]).unwrap();
    assert_eq!(cfg.port, 14002);
}

#[test]
fn env_file_path_from_env_var_and_flag() {
    let files = [
        ("/custom/a.env", "111_AGENT_PORT=12011"),
        ("/custom/b.env", "111_AGENT_PORT=12012"),
    ];
    // Env var selects the file.
    let cfg = resolve(&[("111_AGENT_ENV_FILE", "/custom/a.env")], &[], &files).unwrap();
    assert_eq!(cfg.port, 12011);

    // Flag is honored before the file is read: it beats the env var's path.
    let cfg = resolve(
        &[("111_AGENT_ENV_FILE", "/custom/a.env")],
        &["--env-file", "/custom/b.env"],
        &files,
    )
    .unwrap();
    assert_eq!(cfg.port, 12012);

    // A selected-but-missing env file is fine: defaults continue.
    let cfg = resolve(&[("111_AGENT_ENV_FILE", "/nope.env")], &[], &files).unwrap();
    assert_eq!(cfg.port, 11182);
}

#[test]
fn malformed_env_file_names_the_line() {
    let file = "111_AGENT_PORT=12001\n# fine\n\nbogus line here\n";
    let err = resolve(&[], &[], &[(DEFAULT_ENV_FILE, file)]).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("line 4"), "msg: {msg}");
    assert!(msg.contains(DEFAULT_ENV_FILE), "msg: {msg}");

    let err = resolve(&[], &[], &[(DEFAULT_ENV_FILE, "=nokey\n")]).unwrap_err();
    assert!(format!("{err:#}").contains("line 1"), "{err:#}");
}

#[test]
fn unreadable_env_file_is_an_error() {
    let err = Config::resolve(&|_| None, &[], &|_| {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
    })
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("cannot read env file"),
        "{err:#}"
    );
}

#[test]
fn unknown_flag_is_a_clear_error() {
    let err = resolve(&[], &["--frobnicate"], &[]).unwrap_err();
    assert!(format!("{err:#}").contains("--frobnicate"), "{err:#}");

    let err = resolve(&[], &["stray-positional"], &[]).unwrap_err();
    assert!(format!("{err:#}").contains("unknown"), "{err:#}");
}

#[test]
fn missing_flag_value_is_a_clear_error() {
    let err = resolve(&[], &["--port"], &[]).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("--port"), "msg: {msg}");
    assert!(msg.contains("requires a value"), "msg: {msg}");
}

#[test]
fn bad_port_is_a_clear_error() {
    let err = resolve(&[("111_AGENT_PORT", "abc")], &[], &[]).unwrap_err();
    assert!(format!("{err:#}").contains("port"), "{err:#}");

    let err = resolve(&[], &["--port", "99999"], &[]).unwrap_err();
    assert!(format!("{err:#}").contains("port"), "{err:#}");
}

#[test]
fn guard_allows_loopback_and_private() {
    for base in [
        "http://127.0.0.1:11181",
        "http://localhost:11181",
        "http://[::1]:11181",
        "http://10.7.0.1:11181",
        "http://172.16.5.4:11181",
        "http://172.31.0.2:11181",
        "http://192.168.100.2:11181",
        "http://169.254.10.10:11181",
        "http://[fd00::5]:11181", // IPv6 ULA
        "http://[fe80::1]:11181", // IPv6 link-local
    ] {
        assert!(resolve_with_base(base, &[]).is_ok(), "{base}");
    }
}

#[test]
fn guard_refuses_public_ips_no_wan_ever() {
    for base in [
        "http://8.8.8.8:11181",
        "http://1.1.1.1:11181",
        "http://172.15.0.1:11181", // just outside 172.16/12
        "http://172.32.0.1:11181", // just outside 172.16/12
        "http://[2001:4860:4860::8888]:11181",
    ] {
        let err = resolve_with_base(base, &[]).unwrap_err();
        assert!(
            format!("{err:#}").contains("no WAN inference"),
            "{base}: {err:#}"
        );
    }
}

#[test]
fn guard_refuses_hostnames_ip_literals_only() {
    let err = resolve_with_base("http://model-hub:11181", &[]).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("IP literals only"), "msg: {msg}");
    assert!(msg.contains("no DNS"), "msg: {msg}");
}

#[test]
fn lan_flag_does_not_rescue_a_public_model_base() {
    let err = resolve_with_base("http://8.8.8.8:11181", &["--lan"]).unwrap_err();
    assert!(format!("{err:#}").contains("no WAN inference"), "{err:#}");

    // Same via the env var: lan never influences the guard.
    let err = resolve(
        &[
            ("111_AGENT_MODEL_BASE", "http://8.8.8.8:11181"),
            ("111_AGENT_LAN", "true"),
        ],
        &[],
        &[],
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("no WAN inference"), "{err:#}");
}

#[test]
fn host_helpers_classify() {
    assert!(host_is_loopback("http://127.0.0.1:11181"));
    assert!(host_is_loopback("http://localhost:11181"));
    assert!(host_is_loopback("http://[::1]:11181"));
    assert!(!host_is_loopback("http://10.0.0.1:11181"));

    assert!(host_is_private("http://10.0.0.1:11181"));
    assert!(host_is_private("http://172.16.0.1:11181"));
    assert!(host_is_private("http://192.168.0.1:11181"));
    assert!(host_is_private("http://169.254.1.1:11181"));
    assert!(host_is_private("http://[fd00::1]:11181"));
    assert!(host_is_private("http://[fe80::2]:11181"));
    assert!(!host_is_private("http://8.8.8.8:11181"));
    assert!(!host_is_private("http://127.0.0.1:11181"));
    assert!(!host_is_private("http://model-hub:11181"));
}

#[cfg(unix)]
#[test]
fn store_init_creates_0700_tree_and_is_idempotent() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("state");
    let store = Store::init(&dir).unwrap();
    assert_eq!(store.dir(), dir.as_path());

    for sub in ["", "conversations", "proposals", "tasks"] {
        let p = if sub.is_empty() {
            dir.clone()
        } else {
            dir.join(sub)
        };
        assert!(p.is_dir(), "missing {}", p.display());
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "bad mode for {}", p.display());
    }

    // Second init over the same tree succeeds.
    Store::init(&dir).unwrap();
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}

#[test]
fn store_init_under_a_fresh_tempdir() {
    // Non-unix fallback still asserts the tree exists and init is idempotent.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("state");
    Store::init(&dir).unwrap();
    Store::init(&dir).unwrap();
    for sub in ["conversations", "proposals", "tasks"] {
        assert!(dir.join(sub).is_dir(), "missing {sub}");
    }
}
