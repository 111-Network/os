// Layer 6 V5-3 (os/6-transport/README.md §4, §10): a foreign row already sent again carries `superseded_by` (additive); the page never
// offers a second retry of it (the node refuses it with 409 too) and reads "resent as …" instead of the hand-off question.
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { isForeignRetryable, supersededBy } from "../src/lib/api/networks";
import type { ForeignMessage } from "../src/lib/api/types";
import { ModeProvider, resetModeForTests } from "../src/lib/mode";
import { ForeignRow, type RowActions } from "../src/views/messages/net/thread";

const NOW = Date.parse("2026-10-23T09:00:00Z");

const own = (over: Partial<ForeignMessage> = {}): ForeignMessage => ({
  id: "fm-7",
  conv: "fc-1",
  dir: "own",
  sender: { native: "!a1b2c3d4", name: "" },
  auth: "none",
  text: "meet at the ridge",
  state: "queued",
  evidence: "handed to the radio; no answer came",
  reason_code: "handoff_pending",
  reason: "It may have been sent.",
  group_id: "sg-3",
  part: null,
  relayed_from: null,
  via_internet: false,
  seq: 7,
  at: "2026-10-23T08:59:00Z",
  claim_ts: null,
  truncated: null,
  uncertain: true,
  not_relayed: null,
  ...over,
});

const actions: RowActions = { retry: () => undefined, cancel: () => undefined };

const render = (m: ForeignMessage): string => {
  resetModeForTests();
  try {
    return renderToStaticMarkup(h(ModeProvider, null, h(ForeignRow, { item: { kind: "row", m }, network: null, now: NOW, actions })));
  } finally {
    resetModeForTests();
  }
};

describe("a superseded foreign row (V5-3)", () => {
  it("is never retryable once superseded_by is set; an older daemon's row without the field still is", () => {
    expect(isForeignRetryable(own())).toBe(true);
    expect(isForeignRetryable(own({ superseded_by: null }))).toBe(true);
    expect(isForeignRetryable(own({ superseded_by: "fm-9" }))).toBe(false);
    expect(isForeignRetryable(own({ state: "failed", uncertain: false, reason_code: "no_channel", superseded_by: "fm-9" }))).toBe(false);
    expect(isForeignRetryable(own({ state: "failed", uncertain: false, reason_code: "no_channel" }))).toBe(true);
  });

  it("reads only a contract-shaped id from the wire", () => {
    expect(supersededBy(own({ superseded_by: "fm-9" }))).toBe("fm-9");
    expect(supersededBy(own())).toBeNull();
    for (const bad of ["", "msg-9", "fm-", "fm-9<b>", 9 as unknown as string]) expect(supersededBy(own({ superseded_by: bad }))).toBeNull();
  });

  it("renders 'resent as …' with no retry button and no hand-off question", () => {
    const before = render(own());
    expect(before).toContain('data-testid="retry-fm-7"');
    expect(before).toContain('data-testid="handoff-question"');
    expect(before).not.toContain('data-testid="resent-as"');
    const after = render(own({ superseded_by: "fm-9" }));
    expect(after).toContain('data-testid="resent-as"');
    expect(after).toContain("resent as fm-9");
    expect(after).not.toContain('data-testid="retry-fm-7"');
    expect(after).not.toContain('data-testid="handoff-question"');
  });
});
