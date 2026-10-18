// The chat session behind the Agent view: one module-level object that outlives the view, so a
// reply keeps streaming while the user looks at another page and the transcript is still there on
// return. Plain TypeScript with an external-store surface (`subscribe` / `getSnapshot`); every
// side effect (adapter, announcer, redirect, clock, storage) is injected, so it runs under node.
import type { Adapter } from "../../lib/api/adapter";
import { isApiError } from "../../lib/api/problem";
import type { ChatFrame } from "../../lib/api/sse.gen";
import { FLUSH_INTERVAL_MS, TokenBuffer } from "./token-buffer";
import {
  filterHistory,
  loadConversationId,
  MAX_MESSAGE_CHARS,
  mintConversationId,
  persistConversationId,
  toolLine,
  type ChatRow,
  type KeyValueStore,
  type RandomFill,
} from "./state";

export interface QueuedMessage {
  id: string;
  text: string;
}

/** The reply that is being streamed right now. */
export interface LiveTurn {
  key: string;
  /** Text flushed to React so far (at most 10 updates a second). */
  text: string;
  tools: string[];
}

export type HistoryState = "idle" | "loading" | "ready" | "unavailable" | "error";

export interface ChatSnapshot {
  conversationId: string;
  rows: ChatRow[];
  live: LiveTurn | null;
  queued: QueuedMessage[];
  /** One plain sentence about why a message did not go out; null when there is nothing to say. */
  notice: string | null;
  history: HistoryState;
  /** Set when a prompt was handed over from another page. */
  prepared: { from: string; text: string } | null;
  /** Bumps whenever the session puts new text into the input (staged prompt, restored message). */
  draftSeq: number;
}

export interface SessionDeps {
  getAdapter: () => Promise<Pick<Adapter, "chat">>;
  announce: (text: string) => void;
  /** Routes to /pair for a 401 and returns true. */
  onUnauthenticated: (err: unknown) => boolean;
  now?: () => number;
  setTimer?: (fn: () => void, ms: number) => unknown;
  clearTimer?: (handle: unknown) => void;
  store?: KeyValueStore | null;
  random?: RandomFill;
}

export type SendResult = "sent" | "queued" | "rejected";

const UNAVAILABLE_CODES = new Set(["agent_unavailable", "model_unavailable"]);

export class ChatSession {
  private snap: ChatSnapshot;
  private listeners = new Set<() => void>();
  private readonly buf: TokenBuffer;
  private readonly setTimer: (fn: () => void, ms: number) => unknown;
  private readonly clearTimer: (handle: unknown) => void;

  private serving = true;
  private servingNotice = "The agent is not ready. Your message is queued and sends when it is.";
  /** A send failed with agent/model unavailable: hold the queue until the state changes or the user retries. */
  private blocked = false;
  private streaming = false;
  private abort: AbortController | null = null;
  private timer: unknown = null;
  /** Bumped by `newConversation`; an older run never touches the new conversation. */
  private gen = 0;
  private seq = 0;
  private draftText = "";

  constructor(private readonly deps: SessionDeps) {
    const now = deps.now ?? (() => Date.now());
    this.setTimer = deps.setTimer ?? ((fn, ms) => setTimeout(fn, ms));
    this.clearTimer = deps.clearTimer ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>));
    this.buf = new TokenBuffer(now, FLUSH_INTERVAL_MS);
    this.snap = {
      conversationId: loadConversationId(deps.store, deps.random),
      rows: [],
      live: null,
      queued: [],
      notice: null,
      history: "idle",
      prepared: null,
      draftSeq: 0,
    };
  }

  // --- external store surface ------------------------------------------------------------------
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  getSnapshot = (): ChatSnapshot => this.snap;

  private set(patch: Partial<ChatSnapshot>): void {
    this.snap = { ...this.snap, ...patch };
    for (const l of [...this.listeners]) l();
  }

  private key(prefix: string): string {
    return `${prefix}${++this.seq}`;
  }

  get isStreaming(): boolean {
    return this.streaming;
  }

  // --- input text ------------------------------------------------------------------------------
  /** The current input text (kept here so it survives leaving the page). */
  get draft(): string {
    return this.draftText;
  }
  typeDraft(text: string): void {
    this.draftText = text;
  }
  /** Put text into the input, editable and never sent. `from` adds the "prepared from" banner. */
  stage(text: string, from?: string): void {
    this.draftText = text;
    this.set({ draftSeq: this.snap.draftSeq + 1, prepared: from ? { from, text } : null });
  }
  dismissPrepared(): void {
    if (this.snap.prepared) this.set({ prepared: null });
  }
  clearNotice(): void {
    if (this.snap.notice) this.set({ notice: null });
  }

  // --- agent state -----------------------------------------------------------------------------
  /**
   * Tell the session whether the agent can take a message now. `notice` is the sentence shown when
   * a message is queued instead. Becoming able to send releases the queue and retries the history.
   */
  setServing(serving: boolean, notice: string): void {
    const was = this.serving;
    this.serving = serving;
    this.servingNotice = notice;
    if (serving && !was) {
      this.blocked = false;
      void this.loadHistory();
      this.pump();
    }
  }

  // --- history ---------------------------------------------------------------------------------
  /** Load the stored conversation once; retried after an unavailable/failed attempt. */
  async loadHistory(): Promise<void> {
    const h = this.snap.history;
    if (h === "loading" || h === "ready") return;
    const id = this.snap.conversationId;
    const gen = this.gen;
    this.set({ history: "loading" });
    try {
      const adapter = await this.deps.getAdapter();
      const res = await adapter.chat.history(id);
      if (gen !== this.gen) return;
      const rows: ChatRow[] = filterHistory(res.messages).map((m) => ({
        key: this.key("h"),
        role: m.role === "user" ? "user" : "assistant",
        text: String(m.content),
      }));
      // A message sent while the history was loading wins; the stored turns would only duplicate it.
      if (this.snap.rows.length > 0 || this.streaming) this.set({ history: "ready" });
      else this.set({ history: "ready", rows });
    } catch (err) {
      if (gen !== this.gen) return;
      if (this.deps.onUnauthenticated(err)) {
        this.set({ history: "idle" });
        return;
      }
      this.set({ history: isApiError(err) && UNAVAILABLE_CODES.has(err.code) ? "unavailable" : "error" });
    }
  }

  // --- sending ---------------------------------------------------------------------------------
  /** `sent`: on its way. `queued`: held locally with a notice. `rejected`: nothing happened, keep the text. */
  send(raw: string): SendResult {
    const text = raw.trim();
    if (!text || this.streaming) return "rejected";
    if (text.length > MAX_MESSAGE_CHARS) {
      this.set({ notice: `That message is ${text.length} characters; the limit is ${MAX_MESSAGE_CHARS}. Shorten it and send again.` });
      return "rejected";
    }
    if (this.serving && !this.blocked && this.snap.queued.length === 0) {
      void this.run(text);
      return "sent";
    }
    this.set({ queued: [...this.snap.queued, { id: this.key("q"), text }], notice: this.servingNotice });
    return "queued";
  }

  removeQueued(id: string): void {
    this.set({ queued: this.snap.queued.filter((q) => q.id !== id) });
  }

  /** The user asked to try the queue again now. */
  retryQueued(): void {
    this.blocked = false;
    this.pump();
  }

  private pump(): void {
    if (!this.serving || this.streaming || this.blocked) return;
    const [next, ...rest] = this.snap.queued;
    if (!next) return;
    this.set({ queued: rest });
    void this.run(next.text, next.id);
  }

  /** Stop the reply that is streaming; what has arrived stays in the transcript. */
  stop(): void {
    this.abort?.abort();
  }

  /** Mint a new id, drop the transcript and any queued messages. The old conversation stays on the node. */
  newConversation(): void {
    this.gen++;
    this.abort?.abort();
    this.cancelTimer();
    this.buf.reset();
    this.streaming = false;
    this.blocked = false;
    const id = mintConversationId(this.deps.random);
    persistConversationId(id, this.deps.store);
    this.set({ conversationId: id, rows: [], live: null, queued: [], notice: null, history: "ready" });
  }

  // --- the streaming turn ----------------------------------------------------------------------
  private cancelTimer(): void {
    if (this.timer !== null) {
      this.clearTimer(this.timer);
      this.timer = null;
    }
  }

  /** Hand buffered text to React when the 10 Hz limit allows, else arrange to do it when it does. */
  private schedule(gen: number): void {
    if (this.timer !== null) return;
    const wait = this.buf.waitMs();
    if (wait === Number.POSITIVE_INFINITY) return;
    if (wait <= 0) {
      this.applyFlush(gen);
      return;
    }
    this.timer = this.setTimer(() => {
      this.timer = null;
      this.applyFlush(gen);
    }, wait);
  }

  private applyFlush(gen: number): void {
    if (gen !== this.gen || !this.snap.live) return;
    const text = this.buf.flush();
    if (text === null) {
      this.schedule(gen); // a timer that fired a hair early
      return;
    }
    this.set({ live: { ...this.snap.live, text } });
  }

  private async run(text: string, queueId?: string): Promise<void> {
    const gen = this.gen;
    const userRow: ChatRow = { key: this.key("u"), role: "user", text };
    const liveKey = this.key("a");
    const conversationId = this.snap.conversationId;
    this.streaming = true;
    this.blocked = false;
    this.buf.reset();
    const ac = new AbortController();
    this.abort = ac;
    this.set({ rows: [...this.snap.rows, userRow], live: { key: liveKey, text: "", tools: [] }, notice: null });
    this.deps.announce("agent is replying");

    const tools: string[] = [];
    let terminal: "done" | "error" | "none" = "none";
    let failure: { message: string; code?: string } | null = null;
    let reply = "";
    let iterator: AsyncIterator<ChatFrame> | null = null;
    const close = () => {
      if (iterator) void Promise.resolve(iterator.return?.()).catch(() => {});
    };

    try {
      const adapter = await this.deps.getAdapter();
      iterator = adapter.chat.send({ conversation_id: conversationId, message: text }, { signal: ac.signal })[Symbol.asyncIterator]();
      const aborted = new Promise<"aborted">((resolve) => {
        if (ac.signal.aborted) resolve("aborted");
        else ac.signal.addEventListener("abort", () => resolve("aborted"), { once: true });
      });
      for (;;) {
        const step = await Promise.race([iterator.next(), aborted]);
        if (step === "aborted") break; // leave at once; closing the iterator lets the adapter cancel
        if (step.done) break;
        const frame: ChatFrame = step.value;
        if (gen !== this.gen) break;
        if (frame.event === "token") {
          this.buf.push(frame.data.text);
          this.schedule(gen);
        } else if (frame.event === "tool") {
          tools.push(toolLine(frame.data.name));
          if (this.snap.live) this.set({ live: { ...this.snap.live, tools: [...tools] } });
        } else if (frame.event === "done") {
          terminal = "done";
          reply = frame.data.reply;
          break;
        } else {
          terminal = "error";
          failure = { message: frame.data.error, ...(frame.data.code ? { code: frame.data.code } : {}) };
          break;
        }
      }
    } catch (err) {
      if (!ac.signal.aborted) {
        terminal = "error";
        failure = isApiError(err) ? { message: err.message, code: err.code } : { message: err instanceof Error ? err.message : String(err) };
        if (this.deps.onUnauthenticated(err)) failure = { message: "", code: "unauthenticated" };
      }
    }

    close();
    if (gen !== this.gen) return; // the conversation was replaced while this ran
    this.cancelTimer();
    const streamed = this.buf.text;
    this.streaming = false;
    this.abort = null;

    if (terminal === "done") {
      const finalText = reply.trim() !== "" ? reply : streamed;
      this.commit({ key: liveKey, role: "assistant", text: finalText, ...(tools.length ? { tools } : {}) });
      this.deps.announce("reply complete");
    } else if (ac.signal.aborted) {
      const keep = streamed.trim() !== "" || tools.length > 0;
      this.commit(keep ? { key: liveKey, role: "assistant", text: streamed, stopped: true, ...(tools.length ? { tools } : {}) } : null);
      this.deps.announce("reply stopped");
    } else {
      const f = failure ?? { message: "The reply ended before it was finished.", code: "stream_ended" };
      const nothingYet = streamed.trim() === "" && tools.length === 0;
      if (f.code === "unauthenticated" && nothingYet) {
        // Back to the sign-in; the message returns to the input.
        this.set({ rows: this.snap.rows.filter((r) => r.key !== userRow.key), live: null });
        this.draftText = text;
        this.set({ draftSeq: this.snap.draftSeq + 1 });
      } else if (f.code && UNAVAILABLE_CODES.has(f.code) && nothingYet) {
        // The agent cannot take it right now: hold the message and send it when the state returns.
        this.blocked = true;
        const q: QueuedMessage = { id: queueId ?? this.key("q"), text };
        this.set({
          rows: this.snap.rows.filter((r) => r.key !== userRow.key),
          live: null,
          queued: [q, ...this.snap.queued],
          notice: `${f.message.replace(/[.\s]+$/, "")}. Your message is queued and sends when the agent is back.`,
        });
      } else {
        this.commit({
          key: liveKey,
          role: "assistant",
          text: streamed,
          error: f.message || "The reply failed.",
          ...(tools.length ? { tools } : {}),
        });
      }
    }
    this.pump();
  }

  /** Replace the live bubble by a committed row (or by nothing). */
  private commit(row: ChatRow | null): void {
    this.set({ live: null, rows: row ? [...this.snap.rows, row] : this.snap.rows });
  }
}
