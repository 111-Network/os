// Layer 6 contract pins (6-transport/README.md §10, task S2): the closed enums, the enum growth on existing schemas, the SSE map,
// and fixtures for the new schemas validated against api/openapi.yaml. Fixtures are hand-written here, not daemon output.
import { describe, expect, it } from "vitest";
import {
  EVENTS_EVENT_NAMES,
  FOREIGN_AUTHS,
  FOREIGN_CONV_KINDS,
  FOREIGN_MESSAGE_STATES,
  FOREIGN_REASON_CODES,
  MESSAGE_STATES,
  NEARBY_KINDS,
  NETWORK_MESSAGE_CHANGES,
  PROBLEM_CODES,
  PROPOSAL_KINDS,
} from "../src/lib/api/types";
import { operations, sse } from "./contract.gen";
import { validateAgainst } from "./helpers/validate";

const HASH = "a".repeat(64);
const CONSENT = { action: "edge_enable", subject: "be-1", warning_text: "Everyone in A will have their messages copied to B.", hash: HASH, addons: [] };
const PLAN = { bytes: 40, cap: 232, parts: 1, size_mode: "refuse", airtime_estimate: { millis: 2100, label: "about 2.1 s (estimate)" }, refusal: null, warnings: [] };
const FOREIGN_MSG = {
  id: "fm-7", conv: "fc-3", dir: "in", sender: { native: "!a1b2c3d4", name: "Sam" }, auth: "channel-key", text: "hello", state: "delivered",
  evidence: null, reason_code: null, reason: null, group_id: null, part: null, relayed_from: null, via_internet: false, seq: 12,
  at: "2026-10-19T09:00:00Z", claim_ts: null, truncated: null,
};
const EDGE = {
  id: "be-1", src: "fc-3", dst: "fc-4", src_kind: "channel", dst_kind: "group", enabled: true, enabled_by: "loopback", paused: false, cursor_seq: 10, relayed_seq: 12, honour_ok_to_mqtt: false, oversize: "refuse",
  consent_hash: HASH, counters: { relayed: 3, shed: 0, refused: 1 }, pause_reason: null,
};
const COMPONENT = {
  id: "lxmf", adapter: "lxmf", state: "cached", cached_bytes: 9_000_000, installed_bytes: null, license: { spdx: "MPL-2.0", read_at: "https://example.invalid/l", notice: "n" },
  notice_hash: HASH, acknowledged: false,
};
const NETWORK = {
  id: "meshtastic", label: "Meshtastic", chip: "MESH", state: "live", account: "Bench radio (!a1b2c3d4)",
  copy: { banner: "b", send_warning: "w", relay_tag: "M", auth: { "channel-key": "anyone with this key" } },
  caps: {
    kinds: ["channel", "dm"], max_text_bytes: { channel: { default: 232 }, dm: { default: 220, licensed: 232 } }, evidence: ["queued", "sent", "relayed", "delivered", "failed"],
    airtime: { duty_cycle_permille: 100, bits_per_s: 1070, rule: "time-on-air formula for the modem preset" }, auth_levels: ["channel-key", "pki"], splittable: true, recipient_syntax: null,
  },
  gate: { region_set: true, licensed: false, ham_confirmed: null, uplink_channels: 0 }, counts: { conversations: 2, unread: 0, queued: 0, edges: 1 }, dropped: 0, component: null,
};

describe("Layer 6 enums (the pins an enum bump touches)", () => {
  it("ForeignMessageState is closed at five and MessageState stays at four", () => {
    expect([...FOREIGN_MESSAGE_STATES]).toEqual(["queued", "sent", "relayed", "delivered", "failed"]);
    expect([...MESSAGE_STATES]).toEqual(["queued", "sent", "relayed", "delivered"]);
  });
  it("auth levels, conversation kinds and reason codes", () => {
    expect([...FOREIGN_AUTHS]).toEqual(["none", "channel-key", "pki", "signed", "attested-111"]);
    expect([...FOREIGN_CONV_KINDS]).toEqual(["channel", "dm", "group"]);
    expect(FOREIGN_REASON_CODES).toHaveLength(20);
  });
  it("Proposal.kind, NearbySource.kind and Problem.code grew additively", () => {
    expect([...PROPOSAL_KINDS]).toEqual(["diagnose", "propose", "patch", "chat", "message", "bridge_message", "bridge_edge", "device"]);
    expect([...NEARBY_KINDS]).toEqual(["meshtastic", "reticulum"]);
    for (const c of ["consent_required", "oversize", "component_unavailable", "unsupported_action", "not_supervised"]) expect(PROBLEM_CODES as readonly string[]).toContain(c);
  });
  it("the SSE map has the two Layer 6 events and Layer 5's are untouched", () => {
    expect(sse.events.network_message).toBe("SseNetworkMessage");
    expect(sse.events.bridge).toBe("SseBridge");
    expect([...EVENTS_EVENT_NAMES].slice(0, 11)).toEqual(["status", "services", "devices", "proposal", "agent", "reset", "message", "contact", "fabric", "nearby", "panels"]);
    expect([...NETWORK_MESSAGE_CHANGES]).toEqual(["new", "state", "conversation"]);
  });
});

describe("Layer 6 fixtures validate against the contract", () => {
  it.each([
    ["Network", NETWORK],
    ["Component", COMPONENT],
    ["ForeignConversation", { id: "fc-3", network: "meshtastic", account: "!a1b2c3d4", kind: "channel", native_id: "ch:LongFast:ab12", title: "LongFast", last_seq: 12, unread: 0, pinned: false, send_enabled: true }],
    ["ForeignMessage", FOREIGN_MSG],
    ["ForeignMessage", { ...FOREIGN_MSG, dir: "own", state: "failed", reason_code: "region_unset", reason: "region is not set", group_id: "sg-2", part: { k: 1, n: 2 }, relayed_from: { msg: "fm-1", edge: "be-1" } }],
    ["BridgeEdge", EDGE],
    ["SendGroup", { id: "sg-2", text: "hi", items: [{ target: { network: "meshtastic", conv: "fc-3" }, id: "fm-8", plan: PLAN }, { target: { node_id: "b".repeat(64) }, id: "msg-1", plan: PLAN }] }],
    ["FanoutResult", { schema: 1, group_id: "sg-2", items: [{ target: { network: "lxmf", conv: "fc-9", size_mode: "split" }, id: "fm-8", ids: ["fm-8", "fm-9"], plan: { ...PLAN, parts: 2, size_mode: "split" } }] }],
    ["ConsentRequired", CONSENT],
    ["Problem", { type: "about:blank", title: "Confirm", status: 409, code: "consent_required", consent: CONSENT }],
    ["Problem", { type: "about:blank", title: "Refused", status: 409, code: "oversize", targets: [{ target: { network: "meshtastic", conv: "fc-3" }, code: "oversize", detail: "312 B > 232 B per packet", consent: null }] }],
    ["TransportStatus", {
      schema: 1, interfaces: [{ name: "RNode 1", type: "RNodeInterface", up: true, rx: 10, tx: 20, peers: null }], interfaces_state: "ok",
      posture: { share_instance: true, instance_name: "111", enable_transport: true, lan: { backbone_listen: false, port: 4242, ifac: false }, stock_autointerface: false, propagation_node: false },
      region: "EU_868", callsign: null, order: ["111", "lxmf", "meshtastic"],
    }],
    ["TransportJoin", { schema: 1, address: "192.168.1.20", port: 4242, ifac_name: "111", ifac_passphrase: "x" }],
    ["BridgeSettings", { schema: 1, order: ["111"], adapters: { meshtastic: true }, recipient_lists: [], channels: [], ham: [], announce: { name: null, interval_min: 60 } }],
    ["SseNetworkMessage", { network: "meshtastic", conv: "fc-3", change: "new", seq: 12 }],
    ["SseBridge", { networks: [NETWORK], edges: [EDGE] }],
    ["NearbySource", { id: "s1", kind: "reticulum", label: "heard via Reticulum", device_id: null, nodes: [] }],
    ["Proposal", { schema: 1, id: "prop-1", node_id: null, task_id: null, kind: "bridge_edge", title: "t", body: "from: fc-3\nto: fc-4", created_at: "2026-10-19T09:00:00Z", decision: null }],
  ])("%s", (name, value) => {
    expect(validateAgainst(name, value)).toEqual([]);
  });

  it("a Message-style bad value is rejected: a foreign state outside the closed enum", () => {
    expect(validateAgainst("ForeignMessage", { ...FOREIGN_MSG, state: "read" })).not.toEqual([]);
  });
});

describe("Layer 6 operations", () => {
  it("are all in the contract with the expected verbs", () => {
    const want: Record<string, [string, string]> = {
      listNetworks: ["GET", "/api/v1/networks"],
      listForeignConversations: ["GET", "/api/v1/networks/{net}/conversations"],
      openForeignConversation: ["POST", "/api/v1/networks/{net}/conversations"],
      getForeignConversation: ["GET", "/api/v1/networks/{net}/conversations/{conv}"],
      sendForeignMessage: ["POST", "/api/v1/networks/{net}/conversations/{conv}/messages"],
      previewFanout: ["POST", "/api/v1/messages/fanout/preview"],
      sendFanout: ["POST", "/api/v1/messages/fanout"],
      listBridgeEdges: ["GET", "/api/v1/bridge/edges"],
      createBridgeEdge: ["POST", "/api/v1/bridge/edges"],
      getBridgeEdge: ["GET", "/api/v1/bridge/edges/{id}"],
      patchBridgeEdge: ["PATCH", "/api/v1/bridge/edges/{id}"],
      deleteBridgeEdge: ["DELETE", "/api/v1/bridge/edges/{id}"],
      getBridgeSettings: ["GET", "/api/v1/bridge/settings"],
      patchBridgeSettings: ["PATCH", "/api/v1/bridge/settings"],
      listBridgeComponents: ["GET", "/api/v1/bridge/components"],
      enableBridgeComponent: ["POST", "/api/v1/bridge/components/{id}/enable"],
      disableBridgeComponent: ["POST", "/api/v1/bridge/components/{id}/disable"],
      removeBridgeComponentFiles: ["POST", "/api/v1/bridge/components/{id}/remove-files"],
      getTransport: ["GET", "/api/v1/transport"],
      patchTransportPosture: ["PATCH", "/api/v1/transport/posture"],
      joinTransport: ["POST", "/api/v1/transport/join"],
    };
    for (const [id, [method, path]] of Object.entries(want)) expect([operations[id]?.method, operations[id]?.path], id).toEqual([method, path]);
  });
});
