// Overview logic: the verdict sentence rules, the Needs-you derivation, and the small summaries.
import { describe, expect, it } from "vitest";
import { buildFixtures } from "../src/lib/api/mock/fixtures";
import type { AgentEvent, DeviceRecord, ServiceRecord, Status } from "../src/lib/api/types";
import { IDENTITY_BACKUP_COMMAND } from "../src/lib/commands";
import { verdict, verdictDot } from "../src/lib/verdict";
import {
  activityLine,
  detailText,
  devicesSummary,
  nearbySentence,
  needsYou,
  nodeFacts,
  recentActivity,
  updateText,
} from "../src/views/overview/model";

const NOW = Date.parse("2026-10-13T12:00:00Z");
const fx = buildFixtures(NOW);

const status = (over: Partial<Status> = {}): Status => ({ ...structuredClone(fx.status), ...over });
const services = (patch: Record<string, Partial<ServiceRecord>> = {}): ServiceRecord[] =>
  structuredClone(fx.services).map((s) => ({ ...s, ...(patch[s.id] ?? {}) }));
const devices = (patch: Array<Partial<DeviceRecord>> = []): DeviceRecord[] => structuredClone(fx.devices).map((d, i) => ({ ...d, ...(patch[i] ?? {}) }));
const ERR = { state: "error" } as const;

describe("verdict", () => {
  it("says it is checking while there is no status", () => {
    expect(verdict(null, null, null)).toEqual({ sentence: "Checking your node\u2026", tone: "attention" });
  });

  it("joined: connected, ok, plus the internet clause when the gateway is live", () => {
    const v = verdict(status({ transport_state: "joined" }), services(), devices());
    expect(v.sentence).toBe("Your node is running and connected to nearby nodes \u2014 messages can flow. It can reach the internet.");
    expect(v.tone).toBe("ok");
  });

  it("omits the internet clause when the gateway is not live", () => {
    const v = verdict(status({ transport_state: "joined" }), services({ gateway: { state: "unset" } }), devices());
    expect(v.sentence).toBe("Your node is running and connected to nearby nodes \u2014 messages can flow.");
  });

  it("isolated: alone on the mesh, attention", () => {
    const v = verdict(status({ transport_state: "isolated" }), services({ gateway: { state: "unset" } }), devices());
    expect(v).toEqual({ sentence: "Your node is running. It is alone on the mesh right now \u2014 no nearby nodes heard.", tone: "attention" });
  });

  it("no transport: asks for setup, attention", () => {
    const v = verdict(status({ transport_state: "no transport" }), services({ gateway: { state: "unset" } }), devices());
    expect(v).toEqual({ sentence: "Your node is running. It has no mesh transport yet \u2014 ask the agent to set one up.", tone: "attention" });
  });

  it("transport in error: the spec sentence, error tone", () => {
    const v = verdict(status({ transport_state: "transport degraded" }), services({ transport: ERR, gateway: { state: "unset" } }), devices());
    expect(v).toEqual({ sentence: "Your node is running, but the mesh transport has a problem.", tone: "error" });
  });

  it("transport degraded is an error even before the registry says so", () => {
    expect(verdict(status({ transport_state: "transport degraded" }), null, null)).toEqual({
      sentence: "Your node is running, but the mesh transport has a problem.",
      tone: "error",
    });
  });

  it("does not list the transport twice", () => {
    const v = verdict(status({ transport_state: "transport degraded" }), services({ transport: ERR, gateway: { state: "unset" } }), devices());
    expect(v.sentence.match(/mesh transport/g)).toHaveLength(1);
  });

  it("names any service in error, in plain words", () => {
    const base = services({ gateway: { state: "unset" } });
    const agent = base.map((s) => (s.id === "agent" ? { ...s, ...ERR } : s));
    expect(verdict(status(), agent, devices()).sentence).toBe("Your node is running, but the local AI agent has a problem.");
    const two = agent.map((s) => (s.id === "model" ? { ...s, ...ERR } : s));
    expect(verdict(status(), two, devices()).sentence).toBe("Your node is running, but the local AI agent and the local model have problems.");
    const three = two.map((s) => (s.id === "radio" ? { ...s, ...ERR } : s));
    expect(verdict(status(), three, devices()).sentence).toBe("Your node is running, but the local AI agent, the local model and the LoRa radio have problems.");
    const four = three.map((s) => (s.id === "kb" ? { ...s, ...ERR } : s));
    expect(verdict(status(), four, devices()).sentence).toBe("Your node is running, but 4 things have problems.");
  });

  it("counts devices in error", () => {
    const base = services({ gateway: { state: "unset" } });
    expect(verdict(status(), base, devices([{ state: "error" }])).sentence).toBe("Your node is running, but a connected device has a problem.");
    expect(verdict(status(), base, devices([{ state: "error" }, { state: "error" }])).sentence).toBe(
      "Your node is running, but 2 connected devices have problems.",
    );
    const mixed = base.map((s) => (s.id === "transport" ? { ...s, ...ERR } : s));
    expect(verdict(status(), mixed, devices([{ state: "error" }])).sentence).toBe(
      "Your node is running, but the mesh transport and a connected device have problems.",
    );
  });

  it("reports a failing health check even when nothing can be named", () => {
    const s = status({ health: { status: "error", components: {} } });
    expect(verdict(s, services({ gateway: { state: "unset" } }), devices())).toEqual({
      sentence: "Your node is running, but its health check reports a problem.",
      tone: "error",
    });
  });

  it("a transport that is still starting is not reported as alone or joined", () => {
    const v = verdict(status({ transport_state: "no transport" }), services({ transport: { state: "starting" }, gateway: { state: "unset" } }), devices());
    expect(v).toEqual({ sentence: "Your node is running. The mesh transport is still starting.", tone: "attention" });
  });

  it("never reads ok, and never says the happy sentence, while any service is in error", () => {
    for (const state of ["joined", "isolated", "no transport", "transport degraded"] as const) {
      for (const bad of fx.services.map((s) => s.id)) {
        const v = verdict(status({ transport_state: state }), services({ [bad]: ERR }), devices());
        expect(v.tone).toBe("error");
        expect(v.sentence).not.toContain("messages can flow");
        expect(v.sentence).toMatch(/has a problem|have problems/);
      }
    }
  });

  it("maps tone to a dot, hollow when disconnected", () => {
    expect(verdictDot("ok", false)).toBe("live");
    expect(verdictDot("attention", false)).toBe("unset");
    expect(verdictDot("error", false)).toBe("error");
    expect(verdictDot("error", true)).toBe("unknown");
  });
});

describe("needsYou", () => {
  const quiet = () => services({ model: { state: "live" }, transport: { state: "live" } });

  it("a settled node has nothing open; identity backup stays honestly unknown", () => {
    const n = needsYou(status({ transport_state: "joined" }), quiet(), devices(), []);
    expect(n.items).toEqual([]);
    expect(n.open).toBe(false);
    const identity = n.checklist.find((r) => r.id === "identity");
    expect(identity).toMatchObject({ state: "unknown", done: false, command: IDENTITY_BACKUP_COMMAND });
    expect(identity?.text).toContain("unknown");
    expect(IDENTITY_BACKUP_COMMAND).toBe("sudo -u node111 111 identity backup");
    expect(n.checklist.filter((r) => r.id !== "identity").every((r) => r.done && r.state === "live")).toBe(true);
  });

  it("first run: model missing, transport live, nobody heard yet", () => {
    const n = needsYou(
      status({ transport_state: "isolated" }),
      services({ model: { state: "unset", reason: "No local model is installed yet." }, transport: { state: "live" } }),
      devices(),
      [],
    );
    expect(n.open).toBe(true);
    const row = (id: string) => n.checklist.find((r) => r.id === id);
    expect(row("model")).toMatchObject({ state: "unset", done: false, text: "No local model is installed yet.", href: "/services/model" });
    expect(row("transport")).toMatchObject({ state: "live", done: true });
    expect(row("peer")).toMatchObject({ state: "unset", done: false, text: "none heard yet" });
  });

  it("first peer heard follows transport_state === joined only", () => {
    const peer = (t: Status["transport_state"]) => needsYou(status({ transport_state: t }), quiet(), devices(), []).checklist.find((r) => r.id === "peer");
    expect(peer("joined")).toMatchObject({ state: "live", done: true });
    expect(peer("isolated")?.done).toBe(false);
    expect(peer("no transport")?.text).toBe("needs the mesh transport first");
  });

  it("lists open proposals only, then services in error, then devices in error", () => {
    const n = needsYou(
      status(),
      services({ transport: { state: "error", reason: "Reticulum is not passing traffic." } }),
      devices([{ state: "error", reason: "Port unreadable.", alias: "roof board" }]),
      fx.proposals,
    );
    expect(n.items.map((i) => i.kind)).toEqual(["proposal", "service", "device"]);
    expect(n.items[0]).toMatchObject({ label: "Mesh transport restarted twice in the last hour", href: "/" });
    expect(n.items[1]).toMatchObject({ label: "Mesh transport", detail: "Reticulum is not passing traffic.", href: "/services/transport" });
    expect(n.items[2]).toMatchObject({ label: "roof board", detail: "Port unreadable.", href: "/devices/usb-7c1e0b94a2d3" });
    expect(n.open).toBe(true);
  });

  it("an item makes the block open even when the checklist is done", () => {
    const n = needsYou(status({ transport_state: "joined" }), quiet(), devices(), fx.proposals);
    expect(n.items).toHaveLength(1);
    expect(n.open).toBe(true);
  });

  it("device ids are encoded in the link", () => {
    const n = needsYou(status(), quiet(), devices([{ state: "error", id: "bt/odd id" }]), []);
    expect(n.items[0]?.href).toBe("/devices/bt%2Fodd%20id");
  });

  it("drops checklist rows this device cannot run", () => {
    const n = needsYou(status({ transport_state: "joined" }), services({ model: { can_run_here: false, state: "unset" }, transport: { state: "live" } }), devices(), []);
    expect(n.checklist.map((r) => r.id)).toEqual(["identity", "transport", "peer"]);
  });

  it("starting and errored checklist services read as such", () => {
    const n = needsYou(status(), services({ model: { state: "starting" }, transport: { state: "error", reason: "bad" } }), devices(), []);
    expect(n.checklist.find((r) => r.id === "model")).toMatchObject({ state: "starting", done: false });
    expect(n.checklist.find((r) => r.id === "transport")).toMatchObject({ state: "error", text: "bad" });
  });

  it("before the first snapshot every derived row is unknown and nothing is claimed", () => {
    const n = needsYou(null, null, null, null);
    expect(n.items).toEqual([]);
    expect(n.checklist.map((r) => r.state)).toEqual(["unknown", "unknown", "unknown", "unknown"]);
    expect(n.open).toBe(true);
  });
});

describe("nearby, devices and node summaries", () => {
  it("nearbySentence covers every transport state", () => {
    expect(nearbySentence(status({ transport_state: "joined" }))).toMatch(/heard/);
    expect(nearbySentence(status({ transport_state: "isolated" }))).toMatch(/alone/);
    expect(nearbySentence(status({ transport_state: "no transport" }))).toMatch(/no mesh transport/);
    expect(nearbySentence(status({ transport_state: "transport degraded" }))).toMatch(/problem/);
    expect(nearbySentence(null)).toMatch(/Waiting/);
  });

  it("devicesSummary counts by state", () => {
    const d = devices([{ state: "connected" }, { state: "needs_setup" }, { state: "error" }, { state: "disconnected" }]);
    expect(devicesSummary(d)).toMatchObject({ total: 4, connected: 1, needSetup: 1, error: 1, text: "4 devices: 1 connected, 1 needs setup, 1 has an error, 1 disconnected." });
    expect(devicesSummary(fx.devices).text).toBe("4 devices: 3 connected, 1 needs setup.");
    expect(devicesSummary([]).text).toBe("Nothing attached or paired yet.");
    expect(devicesSummary(null).text).toMatch(/Waiting/);
  });

  it("updateText names each updater state", () => {
    const u = fx.status.update;
    expect(updateText({ ...u, state: "idle", pending: false, last_result: "up_to_date" })).toBe("up to date");
    expect(updateText({ ...u, state: "staged", pending: true, pending_version: "2026.10.14" })).toBe("update 2026.10.14 staged, applies on restart");
    expect(updateText({ ...u, state: "staged", pending: true, pending_version: null })).toBe("update staged, applies on restart");
    expect(updateText({ ...u, state: "failed" })).toBe("last update failed");
    expect(updateText({ ...u, state: "checking" })).toMatch(/checking/);
    // an update that went through leaves the node idle with last_result "staged": say what it is on now
    expect(updateText({ ...u, state: "idle", pending: false, pending_version: null, last_result: "staged" }, "2026.10.14")).toBe("updated to 2026.10.14");
    expect(updateText({ ...u, state: "idle", pending: false, pending_version: null, last_result: "staged" })).toBe("updated");
    expect(updateText({ ...u, state: "idle", pending: false, last_result: null })).toBe("idle");
  });

  it("nodeFacts humanizes uptime, last check, gateway and reach", () => {
    const f = nodeFacts(fx.status, fx.services, NOW);
    expect(f).toMatchObject({
      name: "Workshop laptop",
      label: "k7m2-9x4p-aa",
      version: "2026.10.13",
      uptime: "7 h 6 min",
      lastCheck: "3 h ago",
      gateway: "internet reachable",
      reach: "devices on your local network can open it",
    });
    expect(f.bech32).toMatch(/^node1/);
    expect(f.addresses).toContain("http://111.local");
  });

  it("nodeFacts falls back honestly", () => {
    const s = status();
    s.node.alias = null;
    s.update.last_check = null;
    s.bind = { urls: [], lan: false };
    const f = nodeFacts(s, services({ gateway: { state: "unset" } }), NOW);
    expect(f).toMatchObject({ name: "k7m2-9x4p-aa", lastCheck: "never", gateway: "no internet", reach: "this device only" });
    expect(nodeFacts(s, null, NOW).gateway).toBe("unknown");
  });
});

describe("agent activity lines", () => {
  const ev = (seq: number, kind: AgentEvent["kind"], detail: AgentEvent["detail"] = {}): AgentEvent => ({
    schema: 1,
    seq,
    ts: new Date(NOW + seq * 1000).toISOString(),
    kind,
    detail,
  });

  it("renders kind, time and a one-line detail", () => {
    const line = activityLine(ev(1, "task.submitted", { task_id: "t1", kind: "diagnose" }));
    expect(line).toMatch(/^task\.submitted \u00b7 \d\d:\d\d:\d\d \u00b7 task_id=t1 kind=diagnose$/);
    expect(activityLine(ev(2, "chat.turn"))).toMatch(/^chat\.turn \u00b7 \d\d:\d\d:\d\d$/);
  });

  it("strips control characters and clips long details", () => {
    expect(detailText({ note: "a\u0007b\u202ec" })).toBe("note=abc");
    expect(detailText({ big: "x".repeat(400) })).toHaveLength(121);
    expect(detailText({ nested: { a: 1 }, n: 2 })).toBe('nested={"a":1} n=2');
  });

  it("an unreadable stamp does not break the line", () => {
    expect(activityLine({ ...ev(1, "chat.turn"), ts: "not a date" })).toBe("chat.turn \u00b7 --:--:--");
  });

  it("recentActivity takes the newest n and reads oldest first", () => {
    const events = [5, 4, 3, 2, 1].map((n) => ev(n, "chat.turn", { n }));
    const lines = recentActivity(events, 3);
    expect(lines).toHaveLength(3);
    expect(lines[0]).toContain("n=3");
    expect(lines[2]).toContain("n=5");
  });
});
