// Contacts view model (os/messaging/README.md sections 7 and 16): what a row says about a contact, and the sentence for
// each way "add a contact" can end. Pure: no React.
import { classifyAddInput, contactName, roleLabels, trustLabel, type AddInput } from "../../lib/api/contacts";
import type { Contact } from "../../lib/api/types";
import { errorSentence } from "../messages/model";

export { contactName };

/** Trusted-local name first: alias, else the operator label. */
export const rowName = (c: Pick<Contact, "alias" | "label">): string => contactName(c);

/** The short facts under a name, in order: how the keys were learned, key state, roles, pin. */
export function rowFacts(c: Pick<Contact, "trust" | "has_key" | "roles" | "pinned">): string[] {
  const facts = [trustLabel(c.trust)];
  if (!c.has_key) facts.push("waiting for their card");
  facts.push(...roleLabels(c.roles));
  if (c.pinned) facts.push("pinned");
  return facts;
}

/** Aliases are local names: trimmed, empty clears (null), at most 64 characters. */
export const ALIAS_MAX = 64;
export function aliasFromDraft(draft: string): string | null {
  const t = draft.trim().slice(0, ALIAS_MAX);
  return t === "" ? null : t;
}

/** Contacts proper, sorted: pinned first, then by name. Requests are shown elsewhere. */
export function sortedContacts(contacts: readonly Contact[]): Contact[] {
  return contacts
    .filter((c) => !c.request)
    .sort((a, b) => Number(b.pinned) - Number(a.pinned) || rowName(a).localeCompare(rowName(b)));
}

/** What the box understood, as a hint line. Empty input says nothing. */
export function understood(input: string): { kind: "none" | "ok" | "problem"; text: string } {
  if (input.trim() === "") return { kind: "none", text: "" };
  const a: AddInput = classifyAddInput(input);
  if (a.kind === "invalid") return { kind: "problem", text: a.problem };
  if (a.kind === "node_id") return { kind: "ok", text: `A node id: ${a.bech32.slice(0, 9)}…${a.bech32.slice(-6)}. Their card is asked for from your home hub.` };
  return { kind: "ok", text: `A card (${a.body.card.length} characters). Their keys come with it.` };
}

/** What the page knew of the peer before the add: `unknown` when the contact list had not loaded. */
export type AddPrior = "new" | "contact" | "unknown";

/**
 * The sentence after a successful add. It branches on what the node returned: a peer that already wrote to us comes back as a
 * pending request (nothing changed, and it cannot be written to until accepted); `prior` says whether a real contact was
 * known before, or that the page could not tell. A contact without a key waits for its card; messages wait in the outbox.
 */
export function addedSentence(c: Pick<Contact, "alias" | "label" | "has_key"> & Partial<Pick<Contact, "request">>, prior: AddPrior): string {
  const name = rowName(c);
  if (c.request) return `${name} already wrote to you, so this is a message request. Accept it under message requests to write back. Nothing was sent to them.`;
  if (prior === "contact") return `${name} is already a contact. Nothing changed.`;
  const keyed = c.has_key ? "You can write to them now." : "Their card is being asked for from your home hub; messages wait in the outbox until it arrives.";
  return prior === "unknown" ? `${name} is in your contacts. ${keyed}` : `Added ${name}. ${keyed}`;
}

/** Classify the peer from the contact list as the page held it before the add. Requests are not contacts yet. */
export function addPrior(before: readonly Pick<Contact, "node_id" | "request">[] | null | undefined, nodeId: string): AddPrior {
  if (!before) return "unknown";
  return before.some((c) => c.node_id === nodeId && !c.request) ? "contact" : "new";
}

/** The sentence after a failed add: a key conflict is spelled out because it is the one a person must act on. */
export function addFailureSentence(err: unknown): string {
  const text = errorSentence(err, "The contact could not be added. Try again.");
  return /conflict|different keys|do not match/i.test(text) ? `${text} The stored keys were not changed.` : text;
}

/**
 * Forgetting drops their keys from this node; the history stays until space is needed (API: forgetContact). Messages still
 * waiting to go out are not dropped: they stay in the outbox (waiting for a key), and after a restart the peer is listed
 * again as a contact without keys (mesh111 fabric forget).
 */
export const FORGET_NOTE =
  "Forgetting drops their keys from this node. The messages you already have stay until the space is needed. Messages still waiting to go out stay in the outbox; after a restart the person shows up again as a contact without keys.";
