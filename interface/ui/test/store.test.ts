import { describe, expect, it, vi } from "vitest";
import type { Adapter, ConnectionInfo, ConnectionState, EventsFrameWithId, EventsHandlers } from "../src/lib/api/adapter";
import { ApiError } from "../src/lib/api/problem";
import { ACTIVITY_RING, ApiStore, computeAttention, isDisconnected, openProposalsOf } from "../src/lib/api/store";
import type { AgentEvent, DeviceRecord, Proposal, ServiceRecord, Status } from "../src/lib/api/types";
import { buildFixtures } from "../src/lib/api/mock/fixtures";
import { flush, last } from "./helpers/fakes";

const fx = buildFixtures(Date.parse("2026-10-13T12:00:00Z"));
const clone = <T,>(v: T): T => structuredClone(v);

function fakeAdapter(over: Partial<Record<"status" | "services" | "devices" | "proposals", () => Promise<unknown>>> = {}) {
  const handlers: { current: EventsHandlers | null } = { current: null };
  const calls = { status: 0, services: 0, devices: 0, proposals: [] as (string | undefined)[], unsub: 0 };
  const adapter = {
    kind: "http",
    status: vi.fn(async () => {
      calls.status++;
      return (over.status ? over.status() : clone(fx.status)) as Status;
    }),
    services: vi.fn(async () => {
      calls.services++;
      return (over.services ? over.services() : clone(fx.services)) as ServiceRecord[];
    }),
    devices: vi.fn(async () => {
      calls.devices++;
      return (over.devices ? over.devices() : clone(fx.devices)) as DeviceRecord[];
    }),
    proposals: vi.fn(async (s?: string) => {
      calls.proposals.push(s);
      return (over.proposals ? over.proposals() : clone(fx.proposals)) as Proposal[];
    }),
    events: vi.fn((h: EventsHandlers) => {
      handlers.current = h;
      return () => {
        calls.unsub++;
      };
    }),
  } as unknown as Adapter;
  const frame = (f: EventsFrameWithId) => handlers.current!.onFrame(f);
  const state = (s: ConnectionState, info?: ConnectionInfo) => handlers.current!.onState(s, info);
  return { adapter, calls, frame, state };
}

const agentEvent = (seq: number, ts = "2026-10-13T12:00:00Z"): AgentEvent => ({ schema: 1, seq, ts, kind: "chat.turn", detail: {} });

async function started(over: Parameters<typeof fakeAdapter>[0] = {}, now = () => 1_000) {
  const store = new ApiStore({ now });
  const f = fakeAdapter(over);
  store.attach(f.adapter);
  await flush();
  return { store, ...f };
}

describe("computeAttention", () => {
  it("counts open proposals plus services in error", () => {
    expect(computeAttention(fx.proposals, fx.services)).toEqual({ count: 1, openProposals: 1, erroredServices: 0, erroredDevices: 0 });
    const services = clone(fx.services);
    services[0]!.state = "error";
    services[1]!.state = "error";
    services[2]!.state = "unset"; // unset and starting are not attention
    services[3]!.state = "starting";
    expect(computeAttention(fx.proposals, services)).toEqual({ count: 3, openProposals: 1, erroredServices: 2, erroredDevices: 0 });
  });

  it("also counts devices in error, but not needs_setup or disconnected ones", () => {
    const devices = clone(fx.devices);
    expect(devices.map((d) => d.state)).toContain("needs_setup");
    expect(computeAttention(fx.proposals, fx.services, devices).erroredDevices).toBe(0);
    devices[0]!.state = "error";
    devices[1]!.state = "disconnected";
    expect(computeAttention(fx.proposals, fx.services, devices)).toEqual({ count: 2, openProposals: 1, erroredServices: 0, erroredDevices: 1 });
    expect(computeAttention(null, null, devices)).toEqual({ count: 1, openProposals: 0, erroredServices: 0, erroredDevices: 1 });
    expect(computeAttention(null, null, null).erroredDevices).toBe(0);
  });

  it("is zero while nothing is loaded and ignores decided proposals", () => {
    expect(computeAttention(null, null).count).toBe(0);
    expect(computeAttention(fx.proposals.filter((p) => p.decision), fx.services).count).toBe(0);
    expect(openProposalsOf(fx.proposals).map((p) => p.id)).toEqual(["prop-01j9z3k8m2q4"]);
    expect(openProposalsOf(null)).toEqual([]);
  });
});

describe("isDisconnected", () => {
  it("is false for the first connect and true once a connection was lost", () => {
    expect(isDisconnected("connecting", false)).toBe(false);
    expect(isDisconnected("open", true)).toBe(false);
    expect(isDisconnected("connecting", true)).toBe(true);
    expect(isDisconnected("stale", true)).toBe(true);
    expect(isDisconnected("closed", false)).toBe(true);
  });
});

describe("ApiStore snapshots", () => {
  it("loads status, services, devices and all proposals (newest first) on attach", async () => {
    const { store, calls } = await started();
    const s = store.getState();
    expect(s.status?.node.label).toBe("k7m2-9x4p-aa");
    expect(s.services).toHaveLength(7);
    expect(s.devices?.map((d) => d.kind)).toEqual(["radio", "companion", "bluetooth", "unknown"]);
    expect(s.proposals?.map((p) => p.id)).toEqual(["prop-01j9z3k8m2q4", "prop-01j9y7c2d9f1", "prop-01j9x4t6h8w3"]);
    expect(calls.proposals).toEqual(["all"]);
    expect(s.meta).toEqual({ ready: true, needsPair: false, error: null });
    expect(s.attention).toEqual({ count: 1, openProposals: 1, erroredServices: 0, erroredDevices: 0 });
    expect(s.version).toBe("2026.10.13");
    expect(s.baselineVersion).toBe("2026.10.13");
    expect(s.versionChanged).toBe(false);
  });

  it("flags needsPair when a snapshot call answers 401", async () => {
    const reject = () => Promise.reject(new ApiError({ type: "about:blank", title: "Pair", status: 401, code: "unauthenticated" }));
    const { store } = await started({ status: reject, services: reject, devices: reject, proposals: reject });
    expect(store.getState().meta).toMatchObject({ ready: true, needsPair: true });
    expect(store.getState().meta.error?.code).toBe("unauthenticated");
    expect(store.getState().status).toBeNull();
  });

  it("keeps the other snapshots when one call fails", async () => {
    const { store } = await started({ proposals: () => Promise.reject(ApiError.network(new TypeError("x"))) });
    expect(store.getState().status).not.toBeNull();
    expect(store.getState().proposals).toBeNull();
    expect(store.getState().meta.error?.code).toBe("network");
    expect(store.getState().meta.needsPair).toBe(false);
  });

  it("coalesces concurrent refreshes into one trailing pass", async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const { store, calls } = await started({
      status: async () => {
        await gate;
        return clone(fx.status);
      },
    });
    // attach started pass 1 (blocked on the gate); three more requests must collapse into one rerun
    const extra = [store.refreshAll(), store.refreshAll(), store.refreshAll()];
    release();
    await Promise.all(extra);
    expect(calls.status).toBe(2);
  });

  it("ignores results that arrive after detach", async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const store = new ApiStore();
    const f = fakeAdapter({
      status: async () => {
        await gate;
        return clone(fx.status);
      },
    });
    store.attach(f.adapter);
    store.detach();
    release();
    await flush();
    expect(store.getState().status).toBeNull();
    expect(f.calls.unsub).toBe(1);
  });
});

describe("ApiStore frames", () => {
  it("replaces status and services from frames and recomputes attention", async () => {
    const { store, frame } = await started();
    const services = clone(fx.services);
    services[2]!.state = "error";
    frame({ event: "services", data: { services }, id: "1" });
    expect(store.getState().services?.[2]?.state).toBe("error");
    expect(store.getState().attention.erroredServices).toBe(1);
    expect(store.getState().attention.count).toBe(2);
    const status = clone(fx.status);
    status.transport_state = "transport degraded";
    frame({ event: "status", data: status, id: "2" });
    expect(store.getState().status?.transport_state).toBe("transport degraded");
  });

  it("keeps object identity for unchanged records and unchanged status", async () => {
    const { store, frame } = await started();
    const before = store.getState();
    const services = clone(fx.services);
    services[0]!.state = "error";
    frame({ event: "services", data: { services } });
    const after = store.getState();
    expect(after.services![1]).toBe(before.services![1]);
    expect(after.services![0]).not.toBe(before.services![0]);
    frame({ event: "status", data: clone(fx.status) });
    expect(store.getState().status).toBe(before.status);
  });

  it("refetches proposals on a proposal frame", async () => {
    let list = clone(fx.proposals);
    const { store, frame, calls } = await started({ proposals: async () => list });
    expect(calls.proposals).toEqual(["all"]);
    expect(store.getState().attention.openProposals).toBe(1);
    list = [{ ...clone(fx.proposals[0]!), id: "prop-new", created_at: "2026-10-13T13:00:00Z" }, ...list];
    frame({ event: "proposal", data: { id: "prop-new", title: "t", kind: "diagnose", created_at: "2026-10-13T13:00:00Z" }, id: "9" });
    await flush();
    expect(calls.proposals).toEqual(["all", "all"]);
    expect(calls.status).toBe(1); // only proposals were reloaded
    expect(store.getState().proposals?.[0]?.id).toBe("prop-new");
    expect(store.getState().attention.openProposals).toBe(2);
  });

  it("refetches everything on reset", async () => {
    const { frame, calls } = await started();
    frame({ event: "reset", data: { reason: "restart" } });
    await flush();
    expect(calls.status).toBe(2);
    expect(calls.services).toBe(2);
    expect(calls.devices).toBe(2);
    expect(calls.proposals).toHaveLength(2);
  });

  it("replaces the device list from a devices frame, keeping unchanged records and counting errors", async () => {
    const { store, frame } = await started();
    const before = store.getState();
    const devices = clone(fx.devices);
    devices[0]!.state = "error";
    devices[0]!.reason_code = "service_error";
    frame({ event: "devices", data: { devices }, id: "4" });
    const after = store.getState();
    expect(after.devices?.[0]?.state).toBe("error");
    expect(after.devices?.[1]).toBe(before.devices?.[1]); // unchanged record keeps its identity
    expect(after.devices?.[0]).not.toBe(before.devices?.[0]);
    expect(after.attention).toEqual({ count: 2, openProposals: 1, erroredServices: 0, erroredDevices: 1 });
    // a device leaving (forgotten or unplugged) is reflected by the next snapshot
    frame({ event: "devices", data: { devices: devices.slice(1) }, id: "5" });
    expect(store.getState().devices).toHaveLength(3);
    expect(store.getState().attention.erroredDevices).toBe(0);
  });

  it("starts with no devices until the first snapshot, and ignores a malformed devices frame", async () => {
    const store = new ApiStore();
    expect(store.getState().devices).toBeNull();
    const { store: loaded, frame } = await started();
    frame({ event: "devices", data: {} as never });
    expect(loaded.getState().devices).toHaveLength(4);
  });

  it("refetches devices on demand without touching the other snapshots", async () => {
    let list = clone(fx.devices);
    const { store, calls } = await started({ devices: async () => list });
    list = list.slice(0, 2);
    await store.refreshDevices();
    expect(store.getState().devices).toHaveLength(2);
    expect(calls.devices).toBe(2);
    expect(calls.status).toBe(1);
  });

  it("dedupes agent events on seq, newest first, ring of 200", async () => {
    const { store, frame } = await started();
    frame({ event: "agent", data: agentEvent(1) });
    frame({ event: "agent", data: agentEvent(2) });
    frame({ event: "agent", data: agentEvent(2) }); // replay after reconnect
    frame({ event: "agent", data: agentEvent(1) });
    expect(store.getState().activity.map((a) => a.seq)).toEqual([2, 1]);
    for (let i = 3; i < ACTIVITY_RING + 50; i++) frame({ event: "agent", data: agentEvent(i) });
    const act = store.getState().activity;
    expect(act).toHaveLength(ACTIVITY_RING);
    expect(act[0]!.seq).toBe(ACTIVITY_RING + 49);
    expect(last(act)!.seq).toBe(50);
  });

  it("does not drop a new event whose seq restarted after an agent restart", async () => {
    const { store, frame } = await started();
    frame({ event: "agent", data: agentEvent(1, "2026-10-13T12:00:00Z") });
    frame({ event: "agent", data: agentEvent(1, "2026-10-13T15:00:00Z") });
    expect(store.getState().activity).toHaveLength(2);
  });

  it("tracks a version change from the first status of the page load", async () => {
    const { store, frame } = await started();
    const next = clone(fx.status);
    next.node.version = "2026.10.14";
    frame({ event: "status", data: next });
    expect(store.getState()).toMatchObject({ version: "2026.10.14", baselineVersion: "2026.10.13", versionChanged: true });
    const back = clone(fx.status);
    frame({ event: "status", data: back });
    expect(store.getState().versionChanged).toBe(false);
  });

  it("ignores malformed frames instead of throwing", async () => {
    const { store, frame } = await started();
    expect(() => {
      frame({ event: "services", data: {} as never });
      frame({ event: "status", data: {} as never });
      frame({ event: "agent", data: {} as never });
    }).not.toThrow();
    expect(store.getState().services).toHaveLength(7);
    expect(store.getState().activity).toEqual([]);
  });
});

describe("ApiStore connection", () => {
  it("tracks state, lastEventAt and the disconnected flag", async () => {
    let now = 1_000;
    const { store, state, frame } = await started({}, () => now);
    expect(store.getState().connection).toMatchObject({ state: "connecting", everOpened: false, disconnected: false });
    state("open", { lastEventAt: 900 });
    expect(store.getState().connection).toMatchObject({ state: "open", everOpened: true, disconnected: false, lastEventAt: 900 });
    now = 5_000;
    frame({ event: "status", data: clone(fx.status) });
    expect(store.getState().connection.lastEventAt).toBe(5_000);
    state("stale", { lastEventAt: 5_000 });
    expect(store.getState().connection).toMatchObject({ state: "stale", disconnected: true, lastEventAt: 5_000 });
    state("connecting");
    expect(store.getState().connection).toMatchObject({ disconnected: true, lastEventAt: 5_000 });
    state("open");
    expect(store.getState().connection.disconnected).toBe(false);
  });

  it("probes /status when the stream closes so a lost session reaches the shell", async () => {
    let calls = 0;
    const { store, state } = await started({
      status: () => {
        calls++;
        return calls === 1 ? Promise.resolve(clone(fx.status)) : Promise.reject(new ApiError({ type: "about:blank", title: "x", status: 401, code: "unauthenticated" }));
      },
    });
    expect(store.getState().meta.needsPair).toBe(false);
    state("closed");
    await flush();
    expect(store.getState().meta.needsPair).toBe(true);
    expect(store.getState().connection).toMatchObject({ state: "closed", disconnected: true });
  });
});

describe("ApiStore subscription", () => {
  it("notifies listeners on change and keeps the snapshot reference stable otherwise", async () => {
    const { store, state } = await started();
    const listener = vi.fn();
    const off = store.subscribe(listener);
    const snap = store.getState();
    state("connecting"); // same as current: no change
    expect(listener).not.toHaveBeenCalled();
    expect(store.getState()).toBe(snap);
    state("open");
    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.getState()).not.toBe(snap);
    off();
    state("stale");
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("reset() empties the state for tests and logout", async () => {
    const { store } = await started();
    store.reset();
    expect(store.getState().status).toBeNull();
    expect(store.getState().meta.ready).toBe(false);
  });
});
