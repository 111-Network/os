// Layer 6 data layer (os/60-transport/README.md section 10): the mock's three adapters validated against the contract digest, the
// consent round-trip, relay edges, optional components, HAM and uplink gates, fan-out, transport, and the store's slices and frames
// (`network_message` per conversation, `bridge`). Fixtures are the mock's; the contract digest is the judge.
import { createHash } from "node:crypto";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Adapter } from "../src/lib/api/adapter";
import { confirmGate, consentOf, readConsent, refusalsOf } from "../src/lib/api/bridge";
import { createHttpAdapter } from "../src/lib/api/http";
import { MOCK_PAIR_CODE, SCENARIO_NAMES, createMockAdapter, type MockAdapter, type ScenarioName } from "../src/lib/api/mock";
import { sha256Hex } from "../src/lib/api/mock/sha256";
import {
  applyForeignTail,
  channelsOf,
  dmsOf,
  foreignPath,
  groupParts,
  lowestState,
  mergeForeignPage,
  orderedNetworks,
  paramText,
  summarizeParts,
  upsertForeign,
} from "../src/lib/api/networks";
import { ApiError } from "../src/lib/api/problem";
import { ApiStore, apiStore } from "../src/lib/api/store";
import type { EventsFrameWithId } from "../src/lib/api/adapter";
import type { ForeignMessage } from "../src/lib/api/types";
import { countsAsUnread, foreignUnread } from "../src/lib/attention";
import { operations } from "./contract.gen";
import { FakeClock, flush, jsonResponse, problemResponse } from "./helpers/fakes";
import { validate, validateAgainst } from "./helpers/validate";

const T0 = Date.parse("2026-10-19T09:00:00Z");
const made: MockAdapter[] = [];
afterEach(() => {
  apiStore.detach();
  apiStore.reset();
  for (const a of made.splice(0)) a.mock.dispose();
});

function make(scenario: ScenarioName) {
  const clock = new FakeClock(T0);
  const a = createMockAdapter({ scenario, clock, tokenDelayMs: 0 });
  made.push(a);
  return { a, clock };
}

/** A scenario whose visitor must pair first is paired, like a person who entered the code. */
async function paired(scenario: ScenarioName) {
  const r = make(scenario);
  if (scenario === "lan-unpaired" || scenario === "termux-unpaired") await r.a.auth.pair(MOCK_PAIR_CODE);
  return r;
}

const failure = async (p: Promise<unknown>): Promise<ApiError> => {
  const e = await p.then(
    () => null,
    (x: unknown) => x,
  );
  expect(e).toBeInstanceOf(ApiError);
  return e as ApiError;
};

function expectValid(opId: string, value: unknown) {
  const schema = operations[opId]!.schemasByType["application/json"];
  expect(schema, `${opId} has a JSON response`).toBeTruthy();
  expect(validate(value, schema!), opId).toEqual([]);
}

const sha = (s: string): string => createHash("sha256").update(s, "utf8").digest("hex");

describe("the oneOf-aware validator has teeth", () => {
  it("a null where a schema is wanted, and a wrong shape where oneOf lists two, are caught", () => {
    const good = { target: { network: "meshtastic", conv: "fc-1" }, plan: { bytes: 1, cap: 2, parts: 1, size_mode: "refuse", airtime_estimate: null, refusal: null, warnings: [] } };
    expect(validateAgainst("FanoutPreviewItem", good)).toEqual([]);
    // oneOf [object with required fields, null]: a string matches neither branch
    expect(validateAgainst("FanoutPreviewItem", { ...good, plan: { ...good.plan, airtime_estimate: "soon" } })).not.toEqual([]);
    expect(validateAgainst("FanoutPreviewItem", { ...good, plan: { ...good.plan, airtime_estimate: { millis: 5 } } })).not.toEqual([]); // label missing
    // the fan-out target union: neither a conversation nor a contact
    expect(validateAgainst("FanoutPreviewItem", { ...good, target: { nope: 1 } })).not.toEqual([]);
    expect(validateAgainst("FanoutPreviewItem", { ...good, target: { node_id: "ab".repeat(32) } })).toEqual([]);
  });
});

describe.each(SCENARIO_NAMES)("scenario %s: the Layer 6 routes validate against openapi.yaml", (scenario) => {
  it("networks, conversations and their messages, edges, settings, components, transport", async () => {
    const { a } = await paired(scenario);
    const networks = await a.networks.list();
    expectValid("listNetworks", { schema: 1, networks });
    expect(networks.map((n) => n.id)).toEqual(["lxmf", "meshtastic", "app:deltachat"]);
    for (const n of networks) {
      const convs = await a.networks.conversations(n.id);
      expectValid("listForeignConversations", { schema: 1, network: n.id, conversations: convs });
      for (const c of convs) {
        const t = await a.networks.conversation(n.id, c.id);
        expectValid("getForeignConversation", t);
        const seqs = t.messages.map((m) => m.seq);
        expect(seqs).toEqual([...seqs].sort((x, y) => x - y));
      }
    }
    expectValid("listBridgeEdges", { schema: 1, edges: await a.bridge.edges() });
    expectValid("getBridgeSettings", await a.bridge.settings());
    expectValid("listBridgeComponents", { schema: 1, components: await a.bridge.components() });
    expectValid("getTransport", await a.transport.status());
    expect((await failure(a.networks.conversations("nope"))).status).toBe(404);
  });

  it("every bridge and network_message frame validates against its x-sse-events schema", async () => {
    const { a, clock } = await paired(scenario);
    const frames: EventsFrameWithId[] = [];
    a.events({ onFrame: (f) => frames.push(f), onState: () => undefined });
    await flush();
    await clock.advanceAsync(12_000, 500);
    for (const f of frames.filter((x) => x.event === "bridge" || x.event === "network_message")) {
      expect(validateAgainst(f.event === "bridge" ? "SseBridge" : "SseNetworkMessage", f.data), f.event).toEqual([]);
    }
  });
});

describe("the bridge scenario", () => {
  it("has three adapters: Meshtastic live, Reticulum live with a recipient list, Delta Chat disabled and cached", async () => {
    const { a } = make("bridge");
    const byId = Object.fromEntries((await a.networks.list()).map((n) => [n.id, n]));
    expect(byId.meshtastic).toMatchObject({ state: "live", component: null, dropped: 3, gate: { region_set: true, licensed: true, ham_confirmed: false, uplink_channels: 1 } });
    expect(byId.lxmf).toMatchObject({ state: "live", component: { state: "running", acknowledged: true } });
    expect(byId["app:deltachat"]).toMatchObject({ state: "disabled", component: { state: "cached", cached_bytes: 31_400_000, installed_bytes: null } });
    const settings = await a.bridge.settings();
    expect(settings.recipient_lists).toEqual([expect.objectContaining({ network: "lxmf", name: "Friends", members: expect.any(Array) })]);
    const mesh = await a.networks.conversations("meshtastic");
    expect(channelsOf(mesh)).toHaveLength(2);
    expect(dmsOf(mesh)).toHaveLength(1);
    expect(mesh.find((c) => c.uplink === true)).toMatchObject({ title: "Ridge crew", send_enabled: false, pinned: true });
  });

  it("holds foreign messages in every state, each with evidence, plus inbound, relayed, truncated and handoff-pending rows", async () => {
    const { a } = make("bridge");
    const t = await a.networks.conversation("meshtastic", "fc-1");
    const own = t.messages.filter((m) => m.dir === "own");
    expect(new Set(own.map((m) => m.state))).toEqual(new Set(["queued", "sent", "relayed", "delivered", "failed"]));
    for (const m of own) expect(m.evidence, m.id).toBeTruthy();
    for (const m of t.messages.filter((x) => x.dir === "in")) expect(m).toMatchObject({ state: "delivered", evidence: null });
    expect(t.messages.some((m) => m.via_internet)).toBe(true);
    expect(t.messages.some((m) => m.truncated === 70_000)).toBe(true);
    expect(t.messages.some((m) => m.reason_code === "handoff_pending")).toBe(true);
    expect(t.messages.filter((m) => m.part).map((m) => m.part)).toEqual([{ k: 1, n: 3 }, { k: 2, n: 3 }, { k: 3, n: 3 }]);
    const copies = (await a.networks.conversation("lxmf", "fc-4")).messages.filter((m) => m.relayed_from);
    expect(copies.length).toBeGreaterThan(0);
    expect(copies[0]!.text).toMatch(/^\[[^\]/]+\/M\] /);
  });

  it("the script delivers a channel text and a DM as coalesced network_message frames (no text in them)", async () => {
    const { a, clock } = make("bridge");
    const frames: EventsFrameWithId[] = [];
    a.events({ onFrame: (f) => frames.push(f), onState: () => undefined });
    await flush();
    await clock.advanceAsync(12_000, 500);
    const nm = frames.filter((f) => f.event === "network_message").map((f) => f.data as { conv: string; change: string; id?: string; seq: number });
    expect(nm.map((f) => [f.conv, f.change])).toEqual([["fc-1", "new"], ["fc-3", "new"]]);
    for (const f of nm) {
      expect(f.id).toMatch(/^fm-/);
      expect(Object.keys(f).sort()).toEqual(["change", "conv", "id", "network", "seq"]);
    }
  });

  it("mints consent hashes the way the daemon does: SHA-256 of the exact text", async () => {
    expect(sha256Hex("")).toBe(sha(""));
    expect(sha256Hex("abc")).toBe(sha("abc"));
    expect(sha256Hex("Zoë ‮ <a href>".repeat(40))).toBe(sha("Zoë ‮ <a href>".repeat(40)));
    const { a } = make("bridge");
    for (const c of await a.bridge.components()) expect(c.notice_hash).toBe(sha(c.license.notice));
  });
});

describe("the consent round-trip (relay edges)", () => {
  it("answers 409 with the exact text and its hash, accepts that hash, refuses a stale one", async () => {
    const { a } = make("bridge");
    const body = { src: "fc-2", dst: "fc-4", honour_ok_to_mqtt: false, oversize: "refuse" as const };
    const err = await failure(a.bridge.createEdge(body));
    expect(err).toMatchObject({ status: 409, code: "consent_required" });
    expect(validateAgainst("Problem", err.problem)).toEqual([]);
    const c = consentOf(err)!;
    expect(c.action).toBe("edge_enable");
    expect(c.warning_text).toBe("Everyone in Ridge crew will have their messages copied to Friends. They see the [name/TAG] prefix only if they read Friends.");
    expect(c.hash).toBe(sha(c.warning_text));
    expect(validateAgainst("ConsentRequired", c)).toEqual([]);
    const stale = await failure(a.bridge.createEdge({ ...body, accept_warning: "0".repeat(64) }));
    expect(consentOf(stale)?.hash).toBe(c.hash);
    const edge = await a.bridge.createEdge({ ...body, accept_warning: c.hash });
    expect(edge).toMatchObject({ src: "fc-2", dst: "fc-4", paused: false, consent_hash: c.hash, counters: { relayed: 0, shed: 0, refused: 0 } });
    expectValid("createBridgeEdge", edge);
    expect(edge.cursor_seq).toBeGreaterThan(0); // history is never relayed
  });

  it("names the live add-ons: a default-key source, an uplinked destination, a licensed account", async () => {
    const { a } = make("bridge");
    await a.bridge.deleteEdge("be-2"); // LongFast -> Ridge crew: a default-key source into an uplinked channel of a licensed board
    const err = await failure(a.bridge.createEdge({ src: "fc-1", dst: "fc-2", honour_ok_to_mqtt: false, oversize: "refuse" }));
    expect(err.code).toBe("consent_required");
    const c = consentOf(err)!;
    expect(c.addons).toEqual(expect.arrayContaining(["default_key_source", "uplinked_destination", "licensed_destination"]));
    expect(c.warning_text).toContain("uses the default key: anyone can read it; this copies public traffic.");
    expect(c.warning_text).toContain("is published to the internet by your board");
  });

  it("refuses a DM endpoint, a recipient list as a source, the same conversation twice, and an existing pair", async () => {
    const { a } = make("bridge");
    expect((await failure(a.bridge.createEdge({ src: "fc-3", dst: "fc-4", honour_ok_to_mqtt: false, oversize: "refuse" }))).code).toBe("bad_request");
    expect((await failure(a.bridge.createEdge({ src: "fc-4", dst: "fc-1", honour_ok_to_mqtt: false, oversize: "refuse" }))).code).toBe("bad_request");
    expect((await failure(a.bridge.createEdge({ src: "fc-1", dst: "fc-1", honour_ok_to_mqtt: false, oversize: "refuse" }))).code).toBe("bad_request");
    expect((await failure(a.bridge.createEdge({ src: "fc-1", dst: "fc-4", honour_ok_to_mqtt: false, oversize: "refuse" }))).status).toBe(409);
    expect((await failure(a.bridge.createEdge({ src: "fc-1", dst: "fc-99", honour_ok_to_mqtt: false, oversize: "refuse" }))).status).toBe(404);
  });

  it("pause is always allowed and a user pause resumes without a confirmation", async () => {
    const { a } = make("bridge");
    expect(await a.bridge.patchEdge("be-1", { paused: true })).toMatchObject({ paused: true, pause_reason: "user" });
    expect(await a.bridge.patchEdge("be-1", { paused: false })).toMatchObject({ paused: false, pause_reason: null });
    expect(await a.bridge.patchEdge("be-1", { honour_ok_to_mqtt: true, oversize: "split" })).toMatchObject({ honour_ok_to_mqtt: true, oversize: "split" });
  });

  it("a consent-paused edge resumes only on the NEW text's hash", async () => {
    const { a } = make("bridge");
    const before = (await a.bridge.edge("be-2"))!;
    expect(before).toMatchObject({ paused: true, pause_reason: "consent_paused" });
    const err = await failure(a.bridge.patchEdge("be-2", { paused: false }));
    const c = consentOf(err)!;
    expect(c.action).toBe("edge_resume");
    expect(c.hash).not.toBe(before.consent_hash); // the live facts moved, so the text did
    expect(c.addons).toContain("uplinked_destination");
    expect((await failure(a.bridge.patchEdge("be-2", { paused: false, accept_warning: before.consent_hash! }))).code).toBe("consent_required");
    const resumed = await a.bridge.patchEdge("be-2", { paused: false, accept_warning: c.hash });
    expect(resumed).toMatchObject({ paused: false, pause_reason: null, consent_hash: c.hash });
  });

  it("deleting an edge removes it; the counters are the node's own", async () => {
    const { a } = make("bridge");
    expect((await a.bridge.edge("be-1")).counters).toEqual({ relayed: 2, shed: 1, refused: 3 });
    await a.bridge.deleteEdge("be-1");
    expect((await failure(a.bridge.edge("be-1"))).status).toBe(404);
  });
});

describe("HAM and uplink gates, oversize, split", () => {
  it("a licensed board's readable send is a 409 with the licensee text; confirming (with the hash) records it and the send then queues", async () => {
    const { a } = make("bridge");
    const err = await failure(a.networks.send("meshtastic", "fc-1", { text: "hello" }));
    const c = consentOf(err)!;
    expect(c).toMatchObject({ action: "ham_confirm", subject: "Bench radio (!a1b2c3d4)" });
    expect(c.hash).toBe(sha(c.warning_text));
    expect((await failure(a.bridge.patchSettings({ ham: [{ account: c.subject, confirmed: true }] }))).code).toBe("consent_required");
    await confirmGate(a, c, c.hash);
    expect((await a.bridge.settings()).ham).toEqual([{ account: "Bench radio (!a1b2c3d4)", licensed: true, confirmed: true }]);
    const ok = await a.networks.send("meshtastic", "fc-1", { text: "hello" });
    expectValid("sendForeignMessage", ok);
    expect(ok.id).toMatch(/^fm-/);
  });

  it("an uplinked channel's send switch is off by default, and turning it on needs its text's hash", async () => {
    const { a } = make("bridge");
    const ham = consentOf(await failure(a.networks.send("meshtastic", "fc-1", { text: "x" })))!;
    await confirmGate(a, ham, ham.hash);
    const err = await failure(a.networks.send("meshtastic", "fc-2", { text: "to the crew" }));
    const c = consentOf(err)!;
    expect(c).toMatchObject({ action: "uplink_send", subject: "fc-2" });
    expect(c.warning_text).toContain("this channel is published to the internet by your board");
    await confirmGate(a, c, c.hash);
    expect((await a.bridge.settings()).channels.find((x) => x.conv === "fc-2")).toMatchObject({ send: true, uplink: true });
    expect((await a.networks.send("meshtastic", "fc-2", { text: "to the crew" })).plan.warnings.map((w) => w.code)).toContain("uplink_channel");
  });

  it("turning a gate off needs no confirmation", async () => {
    const { a } = make("bridge");
    const s = await a.bridge.patchSettings({ channels: [{ conv: "fc-1", send: false }], ham: [] });
    expect(s.channels.find((c) => c.conv === "fc-1")?.send).toBe(false);
  });

  it("a text over the cap in refuse mode is 409 oversize with the plain sentence and queues nothing; split queues numbered parts", async () => {
    const { a } = make("bridge");
    const ham = consentOf(await failure(a.networks.send("meshtastic", "fc-1", { text: "x" })))!;
    await confirmGate(a, ham, ham.hash);
    const before = (await a.networks.conversation("meshtastic", "fc-1")).messages.length;
    const long = "word ".repeat(60); // 300 B
    const err = await failure(a.networks.send("meshtastic", "fc-1", { text: long }));
    expect(err).toMatchObject({ status: 409, code: "oversize" });
    expect(err.problem.detail).toBe("300 B > 232 B per packet");
    expect((await a.networks.conversation("meshtastic", "fc-1")).messages).toHaveLength(before);
    const ok = await a.networks.send("meshtastic", "fc-1", { text: long, size_mode: "split" });
    expect(ok.plan.parts).toBeGreaterThan(1);
    expect(ok.ids).toHaveLength(ok.plan.parts);
    const rows = (await a.networks.conversation("meshtastic", "fc-1")).messages.filter((m) => ok.ids!.includes(m.id));
    expect(rows.map((r) => r.part)).toEqual(ok.ids!.map((_, i) => ({ k: i + 1, n: ok.plan.parts })));
    expect(rows.every((r) => r.group_id === rows[0]!.group_id)).toBe(true);
  });
});

describe("fan-out", () => {
  const NODE = "3a7f09c1d5e24b86a0f17c93e8d52b4670a9c1e3f58d2b64079a1c8e3d5f2a41"; // a mock contact

  it("previews every target with bytes, cap, parts, an airtime estimate labelled one, and refusals; writes nothing", async () => {
    const { a } = make("bridge");
    const mesh = (await a.networks.conversations("meshtastic")).length;
    const long = "word ".repeat(60);
    const p = await a.networks.fanoutPreview({ text: long, targets: [{ network: "meshtastic", conv: "fc-3" }, { network: "lxmf", conv: "fc-4" }, { network: "meshtastic", conv: "fc-1", size_mode: "split" }, { node_id: NODE }] });
    expectValid("previewFanout", p);
    expect(p.text_bytes).toBe(300);
    expect(p.ok).toBe(false);
    const [dm, list, split, contact] = p.items;
    expect(dm!.plan.refusal).toMatchObject({ code: "oversize", detail: "300 B > 232 B per packet" }); // a licensed board's DM takes 232 B
    expect(list!.plan).toMatchObject({ bytes: 300, cap: 287, refusal: { code: "oversize", detail: "300 B > 287 B per packet" } });
    expect(list!.plan.airtime_estimate).toEqual({ millis: null, label: "depends on the path" });
    expect(split!.plan.parts).toBeGreaterThan(1);
    expect(split!.plan.warnings.map((w) => w.code)).toContain("parts_out_of_order");
    expect(split!.plan.airtime_estimate!.label).toMatch(/estimate/);
    expect(contact!.plan).toMatchObject({ cap: null, parts: 1, refusal: null });
    expect((await a.networks.conversations("meshtastic")).length).toBe(mesh);
  });

  it("an oversize DM says its own cap: 220 B, or 232 B on a licensed board", async () => {
    const { a } = make("bridge");
    const p = await a.networks.fanoutPreview({ text: "x".repeat(226), targets: [{ network: "meshtastic", conv: "fc-3" }] });
    expect(p.items[0]!.plan.cap).toBe(232); // the mock board is licensed
  });

  it("sending is atomic: one refused target answers 409 with the per-target report and nothing is queued", async () => {
    const { a } = make("bridge");
    const sent = (await a.networks.conversation("lxmf", "fc-5")).messages.length;
    const err = await failure(a.networks.fanout({ text: "z".repeat(400), targets: [{ network: "lxmf", conv: "fc-5" }, { network: "meshtastic", conv: "fc-1" }] }));
    expect(err).toMatchObject({ status: 409 });
    expect(validateAgainst("Problem", err.problem)).toEqual([]);
    const report = refusalsOf(err);
    expect(report.map((r) => r.code).sort()).toEqual(["oversize", "oversize"]);
    expect(report[0]!.detail).toMatch(/B > \d+ B per packet/);
    expect((await a.networks.conversation("lxmf", "fc-5")).messages).toHaveLength(sent);
  });

  it("a consent gate is reported with the text to confirm; after confirming, the fan-out queues every target with its own row and state", async () => {
    const { a, clock } = make("bridge");
    const targets = [{ network: "meshtastic", conv: "fc-1" }, { network: "lxmf", conv: "fc-5" }, { node_id: NODE }];
    const err = await failure(a.networks.fanout({ text: "hello all", targets }));
    expect(err.code).toBe("consent_required");
    const gate = refusalsOf(err).find((r) => r.consent)!;
    expect(gate.consent).toMatchObject({ action: "ham_confirm" });
    await confirmGate(a, gate.consent!, gate.consent!.hash);
    const r = await a.networks.fanout({ text: "hello all", targets });
    expectValid("sendFanout", r);
    expect(r.group_id).toMatch(/^sg-/);
    expect(r.items.map((i) => i.id.slice(0, 3))).toEqual(["fm-", "fm-", "msg"]);
    await clock.advanceAsync(4_000, 100);
    const rows = (await a.networks.conversation("lxmf", "fc-5")).messages.filter((m) => m.id === r.items[1]!.id);
    expect(rows[0]!.state).toBe("delivered");
    const mesh = (await a.networks.conversation("meshtastic", "fc-1")).messages.find((m) => m.id === r.items[0]!.id)!;
    expect(mesh.state).toBe("relayed"); // a channel never reaches delivered
  });

  it("a 111 target that is not a contact is 404", async () => {
    const { a } = make("bridge");
    expect((await failure(a.networks.fanout({ text: "x", targets: [{ node_id: "ab".repeat(32) }] }))).status).toBe(404);
  });
});

describe("optional components (the licence notice flow)", () => {
  it("enable answers 409 with the notice and its hash; the hash installs; a wrong hash does not", async () => {
    const { a } = make("healthy");
    const c0 = (await a.bridge.components()).find((c) => c.id === "lxmf")!;
    expect(c0).toMatchObject({ state: "cached", cached_bytes: 9_000_000, installed_bytes: null, acknowledged: false });
    expectValid("listBridgeComponents", { schema: 1, components: [c0] });
    const err = await failure(a.bridge.enableComponent("lxmf"));
    const c = consentOf(err)!;
    expect(c).toMatchObject({ action: "component_enable", subject: "lxmf", warning_text: c0.license.notice, hash: c0.notice_hash });
    expect((await failure(a.bridge.enableComponent("lxmf", "1".repeat(64)))).code).toBe("consent_required");
    expect(await a.bridge.components().then((l) => l.find((x) => x.id === "lxmf")!.state)).toBe("cached"); // nothing installed on a bad hash
    const on = await a.bridge.enableComponent("lxmf", c.hash);
    expect(on).toMatchObject({ state: "running", acknowledged: true });
    expect(on.installed_bytes).toBeGreaterThan(0);
    expect((await a.networks.list()).find((n) => n.id === "lxmf")!.state).toBe("live");
  });

  it("disable keeps the files; remove-files refuses while running, deletes the tree, and the cached download only when asked", async () => {
    const { a } = make("bridge");
    expect((await failure(a.bridge.removeComponentFiles("lxmf"))).code).toBe("bad_request");
    expect(await a.bridge.disableComponent("lxmf")).toMatchObject({ state: "disabled", installed_bytes: expect.any(Number) });
    expect((await a.networks.list()).find((n) => n.id === "lxmf")!.state).toBe("disabled");
    expect(await a.bridge.removeComponentFiles("lxmf")).toMatchObject({ state: "cached", installed_bytes: null, cached_bytes: 9_000_000 });
    const gone = await a.bridge.removeComponentFiles("lxmf", true);
    expect(gone).toMatchObject({ state: "declared", cached_bytes: null });
    expect((await failure(a.bridge.enableComponent("lxmf", gone.notice_hash))).code).toBe("component_unavailable"); // nothing to install from: fails closed
  });

  it("an unknown component is 404", async () => {
    const { a } = make("bridge");
    expect((await failure(a.bridge.enableComponent("nope"))).status).toBe(404);
  });
});

describe("transport", () => {
  it("the region is unset until a human picks one; an unknown id is 400; the posture change that does not probe goes back", async () => {
    const { a } = make("healthy");
    expect((await a.transport.status()).region).toBeNull();
    expect((await failure(a.transport.patchPosture({ region: "MARS_900" }))).code).toBe("bad_request");
    const r = await a.transport.patchPosture({ region: "EU_868", callsign: "N0CALL" });
    expectValid("patchTransportPosture", r);
    expect(r).toMatchObject({ applied: true, reverted: false, status: { region: "EU_868", callsign: "N0CALL" } });
    expect((await a.networks.list()).find((n) => n.id === "meshtastic")!.state).toBe("live");
    const back = await a.transport.patchPosture({ posture: { propagation_node: true } });
    expect(back).toMatchObject({ applied: false, reverted: true, status: { posture: { propagation_node: false } } });
    expect(back.reason).toMatch(/30 s/);
  });

  it("the join descriptor needs the LAN listener; the passphrase is in the answer only with IFAC", async () => {
    const { a } = make("healthy");
    expect((await failure(a.transport.join())).code).toBe("bad_request");
    await a.transport.patchPosture({ posture: { lan: { backbone_listen: true } } });
    const open = await a.transport.join();
    expectValid("joinTransport", open);
    expect(open).not.toHaveProperty("ifac_passphrase");
    await a.transport.patchPosture({ posture: { lan: { ifac: true } } });
    expect(await a.transport.join()).toMatchObject({ ifac_name: expect.any(String), ifac_passphrase: expect.any(String) });
  });

  it("the preferred order is saved through bridge settings and listed back in that order", async () => {
    const { a } = make("bridge");
    await a.bridge.patchSettings({ order: ["111", "meshtastic", "lxmf", "app:deltachat"] });
    expect((await a.networks.list()).map((n) => n.id)).toEqual(["meshtastic", "lxmf", "app:deltachat"]);
    expect(orderedNetworks(await a.networks.list(), ["lxmf", "meshtastic"]).map((n) => n.id)).toEqual(["lxmf", "meshtastic", "app:deltachat"]);
  });
});

describe("retry and cancel of foreign rows go through the message routes", () => {
  it("cancel drops a queued row with nothing sent; a sent row is 409; retry resends a failed one", async () => {
    const { a, clock } = make("bridge");
    const rows = (await a.networks.conversation("meshtastic", "fc-1")).messages;
    const queued = rows.find((m) => m.state === "queued" && m.reason_code === "rate_limited")!;
    const sent = rows.find((m) => m.state === "sent" && m.dir === "own")!;
    const failed = rows.find((m) => m.state === "failed")!;
    await a.messages.cancel!(queued.id);
    expect((await a.networks.conversation("meshtastic", "fc-1")).messages.some((m) => m.id === queued.id)).toBe(false);
    expect((await failure(a.messages.cancel!(sent.id))).code).toBe("bad_request");
    await a.messages.retry(failed.id);
    await clock.advanceAsync(3_000, 100);
    expect((await a.networks.conversation("meshtastic", "fc-1")).messages.find((m) => m.id === failed.id)!.state).not.toBe("failed");
  });
});

describe("the consent helpers", () => {
  it("readConsent fails closed on a malformed object", () => {
    const ok = { action: "edge_enable", subject: "be-1", warning_text: "x", hash: "a".repeat(64), addons: [] };
    expect(readConsent(ok)).toEqual(ok);
    for (const bad of [null, "x", { ...ok, hash: "ABC" }, { ...ok, hash: "a".repeat(63) }, { ...ok, warning_text: "" }, { ...ok, action: "launch" }, { ...ok, subject: 3 }]) expect(readConsent(bad)).toBeNull();
    expect(readConsent({ ...ok, addons: ["x", 3] })!.addons).toEqual(["x"]);
  });

  it("consentOf reads only a consent_required problem; the extension members survive the http adapter", async () => {
    const consent = { action: "ham_confirm", subject: "Bench", warning_text: "Confirm?", hash: "b".repeat(64), addons: ["licensed_account"] };
    const http = createHttpAdapter({
      fetch: (async () => new Response(JSON.stringify({ type: "about:blank", title: "Confirm", status: 409, code: "consent_required", consent }), { status: 409, headers: { "content-type": "application/problem+json" } })) as typeof fetch,
    });
    const err = await failure(http.bridge.patchSettings({ ham: [{ account: "Bench", confirmed: true }] }));
    expect(consentOf(err)).toEqual(consent);
    expect(consentOf(new Error("x"))).toBeNull();
    const other = await failure(createHttpAdapter({ fetch: (async () => problemResponse(409, "bad_request", "No")) as typeof fetch }).bridge.edges());
    expect(consentOf(other)).toBeNull();
  });
});

describe("the http adapter's Layer 6 details", () => {
  const recorder = (respond: () => Response) => {
    const calls: { method: string; url: string; body: unknown }[] = [];
    const fetchFn = (async (input: RequestInfo | URL, init: RequestInit = {}) => {
      calls.push({ method: init.method ?? "GET", url: String(input), body: typeof init.body === "string" ? JSON.parse(init.body) : undefined });
      return respond();
    }) as typeof fetch;
    return { calls, adapter: createHttpAdapter({ fetch: fetchFn }) };
  };

  it("percent-encodes the colon of app:deltachat in every path and never puts a network id in a query", async () => {
    const { adapter, calls } = recorder(() => jsonResponse({ schema: 1, network: "app:deltachat", conversations: [], conversation: { id: "fc-9" }, messages: [], more: false }));
    await adapter.networks.conversations("app:deltachat");
    await adapter.networks.conversation("app:deltachat", "fc-9", { limit: 20 });
    await adapter.networks.send("app:deltachat", "fc-9", { text: "hi" }).catch(() => undefined);
    expect(calls.map((c) => c.url)).toEqual([
      "/api/v1/networks/app%3Adeltachat/conversations",
      "/api/v1/networks/app%3Adeltachat/conversations/fc-9?limit=20",
      "/api/v1/networks/app%3Adeltachat/conversations/fc-9/messages",
    ]);
  });

  it("a decision carries accept_warning only when one is given", async () => {
    const { adapter, calls } = recorder(() => jsonResponse({}));
    await adapter.decide("prop-1", "accepted");
    await adapter.decide("prop-1", "accepted", "c".repeat(64));
    expect(calls.map((c) => c.body)).toEqual([{ decision: "accepted" }, { decision: "accepted", accept_warning: "c".repeat(64) }]);
  });

  it("component enable and remove-files send the hash and the cache choice; the body is JSON even when empty", async () => {
    const { adapter, calls } = recorder(() => jsonResponse({}));
    await adapter.bridge.enableComponent("lxmf");
    await adapter.bridge.enableComponent("lxmf", "d".repeat(64));
    await adapter.bridge.removeComponentFiles("lxmf", true);
    expect(calls.map((c) => c.body)).toEqual([{}, { accept_notice_hash: "d".repeat(64) }, { cache: true }]);
  });

  it("a conversation reply without messages is a bad response, never an empty thread", async () => {
    const { adapter } = recorder(() => jsonResponse({ schema: 1 }));
    expect((await failure(adapter.networks.conversation("meshtastic", "fc-1"))).code).toBe("upstream_error");
    expect((await failure(adapter.networks.list())).code).toBe("upstream_error");
  });

  it("the join answer is requested uncached", async () => {
    const seen: RequestInit[] = [];
    const adapter = createHttpAdapter({
      fetch: (async (_u: RequestInfo | URL, init: RequestInit = {}) => {
        seen.push(init);
        return jsonResponse({ schema: 1, address: "10.0.0.2", port: 4242 }, 200, { "cache-control": "no-store" });
      }) as typeof fetch,
    });
    await adapter.transport.join();
    expect(seen[0]).toMatchObject({ method: "POST", cache: "no-store" });
  });
});

describe("pure helpers", () => {
  const row = (over: Partial<ForeignMessage>): ForeignMessage => ({
    id: "fm-1", conv: "fc-1", dir: "own", sender: { native: "!a", name: "" }, auth: "none", text: "t", state: "queued", evidence: null, reason_code: null, reason: null, group_id: null, part: null, relayed_from: null,
    via_internet: false, seq: 1, at: "2026-10-19T09:00:00Z", claim_ts: null, truncated: null, ...over,
  });

  it("the lowest state among parts: failed < queued < sent < relayed < delivered; an unknown state reads unknown", () => {
    expect(lowestState(["delivered", "sent", "queued"])).toBe("queued");
    expect(lowestState(["delivered", "failed", "queued"])).toBe("failed");
    expect(lowestState(["delivered", "relayed"])).toBe("relayed");
    expect(lowestState(["sent", "mystery"])).toBe("unknown");
    expect(lowestState([])).toBe("unknown");
  });

  it("parts of one send group are one item in part order; other rows stay single; cancel covers the unsent parts only", () => {
    const rows = [
      row({ id: "fm-1", seq: 1, text: "plain", group_id: "sg-1" }),
      row({ id: "fm-3", seq: 3, group_id: "sg-2", part: { k: 2, n: 3 }, state: "sent" }),
      row({ id: "fm-2", seq: 2, group_id: "sg-2", part: { k: 1, n: 3 }, state: "relayed" }),
      row({ id: "fm-4", seq: 4, group_id: "sg-2", part: { k: 3, n: 3 }, state: "queued" }),
      row({ id: "fm-5", seq: 5, dir: "in" }),
    ];
    const items = groupParts(rows);
    expect(items.map((i) => i.kind)).toEqual(["row", "parts", "row"]);
    const parts = items[1]!;
    if (parts.kind !== "parts") throw new Error("not parts");
    expect(parts.rows.map((r) => r.id)).toEqual(["fm-2", "fm-3", "fm-4"]);
    const s = summarizeParts(parts.rows);
    expect(s).toMatchObject({ state: "queued", have: 3, total: 3, out: 2 });
    expect(s.unsent.map((r) => r.id)).toEqual(["fm-4"]);
  });

  it("a stale snapshot never steps a row down the ladder; a tail inserts and replaces; a window drops what the node dropped", () => {
    const conv = { id: "fc-1" } as never;
    const delivered = row({ id: "fm-1", seq: 1, state: "delivered" });
    expect(upsertForeign([delivered], row({ id: "fm-1", seq: 1, state: "sent" }))[0]!.state).toBe("delivered");
    expect(upsertForeign([row({ id: "fm-1", state: "failed" })], row({ id: "fm-1", state: "delivered" }))[0]!.state).toBe("delivered");
    const page = { conversation: conv, messages: [delivered, row({ id: "fm-2", seq: 2, state: "queued" })], more: false };
    const tail = applyForeignTail(page, { conversation: conv, messages: [row({ id: "fm-2", seq: 2, state: "sent" }), row({ id: "fm-3", seq: 3 })], more: false });
    expect(tail.messages.map((m) => [m.id, m.state])).toEqual([["fm-1", "delivered"], ["fm-2", "sent"], ["fm-3", "queued"]]);
    // the node's window is authoritative: a cancelled row (gone from it) is gone, and a row newer than the window is kept
    const merged = mergeForeignPage(tail, { conversation: conv, messages: [delivered, row({ id: "fm-3", seq: 3 })], more: false });
    expect(merged.messages.map((m) => m.id)).toEqual(["fm-1", "fm-3"]);
    expect(mergeForeignPage(tail, { conversation: conv, messages: [delivered], more: false }).messages.map((m) => m.id)).toEqual(["fm-1", "fm-2", "fm-3"]);
  });

  it("route parameters decode, and a network id with a colon round-trips through the path", () => {
    expect(foreignPath("app:deltachat", "fc-3")).toBe("/messages/net/app%3Adeltachat/fc-3");
    expect(paramText("app%3Adeltachat")).toBe("app:deltachat");
    expect(paramText("app:deltachat")).toBe("app:deltachat");
    expect(paramText("%E0%A4%A")).toBe("%E0%A4%A"); // a malformed escape is returned as it came
    expect(paramText(undefined)).toBeUndefined();
  });
});

describe("the store: Layer 6 slices and frames", () => {
  async function loaded(scenario: ScenarioName = "bridge") {
    const r = make(scenario);
    await apiStore.attach(r.a);
    await flush(30);
    return r;
  }

  it("loads the networks and edges with the snapshot; an older node (404) leaves them null and breaks nothing", async () => {
    const { a } = await loaded();
    expect(apiStore.getState().networks!.map((n) => n.id)).toEqual(["lxmf", "meshtastic", "app:deltachat"]);
    expect(apiStore.getState().edges!.map((e) => e.id)).toEqual(["be-1", "be-2"]);
    void a;
    const old = new ApiStore({ storage: null });
    const base = createMockAdapter({ scenario: "healthy", clock: new FakeClock(T0), tokenDelayMs: 0 });
    made.push(base);
    const partial = { ...base, networks: { ...base.networks, list: async () => Promise.reject(new ApiError({ type: "about:blank", title: "n", status: 404, code: "not_found" })) }, bridge: { ...base.bridge, edges: async () => Promise.reject(new ApiError({ type: "about:blank", title: "n", status: 404, code: "not_found" })) } } as Adapter;
    await old.attach(partial);
    await flush(30);
    expect(old.getState().networks).toBeNull();
    expect(old.getState().edges).toBeNull();
    expect(old.getState().meta).toMatchObject({ ready: true, needsPair: false });
    old.detach();
  });

  it("a bridge frame replaces the networks or the edges it carries, and only those", async () => {
    await loaded();
    const net = apiStore.getState().networks!;
    const edges = apiStore.getState().edges!;
    apiStore.applyFrame({ event: "bridge", data: { edges: [] } });
    expect(apiStore.getState().edges).toEqual([]);
    expect(apiStore.getState().networks).toBe(net);
    apiStore.applyFrame({ event: "bridge", data: { networks: [{ ...net[0]!, dropped: 9 }] } });
    expect(apiStore.getState().networks!.map((n) => [n.id, n.dropped])).toEqual([["lxmf", 9]]);
    void edges;
  });

  it("an open conversation follows network_message frames by re-reading the tail above its newest seq, not the whole window", async () => {
    const { a, clock } = await loaded();
    await apiStore.loadForeign("meshtastic", "fc-3");
    const spy = vi.spyOn(a.networks, "conversation");
    const top = Math.max(...apiStore.getState().foreignPages["fc-3"]!.page!.messages.map((m) => m.seq));
    apiStore.applyFrame({ event: "network_message", data: { network: "meshtastic", conv: "fc-3", change: "new", seq: top + 1 } });
    await flush(30);
    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy.mock.calls[0]![2]).toEqual({ sinceSeq: top, limit: 100 });
    void clock;
  });

  it("frames for the same conversation in one tick are one read; frames for another conversation do not wait for it", async () => {
    const { a } = await loaded();
    await apiStore.loadForeign("meshtastic", "fc-1");
    await apiStore.loadForeign("meshtastic", "fc-3");
    const spy = vi.spyOn(a.networks, "conversation");
    const top1 = Math.max(...apiStore.getState().foreignPages["fc-1"]!.page!.messages.map((m) => m.seq));
    for (let i = 0; i < 5; i++) apiStore.applyFrame({ event: "network_message", data: { network: "meshtastic", conv: "fc-1", change: "new", seq: top1 + i } });
    apiStore.applyFrame({ event: "network_message", data: { network: "meshtastic", conv: "fc-3", change: "new", seq: 999 } });
    await flush(60);
    const byConv = spy.mock.calls.map((c) => c[1]);
    expect(byConv.filter((c) => c === "fc-3")).toHaveLength(1);
    expect(byConv.filter((c) => c === "fc-1").length).toBeLessThanOrEqual(2); // the pass in flight and one trailing pass, however many frames
  });

  it("a state change of a row we hold re-reads from that row (below the newest seq) and the glyph moves up the ladder", async () => {
    const { a, clock } = await loaded();
    const ham = consentOf(await failure(a.networks.send("meshtastic", "fc-3", { text: "x" })))!;
    await confirmGate(a, ham, ham.hash);
    await apiStore.loadForeign("meshtastic", "fc-3");
    const sent = await a.networks.send("meshtastic", "fc-3", { text: "second" });
    await flush(30);
    await clock.advanceAsync(500, 100); // queued -> sent
    await flush(30);
    const later = await a.networks.send("meshtastic", "fc-3", { text: "third" }); // a newer row, so the first one's change sits below the newest seq
    await flush(30);
    expect(apiStore.getState().foreignPages["fc-3"]!.page!.messages.some((m) => m.id === later.id)).toBe(true);
    await clock.advanceAsync(2_500, 100);
    await flush(30);
    const mine = apiStore.getState().foreignPages["fc-3"]!.page!.messages.find((m) => m.id === sent.id)!;
    expect(mine.state).toBe("delivered");
  });

  it("a conversation change (a row cancelled) reloads the window and the row is gone", async () => {
    const { a } = await loaded();
    await apiStore.loadForeign("meshtastic", "fc-1");
    const queued = apiStore.getState().foreignPages["fc-1"]!.page!.messages.find((m) => m.reason_code === "rate_limited")!;
    await a.messages.cancel!(queued.id);
    await flush(60);
    expect(apiStore.getState().foreignPages["fc-1"]!.page!.messages.some((m) => m.id === queued.id)).toBe(false);
  });

  it("opening a conversation that does not exist reads missing, and an unknown frame is ignored", async () => {
    await loaded();
    await apiStore.loadForeign("meshtastic", "fc-404");
    expect(apiStore.getState().foreignPages["fc-404"]).toMatchObject({ state: "missing", page: null });
    expect(() => apiStore.applyFrame({ event: "network_message", data: { nonsense: true } as never })).not.toThrow();
  });

  it("a frame for a conversation nobody opened loads nothing", async () => {
    const { a } = await loaded();
    const spy = vi.spyOn(a.networks, "conversation");
    apiStore.applyFrame({ event: "network_message", data: { network: "meshtastic", conv: "fc-2", id: "fm-12", change: "state", seq: 12 } });
    await flush(30);
    expect(spy).not.toHaveBeenCalled();
  });
});

describe("attention: a stranger's text on another network never raises the header badge", () => {
  it("foreign DMs raise the unread count; a channel only when pinned; a recipient list never", () => {
    expect(countsAsUnread({ kind: "dm", pinned: false })).toBe(true);
    expect(countsAsUnread({ kind: "channel", pinned: false })).toBe(false);
    expect(countsAsUnread({ kind: "channel", pinned: true })).toBe(true);
    expect(countsAsUnread({ kind: "group", pinned: true })).toBe(false);
    expect(
      foreignUnread([
        { kind: "dm", pinned: false, unread: 2 },
        { kind: "channel", pinned: false, unread: 40 },
        { kind: "channel", pinned: true, unread: 1 },
        { kind: "group", pinned: false, unread: 5 },
      ]),
    ).toBe(3);
    expect(foreignUnread(null)).toBe(0);
    expect(foreignUnread(undefined)).toBe(0);
  });

  it("incoming foreign DMs and channel floods leave the store's attention exactly as it was", async () => {
    const { a, clock } = make("bridge");
    await apiStore.attach(a);
    await flush(30);
    const before = { ...apiStore.getState().attention };
    await clock.advanceAsync(12_000, 500); // the scenario delivers a hostile channel text at 6 s and Mara's DM at 9 s
    await flush(30);
    await apiStore.refreshConversations("meshtastic");
    const mesh = apiStore.getState().conversations.meshtastic!;
    expect(foreignUnread(mesh)).toBeGreaterThanOrEqual(3); // the DM (2 now), the pinned channel (1): the unpinned LongFast adds none
    // only the open proposals the scenario seeded count; no foreign text added anything
    expect(apiStore.getState().attention).toEqual(before);
    expect(apiStore.getState().attention.count).toBe(before.openProposals + before.erroredServices + before.erroredDevices + before.openRequests);
  });
});
