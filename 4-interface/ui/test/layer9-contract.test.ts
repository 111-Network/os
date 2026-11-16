// Layer 9 contract pins, part 1 (9-evolution/README.md §3.4, §3.5, §4, §18.1, §18.2, §18.4, task C1): the closed enums, the id families, the
// SSE map's `incident`, the operations, fixtures for `RepairsStatus`, `Incident`, `Attempt`, the lists and the audit entry validated against
// api/openapi.yaml, the repairs adapter's calls and the paging rule, its mock, the store's handling of the `incident` event and the lazy
// `getRepairs()` seam. Fixtures are hand-written here, not daemon output. The Layer 8 precedent is test/layer8-contract.test.ts.
import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Adapter, EventsFrameWithId } from "../src/lib/api/adapter";
import { getRepairs, resetAdapterForTests } from "../src/lib/api/index";
import { createMockAdapter, type MockAdapter } from "../src/lib/api/mock";
import { buildFixtures, buildIncident, buildRepairsStatus } from "../src/lib/api/mock/fixtures";
import { createRepairsMock } from "../src/lib/api/mock/repairs";
import { ApiError } from "../src/lib/api/problem";
import { createHttpRepairs, pageQuery, type RepairsAdapter } from "../src/lib/api/repairs";
import { ApiStore, listAllIncidents, mergeIncidentFrame } from "../src/lib/api/store";
import {
  ATTEMPT_CONFIRM_BY,
  CONSENT_ACTIONS,
  PROBLEM_CODES,
  PROPOSAL_KINDS,
  ATTEMPT_STATES,
  AUDIT_KINDS,
  EVENTS_EVENT_NAMES,
  INCIDENT_AGREEMENTS,
  INCIDENT_EVIDENCE_KINDS,
  INCIDENT_OUTCOMES,
  INCIDENT_SEVERITIES,
  INCIDENT_STATES,
  NEEDS_YOU_KINDS,
  REPAIRS_STORE_STATES,
  REPAIRS_VERDICTS,
  type Attempt,
  type EventsFrame,
  type Incident,
} from "../src/lib/api/types";
import { operations, schemas, sse } from "./contract.gen";
import { FakeClock, flush, jsonResponse, problemResponse } from "./helpers/fakes";
import { validate, validateAgainst } from "./helpers/validate";

const T0 = Date.parse("2026-11-16T09:00:00Z");
const made: MockAdapter[] = [];
afterEach(() => {
  for (const a of made.splice(0)) a.mock.dispose();
  vi.unstubAllGlobals();
  resetAdapterForTests();
});

/** A diagnosed incident with one attempt waiting for the person's confirm (§4.2, §7.6), every optional member filled. */
const FULL: Incident = (() => {
  const i = buildIncident(7);
  const attempt: Attempt = {
    id: "att-0123456789ab",
    proposal_id: "prop-1",
    incident_id: i.id,
    action: "service.restart",
    target: "agent",
    state: "awaiting_confirm",
    probe: { name: "service_running", result: "ok" },
    confirm_by: "session",
  };
  i.count = 3;
  i.severity = "critical";
  i.closed = { ...i.last, seq: 9, wall_claim: 1_790_000_000 };
  i.ruled_out = ["model_missing"];
  i.diagnosis = { cause_id: "oom", by: "t1", confidence: { checks_satisfied: 750, agreement: "agree" } };
  i.explanation = { task_id: "task-9", text: "The agent was killed for memory.\nIt restarts clean." };
  i.attempts = [attempt];
  i.dismissed = { cause_id: "oom" };
  return i;
})();

describe("Layer 9 enums (part 1: the tuples gen-sse.mjs lists in ENUMS)", () => {
  it("the verdict, the needs-you kinds, the store states", () => {
    expect([...REPAIRS_VERDICTS]).toEqual(["clear", "waiting", "confirm"]);
    expect([...NEEDS_YOU_KINDS]).toEqual(["proposal", "confirm", "revert_failed", "revert_blocked", "refused"]);
    expect([...REPAIRS_STORE_STATES]).toEqual(["open", "full", "unavailable", "journal"]);
  });
  it("the incident's state, outcome, severity, evidence kinds and agreement", () => {
    expect([...INCIDENT_STATES]).toEqual(["open", "closed"]);
    expect([...INCIDENT_OUTCOMES]).toEqual(["open", "self_recovered", "fixed", "unresolved", "dismissed"]);
    expect([...INCIDENT_SEVERITIES]).toEqual(["warning", "error", "critical"]);
    expect([...INCIDENT_EVIDENCE_KINDS]).toEqual(["service_record", "health", "counters", "exit", "witness", "update_tombstone", "child_log_tail", "storage"]);
    expect([...INCIDENT_AGREEMENTS]).toEqual(["rule_only", "model_only", "agree", "disagree"]);
  });
  it("the attempt's states are §4.2's eight and `confirm_by` is session or peer", () => {
    expect([...ATTEMPT_STATES]).toEqual(["armed", "awaiting_confirm", "kept", "reverted", "revert_failed", "revert_blocked", "interrupted", "dismissed"]);
    expect([...ATTEMPT_CONFIRM_BY]).toEqual(["session", "peer"]);
  });
  it("the audit kinds are §4's fourteen", () => {
    expect([...AUDIT_KINDS]).toEqual(["proposed", "accepted", "applied", "kept", "reverted", "revert_failed", "dismissed", "undo", "fix_sent", "fix_received", "delegated", "refused", "revert_blocked", "vouch_sent"]);
  });
  it("the SSE map appends `incident` after Layer 8's `dataset`; the earlier seventeen are untouched", () => {
    expect(sse.events.incident).toBe("SseIncident");
    expect([...EVENTS_EVENT_NAMES].slice(15)).toEqual(["storage", "dataset", "incident"]);
    expect([...EVENTS_EVENT_NAMES]).toHaveLength(18);
  });
  it("part 1 appends no Problem code, no consent action and no Proposal kind: those belong to the contract's later parts", () => {
    expect(operations.dismissRepairIncident!.errors).toEqual([404]);
    // Layer 8's closed lists as they stand: part 1 appends nothing to them (a later part appends after these).
    expect([...PROBLEM_CODES].slice(28, 32)).toEqual(["untrusted_publisher", "job_running", "store_unavailable", "range_not_satisfiable"]);
    expect(PROBLEM_CODES[31]).toBe("range_not_satisfiable");
    expect([...PROPOSAL_KINDS].slice(8, 10)).toEqual(["relay", "storage"]);
    expect([...CONSENT_ACTIONS].slice(9, 15)).toEqual(["dataset_fetch", "dataset_override", "datasets_serve", "components_serve", "update_without_copy", "store_start_empty"]);
  });
});

describe("Layer 9 fixtures validate against the contract", () => {
  it("the quiet status and the open incident of the fixtures", () => {
    expect(validateAgainst("RepairsStatus", buildRepairsStatus())).toEqual([]);
    expect(validateAgainst("Incident", buildIncident())).toEqual([]);
    expect(validateAgainst("SseIncident", buildIncident(2))).toEqual([]);
  });
  it("a status that needs the person, with the store full", () => {
    const s = {
      ...buildRepairsStatus(),
      verdict: "confirm",
      open: 2,
      overflow: 1,
      store: { state: "full", reason: "heal class full" },
      model_tasks: { running: 1, waiting: 3 },
      needs_you: NEEDS_YOU_KINDS.map((kind) => ({ kind, id: "x-1", incident_id: kind === "proposal" ? null : "inc-0123456789ab", title: "Restart the agent" })),
    };
    expect(validateAgainst("RepairsStatus", s)).toEqual([]);
    expect(validateAgainst("RepairsStatus", { ...s, verdict: "ok" })).not.toEqual([]);
    expect(validateAgainst("RepairsStatus", { ...s, store: { state: "gone", reason: null } })).not.toEqual([]);
    expect(validateAgainst("RepairsStatus", { ...s, needs_you: [{ kind: "later", id: "x", incident_id: null, title: "t" }] })).not.toEqual([]);
    for (const k of ["verdict", "needs_you", "open", "overflow", "store", "model_tasks"]) {
      const { [k]: _gone, ...rest } = s as Record<string, unknown>;
      expect(validateAgainst("RepairsStatus", rest), k).not.toEqual([]);
    }
  });
  it("a closed, diagnosed incident with an attempt in every state", () => {
    expect(validateAgainst("Incident", FULL)).toEqual([]);
    for (const state of ATTEMPT_STATES) expect(validateAgainst("Attempt", { ...FULL.attempts[0]!, state }), state).toEqual([]);
    for (const by of ["t0", "t1", "hub"]) expect(validateAgainst("Incident", { ...FULL, diagnosis: { ...FULL.diagnosis, by } }), by).toEqual([]);
  });
  it("the ids are `inc-` and `att-` plus 12 hex, never anything else", () => {
    for (const bad of ["inc-0123456789a", "inc-0123456789abc", "INC-0123456789ab", "inc-0123456789ag", "att-0123456789ab"])
      expect(validateAgainst("Incident", { ...FULL, id: bad }), bad).not.toEqual([]);
    expect(validateAgainst("Attempt", { ...FULL.attempts[0]!, id: "inc-0123456789ab" })).not.toEqual([]);
    expect(validate("del-0123456789ab", schemas.DelegationId!)).toEqual([]);
    expect(validate("dft-0123456789ab", schemas.DraftId!)).toEqual([]);
    expect(validate("a".repeat(64), schemas.FixId!)).toEqual([]);
    expect(validate("a".repeat(63), schemas.FixId!)).not.toEqual([]);
  });
  it("the record's closed members refuse a value they do not list", () => {
    for (const patch of [{ state: "paused" }, { outcome: "ok" }, { severity: "fatal" }, { count: 0 }, { schema: 2 }])
      expect(validateAgainst("Incident", { ...FULL, ...patch }), JSON.stringify(patch)).not.toEqual([]);
    expect(validateAgainst("Incident", { ...FULL, evidence: [{ id: "e", kind: "message_body", ref: "r", excerpt: "x" }] })).not.toEqual([]);
    expect(validateAgainst("Incident", { ...FULL, evidence: [{ id: "e", kind: "exit", ref: "r", excerpt: "x".repeat(513) }] })).not.toEqual([]);
    expect(validateAgainst("Incident", { ...FULL, diagnosis: { cause_id: "oom", by: "t3", confidence: { checks_satisfied: 1, agreement: "agree" } } })).not.toEqual([]);
    expect(validateAgainst("Incident", { ...FULL, diagnosis: { cause_id: "oom", by: "t0", confidence: { checks_satisfied: 1001, agreement: "agree" } } })).not.toEqual([]);
    expect(validateAgainst("Incident", { ...FULL, diagnosis: { cause_id: "oom", by: "t0", confidence: { checks_satisfied: 5, agreement: "unsure" } } })).not.toEqual([]);
    expect(validateAgainst("Incident", { ...FULL, opened: { ...FULL.opened, epoch: 3 } })).not.toEqual([]); // `epoch` stays null (§3.5)
    for (const k of Object.keys(FULL)) {
      const { [k]: _gone, ...rest } = FULL as unknown as Record<string, unknown>;
      expect(validateAgainst("Incident", rest), k).not.toEqual([]);
    }
  });
  it("an attempt's `confirm_by` and `probe` are optional-null, never absent, and `confirm_by` is session or peer", () => {
    const a = FULL.attempts[0]!;
    expect(validateAgainst("Attempt", { ...a, probe: null, confirm_by: null, proposal_id: null, incident_id: null })).toEqual([]);
    expect(validateAgainst("Attempt", { ...a, confirm_by: "email" })).not.toEqual([]);
    expect(validateAgainst("Attempt", { ...a, state: "running" })).not.toEqual([]);
    const { confirm_by: _c, ...rest } = a;
    expect(validateAgainst("Attempt", rest)).not.toEqual([]);
  });
  it("the incident list and the audit list carry the paging cursor, `null` when no row follows", () => {
    expect(validateAgainst("IncidentList", { schema: 1, incidents: [FULL], next_after_seq: 9 })).toEqual([]);
    expect(validateAgainst("IncidentList", { schema: 1, incidents: [], next_after_seq: null })).toEqual([]);
    expect(validateAgainst("IncidentList", { schema: 1, incidents: [] })).not.toEqual([]);
    const entry = { seq: 4, boot_id: "b", mono_ms: 1, kind: "kept", proposal_id: "prop-1", action_hash: "a".repeat(64), probe_result: "ok", revert_result: null, confirmer: "human", by: "human" };
    expect(validateAgainst("RepairAudit", { schema: 1, entries: [entry], next_after_seq: 4 })).toEqual([]);
    expect(validateAgainst("RepairAudit", { schema: 1, entries: [{ ...entry, kind: "deleted" }], next_after_seq: 4 })).not.toEqual([]);
    expect(validateAgainst("RepairAudit", { schema: 1, entries: [{ ...entry, by: "model" }], next_after_seq: 4 })).not.toEqual([]);
    expect(validateAgainst("RepairAudit", { schema: 1, entries: [{ ...entry, confirmer: null, action_hash: null, proposal_id: null, probe_result: null }], next_after_seq: null })).toEqual([]);
  });
  it("no Layer 9 part-1 schema carries a secret, a token or a message body", () => {
    const names = Object.keys(schemas).filter((n) => /^(Repairs|Incident|Attempt|NeedsYou|Audit|RepairAudit|SseIncident)/.test(n));
    expect(names.length).toBeGreaterThan(20);
    for (const n of names) expect(JSON.stringify(schemas[n]), n).not.toMatch(/accept_warning|"hash"|token|secret|password|"body"/i);
  });
});

describe("Layer 9 operations (part 1)", () => {
  const want: Record<string, [string, string, number]> = {
    getRepairs: ["GET", "/api/v1/repairs", 200],
    listRepairIncidents: ["GET", "/api/v1/repairs/incidents", 200],
    getRepairIncident: ["GET", "/api/v1/repairs/incidents/{id}", 200],
    dismissRepairIncident: ["POST", "/api/v1/repairs/incidents/{id}/dismiss", 204],
    listRepairAudit: ["GET", "/api/v1/repairs/audit", 200],
  };
  it("are all in the contract with the expected verbs, paths and success codes", () => {
    for (const [id, [method, path, success]] of Object.entries(want)) expect([operations[id]?.method, operations[id]?.path, operations[id]?.success], id).toEqual([method, path, success]);
  });
  it("only the dismissal is a write: the global gate refuses the agent's bearer on it, and it has no request body", () => {
    for (const id of Object.keys(want)) expect(operations[id]!.method === "GET", id).toBe(id !== "dismissRepairIncident");
    expect(operations.dismissRepairIncident!.requestSchema ?? null).toBeNull();
  });
  it("the two lists take `after_seq` and `limit` (at most 1,000); the incident list also `state`", () => {
    expect(operations.listRepairIncidents!.errors).toContain(400);
    expect(operations.listRepairAudit!.errors).toContain(400);
    expect(operations.listRepairIncidents!.query.map((q) => q.name).sort()).toEqual(["after_seq", "limit", "state"]);
    expect(operations.listRepairAudit!.query.map((q) => q.name).sort()).toEqual(["after_seq", "limit"]);
    const yaml = readFileSync(new URL("../../api/openapi.yaml", import.meta.url), "utf8");
    expect(yaml).toMatch(/pageLimit:.*minimum: 1, maximum: 1000, default: 1000/);
    expect(yaml).toMatch(/afterSeq:.*minimum: 0/);
  });
  it("a missing incident is 404 on the read and on the dismissal", () => {
    expect(operations.getRepairIncident!.errors).toContain(404);
    expect(operations.dismissRepairIncident!.errors).toContain(404);
  });
});

/** A fetch recorder for the HTTP adapter. */
function recorder(respond: (c: { method: string; path: string; query: URLSearchParams; headers: Record<string, string> }) => Response | Promise<Response>) {
  const calls: { method: string; path: string; query: URLSearchParams; headers: Record<string, string>; init: RequestInit }[] = [];
  const fetchFn = (async (input: RequestInfo | URL, init: RequestInit = {}) => {
    const url = new URL(String(input), "http://node.test");
    const c = { method: init.method ?? "GET", path: url.pathname, query: url.searchParams, headers: (init.headers ?? {}) as Record<string, string>, init };
    calls.push(c);
    return respond(c);
  }) as typeof fetch;
  return { calls, fetch: fetchFn };
}

describe("the Layer 9 repairs adapter (src/lib/api/repairs.ts)", () => {
  it("each method makes the one call its operation names, same-origin and uncached, with the CSRF header only on the write", async () => {
    const r = recorder((c) => (c.method === "POST" ? new Response(null, { status: 204 }) : jsonResponse(c.path.endsWith("/audit") ? { schema: 1, entries: [], next_after_seq: null } : { schema: 1 })));
    const a = createHttpRepairs({ fetch: r.fetch });
    await a.status();
    await a.incidents({ state: "open", after_seq: 12, limit: 50 });
    await a.incident("inc-0123456789ab");
    await expect(a.dismissIncident("inc-0123456789ab")).resolves.toBeUndefined();
    await a.audit({ limit: 5 });
    expect(r.calls.map((c) => [c.method, c.path])).toEqual([
      ["GET", "/api/v1/repairs"],
      ["GET", "/api/v1/repairs/incidents"],
      ["GET", "/api/v1/repairs/incidents/inc-0123456789ab"],
      ["POST", "/api/v1/repairs/incidents/inc-0123456789ab/dismiss"],
      ["GET", "/api/v1/repairs/audit"],
    ]);
    expect(Object.fromEntries(r.calls[1]!.query)).toEqual({ state: "open", after_seq: "12", limit: "50" });
    expect(Object.fromEntries(r.calls[4]!.query)).toEqual({ limit: "5" });
    for (const c of r.calls) expect(c.init).toMatchObject({ credentials: "same-origin", cache: "no-store" });
    expect(r.calls.map((c) => c.headers["Content-Type"])).toEqual([undefined, undefined, undefined, "application/json", undefined]);
  });
  it("an id is one path segment: a hostile one is percent-encoded, never a path", async () => {
    const r = recorder(() => new Response(null, { status: 204 }));
    await createHttpRepairs({ fetch: r.fetch }).dismissIncident("../x/y?z");
    expect(r.calls[0]!.path).toBe("/api/v1/repairs/incidents/..%2Fx%2Fy%3Fz/dismiss");
  });
  it("pageQuery adds no key for an absent value", () => {
    expect(pageQuery()).toBe("");
    expect(pageQuery({ after_seq: 0 })).toBe("?after_seq=0");
    expect(pageQuery({ state: "closed", limit: 1000 })).toBe("?state=closed&limit=1000");
  });
  it("404, 401 and 400 arrive as the problem's own code; a reply that is not JSON is a bad response; a failed fetch is a network error", async () => {
    for (const [status, code] of [[404, "not_found"], [401, "unauthenticated"], [400, "bad_request"]] as const) {
      const err = await createHttpRepairs({ fetch: recorder(() => problemResponse(status, code)).fetch }).incident("inc-0123456789ab").catch((e: unknown) => e);
      expect((err as ApiError).code).toBe(code);
    }
    const bad = await createHttpRepairs({ fetch: recorder(() => new Response("<html>", { status: 200 })).fetch }).status().catch((e: unknown) => e);
    expect((bad as ApiError).code).toBe("upstream_error");
    const down = createHttpRepairs({ fetch: (() => Promise.reject(new TypeError("Failed to fetch"))) as typeof fetch });
    expect(((await down.status().catch((e: unknown) => e)) as ApiError).code).toBe("network");
  });
  it("listAllIncidents walks `next_after_seq` until null, 1,000 a page, and stops on a cursor that does not advance", async () => {
    const seen: (number | undefined)[] = [];
    const rows = [buildIncident(1), buildIncident(2), buildIncident(3)];
    const pages: Record<string, number | null> = { "-": 2, "2": 3, "3": null };
    const a = {
      incidents: async (q?: { after_seq?: number; limit?: number; state?: string }) => {
        seen.push(q?.after_seq);
        expect(q?.limit).toBe(1000);
        const next = pages[String(q?.after_seq ?? "-")] ?? null;
        return { schema: 1 as const, incidents: rows.filter((r) => r.last.seq > (q?.after_seq ?? 0) && (next === null || r.last.seq <= next)), next_after_seq: next };
      },
    };
    expect((await listAllIncidents(a)).map((i) => i.last.seq)).toEqual([1, 2, 3]);
    expect(seen).toEqual([undefined, 2, 3]);
    // a recurrence between two pages moves `last.seq`: the row comes again and the newest copy wins, once
    const a1 = buildIncident(1);
    const moved: Incident = { ...a1, count: 2, last: { ...a1.last, seq: 10 } };
    const replies = [
      { schema: 1 as const, incidents: [a1, buildIncident(2)], next_after_seq: 2 },
      { schema: 1 as const, incidents: [moved], next_after_seq: null },
    ];
    const twice = { incidents: async () => replies.shift()! };
    const got = await listAllIncidents(twice);
    expect(got.map((i) => [i.id, i.last.seq])).toEqual([[buildIncident(2).id, 2], [a1.id, 10]]);
    let n = 0;
    const stuck = { incidents: async () => ({ schema: 1 as const, incidents: [rows[n++]!], next_after_seq: 5 }) };
    expect(await listAllIncidents(stuck)).toHaveLength(2); // the first page, then one more that does not advance, then it stops
  });
});

describe("the Layer 9 mock answers the contract", () => {
  const direct = () => {
    const frames: EventsFrame[] = [];
    const m = createRepairsMock({ publish: (f) => (frames.push(f), frames.length) });
    return { m, frames };
  };
  it("a quiet node: the status validates and is clear", () => {
    const { m } = direct();
    expect(m.status()).toEqual(buildRepairsStatus());
    expect(m.incidents()).toEqual({ schema: 1, incidents: [], next_after_seq: null });
    expect(validateAgainst("RepairAudit", m.audit())).toEqual([]);
  });
  it("an incident opens with one `incident` frame, is listed, filtered by state and read by id", () => {
    const { m, frames } = direct();
    const a = m.control.open()!;
    const b = m.control.open({ fingerprint: { layer: 6, subject: "transport", reason_code: "stack_down" } })!;
    expect(frames.map((f) => f.event)).toEqual(["incident", "incident"]);
    expect(m.status().open).toBe(2);
    expect(m.incidents({ state: "open" }).incidents.map((i) => i.id)).toEqual([a.id, b.id]);
    expect(m.incidents({ state: "closed" }).incidents).toEqual([]);
    expect(m.incident(b.id).fingerprint.subject).toBe("transport");
    for (const f of frames) expect(validateAgainst(sse.events[f.event]!, f.data), f.event).toEqual([]);
    for (const i of m.incidents().incidents) expect(validateAgainst("Incident", i)).toEqual([]);
  });
  it("a store-closed node's rows (seq 0) list in one page only when `after_seq` is absent; `after_seq=0` lists none", () => {
    const { m } = direct();
    m.control.open({ last: { ...buildIncident(1).last, seq: 0 } });
    m.control.open({ last: { ...buildIncident(2).last, seq: 0 } });
    const all = m.incidents();
    expect(all.incidents).toHaveLength(2);
    expect(all.next_after_seq).toBeNull();
    expect(m.incidents({ after_seq: 0 }).incidents).toHaveLength(0);
  });
  it("paging: `limit` 1..1000, `after_seq` resumes, the last page's cursor is null; a bad number is 400", () => {
    const { m } = direct();
    for (let n = 0; n < 5; n++) m.control.open();
    const p1 = m.incidents({ limit: 2 });
    expect(p1.incidents).toHaveLength(2);
    expect(p1.next_after_seq).toBe(p1.incidents[1]!.last.seq);
    const p2 = m.incidents({ limit: 2, after_seq: p1.next_after_seq! });
    expect(p2.incidents[0]!.last.seq).toBeGreaterThan(p1.next_after_seq!);
    const p3 = m.incidents({ after_seq: p2.next_after_seq! });
    expect(p3.incidents).toHaveLength(1);
    expect(p3.next_after_seq).toBeNull(); // the last page, not only an empty one
    expect(m.incidents({ limit: 5 }).next_after_seq).toBeNull();
    expect(m.incidents({ after_seq: 99 })).toEqual({ schema: 1, incidents: [], next_after_seq: null });
    for (const bad of [{ limit: 0 }, { limit: 1001 }, { limit: 1.5 }, { after_seq: -1 }, { state: "all" as never }])
      expect(() => m.incidents(bad), JSON.stringify(bad)).toThrow(ApiError);
    try {
      m.incidents({ limit: 0 });
    } catch (e) {
      expect((e as ApiError).code).toBe("bad_request");
    }
  });
  it("a 65th open fingerprint opens nothing and counts `overflow` (§3.4)", () => {
    const { m, frames } = direct();
    for (let n = 0; n < 64; n++) expect(m.control.open()).not.toBeNull();
    expect(m.control.open()).toBeNull();
    expect(frames).toHaveLength(64);
    expect(m.status()).toMatchObject({ open: 64, overflow: 1 });
    expect(validateAgainst("RepairsStatus", m.status())).toEqual([]);
  });
  it("a dismissal closes with outcome `dismissed`, records the cause, audits, sends a frame; a missing id is 404; closing twice changes nothing", () => {
    const { m, frames } = direct();
    const i = m.control.open({ diagnosis: FULL.diagnosis })!;
    m.dismissIncident(i.id);
    const after = m.incident(i.id);
    expect(after).toMatchObject({ state: "closed", outcome: "dismissed", dismissed: { cause_id: "oom" } });
    expect(after.closed).not.toBeNull();
    expect(validateAgainst("Incident", after)).toEqual([]);
    expect(frames[frames.length - 1]!.event).toBe("incident");
    expect(m.status().open).toBe(0);
    const audit = m.audit();
    expect(audit.entries.map((e) => [e.kind, e.by])).toEqual([["dismissed", "human"]]);
    expect(validateAgainst("RepairAudit", audit)).toEqual([]);
    const n = frames.length;
    m.dismissIncident(i.id);
    expect(frames).toHaveLength(n);
    expect(() => m.dismissIncident("inc-ffffffffffff")).toThrow(/No such incident/);
    try {
      m.incident("inc-ffffffffffff");
    } catch (e) {
      expect([(e as ApiError).status, (e as ApiError).code]).toEqual([404, "not_found"]);
    }
  });
  it("the store state is reported and a reason only with a state that is not `open`", () => {
    const { m } = direct();
    m.control.setStore("unavailable", "no key");
    expect(m.status().store).toEqual({ state: "unavailable", reason: "no key" });
    m.control.setStore("open", "ignored");
    expect(m.status().store).toEqual({ state: "open", reason: null });
    expect(validateAgainst("RepairsStatus", m.status())).toEqual([]);
  });
  it("through the mock adapter: the same answers from the same bus, and `incident` frames reach a subscriber", async () => {
    const clock = new FakeClock(T0);
    const a = createMockAdapter({ scenario: "healthy", clock, tokenDelayMs: 0 });
    made.push(a);
    const frames: EventsFrameWithId[] = [];
    a.events({ onFrame: (f) => frames.push(f), onState: () => undefined });
    const rp: RepairsAdapter = a.mock.repairs;
    expect(rp.kind).toBe("mock");
    expect(validateAgainst("RepairsStatus", await rp.status())).toEqual([]);
    const opened = a.mock.repairsControl.open()!;
    await clock.advanceAsync(200, 100);
    expect(frames.some((f) => f.event === "incident")).toBe(true);
    expect((await rp.incident(opened.id)).id).toBe(opened.id);
    await expect(rp.dismissIncident(opened.id)).resolves.toBeUndefined();
    expect((await rp.audit()).entries).toHaveLength(1);
  });
  it("getRepairs() hands a mock page the mock's repairs and a real page the HTTP adapter, importing ./repairs lazily", async () => {
    resetAdapterForTests();
    const fetchFn = vi.fn(async () => jsonResponse(buildRepairsStatus()));
    vi.stubGlobal("fetch", fetchFn);
    vi.stubGlobal("location", { search: "" });
    const rp = await getRepairs();
    expect(rp.kind).toBe("http");
    expect((await rp.status()).verdict).toBe("clear");
    expect(fetchFn).toHaveBeenCalledTimes(1);
    const first = getRepairs();
    expect(getRepairs()).toBe(first);
    resetAdapterForTests();
    expect(getRepairs()).not.toBe(first);
  });
});

describe("the store handles `incident`", () => {
  function storeWith(rows: () => Incident[] = () => [buildIncident(1)]) {
    const fx = buildFixtures(T0);
    const adapter = {
      kind: "http",
      status: async () => structuredClone(fx.status),
      services: async () => structuredClone(fx.services),
      devices: async () => structuredClone(fx.devices),
      proposals: async () => [],
      events: () => () => undefined,
    } as unknown as Adapter;
    const calls = { status: 0, incidents: 0 };
    const repairs = {
      kind: "http",
      status: vi.fn(async () => (calls.status++, { ...buildRepairsStatus(), open: rows().length })),
      incidents: vi.fn(async () => (calls.incidents++, { schema: 1 as const, incidents: structuredClone(rows()), next_after_seq: null })),
    } as unknown as RepairsAdapter;
    const load = vi.fn(async () => repairs);
    const store = new ApiStore({ now: () => T0, storage: null });
    return { store, adapter, calls, load };
  }
  const frame = (event: string, data: unknown) => ({ event, data, id: "1" }) as unknown as EventsFrameWithId;

  it("a node that sends no `incident` frame never loads the adapter", async () => {
    const { store, adapter, calls, load } = storeWith();
    await store.attach(adapter);
    store.attachRepairs(load);
    await store.refreshAll();
    await flush();
    expect(load).not.toHaveBeenCalled();
    expect(store.getState().repairs).toBeNull();
    expect(store.getState().incidents).toBeNull();
    expect(calls).toEqual({ status: 0, incidents: 0 });
    store.detach();
  });
  it("a frame with nothing loaded reads the verdict only, never the list; a frame with the list loaded folds in at once and reads the verdict only", async () => {
    const { store, adapter, calls, load } = storeWith();
    await store.attach(adapter);
    store.attachRepairs(load);
    store.applyFrame(frame("incident", buildIncident(1)));
    await flush();
    await flush();
    expect(calls).toEqual({ status: 1, incidents: 0 });
    expect(store.getState().incidents).toBeNull();
    await store.refreshIncidents(); // a Layer 9 view asks for the list
    expect(calls).toEqual({ status: 1, incidents: 1 });
    expect(store.getState().incidents).toHaveLength(1);
    expect(store.getState().repairs!.verdict).toBe("clear");
    const next = { ...buildIncident(2), count: 4 };
    store.applyFrame(frame("incident", next));
    expect(store.getState().incidents!.map((i) => i.id)).toEqual([buildIncident(1).id, next.id]); // before the read comes back
    await flush();
    await flush();
    expect(calls).toEqual({ status: 2, incidents: 1 });
    store.detach();
  });
  it("the newest copy of an id wins and the list stays in `last.seq` order", () => {
    const a = buildIncident(1);
    const b = buildIncident(5);
    const a2: Incident = { ...a, count: 2, last: { ...a.last, seq: 9 } };
    const merged = mergeIncidentFrame([a, b], a2);
    expect(merged.map((i) => [i.id, i.last.seq])).toEqual([[b.id, 5], [a.id, 9]]);
    expect(mergeIncidentFrame([], a)).toEqual([a]);
  });
  it("a malformed frame changes nothing and loads nothing", async () => {
    const { store, adapter, load } = storeWith();
    await store.attach(adapter);
    store.attachRepairs(load);
    for (const bad of [null, {}, { id: 3 }, { id: "inc-0123456789ab", state: "open" }, { ...buildIncident(), attempts: null }, { ...buildIncident(), last: null }])
      store.applyFrame(frame("incident", bad));
    await flush();
    expect(load).not.toHaveBeenCalled();
    expect(store.getState().incidents).toBeNull();
    store.detach();
  });
  it("after `reset` a loaded slice is refetched (the verdict and the list), an unloaded one is not", async () => {
    const quiet = storeWith();
    await quiet.store.attach(quiet.adapter);
    quiet.store.attachRepairs(quiet.load);
    quiet.store.applyFrame(frame("reset", {}));
    await flush();
    await quiet.store.refreshAll();
    await flush();
    expect(quiet.load).not.toHaveBeenCalled();
    quiet.store.detach();

    const { store, adapter, calls, load } = storeWith();
    await store.attach(adapter);
    store.attachRepairs(load);
    store.applyFrame(frame("incident", buildIncident(1)));
    await flush();
    await flush();
    await store.refreshIncidents(); // a Layer 9 view loaded the list
    const before = { ...calls };
    await store.refreshAll();
    await flush();
    await flush();
    expect(calls.status).toBe(before.status + 1);
    expect(calls.incidents).toBe(before.incidents + 1);
    store.detach();
  });
  it("a failing read keeps the previous snapshot (an older node answers 404); reset() clears both slices; detach() forgets the loader", async () => {
    let fail = false;
    const { store, adapter, calls, load } = storeWith(() => {
      if (fail) throw new ApiError({ type: "about:blank", title: "x", status: 404, code: "not_found" });
      return [buildIncident(1)];
    });
    await store.attach(adapter);
    store.attachRepairs(load);
    store.applyFrame(frame("incident", buildIncident(1)));
    await flush();
    await flush();
    await store.refreshIncidents();
    fail = true;
    await store.refreshIncidents();
    expect(store.getState().incidents).toHaveLength(1);
    store.reset();
    expect(store.getState().repairs).toBeNull();
    expect(store.getState().incidents).toBeNull();
    store.detach();
    const n = calls.status;
    await store.refreshRepairs();
    expect(calls.status).toBe(n);
  });
});
