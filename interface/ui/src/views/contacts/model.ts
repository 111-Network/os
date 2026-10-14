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

/** The sentence after a successful add. A contact without a key waits for its card; messages wait in the outbox. */
export function addedSentence(c: Pick<Contact, "alias" | "label" | "has_key">, already: boolean): string {
  const name = rowName(c);
  if (already) return `${name} is already a contact. Nothing changed.`;
  return c.has_key ? `Added ${name}. You can write to them now.` : `Added ${name}. Their card is being asked for from your home hub; messages wait in the outbox until it arrives.`;
}

/** The sentence after a failed add: a key conflict is spelled out because it is the one a person must act on. */
export function addFailureSentence(err: unknown): string {
  const text = errorSentence(err, "The contact could not be added. Try again.");
  return /conflict|different keys|do not match/i.test(text) ? `${text} The stored keys were not changed.` : text;
}

/** Forgetting drops their keys from this node; the history stays until space is needed (API: forgetContact). */
export const FORGET_NOTE = "Forgetting drops their keys from this node. The messages you already have stay until the space is needed.";
