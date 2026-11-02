import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  CHAT_EVENT_NAMES,
  CONSENT_ACTIONS,
  DEVICE_KINDS,
  DEVICE_STATES,
  DEVICE_TRANSPORTS,
  EVENTS_EVENT_NAMES,
  EVIDENCE_TRANSPORTS,
  NEARBY_KINDS,
  PAIR_ROLES,
  PAIR_TLS,
  PAIR_VIAS,
  PROBLEM_CODES,
  PROPOSAL_KINDS,
  REGISTRY_ANSWERS,
  RELAY_GATEWAY_STATES,
  SERVICE_IDS,
  SERVICE_STATES,
} from "../src/lib/api/types";
import { schemas, sse } from "./contract.gen";

const root = resolve(__dirname, "..");

describe("generated types are fresh", () => {
  it("sse.gen.ts and contract.gen.ts match api/openapi.yaml (gen-sse.mjs --check)", () => {
    execFileSync(process.execPath, [join(root, "scripts/gen-sse.mjs"), "--check"], { cwd: root, stdio: "pipe" });
  });

  it("types.gen.ts matches a fresh openapi-typescript run", () => {
    const bin = join(root, "node_modules/.bin/openapi-typescript");
    if (!existsSync(bin)) return;
    const dir = mkdtempSync(join(tmpdir(), "oas-"));
    try {
      const out = join(dir, "types.gen.ts");
      execFileSync(bin, [resolve(root, "../api/openapi.yaml"), "-o", out], { cwd: root, stdio: "pipe" });
      expect(readFileSync(out, "utf8")).toBe(readFileSync(join(root, "src/lib/api/types.gen.ts"), "utf8"));
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  }, 30_000);
});

describe("runtime tuples mirror the contract", () => {
  it("event names include the devices frame", () => {
    expect([...EVENTS_EVENT_NAMES]).toContain("devices");
  });

  it("event names", () => {
    expect([...EVENTS_EVENT_NAMES]).toEqual(Object.keys(sse.events));
    expect([...CHAT_EVENT_NAMES]).toEqual(Object.keys(sse.chat));
  });

  it("enums", () => {
    expect([...SERVICE_IDS]).toEqual(schemas.ServiceId!.enum);
    expect([...SERVICE_STATES]).toEqual(schemas.ServiceState!.enum);
    expect([...DEVICE_KINDS]).toEqual(schemas.DeviceKind!.enum);
    expect([...DEVICE_STATES]).toEqual(schemas.DeviceState!.enum);
    expect([...DEVICE_TRANSPORTS]).toEqual(schemas.DeviceTransport!.enum);
    expect([...PROBLEM_CODES]).toEqual((schemas.Problem!.properties as { code: { enum: string[] } }).code.enum);
  });

  it("Layer 7 enums (the tuples gen-sse.mjs lists in ENUMS)", () => {
    const at = (name: string, ...path: string[]) => path.reduce((n: unknown, k) => (n as Record<string, unknown>)[k], schemas[name]!) as { enum: string[] };
    expect([...CONSENT_ACTIONS]).toEqual(at("ConsentRequired", "properties", "action").enum);
    expect([...NEARBY_KINDS]).toEqual(at("NearbySource", "properties", "kind").enum);
    expect([...PROPOSAL_KINDS]).toEqual(at("Proposal", "properties", "kind").enum);
    expect([...RELAY_GATEWAY_STATES]).toEqual(schemas.RelayGatewayState!.enum);
    expect([...PAIR_ROLES]).toEqual(schemas.PairRole!.enum);
    expect([...PAIR_VIAS]).toEqual(schemas.PairVia!.enum);
    expect([...PAIR_TLS]).toEqual(schemas.PairTls!.enum);
    expect([...REGISTRY_ANSWERS]).toEqual(schemas.RegistryAnswer!.enum);
    expect([...EVIDENCE_TRANSPORTS]).toEqual(schemas.NearbyEvidenceTransport!.enum);
  });

  it("the Layer 7 events are in the map and the tuple", () => {
    expect(sse.events.relay).toBe("SseRelay");
    expect(sse.events.registry).toBe("SseRegistry");
    expect([...EVENTS_EVENT_NAMES]).toContain("relay");
    expect([...EVENTS_EVENT_NAMES]).toContain("registry");
  });
});
