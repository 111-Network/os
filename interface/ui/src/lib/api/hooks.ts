// Thin useSyncExternalStore wrappers over the app store. Selectors return slices of an immutable
// state object, so references are stable between unrelated changes.
import { useSyncExternalStore } from "react";
import { apiStore, type Attention, type ConnectionSnapshot, type StoreMeta, type StoreState } from "./store";
import type { AgentEvent, DeviceRecord, Proposal, ServiceId, ServiceRecord, Status } from "./types";

function useSlice<T>(select: (s: StoreState) => T): T {
  const snapshot = () => select(apiStore.getState());
  // The third argument keeps the hooks usable under react-dom/server (and in tests without a DOM).
  return useSyncExternalStore(apiStore.subscribe, snapshot, snapshot);
}

/** null until the first snapshot arrives. */
export const useStatus = (): Status | null => useSlice((s) => s.status);

/** null until the first snapshot arrives. */
export const useServices = (): ServiceRecord[] | null => useSlice((s) => s.services);

/** null while loading or when the id is unknown. */
export const useService = (id: ServiceId): ServiceRecord | null =>
  useSlice((s) => s.services?.find((x) => x.id === id) ?? null);

/** The device registry (hardware and paired devices); null until the first snapshot arrives. */
export const useDevices = (): DeviceRecord[] | null => useSlice((s) => s.devices);

/** null while loading or when the id is unknown (for example after Forget). */
export const useDevice = (id: string): DeviceRecord | null => useSlice((s) => s.devices?.find((x) => x.id === id) ?? null);

/** Open and decided proposals, newest first; null until loaded. Filter on `decision === null` for open. */
export const useProposals = (): Proposal[] | null => useSlice((s) => s.proposals);

export const useConnection = (): ConnectionSnapshot => useSlice((s) => s.connection);

/** Moves every time a fresh service list arrives (stream frame or snapshot). */
export const useServicesRev = (): number => useSlice((s) => s.servicesRev);

/** Agent events, newest first (ring of 200). */
export const useActivity = (): AgentEvent[] => useSlice((s) => s.activity);

export const useAttention = (): Attention => useSlice((s) => s.attention);

/** True once the node reports a different version than the one this page load started with. */
export const useVersionChanged = (): boolean => useSlice((s) => s.versionChanged);

/** ready / needsPair / last snapshot error. */
export const useApiMeta = (): StoreMeta => useSlice((s) => s.meta);
