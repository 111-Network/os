// Plain-words summary of what needs the user, for the header badge tooltip.
import type { Attention } from "./api/store";

const plural = (n: number, one: string, many: string): string => `${n} ${n === 1 ? one : many}`;

/**
 * Where the header badge leads. Overview's "Needs you" block lists proposals and service and device errors; message
 * requests live in Messages. When requests are the only thing the badge counts, it opens Messages, so the number never
 * leads to a page that does not show what it counted.
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
