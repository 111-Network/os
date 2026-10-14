// The hooks are thin useSyncExternalStore wrappers; render them with react-dom/server (no DOM).
import { createElement as h, type ReactElement } from "react";
import { renderToString } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Adapter, EventsHandlers } from "../src/lib/api/adapter";
import { useActivity, useApiMeta, useAttention, useConnection, useDevice, useDevices, useProposals, useService, useServices, useStatus, useVersionChanged } from "../src/lib/api/hooks";
import { buildFixtures } from "../src/lib/api/mock/fixtures";
import { apiStore } from "../src/lib/api/store";
import { flush } from "./helpers/fakes";

const fx = buildFixtures(Date.parse("2026-10-13T12:00:00Z"));

function Probe(): ReactElement {
  const status = useStatus();
  const services = useServices();
  const radio = useService("radio");
  const proposals = useProposals();
  const devices = useDevices();
  const heltec = useDevice("usb-7c1e0b94a2d3");
  const gone = useDevice("nope");
  const conn = useConnection();
  const activity = useActivity();
  const attention = useAttention();
  const changed = useVersionChanged();
  const meta = useApiMeta();
  return h("pre", null, JSON.stringify({ label: status?.node.label ?? null, n: services?.length ?? null, radio: radio?.state ?? null, p: proposals?.length ?? null, d: devices?.length ?? null, heltec: heltec?.state ?? null, gone: gone?.state ?? null, conn: conn.state, a: activity.length, attention: attention.count, changed, ready: meta.ready }));
}

const render = () => JSON.parse(renderToString(h(Probe)).replace(/<[^>]+>/g, "").replace(/&quot;/g, '"')) as Record<string, unknown>;

afterEach(() => {
  apiStore.detach();
  apiStore.reset();
});

describe("hooks", () => {
  it("render the empty state before any snapshot", () => {
    expect(render()).toEqual({ label: null, n: null, radio: null, p: null, d: null, heltec: null, gone: null, conn: "connecting", a: 0, attention: 0, changed: false, ready: false });
  });

  it("render the loaded state and follow frames", async () => {
    const handlers: { current: EventsHandlers | null } = { current: null };
    const adapter = {
      kind: "http",
      status: async () => structuredClone(fx.status),
      services: async () => structuredClone(fx.services),
      devices: async () => structuredClone(fx.devices),
      proposals: async () => structuredClone(fx.proposals),
      events: vi.fn((hd: EventsHandlers) => {
        handlers.current = hd;
        return () => undefined;
      }),
    } as unknown as Adapter;
    apiStore.attach(adapter);
    await flush();
    expect(render()).toEqual({ label: "k7m2-9x4p-aa", n: 9, radio: "unset", p: 3, d: 4, heltec: "needs_setup", gone: null, conn: "connecting", a: 0, attention: 1, changed: false, ready: true });
    handlers.current!.onState("open", { lastEventAt: 1 });
    const services = structuredClone(fx.services);
    services.find((s) => s.id === "radio")!.state = "error";
    handlers.current!.onFrame({ event: "services", data: { services } });
    expect(render()).toMatchObject({ conn: "open", radio: "error", attention: 2 });
    const devices = structuredClone(fx.devices);
    devices[0]!.state = "error";
    handlers.current!.onFrame({ event: "devices", data: { devices } });
    expect(render()).toMatchObject({ heltec: "error", attention: 3 });
    handlers.current!.onFrame({ event: "devices", data: { devices: devices.slice(1) } });
    expect(render()).toMatchObject({ d: 3, heltec: null, attention: 2 });
  });
});
