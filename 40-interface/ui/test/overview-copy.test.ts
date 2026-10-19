// Overview copy after Layer 5 (as built): the Nearby card's advanced note points at the Nearby page instead of announcing a table
// to come, the services grid carries the Messaging (fabric) service as the node reports it, and no scenario shows pre-Layer-5 phrasing.
import { createElement as h, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import { createMockAdapter, SCENARIO_NAMES, type MockAdapter, type ScenarioName } from "../src/lib/api/mock";
import { apiStore } from "../src/lib/api/store";
import { ModeProvider, resetModeForTests } from "../src/lib/mode";
import OverviewView from "../src/views/overview";
import { FakeClock, flush } from "./helpers/fakes";
import { routed } from "./helpers/routed";

const made: MockAdapter[] = [];
afterEach(() => {
  apiStore.detach();
  apiStore.reset();
  for (const a of made.splice(0)) a.mock.dispose();
});

async function load(scenario: ScenarioName): Promise<void> {
  const a = createMockAdapter({ scenario, clock: new FakeClock(Date.parse("2026-10-14T09:00:00Z")), tokenDelayMs: 0 });
  made.push(a);
  apiStore.attach(a);
  await flush(30);
}

/** The page as the server renders it; `?mode=advanced` is read once from a stand-in `window` (the mode is a URL parameter). */
const html = (el: ReactElement, search = ""): string => {
  const g = globalThis as { window?: unknown };
  const had = g.window;
  g.window = {
    location: { search, hash: "", pathname: "/", protocol: "http:", hostname: "127.0.0.1", port: "11180" },
    history: { state: null, replaceState: () => undefined },
    localStorage: { getItem: () => null, setItem: () => undefined },
  };
  resetModeForTests();
  try {
    return renderToStaticMarkup(h(ModeProvider, null, el));
  } finally {
    if (had === undefined) delete g.window;
    else g.window = had;
    resetModeForTests();
  }
};

const overview = (search = "") => html(routed("/overview", "/overview", OverviewView), search);

/** What the page said before Layer 5 landed; none of it may come back. */
const STALE = [/nearby-nodes table/i, /relay and presence layers/i, /coming with layer 5/i, /messaging arrives with/i, /structural stub/i];

describe("Overview: the Nearby card after Layer 5", () => {
  it("advanced mode points at the Nearby page and names radar and map as Layer 7's", async () => {
    await load("healthy");
    const out = overview("?mode=advanced");
    expect(out).toContain("This card only says whether anyone has been heard.");
    expect(out).toContain("The Meshtastic nodes a board has heard are listed on the Nearby page");
    expect(out).toContain("radar and map views arrive with Layer 7 (Relay &amp; presence)");
    expect(out).toContain('href="#/nearby"');
    expect(out).not.toContain("The nearby-nodes table arrives");
  });

  it("simple mode keeps the one sentence and the link, without the advanced note", async () => {
    await load("healthy");
    const out = overview();
    expect(out).toContain("At least one nearby node has been heard, so messages can flow.");
    expect(out).toContain("See who is nearby");
    expect(out).not.toContain("This card only says whether anyone has been heard.");
  });
});

describe("Overview: the Messaging (fabric) service", () => {
  it("is a card in the services grid, with the sentence the node gave", async () => {
    await load("healthy");
    const out = overview();
    expect(out).toContain('href="#/services/fabric"');
    expect(out).toContain("Messaging");
    expect(out).toContain("Your home hub is reachable; messages are delivered or held for you.");
  });

  it("a store that is full is named in Needs you, in the verdict and on its card", async () => {
    await load("store-full");
    const out = overview();
    expect(out).toContain("the message store has a problem");
    expect(out).toContain("The message store is full; new messages cannot be saved until space is freed.");
    expect(out).toContain('href="#/services/fabric"');
    expect(out).not.toContain("Nothing needs you right now.");
  });

  it("a node that is not enrolled says so on its card, in the words of the node", async () => {
    await load("messaging-new");
    expect(overview()).toContain("No home hub and no other node reachable yet. Set up with the agent to enroll with a hub.");
  });
});

describe("Overview: no pre-Layer-5 phrasing in any scenario", () => {
  it.each([...SCENARIO_NAMES])("%s, simple and advanced", async (scenario) => {
    await load(scenario);
    for (const out of [overview(), overview("?mode=advanced")]) {
      for (const pattern of STALE) expect(out).not.toMatch(pattern);
    }
  });
});
