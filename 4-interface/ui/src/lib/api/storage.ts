// Layer 8 adapter (os/8-storage/README.md §16; contract: api/openapi.yaml "Layer 8"). The node's disk, its one budget, the store and the
// classes, the settings, the export offers, and the three store-state consents (an update without a rollback copy, a set-aside file, an
// empty store). The dataset routes are the contract's second part.
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
//  - Every string a node reports (a file name in a notice, a source URL) is untrusted: text only.
import { ApiError, errorFromResponse, synthProblem } from "./problem";
import {
  CONSENT_ACTIONS,
  type ConsentRequired,
  type StorageExportOfferRequest,
  type StorageSettings,
  type StorageSettingsPatch,
  type StorageStatus,
  type StorageStore,
} from "./types";

const HASH = /^[0-9a-f]{64}$/;

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
}

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

  const withHash = <T extends object>(body: T, acceptWarning: string | undefined): T & { accept_warning?: string } => (acceptWarning === undefined ? body : { ...body, accept_warning: acceptWarning });

  return {
    kind: "http",
    status: () => call<StorageStatus>("GET", "/storage"),
    patchSettings: (patch, acceptWarning) => call<StorageSettings>("PATCH", "/storage/settings", withHash(patch, acceptWarning)),
    answerExportOffer: (cls) => call<void>("POST", "/storage/export-offer", { class: cls }, "none"),
    acceptUpdateWithoutCopy: (release, store, acceptWarning) => call<void>("POST", "/storage/update-without-copy", withHash({ release, store }, acceptWarning), "none"),
    deleteSetAside: (file) => call<void>("DELETE", `/storage/set-aside/${enc(file)}`, undefined, "none"),
    startEmpty: (store, acceptWarning) => call<void>("POST", "/storage/start-empty", withHash({ store }, acceptWarning), "none"),
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
