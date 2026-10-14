// The store's Layer 5 slices: snapshots, frames (message, contact, fabric, nearby, panels), attention, conversations.
import { describe, expect, it, vi } from "vitest";
import type { Adapter, EventsFrameWithId, EventsHandlers } from "../src/lib/api/adapter";
import { buildFixtures } from "../src/lib/api/mock/fixtures";
import { ADA, DOV, ELI, buildNearby, buildPanels, makeContact, pageOf, threadsOf } from "../src/lib/api/mock/messaging";
import { ApiError } from "../src/lib/api/problem";
import { ApiStore, computeAttention } from "../src/lib/api/store";
import type { Contact, FabricStatus, Message, Thread } from "../src/lib/api/types";
import { flush } from "./helpers/fakes";

const NOW = Date.parse("2026-10-14T09:00:00Z");
const fx = buildFixtures(NOW);
const M = fx.messaging;
const clone = <T,>(v: T): T => structuredClone(v);

const msg = (over: Partial<Message> = {}): Message => ({
  id: "msg-new",
  peer: ADA,
  dir: "in",
  text: "new",
  state: "delivered",
  reason_code: null,
  reason: null,
  local_seq: 100,
  received_seq: 50,
  sent_at_claim: null,
  at: "2026-10-14T09:00:00Z",
  ...over,
});

function adapterWith(over: Partial<Record<"threads" | "contacts" | "fabric" | "nearby" | "panels" | "thread", (...a: never[]) => Promise<unknown>>> = {}) {
  const handlers: { current: EventsHandlers | null } = { current: null };
  const calls = { threads: 0, contacts: 0, fabric: 0, nearby: 0, panels: 0, thread: [] as { peer: string; limit?: number }[] };
  const adapter = {
    kind: "http",
    status: async () => clone(fx.status),
    services: async () => clone(fx.services),
    devices: async () => clone(fx.devices),
    proposals: async () => clone(fx.proposals),
    messages: {
      threads: vi.fn(async () => {
        calls.threads++;
        return (over.threads ? over.threads() : threadsOf(clone(M))) as Thread[];
      }),
      thread: vi.fn(async (peer: string, q: { limit?: number } = {}) => {
        calls.thread.push({ peer, ...(q.limit !== undefined ? { limit: q.limit } : {}) });
        if (over.thread) return over.thread();
        const page = pageOf(clone(M), peer, undefined, q.limit ?? 100);
        return { peer, request: M.contacts.find((c) => c.node_id === peer)?.request === true, ...page };
      }),
    },
    contacts: {
      list: vi.fn(async () => {
        calls.contacts++;
        return (over.contacts ? over.contacts() : clone(M.contacts)) as Contact[];
      }),
      ownCard: vi.fn(async () => clone(M.card)),
    },
    fabric: {
      status: vi.fn(async () => {
        calls.fabric++;
        return (over.fabric ? over.fabric() : clone(M.fabric)) as FabricStatus;
      }),
    },
    nearby: vi.fn(async () => {
      calls.nearby++;
      return over.nearby ? over.nearby() : buildNearby(NOW);
    }),
    panels: {
      list: vi.fn(async () => {
        calls.panels++;
        return over.panels ? over.panels() : buildPanels(true);
      }),
    },
    events: vi.fn((h: EventsHandlers) => {
      handlers.current = h;
      return () => undefined;
    }),
  } as unknown as Adapter;
  const frame = (f: Omit<EventsFrameWithId, "id"> & { id?: string }) => handlers.current!.onFrame({ id: String(++seq), ...f } as EventsFrameWithId);
  let seq = 0;
  return { adapter, calls, frame, handlers };
}

async function started(over: Parameters<typeof adapterWith>[0] = {}) {
  const store = new ApiStore({ now: () => 1_000 });
  const a = adapterWith(over);
  store.attach(a.adapter);
  await flush();
  a.handlers.current!.onState("open", { lastEventAt: 1 });
  return { store, ...a };
}

describe("snapshots", () => {
  it("loads threads, contacts, fabric, nearby and panels with the core slices", async () => {
    const { store } = await started();
    const s = store.getState();
    expect(s.threads?.length).toBe(threadsOf(M).length);
    expect(s.contacts?.map((c) => c.node_id)).toEqual(M.contacts.map((c) => c.node_id));
    expect(s.fabric?.hub.enrolled).toBe(true);
    expect(s.nearby?.[0]?.nodes).toHaveLength(3);
    expect(s.panels?.[0]?.id).toBe("meshtastic");
    expect(s.meta).toEqual({ ready: true, needsPair: false, error: null });
  });

  it("a daemon without Layer 5 (the L5 reads fail) leaves the slices empty and the rest working", async () => {
    const down = () => Promise.reject(new ApiError({ type: "about:blank", title: "Not found", status: 404, code: "not_found" }));
    const { store } = await started({ threads: down, contacts: down, fabric: down, nearby: down, panels: down });
    const s = store.getState();
    expect(s.threads).toBeNull();
    expect(s.contacts).toBeNull();
    expect(s.fabric).toBeNull();
    expect(s.status).not.toBeNull();
    expect(s.services).toHaveLength(9);
    expect(s.meta).toEqual({ ready: true, needsPair: false, error: null });
  });

  it("an unpaired visitor is told to pair by a Layer 5 read alone", async () => {
    const unauth = () => Promise.reject(new ApiError({ type: "about:blank", title: "Pair", status: 401, code: "unauthenticated" }));
    const { store } = await started({ threads: unauth });
    expect(store.getState().meta.needsPair).toBe(true);
  });

  it("keeps record identity across a reload when nothing changed", async () => {
    const { store } = await started();
    const before = store.getState();
    await store.refreshAll();
    const after = store.getState();
    expect(after.threads?.[0]).toBe(before.threads?.[0]);
    expect(after.contacts?.[0]).toBe(before.contacts?.[0]);
    expect(after.fabric).toBe(before.fabric);
    expect(after.nearby).toBe(before.nearby);
  });
});

describe("conversations", () => {
  it("loads one on demand and marks it ready", async () => {
    const { store, calls } = await started();
    await store.loadThread(ADA);
    const slot = store.getState().threadPages[ADA]!;
    expect(slot.state).toBe("ready");
    expect(slot.page?.messages.map((m) => m.local_seq)).toEqual([1, 2, 3, 4, 5, 9]);
    expect(calls.thread[0]).toEqual({ peer: ADA, limit: 100 });
  });

  it("reads 404 as 'missing' and any other failure as 'error', keeping a page it already had", async () => {
    const { store } = await started({ thread: () => Promise.reject(new ApiError({ type: "about:blank", title: "No", status: 404, code: "not_found" })) });
    await store.loadThread(DOV);
    expect(store.getState().threadPages[DOV]).toMatchObject({ state: "missing", page: null });

    const ok = await started();
    await ok.store.loadThread(ADA);
    const had = ok.store.getState().threadPages[ADA]!.page;
    (ok.adapter.messages.thread as ReturnType<typeof vi.fn>).mockRejectedValueOnce(ApiError.network(new TypeError("x")));
    await ok.store.loadThread(ADA);
    expect(ok.store.getState().threadPages[ADA]).toMatchObject({ state: "ready", page: had });
    expect(ok.store.getState().threadPages[ADA]!.error?.code).toBe("network");
  });

  it("an open conversation follows a reload with the window it had", async () => {
    const { store, calls } = await started();
    await store.loadThread(ADA, 3);
    calls.thread.length = 0;
    await store.refreshAll();
    expect(calls.thread).toEqual([{ peer: ADA, limit: 3 }]);
  });

  it("takes a message frame into an open conversation at once, and reloads the list", async () => {
    const { store, frame, calls } = await started();
    await store.loadThread(ADA);
    const before = calls.threads;
    frame({ event: "message", data: { change: "new", peer: ADA, message: msg({ id: "m-new", local_seq: 100 }) } });
    const page = store.getState().threadPages[ADA]!.page!;
    expect(page.messages[page.messages.length - 1]?.id).toBe("m-new");
    await flush();
    expect(calls.threads).toBe(before + 1);
  });

  it("a state frame changes the message in place", async () => {
    const { store, frame } = await started();
    await store.loadThread(ADA);
    const own = store.getState().threadPages[ADA]!.page!.messages.find((m) => m.dir === "own" && m.state === "sent")!;
    frame({ event: "message", data: { change: "state", peer: ADA, message: { ...own, state: "delivered" } } });
    const after = store.getState().threadPages[ADA]!.page!.messages;
    expect(after.find((m) => m.id === own.id)?.state).toBe("delivered");
    expect(after).toHaveLength(6);
  });

  it("does not invent a conversation that was never opened", async () => {
    const { store, frame } = await started();
    frame({ event: "message", data: { change: "new", peer: ELI, message: msg({ peer: ELI }) } });
    expect(store.getState().threadPages[ELI]).toBeUndefined();
  });

  it("ignores malformed message and contact frames", async () => {
    const { store, frame } = await started();
    const before = store.getState();
    expect(() => {
      frame({ event: "message", data: {} as never });
      frame({ event: "message", data: { peer: ADA, message: {} } as never });
      frame({ event: "contact", data: { change: "added" } as never });
      frame({ event: "fabric", data: {} as never });
      frame({ event: "nearby", data: { sources: "x" } as never });
      frame({ event: "panels", data: null as never });
    }).not.toThrow();
    expect(store.getState().contacts).toBe(before.contacts);
    expect(store.getState().fabric).toBe(before.fabric);
    expect(store.getState().nearby).toBe(before.nearby);
    expect(store.getState().panels).toBe(before.panels);
  });
});

describe("requests and attention", () => {
  const request = makeContact(DOV, { trust: "tofu", request: true });

  it("counts open message requests toward the badge", () => {
    expect(computeAttention([], [], [], [makeContact(ADA), request]).openRequests).toBe(1);
    expect(computeAttention([], [], [], [makeContact(ADA), request])).toMatchObject({ count: 1, openProposals: 0, erroredServices: 0, erroredDevices: 0, openRequests: 1 });
    expect(computeAttention([], [], [], null).openRequests).toBe(0);
    expect(computeAttention([], [], [], [makeContact(ADA)]).count).toBe(0);
  });

  it("a request frame raises the badge; accepting it (an updated frame) lowers it", async () => {
    const { store, frame } = await started();
    expect(store.getState().attention.openRequests).toBe(0);
    frame({ event: "contact", data: { change: "request", contact: request } });
    expect(store.getState().contacts?.some((c) => c.node_id === DOV && c.request)).toBe(true);
    expect(store.getState().attention).toMatchObject({ openRequests: 1, count: store.getState().attention.openProposals + 1 });
    frame({ event: "contact", data: { change: "updated", contact: { ...request, request: false } } });
    expect(store.getState().attention.openRequests).toBe(0);
    expect(store.getState().contacts?.filter((c) => c.node_id === DOV)).toHaveLength(1);
  });

  it("discarding a request (a removed frame) clears it", async () => {
    const { store, frame } = await started();
    frame({ event: "contact", data: { change: "request", contact: request } });
    frame({ event: "contact", data: { change: "removed", contact: request } });
    expect(store.getState().contacts?.some((c) => c.node_id === DOV)).toBe(false);
    expect(store.getState().attention.openRequests).toBe(0);
  });

  it("accepting a request updates an open conversation's read-only flag", async () => {
    const { store, frame } = await started();
    frame({ event: "contact", data: { change: "request", contact: request } });
    frame({ event: "message", data: { change: "request", peer: DOV, message: msg({ peer: DOV, id: "m1", local_seq: 101 }) } });
    await store.loadThread(DOV);
    expect(store.getState().threadPages[DOV]?.page?.request).toBe(false); // the fake page reads the fixture, not the frame
    // The slot follows contact frames either way.
    frame({ event: "contact", data: { change: "updated", contact: { ...request, request: true } } });
    expect(store.getState().threadPages[DOV]?.page?.request).toBe(true);
    frame({ event: "contact", data: { change: "updated", contact: { ...request, request: false } } });
    expect(store.getState().threadPages[DOV]?.page?.request).toBe(false);
  });
});

describe("fabric, nearby and panels frames", () => {
  it("replaces the fabric snapshot and keeps the object when it did not change", async () => {
    const { store, frame } = await started();
    const before = store.getState().fabric;
    frame({ event: "fabric", data: clone(before!) });
    expect(store.getState().fabric).toBe(before);
    const full = clone(before!);
    full.store.store_full = true;
    frame({ event: "fabric", data: full });
    expect(store.getState().fabric?.store.store_full).toBe(true);
  });

  it("replaces the nearby sources, and keeps them when nothing changed", async () => {
    const { store, frame } = await started();
    const before = store.getState().nearby;
    frame({ event: "nearby", data: { sources: clone(before!) } });
    expect(store.getState().nearby).toBe(before);
    frame({ event: "nearby", data: { sources: [] } });
    expect(store.getState().nearby).toEqual([]);
  });

  it("replaces the panels, keeping unchanged records", async () => {
    const { store, frame } = await started();
    const before = store.getState().panels;
    frame({ event: "panels", data: { panels: clone(before!) } });
    expect(store.getState().panels![0]).toBe(before![0]);
    frame({ event: "panels", data: { panels: buildPanels(false) } });
    expect(store.getState().panels![0]!.state.listening).toBe(false);
  });
});

describe("own card and reset", () => {
  it("loads the own card once", async () => {
    const { store, adapter } = await started();
    await store.loadOwnCard();
    await store.loadOwnCard();
    expect(store.getState().ownCard?.text).toBe(M.card.text);
    expect(adapter.contacts.ownCard).toHaveBeenCalledTimes(1);
  });

  it("detach and reset forget every Layer 5 slice", async () => {
    const { store } = await started();
    await store.loadThread(ADA);
    store.detach();
    store.reset();
    const s = store.getState();
    expect(s.threads).toBeNull();
    expect(s.contacts).toBeNull();
    expect(s.fabric).toBeNull();
    expect(s.nearby).toBeNull();
    expect(s.panels).toBeNull();
    expect(s.threadPages).toEqual({});
    expect(s.ownCard).toBeNull();
  });

  it("a reset frame after the first load reloads everything", async () => {
    const { frame, calls } = await started();
    await flush();
    const before = { ...calls };
    frame({ event: "reset", data: {} as never });
    await flush();
    expect(calls.threads).toBeGreaterThan(before.threads);
    expect(calls.contacts).toBeGreaterThan(before.contacts);
    expect(calls.fabric).toBeGreaterThan(before.fabric);
  });
});
