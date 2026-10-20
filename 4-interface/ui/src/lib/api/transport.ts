// Transport slice (os/6-transport/README.md §11, §15.8): the rnsd posture, region and callsign, as text. Pure.
import type { TransportPosture, TransportPostureResult, TransportStatus } from "./types";

/** The region is a human fact (D17 #10): "not set" until a person picks one, and a board transmits nothing before. */
export const regionText = (t: Pick<TransportStatus, "region"> | null | undefined): string => (t?.region ? t.region : "not set");

export const REGION_UNSET_TEXT = "No region is set, so a Meshtastic board transmits nothing yet. Pick the region you are in; setting it is your confirmation that it is legal where you are.";

/**
 * With `enable_transport` on and a LAN Backbone listener without IFAC, this node routes other people's Reticulum traffic, over
 * its own RNode too (§15.8). IFAC is the limit.
 */
export const routesOthers = (p: Pick<TransportPosture, "enable_transport" | "lan">): boolean => p.enable_transport && p.lan.backbone_listen && !p.lan.ifac;

export const ROUTING_WARNING = "With transport on and the LAN listener open without an IFAC passphrase, this node routes other people's Reticulum traffic, over its own radio too. An IFAC passphrase is the limit: only nodes that know it can use this hub.";

/** The sentence under a posture change the node answered: applied, or reverted and why. */
export function postureOutcomeText(r: Pick<TransportPostureResult, "applied" | "reverted" | "reason">): string {
  if (r.reverted) return `The node tried this and went back to the previous setting.${r.reason ? ` ${r.reason}` : ""}`;
  return r.applied ? "Applied. The node checked that it works." : "Saved.";
}

/** "AutoInterface · up · 3 peers" for one interface line. Names and types are rnsd's strings: text only. */
export function interfaceText(i: TransportStatus["interfaces"][number]): string {
  const peers = i.peers === null ? "" : i.peers === 1 ? " · 1 peer" : ` · ${i.peers} peers`;
  return `${i.type} · ${i.up ? "up" : "down"}${peers}`;
}

export const INTERFACES_STATE_TEXT: Record<TransportStatus["interfaces_state"], string> = {
  ok: "Interface facts are current.",
  stale: "The last interface check is out of date; these facts may have changed.",
  unavailable: "Reticulum is not answering, so no interface facts are available.",
};
