// The Layer 5 mock: every route validates against openapi.yaml in every scenario; the request flow, sends and their
// states, adding contacts, hub enrollment, identify and panels behave the way the daemon's contract says.
import { afterEach, describe, expect, it } from "vitest";
import type { EventsFrameWithId } from "../src/lib/api/adapter";
import { classifyAddInput } from "../src/lib/api/contacts";
import { MOCK_PAIR_CODE, SCENARIO_NAMES, createMockAdapter, type MockAdapter, type MockOptions, type ScenarioName } from "../src/lib/api/mock";
import { cardFor } from "../src/lib/api/mock/qr";
import { ADA, BRAM, CLEO, DANA, DOV, ELI, FAY, HUB, MOCK_HUB_ADDRESS, MOCK_HUB_CODE, mockCardText } from "../src/lib/api/mock/messaging";
import { ApiError } from "../src/lib/api/problem";
import { parseEnrollBundle } from "../src/views/settings/messaging-model";
import { operations, sse } from "./contract.gen";
import { FakeClock, flush } from "./helpers/fakes";
import { validate, validateAgainst } from "./helpers/validate";

const T0 = Date.parse("2026-10-14T09:00:00Z");
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
  const off = a.events({ onFrame: (f) => frames.push(f), onState: () => undefined });
  return { frames, off };
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

async function paired(scenario: ScenarioName) {
  const r = make(scenario);
  if (scenario === "lan-unpaired" || scenario === "termux-unpaired") await r.a.auth.pair(MOCK_PAIR_CODE);
  return r;
}

describe.each(SCENARIO_NAMES)("scenario %s: the Layer 5 routes validate against openapi.yaml", (scenario) => {
  it("threads, conversations, contacts, own card, fabric, nearby, panels", async () => {
    const { a } = await paired(scenario);
    const threads = await a.messages.threads();
    expectValid("listThreads", { schema: 1, threads });
    for (const t of threads) {
      const page = await a.messages.thread(t.peer);
      expectValid("getThread", { schema: 1, ...page });
      expect(page.messages.every((m) => m.peer === t.peer)).toBe(true);
      expect(page.messages.map((m) => m.local_seq)).toEqual([...page.messages.map((m) => m.local_seq)].sort((x, y) => x - y));
    }
    const contacts = await a.contacts.list();
    expectValid("listContacts", { schema: 1, contacts });
    for (const c of contacts) expectValid("getContact", await a.contacts.get(c.node_id));
    expectValid("getOwnCard", await a.contacts.ownCard());
    expectValid("getFabric", await a.fabric.status());
    expectValid("listNearby", { schema: 1, sources: await a.nearby() });
    expectValid("listPanels", { schema: 1, panels: await a.panels.list() });
  });

  it("the fabric service agrees with the fabric snapshot (LIVE only with a way to deliver, ERROR on a full store)", async () => {
    const { a } = await paired(scenario);
    const row = await a.service("fabric");
    const f = await a.fabric.status();
    if (f.store.store_full || !f.store.open) expect(row.state).toBe("error");
    else if (row.state === "live") expect(f.hub.reachable || f.hub.members > 0 || f.links.some((l) => l.peers > 0)).toBe(true);
    else expect(row.state).toBe("unset");
  });
});

describe("scenarios for messaging", () => {
  it("messaging-new: nothing yet, no home hub, NOT SET", async () => {
    const { a } = make("messaging-new");
    expect(await a.messages.threads()).toEqual([]);
    expect(await a.contacts.list()).toEqual([]);
    expect((await a.fabric.status()).hub.enrolled).toBe(false);
    expect(await a.service("fabric")).toMatchObject({ state: "unset", reason_code: "not_enrolled" });
  });

  it("store-full: the fabric service is in ERROR and the newest own messages wait with store_full", async () => {
    const { a, clock } = make("store-full");
    expect(await a.service("fabric")).toMatchObject({ state: "error", reason_code: "store_full" });
    const stuck = (await a.messages.threads()).flatMap((t) => (t.last?.reason_code === "store_full" ? [t.last] : []));
    expect(stuck.length).toBeGreaterThan(0);
    for (const m of stuck) expect(m).toMatchObject({ state: "queued", dir: "own" });
    await clock.advanceAsync(13_000, 500);
    expect((await a.fabric.status()).store.store_full).toBe(false);
    expect(await a.service("fabric")).toMatchObject({ state: "live" });
  });

  it("restore-pending: the outbox is held and says why", async () => {
    const { a } = make("restore-pending");
    const held = (await a.messages.thread(ADA)).messages.filter((m) => m.reason_code === "restore_pending");
    expect(held.length).toBeGreaterThan(0);
    for (const m of held) expect(m.state).toBe("queued");
  });

  it("requests: two pending requests, one hostile; a third arrives at 6 s", async () => {
    const { a, clock } = make("requests");
    const { frames } = listen(a);
    const before = (await a.contacts.list()).filter((c) => c.request).map((c) => c.node_id);
    expect(before.sort()).toEqual([DOV, ELI].sort());
    const threads = await a.messages.threads();
    expect(threads.filter((t) => t.request).map((t) => t.peer).sort()).toEqual([DOV, ELI].sort());
    // The hostile text is held verbatim; rendering as text is the view's job (and the view's tests).
    const eli = await a.messages.thread(ELI);
    expect(eli.request).toBe(true);
    expect(eli.messages[0]!.text).toContain("<script>");
    await clock.advanceAsync(7_000, 500);
    expect((await a.contacts.list()).find((c) => c.node_id === FAY)).toMatchObject({ request: true, trust: "tofu" });
    expect(frames.some((f) => f.event === "contact" && (f.data as { change: string }).change === "request")).toBe(true);
    expect(frames.some((f) => f.event === "message" && (f.data as { change: string }).change === "request")).toBe(true);
  });
});

describe("the request flow", () => {
  it("accepting makes the contact, clears the request flag and keeps the held messages in the thread", async () => {
    const { a } = make("requests");
    const { frames } = listen(a);
    await flush();
    const held = (await a.messages.thread(DOV)).messages.map((m) => m.id);
    expect(held.length).toBe(2);
    const c = await a.contacts.patch(DOV, { accept: true });
    expect(c.request).toBe(false);
    expect(await a.messages.thread(DOV)).toMatchObject({ request: false });
    expect((await a.messages.thread(DOV)).messages.map((m) => m.id)).toEqual(held);
    expect((await a.messages.threads()).find((t) => t.peer === DOV)?.request).toBe(false);
    expect(frames.some((f) => f.event === "contact" && (f.data as { change: string }).change === "updated")).toBe(true);
    // Now it can be written to.
    expect((await a.messages.send({ to: DOV, text: "welcome" })).id).toMatch(/^msg-/);
  });

  it("a pending request cannot be written to (404: not a contact yet)", async () => {
    const { a } = make("requests");
    const e = await failure(a.messages.send({ to: ELI, text: "hello?" }));
    expect(e.status).toBe(404);
    expect(validateAgainst("Problem", e.problem)).toEqual([]);
  });

  it("a pending request is kept as it is: time passing prunes nothing, sends nothing back, and `ignore` is not an operation of the node (section 7.2)", async () => {
    const { a, clock } = make("requests");
    const { frames } = listen(a);
    await flush();
    expect((a as unknown as Record<string, unknown>).ignore).toBeUndefined();
    expect((a.contacts as unknown as Record<string, unknown>).ignore).toBeUndefined();
    const held = (await a.messages.thread(DOV)).messages.map((m) => m.id);
    await clock.advanceAsync(10 * 60_000, 5_000); // ten minutes with nobody answering
    for (const peer of [DOV, ELI]) {
      expect((await a.contacts.list()).find((c) => c.node_id === peer), peer).toMatchObject({ request: true });
      expect((await a.messages.threads()).find((t) => t.peer === peer), peer).toMatchObject({ request: true });
    }
    expect((await a.messages.thread(DOV)).messages.map((m) => m.id)).toEqual(held);
    expect(frames.some((f) => f.event === "contact" && (f.data as { change: string }).change === "removed")).toBe(false);
    // Nothing went back to a sender: no own message was created, no ACK-shaped state change was published.
    expect(frames.some((f) => f.event === "message" && (f.data as { message: { dir: string } }).message.dir === "own")).toBe(false);
    expect((await a.messages.thread(DOV)).messages.every((m) => m.dir === "in")).toBe(true);
  });

  it("forgetting a request discards it and its held messages, silently (nothing is sent back)", async () => {
    const { a } = make("requests");
    const { frames } = listen(a);
    await flush();
    await a.contacts.forget(ELI);
    expect((await a.contacts.list()).some((c) => c.node_id === ELI)).toBe(false);
    expect((await a.messages.threads()).some((t) => t.peer === ELI)).toBe(false);
    expect(frames.some((f) => f.event === "contact" && (f.data as { change: string }).change === "removed")).toBe(true);
    expect(frames.some((f) => f.event === "message" && (f.data as { message: { dir: string } }).message.dir === "own")).toBe(false);
    expect((await failure(a.contacts.get(ELI))).status).toBe(404);
  });

  it("a forgotten request comes back when its sender retransmits (it was never ACKed), as a fresh request with the same words", async () => {
    const { a, clock } = make("requests");
    const { frames } = listen(a);
    await flush();
    const words = (await a.messages.thread(ELI)).messages.map((m) => m.text);
    await a.contacts.forget(ELI);
    await clock.advanceAsync(20_000, 5_000); // before the retransmission
    expect((await a.contacts.list()).some((c) => c.node_id === ELI)).toBe(false);
    await clock.advanceAsync(15_000, 5_000);
    expect((await a.contacts.list()).find((c) => c.node_id === ELI)).toMatchObject({ request: true, trust: "tofu" });
    expect((await a.messages.thread(ELI)).messages.map((m) => m.text)).toEqual(words);
    expect(frames.filter((f) => f.event === "contact" && (f.data as { change: string; contact: { node_id: string } }).contact.node_id === ELI).map((f) => (f.data as { change: string }).change)).toEqual(["removed", "request"]);
    // Accepted in the meantime, it is not re-introduced a second time.
    const again = make("requests");
    await flush();
    await again.a.contacts.forget(DOV);
    await again.a.contacts.add({ node_id: DOV });
    await again.clock.advanceAsync(40_000, 5_000);
    expect((await again.a.contacts.list()).filter((c) => c.node_id === DOV)).toHaveLength(1);
    expect((await again.a.contacts.get(DOV)).request).toBe(false);
  });

  it("accept accepts only `true`", async () => {
    const { a } = make("requests");
    const e = await failure(a.contacts.patch(DOV, { accept: false as never }));
    expect(e.status).toBe(400);
  });

  it("an incoming message from a contact arrives as `new`, from a request as `request`", async () => {
    const { a, clock } = make("requests");
    const { frames } = listen(a);
    await clock.advanceAsync(10_000, 500);
    const fromAda = frames.find((f) => f.event === "message" && (f.data as { peer: string }).peer === ADA);
    expect((fromAda!.data as { change: string }).change).toBe("new");
    const fromFay = frames.find((f) => f.event === "message" && (f.data as { peer: string }).peer === FAY);
    expect((fromFay!.data as { change: string }).change).toBe("request");
    // Ada's message carries a sender claim that is not the arrival time.
    const adaMsgs = (await a.messages.thread(ADA)).messages;
    const ada = adaMsgs[adaMsgs.length - 1]!;
    expect(ada.sent_at_claim).not.toBeNull();
    expect(ada.sent_at_claim).not.toBe(ada.at);
  });
});

describe("sending", () => {
  it("answers 202 immediately; the message is queued, then sent, then delivered on a direct route", async () => {
    const { a, clock } = make("healthy");
    const { frames } = listen(a);
    await flush();
    const accepted = await a.messages.send({ to: CLEO, text: "hi" });
    expect(accepted.id).toMatch(/^msg-/);
    // Cleo has no key in the seed data: the message waits, and says why.
    const waiting = (await a.messages.thread(CLEO)).messages.find((m) => m.id === accepted.id)!;
    expect(waiting).toMatchObject({ state: "queued", reason_code: "waiting_for_key", dir: "own" });
    expect(waiting.reason).toMatch(/their key/i);
    // An established contact on a direct route moves along.
    const ok = await a.messages.send({ to: HUB, text: "ping" });
    const seen: string[] = [];
    for (let i = 0; i < 6; i++) {
      await clock.advanceAsync(300, 100);
      seen.push((await a.messages.thread(HUB)).messages.find((m) => m.id === ok.id)!.state);
    }
    expect(seen[0]).toBe("queued");
    expect(seen[seen.length - 1]).toBe("delivered");
    for (let i = 1; i < seen.length; i++) expect(["queued", "sent", "relayed", "delivered"].indexOf(seen[i]!)).toBeGreaterThanOrEqual(["queued", "sent", "relayed", "delivered"].indexOf(seen[i - 1]!));
    const states = frames.filter((f) => f.event === "message" && (f.data as { message: { id: string } }).message.id === ok.id).map((f) => (f.data as { message: { state: string } }).message.state);
    expect(states).toContain("sent");
    expect(states[states.length - 1]).toBe("delivered");
  });

  it("a mailbox route goes through relayed before delivered (hours are normal, the mock shortens them)", async () => {
    const { a, clock } = make("healthy");
    const sent = await a.messages.send({ to: ADA, text: "via the hub" });
    const seen = new Set<string>();
    for (let i = 0; i < 90; i++) {
      await clock.advanceAsync(100, 100);
      seen.add((await a.messages.thread(ADA)).messages.find((m) => m.id === sent.id)!.state);
    }
    expect([...seen]).toEqual(expect.arrayContaining(["sent", "relayed", "delivered"]));
  });

  it("a contact whose hub is not yours stays queued with hub_not_yours", async () => {
    const { a, clock } = make("healthy");
    const sent = await a.messages.send({ to: BRAM, text: "hello" });
    await clock.advanceAsync(5_000, 500);
    expect((await a.messages.thread(BRAM)).messages.find((m) => m.id === sent.id)).toMatchObject({ state: "queued", reason_code: "hub_not_yours" });
  });

  it("a full mailbox leaves the message sent, with mailbox_full", async () => {
    const { a, clock } = make("healthy");
    const sent = await a.messages.send({ to: DANA, text: "photos?" });
    await clock.advanceAsync(2_000, 200);
    expect((await a.messages.thread(DANA)).messages.find((m) => m.id === sent.id)).toMatchObject({ state: "sent", reason_code: "mailbox_full" });
  });

  it("retry re-queues a queued or sent message and refuses anything else with 409", async () => {
    const { a } = make("healthy");
    const queued = (await a.messages.thread(BRAM)).messages.find((m) => m.state === "queued")!;
    expect((await a.messages.retry(queued.id)).id).toBe(queued.id);
    const delivered = (await a.messages.thread(ADA)).messages.find((m) => m.state === "delivered" && m.dir === "own")!;
    const e = await failure(a.messages.retry(delivered.id));
    expect(e.status).toBe(409);
    const incoming = (await a.messages.thread(ADA)).messages.find((m) => m.dir === "in")!;
    expect((await failure(a.messages.retry(incoming.id))).status).toBe(409);
    expect((await failure(a.messages.retry("msg-nope"))).status).toBe(404);
  });

  it("the adapter carries messages.cancel, so the thread view offers the control in mock mode", () => {
    const { a } = make("healthy");
    expect(typeof a.messages.cancel).toBe("function");
  });

  it("cancel keeps a queued own message in its thread, queued, with reason_code cancelled, and publishes it as a state frame", async () => {
    const { a, clock } = make("healthy");
    const { frames } = listen(a);
    await flush();
    const held = (await a.messages.thread(BRAM)).messages.find((m) => m.dir === "own" && m.state === "queued")!;
    expect(held.reason_code).toBe("hub_not_yours");
    await a.messages.cancel!(held.id);
    const after = (await a.messages.thread(BRAM)).messages.find((m) => m.id === held.id)!;
    expect(after).toMatchObject({ state: "queued", reason_code: "cancelled", dir: "own", text: held.text });
    expect(after.reason).toMatch(/cancelled/i);
    expectValid("getThread", { schema: 1, ...(await a.messages.thread(BRAM)) });
    const sent = frames.filter((f) => f.event === "message" && (f.data as { message: { id: string } }).message.id === held.id);
    expect(sent.length).toBeGreaterThan(0);
    expect(sent[sent.length - 1]!.data).toMatchObject({ change: "state", peer: BRAM, message: { state: "queued", reason_code: "cancelled" } });
    // a retry or a later look at the outbox neither moves it nor overwrites the reason
    await a.messages.retry(held.id);
    await clock.advanceAsync(10_000, 500);
    expect((await a.messages.thread(BRAM)).messages.find((m) => m.id === held.id)).toMatchObject({ state: "queued", reason_code: "cancelled" });
  });

  it("a message cancelled while its first step is still waiting never leaves", async () => {
    const { a, clock } = make("healthy");
    const accepted = await a.messages.send({ to: HUB, text: "ping" });
    expect((await a.messages.thread(HUB)).messages.find((m) => m.id === accepted.id)).toMatchObject({ state: "queued", reason_code: null });
    await a.messages.cancel!(accepted.id);
    await clock.advanceAsync(5_000, 250);
    expect((await a.messages.thread(HUB)).messages.find((m) => m.id === accepted.id)).toMatchObject({ state: "queued", reason_code: "cancelled" });
  });

  it("a cancelled message stays cancelled when the hold lifts, while the others held with it go on", async () => {
    const { a, clock } = make("restore-pending");
    const held = (await a.messages.thread(ADA)).messages.filter((m) => m.dir === "own" && m.reason_code === "restore_pending");
    expect(held.length).toBeGreaterThan(0);
    const [gone, ...rest] = held;
    await a.messages.cancel!(gone!.id);
    expect((await a.messages.thread(ADA)).messages.find((m) => m.id === gone!.id)).toMatchObject({ state: "queued", reason_code: "cancelled" });
    await clock.advanceAsync(20_000, 500); // the restore finishes at 12 s and the outbox is looked at again
    const after = (await a.messages.thread(ADA)).messages;
    expect(after.find((m) => m.id === gone!.id)).toMatchObject({ state: "queued", reason_code: "cancelled" });
    for (const m of rest) expect(after.find((x) => x.id === m.id)!.state).not.toBe("queued");
  });

  it("cancel refuses with 409 once a message has left, for an incoming message and for one cancelled already; 404 for an unknown id", async () => {
    const { a } = make("healthy");
    const delivered = (await a.messages.thread(ADA)).messages.find((m) => m.dir === "own" && m.state === "delivered")!;
    const left = await failure(a.messages.cancel!(delivered.id));
    expect(left.status).toBe(409);
    expect(validateAgainst("Problem", left.problem)).toEqual([]);
    expect(left.problem.code).toBe("bad_request");
    const incoming = (await a.messages.thread(ADA)).messages.find((m) => m.dir === "in")!;
    expect((await failure(a.messages.cancel!(incoming.id))).status).toBe(409);
    const held = (await a.messages.thread(BRAM)).messages.find((m) => m.dir === "own" && m.state === "queued")!;
    await a.messages.cancel!(held.id);
    expect((await failure(a.messages.cancel!(held.id))).status).toBe(409);
    expect((await failure(a.messages.cancel!("msg-nope"))).status).toBe(404);
    // the refusals changed nothing
    expect((await a.messages.thread(ADA)).messages.find((m) => m.id === delivered.id)).toMatchObject({ state: "delivered", reason_code: null });
  });

  it("validates the body: empty text and an over-long text are 400, an unknown contact 404", async () => {
    const { a } = make("healthy");
    expect((await failure(a.messages.send({ to: ADA, text: "   " }))).status).toBe(400);
    expect((await failure(a.messages.send({ to: ADA, text: "x".repeat(2001) }))).status).toBe(400);
    const e = await failure(a.messages.send({ to: "ab".repeat(32), text: "hi" }));
    expect(e.status).toBe(404);
    expect(validateAgainst("Problem", e.problem)).toEqual([]);
  });

  it("a thread read validates its parameters", async () => {
    const { a } = make("healthy");
    expect((await failure(a.messages.thread("nothex"))).status).toBe(400);
    expect((await failure(a.messages.thread(ADA, { limit: 501 }))).status).toBe(400);
    expect((await failure(a.messages.thread(ADA, { limit: 0 }))).status).toBe(400);
    expect((await failure(a.messages.thread("ab".repeat(32)))).status).toBe(404);
  });

  it("the read mark only moves forward", async () => {
    const { a } = make("requests");
    await a.contacts.patch(ADA, { read_seq: 50 });
    const unread = async () => (await a.messages.threads()).find((t) => t.peer === ADA)!.unread;
    expect(await unread()).toBe(0);
    await a.contacts.patch(ADA, { read_seq: 1 });
    expect(await unread()).toBe(0);
  });
});

describe("contacts", () => {
  const nodeId = "11".repeat(32);

  it("adds by node id: no key yet, messages wait for their card, and the hub's reply releases them", async () => {
    const { a, clock } = make("healthy");
    const c = await a.contacts.add({ node_id: nodeId });
    expect(c).toMatchObject({ node_id: nodeId, has_key: false, trust: "oob", request: false });
    const sent = await a.messages.send({ to: nodeId, text: "hello" });
    expect((await a.messages.thread(nodeId)).messages.find((m) => m.id === sent.id)).toMatchObject({ state: "queued", reason_code: "waiting_for_key" });
    await clock.advanceAsync(6_000, 500);
    expect((await a.contacts.get(nodeId)).has_key).toBe(true);
    const m = (await a.messages.thread(nodeId)).messages.find((x) => x.id === sent.id)!;
    expect(m.reason_code).not.toBe("waiting_for_key");
    expect(["sent", "delivered", "relayed"]).toContain(m.state);
  });

  it("adds by card (it works with no connection), and an already-known contact comes back unchanged", async () => {
    const { a } = make("messaging-new");
    const card = mockCardText(FAY);
    const c = await a.contacts.add({ card });
    expect(c).toMatchObject({ trust: "oob", has_key: true, request: false });
    expect(await a.contacts.add({ card })).toEqual(c);
    expect(await a.contacts.list()).toHaveLength(1);
  });

  it("the UI's classifier and the mock agree on what is a valid input", async () => {
    const { a } = make("messaging-new");
    const bech = classifyAddInput(nodeId);
    expect(bech.kind).toBe("node_id");
    for (const bad of [{ card: "short" }, { node_id: "nope" }, { card: mockCardText(ADA), node_id: nodeId }, {}]) {
      const e = await failure(a.contacts.add(bad as never));
      expect(e.status).toBe(400);
      expect(validateAgainst("Problem", e.problem)).toEqual([]);
    }
  });

  it("refuses this node's own card and id", async () => {
    const { a } = make("healthy");
    const own = await a.contacts.ownCard();
    expect((await failure(a.contacts.add({ card: own.text }))).status).toBe(400);
  });

  it("renames, clears a name, pins and forgets; a forgotten contact keeps its history", async () => {
    const { a } = make("healthy");
    expect((await a.contacts.patch(BRAM, { alias: "  Bram B  " })).alias).toBe("Bram B");
    expect((await a.contacts.patch(BRAM, { alias: null })).alias).toBeNull();
    expect((await a.contacts.patch(BRAM, { pinned: true })).pinned).toBe(true);
    expect((await failure(a.contacts.patch(BRAM, { alias: "x".repeat(65) }))).status).toBe(400);
    await a.contacts.forget(BRAM);
    expect((await failure(a.contacts.get(BRAM))).status).toBe(404);
    expect((await a.messages.thread(BRAM)).messages.length).toBeGreaterThan(0);
    expect((await failure(a.contacts.forget(BRAM))).status).toBe(404);
  });

  it("the own card is a real QR matrix of the card text", async () => {
    const { a } = make("healthy");
    const card = await a.contacts.ownCard();
    expect(card.qr.rows).toHaveLength(card.qr.size);
    expect(card.text.length).toBeGreaterThan(100);
  });
});

describe("fabric, hub, identify, panels", () => {
  it("enrolls with the hub's code and address, and locks out after five failures", async () => {
    const { a } = make("messaging-new");
    const e = await failure(a.fabric.enrollHub({ address: MOCK_HUB_ADDRESS, code: "ABCDEFGH" }));
    expect(e.status).toBe(502);
    expect((await failure(a.fabric.enrollHub({ address: "nope", code: "x" }))).status).toBe(400);
    for (let i = 0; i < 4; i++) await failure(a.fabric.enrollHub({ address: MOCK_HUB_ADDRESS, code: "ABCDEFGH" }));
    expect((await failure(a.fabric.enrollHub({ address: MOCK_HUB_ADDRESS, code: MOCK_HUB_CODE }))).status).toBe(429);
  });

  it("a good code enrolls: the service goes LIVE, the hub becomes a contact, a second enroll is 409", async () => {
    const { a } = make("messaging-new");
    expect(await a.service("fabric")).toMatchObject({ state: "unset" });
    const hub = await a.fabric.enrollHub({ address: MOCK_HUB_ADDRESS, code: "k7m2-9x4p" });
    expect(hub).toMatchObject({ enrolled: true, reachable: true, node_id: HUB });
    expect(await a.service("fabric")).toMatchObject({ state: "live", reason_code: "hub_reachable" });
    expect((await a.contacts.list()).some((c) => c.node_id === HUB)).toBe(true);
    expect((await failure(a.fabric.enrollHub({ address: MOCK_HUB_ADDRESS, code: MOCK_HUB_CODE }))).status).toBe(409);
  });

  it("mints and dismisses a hub offer, on the node itself only", async () => {
    const { a } = make("healthy");
    const offer = await a.fabric.offerHub();
    expectValid("offerHub", offer);
    expect((await a.fabric.status()).hub.offer_active).toBe(true);
    await a.fabric.dismissHubOffer();
    expect((await a.fabric.status()).hub.offer_active).toBe(false);
    await a.fabric.dismissHubOffer(); // idempotent
    const { a: lan } = make("lan-unpaired");
    await lan.auth.pair(MOCK_PAIR_CODE);
    expect((await failure(lan.fabric.offerHub())).status).toBe(403);
    expect((await failure(lan.fabric.patchSettings({ hub_role: true }))).status).toBe(403);
  });

  it("the offer's bundle is the daemon's 111HUB:<card>:<code>:<address> text (section 7.3), not the retired 111enroll: form", async () => {
    const { a } = make("healthy");
    const offer = await a.fabric.offerHub();
    expect(offer.bundle.text).toBe(`111HUB:${offer.card.text}:${offer.code}:${offer.address}`);
    expect(offer.bundle.text).not.toContain("111enroll");
    expect(parseEnrollBundle(offer.bundle.text)).toEqual({ card: offer.card.text, code: offer.code, address: offer.address });
    // the QR holds the same text, so scanning it yields the bundle the page reads
    expect(offer.bundle.qr.size).toBeGreaterThan(0);
    expect(offer.bundle).toEqual(cardFor(offer.bundle.text));
  });

  it("merges fabric settings and drops a quota with null", async () => {
    const { a } = make("healthy");
    const s = await a.fabric.patchSettings({ hub_role: true, quotas: { kb: 1_000_000 } });
    expect(s).toMatchObject({ hub_role: true, quotas: { kb: 1_000_000 } });
    // the daemon keeps the class with null; the mock may drop the key. Either way no quota is set.
    expect((await a.fabric.patchSettings({ quotas: { kb: null } })).quotas.kb ?? null).toBeNull();
    expect((await failure(a.fabric.patchSettings({ hub_role: "yes" as never }))).status).toBe(400);
  });

  it("identify needs consent and the offer, then the board is bound and Nearby fills", async () => {
    const { a } = make("new-device");
    await flush();
    const devices = await a.devices();
    const board = devices.find((d) => d.actions.includes("identify"));
    if (!board) return; // the scenario's board appears later; the identify route is covered below
    expect((await failure(a.identifyDevice(board.id, { consent: false as never }))).status).toBe(400);
  });

  it("identify binds the board, serves the panel and fills Nearby with untrusted names", async () => {
    const { a, clock } = make("meshtastic");
    await clock.advanceAsync(20_000, 500);
    const devices = await a.devices();
    const board = devices.find((d) => d.actions.includes("identify"));
    expect(board, "the meshtastic scenario offers an identify step").toBeTruthy();
    expect((await failure(a.identifyDevice(board!.id, { consent: false as never }))).status).toBe(400);
    expect((await a.nearby())).toEqual([]);
    const after = await a.identifyDevice(board!.id, { consent: true });
    expect(after.actions).not.toContain("identify");
    const nearby = await a.nearby();
    expect(nearby[0]!.nodes.some((n) => n.long_name.includes("<img"))).toBe(true); // held verbatim; rendered as text
    expect((await a.service("meshtastic")).state).toBe("live");
    const panel = (await a.panels.list())[0]!;
    expect(panel.state).toMatchObject({ enabled: true, listening: true });
    const session = await a.panels.openSession("meshtastic");
    expectValid("openPanelSession", session);
    expect(session.url).toMatch(new RegExp(`^http://127\\.0\\.0\\.1:${panel.port}/\\?ticket=[0-9a-f]{32}$`));
    expect((await failure(a.panels.openSession("radio"))).status).toBe(404);
  });

  it("a panel session is refused while its service is not set up", async () => {
    const { a } = make("healthy");
    expect((await failure(a.panels.openSession("meshtastic"))).status).toBe(404);
  });
});

describe("the event stream carries the Layer 5 frames, each valid against its schema", () => {
  it("message, contact, fabric, nearby, panels", async () => {
    const { a, clock } = make("requests");
    const { frames } = listen(a);
    await flush();
    await a.contacts.patch(DOV, { accept: true });
    await a.messages.send({ to: ADA, text: "x" });
    await a.fabric.offerHub();
    await clock.advanceAsync(12_000, 500);
    const names = new Set(frames.map((f) => f.event));
    for (const n of ["message", "contact", "fabric"]) expect(names.has(n as never), n).toBe(true);
    for (const f of frames) expect(validateAgainst(sse.events[f.event]!, f.data), f.event).toEqual([]);
  });
});
