#!/usr/bin/env node
// Self-test of the lint's rules: a clean synthetic recording passes every rule, and each hostile mutation of it is
// flagged by exactly the rule that owns it. No browser, no daemon. Run by panel-lint.sh before the real run, so a
// green lint can never be a rule that cannot fail.
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DEFAULT_SANDBOX, evaluate } from "./lib/rules.mjs";
import { classifyUrl, lintBundle } from "./lib/bundle.mjs";

let FAILS = 0;
const say = (level, text) => {
  if (level === "FAIL") FAILS++;
  process.stdout.write(`${level}: selftest: ${text}\n`);
};

const D = "http://192.168.77.5:11180";
const P = "http://192.168.77.5:11190";
const T = "0123456789abcdef0123456789abcdef";
const COOKIE = "p111_meshtastic=" + "ab".repeat(16);
const SEC = {
  "content-security-policy": "frame-ancestors http://192.168.77.5:11180 http://192.168.77.5:80",
  "x-content-type-options": "nosniff",
  "referrer-policy": "no-referrer",
};
let n = 0;
const hop = (p) => ({
  key: `s|${++n}`,
  hop: 0,
  t: n,
  phase: "panel",
  label: "dashboard",
  sid: "s",
  requestId: String(n),
  method: "GET",
  url: `${P}/`,
  docUrl: `${P}/`,
  type: "Other",
  frameId: "F1",
  initiator: {},
  headers: {},
  rawRequestHeaders: {},
  rawResponseHeaders: { ...SEC },
  response: { status: 200 },
  status: 200,
  failed: null,
  redirectedTo: null,
  hasExtraInfo: true,
  ...p,
  origin: new URL(p.url || `${P}/`).origin,
  docOrigin: new URL(p.docUrl || p.url || `${P}/`).origin,
});

function clean() {
  const panelHops = [
    hop({ url: `${P}/?p111_ticket=${T}`, type: "Document", status: 303, redirectedTo: `${P}/`, rawResponseHeaders: { ...SEC, "set-cookie": `${COOKIE}; HttpOnly; SameSite=Strict; Path=/` } }),
    hop({ url: `${P}/`, type: "Document" }),
    hop({ url: `${P}/app.js`, type: "Script" }),
    hop({ url: `${P}/style.css`, type: "Stylesheet" }),
    hop({ url: `${P}/api/v1/toradio`, method: "PUT", type: "Fetch", rawResponseHeaders: { ...SEC } }),
    ...Array.from({ length: 6 }, () => hop({ url: `${P}/api/v1/fromradio?all=true`, type: "Fetch", rawResponseHeaders: { ...SEC, "content-length": "42", "content-type": "application/x-protobuf" } })),
  ];
  const dashSec = { "content-security-policy": `default-src 'self'; frame-ancestors 'none'; frame-src ${P}`, "x-frame-options": "DENY" };
  const dashHops = [
    hop({ url: `${D}/`, type: "Document", phase: "setup", docUrl: `${D}/`, frameId: "M", rawResponseHeaders: { ...dashSec } }),
    hop({ url: `${D}/api/v1/panels/meshtastic/session`, method: "POST", type: "Fetch", docUrl: `${D}/`, frameId: "M", rawResponseHeaders: {} }),
  ];
  const probe = (name, kind, extra, hopExtra) => ({
    name,
    kind,
    title: name,
    fromOrigin: "http://192.168.77.5:11300",
    js: { ok: false, err: "TypeError: Failed to fetch" },
    statuses: [403],
    hops: [hop({ url: `${P}/api/v1/fromradio?all=true`, phase: `probe-${name}`, docUrl: "http://192.168.77.5:11300/", status: 403, rawRequestHeaders: { "sec-fetch-site": "same-site", cookie: COOKIE }, ...hopExtra })],
    expectFetchSite: "same-site",
    cookieSent: true,
    ...extra,
  });
  const probes = [
    probe("same-site-get", "refused", {}, {}),
    probe("cross-site-get", "refused", { expectFetchSite: "cross-site", cookieSent: false, statuses: [401, 403] }, { rawRequestHeaders: { "sec-fetch-site": "cross-site" } }),
    probe("drive-by-post", "refused", { opaque: true, js: { ok: true, type: "opaque", status: 0 }, expectMethod: "POST" }, { method: "POST" }),
    {
      name: "preflight",
      kind: "preflight",
      title: "non-simple PUT",
      fromOrigin: "http://192.168.77.5:11300",
      js: { ok: false, err: "TypeError: Failed to fetch" },
      hops: [
        hop({ url: `${P}/api/v1/toradio`, method: "OPTIONS", phase: "probe-preflight", status: 403, rawRequestHeaders: { origin: "http://192.168.77.5:11300" } }),
        hop({ url: `${P}/api/v1/toradio`, method: "PUT", phase: "probe-preflight", status: 0, failed: { errorText: "net::ERR_FAILED" } }),
      ],
    },
    {
      name: "dashboard-fetch",
      kind: "csp-or-refused",
      title: "fetch from the dashboard origin",
      fromOrigin: D,
      js: { ok: false, err: "TypeError: Failed to fetch" },
      cspMessages: ["security: Refusing to connect to 'http://192.168.77.5:11190/api/v1/fromradio?all=true' because it violates the following Content Security Policy directive: \"connect-src 'self'\"."],
      hops: [],
    },
  ];
  return {
    scenario: "lan",
    panelId: "meshtastic",
    dashboardOrigin: D,
    panelOrigin: P,
    nodeHost: "192.168.77.5",
    optionalHosts: ["tiles.openfreemap.org", "api.meshtastic.org"],
    manifest: { allow: ["serial", "bluetooth"] },
    reqs: [...dashHops, ...panelHops, ...probes.flatMap((p) => p.hops)],
    webSockets: [],
    popups: [],
    panelFrameIds: ["F1"],
    ticket: T,
    iframes: [{ src: `${P}/?p111_ticket=${T}`, sandbox: DEFAULT_SANDBOX.join(" "), allow: "serial; bluetooth", referrerpolicy: "no-referrer" }],
    cookies: [{ name: "p111_meshtastic", domain: "192.168.77.5", path: "/", httpOnly: true, secure: false, sameSite: "Strict", session: true }],
    frameUrls: { panel: `${P}/`, dashboard: `${D}/#/services/meshtastic` },
    panelDom: { cookie: "", state: "connected", nodes: "3", board: "Lint board", polls: "12", frames: "3", errors: "0", put: "ok", host: "192.168.77.5:11190" },
    probes,
  };
}

const clone = (o) => structuredClone(o);
const fails = (ctx) => evaluate(ctx).filter((r) => r.level === "FAIL").map((r) => r.id);

// ---- the clean recording passes
{
  const f = fails(clean());
  if (f.length === 0) say("ok", `a clean recording passes every rule (${evaluate(clean()).filter((r) => r.level === "ok").length} ok)`);
  else say("FAIL", `the clean recording is flagged by ${f.join(", ")}`);
}

// ---- each mutation is caught by the owning rule
const mutations = [
  ["a font CDN request from the panel", "net.own-origin-only", (c) => c.reqs.push(hop({ url: "https://fonts.gstatic.com/s/inter.woff2", type: "Font" }))],
  ["an analytics beacon from the panel", "net.own-origin-only", (c) => c.reqs.push(hop({ url: "https://analytics.example.org/collect", method: "POST", type: "Ping" }))],
  ["a request from the panel to another port on the node", "net.own-origin-only", (c) => c.reqs.push(hop({ url: "http://192.168.77.5:11230/", type: "Fetch" }))],
  ["a request to a host outside the node anywhere in the run", "net.hosts", (c) => c.reqs.push(hop({ url: "https://cdn.example.net/x.js", type: "Script", docUrl: `${D}/`, frameId: "M" }))],
  ["a window opened by the panel", "net.no-popups", (c) => (c.popups = [{ targetId: "T", url: "https://ads.example.net/", openerId: "X", phase: "panel" }])],
  ["a WebSocket", "net.no-websocket", (c) => c.webSockets.push({ url: `ws://${new URL(P).host}/ws` })],
  ["a request of type WebSocket", "net.no-websocket", (c) => c.reqs.push(hop({ url: `${P}/ws`, type: "WebSocket" }))],
  ["the panel requesting the dashboard", "net.no-dashboard", (c) => c.reqs.push(hop({ url: `${D}/api/v1/status`, type: "Fetch", docUrl: `${P}/` }))],
  ["Access-Control-Allow-Origin on a panel response", "cors.absent", (c) => (c.reqs.find((h) => h.url.endsWith("/app.js")).rawResponseHeaders["access-control-allow-origin"] = "*")],
  ["Access-Control-Allow-Credentials on a refusal", "cors.absent", (c) => (c.probes[0].hops[0].rawResponseHeaders = { ...SEC, "access-control-allow-credentials": "true" })],
  ["no raw headers at all", "cors.absent", (c) => c.reqs.forEach((h) => (h.hasExtraInfo = false))],
  ["X-Frame-Options left on a panel response", "headers.framing", (c) => (c.reqs.find((h) => h.url.endsWith("/style.css")).rawResponseHeaders["x-frame-options"] = "DENY")],
  ["frame-ancestors missing", "headers.framing", (c) => (c.reqs.find((h) => h.url.endsWith("/app.js")).rawResponseHeaders["content-security-policy"] = "default-src 'self'")],
  ["frame-ancestors names a stranger", "headers.framing", (c) => (c.reqs.find((h) => h.url.endsWith("/app.js")).rawResponseHeaders["content-security-policy"] = "frame-ancestors http://evil.example:80")],
  ["frame-ancestors * ", "headers.framing", (c) => (c.reqs.find((h) => h.url.endsWith("/app.js")).rawResponseHeaders["content-security-policy"] = "frame-ancestors *")],
  ["no nosniff", "headers.framing", (c) => delete c.reqs.find((h) => h.url.endsWith("/app.js")).rawResponseHeaders["x-content-type-options"]],
  ["the ticket in a second URL", "ticket.once", (c) => c.reqs.push(hop({ url: `${P}/app.js?p111_ticket=${T}`, type: "Script" }))],
  ["the ticket in a Referer", "ticket.once", (c) => (c.reqs.find((h) => h.url.endsWith("/app.js")).rawRequestHeaders.referer = `${P}/?p111_ticket=${T}`)],
  ["the ticket kept in the final frame URL", "ticket.once", (c) => (c.frameUrls.panel = `${P}/?p111_ticket=${T}`)],
  ["the swap redirecting with the ticket", "ticket.once", (c) => (c.reqs[2].redirectedTo = `${P}/?p111_ticket=${T}`)],
  ["no ticket learned", "ticket.once", (c) => (c.ticket = "")],
  ["cookie without HttpOnly", "cookie.set-cookie", (c) => (c.reqs[2].rawResponseHeaders["set-cookie"] = `${COOKIE}; SameSite=Strict; Path=/`)],
  ["cookie with SameSite=Lax", "cookie.set-cookie", (c) => (c.reqs[2].rawResponseHeaders["set-cookie"] = `${COOKIE}; HttpOnly; SameSite=Lax; Path=/`)],
  ["cookie with a Domain", "cookie.set-cookie", (c) => (c.reqs[2].rawResponseHeaders["set-cookie"] = `${COOKIE}; HttpOnly; SameSite=Strict; Path=/; Domain=192.168.77.5`)],
  ["cookie with Max-Age", "cookie.set-cookie", (c) => (c.reqs[2].rawResponseHeaders["set-cookie"] = `${COOKIE}; HttpOnly; SameSite=Strict; Path=/; Max-Age=3600`)],
  ["cookie jar entry not HttpOnly", "cookie.jar", (c) => (c.cookies[0].httpOnly = false)],
  ["cookie jar entry with a leading-dot domain", "cookie.jar", (c) => (c.cookies[0].domain = ".192.168.77.5")],
  ["cookie jar entry SameSite None", "cookie.jar", (c) => (c.cookies[0].sameSite = "None")],
  ["the session cookie readable from the panel's JS", "cookie.js-blind", (c) => (c.panelDom.cookie = "p111_meshtastic=aa")],
  ["a second iframe", "iframe.only-the-panel", (c) => c.iframes.push({ src: "http://192.168.77.5:11191/", sandbox: "", allow: "" })],
  ["the iframe pointing at another origin", "iframe.only-the-panel", (c) => (c.iframes[0].src = "https://evil.example/")],
  ["allow-top-navigation in the sandbox", "iframe.sandbox", (c) => (c.iframes[0].sandbox += " allow-top-navigation")],
  ["no sandbox attribute", "iframe.sandbox", (c) => (c.iframes[0].sandbox = null)],
  ["allow-popups-to-escape-sandbox dropped from default? (a token removed)", "iframe.sandbox", (c) => (c.iframes[0].sandbox = c.iframes[0].sandbox.replace("allow-forms", "").trim())],
  ["allow widened to *", "iframe.allow", (c) => (c.iframes[0].allow = "serial *; bluetooth")],
  ["allow gained camera", "iframe.allow", (c) => (c.iframes[0].allow = "serial; bluetooth; camera")],
  ["frame-src with a second origin", "csp.frame-src", (c) => (c.reqs[0].rawResponseHeaders["content-security-policy"] += " http://192.168.77.5:11191")],
  ["no frame-src at all", "csp.frame-src", (c) => (c.reqs[0].rawResponseHeaders["content-security-policy"] = "default-src 'self'; frame-ancestors 'none'")],
  ["frame-src *", "csp.frame-src", (c) => (c.reqs[0].rawResponseHeaders["content-security-policy"] = "default-src 'self'; frame-ancestors 'none'; frame-src *")],
  ["the dashboard losing frame-ancestors 'none'", "csp.frame-ancestors-none", (c) => (c.reqs[0].rawResponseHeaders["content-security-policy"] = `default-src 'self'; frame-src ${P}`)],
  ["a cross-origin GET answered 200", "probe.same-site-get", (c) => ((c.probes[0].hops[0].status = 200), (c.probes[0].js = { ok: true, status: 200, type: "cors" }))],
  ["a cross-origin GET refused but with CORS headers", "probe.same-site-get", (c) => (c.probes[0].hops[0].rawResponseHeaders["access-control-allow-origin"] = "http://192.168.77.5:11300")],
  ["a cross-site GET that carried the cookie", "probe.cross-site-get", (c) => (c.probes[1].hops[0].rawRequestHeaders.cookie = COOKIE)],
  ["the probe never reaching the panel", "probe.cross-site-get", (c) => (c.probes[1].hops = [])],
  ["a drive-by POST answered 200", "probe.drive-by-post", (c) => (c.probes[2].hops[0].status = 200)],
  ["a cross-site GET answered 200", "probe.cross-site-get", (c) => (c.probes[1].hops[0].status = 200)],
  ["a same-site GET without the fetch-metadata header the loopback expects", "probe.same-site-get", (c) => delete c.probes[0].hops[0].rawRequestHeaders["sec-fetch-site"]],
  ["the page reading a drive-by POST answer (not opaque)", "probe.drive-by-post", (c) => (c.probes[2].js = { ok: true, type: "basic", status: 403 })],
  ["a preflight answered with CORS headers", "probe.preflight", (c) => (c.probes[3].hops[0].rawResponseHeaders["access-control-allow-origin"] = "*")],
  ["a preflight answered 204", "probe.preflight", (c) => (c.probes[3].hops[0].status = 204)],
  ["the PUT actually sent after the preflight", "probe.preflight", (c) => (c.probes[3].hops[1].status = 200)],
  ["the dashboard fetch succeeding", "probe.dashboard-fetch", (c) => (c.probes[4].js = { ok: true, status: 200, type: "basic" })],
  ["the dashboard fetch reaching the panel and getting 200", "probe.dashboard-fetch", (c) => ((c.probes[4].cspMessages = []), (c.probes[4].hops = [hop({ url: `${P}/api/v1/fromradio`, phase: "probe-dash", status: 200 })]))],
  ["the dashboard fetch neither blocked nor sent", "probe.dashboard-fetch", (c) => (c.probes[4].cspMessages = [])],
  ["no shim traffic", "shim.roundtrip", (c) => (c.reqs = c.reqs.filter((h) => !h.url.includes("/api/v1/")))],
  ["the shim answering 502", "shim.roundtrip", (c) => c.reqs.filter((h) => h.url.includes("/api/v1/fromradio")).forEach((h) => (h.status = 502))],
  ["the panel never connected", "shim.panel-state", (c) => (c.panelDom.state = "connecting")],
  ["an empty recording", "cap.panel-document", (c) => (c.reqs = [])],
  ["a panel that never loaded", "cap.panel-document", (c) => c.reqs.filter((h) => h.origin === P && h.type === "Document").forEach((h) => (h.status = 502))],
];
for (const [what, rule, mut] of mutations) {
  const c = clone(clean());
  mut(c);
  const f = fails(c);
  if (f.includes(rule)) say("ok", `${what} -> ${rule} flags it`);
  else say("FAIL", `${what}: ${rule} did not flag it (flagged: ${f.join(", ") || "nothing"})`);
}

// a labelled gap is FAIL with its label by default and a note under knownGaps
{
  const c = clone(clean());
  c.probes[0].gap = "blocked-by P1 guard.rs";
  c.probes[0].hops[0].status = 200;
  const r = evaluate(c).find((x) => x.id === "probe.same-site-get");
  const k = clone(c);
  k.knownGaps = true;
  const rk = evaluate(k).find((x) => x.id === "probe.same-site-get");
  const g = clone(c);
  g.probes[0].hops[0].status = 403; // the gap closed: green again, no label
  const rg = evaluate(g).find((x) => x.id === "probe.same-site-get");
  if (r.level === "FAIL" && r.text.startsWith("GAP (blocked-by P1 guard.rs)") && rk.level === "note" && rk.text.startsWith("KNOWN GAP") && rg.level === "ok") say("ok", "a labelled gap is a FAIL by default, a note under PANEL_LINT_KNOWN_GAPS=1, and turns ok when the guard is fixed");
  else say("FAIL", `gap handling: ${r.level} ${r.text.slice(0, 40)} / ${rk.level} / ${rg.level}`);
}

// a residual (accepted by design) is a note when it fails and ok when it holds
{
  const c = clone(clean());
  c.probes[0].residual = "by design";
  c.probes[0].hops[0].status = 200;
  const r = evaluate(c).find((x) => x.id === "probe.same-site-get");
  const c2 = clone(clean());
  c2.probes[0].residual = "by design";
  const r2 = evaluate(c2).find((x) => x.id === "probe.same-site-get");
  if (r.level === "note" && r.text.startsWith("RESIDUAL") && r2.level === "ok") say("ok", "a residual risk accepted by design is a note when it shows and ok when it does not");
  else say("FAIL", `residual handling: ${r.level} / ${r2.level}`);
}

// an optional host the manifest declares is a note, not a failure
{
  const c = clone(clean());
  c.reqs.push(hop({ url: "https://tiles.openfreemap.org/styles/liberty", type: "Fetch" }));
  const r = evaluate(c);
  const bad = r.filter((x) => x.level === "FAIL" && x.id === "net.own-origin-only");
  const noteIt = r.find((x) => x.id === "net.optional-hosts");
  if (!bad.length && noteIt) say("ok", "a declared optional host (tiles.openfreemap.org) is reported as a note, not a failure");
  else say("FAIL", "a declared optional host was mishandled");
}

// the localhost scenario needs its 308
{
  const c = clone(clean());
  c.scenario = "localhost";
  c.localhost = { startUrl: "http://localhost:11180/" };
  c.reqs.unshift(hop({ url: "http://localhost:11180/", type: "Document", status: 308, phase: "setup", docUrl: "http://localhost:11180/", rawResponseHeaders: { location: `${D}/` } }));
  c.dashboardOrigin = D;
  const ok308 = evaluate(c).find((r) => r.id === "localhost.308");
  const c2 = clone(c);
  c2.reqs[0].status = 200;
  const bad308 = evaluate(c2).find((r) => r.id === "localhost.308");
  if (ok308 && ok308.level === "ok" && bad308 && bad308.level === "FAIL") say("ok", "localhost.308 passes on a 308 to the dashboard origin and fails on a 200");
  else say("FAIL", "localhost.308 does not discriminate");
}

// ---- the static bundle lint
{
  const dir = mkdtempSync(join(tmpdir(), "panel-lint-selftest-"));
  try {
    const bad = join(dir, "bad");
    mkdirSync(bad);
    writeFileSync(
      join(bad, "index.html"),
      `<!doctype html><link rel="stylesheet" href="https://fonts.googleapis.com/css"><script src="//cdn.example.net/x.js"></script>
<img src="/ok.png" srcset="a.png 1x, https://t.example.org/b.png 2x"><button onclick="x()">b</button><a href="javascript:void 0">j</a><link rel="preconnect" href="https://pre.example.com">`,
    );
    const r = lintBundle(bad, { optionalHosts: ["tiles.openfreemap.org"] }).results;
    const flagged = new Set(r.filter((x) => x.level === "FAIL").map((x) => x.id));
    for (const id of ["bundle.same-origin-srcs", "bundle.inline-handlers", "bundle.javascript-urls"]) {
      say(flagged.has(id) ? "ok" : "FAIL", `a hostile index.html is flagged by ${id}`);
    }
    const txt = r.find((x) => x.id === "bundle.same-origin-srcs").text;
    for (const h of ["fonts.googleapis.com", "cdn.example.net", "t.example.org", "pre.example.com"]) {
      say(txt.includes(h) || r.some((x) => x.text.includes(h)) ? "ok" : "FAIL", `the hostile reference to ${h} is reported`);
    }
    const good = join(dir, "good");
    mkdirSync(good);
    writeFileSync(join(good, "index.html"), `<!doctype html><link rel="icon" href="data:,"><link rel="stylesheet" href="style.css"><script src="/app.js"></script><img src="logo.svg"><a href="https://github.com/x">docs</a>`);
    writeFileSync(join(good, "app.js"), `fetch("https://tiles.openfreemap.org/x"); const u = "https://api.example.com/y"; const ns = "http://www.w3.org/2000/svg";`);
    const g = lintBundle(good, { optionalHosts: ["tiles.openfreemap.org"] });
    const gFails = g.results.filter((x) => x.level === "FAIL");
    say(gFails.length === 0 ? "ok" : "FAIL", `a clean index.html passes the static lint${gFails.length ? ` (flagged ${gFails.map((x) => x.id)})` : ""}`);
    const hostsFound = [...g.hosts.keys()].sort().join(",");
    say(hostsFound === "api.example.com,github.com,tiles.openfreemap.org" ? "ok" : "FAIL", `third-party hostnames are found by grep, namespaces skipped (${hostsFound})`);
    const cls = ["", "#x", "data:,", "a/b.js", "/a.js", "//h/x", "https://h/x", "ws://h/x", "javascript:0"].map(classifyUrl).join(" ");
    say(cls === "fragment fragment data relative root-relative protocol-relative absolute absolute javascript" ? "ok" : "FAIL", `URL classes: ${cls}`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

process.stdout.write(`SELFTEST FAILS=${FAILS}\n`);
process.exit(FAILS > 0 ? 1 : 0);
