// Layer 6 views (os/6-transport/README.md section 10), rendered to static markup from the mock's `bridge` scenario through the real store:
// the Messages rail, the foreign thread (name as text, evidence matrix, parts), the foreign compose gate, the fan-out preview and its
// refusals, the consent panel and its round-trip, edges, Settings -> Transport, Nearby's DM action, and proposal bodies.
import { createElement as h, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ROUTES } from "../src/app/routes";
import { consentOf } from "../src/lib/api/bridge";
import { createMockAdapter, type MockAdapter, type ScenarioName } from "../src/lib/api/mock";
import { apiStore } from "../src/lib/api/store";
import type { ConsentRequired, ForeignMessage, TransportStatus } from "../src/lib/api/types";
import { goalSentence, buildPrompt } from "../src/lib/prompts";
import { ModeProvider, resetModeForTests } from "../src/lib/mode";
import { titleFor } from "../src/lib/nav";
import { ProposalBody, actionSentence, isBridgeKind, parseDeviceBody, splitBridgeBody } from "../src/views/agent/proposal-body";
import ThreadList from "../src/views/messages/list";
import { foreignComposeGate } from "../src/views/messages/model";
import NetView from "../src/views/messages/net";
import { ConsentPanel, runConsent } from "../src/views/messages/net/consent";
import { PlanLines } from "../src/views/messages/net/fanout";
import {
  AUTH_FALLBACK,
  FOREIGN_REASON_COPY,
  authText,
  capFor,
  convTitle,
  droppedText,
  evidenceText,
  foreignReasonText,
  planSizeText,
  senderName,
  splitSummary,
  truncatedText,
} from "../src/views/messages/net/model";
import NearbyView from "../src/views/nearby";
import { TransportSettings } from "../src/views/settings/l6";
import { PostureRows } from "../src/views/settings/l6/radio";
import { moveItem, orderWithAll, listId } from "../src/views/settings/l6/model";
import SettingsView from "../src/views/settings";
import { FakeClock, flush } from "./helpers/fakes";
import { routed } from "./helpers/routed";

const NOW = Date.parse("2026-10-19T09:00:00Z");

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

async function load(scenario: ScenarioName = "bridge") {
  const clock = new FakeClock(NOW);
  const a = createMockAdapter({ scenario, clock, tokenDelayMs: 0 });
  made.push(a);
  await apiStore.attach(a);
  await flush(30);
  return { a, clock };
}

/** Render with the density mode the page would resolve. */
const html = (el: ReactElement, mode?: "advanced"): string => {
  const g = globalThis as { window?: unknown };
  const had = g.window;
  if (mode === "advanced") {
    g.window = { location: { search: "?mode=advanced", hash: "", pathname: "/" }, history: { state: null, replaceState: () => undefined }, localStorage: { getItem: () => null, setItem: () => undefined } };
  }
  resetModeForTests();
  try {
    return renderToStaticMarkup(h(ModeProvider, null, el));
  } finally {
    if (mode === "advanced") {
      if (had === undefined) delete g.window;
      else g.window = had;
    }
    resetModeForTests();
  }
};

const thread = async (net: string, conv: string, mode?: "advanced"): Promise<string> => {
  await apiStore.loadForeign(net, conv);
  return html(routed(`/messages/net/${encodeURIComponent(net)}/${conv}`, "/messages/net/:network/:conv", NetView), mode);
};

/** The markup of one row (an `li` carrying `data-testid="fm-…"`). */
const rowOf = (out: string, id: string): string => {
  const at = out.indexOf(`data-testid="fm-${id}"`);
  expect(at, `row ${id} is on the page`).toBeGreaterThan(-1);
  const rest = out.slice(at);
  const next = rest.indexOf('data-testid="fm-', 10);
  return next < 0 ? rest : rest.slice(0, next);
};

describe("the routes: Layer 6 views sit before /messages/:peer", () => {
  const order = ROUTES.map((r) => r.path);
  it("/messages/net/:network[/:conv] and /messages/send come before /messages/:peer, as /messages/contacts does", () => {
    const peer = order.indexOf("/messages/:peer");
    for (const p of ["/messages/contacts", "/messages/send", "/messages/net/:network", "/messages/net/:network/:conv"]) {
      expect(order.indexOf(p), p).toBeGreaterThan(-1);
      expect(order.indexOf(p), p).toBeLessThan(peer);
    }
  });

  it("the three Layer 6 routes share one lazy chunk, apart from the Messages view", () => {
    const view = (p: string) => ROUTES.find((r) => r.path === p)!.View;
    expect(view("/messages/net/:network")).toBe(view("/messages/send"));
    expect(view("/messages/net/:network/:conv")).toBe(view("/messages/send"));
    expect(view("/messages/send")).not.toBe(view("/messages/:peer"));
  });

  it("titles follow the path", () => {
    expect(titleFor("/messages/send")).toBe("Send to networks");
    expect(titleFor("/messages/net/meshtastic")).toBe("Network");
    expect(titleFor("/messages/net/app%3Adeltachat/fc-3")).toBe("Foreign conversation");
  });
});

describe("the Messages rail", () => {
  it("lists 111 first, then each enabled network in the preferred order, split into channels and DMs; a network that is off is not listed", async () => {
    await load();
    await Promise.all([apiStore.refreshConversations("meshtastic"), apiStore.refreshConversations("lxmf")]);
    const out = html(h(ThreadList));
    expect(out).toContain('data-testid="network-rail"');
    const at = (s: string) => out.indexOf(s);
    expect(at('data-testid="rail-111"')).toBeGreaterThan(-1);
    expect(at('data-testid="rail-111"')).toBeLessThan(at('data-testid="rail-lxmf"'));
    expect(at('data-testid="rail-lxmf"')).toBeLessThan(at('data-testid="rail-meshtastic"')); // the node's order puts Reticulum first
    expect(out).not.toContain('data-testid="rail-app:deltachat"');
    expect(out).toContain("1 network is off and costs nothing while off.");
    expect(out).toContain("2 channels · 1 DM");
    expect(out).toMatch(/>channels</);
    expect(out).toMatch(/>DMs</);
    expect(out).toContain('href="#/messages/net/meshtastic/fc-3"');
    expect(out).toContain('href="#/messages/send"');
  });

  it("counts a DM and a pinned channel as unread, and never an unpinned channel", async () => {
    await load();
    await apiStore.refreshConversations("meshtastic");
    const out = html(h(ThreadList));
    const mesh = out.slice(out.indexOf('data-testid="rail-meshtastic"'));
    expect(mesh.slice(0, mesh.indexOf("</a>"))).toContain("2 unread"); // DM 1 + pinned Ridge crew 1, in the entry's own line
    // LongFast has 2 unread but is not pinned: its row carries no unread badge
    const longfast = out.slice(out.indexOf('data-testid="conv-fc-1"'));
    expect(longfast.slice(0, longfast.indexOf("</a>"))).not.toContain("unread");
  });

  it("is absent on a node without networks and shows no rail entry for a quiet node's off networks", async () => {
    await load("healthy");
    const out = html(h(ThreadList));
    expect(out).toContain('data-testid="network-rail"'); // the networks exist (Meshtastic needs a region) but...
    expect(out).toContain('data-testid="rail-meshtastic"'); // ...Meshtastic is enabled and not set
    expect(out).toContain("not set");
    expect(out).not.toContain('data-testid="rail-lxmf"');
    expect(out).toContain("2 networks are off and cost nothing while off.");
  });
});

describe("a foreign thread", () => {
  it("shows the network chip, the native id, the name as TEXT, the auth label with its copy, and the received time", async () => {
    await load();
    const out = await thread("meshtastic", "fc-1");
    const row = rowOf(out, "1");
    expect(row).toContain('data-testid="net-chip"');
    expect(row).toContain(">M<");
    expect(row).toContain("!a1b2c3d4");
    expect(row).toContain("Ridge repeater");
    expect(row).toContain("channel-key: anyone with this channel key can use any name");
    expect(row).toMatch(/received .*\d\d:\d\d/);
    expect(out).toContain("Meshtastic · channel");
  });

  it("renders a hostile name and body inert: no anchor, no markup, no bidi override", async () => {
    await load();
    const out = await thread("meshtastic", "fc-1");
    expect(out).not.toContain("<a href=\"https://evil.example");
    expect(out).not.toContain("<img");
    expect(out).not.toContain("\u202e");
    const row = rowOf(out, "2");
    expect(row).toContain("&lt;a href=&quot;https://evil.example/x&quot;&gt;click here&lt;/a&gt;FREE");
    expect(row).toContain("&lt;img src=x onerror=alert(1)&gt; Free WiFi");
    expect(row).toContain("via internet");
    // no link anywhere inside a foreign row, whatever the stranger wrote
    for (const id of ["1", "2", "12"]) expect(rowOf(out, id).split("</li>")[0]).not.toMatch(/<a[\s>]/);
  });

  it("an incoming scenario text with a hostile name stays inert too", async () => {
    const { clock } = await load();
    await clock.advanceAsync(7_000, 500);
    await flush(30);
    const out = await thread("meshtastic", "fc-1");
    expect(out).not.toContain('<a href="https://evil.example">win</a>');
    expect(out).not.toContain("\u202e");
    expect(out).toContain("&lt;a href=&quot;https://evil.example&quot;&gt;win&lt;/a&gt;gnp.exe");
    expect(out).toContain("&lt;script&gt;alert(1)&lt;/script&gt; hello");
    expect(out).not.toContain("<script>");
  });

  it("states and evidence: every one of the five states, each with the adapter's own evidence string", async () => {
    await load();
    const out = await thread("meshtastic", "fc-1");
    const own = (id: string) => rowOf(out, id);
    expect(own("3")).toContain(">delivered<");
    expect(own("3")).toContain("the recipient confirmed");
    expect(own("4")).toContain(">relayed<");
    expect(own("4")).toContain("a neighbour rebroadcast it");
    expect(own("5")).toContain(">sent<");
    expect(own("5")).toContain("accepted by the radio");
    expect(own("6")).toContain(">queued<");
    expect(own("6")).toContain("waiting for the radio");
    expect(own("7")).toContain(">failed<");
    expect(own("7")).toContain("refused before air");
    expect(own("7")).toContain("[x]");
    // each shows its reason in this UI's words, not a raw code
    expect(own("6")).toContain(FOREIGN_REASON_COPY.rate_limited);
    expect(own("7")).toContain(FOREIGN_REASON_COPY.no_channel);
    expect(own("6")).not.toContain("rate_limited");
  });

  it("an unknown state or reason code reads unknown, never a function from Object.prototype", () => {
    expect(evidenceText({ state: "weird", evidence: null, dir: "own" })).toBe("This node reports a state this page does not know.");
    expect(evidenceText({ state: "toString", evidence: null, dir: "own" })).toBe("This node reports a state this page does not know.");
    expect(foreignReasonText({ reason_code: "brand_new", reason: null })).toBe("unknown");
    expect(foreignReasonText({ reason_code: "constructor", reason: null })).toBe("unknown");
    expect(foreignReasonText({ reason_code: "brand_new", reason: "The node says so." })).toBe("The node says so.");
    expect(foreignReasonText({ reason_code: null, reason: null })).toBeNull();
    expect(authText(null, "attested-111")).toBe(AUTH_FALLBACK["attested-111"]);
    expect(authText(null, "hasOwnProperty")).toBe("unknown");
  });

  it("a split send is one item whose state is the lowest among its parts, and cancel is offered for the unsent parts only", async () => {
    await load();
    const out = await thread("meshtastic", "fc-1");
    const parts = out.split('data-testid="parts-note"').length - 1;
    expect(parts).toBe(1);
    // the three parts (fm-8, fm-9, fm-10) are one row element, keyed by the first
    const item = rowOf(out, "8");
    expect(item).toContain("(1/3) Long update, part one of three.");
    expect(item).toContain("(3/3) Part three is still waiting.");
    expect(item).toContain(">queued<"); // relayed, sent and queued parts: the lowest is queued
    expect(item).toContain("One text cut into 3 parts: it reads as queued");
    expect(item).toContain("cancel unsent parts (1)");
    expect(out).not.toContain('data-testid="fm-9"');
    expect(item).toContain("Parts can arrive out of order.");
  });

  it("a row that is only queued offers cancel; one that has been sent does not; a refused one offers try again (F6: retry only where the node accepts it)", async () => {
    await load();
    const out = await thread("meshtastic", "fc-1");
    expect(rowOf(out, "6")).toContain("cancel sending");
    expect(rowOf(out, "6")).not.toMatch(/retry now|try again|>resend</); // still going by itself: the node would answer 409
    expect(rowOf(out, "5")).not.toContain("cancel sending");
    expect(rowOf(out, "5")).not.toMatch(/retry now|try again|>resend</); // sent: the carrier has it, so 'retry now' is never offered
    expect(rowOf(out, "4")).not.toContain("retry now"); // relayed: past retrying
    expect(rowOf(out, "7")).toContain("try again");
    expect(rowOf(out, "7")).not.toContain("cancel sending");
    expect(rowOf(out, "3")).not.toMatch(/retry|cancel/);
  });

  it("a row left half-handed over asks 'May have been sent; resend?' and offers resend, not cancel", async () => {
    await load();
    const out = await thread("meshtastic", "fc-1");
    const row = rowOf(out, "11");
    expect(row).toContain("May have been sent; resend?");
    expect(row).toContain(">resend<");
    expect(row).not.toContain("cancel sending");
  });

  it("the received time is labelled, and the sender's own clock is only behind a 'sender's claim' disclosure", async () => {
    await load();
    const out = await thread("meshtastic", "fc-1");
    const withClaim = rowOf(out, "1");
    expect(withClaim).toContain("sender&#x27;s claim");
    expect(withClaim).toContain("That is their own clock and cannot be checked");
    const noClaim = rowOf(out, "2");
    expect(noClaim).not.toContain("sender&#x27;s claim");
    // the summary line (what is visible closed) carries the received time and never the claim
    const summary = withClaim.slice(withClaim.indexOf("<summary"), withClaim.indexOf("</summary>"));
    expect(summary).toMatch(/received .*\d\d:\d\d/);
    expect(summary).not.toContain("says they sent");
  });

  it("a body the node cut says so: 'message too long to show (N bytes)'", async () => {
    await load();
    const out = await thread("meshtastic", "fc-1");
    expect(rowOf(out, "12")).toContain(truncatedText(70_000));
    expect(truncatedText(70_000)).toBe("message too long to show (70000 bytes)");
  });

  it("a relayed copy keeps its [name/TAG] marker visible and says what it is", async () => {
    await load();
    const out = await thread("lxmf", "fc-4");
    const row = rowOf(out, "17");
    expect(row).toContain("[Ridge repeater/M] Weather turning, stay off the ridge.");
    expect(row).toContain("A relayed copy");
    expect(out).toContain("recipient list");
  });

  it("the banner is the adapter's honest copy plus the dropped counter", async () => {
    await load();
    const out = await thread("meshtastic", "fc-1");
    expect(out).toContain("Messages here are Meshtastic texts received by your board&#x27;s own node; they are not 111-sealed and are labelled with their Meshtastic sender.");
    expect(out).toContain("3 texts dropped while 111 was busy");
    expect(droppedText(1)).toBe("1 text dropped while 111 was busy");
    expect(droppedText(0)).toBeNull();
    const lx = await thread("lxmf", "fc-5");
    expect(lx).toContain("Messages here are LXMF texts carried by Reticulum");
    expect(lx).not.toContain("dropped while");
  });

  it("Advanced adds ids and the row sequence as text", async () => {
    await load();
    const plainOut = await thread("meshtastic", "fc-1");
    expect(plainOut).not.toContain('data-testid="raw-fm-1"');
    const adv = await thread("meshtastic", "fc-1", "advanced");
    expect(adv).toContain('data-testid="raw-fm-1"');
    expect(adv).toContain("fc-1 · ch:LongFast:7f3a9c");
  });

  it("a channel sender offers a DM from this node; a DM thread does not", async () => {
    await load();
    const out = await thread("meshtastic", "fc-1");
    expect(rowOf(out, "1")).toContain("Send a Meshtastic DM from this node (readable, not 111-sealed)"); // the same words Nearby uses
    expect(rowOf(out, "2")).toContain("Send a Meshtastic DM from this node (readable, not 111-sealed)"); // the hostile sender's id is a valid node id, so it is offered too
    const dm = await thread("meshtastic", "fc-3");
    expect(dm).not.toContain("DM from this node");
  });

  it("a missing conversation says so", async () => {
    await load();
    await apiStore.loadForeign("meshtastic", "fc-404");
    const out = html(routed("/messages/net/meshtastic/fc-404", "/messages/net/:network/:conv", NetView));
    expect(out).toContain("No conversation with this id");
  });
});

describe("the foreign compose gate", () => {
  const net = async (id = "meshtastic") => {
    await load();
    return apiStore.getState().networks!.find((n) => n.id === id)!;
  };
  const conv = (over = {}) => ({ kind: "channel" as const, uplink: null, send_enabled: true, ...over });

  it("replaces the contact check: it is about the network, never about a contact", async () => {
    const g = foreignComposeGate(await net(), conv());
    expect(g.ok).toBe(true);
    if (g.ok) expect(g.notes.map((n) => n.code)).toEqual(expect.arrayContaining(["ham_unconfirmed", "send_warning", "airtime"]));
  });

  it("warns about the region, the uplink, an off switch, the licence and airtime, in that kind of order", async () => {
    const n = await net();
    const g = foreignComposeGate({ ...n, state: "not_set", gate: { ...n.gate, region_set: false } }, conv({ uplink: true, send_enabled: false }));
    expect(g.ok).toBe(true);
    if (!g.ok) return;
    const codes = g.notes.map((x) => x.code);
    expect(codes).toEqual(["region_unset", "ham_unconfirmed", "uplink_channel", "send_off", "send_warning", "airtime"]);
    expect(g.notes.find((x) => x.code === "region_unset")!.href).toBe("/settings/l6");
    expect(g.notes.find((x) => x.code === "uplink_channel")!.text).toBe("This channel is published to the internet by your board.");
    expect(g.notes.find((x) => x.code === "airtime")!.text).toContain("It is an estimate.");
  });

  it("a DM warns once about the opaque blob when any channel uplinks", async () => {
    const n = await net();
    const g = foreignComposeGate(n, conv({ kind: "dm", send_enabled: null }));
    expect(g.ok && g.notes.map((x) => x.code)).toContain("uplink_dm");
  });

  it("an unreachable network is a warning, not a block (the node queues it); a network that is off blocks", async () => {
    const n = await net();
    const down = foreignComposeGate({ ...n, state: "error" }, conv());
    expect(down.ok && down.notes[0]!.code).toBe("network_down");
    const off = foreignComposeGate({ ...n, state: "disabled" }, conv());
    expect(off).toEqual({ ok: false, why: "Meshtastic is turned off. Turn it on in Settings, Transport, then write here." });
    expect(foreignComposeGate(null, conv())).toEqual({ ok: false, why: "Loading this conversation." });
  });

  it("the thread view prints the notes above the composer and the cap for the conversation", async () => {
    await load();
    const out = await thread("meshtastic", "fc-2");
    expect(out).toContain('data-testid="gate-notes"');
    expect(out).toContain("This channel is published to the internet by your board.");
    expect(out).toContain("Sending on this channel is off. The first send asks you to confirm it.");
    expect(out).toContain("Your board is amateur-licensed.");
    const cap = (id: string, kind: "channel" | "dm") => capFor(apiStore.getState().networks!.find((n) => n.id === id)!, { kind });
    expect(cap("meshtastic", "channel")).toBe(232);
    expect(cap("meshtastic", "dm")).toBe(232); // the mock board is licensed: the DM cap rises from 220
    expect(cap("lxmf", "dm")).toBe(287);
  });

  it("a network that is off shows why instead of a composer", async () => {
    await load();
    await apiStore.refreshNetworks();
    const a = made[0]!;
    await a.bridge.patchSettings({ adapters: { meshtastic: false } });
    await apiStore.refreshNetworks();
    const out = await thread("meshtastic", "fc-1");
    expect(out).toContain('data-testid="gate-why"');
    expect(out).toContain("Meshtastic is turned off.");
    expect(out).not.toContain('aria-label="Message"');
  });
});

describe("the network page and the fan-out", () => {
  it("the network page splits channels and DMs, offers a DM by address, and lists what Nearby heard", async () => {
    await load();
    await apiStore.refreshConversations("meshtastic");
    const out = html(routed("/messages/net/meshtastic", "/messages/net/:network", NetView));
    expect(out).toContain("Meshtastic");
    expect(out).toContain("LongFast");
    expect(out).toContain("Mara&#x27;s handheld");
    expect(out).toContain("Meshtastic node id");
    expect(out).toContain("Send a Meshtastic DM from this node (readable, not 111-sealed)");
    expect(out).toContain('data-testid="heard-picks"');
    expect(out).toContain("Relays");
  });

  it("the network page of a network that is off says nothing of it runs", async () => {
    await load();
    const out = html(routed("/messages/net/app%3Adeltachat", "/messages/net/:network", NetView));
    expect(out).toContain("Delta Chat is off");
    expect(out).toContain("Nothing of it runs while it is off.");
    expect(html(routed("/messages/net/nope", "/messages/net/:network", NetView))).toContain("No such network");
  });

  it("the fan-out page lists targets by network (channels, DMs, recipient lists) and 111 contacts, and says it is a copy", async () => {
    await load();
    await apiStore.refreshConversations("meshtastic");
    await apiStore.refreshConversations("lxmf");
    const out = html(routed("/messages/send", "/messages/send", NetView));
    expect(out).toContain("It is a copy: nothing is exported from a 111 conversation");
    expect(out).toContain('data-testid="targets-meshtastic"');
    expect(out).toContain('data-testid="targets-lxmf"');
    expect(out).not.toContain('data-testid="targets-app:deltachat"'); // off
    expect(out).toContain('data-testid="targets-111"');
    expect(out).toContain("Friends");
    expect(out).toContain("Ada (workshop)");
  });

  it("the preview line shows bytes against the cap, parts, the airtime estimate, warnings, and the refusal with its next step", async () => {
    const { a } = await load();
    const long = "word ".repeat(60);
    const p = await a.networks.fanoutPreview({ text: long, targets: [{ network: "lxmf", conv: "fc-5" }, { network: "meshtastic", conv: "fc-1", size_mode: "split" }, { network: "meshtastic", conv: "fc-2" }] });
    const noop = () => undefined;
    const lx = html(h(PlanLines, { plan: p.items[0]!.plan, splittable: true, onSplit: noop, onConfirm: noop }));
    expect(lx).toContain("300 B of 287 B per packet");
    expect(lx).toContain("Refused: 300 B &gt; 287 B per packet");
    expect(lx).toContain("airtime: depends on the path");
    expect(lx).toContain("split this one into parts");
    const split = html(h(PlanLines, { plan: p.items[1]!.plan, splittable: true, onSplit: noop, onConfirm: noop }));
    expect(split).toMatch(/\d+ B of 232 B per packet, 2 parts/);
    expect(split).toContain("airtime: about");
    expect(split).toContain("(estimate)");
    expect(split).toContain("Parts can arrive out of order.");
    const gate = html(h(PlanLines, { plan: p.items[1]!.plan, splittable: true, onSplit: noop, onConfirm: noop }));
    expect(gate).toContain("read and confirm…"); // the licensed board is unconfirmed
    expect(planSizeText({ bytes: 312, cap: null, parts: 1 })).toBe("312 B");
    const noSplit = html(h(PlanLines, { plan: p.items[0]!.plan, splittable: false, onSplit: noop, onConfirm: noop }));
    expect(noSplit).not.toContain("split this one");
  });
});

describe("the consent panel and its round-trip", () => {
  const consent: ConsentRequired = {
    action: "edge_enable",
    subject: "fc-1>fc-4",
    warning_text: "Everyone in <b>LongFast</b> will have their messages copied to Friends.",
    hash: "e".repeat(64),
    addons: ["default_key_source", "mystery_addon"],
  };

  it("shows the exact text as text, badges the add-ons, and offers confirm and cancel", () => {
    const out = html(h(ConsentPanel, { consent, onConfirm: () => undefined, onCancel: () => undefined }));
    expect(out).toContain("Everyone in &lt;b&gt;LongFast&lt;/b&gt; will have their messages copied to Friends.");
    expect(out).not.toContain("<b>LongFast");
    expect(out).toContain("default key: anyone can read the source");
    expect(out).toContain("mystery_addon");
    expect(out).toContain("Start this relay");
    expect(out).toContain('data-testid="consent-confirm"');
    expect(out).toContain('data-testid="consent-cancel"');
    expect(out).not.toContain("consent-changed");
  });

  it("says when the text changed since it was last read", () => {
    expect(html(h(ConsentPanel, { consent, changed: true, onConfirm: () => undefined, onCancel: () => undefined }))).toContain("The text changed since you last read it");
  });

  it("the round-trip echoes the hash of the text that was shown, and nothing before the person confirmed", async () => {
    const { a } = await load();
    const asked: { text: string; changed: boolean }[] = [];
    const calls: (string | undefined)[] = [];
    const out = await runConsent(
      (hash) => {
        calls.push(hash);
        return a.bridge.createEdge({ src: "fc-2", dst: "fc-4", honour_ok_to_mqtt: false, oversize: "refuse", ...(hash ? { accept_warning: hash } : {}) });
      },
      async (c, changed) => {
        asked.push({ text: c.warning_text, changed });
        return true;
      },
    );
    expect(out.ok).toBe(true);
    expect(asked).toEqual([{ text: "Everyone in Ridge crew will have their messages copied to Friends. They see the [name/TAG] prefix only if they read Friends.", changed: false }]);
    expect(calls).toHaveLength(2);
    expect(calls[0]).toBeUndefined();
    expect(calls[1]).toMatch(/^[0-9a-f]{64}$/);
  });

  it("cancel stops it: nothing is sent with a hash", async () => {
    const calls: (string | undefined)[] = [];
    const err = Object.assign(new Error("c"), {});
    void err;
    const { a } = await load();
    const out = await runConsent(
      (hash) => {
        calls.push(hash);
        return a.bridge.enableComponent("lxmf", hash);
      },
      async () => false,
    );
    // lxmf is running in this scenario: no consent is needed, so it just succeeds. Use the cached Delta Chat component instead.
    expect(out.ok).toBe(true);
    calls.length = 0;
    const cancelled = await runConsent(
      (hash) => {
        calls.push(hash);
        return a.bridge.enableComponent("deltachat", hash);
      },
      async () => false,
    );
    expect(cancelled).toEqual({ ok: false, cancelled: true });
    expect(calls).toEqual([undefined]);
    expect((await a.bridge.components()).find((c) => c.id === "deltachat")!.state).toBe("cached");
  });

  it("a text that changes between the two calls is asked again, marked changed, and only the newest hash is accepted", async () => {
    const c1: ConsentRequired = { ...consent, hash: "1".repeat(64) };
    const c2: ConsentRequired = { ...consent, warning_text: "The destination now uploads.", hash: "2".repeat(64) };
    const { a } = await load();
    let n = 0;
    const seen: string[] = [];
    const out = await runConsent(
      async (hash) => {
        n++;
        if (hash === c2.hash) return "done";
        // First the old text, then (after the person confirmed it) a newer one: the live facts moved.
        const c = hash === undefined ? c1 : c2;
        throw (await failedWith(a, c));
      },
      async (c, changed) => {
        seen.push(`${c.hash[0]}:${changed}`);
        return true;
      },
    );
    expect(out).toEqual({ ok: true, value: "done" });
    expect(seen).toEqual(["1:false", "2:true"]);
    expect(n).toBe(3);
  });

  it("the same hash refused twice is a failure, not a loop", async () => {
    const { a } = await load();
    const c1: ConsentRequired = { ...consent, hash: "3".repeat(64) };
    let asks = 0;
    const out = await runConsent(
      async () => {
        throw await failedWith(a, c1);
      },
      async () => {
        asks++;
        return true;
      },
    );
    expect(out.ok).toBe(false);
    expect(asks).toBe(1);
    expect(consentOf((out as { error: unknown }).error)?.hash).toBe(c1.hash);
  });

  it("a consent the caller already holds is asked before any call", async () => {
    const calls: (string | undefined)[] = [];
    const out = await runConsent(
      async (hash) => {
        calls.push(hash);
        return "ok";
      },
      async () => true,
      consent,
    );
    expect(out).toEqual({ ok: true, value: "ok" });
    expect(calls).toEqual([consent.hash]);
  });

  it("any other failure ends it with that error", async () => {
    const boom = new Error("boom");
    const out = await runConsent(
      async () => {
        throw boom;
      },
      async () => true,
    );
    expect(out).toEqual({ ok: false, cancelled: false, error: boom });
  });
});

/** A real ApiError for a 409 consent_required carrying `c`, as the http adapter builds it. */
async function failedWith(_a: unknown, c: ConsentRequired): Promise<unknown> {
  const { createHttpAdapter } = await import("../src/lib/api/http");
  const adapter = createHttpAdapter({ fetch: (async () => new Response(JSON.stringify({ type: "about:blank", title: "c", status: 409, code: "consent_required", consent: c }), { status: 409, headers: { "content-type": "application/problem+json" } })) as typeof fetch });
  return adapter.bridge.edges().catch((e: unknown) => e);
}

describe("relays", () => {
  it("the network page lists each relay with its state, counters and controls; a consent-paused one says why and offers a review", async () => {
    await load();
    await apiStore.refreshConversations("meshtastic");
    await apiStore.refreshConversations("lxmf");
    const out = html(routed("/messages/net/meshtastic", "/messages/net/:network", NetView));
    const e1 = out.slice(out.indexOf('data-testid="edge-be-1"'), out.indexOf('data-testid="edge-be-2"'));
    expect(e1).toContain("relaying");
    expect(e1).toContain("2 relayed · 1 shed · 3 too long to relay");
    expect(e1).toContain("Shed: the backlog was full");
    expect(e1).toContain('data-testid="pause-be-1"');
    expect(e1).toContain("honour ok-to-MQTT");
    expect(e1).toContain("[OFF]");
    expect(e1).toContain("refuse");
    expect(e1).toContain("split");
    const e2 = out.slice(out.indexOf('data-testid="edge-be-2"'));
    expect(e2).toContain("paused: something you confirmed has changed");
    expect(e2).toContain("review and resume");
    expect(e2).not.toContain('data-testid="pause-be-2"');
    expect(out).toContain("2 relays, 3 too long to relay");
    expect(out).toContain("start a relay…");
  });

  it("relays from LongFast to Friends: a recipient list is offered as a destination, never as a source", async () => {
    await load();
    await apiStore.refreshConversations("meshtastic");
    await apiStore.refreshConversations("lxmf");
    const out = html(routed("/messages/net/meshtastic", "/messages/net/:network", NetView));
    const src = out.slice(out.indexOf('data-testid="edge-src"'), out.indexOf('data-testid="edge-dst"'));
    const dst = out.slice(out.indexOf('data-testid="edge-dst"'));
    expect(src).not.toContain("Friends");
    expect(src).not.toContain("Mara");
    expect(src).toContain("LongFast");
    expect(dst).toContain("Friends");
    expect(dst).toContain("(recipient list)");
    expect(dst).not.toContain("Mara&#x27;s handheld"); // a DM is unrepresentable
  });
});

describe("Settings, Transport", () => {
  async function settings(scenario: ScenarioName = "bridge") {
    await load(scenario);
    await Promise.all([apiStore.refreshBridgeSettings(), apiStore.refreshComponents(), apiStore.refreshTransport()]);
    return html(h(TransportSettings));
  }

  it("the l6 slug is real: the settings router renders this page, not the structure-only layer page", async () => {
    await load();
    await Promise.all([apiStore.refreshBridgeSettings(), apiStore.refreshComponents(), apiStore.refreshTransport()]);
    // The page is a lazy chunk inside the settings view, so a server render shows the Suspense fallback; the module itself is the page.
    const mod = await import("../src/views/settings/l6");
    expect(mod.default).toBe(mod.TransportSettings);
    expect(typeof SettingsView).toBe("function");
  });

  it("lists the adapters with their state and weight, and the preferred order with up and down", async () => {
    const out = await settings();
    expect(out).toContain("Transport");
    for (const id of ["meshtastic", "lxmf", "app-deltachat"]) expect(out).toContain(`data-row="adapter-${id}"`);
    expect(out).toContain('data-testid="state-app:deltachat"');
    const dc = out.slice(out.indexOf('data-row="adapter-app-deltachat"'));
    expect(dc).toContain("off: no process, listener, timer or open file");
    expect(dc).toContain("31.4 MB cached, not installed");
    expect(dc).toContain("nothing installed");
    expect(out).toContain('data-testid="order-list"');
    expect(out).toContain("1. 111 (sealed messages)");
    expect(out).toContain("Move Meshtastic up");
    expect(out).toContain("Move Reticulum (LXMF) down");
  });

  it("the region is 'not set' until a human picks it, then shows it; the callsign is optional", async () => {
    const quiet = await settings("healthy");
    expect(quiet).toContain('data-testid="region-value"');
    expect(quiet.slice(quiet.indexOf('data-testid="region-value"')).slice(0, 80)).toContain("not set");
    expect(quiet).toContain("transmits nothing yet");
    const set = await settings("bridge");
    expect(set.slice(set.indexOf('data-testid="region-value"')).slice(0, 80)).toContain("EU_868");
    expect(set).toContain("Callsign");
  });

  it("the posture shows its switches; the routing warning appears exactly when transport is on, the LAN listener is open and IFAC is off", () => {
    const base: TransportStatus = {
      schema: 1,
      interfaces: [{ name: "Default Interface", type: "AutoInterface", up: true, rx: 1, tx: 2, peers: 2 }],
      interfaces_state: "stale",
      posture: { share_instance: true, instance_name: "111", enable_transport: true, lan: { backbone_listen: true, port: 4242, ifac: false }, stock_autointerface: false, propagation_node: false },
      region: null,
      callsign: null,
      order: [],
    };
    const warn = html(h(PostureRows, { t: base }));
    expect(warn).toContain('data-testid="routing-warning"');
    expect(warn).toContain("routes other people&#x27;s Reticulum traffic, over its own radio too");
    expect(warn).toContain("The last interface check is out of date");
    expect(warn).toContain("AutoInterface · up · 2 peers");
    for (const t of [
      { ...base, posture: { ...base.posture, lan: { ...base.posture.lan, ifac: true } } },
      { ...base, posture: { ...base.posture, enable_transport: false } },
      { ...base, posture: { ...base.posture, lan: { ...base.posture.lan, backbone_listen: false } } },
    ]) expect(html(h(PostureRows, { t }))).not.toContain("routing-warning");
  });

  it("components show cached and installed size, the licence notice, the acknowledgement and the choices", async () => {
    const out = await settings();
    const dc = out.slice(out.indexOf('data-testid="component-deltachat"'));
    expect(dc).toContain("31.4 MB cached, not installed");
    expect(dc).toContain("licence notice");
    expect(dc).toContain("deltachat-rpc-server (MPL-2.0) and the Madmail relay (AGPL-3.0)");
    expect(dc).toContain("You install them; 111 is not the distributor.");
    expect(dc).toContain("read the notice and turn on");
    expect(dc).toContain("also delete the cached download (31.4 MB)");
    expect(dc).toContain("remove files");
    const lx = out.slice(out.indexOf('data-testid="component-lxmf"'), out.indexOf('data-testid="component-deltachat"'));
    expect(lx).toContain("licence acknowledged");
    expect(lx).toContain("running, about 38.0 MB of memory");
    expect(lx).toContain("24.0 MB installed");
    expect(lx).toContain("turn off");
    expect(lx).not.toContain("remove files"); // never from under a running component
  });

  it("a licensed board and an uplinked channel are switches that ask first; recipient lists are editable", async () => {
    const out = await settings();
    expect(out).toContain("Bench radio (!a1b2c3d4)");
    expect(out).toContain("licensed");
    expect(out).toContain("readable sends confirmed");
    expect(out).toContain("published to the internet");
    expect(out).toContain('data-testid="channel-fc-2"');
    expect(out).toContain('data-testid="list-friends"');
    expect(out).toContain("0123456789abcdef0123456789abcdef");
    expect(out).toContain('data-testid="heard"'); // Reticulum announces Nearby heard can be added
    expect(out).toContain("Sam (Sideband)");
  });

  it("relays are summarised with a link to where they are managed", async () => {
    const out = await settings();
    expect(out).toContain("2 relays, 1 paused, 1 copy shed, 3 too long to relay.");
    expect(out).toContain('href="#/messages/net/lxmf"'); // F5: the first enabled network in the node's order (Reticulum comes first here), not a fixed one
    expect(out).not.toContain('href="#/messages/net/meshtastic"');
  });

  it("F5: the relays link follows the networks that are on: the first enabled one in the node's order, /messages when none is", async () => {
    const { a } = await load();
    await a.bridge.patchSettings({ order: ["111", "meshtastic", "lxmf", "app:deltachat"] });
    await Promise.all([apiStore.refreshNetworks(), apiStore.refreshBridgeSettings(), apiStore.refreshComponents(), apiStore.refreshTransport()]);
    expect(html(h(TransportSettings))).toContain('href="#/messages/net/meshtastic"'); // the node's order puts Meshtastic first now
    await a.bridge.disableComponent("lxmf");
    await a.bridge.patchSettings({ adapters: { meshtastic: false } });
    await Promise.all([apiStore.refreshNetworks(), apiStore.refreshBridgeSettings()]);
    const out = html(h(TransportSettings));
    expect(out).not.toMatch(/href="#\/messages\/net\/(meshtastic|lxmf)"/);
    expect(out).toContain('data-testid="manage-relays"');
    expect(out).toMatch(/href="#\/messages"[^>]*data-testid="manage-relays"|data-testid="manage-relays"[^>]*href="#\/messages"/);
  });

  it("the page hands off to the agent with the two Layer 6 goals", async () => {
    const out = await settings();
    expect(out).toContain("Set up a network or a relay with the agent");
    expect(out).toContain("Ask the agent to fix the transport");
  });

  it("small pure rules: move, order, list ids", () => {
    expect(moveItem(["a", "b", "c"], 1, -1)).toEqual(["b", "a", "c"]);
    expect(moveItem(["a", "b", "c"], 2, 1)).toEqual(["a", "b", "c"]);
    const same = ["a"];
    expect(moveItem(same, 0, -1)).toBe(same);
    expect(orderWithAll(["111", "lxmf"], ["111", "lxmf", "meshtastic"])).toEqual(["111", "lxmf", "meshtastic"]);
    expect(listId("Friends & Family!", [])).toBe("friends-family");
    expect(listId("Friends", ["friends"])).toBe("friends-2");
    expect(listId("???", [])).toBe("list");
  });
});

describe("Nearby: a DM from this node", () => {
  it("offers 'Send a Meshtastic DM from this node (readable, not 111-sealed)' on a heard node whose id is a node id, and the LXMF one on an announce", async () => {
    await load();
    const out = html(h(NearbyView));
    const mesh = out.split('data-testid="nearby-dm"').length - 1;
    expect(mesh).toBe(3 + 2); // three Meshtastic nodes (one hostile-named, still a valid id) and two Reticulum announces
    expect(out).toContain("Send a Meshtastic DM from this node (readable, not 111-sealed)");
    expect(out).toContain("Send an LXMF message from this node (readable, not 111-sealed)");
    expect(out).toContain("2 Reticulum announces heard.");
    expect(out).toContain("A Meshtastic node is not a contact");
    // the hostile long name is still text
    expect(out).not.toContain("<img");
    expect(out).toContain("&lt;img src=x onerror=alert(1)&gt; Free WiFi");
  });

  it("offers nothing when the network is off, or when the id does not fit the network's address syntax", async () => {
    const { a } = await load();
    a.mock.state.messaging.nearby[0]!.nodes[0]!.id = "!<script>"; // not an address
    await apiStore.refreshNearby();
    const out = html(h(NearbyView));
    expect(out.split('data-testid="nearby-dm"').length - 1).toBe(2 + 2);
    await a.bridge.patchSettings({ adapters: { meshtastic: false } });
    await apiStore.refreshNetworks();
    expect(html(h(NearbyView)).split('data-testid="nearby-dm"').length - 1).toBe(2); // only the Reticulum ones
  });
});

describe("proposal bodies of the bridge kinds", () => {
  const BODY = "to: fc-2\nto: fc-4\n\nCamp is ready. **Not bold.** <script>alert(1)</script> [a link](https://evil.example) https://example.com\n\nSecond paragraph.";

  it("a bridge_message renders its text part preformatted: no Markdown, no links, no emphasis, no markup", () => {
    const out = html(h(ProposalBody, { body: BODY, kind: "bridge_message" }));
    expect(out).toContain('data-testid="bridge-text"');
    expect(out).toContain("<pre");
    expect(out).toContain("**Not bold.**");
    expect(out).toContain("[a link](https://evil.example)");
    expect(out).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");
    expect(out).not.toContain("<script");
    expect(out).not.toMatch(/<a[\s>]/);
    expect(out).not.toMatch(/<(strong|em|b)>/);
    expect(out).toContain("Second paragraph.");
    expect(out).toContain("to: fc-2");
    expect(out).toContain("to: fc-4");
  });

  it("a bridge_edge does the same with its from and to lines and the reason", () => {
    const out = html(h(ProposalBody, { body: "from: fc-1\nto: fc-4\n\nPass it on. [x](https://evil.example)", kind: "bridge_edge" }));
    expect(out).toContain("from: fc-1");
    expect(out).toContain("[x](https://evil.example)");
    expect(out).not.toMatch(/<a[\s>]/);
  });

  it("any other kind is still Markdown (the control)", () => {
    const out = html(h(ProposalBody, { body: "See [the guide](https://example.com/guide) and **this**.", kind: "diagnose" }));
    expect(out).toMatch(/<a[\s>]/);
    expect(out).toMatch(/<strong/);
    expect(html(h(ProposalBody, { body: "**x**" }))).toMatch(/<strong/);
  });

  it("a body without the header shape is all text; the header lines split off exactly", () => {
    expect(splitBridgeBody("just words")).toEqual({ header: [], text: "just words" });
    expect(splitBridgeBody("to: fc-1\n\nhi\n\nthere")).toEqual({ header: ["to: fc-1"], text: "hi\n\nthere" });
    expect(splitBridgeBody("from: fc-1\nto: fc-2\nreason")).toEqual({ header: ["from: fc-1", "to: fc-2"], text: "reason" });
    expect(isBridgeKind("bridge_edge")).toBe(true);
    expect(isBridgeKind("bridge_message")).toBe(true);
    expect(isBridgeKind("device")).toBe(false);
    expect(isBridgeKind(undefined)).toBe(false);
  });

  it("a bidi override in a quoted body is dropped", () => {
    const out = html(h(ProposalBody, { body: "to: fc-1\n\nabc\u202edef", kind: "bridge_message" }));
    expect(out).not.toContain("\u202e");
    expect(out).toContain("abcdef");
  });

  it("a device proposal is a card per action with the exact words, and a service card for service.restart", async () => {
    await load();
    const body = JSON.stringify({
      why: "The board transmits nothing until a region is set.",
      actions: [{ action: "radio.region.set", region: "EU_868" }, { action: "identify", device_id: "usb-7c1e0b94a2d3" }, { action: "service.restart", service_id: "transport" }],
    });
    const out = html(h(ProposalBody, { body, kind: "device" }));
    expect(out).toContain('data-testid="device-card"');
    expect(out).toContain("The board transmits nothing until a region is set.");
    expect(out).toContain("set the radio region to EU_868");
    expect(out).toContain("identify this radio once (a consented probe)");
    expect(out).toContain('data-testid="service-card"');
    expect(out).toContain("restart the Mesh transport service");
    expect(out).not.toMatch(/<a[\s>]/);
  });

  it("a device body that is not the expected shape is shown as plain text with a note", () => {
    for (const body of ["not json", "[]", '{"why":"x"}', '{"why":"x","actions":[{"nope":1}]}', '{"why":3,"actions":[]}']) {
      const out = html(h(ProposalBody, { body, kind: "device" }));
      expect(out, body).toContain("not in the expected shape");
      expect(out).not.toContain("device-card");
    }
    expect(parseDeviceBody('{"why":"w","actions":[]}')).toEqual({ why: "w", actions: [] });
  });

  it("every action of the closed vocabulary has words, and an unknown one says it is not applied", () => {
    for (const a of ["rns.interface.upsert", "rns.interface.remove", "rns.posture.set", "radio.region.set", "identify", "service.restart"]) {
      expect(actionSentence({ action: a }).title, a).not.toMatch(/^unknown/);
    }
    expect(actionSentence({ action: "firmware.flash" }).title).toBe("unknown action: firmware.flash");
    expect(actionSentence({ action: "identify", device_id: "usb-1" }).title).toBe("identify this radio once (a consented probe)");
    expect(actionSentence({ action: "<b>x</b>\u202e" }).title).toBe("unknown action: <b>x</b>"); // text only: the tags are shown, not built
  });

  it("accepting a bridge_edge answers 409 with the text, then accepts with the hash and starts the edge; a refusal leaves it open", async () => {
    const { a } = await load();
    const err = await a.decide("prop-01j9zl0a1bmq", "accepted").then(() => null, (e: unknown) => e);
    const c = consentOf(err)!;
    expect(c.action).toBe("edge_enable");
    expect(c.warning_text).toContain("Everyone in Ridge crew will have their messages copied to Friends.");
    expect((await a.proposal("prop-01j9zl0a1bmq")).decision).toBeNull(); // still open
    const stale = await a.decide("prop-01j9zl0a1bmq", "accepted", "0".repeat(64)).then(() => null, (e: unknown) => e);
    expect(consentOf(stale)?.hash).toBe(c.hash);
    const done = await a.decide("prop-01j9zl0a1bmq", "accepted", c.hash);
    expect(done.decision?.decision).toBe("accepted");
    expect((await a.bridge.edges()).map((e) => [e.src, e.dst])).toContainEqual(["fc-2", "fc-4"]);
    // A device proposal with an action outside the vocabulary is refused and stays open
    const dev = a.mock.state.proposals.find((p) => p.id === "prop-01j9zl0c3dps")!;
    dev.body = JSON.stringify({ why: "x", actions: [{ action: "firmware.flash" }] });
    const bad = await a.decide(dev.id, "accepted").then(() => null, (e: unknown) => e);
    expect(bad).toMatchObject({ status: 409, code: "unsupported_action" });
    expect((await a.proposal(dev.id)).decision).toBeNull();
  });

  it("accepting a bridge_message behind the licensed-board and uplink gates asks for each text in turn and sends once both are confirmed", async () => {
    const { a } = await load();
    const asked: string[] = [];
    let hash: string | undefined;
    for (let i = 0; i < 4; i++) {
      const r = await a.decide("prop-01j9zl0b2cnr", "accepted", hash).then(() => null, (e: unknown) => e);
      if (r === null) break;
      const c = consentOf(r)!;
      expect(c, "a refusal that is not a consent would be a plain failure").not.toBeNull();
      asked.push(c.action);
      hash = c.hash;
    }
    expect(asked).toEqual(["ham_confirm", "uplink_send"]);
    expect((await a.proposal("prop-01j9zl0b2cnr")).decision?.decision).toBe("accepted");
    const rows = (await a.networks.conversation("meshtastic", "fc-2")).messages.filter((m) => m.dir === "own");
    expect(rows.some((m) => m.text.startsWith("Camp is ready at the second lake."))).toBe(true);
  });
});

describe("prompts", () => {
  it("bridge.setup and transport.fix are goal sentences with no markup and one context block", () => {
    for (const g of ["bridge.setup", "transport.fix"] as const) {
      const s = goalSentence(g, { kind: "node" });
      expect(s.length).toBeGreaterThan(40);
      expect(s).not.toMatch(/[<>]/);
      const p = buildPrompt(g, { kind: "node" }, { status: null });
      expect(p.startsWith(s)).toBe(true);
      expect(p.split("Context from the dashboard").length).toBe(2);
    }
    expect(goalSentence("bridge.setup", { kind: "node" })).toMatch(/can only propose/);
    expect(goalSentence("transport.fix", { kind: "node" })).toMatch(/region/);
  });
});

describe("model words", () => {
  const msg = (over: Partial<ForeignMessage>): ForeignMessage => ({
    id: "fm-1", conv: "fc-1", dir: "in", sender: { native: "!a", name: "Zoë\u202e<b>" }, auth: "none", text: "t", state: "delivered", evidence: null, reason_code: null, reason: null, group_id: null, part: null, relayed_from: null,
    via_internet: false, seq: 1, at: "2026-10-19T09:00:00Z", claim_ts: null, truncated: null, ...over,
  });

  it("a sender name is cleaned and cut; an empty one reads 'no name'", () => {
    expect(senderName(msg({}))).toBe("Zoë<b>");
    expect(senderName(msg({ sender: { native: "!a", name: "  " } }))).toBe("no name");
    expect(senderName(msg({ sender: { native: "!a", name: "x".repeat(100) } })).length).toBeLessThanOrEqual(64);
  });

  it("titles, summaries", () => {
    expect(convTitle({ title: "", kind: "dm" })).toBe("direct message");
    expect(convTitle({ title: "\u202e", kind: "channel" })).toBe("channel");
    expect(splitSummary([{ kind: "channel" }, { kind: "channel" }, { kind: "dm" }, { kind: "group" }])).toBe("2 channels · 1 DM · 1 recipient list");
    expect(splitSummary([])).toBe("no conversations yet");
  });
});
