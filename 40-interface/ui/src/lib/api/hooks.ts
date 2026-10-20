// Thin useSyncExternalStore wrappers over the app store. Selectors return slices of an immutable
// state object, so references are stable between unrelated changes.
import { useEffect, useSyncExternalStore } from "react";
import { apiStore, type Attention, type ConnectionSnapshot, type ForeignSlot, type StoreMeta, type StoreState, type ThreadSlot } from "./store";
import type { IgnoredRequests } from "./requests";
import type {
  AgentEvent,
  BridgeComponent,
  BridgeEdge,
  BridgeSettings,
  Card,
  Contact,
  DeviceRecord,
  FabricStatus,
  ForeignConversation,
  HostedPanel,
  NearbySource,
  Network,
  NetworkId,
  NodeId,
  Proposal,
  ServiceId,
  ServiceRecord,
  Status,
  Thread,
  TransportStatus,
} from "./types";

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

// --- Layer 5: messages, contacts, fabric, nearby, panels -----------------------------------------------------

/** Conversations, most recent activity first, message requests included (`request: true`); null until loaded. */
export const useThreads = (): Thread[] | null => useSlice((s) => s.threads);

/** Contacts, pending message requests included (`request: true`); null until loaded. */
export const useContacts = (): Contact[] | null => useSlice((s) => s.contacts);

/** Message requests the user ignored on this device (`peer -> newest received_seq`); they stay pending on the node. */
export const useIgnoredRequests = (): IgnoredRequests => useSlice((s) => s.ignored);

/** One contact by NodeId; null while loading or when unknown. */
export const useContact = (id: NodeId): Contact | null => useSlice((s) => s.contacts?.find((c) => c.node_id === id) ?? null);

const NO_THREAD: ThreadSlot = { state: "loading", page: null, error: null };

/**
 * One conversation. The first render for a peer starts its load (the newest 100 messages); the slot follows
 * `message` frames after that. `limit` raises the window ("show earlier messages", at most 500).
 */
export function useThread(peer: NodeId, limit?: number): ThreadSlot {
  const slot = useSlice((s) => s.threadPages[peer] ?? NO_THREAD);
  useEffect(() => {
    void apiStore.loadThread(peer, limit);
  }, [peer, limit]);
  return slot;
}

/** This node's own card; null until it is loaded (the first use asks for it). */
export function useOwnCard(): Card | null {
  const card = useSlice((s) => s.ownCard);
  useEffect(() => {
    if (!card) void apiStore.loadOwnCard();
  }, [card]);
  return card;
}

/** Links, home hub, custody, store and settings of the messaging fabric; null until loaded. */
export const useFabric = (): FabricStatus | null => useSlice((s) => s.fabric);

/** Presence per source (v1: Meshtastic nodes heard); null until loaded. Names are untrusted RF-origin strings. */
export const useNearby = (): NearbySource[] | null => useSlice((s) => s.nearby);

/** Declared hosted panels and their listener slots; null until loaded. */
export const usePanels = (): HostedPanel[] | null => useSlice((s) => s.panels);

// --- Layer 6: foreign networks, relay edges, components, transport ----------------------------------------------

/** The foreign networks in the preferred order (the rail and the pickers); null until loaded, and for a node older than Layer 6. */
export const useNetworks = (): Network[] | null => useSlice((s) => s.networks);

/** One network by id; null while loading or when unknown. */
export const useNetwork = (id: NetworkId): Network | null => useSlice((s) => s.networks?.find((n) => n.id === id) ?? null);

/**
 * One network's conversations, most recent first. The first render for a network starts the load; `undefined` until it
 * arrives (so a view can tell "not loaded" from "none").
 */
export function useNetworkConversations(net: NetworkId): ForeignConversation[] | undefined {
  const list = useSlice((s) => s.conversations[net]);
  useEffect(() => {
    void apiStore.refreshConversations(net);
  }, [net]);
  return list;
}

/** The conversations of several networks at once (the fan-out picker); a network not loaded yet is missing from the record. */
export function useConversationsOf(nets: readonly NetworkId[]): Record<NetworkId, ForeignConversation[]> {
  const all = useSlice((s) => s.conversations);
  const key = nets.join("\u0000");
  useEffect(() => {
    for (const n of key === "" ? [] : key.split("\u0000")) void apiStore.refreshConversations(n);
  }, [key]);
  return all;
}

const NO_FOREIGN: ForeignSlot = { state: "loading", network: "", page: null, error: null };

/** One foreign conversation by its `fc-…` id. The first render starts its load (the newest 100 rows); the slot follows `network_message` frames. */
export function useForeignThread(net: NetworkId, conv: string, limit?: number): ForeignSlot {
  const slot = useSlice((s) => s.foreignPages[conv] ?? NO_FOREIGN);
  useEffect(() => {
    void apiStore.loadForeign(net, conv, limit);
  }, [net, conv, limit]);
  return slot;
}

/** Relay edges; null until loaded. */
export const useEdges = (): BridgeEdge[] | null => useSlice((s) => s.edges);

/** state/bridge.json as the node mirrors it. The first use asks for it. */
export function useBridgeSettings(): BridgeSettings | null {
  const v = useSlice((s) => s.bridgeSettings);
  useEffect(() => {
    void apiStore.refreshBridgeSettings();
  }, []);
  return v;
}

/** Optional components. The first use asks for them. */
export function useComponents(): BridgeComponent[] | null {
  const v = useSlice((s) => s.components);
  useEffect(() => {
    void apiStore.refreshComponents();
  }, []);
  return v;
}

/** Interfaces, posture, region and callsign. The first use asks for them. */
export function useTransport(): TransportStatus | null {
  const v = useSlice((s) => s.transport);
  useEffect(() => {
    void apiStore.refreshTransport();
  }, []);
  return v;
}
