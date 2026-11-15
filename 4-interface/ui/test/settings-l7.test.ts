// Settings, Relay & presence (os/7-relay/README.md §12; task U2): the consent round trip of every Layer 7 action on the mock, no secret in
// the page, the heartbeat's 409 copy shown as the node wrote it, the pairing bundle and the complement rule, the origin order and the
// newest release seen, the metered list, and the chunk (its own lazy chunk, at most 64 KB gzip). Plus the three follow-ups: the gateway
// row's link to this page, the hub scenario seeding GET /relay, and the consent tuple of bridge.ts. Driven through the real store against
// the mock adapter.
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { gzipSync } from "node:zlib";
import { createElement as h, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import { ApiError } from "../src/lib/api/problem";
import { readConsent, consentOf } from "../src/lib/api/bridge";
import { createMockAdapter, type MockAdapter, type ScenarioName } from "../src/lib/api/mock";
import { carrierConsentText, createRelayMock, GATEWAY_BIT_TEXT, ORIGIN_SERVE_TEXT, PAIR_PLAIN_TEXT } from "../src/lib/api/mock/relay";
import { sha256Hex } from "../src/lib/api/mock/sha256";
import { relayConsentOf, type RelayAdapter } from "../src/lib/api/relay";
import { apiStore } from "../src/lib/api/store";
import { CONSENT_ACTIONS, type ConsentRequired, type RelaySettingsPatch, type RelayStatus } from "../src/lib/api/types";
import type { Proposal } from "../src/lib/api/sse.gen";
import { ModeProvider, resetModeForTests } from "../src/lib/mode";
import { acceptRunsConsent, decideProposal, ProposalCards } from "../src/views/agent/proposals";
import { actionWord, runConsent } from "../src/views/messages/net/consent";
import ServicesView from "../src/views/services";
import { pairFacts, pairLinkState } from "../src/views/services/fabric-model";
import { RelaySettings } from "../src/views/settings/l7";
import { dismissOffer } from "../src/views/settings/messaging-actions";
import { OUTSTANDING_NOTE } from "../src/views/settings/messaging-model";
import { IDLE, pairWith, RelayCallSeed, runWrite, WRITES, type CallState, type Write, type WriteDeps, type WriteOpts } from "../src/views/settings/l7/actions";
import { DiscoverySection } from "../src/views/settings/l7/discovery";
import { GatewaySection, SecretInput } from "../src/views/settings/l7/gateway";
import { MeteredSection } from "../src/views/settings/l7/metered";
import { PairingSection } from "../src/views/settings/l7/pairing";
import { HeartbeatLine, HEARTBEAT_OPTS, PresenceSection } from "../src/views/settings/l7/presence";
import { UpdatesSection } from "../src/views/settings/l7/updates";
import {
  answerText,
  bundleSummary,
  complementOf,
  EMPTY_SECRET,
  heartbeatRows,
  isPlainHttp,
  mismatchNote,
  newestSeen,
  newestSeenText,
  offerNote,
  OFFER_EARLIER_NOTE,
  originName,
  parseCapMb,
  parsePairBundle,
  PATCH,
  pickForBundle,
  problemDetail,
  settlePick,
  TOKEN_CLEAR_NOTE,
  typeSecret,
  UPDATE_FROM_COMMAND,
  withMetered,
  withoutMetered,
} from "../src/views/settings/l7/model";
import { FakeClock, flush } from "./helpers/fakes";
import { routed } from "./helpers/routed";
import { validateAgainst } from "./helpers/validate";

const NOW = Date.parse("2026-11-11T09:00:00Z");
const SERVER = "https://ntfy.example.org";
const SECRET_TOKEN = "tk_secret_9f3a77c1";
const SECRET_TOPIC = "TOPICSECRETABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

const made: MockAdapter[] = [];
afterEach(() => {
  apiStore.detach();
  apiStore.reset();
  for (const a of made.splice(0)) a.mock.dispose();
});

async function load(scenario: ScenarioName = "hub"): Promise<MockAdapter> {
  const a = createMockAdapter({ scenario, clock: new FakeClock(NOW), tokenDelayMs: 0 });
  made.push(a);
  apiStore.attach(a);
  apiStore.attachRelay(async () => a.mock.relay);
  await flush(30);
  await apiStore.refreshRelay();
  return a;
}

const html = (el: ReactElement, search = ""): string => {
  const g = globalThis as { window?: unknown };
  const had = g.window;
  g.window = {
    location: { search, hash: "", pathname: "/", protocol: "http:", hostname: "127.0.0.1", port: "11180" },
    history: { state: null, replaceState: () => undefined },
    localStorage: { getItem: () => null, setItem: () => undefined },
  };
  resetModeForTests();
  try {
    return renderToStaticMarkup(h(ModeProvider, null, el));
  } finally {
    if (had === undefined) delete g.window;
    else g.window = had;
    resetModeForTests();
  }
};

const page = () => html(h(RelaySettings));

/** The consent round trip the page runs, with the person's answers scripted: `asked` collects every text shown. */
function person(answers: boolean[] = [true]) {
  const asked: ConsentRequired[] = [];
  const ask = async (c: ConsentRequired): Promise<boolean> => {
    asked.push(c);
    return answers.shift() ?? false;
  };
  return { asked, ask };
}

/** The write `useRelayCall().patch` makes, without React: the hash goes to the adapter call. */
const NO_DEPS = {} as WriteDeps;
const patchSettings = (relay: RelayAdapter, patch: RelaySettingsPatch, hash?: string): Promise<RelayStatus> => WRITES.patch(patch)(relay, hash, NO_DEPS);

/**
 * `runWrite` (the helper behind every control of the page) with the person's answers scripted and the dependencies the hook would give it:
 * `asked` is every text the dialog showed (and whether it was shown again because it changed), `states` every state the control went through.
 */
function harness(a: MockAdapter, answers: boolean[] = [true], onAsk?: (c: ConsentRequired) => Promise<void>) {
  const asked: { consent: ConsentRequired; changed: boolean }[] = [];
  const states: CallState[] = [];
  const seen = { refreshRelay: 0, refreshFabric: 0 };
  const deps: WriteDeps = {
    attempt: (call) =>
      runConsent(
        (hash) => call(hash),
        async (consent, changed) => {
          asked.push({ consent, changed });
          await onAsk?.(consent);
          return answers.shift() ?? false;
        },
      ),
    relay: async () => a.mock.relay,
    set: (st) => {
      states.push(st);
    },
    refreshRelay: () => {
      seen.refreshRelay++;
    },
    refreshFabric: () => {
      seen.refreshFabric++;
    },
    dismissOffer: () => dismissOffer(a, apiStore),
    unauthenticated: () => false,
  };
  const go = <T,>(write: Write<T>, opts?: WriteOpts<T>) => runWrite(deps, write, opts);
  return { deps, asked, states, seen, go, end: () => states[states.length - 1]! };
}

const esc = (t: string): string => t.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#x27;");
const count = (out: string, needle: string): number => out.split(needle).length - 1;
const PAIR_BUNDLE = (role: string, via: string, address = "192.168.1.31:11183", ch = "c") => `111PAIR:${ch.repeat(224)}:K7M29X4P:${"s".repeat(71)}:${address}:${role}:${via}`;

const fail = async (p: Promise<unknown>): Promise<ApiError> => {
  const e = await p.then(() => null, (x: unknown) => x);
  expect(e).toBeInstanceOf(ApiError);
  return e as ApiError;
};

const NTFY = { carrier: { kind: "ntfy" as const, server: SERVER, mode: "5A" as const } };

// ----------------------------------------------------------------------------------------------------- the consent round trip

describe("every Layer 7 consent is the pinned round trip (section 12)", () => {
  it("carrier_enable: the node's text is shown verbatim, the call repeats with its hash, the hash is on record", async () => {
    const a = await load("healthy");
    const r = a.mock.relay;
    const p = person();
    const out = await runConsent((hash) => patchSettings(r, NTFY, hash), p.ask);
    expect(out.ok).toBe(true);
    expect(p.asked).toHaveLength(1);
    expect(p.asked[0]!.action).toBe("carrier_enable");
    expect(p.asked[0]!.subject).toBe(`ntfy:${SERVER}`);
    expect(p.asked[0]!.warning_text).toBe(carrierConsentText(SERVER, "the other hub"));
    expect(p.asked[0]!.hash).toBe(sha256Hex(p.asked[0]!.warning_text));
    expect((await r.status()).carrier).toMatchObject({ kind: "ntfy", server: SERVER, consent_hash: p.asked[0]!.hash });
  });

  it("a person who says no changes nothing", async () => {
    const a = await load("healthy");
    const r = a.mock.relay;
    const p = person([false]);
    const out = await runConsent((hash) => patchSettings(r, NTFY, hash), p.ask);
    expect(out).toMatchObject({ ok: false, cancelled: true });
    expect((await r.status()).carrier).toMatchObject({ kind: "none", consent_hash: null });
  });

  it("raising a metered cap while the carrier is on asks again with `metered` in the add-ons and the cap in the words", async () => {
    const a = await load("healthy");
    const r = a.mock.relay;
    await runConsent((hash) => patchSettings(r, NTFY, hash), person().ask);
    const next = withMetered([], "ssid", "Home hotspot", "5");
    expect(next.ok).toBe(true);
    if (!next.ok) return;
    const p = person();
    const out = await runConsent((hash) => patchSettings(r, { metered: next.list }, hash), p.ask);
    expect(out.ok).toBe(true);
    expect(p.asked[0]!.addons).toEqual(["metered"]);
    expect(p.asked[0]!.warning_text).toContain("5000000 bytes");
    expect((await r.status()).metered).toEqual([{ match: { ssid: "Home hotspot" }, daily_cap_bytes: 5_000_000 }]);
  });

  it("gateway_bit and origin_serve show the spec's words and turn on only after the repeat", async () => {
    const a = await load("hub");
    const r = a.mock.relay;
    const bit = person();
    expect((await runConsent((hash) => patchSettings(r, { discovery: { public_gateway_bit: true } }, hash), bit.ask)).ok).toBe(true);
    expect(bit.asked[0]).toMatchObject({ action: "gateway_bit", warning_text: GATEWAY_BIT_TEXT });
    const serve = person();
    expect((await runConsent((hash) => patchSettings(r, { origin: { serve: true } }, hash), serve.ask)).ok).toBe(true);
    expect(serve.asked[0]).toMatchObject({ action: "origin_serve", warning_text: ORIGIN_SERVE_TEXT });
    const after = await r.status();
    expect(after.discovery.public_gateway_bit).toBe(true);
    expect(after.origin.serve).toBe(true);
    // already on, nothing to ask: switching it off is not a consent
    const off = person();
    expect((await runConsent((hash) => patchSettings(r, { discovery: { public_gateway_bit: false } }, hash), off.ask)).ok).toBe(true);
    expect(off.asked).toHaveLength(0);
  });

  it("pair_plain: a hub on the open internet asks, the repeat carries the hash, and the pair reads plain (warned)", async () => {
    const a = await load("hub");
    const r = a.mock.relay;
    const bundle = `111PAIR:${"c".repeat(224)}:K7M29X4P:${"s".repeat(71)}:203.0.113.7:11183:peer:tcp`;
    const p = person();
    const out = await runConsent((hash) => pairWith(r, { bundle, role: "peer", via: "tcp", lan: false }, hash), p.ask);
    expect(out.ok).toBe(true);
    expect(p.asked[0]).toMatchObject({ action: "pair_plain", warning_text: PAIR_PLAIN_TEXT });
    const pair = (await r.status()).paired_hubs[0]!;
    expect(pair).toMatchObject({ tls: "plain", lan: false, role: "peer", via: "tcp" });
    expect(pairLinkState(pair)).toBe("plain (warned)");
    expect(pairFacts(pair)).toContain("plain (warned)");
  });

  it("a private address, or a link marked lan, pairs with no question; lan reads `lan`", async () => {
    const a = await load("hub");
    const r = a.mock.relay;
    const priv = person();
    await runConsent((hash) => pairWith(r, { bundle: `111PAIR:${"d".repeat(224)}:K7M29X4P:${"s".repeat(71)}:192.168.1.31:11183:peer:tcp`, role: "peer", via: "tcp", lan: false }, hash), priv.ask);
    const marked = person();
    await runConsent((hash) => pairWith(r, { bundle: `111PAIR:${"e".repeat(224)}:K7M29X4P:${"s".repeat(71)}:203.0.113.9:11183:peer:tcp`, role: "peer", via: "tcp", lan: true }, hash), marked.ask);
    expect(priv.asked).toHaveLength(0);
    expect(marked.asked).toHaveLength(0);
    const pairs = (await r.status()).paired_hubs;
    expect(pairs).toHaveLength(2);
    expect(pairs[0]).toMatchObject({ lan: false, tls: "none" });
    expect(pairs[1]).toMatchObject({ lan: true, tls: "none" });
    expect(pairLinkState(pairs[1]!)).toBe("lan");
  });

  it("a wrong pick is refused by the node (pair_role_mismatch) and nothing is recorded", async () => {
    const a = await load("hub");
    const r = a.mock.relay;
    const bundle = `111PAIR:${"f".repeat(224)}:K7M29X4P:${"s".repeat(71)}:192.168.1.31:11183:leaf:tcp`;
    const err = await fail(pairWith(r, { bundle, role: "leaf", via: "tcp", lan: false }));
    expect([err.status, err.code]).toEqual([409, "pair_role_mismatch"]);
    expect((await r.status()).paired_hubs).toEqual([]);
    expect((await pairWith(r, { bundle, role: "gateway", via: "tcp", lan: false })).role).toBe("gateway");
  });

  it("the consent reader takes every action of the contract (bridge.ts), and a relay proposal runs the round trip on accept", () => {
    for (const action of CONSENT_ACTIONS) {
      const c = readConsent({ action, subject: "x", warning_text: "words", hash: "a".repeat(64), addons: [] });
      expect(c?.action, action).toBe(action);
    }
    expect(readConsent({ action: "not_an_action", subject: "x", warning_text: "words", hash: "a".repeat(64) })).toBeNull();
    const err = new ApiError({ type: "about:blank", title: "Confirm first", status: 409, code: "consent_required", consent: { action: "origin_serve", subject: "origin.serve", warning_text: ORIGIN_SERVE_TEXT, hash: "b".repeat(64), addons: [] } });
    expect(consentOf(err)).toMatchObject({ action: "origin_serve" });
    expect(acceptRunsConsent("relay")).toBe(true);
    expect(acceptRunsConsent("bridge_edge")).toBe(true);
    expect(acceptRunsConsent("bridge_message")).toBe(true);
    expect(acceptRunsConsent("device")).toBe(false);
    expect(acceptRunsConsent(undefined)).toBe(false);
  });
});

// ----------------------------------------------------------------------------------------------------- secrets

describe("secrets are write-only (no value is ever on the page)", () => {
  it("a stored token and topic secret read `set`; the page holds neither value, and the fields start empty", async () => {
    const a = await load("hub");
    const r = a.mock.relay;
    await runConsent((hash) => patchSettings(r, { ...NTFY, carrier: { ...NTFY.carrier, token: SECRET_TOKEN, topic_secret: SECRET_TOPIC } }, hash), person().ask);
    await apiStore.refreshRelay();
    const status = apiStore.getState().relay!;
    expect(status.carrier.token).toEqual({ set: true });
    expect(status.carrier.topic_secret).toEqual({ set: true });
    const out = page();
    expect(out).not.toContain(SECRET_TOKEN);
    expect(out).not.toContain(SECRET_TOPIC);
    expect(JSON.stringify(status)).not.toContain(SECRET_TOKEN);
    expect(out).toMatch(/data-testid="secret-token-state">\(set\)/);
    expect(out).toMatch(/data-testid="secret-topic_secret-state">\(set\)/);
    const inputs = out.match(/<input[^>]*type="password"[^>]*>/g) ?? [];
    expect(inputs).toHaveLength(2);
    for (const i of inputs) {
      expect(i).not.toMatch(/\svalue=/); // uncontrolled: no render ever writes a value attribute
      expect(i).toContain('placeholder="set; type to replace"');
      expect(i).toContain('autoComplete="new-password"');
    }
  });

  it("with no secret stored the fields say `not set`, and a clear sends null", async () => {
    const a = await load("hub");
    const r = a.mock.relay;
    await runConsent((hash) => patchSettings(r, NTFY, hash), person().ask);
    expect((await r.status()).carrier.token).toEqual({ set: false });
    await patchSettings(r, { carrier: { token: SECRET_TOKEN } });
    expect((await r.status()).carrier.token).toEqual({ set: true });
    await patchSettings(r, { carrier: { token: null } });
    expect((await r.status()).carrier.token).toEqual({ set: false });
    await apiStore.refreshRelay();
    expect(page()).toMatch(/data-testid="secret-token-state">\(not set\)/);
  });

  it("an http:// server carries the token-in-clear note", async () => {
    expect(isPlainHttp("http://192.168.1.20:2586")).toBe(true);
    expect(isPlainHttp(" HTTP://ntfy.lan")).toBe(true);
    expect(isPlainHttp("https://ntfy.example.org")).toBe(false);
    const a = await load("hub");
    await runConsent((hash) => patchSettings(a.mock.relay, { carrier: { kind: "ntfy", server: "http://192.168.1.20:2586", mode: "5A" } }, hash), person().ask);
    await apiStore.refreshRelay();
    expect(page()).toContain(TOKEN_CLEAR_NOTE);
    const b = await load("hub");
    await runConsent((hash) => patchSettings(b.mock.relay, NTFY, hash), person().ask);
    await apiStore.refreshRelay();
    expect(page()).not.toContain(TOKEN_CLEAR_NOTE);
  });
});

// ----------------------------------------------------------------------------------------------------- presence

describe("presence: the heartbeat is shown and inert, and the node's refusal is shown as written", () => {
  it("lists each transport with its interval, the airtime line and the bans", async () => {
    await load("hub");
    const out = page();
    expect(out).toContain('data-testid="heartbeat-tcp-lan"');
    expect(out).toMatch(/data-testid="heartbeat-tcp-lan-interval">events only/);
    expect(out).toMatch(/data-testid="heartbeat-rns-interval">every 60 min/);
    expect(out).toContain("set in Layer 6");
    expect(out).toContain("7.3 % of the channel");
    expect(out).toContain("Never on a band with a duty-cycle limit.");
    expect(out).toContain("never more often than every 9 minutes");
  });

  it("a write answers 409 no_such_transport; the detail is what the page shows, unchanged", async () => {
    const a = await load("hub");
    const err = await fail(a.mock.relay.patchSettings({ presence: { heartbeat: [{ transport: "tcp-lan", interval_min: 5 }] } }));
    expect([err.status, err.code]).toEqual([409, "no_such_transport"]);
    expect(problemDetail(err)).toBe("No transport takes a presence beat in this version: events only.");
    expect(problemDetail(err)).toBe(err.problem.detail);
  });

  it("settings_locked for rns shows its detail verbatim, which names the route that does change it", async () => {
    const a = await load("hub");
    const err = await fail(a.mock.relay.patchSettings({ presence: { heartbeat: [{ transport: "rns", interval_min: 5 }] } }));
    expect(err.code).toBe("settings_locked");
    expect(problemDetail(err)).toBe(err.problem.detail);
    expect(problemDetail(err)).toContain("PATCH /api/v1/bridge/settings");
    expect((await a.mock.relay.status()).presence.heartbeat.find((x) => x.transport === "tcp-lan")?.interval_min).toBe(0);
  });

  it("problemDetail falls back to the title, names an unreachable node, and never prints an object", () => {
    expect(problemDetail(new ApiError({ type: "about:blank", title: "Nope", status: 409, code: "no_such_transport" }))).toBe("Nope");
    expect(problemDetail(ApiError.network(new Error("refused")))).toContain("could not be reached");
    expect(problemDetail({ weird: true })).toBe("Something went wrong.");
  });

  it("a malformed heartbeat list draws nothing and does not throw", () => {
    expect(heartbeatRows(null)).toEqual([]);
    expect(heartbeatRows({ presence: { heartbeat: [{ transport: 4, interval_min: "x" }, null] } } as never)).toEqual([]);
    expect(heartbeatRows({ presence: { heartbeat: [{ transport: "x‮y", interval_min: 0, owner: "layer7" }] } }).map((r) => r.transport)).toEqual(["xy"]);
  });
});

// ----------------------------------------------------------------------------------------------------- pairing

describe("pairing: the bundle, the complement rule, the pick", () => {
  const BUNDLE = (role: string, via: string, address = "192.168.1.31:11183") => `111PAIR:${"c".repeat(224)}:K7M29X4P:${"s".repeat(71)}:${address}:${role}:${via}`;

  it("reads three fields from the left and two from the right; an IPv6 address keeps its colons; empty means no address", () => {
    expect(parsePairBundle(BUNDLE("peer", "tcp"))).toMatchObject({ code: "K7M29X4P", address: "192.168.1.31:11183", role: "peer", via: "tcp" });
    expect(parsePairBundle(BUNDLE("leaf", "tcp", "[fd00::1]:11183"))?.address).toBe("[fd00::1]:11183");
    expect(parsePairBundle(BUNDLE("peer", "tcp", ""))?.address).toBe("");
    expect(parsePairBundle(`  ${BUNDLE("gateway", "tcp")}\n`)?.role).toBe("gateway");
  });

  it("anything that is not a bundle is null: the wrong prefix, a bad role or link, too few fields, a spaced or oversize text", () => {
    for (const bad of ["", "111HUB:abc:def", BUNDLE("boss", "tcp"), BUNDLE("peer", "udp"), "111PAIR:abc:def", `${BUNDLE("peer", "tcp")} x`, `${BUNDLE("peer", "tcp")}${"a".repeat(700)}`, BUNDLE("peer", "tcp").replace("c".repeat(224), "!".repeat(224))]) {
      expect(parsePairBundle(bad), bad.slice(0, 30)).toBeNull();
    }
  });

  it("the complement: peer with peer, leaf with gateway; a carrier pair is peer only, a leaf or gateway pair is over the network only", () => {
    expect([complementOf("peer"), complementOf("leaf"), complementOf("gateway")]).toEqual(["peer", "gateway", "leaf"]);
    expect(settlePick("leaf", "carrier", "role")).toEqual({ role: "leaf", via: "tcp" });
    expect(settlePick("gateway", "tcp", "role")).toEqual({ role: "gateway", via: "tcp" });
    expect(settlePick("leaf", "carrier", "via")).toEqual({ role: "peer", via: "carrier" });
    expect(settlePick("peer", "tcp", "via")).toEqual({ role: "peer", via: "tcp" });
  });

  it("says what it read and what to pick; a pick that does not match is named before the node refuses it", () => {
    const b = parsePairBundle(BUNDLE("leaf", "tcp"))!;
    expect(bundleSummary(b)).toContain("pick gateway");
    expect(mismatchNote(b, "gateway", "tcp")).toBeNull();
    expect(mismatchNote(b, "leaf", "tcp")).toContain("pick that");
    expect(mismatchNote(b, "gateway", "carrier")).toContain("pick the same");
    expect(bundleSummary(parsePairBundle(BUNDLE("peer", "tcp", ""))!)).toContain("no address");
  });

  it("the page lists each pair with its protection, the lan mark and unpair; an empty list says so", async () => {
    const a = await load("hub");
    const r = a.mock.relay;
    expect(page()).toContain('data-testid="no-pairs"');
    await pairWith(r, { bundle: BUNDLE("peer", "tcp"), role: "peer", via: "tcp", lan: true });
    await apiStore.refreshRelay();
    const out = page();
    expect(out).toContain('data-testid="paired-hub"');
    expect(out).toContain('data-testid="pair-lan-mark"');
    expect(out).toMatch(/data-testid="pair-link-state"[^>]*>lan</);
    expect(out).toContain("Unpair");
    expect(out).toContain('data-testid="pair-offer"');
  });

  it("the offer: a bundle as QR and text for the role this hub proposes, loopback only", async () => {
    const a = await load("hub");
    const offer = await a.mock.relay.pairOffer({ role: "leaf", via: "tcp" });
    expect(validateAgainst("PairOffer", offer)).toEqual([]);
    expect(parsePairBundle(offer.bundle.text)).toMatchObject({ role: "leaf", via: "tcp", code: offer.code });
  });

  it("the pairing controls are gated, with the reason, on a node that is not a hub", async () => {
    await load("healthy");
    const out = page();
    expect(out).toContain("This needs the hub role");
    expect(out).toMatch(/<button[^>]*aria-disabled="true"[^>]*data-testid="pair-offer"/);
  });
});

// ----------------------------------------------------------------------------------------------------- updates

describe("updates: the origin order, the newest release seen, serving, --from", () => {
  const O = (kind: "hub" | "internet" | "directory" | "url", url: string, newest: string | null, ok: boolean | null = null) => ({
    kind,
    url,
    last_answer: ok === null ? null : { ok, ago_s: 120, detail: ok ? null : "no route" },
    newest_seen: newest,
  });

  it("the newest release seen is the highest calver any origin served, named with its origin", () => {
    const seen = newestSeen([O("hub", "http://192.168.1.20:11180/origin/v1/", "2026.10.2"), O("internet", "https://install.111.network/", "2026.11.0"), O("internet", "https://github.com/111network/os/releases/", "2026.9.4")]);
    expect(seen).toEqual({ calver: "2026.11.0", from: "install.111.network" });
    expect(newestSeenText(seen)).toBe("newest release seen: 2026.11.0 from install.111.network");
    expect(newestSeen([O("hub", "http://192.168.1.20:11180/origin/v1/", "2026.12.1")])?.from).toBe("your hub");
    expect(newestSeenText(newestSeen([O("hub", "x", null)]))).toBe("newest release seen: none yet");
    expect(newestSeen([O("hub", "x", "soon"), O("url", "https://stick.example/", "2026.11.0.0.0")])).toBeNull();
    expect(originName({ kind: "url", url: "not a url‮" })).toBe("not a url");
  });

  it("an origin's last answer reads in words, with the node's detail cleaned", () => {
    const ago = (s: number) => `${s} s ago`;
    expect(answerText(null, ago)).toBe("not asked yet");
    expect(answerText({ ok: true, ago_s: 30, detail: null }, ago)).toBe("answered 30 s ago");
    expect(answerText({ ok: false, ago_s: 30, detail: "no‮ route" }, ago)).toBe("failed 30 s ago: no route");
  });

  it("the mock's origins validate, in the order they are tried, and the page shows the line and the instruction", async () => {
    const a = await load("hub");
    const origins = await a.mock.relay.updateOrigins();
    for (const o of origins) expect(validateAgainst("UpdateOrigin", o)).toEqual([]);
    const out = page();
    expect(out).toContain('data-testid="newest-seen"');
    expect(out).toContain(UPDATE_FROM_COMMAND);
    expect(out).toContain('data-testid="origin-targets"');
    expect(out).toContain("x86_64-unknown-linux-musl");
  });

  it("the internet check is a switch the node keeps", async () => {
    const a = await load("hub");
    expect((await patchSettings(a.mock.relay, { updates: { check_internet: false } })).updates.check_internet).toBe(false);
    await apiStore.refreshRelay();
    expect(page()).toMatch(/data-row="internet-check"[\s\S]*?aria-pressed="false"/);
  });
});

// ----------------------------------------------------------------------------------------------------- metered

describe("metered networks", () => {
  it("adds by Wi-Fi name or interface in whole megabytes, replaces the same link, and removes", () => {
    const one = withMetered([], "ssid", " Home hotspot ", "50");
    expect(one).toEqual({ ok: true, list: [{ match: { ssid: "Home hotspot" }, daily_cap_bytes: 50_000_000 }] });
    if (!one.ok) return;
    const two = withMetered(one.list, "iface", "wwan0", "200");
    expect(two.ok && two.list).toHaveLength(2);
    const changed = withMetered(one.list, "ssid", "Home hotspot", "80");
    expect(changed).toEqual({ ok: true, list: [{ match: { ssid: "Home hotspot" }, daily_cap_bytes: 80_000_000 }] });
    expect(withoutMetered(one.list, "ssid:Home hotspot")).toEqual([]);
    expect(withoutMetered(one.list, "iface:Home hotspot")).toHaveLength(1);
  });

  it("refuses a missing name or a cap that is not a whole number of megabytes", () => {
    for (const [name, cap] of [["", "5"], ["x", ""], ["x", "0"], ["x", "1.5"], ["x", "-3"], ["x", "abc"], ["x", "99999999"], ["x\u0007", "5"], ["y".repeat(65), "5"]] as const) {
      expect(withMetered([], "ssid", name, cap).ok, `${name}/${cap}`).toBe(false);
    }
    expect(parseCapMb(" 12 ")).toBe(12);
    expect(parseCapMb("1000000")).toBe(1_000_000);
  });

  it("the page lists the marked links with their cap", async () => {
    const a = await load("hub");
    await runConsent((hash) => patchSettings(a.mock.relay, NTFY, hash), person().ask);
    const next = withMetered([], "ssid", "Home hotspot", "5");
    if (!next.ok) throw new Error("unreachable");
    await runConsent((hash) => patchSettings(a.mock.relay, { metered: next.list }, hash), person().ask);
    await apiStore.refreshRelay();
    expect(page()).toMatch(/data-testid="metered-row"[\s\S]*?Wi-Fi name Home hotspot, 5\.0 MB a day/);
  });
});

// ----------------------------------------------------------------------------------------------------- the follow-ups

describe("the gateway service row links to this page while it is unset (gateway_none, gateway_off)", () => {
  const services = (reason_code: string, state: "live" | "unset") => {
    const rows = apiStore.getState().services!.map((s) => (s.id === "gateway" ? { ...s, state, reason_code } : s));
    apiStore.applyFrame({ id: "s1", event: "services", data: { services: rows } } as never);
  };
  const gatewayPage = () => html(routed("/services/gateway", "/services/:id", ServicesView));

  it("a member (gateway_none) and a gateway turned off link to #/settings/l7; a live or pending gateway does not", async () => {
    await load("healthy");
    expect(gatewayPage()).toMatch(/data-testid="gateway-settings-link"[^>]*>/);
    expect(gatewayPage()).toContain('href="#/settings/l7"');
    services("gateway_off", "unset");
    expect(gatewayPage()).toContain('href="#/settings/l7"');
    services("carrier_ok", "live");
    expect(gatewayPage()).not.toContain("gateway-settings-link");
    services("carrier_pending", "unset");
    expect(gatewayPage()).not.toContain("gateway-settings-link");
  });

  it("no other service row gets the link", async () => {
    await load("healthy");
    const out = html(routed("/services/fabric", "/services/:id", ServicesView));
    expect(out).not.toContain("gateway-settings-link");
  });
});

describe("a scenario seeds the relay mock", () => {
  it("the hub scenario answers GET /relay like its gateway row: configured, enabled, carrier_ok, then carrier_pending with three waiting", async () => {
    const clock = new FakeClock(NOW);
    const a = createMockAdapter({ scenario: "hub", clock, tokenDelayMs: 0 });
    made.push(a);
    const first = await a.mock.relay.status();
    expect(validateAgainst("RelayStatus", first)).toEqual([]);
    expect(first.gateway).toMatchObject({ enabled: true, state: "carrier_ok", carrier: "ntfy", endpoint: "ntfy.example.org", mode: "5A", checked_ago_s: 240, pending: 0, metered_cap_reached: false });
    expect(first.carrier).toMatchObject({ kind: "ntfy", server: "https://ntfy.example.org" });
    expect(first.carrier.consent_hash).toMatch(/^[0-9a-f]{64}$/);
    await clock.advanceAsync(11_000, 500);
    expect((await a.mock.relay.status()).gateway).toMatchObject({ state: "carrier_pending", pending: 3, checked_ago_s: 20, last_flush_ago_s: 300 });
  });

  it("a member scenario keeps gateway_none and the defaults", async () => {
    const a = createMockAdapter({ scenario: "healthy", clock: new FakeClock(NOW), tokenDelayMs: 0 });
    made.push(a);
    const s = await a.mock.relay.status();
    expect(s.gateway).toMatchObject({ enabled: false, state: "gateway_none", carrier: null, pending: 0 });
    expect(s.carrier).toMatchObject({ kind: "none", consent_hash: null });
  });

  it("a status frame carries the seeded state, and relayControl takes over from the scenario", async () => {
    const clock = new FakeClock(NOW);
    const a = createMockAdapter({ scenario: "hub", clock, tokenDelayMs: 0 });
    made.push(a);
    const frames: RelayStatus[] = [];
    a.events({ onFrame: (f) => f.event === "relay" && frames.push(f.data as RelayStatus), onState: () => undefined });
    await flush(); // the stream opens on a microtask
    a.mock.relayControl.setPending(2);
    await clock.advanceAsync(5, 5);
    expect(frames[frames.length - 1]!.gateway).toMatchObject({ state: "carrier_pending", pending: 2, checked_ago_s: 240 });
    await clock.advanceAsync(11_000, 500); // the scenario's script moves its row; the mock now owns the gateway
    expect((await a.mock.relay.status()).gateway).toMatchObject({ state: "carrier_pending", pending: 2, checked_ago_s: 251 });
    a.mock.relayControl.setPending(0);
    expect((await a.mock.relay.status()).gateway.state).toBe("carrier_ok");
    a.mock.relayControl.setMeteredCapReached(true);
    expect((await a.mock.relay.status()).gateway.metered_cap_reached).toBe(true);
  });

  it("writing a gateway or carrier setting also hands the state to the mock; the flush then clears what waits", async () => {
    const clock = new FakeClock(NOW);
    const a = createMockAdapter({ scenario: "hub", clock, tokenDelayMs: 0 });
    made.push(a);
    await clock.advanceAsync(11_000, 500);
    expect((await a.mock.relay.status()).gateway).toMatchObject({ state: "carrier_pending", pending: 3 });
    await a.mock.relay.patchSettings({ gateway: { enabled: false } });
    expect((await a.mock.relay.status()).gateway).toMatchObject({ enabled: false, state: "gateway_off", pending: 3 });
    await a.mock.relay.patchSettings({ gateway: { enabled: true } });
    await a.mock.relay.flushCarrier();
    await clock.advanceAsync(1000, 100);
    expect((await a.mock.relay.status()).gateway).toMatchObject({ state: "carrier_ok", pending: 0, last_flush_ago_s: 0 });
  });

  it("the scenario's metered flag (a `metered_cap_reached: true` in the gateway detail) shows on GET /relay", async () => {
    const deps = {
      now: () => NOW,
      later: () => undefined,
      publish: () => 0,
      loopback: () => true,
      seed: () => ({ hub: true, gateway: { reason_code: "carrier_pending", detail: { carrier: "ntfy", endpoint: "ntfy.example.org", mode: "5B", checked_ago_s: 5, last_flush_ago_s: 60, pending: 4, metered_cap_reached: true } } }),
    };
    const r = createRelayMock(deps);
    expect(r.status().gateway).toMatchObject({ state: "carrier_pending", pending: 4, metered_cap_reached: true, mode: "5B" });
    expect(r.status().carrier.mode).toBe("5B");
    // a seed on a node that is not a hub, or without a configured gateway, is ignored
    expect(createRelayMock({ ...deps, seed: () => ({ hub: false, gateway: { reason_code: "carrier_ok", detail: {} } }) }).status().gateway.state).toBe("gateway_none");
    expect(createRelayMock({ ...deps, seed: () => ({ hub: true, gateway: { reason_code: "gateway_none", detail: {} } }) }).status().gateway.state).toBe("gateway_none");
    expect(createRelayMock({ ...deps, seed: () => ({ hub: true, gateway: null }) }).status().gateway.state).toBe("gateway_none");
  });
});

// ----------------------------------------------------------------------------------------------------- the one helper (actions.ts)

describe("every write goes through one helper that runs the consent round trip (actions.ts, against the mock)", () => {
  it("a 409 hands the node's text to the dialog verbatim, the confirm repeats the call with accept_warning = its hash, and the control ends with a note", async () => {
    const a = await load("healthy");
    const t = harness(a);
    const r = await t.go(WRITES.patch(PATCH.carrier("ntfy", ` ${SERVER} `, "5A")), { done: "Carrier saved." });
    expect(r.ok).toBe(true);
    expect(t.asked).toHaveLength(1);
    expect(t.asked[0]!.consent.warning_text).toBe(carrierConsentText(SERVER, "the other hub"));
    expect(t.asked[0]!.consent.hash).toBe(sha256Hex(carrierConsentText(SERVER, "the other hub")));
    expect(t.states[0]).toEqual({ pending: true, error: null, note: null });
    expect(t.end()).toEqual({ pending: false, error: null, note: "Carrier saved." });
    expect(t.seen.refreshRelay).toBe(1);
    expect((await a.mock.relay.status()).carrier.consent_hash).toBe(t.asked[0]!.consent.hash);
  });

  it("a cancel leaves the control idle and the node unchanged; the control is never left pending", async () => {
    const a = await load("healthy");
    const t = harness(a, [false]);
    const r = await t.go(WRITES.patch(PATCH.carrier("ntfy", SERVER, "5A")), { done: "Carrier saved." });
    expect(r.ok).toBe(false);
    expect(t.asked).toHaveLength(1);
    expect(t.end()).toEqual(IDLE);
    expect(t.seen.refreshRelay).toBe(0);
    expect((await a.mock.relay.status()).carrier).toMatchObject({ kind: "none", consent_hash: null });
  });

  it("a text that changes while the person reads it is asked again, marked changed, and the repeat carries the new hash", async () => {
    const a = await load("healthy");
    const t = harness(a, [true, true], async (c) => {
      if (c.warning_text.includes("the other hub")) await pairWith(a.mock.relay, { bundle: PAIR_BUNDLE("peer", "tcp"), role: "peer", via: "tcp", lan: true });
    });
    const r = await t.go(WRITES.patch(PATCH.carrier("ntfy", SERVER, "5A")), { done: "Carrier saved." });
    expect(r.ok).toBe(true);
    expect(t.asked.map((x) => x.changed)).toEqual([false, true]);
    const far = (await a.mock.relay.status()).paired_hubs[0]!.label!;
    expect(t.asked[1]!.consent.warning_text).toBe(carrierConsentText(SERVER, far));
    expect(t.asked[1]!.consent.warning_text).not.toBe(t.asked[0]!.consent.warning_text);
    expect((await a.mock.relay.status()).carrier.consent_hash).toBe(t.asked[1]!.consent.hash);
  });

  it("a failure shows the node's words and leaves the control idle again (no secret in any state)", async () => {
    const a = await load("healthy");
    const t = harness(a);
    const r = await t.go(WRITES.patch(PATCH.secret("token", SECRET_TOKEN)), { done: "Access token saved.", errorText: problemDetail });
    expect(r.ok).toBe(true);
    expect(JSON.stringify(t.states)).not.toContain(SECRET_TOKEN);
    const bad = harness(a);
    const f = await bad.go(WRITES.patch(PATCH.carrier("ntfy", "ftp://nope", "5A")));
    expect(f.ok).toBe(false);
    expect(bad.end().pending).toBe(false);
    expect(bad.end().error).toMatch(/\S/);
  });

  it("a 401 goes to /pair and leaves no error text", async () => {
    const a = await load("healthy");
    const t = harness(a);
    let redirected = 0;
    const deps = { ...t.deps, relay: async () => ({ patchSettings: async () => Promise.reject(new ApiError({ type: "about:blank", title: "Unauthenticated", status: 401, code: "unauthenticated" })) }) as unknown as RelayAdapter, unauthenticated: () => ++redirected > 0 };
    expect((await runWrite(deps, WRITES.patch(PATCH.gatewayEnabled(true)))).ok).toBe(false);
    expect(redirected).toBe(1);
  });

  // --- one test per write kind -----------------------------------------------------------------------------------------------

  it("heartbeat: the node refuses (no_such_transport, settings_locked) and the line shows its detail verbatim; no dialog", async () => {
    const a = await load("hub");
    for (const [transport, code] of [["tcp-lan", "no_such_transport"], ["rns", "settings_locked"]] as const) {
      const t = harness(a);
      const r = await t.go(WRITES.patch(PATCH.heartbeat(transport, 5)), HEARTBEAT_OPTS);
      expect(r.ok).toBe(false);
      const err = await fail(a.mock.relay.patchSettings(PATCH.heartbeat(transport, 5)));
      expect(err.code).toBe(code);
      expect(t.asked).toHaveLength(0);
      expect(t.end()).toEqual({ pending: false, error: err.problem.detail, note: null });
    }
    // HeartbeatLine itself: it passes HEARTBEAT_OPTS, and draws the control's error text under the line
    const row = heartbeatRows((await a.mock.relay.status()))[0]!;
    const detail = "No transport takes a presence beat in this version: events only.";
    const out = html(h(RelayCallSeed.Provider, { value: { state: { pending: false, error: detail, note: null } } }, h(HeartbeatLine, { row })));
    expect(out).toContain(`Problem: ${esc(detail)}`);
    expect(HEARTBEAT_OPTS.errorText).toBe(problemDetail);
  });

  it("gateway toggle: on and off, the node's answer is what the page then reads", async () => {
    const a = await load("healthy");
    const t = harness(a);
    expect((await t.go(WRITES.patch(PATCH.gatewayEnabled(true)), { done: "Gateway on." })).ok).toBe(true);
    expect((await a.mock.relay.status()).gateway.enabled).toBe(true);
    expect((await t.go(WRITES.patch(PATCH.gatewayEnabled(false)), { done: "Gateway off." })).ok).toBe(true);
    expect((await a.mock.relay.status()).gateway.enabled).toBe(false);
    expect(t.asked).toHaveLength(0);
  });

  it("carrier form: on asks (carrier_enable), off clears the consent record", async () => {
    const a = await load("healthy");
    const t = harness(a);
    expect((await t.go(WRITES.patch(PATCH.carrier("ntfy", SERVER, "5B")))).ok).toBe(true);
    expect(t.asked[0]!.consent.action).toBe("carrier_enable");
    expect((await a.mock.relay.status()).carrier).toMatchObject({ kind: "ntfy", server: SERVER, mode: "5B" });
    expect((await t.go(WRITES.patch(PATCH.carrier("none", "", "5A")))).ok).toBe(true);
    expect((await a.mock.relay.status()).carrier).toMatchObject({ kind: "none", server: null, consent_hash: null });
    expect(t.asked).toHaveLength(1);
  });

  it("secret: saved and cleared through the helper; only {set} comes back", async () => {
    const a = await load("healthy");
    const t = harness(a);
    await t.go(WRITES.patch(PATCH.carrier("ntfy", SERVER, "5A")));
    await t.go(WRITES.patch(PATCH.secret("token", SECRET_TOKEN)), { done: "Access token saved." });
    await t.go(WRITES.patch(PATCH.secret("topic_secret", SECRET_TOPIC)), { done: "Topic secret saved." });
    const st = await a.mock.relay.status();
    expect([st.carrier.token, st.carrier.topic_secret]).toEqual([{ set: true }, { set: true }]);
    expect(JSON.stringify([st, t.states])).not.toContain(SECRET_TOKEN);
    expect(JSON.stringify([st, t.states])).not.toContain(SECRET_TOPIC);
    await t.go(WRITES.patch(PATCH.secret("token", null)), { done: "Access token cleared." });
    expect((await a.mock.relay.status()).carrier.token).toEqual({ set: false });
  });

  it("metered cap: raising it while the carrier is on asks again; removing the link goes through the same helper", async () => {
    const a = await load("healthy");
    const t = harness(a, [true, true, true]);
    await t.go(WRITES.patch(PATCH.carrier("ntfy", SERVER, "5A")));
    const next = withMetered([], "ssid", "Home hotspot", "5");
    if (!next.ok) throw new Error("input");
    expect((await t.go(WRITES.patch(PATCH.metered(next.list)), { done: "Saved." })).ok).toBe(true);
    expect(t.asked[1]!.consent.addons).toEqual(["metered"]);
    expect((await a.mock.relay.status()).metered).toHaveLength(1);
    expect((await t.go(WRITES.patch(PATCH.metered(withoutMetered(next.list, "ssid:Home hotspot"))), { done: "Removed." })).ok).toBe(true);
    expect((await a.mock.relay.status()).metered).toEqual([]);
  });

  it("check now and flush now: they meet 409 consent_required, the accept goes through PATCH /relay/settings with the hash, the repeat answers 202", async () => {
    for (const which of ["check", "flush"] as const) {
      const a = await load("healthy");
      const r = a.mock.relay;
      await runConsent((hash) => patchSettings(r, NTFY, hash), person().ask);
      // a hub is paired: the text now owed names it, so the consent on record is not the one owed
      await pairWith(r, { bundle: PAIR_BUNDLE("peer", "tcp"), role: "peer", via: "tcp", lan: true });
      const far = (await r.status()).paired_hubs[0]!.label!;
      const met = relayConsentOf(await fail(which === "check" ? r.checkCarrier() : r.flushCarrier()));
      expect(met).toMatchObject({ action: "carrier_enable", warning_text: carrierConsentText(SERVER, far) });
      const status = await r.status();
      const t = harness(a);
      const out = await t.go(WRITES.carrier(which, status.carrier), { done: "Started." });
      expect(out.ok, which).toBe(true);
      expect(t.asked).toHaveLength(1);
      expect(t.asked[0]!.consent.warning_text).toBe(carrierConsentText(SERVER, far));
      expect((await r.status()).carrier.consent_hash).toBe(t.asked[0]!.consent.hash);
      expect(await (which === "check" ? r.checkCarrier() : r.flushCarrier())).toEqual({ schema: 1, started: true });
      // the repeat needs no question
      const again = harness(a);
      expect((await again.go(WRITES.carrier(which, (await r.status()).carrier))).ok).toBe(true);
      expect(again.asked).toHaveLength(0);
      // a cancel starts nothing
    }
    const a = await load("healthy");
    await runConsent((hash) => patchSettings(a.mock.relay, NTFY, hash), person().ask);
    await pairWith(a.mock.relay, { bundle: PAIR_BUNDLE("peer", "tcp"), role: "peer", via: "tcp", lan: true });
    const no = harness(a, [false]);
    expect((await no.go(WRITES.carrier("check", (await a.mock.relay.status()).carrier))).ok).toBe(false);
    expect(no.end()).toEqual(IDLE);
    expect((await a.mock.relay.status()).gateway.checked_ago_s).toBeNull();
  });

  it("a PATCH carrying the hash of the text owed now records that consent even when no value changes; a stale or foreign hash is refused with the current text", async () => {
    const a = await load("healthy");
    const r = a.mock.relay;
    await runConsent((hash) => patchSettings(r, NTFY, hash), person().ask);
    await pairWith(r, { bundle: PAIR_BUNDLE("peer", "tcp"), role: "peer", via: "tcp", lan: true });
    const far = (await r.status()).paired_hubs[0]!.label!;
    const now = carrierConsentText(SERVER, far);
    const stale = relayConsentOf(await fail(r.patchSettings({ carrier: { kind: "ntfy", server: SERVER } }, sha256Hex(carrierConsentText(SERVER, "the other hub")))));
    expect(stale).toMatchObject({ action: "carrier_enable", warning_text: now, hash: sha256Hex(now) });
    expect(relayConsentOf(await fail(r.patchSettings({ carrier: { kind: "ntfy", server: SERVER } }, "0".repeat(64))))?.warning_text).toBe(now);
    expect((await r.status()).carrier.consent_hash).not.toBe(sha256Hex(now));
    await r.patchSettings({ carrier: { kind: "ntfy", server: SERVER } }, sha256Hex(now));
    expect((await r.status()).carrier.consent_hash).toBe(sha256Hex(now));
    // another consent's hash in an unrelated write is not refused as a stale carrier text
    await r.patchSettings({ updates: { check_internet: false } });
  });

  it("origin serve, the gateway bit, the offload bit, the LAN record and the internet check", async () => {
    const a = await load("hub");
    const t = harness(a, [true, true]);
    expect((await t.go(WRITES.patch(PATCH.originServe(true)))).ok).toBe(true);
    expect((await t.go(WRITES.patch(PATCH.gatewayBit(true)))).ok).toBe(true);
    expect(t.asked.map((x) => x.consent.action)).toEqual(["origin_serve", "gateway_bit"]);
    expect(t.asked.map((x) => x.consent.warning_text)).toEqual([ORIGIN_SERVE_TEXT, GATEWAY_BIT_TEXT]);
    for (const w of [PATCH.offloadBit(true), PATCH.lanRecord(true), PATCH.checkInternet(false)]) expect((await t.go(WRITES.patch(w))).ok).toBe(true);
    const st = await a.mock.relay.status();
    expect([st.origin.serve, st.discovery, st.updates.check_internet]).toEqual([true, { lan_record: true, public_gateway_bit: true, offload_bit: true }, false]);
    expect(t.asked).toHaveLength(2);
  });

  it("targets: the list is written whole, and the node's refusal of one is shown as it wrote it", async () => {
    const a = await load("hub");
    const t = harness(a);
    expect((await t.go(WRITES.patch(PATCH.targets(["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"])), { done: "added.", errorText: problemDetail })).ok).toBe(true);
    expect((await a.mock.relay.status()).origin.targets).toEqual(["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"]);
    const refusal = new ApiError({ type: "about:blank", title: "Refused", status: 409, code: "bad_request", detail: "This hub always holds its own target: x86_64-unknown-linux-musl cannot be removed." });
    const refuses = { patchSettings: async () => Promise.reject(refusal) } as unknown as RelayAdapter;
    const deps = { ...t.deps, relay: async () => refuses };
    expect((await runWrite(deps, WRITES.patch(PATCH.targets(["aarch64-unknown-linux-musl"])), { errorText: problemDetail })).ok).toBe(false);
    expect(t.end()).toEqual({ pending: false, error: refusal.problem.detail, note: null });
  });

  it("pairing: pair_plain asks and the repeat carries the hash; a pair marked lan asks nothing; unpair removes it", async () => {
    const a = await load("hub");
    const t = harness(a);
    const plainHub = PAIR_BUNDLE("peer", "tcp", "203.0.113.7:11183", "a");
    expect((await t.go(WRITES.pair({ bundle: plainHub, role: "peer", via: "tcp", lan: false }))).ok).toBe(true);
    expect(t.asked[0]!.consent).toMatchObject({ action: "pair_plain", warning_text: PAIR_PLAIN_TEXT });
    const lanHub = PAIR_BUNDLE("peer", "tcp", "203.0.113.9:11183", "b");
    expect((await t.go(WRITES.pair({ bundle: lanHub, role: "peer", via: "tcp", lan: true }))).ok).toBe(true);
    expect(t.asked).toHaveLength(1);
    const pairs = (await a.mock.relay.status()).paired_hubs;
    expect(pairs.map((p) => p.tls)).toEqual(["plain", "none"]);
    expect((await t.go(WRITES.unpair(pairs[0]!.node_id))).ok).toBe(true);
    expect((await a.mock.relay.status()).paired_hubs).toHaveLength(1);
  });

  it("the pairing code: a bundle mints it (the fabric slice shows it), it is dismissed with the Layer 5 route, and the page shows both states", async () => {
    const a = await load("hub");
    expect(page()).not.toContain("Dismiss this code");
    const t = harness(a);
    const made = await t.go(WRITES.pairOffer({ role: "peer", via: "tcp" }));
    expect(made.ok).toBe(true);
    expect(t.seen.refreshFabric).toBe(1);
    await apiStore.refreshFabric();
    expect(apiStore.getState().fabric?.hub.offer_active).toBe(true);
    // a code minted earlier or elsewhere is visible and can be dismissed
    const out = page();
    expect(out).toContain('data-testid="pair-dismiss"');
    expect(out).toContain(esc(OUTSTANDING_NOTE));
    expect((await t.go(WRITES.dismissOffer(), { done: "The code was dismissed." })).ok).toBe(true);
    expect(apiStore.getState().fabric?.hub.offer_active).toBe(false);
    expect(page()).not.toContain("Dismiss this code");
    // idempotent
    expect((await t.go(WRITES.dismissOffer())).ok).toBe(true);
  });

  it("the earlier code: the line says it stays valid until it is dismissed or replaced; a code on screen needs no line", () => {
    expect(offerNote(false, true, true)).toBe(OFFER_EARLIER_NOTE);
    expect(offerNote(false, true, false)).toBe(OFFER_EARLIER_NOTE);
    expect(OFFER_EARLIER_NOTE).toMatch(/still valid.*until you dismiss it or show a new one/);
    expect(offerNote(false, false, true)).toBe(OUTSTANDING_NOTE);
    expect(offerNote(false, false, false)).toBeNull();
    expect(offerNote(true, true, true)).toBeNull();
  });

  it("the words of the four Layer 7 consent actions; the Layer 6 words are as they were", () => {
    expect([actionWord("carrier_enable"), actionWord("pair_plain"), actionWord("gateway_bit"), actionWord("origin_serve")]).toEqual([
      "Turn the carrier on",
      "Pair on an unencrypted link",
      "Advertise this node as a gateway",
      "Serve update files",
    ]);
    expect([actionWord("edge_enable"), actionWord("uplink_send"), actionWord("component_enable")]).toEqual(["Start this relay", "Turn on sending on an uplinked channel", "Install and turn this on"]);
  });
});

// ----------------------------------------------------------------------------------------------------- every section mounts the helper's dialog

describe("every section renders its controls' consent dialogs (the helper in its consent state)", () => {
  async function ready() {
    const a = await load("hub");
    const r = a.mock.relay;
    await runConsent((hash) => patchSettings(r, NTFY, hash), person().ask);
    await pairWith(r, { bundle: PAIR_BUNDLE("peer", "tcp"), role: "peer", via: "tcp", lan: true });
    await apiStore.refreshRelay();
    return apiStore.getState().relay!;
  }
  const consent: ConsentRequired = {
    action: "carrier_enable",
    subject: `ntfy:${SERVER}`,
    warning_text: carrierConsentText(SERVER, "the other hub"),
    hash: sha256Hex(carrierConsentText(SERVER, "the other hub")),
    addons: [],
  };
  const seeded = (el: ReactElement) => html(h(RelayCallSeed.Provider, { value: { consent } }, el));
  const shows = (out: string, panels: number) => {
    expect(count(out, 'data-testid="consent-panel"')).toBe(panels);
    expect(count(out, esc(consent.warning_text))).toBe(panels);
    expect(count(out, 'data-testid="consent-cancel"')).toBe(panels);
    expect(count(out, 'data-testid="consent-confirm"')).toBe(panels);
  };

  it("presence (a line per transport), gateway (check/flush, toggle, carrier, secrets), paired hubs (show, pair, one per pair), discovery (three), updates (two switches and the targets), metered", async () => {
    const relay = await ready();
    shows(seeded(h(PresenceSection, { relay })), 2);
    shows(seeded(h(GatewaySection, { relay, hubGate: false })), 4);
    shows(seeded(h(PairingSection, { relay, hubGate: false })), 3);
    shows(seeded(h(DiscoverySection, { relay, hubGate: false })), 3);
    shows(seeded(h(UpdatesSection, { relay, hubGate: false })), 3);
    shows(seeded(h(MeteredSection, { relay })), 1);
    shows(seeded(h(RelaySettings)), 16);
  });

  it("without a consent no section shows a panel", async () => {
    const relay = await ready();
    const out = html(h(RelaySettings));
    expect(count(out, 'data-testid="consent-panel"')).toBe(0);
    expect(count(html(h(GatewaySection, { relay, hubGate: false })), "consent-cancel")).toBe(0);
  });

  it("the folder has one `<CallOutput>` for every `useRelayCall()` and no write around the helper", () => {
    const dir = join(__dirname, "../src/views/settings/l7");
    for (const f of readdirSync(dir)) {
      const src = readFileSync(join(dir, f), "utf8");
      if (f !== "actions.ts") {
        expect(src, `${f}: the adapter and the consent flow are reached through actions.ts only`).not.toMatch(/\b(patchSettings|checkCarrier|flushCarrier|dismissHubOffer|useConsentFlow|pairWith|carrierCall)\b/);
        expect(src, `${f}: no adapter call`).not.toMatch(/\b(?:relay|adapter|r)\.(?:pair|pairOffer|unpair|fabric|decide)\b/);
        if (f !== "updates.tsx") expect(src, `${f}: reads go through the store`).not.toMatch(/\bgetRelay\b/);
      }
      expect(count(src, "useRelayCall("), `${f}: a dialog per helper`).toBe(count(src, "<CallOutput"));
    }
    // The heartbeat write hands the node's problem detail to the page (the options are passed where the write is made).
    expect(readFileSync(join(dir, "presence.tsx"), "utf8")).toMatch(/call\.patch\(PATCH\.heartbeat\([^;]*, HEARTBEAT_OPTS\);/);
  });
});

// ----------------------------------------------------------------------------------------------------- a typed secret, the pair pick

describe("a typed, unsaved secret is in no markup", () => {
  it("typing is simulated through the model: the draft says something is typed and the input still renders with no value", () => {
    const draft = typeSecret(EMPTY_SECRET, SECRET_TOKEN);
    expect(draft).toEqual({ filled: true });
    expect(JSON.stringify(draft)).not.toContain(SECRET_TOKEN);
    expect(typeSecret(draft, "")).toEqual(EMPTY_SECRET);
    const out = html(h(SecretInput, { id: "x-in", name: "ntfy-token", isSet: draft.filled }));
    expect(out).not.toContain(SECRET_TOKEN);
    expect(out).not.toMatch(/\svalue=/);
    expect(out).toContain('type="password"');
    expect(out).toContain('autoComplete="new-password"');
  });
});

describe("pairing: the bundle's pick is applied when the bundle changes, never over the owner's own pick", () => {
  const read = (role: string, via: string, address?: string, ch?: string) => parsePairBundle(PAIR_BUNDLE(role, via, address, ch));
  it("a newly read bundle proposes the complement over its link", () => {
    expect(pickForBundle(null, read("leaf", "tcp"))).toEqual({ role: "gateway", via: "tcp" });
    expect(pickForBundle(null, read("peer", "carrier"))).toEqual({ role: "peer", via: "carrier" });
  });
  it("the same bundle, however the text was edited around it, proposes nothing", () => {
    const b = read("leaf", "tcp")!;
    expect(pickForBundle(b, parsePairBundle(`  ${PAIR_BUNDLE("leaf", "tcp")}\n`))).toBeNull();
    expect(pickForBundle(b, read("leaf", "tcp"))).toBeNull();
  });
  it("another bundle proposes again; text that is not a bundle proposes nothing", () => {
    const b = read("leaf", "tcp")!;
    expect(pickForBundle(b, read("peer", "tcp", "10.0.0.2:11183", "d"))).toEqual({ role: "peer", via: "tcp" });
    expect(pickForBundle(b, parsePairBundle("111PAIR:half"))).toBeNull();
    expect(pickForBundle(null, null)).toBeNull();
  });
});

// ----------------------------------------------------------------------------------------------------- a relay proposal

describe("a relay proposal card: accepting runs the consent round trip, dismissing never does", () => {
  const relayProposal = { schema: 1, id: "prop-relay-1", node_id: null, task_id: null, kind: "relay", title: "Turn on the gateway", body: '{"why":"The hub has a carrier ready.","actions":[{"action":"gateway.enable","carrier":"ntfy"}]}', created_at: "2026-11-11T08:00:00Z", decision: null } as Proposal;
  const gated = () => {
    const seen: (string | undefined)[] = [];
    const adapter = {
      decide: async (id: string, decision: string, hash?: string) => {
        seen.push(hash);
        if (decision === "accepted" && hash === undefined) {
          throw new ApiError({ type: "about:blank", title: "Confirm first", status: 409, code: "consent_required", consent: { action: "carrier_enable", subject: "ntfy:x", warning_text: carrierConsentText(SERVER, "the other hub"), hash: sha256Hex(carrierConsentText(SERVER, "the other hub")), addons: [] } });
        }
        return { ...relayProposal, id, decision: { decision, at: "2026-11-11T09:00:00Z" } } as Proposal;
      },
    };
    return { adapter, seen };
  };
  const attemptWith = (answers: boolean[]) => {
    const p = person(answers);
    return { p, attempt: ((call: (hash?: string) => Promise<unknown>) => runConsent((hash) => call(hash), p.ask)) as Parameters<typeof decideProposal>[1] };
  };

  it("accept: the node's text is shown, the decision is repeated with its hash", async () => {
    const { adapter, seen } = gated();
    const { p, attempt } = attemptWith([true]);
    expect(await decideProposal(adapter as never, attempt, relayProposal, "accepted")).toBe("done");
    expect(p.asked).toHaveLength(1);
    expect(p.asked[0]!.warning_text).toBe(carrierConsentText(SERVER, "the other hub"));
    expect(seen).toEqual([undefined, sha256Hex(carrierConsentText(SERVER, "the other hub"))]);
  });

  it("a cancel leaves the proposal open (one call, no repeat); a dismissal is recorded without a question", async () => {
    const { adapter, seen } = gated();
    const no = attemptWith([false]);
    expect(await decideProposal(adapter as never, no.attempt, relayProposal, "accepted")).toBe("cancelled");
    expect(seen).toEqual([undefined]);
    const { adapter: other, seen: seen2 } = gated();
    const dismiss = attemptWith([true]);
    expect(await decideProposal(other as never, dismiss.attempt, relayProposal, "dismissed")).toBe("done");
    expect(dismiss.p.asked).toHaveLength(0);
    expect(seen2).toEqual([undefined]);
  });

  it("a refusal other than a consent is thrown for the card to show; a kind outside Layer 6 and 7 is decided as it was", async () => {
    const refuse = { decide: async () => Promise.reject(new ApiError({ type: "about:blank", title: "No", status: 409, code: "bad_request", detail: "Not available in this version." })) };
    await expect(decideProposal(refuse as never, attemptWith([true]).attempt, relayProposal, "accepted")).rejects.toMatchObject({ code: "bad_request" });
    const { adapter, seen } = gated();
    const plainKind = attemptWith([true]);
    await expect(decideProposal(adapter as never, plainKind.attempt, { ...relayProposal, kind: "diagnose" }, "accepted")).rejects.toMatchObject({ code: "consent_required" });
    expect(plainKind.p.asked).toHaveLength(0);
    expect(seen).toEqual([undefined]);
  });

  it("on the mock a relay proposal is recorded on accept; the card shows its actions", async () => {
    const a = await load("hub");
    a.mock.state.proposals.unshift({ ...relayProposal });
    const { attempt, p } = attemptWith([true]);
    expect(await decideProposal(a, attempt, relayProposal, "accepted")).toBe("done");
    expect(p.asked).toHaveLength(0);
    expect((await a.proposal(relayProposal.id)).decision?.decision).toBe("accepted");
    const out = html(h(ProposalCards, { proposals: [relayProposal], onWhy: () => undefined, onDecided: () => undefined }));
    expect(out).toContain('data-testid="relay-card"');
    expect(out).toContain("turn on the gateway through ntfy");
    expect(out).toContain("Mark accepted");
  });
});

// ----------------------------------------------------------------------------------------------------- the chunk

describe("the page is its own lazy chunk of at most 64 KB gzip", () => {
  it("a production build puts /settings/l7 in a chunk the first paint does not load, under the per-chunk cap", async () => {
    const out = mkdtempSync(join(tmpdir(), "l7-chunk-"));
    try {
      await checkChunk(out);
    } finally {
      rmSync(out, { recursive: true, force: true });
    }
  }, 120_000);

  async function checkChunk(out: string): Promise<void> {
    const { build } = await import("vite");
    const env = process.env.NODE_ENV;
    process.env.NODE_ENV = "production"; // vitest sets "test"; the chunk is measured as it ships
    try {
      await build({ root: join(__dirname, ".."), mode: "production", logLevel: "silent", build: { outDir: out, emptyOutDir: true, reportCompressedSize: false } });
    } finally {
      if (env === undefined) delete process.env.NODE_ENV;
      else process.env.NODE_ENV = env;
    }
    const index = readFileSync(join(out, "index.html"), "utf8");
    const eager = new Set([...index.matchAll(/(?:src|href)="[^"]*\/([^"/]+\.js)"/g)].map((m) => m[1]!));
    const chunks = readdirSync(join(out, "assets")).filter((f) => f.endsWith(".js"));
    const mine = chunks.filter((f) => readFileSync(join(out, "assets", f), "utf8").includes("relay-settings"));
    expect(mine).toHaveLength(1);
    const file = mine[0]!;
    expect(eager.has(file)).toBe(false);
    const gz = gzipSync(readFileSync(join(out, "assets", file)), { level: 9 }).length;
    expect(gz).toBeGreaterThan(1000);
    expect(gz).toBeLessThanOrEqual(64_000);
    // and the whole budget gate agrees
    const gate = spawnSync(process.execPath, [join(__dirname, "..", "scripts", "check-budget.mjs"), "--dist", out], { encoding: "utf8" });
    expect(gate.status, gate.stderr).toBe(0);
    // no module outside the settings view imports the page statically
    const src = readFileSync(join(__dirname, "..", "src", "views", "settings", "index.tsx"), "utf8");
    expect(src).toContain('lazy(() => import("./l7"))');
    expect(src).not.toMatch(/^import .* from "\.\/l7/m);
  }
});
