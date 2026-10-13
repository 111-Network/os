// Mock scenarios: each is a function from the base fixtures to the data the adapter serves plus a
// script of timed events (relative to adapter creation). The adapter (index.ts) owns the live
// state and implements ScriptContext; scenarios only describe what changes and when.
import { RADIO_BOARD_FOUND, RADIO_NO_BOARD, VERSION, heltecDevice, iso, logLine, type Fixtures } from "./fixtures";
import type { AgentEvent, AgentStatus, DeviceRecord, Proposal, ServiceId, ServiceRecord, Settings, Status } from "../types";

export const SCENARIO_NAMES = ["healthy", "first-run", "agent-down", "transport-error", "lan-unpaired", "termux-unpaired", "lan-locked", "updating", "new-device"] as const;
export type ScenarioName = (typeof SCENARIO_NAMES)[number];

/** The pair code `lan-unpaired` accepts (dashes, spaces and case are ignored, like the daemon). */
export const MOCK_PAIR_CODE = "AAAA-BBBB";

export interface MockAuth {
  /** Loopback peers bypass auth and may use loopback-only settings. */
  loopback: boolean;
  /** Policy (`auth/session.loopback_trusted`): false under Termux, where a loopback browser pairs like any other. */
  loopbackTrusted: boolean;
  /** Every /api/v1 call except /auth/* answers 401 until pairing succeeds. */
  requirePair: boolean;
  paired: boolean;
  created: string | null;
  /**
   * The companion record that is "this browser" (`GET /auth/session` -> device_id), for loopback and paired
   * callers. The mock pretends the dev browser is the paired phone fixture so the feature can be seen.
   */
  deviceId: string | null;
}

export interface MockData {
  status: Status;
  services: ServiceRecord[];
  devices: DeviceRecord[];
  /**
   * After a device of a hardware kind (not a companion or node) is forgotten, it reappears this
   * many ms later: the daemon sees the plugged-in board again on its next scan (15 s here). null: forgotten for good.
   */
  replugAfterMs: number | null;
  proposals: Proposal[];
  settings: Settings;
  agentStatus: AgentStatus;
  kbIndex: string;
  kbEntries: Record<string, string>;
  logs: Record<ServiceId, string[]>;
  auth: MockAuth;
}

/** What a script step may do. Every mutation publishes the matching frame on the bus. */
export interface ScriptContext {
  now(): number;
  /** Mutate the status in place; publishes `status`. */
  patchStatus(fn: (s: Status) => void): void;
  /** Merge into a service record (`since` moves when `state` changes); publishes `services`. */
  patchService(id: ServiceId, patch: Partial<ServiceRecord>): void;
  /** Add (or replace) a device in registry order and publish the `devices` frame. */
  addDevice(d: DeviceRecord): void;
  /** Remove a device and publish the `devices` frame. */
  removeDevice(id: string): void;
  /** Add a proposal and publish the `proposal` frame. */
  addProposal(p: Proposal): void;
  /** Publish an `agent` frame with the next agent `seq`. */
  emitAgent(kind: AgentEvent["kind"], detail?: Record<string, unknown>): void;
  /** Append a line to a service's log tail. */
  log(id: ServiceId, text: string): void;
}

export interface TimedEvent {
  /** Milliseconds after the adapter was created. */
  atMs: number;
  label: string;
  run(ctx: ScriptContext): void;
}

export interface Scenario {
  name: ScenarioName;
  summary: string;
  data: MockData;
  script: TimedEvent[];
}

export type ScenarioFn = (base: Fixtures) => Scenario;

function dataFrom(base: Fixtures): MockData {
  const c = structuredClone(base);
  return {
    status: c.status,
    services: c.services,
    devices: c.devices,
    replugAfterMs: 15_000,
    proposals: c.proposals,
    settings: c.settings,
    agentStatus: c.agentStatus,
    kbIndex: c.kbIndex,
    kbEntries: c.kbEntries,
    logs: c.logs,
    auth: { loopback: true, loopbackTrusted: true, requirePair: false, paired: false, created: null, deviceId: c.devices.find((d) => d.kind === "companion")?.id ?? null },
  };
}

function svc(d: MockData, id: ServiceId): ServiceRecord {
  const s = d.services.find((x) => x.id === id);
  if (!s) throw new Error(`mock: no service ${id}`);
  return s;
}

export const healthy: ScenarioFn = (base) => {
  const data = dataFrom(base);
  return {
    name: "healthy",
    summary: "Laptop node, everything that can run is live. A LoRa board waits for setup, a phone is paired. One open diagnose proposal. Forgotten hardware reappears after 15 s.",
    data,
    script: [
      {
        atMs: 8_000,
        label: "agent heard a chat turn",
        run: (ctx) => ctx.emitAgent("chat.turn", { conversation_id: "conv-demo", tools: 1 }),
      },
    ],
  };
};

export const firstRun: ScenarioFn = (base) => {
  const data = dataFrom(base);
  const s = data.status;
  s.node.alias = null;
  s.node.uptime_sec = 240;
  s.transport_state = "isolated";
  s.update.last_check = null;
  s.update.last_result = null;
  s.lan_enabled = false;
  s.bind = { urls: ["http://111.localhost", "http://127.0.0.1:11180"], lan: false };
  data.devices = data.devices.filter((d) => d.kind !== "companion");
  s.offer = { tier: "standard", state: "offered" };
  s.intelligence = { agent: "running", model_server: "absent", agent_url: "http://127.0.0.1:11182", agent_state: "unavailable", model_id: null };
  data.settings = { schema: 1, alias: null, lan_enabled: false, lan_locked: false, offer: "offered" };
  data.proposals = [];
  data.agentStatus = {
    ...data.agentStatus,
    state: "unavailable",
    model: { base: "http://127.0.0.1:11181", reachable: false },
    conversations: 0,
    uptime_sec: 240,
  };
  const model = svc(data, "model");
  model.state = "unset";
  model.reason_code = "no_model";
  model.reason = "No local model is installed yet. The standard tier is on offer.";
  model.can_run_here = true;
  model.actions = [];
  model.detail = { child: "absent", port: 11181, offer: "offered" };
  const transport = svc(data, "transport");
  transport.reason = "Reticulum is running. No other nodes heard yet.";
  transport.detail = { child: "running", peers: "isolated", transport_state: "isolated" };
  return {
    name: "first-run",
    summary: "Fresh install: no peers heard, no model yet (standard tier offered), identity backup unknown.",
    data,
    script: [],
  };
};

export const agentDown: ScenarioFn = (base) => {
  const data = dataFrom(base);
  const s = data.status;
  s.intelligence = { agent: "backoff", model_server: "degraded", agent_url: "http://127.0.0.1:11182", agent_state: "unavailable", model_id: s.intelligence.model_id ?? null };
  const agent = svc(data, "agent");
  agent.state = "error";
  agent.reason_code = "child_backoff";
  agent.reason = "The agent stopped unexpectedly and is being restarted.";
  agent.detail = { child: "backoff", restarts: 4, next_retry_sec: 8 };
  const model = svc(data, "model");
  model.state = "error";
  model.reason_code = "health_503";
  model.reason = "The model server answers but says it is not ready.";
  model.detail = { child: "degraded", port: 11181, last_status: 503 };
  data.logs.agent.push(logLine(base.now - 20_000, "ERROR panic: model client closed"), logLine(base.now - 12_000, "WARN  restarting in 8s (attempt 4)"));
  data.logs.model.push(logLine(base.now - 30_000, "ERROR slot 0 | failed to allocate KV cache"));
  return {
    name: "agent-down",
    summary: "The agent and the model server are in error; chat and agent routes answer 503.",
    data,
    script: [
      {
        atMs: 6_000,
        label: "a queued task fails",
        run: (ctx) => ctx.emitAgent("task.failed", { task_id: "task-01j9z5q0a7tn", reason: "agent_unavailable" }),
      },
    ],
  };
};

export const transportError: ScenarioFn = (base) => {
  const data = dataFrom(base);
  data.status.transport_state = "transport degraded";
  data.status.health = { status: "error", components: { identity: true, kb: true, transport: "degraded", updater_idle: true } };
  const t = svc(data, "transport");
  t.state = "error";
  t.reason_code = "transport_degraded";
  t.reason = "Reticulum is running but is not passing traffic.";
  t.detail = { child: "degraded", peers: "isolated", transport_state: "transport degraded" };
  data.logs.transport.push(
    logLine(base.now - 45_000, "[Error] Interface AutoInterface[Default Interface] lost its link"),
    logLine(base.now - 30_000, "[Warning] No interface is passing traffic"),
  );
  return {
    name: "transport-error",
    summary: "Mesh transport is degraded; the agent leaves a diagnose proposal 10 s in.",
    data,
    script: [
      { atMs: 4_000, label: "diagnose task submitted", run: (ctx) => ctx.emitAgent("task.submitted", { task_id: "task-01j9z8m4c2vd", kind: "diagnose" }) },
      {
        atMs: 10_000,
        label: "proposal written",
        run: (ctx) => {
          const created_at = iso(ctx.now());
          ctx.emitAgent("proposal.written", { id: "prop-01j9z8n7p5ke", task_id: "task-01j9z8m4c2vd" });
          ctx.addProposal({
            schema: 1,
            id: "prop-01j9z8n7p5ke",
            node_id: base.status.node.node_id,
            task_id: "task-01j9z8m4c2vd",
            kind: "diagnose",
            title: "Mesh transport is degraded: no interface passing traffic",
            body: "Reticulum's only interface lost its link about a minute ago. Check that Wi-Fi is connected, then restart the mesh transport from its service page. I can walk you through it.",
            created_at,
            decision: null,
          });
        },
      },
    ],
  };
};

export const lanUnpaired: ScenarioFn = (base) => {
  const data = dataFrom(base);
  data.status.auth = { loopback: false, paired: false };
  data.status.lan_enabled = true;
  data.status.bind = { urls: ["http://111.local", "http://192.168.1.42"], lan: true };
  data.settings.lan_enabled = true;
  data.auth = { ...data.auth, loopback: false, requirePair: true, paired: false, created: null };
  return {
    name: "lan-unpaired",
    summary: `A LAN visitor: every call answers 401 until auth.pair('${MOCK_PAIR_CODE}') succeeds.`,
    data,
    script: [],
  };
};

export const lanLocked: ScenarioFn = (base) => {
  const data = dataFrom(base);
  // DASHBOARD_LAN in the node's environment file pins LAN sharing: a PATCH of lan_enabled answers 409 settings_locked.
  data.settings.lan_enabled = false;
  data.settings.lan_locked = true;
  data.status.lan_enabled = false;
  data.status.bind = { urls: ["http://111.localhost", "http://127.0.0.1:11180"], lan: false };
  return {
    name: "lan-locked",
    summary: "LAN sharing is pinned off by DASHBOARD_LAN in the environment file: settings.lan_locked is true and PATCH lan_enabled answers 409 settings_locked.",
    data,
    script: [],
  };
};

export const termuxUnpaired: ScenarioFn = (base) => {
  const data = dataFrom(base);
  // The node runs on the phone itself and the browser reaches it over loopback, but loopback is not trusted
  // there: every call answers 401 until the code from `111 dashboard` is entered.
  data.status.auth = { loopback: true, paired: false };
  data.auth = { ...data.auth, loopback: true, loopbackTrusted: false, requirePair: true, paired: false, created: null };
  return {
    name: "termux-unpaired",
    summary: `Termux: a loopback browser the node does not trust. Every call answers 401 until auth.pair('${MOCK_PAIR_CODE}') succeeds.`,
    data,
    script: [],
  };
};

export const updating: ScenarioFn = (base) => {
  const data = dataFrom(base);
  const NEXT = "2026.10.14";
  data.status.update = {
    state: "staged",
    pending: true,
    pending_version: NEXT,
    last_check: iso(base.now - 2 * 60_000),
    last_result: "staged",
    floor: "2026.09.01",
    clock_fault: false,
  };
  data.status.health.components = { ...data.status.health.components, updater_idle: false };
  return {
    name: "updating",
    summary: `Update ${VERSION} to ${NEXT}: staged, then probation after 12 s, then idle with the new version after 35 s.`,
    data,
    script: [
      {
        atMs: 12_000,
        label: "probation begins",
        run: (ctx) => ctx.patchStatus((s) => (s.update.state = "probation")),
      },
      {
        atMs: 35_000,
        label: "probation passed: new version live",
        run: (ctx) =>
          ctx.patchStatus((s) => {
            s.node.version = NEXT;
            s.update = { ...s.update, state: "idle", pending: false, pending_version: null, last_result: "staged" };
            s.health.components = { ...s.health.components, updater_idle: true };
          }),
      },
    ],
  };
};

export const newDevice: ScenarioFn = (base) => {
  const data = dataFrom(base);
  data.devices = data.devices.filter((d) => d.kind !== "radio");
  Object.assign(svc(data, "radio"), structuredClone(RADIO_NO_BOARD));
  return {
    name: "new-device",
    summary: "Three devices at first. 8 s in, a Heltec WiFi LoRa 32 V3 is plugged in and appears needing setup. Forgotten hardware reappears after 15 s.",
    data,
    script: [
      {
        atMs: 8_000,
        label: "a LoRa board is plugged in",
        run: (ctx) => {
          ctx.patchService("radio", structuredClone(RADIO_BOARD_FOUND));
          ctx.addDevice(heltecDevice(ctx.now()));
        },
      },
    ],
  };
};

export const scenarios: Record<ScenarioName, ScenarioFn> = {
  healthy,
  "first-run": firstRun,
  "agent-down": agentDown,
  "transport-error": transportError,
  "lan-unpaired": lanUnpaired,
  "termux-unpaired": termuxUnpaired,
  "lan-locked": lanLocked,
  updating,
  "new-device": newDevice,
};

export function resolveScenario(name: string | null | undefined): ScenarioName {
  return (SCENARIO_NAMES as readonly string[]).includes(name ?? "") ? (name as ScenarioName) : "healthy";
}
