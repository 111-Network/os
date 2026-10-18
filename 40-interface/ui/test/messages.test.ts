// Messages: the delivery-state matrix (os/50-messaging/README.md section 9), reasons, times, peer text, thread folding.
import { describe, expect, it } from "vitest";
import {
  applyMessageToPage,
  cleanDeep,
  cleanPeerText,
  conversationThreads,
  isCancellable,
  isCancelled,
  isRetryable,
  MESSAGE_STATE_ORDER,
  mergeThreadPage,
  newestIncomingSeq,
  newestReceivedSeq,
  pendingOwn,
  requestThreads,
  stateRank,
  trimPage,
  unreadTotal,
  upsertMessage,
  type ThreadPage,
} from "../src/lib/api/messages";
import { buildMessaging, cancelQueued, MOCK_REASONS, threadsOf, pageOf, ADA, BRAM, CLEO, DANA, DOV } from "../src/lib/api/mock/messaging";
import type { Message, MessageState, Thread } from "../src/lib/api/types";
import {
  announceIncoming,
  bannerAgent,
  CANCELLED_STATE_COPY,
  claimText,
  composeGate,
  draftProblem,
  errorSentence,
  evidenceName,
  FIX_QUESTION,
  FORGET_REQUEST_CONFIRM,
  glyphFor,
  IGNORED_HERE_NOTE,
  IGNORED_NOTE,
  lastLine,
  MAX_COMPOSE_CHARS,
  messageStateCopy,
  messagingBanner,
  OFFER_QUESTION,
  oneLine,
  peerFromParam,
  reasonText,
  REASON_COPY,
  REASON_COPY_BY_STATE,
  REQUEST_NOTE,
  requestAnnouncement,
  requestItemOf,
  requestPreview,
  SETUP_QUESTION,
  shortId,
  statusLine,
  stateCopy,
  STATE_COPY,
  threadGlyphState,
  threadName,
  timeLabel,
  unreadText,
  unresolved,
  whenText,
} from "../src/views/messages/model";
import { ApiError, synthProblem } from "../src/lib/api/problem";

const NOW = Date.parse("2026-10-14T09:00:00Z");

function msg(over: Partial<Message> = {}): Message {
  return {
    id: "msg-1",
    peer: ADA,
    dir: "own",
    text: "hello",
    state: "queued",
    reason_code: null,
    reason: null,
    local_seq: 1,
    received_seq: null,
    sent_at_claim: null,
    at: "2026-10-14T08:00:00Z",
    ...over,
  };
}

describe("delivery states (section 9)", () => {
  it("has the four states, weakest first, and ranks them", () => {
    expect(MESSAGE_STATE_ORDER).toEqual(["queued", "sent", "relayed", "delivered"]);
    expect(MESSAGE_STATE_ORDER.map(stateRank)).toEqual([0, 1, 2, 3]);
    expect(stateRank("from-the-future")).toBe(-1);
  });

  it("uses the evidence words of section 9, never more than the evidence says", () => {
    expect(STATE_COPY.queued).toMatch(/outbox/i);
    expect(STATE_COPY.queued).toMatch(/no path/i);
    expect(STATE_COPY.queued).toMatch(/key is missing/i);
    expect(STATE_COPY.queued).toMatch(/hub is not yours/i);
    expect(STATE_COPY.sent).toMatch(/written to a link/i);
    expect(STATE_COPY.sent).toMatch(/not confirmed/i);
    expect(STATE_COPY.relayed).toMatch(/mailbox node saved it and will hold it for them/i);
    expect(STATE_COPY.delivered).toMatch(/acknowledged/i);
    // Only the recipient's own ACK says delivered; nothing else may claim it.
    for (const s of ["queued", "sent", "relayed"] as MessageState[]) expect(STATE_COPY[s]).not.toMatch(/delivered/i);
  });

  it("draws a longer glyph as the evidence grows", () => {
    const filled = MESSAGE_STATE_ORDER.map((s) => glyphFor(s).filled);
    expect(filled).toEqual([0, 1, 2, 3]);
    for (const s of MESSAGE_STATE_ORDER) expect(glyphFor(s)).toMatchObject({ label: s, known: true });
  });

  it("renders an unknown state as unknown (the U1b rule), with a sentence that says so", () => {
    expect(glyphFor("teleported")).toEqual({ filled: 0, label: "unknown", known: false });
    expect(statusLine({ state: "teleported" as MessageState, reason_code: null, reason: null }).detail).toMatch(/does not know/i);
  });

  it("an unknown value never resolves to an Object.prototype member (the U1b rule holds for 'constructor' and friends)", () => {
    for (const odd of ["constructor", "toString", "hasOwnProperty", "__proto__", "valueOf"]) {
      expect(glyphFor(odd), odd).toEqual({ filled: 0, label: "unknown", known: false });
      expect(stateCopy(odd), odd).toBe("This node reports a state this page does not know.");
      expect(typeof reasonText({ state: odd as MessageState, reason_code: odd, reason: "the node's own words" }), odd).toBe("string");
      expect(reasonText({ state: odd as MessageState, reason_code: odd, reason: "the node's own words" }), odd).toBe("the node's own words");
      expect(reasonText({ state: odd as MessageState, reason_code: odd, reason: null }), odd).toBeNull();
      expect(statusLine({ state: odd as MessageState, reason_code: odd, reason: null }).label, odd).toBe("unknown");
    }
  });

  it("only an own message that is queued or sent can be retried", () => {
    const table: [Message["dir"], MessageState, boolean][] = [
      ["own", "queued", true],
      ["own", "sent", true],
      ["own", "relayed", false],
      ["own", "delivered", false],
      ["in", "delivered", false],
      ["in", "queued", false],
    ];
    for (const [dir, state, want] of table) expect(isRetryable({ dir, state }), `${dir} ${state}`).toBe(want);
  });

  it("only an own message that is still queued can be cancelled, and a cancelled one is neither cancellable nor retryable", () => {
    const table: [Message["dir"], MessageState, string | null, boolean][] = [
      ["own", "queued", null, true],
      ["own", "queued", "waiting_for_key", true],
      ["own", "queued", "hub_not_yours", true],
      ["own", "queued", "restore_pending", true],
      ["own", "queued", "cancelled", false], // already done
      ["own", "sent", null, false], // a frame was written: the words are out
      ["own", "sent", "mailbox_full", false],
      ["own", "relayed", null, false],
      ["own", "delivered", null, false],
      ["in", "delivered", null, false],
      ["in", "queued", null, false],
    ];
    for (const [dir, state, reason_code, want] of table) expect(isCancellable({ dir, state, reason_code }), `${dir} ${state} ${reason_code}`).toBe(want);
    // without a reason_code at all
    expect(isCancellable({ dir: "own", state: "queued" })).toBe(true);
    // nothing retries a cancelled message: its tombstone is the only thing that goes
    expect(isRetryable({ dir: "own", state: "queued", reason_code: "cancelled" })).toBe(false);
    expect(isRetryable({ dir: "own", state: "queued", reason_code: "waiting_for_key" })).toBe(true);
    expect(isCancelled({ dir: "own", reason_code: "cancelled" })).toBe(true);
    expect(isCancelled({ dir: "own", reason_code: "waiting_for_key" })).toBe(false);
    expect(isCancelled({ dir: "in", reason_code: "cancelled" })).toBe(false);
  });
});

describe("cancelling an own queued message (§4)", () => {
  const seeded = () => buildMessaging(Date.parse("2026-10-14T09:00:00Z")).messages;

  it("the cancelled message stays in the thread as queued, with the reason the node gives and this page's words for it", () => {
    const m = seeded().find((x) => x.peer === CLEO && x.dir === "own")!;
    expect(m).toMatchObject({ state: "queued", reason_code: "waiting_for_key" });
    expect(cancelQueued(m)).toBe("cancelled");
    expect(m).toMatchObject({ state: "queued", reason_code: "cancelled", reason: MOCK_REASONS.cancelled });
    expect(cancelQueued(m)).toBe("not_queued"); // once
    expect(reasonText(m)).toBe(REASON_COPY.cancelled);
    expect(REASON_COPY.cancelled).toMatch(/You cancelled this before it left/);
    expect(REASON_COPY.cancelled).toMatch(/empty placeholder/);
    expect(messageStateCopy(m)).toBe(CANCELLED_STATE_COPY);
    expect(messageStateCopy({ dir: "own", state: "queued", reason_code: "waiting_for_key" })).toBe(STATE_COPY.queued);
    expect(evidenceName(m)).toMatch(/cancelled before it left/);
    expect(evidenceName({ dir: "own", state: "queued", reason_code: null })).toMatch(/outbox/);
  });

  it("the mock refuses what the node refuses: a message that has left, and one that is not ours", () => {
    const all = seeded();
    const sent = all.find((x) => x.dir === "own" && x.state === "sent")!;
    expect(cancelQueued(sent)).toBe("not_queued");
    expect(sent.reason_code).not.toBe("cancelled");
    const delivered = all.find((x) => x.dir === "own" && x.state === "delivered")!;
    expect(cancelQueued(delivered)).toBe("not_queued");
    const incoming = all.find((x) => x.dir === "in")!;
    expect(cancelQueued(incoming)).toBe("not_own");
    expect(incoming.reason_code).toBeNull();
  });
});

describe("reasons", () => {
  it("has the spec wording for the codes the node emits", () => {
    expect(REASON_COPY.waiting_for_key).toMatch(/waiting for their key/i);
    expect(REASON_COPY.hub_not_yours).toMatch(/their hub is not yours; it is delivered when you meet directly/i);
    expect(REASON_COPY.mailbox_full).toMatch(/mailbox full/i);
    expect(REASON_COPY.store_full).toMatch(/store is full/i);
    expect(REASON_COPY.restore_pending).toMatch(/restore/i);
  });

  it("covers every code the node emits, and the mock's sentences agree on the meaning", () => {
    expect(Object.keys(REASON_COPY).sort()).toEqual(Object.keys(MOCK_REASONS).sort());
  });

  it("state x reason matrix: known codes use the UI copy, unknown codes the node's sentence, none says nothing", () => {
    const states: MessageState[] = ["queued", "sent"];
    for (const state of states) {
      for (const code of Object.keys(REASON_COPY)) {
        const want = REASON_COPY_BY_STATE[state]?.[code] ?? REASON_COPY[code];
        expect(reasonText({ state, reason_code: code, reason: "node sentence" }), `${state} ${code}`).toBe(want);
      }
      expect(reasonText({ state, reason_code: "brand_new_code", reason: "The node explains." })).toBe("The node explains.");
      expect(reasonText({ state, reason_code: "brand_new_code", reason: null })).toBeNull();
      expect(reasonText({ state, reason_code: null, reason: null })).toBeNull();
      expect(statusLine({ state, reason_code: null, reason: null }).reason).toBeNull();
    }
  });

  it("a custody REJECT_UNKNOWN_DEST (sent, hub_not_yours) says the hub does not know them, not the queued sentence", () => {
    const queued = reasonText({ state: "queued", reason_code: "hub_not_yours", reason: "node sentence" });
    const sent = reasonText({ state: "sent", reason_code: "hub_not_yours", reason: "node sentence" });
    expect(queued).toBe(REASON_COPY.hub_not_yours);
    expect(sent).toMatch(/your hub does not know them/i);
    expect(sent).not.toBe(queued);
    expect(statusLine({ state: "sent", reason_code: "hub_not_yours", reason: null }).reason).toBe(sent);
  });

  it("a code the UI has no copy for shows the node's own sentence, in any state, so a new daemon reason is not hidden", () => {
    for (const state of MESSAGE_STATE_ORDER) expect(reasonText({ state, reason_code: "some_future_reason", reason: "The node explains." }), state).toBe("The node explains.");
  });

  it("cleans a node sentence before it is shown", () => {
    expect(reasonText({ state: "queued", reason_code: "x", reason: "a‮b" })).toBe("ab");
  });

  it("names the evidence behind each state for Advanced", () => {
    expect(evidenceName({ dir: "own", state: "queued", reason_code: null })).toMatch(/outbox/);
    expect(evidenceName({ dir: "own", state: "sent", reason_code: null })).toMatch(/link/);
    expect(evidenceName({ dir: "own", state: "sent", reason_code: "mailbox_full" })).toMatch(/REJECT_FULL/);
    expect(evidenceName({ dir: "own", state: "sent", reason_code: "hub_not_yours" })).toMatch(/REJECT_UNKNOWN_DEST/);
    expect(evidenceName({ dir: "own", state: "relayed", reason_code: null })).toMatch(/CUSTODY_ACK ACCEPTED/);
    expect(evidenceName({ dir: "own", state: "delivered", reason_code: null })).toMatch(/ACK from the recipient/);
    expect(evidenceName({ dir: "in", state: "delivered", reason_code: null })).toMatch(/received here/);
  });
});

describe("times", () => {
  it("labels an incoming time 'received' and an own time 'written'", () => {
    expect(timeLabel({ dir: "in" })).toBe("received");
    expect(timeLabel({ dir: "own" })).toBe("written");
  });

  it("shows today as a clock time, the week as a weekday, older as a date", () => {
    expect(whenText("2026-10-14T06:05:00Z", NOW, "UTC")).toBe("06:05");
    expect(whenText("2026-10-13T12:30:00Z", NOW, "UTC")).toBe("Tue 12:30");
    expect(whenText("2026-10-02T08:05:00Z", NOW, "UTC")).toBe("2 Oct 08:05");
    expect(whenText("2026-10-14T23:59:00Z", NOW, "UTC")).toBe("23:59"); // a clock in the future is still just a time
  });

  it("says unknown for a missing or broken time instead of inventing one", () => {
    expect(whenText(null, NOW)).toBe("unknown");
    expect(whenText("yesterday", NOW)).toBe("unknown");
    expect(whenText(undefined, NOW)).toBe("unknown");
  });

  it("keeps the sender's claim apart, marked as theirs and unchecked, and only for incoming messages", () => {
    const claim = claimText(msg({ dir: "in", state: "delivered", sent_at_claim: "2026-10-14T05:00:00Z" }), NOW, "UTC");
    expect(claim).toMatch(/sender says/i);
    expect(claim).toMatch(/05:00/);
    expect(claim).toMatch(/cannot be checked/i);
    expect(claim).toMatch(/arrived here/i);
    expect(claimText(msg({ dir: "in", sent_at_claim: null }), NOW)).toBeNull();
    expect(claimText(msg({ dir: "own", sent_at_claim: "2026-10-14T05:00:00Z" }), NOW)).toBeNull();
  });
});

describe("peer text", () => {
  it("drops control characters and explicit bidi overrides, keeps newlines", () => {
    expect(cleanPeerText("a‮b⁦c⁩d\u0007e\u007Ff﻿g h")).toBe("abcdefgh");
    expect(cleanPeerText("line one\nline two\tend")).toBe("line one\nline two\tend");
  });

  it("keeps the joiners and direction marks that real text needs", () => {
    const persian = "می‌خواهم"; // ZWNJ inside a Persian word
    expect(cleanPeerText(persian)).toBe(persian);
    const family = "👩‍👩‍👧";
    expect(cleanPeerText(family)).toBe(family);
    expect(cleanPeerText("a‏b‎c")).toBe("a‏b‎c");
  });

  it("is plain text: markup is not interpreted (React escapes it; nothing here turns it into HTML)", () => {
    expect(cleanPeerText("<script>alert(1)</script>")).toBe("<script>alert(1)</script>");
  });

  it("cleans every string in a block that is printed as JSON (stringify leaves U+202E and the isolates alone)", () => {
    const raw = { peer: ADA, count: 2, ok: true, none: null, messages: [{ text: "a‮b⁦c", reason: "x\u202Ay", nested: { deep: ["\u2067z"] } }] };
    expect(JSON.stringify(raw)).toMatch(/\u202e|‮/);
    const cleaned = cleanDeep(raw);
    expect(cleaned).toEqual({ peer: ADA, count: 2, ok: true, none: null, messages: [{ text: "abc", reason: "xy", nested: { deep: ["z"] } }] });
    expect(JSON.stringify(cleaned)).not.toMatch(/[\u202A-\u202E\u2066-\u2069]/);
    expect(raw.messages[0]!.text).toBe("a‮b⁦c"); // a copy; the input is untouched
  });

  it("makes a one-line summary: flat, cut with an ellipsis, cleaned", () => {
    expect(oneLine("  a\n\nb\tc  ")).toBe("a b c");
    expect(oneLine("x".repeat(100), 10)).toBe("xxxxxxxxx…");
    expect(oneLine("‮abc")).toBe("abc");
  });
});

describe("thread rows", () => {
  const thread = (over: Partial<Thread> = {}): Thread => ({
    peer: ADA,
    alias: null,
    label: "abcd-efgh-ij",
    request: false,
    unread: 0,
    last: null,
    states: { queued: 0, sent: 0, relayed: 0, delivered: 0 },
    ...over,
  });

  it("names a thread by alias, else by the operator label", () => {
    expect(threadName(thread({ alias: "Ada" }))).toBe("Ada");
    expect(threadName(thread({ alias: "  " }))).toBe("abcd-efgh-ij");
    expect(threadName(thread())).toBe("abcd-efgh-ij");
    expect(threadName(thread({ alias: "A‮da" }))).toBe("Ada");
  });

  it("writes the last line: you: for own, plain for incoming, a quiet word for none", () => {
    expect(lastLine(thread())).toBe("no messages yet");
    expect(lastLine(thread({ last: msg({ text: "see you" }) }))).toBe("you: see you");
    expect(lastLine(thread({ last: msg({ dir: "in", state: "delivered", text: "ok\nfine" }) }))).toBe("ok fine");
  });

  it("shows a state glyph only when the last message is an own one", () => {
    expect(threadGlyphState(thread({ last: msg({ state: "relayed" }) }))).toBe("relayed");
    expect(threadGlyphState(thread({ last: msg({ dir: "in", state: "delivered" }) }))).toBeNull();
    expect(threadGlyphState(thread())).toBeNull();
  });

  it("summarises what is still unresolved", () => {
    expect(unresolved({ queued: 2, sent: 1, relayed: 0, delivered: 9 })).toBe("2 queued · 1 sent");
    expect(unresolved({ queued: 0, sent: 0, relayed: 1, delivered: 0 })).toBe("1 relayed");
    expect(unresolved({ queued: 0, sent: 0, relayed: 0, delivered: 5 })).toBe("");
    expect(pendingOwn({ states: { queued: 2, sent: 1, relayed: 1, delivered: 9 } })).toBe(4);
    expect(unreadText(0)).toBe("");
    expect(unreadText(3)).toBe("3 unread");
  });

  it("splits requests from conversations and counts unread only in conversations", () => {
    const ts = [thread({ peer: ADA, unread: 2 }), thread({ peer: DOV, request: true, unread: 1 }), thread({ peer: BRAM, unread: 1 })];
    expect(requestThreads(ts).map((t) => t.peer)).toEqual([DOV]);
    expect(conversationThreads(ts).map((t) => t.peer)).toEqual([ADA, BRAM]);
    expect(unreadTotal(ts)).toBe(3);
    expect(unreadTotal(null)).toBe(0);
  });

  it("previews a request from its held message, or says none is held", () => {
    expect(requestPreview(thread({ last: msg({ dir: "in", text: "hi there" }) }))).toBe("hi there");
    expect(requestPreview(thread())).toBe("no message held");
  });

  it("shortens an id", () => {
    expect(shortId(ADA)).toBe("3a7f09c1…2a41");
    expect(shortId("abc")).toBe("abc");
  });
});

describe("the mock's seeded threads exercise every state and reason", () => {
  const data = buildMessaging(NOW);
  const threads = threadsOf(data);

  it("has an own message in each of the four states", () => {
    const states = new Set(data.messages.filter((m) => m.dir === "own").map((m) => m.state));
    for (const s of MESSAGE_STATE_ORDER) expect(states.has(s), s).toBe(true);
  });

  it("explains every queued or rejected message with a reason the UI knows", () => {
    const explained = data.messages.filter((m) => m.dir === "own" && m.reason_code);
    expect(explained.map((m) => m.reason_code).sort()).toEqual(["hub_not_yours", "mailbox_full", "waiting_for_key"]);
    for (const m of explained) expect(reasonText(m)).toBe(REASON_COPY[m.reason_code!]);
  });

  it("orders threads by latest activity and counts unread against the read mark", () => {
    expect(threads[0]!.peer).toBe(ADA);
    expect(threads.map((t) => t.peer)).toContain(DANA);
    const ada = threads.find((t) => t.peer === ADA)!;
    expect(ada.unread).toBe(0);
    expect(ada.states.delivered).toBe(2);
    const cleo = threads.find((t) => t.peer === CLEO)!;
    expect(cleo.states.queued).toBe(1);
  });

  it("pages a conversation: the newest window, or forward from a sequence", () => {
    const newest = pageOf(data, ADA, undefined, 2);
    expect(newest.messages.map((m) => m.local_seq)).toEqual([5, 9]);
    expect(newest.more).toBe(true);
    const after = pageOf(data, ADA, 3, 10);
    expect(after.messages.map((m) => m.local_seq)).toEqual([4, 5, 9]);
    expect(after.more).toBe(false);
  });
});

describe("folding frames into a loaded conversation", () => {
  const base: ThreadPage = { peer: ADA, request: false, messages: [msg({ id: "a", local_seq: 1 }), msg({ id: "b", local_seq: 3 })], more: false };

  it("inserts by local_seq, replaces by id, and keeps identity when nothing changed", () => {
    const inserted = upsertMessage(base.messages, msg({ id: "c", local_seq: 2 }));
    expect(inserted.map((m) => m.id)).toEqual(["a", "c", "b"]);
    const replaced = upsertMessage(base.messages, msg({ id: "a", local_seq: 1, state: "delivered" }));
    expect(replaced[0]!.state).toBe("delivered");
    expect(replaced).not.toBe(base.messages);
    expect(upsertMessage(base.messages, base.messages[0]!)).toBe(base.messages);
  });

  it("a message never reorders by its timestamps, only by local_seq", () => {
    const late = msg({ id: "z", local_seq: 4, at: "2000-01-01T00:00:00Z", sent_at_claim: "1999-01-01T00:00:00Z" });
    expect(upsertMessage(base.messages, late).map((m) => m.id)).toEqual(["a", "b", "z"]);
  });

  it("applies a frame to a loaded conversation of that peer only", () => {
    const next = applyMessageToPage(base, ADA, msg({ id: "c", local_seq: 5 }), "new");
    expect(next?.messages.map((m) => m.id)).toEqual(["a", "b", "c"]);
    expect(applyMessageToPage(base, BRAM, msg({ id: "c", local_seq: 5 }), "new")).toBe(base);
    expect(applyMessageToPage(undefined, ADA, msg(), "new")).toBeUndefined();
    expect(applyMessageToPage(base, ADA, msg({ id: "a", local_seq: 1 }), "state")).toBe(base);
    expect(applyMessageToPage(base, ADA, msg({ id: "d", local_seq: 6, dir: "in" }), "request")?.request).toBe(true);
  });

  it("finds the newest incoming sequence for the read mark", () => {
    expect(newestIncomingSeq([msg({ local_seq: 1 }), msg({ id: "i1", dir: "in", local_seq: 2 }), msg({ id: "i2", dir: "in", local_seq: 7 }), msg({ local_seq: 9 })])).toBe(7);
    expect(newestIncomingSeq([msg()])).toBeNull();
  });

  it("finds the newest received sequence, which is what an ignored request is keyed on", () => {
    expect(newestReceivedSeq([msg({ id: "i1", dir: "in", local_seq: 2, received_seq: 4 }), msg({ id: "i2", dir: "in", local_seq: 7, received_seq: 9 }), msg({ local_seq: 9 })])).toBe(9);
    expect(newestReceivedSeq([msg()])).toBeNull();
  });
});

describe("an open conversation keeps to its window", () => {
  const own = (n: number): Message => msg({ id: `m${n}`, local_seq: n });
  const window = (n: number): ThreadPage => ({ peer: ADA, request: false, messages: Array.from({ length: n }, (_, i) => own(i + 1)), more: false });

  it("drops the oldest and says there are earlier ones once frames push it past the limit", () => {
    let page: ThreadPage | undefined = window(3);
    for (let n = 4; n <= 10; n++) page = applyMessageToPage(page, ADA, own(n), "new", 3);
    expect(page?.messages.map((m) => m.id)).toEqual(["m8", "m9", "m10"]);
    expect(page?.more).toBe(true);
  });

  it("a long-lived page never holds more than its limit, whatever the number of frames", () => {
    let page: ThreadPage | undefined = window(100);
    for (let n = 101; n <= 1100; n++) page = applyMessageToPage(page, ADA, own(n), "new", 100);
    expect(page?.messages).toHaveLength(100);
    expect(page?.messages[0]?.id).toBe("m1001");
    expect(page?.messages[99]?.id).toBe("m1100");
  });

  it("a state frame inside the window changes nothing about its size, and a page under its limit is left alone", () => {
    const page = window(3);
    expect(trimPage(page, 3)).toBe(page);
    expect(trimPage(page, 100)).toBe(page);
    const next = applyMessageToPage(page, ADA, { ...own(2), state: "sent" }, "state", 3);
    expect(next?.messages).toHaveLength(3);
    expect(next?.more).toBe(false);
  });

  it("a message older than a full window is not kept", () => {
    const full: ThreadPage = { peer: ADA, request: false, messages: [own(10), own(11), own(12)], more: true };
    const next = applyMessageToPage(full, ADA, own(5), "new", 3);
    expect(next?.messages.map((m) => m.id)).toEqual(["m10", "m11", "m12"]);
  });
});

describe("an older response never overwrites newer knowledge (mergeThreadPage)", () => {
  const own = (n: number, state: MessageState = "queued"): Message => msg({ id: `m${n}`, local_seq: n, state });
  const page = (messages: Message[], more = false): ThreadPage => ({ peer: ADA, request: false, messages, more });

  it("keeps a message a frame added after the snapshot was taken", () => {
    const merged = mergeThreadPage(page([own(1), own(2), own(3)]), page([own(1), own(2)]));
    expect(merged.messages.map((m) => m.id)).toEqual(["m1", "m2", "m3"]);
  });

  it("never lowers the evidence a message already has", () => {
    const merged = mergeThreadPage(page([own(1, "delivered")]), page([own(1, "sent")]));
    expect(merged.messages[0]!.state).toBe("delivered");
    expect(mergeThreadPage(page([own(1, "sent")]), page([own(1, "delivered")])).messages[0]!.state).toBe("delivered");
  });

  it("takes the response's window otherwise: older messages outside it go, its `more` stands", () => {
    const merged = mergeThreadPage(page([own(1), own(2), own(3)]), page([own(3), own(4)], true));
    expect(merged.messages.map((m) => m.id)).toEqual(["m3", "m4"]);
    expect(merged.more).toBe(true);
  });

  it("is the response itself when there is nothing to protect, and when the peer differs", () => {
    const fetched = page([own(1), own(2)]);
    expect(mergeThreadPage(null, fetched)).toBe(fetched);
    expect(mergeThreadPage(page([own(1), own(2)]), fetched)).toBe(fetched);
    expect(mergeThreadPage({ ...page([own(9)]), peer: BRAM }, fetched)).toBe(fetched);
  });
});

describe("composer rules", () => {
  const contact = { request: false, has_key: true };

  it("lets you write to a contact, with a hint while their key is missing", () => {
    expect(composeGate(contact, false)).toEqual({ ok: true, hint: null });
    const hint = composeGate({ request: false, has_key: false }, false);
    expect(hint.ok && hint.hint).toMatch(/waiting for their key/i);
  });

  it("refuses a stranger and a pending request, in words", () => {
    const stranger = composeGate(null, false);
    expect(!stranger.ok && stranger.why).toMatch(/add this person/i);
    const pending = composeGate({ request: true, has_key: true }, false);
    expect(!pending.ok && pending.why).toMatch(/accept it to reply/i);
    expect(composeGate(contact, true).ok).toBe(false);
  });

  it("checks the draft", () => {
    expect(draftProblem("")).toMatch(/write something/i);
    expect(draftProblem("  \n ")).toMatch(/write something/i);
    expect(draftProblem("hi")).toBeNull();
    expect(draftProblem("x".repeat(MAX_COMPOSE_CHARS + 1))).toMatch(/longer than/);
    expect(draftProblem("x".repeat(MAX_COMPOSE_CHARS))).toBeNull();
  });
});

describe("the messaging line above the list", () => {
  const live = { state: "live" as const, reason: "ok", reason_code: "hub_reachable" };
  const store = (full: boolean) => ({ store: { open: true, used: 1, watermark: 2, store_full: full } });

  it("says nothing while the service is live and the store has room, or when the daemon has no fabric service", () => {
    expect(messagingBanner(live, store(false))).toBeNull();
    expect(messagingBanner(null, null)).toBeNull();
    expect(messagingBanner(undefined, store(false))).toBeNull();
  });

  it("uses the node's own sentence when messaging is not set up, errored or starting", () => {
    expect(messagingBanner({ state: "unset", reason: "No home hub yet.", reason_code: "not_enrolled" }, null)).toEqual({ kind: "unset", text: "No home hub yet.", fixable: true });
    expect(messagingBanner({ state: "error", reason: "Store failed.", reason_code: "store_failed" }, null)).toEqual({ kind: "error", text: "Store failed.", fixable: true });
    expect(messagingBanner({ state: "starting", reason: "", reason_code: "starting" }, null)).toMatchObject({ kind: "starting", fixable: false });
  });

  it("spells out a full store even when the service row has not caught up", () => {
    const b = messagingBanner(live, store(true));
    expect(b).toMatchObject({ kind: "error", fixable: true });
    expect(b?.text).toMatch(/store is full/i);
  });

  describe("what the agent button asks follows the node's reason code", () => {
    const unset = messagingBanner({ state: "unset", reason: "x", reason_code: "not_enrolled" }, null)!;

    it("a member that is not enrolled is asked to set up (enroll with a hub)", () => {
      const a = bannerAgent(unset, "not_enrolled");
      expect(a).toMatchObject({ goal: "setup", question: SETUP_QUESTION, primary: true });
      expect(a?.label).toMatch(/set up messaging/i);
      expect(bannerAgent(unset, undefined)?.question).toBe(SETUP_QUESTION); // an unknown or missing code is the general set-up
      expect(bannerAgent(unset, "something_new")?.question).toBe(SETUP_QUESTION);
    });

    it("an unreachable home hub is a fix, although the node reports it as NOT SET", () => {
      const a = bannerAgent(unset, "hub_unreachable");
      expect(a).toMatchObject({ goal: "fix", question: FIX_QUESTION, primary: false });
      expect(a?.label).toMatch(/fix/i);
    });

    it("a hub with no members is offered 'let a node use this hub', not asked to enroll", () => {
      const a = bannerAgent(unset, "hub_waiting");
      expect(a?.question).toBe(OFFER_QUESTION);
      expect(a?.label).toMatch(/let a node use this hub/i);
      expect(a?.question).not.toMatch(/enrolling with a home hub/i);
      expect(a?.goal).toBe("setup");
    });

    it("an error is a fix, and a banner that cannot be fixed (starting) offers nothing", () => {
      expect(bannerAgent(messagingBanner({ state: "error", reason: "x", reason_code: "store_full" }, null)!, "store_full")).toMatchObject({ goal: "fix" });
      expect(bannerAgent(messagingBanner({ state: "starting", reason: "", reason_code: "starting" }, null)!, "starting")).toBeNull();
    });
  });
});

describe("what a screen reader is told about a changed list", () => {
  const thread = (peer: string, over: Partial<Thread> = {}): Thread => ({
    peer,
    alias: null,
    label: "abcd-efgh-ij",
    request: false,
    unread: 0,
    last: null,
    states: { queued: 0, sent: 0, relayed: 0, delivered: 0 },
    ...over,
  });
  const request = (peer: string, seq: number): Thread => thread(peer, { request: true, last: msg({ dir: "in", state: "delivered", received_seq: seq }) });

  it("says nothing about what was already there at load", () => {
    const first = announceIncoming(null, [thread(ADA, { unread: 3 }), request(DOV, 4)], {});
    expect(first.say).toBeNull();
  });

  it("announces a new request, and new unread messages in a conversation", () => {
    const base = announceIncoming(null, [thread(ADA)], {}).seen;
    const withRequest = announceIncoming(base, [thread(ADA), request(DOV, 4)], {});
    expect(withRequest.say).toBe("New message request.");
    expect(announceIncoming(withRequest.seen, [thread(ADA), request(DOV, 4)], {}).say).toBeNull();
    expect(announceIncoming(base, [thread(ADA, { unread: 1, alias: "Ada" })], {}).say).toBe("New message from Ada.");
  });

  it("an ignored request that writes again is announced, as the new request it is; ignoring and 'show again' are not", () => {
    const list = [request(DOV, 4)];
    const open = announceIncoming(null, list, {}).seen;
    // The user ignores it: nothing to say.
    const ignoredNow = announceIncoming(open, list, { [DOV]: 4 });
    expect(ignoredNow.say).toBeNull();
    // They show it again: same newest receipt, nothing to say.
    expect(announceIncoming(ignoredNow.seen, list, {}).say).toBeNull();
    // It stays ignored while it holds nothing newer.
    expect(announceIncoming(ignoredNow.seen, list, { [DOV]: 4 }).say).toBeNull();
    // The sender writes again: the fold lifts (the store drops the entry) and the screen reader hears of it.
    expect(announceIncoming(ignoredNow.seen, [request(DOV, 5)], {}).say).toBe("New message request.");
    // A message to a request that is not ignored is not a new request.
    expect(announceIncoming(open, [request(DOV, 5)], {}).say).toBeNull();
  });

  it("a request the user already ignored on this device is not announced when it first appears in the list", () => {
    const base = announceIncoming(null, [thread(ADA)], {}).seen;
    expect(announceIncoming(base, [thread(ADA), request(DOV, 4)], { [DOV]: 4 }).say).toBeNull();
  });
});

describe("ignoring a message request (section 7.2: kept, never auto-pruned, no ACK)", () => {
  const t = { peer: DOV, label: "abcd-efgh-ij", last: msg({ dir: "in", state: "delivered", text: "hi there", received_seq: 7 }) };

  it("says ignore keeps the request pending on the node and only forget request discards it", () => {
    expect(REQUEST_NOTE).toMatch(/ignore .*stays pending on the node/i);
    expect(REQUEST_NOTE).toMatch(/nothing is sent/i);
    expect(IGNORED_NOTE).toMatch(/stay pending on the node/i);
    expect(IGNORED_NOTE).toMatch(/nothing was sent/i);
    expect(IGNORED_NOTE).toMatch(/new message from one of them brings its request back/i);
    expect(IGNORED_HERE_NOTE).toMatch(/stays pending on the node/i);
    expect(FORGET_REQUEST_CONFIRM).toMatch(/their held words are discarded/);
    expect(FORGET_REQUEST_CONFIRM).toMatch(/if they write again it comes back/);
    expect(requestAnnouncement.ignored("Dov")).toMatch(/stays pending/);
    expect(requestAnnouncement.forgotten("Dov")).toMatch(/discarded/);
  });

  it("builds a list item with the newest receipt it holds", () => {
    expect(requestItemOf(t)).toEqual({ peer: DOV, label: "abcd-efgh-ij", preview: "hi there", seq: 7 });
    expect(requestItemOf({ ...t, last: null })).toMatchObject({ preview: "no message held", seq: 0 });
  });
});

describe("route parameter and errors", () => {
  it("accepts a NodeId as hex or node1…, and nothing else", () => {
    expect(peerFromParam(ADA)).toBe(ADA);
    expect(peerFromParam(undefined)).toBeNull();
    expect(peerFromParam("")).toBeNull();
    expect(peerFromParam("contacts")).toBeNull();
    expect(peerFromParam("../../etc/passwd")).toBeNull();
    expect(peerFromParam(ADA.toUpperCase())).toBeNull();
    expect(peerFromParam("node1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s2mc22u")).toBe("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
  });

  it("turns a failure into one sentence", () => {
    expect(errorSentence(new ApiError({ ...synthProblem(400, "Bad", "Text is too long."), code: "bad_request" }), "fallback")).toBe("Text is too long.");
    expect(errorSentence(ApiError.network(new TypeError("x")), "fallback")).toMatch(/could not reach the node/i);
    expect(errorSentence(new Error("boom"), "fallback")).toBe("fallback");
    expect(errorSentence("nope", "fallback")).toBe("fallback");
  });
});
