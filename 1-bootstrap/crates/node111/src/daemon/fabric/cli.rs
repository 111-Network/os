//! `111 fabric offer | enroll [bundle] | status` (`os/5-messaging/README.md`
//! §7.3: "`111 fabric offer` or Settings → Messaging & Data → Let a node use
//! this hub"), and Layer 7's `111 fabric pair --show` and `111 fabric pair
//! <111PAIR: bundle> [--lan]` (`os/7-relay/README.md` §4.6 steps 1–2, §13.1:
//! the phone case of the loopback-only pairing routes). The CLI owns no fabric
//! state: it calls the running daemon's loopback `/api/v1` exactly like the
//! dashboard on the node does — session-less where the dashboard trusts
//! loopback peers (not under Termux, where `401` is reported with the way
//! forward). `POST /fabric/hub/offer` and the pairing routes are
//! loopback-only on the API, which this terminal is.
//!
//! The enrollment code is a secret until spent (it survives wrong attempts up
//! to the hub's lockout), so a **bare code is never taken on the command
//! line** (argv shows in `ps` and shell history): `111 fabric enroll
//! --address HOST:PORT` asks for it on the terminal, input hidden. The whole
//! `111HUB:` bundle a member scans from the hub's QR may ride argv (the
//! offer's own "On the member: …" line prints it so), and so may a whole
//! `111PAIR:` bundle (`111 fabric pair --show` prints the line to run on the
//! other hub); a bare code never does.
//!
//! `111 fabric pair <bundle>` picks, unless told otherwise, the complement of
//! the bundle's role and the bundle's link (§4.6 step 2: `peer` with `peer`,
//! `leaf` with `gateway`); `--role` and `--via` pick explicitly, and a pick
//! that does not fit is the daemon's `409 pair_role_mismatch`. When the link
//! cannot be encrypted (§4.6 step 6) the daemon's warning is printed verbatim
//! and asked on the terminal; only a "yes" repeats the request with the
//! warning's hash, and without a terminal nothing is paired.
//!
//! The client is a tiny blocking HTTP/1.1 exchange over `std::net` (one
//! request, `Connection: close`, ≤ 1 MiB answer, bounded waits): no new
//! dependency, nothing but `127.0.0.1`. The answer is handled as bytes (a
//! chunked body is reassembled before any UTF-8 decoding — no input can
//! split a character and panic the client).

use crate::daemon::relay::pair::{PairBundle, PAIR_PREFIX};
use anyhow::Context;
use mesh111::contacts::{HubBundle, BUNDLE_PREFIX};
use mesh111::fabric::{PairRole, PairVia};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;
use zeroize::Zeroizing;

/// Largest answer read.
pub const MAX_ANSWER: u64 = 1 << 20;
/// Connect bound.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Answer bound (an enrollment waits for the hub, ≤ 20 s on the daemon side).
pub const READ_TIMEOUT: Duration = Duration::from_secs(40);

/// A parsed `111 fabric …` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FabricCmd {
    /// `111 fabric status`.
    Status,
    /// `111 fabric offer`.
    Offer,
    /// `111 fabric enroll [111HUB:BUNDLE] [--address host:port] [--card TEXT]`.
    Enroll {
        /// The hub's whole `111HUB:` bundle (card, code, address); `None` ⇒
        /// the code is read from the terminal and `address` is required.
        bundle: Option<String>,
        address: Option<String>,
        card: Option<String>,
    },
    /// `111 fabric pair --show [--role R] [--via V]` (Layer 7 §4.6 step 1):
    /// the role this hub proposes for the other hub, and the link.
    PairShow { role: PairRole, via: PairVia },
    /// `111 fabric pair 111PAIR:BUNDLE [--role R] [--via V] [--lan]` (§4.6
    /// steps 2–6): `None` picks the complement of the bundle's role and the
    /// bundle's link.
    Pair {
        bundle: String,
        role: Option<PairRole>,
        via: Option<PairVia>,
        lan: bool,
    },
}

/// Usage line (the top-level `USAGE` repeats it).
pub const USAGE: &str = "usage: 111 fabric status | offer | enroll [111HUB:BUNDLE] [--address HOST:PORT] [--card CARD]\n       \
     111 fabric pair --show [--role peer|leaf|gateway] [--via tcp|carrier] | pair 111PAIR:BUNDLE [--role R] [--via V] [--lan]";

/// A bare code where a pairing bundle belongs (module doc).
const NO_PAIR_CODE_ON_ARGV: &str = "a pairing code is never taken on the command line (it would \
     show in `ps` and shell history): pass the other hub's whole 111PAIR: bundle (its \
     `111 fabric pair --show` prints the line to run here)";

/// A bare code on the command line (module doc).
const NO_CODE_ON_ARGV: &str = "an enrollment code is never taken on the command line (it would \
     show in `ps` and shell history): run `111 fabric enroll --address HOST:PORT` and type the code \
     when asked, or pass the hub's whole 111HUB: bundle";

/// `(card, code, address)` of a `111HUB:` bundle; the address may be empty
/// (a hub not yet on the LAN offers `111HUB:<card>:<code>:`).
fn split_bundle(text: &str) -> Option<(String, String, Option<String>)> {
    if let Some(b) = HubBundle::parse(text) {
        return Some((b.card, b.code, Some(b.address)));
    }
    let t = text.trim();
    let head = t.get(..BUNDLE_PREFIX.len())?;
    if !head.eq_ignore_ascii_case(BUNDLE_PREFIX) {
        return None;
    }
    let mut it = t[BUNDLE_PREFIX.len()..].splitn(3, ':');
    let (card, code, address) = (it.next()?, it.next()?, it.next()?);
    (!card.is_empty() && !code.is_empty() && address.is_empty())
        .then(|| (card.to_string(), code.to_string(), None))
}

fn has_bundle_prefix(text: &str) -> bool {
    text.trim()
        .get(..BUNDLE_PREFIX.len())
        .is_some_and(|h| h.eq_ignore_ascii_case(BUNDLE_PREFIX))
}

/// Parse the words after `111 fabric`.
pub fn parse(args: &[String]) -> anyhow::Result<FabricCmd> {
    match args.first().map(String::as_str) {
        Some("status") if args.len() == 1 => Ok(FabricCmd::Status),
        Some("offer") if args.len() == 1 => Ok(FabricCmd::Offer),
        Some("enroll") => {
            let mut arg = None;
            let mut address = None;
            let mut card = None;
            let mut i = 1;
            while i < args.len() {
                let a = args[i].as_str();
                let mut value = |name: &str| -> anyhow::Result<String> {
                    i += 1;
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("{name} needs a value"))
                };
                if let Some(v) = a.strip_prefix("--address=") {
                    address = Some(v.to_string());
                } else if let Some(v) = a.strip_prefix("--card=") {
                    card = Some(v.to_string());
                } else if a == "--address" {
                    address = Some(value("--address")?);
                } else if a == "--card" {
                    card = Some(value("--card")?);
                } else if a.starts_with("--") {
                    anyhow::bail!("unknown fabric enroll flag {a:?}");
                } else if !has_bundle_prefix(a) {
                    anyhow::bail!("{NO_CODE_ON_ARGV}");
                } else if arg.is_none() {
                    anyhow::ensure!(
                        split_bundle(a).is_some(),
                        "that is not a 111HUB: bundle (card, code and address from the hub's offer)"
                    );
                    arg = Some(a.to_string());
                } else {
                    anyhow::bail!("{USAGE}");
                }
                i += 1;
            }
            let needs_address = arg
                .as_deref()
                .and_then(split_bundle)
                .is_none_or(|(_, _, a)| a.is_none());
            anyhow::ensure!(
                address.is_some() || !needs_address,
                "enrolling needs the hub's --address HOST:PORT (or its whole 111HUB: bundle)"
            );
            Ok(FabricCmd::Enroll {
                bundle: arg,
                address,
                card,
            })
        }
        Some("pair") => parse_pair(&args[1..]),
        _ => anyhow::bail!("{USAGE}"),
    }
}

fn has_pair_prefix(text: &str) -> bool {
    text.trim()
        .get(..PAIR_PREFIX.len())
        .is_some_and(|h| h.eq_ignore_ascii_case(PAIR_PREFIX))
}

/// The words after `111 fabric pair`.
fn parse_pair(args: &[String]) -> anyhow::Result<FabricCmd> {
    let (mut show, mut lan) = (false, false);
    let (mut role, mut via, mut bundle) = (None, None, None);
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let mut value = |name: &str| -> anyhow::Result<String> {
            i += 1;
            args.get(i)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("{name} needs a value"))
        };
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (a, None),
        };
        match flag {
            "--show" if inline.is_none() => show = true,
            "--lan" if inline.is_none() => lan = true,
            "--role" => {
                let v = match inline {
                    Some(v) => v,
                    None => value("--role")?,
                };
                role = Some(PairRole::from_code(&v).ok_or_else(|| {
                    anyhow::anyhow!("--role is peer, leaf or gateway, not {v:?}")
                })?);
            }
            "--via" => {
                let v = match inline {
                    Some(v) => v,
                    None => value("--via")?,
                };
                via = Some(
                    PairVia::from_code(&v)
                        .ok_or_else(|| anyhow::anyhow!("--via is tcp or carrier, not {v:?}"))?,
                );
            }
            f if f.starts_with("--") => anyhow::bail!("unknown fabric pair flag {a:?}"),
            _ if !has_pair_prefix(a) => anyhow::bail!("{NO_PAIR_CODE_ON_ARGV}"),
            _ if bundle.is_none() => {
                PairBundle::parse(a).map_err(|e| anyhow::anyhow!("that 111PAIR: bundle: {e}"))?;
                bundle = Some(a.trim().to_string());
            }
            _ => anyhow::bail!("{USAGE}"),
        }
        i += 1;
    }
    match (show, bundle) {
        (true, None) => {
            anyhow::ensure!(
                !lan,
                "--lan is the scanning side's mark: pass it with the other hub's bundle"
            );
            Ok(FabricCmd::PairShow {
                role: role.unwrap_or(PairRole::Peer),
                via: via.unwrap_or(PairVia::Tcp),
            })
        }
        (false, Some(bundle)) => Ok(FabricCmd::Pair {
            bundle,
            role,
            via,
            lan,
        }),
        (true, Some(_)) => {
            anyhow::bail!("--show prints this hub's bundle; scanning another's is a separate run")
        }
        (false, None) => anyhow::bail!("{USAGE}"),
    }
}

/// The `POST /relay/pair` body for `bundle` (the role and link picked as the
/// module doc says), with the accepted warning's hash on the repeat.
pub fn pair_body(
    bundle: &str,
    role: Option<PairRole>,
    via: Option<PairVia>,
    lan: bool,
    accept_warning: Option<&str>,
) -> anyhow::Result<Value> {
    let b = PairBundle::parse(bundle).map_err(|e| anyhow::anyhow!("that 111PAIR: bundle: {e}"))?;
    let mut body = serde_json::json!({
        "bundle": bundle,
        "role": role.unwrap_or(b.role.complement()).code(),
        "via": via.unwrap_or(b.via).code(),
        "lan": lan,
    });
    if let Some(h) = accept_warning {
        body["accept_warning"] = Value::String(h.to_string());
    }
    Ok(body)
}

/// The enrollment request body: a bundle carries card, code and address
/// (`--address` / `--card` override them); without one the code comes from
/// `code` (read from the terminal) and `--address` is required (`--card`
/// too unless the hub is already a contact).
pub fn enroll_body(
    bundle: Option<&str>,
    code: Option<&str>,
    address: Option<&str>,
    card: Option<&str>,
) -> anyhow::Result<Value> {
    let (b_card, b_code, b_address) = match bundle {
        Some(text) => {
            split_bundle(text).ok_or_else(|| anyhow::anyhow!("that is not a 111HUB: bundle"))?
        }
        None => (String::new(), String::new(), None),
    };
    let code = match bundle {
        Some(_) => b_code,
        None => code
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("{NO_CODE_ON_ARGV}"))?,
    };
    anyhow::ensure!(!code.trim().is_empty(), "no enrollment code was entered");
    let address = address.map(str::to_string).or(b_address).ok_or_else(|| {
        anyhow::anyhow!(
            "enrolling needs the hub's --address HOST:PORT (or its whole 111HUB: bundle)"
        )
    })?;
    let mut body = serde_json::json!({ "address": address, "code": code });
    if let Some(c) = card
        .map(str::to_string)
        .or((!b_card.is_empty()).then_some(b_card))
    {
        body["card"] = Value::String(c);
    }
    Ok(body)
}

/// One answer.
#[derive(Debug, Clone)]
pub struct Answer {
    pub status: u16,
    pub body: String,
}

impl Answer {
    fn json(&self) -> Option<Value> {
        serde_json::from_str(&self.body).ok()
    }

    /// The problem+json text, or the status.
    fn explain(&self) -> String {
        match self.json() {
            Some(p) if p.get("title").is_some() => {
                let title = p["title"].as_str().unwrap_or("");
                match p.get("detail").and_then(Value::as_str) {
                    Some(d) => format!("{title} {d}"),
                    None => title.to_string(),
                }
            }
            _ => format!("HTTP {}", self.status),
        }
    }
}

/// Reassemble a chunked body, byte-wise: a size line that is not hex, a
/// chunk shorter than its size, or the last chunk ends it (never a panic).
fn dechunk(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = raw;
    while let Some(eol) = rest.windows(2).position(|w| w == b"\r\n") {
        let n = std::str::from_utf8(&rest[..eol])
            .ok()
            .and_then(|s| s.split(';').next())
            .and_then(|s| usize::from_str_radix(s.trim(), 16).ok())
            .unwrap_or(0);
        let after = &rest[eol + 2..];
        let Some(chunk) = after.get(..n).filter(|_| n > 0) else {
            break;
        };
        out.extend_from_slice(chunk);
        let tail = &after[n..];
        rest = tail.strip_prefix(b"\r\n").unwrap_or(tail);
    }
    out
}

/// One request to the daemon's loopback API on `port`.
pub fn request(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> anyhow::Result<Answer> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut conn = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).with_context(|| {
        format!("the daemon's API on {addr} is not answering — is 111d running?")
    })?;
    conn.set_read_timeout(Some(READ_TIMEOUT))?;
    conn.set_write_timeout(Some(CONNECT_TIMEOUT))?;
    let payload = body.map(Value::to_string);
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nConnection: close\r\n"
    );
    if let Some(p) = &payload {
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            p.len()
        ));
    } else if method != "GET" {
        req.push_str("Content-Type: application/json\r\nContent-Length: 0\r\n");
    }
    req.push_str("\r\n");
    if let Some(p) = &payload {
        req.push_str(p);
    }
    conn.write_all(req.as_bytes())?;
    let mut raw = Vec::new();
    (&mut conn)
        .take(MAX_ANSWER + 16 * 1024)
        .read_to_end(&mut raw)
        .context("reading the daemon's answer")?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("the daemon's answer is not HTTP"))?;
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let rest = &raw[split + 4..];
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("the daemon's answer has no status"))?;
    let chunked = lines.any(|l| {
        l.split_once(':').is_some_and(|(k, v)| {
            k.trim().eq_ignore_ascii_case("transfer-encoding")
                && v.trim().eq_ignore_ascii_case("chunked")
        })
    });
    let body = if chunked {
        dechunk(rest)
    } else {
        rest.to_vec()
    };
    Ok(Answer {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn fail(a: &Answer) -> anyhow::Error {
    if a.status == 401 {
        return anyhow::anyhow!(
            "the dashboard API needs a paired session here (loopback is not trusted on this \
             device) — use Settings → Messaging & Data in a paired browser"
        );
    }
    anyhow::anyhow!("{}", a.explain())
}

fn yes(v: &Value) -> bool {
    v.as_bool().unwrap_or(false)
}

/// The enrollment code from the terminal, input hidden (never a pipe, never
/// argv — module doc).
pub fn read_code_from_tty() -> anyhow::Result<Zeroizing<String>> {
    anyhow::ensure!(
        unsafe { libc::isatty(0) } == 1,
        "the enrollment code is read from a terminal (never a pipe or the command line); \
         or pass the hub's whole 111HUB: bundle"
    );
    Ok(Zeroizing::new(crate::cli::read_hidden(
        "enrollment code from the hub (8 characters, input hidden): ",
    )?))
}

/// Run `cmd` against the API on `port`, writing the human report to `out`
/// (a bare enrollment code is read from the terminal; the pairing warning is
/// asked there).
pub fn run_on(port: u16, cmd: &FabricCmd, out: &mut dyn Write) -> anyhow::Result<()> {
    run_with(port, cmd, out, &mut read_code_from_tty)
}

/// The owner's answer to the pairing warning (§4.6 step 6), on the terminal:
/// the text verbatim, then `[y/N]`; no terminal is a "no".
pub fn confirm_on_tty(text: &str) -> anyhow::Result<bool> {
    anyhow::ensure!(
        unsafe { libc::isatty(0) } == 1,
        "this link cannot be encrypted, and the warning needs an answer on a terminal: {text}"
    );
    eprint!("\n{text} [y/N] ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let a = line.trim();
    Ok(a.eq_ignore_ascii_case("y") || a.eq_ignore_ascii_case("yes"))
}

/// `111 fabric pair …` against the API on `port`; `confirm` answers the
/// pairing warning (module doc).
pub fn run_pair_with(
    port: u16,
    cmd: &FabricCmd,
    out: &mut dyn Write,
    confirm: &mut dyn FnMut(&str) -> anyhow::Result<bool>,
) -> anyhow::Result<()> {
    match cmd {
        FabricCmd::PairShow { role, via } => {
            let body = serde_json::json!({ "role": role.code(), "via": via.code() });
            let a = request(port, "POST", "/api/v1/relay/pair/offer", Some(&body))?;
            if a.status != 200 {
                return Err(fail(&a));
            }
            let v = a
                .json()
                .ok_or_else(|| anyhow::anyhow!("unreadable pairing offer"))?;
            let bundle = v["bundle"]["text"].as_str().unwrap_or("");
            let shown = PairBundle::parse(bundle).ok();
            writeln!(
                out,
                "Pairing code: {}   (single use; shown once)",
                v["code"].as_str().unwrap_or("?")
            )?;
            writeln!(
                out,
                "Proposed:     the other hub is this hub's {} over {}",
                role.code(),
                via.code()
            )?;
            match shown.as_ref().and_then(|b| b.address) {
                Some(addr) => writeln!(out, "Hub address:  {addr}")?,
                None => writeln!(
                    out,
                    "Hub address:  none (the other hub must reach this one, or dial it first)"
                )?,
            }
            writeln!(
                out,
                "Link key:     {}",
                v["link_spki"].as_str().unwrap_or("?")
            )?;
            writeln!(out, "On the other hub: 111 fabric pair '{bundle}'")?;
            if let Ok(qr) = crate::cli::render_qr(bundle) {
                writeln!(out, "\nOr scan:\n{qr}")?;
            }
        }
        FabricCmd::Pair {
            bundle,
            role,
            via,
            lan,
        } => {
            let body = pair_body(bundle, *role, *via, *lan, None)?;
            let mut a = request(port, "POST", "/api/v1/relay/pair", Some(&body))?;
            if a.status == 409 {
                if let Some(c) = a
                    .json()
                    .filter(|p| p["code"] == "consent_required")
                    .map(|p| p["consent"].clone())
                {
                    let text = c["warning_text"].as_str().unwrap_or("");
                    let hash = c["hash"].as_str().unwrap_or("");
                    if !confirm(text)? {
                        writeln!(out, "Not paired.")?;
                        return Ok(());
                    }
                    let again = pair_body(bundle, *role, *via, *lan, Some(hash))?;
                    a = request(port, "POST", "/api/v1/relay/pair", Some(&again))?;
                }
            }
            if a.status != 202 {
                return Err(fail(&a));
            }
            let v = a.json().unwrap_or(Value::Null);
            writeln!(
                out,
                "Pairing with hub {} ({}) as this hub's {} over {}, link {}{}.",
                v["label"].as_str().unwrap_or("?"),
                v["node_id"].as_str().unwrap_or("?"),
                v["role"].as_str().unwrap_or("?"),
                v["via"].as_str().unwrap_or("?"),
                v["tls"].as_str().unwrap_or("?"),
                if yes(&v["lan"]) {
                    ", marked LAN or tunnel"
                } else {
                    ""
                },
            )?;
            writeln!(
                out,
                "The pair forwards once both hubs have enrolled: scan this hub's bundle on the \
                 other hub too (111 fabric pair --show here)."
            )?;
        }
        _ => anyhow::bail!("{USAGE}"),
    }
    Ok(())
}

/// [`run_on`] with the source of a bare enrollment code supplied.
pub fn run_with(
    port: u16,
    cmd: &FabricCmd,
    out: &mut dyn Write,
    read_code: &mut dyn FnMut() -> anyhow::Result<Zeroizing<String>>,
) -> anyhow::Result<()> {
    match cmd {
        FabricCmd::Status => {
            let a = request(port, "GET", "/api/v1/fabric", None)?;
            if a.status != 200 {
                return Err(fail(&a));
            }
            let v = a
                .json()
                .ok_or_else(|| anyhow::anyhow!("unreadable status"))?;
            write_status(&v, out)?;
        }
        FabricCmd::Offer => {
            let a = request(
                port,
                "POST",
                "/api/v1/fabric/hub/offer",
                Some(&serde_json::json!({})),
            )?;
            if a.status != 200 {
                return Err(fail(&a));
            }
            let v = a
                .json()
                .ok_or_else(|| anyhow::anyhow!("unreadable offer"))?;
            let code = v["code"].as_str().unwrap_or("?");
            let bundle = v["bundle"]["text"].as_str().unwrap_or("");
            writeln!(out, "Enrollment code: {code}   (single use; shown once)")?;
            match v["address"].as_str() {
                Some(addr) => writeln!(out, "Hub address:     {addr}")?,
                None => writeln!(
                    out,
                    "Hub address:     none on the LAN yet — turn on the LAN listener (Settings → Messaging & Data)"
                )?,
            }
            writeln!(out, "On the member: 111 fabric enroll '{bundle}'")?;
            if let Ok(qr) = crate::cli::render_qr(bundle) {
                writeln!(out, "\nOr scan:\n{qr}")?;
            }
        }
        FabricCmd::Enroll {
            bundle,
            address,
            card,
        } => {
            let code = match bundle {
                Some(_) => None,
                None => Some(read_code()?),
            };
            let body = enroll_body(
                bundle.as_deref(),
                code.as_deref().map(String::as_str),
                address.as_deref(),
                card.as_deref(),
            )?;
            let a = request(port, "POST", "/api/v1/fabric/hub/enroll", Some(&body))?;
            if a.status != 200 {
                return Err(fail(&a));
            }
            let v = a.json().unwrap_or(Value::Null);
            writeln!(
                out,
                "Enrolled with hub {} at {} ({}).",
                v["node_id"].as_str().unwrap_or("?"),
                v["address"].as_str().unwrap_or("?"),
                hub_state(&v)
            )?;
        }
        FabricCmd::PairShow { .. } | FabricCmd::Pair { .. } => {
            run_pair_with(port, cmd, out, &mut confirm_on_tty)?;
        }
    }
    Ok(())
}

/// The three honest states of the home hub (`FabricHub`): `reachable` (the
/// hub answered on the current connection), `connected` without it (a
/// connection this node may send on is up, the hub has not answered yet), and
/// neither. A node never says "not reachable" while it is connected and
/// merely waiting.
fn hub_state(hub: &Value) -> &'static str {
    if yes(&hub["reachable"]) {
        "reachable"
    } else if yes(&hub["connected"]) {
        "connected, waiting for the hub's answer"
    } else {
        "not connected"
    }
}

fn write_status(v: &Value, out: &mut dyn Write) -> std::io::Result<()> {
    let store = &v["store"];
    if !yes(&store["open"]) {
        writeln!(
            out,
            "Messaging: not running (the message store is not open)"
        )?;
    } else {
        writeln!(
            out,
            "Messaging: running — store {} of {} bytes{}",
            store["used"],
            store["watermark"],
            if yes(&store["store_full"]) {
                " (FULL: new custody and bulk are refused)"
            } else {
                ""
            }
        )?;
    }
    let hub = &v["hub"];
    if yes(&hub["enrolled"]) {
        writeln!(
            out,
            "Home hub:  {} at {} — {}, cursor {}",
            hub["node_id"].as_str().unwrap_or("?"),
            hub["address"].as_str().unwrap_or("?"),
            hub_state(hub),
            hub["cursor"]
        )?;
    } else {
        writeln!(
            out,
            "Home hub:  none (111 fabric enroll '<111HUB: bundle>', or --address HOST:PORT and type the code)"
        )?;
    }
    let s = &v["settings"];
    let on = |b: &Value| if yes(b) { "on" } else { "off" };
    writeln!(
        out,
        "Hub role:  {} ({} members{}) · LAN listener {} · custody opt-in {}",
        on(&s["hub_role"]),
        hub["members"],
        if yes(&hub["offer_active"]) {
            ", an offer is open"
        } else {
            ""
        },
        on(&s["listen_lan"]),
        on(&s["custody_opt_in"]),
    )?;
    writeln!(
        out,
        "Custody:   {} items, {} bytes held for members",
        v["custody"]["count"], v["custody"]["bytes"]
    )?;
    for l in v["links"].as_array().into_iter().flatten() {
        writeln!(
            out,
            "Link:      {} ({}) {} — {} peers{}",
            l["id"].as_str().unwrap_or("?"),
            l["kind"].as_str().unwrap_or("?"),
            l["state"].as_str().unwrap_or("?"),
            l["peers"],
            l["detail"]
                .as_str()
                .map(|d| format!(": {d}"))
                .unwrap_or_default()
        )?;
    }
    Ok(())
}

/// `111 fabric …` against the local daemon (`:11180`).
pub fn run(cmd: &FabricCmd) -> anyhow::Result<()> {
    let mut out = std::io::stdout();
    run_on(crate::daemon::dashboard::DEFAULT_PORT, cmd, &mut out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    const BUNDLE: &str = "111HUB:CARDTEXT:ABCDEFGH:192.168.1.4:11183";

    /// A `GET /api/v1/fabric` body with the given hub flags (enrolled).
    fn status_with_hub(connected: bool, reachable: bool) -> Value {
        serde_json::json!({
            "store": { "open": true, "used": 10, "watermark": 100, "store_full": false },
            "hub": {
                "enrolled": true,
                "connected": connected,
                "reachable": reachable,
                "cursor": 7,
                "node_id": "ab".repeat(32),
                "address": "10.0.0.1:11183",
                "members": 0,
                "offer_active": false
            },
            "settings": { "hub_role": false, "listen_lan": true, "custody_opt_in": false },
            "custody": { "count": 0, "bytes": 0 },
            "links": []
        })
    }

    fn home_hub_line(v: &Value) -> String {
        let mut out = Vec::new();
        write_status(v, &mut out).unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .find(|l| l.starts_with("Home hub:"))
            .expect("a Home hub line")
            .to_string()
    }

    /// L5 fix D: the status line tells the three hub states apart; a member
    /// that is connected and waiting is not "not reachable".
    #[test]
    fn status_tells_reachable_waiting_and_not_connected_apart() {
        let tail = |c, r| {
            let l = home_hub_line(&status_with_hub(c, r));
            l.split(" — ")
                .nth(1)
                .expect("the state follows the address")
                .to_string()
        };
        assert_eq!(tail(true, true), "reachable, cursor 7");
        assert_eq!(
            tail(true, false),
            "connected, waiting for the hub's answer, cursor 7"
        );
        assert_eq!(tail(false, false), "not connected, cursor 7");
        // an older daemon that does not send `connected` reads as not connected
        let mut old = status_with_hub(false, false);
        old["hub"].as_object_mut().unwrap().remove("connected");
        assert!(home_hub_line(&old).contains("not connected, cursor 7"));
        // no home hub: the enrollment hint, whatever the flags say
        let mut none = status_with_hub(false, false);
        none["hub"]["enrolled"] = Value::Bool(false);
        assert!(home_hub_line(&none).starts_with("Home hub:  none"));
    }

    #[test]
    fn enroll_result_uses_the_same_three_states() {
        for (c, r, want) in [
            (true, true, "reachable"),
            (true, false, "connected, waiting for the hub's answer"),
            (false, false, "not connected"),
        ] {
            assert_eq!(hub_state(&status_with_hub(c, r)["hub"]), want);
        }
    }

    #[test]
    fn parse_matrix() {
        assert_eq!(parse(&s(&["status"])).unwrap(), FabricCmd::Status);
        assert_eq!(parse(&s(&["offer"])).unwrap(), FabricCmd::Offer);
        // a bundle may ride argv (with an address override)
        assert_eq!(
            parse(&s(&["enroll", BUNDLE, "--address", "10.0.0.1:11183"])).unwrap(),
            FabricCmd::Enroll {
                bundle: Some(BUNDLE.into()),
                address: Some("10.0.0.1:11183".into()),
                card: None
            }
        );
        // no bundle: the code is typed later, the address is required
        assert_eq!(
            parse(&s(&["enroll", "--card=XYZ", "--address=10.0.0.1:11183"])).unwrap(),
            FabricCmd::Enroll {
                bundle: None,
                address: Some("10.0.0.1:11183".into()),
                card: Some("XYZ".into())
            }
        );
        // an address-less bundle (hub not on the LAN yet) needs --address
        assert!(parse(&s(&["enroll", "111HUB:CARD:CODE:"])).is_err());
        assert!(parse(&s(&[
            "enroll",
            "111HUB:CARD:CODE:",
            "--address",
            "10.0.0.1:1"
        ]))
        .is_ok());
        for bad in [
            &["status", "x"][..],
            &["offer", "now"],
            &["enroll"],
            &["enroll", BUNDLE, BUNDLE],
            &["enroll", BUNDLE, "--address"],
            &["enroll", BUNDLE, "--bogus"],
            &["enroll", "111HUB:junk"],
            &["nope"],
            &[],
        ] {
            assert!(parse(&s(bad)).is_err(), "{bad:?}");
        }
    }

    /// F2 review m4: a bare code on argv is refused (it would sit in `ps`
    /// and shell history), with the way forward.
    #[test]
    fn a_bare_code_is_never_taken_from_argv() {
        for argv in [
            &["enroll", "ABCD-EFGH", "--address", "10.0.0.1:11183"][..],
            &["enroll", "--address", "10.0.0.1:11183", "ABCDEFGH"],
            &["enroll", "--card=XYZ", "C"],
        ] {
            let e = parse(&s(argv)).unwrap_err().to_string();
            assert!(e.contains("never taken on the command line"), "{e}");
            assert!(e.contains("type the code"), "{e}");
        }
    }

    #[test]
    fn enroll_bodies() {
        let b = enroll_body(Some(BUNDLE), None, None, None).unwrap();
        assert_eq!(b["address"], "192.168.1.4:11183");
        assert_eq!(b["code"], "ABCDEFGH");
        assert_eq!(b["card"], "CARDTEXT");
        let b = enroll_body(Some(BUNDLE), None, Some("10.0.0.9:1"), Some("OTHER")).unwrap();
        assert_eq!(
            (b["address"].as_str(), b["card"].as_str()),
            (Some("10.0.0.9:1"), Some("OTHER"))
        );
        let b = enroll_body(None, Some("ABCD-EFGH"), Some("10.0.0.1:11183"), None).unwrap();
        assert_eq!(b["code"], "ABCD-EFGH");
        assert!(b.get("card").is_none());
        let b = enroll_body(Some("111HUB:CARD:CODE:"), None, Some("10.0.0.1:1"), None).unwrap();
        assert_eq!(
            (b["code"].as_str(), b["card"].as_str()),
            (Some("CODE"), Some("CARD"))
        );
        assert!(enroll_body(None, Some("ABCD-EFGH"), None, None).is_err());
        assert!(enroll_body(None, None, Some("10.0.0.1:1"), None).is_err());
        assert!(enroll_body(None, Some("  "), Some("10.0.0.1:1"), None).is_err());
        assert!(enroll_body(Some("111HUB:CARD:CODE:"), None, None, None).is_err());
    }

    /// A `111PAIR:` bundle for the parser tests (shape only: the daemon
    /// verifies the card).
    fn pair_bundle(role: &str, via: &str) -> String {
        let spki = crate::daemon::fabric::settings::LinkSpki::from_der(
            &[
                &crate::daemon::fabric::settings::ED25519_SPKI_PREFIX[..],
                &[3u8; 32][..],
            ]
            .concat(),
        )
        .unwrap()
        .text();
        format!("111PAIR:CARDTEXT:BCDF2345:{spki}:[2001:db8::7]:11183:{role}:{via}")
    }

    /// Layer 7 §4.6: `111 fabric pair --show` and `111 fabric pair <bundle>`.
    #[test]
    fn pair_parse_matrix() {
        assert_eq!(
            parse(&s(&["pair", "--show"])).unwrap(),
            FabricCmd::PairShow {
                role: PairRole::Peer,
                via: PairVia::Tcp
            }
        );
        assert_eq!(
            parse(&s(&["pair", "--show", "--role", "leaf", "--via=tcp"])).unwrap(),
            FabricCmd::PairShow {
                role: PairRole::Leaf,
                via: PairVia::Tcp
            }
        );
        let b = pair_bundle("gateway", "tcp");
        assert_eq!(
            parse(&s(&["pair", &b, "--lan"])).unwrap(),
            FabricCmd::Pair {
                bundle: b.clone(),
                role: None,
                via: None,
                lan: true
            }
        );
        assert_eq!(
            parse(&s(&["pair", "--role=leaf", &b])).unwrap(),
            FabricCmd::Pair {
                bundle: b.clone(),
                role: Some(PairRole::Leaf),
                via: None,
                lan: false
            }
        );
        for bad in [
            vec!["pair"],
            vec!["pair", "--show", &b],
            vec!["pair", "--show", "--lan"],
            vec!["pair", "--show", "--role", "member"],
            vec!["pair", "--show", "--via"],
            vec!["pair", &b, &b],
            vec!["pair", &b, "--bogus"],
            vec!["pair", "111PAIR:junk"],
            vec!["pair", "--show=yes"],
        ] {
            assert!(parse(&s(&bad)).is_err(), "{bad:?}");
        }
        // a bare code never rides argv
        let e = parse(&s(&["pair", "BCDF-2345"])).unwrap_err().to_string();
        assert!(e.contains("never taken on the command line"), "{e}");
    }

    /// §4.6 step 2: the default pick is the complement of the bundle's role
    /// and the bundle's link; the repeat carries the accepted hash.
    #[test]
    fn pair_bodies_pick_the_complement() {
        for (shown, picked) in [("peer", "peer"), ("leaf", "gateway"), ("gateway", "leaf")] {
            let b = pair_bundle(shown, "tcp");
            let v = pair_body(&b, None, None, false, None).unwrap();
            assert_eq!(v["role"], picked);
            assert_eq!(v["via"], "tcp");
            assert_eq!(v["lan"], false);
            assert!(v.get("accept_warning").is_none());
            assert_eq!(v["bundle"], b.as_str());
        }
        let b = pair_bundle("peer", "carrier");
        let h = "ab".repeat(32);
        let v = pair_body(&b, Some(PairRole::Leaf), None, true, Some(&h)).unwrap();
        assert_eq!(
            (v["role"].as_str(), v["via"].as_str(), v["lan"].as_bool()),
            (Some("leaf"), Some("carrier"), Some(true))
        );
        assert_eq!(v["accept_warning"], h.as_str());
        assert!(pair_body("111PAIR:x", None, None, false, None).is_err());
    }

    /// F2 review m4: chunked bodies are reassembled as bytes — a multibyte
    /// character split across chunks, a lying size or junk never panic.
    #[test]
    fn dechunk_is_byte_safe() {
        assert_eq!(
            dechunk(b"1\r\n\xc3\r\n3\r\n\xa9 \xe2\r\n2\r\n\x80\x94\r\n0\r\n\r\n"),
            "é —".as_bytes()
        );
        assert_eq!(dechunk(b"4;ext=1\r\nabcd\r\n0\r\n\r\n"), b"abcd");
        assert_eq!(
            dechunk(b"10\r\n\xc3\xa9"),
            b"",
            "a chunk shorter than its size"
        );
        assert_eq!(dechunk(b"zz\r\nabc"), b"");
        assert_eq!(dechunk(b"\xff\xfe\r\n"), b"");
        assert_eq!(dechunk(b""), b"");
        assert_eq!(dechunk(b"2\r\nab"), b"ab", "a last chunk without its CRLF");
    }
}
