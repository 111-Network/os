// Base fixtures for the dev-only mock: one believable laptop node ("Node2B") with all endpoints'
// data, shaped exactly like api/openapi.yaml. Scenarios (scenarios.ts) clone and bend these.
// Nothing here is imported by production code: the mock is reached only via a DEV-guarded dynamic import.
import type { AgentStatus, DeviceRecord, Proposal, ServiceId, ServiceRecord, Settings, Status } from "../types";
import { buildMessaging, type MessagingData } from "./messaging";

export const NODE_ID = "9f2c41d7a0b35e68c1d4f7092ab36e5d8c70f1e24b9a6d3057c8e1f2a4b6d9c3";
export const NODE_LABEL = "k7m2-9x4p-aa";
export const NODE_BECH32 = "node1n5c8x0q4zrk3g7v2ld9wm6tyh0ea5fu8s4j2p7qa3xk9d6c1ly0m8w2g4";
export const VERSION = "2026.10.13";
export const MODEL_ID = "qwen2.5-3b-instruct-q4_k_m";
export const HELTEC_ID = "usb-7c1e0b94a2d3";
export const KB_SHA = "5be1f09a7c2d84e3b6a1900fd27c4e58a3b9d1c06e7f24a85b3c9d0e1f6a7b82";

export const iso = (ms: number): string => new Date(ms).toISOString();

/**
 * `Status.layers` as 111d reports it (status.rs `layers()`): layers 0 to 4 as on the mock laptop, plus 5 (Messaging & data,
 * os/50-messaging/README.md) while the fabric runs: its `fabric` service row is LIVE and no identity restore is pending
 * (nothing is sealed or sent until it finishes). An unset or errored fabric (no hub yet, a full store) and a restore floor
 * leave 5 out.
 */
export function layersRunning(fabricState: ServiceRecord["state"] | undefined, restorePending: boolean): number[] {
  const layers = [0, 1, 2, 3, 4];
  if (fabricState === "live" && !restorePending) layers.push(5);
  return layers;
}

export interface Fixtures {
  /** Epoch ms the fixtures were built at; "since" / "created_at" fields are relative to it. */
  now: number;
  status: Status;
  services: ServiceRecord[];
  /** Registry order: radio board, paired phone, bluetooth adapter, unidentified serial device. */
  devices: DeviceRecord[];
  proposals: Proposal[];
  settings: Settings;
  agentStatus: AgentStatus;
  kbIndex: string;
  kbEntries: Record<string, string>;
  /** Log lines per service id; empty for services without a child process. */
  logs: Record<ServiceId, string[]>;
  /** Layer 5: contacts, messages, fabric, nearby, panels (mock/messaging.ts). */
  messaging: MessagingData;
}

const MIN = 60_000;
const HOUR = 60 * MIN;

const LOG_TEMPLATES: Partial<Record<ServiceId, string[]>> = {
  agent: [
    "INFO  http listening addr=127.0.0.1:11182",
    "INFO  model reachable base=http://127.0.0.1:11181",
    "INFO  chat turn conversation=conv-4k2m9 tools=1 ms=2140",
    "INFO  kb loaded entries=4",
    "INFO  health ok",
  ],
  model: [
    "INFO  llama-server: model loaded n_ctx=4096 n_threads=4",
    "INFO  slot 0 | prompt processing done, n_tokens=212",
    "INFO  slot 0 | generation done, n_tokens=96 tok/s=14.2",
    "INFO  GET /health 200",
  ],
  transport: [
    "[Notice] Started Reticulum instance",
    "[Notice] Interface AutoInterface[Default Interface] is up",
    "[Notice] Heard announce from <6b2f9a4c0d>",
    "[Notice] Path to <6b2f9a4c0d> is 1 hop away",
  ],
  radio: ["INFO  no meshtasticd instance configured"],
  meshtastic: [
    "INFO  meshmud: downlink listening addr=127.0.0.1:11230",
    "INFO  meshmud: uplink connected tcp://192.168.1.60:4403",
    "INFO  presence: node database written nodes=3",
  ],
};

function logLines(id: ServiceId, now: number, count = 120): string[] {
  const t = LOG_TEMPLATES[id];
  if (!t) return [];
  const out: string[] = [];
  for (let k = 0; k < count; k++) {
    const at = now - (count - k) * 15_000;
    out.push(`${iso(at).slice(0, 19)}Z ${t[k % t.length]}`);
  }
  return out;
}

const KB_ENTRIES: Record<string, string> = {
  "projects/meshtastic.md": `# Meshtastic

Meshtastic is an open LoRa mesh. A small radio board relays short messages between nodes several kilometres apart, with no internet.

## What the node needs
- A supported LoRa board on USB or serial.
- \`meshtasticd\` running on this node (the dashboard shows this as "LoRa radio").

## Set up
1. Plug the board in and wait a few seconds.
2. Ask the agent to set up the LoRa radio. It will read this page and give you the exact steps.
3. Confirm the service turns LIVE in the sidebar.

## Troubleshooting
- Port 4403 refused: the daemon is not running; restart it from the service page.
- No device found: try another cable; many cables are charge-only.
`,
  "projects/reticulum.md": `# Reticulum

Reticulum is the network stack the 111 node uses to reach other nodes without a central server.

## Check it is healthy
- The sidebar shows "Mesh transport" as LIVE.
- The Overview says whether other nodes were heard (joined) or not (isolated).

## Common problems
- **Degraded**: the daemon runs but no interface passes traffic. Check Wi-Fi or the radio link.
- **Not set**: python3 or the Reticulum virtual environment is missing; the agent can list the steps.
`,
  "projects/llama-cpp.md": `# llama.cpp (the local model)

\`llama-server\` runs the language model on this node. Everything stays on the device.

## Sizing
- 4 GB RAM: a 1-3B model, quantized to Q4.
- 8 GB RAM or more: a 3-7B model.

## Health
The service is LIVE when \`GET /health\` answers 200. Loading a model can take up to two minutes after start.
`,
  "devices/heltec-wifi-lora-32-v3.md": `# Heltec WiFi LoRa 32 V3

An ESP32-S3 board with a LoRa radio and a small display. Common hardware for Meshtastic.

## How the node recognizes it
- USB ids \`303a:1001\` (Espressif USB JTAG/serial). It appears under \`/dev/serial/by-id/\`.

## Set up
1. Make sure the board runs Meshtastic firmware. If it shows the stock Heltec demo, flash Meshtastic first.
2. Ask the agent to set up the LoRa radio. It will read the Meshtastic page and give you the steps.
3. Confirm "LoRa radio" turns LIVE in the sidebar and this device turns connected.

## Troubleshooting
- Permission denied on the port: the node user needs to be in the \`dialout\` group.
- Port busy: another program, for example a serial monitor, has the port open.
`,
  "projects/opencode.md": `# opencode (code agent)

A coding assistant that runs on hub nodes. Laptop nodes (category 2B) do not run it.

## Requirements
- A hub-category node.
- \`opencode\` on the PATH.
`,
};

const KB_COUNT = Object.keys(KB_ENTRIES).length;

const KB_INDEX = `# 111 network knowledge base

> Offline recipes and concepts for running a 111 node. Read by the local agent and by you.

## Devices
- [Heltec WiFi LoRa 32 V3](devices/heltec-wifi-lora-32-v3.md): identify and set up this LoRa board

## Projects
- [Meshtastic](projects/meshtastic.md): LoRa radio mesh messaging, board setup and troubleshooting
- [Reticulum](projects/reticulum.md): the mesh transport, health checks and common faults
- [llama.cpp](projects/llama-cpp.md): the local model server, sizing and health
- [opencode](projects/opencode.md): the code agent for hub nodes
`;

/** Radio service facts while no LoRa board is attached. */
export const RADIO_NO_BOARD = {
  reason_code: "no_radio",
  reason: "No LoRa radio was detected on this device.",
  can_run_here: false,
  requires: "A LoRa board, for example a Meshtastic-compatible USB device.",
  detail: { devices: [] as string[], port: 4403 },
};

/** Radio service facts once a board is attached but meshtasticd is not installed. */
export const RADIO_BOARD_FOUND = {
  reason_code: "radio_unbound",
  reason: "A LoRa board is plugged in, but nothing is bound to it yet.",
  can_run_here: true,
  requires: "a mesh stack bound to the board — a Reticulum RNode interface or Meshtastic.",
  detail: { devices: [HELTEC_ID] as string[], port: 4403 },
};

export function heltecDevice(now: number): DeviceRecord {
  return {
    id: HELTEC_ID,
    kind: "radio",
    alias: null,
    name: "Heltec WiFi LoRa 32 V3",
    state: "needs_setup",
    transport: "usb-serial",
    first_seen: iso(now - 4 * MIN),
    since: iso(now - 4 * MIN),
    last_seen: iso(now),
    reason_code: "no_driver_service",
    reason: "The board is plugged in, but the LoRa radio service is not running yet.",
    capabilities: ["lora"],
    service_id: "radio",
    kb_entry: "devices/heltec-wifi-lora-32-v3.md",
    identity: {
      vendor_id: "303a",
      product_id: "1001",
      serial: "90:E5:B1:A1:2C:48",
      port: "/dev/serial/by-id/usb-Espressif_USB_JTAG_serial_debug_unit_90:E5:B1:A1:2C:48-if00",
    },
    actions: ["rename", "forget", "agent"],
    detail: { driver: "cdc_acm", usb_path: "1-2", manufacturer: "Espressif", product_string: "USB JTAG/serial debug unit", matched_board: "heltec-wifi-lora-32-v3" },
  };
}

function baseDevices(now: number): DeviceRecord[] {
  return [
    heltecDevice(now),
    {
      id: "ses-4be2a91c07d5",
      kind: "companion",
      alias: null,
      name: "Android phone",
      state: "connected",
      transport: "lan",
      first_seen: iso(now - 2 * 24 * HOUR),
      since: iso(now - 2 * 24 * HOUR),
      last_seen: iso(now - 20_000),
      reason_code: "session_active",
      reason: "Paired with the dashboard and active on the local network.",
      capabilities: ["dashboard-session"],
      identity: {
        ip: "192.168.1.57",
        user_agent: "Mozilla/5.0 (Linux; Android 14; Pixel 6) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Mobile Safari/537.36",
      },
      actions: ["rename", "forget", "agent"],
      detail: { session_created: iso(now - 2 * 24 * HOUR), peer: "192.168.1.57" },
    },
    {
      id: "bt-hci0-5c3d8a1e",
      kind: "bluetooth",
      alias: null,
      name: "Bluetooth adapter (hci0)",
      state: "connected",
      transport: "bluetooth",
      first_seen: iso(now - 7 * HOUR),
      since: iso(now - 7 * HOUR),
      last_seen: iso(now),
      reason_code: "present",
      reason: "The adapter is present and powered on.",
      capabilities: ["ble"],
      identity: { mac: "A4:C3:F0:12:7B:9E", hci: "hci0" },
      actions: ["rename", "forget", "agent"],
      detail: { powered: true, address_type: "public" },
    },
    {
      id: "usb-b3f70d1e58a9",
      kind: "unknown",
      alias: null,
      name: "USB serial device",
      state: "connected",
      transport: "usb-serial",
      first_seen: iso(now - HOUR),
      since: iso(now - HOUR),
      last_seen: iso(now),
      reason_code: "unidentified",
      reason: "Something is plugged in, but it is not a board the node knows.",
      capabilities: [],
      identity: { vendor_id: "1a86", product_id: "7523", port: "/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0" },
      actions: ["rename", "forget", "agent"],
      detail: { driver: "ch341", usb_path: "1-4" },
    },
  ];
}

export function buildFixtures(now: number): Fixtures {
  const svc = (s: Omit<ServiceRecord, "since"> & { sinceMs: number }): ServiceRecord => {
    const { sinceMs, ...rest } = s;
    return { ...rest, since: iso(now - sinceMs) };
  };

  const services: ServiceRecord[] = [
    svc({
      id: "agent",
      name: "111-agent",
      plain_name: "Local AI agent",
      kind: "child",
      state: "live",
      sinceMs: 7 * HOUR,
      reason_code: "health_ok",
      reason: "Running and answering health checks.",
      can_run_here: true,
      requires: "Standard tier: the 111-agent program and a local model.",
      description:
        "A small assistant that runs on this node. It reads the built-in knowledge base and the node's health, answers your questions, and writes proposals you can accept or dismiss. It cannot touch hardware or run commands.",
      actions: ["restart", "log"],
      detail: { child: "running", port: 11182, url: "http://127.0.0.1:11182", model_reachable: true },
    }),
    svc({
      id: "model",
      name: "llama-server",
      plain_name: "Local model",
      kind: "child",
      state: "live",
      sinceMs: 7 * HOUR,
      reason_code: "health_ok",
      reason: "Model loaded and answering health checks.",
      can_run_here: true,
      requires: "Standard tier: llama-server and a model file of about 2 GB.",
      description:
        "The language model the agent thinks with. It runs entirely on this node, so chat works without the internet.",
      kb_entry: "projects/llama-cpp.md",
      actions: ["restart", "log"],
      detail: { child: "running", port: 11181, model_id: MODEL_ID, offer: "accepted" },
    }),
    svc({
      id: "transport",
      name: "rnsd",
      plain_name: "Mesh transport",
      kind: "child",
      state: "live",
      sinceMs: 7 * HOUR,
      reason_code: "rnsd_running",
      reason: "Reticulum is running and has heard other nodes.",
      can_run_here: true,
      requires: "python3 and the node's Reticulum virtual environment.",
      description:
        "Reticulum carries messages between nodes with no central server. It finds neighbours over Wi-Fi, LoRa and other links.",
      kb_entry: "projects/reticulum.md",
      actions: ["restart", "log"],
      detail: { child: "running", peers: "joined", transport_state: "joined" },
    }),
    svc({
      id: "radio",
      name: "meshtasticd",
      plain_name: "LoRa radio",
      kind: "detected",
      state: "unset",
      sinceMs: 4 * MIN,
      ...RADIO_BOARD_FOUND,
      description:
        "Long-range radio for the mesh: it lets this node talk to others several kilometres away without Wi-Fi or internet. A LoRa board must be bound to a stack before it carries traffic.",
      kb_entry: "projects/meshtastic.md",
      actions: [],
    }),
    svc({
      id: "kb",
      name: "kb",
      plain_name: "Knowledge base",
      kind: "builtin",
      state: "live",
      sinceMs: 7 * HOUR,
      reason_code: "verified",
      reason: `Knowledge base verified: ${KB_COUNT} entries.`,
      can_run_here: true,
      description:
        "The offline library of install recipes and concepts that ships with the node. The agent reads it to answer you.",
      actions: [],
      detail: { sha256: KB_SHA, entries: KB_COUNT },
    }),
    svc({
      id: "gateway",
      name: "wan",
      plain_name: "Internet gateway",
      kind: "detected",
      state: "live",
      sinceMs: 3 * HOUR,
      reason_code: "wan_ok",
      reason: "The internet is reachable from this node.",
      can_run_here: true,
      description:
        "Shows whether this node can reach the internet. A node with a working connection can later act as a gateway for others on the mesh.",
      actions: [],
      detail: { probe: "204", interval_sec: 60, last_probe: iso(now - 40_000) },
    }),
    svc({
      id: "codeagent",
      name: "opencode",
      plain_name: "Code agent",
      kind: "detected",
      state: "unset",
      sinceMs: 7 * HOUR,
      reason_code: "not_hub",
      reason: "Only hub nodes run the code agent.",
      can_run_here: false,
      requires: "A hub-category node with opencode installed.",
      description: "A coding assistant (opencode) that runs on hub nodes and helps maintain the network's software.",
      kb_entry: "projects/opencode.md",
      actions: [],
      detail: { category: "Node2B", needs: "Hub4A or Hub4B" },
    }),
    // Layer 5 (interface README 7.2; the wording mirrors the registry rules in 111d).
    svc({
      id: "fabric",
      name: "mesh111",
      plain_name: "Messaging",
      kind: "builtin",
      state: "live",
      sinceMs: 7 * HOUR,
      reason_code: "hub_reachable",
      reason: "Your home hub is reachable; messages are delivered or held for you.",
      can_run_here: true,
      requires: "A home hub to enroll with, or another 111 node reachable on this network.",
      description:
        "Sealed messages between your nodes and your contacts. A message goes straight to the other node when it can be reached, or waits with your home hub until it comes back. It runs inside the node; only the two ends can read a message.",
      actions: [],
      detail: {
        started: true,
        store: { open: true, full: false },
        hub: { enrolled: true, reachable: true, connected: true },
        hub_role: false,
        members: null,
        attributed_links: 1,
      },
    }),
    svc({
      id: "meshtastic",
      name: "meshmud",
      plain_name: "Meshtastic",
      kind: "child",
      state: "unset",
      sinceMs: 4 * MIN,
      reason_code: "no_meshtastic",
      reason: "No Meshtastic board is set up on this node.",
      can_run_here: true,
      requires: "A Meshtastic board identified and bound to this node: plugged in by USB (Linux), or a Wi-Fi board reached by its address.",
      description:
        "Meshtastic's own web client for the Meshtastic board bound to this node, and the list of Meshtastic nodes it hears. What you do in that client goes to your radio and out over Meshtastic; 111 does not see or store it, and 111 itself never transmits on Meshtastic.",
      kb_entry: "projects/meshtastic.md",
      actions: [],
      detail: { child: "absent", board: null },
    }),
  ];

  const messaging = buildMessaging(now, NODE_ID);
  const status: Status = {
    schema: 1,
    node: {
      node_id: NODE_ID,
      label: NODE_LABEL,
      bech32: NODE_BECH32,
      alias: "Workshop laptop",
      category: "Node2B",
      version: VERSION,
      uptime_sec: 7 * 3600 + 412,
    },
    kb: { sha256: KB_SHA, entries: KB_COUNT },
    transport_state: "joined",
    health: { status: "ok", components: { identity: true, kb: true, transport: "alive", updater_idle: true } },
    update: {
      state: "idle",
      pending: false,
      pending_version: null,
      last_check: iso(now - 3 * HOUR - 12 * MIN),
      last_result: "up_to_date",
      floor: "2026.09.01",
      clock_fault: false,
    },
    offer: { tier: "standard", state: "accepted" },
    error_state: null,
    intelligence: {
      agent: "running",
      model_server: "running",
      agent_url: "http://127.0.0.1:11182",
      agent_state: "local",
      model_id: MODEL_ID,
    },
    bind: { urls: ["http://111.localhost", "http://111.local", "http://192.168.1.42", "http://127.0.0.1:11180"], lan: true },
    lan_enabled: true,
    auth: { loopback: true, paired: false },
    layers: layersRunning(services.find((x) => x.id === "fabric")?.state, messaging.restorePending),
  };

  const proposals: Proposal[] = [
    {
      schema: 1,
      id: "prop-01j9z3k8m2q4",
      node_id: NODE_ID,
      task_id: "task-01j9z3k7x1ab",
      kind: "diagnose",
      title: "Mesh transport restarted twice in the last hour",
      body: `**What I saw**

The Reticulum daemon was restarted by the supervisor at 07:12 and 07:48. Both times it came back within 6 seconds and heard the same neighbour again.

**Likely cause**

The Wi-Fi interface dropped for a few seconds, which makes \`rnsd\` exit when its only interface disappears.

**Suggested next step**

Nothing to fix right now. If it repeats, ask me to check the Wi-Fi power-saving setting.
`,
      created_at: iso(now - 25 * MIN),
      decision: null,
    },
    {
      schema: 1,
      id: "prop-01j9y7c2d9f1",
      node_id: NODE_ID,
      task_id: "task-01j9y7bz40xk",
      kind: "propose",
      title: "Raise the model context window to 8192",
      body: `Longer conversations are being truncated at 4096 tokens. This node has enough free memory to double the window.

**Change**: set \`n_ctx\` to 8192 in the model server settings. Memory use grows by about 300 MB.
`,
      created_at: iso(now - 26 * HOUR),
      decision: { decision: "accepted", at: iso(now - 25 * HOUR) },
    },
    {
      schema: 1,
      id: "prop-01j9x4t6h8w3",
      node_id: NODE_ID,
      task_id: null,
      kind: "chat",
      title: "Notes from chat: pairing a phone",
      body: `You asked how to reach the dashboard from a phone. Turn on LAN sharing in Settings, restart the node, then open \`http://111.local\` and enter the pair code printed by \`111 dashboard\`.`,
      created_at: iso(now - 3 * 24 * HOUR),
      decision: { decision: "dismissed", at: iso(now - 3 * 24 * HOUR + 4 * MIN) },
    },
  ];

  const logs = Object.fromEntries(
    (["agent", "model", "transport", "radio", "kb", "gateway", "codeagent", "fabric", "meshtastic"] as const).map((id) => [id, logLines(id, now)]),
  ) as Record<ServiceId, string[]>;

  return {
    now,
    status,
    services,
    devices: baseDevices(now),
    proposals,
    settings: { schema: 1, alias: "Workshop laptop", lan_enabled: true, lan_locked: false, offer: "accepted" },
    agentStatus: {
      schema: 1,
      state: "local",
      model: { base: "http://127.0.0.1:11181", reachable: true, id: MODEL_ID },
      kb: { present: true, entries: KB_COUNT },
      conversations: 3,
      tasks_open: 0,
      uptime_sec: 7 * 3600,
      version: VERSION,
      node_id: NODE_ID,
      category: "Node2B",
    },
    kbIndex: KB_INDEX,
    kbEntries: { ...KB_ENTRIES },
    logs,
    messaging,
  };
}

/** Append a fresh log line to a service's tail (used by scenario scripts). */
export function logLine(at: number, text: string): string {
  return `${iso(at).slice(0, 19)}Z ${text}`;
}
