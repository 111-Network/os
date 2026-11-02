// Layer 7 contract pins (7-relay/README.md §13.1, §13.2, task C1): the closed enums, the enum growth on existing schemas, the SSE map,
// the operations, fixtures for the new schemas validated against api/openapi.yaml, the relay adapter's mock and the store's handling of
// the two new events. Fixtures are hand-written here, not daemon output. The Layer 6 precedent is test/layer6-contract.test.ts.
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Adapter, EventsFrameWithId } from "../src/lib/api/adapter";
import { getRelay, resetAdapterForTests } from "../src/lib/api/index";
import { createMockAdapter, type MockAdapter } from "../src/lib/api/mock";
import { buildFixtures } from "../src/lib/api/mock/fixtures";
import { createRelayMock } from "../src/lib/api/mock/relay";
import { carrierConsentText, GATEWAY_BIT_TEXT, ORIGIN_SERVE_TEXT, PAIR_PLAIN_TEXT, isPrivateAddress } from "../src/lib/api/mock/relay";
import { ageLabel, heardAgoNow, isOneOneOneSource, nodeEvidence } from "../src/lib/api/nearby";
import { ApiError } from "../src/lib/api/problem";
import { relayConsentOf, type RelayAdapter } from "../src/lib/api/relay";
import { ApiStore } from "../src/lib/api/store";
import {
  CONSENT_ACTIONS,
  EVENTS_EVENT_NAMES,
  EVIDENCE_TRANSPORTS,
  MESSAGE_STATES,
  NEARBY_KINDS,
  PAIR_ROLES,
  PAIR_TLS,
  PAIR_VIAS,
  PROBLEM_CODES,
  PROPOSAL_KINDS,
  REGISTRY_ANSWERS,
  RELAY_GATEWAY_STATES,
  type RelayStatus,
} from "../src/lib/api/types";
import { ProposalBody, parseRelayBody, relayActionSentence } from "../src/views/agent/proposal-body";
import { operations, schemas, sse } from "./contract.gen";
import { FakeClock, flush } from "./helpers/fakes";
import { validateAgainst } from "./helpers/validate";

const HASH = "a".repeat(64);
const NODE = "ab".repeat(32);
const T0 = Date.parse("2026-11-02T09:00:00Z");

/** The §13.1 example, plus the keys this contract adds (`carrier` for the write-only flags, `metered`). */
const STATUS: RelayStatus = {
  schema: 1,
  gateway: { enabled: true, state: "carrier_ok", carrier: "ntfy", endpoint: "ntfy.example.org", mode: "5A", checked_ago_s: 240, last_flush_ago_s: 240, pending: 0, metered_cap_reached: false },
  carrier: { kind: "ntfy", server: "https://ntfy.example.org", mode: "5A", token: { set: true }, topic_secret: { set: false }, consent_hash: HASH },
  paired_hubs: [{ node_id: NODE, label: "abcd-efgh-ij", role: "peer", via: "tcp", tls: "rpk", lan: false, reachable: true, pending: 2, reposts: 0 }],
  forward: { refused: 0, too_large: 0 },
  presence: { heartbeat: [{ transport: "rns", interval_min: 60, owner: "layer6" }] },
  discovery: { lan_record: false, public_gateway_bit: false, offload_bit: false },
  origin: { serve: false, targets: ["x86_64-unknown-linux-musl"], bytes: 0 },
  updates: { check_internet: true },
  metered: [{ match: { ssid: "Home" }, daily_cap_bytes: 5_000_000 }],
};
const ROW = { id: NODE, label: "abcd-efgh-ij", node_id: NODE, address: "192.168.1.31:11183", roles: ["mailbox"], answered_by: "hint", heard_ago_s: 1 };
const NEARBY_111 = {
  id: "111", kind: "111", label: "111 nodes heard by this node", device_id: null,
  nodes: [{
    id: NODE, num: 0, long_name: "Sam's laptop", short_name: "abcd-efgh-ij", last_heard_hint: null, heard_ago_s: 95, heard_seq: 18342,
    evidence: [{ transport: "tcp-lan", text: "a message from them arrived (verified)" }], roles: ["mailbox"], position_precision_bits: null,
  }],
};

describe("Layer 7 enums (the pins an enum bump touches)", () => {
  it("Proposal.kind is 9 with `relay` last; NearbySource.kind gained `111`", () => {
    expect([...PROPOSAL_KINDS]).toEqual(["diagnose", "propose", "patch", "chat", "message", "bridge_message", "bridge_edge", "device", "relay"]);
    expect([...NEARBY_KINDS]).toEqual(["meshtastic", "reticulum", "111"]);
  });
  it("Problem.code appends exactly the five codes; `needs_hub` and `settings_locked` already existed", () => {
    expect([...PROBLEM_CODES].slice(-5)).toEqual(["no_such_transport", "licensed_board", "metered_cap", "lan_off", "pair_role_mismatch"]);
    for (const c of ["needs_hub", "settings_locked", "unsupported_action", "component_unavailable", "consent_required", "rate_limited", "loopback_only"]) expect(PROBLEM_CODES as readonly string[]).toContain(c);
  });
  it("ConsentRequired.action appends the four, in order", () => {
    expect([...CONSENT_ACTIONS]).toEqual(["edge_enable", "edge_resume", "component_enable", "ham_confirm", "uplink_send", "carrier_enable", "pair_plain", "gateway_bit", "origin_serve"]);
  });
  it("evidence transports are the closed set tcp-lan | hub | carrier", () => {
    expect([...EVIDENCE_TRANSPORTS]).toEqual(["tcp-lan", "hub", "carrier"]);
  });
  it("the gateway badge has four states, roles three, links two, answers two", () => {
    expect([...RELAY_GATEWAY_STATES]).toEqual(["gateway_none", "carrier_ok", "carrier_pending", "gateway_off"]);
    expect([...PAIR_ROLES]).toEqual(["peer", "leaf", "gateway"]);
    expect([...PAIR_VIAS]).toEqual(["tcp", "carrier"]);
    expect([...PAIR_TLS]).toEqual(["rpk", "plain", "none"]);
    expect([...REGISTRY_ANSWERS]).toEqual(["hint", "signed"]);
  });
  it("MessageState stays at four (`relayed` is widened in its meaning, not its set)", () => {
    expect([...MESSAGE_STATES]).toEqual(["queued", "sent", "relayed", "delivered"]);
    expect(String(schemas.MessageState!.description)).toContain("forwarding");
  });
  it("the SSE map appends `relay` and `registry` after the Layer 6 events; the earlier thirteen are untouched", () => {
    expect(sse.events.relay).toBe("SseRelay");
    expect(sse.events.registry).toBe("SseRegistry");
    expect([...EVENTS_EVENT_NAMES]).toEqual([
      "status", "services", "devices", "proposal", "agent", "reset", "message", "contact", "fabric", "nearby", "panels", "network_message", "bridge", "relay", "registry",
    ]);
  });
});

describe("Layer 7 fixtures validate against the contract", () => {
  it.each([
    ["RelayStatus", STATUS],
    ["RelayStatus", { ...STATUS, gateway: { ...STATUS.gateway, enabled: false, state: "gateway_none", carrier: null, endpoint: null, mode: null, checked_ago_s: null, last_flush_ago_s: null } }],
    ["PairedHub", { node_id: NODE, label: "abcd-efgh-ij", role: "leaf", via: "tcp", tls: "none", lan: true, reachable: false, pending: 0, reposts: 0 }],
    ["PairedHub", { node_id: NODE, label: "abcd-efgh-ij", role: "peer", via: "carrier", tls: "none", lan: false, reachable: true, pending: 1, reposts: 3 }],
    ["RelaySettingsPatch", { carrier: { kind: "ntfy", server: "https://ntfy.example.org", mode: "5A", token: "tk_x" }, accept_warning: HASH }],
    ["RelaySettingsPatch", { gateway: { enabled: true }, discovery: { public_gateway_bit: true }, origin: { serve: true, targets: ["aarch64-unknown-linux-musl"] }, updates: { check_internet: false }, metered: [{ match: { iface: "wlan0" }, daily_cap_bytes: 1000000 }] }],
    ["PairOfferRequest", { role: "gateway", via: "tcp" }],
    ["PairOffer", { schema: 1, bundle: { text: "111PAIR:x", qr: { size: 1, rows: ["1"] } }, code: "K7M29X4P", link_spki: "abcd" }],
    ["PairRequest", { bundle: "111PAIR:x:K7M29X4P:s::peer:tcp", role: "peer", via: "tcp" }],
    ["PairRequest", { bundle: "111PAIR:x:K7M29X4P:s:203.0.113.7:11183:peer:tcp", role: "peer", via: "tcp", lan: false, accept_warning: HASH }],
    ["RelayAccepted", { schema: 1, started: true }],
    ["RegistryRow", ROW],
    ["RegistryRow", { ...ROW, node_id: null, address: null, answered_by: "signed", heard_ago_s: null }],
    ["SseRegistry", { rows: [ROW] }],
    ["SseRelay", STATUS],
    ["UpdateOrigin", { kind: "hub", url: "http://192.168.1.20:11180/origin/v1/", last_answer: { ok: true, ago_s: 12, detail: null }, newest_seen: "2026.11.0" }],
    ["UpdateOrigin", { kind: "internet", url: "https://install.111.network/", last_answer: null, newest_seen: null }],
    ["GatewayDetail", { carrier: "paired-hub", endpoint: "abcd-efgh-ij", mode: null, checked_ago_s: 3, last_flush_ago_s: null, pending: 0, error: null }],
    ["GatewayOffer", { carrier: "ntfy", mode: "5B" }],
    ["HubOffer", { schema: 1, code: "K7M29X4P", address: "192.168.1.20:11183", card: { text: "c", qr: { size: 1, rows: ["1"] } }, bundle: { text: "b", qr: { size: 1, rows: ["1"] } }, gateway: { carrier: "ntfy", mode: "5A" } }],
    ["NearbySource", NEARBY_111],
    ["NearbyNode", { id: "!a1b2c3d4", num: 1, long_name: "n", short_name: "n", last_heard_hint: null }],
    ["ConsentRequired", { action: "carrier_enable", subject: "ntfy:https://ntfy.example.org", warning_text: "t", hash: HASH, addons: ["metered"] }],
    ["ConsentRequired", { action: "pair_plain", subject: NODE, warning_text: "t", hash: HASH, addons: [] }],
    ["Problem", { type: "about:blank", title: "Confirm", status: 409, code: "consent_required", consent: { action: "origin_serve", subject: "origin.serve", warning_text: "t", hash: HASH, addons: [] } }],
    ["Problem", { type: "about:blank", title: "No", status: 409, code: "no_such_transport" }],
    ["Problem", { type: "about:blank", title: "No", status: 409, code: "licensed_board" }],
    ["Problem", { type: "about:blank", title: "No", status: 409, code: "metered_cap" }],
    ["Problem", { type: "about:blank", title: "No", status: 409, code: "lan_off" }],
    ["Problem", { type: "about:blank", title: "No", status: 409, code: "pair_role_mismatch" }],
    ["Proposal", { schema: 1, id: "prop-1", node_id: null, task_id: null, kind: "relay", title: "t", body: '{"why":"w","actions":[{"action":"carrier.flush"}]}', created_at: "2026-11-02T09:00:00Z", decision: null }],
    ["Message", { id: "msg-1", peer: NODE, dir: "own", text: "hi", state: "relayed", reason_code: "forwarding", reason: "Your hub holds it and passes it to their hub.", local_seq: 1, received_seq: null, sent_at_claim: null, at: "2026-11-02T09:00:00Z" }],
  ])("%s", (name, value) => {
    expect(validateAgainst(name, value)).toEqual([]);
  });

  it.each([
    ["a secret value in the status (the carrier token is write-only)", "RelayStatus", { ...STATUS, carrier: { ...STATUS.carrier, token: "tk_secret" } }],
    ["a status without the carrier block", "RelayStatus", (({ carrier: _c, ...rest }) => rest)(STATUS)],
    ["a gateway state outside the four", "RelayStatus", { ...STATUS, gateway: { ...STATUS.gateway, state: "online" } }],
    ["an unknown pair role", "PairedHub", { ...STATUS.paired_hubs[0], role: "boss" }],
    ["an evidence transport outside the closed set", "NearbyEvidence", { transport: "wifi", text: "x" }],
    ["a node id that is not 64 hex", "PairedHub", { ...STATUS.paired_hubs[0], node_id: "abc" }],
    ["a consent action outside the enum", "ConsentRequired", { action: "free_money", subject: "x", warning_text: "t", hash: HASH, addons: [] }],
    ["a proposal kind outside the enum", "Proposal", { schema: 1, id: "p", node_id: null, task_id: null, kind: "rm -rf", title: "t", body: "b", created_at: "2026-11-02T09:00:00Z", decision: null }],
    ["a pair request without a role", "PairRequest", { bundle: "x", via: "tcp" }],
    ["a position precision beyond 32 bits", "NearbyNode", { id: "x", num: 0, long_name: "n", short_name: "n", last_heard_hint: null, position_precision_bits: 40 }],
  ])("rejects %s", (_why, name, value) => {
    expect(validateAgainst(name as string, value)).not.toEqual([]);
  });

  it("no Layer 7 response schema has a string-typed token or topic secret", () => {
    const walk = (node: unknown, path: string, out: string[]): void => {
      if (typeof node !== "object" || node === null) return;
      for (const [k, v] of Object.entries(node)) {
        if ((k === "token" || k === "topic_secret") && (v as { type?: unknown }).type === "string") out.push(`${path}.${k}`);
        walk(v, `${path}.${k}`, out);
      }
    };
    const bad: string[] = [];
    for (const name of ["RelayStatus", "RelayCarrier", "SseRelay", "PairedHub", "PairOffer", "RegistryRow", "UpdateOrigin"]) walk(schemas[name], name, bad);
    expect(bad).toEqual([]);
  });
});

describe("Layer 7 operations", () => {
  it("are all in the contract with the expected verbs and paths", () => {
    const want: Record<string, [string, string]> = {
      getRelay: ["GET", "/api/v1/relay"],
      patchRelaySettings: ["PATCH", "/api/v1/relay/settings"],
      offerRelayPair: ["POST", "/api/v1/relay/pair/offer"],
      pairRelayHub: ["POST", "/api/v1/relay/pair"],
      unpairRelayHub: ["DELETE", "/api/v1/relay/paired/{node_id}"],
      checkRelayCarrier: ["POST", "/api/v1/relay/carrier/check"],
      flushRelayCarrier: ["POST", "/api/v1/relay/carrier/flush"],
      listRegistry: ["GET", "/api/v1/registry"],
      browseRegistry: ["POST", "/api/v1/registry/browse"],
      runNearbyHops: ["POST", "/api/v1/nearby/hops"],
      listUpdateOrigins: ["GET", "/api/v1/update/origins"],
    };
    for (const [id, [method, path]] of Object.entries(want)) expect([operations[id]?.method, operations[id]?.path], id).toEqual([method, path]);
  });
  it("success codes: 202 for the started work, 204 for unpair, 200 for reads and the settings merge", () => {
    expect(Object.fromEntries(["getRelay", "patchRelaySettings", "offerRelayPair", "pairRelayHub", "unpairRelayHub", "checkRelayCarrier", "flushRelayCarrier", "listRegistry", "browseRegistry", "runNearbyHops", "listUpdateOrigins"].map((id) => [id, operations[id]!.success]))).toEqual({
      getRelay: 200, patchRelaySettings: 200, offerRelayPair: 200, pairRelayHub: 202, unpairRelayHub: 204, checkRelayCarrier: 202, flushRelayCarrier: 202, listRegistry: 200, browseRegistry: 202, runNearbyHops: 202, listUpdateOrigins: 200,
    });
  });
  it("the loopback-only pair routes and the consent gates document their 403 and 409", () => {
    for (const id of ["offerRelayPair", "pairRelayHub", "unpairRelayHub"]) expect(operations[id]!.errors, id).toContain(403);
    for (const id of ["patchRelaySettings", "pairRelayHub", "checkRelayCarrier", "flushRelayCarrier", "browseRegistry", "runNearbyHops"]) expect(operations[id]!.errors, id).toContain(409);
    expect(operations.runNearbyHops!.errors).toContain(429);
  });
  it("/origin/v1/* stays outside the contract", () => {
    expect(Object.values(operations).some((o) => o.path.includes("/origin/"))).toBe(false);
  });
});

// --- the mock and the relay adapter -----------------------------------------------------------------------------------------

const made: MockAdapter[] = [];
afterEach(() => {
  for (const a of made.splice(0)) a.mock.dispose();
  vi.unstubAllGlobals();
  resetAdapterForTests();
});

function make(opts: Parameters<typeof createMockAdapter>[0] = {}) {
  const clock = new FakeClock(T0);
  const a = createMockAdapter({ scenario: "healthy", clock, tokenDelayMs: 0, ...opts });
  made.push(a);
  const frames: EventsFrameWithId[] = [];
  a.events({ onFrame: (f) => frames.push(f), onState: () => undefined });
  return { a, r: a.mock.relay, ctl: a.mock.relayControl, clock, frames };
}
const fail = async (p: Promise<unknown>): Promise<ApiError> => {
  const e = await p.then(() => null, (x: unknown) => x);
  expect(e).toBeInstanceOf(ApiError);
  return e as ApiError;
};

describe("the Layer 7 mock answers the contract", () => {
  it("every route's answer validates, and the status carries no secret", async () => {
    const { r, clock } = make();
    expect(validateAgainst("RelayStatus", await r.status())).toEqual([]);
    await r.patchSettings({ carrier: { token: "tk_secret", topic_secret: "x".repeat(52) } }).catch(() => undefined);
    const bundle = await r.pairOffer({ role: "peer", via: "tcp" });
    expect(validateAgainst("PairOffer", bundle)).toEqual([]);
    await r.browseRegistry();
    await clock.advanceAsync(1000, 100);
    expect(validateAgainst("RegistryRow", (await r.registry())[0])).toEqual([]);
    for (const o of await r.updateOrigins()) expect(validateAgainst("UpdateOrigin", o)).toEqual([]);
    expect(JSON.stringify(await r.status())).not.toContain("tk_secret");
  });

  it("a carrier turns on only through the consent round-trip, and a stale hash is refused again", async () => {
    const { r } = make();
    const patch = { carrier: { kind: "ntfy" as const, server: "https://ntfy.example.org", mode: "5A" as const, token: "tk_secret" } };
    const first = await fail(r.patchSettings(patch));
    expect([first.status, first.code]).toEqual([409, "consent_required"]);
    const consent = relayConsentOf(first)!;
    expect(consent.action).toBe("carrier_enable");
    expect(consent.subject).toBe("ntfy:https://ntfy.example.org");
    expect(consent.warning_text).toBe(carrierConsentText("https://ntfy.example.org", "the other hub"));
    expect(await fail(r.patchSettings(patch, "0".repeat(64)))).toMatchObject({ code: "consent_required" });
    const after = await r.patchSettings(patch, consent.hash);
    expect(after.carrier).toMatchObject({ kind: "ntfy", token: { set: true }, topic_secret: { set: false }, consent_hash: consent.hash });
    expect(JSON.stringify(after)).not.toContain("tk_secret");
  });

  it("raising a metered cap while the carrier is on asks again, with `metered` in the add-ons", async () => {
    const { r } = make();
    const base = { carrier: { kind: "ntfy" as const, server: "https://ntfy.example.org" } };
    await r.patchSettings(base, relayConsentOf(await fail(r.patchSettings(base)))!.hash);
    const capped = { metered: [{ match: { ssid: "Home" }, daily_cap_bytes: 5000 }] };
    const c = relayConsentOf(await fail(r.patchSettings(capped)))!;
    expect(c.addons).toEqual(["metered"]);
    expect(c.warning_text).toContain("5000 bytes");
    expect((await r.patchSettings(capped, c.hash)).metered).toHaveLength(1);
  });

  it("gateway_bit and origin_serve are consent round-trips; origin_serve and the gateway need a hub", async () => {
    const { r } = make();
    expect(relayConsentOf(await fail(r.patchSettings({ discovery: { public_gateway_bit: true } })))).toMatchObject({ action: "gateway_bit", warning_text: GATEWAY_BIT_TEXT });
    expect(relayConsentOf(await fail(r.patchSettings({ origin: { serve: true } })))).toMatchObject({ action: "origin_serve", warning_text: ORIGIN_SERVE_TEXT });
  });

  it("a node without the hub role answers needs_hub (gateway, origin, pairing); without LAN sharing, lan_off; without Reticulum, component_unavailable", () => {
    const deps = { now: () => T0, later: () => undefined, publish: () => 0, loopback: () => true };
    const bare = createRelayMock(deps, { hub: false, lan: false, reticulum: false });
    const code = (fn: () => unknown): string => {
      try {
        fn();
      } catch (e) {
        return (e as ApiError).code;
      }
      return "no error";
    };
    expect(code(() => bare.patchSettings({ gateway: { enabled: true } }))).toBe("needs_hub");
    expect(code(() => bare.patchSettings({ origin: { serve: true } }))).toBe("needs_hub");
    expect(code(() => bare.pairOffer({ role: "peer", via: "tcp" }))).toBe("needs_hub");
    expect(code(() => bare.pair({ bundle: "111PAIR:x", role: "peer", via: "tcp" }))).toBe("needs_hub");
    expect(code(() => bare.browseRegistry())).toBe("lan_off");
    expect(code(() => bare.nearbyHops())).toBe("component_unavailable");
    const hubNoLan = createRelayMock(deps, { lan: false });
    expect(code(() => hubNoLan.patchSettings({ origin: { serve: true } }))).toBe("lan_off");
  });

  it("a heartbeat write is refused in v1: no_such_transport, settings_locked for rns; nothing is written", async () => {
    const { r } = make();
    const before = await r.status();
    expect(await fail(r.patchSettings({ presence: { heartbeat: [{ transport: "tcp-lan", interval_min: 5 }] } }))).toMatchObject({ status: 409, code: "no_such_transport" });
    const locked = await fail(r.patchSettings({ presence: { heartbeat: [{ transport: "rns", interval_min: 5 }] } }));
    expect(locked.code).toBe("settings_locked");
    expect(locked.problem.detail).toContain("PATCH /api/v1/bridge/settings");
    expect(await r.status()).toEqual(before);
  });

  it("pairing: the complement rule, the plain warning, lan, and the loopback gate", async () => {
    const { a, r, clock } = make();
    const offer = (extra = ""): string => `111PAIR:${"c".repeat(224)}:K7M29X4P:${"s".repeat(71)}:${extra}:peer:tcp`;
    expect(await fail(r.pair({ bundle: offer("192.168.1.31:11183"), role: "leaf", via: "tcp" }))).toMatchObject({ status: 409, code: "pair_role_mismatch" });
    expect((await r.status()).paired_hubs).toEqual([]);
    const warn = relayConsentOf(await fail(r.pair({ bundle: offer("203.0.113.7:11183"), role: "peer", via: "tcp" })))!;
    expect(warn).toMatchObject({ action: "pair_plain", warning_text: PAIR_PLAIN_TEXT });
    const hub = await r.pair({ bundle: offer("203.0.113.7:11183"), role: "peer", via: "tcp", accept_warning: warn.hash });
    expect(validateAgainst("PairedHub", hub)).toEqual([]);
    expect(hub).toMatchObject({ tls: "plain", lan: false, reachable: false });
    await clock.advanceAsync(2000, 100);
    expect((await r.status()).paired_hubs[0]!.reachable).toBe(true);
    // a LAN mark pairs a non-private address without the warning
    const other = offer("198.51.100.9:11183").replace("c".repeat(224), "d".repeat(224));
    expect(await r.pair({ bundle: other, role: "peer", via: "tcp", lan: true })).toMatchObject({ lan: true, tls: "none" });
    await r.unpair(hub.node_id);
    await r.unpair(hub.node_id); // idempotent
    expect((await r.status()).paired_hubs).toHaveLength(1);
    // loopback only
    const state = a.mock.state;
    state.auth.loopback = false;
    expect(await fail(r.pair({ bundle: offer("192.168.1.9:11183"), role: "peer", via: "tcp" }))).toMatchObject({ status: 403, code: "loopback_only" });
    expect(await fail(r.pairOffer({ role: "peer", via: "tcp" }))).toMatchObject({ code: "loopback_only" });
    expect(await fail(r.unpair(NODE))).toMatchObject({ code: "loopback_only" });
  });

  it("a carrier pair is peer-only and a leaf or gateway pair is tcp-only", async () => {
    const { r } = make();
    expect(await fail(r.pairOffer({ role: "leaf", via: "carrier" }))).toMatchObject({ code: "pair_role_mismatch" });
    expect(await fail(r.pairOffer({ role: "gateway", via: "carrier" }))).toMatchObject({ code: "pair_role_mismatch" });
    expect(await r.pairOffer({ role: "peer", via: "carrier" })).toMatchObject({ schema: 1 });
  });

  it("check and flush: no carrier, no consent, the metered cap, then the frames", async () => {
    const { r, ctl, clock, frames } = make();
    expect(await fail(r.checkCarrier())).toMatchObject({ code: "bad_request" });
    const patch = { carrier: { kind: "ntfy" as const, server: "https://ntfy.example.org" }, gateway: { enabled: true } };
    await r.patchSettings(patch, relayConsentOf(await fail(r.patchSettings(patch)))!.hash);
    ctl.setMeteredCapReached(true);
    expect(await fail(r.flushCarrier())).toMatchObject({ status: 409, code: "metered_cap" });
    ctl.setMeteredCapReached(false);
    expect(await r.checkCarrier()).toEqual({ schema: 1, started: true });
    await clock.advanceAsync(1000, 100);
    const status = await r.status();
    expect(status.gateway).toMatchObject({ state: "carrier_ok", carrier: "ntfy", endpoint: "ntfy.example.org" });
    expect(frames.filter((f) => f.event === "relay").length).toBeGreaterThan(0);
  });

  it("the registry browse answers 202 and then a `registry` frame; the hop read is rate limited", async () => {
    const { r, clock, frames } = make();
    expect(await r.browseRegistry()).toEqual({ schema: 1, started: true });
    await clock.advanceAsync(1000, 100);
    const f = frames.find((x) => x.event === "registry");
    expect(f && validateAgainst("SseRegistry", f.data)).toEqual([]);
    expect(await r.nearbyHops()).toEqual({ schema: 1, started: true });
    expect(await fail(r.nearbyHops())).toMatchObject({ status: 429, code: "rate_limited" });
    await clock.advanceAsync(31_000, 1000);
    expect(await r.nearbyHops()).toEqual({ schema: 1, started: true });
  });

  it("every frame the relay mock publishes validates against its x-sse-events schema", async () => {
    const { r, ctl, clock, frames } = make();
    await r.browseRegistry();
    ctl.setPending(3);
    ctl.bumpForward({ refused: 1, too_large: 2 });
    const patch = { discovery: { lan_record: true } };
    await r.patchSettings(patch);
    await clock.advanceAsync(1500, 100);
    const l7 = frames.filter((f) => f.event === "relay" || f.event === "registry");
    expect(l7.length).toBeGreaterThanOrEqual(4);
    for (const f of l7) expect(validateAgainst(sse.events[f.event]!, f.data), f.event).toEqual([]);
  });

  it("getRelay() hands a mock page the mock's relay and a real page the HTTP adapter, importing ./relay lazily", async () => {
    resetAdapterForTests();
    const fetchFn = vi.fn(async () => new Response(JSON.stringify({ schema: 1, rows: [] }), { status: 200, headers: { "content-type": "application/json" } }));
    vi.stubGlobal("fetch", fetchFn);
    vi.stubGlobal("location", { search: "" });
    const relay: RelayAdapter = await getRelay();
    expect(relay.kind).toBe("http");
    expect(await relay.registry()).toEqual([]);
    expect(fetchFn).toHaveBeenCalledTimes(1);
    expect(getRelay()).toBe(getRelay()); // memoized
  });
});

// --- the store -----------------------------------------------------------------------------------------------------------------

describe("the store handles `relay` and `registry`", () => {
  function storeWith() {
    const fx = buildFixtures(T0);
    const handlers: { current: Parameters<Adapter["events"]>[0] | null } = { current: null };
    const adapter = {
      kind: "http",
      status: async () => structuredClone(fx.status),
      services: async () => structuredClone(fx.services),
      devices: async () => structuredClone(fx.devices),
      proposals: async () => [],
      events: (hd: Parameters<Adapter["events"]>[0]) => {
        handlers.current = hd;
        return () => undefined;
      },
    } as unknown as Adapter;
    const relayCalls = { status: 0, registry: 0 };
    let status = STATUS;
    const relay = {
      kind: "http",
      status: vi.fn(async () => (relayCalls.status++, structuredClone(status))),
      registry: vi.fn(async () => (relayCalls.registry++, [ROW])),
    } as unknown as RelayAdapter;
    const load = vi.fn(async () => relay);
    const store = new ApiStore({ now: () => T0, storage: null });
    return { store, adapter, handlers, relayCalls, load, setStatus: (s: RelayStatus) => (status = s) };
  }
  const frame = (event: string, data: unknown) => ({ event, data, id: "1" }) as unknown as EventsFrameWithId;

  it("a `relay` frame replaces the snapshot (and keeps the object when nothing changed); a `registry` frame replaces the rows", async () => {
    const { store, adapter } = storeWith();
    await store.attach(adapter);
    expect(store.getState().relay).toBeNull();
    store.applyFrame(frame("relay", STATUS));
    const first = store.getState().relay;
    expect(first).toEqual(STATUS);
    store.applyFrame(frame("relay", structuredClone(STATUS)));
    expect(store.getState().relay).toBe(first);
    store.applyFrame(frame("registry", { rows: [ROW] }));
    expect(store.getState().registry).toEqual([ROW]);
    store.applyFrame(frame("registry", { rows: "nope" }));
    expect(store.getState().registry).toEqual([ROW]); // a malformed frame changes nothing
    store.applyFrame(frame("relay", { gateway: null }));
    expect(store.getState().relay).toBe(first);
    store.detach();
  });

  it("after `reset` the loaded Layer 7 slices are refetched, and a node that never loaded them asks nothing", async () => {
    const quiet = storeWith();
    await quiet.store.attach(quiet.adapter);
    quiet.store.attachRelay(quiet.load);
    quiet.store.applyFrame(frame("reset", { reason: "restart" }));
    await quiet.store.refreshAll();
    await flush();
    expect(quiet.load).not.toHaveBeenCalled();
    quiet.store.detach();

    const { store, adapter, relayCalls, load, setStatus } = storeWith();
    await store.attach(adapter);
    store.attachRelay(load);
    await flush();
    store.applyFrame(frame("relay", { ...STATUS, forward: { refused: 9, too_large: 9 } }));
    store.applyFrame(frame("registry", { rows: [] }));
    setStatus({ ...STATUS, forward: { refused: 1, too_large: 0 } });
    store.applyFrame(frame("reset", { reason: "restart" })); // the first reset is the snapshot the attach already fetched
    await store.refreshAll();
    await flush();
    expect(relayCalls).toEqual({ status: 1, registry: 1 });
    expect(store.getState().relay?.forward).toEqual({ refused: 1, too_large: 0 });
    expect(store.getState().registry).toEqual([ROW]);
    store.detach();
  });

  it("a failing Layer 7 read keeps the previous snapshot (an older node answers 404)", async () => {
    const { store, adapter, load } = storeWith();
    await store.attach(adapter);
    store.attachRelay(async () => ({ ...(await load()), status: async () => Promise.reject(new ApiError({ type: "about:blank", title: "x", status: 404, code: "not_found" })) }) as RelayAdapter);
    store.applyFrame(frame("relay", STATUS));
    await store.refreshRelay();
    expect(store.getState().relay).toEqual(STATUS);
    store.detach();
  });

  it("reset() clears the slices; detach() forgets the loader", async () => {
    const { store, adapter, load, relayCalls } = storeWith();
    await store.attach(adapter);
    store.attachRelay(load);
    store.applyFrame(frame("relay", STATUS));
    store.reset();
    expect(store.getState().relay).toBeNull();
    expect(store.getState().registry).toBeNull();
    store.detach();
    await store.refreshRelay();
    expect(relayCalls.status).toBe(0);
  });
});

// --- Nearby's `111` source, the proposal card ---------------------------------------------------------------------------------------

describe("Nearby's 111 source helpers", () => {
  it("recognises the source and keeps only evidence with a transport in the closed set, as cleaned text", () => {
    expect(isOneOneOneSource({ kind: "111" })).toBe(true);
    expect(isOneOneOneSource({ kind: "reticulum" })).toBe(false);
    const node = NEARBY_111.nodes[0]!;
    expect(nodeEvidence(node as never)).toEqual([{ transport: "tcp-lan", text: "a message from them arrived (verified)" }]);
    const dirty = { evidence: [{ transport: "wifi", text: "x" }, { transport: "hub", text: "heard‮ through\u0000 your hub" }, { transport: "carrier", text: "   " }, null] } as never;
    const out = nodeEvidence(dirty);
    expect(out).toHaveLength(1);
    expect(out[0]!.transport).toBe("hub");
    expect(out[0]!.text).not.toMatch(/[‮\u0000]/);
    expect(nodeEvidence({} as never)).toEqual([]);
  });
  it("ages: the page adds its own clock to heard_ago_s and never invents one", () => {
    expect(heardAgoNow({ heard_ago_s: 95 }, 10_000, 40_000)).toBe(125);
    expect(heardAgoNow({ heard_ago_s: 95 }, 10_000, 5_000)).toBe(95);
    expect(heardAgoNow({ heard_ago_s: null }, 0, 1e9)).toBeNull();
    expect(heardAgoNow({}, 0, 1e9)).toBeNull();
    expect([5, 30, 120, 7200, 200000].map(ageLabel)).toEqual(["just now", "30 s ago", "2 min ago", "2 h ago", "2 d ago"]);
  });
  it("private addresses follow §4.1 rule 1", () => {
    for (const a of ["10.0.0.1:11183", "192.168.1.20:11183", "172.16.0.1:1", "100.64.0.1:1", "169.254.1.1:1", "[fd12::1]:11183", "[fe80::1]:1"]) expect(isPrivateAddress(a), a).toBe(true);
    for (const a of ["203.0.113.7:11183", "172.32.0.1:1", "100.128.0.1:1", "hub.example.org:11183", "[2001:db8::1]:11183"]) expect(isPrivateAddress(a), a).toBe(false);
  });
});

describe("the `relay` proposal card", () => {
  const BODY = JSON.stringify({ why: "Your carrier is set up; flush it.", actions: [{ action: "gateway.enable", carrier: "ntfy", mode: "5A" }, { action: "hub.pair" }, { action: "presence.interval", transport: "tcp-lan", minutes: 5 }, { action: "carrier.flush" }] });
  it("parses the closed JSON shape and nothing else", () => {
    expect(parseRelayBody(BODY)?.actions).toHaveLength(4);
    for (const bad of ["not json", "[]", '{"why":1,"actions":[]}', '{"why":"w","actions":[{"nope":1}]}', '{"why":"w"}']) expect(parseRelayBody(bad), bad).toBeNull();
  });
  it("renders a card per action in the spec's words, and flags presence.interval as unavailable", () => {
    const out = renderToStaticMarkup(h(ProposalBody, { body: BODY, kind: "relay" }));
    expect(out).toContain('data-testid="relay-card"');
    expect(out.match(/data-testid="relay-action"/g)).toHaveLength(4);
    expect(out).toContain("turn on the gateway through ntfy (mode 5A)");
    expect(out).toContain("open the pairing screen");
    expect(out).toContain("Not available in this version");
    expect(relayActionSentence({ action: "carrier.flush" }).title).toContain("carrier");
    expect(relayActionSentence({ action: "format.disk" }).title).toContain("unknown action");
  });
  it("draws model-written strings as text, never as Markdown, links or markup", () => {
    const body = JSON.stringify({ why: "**bold** [x](https://evil.example) <img src=x onerror=alert(1)>", actions: [{ action: "gateway.enable", carrier: "<script>x</script>", mode: "5A" }] });
    const out = renderToStaticMarkup(h(ProposalBody, { body, kind: "relay" }));
    expect(out).not.toContain("<img");
    expect(out).not.toContain("<script");
    expect(out).not.toContain("<a ");
    expect(out).not.toContain("<strong");
    expect(out).toContain("&lt;img");
  });
  it("a malformed body is shown as plain text with a note, not as a card", () => {
    const out = renderToStaticMarkup(h(ProposalBody, { body: "do **this**", kind: "relay" }));
    expect(out).not.toContain("relay-card");
    expect(out).toContain("not in the expected shape");
    expect(out).toContain("do **this**");
  });
});
