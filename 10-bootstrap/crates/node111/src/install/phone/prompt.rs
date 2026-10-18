//! Operator confirmations for the phone flow. They read the controlling
//! terminal (`/dev/tty`), never stdin: under `curl … | sh -s -- --phone` stdin
//! is the exhausted script pipe, and an EOF there must never count as "yes".
//! No terminal, or end of input, fails CLOSED. `--yes` is the explicit
//! non-interactive opt-in.

use std::io::BufRead;

/// The controlling terminal, opened for reading.
pub fn open_tty() -> std::io::Result<std::io::BufReader<std::fs::File>> {
    std::fs::File::open("/dev/tty").map(std::io::BufReader::new)
}

/// One operator answer. `Ok(None)` = `--yes` was given (nothing is read).
/// Errors when there is no terminal to ask or it reached end of input —
/// the caller must stop, never proceed.
pub fn answer<R: BufRead>(
    tty: std::io::Result<R>,
    yes: bool,
    what: &str,
) -> anyhow::Result<Option<String>> {
    if yes {
        return Ok(None);
    }
    let mut tty = tty.map_err(|e| {
        anyhow::anyhow!(
            "{what} needs your confirmation, but there is no terminal to ask ({e}) — \
             run the installer from a terminal, or pass --yes to confirm in advance"
        )
    })?;
    let mut line = String::new();
    let n = tty.read_line(&mut line)?;
    anyhow::ensure!(
        n > 0,
        "{what}: no answer (end of input) — nothing confirmed, stopping. \
         Run the installer from a terminal, or pass --yes to confirm in advance"
    );
    Ok(Some(line))
}

/// `[y/N]` question: true only on `y`/`yes` (or `--yes`).
pub fn confirm<R: BufRead>(tty: std::io::Result<R>, yes: bool, what: &str) -> anyhow::Result<bool> {
    Ok(match answer(tty, yes, what)? {
        None => true,
        Some(l) => {
            let l = l.trim();
            l.eq_ignore_ascii_case("y") || l.eq_ignore_ascii_case("yes")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn no_tty() -> std::io::Result<Cursor<Vec<u8>>> {
        Err(std::io::Error::from_raw_os_error(libc::ENXIO))
    }

    fn tty(s: &str) -> std::io::Result<Cursor<Vec<u8>>> {
        Ok(Cursor::new(s.as_bytes().to_vec()))
    }

    #[test]
    fn no_terminal_fails_closed() {
        let e = answer(no_tty(), false, "the lockdown").unwrap_err();
        assert!(e.to_string().contains("no terminal"), "{e}");
        assert!(e.to_string().contains("--yes"), "{e}");
        assert!(confirm(no_tty(), false, "x").is_err());
    }

    #[test]
    fn end_of_input_is_not_a_yes() {
        // the `curl | sh` case: an exhausted pipe reads EOF
        let e = answer(tty(""), false, "the fresh-LAN sshd check").unwrap_err();
        assert!(e.to_string().contains("end of input"), "{e}");
        assert!(confirm(tty(""), false, "x").is_err());
    }

    #[test]
    fn answers_and_the_yes_flag() {
        assert_eq!(
            answer(tty("\n"), false, "x").unwrap().as_deref(),
            Some("\n")
        );
        assert!(confirm(tty("y\n"), false, "x").unwrap());
        assert!(confirm(tty("YES\n"), false, "x").unwrap());
        assert!(!confirm(tty("\n"), false, "x").unwrap());
        assert!(!confirm(tty("n\n"), false, "x").unwrap());
        // --yes reads nothing, even with no terminal
        assert_eq!(answer(no_tty(), true, "x").unwrap(), None);
        assert!(confirm(no_tty(), true, "x").unwrap());
    }
}
