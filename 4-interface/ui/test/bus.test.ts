import { describe, expect, it } from "vitest";
import type { EventsFrameWithId } from "../src/lib/api/adapter";
import { MockBus } from "../src/lib/api/mock/bus";
import type { EventsFrame, Status } from "../src/lib/api/types";
import { FakeClock, flush, last } from "./helpers/fakes";

const agent = (n: number): EventsFrame => ({ event: "agent", data: { schema: 1, seq: n, ts: "2026-10-13T12:00:00Z", kind: "chat.turn", detail: {} } });

function setup(opts: { capacity?: number; gate?: () => boolean } = {}) {
  const clock = new FakeClock();
  const bus = new MockBus({ clock, snapshot: () => ({ node: { uptime_sec: clock.now() } }) as unknown as Status, ...opts });
  const connect = (lastEventId?: number) => {
    const frames: EventsFrameWithId[] = [];
    const states: string[] = [];
    const off = bus.subscribe({ onFrame: (f) => frames.push(f), onState: (s) => states.push(s) }, lastEventId);
    return { frames, states, off };
  };
  return { clock, bus, connect };
}

describe("MockBus", () => {
  it("assigns one global monotonic id as a string and delivers to every subscriber", async () => {
    const { bus, connect } = setup();
    const a = connect();
    const b = connect();
    await flush();
    bus.publish(agent(1));
    bus.publish(agent(2));
    for (const s of [a, b]) expect(s.frames.filter((f) => f.id).map((f) => f.id)).toEqual(["1", "2"]);
    expect(bus.lastId).toBe(2);
  });

  it("answers a connection without Last-Event-ID with reset (restart) and no replay", async () => {
    const { bus, connect } = setup();
    bus.publish(agent(1));
    const s = connect();
    await flush();
    expect(s.states).toEqual(["connecting", "open"]);
    expect(s.frames).toEqual([{ event: "reset", data: { reason: "restart" } }]);
  });

  it("replays frames after Last-Event-ID when they are still in the ring", async () => {
    const { bus, connect } = setup();
    for (let i = 1; i <= 5; i++) bus.publish(agent(i));
    const s = connect(2);
    await flush();
    expect(s.frames.map((f) => f.id)).toEqual(["3", "4", "5"]);
    bus.publish(agent(6));
    expect(last(s.frames)!.id).toBe("6");
  });

  it("replays nothing when the client is already current", async () => {
    const { bus, connect } = setup();
    for (let i = 1; i <= 3; i++) bus.publish(agent(i));
    const s = connect(3);
    await flush();
    expect(s.frames).toEqual([]);
  });

  it("resets with reason expired when the id fell out of the ring or is from the future", async () => {
    const { bus, connect } = setup({ capacity: 4 });
    for (let i = 1; i <= 10; i++) bus.publish(agent(i)); // ring holds 7..10
    expect(bus.size).toBe(4);
    const old = connect(2);
    const future = connect(99);
    const edge = connect(6); // 7 is the oldest kept, so 6 is still replayable
    await flush();
    expect(old.frames).toEqual([{ event: "reset", data: { reason: "expired" } }]);
    expect(future.frames).toEqual([{ event: "reset", data: { reason: "expired" } }]);
    expect(edge.frames.map((f) => f.id)).toEqual(["7", "8", "9", "10"]);
  });

  it("disconnect(ms) drops the stream, then reconnects with replay of the gap", async () => {
    const { bus, clock, connect } = setup();
    const s = connect();
    await flush();
    bus.publish(agent(1));
    bus.disconnect(5_000);
    expect(s.states).toEqual(["connecting", "open", "connecting"]);
    bus.publish(agent(2));
    bus.publish(agent(3));
    expect(s.frames.map((f) => f.id)).toEqual([undefined, "1"]); // reset, then 1; nothing while offline
    clock.advance(5_000);
    expect(last(s.states)).toBe("open");
    expect(s.frames.map((f) => f.id)).toEqual([undefined, "1", "2", "3"]);
  });

  it("a new daemon lifetime makes reconnecting clients reset with reason restart", async () => {
    const { bus, clock, connect } = setup();
    const s = connect();
    await flush();
    for (let i = 1; i <= 3; i++) bus.publish(agent(i));
    bus.disconnect(1_000);
    bus.newLifetime();
    expect(bus.size).toBe(0);
    clock.advance(1_000);
    expect(last(s.frames)).toEqual({ event: "reset", data: { reason: "restart" } });
    bus.publish(agent(4));
    expect(last(s.frames)!.id).toBe("1"); // ids restart with the lifetime
    // a later drop resumes from the new sequence rather than resetting again
    bus.disconnect(1_000);
    bus.publish(agent(5));
    clock.advance(1_000);
    expect(last(s.frames)!.id).toBe("2");
    expect(s.frames.filter((f) => f.event === "reset")).toHaveLength(2); // the initial connect + the restart
  });

  it("sends a status heartbeat every 30 s until stopped", async () => {
    const { bus, clock, connect } = setup();
    const s = connect();
    await flush();
    bus.start();
    clock.advance(29_999);
    expect(s.frames.filter((f) => f.event === "status")).toHaveLength(0);
    clock.advance(1);
    clock.advance(30_000);
    expect(s.frames.filter((f) => f.event === "status")).toHaveLength(2);
    bus.stop();
    clock.advance(120_000);
    expect(s.frames.filter((f) => f.event === "status")).toHaveLength(2);
    expect(clock.pending).toBe(0);
  });

  it("refuses gated subscribers like a 401 and lets them in on retryGated()", async () => {
    let open = false;
    const { bus, connect } = setup({ gate: () => open });
    const s = connect();
    await flush();
    expect(s.states).toEqual(["connecting", "closed"]);
    bus.publish(agent(1));
    expect(s.frames).toEqual([]);
    open = true;
    bus.retryGated();
    await flush();
    expect(s.states.slice(-2)).toEqual(["connecting", "open"]);
    expect(s.frames[0]).toEqual({ event: "reset", data: { reason: "restart" } });
  });

  it("stops delivering after unsubscribe", async () => {
    const { bus, connect } = setup();
    const s = connect();
    await flush();
    s.off();
    bus.publish(agent(1));
    expect(s.frames.filter((f) => f.event === "agent")).toEqual([]);
  });
});
