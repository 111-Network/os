// Ignored message requests (os/messaging/README.md section 7.2): a fold the page keeps for itself, keyed
// (peer, newest received_seq), cleared by newer activity. The request itself stays pending on the node.
import { describe, expect, it } from "vitest";
import { own } from "../src/lib/api/own";
import {
  browserStorage,
  IGNORED_MAX,
  IGNORED_STORAGE_KEY,
  ignoreRequest,
  isIgnored,
  loadIgnored,
  NO_IGNORED,
  parseIgnored,
  pruneIgnored,
  requestSeq,
  restoreRequest,
  saveIgnored,
  type KeyValueStorage,
} from "../src/lib/api/requests";
import { makeContact } from "../src/lib/api/mock/messaging";
import type { Message, Thread } from "../src/lib/api/types";

const A = "a".repeat(64);
const B = "b".repeat(64);
const C = "c".repeat(64);

const msg = (seq: number | null): Message => ({
  id: `m-${seq}`,
  peer: A,
  dir: "in",
  text: "hi",
  state: "delivered",
  reason_code: null,
  reason: null,
  local_seq: seq ?? 0,
  received_seq: seq,
  sent_at_claim: null,
  at: "2026-10-14T09:00:00Z",
});
const thread = (peer: string, seq: number | null, request = true): Thread => ({
  peer,
  alias: null,
  label: "abcd-efgh-ij",
  request,
  unread: 0,
  last: seq === null ? null : msg(seq),
  states: { queued: 0, sent: 0, relayed: 0, delivered: 0 },
});
const req = (peer: string, request = true) => makeContact(peer, { trust: "tofu", request });

function memory(initial: Record<string, string> = {}): KeyValueStorage & { data: Record<string, string> } {
  const data = { ...initial };
  return {
    data,
    getItem: (k) => (k in data ? data[k]! : null),
    setItem: (k, v) => {
      data[k] = v;
    },
    removeItem: (k) => {
      delete data[k];
    },
  };
}

describe("the fold: (peer, newest received_seq)", () => {
  it("hides a request while it holds nothing newer than what was dismissed", () => {
    const ignored = ignoreRequest(NO_IGNORED, A, 7);
    expect(isIgnored(ignored, A, 7)).toBe(true);
    expect(isIgnored(ignored, A, 3)).toBe(true);
    expect(isIgnored(ignored, A, 8)).toBe(false); // a newer message lifts the fold
    expect(isIgnored(ignored, B, 7)).toBe(false); // only that peer
  });

  it("never hides what it cannot judge: an unknown newest sequence shows the request", () => {
    expect(isIgnored(ignoreRequest(NO_IGNORED, A, 7), A, null)).toBe(false);
  });

  it("a request that holds no message is keyed at 0", () => {
    expect(requestSeq({ last: null })).toBe(0);
    expect(requestSeq(thread(A, 9))).toBe(9);
    expect(isIgnored(ignoreRequest(NO_IGNORED, A, 0), A, 0)).toBe(true);
    expect(isIgnored(ignoreRequest(NO_IGNORED, A, 0), A, 1)).toBe(false);
  });

  it("ignoring again raises the key, never lowers it, and does not touch the input", () => {
    const once = ignoreRequest(NO_IGNORED, A, 7);
    expect(ignoreRequest(once, A, 9)[A]).toBe(9);
    expect(ignoreRequest(once, A, 2)[A]).toBe(7);
    expect(once[A]).toBe(7);
    expect(NO_IGNORED).toEqual({});
  });

  it("restoring removes the entry (and is the same object when there was none)", () => {
    const ignored = ignoreRequest(ignoreRequest(NO_IGNORED, A, 1), B, 2);
    expect(restoreRequest(ignored, A)).toEqual({ [B]: 2 });
    expect(restoreRequest(ignored, C)).toBe(ignored);
  });

  it("is bounded by the node's pending-request quota: the oldest entries make room", () => {
    let ignored = NO_IGNORED;
    for (let i = 0; i < IGNORED_MAX + 10; i++) ignored = ignoreRequest(ignored, i.toString(16).padStart(64, "0"), i);
    expect(Object.keys(ignored)).toHaveLength(IGNORED_MAX);
    expect(own(ignored, (0).toString(16).padStart(64, "0"))).toBe(false);
    expect(own(ignored, (IGNORED_MAX + 9).toString(16).padStart(64, "0"))).toBe(true);
  });
});

describe("pruning what no longer means anything", () => {
  const ignored = { [A]: 5, [B]: 5, [C]: 5 };

  it("drops a peer that is no longer a pending request (accepted or forgotten)", () => {
    const pruned = pruneIgnored(ignored, [req(A), req(B, false)], null);
    expect(pruned).toEqual({ [A]: 5 }); // B accepted, C forgotten
  });

  it("drops a peer whose request gained a newer message, keeps one that did not", () => {
    const pruned = pruneIgnored(ignored, [req(A), req(B), req(C)], [thread(A, 5), thread(B, 6), thread(C, 4)]);
    expect(pruned).toEqual({ [A]: 5, [C]: 5 });
  });

  it("an unloaded list proves nothing: nothing is dropped", () => {
    expect(pruneIgnored(ignored, null, null)).toBe(ignored);
    expect(pruneIgnored(ignored, [req(A), req(B), req(C)], null)).toBe(ignored);
    expect(pruneIgnored(ignored, null, [thread(A, 5)])).toBe(ignored);
  });

  it("a request whose thread is not in the list yet is kept", () => {
    expect(pruneIgnored(ignored, [req(A), req(B), req(C)], [thread(A, 5)])).toBe(ignored);
  });

  it("is the same object when nothing was dropped, and for an empty set", () => {
    expect(pruneIgnored(NO_IGNORED, [], [])).toBe(NO_IGNORED);
    expect(pruneIgnored(ignored, [req(A), req(B), req(C)], [thread(A, 5), thread(B, 5), thread(C, 5)])).toBe(ignored);
  });
});

describe("keeping the set on this device", () => {
  it("round-trips through storage, and an empty set removes the key", () => {
    const store = memory();
    saveIgnored(store, { [A]: 5, [B]: 9 });
    expect(JSON.parse(store.data[IGNORED_STORAGE_KEY]!)).toEqual({ [A]: 5, [B]: 9 });
    expect(loadIgnored(store)).toEqual({ [A]: 5, [B]: 9 });
    saveIgnored(store, NO_IGNORED);
    expect(IGNORED_STORAGE_KEY in store.data).toBe(false);
    expect(loadIgnored(store)).toBe(NO_IGNORED);
  });

  it("trusts nothing it reads back: junk, wrong shapes and bad entries are dropped", () => {
    expect(parseIgnored(null)).toBe(NO_IGNORED);
    expect(parseIgnored("")).toBe(NO_IGNORED);
    expect(parseIgnored("{not json")).toBe(NO_IGNORED);
    expect(parseIgnored("[1,2]")).toBe(NO_IGNORED);
    expect(parseIgnored("null")).toBe(NO_IGNORED);
    expect(parseIgnored(JSON.stringify({ [A]: 5, nothex: 3, [B]: -1, [C]: 1.5, ["d".repeat(64)]: "7", __proto__: 3 }))).toEqual({ [A]: 5 });
    const polluted = parseIgnored('{"__proto__":{"x":1}}');
    expect(({} as Record<string, unknown>).x).toBeUndefined();
    expect(polluted).toBe(NO_IGNORED);
  });

  it("works with no storage, and with storage that throws (private windows, blocked site data)", () => {
    const boom: KeyValueStorage = {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("full");
      },
      removeItem: () => {
        throw new Error("blocked");
      },
    };
    expect(loadIgnored(null)).toBe(NO_IGNORED);
    expect(loadIgnored(boom)).toBe(NO_IGNORED);
    expect(() => saveIgnored(null, { [A]: 1 })).not.toThrow();
    expect(() => saveIgnored(boom, { [A]: 1 })).not.toThrow();
    expect(() => saveIgnored(boom, NO_IGNORED)).not.toThrow();
  });

  it("finds no browser storage in this environment, and does not throw looking", () => {
    expect(browserStorage()).toBeNull();
  });
});

describe("table lookups", () => {
  it("own() sees a table's keys and nothing from Object.prototype", () => {
    const table = { a: 1 };
    expect(own(table, "a")).toBe(true);
    for (const k of ["constructor", "toString", "hasOwnProperty", "__proto__", "valueOf", "nope"]) expect(own(table, k), k).toBe(false);
  });
});
