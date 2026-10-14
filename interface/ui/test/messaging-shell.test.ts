// Shell wiring for Layer 5 (U1): routes, titles, the sidebar row, and the header badge text for message requests.
import { describe, expect, it } from "vitest";
import { ROUTES } from "../src/app/routes";
import { attentionHref, attentionSummary } from "../src/lib/attention";
import { isActive, minModeFor, NAV, titleFor, visibleNav } from "../src/lib/nav";

describe("routes", () => {
  const paths = ROUTES.map((r) => r.path);

  it("has the conversation list, the contacts page and one conversation, with contacts matched first", () => {
    expect(paths).toContain("/messages");
    expect(paths).toContain("/messages/contacts");
    expect(paths).toContain("/messages/:peer");
    expect(paths.indexOf("/messages/contacts")).toBeLessThan(paths.indexOf("/messages/:peer"));
  });

  it("gives every route its own view component", () => {
    for (const r of ROUTES) expect(r.View, r.path).toBeTruthy();
  });
});

describe("titles and the sidebar", () => {
  it("names the Layer 5 pages", () => {
    expect(titleFor("/messages")).toBe("Messages");
    expect(titleFor("/messages/contacts")).toBe("Contacts");
    expect(titleFor(`/messages/${"ab".repeat(32)}`)).toBe("Conversation");
  });

  it("keeps Messages as the one core row, active on its children, for everyone", () => {
    expect(NAV.filter((i) => i.href.startsWith("/messages")).map((i) => i.href)).toEqual(["/messages"]);
    expect(isActive("/messages", "/messages/contacts")).toBe(true);
    expect(isActive("/messages", `/messages/${"ab".repeat(32)}`)).toBe(true);
    expect(isActive("/messages", "/messagesx")).toBe(false);
    expect(minModeFor("/messages")).toBeUndefined();
    expect(minModeFor("/messages/contacts")).toBeUndefined();
    expect(visibleNav("simple", null).core.map((i) => i.href)).toContain("/messages");
  });
});

describe("the header badge text", () => {
  it("counts message requests in plain words, alone or with the rest", () => {
    expect(attentionSummary({ openProposals: 0, erroredServices: 0, erroredDevices: 0, openRequests: 1 })).toBe("1 message request");
    expect(attentionSummary({ openProposals: 1, erroredServices: 0, erroredDevices: 0, openRequests: 3 })).toBe("1 proposal, 3 message requests");
    expect(attentionSummary({ openProposals: 2, erroredServices: 1, erroredDevices: 1, openRequests: 2 })).toBe("2 proposals, 1 service error, 1 device error, 2 message requests");
  });

  it("is unchanged when there are none, and tolerates an attention object without the field", () => {
    expect(attentionSummary({ openProposals: 0, erroredServices: 0, erroredDevices: 0, openRequests: 0 })).toBe("Nothing needs you");
    expect(attentionSummary({ openProposals: 2, erroredServices: 1, erroredDevices: 1 })).toBe("2 proposals, 1 service error, 1 device error");
  });
});

describe("where the header badge leads", () => {
  const none = { openProposals: 0, erroredServices: 0, erroredDevices: 0 };

  it("opens Messages when message requests are all it counts (Overview's block does not list them)", () => {
    expect(attentionHref({ ...none, openRequests: 1 })).toBe("/messages");
    expect(attentionHref({ ...none, openRequests: 5 })).toBe("/messages");
  });

  it("opens Overview's 'Needs you' whenever anything it lists is pending, requests or not", () => {
    expect(attentionHref({ ...none, openProposals: 1, openRequests: 0 })).toBe("/overview#needs-you");
    expect(attentionHref({ ...none, erroredServices: 1, openRequests: 2 })).toBe("/overview#needs-you");
    expect(attentionHref({ ...none, erroredDevices: 1, openRequests: 2 })).toBe("/overview#needs-you");
    expect(attentionHref({ ...none })).toBe("/overview#needs-you");
    expect(attentionHref({ ...none, openRequests: 0 })).toBe("/overview#needs-you");
  });
});

describe("Layer 5 pages (U2): Nearby and Settings -> Messaging & data", () => {
  const paths = ROUTES.map((r) => r.path);

  it("Nearby and the layer pages are routes; /settings/l5 is served by the layer route, after Licenses", () => {
    expect(paths).toContain("/nearby");
    expect(paths).toContain("/settings/:layer");
    expect(paths.indexOf("/settings/licenses")).toBeLessThan(paths.indexOf("/settings/:layer"));
  });

  it("titles", () => {
    expect(titleFor("/nearby")).toBe("Nearby");
    expect(titleFor("/settings/l5")).toBe("Messaging & data · L5");
    expect(titleFor("/services/fabric", [{ id: "fabric", plain_name: "Messaging" } as never])).toBe("Messaging");
    expect(titleFor("/services/meshtastic", [{ id: "meshtastic", plain_name: "Meshtastic" } as never])).toBe("Meshtastic");
  });

  it("the Messaging & data page is reachable in simple mode and never marked 'advanced view'; the other layer pages are advanced", () => {
    expect(minModeFor("/settings/l5")).toBeUndefined();
    expect(visibleNav("simple", null).settings.map((i) => i.href)).toEqual(["/settings", "/settings/l5"]);
    expect(visibleNav("advanced", null).settings.map((i) => i.href)).toContain("/settings/l5");
    for (const n of [0, 1, 2, 3, 4, 6, 7, 8, 9]) expect(minModeFor(`/settings/l${n}`)).toBe("advanced");
    expect(isActive("/settings/l5", "/settings/l5")).toBe(true);
  });
});
