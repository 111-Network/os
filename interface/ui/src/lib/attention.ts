// Plain-words summary of what needs the user, for the header badge tooltip.
import type { Attention } from "./api/store";

const plural = (n: number, one: string, many: string): string => `${n} ${n === 1 ? one : many}`;

/** "2 proposals, 1 service error, 1 device error". Nothing pending reads "Nothing needs you". */
export function attentionSummary(a: Pick<Attention, "openProposals" | "erroredServices" | "erroredDevices">): string {
  const parts: string[] = [];
  if (a.openProposals > 0) parts.push(plural(a.openProposals, "proposal", "proposals"));
  if (a.erroredServices > 0) parts.push(plural(a.erroredServices, "service error", "service errors"));
  if (a.erroredDevices > 0) parts.push(plural(a.erroredDevices, "device error", "device errors"));
  return parts.length > 0 ? parts.join(", ") : "Nothing needs you";
}
