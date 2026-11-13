//! Layer 7 §4.1–§4.4 (row D5; the node111 half of G11): TLS 1.3 with raw
//! public keys for paired-hub links, through M6's `tcp-lan` stream seam.
//!
//! Two (or more) real `TcpLan` adapters on loopback, each with
//! `relay::tls::wrap` over its own pair table and a link key in its own
//! `secrets/`; raw rustls peers stand in for the misbehaving far ends (a
//! wrong key, an X.509 certificate, TLS 1.2, an ECDSA key). "Before any 111
//! byte" is checked from the far end: a raw TLS peer that reaches the
//! application layer reads nothing, and a raw TCP peer reads nothing at all.
//! Loopback is never a peer in production (`refuse_loopback`), and it is not
//! private (§4.1 rule 1), so these links run TLS.
//!
//! Fixtures (`tests/fixtures/l7-D5/`, made once with OpenSSL 3.0):
//! `ecdsa-p256.pk8.der` (`openssl genpkey -algorithm EC -pkeyopt
//! ec_paramgen_curve:P-256`, then `openssl pkcs8 -topk8 -nocrypt -outform
//! DER`) and `x509-ed25519.der` (`openssl
//! req -x509 -new -subj /CN=111-d5-fixture -days 36500 -outform DER` over the
//! RFC 8032 §7.1 TEST 1 key as PKCS#8).

use id111::announce::Announce;
use id111::keyset::{Keyset, NodeId};
use mesh111::link::tcp_lan::{
    encode_frame, read_frame, read_hello, StreamWrap, TcpLan, TcpLanConfig,
};
use mesh111::link::tcp_lan::{HELLO, T_ANNOUNCE};
use mesh111::link::{AdmitAll, Dialer, Frame, Link, LinkEvent, LinkId, PeerAddr};
use node111::daemon::fabric::settings::{LinkSpki, PairLink, PairTable};
use node111::daemon::relay::linkkey::{self, LinkKey};
use node111::daemon::relay::tls;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::AlwaysResolvesClientRawPublicKeys;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::AlwaysResolvesServerRawPublicKeys;
use rustls::sign::CertifiedKey;
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme,
    SupportedProtocolVersion,
};
use sha2::{Digest, Sha256};
use std::alloc::{GlobalAlloc, Layout, System};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

const WAIT: Duration = Duration::from_secs(10);
/// PKCS#8 v1 prefix of an Ed25519 key (RFC 8410 §7), then the 32-byte seed.
const PKCS8_ED25519: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];
/// RFC 8032 §7.1 TEST 1: the key inside `x509-ed25519.der`.
const RFC8032_SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";

fn fixture(name: &str) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/l7-D5")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// A test identity: its NodeId and stored announce bytes.
fn ident(label: u8) -> (NodeId, Vec<u8>) {
    let mut b = [0u8; 64];
    b[..32].copy_from_slice(&Sha256::digest([b"d5-sign".as_slice(), &[label]].concat()));
    b[32..].copy_from_slice(&Sha256::digest([b"d5-ecdh".as_slice(), &[label]].concat()));
    let k = Keyset::from_bytes(&b).unwrap();
    let ann = Announce::create(&k, 1, &[1, 0, 0, 0]).unwrap().encode();
    (k.node_id(), ann)
}

fn unhex32(s: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

/// An Ed25519 raw-public-key identity from a seed: the SPKI as the one
/// "certificate" (RFC 7250).
fn ed25519(seed: &[u8; 32]) -> Arc<CertifiedKey> {
    let mut der = PKCS8_ED25519.to_vec();
    der.extend_from_slice(seed);
    let key = rustls::crypto::ring::sign::any_eddsa_type(&PrivatePkcs8KeyDer::from(der)).unwrap();
    let spki = key.public_key().unwrap().as_ref().to_vec();
    Arc::new(CertifiedKey::new(vec![CertificateDer::from(spki)], key))
}

fn spki_of(ck: &CertifiedKey) -> LinkSpki {
    LinkSpki::from_der(ck.cert[0].as_ref()).unwrap()
}

/// The seed a hub's `secrets/link-key` holds (to impersonate it).
fn seed_of(root: &Path) -> [u8; 32] {
    unhex32(std::fs::read_to_string(linkkey::path(root)).unwrap().trim())
}

/// One node: a `TcpLan` with the D5 wrap over its own pair table.
struct Hub {
    _dir: tempfile::TempDir,
    root: PathBuf,
    id: NodeId,
    ann: Vec<u8>,
    pairs: PairTable,
    lan: TcpLan,
    events: mpsc::UnboundedReceiver<LinkEvent>,
}

impl Hub {
    /// `wrapped = false`: a member built before Layer 7 (no wrap at all).
    fn new(label: u8, wrapped: bool) -> Hub {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let (id, ann) = ident(label);
        let pairs: PairTable = Arc::new(RwLock::new(Vec::new()));
        let mut cfg = TcpLanConfig::new(ann.clone());
        cfg.refuse_loopback = false; // several nodes on one host
        cfg.handshake_timeout = Duration::from_secs(5);
        cfg.backoff_min = Duration::from_millis(100);
        cfg.backoff_max = Duration::from_millis(400);
        // A short read queue, so a slow reader backs up into the sockets.
        cfg.inbound_queue = 16;
        if wrapped {
            cfg.wrap = tls::wrap(&root.join("fabric"), &pairs);
            assert!(cfg.wrap.is_some());
        }
        let (tx, events) = mpsc::unbounded_channel();
        let lan = TcpLan::new(LinkId::new("tcp-lan"), cfg, tx, Arc::new(AdmitAll));
        Hub {
            _dir: dir,
            root,
            id,
            ann,
            pairs,
            lan,
            events,
        }
    }

    /// A hub with a link key (its first pairing minted it).
    fn keyed(label: u8) -> Hub {
        let h = Hub::new(label, true);
        LinkKey::load_or_mint(&h.root).unwrap();
        h
    }

    fn spki(&self) -> LinkSpki {
        *LinkKey::load(&self.root).unwrap().unwrap().spki()
    }

    /// Record a pair (the daemon's half, M5's `PairLink`) in the live table.
    fn pair(&self, node_id: NodeId, address: Option<SocketAddr>, pin: Option<LinkSpki>) {
        self.pairs.write().unwrap().push(PairLink {
            address,
            link_spki: pin,
            ..PairLink::new(node_id)
        });
    }

    async fn listen(&self) -> SocketAddr {
        self.lan
            .listen("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap()
    }

    /// The next connection reported: (peer, outbound, expected).
    async fn connected(&mut self) -> (PeerAddr, bool, Option<NodeId>) {
        loop {
            match tokio::time::timeout(WAIT, self.events.recv()).await {
                Ok(Some(LinkEvent::Connected {
                    peer,
                    outbound,
                    expected,
                    ..
                })) => return (peer, outbound, expected),
                Ok(Some(LinkEvent::Disconnected { .. })) => continue,
                other => panic!("no connection: {other:?}"),
            }
        }
    }

    /// No connection reported within `ms`.
    async fn quiet(&mut self, ms: u64) {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
        while let Ok(Some(ev)) = tokio::time::timeout_at(deadline, self.events.recv()).await {
            assert!(
                !matches!(ev, LinkEvent::Connected { .. }),
                "a connection was reported: {ev:?}"
            );
        }
        assert_eq!(self.lan.connection_count(), 0);
    }
}

/// A recording TCP relay in front of `target`: every byte each way.
struct Tap {
    addr: SocketAddr,
    up: Arc<Mutex<Vec<u8>>>,
    down: Arc<Mutex<Vec<u8>>>,
}

async fn tap(target: SocketAddr) -> Tap {
    async fn pump(
        mut r: tokio::net::tcp::OwnedReadHalf,
        mut w: tokio::net::tcp::OwnedWriteHalf,
        rec: Arc<Mutex<Vec<u8>>>,
    ) {
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = match r.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            rec.lock().unwrap().extend_from_slice(&buf[..n]);
            if w.write_all(&buf[..n]).await.is_err() {
                break;
            }
        }
        let _ = w.shutdown().await;
    }
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let t = Tap {
        addr: l.local_addr().unwrap(),
        up: Arc::default(),
        down: Arc::default(),
    };
    let (up, down) = (t.up.clone(), t.down.clone());
    tokio::spawn(async move {
        while let Ok((c, _)) = l.accept().await {
            let Ok(s) = TcpStream::connect(target).await else {
                continue;
            };
            let (cr, cw) = c.into_split();
            let (sr, sw) = s.into_split();
            tokio::spawn(pump(cr, sw, up.clone()));
            tokio::spawn(pump(sr, cw, down.clone()));
        }
    });
    t
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

// ---------------------------------------------------------------- raw peers

/// Accepts any certificate and any signature: a far end that checks nothing
/// (so every refusal in these tests is ours). `rpk` asks for a raw key.
#[derive(Debug)]
struct AnyPeer {
    rpk: bool,
}

fn all_schemes() -> Vec<SignatureScheme> {
    rustls::crypto::ring::default_provider()
        .signature_verification_algorithms
        .supported_schemes()
}

impl ServerCertVerifier for AnyPeer {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        all_schemes()
    }
    fn requires_raw_public_keys(&self) -> bool {
        self.rpk
    }
}

impl ClientCertVerifier for AnyPeer {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn client_auth_mandatory(&self) -> bool {
        false
    }
    fn verify_client_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        all_schemes()
    }
    fn requires_raw_public_keys(&self) -> bool {
        self.rpk
    }
}

fn ring() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// A raw client presenting `id` as a raw public key (TLS 1.3).
fn rpk_client(id: Arc<CertifiedKey>) -> ClientConfig {
    ClientConfig::builder_with_provider(ring())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyPeer { rpk: true }))
        .with_client_cert_resolver(Arc::new(AlwaysResolvesClientRawPublicKeys::new(id)))
}

/// A raw X.509 peer's identity: the fixture certificate and its key.
fn x509_identity() -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
    let mut der = PKCS8_ED25519.to_vec();
    der.extend_from_slice(&unhex32(RFC8032_SEED));
    (
        vec![CertificateDer::from(fixture("x509-ed25519.der"))],
        PrivateKeyDer::Pkcs8(der.into()),
    )
}

/// A raw client presenting the X.509 fixture over `versions`; `rpk`: it
/// asks for the server's raw key (so only its own certificate is X.509).
fn x509_client(versions: &[&'static SupportedProtocolVersion], rpk: bool) -> ClientConfig {
    let (chain, key) = x509_identity();
    ClientConfig::builder_with_provider(ring())
        .with_protocol_versions(versions)
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyPeer { rpk }))
        .with_client_auth_cert(chain, key)
        .unwrap()
}

/// An ECDSA P-256 raw-public-key identity (the fixture key).
fn ecdsa() -> Arc<CertifiedKey> {
    let key = rustls::crypto::ring::sign::any_ecdsa_type(&PrivateKeyDer::Pkcs8(
        fixture("ecdsa-p256.pk8.der").into(),
    ))
    .unwrap();
    let spki = key.public_key().unwrap().as_ref().to_vec();
    Arc::new(CertifiedKey::new(vec![CertificateDer::from(spki)], key))
}

/// Run `cfg` against `at`; every plaintext byte the far end sends before it
/// closes or refuses (empty: refused before any 111 byte).
async fn raw_attempt(at: SocketAddr, cfg: ClientConfig) -> Vec<u8> {
    let tcp = TcpStream::connect(at).await.unwrap();
    let name = ServerName::IpAddress(at.ip().into());
    let mut tls = match tokio::time::timeout(
        WAIT,
        tokio_rustls::TlsConnector::from(Arc::new(cfg)).connect(name, tcp),
    )
    .await
    .expect("the handshake ends")
    {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let mut got = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        match tokio::time::timeout(WAIT, tls.read(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) => return got,
            Ok(Ok(n)) => got.extend_from_slice(&buf[..n]),
            Err(_) => panic!("the far end kept the connection open"),
        }
    }
}

/// A raw TLS server for one connection: `Ok(bytes read)` when its handshake
/// completed, `Err` when it failed.
async fn raw_server(
    cfg: ServerConfig,
) -> (SocketAddr, tokio::task::JoinHandle<Result<Vec<u8>, ()>>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = l.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (tcp, _) = l.accept().await.unwrap();
        let mut tls = tokio_rustls::TlsAcceptor::from(Arc::new(cfg))
            .accept(tcp)
            .await
            .map_err(|_| ())?;
        let mut got = Vec::new();
        let mut buf = [0u8; 512];
        while let Ok(Ok(n)) = tokio::time::timeout(WAIT, tls.read(&mut buf)).await {
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
        Ok(got)
    });
    (at, task)
}

fn rpk_server(id: Arc<CertifiedKey>) -> ServerConfig {
    ServerConfig::builder_with_provider(ring())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_client_cert_verifier(Arc::new(AnyPeer { rpk: true }))
        .with_cert_resolver(Arc::new(AlwaysResolvesServerRawPublicKeys::new(id)))
}

fn x509_server(versions: &[&'static SupportedProtocolVersion]) -> ServerConfig {
    let (chain, key) = x509_identity();
    ServerConfig::builder_with_provider(ring())
        .with_protocol_versions(versions)
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .unwrap()
}

/// Hub `b` dials the raw server at `at` as pair `far`, pinned to `pin`; the
/// dial is refused (no connection), and the raw server never completes a
/// handshake that reads a 111 byte.
async fn refused_by_the_dialer(
    b: &mut Hub,
    far: NodeId,
    pin: LinkSpki,
    at: SocketAddr,
    server: tokio::task::JoinHandle<Result<Vec<u8>, ()>>,
) {
    b.pair(far, Some(at), Some(pin));
    b.lan.maintain(at, Some(far));
    let seen = tokio::time::timeout(WAIT, server)
        .await
        .expect("the server ends")
        .unwrap();
    assert!(
        seen.is_err() || seen == Ok(Vec::new()),
        "a 111 byte arrived: {seen:?}"
    );
    b.quiet(300).await;
    b.lan.release(at);
}

// ---------------------------------------------------------------- the tests

/// §4.1 rule 1, §4.3, §4.4: two hubs pinned to each other's link keys meet
/// over TLS 1.3 RPK through M6's seam; the 111 wire runs inside unchanged
/// (both announces, a frame each way); on the wire there are TLS records
/// only, and no cleartext 111 byte, NodeId or announce (§4.7).
#[tokio::test]
async fn the_pinned_handshake_runs_end_to_end_through_the_seam() {
    let mut a = Hub::keyed(1);
    let mut b = Hub::keyed(2);
    let at = a.listen().await;
    let t = tap(at).await;
    // Pins recorded after the wraps were built: the live table counts.
    a.pair(b.id, None, Some(b.spki()));
    b.pair(a.id, Some(t.addr), Some(a.spki()));
    b.lan.maintain(t.addr, Some(a.id));

    let (to_a, out_b, expected) = b.connected().await;
    assert!(out_b);
    assert_eq!(expected, Some(a.id));
    let (to_b, out_a, _) = a.connected().await;
    assert!(!out_a);
    assert_eq!(
        a.lan.recv().await,
        (to_b.clone(), Frame::announce(b.ann.clone()))
    );
    assert_eq!(
        b.lan.recv().await,
        (to_a.clone(), Frame::announce(a.ann.clone()))
    );

    b.lan
        .send(to_a.clone(), Frame::sealed(vec![7; 300]))
        .await
        .unwrap();
    assert_eq!(
        a.lan.recv().await,
        (to_b.clone(), Frame::sealed(vec![7; 300]))
    );
    a.lan
        .send(to_b.clone(), Frame::sealed(vec![9; 40]))
        .await
        .unwrap();
    assert_eq!(
        b.lan.recv().await,
        (to_a.clone(), Frame::sealed(vec![9; 40]))
    );

    let up = t.up.lock().unwrap().clone();
    let down = t.down.lock().unwrap().clone();
    assert_eq!((up[0], down[0]), (0x16, 0x16), "TLS records both ways");
    for wire in [&up, &down] {
        assert!(!contains(wire, &HELLO), "no cleartext hello");
        assert!(!contains(wire, &a.ann[..40]) && !contains(wire, &b.ann[..40]));
        assert!(!contains(wire, a.id.as_bytes()) && !contains(wire, b.id.as_bytes()));
        assert!(!contains(wire, &[7; 300]));
    }
    a.lan.shutdown();
    b.lan.shutdown();
}

/// A bulk run over a pinned link: 320 frames of 60,000 B (19 MB) A → B,
/// read slowly so the sockets back up; every frame arrives, in order, and
/// none waits for a keepalive. (Whether a write meets a full socket is the
/// kernel's choice, so this run cannot force the tail `RpkStream` pushes
/// out; that mechanism is unit-tested in `relay::tls`.)
#[tokio::test]
async fn a_bulk_run_never_waits_for_a_keepalive() {
    let mut a = Hub::keyed(1);
    let mut b = Hub::keyed(2);
    let at = a.listen().await;
    a.pair(b.id, None, Some(b.spki()));
    b.pair(a.id, Some(at), Some(a.spki()));
    b.lan.maintain(at, Some(a.id));
    let (to_a, _, _) = b.connected().await;
    let (to_b, _, _) = a.connected().await;
    a.lan.recv().await;
    b.lan.recv().await;

    let frames: Vec<Vec<u8>> = (0..320u16).map(|i| vec![(i % 251) as u8; 60_000]).collect();
    let sender = {
        let (lan, to_b, frames) = (a.lan.clone(), to_b.clone(), frames.clone());
        tokio::spawn(async move {
            for f in frames {
                lan.send(to_b.clone(), Frame::sealed(f)).await.unwrap();
            }
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    for f in &frames {
        tokio::time::sleep(Duration::from_millis(2)).await;
        let got = tokio::time::timeout(WAIT, b.lan.recv())
            .await
            .expect("no stall");
        assert_eq!(got, (to_a.clone(), Frame::sealed(f.clone())));
    }
    sender.await.unwrap();
    a.lan.shutdown();
    b.lan.shutdown();
}

/// A bulk run both ways at once over a pinned link: 160 frames of 60,000 B
/// (9.6 MB) each way, both read slowly. Each side's reader keeps polling
/// the stream (frames arrive) while its writer meets a full socket, so the
/// reader's flush takes the writer's socket wakeup; `RpkStream` hands it on,
/// and every frame arrives both ways, in order, without a stall. (Built
/// without the hand-over, this run stalled A → B at frame 69: the writer
/// task stayed parked for good.)
#[tokio::test]
async fn a_bulk_run_both_ways_never_stalls() {
    let mut a = Hub::keyed(1);
    let mut b = Hub::keyed(2);
    let at = a.listen().await;
    a.pair(b.id, None, Some(b.spki()));
    b.pair(a.id, Some(at), Some(a.spki()));
    b.lan.maintain(at, Some(a.id));
    let (to_a, _, _) = b.connected().await;
    let (to_b, _, _) = a.connected().await;
    a.lan.recv().await;
    b.lan.recv().await;

    let frames: Vec<Vec<u8>> = (0..160u16).map(|i| vec![(i % 251) as u8; 60_000]).collect();
    let send = |lan: TcpLan, to: PeerAddr, frames: Vec<Vec<u8>>| {
        tokio::spawn(async move {
            for f in frames {
                lan.send(to.clone(), Frame::sealed(f)).await.unwrap();
            }
        })
    };
    let ab = send(a.lan.clone(), to_b.clone(), frames.clone());
    let ba = send(b.lan.clone(), to_a.clone(), frames.clone());
    tokio::time::sleep(Duration::from_millis(300)).await;
    for (i, f) in frames.iter().enumerate() {
        tokio::time::sleep(Duration::from_millis(2)).await;
        let at_b = tokio::time::timeout(WAIT, b.lan.recv()).await;
        let at_b = at_b.unwrap_or_else(|_| panic!("A → B stalled at frame {i}"));
        assert_eq!(at_b, (to_a.clone(), Frame::sealed(f.clone())));
        let at_a = tokio::time::timeout(WAIT, a.lan.recv()).await;
        let at_a = at_a.unwrap_or_else(|_| panic!("B → A stalled at frame {i}"));
        assert_eq!(at_a, (to_b.clone(), Frame::sealed(f.clone())));
    }
    ab.await.unwrap();
    ba.await.unwrap();
    a.lan.shutdown();
    b.lan.shutdown();
}

/// §4.1 rule 2: plain members are served on the same listener as TLS pairs:
/// a member built before Layer 7 and a member whose wrap holds no pin.
#[tokio::test]
async fn plain_members_are_served_on_the_same_listener() {
    let mut a = Hub::keyed(1);
    let mut b = Hub::keyed(2);
    let mut old = Hub::new(3, false);
    let mut member = Hub::new(4, true);
    let at = a.listen().await;
    a.pair(b.id, None, Some(b.spki()));
    b.pair(a.id, Some(at), Some(a.spki()));

    b.lan.maintain(at, Some(a.id));
    assert!(b.connected().await.1);
    a.connected().await;
    for m in [&mut old, &mut member] {
        m.lan.maintain(at, Some(a.id));
        let (to_a, out, _) = m.connected().await;
        assert!(out);
        let (to_m, _, _) = a.connected().await;
        assert_eq!(m.lan.recv().await, (to_a, Frame::announce(a.ann.clone())));
        loop {
            let (from, f) = a.lan.recv().await;
            if from == to_m {
                assert_eq!(f, Frame::announce(m.ann.clone()));
                break;
            }
        }
    }
    assert_eq!(a.lan.connection_count(), 3);
    for h in [&a, &b, &old, &member] {
        h.lan.shutdown();
    }
}

/// §4.1 rule 2, §14 rule 7: a client whose key is not pinned is refused in
/// the handshake: no connection on either side, and a raw client presenting
/// it reads no 111 byte.
#[tokio::test]
async fn a_wrong_client_key_is_refused_before_any_111_byte() {
    let mut a = Hub::keyed(1);
    let mut b = Hub::keyed(2);
    let other = ed25519(&[0x42; 32]);
    let at = a.listen().await;
    a.pair(b.id, None, Some(spki_of(&other))); // A pinned another key for B
    b.pair(a.id, Some(at), Some(a.spki()));
    b.lan.maintain(at, Some(a.id));
    a.quiet(1_200).await;
    b.quiet(10).await;
    b.lan.release(at);

    let as_b = ed25519(&seed_of(&b.root));
    assert_eq!(raw_attempt(at, rpk_client(as_b)).await, Vec::<u8>::new());
    // The pinned key is admitted by the same listener (the control).
    let pinned = raw_attempt(at, rpk_client(other)).await;
    assert!(
        pinned.starts_with(&HELLO),
        "the pinned key reaches the 111 hello"
    );
    a.lan.shutdown();
}

/// §4.3: a server whose key is not the pin is refused by the dialer: no
/// connection, and the server reads no 111 byte.
#[tokio::test]
async fn a_wrong_server_key_is_refused_before_any_111_byte() {
    let mut a = Hub::keyed(1);
    let mut b = Hub::keyed(2);
    let at = a.listen().await;
    a.pair(b.id, None, Some(b.spki()));
    b.pair(a.id, Some(at), Some(spki_of(&ed25519(&[0x43; 32]))));
    b.lan.maintain(at, Some(a.id));
    b.quiet(1_200).await;
    a.quiet(10).await;
    b.lan.release(at);

    // A raw server presenting an unpinned Ed25519 key.
    let (at, server) = raw_server(rpk_server(ed25519(&[0x44; 32]))).await;
    let far = ident(5).0;
    refused_by_the_dialer(&mut b, far, spki_of(&ed25519(&[0x45; 32])), at, server).await;
    a.lan.shutdown();
}

/// §4.3 "no X.509": an X.509 certificate is refused on both sides, even one
/// whose key is the pinned key, before any 111 byte. Which check refuses is
/// pinned by `each_refusal_is_the_checks_own_error` (the certificate type,
/// negotiated first); this test shows the end-to-end outcome only.
#[tokio::test]
async fn an_x509_certificate_is_refused_both_ways() {
    let a = Hub::keyed(1);
    let mut b = Hub::keyed(2);
    let pinned = spki_of(&ed25519(&unhex32(RFC8032_SEED)));
    let at = a.listen().await;
    a.pair(b.id, None, Some(pinned));
    for rpk in [false, true] {
        assert_eq!(
            raw_attempt(at, x509_client(&[&rustls::version::TLS13], rpk)).await,
            Vec::<u8>::new()
        );
    }
    let (sat, server) = raw_server(x509_server(&[&rustls::version::TLS13])).await;
    refused_by_the_dialer(&mut b, ident(5).0, pinned, sat, server).await;
    a.lan.shutdown();
}

/// §4.3 "TLS 1.3 only": a TLS 1.2 client and a TLS 1.2 server are refused,
/// before any 111 byte. TLS 1.3 is guarded twice (the version list and the
/// TLS 1.3-only provider; removing one alone changes nothing here), each
/// pinned by `relay::tls`'s unit test `tls13_only_by_both_guards`.
#[tokio::test]
async fn tls12_is_refused_both_ways() {
    let a = Hub::keyed(1);
    let mut b = Hub::keyed(2);
    let pinned = spki_of(&ed25519(&unhex32(RFC8032_SEED)));
    let at = a.listen().await;
    a.pair(b.id, None, Some(pinned));
    assert_eq!(
        raw_attempt(at, x509_client(&[&rustls::version::TLS12], false)).await,
        Vec::<u8>::new()
    );
    let (sat, server) = raw_server(x509_server(&[&rustls::version::TLS12])).await;
    refused_by_the_dialer(&mut b, ident(5).0, pinned, sat, server).await;
    a.lan.shutdown();
}

/// §4.3 "Ed25519 only": an ECDSA P-256 raw public key is refused both ways,
/// before any 111 byte. Here the pin comparison refuses it first (an Ed25519
/// pin never equals an ECDSA SPKI), so this test cannot see the scheme
/// check; that check is isolated in `relay::tls`'s unit test
/// `the_scheme_check_refuses_a_valid_non_ed25519_signature` (a valid ECDSA
/// signature over a key handed in as if pinned).
#[tokio::test]
async fn a_non_ed25519_key_is_refused_both_ways() {
    let a = Hub::keyed(1);
    let mut b = Hub::keyed(2);
    let at = a.listen().await;
    a.pair(b.id, None, Some(b.spki()));
    assert_eq!(raw_attempt(at, rpk_client(ecdsa())).await, Vec::<u8>::new());
    let (sat, server) = raw_server(rpk_server(ecdsa())).await;
    refused_by_the_dialer(&mut b, ident(5).0, a.spki(), sat, server).await;
    a.lan.shutdown();
}

/// The rustls error inside an `io::Error` from the wrap or a raw peer.
fn rustls_err(e: &std::io::Error) -> Option<rustls::Error> {
    e.get_ref()
        .and_then(|i| i.downcast_ref::<rustls::Error>())
        .cloned()
}

/// A raw client `cfg` dials `w`'s inbound side: the wrap's error.
async fn inbound_refusal(w: &dyn StreamWrap, cfg: ClientConfig) -> Option<rustls::Error> {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = tokio::spawn(raw_attempt(l.local_addr().unwrap(), cfg));
    let (s, remote) = l.accept().await.unwrap();
    let e = w.inbound(remote, s).await.err().expect("refused");
    assert_eq!(client.await.unwrap(), Vec::<u8>::new(), "no 111 byte");
    rustls_err(&e)
}

/// `w` dials a raw server `cfg` as pair `far` pinned to `pin`: the wrap's
/// error and the far server's.
async fn outbound_refusal(
    root: &Path,
    far: NodeId,
    pin: LinkSpki,
    cfg: ServerConfig,
) -> (Option<rustls::Error>, Option<rustls::Error>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = l.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = l.accept().await.unwrap();
        let r = tokio_rustls::TlsAcceptor::from(Arc::new(cfg))
            .accept(tcp)
            .await;
        r.err().and_then(|e| rustls_err(&e))
    });
    let pairs: PairTable = Arc::new(RwLock::new(vec![PairLink {
        address: Some(at),
        link_spki: Some(pin),
        ..PairLink::new(far)
    }]));
    let w = tls::wrap(&root.join("fabric"), &pairs).unwrap();
    let tcp = TcpStream::connect(at).await.unwrap();
    let e = w.outbound(at, Some(far), tcp).await.err().expect("refused");
    (rustls_err(&e), server.await.unwrap())
}

/// §4.3, row D5b: each refusal carries the error of the check that makes
/// it, at the wrap, so removing that check changes the error and fails this
/// test (checked by mutation on 2026-11-12: with the client-certificate-type
/// check off, an X.509 client certificate reaches the pin comparison and
/// fails as `InvalidCertificate(ApplicationVerificationFailure)` instead).
/// - Inbound, an X.509 client that asks for this hub's raw key: the
///   client-certificate type (`PinnedClients::requires_raw_public_keys`),
///   `PeerIncompatible(IncorrectCertificateTypeExtension)`.
/// - Inbound, an X.509 client that does not: the server-certificate type
///   (raw keys only), the same error.
/// - Inbound, TLS 1.2: `PeerIncompatible(Tls12NotOfferedOrEnabled)` (two
///   guards; see `tls12_is_refused_both_ways`).
/// - Inbound, an unpinned raw key: the pin comparison,
///   `InvalidCertificate(ApplicationVerificationFailure)`.
/// - Outbound, an X.509 server: this hub's hello offers raw keys only, and
///   the far rustls server refuses it (`IncorrectCertificateTypeExtension`
///   there, `AlertReceived(HandshakeFailure)` here). A far end that ignored
///   the offer and sent a certificate would meet the pin comparison (a 44 B
///   SPKI never equals a certificate); `requires_raw_public_keys` on both
///   verifiers is asserted in `relay::tls`'s unit tests.
/// - Outbound, a TLS 1.2 server: `AlertReceived(ProtocolVersion)` here,
///   `Tls12NotOfferedOrEnabled` there.
#[tokio::test]
async fn each_refusal_is_the_checks_own_error() {
    use rustls::{AlertDescription, CertificateError, Error, PeerIncompatible};
    let root = tempfile::tempdir().unwrap();
    LinkKey::load_or_mint(root.path()).unwrap();
    let far = ident(5).0;
    let pinned = spki_of(&ed25519(&unhex32(RFC8032_SEED)));
    let pairs: PairTable = Arc::new(RwLock::new(vec![PairLink {
        link_spki: Some(pinned),
        ..PairLink::new(far)
    }]));
    let w = tls::wrap(&root.path().join("fabric"), &pairs).unwrap();
    let cert_type = Some(Error::PeerIncompatible(
        PeerIncompatible::IncorrectCertificateTypeExtension,
    ));
    let tls13 = &[&rustls::version::TLS13][..];
    let tls12 = &[&rustls::version::TLS12][..];
    assert_eq!(
        inbound_refusal(w.as_ref(), x509_client(tls13, true)).await,
        cert_type,
        "the client-certificate type"
    );
    assert_eq!(
        inbound_refusal(w.as_ref(), x509_client(tls13, false)).await,
        cert_type,
        "the server-certificate type"
    );
    assert_eq!(
        inbound_refusal(w.as_ref(), x509_client(tls12, false)).await,
        Some(Error::PeerIncompatible(
            PeerIncompatible::Tls12NotOfferedOrEnabled
        ))
    );
    assert_eq!(
        inbound_refusal(w.as_ref(), rpk_client(ed25519(&[0x47; 32]))).await,
        Some(Error::InvalidCertificate(
            CertificateError::ApplicationVerificationFailure
        )),
        "the pin comparison"
    );
    assert_eq!(
        outbound_refusal(root.path(), far, pinned, x509_server(tls13)).await,
        (
            Some(Error::AlertReceived(AlertDescription::HandshakeFailure)),
            cert_type
        )
    );
    assert_eq!(
        outbound_refusal(root.path(), far, pinned, x509_server(tls12)).await,
        (
            Some(Error::AlertReceived(AlertDescription::ProtocolVersion)),
            Some(Error::PeerIncompatible(
                PeerIncompatible::Tls12NotOfferedOrEnabled
            ))
        )
    );
}

/// S7 (§4.4): a TLS peer whose key matched pair B and whose first announce
/// names another NodeId is refused: nothing follows the hello and the own
/// announce, and no connection is reported.
#[tokio::test]
async fn a_mismatched_first_announce_is_refused() {
    let mut a = Hub::keyed(1);
    let b = Hub::keyed(2);
    let (_, other_ann) = ident(3);
    let at = a.listen().await;
    a.pair(b.id, None, Some(b.spki()));
    let tcp = TcpStream::connect(at).await.unwrap();
    let mut tls =
        tokio_rustls::TlsConnector::from(Arc::new(rpk_client(ed25519(&seed_of(&b.root)))))
            .connect(ServerName::IpAddress(at.ip().into()), tcp)
            .await
            .unwrap();
    read_hello(&mut tls).await.unwrap();
    assert_eq!(
        read_frame(&mut tls).await.unwrap(),
        (T_ANNOUNCE, a.ann.clone())
    );
    tls.write_all(&HELLO).await.unwrap();
    tls.write_all(&encode_frame(T_ANNOUNCE, &other_ann).unwrap())
        .await
        .unwrap();
    tls.flush().await.unwrap();
    let mut rest = Vec::new();
    let _ = tokio::time::timeout(WAIT, tls.read_to_end(&mut rest))
        .await
        .expect("closed");
    assert!(rest.is_empty(), "no other 111 byte: {rest:?}");
    a.quiet(200).await;

    // The same key with B's own announce is admitted (the control).
    let tcp = TcpStream::connect(at).await.unwrap();
    let mut tls =
        tokio_rustls::TlsConnector::from(Arc::new(rpk_client(ed25519(&seed_of(&b.root)))))
            .connect(ServerName::IpAddress(at.ip().into()), tcp)
            .await
            .unwrap();
    read_hello(&mut tls).await.unwrap();
    read_frame(&mut tls).await.unwrap();
    tls.write_all(&HELLO).await.unwrap();
    tls.write_all(&encode_frame(T_ANNOUNCE, &b.ann).unwrap())
        .await
        .unwrap();
    tls.flush().await.unwrap();
    assert!(!a.connected().await.1);
    a.lan.shutdown();
}

/// §4.1 rule 4: plain to a non-private pair is refused (no byte reaches the
/// far end) until its `pair_plain` warning hash is recorded; a `lan = true`
/// pair runs plain.
#[tokio::test]
async fn plain_to_a_non_private_pair_needs_the_warning() {
    let b = Hub::keyed(2);
    let far = ident(6).0;
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = l.local_addr().unwrap();
    b.pair(far, Some(at), None);
    b.lan.maintain(at, Some(far));
    for _ in 0..2 {
        let (mut s, _) = tokio::time::timeout(WAIT, l.accept())
            .await
            .unwrap()
            .unwrap();
        let mut buf = [0u8; 8];
        let n = tokio::time::timeout(WAIT, s.read(&mut buf))
            .await
            .unwrap()
            .unwrap_or(0);
        assert_eq!(n, 0, "refused before any byte");
    }
    b.pairs.write().unwrap()[0].warning_hash = Some([0x5a; 32]);
    let first = loop {
        let (mut s, _) = tokio::time::timeout(WAIT, l.accept())
            .await
            .unwrap()
            .unwrap();
        let mut buf = [0u8; 5];
        if s.read_exact(&mut buf).await.is_ok() {
            break buf;
        }
    };
    assert_eq!(first, HELLO, "warned: plain");
    b.lan.release(at);

    let l2 = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at2 = l2.local_addr().unwrap();
    let far2 = ident(7).0;
    b.pairs.write().unwrap().push(PairLink {
        address: Some(at2),
        link_spki: Some(spki_of(&ed25519(&[0x46; 32]))),
        lan: true,
        ..PairLink::new(far2)
    });
    b.lan.maintain(at2, Some(far2));
    let (mut s, _) = tokio::time::timeout(WAIT, l2.accept())
        .await
        .unwrap()
        .unwrap();
    let mut buf = [0u8; 5];
    s.read_exact(&mut buf).await.unwrap();
    assert_eq!(buf, HELLO, "lan = true: plain even with a pin");
    b.lan.release(at2);
    b.lan.shutdown();
}

/// Live heap bytes of this test binary (the §16 measurement below): a
/// counting wrapper around the system allocator. Two relaxed atomic adds per
/// allocation; nothing else changes for the other tests.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call forwards to `System` with the caller's arguments; the
// counter only observes sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            LIVE.fetch_add(l.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(l);
        if !p.is_null() {
            LIVE.fetch_add(l.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l);
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = System.realloc(p, l, new);
        if !q.is_null() {
            LIVE.fetch_add(new, Ordering::Relaxed);
            LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        }
        q
    }
}

#[global_allocator]
static HEAP: Counting = Counting;

/// This process's resident set (`VmRSS`), in bytes; 0 when unreadable.
fn rss() -> i64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("VmRSS:"))
                .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<i64>().ok())
        })
        .map_or(0, |kb| kb * 1024)
}

/// The live heap now, signed (the deltas below may be negative noise).
fn live() -> i64 {
    i64::try_from(LIVE.load(Ordering::SeqCst)).unwrap_or(i64::MAX)
}

/// The §16 record: the memory one pinned TLS session holds, against the same
/// session plain (`lan = true`). One client hub dials one server hub through
/// `N` recording relays (each relay address is one more connection to the
/// same pair), each session carrying one 60,000 B frame each way, then idle.
/// Measured as live heap (exact; process RSS is printed beside it, blurred
/// by the allocator's reuse) after a first session built each hub's
/// one-time TLS state. A measurement, not a gate: `cargo test -p node111
/// --test relay_tls -- --ignored --nocapture memory_of_pinned_sessions`.
#[tokio::test]
#[ignore = "a measurement for the D5 record"]
async fn memory_of_pinned_sessions() {
    const N: i64 = 32;
    /// One more session B → A through a new relay, one frame each way.
    async fn session(a: &mut Hub, b: &mut Hub, at: SocketAddr, taps: &mut Vec<Tap>) {
        let t = tap(at).await;
        b.lan.maintain(t.addr, Some(a.id));
        let (to_a, _, _) = b.connected().await;
        let (to_b, _, _) = a.connected().await;
        b.lan.recv().await;
        a.lan.recv().await;
        b.lan
            .send(to_a, Frame::sealed(vec![1; 60_000]))
            .await
            .unwrap();
        a.lan.recv().await;
        a.lan
            .send(to_b, Frame::sealed(vec![2; 60_000]))
            .await
            .unwrap();
        b.lan.recv().await;
        taps.push(t);
    }
    /// (first session, per later session) as (live heap, RSS) deltas.
    async fn run(tls: bool) -> ((i64, i64), (i64, i64)) {
        let mut a = Hub::keyed(1);
        let mut b = Hub::keyed(2);
        let at = a.listen().await;
        a.pair(b.id, None, Some(b.spki()));
        b.pairs.write().unwrap().push(PairLink {
            address: Some(at),
            link_spki: Some(a.spki()),
            lan: !tls,
            ..PairLink::new(a.id)
        });
        let mut taps = Vec::new();
        let before = (live(), rss());
        session(&mut a, &mut b, at, &mut taps).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let warm = (live(), rss());
        for _ in 0..N {
            session(&mut a, &mut b, at, &mut taps).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let after = (live(), rss());
        a.lan.shutdown();
        b.lan.shutdown();
        // Let this run's connections close and its relays' pumps (which hold
        // the recordings) end, so the next run starts from a settled heap.
        drop((a, b, taps));
        tokio::time::sleep(Duration::from_millis(500)).await;
        (
            (warm.0 - before.0, warm.1 - before.1),
            ((after.0 - warm.0) / N, (after.1 - warm.1) / N),
        )
    }
    // TLS first: its RSS figures come from pages no earlier run touched.
    let (first_tls, tls) = run(true).await;
    let (first_plain, plain) = run(false).await;
    let per_end = |x: i64| x / 2;
    println!(
        "D5 memory (live heap): per session (both ends and its relay) plain {} B, TLS {} B: \
         TLS adds {} B per session, ≈ {} B per end; first session plain {} B, TLS {} B \
         (one-time TLS state of two hubs ≈ {} B)",
        plain.0,
        tls.0,
        tls.0 - plain.0,
        per_end(tls.0 - plain.0),
        first_plain.0,
        first_tls.0,
        first_tls.0 - first_plain.0 - (tls.0 - plain.0),
    );
    println!(
        "D5 memory (process RSS; the plain run reuses pages the TLS run freed): per session \
         TLS {} B, plain {} B; first session TLS {} B, plain {} B",
        tls.1, plain.1, first_tls.1, first_plain.1,
    );
}
