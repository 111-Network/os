// The one typed seam between the UI and the node (spec §9). Two implementations:
// http.ts (real daemon) and mock/ (dev only, loaded by dynamic import). Components never talk to
// an adapter directly for reads: they use the store hooks. Actions and chat go through getAdapter().
//
// Every method rejects with ApiError (problem.ts) on problem+json, on transport failure
// (code "network") and on 401 (code "unauthenticated").
import type {
  AgentStatus,
  AuthSession,
  Card,
  ChatFrame,
  ChatHistory,
  ChatRequest,
  Contact,
  ContactCreate,
  ContactPatch,
  Decision,
  DeviceRecord,
  EventsFrame,
  FabricHub,
  FabricSettings,
  FabricSettingsPatch,
  FabricStatus,
  HostedPanel,
  HubEnrollRequest,
  HubOffer,
  IdentifyRequest,
  LogTail,
  Message,
  MessageId,
  NearbySource,
  NodeId,
  PanelSession,
  Proposal,
  SendMessageRequest,
  ServiceId,
  ServiceRecord,
  Settings,
  SettingsPatch,
  Status,
  TaskRequest,
  Thread,
} from "./types";
import type { ThreadPage } from "./messages";

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

/** Body of POST /devices/{id}/flash: the offered image's sha256 and the device's name, typed by the user. */
export interface FlashRequest {
  sha256: string;
  confirm: string;
}

/** `202` from POST /devices/{id}/flash. The new firmware shows up as a `devices` frame, not in this reply. */
export interface FlashReply {
  schema: number;
  id: string;
  flashing: boolean;
}

/** Query of `GET /messages/threads/{peer}`: `limit` defaults to 100 (max 500); `sinceSeq` pages forward from a sequence. */
export interface ThreadQuery {
  sinceSeq?: number;
  limit?: number;
}

/** `202` from POST /messages: queued on this node. Delivery is never promised by this call (§9). */
export interface MessageAccepted {
  schema: number;
  id: MessageId;
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
  /**
   * Flash the board's `flash_offer` image (D17: human-confirmed only). Layer 6 core: until it ships the
   * daemon answers 404 `not_found`; 409 `offer_stale` when the sha256 no longer matches or a flash already runs.
   */
  flashDevice(id: string, body: FlashRequest): Promise<FlashReply>;

  /**
   * The consented probe of a board (Layer 5 §14; `actions` contains `identify`). Opening a serial port can reset the
   * board, so `consent: true` is required and typed as the literal. 409 `port_busy` when another owner holds the port;
   * 502 `upstream_error` when meshmud or the presence reader did not answer.
   */
  identifyDevice(id: string, body: IdentifyRequest): Promise<DeviceRecord>;

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

  /** Messages (Layer 5 §15, os/50-messaging/README.md §9). Order is `local_seq`; delivery evidence is `Message.state`. */
  messages: {
    /** Unwraps `{schema, threads}`: most recent activity first, including message-request threads. */
    threads(): Promise<Thread[]>;
    /** One conversation. Without `sinceSeq` the newest `limit` messages; `more` says older ones exist. */
    thread(peer: NodeId, q?: ThreadQuery): Promise<ThreadPage>;
    /** Queue a text to a contact. Always 202 `queued`; 404 when `to` is not a contact; 400 when the text cannot be carried. */
    send(req: SendMessageRequest): Promise<MessageAccepted>;
    /** Re-queue an own message that is `queued` or `sent`; 409 `bad_request` otherwise. */
    retry(id: MessageId): Promise<Message>;
    /**
     * Cancel an own message that is still `queued` (`POST /messages/{id}/cancel`, spec 4: before it leaves, an empty
     * tombstone goes instead, so no message number is abandoned). Resolves when the node accepted (202); the message stays in
     * its thread as `queued` with `reason_code: cancelled`, and that arrives as a `message` frame (the contract's reply is the
     * message after cancelling; the page does not read it and re-reads the thread). 409 `bad_request` once it has left or when it
     * is not an own message. Optional on the type until every adapter carries the contract's `cancelMessage`: the thread view
     * offers the control only where it is present.
     */
    cancel?(id: MessageId): Promise<void>;
  };

  /** Contacts and message requests (§7, §15). */
  contacts: {
    /** Unwraps `{schema, contacts}`; pending message requests have `request: true`. */
    list(): Promise<Contact[]>;
    /** By `card` text or `node_id` alone; an already-known contact is returned unchanged; a key conflict is 409 `bad_request`. */
    add(body: ContactCreate): Promise<Contact>;
    /** This node's own card: base32 text and a QR module matrix the UI draws. */
    ownCard(): Promise<Card>;
    get(id: NodeId): Promise<Contact>;
    /** Alias, accept a pending request (`accept: true`), pin, mark the thread read (`read_seq`). */
    patch(id: NodeId, patch: ContactPatch): Promise<Contact>;
    /** Forget (and, for a pending request, discard it). 204. */
    forget(id: NodeId): Promise<void>;
  };

  /** The fabric: links, home hub, custody, store, settings (§8–§10, §15). */
  fabric: {
    status(): Promise<FabricStatus>;
    /** Member side: enroll with a hub (`ENROLL`, §7.3). 409 already enrolled; 429 lockout; 502 hub unreachable or refused. */
    enrollHub(body: HubEnrollRequest): Promise<FabricHub>;
    /** Hub side, loopback-only: mint a one-time code with the hub's card and address as one bundle. */
    offerHub(): Promise<HubOffer>;
    /** Hub side, loopback-only: dismiss the outstanding code. Idempotent 204. */
    dismissHubOffer(): Promise<void>;
    /** Loopback-only partial merge of hub role, LAN listener, custody opt-in and per-class quotas. */
    patchSettings(patch: FabricSettingsPatch): Promise<FabricSettings>;
  };

  /** Unwraps `{schema, sources}`: presence per source (v1: Meshtastic nodes heard). Names are untrusted RF-origin strings. */
  nearby(): Promise<NearbySource[]>;

  /** Hosted panels (§13). */
  panels: {
    /** Unwraps `{schema, panels}`: the declared panels and their listener slots. */
    list(): Promise<HostedPanel[]>;
    /** Mint a one-time ticket URL for a panel's own origin (single use, 30 s). 404 when unknown or its service is not set up. */
    openSession(id: ServiceId): Promise<PanelSession>;
  };

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
