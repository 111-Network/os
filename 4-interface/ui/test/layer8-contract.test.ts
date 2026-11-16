// Layer 8 contract pins, part 1 (8-storage/README.md §16.1-§16.4, §16.6, task C1): the closed enums, the enum growth on existing
// schemas, the SSE map, the operations, fixtures for the new schemas validated against api/openapi.yaml, the storage adapter's mock, the
// store's handling of the `storage` event, the `storage` proposal card and the consent dialog's `facts`. Fixtures are hand-written here,
// not daemon output. The Layer 7 precedent is test/layer7-contract.test.ts; the dataset shapes are the contract's second part (C2).
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Adapter, EventsFrameWithId } from "../src/lib/api/adapter";
import { getStorage, resetAdapterForTests } from "../src/lib/api/index";
import { createMockAdapter, type MockAdapter } from "../src/lib/api/mock";
import {
  buildFixtures,
  buildStorageStatus,
} from "../src/lib/api/mock/fixtures";
import { buildFabric } from "../src/lib/api/mock/messaging";
import {
  componentsServeText,
  createStorageMock,
  datasetsServeText,
  startEmptyText,
  updateWithoutCopyText,
} from "../src/lib/api/mock/storage";
import { sha256Hex } from "../src/lib/api/mock/sha256";
import { ApiError } from "../src/lib/api/problem";
import { type StorageAdapter } from "../src/lib/api/storage";
import { ApiStore, mergeStorageFrame } from "../src/lib/api/store";
import {
  CONSENT_ACTIONS,
  EVENTS_EVENT_NAMES,
  EXPORT_OFFER_CLASSES,
  PROBLEM_CODES,
  PROPOSAL_KINDS,
  STORAGE_CLASS_IDS,
  STORAGE_INTEGRITY_STATES,
  STORAGE_KEEP_RULES,
  STORAGE_NODE_STATES,
  STORAGE_NOTICE_KINDS,
  STORAGE_STORES,
  type EventsFrame,
  type StorageStatus,
} from "../src/lib/api/types";
import {
  ConsentPanel,
  factsLine,
  runConsent,
} from "../src/views/messages/net/consent";
import {
  ProposalBody,
  parseStorageBody,
  storageActionSentence,
} from "../src/views/agent/proposal-body";
import {
  acceptRunsConsent,
  appliedLine,
  ProposalCards,
} from "../src/views/agent/proposals";
import { applyingProposalsOf } from "../src/lib/api/store";
import { operations, schemas, sse } from "./contract.gen";
import { FakeClock, flush } from "./helpers/fakes";
import { validateAgainst } from "./helpers/validate";

const HASH = "a".repeat(64);
const NODE = "ab".repeat(32);
const T0 = Date.parse("2026-11-16T09:00:00Z");

/** The §16.2 example, completed (the page's other classes), with the three notices and a walked figure. */
const STATUS: StorageStatus = (() => {
  const s = buildStorageStatus();
  s.node_state = {
    state: "unavailable",
    reason: "its file was damaged and set aside",
    file_bytes: null,
  };
  s.integrity = { fabric: "ok", node_state: "failed" };
  s.disk.origin = {
    free: 900_000_000_000,
    total: 1_000_000_000_000,
    floor: 10_000_000_000,
    disk_low: false,
  };
  s.notices = [
    { kind: "unreadable_agent_files", count: 2 },
    {
      kind: "update_refused_space",
      release: "2026.11.2",
      store: "fabric",
      needed_bytes: 500_000_000,
    },
    { kind: "set_aside_store", file: "state.db.damaged-1", bytes: 4_194_304 },
  ];
  const agent = s.classes.find((c) => c.id === "agent")!;
  agent.full = true;
  agent.export_offer_needed = true;
  agent.beside = {
    spool: { bytes: 0, at_least: false },
    work: { bytes: 1_200_000, at_least: true },
    legacy_v0: { bytes: 0, at_least: false },
  };
  s.classes.find((c) => c.id === "kb")!.at_least = true;
  return s;
})();

describe("Layer 8 enums (the pins an enum bump touches)", () => {
  it("Proposal.kind is 10 with `storage` last", () => {
    expect([...PROPOSAL_KINDS]).toEqual([
      "diagnose",
      "propose",
      "patch",
      "chat",
      "message",
      "bridge_message",
      "bridge_edge",
      "device",
      "relay",
      "storage",
    ]);
  });
  it("Problem.code appends exactly the five codes after Layer 7's; none already existed", () => {
    expect([...PROBLEM_CODES].slice(27)).toEqual([
      "insufficient_space",
      "untrusted_publisher",
      "job_running",
      "store_unavailable",
      "range_not_satisfiable",
    ]);
    expect(new Set(PROBLEM_CODES).size).toBe(PROBLEM_CODES.length);
  });
  it("ConsentRequired.action appends the six after Layer 7's nine, in order", () => {
    expect([...CONSENT_ACTIONS].slice(9)).toEqual([
      "dataset_fetch",
      "dataset_override",
      "datasets_serve",
      "components_serve",
      "update_without_copy",
      "store_start_empty",
    ]);
    expect([...CONSENT_ACTIONS].slice(0, 5)).toEqual([
      "edge_enable",
      "edge_resume",
      "component_enable",
      "ham_confirm",
      "uplink_send",
    ]);
  });
  it("the storage classes are the closed set of §4.1, in its order", () => {
    expect([...STORAGE_CLASS_IDS]).toEqual([
      "fabric",
      "agent",
      "heal",
      "logs",
      "datasets",
      "updates",
      "components",
      "bridge",
      "kb",
    ]);
    expect(STATUS.classes.map((c) => c.id)).toEqual([...STORAGE_CLASS_IDS]);
  });
  it("stores, notice kinds, keep rules, node-state states and the offer classes", () => {
    expect([...STORAGE_STORES]).toEqual(["fabric", "node_state"]);
    expect([...STORAGE_NOTICE_KINDS]).toEqual([
      "unreadable_agent_files",
      "update_refused_space",
      "set_aside_store",
    ]);
    expect([...STORAGE_KEEP_RULES]).toEqual(["termux", "hub", "linux", "user"]);
    expect([...STORAGE_NODE_STATES]).toEqual(["open", "closed", "unavailable"]);
    expect([...STORAGE_INTEGRITY_STATES]).toEqual([
      "checking",
      "ok",
      "failed",
      null,
    ]);
    expect([...EXPORT_OFFER_CLASSES]).toEqual(["fabric", "agent", "heal"]);
  });
  it("Status.update.last_result gained `refused_space`, appended before null", () => {
    const last = (
      schemas.Status!.properties as Record<
        string,
        { properties: { last_result: { enum: unknown[] } } }
      >
    ).update!.properties.last_result.enum;
    expect(last).toEqual([
      "up_to_date",
      "staged",
      "refused",
      "degraded",
      "refused_space",
      null,
    ]);
  });
  it("the SSE map appends `storage` after Layer 7's `registry`; the earlier fifteen are untouched", () => {
    expect(sse.events.storage).toBe("SseStorage");
    expect([...EVENTS_EVENT_NAMES].slice(13)).toEqual([
      "relay",
      "registry",
      "storage",
    ]);
    expect([...EVENTS_EVENT_NAMES]).toHaveLength(16);
  });
});

describe("Layer 8 fixtures validate against the contract", () => {
  const settings = {
    keep_bytes: null,
    keep_default: 2_147_483_648,
    caps: { agent: null, heal: 1_000_000 },
    logs: { max_file_bytes: 4_194_304 },
    follow: [NODE],
    sources: { [NODE]: ["https://mirror.example.org/"] },
    origin: {
      datasets: false,
      components: true,
      termux: false,
      max_streams: 4,
    },
  };
  it.each([
    ["StorageStatus", buildStorageStatus()],
    ["StorageStatus", STATUS],
    ["StorageStatus", { ...STATUS, settings }],
    [
      "SseStorage",
      {
        ...buildStorageStatus(),
        classes: buildStorageStatus().classes.slice(0, 4),
      },
    ],
    ["StorageSettings", settings],
    [
      "StorageSettingsPatch",
      {
        keep_bytes: null,
        caps: { agent: 1, heal: null },
        logs: { max_file_bytes: 1 },
        follow: [],
        sources: {},
        origin: { datasets: true, max_streams: 16 },
        accept_warning: HASH,
      },
    ],
    ["StorageExportOfferRequest", { class: "fabric" }],
    [
      "UpdateWithoutCopyRequest",
      { release: "2026.11.2", store: "node_state", accept_warning: HASH },
    ],
    ["StartEmptyRequest", { store: "fabric" }],
    [
      "StorageNotice",
      {
        kind: "update_refused_space",
        release: "2026.11.2",
        store: "node_state",
        needed_bytes: 9,
      },
    ],
    [
      "StorageClass",
      {
        id: "agent",
        layer: 3,
        bytes: 0,
        cap: null,
        budgeted: true,
        export_offer_needed: false,
        full: false,
        at_least: false,
        beside: {
          spool: { bytes: 0, at_least: false },
          work: { bytes: 0, at_least: false },
          legacy_v0: { bytes: 0, at_least: false },
        },
      },
    ],
    [
      "ConsentRequired",
      {
        action: "dataset_fetch",
        subject: "abcd-efgh-ij:wikipedia@2026.10",
        warning_text: "t",
        hash: HASH,
        addons: ["metered", "non_commercial"],
        facts: { free: 5, free_after: 3, days: 2 },
      },
    ],
    [
      "ConsentRequired",
      {
        action: "dataset_override",
        subject: "abcd-efgh-ij:wikipedia@2026.10",
        warning_text: "t",
        hash: HASH,
        addons: [],
        facts: { remaining: 100 },
      },
    ],
    [
      "ConsentRequired",
      {
        action: "update_without_copy",
        subject: "2026.11.2:fabric",
        warning_text: "t",
        hash: HASH,
        addons: [],
        facts: { needed: 5, free: 3 },
      },
    ],
    [
      "ConsentRequired",
      {
        action: "datasets_serve",
        subject: "abcd-efgh-ij",
        warning_text: "t",
        hash: HASH,
        addons: [],
      },
    ],
    [
      "ConsentRequired",
      {
        action: "components_serve",
        subject: "abcd-efgh-ij",
        warning_text: "t",
        hash: HASH,
        addons: ["termux"],
      },
    ],
    [
      "ConsentRequired",
      {
        action: "store_start_empty",
        subject: "node_state",
        warning_text: "t",
        hash: HASH,
        addons: [],
      },
    ],
    [
      "Problem",
      {
        type: "about:blank",
        title: "No",
        status: 409,
        code: "insufficient_space",
      },
    ],
    [
      "Problem",
      {
        type: "about:blank",
        title: "No",
        status: 409,
        code: "untrusted_publisher",
      },
    ],
    [
      "Problem",
      { type: "about:blank", title: "No", status: 409, code: "job_running" },
    ],
    [
      "Problem",
      {
        type: "about:blank",
        title: "No",
        status: 503,
        code: "store_unavailable",
      },
    ],
    [
      "Problem",
      {
        type: "about:blank",
        title: "No",
        status: 416,
        code: "range_not_satisfiable",
      },
    ],
    [
      "Proposal",
      {
        schema: 1,
        id: "prop-1",
        node_id: null,
        task_id: null,
        kind: "storage",
        title: "t",
        body: '{"why":"w","actions":[{"action":"quota.set","class":"keep","bytes":null}]}',
        created_at: "2026-11-16T09:00:00Z",
        decision: null,
      },
    ],
    [
      "Proposal",
      {
        schema: 1,
        id: "prop-1",
        node_id: null,
        task_id: null,
        kind: "storage",
        title: "t",
        body: "{}",
        created_at: "2026-11-16T09:00:00Z",
        decision: {
          decision: "accepted",
          at: "2026-11-16T09:01:00Z",
          applied: { pending: true },
        },
      },
    ],
    [
      "Proposal",
      {
        schema: 1,
        id: "prop-1",
        node_id: null,
        task_id: null,
        kind: "propose",
        title: "t",
        body: "b",
        created_at: "2026-11-16T09:00:00Z",
        decision: { decision: "dismissed", at: "2026-11-16T09:01:00Z" },
      },
    ],
    [
      "FabricStatus",
      {
        ...buildFabric(),
        unconsumed: { agent_op: 0, kb: 1, update: 0, patch: 0, sync: 2 },
      },
    ],
    ["FabricStatus", buildFabric()],
  ])("%s", (name, value) => {
    expect(validateAgainst(name, value)).toEqual([]);
  });

  it.each([
    [
      "a class outside the closed set",
      "StorageClass",
      { ...buildStorageStatus().classes[0], id: "downloads" },
    ],
    [
      "a node-state state outside the three",
      "StorageNodeState",
      { state: "weird", reason: null, file_bytes: 0 },
    ],
    [
      "an integrity state outside the three",
      "StorageIntegrity",
      { fabric: "fine", node_state: null },
    ],
    [
      "a keep rule outside the four",
      "StorageKeep",
      { bytes: 1, default: 1, effective: 1, rule: "owner" },
    ],
    [
      "a notice kind outside the three",
      "StorageNotice",
      { kind: "free_money" },
    ],
    [
      "a status without the disk block",
      "StorageStatus",
      (({ disk: _d, ...rest }) => rest)(buildStorageStatus()),
    ],
    [
      "a negative byte count",
      "StorageDisk",
      { free: -1, total: 1, floor: 1, disk_low: false, origin: null },
    ],
    [
      "a stream cap past 16",
      "StorageSettingsPatch",
      { origin: { max_streams: 17 } },
    ],
    [
      "a stream cap of 0",
      "StorageSettingsPatch",
      { origin: { max_streams: 0 } },
    ],
    [
      "an export offer for the logs",
      "StorageExportOfferRequest",
      { class: "logs" },
    ],
    [
      "an update consent for no store",
      "UpdateWithoutCopyRequest",
      { release: "2026.11.2" },
    ],
    [
      "a start-empty for a made-up store",
      "StartEmptyRequest",
      { store: "datasets" },
    ],
    [
      "a follow entry that is not a NodeId",
      "StorageSettingsPatch",
      { follow: ["abc"] },
    ],
    [
      "facts with a fraction of a byte",
      "ConsentRequired",
      {
        action: "dataset_override",
        subject: "x",
        warning_text: "t",
        hash: HASH,
        addons: [],
        facts: { remaining: 1.5 },
      },
    ],
    ["a walk without at_least", "StorageWalk", { bytes: 1 }],
    [
      "a proposal kind outside the ten",
      "Proposal",
      {
        schema: 1,
        id: "p",
        node_id: null,
        task_id: null,
        kind: "format",
        title: "t",
        body: "b",
        created_at: "2026-11-16T09:00:00Z",
        decision: null,
      },
    ],
    [
      "a consent action outside the fifteen",
      "ConsentRequired",
      {
        action: "free_money",
        subject: "x",
        warning_text: "t",
        hash: HASH,
        addons: [],
      },
    ],
  ])("rejects %s", (_why, name, value) => {
    expect(validateAgainst(name as string, value)).not.toEqual([]);
  });

  it("no storage response schema carries a consent hash, a token or a secret, and `facts` never joins `hash`", () => {
    const walk = (node: unknown, path: string, out: string[]): void => {
      if (typeof node !== "object" || node === null) return;
      for (const [k, v] of Object.entries(node)) {
        if (
          ["hash", "token", "secret", "password", "accept_warning"].includes(
            k,
          ) &&
          path.endsWith(".properties")
        )
          out.push(`${path}.${k}`);
        walk(v, `${path}.${k}`, out);
      }
    };
    const bad: string[] = [];
    for (const name of [
      "StorageStatus",
      "SseStorage",
      "StorageSettings",
      "StorageClass",
      "StorageNotice",
      "StorageDisk",
      "StorageKeep",
    ])
      walk(schemas[name], name, bad);
    expect(bad).toEqual([]);
    const facts = (
      schemas.ConsentRequired!.properties as Record<
        string,
        { properties: Record<string, unknown> }
      >
    ).facts!;
    expect(Object.keys(facts.properties).sort()).toEqual([
      "days",
      "free",
      "free_after",
      "needed",
      "remaining",
    ]);
    expect(
      String(
        (
          schemas.ConsentRequired!.properties as Record<
            string,
            { description: string }
          >
        ).facts!.description,
      ),
    ).toContain("never part of `hash`");
  });
});

describe("Layer 8 operations (part 1)", () => {
  const want: Record<string, [string, string, number]> = {
    getStorage: ["GET", "/api/v1/storage", 200],
    patchStorageSettings: ["PATCH", "/api/v1/storage/settings", 200],
    answerStorageExportOffer: ["POST", "/api/v1/storage/export-offer", 204],
    acceptUpdateWithoutCopy: [
      "POST",
      "/api/v1/storage/update-without-copy",
      204,
    ],
    deleteStorageSetAside: ["DELETE", "/api/v1/storage/set-aside/{file}", 204],
    startEmptyStore: ["POST", "/api/v1/storage/start-empty", 204],
  };
  it("are all in the contract with the expected verbs, paths and success codes", () => {
    for (const [id, [method, path, success]] of Object.entries(want))
      expect(
        [operations[id]?.method, operations[id]?.path, operations[id]?.success],
        id,
      ).toEqual([method, path, success]);
  });
  it("the loopback-only writes document their 403, the consent gates their 409", () => {
    for (const id of [
      "patchStorageSettings",
      "deleteStorageSetAside",
      "startEmptyStore",
    ])
      expect(operations[id]!.errors, id).toContain(403);
    for (const id of [
      "patchStorageSettings",
      "acceptUpdateWithoutCopy",
      "startEmptyStore",
    ])
      expect(operations[id]!.errors, id).toContain(409);
    expect(operations.deleteStorageSetAside!.errors).toContain(404);
    expect(operations.acceptUpdateWithoutCopy!.errors).not.toContain(403); // a dashboard session or loopback (§16.1)
    expect(operations.decideProposal!.errors).toEqual(
      expect.arrayContaining([403, 503]),
    );
    expect(operations.chatHistory!.errors).toContain(503);
  });
  it("only `getStorage` is a read: every other Layer 8 operation is a write, so the global gate refuses the agent's bearer on it", () => {
    for (const id of Object.keys(want))
      expect(operations[id]!.method === "GET", id).toBe(id === "getStorage");
  });
  it("the file parameter is one path segment named `file`", () => {
    expect(operations.deleteStorageSetAside!.path).toContain("{file}");
  });
});

// --- the mock and the storage adapter ----------------------------------------------------------------------------------------------

const made: MockAdapter[] = [];
afterEach(() => {
  for (const a of made.splice(0)) a.mock.dispose();
  vi.unstubAllGlobals();
  resetAdapterForTests();
});

const fail = async (
  p: Promise<unknown> | (() => unknown),
): Promise<ApiError> => {
  const e = await (
    typeof p === "function" ? Promise.resolve().then(p) : p
  ).then(
    () => null,
    (x: unknown) => x,
  );
  expect(e).toBeInstanceOf(ApiError);
  return e as ApiError;
};

function direct(
  over: {
    loopback?: boolean;
    hub?: boolean;
    lan?: boolean;
    termuxPins?: boolean;
  } = {},
) {
  const frames: EventsFrame[] = [];
  const state = {
    loopback: over.loopback ?? true,
    hub: over.hub ?? true,
    lan: over.lan ?? true,
  };
  const m = createStorageMock(
    {
      publish: (f) => frames.push(f) as unknown as number,
      loopback: () => state.loopback,
      hub: () => state.hub,
      lan: () => state.lan,
      nodeId: () => NODE,
    },
    { termuxPins: over.termuxPins },
  );
  return { m, frames, state };
}
const consentOfErr = (e: ApiError) => e.problem.consent!;

describe("the Layer 8 mock answers the contract", () => {
  it("the status validates, carries the settings beside the figures and no consent hash", () => {
    const { m } = direct();
    const s = m.status();
    expect(validateAgainst("StorageStatus", s)).toEqual([]);
    expect(s.settings!.origin).toEqual({
      datasets: false,
      components: false,
      termux: false,
      max_streams: 4,
    });
    expect(JSON.stringify(s)).not.toMatch(/accept_warning|"hash"/);
    expect(s.classes.map((c) => c.id)).toEqual([...STORAGE_CLASS_IDS]);
  });

  it("the budget moves the caps (§4.2 item 3), `keep.effective` is their sum, and each change sends one `storage` frame of the four budgeted classes", () => {
    const { m, frames } = direct();
    const before = m.status();
    expect(before.keep.rule).toBe("hub");
    m.patchSettings({ keep_bytes: 4 * 1024 ** 3 });
    const after = m.status();
    expect(after.keep).toMatchObject({ bytes: 4 * 1024 ** 3, rule: "user" });
    const cap = (id: string) => after.classes.find((c) => c.id === id)!.cap!;
    expect(after.keep.effective).toBe(
      cap("fabric") + cap("agent") + cap("heal") + cap("logs"),
    );
    expect(cap("fabric")).toBeGreaterThan(
      before.classes.find((c) => c.id === "fabric")!.cap!,
    );
    expect(frames).toHaveLength(1);
    const f = frames[0] as Extract<EventsFrame, { event: "storage" }>;
    expect(f.data.classes.map((c) => c.id)).toEqual([
      "fabric",
      "agent",
      "heal",
      "logs",
    ]);
    expect(f.data.settings).toBeUndefined();
    expect(validateAgainst(sse.events.storage!, f.data)).toEqual([]);
    m.patchSettings({ caps: { agent: 123 } });
    expect(m.status().classes.find((c) => c.id === "agent")!.cap).toBe(123);
    m.patchSettings({ logs: { max_file_bytes: 1 }, follow: [NODE] });
    expect(frames).toHaveLength(2); // a change §16.3 does not list sends nothing
  });

  it("settings are loopback-only (403), validated (400) and unchanged by a refusal", async () => {
    const { m, state } = direct();
    state.loopback = false;
    expect((await fail(() => m.patchSettings({ keep_bytes: 1 }))).code).toBe(
      "loopback_only",
    );
    expect(
      (await fail(() => m.deleteSetAside("state.db.damaged-1"))).code,
    ).toBe("loopback_only");
    expect((await fail(() => m.startEmpty("fabric"))).code).toBe(
      "loopback_only",
    );
    state.loopback = true;
    for (const bad of [
      { keep_bytes: -1 },
      { keep_bytes: 1.5 },
      { caps: { heal: -5 } },
      { logs: { max_file_bytes: 0 } },
      { follow: ["nope"] },
      { sources: { nope: ["https://x"] } },
      { sources: { [NODE]: [""] } },
      { origin: { max_streams: 0 } },
      { origin: { max_streams: 17 } },
      { origin: { termux: true, components: true } },
    ]) {
      expect(
        (await fail(() => m.patchSettings(bad as never))).code,
        JSON.stringify(bad),
      ).toBe("bad_request");
    }
    expect(m.status().settings).toEqual(direct().m.status().settings);
  });

  it("`origin.datasets` and `origin.components` are consent round-trips with the spec's text; a stale or foreign hash is refused again", async () => {
    const { m, state } = direct();
    const e = await fail(() => m.patchSettings({ origin: { datasets: true } }));
    expect(e.status).toBe(409);
    expect(consentOfErr(e)).toMatchObject({
      action: "datasets_serve",
      warning_text: datasetsServeText(),
      hash: sha256Hex(datasetsServeText()),
      addons: [],
    });
    expect(
      (await fail(() => m.patchSettings({ origin: { datasets: true } }, HASH)))
        .code,
    ).toBe("consent_required");
    expect(m.status().settings!.origin.datasets).toBe(false);
    expect(
      m.patchSettings(
        { origin: { datasets: true } },
        sha256Hex(datasetsServeText()),
      ).origin.datasets,
    ).toBe(true);
    // turning it off asks nothing, and on again is on record under the same text
    expect(
      m.patchSettings({ origin: { datasets: false } }).origin.datasets,
    ).toBe(false);
    expect(
      m.patchSettings({ origin: { datasets: true } }).origin.datasets,
    ).toBe(true);
    const c = await fail(() =>
      m.patchSettings({ origin: { components: true } }),
    );
    expect(consentOfErr(c)).toMatchObject({
      action: "components_serve",
      warning_text: componentsServeText(false),
      addons: [],
    });
    expect(
      m.patchSettings(
        { origin: { components: true, max_streams: 8 } },
        sha256Hex(componentsServeText(false)),
      ).origin,
    ).toMatchObject({ components: true, max_streams: 8 });
    // a node without the hub role or without LAN sharing is refused before any text is shown
    state.hub = false;
    const d = direct({ hub: false });
    expect(
      (await fail(() => d.m.patchSettings({ origin: { datasets: true } })))
        .code,
    ).toBe("needs_hub");
    const l = direct({ lan: false });
    expect(
      (await fail(() => l.m.patchSettings({ origin: { components: true } })))
        .code,
    ).toBe("lan_off");
  });

  it("the Termux tick is not offered in v1 and, once pins are verified, asks under `components_serve` with the addon `termux`", async () => {
    const off = direct();
    expect(
      (await fail(() => off.m.patchSettings({ origin: { termux: true } })))
        .code,
    ).toBe("bad_request");
    const { m } = direct({ termuxPins: true });
    expect(
      (await fail(() => m.patchSettings({ origin: { termux: true } }))).code,
    ).toBe("bad_request"); // the tick sits under serving program files
    const e = await fail(() =>
      m.patchSettings({ origin: { components: true, termux: true } }),
    );
    expect(consentOfErr(e)).toMatchObject({
      action: "components_serve",
      addons: ["termux"],
      warning_text: componentsServeText(true),
    });
    expect(
      m.patchSettings(
        { origin: { components: true, termux: true } },
        sha256Hex(componentsServeText(true)),
      ).origin,
    ).toMatchObject({ components: true, termux: true });
    expect(
      m.patchSettings({ origin: { components: false } }).origin,
    ).toMatchObject({ components: false, termux: false });
  });

  it("export offers: a class answers once, an unknown class is 400, the agent's classes answer 503 while the store is unavailable", async () => {
    const { m, frames } = direct();
    const ctl = m.control;
    ctl.setExportOfferNeeded("agent", true, true);
    expect(m.status().classes.find((c) => c.id === "agent")).toMatchObject({
      export_offer_needed: true,
      full: true,
    });
    m.answerExportOffer("agent");
    expect(m.status().classes.find((c) => c.id === "agent")).toMatchObject({
      export_offer_needed: false,
      full: false,
    });
    const n = frames.length;
    m.answerExportOffer("agent"); // already answered: no change, no frame
    expect(frames).toHaveLength(n);
    expect((await fail(() => m.answerExportOffer("logs" as never))).code).toBe(
      "bad_request",
    );
    ctl.setNodeState("unavailable", "damaged");
    expect((await fail(() => m.answerExportOffer("heal"))).code).toBe(
      "store_unavailable",
    );
    m.answerExportOffer("fabric"); // the fabric's own store is separate
    ctl.setExportOfferNeeded("fabric", true, true);
    expect(m.status().classes.find((c) => c.id === "fabric")!.full).toBe(false); // `full` is for agent and heal only
  });

  it("an update without a rollback copy: 400 for a pair no notice names, then the consent with `facts` {needed, free}, then 204", async () => {
    const { m } = direct();
    expect(
      (await fail(() => m.acceptUpdateWithoutCopy("2026.11.2", "fabric"))).code,
    ).toBe("bad_request");
    m.control.refuseUpdateForSpace("2026.11.2", "fabric", 500_000_000);
    expect(
      (await fail(() => m.acceptUpdateWithoutCopy("2026.11.2", "node_state")))
        .code,
    ).toBe("bad_request");
    const e = await fail(() =>
      m.acceptUpdateWithoutCopy("2026.11.2", "fabric"),
    );
    expect(consentOfErr(e)).toMatchObject({
      action: "update_without_copy",
      subject: "2026.11.2:fabric",
      warning_text: updateWithoutCopyText("2026.11.2", "fabric"),
      facts: { needed: 500_000_000, free: 12_400_000_000 },
    });
    expect(consentOfErr(e).hash).toBe(sha256Hex(consentOfErr(e).warning_text));
    expect(validateAgainst("ConsentRequired", consentOfErr(e))).toEqual([]);
    expect(
      (await fail(() => m.acceptUpdateWithoutCopy("2026.11.2", "fabric", HASH)))
        .code,
    ).toBe("consent_required");
    expect(
      m.acceptUpdateWithoutCopy("2026.11.2", "fabric", consentOfErr(e).hash),
    ).toBeUndefined();
    expect(updateWithoutCopyText("2026.11.2", "node_state")).toContain(
      "the agent's history and repair records stay closed",
    );
    expect(updateWithoutCopyText("2026.11.2", "fabric")).toContain(
      "messages stay closed",
    );
  });

  it("a set-aside file is deleted by the name a notice gives; any other name is 404, a path never matches", async () => {
    const { m, frames } = direct();
    m.control.damageStore("node_state", "state.db.damaged-1", 4096);
    for (const name of [
      "state.db",
      "../state.db.damaged-1",
      "state.db.damaged-2",
      "",
      "state.db.damaged-1/../x",
      "x.damaged-1",
    ])
      expect((await fail(() => m.deleteSetAside(name))).code, name).toBe(
        "not_found",
      );
    const n = frames.length;
    expect(m.deleteSetAside("state.db.damaged-1")).toBeUndefined();
    expect(
      m.status().notices.filter((x) => x.kind === "set_aside_store"),
    ).toEqual([]);
    expect(frames.length).toBe(n + 1);
    expect(
      (await fail(() => m.deleteSetAside("state.db.damaged-1"))).code,
    ).toBe("not_found");
  });

  it("an empty store starts only for a closed one, after the consent that names the damaged file; the file stays", async () => {
    const { m } = direct();
    expect((await fail(() => m.startEmpty("node_state"))).code).toBe(
      "bad_request",
    );
    m.control.damageStore("node_state", "state.db.damaged-1", 4096);
    expect(m.status().node_state).toMatchObject({ state: "unavailable" });
    expect(m.status().integrity.node_state).toBe("failed");
    const e = await fail(() => m.startEmpty("node_state"));
    expect(consentOfErr(e)).toMatchObject({
      action: "store_start_empty",
      subject: "node_state",
      warning_text: startEmptyText("node_state", "state.db.damaged-1"),
    });
    expect((await fail(() => m.startEmpty("node_state", HASH))).code).toBe(
      "consent_required",
    );
    expect(m.startEmpty("node_state", consentOfErr(e).hash)).toBeUndefined();
    const s = m.status();
    expect(s.node_state.state).toBe("closed");
    expect(s.notices).toContainEqual({
      kind: "set_aside_store",
      file: "state.db.damaged-1",
      bytes: 4096,
    }); // only the person deletes it
    expect(
      (await fail(() => m.startEmpty("node_state", consentOfErr(e).hash))).code,
    ).toBe("bad_request"); // no longer closed
  });

  it("an open or a close of the node-state store on use sends no frame; a change to or from `unavailable` does", () => {
    const { m, frames } = direct();
    m.control.setNodeState("open");
    m.control.setNodeState("closed");
    expect(frames).toHaveLength(0);
    m.control.setNodeState(
      "unavailable",
      "its file is newer than this release",
    );
    expect(frames).toHaveLength(1);
    expect(m.status().node_state).toMatchObject({
      state: "unavailable",
      reason: "its file is newer than this release",
    });
  });

  it("every frame the storage mock publishes validates against its x-sse-events schema", () => {
    const { m, frames } = direct();
    m.control.setDiskLow(true);
    m.control.setIntegrity("fabric", "checking");
    m.control.setUnreadableAgentFiles(3);
    m.control.refuseUpdateForSpace("2026.11.2", "node_state", 1);
    m.control.damageStore("fabric", "store.db.damaged-1", 10);
    m.patchSettings({ keep_bytes: 3_000_000_000 });
    expect(frames.length).toBe(6);
    for (const f of frames)
      expect(validateAgainst(sse.events[f.event]!, f.data), f.event).toEqual(
        [],
      );
  });

  it("through the mock adapter: the adapter answers the same, from the same bus, and the hub scenario's role is read", async () => {
    const clock = new FakeClock(T0);
    const a = createMockAdapter({
      scenario: "healthy",
      clock,
      tokenDelayMs: 0,
    });
    made.push(a);
    const frames: EventsFrameWithId[] = [];
    a.events({ onFrame: (f) => frames.push(f), onState: () => undefined });
    const st: StorageAdapter = a.mock.storage;
    expect(st.kind).toBe("mock");
    expect(validateAgainst("StorageStatus", await st.status())).toEqual([]);
    a.mock.storageControl.setDiskLow(true);
    await clock.advanceAsync(200, 100);
    expect(frames.some((f) => f.event === "storage")).toBe(true);
    expect(
      (await fail(st.patchSettings({ origin: { datasets: true } }))).code,
    ).toBe("needs_hub"); // the healthy scenario has no hub role
    await expect(st.answerExportOffer("agent")).resolves.toBeUndefined();
  });

  it("getStorage() hands a mock page the mock's storage and a real page the HTTP adapter, importing ./storage lazily", async () => {
    resetAdapterForTests();
    const fetchFn = vi.fn(
      async () =>
        new Response(JSON.stringify(buildStorageStatus()), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
    );
    vi.stubGlobal("fetch", fetchFn);
    vi.stubGlobal("location", { search: "" });
    const st: StorageAdapter = await getStorage();
    expect(st.kind).toBe("http");
    expect((await st.status()).keep.rule).toBe("linux");
    expect(fetchFn).toHaveBeenCalledTimes(1);
    const first = getStorage();
    expect(getStorage()).toBe(first); // memoized
    resetAdapterForTests();
    expect(getStorage()).not.toBe(first); // forgotten: a new promise after the reset
  });
});

// --- the store -----------------------------------------------------------------------------------------------------------------

describe("the store handles `storage`", () => {
  function storeWith(
    status: () => StorageStatus = () => structuredClone(STATUS),
  ) {
    const fx = buildFixtures(T0);
    const adapter = {
      kind: "http",
      status: async () => structuredClone(fx.status),
      services: async () => structuredClone(fx.services),
      devices: async () => structuredClone(fx.devices),
      proposals: async () => [],
      events: () => () => undefined,
    } as unknown as Adapter;
    const calls = { status: 0 };
    const storage = {
      kind: "http",
      status: vi.fn(async () => (calls.status++, status())),
    } as unknown as StorageAdapter;
    const load = vi.fn(async () => storage);
    const store = new ApiStore({ now: () => T0, storage: null });
    return { store, adapter, calls, load };
  }
  const frame = (event: string, data: unknown) =>
    ({ event, data, id: "1" }) as unknown as EventsFrameWithId;
  const four = (s: StorageStatus): StorageStatus => ({
    ...structuredClone(s),
    settings: undefined,
    classes: s.classes.slice(0, 4),
  });

  it("a `storage` frame is folded in at once and completed by one GET /storage; a node that sends none never loads the adapter", async () => {
    const quiet = storeWith();
    await quiet.store.attach(quiet.adapter);
    quiet.store.attachStorage(quiet.load);
    await quiet.store.refreshAll();
    await flush();
    expect(quiet.load).not.toHaveBeenCalled();
    expect(quiet.store.getState().storage).toBeNull();
    quiet.store.detach();

    const { store, adapter, calls, load } = storeWith();
    await store.attach(adapter);
    store.attachStorage(load);
    const f = four(buildStorageStatus());
    store.applyFrame(frame("storage", f));
    expect(store.getState().storage!.classes).toHaveLength(4); // the frame, before the read comes back
    await flush();
    await flush();
    expect(calls.status).toBe(1);
    expect(store.getState().storage!.classes).toHaveLength(9);
    expect(store.getState().storage!.notices).toEqual(STATUS.notices);
    store.detach();
  });

  it("a frame replaces the budgeted classes in place and keeps the walked ones and the settings from the last read", () => {
    const prev: StorageStatus = {
      ...structuredClone(STATUS),
      settings: {
        keep_bytes: null,
        keep_default: 1,
        caps: { agent: null, heal: null },
        logs: { max_file_bytes: 1 },
        follow: [],
        sources: {},
        origin: {
          datasets: false,
          components: false,
          termux: false,
          max_streams: 4,
        },
      },
    };
    const next = four(buildStorageStatus());
    next.classes[0] = { ...next.classes[0]!, bytes: 5 };
    const merged = mergeStorageFrame(prev, next);
    expect(merged.classes.map((c) => c.id)).toEqual([...STORAGE_CLASS_IDS]);
    expect(merged.classes[0]!.bytes).toBe(5);
    expect(merged.classes.find((c) => c.id === "datasets")).toEqual(
      prev.classes.find((c) => c.id === "datasets"),
    );
    expect(merged.settings).toEqual(prev.settings);
    expect(merged.notices).toEqual([]); // the frame's own notices stand
    expect(mergeStorageFrame(null, next)).toBe(next);
    const same = mergeStorageFrame(prev, {
      ...structuredClone(prev),
      settings: undefined,
      classes: prev.classes.slice(0, 4),
    });
    expect(same).toBe(prev); // nothing changed: the same object
  });

  it("a malformed frame changes nothing; after `reset` a loaded slice is refetched, an unloaded one is not", async () => {
    const { store, adapter, calls, load } = storeWith();
    await store.attach(adapter);
    store.attachStorage(load);
    store.applyFrame(frame("storage", { disk: 1 }));
    store.applyFrame(frame("storage", null));
    store.applyFrame(frame("storage", { ...four(STATUS), classes: "nope" }));
    expect(store.getState().storage).toBeNull();
    await flush();
    expect(load).not.toHaveBeenCalled();
    store.applyFrame(frame("storage", four(STATUS)));
    await flush();
    await flush();
    expect(calls.status).toBe(1);
    store.applyFrame(frame("reset", { reason: "restart" })); // the first reset is the snapshot the attach already fetched
    await store.refreshAll();
    await flush();
    await flush();
    expect(calls.status).toBe(2);
    store.detach();
  });

  it("a failing read keeps the previous snapshot (an older node answers 404); reset() clears the slice; detach() forgets the loader", async () => {
    const { store, adapter, calls } = storeWith();
    await store.attach(adapter);
    store.attachStorage(
      async () =>
        ({
          kind: "http",
          status: async () =>
            Promise.reject(
              new ApiError({
                type: "about:blank",
                title: "x",
                status: 404,
                code: "not_found",
              }),
            ),
        }) as unknown as StorageAdapter,
    );
    store.applyFrame(frame("storage", four(STATUS)));
    await store.refreshStorage();
    expect(store.getState().storage!.classes).toHaveLength(4);
    store.reset();
    expect(store.getState().storage).toBeNull();
    store.detach();
    await store.refreshStorage();
    expect(calls.status).toBe(0);
  });
});

// --- the proposal card, the consent dialog ---------------------------------------------------------------------------------------------

describe("the `storage` proposal card", () => {
  const BODY = JSON.stringify({
    why: "Your disk is nearly full.",
    actions: [
      { action: "quota.set", class: "keep", bytes: 4_000_000_000 },
      { action: "quota.set", class: "agent", bytes: null },
      { action: "dataset.remove", id: "wikipedia-en", version: "2026.01" },
      {
        action: "dataset.fetch",
        publisher: "ab".repeat(32),
        id: "osm-pl",
        version: "2026.10",
      },
    ],
  });
  it("parses the closed JSON shape and nothing else", () => {
    expect(parseStorageBody(BODY)?.actions).toHaveLength(4);
    for (const bad of [
      "not json",
      "[]",
      '{"why":1,"actions":[]}',
      '{"why":"w","actions":[{"nope":1}]}',
      '{"why":"w"}',
    ])
      expect(parseStorageBody(bad), bad).toBeNull();
  });
  it("renders a card per action in the spec's words, and says a size change needs the node itself", () => {
    const out = renderToStaticMarkup(
      h(ProposalBody, { body: BODY, kind: "storage" }),
    );
    expect(out).toContain('data-testid="storage-card"');
    expect(out.match(/data-testid="storage-action"/g)).toHaveLength(4);
    expect(out).toContain("set the space set aside for this node to 4.0 GB");
    expect(out).toContain("remove the cap on the agent&#x27;s state");
    expect(out).toContain("remove wikipedia-en 2026.01 from the home hub");
    expect(out).toContain("download osm-pl 2026.10 from abababab");
    expect(out).toContain("not on it, accepting is refused");
    expect(storageActionSentence({ action: "format.disk" }).title).toContain(
      "unknown action",
    );
    expect(
      storageActionSentence({ action: "quota.set", class: "logs", bytes: -3 })
        .title,
    ).toContain("an unreadable size");
    expect(
      storageActionSentence({ action: "quota.set", class: "heal", bytes: 12 })
        .title,
    ).toBe("set repair records to 12 B");
  });
  it("draws model-written strings as text, never as Markdown, links or markup", () => {
    const body = JSON.stringify({
      why: "**bold** [x](https://evil.example) <img src=x onerror=alert(1)>",
      actions: [
        {
          action: "dataset.fetch",
          id: "<script>x</script>",
          version: "[a](b)",
        },
      ],
    });
    const out = renderToStaticMarkup(
      h(ProposalBody, { body, kind: "storage" }),
    );
    expect(out).not.toContain("<img");
    expect(out).not.toContain("<script");
    expect(out).not.toContain("<a ");
    expect(out).not.toContain("<strong");
    expect(out).toContain("&lt;img");
  });
  it("a malformed body is shown as plain text with a note, not as a card, and never as Markdown", () => {
    const out = renderToStaticMarkup(
      h(ProposalBody, { body: "do **this**", kind: "storage" }),
    );
    expect(out).not.toContain("storage-card");
    expect(out).toContain("not in the expected shape");
    expect(out).toContain("do **this**");
    expect(out).not.toContain("<strong");
  });
  it("accepting a storage proposal runs the consent round-trip, like bridge and relay; the other kinds do not", () => {
    expect(acceptRunsConsent("storage")).toBe(true);
    expect(acceptRunsConsent("relay")).toBe(true);
    expect(acceptRunsConsent("device")).toBe(false);
  });
  it("a decision's `applied` record reads as one line of text, only while it is pending", () => {
    expect(appliedLine(null)).toBeNull();
    expect(
      appliedLine({ decision: "accepted", at: "2026-11-16T09:00:00Z" }),
    ).toBeNull();
    expect(
      appliedLine({
        decision: "accepted",
        at: "x",
        applied: { pending: true },
      }),
    ).toBe("Being applied now.");
    expect(
      appliedLine({ decision: "accepted", at: "x", applied: {} }),
    ).toBeNull();
  });
  const PROP = (id: string, decision: unknown) =>
    ({
      schema: 1,
      id,
      node_id: null,
      task_id: null,
      kind: "storage",
      title: `title-${id}`,
      body: "{}",
      created_at: "2026-11-16T09:00:00Z",
      decision,
    }) as never;
  it("a decided proposal whose apply is pending is listed, and only that one", () => {
    const pend = PROP("p1", {
      decision: "accepted",
      at: "x",
      applied: { pending: true },
    });
    const list = [
      PROP("p0", null),
      pend,
      PROP("p2", { decision: "dismissed", at: "x" }),
      PROP("p3", { decision: "accepted", at: "x", applied: {} }),
    ];
    expect(applyingProposalsOf(list).map((p) => p.id)).toEqual(["p1"]);
    expect(applyingProposalsOf(null)).toEqual([]);
  });
  it("the cards render the pending line for it, without Accept or Dismiss, even with no open proposal", () => {
    const pend = PROP("p1", {
      decision: "accepted",
      at: "x",
      applied: { pending: true },
    });
    const out = renderToStaticMarkup(
      h(ProposalCards, {
        proposals: [],
        applying: [pend],
        onWhy: () => undefined,
        onDecided: () => undefined,
      }),
    );
    expect(out).toContain("title-p1");
    expect(out).toContain("Being applied now.");
    expect(out).not.toContain("Mark accepted");
    expect(out).not.toContain("Dismiss");
  });
  it("`bytes: null` is worded per class", () => {
    const t = (c: string) =>
      storageActionSentence({ action: "quota.set", class: c, bytes: null })
        .title;
    expect(t("keep")).toBe(
      "set the space set aside for this node back to its default",
    );
    expect(t("agent")).toBe(
      "remove the cap on the agent's state (it takes its share again)",
    );
    expect(t("heal")).toContain("takes its share again");
    expect(t("transit")).toBe("remove the limit on the transit class");
  });
});

describe("the consent dialog shows `facts` under the text and has a word for each new action", () => {
  const consent = (over: Record<string, unknown>) =>
    ({
      action: "dataset_fetch",
      subject: "abcd-efgh-ij:wikipedia@2026.10",
      warning_text: "Download Wikipedia 2026.10, 90 GB.",
      hash: HASH,
      addons: [],
      ...over,
    }) as Parameters<typeof ConsentPanel>[0]["consent"];
  const render = (c: ReturnType<typeof consent>) =>
    renderToStaticMarkup(
      h(ConsentPanel, {
        consent: c,
        onConfirm: () => undefined,
        onCancel: () => undefined,
      }),
    );
  it("factsLine words each action's figures as the spec does (§15.2), and says nothing else", () => {
    expect(
      factsLine("dataset_fetch", {
        free: 12_400_000_000,
        free_after: 3_400_000_000,
        days: 3,
      }),
    ).toBe(
      "Free space now: 12.4 GB; after this: 3.4 GB. About 3 days on this link.",
    );
    expect(factsLine("dataset_fetch", { free: 5000, free_after: 1000 })).toBe(
      "Free space now: 5 kB; after this: 1 kB.",
    );
    expect(
      factsLine("dataset_fetch", { free: 5000, free_after: 1000, days: 1 }),
    ).toContain("About 1 day on");
    expect(factsLine("dataset_override", { remaining: 2_000_000_000 })).toBe(
      "Up to 2.0 GB more.",
    );
    expect(
      factsLine("update_without_copy", {
        needed: 500_000_000,
        free: 100_000_000,
      }),
    ).toBe("Needed: 500.0 MB; free now: 100.0 MB.");
    expect(factsLine("dataset_fetch", undefined)).toBeNull();
    expect(factsLine("dataset_fetch", { remaining: 5 })).toBeNull(); // another action's figure
    expect(factsLine("datasets_serve", { free: 1, free_after: 1 })).toBeNull();
    expect(factsLine("store_start_empty", {})).toBeNull();
  });
  it("shows the facts line under warning_text, apart from the hashed text, and the action's own heading", () => {
    const out = renderToStaticMarkup(
      h(ConsentPanel, {
        consent: consent({
          facts: { free: 12_400_000_000, free_after: 3_400_000_000, days: 3 },
        }),
        onConfirm: () => undefined,
        onCancel: () => undefined,
      }),
    );
    expect(out.indexOf("Download Wikipedia 2026.10, 90 GB.")).toBeLessThan(
      out.indexOf('data-testid="consent-facts"'),
    );
    expect(out).toContain(
      "Free space now: 12.4 GB; after this: 3.4 GB. About 3 days on this link.",
    );
    expect(out).toContain("confirm · Download this dataset");
    expect(render(consent({}))).not.toContain("consent-facts");
  });
  it.each([
    ["dataset_fetch", "Download this dataset"],
    ["dataset_override", "Continue past today&#x27;s allowance"],
    ["datasets_serve", "Serve dataset files to this network"],
    ["components_serve", "Serve program files to this network"],
    ["update_without_copy", "Update without a rollback copy"],
    ["store_start_empty", "Start an empty store"],
  ])("%s has its own word", (action, word) => {
    expect(render(consent({ action }))).toContain(word);
  });
  it("renders every Layer 8 action's text as text, and a hostile publisher string cannot inject markup", () => {
    const out = render(
      consent({
        action: "dataset_fetch",
        warning_text: "Download <img src=x onerror=alert(1)> 2026.10.",
        facts: { free: 1, free_after: 0 },
      }),
    );
    expect(out).not.toContain("<img");
    expect(out).toContain("&lt;img");
  });
  it("the round-trip keeps `facts` for the dialog and repeats the call with the hash of the text asked about, not the facts", async () => {
    const problem = (facts: Record<string, number>) =>
      new ApiError({
        type: "about:blank",
        title: "Confirm",
        status: 409,
        code: "consent_required",
        consent: {
          action: "update_without_copy",
          subject: "2026.11.2:fabric",
          warning_text:
            "Install 2026.11.2 without a rollback copy of messages.",
          hash: HASH,
          addons: [],
          facts,
        },
      });
    const asked: unknown[] = [];
    const calls: (string | undefined)[] = [];
    const out = await runConsent(
      async (hash) => {
        calls.push(hash);
        if (hash === undefined) throw problem({ needed: 5, free: 3, junk: 9 });
        return "done";
      },
      async (c) => (asked.push(c.facts), true),
    );
    expect(out).toEqual({ ok: true, value: "done" });
    expect(asked).toEqual([{ needed: 5, free: 3 }]);
    expect(calls).toEqual([undefined, HASH]);
  });
});
