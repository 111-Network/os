// The Panel tab (os/50-messaging/README.md section 13): the iframe's attributes, the ticket URL the frame may open, the
// secure-context gate, the loopback-only copy, a reload that mints a fresh ticket, "Open full client", and the service page
// that carries it (rendered from the mock's `meshtastic` scenario through the real store).
import { createElement as h, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import { createMockAdapter, type MockAdapter, type ScenarioName } from "../src/lib/api/mock";
import { recordPanelsAtLoad, resetPanelsAtLoadForTests } from "../src/lib/api/http";
import { ApiError } from "../src/lib/api/problem";
import { apiStore } from "../src/lib/api/store";
import type { HostedPanel, PanelSession } from "../src/lib/api/types";
import { ModeProvider, resetModeForTests } from "../src/lib/mode";
import ServicesView from "../src/views/services";
import { PanelFrame, PanelHelp, PanelPolicyNotice, PanelTab } from "../src/views/services/panel";
import {
  allowAttr,
  checkMintReply,
  classifyMint,
  clampExpiresIn,
  cspListedAtLoad,
  dashboardPort,
  isPanelFrameBlock,
  LOOPBACK_ONLY_PANEL_TEXT,
  loopbackDashboardUrl,
  mintFailureText,
  optionalHostsText,
  PANEL_CSP_TEXT,
  PANEL_EXPIRED_HELP,
  PANEL_PORT_MAX,
  PANEL_PORT_MIN,
  PANEL_SANDBOX,
  panelGate,
  panelNodeAddress,
  panelNotes,
  panelTitle,
  PanelUrlError,
  SECURE_CONTEXT_TEXT,
  ticketSrc,
  watchFrameBlock,
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

  it("a string that is not a URL is never framed as a path on the panel origin", () => {
    // A relative reference resolves against the panel origin (documented): this is a path on 127.0.0.1:11190 and nothing else,
    // never the raw string as an address. The node's own reply is shape-checked before it gets here (checkMintReply).
    expect(ok("not a url at all %%%")).toBe("http://127.0.0.1:11190/not%20a%20url%20at%20all%20%%%");
    for (const bad of [undefined, null, 0, 11190, {}, [], ["http://127.0.0.1:11190/"], true, ""] as unknown[]) {
      expect(ticketSrc(bad, panel(), LOOPBACK), String(bad)).toBeNull();
    }
    expect(ticketSrc("   ", panel(), LOOPBACK)).toBeNull();
  });

  it("host tricks: userinfo before a host, protocol-relative, backslash and mixed-slash forms all fail closed", () => {
    for (const bad of [
      "http://127.0.0.1:11190@evil.com/",
      "http://127.0.0.1:11190@evil.com:11190/",
      "//evil.com:11190/?p111_ticket=abc",
      "//127.0.0.1.evil.com:11190/",
      "\\\\evil.com:11190\\",
      "http:\\\\evil.com:11190\\x",
      "http:/\\evil.com:11190/",
      "/\\evil.com:11190/",
      "http://evil.com\\@127.0.0.1:11190/",
      "http://127.0.0.1:11190.evil.com/",
      "http://[::ffff:127.0.0.1]:11190/",
      "http://localhost:11190/",
      "http://127.1:11190/",
      "http://2130706433:11190/",
      "http://127.0.0.1.:11190/",
      "https://127.0.0.1:11190@evil.com",
      "about:blank",
      "blob:http://127.0.0.1:11190/uuid",
      "javascript:alert(1)",
      "java\tscript:alert(1)",
      "data:text/html,<script>1</script>",
    ]) {
      const got = ticketSrc(bad, panel(), LOOPBACK);
      // either refused outright, or rebuilt onto the one origin this page computes (never anything else)
      expect(got === null || got.startsWith("http://127.0.0.1:11190/"), `${bad} -> ${got}`).toBe(true);
      if (got !== null) expect(got, bad).not.toMatch(/evil/);
    }
    // the ones that name another host or carry credentials are refused, not merely normalised
    for (const refused of ["http://127.0.0.1:11190@evil.com/", "//evil.com:11190/?p111_ticket=abc", "http://evil.com\\@127.0.0.1:11190/", "http://127.0.0.1:11190.evil.com/", "http://localhost:11190/"]) {
      expect(ticketSrc(refused, panel(), LOOPBACK), refused).toBeNull();
    }
  });

  it("a ticket on the https page is rebuilt onto the https origin, never framed as plain http", () => {
    const p = panel();
    // the node answers a URL on the right host and slot but with the other scheme: the frame still gets the page's own scheme
    expect(ticketSrc("http://node.example:11190/?p111_ticket=abc#x", p, HTTPS)).toBe("https://node.example:11190/?p111_ticket=abc");
    expect(ticketSrc("https://node.example:11190/?p111_ticket=abc", p, HTTPS)).toBe("https://node.example:11190/?p111_ticket=abc");
    // and on the http page an https URL is rebuilt onto http
    expect(ticketSrc("https://127.0.0.1:11190/?p111_ticket=abc", p, LOOPBACK)).toBe("http://127.0.0.1:11190/?p111_ticket=abc");
  });

  it("the panel's slot must be inside the contract's range and never the dashboard's own port", () => {
    expect([PANEL_PORT_MIN, PANEL_PORT_MAX]).toEqual([11190, 11229]);
    // the dashboard's own origin: a frame with allow-same-origin there would be a full sandbox escape
    expect(ticketSrc("http://127.0.0.1:11180/?t", panel({ port: 11180 }), LOOPBACK)).toBeNull();
    expect(ticketSrc("/?t", panel({ port: 11180 }), LOOPBACK)).toBeNull();
    // in range, but it IS the port the dashboard is on (a dashboard moved onto a panel slot)
    expect(ticketSrc("http://127.0.0.1:11195/?t", panel({ port: 11195 }), { ...LOOPBACK, port: "11195" })).toBeNull();
    // the default port counts when the address bar shows none
    expect(dashboardPort({ protocol: "http:", hostname: "h", port: "" })).toBe("80");
    expect(dashboardPort({ protocol: "https:", hostname: "h", port: "" })).toBe("443");
    expect(dashboardPort(LOOPBACK)).toBe("11180");
    // outside the range, in either direction, or not a whole number
    for (const port of [0, 80, 11189, 11230, 65535, -1, 11190.5, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(ticketSrc(`http://127.0.0.1:${port}/?t`, panel({ port }), LOOPBACK), String(port)).toBeNull();
    }
    expect(ticketSrc("http://127.0.0.1:11190/?t", panel({ port: PANEL_PORT_MIN }), LOOPBACK)).not.toBeNull();
    expect(ticketSrc("http://127.0.0.1:11229/?t", panel({ port: PANEL_PORT_MAX }), LOOPBACK)).not.toBeNull();
    expect(ticketSrc("http://127.0.0.1:11190/?t", panel({ port: "11190" as unknown as number }), LOOPBACK)).toBeNull();
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


// ------------------------------------------------------------------------------------------ the mint reply

describe("the mint reply is data, not a promise", () => {
  const src = (u: string) => ticketSrc(u, panel(), LOOPBACK);
  const ticket = "http://127.0.0.1:11190/?p111_ticket=t1";

  it("expires_in is a whole number of seconds from 1 to 300; anything else is the documented 30", () => {
    expect(clampExpiresIn(30)).toBe(30);
    expect(clampExpiresIn(30.9)).toBe(30);
    expect(clampExpiresIn(0)).toBe(1);
    expect(clampExpiresIn(-5)).toBe(1);
    expect(clampExpiresIn(1e9)).toBe(300);
    for (const odd of [undefined, null, "30", Number.NaN, Number.POSITIVE_INFINITY, {}, []] as unknown[]) expect(clampExpiresIn(odd), String(odd)).toBe(30);
  });

  it("a reply needs a non-empty string url", () => {
    expect(checkMintReply({ schema: 1, url: ticket, expires_in: 30 })).toEqual({ url: ticket, expiresIn: 30 });
    for (const bad of [undefined, null, 0, "x", {}, { url: 7 }, { url: {} }, { url: ["a"] }, { url: "" }, { url: "  " }, { schema: 1, expires_in: 30 }] as unknown[]) {
      expect(checkMintReply(bad), JSON.stringify(bad)).toBeNull();
    }
  });

  it("start(): a reply without a url (or with a non-string one) is a bad_url failure, never a frame at /undefined", async () => {
    for (const reply of [{ schema: 1, expires_in: 30 }, { schema: 1, url: { href: ticket }, expires_in: 30 }, { schema: 1, url: 42, expires_in: 30 }, { schema: 1, url: "", expires_in: 30 }]) {
      const c = new PanelSessionController({ mint: async () => reply as unknown as PanelSession, src });
      await c.start();
      expect(c.getState(), JSON.stringify(reply)).toMatchObject({ kind: "failed", why: "bad_url" });
      expect(JSON.stringify(c.getState())).not.toMatch(/undefined|object/);
    }
  });

  it("start(): a missing, absurd or non-numeric expires_in never reaches the slow-frame timer as NaN", async () => {
    for (const [reply, want] of [
      [{ schema: 1, url: ticket }, 30],
      [{ schema: 1, url: ticket, expires_in: "soon" }, 30],
      [{ schema: 1, url: ticket, expires_in: 0 }, 1],
      [{ schema: 1, url: ticket, expires_in: 86_400 }, 300],
    ] as const) {
      const c = new PanelSessionController({ mint: async () => reply as unknown as PanelSession, src });
      await c.start();
      const state = c.getState();
      expect(state, JSON.stringify(reply)).toMatchObject({ kind: "ready", expiresIn: want });
      expect(Number.isInteger((state as { expiresIn: number }).expiresIn)).toBe(true);
    }
  });

  it("fresh(): a malformed reply is a PanelUrlError the full-client path reports as bad_url", async () => {
    const c = new PanelSessionController({ mint: async () => ({ schema: 1 }) as unknown as PanelSession, src });
    await expect(c.fresh()).rejects.toBeInstanceOf(PanelUrlError);
    expect(classifyMint(new PanelUrlError())).toBe("bad_url");
    const win = { opener: "page" as unknown, closed: false, replaced: [] as string[], document: { body: { textContent: "" as string | null } }, location: { replace: (u: string) => void win.replaced.push(u) }, close: () => void (win.closed = true) };
    const result = await openFullClient(() => c.fresh(), () => win as OpenedWindow);
    expect(result).toMatchObject({ kind: "failed", why: "bad_url", text: mintFailureText("bad_url") });
    expect(win.closed).toBe(true);
    expect(win.replaced).toEqual([]);
  });

  it("fresh(): a failed mint reaches onError too, so a 401 from Open full client goes to the pairing page", async () => {
    const errors: unknown[] = [];
    const err = new ApiError({ type: "about:blank", title: "Unauthenticated", status: 401, code: "unauthenticated" });
    const c = new PanelSessionController({ mint: () => Promise.reject(err), src, onError: (e) => errors.push(e) });
    await expect(c.fresh()).rejects.toBe(err);
    expect(errors).toEqual([err]);
    // a reply the page would not frame is not an API error: onError is not told
    const c2 = new PanelSessionController({ mint: async () => ({ schema: 1, url: "http://evil.example:11190/", expires_in: 30 }), src, onError: (e) => errors.push(e) });
    await expect(c2.fresh()).rejects.toBeInstanceOf(PanelUrlError);
    expect(errors).toEqual([err]);
  });
});

// ----------------------------------------------------------------------- the page's own policy (section 13)

describe("a panel switched on after the page loaded is refused by the page's own CSP", () => {
  const ORIGIN = "http://192.168.1.42:11190";
  const ticketUrl = `${ORIGIN}/?p111_ticket=00112233445566778899aabbccddeeff`;
  /** A `securitypolicyviolation` event as a browser sends it, on a real EventTarget. */
  const violation = (init: { effectiveDirective?: string; violatedDirective?: string; blockedURI?: string }): Event => Object.assign(new Event("securitypolicyviolation", { bubbles: true }), init);

  it("the frame-type directives on the panel's origin are the page's own refusal", () => {
    for (const directive of ["frame-src", "child-src", "default-src"]) {
      expect(isPanelFrameBlock({ effectiveDirective: directive, blockedURI: ticketUrl }, ORIGIN), directive).toBe(true);
    }
    // older browsers only fill violatedDirective ("frame-src 'self'")
    expect(isPanelFrameBlock({ violatedDirective: "frame-src 'self' http://192.168.1.99:11190", blockedURI: ticketUrl }, ORIGIN)).toBe(true);
    // what headless Chrome sent for a framed http://127.0.0.1:19190/?p111_ticket=abc#frag under a CSP with no frame-src (checked by
    // hand against a real page): the origin only in blockedURI, the ticket stripped by the browser itself, fired at the document
    expect(isPanelFrameBlock({ effectiveDirective: "frame-src", violatedDirective: "frame-src", blockedURI: "http://127.0.0.1:19190" }, "http://127.0.0.1:19190")).toBe(true);
    // the origin only is compared: path and query (the ticket) do not matter, and a bare origin is enough
    expect(isPanelFrameBlock({ effectiveDirective: "frame-src", blockedURI: ORIGIN }, ORIGIN)).toBe(true);
    expect(isPanelFrameBlock({ effectiveDirective: "frame-src", blockedURI: `${ORIGIN}/other` }, `${ORIGIN}/`)).toBe(true);
  });

  it("anything else is not: another origin, another port, another directive, an inline or unparsable blocked uri", () => {
    expect(isPanelFrameBlock({ effectiveDirective: "frame-src", blockedURI: "http://192.168.1.42:11191/" }, ORIGIN)).toBe(false);
    expect(isPanelFrameBlock({ effectiveDirective: "frame-src", blockedURI: "http://192.168.1.43:11190/" }, ORIGIN)).toBe(false);
    expect(isPanelFrameBlock({ effectiveDirective: "frame-src", blockedURI: "https://192.168.1.42:11190/" }, ORIGIN)).toBe(false);
    for (const directive of ["script-src", "script-src-elem", "style-src", "img-src", "connect-src", "frame-ancestors", "form-action", "base-uri", ""]) {
      expect(isPanelFrameBlock({ effectiveDirective: directive, blockedURI: ticketUrl }, ORIGIN), directive).toBe(false);
    }
    for (const blockedURI of ["inline", "eval", "", "data", "about", "not a url", undefined, null, 7] as unknown[]) {
      expect(isPanelFrameBlock({ effectiveDirective: "frame-src", blockedURI } as never, ORIGIN), String(blockedURI)).toBe(false);
    }
    expect(isPanelFrameBlock({ effectiveDirective: "frame-src", blockedURI: ticketUrl }, "not an origin")).toBe(false);
  });

  it("watchFrameBlock hears the document's violation event, once per refusal, and only for the panel's origin", () => {
    const doc = new EventTarget();
    let blocks = 0;
    const off = watchFrameBlock(doc, ORIGIN, () => blocks++);
    doc.dispatchEvent(violation({ effectiveDirective: "script-src", blockedURI: "inline" })); // unrelated
    doc.dispatchEvent(violation({ effectiveDirective: "frame-src", blockedURI: "http://192.168.1.42:11191/" })); // another slot
    expect(blocks).toBe(0);
    doc.dispatchEvent(violation({ effectiveDirective: "frame-src", blockedURI: ticketUrl }));
    expect(blocks).toBe(1);
    doc.dispatchEvent(new Event("something-else"));
    expect(blocks).toBe(1);
    off();
    doc.dispatchEvent(violation({ effectiveDirective: "frame-src", blockedURI: ticketUrl }));
    expect(blocks).toBe(1); // unsubscribed
  });

  it("the ticket in the blocked uri is never kept: the callback gets nothing", () => {
    const doc = new EventTarget();
    const heard: unknown[][] = [];
    watchFrameBlock(doc, ORIGIN, (...args: unknown[]) => void heard.push(args));
    doc.dispatchEvent(violation({ effectiveDirective: "frame-src", blockedURI: ticketUrl }));
    expect(heard).toEqual([[]]);
  });

  it("warns ahead only for a panel that the page's first GET /panels said was not enabled and listening", () => {
    const at = new Map([["meshtastic", false], ["kb", true]]);
    expect(cspListedAtLoad("meshtastic", at)).toBe(false);
    expect(cspListedAtLoad("kb", at)).toBe(true);
    expect(cspListedAtLoad("unknown-panel", at)).toBeNull();
    expect(cspListedAtLoad("meshtastic", null)).toBeNull();
    expect(cspListedAtLoad("meshtastic", undefined)).toBeNull();
  });

  it("the first panel list of a page load is remembered once; enabled and listening both count", () => {
    resetPanelsAtLoadForTests();
    recordPanelsAtLoad([
      { id: "meshtastic", state: { enabled: true, listening: true, health: "up" } },
      { id: "kb", state: { enabled: true, listening: false, health: "up" } },
      { id: "fabric", state: { enabled: false, listening: false, health: "unknown" } },
    ]);
    recordPanelsAtLoad([{ id: "fabric", state: { enabled: true, listening: true, health: "up" } }]); // a later list does not rewrite history
    return import("../src/lib/api/http").then(({ panelsSeenAtLoad }) => {
      expect([...panelsSeenAtLoad()!]).toEqual([["meshtastic", true], ["kb", false], ["fabric", false]]);
      resetPanelsAtLoadForTests();
      expect(panelsSeenAtLoad()).toBeNull();
    });
  });

  it("the notice says what happened in the page's own words and offers reload; the observed one is an alert", () => {
    const seen = renderToStaticMarkup(h(PanelPolicyNotice, { observed: true, onReloadPage: () => undefined }));
    expect(seen).toContain(PANEL_CSP_TEXT);
    expect(PANEL_CSP_TEXT).toMatch(/loaded before the panel was switched on/);
    expect(PANEL_CSP_TEXT).toMatch(/Reload the page/);
    expect(seen).toContain("Reload this page");
    expect(seen).toContain('role="alert"');
    expect(seen).not.toContain("<iframe");
    const warned = renderToStaticMarkup(h(PanelPolicyNotice, { observed: false, onReloadPage: () => undefined }));
    expect(warned).toContain('role="status"');
  });
});

describe("the Panel tab when the page's policy is known not to list the panel", () => {
  const service = () => apiStore.getState().services!.find((s) => s.id === "meshtastic")!;
  afterEach(() => resetPanelsAtLoadForTests());

  it("a panel that was not on when the page loaded: no ticket is minted, no frame, the reload notice and Open full client", async () => {
    await loadLive();
    recordPanelsAtLoad([{ id: "meshtastic", state: { enabled: false, listening: false, health: "unknown" } }]);
    const out = html(h(PanelTab, { service: service(), panel: panel() }), LOOPBACK);
    expect(out).toContain('data-testid="panel-csp"');
    expect(out).toContain(PANEL_CSP_TEXT);
    expect(out).toContain("Reload this page");
    expect(out).toContain("Open full client");
    expect(out).not.toContain(">Reload<"); // a new ticket cannot get past the page's policy
    expect(out).not.toContain("<iframe");
    expect(out).not.toContain("Opening the panel"); // nothing is being minted
    expect(out).not.toContain("If the client asks for a node address");
    // the honest copy about what the panel transmits stays
    expect(out).toContain("111 does not see it and does not store it");
  });

  it("a panel that was on when the page loaded takes the normal path", async () => {
    await loadLive();
    recordPanelsAtLoad([{ id: "meshtastic", state: { enabled: true, listening: true, health: "up" } }]);
    const out = html(h(PanelTab, { service: service(), panel: panel() }), LOOPBACK);
    expect(out).not.toContain('data-testid="panel-csp"');
    expect(out).toContain(">Reload<");
    expect(out).toContain("Opening the panel");
  });

  it("with no first answer the page does not guess", async () => {
    await loadLive();
    const out = html(h(PanelTab, { service: service(), panel: panel() }), LOOPBACK);
    expect(out).not.toContain('data-testid="panel-csp"');
    expect(out).toContain(">Reload<");
  });
});

// ----------------------------------------------------------------- what the panel's own page cannot say

describe("§13 'session expired' and §14 'node address' help under the chrome", () => {
  it("the node address is the panel's own host:port, for the web client's connect dialog", () => {
    expect(panelNodeAddress(panel(), LOOPBACK)).toBe("127.0.0.1:11190");
    expect(panelNodeAddress(panel(), LAN)).toBe("192.168.1.42:11190");
    expect(panelNodeAddress(panel(), MDNS)).toBe("111.local:11190");
    expect(panelNodeAddress(panel(), HTTPS)).toBe("node.example:11190");
    expect(panelNodeAddress(panel({ port: 11191 }), { protocol: "http:", hostname: "::1", port: "11180" })).toBe("[::1]:11191");
  });

  it("is shown with a copy button, and says Reload fixes an expired session", () => {
    const out = renderToStaticMarkup(h(PanelHelp, { panel: panel(), env: LAN }));
    expect(out).toContain("If the client asks for a node address, use");
    expect(out).toContain('data-testid="panel-node-address"');
    expect(out).toContain(">192.168.1.42:11190<");
    expect(out).toContain("Copy the node address for the panel&#x27;s connect dialog");
    expect(out).toContain(PANEL_EXPIRED_HELP);
    expect(PANEL_EXPIRED_HELP).toMatch(/session expired/);
    expect(PANEL_EXPIRED_HELP).toMatch(/press Reload/);
  });

  it("the framed tab carries it (on the address this page was opened on), the gated tabs do not", async () => {
    const live = async () => {
      const a = await load("meshtastic");
      const board = (await a.devices()).find((d) => d.actions.includes("identify"))!;
      await a.identifyDevice(board.id, { consent: true });
      await flush(30);
    };
    await live();
    const service = apiStore.getState().services!.find((s) => s.id === "meshtastic")!;
    const framed = html(h(PanelTab, { service, panel: panel() }), LAN);
    expect(framed).toContain("If the client asks for a node address, use");
    expect(framed).toContain("192.168.1.42:11190");
    expect(framed).toContain(PANEL_EXPIRED_HELP);
    const off = html(h(PanelTab, { service, panel: panel({ state: { enabled: false, listening: false, health: "unknown" } }) }), LAN);
    expect(off).not.toContain("If the client asks for a node address");
  });
});
