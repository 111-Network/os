// Who has to enter a pair code? Pure, so the routing rule is tested without a browser.
//
// Loopback trust is a node policy (`GET /auth/session` -> `loopback_trusted`): on a laptop a loopback
// peer is authenticated without pairing, but under Termux/Android (or `DASHBOARD_TRUST_LOOPBACK=false`) it
// is not, and the phone's own browser pairs once like any LAN visitor. An older daemon omits the field,
// which means the old behaviour: trusted.

export interface SessionFacts {
  loopback: boolean;
  paired: boolean;
  /** Policy: loopback peers need no pairing. Absent on older daemons (treated as true). */
  loopback_trusted?: boolean | undefined;
}

/** True when this browser has no way in without a pair code, so the shell must show /pair. */
export function needsPairCode(s: SessionFacts): boolean {
  if (s.paired) return false;
  if (s.loopback && s.loopback_trusted !== false) return false;
  return true;
}

/** A loopback browser that the node's policy does not trust (Termux): the pair screen says so. */
export function isUntrustedLoopback(s: SessionFacts): boolean {
  return s.loopback && !s.paired && s.loopback_trusted === false;
}

/** The sentence at the top of the pair screen. */
export function pairIntro(untrustedLoopback: boolean): { text: string; command: string; tail: string } {
  return untrustedLoopback
    ? { text: "This phone's browser pairs once. The code is shown by ", command: "111 dashboard", tail: " (the installer opened this link with it)." }
    : { text: "On the node run ", command: "111 dashboard", tail: " and type the pair code shown there." };
}
