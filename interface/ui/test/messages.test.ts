// Messages: the delivery-state matrix (os/messaging/README.md section 9), reasons, times, peer text, thread folding.
import { describe, expect, it } from "vitest";
import {
  applyMessageToPage,
  cleanPeerText,
  conversationThreads,
  isRetryable,
  MESSAGE_STATE_ORDER,
  newestIncomingSeq,
  pendingOwn,
  requestThreads,
  stateRank,
  unreadTotal,
  upsertMessage,
  type ThreadPage,
} from "../src/lib/api/messages";
import { buildMessaging, MOCK_REASONS, threadsOf, pageOf, ADA, BRAM, CLEO, DANA, DOV } from "../src/lib/api/mock/messaging";
import type { Message, MessageState, Thread } from "../src/lib/api/types";
import {
  claimText,
  composeGate,
  draftProblem,
  errorSentence,
  evidenceName,
  glyphFor,
  lastLine,
  MAX_COMPOSE_CHARS,
  messagingBanner,
  oneLine,
  peerFromParam,
  reasonText,
  REASON_COPY,
  requestPreview,
  shortId,
  statusLine,
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
        expect(reasonText({ reason_code: code, reason: "node sentence" }), `${state} ${code}`).toBe(REASON_COPY[code]);
      }
      expect(reasonText({ reason_code: "brand_new_code", reason: "The node explains." })).toBe("The node explains.");
      expect(reasonText({ reason_code: "brand_new_code", reason: null })).toBeNull();
      expect(reasonText({ reason_code: null, reason: null })).toBeNull();
      expect(statusLine({ state, reason_code: null, reason: null }).reason).toBeNull();
    }
  });

  it("cleans a node sentence before it is shown", () => {
    expect(reasonText({ reason_code: "x", reason: "a‮b" })).toBe("ab");
  });

  it("names the evidence behind each state for Advanced", () => {
    expect(evidenceName({ dir: "own", state: "queued", reason_code: null })).toMatch(/outbox/);
    expect(evidenceName({ dir: "own", state: "sent", reason_code: null })).toMatch(/link/);
    expect(evidenceName({ dir: "own", state: "sent", reason_code: "mailbox_full" })).toMatch(/REJECT_FULL/);
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
