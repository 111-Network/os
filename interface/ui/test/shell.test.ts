// Pure-function tests for the app shell: mode resolution, nav filtering, attention text, layers,
// route/location helpers, pair-code handling, banner text and the announcer throttle.
import { describe, expect, it } from "vitest";
import { disconnectedText, formatAgo } from "../src/app/format";
import { DISCONNECTED_TEXT, RECONNECTED_TEXT, connectionAnnouncement, createAnnouncer, createRegionWriter } from "../src/app/LiveRegion";
import { cleanPath, hrefTo, routeFragment } from "../src/app/location";
import { formatPairCode, isCompletePairCode, retrySeconds, waitPhrase } from "../src/app/pair-code";
import { isReturnable, rememberReturn, takeReturn, type ReturnStore } from "../src/app/pair-return";
import type { ServiceRecord } from "../src/lib/api/types";
import { attentionSummary } from "../src/lib/attention";
import { LAYERS, layerBySlug } from "../src/lib/layers";
import { resolveMode } from "../src/lib/mode";
import {
  categoryLabel,
  dotState,
  glyphOf,
  healthDot,
  isActive,
  meetsMode,
  minModeFor,
  NAV,
  titleFor,
  visibleNav,
} from "../src/lib/nav";

function svc(id: ServiceRecord["id"], plain: string, over: Partial<ServiceRecord> = {}): ServiceRecord {
  return {
    id,
    name: `111-${id}`,
    plain_name: plain,
    kind: "child",
    state: "live",
    since: "2026-10-13T00:00:00Z",
    reason_code: "verified",
    reason: "ok",
    can_run_here: true,
    description: "",
    actions: [],
    detail: {},
    ...over,
  };
}

const SERVICES: ServiceRecord[] = [
  svc("agent", "Local AI agent"),
  svc("model", "Local model"),
  svc("transport", "Mesh transport", { state: "error" }),
  svc("radio", "LoRa radio", { state: "unset" }),
  svc("gateway", "Internet gateway", { state: "unset", can_run_here: false }),
];

describe("resolveMode", () => {
  it("prefers the URL over storage over the default", () => {
    expect(resolveMode("advanced", "simple")).toBe("advanced");
    expect(resolveMode("simple", "advanced")).toBe("simple");
    expect(resolveMode(null, "advanced")).toBe("advanced");
    expect(resolveMode(undefined, undefined)).toBe("simple");
    expect(resolveMode(null, null)).toBe("simple");
  });

  it("ignores junk and falls through to the next source", () => {
    expect(resolveMode("expert", "advanced")).toBe("advanced");
    expect(resolveMode(null, "garbage")).toBe("simple");
    expect(resolveMode("", "advanced")).toBe("advanced");
  });

  it("accepts case and whitespace in the URL value", () => {
    expect(resolveMode(" Advanced ", null)).toBe("advanced");
  });
});

describe("visibleNav", () => {
  it("simple mode shows the core, no Logs, no layer pages", () => {
    const n = visibleNav("simple", SERVICES);
    expect(n.core.map((i) => i.label)).toEqual(["Overview", "Agent", "Devices", "Nearby", "Messages"]);
    expect(n.core.map((i) => i.href)).toEqual(["/overview", "/", "/devices", "/nearby", "/messages"]);
    expect(n.advanced).toEqual([]);
    expect(n.settings.map((i) => i.href)).toEqual(["/settings"]);
  });

  it("advanced mode appends Logs and the ten layer pages", () => {
    const n = visibleNav("advanced", SERVICES);
    expect(n.advanced.map((i) => i.href)).toEqual(["/logs"]);
    expect(n.settings).toHaveLength(11);
    expect(n.settings[0]?.label).toBe("Essentials");
    expect(n.settings.slice(1).map((i) => i.href)).toEqual(LAYERS.map((l) => `/settings/${l.slug}`));
    expect(n.settings[1]?.label).toBe("Knowledge · L0");
  });

  it("builds one service row per runnable record and groups the rest", () => {
    const n = visibleNav("simple", SERVICES);
    expect(n.services.map((s) => [s.id, s.label, s.href, s.state])).toEqual([
      ["agent", "Local AI agent", "/services/agent", "live"],
      ["model", "Local model", "/services/model", "live"],
      ["transport", "Mesh transport", "/services/transport", "error"],
      ["radio", "LoRa radio", "/services/radio", "unset"],
    ]);
    expect(n.unavailable.map((s) => s.id)).toEqual(["gateway"]);
  });

  it("services rows are the same in both modes", () => {
    expect(visibleNav("advanced", SERVICES).services).toEqual(visibleNav("simple", SERVICES).services);
  });

  it("has no service rows before the first snapshot", () => {
    const n = visibleNav("simple", null);
    expect(n.services).toEqual([]);
    expect(n.unavailable).toEqual([]);
    expect(n.core).toHaveLength(5);
  });

  it("every static item has a unique id and an absolute route", () => {
    expect(new Set(NAV.map((i) => i.id)).size).toBe(NAV.length);
    for (const i of NAV) expect(i.href.startsWith("/")).toBe(true);
  });
});

describe("nav helpers", () => {
  it("meetsMode", () => {
    expect(meetsMode("simple", undefined)).toBe(true);
    expect(meetsMode("simple", "simple")).toBe(true);
    expect(meetsMode("simple", "advanced")).toBe(false);
    expect(meetsMode("advanced", "advanced")).toBe(true);
  });

  it("dotState goes unknown when disconnected", () => {
    expect(dotState("live", false)).toBe("live");
    expect(dotState("starting", false)).toBe("starting");
    expect(dotState("live", true)).toBe("unknown");
    expect(dotState("error", true)).toBe("unknown");
  });

  it("healthDot", () => {
    expect(healthDot(null, false)).toBe("unknown");
    expect(healthDot({ health: { status: "ok" } }, false)).toBe("live");
    expect(healthDot({ health: { status: "error" } }, false)).toBe("error");
    expect(healthDot({ health: {} }, false)).toBe("unknown");
    expect(healthDot({ health: { status: "ok" } }, true)).toBe("unknown");
  });

  it("isActive matches exactly for / and /settings, by prefix for the rest", () => {
    expect(isActive("/", "/")).toBe(true);
    expect(isActive("/", "/overview")).toBe(false);
    expect(isActive("/devices", "/devices")).toBe(true);
    expect(isActive("/devices", "/devices/add")).toBe(true);
    expect(isActive("/devices", "/devices-x")).toBe(false);
    expect(isActive("/settings", "/settings")).toBe(true);
    expect(isActive("/settings", "/settings/l3")).toBe(false);
    expect(isActive("/settings/l3", "/settings/l3")).toBe(true);
    expect(isActive("/services/agent", "/services/agent")).toBe(true);
    expect(isActive("/services/agent", "/services/model")).toBe(false);
  });

  it("glyphOf and categoryLabel", () => {
    expect(glyphOf("Overview")).toBe("OV");
    expect(glyphOf("Local AI agent")).toBe("LO");
    expect(glyphOf("x")).toBe("X");
    expect(categoryLabel("Node2A")).toBe("Node 2A");
    expect(categoryLabel("Relay1")).toBe("Relay 1");
    expect(categoryLabel("Hub4B")).toBe("Hub 4B");
  });

  it("minModeFor marks only the advanced routes", () => {
    expect(minModeFor("/logs")).toBe("advanced");
    expect(minModeFor("/settings/l4")).toBe("advanced");
    expect(minModeFor("/settings/l99")).toBeUndefined();
    expect(minModeFor("/settings")).toBeUndefined();
    expect(minModeFor("/overview")).toBeUndefined();
    expect(minModeFor("/services/agent")).toBeUndefined();
  });

  it("titleFor", () => {
    expect(titleFor("/")).toBe("Agent");
    expect(titleFor("/overview")).toBe("Overview");
    expect(titleFor("/devices/add")).toBe("Add device");
    expect(titleFor("/devices/abc")).toBe("Device");
    expect(titleFor("/services/radio", SERVICES)).toBe("LoRa radio");
    expect(titleFor("/services/radio", null)).toBe("Service");
    expect(titleFor("/settings/l3")).toBe("Intelligence · L3");
    expect(titleFor("/settings/zzz")).toBe("Settings");
    expect(titleFor("/kb/projects/meshtastic.md")).toBe("Knowledge base");
    expect(titleFor("/nope")).toBe("Not found");
    expect(titleFor("/a/b/c")).toBe("Not found");
    expect(titleFor("/devices/a/b")).toBe("Not found");
  });
});

describe("attentionSummary", () => {
  it("joins the non-zero parts in a fixed order", () => {
    expect(attentionSummary({ openProposals: 2, erroredServices: 1, erroredDevices: 1 })).toBe("2 proposals, 1 service error, 1 device error");
  });

  it("pluralizes each part", () => {
    expect(attentionSummary({ openProposals: 1, erroredServices: 2, erroredDevices: 3 })).toBe("1 proposal, 2 service errors, 3 device errors");
  });

  it("skips zero parts", () => {
    expect(attentionSummary({ openProposals: 0, erroredServices: 3, erroredDevices: 0 })).toBe("3 service errors");
  });

  it("says so when nothing needs the user", () => {
    expect(attentionSummary({ openProposals: 0, erroredServices: 0, erroredDevices: 0 })).toBe("Nothing needs you");
  });
});

describe("layers catalogue", () => {
  it("has ten entries numbered 0 to 9 with matching slugs", () => {
    expect(LAYERS).toHaveLength(10);
    LAYERS.forEach((l, i) => {
      expect(l.n).toBe(i);
      expect(l.slug).toBe(`l${i}`);
    });
  });

  it("has unique slugs and unique titles", () => {
    expect(new Set(LAYERS.map((l) => l.slug)).size).toBe(10);
    expect(new Set(LAYERS.map((l) => l.title)).size).toBe(10);
  });

  it("uses the agreed plain titles, in order", () => {
    expect(LAYERS.map((l) => l.title)).toEqual([
      "Knowledge",
      "Install & updates",
      "Identity",
      "Intelligence",
      "Interface",
      "Messaging",
      "Transport",
      "Relay & presence",
      "Storage",
      "Repairs",
    ]);
  });

  it("titles are plain words, descriptions are one sentence each", () => {
    for (const l of LAYERS) {
      expect(l.title).not.toMatch(/layer|\bL\d\b|\d/i);
      expect(l.description.length).toBeGreaterThan(40);
      expect(l.description.endsWith(".")).toBe(true);
      expect(l.description.slice(0, -1)).not.toMatch(/\.\s/); // a single sentence
    }
  });

  it("layerBySlug", () => {
    expect(layerBySlug("l3")?.title).toBe("Intelligence");
    expect(layerBySlug("l10")).toBeUndefined();
    expect(layerBySlug(undefined)).toBeUndefined();
  });
});

describe("location helpers", () => {
  it("cleanPath strips query, fragment and trailing slashes", () => {
    expect(cleanPath("/overview#needs-you")).toBe("/overview");
    expect(cleanPath("/pair?code=ABCD-EFGH")).toBe("/pair");
    expect(cleanPath("/devices/")).toBe("/devices");
    expect(cleanPath("/")).toBe("/");
    expect(cleanPath("")).toBe("/");
    expect(cleanPath("/#x")).toBe("/");
    expect(cleanPath("kb/a/b.md")).toBe("/kb/a/b.md");
  });

  it("routeFragment reads the second #", () => {
    expect(routeFragment("#/overview#needs-you")).toBe("needs-you");
    expect(routeFragment("#/overview")).toBeNull();
    expect(routeFragment("#/overview#")).toBeNull();
    expect(routeFragment("")).toBeNull();
  });

  it("hrefTo", () => {
    expect(hrefTo("/overview#needs-you")).toBe("#/overview#needs-you");
  });
});

describe("pair code", () => {
  it("uppercases, strips junk and inserts the dash after four", () => {
    expect(formatPairCode("abcd")).toBe("ABCD");
    expect(formatPairCode("abcde")).toBe("ABCD-E");
    expect(formatPairCode("abcd-efgh")).toBe("ABCD-EFGH");
    expect(formatPairCode(" ab cd - ef gh ")).toBe("ABCD-EFGH");
  });

  it("caps at eight characters", () => {
    expect(formatPairCode("abcdefghijkl")).toBe("ABCD-EFGH");
  });

  it("lets a backspace over the dash settle without getting stuck", () => {
    expect(formatPairCode("ABCD-")).toBe("ABCD");
  });

  it("isCompletePairCode", () => {
    expect(isCompletePairCode("ABCD-EFGH")).toBe(true);
    expect(isCompletePairCode("ABCD-EFG")).toBe(false);
    expect(isCompletePairCode("")).toBe(false);
  });

  it("retrySeconds reads the detail, else 60", () => {
    expect(retrySeconds(undefined)).toBe(60);
    expect(retrySeconds("Wait a minute, then try again.")).toBe(60);
    expect(retrySeconds("Try again in 45 seconds.")).toBe(45);
    expect(retrySeconds("retry after 30s")).toBe(30);
    expect(retrySeconds("wait 2 minutes")).toBe(120);
  });
});

describe("banner text", () => {
  it("formatAgo", () => {
    expect(formatAgo(0)).toBe("0s ago");
    expect(formatAgo(12.9)).toBe("12s ago");
    expect(formatAgo(59)).toBe("59s ago");
    expect(formatAgo(60)).toBe("1m ago");
    expect(formatAgo(3599)).toBe("59m ago");
    expect(formatAgo(7200)).toBe("2h ago");
    expect(formatAgo(-5)).toBe("0s ago");
  });

  it("disconnectedText", () => {
    expect(disconnectedText(1_000, 31_000)).toBe("Disconnected — last update 30s ago");
    expect(disconnectedText(null, 31_000)).toBe("Disconnected — no update received yet");
  });
});

describe("connectionAnnouncement", () => {
  const up = { disconnected: false, everOpened: true, needsPair: false };
  it("says Disconnected once, then Reconnected when the stream is back", () => {
    let r = connectionAnnouncement(false, { ...up, disconnected: true });
    expect(r).toEqual({ text: DISCONNECTED_TEXT, announced: true });
    r = connectionAnnouncement(r.announced, { ...up, disconnected: true });
    expect(r).toEqual({ text: null, announced: true });
    r = connectionAnnouncement(r.announced, up);
    expect(r).toEqual({ text: RECONNECTED_TEXT, announced: false });
    expect(connectionAnnouncement(r.announced, up)).toEqual({ text: null, announced: false });
  });
  it("says nothing to a visitor who never connected (unpaired): there is no last known state", () => {
    expect(connectionAnnouncement(false, { disconnected: true, everOpened: false, needsPair: true })).toEqual({ text: null, announced: false });
    expect(connectionAnnouncement(false, { disconnected: true, everOpened: true, needsPair: true })).toEqual({ text: null, announced: false });
  });
  it("announces Reconnected after pairing even if the disconnect was announced before the header unmounted", () => {
    const heard = connectionAnnouncement(false, { disconnected: true, everOpened: true, needsPair: false });
    // the session expired: pairing required, the stream paused (still disconnected) ... then the user pairs
    const paused = connectionAnnouncement(heard.announced, { disconnected: true, everOpened: true, needsPair: true });
    expect(paused.text).toBeNull();
    expect(connectionAnnouncement(paused.announced, up).text).toBe(RECONNECTED_TEXT);
  });
});

describe("createAnnouncer", () => {
  function rig() {
    let t = 0;
    const sent: string[] = [];
    const timers: { at: number; fn: () => void }[] = [];
    const say = createAnnouncer({
      emit: (s) => sent.push(s),
      now: () => t,
      setTimer: (fn, ms) => {
        timers.push({ at: t + ms, fn });
        return timers.length;
      },
    });
    const advance = (ms: number) => {
      t += ms;
      for (const x of timers.filter((x) => x.at <= t)) {
        timers.splice(timers.indexOf(x), 1);
        x.fn();
      }
    };
    return { say, sent, advance };
  }

  it("sends the first message at once", () => {
    const r = rig();
    r.say("a");
    expect(r.sent).toEqual(["a"]);
  });

  it("holds early messages and sends only the latest when the interval ends", () => {
    const r = rig();
    r.say("a");
    r.advance(500);
    r.say("b");
    r.advance(500);
    r.say("c");
    expect(r.sent).toEqual(["a"]);
    r.advance(999);
    expect(r.sent).toEqual(["a"]);
    r.advance(1);
    expect(r.sent).toEqual(["a", "c"]);
  });

  it("sends immediately again once the interval has passed", () => {
    const r = rig();
    r.say("a");
    r.advance(2000);
    r.say("b");
    expect(r.sent).toEqual(["a", "b"]);
  });

  it("never sends more than one message per interval", () => {
    const r = rig();
    for (let i = 0; i < 10; i++) {
      r.say(`m${i}`);
      r.advance(300);
    }
    r.advance(4000);
    expect(r.sent.length).toBeLessThanOrEqual(3);
    expect(r.sent[0]).toBe("m0");
    expect(r.sent[r.sent.length - 1]).toBe("m9");
  });
});

describe("createRegionWriter (the same sentence twice is still announced)", () => {
  function rig() {
    let t = 0;
    const writes: string[] = [];
    const timers = new Map<number, { at: number; fn: () => void }>();
    let seq = 0;
    const w = createRegionWriter({
      set: (x) => writes.push(x),
      now: () => t,
      setTimer: (fn, ms) => {
        timers.set(++seq, { at: t + ms, fn });
        return seq;
      },
      clearTimer: (h) => void timers.delete(h as number),
    });
    const advance = (ms: number) => {
      t += ms;
      for (const [id, x] of [...timers]) {
        if (x.at <= t) {
          timers.delete(id);
          x.fn();
        }
      }
    };
    return { w, writes, advance };
  }
  it("writes a new sentence at once", () => {
    const { w, writes } = rig();
    w.show("Check started.");
    w.show("Restarting.");
    expect(writes).toEqual(["Check started.", "Restarting."]);
  });
  it("empties the region, then writes an identical sentence again", () => {
    const { w, writes, advance } = rig();
    w.show("Check started.");
    w.show("Check started.");
    expect(writes).toEqual(["Check started.", ""]);
    advance(80);
    expect(writes).toEqual(["Check started.", "", "Check started."]);
  });
  it("a different sentence arriving during the gap wins", () => {
    const { w, writes, advance } = rig();
    w.show("a");
    w.show("a");
    w.show("b");
    advance(500);
    expect(writes[writes.length - 1]).toBe("b");
  });
  it("clearIf clears only the text it was told is stale", () => {
    const { w, writes, advance } = rig();
    w.show("old");
    advance(5000);
    w.show("new");
    w.clearIf("old");
    expect(writes[writes.length - 1]).toBe("new");
    advance(5000);
    w.clearIf("new");
    expect(writes[writes.length - 1]).toBe("");
  });
  it("a message announced just before the route changed is given time to be read, then goes", () => {
    const { w, writes, advance } = rig();
    w.show("forgotten");
    advance(60); // the page changes 60 ms later
    w.clearIf("forgotten");
    expect(writes).toEqual(["forgotten"]);
    advance(1900);
    expect(writes).toEqual(["forgotten"]);
    advance(100);
    expect(writes).toEqual(["forgotten", ""]);
  });
  it("a newer message cancels the pending clear", () => {
    const { w, writes, advance } = rig();
    w.show("forgotten");
    w.clearIf("forgotten");
    w.show("renamed");
    advance(5000);
    expect(writes).toEqual(["forgotten", "renamed"]);
  });
});

describe("pair screen helpers", () => {
  it("waitPhrase gives a wait that can sit still in an alert", () => {
    expect(waitPhrase(60)).toBe("about a minute");
    expect(waitPhrase(45)).toBe("about 45 seconds");
    expect(waitPhrase(120)).toBe("about 2 minutes");
    expect(waitPhrase(0)).toBe("about a second");
  });
  const mem = (): ReturnStore => {
    const m = new Map<string, string>();
    return { getItem: (k) => m.get(k) ?? null, setItem: (k, v) => void m.set(k, v), removeItem: (k) => void m.delete(k) };
  };
  it("pairing returns to the route the visitor was on, once", () => {
    const store = mem();
    rememberReturn("/devices/add", store);
    expect(takeReturn(store)).toBe("/devices/add");
    expect(takeReturn(store)).toBe("/");
  });
  it("only plain in-app routes are remembered", () => {
    for (const bad of ["/pair", "/", "", "//evil.example/x", "https://evil.example", "/a?b=1", "/a#b", "javascript:alert(1)", "/" + "a".repeat(300)]) {
      expect(isReturnable(bad), bad).toBe(false);
    }
    for (const good of ["/devices", "/services/radio", "/kb/projects/a%20b.md", "/settings/l3"]) expect(isReturnable(good), good).toBe(true);
    const store = mem();
    store.setItem("111.ui.return", "//evil.example");
    expect(takeReturn(store)).toBe("/");
    expect(takeReturn(null)).toBe("/");
  });
});
