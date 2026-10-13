import { describe, expect, it } from "vitest";
import { ApiError } from "../src/lib/api/problem";
import type { ChatFrame, ServiceRecord, Status } from "../src/lib/api/sse.gen";
import { ChatSession, type SessionDeps } from "../src/views/agent/session";
import {
  agentState,
  CONVERSATION_ID_RE,
  CONVERSATION_KEY,
  filterHistory,
  loadConversationId,
  mintConversationId,
  queuedNotice,
  reasonClause,
  toolLine,
  whyPrompt,
  type HistoryMessage,
  type KeyValueStore,
} from "../src/views/agent/state";
import { TokenBuffer } from "../src/views/agent/token-buffer";
import { FakeClock } from "./helpers/fakes";

/** Let every pending promise continuation (several per streamed frame) run. */
const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

// ---------------------------------------------------------------- fixtures

const svc = (id: ServiceRecord["id"], state: ServiceRecord["state"], extra: Partial<ServiceRecord> = {}): ServiceRecord =>
  ({ id, name: id, plain_name: id, kind: "child", state, since: "2026-10-13T00:00:00Z", reason_code: "x", reason: `${id} is ${state}.`, can_run_here: true, description: "", actions: [], detail: {}, ...extra }) as ServiceRecord;

const status = (patch: Record<string, unknown> = {}): Status =>
  ({
    node: { label: "k7m2-9x4p-aa", alias: null, category: "Node2B", version: "2026.10.13" },
    layers: [0, 1, 2, 3],
    offer: { tier: "standard", state: "offered" },
    intelligence: { agent: "running", model_server: "running", agent_state: "local", model_id: "qwen" },
    auth: { loopback: true, paired: true },
    ...patch,
  }) as unknown as Status;

function mem(initial: Record<string, string> = {}): KeyValueStore & { map: Map<string, string> } {
  const map = new Map(Object.entries(initial));
  return { map, getItem: (k) => map.get(k) ?? null, setItem: (k, v) => void map.set(k, v) };
}

// ---------------------------------------------------------------- agentState

describe("agentState", () => {
  const cases: { name: string; agent: ServiceRecord["state"]; model: ServiceRecord["state"]; agentState?: string; kind: string; canSend: boolean; dot: string }[] = [
    { name: "agent live + model live", agent: "live", model: "live", kind: "ready", canSend: true, dot: "live" },
    { name: "agent starting", agent: "starting", model: "live", kind: "starting", canSend: false, dot: "starting" },
    { name: "model starting", agent: "live", model: "starting", kind: "starting", canSend: false, dot: "starting" },
    { name: "both starting", agent: "starting", model: "starting", kind: "starting", canSend: false, dot: "starting" },
    { name: "model unset", agent: "live", model: "unset", kind: "no-model", canSend: false, dot: "unset" },
    { name: "agent error", agent: "error", model: "live", kind: "unavailable", canSend: false, dot: "error" },
    { name: "agent unset", agent: "unset", model: "live", kind: "unavailable", canSend: false, dot: "unset" },
    { name: "agent error beats model unset", agent: "error", model: "unset", kind: "unavailable", canSend: false, dot: "error" },
    { name: "agent starting beats model unset", agent: "starting", model: "unset", kind: "starting", canSend: false, dot: "starting" },
    { name: "model error", agent: "live", model: "error", kind: "unavailable", canSend: false, dot: "error" },
    { name: "offloaded, model unset locally", agent: "live", model: "unset", agentState: "offloaded", kind: "offloaded", canSend: true, dot: "live" },
    { name: "offloaded, model error locally", agent: "live", model: "error", agentState: "offloaded", kind: "offloaded", canSend: true, dot: "live" },
    { name: "offloaded but the agent itself is in error", agent: "error", model: "unset", agentState: "offloaded", kind: "unavailable", canSend: false, dot: "error" },
    { name: "agent_state unavailable does not override a live registry", agent: "live", model: "live", agentState: "unavailable", kind: "ready", canSend: true, dot: "live" },
  ];
  for (const c of cases) {
    it(c.name, () => {
      const s = agentState([svc("agent", c.agent), svc("model", c.model)], status({ intelligence: { agent_state: c.agentState ?? "local" } }));
      expect(s.kind).toBe(c.kind);
      expect(s.canSend).toBe(c.canSend);
      expect(s.dot).toBe(c.dot);
    });
  }

  it("is loading (sends are attempted) before the first services snapshot", () => {
    const s = agentState(null, null);
    expect(s.kind).toBe("loading");
    expect(s.canSend).toBe(true);
    expect(s.label).toBe("connecting");
  });

  it("uses the chip words from the spec", () => {
    const label = (a: ServiceRecord["state"], m: ServiceRecord["state"], st = status()) => agentState([svc("agent", a), svc("model", m)], st).label;
    expect(label("live", "live")).toBe("ready");
    expect(label("starting", "live")).toBe("starting");
    expect(label("live", "unset")).toBe("no model yet");
    expect(label("error", "live")).toBe("unavailable");
    expect(label("live", "unset", status({ intelligence: { agent_state: "offloaded" } }))).toBe("served by hub");
  });

  it("carries the plain reason and the offer state", () => {
    const down = agentState([svc("agent", "error", { reason: "The agent stopped unexpectedly." }), svc("model", "live")], status());
    expect(down.reason).toBe("The agent stopped unexpectedly.");
    const nomodel = agentState([svc("agent", "live"), svc("model", "unset", { can_run_here: false })], status({ offer: { tier: "standard", state: "accepted" } }));
    expect(nomodel.offer).toBe("accepted");
    expect(nomodel.modelCanRun).toBe(false);
    expect(agentState([svc("agent", "live"), svc("model", "unset")], status({ offer: null })).offer).toBeNull();
  });

  it("treats a missing agent or model record as unavailable", () => {
    expect(agentState([svc("model", "live")], status()).kind).toBe("unavailable");
    expect(agentState([svc("agent", "live")], status()).kind).toBe("unavailable");
  });

  it("builds a plain queued sentence per state", () => {
    const down = agentState([svc("agent", "error", { reason: "The agent stopped unexpectedly." }), svc("model", "live")], status());
    expect(queuedNotice(down)).toBe("The agent is unavailable: The agent stopped unexpectedly. Your message is queued and sends when it is back.");
    expect(queuedNotice(agentState([svc("agent", "starting"), svc("model", "live")], status()))).toMatch(/starting/);
    expect(queuedNotice(agentState([svc("agent", "live"), svc("model", "unset")], status()))).toMatch(/no local model/);
    expect(reasonClause("Done. ")).toBe("Done");
    expect(reasonClause(null)).toBe("");
  });
});

// ---------------------------------------------------------------- filterHistory

describe("filterHistory", () => {
  it("hides tool rows, system rows and assistant rows without content", () => {
    const messages: HistoryMessage[] = [
      { role: "system", content: "You are the node agent." },
      { role: "user", content: "hi" },
      { role: "assistant", content: null, tool_calls: [{ id: "c1" }] },
      { role: "tool", content: "{}" },
      { role: "assistant", content: "" },
      { role: "assistant", content: "   " },
      { role: "assistant" },
      { role: "assistant", content: "Hello" },
    ];
    expect(filterHistory(messages)).toEqual([
      { role: "user", content: "hi" },
      { role: "assistant", content: "Hello" },
    ]);
  });
  it("keeps an assistant row that has both text and tool calls, and the order", () => {
    const m: HistoryMessage[] = [
      { role: "user", content: "a" },
      { role: "assistant", content: "Let me look.", tool_calls: [{ id: "c1" }] },
      { role: "tool", content: "x" },
      { role: "assistant", content: "Done." },
    ];
    expect(filterHistory(m).map((x) => x.content)).toEqual(["a", "Let me look.", "Done."]);
  });
  it("tolerates null and undefined", () => {
    expect(filterHistory(null)).toEqual([]);
    expect(filterHistory(undefined)).toEqual([]);
  });
});

// ---------------------------------------------------------------- TokenBuffer

describe("TokenBuffer", () => {
  it("flushes the first text at once, then at most every 100 ms", () => {
    const clock = new FakeClock(0);
    const buf = new TokenBuffer(clock.now);
    expect(buf.waitMs()).toBe(Infinity);
    buf.push("Hel");
    expect(buf.waitMs()).toBe(0);
    expect(buf.flush()).toBe("Hel");
    buf.push("lo");
    expect(buf.dirty).toBe(true);
    expect(buf.waitMs()).toBe(100);
    expect(buf.flush()).toBeNull();
    clock.advance(60);
    expect(buf.waitMs()).toBe(40);
    expect(buf.flush()).toBeNull();
    clock.advance(40);
    expect(buf.flush()).toBe("Hello");
    expect(buf.dirty).toBe(false);
    expect(buf.waitMs()).toBe(Infinity);
  });

  it("never exceeds 10 flushes a second under a dense stream", () => {
    const clock = new FakeClock(0);
    const buf = new TokenBuffer(clock.now);
    let flushes = 0;
    for (let t = 0; t < 1000; t += 5) {
      buf.push("x");
      if (buf.flush() !== null) flushes++;
      clock.advance(5);
    }
    expect(flushes).toBeLessThanOrEqual(11); // leading edge + one per 100 ms
    expect(flushes).toBeGreaterThanOrEqual(10);
    expect(buf.take()).toHaveLength(200);
  });

  it("take() ignores the limit and keeps the whole text; reset() starts over", () => {
    const clock = new FakeClock(0);
    const buf = new TokenBuffer(clock.now);
    buf.push("a");
    buf.take();
    buf.push("b");
    expect(buf.flush()).toBeNull();
    expect(buf.take()).toBe("ab");
    expect(buf.text).toBe("ab");
    buf.reset();
    expect(buf.text).toBe("");
    expect(buf.dirty).toBe(false);
    buf.push("c");
    expect(buf.waitMs()).toBe(0);
  });

  it("honours a custom interval", () => {
    const clock = new FakeClock(0);
    const buf = new TokenBuffer(clock.now, 250);
    buf.push("a");
    buf.take();
    buf.push("b");
    expect(buf.waitMs()).toBe(250);
  });
});

// ---------------------------------------------------------------- conversation id

describe("conversation id", () => {
  it("is conv- plus 24 characters of [a-z0-9]", () => {
    for (let i = 0; i < 50; i++) {
      const id = mintConversationId();
      expect(id).toMatch(CONVERSATION_ID_RE);
      expect(id).toHaveLength(29);
    }
    expect(new Set(Array.from({ length: 50 }, () => mintConversationId())).size).toBe(50);
  });

  it("maps bytes to the alphabet and skips biased bytes (>= 252)", () => {
    const seq = [255, 253, 252, 0, 1, 2, 25, 26, 35, 36, 71, 251];
    let call = 0;
    const id = mintConversationId((bytes) => {
      // The first call yields the interesting bytes, later calls cycle through valid ones.
      for (let i = 0; i < bytes.length; i++) bytes[i] = call === 0 ? (seq[i % seq.length] as number) : i;
      call++;
      return bytes;
    });
    expect(id).toMatch(CONVERSATION_ID_RE);
    // 255, 253 and 252 are skipped; then 0->a 1->b 2->c 25->z 26->0 35->9 36->a 71->9 251->9.
    expect(id.slice(5, 14)).toBe("abcz09a99");
  });

  it("is stored under 111.chat.conversation and reused", () => {
    const store = mem();
    const first = loadConversationId(store);
    expect(store.map.get(CONVERSATION_KEY)).toBe(first);
    expect(loadConversationId(store)).toBe(first);
  });

  it("mints a new one when the stored value breaks the contract", () => {
    const store = mem({ [CONVERSATION_KEY]: "not valid!" });
    const id = loadConversationId(store);
    expect(id).toMatch(CONVERSATION_ID_RE);
    expect(store.map.get(CONVERSATION_KEY)).toBe(id);
    expect(loadConversationId(mem({ [CONVERSATION_KEY]: "x".repeat(129) }))).toMatch(CONVERSATION_ID_RE);
    expect(loadConversationId(mem({ [CONVERSATION_KEY]: "legacy_ID-1" }))).toBe("legacy_ID-1");
  });

  it("survives storage that throws or is missing", () => {
    const broken: KeyValueStore = {
      getItem() {
        throw new Error("blocked");
      },
      setItem() {
        throw new Error("blocked");
      },
    };
    expect(loadConversationId(broken)).toMatch(CONVERSATION_ID_RE);
    expect(loadConversationId(null)).toMatch(CONVERSATION_ID_RE);
  });
});

// ---------------------------------------------------------------- small pure helpers

describe("toolLine and whyPrompt", () => {
  it("names the agent's fixed tools in plain words and shows unknown ones safely", () => {
    expect(toolLine("kb.read")).toBe("[reading kb]");
    expect(toolLine("logs.read")).toBe("[reading logs]");
    expect(toolLine("mystery.tool")).toBe("[using tool: mystery.tool]");
    expect(toolLine("evil\u001b[31m")).toBe("[using tool: evil[31m]");
    expect(toolLine("")).toBe("[using a tool]");
  });

  it("stages the explain sentence in front of the node check prompt", () => {
    const p = whyPrompt({ id: "prop-1", title: 'Raise "ctx"\u0007' }, status());
    expect(p.startsWith(`Explain proposal prop-1 ("Raise 'ctx'") in plain words: what it changes, the risk, and how to undo it.`)).toBe(true);
    expect(p).toContain("Check this node end to end and tell me what needs attention.");
    expect(p).toContain("category Node2B");
  });
});

// ---------------------------------------------------------------- ChatSession

type Fed = ChatFrame | Error | "end";

/** A chat.send whose frames the test pushes by hand. */
function feed() {
  const queue: Fed[] = [];
  let wake: (() => void) | null = null;
  const calls: { conversation_id: string; message: string; signal?: AbortSignal | undefined }[] = [];
  let closed = false;
  async function* send(req: { conversation_id: string; message: string }, opts?: { signal?: AbortSignal }): AsyncGenerator<ChatFrame, void, void> {
    calls.push({ ...req, signal: opts?.signal });
    const signal = opts?.signal;
    // Like the real adapters: an aborted signal ends the wait with an AbortError.
    const aborted = new Promise<never>((_, reject) => {
      const fail = () => reject(new DOMException("The operation was aborted.", "AbortError"));
      if (signal?.aborted) fail();
      else signal?.addEventListener("abort", fail, { once: true });
    });
    aborted.catch(() => {});
    try {
      for (;;) {
        while (queue.length === 0) await Promise.race([new Promise<void>((r) => (wake = r)), aborted]);
        const next = queue.shift() as Fed;
        if (next === "end") return;
        if (next instanceof Error) throw next;
        yield next;
      }
    } finally {
      closed = true;
    }
  }
  return {
    send,
    calls,
    get closed() {
      return closed;
    },
    push(...frames: Fed[]) {
      queue.push(...frames);
      wake?.();
    },
  };
}

const token = (text: string): ChatFrame => ({ event: "token", data: { text } });
const tool = (name: string): ChatFrame => ({ event: "tool", data: { name } });
const done = (reply: string): ChatFrame => ({ event: "done", data: { conversation_id: "c", reply, tool_trace: [] } });
const unavailable = (detail = "The agent stopped unexpectedly."): ApiError =>
  new ApiError({ type: "about:blank", title: "The local agent is not available", status: 503, code: "agent_unavailable", detail });

function setup(opts: { history?: HistoryMessage[]; historyError?: unknown; sends?: ReturnType<typeof feed>[] } = {}) {
  const clock = new FakeClock(1_000);
  const store = mem();
  const announced: string[] = [];
  const unauth: unknown[] = [];
  const feeds = opts.sends ?? [feed()];
  let n = 0;
  const adapter = {
    chat: {
      send: (req: { conversation_id: string; message: string }, o?: { signal?: AbortSignal }) => {
        const f = feeds[Math.min(n++, feeds.length - 1)] as ReturnType<typeof feed>;
        return f.send(req, o);
      },
      history: async (id: string) => {
        if (opts.historyError) throw opts.historyError;
        return { schema: 1, conversation_id: id, messages: opts.history ?? [] };
      },
    },
  };
  const deps: SessionDeps = {
    getAdapter: async () => adapter as unknown as Awaited<ReturnType<SessionDeps["getAdapter"]>>,
    announce: (t) => announced.push(t),
    onUnauthenticated: (e) => {
      unauth.push(e);
      return e instanceof ApiError && e.unauthenticated;
    },
    now: clock.now,
    setTimer: clock.setTimer,
    clearTimer: clock.clearTimer,
    store,
  };
  const session = new ChatSession(deps);
  const liveTexts: string[] = [];
  session.subscribe(() => {
    const t = session.getSnapshot().live?.text;
    if (t !== undefined && liveTexts[liveTexts.length - 1] !== t) liveTexts.push(t);
  });
  return { session, clock, store, announced, unauth, feeds, liveTexts };
}

describe("ChatSession streaming", () => {
  it("batches tokens to 10 Hz, then replaces the live bubble with the reply on done and keeps tool lines", async () => {
    const t = setup();
    const f = t.feeds[0] as ReturnType<typeof feed>;
    expect(t.session.send("  hello  ")).toBe("sent");
    await flush();
    let snap = t.session.getSnapshot();
    expect(snap.rows).toEqual([{ key: expect.any(String), role: "user", text: "hello" }]);
    expect(snap.live?.text).toBe("");
    expect(f.calls[0]).toMatchObject({ conversation_id: snap.conversationId, message: "hello" });
    expect(t.announced).toEqual(["agent is replying"]);

    f.push(token("Let"), token(" me"), token(" check"), tool("kb.read"));
    await flush();
    snap = t.session.getSnapshot();
    expect(snap.live?.text).toBe("Let"); // leading edge only; the rest waits for the 100 ms window
    expect(snap.live?.tools).toEqual(["[reading kb]"]);
    t.clock.advance(100);
    expect(t.session.getSnapshot().live?.text).toBe("Let me check");

    f.push(token(" the kb. "), token("All good."), done("All good."));
    await flush();
    snap = t.session.getSnapshot();
    expect(snap.live).toBeNull();
    expect(snap.rows).toHaveLength(2);
    expect(snap.rows[1]).toMatchObject({ role: "assistant", text: "All good.", tools: ["[reading kb]"] });
    expect(t.announced).toEqual(["agent is replying", "reply complete"]);
    expect(f.closed).toBe(true);
    expect(t.session.isStreaming).toBe(false);
    // The flushed texts only ever grew, one per 100 ms window.
    expect(t.liveTexts.filter((x) => x !== "")).toEqual(["Let", "Let me check"]);
  });

  it("falls back to the streamed text when done carries an empty reply", async () => {
    const t = setup();
    const f = t.feeds[0] as ReturnType<typeof feed>;
    t.session.send("q");
    await flush();
    f.push(token("streamed answer"), done("   "));
    await flush();
    expect(t.session.getSnapshot().rows[1]).toMatchObject({ text: "streamed answer" });
  });

  it("shows an error frame in the bubble and keeps the user's message", async () => {
    const t = setup();
    const f = t.feeds[0] as ReturnType<typeof feed>;
    t.session.send("question");
    await flush();
    f.push(token("partial "), { event: "error", data: { error: "model crashed", code: "model_error" } });
    await flush();
    const snap = t.session.getSnapshot();
    expect(snap.live).toBeNull();
    expect(snap.rows[0]).toMatchObject({ role: "user", text: "question" });
    expect(snap.rows[1]).toMatchObject({ role: "assistant", text: "partial ", error: "model crashed" });
    expect(snap.queued).toEqual([]);
  });

  it("turns a stream that ends without a terminal frame into a visible error", async () => {
    const t = setup();
    const f = t.feeds[0] as ReturnType<typeof feed>;
    t.session.send("q");
    await flush();
    f.push(token("half"), "end");
    await flush();
    const snap = t.session.getSnapshot();
    expect(snap.rows[1]).toMatchObject({ text: "half", error: expect.stringContaining("ended") });
  });

  it("shows other API errors under the user's message", async () => {
    const f = feed();
    const t = setup({ sends: [f] });
    t.session.send("q");
    await flush();
    f.push(new ApiError({ type: "about:blank", title: "Slow down", status: 429, code: "rate_limited", detail: "Too many messages." }));
    await flush();
    const snap = t.session.getSnapshot();
    expect(snap.rows.map((r) => r.role)).toEqual(["user", "assistant"]);
    expect(snap.rows[1]).toMatchObject({ error: "Too many messages." });
    expect(snap.queued).toEqual([]);
  });

  it("refuses a second send while streaming and over-long or empty messages", async () => {
    const t = setup();
    expect(t.session.send("   ")).toBe("rejected");
    expect(t.session.send("x".repeat(16001))).toBe("rejected");
    expect(t.session.getSnapshot().notice).toMatch(/16000/);
    t.session.send("one");
    await flush();
    expect(t.session.send("two")).toBe("rejected");
    expect(t.session.getSnapshot().rows).toHaveLength(1);
  });
});

describe("ChatSession stop", () => {
  it("aborts the request, closes the iterator and keeps what arrived", async () => {
    const t = setup();
    const f = t.feeds[0] as ReturnType<typeof feed>;
    t.session.send("tell me");
    await flush();
    f.push(token("Starting to"), tool("logs.read"));
    await flush();
    t.session.stop();
    await flush();
    const snap = t.session.getSnapshot();
    expect(f.calls[0]?.signal?.aborted).toBe(true);
    expect(f.closed).toBe(true);
    expect(snap.live).toBeNull();
    expect(t.session.isStreaming).toBe(false);
    expect(snap.rows[1]).toMatchObject({ role: "assistant", text: "Starting to", stopped: true, tools: ["[reading logs]"] });
    expect(snap.rows.every((r) => r.error === undefined)).toBe(true);
    expect(t.announced[t.announced.length - 1]).toBe("reply stopped");
    // And the box is free again.
    expect(t.session.send("again")).toBe("sent");
  });

  it("drops the bubble when nothing had arrived yet", async () => {
    const t = setup();
    t.session.send("q");
    await flush();
    t.session.stop();
    await flush();
    const snap = t.session.getSnapshot();
    expect(snap.rows.map((r) => r.role)).toEqual(["user"]);
    expect(snap.live).toBeNull();
  });
});

describe("ChatSession queue", () => {
  it("queues while the agent cannot serve and sends in order when it can", async () => {
    const f1 = feed();
    const f2 = feed();
    const t = setup({ sends: [f1, f2] });
    t.session.setServing(false, "The agent is unavailable: stopped. Your message is queued.");
    expect(t.session.send("first")).toBe("queued");
    expect(t.session.send("second")).toBe("queued");
    let snap = t.session.getSnapshot();
    expect(snap.queued.map((q) => q.text)).toEqual(["first", "second"]);
    expect(snap.notice).toBe("The agent is unavailable: stopped. Your message is queued.");
    expect(f1.calls).toHaveLength(0);

    t.session.setServing(true, "");
    await flush();
    snap = t.session.getSnapshot();
    expect(f1.calls.map((c) => c.message)).toEqual(["first"]);
    expect(snap.queued.map((q) => q.text)).toEqual(["second"]);
    f1.push(token("ok"), done("ok"));
    await flush();
    // The next queued message goes out once the first reply is complete.
    expect(f2.calls.map((c) => c.message)).toEqual(["second"]);
    f2.push(done("fine"));
    await flush();
    snap = t.session.getSnapshot();
    expect(snap.queued).toEqual([]);
    expect(snap.rows.map((r) => `${r.role}:${r.text}`)).toEqual(["user:first", "assistant:ok", "user:second", "assistant:fine"]);
  });

  it("moves a message to the queue when the send itself answers agent_unavailable, and releases it when the state returns", async () => {
    const f1 = feed();
    const f2 = feed();
    const t = setup({ sends: [f1, f2] });
    expect(t.session.send("hello")).toBe("sent");
    await flush();
    f1.push(unavailable());
    await flush();
    let snap = t.session.getSnapshot();
    expect(snap.rows).toEqual([]);
    expect(snap.live).toBeNull();
    expect(snap.queued.map((q) => q.text)).toEqual(["hello"]);
    expect(snap.notice).toBe("The agent stopped unexpectedly. Your message is queued and sends when the agent is back.");

    // Same state again: the queue stays put (no retry loop).
    t.session.setServing(true, "");
    await flush();
    expect(f2.calls).toHaveLength(0);

    // The state goes away and comes back: the queue is released.
    t.session.setServing(false, "down");
    t.session.setServing(true, "");
    await flush();
    expect(f2.calls.map((c) => c.message)).toEqual(["hello"]);
    f2.push(done("hi!"));
    await flush();
    snap = t.session.getSnapshot();
    expect(snap.queued).toEqual([]);
    expect(snap.rows.map((r) => r.text)).toEqual(["hello", "hi!"]);
  });

  it("also treats model_unavailable as queue-worthy, and 'try now' retries", async () => {
    const f1 = feed();
    const f2 = feed();
    const t = setup({ sends: [f1, f2] });
    t.session.send("hello");
    await flush();
    f1.push(new ApiError({ type: "about:blank", title: "No model", status: 503, code: "model_unavailable", detail: "No local model is installed yet." }));
    await flush();
    expect(t.session.getSnapshot().queued).toHaveLength(1);
    t.session.retryQueued();
    await flush();
    expect(f2.calls).toHaveLength(1);
  });

  it("lets the user remove a queued message", () => {
    const t = setup();
    t.session.setServing(false, "n");
    t.session.send("a");
    const id = t.session.getSnapshot().queued[0]?.id as string;
    t.session.removeQueued(id);
    expect(t.session.getSnapshot().queued).toEqual([]);
  });
});

describe("ChatSession auth", () => {
  it("redirects on 401, removes the row and puts the message back in the input", async () => {
    const f = feed();
    const t = setup({ sends: [f] });
    t.session.send("secret question");
    await flush();
    const seq = t.session.getSnapshot().draftSeq;
    f.push(new ApiError({ type: "about:blank", title: "Pairing required", status: 401, code: "unauthenticated" }));
    await flush();
    const snap = t.session.getSnapshot();
    expect(t.unauth).toHaveLength(1);
    expect(snap.rows).toEqual([]);
    expect(snap.queued).toEqual([]);
    expect(t.session.draft).toBe("secret question");
    expect(snap.draftSeq).toBe(seq + 1);
  });
});

describe("ChatSession conversation", () => {
  it("starts with a stored or minted id and persists a new one on newConversation", async () => {
    const t = setup();
    const first = t.session.getSnapshot().conversationId;
    expect(first).toMatch(CONVERSATION_ID_RE);
    expect(t.store.map.get(CONVERSATION_KEY)).toBe(first);

    t.session.setServing(false, "n");
    t.session.send("queued one");
    t.session.newConversation();
    const snap = t.session.getSnapshot();
    expect(snap.conversationId).not.toBe(first);
    expect(snap.conversationId).toMatch(CONVERSATION_ID_RE);
    expect(t.store.map.get(CONVERSATION_KEY)).toBe(snap.conversationId);
    expect(snap.rows).toEqual([]);
    expect(snap.queued).toEqual([]);
    expect(snap.notice).toBeNull();
  });

  it("a stream that is still running when the conversation is replaced never touches the new one", async () => {
    const f = feed();
    const t = setup({ sends: [f] });
    t.session.send("old");
    await flush();
    f.push(token("late"));
    await flush();
    t.session.newConversation();
    f.push(token(" tokens"), done("late reply"));
    await flush();
    t.clock.advance(500);
    const snap = t.session.getSnapshot();
    expect(snap.rows).toEqual([]);
    expect(snap.live).toBeNull();
    expect(t.session.isStreaming).toBe(false);
    expect(f.calls[0]?.signal?.aborted).toBe(true);
  });

  it("loads the history once and shows only user and assistant text", async () => {
    const t = setup({
      history: [
        { role: "system", content: "s" },
        { role: "user", content: "hi" },
        { role: "assistant", content: null, tool_calls: [{ id: "1" }] },
        { role: "tool", content: "{}" },
        { role: "assistant", content: "Hello" },
      ],
    });
    await t.session.loadHistory();
    await t.session.loadHistory();
    const snap = t.session.getSnapshot();
    expect(snap.history).toBe("ready");
    expect(snap.rows.map((r) => `${r.role}:${r.text}`)).toEqual(["user:hi", "assistant:Hello"]);
  });

  it("marks history unavailable on agent_unavailable and retries when the state returns", async () => {
    let fail = true;
    const f = feed();
    const t = setup({ sends: [f] });
    const adapter = (await (t.session as unknown as { deps: SessionDeps }).deps.getAdapter()) as unknown as { chat: { history: (id: string) => Promise<unknown> } };
    adapter.chat.history = async (id: string) => {
      if (fail) throw unavailable();
      return { schema: 1, conversation_id: id, messages: [{ role: "user", content: "back" }] };
    };
    await t.session.loadHistory();
    expect(t.session.getSnapshot().history).toBe("unavailable");
    fail = false;
    t.session.setServing(false, "n");
    t.session.setServing(true, "");
    await flush();
    expect(t.session.getSnapshot().history).toBe("ready");
    expect(t.session.getSnapshot().rows.map((r) => r.text)).toEqual(["back"]);
  });

  it("does not overwrite messages sent while the history was loading", async () => {
    const t = setup({ history: [{ role: "user", content: "old" }] });
    const load = t.session.loadHistory();
    t.session.send("new");
    await load;
    await flush();
    expect(t.session.getSnapshot().rows.map((r) => r.text)).toEqual(["new"]);
  });
});

describe("ChatSession staged text", () => {
  it("puts a prepared prompt into the input with a banner, never sends it, and can dismiss the banner", () => {
    const t = setup();
    t.session.stage("Set up the LoRa radio", "LoRa radio");
    const snap = t.session.getSnapshot();
    expect(t.session.draft).toBe("Set up the LoRa radio");
    expect(snap.draftSeq).toBe(1);
    expect(snap.prepared).toEqual({ from: "LoRa radio", text: "Set up the LoRa radio" });
    expect(snap.rows).toEqual([]);
    expect((t.feeds[0] as ReturnType<typeof feed>).calls).toHaveLength(0);
    t.session.dismissPrepared();
    expect(t.session.getSnapshot().prepared).toBeNull();
    expect(t.session.draft).toBe("Set up the LoRa radio");
    t.session.stage("plain text");
    expect(t.session.getSnapshot().prepared).toBeNull();
    expect(t.session.getSnapshot().draftSeq).toBe(2);
  });
});
