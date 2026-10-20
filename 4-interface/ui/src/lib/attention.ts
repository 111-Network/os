// Plain-words summary of what needs the user, for the header badge tooltip.
import type { Attention } from "./api/store";
import type { ForeignConversation } from "./api/types";

const plural = (n: number, one: string, many: string): string => `${n} ${n === 1 ? one : many}`;

/**
 * Where the header badge leads. Overview's "Needs you" block lists everything the badge counts: proposals, service and
 * device errors, and "N message requests" (`requestsItem`). When requests are the only thing counted, the badge skips the
 * extra hop and opens Messages directly, which is where that Overview row leads and where a request is answered. Anything
 * else pending opens "Needs you", which lists it all, requests included.
 */
export function attentionHref(a: Pick<Attention, "openProposals" | "erroredServices" | "erroredDevices"> & Partial<Pick<Attention, "openRequests">>): string {
  const requestsOnly = (a.openRequests ?? 0) > 0 && a.openProposals + a.erroredServices + a.erroredDevices === 0;
  return requestsOnly ? "/messages" : "/overview#needs-you";
}

/** "2 proposals, 1 service error, 1 device error, 1 message request". Nothing pending reads "Nothing needs you". */
export function attentionSummary(a: Pick<Attention, "openProposals" | "erroredServices" | "erroredDevices"> & Partial<Pick<Attention, "openRequests">>): string {
  const parts: string[] = [];
  if (a.openProposals > 0) parts.push(plural(a.openProposals, "proposal", "proposals"));
  if (a.erroredServices > 0) parts.push(plural(a.erroredServices, "service error", "service errors"));
  if (a.erroredDevices > 0) parts.push(plural(a.erroredDevices, "device error", "device errors"));
  if ((a.openRequests ?? 0) > 0) parts.push(plural(a.openRequests!, "message request", "message requests"));
  return parts.length > 0 ? parts.join(", ") : "Nothing needs you";
}

// --- Foreign networks (os/6-transport/README.md §10) ------------------------------------------------------------------------------
//
// The header badge counts proposals, errors and message requests: things that need a decision. It does not count unread messages,
// and a stranger's text on another network never raises it. A foreign DM to the board raises the unread count of its network only,
// and channel traffic raises nothing unless the channel is pinned. These two functions are the whole rule; `computeAttention`
// (lib/api/store.ts) takes no foreign input at all, which a test pins.

/** Does this conversation's unread count towards the unread number the Messages rail shows? A DM does, a channel only when pinned, a recipient list never. */
export const countsAsUnread = (c: Pick<ForeignConversation, "kind" | "pinned">): boolean => c.kind === "dm" || (c.kind === "channel" && c.pinned);

/** The unread number for a network's rail entry. Never the header badge. */
export const foreignUnread = (convs: readonly Pick<ForeignConversation, "kind" | "pinned" | "unread">[] | null | undefined): number =>
  (convs ?? []).reduce((n, c) => (countsAsUnread(c) ? n + c.unread : n), 0);
