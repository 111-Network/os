import { describe, expect, it } from "vitest";
import { boundLog, buildPrompt, sanitizeLine, stagePrompt, takePrompt, type StorageLike } from "../src/lib/prompts";
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
