import { afterEach, describe, expect, it } from "vitest";
import type { EventsFrameWithId } from "../src/lib/api/adapter";
import { ApiError } from "../src/lib/api/problem";
import { MOCK_PAIR_CODE, SCENARIO_NAMES, createMockAdapter, type MockAdapter, type MockOptions, type ScenarioName } from "../src/lib/api/mock";
import { buildFixtures } from "../src/lib/api/mock/fixtures";
import type { DeviceRecord } from "../src/lib/api/types";
import { finalSentence, pickReply, tokenize } from "../src/lib/api/mock/chat";
import { SERVICE_IDS, type ChatFrame, type ServiceId } from "../src/lib/api/types";
import { needsPairCode } from "../src/app/pairing";
import { operations, sse } from "./contract.gen";
import { FakeClock, collect, flush, last } from "./helpers/fakes";
import { validate, validateAgainst } from "./helpers/validate";

const T0 = Date.parse("2026-10-13T12:00:00Z");
const made: MockAdapter[] = [];
afterEach(() => {
  for (const a of made.splice(0)) a.mock.dispose();
});

function make(scenario: ScenarioName, extra: Partial<MockOptions> = {}) {
  const clock = new FakeClock(T0);
  const a = createMockAdapter({ scenario, clock, tokenDelayMs: 0, ...extra });
  made.push(a);
  return { a, clock };
}

function listen(a: MockAdapter) {
  const frames: EventsFrameWithId[] = [];
  const states: string[] = [];
  const off = a.events({ onFrame: (f) => frames.push(f), onState: (s) => states.push(s) });
  return { frames, states, off };
}

const failure = async (p: Promise<unknown>): Promise<ApiError> => {
  const e = await p.then(
    () => null,
    (x: unknown) => x,
  );
  expect(e).toBeInstanceOf(ApiError);
  return e as ApiError;
};

function expectValid(opId: string, value: unknown, contentType = "application/json") {
  const schema = operations[opId]!.schemasByType[contentType];
  expect(schema, `${opId} has a ${contentType} response`).toBeTruthy();
  expect(validate(value, schema!), opId).toEqual([]);
}

describe("contract validator has teeth", () => {
  it("rejects bad values", () => {
    const f = buildFixtures(T0);
    expect(validateAgainst("Status", f.status)).toEqual([]);
    expect(validateAgainst("Status", { ...f.status, transport_state: "joined!" })).not.toEqual([]);
    const { health: _h, ...noHealth } = f.status;
    expect(validateAgainst("Status", noHealth)).toContain("$: missing required \"health\"");
    expect(validateAgainst("ServiceRecord", { ...f.services[0], state: "dead" })).not.toEqual([]);
    expect(validateAgainst("ServiceRecord", { ...f.services[0], since: "yesterday" })).not.toEqual([]);
    expect(validateAgainst("Proposal", { ...f.proposals[0], kind: "wish" })).not.toEqual([]);
    expect(validateAgainst("Proposal", { ...f.proposals[0], decision: "accepted" })).not.toEqual([]);
    expect(validateAgainst("ChatRequest", { conversation_id: "bad id!", message: "x" })).not.toEqual([]);
  });
});

describe("fixtures", () => {
  const f = buildFixtures(T0);

  it("describe the Node2B laptop node from the brief", () => {
    expect(f.status.node).toMatchObject({ category: "Node2B", label: "k7m2-9x4p-aa", version: "2026.10.13" });
    expect(f.status.node.node_id).toMatch(/^[0-9a-f]{64}$/);
    expect(f.status.node.bech32).toMatch(/^node1[a-z0-9]+$/);
    expect(f.status.node.label).toMatch(/^[a-z0-9]{4}-[a-z0-9]{4}-[a-z0-9]{2}$/);
  });

  it("has the seven services in registry order, each pointing at a real KB entry", () => {
    expect(f.services.map((s) => s.id)).toEqual([...SERVICE_IDS]);
    for (const s of f.services) {
      if (s.kb_entry) expect(f.kbEntries[s.kb_entry], s.kb_entry).toBeTruthy();
      expect(s.reason_code).toMatch(/^[a-z0-9_]+$/);
    }
    expect(f.services.find((s) => s.id === "radio")).toMatchObject({ kb_entry: "projects/meshtastic.md", reason_code: "radio_unbound", can_run_here: true });
    expect(f.services.find((s) => s.id === "transport")?.kb_entry).toBe("projects/reticulum.md");
    expect(f.services.find((s) => s.id === "model")?.kb_entry).toBe("projects/llama-cpp.md");
    expect(f.services.find((s) => s.id === "codeagent")?.kb_entry).toBe("projects/opencode.md");
  });

  it("has four devices: a Heltec board needing setup, a paired phone, a Bluetooth adapter, an unidentified serial device", () => {
    expect(f.devices.map((d) => [d.kind, d.state])).toEqual([
      ["radio", "needs_setup"],
      ["companion", "connected"],
      ["bluetooth", "connected"],
      ["unknown", "connected"],
    ]);
    const [heltec, phone, bt, unknown] = f.devices as [DeviceRecord, DeviceRecord, DeviceRecord, DeviceRecord];
    expect(heltec).toMatchObject({
      name: "Heltec WiFi LoRa 32 V3",
      transport: "usb-serial",
      reason_code: "no_driver_service",
      service_id: "radio",
      kb_entry: "devices/heltec-wifi-lora-32-v3.md",
      capabilities: ["lora"],
      identity: { vendor_id: "303a", product_id: "1001" },
    });
    expect(heltec.identity.serial).toEqual(expect.any(String));
    expect(heltec.identity.port).toMatch(/^\/dev\/serial\/by-id\//);
    expect(phone).toMatchObject({ transport: "lan", capabilities: ["dashboard-session"], reason_code: "session_active" });
    expect(phone.identity).toMatchObject({ ip: expect.stringMatching(/^\d+\.\d+\.\d+\.\d+$/), user_agent: expect.stringContaining("Mozilla") });
    expect(bt).toMatchObject({ transport: "bluetooth", capabilities: ["ble"] });
    expect(unknown).toMatchObject({ transport: "usb-serial", reason_code: "unidentified", capabilities: [] });
    expect(new Set(f.devices.map((d) => d.id)).size).toBe(4);
    for (const d of f.devices) {
      if (d.kb_entry) expect(f.kbEntries[d.kb_entry], d.kb_entry).toBeTruthy();
      if (d.service_id) expect(f.services.some((s) => s.id === d.service_id)).toBe(true);
      expect(d.reason_code).toMatch(/^[a-z0-9_]+$/);
      expect(d.actions).toEqual(expect.arrayContaining(["rename", "forget"]));
    }
  });

  it("is internally consistent: the radio service notices the board, and a paired phone implies LAN sharing", () => {
    expect(f.status.lan_enabled).toBe(true);
    expect(f.status.bind.lan).toBe(true);
    expect(f.settings.lan_enabled).toBe(true);
    const radio = f.services.find((s) => s.id === "radio")!;
    expect(radio.detail.devices).toEqual([f.devices[0]!.id]);
  });

  it("has three proposals: open diagnose, accepted, chat", () => {
    expect(f.proposals.map((p) => [p.kind, p.decision?.decision ?? "open"])).toEqual([
      ["diagnose", "open"],
      ["propose", "accepted"],
      ["chat", "dismissed"],
    ]);
  });

  it("lists every KB entry in llms.txt", () => {
    for (const path of Object.keys(f.kbEntries)) expect(f.kbIndex).toContain(`(${path})`);
    expect(Object.keys(f.kbEntries).length).toBeGreaterThanOrEqual(2);
  });
});

describe.each(SCENARIO_NAMES)("scenario %s: every result validates against openapi.yaml", (scenario) => {
  it("status, services, proposals, settings, kb, auth, logs", async () => {
    const { a } = make(scenario);
    if (scenario === "lan-unpaired" || scenario === "termux-unpaired") await a.auth.pair(MOCK_PAIR_CODE);

    expectValid("getStatus", await a.status());
    const services = await a.services();
    expectValid("listServices", { schema: 1, services });
    expect(services.map((s) => s.id)).toEqual([...SERVICE_IDS]);
    for (const s of services) {
      expectValid("getService", await a.service(s.id));
      expectValid("getServiceLog", await a.serviceLog(s.id, 30));
    }
    expectValid("listProposals", { schema: 1, proposals: await a.proposals("all") });
    const devices = await a.devices();
    expectValid("listDevices", { schema: 1, devices });
    expect(devices.length).toBeGreaterThanOrEqual(3);
    for (const d of devices) expectValid("getDevice", await a.device(d.id));
    expectValid("getSettings", await a.settings());
    expect(typeof (await a.kb.index())).toBe("string");
    for (const s of services) if (s.kb_entry) expect(await a.kb.entry(s.kb_entry)).toMatch(/^# /);
    expectValid("authSession", await a.auth.session());
  });

  it("agent routes either validate or fail with a valid problem+json", async () => {
    const { a } = make(scenario);
    if (scenario === "lan-unpaired" || scenario === "termux-unpaired") await a.auth.pair(MOCK_PAIR_CODE);
    const agentLive = (await a.service("agent")).state === "live";
    const modelLive = (await a.service("model")).state === "live";

    if (agentLive) {
      expectValid("agentStatus", await a.agentStatus());
      expectValid("chatHistory", await a.chat.history("conv-1"));
      const accepted = await a.agentTask({ kind: "diagnose", note: null });
      expect(validate(accepted, operations.agentTask!.schema!)).toEqual([]);
    } else {
      for (const p of [a.agentStatus(), a.chat.history("conv-1"), a.agentTask({ kind: "diagnose" })]) {
        const e = await failure(p);
        expect(e).toMatchObject({ status: 503, code: "agent_unavailable" });
        expect(validateAgainst("Problem", e.problem)).toEqual([]);
      }
    }

    const frames = await collect(a.chat.send({ conversation_id: "conv-1", message: "hello" })).catch((e: unknown) => e);
    if (agentLive && modelLive) {
      for (const f of frames as ChatFrame[]) expect(validateAgainst(sse.chat[f.event]!, f.data), f.event).toEqual([]);
      const done = last(frames as ChatFrame[])!;
      expect(done.event).toBe("done");
      expectValid("chat", { schema: 1, ...(done.data as object) });
    } else {
      expect(frames).toBeInstanceOf(ApiError);
      expect(validateAgainst("Problem", (frames as ApiError).problem)).toEqual([]);
      expect((frames as ApiError).code).toBe(agentLive ? "model_unavailable" : "agent_unavailable");
    }
  });

  it("every event frame validates against its x-sse-events schema", async () => {
    const { a, clock } = make(scenario);
    if (scenario === "lan-unpaired" || scenario === "termux-unpaired") await a.auth.pair(MOCK_PAIR_CODE);
    const { frames } = listen(a);
    await flush();
    await clock.advanceAsync(65_000, 500);
    expect(frames.length).toBeGreaterThan(0);
    for (const f of frames) expect(validateAgainst(sse.events[f.event]!, f.data), f.event).toEqual([]);
  });
});

describe("scenarios", () => {
  it("healthy: everything that can run is live; radio and code agent are not available here", async () => {
    const { a } = make("healthy");
    const states = Object.fromEntries((await a.services()).map((s) => [s.id, s.state]));
    expect(states).toEqual({ agent: "live", model: "live", transport: "live", radio: "unset", kb: "live", gateway: "live", codeagent: "unset" });
    const status = await a.status();
    expect(status.transport_state).toBe("joined");
    expect(status.health.status).toBe("ok");
    expect(await a.proposals()).toHaveLength(1);
  });

  it("first-run: no peers, model absent with the offer offered, no proposals", async () => {
    const { a, clock } = make("first-run");
    const status = await a.status();
    expect(status).toMatchObject({ transport_state: "isolated", offer: { state: "offered" }, node: { alias: null } });
    expect(status.intelligence.model_id).toBeNull();
    expect((await a.service("transport")).detail.peers).toBe("isolated");
    expect(await a.service("model")).toMatchObject({ state: "unset", reason_code: "no_model", detail: { offer: "offered" } });
    expect((await a.settings()).offer).toBe("offered");
    expect(await a.proposals("all")).toEqual([]);
    expect(status.lan_enabled).toBe(false);
    expect((await a.devices()).map((d) => d.kind)).toEqual(["radio", "bluetooth", "unknown"]); // nobody has paired a phone yet
    const e = await failure(collect(a.chat.send({ conversation_id: "conv-1", message: "hi" })));
    expect(e).toMatchObject({ status: 503, code: "model_unavailable" });

    // accepting the offer starts the model, and chat works once it is live
    const { frames } = listen(a);
    await flush();
    await a.patchSettings({ offer: "accepted" });
    expect((await a.service("model")).state).toBe("starting");
    await clock.advanceAsync(8_100, 100);
    expect((await a.service("model")).state).toBe("live");
    expect((await a.status()).intelligence.agent_state).toBe("local");
    expect((await a.agentStatus()).model?.reachable).toBe(true);
    expect(frames.filter((f) => f.event === "services").length).toBeGreaterThanOrEqual(2);
    const chat = await collect(a.chat.send({ conversation_id: "conv-1", message: "hi" }));
    expect(last(chat)?.event).toBe("done");
  });

  it("agent-down: agent and model in error, agent routes answer 503", async () => {
    const { a } = make("agent-down");
    expect(await a.service("agent")).toMatchObject({ state: "error", reason_code: "child_backoff" });
    expect(await a.service("model")).toMatchObject({ state: "error", reason_code: "health_503" });
    expect((await a.status()).intelligence).toMatchObject({ agent: "backoff", agent_state: "unavailable" });
    expect(await failure(a.agentStatus())).toMatchObject({ status: 503, code: "agent_unavailable" });
    expect(await failure(collect(a.chat.send({ conversation_id: "conv-1", message: "hi" })))).toMatchObject({ code: "agent_unavailable" });
    // proposals are files the daemon reads itself: still available
    expect(await a.proposals()).toHaveLength(1);
  });

  it("transport-error: degraded transport, then the agent leaves a diagnose proposal", async () => {
    const { a, clock } = make("transport-error");
    expect(await a.service("transport")).toMatchObject({ state: "error", reason_code: "transport_degraded" });
    expect((await a.status()).transport_state).toBe("transport degraded");
    const { frames } = listen(a);
    await flush();
    expect(await a.proposals()).toHaveLength(1);
    await clock.advanceAsync(10_100, 100);
    const proposalFrames = frames.filter((f) => f.event === "proposal");
    expect(proposalFrames).toHaveLength(1);
    expect(proposalFrames[0]!.data).toMatchObject({ id: "prop-01j9z8n7p5ke", kind: "diagnose" });
    expect(await a.proposals()).toHaveLength(2);
    expect(frames.filter((f) => f.event === "agent").map((f) => (f.data as { kind: string }).kind)).toEqual(["task.submitted", "proposal.written"]);
  });

  it("updating: staged, then probation, then idle with the new version", async () => {
    const { a, clock } = make("updating");
    const { frames } = listen(a);
    await flush();
    expect((await a.status()).update).toMatchObject({ state: "staged", pending: true, pending_version: "2026.10.14" });
    expect((await a.status()).node.version).toBe("2026.10.13");
    await clock.advanceAsync(12_100, 100);
    expect((await a.status()).update.state).toBe("probation");
    expect((await a.status()).node.version).toBe("2026.10.13");
    await clock.advanceAsync(23_000, 100);
    const s = await a.status();
    expect(s.update).toMatchObject({ state: "idle", pending: false, pending_version: null });
    expect(s.node.version).toBe("2026.10.14");
    const versions = frames.filter((f) => f.event === "status").map((f) => (f.data as { node: { version: string } }).node.version);
    expect(last(versions)).toBe("2026.10.14");
    expect(new Set(versions)).toEqual(new Set(["2026.10.13", "2026.10.14"]));
  });

  it("lan-unpaired: 401 everywhere until pairing succeeds, then loopback-only settings answer 403", async () => {
    const { a } = make("lan-unpaired");
    const calls: Promise<unknown>[] = [a.status(), a.services(), a.service("agent"), a.serviceLog("agent"), a.proposals(), a.settings(), a.kb.index(), a.agentStatus(), a.chat.history("c"), a.restart(), a.updateCheck(), a.decide("p", "accepted"), a.patchSettings({ alias: "x" })];
    for (const c of calls) expect(await failure(c)).toMatchObject({ status: 401, code: "unauthenticated" });
    expect(await failure(collect(a.chat.send({ conversation_id: "conv-1", message: "hi" })))).toMatchObject({ code: "unauthenticated" });

    expect(await a.auth.session()).toMatchObject({ loopback: false, paired: false, lan_enabled: true });
    const { states, frames } = listen(a);
    await flush();
    expect(states).toEqual(["connecting", "closed"]);
    expect(frames).toEqual([]);

    expect(await failure(a.auth.pair("ZZZZ-ZZZZ"))).toMatchObject({ status: 401, code: "unauthenticated" });
    expect(await a.auth.pair("aaaa bbbb")).toMatchObject({ paired: true, loopback: false });
    await flush();
    expect(states.slice(-2)).toEqual(["connecting", "open"]);
    expect(frames[0]).toEqual({ event: "reset", data: { reason: "restart" } });

    expect((await a.status()).auth).toEqual({ loopback: false, paired: true });
    expect(await a.patchSettings({ alias: "Kitchen" })).toMatchObject({ alias: "Kitchen" });
    expect(await failure(a.patchSettings({ lan_enabled: false }))).toMatchObject({ status: 403, code: "loopback_only" });
    expect(await failure(a.patchSettings({ offer: "dismissed" }))).toMatchObject({ code: "loopback_only" });

    await a.auth.logout();
    expect(await failure(a.status())).toMatchObject({ code: "unauthenticated" });
  });

  it("auth/session carries the loopback policy and the caller's own device id", async () => {
    const healthy = make("healthy").a;
    expect(await healthy.auth.session()).toMatchObject({ loopback: true, loopback_trusted: true, paired: false, device_id: "ses-4be2a91c07d5" });
    expectValid("authSession", await healthy.auth.session());
    const lan = make("lan-unpaired").a;
    expect(await lan.auth.session()).toMatchObject({ loopback: false, loopback_trusted: true, device_id: null });
    await lan.auth.pair(MOCK_PAIR_CODE);
    expect(await lan.auth.session()).toMatchObject({ paired: true, device_id: "ses-4be2a91c07d5" });
  });

  it("termux-unpaired: a loopback browser that is not trusted gets 401 until it pairs, like a LAN visitor", async () => {
    const { a } = make("termux-unpaired");
    const session = await a.auth.session();
    expect(session).toMatchObject({ loopback: true, loopback_trusted: false, paired: false, device_id: null });
    expectValid("authSession", session);
    expect(needsPairCode(session)).toBe(true);
    for (const c of [a.status(), a.services(), a.devices(), a.settings(), a.proposals()]) expect(await failure(c)).toMatchObject({ status: 401, code: "unauthenticated" });
    const { states } = listen(a);
    await flush();
    expect(states).toEqual(["connecting", "closed"]);

    expect(await failure(a.auth.pair("ZZZZ-ZZZZ"))).toMatchObject({ status: 401 });
    expect(await a.auth.pair("aaaa-bbbb")).toMatchObject({ paired: true, loopback: true, loopback_trusted: false, device_id: "ses-4be2a91c07d5" });
    await flush();
    expect(states.slice(-1)).toEqual(["open"]);
    expect((await a.status()).auth).toEqual({ loopback: true, paired: true });
    expect(needsPairCode(await a.auth.session())).toBe(false);
    // still on loopback, so the loopback-only settings work once paired
    expect(await a.patchSettings({ lan_enabled: false })).toMatchObject({ lan_enabled: false });
  });

  it("lan-unpaired: five wrong codes lock pairing for a minute", async () => {
    const { a, clock } = make("lan-unpaired");
    for (let i = 0; i < 5; i++) expect(await failure(a.auth.pair("NOPE-NOPE"))).toMatchObject({ code: "unauthenticated" });
    expect(await failure(a.auth.pair(MOCK_PAIR_CODE))).toMatchObject({ status: 429, code: "rate_limited" });
    clock.advance(60_001);
    expect(await a.auth.pair(MOCK_PAIR_CODE)).toMatchObject({ paired: true });
  });

  it("new-device: starts with three devices, then a Heltec board appears and the radio service notices it", async () => {
    const { a, clock } = make("new-device");
    const { frames } = listen(a);
    await flush();
    expect((await a.devices()).map((d) => d.kind)).toEqual(["companion", "bluetooth", "unknown"]);
    expect(await a.service("radio")).toMatchObject({ reason_code: "no_radio", can_run_here: false });
    await failure(a.device("usb-7c1e0b94a2d3"));
    await clock.advanceAsync(7_900, 100);
    expect(frames.filter((f) => f.event === "devices")).toHaveLength(0);
    await clock.advanceAsync(200, 100);
    const device = frames.filter((f) => f.event === "devices");
    expect(device).toHaveLength(1);
    const list = (device[0]!.data as { devices: DeviceRecord[] }).devices;
    expect(list.map((d) => d.kind)).toEqual(["radio", "companion", "bluetooth", "unknown"]); // registry order is stable
    expect(list[0]).toMatchObject({ name: "Heltec WiFi LoRa 32 V3", state: "needs_setup", reason_code: "no_driver_service" });
    expect(await a.device("usb-7c1e0b94a2d3")).toMatchObject({ state: "needs_setup" });
    expect(await a.service("radio")).toMatchObject({ reason_code: "radio_unbound", can_run_here: true });
    expect(frames.filter((f) => f.event === "services")).toHaveLength(1);
    expect((await a.proposals()).length).toBe(1); // needs_setup is not an error, so nothing new needs attention
  });

  it("refuses to build an unknown scenario", () => {
    expect(() => createMockAdapter({ scenario: "nope" as ScenarioName, clock: new FakeClock() })).toThrow(/unknown scenario/);
  });
});

describe("mock devices", () => {
  const HELTEC = "usb-7c1e0b94a2d3";
  const PHONE = "ses-4be2a91c07d5";

  it("patchDevice renames, trims, clears, validates, and announces the change", async () => {
    const { a } = make("healthy");
    const { frames } = listen(a);
    await flush();
    const renamed = await a.patchDevice(HELTEC, { alias: "  Roof radio  " });
    expect(renamed.alias).toBe("Roof radio");
    expectValid("patchDevice", renamed);
    expect((await a.device(HELTEC)).alias).toBe("Roof radio");
    const frame = last(frames.filter((f) => f.event === "devices"))!;
    expect((frame.data as { devices: DeviceRecord[] }).devices.find((d) => d.id === HELTEC)?.alias).toBe("Roof radio");
    expect((await a.patchDevice(HELTEC, { alias: "" })).alias).toBeNull();
    await a.patchDevice(HELTEC, { alias: "again" });
    expect((await a.patchDevice(HELTEC, { alias: null })).alias).toBeNull();
    expect((await a.patchDevice(HELTEC, {})).alias).toBeNull(); // no alias key: nothing changes
    const e = await failure(a.patchDevice(HELTEC, { alias: "x".repeat(65) }));
    expect(e).toMatchObject({ status: 400, code: "bad_request" });
    expect(e.problem.errors?.[0]?.pointer).toBe("/alias");
    expect(await failure(a.patchDevice("nope", { alias: "x" }))).toMatchObject({ status: 404, code: "not_found" });
  });

  it("returns copies, so callers cannot corrupt the registry", async () => {
    const { a } = make("healthy");
    const list = await a.devices();
    list[0]!.state = "error";
    expect((await a.devices())[0]!.state).toBe("needs_setup");
  });

  it("forgetDevice removes the device and announces it; unknown ids are 404", async () => {
    const { a } = make("healthy");
    const { frames } = listen(a);
    await flush();
    expect(await a.forgetDevice(PHONE)).toBeUndefined();
    expect((await a.devices()).map((d) => d.id)).not.toContain(PHONE);
    const frame = last(frames.filter((f) => f.event === "devices"))!;
    expect((frame.data as { devices: DeviceRecord[] }).devices).toHaveLength(3);
    expect(await failure(a.device(PHONE))).toMatchObject({ status: 404, code: "not_found" });
    expect(await failure(a.forgetDevice(PHONE))).toMatchObject({ code: "not_found" });
  });

  it("forgotten hardware reappears after 15 s with its alias dropped, like a re-plug; a paired phone does not", async () => {
    const { a, clock } = make("healthy");
    const { frames } = listen(a);
    await flush();
    await a.patchDevice(HELTEC, { alias: "Bench" });
    const firstSeen = (await a.device(HELTEC)).first_seen;
    await a.forgetDevice(HELTEC);
    await a.forgetDevice(PHONE);
    await clock.advanceAsync(14_900, 100);
    expect((await a.devices()).map((d) => d.kind)).toEqual(["bluetooth", "unknown"]);
    await clock.advanceAsync(200, 100);
    const after = await a.devices();
    expect(after.map((d) => d.kind)).toEqual(["radio", "bluetooth", "unknown"]); // back in registry order, phone gone for good
    const back = after[0]!;
    expect(back).toMatchObject({ id: HELTEC, alias: null, state: "needs_setup" });
    expect(Date.parse(back.first_seen)).toBeGreaterThan(Date.parse(firstSeen));
    expect(frames.filter((f) => f.event === "devices")).toHaveLength(4); // rename, forget, forget, reappear
    await clock.advanceAsync(60_000, 1_000);
    expect((await a.devices()).map((d) => d.id)).not.toContain(PHONE);
  });

  it("forgetting the same hardware again schedules another reappearance", async () => {
    const { a, clock } = make("healthy");
    await a.forgetDevice(HELTEC);
    await clock.advanceAsync(15_100, 100);
    await a.forgetDevice(HELTEC);
    expect((await a.devices()).some((d) => d.id === HELTEC)).toBe(false);
    await clock.advanceAsync(15_100, 100);
    expect((await a.devices()).some((d) => d.id === HELTEC)).toBe(true);
  });

  it("requires pairing like every other route", async () => {
    const { a } = make("lan-unpaired");
    for (const p of [a.devices(), a.device(HELTEC), a.patchDevice(HELTEC, { alias: "x" }), a.forgetDevice(HELTEC)]) {
      expect(await failure(p)).toMatchObject({ status: 401, code: "unauthenticated" });
    }
    await a.auth.pair(MOCK_PAIR_CODE);
    expect(await a.devices()).toHaveLength(4);
  });

  it("a device in error is what the header badge would count (registry state, not the mock, decides)", async () => {
    const { a } = make("healthy");
    const d = a.mock.state.devices.find((x) => x.id === HELTEC)!;
    d.state = "error";
    d.reason_code = "service_error";
    expect((await a.device(HELTEC)).state).toBe("error");
    expectValid("getDevice", await a.device(HELTEC));
  });
});

describe("mock chat", () => {
  const MESSAGES = ["How do I set up the radio?", "Why is the model slow?", "Is my mesh connected to any neighbours?", "Hello there"];

  it.each(MESSAGES)("streams 25-60 tokens, one kb.read tool in the middle, and a final-sentence reply: %s", async (message) => {
    const { a } = make("healthy");
    const frames = await collect(a.chat.send({ conversation_id: "conv-1", message }));
    const tokens = frames.filter((f) => f.event === "token");
    expect(tokens.length).toBeGreaterThanOrEqual(25);
    expect(tokens.length).toBeLessThanOrEqual(60);
    const tools = frames.filter((f) => f.event === "tool");
    expect(tools).toEqual([{ event: "tool", data: { name: "kb.read" } }]);
    const at = frames.findIndex((f) => f.event === "tool");
    expect(frames.slice(0, at).some((f) => f.event === "token")).toBe(true);
    expect(frames.slice(at).some((f) => f.event === "token")).toBe(true);
    const final = last(frames)!;
    expect(final.event).toBe("done");
    expect(frames.filter((f) => f.event === "done" || f.event === "error")).toHaveLength(1);
    const streamed = tokens.map((f) => (f.data as { text: string }).text).join("");
    const done = final.data as { conversation_id: string; reply: string; tool_trace: { name: string; ok: boolean }[] };
    expect(done.conversation_id).toBe("conv-1");
    expect(done.tool_trace).toEqual([{ name: "kb.read", ok: true }]);
    expect(done.reply).not.toBe(streamed.trim()); // the reply is only the final sentence, unlike the deltas
    expect(done.reply.length).toBeLessThan(streamed.length / 2 + 80);
    expect(streamed.trim().endsWith(done.reply)).toBe(true);
    expect(done.reply.match(/[.!?]/g)).toHaveLength(1);
  });

  it("paces tokens at about 40 ms", async () => {
    const { a, clock } = make("healthy", { tokenDelayMs: 40 });
    const seen: string[] = [];
    const run = (async () => {
      for await (const f of a.chat.send({ conversation_id: "conv-1", message: "hello" })) seen.push(f.event);
    })();
    await flush();
    await clock.advanceAsync(400, 10);
    const tokensAt400 = seen.filter((e) => e === "token").length;
    expect(tokensAt400).toBeGreaterThanOrEqual(5);
    expect(tokensAt400).toBeLessThanOrEqual(12);
    expect(seen).not.toContain("done");
    await clock.advanceAsync(6_000, 10);
    await run;
    expect(last(seen)).toBe("done");
  });

  it("stream:false yields one done with the final sentence", async () => {
    const { a } = make("healthy");
    const frames = await collect(a.chat.send({ conversation_id: "conv-2", message: "radio please", stream: false }));
    expect(frames).toHaveLength(1);
    expect(frames[0]).toMatchObject({ event: "done", data: { conversation_id: "conv-2", reply: finalSentence(pickReply("radio").post) } });
  });

  it("validates the request like the daemon", async () => {
    const { a } = make("healthy");
    const e1 = await failure(collect(a.chat.send({ conversation_id: "bad id!", message: "hi" })));
    expect(e1).toMatchObject({ status: 400, code: "bad_request" });
    expect(e1.problem.errors?.[0]?.pointer).toBe("/conversation_id");
    expect(await failure(collect(a.chat.send({ conversation_id: "c", message: "" })))).toMatchObject({ code: "bad_request" });
    expect(await failure(collect(a.chat.send({ conversation_id: "c", message: "x".repeat(16001) })))).toMatchObject({ code: "bad_request" });
  });

  it("stops when the caller aborts", async () => {
    const { a } = make("healthy");
    const ctl = new AbortController();
    const seen: string[] = [];
    const err = await (async () => {
      for await (const f of a.chat.send({ conversation_id: "c", message: "hi" }, { signal: ctl.signal })) {
        seen.push(f.event);
        if (seen.length === 3) ctl.abort();
      }
    })().catch((e: unknown) => e);
    expect(seen).toHaveLength(3);
    expect((err as Error).name).toBe("AbortError");
  });

  it("records the turn in OpenAI-shaped history, including rows the UI must hide", async () => {
    const { a } = make("healthy");
    expect((await a.chat.history("conv-9")).messages).toEqual([]);
    await collect(a.chat.send({ conversation_id: "conv-9", message: "hi" }));
    const { messages } = await a.chat.history("conv-9");
    expect(messages.map((m) => m.role)).toEqual(["user", "assistant", "tool", "assistant"]);
    expect(messages[0]!.content).toBe("hi");
    expect(messages[1]!.tool_calls).toHaveLength(1);
    expect(messages[3]!.content).toBe(finalSentence(pickReply("hi").post));
  });

  it("tokenizes into words that keep their spacing", () => {
    expect(tokenize("a bb  c").join("")).toBe("a bb  c");
    expect(finalSentence("One. Two three! Four five.")).toBe("Four five.");
  });
});

describe("mock actions", () => {
  it("decide: 404, 409 and success", async () => {
    const { a } = make("healthy");
    expect(await failure(a.decide("nope", "accepted"))).toMatchObject({ status: 404, code: "not_found" });
    const done = await a.decide("prop-01j9z3k8m2q4", "dismissed");
    expect(done.decision).toMatchObject({ decision: "dismissed" });
    expectValid("decideProposal", done);
    expect(await failure(a.decide("prop-01j9z3k8m2q4", "accepted"))).toMatchObject({ status: 409, code: "already_decided" });
    expect(await a.proposals("open")).toHaveLength(0);
    expect(await a.proposals("decided")).toHaveLength(3);
    expect(await failure(a.decide("prop-01j9y7c2d9f1", "maybe" as never))).toMatchObject({ code: "bad_request" });
  });

  it("agentTask: submits, then the agent writes a proposal and announces it", async () => {
    const { a, clock } = make("healthy");
    const { frames } = listen(a);
    await flush();
    const accepted = await a.agentTask({ kind: "propose", note: "tune it" });
    expect(accepted.task_id).toMatch(/^task-/);
    await clock.advanceAsync(4_000, 100);
    expect(frames.filter((f) => f.event === "agent").map((f) => (f.data as { kind: string }).kind)).toEqual(expect.arrayContaining(["task.submitted", "task.done", "proposal.written"]));
    expect(frames.filter((f) => f.event === "proposal")).toHaveLength(1);
    expect((await a.proposals("open")).some((p) => p.task_id === accepted.task_id)).toBe(true);
    expect(await failure(a.agentTask({ kind: "patch" }))).toMatchObject({ status: 409, code: "needs_hub" });
  });

  it("patchSettings: alias trims, clears and validates; lan_enabled waits for a restart", async () => {
    const { a, clock } = make("healthy");
    expect(await a.patchSettings({ alias: "  Garage  " })).toMatchObject({ alias: "Garage" });
    expect((await a.status()).node.alias).toBe("Garage");
    expect(await a.patchSettings({ alias: "" })).toMatchObject({ alias: null });
    expect(await a.patchSettings({ alias: null })).toMatchObject({ alias: null });
    const e = await failure(a.patchSettings({ alias: "x".repeat(65) }));
    expect(e).toMatchObject({ code: "bad_request" });
    expect(e.problem.errors?.[0]?.pointer).toBe("/alias");
    await a.patchSettings({ lan_enabled: false });
    expect((await a.settings()).lan_enabled).toBe(false);
    expect((await a.status()).lan_enabled).toBe(false);
    expect((await a.status()).bind.lan).toBe(true); // takes effect on restart
    await a.restart();
    await clock.advanceAsync(3_000, 100);
    expect((await a.status()).bind.lan).toBe(false);
    expect((await a.status()).bind.urls).not.toContain("http://111.local");
  });

  it("updateCheck: checking, then idle with a fresh last_check; a second call is not started", async () => {
    const { a, clock } = make("healthy");
    const { frames } = listen(a);
    await flush();
    expect(await a.updateCheck()).toEqual({ schema: 1, started: true });
    expect((await a.status()).update.state).toBe("checking");
    expect(await a.updateCheck()).toEqual({ schema: 1, started: false });
    await clock.advanceAsync(1_600, 100);
    const u = (await a.status()).update;
    expect(u).toMatchObject({ state: "idle", last_result: "up_to_date" });
    expect(Date.parse(u.last_check!)).toBe(clock.now() - 100);
    const states = frames.filter((f) => f.event === "status").map((f) => (f.data as { update: { state: string } }).update.state);
    expect(states).toEqual(["checking", "idle"]);
  });

  it("restart: the daemon drops for a moment, comes back with a reset, children start then go live", async () => {
    const { a, clock } = make("healthy");
    const { frames, states } = listen(a);
    await flush();
    await clock.advanceAsync(20_000, 500);
    const before = await a.status();
    expect(await a.restart()).toEqual({ schema: 1, restarting: true });
    await clock.advanceAsync(400, 100);
    expect((await failure(a.status())).code).toBe("network");
    expect(states.slice(-1)).toEqual(["connecting"]);
    await clock.advanceAsync(2_500, 100);
    expect(states.slice(-1)).toEqual(["open"]);
    expect(last(frames)).toEqual({ event: "reset", data: { reason: "restart" } });
    expect((await a.status()).node.uptime_sec).toBeLessThan(before.node.uptime_sec);
    expect((await a.service("agent")).state).toBe("starting");
    expect((await a.service("kb")).state).toBe("live");
    await clock.advanceAsync(6_100, 100);
    expect((await a.service("agent")).state).toBe("live");
    expect(last(frames)!.event).toBe("services");
  });

  it("restartService: starting now, live again after 1.5 s, one services frame each", async () => {
    const { a, clock } = make("healthy");
    const { frames } = listen(a);
    await flush();
    const before = frames.length;
    const reply = await a.restartService("model");
    expect(reply).toEqual({ schema: 1, id: "model", restarting: true });
    expectValid("restartService", reply);
    expect((await a.service("model")).state).toBe("starting");
    expect(frames.slice(before).map((f) => f.event)).toEqual(["services"]);
    await clock.advanceAsync(1_400, 100);
    expect((await a.service("model")).state).toBe("starting");
    await clock.advanceAsync(200, 100);
    expect(await a.service("model")).toMatchObject({ state: "live", reason_code: "health_ok" });
    expect(frames.slice(before).filter((f) => f.event === "services")).toHaveLength(2);
    for (const f of frames) expect(validateAgainst(sse.events[f.event]!, f.data), f.event).toEqual([]);
  });

  it("restartService: an errored child comes back healthy and the agent works again", async () => {
    const { a, clock } = make("agent-down");
    expect((await failure(a.agentStatus())).code).toBe("agent_unavailable");
    await a.restartService("agent");
    await clock.advanceAsync(1_600, 100);
    expect((await a.service("agent")).state).toBe("live");
    expect((await a.status()).intelligence.agent).toBe("running");
    expect((await a.agentStatus()).state).toBe("local");
  });

  it("restartService: 409 not_supervised without the restart action, 404 for an unknown id", async () => {
    const { a } = make("healthy");
    const e = await failure(a.restartService("kb"));
    expect(e).toMatchObject({ status: 409, code: "not_supervised" });
    expect(validateAgainst("Problem", e.problem)).toEqual([]);
    expect(await failure(a.restartService("nope" as never))).toMatchObject({ status: 404, code: "not_found" });
  });

  it("lan-locked: settings say so, a PATCH of lan_enabled answers 409 settings_locked and changes nothing", async () => {
    const { a } = make("lan-locked");
    expect(await a.settings()).toMatchObject({ lan_enabled: false, lan_locked: true });
    const e = await failure(a.patchSettings({ lan_enabled: true, alias: "Roof" }));
    expect(e).toMatchObject({ status: 409, code: "settings_locked" });
    expect(validateAgainst("Problem", e.problem)).toEqual([]);
    expect(await a.settings()).toMatchObject({ lan_enabled: false, alias: "Workshop laptop" }); // not even the alias half applied
    expect(await a.patchSettings({ alias: "Roof" })).toMatchObject({ alias: "Roof", lan_locked: true });
  });

  it("kb: refuses traversal, 404s unknown paths", async () => {
    const { a } = make("healthy");
    for (const p of ["../etc/passwd", "projects/../../x", "/etc/passwd", "a\\..\\b", ""]) expect(await failure(a.kb.entry(p)), p).toMatchObject({ status: 400, code: "bad_request" });
    expect(await failure(a.kb.entry("projects/nothing.md"))).toMatchObject({ status: 404, code: "not_found" });
    expect(await a.kb.entry("projects/meshtastic.md")).toContain("Meshtastic");
  });

  it("serviceLog: clamps n, 404s unknown ids, reports source none without a log", async () => {
    const { a } = make("healthy");
    expect((await a.serviceLog("agent", 5)).lines).toHaveLength(5);
    expect((await a.serviceLog("agent")).lines).toHaveLength(100);
    expect((await a.serviceLog("agent", 9999)).lines.length).toBeLessThanOrEqual(500);
    expect((await a.serviceLog("agent", 0)).lines).toHaveLength(1);
    expect(await a.serviceLog("kb")).toMatchObject({ source: "none", lines: [] });
    expect(await failure(a.service("bogus" as ServiceId))).toMatchObject({ status: 404, code: "not_found" });
    expect(await failure(a.serviceLog("bogus" as ServiceId))).toMatchObject({ code: "not_found" });
  });

  it("returns copies, so callers cannot corrupt the mock", async () => {
    const { a } = make("healthy");
    const s = await a.services();
    s[0]!.state = "error";
    expect((await a.services())[0]!.state).toBe("live");
  });
});

describe("mock event stream", () => {
  it("starts with reset, then status every 30 s with a growing uptime and monotonic ids", async () => {
    const { a, clock } = make("healthy");
    const { frames, states } = listen(a);
    await flush();
    expect(states).toEqual(["connecting", "open"]);
    expect(frames).toEqual([{ event: "reset", data: { reason: "restart" } }]);
    await clock.advanceAsync(95_000, 500);
    const status = frames.filter((f) => f.event === "status");
    expect(status).toHaveLength(3);
    const up = status.map((f) => (f.data as { node: { uptime_sec: number } }).node.uptime_sec);
    expect(up[1]! - up[0]!).toBe(30);
    expect(up[2]! - up[1]!).toBe(30);
    const ids = frames.filter((f) => f.id).map((f) => Number(f.id));
    expect(ids).toEqual([...ids].sort((x, y) => x - y));
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("dropConnection reconnects with replay of what was missed (no reset)", async () => {
    const { a, clock } = make("healthy");
    const { frames, states } = listen(a);
    await flush();
    await clock.advanceAsync(30_500, 500); // one status heartbeat delivered
    const seen = frames.length;
    a.mock.dropConnection(40_000);
    expect(states.slice(-1)).toEqual(["connecting"]);
    await clock.advanceAsync(41_000, 500); // one heartbeat falls inside the outage
    expect(states.slice(-1)).toEqual(["open"]);
    expect(frames.slice(seen).map((f) => f.event)).toEqual(["status"]);
    expect(frames.some((f, i) => i > 0 && f.event === "reset")).toBe(false);
  });
});
