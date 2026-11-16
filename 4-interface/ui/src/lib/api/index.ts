// Adapter selection and the app-wide singleton.
//
//   ?adapter=mock   dev only: in-browser mock (fixtures + scripted events); ?scenario= picks one
//   anything else   the real daemon over same-origin fetch / EventSource
//
// The mock is reached ONLY through the dynamic import below, inside an `import.meta.env.DEV`
// branch. Vite replaces that constant with `false` in production builds, so the branch (and the
// mock chunk) is removed and fixtures never ship.
import type { Adapter } from "./adapter";
import { createHttpAdapter } from "./http";
import type { RelayAdapter } from "./relay";
import type { RepairsAdapter } from "./repairs";
import type { StorageAdapter } from "./storage";
import { apiStore } from "./store";

export type { Adapter } from "./adapter";
export type { RelayAdapter } from "./relay";
export type { RepairsAdapter } from "./repairs";
export type { StorageAdapter } from "./storage";
export { ApiError, isApiError, isProblem } from "./problem";
export { apiStore } from "./store";

export async function selectAdapter(search: string): Promise<Adapter> {
  if (import.meta.env.DEV) {
    const q = new URLSearchParams(search);
    if (q.get("adapter") === "mock") {
      const { createMockAdapter, resolveScenario } = await import("./mock");
      return createMockAdapter({ scenario: resolveScenario(q.get("scenario")) });
    }
  }
  return createHttpAdapter();
}

let singleton: Promise<Adapter> | null = null;

/** The adapter for this page load (memoized). Components call this for actions and chat only. */
export function getAdapter(): Promise<Adapter> {
  singleton ??= selectAdapter(typeof location === "undefined" ? "" : location.search);
  return singleton;
}

let relaySingleton: Promise<RelayAdapter> | null = null;

/**
 * The Layer 7 adapter for this page load (memoized). A separate seam from `getAdapter` so the Layer 7 code stays out of the eager chunk:
 * `./relay` is imported here, dynamically and only when a Layer 7 view (or the store, after a `relay` frame) asks, and the mock's
 * relay is reached only through the mock adapter, inside the same `import.meta.env.DEV` guard as the mock itself.
 */
export function getRelay(): Promise<RelayAdapter> {
  relaySingleton ??= (async () => {
    const adapter = await getAdapter();
    const mockRelay = (adapter as { mock?: { relay?: RelayAdapter } }).mock?.relay;
    if (adapter.kind === "mock" && mockRelay) return mockRelay;
    const { createHttpRelay } = await import("./relay");
    return createHttpRelay();
  })();
  return relaySingleton;
}

let storageSingleton: Promise<StorageAdapter> | null = null;

/**
 * The Layer 8 adapter for this page load (memoized), the `getRelay` seam again: `./storage` is imported here, dynamically and only when a
 * Layer 8 view (or the store, after a `storage` frame) asks, and the mock's storage is reached only through the mock adapter, inside the
 * same `import.meta.env.DEV` guard as the mock itself. Its size counts in Layer 8's lazy allowance (os/8-storage/README.md §15.4).
 */
export function getStorage(): Promise<StorageAdapter> {
  storageSingleton ??= (async () => {
    const adapter = await getAdapter();
    const mockStorage = (adapter as { mock?: { storage?: StorageAdapter } }).mock?.storage;
    if (adapter.kind === "mock" && mockStorage) return mockStorage;
    const { createHttpStorage } = await import("./storage");
    return createHttpStorage();
  })();
  return storageSingleton;
}

let repairsSingleton: Promise<RepairsAdapter> | null = null;

/**
 * The Layer 9 adapter for this page load (memoized), the `getStorage` seam again: `./repairs` is imported here, dynamically and only when
 * a Layer 9 view (or the store, after an `incident` frame) asks, and the mock's repairs are reached only through the mock adapter, inside
 * the same `import.meta.env.DEV` guard as the mock itself. Its size counts in Layer 9's lazy allowance (os/9-evolution/README.md §14).
 */
export function getRepairs(): Promise<RepairsAdapter> {
  repairsSingleton ??= (async () => {
    const adapter = await getAdapter();
    const mockRepairs = (adapter as { mock?: { repairs?: RepairsAdapter } }).mock?.repairs;
    if (adapter.kind === "mock" && mockRepairs) return mockRepairs;
    const { createHttpRepairs } = await import("./repairs");
    return createHttpRepairs();
  })();
  return repairsSingleton;
}

/** Test seam: forget the memoized adapters. */
export function resetAdapterForTests(): void {
  singleton = null;
  relaySingleton = null;
  storageSingleton = null;
  repairsSingleton = null;
}

/** Boot helper for main.tsx: pick the adapter, hand it to the store, return it. */
export async function startApi(): Promise<Adapter> {
  const adapter = await getAdapter();
  apiStore.attach(adapter);
  apiStore.attachRelay(getRelay); // after attach: `detach` forgets the loader, and the store calls it only once a Layer 7 slice is loaded
  apiStore.attachStorage(getStorage); // the same for Layer 8: called after a `storage` frame, or to re-read a loaded slice
  apiStore.attachRepairs(getRepairs); // and for Layer 9: called after an `incident` frame, or to re-read a loaded slice
  return adapter;
}
