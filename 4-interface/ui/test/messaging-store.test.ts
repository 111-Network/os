// The store's Layer 5 slices: snapshots, frames (message, contact, fabric, nearby, panels), attention, conversations.
import { describe, expect, it, vi } from "vitest";
import type { Adapter, EventsFrameWithId, EventsHandlers } from "../src/lib/api/adapter";
import { buildFixtures } from "../src/lib/api/mock/fixtures";
import { ADA, DOV, ELI, buildNearby, buildPanels, makeContact, pageOf, threadsOf } from "../src/lib/api/mock/messaging";
import { ApiError } from "../src/lib/api/problem";
import { IGNORED_STORAGE_KEY, NO_IGNORED, type KeyValueStorage } from "../src/lib/api/requests";
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
      // Ignoring a request must never reach these (section 7.2: kept, no ACK); the tests assert they stay untouched.
      patch: vi.fn(async () => ({})),
      forget: vi.fn(async () => undefined),
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

/** A Web Storage stand-in that keeps what is written to it. */
function memory(initial: Record<string, string> = {}): KeyValueStorage & { data: Record<string, string> } {
  const data = { ...initial };
  return {
    data,
    getItem: (k) => (k in data ? data[k]! : null),
    setItem: (k, v) => {
      data[k] = v;
    },
    removeItem: (k) => {
      delete data[k];
    },
  };
}

async function started(over: Parameters<typeof adapterWith>[0] = {}, deps: { storage?: KeyValueStorage | null } = {}) {
  const store = new ApiStore({ now: () => 1_000, ...deps });
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

describe("ignoring a request is a fold on this device, not a call to the node (section 7.2)", () => {
  const request = makeContact(DOV, { trust: "tofu", request: true });
  const held = (peer: string, seq: number): Message => msg({ id: `held-${peer.slice(0, 4)}-${seq}`, peer, local_seq: 200 + seq, received_seq: seq });
  const requestThread = (seq: number): Thread => ({
    peer: DOV,
    alias: null,
    label: request.label,
    request: true,
    unread: 1,
    last: held(DOV, seq),
    states: { queued: 0, sent: 0, relayed: 0, delivered: 0 },
  });

  /** A node with one pending request (newest receipt `seq`), whose answers the test can change. */
  async function withRequest(seq = 7, storage: KeyValueStorage | null = memory()) {
    const node = { contacts: [makeContact(ADA), request] as Contact[], threads: [requestThread(seq)] as Thread[] };
    const r = await started({ contacts: async () => clone(node.contacts), threads: async () => clone(node.threads) }, { storage });
    return { ...r, node, storage };
  }

  it("keeps the request pending on the node, asks it for nothing, and takes it out of the attention count", async () => {
    const { store, adapter } = await withRequest();
    expect(store.getState().attention.openRequests).toBe(1);
    store.ignoreRequest(DOV, 7);
    expect(adapter.contacts.forget).not.toHaveBeenCalled();
    expect(adapter.contacts.patch).not.toHaveBeenCalled();
    const s = store.getState();
    expect(s.contacts?.some((c) => c.node_id === DOV && c.request)).toBe(true); // still pending
    expect(s.threads?.some((t) => t.peer === DOV && t.request)).toBe(true);
    expect(s.ignored).toEqual({ [DOV]: 7 });
    expect(s.attention).toMatchObject({ openRequests: 0 });
    expect(s.attention.count).toBe(s.attention.openProposals + s.attention.erroredServices + s.attention.erroredDevices);
  });

  it("brings the request back when its sender writes again (a newer received_seq), and only then", async () => {
    const { store, node, frame } = await withRequest(7);
    store.ignoreRequest(DOV, 7);
    // The list reloads with nothing new (a reload for another reason): still folded.
    await store.refreshThreads();
    expect(store.getState().ignored).toEqual({ [DOV]: 7 });
    expect(store.getState().attention.openRequests).toBe(0);
    // A message frame alone does not decide: the reloaded list does.
    node.threads = [requestThread(8)];
    frame({ event: "message", data: { change: "request", peer: DOV, message: held(DOV, 8) } });
    await flush();
    expect(store.getState().ignored).toEqual({});
    expect(store.getState().attention.openRequests).toBe(1);
  });

  it("is not lifted by activity from someone else", async () => {
    const { store, node } = await withRequest(7);
    store.ignoreRequest(DOV, 7);
    node.threads = [requestThread(7), { ...requestThread(40), peer: ELI, label: "other" }];
    await store.refreshThreads();
    expect(store.getState().ignored).toEqual({ [DOV]: 7 });
  });

  it("is dropped once the request is accepted or forgotten, so a fresh INTRO later starts visible", async () => {
    const accepted = await withRequest();
    accepted.store.ignoreRequest(DOV, 7);
    accepted.frame({ event: "contact", data: { change: "updated", contact: { ...request, request: false } } });
    expect(accepted.store.getState().ignored).toEqual({});

    const forgotten = await withRequest();
    forgotten.store.ignoreRequest(DOV, 7);
    forgotten.frame({ event: "contact", data: { change: "removed", contact: request } });
    expect(forgotten.store.getState().ignored).toEqual({});
    forgotten.frame({ event: "contact", data: { change: "request", contact: request } });
    expect(forgotten.store.getState().attention.openRequests).toBe(1);
  });

  it("'show again' puts it back in the count, with no call to the node either", async () => {
    const { store, adapter } = await withRequest();
    store.ignoreRequest(DOV, 7);
    store.restoreRequest(DOV);
    expect(store.getState().ignored).toEqual({});
    expect(store.getState().attention.openRequests).toBe(1);
    store.restoreRequest(DOV); // nothing to restore: no change, no throw
    expect(adapter.contacts.forget).not.toHaveBeenCalled();
    expect(adapter.contacts.patch).not.toHaveBeenCalled();
  });

  it("is per device: written to storage, read back by the next page load, and cleared from it with the entry", async () => {
    const storage = memory();
    const first = await withRequest(7, storage);
    first.store.ignoreRequest(DOV, 7);
    expect(JSON.parse(storage.data[IGNORED_STORAGE_KEY]!)).toEqual({ [DOV]: 7 });

    const next = await withRequest(7, storage); // a new page load against the same node
    expect(next.store.getState().ignored).toEqual({ [DOV]: 7 });
    expect(next.store.getState().attention.openRequests).toBe(0);

    next.store.restoreRequest(DOV);
    expect(IGNORED_STORAGE_KEY in storage.data).toBe(false);
  });

  it("survives a reset (the fold is on the device, not in the page state), and a page without storage still works", async () => {
    const storage = memory();
    const { store } = await withRequest(7, storage);
    store.ignoreRequest(DOV, 7);
    store.detach();
    store.reset();
    expect(store.getState().ignored).toEqual({ [DOV]: 7 });
    const bare = await withRequest(7, null);
    bare.store.ignoreRequest(DOV, 7);
    expect(bare.store.getState().attention.openRequests).toBe(0);
  });

  it("does not hide a request whose thread is not loaded: not knowing is no reason to hide", () => {
    const ignored = { [DOV]: 7 };
    const contacts = [makeContact(ADA), request];
    expect(computeAttention([], [], [], contacts, ignored, null).openRequests).toBe(1);
    expect(computeAttention([], [], [], contacts, ignored, []).openRequests).toBe(1);
    expect(computeAttention([], [], [], contacts, ignored, [requestThread(7)]).openRequests).toBe(0);
    expect(computeAttention([], [], [], contacts, ignored, [requestThread(8)]).openRequests).toBe(1);
    expect(computeAttention([], [], [], contacts, NO_IGNORED, [requestThread(7)]).openRequests).toBe(1);
  });
});

describe("a conversation never steps back in time", () => {
  /** Hand-resolved answers to `GET /messages/threads/{peer}`, in the order they were asked for. */
  function manual() {
    const waiting: ((page: unknown) => void)[] = [];
    return { waiting, thread: () => new Promise((resolve) => waiting.push(resolve)) };
  }
  const base = () => ({ peer: ADA, request: false, ...pageOf(clone(M), ADA, undefined, 100) });
  const sentMsg = (state: Message["state"]): Message => msg({ id: "msg-just-sent", dir: "own", local_seq: 500, received_seq: null, state, text: "just sent" });

  it("an older response that arrives last does not replace a newer one", async () => {
    const { waiting, thread } = manual();
    const { store } = await started({ thread: thread as never });
    const l0 = store.loadThread(ADA); // asked before the send
    const l1 = store.loadThread(ADA); // asked after it
    waiting[1]!({ ...base(), messages: [...base().messages, sentMsg("queued")] });
    await l1;
    expect(store.getState().threadPages[ADA]!.page!.messages.some((m) => m.id === "msg-just-sent")).toBe(true);
    waiting[0]!(base()); // the old answer, without the message
    await l0;
    expect(store.getState().threadPages[ADA]!.page!.messages.some((m) => m.id === "msg-just-sent")).toBe(true);
  });

  it("an older answer cannot lower a state (delivered stays delivered)", async () => {
    const { waiting, thread } = manual();
    const { store } = await started({ thread: thread as never });
    const l0 = store.loadThread(ADA);
    const l1 = store.loadThread(ADA);
    waiting[1]!({ ...base(), messages: [...base().messages, sentMsg("delivered")] });
    await l1;
    waiting[0]!({ ...base(), messages: [...base().messages, sentMsg("sent")] });
    await l0;
    expect(store.getState().threadPages[ADA]!.page!.messages.find((m) => m.id === "msg-just-sent")?.state).toBe("delivered");
  });

  it("a message frame that is newer than the snapshot a pending load carries is kept", async () => {
    const { waiting, thread } = manual();
    const { store, frame } = await started({ thread: thread as never });
    const first = store.loadThread(ADA);
    waiting[0]!(base());
    await first;
    const reload = store.loadThread(ADA); // the snapshot below was taken before the frame
    frame({ event: "message", data: { change: "new", peer: ADA, message: sentMsg("queued") } });
    waiting[1]!(base());
    await reload;
    const page = store.getState().threadPages[ADA]!.page!;
    expect(page.messages.map((m) => m.id)).toContain("msg-just-sent");
    expect(page.messages.map((m) => m.local_seq)).toEqual([...page.messages.map((m) => m.local_seq)].sort((a, b) => a - b));
  });

  it("an answer from before a reset is not mistaken for a current one", async () => {
    const { waiting, thread } = manual();
    const { store, adapter } = await started({ thread: thread as never });
    const stale = store.loadThread(ADA);
    store.reset();
    store.detach();
    store.attach(adapter);
    await flush();
    waiting[0]!(base()); // answers the call that started before the reset
    await stale;
    expect(store.getState().threadPages[ADA]?.page ?? null).toBeNull();
  });
});

describe("an open conversation keeps to its window", () => {
  it("frames past the limit drop the oldest message, so a long-lived tab does not grow without bound", async () => {
    const { store, frame } = await started();
    await store.loadThread(ADA, 6);
    const first = store.getState().threadPages[ADA]!.page!;
    expect(first.messages).toHaveLength(6);
    for (let n = 0; n < 40; n++) frame({ event: "message", data: { change: "new", peer: ADA, message: msg({ id: `flood-${n}`, local_seq: 1000 + n, received_seq: 1000 + n }) } });
    const page = store.getState().threadPages[ADA]!.page!;
    expect(page.messages).toHaveLength(6);
    expect(page.messages[5]!.id).toBe("flood-39");
    expect(page.more).toBe(true);
  });

  it("a wider window set by 'show earlier messages' is the one that holds", async () => {
    const { store, frame } = await started();
    await store.loadThread(ADA, 3);
    await store.loadThread(ADA, 8);
    for (let n = 0; n < 5; n++) frame({ event: "message", data: { change: "new", peer: ADA, message: msg({ id: `w-${n}`, local_seq: 1000 + n }) } });
    expect(store.getState().threadPages[ADA]!.page!.messages).toHaveLength(8);
  });
});

describe("reloading the lists keeps identity without comparing every pair", () => {
  const hex = (i: number) => i.toString(16).padStart(64, "0");
  const bulk = (n: number): Thread[] =>
    Array.from({ length: n }, (_, i) => ({
      peer: hex(i),
      alias: null,
      label: `t${i}`,
      request: i % 2 === 0,
      unread: 0,
      last: msg({ id: `m${i}`, peer: hex(i), local_seq: i + 1, text: "x".repeat(400) }),
      states: { queued: 0, sent: 0, relayed: 0, delivered: 0 },
    }));

  it("1500 threads with long last lines reload in a blink, every unchanged record is the same object, a changed one is new", async () => {
    const list = bulk(1500);
    const { store } = await started({ threads: async () => clone(list) });
    const before = store.getState().threads!;
    expect(before).toHaveLength(1500);
    list[700] = { ...list[700]!, unread: 3 };
    const t0 = performance.now();
    await store.refreshThreads();
    const took = performance.now() - t0;
    const after = store.getState().threads!;
    expect(after[0]).toBe(before[0]);
    expect(after[1499]).toBe(before[1499]);
    expect(after[699]).toBe(before[699]);
    expect(after[700]).not.toBe(before[700]);
    expect(after[700]!.unread).toBe(3);
    expect(took).toBeLessThan(250);
  });

  it("matches by key, so a reordered list keeps identity too", async () => {
    const list = bulk(5);
    const { store } = await started({ threads: async () => clone(list) });
    const before = store.getState().threads!;
    list.reverse();
    await store.refreshThreads();
    const after = store.getState().threads!;
    expect(after.map((t) => t.peer)).toEqual(list.map((t) => t.peer));
    for (const t of after) expect(t).toBe(before.find((b) => b.peer === t.peer));
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
