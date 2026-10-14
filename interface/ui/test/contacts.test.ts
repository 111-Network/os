// Contacts: NodeId parsing against the identity layer's known answers, card text, the add-box classifier, the QR matrix
// and its round trip through the real decoder (jsQR), camera support, and the contact list rules.
import { describe, expect, it } from "vitest";
import {
  BECH32_NODE_LENGTH,
  CARD_MAX_CHARS,
  CARD_MIN_CHARS,
  classifyAddInput,
  contactName,
  createBody,
  drawQr,
  encodeNodeBech32,
  openRequests,
  parseCardText,
  parseNodeId,
  qrModules,
  qrScale,
  QR_QUIET_ZONE,
  realContacts,
  removeContact,
  roleLabels,
  trustLabel,
  upsertContact,
  type QrContext,
} from "../src/lib/api/contacts";
import { cardFor, encodeQr } from "../src/lib/api/mock/qr";
import { ADA, BRAM, DOV, makeContact, mockCardText } from "../src/lib/api/mock/messaging";
import { isLoopbackHost, isSecureOrigin, panelOrigin, panelShowsFrame } from "../src/lib/api/panels";
import type { Contact } from "../src/lib/api/types";
import { addedSentence, addFailureSentence, aliasFromDraft, ALIAS_MAX, rowFacts, rowName, sortedContacts, understood } from "../src/views/contacts/model";
import { cameraErrorText, cameraSupport, decodeFrame, fitScale, loadDecoder, MAX_SCAN_FILE_BYTES, scanFile } from "../src/views/contacts/scan";
import { ApiError, synthProblem } from "../src/lib/api/problem";

// Known answers from os/identity/test-vectors/encoding.json (BIP-173 bech32, hrp "node", NOT bech32m).
const KATS = [
  { hex: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f", bech32: "node1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s2mc22u" },
  { hex: "6f1d5331e36e16d341fa06c9f622cc8569710f2a62d7491e9be8e953df844f01", bech32: "node1duw4xv0rdctdxs06qmylvgkvs45hzre2vtt5j85mar548huyfuqs44v6c9" },
  { hex: "0".repeat(64), bech32: "node1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqr0ng29" },
  { hex: "f".repeat(64), bech32: "node1lllllllllllllllllllllllllllllllllllllllllllllllllllsp3serj" },
  { hex: "5ddf5911edf0541c88e864d7590045c410f420d04e73f9d8507dc856591506b0", bech32: "node1th04jy0d7p2pez8gvnt4jqz9csg0ggxsfeelnkzs0hy9vkg4q6cqpkg7w5" },
];

const NEGATIVES = [
  ["bech32m checksum over the same payload", "node1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0sl8gx07"],
  ["bech32m, uppercase", "NODE1QQQSYQCYQ5RQWZQFPG9SCRGWPUGPZYSNZS23V9CCRYDPK8QARC0SL8GX07"],
  ["0-byte payload", "node1su9jf6"],
  ["31-byte payload", "node1tfd95kj6tfd95kj6tfd95kj6tfd95kj6tfd95kj6tfd95kj6tge7qgwk"],
  ["33-byte payload", "node1tfd95kj6tfd95kj6tfd95kj6tfd95kj6tfd95kj6tfd95kj6tfd95kd0d2p"],
  ["leading capital", "Node1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s2mc22u"],
  ["one lowercase char in the QR form", "NODE1QQQSYqCYQ5RQWZQFPG9SCRGWPUGPZYSNZS23V9CCRYDPK8QARC0S2MC22U"],
  ["npub", "npub1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0st5hsmq"],
  ["tampered data character", "node1qqqsyqcyq5rqwzqxpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s2mc22u"],
  ["character outside the charset", "node1qbqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s2mc22u"],
  ["non-canonical zero padding", "node1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc03hdvlhw"],
  ["empty", ""],
  ["bare node", "node"],
  ["node1", "node1"],
  ["garbage", "node1QQQSYQCYQ5!"],
] as const;

describe("parseNodeId", () => {
  it("encodes and decodes the identity layer's known answers", () => {
    for (const k of KATS) {
      expect(encodeNodeBech32(k.hex)).toBe(k.bech32);
      expect(parseNodeId(k.bech32)).toEqual({ ok: true, hex: k.hex, bech32: k.bech32 });
      expect(parseNodeId(k.hex)).toEqual({ ok: true, hex: k.hex, bech32: k.bech32 });
      expect(k.bech32).toHaveLength(BECH32_NODE_LENGTH);
    }
  });

  it("reads the UPPERCASE QR form and tolerates whitespace around a pasted id", () => {
    const k = KATS[0]!;
    expect(parseNodeId(k.bech32.toUpperCase())).toEqual({ ok: true, hex: k.hex, bech32: k.bech32 });
    expect(parseNodeId(`  ${k.bech32}\n`)).toMatchObject({ ok: true, hex: k.hex });
    expect(parseNodeId(`${k.bech32.slice(0, 20)} ${k.bech32.slice(20)}`)).toMatchObject({ ok: true, hex: k.hex });
  });

  it("refuses every negative vector, whatever the reason", () => {
    for (const [name, input] of NEGATIVES) expect(parseNodeId(input).ok, name).toBe(false);
  });

  it("names the reason in the cases a person can fix", () => {
    expect(parseNodeId("")).toEqual({ ok: false, problem: "empty" });
    expect(parseNodeId("Node1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s2mc22u")).toEqual({ ok: false, problem: "mixed_case" });
    expect(parseNodeId("npub1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0st5hsmq")).toEqual({ ok: false, problem: "wrong_prefix" });
    expect(parseNodeId("node1qqqsyqcyq5rqwz")).toEqual({ ok: false, problem: "wrong_length" });
    expect(parseNodeId("node1qbqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s2mc22u")).toEqual({ ok: false, problem: "bad_character" });
    expect(parseNodeId("node1qqqsyqcyq5rqwzqxpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s2mc22u")).toEqual({ ok: false, problem: "bad_checksum" });
    expect(parseNodeId("000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F")).toEqual({ ok: false, problem: "uppercase_hex" });
  });

  it("never throws on odd input", () => {
    for (const s of ["\u0000", "node1" + "q".repeat(5000), "💥", "node1‮qqq", " ", "0x" + "0".repeat(64)]) expect(() => parseNodeId(s)).not.toThrow();
  });
});

describe("card text", () => {
  const card = mockCardText(ADA);

  it("accepts letters and digits, drops grouping, and bounds the length", () => {
    expect(card).toHaveLength(224);
    expect(parseCardText(card)).toEqual({ ok: true, card });
    const grouped = card.match(/.{1,8}/g)!.join("-");
    expect(parseCardText(grouped)).toEqual({ ok: true, card });
    expect(parseCardText(card.match(/.{1,40}/g)!.join("\n"))).toEqual({ ok: true, card });
  });

  it("refuses empty, short, long and non-alphanumeric text", () => {
    expect(parseCardText("  ")).toEqual({ ok: false, problem: "empty" });
    expect(parseCardText("a".repeat(CARD_MIN_CHARS - 1))).toEqual({ ok: false, problem: "too_short" });
    expect(parseCardText("a".repeat(CARD_MAX_CHARS + 1))).toEqual({ ok: false, problem: "too_long" });
    expect(parseCardText("a".repeat(150) + "!")).toEqual({ ok: false, problem: "bad_character" });
    expect(parseCardText("a".repeat(150) + "é")).toEqual({ ok: false, problem: "bad_character" });
  });
});

describe("classifyAddInput (the add box)", () => {
  const k = KATS[1]!;

  it("sends a node id as 64 hex, whichever form it was typed in", () => {
    for (const input of [k.bech32, k.bech32.toUpperCase(), k.hex, `  ${k.hex} `]) {
      const r = classifyAddInput(input);
      expect(r).toMatchObject({ kind: "node_id", body: { node_id: k.hex }, bech32: k.bech32 });
    }
  });

  it("sends a card as cleaned text", () => {
    const card = mockCardText(BRAM);
    expect(classifyAddInput(`${card.slice(0, 100)}\n${card.slice(100)}`)).toEqual({ kind: "card", body: { card } });
  });

  it("refuses anything it cannot classify, with a sentence", () => {
    for (const input of ["", "   ", "hello", "node1abc", "node1qqqsyqcyq5rqwzqxpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s2mc22u", "a".repeat(64) + "G", "0".repeat(64).toUpperCase().replace(/0/g, "A")]) {
      const r = classifyAddInput(input);
      expect(r.kind, input).toBe("invalid");
      if (r.kind === "invalid") expect(r.problem.length).toBeGreaterThan(10);
    }
  });

  it("builds the request body only for a valid input", () => {
    expect(createBody(classifyAddInput(k.hex))).toEqual({ node_id: k.hex });
    expect(createBody(classifyAddInput("nope"))).toBeNull();
  });

  it("tells the person what it understood", () => {
    expect(understood("")).toEqual({ kind: "none", text: "" });
    expect(understood(k.bech32)).toMatchObject({ kind: "ok" });
    expect(understood(k.bech32).text).toMatch(/node id/i);
    expect(understood(mockCardText(ADA)).text).toMatch(/224 characters/);
    expect(understood("zzz")).toMatchObject({ kind: "problem" });
  });
});

describe("the card QR", () => {
  const card = cardFor(mockCardText(ADA));

  it("accepts a square matrix of 0/1 within QR bounds and nothing else", () => {
    const m = qrModules(card.qr)!;
    expect(m).toHaveLength(card.qr.size);
    expect(m.every((row) => row.length === card.qr.size)).toBe(true);
    expect(qrModules(null)).toBeNull();
    expect(qrModules(undefined)).toBeNull();
    expect(qrModules({ size: 20, rows: Array(20).fill("0".repeat(20)) })).toBeNull();
    expect(qrModules({ size: 178, rows: Array(178).fill("0".repeat(178)) })).toBeNull();
    expect(qrModules({ size: 21, rows: Array(20).fill("0".repeat(21)) })).toBeNull();
    expect(qrModules({ size: 21, rows: Array(21).fill("0".repeat(20)) })).toBeNull();
    expect(qrModules({ size: 21, rows: Array(21).fill("0".repeat(20) + "2") })).toBeNull();
    expect(qrModules({ size: 21.5, rows: [] })).toBeNull();
    expect(qrModules({ size: 21, rows: [...Array(20).fill("0".repeat(21)), 5 as never] })).toBeNull();
  });

  it("scales to whole pixels, at least 2 per module", () => {
    expect(qrScale(21, 320)).toBe(Math.floor(320 / 29));
    expect(qrScale(177, 100)).toBe(2);
  });

  function rasterize(modules: boolean[][], scale: number): { rgba: Uint8ClampedArray; side: number } {
    const side = (modules.length + 2 * QR_QUIET_ZONE) * scale;
    const rgba = new Uint8ClampedArray(side * side * 4);
    let color = [255, 255, 255];
    const ctx: QrContext = {
      get fillStyle() {
        return color.join(",");
      },
      set fillStyle(v) {
        color = String(v) === "#000000" ? [0, 0, 0] : [255, 255, 255];
      },
      fillRect(x, y, w, h) {
        for (let j = y; j < y + h; j++)
          for (let i = x; i < x + w; i++) {
            const o = (j * side + i) * 4;
            rgba[o] = color[0]!;
            rgba[o + 1] = color[1]!;
            rgba[o + 2] = color[2]!;
            rgba[o + 3] = 255;
          }
      },
    };
    expect(drawQr(ctx, modules, scale, "#000000", "#ffffff")).toBe(side);
    return { rgba, side };
  }

  it("draws the quiet zone light and dark modules dark", () => {
    const modules = qrModules(card.qr)!;
    const { rgba, side } = rasterize(modules, 4);
    expect(rgba[0]).toBe(255); // corner of the quiet zone
    const first = modules[0]!.findIndex(Boolean); // first dark module of the top row
    const o = ((QR_QUIET_ZONE * 4) * side + (QR_QUIET_ZONE + first) * 4) * 4;
    expect(rgba[o]).toBe(0);
  });

  it("round trip: the matrix the page draws is read back by jsQR as the card text", async () => {
    const modules = qrModules(card.qr)!;
    const scale = qrScale(modules.length, 400);
    const { rgba, side } = rasterize(modules, scale);
    const decode = await loadDecoder();
    expect(decodeFrame(decode, rgba, side, side)).toBe(card.text);
  });

  it("round trip of a NodeId QR in the UPPERCASE form the identity layer prints", async () => {
    const id = KATS[0]!.bech32.toUpperCase();
    const bytes = new TextEncoder().encode(id);
    const qr = encodeQr(bytes);
    const modules = qrModules(qr)!;
    const { rgba, side } = rasterize(modules, 6);
    const decode = await loadDecoder();
    const text = decodeFrame(decode, rgba, side, side);
    expect(text).toBe(id);
    expect(classifyAddInput(text!)).toMatchObject({ kind: "node_id", body: { node_id: KATS[0]!.hex } });
  });

  it("a blank frame, a wrong-size buffer and a throwing decoder all read as 'no code'", async () => {
    const decode = await loadDecoder();
    const blank = new Uint8ClampedArray(64 * 64 * 4).fill(255);
    expect(decodeFrame(decode, blank, 64, 64)).toBeNull();
    expect(decodeFrame(decode, blank, 10, 10)).toBeNull();
    expect(decodeFrame(decode, blank, 0, 64)).toBeNull();
    expect(
      decodeFrame(() => {
        throw new Error("boom");
      }, blank, 64, 64),
    ).toBeNull();
  });
});

describe("camera and file scanning", () => {
  it("offers the camera on HTTPS and loopback only", () => {
    expect(cameraSupport({ protocol: "https:", hostname: "node.example" }, true).ok).toBe(true);
    expect(cameraSupport({ protocol: "http:", hostname: "127.0.0.1" }, true).ok).toBe(true);
    expect(cameraSupport({ protocol: "http:", hostname: "localhost" }, true).ok).toBe(true);
    expect(cameraSupport({ protocol: "http:", hostname: "[::1]" }, true).ok).toBe(true);
    const lan = cameraSupport({ protocol: "http:", hostname: "192.168.1.20" }, true);
    expect(lan).toMatchObject({ ok: false, reason: "insecure" });
    expect(!lan.ok && lan.text).toMatch(/choose a picture/i);
    expect(cameraSupport({ protocol: "http:", hostname: "node.local" }, true).ok).toBe(false);
    expect(cameraSupport({ protocol: "https:", hostname: "node.example" }, true, false)).toMatchObject({ ok: false, reason: "insecure" });
    expect(cameraSupport({ protocol: "https:", hostname: "node.example" }, false)).toMatchObject({ ok: false, reason: "no_camera" });
  });

  it("does not treat a lookalike host as loopback", () => {
    for (const h of ["127.0.0.1.evil.example", "localhost.evil.example", "my127.0.0.1", "0.0.0.0", "10.0.0.1"]) expect(isLoopbackHost(h), h).toBe(false);
    expect(isSecureOrigin({ protocol: "http:", hostname: "127.0.0.1.evil.example" })).toBe(false);
    expect(isLoopbackHost("app.localhost")).toBe(true);
  });

  it("explains a camera failure in plain words", () => {
    expect(cameraErrorText("NotAllowedError")).toMatch(/not allowed/i);
    expect(cameraErrorText("NotFoundError")).toMatch(/no camera/i);
    expect(cameraErrorText("NotReadableError")).toMatch(/in use/i);
    expect(cameraErrorText(undefined)).toMatch(/could not be started/i);
    for (const n of ["NotAllowedError", "NotFoundError", "NotReadableError", "AbortError"]) expect(cameraErrorText(n)).toMatch(/picture/i);
  });

  it("fits a frame to the scan size without enlarging it", () => {
    expect(fitScale(640, 480)).toBe(1);
    expect(fitScale(2048, 1024)).toBe(0.5);
    expect(fitScale(1000, 3000, 1000)).toBeCloseTo(1 / 3);
  });

  it("refuses a file that is too large or not a picture before reading it", async () => {
    const big = { size: MAX_SCAN_FILE_BYTES + 1, type: "image/png" } as File;
    expect(await scanFile(big)).toMatchObject({ ok: false });
    expect(await scanFile({ size: 10, type: "text/plain" } as File)).toEqual({ ok: false, problem: "That file is not a picture." });
  });
});

describe("panel origins (the same secure-origin rule the camera uses)", () => {
  it("builds http://<host as typed>:<port> and brackets IPv6", () => {
    expect(panelOrigin({ port: 11240 }, { protocol: "http:", hostname: "192.168.1.20" })).toBe("http://192.168.1.20:11240");
    expect(panelOrigin({ port: 11240 }, { protocol: "http:", hostname: "::1" })).toBe("http://[::1]:11240");
    expect(panelOrigin({ port: 11240 }, { protocol: "https:", hostname: "node.example" })).toBe("https://node.example:11240");
  });

  it("frames a secure-context panel only on a trustworthy dashboard origin, and only while it listens", () => {
    const panel = { needs_secure_context: true, state: { enabled: true, listening: true, health: "up" as const } };
    expect(panelShowsFrame(panel, { protocol: "http:", hostname: "127.0.0.1" })).toBe(true);
    expect(panelShowsFrame(panel, { protocol: "http:", hostname: "192.168.1.20" })).toBe(false);
    expect(panelShowsFrame({ ...panel, needs_secure_context: false }, { protocol: "http:", hostname: "192.168.1.20" })).toBe(true);
    expect(panelShowsFrame({ ...panel, state: { ...panel.state, listening: false } }, { protocol: "http:", hostname: "127.0.0.1" })).toBe(false);
  });
});

describe("contact rows and lists", () => {
  const ada = makeContact(ADA, { alias: "Ada", trust: "enrolled", pinned: true, roles: ["mailbox", "mystery"] });
  const bram = makeContact(BRAM, { alias: null, trust: "oob", has_key: false });
  const dov = makeContact(DOV, { trust: "tofu", request: true });

  it("names by alias, else label", () => {
    expect(rowName(ada)).toBe("Ada");
    expect(contactName(bram)).toBe(bram.label);
    expect(contactName({ alias: "  ", label: "x" })).toBe("x");
  });

  it("states how the keys were learned, key state, known roles and pin; unknown trust reads unknown", () => {
    expect(rowFacts(ada)).toEqual(["enrolled with a hub", "mailbox", "pinned"]);
    expect(rowFacts(bram)).toEqual(["added from a card", "waiting for their card"]);
    expect(trustLabel("tofu")).toBe("introduced by their message");
    expect(trustLabel("???")).toBe("unknown");
    expect(roleLabels(["mailbox", "always_on_hub", "lora_attached", "x"])).toEqual(["mailbox", "always-on hub", "LoRa"]);
    expect(roleLabels(undefined)).toEqual([]);
  });

  it("separates requests from contacts and sorts pinned first, then by name", () => {
    const list = [bram, dov, ada];
    expect(openRequests(list).map((c) => c.node_id)).toEqual([DOV]);
    expect(openRequests(null)).toEqual([]);
    expect(realContacts(list).map((c) => c.node_id)).toEqual([BRAM, ADA]);
    expect(sortedContacts(list).map((c) => c.node_id)).toEqual([ADA, BRAM]);
  });

  it("upserts and removes without mutating, keeping identity when nothing changed", () => {
    const list: Contact[] = [ada, bram];
    expect(upsertContact(list, structuredClone(ada))).toBe(list);
    const changed = upsertContact(list, { ...ada, alias: "Ada W" });
    expect(changed).not.toBe(list);
    expect(changed[0]!.alias).toBe("Ada W");
    expect(list[0]!.alias).toBe("Ada");
    expect(upsertContact(list, dov)).toHaveLength(3);
    expect(upsertContact(null, dov)).toEqual([dov]);
    expect(removeContact(list, ADA).map((c) => c.node_id)).toEqual([BRAM]);
    expect(removeContact(null, ADA)).toEqual([]);
  });

  it("trims an alias, clears it when empty, and caps it", () => {
    expect(aliasFromDraft("  Ada  ")).toBe("Ada");
    expect(aliasFromDraft("   ")).toBeNull();
    expect(aliasFromDraft("x".repeat(100))).toHaveLength(ALIAS_MAX);
  });

  it("says what an add did", () => {
    expect(addedSentence(ada, false)).toMatch(/added ada/i);
    expect(addedSentence(bram, false)).toMatch(/home hub/i);
    expect(addedSentence(bram, false)).toMatch(/outbox/i);
    expect(addedSentence(ada, true)).toMatch(/already a contact/i);
  });

  it("surfaces a key conflict as a conflict and leaves other failures as the node said them", () => {
    const conflict = new ApiError({ ...synthProblem(409, "Conflict", "The card has different keys for a known node."), code: "bad_request" });
    expect(addFailureSentence(conflict)).toMatch(/different keys/);
    expect(addFailureSentence(conflict)).toMatch(/not changed/i);
    const other = new ApiError({ ...synthProblem(400, "Bad", "That is not a card."), code: "bad_request" });
    expect(addFailureSentence(other)).toBe("That is not a card.");
    expect(addFailureSentence(new Error("x"))).toMatch(/could not be added/i);
  });
});
