// Layer 9 mock (os/9-evolution/README.md §3.4, §3.5, §18): the Repairs status, the incidents and their dismissal, the audit trail and the
// `incident` frame, for part 1 of the contract (task C1). The routes are synchronous and throw ApiError; the adapter (index.ts) wraps
// them in its latency, auth gate and structuredClone, and republishes the frames this file hands to `publish`. The verdict and the open
// count are computed from the incidents held; the verdict is always `clear` here (the mock holds no proposal, confirm or refusal yet). Nothing here is imported by production code.
import { ApiError } from "../problem";
import type { IncidentQuery, PageQuery } from "../repairs";
import type { AuditEntry, EventsFrame, Incident, IncidentList, Problem, ProblemCode, RepairAudit, RepairsStatus } from "../types";
import { buildIncident, buildRepairsStatus } from "./fixtures";

const problem = (status: number, code: ProblemCode, title: string, detail?: string): ApiError => {
  const p: Problem = { type: "about:blank", title, status, code, ...(detail === undefined ? {} : { detail }) };
  return new ApiError(p);
};

const MAX_OPEN = 64;
const PAGE_MAX = 1000;

export interface RepairsMockDeps {
  /** Publish a frame on the mock's bus; returns its id. */
  publish(frame: EventsFrame): number;
}

/** The synchronous routes; index.ts wraps each in its latency, auth gate and clone. */
export interface RepairsRoutes {
  status(): RepairsStatus;
  incidents(query?: IncidentQuery): IncidentList;
  incident(id: string): Incident;
  dismissIncident(id: string): void;
  audit(query?: PageQuery): RepairAudit;
  /** Mock controls for tests and scenario scripts. */
  readonly control: {
    /** Open an incident (the next number picks its id and `seq`), one `incident` frame; past 64 open it opens nothing and `overflow` counts. Returns the incident, or null on overflow. */
    open(patch?: Partial<Incident>): Incident | null;
    /** Close an incident as `self_recovered` (or another outcome): one `incident` frame. */
    close(id: string, outcome?: Incident["outcome"]): void;
    /** The store's state as `GET /repairs` reports it. */
    setStore(state: RepairsStatus["store"]["state"], reason?: string | null): void;
    /** Add one audit entry (the next `seq`). */
    audit(entry: Partial<AuditEntry> & Pick<AuditEntry, "kind">): AuditEntry;
  };
}

/** `after_seq` and `limit` as the node reads them: a whole number, `limit` 1..=1000 and 1000 when absent; anything else is 400 `bad_request`. */
function paging(q: PageQuery | undefined): { after: number | undefined; limit: number } {
  const after = q?.after_seq;
  const limit = q?.limit ?? PAGE_MAX;
  if (after !== undefined && (!Number.isSafeInteger(after) || after < 0)) throw problem(400, "bad_request", "Bad request", "after_seq is not a whole number");
  if (!Number.isSafeInteger(limit) || limit < 1 || limit > PAGE_MAX) throw problem(400, "bad_request", "Bad request", "limit is 1 to 1000");
  return { after, limit };
}

export function createRepairsMock(deps: RepairsMockDeps): RepairsRoutes {
  const rows: Incident[] = [];
  const audit: AuditEntry[] = [];
  let seq = 0;
  let overflow = 0;
  let store: RepairsStatus["store"] = { state: "open", reason: null };

  const frame = (i: Incident): void => void deps.publish({ event: "incident", data: structuredClone(i) });
  const find = (id: string): Incident => {
    const i = rows.find((r) => r.id === id);
    if (!i) throw problem(404, "not_found", "Not found", "No such incident.");
    return i;
  };

  return {
    status() {
      const open = rows.filter((r) => r.state === "open").length;
      return { ...buildRepairsStatus(), verdict: "clear", open, overflow, store: { ...store } };
    },
    incidents(query) {
      const { after, limit } = paging(query);
      if (query?.state !== undefined && query.state !== "open" && query.state !== "closed") throw problem(400, "bad_request", "Bad request", "state is open or closed");
      const all = rows.filter((r) => (query?.state === undefined || r.state === query.state) && (after === undefined || r.last.seq > after)).sort((a, b) => a.last.seq - b.last.seq);
      const page = all.slice(0, limit);
      return { schema: 1, incidents: structuredClone(page), next_after_seq: page.length === all.length ? null : page[page.length - 1]!.last.seq };
    },
    incident: (id) => structuredClone(find(id)),
    dismissIncident(id) {
      const i = find(id);
      if (i.state === "closed") return;
      seq += 1;
      i.state = "closed";
      i.outcome = "dismissed";
      i.dismissed = { cause_id: i.diagnosis?.cause_id ?? null };
      i.closed = { ...i.last, seq };
      i.last = { ...i.last, seq };
      audit.push({ seq: (seq += 1), boot_id: i.last.boot_id, mono_ms: i.last.mono_ms, kind: "dismissed", proposal_id: null, action_hash: null, probe_result: null, revert_result: null, confirmer: null, by: "human" });
      frame(i);
    },
    audit(query) {
      const { after, limit } = paging(query);
      const all = audit.filter((a) => after === undefined || a.seq > after);
      const page = all.slice(0, limit);
      return { schema: 1, entries: structuredClone(page), next_after_seq: page.length === all.length ? null : page[page.length - 1]!.seq };
    },
    control: {
      open(patch) {
        if (rows.filter((r) => r.state === "open").length >= MAX_OPEN) {
          overflow += 1;
          return null;
        }
        seq += 1;
        const base = buildIncident(seq);
        const i: Incident = { ...base, ...patch, id: patch?.id ?? base.id };
        rows.push(i);
        frame(i);
        return structuredClone(i);
      },
      close(id, outcome = "self_recovered") {
        const i = find(id);
        seq += 1;
        i.state = "closed";
        i.outcome = outcome;
        i.last = { ...i.last, seq };
        i.closed = { ...i.last };
        frame(i);
      },
      setStore(state, reason = null) {
        store = { state, reason: state === "open" ? null : reason };
      },
      audit(entry) {
        seq += 1;
        const e: AuditEntry = { seq, boot_id: "3f2a9c1e5b7d4a0c8e6f1b2d9a7c5e30", mono_ms: 41_000 + seq, proposal_id: null, action_hash: null, probe_result: null, revert_result: null, confirmer: null, by: "daemon", ...entry };
        audit.push(e);
        return structuredClone(e);
      },
    },
  };
}
