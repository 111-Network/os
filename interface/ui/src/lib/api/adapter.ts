// The one typed seam between the UI and the node (spec §9). Two implementations:
// http.ts (real daemon) and mock/ (dev only, loaded by dynamic import). Components never talk to
// an adapter directly for reads: they use the store hooks. Actions and chat go through getAdapter().
//
// Every method rejects with ApiError (problem.ts) on problem+json, on transport failure
// (code "network") and on 401 (code "unauthenticated").
import type {
  AgentStatus,
  AuthSession,
  ChatFrame,
  ChatHistory,
  ChatRequest,
  Decision,
  DeviceRecord,
  EventsFrame,
  LogTail,
  Proposal,
  ServiceId,
  ServiceRecord,
  Settings,
  SettingsPatch,
  Status,
  TaskRequest,
} from "./types";

/**
 * connecting: no open connection yet, or the browser is retrying after a drop.
 * open:       connected and a frame arrived within the staleness window.
 * stale:      connected, but silent for longer than the staleness window.
 * closed:     the connection failed fatally (HTTP error such as 401, or no EventSource); the
 *             adapter retries with backoff and reports `connecting` again when it does.
 */
export type ConnectionState = "connecting" | "open" | "stale" | "closed";

export interface ConnectionInfo {
  /** Epoch ms of the last frame (or of the connection opening, before any frame). */
  lastEventAt?: number;
  /** `id:` of the last frame seen on this adapter's stream. */
  lastEventId?: string;
}

/** A frame as delivered to the store; `id` is the SSE `id:` (global monotonic sequence). */
export type EventsFrameWithId = EventsFrame & { id?: string };

export interface EventsHandlers {
  onFrame(frame: EventsFrameWithId): void;
  onState(state: ConnectionState, info?: ConnectionInfo): void;
}

export type ProposalFilter = "open" | "decided" | "all";

export interface TaskAccepted {
  schema: number;
  task_id: string;
}
export interface RestartReply {
  schema: number;
  restarting: boolean;
}
/** `202` from POST /services/{id}/restart. */
export interface ServiceRestartReply {
  schema: number;
  id: string;
  restarting: boolean;
}
export interface UpdateCheckReply {
  schema: number;
  started: boolean;
}

/**
 * What callers pass to chat.send. Same as the contract's ChatRequest but with `stream` optional:
 * openapi-typescript marks properties that have a schema default as required.
 */
export type ChatInput = Omit<ChatRequest, "stream"> & { stream?: boolean };

/** Body of PATCH /devices/{id}. `alias: null` (or an empty string) clears the name. */
export interface DevicePatch {
  alias?: string | null;
}

export interface SendOptions {
  /** Abort the in-flight request. Breaking out of the `for await` loop aborts it too. */
  signal?: AbortSignal;
}

export interface Adapter {
  /** Which implementation this is; the mock also exposes its controls on `adapter.mock`. */
  readonly kind: "http" | "mock";

  status(): Promise<Status>;
  /** Unwraps `{schema, services}`. */
  services(): Promise<ServiceRecord[]>;
  service(id: ServiceId): Promise<ServiceRecord>;
  serviceLog(id: ServiceId, n?: number): Promise<LogTail>;
  /**
   * Restart one child the daemon supervises (`actions` contains `restart`). Resolves when the daemon
   * accepted the request; the new state arrives as `services` frames. 409 `not_supervised` otherwise.
   */
  restartService(id: ServiceId): Promise<ServiceRestartReply>;

  chat: {
    /**
     * Streams `token` / `tool` frames and ends with exactly one terminal frame, `done` or `error`.
     * Pre-stream failures (400, 401, 503 agent/model down) reject on the first iteration with
     * ApiError. `stream:false` yields a single synthesized `done`. If the connection ends cleanly
     * without a terminal frame, a synthesized `error` frame (code `stream_ended`) is yielded.
     */
    send(req: ChatInput, opts?: SendOptions): AsyncIterable<ChatFrame>;
    history(conversationId: string): Promise<ChatHistory>;
  };

  /** Unwraps `{schema, devices}`: hardware attached to, and devices paired with, this node. */
  devices(): Promise<DeviceRecord[]>;
  device(id: string): Promise<DeviceRecord>;
  patchDevice(id: string, patch: DevicePatch): Promise<DeviceRecord>;
  /** Forget: drops the alias and pairing record. Detected hardware reappears when plugged in again. */
  forgetDevice(id: string): Promise<void>;

  agentStatus(): Promise<AgentStatus>;
  agentTask(req: TaskRequest): Promise<TaskAccepted>;

  /** Unwraps `{schema, proposals}`. Default filter is the server default, `open`. */
  proposals(status?: ProposalFilter): Promise<Proposal[]>;
  proposal(id: string): Promise<Proposal>;
  decide(id: string, decision: Decision): Promise<Proposal>;

  settings(): Promise<Settings>;
  patchSettings(patch: SettingsPatch): Promise<Settings>;

  restart(): Promise<RestartReply>;
  updateCheck(): Promise<UpdateCheckReply>;

  kb: {
    /** Root llms.txt as text. */
    index(): Promise<string>;
    /** One markdown entry by relative path, e.g. `projects/meshtastic.md`. */
    entry(path: string): Promise<string>;
  };

  auth: {
    /** Exempt from 401: reports `paired:false` instead. */
    session(): Promise<AuthSession>;
    pair(code: string): Promise<AuthSession>;
    logout(): Promise<void>;
  };

  /**
   * Subscribe to the live stream. Returns an unsubscribe function; after it runs no handler is
   * called again. `onState('connecting')` is reported synchronously-ish (before any frame).
   */
  events(handlers: EventsHandlers): () => void;
}
