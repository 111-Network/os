import { describe, expect, it } from "vitest";
import type { ConnectionInfo, ConnectionState, EventsFrameWithId } from "../src/lib/api/adapter";
import { DEFAULT_STALE_AFTER_MS, connectEvents } from "../src/lib/api/events-client";
import { createHttpAdapter } from "../src/lib/api/http";
import { FakeClock, fakeEventSourceCtor, last } from "./helpers/fakes";

function setup(over: { staleAfterMs?: number; noEventSource?: boolean } = {}) {
  const clock = new FakeClock();
  const { ctor, instances } = fakeEventSourceCtor();
  const frames: EventsFrameWithId[] = [];
  const states: { state: ConnectionState; info: ConnectionInfo | undefined }[] = [];
  const handlers = {
    onFrame: (f: EventsFrameWithId) => frames.push(f),
    onState: (state: ConnectionState, info?: ConnectionInfo) => states.push({ state, info }),
  };
  const stop = connectEvents(
    {
      ...(over.noEventSource ? {} : { EventSource: ctor }),
      url: "/api/v1/events",
      now: clock.now,
      setTimer: clock.setTimer,
      clearTimer: clock.clearTimer,
      ...(over.staleAfterMs ? { staleAfterMs: over.staleAfterMs } : {}),
    },
    handlers,
  );
  const names = () => states.map((s) => s.state);
  return { clock, instances, frames, states, names, stop };
}

const STATUS = { schema: 1, node: { version: "2026.10.13" } };

describe("connectEvents", () => {
  it("marks the stream stale after 45 s of silence, as the spec says", () => {
    expect(DEFAULT_STALE_AFTER_MS).toBe(45_000);
  });

  it("connects to the events URL, listens to every named event, reports connecting then open", () => {
    const t = setup();
    expect(t.instances).toHaveLength(1);
    expect(t.instances[0]!.url).toBe("/api/v1/events");
    for (const name of ["status", "services", "devices", "proposal", "agent", "reset"]) expect(t.instances[0]!.hasListener(name), name).toBe(true);
    expect(t.names()).toEqual(["connecting"]);
    t.instances[0]!.open();
    expect(t.names()).toEqual(["connecting", "open"]);
    expect(t.states[1]!.info?.lastEventAt).toBe(t.clock.now());
  });

  it("delivers parsed frames with the SSE id and tracks lastEventId", () => {
    const t = setup();
    const es = t.instances[0]!;
    es.open();
    es.message("status", STATUS, "11");
    es.message("services", { services: [] }, "12");
    es.message("devices", { devices: [] }, "12b");
    es.message("agent", { schema: 1, seq: 4, ts: "2026-10-13T08:00:00Z", kind: "chat.turn", detail: {} }, "13");
    es.message("reset", { reason: "restart" });
    expect(t.frames).toEqual([
      { event: "status", data: STATUS, id: "11" },
      { event: "services", data: { services: [] }, id: "12" },
      { event: "devices", data: { devices: [] }, id: "12b" },
      { event: "agent", data: expect.objectContaining({ seq: 4 }), id: "13" },
      { event: "reset", data: { reason: "restart" } },
    ]);
  });

  it("drops a malformed payload but still counts it as a sign of life", () => {
    const t = setup();
    const es = t.instances[0]!;
    es.open();
    t.clock.advance(40_000);
    es.message("status", "{not json");
    es.message("status", "42");
    expect(t.frames).toEqual([]);
    t.clock.advance(40_000); // would be stale at 80 s from open, but the frame at 40 s re-armed the watchdog
    expect(t.names()).toEqual(["connecting", "open"]);
  });

  it("goes stale after the staleness window with no frame, and recovers on the next frame", () => {
    const t = setup();
    const es = t.instances[0]!;
    es.open();
    t.clock.advance(DEFAULT_STALE_AFTER_MS - 1);
    expect(t.names()).toEqual(["connecting", "open"]);
    t.clock.advance(1);
    expect(t.names()).toEqual(["connecting", "open", "stale"]);
    const openedAt = t.states[1]!.info!.lastEventAt;
    expect(t.states[2]!.info?.lastEventAt).toBe(openedAt);
    t.clock.advance(5_000);
    es.message("status", STATUS, "2");
    expect(t.names()).toEqual(["connecting", "open", "stale", "open"]);
    expect(t.states[3]!.info?.lastEventAt).toBe(t.clock.now());
    expect(t.frames).toHaveLength(1);
  });

  it("stays open across the 30 s status heartbeat with the default window", () => {
    const t = setup();
    const es = t.instances[0]!;
    es.open();
    for (let i = 0; i < 6; i++) {
      t.clock.advance(30_000);
      es.message("status", STATUS, String(i));
    }
    expect(t.names()).toEqual(["connecting", "open"]);
  });

  it("honours a custom staleness window (20 s)", () => {
    const t = setup({ staleAfterMs: 20_000 });
    t.instances[0]!.open();
    t.clock.advance(19_999);
    expect(t.names()).not.toContain("stale");
    t.clock.advance(1);
    expect(t.names()).toContain("stale");
  });

  it("reports connecting while the browser retries, and does not go stale meanwhile", () => {
    const t = setup();
    const es = t.instances[0]!;
    es.open();
    es.error(false);
    expect(t.names()).toEqual(["connecting", "open", "connecting"]);
    t.clock.advance(5 * 60_000);
    expect(t.names()).toEqual(["connecting", "open", "connecting"]);
    expect(t.instances).toHaveLength(1); // the browser's own reconnect, no new object
    es.open();
    expect(t.names().slice(-1)).toEqual(["open"]);
    es.message("status", STATUS, "9");
    t.clock.advance(DEFAULT_STALE_AFTER_MS);
    expect(t.names().slice(-1)).toEqual(["stale"]);
  });

  it("on a fatal error reports closed and retries with backoff using a new EventSource", () => {
    const t = setup();
    t.instances[0]!.open();
    t.instances[0]!.error(true);
    expect(t.names().slice(-1)).toEqual(["closed"]);
    expect(t.instances[0]!.closed).toBe(true);
    t.clock.advance(999);
    expect(t.instances).toHaveLength(1);
    t.clock.advance(1);
    expect(t.instances).toHaveLength(2);
    expect(t.names().slice(-1)).toEqual(["connecting"]);
    t.instances[1]!.error(true);
    t.clock.advance(1_999);
    expect(t.instances).toHaveLength(2);
    t.clock.advance(1);
    expect(t.instances).toHaveLength(3);
    t.instances[2]!.error(true);
    t.clock.advance(4_000);
    expect(t.instances).toHaveLength(4);
    t.instances[3]!.open(); // success resets the backoff
    t.instances[3]!.error(true);
    t.clock.advance(1_000);
    expect(t.instances).toHaveLength(5);
  });

  it("caps the backoff at 15 s", () => {
    const t = setup();
    for (let i = 0; i < 8; i++) {
      t.instances[t.instances.length - 1]!.error(true);
      t.clock.advance(15_000);
    }
    const before = t.instances.length;
    t.instances[before - 1]!.error(true);
    t.clock.advance(14_999);
    expect(t.instances).toHaveLength(before);
    t.clock.advance(1);
    expect(t.instances).toHaveLength(before + 1);
  });

  it("unsubscribe closes the source, clears timers and silences every callback", () => {
    const t = setup();
    const es = t.instances[0]!;
    es.open();
    t.stop();
    expect(es.closed).toBe(true);
    expect(t.clock.pending).toBe(0);
    const n = t.states.length;
    es.message("status", STATUS, "1");
    es.error(true);
    t.clock.advance(120_000);
    expect(t.frames).toEqual([]);
    expect(t.states).toHaveLength(n);
    expect(t.instances).toHaveLength(1);
    t.stop(); // idempotent
  });

  it("unsubscribe during a retry wait cancels the retry", () => {
    const t = setup();
    t.instances[0]!.error(true);
    t.stop();
    t.clock.advance(60_000);
    expect(t.instances).toHaveLength(1);
  });

  it("reports closed when there is no EventSource at all", () => {
    const t = setup({ noEventSource: true });
    expect(t.names()).toEqual(["closed"]);
  });

  it("ignores events from a source that has been replaced", () => {
    const t = setup();
    const old = t.instances[0]!;
    old.error(true);
    t.clock.advance(1_000);
    old.message("status", STATUS, "5");
    expect(t.frames).toEqual([]);
  });
});

describe("http adapter events()", () => {
  it("wires the adapter's EventSource, clock and /api/v1/events URL", () => {
    const clock = new FakeClock();
    const { ctor, instances } = fakeEventSourceCtor();
    const adapter = createHttpAdapter({ EventSource: ctor, now: clock.now, setTimer: clock.setTimer, clearTimer: clock.clearTimer, base: "http://node.test" });
    const states: ConnectionState[] = [];
    const frames: EventsFrameWithId[] = [];
    const stop = adapter.events({ onFrame: (f) => frames.push(f), onState: (s) => states.push(s) });
    expect(instances[0]!.url).toBe("http://node.test/api/v1/events");
    instances[0]!.open();
    instances[0]!.message("reset", { reason: "expired" });
    expect(states).toEqual(["connecting", "open"]);
    expect(frames).toEqual([{ event: "reset", data: { reason: "expired" } }]);
    clock.advance(DEFAULT_STALE_AFTER_MS);
    expect(last(states)).toBe("stale");
    stop();
    expect(instances[0]!.closed).toBe(true);
  });
});
