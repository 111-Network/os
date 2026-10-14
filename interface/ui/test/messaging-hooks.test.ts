// The Layer 5 hooks render from the store (react-dom/server, no DOM), before and after the snapshots arrive.
import { createElement as h, type ReactElement } from "react";
import { renderToString } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import { useAttention, useContact, useContacts, useFabric, useNearby, useOwnCard, usePanels, useThread, useThreads } from "../src/lib/api/hooks";
import { createMockAdapter, type MockAdapter } from "../src/lib/api/mock";
import { ADA } from "../src/lib/api/mock/messaging";
import { apiStore } from "../src/lib/api/store";
import { FakeClock, flush } from "./helpers/fakes";

function Probe(): ReactElement {
  const threads = useThreads();
  const contacts = useContacts();
  const ada = useContact(ADA);
  const nobody = useContact("00".repeat(32));
  const fabric = useFabric();
  const nearby = useNearby();
  const panels = usePanels();
  const slot = useThread(ADA);
  const card = useOwnCard();
  const attention = useAttention();
  return h(
    "pre",
    null,
    JSON.stringify({
      threads: threads?.length ?? null,
      contacts: contacts?.length ?? null,
      ada: ada?.alias ?? null,
      nobody: nobody?.alias ?? null,
      hub: fabric ? fabric.hub.enrolled : null,
      nearby: nearby?.length ?? null,
      panels: panels?.length ?? null,
      slot: slot.state,
      page: slot.page?.messages.length ?? null,
      card: card?.qr.size ?? null,
      requests: attention.openRequests,
    }),
  );
}

const render = () => JSON.parse(renderToString(h(Probe)).replace(/<[^>]+>/g, "").replace(/&quot;/g, '"')) as Record<string, unknown>;

const made: MockAdapter[] = [];
afterEach(() => {
  apiStore.detach();
  apiStore.reset();
  for (const a of made.splice(0)) a.mock.dispose();
});

describe("Layer 5 hooks", () => {
  it("render empty before any snapshot", () => {
    expect(render()).toEqual({ threads: null, contacts: null, ada: null, nobody: null, hub: null, nearby: null, panels: null, slot: "loading", page: null, card: null, requests: 0 });
  });

  it("render the loaded slices, a conversation once opened, and the card once asked for", async () => {
    const a = createMockAdapter({ scenario: "requests", clock: new FakeClock(Date.parse("2026-10-14T09:00:00Z")), tokenDelayMs: 0 });
    made.push(a);
    apiStore.attach(a);
    await flush(30);
    await apiStore.loadThread(ADA);
    await apiStore.loadOwnCard();
    const r = render();
    expect(r).toMatchObject({ threads: 6, contacts: 7, ada: "Ada (workshop)", nobody: null, hub: true, nearby: 0, panels: 1, slot: "ready", page: 6, card: expect.any(Number), requests: 2 });
  });
});
