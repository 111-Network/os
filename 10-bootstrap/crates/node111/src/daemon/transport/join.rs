//! Phone relay v1 (`os/60-transport/README.md` §11 "Phone relay v1"; D17 #4:
//! TCP to the hub first): the hub's **join descriptor** and the phone's side
//! that turns one into an rnsd `TCPClientInterface` entry.
//!
//! - **Hub.** With LAN sharing on and `[posture.lan].backbone.listen`, the
//!   rendered config carries a `BackboneInterface` listener on 4242
//!   (`config.rs`). [`join_descriptor`] answers `{schema, address, port,
//!   ifac_name?, ifac_passphrase?}` (contract `TransportJoin`), only for a
//!   paired or loopback session ([`Session`]; the route, `POST
//!   /transport/join`, is C1's and is `no-store`). The passphrase never rides
//!   in the QR (the QR carries address, port and a pair code; the
//!   descriptor is fetched over the paired session after the ceremony).
//! - **Phone.** [`apply_join`] validates a descriptor (untrusted input: every
//!   field checked before use) and writes an `owner = "rnsd"`, `type =
//!   "TCPClientInterface"` entry (`target_host`, `target_port`, `ifac`,
//!   `network_name`) through the spine, which renders and attaches it. The
//!   passphrase is stored under `secrets/` (0600), never in
//!   `interfaces.toml`, and never logged (`Debug` is redacted).

use super::config::{self, valid_passphrase, Secrets, IFAC_NAME};
use super::interfaces::{
    sanitize_name, valid_host, valid_netname, IfKind, IfOwner, Interface, InterfacesFile,
};
use super::posture::{Applied, ApplyError, Spine};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// `TransportJoin.schema`.
pub const JOIN_SCHEMA: u32 = 1;

/// The phone-relay join descriptor (contract `TransportJoin`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinDescriptor {
    pub schema: u32,
    /// The hub's LAN address.
    pub address: String,
    pub port: u16,
    /// IFAC network name; absent when IFAC is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ifac_name: Option<String>,
    /// IFAC passphrase; absent when IFAC is off. Never in a QR, never logged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ifac_passphrase: Option<String>,
}

impl std::fmt::Debug for JoinDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinDescriptor")
            .field("schema", &self.schema)
            .field("address", &self.address)
            .field("port", &self.port)
            .field("ifac_name", &self.ifac_name)
            .field(
                "ifac_passphrase",
                &self.ifac_passphrase.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// The session a descriptor is asked on (C1 maps the request to this).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    Loopback,
    Paired,
    /// Anything else (an unpaired LAN peer, the agent's bearer): refused.
    Other,
}

/// Why no descriptor was produced or applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinError {
    /// LAN sharing is off: no listener to join (`409`).
    LanOff,
    /// `backbone.listen` is off (`409`).
    NoListener,
    /// IFAC is on but the passphrase is missing from `secrets/`.
    IfacSecretMissing,
    /// Not a paired or loopback session.
    Untrusted,
    /// The address is not a usable unicast address.
    BadAddress,
    /// A descriptor field failed validation.
    BadDescriptor(&'static str),
    /// The phone could not store the entry.
    Apply(ApplyError),
    /// The passphrase could not be stored.
    Secret(String),
}

impl std::fmt::Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JoinError::LanOff => f.write_str("LAN sharing is off, so there is no relay to join"),
            JoinError::NoListener => f.write_str("the Backbone listener is off on this hub"),
            JoinError::IfacSecretMissing => f.write_str("IFAC is on but its passphrase is missing"),
            JoinError::Untrusted => f.write_str("only a paired or loopback session may join"),
            JoinError::BadAddress => f.write_str("the hub address is not a unicast address"),
            JoinError::BadDescriptor(why) => write!(f, "the join descriptor is invalid: {why}"),
            JoinError::Apply(e) => write!(f, "{e}"),
            JoinError::Secret(e) => write!(f, "the IFAC passphrase was not stored: {e}"),
        }
    }
}

impl std::error::Error for JoinError {}

fn usable(ip: IpAddr) -> bool {
    !(ip.is_loopback() || ip.is_unspecified() || ip.is_multicast())
        && !matches!(ip, IpAddr::V4(v4) if v4.is_broadcast())
}

/// The hub's descriptor (module doc). `address` is the hub's LAN address
/// the phone can reach (C1 picks it, e.g. the request's local address or
/// `dashboard::bind::lan_ipv4s()`).
pub fn join_descriptor(
    ifs: &InterfacesFile,
    secrets: &Secrets,
    lan_sharing: bool,
    address: IpAddr,
    session: Session,
) -> Result<JoinDescriptor, JoinError> {
    if session == Session::Other {
        return Err(JoinError::Untrusted);
    }
    if !lan_sharing {
        return Err(JoinError::LanOff);
    }
    let bb = &ifs.posture.lan.backbone;
    if !bb.listen {
        return Err(JoinError::NoListener);
    }
    if !usable(address) {
        return Err(JoinError::BadAddress);
    }
    let (ifac_name, ifac_passphrase) = if bb.ifac {
        let pass = secrets
            .ifac_passphrase()
            .ok_or(JoinError::IfacSecretMissing)?;
        (Some(IFAC_NAME.to_string()), Some(pass.to_string()))
    } else {
        (None, None)
    };
    Ok(JoinDescriptor {
        schema: JOIN_SCHEMA,
        address: address.to_string(),
        port: bb.port,
        ifac_name,
        ifac_passphrase,
    })
}

/// Check an untrusted descriptor field by field.
pub fn validate(d: &JoinDescriptor) -> Result<(), JoinError> {
    if d.schema != JOIN_SCHEMA {
        return Err(JoinError::BadDescriptor("unknown schema"));
    }
    if d.port == 0 {
        return Err(JoinError::BadDescriptor("port 0"));
    }
    match d.address.parse::<IpAddr>() {
        Ok(ip) if !usable(ip) => return Err(JoinError::BadAddress),
        Ok(_) => {}
        Err(_) if valid_host(&d.address) && d.address != "localhost" => {}
        Err(_) => return Err(JoinError::BadDescriptor("address")),
    }
    match (&d.ifac_name, &d.ifac_passphrase) {
        (None, None) => Ok(()),
        (Some(n), Some(p)) if valid_netname(n) && valid_passphrase(p) => Ok(()),
        (Some(_), Some(_)) => Err(JoinError::BadDescriptor("IFAC name or passphrase")),
        _ => Err(JoinError::BadDescriptor(
            "IFAC name and passphrase come together",
        )),
    }
}

/// The phone's entry for a (validated) descriptor: `111 hub <address>`.
pub fn client_entry(d: &JoinDescriptor) -> Result<Interface, JoinError> {
    validate(d)?;
    let name = sanitize_name(&format!("111 hub {}", d.address))
        .ok_or(JoinError::BadDescriptor("address"))?;
    let mut params = toml::Table::new();
    params.insert("target_host".into(), toml::Value::String(d.address.clone()));
    params.insert(
        "target_port".into(),
        toml::Value::Integer(i64::from(d.port)),
    );
    if let Some(n) = &d.ifac_name {
        params.insert("ifac".into(), toml::Value::Boolean(true));
        params.insert("network_name".into(), toml::Value::String(n.clone()));
    }
    Ok(Interface {
        name,
        owner: IfOwner::Rnsd,
        kind: IfKind::TcpClient,
        device: None,
        port: None,
        region: None,
        uplink: None,
        params,
    })
}

impl Spine {
    /// [`join_descriptor`] for this hub (secrets read, minted if absent).
    pub fn join_descriptor(
        &self,
        address: IpAddr,
        session: Session,
    ) -> Result<JoinDescriptor, JoinError> {
        let f = self.file();
        let secrets =
            Secrets::for_file(&self.layout().root, &f).map_err(|_| JoinError::IfacSecretMissing)?;
        join_descriptor(&f, &secrets, self.lan_sharing(), address, session)
    }
}

/// The phone side (module doc): store the passphrase, write the entry
/// (replacing an earlier join of the same hub), render and attach it.
/// Returns the entry's name and what the apply did.
pub async fn apply_join(spine: &Spine, d: &JoinDescriptor) -> Result<(String, Applied), JoinError> {
    let entry = client_entry(d)?;
    let name = entry.name.clone();
    if let Some(pass) = &d.ifac_passphrase {
        config::write_client_ifac(&spine.layout().root, &name, pass)
            .map_err(|e| JoinError::Secret(format!("{e:#}")))?;
    }
    let applied = spine
        .update(move |f| f.upsert(entry))
        .await
        .map_err(JoinError::Apply)?;
    Ok((name, applied))
}
