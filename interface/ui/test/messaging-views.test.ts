// The Messages and Contacts views, rendered to static markup from the mock scenarios through the real store. This is the
// state matrix the person sees: words, glyph labels, reasons, the request section, the density flags, and that
// peer-authored text is escaped.
import { createElement as h, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import { createMockAdapter, type MockAdapter, type ScenarioName } from "../src/lib/api/mock";
import { buildFixtures } from "../src/lib/api/mock/fixtures";
import { ADA, BRAM, CLEO, DANA, DOV, ELI, makeContact } from "../src/lib/api/mock/messaging";
import { requestSeq } from "../src/lib/api/requests";
import { apiStore } from "../src/lib/api/store";
import { Header } from "../src/app/Header";
import { ModeProvider, resetModeForTests } from "../src/lib/mode";
import ContactsView from "../src/views/contacts";
import { OwnCard } from "../src/views/contacts/own-card";
import { AddContact } from "../src/views/contacts/add";
import ThreadList from "../src/views/messages/list";
import ThreadView from "../src/views/messages/thread";
import { MessagingBanner } from "../src/views/messages/banner";
import { IGNORED_HERE_NOTE, IGNORED_NOTE, REASON_COPY, STATE_COPY } from "../src/views/messages/model";
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

  it("'ignore' folds a request on this device: it leaves the open list, waits under 'ignored requests', and is not deleted", async () => {
    const a = await load("requests");
    expect(apiStore.getState().attention.openRequests).toBe(2);
    const eli = apiStore.getState().threads!.find((t) => t.peer === ELI)!;
    apiStore.ignoreRequest(ELI, requestSeq(eli));
    const out = html(h(ThreadList));
    expect(out).toContain("message requests (1)");
    expect(out).toContain("ignored requests (1)");
    expect(out).toContain(IGNORED_NOTE);
    expect(out).toMatch(/>show again</);
    expect(out).toMatch(/>ignore</); // Dov's, still open
    expect(out).toMatch(/>forget request</);
    // The ignored one is inside the fold: after the disclosure opens, not in the open list before it.
    const open = out.slice(0, out.indexOf("ignored requests (1)"));
    const folded = out.slice(out.indexOf("ignored requests (1)"));
    expect(open).toContain("(if this is the wrong node, ignore me)");
    expect(open).not.toContain("Free coin");
    expect(folded).toContain("Free coin");
    // It is still pending on the node, and the badge no longer counts it.
    expect((await a.contacts.list()).find((c) => c.node_id === ELI)).toMatchObject({ request: true });
    expect(apiStore.getState().attention.openRequests).toBe(1);
  });

  it("ignoring every request leaves 'No new requests.' and the fold; the Contacts page shows the same fold", async () => {
    await load("requests");
    for (const t of apiStore.getState().threads!.filter((x) => x.request)) apiStore.ignoreRequest(t.peer, requestSeq(t));
    const list = html(h(ThreadList));
    expect(list).toContain("message requests (0)");
    expect(list).toContain("No new requests.");
    expect(list).toContain("ignored requests (2)");
    const contacts = html(h(ContactsView));
    expect(contacts).toContain("ignored requests (2)");
    expect(contacts).toContain("No new requests.");
    apiStore.restoreRequest(ELI);
    expect(html(h(ThreadList))).toContain("message requests (1)");
  });

  it("a request that writes again is no longer folded", async () => {
    const a = await load("requests");
    const eli = apiStore.getState().threads!.find((t) => t.peer === ELI)!;
    apiStore.ignoreRequest(ELI, requestSeq(eli));
    expect(html(h(ThreadList))).toContain("ignored requests (1)");
    // The sender writes again: the node stores the message and publishes it.
    const M = a.mock.state.messaging;
    const again = { ...peerMessage("one more thing"), id: "msg-again", peer: ELI, local_seq: M.nextSeq++, received_seq: M.nextRecv++ };
    M.messages.push(again);
    a.mock.bus.publish({ event: "message", data: { change: "request", peer: ELI, message: again } });
    await flush(30); // the frame reloads the list
    expect(apiStore.getState().ignored).toEqual({});
    expect(apiStore.getState().attention.openRequests).toBe(2);
    const out = html(h(ThreadList));
    expect(out).toContain("message requests (2)");
    expect(out).not.toContain("ignored requests");
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

  it("asks the agent by reason code: a hub with no members is offered 'let a node use this hub', an unreachable hub is a fix", async () => {
    await load("messaging-new");
    const services = apiStore.getState().services!;
    const setRow = (reason_code: string, reason: string) => {
      apiStore.applyFrame({ id: "9", event: "services", data: { services: services.map((s) => (s.id === "fabric" ? { ...s, state: "unset" as const, reason_code, reason } : s)) } });
      return html(h(MessagingBanner));
    };
    const waiting = setRow("hub_waiting", "This node is a home hub, waiting for the first node to enroll.");
    expect(waiting).toContain("Let a node use this hub, with the agent");
    expect(waiting).not.toContain("Set up messaging with the agent");
    const unreachable = setRow("hub_unreachable", "Your home hub cannot be reached right now; messages wait until it can.");
    expect(unreachable).toContain("Ask the agent to fix it");
    expect(unreachable).not.toContain("Set up messaging with the agent");
    const notEnrolled = setRow("not_enrolled", "No home hub and no other node reachable yet.");
    expect(notEnrolled).toContain("Set up messaging with the agent");
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

/** An incoming message as the node would publish it, for frames. */
const peerMessage = (text: string) => ({
  id: `msg-test-${text.length}-${text.charCodeAt(0)}`,
  peer: ADA,
  dir: "in" as const,
  text,
  state: "delivered" as const,
  reason_code: null,
  reason: null,
  local_seq: 800,
  received_seq: 800,
  sent_at_claim: null,
  at: "2026-10-14T09:00:00Z",
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
    expect(adv).toContain("what this state means: ACK from the recipient");
    expect(adv).not.toContain("evidence:");
    expect(adv).toContain("CUSTODY_ACK ACCEPTED");
    expect(adv).toContain("raw thread");
  });

  it("a pending request is read-only: no composer, the reason is given, accept, ignore and forget request are offered", async () => {
    await load("requests");
    await apiStore.loadThread(DOV);
    const out = html(h(ThreadView, { peer: DOV }));
    expect(out).not.toContain('aria-label="Message"');
    expect(out).toContain("Accept it to reply");
    expect(out).toMatch(/>accept</);
    expect(out).toMatch(/>ignore</);
    expect(out).toMatch(/>forget request</);
    expect(out).not.toContain(IGNORED_HERE_NOTE);
    expect(out).toContain("Is this the workshop node?");
  });

  it("an ignored request says so in its conversation, offers 'show again', and is still read-only and acceptable", async () => {
    await load("requests");
    await apiStore.loadThread(DOV);
    apiStore.ignoreRequest(DOV, requestSeq(apiStore.getState().threads!.find((t) => t.peer === DOV)!));
    const out = html(h(ThreadView, { peer: DOV }));
    expect(out).toContain(IGNORED_HERE_NOTE);
    expect(out).toMatch(/>show again</);
    expect(out).not.toMatch(/>ignore</);
    expect(out).toMatch(/>accept</);
    expect(out).toMatch(/>forget request</);
    expect(out).not.toContain('aria-label="Message"');
  });

  it("peer text is isolated for its own direction (dir=auto), so Persian reads right to left inside an LTR page", async () => {
    await load("healthy");
    await apiStore.loadThread(ADA);
    apiStore.applyFrame({ id: "9", event: "message", data: { change: "new", peer: ADA, message: peerMessage("سلام، حالت چطوره؟ ۱۲۳") } });
    const out = html(h(ThreadView, { peer: ADA }));
    expect(out).toMatch(/<div dir="auto"[^>]*>سلام، حالت چطوره؟ ۱۲۳<\/div>/);
    expect(out.match(/<div dir="auto"/g)!.length).toBeGreaterThanOrEqual(6); // every bubble, own and incoming
  });

  it("previews of held requests and last lines are isolated for their own direction too", async () => {
    await load("requests");
    const out = html(h(ThreadList));
    expect(out).toMatch(/<p dir="auto"[^>]*>[^<]*Free coin/);
    expect(out).toMatch(/<span dir="auto"[^>]*>[^<]*Thanks|<span dir="auto"[^>]*>you: /);
  });

  it("the Advanced raw block carries no bidi override that the bubbles have stripped", async () => {
    await load("healthy");
    await apiStore.loadThread(ADA);
    apiStore.applyFrame({ id: "9", event: "message", data: { change: "new", peer: ADA, message: peerMessage("abc‮def⁦ghi") } });
    const adv = html(h(ThreadView, { peer: ADA }), "advanced");
    expect(adv).toContain("raw thread");
    expect(adv).toContain("abcdefghi");
    expect(adv).not.toMatch(/[\u202A-\u202E\u2066-\u2069]/);
  });

  it("the per-message time line is the shared disclosure: a visible '>' marker, a focus ring and a 44 px target", async () => {
    await load("healthy");
    await apiStore.loadThread(ADA);
    const out = html(h(ThreadView, { peer: ADA }));
    const summaries = out.match(/<summary[^>]*>/g)!;
    expect(summaries.length).toBeGreaterThanOrEqual(6);
    for (const s of summaries) {
      expect(s).toContain("min-h-11");
      expect(s).toContain("focus-visible:ring-2");
    }
    expect(out).toMatch(/<summary[^>]*>.{0,120}class="inline-block w-3 shrink-0 text-center">&gt;<\/span>/);
    expect(out).not.toContain("min-h-8");
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

  it("the header badge opens Messages when message requests are all it counts, and Overview otherwise", async () => {
    const header = () => html(h(Header, { path: "/messages", drawerOpen: false, desktop: true, onMenu: () => undefined }));
    expect(header()).not.toContain("needs-you");
    apiStore.applyFrame({ id: "1", event: "contact", data: { change: "request", contact: makeContact(DOV, { trust: "tofu", request: true }) } });
    expect(apiStore.getState().attention).toMatchObject({ count: 1, openRequests: 1, openProposals: 0 });
    const only = header();
    expect(only).toContain('href="#/messages"');
    expect(only).not.toContain("needs-you");
    expect(only).toContain("1 message request");
    // Something else needs the person too: the badge goes back to the block that lists it.
    const services = buildFixtures(Date.parse("2026-10-14T09:00:00Z")).services;
    apiStore.applyFrame({ id: "2", event: "services", data: { services: services.map((s, i) => (i === 0 ? { ...s, state: "error" as const } : s)) } });
    expect(apiStore.getState().attention).toMatchObject({ count: 2, openRequests: 1, erroredServices: 1 });
    expect(header()).toContain('href="#/overview#needs-you"');
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
