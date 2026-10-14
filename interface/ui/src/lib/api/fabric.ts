// Fabric slice (os/messaging/README.md §8–§10, §15): pure helpers over FabricStatus. The store keeps the snapshot
// (`GET /fabric`, replaced by each `fabric` frame); views and the Settings page read these summaries.
import type { FabricHub, FabricStatus, FabricStore } from "./types";

/** The store is full: new custody and bulk are refused, and new messages wait (§10.4). */
export const isStoreFull = (f: Pick<FabricStatus, "store"> | null | undefined): boolean => f?.store.store_full === true;

/** The store is not open (it failed to open, or the daemon has not started the fabric yet). */
export const isStoreOpen = (f: Pick<FabricStatus, "store"> | null | undefined): boolean => f?.store.open === true;

/** Bytes used as a share of the high watermark, 0–100 (not above, so a gauge never overflows). */
export function storePercent(s: Pick<FabricStore, "used" | "watermark">): number {
  if (!(s.watermark > 0) || !(s.used >= 0)) return 0;
  return Math.min(100, Math.round((s.used / s.watermark) * 100));
}

const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

/** 1 KB = 1000 bytes, one decimal below 10 (the unit the rest of the dashboard uses). */
export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "unknown";
  let v = n;
  let u = 0;
  while (v >= 1000 && u < UNITS.length - 1) {
    v /= 1000;
    u++;
  }
  return `${u === 0 || v >= 10 ? Math.round(v) : v.toFixed(1)} ${UNITS[u]}`;
}

/** Where this node stands with a home hub, in one phrase (member side first, then the hub role). */
export function hubSummary(h: Pick<FabricHub, "enrolled" | "reachable" | "members" | "offer_active">): string {
  if (h.enrolled) return h.reachable ? "Home hub reachable" : "Home hub enrolled, not reachable right now";
  if (h.members > 0) return h.members === 1 ? "Home hub for 1 member" : `Home hub for ${h.members} members`;
  if (h.offer_active) return "Waiting for a node to enroll";
  return "No home hub";
}

/** Can a message to a contact leave this node by a link right now? A member needs a reachable hub; a direct link also works. */
export const hasUpLink = (f: Pick<FabricStatus, "links"> | null | undefined): boolean => f?.links.some((l) => l.state === "up") === true;
