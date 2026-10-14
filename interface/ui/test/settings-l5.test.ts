// Settings -> Messaging & data (os/messaging/README.md sections 7.3, 9, 10, 16): what is typed and scanned (parsed by the page, judged
// by the node), the calls the page makes and what it shows afterwards, the loopback-only rows over the LAN, and the fabric service
// page. Driven through the real store against the mock adapter's `messaging-new`, `healthy`, `store-full`, `restore-pending` and
// `lan-unpaired` scenarios.
import { createElement as h, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import { createMockAdapter, MOCK_PAIR_CODE, type MockAdapter, type ScenarioName } from "../src/lib/api/mock";
import { HUB, MOCK_HUB_ADDRESS, MOCK_HUB_CODE } from "../src/lib/api/mock/messaging";
import { ApiError } from "../src/lib/api/problem";
import { apiStore } from "../src/lib/api/store";
import type { FabricStatus, HubOffer } from "../src/lib/api/types";
import { ModeProvider, resetModeForTests } from "../src/lib/mode";
import ServicesView from "../src/views/services";
import { custodyText, hubLine, hubName, linkLabel, linkLine, profileSummary, STORE_FULL_TEXT } from "../src/views/services/fabric-model";
import { changeQuotas, changeSetting, dismissOffer, enrollWithHub, mintOffer } from "../src/views/settings/messaging-actions";
import {
  CODE_LENGTH,
  codeComplete,
  enrollErrorText,
  enrollRequest,
  fabricAgent,
  formatCode,
  normalizeCode,
  offerAddressText,
  offerCodeText,
  parseAddress,
  parseEnrollBundle,
  parseQuota,
  quotaPatch,
  quotaRows,
  restorePending,
  RESTORE_PENDING_TEXT,
} from "../src/views/settings/messaging-model";
import { MessagingSettings, OfferCard } from "../src/views/settings/messaging";
import SettingsView from "../src/views/settings";
import { LOOPBACK_ONLY_TEXT } from "../src/views/settings/model";
import { FakeClock, flush } from "./helpers/fakes";
import { routed } from "./helpers/routed";

const NOW = Date.parse("2026-10-14T09:00:00Z");
const made: MockAdapter[] = [];
afterEach(() => {
  apiStore.detach();
  apiStore.reset();
  for (const a of made.splice(0)) a.mock.dispose();
});

async function load(scenario: ScenarioName): Promise<MockAdapter> {
  const a = createMockAdapter({ scenario, clock: new FakeClock(NOW), tokenDelayMs: 0 });
  made.push(a);
  apiStore.attach(a);
  await flush(30);
  return a;
}

const html = (el: ReactElement, search = ""): string => {
  const g = globalThis as { window?: unknown };
  const had = g.window;
  g.window = { location: { search, hash: "", pathname: "/", protocol: "http:", hostname: "127.0.0.1", port: "11180" }, history: { state: null, replaceState: () => undefined }, localStorage: { getItem: () => null, setItem: () => undefined } };
  resetModeForTests();
  try {
    return renderToStaticMarkup(h(ModeProvider, null, el));
  } finally {
    if (had === undefined) delete g.window;
    else g.window = had;
    resetModeForTests();
  }
};

const fabric = (): FabricStatus => apiStore.getState().fabric!;
/** "ON" or "OFF" as the switch labelled `label` shows it in the page. */
const toggleOf = (out: string, label: string): string | null => new RegExp(`<span>${label}</span> <span aria-hidden="true">\\[(ON|OFF)\\]</span>`).exec(out)?.[1] ?? null;
const problem = (status: number, code: string, detail?: string) => new ApiError({ type: "about:blank", title: "t", status, code: code as never, ...(detail ? { detail } : {}) });

// ---------------------------------------------------------------------------------------------- parsing

describe("what is typed into the enroll form", () => {
  it("the code is 8 letters and digits, shown xxxx-xxxx; dashes, spaces and case are dropped", () => {
    expect(CODE_LENGTH).toBe(8);
    expect(formatCode("k7m29x4p")).toBe("K7M2-9X4P");
    expect(formatCode("k7m2 9x-4p!!extra")).toBe("K7M2-9X4P");
    expect(formatCode("K7M")).toBe("K7M");
    expect(normalizeCode("k7m2-9x4p")).toBe("K7M29X4P");
    expect(codeComplete("K7M2-9X4P")).toBe(true);
    expect(codeComplete("K7M2-9X4")).toBe(false);
    expect(codeComplete("")).toBe(false);
  });

  it("the address is host:port: a name, an IPv4 or a bracketed IPv6 address, and a port from 1 to 65535", () => {
    for (const ok of ["192.168.1.20:11183", "hub.local:11183", "111-hub.lan:1", "[fe80::1]:11183", "  10.0.0.2:65535  "]) {
      expect(parseAddress(ok), ok).toMatchObject({ ok: true });
    }
    expect(parseAddress("192.168.1.20:11183")).toEqual({ ok: true, address: "192.168.1.20:11183" });
    for (const bad of ["", "192.168.1.20", "192.168.1.20:0", "192.168.1.20:65536", "192.168.1.20:abc", "http://192.168.1.20:11183", "192.168.1.20:11183/path", ":11183", "bad host:11183", "-bad.example:1"]) {
      expect(parseAddress(bad), bad).toMatchObject({ ok: false });
    }
    expect(parseAddress("http://hub:1")).toMatchObject({ ok: false, text: expect.stringMatching(/without http/) });
  });

  it("the request carries the normalized code, the checked address, and the card when there is one", () => {
    expect(enrollRequest("192.168.1.20:11183", "k7m2-9x4p", null)).toEqual({ ok: true, body: { address: "192.168.1.20:11183", code: "K7M29X4P" } });
    expect(enrollRequest("192.168.1.20:11183", "K7M2-9X4P", "CARDTEXT")).toEqual({ ok: true, body: { address: "192.168.1.20:11183", code: "K7M29X4P", card: "CARDTEXT" } });
    expect(enrollRequest("", "K7M2-9X4P", null)).toMatchObject({ ok: false, text: expect.stringMatching(/address/) });
    expect(enrollRequest("192.168.1.20:11183", "K7M2", null)).toMatchObject({ ok: false, text: expect.stringMatching(/8 letters and digits/) });
  });

  it("a hub's enrollment text (111HUB:<card>:<code>:<address>) is read into card, code and address; anything else is not understood", () => {
    const card = "MFRGGZDFMZTWQ2LKNNWG23TPOBYXE43UOV3HO6DZPJAGK43FOJUWIZLTEB";
    expect(parseEnrollBundle(`111HUB:${card}:K7M29X4P:192.168.1.20:11183`)).toEqual({ card, code: "K7M29X4P", address: "192.168.1.20:11183" });
    // the prefix's case does not matter; the address may contain colons (IPv6) and is last
    expect(parseEnrollBundle(`  111hub:${card}:k7m2-9x4p:[fe80::1]:11183\n`)).toEqual({ card, code: "K7M29X4P", address: "[fe80::1]:11183" });
    // a hub with no LAN address yet: the address field is empty, the page asks for it
    expect(parseEnrollBundle(`111HUB:${card}:K7M29X4P:`)).toEqual({ card, code: "K7M29X4P", address: "" });
    for (const bad of [
      "",
      "K7M2-9X4P",
      "111HUB:",
      `111HUB:${card}:K7M29X4P`, // no address field at all
      `111HUB:${card}:K7M29X4P:192.168.1.20:99999`,
      `111HUB:${card}:K7M2:192.168.1.20:11183`,
      `111HUB::K7M29X4P:192.168.1.20:11183`,
      `111HUB:short:K7M29X4P:192.168.1.20:11183`,
      `111HUB:${card}:K7M29X4P:http://192.168.1.20:11183`,
      `111HUB:${"A".repeat(700)}:K7M29X4P:192.168.1.20:11183`,
      `http://x/${card}`,
    ]) {
      expect(parseEnrollBundle(bad), bad.slice(0, 40)).toBeNull();
    }
  });

  it("the node's answer to an enroll becomes one plain sentence", () => {
    expect(enrollErrorText(problem(409, "bad_request"))).toMatch(/already has a home hub/);
    expect(enrollErrorText(problem(429, "rate_limited"))).toMatch(/locked this code after five wrong tries/);
    expect(enrollErrorText(problem(502, "upstream_error"))).toMatch(/could not be reached, or it did not accept that code/);
    expect(enrollErrorText(problem(400, "bad_request", "address must be host:port"))).toBe("address must be host:port");
    expect(enrollErrorText(ApiError.network(new Error("x")))).toMatch(/could not be reached/);
    expect(enrollErrorText(new Error("boom"))).toBe("boom");
    expect(enrollErrorText("odd")).toBe("Something went wrong.");
  });
});

describe("quotas", () => {
  it("shows the known classes first, in megabytes, and any other class the node reports", () => {
    const rows = quotaRows({ msg: 50_000_000, kb: null, extra: 1_500_000 });
    expect(rows.map((r) => [r.id, r.mb])).toEqual([["msg", "50"], ["agent_op", ""], ["kb", ""], ["update", ""], ["patch", ""], ["extra", "1.5"]]);
    expect(quotaRows({}).every((r) => r.mb === "")).toBe(true);
  });
  it("a box is a number of megabytes or empty for none", () => {
    expect(parseQuota("")).toEqual({ ok: true, bytes: null });
    expect(parseQuota("50")).toEqual({ ok: true, bytes: 50_000_000 });
    expect(parseQuota(" 0.5 ")).toEqual({ ok: true, bytes: 500_000 });
    for (const bad of ["-1", "1e3", "abc", "5 MB", "1,5"]) expect(parseQuota(bad), bad).toMatchObject({ ok: false });
    expect(parseQuota("9".repeat(30))).toMatchObject({ ok: false });
  });
  it("only changed classes go to the node; clearing a box drops its quota", () => {
    const current = { msg: 50_000_000 };
    expect(quotaPatch(current, { msg: "50", kb: "" })).toEqual({ ok: true, quotas: null });
    expect(quotaPatch(current, { msg: "", kb: "10" })).toEqual({ ok: true, quotas: { msg: null, kb: 10_000_000 } });
    expect(quotaPatch(current, { msg: "x" })).toMatchObject({ ok: false });
  });
});

describe("restore wait and the agent's goal", () => {
  it("a restore is pending when the fabric row says so or an own message is queued for it", async () => {
    await load("restore-pending");
    const svc = apiStore.getState().services!.find((s) => s.id === "fabric")!;
    expect(restorePending(svc, apiStore.getState().threads)).toBe(true);
    expect(restorePending(null, null)).toBe(false);
    expect(restorePending({ reason_code: "restore_pending" }, null)).toBe(true);
  });
  it("is not pending on a healthy node", async () => {
    await load("healthy");
    expect(restorePending(apiStore.getState().services!.find((s) => s.id === "fabric"), apiStore.getState().threads)).toBe(false);
  });
  it("offers a fix for a full or closed store, an error and an unreachable hub; set-up when NOT SET; nothing special when it works", async () => {
    await load("healthy");
    const f = fabric();
    const full = { ...f, store: { ...f.store, store_full: true } };
    expect(fabricAgent({ state: "live", reason_code: "hub_reachable" }, full)).toMatchObject({ goal: "messaging.fix" });
    expect(fabricAgent({ state: "live", reason_code: "hub_reachable" }, { ...f, store: { ...f.store, open: false } })).toMatchObject({ goal: "messaging.fix" });
    expect(fabricAgent({ state: "error", reason_code: "store_failed" }, f)).toMatchObject({ goal: "messaging.fix" });
    expect(fabricAgent({ state: "unset", reason_code: "hub_unreachable" }, f)).toMatchObject({ goal: "messaging.fix" });
    expect(fabricAgent({ state: "unset", reason_code: "not_enrolled" }, f)).toMatchObject({ goal: "messaging.setup", label: "Set up messaging with the agent" });
    expect(fabricAgent({ state: "live", reason_code: "hub_reachable" }, f)).toBeNull();
    expect(fabricAgent(null, null)).toBeNull();
  });
});

// ---------------------------------------------------------------------------------------------- flows

describe("enroll against the mock hub", () => {
  it("a wrong code is refused and nothing changes; the fifth failure locks it; the right code enrolls and the store follows", async () => {
    const a = await load("messaging-new");
    expect(fabric().hub.enrolled).toBe(false);
    const wrong = enrollRequest(MOCK_HUB_ADDRESS, "AAAA-AAAA", null);
    expect(wrong.ok).toBe(true);
    if (!wrong.ok) return;
    const failed = await enrollWithHub(a, apiStore, wrong.body).catch((e: unknown) => e);
    expect(enrollErrorText(failed)).toMatch(/could not be reached, or it did not accept/);
    expect(fabric().hub.enrolled).toBe(false);
    for (let i = 0; i < 4; i++) await enrollWithHub(a, apiStore, wrong.body).catch(() => undefined);
    const locked = await enrollWithHub(a, apiStore, { ...wrong.body, code: MOCK_HUB_CODE }).catch((e: unknown) => e);
    expect(enrollErrorText(locked)).toMatch(/locked this code after five wrong tries/);
    expect(fabric().hub.enrolled).toBe(false);
  });

  it("a typed code in any case, with the hub's address, enrolls; the hub appears as a contact and the service goes LIVE", async () => {
    const a = await load("messaging-new");
    const req = enrollRequest(MOCK_HUB_ADDRESS, "k7m2-9x4p", null);
    if (!req.ok) throw new Error(req.text);
    await enrollWithHub(a, apiStore, req.body);
    expect(fabric().hub).toMatchObject({ enrolled: true, reachable: true, node_id: HUB, address: MOCK_HUB_ADDRESS });
    expect(apiStore.getState().contacts!.some((c) => c.node_id === HUB && c.trust === "enrolled")).toBe(true);
    expect(apiStore.getState().services!.find((s) => s.id === "fabric")).toMatchObject({ state: "live", reason_code: "hub_reachable" });
    expect(hubLine(fabric().hub, apiStore.getState().contacts)).toBe("Home hub reachable: Workshop hub");
    // a second enroll is refused with the node's 409
    const again = await enrollWithHub(a, apiStore, req.body).catch((e: unknown) => e);
    expect(enrollErrorText(again)).toMatch(/already has a home hub/);
  });

  it("the page then shows the enrolled hub instead of the form", async () => {
    const a = await load("messaging-new");
    const before = html(h(MessagingSettings));
    expect(before).toContain("the hub&#x27;s enrollment text");
    expect(before).toContain(">Enroll<");
    expect(before).toContain("Set up messaging with the agent");
    expect(before).toContain("NOT SET");
    const req = enrollRequest(MOCK_HUB_ADDRESS, MOCK_HUB_CODE, null);
    if (!req.ok) throw new Error(req.text);
    await enrollWithHub(a, apiStore, req.body);
    const after = html(h(MessagingSettings));
    expect(after).not.toContain("the hub&#x27;s enrollment text");
    expect(after).toContain("Workshop hub");
    expect(after).toContain("Home hub reachable");
    expect(after).toContain(MOCK_HUB_ADDRESS);
    expect(after).toContain("To leave this hub, forget it in");
  });
});

describe("the hub's code", () => {
  it("minting shows a one-time code and the store learns an offer is outstanding; dismissing clears it; dismissing again is fine", async () => {
    const a = await load("healthy");
    expect(fabric().hub.offer_active).toBe(false);
    const offer = await mintOffer(a, apiStore);
    expect(offer.code).toMatch(/^[A-Z0-9]{8}$/);
    expect(offer.bundle.text).toContain(offer.code);
    expect(fabric().hub.offer_active).toBe(true);
    // the real daemon's bundle is `111HUB:<card>:<code>:<address>`; the page reads that back into the same code and address
    const read = parseEnrollBundle(`111HUB:${offer.card.text}:${offer.code}:${offer.address}`);
    expect(read).toMatchObject({ address: offer.address, code: offer.code });
    await dismissOffer(a, apiStore);
    expect(fabric().hub.offer_active).toBe(false);
    await dismissOffer(a, apiStore);
    expect(fabric().hub.offer_active).toBe(false);
  });

  it("an outstanding code that was shown before is described, never re-displayed", async () => {
    const a = await load("healthy");
    await mintOffer(a, apiStore);
    const out = html(h(MessagingSettings));
    expect(out).toContain('data-testid="offer-outstanding"');
    expect(out).toContain("it was shown once and cannot be shown again");
    expect(out).toContain("Show a new code");
    expect(out).toContain("Dismiss the code");
    expect(out).not.toContain('data-testid="offer-code"');
  });

  it("the code card shows the grouped code, the address, a QR and the text to copy, and the rules", async () => {
    const a = await load("healthy");
    const offer = await mintOffer(a, apiStore);
    const out = html(h(OfferCard, { offer }));
    expect(out).toContain(offerCodeText(offer));
    expect(offerCodeText(offer)).toMatch(/^[A-Z0-9]{4}-[A-Z0-9]{4}$/);
    expect(out).toContain(offer.address!);
    expect(out).toContain("<canvas");
    expect(out).toContain("The code works once");
    expect(out).toContain("After five wrong tries the hub locks it");
  });

  it("a hub with no LAN address says so instead of showing nothing", () => {
    const offer = { schema: 1, code: "K7M29X4P", address: null, card: { text: "x", qr: { size: 21, rows: [] } }, bundle: { text: "x", qr: { size: 21, rows: [] } } } as unknown as HubOffer;
    expect(offerAddressText(offer)).toMatch(/does not know its network address yet/);
    expect(html(h(OfferCard, { offer }))).toContain("Turn on the LAN listener");
  });
});

describe("settings: the node answers, then the page shows it", () => {
  it("a switch changes only when the node's answer is in; the store, which the switch reads, moves with it", async () => {
    const a = await load("healthy");
    expect(fabric().settings.hub_role).toBe(false);
    const pending = changeSetting(a, apiStore, "hub_role", true);
    expect(fabric().settings.hub_role).toBe(false); // nothing optimistic while the request is out
    expect(toggleOf(html(h(MessagingSettings)), "Act as a hub")).toBe("OFF");
    const answered = await pending;
    expect(answered.hub_role).toBe(true);
    expect(fabric().settings.hub_role).toBe(true);
    expect(toggleOf(html(h(MessagingSettings)), "Act as a hub")).toBe("ON");
  });

  it("each of the three switches writes its own field and nothing else", async () => {
    const a = await load("healthy");
    const was = { ...fabric().settings };
    await changeSetting(a, apiStore, "custody_opt_in", true);
    expect(fabric().settings).toEqual({ ...was, custody_opt_in: true });
    await changeSetting(a, apiStore, "listen_lan", !was.listen_lan);
    expect(fabric().settings).toEqual({ ...was, custody_opt_in: true, listen_lan: !was.listen_lan });
    await changeSetting(a, apiStore, "hub_role", true);
    expect(fabric().settings.hub_role).toBe(true);
    const out = html(h(MessagingSettings));
    expect(toggleOf(out, "Hold mail for others")).toBe("ON");
    expect(toggleOf(out, "LAN listener")).toBe(was.listen_lan ? "OFF" : "ON");
  });

  it("the LAN-listener row is honest that the link between nodes is not encrypted", async () => {
    await load("healthy");
    const out = html(h(MessagingSettings));
    expect(out).toContain("not encrypted in this version");
    expect(out).toContain("not what the messages say");
  });

  it("quotas: saved in bytes by class, cleared with null, and shown in megabytes afterwards", async () => {
    const a = await load("healthy");
    await changeQuotas(a, apiStore, { msg: 50_000_000, kb: 10_000_000 });
    expect(fabric().settings.quotas).toEqual({ msg: 50_000_000, kb: 10_000_000 });
    await changeQuotas(a, apiStore, { kb: null });
    expect(fabric().settings.quotas).toEqual({ msg: 50_000_000 });
    expect(quotaRows(fabric().settings.quotas).find((r) => r.id === "msg")!.mb).toBe("50");
    expect(html(h(MessagingSettings), "?mode=advanced")).toContain('value="50"');
  });

  it("a refusal is the node's and changes nothing", async () => {
    const a = await load("healthy");
    const err = await a.fabric.patchSettings({ hub_role: "yes" as unknown as boolean }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).status).toBe(400);
    expect(fabric().settings.hub_role).toBe(false);
  });
});

describe("over the LAN the loopback-only rows are disabled, like Essentials", () => {
  it("a paired LAN session sees the node's refusal (403 loopback_only) for every loopback-only call", async () => {
    const a = await load("lan-unpaired");
    await a.auth.pair(MOCK_PAIR_CODE);
    await apiStore.refreshAll();
    expect(apiStore.getState().status!.auth.loopback).toBe(false);
    for (const call of [() => changeSetting(a, apiStore, "hub_role", true), () => mintOffer(a, apiStore), () => dismissOffer(a, apiStore), () => changeQuotas(a, apiStore, { msg: 1 })]) {
      const err = await call().catch((e: unknown) => e);
      expect((err as ApiError).code).toBe("loopback_only");
    }
    // enrolling this node with a hub is not loopback-only
    const req = enrollRequest(MOCK_HUB_ADDRESS, MOCK_HUB_CODE, null);
    if (!req.ok) throw new Error(req.text);
    await enrollWithHub(a, apiStore, req.body).catch((e: unknown) => {
      expect((e as ApiError).code).not.toBe("loopback_only");
    });
  });

  it("the page shows the switches and the offer button disabled with the same words as Essentials", async () => {
    const a = await load("lan-unpaired");
    await a.auth.pair(MOCK_PAIR_CODE);
    await apiStore.refreshAll();
    const out = html(h(MessagingSettings));
    expect(out).toContain(LOOPBACK_ONLY_TEXT);
    expect(out.split(LOOPBACK_ONLY_TEXT).length - 1).toBeGreaterThanOrEqual(4); // offer, hub role, LAN listener, custody
    expect(out).toMatch(/<button[^>]*disabled=""[^>]*>[^<]*(<span>Act as a hub<\/span>)/);
    expect(out).toMatch(/<button[^>]*disabled=""[^>]*>Show a one-time code/);
  });

  it("on the node itself nothing is disabled", async () => {
    await load("healthy");
    const out = html(h(MessagingSettings));
    expect(out).not.toContain(LOOPBACK_ONLY_TEXT);
    expect(out).not.toMatch(/<button[^>]*disabled=""[^>]*>Show a one-time code/);
  });
});

// ---------------------------------------------------------------------------------------------- the page

describe("the Messaging & data page", () => {
  it("is what /settings/l5 renders, with the page title and the layer's description", async () => {
    await load("healthy");
    const out = html(routed("/settings/l5", "/settings/:layer", SettingsView));
    expect(out).toContain('data-testid="messaging-settings"');
    expect(out).toContain("Messaging &amp; data");
    expect(out).toContain("Layer 5");
    expect(out).toContain("Your home hub is reachable");
    // the other layer pages keep their structure-only page
    const l3 = html(routed("/settings/l3", "/settings/:layer", SettingsView));
    expect(l3).not.toContain('data-testid="messaging-settings"');
    expect(l3).toContain("Intelligence");
  });

  it("healthy: the enrolled hub, the offer button, the three switches, the gauge, the restore note and the export instruction", async () => {
    await load("healthy");
    const out = html(h(MessagingSettings));
    expect(out).toContain("Workshop hub");
    expect(out).toContain("Home hub reachable");
    expect(out).toContain("Show a one-time code");
    expect(toggleOf(out, "Act as a hub")).toBe("OFF");
    expect(toggleOf(out, "LAN listener")).toBe("ON");
    expect(toggleOf(out, "Hold mail for others")).toBe("OFF");
    expect(out).toContain('role="meter"');
    expect(out).toMatch(/1%/);
    expect(out).toContain("Nothing is waiting on a restore.");
    expect(out).toContain("111 fabric status");
    expect(out).toContain("An encrypted export of your message history");
    expect(out).toContain("It never deletes a message that has not been delivered");
    // no quota editor in simple mode
    expect(out).not.toContain("Save quotas");
    expect(html(h(MessagingSettings), "?mode=advanced")).toContain("Save quotas");
  });

  it("store-full: the gauge is full, the copy is the spec's, and the agent is offered the fix", async () => {
    await load("store-full");
    const out = html(h(MessagingSettings));
    expect(out).toContain('aria-valuenow="100"');
    expect(out).toContain(STORE_FULL_TEXT);
    expect(out).toContain('data-testid="store-full"');
    expect(out).toContain("Ask the agent to free space");
    expect(out).toContain("Ask the agent to fix it");
    expect(out).toContain("ERROR");
  });

  it("restore pending: says the node is waiting and that nothing is sealed or sent until it is done", async () => {
    await load("restore-pending");
    const out = html(h(MessagingSettings));
    expect(out).toContain('data-testid="restore-pending"');
    expect(out).toContain(RESTORE_PENDING_TEXT);
    expect(out).toContain("it must be sure it never reuses a message number");
  });

  it("before the node has reported, it says so instead of showing empty controls", () => {
    const out = html(h(MessagingSettings));
    expect(out).toContain("Messaging is not reported yet");
    expect(out).not.toContain("Show a one-time code");
  });
});

// ------------------------------------------------------------------------------------ the service page

describe("the fabric service page", () => {
  const page = () => html(routed("/services/fabric", "/services/:id", ServicesView));

  it("LIVE: links, the home hub, custody, the store gauge, how delivery is reported (the section 9 sentences), and the way on", async () => {
    await load("healthy");
    const out = page();
    expect(out).toContain('data-testid="fabric-details"');
    expect(out).toContain("Local network (tcp-lan)");
    expect(out).toContain("up to 16 KB per frame, reliable, bulk");
    expect(out).toContain("Home hub reachable: Workshop hub");
    expect(out).toContain("This node is not acting as a hub, so it holds nothing for anyone.");
    expect(out).toContain('role="meter"');
    expect(out).toContain("How delivery is reported");
    expect(out).toContain("In the outbox on this node: there is no path to them yet, their key is missing, or their hub is not yours.");
    expect(out).toContain("A mailbox node saved it and will hold it for them.");
    expect(out).toContain("Their node acknowledged it. Only that acknowledgement says delivered.");
    expect(out).toContain('href="#/messages"');
    expect(out).toContain('href="#/settings/l5"');
    expect(out).toContain("LIVE");
  });

  it("NOT SET with no hub: the setup hand-off, the enroll link, and the links are shown as down", async () => {
    await load("messaging-new");
    const out = page();
    expect(out).toContain("NOT SET");
    expect(out).toContain("Set up with the agent");
    expect(out).toContain("Enroll with a hub");
    expect(out).toContain("down");
  });

  it("ERROR (store full): the fix hand-off, the store-full copy and the gauge", async () => {
    await load("store-full");
    const out = page();
    expect(out).toContain("ERROR");
    expect(out).toContain("Fix with the agent");
    expect(out).toContain(STORE_FULL_TEXT);
  });

  it("the words: link kinds, states, hub name, custody", () => {
    expect(linkLabel("tcp-lan")).toBe("Local network (tcp-lan)");
    expect(linkLabel("kiss-modem")).toBe("LoRa modem (kiss-modem)");
    expect(linkLabel("toString")).toBe("unknown link");
    expect(linkLine({ state: "up", peers: 2, detail: null })).toBe("up · 2 peers");
    expect(linkLine({ state: "down", peers: 0, detail: null })).toBe("down");
    expect(linkLine({ state: "degraded", peers: 0, detail: "the modem stopped answering‮" })).toBe("degraded · the modem stopped answering");
    expect(profileSummary({ mtu_bytes: 255, framing: "datagram", reliable: false, broadcast: true, cost_class: "low_bandwidth", airtime: { duty_cycle_permille: 10, bits_per_s: 5470 } }, true)).toBe(
      "up to 255 B per frame, best effort, low bandwidth, broadcast, duty cycle 1.0%, 5470 bit/s",
    );
    expect(hubName({ node_id: null }, [])).toBeNull();
    expect(hubName({ node_id: "ab".repeat(32) }, [])).toBe("abababab…abab");
    expect(custodyText({ count: 1, bytes: 1500 }, true)).toBe("Holding 1 sealed item (1.5 KB) for the nodes that use this one as their hub. They are encrypted for their owners; this node cannot read them.");
    expect(custodyText({ count: 0, bytes: 0 }, true)).toBe("Holding nothing for anyone right now.");
  });
});
