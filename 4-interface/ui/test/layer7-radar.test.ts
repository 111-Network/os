// Layer 7 radar (os/7-relay/README.md section 10, task U1): the rings, the order inside a ring, the angle rule, the uncertainty cells, the
// legend verbatim, the glyph by source, the age from the page's clock, and the three buttons ("Find my hub" only with a home hub, one
// browse per press). The model is pure; the view is rendered from the mock's scenarios through the real store.
import { createElement as h, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { EffectCallback } from "react";
import { createMockAdapter, type MockAdapter, type ScenarioName } from "../src/lib/api/mock";
import { ApiError } from "../src/lib/api/problem";
import { apiStore } from "../src/lib/api/store";
import type { NearbyNode, NearbySource, RegistryRow } from "../src/lib/api/types";
import { ModeProvider, resetModeForTests } from "../src/lib/mode";
import NearbyView, { RegistryList } from "../src/views/nearby";
import {
  bearingDeg,
  cellKm,
  cellSidePx,
  COARSEST_CELL_KM,
  createRadarActions,
  dotPoint,
  evidenceLine,
  glyphFor,
  idAngleDeg,
  nearbyState,
  MAX_HOP_RINGS,
  MAX_RADAR_DOTS,
  RADAR_BOX,
  RADAR_LEGEND,
  radarModel,
  registryLine,
  ringName,
  ringRadius,
  type RadarDot,
} from "../src/views/nearby/model";
import { dotTitle, RadarSvg } from "../src/views/nearby/radar";
import { FakeClock, flush } from "./helpers/fakes";

// Effects do not run in a static render. While `effects.capture` is on, `useEffect` records its callback so a test can run the one it
// is about (the mount read of the registry); otherwise it is the real hook.
const effects = vi.hoisted(() => ({ capture: false, fns: [] as Array<() => unknown>, deps: [] as Array<unknown[] | undefined> }));
vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  const base = ((actual as { default?: object }).default ?? actual) as typeof import("react");
  return { ...base, default: base, useEffect: (fn: EffectCallback, deps?: unknown[]) => (effects.capture ? void (effects.fns.push(fn), effects.deps.push(deps)) : base.useEffect(fn, deps)) };
});

const NOW = Date.parse("2026-11-11T09:00:00Z");
const LEGEND_FROM_THE_SPEC =
  "Rings count hops: the inner ring is heard directly. Direction means nothing unless both nodes shared a position; otherwise a node's place on its ring comes from its id. Shaded areas are positions the sender's radio rounded. The outer ring holds nodes whose hop count is unknown.";

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

let clock = new FakeClock(NOW);
async function load(scenario: ScenarioName): Promise<MockAdapter> {
  clock = new FakeClock(NOW);
  const a = createMockAdapter({ scenario, clock, tokenDelayMs: 0 });
  made.push(a);
  apiStore.attach(a);
  await flush(30);
  return a;
}

const html = (el: ReactElement): string => {
  const g = globalThis as { window?: unknown };
  const had = g.window;
  g.window = { location: { search: "", hash: "", pathname: "/", protocol: "http:", hostname: "127.0.0.1", port: "11180" }, history: { state: null, replaceState: () => undefined }, localStorage: { getItem: () => null, setItem: () => undefined } };
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
const mesh = (nodes: NearbyNode[]): NearbySource => ({ id: "meshtastic:usb-1", kind: "meshtastic", label: "heard via Meshtastic on Bench radio", device_id: "usb-1", nodes });
const ret = (nodes: NearbyNode[]): NearbySource => ({ id: "reticulum-lxmf", kind: "reticulum", label: "heard via Reticulum announces", device_id: null, nodes });
const one = (nodes: NearbyNode[]): NearbySource => ({ id: "111", kind: "111", label: "111 nodes heard by this node", device_id: null, nodes });
const peer = (id: string, evidence: NearbyNode["evidence"], over: Partial<NearbyNode> = {}): NearbyNode =>
  node({ id, num: 0, long_name: `peer ${id.slice(0, 4)}`, short_name: id.slice(0, 4), heard_ago_s: 30, heard_seq: 1, evidence, roles: [], ...over });
const id64 = (c: string): string => c.repeat(64);

const model = (sources: NearbySource[], extra: { own?: { lat: number; lon: number } | null; receivedAtMs?: number; nowMs?: number } = {}) =>
  radarModel(sources, { own: extra.own ?? null, receivedAtMs: extra.receivedAtMs ?? NOW, nowMs: extra.nowMs ?? NOW });
const dotOf = (m: ReturnType<typeof model>, id: string): RadarDot => {
  const d = m.dots.find((x) => x.key.endsWith(`/${id}`));
  if (!d) throw new Error(`no dot for ${id}`);
  return d;
};

// ------------------------------------------------------------------------------------------------ rings

describe("radar rings (section 10.1 rule 1, 5)", () => {
  it("a Meshtastic node sits on ring hops + 1: direct on ring 1, unknown on the outer ring", () => {
    const m = model([mesh([node({ id: "!a", hops_away: 0 }), node({ id: "!b", hops_away: 1 }), node({ id: "!c", hops_away: 3 }), node({ id: "!d", hops_away: null }), node({ id: "!e", hops_away: -2 })])]);
    expect(dotOf(m, "!a").ring).toBe(1);
    expect(dotOf(m, "!b").ring).toBe(2);
    expect(dotOf(m, "!c").ring).toBe(4);
    expect(dotOf(m, "!d").ring).toBeNull();
    expect(dotOf(m, "!e").ring).toBeNull();
    expect(dotOf(m, "!c").hops).toBe(3);
    expect(dotOf(m, "!d").hops).toBeNull();
    expect(m.rings).toBe(4);
  });

  it("a Reticulum node with a hop figure from the hop read takes the same rule; without one it is unplaced", () => {
    const m = model([ret([node({ id: "r1", hops_away: 2 }), node({ id: "r2" })])]);
    expect(dotOf(m, "r1").ring).toBe(3);
    expect(dotOf(m, "r2").ring).toBeNull();
  });

  it("a 111 node on the LAN is ring 1, one heard only through the hub ring 2, and one with no usable evidence is unplaced", () => {
    const lan = id64("a");
    const both = id64("b");
    const hub = id64("c");
    const carrier = id64("d");
    const m = model([
      one([
        peer(lan, [{ transport: "tcp-lan", text: "your hub answered" }]),
        peer(both, [{ transport: "hub", text: "a message from them arrived (verified)" }, { transport: "tcp-lan", text: "a message from them arrived (verified)" }]),
        peer(hub, [{ transport: "hub", text: "a message from them arrived (verified)" }]),
        peer(carrier, [{ transport: "carrier", text: "paired hub answered" }]),
      ]),
    ]);
    expect(dotOf(m, lan).ring).toBe(1);
    expect(dotOf(m, both).ring).toBe(1);
    expect(dotOf(m, hub).ring).toBe(2);
    expect(dotOf(m, carrier).ring).toBeNull();
  });

  it("an unknown evidence transport is never read as a closer ring", () => {
    const x = id64("e");
    const m = model([one([peer(x, [{ transport: "wifi-direct" as never, text: "heard" }])])]);
    expect(dotOf(m, x).ring).toBeNull();
  });

  it("there are at least two hop rings, an outer unplaced ring beyond them, and ring 8 is the last (a farther node says so)", () => {
    expect(model([]).rings).toBe(2);
    const far = model([mesh([node({ id: "!far", hops_away: 40 }), node({ id: "!near", hops_away: 0 })])]);
    expect(far.rings).toBe(MAX_HOP_RINGS);
    expect(far.capped).toBe(true);
    expect(dotOf(far, "!far").ring).toBe(MAX_HOP_RINGS);
    expect(dotOf(far, "!far").hops).toBe(40);
    expect(ringName(MAX_HOP_RINGS, true)).toBe("7+ hops");
    expect(model([mesh([node({ id: "!x", hops_away: 7 })])]).capped).toBe(false);
  });

  it("ring names count hops", () => {
    expect([ringName(1), ringName(2), ringName(4), ringName(null)]).toEqual(["heard directly", "1 hop", "3 hops", "hops unknown"]);
  });

  it("ring radii grow outward and every dot sits inside its ring's band, the unplaced ones beyond the last hop ring", () => {
    const rings = 3;
    for (let j = 0; j <= rings; j++) expect(ringRadius(j + 1, rings)).toBeGreaterThan(ringRadius(j, rings));
    expect(ringRadius(rings + 1, rings)).toBe(RADAR_BOX.outer);
    const dist = (d: Pick<RadarDot, "ring" | "depth" | "angle">) => {
      const p = dotPoint(d, rings);
      return Math.hypot(p.x - RADAR_BOX.centre, p.y - RADAR_BOX.centre);
    };
    for (const ring of [1, 2, 3, null]) {
      const r = dist({ ring, depth: 0.5, angle: 123 });
      const k = ring ?? rings + 1;
      expect(r).toBeGreaterThan(ringRadius(k - 1, rings));
      expect(r).toBeLessThan(ringRadius(k, rings));
    }
  });
});

// ------------------------------------------------------------------------------------------------ order

describe("order inside a ring (rule 2)", () => {
  const depthOf = (m: ReturnType<typeof model>, id: string) => dotOf(m, id).depth;

  it("stronger SNR sits nearer the inside", () => {
    const m = model([mesh([node({ id: "!low", hops_away: 1, snr: -8 }), node({ id: "!high", hops_away: 1, snr: 7 }), node({ id: "!mid", hops_away: 1, snr: 0 })])]);
    expect(depthOf(m, "!high")).toBeLessThan(depthOf(m, "!mid"));
    expect(depthOf(m, "!mid")).toBeLessThan(depthOf(m, "!low"));
  });

  it("without SNR the more recent heard_ago_s sits nearer the inside; a row without an age is outermost", () => {
    const m = model([one([peer(id64("a"), [{ transport: "hub", text: "x" }], { heard_ago_s: 600 }), peer(id64("b"), [{ transport: "hub", text: "x" }], { heard_ago_s: 5 }), peer(id64("c"), [{ transport: "hub", text: "x" }], { heard_ago_s: null })])]);
    expect(depthOf(m, id64("b"))).toBeLessThan(depthOf(m, id64("a")));
    expect(depthOf(m, id64("a"))).toBeLessThan(depthOf(m, id64("c")));
  });

  it("a node with an SNR is inside one without; the order is stable and never uses a node's own clock", () => {
    const a = node({ id: "!a", hops_away: 1, snr: null, last_heard_hint: "2026-11-11T08:59:59Z" });
    const b = node({ id: "!b", hops_away: 1, snr: -15, last_heard_hint: "2020-01-01T00:00:00Z" });
    const m = model([mesh([a, b])]);
    expect(depthOf(m, "!b")).toBeLessThan(depthOf(m, "!a"));
    expect(model([mesh([b, a])]).dots.map((d) => d.key)).toEqual(m.dots.map((d) => d.key));
  });

  it("a lone node sits mid-band; rings are listed nearest first", () => {
    const m = model([mesh([node({ id: "!z", hops_away: 2 }), node({ id: "!y", hops_away: 0 })])]);
    expect(depthOf(m, "!z")).toBe(0.5);
    expect(m.dots.map((d) => d.ring)).toEqual([1, 3]);
  });
});

// ------------------------------------------------------------------------------------------------ angle

describe("angle (rule 3)", () => {
  const OWN = { lat: 52, lon: 5 };

  it("the bearing is used only when this node and the peer both report a position", () => {
    const north = node({ id: "!n", hops_away: 0, position: { lat: 53, lon: 5 } });
    const east = node({ id: "!e", hops_away: 0, position: { lat: 52, lon: 6 } });
    const silent = node({ id: "!s", hops_away: 0, position: null });
    const m = model([mesh([north, east, silent])], { own: OWN });
    expect(dotOf(m, "!n").angleFrom).toBe("bearing");
    expect(dotOf(m, "!n").angle).toBeCloseTo(0, 3);
    expect(dotOf(m, "!e").angleFrom).toBe("bearing");
    expect(dotOf(m, "!e").angle).toBeGreaterThan(89);
    expect(dotOf(m, "!e").angle).toBeLessThan(91);
    expect(dotOf(m, "!s").angleFrom).toBe("id");
    // this node says nothing: nobody gets a bearing, whatever they report
    const alone = model([mesh([north, east])]);
    expect(alone.dots.every((d) => d.angleFrom === "id")).toBe(true);
    expect(model([mesh([north])], { own: null }).dots[0]!.angle).toBe(idAngleDeg("!n"));
  });

  it("a stable hash of the id otherwise: the same id is on the same bearing every time, different ids differ, and it is a bearing", () => {
    const id = id64("a");
    expect(idAngleDeg(id)).toBe(idAngleDeg(id));
    expect(idAngleDeg(id64("b"))).not.toBe(idAngleDeg(id));
    for (const x of ["", "!a1b2c3d4", id, "é\u0000￿"]) {
      const a = idAngleDeg(x);
      expect(a).toBeGreaterThanOrEqual(0);
      expect(a).toBeLessThan(360);
    }
    const m1 = model([one([peer(id, [{ transport: "hub", text: "x" }])])]);
    const m2 = model([one([peer(id, [{ transport: "hub", text: "x" }], { heard_ago_s: 9000 })])]);
    expect(m1.dots[0]!.angle).toBe(m2.dots[0]!.angle);
  });

  it("an out-of-range or non-finite position gives no bearing and no cell", () => {
    const bad = node({ id: "!bad", hops_away: 0, position: { lat: 95, lon: 5 } });
    const nan = node({ id: "!nan", hops_away: 0, position: { lat: Number.NaN, lon: 5 } });
    const m = model([mesh([bad, nan])], { own: OWN });
    for (const id of ["!bad", "!nan"]) {
      expect(dotOf(m, id).angleFrom).toBe("id");
      expect(dotOf(m, id).cell).toBeNull();
    }
    expect(model([mesh([bad])], { own: { lat: 200, lon: 0 } }).dots[0]!.angleFrom).toBe("id");
  });

  it("bearingDeg: the four directions, and none for the same point", () => {
    expect(bearingDeg({ lat: 0, lon: 0 }, { lat: 1, lon: 0 })).toBeCloseTo(0, 6);
    expect(bearingDeg({ lat: 0, lon: 0 }, { lat: 0, lon: 1 })).toBeCloseTo(90, 6);
    expect(bearingDeg({ lat: 0, lon: 0 }, { lat: -1, lon: 0 })).toBeCloseTo(180, 6);
    expect(bearingDeg({ lat: 0, lon: 0 }, { lat: 0, lon: -1 })).toBeCloseTo(270, 6);
    expect(bearingDeg({ lat: 10, lon: 10 }, { lat: 10, lon: 10 })).toBeNull();
    const same = model([mesh([node({ id: "!same", hops_away: 0, position: { lat: 52, lon: 5 } })])], { own: { lat: 52, lon: 5 } });
    expect(same.dots[0]!.angleFrom).toBe("id");
  });

  it("north is up and east is right on the box", () => {
    const n = dotPoint({ ring: 1, depth: 0.5, angle: 0 }, 2);
    const e = dotPoint({ ring: 1, depth: 0.5, angle: 90 }, 2);
    expect(n.x).toBeCloseTo(RADAR_BOX.centre, 6);
    expect(n.y).toBeLessThan(RADAR_BOX.centre);
    expect(e.y).toBeCloseTo(RADAR_BOX.centre, 6);
    expect(e.x).toBeGreaterThan(RADAR_BOX.centre);
  });
});

// ------------------------------------------------------------------------------------------------ cells

describe("uncertainty cells (rule 4)", () => {
  it("every Meshtastic position is the coarsest cell, 10 bits, 46.7 km, in v1 (the precision field is null)", () => {
    const m = model([mesh([node({ id: "!p", hops_away: 0, position: { lat: 52.3731, lon: 4.8922, alt_m: 41 }, position_precision_bits: null }), node({ id: "!q", hops_away: 1, position: { lat: 1, lon: 2 } })])]);
    expect(dotOf(m, "!p").cell).toEqual({ bits: 10, km: 46.7 });
    expect(dotOf(m, "!q").cell).toEqual({ bits: 10, km: COARSEST_CELL_KM });
  });

  it("no position, no cell; Reticulum and 111 rows draw none either", () => {
    const m = model([
      mesh([node({ id: "!np", hops_away: 0, position: null })]),
      ret([node({ id: "r", hops_away: 1, position: { lat: 1, lon: 1 } })]),
      one([peer(id64("a"), [{ transport: "hub", text: "x" }])]),
    ]);
    expect(m.dots.every((d) => d.cell === null)).toBe(true);
  });

  it("a known precision shrinks the cell by half per bit; anything unknown or coarser stays the coarsest, never finer than it knows", () => {
    expect(cellKm(10)).toBeCloseTo(46.7, 6);
    expect(cellKm(13)).toBeCloseTo(5.84, 1);
    expect(cellKm(16)).toBeCloseTo(0.73, 2);
    expect(cellKm(19)).toBeCloseTo(0.0913, 3);
    expect(cellKm(32) * 100_000).toBeCloseTo(1.1, 1); // 1.1 cm
    for (const bits of [null, undefined, 0, 9, -1, 33, 12.5, Number.NaN, Number.POSITIVE_INFINITY]) expect(cellKm(bits)).toBe(COARSEST_CELL_KM);
    const m = model([mesh([node({ id: "!f", hops_away: 0, position: { lat: 1, lon: 1 }, position_precision_bits: 16 }), node({ id: "!g", hops_away: 0, position: { lat: 1, lon: 1 }, position_precision_bits: 3 })])]);
    expect(dotOf(m, "!f").cell).toEqual({ bits: 16, km: expect.closeTo(0.73, 2) });
    expect(dotOf(m, "!g").cell).toEqual({ bits: 10, km: 46.7 });
  });

  it("a cell is drawn in the box as a shaded square bounded by the coarsest, finer ones smaller", () => {
    expect(cellSidePx(46.7)).toBe(30);
    expect(cellSidePx(0.0001)).toBeLessThan(cellSidePx(5.8));
    expect(cellSidePx(5.8)).toBeLessThan(cellSidePx(46.7));
    expect(cellSidePx(0)).toBeGreaterThanOrEqual(8);
    const svg = renderToStaticMarkup(h(RadarSvg, { model: model([mesh([node({ id: "!p", hops_away: 0, position: { lat: 1, lon: 1 } }), node({ id: "!n", hops_away: 0 })])]) }));
    expect(svg.match(/data-testid="radar-cell"/g)).toHaveLength(1);
    expect(svg.match(/data-testid="radar-dot"/g)).toHaveLength(2);
  });
});

// ------------------------------------------------------------------------------------------------ glyphs, age, caps

describe("glyphs, ages and caps", () => {
  it("the glyph follows the source kind", () => {
    expect([glyphFor("111"), glyphFor("meshtastic"), glyphFor("reticulum"), glyphFor("zigbee")]).toEqual(["triangle", "circle", "diamond", "square"]);
    const m = model([mesh([node({ id: "!m", hops_away: 0 })]), ret([node({ id: "r", hops_away: 0 })]), one([peer(id64("a"), [{ transport: "tcp-lan", text: "x" }])])]);
    expect(m.dots.map((d) => d.glyph).sort()).toEqual(["circle", "diamond", "triangle"]);
    const svg = renderToStaticMarkup(h(RadarSvg, { model: m }));
    expect(svg).toContain("<polygon");
    expect(svg).toContain("<circle");
  });

  it("a 111 row's age is its heard_ago_s plus the page's own clock since the frame, and never from a node's clock", () => {
    const x = id64("a");
    const src = [one([peer(x, [{ transport: "tcp-lan", text: "x" }], { heard_ago_s: 20, last_heard_hint: "2000-01-01T00:00:00Z" })])];
    expect(dotOf(model(src), x).ageS).toBe(20);
    expect(dotOf(model(src, { nowMs: NOW + 100_999 }), x).ageS).toBe(120);
    expect(dotOf(model(src, { nowMs: NOW - 5_000 }), x).ageS).toBe(20); // a clock that went back never makes a row fresher than its frame
    expect(dotOf(model([mesh([node({ id: "!m", hops_away: 0 })])]), "!m").ageS).toBeNull();
    const t = dotTitle(dotOf(model(src, { nowMs: NOW + 100_999 }), x), false);
    expect(t).toContain("heard 2 min ago");
    expect(t).toContain("heard directly");
  });

  it("at most MAX_RADAR_DOTS are drawn, nearest first, and the rest are counted", () => {
    const nodes = Array.from({ length: MAX_RADAR_DOTS + 25 }, (_, i) => node({ id: `!n${i}`, hops_away: i < 10 ? 0 : 5 }));
    const m = model([mesh(nodes)]);
    expect(m.dots).toHaveLength(MAX_RADAR_DOTS);
    expect(m.hidden).toBe(25);
    expect(m.dots.slice(0, 10).every((d) => d.ring === 1)).toBe(true);
  });

  it("a hostile name is cleaned and cut before it labels a dot", () => {
    const m = model([mesh([node({ id: "!h", hops_away: 0, long_name: "‮<script>alert(1)</script>".repeat(10) })])]);
    expect(m.dots[0]!.name).not.toContain("‮");
    expect(Array.from(m.dots[0]!.name).length).toBeLessThanOrEqual(40);
    const svg = renderToStaticMarkup(h(RadarSvg, { model: m }));
    expect(svg).not.toContain("<script");
  });

  it("the evidence line says where in our own words and keeps the node's sentence as text", () => {
    expect(evidenceLine({ transport: "tcp-lan", text: "a message from them arrived (verified)" })).toBe("on this network: a message from them arrived (verified)");
    expect(evidenceLine({ transport: "hub", text: "x" })).toBe("through your hub: x");
    expect(evidenceLine({ transport: "carrier", text: "paired hub answered" })).toBe("through a carrier: paired hub answered");
  });
});

// ------------------------------------------------------------------------------------------------ the legend

describe("the legend (rule 6)", () => {
  it("is the spec's text, verbatim", () => {
    expect(RADAR_LEGEND).toBe(LEGEND_FROM_THE_SPEC);
  });
});

// ------------------------------------------------------------------------------------------------ the view

describe("the Nearby view's radar", () => {
  async function loadHeard(): Promise<MockAdapter> {
    const a = await load("meshtastic");
    const board = (await a.devices()).find((d) => d.actions.includes("identify"))!;
    await a.identifyDevice(board.id, { consent: true });
    await flush(30);
    return a;
  }

  it("draws the SVG radar with the legend verbatim, one dot per node heard, and no ASCII placeholder", async () => {
    await loadHeard();
    const out = html(h(NearbyView));
    expect(out).toContain('data-testid="radar"');
    expect(out).toContain("<svg");
    expect(out).not.toContain("radar placeholder");
    expect(out).not.toContain("arrive with Layer 7");
    const legend = /data-testid="radar-legend">([^<]*)</.exec(out)?.[1];
    expect(legend?.replace(/&#x27;/g, "'")).toBe(LEGEND_FROM_THE_SPEC);
    expect(out.match(/data-testid="radar-dot"/g)).toHaveLength(3);
    expect(out.match(/data-testid="radar-cell"/g)).toHaveLength(1); // only the Ridge repeater reports a position
    expect(out).toContain("Look for 111 nodes here");
    expect(out).toContain('data-testid="radar-map"'); // the board is bound and the panel is served
  });

  it("shows a 111 source's rows with their evidence and age from the page's clock", async () => {
    const a = await load("healthy");
    const x = id64("a");
    a.mock.bus.publish({ event: "nearby", data: { sources: [one([peer(x, [{ transport: "hub", text: "a message from them arrived (verified)" }, { transport: "tcp-lan", text: "your hub answered" }], { heard_ago_s: 95, long_name: "Sam's laptop" })])] } });
    await flush(10);
    const out = html(h(NearbyView));
    expect(out).toContain("Sam&#x27;s laptop");
    expect(out).toContain("through your hub: a message from them arrived (verified)");
    expect(out).toContain("on this network: your hub answered");
    expect(out).toContain("heard 1 min ago");
    expect(out).toContain("1 111 node heard.");
    expect(out).not.toContain("Meshtastic nodes heard through");
    expect(out).not.toContain("last heard not reported");
    expect(out).toContain('data-ring="1"');
  });

  it("offers the hop read only when a Reticulum source is listed", async () => {
    await load("healthy");
    expect(html(h(NearbyView))).not.toContain("radar-hops");
    apiStore.detach();
    apiStore.reset();
    await load("bridge");
    expect(html(h(NearbyView))).toContain('data-testid="radar-hops"');
  });

  it("offers no map link while no Meshtastic panel is served", async () => {
    await load("healthy");
    expect(html(h(NearbyView))).not.toContain("radar-map");
  });

  it("'Find my hub' is shown only on a node with a home hub", async () => {
    await loadHeard();
    expect(apiStore.getState().fabric?.hub.enrolled).toBe(true);
    expect(html(h(NearbyView))).toContain('data-testid="radar-find-hub"');
    apiStore.detach();
    apiStore.reset();
    await load("messaging-new");
    expect(apiStore.getState().fabric?.hub.enrolled).toBe(false);
    const out = html(h(NearbyView));
    expect(out).not.toContain("Find my hub");
    expect(out).toContain("Look for 111 nodes here");
  });
});

// ------------------------------------------------------------------------------------------------ the buttons

describe("the radar's buttons", () => {
  it("'Look for 111 nodes here' and 'Find my hub' each send exactly one POST /registry/browse, and a registry frame follows into the store", async () => {
    const a = await load("healthy");
    const browse = vi.spyOn(a.mock.relay, "browseRegistry");
    const hops = vi.spyOn(a.mock.relay, "nearbyHops");
    const actions = createRadarActions(async () => a.mock.relay);
    expect(apiStore.getState().registry).toBeNull();
    const look = await actions.look();
    expect(look.tone).toBe("info");
    expect(browse).toHaveBeenCalledTimes(1);
    expect(apiStore.getState().registry).toBeNull(); // the 202 carries no rows: they arrive as a frame
    await clock.advanceAsync(1000, 100);
    await flush(10);
    const rows = apiStore.getState().registry;
    expect(rows).toHaveLength(2);
    expect(rows!.map((r) => r.answered_by).sort()).toEqual(["hint", "signed"]);
    const find = await actions.find();
    expect(find.tone).toBe("info");
    expect(find.text).toContain("your hub");
    expect(browse).toHaveBeenCalledTimes(2);
    expect(hops).not.toHaveBeenCalled();
    await clock.advanceAsync(1000, 100);
    await flush(10);
    expect(apiStore.getState().registry).toHaveLength(2);
  });

  it("the hop read is one POST /nearby/hops; a second inside 30 s is the node's own 429 sentence, shown as text", async () => {
    const a = await load("healthy");
    const hops = vi.spyOn(a.mock.relay, "nearbyHops");
    const actions = createRadarActions(async () => a.mock.relay);
    expect((await actions.hops()).tone).toBe("info");
    const again = await actions.hops();
    expect(hops).toHaveBeenCalledTimes(2);
    expect(again.tone).toBe("problem");
    expect(again.text).toBe("Hop counts were read less than 30 seconds ago.");
  });

  it("a refusal or a network failure is a problem sentence, and the failed press is still one call", async () => {
    const down = { browseRegistry: vi.fn(async () => { throw ApiError.network(new Error("down")); }), nearbyHops: vi.fn() };
    const r = await createRadarActions(async () => down).look();
    expect(r.tone).toBe("problem");
    expect(r.text.length).toBeGreaterThan(0);
    expect(down.browseRegistry).toHaveBeenCalledTimes(1);
    const broken = await createRadarActions(async () => { throw new Error("chunk failed"); }).find();
    expect(broken).toMatchObject({ tone: "problem", text: "Could not look for your hub." });
  });
});

// ------------------------------------------------------------------------------------------------ the registry answers

const HUB = id64("9");
const hintRow = (over: Partial<RegistryRow> = {}): RegistryRow => ({ id: "inst-1", label: "abcd-efgh-ij", node_id: id64("1"), address: "192.168.1.31:11183", roles: ["hub", "gateway"], answered_by: "hint", heard_ago_s: 1, ...over });
const signedRow = (over: Partial<RegistryRow> = {}): RegistryRow => ({ id: id64("2"), label: "kind-otter-ab", node_id: id64("2"), address: null, roles: ["mailbox"], answered_by: "signed", heard_ago_s: 95, ...over });
const rowsOf = (out: string): string[] => [...out.matchAll(/data-testid="radar-registry-row"[^>]*>([^<]*)</g)].map((m) => m[1]!.replace(/&#x27;/g, "'"));

describe("registry answers say which layer answered (section 7.6)", () => {
  it("the words: hint (unverified), signed by this contact, anything else unknown", () => {
    expect(registryLine(hintRow(), null, null)).toContain("hint (unverified)");
    expect(registryLine(signedRow(), null, null)).toContain("signed by this contact");
    expect(registryLine({ ...signedRow(), answered_by: "asked" as never }, null, null)).toContain("unknown");
  });

  it("a malformed row (no roles, a non-string label) cannot throw during render", () => {
    const bad = { ...signedRow(), roles: undefined, label: 42 } as never;
    let line = "";
    expect(() => (line = registryLine(bad, null, null))).not.toThrow();
    expect(line).toBe(" · signed by this contact");
  });

  it("a hint's label and roles are cleaned and presented as its claim", () => {
    const line = registryLine(hintRow({ label: "‮evil<b>\nname", roles: ["hub\u0007", "‮mailbox"] }), null, "just now");
    expect(line).not.toMatch(/[‮\u0007\n]/);
    expect(line).toContain("(its own claim)");
    expect(line).toContain("roles it claims: hub, mailbox");
    expect(line).toBe("evil<b> name (its own claim) · roles it claims: hub, mailbox · hint (unverified) · just now");
  });

  it("a signed row is the contact's word, and only it is marked 'your hub'", () => {
    expect(registryLine(signedRow({ node_id: HUB }), HUB, "1 min ago")).toBe("kind-otter-ab · mailbox · signed by this contact · 1 min ago · your hub");
    expect(registryLine(signedRow(), HUB, "1 min ago")).not.toContain("your hub");
  });

  it("a hint that names the home hub says so as not verified, never 'your hub'; an unknown layer is not 'your hub' either", () => {
    const hint = registryLine(hintRow({ node_id: HUB }), HUB, null);
    expect(hint).toContain("its hint names your hub (not verified)");
    expect(hint).not.toMatch(/ · your hub/);
    const unknown = registryLine({ ...hintRow({ node_id: HUB }), answered_by: "future" as never }, HUB, null);
    expect(unknown).toContain("unknown");
    expect(unknown).not.toMatch(/ · your hub/);
  });

  it("the page lists a hint row and a signed row with their layer and age, and a hint naming the home hub", async () => {
    const a = await load("healthy");
    expect(a).toBeDefined();
    apiStore.applyFrame({ event: "registry", data: { rows: [hintRow({ node_id: id64("1") }), signedRow()] } });
    const out = html(h(NearbyView));
    const rows = rowsOf(out);
    expect(rows).toEqual([
      "abcd-efgh-ij (its own claim) · roles it claims: hub, gateway · hint (unverified) · just now",
      "kind-otter-ab · mailbox · signed by this contact · 1 min ago",
    ]);
  });

  it("on a node with a home hub, a signed row of the hub is 'your hub' and a hint naming it is 'not verified'", async () => {
    await load("meshtastic");
    const hub = apiStore.getState().fabric?.hub;
    expect(hub?.enrolled).toBe(true);
    const hubId = hub!.node_id!;
    apiStore.applyFrame({ event: "registry", data: { rows: [hintRow({ node_id: hubId }), signedRow({ node_id: hubId, id: hubId })] } });
    const rows = rowsOf(html(h(NearbyView)));
    expect(rows[0]).toContain("its hint names your hub (not verified)");
    expect(rows[0]).not.toMatch(/ · your hub/);
    expect(rows[1]).toMatch(/signed by this contact · 1 min ago · your hub$/);
  });

  it("a registry row ages from the registry's own receive time, not the nearby snapshot's", () => {
    const row = signedRow({ heard_ago_s: 2 });
    const list = (receivedAt: number, now: number) => rowsOf(renderToStaticMarkup(h(RegistryList, { rows: [row], hubId: null, receivedAt, now })));
    expect(list(NOW, NOW)).toEqual(["kind-otter-ab · mailbox · signed by this contact · just now"]);
    // received 10 minutes ago: the row has been held for that long
    expect(list(NOW - 600_000, NOW)[0]).toContain("10 min ago");
    // received just now (a browse answered ten minutes after the page opened): the row is fresh, whatever the nearby frame's age
    expect(list(NOW, NOW + 3_000)[0]).toContain("just now");
    expect(list(NOW, NOW + 3_000)[0]).not.toContain("min ago");
  });
});

// ------------------------------------------------------------------------------------------------ the mount read

describe("the registry is read once on mount", () => {
  const runMountEffects = (): void => {
    const deps = effects.deps.splice(0);
    effects.fns.splice(0).forEach((fn, i) => {
      if (!String(fn).includes("refreshRegistry")) return;
      expect(deps[i]).toEqual([]); // once on mount: no dependency to re-run on, no poll
      fn();
    });
  };
  const mountView = (): string => {
    effects.capture = true;
    try {
      return html(h(NearbyView));
    } finally {
      effects.capture = false;
    }
  };
  afterEach(() => {
    effects.capture = false;
    effects.fns.length = 0;
    effects.deps.length = 0;
  });

  it("calls refreshRegistry once, a GET with no poll, and shows the rows a reload would have lost", async () => {
    const a = await load("healthy");
    apiStore.attachRelay(async () => ({ ...a.mock.relay, registry: async () => [signedRow()] }));
    const refresh = vi.spyOn(apiStore, "refreshRegistry");
    expect(apiStore.getState().registry).toBeNull();
    mountView();
    runMountEffects();
    expect(refresh).toHaveBeenCalledTimes(1);
    await flush(10);
    expect(apiStore.getState().registry).toHaveLength(1);
    expect(rowsOf(html(h(NearbyView)))).toEqual(["kind-otter-ab · mailbox · signed by this contact · 1 min ago"]);
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("a node without Layer 7 answers 404: the slice stays null and the page is as it was", async () => {
    const a = await load("healthy");
    const before = html(h(NearbyView));
    const registry = vi.fn(async () => Promise.reject(new ApiError({ type: "about:blank", title: "Not found", status: 404, code: "not_found" })));
    apiStore.attachRelay(async () => ({ ...a.mock.relay, registry }));
    const refresh = vi.spyOn(apiStore, "refreshRegistry");
    mountView();
    runMountEffects();
    await flush(10);
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(registry).toHaveBeenCalledTimes(1);
    expect(apiStore.getState().registry).toBeNull();
    const after = html(h(NearbyView));
    expect(after).toBe(before);
    expect(after).not.toContain('data-testid="radar-registry"');
  });
});

// ------------------------------------------------------------------------------------------------ the board states (section 2.3)

describe("the Meshtastic states read only the Meshtastic sources", () => {
  const live = { state: "live" as const, reason_code: "meshtastic_live", reason: "" };
  const unset = { state: "unset" as const, reason_code: "no_meshtastic", reason: "" };
  const erroring = { state: "error" as const, reason_code: "child_backoff", reason: "" };
  const starting = { state: "starting" as const, reason_code: "node_db_pending", reason: "" };

  it("an empty 111 source never turns a node with no board into 'empty'", () => {
    expect(nearbyState([one([])], unset, false).kind).toBe("no_board");
    expect(nearbyState([one([])], null, false).kind).toBe("no_board");
    expect(nearbyState([one([]), ret([])], null, false).kind).toBe("no_board");
    expect(nearbyState([mesh([])], null, false).kind).toBe("empty");
    expect(nearbyState([one([])], live, false).kind).toBe("empty");
  });

  it("an empty 111 source changes no other board state either", () => {
    expect(nearbyState([one([])], { state: "unset", reason_code: "meshtastic_unbound", reason: "" }, false).kind).toBe("unbound");
    expect(nearbyState([one([])], erroring, false).kind).toBe("error");
    expect(nearbyState([one([])], starting, false).kind).toBe("starting");
    expect(nearbyState(null, null, false, true).kind).toBe("unavailable");
  });

  it("a 111-only list carries no Meshtastic stale text, whatever the Meshtastic service says", () => {
    const l = [one([peer(id64("a"), [{ transport: "tcp-lan", text: "your hub answered" }])])];
    expect(nearbyState(l, erroring, false)).toEqual({ kind: "list", stale: false, text: null });
    expect(nearbyState(l, starting, false)).toEqual({ kind: "list", stale: false, text: null });
    // a lost connection is about the node itself and still marks every list
    expect(nearbyState(l, erroring, true)).toMatchObject({ kind: "list", stale: true });
  });

  it("a list with Meshtastic nodes keeps its stale text, and 111 rows beside them do not remove it", () => {
    const l = [mesh([node({ id: "!a", hops_away: 0 })]), one([peer(id64("a"), [{ transport: "tcp-lan", text: "x" }])])];
    expect(nearbyState(l, erroring, false)).toMatchObject({ kind: "list", stale: true, text: "The Meshtastic service has a problem, so this is the last list it heard." });
    expect(nearbyState(l, starting, false)).toMatchObject({ kind: "list", stale: true });
    expect(nearbyState(l, live, false)).toEqual({ kind: "list", stale: false, text: null });
  });

  it("the page of a 111-only list under a failing Meshtastic service shows no stale notice", async () => {
    const a = await load("healthy");
    a.mock.bus.publish({ event: "nearby", data: { sources: [one([peer(id64("a"), [{ transport: "tcp-lan", text: "your hub answered" }])])] } });
    await flush(10);
    const services = apiStore.getState().services!.map((x) => (x.id === "meshtastic" ? { ...x, state: "error" as const, reason_code: "child_backoff", reason: "" } : x));
    apiStore.applyFrame({ event: "services", data: { services } });
    const out = html(h(NearbyView));
    expect(out).toContain('data-testid="nearby-lists"');
    expect(out).not.toContain('data-testid="nearby-stale"');
    expect(out).not.toContain("last list it heard");
  });
});
