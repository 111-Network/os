// Settings, Relay & presence: the calls the page makes and the one helper that runs them (os/7-relay/README.md §12, §13.1). Every Layer 7
// consent is the pinned round trip: the node answers 409 `consent_required` with the exact text and its hash, the page shows the text
// verbatim (the shared consent panel) and repeats the call with `accept_warning` = that hash. Nothing here keeps a secret: a token or
// a topic secret goes out in one call and is never held, shown or logged.
//
// The round trip is structural, not a choice each control makes: every write of the page is a method of the object `useRelayCall()`
// returns, each of them hands the hash to the adapter call it makes, and the object always carries the `dialog` element that the section
// renders through `<CallOutput>` (call-view.tsx). The generic runner is private to this file; a control cannot reach the adapter around it.
// `runWrite` is the same logic without React (its `deps` are the hook's), so the tests run it against the mock.
import { createContext, createElement, useCallback, useContext, useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { redirectIfUnauthenticated } from "../../../app/AuthGate";
import { apiStore, getAdapter, getRelay } from "../../../lib/api";
import type { RelayAdapter, PairInput } from "../../../lib/api/relay";
import type { ConsentRequired, PairOffer, PairOfferRequest, PairedHub, RelaySettingsPatch, RelayStatus } from "../../../lib/api/types";
import { ConsentPanel, useConsentFlow, type ConsentOutcome } from "../../messages/net/consent";
import { dismissOffer } from "../messaging-actions";
import { problemText } from "../model";

/** The relay slice the page draws. The first use asks for it once (a one-shot GET, no poll); a `relay` frame keeps it current. */
export function useRelayStatus(): RelayStatus | null {
  const get = () => apiStore.getState().relay;
  const v = useSyncExternalStore(apiStore.subscribe, get, get);
  useEffect(() => {
    void apiStore.refreshRelay();
  }, []);
  return v;
}

export interface CallState {
  pending: boolean;
  error: string | null;
  /** A short line after a call went through, for a polite status. */
  note: string | null;
}

export const IDLE: CallState = { pending: false, error: null, note: null };

// --- the calls ---------------------------------------------------------------------------------------------------------------

/** What a write needs from its surroundings besides the relay adapter: the Layer 5 dismissal and a fresh read of the fabric slice. */
export interface WriteDeps {
  /** `attempt(call)`: the consent round trip around `call` (the shared consent flow; a test scripts the person's answers). */
  attempt<T>(call: (hash?: string) => Promise<T>): Promise<ConsentOutcome<T>>;
  relay(): Promise<RelayAdapter>;
  /** The call's state as the control shows it. */
  set(state: CallState): void;
  /** A call went through: read the relay slice again (the `relay` frame may not come). */
  refreshRelay(): void;
  /** A pairing code was minted: the fabric slice's `hub.offer_active` moved. */
  refreshFabric(): void;
  /** `DELETE /fabric/hub/offer`, the Layer 5 route a pairing code is dismissed with (settings/messaging-actions.ts). */
  dismissOffer(): Promise<void>;
  /** A 401 sends the browser to /pair: true when it did. */
  unauthenticated(err: unknown): boolean;
}

/** One write: `hash` is set only on the repeat after the person read a consent text. */
export type Write<T> = (relay: RelayAdapter, hash: string | undefined, deps: WriteDeps) => Promise<T>;

export interface WriteOpts<T> {
  done?: string | ((value: T) => string);
  errorText?: (e: unknown) => string;
}

/** `POST /relay/pair`: the repeat after a `pair_plain` consent carries the hash. */
export const pairWith = (relay: RelayAdapter, body: PairInput, hash?: string): Promise<PairedHub> => relay.pair(hash === undefined ? body : { ...body, accept_warning: hash });

/**
 * Check or flush the carrier. These two routes take no hash (they answer 202 or 409 `consent_required`); the home of the
 * `carrier_enable` acceptance is `PATCH /relay/settings`: a patch whose `accept_warning` is the hash of the text owed now for the carrier the
 * node holds records that consent even when it changes no stored value. So a hash, when there is one, is accepted through that write and
 * the call is made again. A stale or foreign hash is refused 409 with the current text, which the round trip then shows.
 */
export async function carrierCall(relay: RelayAdapter, which: "check" | "flush", carrier: Pick<RelayStatus["carrier"], "kind" | "server">, hash?: string): Promise<void> {
  if (hash !== undefined) await relay.patchSettings({ carrier: { kind: carrier.kind, server: carrier.server } }, hash);
  await (which === "check" ? relay.checkCarrier() : relay.flushCarrier());
}

/** Every write the page makes. Each one hands `hash` to the adapter call it makes. */
export const WRITES = {
  /** `PATCH /relay/settings`. */
  patch: (patch: RelaySettingsPatch): Write<RelayStatus> => (relay, hash) => relay.patchSettings(patch, hash),
  /** `POST /relay/pair/offer`: mints a code, so the fabric slice's `offer_active` is read again. */
  pairOffer: (body: PairOfferRequest): Write<PairOffer> => async (relay, _hash, deps) => {
    const offer = await relay.pairOffer(body);
    deps.refreshFabric();
    return offer;
  },
  pair: (body: PairInput): Write<PairedHub> => (relay, hash) => pairWith(relay, body, hash),
  unpair: (nodeId: string): Write<void> => (relay) => relay.unpair(nodeId),
  carrier: (which: "check" | "flush", carrier: Pick<RelayStatus["carrier"], "kind" | "server">): Write<void> => (relay, hash) => carrierCall(relay, which, carrier, hash),
  /** `DELETE /fabric/hub/offer`. */
  dismissOffer: (): Write<void> => (_relay, _hash, deps) => deps.dismissOffer(),
};

/** How a write ended: it went through with this value, or it did not (the person cancelled, or it failed and the state says how). */
export type Ran<T> = { ok: true; value: T } | { ok: false };

/**
 * Run one write with the consent round trip around it, and set the control's state as it goes: pending while it runs, idle again when the
 * person cancelled, the node's own words when it failed. It never leaves a control pending: every path ends in `set(...)`.
 */
export async function runWrite<T>(deps: WriteDeps, write: Write<T>, opts: WriteOpts<T> = {}): Promise<Ran<T>> {
  deps.set({ pending: true, error: null, note: null });
  try {
    const relay = await deps.relay();
    const out = await deps.attempt((hash) => write(relay, hash, deps));
    if (out.ok) {
      deps.set({ pending: false, error: null, note: typeof opts.done === "function" ? opts.done(out.value) : (opts.done ?? null) });
      deps.refreshRelay();
      return { ok: true, value: out.value };
    }
    if (out.cancelled) {
      deps.set(IDLE);
      return { ok: false };
    }
    throw out.error;
  } catch (err) {
    if (!deps.unauthenticated(err)) deps.set({ pending: false, error: (opts.errorText ?? problemText)(err), note: null });
    return { ok: false };
  }
}

// --- the helper ---------------------------------------------------------------------------------------------------------------

export interface RelayCall {
  /** The consent panel when a write is waiting on one. Render it where the control is, through `<CallOutput>`. */
  dialog: ReactNode;
  state: CallState;
  patch(patch: RelaySettingsPatch, opts?: WriteOpts<RelayStatus>): Promise<Ran<RelayStatus>>;
  pairOffer(body: PairOfferRequest, opts?: WriteOpts<PairOffer>): Promise<Ran<PairOffer>>;
  pair(body: PairInput, opts?: WriteOpts<PairedHub>): Promise<Ran<PairedHub>>;
  unpair(nodeId: string, opts?: WriteOpts<void>): Promise<Ran<void>>;
  carrier(which: "check" | "flush", carrier: Pick<RelayStatus["carrier"], "kind" | "server">, opts?: WriteOpts<void>): Promise<Ran<void>>;
  /** `DELETE /fabric/hub/offer`: ok when the node holds no pairing code any more. */
  dismissOffer(opts?: WriteOpts<void>): Promise<Ran<void>>;
}

/**
 * Test seam. A statically rendered page cannot hold a promise open, so a test hands the helper a consent (and, if it wants, a call state)
 * to start in: the section then has to render the helper's own dialog for the text to appear. Nothing in production provides it.
 */
export const RelayCallSeed = createContext<{ consent?: ConsentRequired; state?: CallState } | null>(null);

export function useRelayCall(): RelayCall {
  const { dialog: live, attempt } = useConsentFlow();
  const seed = useContext(RelayCallSeed);
  const [state, setState] = useState<CallState>(IDLE);
  const alive = useRef(true);
  const running = useRef(false);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const run = useCallback(
    async <T,>(write: Write<T>, opts?: WriteOpts<T>): Promise<Ran<T>> => {
      if (running.current) return { ok: false }; // one write at a time per control
      running.current = true;
      try {
        return await runWrite<T>(
          {
            attempt,
            relay: getRelay,
            set: (s) => {
              if (alive.current) setState(s);
            },
            refreshRelay: () => void apiStore.refreshRelay(),
            refreshFabric: () => void apiStore.refreshFabric(),
            dismissOffer: async () => dismissOffer(await getAdapter(), apiStore),
            unauthenticated: redirectIfUnauthenticated,
          },
          write,
          opts,
        );
      } finally {
        running.current = false;
      }
    },
    [attempt],
  );
  const seeded = seed?.consent ? createElement(ConsentPanel, { consent: seed.consent, onConfirm: () => undefined, onCancel: () => undefined }) : null;
  return {
    dialog: live ?? seeded,
    state: seed?.state ?? state,
    patch: (patch, opts) => run(WRITES.patch(patch), opts),
    pairOffer: (body, opts) => run(WRITES.pairOffer(body), opts),
    pair: (body, opts) => run(WRITES.pair(body), opts),
    unpair: (nodeId, opts) => run(WRITES.unpair(nodeId), opts),
    carrier: (which, carrier, opts) => run(WRITES.carrier(which, carrier), opts),
    dismissOffer: (opts) => run(WRITES.dismissOffer(), opts),
  };
}
