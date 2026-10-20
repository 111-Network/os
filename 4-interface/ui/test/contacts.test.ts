// Contacts: NodeId parsing against the identity layer's known answers, card text, the add-box classifier, the QR matrix
// and its round trip through the real decoder (jsQR), camera support, and the contact list rules.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  BECH32_NODE_LENGTH,
  CARD_MAX_CHARS,
  CARD_MIN_CHARS,
  CARD_RAW_MAX_CHARS,
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
import { addedSentence, addFailureSentence, addPrior, aliasFromDraft, ALIAS_MAX, FORGET_NOTE, rowFacts, rowName, sortedContacts, understood } from "../src/views/contacts/model";
import {
  cameraErrorText,
  cameraSupport,
  decodeFrame,
  fitScale,
  imageSize,
  loadDecoder,
  MAX_SCAN_FILE_BYTES,
  MAX_SCAN_PIXELS,
  openCapture,
  pictureSizeProblem,
  scanFile,
  type QrDecoder,
} from "../src/views/contacts/scan";
import { ApiError, synthProblem } from "../src/lib/api/problem";

// The identity layer's known answers and negatives, loaded from the file itself (os/2-identity/test-vectors/encoding.json)
// so this test cannot drift from them: BIP-173 bech32, hrp "node", NOT bech32m.
interface Vector {
  name: string;
  hex: string;
  bech32: string;
  bech32_qr: string;
}
interface Negative {
  name: string;
  input: string;
  expect: string;
}
const VECTORS = JSON.parse(readFileSync(fileURLToPath(new URL("../../../2-identity/test-vectors/encoding.json", import.meta.url)), "utf8")) as {
  kats: Vector[];
  negatives: { hex: Negative[]; bech32: Negative[]; record: unknown[] };
};
const KATS = VECTORS.kats;

describe("parseNodeId", () => {
  it("encodes and decodes every known answer in encoding.json, in all three forms", () => {
    expect(KATS.length).toBeGreaterThanOrEqual(5);
    for (const k of KATS) {
      expect(encodeNodeBech32(k.hex), k.name).toBe(k.bech32);
      expect(parseNodeId(k.bech32), k.name).toEqual({ ok: true, hex: k.hex, bech32: k.bech32 });
      expect(parseNodeId(k.bech32_qr), `${k.name} (QR form)`).toEqual({ ok: true, hex: k.hex, bech32: k.bech32 });
      expect(parseNodeId(k.hex), k.name).toEqual({ ok: true, hex: k.hex, bech32: k.bech32 });
      expect(k.bech32).toHaveLength(BECH32_NODE_LENGTH);
    }
  });

  it("reads the UPPERCASE QR form and tolerates whitespace around a pasted id", () => {
    const k = KATS[0]!;
    expect(parseNodeId(k.bech32.toUpperCase())).toEqual({ ok: true, hex: k.hex, bech32: k.bech32 });
    expect(parseNodeId(`  ${k.bech32}\n`)).toMatchObject({ ok: true, hex: k.hex });
    expect(parseNodeId(`${k.bech32.slice(0, 20)} ${k.bech32.slice(20)}`)).toMatchObject({ ok: true, hex: k.hex });
  });

  it("refuses every hex and bech32 negative in encoding.json (none is skipped), and names a wrong prefix or length as such", () => {
    const { hex, bech32 } = VECTORS.negatives;
    expect(hex).toHaveLength(8);
    expect(bech32.length).toBeGreaterThanOrEqual(19);
    for (const n of [...hex, ...bech32]) {
      const r = parseNodeId(n.input);
      expect(r.ok, n.name).toBe(false);
      if (!r.ok && n.expect === "WrongHrp") expect(r.problem, n.name).toBe("wrong_prefix");
      if (!r.ok && n.expect.startsWith("InvalidLength") && bech32.includes(n)) expect(r.problem, n.name).toBe("wrong_length");
    }
  });

  it("refuses the bech32 negatives that the old hand-copied list left out (64-byte payload, test1, nsec1, 1qqq…)", () => {
    const names = VECTORS.negatives.bech32.map((n) => n.name);
    for (const part of ["64-byte", "wrong HRP test", "wrong HRP nsec", "1qqqsyqcyq5"]) expect(names.some((n) => n.includes(part)), part).toBe(true);
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

  it("reads card text the way the daemon's parse_card_text does: '=' padding is dropped like whitespace and dashes, any case", () => {
    expect(parseCardText(`${card}====`)).toEqual({ ok: true, card });
    expect(parseCardText(`${card}=`)).toEqual({ ok: true, card });
    expect(parseCardText(card.match(/.{1,8}/g)!.join("= ")).ok).toBe(true);
    expect(parseCardText(card.toLowerCase())).toEqual({ ok: true, card: card.toLowerCase() });
    expect(parseCardText("====")).toEqual({ ok: false, problem: "empty" });
    expect(classifyAddInput(`${card}==`)).toEqual({ kind: "card", body: { card } });
  });

  it("caps the card at the daemon's 512 characters, and the raw paste at 512 plus room for grouping marks", () => {
    expect(CARD_MAX_CHARS).toBe(512);
    expect(CARD_RAW_MAX_CHARS).toBeGreaterThan(CARD_MAX_CHARS);
    expect(parseCardText("a".repeat(CARD_MAX_CHARS))).toEqual({ ok: true, card: "a".repeat(CARD_MAX_CHARS) });
    expect(parseCardText("a".repeat(CARD_MAX_CHARS + 1))).toEqual({ ok: false, problem: "too_long" });
    // Grouped text with marks is fine up to the raw cap, as long as the card itself fits.
    expect(parseCardText("a".repeat(CARD_MAX_CHARS - 4).match(/.{1,4}/g)!.join("-")).ok).toBe(true);
    expect(parseCardText("a".repeat(CARD_RAW_MAX_CHARS + 1))).toEqual({ ok: false, problem: "too_long" });
  });

  it("refuses a huge paste before running any pattern over it", () => {
    const big = "A".repeat(5_000_000);
    const t0 = performance.now();
    expect(parseCardText(big)).toEqual({ ok: false, problem: "too_long" });
    expect(classifyAddInput(big)).toMatchObject({ kind: "invalid" });
    expect(classifyAddInput(" ".repeat(5_000_000))).toMatchObject({ kind: "invalid" });
    expect(performance.now() - t0).toBeLessThan(50);
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

describe("the picture cap counts pixels, not just bytes", () => {
  const view = (n: number) => {
    const b = new Uint8Array(n);
    return { b, dv: new DataView(b.buffer) };
  };
  const ascii = (b: Uint8Array, at: number, s: string) => b.set([...s].map((c) => c.charCodeAt(0)), at);

  const png = (w: number, h: number) => {
    const { b, dv } = view(33);
    b.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], 0);
    dv.setUint32(8, 13);
    ascii(b, 12, "IHDR");
    dv.setUint32(16, w);
    dv.setUint32(20, h);
    return b;
  };
  const gif = (w: number, h: number) => {
    const { b, dv } = view(13);
    ascii(b, 0, "GIF89a");
    dv.setUint16(6, w, true);
    dv.setUint16(8, h, true);
    return b;
  };
  /** SOI, an APP1 of `meta` bytes (EXIF can be large), a DHT (marker C4, not a frame) and then SOF0 (or SOF2 when progressive). */
  const jpeg = (w: number, h: number, meta = 300, progressive = false) => {
    const parts: number[] = [0xff, 0xd8, 0xff, 0xe1, (meta + 2) >> 8, (meta + 2) & 0xff, ...new Array<number>(meta).fill(0x45), 0xff, 0xc4, 0x00, 0x04, 0x00, 0x00];
    parts.push(0xff, progressive ? 0xc2 : 0xc0, 0x00, 0x11, 0x08, h >> 8, h & 0xff, w >> 8, w & 0xff, 0x03, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xda, 0x00, 0x02);
    return Uint8Array.from(parts);
  };
  const webpX = (w: number, h: number) => {
    const { b, dv } = view(30);
    ascii(b, 0, "RIFF");
    dv.setUint32(4, 22, true);
    ascii(b, 8, "WEBP");
    ascii(b, 12, "VP8X");
    dv.setUint32(16, 10, true);
    b[24] = (w - 1) & 0xff;
    b[25] = ((w - 1) >> 8) & 0xff;
    b[26] = ((w - 1) >> 16) & 0xff;
    b[27] = (h - 1) & 0xff;
    b[28] = ((h - 1) >> 8) & 0xff;
    b[29] = ((h - 1) >> 16) & 0xff;
    return b;
  };
  const webpL = (w: number, h: number) => {
    const { b, dv } = view(30);
    ascii(b, 0, "RIFF");
    ascii(b, 8, "WEBP");
    ascii(b, 12, "VP8L");
    b[20] = 0x2f;
    dv.setUint32(21, ((w - 1) & 0x3fff) | (((h - 1) & 0x3fff) << 14), true);
    return b;
  };
  const webpLossy = (w: number, h: number) => {
    const { b, dv } = view(30);
    ascii(b, 0, "RIFF");
    ascii(b, 8, "WEBP");
    ascii(b, 12, "VP8 ");
    b.set([0x9d, 0x01, 0x2a], 23);
    dv.setUint16(26, w, true);
    dv.setUint16(28, h, true);
    return b;
  };
  const bmp = (w: number, h: number) => {
    const { b, dv } = view(54);
    ascii(b, 0, "BM");
    dv.setUint32(14, 40, true);
    dv.setInt32(18, w, true);
    dv.setInt32(22, -h, true); // top-down
    return b;
  };

  it("reads the declared size from the header of PNG, GIF, JPEG, WebP (three kinds) and BMP", () => {
    expect(imageSize(png(640, 480))).toEqual({ width: 640, height: 480 });
    expect(imageSize(png(40000, 40000))).toEqual({ width: 40000, height: 40000 });
    expect(imageSize(gif(321, 123))).toEqual({ width: 321, height: 123 });
    expect(imageSize(jpeg(4032, 3024))).toEqual({ width: 4032, height: 3024 });
    expect(imageSize(jpeg(65535, 65535, 60_000, true))).toEqual({ width: 65535, height: 65535 }); // after a large metadata segment
    expect(imageSize(webpX(1234, 567))).toEqual({ width: 1234, height: 567 });
    expect(imageSize(webpL(800, 600))).toEqual({ width: 800, height: 600 });
    expect(imageSize(webpLossy(1024, 768))).toEqual({ width: 1024, height: 768 });
    expect(imageSize(bmp(300, 200))).toEqual({ width: 300, height: 200 });
  });

  it("does not take a Huffman table (marker C4) for a frame header, and gives up rather than guess", () => {
    const j = jpeg(100, 50);
    expect(imageSize(j)).toEqual({ width: 100, height: 50 });
    expect(imageSize(j.slice(0, 20))).toBeNull(); // cut short inside the metadata
    expect(imageSize(Uint8Array.from([0xff, 0xd8, 0xff, 0xda, 0x00, 0x02, 0, 0]))).toBeNull(); // a scan with no frame header
    expect(imageSize(new TextEncoder().encode("<svg xmlns='http://www.w3.org/2000/svg' width='99999' height='99999'/>"))).toBeNull();
    expect(imageSize(new Uint8Array(0))).toBeNull();
    expect(imageSize(Uint8Array.from([0xff, 0xd8, 0xff, 0xe0, 0x00, 0x00, 0xff, 0xe0, 0x00, 0x00]))).toBeNull(); // zero-length segments cannot loop
    for (let n = 0; n < 40; n++) expect(() => imageSize(png(10, 10).slice(0, n))).not.toThrow();
  });

  it("refuses a picture above the pixel cap, one that cannot be measured, and an empty one; allows the cap itself", () => {
    expect(MAX_SCAN_PIXELS).toBe(36_000_000);
    expect(pictureSizeProblem({ width: 640, height: 480 })).toBeNull();
    expect(pictureSizeProblem({ width: 6000, height: 6000 })).toBeNull();
    expect(pictureSizeProblem({ width: 6001, height: 6000 })).toMatch(/too many pixels/i);
    expect(pictureSizeProblem({ width: 40000, height: 40000 })).toMatch(/too many pixels/i);
    expect(pictureSizeProblem(null)).toMatch(/png or jpeg/i);
    expect(pictureSizeProblem({ width: 0, height: 10 })).toMatch(/could not be opened/i);
  });

  afterEach(() => vi.unstubAllGlobals());

  it("never decodes a small file that declares billions of pixels", async () => {
    const decode = vi.fn(() => Promise.resolve({ width: 1, height: 1, close: () => undefined }));
    vi.stubGlobal("createImageBitmap", decode);
    const bomb = new File([png(40000, 40000)], "bomb.png", { type: "image/png" });
    expect(bomb.size).toBeLessThan(MAX_SCAN_FILE_BYTES);
    const r = await scanFile(bomb);
    expect(r).toMatchObject({ ok: false });
    expect(!r.ok && r.problem).toMatch(/too many pixels/i);
    expect(decode).not.toHaveBeenCalled();
  });

  it("does not decode a picture it cannot measure, and does decode an ordinary one", async () => {
    const decode = vi.fn(() => Promise.reject(new Error("stop here")));
    vi.stubGlobal("createImageBitmap", decode);
    expect(await scanFile(new File([new Uint8Array(500)], "mystery.png", { type: "image/png" }))).toMatchObject({ ok: false });
    expect(decode).not.toHaveBeenCalled();
    expect(await scanFile(new File([png(800, 600)], "ok.png", { type: "image/png" }))).toEqual({ ok: false, problem: "That picture could not be opened." });
    expect(decode).toHaveBeenCalledTimes(1);
  });
});

describe("the camera is only asked for once the decoder is in hand (openCapture)", () => {
  const track = () => ({ stop: vi.fn() });
  const streamOf = (tracks: { stop: () => void }[]) => ({ getTracks: () => tracks }) as unknown as MediaStream;
  const decode: QrDecoder = () => null;

  it("a decoder chunk that fails to load never reaches getUserMedia, so no camera can be left on", async () => {
    const media = vi.fn(() => Promise.resolve(streamOf([track()])));
    await expect(openCapture({ decoder: () => Promise.reject(new Error("chunk 404")), media, stopped: () => false })).rejects.toThrow("chunk 404");
    expect(media).not.toHaveBeenCalled();
  });

  it("a view closed while the decoder loads never asks for the camera", async () => {
    const media = vi.fn(() => Promise.resolve(streamOf([track()])));
    let closed = false;
    const r = await openCapture({
      decoder: async () => {
        closed = true;
        return decode;
      },
      media,
      stopped: () => closed,
    });
    expect(r).toBeNull();
    expect(media).not.toHaveBeenCalled();
  });

  it("a camera that arrives after the view was closed is switched off at once", async () => {
    const t1 = track();
    const t2 = track();
    let closed = false;
    const r = await openCapture({
      decoder: async () => decode,
      media: async () => {
        closed = true; // the person closed the dialog while the permission prompt was open
        return streamOf([t1, t2]);
      },
      stopped: () => closed,
    });
    expect(r).toBeNull();
    expect(t1.stop).toHaveBeenCalledTimes(1);
    expect(t2.stop).toHaveBeenCalledTimes(1);
  });

  it("hands back the decoder and the running stream, untouched, when all goes well", async () => {
    const t = track();
    const stream = streamOf([t]);
    const r = await openCapture({ decoder: async () => decode, media: async () => stream, stopped: () => false });
    expect(r).toEqual({ decode, stream });
    expect(t.stop).not.toHaveBeenCalled();
  });

  it("a camera that is refused rejects with the browser's error for the view to word", async () => {
    const denied = Object.assign(new Error("denied"), { name: "NotAllowedError" });
    await expect(openCapture({ decoder: async () => decode, media: () => Promise.reject(denied), stopped: () => false })).rejects.toBe(denied);
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
    expect(addedSentence(ada, "new")).toMatch(/added ada/i);
    expect(addedSentence(bram, "new")).toMatch(/home hub/i);
    expect(addedSentence(bram, "new")).toMatch(/outbox/i);
    expect(addedSentence(ada, "contact")).toMatch(/already a contact/i);
  });

  it("an add that returns a pending request says so: nothing changed, accept it to write back, no 'already a contact'", () => {
    const pending = addedSentence(dov, "contact"); // even if the page's list counted the request as known
    expect(pending).toMatch(/already wrote to you/i);
    expect(pending).toMatch(/message request/i);
    expect(pending).toMatch(/accept/i);
    expect(pending).not.toMatch(/already a contact/i);
    expect(pending).not.toMatch(/^added/i);
    expect(addedSentence(dov, "new")).toBe(pending);
    expect(addedSentence(dov, "unknown")).toBe(pending);
  });

  it("when the contact list never loaded it does not claim 'Added'", () => {
    const unknown = addedSentence(ada, "unknown");
    expect(unknown).not.toMatch(/^added/i);
    expect(unknown).not.toMatch(/already a contact/i);
    expect(unknown).toMatch(/in your contacts/i);
    expect(addedSentence(bram, "unknown")).toMatch(/outbox/i);
  });

  it("works out what the page knew before the add: requests are not contacts, a list that never loaded is unknown", () => {
    expect(addPrior(null, ADA)).toBe("unknown");
    expect(addPrior(undefined, ADA)).toBe("unknown");
    expect(addPrior([ada, dov], ADA)).toBe("contact");
    expect(addPrior([ada, dov], DOV)).toBe("new"); // the node's answer carries `request`, the sentence branches on that
    expect(addPrior([ada, dov], BRAM)).toBe("new");
    expect(addPrior([], BRAM)).toBe("new");
  });

  it("the forget note says what forgetting keeps: history stays, and the outbox, which brings the person back after a restart", () => {
    expect(FORGET_NOTE).toMatch(/drops their keys/i);
    expect(FORGET_NOTE).toMatch(/stay until the space is needed/i);
    expect(FORGET_NOTE).toMatch(/waiting to go out stay in the outbox/i);
    expect(FORGET_NOTE).toMatch(/after a restart/i);
    expect(FORGET_NOTE).toMatch(/contact without keys/i);
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
