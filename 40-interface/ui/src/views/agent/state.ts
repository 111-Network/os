// Pure helpers for the Agent view: the state machine, history filter, conversation id, tool lines
// and the "Why?" prompt. No React and no DOM, so all of it runs under the node test environment.
import type { ChatHistory, Proposal, ServiceRecord, Status } from "../../lib/api/sse.gen";
import { buildPrompt, NEARBY_QUESTION, questionPrompt, sanitizeLine } from "../../lib/prompts";
import { NO_SETUP_SERVICES } from "../services/model";

// ---------------------------------------------------------------- agent state machine

/**
 * ready       agent live, model live
 * starting    agent or model is starting
 * no-model    agent live, model unset (the standard-tier offer applies)
 * unavailable agent unset/error, or model in error
 * offloaded   agent live and served by a hub (`intelligence.agent_state === "offloaded"`)
 * loading     no services snapshot yet (the first sends are attempted, not refused)
 */
export type AgentStateKind = "ready" | "starting" | "no-model" | "unavailable" | "offloaded" | "loading";

export type ChipDot = "live" | "unset" | "error" | "starting" | "unknown";

export interface AgentStateInfo {
  kind: AgentStateKind;
  /** Chip text next to the status dot. */
  label: string;
  dot: ChipDot;
  /** One plain sentence about why, when the state is not ready; null otherwise. */
  reason: string | null;
  /** A send can go to the agent now. When false, sends are queued locally. */
  canSend: boolean;
  /** `status.offer.state`, only meaningful for no-model. */
  offer: "offered" | "accepted" | "dismissed" | null;
  /** The model service says it can run on this device (the offer can be accepted). */
  modelCanRun: boolean;
}

const LABELS: Record<AgentStateKind, string> = {
  ready: "ready",
  starting: "starting",
  "no-model": "no model yet",
  unavailable: "unavailable",
  offloaded: "served by hub",
  loading: "connecting",
};

function info(kind: AgentStateKind, patch: Partial<AgentStateInfo> = {}): AgentStateInfo {
  const dot: ChipDot =
    kind === "ready" || kind === "offloaded" ? "live" : kind === "starting" ? "starting" : kind === "no-model" ? "unset" : kind === "unavailable" ? "error" : "unknown";
  return {
    kind,
    label: LABELS[kind],
    dot,
    reason: null,
    canSend: kind === "ready" || kind === "offloaded" || kind === "loading",
    offer: null,
    modelCanRun: true,
    ...patch,
  };
}

/**
 * Derive the chat state from the registry's `agent` and `model` records and the status.
 * The services registry is authoritative (spec 7); status only adds `offloaded` and the offer.
 */
export function agentState(services: readonly ServiceRecord[] | null, status: Status | null): AgentStateInfo {
  const offer = status?.offer?.state ?? null;
  if (!services) return info("loading", { offer });
  const agent = services.find((s) => s.id === "agent");
  const model = services.find((s) => s.id === "model");

  if (!agent) return info("unavailable", { offer, reason: "The local agent is not part of this node." });
  if (agent.state === "unset" || agent.state === "error") {
    return info("unavailable", { offer, dot: agent.state === "unset" ? "unset" : "error", reason: agent.reason || "The local agent is not available." });
  }
  if (agent.state === "starting") return info("starting", { offer, reason: agent.reason || "The agent is starting." });

  // Agent is live from here on.
  if (status?.intelligence?.agent_state === "offloaded") return info("offloaded", { offer });
  if (!model) return info("unavailable", { offer, reason: "The local model is not part of this node." });

  switch (model.state) {
    case "live":
      return info("ready", { offer });
    case "starting":
      return info("starting", { offer, reason: model.reason || "The model is starting." });
    case "unset":
      return info("no-model", { offer, modelCanRun: model.can_run_here, reason: model.reason || "No local model is installed yet." });
    case "error":
      return info("unavailable", { offer, reason: model.reason || "The local model is not available." });
  }
}

/** `reason` as a clause: no trailing punctuation, so the caller can add its own. */
export function reasonClause(reason: string | null): string {
  return reason ? reason.replace(/[.\s]+$/, "") : "";
}

/** The plain sentence shown when a message cannot be sent right now. */
export function queuedNotice(state: AgentStateInfo): string {
  switch (state.kind) {
    case "starting":
      return "The agent is starting. Your message is queued and sends when it is ready.";
    case "no-model":
      return "There is no local model yet. Your message is queued and sends once a model is running.";
    case "unavailable": {
      const why = reasonClause(state.reason);
      return `The agent is unavailable${why ? `: ${why}` : ""}. Your message is queued and sends when it is back.`;
    }
    default:
      return "The agent is not ready. Your message is queued and sends when it is.";
  }
}

// ---------------------------------------------------------------- history

export type HistoryMessage = ChatHistory["messages"][number];

export interface ChatRow {
  key: string;
  role: "user" | "assistant";
  text: string;
  /** Muted `[reading ...]` lines kept under a live reply. */
  tools?: string[];
  /** Plain error text shown under `text` in destructive styling. */
  error?: string;
  /** The user stopped this reply early. */
  stopped?: boolean;
}

/** Hide `tool` and `system` rows and anything without visible text (e.g. assistant tool-call stubs). */
export function filterHistory(messages: readonly HistoryMessage[] | null | undefined): HistoryMessage[] {
  if (!messages) return [];
  return messages.filter((m) => {
    if (m.role !== "user" && m.role !== "assistant") return false;
    return typeof m.content === "string" && m.content.trim() !== "";
  });
}

// ---------------------------------------------------------------- tool lines

const TOOL_LINES: Record<string, string> = {
  "kb.read": "reading kb",
  "health.read": "checking node health",
  "logs.read": "reading logs",
  "proposal.write": "writing a proposal",
  "task.submit": "submitting a task",
};

/** `[reading kb]` for the agent's fixed tools; unknown names are shown as given (control chars stripped). */
export function toolLine(name: string): string {
  const known = TOOL_LINES[name];
  if (known) return `[${known}]`;
  const clean = sanitizeLine(String(name)).trim().slice(0, 64);
  return clean ? `[using tool: ${clean}]` : "[using a tool]";
}

// ---------------------------------------------------------------- conversation id

export const CONVERSATION_KEY = "111.chat.conversation";
/** What `mintConversationId` produces. */
export const CONVERSATION_ID_RE = /^conv-[a-z0-9]{24}$/;
/** What the contract accepts (ConversationId: 1-128 of letters, digits, `_`, `-`). */
const CONTRACT_ID_RE = /^[A-Za-z0-9_-]{1,128}$/;
const ALPHABET = "abcdefghijklmnopqrstuvwxyz0123456789";

export type RandomFill = (bytes: Uint8Array) => Uint8Array;

const browserRandom: RandomFill = (bytes) => {
  const c = (globalThis as { crypto?: { getRandomValues?: <T extends ArrayBufferView>(a: T) => T } }).crypto;
  if (!c?.getRandomValues) throw new Error("crypto.getRandomValues is not available");
  c.getRandomValues(bytes);
  return bytes;
};

/**
 * `conv-` plus 24 characters of `[a-z0-9]` from `crypto.getRandomValues`. Bytes at or above 252 are
 * skipped (252 = 7 x 36) so every character is equally likely.
 */
export function mintConversationId(fill: RandomFill = browserRandom): string {
  let out = "";
  while (out.length < 24) {
    const bytes = fill(new Uint8Array(32));
    for (const b of bytes) {
      if (b >= 252) continue;
      out += ALPHABET[b % 36];
      if (out.length === 24) break;
    }
  }
  return `conv-${out}`;
}

export interface KeyValueStore {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
}

function localStore(): KeyValueStore | null {
  try {
    return typeof localStorage !== "undefined" ? localStorage : null;
  } catch {
    return null; // storage blocked
  }
}

export function persistConversationId(id: string, store: KeyValueStore | null = localStore()): void {
  try {
    store?.setItem(CONVERSATION_KEY, id);
  } catch {
    /* the id still works for this page load */
  }
}

/** The stored id when it satisfies the contract, else a fresh one (stored when possible). */
export function loadConversationId(store: KeyValueStore | null = localStore(), fill?: RandomFill): string {
  try {
    const stored = store?.getItem(CONVERSATION_KEY);
    if (stored && CONTRACT_ID_RE.test(stored)) return stored;
  } catch {
    /* unreadable: mint */
  }
  const id = mintConversationId(fill);
  persistConversationId(id, store);
  return id;
}

// ---------------------------------------------------------------- starters

/** A prepared message offered on an empty conversation. Never sent by itself. */
export interface Starter {
  label: string;
  text: string;
}

/** At most this many service starters, so the empty state stays a nudge. */
export const MAX_SERVICE_STARTERS = 2;

/**
 * The empty conversation's starting points. Nothing here is per-service: every service starter is the
 * one prompt builder over a registry record. A service that is in error comes first (fix), then ones that
 * can be set up on this device; the agent itself is left out (it cannot fix itself from a chat to itself).
 */
export function starters(services: readonly ServiceRecord[] | null, status: Status | null): Starter[] {
  const forServices: Starter[] = [];
  const list = (services ?? []).filter((s) => s.id !== "agent" && s.can_run_here !== false);
  for (const s of list.filter((x) => x.state === "error")) {
    forServices.push({ label: `fix the ${s.plain_name}`, text: buildPrompt("fix", { kind: "service", service: s }, { status }) });
  }
  for (const s of list.filter((x) => x.state === "unset" && !NO_SETUP_SERVICES.has(x.id))) {
    forServices.push({ label: `set up the ${s.plain_name}`, text: buildPrompt("setup", { kind: "service", service: s }, { status }) });
  }
  return [
    { label: "check this node", text: buildPrompt("check", { kind: "node" }, { status }) },
    ...forServices.slice(0, MAX_SERVICE_STARTERS),
    { label: "who is nearby?", text: questionPrompt(NEARBY_QUESTION, { kind: "node" }, { status }) },
  ];
}

// ---------------------------------------------------------------- proposals

/** The prompt staged by a proposal card's "Why?" button. */
export function whyPrompt(proposal: Pick<Proposal, "id" | "title">, status: Status | null): string {
  const title = sanitizeLine(proposal.title).replace(/"/g, "'").slice(0, 120);
  const head = `Explain proposal ${proposal.id} ("${title}") in plain words: what it changes, the risk, and how to undo it.`;
  return questionPrompt(head, { kind: "node" }, { status });
}

/** Maximum message length the contract accepts. */
export const MAX_MESSAGE_CHARS = 16000;
