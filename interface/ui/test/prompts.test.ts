import { describe, expect, it } from "vitest";
import { boundJson, boundLog, buildPrompt, questionPrompt, sanitizeLine, stagePrompt, takePrompt, truncateUtf8, type StorageLike } from "../src/lib/prompts";
import type { DeviceRecord, ServiceRecord, Status } from "../src/lib/api/sse.gen";

const status = { node: { label: "k7m2-9x4p-aa", alias: null, category: "Node2B", version: "2026.10.13" }, layers: [0, 1, 2, 3] } as unknown as Status;
const service = { id: "radio", name: "radio", plain_name: "LoRa radio", state: "unset", reason_code: "radio_unbound", reason: "board present, nothing bound", can_run_here: true, requires: "a mesh stack", kb_entry: "projects/meshtastic.md", detail: { x: 1 } } as unknown as ServiceRecord;
const device = { id: "usb-1", kind: "radio", alias: null, name: "Likely: Heltec WiFi LoRa 32 V3", state: "needs_setup", transport: "usb-serial", reason_code: "radio_unbound", reason: "nothing bound", match: { confidence: "likely", rule: "lora-table 10c4:ea60" }, firmware: { family: "unknown", version: null, source: "unknown" }, driver: null, capabilities: ["lora"], service_id: "radio", identity: { port: "/dev/ttyUSB0" }, kb_entry: "devices/heltec-wifi-lora-32-v3.md" } as unknown as DeviceRecord;

function mem(): StorageLike {
  const m = new Map<string, string>();
  return { getItem: (k) => m.get(k) ?? null, setItem: (k, v) => void m.set(k, v), removeItem: (k) => void m.delete(k) };
}

describe("buildPrompt", () => {
  it("setup for a service carries goal, node line, subject json, kb entries", () => {
    const p = buildPrompt("setup", { kind: "service", service, devices: [device] }, { status });
    expect(p.startsWith("Set up the LoRa radio on this node and test it.")).toBe(true);
    expect(p).toContain("category Node2B");
    expect(p).toContain("layers running 0, 1, 2, 3");
    expect(p).toContain('"reason_code":"radio_unbound"');
    expect(p).toContain("KB entries: projects/meshtastic.md, devices/heltec-wifi-lora-32-v3.md");
    expect(p).toContain("- related:");
  });
  it("connect over the registry names everything", () => {
    const p = buildPrompt("connect", { kind: "registry", devices: [device], services: [service] }, { status, rawFacts: { a: 1 } });
    expect(p).toContain("Look at everything connected or paired to this node");
    expect(p).toContain("- raw facts:");
  });
  it("fix for a device uses alias over name and includes the driving service", () => {
    const p = buildPrompt("fix", { kind: "device", device: { ...device, alias: "roof board" }, service }, { status });
    expect(p.startsWith("Fix the roof board on this node.")).toBe(true);
    expect(p).toContain('"plain_name":"LoRa radio"');
  });
  it("bounds and sanitizes log lines", () => {
    const lines = Array.from({ length: 50 }, (_, i) => `line ${i}\u0007‮`);
    const b = boundLog(lines);
    expect(b.length).toBeLessThanOrEqual(20);
    expect(b[b.length - 1]).toBe("line 49");
    expect(sanitizeLine("a\u0000b\rc")).toBe("abc");
    const p = buildPrompt("check", { kind: "node" }, { status, logLines: lines });
    expect(p).toContain("- last log lines:");
    expect(p).not.toContain("‮");
  });
});

describe("flash offers in the device context", () => {
  const offer = {
    reason: "blank",
    image: { family: "rnode", version: "1.78", file: "share/firmware/rnode-heltec-v3-1.78.bin", sha256: "f87a2e8af657f07642eee8206b15f83e295c58528c92e497b5d3b5ec8c84ed9c", board: "Heltec WiFi LoRa 32 V3" },
    erases: "everything on the board, including any Meshtastic settings",
  } as const;

  it("a board with an offer carries it, in full and as data", () => {
    const p = buildPrompt("setup", { kind: "device", device: { ...device, flash_offer: offer } }, { status });
    expect(p).toContain('"flash_offer":{"reason":"blank","image":{"family":"rnode","version":"1.78","board":"Heltec WiFi LoRa 32 V3","file":"share/firmware/rnode-heltec-v3-1.78.bin","sha256":"f87a2e8a');
    expect(p).toContain('"erases":"everything on the board, including any Meshtastic settings"');
  });
  it("a board without one gains nothing: the prompt is exactly what it was", () => {
    const plain = buildPrompt("setup", { kind: "device", device }, { status });
    expect(plain).not.toContain("flash_offer");
    expect(buildPrompt("setup", { kind: "device", device: { ...device, flash_offer: null } }, { status })).toBe(plain);
    expect(buildPrompt("connect", { kind: "registry", devices: [{ ...device, flash_offer: null }], services: [] }, { status })).not.toContain("flash_offer");
  });
  it("the registry prompt carries the offer of the board that has one", () => {
    const p = buildPrompt("connect", { kind: "registry", devices: [{ ...device, flash_offer: offer }, { ...device, id: "usb-2" }], services: [] }, { status });
    expect(p.match(/flash_offer/g)).toHaveLength(1);
  });
  it("strings are cleaned and cut, and the whole subject stays bounded", () => {
    const evil = { ...offer, erases: `all\u0007${"x".repeat(5000)}`, image: { ...offer.image, file: "f".repeat(5000), board: "b\u202e".repeat(500) } };
    const p = buildPrompt("setup", { kind: "device", device: { ...device, flash_offer: evil } }, { status });
    expect(p).not.toMatch(/[\u0007\u202e]/);
    expect(p).not.toContain("x".repeat(301));
    expect(p).not.toContain("f".repeat(161));
    const subject = p.split("\n").find((l) => l.startsWith("- subject:"))!;
    expect(new TextEncoder().encode(subject).length).toBeLessThanOrEqual(4096 + 64);
  });
});

describe("one goal per prompt", () => {
  const goals = ["Set up the", "Fix the", "Look at ", "Check this node end to end"];
  it("context emits the context block only, with no goal sentence", () => {
    const p = buildPrompt("context", { kind: "node" }, { status });
    expect(p.startsWith("Context from the dashboard")).toBe(true);
    for (const g of goals) expect(p).not.toContain(g);
    expect(p).toContain("category Node2B");
  });
  it("a hint still comes first in a context-only prompt", () => {
    expect(buildPrompt("context", { kind: "node" }, { status, hint: "the roof board" }).startsWith("Hint from the user: the roof board")).toBe(true);
  });
  it("questionPrompt puts the question over the context and carries exactly one goal", () => {
    const p = questionPrompt("Who is nearby?", { kind: "node" }, { status });
    expect(p.startsWith("Who is nearby?\n\nContext from the dashboard")).toBe(true);
    for (const g of goals) expect(p).not.toContain(g);
  });
  it("the four goals still lead with their sentence", () => {
    expect(buildPrompt("check", { kind: "node" }, { status }).startsWith("Check this node end to end")).toBe(true);
  });
});

describe("bounding by bytes", () => {
  it("never cuts a surrogate pair in half", () => {
    // each emoji is 4 UTF-8 bytes and two UTF-16 units; a cut at 6 bytes falls inside the second one
    const s = "😀😀😀";
    const cut = truncateUtf8(s, 6);
    expect(cut).toBe("😀");
    expect(new TextEncoder().encode(cut).length).toBeLessThanOrEqual(6);
    expect(new TextDecoder().decode(new TextEncoder().encode(cut))).toBe(cut); // a lone surrogate would come back as U+FFFD
    expect(truncateUtf8(s, 8)).toBe("😀😀");
    expect(truncateUtf8(s, 3)).toBe("");
    expect(truncateUtf8("abc", 3)).toBe("abc");
  });
  it("boundJson truncates with an emoji at the boundary and stays well-formed", () => {
    const out = boundJson({ t: "a😀😀😀😀" }, 14);
    expect(out.endsWith("…(truncated)")).toBe(true);
    const body = out.slice(0, -"…(truncated)".length);
    expect(new TextEncoder().encode(body).length).toBeLessThanOrEqual(14);
    expect(new TextDecoder("utf-8", { fatal: true }).decode(new TextEncoder().encode(body))).toBe(body); // valid UTF-16 -> UTF-8 round trip
    expect(body).not.toContain("\ufffd");
  });
  it("boundJson leaves small values alone", () => {
    expect(boundJson({ a: 1 }, 100)).toBe('{"a":1}');
  });
});

describe("hand-off", () => {
  it("stages and takes once by id", () => {
    const storage = mem();
    const id = stagePrompt("hello", "LoRa radio", "setup", { storage, now: () => 1, id: () => "abc" });
    expect(id).toBe("abc");
    expect(takePrompt("zzz", { storage })).toBeNull();
    const got = takePrompt("abc", { storage });
    expect(got?.text).toBe("hello");
    expect(takePrompt("abc", { storage })).toBeNull();
  });
});
