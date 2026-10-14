// The Messages and Contacts views, rendered to static markup from the mock scenarios through the real store. This is the
// state matrix the person sees: words, glyph labels, reasons, the request section, the density flags, and that
// peer-authored text is escaped.
import { createElement as h, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import { createMockAdapter, type MockAdapter, type ScenarioName } from "../src/lib/api/mock";
import { ADA, BRAM, CLEO, DANA, DOV, ELI } from "../src/lib/api/mock/messaging";
import { apiStore } from "../src/lib/api/store";
import { ModeProvider, resetModeForTests } from "../src/lib/mode";
import ContactsView from "../src/views/contacts";
import { OwnCard } from "../src/views/contacts/own-card";
import { AddContact } from "../src/views/contacts/add";
import ThreadList from "../src/views/messages/list";
import ThreadView from "../src/views/messages/thread";
import { MessagingBanner } from "../src/views/messages/banner";
import { REASON_COPY, STATE_COPY } from "../src/views/messages/model";
import { FakeClock, flush } from "./helpers/fakes";

const made: MockAdapter[] = [];
afterEach(() => {
  apiStore.detach();
  apiStore.reset();
  for (const a of made.splice(0)) a.mock.dispose();
});

async function load(scenario: ScenarioName) {
  const a = createMockAdapter({ scenario, clock: new FakeClock(Date.parse("2026-10-14T09:00:00Z")), tokenDelayMs: 0 });
  made.push(a);
  apiStore.attach(a);
  await flush(30);
  return a;
}

/** Render with the density mode the page would resolve (`?mode=advanced` is what the toggle persists and the URL can carry). */
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

describe("Messages list", () => {
  it("shows each conversation with its name, last line and the glyph of its last own message", async () => {
    await load("healthy");
    const out = html(h(ThreadList));
    expect(out).toContain("Ada (workshop)");
    expect(out).toContain("you: On my way back now.");
    expect(out).toContain(">sent<"); // Ada's last own message is `sent`
    expect(out).toContain("Dana");
    expect(out).toContain("you: Do you still have the photos");
    expect(out).toContain("Bram");
    // No requests in the healthy scenario, so no request section.
    expect(out).not.toContain("message requests");
    // Rows link to the thread by NodeId.
    expect(out).toContain(`href="#/messages/${ADA}"`);
    expect(out).toContain('href="#/messages/contacts"');
  });

  it("puts requests first, with accept and ignore, the held words as escaped text, and no sender-supplied name", async () => {
    await load("requests");
    const out = html(h(ThreadList));
    expect(out).toContain("message requests (2)");
    expect(out.indexOf("message requests")).toBeLessThan(out.indexOf("conversations"));
    expect(out).toContain("(if this is the wrong node, ignore me)");
    // The hostile request: markup arrives as text.
    expect(out).not.toContain("<script>");
    expect(out).toContain("&lt;script&gt;alert(&#x27;hi&#x27;)&lt;/script&gt;");
    expect(out).toMatch(/>accept</);
    expect(out).toMatch(/>ignore</);
    expect(out).toContain("nothing is sent back until you accept");
    // The label made from the id, never something the sender chose.
    expect(out).toMatch(/[a-z0-9]{4}-[a-z0-9]{4}-[a-z0-9]{2}/);
  });

  it("is honest when empty: it says so and offers to add a contact", async () => {
    await load("messaging-new");
    const out = html(h(ThreadList));
    expect(out).toContain("No conversations yet.");
    expect(out).toContain("Add a contact");
    expect(out).toContain("messaging not set up");
    expect(out).toContain("No home hub and no other node reachable yet");
  });

  it("says a full store blocks new messages and points at the agent", async () => {
    await load("store-full");
    const out = html(h(ThreadList));
    expect(out).toContain("messaging problem");
    expect(out).toMatch(/store is full/i);
    expect(out).toContain("Ask the agent to fix it");
  });

  it("says nothing extra while messaging is live", async () => {
    await load("healthy");
    expect(html(h(MessagingBanner))).toBe("");
  });

  it("says loading before the first snapshot, never an invented inbox", () => {
    const out = html(h(ThreadList));
    expect(out).toContain("Loading messages");
    expect(out).not.toContain("Ada");
  });

  it("Advanced adds the label and id; Simple does not", async () => {
    await load("healthy");
    expect(html(h(ThreadList))).not.toContain("waiting:");
    const adv = html(h(ThreadList), "advanced");
    expect(adv).toContain("waiting:");
    expect(adv).toContain(ADA.slice(0, 8));
  });
});

describe("a conversation", () => {
  it("renders every state with its label and the section 9 copy behind the disclosure, and the reasons inline", async () => {
    await load("healthy");
    await apiStore.loadThread(ADA);
    const ada = html(h(ThreadView, { peer: ADA }));
    for (const s of ["delivered", "relayed", "sent"]) expect(ada).toContain(`>${s}<`);
    expect(ada).toContain(STATE_COPY.delivered);
    expect(ada).toContain(STATE_COPY.relayed);
    expect(ada).toContain("received");
    expect(ada).toContain("written");
    // The composer is there, with the attachments note.
    expect(ada).toContain("attachments arrive next");
    expect(ada).toContain('aria-label="Message"');

    await apiStore.loadThread(BRAM);
    expect(html(h(ThreadView, { peer: BRAM }))).toContain(REASON_COPY.hub_not_yours);
    await apiStore.loadThread(CLEO);
    expect(html(h(ThreadView, { peer: CLEO }))).toContain(REASON_COPY.waiting_for_key);
    await apiStore.loadThread(DANA);
    const dana = html(h(ThreadView, { peer: DANA }));
    expect(dana).toContain(REASON_COPY.mailbox_full);
    expect(dana).toContain(">sent<");
  });

  it("keeps the sender's claimed time inside the details, marked unverified; the receipt time is the label", async () => {
    await load("healthy");
    await apiStore.loadThread(ADA);
    const out = html(h(ThreadView, { peer: ADA }));
    expect(out).toContain("unverified sender time");
    expect(out).toContain("cannot be checked");
    // The claim only appears inside a <details> element, never in a bubble or summary.
    const claim = out.indexOf("unverified sender time");
    expect(out.lastIndexOf("<details", claim)).toBeGreaterThan(out.lastIndexOf("</details>", claim));
  });

  it("offers retry only on queued and sent own messages", async () => {
    await load("healthy");
    await apiStore.loadThread(ADA);
    const out = html(h(ThreadView, { peer: ADA }));
    // Ada's thread: 2 delivered, 1 relayed, 1 sent -> exactly one retry (the sent one); relayed and delivered have none.
    expect(out.match(/retry now/g)?.length).toBe(1);
  });

  it("Advanced appends the raw msg_id, epoch and n, the evidence, and the raw thread", async () => {
    await load("healthy");
    await apiStore.loadThread(ADA);
    expect(html(h(ThreadView, { peer: ADA }))).not.toContain("msg_id stream");
    const adv = html(h(ThreadView, { peer: ADA }), "advanced");
    expect(adv).toMatch(/msg_id stream 0 epoch 7340 n \d+/);
    expect(adv).toContain("evidence: ACK from the recipient");
    expect(adv).toContain("CUSTODY_ACK ACCEPTED");
    expect(adv).toContain("raw thread");
  });

  it("a pending request is read-only: no composer, the reason is given, accept and ignore are offered", async () => {
    await load("requests");
    await apiStore.loadThread(DOV);
    const out = html(h(ThreadView, { peer: DOV }));
    expect(out).not.toContain('aria-label="Message"');
    expect(out).toContain("Accept it to reply");
    expect(out).toMatch(/>accept</);
    expect(out).toContain("Is this the workshop node?");
  });

  it("escapes hostile text in a conversation", async () => {
    await load("requests");
    await apiStore.loadThread(ELI);
    const out = html(h(ThreadView, { peer: ELI }));
    expect(out).not.toContain("<script>");
    expect(out).toContain("&lt;script&gt;");
  });

  it("a stranger's id says there is no such conversation instead of showing a composer", async () => {
    await load("healthy");
    await apiStore.loadThread("ab".repeat(32));
    const out = html(h(ThreadView, { peer: "ab".repeat(32) }));
    expect(out).toContain("No conversation with this id");
    expect(out).not.toContain('aria-label="Message"');
  });

  it("an empty conversation with a contact still gets a composer and says what happens to what you write", async () => {
    await load("healthy");
    // HUB is a contact with no messages.
    const hub = "8b46d2f1a09e537c4b81e60d9a3f75c2e1084d6b93a7f50c2e8d14b6a09f3c75";
    await apiStore.loadThread(hub);
    const out = html(h(ThreadView, { peer: hub }));
    expect(out).toContain("Nothing here yet.");
    expect(out).toContain('aria-label="Message"');
  });
});

describe("Contacts", () => {
  it("lists contacts with their facts, pinned first, with the four actions", async () => {
    await load("healthy");
    const out = html(h(ContactsView));
    expect(out).toContain("Ada (workshop)");
    expect(out.indexOf("Ada (workshop)")).toBeLessThan(out.indexOf("Dana")); // pinned first
    expect(out).toContain("enrolled with a hub");
    expect(out).toContain("no card yet"); // Cleo has no key
    for (const a of ["message", "rename", "unpin", "forget"]) expect(out).toContain(`>${a}<`);
    expect(out).toContain("Add a contact".toLowerCase());
    expect(out).toContain("your card");
  });

  it("shows requests above the list, and a node-id-less honest empty state", async () => {
    await load("requests");
    const out = html(h(ContactsView));
    expect(out).toContain("message requests (2)");
    await load("messaging-new");
    expect(html(h(ContactsView))).toContain("No contacts yet.");
  });

  it("add: explains both ways in, offers a picture, and hides the camera on an insecure origin", () => {
    const out = html(h(AddContact));
    expect(out).toContain("Card text or node1… id");
    expect(out).toContain("choose a QR picture");
    expect(out).toContain("Add contact");
    // No window in this renderer, so the origin is not trustworthy: no camera button, and it says why.
    expect(out).not.toContain("scan with the camera");
    expect(out).toContain("only works on a secure page");
  });

  it("own card: a canvas for the QR, the text with a copy button, and the node id", async () => {
    await load("healthy");
    await apiStore.loadOwnCard();
    const out = html(h(OwnCard));
    expect(out).toContain('<canvas');
    expect(out).toContain('aria-label="Your contact card as a QR code"');
    expect(out).toContain("Copy your card text");
    expect(out).toContain("Copy your node id");
    expect(out).toContain("holds no secret");
  });

  it("own card shows loading, not a blank, before the card arrives", () => {
    expect(html(h(OwnCard))).toContain("Loading your card");
  });
});
