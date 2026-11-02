//! Layer 6 B3 acceptance (os/6-transport/README.md §5, §16 G2): the Meshtastic bridge
//! client against a **fake meshmud** — a TCP server in this test (std threads) that speaks
//! the `0x94 0xC3 <len hi> <len lo>` framing with prost-encoded `FromRadio` frames built from
//! the `meshtastic` crate's own types, deframes what the client writes strictly (any byte
//! outside a frame is a framing error) and records every `ToRadio`. No hardware, no
//! meshtasticd; the monotonic clock is a `ManualClock` the tests move.
//!
//! Covered (G2): one `want_config_id` per connection, never 0, 1, 69420, 69421 · replays
//! ingested without duplicates · text before `config_complete_id` buffered and mapped ·
//! the nonce-0 heartbeat on the test tick · the allowlist drops and counts everything else,
//! and a dropped second `want_config_id` wedges nothing · spacing ≥ 2.1 s · `free = 0`
//! pauses · every `Routing.Error` of §5 · `want_ack = true` and the reliable-record path
//! (implicit ack) · `key_unknown` resumes on a NodeInfo with a key · region `UNSET` ·
//! `via_mqtt` and the receive mapping · live `uplink_enabled` / `is_licensed` · a sentinel
//! PSK in nothing the adapter emits · the prost shapes 232 / 220 and `PRIVATE_APP`
//! 231 / 219 against 239 / 227 · the shim drops `ToRadio.disconnect` · Nearby parity per
//! field on the committed `FromRadio` fixture (`tests/fixtures/meshtastic/`) · undecodable
//! (JSON stub) frames are noise · capped reconnect back-off · the source scan for
//! `ToRadio` constructors. B3 review: an unheard own NodeInfo is never Live (F1) · another
//! client's partial replay does not configure the session (F2) · Nearby pushes coalesce on
//! the monotonic clock (F3) · Wi-Fi PSK / MQTT password sentinels in nothing emitted (F5) ·
//! "waiting" once per job, no terminal event dropped (F7).

use meshtastic::protobufs as pb;
use meshtastic::Message;
use node111::daemon::bridge::{
    Adapter, AdapterState, Auth, ConvKind, ConvRef, ForeignMessageState as S, Inbound, LinkState,
    Reason, Retry, SendEvent, SendHandle, SendOpts,
};
use node111::daemon::meshtastic::allowlist::{self, AllowStats, AllowlistWriter, Refused};
use node111::daemon::meshtastic::client::{ClientConfig, ManualClock, MeshtasticAdapter};
use node111::daemon::meshtastic::presence;
use node111::daemon::meshtastic::shim::{self, MeshtasticShim, ShimConfig};
use node111::daemon::panels::{Shim, ShimRequest, ShimSession};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MY: u32 = 0x0a0a_0a0a;
const KEY: [u8; 32] = [0x42; 32];
/// The sentinel PSK: a fixed 16-byte pattern that must appear nowhere.
const SENTINEL: [u8; 16] = [
    0x5a, 0xa5, 0xc3, 0x3c, 0x96, 0x69, 0x0f, 0xf0, 0x5a, 0xa5, 0xc3, 0x3c, 0x96, 0x69, 0x0f, 0xf0,
];
/// A private key the board hands every client in its security config (never kept).
const SECRET_PRIVATE_KEY: [u8; 32] = [0xe7; 32];
/// The board's Wi-Fi PSK and MQTT password (config every client is handed; never kept, F5).
const SECRET_WIFI: &str = "wifi-sentinel-Qz7xK2pW";
const SECRET_MQTT: &str = "mqtt-sentinel-Lm4vR8tN";
const BCAST: u32 = 0xffff_ffff;
const T: Duration = Duration::from_secs(5);

// ------------------------------------------------------------------ frames

fn framed(p: &[u8]) -> Vec<u8> {
    let mut v = vec![0x94, 0xc3, (p.len() >> 8) as u8, p.len() as u8];
    v.extend_from_slice(p);
    v
}

fn fr(v: pb::from_radio::PayloadVariant) -> Vec<u8> {
    framed(
        &pb::FromRadio {
            id: 0,
            payload_variant: Some(v),
        }
        .encode_to_vec(),
    )
}

use pb::from_radio::PayloadVariant as F;

fn my_info(num: u32) -> Vec<u8> {
    fr(F::MyInfo(pb::MyNodeInfo {
        my_node_num: num,
        ..Default::default()
    }))
}

fn user(num: u32, long: &str, short: &str, key: Option<[u8; 32]>, licensed: bool) -> pb::User {
    pb::User {
        id: format!("!{num:08x}"),
        long_name: long.into(),
        short_name: short.into(),
        public_key: key.map(|k| k.to_vec()).unwrap_or_default(),
        is_licensed: licensed,
        ..Default::default()
    }
}

fn node_info(ni: pb::NodeInfo) -> Vec<u8> {
    fr(F::NodeInfo(ni))
}

fn simple_node(num: u32, long: &str, key: Option<[u8; 32]>, licensed: bool) -> Vec<u8> {
    node_info(pb::NodeInfo {
        num,
        user: Some(user(num, long, "XX", key, licensed)),
        ..Default::default()
    })
}

fn channel(index: i32, name: &str, psk: &[u8], uplink: bool, role: pb::channel::Role) -> Vec<u8> {
    fr(F::Channel(pb::Channel {
        index,
        settings: Some(pb::ChannelSettings {
            psk: psk.to_vec(),
            name: name.into(),
            uplink_enabled: uplink,
            ..Default::default()
        }),
        role: role as i32,
    }))
}

fn lora(region: i32, preset: i32) -> Vec<u8> {
    fr(F::Config(pb::Config {
        payload_variant: Some(pb::config::PayloadVariant::Lora(pb::config::LoRaConfig {
            use_preset: true,
            modem_preset: preset,
            region,
            hop_limit: 3,
            tx_enabled: true,
            ..Default::default()
        })),
    }))
}

fn security() -> Vec<u8> {
    fr(F::Config(pb::Config {
        payload_variant: Some(pb::config::PayloadVariant::Security(
            pb::config::SecurityConfig {
                public_key: vec![0x11; 32],
                private_key: SECRET_PRIVATE_KEY.to_vec(),
                ..Default::default()
            },
        )),
    }))
}

fn network() -> Vec<u8> {
    fr(F::Config(pb::Config {
        payload_variant: Some(pb::config::PayloadVariant::Network(
            pb::config::NetworkConfig {
                wifi_enabled: true,
                wifi_ssid: "bench".into(),
                wifi_psk: SECRET_WIFI.into(),
                ..Default::default()
            },
        )),
    }))
}

fn mqtt() -> Vec<u8> {
    fr(F::ModuleConfig(pb::ModuleConfig {
        payload_variant: Some(pb::module_config::PayloadVariant::Mqtt(
            pb::module_config::MqttConfig {
                enabled: true,
                username: "bench".into(),
                password: SECRET_MQTT.into(),
                ..Default::default()
            },
        )),
    }))
}

fn complete(id: u32) -> Vec<u8> {
    fr(F::ConfigCompleteId(id))
}

fn mesh(p: pb::MeshPacket) -> Vec<u8> {
    fr(F::Packet(p))
}

fn data_packet(from: u32, to: u32, port: pb::PortNum, payload: Vec<u8>) -> pb::MeshPacket {
    pb::MeshPacket {
        from,
        to,
        id: 0x7000_0000 ^ from,
        payload_variant: Some(pb::mesh_packet::PayloadVariant::Decoded(pb::Data {
            portnum: port as i32,
            payload,
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn text_in(from: u32, to: u32, channel: u32, id: u32, text: &str) -> pb::MeshPacket {
    let mut p = data_packet(
        from,
        to,
        pb::PortNum::TextMessageApp,
        text.as_bytes().to_vec(),
    );
    p.channel = channel;
    p.id = id;
    p
}

fn routing(from: u32, request_id: u32, err: pb::routing::Error) -> Vec<u8> {
    let r = pb::Routing {
        variant: Some(pb::routing::Variant::ErrorReason(err as i32)),
    };
    let mut p = data_packet(from, MY, pb::PortNum::RoutingApp, r.encode_to_vec());
    if let Some(pb::mesh_packet::PayloadVariant::Decoded(d)) = &mut p.payload_variant {
        d.request_id = request_id;
    }
    mesh(p)
}

fn queue_status(res: i32, free: u32, id: u32) -> Vec<u8> {
    fr(F::QueueStatus(pb::QueueStatus {
        res,
        free,
        maxlen: 16,
        mesh_packet_id: id,
    }))
}

fn nodeinfo_packet(from: u32, u: pb::User) -> Vec<u8> {
    let mut p = data_packet(from, BCAST, pb::PortNum::NodeinfoApp, u.encode_to_vec());
    p.rx_rssi = -100;
    p.rx_time = 1_760_000_500;
    mesh(p)
}

// ------------------------------------------------------------------ the board

/// What the fake board answers a `want_config_id` with.
#[derive(Clone)]
struct Board {
    my: u32,
    licensed: bool,
    region: i32,
    /// (index, name, psk, uplink, role)
    channels: Vec<(i32, String, Vec<u8>, bool, pb::channel::Role)>,
    nodes: Vec<Vec<u8>>,
    /// Frames before `config_complete_id` (live text racing the config stream).
    before_complete: Vec<Vec<u8>>,
    /// Answer no `want_config_id` at all (a board that never completes).
    silent: bool,
    /// Leave the board's own NodeInfo out of the stream (B3 review F1).
    omit_own: bool,
}

impl Default for Board {
    fn default() -> Self {
        Board {
            my: MY,
            licensed: false,
            region: pb::config::lo_ra_config::RegionCode::Eu868 as i32,
            channels: vec![
                (0, String::new(), vec![1], false, pb::channel::Role::Primary),
                (
                    1,
                    "Ops".into(),
                    SENTINEL.to_vec(),
                    true,
                    pb::channel::Role::Secondary,
                ),
                (2, String::new(), vec![], false, pb::channel::Role::Disabled),
            ],
            nodes: vec![
                simple_node(0x0b0b_0b0b, "Peer B", Some([0xb0; 32]), false),
                simple_node(0x0c0c_0c0c, "Peer C", None, false),
            ],
            before_complete: Vec::new(),
            silent: false,
            omit_own: false,
        }
    }
}

fn config_stream(b: &Board, complete_id: u32) -> Vec<u8> {
    let mut out = b"DEBUG | 12:00:00 boot noise\r\n".to_vec();
    out.extend(my_info(b.my));
    if !b.omit_own {
        out.extend(own_node(b));
    }
    for (i, name, psk, uplink, role) in &b.channels {
        out.extend(channel(*i, name, psk, *uplink, *role));
    }
    out.extend(lora(b.region, 0));
    out.extend(security());
    out.extend(network());
    out.extend(mqtt());
    for n in &b.nodes {
        out.extend_from_slice(n);
    }
    for f in &b.before_complete {
        out.extend_from_slice(f);
    }
    out.extend(complete(complete_id));
    out
}

fn own_node(b: &Board) -> Vec<u8> {
    simple_node(b.my, "Bench board", Some([0xaa; 32]), b.licensed)
}

/// Another client's nodes-only replay (69421 on 2.7): my info, the node DB, the completion;
/// no channels and no LoRa config.
fn nodes_only_stream(b: &Board, complete_id: u32) -> Vec<u8> {
    let mut out = my_info(b.my);
    out.extend(own_node(b));
    for n in &b.nodes {
        out.extend_from_slice(n);
    }
    out.extend(complete(complete_id));
    out
}

// ------------------------------------------------------------------ the fake meshmud

struct FakeState {
    conns: Vec<(usize, TcpStream)>,
    accepted: usize,
    closed: usize,
    received: Vec<(usize, Vec<u8>)>,
    framing_errors: usize,
    board: Board,
}

struct Fake {
    addr: SocketAddr,
    st: Arc<Mutex<FakeState>>,
}

impl Fake {
    fn start(board: Board) -> Fake {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let st = Arc::new(Mutex::new(FakeState {
            conns: Vec::new(),
            accepted: 0,
            closed: 0,
            received: Vec::new(),
            framing_errors: 0,
            board,
        }));
        let shared = Arc::clone(&st);
        std::thread::spawn(move || {
            for c in l.incoming() {
                let Ok(c) = c else { break };
                let id = {
                    let mut g = shared.lock().unwrap();
                    g.accepted += 1;
                    let id = g.accepted;
                    g.conns.push((id, c.try_clone().unwrap()));
                    id
                };
                let st = Arc::clone(&shared);
                std::thread::spawn(move || serve(id, c, st));
            }
        });
        Fake { addr, st }
    }

    fn g(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.st.lock().unwrap()
    }

    fn set_board(&self, b: Board) {
        self.g().board = b;
    }

    /// Write raw bytes to every open connection (meshmud forwards to every downlink).
    fn push(&self, bytes: &[u8]) {
        for (_, c) in self.g().conns.iter_mut() {
            let _ = c.write_all(bytes);
        }
    }

    /// meshmud's uplink dropped: every downlink closes.
    fn close_all(&self) {
        for (_, c) in self.g().conns.drain(..) {
            let _ = c.shutdown(std::net::Shutdown::Both);
        }
    }

    fn accepted(&self) -> usize {
        self.g().accepted
    }

    fn raw(&self) -> Vec<(usize, Vec<u8>)> {
        self.g().received.clone()
    }

    fn decoded(&self) -> Vec<(usize, pb::ToRadio)> {
        self.raw()
            .into_iter()
            .map(|(c, p)| {
                (
                    c,
                    pb::ToRadio::decode(&p[..]).expect("only ToRadio arrives"),
                )
            })
            .collect()
    }

    fn want_configs(&self) -> Vec<(usize, u32)> {
        self.decoded()
            .into_iter()
            .filter_map(|(c, m)| match m.payload_variant {
                Some(pb::to_radio::PayloadVariant::WantConfigId(id)) => Some((c, id)),
                _ => None,
            })
            .collect()
    }

    fn texts(&self) -> Vec<pb::MeshPacket> {
        self.decoded()
            .into_iter()
            .filter_map(|(_, m)| match m.payload_variant {
                Some(pb::to_radio::PayloadVariant::Packet(p)) => Some(p),
                _ => None,
            })
            .collect()
    }

    fn heartbeats(&self) -> Vec<u32> {
        self.decoded()
            .into_iter()
            .filter_map(|(_, m)| match m.payload_variant {
                Some(pb::to_radio::PayloadVariant::Heartbeat(h)) => Some(h.nonce),
                _ => None,
            })
            .collect()
    }

    fn framing_errors(&self) -> usize {
        self.g().framing_errors
    }
}

/// Strict deframer; a `want_config_id` gets the board's config stream back.
fn serve(id: usize, mut c: TcpStream, st: Arc<Mutex<FakeState>>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = match c.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);
        while buf.len() >= 4 {
            if buf[..2] != [0x94, 0xc3] {
                st.lock().unwrap().framing_errors += 1;
                buf.clear();
                break;
            }
            let len = usize::from(u16::from_be_bytes([buf[2], buf[3]]));
            if buf.len() < 4 + len {
                break;
            }
            let p = buf[4..4 + len].to_vec();
            buf.drain(..4 + len);
            let reply = {
                let mut g = st.lock().unwrap();
                g.received.push((id, p.clone()));
                match pb::ToRadio::decode(&p[..])
                    .ok()
                    .and_then(|m| m.payload_variant)
                {
                    Some(pb::to_radio::PayloadVariant::WantConfigId(cid)) if !g.board.silent => {
                        Some(config_stream(&g.board, cid))
                    }
                    _ => None,
                }
            };
            if let Some(r) = reply {
                let _ = c.write_all(&r);
            }
        }
    }
    let mut g = st.lock().unwrap();
    g.closed += 1;
    g.conns.retain(|(i, _)| *i != id);
}

// ------------------------------------------------------------------ the harness

struct H {
    fake: Fake,
    clock: Arc<ManualClock>,
    a: Arc<MeshtasticAdapter>,
}

async fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + T;
    while !ok() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Let the client's task run (real time) without moving the manual clock.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(80)).await;
}

fn cfg(addr: SocketAddr) -> ClientConfig {
    let mut c = ClientConfig::production(addr);
    c.reconnect_initial = Duration::from_secs(1);
    c.reconnect_max = Duration::from_secs(4);
    c
}

async fn harness(board: Board) -> H {
    let fake = Fake::start(board);
    let clock = ManualClock::new();
    let a = MeshtasticAdapter::with_clock(cfg(fake.addr), KEY, clock.clone());
    a.start();
    wait_until("configured", || a.mesh_facts().configured).await;
    H { fake, clock, a }
}

impl H {
    fn channel_conv(&self, index: u32) -> ConvRef {
        let f = self.a.mesh_facts();
        let c = f
            .channels
            .iter()
            .find(|c| c.index == index)
            .expect("channel in the table");
        ConvRef {
            network: "meshtastic".into(),
            account: f.my_node.clone().unwrap(),
            kind: ConvKind::Channel,
            native_id: c.conv_id.clone(),
        }
    }

    fn dm(&self, num: u32) -> ConvRef {
        ConvRef {
            network: "meshtastic".into(),
            account: format!("!{MY:08x}"),
            kind: ConvKind::Dm,
            native_id: format!("dm:!{num:08x}"),
        }
    }

    /// Send and wait until the fake sees the text; returns its packet and the handle.
    async fn send_written(&self, conv: &ConvRef, text: &str) -> (pb::MeshPacket, SendHandle) {
        let before = self.fake.texts().len();
        let h = self.a.send(conv, text, SendOpts::default()).await;
        wait_until("text at the fake", || self.fake.texts().len() > before).await;
        (self.fake.texts().last().cloned().unwrap(), h)
    }

    async fn stop(self) {
        self.a.stop().await;
        assert!(!self.a.running());
    }
}

/// Every event the handle yields until it closes (bounded wait).
async fn all_events(h: &mut SendHandle) -> Vec<SendEvent> {
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout(T, h.events.recv()).await {
            Ok(Some(e)) => out.push(e),
            Ok(None) => return out,
            Err(_) => panic!("handle never closed; so far: {out:?}"),
        }
    }
}

/// Events available now (no wait for close).
async fn some_events(h: &mut SendHandle) -> Vec<SendEvent> {
    let mut out = Vec::new();
    while let Ok(Some(e)) = tokio::time::timeout(Duration::from_millis(150), h.events.recv()).await
    {
        out.push(e);
    }
    out
}

/// Every inbound item available now.
async fn drain(a: &MeshtasticAdapter) -> Vec<Inbound> {
    let mut out = Vec::new();
    while let Ok(i) = tokio::time::timeout(Duration::from_millis(150), a.recv()).await {
        out.push(i);
    }
    out
}

fn texts(items: &[Inbound]) -> Vec<node111::daemon::bridge::InboundText> {
    items
        .iter()
        .filter_map(|i| match i {
            Inbound::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

fn convs(items: &[Inbound]) -> Vec<(ConvRef, String)> {
    items
        .iter()
        .filter_map(|i| match i {
            Inbound::Conversation { conv, title } => Some((conv.clone(), title.clone())),
            _ => None,
        })
        .collect()
}

fn failed(reason: Reason, retry: Retry) -> SendEvent {
    SendEvent::Failed { reason, retry }
}

fn ev(state: S, evidence: &str) -> SendEvent {
    SendEvent::Evidence {
        state,
        evidence: evidence.into(),
    }
}

fn accepted(p: &pb::MeshPacket) -> SendEvent {
    SendEvent::Accepted {
        native_id: format!("{:08x}", p.id),
    }
}

// ------------------------------------------------------------------ handshake

#[tokio::test]
async fn one_want_config_per_connection_never_reserved_and_replays_change_nothing() {
    let board = Board::default();
    let h = harness(board.clone()).await;
    let w = h.fake.want_configs();
    assert_eq!(w.len(), 1, "one want_config_id");
    assert!(allowlist::config_id_ok(w[0].1), "id {}", w[0].1);
    assert_eq!(h.fake.framing_errors(), 0);
    let first = drain(&h.a).await;
    assert!(first.contains(&Inbound::Link(LinkState::Up)));
    let ups = convs(&first);
    assert_eq!(
        ups.len(),
        2,
        "two active channels, read-only from first run: {ups:?}"
    );
    assert_eq!(
        ups[0].1, "LongFast",
        "an unnamed primary is the preset's name"
    );
    assert!(ups[0].0.native_id.starts_with("ch:LongFast:"));
    assert_eq!(ups[1].1, "Ops");

    // another client's want_config_id replays the stream to every downlink, with its
    // nonce (the web client's 69420 here): ingested, nothing new, nothing re-sent
    for _ in 0..3 {
        h.fake.push(&config_stream(&board, 69_420));
    }
    settle().await;
    assert_eq!(h.fake.want_configs().len(), 1, "never re-sent in a session");
    let again = drain(&h.a).await;
    assert!(
        convs(&again).is_empty() && texts(&again).is_empty(),
        "no duplicates: {again:?}"
    );
    assert_eq!(h.a.mesh_facts().channels.len(), 2);
    assert_eq!(h.a.mesh_facts().nodes, 2);

    // a replay with a renamed channel: one upsert, the new name
    let mut renamed = board.clone();
    renamed.channels[1].1 = "Ops2".into();
    h.fake.push(&config_stream(&renamed, 69_421));
    settle().await;
    let after = drain(&h.a).await;
    let ups = convs(&after);
    assert_eq!(ups.len(), 1, "{after:?}");
    assert_eq!(ups[0].1, "Ops2");

    // meshmud's uplink drops: every downlink closes, the client reconnects after its
    // back-off and asks once more, on the new connection, with a fresh id
    h.fake.close_all();
    wait_until("backing off", || h.a.mesh_facts().counters.backoffs == 1).await;
    assert!(drain(&h.a).await.contains(&Inbound::Link(LinkState::Down)));
    h.clock.advance(Duration::from_secs(1));
    wait_until("reconnected", || h.a.mesh_facts().configured).await;
    let w = h.fake.want_configs();
    assert_eq!(w.len(), 2, "{w:?}");
    assert_ne!(w[0].0, w[1].0, "one per connection");
    assert!(w.iter().all(|(_, id)| allowlist::config_id_ok(*id)));
    assert_eq!(h.fake.accepted(), 2);
    h.stop().await;
}

#[tokio::test]
async fn text_before_config_complete_is_buffered_and_mapped() {
    let board = Board {
        before_complete: vec![
            mesh(text_in(0x0b0b_0b0b, BCAST, 1, 0x100, "early on Ops")),
            mesh(text_in(0x0b0b_0b0b, BCAST, 5, 0x101, "no such channel")),
        ],
        ..Default::default()
    };
    let h = harness(board).await;
    let items = drain(&h.a).await;
    let t = texts(&items);
    assert_eq!(t.len(), 1, "{items:?}");
    assert_eq!(t[0].body, "early on Ops");
    assert_eq!(t[0].conv, h.channel_conv(1));
    assert_eq!(t[0].auth, Auth::ChannelKey);
    assert_eq!(t[0].sender_name, "Peer B");
    assert_eq!(t[0].native_msg_id, "00000100");
    // the upserts came first, then the mapped text
    let pos_up = items
        .iter()
        .position(|i| matches!(i, Inbound::Conversation { .. }))
        .unwrap();
    let pos_text = items
        .iter()
        .position(|i| matches!(i, Inbound::Text(_)))
        .unwrap();
    assert!(pos_up < pos_text);
    assert_eq!(h.a.mesh_facts().counters.unknown_channel, 1);
    h.stop().await;
}

#[tokio::test]
async fn nonce_zero_heartbeat_on_the_tick() {
    let h = harness(Board::default()).await;
    assert!(h.fake.heartbeats().is_empty());
    h.clock.advance(Duration::from_secs(299));
    settle().await;
    assert!(h.fake.heartbeats().is_empty(), "not before 5 minutes");
    h.clock.advance(Duration::from_secs(1));
    wait_until("heartbeat", || h.fake.heartbeats().len() == 1).await;
    assert_eq!(h.fake.heartbeats(), vec![0]);
    let raw = h.fake.raw();
    assert!(
        raw.iter().any(|(_, p)| p == &[0x3a, 0x00]),
        "the exact keep-alive bytes"
    );
    h.clock.advance(Duration::from_secs(300));
    wait_until("second heartbeat", || h.fake.heartbeats().len() == 2).await;
    assert!(h.fake.heartbeats().iter().all(|n| *n == 0), "never nonce 1");
    h.stop().await;
}

// ------------------------------------------------------------------ the allowlist

#[tokio::test]
async fn the_allowlist_drops_and_counts_everything_else_and_never_wedges() {
    let fake = Fake::start(Board {
        silent: true,
        ..Default::default()
    });
    let s = tokio::net::TcpStream::connect(fake.addr).await.unwrap();
    let (_rd, wr) = s.into_split();
    let stats = Arc::new(AllowStats::new(None));
    let mut w = AllowlistWriter::new(wr, stats.clone());
    let want = |id: u32| {
        framed(
            &pb::ToRadio {
                payload_variant: Some(pb::to_radio::PayloadVariant::WantConfigId(id)),
            }
            .encode_to_vec(),
        )
    };
    let hb = |nonce: u32| {
        framed(
            &pb::ToRadio {
                payload_variant: Some(pb::to_radio::PayloadVariant::Heartbeat(pb::Heartbeat {
                    nonce,
                })),
            }
            .encode_to_vec(),
        )
    };
    let pkt = |p: pb::MeshPacket| {
        framed(
            &pb::ToRadio {
                payload_variant: Some(pb::to_radio::PayloadVariant::Packet(p)),
            }
            .encode_to_vec(),
        )
    };
    let text = node111::daemon::meshtastic::client::text_packet(BCAST, 0, 0x55, 3, b"hi");
    let mut admin = text.clone();
    if let Some(pb::mesh_packet::PayloadVariant::Decoded(d)) = &mut admin.payload_variant {
        d.portnum = pb::PortNum::AdminApp as i32;
    }
    let mut position = text.clone();
    if let Some(pb::mesh_packet::PayloadVariant::Decoded(d)) = &mut position.payload_variant {
        d.portnum = pb::PortNum::PositionApp as i32;
    }
    let mut private = text.clone();
    if let Some(pb::mesh_packet::PayloadVariant::Decoded(d)) = &mut private.payload_variant {
        d.portnum = pb::PortNum::PrivateApp as i32;
    }
    let refused: Vec<(Vec<u8>, Refused)> = vec![
        (want(1), Refused::WantConfigReserved),
        (want(69_420), Refused::WantConfigReserved),
        (want(69_421), Refused::WantConfigReserved),
        (framed(&[0x20, 0x01]), Refused::Disconnect),
        (framed(&[0x2a, 0x02, 0x08, 0x01]), Refused::Xmodem),
        (framed(&[0x32, 0x00]), Refused::MqttProxy),
        (hb(1), Refused::HeartbeatNonce),
        (pkt(admin), Refused::Packet),
        (pkt(position), Refused::Packet),
        (pkt(private), Refused::Packet),
        (b"not a frame".to_vec(), Refused::NotFramed),
    ];
    // allowed: the connection's one want_config_id, then a second one (dropped), then a
    // heartbeat and a text: both still pass — a dropped want_config wedges nothing
    allowlist::write_all_flush(&mut w, &want(0x1234_5678))
        .await
        .unwrap();
    for (bytes, _) in &refused {
        allowlist::write_all_flush(&mut w, bytes).await.unwrap();
    }
    allowlist::write_all_flush(&mut w, &want(0x2222_2222))
        .await
        .unwrap();
    allowlist::write_all_flush(&mut w, &hb(0)).await.unwrap();
    allowlist::write_all_flush(&mut w, &pkt(text.clone()))
        .await
        .unwrap();
    wait_until("allowed frames at the fake", || fake.raw().len() == 3).await;
    settle().await;
    assert_eq!(fake.framing_errors(), 0);
    let got = fake.decoded();
    assert_eq!(got.len(), 3, "only the allowed three");
    assert!(matches!(
        got[0].1.payload_variant,
        Some(pb::to_radio::PayloadVariant::WantConfigId(0x1234_5678))
    ));
    assert!(matches!(
        got[1].1.payload_variant,
        Some(pb::to_radio::PayloadVariant::Heartbeat(pb::Heartbeat {
            nonce: 0
        }))
    ));
    assert_eq!(fake.texts(), vec![text]);
    let by = stats.by_reason();
    for (_, why) in &refused {
        assert!(by.get(why).copied().unwrap_or(0) >= 1, "{why:?} counted");
    }
    assert_eq!(by[&Refused::WantConfigRepeat], 1);
    assert_eq!(stats.passed(), 3);
    assert_eq!(stats.dropped(), refused.len() as u64 + 1);
}

// ------------------------------------------------------------------ the send queue

#[tokio::test]
async fn texts_are_spaced_at_least_2_1_s_on_the_monotonic_clock() {
    let h = harness(Board::default()).await;
    let conv = h.channel_conv(0);
    let mut handles = Vec::new();
    for i in 0..3 {
        handles.push(h.a.send(&conv, &format!("t{i}"), SendOpts::default()).await);
    }
    wait_until("first text", || h.fake.texts().len() == 1).await;
    h.clock.advance(Duration::from_millis(2000));
    settle().await;
    assert_eq!(h.fake.texts().len(), 1, "2.0 s is too early");
    h.clock.advance(Duration::from_millis(100));
    wait_until("second text at 2.1 s", || h.fake.texts().len() == 2).await;
    h.clock.advance(Duration::from_millis(2099));
    settle().await;
    assert_eq!(h.fake.texts().len(), 2);
    h.clock.advance(Duration::from_millis(1));
    wait_until("third text", || h.fake.texts().len() == 3).await;
    let t = h.fake.texts();
    let ids: std::collections::HashSet<u32> = t.iter().map(|p| p.id).collect();
    assert_eq!(ids.len(), 3, "packet ids never reused");
    assert!(t.iter().all(|p| p.id != 0 && p.from == 0 && p.want_ack));
    assert_eq!(t[0].to, BCAST);
    assert_eq!(h.a.mesh_facts().counters.texts_written, 3);
    h.stop().await;
}

#[tokio::test]
async fn queue_status_free_zero_pauses_the_queue_and_the_boards_status_resumes_it() {
    let h = harness(Board::default()).await;
    h.fake.push(&queue_status(0, 0, 0));
    wait_until("paused", || h.a.mesh_facts().paused).await;
    let mut handle =
        h.a.send(&h.channel_conv(0), "wait", SendOpts::default())
            .await;
    let evs = some_events(&mut handle).await;
    assert_eq!(evs, vec![ev(S::Queued, "waiting for the radio")]);
    h.clock.advance(Duration::from_secs(3));
    settle().await;
    assert!(
        h.fake.texts().is_empty(),
        "nothing goes out while the board's queue is full"
    );
    // keep-alive while paused (a nonce-0 heartbeat, backed off); the board's own status
    // change resumes the queue (B3 review F6: no answer to the heartbeat is assumed)
    h.clock.advance(Duration::from_secs(2));
    wait_until("probe heartbeat", || !h.fake.heartbeats().is_empty()).await;
    h.fake.push(&queue_status(0, 4, 0));
    wait_until("resumed and written", || h.fake.texts().len() == 1).await;
    assert!(!h.a.mesh_facts().paused);
    h.stop().await;
}

#[tokio::test]
async fn want_ack_and_the_reliable_record_path() {
    let h = harness(Board::default()).await;
    // channel text: want_ack on, QueueStatus res 0 ⇒ accepted after the settle, then the
    // implicit ack (a routing NONE from our own node) ⇒ relayed, and the handle closes
    let (p, mut handle) = h.send_written(&h.channel_conv(1), "to Ops").await;
    assert!(p.want_ack, "want_ack = true for channel text (§5)");
    assert_eq!((p.to, p.channel, p.from), (BCAST, 1, 0));
    let seen = h.a.mesh_facts().counters.queue_statuses;
    h.fake.push(&queue_status(0, 15, p.id));
    wait_until("status seen", || {
        h.a.mesh_facts().counters.queue_statuses > seen
    })
    .await;
    assert!(
        some_events(&mut handle).await.is_empty(),
        "not before the settle"
    );
    h.clock.advance(Duration::from_millis(999));
    assert!(
        some_events(&mut handle).await.is_empty(),
        "not before the settle"
    );
    h.clock.advance(Duration::from_millis(1));
    let evs = some_events(&mut handle).await;
    assert_eq!(
        evs,
        vec![accepted(&p), ev(S::Sent, "accepted by the radio")]
    );
    h.fake.push(&routing(MY, p.id, pb::routing::Error::None));
    assert_eq!(
        all_events(&mut handle).await,
        vec![ev(S::Relayed, "a neighbour rebroadcast it")]
    );

    // DM: implicit ack ⇒ "relayed, not confirmed", then the recipient's real ack
    h.clock.advance(Duration::from_millis(2100));
    let (p, mut handle) = h.send_written(&h.dm(0x0b0b_0b0b), "hi B").await;
    assert!(p.want_ack, "want_ack = true for DMs");
    assert_eq!(p.to, 0x0b0b_0b0b);
    h.fake.push(&routing(MY, p.id, pb::routing::Error::None));
    let evs = some_events(&mut handle).await;
    assert_eq!(
        evs,
        vec![accepted(&p), ev(S::Relayed, "relayed, not confirmed")]
    );
    h.fake
        .push(&routing(0x0b0b_0b0b, p.id, pb::routing::Error::None));
    assert_eq!(
        all_events(&mut handle).await,
        vec![ev(S::Delivered, "the recipient confirmed")]
    );
    h.stop().await;
}

#[tokio::test]
async fn a_rate_limit_after_a_res_0_status_is_not_accepted() {
    let h = harness(Board::default()).await;
    let (p, mut handle) = h.send_written(&h.channel_conv(0), "busy").await;
    // the firmware queues a res-0 QueueStatus and then RATE_LIMIT_EXCEEDED
    let mut both = queue_status(0, 15, p.id);
    both.extend(routing(MY, p.id, pb::routing::Error::RateLimitExceeded));
    h.fake.push(&both);
    assert_eq!(
        all_events(&mut handle).await,
        vec![failed(Reason::RateLimited, Retry::Later)]
    );
    h.stop().await;
}

#[tokio::test]
async fn every_routing_error_of_section_5_maps_as_specified() {
    use pb::routing::Error as E;
    let h = harness(Board::default()).await;
    let cases: Vec<(E, bool, Vec<SendEvent>)> = {
        let pre = |r| vec![failed(r, Retry::Never)];
        let later = |r| vec![failed(r, Retry::Later)];
        vec![
            (E::NoChannel, false, pre(Reason::NoChannel)),
            (E::TooLarge, false, pre(Reason::TooLarge)),
            (E::NotAuthorized, false, pre(Reason::NotAuthorized)),
            (E::PkiFailed, true, pre(Reason::PkiFailed)),
            (E::GotNak, true, pre(Reason::GotNak)),
            (E::DutyCycleLimit, false, later(Reason::DutyCycle)),
            (E::RateLimitExceeded, false, later(Reason::RateLimited)),
            (E::PkiSendFailPublicKey, true, later(Reason::KeyUnknown)),
            (E::PkiUnknownPubkey, true, later(Reason::KeyUnknown)),
            // post-air: the channel keeps `sent`, "transmitted, not confirmed"
            (E::NoRoute, false, Vec::new()),
            (E::Timeout, false, Vec::new()),
            (E::MaxRetransmit, false, Vec::new()),
            (E::NoResponse, false, Vec::new()),
            // … and a DM's post-air NAK is later
            (E::NoRoute, true, Vec::new()),
            (E::Timeout, true, Vec::new()),
            (E::MaxRetransmit, true, Vec::new()),
            (E::NoResponse, true, Vec::new()),
        ]
    };
    for (err, dm, want) in cases {
        h.clock.advance(Duration::from_millis(2100));
        let conv = if dm {
            h.dm(0x0b0b_0b0b)
        } else {
            h.channel_conv(0)
        };
        let (p, mut handle) = h.send_written(&conv, &format!("{err:?}")).await;
        h.fake.push(&routing(MY, p.id, err));
        let got = all_events(&mut handle).await;
        let want = if want.is_empty() {
            let reason = match err {
                E::NoRoute => Reason::NoRoute,
                E::Timeout => Reason::Timeout,
                E::MaxRetransmit => Reason::MaxRetransmit,
                _ => Reason::NoResponse,
            };
            let mut v = vec![accepted(&p), ev(S::Sent, "transmitted, not confirmed")];
            if dm {
                v.push(failed(reason, Retry::Later));
            }
            v
        } else {
            want
        };
        assert_eq!(got, want, "{err:?} (dm: {dm})");
    }
    h.stop().await;
}

#[tokio::test]
async fn key_unknown_resumes_on_a_nodeinfo_with_a_key() {
    let h = harness(Board::default()).await;
    // Peer C is in the node DB without a key
    let c = 0x0c0c_0c0c;
    let (p, mut handle) = h.send_written(&h.dm(c), "hello C").await;
    h.fake
        .push(&routing(MY, p.id, pb::routing::Error::PkiSendFailPublicKey));
    assert_eq!(
        all_events(&mut handle).await,
        vec![failed(Reason::KeyUnknown, Retry::Later)]
    );
    let _ = drain(&h.a).await;
    // a NodeInfo from C without a key: nothing resumes
    h.fake
        .push(&nodeinfo_packet(c, user(c, "Peer C", "C", None, false)));
    settle().await;
    assert!(convs(&drain(&h.a).await).is_empty());
    // … with its key: the "key learned" adapter event (a DM upsert the core retries on)
    h.fake.push(&nodeinfo_packet(
        c,
        user(c, "Peer C", "C", Some([0xc0; 32]), false),
    ));
    let items = drain(&h.a).await;
    let ups = convs(&items);
    assert_eq!(ups.len(), 1, "{items:?}");
    assert_eq!(ups[0].0, h.dm(c));
    assert_eq!(ups[0].1, "Peer C");
    // the core's retry goes out with a fresh packet id
    h.clock.advance(Duration::from_millis(2100));
    let (p2, _h2) = h.send_written(&h.dm(c), "hello C").await;
    assert_ne!(p2.id, p.id);
    h.stop().await;
}

#[tokio::test]
async fn region_unset_fails_before_air_and_other_pre_air_refusals() {
    let h = harness(Board {
        region: 0,
        ..Default::default()
    })
    .await;
    assert_eq!(h.a.caps().state, AdapterState::NotSet);
    assert_eq!(h.a.mesh_facts().region.as_deref(), Some("UNSET"));
    assert_eq!(h.a.facts().region_set, Some(false));
    let mut handle = h.a.send(&h.channel_conv(0), "x", SendOpts::default()).await;
    assert_eq!(
        all_events(&mut handle).await,
        vec![failed(Reason::RegionUnset, Retry::Never)]
    );
    settle().await;
    assert!(
        h.fake.texts().is_empty(),
        "the board transmits nothing; we hand it nothing"
    );
    h.stop().await;

    let h = harness(Board::default()).await;
    assert_eq!(h.a.caps().state, AdapterState::Live);
    let refusals = [
        // an unknown channel, another account, a group, our own node, too large
        (
            ConvRef {
                native_id: "ch:LongFast:0000000000000000".into(),
                ..h.channel_conv(0)
            },
            "x".to_string(),
            Reason::NoChannel,
        ),
        (
            ConvRef {
                account: "!0d0d0d0d".into(),
                ..h.channel_conv(0)
            },
            "x".to_string(),
            Reason::NoChannel,
        ),
        (
            ConvRef {
                kind: ConvKind::Group,
                ..h.channel_conv(0)
            },
            "x".to_string(),
            Reason::NoChannel,
        ),
        (h.dm(MY), "x".to_string(), Reason::NoChannel),
        (h.channel_conv(0), "x".repeat(233), Reason::TooLarge),
        (h.dm(0x0b0b_0b0b), "x".repeat(221), Reason::TooLarge),
    ];
    for (conv, text, reason) in refusals {
        let mut handle = h.a.send(&conv, &text, SendOpts::default()).await;
        assert_eq!(
            all_events(&mut handle).await,
            vec![failed(reason, Retry::Never)],
            "{conv:?}"
        );
    }
    // the caps fit exactly
    let (p, _h1) = h.send_written(&h.channel_conv(0), &"c".repeat(232)).await;
    assert_eq!(decoded_len(&p), 232);
    h.clock.advance(Duration::from_millis(2100));
    let (p, _h2) = h.send_written(&h.dm(0x0b0b_0b0b), &"d".repeat(220)).await;
    assert_eq!(decoded_len(&p), 220);
    h.stop().await;
}

fn decoded_len(p: &pb::MeshPacket) -> usize {
    match &p.payload_variant {
        Some(pb::mesh_packet::PayloadVariant::Decoded(d)) => d.payload.len(),
        _ => 0,
    }
}

#[tokio::test]
async fn a_licensed_board_allows_232_byte_dms_and_facts_follow_live_changes() {
    let h = harness(Board::default()).await;
    let f = h.a.mesh_facts();
    assert_eq!(f.licensed, Some(false));
    assert_eq!(f.long_name.as_deref(), Some("Bench board"));
    let uplinks: Vec<bool> = f.channels.iter().map(|c| c.uplink_enabled).collect();
    assert_eq!(uplinks, vec![false, true]);
    assert!(f.channels[0].default_key, "the primary uses the public key");
    assert!(!f.channels[1].default_key);
    let af = h.a.facts();
    assert_eq!(af.accounts.len(), 1);
    assert_eq!(af.accounts[0].id, "!0a0a0a0a");
    assert!(!af.accounts[0].licensed);
    assert_eq!(af.region_set, Some(true));
    assert!(!af.ids_globally_unique);

    // the board becomes licensed and the primary is uplinked (another client changed
    // them; the replay carries the new facts)
    let mut b = Board {
        licensed: true,
        ..Default::default()
    };
    b.channels[0].3 = true;
    h.fake.push(&config_stream(&b, 69_420));
    wait_until("licensed", || h.a.mesh_facts().licensed == Some(true)).await;
    let f = h.a.mesh_facts();
    assert!(f.channels.iter().all(|c| c.uplink_enabled));
    let af = h.a.facts();
    assert!(af.accounts[0].licensed);
    assert!(af.channels.iter().all(|c| c.uplink));
    // licensed: the firmware skips PKI, so a DM may carry 232 B
    let (p, _h) = h.send_written(&h.dm(0x0b0b_0b0b), &"l".repeat(232)).await;
    assert_eq!(decoded_len(&p), 232);
    // … and back via a live NodeInfo packet from our own node
    h.fake.push(&nodeinfo_packet(
        MY,
        user(MY, "Bench board", "BB", Some([0xaa; 32]), false),
    ));
    wait_until("unlicensed", || h.a.mesh_facts().licensed == Some(false)).await;
    // the channel's uplink switch goes off again in a later replay
    h.fake.push(&config_stream(&Board::default(), 69_421));
    wait_until("uplink off", || {
        !h.a.mesh_facts().channels[0].uplink_enabled
    })
    .await;
    h.stop().await;
}

// ------------------------------------------------------------------ B3 review

/// F1 (MAJOR): the HAM consent fact failed open: with the board's own NodeInfo unheard,
/// `licensed` was unknown, the account read as unlicensed and `caps` said Live, so a
/// readable send went through on what may be a licensed board.
#[tokio::test]
async fn f1_an_unheard_own_nodeinfo_never_reports_live_nor_lets_a_readable_send_through() {
    let board = Board {
        omit_own: true,
        licensed: true,
        ..Default::default()
    };
    let h = harness(board.clone()).await;
    let items = drain(&h.a).await;
    assert!(
        !items.contains(&Inbound::Link(LinkState::Up)),
        "no link up without the consent facts: {items:?}"
    );
    for round in 0..2 {
        let f = h.a.mesh_facts();
        assert!(f.configured, "our own stream completed");
        assert_eq!(f.licensed, None);
        assert_eq!(h.a.caps().state, AdapterState::NotSet, "round {round}");
        assert!(
            f.not_live.as_deref().unwrap_or("").contains("own node"),
            "{:?}",
            f.not_live
        );
        // unknown is not "unlicensed": the restrictive reading
        assert!(h.a.facts().accounts[0].licensed);
        for conv in [h.channel_conv(0), h.channel_conv(1), h.dm(0x0b0b_0b0b)] {
            let mut hd = h.a.send(&conv, "readable", SendOpts::default()).await;
            assert_eq!(
                all_events(&mut hd).await,
                vec![failed(Reason::AdapterDown, Retry::Later)]
            );
        }
        h.clock.advance(Duration::from_secs(10));
        settle().await;
        assert!(h.fake.texts().is_empty(), "nothing went on air");
        // another client's replay without the own node changes nothing
        h.fake.push(&config_stream(&board, 69_420));
        settle().await;
    }
    // the own NodeInfo arrives (licensed): live, the link comes up, the fact is exact
    h.fake.push(&own_node(&board));
    wait_until("live", || h.a.caps().state == AdapterState::Live).await;
    assert!(drain(&h.a).await.contains(&Inbound::Link(LinkState::Up)));
    assert_eq!(h.a.mesh_facts().licensed, Some(true));
    assert_eq!(h.a.mesh_facts().not_live, None);
    assert!(h.a.facts().accounts[0].licensed);
    h.send_written(&h.channel_conv(0), "now").await;
    h.stop().await;
}

/// F2: another client's partial replay (its nodes-only 69421 pre-empting ours on the first
/// connect) closed with any `config_complete_id` and marked the session configured with no
/// channel table and no LoRa config.
#[tokio::test]
async fn f2_another_clients_partial_replay_does_not_configure_the_session() {
    let board = Board {
        silent: true,
        ..Default::default()
    };
    let fake = Fake::start(board.clone());
    let clock = ManualClock::new();
    let a = MeshtasticAdapter::with_clock(cfg(fake.addr), KEY, clock.clone());
    a.start();
    wait_until("asked once", || fake.want_configs().len() == 1).await;
    fake.push(&nodes_only_stream(&board, 69_421));
    wait_until("partial seen", || {
        a.mesh_facts().counters.partial_completions == 1
    })
    .await;
    let f = a.mesh_facts();
    assert!(!f.configured, "a partial table configures nothing");
    assert!(f.channels.is_empty() && f.region.is_none());
    assert_eq!(a.caps().state, AdapterState::NotSet);
    assert!(
        f.not_live
            .as_deref()
            .unwrap_or("")
            .contains("config stream"),
        "{:?}",
        f.not_live
    );
    let conv = ConvRef {
        network: "meshtastic".into(),
        account: format!("!{MY:08x}"),
        kind: ConvKind::Dm,
        native_id: "dm:!0b0b0b0b".into(),
    };
    let mut hd = a.send(&conv, "too early", SendOpts::default()).await;
    assert_eq!(
        all_events(&mut hd).await,
        vec![failed(Reason::AdapterDown, Retry::Later)]
    );
    assert!(!drain(&a).await.contains(&Inbound::Link(LinkState::Up)));
    // a later full replay (the web client's 69420) completes the session
    fake.push(&config_stream(&board, 69_420));
    wait_until("configured", || a.mesh_facts().configured).await;
    assert_eq!(a.caps().state, AdapterState::Live);
    assert_eq!(a.mesh_facts().channels.len(), 2);
    assert!(drain(&a).await.contains(&Inbound::Link(LinkState::Up)));
    assert_eq!(fake.want_configs().len(), 1, "never asked again");
    a.stop().await;
}

/// F3: every packet from any node rebuilt and pushed the whole Nearby source to the core.
#[tokio::test]
async fn f3_a_burst_of_packets_yields_one_nearby_push_per_window() {
    let h = harness(Board::default()).await;
    let first = drain(&h.a).await;
    let nearby = |items: &[Inbound]| {
        items
            .iter()
            .filter(|i| matches!(i, Inbound::Nearby(_)))
            .count()
    };
    assert_eq!(nearby(&first), 1, "the configured node DB goes out at once");
    let v0 = h.a.nearby_version();
    // a burst inside the window, each packet read on its own (the core drains between)
    let mut pushed = 0;
    for i in 0..8u32 {
        let from = 0x0d00_0000 + i;
        let p = data_packet(
            from,
            BCAST,
            pb::PortNum::PositionApp,
            pb::Position {
                latitude_i: Some(520_000_000 + i as i32),
                longitude_i: Some(130_000_000),
                ..Default::default()
            }
            .encode_to_vec(),
        );
        h.fake.push(&mesh(p));
        wait_until("node heard", || h.a.nearby_version() > v0 + u64::from(i)).await;
        pushed += nearby(&drain(&h.a).await);
    }
    assert_eq!(pushed, 0, "coalesced inside the window");
    assert!(
        h.a.nearby_version() >= v0 + 8,
        "the module's tick still sees every change"
    );
    // the window passes: the next read after a packet carries the whole burst once
    h.clock.advance(Duration::from_secs(5));
    h.fake.push(&nodeinfo_packet(
        0x0b0b_0b0b,
        user(0x0b0b_0b0b, "Peer B", "PB", Some([0xb0; 32]), false),
    ));
    let items = drain(&h.a).await;
    assert_eq!(nearby(&items), 1, "{items:?}");
    let Some(Inbound::Nearby(v)) = items.iter().find(|i| matches!(i, Inbound::Nearby(_))) else {
        unreachable!()
    };
    let nodes = v["nodes"].as_array().map_or(0, Vec::len);
    assert_eq!(nodes, 2 + 8, "the burst's nodes are in the one push");
    assert_eq!(h.a.mesh_facts().counters.nearby_pushes, 2);
    h.stop().await;
}

/// F7: "waiting for the radio" was re-emitted on every pause, so a job that sat through
/// many `free = 0` cycles filled its 16-slot channel and its `Accepted` was dropped.
#[tokio::test]
async fn f7_waiting_is_noted_once_per_job_and_a_terminal_event_is_never_dropped() {
    let h = harness(Board::default()).await;
    // a first text holds the pacing window (the manual clock stands still)
    let (_first, _h1) = h.send_written(&h.channel_conv(0), "first").await;
    h.fake.push(&queue_status(0, 0, 0));
    wait_until("paused", || h.a.mesh_facts().paused).await;
    let mut hd =
        h.a.send(&h.channel_conv(1), "second", SendOpts::default())
            .await;
    // twenty pause / resume cycles while it waits; nobody reads the handle meanwhile
    for _ in 0..20 {
        h.fake.push(&queue_status(0, 4, 0));
        h.fake.push(&queue_status(0, 0, 0));
    }
    let n = h.a.mesh_facts().counters.queue_statuses;
    wait_until("cycles read", || {
        h.a.mesh_facts().counters.queue_statuses >= n + 40
    })
    .await;
    assert_eq!(h.fake.texts().len(), 1, "still held");
    h.fake.push(&queue_status(0, 4, 0));
    wait_until("resumed", || !h.a.mesh_facts().paused).await;
    h.clock.advance(Duration::from_millis(2100));
    wait_until("second written", || h.fake.texts().len() == 2).await;
    let p = h.fake.texts()[1].clone();
    h.fake.push(&queue_status(0, 4, p.id));
    settle().await;
    h.clock.advance(Duration::from_secs(1));
    settle().await;
    h.fake.push(&routing(MY, p.id, pb::routing::Error::None));
    let evs = all_events(&mut hd).await;
    assert_eq!(
        evs,
        vec![
            ev(S::Queued, "waiting for the radio"),
            accepted(&p),
            ev(S::Sent, "accepted by the radio"),
            ev(S::Relayed, "a neighbour rebroadcast it"),
        ]
    );
    h.stop().await;
}

// ------------------------------------------------------------------ receive

#[tokio::test]
async fn receive_maps_channels_dms_tapbacks_echoes_and_via_mqtt() {
    let h = harness(Board::default()).await;
    let _ = drain(&h.a).await;
    let b = 0x0b0b_0b0b;
    let mut over_mqtt = text_in(b, BCAST, 0, 0x201, "via the internet");
    over_mqtt.via_mqtt = true;
    over_mqtt.rx_snr = 6.25;
    over_mqtt.rx_rssi = -91;
    over_mqtt.hop_start = 3;
    over_mqtt.hop_limit = 1;
    over_mqtt.rx_time = 1_760_000_000;
    if let Some(pb::mesh_packet::PayloadVariant::Decoded(d)) = &mut over_mqtt.payload_variant {
        d.bitfield = Some(1);
    }
    let mut dm = text_in(b, MY, 0, 0x202, "a PKI DM");
    dm.pki_encrypted = true;
    dm.public_key = vec![0xb0; 32];
    let legacy = text_in(b, MY, 0, 0x203, "not PKI");
    let mut tapback = text_in(b, BCAST, 0, 0x204, "👍");
    if let Some(pb::mesh_packet::PayloadVariant::Decoded(d)) = &mut tapback.payload_variant {
        d.emoji = 1;
        d.reply_id = 0x201;
    }
    let echo = text_in(MY, BCAST, 1, 0x205, "my own");
    let elsewhere = text_in(b, 0x0c0c_0c0c, 0, 0x206, "for C");
    let mut bad_utf8 = text_in(b, BCAST, 0, 0x207, "");
    if let Some(pb::mesh_packet::PayloadVariant::Decoded(d)) = &mut bad_utf8.payload_variant {
        d.payload = vec![b'o', b'k', 0xff];
    }
    for p in [over_mqtt, dm, legacy, tapback, echo, elsewhere, bad_utf8] {
        h.fake.push(&mesh(p));
    }
    settle().await;
    let items = drain(&h.a).await;
    let t = texts(&items);
    assert_eq!(t.len(), 5, "{t:?}");
    // channel text: channel-key, the meta the bridge core keeps
    assert_eq!(t[0].conv, h.channel_conv(0));
    assert_eq!(t[0].auth, Auth::ChannelKey);
    assert_eq!(t[0].sender_native, "!0b0b0b0b");
    assert_eq!(t[0].meta["via_mqtt"], true);
    assert_eq!(t[0].meta["snr"], 6.25);
    assert_eq!(t[0].meta["rssi"], -91);
    assert_eq!(t[0].meta["hops"], 2);
    assert_eq!(t[0].meta["ok_to_mqtt"], true);
    assert_eq!(t[0].claim_ts, Some(1_760_000_000));
    // a PKI DM to the board is pki; its conversation is upserted first
    assert_eq!(t[1].conv, h.dm(b));
    assert_eq!(t[1].auth, Auth::Pki);
    assert_eq!(t[1].meta["via_mqtt"], false);
    assert!(convs(&items)
        .iter()
        .any(|(c, title)| c == &h.dm(b) && title == "Peer B"));
    // a non-PKI DM (a licensed board's) is the channel's key at best
    assert_eq!(t[2].auth, Auth::ChannelKey);
    // the board's own node: flagged as our echo
    assert_eq!(t[3].meta["own_echo"], true);
    assert_eq!(t[3].conv, h.channel_conv(1));
    // invalid UTF-8 is replaced, never refused
    assert_eq!(t[4].body, "ok\u{fffd}");
    let c = h.a.mesh_facts().counters;
    assert_eq!(c.tapbacks, 1, "a tapback is not a message");
    assert_eq!(c.not_for_us, 1);
    h.stop().await;
}

#[tokio::test]
async fn undecodable_frames_are_noise_and_change_nothing() {
    let h = harness(Board::default()).await;
    // what the Layer 5 JSON stub harness sends: JSON where protobuf would be
    h.fake
        .push(&framed(br#"{"my_node_info":{"num":168430090}}"#));
    h.fake.push(&framed(br#"{"rebooted":true}"#));
    h.fake.push(b"INFO | plain debug text\r\n");
    wait_until("noise counted", || {
        h.a.mesh_facts().counters.noise_frames == 2
    })
    .await;
    let f = h.a.mesh_facts();
    assert!(f.connected && f.configured);
    let (p, _h) = h.send_written(&h.channel_conv(0), "still fine").await;
    assert_eq!(decoded_len(&p), 10);
    h.stop().await;
}

#[tokio::test]
async fn reconnect_back_off_doubles_and_is_capped() {
    let h = harness(Board::default()).await;
    // from now on the board never completes a config stream, so nothing resets the wait
    h.fake.set_board(Board {
        silent: true,
        ..Default::default()
    });
    for (n, wait) in [1u64, 2, 4, 4, 4].into_iter().enumerate() {
        // connections so far: the first one plus one per earlier round
        let accepts = n + 1;
        h.fake.close_all();
        wait_until("backing off", || {
            h.a.mesh_facts().counters.backoffs == n as u64 + 1
        })
        .await;
        if wait > 1 {
            h.clock.advance(Duration::from_secs(wait - 1));
            settle().await;
            assert_eq!(h.fake.accepted(), accepts, "not before {wait} s");
        }
        h.clock.advance(Duration::from_secs(1));
        wait_until("reconnected", || h.fake.accepted() == accepts + 1).await;
        wait_until("connected", || h.a.mesh_facts().connected).await;
    }
    // each connection asked once
    let per_conn: std::collections::HashMap<usize, usize> =
        h.fake
            .want_configs()
            .iter()
            .fold(Default::default(), |mut m, (c, _)| {
                *m.entry(*c).or_default() += 1;
                m
            });
    assert!(per_conn.values().all(|n| *n == 1), "{per_conn:?}");
    h.stop().await;
}

#[tokio::test]
async fn stopping_gives_queued_jobs_back_and_leaves_nothing_running() {
    let h = harness(Board::default()).await;
    h.fake.push(&queue_status(0, 0, 0));
    wait_until("paused", || h.a.mesh_facts().paused).await;
    let mut handle =
        h.a.send(&h.channel_conv(0), "held", SendOpts::default())
            .await;
    assert_eq!(
        some_events(&mut handle).await,
        vec![ev(S::Queued, "waiting for the radio")]
    );
    let _ = drain(&h.a).await;
    h.a.stop().await;
    assert_eq!(
        all_events(&mut handle).await,
        vec![failed(Reason::AdapterDown, Retry::Later)]
    );
    assert!(!h.a.running());
    assert_eq!(h.a.caps().state, AdapterState::NotSet);
    assert!(drain(&h.a).await.contains(&Inbound::Link(LinkState::Down)));
    // not running: a send is handed straight back
    let mut handle = h.a.send(&h.channel_conv(0), "x", SendOpts::default()).await;
    assert_eq!(
        all_events(&mut handle).await,
        vec![failed(Reason::AdapterDown, Retry::Later)]
    );
    wait_until("the fake saw the close", || h.fake.g().conns.is_empty()).await;
}

#[tokio::test]
async fn no_client_runs_without_the_service() {
    // the Meshtastic module with no binding: an attached client never starts (the Pi
    // floor: no task, socket or timer while service `meshtastic` is off)
    let dir = tempfile::tempdir().unwrap();
    let layout = node111::daemon::DaemonLayout {
        root: dir.path().to_path_buf(),
        etc: dir.path().join("etc"),
    };
    let m = node111::daemon::meshtastic::Meshtastic::new(
        node111::daemon::meshtastic::MeshtasticConfig::production(&layout, false),
        Arc::new(node111::daemon::ports::PortTable::new()),
    );
    let fake = Fake::start(Board::default());
    let c = MeshtasticAdapter::with_clock(cfg(fake.addr), KEY, ManualClock::new());
    m.attach_client(Arc::clone(&c));
    m.tick_once().await;
    settle().await;
    assert!(!c.running());
    assert_eq!(fake.accepted(), 0);
    assert!(m.nearby_sources().is_empty());
}

// ------------------------------------------------------------------ the sentinel

/// The secrets as hex and as decimal digit runs.
fn secret_digits(bytes: &[u8]) -> (String, String) {
    (
        bytes.iter().map(|b| format!("{b:02x}")).collect(),
        bytes.iter().map(|b| b.to_string()).collect(),
    )
}

/// Every way the sentinel PSK (or the board's private key) could show up in text, in any
/// formatting: the text's hex digits and its decimal digits are each run together (so
/// `[5a, a5, …]`, `0x5a 0xa5`, `5AA5…` and `[90, 165, …]` all match), plus base64.
fn assert_clean(what: &str, text: &str) {
    let lower = text.replace("0x", "").replace("0X", "").to_lowercase();
    let hex_soup: String = lower.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let dec_soup: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    for (name, secret) in [
        ("PSK", &SENTINEL[..]),
        ("private key", &SECRET_PRIVATE_KEY[..]),
    ] {
        let (hex, dec) = secret_digits(secret);
        // the first 12 bytes are plenty (a random match is about 2^-96)
        let (hex, dec) = (&hex[..24], &dec[..dec.len().min(30)]);
        assert!(
            !hex_soup.contains(hex),
            "{what} carries the {name} (hex): {text}"
        );
        assert!(
            !dec_soup.contains(dec),
            "{what} carries the {name} (decimal): {text}"
        );
        let b64 = node111::daemon::dashboard::routes::base64(secret);
        assert!(
            !text.contains(&b64[..16]),
            "{what} carries the {name} (base64): {text}"
        );
    }
    for (name, secret) in [("Wi-Fi PSK", SECRET_WIFI), ("MQTT password", SECRET_MQTT)] {
        assert!(!text.contains(secret), "{what} carries the {name}: {text}");
        let b64 = node111::daemon::dashboard::routes::base64(secret.as_bytes());
        assert!(
            !text.contains(&b64[..16]),
            "{what} carries the {name} (base64): {text}"
        );
    }
}

fn assert_clean_bytes(what: &str, bytes: &[u8]) {
    assert!(
        !bytes.windows(SENTINEL.len()).any(|w| w == SENTINEL),
        "{what} carries the sentinel PSK"
    );
    assert!(
        !bytes.windows(8).any(|w| w == &SECRET_PRIVATE_KEY[..8]),
        "{what} carries the private key"
    );
    for s in [SECRET_WIFI, SECRET_MQTT] {
        assert!(
            !bytes.windows(s.len()).any(|w| w == s.as_bytes()),
            "{what} carries a Wi-Fi / MQTT secret"
        );
    }
}

#[tokio::test]
async fn a_sentinel_psk_appears_in_nothing_the_adapter_emits() {
    let board = Board {
        before_complete: vec![mesh(text_in(0x0b0b_0b0b, BCAST, 1, 0x300, "on Ops"))],
        ..Default::default()
    };
    let h = harness(board.clone()).await;
    // replays carry the PSK again; they change nothing and leak nothing
    h.fake.push(&config_stream(&board, 69_420));
    h.fake.push(&framed(b"\x00\x01garbage"));
    let (p, mut handle) = h.send_written(&h.channel_conv(1), "to Ops").await;
    h.fake
        .push(&routing(MY, p.id, pb::routing::Error::NoChannel));
    let evs = all_events(&mut handle).await;
    settle().await;
    let items = drain(&h.a).await;
    assert!(!texts(&items).is_empty(), "the capture saw texts");
    assert!(!h.a.recent_log().is_empty(), "the capture saw log lines");

    // the PSK was read: the Ops conversation differs from a default-key channel's id
    let ops = h.channel_conv(1);
    assert!(ops.native_id.starts_with("ch:Ops:"));

    let mut captured: Vec<(String, String)> = Vec::new();
    for i in &items {
        captured.push(("inbound (json)".into(), serde_json::to_string(i).unwrap()));
        captured.push(("inbound (debug)".into(), format!("{i:?}")));
    }
    for e in &evs {
        captured.push(("send event".into(), format!("{e:?}")));
    }
    let facts = h.a.mesh_facts();
    captured.push((
        "facts (json)".into(),
        serde_json::to_string(&facts).unwrap(),
    ));
    captured.push(("facts (debug)".into(), format!("{facts:?}")));
    captured.push((
        "adapter facts".into(),
        serde_json::to_string(&h.a.facts()).unwrap(),
    ));
    captured.push(("caps".into(), serde_json::to_string(&h.a.caps()).unwrap()));
    captured.push(("adapter (debug)".into(), format!("{:?}", h.a)));
    captured.push(("reader document".into(), h.a.reader_document().to_string()));
    captured.push((
        "nearby source".into(),
        serde_json::to_string(&h.a.nearby_source()).unwrap(),
    ));
    captured.push(("allow stats".into(), format!("{:?}", h.a.allow_stats())));
    for l in h.a.recent_log() {
        captured.push(("log line".into(), l));
    }
    for (what, text) in &captured {
        assert_clean(what, text);
        assert_clean_bytes(what, text.as_bytes());
    }
    // nor anything the client sent the board
    for (_, p) in h.fake.raw() {
        assert_clean_bytes("a ToRadio", &p);
    }
    h.stop().await;
}

// ------------------------------------------------------------------ shapes

#[test]
fn the_prost_encode_fixes_the_shapes_against_the_firmware_limits() {
    // the firmware checks the encoded `Data` against 255 - 16 = 239 B, minus 12 B for PKI
    // (227 B), and adds the 2-byte `bitfield` itself (R01 §3.4, R06 G7; src, not hardware)
    const FIRMWARE: usize = 239;
    const FIRMWARE_PKI: usize = 227;
    let data = |port: pb::PortNum, n: usize| pb::Data {
        portnum: port as i32,
        payload: vec![b'x'; n],
        bitfield: Some(1),
        ..Default::default()
    };
    for (port, plain, pki) in [
        (pb::PortNum::TextMessageApp, 232usize, 220usize),
        (pb::PortNum::PrivateApp, 231, 219),
    ] {
        assert_eq!(data(port, plain).encoded_len(), FIRMWARE, "{port:?} plain");
        assert_eq!(data(port, plain + 1).encoded_len(), FIRMWARE + 1);
        assert_eq!(data(port, pki).encoded_len(), FIRMWARE_PKI, "{port:?} PKI");
        assert_eq!(data(port, pki + 1).encoded_len(), FIRMWARE_PKI + 1);
    }
    // the adapter's caps are those figures
    use node111::daemon::meshtastic::client as c;
    assert_eq!(
        (
            c::TEXT_CHANNEL_MAX,
            c::TEXT_PKI_DM_MAX,
            c::TEXT_LICENSED_DM_MAX
        ),
        (232, 220, 232)
    );
    // a max-size text ToRadio frames under the 512-byte stream limit and passes the
    // allowlist
    let p = c::text_packet(BCAST, 7, u32::MAX, 7, &[b'y'; 232]);
    let t = pb::ToRadio {
        payload_variant: Some(pb::to_radio::PayloadVariant::Packet(p)),
    }
    .encode_to_vec();
    assert!(shim::frame(&t).is_ok());
    assert_eq!(allowlist::classify(&t, true), Ok(allowlist::Allowed::Text));
}

// ------------------------------------------------------------------ the shim

#[tokio::test]
async fn the_shim_drops_toradio_disconnect() {
    let fake = Fake::start(Board {
        silent: true,
        ..Default::default()
    });
    let s = MeshtasticShim::new(ShimConfig::production(fake.addr));
    let sess = ShimSession {
        panel: "meshtastic".into(),
        session: "viewer-1".into(),
        peer: "127.0.0.1".parse().unwrap(),
    };
    let put = |body: &'static [u8]| ShimRequest {
        method: axum::http::Method::PUT,
        path: "/api/v1/toradio".into(),
        query: None,
        content_type: Some(shim::PROTOBUF.into()),
        body: axum::body::Bytes::from_static(body),
    };
    // a raw disconnect (what a hostile page could PUT): 200, discarded, nothing opened
    let r = s.handle(&sess, put(&[0x20, 0x01])).await.unwrap();
    assert_eq!(r.status, axum::http::StatusCode::OK);
    assert!(r.body.is_empty());
    assert_eq!(s.disconnects_dropped(), 1);
    settle().await;
    assert_eq!(fake.accepted(), 0, "no downlink opened for it");
    // everything else is still reframed verbatim
    let r = s.handle(&sess, put(&[0x3a, 0x00])).await.unwrap();
    assert_eq!(r.status, axum::http::StatusCode::OK);
    wait_until("heartbeat reframed", || fake.raw().len() == 1).await;
    assert_eq!(fake.raw()[0].1, vec![0x3a, 0x00]);
    // a second disconnect on the open session: still dropped, the session stays
    let _ = s.handle(&sess, put(&[0x20, 0x01])).await.unwrap();
    assert_eq!(s.disconnects_dropped(), 2);
    settle().await;
    assert_eq!(fake.raw().len(), 1);
    assert_eq!(s.sessions(), 1);
    s.close_all();
}

// ------------------------------------------------------------------ Nearby parity

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meshtastic")
}

const PARITY_MY: u32 = 0x0a0a_0a0a;

fn parity_node(
    num: u32,
    u: Option<pb::User>,
    pos: Option<pb::Position>,
    snr: f32,
    last_heard: u32,
    util: Option<f32>,
    hops: Option<u32>,
) -> Vec<u8> {
    node_info(pb::NodeInfo {
        num,
        user: u,
        position: pos,
        snr,
        last_heard,
        device_metrics: util.map(|u| pb::DeviceMetrics {
            channel_utilization: Some(u),
            battery_level: Some(80),
            ..Default::default()
        }),
        hops_away: hops,
        ..Default::default()
    })
}

fn pos(lat_i: i32, lon_i: i32, alt: Option<i32>) -> Option<pb::Position> {
    Some(pb::Position {
        latitude_i: Some(lat_i),
        longitude_i: Some(lon_i),
        altitude: alt,
        ..Default::default()
    })
}

/// The committed fixture: one board's config stream and live packets (see
/// `tests/fixtures/meshtastic/README.md`).
fn parity_frames() -> Vec<u8> {
    let a = 0x1b2c_3d4e;
    let b = 0x00b0_b0b0;
    let c = 0x00c0_c0c0;
    let d = 0x00d0_d0d0;
    let e = 0x00e0_e0e0;
    let f = 0x00f0_f0f0;
    let mut out = Vec::new();
    out.extend(my_info(PARITY_MY));
    out.extend(parity_node(
        PARITY_MY,
        Some(user(
            PARITY_MY,
            "Bench board",
            "BB",
            Some([0xaa; 32]),
            false,
        )),
        pos(515_000_000, -1_000_000, Some(10)),
        0.0,
        1_760_000_000,
        Some(3.5),
        None,
    ));
    out.extend(parity_node(
        a,
        Some(user(a, "Hilltop relay", "HT", Some([0xa1; 32]), false)),
        pos(515_074_000, -1_278_000, Some(35)),
        5.5,
        1_760_000_000,
        Some(7.25),
        Some(1),
    ));
    out.extend(parity_node(
        b,
        Some(user(
            b,
            "Ab\u{202e}cdefghijklmnopqrstuvwxyz0123456789XYZ",
            "\u{1b}]0;x",
            Some([0xb1; 32]),
            false,
        )),
        pos(0, 0, None),
        -7.3,
        1_760_000_100,
        None,
        Some(0),
    ));
    out.extend(parity_node(c, None, None, 0.0, 0, None, None));
    out.extend(parity_node(
        d,
        Some(user(d, "Ünïcødé Ñame 😀😀", "Ü😀", None, true)),
        pos(-337_000_000, 1_512_000_000, Some(200_000)),
        0.0,
        0,
        Some(33.333),
        Some(9),
    ));
    out.extend(channel(0, "", &[1], false, pb::channel::Role::Primary));
    out.extend(channel(
        1,
        "Ops",
        &SENTINEL,
        true,
        pb::channel::Role::Secondary,
    ));
    out.extend(lora(pb::config::lo_ra_config::RegionCode::Eu868 as i32, 0));
    out.extend(security());
    out.extend(complete(0x00c0_ffee));
    // live packets after the config stream
    let mut t = text_in(a, BCAST, 0, 0x1111, "hello mesh");
    t.rx_snr = 6.25;
    t.rx_rssi = -91;
    t.rx_time = 1_760_000_200;
    t.hop_start = 3;
    t.hop_limit = 2;
    out.extend(mesh(t));
    let p = pb::Position {
        latitude_i: Some(482_000_000),
        longitude_i: Some(23_000_000),
        altitude: Some(120),
        ..Default::default()
    };
    out.extend(mesh(data_packet(
        b,
        BCAST,
        pb::PortNum::PositionApp,
        p.encode_to_vec(),
    )));
    let tel = pb::Telemetry {
        time: 0,
        variant: Some(pb::telemetry::Variant::DeviceMetrics(pb::DeviceMetrics {
            channel_utilization: Some(12.5),
            battery_level: Some(90),
            ..Default::default()
        })),
    };
    out.extend(mesh(data_packet(
        c,
        BCAST,
        pb::PortNum::TelemetryApp,
        tel.encode_to_vec(),
    )));
    let mut ni = data_packet(
        e,
        BCAST,
        pb::PortNum::NodeinfoApp,
        user(e, "Echo", "E", Some([0xe1; 32]), false).encode_to_vec(),
    );
    ni.rx_rssi = -120;
    ni.rx_time = 1_760_000_300;
    out.extend(mesh(ni));
    let mut dm = text_in(f, PARITY_MY, 0, 0x2222, "a DM from a stranger");
    dm.pki_encrypted = true;
    dm.rx_snr = -2.5;
    out.extend(mesh(dm));
    let r = pb::Routing {
        variant: Some(pb::routing::Variant::ErrorReason(0)),
    };
    let mut rt = data_packet(a, PARITY_MY, pb::PortNum::RoutingApp, r.encode_to_vec());
    if let Some(pb::mesh_packet::PayloadVariant::Decoded(dd)) = &mut rt.payload_variant {
        dd.request_id = 0x3333;
    }
    out.extend(mesh(rt));
    let tel2 = pb::Telemetry {
        time: 0,
        variant: Some(pb::telemetry::Variant::DeviceMetrics(pb::DeviceMetrics {
            battery_level: Some(70),
            ..Default::default()
        })),
    };
    out.extend(mesh(data_packet(
        a,
        BCAST,
        pb::PortNum::TelemetryApp,
        tel2.encode_to_vec(),
    )));
    out
}

#[test]
fn the_parity_fixture_is_current() {
    let path = fixtures().join("parity.fromradio");
    let want = parity_frames();
    if std::env::var_os("MESHTASTIC_FIXTURE_WRITE").is_some() {
        std::fs::create_dir_all(fixtures()).unwrap();
        std::fs::write(&path, &want).unwrap();
    }
    let got = std::fs::read(&path).expect("committed fixture");
    assert_eq!(got, want, "regenerate with MESHTASTIC_FIXTURE_WRITE=1");
}

/// Cut a name to 24 UTF-8 bytes on a character boundary (the protobuf limit; the reader
/// cuts at 40).
fn cut24(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if out.len() + c.len_utf8() > 24 {
            break;
        }
        out.push(c);
    }
    out.trim_end().to_string()
}

const FIELDS: [&str; 11] = [
    "id",
    "num",
    "long_name",
    "short_name",
    "pubkey",
    "snr",
    "rssi",
    "hops_away",
    "last_heard_hint",
    "position",
    "channel_utilization",
];

fn compare_field(field: &str, num: u64, got: &Value, want: &Value) {
    if field.ends_with("_name") {
        let g = got.as_str().unwrap_or_default();
        let w = want.as_str().unwrap_or_default();
        assert_eq!(cut24(g), cut24(w), "node {num:#x} {field}");
    } else {
        assert_eq!(got, want, "node {num:#x} {field}");
    }
}

#[tokio::test]
async fn nearby_parity_per_field_on_the_committed_fixture() {
    let frames = std::fs::read(fixtures().join("parity.fromradio")).unwrap();
    let expected: Value =
        serde_json::from_slice(&std::fs::read(fixtures().join("parity.expected.json")).unwrap())
            .unwrap();
    // the fake answers the want_config_id with the fixture verbatim (its complete id is
    // not ours: any complete id ends a config stream)
    let fake = Fake::start(Board {
        silent: true,
        ..Default::default()
    });
    let clock = ManualClock::new();
    let a = MeshtasticAdapter::with_clock(cfg(fake.addr), KEY, clock);
    a.set_source("devabc", "Bench radio");
    a.start();
    wait_until("connected", || !fake.want_configs().is_empty()).await;
    fake.push(&frames);
    wait_until("fixture ingested", || {
        a.reader_document()["nodes"]
            .as_array()
            .is_some_and(|n| n.len() == 6)
            && a.mesh_facts().configured
    })
    .await;
    settle().await;

    // 1. the reader-format rows, field by field
    let got = a.reader_document();
    assert_eq!(got["schema"], 1);
    let g = got["nodes"].as_array().unwrap();
    let w = expected["nodes"].as_array().unwrap();
    assert_eq!(g.len(), w.len(), "same nodes: {got}");
    for (gn, wn) in g.iter().zip(w) {
        let num = wn["num"].as_u64().unwrap();
        assert_eq!(gn["num"], wn["num"]);
        for field in FIELDS {
            compare_field(field, num, &gn[field], &wn[field]);
        }
        let extra: Vec<&String> = gn
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| !FIELDS.contains(&k.as_str()))
            .collect();
        assert!(extra.is_empty(), "no field outside the schema: {extra:?}");
    }

    // 2. what Nearby publishes: the reader's file through the daemon's ingest vs the
    //    client's feed, per field
    let from_reader = presence::parse(&serde_json::to_vec(&expected).unwrap()).unwrap();
    let from_client = a.nearby_nodes();
    assert_eq!(from_client.len(), from_reader.len());
    for (c, r) in from_client.iter().zip(&from_reader) {
        let cv = serde_json::to_value(c).unwrap();
        let rv = serde_json::to_value(r).unwrap();
        for (k, v) in rv.as_object().unwrap() {
            compare_field(k, r.num.into(), &cv[k], v);
        }
    }
    let src = serde_json::to_value(a.nearby_source()).unwrap();
    assert_eq!(src["kind"], "meshtastic");
    assert_eq!(src["id"], "meshtastic:devabc");
    assert_eq!(src["label"], "heard via Meshtastic on Bench radio");

    // 3. the same fixture also yields the two inbound texts (one channel, one PKI DM)
    let items = drain(&a).await;
    let t = texts(&items);
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].body, "hello mesh");
    assert_eq!(t[1].auth, Auth::Pki);
    let nearby: Vec<&Value> = items
        .iter()
        .filter_map(|i| match i {
            Inbound::Nearby(v) => Some(v),
            _ => None,
        })
        .collect();
    assert!(
        !nearby.is_empty(),
        "the Nearby update is coalesced into recv"
    );
    a.stop().await;
}

// ------------------------------------------------------------------ the source scan

/// §5 "a grep test fails the build on any other `ToRadio` constructor": in `src/`, only
/// `daemon/meshtastic/client.rs` builds a `ToRadio` (exactly three constructors), and the
/// only `ToRadio` variants named anywhere are the three allowed ones. The test modules of
/// `client.rs` and `allowlist.rs` (which build refused frames to test the drop) are
/// skipped; every other file is scanned whole.
#[test]
fn no_other_toradio_constructor_in_src() {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    assert!(files.len() > 50, "the scan sees the tree");
    let ours = [
        "daemon/meshtastic/client.rs",
        "daemon/meshtastic/allowlist.rs",
    ];
    let mut constructors = Vec::new();
    for f in &files {
        let rel = f
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(f).unwrap();
        let body = if ours.contains(&rel.as_str()) {
            let cut = text
                .find("#[cfg(test)]\nmod tests")
                .expect("our files end with their test module");
            text[..cut].to_string()
        } else {
            text
        };
        let flat: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
        for (i, _) in flat.match_indices("ToRadio {") {
            constructors.push((rel.clone(), flat[i.saturating_sub(40)..i + 9].to_string()));
        }
        for bad in [
            "ToRadio{",
            "ToRadio::default",
            "ToRadio::new",
            "to_radio::*",
        ] {
            assert!(!flat.contains(bad), "{rel}: `{bad}`");
        }
        for (i, _) in flat.match_indices("to_radio::") {
            let rest = &flat[i + "to_radio::".len()..];
            assert!(
                [
                    "PayloadVariant::Packet(",
                    "PayloadVariant::WantConfigId(",
                    "PayloadVariant::Heartbeat("
                ]
                .iter()
                .any(|ok| rest.starts_with(ok)),
                "{rel}: a ToRadio variant outside the allowlist: to_radio::{}",
                &rest[..rest.len().min(40)]
            );
        }
    }
    assert_eq!(constructors.len(), 3, "{constructors:?}");
    assert!(constructors
        .iter()
        .all(|(f, _)| f == "daemon/meshtastic/client.rs"));
}
