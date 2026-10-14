// Pure logic for the firmware-flash offer and its confirmation (Layer 6, D17). No React: unit-tested in node.
//
// The daemon decides who gets an offer (`DeviceRecord.flash_offer`); this module only turns the offer into
// plain words, says whether the confirmation is complete, and turns a failed request into one honest sentence.
// Human-confirmed only: nothing here can start a flash by itself.
import { isApiError } from "../../lib/api/problem";
import type { DeviceRecord, FlashOffer } from "../../lib/api/types";
import type { FlashRequest } from "../../lib/api/adapter";
import { confidenceLabel, displayName, kindTransport } from "../../lib/devices";
import { sanitizeLine } from "../../lib/prompts";

// ---------------------------------------------------------------- the tag and fixed copy

/** Text-only tag beside the NOT SET dot in the device list and on Overview. */
export const NEEDS_FIRMWARE = "NEEDS FIRMWARE";

/** True when the daemon offers firmware for this board. The UI renders the offer; it never decides on its own. */
export function needsFirmware(d: Pick<DeviceRecord, "flash_offer">): boolean {
  return !!d.flash_offer;
}

export const FLASH_NOT_AVAILABLE = "Flashing is not available on this node yet (the flashing core lands in Layer 6). The offer stays here.";
export const FLASH_STALE = "The offer changed; reload the device.";
export const FLASH_SOURCE = "Inside this node's signed release. Nothing is downloaded.";
export const FLASH_TITLE = "This board has no usable firmware.";

const FAMILY_NAME: Record<string, string> = { rnode: "RNode" };

/** Untrusted text from the node, cleaned and cut. */
function clean(s: unknown, max: number): string {
  return typeof s === "string" ? sanitizeLine(s).trim().slice(0, max) : "";
}

export function familyName(family: string): string {
  return FAMILY_NAME[family] ?? (clean(family, 24) || "new");
}

/** "Flash RNode firmware…": the one primary action of the panel. */
export function flashButtonLabel(offer: Pick<FlashOffer, "image">): string {
  return `Flash ${familyName(offer.image.family)} firmware…`;
}

// ---------------------------------------------------------------- offer -> display lines

export interface FlashLine {
  label: string;
  value: string;
}

export interface FlashDisplay {
  /** The board as the user knows it: their name, and the detected name when it differs. */
  board: string;
  /** What the node found on it, one sentence ("blank" and "unknown" read differently). */
  why: string;
  /** Present when the match is only by USB ids (they name the chip, not the board): tell the user to check. */
  check: string | null;
  /** Exactly what would be written. */
  lines: FlashLine[];
  sha: { short: string; full: string };
  /** Where the image comes from. */
  source: string;
  /** The warning sentence built from `erases`. */
  erases: string;
}

const HEX64 = /^[0-9a-f]{64}$/i;

/** "f87a2e8a…c84ed9c" for a 64-hex digest (first 8 and last 8 characters); anything else is cut to fit and shown as it came. */
export function shortSha(sha: string): string {
  const s = clean(sha, 128);
  return HEX64.test(s) ? `${s.slice(0, 8)}…${s.slice(-8)}` : s.length > 17 ? `${s.slice(0, 16)}…` : s;
}

/**
 * `erases` names what is lost as a phrase ("everything on the board, including any Meshtastic settings").
 * The warning sentence is built around it; a missing phrase falls back to the broadest true statement.
 */
export function erasesSentence(erases: string | undefined | null): string {
  const phrase = clean(erases, 300).replace(/[.\s]+$/, "");
  return phrase === "" ? "Flashing erases everything that is on the board now." : `Flashing erases ${phrase}.`;
}

export function whySentence(reason: FlashOffer["reason"] | string): string {
  return reason === "blank"
    ? "Nothing answered on this board, so it looks blank. It has no firmware this node can use."
    : "Something answered on this board, but it is not firmware this node can use.";
}

export function offerDisplay(device: Pick<DeviceRecord, "alias" | "name" | "kind" | "transport" | "match">, offer: FlashOffer): FlashDisplay {
  const shown = clean(displayName(device), 80);
  const detected = clean(device.name, 80);
  const board = shown !== detected && detected !== "" ? `${shown} (detected as ${detected})` : shown;
  const image = offer.image;
  const target = clean(image.board, 80);
  const full = clean(image.sha256, 128);
  const version = clean(image.version, 40);
  return {
    board: `${board} · ${kindTransport(device)}`,
    why: whySentence(offer.reason),
    check:
      confidenceLabel(device) === "identified"
        ? null
        : `The node matched this board by its USB ids, which name the chip, not the board. Check that it really is a ${target || "board of that kind"} before you go on.`,
    lines: [
      { label: "Firmware", value: `${familyName(image.family)}${version ? ` ${version}` : ""}` },
      { label: "Built for", value: target || "unknown" },
      { label: "Image file", value: clean(image.file, 200) || "unknown" },
      { label: "SHA-256", value: shortSha(full) },
    ],
    sha: { short: shortSha(full), full },
    source: FLASH_SOURCE,
    erases: erasesSentence(offer.erases),
  };
}

// ---------------------------------------------------------------- confirmation

export type FlashCheckId = "erase" | "own";

export const FLASH_CHECKS: ReadonlyArray<{ id: FlashCheckId; label: string }> = [
  { id: "erase", label: "I understand everything on the board will be erased" },
  { id: "own", label: "This board is mine or I am allowed to flash it" },
];

export interface FlashConfirm {
  checks: Record<FlashCheckId, boolean>;
  typed: string;
}

export const EMPTY_CONFIRM: FlashConfirm = { checks: { erase: false, own: false }, typed: "" };

/** The typed name equals the displayed name: both trimmed, case-sensitive, and not empty. */
export function nameMatches(typed: string, name: string): boolean {
  const want = name.trim();
  return want !== "" && typed.trim() === want;
}

export type FlashMissing = FlashCheckId | "name";

/** What the user still has to do, in the order the form shows it. Empty when the confirmation is complete. */
export function confirmMissing(c: FlashConfirm, name: string): FlashMissing[] {
  const out: FlashMissing[] = [];
  for (const { id } of FLASH_CHECKS) if (!c.checks[id]) out.push(id);
  if (!nameMatches(c.typed, name)) out.push("name");
  return out;
}

export function confirmValid(c: FlashConfirm, name: string): boolean {
  return confirmMissing(c, name).length === 0;
}

/** One sentence for the screen reader and the eye: why "Flash now" does nothing yet. Empty when it will work. */
export function missingText(missing: readonly FlashMissing[]): string {
  if (missing.length === 0) return "";
  const parts: string[] = [];
  const boxes = missing.filter((m) => m !== "name").length;
  if (boxes === FLASH_CHECKS.length) parts.push("tick both boxes");
  else if (missing.includes("erase")) parts.push("tick the first box");
  else if (missing.includes("own")) parts.push("tick the second box");
  if (missing.includes("name")) parts.push("type the name exactly as shown");
  return `Still needed: ${parts.join(" and ")}.`;
}

/** The request body. Only built from a complete confirmation, and always repeats the sha256 of the offer on screen. */
export function flashRequest(offer: FlashOffer, c: FlashConfirm): FlashRequest {
  return { sha256: offer.image.sha256, confirm: c.typed.trim() };
}

// ---------------------------------------------------------------- after the request

/**
 * One honest sentence for a failed flash request. 404 is the expected answer until Layer 6 ships the route;
 * 409 is a stale offer (sha256 mismatch) or a flash that is already running.
 */
export function flashErrorText(err: unknown): string {
  if (!isApiError(err)) return "Could not start the flash. Try again.";
  if (err.code === "network") return "Could not reach the node. Check that it is running and try again.";
  if (err.status === 404) return FLASH_NOT_AVAILABLE;
  if (err.status === 409) return err.code === "offer_stale" ? FLASH_STALE : err.problem.detail || "This board is already being flashed.";
  if (err.status === 400) return err.problem.detail || "The node did not accept the confirmation. Check the name and try again.";
  return err.problem.detail || err.problem.title || "Could not start the flash. Try again.";
}

/** After 202 the node reports the new firmware as a `devices` frame. If it has not by now, say so. */
export const FLASH_STALLED_AFTER_MS = 90_000;

export function flashStalled(startedAt: number, now: number): boolean {
  return now - startedAt >= FLASH_STALLED_AFTER_MS;
}

/** The board reports a family this node can use. */
export function hasFirmwareNow(d: Pick<DeviceRecord, "firmware">): boolean {
  const f = d.firmware?.family;
  return !!f && f !== "none" && f !== "unknown";
}
