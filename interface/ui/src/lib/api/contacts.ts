// Contacts slice (os/messaging/README.md §7, §15, §16; os/identity/README.md §7): pure helpers for what a user types,
// pastes or scans into "add a contact", plus the card QR matrix and the list operations the store applies.
//
// Nothing here is authoritative: the daemon validates a card against its own announce rules and a NodeId against
// the expected length. These checks only catch typos before a request leaves the page, and every one fails closed
// (an input that cannot be classified is refused with a plain sentence, never guessed at).
import { own } from "./own";
import type { Card, Contact, ContactCreate, NodeId } from "./types";

/** The two request bodies of `POST /contacts` (exactly one of the keys). */
export type ContactAddByCard = Extract<ContactCreate, { card: string }>;
export type ContactAddByNodeId = Extract<ContactCreate, { node_id: string }>;

// --- NodeId: 64 lowercase hex, or bech32 per BIP-173 (NOT bech32m) with hrp `node` ---------------------------------

const BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const BECH32_GEN = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
/** hrp `node`, `1`, 52 data characters (32 bytes) and the 6-character checksum. */
export const BECH32_NODE_LENGTH = 63;

function polymod(values: readonly number[]): number {
  let chk = 1;
  for (const v of values) {
    const top = chk >>> 25;
    chk = ((chk & 0x1ffffff) << 5) ^ v;
    for (let i = 0; i < 5; i++) if ((top >>> i) & 1) chk ^= BECH32_GEN[i]!;
  }
  return chk >>> 0;
}

function hrpExpand(hrp: string): number[] {
  const out: number[] = [];
  for (let i = 0; i < hrp.length; i++) out.push(hrp.charCodeAt(i) >>> 5);
  out.push(0);
  for (let i = 0; i < hrp.length; i++) out.push(hrp.charCodeAt(i) & 31);
  return out;
}

/** Regroup 5-bit values into bytes. Padding must be fewer than 5 bits and all zero (BIP-173). Null when it is not. */
function fiveToEight(data: readonly number[]): number[] | null {
  let acc = 0;
  let bits = 0;
  const out: number[] = [];
  for (const v of data) {
    acc = (acc << 5) | v;
    bits += 5;
    while (bits >= 8) {
      bits -= 8;
      out.push((acc >>> bits) & 0xff);
    }
    acc &= (1 << bits) - 1;
  }
  if (bits >= 5 || acc !== 0) return null;
  return out;
}

/** Why an input is not a NodeId. */
export type NodeIdProblem = "empty" | "mixed_case" | "uppercase_hex" | "wrong_prefix" | "wrong_length" | "bad_character" | "bad_checksum";

export type NodeIdResult = { ok: true; hex: NodeId; bech32: string } | { ok: false; problem: NodeIdProblem };

const HEX64 = /^[0-9a-f]{64}$/;

const toHex = (bytes: readonly number[]): string => bytes.map((b) => b.toString(16).padStart(2, "0")).join("");

/**
 * Decode a typed or scanned NodeId. Accepts 64 lowercase hex or `node1…` bech32 (BIP-173): all lowercase for display,
 * all UPPERCASE for QR transfer, never mixed. The data part must be exactly 32 bytes (BIP-173's insertion/deletion
 * weakness is only closed for fixed-size payloads, identity README §7).
 */
export function parseNodeId(input: string): NodeIdResult {
  const raw = input.replace(/\s+/g, "");
  if (raw === "") return { ok: false, problem: "empty" };
  if (HEX64.test(raw)) return { ok: true, hex: raw, bech32: encodeNodeBech32(raw) };
  if (/^[0-9A-Fa-f]{64}$/.test(raw)) return { ok: false, problem: "uppercase_hex" }; // hex is canonical lowercase only
  if (raw !== raw.toLowerCase() && raw !== raw.toUpperCase()) return { ok: false, problem: "mixed_case" };
  const s = raw.toLowerCase();
  if (!s.startsWith("node1")) return { ok: false, problem: "wrong_prefix" };
  if (s.length !== BECH32_NODE_LENGTH) return { ok: false, problem: "wrong_length" };
  const data: number[] = [];
  for (const ch of s.slice(5)) {
    const v = BECH32_CHARSET.indexOf(ch);
    if (v < 0) return { ok: false, problem: "bad_character" };
    data.push(v);
  }
  if (polymod([...hrpExpand("node"), ...data]) !== 1) return { ok: false, problem: "bad_checksum" };
  const bytes = fiveToEight(data.slice(0, -6));
  if (!bytes || bytes.length !== 32) return { ok: false, problem: "wrong_length" };
  return { ok: true, hex: toHex(bytes), bech32: s };
}

/** Encode 32 bytes of hex as `node1…` (display form). Used for fixtures and for showing a typed hex id the way the daemon does. */
export function encodeNodeBech32(hex: string): string {
  const bytes: number[] = [];
  for (let i = 0; i < hex.length; i += 2) bytes.push(parseInt(hex.slice(i, i + 2), 16));
  const data: number[] = [];
  let acc = 0;
  let bits = 0;
  for (const b of bytes) {
    acc = (acc << 8) | b;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      data.push((acc >>> bits) & 31);
    }
    acc &= (1 << bits) - 1;
  }
  if (bits > 0) data.push((acc << (5 - bits)) & 31);
  const mod = polymod([...hrpExpand("node"), ...data, 0, 0, 0, 0, 0, 0]) ^ 1;
  const checksum = Array.from({ length: 6 }, (_, i) => (mod >>> (5 * (5 - i))) & 31);
  return `node1${[...data, ...checksum].map((v) => BECH32_CHARSET[v]).join("")}`;
}

export const NODE_ID_PROBLEM_TEXT: Record<NodeIdProblem, string> = {
  empty: "Paste a card or a node1… id first.",
  mixed_case: "A node id is all lowercase or all uppercase, never mixed. Check how it was copied.",
  uppercase_hex: "A node id in hex is lowercase. Use the node1… form, or type the hex in lowercase.",
  wrong_prefix: "A node id starts with node1.",
  wrong_length: "That node id is the wrong length. A complete one is 63 characters.",
  bad_character: "That node id has a character that cannot appear in one.",
  bad_checksum: "That node id does not check out: one character is wrong. Copy it again.",
};

// --- Card text: the base32 form of the node's announce (§7.2 (1)) -------------------------------------------------

/** A genesis announce is 137–169 bytes (L2 §6.2): about 220–271 base32 characters. */
export const CARD_MIN_CHARS = 120;
/** The daemon's `MAX_CARD_TEXT` (mesh111 contacts.rs): the longest card text it accepts. The card the page sends has no separators. */
export const CARD_MAX_CHARS = 512;
/** Pasted text longer than this is refused before any pattern runs: the card cap plus room for the grouping marks people add. */
export const CARD_RAW_MAX_CHARS = CARD_MAX_CHARS + 256;

export type CardProblem = "empty" | "too_short" | "too_long" | "bad_character";

export type CardResult = { ok: true; card: string } | { ok: false; problem: CardProblem };

/**
 * Normalize pasted or scanned card text the way the daemon's `parse_card_text` reads it: whitespace, the dashes people
 * group it with and `=` padding are dropped (any case is fine); what is left must be plain letters and digits and fit
 * the daemon's cap. The daemon decodes and verifies the announce itself.
 */
export function parseCardText(input: string): CardResult {
  if (input.length > CARD_RAW_MAX_CHARS) return { ok: false, problem: "too_long" };
  const card = input.replace(/[\s=-]+/g, "");
  if (card === "") return { ok: false, problem: "empty" };
  if (!/^[A-Za-z0-9]+$/.test(card)) return { ok: false, problem: "bad_character" };
  if (card.length < CARD_MIN_CHARS) return { ok: false, problem: "too_short" };
  if (card.length > CARD_MAX_CHARS) return { ok: false, problem: "too_long" };
  return { ok: true, card };
}

export const CARD_PROBLEM_TEXT: Record<CardProblem, string> = {
  empty: NODE_ID_PROBLEM_TEXT.empty,
  too_short: "That is too short to be a card. A card is about 220 characters; a node id starts with node1.",
  too_long: "That is too long to be a card.",
  bad_character: "A card is letters and digits only. Check that nothing else was copied with it.",
};

/** What the "add a contact" box understood. */
export type AddInput =
  | { kind: "node_id"; body: ContactAddByNodeId; bech32: string }
  | { kind: "card"; body: ContactAddByCard }
  | { kind: "invalid"; problem: string };

/**
 * Classify one pasted or scanned string: a `node1…` / hex NodeId, or a card. A NodeId is sent as 64 hex (the
 * canonical form, §7.2 (2)); a card as the cleaned text. Anything else is refused with a sentence.
 */
export function classifyAddInput(input: string): AddInput {
  if (input.length > CARD_RAW_MAX_CHARS) return { kind: "invalid", problem: CARD_PROBLEM_TEXT.too_long };
  const raw = input.trim();
  if (raw === "") return { kind: "invalid", problem: NODE_ID_PROBLEM_TEXT.empty };
  const compact = raw.replace(/\s+/g, "");
  if (/^node1/i.test(compact) || /^[0-9a-fA-F]{64}$/.test(compact)) {
    const id = parseNodeId(compact);
    if (id.ok) return { kind: "node_id", body: { node_id: id.hex }, bech32: id.bech32 };
    return { kind: "invalid", problem: NODE_ID_PROBLEM_TEXT[id.problem] };
  }
  const card = parseCardText(raw);
  if (card.ok) return { kind: "card", body: { card: card.card } };
  return { kind: "invalid", problem: CARD_PROBLEM_TEXT[card.problem] };
}

/** `ContactCreate` for a classified input, or null when it was invalid. */
export const createBody = (a: AddInput): ContactCreate | null => (a.kind === "invalid" ? null : a.body);

// --- Card QR (the module matrix the UI draws itself, §7.2) --------------------------------------------------------

export const QR_MIN_SIZE = 21; // version 1
export const QR_MAX_SIZE = 177; // version 40
/** Light border around the symbol, in modules. The API's matrix has none (`QrMatrix`). */
export const QR_QUIET_ZONE = 4;

/** Dark modules as booleans, or null when the matrix is not a square of '0'/'1' within QR bounds. Never throws. */
export function qrModules(qr: Card["qr"] | null | undefined): boolean[][] | null {
  if (!qr || !Number.isInteger(qr.size) || qr.size < QR_MIN_SIZE || qr.size > QR_MAX_SIZE) return null;
  if (!Array.isArray(qr.rows) || qr.rows.length !== qr.size) return null;
  const out: boolean[][] = [];
  for (const row of qr.rows) {
    if (typeof row !== "string" || row.length !== qr.size || !/^[01]+$/.test(row)) return null;
    out.push(Array.from(row, (c) => c === "1"));
  }
  return out;
}

/** Minimal 2-D context the drawing needs, so it can be tested without a canvas. */
export interface QrContext {
  fillStyle: string | CanvasGradient | CanvasPattern;
  fillRect(x: number, y: number, w: number, h: number): void;
}

/** Whole pixels per module so the symbol stays crisp: as many as fit `targetPx`, at least 2. */
export function qrScale(size: number, targetPx: number): number {
  return Math.max(2, Math.floor(targetPx / (size + 2 * QR_QUIET_ZONE)));
}

/**
 * Paint the symbol with its quiet zone: `light` over the whole plate, `dark` for each dark module. Returns the
 * canvas side in pixels. A scanner needs dark modules on a light plate, so the colors are the caller's.
 */
export function drawQr(ctx: QrContext, modules: readonly (readonly boolean[])[], scale: number, dark: string, light: string): number {
  const side = (modules.length + 2 * QR_QUIET_ZONE) * scale;
  ctx.fillStyle = light;
  ctx.fillRect(0, 0, side, side);
  ctx.fillStyle = dark;
  modules.forEach((row, y) =>
    row.forEach((on, x) => {
      if (on) ctx.fillRect((x + QR_QUIET_ZONE) * scale, (y + QR_QUIET_ZONE) * scale, scale, scale);
    }),
  );
  return side;
}

// --- Contact list operations (store) -----------------------------------------------------------------------------

/** Pending message requests (§7.2 (3)): not contacts until accepted. */
export const openRequests = (contacts: readonly Contact[] | null | undefined): Contact[] => (contacts ? contacts.filter((c) => c.request) : []);

/** Contacts proper: everything that is not a pending request. */
export const realContacts = (contacts: readonly Contact[]): Contact[] => contacts.filter((c) => !c.request);

/** Replace by `node_id` or append. Keeps the previous object when nothing changed. */
export function upsertContact(contacts: readonly Contact[] | null, contact: Contact): Contact[] {
  const list = contacts ?? [];
  const at = list.findIndex((c) => c.node_id === contact.node_id);
  if (at < 0) return [...list, contact];
  if (JSON.stringify(list[at]) === JSON.stringify(contact)) return list as Contact[];
  const next = list.slice();
  next[at] = contact;
  return next;
}

export const removeContact = (contacts: readonly Contact[] | null, nodeId: NodeId): Contact[] => (contacts ?? []).filter((c) => c.node_id !== nodeId);

/** The name to show: the local alias, else the operator label. Never the bech32 (too long) and never a claim of the peer. */
export const contactName = (c: Pick<Contact, "alias" | "label">): string => (c.alias && c.alias.trim() !== "" ? c.alias : c.label);

/** Role strings the UI knows (appdata, §7.3). Unknown roles are ignored. */
export const ROLE_LABEL: Record<string, string> = {
  mailbox: "mailbox",
  always_on_hub: "always-on hub",
  lora_attached: "LoRa",
};

/** Plain labels for the roles this UI knows, in the order the contact lists them. */
export function roleLabels(roles: readonly string[] | undefined): string[] {
  return (roles ?? []).flatMap((r) => (own(ROLE_LABEL, r) ? [ROLE_LABEL[r]!] : []));
}

/** How the keys were learned, in words (§7.1). Unknown values read as "unknown". */
export function trustLabel(trust: string): string {
  switch (trust) {
    case "oob":
      return "added from a card";
    case "tofu":
      return "introduced by their message";
    case "enrolled":
      return "enrolled with a hub";
    default:
      return "unknown";
  }
}
