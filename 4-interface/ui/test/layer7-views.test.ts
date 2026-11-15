// Layer 7 views (7-relay/README.md sections 3.5, 3.8, 5.2, 5.6, 5.8, 12, 13.1; task U3): the gateway badge's four states (and never "online")
// on Overview, on the `gateway` service row and in the fabric view, the metered line, the fabric view's paired hubs and forward counts,
// the `forwarding` copy in Messages, and the mock fixture's gateway row validated against the contract. Driven through the real store
// against the mock adapter's `healthy` scenario, with the relay slice filled by a `relay` frame the way the stream would.
import { createElement as h, type EffectCallback, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import { buildFixtures, GATEWAY_CARRIER_OK, GATEWAY_CARRIER_PENDING } from "../src/lib/api/mock/fixtures";
import { createMockAdapter, SCENARIO_NAMES, type MockAdapter, type ScenarioName } from "../src/lib/api/mock";
import { apiStore } from "../src/lib/api/store";
import { RELAY_GATEWAY_STATES, type PairedHub, type RelayStatus, type ServiceRecord } from "../src/lib/api/types";
import { ApiError } from "../src/lib/api/problem";
import { ModeProvider, resetModeForTests } from "../src/lib/mode";
import { evidenceName, reasonText as messageReason, REASON_COPY, statusLine } from "../src/views/messages/model";
import OverviewView from "../src/views/overview";
import { nodeFacts } from "../src/views/overview/model";
import ServicesView from "../src/views/services";
import { PairedHubRow, RelayPanels } from "../src/views/services/fabric";
import { CARRIER_MAX_ITEM_BYTES, groupDigits, pairFacts, pairLinkState, refusedText, relayBadge, tooLargeText } from "../src/views/services/fabric-model";
import { ageAgoText, flattenDetail, gatewayBadge, METERED_CAP_TEXT, reasonText } from "../src/views/services/model";
import { FakeClock, flush } from "./helpers/fakes";
import { routed } from "./helpers/routed";
import { validateAgainst } from "./helpers/validate";

// Effects do not run in a static render. While `effects.capture` is on, `useEffect` records its callback so a test can run the one it is
// about (Overview's one-shot read of the relay slice); otherwise it is the real hook.
const effects = vi.hoisted(() => ({ capture: false, fns: [] as Array<() => unknown> }));
vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  const base = ((actual as { default?: object }).default ?? actual) as typeof import("react");
  return { ...base, default: base, useEffect: (fn: EffectCallback, deps?: unknown[]) => (effects.capture ? void effects.fns.push(fn) : base.useEffect(fn, deps)) };
});

const NOW = Date.parse("2026-11-11T09:00:00Z");
const NODE = "ab".repeat(32);
const NODE2 = "cd".repeat(32);

const made: MockAdapter[] = [];
afterEach(() => {
  apiStore.detach();
  apiStore.reset();
  for (const a of made.splice(0)) a.mock.dispose();
});

async function load(scenario: ScenarioName = "healthy"): Promise<MockAdapter> {
  const a = createMockAdapter({ scenario, clock: new FakeClock(NOW), tokenDelayMs: 0 });
  made.push(a);
  apiStore.attach(a);
  await flush(30);
  return a;
}

const html = (el: ReactElement, search = ""): string => {
  const g = globalThis as { window?: unknown };
  const had = g.window;
  g.window = {
    location: { search, hash: "", pathname: "/", protocol: "http:", hostname: "127.0.0.1", port: "11180" },
    history: { state: null, replaceState: () => undefined },
    localStorage: { getItem: () => null, setItem: () => undefined },
  };
  resetModeForTests();
  try {
    return renderToStaticMarkup(h(ModeProvider, null, el));
  } finally {
    if (had === undefined) delete g.window;
    else g.window = had;
    resetModeForTests();
  }
};

const pair = (over: Partial<PairedHub> = {}): PairedHub => ({
  node_id: NODE, label: "abcd-efgh-ij", role: "peer", via: "tcp", tls: "rpk", lan: false, reachable: true, pending: 2, reposts: 0, ...over,
});

const STATUS = (over: Partial<RelayStatus> = {}): RelayStatus => ({
  schema: 1,
  gateway: { enabled: true, state: "carrier_ok", carrier: "ntfy", endpoint: "ntfy.example.org", mode: "5A", checked_ago_s: 240, last_flush_ago_s: 240, pending: 0, metered_cap_reached: false },
  carrier: { kind: "ntfy", server: "https://ntfy.example.org", mode: "5A", token: { set: true }, topic_secret: { set: false }, consent_hash: "a".repeat(64) },
  paired_hubs: [pair()],
  forward: { refused: 0, too_large: 0 },
  presence: { heartbeat: [] },
  discovery: { lan_record: false, public_gateway_bit: false, offload_bit: false },
  origin: { serve: false, targets: ["x86_64-unknown-linux-musl"], bytes: 0 },
  updates: { check_internet: true },
  metered: [],
  ...over,
});

/** The relay slice arrives as a `relay` frame (a full RelayStatus) the way the stream sends it. */
const relayFrame = (status: RelayStatus): void => apiStore.applyFrame({ id: "r1", event: "relay", data: status } as never);

/** The `gateway` service row with a code and detail, replaced in the loaded list the way a `services` frame would. */
function setGateway(reason_code: string, detail: Record<string, unknown>, state: ServiceRecord["state"] = "unset"): void {
  const services = apiStore.getState().services!.map((s) => (s.id === "gateway" ? { ...s, state, reason_code, detail } : s));
  apiStore.applyFrame({ id: "s1", event: "services", data: { services } } as never);
}

const fabricPage = () => html(routed("/services/fabric", "/services/:id", ServicesView));
const gatewayPage = () => html(routed("/services/gateway", "/services/:id", ServicesView));
const overview = () => html(routed("/overview", "/overview", OverviewView), "?mode=advanced");

// ---------------------------------------------------------------------------------------------- the badge

describe("the gateway badge: four states, never online (section 3.8)", () => {
  const D = { carrier: "ntfy", endpoint: "ntfy.example.org", mode: "5A", checked_ago_s: 240, last_flush_ago_s: 600, pending: 0, error: null };

  it("each state has its own words", () => {
    expect(gatewayBadge("gateway_none", { ...D, carrier: null })).toBe("Not a gateway");
    expect(gatewayBadge("carrier_ok", D)).toBe("Gateway: carrier reachable at last check, 4 min ago");
    expect(gatewayBadge("carrier_pending", { ...D, pending: 3 })).toBe("Gateway: last flush 10 min ago, 3 waiting");
    expect(gatewayBadge("gateway_off", { ...D, carrier: null })).toBe("Gateway off (by you)");
  });

  it("carrier_pending says what it waits on: items, or a check the carrier did not answer", () => {
    expect(gatewayBadge("carrier_pending", { ...D, pending: 1, last_flush_ago_s: null })).toBe("Gateway: last flush not yet, 1 waiting");
    expect(gatewayBadge("carrier_pending", { ...D, pending: 0 })).toBe("Gateway: the carrier did not answer at the last check, 4 min ago");
    expect(gatewayBadge("carrier_pending", { pending: 0, checked_ago_s: null })).toBe("Gateway: the carrier did not answer at the last check");
  });

  it("covers every state of the contract, and none says online or internet", () => {
    for (const code of RELAY_GATEWAY_STATES) {
      const text = gatewayBadge(code, { ...D, pending: 2 });
      expect(text, code).not.toBe("unknown");
      expect(text, code).not.toMatch(/online|internet/i);
    }
  });

  it("a retired or unknown code reads unknown; a missing or hostile detail cannot break it", () => {
    for (const code of ["wan_ok", "wan_offline", "wan_unknown", "toString", "", null, undefined]) expect(gatewayBadge(code, D), String(code)).toBe("unknown");
    expect(gatewayBadge("carrier_ok", null)).toBe("Gateway: carrier reachable at last check");
    expect(gatewayBadge("carrier_ok", { checked_ago_s: -5 })).toBe("Gateway: carrier reachable at last check");
    expect(gatewayBadge("carrier_ok", { checked_ago_s: "soon" })).toBe("Gateway: carrier reachable at last check");
    expect(gatewayBadge("carrier_pending", { pending: Number.NaN })).toBe("Gateway: the carrier did not answer at the last check");
  });

  it("ages are monotonic seconds, worded as ages", () => {
    expect(ageAgoText(3)).toBe("just now");
    expect(ageAgoText(45)).toBe("45 s ago");
    expect(ageAgoText(240)).toBe("4 min ago");
    expect(ageAgoText(null)).toBeNull();
    expect(ageAgoText(Number.POSITIVE_INFINITY)).toBeNull();
  });

  it("the gateway row's reason and detail table use the badge and the ages, not the retired probe words", () => {
    const row = { ...buildFixtures(NOW).services.find((s) => s.id === "gateway")!, ...structuredClone(GATEWAY_CARRIER_OK) } as ServiceRecord;
    expect(reasonText(row)).toBe("Gateway: carrier reachable at last check, 4 min ago");
    expect(reasonText({ ...row, reason_code: "gateway_off", state: "unset" })).toBe("Gateway off (by you)");
    expect(reasonText({ ...row, ...structuredClone(GATEWAY_CARRIER_PENDING) } as ServiceRecord)).toBe("Gateway: last flush 5 min ago, 3 waiting");
    // a gateway code on another service id keeps the node's own sentence
    expect(reasonText({ ...row, id: "kb" })).toBe(row.reason);
    expect(flattenDetail(row.detail, NOW)).toEqual([
      { label: "Carrier", value: "ntfy" },
      { label: "Endpoint", value: "ntfy.example.org" },
      { label: "Mode", value: "5A" },
      { label: "Last check", value: "4 min ago" },
      { label: "Last flush", value: "4 min ago" },
      { label: "Pending", value: "0" },
    ]);
  });

  it("a code outside the four reads unknown on the gateway row, never the node's own sentence (an older node's wan_*)", () => {
    const row = buildFixtures(NOW).services.find((s) => s.id === "gateway")!;
    for (const code of ["wan_ok", "wan_offline", "wan_unknown", "toString", ""]) {
      expect(reasonText({ ...row, reason_code: code, reason: "This node can reach the internet." }), code).toBe("unknown");
    }
    expect(reasonText({ ...row, id: "kb", reason_code: "wan_ok", reason: "The node's sentence." })).toBe("The node's sentence.");
  });
});

// ----------------------------------------------------------------------------------------- the fixture

describe("the mock fixture's gateway row (section 3.5 item 4, S18)", () => {
  const fx = buildFixtures(NOW);
  const row = fx.services.find((s) => s.id === "gateway")!;

  it("validates as a ServiceRecord and its detail as GatewayDetail", () => {
    expect(validateAgainst("ServiceRecord", row)).toEqual([]);
    expect(validateAgainst("GatewayDetail", row.detail)).toEqual([]);
    for (const s of fx.services) expect(validateAgainst("ServiceRecord", s), s.id).toEqual([]);
  });

  it("the healthy node is a member: gateway_none, state unset, every fact null, nothing pending (the badge is the hub's own, section 3.8)", () => {
    expect(row).toMatchObject({ state: "unset", reason_code: "gateway_none", reason: "Not a gateway" });
    expect(row.detail).toEqual({ carrier: null, endpoint: null, mode: null, checked_ago_s: null, last_flush_ago_s: null, pending: 0, error: null });
    expect(fx.messaging.fabric.settings.hub_role).toBe(false);
  });

  it("carries one of the four codes and none of the retired probe keys", () => {
    expect(RELAY_GATEWAY_STATES as readonly string[]).toContain(row.reason_code);
    for (const gone of ["wan_ok", "wan_offline", "wan_unknown"]) expect(row.reason_code).not.toBe(gone);
    for (const key of ["wan_ok", "interval_sec", "last_probe", "probe", "online", "probe_url", "checked_at"]) expect(Object.keys(row.detail)).not.toContain(key);
    expect(row.description).not.toMatch(/once a minute|reach the internet/i);
    expect(JSON.stringify(row)).not.toMatch(/online/i);
  });

  it("the mock's whole status still validates", () => {
    expect(validateAgainst("Status", fx.status)).toEqual([]);
  });

  it("only a hub scenario carries a gateway: carrier_ok (and, later, carrier_pending) with hub_role true; every other scenario is gateway_none", () => {
    for (const name of SCENARIO_NAMES) {
      const a = createMockAdapter({ scenario: name, clock: new FakeClock(NOW), tokenDelayMs: 0 });
      made.push(a);
      const data = a.mock.scenario.data;
      const gw = data.services.find((s) => s.id === "gateway")!;
      expect(validateAgainst("ServiceRecord", gw), name).toEqual([]);
      expect(validateAgainst("GatewayDetail", gw.detail), name).toEqual([]);
      if (name === "hub") {
        expect(data.messaging.fabric.settings.hub_role).toBe(true);
        expect(gw).toMatchObject({ state: "live", reason_code: "carrier_ok" });
        expect(gatewayBadge(gw.reason_code, gw.detail)).toBe(gw.reason);
        expect(gatewayBadge(GATEWAY_CARRIER_PENDING.reason_code, GATEWAY_CARRIER_PENDING.detail)).toBe(GATEWAY_CARRIER_PENDING.reason);
        expect(validateAgainst("GatewayDetail", GATEWAY_CARRIER_PENDING.detail)).toEqual([]);
      } else {
        expect(gw.reason_code, name).toBe("gateway_none");
        expect(gw.state, name).toBe("unset");
      }
    }
  });
});

// --------------------------------------------------------------------------------------------- Overview

describe("Overview shows the badge from the gateway service row, and the metered line from the relay slice", () => {
  it("the row's reason_code picks the badge; every state, never online", async () => {
    await load();
    const want: Array<[string, Record<string, unknown>, string]> = [
      ["gateway_none", { carrier: null, endpoint: null, mode: null, checked_ago_s: null, last_flush_ago_s: null, pending: 0, error: null }, "Not a gateway"],
      ["carrier_ok", { carrier: "ntfy", endpoint: "ntfy.example.org", mode: "5A", checked_ago_s: 120, last_flush_ago_s: 120, pending: 0, error: null }, "Gateway: carrier reachable at last check, 2 min ago"],
      ["carrier_pending", { carrier: "ntfy", endpoint: "ntfy.example.org", mode: "5A", checked_ago_s: 120, last_flush_ago_s: 300, pending: 4, error: null }, "Gateway: last flush 5 min ago, 4 waiting"],
      ["gateway_off", { carrier: null, endpoint: null, mode: null, checked_ago_s: null, last_flush_ago_s: null, pending: 0, error: null }, "Gateway off (by you)"],
    ];
    for (const [code, detail, text] of want) {
      setGateway(code, detail, code === "carrier_ok" ? "live" : "unset");
      const out = overview();
      expect(out, code).toContain('data-testid="gateway-badge"');
      expect(out, code).toContain(`<span>${text}</span>`);
      expect(out, code).not.toMatch(/>online</i);
      expect(nodeFacts(apiStore.getState().status!, apiStore.getState().services, NOW).gateway, code).toBe(text);
    }
  });

  it("an older node's retired code reads unknown, not a claim", async () => {
    await load();
    setGateway("wan_ok", { probe: "204" }, "live");
    expect(overview()).toContain("<span>unknown</span>");
  });

  it("a relay slice with no gateway object does not throw and shows no metered line", async () => {
    await load("hub");
    relayFrame({ ...STATUS(), gateway: undefined } as never);
    let out = "";
    expect(() => (out = overview())).not.toThrow();
    expect(out).not.toContain(METERED_CAP_TEXT);
  });

  it("the metered line shows beside the badge only when the relay slice says the cap is reached", async () => {
    await load("hub");
    expect(apiStore.getState().relay).toBeNull();
    expect(overview()).not.toContain(METERED_CAP_TEXT);
    relayFrame(STATUS());
    expect(overview()).not.toContain(METERED_CAP_TEXT);
    relayFrame(STATUS({ gateway: { ...STATUS().gateway, metered_cap_reached: true, state: "carrier_pending", pending: 3 } }));
    const out = overview();
    expect(out).toContain(METERED_CAP_TEXT);
    expect(out).toMatch(/metered cap reached/i);
  });

  /** Run the mount effects of an Overview render (a static render skips them) that touch the relay slice, then let the store settle. */
  async function mountOverview(): Promise<void> {
    effects.capture = true;
    try {
      overview();
      for (const fn of effects.fns.splice(0)) if (String(fn).includes("refreshRelay")) fn();
    } finally {
      effects.capture = false;
      effects.fns.length = 0;
    }
    await flush(30);
  }

  /** A relay loader that counts the GETs it serves and answers with `status`. */
  function countingRelay(a: MockAdapter, status: RelayStatus): { gets: () => number } {
    let n = 0;
    apiStore.attachRelay(async () => ({ ...a.mock.relay, status: async () => (n++, structuredClone(status)) }));
    return { gets: () => n };
  }

  const METERED = STATUS({ gateway: { ...STATUS().gateway, metered_cap_reached: true, state: "carrier_pending", pending: 3 } });

  it("a cold load of a hub scenario reads the relay slice once and shows the metered line (the daemon sends no snapshot on connect)", async () => {
    const a = await load("hub");
    const relay = countingRelay(a, METERED);
    expect(apiStore.getState().relay).toBeNull();
    expect(overview()).not.toContain(METERED_CAP_TEXT);
    await mountOverview();
    expect(relay.gets()).toBe(1);
    expect(apiStore.getState().relay?.gateway.metered_cap_reached).toBe(true);
    expect(overview()).toContain(METERED_CAP_TEXT);
  });

  it("carrier_pending reads it too; carrier_ok whose cap is not reached shows no line", async () => {
    const a = await load("hub");
    setGateway("carrier_pending", { ...GATEWAY_CARRIER_PENDING.detail }, "unset");
    const relay = countingRelay(a, STATUS());
    await mountOverview();
    expect(relay.gets()).toBe(1);
    expect(overview()).not.toContain(METERED_CAP_TEXT);
  });

  it("the hub scenario's script moves its gateway row from carrier_ok to carrier_pending", async () => {
    const clock = new FakeClock(NOW);
    const a = createMockAdapter({ scenario: "hub", clock, tokenDelayMs: 0 });
    made.push(a);
    expect(await a.service("gateway")).toMatchObject({ state: "live", reason_code: "carrier_ok" });
    await clock.advanceAsync(11_000, 500);
    expect(await a.service("gateway")).toMatchObject({ state: "unset", reason_code: "carrier_pending", detail: { pending: 3 } });
  });

  it("a member (gateway_none), a gateway turned off, an older node's wan_ok and no services at all send no relay GET", async () => {
    const a = await load("healthy");
    const relay = countingRelay(a, METERED);
    await mountOverview();
    for (const [code, state] of [["gateway_none", "unset"], ["gateway_off", "unset"], ["wan_ok", "live"], ["toString", "unset"]] as const) {
      setGateway(code, { ...GATEWAY_CARRIER_OK.detail, carrier: null }, state);
      await mountOverview();
    }
    expect(relay.gets()).toBe(0);
    expect(apiStore.getState().relay).toBeNull();
    expect(overview()).not.toContain(METERED_CAP_TEXT);
    // before the services have loaded
    apiStore.reset();
    await mountOverview();
    expect(relay.gets()).toBe(0);
  });

  it("a daemon without Layer 7 (the GET answers 404) leaves the slice null and the page as it was", async () => {
    const a = await load("hub");
    apiStore.attachRelay(async () => ({ ...a.mock.relay, status: async () => Promise.reject(new ApiError({ type: "about:blank", title: "Not found", status: 404, code: "not_found" })) }));
    await mountOverview();
    expect(apiStore.getState().relay).toBeNull();
    const out = overview();
    expect(out).toContain('data-testid="gateway-badge"');
    expect(out).not.toContain(METERED_CAP_TEXT);
  });
});

// ---------------------------------------------------------------------------------- the gateway service row

describe("the gateway service row page", () => {
  it("words its reason with the badge and its details as ages", async () => {
    await load("hub");
    const out = gatewayPage();
    expect(out).toContain("Gateway: carrier reachable at last check, 4 min ago");
    expect(out).toContain("Last check");
    expect(out).toContain("4 min ago");
    expect(out).not.toMatch(/wan_ok|interval sec|last probe/i);
  });

  it("an older node's retired code reads unknown on the page, not the node's own internet sentence", async () => {
    await load();
    const services = apiStore.getState().services!.map((x) => (x.id === "gateway" ? { ...x, state: "live" as const, reason_code: "wan_ok", reason: "This node can reach the internet.", detail: { probe: "204" } } : x));
    apiStore.applyFrame({ id: "s2", event: "services", data: { services } } as never);
    const out = gatewayPage();
    expect(out).toContain("unknown");
    expect(out).not.toContain("This node can reach the internet.");
  });
});

// ------------------------------------------------------------------------------------------ fabric view

describe("the fabric view: paired hubs, link state, forward counts (sections 5.2, 5.8, 12)", () => {
  it("the link state is tls rpk, lan, or plain (warned); a carrier pair rides its carrier; an unknown value reads unknown", () => {
    expect(pairLinkState(pair({ tls: "rpk" }))).toBe("tls rpk");
    expect(pairLinkState(pair({ tls: "none", lan: true }))).toBe("lan");
    expect(pairLinkState(pair({ tls: "plain" }))).toBe("plain (warned)");
    expect(pairLinkState(pair({ tls: "none", via: "carrier" }))).toBe("carrier");
    expect(pairLinkState(pair({ tls: "none", lan: false }))).toBe("unknown");
    expect(pairLinkState({ tls: "quantum" as never, lan: false, via: "tcp" })).toBe("unknown");
  });

  it("a pair's facts: role, link, protection, reachable, pending, re-posts", () => {
    expect(pairFacts(pair())).toEqual(["peer hub", "over the network", "tls rpk", "reachable", "2 items waiting"]);
    expect(pairFacts(pair({ reachable: false, pending: 0, tls: "plain" }))).toEqual(["peer hub", "over the network", "plain (warned)", "not reachable", "nothing waiting"]);
    expect(pairFacts(pair({ via: "carrier", tls: "none", pending: 1, reposts: 3 }))).toEqual(["peer hub", "through the carrier", "carrier", "reachable", "1 item waiting", "re-posted 3 times"]);
    expect(pairFacts(pair({ via: "carrier", tls: "none", reposts: 1 }))[5]).toBe("re-posted once");
    expect(pairFacts(pair({ role: "leaf" }))[0]).toBe("leaf (this hub is its gateway)");
    expect(pairFacts(pair({ role: "gateway" }))[0]).toBe("gateway (it is this hub's upstream)");
    expect(pairFacts(pair({ role: "toString" as never }))[0]).toBe("unknown role");
  });

  it("the forward counts: refused is this session's, too large names the 2,937 B bound (never a per-item size)", () => {
    expect(CARRIER_MAX_ITEM_BYTES).toBe(2937);
    expect(groupDigits(2937)).toBe("2,937");
    expect(groupDigits(1_234_567)).toBe("1,234,567");
    expect(groupDigits(Number.NaN)).toBe("0");
    expect(refusedText(1)).toMatch(/^1 item refused by every hub they could go to since this node started\./);
    expect(refusedText(12)).toMatch(/^12 items refused/);
    expect(tooLargeText(2)).toContain("2 items too large for the carrier");
    expect(tooLargeText(2)).toContain("2,937 B");
    expect(tooLargeText(2)).not.toMatch(/found/i);
  });

  it("the badge line comes from the hub's own relay snapshot", () => {
    expect(relayBadge(STATUS())).toBe("Gateway: carrier reachable at last check, 4 min ago");
    expect(relayBadge(STATUS({ gateway: { ...STATUS().gateway, enabled: false, state: "gateway_off", carrier: null, endpoint: null, mode: null } }))).toBe("Gateway off (by you)");
  });

  it("a peer's label is cleaned, isolated and cut at nothing but its own text", () => {
    const out = renderToStaticMarkup(h(PairedHubRow, { pair: pair({ label: "ab‮cd-efgh-ij" }) }));
    expect(out).toContain('data-testid="paired-hub"');
    expect(out).not.toContain("‮");
    expect(out).toContain("[unicode-bidi:isolate]");
    expect(out).toContain("peer hub");
  });

  it("a snapshot missing forward, gateway or paired_hubs draws less and never throws", () => {
    const draw = (relay: unknown, hubRole: boolean): string => renderToStaticMarkup(h(RelayPanels, { relay: relay as RelayStatus, hubRole }));
    const { forward: _f, ...noForward } = STATUS();
    const { gateway: _g, ...noGateway } = STATUS();
    const { paired_hubs: _p, ...noPairs } = STATUS();
    expect(() => draw(noForward, true)).not.toThrow();
    expect(draw(noForward, true)).toContain('data-testid="paired-hub"');
    expect(draw(noForward, true)).not.toContain('data-testid="forward-counts"');
    expect(draw(noGateway, true)).toContain('data-testid="paired-hub"');
    expect(draw(noGateway, true)).toContain("unknown");
    expect(draw(noGateway, true)).not.toContain('data-testid="metered-cap"');
    expect(draw(noPairs, true)).toContain("No hub is paired with this one.");
    // a member with nothing to show draws nothing at all, even from an empty or partial body
    expect(draw({}, false)).toBe("");
    expect(draw({ forward: { refused: "x", too_large: -3 }, gateway: null, paired_hubs: null }, false)).toBe("");
    // a malformed count never prints as a number
    expect(draw({ ...STATUS(), forward: { refused: Number.NaN, too_large: 2 } }, true)).toContain("2 items too large for the carrier");
    expect(draw({ ...STATUS(), forward: { refused: Number.NaN, too_large: 2 } }, true)).not.toContain("refused by every hub");
  });

  it("without the relay slice the fabric page shows no Layer 7 panels (an older node, or not loaded yet)", async () => {
    await load();
    const out = fabricPage();
    expect(out).toContain('data-testid="fabric-details"');
    expect(out).not.toContain("paired hubs");
    expect(out).not.toContain('data-testid="gateway-badge"');
  });

  it("with it: the badge, each pair with its facts, the counts and the bound", async () => {
    await load();
    relayFrame(
      STATUS({
        paired_hubs: [pair(), pair({ node_id: NODE2, label: "wxyz-abcd-ef", via: "carrier", tls: "none", reachable: false, pending: 5, reposts: 2 })],
        forward: { refused: 3, too_large: 4 },
      }),
    );
    const out = fabricPage();
    expect(out).toContain('data-testid="gateway-badge"');
    expect(out).toContain("Gateway: carrier reachable at last check, 4 min ago");
    expect(out.match(/data-testid="paired-hub"/g)).toHaveLength(2);
    expect(out).toContain("abcd-efgh-ij");
    expect(out).toContain("tls rpk");
    expect(out).toContain("2 items waiting");
    expect(out).toContain("not reachable");
    expect(out).toContain("re-posted 2 times");
    expect(out).toContain("3 items refused by every hub they could go to since this node started.");
    expect(out).toContain("4 items too large for the carrier");
    expect(out).toContain("2,937 B");
    expect(out).not.toContain('data-testid="metered-cap"');
  });

  it("the metered line sits beside the badge in the fabric view", async () => {
    await load();
    relayFrame(STATUS({ gateway: { ...STATUS().gateway, metered_cap_reached: true, state: "carrier_pending", pending: 1 } }));
    const out = fabricPage();
    expect(out).toContain('data-testid="metered-cap"');
    expect(out).toContain(METERED_CAP_TEXT);
    expect(out).toContain("Gateway: last flush 4 min ago, 1 waiting");
  });

  it("a hub with nothing paired says so; a node with no hub role, no pair and no counts shows no panels", async () => {
    await load();
    relayFrame(STATUS({ paired_hubs: [], forward: { refused: 0, too_large: 0 } }));
    expect(fabricPage()).not.toContain("No hub is paired with this one.");
    const fabric = apiStore.getState().fabric!;
    apiStore.applyFrame({ id: "f1", event: "fabric", data: { ...fabric, settings: { ...fabric.settings, hub_role: true } } } as never);
    expect(apiStore.getState().fabric!.settings.hub_role).toBe(true);
    const out = fabricPage();
    expect(out).toContain("No hub is paired with this one.");
    expect(out).not.toContain("items refused");
    expect(out).not.toContain("too large for the carrier");
  });
});

// ------------------------------------------------------------------------------------------- Messages

describe("Messages: reason `forwarding` (section 5.6)", () => {
  const SENTENCE = "Your hub holds it and passes it to their hub.";
  const m = { state: "relayed", reason_code: "forwarding", reason: "node sentence" } as const;

  it("a relayed message with reason forwarding reads the UI sentence, not the node's", () => {
    expect(messageReason(m)).toBe(SENTENCE);
    expect(statusLine(m).reason).toBe(SENTENCE);
  });

  it("no copy says the dest's hub was found: not the sentence, the state copy, nor the evidence name", () => {
    const all = [SENTENCE, statusLine(m).detail, evidenceName({ dir: "own", ...m })];
    for (const text of all) expect(text, text).not.toMatch(/\b(found|located|reached|arrived|taken by)\b/i);
    expect(evidenceName({ dir: "own", ...m })).toContain("nothing yet says that hub took it");
    expect(evidenceName({ dir: "own", state: "relayed", reason_code: null })).toBe("CUSTODY_ACK ACCEPTED from a mailbox-role node for a known member");
  });

  it("plain relayed (no hint, no reason) keeps its sentence, and forwarding does not leak to other states", () => {
    expect(messageReason({ state: "relayed", reason_code: null, reason: null })).toBeNull();
    expect(messageReason({ state: "queued", reason_code: "forwarding", reason: "The node's own sentence." })).toBe("The node's own sentence.");
    expect(Object.keys(REASON_COPY)).not.toContain("forwarding");
  });
});
