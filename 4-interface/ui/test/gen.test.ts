import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  CHAT_EVENT_NAMES,
  COMPONENT_STARTS,
  CONSENT_ACTIONS,
  DATASET_KINDS,
  DATASET_LOCATIONS,
  DATASET_STATES,
  DATASET_USED_BY,
  DEVICE_KINDS,
  DEVICE_STATES,
  DEVICE_TRANSPORTS,
  EVENTS_EVENT_NAMES,
  EVIDENCE_TRANSPORTS,
  EXPORT_OFFER_CLASSES,
  JOB_STATES,
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
  STORAGE_CLASS_IDS,
  STORAGE_INTEGRITY_STATES,
  STORAGE_KEEP_RULES,
  STORAGE_NODE_STATES,
  STORAGE_NOTICE_KINDS,
  STORAGE_STORES,
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

  it("Layer 8 enums, part 1 (the tuples gen-sse.mjs lists in ENUMS)", () => {
    const at = (name: string, ...path: string[]) => path.reduce((n: unknown, k) => (n as Record<string, unknown>)[k], schemas[name]!) as { enum: unknown[] };
    expect([...STORAGE_CLASS_IDS]).toEqual(schemas.StorageClassId!.enum);
    expect([...STORAGE_STORES]).toEqual(schemas.StorageStore!.enum);
    expect([...STORAGE_NOTICE_KINDS]).toEqual(at("StorageNotice", "properties", "kind").enum);
    expect([...STORAGE_KEEP_RULES]).toEqual(at("StorageKeep", "properties", "rule").enum);
    expect([...STORAGE_INTEGRITY_STATES]).toEqual(schemas.StorageIntegrityState!.enum);
    expect([...STORAGE_NODE_STATES]).toEqual(at("StorageNodeState", "properties", "state").enum);
    expect([...EXPORT_OFFER_CLASSES]).toEqual(at("StorageExportOfferRequest", "properties", "class").enum);
  });

  it("the Layer 8 `storage` event is in the map and the tuple", () => {
    expect(sse.events.storage).toBe("SseStorage");
    expect([...EVENTS_EVENT_NAMES]).toContain("storage");
  });

  it("Layer 8 enums, part 2 (the dataset tuples gen-sse.mjs lists in ENUMS)", () => {
    expect([...DATASET_KINDS]).toEqual(schemas.DatasetKind!.enum);
    expect([...DATASET_STATES]).toEqual(["not_held", "partial", "present", "unverified", "disk_absent"]);
    expect([...DATASET_STATES]).toEqual(schemas.DatasetState!.enum);
    expect([...DATASET_LOCATIONS]).toEqual(["origin", "in_place", null]);
    expect([...DATASET_USED_BY]).toEqual(["map", "encyclopedia", "agent", "updates"]);
    const jobState = schemas.JobState as { anyOf: { enum?: unknown[]; pattern?: string }[] };
    expect([...JOB_STATES]).toEqual(jobState.anyOf[0]!.enum);
    // the closed part holds every state of §7.4, with `paused (metered link)` and `failed (source cannot resume)` among them
    expect([...JOB_STATES]).toEqual(expect.arrayContaining(["queued", "running", "paused (offline)", "paused (metered cap)", "paused (metered link)", "paused (disk low)", "paused (source busy)", "done", "failed (name mismatch)", "failed (source cannot resume)", "cancelled"]));
    expect([...COMPONENT_STARTS]).toEqual(["always", "on_demand"]);
  });

  it("the Layer 8 `dataset` event is in the map and the tuple, after `storage`", () => {
    expect(sse.events.dataset).toBe("SseDataset");
    expect([...EVENTS_EVENT_NAMES].slice(-2)).toEqual(["storage", "dataset"]);
  });
});
