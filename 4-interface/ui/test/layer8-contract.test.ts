// Layer 8 contract pins, part 1 (8-storage/README.md §16.1-§16.4, §16.6, task C1): the closed enums, the enum growth on existing
// schemas, the SSE map, the operations, fixtures for the new schemas validated against api/openapi.yaml, the storage adapter's mock, the
// store's handling of the `storage` event, the `storage` proposal card and the consent dialog's `facts`; and part 2 (task C2, the last
// describes of this file): the dataset, job, holding, publisher and tile shapes, the `dataset` event, the routes under `/datasets`,
// `/tiles` and the component stop, `Component.adapter` nullable, `panelId` as a service or a component id, the mock's jobs and consents,
// and the store's `dataset` handling. Fixtures are hand-written here, not daemon output. The Layer 7 precedent is
// test/layer7-contract.test.ts.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
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
  MOCK_HOME_HUB,
  componentsServeText,
  createStorageMock,
  datasetsServeText,
  startEmptyText,
  updateWithoutCopyText,
} from "../src/lib/api/mock/storage";
import { sha256Hex } from "../src/lib/api/mock/sha256";
import { ApiError } from "../src/lib/api/problem";
import { panelFor } from "../src/lib/api/panels";
import {
  datasetKey,
  datasetPath,
  holdingOf,
  jobEnded,
  jobPaused,
  jobPercent,
  tileUrl,
  type DatasetRef,
  type StorageAdapter,
} from "../src/lib/api/storage";
import { ApiStore, mergeJobFrame, mergeStorageFrame } from "../src/lib/api/store";
import {
  CONSENT_ACTIONS,
  EVENTS_EVENT_NAMES,
  EXPORT_OFFER_CLASSES,
  JOB_STATES,
  PROBLEM_CODES,
  PROPOSAL_KINDS,
  STORAGE_CLASS_IDS,
  STORAGE_INTEGRITY_STATES,
  STORAGE_KEEP_RULES,
  STORAGE_NODE_STATES,
  STORAGE_NOTICE_KINDS,
  STORAGE_STORES,
  type DatasetAdoptRequest,
  type DatasetDraftRequest,
  type DatasetEntry,
  type DatasetsListing,
  type EventsFrame,
  type HostedPanel,
  type Holding,
  type Job,
  type Publisher,
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
    expect([...PROBLEM_CODES].slice(27, 32)).toEqual([
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
  it("the SSE map appends `storage` and `dataset` after Layer 7's `registry`; the earlier fifteen are untouched", () => {
    expect(sse.events.storage).toBe("SseStorage");
    expect([...EVENTS_EVENT_NAMES].slice(13, 17)).toEqual([
      "relay",
      "registry",
      "storage",
      "dataset",
    ]);
    expect([...EVENTS_EVENT_NAMES].length).toBeGreaterThanOrEqual(17); // Layer 9 appends `incident` (test/layer9-contract.test.ts pins the whole list)
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

// =====================================================================================================================================
// Part 2 (task C2): datasets, tiles, the server component and the child panel (§16.1 rows, §16.2 shapes, §16.3 `dataset`, §16.6 (3)-(5), (7), (9), (10))
// =====================================================================================================================================

const HOME = MOCK_HOME_HUB;
const SHA = (c: string) => c.repeat(64);

/** A hand-written entry with every §5.3 field, then a held one and a draft. */
const ENTRY: DatasetEntry = {
  id: "wikipedia-en-simple",
  kind: "zim",
  name: "Simple English Wikipedia",
  version: "2026-10",
  seq: 3,
  size: 7_400_028_224,
  sha256: SHA("1"),
  blake3: SHA("2"),
  ob_log: 14,
  outboard_sha256: SHA("3"),
  licence: "CC-BY-SA-4.0",
  attribution: "Text from Wikipedia contributors, CC BY-SA 4.0.",
  source_url: "https://download.kiwix.org/zim/wikipedia/",
  built: "2026-10-02",
  requires: "kiwix",
  fits: "3B",
  used_by: ["encyclopedia", "agent"],
  book: "wikipedia_en_simple_all",
  state: "present",
  location: "origin",
  non_commercial: false,
};
const JOB: Job = {
  id: "job-0001",
  publisher: HOME,
  entry: "field-medicine",
  version: "2026-08",
  state: "running",
  done_bytes: 1_000_000,
  total_bytes: 2_100_000_000,
  source: "http://192.168.1.2:11180",
  metered: { match: { ssid: "Cafe" }, cap: 1_000_000_000, days: 3 },
  override_bytes: 0,
};

describe("Layer 8 part 2: the shapes validate against the contract", () => {
  const holding: Holding = { publisher: HOME, id: "wikipedia-en-simple", version: "2026-10", sha256: SHA("1"), bytes: 7_400_028_224, location: "origin", state: "present", used_by: ["encyclopedia"] };
  const publisher: Publisher = { node_id: HOME, label: "ABCD-EFGH-JK", home: true, seq: 3, source: "http://192.168.1.2:11180", sources: ["https://mirror.example.org/"], skipped: 1, entries: [ENTRY, { ...ENTRY, id: "x", state: "not_held", location: null }] };
  const listing: DatasetsListing = { schema: 1, publishers: [publisher], holdings: [holding], jobs: [JOB] };

  it("the listing, with its publisher, entries, holding and job", () => {
    expect(validateAgainst("DatasetsListing", listing)).toEqual([]);
    expect(validateAgainst("Publisher", publisher)).toEqual([]);
    expect(validateAgainst("DatasetEntry", ENTRY)).toEqual([]);
    expect(validateAgainst("Holding", holding)).toEqual([]);
    expect(validateAgainst("Job", JOB)).toEqual([]);
  });
  it("a publisher that was only imported has no source; an in-place holding is located so", () => {
    expect(validateAgainst("Publisher", { ...publisher, source: null, sources: [] })).toEqual([]);
    expect(validateAgainst("Holding", { ...holding, location: "in_place", state: "disk_absent" })).toEqual([]);
  });
  it("the one state enum is closed: an unknown state or location is refused, on an entry and on a holding", () => {
    expect(validateAgainst("DatasetEntry", { ...ENTRY, state: "downloaded" })).not.toEqual([]);
    expect(validateAgainst("Holding", { ...holding, state: "not_held" })).toEqual([]); // the enum is one: the node never sends it on a holding, the contract does not split it
    expect(validateAgainst("Holding", { ...holding, state: "gone" })).not.toEqual([]);
    expect(validateAgainst("DatasetEntry", { ...ENTRY, location: "cloud" })).not.toEqual([]);
  });
  it("an entry carries every field the signed catalogue has; a missing one is refused", () => {
    for (const k of ["id", "kind", "name", "version", "seq", "size", "sha256", "blake3", "ob_log", "outboard_sha256", "licence", "attribution", "source_url", "built", "used_by", "state", "location", "non_commercial"]) {
      const { [k as keyof DatasetEntry]: _gone, ...rest } = ENTRY;
      expect(validateAgainst("DatasetEntry", rest), k).not.toEqual([]);
    }
    const { requires: _r, fits: _f, book: _b, ...optional } = ENTRY;
    expect(validateAgainst("DatasetEntry", optional)).toEqual([]); // `requires`, `fits` and `book` are optional (§5.3)
  });
  it("an id, a version and a hash outside their patterns are refused (an id is never a path)", () => {
    expect(validateAgainst("DatasetEntry", { ...ENTRY, id: "../etc" })).not.toEqual([]);
    expect(validateAgainst("DatasetEntry", { ...ENTRY, id: "Upper" })).not.toEqual([]);
    expect(validateAgainst("DatasetEntry", { ...ENTRY, version: "a/b" })).not.toEqual([]);
    expect(validateAgainst("DatasetEntry", { ...ENTRY, sha256: "xyz" })).not.toEqual([]);
    expect(validateAgainst("DatasetEntry", { ...ENTRY, ob_log: 9 })).not.toEqual([]);
    expect(validateAgainst("DatasetEntry", { ...ENTRY, kind: "patch" })).not.toEqual([]); // an entry of a kind this node does not know is skipped by the node, never sent
  });
  it("JobState and PanelId use anyOf: the failed literals match the open pattern, every ServiceId matches ComponentId", () => {
    expect((schemas.JobState as { oneOf?: unknown }).oneOf).toBeUndefined();
    expect((schemas.JobState as { anyOf?: unknown[] }).anyOf).toHaveLength(2);
    expect((schemas.PanelId as { oneOf?: unknown }).oneOf).toBeUndefined();
    const re = new RegExp((schemas.JobState as { anyOf: { pattern?: string }[] }).anyOf[1]!.pattern!);
    expect(JOB_STATES.filter((s) => re.test(s))).toContain("failed (source cannot resume)"); // the overlap that forbids oneOf
  });
  it("every Job.state of §7.4 validates, `failed (<reason>)` with any short reason, and a pause is a state, not an error", () => {
    for (const state of [...JOB_STATES, "failed (disk write error)", "failed (x)"]) expect(validateAgainst("Job", { ...JOB, state }), state).toEqual([]);
    for (const bad of ["paused", "failed", "failed ()", "failed (" + "x".repeat(81) + ")", "Running", "paused (nothing)"]) expect(validateAgainst("Job", { ...JOB, state: bad }), bad).not.toEqual([]);
  });
  it("the job id is `job-…`, minted by 111d; metered is a rule or null; override_bytes is a count", () => {
    expect(validateAgainst("Job", { ...JOB, id: "msg-1" })).not.toEqual([]);
    expect(validateAgainst("Job", { ...JOB, metered: null, source: null })).toEqual([]);
    expect(validateAgainst("Job", { ...JOB, metered: { match: { iface: "wwan0" }, cap: 1, days: null } })).toEqual([]);
    expect(validateAgainst("Job", { ...JOB, metered: { match: {}, cap: 0, days: 1 } })).not.toEqual([]);
    expect(validateAgainst("Job", { ...JOB, override_bytes: -1 })).not.toEqual([]);
    const { override_bytes: _o, ...noOverride } = JOB;
    expect(validateAgainst("Job", noOverride)).not.toEqual([]);
  });
  it("the `dataset` frame is a Job", () => {
    expect(sse.events.dataset).toBe("SseDataset");
    expect(validateAgainst("SseDataset", JOB)).toEqual([]);
    expect(validateAgainst("SseDataset", { ...JOB, state: "nope" })).not.toEqual([]);
  });
  it("drafts, the request bodies, the search result and the tiles listing", () => {
    expect(validateAgainst("DatasetDrafts", { schema: 1, drafts: [{ draft: "x@1", entry: { ...ENTRY, id: "x", version: "1" } }] })).toEqual([]);
    expect(validateAgainst("DatasetDrafts", { schema: 1, drafts: [{ draft: "x/1", entry: ENTRY }] })).not.toEqual([]);
    expect(validateAgainst("DatasetFetchRequest", {})).toEqual([]);
    expect(validateAgainst("DatasetFetchRequest", { accept_warning: HASH, accept_override: HASH })).toEqual([]);
    expect(validateAgainst("DatasetFetchRequest", { accept_warning: "nope" })).not.toEqual([]);
    const draftReq: DatasetDraftRequest = { url: "https://mirror.example.org/x.zim", sha256: SHA("4"), id: "x", kind: "zim", name: "X", version: "1", licence: "CC0-1.0", attribution: "none", source_url: "https://mirror.example.org/", used_by: ["encyclopedia"] };
    expect(validateAgainst("DatasetDraftRequest", draftReq)).toEqual([]);
    expect(validateAgainst("DatasetDraftRequest", { ...draftReq, url: "" })).not.toEqual([]);
    expect(validateAgainst("DatasetPublishRequest", { drafts: ["x@1"], remove: [{ id: "y", version: "2" }] })).toEqual([]);
    expect(validateAgainst("DatasetPublishRequest", { drafts: [] })).not.toEqual([]); // both lists are named
    const adopt: DatasetAdoptRequest = { request: "adopt-3.json", incoming: [SHA("5")], catalogues: [HOME], drafts: [{ ...ENTRY }], in_place: [{ sha256: SHA("6"), path: "/mnt/disk/x.zim" }], follow: [HOME], sources: { [HOME]: ["https://mirror.example.org/"] } };
    expect(validateAgainst("DatasetAdoptRequest", adopt)).toEqual([]);
    expect(validateAgainst("DatasetAdoptRequest", { ...adopt, request: "../adopt-3.json" })).not.toEqual([]);
    expect(validateAgainst("DatasetSearchResult", { schema: 1, format: "opensearch-xml", body: "<feed/>" })).toEqual([]);
    expect(validateAgainst("DatasetSearchResult", { schema: 1, format: "json", body: "" })).not.toEqual([]);
    expect(validateAgainst("DatasetSearchResult", { schema: 1, format: "opensearch-xml", body: "x".repeat(8193) })).not.toEqual([]); // capped at 8 KiB
    const set = { id: "world-overview", publisher: HOME, version: "2026-09", name: "World", bytes: 120_000_000, licence: "ODbL-1.0", attribution: "OSM", url: `/api/v1/tiles/${HOME}/world-overview/2026-09` };
    expect(validateAgainst("TileSets", { schema: 1, sets: [set] })).toEqual([]);
    const { url: _u, ...noUrl } = set;
    expect(validateAgainst("TileSets", { schema: 1, sets: [noUrl] })).not.toEqual([]);
  });
  it("`Component.adapter` is a network id or null and stays required; `start` is optional and closed", () => {
    const comp = { id: "kiwix", adapter: null, state: "cached", cached_bytes: 5_000_000, installed_bytes: null, license: { spdx: "GPL-3.0-or-later", read_at: "https://github.com/kiwix/kiwix-tools", notice: "n" }, notice_hash: HASH, acknowledged: false };
    expect(validateAgainst("Component", comp)).toEqual([]);
    expect(validateAgainst("Component", { ...comp, start: "on_demand" })).toEqual([]);
    expect(validateAgainst("Component", { ...comp, start: "sometimes" })).not.toEqual([]);
    expect(validateAgainst("Component", { ...comp, adapter: "lxmf" })).toEqual([]);
    const { adapter: _a, ...gone } = comp;
    expect(validateAgainst("Component", gone), "the key never vanishes (`Component.required`)").not.toEqual([]);
    expect((schemas.Component as { required: string[] }).required).toContain("adapter");
    expect((schemas.Component as { required: string[] }).required).not.toContain("start");
  });
  it("`panelId` is a service id or a component id, and a child panel is listed under its component id", () => {
    const param = (JSON.stringify(operations.openPanelSession)).length > 0 ? (schemas.PanelId as { anyOf: { $ref: string }[] }) : null;
    expect(param!.anyOf.map((b) => b.$ref)).toEqual(["#/components/schemas/ServiceId", "#/components/schemas/ComponentId"]);
    expect(validateAgainst("PanelId", "meshtastic")).toEqual([]);
    expect(validateAgainst("PanelId", "kiwix")).toEqual([]);
    expect(validateAgainst("PanelId", "Bad Id")).not.toEqual([]);
    expect(validateAgainst("Panel", { id: "kiwix", port: 11191, entry: "/", needs_secure_context: false, allow: [], optional_hosts: [], state: { enabled: true, listening: true, health: "up" } })).toEqual([]);
    expect(panelFor([{ id: "kiwix", port: 11191 } as unknown as HostedPanel], "kiwix")?.port).toBe(11191);
    expect(panelFor([{ id: "meshtastic", port: 11190 } as unknown as HostedPanel], "kiwix")).toBeNull();
  });
  it("no dataset response shape carries a consent hash, a token or a secret; the hashes live in the request bodies only", () => {
    const seen = new Set<string>();
    const walk = (node: unknown, path: string): void => {
      if (typeof node !== "object" || node === null) return;
      const o = node as Record<string, unknown>;
      if (typeof o.$ref === "string") {
        const name = o.$ref.split("/").pop()!;
        if (!seen.has(name)) {
          seen.add(name);
          walk(schemas[name], name);
        }
        return;
      }
      for (const [k, v] of Object.entries(o)) {
        if (k === "properties") for (const prop of Object.keys(v as object)) expect(prop, `${path}.${prop}`).not.toMatch(/hash|token|secret|password|accept_|bundle/i);
        walk(v, `${path}.${k}`);
      }
    };
    for (const id of ["listDatasets", "listDatasetDrafts", "searchDatasets", "listTiles"]) walk(operations[id]!.schema, id);
    walk(schemas.Job, "Job");
    expect(seen.has("DatasetEntry")).toBe(true);
    expect(seen.has("Job")).toBe(true);
  });
});

describe("Layer 8 operations (part 2)", () => {
  const want: Record<string, [string, string, number]> = {
    listDatasets: ["GET", "/api/v1/datasets", 200],
    checkDatasets: ["POST", "/api/v1/datasets/check", 202],
    fetchDataset: ["POST", "/api/v1/datasets/entries/{publisher}/{id}/{version}/fetch", 202],
    removeDataset: ["DELETE", "/api/v1/datasets/entries/{publisher}/{id}/{version}", 204],
    cancelDatasetJob: ["DELETE", "/api/v1/datasets/jobs/{job}", 204],
    resumeDatasetJob: ["POST", "/api/v1/datasets/jobs/{job}/resume", 202],
    listDatasetDrafts: ["GET", "/api/v1/datasets/drafts", 200],
    createDatasetDraft: ["POST", "/api/v1/datasets/drafts", 202],
    publishDatasets: ["POST", "/api/v1/datasets/publish", 200],
    searchDatasets: ["GET", "/api/v1/datasets/search", 200],
    adoptDatasets: ["POST", "/api/v1/datasets/adopt", 204],
    listTiles: ["GET", "/api/v1/tiles", 200],
    getTile: ["GET", "/api/v1/tiles/{publisher}/{id}/{version}", 200],
    stopBridgeComponent: ["POST", "/api/v1/bridge/components/{id}/stop", 204],
  };
  it("are all in the contract with the expected verbs, paths and success codes", () => {
    for (const [id, [method, path, success]] of Object.entries(want)) expect([operations[id]?.method, operations[id]?.path, operations[id]?.success], id).toEqual([method, path, success]);
  });
  it("an entry lives under `entries/`, so no `id` meets a literal sibling: every other `/datasets/<x>` is a fixed word", () => {
    const literals = new Set(["check", "drafts", "publish", "search", "jobs", "adopt", "entries"]);
    const second = Object.values(operations).filter((o) => o.path.startsWith("/api/v1/datasets/")).map((o) => o.path.split("/")[4]!);
    expect(new Set(second)).toEqual(literals);
    for (const o of Object.values(operations).filter((x) => x.path.startsWith("/api/v1/datasets/entries/"))) expect(o.pathParams.slice(0, 3)).toEqual(["publisher", "id", "version"]);
  });
  it("only the reads are GET: every other operation is a write, so the global gate refuses the agent's bearer on it (401)", () => {
    const reads = new Set(["listDatasets", "listDatasetDrafts", "searchDatasets", "listTiles", "getTile"]);
    for (const id of Object.keys(want)) expect(operations[id]!.method === "GET", id).toBe(reads.has(id));
  });
  it("the refusals §16.1 names are documented per route", () => {
    expect(operations.fetchDataset!.errors).toEqual(expect.arrayContaining([404, 409]));
    expect(operations.resumeDatasetJob!.errors).toEqual(expect.arrayContaining([404, 409]));
    expect(operations.createDatasetDraft!.errors).toEqual(expect.arrayContaining([400, 409]));
    expect(operations.publishDatasets!.errors).toContain(409);
    expect(operations.searchDatasets!.errors).toEqual(expect.arrayContaining([400, 409]));
    expect(operations.adoptDatasets!.errors).toEqual(expect.arrayContaining([400, 403, 409])); // loopback only: 403 otherwise
    expect(operations.stopBridgeComponent!.errors).toEqual(expect.arrayContaining([400, 404]));
    expect(operations.getTile!.errors).toEqual(expect.arrayContaining([404, 416]));
  });
  it("a tile is `application/octet-stream`, with a 206 beside the 200 and the 416 carrying a problem+json body", () => {
    const yaml = readFileSync(resolve(__dirname, "../../api/openapi.yaml"), "utf8");
    const block = yaml.slice(yaml.indexOf("  /api/v1/tiles/{publisher}/{id}/{version}:"), yaml.indexOf("  /api/v1/bridge/components/{id}/stop:"));
    expect(block).toMatch(/"200":[^\n]*application\/octet-stream/);
    expect(block).toMatch(/"206":[^\n]*application\/octet-stream/);
    expect(block).toMatch(/"416":[^\n]*range_not_satisfiable[^\n]*components\/responses\/Problem/);
    expect(operations.getTile!.contentType).toBe("application/octet-stream");
    // HEAD is in the contract (the YAML) but not in the digest, which lists GET, POST, PUT, PATCH and DELETE: the gate refuses a bearer on HEAD
    expect(block).toMatch(/\n    head:\n      operationId: headTile\n/);
    expect(operations.headTile).toBeUndefined();
    expect(PROBLEM_CODES).toContain("range_not_satisfiable");
  });
  it("the search takes q, book, publisher and n, and only q is required", () => {
    expect(operations.searchDatasets!.query).toEqual([
      { name: "q", required: true },
      { name: "book", required: false },
      { name: "publisher", required: false },
      { name: "n", required: false },
    ]);
  });
  it("the conventions name what C2 added: job-…, the loopback-only adopt, the Not gated catalogue and notice routes, the coalescing and the 416", () => {
    const readme = readFileSync(resolve(__dirname, "../../api/README.md"), "utf8");
    expect(readme).toMatch(/`job-…`, minted by `111d`/);
    expect(readme).toMatch(/`POST \/datasets\/adopt`/);
    expect(readme).toMatch(/\/origin\/v1\/catalogue\/\*` and `\/origin\/v1\/notice\/\*`/);
    expect(readme).toMatch(/coalesced per job/);
    expect(readme).toMatch(/416 with a `problem\+json` body, code `range_not_satisfiable`/);
    expect(readme).toMatch(/Publisher strings are untrusted/);
  });
});

// --- the mock ----------------------------------------------------------------------------------------------------------------------

const WIKI = { publisher: HOME, id: "wikipedia-en-simple", version: "2026-10" };
const MAP = { publisher: HOME, id: "world-overview", version: "2026-09" };
const MED = { publisher: HOME, id: "field-medicine", version: "2026-08" };

/** The two-step round-trip a person performs: ask, read the text, repeat with its hash (and the override's when asked). */
function fetchWith(m: ReturnType<typeof direct>["m"], ref: DatasetRef): Job {
  let accept: { warning?: string; override?: string } = {};
  for (let i = 0; i < 3; i++) {
    try {
      return m.fetchDataset(ref, accept);
    } catch (e) {
      const c = (e as ApiError).problem?.consent;
      if (!c) throw e;
      accept = { ...accept, [c.action === "dataset_override" ? "override" : "warning"]: c.hash };
    }
  }
  throw new Error("a consent loop");
}
const stateOf = (m: ReturnType<typeof direct>["m"], ref: DatasetRef) => m.datasets().publishers.flatMap((p) => p.entries.map((e) => [p.node_id, e] as const)).find(([p, e]) => p === ref.publisher && e.id === ref.id && e.version === ref.version)![1];

describe("the Layer 8 mock: the listing", () => {
  it("lists the home hub with its entries (the held one present, the rest not held) and validates", () => {
    const { m } = direct();
    const l = m.datasets();
    expect(validateAgainst("DatasetsListing", l)).toEqual([]);
    expect(l.publishers).toHaveLength(1);
    expect(l.publishers[0]).toMatchObject({ node_id: HOME, home: true });
    const byId = Object.fromEntries(l.publishers[0]!.entries.map((e) => [e.id, e]));
    expect(byId["wikipedia-en-simple"]).toMatchObject({ state: "present", location: "origin", non_commercial: false });
    expect(byId["world-overview"]).toMatchObject({ state: "not_held", location: null });
    expect(byId["field-medicine"]).toMatchObject({ state: "not_held", non_commercial: true }); // an `-NC-` licence is tagged
    expect(l.holdings).toHaveLength(1);
    expect(l.jobs).toEqual([]);
  });
  it("the held bytes are the `datasets` class figure the status shows", () => {
    const { m } = direct();
    expect(m.status().classes.find((c) => c.id === "datasets")!.bytes).toBe(m.datasets().holdings.reduce((n, h) => n + h.bytes, 0));
  });
  it("a followed publisher with no catalogue yet is listed empty with seq 0; check changes nothing and answers", () => {
    const { m } = direct();
    m.patchSettings({ follow: ["cd".repeat(32)] });
    const p = m.datasets().publishers.find((x) => x.node_id === "cd".repeat(32))!;
    expect(p).toMatchObject({ home: false, seq: 0, source: null, entries: [] });
    expect(() => m.checkDatasets()).not.toThrow();
  });
  it("a catalogue that arrives (a newer `seq`, entries a node does not know counted in `skipped`) is listed with the source it came from", () => {
    const { m } = direct();
    const f = "cd".repeat(32);
    m.patchSettings({ follow: [f], sources: { [f]: ["https://mirror.example.org/"] } });
    m.control.addCatalogue(f, [{ ...ENTRY, id: "maps-x", kind: "pmtiles", used_by: ["map"] }], { skipped: 2, seq: 7, source: "https://mirror.example.org/" });
    const p = m.datasets().publishers.find((x) => x.node_id === f)!;
    expect(p).toMatchObject({ seq: 7, skipped: 2, source: "https://mirror.example.org/", sources: ["https://mirror.example.org/"] });
    expect(p.entries[0]).toMatchObject({ id: "maps-x", state: "not_held" });
    expect(validateAgainst("Publisher", p)).toEqual([]);
  });
});

describe("the Layer 8 mock: a fetch", () => {
  it("is a consent first: the exact text (the stable facts), its hash, `facts` apart from the hash, then a 202 job that queues no other", () => {
    const { m, frames } = direct();
    let err: ApiError | null = null;
    try {
      m.fetchDataset(MAP);
    } catch (e) {
      err = e as ApiError;
    }
    expect(err!.code).toBe("consent_required");
    expect(err!.status).toBe(409);
    const c = consentOfErr(err!);
    expect(c.action).toBe("dataset_fetch");
    expect(c.subject).toBe(`${HOME.slice(0, 16)}:world-overview@2026-09`);
    expect(c.warning_text).toContain("Download World overview map 2026-09, 120.0 MB, from http://192.168.1.2:11180.");
    expect(c.warning_text).toContain("Licence: ODbL-1.0.");
    expect(c.warning_text).toContain("Used by: map.");
    expect(c.warning_text).toContain("Meant for: 2A and up.");
    expect(c.warning_text).toContain("Connection: No metered link is set. You can pause or cancel it at any time.");
    expect(c.hash).toBe(sha256Hex(c.warning_text));
    expect(c.facts).toEqual({ free: 12_400_000_000, free_after: 12_280_000_000 });
    expect(c.warning_text).not.toContain("12400000000"); // the live figures are not in the hashed text (§7.7)
    expect(frames).toEqual([]);
    const j = m.fetchDataset(MAP, { warning: c.hash });
    expect(validateAgainst("Job", j)).toEqual([]);
    expect(j).toMatchObject({ publisher: HOME, entry: "world-overview", version: "2026-09", state: "running", done_bytes: 0, total_bytes: 120_000_000, override_bytes: 0, metered: null });
    expect(j.id).toMatch(/^job-/);
    expect(frames.map((f) => f.event)).toEqual(["dataset"]);
  });
  it("a stale hash asks again; a live figure that moved does not change the hash", () => {
    const { m } = direct();
    const first = consentOfErr(fail0(() => m.fetchDataset(MAP)));
    expect(consentOfErr(fail0(() => m.fetchDataset(MAP, { warning: "f".repeat(64) }))).hash).toBe(first.hash);
    m.control.setFree(9_000_000_000);
    const again = consentOfErr(fail0(() => m.fetchDataset(MAP)));
    expect(again.hash).toBe(first.hash);
    expect(again.facts).toEqual({ free: 9_000_000_000, free_after: 8_880_000_000 });
  });
  it("a second fetch queues behind the running one (at most one runs), and the same entry again is 409 job_running", () => {
    const { m } = direct();
    const a = fetchWith(m, MAP);
    const b = fetchWith(m, MED);
    expect([a.state, b.state]).toEqual(["running", "queued"]);
    expect(m.datasets().jobs.filter((j) => j.state === "running")).toHaveLength(1);
    const dup = fail0(() => fetchWith(m, MED));
    expect([dup.status, dup.code]).toEqual([409, "job_running"]);
  });
  it("the queue moves on: a job that reaches its total is done, its holding is present, the next queued one starts, and each change sends a frame", () => {
    const { m, frames } = direct();
    const a = fetchWith(m, MAP);
    const b = fetchWith(m, MED);
    frames.length = 0;
    m.control.advanceJob(a.id, 60_000_000);
    expect(m.datasets().jobs.find((j) => j.id === a.id)).toMatchObject({ state: "running", done_bytes: 60_000_000 });
    expect(stateOf(m, MAP)).toMatchObject({ state: "partial", location: "origin" });
    m.control.advanceJob(a.id, 999_000_000_000); // never past its total
    const done = m.datasets();
    expect(done.jobs.find((j) => j.id === a.id)).toMatchObject({ state: "done", done_bytes: 120_000_000 });
    expect(done.jobs.find((j) => j.id === b.id)!.state).toBe("running");
    expect(done.holdings.find((h) => h.id === "world-overview")).toMatchObject({ state: "present", bytes: 120_000_000, location: "origin" });
    expect(stateOf(m, MAP).state).toBe("present");
    expect(m.status().classes.find((c) => c.id === "datasets")!.bytes).toBe(7_400_028_224 + 120_000_000);
    expect(m.status().disk.free).toBe(12_400_000_000 - 120_000_000);
    expect(frames.map((f) => [f.event, (f.data as Job).id, (f.data as Job).state])).toEqual([
      ["dataset", a.id, "running"],
      ["dataset", a.id, "done"],
      ["dataset", b.id, "running"],
    ]);
    for (const f of frames) expect(validateAgainst(sse.events[f.event]!, f.data)).toEqual([]);
  });
  it("only a publisher the node follows (the home hub is always trusted): another is 409 untrusted_publisher, then listed once followed", () => {
    const { m } = direct();
    const other = "cd".repeat(32);
    const e = fail0(() => m.fetchDataset({ publisher: other, id: "x", version: "1" }));
    expect([e.status, e.code]).toEqual([409, "untrusted_publisher"]);
    m.patchSettings({ follow: [other] });
    m.control.addCatalogue(other, [{ ...ENTRY, id: "x", version: "1", size: 5000, used_by: [] }]);
    expect(fetchWith(m, { publisher: other, id: "x", version: "1" })).toMatchObject({ publisher: other, state: "running" });
  });
  it("an unlisted entry is 404, a held one is a bad request, a malformed key is a bad request", () => {
    const { m } = direct();
    expect(fail0(() => m.fetchDataset({ ...MAP, id: "nope" })).status).toBe(404);
    expect(fail0(() => m.fetchDataset(WIKI)).code).toBe("bad_request");
    expect(fail0(() => m.fetchDataset({ ...MAP, id: "../x" })).code).toBe("bad_request");
    expect(fail0(() => m.fetchDataset({ ...MAP, publisher: "short" })).code).toBe("bad_request");
  });
  it("space is checked before any byte moves: 409 insufficient_space, with nothing queued", () => {
    const { m } = direct();
    m.control.setFree(300_000_000); // the 1 % floor is 298 MB: 120 MB would leave 180 MB
    const e = fail0(() => m.fetchDataset(MAP));
    expect([e.status, e.code]).toEqual([409, "insufficient_space"]);
    expect(m.datasets().jobs).toEqual([]);
    m.control.setFree(12_400_000_000);
    expect(fetchWith(m, MAP).state).toBe("running");
  });
  it("an `-NC-` entry carries the non_commercial addon and says so in the text", () => {
    const { m } = direct();
    const c = consentOfErr(fail0(() => m.fetchDataset(MED)));
    expect(c.addons).toEqual(["non_commercial"]);
    expect(c.warning_text).toContain("Licence: CC-BY-NC-4.0, non-commercial.");
  });
  it("a publisher's string is text in the consent and the job: a hostile name is carried verbatim, never interpreted", () => {
    const { m } = direct();
    const f = "cd".repeat(32);
    const hostile = "<img src=x onerror=alert(1)> **bold** [link](javascript:x)";
    m.patchSettings({ follow: [f] });
    m.control.addCatalogue(f, [{ ...ENTRY, id: "evil", version: "1", name: hostile, size: 5000, attribution: hostile, used_by: [] }]);
    const c = consentOfErr(fail0(() => m.fetchDataset({ publisher: f, id: "evil", version: "1" })));
    expect(c.warning_text).toContain(hostile);
    expect(validateAgainst("Publisher", m.datasets().publishers.find((p) => p.node_id === f)!)).toEqual([]);
  });
});

/** The ApiError a callback throws (a failing test if it does not). */
function fail0(fn: () => unknown): ApiError {
  try {
    fn();
  } catch (e) {
    expect(e).toBeInstanceOf(ApiError);
    return e as ApiError;
  }
  throw new Error("expected a refusal");
}

describe("the Layer 8 mock: jobs, pauses and the metered window", () => {
  it("a pause is a state, never an error: every pause of §7.4 is reachable, validates and sends one frame, and the next queued job starts", () => {
    for (const why of ["offline", "metered cap", "metered link", "disk low", "source busy"] as const) {
      const { m, frames } = direct();
      const a = fetchWith(m, MAP);
      const b = fetchWith(m, MED);
      frames.length = 0;
      m.control.pauseJob(a.id, why);
      expect(m.datasets().jobs.find((j) => j.id === a.id)!.state).toBe(`paused (${why})`);
      expect(m.datasets().jobs.find((j) => j.id === b.id)!.state).toBe("running");
      expect(frames).toHaveLength(2);
      expect(validateAgainst("Job", m.datasets().jobs[0])).toEqual([]);
    }
  });
  it("a job paused offline resumes at once (no consent); a job that is not paused is 409 bad_request; an unknown or ended job is 404", () => {
    const { m } = direct();
    const a = fetchWith(m, MAP);
    expect(fail0(() => m.resumeDatasetJob(a.id)).code).toBe("bad_request");
    m.control.pauseJob(a.id, "offline");
    expect(m.resumeDatasetJob(a.id).state).toBe("running");
    expect(fail0(() => m.resumeDatasetJob("job-zzzz")).status).toBe(404);
    m.cancelDatasetJob(a.id);
    expect(fail0(() => m.resumeDatasetJob(a.id)).status).toBe(404);
  });
  it("a metered link names its rule in the text and the days in `facts`; `Job.metered` shows the rule and the days", () => {
    const { m } = direct();
    m.control.setMetered({ match: { ssid: "Cafe" }, cap: 1_000_000_000 });
    const c = consentOfErr(fail0(() => m.fetchDataset(MED)));
    expect(c.addons).toEqual(["metered", "non_commercial"]);
    expect(c.warning_text).toContain("'Cafe' is marked metered with a daily cap of 1.0 GB: this download pauses before the cap, leaving room for messages, and goes on in the next day's allowance.");
    expect(c.facts).toEqual({ free: 12_400_000_000, free_after: 10_300_000_000, days: 3 });
    const j = m.fetchDataset(MED, { warning: c.hash });
    expect(j.metered).toEqual({ match: { ssid: "Cafe" }, cap: 1_000_000_000, days: 3 });
    expect(validateAgainst("Job", j)).toEqual([]);
  });
  it("a job that left the link it was accepted on pauses `paused (metered link)`; the resume asks for the new text, naming the new rule, until its hash is repeated", () => {
    const { m } = direct();
    const j = fetchWith(m, MED); // accepted with no metered link
    m.control.setMetered({ match: { iface: "wwan0" }, cap: 500_000_000 });
    m.control.pauseJob(j.id, "metered link");
    const e = fail0(() => m.resumeDatasetJob(j.id));
    const c = consentOfErr(e);
    expect(c.action).toBe("dataset_fetch");
    expect(c.warning_text).toContain("'wwan0' is marked metered with a daily cap of 500.0 MB");
    expect(c.facts).toMatchObject({ days: 5 });
    const after = m.resumeDatasetJob(j.id, { warning: c.hash });
    expect(after.state).toBe("running");
    expect(after.metered).toMatchObject({ match: { iface: "wwan0" }, cap: 500_000_000 });
  });
  it("at a spent window a fetch asks the fetch consent and then the override's own text; a job past the cap counts its override bytes and pauses nothing else", () => {
    const { m } = direct();
    m.control.setMetered({ match: { ssid: "Cafe" }, cap: 1_000_000_000 });
    m.control.setMeteredCapReached(true);
    const first = consentOfErr(fail0(() => m.fetchDataset(MAP)));
    expect(first.action).toBe("dataset_fetch");
    const second = consentOfErr(fail0(() => m.fetchDataset(MAP, { warning: first.hash })));
    expect(second.action).toBe("dataset_override");
    expect(second.subject).toBe(first.subject);
    expect(second.warning_text).toBe("Continue downloading World overview map past today's allowance on 'Cafe'. This uses more than the 1.0 GB you set for this link, which may cost you money. It applies to this download only; messages keep their own room.");
    expect(second.facts).toEqual({ remaining: 120_000_000 });
    expect(second.hash).not.toBe(first.hash);
    const j = m.fetchDataset(MAP, { warning: first.hash, override: second.hash });
    expect(j.state).toBe("running");
    m.control.advanceJob(j.id, 20_000_000);
    expect(m.datasets().jobs[0]).toMatchObject({ done_bytes: 20_000_000, override_bytes: 20_000_000 });
  });
  it("a running job pauses `paused (metered cap)` when the window is spent; its resume asks for the override (`accept_override`), then runs", () => {
    const { m } = direct();
    m.control.setMetered({ match: { ssid: "Cafe" }, cap: 1_000_000_000 });
    const j = fetchWith(m, MED);
    m.control.setMeteredCapReached(true);
    expect(m.datasets().jobs[0]!.state).toBe("paused (metered cap)");
    const c = consentOfErr(fail0(() => m.resumeDatasetJob(j.id)));
    expect(c.action).toBe("dataset_override");
    expect(c.facts).toEqual({ remaining: 2_100_000_000 });
    expect(m.resumeDatasetJob(j.id, { override: c.hash }).state).toBe("running");
    m.control.advanceJob(j.id, 1_000_000);
    expect(m.datasets().jobs[0]!.override_bytes).toBe(1_000_000);
  });
  it("a job that fails keeps its reason as a state, leaves no holding and starts the next one; a cancel deletes the partial and 404s afterwards", () => {
    const { m } = direct();
    const a = fetchWith(m, MAP);
    const b = fetchWith(m, MED);
    m.control.failJob(a.id, "source cannot resume");
    const l = m.datasets();
    expect(l.jobs.find((j) => j.id === a.id)!.state).toBe("failed (source cannot resume)");
    expect(l.holdings.some((h) => h.id === "world-overview")).toBe(false);
    expect(stateOf(m, MAP).state).toBe("not_held");
    expect(l.jobs.find((j) => j.id === b.id)!.state).toBe("running");
    m.cancelDatasetJob(b.id);
    expect(m.datasets().jobs.find((j) => j.id === b.id)!.state).toBe("cancelled");
    expect(stateOf(m, MED).state).toBe("not_held");
    expect(fail0(() => m.cancelDatasetJob(b.id)).status).toBe(404);
    expect(fail0(() => m.cancelDatasetJob("job-zzzz")).status).toBe(404);
    // an ended entry may be fetched again
    expect(fetchWith(m, MAP).state).toBe("running");
  });
  it("only the last few ended jobs are kept", () => {
    const { m } = direct();
    for (let i = 0; i < 6; i++) {
      const j = fetchWith(m, i % 2 ? MAP : MED);
      m.control.failJob(j.id, "x");
    }
    expect(m.datasets().jobs.filter((j) => j.state.startsWith("failed")).length).toBeLessThanOrEqual(4);
  });
  it("removing a held dataset is 204, frees its figure, and 404s a second time or for one that is not held; the partial of a running job is not removable", () => {
    const { m } = direct();
    m.removeDataset(WIKI);
    expect(m.datasets().holdings).toEqual([]);
    expect(m.status().classes.find((c) => c.id === "datasets")!.bytes).toBe(0);
    expect(stateOf(m, WIKI).state).toBe("not_held");
    expect(fail0(() => m.removeDataset(WIKI)).status).toBe(404);
    fetchWith(m, MAP);
    expect(fail0(() => m.removeDataset(MAP)).status).toBe(404);
  });
  it("an out-of-tree file that went `unverified` or `disk_absent` shows it on both the entry and the holding", () => {
    const { m } = direct();
    m.control.setHoldingState(WIKI, "disk_absent");
    expect(stateOf(m, WIKI).state).toBe("disk_absent");
    expect(m.datasets().holdings[0]!.state).toBe("disk_absent");
    m.control.setHoldingState(WIKI, "unverified");
    expect(stateOf(m, WIKI).state).toBe("unverified");
    expect(validateAgainst("DatasetsListing", m.datasets())).toEqual([]);
  });
});

describe("the Layer 8 mock: drafts, publishing, search, tiles and the component stop", () => {
  const REQ: DatasetDraftRequest = { url: "https://mirror.example.org/x.zim", id: "x", kind: "zim", name: "X", version: "1", licence: "CC0-1.0", attribution: "none", source_url: "https://mirror.example.org/", used_by: ["encyclopedia"], book: "x_book" };
  it("a first copy is a hub's: 409 needs_hub without the role; with it the consent round-trip, a job from the upstream, then a draft", () => {
    const off = direct({ hub: false });
    expect(fail0(() => off.m.createDatasetDraft(REQ)).code).toBe("needs_hub");
    const { m } = direct();
    const c = consentOfErr(fail0(() => m.createDatasetDraft(REQ)));
    expect(c.action).toBe("dataset_fetch");
    expect(c.warning_text).toContain("https://mirror.example.org/x.zim"); // the text names the source the job will use
    const j = m.createDatasetDraft({ ...REQ, accept_warning: c.hash });
    expect(j).toMatchObject({ publisher: NODE, entry: "x", source: "https://mirror.example.org/x.zim", state: "running" });
    expect(m.datasetDrafts().drafts).toEqual([]); // not before its bytes passed
    m.control.advanceJob(j.id, 1_000_000_000);
    const d = m.datasetDrafts();
    expect(validateAgainst("DatasetDrafts", d)).toEqual([]);
    expect(d.drafts.map((x) => x.draft)).toEqual(["x@1"]);
    expect(d.drafts[0]!.entry).toMatchObject({ id: "x", version: "1", book: "x_book", state: "present" });
  });
  it("a draft request is validated: a URL that is not http(s), an id outside its pattern, a kind it does not read, a short sha256", () => {
    const { m } = direct();
    expect(fail0(() => m.createDatasetDraft({ ...REQ, url: "ftp://x" })).code).toBe("bad_request");
    expect(fail0(() => m.createDatasetDraft({ ...REQ, id: "../x" })).code).toBe("bad_request");
    expect(fail0(() => m.createDatasetDraft({ ...REQ, kind: "patch" as "zim" })).code).toBe("bad_request");
    expect(fail0(() => m.createDatasetDraft({ ...REQ, sha256: "abc" })).code).toBe("bad_request");
  });
  it("publishing moves the named drafts into the hub's own catalogue under one new `seq`, drops the removals, and is 409 needs_hub without the role", () => {
    const off = direct({ hub: false });
    expect(fail0(() => off.m.publishDatasets({ drafts: [], remove: [] })).code).toBe("needs_hub");
    const { m } = direct();
    const j = fetchDraft(m, REQ);
    m.control.advanceJob(j.id, 1_000_000_000);
    expect(fail0(() => m.publishDatasets({ drafts: ["nope@1"], remove: [] })).code).toBe("bad_request");
    expect(fail0(() => m.publishDatasets({ drafts: [], remove: [] })).code).toBe("bad_request");
    const r = m.publishDatasets({ drafts: ["x@1"], remove: [] });
    expect(r).toEqual({ seq: 1 });
    const mine = m.datasets().publishers.find((p) => p.node_id === NODE)!;
    expect(mine).toMatchObject({ seq: 1, home: false });
    expect(mine.entries.map((e) => [e.id, e.seq])).toEqual([["x", 1]]);
    expect(m.datasetDrafts().drafts).toEqual([]);
    expect(m.publishDatasets({ drafts: [], remove: [{ id: "x", version: "1" }] })).toEqual({ seq: 2 });
    expect(fail0(() => m.publishDatasets({ drafts: [], remove: [{ id: "x", version: "1" }] })).code).toBe("bad_request");
  });
  it("the search is 409 component_unavailable while the encyclopedia is off, then answers opensearch-xml capped at 8 KiB; a book without a `book` field cannot be searched by name", () => {
    const { m } = direct();
    const off = fail0(() => m.searchDatasets({ q: "water" }));
    expect([off.status, off.code, off.problem.detail]).toEqual([409, "component_unavailable", "The encyclopedia is off."]);
    m.control.setKiwix(true);
    const r = m.searchDatasets({ q: "water", book: "wikipedia-en-simple", n: 5 });
    expect(validateAgainst("DatasetSearchResult", r)).toEqual([]);
    expect(r.format).toBe("opensearch-xml");
    expect(m.searchDatasets({ q: "x".repeat(256) }).body.length).toBeLessThanOrEqual(8192);
    expect(fail0(() => m.searchDatasets({ q: "water", book: "world-overview" })).code).toBe("bad_request");
    expect(fail0(() => m.searchDatasets({ q: "water", book: "nope" })).code).toBe("bad_request");
    expect(fail0(() => m.searchDatasets({ q: "" })).code).toBe("bad_request");
    expect(fail0(() => m.searchDatasets({ q: "a", n: 21 })).code).toBe("bad_request");
    expect(fail0(() => m.searchDatasets({ q: "a", n: 0 })).code).toBe("bad_request");
    expect(fail0(() => m.searchDatasets({ q: "a", publisher: "zz" })).code).toBe("bad_request");
  });
  it("markup in the query is not echoed into the XML as markup", () => {
    const { m } = direct();
    m.control.setKiwix(true);
    expect(m.searchDatasets({ q: "<script>x</script>" }).body).not.toContain("<script>");
  });
  it("the tiles listing holds a held `pmtiles` set only, with the path of its file and the publisher's strings", () => {
    const { m } = direct();
    expect(m.tiles()).toEqual({ schema: 1, sets: [] }); // the held entry is a ZIM
    const j = fetchWith(m, MAP);
    expect(m.tiles().sets).toEqual([]); // partial: not listed
    m.control.advanceJob(j.id, 120_000_000);
    const t = m.tiles();
    expect(validateAgainst("TileSets", t)).toEqual([]);
    expect(t.sets).toEqual([{ id: "world-overview", publisher: HOME, version: "2026-09", name: "World overview map", bytes: 120_000_000, licence: "ODbL-1.0", attribution: expect.stringContaining("Open Database License"), url: `/api/v1/tiles/${HOME}/world-overview/2026-09` }]);
    expect(t.sets[0]!.url).toBe(tileUrl(t.sets[0]!));
    m.control.setHoldingState(MAP, "unverified");
    expect(m.tiles().sets).toEqual([]); // an unverified file answers 404 and leaves the tiles list (§5.5)
  });
  it("the component stop: 404 for an unknown id, 400 for one that is not on_demand, 204 for kiwix, which then runs no more", () => {
    const { m } = direct();
    expect(fail0(() => m.stopComponent("nope")).status).toBe(404);
    m.control.setKiwix(true);
    m.stopComponent("kiwix");
    expect(m.control.kiwixRunning()).toBe(false);
    m.stopComponent("kiwix"); // idempotent for one that is not running
    const m2 = createStorageMock({ publish: () => 0, loopback: () => true, componentStart: (id) => (id === "lxmf" ? "always" : undefined) });
    const e = fail0(() => m2.stopComponent("lxmf"));
    expect([e.status, e.code]).toEqual([400, "bad_request"]);
  });
});

function fetchDraft(m: ReturnType<typeof direct>["m"], req: DatasetDraftRequest): Job {
  const c = consentOfErr(fail0(() => m.createDatasetDraft(req)));
  return m.createDatasetDraft({ ...req, accept_warning: c.hash });
}

describe("the Layer 8 mock through the adapter", () => {
  it("answers the dataset calls from the same bus, republishes `dataset` frames that validate, and the mock components say how they start", async () => {
    const clock = new FakeClock(T0);
    const a = createMockAdapter({ scenario: "healthy", clock, tokenDelayMs: 0 });
    made.push(a);
    const frames: EventsFrameWithId[] = [];
    a.events({ onFrame: (f) => frames.push(f), onState: () => undefined });
    const st: StorageAdapter = a.mock.storage;
    const l = await st.datasets();
    expect(validateAgainst("DatasetsListing", l)).toEqual([]);
    const err = await fail(st.fetchDataset(MAP));
    expect(err.code).toBe("consent_required");
    const j = await st.fetchDataset(MAP, { warning: err.problem.consent!.hash });
    await clock.advanceAsync(200, 100);
    expect(frames.filter((f) => f.event === "dataset").map((f) => (f.data as Job).id)).toContain(j.id);
    for (const f of frames.filter((x) => x.event === "dataset")) expect(validateAgainst("SseDataset", f.data)).toEqual([]);
    await expect(st.cancelDatasetJob(j.id)).resolves.toBeUndefined();
    expect(validateAgainst("TileSets", await st.tiles())).toEqual([]);
    expect((await fail(st.stopComponent("lxmf"))).code).toBe("bad_request"); // the mock's bridge components start `always`
    expect((await fail(st.stopComponent("ghost"))).status).toBe(404);
    expect(a.mock.storageControl.kiwixRunning()).toBe(false);
    for (const c of await a.bridge.components()) expect(validateAgainst("Component", c), c.id).toEqual([]);
  });
});

// --- the store -----------------------------------------------------------------------------------------------------------------------

describe("the store handles `dataset`", () => {
  const LISTING: DatasetsListing = { schema: 1, publishers: [], holdings: [], jobs: [JOB] };
  function storeWith() {
    const fx = buildFixtures(T0);
    const adapter = {
      kind: "http",
      status: async () => structuredClone(fx.status),
      services: async () => structuredClone(fx.services),
      devices: async () => structuredClone(fx.devices),
      proposals: async () => [],
      events: () => () => undefined,
    } as unknown as Adapter;
    let listing = structuredClone(LISTING);
    const storage = {
      kind: "http",
      status: vi.fn(async () => structuredClone(STATUS)),
      datasets: vi.fn(async () => structuredClone(listing)),
    } as unknown as StorageAdapter;
    const load = vi.fn(async () => storage);
    const store = new ApiStore({ now: () => T0, storage: null });
    return { store, adapter, storage, load, setListing: (l: DatasetsListing) => (listing = l) };
  }
  const frame = (event: string, data: unknown) => ({ event, data, id: "1" }) as unknown as EventsFrameWithId;

  it("a node with no job sends no frame and the adapter is never loaded; the slice starts null", async () => {
    const { store, adapter, load } = storeWith();
    await store.attach(adapter);
    store.attachStorage(load);
    await store.refreshAll();
    await flush();
    expect(load).not.toHaveBeenCalled();
    expect(store.getState().datasets).toBeNull();
    store.detach();
  });
  it("a first `dataset` frame loads the adapter once and reads the listing; nothing else is fetched", async () => {
    const { store, adapter, load, storage } = storeWith();
    await store.attach(adapter);
    store.attachStorage(load);
    store.applyFrame(frame("dataset", JOB));
    await flush();
    await flush();
    expect(storage.datasets).toHaveBeenCalledTimes(1);
    expect(storage.status).not.toHaveBeenCalled(); // `storage` not loaded: not asked
    expect(store.getState().datasets!.jobs).toEqual([JOB]);
    store.detach();
  });
  it("on a loaded listing a progress frame (same state) replaces the job row in place and reads nothing; a state change re-reads the listing", async () => {
    const { store, adapter, load, storage, setListing } = storeWith();
    await store.attach(adapter);
    store.attachStorage(load);
    store.applyFrame(frame("dataset", JOB));
    await flush();
    await flush();
    const calls = (storage.datasets as ReturnType<typeof vi.fn>).mock.calls.length;
    store.applyFrame(frame("dataset", { ...JOB, done_bytes: 5_000_000 }));
    expect(store.getState().datasets!.jobs).toEqual([{ ...JOB, done_bytes: 5_000_000 }]);
    await flush();
    expect((storage.datasets as ReturnType<typeof vi.fn>).mock.calls.length).toBe(calls);
    setListing({ ...LISTING, holdings: [{ publisher: HOME, id: "field-medicine", version: "2026-08", sha256: SHA("7"), bytes: 2_100_000_000, location: "origin", state: "present", used_by: [] }], jobs: [{ ...JOB, state: "done" }] });
    store.applyFrame(frame("dataset", { ...JOB, state: "done" }));
    await flush();
    await flush();
    expect((storage.datasets as ReturnType<typeof vi.fn>).mock.calls.length).toBe(calls + 1);
    expect(store.getState().datasets!.holdings).toHaveLength(1);
    store.detach();
  });
  it("a progress frame that arrives while a listing read is in flight is not overwritten by that older snapshot", async () => {
    const { store, adapter, load, storage } = storeWith();
    await store.attach(adapter);
    store.attachStorage(load);
    store.applyFrame(frame("dataset", JOB));
    await flush();
    await flush();
    const job2 = { ...JOB, id: "job-0002", done_bytes: 0 };
    let release: (l: DatasetsListing) => void = () => undefined;
    (storage.datasets as ReturnType<typeof vi.fn>).mockImplementationOnce(() => new Promise<DatasetsListing>((r) => (release = r)));
    store.applyFrame(frame("dataset", job2)); // new job: re-read starts, the node snapshots done_bytes = 0
    await flush();
    store.applyFrame(frame("dataset", { ...job2, done_bytes: 16_000_000 })); // same state: merged, no re-read
    await flush();
    expect(store.getState().datasets!.jobs.find((j) => j.id === "job-0002")!.done_bytes).toBe(16_000_000);
    release({ ...LISTING, jobs: [JOB, job2] });
    await flush();
    await flush();
    expect(store.getState().datasets!.jobs.find((j) => j.id === "job-0002")!.done_bytes).toBe(16_000_000);
    store.detach();
  });
  it("a job that is new to a loaded listing joins it; a malformed frame changes nothing", async () => {
    const { store, adapter, load } = storeWith();
    await store.attach(adapter);
    store.attachStorage(load);
    store.applyFrame(frame("dataset", JOB));
    await flush();
    await flush();
    store.applyFrame(frame("dataset", { id: 5 }));
    store.applyFrame(frame("dataset", null));
    store.applyFrame(frame("dataset", { ...JOB, state: 7 }));
    expect(store.getState().datasets!.jobs).toEqual([JOB]);
    store.detach();
  });
  it("mergeJobFrame keeps the same object for an identical frame, and appends a new id", () => {
    expect(mergeJobFrame(LISTING, { ...JOB })).toBe(LISTING);
    const next = mergeJobFrame(LISTING, { ...JOB, id: "job-0002" });
    expect(next.jobs.map((j) => j.id)).toEqual(["job-0001", "job-0002"]);
    expect(LISTING.jobs).toHaveLength(1);
  });
  it("after `reset` a loaded listing is refetched and an unloaded one is not; reset() clears the slice and a failing read keeps the old one", async () => {
    const quiet = storeWith();
    await quiet.store.attach(quiet.adapter);
    quiet.store.attachStorage(quiet.load);
    quiet.store.applyFrame(frame("reset", {}));
    await flush();
    await flush();
    expect(quiet.storage.datasets).not.toHaveBeenCalled();
    quiet.store.detach();

    const { store, adapter, load, storage, setListing } = storeWith();
    await store.attach(adapter);
    store.attachStorage(load);
    store.applyFrame(frame("dataset", JOB));
    await flush();
    await flush();
    setListing({ ...LISTING, jobs: [] });
    store.applyFrame(frame("reset", {}));
    await flush();
    await flush();
    await flush();
    expect(store.getState().datasets!.jobs).toEqual([]);
    (storage.datasets as ReturnType<typeof vi.fn>).mockRejectedValueOnce(new Error("404"));
    await store.refreshDatasets();
    expect(store.getState().datasets!.jobs).toEqual([]); // kept
    store.reset();
    expect(store.getState().datasets).toBeNull();
    store.detach();
  });
});

describe("the dataset helpers of the storage adapter", () => {
  it("a tile's URL is built from its parts, encoded, never from a string the node sent", () => {
    expect(tileUrl({ publisher: HOME, id: "world-overview", version: "2026-09" })).toBe(`/api/v1/tiles/${HOME}/world-overview/2026-09`);
    expect(tileUrl({ publisher: "p", id: "../x", version: "1/2" })).toBe("/api/v1/tiles/p/..%2Fx/1%2F2");
    expect(tileUrl({ publisher: "p", id: "x", version: "1" }, "http://127.0.0.1:11180/")).toBe("http://127.0.0.1:11180/api/v1/tiles/p/x/1");
    expect(datasetPath({ publisher: "p", id: "x", version: "1" })).toBe("p/x/1");
    expect(datasetKey({ publisher: "p", id: "x", version: "1" })).toBe("p/x/1");
  });
  it("job states: ended, paused and a whole percent", () => {
    for (const s of ["done", "cancelled", "failed (name mismatch)", "failed (anything)"]) expect([jobEnded(s), jobPaused(s)], s).toEqual([true, false]);
    for (const s of ["paused (offline)", "paused (metered link)"]) expect([jobEnded(s), jobPaused(s)], s).toEqual([false, true]);
    for (const s of ["queued", "running"]) expect([jobEnded(s), jobPaused(s)], s).toEqual([false, false]);
    expect(jobPercent({ done_bytes: 1, total_bytes: 3 })).toBe(33);
    expect(jobPercent({ done_bytes: 5, total_bytes: 3 })).toBe(100);
    expect(jobPercent({ done_bytes: 5, total_bytes: 0 })).toBe(0);
  });
  it("holdingOf finds a held entry by publisher, id and version", () => {
    const h: Holding = { publisher: HOME, id: "a", version: "1", sha256: SHA("1"), bytes: 1, location: "origin", state: "present", used_by: [] };
    expect(holdingOf([h], HOME, { id: "a", version: "1" })).toBe(h);
    expect(holdingOf([h], "x", { id: "a", version: "1" })).toBeUndefined();
    expect(holdingOf([h], HOME, { id: "a", version: "2" })).toBeUndefined();
  });
});
