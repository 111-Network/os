// Layer 8 adapter (os/8-storage/README.md §16; contract: api/openapi.yaml "Layer 8"). The node's disk, its one budget, the store and the
// classes, the settings, the export offers, and the three store-state consents (an update without a rollback copy, a set-aside file, an
// empty store); and, as the contract's second part (task C2), the dataset routes (listing, check, fetch, remove, jobs, drafts, publish,
// search, adopt), the tiles listing and the stop of a running on-demand component (kiwix-serve, §9.3).
//
// A separate module on purpose (the Pi floor, the lazy budget of §15.4): the Layer 8 views are lazy, so this adapter is reached only
// through `getStorage()` in ./index.ts, which imports it dynamically, and the store holds the loader, never the module. Nothing here
// runs at start-up. Every method rejects with ApiError (problem.ts) on problem+json, on transport failure (code "network") and on 401.
//
// Rules the views rely on:
//  - A consent gate is a 409 `consent_required` with the exact text and its hash (`storageConsentOf`); the call is repeated with
//    `accept_warning` = that hash. `facts` ride beside the text and are never part of the hash.
//  - The writes that §15.1 makes loopback-only answer 403 `loopback_only` from a paired session (`PATCH /storage/settings`,
//    `DELETE /storage/set-aside/{file}`, `POST /storage/start-empty`); the views read that code and say so.
//  - Every string a node reports (a file name in a notice, a source URL) is untrusted: text only. That holds for every publisher string
//    too (a label, an entry's name, version, licence, attribution, source URL, build date and book, a job's source): never Markdown or HTML.
//  - A dataset is keyed by publisher, id and version (`DatasetRef`), under `/datasets/entries/` so no id meets a literal sibling route.
//    A fetch or a resume is the consent round-trip: 409 `consent_required` (`dataset_fetch`; `dataset_override` at a metered cap) with
//    `facts` under the text, repeated with `accept_warning` (or `accept_override`) = the hash. 409 `insufficient_space`,
//    `untrusted_publisher`, `job_running`, `needs_hub` are plain problems the views word.
//  - `POST /datasets/adopt` is for `111 dataset import`, loopback only, never a session: the page has no method for it on purpose.
import { ApiError, errorFromResponse, synthProblem } from "./problem";
import {
  CONSENT_ACTIONS,
  type ConsentRequired,
  type DatasetDrafts,
  type DatasetDraftRequest,
  type DatasetEntry,
  type DatasetPublishRequest,
  type DatasetSearchResult,
  type DatasetsListing,
  type Holding,
  type Job,
  type TileSet,
  type TileSets,
  type StorageExportOfferRequest,
  type StorageSettings,
  type StorageSettingsPatch,
  type StorageStatus,
  type StorageStore,
} from "./types";

const HASH = /^[0-9a-f]{64}$/;

/** A dataset as a holding is keyed (§5.5): the publisher's NodeId, the entry id and its version. */
export interface DatasetRef {
  publisher: string;
  id: string;
  version: string;
}

/** The two hashes a fetch or a resume repeats with (§7.7, §7.8): the `dataset_fetch` text's and, at a metered cap, the `dataset_override` text's. */
export interface DatasetAccept {
  warning?: string;
  override?: string;
}

/** `GET /datasets/search` (§9.4): `publisher` defaults to the home hub, `book` is an entry id, `n` 1..20. */
export interface DatasetSearchQuery {
  q: string;
  book?: string;
  publisher?: string;
  n?: number;
}

/** The Layer 8 adapter. The HTTP one is `createHttpStorage`; the dev mock (mock/storage.ts) answers the same calls. */
export interface StorageAdapter {
  readonly kind: "http" | "mock";
  /** `GET /storage`: every figure, computed on request. */
  status(): Promise<StorageStatus>;
  /**
   * `PATCH /storage/settings` (loopback only): a partial merge. `origin.datasets` and `origin.components` answer 409 `consent_required`
   * (`storageConsentOf`); repeat the call with `acceptWarning` = the hash of the text the person read. 409 `needs_hub`, `lan_off`.
   */
  patchSettings(patch: StorageSettingsPatch, acceptWarning?: string): Promise<StorageSettings>;
  /** `POST /storage/export-offer`: the person answered a class's export offer (§4.5). 204. */
  answerExportOffer(cls: StorageExportOfferRequest["class"]): Promise<void>;
  /** `POST /storage/update-without-copy`: 409 `consent_required` (`update_without_copy`, with `facts`) until `acceptWarning` carries the hash; then 204. */
  acceptUpdateWithoutCopy(release: string, store: StorageStore, acceptWarning?: string): Promise<void>;
  /** `DELETE /storage/set-aside/{file}` (loopback only): delete one set-aside file a `set_aside_store` notice names. 204; 404 for any other name. */
  deleteSetAside(file: string): Promise<void>;
  /** `POST /storage/start-empty` (loopback only): 409 `consent_required` (`store_start_empty`) until `acceptWarning` carries the hash; then 204. */
  startEmpty(store: StorageStore, acceptWarning?: string): Promise<void>;

  // --- datasets (the contract's second part, §16.1) ---
  /** `GET /datasets`: the trusted publishers' catalogues with each entry's held state, the holdings and the jobs. */
  datasets(): Promise<DatasetsListing>;
  /** `POST /datasets/check`: ask each source for the catalogues now (202); the outcome arrives on `datasets()`. */
  checkDatasets(): Promise<void>;
  /** `POST /datasets/entries/{publisher}/{id}/{version}/fetch` → the queued job (202). 409 `consent_required` (`storageConsentOf`), `insufficient_space`, `untrusted_publisher`, `job_running`. */
  fetchDataset(ref: DatasetRef, accept?: DatasetAccept): Promise<Job>;
  /** `DELETE /datasets/entries/{publisher}/{id}/{version}`: remove a held dataset (204). */
  removeDataset(ref: DatasetRef): Promise<void>;
  /** `DELETE /datasets/jobs/{job}`: cancel a job; its `.part` goes (204). */
  cancelDatasetJob(job: string): Promise<void>;
  /** `POST /datasets/jobs/{job}/resume` → the job (202). A job `paused (metered link)` or `paused (metered cap)` answers 409 `consent_required` until `accept` carries the hash. */
  resumeDatasetJob(job: string, accept?: DatasetAccept): Promise<Job>;
  /** `GET /datasets/drafts`: entries not yet published. */
  datasetDrafts(): Promise<DatasetDrafts>;
  /** `POST /datasets/drafts`: a first copy from upstream (202, the job); the `dataset_fetch` consent; 409 `needs_hub`. */
  createDatasetDraft(req: DatasetDraftRequest): Promise<Job>;
  /** `POST /datasets/publish` → `{seq}`; 409 `needs_hub`. */
  publishDatasets(req: DatasetPublishRequest): Promise<{ seq: number }>;
  /** `GET /datasets/search` (§9.4): the held encyclopedias; 409 `component_unavailable` while kiwix-serve is off. */
  searchDatasets(query: DatasetSearchQuery): Promise<DatasetSearchResult>;
  /** `GET /tiles`: the held PMTiles sets. The files themselves are read through `tileUrl`, never through a method here. */
  tiles(): Promise<TileSets>;
  /** `POST /bridge/components/{id}/stop`: stop a running `on_demand` component (204); 400 for one that is not. Its files and its enable switch stay. */
  stopComponent(id: string): Promise<void>;
}

/** The path a dataset is addressed by below `/api/v1/datasets/entries/` (each part percent-encoded; the patterns of §5.3 need none). */
export const datasetPath = (r: DatasetRef): string => `${encodeURIComponent(r.publisher)}/${encodeURIComponent(r.id)}/${encodeURIComponent(r.version)}`;

/** A tile set's file URL on this origin (`GET /tiles/{publisher}/{id}/{version}`), built from its own parts, never from the node's `url` string. */
export const tileUrl = (s: Pick<TileSet, "publisher" | "id" | "version">, base = ""): string =>
  `${base.replace(/\/+$/, "")}/api/v1/tiles/${datasetPath(s)}`;

/** A key for a listed entry or a holding, the same on both (publisher, id, version). */
export const datasetKey = (r: DatasetRef): string => `${r.publisher}/${r.id}/${r.version}`;

/** The holding of an entry, if the node holds it (the same key on both). */
export const holdingOf = (holdings: readonly Holding[], publisher: string, e: Pick<DatasetEntry, "id" | "version">): Holding | undefined =>
  holdings.find((h) => h.publisher === publisher && h.id === e.id && h.version === e.version);

/** A job that is over for good (`done`, `cancelled` or `failed (<reason>)`): nothing resumes it. */
export const jobEnded = (state: Job["state"]): boolean => state === "done" || state === "cancelled" || state.startsWith("failed");

/** A job a person may resume (any `paused (<why>)`). */
export const jobPaused = (state: Job["state"]): boolean => state.startsWith("paused");

/** Whole percent of a job done, 0..100; 0 while the total is unknown. */
export const jobPercent = (j: Pick<Job, "done_bytes" | "total_bytes">): number =>
  j.total_bytes > 0 ? Math.min(100, Math.max(0, Math.floor((j.done_bytes / j.total_bytes) * 100))) : 0;

export interface StorageDeps {
  fetch?: typeof fetch;
  /** Origin prefix; default "" (same-origin, which is the only production mode). */
  base?: string;
}

function isAbort(e: unknown): boolean {
  return typeof e === "object" && e !== null && (e as { name?: string }).name === "AbortError";
}

export function createHttpStorage(deps: StorageDeps = {}): StorageAdapter {
  const api = `${(deps.base ?? "").replace(/\/+$/, "")}/api/v1`;
  const doFetch = (input: string, init: RequestInit): Promise<Response> => (deps.fetch ?? globalThis.fetch)(input, init);
  const enc = encodeURIComponent;

  async function call<T>(method: string, path: string, body?: unknown, as: "json" | "none" = "json"): Promise<T> {
    const headers: Record<string, string> = { Accept: "application/json" };
    const init: RequestInit = { method, headers, credentials: "same-origin", cache: "no-store" };
    if (method !== "GET") {
      // The daemon's CSRF rule: every mutating route requires this header, body or not.
      headers["Content-Type"] = "application/json";
      if (body !== undefined) init.body = JSON.stringify(body);
    }
    let res: Response;
    try {
      res = await doFetch(`${api}${path}`, init);
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

  const withAccept = (accept: DatasetAccept | undefined): { accept_warning?: string; accept_override?: string } => ({
    ...(accept?.warning === undefined ? {} : { accept_warning: accept.warning }),
    ...(accept?.override === undefined ? {} : { accept_override: accept.override }),
  });
  const withHash = <T extends object>(body: T, acceptWarning: string | undefined): T & { accept_warning?: string } => (acceptWarning === undefined ? body : { ...body, accept_warning: acceptWarning });

  return {
    kind: "http",
    status: () => call<StorageStatus>("GET", "/storage"),
    patchSettings: (patch, acceptWarning) => call<StorageSettings>("PATCH", "/storage/settings", withHash(patch, acceptWarning)),
    answerExportOffer: (cls) => call<void>("POST", "/storage/export-offer", { class: cls }, "none"),
    acceptUpdateWithoutCopy: (release, store, acceptWarning) => call<void>("POST", "/storage/update-without-copy", withHash({ release, store }, acceptWarning), "none"),
    deleteSetAside: (file) => call<void>("DELETE", `/storage/set-aside/${enc(file)}`, undefined, "none"),
    startEmpty: (store, acceptWarning) => call<void>("POST", "/storage/start-empty", withHash({ store }, acceptWarning), "none"),
    datasets: () => call<DatasetsListing>("GET", "/datasets"),
    checkDatasets: () => call<void>("POST", "/datasets/check", undefined, "none"),
    fetchDataset: (ref, accept) => call<Job>("POST", `/datasets/entries/${datasetPath(ref)}/fetch`, withAccept(accept)),
    removeDataset: (ref) => call<void>("DELETE", `/datasets/entries/${datasetPath(ref)}`, undefined, "none"),
    cancelDatasetJob: (job) => call<void>("DELETE", `/datasets/jobs/${enc(job)}`, undefined, "none"),
    resumeDatasetJob: (job, accept) => call<Job>("POST", `/datasets/jobs/${enc(job)}/resume`, withAccept(accept)),
    datasetDrafts: () => call<DatasetDrafts>("GET", "/datasets/drafts"),
    createDatasetDraft: (req) => call<Job>("POST", "/datasets/drafts", req),
    publishDatasets: (req) => call<{ seq: number }>("POST", "/datasets/publish", req),
    searchDatasets: (query) => {
      const qs = new URLSearchParams({ q: query.q });
      if (query.book !== undefined) qs.set("book", query.book);
      if (query.publisher !== undefined) qs.set("publisher", query.publisher);
      if (query.n !== undefined) qs.set("n", String(query.n));
      return call<DatasetSearchResult>("GET", `/datasets/search?${qs.toString()}`);
    },
    tiles: () => call<TileSets>("GET", "/tiles"),
    stopComponent: (id) => call<void>("POST", `/bridge/components/${enc(id)}/stop`, undefined, "none"),
  };
}

// --- consent (the pinned round-trip, os/7-relay/README.md §12; Layer 8's six actions, os/8-storage/README.md §15.2) -----------------------

const FACT_KEYS = ["free", "free_after", "days", "remaining", "needed"] as const;

/** `facts` as a clean object of non-negative integers, or undefined when the node sent none (or sent nothing usable). */
export function consentFacts(raw: unknown): NonNullable<ConsentRequired["facts"]> | undefined {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return undefined;
  const out: Record<string, number> = {};
  for (const k of FACT_KEYS) {
    const v = (raw as Record<string, unknown>)[k];
    if (typeof v === "number" && Number.isSafeInteger(v) && v >= 0) out[k] = v;
  }
  return Object.keys(out).length > 0 ? out : undefined;
}

/**
 * A well-formed `ConsentRequired` from a failed Layer 8 call, or null; `facts` (the live figures under the text, never hashed) are kept
 * when they read. A malformed consent is never offered (fail closed).
 */
export function storageConsentOf(err: unknown): ConsentRequired | null {
  if (!(err instanceof ApiError) || err.code !== "consent_required") return null;
  const c = err.problem.consent as Partial<ConsentRequired> | undefined;
  if (typeof c !== "object" || c === null) return null;
  if (typeof c.warning_text !== "string" || c.warning_text === "" || typeof c.hash !== "string" || !HASH.test(c.hash) || typeof c.subject !== "string") return null;
  if (typeof c.action !== "string" || !(CONSENT_ACTIONS as readonly string[]).includes(c.action)) return null;
  const addons = Array.isArray(c.addons) ? c.addons.filter((a): a is string => typeof a === "string") : [];
  const facts = consentFacts((c as { facts?: unknown }).facts);
  const out: ConsentRequired = { action: c.action as ConsentRequired["action"], subject: c.subject, warning_text: c.warning_text, hash: c.hash, addons };
  if (facts) out.facts = facts;
  return out;
}
