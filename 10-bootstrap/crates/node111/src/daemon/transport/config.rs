//! rnsd config, **renderer v2** (`os/60-transport/README.md` §11 "Renderer
//! v2", §6 "rnsd posture v2"; carriers M1): `<root>/rns/config` rendered
//! from the parsed `state/interfaces.toml` ([`render`]), never by hand.
//!
//! - **Head.** `assets/rns/config.tmpl` keeps only the fixed comment head and
//!   `schema-version` first; [`SCHEMA_VERSION`] is 2. A v1 binary refuses to
//!   regenerate a file whose version is above its own (`mod.rs`, the v1
//!   takeover check), so rollback leaves a v2 file alone; v2 regenerates
//!   over a v1 file ([`should_regenerate`]).
//! - **`[reticulum]`.** `share_instance` (default Yes), `instance_name`
//!   (default `111`), `rpc_key` (hex of `secrets/rns-rpc-key`, 32 bytes from
//!   the OS CSPRNG, minted when absent; since RNS 1.5.5 it authorises
//!   `rnstatus --attach|--detach` over the control RPC), `enable_transport`
//!   per `[posture]` (default: true on Cat 3/4).
//! - **`[interfaces]`.** The `111.network` AutoInterface (kept from v1);
//!   while LAN sharing is on, the `[posture.lan]` items: a
//!   `BackboneInterface` listener on 0.0.0.0:4242 when `backbone.listen`
//!   (IFAC name [`IFAC_NAME`] and the passphrase of `secrets/ifac-passphrase`
//!   when `backbone.ifac`), the stock AutoInterface (group `reticulum`) and
//!   the propagation-node marker when opted in; then one stanza per `owner =
//!   "rnsd"` entry in file order: `RNodeInterface` with the region's
//!   frequency plan applied (in band, power under the limit, the duty cycle
//!   as `airtime_limit_long`) and `TCPClientInterface` (the phone-relay
//!   client). An entry that cannot be rendered (no region, a missing
//!   parameter, out of band, a missing IFAC secret, a port claim refused) is
//!   **left out** with a reason ([`Rendered::left_out`]); nothing is guessed.
//! - **Modes (§11, G4).** `rns/config` 0600, `rns/` 0700, `secrets/` 0700
//!   with 0600 files, and the LXMF helper's client dir `bridge/lxmf/rns`
//!   0700 with its own 0600 config ([`render_client_config`]:
//!   `share_instance = Yes`, `instance_name`, `rpc_key`, no interfaces).
//! - **No rewrite** when the content is unchanged ([`write_rendered`]), so a
//!   re-render never restarts or reloads anything by itself.

use super::interfaces::{
    self, missing_param, region, IfKind, IfOwner, Interface, InterfacesFile, Radio, AUTO_NAME,
    BACKBONE_NAME, RNODE_PARAMS, STOCK_AUTO_NAME,
};
use crate::daemon::DaemonLayout;
use crate::install::categorize::Category;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};

const TEMPLATE: &str = include_str!("../../../assets/rns/config.tmpl");

/// The rendered file's `schema-version` (§11: 1 → 2).
pub const SCHEMA_VERSION: u32 = 2;
/// Read cap of an existing config (the rendered file is a few KB).
pub const MAX_CONFIG_BYTES: u64 = 256 * 1024;
/// Read cap of one secret file.
const MAX_SECRET_BYTES: u64 = 256;
/// The IFAC network name of the hub's Backbone listener (the passphrase is
/// the secret; the name only scopes it).
pub const IFAC_NAME: &str = "111-lan";
/// `<root>/secrets` (§2 "State": 0700, 0600 files, never on any API).
pub const SECRETS_DIR: &str = "secrets";
/// The shared-instance control key.
pub const RPC_KEY_FILE: &str = "rns-rpc-key";
/// The hub's Backbone IFAC passphrase (minted when `backbone.ifac` is on).
pub const IFAC_FILE: &str = "ifac-passphrase";
/// Prefix of a phone-relay client's IFAC passphrase file (+ hex of the
/// interface name; written by `join::apply_join`).
pub const CLIENT_IFAC_PREFIX: &str = "ifac-client-";
/// The `111.network` AutoInterface group (v1).
pub const AUTO_GROUP: &str = "111.network";

pub fn config_dir(layout: &DaemonLayout) -> PathBuf {
    layout.root.join("rns")
}

pub fn config_path(layout: &DaemonLayout) -> PathBuf {
    config_dir(layout).join("config")
}

/// The LXMF helper's client config dir (`bridge/lxmf/rns`, §2, §6).
pub fn client_dir(root: &Path) -> PathBuf {
    root.join("bridge/lxmf/rns")
}

/// The LXMF helper's client config.
pub fn client_config_path(root: &Path) -> PathBuf {
    client_dir(root).join("config")
}

/// `<root>/secrets`.
pub fn secrets_dir(root: &Path) -> PathBuf {
    root.join(SECRETS_DIR)
}

/// The file holding a phone-relay client's IFAC passphrase for the
/// interface named `name` (hex of the name: unique for unique names).
pub fn client_ifac_path(root: &Path, name: &str) -> PathBuf {
    let hex: String = name.bytes().map(|b| format!("{b:02x}")).collect();
    secrets_dir(root).join(format!("{CLIENT_IFAC_PREFIX}{hex}"))
}

// ---------------------------------------------------------------- secrets

/// The minimal vault (§2): never seed-derived, never on any API, never
/// logged (`Debug` is redacted).
#[derive(Clone, PartialEq, Eq)]
pub struct Secrets {
    rpc_key: [u8; 32],
    ifac_passphrase: Option<String>,
    /// Phone-relay client passphrases by interface name.
    client_ifac: BTreeMap<String, String>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secrets")
            .field("rpc_key", &"<redacted>")
            .field(
                "ifac_passphrase",
                &self.ifac_passphrase.as_ref().map(|_| "<redacted>"),
            )
            .field("client_ifac", &self.client_ifac.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// An IFAC passphrase as 111 mints and accepts it: 16-128 ASCII letters and
/// digits (nothing rnsd's ConfigObj reads as syntax).
pub fn valid_passphrase(s: &str) -> bool {
    (16..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let txt = std::str::from_utf8(chunk).ok()?;
        out[i] = u8::from_str_radix(txt, 16).ok()?;
    }
    Some(out)
}

/// Read a secret file: `O_NOFOLLOW`, regular, ≤ 256 bytes, mode repaired
/// to 0600. `Ok(None)` when absent.
fn read_secret(p: &Path) -> anyhow::Result<Option<String>> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let f = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(p)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => anyhow::bail!("{}: {e}", p.display()),
    };
    let meta = f.metadata()?;
    anyhow::ensure!(meta.is_file(), "{} is not a regular file", p.display());
    anyhow::ensure!(
        meta.len() <= MAX_SECRET_BYTES,
        "{} is too large",
        p.display()
    );
    if meta.permissions().mode() & 0o777 != 0o600 {
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    let mut buf = Vec::new();
    f.take(MAX_SECRET_BYTES).read_to_end(&mut buf)?;
    Ok(Some(String::from_utf8_lossy(&buf).trim().to_string()))
}

fn write_secret(p: &Path, text: &str) -> anyhow::Result<()> {
    crate::util::fs::atomic_write(p, format!("{text}\n").as_bytes(), 0o600)
}

fn mint(n: usize) -> anyhow::Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf).map_err(|e| anyhow::anyhow!("getrandom: {e}"))?;
    Ok(buf)
}

impl Secrets {
    /// Fixed secrets (tests, golden renders).
    pub fn new(rpc_key: [u8; 32]) -> Self {
        Secrets {
            rpc_key,
            ifac_passphrase: None,
            client_ifac: BTreeMap::new(),
        }
    }

    /// With the hub's Backbone IFAC passphrase.
    pub fn with_ifac(mut self, passphrase: &str) -> Self {
        self.ifac_passphrase = Some(passphrase.to_string());
        self
    }

    /// With a phone-relay client's passphrase for interface `name`.
    pub fn with_client_ifac(mut self, name: &str, passphrase: &str) -> Self {
        self.client_ifac
            .insert(name.to_string(), passphrase.to_string());
        self
    }

    /// `rpc_key` as rendered (64 lowercase hex).
    pub fn rpc_key_hex(&self) -> String {
        hex(&self.rpc_key)
    }

    /// The hub's IFAC passphrase, when one exists.
    pub fn ifac_passphrase(&self) -> Option<&str> {
        self.ifac_passphrase.as_deref()
    }

    /// The client passphrase for interface `name`.
    pub fn client_ifac(&self, name: &str) -> Option<&str> {
        self.client_ifac.get(name).map(String::as_str)
    }

    /// Load `secrets/` under `root`, minting what is absent: the rpc key
    /// always, the IFAC passphrase when `want_ifac`. `secrets/` is 0700, every
    /// file 0600. An unreadable or malformed rpc key or hub passphrase is
    /// replaced by a fresh one (logged: every consumer is rendered by this
    /// module from the same value, so a re-mint only means one rnsd
    /// restart); a malformed client passphrase is left out (its stanza is not
    /// rendered). `clients` names the TCPClient entries with `ifac` on.
    pub fn load_or_mint(root: &Path, want_ifac: bool, clients: &[&str]) -> anyhow::Result<Secrets> {
        let dir = secrets_dir(root);
        interfaces::ensure_dir(&dir, true)?;
        let key_path = dir.join(RPC_KEY_FILE);
        let rpc_key = match read_secret(&key_path)
            .ok()
            .flatten()
            .as_deref()
            .and_then(unhex32)
        {
            Some(k) => k,
            None => {
                if key_path.exists() {
                    eprintln!(
                        "111 transport: {} is not a 32-byte hex key; minting a new one",
                        key_path.display()
                    );
                }
                let mut k = [0u8; 32];
                k.copy_from_slice(&mint(32)?);
                write_secret(&key_path, &hex(&k))?;
                k
            }
        };
        let ifac_path = dir.join(IFAC_FILE);
        let ifac_passphrase = match read_secret(&ifac_path).ok().flatten() {
            Some(p) if valid_passphrase(&p) => Some(p),
            other if want_ifac => {
                if other.is_some() {
                    eprintln!(
                        "111 transport: {} is malformed; minting a new IFAC passphrase",
                        ifac_path.display()
                    );
                }
                let p = hex(&mint(16)?);
                write_secret(&ifac_path, &p)?;
                Some(p)
            }
            _ => None,
        };
        let mut client_ifac = BTreeMap::new();
        for name in clients {
            if let Some(p) = read_secret(&client_ifac_path(root, name))
                .ok()
                .flatten()
                .filter(|p| valid_passphrase(p))
            {
                client_ifac.insert(name.to_string(), p);
            }
        }
        Ok(Secrets {
            rpc_key,
            ifac_passphrase,
            client_ifac,
        })
    }

    /// [`Secrets::load_or_mint`] for what `ifs` needs.
    pub fn for_file(root: &Path, ifs: &InterfacesFile) -> anyhow::Result<Secrets> {
        let clients: Vec<&str> = ifs
            .interfaces
            .iter()
            .filter(|i| i.kind == IfKind::TcpClient && i.flag("ifac"))
            .map(|i| i.name.as_str())
            .collect();
        Self::load_or_mint(root, ifs.posture.lan.backbone.ifac, &clients)
    }
}

/// Store a phone-relay client's IFAC passphrase (0600 under `secrets/`).
pub fn write_client_ifac(root: &Path, name: &str, passphrase: &str) -> anyhow::Result<()> {
    anyhow::ensure!(valid_passphrase(passphrase), "IFAC passphrase refused");
    interfaces::ensure_dir(&secrets_dir(root), true)?;
    write_secret(&client_ifac_path(root, name), passphrase)
}

// ---------------------------------------------------------------- render

/// A stanza that was not rendered, and why (one plain sentence).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    pub name: String,
    pub reason: String,
}

/// The rendered config and what it left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub text: String,
    pub left_out: Vec<LeftOut>,
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "Yes"
    } else {
        "No"
    }
}

fn fmt_value(v: &toml::Value) -> String {
    match v {
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(x) => fmt_num(*x),
        toml::Value::Boolean(b) => if *b { "yes" } else { "no" }.into(),
        toml::Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

fn fmt_num(x: f64) -> String {
    if x.fract() == 0.0 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

fn stanza(out: &mut String, name: &str, pairs: &[(&str, String)]) {
    let _ = writeln!(out, "  [[{name}]]");
    for (k, v) in pairs {
        let _ = writeln!(out, "    {k} = {v}");
    }
}

fn number(v: &toml::Value) -> Option<f64> {
    match v {
        toml::Value::Integer(i) => Some(*i as f64),
        toml::Value::Float(x) => Some(*x),
        _ => None,
    }
}

/// An RNode stanza's pairs with the region's frequency plan applied, or why
/// it is left out.
fn rnode_pairs(i: &Interface, radio: &Radio) -> Result<Vec<(&'static str, String)>, String> {
    let Some(region_id) = i.effective_region(radio) else {
        return Err(
            "no radio region is set; a human sets it first (transmit legality is a per-node fact)"
                .into(),
        );
    };
    let plan =
        region(region_id).ok_or_else(|| format!("region {region_id} is not in the table"))?;
    if let Some(key) = missing_param(i) {
        return Err(format!("parameter {key} is missing"));
    }
    let port = i.port.clone().ok_or("no port")?;
    let freq = i.int("frequency").unwrap_or(0);
    let bw = i.int("bandwidth").unwrap_or(0);
    let lo = freq - bw / 2;
    let hi = freq + bw / 2;
    if lo < plan.start_hz as i64 || hi > plan.end_hz as i64 {
        return Err(format!(
            "frequency {freq} Hz with bandwidth {bw} Hz is outside {} ({}-{} Hz)",
            plan.id, plan.start_hz, plan.end_hz
        ));
    }
    let tx = i.int("txpower").unwrap_or(0);
    if tx > i64::from(plan.power_limit_dbm) {
        return Err(format!(
            "txpower {tx} dBm is above the {} limit of {} dBm",
            plan.id, plan.power_limit_dbm
        ));
    }
    let mut pairs: Vec<(&'static str, String)> = vec![
        ("type", "RNodeInterface".into()),
        ("enabled", "yes".into()),
        ("port", port),
    ];
    let duty = f64::from(plan.duty_cycle_pct);
    for spec in RNODE_PARAMS {
        let given = i.params.get(spec.key);
        if spec.key == "airtime_limit_long" && plan.duty_cycle_pct < 100 {
            let v = given.and_then(number).map_or(duty, |g| g.min(duty));
            pairs.push((spec.key, fmt_num(v)));
            continue;
        }
        if let Some(v) = given {
            pairs.push((spec.key, fmt_value(v)));
        }
    }
    Ok(pairs)
}

/// A phone-relay client stanza's pairs, or why it is left out.
fn tcp_client_pairs(
    i: &Interface,
    secrets: &Secrets,
) -> Result<Vec<(&'static str, String)>, String> {
    if let Some(key) = missing_param(i) {
        return Err(format!("parameter {key} is missing"));
    }
    let mut pairs: Vec<(&'static str, String)> = vec![
        ("type", "TCPClientInterface".into()),
        ("enabled", "yes".into()),
        ("target_host", i.text("target_host").unwrap_or("").into()),
        ("target_port", i.int("target_port").unwrap_or(0).to_string()),
    ];
    if i.flag("ifac") {
        let pass = secrets
            .client_ifac(&i.name)
            .ok_or("its IFAC passphrase is missing from secrets/ (join the hub again)")?;
        pairs.push((
            "network_name",
            i.text("network_name").unwrap_or(IFAC_NAME).into(),
        ));
        pairs.push(("passphrase", pass.into()));
    }
    Ok(pairs)
}

/// Render rnsd's config from the parsed `interfaces.toml` (module doc).
/// Deterministic for the same inputs.
pub fn render(
    ifs: &InterfacesFile,
    secrets: &Secrets,
    lan_sharing: bool,
    category: Option<Category>,
) -> String {
    render_with(ifs, secrets, lan_sharing, category, &[]).text
}

/// [`render`] and the stanzas it left out; `exclude` leaves out more (a
/// refused port claim, §12), with the reason given.
pub fn render_with(
    ifs: &InterfacesFile,
    secrets: &Secrets,
    lan_sharing: bool,
    category: Option<Category>,
    exclude: &[LeftOut],
) -> Rendered {
    let mut out = TEMPLATE.replace("@@SCHEMA_VERSION@@", &SCHEMA_VERSION.to_string());
    let mut left_out = Vec::new();
    let p = &ifs.posture;
    let _ = write!(
        out,
        "\n[reticulum]\n  share_instance = {}\n  instance_name = {}\n  rpc_key = {}\n  \
         enable_transport = {}\n",
        yes_no(p.share_instance),
        p.instance_name,
        secrets.rpc_key_hex(),
        yes_no(p.enable_transport_for(category)),
    );
    out.push_str("\n[interfaces]\n");
    stanza(
        &mut out,
        AUTO_NAME,
        &[
            ("type", "AutoInterface".into()),
            ("enabled", "yes".into()),
            ("group_id", AUTO_GROUP.into()),
        ],
    );
    if lan_sharing {
        let lan = &p.lan;
        if lan.backbone.listen {
            let mut pairs: Vec<(&str, String)> = vec![
                ("type", "BackboneInterface".into()),
                ("enabled", "yes".into()),
                ("listen_on", "0.0.0.0".into()),
                ("port", lan.backbone.port.to_string()),
            ];
            let ok = if lan.backbone.ifac {
                match secrets.ifac_passphrase() {
                    Some(pass) => {
                        pairs.push(("network_name", IFAC_NAME.into()));
                        pairs.push(("passphrase", pass.into()));
                        true
                    }
                    None => {
                        left_out.push(LeftOut {
                            name: BACKBONE_NAME.into(),
                            reason: "IFAC is on but no passphrase is in secrets/; the listener \
                                     stays closed"
                                .into(),
                        });
                        false
                    }
                }
            } else {
                true
            };
            if ok {
                stanza(&mut out, BACKBONE_NAME, &pairs);
            }
        }
        if lan.stock_autointerface {
            stanza(
                &mut out,
                STOCK_AUTO_NAME,
                &[("type", "AutoInterface".into()), ("enabled", "yes".into())],
            );
        }
        if lan.propagation_node {
            out.push_str(
                "  # propagation node: opted in (an LXMF router role the helper carries; no \
                 rnsd key)\n",
            );
        }
    }
    for i in ifs.interfaces.iter().filter(|i| i.owner == IfOwner::Rnsd) {
        if let Some(x) = exclude
            .iter()
            .find(|x| x.name.eq_ignore_ascii_case(&i.name))
        {
            left_out.push(x.clone());
            continue;
        }
        let pairs = match i.kind {
            IfKind::RNode => rnode_pairs(i, &ifs.radio),
            IfKind::TcpClient => tcp_client_pairs(i, secrets),
            IfKind::Meshtastic => continue,
        };
        match pairs {
            Ok(pairs) => stanza(&mut out, &i.name, &pairs),
            Err(reason) => left_out.push(LeftOut {
                name: i.name.clone(),
                reason,
            }),
        }
    }
    Rendered {
        text: out,
        left_out,
    }
}

/// The LXMF helper's client config (§6 (1), §11 modes): a shared-instance
/// client of 111d's rnsd with no interfaces of its own.
pub fn render_client_config(secrets: &Secrets, instance_name: &str) -> String {
    format!(
        "# The LXMF helper's Reticulum client config (os/60-transport/README.md §6, §11), rendered\n\
         # by 111d. A shared-instance client with no interfaces: it attaches to 111d's rnsd and\n\
         # carries no traffic of its own. 0600: it carries the rpc_key.\n\
         schema-version = {SCHEMA_VERSION}\n\n\
         [reticulum]\n  share_instance = Yes\n  instance_name = {instance_name}\n  rpc_key = {}\n  \
         enable_transport = No\n\n[interfaces]\n",
        secrets.rpc_key_hex()
    )
}

/// Write the helper's client config: `bridge/lxmf` and `bridge/lxmf/rns`
/// 0700, `config` 0600; true when the content changed. Called by the
/// component runner on enable (S4/C3), never at daemon start (a disabled
/// component leaves nothing on disk, §2.1 rule 2).
pub fn write_client_config(
    root: &Path,
    secrets: &Secrets,
    instance_name: &str,
) -> anyhow::Result<bool> {
    let dir = client_dir(root);
    if let Some(parent) = dir.parent() {
        interfaces::ensure_dir(parent, true)?;
    }
    interfaces::ensure_dir(&dir, true)?;
    write_file_if_changed(
        &client_config_path(root),
        &render_client_config(secrets, instance_name),
    )
}

fn read_capped(p: &Path) -> Option<String> {
    let f = std::fs::File::open(p).ok()?;
    let mut buf = Vec::new();
    f.take(MAX_CONFIG_BYTES).read_to_end(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

fn write_file_if_changed(path: &Path, content: &str) -> anyhow::Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    if read_capped(path).is_some_and(|c| c == content) {
        // same bytes: no rewrite (no reload), but the mode is still 0600
        if let Ok(m) = std::fs::symlink_metadata(path) {
            if m.is_file() && m.permissions().mode() & 0o777 != 0o600 {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            }
        }
        return Ok(false);
    }
    crate::util::fs::atomic_write(path, content.as_bytes(), 0o600)?;
    Ok(true)
}

/// Write `content` to `rns/config` (dir 0700, file 0600, atomic). Returns
/// false when the file already had exactly this content.
pub fn write_rendered(layout: &DaemonLayout, content: &str) -> anyhow::Result<bool> {
    interfaces::ensure_dir(&config_dir(layout), true)?;
    write_file_if_changed(&config_path(layout), content)
}

/// The text of the current `rns/config` (`None`: absent or unreadable).
pub fn current_text(layout: &DaemonLayout) -> Option<String> {
    read_capped(&config_path(layout))
}

/// This binary may (re)write `rns/config`: no file, or one of this or an
/// older schema-version (a newer one belongs to a newer binary).
pub fn should_regenerate(layout: &DaemonLayout) -> bool {
    !matches!(existing_schema_version(layout), Some(v) if v > SCHEMA_VERSION)
}

/// The node category recorded at the last re-probe (`state/facts.json`).
pub fn category_of(layout: &DaemonLayout) -> Option<Category> {
    let f = std::fs::File::open(layout.state_dir().join("facts.json")).ok()?;
    let mut buf = Vec::new();
    f.take(1024 * 1024).read_to_end(&mut buf).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&buf).ok()?;
    serde_json::from_value(v.get("category")?.clone()).ok()
}

/// LAN sharing as configured (the env-file override, else `settings.json`).
pub fn lan_sharing(layout: &DaemonLayout) -> bool {
    let env = crate::daemon::load_env(&layout.etc).unwrap_or_default();
    crate::daemon::dashboard::bind::lan_configured(&env, &layout.state_dir())
}

/// Render from what is on disk and write it when changed: the file under
/// `state/` (read-only; absent or unreadable ⇒ the defaults), the secrets
/// (minted when absent), LAN sharing and the category. The daemon's spine
/// renders from its in-memory file instead (`posture.rs`).
pub fn write_if_changed(layout: &DaemonLayout) -> anyhow::Result<bool> {
    let ifs = interfaces::read(&interfaces::path(&layout.state_dir())).unwrap_or_default();
    let secrets = Secrets::for_file(&layout.root, &ifs)?;
    let text = render(&ifs, &secrets, lan_sharing(layout), category_of(layout));
    write_rendered(layout, &text)
}

/// Read the schema-version from an existing config (the takeover seam: a
/// bumped version means someone newer owns the file — stop regenerating).
pub fn existing_schema_version(layout: &DaemonLayout) -> Option<u32> {
    let text = read_capped(&config_path(layout))?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("schema-version") {
            let rest = rest.trim_start_matches(['=', ' ']);
            if let Ok(v) = rest.parse::<u32>() {
                return Some(v);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(root: &Path) -> DaemonLayout {
        DaemonLayout {
            root: root.to_path_buf(),
            etc: root.join("etc"),
        }
    }

    /// The v1 binary's rule (`mod.rs` v1: regenerate unless `Some(v) if v > 1`).
    fn v1_regenerates(v: Option<u32>) -> bool {
        !matches!(v, Some(v) if v > 1)
    }

    #[test]
    fn v1_leaves_v2_alone_and_v2_regenerates_v1() {
        let dir = tempfile::tempdir().unwrap();
        let l = layout(dir.path());
        assert!(write_if_changed(&l).unwrap());
        assert_eq!(existing_schema_version(&l), Some(SCHEMA_VERSION));
        assert!(
            !v1_regenerates(existing_schema_version(&l)),
            "v1 leaves a v2 file alone"
        );
        assert!(!write_if_changed(&l).unwrap(), "no rewrite when unchanged");
        // a v1 file is regenerated by v2
        std::fs::write(config_path(&l), "schema-version = 1\n[reticulum]\n").unwrap();
        assert!(should_regenerate(&l));
        assert!(write_if_changed(&l).unwrap());
        assert_eq!(existing_schema_version(&l), Some(2));
        // a newer one is not
        std::fs::write(config_path(&l), "schema-version = 3\n").unwrap();
        assert!(!should_regenerate(&l));
    }

    #[test]
    fn secrets_mint_once_with_modes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let a = Secrets::load_or_mint(dir.path(), false, &[]).unwrap();
        assert!(a.ifac_passphrase().is_none());
        let b = Secrets::load_or_mint(dir.path(), true, &[]).unwrap();
        assert_eq!(a.rpc_key_hex(), b.rpc_key_hex(), "minted once");
        let pass = b.ifac_passphrase().unwrap().to_string();
        assert!(valid_passphrase(&pass));
        let c = Secrets::load_or_mint(dir.path(), true, &[]).unwrap();
        assert_eq!(c.ifac_passphrase(), Some(pass.as_str()));
        let mode = |p: &Path| p.metadata().unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&secrets_dir(dir.path())), 0o700);
        assert_eq!(mode(&secrets_dir(dir.path()).join(RPC_KEY_FILE)), 0o600);
        assert_eq!(mode(&secrets_dir(dir.path()).join(IFAC_FILE)), 0o600);
        assert!(!format!("{c:?}").contains(&pass), "Debug is redacted");
        // a malformed key is replaced
        std::fs::write(secrets_dir(dir.path()).join(RPC_KEY_FILE), "zz").unwrap();
        let d = Secrets::load_or_mint(dir.path(), false, &[]).unwrap();
        assert_ne!(d.rpc_key_hex(), a.rpc_key_hex());
    }
}
