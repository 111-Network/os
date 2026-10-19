// Settings -> Messaging & data (os/50-messaging/README.md sections 7.3, 9, 10, 16): what is typed and scanned (parsed by the page, judged
// by the node), the calls the page makes and what it shows afterwards, the loopback-only rows over the LAN, and the fabric service
// page. Driven through the real store against the mock adapter's `messaging-new`, `healthy`, `store-full`, `restore-pending` and
// `lan-unpaired` scenarios.
import { createElement as h, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import { createMockAdapter, MOCK_PAIR_CODE, type MockAdapter, type ScenarioName } from "../src/lib/api/mock";
import { buildFabric, HUB, hubBundleText, MOCK_HUB_ADDRESS, MOCK_HUB_CODE, MOCK_QUOTA_CLASSES, mockCardText } from "../src/lib/api/mock/messaging";
import { ApiError } from "../src/lib/api/problem";
import { apiStore } from "../src/lib/api/store";
import type { FabricStatus, HubOffer } from "../src/lib/api/types";
import { ModeProvider, resetModeForTests } from "../src/lib/mode";
import ServicesView from "../src/views/services";
import { hubSummary, restoreFloor } from "../src/lib/api/fabric";
import { custodyText, hubLine, hubName, linkLabel, linkLine, profileSummary, STORE_FULL_TEXT, STORE_PRUNE_TEXT } from "../src/views/services/fabric-model";
import { HubLine } from "../src/views/services/fabric";
import { MAX_NAME_CHARS } from "../src/views/nearby/model";
import { changeQuotas, changeSetting, dismissOffer, enrollWithHub, mintOffer } from "../src/views/settings/messaging-actions";
import {
  ADDRESS_EDITED_NOTE,
  applyEnrollText,
  CODE_LENGTH,
  codeComplete,
  editEnrollAddress,
  enrollErrorText,
  enrollRequest,
  fabricAgent,
  formatCode,
  normalizeCode,
  offerAddressText,
  offerCodeText,
  offerGone,
  offerStep,
  OFFER_GONE_TEXT,
  parseAddress,
  parseEnrollBundle,
  parseQuota,
  QUOTA_LABELS,
  quotaPatch,
  quotaRows,
  restorePending,
  RESTORE_CLEAR_TEXT,
  RESTORE_HELP,
  RESTORE_PENDING_TEXT,
  RESTORE_UNKNOWN_TEXT,
  restoreState,
  type EnrollFields,
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

  // Generated by the crate itself: mesh111::contacts::{card_text, HubBundle::text} over an id111 announce (see os/50-messaging/
  // crates/mesh111/src/contacts.rs), not typed by hand. A scratch crate depending on mesh111 printed these.
  const CRATE_CARD = "AEYDB2AEQXUI2NZIC7WVLM755UOSJMUXPVPP2V3VGRHEEVBWNS6VWUB5ZNBYWHHQDSBDDKUUMTD37GAKARL7OLECPID2EMOOE4SNTQQSAMAAAAAAAAAAAAIBAAAASCIJBEEQSCIJX47LRFDFWOT6QGZS3HPONHXC46M3IUGPW6PWBKJYAHG5EEEUOCDFXXH6K3T7BEXY723VJPNYRQU2YIJYQT7E47XUP45IMLVP3GNT6CA";
  const CRATE_V4 = `111HUB:${CRATE_CARD}:K7M29X4P:192.168.1.20:11183`;
  const CRATE_V6 = `111HUB:${CRATE_CARD}:0123BCDF:[fe80::1]:11183`;
  const CRATE_NO_ADDRESS = `111HUB:${CRATE_CARD}:K7M29X4P:`;

  it("reads the crate's own HubBundle::text() output (IPv4, IPv6, and a hub with no LAN address)", () => {
    expect(CRATE_CARD).toHaveLength(239); // 149-byte announce (73 head + 12 appdata + 64 signature), unpadded base32
    expect(parseEnrollBundle(CRATE_V4)).toEqual({ card: CRATE_CARD, code: "K7M29X4P", address: "192.168.1.20:11183" });
    expect(parseEnrollBundle(CRATE_V6)).toEqual({ card: CRATE_CARD, code: "0123BCDF", address: "[fe80::1]:11183" });
    expect(parseEnrollBundle(CRATE_NO_ADDRESS)).toEqual({ card: CRATE_CARD, code: "K7M29X4P", address: "" });
  });

  it("the mock hub's enrollment text is the daemon's shape, and the page reads it back into the same code and address", () => {
    expect(hubBundleText("CARD", "K7M29X4P", "192.168.1.20:11183")).toBe("111HUB:CARD:K7M29X4P:192.168.1.20:11183");
    expect(hubBundleText(CRATE_CARD, "K7M29X4P", "192.168.1.20:11183")).toBe(CRATE_V4);
    const text = hubBundleText(mockCardText(HUB), "HUB00001", "192.168.1.42:11183");
    expect(text.startsWith("111HUB:")).toBe(true);
    expect(parseEnrollBundle(text)).toEqual({ card: mockCardText(HUB), code: "HUB00001", address: "192.168.1.42:11183" });
  });

  it("a bare code drops the hub card an earlier text filled in; a whole text replaces it", () => {
    const fresh: EnrollFields = { address: "", code: "", card: null };
    const a = applyEnrollText(CRATE_V4, fresh);
    expect(a.fields).toEqual({ address: "192.168.1.20:11183", code: "K7M2-9X4P", card: CRATE_CARD });
    expect(a.read).toMatch(/Read the hub's address and code/);
    // the person then pastes a code for another hub: the card of the first hub must not ride along
    const b = applyEnrollText("ABCD-2345", a.fields);
    expect(b.fields).toEqual({ address: "192.168.1.20:11183", code: "ABCD-2345", card: null });
    expect(b.read).toMatch(/Read a code/);
    expect(enrollRequest(b.fields.address, b.fields.code, b.fields.card)).toEqual({ ok: true, body: { address: "192.168.1.20:11183", code: "ABCD2345" } });
    // a bare code with nothing before it
    expect(applyEnrollText("k7m29x4p", fresh).fields).toEqual({ address: "", code: "K7M2-9X4P", card: null });
    // text that is neither changes nothing and says so
    const c = applyEnrollText("hello", a.fields);
    expect(c.fields).toBe(a.fields);
    expect(c.read).toMatch(/not a hub's enrollment text/);
  });

  it("a second whole text for another hub replaces card, code and address, and never keeps the first hub's address", () => {
    const first = applyEnrollText(CRATE_V4, { address: "", code: "", card: null }).fields;
    const otherCard = CRATE_CARD.replace(/^AEYD/, "BFYD");
    const second = applyEnrollText(`111HUB:${otherCard}:ABCD2345:`, first); // the other hub does not know its address yet
    expect(second.fields).toEqual({ address: "", code: "ABCD-2345", card: otherCard });
    expect(second.read).toMatch(/did not know its network address/);
    // the same hub again with no address keeps what the person typed
    const typed: EnrollFields = { address: "10.0.0.9:11183", code: "", card: CRATE_CARD };
    expect(applyEnrollText(CRATE_NO_ADDRESS, typed).fields.address).toBe("10.0.0.9:11183");
  });

  it("editing the address after a whole text drops the hub's card and says so; before one it just edits", () => {
    const filled = applyEnrollText(CRATE_V4, { address: "", code: "", card: null }).fields;
    const edited = editEnrollAddress(filled, "192.168.1.21:11183");
    expect(edited.fields).toEqual({ address: "192.168.1.21:11183", code: "K7M2-9X4P", card: null });
    expect(edited.read).toBe(ADDRESS_EDITED_NOTE);
    expect(enrollRequest(edited.fields.address, edited.fields.code, edited.fields.card)).toEqual({ ok: true, body: { address: "192.168.1.21:11183", code: "K7M29X4P" } });
    const plain = editEnrollAddress({ address: "", code: "", card: null }, "192.168.1");
    expect(plain).toEqual({ fields: { address: "192.168.1", code: "", card: null }, read: null });
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
  it("one row per class the node reports, in plain words, the daemon's six first; a null class has an empty box", () => {
    // the daemon's `settings.quotas`: every refusable class (kb, update, patch, sync, object, custody), null when unset
    const rows = quotaRows({ custody: null, sync: 2_000_000, object: null, patch: null, update: null, kb: 50_000_000 });
    expect(rows.map((r) => [r.id, r.label, r.mb])).toEqual([
      ["kb", "Knowledge base", "50"],
      ["update", "Updates", ""],
      ["patch", "Patches", ""],
      ["sync", "Synced operations", "2"],
      ["object", "Files and attachments", ""],
      ["custody", "Mail held for others", ""],
    ]);
    expect(Object.keys(QUOTA_LABELS)).toEqual(["kb", "update", "patch", "sync", "object", "custody"]);
  });
  it("never offers a class the node did not report: not msg or agent_op (the node refuses them), not an empty set", () => {
    expect(quotaRows({})).toEqual([]);
    expect(quotaRows({ kb: null }).map((r) => r.id)).toEqual(["kb"]);
    const ids = quotaRows({ kb: null, sync: 1 }).map((r) => r.id);
    expect(ids).not.toContain("msg");
    expect(ids).not.toContain("agent_op");
  });
  it("a class this page has no word for keeps the node's own name, cleaned and cut", () => {
    const rows = quotaRows({ kb: null, extra: 1_500_000, ["bad\u202ename" + "x".repeat(80)]: null });
    expect(rows.map((r) => r.id).slice(0, 2)).toEqual(["kb", "extra"]);
    expect(rows[1]).toMatchObject({ label: "extra", mb: "1.5" });
    expect(rows[2]!.label).not.toContain("\u202e");
    expect(rows[2]!.label.length).toBeLessThanOrEqual(40);
  });
  it("the mock reports the same six classes, all unset", () => {
    expect(Object.keys(buildFabric().settings.quotas)).toEqual([...MOCK_QUOTA_CLASSES]);
    expect(Object.values(buildFabric().settings.quotas).every((v) => v === null)).toBe(true);
  });
  it("a box is a number of megabytes or empty for none", () => {
    expect(parseQuota("")).toEqual({ ok: true, bytes: null });
    expect(parseQuota("50")).toEqual({ ok: true, bytes: 50_000_000 });
    expect(parseQuota(" 0.5 ")).toEqual({ ok: true, bytes: 500_000 });
    for (const bad of ["-1", "1e3", "abc", "5 MB", "1,5"]) expect(parseQuota(bad), bad).toMatchObject({ ok: false });
    expect(parseQuota("9".repeat(30))).toMatchObject({ ok: false });
  });
  it("only changed classes go to the node; clearing a box drops its quota", () => {
    const current = { sync: 50_000_000, kb: null };
    expect(quotaPatch(current, { sync: "50", kb: "" })).toEqual({ ok: true, quotas: null });
    expect(quotaPatch(current, { sync: "", kb: "10" })).toEqual({ ok: true, quotas: { sync: null, kb: 10_000_000 } });
    expect(quotaPatch(current, { sync: "x" })).toMatchObject({ ok: false });
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
  it("the store's own report is the answer when the node gives one: true is pending, false is clear, absent is 'not reported'", () => {
    const svc = { reason_code: "hub_reachable" };
    expect(restoreFloor({ store: { restore_pending: true } })).toBe(true);
    expect(restoreFloor({ store: { restore_pending: false } })).toBe(false);
    expect(restoreFloor({ store: {} })).toBeNull();
    expect(restoreFloor({ store: { restore_pending: "yes" } })).toBeNull();
    expect(restoreFloor(null)).toBeNull();
    // the daemon refuses a send while the floor is up and queues nothing, so a node restored from its words has no queued message
    // to find: only the store's report can say so
    expect(restoreState(svc, [], { store: { restore_pending: true } })).toBe("pending");
    expect(restoreState(svc, [], { store: { restore_pending: false } })).toBe("clear");
    expect(restoreState(svc, [], { store: {} })).toBe("unknown");
    expect(restoreState(svc, [], null)).toBe("unknown");
    expect(restoreState(svc, null, undefined)).toBe("unknown");
    // the older signals still say pending, and win over a report of "no" (an outbox message is waiting on it)
    expect(restoreState({ reason_code: "restore_pending" }, [], { store: { restore_pending: false } })).toBe("pending");
  });
  it("the copy follows the §15 as-built: new messages are refused, ones in the outbox wait", () => {
    expect(RESTORE_HELP).toMatch(/refuses new messages/);
    expect(RESTORE_HELP).toMatch(/already in the outbox wait there, queued/);
    expect(RESTORE_HELP).not.toMatch(/Messages you write meanwhile stay queued/);
    expect(RESTORE_PENDING_TEXT).toMatch(/New messages are refused until then/);
    expect(RESTORE_PENDING_TEXT).not.toMatch(/what you write waits in the outbox/);
  });
  it("the page says 'not reported' rather than 'nothing is waiting' when the node does not say, and says pending from the report alone", async () => {
    await load("healthy");
    const was = apiStore.getState().fabric!;
    // the mock reports restore_pending: false, as the regenerated contract's daemon will
    expect(html(h(MessagingSettings))).toContain(RESTORE_CLEAR_TEXT);
    const without = { ...was, store: Object.fromEntries(Object.entries(was.store).filter(([k]) => k !== "restore_pending")) } as unknown as FabricStatus;
    apiStore.detach();
    apiStore.reset();
    const a = await load("healthy");
    const real = a.fabric.status.bind(a.fabric);
    a.fabric.status = async () => ({ ...(await real()), store: without.store });
    await apiStore.refreshFabric();
    const unknown = html(h(MessagingSettings));
    expect(unknown).toContain(RESTORE_UNKNOWN_TEXT);
    expect(unknown).not.toContain(RESTORE_CLEAR_TEXT);
    expect(unknown).not.toContain('data-testid="restore-pending"');
    const pending = { ...was, store: { ...was.store, restore_pending: true } } as unknown as FabricStatus;
    a.fabric.status = async () => pending;
    await apiStore.refreshFabric();
    const out = html(h(MessagingSettings));
    expect(out).toContain('data-testid="restore-pending"');
    expect(out).toContain(RESTORE_PENDING_TEXT);
    expect(out).not.toContain(RESTORE_CLEAR_TEXT);
  });
  it("offers a fix for a full or closed store, an error and an unreachable hub; set-up when NOT SET; nothing special when it works", async () => {
    await load("healthy");
    const f = fabric();
    const full = { ...f, store: { ...f.store, store_full: true } };
    expect(fabricAgent({ state: "live", reason_code: "hub_reachable" }, full)).toMatchObject({ goal: "messaging.fix" });
    expect(fabricAgent({ state: "live", reason_code: "hub_reachable" }, { ...f, store: { ...f.store, open: false } })).toMatchObject({ goal: "messaging.fix" });
    expect(fabricAgent({ state: "error", reason_code: "store_failed" }, f)).toMatchObject({ goal: "messaging.fix" });
    expect(fabricAgent({ state: "unset", reason_code: "hub_unreachable" }, f)).toMatchObject({ goal: "messaging.fix" });
    // connected to the home hub, no answer yet: a quiet check, never a set-up for a node that is already enrolled
    expect(fabricAgent({ state: "unset", reason_code: "hub_waiting_answer" }, f)).toEqual({ goal: "messaging.fix", label: "Ask the agent to check the hub", primary: false });
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

  it("a code on screen is dead once the node stops holding one: only after the page saw it held", () => {
    expect(OFFER_GONE_TEXT).toBe("The code was used or dismissed.");
    expect(offerGone(true, false)).toBe(true); // a member enrolled with it, or it was dismissed from the CLI
    expect(offerGone(true, true)).toBe(false); // still held
    expect(offerGone(false, false)).toBe(false); // never seen held (the refresh after the mint has not landed): keep showing it
    expect(offerGone(false, true)).toBe(false);
  });

  it("the watch: a shown code is cleared, and the person told, when the node stops holding it; not when they dismissed it themselves", () => {
    // minted: the node holds it (the refresh after the mint landed), the page remembers having seen that
    let st = offerStep(true, false, true, false);
    expect(st).toEqual({ seen: true, clear: false, say: false });
    // a member enrolls with the code, or `111 fabric offer` replaced/dismissed it: offer_active turns false
    st = offerStep(true, st.seen, false, false);
    expect(st).toEqual({ seen: false, clear: true, say: true });
    // the person pressed "Dismiss the code" here: the code goes, with no "used or dismissed" notice (they know)
    expect(offerStep(true, true, false, true)).toEqual({ seen: false, clear: true, say: false });
    // the refresh after a mint has not landed yet: the code is not cleared
    expect(offerStep(true, false, false, false)).toEqual({ seen: false, clear: false, say: false });
    // nothing on screen: nothing to do, and the memory resets
    expect(offerStep(false, true, false, false)).toEqual({ seen: false, clear: false, say: false });
    expect(offerStep(false, false, true, false)).toEqual({ seen: false, clear: false, say: false });
    // "Show a new code" while one is held keeps it held throughout
    expect(offerStep(true, true, true, false)).toEqual({ seen: true, clear: false, say: false });
  });

  it("the node's own view: minted, held, then used up (the mock's enroll clears the offer) - offer_active turns false", async () => {
    const a = await load("healthy");
    await mintOffer(a, apiStore);
    expect(fabric().hub.offer_active).toBe(true);
    expect(offerGone(true, fabric().hub.offer_active)).toBe(false);
    await dismissOffer(a, apiStore); // the same transition the CLI's dismiss or a used code makes in the fabric frame
    expect(fabric().hub.offer_active).toBe(false);
    expect(offerGone(true, fabric().hub.offer_active)).toBe(true);
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
    await changeQuotas(a, apiStore, { sync: 50_000_000, kb: 10_000_000 });
    expect(fabric().settings.quotas).toMatchObject({ sync: 50_000_000, kb: 10_000_000 });
    await changeQuotas(a, apiStore, { kb: null });
    expect(fabric().settings.quotas.kb ?? null).toBeNull(); // the daemon keeps the key with null; the mock may drop it
    expect(fabric().settings.quotas.sync).toBe(50_000_000);
    expect(quotaRows(fabric().settings.quotas).find((r) => r.id === "sync")!.mb).toBe("50");
    const adv = html(h(MessagingSettings), "?mode=advanced");
    expect(adv).toContain('value="50"');
    expect(adv).toContain("Synced operations");
    expect(adv).not.toContain("Agent operations"); // a class the node refuses is not offered
  });

  it("the quota editor offers the node's own classes with plain labels, and nothing the node would refuse", async () => {
    await load("healthy");
    const adv = html(h(MessagingSettings), "?mode=advanced");
    for (const label of ["Knowledge base", "Updates", "Patches", "Synced operations", "Files and attachments", "Mail held for others"]) expect(adv, label).toContain(label);
    for (const gone of ["Messages</span>", "Agent operations", ">msg<", "agent_op"]) expect(adv, gone).not.toContain(gone);
    expect(adv).toContain("Save quotas");
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
    expect(out).toContain('Home hub reachable: <span dir="auto" class="[unicode-bidi:isolate] break-words">Workshop hub</span>');
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


// ------------------------------------------------------------------------------- the home-hub phrase

describe("the home-hub phrase tells reachable, connected and waiting, and down apart", () => {
  const phrase = (over: Partial<FabricStatus["hub"]>) =>
    hubSummary({ enrolled: true, connected: false, reachable: false, members: 0, offer_active: false, ...over });

  it("an enrolled member: answered, connected without an answer, neither", () => {
    expect(phrase({ connected: true, reachable: true })).toBe("Home hub reachable");
    expect(phrase({ connected: true, reachable: false })).toBe("Home hub enrolled, connected, waiting for its answer");
    expect(phrase({ connected: false, reachable: false })).toBe("Home hub enrolled, not reachable right now");
  });

  it("an answered hub reads reachable even if the connection flag lags", () => {
    expect(phrase({ connected: false, reachable: true })).toBe("Home hub reachable");
  });

  it("a node that is not enrolled never reads as waiting on a hub", () => {
    expect(phrase({ enrolled: false, connected: true, members: 2 })).toBe("Home hub for 2 members");
    expect(phrase({ enrolled: false, connected: true, offer_active: true })).toBe("Waiting for a node to enroll");
    expect(phrase({ enrolled: false })).toBe("No home hub");
  });

  it("the Services line keeps the hub's name after the waiting phrase", () => {
    const hub: FabricStatus["hub"] = { enrolled: true, connected: true, reachable: false, cursor: 0, node_id: HUB, address: null, members: 0, offer_active: false };
    expect(hubLine(hub, [])).toMatch(/^Home hub enrolled, connected, waiting for its answer: /);
  });
});

// ------------------------------------------------------------------------------- peer-authored hub name

describe("the home hub's name is peer text: cleaned, cut, and isolated from the words around it", () => {
  const hub = (over: Partial<FabricStatus["hub"]> = {}): FabricStatus["hub"] => ({ enrolled: true, connected: true, reachable: true, cursor: 1, node_id: HUB, address: null, members: 0, offer_active: false, ...over });
  const contact = (label: string, alias: string | null = null) => [{ node_id: HUB, bech32: "node1x", label, alias, trust: "enrolled" as const, request: false, pinned: false, has_key: true, roles: [], home_hub: null }];

  it("hubName flattens, strips direction overrides and cuts a long label", () => {
    const hostile = `evil\u202e name\n${"x".repeat(200)}`;
    const name = hubName({ node_id: HUB }, contact(hostile))!;
    expect(name).not.toContain("\u202e");
    expect(name).not.toContain("\n");
    expect(Array.from(name).length).toBeLessThanOrEqual(MAX_NAME_CHARS);
    expect(name.endsWith("…")).toBe(true);
    // an alias is the person's own, but goes through the same cut
    expect(hubName({ node_id: HUB }, contact("zzzz-zzzz-zz", "Workshop hub"))).toBe("Workshop hub");
    // emoji are cut by code point, never in half
    const emoji = hubName({ node_id: HUB }, contact("😀".repeat(100)))!;
    expect(emoji).not.toMatch(/[\ud800-\udfff]/u); // /u: only a lone surrogate matches
  });

  it("the page draws it in a dir=auto, bidi-isolated, wrapping span after a fixed summary", () => {
    const rtl = "שלום עולם ".repeat(10);
    const out = renderToStaticMarkup(h(HubLine, { hub: hub(), contacts: contact(rtl) as never }));
    expect(out.startsWith("Home hub reachable: <span dir=\"auto\" class=\"[unicode-bidi:isolate] break-words\">")).toBe(true);
    expect(out).toContain("שלום");
    expect(out.endsWith("</span>")).toBe(true);
    expect(out.match(/<span /g)).toHaveLength(1);
    // not enrolled: just the summary, no name
    expect(renderToStaticMarkup(h(HubLine, { hub: hub({ enrolled: false, reachable: false }), contacts: contact("x") as never }))).toBe("No home hub");
    // the plain-text form (hubLine) is cleaned and cut too
    expect(hubLine(hub(), contact(`a\u202eb${"y".repeat(100)}`) as never).length).toBeLessThan(80);
  });

  it("the settings page draws it the same way", async () => {
    await load("healthy");
    const out = html(h(MessagingSettings));
    expect(out).toContain('<span dir="auto" class="[unicode-bidi:isolate] break-words">Workshop hub</span>');
  });
});

// ----------------------------------------------------------------------------------------- §10.4 (4) copy

describe("the prune and store-full copy says all of §10.4 (4)", () => {
  it("never deletes undelivered messages, mail held for members, pending requests, pinned items, or contacts", () => {
    for (const text of [STORE_PRUNE_TEXT, STORE_FULL_TEXT]) {
      expect(text).toMatch(/message/);
      expect(text).toMatch(/mail/);
      expect(text).toMatch(/pending request/);
      expect(text).toMatch(/pinned item/);
      expect(text).toMatch(/contacts?\b/);
    }
    // it must not suggest that only a pinned contact is safe
    expect(STORE_PRUNE_TEXT).not.toMatch(/pinned contact/);
    expect(STORE_FULL_TEXT).not.toMatch(/pinned contact/);
    expect(STORE_PRUNE_TEXT).toMatch(/a pinned item or a contact/);
    expect(STORE_FULL_TEXT).toMatch(/pinned items and contacts all stay/);
  });
});
