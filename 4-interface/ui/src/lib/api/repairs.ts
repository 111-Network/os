// Layer 9 adapter (os/9-evolution/README.md §18; contract: api/openapi.yaml "Layer 9", part 1: the incidents). What the Repairs view
// reads: the verdict and what needs the person (`GET /repairs`), the incidents with their attempts (`GET /repairs/incidents`, `/{id}`),
// the dismissal of one (`POST /repairs/incidents/{id}/dismiss`) and the audit trail (`GET /repairs/audit`). The routes of the contract's
// later parts (the incident task, the changes and their undo, the confirms, the settings, the fixes, the drafts, the playbooks) join this
// adapter with the rows that land them.
//
// A separate module on purpose (the Pi floor, the lazy budget of §14): the Repairs view is lazy, so this adapter is reached only through
// `getRepairs()` in ./index.ts, which imports it dynamically, and the store holds the loader, never the module. Nothing here runs at
// start-up. Every method rejects with ApiError (problem.ts) on problem+json, on transport failure (code "network") and on 401.
//
// Rules the views rely on:
//  - Paging (§18.1): `after_seq` is the last `seq` read, `limit` is at most 1,000 and 1,000 when absent; `next_after_seq` is the last row's
//    `seq`, or null when no row follows (an incident row's `seq` is its `last.seq`). `listAllIncidents` in ./store walks it, so the store
//    does not import this module.
//  - An incident's excerpts, subjects, titles and an explanation are untrusted: text only, preformatted, never Markdown or HTML.
//  - The dismissal is a human's (a loopback or paired session); the agent's bearer is refused (401) on every write.
import { ApiError, errorFromResponse, synthProblem } from "./problem";
import type { Incident, IncidentList, IncidentState, RepairAudit, RepairsStatus } from "./types";



export interface PageQuery {
  /** The `seq` of the last row already read. */
  after_seq?: number;
  /** 1..=1,000; absent reads 1,000. */
  limit?: number;
}

export interface IncidentQuery extends PageQuery {
  state?: IncidentState;
}

/** The Layer 9 adapter. The HTTP one is `createHttpRepairs`; the dev mock (mock/repairs.ts) answers the same calls. */
export interface RepairsAdapter {
  readonly kind: "http" | "mock";
  /** `GET /repairs`: the verdict, what needs the person, the counts, the store and the model queue. */
  status(): Promise<RepairsStatus>;
  /** `GET /repairs/incidents`: one page, oldest `seq` first. */
  incidents(query?: IncidentQuery): Promise<IncidentList>;
  /** `GET /repairs/incidents/{id}`; 404 `not_found` for an id the node does not hold. */
  incident(id: string): Promise<Incident>;
  /** `POST /repairs/incidents/{id}/dismiss` (a human): 204; 404 `not_found`. */
  dismissIncident(id: string): Promise<void>;
  /** `GET /repairs/audit` (a session or loopback): one page, oldest `seq` first. */
  audit(query?: PageQuery): Promise<RepairAudit>;
}

export interface RepairsDeps {
  fetch?: typeof fetch;
  /** Origin prefix; default "" (same-origin, which is the only production mode). */
  base?: string;
}

function isAbort(e: unknown): boolean {
  return typeof e === "object" && e !== null && (e as { name?: string }).name === "AbortError";
}

/** `?after_seq&limit` (and `state`) as a query string, with no key for an absent value. */
export function pageQuery(q: IncidentQuery = {}): string {
  const qs = new URLSearchParams();
  if (q.state !== undefined) qs.set("state", q.state);
  if (q.after_seq !== undefined) qs.set("after_seq", String(q.after_seq));
  if (q.limit !== undefined) qs.set("limit", String(q.limit));
  const s = qs.toString();
  return s === "" ? "" : `?${s}`;
}

export function createHttpRepairs(deps: RepairsDeps = {}): RepairsAdapter {
  const api = `${(deps.base ?? "").replace(/\/+$/, "")}/api/v1`;
  const doFetch = (input: string, init: RequestInit): Promise<Response> => (deps.fetch ?? globalThis.fetch)(input, init);

  async function call<T>(method: string, path: string, as: "json" | "none" = "json"): Promise<T> {
    const headers: Record<string, string> = { Accept: "application/json" };
    // The daemon's CSRF rule: every mutating route requires this header, body or not.
    if (method !== "GET") headers["Content-Type"] = "application/json";
    let res: Response;
    try {
      res = await doFetch(`${api}${path}`, { method, headers, credentials: "same-origin", cache: "no-store" });
    } catch (e) {
      if (isAbort(e)) throw e;
      throw ApiError.network(e);
    }
    if (!res.ok) throw await errorFromResponse(res);
    if (as === "none") return undefined as T;
    try {
      return (await res.json()) as T;
    } catch (e) {
      throw new ApiError(synthProblem(res.status, "Bad response", "The node answered with something that is not JSON."), { cause: e, code: "upstream_error" });
    }
  }

  return {
    kind: "http",
    status: () => call<RepairsStatus>("GET", "/repairs"),
    incidents: (q) => call<IncidentList>("GET", `/repairs/incidents${pageQuery(q)}`),
    incident: (id) => call<Incident>("GET", `/repairs/incidents/${encodeURIComponent(id)}`),
    dismissIncident: (id) => call<void>("POST", `/repairs/incidents/${encodeURIComponent(id)}/dismiss`, "none"),
    audit: (q) => call<RepairAudit>("GET", `/repairs/audit${pageQuery(q)}`),
  };
}
