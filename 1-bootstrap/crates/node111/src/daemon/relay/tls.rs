//! Layer 7 §4.1, §4.3–§4.5, §4.7 (row D5): TLS 1.3 with RFC 7250 raw public
//! keys for non-private paired-hub links, through `tokio-rustls` on rustls's
//! `ring` provider: M6's `StreamWrap` (§4.4) implemented here.
//!
//! **Which links (§4.1).**
//! - **Outbound** ([`outbound_rule`]): a dial that names no pair (a member's
//!   link to its hub, the home hub, a hand-configured peer: rule 3) is plain,
//!   byte-identical with `tcp-lan` before Layer 7. A dial to a paired hub is
//!   plain when the dialled address is private ([`is_private`]: RFC 1918,
//!   RFC 6598, ULA `fc00::/7`, link-local) or the owner marked the pair
//!   `lan = true`; otherwise it runs inside TLS with the pinned link key, and
//!   a non-private pair with no pinned key runs plain only when its
//!   `pair_plain` warning hash is recorded, else the dial is refused before
//!   any byte (rule 4). Loopback is never a peer (`tcp-lan` refuses it by
//!   default), so it is not private here: a test that turns the refusal off
//!   gets TLS over loopback.
//! - **Inbound** on `:11183` (rule 2): with no pinned pair at all the stream
//!   passes through untouched (nothing is read first, so the wire and its
//!   order are what they were: the listener's hello and announce go first,
//!   and a TLS client then meets exactly what every plain peer is shown, its
//!   record fails the hello check and the connection ends; with no pin there
//!   is no TLS peer to admit). Otherwise the first byte is peeked without
//!   consuming it: `0x16` (a TLS handshake record) goes to the TLS path,
//!   `0x31` (the `111F` hello) is served plain as today, anything else closes
//!   the connection. A TLS client must present a pinned link key; an
//!   unpinned one ends the handshake, before any 111 byte. The wrap names the
//!   pair whose pinned SPKI matched (S7), and `tcp-lan` refuses a first
//!   announce naming another NodeId.
//!
//! **The configuration (§4.3, normative).** TLS 1.3 only (the provider keeps
//! only the TLS 1.3 suites and both builders take `TLS13` alone); Ed25519
//! only (each verifier advertises `ED25519` and refuses any other scheme);
//! raw public keys on both sides (`only_raw_public_keys` on both resolvers,
//! `requires_raw_public_keys` on both verifiers: a peer that offers or
//! presents an X.509 certificate fails the certificate-type negotiation);
//! client authentication mandatory; each verifier compares the presented
//! SPKI byte-exact with the pin (the client: the one pin of the pair it
//! dialled; the server: any pinned pair in the live table, exactly one) and
//! then checks the signature with `verify_tls13_signature_with_raw_key`. No
//! X.509, no CA, no SNI (`enable_sni = false`, and the name is the dialled IP
//! literal), no name check, no clock (the `now` rustls passes is ignored).
//! Session resumption is off on both sides (no tickets, no session cache), so
//! every connection runs the full handshake and re-checks the pin; 0-RTT is
//! never offered or accepted.
//!
//! **Weight (§16; the "never adds" clause).** [`wrap`] hands `tcp-lan` one
//! small object on every node that runs `tcp-lan` (it holds the path of the
//! node root and a clone of the live pair table): the fabric reads it once
//! when it builds the adapter, and a hub that pairs later must not need a
//! restart. While no pair holds a pinned key, its calls box the `TcpStream`
//! and return it: no peek, no file read, no TLS state. The link key and the
//! rustls configurations are read and built at the first TLS handshake and
//! kept (rebuilt only when the key file changed). No process, thread, timer
//! or listener.
//!
//! **Buffered writes.** `tcp-lan`'s writer never flushes (a `TcpStream` has
//! nothing to flush), while rustls may keep the tail of a write when the
//! socket is full. [`RpkStream`] has the connection's reader task, always
//! parked on the stream, push such a tail out as soon as the socket takes
//! it, so no frame waits for the next write or keepalive. A socket holds one
//! write-readiness waker, so the reader's flush may take the wakeup of a
//! writer parked on a full TLS buffer; the reader hands it on once its flush
//! completes.

use super::linkkey::LinkKey;
use crate::daemon::fabric::settings::{LinkSpki, PairLink, PairTable};
use futures_util::future::BoxFuture;
use id111::keyset::NodeId;
use mesh111::link::tcp_lan::{BoxedStream, StreamWrap};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{AlwaysResolvesClientRawPublicKeys, Resumption};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, SubjectPublicKeyInfoDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::{AlwaysResolvesServerRawPublicKeys, NoServerSessionStorage};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig,
    SignatureScheme, SupportedCipherSuite,
};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{ready, Context, Poll, Waker};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

/// The first byte of a TLS handshake record (§4.1 rule 2).
pub const TLS_HANDSHAKE_RECORD: u8 = 0x16;
/// The first byte of the `111F` hello (§4.1 rule 2; L5 §8.1).
pub const PLAIN_HELLO_FIRST: u8 = mesh111::link::tcp_lan::HELLO[0];

/// The one signature scheme either side accepts (§4.3: Ed25519 only).
const SCHEMES: [SignatureScheme; 1] = [SignatureScheme::ED25519];

/// §4.1 rule 1: private = RFC 1918, RFC 6598 (`100.64.0.0/10`), ULA
/// `fc00::/7`, link-local (`169.254.0.0/16`, `fe80::/10`); an IPv4-mapped
/// IPv6 address is judged by its IPv4 address. Loopback is never a peer and
/// is not private.
pub fn is_private(ip: IpAddr) -> bool {
    fn v4(a: Ipv4Addr) -> bool {
        let o = a.octets();
        a.is_private() || a.is_link_local() || (o[0] == 100 && o[1] & 0xc0 == 64)
    }
    match ip {
        IpAddr::V4(a) => v4(a),
        IpAddr::V6(a) => match a.to_ipv4_mapped() {
            Some(m) => v4(m),
            None => {
                let s = a.segments()[0];
                s & 0xfe00 == 0xfc00 || s & 0xffc0 == 0xfe80
            }
        },
    }
}

/// How a dial runs (§4.1 rules 1, 3 and 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outbound {
    /// Plain `tcp-lan`, as before Layer 7.
    Plain,
    /// TLS 1.3 with raw public keys, the far hub pinned to this SPKI.
    Tls(LinkSpki),
    /// Refused before any byte: a non-private pair with neither a pinned key
    /// nor an accepted `pair_plain` warning.
    Refused,
}

/// The rule for a dial to `addr` expecting `expected` (§4.1). The pair is
/// the one `expected` names, else the one recorded at `addr`; none ⇒ plain
/// (rule 3). A pair: plain at a private address or when `lan` (rule 1), TLS
/// when its link key is pinned, else plain only with a recorded warning hash
/// (rule 4), else refused. A pinned key wins over a recorded warning: the
/// warning admits plain only where TLS cannot run.
pub fn outbound_rule(pairs: &[PairLink], addr: SocketAddr, expected: Option<NodeId>) -> Outbound {
    let canon = |a: SocketAddr| SocketAddr::new(a.ip().to_canonical(), a.port());
    let pair = expected
        .and_then(|id| pairs.iter().find(|p| p.node_id == id))
        .or_else(|| {
            pairs
                .iter()
                .find(|p| p.address.is_some_and(|a| canon(a) == canon(addr)))
        });
    let Some(p) = pair else {
        return Outbound::Plain;
    };
    if p.lan || is_private(addr.ip()) {
        Outbound::Plain
    } else if let Some(pin) = p.link_spki {
        Outbound::Tls(pin)
    } else if p.warning_hash.is_some() {
        Outbound::Plain
    } else {
        Outbound::Refused
    }
}

/// The pair whose pinned SPKI is `spki`, byte-exact; `None` when no pair or
/// more than one pins it (§4.1 rule 2, S7: the match must name one pair).
pub fn pinned_pair(pairs: &[PairLink], spki: &[u8]) -> Option<NodeId> {
    let mut hits = pairs
        .iter()
        .filter(|p| p.link_spki.is_some_and(|k| k.as_der().as_slice() == spki));
    let first = hits.next()?;
    hits.next().is_none().then_some(first.node_id)
}

/// The signature over the handshake, by the key already compared with the
/// pin (§4.3): Ed25519 only, then `verify_tls13_signature_with_raw_key`.
fn check_signature(
    message: &[u8],
    cert: &CertificateDer<'_>,
    dss: &DigitallySignedStruct,
    algs: &WebPkiSupportedAlgorithms,
) -> Result<HandshakeSignatureValid, rustls::Error> {
    if dss.scheme != SignatureScheme::ED25519 {
        return Err(rustls::PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme.into());
    }
    rustls::crypto::verify_tls13_signature_with_raw_key(
        message,
        &SubjectPublicKeyInfoDer::from(cert.as_ref()),
        dss,
        algs,
    )
}

/// A presented key that is not the pin: refused (§4.3).
fn not_pinned() -> rustls::Error {
    rustls::Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure)
}

/// The client's check of the server (§4.3): the one pin of the dialled pair.
#[derive(Debug)]
struct PinnedServer {
    pin: LinkSpki,
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedServer {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if intermediates.is_empty() && end_entity.as_ref() == self.pin.as_der().as_slice() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(not_pinned())
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.3 only".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        check_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        SCHEMES.to_vec()
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

/// The server's check of the client (§4.3): mandatory, against every pinned
/// pair in the live table (a pin recorded or removed while the node runs
/// applies to the next handshake).
#[derive(Debug)]
struct PinnedClients {
    pairs: PairTable,
    algs: WebPkiSupportedAlgorithms,
}

impl ClientCertVerifier for PinnedClients {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        let pinned = intermediates.is_empty()
            && self
                .pairs
                .read()
                .is_ok_and(|p| pinned_pair(&p, end_entity.as_ref()).is_some());
        if pinned {
            Ok(ClientCertVerified::assertion())
        } else {
            Err(not_pinned())
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.3 only".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        check_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        SCHEMES.to_vec()
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

/// rustls's `ring` provider with the TLS 1.3 suites only (§4.3, §4.5).
fn provider() -> Arc<CryptoProvider> {
    let mut p = rustls::crypto::ring::default_provider();
    p.cipher_suites
        .retain(|s| matches!(s, SupportedCipherSuite::Tls13(_)));
    Arc::new(p)
}

/// The TLS state of one link key, built at the first TLS handshake.
struct TlsState {
    /// The own key's SPKI (the cache key: a re-minted key rebuilds).
    spki: LinkSpki,
    provider: Arc<CryptoProvider>,
    resolver_client: Arc<AlwaysResolvesClientRawPublicKeys>,
    server: Arc<ServerConfig>,
}

impl TlsState {
    fn new(key: &LinkKey, pairs: &PairTable) -> Result<TlsState, rustls::Error> {
        let provider = provider();
        let algs = provider.signature_verification_algorithms;
        let certified = key.certified();
        let mut server = ServerConfig::builder_with_provider(Arc::clone(&provider))
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_client_cert_verifier(Arc::new(PinnedClients {
                pairs: Arc::clone(pairs),
                algs,
            }))
            .with_cert_resolver(Arc::new(AlwaysResolvesServerRawPublicKeys::new(
                Arc::clone(&certified),
            )));
        server.session_storage = Arc::new(NoServerSessionStorage {});
        server.send_tls13_tickets = 0;
        server.max_early_data_size = 0;
        Ok(TlsState {
            spki: *key.spki(),
            provider,
            resolver_client: Arc::new(AlwaysResolvesClientRawPublicKeys::new(certified)),
            server: Arc::new(server),
        })
    }

    /// A client configuration for one dial, the far hub pinned to `pin`.
    fn client(&self, pin: LinkSpki) -> Result<Arc<ClientConfig>, rustls::Error> {
        let mut c = ClientConfig::builder_with_provider(Arc::clone(&self.provider))
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedServer {
                pin,
                algs: self.provider.signature_verification_algorithms,
            }))
            .with_client_cert_resolver(Arc::clone(&self.resolver_client) as _);
        c.resumption = Resumption::disabled();
        c.enable_sni = false;
        c.enable_early_data = false;
        Ok(Arc::new(c))
    }
}

fn io_err(kind: io::ErrorKind, what: impl std::fmt::Display) -> io::Error {
    io::Error::new(kind, what.to_string())
}

/// The `tcp-lan` stream wrap (§4.4): the rules of §4.1 over the live pair
/// table, TLS through `tokio-rustls` where they ask for it.
pub struct RpkWrap {
    /// The node root (`<root>/secrets/link-key`, §4.2).
    root: PathBuf,
    /// The daemon's live pairing table (M5).
    pairs: PairTable,
    /// Built at the first TLS handshake (module doc, "Weight").
    tls: Mutex<Option<Arc<TlsState>>>,
}

impl std::fmt::Debug for RpkWrap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpkWrap")
            .field("root", &self.root)
            .field("tls_built", &self.tls_built())
            .finish_non_exhaustive()
    }
}

impl RpkWrap {
    /// A wrap over `pairs` for the node rooted at `root`. Builds nothing.
    pub fn new(root: PathBuf, pairs: PairTable) -> RpkWrap {
        RpkWrap {
            root,
            pairs,
            tls: Mutex::new(None),
        }
    }

    /// Whether the TLS state exists (it does only after a TLS handshake).
    pub fn tls_built(&self) -> bool {
        self.tls.lock().map(|t| t.is_some()).unwrap_or(true)
    }

    /// Whether any pair holds a pinned key (a poisoned table reads as yes:
    /// the peek path then refuses what it cannot check).
    fn any_pin(&self) -> bool {
        self.pairs
            .read()
            .map(|p| p.iter().any(|l| l.link_spki.is_some()))
            .unwrap_or(true)
    }

    /// The TLS state for the stored link key; rebuilt when the key changed.
    /// No key ⇒ an error: TLS cannot run before this hub's first pairing.
    fn tls(&self) -> io::Result<Arc<TlsState>> {
        let key = LinkKey::load(&self.root)
            .map_err(|e| io_err(io::ErrorKind::PermissionDenied, e))?
            .ok_or_else(|| io_err(io::ErrorKind::NotFound, "no link key"))?;
        let mut held = self.tls.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = held.as_ref().filter(|s| s.spki == *key.spki()) {
            return Ok(Arc::clone(s));
        }
        let s = Arc::new(
            TlsState::new(&key, &self.pairs).map_err(|e| io_err(io::ErrorKind::Other, e))?,
        );
        *held = Some(Arc::clone(&s));
        Ok(s)
    }

    async fn accept(&self, stream: TcpStream) -> io::Result<(BoxedStream, Option<NodeId>)> {
        if !self.any_pin() {
            return Ok((Box::new(stream), None));
        }
        let mut first = [0u8; 1];
        if stream.peek(&mut first).await? == 0 {
            return Err(io_err(io::ErrorKind::UnexpectedEof, "closed before a byte"));
        }
        match first[0] {
            PLAIN_HELLO_FIRST => Ok((Box::new(stream), None)),
            TLS_HANDSHAKE_RECORD => {
                let state = self.tls()?;
                let tls = tokio_rustls::TlsAcceptor::from(Arc::clone(&state.server))
                    .accept(stream)
                    .await?;
                // The verifier matched exactly one pair; read it back from the
                // table as it is now (a pair removed meanwhile is refused).
                let matched = tls
                    .get_ref()
                    .1
                    .peer_certificates()
                    .and_then(|c| c.first())
                    .and_then(|spki| {
                        self.pairs
                            .read()
                            .ok()
                            .and_then(|p| pinned_pair(&p, spki.as_ref()))
                    })
                    .ok_or_else(|| io_err(io::ErrorKind::PermissionDenied, "no pinned pair"))?;
                Ok((Box::new(RpkStream::new(tls)), Some(matched)))
            }
            other => Err(io_err(
                io::ErrorKind::InvalidData,
                format_args!("first byte 0x{other:02x} is neither TLS nor 111"),
            )),
        }
    }

    async fn dial(
        &self,
        addr: SocketAddr,
        expected: Option<NodeId>,
        stream: TcpStream,
    ) -> io::Result<BoxedStream> {
        let rule = match self.pairs.read() {
            Ok(p) => outbound_rule(&p, addr, expected),
            Err(_) => Outbound::Refused,
        };
        match rule {
            Outbound::Plain => Ok(Box::new(stream)),
            Outbound::Refused => Err(io_err(
                io::ErrorKind::PermissionDenied,
                "plain to a non-private paired hub needs its pair_plain warning (§4.1 rule 4)",
            )),
            Outbound::Tls(pin) => {
                let state = self.tls()?;
                let config = state
                    .client(pin)
                    .map_err(|e| io_err(io::ErrorKind::Other, e))?;
                let tls = tokio_rustls::TlsConnector::from(config)
                    .connect(ServerName::IpAddress(addr.ip().into()), stream)
                    .await?;
                Ok(Box::new(RpkStream::new(tls)))
            }
        }
    }
}

impl StreamWrap for RpkWrap {
    fn outbound(
        &self,
        addr: SocketAddr,
        expected: Option<NodeId>,
        stream: TcpStream,
    ) -> BoxFuture<'_, io::Result<BoxedStream>> {
        Box::pin(self.dial(addr, expected, stream))
    }

    fn inbound(
        &self,
        _remote: SocketAddr,
        stream: TcpStream,
    ) -> BoxFuture<'_, io::Result<(BoxedStream, Option<NodeId>)>> {
        Box::pin(self.accept(stream))
    }
}

/// A TLS stream whose buffered tail is pushed out by the reader (module doc,
/// "Buffered writes"): every read first flushes, and a write that left bytes
/// in the TLS buffer asks the socket to wake the reader once it can take
/// them.
///
/// A socket keeps one write-readiness waker (tokio's `ScheduledIo`: the last
/// task to meet a full socket owns it). A writer parked on a full TLS buffer
/// can therefore lose its wakeup to the reader's flush; the reader wakes it
/// once a flush completes, so a parked write never waits for a wakeup that
/// went to the other task.
pub struct RpkStream<S> {
    inner: S,
    /// The task that reads (`tcp-lan`'s connection task).
    reader: Option<Waker>,
    /// The task whose write or flush is parked on the socket, if any.
    writer: Option<Waker>,
}

/// Keep `slot` waking `cx`'s task (cloned only when it changed).
fn remember(slot: &mut Option<Waker>, cx: &Context<'_>) {
    if !slot.as_ref().is_some_and(|w| w.will_wake(cx.waker())) {
        *slot = Some(cx.waker().clone());
    }
}

impl<S> RpkStream<S> {
    fn new(inner: S) -> RpkStream<S> {
        RpkStream {
            inner,
            reader: None,
            writer: None,
        }
    }

    /// A write-side poll's outcome: a parked writer is remembered (the
    /// reader's flush may take its socket wakeup), a finished one forgotten.
    fn writer_polled<T>(&mut self, cx: &Context<'_>, p: Poll<T>) -> Poll<T> {
        if p.is_pending() {
            remember(&mut self.writer, cx);
        } else {
            self.writer = None;
        }
        p
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for RpkStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        remember(&mut this.reader, cx);
        // Errors surface on the read or the next write. A finished flush
        // (the socket took the tail, or failed) releases a parked writer.
        if Pin::new(&mut this.inner).poll_flush(cx).is_ready() {
            if let Some(w) = this.writer.take() {
                w.wake();
            }
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for RpkStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let p = Pin::new(&mut this.inner).poll_write(cx, buf);
        let n = ready!(this.writer_polled(cx, p))?;
        if let Some(w) = this.reader.clone() {
            let _ = Pin::new(&mut this.inner).poll_flush(&mut Context::from_waker(&w));
        }
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let p = Pin::new(&mut this.inner).poll_flush(cx);
        this.writer_polled(cx, p)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let p = Pin::new(&mut this.inner).poll_shutdown(cx);
        this.writer_polled(cx, p)
    }
}

/// The `tcp-lan` stream wrap (§4.1 rule 1, §4.4), read once by the fabric
/// when it builds `tcp-lan` (`daemon/fabric/mod.rs`, row M5). `fabric_dir` is
/// `<root>/fabric` (the link key lives in `<root>/secrets/`, §4.2); `pairs`
/// is the daemon's live pairing table (§4.6: the pins, `lan`, the warning
/// hash). Always a wrap (a hub that pairs while running needs no restart);
/// it passes every stream through untouched while no pair holds a pinned
/// key, and builds its TLS state only at the first TLS handshake (module
/// doc, "Weight").
pub fn wrap(fabric_dir: &Path, pairs: &PairTable) -> Option<Arc<dyn StreamWrap>> {
    let root = fabric_dir.parent().unwrap_or(fabric_dir).to_path_buf();
    Some(Arc::new(RpkWrap::new(root, Arc::clone(pairs))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::RwLock;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn id(b: u8) -> NodeId {
        NodeId::from([b; 32])
    }

    fn spki(fill: u8) -> LinkSpki {
        let mut der = crate::daemon::fabric::settings::ED25519_SPKI_PREFIX.to_vec();
        der.extend_from_slice(&[fill; 32]);
        LinkSpki::from_der(&der).unwrap()
    }

    fn pair(b: u8, addr: &str) -> PairLink {
        PairLink {
            address: Some(addr.parse().unwrap()),
            link_spki: Some(spki(b)),
            ..PairLink::new(id(b))
        }
    }

    #[test]
    fn private_is_rfc1918_rfc6598_ula_and_link_local() {
        for p in [
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.2",
            "100.64.0.1",
            "100.127.255.255",
            "169.254.3.4",
            "fc00::1",
            "fdff::1",
            "fe80::1",
            "febf::1",
            "::ffff:192.168.1.2",
            "::ffff:100.100.1.1",
        ] {
            assert!(is_private(p.parse().unwrap()), "{p}");
        }
        for p in [
            "203.0.113.7",
            "172.32.0.1",
            "100.128.0.1",
            "100.63.255.255",
            "8.8.8.8",
            "127.0.0.1",
            "::1",
            "2001:db8::1",
            "fec0::1",
            "::ffff:203.0.113.7",
            "0.0.0.0",
        ] {
            assert!(!is_private(p.parse().unwrap()), "{p}");
        }
    }

    /// §4.1 rules 1, 3 and 4 for every kind of dial.
    #[test]
    fn the_outbound_rule() {
        let public: SocketAddr = "203.0.113.7:11183".parse().unwrap();
        let lan: SocketAddr = "192.168.1.9:11183".parse().unwrap();
        let p = pair(2, "203.0.113.7:11183");
        // Rule 3: not a pair (a member's hub, a peer): plain.
        assert_eq!(outbound_rule(&[], public, Some(id(9))), Outbound::Plain);
        assert_eq!(
            outbound_rule(std::slice::from_ref(&p), public, Some(id(2))),
            Outbound::Tls(spki(2))
        );
        // Found by address when the dial names no NodeId (the pairing dial).
        assert_eq!(
            outbound_rule(std::slice::from_ref(&p), public, None),
            Outbound::Tls(spki(2))
        );
        // An address another node's dial names still follows the pair's rule.
        assert_eq!(
            outbound_rule(std::slice::from_ref(&p), public, Some(id(9))),
            Outbound::Tls(spki(2))
        );
        // The mapped form of the same address is the same address.
        let mapped: SocketAddr = "[::ffff:203.0.113.7]:11183".parse().unwrap();
        assert_eq!(
            outbound_rule(std::slice::from_ref(&p), mapped, None),
            Outbound::Tls(spki(2))
        );
        // Rule 1: a private address or `lan` is plain, pinned or not.
        assert_eq!(
            outbound_rule(std::slice::from_ref(&p), lan, Some(id(2))),
            Outbound::Plain
        );
        let marked = PairLink {
            lan: true,
            ..p.clone()
        };
        assert_eq!(
            outbound_rule(&[marked], public, Some(id(2))),
            Outbound::Plain
        );
        // Rule 4: non-private without a pin: plain only when warned.
        let bare = PairLink {
            link_spki: None,
            ..p.clone()
        };
        assert_eq!(
            outbound_rule(std::slice::from_ref(&bare), public, Some(id(2))),
            Outbound::Refused
        );
        let warned = PairLink {
            warning_hash: Some([7; 32]),
            ..bare
        };
        assert_eq!(
            outbound_rule(std::slice::from_ref(&warned), public, Some(id(2))),
            Outbound::Plain
        );
        // A pin wins over a recorded warning.
        let both = PairLink {
            link_spki: Some(spki(2)),
            ..warned
        };
        assert_eq!(
            outbound_rule(&[both], public, Some(id(2))),
            Outbound::Tls(spki(2))
        );
    }

    #[test]
    fn a_pin_names_exactly_one_pair() {
        let a = pair(2, "203.0.113.7:1");
        let b = pair(3, "203.0.113.8:1");
        assert_eq!(
            pinned_pair(&[a.clone(), b.clone()], spki(3).as_der()),
            Some(id(3))
        );
        assert_eq!(
            pinned_pair(std::slice::from_ref(&a), spki(3).as_der()),
            None
        );
        assert_eq!(
            pinned_pair(std::slice::from_ref(&a), &spki(2).as_der()[..43]),
            None
        );
        let twin = PairLink {
            link_spki: Some(spki(2)),
            ..PairLink::new(id(4))
        };
        assert_eq!(pinned_pair(&[a, twin], spki(2).as_der()), None, "ambiguous");
    }

    #[test]
    fn the_server_pin_is_byte_exact_and_chainless() {
        let v = PinnedServer {
            pin: spki(2),
            algs: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        };
        let name = ServerName::IpAddress(IpAddr::from([203, 0, 113, 7]).into());
        let now = UnixTime::since_unix_epoch(std::time::Duration::ZERO);
        let ok = CertificateDer::from(spki(2).as_der().to_vec());
        let wrong = CertificateDer::from(spki(3).as_der().to_vec());
        assert!(v.verify_server_cert(&ok, &[], &name, &[], now).is_ok());
        assert!(v.verify_server_cert(&wrong, &[], &name, &[], now).is_err());
        assert!(v
            .verify_server_cert(&ok, std::slice::from_ref(&ok), &name, &[], now)
            .is_err());
        // The scheme check (Ed25519 only) runs in the handshake tests
        // (`tests/relay_tls.rs`: an ECDSA peer); a `DigitallySignedStruct`
        // cannot be built outside rustls.
        assert_eq!(v.supported_verify_schemes(), vec![SignatureScheme::ED25519]);
        assert!(v.requires_raw_public_keys());
    }

    #[test]
    fn the_client_pin_reads_the_live_table() {
        let pairs: PairTable = Arc::new(RwLock::new(vec![]));
        let v = PinnedClients {
            pairs: Arc::clone(&pairs),
            algs: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        };
        let now = UnixTime::since_unix_epoch(std::time::Duration::ZERO);
        let c = CertificateDer::from(spki(2).as_der().to_vec());
        assert!(v.verify_client_cert(&c, &[], now).is_err());
        pairs.write().unwrap().push(pair(2, "203.0.113.7:1"));
        assert!(v.verify_client_cert(&c, &[], now).is_ok());
        assert!(v
            .verify_client_cert(&c, std::slice::from_ref(&c), now)
            .is_err());
        assert!(v.client_auth_mandatory() && v.offer_client_auth());
        assert!(v.requires_raw_public_keys());
        pairs.write().unwrap().clear();
        assert!(
            v.verify_client_cert(&c, &[], now).is_err(),
            "unpinned at once"
        );
    }

    async fn loopback() -> (TcpStream, TcpStream) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let c = TcpStream::connect(l.local_addr().unwrap()).await.unwrap();
        let (s, _) = l.accept().await.unwrap();
        (c, s)
    }

    /// The "never adds" clause: with no pinned pair a stream passes through
    /// unread (the server's hello may go first, as before Layer 7), and no
    /// key is read, no `secrets/` made, no TLS state built.
    #[tokio::test]
    async fn without_a_pin_nothing_is_read_or_built() {
        let d = tempfile::tempdir().unwrap();
        let pairs: PairTable = Arc::new(RwLock::new(vec![PairLink::new(id(2))]));
        let w = RpkWrap::new(d.path().to_path_buf(), pairs);
        let (mut c, s) = loopback().await;
        let remote = c.local_addr().unwrap();
        let (mut s, matched) =
            tokio::time::timeout(std::time::Duration::from_secs(2), w.inbound(remote, s))
                .await
                .expect("no byte awaited")
                .unwrap();
        assert_eq!(matched, None);
        s.write_all(b"111F\x01").await.unwrap();
        let mut b = [0u8; 5];
        c.read_exact(&mut b).await.unwrap();
        assert_eq!(&b, b"111F\x01");
        let (_c2, s2) = loopback().await;
        let at = s2.peer_addr().unwrap();
        w.outbound(at, Some(id(9)), s2).await.unwrap();
        assert!(!w.tls_built());
        assert!(
            std::fs::read_dir(d.path()).unwrap().next().is_none(),
            "nothing on disk"
        );
        assert!(wrap(&d.path().join("fabric"), &Arc::new(RwLock::new(vec![]))).is_some());
    }

    /// §4.1 rule 2: with a pin, `0x31` is served plain (the byte stays in
    /// the stream), anything but `0x16` or `0x31` closes, and an empty
    /// connection closes.
    #[tokio::test]
    async fn the_peek_dispatches_on_the_first_byte() {
        let d = tempfile::tempdir().unwrap();
        let pairs: PairTable = Arc::new(RwLock::new(vec![pair(2, "203.0.113.7:1")]));
        let w = RpkWrap::new(d.path().to_path_buf(), pairs);

        let (mut c, s) = loopback().await;
        c.write_all(b"111F\x01").await.unwrap();
        let (mut s, matched) = w.inbound(c.local_addr().unwrap(), s).await.unwrap();
        assert_eq!(matched, None);
        let mut b = [0u8; 5];
        s.read_exact(&mut b).await.unwrap();
        assert_eq!(&b, b"111F\x01", "nothing consumed");

        let (mut c, s) = loopback().await;
        c.write_all(b"GET / HTTP/1.1\r\n").await.unwrap();
        assert_eq!(
            w.inbound(c.local_addr().unwrap(), s)
                .await
                .err()
                .map(|e| e.kind()),
            Some(io::ErrorKind::InvalidData)
        );

        let (c, s) = loopback().await;
        let at = c.local_addr().unwrap();
        drop(c);
        assert!(w.inbound(at, s).await.is_err());

        // A TLS hello with no link key on disk: refused, nothing built.
        let (mut c, s) = loopback().await;
        c.write_all(&[0x16, 0x03, 0x01, 0x00, 0x05]).await.unwrap();
        assert!(w.inbound(c.local_addr().unwrap(), s).await.is_err());
        assert!(!w.tls_built());
    }

    /// §4.1 rule 4 at the wrap: a refused dial returns before any byte.
    #[tokio::test]
    async fn a_refused_dial_writes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let (mut far, near) = loopback().await;
        let at = near.peer_addr().unwrap();
        let bare = PairLink {
            address: Some(at),
            ..PairLink::new(id(2))
        };
        let w = RpkWrap::new(d.path().to_path_buf(), Arc::new(RwLock::new(vec![bare])));
        let e = w.outbound(at, Some(id(2)), near).await.err().unwrap();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
        let mut b = [0u8; 1];
        assert_eq!(far.read(&mut b).await.unwrap(), 0, "closed, no byte");
    }

    /// A stand-in for a TLS stream over a socket: a write is taken whole but,
    /// while the socket is not `writable`, leaves a tail (`pending`) that
    /// only a flush pushes out; a write that meets a tail is refused
    /// (`Pending`, the TLS buffer is full). Like tokio's `ScheduledIo`, the
    /// socket keeps ONE write-readiness waker (`slot`): the last task that
    /// met a full socket owns it.
    #[derive(Default)]
    struct Tail {
        pending: bool,
        writable: bool,
        slot: Option<Waker>,
    }

    impl Tail {
        fn full(&mut self, cx: &Context<'_>) -> bool {
            let full = self.pending && !self.writable;
            if full {
                self.slot = Some(cx.waker().clone());
            }
            full
        }
    }

    impl AsyncRead for Tail {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }

    impl AsyncWrite for Tail {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            let this = self.get_mut();
            if this.full(cx) {
                return Poll::Pending;
            }
            this.pending = !this.writable;
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            let this = self.get_mut();
            if this.full(cx) {
                return Poll::Pending;
            }
            this.pending = false;
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[derive(Default)]
    struct Count(std::sync::atomic::AtomicUsize);

    impl std::task::Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Module doc, "Buffered writes": a write that leaves a tail in the TLS
    /// buffer arms the socket to wake the reader task (parked on the stream),
    /// and the reader's next poll pushes the tail out; nobody waits for the
    /// writer's next frame or keepalive.
    #[test]
    fn the_reader_pushes_out_a_written_tail() {
        let reader = Arc::new(Count::default());
        let writer = Arc::new(Count::default());
        let rw = Waker::from(Arc::clone(&reader));
        let ww = Waker::from(Arc::clone(&writer));
        let mut s = RpkStream::new(Tail::default());
        let mut buf = [0u8; 8];
        let mut rb = ReadBuf::new(&mut buf);
        assert!(Pin::new(&mut s)
            .poll_read(&mut Context::from_waker(&rw), &mut rb)
            .is_pending());
        let wrote = Pin::new(&mut s).poll_write(&mut Context::from_waker(&ww), b"frame");
        assert!(matches!(wrote, Poll::Ready(Ok(5))));
        assert!(s.inner.pending, "a tail is left");
        let armed = s.inner.slot.take().expect("the socket is armed");
        assert!(armed.will_wake(&rw), "for the reader, not the idle writer");
        s.inner.writable = true;
        armed.wake();
        assert_eq!(reader.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        let _ = Pin::new(&mut s).poll_read(&mut Context::from_waker(&rw), &mut rb);
        assert!(!s.inner.pending, "the reader pushed the tail out");
        assert_eq!(writer.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    /// The one write-readiness slot: a writer parked on a full TLS buffer
    /// loses its socket wakeup to the reader's flush; the reader, once its
    /// flush completes, wakes the writer, which then writes. Without the
    /// hand-over the writer task stays parked for good (no keepalive, no
    /// frame) while the reader alone is woken.
    #[test]
    fn a_parked_writer_is_woken_after_the_reader_takes_its_wakeup() {
        use std::sync::atomic::Ordering::SeqCst;
        let reader = Arc::new(Count::default());
        let writer = Arc::new(Count::default());
        let rw = Waker::from(Arc::clone(&reader));
        let ww = Waker::from(Arc::clone(&writer));
        let mut s = RpkStream::new(Tail::default());
        let mut buf = [0u8; 8];
        let mut rb = ReadBuf::new(&mut buf);
        let mut read = |s: &mut RpkStream<Tail>| {
            let _ = Pin::new(s).poll_read(&mut Context::from_waker(&rw), &mut rb);
        };
        read(&mut s);
        // A first write leaves a tail; a second meets the full buffer and
        // parks, its waker in the socket's one slot.
        let write = |s: &mut RpkStream<Tail>| {
            Pin::new(s).poll_write(&mut Context::from_waker(&ww), b"frame")
        };
        assert!(matches!(write(&mut s), Poll::Ready(Ok(5))));
        assert!(write(&mut s).is_pending());
        assert!(s.inner.slot.as_ref().is_some_and(|w| w.will_wake(&ww)));
        // A frame arrives: the reader's flush meets the full socket and
        // takes the slot.
        read(&mut s);
        assert!(s.inner.slot.as_ref().is_some_and(|w| w.will_wake(&rw)));
        // The socket drains: only the slot's owner, the reader, is woken.
        s.inner.writable = true;
        s.inner.slot.take().unwrap().wake();
        assert_eq!((reader.0.load(SeqCst), writer.0.load(SeqCst)), (1, 0));
        // Its flush completes and hands the wakeup on to the writer.
        read(&mut s);
        assert!(!s.inner.pending);
        assert_eq!(writer.0.load(SeqCst), 1, "the parked writer is woken");
        assert!(matches!(write(&mut s), Poll::Ready(Ok(5))));
        // A finished write is forgotten: the next flush wakes nobody.
        read(&mut s);
        assert_eq!(writer.0.load(SeqCst), 1);
    }
}
