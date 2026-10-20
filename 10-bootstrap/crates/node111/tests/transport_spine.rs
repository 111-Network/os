//! Layer 6 B4 acceptance: the transport spine (`os/60-transport/README.md`
//! §11; §16 G4 first cut). `state/interfaces.toml` (load, save, 0600,
//! `deny_unknown_fields`, moved aside, newer left alone), validation,
//! migration from `devices.json` + `rns/config` with the divergence line,
//! golden renders per posture and stanza set (`tests/fixtures/rns/*.golden`,
//! byte-exact; `UPDATE_GOLDEN=1` rewrites them), modes, the v1/v2 takeover
//! seam, rnstatus parser goldens (`tests/fixtures/rnstatus/*`), the argv of
//! attach/detach/reload, the facts tick, the posture apply path (stanza-only
//! ⇒ `--reload`/`--attach`/`--detach`, `[reticulum]` ⇒ restart with the
//! dependants in order, a real `ChildControl` restart), the join descriptor
//! round trip and the `PortTable` claims.
//!
//! No test starts Python or imports Reticulum: rnstatus is a fake `Runner`.
//! `SPINE_KEEP_ROOT=<dir>` makes `modes_on_a_scratch_root` render into that
//! dir and leave it for `60-transport/verify/spine.sh`'s independent check.

use node111::daemon::devices::store::{self, MeshtasticBinding, Store};
use node111::daemon::meshtastic::meshmud::Uplink;
use node111::daemon::ports::PortTable;
use node111::daemon::supervise::{spawn_child, BackoffPolicy, ChildSpec, ChildState};
use node111::daemon::transport::config::{self, Secrets};
use node111::daemon::transport::interfaces::{
    self, IfKind, IfOwner, Interface, InterfacesFile, LoadError,
};
use node111::daemon::transport::join::{self, JoinError, Session};
use node111::daemon::transport::posture::{BoxFut, RestartHooks, RnsdChild, Spine};
use node111::daemon::transport::rnstatus::{
    self, CacheState, FactsCache, InterfaceFacts, Rnstatus, RnstatusError, TickConfig,
};
use node111::daemon::DaemonLayout;
use node111::install::categorize::Category;
use node111::util::run::{Output, Runner};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---------------------------------------------------------------- helpers

fn layout_at(root: &Path) -> DaemonLayout {
    DaemonLayout {
        root: root.to_path_buf(),
        etc: root.join("etc"),
    }
}

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel)
}

fn golden(name: &str, text: &str) {
    let p = fixture(&format!("rns/{name}.golden"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&p, text).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&p)
        .unwrap_or_else(|_| panic!("missing {} (UPDATE_GOLDEN=1 writes it)", p.display()));
    assert!(
        text == want,
        "golden {name} differs (UPDATE_GOLDEN=1 rewrites it)\n--- want\n{want}\n--- got\n{text}"
    );
}

fn out(status: i32, stdout: &str) -> Output {
    Output {
        status,
        stdout: stdout.to_string(),
        stderr: String::new(),
    }
}

/// Fixed secrets for byte-exact goldens.
fn secrets() -> Secrets {
    Secrets::new([0x11; 32])
        .with_ifac("0123456789abcdef0123456789abcdef")
        .with_client_ifac("111 hub 10.0.0.1", "fedcba9876543210fedcba9876543210")
}

fn params(pairs: &[(&str, toml::Value)]) -> toml::Table {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn int(i: i64) -> toml::Value {
    toml::Value::Integer(i)
}

fn rnode(name: &str, port: &str, region: Option<&str>, freq: i64) -> Interface {
    Interface {
        name: name.into(),
        owner: IfOwner::Rnsd,
        kind: IfKind::RNode,
        device: None,
        port: Some(port.into()),
        region: region.map(str::to_string),
        uplink: None,
        params: params(&[
            ("frequency", int(freq)),
            ("bandwidth", int(125_000)),
            ("txpower", int(14)),
            ("spreadingfactor", int(8)),
            ("codingrate", int(5)),
        ]),
    }
}

fn binding(path: &str, num: u32) -> MeshtasticBinding {
    MeshtasticBinding::new(Uplink::serial(path).unwrap(), num, Some("HELTEC_V3"), None).unwrap()
}

/// Records every rnstatus argv (after `--config <dir>`) into a shared log;
/// a reading answers `read_out`.
#[derive(Clone)]
struct Rec {
    log: Arc<Mutex<Vec<String>>>,
    argv: Arc<Mutex<Vec<Vec<String>>>>,
    read_out: Arc<Mutex<Output>>,
}

impl Rec {
    fn new(log: &Arc<Mutex<Vec<String>>>) -> Self {
        Rec {
            log: Arc::clone(log),
            argv: Arc::new(Mutex::new(Vec::new())),
            read_out: Arc::new(Mutex::new(out(0, ""))),
        }
    }
}

impl Runner for Rec {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        assert!(program.ends_with("venvs/rns/bin/rnstatus"), "{program}");
        assert_eq!(args[0], "--config");
        let mut v = vec![program.to_string()];
        v.extend(args.iter().map(|a| a.to_string()));
        self.argv.lock().unwrap().push(v);
        if args.len() == 2 {
            self.log.lock().unwrap().push("rnstatus read".into());
            return Ok(self.read_out.lock().unwrap().clone());
        }
        self.log
            .lock()
            .unwrap()
            .push(format!("rnstatus {} {}", args[2], args[3]));
        Ok(out(0, ""))
    }

    fn run_as(&self, _: &str, _: &str, _: &[&str]) -> anyhow::Result<Output> {
        anyhow::bail!("not used")
    }
}

struct Hooks {
    log: Arc<Mutex<Vec<String>>>,
    deps: Vec<String>,
}

impl RestartHooks for Hooks {
    fn dependants(&self) -> BoxFut<'_, Vec<String>> {
        self.log.lock().unwrap().push("dependants".into());
        let d = self.deps.clone();
        Box::pin(async move { d })
    }

    fn stop_before<'a>(&'a self, id: &'a str) -> BoxFut<'a, bool> {
        self.log.lock().unwrap().push(format!("stop_before {id}"));
        Box::pin(async { true })
    }

    fn start_after<'a>(&'a self, id: &'a str) -> BoxFut<'a, ()> {
        self.log.lock().unwrap().push(format!("start_after {id}"));
        Box::pin(async {})
    }
}

struct FakeRnsd {
    log: Arc<Mutex<Vec<String>>>,
    running: AtomicBool,
}

impl RnsdChild for FakeRnsd {
    fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn restart(&self, _grace: Duration) -> BoxFut<'_, bool> {
        self.log.lock().unwrap().push("restart rnsd".into());
        Box::pin(async { true })
    }
}

/// A spine on `root` whose rnstatus is `rec`, rnsd a fake that runs, and
/// whose restart hooks report `deps`.
fn spine_at(root: &Path, rec: &Rec, lan: bool, deps: &[&str]) -> Arc<Spine> {
    let l = layout_at(root);
    let st = l.state_dir();
    let startup = interfaces::startup(&st, &st.join(store::FILE), &config::config_path(&l));
    let rn = Arc::new(Rnstatus::new(Arc::new(rec.clone()), root));
    let spine = Spine::new(l, startup, rn, lan, Some(Category::Hub4A));
    spine.set_rnsd(Arc::new(FakeRnsd {
        log: Arc::clone(&rec.log),
        running: AtomicBool::new(true),
    }));
    spine.set_restart_hooks(Arc::new(Hooks {
        log: Arc::clone(&rec.log),
        deps: deps.iter().map(|s| s.to_string()).collect(),
    }));
    spine.write_config().unwrap();
    spine
}

fn take(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    std::mem::take(&mut *log.lock().unwrap())
}

// ---------------------------------------------------------------- the file

#[test]
fn toml_load_save_modes_and_moved_aside() {
    let dir = tempfile::tempdir().unwrap();
    let st = dir.path().join("state");
    let mut f = InterfacesFile::default();
    f.radio.region = Some("EU_868".into());
    f.interfaces
        .push(rnode("RNode 1", "/dev/ttyACM0", None, 869_525_000));
    interfaces::save(&st, &f).unwrap();
    let p = interfaces::path(&st);
    assert_eq!(mode(&p), 0o600);
    assert_eq!(mode(&st), 0o700, "created 0700");
    assert_eq!(interfaces::read(&p).unwrap(), f);
    // a wrong mode is repaired on load
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(interfaces::read(&p).unwrap(), f);
    assert_eq!(mode(&p), 0o600);
    // an invalid save is refused, nothing written
    let mut bad = f.clone();
    bad.interfaces
        .push(rnode("RNode 2", "/dev/ttyACM0", None, 869_525_000));
    assert!(interfaces::save(&st, &bad).is_err());
    assert_eq!(interfaces::read(&p).unwrap(), f);
    // an unknown key: moved aside, defaults written, logged
    std::fs::write(&p, "schema = 1\nwhat = true\n").unwrap();
    let s = interfaces::startup(&st, &st.join(store::FILE), &dir.path().join("rns/config"));
    assert!(!s.read_only);
    assert_eq!(s.file, InterfacesFile::default());
    assert!(st.join("interfaces.toml.invalid-1").exists());
    assert!(s.notes[0].contains("moved aside"), "{:?}", s.notes);
    assert_eq!(interfaces::read(&p).unwrap(), InterfacesFile::default());
    // too large: refused before the parse
    std::fs::write(&p, "#".repeat(70 * 1024)).unwrap();
    assert!(matches!(interfaces::read(&p), Err(LoadError::TooLarge(_))));
    // a symlink is never followed
    std::fs::remove_file(&p).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", &p).unwrap();
    assert_eq!(interfaces::read(&p), Err(LoadError::NotRegular));
    let s = interfaces::startup(&st, &st.join(store::FILE), &dir.path().join("rns/config"));
    assert!(st.join("interfaces.toml.invalid-2").is_symlink());
    assert!(s.notes[0].contains("moved aside"));
    assert!(!p.is_symlink());
}

#[test]
fn newer_schema_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let st = dir.path().join("state");
    std::fs::create_dir_all(&st).unwrap();
    let p = interfaces::path(&st);
    let text = "schema = 2\n[future]\nx = 1\n";
    std::fs::write(&p, text).unwrap();
    let s = interfaces::startup(&st, &st.join(store::FILE), &dir.path().join("rns/config"));
    assert!(s.read_only);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), text, "untouched");
    assert!(s.notes[0].contains("newer"), "{:?}", s.notes);
}

#[test]
fn validation_refuses_bad_port_region_and_a_second_owner() {
    let mut f = InterfacesFile::default();
    f.interfaces.push(rnode(
        "RNode 1",
        "/dev/serial/by-id/usb-X-if00",
        Some("EU_868"),
        869_525_000,
    ));
    f.validate().unwrap();
    let mut g = f.clone();
    g.interfaces[0].port = Some("/dev/ttyUSB0;reboot".into());
    assert!(matches!(
        g.validate(),
        Err(interfaces::Invalid::BadPort { .. })
    ));
    let mut g = f.clone();
    g.interfaces[0].region = Some("EU_869".into());
    assert!(matches!(
        g.validate(),
        Err(interfaces::Invalid::UnknownRegion(_))
    ));
    let mut g = f.clone();
    g.interfaces.push(rnode(
        "RNode 2",
        "/dev/serial/by-id/usb-X-if00",
        Some("EU_868"),
        869_525_000,
    ));
    assert!(matches!(
        g.validate(),
        Err(interfaces::Invalid::PortHeld { .. })
    ));
    let mut g = f;
    g.set_meshtastic_binding(Some((
        "abcd1234abcd1234",
        &binding("/dev/serial/by-id/usb-X-if00", 9),
    )));
    assert!(matches!(
        g.validate(),
        Err(interfaces::Invalid::PortHeld { .. })
    ));
}

// ---------------------------------------------------------------- migration

const V1_CONFIG: &str = "schema-version = 1\n\n[reticulum]\n  enable_transport = no\n  share_instance = no\n\n\
[interfaces]\n  [[Auto Interface]]\n    type = AutoInterface\n    enabled = yes\n    group_id = 111.network\n\n\
  [[Roof RNode]]\n    type = RNodeInterface\n    interface_enabled = True\n\
    port = /dev/serial/by-id/usb-Silicon_Labs_CP2102-if00-port0\n    frequency = 869525000\n\
    bandwidth = 125000\n    txpower = 14\n    spreadingfactor = 8\n    codingrate = 5\n";

/// A Layer 5 `devices.json` holding `b` (written through the mirror, which
/// also records the Meshtastic identify result a binding needs to load).
fn devices_json_with_binding(p: &Path, b: &MeshtasticBinding) {
    assert!(store::mirror_meshtastic_binding(p, Some(("bbbb0000bbbb0000", b))).is_some());
    assert!(Store::load(p).meshtastic_binding().is_some());
}

#[test]
fn migration_from_devices_json_and_rns_config_then_divergence() {
    let dir = tempfile::tempdir().unwrap();
    let l = layout_at(dir.path());
    let st = l.state_dir();
    let devices = st.join(store::FILE);
    std::fs::create_dir_all(&st).unwrap();
    let b = binding("/dev/serial/by-id/usb-Heltec-if00", 0x0a0a0a0a);
    devices_json_with_binding(&devices, &b);
    std::fs::create_dir_all(config::config_dir(&l)).unwrap();
    std::fs::write(config::config_path(&l), V1_CONFIG).unwrap();

    let s = interfaces::startup(&st, &devices, &config::config_path(&l));
    assert!(!s.read_only);
    let kinds: Vec<_> = s
        .file
        .interfaces
        .iter()
        .map(|i| (i.name.as_str(), i.owner, i.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("Meshtastic board", IfOwner::D111, IfKind::Meshtastic),
            ("Roof RNode", IfOwner::Rnsd, IfKind::RNode),
        ]
    );
    assert!(s
        .notes
        .iter()
        .any(|n| n.contains("created from devices.json")));
    assert_eq!(mode(&interfaces::path(&st)), 0o600);
    // the binding is read from the TOML from now on
    assert_eq!(
        interfaces::meshtastic_binding_at(dir.path()),
        Some(("bbbb0000bbbb0000".to_string(), b.clone()))
    );
    // a migrated RNode has no region: it is left out of the render
    let r = config::render_with(&s.file, &secrets(), false, None, &[]);
    assert!(!r.text.contains("Roof RNode"));
    assert_eq!(r.left_out[0].name, "Roof RNode");
    assert!(r.left_out[0].reason.contains("no radio region"));

    // second start, same values: nothing to say
    let again = interfaces::startup(&st, &devices, &config::config_path(&l));
    assert!(again.notes.is_empty(), "{:?}", again.notes);
    assert_eq!(again.file, s.file);

    // a Layer 5 write moved the binding in devices.json: the TOML wins,
    // one line names both values, devices.json mirrors the TOML again
    let other =
        MeshtasticBinding::new(Uplink::tcp("10.0.0.6:4403").unwrap(), 6, None, None).unwrap();
    let mut d = Store::load(&devices);
    d.devices.get_mut("bbbb0000bbbb0000").unwrap().meshtastic = Some(other);
    d.mark_dirty();
    d.flush(true);
    let div = interfaces::startup(&st, &devices, &config::config_path(&l));
    assert_eq!(div.notes.len(), 1, "{:?}", div.notes);
    let line = &div.notes[0];
    assert!(line.contains("10.0.0.6:4403"), "{line}");
    assert!(line.contains("/dev/serial/by-id/usb-Heltec-if00"), "{line}");
    assert!(line.contains("interfaces.toml wins"), "{line}");
    assert_eq!(Store::load(&devices).meshtastic_binding().unwrap().1, b);
}

#[test]
fn migration_without_sources_is_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let st = dir.path().join("state");
    let s = interfaces::startup(&st, &st.join(store::FILE), &dir.path().join("rns/config"));
    assert_eq!(s.file, InterfacesFile::default());
    assert!(interfaces::path(&st).exists());
    assert!(!st.join(store::FILE).exists(), "nothing to mirror");
}

// ---------------------------------------------------------------- renders

fn stanza_set() -> InterfacesFile {
    let mut f = InterfacesFile::default();
    f.radio.region = Some("EU_868".into());
    // in band, power under the limit; EU_868's 10 % duty cycle is applied
    f.interfaces.push(rnode(
        "Roof RNode",
        "/dev/serial/by-id/usb-Silicon_Labs_CP2102-if00-port0",
        None,
        869_525_000,
    ));
    // a per-device region override (US), with optional params
    let mut us = rnode(
        "Field RNode",
        "tcp://10.0.0.5:4403",
        Some("US"),
        914_875_000,
    );
    us.params.insert("txpower".into(), int(22));
    us.params
        .insert("airtime_limit_short".into(), toml::Value::Float(33.5));
    us.params
        .insert("id_callsign".into(), toml::Value::String("W1AW".into()));
    us.params.insert("id_interval".into(), int(600));
    f.interfaces.push(us);
    // out of band for EU_868: left out
    f.interfaces
        .push(rnode("Wrong Band", "/dev/ttyACM3", None, 868_100_000));
    // a phone-relay client with IFAC
    f.interfaces.push(Interface {
        name: "111 hub 10.0.0.1".into(),
        owner: IfOwner::Rnsd,
        kind: IfKind::TcpClient,
        device: None,
        port: None,
        region: None,
        uplink: None,
        params: params(&[
            ("target_host", toml::Value::String("10.0.0.1".into())),
            ("target_port", int(4242)),
            ("ifac", toml::Value::Boolean(true)),
            ("network_name", toml::Value::String("111-lan".into())),
        ]),
    });
    // the 111d Meshtastic board is never rendered into rnsd's config
    f.set_meshtastic_binding(Some((
        "bbbb0000bbbb0000",
        &binding("/dev/serial/by-id/usb-Heltec-if00", 7),
    )));
    f.validate().unwrap();
    f
}

#[test]
fn golden_renders_per_posture_and_stanza_set() {
    let s = secrets();
    let base = InterfacesFile::default();

    // posture per category: enable_transport follows Cat 3/4 unless set
    golden(
        "default-node2a",
        &config::render(&base, &s, false, Some(Category::Node2A)),
    );
    golden(
        "default-hub4a",
        &config::render(&base, &s, false, Some(Category::Hub4A)),
    );
    let mut off = base.clone();
    off.posture.enable_transport = Some(false);
    off.posture.share_instance = false;
    off.posture.instance_name = "lab".into();
    golden(
        "posture-explicit",
        &config::render(&off, &s, false, Some(Category::Hub4A)),
    );

    // LAN sharing off ⇒ no [posture.lan] items, whatever they say
    let mut lan = base.clone();
    lan.posture.lan.stock_autointerface = true;
    lan.posture.lan.propagation_node = true;
    assert_eq!(
        config::render(&lan, &s, false, Some(Category::Node2A)),
        config::render(&base, &s, false, Some(Category::Node2A))
    );
    // LAN sharing on: the Backbone listener, IFAC off
    golden(
        "lan-backbone",
        &config::render(&base, &s, true, Some(Category::Hub4A)),
    );
    // IFAC on
    let mut ifac = base.clone();
    ifac.posture.lan.backbone.ifac = true;
    golden(
        "lan-backbone-ifac",
        &config::render(&ifac, &s, true, Some(Category::Hub4A)),
    );
    // IFAC on but no passphrase: the listener stays closed
    let r = config::render_with(
        &ifac,
        &Secrets::new([0x11; 32]),
        true,
        Some(Category::Hub4A),
        &[],
    );
    assert!(!r.text.contains("BackboneInterface"));
    assert_eq!(r.left_out[0].name, "111 LAN Backbone");
    // the opt-ins, listener off
    lan.posture.lan.backbone.listen = false;
    golden(
        "lan-optins",
        &config::render(&lan, &s, true, Some(Category::Bridge3)),
    );

    // the stanza set: regions applied, the out-of-band and no-region
    // stanzas left out with their reasons
    let f = stanza_set();
    let r = config::render_with(&f, &s, false, Some(Category::Hub4A), &[]);
    golden("stanzas", &r.text);
    let reasons: Vec<_> = r.left_out.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(reasons, ["Wrong Band"]);
    assert!(r.left_out[0].reason.contains("outside EU_868"));
    // with no region at all every RNode is left out
    let mut none = f.clone();
    none.radio.region = None;
    let r = config::render_with(&none, &s, false, Some(Category::Hub4A), &[]);
    let names: Vec<_> = r.left_out.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(
        names,
        ["Roof RNode", "Wrong Band"],
        "Field RNode has its own region"
    );
    assert!(r.text.contains("[[Field RNode]]"));
    // power above the region's limit: left out
    let mut hot = f.clone();
    hot.interfaces[0].params.insert("txpower".into(), int(30));
    let r = config::render_with(&hot, &s, false, None, &[]);
    assert!(r
        .left_out
        .iter()
        .any(|l| l.reason.contains("above the EU_868 limit")));
    // a client whose passphrase is missing: left out
    let r = config::render_with(&f, &Secrets::new([0x11; 32]), false, None, &[]);
    assert!(r.left_out.iter().any(|l| l.name == "111 hub 10.0.0.1"));

    // the helper's client config
    golden("client", &config::render_client_config(&s, "111"));
}

#[test]
fn modes_on_a_scratch_root() {
    let keep = std::env::var_os("SPINE_KEEP_ROOT").map(PathBuf::from);
    let tmp = tempfile::tempdir().unwrap();
    let root = keep.clone().unwrap_or_else(|| tmp.path().to_path_buf());
    let l = layout_at(&root);
    // a v1-era tree: rns/ 0755 with a 0644 config
    std::fs::create_dir_all(config::config_dir(&l)).unwrap();
    std::fs::set_permissions(
        config::config_dir(&l),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::write(config::config_path(&l), V1_CONFIG).unwrap();
    std::fs::set_permissions(
        config::config_path(&l),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let spine = spine_at(&root, &Rec::new(&log), false, &[]);
    let s = Secrets::for_file(&root, &spine.file()).unwrap();
    config::write_client_config(&root, &s, "111").unwrap();
    assert_eq!(mode(&config::config_dir(&l)), 0o700);
    assert_eq!(mode(&config::config_path(&l)), 0o600);
    assert_eq!(mode(&config::client_dir(&root)), 0o700);
    assert_eq!(mode(&root.join("bridge/lxmf")), 0o700);
    assert_eq!(mode(&config::client_config_path(&root)), 0o600);
    assert_eq!(mode(&config::secrets_dir(&root)), 0o700);
    assert_eq!(
        mode(&config::secrets_dir(&root).join(config::RPC_KEY_FILE)),
        0o600
    );
    assert_eq!(mode(&interfaces::path(&l.state_dir())), 0o600);
    // the rpc_key is the same in both configs
    let key = s.rpc_key_hex();
    assert!(std::fs::read_to_string(config::config_path(&l))
        .unwrap()
        .contains(&format!("rpc_key = {key}")));
    assert!(std::fs::read_to_string(config::client_config_path(&root))
        .unwrap()
        .contains(&format!("rpc_key = {key}")));
    // with SPINE_KEEP_ROOT the tree stays for verify/spine.sh's own check
    drop(tmp);
}

#[test]
fn v1_leaves_v2_alone_and_v2_regenerates_v1() {
    let dir = tempfile::tempdir().unwrap();
    let l = layout_at(dir.path());
    std::fs::create_dir_all(config::config_dir(&l)).unwrap();
    std::fs::write(config::config_path(&l), V1_CONFIG).unwrap();
    assert_eq!(config::existing_schema_version(&l), Some(1));
    assert!(config::should_regenerate(&l));
    let log = Arc::new(Mutex::new(Vec::new()));
    let spine = spine_at(dir.path(), &Rec::new(&log), false, &[]);
    assert_eq!(config::existing_schema_version(&l), Some(2));
    // the v1 binary's rule: regenerate unless `Some(v) if v > 1`
    let v1_regenerates = |v: Option<u32>| !matches!(v, Some(v) if v > 1);
    assert!(!v1_regenerates(config::existing_schema_version(&l)));
    // a newer file is left alone by v2
    std::fs::write(config::config_path(&l), "schema-version = 3\n").unwrap();
    assert!(!config::should_regenerate(&l));
    assert_eq!(
        spine.write_config(),
        Err(node111::daemon::transport::posture::ApplyError::NewerConfig)
    );
    assert_eq!(
        std::fs::read_to_string(config::config_path(&l)).unwrap(),
        "schema-version = 3\n"
    );
}

// ---------------------------------------------------------------- rnstatus

fn facts(name: &str, kind: &str, up: bool, rx: u64, tx: u64, peers: Option<u64>) -> InterfaceFacts {
    InterfaceFacts {
        name: name.into(),
        kind: kind.into(),
        up,
        rx,
        tx,
        peers,
    }
}

#[test]
fn rnstatus_parser_goldens() {
    let read = |n: &str| std::fs::read_to_string(fixture(&format!("rnstatus/{n}"))).unwrap();
    assert_eq!(
        rnstatus::parse_text(&read("hub.txt")),
        [
            facts("rns/111", "Shared Instance", true, 11_980, 12_400, Some(1)),
            facts(
                "Auto Interface",
                "AutoInterface",
                true,
                2_200,
                3_210,
                Some(2)
            ),
            facts(
                "111 LAN Backbone",
                "BackboneInterface",
                true,
                1_500,
                980,
                Some(1)
            ),
            facts("Roof RNode", "RNodeInterface", false, 0, 0, None),
        ]
    );
    assert_eq!(
        rnstatus::parse_text(&read("phone.txt")),
        [
            facts("rns/111", "Shared Instance", true, 0, 0, Some(0)),
            facts("Auto Interface", "AutoInterface", true, 0, 144, Some(0)),
            facts("111 hub 10.0.0.1", "TCPInterface", true, 4_110, 2_050, None),
        ]
    );
    assert_eq!(
        rnstatus::parse_text(&read("legacy-traffic.txt")),
        [
            facts("rns/111", "Shared Instance", true, 980, 1_210, Some(2)),
            facts("Auto Interface", "AutoInterface", false, 0, 0, None),
        ]
    );
    // B4 review F1: both arrows on one line, a non-ASCII byte before a
    // size, an unparsable value: read what parses, never panic
    assert_eq!(
        rnstatus::parse_text(&read("arrows.txt")),
        [
            facts("rns/111", "Shared Instance", true, 980, 1_210, Some(1)),
            facts("Roof RNode", "RNodeInterface", true, 5, 2_470, Some(3)),
            facts("111 hub 10.0.0.1", "TCPInterface", true, 2_000, 1_000, None),
        ]
    );
    assert!(rnstatus::parse_text(&read("no-instance.txt")).is_empty());
    // -j: unverified shape, parsed but not used by the daemon
    assert_eq!(
        rnstatus::parse_json(&read("status.json")).unwrap(),
        [
            facts("rns/111", "LocalInterface", true, 11_980, 12_400, Some(2)),
            facts(
                "Auto Interface",
                "AutoInterface",
                true,
                2_200,
                3_210,
                Some(2)
            ),
        ]
    );
    // untrusted names are cleaned
    let evil = " RNodeInterface[\u{202e}x\u{7}y]\n    Status    : Up\n";
    assert_eq!(rnstatus::parse_text(evil)[0].name, "xy");
}

#[test]
fn attach_detach_reload_argv_and_errors() {
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let rec = Rec::new(&log);
    let rn = Rnstatus::new(Arc::new(rec.clone()), dir.path());
    rn.attach("RNode 1").unwrap();
    rn.detach("RNode 1").unwrap();
    rn.reload("Auto Interface").unwrap();
    let prog = dir.path().join("venvs/rns/bin/rnstatus");
    let cfg = dir.path().join("rns");
    let want = |flag: &str, name: &str| {
        vec![
            prog.to_string_lossy().into_owned(),
            "--config".into(),
            cfg.to_string_lossy().into_owned(),
            flag.into(),
            name.into(),
        ]
    };
    assert_eq!(
        *rec.argv.lock().unwrap(),
        [
            want("--attach", "RNode 1"),
            want("--detach", "RNode 1"),
            want("--reload", "Auto Interface"),
        ]
    );
    // errors through MockRunner: a missing name, no instance, a failure
    let p = prog.to_string_lossy().into_owned();
    let c = cfg.to_string_lossy().into_owned();
    let mock = node111::testkit::MockRunner::new(vec![
        (
            format!("{p} --config {c} --attach Gone"),
            out(1, "Interface \"Gone\" does not exist\n"),
        ),
        (
            format!("{p} --config {c} --detach X"),
            out(1, "No shared RNS instance available to get status from\n"),
        ),
        (format!("{p} --config {c} --reload X"), out(2, "boom\n")),
        (
            format!("{p} --config {c}"),
            out(1, "No shared RNS instance available to get status from\n"),
        ),
    ]);
    let rn = Rnstatus::new(Arc::new(mock), dir.path());
    assert_eq!(
        rn.attach("Gone"),
        Err(RnstatusError::NotFound("Gone".into()))
    );
    assert_eq!(rn.detach("X"), Err(RnstatusError::NoInstance));
    assert!(matches!(
        rn.reload("X"),
        Err(RnstatusError::Exit { status: 2, .. })
    ));
    assert_eq!(rn.read(), Err(RnstatusError::NoInstance));
    // the production runner: an absent program is a spawn error
    let real = Rnstatus::production(dir.path());
    assert!(matches!(real.read(), Err(RnstatusError::Spawn(_))));
}

#[test]
fn timed_runner_kills_a_hung_rnstatus() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("venvs/rns/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let prog = bin.join("rnstatus");
    std::fs::write(&prog, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&prog, std::fs::Permissions::from_mode(0o755)).unwrap();
    let runner = rnstatus::TimedRunner {
        timeout: Duration::from_millis(300),
        home: dir.path().to_path_buf(),
    };
    let t0 = std::time::Instant::now();
    let rn = Rnstatus::new(Arc::new(runner), dir.path());
    assert!(matches!(rn.read(), Err(RnstatusError::Spawn(_))));
    assert!(t0.elapsed() < Duration::from_secs(5));
    // a real script's output is parsed
    std::fs::write(
        &prog,
        format!(
            "#!/bin/sh\ncat '{}'\n",
            fixture("rnstatus/hub.txt").display()
        ),
    )
    .unwrap();
    let rn = Rnstatus::production(dir.path());
    assert_eq!(rn.read().unwrap().len(), 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn facts_tick_fills_the_cache_only_on_demand() {
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let rec = Rec::new(&log);
    *rec.read_out.lock().unwrap() = out(
        0,
        &std::fs::read_to_string(fixture("rnstatus/hub.txt")).unwrap(),
    );
    let rn = Arc::new(Rnstatus::new(Arc::new(rec.clone()), dir.path()));
    let cache = FactsCache::new(TickConfig {
        period: Duration::from_millis(40),
        demand_window: Duration::from_millis(200),
        backoff_max: Duration::from_millis(160),
    });
    let running = Arc::new(AtomicBool::new(true));
    let r2 = Arc::clone(&running);
    let ticks = Arc::new(Mutex::new(0u32));
    let t2 = Arc::clone(&ticks);
    let task = tokio::spawn(rnstatus::tick(
        Arc::clone(&cache),
        rn,
        Arc::new(move || r2.load(Ordering::SeqCst)),
        move |_c: &FactsCache| *t2.lock().unwrap() += 1,
    ));
    // a reader asks: the first reading fills the cache (demand only, F2)
    let _ = cache.snapshot();
    let wait = |want: CacheState| {
        let cache = Arc::clone(&cache);
        async move {
            for _ in 0..100 {
                if cache.peek().state == want {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            false
        }
    };
    assert!(wait(CacheState::Ok).await);
    let s = cache.snapshot();
    assert_eq!(s.interfaces.len(), 4);
    assert_eq!(cache.joined(), Some(true));
    assert_eq!(cache.serving(), Some(true));
    // no demand: the tick keeps ticking but stops starting rnstatus
    tokio::time::sleep(Duration::from_millis(400)).await;
    let reads = |l: &Arc<Mutex<Vec<String>>>| {
        l.lock()
            .unwrap()
            .iter()
            .filter(|x| *x == "rnstatus read")
            .count()
    };
    let idle = reads(&log);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(reads(&log), idle, "idle: no rnstatus runs");
    assert!(*ticks.lock().unwrap() > 5);
    assert_eq!(cache.peek().state, CacheState::Stale);
    // a reader nudges it: fresh again
    let _ = cache.snapshot();
    assert!(wait(CacheState::Ok).await);
    assert!(reads(&log) > idle);
    // rnstatus fails: unavailable, no interfaces
    *rec.read_out.lock().unwrap() = out(1, "No shared RNS instance available to get status from\n");
    for _ in 0..50 {
        let _ = cache.snapshot();
        if cache.peek().state == CacheState::Unavailable {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(cache.peek().state, CacheState::Unavailable);
    assert!(cache.peek().interfaces.is_empty());
    // rnsd down: unavailable without running rnstatus
    running.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let n = reads(&log);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = cache.snapshot();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(reads(&log), n);
    assert_eq!(cache.peek().state, CacheState::Unavailable);
    task.abort();
}

// ---------------------------------------------------------------- posture

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stanza_changes_attach_reload_detach_without_restart() {
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let rec = Rec::new(&log);
    let spine = spine_at(dir.path(), &rec, false, &["lxmf"]);
    take(&log);
    // a new RNode: --attach only
    let a = spine
        .update(|f| {
            f.radio.region = Some("EU_868".into());
            f.interfaces
                .push(rnode("RNode 1", "/dev/ttyACM0", None, 869_525_000));
        })
        .await
        .unwrap();
    assert!(!a.restarted);
    assert_eq!(take(&log), ["rnstatus --attach RNode 1"]);
    // its parameters change: --reload
    let a = spine
        .update(|f| {
            f.interfaces[0]
                .params
                .insert("spreadingfactor".into(), int(9));
        })
        .await
        .unwrap();
    assert_eq!(a.diff.reload, ["RNode 1"]);
    assert_eq!(take(&log), ["rnstatus --reload RNode 1"]);
    // the same file again: nothing runs
    let a = spine.apply().await.unwrap();
    assert!(!a.written && a.diff.is_empty());
    assert!(take(&log).is_empty());
    // removed: --detach
    spine
        .update(|f| assert!(f.remove("RNode 1")))
        .await
        .unwrap();
    assert_eq!(take(&log), ["rnstatus --detach RNode 1"]);
    // a link change re-renders (unchanged) and reloads the AutoInterface
    let a = spine.on_link_change().await.unwrap();
    assert!(!a.restarted);
    assert_eq!(take(&log), ["rnstatus --reload Auto Interface"]);
    // LAN sharing on: the Backbone listener is attached live
    spine.set_lan_sharing(true).await.unwrap();
    assert_eq!(take(&log), ["rnstatus --attach 111 LAN Backbone"]);
    // an invalid change is refused and nothing runs
    let e = spine
        .update(|f| f.posture.instance_name = "bad name".into())
        .await;
    assert!(e.is_err());
    assert!(take(&log).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reticulum_change_restarts_with_dependants_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let rec = Rec::new(&log);
    let spine = spine_at(dir.path(), &rec, false, &["lxmf", "other"]);
    take(&log);
    let a = spine
        .update(|f| f.posture.enable_transport = Some(false))
        .await
        .unwrap();
    assert!(a.restarted && a.diff.restart);
    assert_eq!(
        take(&log),
        [
            "dependants",
            "stop_before lxmf",
            "stop_before other",
            "restart rnsd",
            "start_after lxmf",
            "start_after other",
        ],
        "never an rnstatus control call, never pkill"
    );
    assert_eq!(
        a.actions,
        [
            "stop lxmf",
            "stop other",
            "restart rnsd",
            "start lxmf",
            "start other"
        ]
    );
    // share_instance and instance_name are posture too
    spine
        .update(|f| f.posture.instance_name = "lab".into())
        .await
        .unwrap();
    assert!(take(&log).contains(&"restart rnsd".to_string()));
    // a rendered config with the new posture
    let text = std::fs::read_to_string(config::config_path(&layout_at(dir.path()))).unwrap();
    assert!(text.contains("enable_transport = No"));
    assert!(text.contains("instance_name = lab"));
    // share_instance off: no control RPC, so a stanza change restarts too
    spine
        .update(|f| f.posture.share_instance = false)
        .await
        .unwrap();
    take(&log);
    let a = spine
        .update(|f| {
            f.radio.region = Some("EU_868".into());
            f.interfaces
                .push(rnode("RNode 1", "/dev/ttyACM0", None, 869_525_000));
        })
        .await
        .unwrap();
    assert!(a.restarted);
    assert!(!take(&log).iter().any(|l| l.starts_with("rnstatus")));
    // and a link change reloads nothing
    spine.on_link_change().await.unwrap();
    assert!(take(&log).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_real_child_control_restarts_rnsd() {
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let rec = Rec::new(&log);
    let l = layout_at(dir.path());
    let st = l.state_dir();
    let startup = interfaces::startup(&st, &st.join(store::FILE), &config::config_path(&l));
    let spine = Spine::new(
        l,
        startup,
        Arc::new(Rnstatus::new(Arc::new(rec), dir.path())),
        false,
        None,
    );
    spine.set_grace(Duration::from_secs(2));
    spine.write_config().unwrap();
    let child = spawn_child(ChildSpec {
        name: "fake-rnsd".into(),
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".into(), "exec sleep 300".into()],
        env: Vec::new(),
        backoff: BackoffPolicy::default(),
        stdout_log: None,
    });
    let control = child.control();
    for _ in 0..100 {
        if control.pid().is_some() && control.state() == ChildState::Running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let pid = control.pid().unwrap();
    spine.set_rnsd(Arc::new(control.clone()));
    let a = spine
        .update(|f| f.posture.share_instance = false)
        .await
        .unwrap();
    assert!(a.restarted, "{a:?}");
    let mut new = None;
    for _ in 0..200 {
        if let Some(p) = control.pid().filter(|p| *p != pid) {
            new = Some(p);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(new.is_some(), "rnsd respawned with a new pid");
    child.control().stop_and_wait(Duration::from_secs(2)).await;
}

// ---------------------------------------------------------------- join

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn join_descriptor_and_apply_join_round_trip() {
    let hub_dir = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let hub = spine_at(hub_dir.path(), &Rec::new(&log), true, &[]);
    let addr: std::net::IpAddr = "10.0.0.1".parse().unwrap();
    // IFAC off: address and port only
    let d = hub.join_descriptor(addr, Session::Paired).unwrap();
    assert_eq!(
        serde_json::to_value(&d).unwrap(),
        serde_json::json!({"schema": 1, "address": "10.0.0.1", "port": 4242})
    );
    // IFAC on: the name and passphrase, minted into secrets/
    hub.update(|f| f.posture.lan.backbone.ifac = true)
        .await
        .unwrap();
    let d = hub.join_descriptor(addr, Session::Loopback).unwrap();
    let pass = d.ifac_passphrase.clone().unwrap();
    assert_eq!(d.ifac_name.as_deref(), Some("111-lan"));
    assert!(!format!("{d:?}").contains(&pass), "Debug is redacted");
    let hub_text =
        std::fs::read_to_string(config::config_path(&layout_at(hub_dir.path()))).unwrap();
    assert!(hub_text.contains(&format!("passphrase = {pass}")));
    // refusals
    assert_eq!(
        hub.join_descriptor(addr, Session::Other),
        Err(JoinError::Untrusted)
    );
    assert_eq!(
        hub.join_descriptor("127.0.0.1".parse().unwrap(), Session::Paired),
        Err(JoinError::BadAddress)
    );
    hub.set_lan_sharing(false).await.unwrap();
    assert_eq!(
        hub.join_descriptor(addr, Session::Paired),
        Err(JoinError::LanOff)
    );

    // the phone applies it (over JSON, as it arrives)
    let wire = serde_json::to_string(&d).unwrap();
    let got: join::JoinDescriptor = serde_json::from_str(&wire).unwrap();
    let phone_dir = tempfile::tempdir().unwrap();
    let plog = Arc::new(Mutex::new(Vec::new()));
    let phone = spine_at(phone_dir.path(), &Rec::new(&plog), false, &[]);
    take(&plog);
    let (name, applied) = join::apply_join(&phone, &got).await.unwrap();
    assert_eq!(name, "111 hub 10.0.0.1");
    assert_eq!(applied.actions, ["attach 111 hub 10.0.0.1"]);
    let entry = phone.file().interface(&name).unwrap().clone();
    assert_eq!(entry.owner, IfOwner::Rnsd);
    assert_eq!(entry.kind, IfKind::TcpClient);
    assert_eq!(entry.text("target_host"), Some("10.0.0.1"));
    assert_eq!(entry.int("target_port"), Some(4242));
    let toml_text =
        std::fs::read_to_string(interfaces::path(&layout_at(phone_dir.path()).state_dir()))
            .unwrap();
    assert!(
        !toml_text.contains(&pass),
        "the passphrase never sits in interfaces.toml"
    );
    let secret = config::client_ifac_path(phone_dir.path(), &name);
    assert_eq!(mode(&secret), 0o600);
    let text = std::fs::read_to_string(config::config_path(&layout_at(phone_dir.path()))).unwrap();
    assert!(text.contains("  [[111 hub 10.0.0.1]]\n    type = TCPClientInterface\n    enabled = yes\n    target_host = 10.0.0.1\n    target_port = 4242\n    network_name = 111-lan\n"), "{text}");
    assert!(text.contains(&format!("passphrase = {pass}")));
    // joining again replaces, not duplicates
    let (_, again) = join::apply_join(&phone, &got).await.unwrap();
    assert!(again.diff.is_empty());
    assert_eq!(phone.file().interfaces.len(), 1);

    // untrusted descriptors are refused field by field
    let mut bad = got.clone();
    bad.address = "10.0.0.1\n[reticulum]".into();
    assert!(join::validate(&bad).is_err());
    let mut bad = got.clone();
    bad.ifac_passphrase = Some("short".into());
    assert!(join::validate(&bad).is_err());
    let mut bad = got.clone();
    bad.ifac_name = None;
    assert!(join::validate(&bad).is_err());
    let mut bad = got;
    bad.address = "0.0.0.0".into();
    assert_eq!(join::validate(&bad), Err(JoinError::BadAddress));
    assert!(serde_json::from_str::<join::JoinDescriptor>(
        r#"{"schema":1,"address":"10.0.0.1","port":4242,"extra":1}"#
    )
    .is_err());
}

// ---------------------------------------------------------------- ports

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn port_table_claims_on_attach_and_release_on_detach() {
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let spine = spine_at(dir.path(), &Rec::new(&log), false, &[]);
    let table = Arc::new(PortTable::new());
    spine
        .adopt_ports(Arc::clone(&table), Arc::new(|| 7))
        .await
        .unwrap();
    take(&log);
    // attach claims for rnsd first
    spine
        .update(|f| {
            f.radio.region = Some("EU_868".into());
            f.interfaces
                .push(rnode("RNode 1", "/dev/ttyACM0", None, 869_525_000));
        })
        .await
        .unwrap();
    assert_eq!(table.is_held("/dev/ttyACM0").unwrap().task, "rnsd");
    assert_eq!(take(&log), ["rnstatus --attach RNode 1"]);
    // a port another task holds: left out, never attached, holder named
    table.claim("/dev/ttyACM1", "meshmud", 8).unwrap();
    let a = spine
        .update(|f| {
            f.interfaces
                .push(rnode("RNode 2", "/dev/ttyACM1", None, 869_525_000));
        })
        .await
        .unwrap();
    assert!(take(&log).is_empty());
    assert_eq!(a.left_out[0].name, "RNode 2");
    assert!(a.left_out[0].reason.contains("meshmud"));
    assert_eq!(table.is_held("/dev/ttyACM1").unwrap().task, "meshmud");
    assert_eq!(spine.left_out()[0].name, "RNode 2");
    // detach releases
    spine
        .update(|f| {
            f.remove("RNode 1");
        })
        .await
        .unwrap();
    assert_eq!(take(&log), ["rnstatus --detach RNode 1"]);
    assert!(table.is_held("/dev/ttyACM0").is_none());
    // the holder lets go: the next apply claims and attaches
    table.release("/dev/ttyACM1", "meshmud").unwrap();
    spine.apply().await.unwrap();
    assert_eq!(take(&log), ["rnstatus --attach RNode 2"]);
    assert_eq!(table.is_held("/dev/ttyACM1").unwrap().task, "rnsd");
    // a 111d Meshtastic uplink is claimed for meshmud on adopt
    let b = binding("/dev/serial/by-id/usb-Heltec-if00", 5);
    spine
        .set_meshtastic_binding(Some(("bbbb0000bbbb0000".into(), b)))
        .await
        .unwrap();
    let t2 = Arc::new(PortTable::new());
    spine
        .adopt_ports(Arc::clone(&t2), Arc::new(|| 9))
        .await
        .unwrap();
    assert_eq!(
        t2.is_held("/dev/serial/by-id/usb-Heltec-if00")
            .unwrap()
            .task,
        "meshmud"
    );
    // and the binding was mirrored into devices.json (write-only copy)
    let d = Store::load(&layout_at(dir.path()).state_dir().join(store::FILE));
    assert_eq!(d.meshtastic_binding().unwrap().0, "bbbb0000bbbb0000");
}

// ---------------------------------------------------------------- B4 review

fn reads(l: &Arc<Mutex<Vec<String>>>) -> usize {
    l.lock()
        .unwrap()
        .iter()
        .filter(|x| *x == "rnstatus read")
        .count()
}

/// B4 review F2: with `share_instance = false` no rnstatus runs, reader or
/// not; with it on, rnstatus runs only while a reader asked within the
/// demand window and stops once the window passes without a read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn f2_tick_runs_rnstatus_on_demand_and_never_without_a_shared_instance() {
    let dir = tempfile::tempdir().unwrap();
    let st = layout_at(dir.path()).state_dir();
    let mut f = InterfacesFile::default();
    f.posture.share_instance = false;
    interfaces::save(&st, &f).unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let rec = Rec::new(&log);
    *rec.read_out.lock().unwrap() = out(
        0,
        &std::fs::read_to_string(fixture("rnstatus/hub.txt")).unwrap(),
    );
    let spine = spine_at(dir.path(), &rec, false, &[]);
    let rn = spine.rnstatus();
    assert!(
        !rn.shares_instance(),
        "the spine passed the file's posture on"
    );
    let cache = FactsCache::new(TickConfig {
        period: Duration::from_millis(20),
        demand_window: Duration::from_millis(250),
        backoff_max: Duration::from_millis(40),
    });
    let ticks = Arc::new(Mutex::new(0u32));
    let t2 = Arc::clone(&ticks);
    let task = tokio::spawn(rnstatus::tick(
        Arc::clone(&cache),
        Arc::clone(&rn),
        Arc::new(|| true),
        move |_c: &FactsCache| *t2.lock().unwrap() += 1,
    ));
    // no shared instance, no reader: many ticks, zero runs
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(*ticks.lock().unwrap() > 5, "the tick ticks");
    assert_eq!(reads(&log), 0, "no shared instance, no reader: no rnstatus");
    // no shared instance, a reader: still zero runs
    for _ in 0..15 {
        assert_eq!(cache.snapshot().state, CacheState::Unavailable);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(reads(&log), 0, "no shared instance: no control RPC to ask");
    // the reader goes away for longer than the window, then share_instance
    // turns on (a posture change: a restart): zero runs without demand
    tokio::time::sleep(Duration::from_millis(300)).await;
    spine
        .update(|f| f.posture.share_instance = true)
        .await
        .unwrap();
    assert!(rn.shares_instance());
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(reads(&log), 0, "no reader within the window: no rnstatus");
    // a reader within the window: runs happen
    for _ in 0..10 {
        let _ = cache.snapshot();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(reads(&log) > 0, "a reader: rnstatus runs");
    assert_eq!(cache.peek().state, CacheState::Ok);
    // the window passes without a read: the runs stop
    tokio::time::sleep(Duration::from_millis(400)).await;
    let n = reads(&log);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(reads(&log), n, "no read within the window: no more runs");
    // share_instance off again: the tick records nothing to read
    spine
        .update(|f| f.posture.share_instance = false)
        .await
        .unwrap();
    for _ in 0..10 {
        let _ = cache.snapshot();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(reads(&log), n, "off again: no runs even with a reader");
    assert_eq!(cache.peek().state, CacheState::Unavailable);
    task.abort();
}

/// B4 review F3: an unreadable `secrets/rns-rpc-key` is `ApplyError::Write`
/// and nothing rotates (a symlink fails the `O_NOFOLLOW` read even as root).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn f3_unreadable_rpc_key_fails_the_apply_without_rotating() {
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let spine = spine_at(dir.path(), &Rec::new(&log), false, &[]);
    take(&log);
    let cfg = config::config_path(&layout_at(dir.path()));
    let before = std::fs::read_to_string(&cfg).unwrap();
    let key = config::secrets_dir(dir.path()).join(config::RPC_KEY_FILE);
    let held = dir.path().join("held");
    std::fs::rename(&key, &held).unwrap();
    std::os::unix::fs::symlink(&held, &key).unwrap();
    let e = spine.apply().await;
    assert!(
        matches!(
            e,
            Err(node111::daemon::transport::posture::ApplyError::Write(_))
        ),
        "{e:?}"
    );
    assert!(std::fs::symlink_metadata(&key)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        std::fs::read_to_string(&cfg).unwrap(),
        before,
        "no re-render"
    );
    assert!(take(&log).is_empty(), "no restart, no rnstatus");
}

/// B4 review F6: a refused or failed join update leaves no orphaned client
/// secret, and a failed re-join puts the earlier passphrase back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn f6_failed_join_leaves_no_orphaned_secret() {
    let d = join::JoinDescriptor {
        schema: join::JOIN_SCHEMA,
        address: "10.0.0.1".into(),
        port: 4242,
        ifac_name: Some("111-lan".into()),
        ifac_passphrase: Some("0123456789abcdef0123456789abcdef".into()),
    };
    let name = "111 hub 10.0.0.1";
    // refused: a newer interfaces.toml makes the spine read-only
    let dir = tempfile::tempdir().unwrap();
    let st = layout_at(dir.path()).state_dir();
    std::fs::create_dir_all(&st).unwrap();
    std::fs::write(interfaces::path(&st), "schema = 9\n").unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let spine = spine_at(dir.path(), &Rec::new(&log), false, &[]);
    assert!(spine.read_only());
    let e = join::apply_join(&spine, &d).await;
    assert!(matches!(e, Err(JoinError::Apply(_))), "{e:?}");
    assert!(
        !config::client_ifac_path(dir.path(), name).exists(),
        "no orphaned client secret"
    );
    // failed after the save: rns/config cannot be written; a re-join puts
    // the first passphrase back
    let dir = tempfile::tempdir().unwrap();
    let spine = spine_at(dir.path(), &Rec::new(&log), false, &[]);
    join::apply_join(&spine, &d).await.unwrap();
    let secret = config::client_ifac_path(dir.path(), name);
    let first = std::fs::read_to_string(&secret).unwrap();
    let cfg = config::config_path(&layout_at(dir.path()));
    std::fs::remove_file(&cfg).unwrap();
    std::fs::create_dir_all(cfg.join("blocker")).unwrap();
    let mut again = d.clone();
    again.port = 4243;
    again.ifac_passphrase = Some("fedcba9876543210fedcba9876543210".into());
    let e = join::apply_join(&spine, &again).await;
    assert!(matches!(e, Err(JoinError::Apply(_))), "{e:?}");
    assert_eq!(std::fs::read_to_string(&secret).unwrap(), first);
    // a first join that fails removes what it stored
    let dir = tempfile::tempdir().unwrap();
    let spine = spine_at(dir.path(), &Rec::new(&log), false, &[]);
    let cfg = config::config_path(&layout_at(dir.path()));
    std::fs::remove_file(&cfg).unwrap();
    std::fs::create_dir_all(cfg.join("blocker")).unwrap();
    assert!(join::apply_join(&spine, &d).await.is_err());
    assert!(!config::client_ifac_path(dir.path(), name).exists());
}

/// B4 review F8: ports claimed for a render whose write fails are released
/// again; a claim held from an earlier good apply stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn f8_failed_write_releases_the_new_claims() {
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let spine = spine_at(dir.path(), &Rec::new(&log), false, &[]);
    let table = Arc::new(PortTable::new());
    spine
        .adopt_ports(Arc::clone(&table), Arc::new(|| 7))
        .await
        .unwrap();
    spine
        .update(|f| {
            f.radio.region = Some("EU_868".into());
            f.interfaces
                .push(rnode("RNode 1", "/dev/ttyACM0", None, 869_525_000));
        })
        .await
        .unwrap();
    assert_eq!(table.is_held("/dev/ttyACM0").unwrap().task, "rnsd");
    // rns/config cannot be written (a non-empty directory in its place:
    // fails even as root, unlike a read-only dir)
    let cfg = config::config_path(&layout_at(dir.path()));
    std::fs::remove_file(&cfg).unwrap();
    std::fs::create_dir_all(cfg.join("blocker")).unwrap();
    let e = spine
        .update(|f| {
            f.interfaces
                .push(rnode("RNode 2", "/dev/ttyACM1", None, 869_525_000));
        })
        .await;
    assert!(
        matches!(
            e,
            Err(node111::daemon::transport::posture::ApplyError::Write(_))
        ),
        "{e:?}"
    );
    assert!(
        table.is_held("/dev/ttyACM1").is_none(),
        "the new claim is released on the error path"
    );
    assert_eq!(
        table.is_held("/dev/ttyACM0").unwrap().task,
        "rnsd",
        "the claim of the running stanza stays"
    );
    // a fresh table (adopt_ports → apply) and the daemon-start render
    // (write_config) likewise
    let t2 = Arc::new(PortTable::new());
    assert!(spine
        .adopt_ports(Arc::clone(&t2), Arc::new(|| 8))
        .await
        .is_err());
    assert!(t2.held().is_empty(), "nothing claimed is kept");
    assert!(spine.write_config().is_err());
    assert!(t2.held().is_empty(), "nothing claimed is kept");
}
