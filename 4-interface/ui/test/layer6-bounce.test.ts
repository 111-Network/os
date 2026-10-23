// The C2 review bounce (os/6-transport/TASKS.md, C2 "Review"): each finding is closed by a test named after it, and each of the two majors
// (F1, F2) fails without its fix. Fixtures are the mock's `bridge` scenario through the real store, as layer6-api.test.ts does.
import { createElement as h, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { confirmGate, consentOf } from "../src/lib/api/bridge";
import { createHttpAdapter } from "../src/lib/api/http";
import { createMockAdapter, type MockAdapter } from "../src/lib/api/mock";
import { ApiError } from "../src/lib/api/problem";
import { isForeignCancellable, isForeignRetryable, isHandoffPending, mergeForeignPage, upsertForeign } from "../src/lib/api/networks";
import { apiStore } from "../src/lib/api/store";
import type { ConsentRequired, ForeignConversation, ForeignMessage, Network, PlanRefusal } from "../src/lib/api/types";
import { foreignUnread } from "../src/lib/attention";
import { ModeProvider, resetModeForTests } from "../src/lib/mode";
import ThreadList from "../src/views/messages/list";
import { errorSentence } from "../src/views/messages/model";
import { NetworkBanner } from "../src/views/messages/net/banner";
import { ConsentPanel } from "../src/views/messages/net/consent";
import { PlanLines } from "../src/views/messages/net/fanout";
import { FakeClock, flush } from "./helpers/fakes";

const T0 = Date.parse("2026-10-19T09:00:00Z");
const made: MockAdapter[] = [];

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(T0);
});
afterEach(() => {
  vi.useRealTimers();
  apiStore.detach();
  apiStore.reset();
  for (const a of made.splice(0)) a.mock.dispose();
});

async function loaded() {
  const clock = new FakeClock(T0);
  const a = createMockAdapter({ scenario: "bridge", clock, tokenDelayMs: 0 });
  made.push(a);
  await apiStore.attach(a);
  await flush(30);
  return { a, clock };
}

const html = (el: ReactElement): string => {
  resetModeForTests();
  try {
    return renderToStaticMarkup(h(ModeProvider, null, el));
  } finally {
    resetModeForTests();
  }
};

const held = (conv: string, id: string): ForeignMessage | undefined => apiStore.getState().foreignPages[conv]?.page?.messages.find((m) => m.id === id);
const dm = (): ForeignConversation => apiStore.getState().conversations.meshtastic!.find((c) => c.id === "fc-3")!;

const row = (over: Partial<ForeignMessage> = {}): ForeignMessage => ({
  id: "fm-1",
  conv: "fc-1",
  dir: "own",
  sender: { native: "!a1b2c3d4", name: "" },
  auth: "none",
  text: "x",
  state: "queued",
  evidence: null,
  reason_code: null,
  reason: null,
  group_id: null,
  part: null,
  relayed_from: null,
  via_internet: false,
  seq: 1,
  at: "2026-10-19T09:00:00Z",
  claim_ts: null,
  truncated: null,
  ...over,
});

describe("F1: the page never hides what the node says about a row", () => {
  it("F1: a queued row the node refuses before air reads failed with its reason, on the tail read and on a full re-read", async () => {
    const { a, clock } = await loaded();
    await apiStore.loadForeign("meshtastic", "fc-1");
    // LongFast is sent through a licensed board: the node asks the person once, and the round trip echoes its hash.
    const gate = consentOf(await a.networks.send("meshtastic", "fc-1", { text: "to a channel that is gone" }).then(() => null, (e: unknown) => e));
    if (gate) await confirmGate(a, gate, gate.hash);
    const sent = await a.networks.send("meshtastic", "fc-1", { text: "to a channel that is gone" });
    await flush(30);
    expect(held("fc-1", sent.id)!.state).toBe("queued");

    a.mock.bridgeRefuse(sent.id, "no_channel"); // the node's answer: refused before air
    await flush(30);
    const refused = held("fc-1", sent.id)!;
    expect(refused.state).toBe("failed");
    expect(refused.reason_code).toBe("no_channel");
    expect(refused.reason).toBeTruthy();

    await apiStore.loadForeign("meshtastic", "fc-1"); // a full re-read keeps what the node said
    expect(held("fc-1", sent.id)).toMatchObject({ state: "failed", reason_code: "no_channel" });
    await clock.advanceAsync(3_000, 100); // the delivery chain does not carry a refused row on
    await flush(30);
    expect(held("fc-1", sent.id)!.state).toBe("failed");
  });

  it("F1: a retry's re-queue is shown (failed -> queued -> on), and delivered is the only state a stale snapshot cannot undo", async () => {
    const { a, clock } = await loaded();
    await apiStore.loadForeign("meshtastic", "fc-1");
    const refused = [...apiStore.getState().foreignPages["fc-1"]!.page!.messages].find((m) => m.state === "failed")!;
    expect(refused.reason_code).toBe("no_channel");
    await a.messages.retry(refused.id);
    await apiStore.loadForeign("meshtastic", "fc-1");
    expect(["queued", "sent"]).toContain(held("fc-1", refused.id)!.state);
    expect(held("fc-1", refused.id)!.reason_code).toBeNull();
    await clock.advanceAsync(3_000, 100);
    await flush(30);
    expect(held("fc-1", refused.id)!.state).toBe("relayed");

    // the pure rules: queued -> failed and failed -> queued are taken; delivered stays
    expect(upsertForeign([row({ state: "queued" })], row({ state: "failed", reason_code: "no_channel" }))[0]!.state).toBe("failed");
    expect(upsertForeign([row({ state: "failed" })], row({ state: "queued" }))[0]!.state).toBe("queued");
    expect(upsertForeign([row({ state: "delivered" })], row({ state: "sent" }))[0]!.state).toBe("delivered");
    const conv = apiStore.getState().foreignPages["fc-1"]!.page!.conversation;
    const cur = { conversation: conv, messages: [row({ state: "queued" })], more: false };
    expect(mergeForeignPage(cur, { conversation: conv, messages: [row({ state: "failed", reason_code: "no_channel" })], more: false }).messages[0]!.state).toBe("failed");
    expect(mergeForeignPage({ ...cur, messages: [row({ state: "delivered" })] }, { conversation: conv, messages: [row({ state: "relayed" })], more: false }).messages[0]!.state).toBe("delivered");
  });
});

describe("F2: an open DM does not count up", () => {
  it("F2: after a text lands on the DM a view holds open, its unread is 0 in the list, the rail and the badge; closed, it counts", async () => {
    const { a } = await loaded();
    await apiStore.refreshConversations("meshtastic");
    expect(dm().unread).toBe(1);

    const release = apiStore.openForeign("fc-3"); // the thread view is mounted
    await apiStore.loadForeign("meshtastic", "fc-3"); // a person's read of the window: the node clears unread
    await apiStore.refreshConversations("meshtastic");
    expect(dm().unread).toBe(0);

    a.mock.bridgeReceive("fc-3", { native: "!0f3e9a12", name: "Mara" }, "ping while reading");
    await flush(60);
    expect(apiStore.getState().foreignPages["fc-3"]!.page!.messages.slice(-1)[0]!.text).toBe("ping while reading");
    expect(dm().unread).toBe(0);
    const mesh = apiStore.getState().conversations.meshtastic!;
    expect(foreignUnread(mesh)).toBe(1); // only the pinned Ridge crew channel
    const rail = html(h(ThreadList));
    const entry = rail.slice(rail.indexOf('data-testid="rail-meshtastic"'));
    expect(entry.slice(0, entry.indexOf("</a>"))).toContain("1 unread");
    expect(entry.slice(0, entry.indexOf("</a>"))).not.toContain("2 unread");

    release(); // the view is gone: a text now is unread
    a.mock.bridgeReceive("fc-3", { native: "!0f3e9a12", name: "Mara" }, "ping after leaving");
    await flush(60);
    expect(dm().unread).toBe(1);
  });

  it("F2: a state frame replacing a waiting new frame still owes the read; two views of one conversation do not release each other", async () => {
    const { a } = await loaded();
    const r1 = apiStore.openForeign("fc-3");
    const r2 = apiStore.openForeign("fc-3");
    await apiStore.loadForeign("meshtastic", "fc-3");
    await apiStore.refreshConversations("meshtastic");
    const spy = vi.spyOn(a.networks, "conversation");
    const top = Math.max(...apiStore.getState().foreignPages["fc-3"]!.page!.messages.map((m) => m.seq));
    const frame = (change: "new" | "state", id?: string) =>
      apiStore.applyFrame({ event: "network_message", data: { network: "meshtastic", conv: "fc-3", change, seq: top + 1, ...(id ? { id } : {}) } });
    frame("state", "fm-14"); // starts a pass; the two frames below land while it is in flight and wait for the next one
    frame("new");
    frame("state", "fm-14"); // replaces the waiting new frame, and must not lose what it owed
    await flush(60);
    expect(spy.mock.calls.length).toBeGreaterThanOrEqual(2);
    expect(spy.mock.calls[spy.mock.calls.length - 1]![2]?.sinceSeq).toBeUndefined(); // the last pass read the window, not a since_seq tail
    r1();
    spy.mockClear();
    frame("new");
    await flush(60);
    expect(spy.mock.calls.some((c) => c[2]?.sinceSeq === undefined)).toBe(true); // r2 still holds it open
    r2();
    spy.mockClear();
    frame("new");
    await flush(60);
    expect(spy.mock.calls.every((c) => c[2]?.sinceSeq !== undefined)).toBe(true); // nobody holds it: a tail
  });
});

describe("F3: foreign strings are cleaned and direction-isolated", () => {
  const BIDI = "‮⁦";
  const consent: ConsentRequired = { action: "edge_enable", subject: "fc-1>fc-4", warning_text: `Copies ${BIDI}gnp.exe into Friends.`, hash: "e".repeat(64), addons: [] };

  it("F3: the consent warning text drops overrides and isolates its direction; the hash is the node's, untouched", () => {
    const out = html(h(ConsentPanel, { consent, onConfirm: () => undefined, onCancel: () => undefined }));
    expect(out).not.toMatch(/[‪-‮⁦-⁩]/);
    expect(out).toContain("Copies gnp.exe into Friends.");
    const pre = out.slice(out.indexOf('<pre id="consent-text"'));
    expect(pre.slice(0, pre.indexOf(">"))).toContain('dir="auto"');
    expect(pre.slice(0, pre.indexOf(">"))).toContain("unicode-bidi:isolate");
    expect(consent.hash).toBe("e".repeat(64));
  });

  it("F3: the network account is text, cleaned, in an isolated span", async () => {
    const { a } = await loaded();
    const net = (await a.networks.list())[0]! as Network;
    const out = html(h(NetworkBanner, { network: { ...net, account: `Board ${BIDI}evil !a1b2` } }));
    expect(out).not.toMatch(/[‪-‮⁦-⁩]/);
    const span = out.slice(out.indexOf('data-testid="network-account"') - 80);
    expect(span.slice(0, span.indexOf("</span>"))).toContain("Board evil !a1b2");
    expect(span.slice(0, span.indexOf("</span>"))).toContain('dir="auto"');
  });

  it("F3: a preview's warnings and refusal detail are cleaned and isolated", () => {
    const plan = {
      bytes: 300,
      cap: 200,
      parts: 1,
      size_mode: "refuse",
      airtime_estimate: null,
      refusal: { code: "oversize", detail: `Over by ${BIDI}99 bytes`, consent: null } as PlanRefusal,
      warnings: [{ code: "x", text: `Careful ${BIDI}now` }],
    } as never;
    const out = html(h(PlanLines, { plan, onSplit: () => undefined, onConfirm: () => undefined, splittable: false }));
    expect(out).not.toMatch(/[‪-‮⁦-⁩]/);
    expect(out).toContain("Refused: Over by 99 bytes");
    expect(out).toContain("Careful now");
    expect(out).toMatch(/dir="auto"[^>]*unicode-bidi:isolate[^>]*data-testid="refusal-detail"/);
  });

  it("F3: a Problem detail that quotes a title is cleaned before it is a sentence on the screen", () => {
    const err = new ApiError({ type: "about:blank", title: "Refused", status: 409, code: "bad_request", detail: `No DM for ${BIDI}Mara‮ Ridge` });
    expect(errorSentence(err, "fallback")).toBe("No DM for Mara Ridge");
    expect(errorSentence(new ApiError({ type: "about:blank", title: `T${BIDI}itle`, status: 400, code: "bad_request" }), "fallback")).toBe("Title");
    expect(errorSentence(new Error("x"), "fallback")).toBe("fallback");
  });
});

describe("F4: cancel is a call, never only an announcement", () => {
  it("F4: both adapters carry messages.cancel, and a foreign cancel reaches the node", async () => {
    const { a } = await loaded();
    expect(typeof a.messages.cancel).toBe("function");
    const http = createHttpAdapter({ fetch: vi.fn() as never });
    expect(typeof http.messages.cancel).toBe("function");
    await apiStore.loadForeign("meshtastic", "fc-1");
    const queued = apiStore.getState().foreignPages["fc-1"]!.page!.messages.find((m) => m.reason_code === "rate_limited")!;
    await a.messages.cancel(queued.id);
    await flush(60);
    expect(held("fc-1", queued.id)).toBeUndefined();
  });
});

describe("F6: retry is offered where the node accepts it", () => {
  it("F6: uncertain keys on the contract's field or the display code; retry only on failed and uncertain rows, never on sent or queued", () => {
    expect(isHandoffPending(row({ uncertain: true, reason_code: null }))).toBe(true);
    expect(isHandoffPending(row({ uncertain: true, reason_code: "rate_limited" }))).toBe(true); // another reason is kept; the question still stands
    expect(isHandoffPending(row({ reason_code: "handoff_pending" }))).toBe(true); // an older daemon
    expect(isHandoffPending(row({ state: "queued" }))).toBe(false);
    expect(isHandoffPending(row({ dir: "in", uncertain: true }))).toBe(false);

    expect(isForeignRetryable(row({ state: "failed", reason_code: "no_channel" }))).toBe(true);
    expect(isForeignRetryable(row({ state: "queued", uncertain: true }))).toBe(true);
    expect(isForeignRetryable(row({ state: "sent" }))).toBe(false);
    expect(isForeignRetryable(row({ state: "sent", uncertain: true }))).toBe(true); // the node's own mark wins over the state it shows
    expect(isForeignRetryable(row({ state: "queued", reason_code: "rate_limited" }))).toBe(false);
    expect(isForeignRetryable(row({ state: "relayed" }))).toBe(false);
    expect(isForeignRetryable(row({ state: "failed", reason_code: "cancelled" }))).toBe(false);
    expect(isForeignRetryable(row({ dir: "in", state: "delivered" }))).toBe(false);

    expect(isForeignCancellable(row({ state: "queued" }))).toBe(true);
    expect(isForeignCancellable(row({ state: "queued", uncertain: true }))).toBe(false); // it may have left
  });

  it("F6: the node's 409 on a retry it will not take is shown and changes nothing locally", async () => {
    const { a } = await loaded();
    await apiStore.loadForeign("meshtastic", "fc-1");
    const sentRow = apiStore.getState().foreignPages["fc-1"]!.page!.messages.find((m) => m.state === "sent" && m.dir === "own")!;
    const before = JSON.stringify(sentRow);
    const err = await a.messages.retry(sentRow.id).then(
      () => null,
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).status).toBe(409);
    expect(JSON.stringify(held("fc-1", sentRow.id))).toBe(before);
  });
});

describe("F7: frames refresh what a view holds", () => {
  it("F7: a conversation frame reads the network's list only where one is held, and /networks only where none is", async () => {
    const { a } = await loaded();
    expect(apiStore.getState().conversations.meshtastic).toBeUndefined();
    const lists = vi.spyOn(a.networks, "conversations");
    const nets = vi.spyOn(a.networks, "list");

    apiStore.applyFrame({ event: "network_message", data: { network: "meshtastic", conv: "fc-9", change: "conversation", seq: 0 } });
    await flush(30);
    expect(lists).not.toHaveBeenCalled(); // nobody holds a list
    expect(nets).toHaveBeenCalledTimes(1); // so the counts come from /networks

    await apiStore.refreshConversations("meshtastic");
    lists.mockClear();
    nets.mockClear();
    apiStore.applyFrame({ event: "network_message", data: { network: "meshtastic", conv: "fc-9", change: "conversation", seq: 0 } });
    await flush(30);
    expect(lists).toHaveBeenCalledTimes(1);
    expect(nets).not.toHaveBeenCalled(); // the list says what the rail needs

    lists.mockClear();
    apiStore.applyFrame({ event: "network_message", data: { network: "lxmf", conv: "fc-4", change: "state", id: "fm-16", seq: 1 } });
    await flush(30);
    expect(lists).not.toHaveBeenCalled(); // no view holds the Reticulum list
  });
});
