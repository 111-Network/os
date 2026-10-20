// Bridge slice (os/6-transport/README.md §2.1, §6, §8, §9): the consent round-trip, relay edges, recipient lists and optional
// components. Pure helpers: the store keeps the lists, the views draw them.
//
// A consent is a 409 `consent_required` that carries the exact text and its SHA-256: the client shows the text verbatim and repeats
// the call with the hash. The text may quote untrusted channel names, so it is drawn as text only.
import type { Adapter } from "./adapter";
import { isApiError } from "./problem";
import type { BridgeComponent, BridgeEdge, ConsentRequired, ForeignConversation, NetworkCaps, TargetRefusal } from "./types";

const HASH = /^[0-9a-f]{64}$/;

const CONSENT_ACTIONS: readonly string[] = ["edge_enable", "edge_resume", "component_enable", "ham_confirm", "uplink_send"];

/** A well-formed `ConsentRequired`, or null. A malformed one is never offered for confirmation (fail closed). */
export function readConsent(x: unknown): ConsentRequired | null {
  if (typeof x !== "object" || x === null) return null;
  const c = x as Partial<ConsentRequired>;
  if (typeof c.warning_text !== "string" || c.warning_text === "" || typeof c.hash !== "string" || !HASH.test(c.hash)) return null;
  if (typeof c.action !== "string" || !CONSENT_ACTIONS.includes(c.action) || typeof c.subject !== "string") return null;
  const addons = Array.isArray(c.addons) ? c.addons.filter((a): a is string => typeof a === "string") : [];
  return { action: c.action as ConsentRequired["action"], subject: c.subject, warning_text: c.warning_text, hash: c.hash, addons };
}

/** The consent a failed call carries: 409 `consent_required` with the `consent` extension member. */
export function consentOf(err: unknown): ConsentRequired | null {
  if (!isApiError(err) || err.code !== "consent_required") return null;
  return readConsent(err.problem.consent);
}

/** The per-target report of a refused fan-out (409 `oversize` or `consent_required` with `targets`). Entries that do not read are dropped. */
export function refusalsOf(err: unknown): TargetRefusal[] {
  if (!isApiError(err)) return [];
  const list = err.problem.targets;
  if (!Array.isArray(list)) return [];
  return list.filter((t): t is TargetRefusal => typeof t === "object" && t !== null && typeof (t as TargetRefusal).code === "string" && typeof (t as TargetRefusal).detail === "string");
}

/**
 * Confirm a gate the node named in a 409: a HAM confirmation or an uplinked channel's send switch is `PATCH /bridge/settings` with the
 * hash of the text the person read. (Edge and component confirmations ride on their own calls.) A changed text answers 409 again.
 */
export async function confirmGate(adapter: Pick<Adapter, "bridge">, consent: ConsentRequired, hash: string): Promise<void> {
  if (consent.action === "ham_confirm") await adapter.bridge.patchSettings({ ham: [{ account: consent.subject, confirmed: true, accept_warning: hash }] });
  else if (consent.action === "uplink_send") await adapter.bridge.patchSettings({ channels: [{ conv: consent.subject, send: true, accept_warning: hash }] });
  else throw new Error(`not a settings gate: ${consent.action}`);
}

// --- Edges -------------------------------------------------------------------------------------------------------------

/** The copy under an edge's state. A consent-paused edge says why and what resumes it. */
export function edgeStateText(e: Pick<BridgeEdge, "enabled" | "paused" | "pause_reason">): string {
  if (!e.enabled) return "off: an end of this relay is on a network that is not enabled, so nothing is copied";
  if (!e.paused) return "relaying";
  if (e.pause_reason === "consent_paused") return "paused: something you confirmed has changed (uplink, licence or the channel key). Resuming asks you again.";
  return "paused by you";
}

export const edgeIsConsentPaused = (e: Pick<BridgeEdge, "paused" | "pause_reason">): boolean => e.paused && e.pause_reason === "consent_paused";

/** "3 relayed · 1 shed · 2 too long to relay" (refused counts only the copies not made, so it reads as the node's own word). */
export function edgeCountersText(e: Pick<BridgeEdge, "counters">): string {
  const c = e.counters;
  return `${c.relayed} relayed · ${c.shed} shed · ${c.refused} too long to relay`;
}

// --- Recipient lists ----------------------------------------------------------------------------------------------------

/** Lowercase, no spaces, no `<` `>` that a paste of `<abcd…>` brings. */
export const normalizeAddress = (s: string): string => s.replace(/[\s<>]/g, "").toLowerCase();

/** Why this is not an address the network accepts, or null. `syntax` is `caps.recipient_syntax`; a network without one takes none. */
export function addressProblem(syntax: NetworkCaps["recipient_syntax"], raw: string): string | null {
  const s = normalizeAddress(raw);
  if (s === "") return "Type or paste an address first.";
  if (!syntax) return "This network does not take typed addresses.";
  let re: RegExp;
  try {
    re = new RegExp(syntax.pattern);
  } catch {
    return "This page cannot check the address, so it is not added.";
  }
  return re.test(s) ? null : `Not a ${syntax.label}. Example: ${syntax.example}`;
}

/** Split pasted text into addresses (whitespace, commas and semicolons separate them); duplicates are dropped, order is kept. */
export function parseAddresses(text: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const part of text.split(/[\s,;]+/)) {
    const a = normalizeAddress(part);
    if (a !== "" && !seen.has(a)) {
      seen.add(a);
      out.push(a);
    }
  }
  return out;
}

/** The members of a recipient list as a DM-less short label: `3 people`. */
export const membersText = (n: number): string => (n === 1 ? "1 address" : `${n} addresses`);

// --- Conversations as edge endpoints -----------------------------------------------------------------------------------

/** Both ends of an edge are channels or groups (a DM is unrepresentable, §8). */
export const edgeEndpoint = (c: Pick<ForeignConversation, "kind">): boolean => c.kind === "channel" || c.kind === "group";

/** A recipient list can be a destination only (§8): there is no Reticulum source conversation to read. */
export const edgeDestinationOnly = (c: Pick<ForeignConversation, "kind">): boolean => c.kind === "group";

// --- Components ---------------------------------------------------------------------------------------------------------

/** "9.0 MB", "412 kB", "3 B". Decimal units, like the sizes the manifest records. null is "unknown". */
export function formatBytes(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n) || n < 0) return "unknown";
  if (n < 1000) return `${Math.round(n)} B`;
  if (n < 1_000_000) return `${Math.round(n / 1000)} kB`;
  if (n < 1_000_000_000) return `${(n / 1_000_000).toFixed(1)} MB`;
  return `${(n / 1_000_000_000).toFixed(1)} GB`;
}

/** What a component weighs, in the words of §2.1: nothing runs while it is off; the sizes are what enabling adds. */
export function componentWeightFacts(c: Pick<BridgeComponent, "state" | "cached_bytes" | "installed_bytes" | "rss_bytes">): string[] {
  const facts: string[] = [];
  const off = c.state === "declared" || c.state === "cached" || c.state === "disabled" || c.state === "installed";
  if (c.state === "running") facts.push(c.rss_bytes != null ? `running, about ${formatBytes(c.rss_bytes)} of memory` : "running");
  else if (off) facts.push("off: no process, listener, timer or open file");
  if (c.cached_bytes != null) facts.push(c.state === "cached" || c.state === "declared" ? `${formatBytes(c.cached_bytes)} cached, not installed` : `${formatBytes(c.cached_bytes)} cached download`);
  else if (c.state === "declared") facts.push("no download cached: it has to be placed in the cache before it can be enabled");
  if (c.installed_bytes != null) facts.push(`${formatBytes(c.installed_bytes)} installed`);
  else if (c.state !== "running") facts.push("nothing installed");
  return facts;
}

export const componentIsOn = (c: Pick<BridgeComponent, "state">): boolean => c.state === "running";
/** Files exist that "remove files" would delete. */
export const componentHasFiles = (c: Pick<BridgeComponent, "state" | "installed_bytes" | "cached_bytes">): boolean =>
  c.installed_bytes != null || c.cached_bytes != null || c.state === "installed" || c.state === "disabled";
