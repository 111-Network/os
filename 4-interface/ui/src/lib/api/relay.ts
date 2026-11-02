// Layer 7 adapter (os/7-relay/README.md §13; contract: api/openapi.yaml "Layer 7"). The relay status and settings, hub pairing, the
// service registry, the Reticulum hop read and the update origins.
//
// A separate module on purpose (the Pi floor and the eager budget): the Layer 7 views are lazy, so this adapter is reached only through
// `getRelay()` in ./index.ts, which imports it dynamically, and the store holds the loader, never the module. Nothing here runs at
// start-up. The shape follows `Adapter`: every method rejects with ApiError (problem.ts) on problem+json, on transport failure (code
// "network") and on 401 (code "unauthenticated").
//
// Rules the views rely on:
//  - Secrets are write-only. `RelayStatus` carries `{set: boolean}` for the carrier token and topic secret, never a value; this module
//    never logs, stores or echoes a body.
//  - A consent gate is a 409 `consent_required` with the exact text and its hash (`relayConsentOf`); the call is repeated with
//    `accept_warning` = that hash (`patchSettings(patch, hash)`, `pair({..., accept_warning})`).
//  - Every string a node sends (labels, endpoints, urls, evidence) is untrusted: text only.
import { ApiError, errorFromResponse, synthProblem } from "./problem";
import {
  CONSENT_ACTIONS,
  type ConsentRequired,
  type NodeId,
  type PairOffer,
  type PairOfferRequest,
  type PairRequest,
  type PairedHub,
  type RegistryRow,
  type RelayAccepted,
  type RelaySettingsPatch,
  type RelayStatus,
  type UpdateOrigin,
} from "./types";

const HASH = /^[0-9a-f]{64}$/;

/**
 * What callers pass to `pair`. The contract's `PairRequest` with `lan` optional: openapi-typescript marks a property that has a schema
 * default (`lan`, default false) as required, as it does for `ChatInput`.
 */
export type PairInput = Omit<PairRequest, "lan"> & { lan?: boolean };

/** The Layer 7 adapter. The HTTP one is `createHttpRelay`; the dev mock (mock/relay.ts) answers the same calls. */
export interface RelayAdapter {
  readonly kind: "http" | "mock";
  /** `GET /relay`: the whole Layer 7 state. No secret is in it. */
  status(): Promise<RelayStatus>;
  /**
   * `PATCH /relay/settings`: a partial merge. A consent gate answers 409 `consent_required` (`relayConsentOf`); repeat the call with
   * `acceptWarning` = the hash of the text the person read. 409 `no_such_transport` / `settings_locked` for a heartbeat write (§2.4).
   */
  patchSettings(patch: RelaySettingsPatch, acceptWarning?: string): Promise<RelayStatus>;
  /** `POST /relay/pair/offer` (loopback only): mint the `111PAIR:` bundle for the role this hub proposes for the other. */
  pairOffer(body: PairOfferRequest): Promise<PairOffer>;
  /** `POST /relay/pair` (loopback only): 202 with the new pair. 409 `consent_required` (`pair_plain`), `pair_role_mismatch`, `needs_hub`. */
  pair(body: PairInput): Promise<PairedHub>;
  /** `DELETE /relay/paired/{node_id}` (loopback only). Idempotent 204. */
  unpair(nodeId: NodeId): Promise<void>;
  /** `POST /relay/carrier/check`: 202; the outcome arrives as a `relay` frame. 409 `consent_required`, `metered_cap`, `licensed_board`. */
  checkCarrier(): Promise<RelayAccepted>;
  /** `POST /relay/carrier/flush`: 202; progress arrives as `relay` frames. Errors as `checkCarrier`. */
  flushCarrier(): Promise<RelayAccepted>;
  /** `GET /registry`: unwraps `{schema, rows}`. */
  registry(): Promise<RegistryRow[]>;
  /** `POST /registry/browse`: a DNS-SD browse of at most 5 s; the rows arrive as a `registry` frame. 409 `lan_off`. */
  browseRegistry(): Promise<RelayAccepted>;
  /** `POST /nearby/hops`: one read of Reticulum hop counts; they arrive on the `nearby` frame. 409 `component_unavailable`; 429 within 30 s. */
  nearbyHops(): Promise<RelayAccepted>;
  /** `GET /update/origins`: unwraps `{schema, origins}`, in the order they are tried. */
  updateOrigins(): Promise<UpdateOrigin[]>;
}

export interface RelayDeps {
  fetch?: typeof fetch;
  /** Origin prefix; default "" (same-origin, which is the only production mode). */
  base?: string;
}

function isAbort(e: unknown): boolean {
  return typeof e === "object" && e !== null && (e as { name?: string }).name === "AbortError";
}

export function createHttpRelay(deps: RelayDeps = {}): RelayAdapter {
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

  /** Unwrap `{schema, <key>: [...]}`; a reply without the list is a bad response, never an empty one. */
  async function list<T>(path: string, key: string, what: string): Promise<T[]> {
    const body = await call<Record<string, unknown>>("GET", path);
    const items = body?.[key];
    if (!Array.isArray(items)) throw new ApiError(synthProblem(200, "Bad response", `Missing ${what} list.`), { code: "upstream_error" });
    return items as T[];
  }

  return {
    kind: "http",
    status: () => call<RelayStatus>("GET", "/relay"),
    patchSettings: (patch, acceptWarning) => call<RelayStatus>("PATCH", "/relay/settings", acceptWarning === undefined ? patch : { ...patch, accept_warning: acceptWarning }),
    pairOffer: (body) => call<PairOffer>("POST", "/relay/pair/offer", body),
    pair: (body) => call<PairedHub>("POST", "/relay/pair", body),
    unpair: (nodeId) => call<void>("DELETE", `/relay/paired/${enc(nodeId)}`, undefined, "none"),
    checkCarrier: () => call<RelayAccepted>("POST", "/relay/carrier/check"),
    flushCarrier: () => call<RelayAccepted>("POST", "/relay/carrier/flush"),
    registry: () => list<RegistryRow>("/registry", "rows", "registry"),
    browseRegistry: () => call<RelayAccepted>("POST", "/registry/browse"),
    nearbyHops: () => call<RelayAccepted>("POST", "/nearby/hops"),
    updateOrigins: () => list<UpdateOrigin>("/update/origins", "origins", "update origin"),
  };
}

// --- consent (the pinned round-trip, os/7-relay/README.md §12) ---------------------------------------------------------------

/**
 * A well-formed `ConsentRequired` from a failed Layer 7 call, or null. Like `consentOf` (bridge.ts) but over the whole generated action
 * set, so `carrier_enable`, `pair_plain`, `gateway_bit` and `origin_serve` pass. A malformed consent is never offered (fail closed).
 */
export function relayConsentOf(err: unknown): ConsentRequired | null {
  if (!(err instanceof ApiError) || err.code !== "consent_required") return null;
  const c = err.problem.consent as Partial<ConsentRequired> | undefined;
  if (typeof c !== "object" || c === null) return null;
  if (typeof c.warning_text !== "string" || c.warning_text === "" || typeof c.hash !== "string" || !HASH.test(c.hash) || typeof c.subject !== "string") return null;
  if (typeof c.action !== "string" || !(CONSENT_ACTIONS as readonly string[]).includes(c.action)) return null;
  const addons = Array.isArray(c.addons) ? c.addons.filter((a): a is string => typeof a === "string") : [];
  return { action: c.action as ConsentRequired["action"], subject: c.subject, warning_text: c.warning_text, hash: c.hash, addons };
}
