// Overview's Needs-you row for message requests (Layer 5, U2) and the Messages banner's Layer 5 goals, through the real store.
import { createElement as h, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import { createMockAdapter, type MockAdapter, type ScenarioName } from "../src/lib/api/mock";
import { requestSeq } from "../src/lib/api/requests";
import { apiStore } from "../src/lib/api/store";
import { buildPrompt } from "../src/lib/prompts";
import { ModeProvider, resetModeForTests } from "../src/lib/mode";
import { MessagingBanner, messagingGoal } from "../src/views/messages/banner";
import { bannerAgent, messagingBanner } from "../src/views/messages/model";
import OverviewView from "../src/views/overview";
import { FakeClock, flush } from "./helpers/fakes";
import { routed } from "./helpers/routed";

const made: MockAdapter[] = [];
afterEach(() => {
  apiStore.detach();
  apiStore.reset();
  for (const a of made.splice(0)) a.mock.dispose();
});

async function load(scenario: ScenarioName): Promise<MockAdapter> {
  const a = createMockAdapter({ scenario, clock: new FakeClock(Date.parse("2026-10-14T09:00:00Z")), tokenDelayMs: 0 });
  made.push(a);
  apiStore.attach(a);
  await flush(30);
  return a;
}

const html = (el: ReactElement): string => {
  resetModeForTests();
  try {
    return renderToStaticMarkup(h(ModeProvider, null, el));
  } finally {
    resetModeForTests();
  }
};

const overview = () => html(routed("/overview", "/overview", OverviewView));

describe("Overview: message requests", () => {
  it("lists '2 message requests' in Needs you, linking to Messages, with no sender text", async () => {
    await load("requests");
    const out = overview();
    expect(out).toContain("2 message requests");
    expect(out).toContain('href="#/messages"');
    expect(out).toContain("Someone you have not added wrote to you. Nothing is sent back until you accept.");
    expect(out).not.toContain("Free coin");
    expect(out).not.toContain("<script>");
    expect(out).not.toContain("Nothing needs you right now.");
  });

  it("an ignored request is not counted (the header badge and this row agree)", async () => {
    await load("requests");
    const threads = apiStore.getState().threads!.filter((t) => t.request);
    apiStore.ignoreRequest(threads[0]!.peer, requestSeq(threads[0]!));
    expect(overview()).toContain("1 message request");
    for (const t of threads.slice(1)) apiStore.ignoreRequest(t.peer, requestSeq(t));
    const none = overview();
    expect(none).not.toMatch(/\d+ message requests?</);
    expect(none).not.toContain("Someone you have not added wrote to you");
  });

  it("a node with no requests shows no such row", async () => {
    await load("healthy");
    const out = overview();
    expect(out).not.toMatch(/\d+ message requests?/);
  });
});

describe("the Messages banner asks with the Layer 5 goals", () => {
  it("maps the banner's fix and set-up to messaging.fix and messaging.setup", () => {
    expect(messagingGoal("fix")).toBe("messaging.fix");
    expect(messagingGoal("setup")).toBe("messaging.setup");
    const unset = messagingBanner({ state: "unset", reason: "No hub.", reason_code: "not_enrolled" }, null)!;
    expect(messagingGoal(bannerAgent(unset, "not_enrolled")!.goal)).toBe("messaging.setup");
    expect(messagingGoal(bannerAgent(unset, "hub_waiting")!.goal)).toBe("messaging.setup");
    expect(messagingGoal(bannerAgent(unset, "hub_unreachable")!.goal)).toBe("messaging.fix");
    const full = messagingBanner({ state: "error", reason: "full", reason_code: "store_full" }, null)!;
    expect(messagingGoal(bannerAgent(full, "store_full")!.goal)).toBe("messaging.fix");
  });

  it("the prepared message is the goal sentence over the fabric service's facts, not the banner's old question", async () => {
    await load("store-full");
    const service = apiStore.getState().services!.find((s) => s.id === "fabric")!;
    const text = buildPrompt("messaging.fix", { kind: "service", service }, { status: apiStore.getState().status });
    expect(text.startsWith("Fix messaging on this node.")).toBe(true);
    expect(text).toContain('"reason_code":"store_full"');
    expect(text).not.toContain("Messaging on this node has a problem. Check what is wrong");
  });

  it("the banner still renders its sentence and now links to the messaging settings", async () => {
    await load("messaging-new");
    const out = html(h(MessagingBanner));
    expect(out).toContain("messaging not set up");
    expect(out).toContain("Set up messaging with the agent");
    expect(out).toContain('href="#/settings/l5"');
    expect(out).toContain("Messaging settings");
  });
});
