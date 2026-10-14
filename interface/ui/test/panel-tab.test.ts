// The Panel tab (os/messaging/README.md section 13): the iframe's attributes, the ticket URL the frame may open, the
// secure-context gate, the loopback-only copy, a reload that mints a fresh ticket, "Open full client", and the service page
// that carries it (rendered from the mock's `meshtastic` scenario through the real store).
import { createElement as h, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import { createMockAdapter, type MockAdapter, type ScenarioName } from "../src/lib/api/mock";
import { ApiError } from "../src/lib/api/problem";
import { apiStore } from "../src/lib/api/store";
import type { HostedPanel, PanelSession } from "../src/lib/api/types";
import { ModeProvider, resetModeForTests } from "../src/lib/mode";
import ServicesView from "../src/views/services";
import { PanelFrame, PanelTab } from "../src/views/services/panel";
import {
  allowAttr,
  classifyMint,
  LOOPBACK_ONLY_PANEL_TEXT,
  loopbackDashboardUrl,
  mintFailureText,
  optionalHostsText,
  PANEL_SANDBOX,
  panelGate,
  panelNotes,
  panelTitle,
  SECURE_CONTEXT_TEXT,
  ticketSrc,
  type PanelEnv,
} from "../src/views/services/panel-model";
import { openFullClient, PanelSessionController, type OpenedWindow } from "../src/views/services/panel-session";
import { servicePageState } from "../src/views/services/model";
import { nextTab } from "../src/views/services/tabs";
import { FakeClock, flush } from "./helpers/fakes";
import { routed } from "./helpers/routed";

const LOOPBACK: PanelEnv = { protocol: "http:", hostname: "127.0.0.1", port: "11180" };
const LAN: PanelEnv = { protocol: "http:", hostname: "192.168.1.42", port: "11180" };
const MDNS: PanelEnv = { protocol: "http:", hostname: "111.local", port: "" };
const HTTPS: PanelEnv = { protocol: "https:", hostname: "node.example", port: "" };

const panel = (over: Partial<HostedPanel> = {}): HostedPanel => ({
  id: "meshtastic",
  port: 11190,
  entry: "/",
  needs_secure_context: false,
  allow: ["serial", "bluetooth"],
  optional_hosts: ["tiles.openfreemap.org", "api.meshtastic.org"],
  state: { enabled: true, listening: true, health: "up", dropped: 0 },
  title: "Meshtastic",
  secure_note: "USB or Bluetooth from this browser works only on the node itself.",
  ...over,
});

// ------------------------------------------------------------------------------------------ the frame attributes

describe("the iframe's sandbox and permissions", () => {
  it("is the sandbox of section 13, exactly, and never lets the panel move the dashboard", () => {
    expect(PANEL_SANDBOX).toBe("allow-scripts allow-same-origin allow-forms allow-popups allow-modals allow-downloads");
    expect(PANEL_SANDBOX).not.toContain("allow-top-navigation");
    expect(PANEL_SANDBOX.split(" ")).not.toContain("allow-top-navigation-by-user-activation");
  });

  it("allow comes from panel.allow, as bare feature names the browser scopes to the frame's own origin", () => {
    expect(allowAttr(["serial", "bluetooth"])).toBe("serial; bluetooth");
    expect(allowAttr([])).toBeNull();
    expect(allowAttr(["serial", "serial"])).toBe("serial");
  });

  it("drops anything that is not a plain feature name (no '*', no other origins, no extra directives)", () => {
    expect(allowAttr(["geolocation *", "camera https://evil.example", "microphone; camera", "SERIAL", "", "usb"])).toBe("usb");
    expect(allowAttr(["*"])).toBeNull();
  });

  it("the rendered frame carries the sandbox, allow and the ticket URL once, with no referrer", () => {
    const ticket = "http://127.0.0.1:11190/?p111_ticket=00112233445566778899aabbccddeeff";
    const out = renderToStaticMarkup(h(PanelFrame, { src: ticket, title: "Meshtastic", panel: panel() }));
    expect(out).toMatch(/^<iframe /);
    expect(out).toContain(`sandbox="${PANEL_SANDBOX}"`);
    expect(out).toContain('allow="serial; bluetooth"');
    expect(out).toContain('referrerPolicy="no-referrer"');
    expect(out).toContain('title="Meshtastic panel"');
    expect(out).not.toContain("allow-top-navigation");
    expect(out.split("00112233445566778899aabbccddeeff")).toHaveLength(2); // the ticket appears once, in `src`
    expect(out).toContain(`src="${ticket}"`);
  });

  it("a panel that asks for no features gets no allow attribute", () => {
    const out = renderToStaticMarkup(h(PanelFrame, { src: "http://127.0.0.1:11190/", title: "P", panel: panel({ allow: [] }) }));
    expect(out).not.toContain("allow=");
  });
});

// ------------------------------------------------------------------------------------------ the ticket URL

describe("ticketSrc: only the panel origin this page computes is ever framed", () => {
  const ok = (raw: string, env: PanelEnv = LOOPBACK, p = panel()) => ticketSrc(raw, p, env);

  it("accepts the daemon's URL on the typed host and the panel's slot", () => {
    expect(ok("http://127.0.0.1:11190/?p111_ticket=abc")).toBe("http://127.0.0.1:11190/?p111_ticket=abc");
    expect(ok("http://192.168.1.42:11190/app?p111_ticket=abc", LAN)).toBe("http://192.168.1.42:11190/app?p111_ticket=abc");
    expect(ok("http://111.local:11190/?p111_ticket=abc", MDNS)).toBe("http://111.local:11190/?p111_ticket=abc");
  });

  it("a relative URL resolves against the panel origin; the fragment is dropped", () => {
    expect(ok("/?p111_ticket=abc#frag")).toBe("http://127.0.0.1:11190/?p111_ticket=abc");
  });

  it("refuses another host, another port, another scheme, and credentials", () => {
    expect(ok("http://evil.example:11190/?p111_ticket=abc")).toBeNull();
    expect(ok("http://127.0.0.1:11191/?p111_ticket=abc")).toBeNull();
    expect(ok("http://127.0.0.1/?p111_ticket=abc")).toBeNull();
    expect(ok("javascript:alert(1)")).toBeNull();
    expect(ok("data:text/html,<script>1</script>")).toBeNull();
    expect(ok("http://user:pw@127.0.0.1:11190/")).toBeNull();
    expect(ok("http://192.168.1.42:11190/", LOOPBACK)).toBeNull(); // the LAN address is not the host this page was opened on
    expect(ok("not a url at all %%%", LOOPBACK)).not.toBe("not a url at all %%%");
  });

  it("IPv6 hosts compare bracketed or not", () => {
    const v6: PanelEnv = { protocol: "http:", hostname: "[::1]", port: "11180" };
    expect(ticketSrc("http://[::1]:11190/?p111_ticket=abc", panel(), v6)).toBe("http://[::1]:11190/?p111_ticket=abc");
    expect(ticketSrc("http://[::2]:11190/?p111_ticket=abc", panel(), v6)).toBeNull();
  });
});

// ------------------------------------------------------------------------------------------ the gates

describe("panelGate", () => {
  it("frames a panel on a loopback origin, over HTTPS, and on a LAN origin when it needs no secure context", () => {
    const secure = panel({ needs_secure_context: true });
    expect(panelGate(secure, LOOPBACK, "meshtastic")).toEqual({ kind: "frame" });
    expect(panelGate(secure, HTTPS, "meshtastic")).toEqual({ kind: "frame" });
    expect(panelGate(secure, { ...LOOPBACK, hostname: "111.localhost" }, "meshtastic")).toEqual({ kind: "frame" });
    expect(panelGate(panel(), LAN, "meshtastic")).toEqual({ kind: "frame" }); // the real Meshtastic declaration: `false`
    expect(panelGate(panel(), MDNS, "meshtastic")).toEqual({ kind: "frame" });
  });

  it("needs_secure_context on a LAN address or 111.local: no frame, the loopback URL instead", () => {
    const secure = panel({ needs_secure_context: true });
    const lan = panelGate(secure, LAN, "meshtastic");
    expect(lan).toEqual({ kind: "secure_context", text: SECURE_CONTEXT_TEXT, url: "http://127.0.0.1:11180/#/services/meshtastic" });
    const mdns = panelGate(secure, MDNS, "meshtastic");
    expect(mdns).toMatchObject({ kind: "secure_context", url: "http://127.0.0.1/#/services/meshtastic" });
    expect(SECURE_CONTEXT_TEXT).toMatch(/node itself, or over HTTPS/);
  });

  it("a panel that is off or not listening is never framed, whatever the origin", () => {
    expect(panelGate(panel({ state: { enabled: false, listening: false, health: "unknown" } }), LOOPBACK, "meshtastic").kind).toBe("off");
    expect(panelGate(panel({ state: { enabled: true, listening: false, health: "up" } }), LOOPBACK, "meshtastic").kind).toBe("not_listening");
    // off wins over the secure-context gate: there is nothing to open on the node either
    expect(panelGate(panel({ needs_secure_context: true, state: { enabled: false, listening: false, health: "unknown" } }), LAN, "meshtastic").kind).toBe("off");
  });

  it("the loopback URL keeps the dashboard's port and leaves out the default one", () => {
    expect(loopbackDashboardUrl({ ...LAN, port: "11180" }, "meshtastic")).toBe("http://127.0.0.1:11180/#/services/meshtastic");
    expect(loopbackDashboardUrl({ ...LAN, port: "" }, "meshtastic")).toBe("http://127.0.0.1/#/services/meshtastic");
    expect(loopbackDashboardUrl({ ...LAN, port: "80" }, "meshtastic")).toBe("http://127.0.0.1/#/services/meshtastic");
  });
});

describe("a failed mint", () => {
  const problem = (status: number, code: string, detail?: string) => new ApiError({ type: "about:blank", title: "t", status, code: code as never, ...(detail ? { detail } : {}) });

  it("a 403 loopback_only is a loopback-only panel opened over the network", () => {
    expect(classifyMint(problem(403, "loopback_only"))).toBe("loopback_only");
    expect(mintFailureText("loopback_only")).toBe(LOOPBACK_ONLY_PANEL_TEXT);
    expect(LOOPBACK_ONLY_PANEL_TEXT).toMatch(/only opens on the node itself/);
  });
  it("sorts the other answers", () => {
    expect(classifyMint(problem(404, "not_found"))).toBe("not_found");
    expect(classifyMint(ApiError.network(new Error("down")))).toBe("network");
    expect(classifyMint(problem(500, "internal"))).toBe("other");
    expect(classifyMint(new Error("x"))).toBe("other");
    expect(mintFailureText("other", "The panel host is stopping.")).toBe("The panel host is stopping.");
    expect(mintFailureText("other", "  ")).toMatch(/refused/);
    expect(mintFailureText("bad_url")).toMatch(/would not open/);
  });
});

// ------------------------------------------------------------------------------------------ the session

describe("PanelSessionController", () => {
  const session = (n: number): PanelSession => ({ schema: 1, url: `http://127.0.0.1:11190/?p111_ticket=ticket${n}`, expires_in: 30 });
  const src = (u: string) => ticketSrc(u, panel(), LOOPBACK);

  /** A mint that hands out ticket1, ticket2, ... and lets a test hold an answer back. */
  function minter() {
    const calls: Array<{ resolve(s: PanelSession): void; reject(e: unknown): void }> = [];
    return {
      calls,
      mint: () => new Promise<PanelSession>((resolve, reject) => calls.push({ resolve, reject })),
    };
  }

  it("opening mints one ticket and points the frame at it", async () => {
    const m = minter();
    const c = new PanelSessionController({ mint: m.mint, src });
    expect(c.getState().kind).toBe("idle");
    const started = c.start();
    expect(c.getState()).toEqual({ kind: "minting", gen: 1 });
    m.calls[0]!.resolve(session(1));
    await started;
    expect(c.getState()).toEqual({ kind: "ready", gen: 1, src: "http://127.0.0.1:11190/?p111_ticket=ticket1", expiresIn: 30 });
    expect(m.calls).toHaveLength(1);
  });

  it("a reload asks the node for a fresh ticket and rebuilds the frame (new generation, new URL)", async () => {
    let n = 0;
    const seen: number[] = [];
    const c = new PanelSessionController({
      mint: async () => {
        seen.push(++n);
        return session(n);
      },
      src,
    });
    await c.start();
    const first = c.getState();
    await c.reload();
    const second = c.getState();
    expect(seen).toEqual([1, 2]);
    expect(first).toMatchObject({ kind: "ready", gen: 1 });
    expect(second).toMatchObject({ kind: "ready", gen: 2 });
    expect((first as { src: string }).src).not.toBe((second as { src: string }).src);
    expect((second as { src: string }).src).toContain("ticket2");
  });

  it("the newest start wins: an older answer that arrives late is dropped", async () => {
    const m = minter();
    const c = new PanelSessionController({ mint: m.mint, src });
    const a = c.start();
    const b = c.reload();
    m.calls[1]!.resolve(session(2));
    await b;
    m.calls[0]!.resolve(session(1)); // the spent first ticket arrives after the second
    await a;
    expect(c.getState()).toMatchObject({ kind: "ready", gen: 2 });
    expect((c.getState() as { src: string }).src).toContain("ticket2");
  });

  it("a loopback-only refusal is a failed state with the honest sentence, and no frame URL", async () => {
    const c = new PanelSessionController({
      mint: async () => {
        throw new ApiError({ type: "about:blank", title: "Loopback only", status: 403, code: "loopback_only", detail: "x" });
      },
      src,
    });
    await c.start();
    expect(c.getState()).toEqual({ kind: "failed", gen: 1, why: "loopback_only", text: LOOPBACK_ONLY_PANEL_TEXT });
  });

  it("a ticket URL on another host or port never reaches the frame", async () => {
    const c = new PanelSessionController({ mint: async () => ({ schema: 1, url: "http://evil.example:11190/?p111_ticket=x", expires_in: 30 }), src });
    await c.start();
    expect(c.getState()).toMatchObject({ kind: "failed", why: "bad_url" });
    expect(JSON.stringify(c.getState())).not.toContain("evil.example");
  });

  it("tells the view about every failed mint (a 401 goes to the pairing page there)", async () => {
    const errors: unknown[] = [];
    const err = new ApiError({ type: "about:blank", title: "Unauthenticated", status: 401, code: "unauthenticated" });
    const c = new PanelSessionController({ mint: () => Promise.reject(err), src, onError: (e) => errors.push(e) });
    await c.start();
    expect(errors).toEqual([err]);
  });

  it("an answer after the tab closed is dropped, and starting again works", async () => {
    const m = minter();
    const c = new PanelSessionController({ mint: m.mint, src });
    const a = c.start();
    c.dispose();
    m.calls[0]!.resolve(session(1));
    await a;
    expect(c.getState().kind).toBe("minting"); // untouched since the dispose
    const b = c.start();
    m.calls[1]!.resolve(session(2));
    await b;
    expect(c.getState()).toMatchObject({ kind: "ready" });
  });

  it("fresh() mints a separate ticket for another window without touching the frame", async () => {
    let n = 0;
    const c = new PanelSessionController({ mint: async () => session(++n), src });
    await c.start();
    const before = c.getState();
    const url = await c.fresh();
    expect(url).toContain("ticket2");
    expect(c.getState()).toBe(before);
    await expect(new PanelSessionController({ mint: async () => ({ schema: 1, url: "http://evil.example:11190/", expires_in: 30 }), src }).fresh()).rejects.toThrow(/would not open/);
  });

  it("listeners hear every change and can unsubscribe", async () => {
    const c = new PanelSessionController({ mint: async () => session(1), src });
    let heard = 0;
    const off = c.subscribe(() => heard++);
    await c.start();
    expect(heard).toBe(2); // minting, ready
    off();
    await c.reload();
    expect(heard).toBe(2);
  });
});

describe("Open full client", () => {
  const fake = () => {
    const w = { opener: "page" as unknown, closed: false, replaced: [] as string[], document: { body: { textContent: "" as string | null } }, location: { replace: (u: string) => void w.replaced.push(u) }, close: () => void (w.closed = true) };
    return w as OpenedWindow & typeof w;
  };

  it("opens the window first (inside the click), cuts it off from the page, and then points it at a fresh ticket", async () => {
    const win = fake();
    const order: string[] = [];
    const result = await openFullClient(
      async () => {
        order.push(`mint (opener=${String(win.opener)})`);
        return "http://127.0.0.1:11190/?p111_ticket=abc";
      },
      () => {
        order.push("open");
        return win;
      },
    );
    expect(result).toEqual({ kind: "opened" });
    expect(order).toEqual(["open", "mint (opener=null)"]);
    expect(win.replaced).toEqual(["http://127.0.0.1:11190/?p111_ticket=abc"]);
    expect(win.closed).toBe(false);
  });

  it("a blocked pop-up is said, and nothing is minted", async () => {
    let minted = 0;
    const result = await openFullClient(async () => (minted++, "x"), () => null);
    expect(result).toEqual({ kind: "blocked" });
    expect(minted).toBe(0);
  });

  it("a refused mint closes the empty window and says why", async () => {
    const win = fake();
    const result = await openFullClient(
      async () => {
        throw new ApiError({ type: "about:blank", title: "Loopback only", status: 403, code: "loopback_only" });
      },
      () => win,
    );
    expect(result).toMatchObject({ kind: "failed", why: "loopback_only", text: LOOPBACK_ONLY_PANEL_TEXT });
    expect(win.closed).toBe(true);
    expect(win.replaced).toEqual([]);
  });
});

describe("tabs", () => {
  it("arrow keys wrap", () => {
    expect(nextTab(2, 0, 1)).toBe(1);
    expect(nextTab(2, 1, 1)).toBe(0);
    expect(nextTab(2, 0, -1)).toBe(1);
    expect(nextTab(0, 0, 1)).toBe(0);
  });
});

// ------------------------------------------------------------------------------------------ copy

describe("honest copy", () => {
  it("the Meshtastic panel says what it transmits (section 14), whose keys protect it, and which hosts a browser may contact", () => {
    const n = panelNotes("meshtastic")!;
    expect(n.transmits).toMatch(/goes to your radio and out over Meshtastic\. 111 does not see it and does not store it\./);
    expect(n.keys).toBe("Meshtastic protects messages with its own keys; the default channel's key is public. Nothing here is 111-sealed.");
    expect(n.residual).toMatch(/phone/);
    expect(optionalHostsText(["tiles.openfreemap.org", "api.meshtastic.org"])).toMatch(/may contact tiles\.openfreemap\.org, api\.meshtastic\.org while the panel is open/);
    expect(optionalHostsText([])).toBeNull();
    expect(panelNotes("toString")).toBeNull();
    expect(panelNotes("kb")).toBeNull();
  });

  it("the frame's title is the manifest's, else the service's plain name", () => {
    expect(panelTitle({ title: "Meshtastic" }, "Meshtastic web")).toBe("Meshtastic");
    expect(panelTitle({}, "Meshtastic web")).toBe("Meshtastic web");
    expect(panelTitle({ title: "  " }, "Meshtastic web")).toBe("Meshtastic web");
  });
});

// ------------------------------------------------------------------------------------- rendering the tab

type Loc = { protocol: string; hostname: string; port: string };

/** Render with `window.location` as a page on `loc` would see it (the tab reads the origin it is served from). */
function withWindow<T>(loc: Loc, fn: () => T, search = ""): T {
  const g = globalThis as { window?: unknown };
  const had = g.window;
  g.window = {
    location: { ...loc, search, hash: "", pathname: "/" },
    history: { state: null, replaceState: () => undefined },
    localStorage: { getItem: () => null, setItem: () => undefined },
  };
  resetModeForTests();
  try {
    return fn();
  } finally {
    if (had === undefined) delete g.window;
    else g.window = had;
    resetModeForTests();
  }
}

const html = (el: ReactElement, loc: Loc = LOOPBACK, search = ""): string => withWindow(loc, () => renderToStaticMarkup(h(ModeProvider, null, el)), search);

const made: MockAdapter[] = [];
afterEach(() => {
  apiStore.detach();
  apiStore.reset();
  for (const a of made.splice(0)) a.mock.dispose();
});

async function load(scenario: ScenarioName): Promise<MockAdapter> {
  const a = createMockAdapter({ scenario, clock: new FakeClock(Date.parse("2026-10-14T09:00:00Z")), tokenDelayMs: 0 });
  made.push(a);
  apiStore.attach(a);
  await flush(30);
  return a;
}

/** The `meshtastic` scenario after the consented identify: the service is LIVE and carries its panel. */
async function loadLive(): Promise<MockAdapter> {
  const a = await load("meshtastic");
  const board = (await a.devices()).find((d) => d.actions.includes("identify"))!;
  await a.identifyDevice(board.id, { consent: true });
  await flush(30);
  return a;
}

describe("PanelTab states", () => {
  const service = () => apiStore.getState().services!.find((s) => s.id === "meshtastic")!;

  it("secure-context gate: on a LAN address a panel that needs one shows the explanation and the loopback URL, and no frame", async () => {
    await loadLive();
    const p = panel({ needs_secure_context: true });
    const out = html(h(PanelTab, { service: service(), panel: p }), LAN);
    expect(out).not.toContain("<iframe");
    expect(out).toContain("This panel needs a secure page");
    expect(out).toContain(SECURE_CONTEXT_TEXT);
    expect(out).toContain("http://127.0.0.1:11180/#/services/meshtastic");
    expect(out).toContain("open it on the node");
    // the chrome still offers the agent, but not reload / open full client (there is nothing to reload)
    expect(out).toContain("Ask the agent about this panel");
    expect(out).not.toContain(">Reload<");
    expect(out).not.toContain("Open full client");
  });

  it("the same panel on the node itself goes to the frame path: chrome, secure note, the transmit sentence and the hosts", async () => {
    await loadLive();
    const p = panel({ needs_secure_context: true });
    const out = html(h(PanelTab, { service: service(), panel: p }), LOOPBACK);
    expect(out).not.toContain("This panel needs a secure page");
    expect(out).toContain(">Reload<");
    expect(out).toContain("Open full client");
    expect(out).toContain("Ask the agent about this panel");
    expect(out).toContain("Opening the panel"); // the ticket is minted by an effect; nothing is framed before it arrives
    expect(out).toContain("this panel transmits");
    expect(out).toContain("111 does not see it and does not store it");
    expect(out).toContain("tiles.openfreemap.org, api.meshtastic.org");
    expect(out).toContain("USB or Bluetooth from this browser works only on the node itself.");
    expect(out).not.toContain("<iframe"); // no ticket yet: no frame, and never a frame without one
  });

  it("a panel that needs no secure context is offered on a LAN address, with its Web Serial note", async () => {
    await loadLive();
    const out = html(h(PanelTab, { service: service(), panel: panel() }), LAN);
    expect(out).not.toContain("needs a secure page");
    expect(out).toContain(">Reload<");
    expect(out).toContain("USB or Bluetooth from this browser works only on the node itself.");
  });

  it("off and not listening say so, without a frame or the reload buttons", async () => {
    await loadLive();
    const off = html(h(PanelTab, { service: service(), panel: panel({ state: { enabled: false, listening: false, health: "unknown" } }) }));
    expect(off).toContain("Not switched on");
    expect(off).not.toContain("<iframe");
    expect(off).not.toContain(">Reload<");
    const quiet = html(h(PanelTab, { service: service(), panel: panel({ state: { enabled: true, listening: false, health: "up" } }) }));
    expect(quiet).toContain("Not listening yet");
  });

  it("a service that is down behind a panel that still opens says so above the frame", async () => {
    await loadLive();
    const out = html(h(PanelTab, { service: service(), panel: panel({ state: { enabled: true, listening: true, health: "down" } }) }));
    expect(out).toContain("service down");
    expect(out).toContain("The service behind this panel is not answering right now");
  });

  it("the shim's dropped-frame count is surfaced in Advanced only", async () => {
    await loadLive();
    const p = panel({ state: { enabled: true, listening: true, health: "up", dropped: 7 } });
    const simple = html(h(PanelTab, { service: service(), panel: p }));
    expect(simple).not.toContain("Frames dropped");
    const adv = html(h(PanelTab, { service: service(), panel: p }), LOOPBACK, "?mode=advanced");
    expect(adv).toContain("Frames dropped");
    expect(adv).toContain("7 frames were dropped, oldest first");
    expect(adv).toContain("11190");
  });
});

describe("the service page with a panel", () => {
  const page = (id: string, loc: Loc = LOOPBACK, search = "") =>
    html(routed(`/services/${id}`, "/services/:id", ServicesView), loc, search);

  it("offers a Panel tab beside the details on a LIVE service that carries a panel; the frame is not mounted until it is opened", async () => {
    await loadLive();
    const out = page("meshtastic");
    expect(out).toContain('role="tablist"');
    expect(out).toMatch(/role="tab"[^>]*aria-selected="true"[^>]*>Details</);
    expect(out).toMatch(/role="tab"[^>]*aria-selected="false"[^>]*>Panel</);
    expect(out).not.toContain("<iframe");
    expect(out).not.toContain("Opening the panel"); // nothing is minted before the tab is opened
    // the honest copy is on the page itself, not only in the tab
    expect(out).toContain("111 does not see it and does not store it");
    expect(out).toContain("Meshtastic protects messages with its own keys");
    expect(out).toContain("If this node runs on a phone");
    expect(out).toContain("3 Meshtastic nodes heard.");
  });

  it("a service without a panel has no tabs", async () => {
    await load("healthy");
    expect(page("radio")).not.toContain('role="tablist"');
    expect(page("agent")).not.toContain('role="tablist"');
  });

  it("a service that is not LIVE has no Panel tab even if its record names a panel", async () => {
    await load("meshtastic");
    const svc = apiStore.getState().services!.find((s) => s.id === "meshtastic")!;
    expect(servicePageState({ ...svc, state: "unset", panel: panel() }, true).panelTab).toBe(false);
    expect(servicePageState({ ...svc, state: "error", panel: panel() }, true).panelTab).toBe(false);
    expect(servicePageState({ ...svc, state: "live", panel: panel() }, true).panelTab).toBe(true);
    expect(servicePageState({ ...svc, state: "live", panel: null }, true).panelTab).toBe(false);
    expect(page("meshtastic")).not.toContain('role="tablist"');
  });

  it("NOT SET Meshtastic and Messaging ask the agent with their Layer 5 goals", async () => {
    await load("meshtastic");
    const svc = (id: string) => apiStore.getState().services!.find((s) => s.id === id)!;
    expect(servicePageState(svc("meshtastic"), true).handoff).toMatchObject({ goal: "meshtastic.setup", mode: "agent" });
    expect(servicePageState({ ...svc("fabric"), state: "unset" }, true).handoff).toMatchObject({ goal: "messaging.setup" });
    expect(servicePageState({ ...svc("fabric"), state: "error" }, true).handoff).toMatchObject({ goal: "messaging.fix" });
    expect(servicePageState({ ...svc("meshtastic"), state: "error" }, true).handoff).toMatchObject({ goal: "fix" });
    expect(servicePageState(svc("radio"), true).handoff).toMatchObject({ goal: "setup" });
  });
});
