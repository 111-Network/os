//! Layer 7 §4.6 (row D6): pairing two hubs, the daemon's side.
//!
//! - **The bundle** ([`PairBundle`], §4.6 step 1):
//!   `111PAIR:<card b32>:<code>:<link SPKI b32>:<address or empty>:<role>:<via>`, printed by the
//!   offer and parsed by the scan, here in node111 (the `111HUB:` parser in mesh111 is untouched).
//!   Five `:` separators follow the prefix; the parser takes three fields from the left and two
//!   from the right, and the address is what remains, so a bracketed IPv6 literal with its port
//!   works. Sizes are checked before anything is parsed ([`BUNDLE_MAX`], [`ADDRESS_MAX`]). The
//!   address is an IP literal (`ip:port`, `[v6]:port`): the offer prints one, and a name in a
//!   bundle is refused rather than resolved (nothing of the bundle goes to a resolver). An empty
//!   link-SPKI field is read as "the showing hub's build offers no TLS-RPK", the one case that
//!   makes the §4.6 step 6 warning reachable (row D6 deviation; every bundle this build prints
//!   carries its SPKI).
//! - **The rules** (§4.6 step 2, C3): the scanning hub's pick must be the complement of the
//!   bundle's role (`peer` ↔ `peer`, `leaf` ↔ `gateway`) with the same `via`, and the pair must be
//!   one §4.6 allows (`carrier` ⇒ `peer`; `leaf`, `gateway` ⇒ `tcp`), else
//!   `409 pair_role_mismatch` and nothing is recorded ([`pick_refusal`], [`role_via_refusal`]).
//! - **The warning** (§4.6 step 6, D30 Q21; §4.1 rule 4): when TLS-RPK cannot be negotiated (the
//!   far bundle carries no link key), the pair is not marked `lan`, and the address the two hubs
//!   meet at is not private, the scan answers `409 consent_required` with `consent.action =
//!   pair_plain`, the subject the far NodeId and [`PAIR_PLAIN_WARNING`] verbatim; the repeat with
//!   `accept_warning` = its SHA-256 records the hash with the pair ([`PairLink::warning_hash`]);
//!   refusing pairs nothing. A `lan = true` pair runs plain with no consent (§4.1 rule 1).
//! - **Pin, then enrol both ways** (§4.6 steps 3–4; S22): the scan mints this hub's link key if
//!   it has none ([`LinkKey::load_or_mint`], §4.2), records this hub's half of the row
//!   ([`FabricHandle::set_pair_link`]: the far address, the far link SPKI pinned for the TLS
//!   wrap, `lan`, the warning hash, `far_holds_address`) and only then runs `Fabric::pair` (the
//!   `ENROLL` with the far code), so the pin is in place for its handshake. The `ENROLL` rides a
//!   connection already attributed to the far hub when one exists (the second `ENROLL` of a
//!   pairing rides the connection the first hub opened), else dials the bundle's address; it
//!   never rides a carrier (mesh111 enrols over `tcp-lan` only). A pin added or removed is read
//!   by the next handshake: the TLS wrap's server verifier reads the live pair table on every
//!   handshake and a dial builds its client configuration from it, so the cached TLS state
//!   (`relay::tls`, row D5b) holds only the link key and no stamp has to move for a pin.
//! - **One attempt in the background** ([`PairAttempts`]): the route waits at most
//!   [`PAIR_FIRST_WAIT`] for the attempt's first answer (a refusal is then the route's `502`, and
//!   nothing is left recorded), then answers `202` while the attempt goes on for at most
//!   [`PAIR_WINDOW`]: the far owner must pin this hub (scan its bundle) before a TLS dial is
//!   admitted, so the dial retries with the adapter's back-off until then. A bundle with no
//!   address and no connection yet waits for the far hub's own `ENROLL` to arrive here (a
//!   `FabricEvent::Settings` naming it with `theirs`), then enrols on that connection. When the
//!   window ends or the far hub refuses, the attempt is dropped (mesh111 releases its dial) and
//!   this hub's half is forgotten unless the fabric holds a record for the hub. The fabric has
//!   one enrolment slot: a newer pairing or enrolment supersedes an older one.
//! - **Unpair** ([`unpair`], §13.1 `DELETE /relay/paired/{node_id}`): the attempt in flight for
//!   the hub is dropped, `Fabric::unpair` drops the record and returns the forward rows held for it
//!   to the other targets (M4's `drop_pair` → `fwd.orphan`), and the half and its pinned key go
//!   ([`FabricHandle::forget_pair_link`]), so the next handshake from that hub is refused.
//!
//! **Never adds** (row D6): a dial, a record, a key or a task on a node without the hub role:
//! both routes answer `409 needs_hub` before anything is minted, pinned or started. On a hub the
//! only weight is the attempt an owner started, bounded by [`PAIR_WINDOW`].

use crate::daemon::components::consent::{ConsentRequired, ProblemWith};
use crate::daemon::dashboard::api_v1::Problem;
use crate::daemon::fabric::settings::{dial_refusal, LinkSpki, PairLink};
use crate::daemon::fabric::{self, json, FabricHandle};
use crate::daemon::relay::linkkey::LinkKey;
use crate::daemon::relay::tls::is_private;
use axum::http::StatusCode;
use id111::encoding::NodeIdDisplay;
use id111::keyset::NodeId;
use mesh111::contacts::{card_text, parse_card_text, verify_card, MAX_CARD_TEXT};
use mesh111::fabric::{FabricError, FabricEvent, PairRole, PairVia, PairView, PairedHub};
use mesh111::sealer::Sealer;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, oneshot};

/// The bundle's prefix (§4.6 step 1).
pub const PAIR_PREFIX: &str = "111PAIR:";
/// The longest bundle text read (the contract's `PairRequest.bundle` `maxLength`).
pub const BUNDLE_MAX: usize = 1024;
/// The longest address field (§4.6 step 1: 0–47, a full IPv6 literal in brackets and a port).
pub const ADDRESS_MAX: usize = 47;
/// The longest code field read (the code is 8 symbols, L5 §7.3; a typed form may carry dashes).
pub const CODE_MAX: usize = 16;
/// The D30 Q21 warning, verbatim (§4.6 step 6).
pub const PAIR_PLAIN_WARNING: &str = "This hub is on the open internet and this link cannot be \
encrypted. Anyone on the path between the two hubs can see which hubs talk, when, and how much. \
Message contents stay sealed. Pair anyway?";
/// How long the scan route waits for the attempt's first answer before it answers `202`.
pub const PAIR_FIRST_WAIT: Duration = Duration::from_secs(3);
/// How long one pairing attempt may wait for the far hub (its owner scans this hub's bundle,
/// its `ENROLL` arrives): then it is dropped.
pub const PAIR_WINDOW: Duration = Duration::from_secs(10 * 60);

// ---------------------------------------------------------------- the bundle

/// A parsed `111PAIR:` bundle (§4.6 step 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairBundle {
    /// The showing hub's card text (its stored announce, base32; verified by [`far_node`]).
    pub card: String,
    /// The one-time pairing code (L5 §7.3; normalized by the fabric).
    pub code: String,
    /// The showing hub's link key (§4.2); `None` when the field is empty.
    pub link_spki: Option<LinkSpki>,
    /// The showing hub's `tcp-lan` address; `None` when the field is empty.
    pub address: Option<SocketAddr>,
    /// The role the showing hub records for the scanning hub, from its own side.
    pub role: PairRole,
    /// The link.
    pub via: PairVia,
}

/// Why a bundle text was refused (each a `400`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleError {
    /// Longer than [`BUNDLE_MAX`].
    TooLong,
    /// It does not start with `111PAIR:`.
    Prefix,
    /// Fewer than the six fields.
    Fields,
    /// The card field is empty, too long or not base32.
    Card,
    /// The code field is empty, too long or holds other characters.
    Code,
    /// The link-SPKI field is neither empty nor 71 base32 characters of an Ed25519 SPKI.
    LinkSpki,
    /// The address is neither empty nor an IP literal with a port that names a peer.
    Address,
    /// Not `peer`, `leaf` or `gateway`.
    Role,
    /// Not `tcp` or `carrier`.
    Via,
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            BundleError::TooLong => "the bundle is longer than 1024 characters",
            BundleError::Prefix => "a pairing bundle starts with 111PAIR:",
            BundleError::Fields => {
                "a pairing bundle has six fields: card, code, link key, address, role, via"
            }
            BundleError::Card => "the bundle's card is not a card",
            BundleError::Code => "the bundle's code is not a pairing code",
            BundleError::LinkSpki => "the bundle's link key is not an Ed25519 key (71 characters)",
            BundleError::Address => {
                "the bundle's address is not ip:port (an IP literal, [v6]:port for IPv6) naming a peer"
            }
            BundleError::Role => "the bundle's role is not peer, leaf or gateway",
            BundleError::Via => "the bundle's link is not tcp or carrier",
        })
    }
}

impl std::error::Error for BundleError {}

fn base32_text(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
}

impl PairBundle {
    /// The bundle text (§4.6 step 1). The address of an IPv6 literal is bracketed
    /// (`SocketAddr`'s own form), the role and link are the lower-case words.
    pub fn text(&self) -> String {
        format!(
            "{PAIR_PREFIX}{}:{}:{}:{}:{}:{}",
            self.card,
            self.code,
            self.link_spki.map(|k| k.text()).unwrap_or_default(),
            self.address.map(|a| a.to_string()).unwrap_or_default(),
            self.role.code(),
            self.via.code(),
        )
    }

    /// Parse a bundle text: surrounding whitespace and the case of the prefix, role and link are
    /// ignored; three fields from the left, two from the right, the address the rest.
    pub fn parse(text: &str) -> Result<PairBundle, BundleError> {
        let t = text.trim();
        if t.len() > BUNDLE_MAX {
            return Err(BundleError::TooLong);
        }
        let head = t.get(..PAIR_PREFIX.len()).ok_or(BundleError::Prefix)?;
        if !head.eq_ignore_ascii_case(PAIR_PREFIX) {
            return Err(BundleError::Prefix);
        }
        let rest = &t[PAIR_PREFIX.len()..];
        let mut left = rest.splitn(4, ':');
        let (card, code, spki, tail) = match (left.next(), left.next(), left.next(), left.next()) {
            (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
            _ => return Err(BundleError::Fields),
        };
        let mut right = tail.rsplitn(3, ':');
        let (via, role, address) = match (right.next(), right.next(), right.next()) {
            (Some(v), Some(r), Some(a)) => (v, r, a),
            _ => return Err(BundleError::Fields),
        };
        if card.is_empty() || card.len() > MAX_CARD_TEXT || !base32_text(card) {
            return Err(BundleError::Card);
        }
        if code.is_empty()
            || code.len() > CODE_MAX
            || !code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(BundleError::Code);
        }
        let link_spki = match spki {
            "" => None,
            s => Some(LinkSpki::from_text(s).ok_or(BundleError::LinkSpki)?),
        };
        let address = match address {
            "" => None,
            a if a.len() > ADDRESS_MAX => return Err(BundleError::Address),
            a => {
                let s = a.parse::<SocketAddr>().map_err(|_| BundleError::Address)?;
                if dial_refusal(&s, false).is_some() {
                    return Err(BundleError::Address);
                }
                Some(s)
            }
        };
        let role = PairRole::from_code(&role.to_ascii_lowercase()).ok_or(BundleError::Role)?;
        let via = PairVia::from_code(&via.to_ascii_lowercase()).ok_or(BundleError::Via)?;
        Ok(PairBundle {
            card: card.to_string(),
            code: code.to_string(),
            link_spki,
            address,
            role,
            via,
        })
    }
}

/// The NodeId the bundle's card names, its signature checked (L5 §7.2: the card is the hub's
/// stored announce).
pub fn far_node(bundle: &PairBundle) -> Result<NodeId, mesh111::contacts::CardError> {
    Ok(verify_card(&parse_card_text(&bundle.card)?)?.node_id())
}

// ---------------------------------------------------------------- the rules

/// §4.6 step 2: a `carrier` pair is `peer` only; a `leaf` or `gateway` pair is `tcp` only.
pub fn role_via_refusal(role: PairRole, via: PairVia) -> Option<&'static str> {
    match (role, via) {
        (PairRole::Peer, _) | (_, PairVia::Tcp) => None,
        _ => Some("a carrier pair is a peer; a leaf or gateway pair is tcp only"),
    }
}

/// §4.6 step 2, C3: the scanning hub's pick (`role`, `via`, from its own side) against the
/// bundle's: the complement of its role, the same link, and a pair §4.6 allows.
pub fn pick_refusal(bundle: &PairBundle, role: PairRole, via: PairVia) -> Option<&'static str> {
    if bundle.role.complement() != role {
        return Some(
            "the role must be the complement of the bundle's: peer with peer, leaf with gateway",
        );
    }
    if bundle.via != via {
        return Some("the link must be the bundle's (tcp with tcp, carrier with carrier)");
    }
    role_via_refusal(role, via)
}

/// `409 pair_role_mismatch` (§4.6 step 2; nothing recorded).
pub fn mismatch(why: &str) -> Problem {
    Problem::new(
        StatusCode::CONFLICT,
        "pair_role_mismatch",
        "Those roles do not fit together.",
    )
    .detail(why.to_string())
}

/// `409 needs_hub` (§5.11: pairing needs the hub role).
pub fn needs_hub() -> Problem {
    Problem::new(
        StatusCode::CONFLICT,
        "needs_hub",
        "Only a hub pairs with another hub.",
    )
    .detail(
        "Turn on \"Let other nodes use this one as their home hub\" (Settings → Messaging & \
         Data), then pair.",
    )
}

fn restore_pending() -> Problem {
    Problem::new(
        StatusCode::CONFLICT,
        "restore_pending",
        "This node has no card yet, so it cannot pair.",
    )
    .detail("Nothing is announced until the identity restore completes (§4).")
}

fn bad_request(detail: impl Into<String>) -> Problem {
    Problem::new(
        StatusCode::BAD_REQUEST,
        "bad_request",
        "The request was refused.",
    )
    .detail(detail)
}

// ---------------------------------------------------------------- the warning

/// SHA-256 of [`PAIR_PLAIN_WARNING`]: what a pair accepted unencrypted records (§4.6 step 6).
pub fn pair_plain_hash() -> [u8; 32] {
    Sha256::digest(PAIR_PLAIN_WARNING.as_bytes()).into()
}

/// [`pair_plain_hash`] as 64 lowercase hex characters (the contract's `Sha256Hex`, the form
/// `accept_warning` echoes).
pub fn pair_plain_hash_hex() -> String {
    pair_plain_hash()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// §4.6 step 6, §4.1 rule 4: the pair would run plain over a non-private address. TLS cannot be
/// negotiated when the far bundle carries no link key; `lan` is the owner's mark (plain by
/// choice, no warning); `meet` is the address the two hubs meet at (the far bundle's, else the
/// one this hub shows), and an unknown one is not shown to be private.
pub fn needs_pair_plain(far_spki: Option<LinkSpki>, lan: bool, meet: Option<SocketAddr>) -> bool {
    far_spki.is_none() && !lan && !meet.is_some_and(|a| is_private(a.ip()))
}

/// The `consent` member of the `pair_plain` round trip (contract `ConsentRequired`).
pub fn pair_plain_consent(far: &NodeId) -> ConsentRequired {
    ConsentRequired {
        action: "pair_plain",
        subject: json::hex(far),
        warning_text: PAIR_PLAIN_WARNING.to_string(),
        hash: pair_plain_hash_hex(),
        addons: Vec::new(),
    }
}

/// `409 consent_required` with the `pair_plain` consent (§13.1; nothing recorded).
pub fn pair_plain_problem(far: &NodeId) -> ProblemWith {
    ProblemWith::new(
        Problem::new(
            StatusCode::CONFLICT,
            "consent_required",
            "This link cannot be encrypted: confirm first.",
        )
        .detail("Show the warning, then repeat the request with accept_warning = its hash."),
    )
    .with(
        "consent",
        serde_json::to_value(pair_plain_consent(far)).unwrap_or_default(),
    )
}

// ---------------------------------------------------------------- the row

/// Whether the far hub will hold this hub's address (§4.6 step 5; M5's
/// `PairLink::far_holds_address`), from `own` (the address this hub's bundle shows) and `far`
/// (the far bundle's). True only when the far hub can reach `own`: a public address; a private
/// one only beside a private far address of the same family and the same /24 (IPv4) or /64
/// (IPv6), a LAN; loopback only beside loopback (one host: tests). Otherwise false, the safe
/// side: this hub then dials whenever it holds the far address (two hubs that both read false
/// both dial, and keep two connections, rather than wait for each other).
pub fn far_holds_address(own: Option<SocketAddr>, far: Option<SocketAddr>) -> bool {
    let Some(own) = own.map(|a| a.ip().to_canonical()) else {
        return false;
    };
    let far = far.map(|a| a.ip().to_canonical());
    if own.is_loopback() {
        return far.is_some_and(|f| f.is_loopback());
    }
    if !is_private(own) {
        return true;
    }
    match (own, far) {
        (IpAddr::V4(o), Some(IpAddr::V4(f))) => {
            is_private(f.into()) && o.octets()[..3] == f.octets()[..3]
        }
        (IpAddr::V6(o), Some(IpAddr::V6(f))) => {
            is_private(f.into()) && o.segments()[..4] == f.segments()[..4]
        }
        _ => false,
    }
}

/// The contract's `PairTls` of a pair (§12): `none` for a `carrier` pair, a `lan` pair, or a
/// private meeting address; `rpk` when the far link key is pinned; else `plain` (the owner
/// accepted the warning).
pub fn tls_view(link: &PairLink, via: PairVia, meet: Option<SocketAddr>) -> &'static str {
    if via == PairVia::Carrier || link.lan || meet.is_some_and(|a| is_private(a.ip())) {
        "none"
    } else if link.link_spki.is_some() {
        "rpk"
    } else {
        "plain"
    }
}

/// The contract's `PairedHub` of one pair: this hub's half, the role and link from this hub's
/// side, the address it meets the far hub at, and the fabric's view when it has one (`reachable`,
/// `pending`, `reposts`; all off before the pair is recorded). D10's `GET /relay` may build its
/// `paired_hubs` with it.
pub fn paired_hub_json(
    link: &PairLink,
    role: PairRole,
    via: PairVia,
    meet: Option<SocketAddr>,
    view: Option<&PairView>,
) -> Value {
    let d = NodeIdDisplay::new(link.node_id.as_bytes());
    json!({
        "node_id": d.hex(),
        "label": d.label(),
        "role": role.code(),
        "via": via.code(),
        "tls": tls_view(link, via, meet),
        "lan": link.lan,
        "reachable": view.is_some_and(|v| v.reachable),
        "pending": view.map_or(0, |v| v.pending),
        "reposts": view.map_or(0, |v| v.reposts),
    })
}

// ---------------------------------------------------------------- the attempts

/// The pairing attempts in flight on this hub, one per far hub (module doc). Kept in
/// `ApiState` ext, created at the first pairing.
#[derive(Debug, Default)]
pub struct PairAttempts {
    inflight: tokio::sync::Mutex<HashMap<NodeId, Attempt>>,
    next: AtomicU64,
}

#[derive(Debug)]
struct Attempt {
    id: u64,
    task: tokio::task::AbortHandle,
}

impl PairAttempts {
    /// Whether an attempt for `far` is in flight.
    pub async fn in_flight(&self, far: &NodeId) -> bool {
        self.inflight
            .lock()
            .await
            .get(far)
            .is_some_and(|a| !a.task.is_finished())
    }
}

/// The attempt's first answer to the scan route.
enum First {
    /// `ENROLL_OK`: this hub's enrolment is recorded.
    Done(PairedHub),
    /// Waiting for the far hub's connection (no address to dial).
    Waiting,
    /// Refused; this hub's half is already forgotten unless the fabric records the hub.
    Refused(Problem),
}

/// What one attempt needs (owned: it outlives the route).
struct AttemptArgs {
    far: NodeId,
    address: Option<SocketAddr>,
    code: String,
    card: String,
    role: PairRole,
    via: PairVia,
    window: Duration,
}

/// A refusal of the scan route: a problem, or the consent round trip.
#[derive(Debug, Clone)]
pub enum PairRefusal {
    /// problem+json.
    Problem(Problem),
    /// `409 consent_required` with its `consent` member.
    Consent(ProblemWith),
}

impl From<Problem> for PairRefusal {
    fn from(p: Problem) -> Self {
        PairRefusal::Problem(p)
    }
}

impl axum::response::IntoResponse for PairRefusal {
    fn into_response(self) -> axum::response::Response {
        match self {
            PairRefusal::Problem(p) => p.into_response(),
            PairRefusal::Consent(p) => p.into_response(),
        }
    }
}

/// The scan request (§13.1 `PairRequest`), parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairPick {
    /// The far hub's bundle text.
    pub bundle: String,
    /// The far hub's role from this hub's side.
    pub role: PairRole,
    /// The link.
    pub via: PairVia,
    /// The owner's LAN or tunnel mark (§4.1 rule 1).
    pub lan: bool,
    /// The `pair_plain` hash the owner accepted (64 lowercase hex).
    pub accept_warning: Option<String>,
}

/// A fabric failure of an attempt, as the scan route answers it.
fn attempt_problem(e: &FabricError) -> Problem {
    use mesh111::mailbox::EnrollStatus;
    let upstream = |title: &str| Problem::new(StatusCode::BAD_GATEWAY, "upstream_error", title);
    match e {
        FabricError::Invalid("pairing needs the hub role") => needs_hub(),
        FabricError::Enroll(EnrollStatus::LockedOut) => {
            upstream("The other hub locked its offer after 5 wrong codes.")
                .detail("Show a fresh bundle on the other hub and scan that.")
        }
        FabricError::Enroll(s) => upstream("The other hub refused the pairing code.").detail(
            format!("{s:?}: the code is wrong, spent or dismissed; show a fresh bundle there."),
        ),
        FabricError::NoLink => upstream("No link can reach the other hub.")
            .detail("Check its address, and that this hub's messaging link is running."),
        other => fabric::problem(other),
    }
}

/// This hub's link key under the node root `root` (`<root>/secrets/link-key`, §4.2), minted at
/// the first pairing; the file work runs on a blocking thread.
async fn link_key(root: PathBuf) -> Result<LinkKey, Problem> {
    let failed = |e: String| {
        Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "This hub's link key could not be read or made.",
        )
        .detail(e)
    };
    match tokio::task::spawn_blocking(move || LinkKey::load_or_mint(&root)).await {
        Ok(Ok(k)) => Ok(k),
        Ok(Err(e)) => Err(failed(e.to_string())),
        Err(e) => Err(failed(e.to_string())),
    }
}

// ---------------------------------------------------------------- the routes' work

/// §4.6 step 1, `POST /relay/pair/offer {role, via}`: mint a pairing code (`Fabric::offer_pair`,
/// one open offer at a time, member or pairing; dismissed with `DELETE /fabric/hub/offer`) and
/// draw the bundle with this hub's current card, its link key (minted at the first pairing) and
/// its `tcp-lan` address (`FabricHandle::offer_address`, as the Layer 5 offer: the bound
/// listener, with an unspecified bind the lowest-numbered IPv4 of the node's interfaces). The
/// contract's `PairOffer`.
pub async fn offer(
    h: &Arc<FabricHandle>,
    root: &Path,
    role: PairRole,
    via: PairVia,
) -> Result<Value, Problem> {
    if let Some(why) = role_via_refusal(role, via) {
        return Err(mismatch(why));
    }
    let f = h.fabric().ok_or_else(fabric::not_running)?;
    if !f
        .settings()
        .await
        .map_err(|e| fabric::problem(&e))?
        .hub_role
    {
        return Err(needs_hub());
    }
    let announce = h.own_announce();
    if announce.is_empty() {
        return Err(restore_pending());
    }
    let key = link_key(root.to_path_buf()).await?;
    let address = h.offer_address().await;
    let view = f
        .offer_pair(address, role, via)
        .await
        .map_err(|e| match e {
            FabricError::Invalid("pairing needs the hub role") => needs_hub(),
            e => fabric::problem(&e),
        })?;
    let bundle = PairBundle {
        card: card_text(&announce),
        code: view.code.replace('-', ""),
        link_spki: Some(*key.spki()),
        address: view.address,
        role,
        via,
    };
    let drawn = json::card(&bundle.text()).map_err(|e| {
        Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "The bundle could not be drawn.",
        )
        .detail(e)
    })?;
    Ok(json!({
        "schema": 1,
        "bundle": drawn,
        "code": view.code,
        "link_spki": key.text(),
    }))
}

/// §4.6 steps 2–6, `POST /relay/pair`: check the pick, ask the warning when it applies, pin the
/// far hub, record this hub's half and start the attempt (module doc). The contract's
/// `PairedHub` (`202`): `reachable`, `pending` and `reposts` as the fabric last showed them.
pub async fn pair(
    h: &Arc<FabricHandle>,
    attempts: &Arc<PairAttempts>,
    root: &Path,
    pick: PairPick,
) -> Result<Value, PairRefusal> {
    pair_within(h, attempts, root, pick, PAIR_WINDOW).await
}

/// [`pair`] with the attempt's window given (tests).
pub async fn pair_within(
    h: &Arc<FabricHandle>,
    attempts: &Arc<PairAttempts>,
    root: &Path,
    pick: PairPick,
    window: Duration,
) -> Result<Value, PairRefusal> {
    let bundle = PairBundle::parse(&pick.bundle).map_err(|e| bad_request(e.to_string()))?;
    if let Some(why) = pick_refusal(&bundle, pick.role, pick.via) {
        return Err(mismatch(why).into());
    }
    let f = h.fabric().ok_or_else(fabric::not_running)?;
    let settings = f.settings().await.map_err(|e| fabric::problem(&e))?;
    if !settings.hub_role {
        return Err(needs_hub().into());
    }
    if h.own_announce().is_empty() {
        return Err(restore_pending().into());
    }
    let far = far_node(&bundle)
        .map_err(|e| bad_request(format!("the bundle's card does not verify: {e}")))?;
    if h.sealer().is_some_and(|s| s.node_id() == far) {
        return Err(
            bad_request("that is this hub's own bundle: a hub never pairs with itself").into(),
        );
    }
    if settings.home_hub.is_some_and(|x| x.node_id == far) {
        return Err(bad_request(
            "that hub is this node's home hub, and a home hub is never a paired hub",
        )
        .into());
    }
    if let Some(rec) = settings.paired_hubs.iter().find(|p| p.node_id == far) {
        if rec.whole() {
            return Err(Problem::new(
                StatusCode::CONFLICT,
                "bad_request",
                "This hub is already paired with that hub.",
            )
            .detail("Unpair it first (it then needs a fresh bundle from the other hub).")
            .into());
        }
        if rec.role != pick.role || rec.via != pick.via {
            return Err(mismatch(
                "that hub's pairing is recorded with another role or link: pick the recorded one, or unpair first",
            )
            .into());
        }
    }
    if let Some(why) = bundle
        .address
        .as_ref()
        .and_then(|a| dial_refusal(a, h.refuse_loopback()))
    {
        return Err(bad_request(format!("the bundle's address: {why}")).into());
    }
    let own = h.offer_address().await;
    let meet = bundle.address.or(own);
    let warning_hash = if needs_pair_plain(bundle.link_spki, pick.lan, meet) {
        match pick.accept_warning.as_deref() {
            Some(x) if x == pair_plain_hash_hex() => Some(pair_plain_hash()),
            _ => return Err(PairRefusal::Consent(pair_plain_problem(&far))),
        }
    } else {
        None
    };
    // §4.2: this hub's link key exists before its first TLS handshake.
    link_key(root.to_path_buf()).await?;
    let tcp = pick.via == PairVia::Tcp;
    let link = PairLink {
        node_id: far,
        address: bundle.address.filter(|_| tcp),
        link_spki: bundle.link_spki,
        lan: pick.lan,
        warning_hash,
        far_holds_address: tcp && far_holds_address(own, bundle.address),
    };
    let args = AttemptArgs {
        far,
        address: bundle.address,
        code: bundle.code.clone(),
        card: bundle.card.clone(),
        role: pick.role,
        via: pick.via,
        window,
    };
    let (tx, rx) = oneshot::channel();
    {
        let mut inflight = attempts.inflight.lock().await;
        if let Some(old) = inflight.remove(&far) {
            old.task.abort();
        }
        h.set_pair_link(link.clone()).await.map_err(|e| match e {
            FabricError::Invalid("pairing needs the hub role") => needs_hub(),
            e => fabric::problem(&e),
        })?;
        let id = attempts.next.fetch_add(1, Ordering::Relaxed);
        let task = tokio::spawn(run_attempt(
            Arc::clone(h),
            Arc::clone(attempts),
            id,
            args,
            tx,
        ));
        inflight.insert(
            far,
            Attempt {
                id,
                task: task.abort_handle(),
            },
        );
    }
    let first_wait = PAIR_FIRST_WAIT.min(h.enroll_timeout());
    let record = match tokio::time::timeout(first_wait, rx).await {
        Ok(Ok(First::Refused(p))) => return Err(p.into()),
        Ok(Ok(First::Done(rec))) => Some(rec),
        // waiting, still dialling, or replaced meanwhile by a newer scan: 202
        Ok(Ok(First::Waiting)) | Ok(Err(_)) | Err(_) => None,
    };
    let role = record.map_or(pick.role, |r| r.role);
    let via = record.map_or(pick.via, |r| r.via);
    let views = h.pair_views();
    let view = views.views.iter().find(|v| v.hub.node_id == far);
    Ok(paired_hub_json(&link, role, via, meet, view))
}

/// `DELETE /relay/paired/{node_id}`: drop the attempt in flight for the hub, the fabric's
/// record (`Fabric::unpair`: the forward rows held for it return to the other targets) and this
/// hub's half with its pinned key. Idempotent.
pub async fn unpair(
    h: &Arc<FabricHandle>,
    attempts: &Arc<PairAttempts>,
    far: NodeId,
) -> Result<(), Problem> {
    let f = h.fabric().ok_or_else(fabric::not_running)?;
    let mut inflight = attempts.inflight.lock().await;
    if let Some(old) = inflight.remove(&far) {
        old.task.abort();
    }
    f.unpair(far).await.map_err(|e| fabric::problem(&e))?;
    h.forget_pair_link(&far).await;
    drop(inflight);
    Ok(())
}

/// One attempt (module doc): `Fabric::pair` within the window; with no address to dial, wait
/// for the far hub's `ENROLL` to arrive here and enrol on its connection.
async fn run_attempt(
    h: Arc<FabricHandle>,
    attempts: Arc<PairAttempts>,
    id: u64,
    a: AttemptArgs,
    first: oneshot::Sender<First>,
) {
    let mut first = Some(first);
    let label = NodeIdDisplay::new(a.far.as_bytes()).label();
    let Some(f) = h.fabric() else {
        end_attempt(&h, &attempts, id, a.far, false).await;
        if let Some(tx) = first.take() {
            let _ = tx.send(First::Refused(fabric::not_running()));
        }
        return;
    };
    let deadline = tokio::time::Instant::now() + a.window;
    // subscribed before the first try: the far hub's ENROLL may land between the two
    let mut events = a.address.is_none().then(|| f.subscribe());
    let outcome = loop {
        let attempt = f.pair(a.address, &a.code, &a.card, a.role, a.via);
        match tokio::time::timeout_at(deadline, attempt).await {
            Ok(Ok(rec)) => break Ok(rec),
            Ok(Err(FabricError::NoLink)) if a.address.is_none() => {
                if let Some(tx) = first.take() {
                    let _ = tx.send(First::Waiting);
                }
                let Some(rx) = events.as_mut() else {
                    break Err(None);
                };
                if !far_enrolled_here(f, rx, &a.far, deadline).await {
                    break Err(None);
                }
            }
            Ok(Err(e)) => break Err(Some(e)),
            Err(_) => break Err(None),
        }
    };
    match outcome {
        Ok(rec) => {
            end_attempt(&h, &attempts, id, a.far, true).await;
            eprintln!(
                "111 relay: enrolled at hub {label} ({}): {}",
                a.via.code(),
                if rec.whole() {
                    "the pair is whole"
                } else {
                    "the pair forwards once that hub has enrolled here too"
                }
            );
            if let Some(tx) = first.take() {
                let _ = tx.send(First::Done(rec));
            }
        }
        Err(e) => {
            end_attempt(&h, &attempts, id, a.far, false).await;
            let problem = match &e {
                Some(e) => attempt_problem(e),
                None => Problem::new(
                    StatusCode::BAD_GATEWAY,
                    "upstream_error",
                    "The other hub did not answer in time.",
                ),
            };
            eprintln!(
                "111 relay: pairing with hub {label} ended: {}",
                problem.detail.as_deref().unwrap_or(&problem.title)
            );
            if let Some(tx) = first.take() {
                let _ = tx.send(First::Refused(problem));
            }
        }
    }
}

/// Wait (at most until `deadline`) for the far hub's `ENROLL` to be recorded here: a
/// `Settings` event whose record of `far` has `theirs` (after a lag, the settings re-read).
async fn far_enrolled_here(
    f: &mesh111::fabric::Fabric,
    rx: &mut broadcast::Receiver<FabricEvent>,
    far: &NodeId,
    deadline: tokio::time::Instant,
) -> bool {
    let theirs = |s: &mesh111::fabric::FabricSettings| {
        s.paired_hubs.iter().any(|p| p.node_id == *far && p.theirs)
    };
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Err(_) | Ok(Err(broadcast::error::RecvError::Closed)) => return false,
            Ok(Ok(FabricEvent::Settings(s))) if theirs(&s) => return true,
            Ok(Ok(_)) => {}
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
                if f.settings().await.is_ok_and(|s| theirs(&s)) {
                    return true;
                }
            }
        }
    }
}

/// The attempt `id` ended: its registry entry goes (if it is still its own) and, when it failed,
/// this hub's half of the row unless the fabric records the far hub. Under the registry lock, so
/// a newer scan of the same hub never has its half forgotten by an older attempt.
async fn end_attempt(
    h: &Arc<FabricHandle>,
    attempts: &Arc<PairAttempts>,
    id: u64,
    far: NodeId,
    enrolled: bool,
) {
    let mut inflight = attempts.inflight.lock().await;
    if inflight.get(&far).is_none_or(|x| x.id != id) {
        return;
    }
    inflight.remove(&far);
    if enrolled {
        return;
    }
    let recorded = match h.fabric() {
        Some(f) => f
            .settings()
            .await
            .map(|s| s.paired_hubs.iter().any(|p| p.node_id == far))
            .unwrap_or(true),
        None => true,
    };
    if !recorded {
        h.forget_pair_link(&far).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::fabric::settings::ED25519_SPKI_PREFIX;

    fn spki(fill: u8) -> LinkSpki {
        let mut der = ED25519_SPKI_PREFIX.to_vec();
        der.extend_from_slice(&[fill; 32]);
        LinkSpki::from_der(&der).unwrap()
    }

    fn bundle(address: Option<&str>, role: PairRole, via: PairVia) -> PairBundle {
        PairBundle {
            card: "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".into(),
            code: "BCDF2345".into(),
            link_spki: Some(spki(7)),
            address: address.map(|a| a.parse().unwrap()),
            role,
            via,
        }
    }

    #[test]
    fn the_bundle_round_trips_with_every_address_form() {
        for a in [
            None,
            Some("192.168.1.20:11183"),
            Some("203.0.113.7:11183"),
            Some("[2001:db8::7]:11183"),
            Some("[fd00:1:2:3:4:5:6:7]:65535"),
            Some("[fd12:3456:789a:bcde:f012:3456:789a:bcde]:65535"),
            Some("[::ffff:198.51.100.1]:11183"),
        ] {
            for role in PairRole::ALL {
                for via in PairVia::ALL {
                    let b = bundle(a, role, via);
                    let text = b.text();
                    assert!(text.starts_with("111PAIR:"), "{text}");
                    assert_eq!(PairBundle::parse(&text), Ok(b.clone()), "{text}");
                    // five separators follow the prefix (the address may hold more)
                    let fields = text[PAIR_PREFIX.len()..].matches(':').count();
                    assert!(fields >= 5, "{text}");
                }
            }
        }
        // no link key: the empty field reads as none, and prints back empty
        let mut b = bundle(Some("203.0.113.7:11183"), PairRole::Peer, PairVia::Tcp);
        b.link_spki = None;
        let text = b.text();
        assert!(
            text.contains("BCDF2345::203.0.113.7:11183:peer:tcp"),
            "{text}"
        );
        assert_eq!(PairBundle::parse(&text), Ok(b));
    }

    #[test]
    fn the_parser_takes_three_from_the_left_and_two_from_the_right() {
        let s = spki(1).text();
        let t = format!("  111pair:CARD:BCDF-2345:{s}:[2001:db8::1]:11183:LEAF:TCP \n");
        let b = PairBundle::parse(&t).unwrap();
        assert_eq!(b.card, "CARD");
        assert_eq!(b.code, "BCDF-2345");
        assert_eq!(b.link_spki, Some(spki(1)));
        assert_eq!(b.address, Some("[2001:db8::1]:11183".parse().unwrap()));
        assert_eq!((b.role, b.via), (PairRole::Leaf, PairVia::Tcp));
        let b = PairBundle::parse(&format!("111PAIR:CARD:CODE:{s}::peer:carrier")).unwrap();
        assert_eq!(b.address, None);
        assert_eq!(b.via, PairVia::Carrier);
    }

    #[test]
    fn malformed_bundles_are_refused_with_their_reason() {
        let s = spki(1).text();
        let cases: Vec<(String, BundleError)> = vec![
            (format!("111PAIR:{}", "A".repeat(1100)), BundleError::TooLong),
            ("111HUB:CARD:CODE:10.0.0.1:11183".into(), BundleError::Prefix),
            ("111PAI".into(), BundleError::Prefix),
            ("".into(), BundleError::Prefix),
            (format!("111PAIR:CARD:CODE:{s}"), BundleError::Fields),
            (format!("111PAIR:CARD:CODE:{s}:peer:tcp"), BundleError::Fields),
            (format!("111PAIR::CODE:{s}::peer:tcp"), BundleError::Card),
            (format!("111PAIR:CA_RD:CODE:{s}::peer:tcp"), BundleError::Card),
            (format!("111PAIR:CARD1:CODE:{s}::peer:tcp"), BundleError::Card),
            (format!("111PAIR:{}:CODE:{s}::peer:tcp", "A".repeat(600)), BundleError::Card),
            (format!("111PAIR:CARD::{s}::peer:tcp"), BundleError::Code),
            (format!("111PAIR:CARD:{}:{s}::peer:tcp", "B".repeat(17)), BundleError::Code),
            (format!("111PAIR:CARD:CO DE:{s}::peer:tcp"), BundleError::Code),
            ("111PAIR:CARD:CODE:ABC::peer:tcp".into(), BundleError::LinkSpki),
            (format!("111PAIR:CARD:CODE:{}::peer:tcp", "A".repeat(71)), BundleError::LinkSpki),
            (format!("111PAIR:CARD:CODE:{s}:hub.example:11183:peer:tcp"), BundleError::Address),
            (format!("111PAIR:CARD:CODE:{s}:10.0.0.1:peer:tcp"), BundleError::Address),
            (format!("111PAIR:CARD:CODE:{s}:10.0.0.1:0:peer:tcp"), BundleError::Address),
            (format!("111PAIR:CARD:CODE:{s}:0.0.0.0:11183:peer:tcp"), BundleError::Address),
            (format!("111PAIR:CARD:CODE:{s}:[::]:11183:peer:tcp"), BundleError::Address),
            (format!("111PAIR:CARD:CODE:{s}:224.0.0.1:11183:peer:tcp"), BundleError::Address),
            (format!("111PAIR:CARD:CODE:{s}:2001:db8::1:11183:peer:tcp"), BundleError::Address),
            (
                format!("111PAIR:CARD:CODE:{s}:[ffff:ffff:ffff:ffff:ffff:ffff:255.255.255.255]:11183:peer:tcp"),
                BundleError::Address,
            ),
            (format!("111PAIR:CARD:CODE:{s}::member:tcp"), BundleError::Role),
            (format!("111PAIR:CARD:CODE:{s}::peer:udp"), BundleError::Via),
            (format!("111PAIR:CARD:CODE:{s}::peer:"), BundleError::Via),
        ];
        for (text, want) in cases {
            assert_eq!(PairBundle::parse(&text), Err(want), "{text}");
            assert!(!want.to_string().is_empty());
        }
    }

    #[test]
    fn the_pick_is_the_complement_with_the_same_link() {
        use PairRole::*;
        use PairVia::*;
        for shown in PairRole::ALL {
            for shown_via in PairVia::ALL {
                let b = bundle(None, shown, shown_via);
                for role in PairRole::ALL {
                    for via in PairVia::ALL {
                        let complement = matches!(
                            (shown, role),
                            (Peer, Peer) | (Leaf, Gateway) | (Gateway, Leaf)
                        );
                        let allowed = via == Tcp || role == Peer;
                        let ok = complement && via == shown_via && allowed;
                        assert_eq!(
                            pick_refusal(&b, role, via).is_none(),
                            ok,
                            "{shown:?}/{shown_via:?} picked {role:?}/{via:?}"
                        );
                    }
                }
            }
        }
        // the via rules alone
        assert!(role_via_refusal(Peer, Carrier).is_none());
        assert!(role_via_refusal(Peer, Tcp).is_none());
        assert!(role_via_refusal(Leaf, Tcp).is_none());
        assert!(role_via_refusal(Gateway, Tcp).is_none());
        assert!(role_via_refusal(Leaf, Carrier).is_some());
        assert!(role_via_refusal(Gateway, Carrier).is_some());
        let p = mismatch("x");
        assert_eq!((p.status, p.code), (409, "pair_role_mismatch"));
        let p = needs_hub();
        assert_eq!((p.status, p.code), (409, "needs_hub"));
    }

    #[test]
    fn the_warning_applies_only_to_plain_over_a_non_private_address() {
        let public: SocketAddr = "203.0.113.7:11183".parse().unwrap();
        let lan: SocketAddr = "192.168.1.20:11183".parse().unwrap();
        let cgnat: SocketAddr = "100.64.0.9:11183".parse().unwrap();
        // no link key, not marked, public: the warning
        assert!(needs_pair_plain(None, false, Some(public)));
        // an unknown meeting address is not shown to be private
        assert!(needs_pair_plain(None, false, None));
        // TLS can run, the owner marked it a LAN or tunnel, or the address is private
        assert!(!needs_pair_plain(Some(spki(3)), false, Some(public)));
        assert!(!needs_pair_plain(None, true, Some(public)));
        assert!(!needs_pair_plain(None, false, Some(lan)));
        assert!(!needs_pair_plain(None, false, Some(cgnat)));
        // the text and its hash, verbatim (D30 Q21)
        assert_eq!(
            PAIR_PLAIN_WARNING,
            "This hub is on the open internet and this link cannot be encrypted. Anyone on the \
             path between the two hubs can see which hubs talk, when, and how much. Message \
             contents stay sealed. Pair anyway?"
        );
        let hex = pair_plain_hash_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        assert_eq!(
            hex,
            crate::daemon::bridge::settings::sha256_hex(PAIR_PLAIN_WARNING)
        );
        let far = NodeId::from([9; 32]);
        let c = pair_plain_consent(&far);
        assert_eq!(c.action, "pair_plain");
        assert_eq!(c.subject, "09".repeat(32));
        assert_eq!(c.warning_text, PAIR_PLAIN_WARNING);
        assert_eq!(c.hash, hex);
        let body = pair_plain_problem(&far).body();
        assert_eq!(body["code"], "consent_required");
        assert_eq!(body["status"], 409);
        assert_eq!(body["consent"]["action"], "pair_plain");
        assert_eq!(body["consent"]["hash"], hex.as_str());
    }

    #[test]
    fn far_holds_address_only_where_the_far_hub_can_reach_it() {
        let a = |s: &str| Some(s.parse::<SocketAddr>().unwrap());
        // nothing shown: the far hub holds nothing
        assert!(!far_holds_address(None, a("203.0.113.7:1")));
        // a public address: any far hub reaches it
        assert!(far_holds_address(
            a("198.51.100.2:11183"),
            a("192.168.1.9:11183")
        ));
        assert!(far_holds_address(a("198.51.100.2:11183"), None));
        assert!(far_holds_address(a("[2001:db8::2]:11183"), None));
        // a private address: only beside a private far address on the same /24 or /64
        assert!(far_holds_address(
            a("192.168.1.20:11183"),
            a("192.168.1.9:11183")
        ));
        assert!(!far_holds_address(
            a("192.168.1.20:11183"),
            a("192.168.7.9:11183")
        ));
        assert!(!far_holds_address(
            a("10.0.0.20:11183"),
            a("203.0.113.7:11183")
        ));
        assert!(!far_holds_address(a("10.0.0.20:11183"), None));
        assert!(!far_holds_address(
            a("100.64.0.1:11183"),
            a("100.64.9.1:11183")
        ));
        assert!(far_holds_address(
            a("[fd00::1]:11183"),
            a("[fd00::9]:11183")
        ));
        assert!(!far_holds_address(
            a("[fd00::1]:11183"),
            a("[fd00:0:0:1::9]:11183")
        ));
        assert!(!far_holds_address(
            a("192.168.1.20:11183"),
            a("[fd00::9]:11183")
        ));
        // loopback only beside loopback (one host)
        assert!(far_holds_address(a("127.0.0.1:2"), a("127.0.0.1:3")));
        assert!(!far_holds_address(a("127.0.0.1:2"), a("203.0.113.7:3")));
    }

    #[test]
    fn the_tls_view_names_how_the_link_runs() {
        let public: SocketAddr = "203.0.113.7:11183".parse().unwrap();
        let private: SocketAddr = "10.1.2.3:11183".parse().unwrap();
        let mut l = PairLink::new(NodeId::from([4; 32]));
        l.link_spki = Some(spki(4));
        assert_eq!(tls_view(&l, PairVia::Tcp, Some(public)), "rpk");
        assert_eq!(tls_view(&l, PairVia::Tcp, Some(private)), "none");
        assert_eq!(tls_view(&l, PairVia::Carrier, Some(public)), "none");
        l.lan = true;
        assert_eq!(tls_view(&l, PairVia::Tcp, Some(public)), "none");
        l.lan = false;
        l.link_spki = None;
        l.warning_hash = Some(pair_plain_hash());
        assert_eq!(tls_view(&l, PairVia::Tcp, Some(public)), "plain");
        let v = paired_hub_json(&l, PairRole::Gateway, PairVia::Tcp, Some(public), None);
        assert_eq!(v["node_id"], "04".repeat(32));
        assert_eq!(v["role"], "gateway");
        assert_eq!(v["via"], "tcp");
        assert_eq!(v["tls"], "plain");
        assert_eq!(v["lan"], false);
        assert_eq!(v["reachable"], false);
        assert_eq!(v["pending"], 0);
        assert_eq!(v["reposts"], 0);
        assert_eq!(v["label"].as_str().unwrap().len(), "xxxx-xxxx-xx".len());
    }

    #[test]
    fn attempt_failures_map_to_the_contract() {
        use mesh111::mailbox::EnrollStatus;
        let p = attempt_problem(&FabricError::Enroll(EnrollStatus::LockedOut));
        assert_eq!((p.status, p.code), (502, "upstream_error"));
        let p = attempt_problem(&FabricError::NoLink);
        assert_eq!((p.status, p.code), (502, "upstream_error"));
        let p = attempt_problem(&FabricError::Invalid("pairing needs the hub role"));
        assert_eq!((p.status, p.code), (409, "needs_hub"));
        let p = attempt_problem(&FabricError::Invalid("too many paired hubs"));
        assert_eq!((p.status, p.code), (400, "bad_request"));
    }
}
