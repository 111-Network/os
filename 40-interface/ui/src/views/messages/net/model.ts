// Foreign networks in Messages (os/60-transport/README.md §3, §4, §9, §10): the words for a foreign row's evidence and reason, the
// auth label, the banner, the dropped-text counter and the fan-out preview. Pure: no React, no clock of its own.
//
// Every string that came from the air (a title, a sender name, a native id, a body) is untrusted. It is cleaned again here
// (`cleanPeerText`) and drawn as text only: no helper in this file ever builds a link, markup or a prompt from one.
import { cleanPeerText } from "../../../lib/api/messages";
import { lookup, type ForeignPage } from "../../../lib/api/networks";
import type { FanoutTarget, ForeignAuth, ForeignConversation, ForeignMessage, Network, SendPlan } from "../../../lib/api/types";

/** `s` cut to at most `max` characters (code points, so an emoji is never split) with an ellipsis. */
export function clip(s: string, max: number): string {
  const chars = Array.from(s);
  return chars.length <= max ? s : `${chars.slice(0, Math.max(0, max - 1)).join("").trimEnd()}…`;
}

/** Text for display: control and direction-override characters gone, whitespace flattened, cut. */
export const plain = (s: string, max: number): string => clip(cleanPeerText(s).replace(/\s+/g, " ").trim(), max);

/** A sender's name is cut to this many characters on screen (the daemon already cuts to 64 bytes). */
export const MAX_NAME_CHARS = 64;

// --- Evidence -----------------------------------------------------------------------------------------------------------

/** What each foreign state means in general words. The adapter's own `evidence` string says it for the network at hand. */
export const FOREIGN_STATE_COPY: Record<string, string> = {
  queued: "Waiting on this node: the radio, a path or a key is not ready yet.",
  sent: "Handed to the carrier. Nothing has confirmed it yet.",
  relayed: "A neighbour or a relay took it on. The recipient has not confirmed it.",
  delivered: "Confirmed by the other side.",
  failed: "Refused before it left. Nothing was transmitted.",
};

export const foreignStateCopy = (state: string): string => lookup(FOREIGN_STATE_COPY, state) ?? "This node reports a state this page does not know.";

/** Why a row waits or was refused, by `reason_code` (§4, §5, §8). The after-air codes keep `sent` or `relayed`: "transmitted, not confirmed". */
export const FOREIGN_REASON_COPY: Record<string, string> = {
  region_unset: "The radio region is not set, so the board transmits nothing. Set it in Settings, Transport.",
  no_channel: "The board has no such channel.",
  too_large: "The text is larger than one packet carries.",
  not_authorized: "The board refused this send.",
  pki_failed: "The board could not encrypt this direct message.",
  got_nak: "The board answered with a refusal.",
  adapter_down: "This network is not reachable right now. The text waits on this node.",
  no_route: "Transmitted, not confirmed: no route was found.",
  timeout: "Transmitted, not confirmed: no answer came back in time.",
  max_retransmit: "Transmitted, not confirmed: it was retried and still did not get through.",
  no_response: "Transmitted, not confirmed: the other side did not answer.",
  duty_cycle: "The region's duty-cycle cap is reached. It goes out later.",
  rate_limited: "The radio asked for a pause between texts. It goes out later.",
  key_unknown: "The board does not have their key yet. It goes out once it hears their node.",
  peer_unannounced: "No announce from them yet, so there is no path. It goes out when they announce.",
  uplink_off: "Sending on this channel is off: it is published to the internet by your board.",
  ham_unconfirmed: "Your board is amateur-licensed and readable sends are not confirmed yet.",
  consent_paused: "Something you confirmed has changed, so the relay paused.",
  cancelled: "You cancelled this before it left.",
  oversize: "Too long for one packet.",
  handoff_pending: "It may have been sent before this node stopped. Resend it only if you want it sent again.",
};

/** Reason text: this UI's copy for a code it knows, else the node's own sentence, else "unknown" (the U1b rule). null when nothing needs explaining. */
export function foreignReasonText(m: Pick<ForeignMessage, "reason_code" | "reason">): string | null {
  const code = m.reason_code;
  if (!code) return null;
  const known = lookup(FOREIGN_REASON_COPY, code);
  if (known) return known;
  const given = m.reason ? cleanPeerText(m.reason).trim() : "";
  return given !== "" ? given : "unknown";
}

export const HANDOFF_QUESTION = "May have been sent; resend?";

/** The evidence the adapter printed for a row, cleaned; the general state copy stands in when it printed none. */
export const evidenceText = (m: { state: string; evidence: string | null; dir: string }): string => {
  if (m.dir === "in") return "arrived on this node";
  const given = m.evidence ? cleanPeerText(m.evidence).trim() : "";
  return given !== "" ? given : foreignStateCopy(m.state);
};

// --- Names, ids, auth ------------------------------------------------------------------------------------------------------

/** The sender's own name for display: cleaned, cut, never markup. An empty name reads "no name". */
export const senderName = (m: Pick<ForeignMessage, "sender">): string => plain(m.sender.name, MAX_NAME_CHARS) || "no name";

/** The native id (`!a1b2c3d4`, a hash) as text, cut for a small screen. */
export const nativeIdText = (id: string, max = 40): string => plain(id, max);

/** What stands in for a level this table does not know. */
export const AUTH_FALLBACK: Record<ForeignAuth, string> = {
  none: "nothing vouches for this name",
  "channel-key": "anyone with this channel key can use any name",
  pki: "the board's PKI proved the sender key, not the name",
  signed: "the carrier validated a signature; the name is still self-declared",
  "attested-111": "vouched for by another 111 node",
};

/** The label copy for an `auth` level: the network's own (from its manifest), else the general one, else "unknown". */
export function authText(network: Pick<Network, "copy"> | null | undefined, auth: string): string {
  const own = network ? lookup(network.copy.auth, auth) : undefined;
  if (own) return cleanPeerText(own);
  return lookup(AUTH_FALLBACK as Record<string, string>, auth) ?? "unknown";
}

export function truncatedText(n: number): string {
  return `message too long to show (${n} bytes)`;
}

/** "The sender says they sent this at …": their own clock, which cannot be checked. Behind a disclosure, never beside the received time. */
export function claimLine(claim: string | null, fmt: (iso: string) => string): string | null {
  return claim ? `The sender says they sent this at ${fmt(claim)}. That is their own clock and cannot be checked; rows are ordered by when they arrived here.` : null;
}

/** "relayed copy": a row an edge made. The marker `[name/TAG]` is in the text itself and stays visible. */
export const RELAYED_COPY_NOTE = "A relayed copy: this node copied it from another conversation. The [name/TAG] prefix names the original sender.";

export const VIA_INTERNET_TEXT = "via internet";

// --- Network banner ----------------------------------------------------------------------------------------------------------

export function droppedText(n: number): string | null {
  return n > 0 ? (n === 1 ? "1 text dropped while 111 was busy" : `${n} texts dropped while 111 was busy`) : null;
}

/** One line for a network that is not live. */
export function networkStateLine(n: Pick<Network, "state" | "reason" | "label">): string | null {
  const given = n.reason ? cleanPeerText(n.reason).trim() : "";
  switch (n.state) {
    case "live":
      return null;
    case "not_set":
      return given || `${n.label} needs something from you before it can send.`;
    case "error":
      return given || `${n.label} is not working. Texts wait on this node.`;
    case "disabled":
      return given || `${n.label} is turned off.`;
    default:
      return "This node reports a state for this network that this page does not know.";
  }
}

export const NETWORK_STATE_WORD: Record<string, string> = { live: "live", not_set: "not set", error: "error", disabled: "off" };
export const networkStateWord = (state: string): string => lookup(NETWORK_STATE_WORD, state) ?? "unknown";

// --- Conversations ----------------------------------------------------------------------------------------------------------

/** A conversation's title as text; a title that cleans to nothing falls back to its kind. */
export const convTitle = (c: Pick<ForeignConversation, "title" | "kind">): string => plain(c.title, 60) || (c.kind === "dm" ? "direct message" : c.kind === "channel" ? "channel" : "group");

export const KIND_WORD: Record<string, string> = { channel: "channel", dm: "DM", group: "recipient list" };
export const kindWord = (kind: string): string => lookup(KIND_WORD, kind) ?? "unknown";

/** "2 channels · 1 DM" for a network's rail entry, from its conversations. */
export function splitSummary(convs: readonly Pick<ForeignConversation, "kind">[]): string {
  const n = (k: string) => convs.filter((c) => c.kind === k).length;
  const parts: string[] = [];
  if (n("channel") > 0) parts.push(`${n("channel")} ${n("channel") === 1 ? "channel" : "channels"}`);
  if (n("dm") > 0) parts.push(`${n("dm")} ${n("dm") === 1 ? "DM" : "DMs"}`);
  if (n("group") > 0) parts.push(`${n("group")} ${n("group") === 1 ? "recipient list" : "recipient lists"}`);
  return parts.length > 0 ? parts.join(" · ") : "no conversations yet";
}

/** The last line of a held page for a list, as text. */
export function pageLastLine(page: ForeignPage | null | undefined): string {
  const last = page?.messages[page.messages.length - 1];
  return last ? plain(last.text, 80) : "no messages yet";
}

// --- Send plan (§9) ----------------------------------------------------------------------------------------------------------

/** "212 B of 232 B per packet", plus the parts when it is cut. */
export function planSizeText(p: Pick<SendPlan, "bytes" | "cap" | "parts">): string {
  const base = p.cap === null ? `${p.bytes} B` : `${p.bytes} B of ${p.cap} B per packet`;
  return p.parts > 1 ? `${base}, ${p.parts} parts` : base;
}

/** "about 2.1 s (estimate)": the adapter labels its own estimate, and this page adds the word when the label forgot it. */
export function airtimeText(p: Pick<SendPlan, "airtime_estimate">): string | null {
  const a = p.airtime_estimate;
  if (!a) return null;
  const label = cleanPeerText(a.label).trim();
  return /estimate|depends/i.test(label) ? label : `${label} (estimate)`;
}

/** A key for a target: its conversation, or the contact's node id. */
export const targetKey = (t: FanoutTarget): string => ("node_id" in t ? `c:${t.node_id}` : `f:${t.conv}`);

/** The one-line description of what a size mode does, for the picker. */
export const SIZE_MODE_HELP: Record<"refuse" | "split", string> = {
  refuse: "Refuse: nothing is sent to this target if the text is too long for one packet.",
  split: "Split: cut at word boundaries into numbered parts, (1/3) first. Parts can arrive out of order.",
};

export const PREVIEW_ESTIMATE_NOTE = "Airtime is an estimate from the radio's settings, not a promise.";

/** A byte count as a person reads it. */
export const bytesText = (n: number): string => `${n} B`;

/** The most one text may carry in this conversation, in bytes, or null when the carrier states none. A licensed board raises a DM's cap. */
export function capFor(network: Pick<Network, "caps" | "gate">, conv: Pick<ForeignConversation, "kind">): number | null {
  const per = network.caps.max_text_bytes[conv.kind];
  if (!per) return null;
  const licensed = conv.kind === "dm" && network.gate.licensed === true ? per.licensed : undefined;
  return licensed ?? per.default ?? null;
}

const encoder = new TextEncoder();
export const utf8Length = (s: string): number => encoder.encode(s).length;
