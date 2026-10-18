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
import { apiStore } from "./store";

export type { Adapter } from "./adapter";
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

/** Test seam: forget the memoized adapter. */
export function resetAdapterForTests(): void {
  singleton = null;
}

/** Boot helper for main.tsx: pick the adapter, hand it to the store, return it. */
export async function startApi(): Promise<Adapter> {
  const adapter = await getAdapter();
  apiStore.attach(adapter);
  return adapter;
}
