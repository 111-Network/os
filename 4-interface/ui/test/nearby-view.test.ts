// Nearby (os/5-messaging/README.md section 14, 16): the real list per source, rendered from the mock's `meshtastic` scenario through
// the real store. Names are untrusted RF-origin strings: escaped, cut, never a link, never in a prompt.
import { createElement as h, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMockAdapter, type MockAdapter, type ScenarioName } from "../src/lib/api/mock";
import { apiStore } from "../src/lib/api/store";
import type { NearbyNode, NearbySource } from "../src/lib/api/types";
import { ModeProvider, resetModeForTests } from "../src/lib/mode";
import NearbyView from "../src/views/nearby";
import {
  CLOCK_AHEAD_TEXT,
  clip,
  displayName,
  heardLine,
  heardSummary,
  heardText,
  hopsText,
  nearbyState,
  positionText,
  shortName,
  signalText,
  sortedNodes,
  sourceHeading,
  STALE_TEXT,
  STARTING_STALE_TEXT,
  utilizationText,
} from "../src/views/nearby/model";
import { FakeClock, flush } from "./helpers/fakes";

const NOW = Date.parse("2026-10-14T09:00:00Z");

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
});

const made: MockAdapter[] = [];
afterEach(() => {
  vi.useRealTimers();
  apiStore.detach();
  apiStore.reset();
  for (const a of made.splice(0)) a.mock.dispose();
});

async function load(scenario: ScenarioName): Promise<MockAdapter> {
  const a = createMockAdapter({ scenario, clock: new FakeClock(NOW), tokenDelayMs: 0 });
  made.push(a);
  apiStore.attach(a);
  await flush(30);
  return a;
}

async function loadHeard(): Promise<MockAdapter> {
  const a = await load("meshtastic");
  const board = (await a.devices()).find((d) => d.actions.includes("identify"))!;
  await a.identifyDevice(board.id, { consent: true });
  await flush(30);
  return a;
}

const html = (el: ReactElement, search = ""): string => {
  const g = globalThis as { window?: unknown };
  const had = g.window;
  g.window = { location: { search, hash: "", pathname: "/", protocol: "http:", hostname: "127.0.0.1", port: "11180" }, history: { state: null, replaceState: () => undefined }, localStorage: { getItem: () => null, setItem: () => undefined } };
  resetModeForTests();
  try {
    return renderToStaticMarkup(h(ModeProvider, null, el));
  } finally {
    if (had === undefined) delete g.window;
    else g.window = had;
    resetModeForTests();
  }
};

const node = (over: Partial<NearbyNode> = {}): NearbyNode => ({ id: "!00000001", num: 1, long_name: "Node", short_name: "ND", pubkey: null, snr: null, rssi: null, hops_away: null, last_heard_hint: null, position: null, channel_utilization: null, ...over });
const source = (nodes: NearbyNode[], over: Partial<NearbySource> = {}): NearbySource => ({ id: "meshtastic:usb-1", kind: "meshtastic", label: "heard via Meshtastic on Bench radio", device_id: "usb-1", nodes, ...over });

// --------------------------------------------------------------------------------------------- the list

describe("Nearby from the meshtastic scenario", () => {
  it("lists the nodes heard, grouped under the source's heading, with the figures the board reported", async () => {
    await loadHeard();
    const out = html(h(NearbyView));
    expect(out).toContain("3 Meshtastic nodes heard through 1 board.");
    expect(out).toContain("heard via Meshtastic on Bench radio");
    expect(out).toContain("Ridge repeater");
    expect(out).toContain("RDG1");
    expect(out).toContain("1 hop away");
    expect(out).toContain("SNR 6.5 dB, RSSI -92 dBm");
    expect(out).toContain("7.4% of the channel in use");
    expect(out).toContain("position, as it reports it: 52.3731, 4.8922, 41 m");
    expect(out).toContain("Mara&#x27;s handheld");
    expect(out).toContain("3 hops away");
    expect(out).toContain("direct"); // the hostile-named node is heard directly
  });

  it("last heard reads through agoText, from the node's own clock, and says when it is not reported", async () => {
    await loadHeard();
    const out = html(h(NearbyView));
    expect(out).toContain("last heard 4 min ago");
    expect(out).toContain("last heard 52 min ago");
    expect(out).toContain("last heard not reported");
    expect(out).toContain("node&#x27;s own clock");
    expect(out).toContain("a hint, not proof");
  });

  it("orders nearest first and never by the node-clock hint", async () => {
    await loadHeard();
    const out = html(h(NearbyView));
    const direct = out.indexOf("Free WiFi"); // 0 hops
    const one = out.indexOf("Ridge repeater"); // 1 hop, heard 4 min ago
    const three = out.indexOf("Mara"); // 3 hops, heard 52 min ago
    expect(direct).toBeLessThan(one);
    expect(one).toBeLessThan(three);
  });

  it("a hostile name is escaped, never markup, never a link", async () => {
    await loadHeard();
    const out = html(h(NearbyView));
    expect(out).toContain("&lt;img src=x onerror=alert(1)&gt; Free WiFi");
    expect(out).toContain("&lt;b&gt;");
    expect(out).not.toContain("<img");
    expect(out).not.toContain("<b>");
    expect(out).not.toContain("onerror=alert(1)>"); // the attribute text only ever appears escaped
    // no anchor inside any node row: names, ids and positions are text
    const rows = out.split('data-testid="nearby-node"').slice(1);
    expect(rows).toHaveLength(3);
    for (const row of rows) expect(row.split("</li>")[0]).not.toMatch(/<a[\s>]/);
  });

  it("is not a contact list: no write-to-them action, and it says so", async () => {
    await loadHeard();
    const out = html(h(NearbyView));
    expect(out).not.toMatch(/Message them|Write to them|Add contact/i);
    expect(out).toContain("A Meshtastic node is not a contact");
  });

  it("advanced mode adds the id, number and key as plain text", async () => {
    await loadHeard();
    expect(html(h(NearbyView))).not.toContain("number 2712847316");
    const adv = html(h(NearbyView), "?mode=advanced");
    expect(adv).toContain("id !a1b2c3d4 · number 2712847316");
    expect(adv).toContain("key ");
  });

  it("applies an SSE `nearby` frame: a new node appears, a gone one disappears", async () => {
    await loadHeard();
    apiStore.applyFrame({ event: "nearby", data: { sources: [source([node({ id: "!feedbeef", long_name: "Hilltop", short_name: "HILL", hops_away: 2 })])] } });
    const out = html(h(NearbyView));
    expect(out).toContain("1 Meshtastic node heard through 1 board.");
    expect(out).toContain("Hilltop");
    expect(out).not.toContain("Ridge repeater");
  });

  it("when the node stops answering, the list is marked as the last known one", async () => {
    await loadHeard();
    apiStore.setConnection("closed");
    const out = html(h(NearbyView));
    expect(out).toContain('data-testid="nearby-stale"');
    expect(out).toContain(STALE_TEXT);
    expect(out).toContain("Ridge repeater"); // still shown, never invented, never dropped
  });

  it("the agent question carries no node name: the prompt is built from the node's own status only", async () => {
    await loadHeard();
    const out = html(h(NearbyView));
    expect(out).toContain("Ask the agent who is nearby");
    // the button builds its prompt on click, from `questionPrompt(NEARBY_QUESTION, {kind: node}, ...)`: there is no path from `nearby` into it
    const src = await import("node:fs").then((fs) => fs.readFileSync(new URL("../src/views/nearby/index.tsx", import.meta.url), "utf8"));
    expect(src).toMatch(/questionPrompt\(NEARBY_QUESTION, \{ kind: "node" \}, \{ status \}\)/);
    expect(src).not.toMatch(/dangerouslySetInnerHTML|href=\{[^}]*(name|node\.)/);
  });
});

// ----------------------------------------------------------------------------------------- empty states

describe("Nearby empty states", () => {
  it("no board: says there is nothing to listen with and links to the Meshtastic page and Devices", async () => {
    await load("meshtastic");
    const out = html(h(NearbyView));
    expect(out).toContain("No Meshtastic board");
    expect(out).toContain("nothing to listen with");
    expect(out).toContain('href="#/services/meshtastic"');
    expect(out).toContain('href="#/devices"');
    expect(out).not.toContain('data-testid="nearby-node"');
  });

  it("before the first read it says it is waiting; after a read that failed it says the node did not report", async () => {
    const out = html(h(NearbyView));
    expect(out).toContain("Reading what this node hears");
    expect(nearbyState(null, null, false, true).kind).toBe("unavailable");
    expect(nearbyState(null, null, false, false).kind).toBe("loading");
  });

  it("the service states pick the sentence: unbound, starting, error, listening with nobody heard", () => {
    const svc = (state: "unset" | "starting" | "error" | "live", reason_code: string, reason = "") => ({ state, reason_code, reason });
    expect(nearbyState([], svc("unset", "no_meshtastic"), false).kind).toBe("no_board");
    expect(nearbyState([], svc("unset", "meshtastic_unbound"), false).kind).toBe("unbound");
    expect(nearbyState([], svc("starting", "node_db_pending", "meshmud is connected; waiting for the board's list of nodes."), false)).toMatchObject({ kind: "starting", text: "meshmud is connected; waiting for the board's list of nodes." });
    expect(nearbyState([], svc("error", "uplink_lost", "meshmud lost its connection."), false)).toMatchObject({ kind: "error", text: "meshmud lost its connection." });
    expect(nearbyState([source([])], svc("live", "meshtastic_live"), false).kind).toBe("empty");
    expect(nearbyState([], svc("live", "meshtastic_live"), false).kind).toBe("empty");
    expect(nearbyState([], null, false).kind).toBe("no_board");
  });

  it("nodes win over every service state; an error or a lost connection only marks the list stale", () => {
    const s = [source([node()])];
    expect(nearbyState(s, { state: "live", reason_code: "meshtastic_live", reason: "" }, false)).toEqual({ kind: "list", stale: false, text: null });
    expect(nearbyState(s, { state: "error", reason_code: "child_backoff", reason: "" }, false)).toMatchObject({ kind: "list", stale: true });
    expect(nearbyState(s, { state: "live", reason_code: "meshtastic_live", reason: "" }, true)).toMatchObject({ kind: "list", stale: true, text: STALE_TEXT });
  });

  it("a starting service still serves the last snapshot, so the list is marked last known, not presented as live", () => {
    const s = [source([node()])];
    const starting = { state: "starting" as const, reason_code: "node_db_pending", reason: "meshmud is connected; waiting for the board's list of nodes." };
    expect(nearbyState(s, starting, false)).toEqual({ kind: "list", stale: true, text: STARTING_STALE_TEXT });
    expect(STARTING_STALE_TEXT).toMatch(/Last known list/);
    expect(STARTING_STALE_TEXT).toMatch(/starting/);
    // the lost connection still takes precedence in the words
    expect(nearbyState(s, starting, true)).toMatchObject({ kind: "list", stale: true, text: STALE_TEXT });
  });

  it("the page marks a list shown while the service restarts, and still shows it", async () => {
    await loadHeard();
    expect(html(h(NearbyView))).not.toContain('data-testid="nearby-stale"');
    const services = apiStore.getState().services!.map((x) => (x.id === "meshtastic" ? { ...x, state: "starting" as const, reason_code: "node_db_pending", reason: "meshmud is connected; waiting for the board's list of nodes." } : x));
    apiStore.applyFrame({ event: "services", data: { services } });
    const out = html(h(NearbyView));
    expect(out).toContain('data-testid="nearby-stale"');
    expect(out).toContain(STARTING_STALE_TEXT.replace(/'/g, "&#x27;"));
    expect(out).toContain("Ridge repeater"); // still shown, never invented, never dropped
  });
});

// ------------------------------------------------------------------------------------------- the model

describe("Nearby model: untrusted text", () => {
  it("clip cuts by characters, never inside a surrogate pair, and adds an ellipsis", () => {
    expect(clip("abc", 3)).toBe("abc");
    expect(clip("abcdef", 4)).toBe("abc…");
    expect(clip("😀😀😀😀", 3)).toBe("😀😀…");
    expect(Array.from(clip("😀".repeat(100), 40))).toHaveLength(40);
  });

  it("names are cleaned (controls, bidi overrides), flattened and cut to 40 characters", () => {
    const long = "A".repeat(200);
    expect(displayName(node({ long_name: long }))).toBe(`${"A".repeat(39)}…`);
    expect(displayName(node({ long_name: "evil‮gnp.exe\u0007\nname" }))).toBe("evilgnp.exe name");
    expect(displayName(node({ long_name: "   ", short_name: "SH" }))).toBe("SH");
    expect(displayName(node({ long_name: "", short_name: "", id: "!abc‮123" }))).toBe("!abc123");
  });

  it("the rendered long hostile name is truncated on the page", async () => {
    await load("meshtastic");
    apiStore.applyFrame({ event: "nearby", data: { sources: [source([node({ long_name: `${"W".repeat(300)}<script>alert(1)</script>`, short_name: "S".repeat(60) })])] } });
    const out = html(h(NearbyView));
    expect(out).toContain(`${"W".repeat(39)}…`);
    expect(out).not.toContain("W".repeat(41));
    expect(out).not.toContain("<script>");
    expect(out).not.toContain("S".repeat(9)); // short names are cut to 8 characters
  });

  it("the short name is shown only when it adds something", () => {
    expect(shortName(node({ long_name: "Ridge repeater", short_name: "RDG1" }))).toBe("RDG1");
    expect(shortName(node({ long_name: "RDG1", short_name: "RDG1" }))).toBeNull();
    expect(shortName(node({ long_name: "", short_name: "" }))).toBeNull();
  });

  it("figures are text and tolerate a board that reports nothing, or nonsense", () => {
    expect(signalText(node({ snr: null, rssi: null }))).toBeNull();
    expect(signalText(node({ snr: -4, rssi: -118 }))).toBe("SNR -4 dB, RSSI -118 dBm");
    expect(signalText(node({ snr: Number.NaN, rssi: -90 }))).toBe("RSSI -90 dBm");
    expect(hopsText(node({ hops_away: 0 }))).toBe("direct");
    expect(hopsText(node({ hops_away: 1 }))).toBe("1 hop away");
    expect(hopsText(node({ hops_away: 7 }))).toBe("7 hops away");
    expect(hopsText(node({ hops_away: null }))).toBeNull();
    expect(hopsText(node({ hops_away: -2 }))).toBeNull();
    expect(utilizationText(node({ channel_utilization: 7.44 }))).toBe("7.4% of the channel in use");
    expect(utilizationText(node({ channel_utilization: null }))).toBeNull();
    expect(positionText(node({ position: { lat: 52.37312, lon: 4.89222, alt_m: null } }))).toBe("52.3731, 4.8922");
    expect(positionText(node({ position: { lat: 91, lon: 0 } }))).toBeNull();
    expect(positionText(node({ position: { lat: Number.POSITIVE_INFINITY, lon: 0 } }))).toBeNull();
    expect(positionText(node({ position: null }))).toBeNull();
  });

  it("last heard goes through agoText; a missing or unparsable hint is 'not reported' or 'unknown'", () => {
    expect(heardText(node({ last_heard_hint: new Date(NOW - 4 * 60_000).toISOString() }), NOW)).toBe("4 min ago");
    expect(heardText(node({ last_heard_hint: new Date(NOW - 3 * 3600_000).toISOString() }), NOW)).toBe("3 h ago");
    expect(heardText(node({ last_heard_hint: null }), NOW)).toBe("not reported");
    expect(heardText(node({ last_heard_hint: "yesterday-ish" }), NOW)).toBe("unknown");
  });

  it("a hint in this node's future never reads 'just now': a board whose clock is ahead (or lying) is said to be", () => {
    const at = (ms: number) => node({ last_heard_hint: new Date(NOW + ms).toISOString() });
    expect(CLOCK_AHEAD_TEXT).toBe("clock ahead of this node");
    // a few seconds ahead is ordinary skew between two clocks
    expect(heardText(at(5_000), NOW)).toBe("just now");
    expect(heardText(at(60_000), NOW)).toBe("just now");
    // more than a minute ahead is not
    expect(heardText(at(61_000), NOW)).toBe(CLOCK_AHEAD_TEXT);
    expect(heardText(at(3 * 3600_000), NOW)).toBe(CLOCK_AHEAD_TEXT);
    expect(heardText(at(400 * 86_400_000), NOW)).toBe(CLOCK_AHEAD_TEXT);
    // the past is unchanged
    expect(heardText(at(-10_000), NOW)).toBe("10 s ago");
    expect(heardLine(at(-4 * 60_000), NOW)).toBe("last heard 4 min ago");
    expect(heardLine(at(3600_000), NOW)).toBe("last heard: clock ahead of this node");
    expect(heardLine(node({ last_heard_hint: null }), NOW)).toBe("last heard not reported");
  });

  it("the page shows it for a node whose clock is ahead, and a node heard a minute ago still reads normally", async () => {
    await loadHeard();
    const fresh = source([
      node({ id: "!c10c", long_name: "Time traveller", last_heard_hint: new Date(NOW + 2 * 3600_000).toISOString() }),
      node({ id: "!0a11", long_name: "Normal node", last_heard_hint: new Date(NOW - 60_000).toISOString() }),
    ]);
    apiStore.applyFrame({ event: "nearby", data: { sources: [fresh] } });
    const out = html(h(NearbyView));
    expect(out).toContain("last heard: clock ahead of this node");
    expect(out).not.toContain("last heard just now");
    expect(out).toContain("last heard 1 min ago");
  });

  it("order: hops, then name, then id; the clock hint never matters", () => {
    const a = node({ id: "!a", long_name: "Zed", hops_away: 1, last_heard_hint: new Date(NOW).toISOString() });
    const b = node({ id: "!b", long_name: "Amy", hops_away: 1, last_heard_hint: new Date(NOW - 86_400_000).toISOString() });
    const c = node({ id: "!c", long_name: "Bob", hops_away: null });
    const d = node({ id: "!d", long_name: "Cat", hops_away: 0 });
    expect(sortedNodes([a, b, c, d]).map((n) => n.id)).toEqual(["!d", "!b", "!a", "!c"]);
  });

  it("the source heading is the daemon's label, else built from the board in the device list, else plain", () => {
    expect(sourceHeading(source([]), null)).toBe("heard via Meshtastic on Bench radio");
    expect(sourceHeading(source([], { label: "" }), [{ id: "usb-1", alias: "Roof board", name: "Heltec" } as never])).toBe("heard via Meshtastic on Roof board");
    expect(sourceHeading(source([], { label: "" }), [{ id: "usb-1", alias: null, name: "Heltec V3" } as never])).toBe("heard via Meshtastic on Heltec V3");
    expect(sourceHeading(source([], { label: "  " }), null)).toBe("heard via Meshtastic");
    expect(sourceHeading(source([], { label: "heard via Meshtastic on ‮evil" }), null)).toBe("heard via Meshtastic on evil");
    expect(Array.from(sourceHeading(source([], { label: "x".repeat(500) }), null)).length).toBeLessThanOrEqual(80);
  });

  it("the count line", () => {
    expect(heardSummary([source([node(), node({ id: "!2" })])])).toBe("2 Meshtastic nodes heard through 1 board.");
    expect(heardSummary([source([node()]), source([node()], { id: "b" })])).toBe("2 Meshtastic nodes heard through 2 boards.");
    expect(heardSummary([source([node()])])).toBe("1 Meshtastic node heard through 1 board.");
  });
});
