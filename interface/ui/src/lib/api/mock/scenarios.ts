// Mock scenarios: each is a function from the base fixtures to the data the adapter serves plus a
// script of timed events (relative to adapter creation). The adapter (index.ts) owns the live
// state and implements ScriptContext; scenarios only describe what changes and when.
import { RADIO_BOARD_FOUND, RADIO_NO_BOARD, VERSION, heltecDevice, iso, logLine, type Fixtures } from "./fixtures";
import { ADA, DOV, ELI, FAY, fabricRow, makeContact, type MessagingData } from "./messaging";
import type { AgentEvent, AgentStatus, Contact, DeviceRecord, FlashOffer, NodeId, Proposal, ServiceId, ServiceRecord, Settings, Status } from "../types";

export const SCENARIO_NAMES = [
  "healthy",
  "first-run",
  "agent-down",
  "transport-error",
  "lan-unpaired",
  "termux-unpaired",
  "lan-locked",
  "updating",
  "new-device",
  "blank-board",
  "messaging-new",
  "requests",
  "store-full",
  "restore-pending",
  "meshtastic",
] as const;
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
  /** Layer 5: contacts, conversations, fabric, nearby, panels. */
  messaging: MessagingData;
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
  /** An incoming message from `peer` (a `message` frame; `request` while the peer is an unaccepted request). */
  receive(peer: NodeId, text: string, opts?: { claimOffsetMs?: number }): void;
  /** A new message request (INTRO, tofu): the contact with `request: true` and its held messages; `contact` and `message` frames. */
  introduce(contact: Contact, texts: string[]): void;
  /**
   * Change the messaging data in place (restore floor, store, links, hub), then re-derive the `fabric` service row, re-run
   * every queued message and publish the `fabric` and `services` frames.
   */
  patchMessaging(fn: (m: MessagingData) => void): void;
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
    messaging: c.messaging,
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

/** The image `blank-board` offers. The mock's flashDevice writes exactly this when the flash "finishes". */
export const BLANK_BOARD_OFFER: FlashOffer = {
  reason: "blank",
  image: {
    family: "rnode",
    version: "1.78",
    file: "share/firmware/rnode-heltec-v3-1.78.bin",
    sha256: "f87a2e8af657f07642eee8206b15f83e295c58528c92e497b5d3b5ec8c84ed9c",
    board: "Heltec WiFi LoRa 32 V3",
  },
  erases: "everything on the board, including any Meshtastic settings",
};

export const blankBoard: ScenarioFn = (base) => {
  const data = dataFrom(base);
  // The same Heltec as every other scenario, but nothing answered on it: no firmware, nothing bound, and the
  // signed release carries an RNode image for it (Layer 6, D17). Everything else is unchanged.
  const board = data.devices.find((d) => d.kind === "radio");
  if (!board) throw new Error("mock: no radio fixture");
  board.match = { confidence: "likely", rule: "lora-table 303a:1001" };
  board.firmware = { family: "none", version: null, source: "probe" };
  board.driver = null;
  board.reason_code = "unbound";
  board.reason = "The board is plugged in, but no firmware answered on it, so nothing can drive it yet.";
  board.actions = ["rename", "forget", "agent", "flash"];
  board.flash_offer = structuredClone(BLANK_BOARD_OFFER);
  return {
    name: "blank-board",
    summary: "A Heltec WiFi LoRa 32 V3 with no firmware, offered an RNode image. POST /devices/{id}/flash answers 202; 2 s later the board reports RNode 1.78 and the offer is gone.",
    data,
    script: [],
  };
};

/** Re-derive the `fabric` service row from the messaging data (what 111d's registry does every tick). */
function syncFabricRow(d: MockData): void {
  Object.assign(svc(d, "fabric"), fabricRow(d.messaging.fabric));
}

export const messagingNew: ScenarioFn = (base) => {
  const data = dataFrom(base);
  const m = data.messaging;
  // A node that has never messaged anyone: no hub, no contacts, no conversations; the first thing to do is enroll or add a card.
  m.contacts = [];
  m.messages = [];
  m.routes = {};
  m.readMarks = {};
  m.nextSeq = 1;
  m.nextRecv = 1;
  m.nextMsg = 1;
  m.fabric.hub = { enrolled: false, reachable: false, cursor: 0, node_id: null, address: null, members: 0, offer_active: false };
  m.fabric.links = m.fabric.links.map((l) => ({ ...l, state: "down", peers: 0 }));
  m.fabric.store.used = 120_000;
  syncFabricRow(data);
  return {
    name: "messaging-new",
    summary: "Messaging never used: no contacts, no conversations, no home hub (NOT SET). Enroll with the code K7M2-9X4P at 192.168.1.20:11183, or add a contact first.",
    data,
    script: [],
  };
};

export const requests: ScenarioFn = (base) => {
  const data = dataFrom(base);
  const m = data.messaging;
  const hold = (peer: NodeId, texts: string[], agoMs: number) => {
    texts.forEach((text, i) => {
      const seq = m.nextSeq++;
      m.messages.push({
        id: `msg-01j9zc${seq.toString().padStart(2, "0")}r1q7`,
        peer,
        dir: "in",
        text,
        state: "delivered",
        reason_code: null,
        reason: null,
        local_seq: seq,
        received_seq: m.nextRecv++,
        sent_at_claim: i === 0 ? iso(base.now - agoMs - 3_600_000) : null,
        at: iso(base.now - agoMs + i * 20_000),
        msg_id: { stream: 0, epoch: 9120, n: i + 1 },
      });
    });
  };
  m.contacts.push(makeContact(DOV, { alias: null, trust: "tofu", request: true }), makeContact(ELI, { trust: "tofu", request: true }));
  hold(DOV, ["Hi, I got your card from the meetup. Is this the workshop node?", "(if this is the wrong node, ignore me)"], 14 * 60_000);
  hold(ELI, ["<script>alert('hi')</script> Free coin! Click here"], 3 * 60_000);
  return {
    name: "requests",
    summary:
      "Two pending message requests (INTROs from people not yet contacts), one of them junk. 6 s in a third request arrives; 9 s in a normal message arrives from Ada. Accepting makes the contact and moves the messages into a thread.",
    data,
    script: [
      {
        atMs: 6_000,
        label: "a third message request arrives",
        run: (ctx) => ctx.introduce(makeContact(FAY, { trust: "tofu", request: true }), ["Hello from the lake house! Is anyone on this channel?"]),
      },
      { atMs: 9_000, label: "Ada replies", run: (ctx) => ctx.receive(ADA, "Thanks, got your message. See you at 3.", { claimOffsetMs: -4_000 }) },
    ],
  };
};

export const storeFull: ScenarioFn = (base) => {
  const data = dataFrom(base);
  const m = data.messaging;
  m.fabric.store = { ...m.fabric.store, used: m.fabric.store.watermark, store_full: true };
  // The newest own messages cannot be saved to the outbox any more.
  for (const msg of m.messages.filter((x) => x.dir === "own").slice(-2)) {
    msg.state = "queued";
    msg.reason_code = "store_full";
    msg.reason = "This node's store is full; new messages wait until space is freed.";
  }
  syncFabricRow(data);
  return {
    name: "store-full",
    summary: "The message store is full (the fabric service is in ERROR, store_full). The newest own messages are queued with reason store_full. 12 s in space is freed and the queue moves again.",
    data,
    script: [
      {
        atMs: 12_000,
        label: "space is freed",
        run: (ctx) => ctx.patchMessaging((x) => (x.fabric.store = { ...x.fabric.store, used: Math.round(x.fabric.store.watermark * 0.7), store_full: false })),
      },
    ],
  };
};

export const restorePending: ScenarioFn = (base) => {
  const data = dataFrom(base);
  const m = data.messaging;
  m.restorePending = true;
  const last = m.messages.filter((x) => x.dir === "own" && x.peer === ADA).slice(-1)[0];
  if (last) {
    last.state = "queued";
    last.reason_code = "restore_pending";
    last.reason = "Waiting for the identity restore to finish; nothing is sent until it does.";
  }
  return {
    name: "restore-pending",
    summary: "The identity holder reports a restore floor: nothing is sealed, so own messages stay queued with reason restore_pending. 12 s in the restore finishes.",
    data,
    script: [{ atMs: 12_000, label: "restore finishes", run: (ctx) => ctx.patchMessaging((x) => (x.restorePending = false)) }],
  };
};

export const meshtastic: ScenarioFn = (base) => {
  const data = dataFrom(base);
  // The same Heltec as everywhere, but one that Meshtastic could be on: it offers the consented identify probe.
  const board = data.devices.find((d) => d.kind === "radio");
  if (!board) throw new Error("mock: no radio fixture");
  board.actions = ["rename", "forget", "agent", "identify"];
  return {
    name: "meshtastic",
    summary: "A Heltec that offers `identify`. POST /devices/{id}/identify (consent true) binds it: the meshtastic service goes LIVE, Nearby fills with three nodes (one with a hostile name) and the panel is served.",
    data,
    script: [],
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
  "blank-board": blankBoard,
  "messaging-new": messagingNew,
  requests,
  "store-full": storeFull,
  "restore-pending": restorePending,
  meshtastic,
};

export function resolveScenario(name: string | null | undefined): ScenarioName {
  return (SCENARIO_NAMES as readonly string[]).includes(name ?? "") ? (name as ScenarioName) : "healthy";
}
