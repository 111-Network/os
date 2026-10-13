// Dev-only mock adapter. Same contract as http.ts, no daemon needed:
//   createMockAdapter({ scenario })  -> Adapter & { mock: MockControls }
// Reached ONLY through the DEV-guarded dynamic import in ../index.ts; never import it statically.
import type { Adapter, ChatInput, DevicePatch, EventsHandlers, ProposalFilter, RestartReply, SendOptions, TaskAccepted, UpdateCheckReply } from "../adapter";
import { ApiError } from "../problem";
import type {
  AuthSession,
  ChatFrame,
  ChatHistory,
  Decision,
  DeviceRecord,
  LogTail,
  Problem,
  ProblemCode,
  Proposal,
  ServiceId,
  ServiceRecord,
  Settings,
  SettingsPatch,
  Status,
  TaskRequest,
} from "../types";
import { SERVICE_IDS } from "../types";
import { MockBus, realClock, type MockClock } from "./bus";
import { finalSentence, pickReply, tokenize } from "./chat";
import { buildFixtures, iso, logLine } from "./fixtures";
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
            return { schema: 1, loopback: state.auth.loopback, paired: true, lan_enabled: state.status.lan_enabled, created: state.auth.created };
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
