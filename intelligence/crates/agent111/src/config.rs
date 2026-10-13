//! Configuration: defaults < env file (`/etc/111/agent.env`) < `111_AGENT_*`
//! env vars < CLI flags, plus the startup model-base guard (no WAN inference,
//! ever; the `lan` flag never influences it).
//!
//! Resolution is pure: [`Config::resolve`] takes the environment, argv, and
//! the env-file reader as parameters, so tests never touch real process
//! state. [`Config::load`] wires it to the real world.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail};

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub model_base: String,
    pub model_id: String,
    pub kb_dir: PathBuf,
    pub state_dir: PathBuf,
    pub category: String,
    pub daemon_health_url: String,
    pub opencode_path: PathBuf,
    /// Node identity (Layer 2 NodeId hex), handed down by 111d; stamps proposals.
    pub node_id: Option<String>,
    pub lan: bool,
}

/// Default env-file path; overridden by `111_AGENT_ENV_FILE` / `--env-file`.
pub const DEFAULT_ENV_FILE: &str = "/etc/111/agent.env";

impl Default for Config {
    fn default() -> Self {
        Self {
            port: 11182,
            model_base: "http://127.0.0.1:11181".to_string(),
            model_id: "qwen3-4b".to_string(),
            kb_dir: PathBuf::from("/var/lib/111/kb/current"),
            state_dir: PathBuf::from("/var/lib/111/agent"),
            category: "unknown".to_string(),
            daemon_health_url: "http://127.0.0.1:11180/health".to_string(),
            opencode_path: PathBuf::from("opencode"),
            node_id: None,
            lan: false,
        }
    }
}

impl Config {
    /// Load from the real process env and argv, then apply the guard.
    pub fn load() -> anyhow::Result<Self> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        Self::resolve(&env_opt, &args, &read_env_file)
    }

    /// Pure resolution: flags > env vars > env file > defaults. `args` are
    /// argv without the program name. The env-file path itself follows the
    /// same chain (`--env-file` > `111_AGENT_ENV_FILE` > [`DEFAULT_ENV_FILE`])
    /// and is honored before the file is read. A missing env file is fine;
    /// a malformed one is a startup error naming the line. Ends with the
    /// model-base guard: a refusal aborts startup.
    pub fn resolve(
        env: &dyn Fn(&str) -> Option<String>,
        args: &[String],
        read_file: &dyn Fn(&Path) -> std::io::Result<String>,
    ) -> anyhow::Result<Self> {
        let flags = Flags::parse(args)?;

        let env_file = flags
            .env_file
            .clone()
            .or_else(|| env("111_AGENT_ENV_FILE").filter(|v| !v.is_empty()))
            .unwrap_or_else(|| DEFAULT_ENV_FILE.to_string());
        let file_vars = match read_file(Path::new(&env_file)) {
            Ok(content) => parse_env_file(Path::new(&env_file), &content)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => bail!("cannot read env file {env_file}: {e}"),
        };

        let mut cfg = Self::default();
        if let Some(v) = pick(&flags.port, env, &file_vars, "111_AGENT_PORT") {
            cfg.port = v
                .parse()
                .map_err(|_| anyhow!("bad port value {v:?} (expected 0..=65535)"))?;
        }
        if let Some(v) = pick(&flags.model_base, env, &file_vars, "111_AGENT_MODEL_BASE") {
            cfg.model_base = v;
        }
        if let Some(v) = pick(&flags.model_id, env, &file_vars, "111_AGENT_MODEL_ID") {
            cfg.model_id = v;
        }
        if let Some(v) = pick(&flags.kb_dir, env, &file_vars, "111_AGENT_KB_DIR") {
            cfg.kb_dir = PathBuf::from(v);
        }
        if let Some(v) = pick(&flags.state_dir, env, &file_vars, "111_AGENT_STATE_DIR") {
            cfg.state_dir = PathBuf::from(v);
        }
        if let Some(v) = pick(&flags.category, env, &file_vars, "111_AGENT_CATEGORY") {
            cfg.category = v;
        }
        if let Some(v) = pick(
            &flags.daemon_health_url,
            env,
            &file_vars,
            "111_AGENT_DAEMON_HEALTH_URL",
        ) {
            cfg.daemon_health_url = v;
        }
        if let Some(v) = pick(
            &flags.opencode_path,
            env,
            &file_vars,
            "111_AGENT_OPENCODE_PATH",
        ) {
            cfg.opencode_path = PathBuf::from(v);
        }
        if let Some(v) = pick(&flags.node_id, env, &file_vars, "111_AGENT_NODE_ID") {
            cfg.node_id = Some(v);
        }
        if flags.lan {
            cfg.lan = true;
        } else if let Some(v) = pick(&None, env, &file_vars, "111_AGENT_LAN") {
            cfg.lan = parse_truthy(&v);
        }

        guard_model_base(&cfg)?;
        Ok(cfg)
    }
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn read_env_file(path: &Path) -> std::io::Result<String> {
    std::fs::read_to_string(path)
}

/// One field, one precedence chain: flag, then env var, then env-file line.
/// Empty values from env/file count as unset.
fn pick(
    flag: &Option<String>,
    env: &dyn Fn(&str) -> Option<String>,
    file: &HashMap<String, String>,
    key: &str,
) -> Option<String> {
    if let Some(v) = flag {
        return Some(v.clone());
    }
    if let Some(v) = env(key).filter(|v| !v.is_empty()) {
        return Some(v);
    }
    file.get(key).filter(|v| !v.is_empty()).cloned()
}

fn parse_truthy(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes")
}

/// KEY=VALUE lines; `#` comments and blank lines skipped, surrounding
/// whitespace trimmed. Anything else is a startup error naming the line.
fn parse_env_file(path: &Path, content: &str) -> anyhow::Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    for (n, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| {
            anyhow!(
                "{}: malformed line {}: expected KEY=VALUE",
                path.display(),
                n + 1
            )
        })?;
        let key = key.trim();
        if key.is_empty() {
            bail!("{}: malformed line {}: empty key", path.display(), n + 1);
        }
        out.insert(key.to_string(), value.trim().to_string());
    }
    Ok(out)
}

#[derive(Default)]
struct Flags {
    port: Option<String>,
    model_base: Option<String>,
    model_id: Option<String>,
    kb_dir: Option<String>,
    state_dir: Option<String>,
    category: Option<String>,
    daemon_health_url: Option<String>,
    opencode_path: Option<String>,
    node_id: Option<String>,
    env_file: Option<String>,
    lan: bool,
}

impl Flags {
    /// `--name value` and `--name=value` both accepted; `--lan` is bare.
    /// Unknown flags and missing values are clear startup errors.
    fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut f = Self::default();
        let mut i = 0;
        while i < args.len() {
            let arg = args[i].as_str();
            let (name, inline) = match arg.split_once('=') {
                Some((n, v)) if n.starts_with("--") => (n, Some(v)),
                _ => (arg, None),
            };
            let value = |i: &mut usize| -> anyhow::Result<String> {
                if let Some(v) = inline {
                    return Ok(v.to_string());
                }
                *i += 1;
                args.get(*i)
                    .cloned()
                    .ok_or_else(|| anyhow!("flag {name} requires a value"))
            };
            match name {
                "--port" => f.port = Some(value(&mut i)?),
                "--model-base" => f.model_base = Some(value(&mut i)?),
                "--model-id" => f.model_id = Some(value(&mut i)?),
                "--kb-dir" => f.kb_dir = Some(value(&mut i)?),
                "--state-dir" => f.state_dir = Some(value(&mut i)?),
                "--category" => f.category = Some(value(&mut i)?),
                "--daemon-health-url" => f.daemon_health_url = Some(value(&mut i)?),
                "--opencode-path" => f.opencode_path = Some(value(&mut i)?),
                "--node-id" => f.node_id = Some(value(&mut i)?),
                "--env-file" => f.env_file = Some(value(&mut i)?),
                "--lan" => {
                    if inline.is_some() {
                        bail!("flag --lan does not take a value");
                    }
                    f.lan = true;
                }
                other => bail!("unknown flag or argument: {other}"),
            }
            i += 1;
        }
        Ok(f)
    }
}

/// The honesty guard: loopback and private/mesh model bases are allowed
/// (private *is* the hub-offload case); any public IP is refused with no
/// override; non-IP hostnames are refused (IP literals only in v1 — no DNS
/// at startup). Never influenced by `cfg.lan`.
fn guard_model_base(cfg: &Config) -> anyhow::Result<()> {
    let base = &cfg.model_base;
    if host_is_loopback(base) || host_is_private(base) {
        return Ok(());
    }
    match host_ip(base) {
        Some(ip) => {
            bail!("refusing model base {base:?}: {ip} is a public address — no WAN inference, ever")
        }
        None => {
            let host = host_part(base).unwrap_or_default();
            if host.is_empty() {
                bail!(
                    "refusing model base {base:?}: cannot parse a host — use an IP literal (loopback or private)"
                );
            }
            bail!(
                "refusing model base {base:?}: host {host:?} is not an IP literal — IP literals only in v1 (no DNS at startup)"
            )
        }
    }
}

/// True when an OpenAI-compatible base URL points at loopback (local model).
pub fn host_is_loopback(base: &str) -> bool {
    match host_part(base) {
        Some(h) => h == "127.0.0.1" || h == "localhost" || h == "::1" || h == "[::1]",
        None => false,
    }
}

/// True when an OpenAI-compatible base URL points at a private/mesh address
/// (the hub-offload case): IPv4 10/8, 172.16/12, 192.168/16, 169.254/16;
/// IPv6 fc00::/7 (ULA) and fe80::/10 (link-local). Hostnames are not IPs,
/// so they are never "private" here.
pub fn host_is_private(base: &str) -> bool {
    match host_ip(base) {
        Some(IpAddr::V4(v4)) => {
            let o = v4.octets();
            o[0] == 10
                || (o[0] == 172 && (16..=31).contains(&o[1]))
                || (o[0] == 192 && o[1] == 168)
                || (o[0] == 169 && o[1] == 254)
        }
        Some(IpAddr::V6(v6)) => {
            let first = v6.segments()[0];
            (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
        None => false,
    }
}

/// Parse the host portion of a base URL as an IP literal, tolerating the
/// `[v6]` bracket form. Non-IP hostnames yield None.
fn host_ip(base: &str) -> Option<IpAddr> {
    let host = host_part(base)?;
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(&host);
    bare.parse().ok()
}

/// Extract the host portion of a URL, without the port or scheme.
fn host_part(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let hostport = after_scheme.split('/').next()?;
    if hostport.is_empty() {
        return None;
    }
    if let Some(rest) = hostport.strip_prefix('[') {
        // [v6]:port
        let end = rest.find(']')?;
        return Some(format!("[{}]", &rest[..end]));
    }
    Some(hostport.split(':').next()?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_table() {
        assert!(host_is_loopback("http://127.0.0.1:11181"));
        assert!(host_is_loopback("http://localhost:11181"));
        assert!(host_is_loopback("http://[::1]:11181"));
        assert!(!host_is_loopback("http://10.0.0.1:11181"));
        assert!(!host_is_loopback("http://8.8.8.8:11181"));
    }

    #[test]
    fn private_table() {
        for base in [
            "http://10.0.0.1:11181",
            "http://10.255.255.254:11181",
            "http://172.16.0.1:11181",
            "http://172.31.255.254:11181",
            "http://192.168.1.1:11181",
            "http://169.254.0.1:11181",
            "http://[fd00::1]:11181",
            "http://[fc00::abcd]:11181",
            "http://[fe80::1]:11181",
        ] {
            assert!(host_is_private(base), "{base}");
        }
        for base in [
            "http://8.8.8.8:11181",
            "http://172.15.0.1:11181", // just below 172.16/12
            "http://172.32.0.1:11181", // just above 172.16/12
            "http://11.0.0.1:11181",
            "http://127.0.0.1:11181", // loopback is not "private" here
            "http://localhost:11181", // hostnames are not IP literals
            "http://model-hub:11181",
        ] {
            assert!(!host_is_private(base), "{base}");
        }
    }

    #[test]
    fn env_file_parser() {
        let vars = parse_env_file(
            Path::new("test.env"),
            "# comment\n\n 111_AGENT_PORT=12000 \n111_AGENT_MODEL_ID=phi-4\n",
        )
        .unwrap();
        assert_eq!(vars.get("111_AGENT_PORT").unwrap(), "12000");
        assert_eq!(vars.get("111_AGENT_MODEL_ID").unwrap(), "phi-4");
        let err = parse_env_file(Path::new("test.env"), "ok=1\nbogus line\n").unwrap_err();
        assert!(err.to_string().contains("line 2"), "{err}");
    }
}
