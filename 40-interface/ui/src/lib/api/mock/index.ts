// Dev-only mock adapter. Same contract as http.ts, no daemon needed:
//   createMockAdapter({ scenario })  -> Adapter & { mock: MockControls }
// Reached ONLY through the DEV-guarded dynamic import in ../index.ts; never import it statically.
import type { Adapter, ChatInput, DevicePatch, EventsHandlers, FlashReply, FlashRequest, MessageAccepted, ProposalFilter, RestartReply, SendOptions, ServiceRestartReply, TaskAccepted, ThreadQuery, UpdateCheckReply } from "../adapter";
import { CARD_PROBLEM_TEXT, NODE_ID_PROBLEM_TEXT, parseCardText, parseNodeId } from "../contacts";
import { ApiError } from "../problem";
import type {
  AuthSession,
  Card,
  ChatFrame,
  ChatHistory,
  Contact,
  ContactCreate,
  ContactPatch,
  Decision,
  DeviceRecord,
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
  NearbySource,
  NodeId,
  PanelSession,
  Problem,
  ProblemCode,
  Proposal,
  SendMessageRequest,
  ServiceId,
  ServiceRecord,
  Settings,
  SettingsPatch,
  Status,
  TaskRequest,
  Thread,
} from "../types";
import { SERVICE_IDS } from "../types";
import { MockBus, realClock, type MockClock } from "./bus";
import { finalSentence, pickReply, tokenize } from "./chat";
import { buildFixtures, iso, layersRunning, logLine } from "./fixtures";
import { HUB, MOCK_HUB_ADDRESS, MOCK_REASONS, buildNearby, buildPanels, cancelQueued, fabricRow, hex64, hubBundleText, makeContact, meshtasticLiveRow, pageOf, threadsOf } from "./messaging";
import { cardFor } from "./qr";
import { MOCK_PAIR_CODE, SCENARIO_NAMES, resolveScenario, scenarios, type MockData, type Scenario, type ScenarioName, type ScriptContext } from "./scenarios";

export { SCENARIO_NAMES, resolveScenario, MOCK_PAIR_CODE };
export type { ScenarioName };
export * from "./bus";

export interface MockOptions {
  scenario?: ScenarioName;
  clock?: MockClock;
  /** Delay between streamed chat tokens. Default 40. */
  tokenDelayMs?: number;
  /** Simulated REST latency. Default 60 with the real clock, 0 with an injected clock. */
  latencyMs?: number;
  /** Status heartbeat period. Default 30 000. */
  heartbeatMs?: number;
  /** Start the heartbeat and the scenario script. Default true. */
  autoStart?: boolean;
}

export interface MockControls {
  readonly scenario: Scenario;
  /** Live state; mutate it and call the matching `bus.publish` to bend a scenario by hand. */
  readonly state: MockData;
  readonly bus: MockBus;
  /** Drop the event stream for `ms`, then reconnect with Last-Event-ID replay (or reset). */
  dropConnection(ms: number): void;
  dispose(): void;
}

export type MockAdapter = Adapter & { readonly mock: MockControls };

const CHAT_ID = /^[A-Za-z0-9_-]{1,128}$/;
const KB_UNSAFE = /(^|[\\/])\.\.([\\/]|$)|^[\\/]|\0/;

function problem(status: number, code: ProblemCode, title: string, detail?: string, errors?: Problem["errors"]): ApiError {
  const p: Problem = { type: "about:blank", title, status, code };
  if (detail) p.detail = detail;
  if (errors) p.errors = errors;
  return new ApiError(p);
}

export function createMockAdapter(opts: MockOptions = {}): MockAdapter {
  const clock = opts.clock ?? realClock;
  const name = opts.scenario ?? "healthy";
  const makeScenario = scenarios[name];
  if (!makeScenario) throw new Error(`mock: unknown scenario "${name}" (have: ${SCENARIO_NAMES.join(", ")})`);
  const t0 = clock.now();
  const scenario = makeScenario(buildFixtures(t0));
  const state = scenario.data;
  const tokenDelay = opts.tokenDelayMs ?? 40;
  const latency = opts.latencyMs ?? (opts.clock ? 0 : 60);

  let uptimeOrigin = t0 - state.status.node.uptime_sec * 1000;
  let downUntil = 0;
  let agentSeq = 0;
  let pairFailures = 0;
  let pairBlockedUntil = 0;
  let disposed = false;
  const histories = new Map<string, NonNullable<ChatHistory["messages"]>>();
  const timers = new Set<unknown>();

  const later = (ms: number, fn: () => void): void => {
    const h = clock.setTimer(() => {
      timers.delete(h);
      if (!disposed) fn();
    }, ms);
    timers.add(h);
  };
  const sleep = (ms: number): Promise<void> => (ms <= 0 ? Promise.resolve() : new Promise((res) => later(ms, res)));

  const statusNow = (): Status => {
    const s = structuredClone(state.status);
    s.node.uptime_sec = Math.max(0, Math.floor((clock.now() - uptimeOrigin) / 1000));
    return s;
  };
  const publishStatus = () => bus.publish({ event: "status", data: statusNow() });
  const publishServices = () => bus.publish({ event: "services", data: { services: structuredClone(state.services) } });
  const svc = (id: ServiceId): ServiceRecord => {
    const s = state.services.find((x) => x.id === id);
    if (!s) throw problem(404, "not_found", "No such service", id);
    return s;
  };

  const bus = new MockBus({
    clock,
    snapshot: statusNow,
    ...(opts.heartbeatMs !== undefined ? { heartbeatMs: opts.heartbeatMs } : {}),
    gate: () => !state.auth.requirePair || state.auth.paired,
  });

  // --- device registry -----------------------------------------------------------------------
  const baseOrder = buildFixtures(t0).devices.map((d) => d.id);
  const rank = (id: string): number => {
    const i = baseOrder.indexOf(id);
    return i < 0 ? Number.MAX_SAFE_INTEGER : i;
  };
  /** Copies of every device ever shown, to bring forgotten hardware back as it was. */
  const originals = new Map<string, DeviceRecord>(state.devices.map((d) => [d.id, structuredClone(d)]));
  const publishDevices = () => bus.publish({ event: "devices", data: { devices: structuredClone(state.devices) } });
  const device = (id: string): DeviceRecord => {
    const d = state.devices.find((x) => x.id === id);
    if (!d) throw problem(404, "not_found", "No such device", id);
    return d;
  };
  function insertDevice(d: DeviceRecord): void {
    state.devices = state.devices.filter((x) => x.id !== d.id);
    const at = state.devices.findIndex((x) => rank(x.id) > rank(d.id));
    state.devices.splice(at < 0 ? state.devices.length : at, 0, d);
    originals.set(d.id, structuredClone(d));
  }

  /** How long a flash takes in the mock. The contract has no "flashing" device state, so the record stays as it was meanwhile. */
  const FLASH_MS = 2000;
  /** Boards a flash is running on: a second request answers 409 until it finishes. */
  const flashing = new Set<string>();
  function finishFlash(id: string): void {
    flashing.delete(id);
    const d = state.devices.find((x) => x.id === id);
    const offer = d?.flash_offer;
    if (!d || !offer) return; // forgotten (or changed) while flashing
    d.firmware = { family: offer.image.family, version: offer.image.version, source: "probe" };
    d.flash_offer = null;
    d.actions = d.actions.filter((a) => a !== "flash");
    d.reason_code = "radio_unbound";
    d.reason = "RNode firmware is on the board, but nothing is bound to it yet.";
    d.last_seen = iso(clock.now());
    originals.set(d.id, structuredClone(d)); // a forgotten board that is plugged in again keeps its new firmware
    publishDevices();
  }

  // --- Layer 5: messages, contacts, fabric, nearby, panels -----------------------------------------------
  const M = state.messaging;
  /** The longest text the mock carries; the daemon refuses what Tier 0/1 framing cannot (§5) with the same 400. */
  const MAX_TEXT = 2000;
  /** How long after a forgotten request its un-ACKed sender retransmits the INTRO. */
  const RETRANSMIT_MS = 30_000;
  const contactOf = (id: string): Contact | undefined => M.contacts.find((c) => c.node_id === id);
  const publishMessage = (m: Message, change: "new" | "state" | "request") =>
    bus.publish({ event: "message", data: { change, peer: m.peer, message: structuredClone(m) } });
  const publishContact = (c: Contact, change: "added" | "updated" | "removed" | "request") =>
    bus.publish({ event: "contact", data: { change, contact: structuredClone(c) } });
  const publishFabric = () => bus.publish({ event: "fabric", data: structuredClone(M.fabric) });
  const publishNearby = () => bus.publish({ event: "nearby", data: { sources: structuredClone(M.nearby) } });
  const publishPanels = () => bus.publish({ event: "panels", data: { panels: structuredClone(M.panels) } });
  const loopbackOnly = () => {
    if (!state.auth.loopback) throw problem(403, "loopback_only", "Available on the node itself", "Change this from the machine the node runs on.");
  };

  /**
   * Re-derive the `fabric` row from the snapshot and publish both frames (the registry does this each tick in the daemon).
   * `status.layers` follows from the row and the restore floor (5 only while Messaging runs); like the daemon, a changed
   * snapshot is published as a `status` frame.
   */
  function syncFabric(): void {
    ctx.patchService("fabric", fabricRow(M.fabric));
    publishFabric();
    const layers = layersRunning(svc("fabric").state, M.restorePending);
    if (layers.join() !== state.status.layers.join()) {
      state.status.layers = layers;
      publishStatus();
    }
  }

  /** Why a message cannot leave right now, or null: the reason codes of §15, in the order the daemon checks them. */
  function blockOf(peer: NodeId): string | null {
    if (M.restorePending) return "restore_pending";
    if (M.fabric.store.store_full) return "store_full";
    const c = contactOf(peer);
    if (!c || !c.has_key) return "waiting_for_key";
    if ((M.routes[peer] ?? "direct") === "hub_not_yours") return "hub_not_yours";
    return null;
  }

  function setMessage(m: Message, st: Message["state"], code: string | null): void {
    m.state = st;
    m.reason_code = code;
    m.reason = code ? (MOCK_REASONS[code] ?? null) : null;
    publishMessage(m, "state");
  }

  /** Message ids with a delivery chain running, so a retry never starts a second one. */
  const inflight = new Set<string>();
  const RANK: Record<string, number> = { queued: 0, sent: 1, relayed: 2, delivered: 3 };

  /**
   * Move one own message along: blocked stays queued with the reason; otherwise sent, then relayed or delivered by the route.
   * A cancelled message (spec 4: `queued`, `reason_code: cancelled`, an empty placeholder goes in its place) is never moved
   * again, so a later retry or re-evaluation of the outbox does not overwrite that reason.
   */
  function dispatch(m: Message): void {
    if (m.dir !== "own" || (m.state !== "queued" && m.state !== "sent")) return;
    if (m.reason_code === "cancelled") return;
    const block = blockOf(m.peer);
    if (block) {
      if (m.state !== "queued" || m.reason_code !== block) setMessage(m, "queued", block);
      return;
    }
    if (inflight.has(m.id)) return;
    const route = M.routes[m.peer] ?? "direct";
    const steps: [number, Message["state"], string | null][] =
      route === "mailbox_full"
        ? [[400, "sent", "mailbox_full"]]
        : route === "mailbox"
          ? [[400, "sent", null], [800, "relayed", null], [6000, "delivered", null]]
          : [[400, "sent", null], [700, "delivered", null]];
    const todo = steps.filter(([, st]) => RANK[st]! > RANK[m.state]! || (st === m.state && m.reason_code !== null && m.reason_code !== steps[0]?.[2]));
    if (todo.length === 0) return;
    if (m.state === "queued" && m.reason_code) setMessage(m, "queued", null);
    inflight.add(m.id);
    const next = (i: number) => {
      const step = todo[i];
      if (!step) {
        inflight.delete(m.id);
        return;
      }
      later(step[0], () => {
        // Gone, or cancelled while this step was waiting (it was still queued, so it never leaves): the chain stops here.
        if (!M.messages.includes(m) || m.reason_code === "cancelled") {
          inflight.delete(m.id);
          return;
        }
        setMessage(m, step[1], step[2]);
        next(i + 1);
      });
    };
    next(0);
  }

  /** Something changed that could unblock the outbox (a key arrived, space was freed, a restore finished). */
  const reevaluate = () => {
    for (const m of M.messages) if (m.dir === "own" && (m.state === "queued" || m.state === "sent")) dispatch(m);
  };

  const incoming = (peer: NodeId, text: string, claimOffsetMs: number | null, change: "new" | "request"): Message => {
    const seq = M.nextSeq++;
    const n = M.messages.filter((x) => x.peer === peer && x.dir === "in").length + 1;
    const m: Message = {
      id: `msg-01j9zd${(M.nextMsg++).toString().padStart(4, "0")}`,
      peer,
      dir: "in",
      text,
      state: "delivered",
      reason_code: null,
      reason: null,
      local_seq: seq,
      received_seq: M.nextRecv++,
      sent_at_claim: claimOffsetMs === null ? null : iso(clock.now() + claimOffsetMs),
      at: iso(clock.now()),
      msg_id: { stream: 0, epoch: 9120, n },
    };
    M.messages.push(m);
    publishMessage(m, change);
    return m;
  };

  /** Asking the home hub for a typed NodeId's card: it answers, in the mock, when a hub is enrolled and reachable. */
  function askHubForCard(id: NodeId): void {
    later(4000, () => {
      const c = contactOf(id);
      if (!c || c.has_key || !(M.fabric.hub.enrolled && M.fabric.hub.reachable)) return;
      c.has_key = true;
      publishContact(c, "updated");
      reevaluate();
    });
  }

  const messageById = (id: string): Message => {
    const m = M.messages.find((x) => x.id === id);
    if (!m) throw problem(404, "not_found", "No such message", id);
    return m;
  };
  const contactById = (id: string): Contact => {
    const c = contactOf(id);
    if (!c) throw problem(404, "not_found", "No such contact", id);
    return c;
  };

  const ctx: ScriptContext = {
    now: () => clock.now(),
    patchStatus(fn) {
      fn(state.status);
      publishStatus();
    },
    patchService(id, patch) {
      const s = svc(id);
      if (patch.state && patch.state !== s.state) s.since = iso(clock.now());
      Object.assign(s, patch);
      publishServices();
    },
    addDevice(d) {
      insertDevice(d);
      publishDevices();
    },
    removeDevice(id) {
      state.devices = state.devices.filter((x) => x.id !== id);
      publishDevices();
    },
    addProposal(p) {
      state.proposals.unshift(p);
      bus.publish({ event: "proposal", data: { id: p.id, title: p.title, kind: p.kind, created_at: p.created_at } });
    },
    emitAgent(kind, detail = {}) {
      bus.publish({ event: "agent", data: { schema: 1, seq: ++agentSeq, ts: iso(clock.now()), kind, detail } });
    },
    log(id, text) {
      const lines = state.logs[id];
      lines.push(logLine(clock.now(), text));
      if (lines.length > 500) lines.splice(0, lines.length - 500);
    },
    receive(peer, text, o) {
      incoming(peer, text, o?.claimOffsetMs ?? null, contactOf(peer)?.request ? "request" : "new");
    },
    introduce(contact, texts) {
      M.contacts.push(structuredClone(contact));
      publishContact(contact, "request");
      for (const t of texts) incoming(contact.node_id, t, null, "request");
    },
    patchMessaging(fn) {
      fn(M);
      syncFabric();
      reevaluate();
    },
  };

  /** Wrap one REST call: latency, "daemon down" window, pairing gate. */
  async function rest<T>(fn: () => T, o: { auth?: boolean } = {}): Promise<T> {
    if (latency > 0) await sleep(latency);
    else await Promise.resolve();
    if (clock.now() < downUntil) throw ApiError.network(new TypeError("Failed to fetch"));
    if (o.auth !== false && state.auth.requirePair && !state.auth.paired) {
      throw problem(401, "unauthenticated", "Pairing required", "Enter the pair code shown by `111 dashboard` on the node.");
    }
    return structuredClone(fn());
  }

  function agentBlock(): ApiError | null {
    const agent = svc("agent");
    if (agent.state !== "live") return problem(503, "agent_unavailable", "The local agent is not available", agent.reason);
    return null;
  }
  function chatBlock(): ApiError | null {
    const a = agentBlock();
    if (a) return a;
    const model = svc("model");
    if (model.state !== "live") return problem(503, "model_unavailable", "The local model is not available", model.reason);
    return null;
  }

  function recordTurn(req: ChatInput, pre: string, final: string): void {
    const h = histories.get(req.conversation_id) ?? [];
    h.push(
      { role: "user", content: req.message },
      { role: "assistant", content: pre.trim(), tool_calls: [{ id: "call_1", type: "function", function: { name: "kb.read", arguments: "{}" } }] },
      { role: "tool", content: "(knowledge base entry)" },
      { role: "assistant", content: final },
    );
    histories.set(req.conversation_id, h);
    ctx.emitAgent("chat.turn", { conversation_id: req.conversation_id, tool_trace: [{ name: "kb.read", ok: true }] });
  }

  async function* chatSend(req: ChatInput, o?: SendOptions): AsyncGenerator<ChatFrame, void, void> {
    await rest(() => {
      const errors: NonNullable<Problem["errors"]> = [];
      if (!CHAT_ID.test(String(req.conversation_id ?? ""))) errors.push({ pointer: "/conversation_id", message: "must match [A-Za-z0-9_-]{1,128}" });
      const m = req.message;
      if (typeof m !== "string" || m.length < 1 || m.length > 16000) errors.push({ pointer: "/message", message: "must be 1-16000 characters" });
      if (errors.length) throw problem(400, "bad_request", "Invalid chat request", undefined, errors);
      const block = chatBlock();
      if (block) throw block;
    });
    const canned = pickReply(req.message);
    const final = finalSentence(canned.post);
    const trace = [{ name: "kb.read", ok: true }];
    const checkAbort = () => {
      if (o?.signal?.aborted) throw new DOMException("The operation was aborted.", "AbortError");
    };
    if (req.stream === false) {
      recordTurn(req, canned.pre, final);
      yield { event: "done", data: { conversation_id: req.conversation_id, reply: final, tool_trace: trace } };
      return;
    }
    for (const text of tokenize(canned.pre)) {
      await sleep(tokenDelay);
      checkAbort();
      yield { event: "token", data: { text } };
    }
    await sleep(tokenDelay * 3);
    checkAbort();
    yield { event: "tool", data: { name: "kb.read" } };
    await sleep(tokenDelay * 3);
    for (const text of tokenize(canned.post)) {
      await sleep(tokenDelay);
      checkAbort();
      yield { event: "token", data: { text } };
    }
    recordTurn(req, canned.pre, final);
    yield { event: "done", data: { conversation_id: req.conversation_id, reply: final, tool_trace: trace } };
  }

  function startOfflineWindow(ms: number): void {
    downUntil = clock.now() + ms;
    bus.disconnect(ms);
    bus.newLifetime();
  }

  /** The daemon restarts: brief outage, new event-id lifetime, children `starting` then live again. */
  function simulateRestart(): void {
    later(300, () => {
      const outage = 2500;
      startOfflineWindow(outage);
      uptimeOrigin = clock.now() + outage;
      state.status.bind.lan = state.status.lan_enabled;
      state.status.bind.urls = state.status.lan_enabled
        ? ["http://111.local", "http://192.168.1.42", "http://127.0.0.1:11180"]
        : ["http://111.localhost", "http://127.0.0.1:11180"];
      const touched = state.services.filter((s) => s.kind === "child" && s.state === "live").map((s) => ({ s, keep: { reason: s.reason, reason_code: s.reason_code } }));
      for (const { s } of touched) {
        s.state = "starting";
        s.since = iso(clock.now() + outage);
        s.reason_code = "starting";
        s.reason = "Starting up.";
      }
      later(outage + 6000, () => {
        for (const { s, keep } of touched) {
          s.state = "live";
          s.since = iso(clock.now());
          s.reason = keep.reason;
          s.reason_code = keep.reason_code;
        }
        publishServices();
      });
    });
  }

  /** How long a supervised child shows `starting` after POST /services/{id}/restart. */
  const CHILD_RESTART_MS = 1500;

  /**
   * One supervised child restarts: `starting` now, `live` again after CHILD_RESTART_MS. A child that was
   * in `error` comes back healthy (the supervisor's respawn clears the back-off), as the healthy fixture has it.
   */
  function restartChild(id: ServiceId): void {
    const s = svc(id);
    const was = { state: s.state, reason: s.reason, reason_code: s.reason_code, detail: structuredClone(s.detail) };
    ctx.patchService(id, { state: "starting", reason_code: "starting", reason: "Restarting.", detail: { ...s.detail, child: "running" } });
    later(CHILD_RESTART_MS, () => {
      if (was.state !== "error") {
        ctx.patchService(id, { state: "live", reason: was.reason, reason_code: was.reason_code, detail: was.detail });
        return;
      }
      const base = buildFixtures(t0);
      const healthy = base.services.find((x) => x.id === id);
      if (healthy) ctx.patchService(id, { state: "live", reason: healthy.reason, reason_code: healthy.reason_code, detail: structuredClone(healthy.detail) });
      ctx.patchStatus((st) => {
        if (id === "agent") st.intelligence = { ...st.intelligence, agent: base.status.intelligence.agent, agent_state: base.status.intelligence.agent_state };
        if (id === "model") st.intelligence = { ...st.intelligence, model_server: base.status.intelligence.model_server };
        if (id === "transport") {
          st.transport_state = base.status.transport_state;
          st.health = structuredClone(base.status.health);
        }
      });
    });
  }

  function simulateTask(req: TaskRequest, taskId: string): void {
    later(400, () => ctx.emitAgent("task.submitted", { task_id: taskId, kind: req.kind }));
    later(3500, () => {
      const id = `prop-${taskId.slice(5)}`;
      ctx.emitAgent("task.done", { task_id: taskId });
      ctx.emitAgent("proposal.written", { id, task_id: taskId });
      ctx.addProposal({
        schema: 1,
        id,
        node_id: state.status.node.node_id,
        task_id: taskId,
        kind: req.kind,
        title: req.kind === "diagnose" ? "Diagnosis ready for review" : "A change is proposed for review",
        body: `Task ${taskId} finished.${req.note ? `\n\nYou asked: ${req.note}` : ""}\n\nNothing was changed on the node. Accept to record that you agree; dismiss to close it.`,
        created_at: iso(clock.now()),
        decision: null,
      });
    });
  }

  function acceptOffer(): void {
    state.settings.offer = "accepted";
    if (state.status.offer) state.status.offer.state = "accepted";
    const model = svc("model");
    if (model.state === "unset") {
      ctx.patchService("model", { state: "starting", reason_code: "starting", reason: "Downloading and starting the model.", detail: { ...model.detail, child: "running", offer: "accepted" } });
      later(8000, () => {
        ctx.patchService("model", {
          state: "live",
          reason_code: "health_ok",
          reason: "Model loaded and answering health checks.",
          actions: ["restart", "log"],
          detail: { child: "running", port: 11181, model_id: "qwen2.5-3b-instruct-q4_k_m", offer: "accepted" },
        });
        ctx.patchStatus((s) => {
          s.intelligence = { ...s.intelligence, model_server: "running", agent_state: "local", model_id: "qwen2.5-3b-instruct-q4_k_m" };
        });
        state.agentStatus = { ...state.agentStatus, state: "local", model: { base: "http://127.0.0.1:11181", reachable: true, id: "qwen2.5-3b-instruct-q4_k_m" } };
      });
    }
  }

  const adapter: Adapter = {
    kind: "mock",

    status: () => rest(statusNow),
    services: () => rest(() => state.services),
    service: (id) => rest(() => svc(id)),
    restartService: (id) =>
      rest((): ServiceRestartReply => {
        const s = svc(id);
        if (!s.actions.includes("restart")) {
          throw problem(409, "not_supervised", "This service is not supervised by the node", "The node does not run this one itself, so it cannot restart it.");
        }
        restartChild(id);
        return { schema: 1, id, restarting: true };
      }),
    serviceLog: (id, n) =>
      rest((): LogTail => {
        svc(id); // 404 for unknown ids
        const count = Math.min(500, Math.max(1, Number.isFinite(n) ? Math.trunc(n as number) : 100));
        const lines = state.logs[id];
        return { schema: 1, service: id, source: lines.length ? `/var/lib/111/log/${svc(id).name}.log` : "none", lines: lines.slice(-count) };
      }),

    chat: {
      send: chatSend,
      history: (conversationId) =>
        rest(() => {
          const block = agentBlock();
          if (block) throw block;
          return { schema: 1, conversation_id: conversationId, messages: histories.get(conversationId) ?? [] } satisfies ChatHistory;
        }),
    },

    devices: () => rest(() => state.devices),
    device: (id) => rest(() => device(id)),
    patchDevice: (id: string, patch: DevicePatch) =>
      rest((): DeviceRecord => {
        const d = device(id);
        if ("alias" in patch) {
          const a = patch.alias;
          if (a !== null && a !== undefined && (typeof a !== "string" || a.length > 64)) {
            throw problem(400, "bad_request", "Invalid name", undefined, [{ pointer: "/alias", message: "must be at most 64 characters" }]);
          }
          d.alias = typeof a === "string" && a.trim() !== "" ? a.trim() : null;
          publishDevices();
        }
        return d;
      }),
    forgetDevice: (id: string) =>
      rest(() => {
        const d = device(id);
        ctx.removeDevice(id);
        const hardware = d.kind !== "companion" && d.kind !== "node";
        if (hardware && state.replugAfterMs !== null) {
          later(state.replugAfterMs, () => {
            const original = originals.get(id);
            if (!original || state.devices.some((x) => x.id === id)) return;
            const now = iso(clock.now());
            ctx.addDevice({ ...structuredClone(original), alias: null, first_seen: now, since: now, last_seen: now });
          });
        }
      }).then(() => undefined),

    flashDevice: (id: string, body: FlashRequest) =>
      rest((): FlashReply => {
        const d = device(id);
        const offer = d.flash_offer;
        const stale = !offer || !d.actions.includes("flash") || body?.sha256 !== offer.image.sha256;
        if (stale || flashing.has(id)) {
          throw problem(
            409,
            "offer_stale",
            "The flash offer changed",
            !stale ? "A flash is already running on this board." : "The offer for this board is not the one you confirmed. Reload the device and read it again.",
          );
        }
        const shown = (d.alias ?? "").trim() !== "" ? (d.alias as string) : d.name;
        if (typeof body.confirm !== "string" || body.confirm.trim() !== shown.trim()) {
          throw problem(400, "bad_request", "Invalid confirmation", undefined, [{ pointer: "/confirm", message: "must be the device's name, as shown" }]);
        }
        flashing.add(id);
        later(FLASH_MS, () => finishFlash(id));
        return { schema: 1, id, flashing: true };
      }),

    identifyDevice: (id: string, body: IdentifyRequest) =>
      rest((): DeviceRecord => {
        const d = device(id);
        if (!body || (body as { consent?: unknown }).consent !== true) {
          throw problem(400, "bad_request", "Consent is required", "Opening a serial port can reset the board. Say that you agree first.", [{ pointer: "/consent", message: "must be true" }]);
        }
        if (!d.actions.includes("identify")) throw problem(400, "bad_request", "This device cannot be identified", "It does not offer the identify step.");
        // The probe finds Meshtastic: the board is bound, meshmud serves it, the reader fills Nearby, the panel is served.
        d.firmware = { family: "meshtastic", version: "2.5.9", source: "probe" };
        d.driver = { stack: "meshtastic", interface: "meshmud", live: true };
        d.service_id = "meshtastic";
        d.state = "connected";
        d.reason_code = "stack_bound";
        d.reason = "Meshtastic is on the board and meshmud drives it.";
        d.actions = d.actions.filter((a) => a !== "identify");
        d.last_seen = iso(clock.now());
        publishDevices();
        M.nearby = buildNearby(clock.now());
        M.panels = buildPanels(true);
        const name = (d.alias ?? "").trim() !== "" ? (d.alias as string) : d.name;
        const live = meshtasticLiveRow(name, M.nearby.reduce((n, x) => n + x.nodes.length, 0));
        ctx.patchService("meshtastic", { ...live, panel: structuredClone(M.panels[0]!) });
        ctx.patchService("radio", { state: "live", reason_code: "radio_bound_meshtastic", reason: "A Meshtastic board is bound, driven by meshmud.", detail: { devices: [d.id], port: 4403, driven_by: "meshmud" } });
        publishNearby();
        publishPanels();
        return d;
      }),

    agentStatus: () =>
      rest(() => {
        const block = agentBlock();
        if (block) throw block;
        return state.agentStatus;
      }),
    agentTask: (req) =>
      rest((): TaskAccepted => {
        if (!["diagnose", "propose", "patch"].includes(req.kind)) {
          throw problem(400, "bad_request", "Unknown task kind", undefined, [{ pointer: "/kind", message: "must be diagnose, propose or patch" }]);
        }
        const block = agentBlock();
        if (block) throw block;
        if (req.kind === "patch") throw problem(409, "needs_hub", "This task needs a hub node", "Patch tasks run only on hub-category nodes.");
        const taskId = `task-${Math.floor(clock.now() / 1000).toString(36)}${(++agentSeq).toString(36).padStart(4, "0")}`;
        simulateTask(req, taskId);
        return { schema: 1, task_id: taskId };
      }),

    proposals: (filter: ProposalFilter = "open") =>
      rest(() => {
        const all = [...state.proposals].sort((a, b) => (a.created_at < b.created_at ? 1 : -1));
        return filter === "all" ? all : all.filter((p) => (filter === "open" ? p.decision === null : p.decision !== null));
      }),
    proposal: (id) =>
      rest(() => {
        const p = state.proposals.find((x) => x.id === id);
        if (!p) throw problem(404, "not_found", "No such proposal", id);
        return p;
      }),
    decide: (id: string, decision: Decision) =>
      rest((): Proposal => {
        if (decision !== "accepted" && decision !== "dismissed") {
          throw problem(400, "bad_request", "Invalid decision", undefined, [{ pointer: "/decision", message: "must be accepted or dismissed" }]);
        }
        const p = state.proposals.find((x) => x.id === id);
        if (!p) throw problem(404, "not_found", "No such proposal", id);
        if (p.decision) throw problem(409, "already_decided", "This proposal was already decided", `It was ${p.decision.decision}.`);
        p.decision = { decision, at: iso(clock.now()) };
        return p;
      }),

    settings: () => rest(() => state.settings),
    patchSettings: (patch: SettingsPatch) =>
      rest((): Settings => {
        if (!state.auth.loopback && ("lan_enabled" in patch || "offer" in patch)) {
          throw problem(403, "loopback_only", "Available on the node itself", "Change this setting from the machine the node runs on.");
        }
        if ("lan_enabled" in patch && state.settings.lan_locked) {
          throw problem(409, "settings_locked", "LAN sharing is pinned", "DASHBOARD_LAN in the node's environment file sets this. Change it there.");
        }
        if ("alias" in patch) {
          const a = patch.alias;
          if (a !== null && a !== undefined && (typeof a !== "string" || a.length > 64)) {
            throw problem(400, "bad_request", "Invalid name", undefined, [{ pointer: "/alias", message: "must be at most 64 characters" }]);
          }
          const alias = typeof a === "string" && a.trim() !== "" ? a.trim() : null;
          state.settings.alias = alias;
          state.status.node.alias = alias;
        }
        if ("lan_enabled" in patch && typeof patch.lan_enabled === "boolean") {
          state.settings.lan_enabled = patch.lan_enabled;
          state.status.lan_enabled = patch.lan_enabled; // takes effect on restart
        }
        if ("offer" in patch) {
          if (patch.offer !== "accepted" && patch.offer !== "dismissed") {
            throw problem(400, "bad_request", "Invalid offer", undefined, [{ pointer: "/offer", message: "must be accepted or dismissed" }]);
          }
          if (patch.offer === "accepted") acceptOffer();
          else {
            state.settings.offer = "dismissed";
            if (state.status.offer) state.status.offer.state = "dismissed";
          }
        }
        publishStatus();
        return state.settings;
      }),

    restart: () =>
      rest((): RestartReply => {
        simulateRestart();
        return { schema: 1, restarting: true };
      }),
    updateCheck: () =>
      rest((): UpdateCheckReply => {
        const u = state.status.update;
        if (u.state !== "idle" && u.state !== "failed") return { schema: 1, started: false };
        u.state = "checking";
        publishStatus();
        later(1500, () => {
          ctx.patchStatus((s) => {
            s.update = { ...s.update, state: "idle", last_check: iso(clock.now()), last_result: "up_to_date" };
          });
        });
        return { schema: 1, started: true };
      }),

    messages: {
      threads: () => rest((): Thread[] => threadsOf(M)),
      thread: (peer: NodeId, q: ThreadQuery = {}) =>
        rest(() => {
          if (!/^[0-9a-f]{64}$/.test(peer)) throw problem(400, "bad_request", "Invalid peer", undefined, [{ pointer: "/peer", message: "must be 64 lowercase hex characters" }]);
          const limit = q.limit ?? 100;
          if (!Number.isInteger(limit) || limit < 1 || limit > 500) throw problem(400, "bad_request", "Invalid limit", undefined, [{ pointer: "/limit", message: "must be between 1 and 500" }]);
          if (q.sinceSeq !== undefined && (!Number.isInteger(q.sinceSeq) || q.sinceSeq < 0)) throw problem(400, "bad_request", "Invalid since_seq", undefined, [{ pointer: "/since_seq", message: "must be 0 or more" }]);
          const known = contactOf(peer);
          if (!known && !M.messages.some((m) => m.peer === peer)) throw problem(404, "not_found", "No such conversation", peer);
          const page = pageOf(M, peer, q.sinceSeq, limit);
          return { peer, request: known?.request === true, messages: page.messages, more: page.more };
        }),
      send: (req: SendMessageRequest) =>
        rest((): MessageAccepted => {
          const errors: NonNullable<Problem["errors"]> = [];
          if (typeof req?.to !== "string") errors.push({ pointer: "/to", message: "must be a node id" });
          if (typeof req?.text !== "string" || req.text.trim() === "") errors.push({ pointer: "/text", message: "must not be empty" });
          else if (req.text.length > MAX_TEXT) errors.push({ pointer: "/text", message: `must be at most ${MAX_TEXT} characters` });
          if (errors.length) throw problem(400, "bad_request", "Invalid message", "A text that the framing cannot carry is refused, never cut short.", errors);
          const to = contactOf(req.to);
          if (!to || to.request) throw problem(404, "not_found", "Not a contact", "Add them as a contact first.");
          const code = blockOf(to.node_id);
          const m: Message = {
            id: `msg-01j9zd${(M.nextMsg++).toString().padStart(4, "0")}`,
            peer: to.node_id,
            dir: "own",
            text: req.text,
            state: "queued",
            reason_code: code,
            reason: code ? (MOCK_REASONS[code] ?? null) : null,
            local_seq: M.nextSeq++,
            received_seq: null,
            sent_at_claim: null,
            at: iso(clock.now()),
            msg_id: { stream: 0, epoch: 9120, n: M.messages.filter((x) => x.peer === to.node_id && x.dir === "own").length + 1 },
          };
          M.messages.push(m);
          publishMessage(m, "new");
          dispatch(m);
          return { schema: 1, id: m.id };
        }),
      retry: (id: string) =>
        rest((): Message => {
          const m = messageById(id);
          if (m.dir !== "own" || (m.state !== "queued" && m.state !== "sent")) {
            throw problem(409, "bad_request", "Not retryable", "Only an own message that is queued or sent can be retried.");
          }
          dispatch(m);
          return m;
        }),
      // POST /messages/{id}/cancel (spec 4): an own message that is still queued stays in its thread, queued, with
      // reason_code cancelled; 409 once it has left, when it is not ours, or when it was cancelled already; 404 when unknown.
      cancel: (id: string) =>
        rest((): void => {
          const m = messageById(id);
          if (cancelQueued(m) !== "cancelled") {
            throw problem(409, "bad_request", "Not cancellable", "Only an own message that is still queued can be cancelled.");
          }
          publishMessage(m, "state");
        }),
    },

    contacts: {
      list: () => rest((): Contact[] => M.contacts),
      add: (body: ContactCreate) =>
        rest((): Contact => {
          const card = (body as { card?: unknown } | null)?.card;
          const nodeId = (body as { node_id?: unknown } | null)?.node_id;
          if ((card === undefined) === (nodeId === undefined)) {
            throw problem(400, "bad_request", "Invalid contact", "Give a card or a node id, not both and not neither.");
          }
          if (card !== undefined) {
            const parsed = typeof card === "string" ? parseCardText(card) : null;
            if (!parsed || !parsed.ok) {
              throw problem(400, "bad_request", "That is not a card", parsed ? CARD_PROBLEM_TEXT[parsed.problem] : undefined, [{ pointer: "/card", message: "not a card" }]);
            }
            if (parsed.card.toLowerCase() === M.card.text.toLowerCase()) throw problem(400, "bad_request", "That is this node's own card", "Share your card with others; there is nothing to add.");
            const id = hex64(parsed.card.toLowerCase());
            const known = contactOf(id);
            if (known) return known;
            const c = makeContact(id, { trust: "oob", has_key: true });
            M.contacts.push(c);
            M.routes[id] = "direct";
            publishContact(c, "added");
            return c;
          }
          const parsed = typeof nodeId === "string" ? parseNodeId(nodeId) : null;
          if (!parsed || !parsed.ok) {
            throw problem(400, "bad_request", "That is not a node id", parsed ? NODE_ID_PROBLEM_TEXT[parsed.problem] : undefined, [{ pointer: "/node_id", message: "not a node id" }]);
          }
          if (parsed.hex === M.ownNodeId) throw problem(400, "bad_request", "That is this node", "A node does not message itself.");
          const known = contactOf(parsed.hex);
          if (known) return known;
          const c = makeContact(parsed.hex, { trust: "oob", has_key: false });
          M.contacts.push(c);
          M.routes[parsed.hex] = "direct";
          publishContact(c, "added");
          askHubForCard(parsed.hex);
          return c;
        }),
      ownCard: () => rest((): Card => M.card),
      get: (id: string) => rest((): Contact => contactById(id)),
      patch: (id: string, patch: ContactPatch) =>
        rest((): Contact => {
          const c = contactById(id);
          const errors: NonNullable<Problem["errors"]> = [];
          if ("alias" in patch && patch.alias !== null && patch.alias !== undefined && (typeof patch.alias !== "string" || patch.alias.length > 64)) errors.push({ pointer: "/alias", message: "must be at most 64 characters" });
          if ("accept" in patch && (patch as { accept?: unknown }).accept !== true) errors.push({ pointer: "/accept", message: "only true is valid" });
          if ("pinned" in patch && typeof patch.pinned !== "boolean") errors.push({ pointer: "/pinned", message: "must be true or false" });
          if ("read_seq" in patch && (!Number.isInteger(patch.read_seq) || (patch.read_seq as number) < 0)) errors.push({ pointer: "/read_seq", message: "must be 0 or more" });
          if (errors.length) throw problem(400, "bad_request", "Invalid change", undefined, errors);
          let changed = false;
          if ("alias" in patch) {
            const alias = typeof patch.alias === "string" && patch.alias.trim() !== "" ? patch.alias.trim() : null;
            if (c.alias !== alias) changed = true;
            c.alias = alias;
          }
          if (patch.accept === true && c.request) {
            c.request = false;
            changed = true;
          }
          if (typeof patch.pinned === "boolean" && c.pinned !== patch.pinned) {
            c.pinned = patch.pinned;
            changed = true;
          }
          if (typeof patch.read_seq === "number") M.readMarks[id] = Math.max(M.readMarks[id] ?? 0, patch.read_seq);
          if (changed) publishContact(c, "updated");
          return c;
        }),
      forget: (id: string) =>
        rest(() => {
          const c = contactById(id);
          M.contacts = M.contacts.filter((x) => x !== c);
          delete M.routes[id];
          // "Forget request" (the UI's "ignore" never reaches the node, §7.2): a discarded request takes its held
          // messages with it; a forgotten contact keeps its history (until capacity pruning).
          if (c.request) {
            const held = M.messages.filter((m) => m.peer === id && m.dir === "in").map((m) => m.text);
            M.messages = M.messages.filter((m) => m.peer !== id);
            // The sender was never ACKed (§9: kept until delivered), so it still has its copy and retransmits on its
            // next link event: the request comes back, as a fresh INTRO with the same words.
            later(RETRANSMIT_MS, () => {
              if (!contactOf(id)) ctx.introduce({ ...c, request: true }, held);
            });
          }
          publishContact(c, "removed");
        }).then(() => undefined),
    },

    fabric: {
      status: () => rest((): FabricStatus => M.fabric),
      enrollHub: (body: HubEnrollRequest) =>
        rest((): FabricHub => {
          const errors: NonNullable<Problem["errors"]> = [];
          if (typeof body?.address !== "string" || !/^[A-Za-z0-9.[\]:-]+:\d{1,5}$/.test(body.address.trim())) errors.push({ pointer: "/address", message: "must be host:port" });
          const code = typeof body?.code === "string" ? body.code.replace(/[\s-]/g, "").toUpperCase() : "";
          if (code.length !== 8) errors.push({ pointer: "/code", message: "must be 8 characters" });
          if (errors.length) throw problem(400, "bad_request", "Invalid enrollment", undefined, errors);
          if (M.fabric.hub.enrolled) throw problem(409, "bad_request", "Already enrolled", "This node has one home hub. Forget it before enrolling with another.");
          if (M.enrollFailures >= 5) throw problem(429, "rate_limited", "Too many attempts", "The hub locked this code after five failures. Ask its owner for a new one.");
          if (code !== M.hubCode || body.address.trim() !== MOCK_HUB_ADDRESS) {
            M.enrollFailures++;
            throw problem(502, "upstream_error", "The hub did not accept that", "It could not be reached, or it refused the code.");
          }
          M.enrollFailures = 0;
          M.fabric.hub = { ...M.fabric.hub, enrolled: true, reachable: true, cursor: 0, node_id: HUB, address: MOCK_HUB_ADDRESS };
          M.fabric.links = M.fabric.links.map((l) => ({ ...l, state: "up", peers: Math.max(1, l.peers) }));
          if (!contactOf(HUB)) {
            const hub = makeContact(HUB, { alias: "Workshop hub", trust: "enrolled", roles: ["mailbox", "always_on_hub"], pinned: true });
            M.contacts.push(hub);
            M.routes[HUB] = "direct";
            publishContact(hub, "added");
          }
          syncFabric();
          reevaluate();
          return M.fabric.hub;
        }),
      offerHub: () =>
        rest((): HubOffer => {
          loopbackOnly();
          const code = `HUB${(++M.tickets).toString().padStart(5, "0")}`;
          const address = "192.168.1.42:11183";
          M.offer = { code };
          M.fabric.hub = { ...M.fabric.hub, offer_active: true };
          publishFabric();
          return { schema: 1, code, address, card: M.card, bundle: cardFor(hubBundleText(M.card.text, code, address)) };
        }),
      dismissHubOffer: () =>
        rest(() => {
          loopbackOnly();
          M.offer = null;
          if (M.fabric.hub.offer_active) {
            M.fabric.hub = { ...M.fabric.hub, offer_active: false };
            publishFabric();
          }
        }).then(() => undefined),
      patchSettings: (patch: FabricSettingsPatch) =>
        rest((): FabricSettings => {
          loopbackOnly();
          const errors: NonNullable<Problem["errors"]> = [];
          for (const key of ["hub_role", "listen_lan", "custody_opt_in"] as const) {
            if (key in patch && typeof patch[key] !== "boolean") errors.push({ pointer: `/${key}`, message: "must be true or false" });
          }
          if ("quotas" in patch) {
            const q = patch.quotas as Record<string, unknown> | undefined;
            if (!q || typeof q !== "object" || Object.values(q).some((v) => v !== null && (typeof v !== "number" || !Number.isFinite(v) || v < 0))) {
              errors.push({ pointer: "/quotas", message: "must map classes to a number of bytes or null" });
            }
          }
          if (errors.length) throw problem(400, "bad_request", "Invalid settings", undefined, errors);
          const next = { ...M.fabric.settings };
          if (typeof patch.hub_role === "boolean") next.hub_role = patch.hub_role;
          if (typeof patch.listen_lan === "boolean") next.listen_lan = patch.listen_lan;
          if (typeof patch.custody_opt_in === "boolean") next.custody_opt_in = patch.custody_opt_in;
          if (patch.quotas) {
            const quotas = { ...next.quotas };
            for (const [k, v] of Object.entries(patch.quotas)) {
              if (v === null) delete quotas[k];
              else quotas[k] = v;
            }
            next.quotas = quotas;
          }
          M.fabric.settings = next;
          syncFabric();
          return M.fabric.settings;
        }),
    },

    nearby: () => rest((): NearbySource[] => M.nearby),

    panels: {
      list: () => rest((): HostedPanel[] => M.panels),
      openSession: (id: ServiceId) =>
        rest((): PanelSession => {
          const panel = M.panels.find((p) => p.id === id);
          if (!panel || svc(id).state !== "live") throw problem(404, "not_found", "No such panel", "Unknown panel, or its service is not set up.");
          return { schema: 1, url: `http://127.0.0.1:${panel.port}${panel.entry}?ticket=${hex64(`${id}:${++M.tickets}`).slice(0, 32)}`, expires_in: 30 };
        }),
    },

    kb: {
      index: () => rest(() => state.kbIndex),
      entry: (path) =>
        rest(() => {
          if (!path || KB_UNSAFE.test(path)) throw problem(400, "bad_request", "Invalid path", "Paths are relative to the knowledge base and cannot leave it.");
          const entry = state.kbEntries[path];
          if (entry === undefined) throw problem(404, "not_found", "No such knowledge base entry", path);
          return entry;
        }),
    },

    auth: {
      session: () =>
        rest(
          (): AuthSession => ({
            schema: 1,
            loopback: state.auth.loopback,
            paired: state.auth.paired,
            lan_enabled: state.status.lan_enabled,
            created: state.auth.created,
            loopback_trusted: state.auth.loopbackTrusted,
            device_id: (state.auth.loopback && state.auth.loopbackTrusted) || state.auth.paired ? state.auth.deviceId : null,
          }),
          { auth: false },
        ),
      pair: (code) =>
        rest(
          (): AuthSession => {
            if (clock.now() < pairBlockedUntil) throw problem(429, "rate_limited", "Too many attempts", "Wait a minute, then try again.");
            const norm = String(code).replace(/[\s-]/g, "").toUpperCase();
            if (norm !== MOCK_PAIR_CODE.replace("-", "")) {
              if (++pairFailures >= 5) {
                pairFailures = 0;
                pairBlockedUntil = clock.now() + 60_000;
              }
              throw problem(401, "unauthenticated", "That code did not match", "Check the code printed by `111 dashboard` on the node.");
            }
            pairFailures = 0;
            state.auth.paired = true;
            state.auth.created = iso(clock.now());
            state.status.auth.paired = true;
            queueMicrotask(() => bus.retryGated());
            return {
              schema: 1,
              loopback: state.auth.loopback,
              paired: true,
              lan_enabled: state.status.lan_enabled,
              created: state.auth.created,
              loopback_trusted: state.auth.loopbackTrusted,
              device_id: state.auth.deviceId,
            };
          },
          { auth: false },
        ),
      logout: () =>
        rest(
          () => {
            state.auth.paired = false;
            state.auth.created = null;
            state.status.auth.paired = false;
          },
          { auth: false },
        ).then(() => undefined),
    },

    events: (handlers: EventsHandlers) => bus.subscribe(handlers),
  };

  const mock: MockControls = {
    scenario,
    state,
    bus,
    dropConnection: (ms) => bus.disconnect(ms),
    dispose() {
      disposed = true;
      bus.stop();
      for (const t of timers) clock.clearTimer(t);
      timers.clear();
    },
  };

  if (opts.autoStart !== false) {
    bus.start();
    for (const step of scenario.script) later(step.atMs, () => step.run(ctx));
  }

  return Object.assign(adapter, { mock });
}

/** Names of every service id, for fixtures consumers that want a stable order. */
export const MOCK_SERVICE_ORDER = SERVICE_IDS;
