// The real adapter: fetch for request/response, fetch + parseSseStream for the chat stream,
// EventSource (events-client.ts) for the live stream. All environment access is injected.
import type {
  Adapter,
  ChatInput,
  DevicePatch,
  EventsHandlers,
  FlashReply,
  ForeignThreadQuery,
  FlashRequest,
  MessageAccepted,
  ProposalFilter,
  RestartReply,
  SendOptions,
  ServiceRestartReply,
  TaskAccepted,
  ThreadQuery,
  UpdateCheckReply,
} from "./adapter";
import { connectEvents, DEFAULT_STALE_AFTER_MS, type EventSourceCtor } from "./events-client";
import { ApiError, errorFromResponse, synthProblem } from "./problem";
import type { ThreadPage } from "./messages";
import { parseSseStream } from "./sse";
import {
  CHAT_EVENT_NAMES,
  type AgentStatus,
  type AuthSession,
  type BridgeComponent,
  type BridgeEdge,
  type BridgeEdgeCreate,
  type BridgeEdgePatch,
  type BridgeSettings,
  type BridgeSettingsPatch,
  type Card,
  type ChatFrame,
  type ChatHistory,
  type ChatReply,
  type Contact,
  type ContactCreate,
  type ContactPatch,
  type Decision,
  type DeviceRecord,
  type FabricHub,
  type FabricSettings,
  type FabricSettingsPatch,
  type FabricStatus,
  type FanoutPreview,
  type FanoutRequest,
  type FanoutResult,
  type ForeignConversation,
  type ForeignSendAccepted,
  type ForeignSendRequest,
  type ForeignThread,
  type HostedPanel,
  type HubEnrollRequest,
  type HubOffer,
  type IdentifyRequest,
  type LogTail,
  type Message,
  type NearbySource,
  type Network,
  type PanelSession,
  type Proposal,
  type SendMessageRequest,
  type ServiceId,
  type ServiceRecord,
  type Settings,
  type SettingsPatch,
  type Status,
  type TaskRequest,
  type Thread,
  type TransportJoin,
  type TransportPosturePatch,
  type TransportPostureResult,
  type TransportStatus,
} from "./types";

export interface HttpDeps {
  fetch?: typeof fetch;
  EventSource?: EventSourceCtor;
  now?: () => number;
  setTimer?: (fn: () => void, ms: number) => unknown;
  clearTimer?: (handle: unknown) => void;
  /** Origin prefix; default "" (same-origin, which is the only production mode). */
  base?: string;
  staleAfterMs?: number;
}

type Query = Record<string, string | number | undefined>;

// What the node said about each hosted panel the first time this page asked (`GET /panels` at boot): true = enabled and
// listening. The dashboard's Content-Security-Policy is computed when the document is fetched and names only the panels that
// were enabled and listening then (os/5-messaging/README.md section 13), so the first answer is the page's best evidence for
// whether a panel's origin is in the `frame-src` this page is stuck with. Later answers never replace it.
let panelsAtLoad: ReadonlyMap<string, boolean> | null = null;

/** Remember the first panel list of this page load; later calls change nothing. */
export function recordPanelsAtLoad(panels: ReadonlyArray<Pick<HostedPanel, "id" | "state">>): void {
  panelsAtLoad ??= new Map(panels.map((p) => [p.id, p.state?.enabled === true && p.state?.listening === true] as const));
}

/** The first panel list of this page load (panel id -> enabled and listening), or null before one arrived. */
export const panelsSeenAtLoad = (): ReadonlyMap<string, boolean> | null => panelsAtLoad;

/** Test seam: forget the remembered first list. */
export function resetPanelsAtLoadForTests(): void {
  panelsAtLoad = null;
}

interface CallOptions {
  query?: Query;
  body?: unknown;
  as?: "json" | "text" | "none";
  signal?: AbortSignal;
}

function isAbort(e: unknown): boolean {
  return typeof e === "object" && e !== null && (e as { name?: string }).name === "AbortError";
}

function qs(query?: Query): string {
  if (!query) return "";
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(query)) if (v !== undefined) p.set(k, String(v));
  const s = p.toString();
  return s ? `?${s}` : "";
}

export function createHttpAdapter(deps: HttpDeps = {}): Adapter {
  const api = `${(deps.base ?? "").replace(/\/+$/, "")}/api/v1`;
  const now = deps.now ?? (() => Date.now());
  const setTimer = deps.setTimer ?? ((fn: () => void, ms: number) => setTimeout(fn, ms));
  const clearTimer = deps.clearTimer ?? ((h: unknown) => clearTimeout(h as ReturnType<typeof setTimeout>));
  // Resolved per call so tests (and late polyfills) can swap the global.
  const doFetch = (input: string, init: RequestInit): Promise<Response> => (deps.fetch ?? globalThis.fetch)(input, init);

  async function send(method: string, path: string, o: CallOptions, accept: string): Promise<Response> {
    const headers: Record<string, string> = { Accept: accept };
    const init: RequestInit = { method, headers, credentials: "same-origin", cache: "no-store" };
    if (o.signal) init.signal = o.signal;
    if (method !== "GET") {
      // The daemon's CSRF rule: every mutating route requires this header, body or not.
      headers["Content-Type"] = "application/json";
      if (o.body !== undefined) init.body = JSON.stringify(o.body);
    }
    let res: Response;
    try {
      res = await doFetch(`${api}${path}${qs(o.query)}`, init);
    } catch (e) {
      if (isAbort(e)) throw e;
      throw ApiError.network(e);
    }
    if (!res.ok) throw await errorFromResponse(res);
    return res;
  }

  async function call<T>(method: string, path: string, o: CallOptions = {}): Promise<T> {
    const as = o.as ?? "json";
    const res = await send(method, path, o, as === "text" ? "text/plain, text/markdown, */*" : "application/json");
    if (as === "none") return undefined as T;
    if (as === "text") return (await res.text()) as T;
    try {
      return (await res.json()) as T;
    } catch (e) {
      throw new ApiError(synthProblem(res.status, "Bad response", "The node answered with something that is not JSON."), { cause: e, code: "upstream_error" });
    }
  }

  const enc = encodeURIComponent;

  /** Unwrap `{schema, <key>: [...]}`; a reply without the list is a bad response, never an empty one. */
  async function list<T>(path: string, key: string, what: string): Promise<T[]> {
    const body = await call<Record<string, unknown>>("GET", path);
    const items = body?.[key];
    if (!Array.isArray(items)) throw new ApiError(synthProblem(200, "Bad response", `Missing ${what} list.`), { code: "upstream_error" });
    return items as T[];
  }

  async function* chatSend(req: ChatInput, opts?: SendOptions): AsyncGenerator<ChatFrame, void, void> {
    if (req.stream === false) {
      const reply = await call<ChatReply>("POST", "/chat", { body: req, ...(opts?.signal ? { signal: opts.signal } : {}) });
      yield { event: "done", data: { conversation_id: reply.conversation_id, reply: reply.reply, tool_trace: reply.tool_trace } };
      return;
    }
    const controller = new AbortController();
    const onAbort = () => controller.abort();
    if (opts?.signal) {
      if (opts.signal.aborted) controller.abort();
      else opts.signal.addEventListener("abort", onAbort, { once: true });
    }
    try {
      const res = await send("POST", "/chat", { body: { ...req, stream: true }, signal: controller.signal }, "text/event-stream");
      const type = res.headers.get("content-type") ?? "";
      if (!type.includes("text/event-stream")) {
        // Tolerate a JSON ChatReply even though a stream was requested.
        const reply = (await res.json()) as ChatReply;
        yield { event: "done", data: { conversation_id: reply.conversation_id, reply: reply.reply, tool_trace: reply.tool_trace } };
        return;
      }
      if (!res.body) throw new ApiError(synthProblem(res.status, "Bad response", "The node sent no stream body."), { code: "upstream_error" });
      let terminal = false;
      try {
        for await (const msg of parseSseStream(res.body.getReader())) {
          if (!msg.event || !(CHAT_EVENT_NAMES as readonly string[]).includes(msg.event)) continue; // forward-compatible
          let data: unknown;
          try {
            data = JSON.parse(msg.data);
          } catch {
            yield { event: "error", data: { error: "The node sent a frame that could not be read.", code: "bad_frame" } };
            return;
          }
          const frame = { event: msg.event, data } as ChatFrame;
          yield frame;
          if (frame.event === "done" || frame.event === "error") {
            terminal = true;
            return;
          }
        }
      } catch (e) {
        if (isAbort(e) || e instanceof ApiError) throw e;
        throw ApiError.network(e);
      }
      if (!terminal) yield { event: "error", data: { error: "The connection closed before the reply finished.", code: "stream_ended" } };
    } finally {
      opts?.signal?.removeEventListener("abort", onAbort);
      controller.abort(); // no-op after completion; closes the connection if the consumer stopped early
    }
  }

  return {
    kind: "http",

    status: () => call<Status>("GET", "/status"),
    services: async () => {
      const body = await call<{ services?: ServiceRecord[] }>("GET", "/services");
      if (!Array.isArray(body.services)) throw new ApiError(synthProblem(200, "Bad response", "Missing services list."), { code: "upstream_error" });
      return body.services;
    },
    service: (id: ServiceId) => call<ServiceRecord>("GET", `/services/${enc(id)}`),
    serviceLog: (id: ServiceId, n?: number) => call<LogTail>("GET", `/services/${enc(id)}/log`, { query: { n } }),
    restartService: (id: ServiceId) => call<ServiceRestartReply>("POST", `/services/${enc(id)}/restart`),

    chat: {
      send: chatSend,
      history: (conversationId: string) => call<ChatHistory>("GET", "/chat/history", { query: { conversation_id: conversationId } }),
    },

    devices: async () => {
      const body = await call<{ devices?: DeviceRecord[] }>("GET", "/devices");
      if (!Array.isArray(body.devices)) throw new ApiError(synthProblem(200, "Bad response", "Missing devices list."), { code: "upstream_error" });
      return body.devices;
    },
    device: (id: string) => call<DeviceRecord>("GET", `/devices/${enc(id)}`),
    patchDevice: (id: string, patch: DevicePatch) => call<DeviceRecord>("PATCH", `/devices/${enc(id)}`, { body: patch }),
    forgetDevice: (id: string) => call<void>("DELETE", `/devices/${enc(id)}`, { as: "none" }),
    flashDevice: (id: string, body: FlashRequest) => call<FlashReply>("POST", `/devices/${enc(id)}/flash`, { body }),

    identifyDevice: (id: string, body: IdentifyRequest) => call<DeviceRecord>("POST", `/devices/${enc(id)}/identify`, { body }),

    agentStatus: () => call<AgentStatus>("GET", "/agent/status"),
    agentTask: (req: TaskRequest) => call<TaskAccepted>("POST", "/agent/task", { body: req }),

    proposals: async (status?: ProposalFilter) => {
      const body = await call<{ proposals?: Proposal[] }>("GET", "/proposals", { query: { status } });
      if (!Array.isArray(body.proposals)) throw new ApiError(synthProblem(200, "Bad response", "Missing proposals list."), { code: "upstream_error" });
      return body.proposals;
    },
    proposal: (id: string) => call<Proposal>("GET", `/proposals/${enc(id)}`),
    decide: (id: string, decision: Decision, acceptWarning?: string) =>
      call<Proposal>("POST", `/proposals/${enc(id)}/decision`, { body: acceptWarning === undefined ? { decision } : { decision, accept_warning: acceptWarning } }),

    settings: () => call<Settings>("GET", "/settings"),
    patchSettings: (patch: SettingsPatch) => call<Settings>("PATCH", "/settings", { body: patch }),

    restart: () => call<RestartReply>("POST", "/node/restart"),
    updateCheck: () => call<UpdateCheckReply>("POST", "/update/check"),

    messages: {
      threads: () => list<Thread>("/messages/threads", "threads", "threads"),
      thread: async (peer: string, q: ThreadQuery = {}) => {
        const body = await call<Partial<ThreadPage>>("GET", `/messages/threads/${enc(peer)}`, { query: { since_seq: q.sinceSeq, limit: q.limit } });
        if (!Array.isArray(body?.messages)) throw new ApiError(synthProblem(200, "Bad response", "Missing messages list."), { code: "upstream_error" });
        return { peer: body.peer ?? peer, request: body.request === true, messages: body.messages, more: body.more === true } satisfies ThreadPage;
      },
      send: (req: SendMessageRequest) => call<MessageAccepted>("POST", "/messages", { body: req }),
      retry: (id: string) => call<Message>("POST", `/messages/${enc(id)}/retry`),
      // The contract's `cancelMessage` (POST /messages/{id}/cancel, 202). Written against the route, not against a generated
      // type, so it compiles whether or not types.gen.ts carries the operation; the reply (the message after cancelling) is not
      // read: the new state is a `message` frame, and the view re-reads the thread.
      cancel: (id: string) => call<void>("POST", `/messages/${enc(id)}/cancel`, { as: "none" }),
    },

    contacts: {
      list: () => list<Contact>("/contacts", "contacts", "contacts"),
      add: (body: ContactCreate) => call<Contact>("POST", "/contacts", { body }),
      ownCard: () => call<Card>("GET", "/contacts/me/card"),
      get: (id: string) => call<Contact>("GET", `/contacts/${enc(id)}`),
      patch: (id: string, patch: ContactPatch) => call<Contact>("PATCH", `/contacts/${enc(id)}`, { body: patch }),
      forget: (id: string) => call<void>("DELETE", `/contacts/${enc(id)}`, { as: "none" }),
    },

    fabric: {
      status: () => call<FabricStatus>("GET", "/fabric"),
      enrollHub: (body: HubEnrollRequest) => call<FabricHub>("POST", "/fabric/hub/enroll", { body }),
      offerHub: () => call<HubOffer>("POST", "/fabric/hub/offer"),
      dismissHubOffer: () => call<void>("DELETE", "/fabric/hub/offer", { as: "none" }),
      patchSettings: (patch: FabricSettingsPatch) => call<FabricSettings>("PATCH", "/fabric/settings", { body: patch }),
    },

    nearby: () => list<NearbySource>("/nearby", "sources", "sources"),

    networks: {
      list: () => list<Network>("/networks", "networks", "networks"),
      conversations: (net: string) => list<ForeignConversation>(`/networks/${enc(net)}/conversations`, "conversations", "conversations"),
      openDm: (net: string, nativeId: string) => call<ForeignConversation>("POST", `/networks/${enc(net)}/conversations`, { body: { kind: "dm", native_id: nativeId } }),
      conversation: async (net: string, conv: string, q: ForeignThreadQuery = {}) => {
        const body = await call<Partial<ForeignThread>>("GET", `/networks/${enc(net)}/conversations/${enc(conv)}`, { query: { since_seq: q.sinceSeq, limit: q.limit } });
        if (!Array.isArray(body?.messages) || !body.conversation) throw new ApiError(synthProblem(200, "Bad response", "Missing messages list."), { code: "upstream_error" });
        return { schema: body.schema ?? 1, conversation: body.conversation, messages: body.messages, more: body.more === true } satisfies ForeignThread;
      },
      send: (net: string, conv: string, body: ForeignSendRequest) => call<ForeignSendAccepted>("POST", `/networks/${enc(net)}/conversations/${enc(conv)}/messages`, { body }),
      fanoutPreview: (body: FanoutRequest) => call<FanoutPreview>("POST", "/messages/fanout/preview", { body }),
      fanout: (body: FanoutRequest) => call<FanoutResult>("POST", "/messages/fanout", { body }),
    },

    bridge: {
      edges: () => list<BridgeEdge>("/bridge/edges", "edges", "edges"),
      createEdge: (body: BridgeEdgeCreate) => call<BridgeEdge>("POST", "/bridge/edges", { body }),
      edge: (id: string) => call<BridgeEdge>("GET", `/bridge/edges/${enc(id)}`),
      patchEdge: (id: string, patch: BridgeEdgePatch) => call<BridgeEdge>("PATCH", `/bridge/edges/${enc(id)}`, { body: patch }),
      deleteEdge: (id: string) => call<void>("DELETE", `/bridge/edges/${enc(id)}`, { as: "none" }),
      settings: () => call<BridgeSettings>("GET", "/bridge/settings"),
      patchSettings: (patch: BridgeSettingsPatch) => call<BridgeSettings>("PATCH", "/bridge/settings", { body: patch }),
      components: () => list<BridgeComponent>("/bridge/components", "components", "components"),
      enableComponent: (id: string, acceptNoticeHash?: string) =>
        call<BridgeComponent>("POST", `/bridge/components/${enc(id)}/enable`, { body: acceptNoticeHash === undefined ? {} : { accept_notice_hash: acceptNoticeHash } }),
      disableComponent: (id: string) => call<BridgeComponent>("POST", `/bridge/components/${enc(id)}/disable`),
      removeComponentFiles: (id: string, cache?: boolean) =>
        call<BridgeComponent>("POST", `/bridge/components/${enc(id)}/remove-files`, { body: cache === undefined ? {} : { cache } }),
    },

    transport: {
      status: () => call<TransportStatus>("GET", "/transport"),
      patchPosture: (patch: TransportPosturePatch) => call<TransportPostureResult>("PATCH", "/transport/posture", { body: patch }),
      join: () => call<TransportJoin>("POST", "/transport/join"),
    },

    panels: {
      list: async () => {
        const panels = await list<HostedPanel>("/panels", "panels", "panels");
        recordPanelsAtLoad(panels);
        return panels;
      },
      openSession: (id: ServiceId) => call<PanelSession>("POST", `/panels/${enc(id)}/session`),
    },

    kb: {
      index: () => call<string>("GET", "/kb", { as: "text" }),
      entry: (path: string) => call<string>("GET", "/kb/entry", { query: { path }, as: "text" }),
    },

    auth: {
      session: () => call<AuthSession>("GET", "/auth/session"),
      pair: (code: string) => call<AuthSession>("POST", "/auth/pair", { body: { code } }),
      logout: () => call<void>("POST", "/auth/logout", { as: "none" }),
    },

    events: (handlers: EventsHandlers) =>
      connectEvents(
        {
          EventSource: deps.EventSource ?? (globalThis as { EventSource?: EventSourceCtor }).EventSource,
          url: `${api}/events`,
          now,
          setTimer,
          clearTimer,
          staleAfterMs: deps.staleAfterMs ?? DEFAULT_STALE_AFTER_MS,
        },
        handlers,
      ),
  };
}
