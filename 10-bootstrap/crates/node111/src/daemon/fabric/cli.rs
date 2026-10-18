//! `111 fabric offer | enroll [bundle] | status` (`os/50-messaging/README.md`
//! §7.3: "`111 fabric offer` or Settings → Messaging & Data → Let a node use
//! this hub"). The CLI owns no fabric state: it calls the running daemon's
//! loopback `/api/v1` exactly like the dashboard on the node does —
//! session-less where the dashboard trusts loopback peers (not under Termux,
//! where `401` is reported with the way forward). `POST /fabric/hub/offer`
//! is loopback-only on the API, which this terminal is.
//!
//! The enrollment code is a secret until spent (it survives wrong attempts up
//! to the hub's lockout), so a **bare code is never taken on the command
//! line** (argv shows in `ps` and shell history): `111 fabric enroll
//! --address HOST:PORT` asks for it on the terminal, input hidden. The whole
//! `111HUB:` bundle a member scans from the hub's QR may ride argv (the
//! offer's own "On the member: …" line prints it so).
//!
//! The client is a tiny blocking HTTP/1.1 exchange over `std::net` (one
//! request, `Connection: close`, ≤ 1 MiB answer, bounded waits): no new
//! dependency, nothing but `127.0.0.1`. The answer is handled as bytes (a
//! chunked body is reassembled before any UTF-8 decoding — no input can
//! split a character and panic the client).

use anyhow::Context;
use mesh111::contacts::{HubBundle, BUNDLE_PREFIX};
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
}

/// Usage line (the top-level `USAGE` repeats it).
pub const USAGE: &str =
    "usage: 111 fabric status | offer | enroll [111HUB:BUNDLE] [--address HOST:PORT] [--card CARD]";

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
        _ => anyhow::bail!("{USAGE}"),
    }
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
/// (a bare enrollment code is read from the terminal).
pub fn run_on(port: u16, cmd: &FabricCmd, out: &mut dyn Write) -> anyhow::Result<()> {
    run_with(port, cmd, out, &mut read_code_from_tty)
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
                if yes(&v["reachable"]) {
                    "reachable"
                } else {
                    "not reachable right now"
                }
            )?;
        }
    }
    Ok(())
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
            if yes(&hub["reachable"]) {
                "reachable"
            } else {
                "not reachable"
            },
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
