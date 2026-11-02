//! `rnstatus`, adopted from `venvs/rns` (`os/6-transport/README.md` §11
//! "Live attach, detach, reload" and "`transport.read`"; R05 §3.9).
//!
//! - **Control.** [`Rnstatus::attach`] / [`Rnstatus::detach`] /
//!   [`Rnstatus::reload`] run `rnstatus --config <root>/rns --attach|--detach|
//!   --reload <name>` over rnsd's control RPC (RNS ≥ 1.5.5, *run* against
//!   1.5.7: attach re-reads the config file and starts that stanza, a
//!   missing name answers "does not exist"). The name is one argv element;
//!   no shell is involved. They replace v1's `pkill -f rnsd`.
//! - **Facts.** [`Rnstatus::read`] parses rnstatus's text output
//!   ([`parse_text`]) into [`InterfaceFacts`] `{name, type, up, rx, tx,
//!   peers}`. `rnstatus -j` is not used: its schema has not been captured
//!   from a run (*unverified*; [`parse_json`] reads the shape the RNS source
//!   prints and is tested against an unverified fixture only). Names and
//!   types are rnsd's strings: untrusted text, cleaned and capped.
//! - **Cache.** rnstatus is a Python process, so it runs **on a tick into a
//!   [`FactsCache`]** ([`tick`]); the route and the agent tool read the
//!   cache, never the request path. The tick runs it only while rnsd runs
//!   **with a shared instance** ([`Rnstatus::shares_instance`]: with
//!   `share_instance = No` there is no control RPC to ask) and someone read
//!   the facts within [`TickConfig::demand_window`] — demand only, never
//!   "no reading yet" (B4 review F2: that clause spawned Python every
//!   backoff period for the daemon's lifetime with nobody reading) — backs
//!   off on failure, and is woken early when a reader finds the cache stale:
//!   an idle node starts no Python process on a timer (the Pi floor; the
//!   `rnsd.log` heuristic carries the display state meanwhile). No wall
//!   clock: `Instant` only.
//! - **Parsing is fail-closed** on any input (B4 review F1): rnstatus's
//!   output is untrusted text, so every slice is taken at a char boundary
//!   and an unparsed value is `None`, never a panic.
//! - **Ownership gate (fix wave FA1, §11 "rnsd supervision", §16 G4).**
//!   Every rnstatus run builds an `RNS.Reticulum` that binds the
//!   shared-instance socket `@rns/<instance_name>` *first* and detaches it
//!   only afterwards (RNS 1.5.7 `Reticulum.py` `__start_local_interface`,
//!   `require_shared_instance`); run while rnsd is down or still starting,
//!   it briefly *is* the instance, and an rnsd respawned in that window
//!   attaches as its client and never serves again. So once
//!   [`Rnstatus::set_owner_gate`] names the supervised rnsd's pid (the
//!   daemon does at transport start), **no rnstatus is started unless
//!   `@rns/<instance>` is listening and held by that pid** ([`owner`]:
//!   `/proc/net/unix` and `/proc/<pid>/fd`, no Python); otherwise the call
//!   answers [`RnstatusError::NotOwned`] without spawning anything. This
//!   covers the facts tick, the applier's probe and the spine's
//!   attach/detach/reload; the components runner's serving probe asks
//!   [`owner::supervised_ready`] for the same fact.
//! - **Runner.** Through [`Runner`] (tests inject a fake); production uses
//!   [`TimedRunner`], which kills an rnstatus that runs past
//!   [`RNSTATUS_TIMEOUT`] and caps its output at [`MAX_OUTPUT`] before any
//!   parse.

use crate::daemon::devices::clean;
use crate::util::run::{Output, Runner};
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Longest one rnstatus run may take.
pub const RNSTATUS_TIMEOUT: Duration = Duration::from_secs(10);
/// Output cap per stream (bytes), applied before parsing.
pub const MAX_OUTPUT: usize = 256 * 1024;
/// Most interfaces kept from one reading.
pub const MAX_FACTS: usize = 64;

/// One rnsd interface (contract `TransportInterface`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InterfaceFacts {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub up: bool,
    /// Bytes received (rnstatus prints a rounded decimal size).
    pub rx: u64,
    /// Bytes sent.
    pub tx: u64,
    /// Heard peers, connected clients or served programs; `None` when the
    /// type reports none.
    pub peers: Option<u64>,
}

/// Why an rnstatus call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RnstatusError {
    /// It could not be started (absent venv) or was killed at the timeout.
    Spawn(String),
    /// No shared instance answered (rnsd down, or not sharing).
    NoInstance,
    /// `--attach|--detach|--reload` of a name rnsd does not know.
    NotFound(String),
    /// A non-zero exit with this (cleaned, capped) detail.
    Exit { status: i32, detail: String },
    /// Not run (FA1): the shared instance is not (yet) held by the
    /// supervised rnsd, so an rnstatus started now could take it over.
    NotOwned(String),
}

impl std::fmt::Display for RnstatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RnstatusError::Spawn(e) => write!(f, "rnstatus did not run: {e}"),
            RnstatusError::NoInstance => f.write_str("no shared rnsd instance answered"),
            RnstatusError::NotFound(n) => write!(f, "rnsd has no interface named {n:?}"),
            RnstatusError::Exit { status, detail } => {
                write!(f, "rnstatus exited {status}: {detail}")
            }
            RnstatusError::NotOwned(why) => write!(f, "rnstatus not run: {why}"),
        }
    }
}

impl std::error::Error for RnstatusError {}

/// Where the supervised rnsd's pid is read (`None` while it is not
/// running). Blocking-safe and cheap: called before every rnstatus run.
pub type PidSource = Arc<dyn Fn() -> Option<u32> + Send + Sync>;

/// rnstatus against one rnsd config dir.
pub struct Rnstatus {
    runner: Arc<dyn Runner>,
    program: String,
    config_dir: String,
    root: PathBuf,
    /// The rendered config's `share_instance` (set by the spine on every
    /// render, §11); the facts tick reads nothing while it is false.
    shared: AtomicBool,
    /// FA1: the supervised rnsd's pid; when set, nothing runs unless that
    /// pid holds the shared instance (module doc). `None`: ungated (test
    /// runners only; the daemon always sets it).
    gate: std::sync::RwLock<Option<PidSource>>,
}

impl Rnstatus {
    /// `<root>/venvs/rns/bin/rnstatus --config <root>/rns` through `runner`.
    pub fn new(runner: Arc<dyn Runner>, root: &Path) -> Self {
        Rnstatus {
            runner,
            program: program(root).to_string_lossy().into_owned(),
            config_dir: root.join("rns").to_string_lossy().into_owned(),
            root: root.to_path_buf(),
            shared: AtomicBool::new(true),
            gate: std::sync::RwLock::new(None),
        }
    }

    /// FA1: from now on run nothing unless the pid `rnsd` answers holds the
    /// shared-instance socket (module doc). The daemon sets it at transport
    /// start, before rnsd is spawned (the source answers `None` until then).
    pub fn set_owner_gate(&self, rnsd: PidSource) {
        *self.gate.write().unwrap_or_else(|p| p.into_inner()) = Some(rnsd);
    }

    /// The daemon root this runs against.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The owner gate (FA1): `Ok` when ungated or when the supervised rnsd
    /// holds `@rns/<instance>`; never starts a process.
    pub fn gate(&self) -> Result<(), RnstatusError> {
        let src = self.gate.read().unwrap_or_else(|p| p.into_inner()).clone();
        match src {
            None => Ok(()),
            Some(pid) => owner::ready(&self.root, pid()).map_err(RnstatusError::NotOwned),
        }
    }

    /// Record the rendered config's `share_instance` (the spine calls this
    /// at start and after every write). Default true.
    pub fn set_shares_instance(&self, on: bool) {
        self.shared.store(on, Ordering::SeqCst);
    }

    /// Whether rnsd runs a shared instance (a control RPC rnstatus can ask).
    pub fn shares_instance(&self) -> bool {
        self.shared.load(Ordering::SeqCst)
    }

    /// The production runner: [`TimedRunner`] with `HOME=<root>`.
    pub fn production(root: &Path) -> Self {
        Self::new(Arc::new(TimedRunner::new(root)), root)
    }

    /// The program path this runs.
    pub fn program(&self) -> &str {
        &self.program
    }

    fn control(&self, flag: &str, name: &str) -> Result<(), RnstatusError> {
        self.gate()?;
        let out = self
            .runner
            .run(&self.program, &["--config", &self.config_dir, flag, name])
            .map_err(|e| RnstatusError::Spawn(detail(&format!("{e:#}"))))?;
        let text = format!("{}\n{}", out.stdout, out.stderr);
        if text.contains("does not exist") {
            return Err(RnstatusError::NotFound(name.to_string()));
        }
        if no_instance(&text) {
            return Err(RnstatusError::NoInstance);
        }
        if !out.ok() {
            return Err(RnstatusError::Exit {
                status: out.status,
                detail: detail(&text),
            });
        }
        Ok(())
    }

    /// Start the stanza `name` from the config file (`--attach`).
    pub fn attach(&self, name: &str) -> Result<(), RnstatusError> {
        self.control("--attach", name)
    }

    /// Stop the interface `name` (`--detach`).
    pub fn detach(&self, name: &str) -> Result<(), RnstatusError> {
        self.control("--detach", name)
    }

    /// Re-read and restart the interface `name` (`--reload`).
    pub fn reload(&self, name: &str) -> Result<(), RnstatusError> {
        self.control("--reload", name)
    }

    /// One reading of the interface facts (blocking).
    pub fn read(&self) -> Result<Vec<InterfaceFacts>, RnstatusError> {
        self.gate()?;
        let out = self
            .runner
            .run(&self.program, &["--config", &self.config_dir])
            .map_err(|e| RnstatusError::Spawn(detail(&format!("{e:#}"))))?;
        if no_instance(&out.stdout) || no_instance(&out.stderr) {
            return Err(RnstatusError::NoInstance);
        }
        if !out.ok() {
            return Err(RnstatusError::Exit {
                status: out.status,
                detail: detail(&format!("{}\n{}", out.stdout, out.stderr)),
            });
        }
        Ok(parse_text(&out.stdout))
    }
}

/// `<root>/venvs/rns/bin/rnstatus`.
pub fn program(root: &Path) -> PathBuf {
    root.join("venvs/rns/bin/rnstatus")
}

fn no_instance(text: &str) -> bool {
    text.contains("No shared RNS instance") || text.contains("Could not get RNS status")
}

/// A one-line, cleaned, capped detail of untrusted output.
fn detail(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("");
    clean(line, 160).unwrap_or_default()
}

// ---------------------------------------------------------------- parsing

/// A size as rnstatus prints it (`RNS.prettysize`: decimal units, e.g.
/// `980 B`, `1.21 KB`, `3.40 MB`) in bytes, rounded; `None` when unparsed.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: f64 = num.parse().ok()?;
    let unit = unit.trim();
    let mult = match unit.trim_end_matches('B').trim() {
        "" => 1.0,
        "K" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        "P" => 1e15,
        "E" => 1e18,
        "Z" => 1e21,
        "Y" => 1e24,
        _ => return None,
    };
    if !unit.ends_with('B') && !unit.is_empty() {
        return None;
    }
    let v = (n * mult).round();
    Some(if v >= u64::MAX as f64 {
        u64::MAX
    } else {
        v as u64
    })
}

/// The size next to an arrow in a traffic segment: `↑1.21 KB  0 bps` (RNS
/// 1.x) or `1.21 KB↑` (older releases). Fail-closed on any input (B4
/// review F1): every index is a char boundary (the byte after a matched
/// char, never `+ 1`), so the other arrow or any non-ASCII byte rnsd prints
/// before a size cannot panic; when the new form does not parse (`1.21 KB↑
/// 980 B↓`: both arrows on one line) the old form is tried.
fn size_at(segment: &str, arrow: char) -> Option<u64> {
    let at = segment.find(arrow)?;
    let after = segment[at + arrow.len_utf8()..].trim_start();
    if after.starts_with(|c: char| c.is_ascii_digit()) {
        // new form: the size follows, up to two spaces (then the rate)
        let end = after.find("  ").unwrap_or(after.len());
        if let Some(v) = parse_size(&after[..end]) {
            return Some(v);
        }
    }
    // old form: the size precedes the arrow
    let before = segment[..at].trim_end();
    let start = before
        .char_indices()
        .rev()
        .find(|&(_, c)| !(c.is_ascii_alphanumeric() || c == '.' || c == ' '))
        .map_or(0, |(i, c)| i + c.len_utf8());
    parse_size(before.get(start..)?.trim())
}

fn header(line: &str) -> Option<(String, String)> {
    if !line.starts_with(' ') || line.starts_with("  ") {
        return None;
    }
    let t = line.trim();
    let open = t.find('[')?;
    let inner = t.strip_suffix(']')?.get(open + 1..)?;
    let kind = t[..open].trim();
    if kind.is_empty() {
        return None;
    }
    let name = if kind == "Shared Instance" {
        inner
    } else {
        // `TCPInterface[Name/10.0.0.1:4242]`: stanza names never hold `/`
        inner.split('/').next().unwrap_or(inner)
    };
    Some((clean(kind, 64)?, clean(name, 128).unwrap_or_default()))
}

fn leading_int(s: &str) -> Option<u64> {
    let s = s.trim();
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    s[..end].parse().ok()
}

/// Parse rnstatus's text output (no `-a`: spawned client sub-interfaces
/// are not listed). Golden-tested against `tests/fixtures/rnstatus/*`.
pub fn parse_text(text: &str) -> Vec<InterfaceFacts> {
    let mut out: Vec<InterfaceFacts> = Vec::new();
    let mut in_traffic = false;
    for line in text.lines() {
        if let Some((kind, name)) = header(line) {
            if out.len() >= MAX_FACTS {
                break;
            }
            out.push(InterfaceFacts {
                name,
                kind,
                up: false,
                rx: 0,
                tx: 0,
                peers: None,
            });
            in_traffic = false;
            continue;
        }
        let Some(cur) = out.last_mut() else { continue };
        let t = line.trim();
        let (key, value) = match t.split_once(':') {
            Some((k, v)) if !k.contains(['↑', '↓']) => (k.trim(), v),
            _ => ("", t),
        };
        match key {
            "Status" => {
                cur.up = value.trim().starts_with("Up");
                in_traffic = false;
            }
            "Clients" | "Serving" | "Peers" => {
                cur.peers = leading_int(value);
                in_traffic = false;
            }
            "Traffic" => in_traffic = true,
            "" => {}
            _ => in_traffic = false,
        }
        if in_traffic && (key == "Traffic" || key.is_empty()) {
            if let Some(tx) = size_at(value, '↑') {
                cur.tx = tx;
            }
            if let Some(rx) = size_at(value, '↓') {
                cur.rx = rx;
            }
        }
    }
    out
}

/// Parse `rnstatus -j` (*unverified*: the shape the RNS source prints,
/// `{"interfaces": [{"name", "short_name", "type", "status", "rxb", "txb",
/// "clients", "peers", …}], …}`; not used until a run captures it).
pub fn parse_json(text: &str) -> Option<Vec<InterfaceFacts>> {
    if text.len() > MAX_OUTPUT {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let list = v.get("interfaces")?.as_array()?;
    let mut out = Vec::new();
    for i in list.iter().take(MAX_FACTS) {
        let full = i.get("name").and_then(|n| n.as_str()).unwrap_or("");
        let (hkind, hname) = header(&format!(" {full}")).unwrap_or_default();
        let name = i
            .get("short_name")
            .and_then(|n| n.as_str())
            .and_then(|n| clean(n, 128))
            .unwrap_or(hname);
        let kind = i
            .get("type")
            .and_then(|n| n.as_str())
            .and_then(|n| clean(n, 64))
            .unwrap_or(hkind);
        let num = |k: &str| i.get(k).and_then(serde_json::Value::as_u64);
        out.push(InterfaceFacts {
            name,
            kind,
            up: i
                .get("status")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            rx: num("rxb").unwrap_or(0),
            tx: num("txb").unwrap_or(0),
            peers: num("peers").or_else(|| num("clients")),
        });
    }
    Some(out)
}

// ---------------------------------------------------------------- owner

/// Who holds the shared-instance socket (fix wave FA1; §11 "rnsd
/// supervision", §16 G4 "a double crash that must not partition").
///
/// RNS on Linux serves the shared instance on the abstract `AF_UNIX`
/// socket `\0rns/<instance_name>` (RNS 1.5.7 `LocalInterface.py`: the
/// renderer writes `instance_name` into `[reticulum]`, and never
/// `shared_instance_type = tcp`), shown in `/proc/net/unix` as
/// `@rns/<instance_name>`. [`ready`] answers whether that socket is
/// *listening* and its inode is open in `/proc/<pid>/fd` of the supervised
/// rnsd: one read of `/proc/net/unix` and one directory walk of that pid's
/// fds, no Python, no allocation per line. [`shared_instance_owner`] also
/// names any other holder (a walk of every pid: the diagnostic path only).
///
/// `/proc/net/unix` is the reader's network namespace, which is rnsd's
/// (111d and its children share it). Untrusted input everywhere: the file
/// and the config are size-capped before parsing, and a line that does not
/// parse is skipped, never a panic.
///
/// The registry ([`register`], [`supervised_ready`]) lets a caller that
/// knows only the daemon root (the components runner's serving probe) ask
/// the same question of the rnsd the transport supervises for that root;
/// an unregistered root is never ready (fail closed).
pub mod owner {
    use super::PidSource;
    use std::io::Read;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    /// The kernel's table of unix sockets (the reader's netns).
    pub const PROC_NET_UNIX: &str = "/proc/net/unix";
    /// Most of `/proc/net/unix` read (bytes); a larger table is cut there.
    pub const MAX_NET_UNIX: u64 = 8 * 1024 * 1024;
    /// Most of `rns/config` read (bytes).
    pub const MAX_CONFIG: u64 = 256 * 1024;
    /// Most fds looked at per process.
    pub const MAX_FDS: usize = 65_536;
    /// `__SO_ACCEPTCON` in the Flags column: a listening socket.
    const SO_ACCEPTCON: u32 = 0x0001_0000;
    /// Longest instance name taken from the config.
    const MAX_INSTANCE: usize = 128;

    fn read_capped(path: &Path, cap: u64) -> Option<String> {
        let f = std::fs::File::open(path).ok()?;
        let mut buf = Vec::new();
        f.take(cap).read_to_end(&mut buf).ok()?;
        Some(String::from_utf8_lossy(&buf).into_owned())
    }

    /// The `[reticulum]` section of an rnsd config (ConfigObj) → the
    /// instance's socket name `@rns/<instance_name>` (`instance_name`
    /// absent: RNS's `default`). `None` when the config shares no instance
    /// (`share_instance` off) or shares it over TCP: there is no abstract
    /// socket to check, so nothing is ever ready (fail closed).
    pub fn socket_name_in(config: &str) -> Option<String> {
        let mut section = false;
        let mut name: Option<&str> = None;
        let mut shared = true;
        for raw in config.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.starts_with('[') {
                section = line.trim_matches(|c| c == '[' || c == ']').trim() == "reticulum"
                    && !line.starts_with("[[");
                continue;
            }
            if !section {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
            match k.trim() {
                "instance_name" => name = Some(v),
                "share_instance" => {
                    shared = matches!(v.to_ascii_lowercase().as_str(), "yes" | "true" | "on" | "1")
                }
                "shared_instance_type" if v.eq_ignore_ascii_case("tcp") => return None,
                _ => {}
            }
        }
        if !shared {
            return None;
        }
        let name = name.unwrap_or("default");
        let ok = !name.is_empty()
            && name.len() <= MAX_INSTANCE
            && name.bytes().all(|b| b.is_ascii_graphic());
        ok.then(|| format!("@rns/{name}"))
    }

    /// `<root>/rns/config` → the shared instance's socket name.
    pub fn socket_name(root: &Path) -> Option<String> {
        socket_name_in(&read_capped(&root.join("rns/config"), MAX_CONFIG)?)
    }

    /// The inode of the **listening** unix socket bound to `name` in the
    /// text of `/proc/net/unix` (`Num RefCount Protocol Flags Type St Inode
    /// Path`). Accepted connections carry the same path but not the
    /// listening flag; they are not the instance.
    pub fn listener_inode(net_unix: &str, name: &str) -> Option<u64> {
        for line in net_unix.lines() {
            let mut it = line.split_whitespace();
            let (Some(flags), Some(_ty), Some(st), Some(inode), Some(path)) =
                (it.nth(3), it.next(), it.next(), it.next(), it.next())
            else {
                continue;
            };
            if path != name || it.next().is_some() || st != "01" {
                continue;
            }
            let Ok(flags) = u32::from_str_radix(flags, 16) else {
                continue;
            };
            if flags & SO_ACCEPTCON == 0 {
                continue;
            }
            if let Ok(inode) = inode.parse::<u64>() {
                if inode != 0 {
                    return Some(inode);
                }
            }
        }
        None
    }

    /// Whether an fd directory (`/proc/<pid>/fd`) holds `socket:[inode]`.
    pub fn fd_dir_holds(dir: &Path, inode: u64) -> bool {
        let want = format!("socket:[{inode}]");
        let Ok(rd) = std::fs::read_dir(dir) else {
            return false;
        };
        rd.take(MAX_FDS).flatten().any(|e| {
            std::fs::read_link(e.path()).is_ok_and(|t| t.as_os_str().as_bytes() == want.as_bytes())
        })
    }

    fn fd_dir(pid: u32) -> PathBuf {
        PathBuf::from(format!("/proc/{pid}/fd"))
    }

    /// The listening inode of `root`'s shared instance now, with its name.
    fn current(root: &Path) -> Result<(String, u64), String> {
        let name = socket_name(root)
            .ok_or_else(|| "rns/config shares no instance over a local socket".to_string())?;
        let table = read_capped(Path::new(PROC_NET_UNIX), MAX_NET_UNIX)
            .ok_or_else(|| format!("{PROC_NET_UNIX} is not readable"))?;
        match listener_inode(&table, &name) {
            Some(inode) => Ok((name, inode)),
            None => Err(format!("nothing listens on {name} yet")),
        }
    }

    /// `Ok` when `root`'s shared instance listens and `rnsd` (the
    /// supervised pid; `None`: not running) holds it; otherwise why not.
    pub fn ready(root: &Path, rnsd: Option<u32>) -> Result<(), String> {
        let pid = rnsd.ok_or_else(|| "rnsd is not running".to_string())?;
        let (name, inode) = current(root)?;
        if fd_dir_holds(&fd_dir(pid), inode) {
            Ok(())
        } else {
            Err(format!(
                "{name} is not held by the supervised rnsd (pid {pid})"
            ))
        }
    }

    /// Whether `pid` holds `root`'s listening shared-instance socket.
    pub fn owned_by(root: &Path, pid: u32) -> bool {
        ready(root, Some(pid)).is_ok()
    }

    /// Who holds `root`'s listening shared-instance socket: `supervised`
    /// first (cheap), then any process (a walk of `/proc`: diagnostics
    /// only). `None`: nothing listens, or the holder is not visible.
    pub fn shared_instance_owner(root: &Path, supervised: Option<u32>) -> Option<u32> {
        let (_, inode) = current(root).ok()?;
        if let Some(pid) = supervised {
            if fd_dir_holds(&fd_dir(pid), inode) {
                return Some(pid);
            }
        }
        holder_of(inode)
    }

    /// Any process holding the socket `inode` (diagnostics).
    pub fn holder_of(inode: u64) -> Option<u32> {
        std::fs::read_dir("/proc").ok()?.flatten().find_map(|e| {
            let pid: u32 = e.file_name().to_str()?.parse().ok()?;
            fd_dir_holds(&fd_dir(pid), inode).then_some(pid)
        })
    }

    /// A process's command name (diagnostics; cleaned, capped).
    pub fn comm(pid: u32) -> String {
        let c = read_capped(Path::new(&format!("/proc/{pid}/comm")), 64).unwrap_or_default();
        crate::daemon::devices::clean(c.trim(), 32).unwrap_or_default()
    }

    // ---- registry: root → the supervised rnsd's pid

    type Entry = (PathBuf, u64, PidSource);
    static REGISTRY: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    fn registry() -> std::sync::MutexGuard<'static, Vec<Entry>> {
        REGISTRY.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Record where the supervised rnsd's pid for `root` is read (the
    /// transport does at start); replaces an earlier one. Returns the
    /// token [`unregister`] takes.
    pub fn register(root: &Path, rnsd: PidSource) -> u64 {
        let token = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut r = registry();
        r.retain(|(p, _, _)| p != root);
        r.push((root.to_path_buf(), token, rnsd));
        token
    }

    /// Drop `root`'s entry if it is still the one `token` registered.
    pub fn unregister(root: &Path, token: u64) {
        registry().retain(|(p, t, _)| !(p == root && *t == token));
    }

    /// The supervised rnsd's pid for `root` (`None`: none registered, or
    /// it is not running).
    pub fn supervised_pid(root: &Path) -> Option<u32> {
        let src = registry()
            .iter()
            .find(|(p, _, _)| p == root)
            .map(|(_, _, s)| s.clone())?;
        src()
    }

    /// [`ready`] for the rnsd registered for `root`; an unregistered root
    /// is never ready.
    pub fn supervised_ready(root: &Path) -> Result<(), String> {
        ready(root, supervised_pid(root))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::Arc;

        const TABLE: &str = "\
Num       RefCount Protocol Flags    Type St Inode Path
0000000000000000: 00000002 00000000 00010000 0001 01 41001 @rns/111/rpc
0000000000000000: 00000003 00000000 00000000 0001 03 41003 @rns/111
0000000000000000: 00000002 00000000 00010000 0001 01 41002 @rns/111
0000000000000000: 00000002 00000000 00010000 0001 01 41004 @rns/1111
0000000000000000: 00000002 00000000 00010000 0001 01  9999 /run/systemd/notify
0000000000000000: 00000003 00000000 00000000 0001 03 41005
garbage line
0000000000000000: 00000002 00000000 zzzzzzzz 0001 01 41006 @rns/odd
0000000000000000: 00000002 00000000 00010000 0001 01 41007 @rns/sp ace
";

        #[test]
        fn the_listener_is_found_by_name_and_flag() {
            // the listening one, not the accepted connection (03) with the same path,
            // not the rpc socket, not a longer name
            assert_eq!(listener_inode(TABLE, "@rns/111"), Some(41002));
            assert_eq!(listener_inode(TABLE, "@rns/111/rpc"), Some(41001));
            assert_eq!(listener_inode(TABLE, "@rns/1111"), Some(41004));
            assert_eq!(listener_inode(TABLE, "@rns/11"), None);
            assert_eq!(listener_inode(TABLE, "@rns/odd"), None, "bad flags skipped");
            assert_eq!(
                listener_inode(TABLE, "@rns/sp"),
                None,
                "a name with a space never matches"
            );
            assert_eq!(listener_inode("", "@rns/111"), None);
            // only connected sockets: nothing listens
            let only_conn = "0000000000000000: 00000003 00000000 00000000 0001 03 41003 @rns/111\n";
            assert_eq!(listener_inode(only_conn, "@rns/111"), None);
        }

        #[test]
        fn the_config_names_the_socket() {
            let c = "schema-version = 2\n\n[reticulum]\n  share_instance = Yes\n  instance_name = 111\n  rpc_key = aa\n\n[interfaces]\n  [[Auto Interface]]\n    instance_name = nope\n";
            assert_eq!(socket_name_in(c).as_deref(), Some("@rns/111"));
            assert_eq!(
                socket_name_in("[reticulum]\n  share_instance = Yes\n").as_deref(),
                Some("@rns/default")
            );
            assert_eq!(
                socket_name_in("[reticulum]\n  share_instance = No\n  instance_name = 111\n"),
                None
            );
            assert_eq!(
                socket_name_in(
                    "[reticulum]\n  instance_name = 111\n  shared_instance_type = tcp\n"
                ),
                None
            );
            assert_eq!(
                socket_name_in("[reticulum]\n  instance_name = a\u{1}b\n"),
                None
            );
            // a key outside [reticulum] does not count
            assert_eq!(
                socket_name_in("[logging]\n  instance_name = x\n[reticulum]\n").as_deref(),
                Some("@rns/default")
            );
        }

        #[test]
        fn a_fake_fd_dir_holds_the_inode() {
            let d = tempfile::tempdir().unwrap();
            let fd = d.path().join("fd");
            std::fs::create_dir(&fd).unwrap();
            std::os::unix::fs::symlink("/dev/null", fd.join("0")).unwrap();
            std::os::unix::fs::symlink("socket:[41001]", fd.join("3")).unwrap();
            std::os::unix::fs::symlink("pipe:[41002]", fd.join("4")).unwrap();
            assert!(fd_dir_holds(&fd, 41001));
            assert!(
                !fd_dir_holds(&fd, 41002),
                "a pipe of that number is not the socket"
            );
            assert!(!fd_dir_holds(&fd, 4100), "no prefix match");
            assert!(!fd_dir_holds(&d.path().join("absent"), 41001));
        }

        fn unique(tag: &str) -> String {
            use std::time::{SystemTime, UNIX_EPOCH};
            let n = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            format!("fa1-{tag}-{}-{n}", std::process::id())
        }

        fn root_for(instance: &str) -> tempfile::TempDir {
            let d = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(d.path().join("rns")).unwrap();
            std::fs::write(
                d.path().join("rns/config"),
                format!("[reticulum]\n  share_instance = Yes\n  instance_name = {instance}\n"),
            )
            .unwrap();
            d
        }

        /// The live path: this process binds the abstract socket, so it is
        /// the owner; an unregistered root and another pid are not.
        #[test]
        fn a_live_listener_is_owned_by_its_process() {
            use std::os::linux::net::SocketAddrExt;
            let inst = unique("live");
            let d = root_for(&inst);
            let me = std::process::id();
            assert!(ready(d.path(), Some(me)).is_err(), "nothing listens yet");
            let addr =
                std::os::unix::net::SocketAddr::from_abstract_name(format!("rns/{inst}")).unwrap();
            let l = std::os::unix::net::UnixListener::bind_addr(&addr).unwrap();
            assert_eq!(ready(d.path(), Some(me)), Ok(()));
            assert!(ready(d.path(), None).is_err(), "no rnsd pid: not ready");
            assert!(!owned_by(d.path(), 1), "pid 1 does not hold it");
            assert_eq!(shared_instance_owner(d.path(), Some(1)), Some(me));
            // the registry: unregistered → never ready; registered → ready
            assert!(supervised_ready(d.path()).is_err());
            let t = register(d.path(), Arc::new(move || Some(me)));
            assert_eq!(supervised_ready(d.path()), Ok(()));
            let t2 = register(d.path(), Arc::new(|| None));
            unregister(d.path(), t); // a stale token removes nothing
            assert!(
                supervised_ready(d.path()).is_err(),
                "the newer entry answers"
            );
            unregister(d.path(), t2);
            assert_eq!(supervised_pid(d.path()), None);
            drop(l);
            assert!(
                ready(d.path(), Some(me)).is_err(),
                "closed: nothing listens"
            );
        }
    }
}

// ---------------------------------------------------------------- runner

/// The production [`Runner`] for rnstatus: no shell, stdin closed, `HOME`
/// set, stdout and stderr capped at [`MAX_OUTPUT`], killed after the
/// timeout (then an error, never a partial reading).
pub struct TimedRunner {
    pub timeout: Duration,
    pub home: PathBuf,
}

impl TimedRunner {
    pub fn new(root: &Path) -> Self {
        TimedRunner {
            timeout: RNSTATUS_TIMEOUT,
            home: root.to_path_buf(),
        }
    }
}

fn drain(r: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(r) = r {
            let _ = r.take(MAX_OUTPUT as u64).read_to_end(&mut buf);
        }
        buf
    })
}

impl Runner for TimedRunner {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        use std::process::{Command, Stdio};
        let mut child = Command::new(program)
            .args(args)
            .env("HOME", &self.home)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow::anyhow!("spawn {program}: {e}"))?;
        let out = drain(child.stdout.take());
        let err = drain(child.stderr.take());
        let t0 = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if t0.elapsed() < self.timeout => {
                    std::thread::sleep(Duration::from_millis(25))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        let stdout = out.join().unwrap_or_default();
        let stderr = err.join().unwrap_or_default();
        let Some(status) = status else {
            anyhow::bail!("{program} killed after {:?}", self.timeout);
        };
        let code = match status.code() {
            Some(c) => c,
            None => {
                use std::os::unix::process::ExitStatusExt;
                -status.signal().unwrap_or(0)
            }
        };
        Ok(Output {
            status: code,
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }

    fn run_as(&self, _user: &str, _program: &str, _args: &[&str]) -> anyhow::Result<Output> {
        anyhow::bail!("rnstatus never runs as another user")
    }
}

// ---------------------------------------------------------------- cache

/// The cache's freshness (contract `TransportStatus.interfaces_state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheState {
    /// The last reading succeeded within two tick periods.
    Ok,
    /// The last good reading is older (the tick did not run).
    Stale,
    /// rnsd is not running, or the last rnstatus run failed: no interfaces.
    Unavailable,
}

/// What [`FactsCache::snapshot`] hands a reader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FactsSnapshot {
    pub state: CacheState,
    pub interfaces: Vec<InterfaceFacts>,
}

/// The tick's timing.
#[derive(Debug, Clone, Copy)]
pub struct TickConfig {
    /// Tick period (also the cadence of the log fallback of the display
    /// state, which v1 read every 30 s).
    pub period: Duration,
    /// rnstatus runs only when the facts were read this recently (demand
    /// only, B4 review F2).
    pub demand_window: Duration,
    /// Cap of the doubled delay after failures.
    pub backoff_max: Duration,
}

impl Default for TickConfig {
    fn default() -> Self {
        TickConfig {
            period: Duration::from_secs(30),
            demand_window: Duration::from_secs(600),
            backoff_max: Duration::from_secs(300),
        }
    }
}

#[derive(Debug, Default)]
struct CacheInner {
    facts: Vec<InterfaceFacts>,
    last_ok: Option<Instant>,
    last_attempt: Option<Instant>,
    last_failed: bool,
    down: bool,
    last_demand: Option<Instant>,
}

/// The rnstatus cache (module doc). Shared as `Arc<FactsCache>`.
pub struct FactsCache {
    inner: Mutex<CacheInner>,
    nudge: tokio::sync::Notify,
    cfg: TickConfig,
}

impl FactsCache {
    pub fn new(cfg: TickConfig) -> Arc<Self> {
        Arc::new(FactsCache {
            inner: Mutex::new(CacheInner {
                down: true,
                ..CacheInner::default()
            }),
            nudge: tokio::sync::Notify::new(),
            cfg,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CacheInner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn state_of(&self, i: &CacheInner) -> CacheState {
        if i.down || i.last_failed {
            return CacheState::Unavailable;
        }
        match i.last_ok {
            Some(t) if t.elapsed() <= self.cfg.period * 2 => CacheState::Ok,
            Some(_) => CacheState::Stale,
            None => CacheState::Unavailable,
        }
    }

    fn view(&self, i: &CacheInner) -> FactsSnapshot {
        let state = self.state_of(i);
        FactsSnapshot {
            state,
            interfaces: if state == CacheState::Unavailable {
                Vec::new()
            } else {
                i.facts.clone()
            },
        }
    }

    /// The facts for a reader (the route, the agent tool): marks demand and,
    /// when the cache is not fresh and the last attempt is a period old,
    /// wakes the tick. Never runs rnstatus itself.
    pub fn snapshot(&self) -> FactsSnapshot {
        let mut i = self.lock();
        i.last_demand = Some(Instant::now());
        let view = self.view(&i);
        let due = i
            .last_attempt
            .is_none_or(|t| t.elapsed() >= self.cfg.period);
        drop(i);
        if view.state != CacheState::Ok && due {
            self.nudge.notify_one();
        }
        view
    }

    /// The facts without marking demand (the display state).
    pub fn peek(&self) -> FactsSnapshot {
        let i = self.lock();
        self.view(&i)
    }

    /// Record one reading.
    pub fn record(&self, r: Result<Vec<InterfaceFacts>, RnstatusError>) {
        let mut i = self.lock();
        let now = Instant::now();
        i.last_attempt = Some(now);
        i.down = false;
        match r {
            Ok(f) => {
                i.facts = f;
                i.last_ok = Some(now);
                i.last_failed = false;
            }
            Err(_) => i.last_failed = true,
        }
    }

    /// rnsd is not running (or shares no instance): nothing to read. Stamps
    /// the attempt, so a reader does not wake the tick on every request
    /// while rnsd is down (B4 review F7: each wake reads `rnsd.log`).
    pub fn record_down(&self) {
        let mut i = self.lock();
        i.down = true;
        i.facts.clear();
        i.last_ok = None;
        i.last_attempt = Some(Instant::now());
    }

    /// The tick should run rnstatus now: a reader asked within the demand
    /// window. Demand only (B4 review F2): "no reading yet" is not demand.
    fn wanted(&self) -> bool {
        let i = self.lock();
        i.last_demand
            .is_some_and(|t| t.elapsed() <= self.cfg.demand_window)
    }

    /// From a fresh reading: any interface but the shared instance is up
    /// with a peer or client (`None`: no fresh reading, use the fallback).
    pub fn joined(&self) -> Option<bool> {
        let s = self.peek();
        (s.state == CacheState::Ok).then(|| {
            s.interfaces
                .iter()
                .any(|f| f.kind != "Shared Instance" && f.up && f.peers.unwrap_or(0) > 0)
        })
    }

    /// From a fresh reading: the shared instance is listed and up (`None`:
    /// no fresh reading).
    pub fn serving(&self) -> Option<bool> {
        let s = self.peek();
        (s.state == CacheState::Ok).then(|| {
            s.interfaces
                .iter()
                .any(|f| f.kind == "Shared Instance" && f.up)
        })
    }

    /// The tick timing.
    pub fn config(&self) -> TickConfig {
        self.cfg
    }
}

/// The tick (module doc): every period (or when nudged), `each_tick` first
/// (the display-state fallback), then, while `running()`, `rn` shares an
/// instance and a reader asked within the demand window, one rnstatus
/// reading into `cache` on a blocking thread. Ends when the task is aborted
/// (the handle's stop).
pub async fn tick<F>(
    cache: Arc<FactsCache>,
    rn: Arc<Rnstatus>,
    running: Arc<dyn Fn() -> bool + Send + Sync>,
    mut each_tick: F,
) where
    F: FnMut(&FactsCache) + Send,
{
    let cfg = cache.config();
    // the first reading one period after start (rnsd needs a moment to
    // own the shared instance)
    let mut delay = cfg.period;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = cache.nudge.notified() => {}
        }
        each_tick(&cache);
        if !running() || !rn.shares_instance() {
            // no rnsd, or no shared instance to ask (no control RPC): skip
            // the reading outright (B4 review F2)
            cache.record_down();
            delay = cfg.period;
            continue;
        }
        if !cache.wanted() {
            delay = cfg.period;
            continue;
        }
        let r = {
            let rn = Arc::clone(&rn);
            tokio::task::spawn_blocking(move || rn.read())
                .await
                .unwrap_or_else(|e| Err(RnstatusError::Spawn(e.to_string())))
        };
        let ok = r.is_ok();
        cache.record(r);
        delay = if ok {
            cfg.period
        } else {
            (delay.max(cfg.period) * 2).min(cfg.backoff_max)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("980 B"), Some(980));
        assert_eq!(parse_size("1.21 KB"), Some(1210));
        assert_eq!(parse_size("3.40 MB"), Some(3_400_000));
        assert_eq!(parse_size("0 B"), Some(0));
        assert_eq!(parse_size("12 bps"), None);
        assert_eq!(parse_size("x"), None);
        assert_eq!(size_at("↑1.21 KB  0 bps", '↑'), Some(1210));
        assert_eq!(size_at("↓980 B    12 bps", '↓'), Some(980));
        assert_eq!(size_at("Traffic   : 2.47 KB↑", '↑'), Some(2470));
        assert_eq!(size_at("  2.20 KB↓", '↓'), Some(2200));
    }

    /// B4 review F1: the old-form branch sliced at `rfind + 1`, inside a
    /// multi-byte char (the other arrow, any non-ASCII byte), and panicked.
    #[test]
    fn f1_size_at_multibyte_before_a_size() {
        // both arrows on one line (old form)
        assert_eq!(size_at("1.21 KB↑ 980 B↓", '↑'), Some(1210));
        assert_eq!(size_at("1.21 KB↑ 980 B↓", '↓'), Some(980));
        // both arrows on one line (new form)
        assert_eq!(size_at("↑1.21 KB  0 bps  ↓980 B  12 bps", '↑'), Some(1210));
        assert_eq!(size_at("↑1.21 KB  0 bps  ↓980 B  12 bps", '↓'), Some(980));
        // a non-ASCII prefix
        assert_eq!(size_at("é2.47 KB↑", '↑'), Some(2470));
        assert_eq!(size_at("Traffic €🦀5 B↓", '↓'), Some(5));
        assert_eq!(size_at("↓↓", '↓'), None);
        assert_eq!(size_at("é↑", '↑'), None);
        assert_eq!(size_at("↑é", '↑'), None);
    }

    /// xorshift64: deterministic noise without a dependency.
    struct Noise(u64);

    impl Noise {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    fn parse_all(text: &str) {
        let _ = parse_text(text);
        let _ = parse_json(text);
        let _ = detail(text);
        for line in text.lines() {
            let _ = header(line);
            let _ = parse_size(line);
            let _ = leading_int(line);
            for arrow in ['↑', '↓'] {
                let _ = size_at(line, arrow);
            }
        }
    }

    /// B4 review F1: the parser is fail-closed on any input — random bytes
    /// and arrow / non-ASCII / layout mixes never panic.
    #[test]
    fn f1_parser_never_panics_on_noise() {
        const PIECES: &[&str] = &[
            "↑",
            "↓",
            "é",
            "€",
            "🦀",
            "\u{202e}",
            "\u{0}",
            "\t",
            " ",
            "  ",
            ".",
            "..",
            "0",
            "1",
            "9",
            "1.21",
            "980",
            " B",
            " KB",
            "MB",
            " bps",
            "B",
            ":",
            "[",
            "]",
            "/",
            "\n",
            " Shared Instance[rns/111]\n",
            " RNodeInterface[Roof",
            "    Status    : Up\n",
            "    Traffic   : ",
            "                ",
            "    Peers     : ",
            "    Clients   : ",
            "{\"interfaces\": [",
            "{\"name\": \"x\"}",
            "]}",
        ];
        let mut n = Noise(0x9E37_79B9_7F4A_7C15);
        for _ in 0..400 {
            let len = n.below(240);
            let bytes: Vec<u8> = (0..len)
                .map(|_| match n.below(4) {
                    // bias towards the arrows' and other UTF-8 lead/tail bytes
                    0 => [0xE2, 0x86, 0x91, 0x93, 0xC3, 0xA9, 0xF0, 0x9F][n.below(8)],
                    _ => (n.next() & 0xFF) as u8,
                })
                .collect();
            parse_all(&String::from_utf8_lossy(&bytes));
        }
        for _ in 0..400 {
            let parts = n.below(60);
            let text: String = (0..parts).map(|_| PIECES[n.below(PIECES.len())]).collect();
            parse_all(&text);
        }
    }

    #[test]
    fn headers() {
        assert_eq!(
            header(" AutoInterface[Auto Interface]"),
            Some(("AutoInterface".into(), "Auto Interface".into()))
        );
        assert_eq!(
            header(" BackboneInterface[111 LAN Backbone/0.0.0.0:4242]"),
            Some(("BackboneInterface".into(), "111 LAN Backbone".into()))
        );
        assert_eq!(
            header(" Shared Instance[rns/111]"),
            Some(("Shared Instance".into(), "rns/111".into()))
        );
        assert_eq!(header("    Status    : Up"), None);
        assert_eq!(header(" Transport Instance <ab12> running"), None);
        assert_eq!(header(" [x]"), None);
    }

    #[test]
    fn cache_states() {
        let c = FactsCache::new(TickConfig {
            period: Duration::from_millis(50),
            ..TickConfig::default()
        });
        assert_eq!(c.peek().state, CacheState::Unavailable);
        assert!(!c.wanted(), "F2: nothing read yet is not demand");
        c.record(Ok(vec![InterfaceFacts {
            name: "Auto Interface".into(),
            kind: "AutoInterface".into(),
            up: true,
            rx: 1,
            tx: 2,
            peers: Some(1),
        }]));
        assert_eq!(c.peek().state, CacheState::Ok);
        assert_eq!(c.joined(), Some(true));
        assert_eq!(c.serving(), Some(false));
        assert!(!c.wanted(), "no demand yet");
        let _ = c.snapshot();
        assert!(c.wanted(), "a reader asked");
        std::thread::sleep(Duration::from_millis(120));
        let s = c.peek();
        assert_eq!(s.state, CacheState::Stale);
        assert_eq!(s.interfaces.len(), 1, "stale keeps the last good reading");
        c.record(Err(RnstatusError::NoInstance));
        let s = c.peek();
        assert_eq!(s.state, CacheState::Unavailable);
        assert!(s.interfaces.is_empty());
        c.record_down();
        assert_eq!(c.peek().state, CacheState::Unavailable);
        assert_eq!(c.joined(), None);
    }

    /// B4 review F2: demand is a reader within the window (10 min by
    /// default); without one the tick starts no rnstatus, whatever the
    /// cache holds.
    #[test]
    fn f2_wanted_is_demand_only() {
        let c = FactsCache::new(TickConfig::default());
        assert!(!c.wanted(), "no reading and no reader: not wanted");
        c.record(Err(RnstatusError::NoInstance));
        assert!(!c.wanted(), "a failed reading is not demand either");
        let _ = c.snapshot();
        assert!(c.wanted(), "a reader just asked");
        // the reader was 9 min ago: still wanted; 10 min 1 s ago: not
        let ago = |d: Duration| Instant::now().checked_sub(d);
        if let Some(t) = ago(Duration::from_secs(9 * 60)) {
            c.lock().last_demand = Some(t);
            assert!(c.wanted());
        }
        if let Some(t) = ago(Duration::from_secs(601)) {
            c.lock().last_demand = Some(t);
            assert!(!c.wanted(), "10 min without a read: stops");
        }
    }

    /// B4 review F7: while rnsd is down, a reader wakes the tick at most once
    /// a period (record_down stamps the attempt), not on every request.
    #[tokio::test]
    async fn f7_snapshot_does_not_nudge_on_every_call_while_down() {
        let c = FactsCache::new(TickConfig::default());
        // a fresh cache: the first reader wakes the tick
        let _ = c.snapshot();
        let woke = tokio::time::timeout(Duration::from_millis(50), c.nudge.notified()).await;
        assert!(woke.is_ok(), "the first reader wakes the tick");
        // the tick found rnsd down
        c.record_down();
        for _ in 0..5 {
            let s = c.snapshot();
            assert_eq!(s.state, CacheState::Unavailable);
        }
        let woke = tokio::time::timeout(Duration::from_millis(50), c.nudge.notified()).await;
        assert!(woke.is_err(), "no wake within the period after record_down");
    }
}
