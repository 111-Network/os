// Daemon <-> parser drift gate. test/tapes/*.sse are byte-for-byte recordings of what the real daemon
// sent (events stream and chat stream). Each tape goes through the real SSE parser and the real frame
// mappers (events-client.ts for /events, http.ts chat.send for /chat), and every frame must use an
// event name from the contract and carry a payload that validates against its schema (required keys
// included). A failure names the tape and the frame index, so a daemon change that the UI would
// silently drop shows up here, not in a browser.
import { readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { EventsFrameWithId } from "../src/lib/api/adapter";
import { connectEvents } from "../src/lib/api/events-client";
import { createHttpAdapter } from "../src/lib/api/http";
import { parseSseStream, type SseMessage } from "../src/lib/api/sse";
import { CHAT_EVENT_NAMES, EVENTS_EVENT_NAMES } from "../src/lib/api/types";
import { schemas, sse } from "./contract.gen";
import { FakeClock, collect, fakeEventSourceCtor, last, sseResponse } from "./helpers/fakes";
import { validateAgainst } from "./helpers/validate";

const TAPE_DIR = resolve(__dirname, "tapes");
const tapes = readdirSync(TAPE_DIR)
  .filter((f) => f.endsWith(".sse"))
  .sort();

type Kind = "events" | "chat";
const kindOf = (tape: string): Kind => {
  if (tape.startsWith("events")) return "events";
  if (tape.startsWith("chat")) return "chat";
  throw new Error(`${tape}: name a tape events*.sse or chat*.sse so the gate knows which stream it recorded`);
};

const bytesOf = (tape: string): Uint8Array => new Uint8Array(readFileSync(join(TAPE_DIR, tape)));

/** The tape in `size`-byte chunks, so parser state carried across chunk boundaries is exercised too. */
async function* chunked(data: Uint8Array, size: number): AsyncGenerator<Uint8Array> {
  for (let i = 0; i < data.length; i += size) yield data.slice(i, i + size);
}

async function parse(tape: string, size = Number.POSITIVE_INFINITY): Promise<SseMessage[]> {
  const data = bytesOf(tape);
  return collect(parseSseStream(chunked(data, Math.min(size, Math.max(1, data.length)))));
}

const at = (tape: string, i: number, event?: string) => `${tape} frame #${i}${event ? ` (${event})` : ""}`;

describe("recorded tapes exist", () => {
  it("has the events and chat recordings (deleting them would turn this gate off)", () => {
    expect(tapes.filter((t) => kindOf(t) === "events").length).toBeGreaterThanOrEqual(3);
    expect(tapes.filter((t) => kindOf(t) === "chat").length).toBeGreaterThanOrEqual(3);
  });

  it("the contract and the runtime name lists agree (the gate checks against both)", () => {
    expect(Object.keys(sse.events)).toEqual([...EVENTS_EVENT_NAMES]);
    expect(Object.keys(sse.chat)).toEqual([...CHAT_EVENT_NAMES]);
  });
});

describe.each(tapes)("tape %s", (tape) => {
  const kind = kindOf(tape);
  const names = kind === "events" ? sse.events : sse.chat;

  it("parses the same whole or in small chunks, with no empty tape", async () => {
    const whole = await parse(tape);
    expect(whole.length, `${tape} has no frames`).toBeGreaterThan(0);
    expect(await parse(tape, 7), `${tape}: 7-byte chunks parse differently`).toEqual(whole);
    expect(await parse(tape, 1), `${tape}: 1-byte chunks parse differently`).toEqual(whole);
  });

  it("every frame has a contract event name and a payload that validates against its schema", async () => {
    const messages = await parse(tape);
    messages.forEach((m, i) => {
      const where = at(tape, i, m.event);
      expect(m.event, `${where}: frame has no event name`).toBeTruthy();
      expect(Object.keys(names), `${where}: event name is not in the ${kind} contract`).toContain(m.event);
      let payload: unknown;
      expect(() => (payload = JSON.parse(m.data)), `${where}: data is not JSON`).not.toThrow();
      expect(typeof payload === "object" && payload !== null && !Array.isArray(payload), `${where}: data is not a JSON object`).toBe(true);
      const schemaName = names[m.event!]!;
      expect(schemas[schemaName], `${where}: schema ${schemaName} missing from the contract digest`).toBeTruthy();
      expect(validateAgainst(schemaName, payload), `${where}: payload does not match ${schemaName}`).toEqual([]);
    });
  });

  if (kind === "events") {
    it("ids are present and strictly increasing (the replay cursor)", async () => {
      const messages = await parse(tape);
      let prev = -1n;
      messages.forEach((m, i) => {
        expect(m.id, `${at(tape, i, m.event)}: missing id:`).toMatch(/^\d+$/);
        const id = BigInt(m.id!);
        expect(id > prev, `${at(tape, i, m.event)}: id ${m.id} does not increase (previous ${prev})`).toBe(true);
        prev = id;
      });
    });

    it("the events client maps every frame: none dropped, event, data and id intact", async () => {
      const messages = await parse(tape);
      const clock = new FakeClock();
      const { ctor, instances } = fakeEventSourceCtor();
      const frames: EventsFrameWithId[] = [];
      const stop = connectEvents(
        { EventSource: ctor, url: "/api/v1/events", now: clock.now, setTimer: clock.setTimer, clearTimer: clock.clearTimer },
        { onFrame: (f) => frames.push(f), onState: () => undefined },
      );
      const es = instances[0]!;
      es.open();
      messages.forEach((m, i) => {
        const before = frames.length;
        es.message(m.event!, m.data, m.id);
        expect(frames.length, `${at(tape, i, m.event)}: the events client dropped this frame`).toBe(before + 1);
        expect(last(frames), at(tape, i, m.event)).toEqual({ event: m.event, data: JSON.parse(m.data), ...(m.id ? { id: m.id } : {}) });
      });
      stop();
      expect(frames).toHaveLength(messages.length);
    });
  } else {
    it("the chat client yields every frame in order and ends on exactly one terminal frame", async () => {
      const messages = await parse(tape);
      const adapter = createHttpAdapter({ fetch: async () => sseResponse([bytesOf(tape)]) });
      const frames = await collect(adapter.chat.send({ conversation_id: "conv-tape", message: "hi" }));
      messages.forEach((m, i) => {
        expect(frames[i], `${at(tape, i, m.event)}: the chat client yielded nothing here`).toBeDefined();
        expect(frames[i], at(tape, i, m.event)).toEqual({ event: m.event, data: JSON.parse(m.data) });
      });
      expect(frames, `${tape}: the chat client yielded extra frames`).toHaveLength(messages.length);
      const terminals = frames.filter((f) => f.event === "done" || f.event === "error");
      expect(terminals, `${tape}: expected exactly one done|error frame`).toHaveLength(1);
      expect(last(frames)!.event, `${tape}: the terminal frame is not last`).toMatch(/^(done|error)$/);
    });
  }
});

describe("the gate has teeth", () => {
  it("the validator flags a missing required key and a wrong type, and failures name tape and frame", () => {
    expect(validateAgainst(sse.events.reset!, {}).join("\n")).toContain('missing required "reason"');
    expect(validateAgainst(sse.chat.token!, { text: 1 })).not.toEqual([]);
    expect(validateAgainst(sse.chat.done!, { conversation_id: "c" })).not.toEqual([]);
    expect(at("events.sse", 0, "reset")).toBe("events.sse frame #0 (reset)");
  });
});
