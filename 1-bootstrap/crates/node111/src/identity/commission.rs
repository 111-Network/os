//! `111 identity commission` — the CLI half of the Cat-1 commissioning
//! ceremony (L2 spec §8.3; owner of the ceremony itself is
//! `id111::commission`). This module drives the pure state machine and owns
//! every side effect the state machine refuses to have: the TTY gate, the
//! words/QR/NodeId display, the interactive type-back (hidden input), and the
//! event log.
//!
//! Hard rules carried from the spec:
//!
//! - **TTY only** (§8.3/§12 rule 10): the words land on stdout and the
//!   type-back reads stdin, so BOTH must be a terminal — a pipe is refused
//!   before any keygen happens.
//! - **Verify-then-provision** (§8.3): the bundle is only requested after the
//!   operator typed back the sampled words; the state machine enforces it.
//! - **Never print the keyset.** Only the words and the project QR payload
//!   (the backup, §8.3 step 2) are secret output, and they exist to be written
//!   down; the provisioning bundle is shown as its NON-secret summary
//!   (NodeId, descriptor, challenge nonce).
//! - **No relay transport yet (§15).** Until the USB-UART/BLE byte protocol is
//!   specced with the Cat-1 firmware, the real path ends in a clear "transport
//!   pending" dry run (commissioner copies wiped, non-zero exit);
//!   `--loopback-test` runs the whole ceremony against `LoopbackRelay` so the
//!   verb is exercisable now.
//! - No QR crate exists in this binary's frozen dep set: the QR is shown as
//!   its raw hex payload plus a note (a text-QR renderer is out of scope).

use std::io::Write;

use id111::commission::{
    CommissionAbort, CommissionCeremony, CommissionError, CommissionRecord, LoopbackRelay,
    RelayChannel,
};
use id111::encoding::NodeIdDisplay;
use zeroize::Zeroizing;

/// Failed type-back verifications tolerated before the ceremony aborts. Bounds
/// the loop (a dead tty makes `read_hidden` return empty strings forever) and
/// stops an operator from fumbling through a backup they have not actually
/// written down correctly — they restart with a fresh identity instead.
pub const MAX_VERIFY_ATTEMPTS: u32 = 3;

/// Where step 4 (provision) goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// No real channel exists yet (§15): show the bundle's non-secret summary,
    /// state that the transport is pending, wipe, and report a dry run.
    Pending,
    /// `--loopback-test`: run step 4 against the in-process `LoopbackRelay`.
    Loopback,
}

/// How a driven ceremony ended.
#[derive(Debug)]
pub enum Outcome {
    /// Challenge-verified and wiped (loopback only until §15 lands).
    Commissioned(CommissionRecord),
    /// Dry run: nothing was provisioned; the commissioner's copies are wiped.
    Pending(CommissionAbort),
}

/// The ceremony's four side-effect channels, injected so the whole driver is
/// testable without a terminal.
pub struct Io<'a> {
    /// Ceremony display (stdout — a TTY by the time this runs).
    pub out: &'a mut dyn Write,
    /// Hidden single-line input (typed-back words — the master secret).
    pub read_hidden: &'a mut dyn FnMut(&str) -> anyhow::Result<String>,
    /// Visible single-line input (pause prompts).
    pub read_visible: &'a mut dyn FnMut(&str) -> anyhow::Result<String>,
    /// Event log (stderr). Public values only — never material.
    pub log: &'a mut dyn FnMut(&str),
}

/// §8.3/§12 rule 10 gate (pure, so the refusal is testable): the ceremony
/// prints secrets to stdout and reads secrets from stdin — refuse unless both
/// are terminals.
pub fn require_tty(stdin_is_tty: bool, stdout_is_tty: bool) -> anyhow::Result<()> {
    anyhow::ensure!(
        stdin_is_tty && stdout_is_tty,
        "identity commission requires an interactive terminal on stdin and stdout \
         (the words would land in logs/shell history, and the type-back is never read from a pipe)"
    );
    Ok(())
}

/// [`require_tty`] over the process's real stdin/stdout.
pub fn require_stdio_tty() -> anyhow::Result<()> {
    let stdin_tty = unsafe { libc::isatty(0) } == 1;
    let stdout_tty = unsafe { libc::isatty(1) } == 1;
    require_tty(stdin_tty, stdout_tty)
}

/// Lowercase hex (the QR payload is 20/36 bytes — `id111::encoding::hex_encode`
/// only takes 32-byte keys).
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn write_node_id(out: &mut dyn Write, d: &NodeIdDisplay) -> std::io::Result<()> {
    writeln!(out, "  label   {}", d.label())?;
    writeln!(out, "  node1…  {}", d.bech32())?;
    writeln!(out, "  hex     {}", d.hex())
}

/// Begin a ceremony (entropy-verified keygen, §8.3 step 1) and drive it.
pub fn run(transport: Transport, io: &mut Io<'_>) -> anyhow::Result<Outcome> {
    let mut ceremony = CommissionCeremony::begin()?;
    drive(&mut ceremony, transport, io)
}

/// Drive a begun ceremony to its end. On ANY failure the ceremony is aborted
/// (commissioner copies wiped) and the abort event is logged before the error
/// propagates — no failure path leaves key material alive.
pub fn drive(
    ceremony: &mut CommissionCeremony,
    transport: Transport,
    io: &mut Io<'_>,
) -> anyhow::Result<Outcome> {
    match drive_steps(ceremony, transport, io) {
        Ok(outcome) => Ok(outcome),
        Err(e) => {
            if ceremony.step() != id111::commission::CeremonyStep::Wiped {
                let abort = ceremony.abort();
                (io.log)(&abort.log_line());
            }
            Err(e)
        }
    }
}

fn drive_steps(
    ceremony: &mut CommissionCeremony,
    transport: Transport,
    io: &mut Io<'_>,
) -> anyhow::Result<Outcome> {
    writeln!(
        io.out,
        "111 identity commission — Cat-1 relay commissioning ceremony (L2 §8.3)"
    )?;
    match transport {
        Transport::Loopback => writeln!(
            io.out,
            "\n*** LOOPBACK TEST: the relay is an in-process test double. Nothing is provisioned anywhere; \
             this identity is throwaway — do NOT keep or reuse these words. ***"
        )?,
        Transport::Pending => writeln!(
            io.out,
            "\nNOTE: the relay transport (USB-UART / BLE) is not built yet (L2 §15). This run is a DRY RUN: \
             it generates and verifies a backup but cannot provision a relay, and it ends by wiping this \
             computer's copy. Use --loopback-test to exercise the full flow."
        )?,
    }
    writeln!(
        io.out,
        "\nStep 1/4  entropy-verified keygen: done (getrandom, weak-key self-test passed)."
    )?;

    // ---- step 2: display (secret: words + QR payload; they exist to be written down) ----
    {
        let artifacts = ceremony.artifacts()?;
        let words = artifacts.words();
        writeln!(
            io.out,
            "\nStep 2/4  write these {} words down on paper and keep them secret:\n",
            words.split_whitespace().count()
        )?;
        for (i, w) in words.split_whitespace().enumerate() {
            writeln!(io.out, "  {:2}. {w}", i + 1)?;
        }
        writeln!(
            io.out,
            "\nBackup descriptor (write it under the words): {}",
            artifacts.descriptor
        )?;
        writeln!(
            io.out,
            "Project QR payload ({} bytes = 4B descriptor ‖ raw entropy), hex: {}",
            artifacts.qr_payload().len(),
            hex(artifacts.qr_payload())
        )?;
        writeln!(
            io.out,
            "  (this build has no QR renderer: feed the payload bytes to any QR encoder in byte mode, \
             QR version 2-3, error-correction level L. It is a project variant of CompactSeedQR — \
             raw-entropy-only tools will NOT parse it.)"
        )?;
    }
    writeln!(
        io.out,
        "\nNodeId of the relay being commissioned (record it):"
    )?;
    write_node_id(io.out, &ceremony.display())?;

    // ---- step 3: verify-on-write-down (Codex32-style sampled type-back) ----
    writeln!(
        io.out,
        "\nStep 3/4  verify-on-write-down: type back sampled words FROM YOUR PAPER (input hidden). \
         The relay is not touched until this passes."
    )?;
    (io.read_visible)("press Enter when the words are on paper: ")?;
    loop {
        let positions = ceremony.write_down_positions().to_vec();
        let mut answers: Vec<Zeroizing<String>> = Vec::with_capacity(positions.len());
        for &p in &positions {
            io.out.flush()?;
            answers.push(Zeroizing::new((io.read_hidden)(&format!(
                "  word #{} (input hidden): ",
                p + 1
            ))?));
        }
        let refs: Vec<&str> = answers.iter().map(|a| a.as_str()).collect();
        match ceremony.verify_write_down(&refs) {
            Ok(()) => {
                writeln!(io.out, "  verified: the backup is made.")?;
                break;
            }
            Err(CommissionError::WriteDownMismatch { attempts }) => {
                anyhow::ensure!(
                    attempts < MAX_VERIFY_ATTEMPTS,
                    "write-down verification failed {attempts} times — ceremony aborted, nothing provisioned. \
                     Destroy the paper and start over"
                );
                writeln!(
                    io.out,
                    "  not verified (attempt {attempts} of {MAX_VERIFY_ATTEMPTS}): those words do not match what \
                     was generated. Check your paper against the words above — different words are sampled \
                     each attempt."
                )?;
            }
            Err(e) => return Err(e.into()),
        }
    }

    // ---- step 4: provision + challenge-verify (or the pending dry run) ----
    match transport {
        Transport::Loopback => {
            let mut relay = LoopbackRelay::new();
            writeln!(
                io.out,
                "\nStep 4/4  provision over: {} — nothing leaves this process.",
                relay.channel().as_str()
            )?;
            let summary = ceremony.provision_via(&mut relay)?;
            writeln!(
                io.out,
                "  provisioning bundle (non-secret summary; the 64B keyset is never shown):"
            )?;
            write_bundle_summary(io.out, &summary)?;
            anyhow::ensure!(
                relay.committed_node_id() == Some(ceremony.node_id()),
                "loopback relay committed a different NodeId than the ceremony generated"
            );
            writeln!(
                io.out,
                "  challenge-verify: OK — the relay signed \"111/commission-challenge/v1\" ‖ nonce with the \
                 provisioned signing key (strict predicate, §3)."
            )?;
            let record = ceremony.finish()?;
            (io.log)(&record.log_line());
            writeln!(
                io.out,
                "  commissioner copies wiped (keyset, words, QR payload)."
            )?;
            writeln!(
                io.out,
                "\nCommissionRecord (the only thing this ceremony retains):"
            )?;
            write_node_id(io.out, &NodeIdDisplay::new(record.node_id.as_bytes()))?;
            writeln!(io.out, "  descriptor        {}", record.descriptor)?;
            writeln!(io.out, "  verified_at_step  {}", record.verified_at_step)?;
            Ok(Outcome::Commissioned(record))
        }
        Transport::Pending => {
            writeln!(
                io.out,
                "\nStep 4/4  provision: RELAY TRANSPORT PENDING (L2 §15)."
            )?;
            let summary = ceremony.provisioning_bundle()?.summary();
            writeln!(
                io.out,
                "  provisioning bundle (non-secret summary; the 64B keyset is NOT shown and was NOT sent):"
            )?;
            write_bundle_summary(io.out, &summary)?;
            writeln!(
                io.out,
                "  The USB-UART / BLE byte protocol is specified with the Cat-1 firmware (§15); until then \
                 there is no channel to send this bundle over."
            )?;
            let abort = ceremony.abort();
            (io.log)(&abort.log_line());
            writeln!(
                io.out,
                "\nDry run complete: NO relay was provisioned and this computer's copy is wiped. Destroy the \
                 paper from this run. Exercise the full flow now with: 111 identity commission --loopback-test"
            )?;
            Ok(Outcome::Pending(abort))
        }
    }
}

fn write_bundle_summary(
    out: &mut dyn Write,
    s: &id111::commission::BundleSummary,
) -> std::io::Result<()> {
    write_node_id(out, &NodeIdDisplay::new(s.node_id.as_bytes()))?;
    writeln!(out, "  descriptor        {}", s.descriptor)?;
    writeln!(out, "  challenge nonce   {}", hex(&s.challenge_nonce))
}

#[cfg(test)]
mod tests {
    use super::*;
    use id111::commission::CeremonyStep;
    use id111::keyset::Keyset;

    struct Harness {
        out: Vec<u8>,
        hidden_prompts: Vec<String>,
        visible_prompts: Vec<String>,
        log: Vec<String>,
    }

    /// Run `drive` with scripted I/O. `answer(prompt) -> reply` stands in for
    /// the operator reading their paper: it maps "word #N" to words[N-1] unless
    /// the test makes it lie.
    fn run_scripted(
        ceremony: &mut CommissionCeremony,
        transport: Transport,
        mut answer: impl FnMut(&str) -> String,
    ) -> (anyhow::Result<Outcome>, Harness) {
        let mut h = Harness {
            out: Vec::new(),
            hidden_prompts: Vec::new(),
            visible_prompts: Vec::new(),
            log: Vec::new(),
        };
        let (hp, vp, lg) = (
            std::cell::RefCell::new(&mut h.hidden_prompts),
            std::cell::RefCell::new(&mut h.visible_prompts),
            std::cell::RefCell::new(&mut h.log),
        );
        let mut read_hidden = |p: &str| -> anyhow::Result<String> {
            hp.borrow_mut().push(p.to_string());
            Ok(answer(p))
        };
        let mut read_visible = |p: &str| -> anyhow::Result<String> {
            vp.borrow_mut().push(p.to_string());
            Ok(String::new())
        };
        let mut log = |l: &str| lg.borrow_mut().push(l.to_string());
        let mut io = Io {
            out: &mut h.out,
            read_hidden: &mut read_hidden,
            read_visible: &mut read_visible,
            log: &mut log,
        };
        let r = drive(ceremony, transport, &mut io);
        (r, h)
    }

    fn honest_reader(words: &str) -> impl FnMut(&str) -> String + '_ {
        move |prompt: &str| {
            let n: usize = prompt
                .split('#')
                .nth(1)
                .and_then(|r| r.split_whitespace().next())
                .and_then(|n| n.parse().ok())
                .unwrap_or_else(|| panic!("not a word prompt: {prompt}"));
            words.split(' ').nth(n - 1).unwrap().to_string()
        }
    }

    fn text(h: &Harness) -> String {
        String::from_utf8(h.out.clone()).unwrap()
    }

    fn begun() -> (CommissionCeremony, String) {
        let c = CommissionCeremony::begin().unwrap();
        let words = c.artifacts().unwrap().words().to_string();
        (c, words)
    }

    #[test]
    fn tty_gate_refuses_pipes_on_either_end() {
        assert!(require_tty(true, true).is_ok());
        for (i, o) in [(false, false), (true, false), (false, true)] {
            let err = require_tty(i, o).unwrap_err().to_string();
            assert!(err.contains("interactive terminal"), "{err}");
        }
    }

    #[test]
    fn real_stdio_gate_refuses_when_not_a_terminal() {
        if unsafe { libc::isatty(0) } == 1 && unsafe { libc::isatty(1) } == 1 {
            return; // `--nocapture` on a real terminal: the refusal is not exercisable here
        }
        assert!(require_stdio_tty().is_err());
    }

    #[test]
    fn hex_is_lowercase_and_fixed_width() {
        assert_eq!(hex(&[0x00, 0x0f, 0xab, 0xff]), "000fabff");
        assert_eq!(hex(&[]), "");
    }

    #[test]
    fn loopback_ceremony_runs_end_to_end_and_never_prints_the_keyset() {
        let (mut c, words) = begun();
        let keyset_hex = hex(&Keyset::derive_from_mnemonic(&words).unwrap().to_bytes()[..]);
        let node = c.node_id();
        let (r, h) = run_scripted(&mut c, Transport::Loopback, honest_reader(&words));
        let record = match r.unwrap() {
            Outcome::Commissioned(rec) => rec,
            other => panic!("{other:?}"),
        };
        assert_eq!(record.node_id, node);
        assert_eq!(record.verified_at_step, CeremonyStep::ChallengeVerified);
        assert_eq!(c.step(), CeremonyStep::Wiped);

        let out = text(&h);
        // Display: every word, descriptor, QR hex, NodeId in all three forms.
        for (i, w) in words.split(' ').enumerate() {
            assert!(
                out.contains(&format!("{:2}. {w}", i + 1)),
                "word {} missing",
                i + 1
            );
        }
        let d = NodeIdDisplay::new(node.as_bytes());
        assert!(out.contains("01010101"));
        assert!(out.contains("Project QR payload (20 bytes"));
        assert!(out.contains(&d.hex()) && out.contains(&d.bech32()) && out.contains(&d.label()));
        assert!(out.contains("LOOPBACK TEST"));
        assert!(out.contains("challenge-verify: OK"));
        assert!(out.contains("CommissionRecord"));
        assert!(out.contains("verified_at_step  challenge-verified"));
        assert!(out.contains("challenge nonce"));
        // The private keyset (and its signing-seed half) NEVER reaches output or log.
        let everything = format!("{out}\n{}\n{:?}", h.log.join("\n"), h.hidden_prompts);
        assert!(!everything.contains(&keyset_hex), "64B keyset leaked");
        assert!(
            !everything.contains(&keyset_hex[..64]),
            "signing seed leaked"
        );
        assert!(
            !everything.contains(&keyset_hex[64..]),
            "ECDH scalar leaked"
        );
        // Interaction shape: one pause, 4 hidden word prompts, one event line.
        assert_eq!(h.visible_prompts.len(), 1);
        assert_eq!(h.hidden_prompts.len(), 4);
        assert!(h.hidden_prompts.iter().all(|p| p.contains("input hidden")));
        assert_eq!(h.log.len(), 1);
        assert!(h.log[0].starts_with("commissioned node="), "{}", h.log[0]);
        assert!(!h.log[0].contains(&words));
    }

    #[test]
    fn pending_transport_is_a_dry_run_that_wipes_and_says_so() {
        let (mut c, words) = begun();
        let keyset_hex = hex(&Keyset::derive_from_mnemonic(&words).unwrap().to_bytes()[..]);
        let (r, h) = run_scripted(&mut c, Transport::Pending, honest_reader(&words));
        let abort = match r.unwrap() {
            Outcome::Pending(a) => a,
            other => panic!("{other:?}"),
        };
        assert_eq!(abort.reached, CeremonyStep::BundleIssued);
        assert_eq!(c.step(), CeremonyStep::Wiped);
        let out = text(&h);
        assert!(out.contains("RELAY TRANSPORT PENDING (L2 §15)"));
        assert!(out.contains("NO relay was provisioned"));
        assert!(out.contains("challenge nonce"));
        assert!(!out.contains("challenge-verify: OK"));
        assert!(!out.contains("CommissionRecord"));
        assert!(!out.contains(&keyset_hex), "keyset leaked");
        assert_eq!(h.log.len(), 1);
        assert!(h.log[0].starts_with("aborted node="), "{}", h.log[0]);
        assert!(h.log[0].contains("reached=bundle-issued"));
    }

    #[test]
    fn one_wrong_attempt_then_a_right_one_passes_with_resampled_words() {
        let (mut c, words) = begun();
        let mut honest = honest_reader(&words);
        let mut calls = 0;
        let (r, h) = run_scripted(&mut c, Transport::Loopback, |p| {
            calls += 1;
            if calls <= 4 {
                "not-a-bip39-word".to_string() // first attempt: all wrong
            } else {
                honest(p)
            }
        });
        assert!(matches!(r.unwrap(), Outcome::Commissioned(_)));
        let out = text(&h);
        assert!(out.contains("not verified (attempt 1 of 3)"), "{out}");
        assert!(out.contains("verified: the backup is made."));
        assert_eq!(h.hidden_prompts.len(), 8);
    }

    #[test]
    fn repeated_failure_aborts_wipes_and_logs_and_never_provisions() {
        let (mut c, _words) = begun();
        let (r, h) = run_scripted(&mut c, Transport::Loopback, |_| String::new());
        let err = r.unwrap_err().to_string();
        assert!(
            err.contains("failed 3 times") && err.contains("nothing provisioned"),
            "{err}"
        );
        assert_eq!(c.step(), CeremonyStep::Wiped, "failure path must wipe");
        assert!(matches!(
            c.provisioning_bundle(),
            Err(CommissionError::Wiped)
        ));
        assert_eq!(
            h.hidden_prompts.len(),
            12,
            "3 attempts x 4 words, then stop (no endless loop)"
        );
        let out = text(&h);
        assert!(
            !out.contains("Step 4/4"),
            "relay stage must never be reached"
        );
        assert_eq!(h.log.len(), 1);
        assert!(h.log[0].contains("reached=keygen"), "{}", h.log[0]);
    }

    #[test]
    fn io_failure_mid_ceremony_still_wipes() {
        let (mut c, _words) = begun();
        let mut out: Vec<u8> = Vec::new();
        let mut read_hidden =
            |_: &str| -> anyhow::Result<String> { anyhow::bail!("tty went away") };
        let mut read_visible = |_: &str| -> anyhow::Result<String> { Ok(String::new()) };
        let mut logged = Vec::new();
        let mut log = |l: &str| logged.push(l.to_string());
        let mut io = Io {
            out: &mut out,
            read_hidden: &mut read_hidden,
            read_visible: &mut read_visible,
            log: &mut log,
        };
        let err = drive(&mut c, Transport::Loopback, &mut io).unwrap_err();
        assert!(err.to_string().contains("tty went away"));
        assert_eq!(c.step(), CeremonyStep::Wiped);
        assert_eq!(logged.len(), 1);
        assert!(logged[0].starts_with("aborted node="));
    }
}
