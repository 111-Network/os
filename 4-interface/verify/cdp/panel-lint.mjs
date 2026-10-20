#!/usr/bin/env node
// Panel offline lint + G3 driver (os/5-messaging/README.md section 13 "Offline lint", section 20 G3).
// Run by 4-interface/verify/panel-lint.sh inside the namespaces, after the daemon is up. No npm deps (Node >= 22).
//
//   node panel-lint.mjs --root <scratch node root> --out <dir> [--lan-host 192.168.77.5] [--cross-host 192.168.77.9]
//                       [--attacker-port 11300] [--dwell 10] [--chrome <path>] [--no-canary]
//
// Scenarios (each in its own browser context, every request recorded over CDP):
//   lan        dashboard on the node's LAN address, paired through the pair form, service page > Panel tab, the ticket
//              URL into the panel origin, 10 s of the panel running; then the dashboard-origin fetch, a reload (the
//              dashboard re-mints), a spent-ticket replay, and probes from other origins (same site, cross site).
//   localhost  http://localhost:11180 -> 308 -> 127.0.0.1, implicit loopback session, Panel tab, panel running.
//   canary     a deliberately hostile bundle is served in place of the real one; the lint must flag it. Proves the
//              recorder and the rules can fail (the files are restored afterwards).
// Output: `ok:` / `FAIL:` / `note:` lines; the exit status is 1 on any FAIL. Request tables at the end.
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { Browser, findChrome, launchChrome, sleep, waitFor } from "./lib/cdp.mjs";
import { Recorder, originOf } from "./lib/record.mjs";
import { evaluate, print, requestTable } from "./lib/rules.mjs";
import { lintBundle } from "./lib/bundle.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));

// ------------------------------------------------------------------------------------------------ arguments

function parseArgs(argv) {
  const o = { dwell: 10, "lan-host": "192.168.77.5", "cross-host": "192.168.77.9", "attacker-port": 11300, "dashboard-port": 11180, "bundle-kind": "placeholder" };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) continue;
    const k = a.slice(2);
    if (k === "no-canary") o[k] = true;
    else o[k] = argv[++i];
  }
  o.dwell = Number(o.dwell);
  return o;
}
const args = parseArgs(process.argv.slice(2));
for (const need of ["root", "out"]) {
  if (!args[need]) {
    process.stderr.write(`panel-lint.mjs: --${need} is required\n`);
    process.exit(64);
  }
}
mkdirSync(args.out, { recursive: true });

let FAILS = 0;
const say = (level, id, text) => {
  if (level === "FAIL") FAILS++;
  process.stdout.write(`${level}: ${id}: ${text}\n`);
};
const info = (s) => process.stdout.write(`${s}\n`);
const emit = (scope, results) => {
  for (const r of results) say(r.level, `${scope}/${r.id}`, r.text);
};

// ------------------------------------------------------------------------------------------------ the manifest

/** The bits of assets/panels/<id>.toml the lint compares against (a section-aware line reader; the file is flat). */
function readManifest(path) {
  const out = { allow: [], optional_hosts: [], sandbox: "default" };
  let sect = "";
  for (const raw of readFileSync(path, "utf8").split("\n")) {
    const line = raw.replace(/\s+#.*$/, "").trim();
    if (!line || line.startsWith("#")) continue;
    const s = /^\[([a-z.]+)\]$/.exec(line);
    if (s) {
      sect = s[1];
      continue;
    }
    const kv = /^([a-z_]+)\s*=\s*(.+)$/.exec(line);
    if (!kv) continue;
    const arr = /^\[(.*)\]$/.exec(kv[2]);
    const val = arr ? [...arr[1].matchAll(/"([^"]*)"/g)].map((m) => m[1]) : /^"(.*)"$/.exec(kv[2])?.[1] ?? kv[2];
    if (sect === "frame" && kv[1] === "allow") out.allow = val;
    if (sect === "frame" && kv[1] === "sandbox") out.sandbox = val;
    if (sect === "frame" && kv[1] === "needs_secure_context") out.needs_secure_context = val === "true";
    if (sect === "offline" && kv[1] === "optional_hosts") out.optional_hosts = val;
    if (sect === "source" && kv[1] === "bundle") out.bundle = val;
    if (sect === "" && kv[1] === "id") out.id = val;
  }
  return out;
}

// ------------------------------------------------------------------------------------------------ one scenario

const TAB = `[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === 'Panel')`;
const BUTTON = (label) => `[...document.querySelectorAll('button')].find((t) => t.textContent.trim() === ${JSON.stringify(label)})`;
const REAL_DOM = `(() => ({ cookie: document.cookie, url: location.href, host: null, inputs: [...document.querySelectorAll('input,textarea')].map((i) => i.value).filter((v) => v) }))()`;
const DOM = `(() => { const d = document.body ? document.body.dataset : {}; const h = document.getElementById('host'); return { cookie: document.cookie, state: d.state, nodes: d.nodes, board: d.board, polls: d.polls, frames: d.frames, errors: d.errors, put: d.put, host: h ? h.value : null, url: location.href }; })()`;

/**
 * Run one browser scenario. opts: { name, startUrl, dashboardOrigin, panelOrigin, nodeHost, code, dwell, full, manifest,
 * panelId }. `full` adds the dashboard-origin fetch, the reload, the spent-ticket replay and the other-origin probes.
 * Returns the evaluate() context (and writes the raw recording to --out).
 */
async function scenario(b, opts) {
  const { name, dashboardOrigin, panelOrigin } = opts;
  const panelId = opts.panelId;
  const rec = new Recorder(b);
  const { browserContextId } = await b.send("Target.createBrowserContext", { disposeOnDetach: false });
  const cleanup = [];
  try {
    info(`\n== scenario ${name}: ${opts.startUrl}`);
    rec.setPhase("setup");
    const dash = await rec.openTab(opts.startUrl, "dashboard", browserContextId);
    const S = dash.sid;
    cleanup.push(() => rec.closeTab(dash.targetId));
    const ev = (expr, o) => b.evaluate(S, expr, o);

    // --- pair (LAN visitors need a code; a trusted loopback visitor never sees the form)
    if (opts.code) {
      await waitFor(() => ev(`!!document.querySelector('#pair-code') && document.querySelector('#pair-code').value.length >= 9`), { timeout: 30000, what: "the pair form prefilled from #/pair?code=" });
      await ev(`document.querySelector('form button[type=submit]').click(); true`);
      await waitFor(() => ev(`!location.hash.startsWith('#/pair')`), { timeout: 30000, what: "leaving /pair after pairing" });
      say("ok", `${name}/pair`, "paired through the pair form (the browser is a LAN visitor: no implicit session)");
    } else {
      await waitFor(() => ev(`document.readyState === 'complete' && !!document.getElementById('root') && document.getElementById('root').children.length > 0`), { timeout: 30000, what: "the dashboard to render" });
      const first = rec.requests().find((h) => h.type === "Document");
      say("note", `${name}/session`, `dashboard rendered at ${await ev("location.origin")} with the implicit loopback session (document first requested as ${first ? first.url : "?"})`);
    }

    // --- service page, Panel tab
    await ev(`location.hash = '#/services/${panelId}'; true`);
    try {
      await waitFor(() => ev(`!!(${TAB})`), { timeout: 60000, interval: 250, what: `the Panel tab on #/services/${panelId} (the service must be LIVE)` });
    } catch (e) {
      const svc = await ev(`fetch('/api/v1/services/${panelId}').then((r) => r.text())`).catch(() => "?");
      say("FAIL", `${name}/panel-tab`, `${e.message}; service record: ${String(svc).slice(0, 400)}`);
      throw e;
    }
    say("ok", `${name}/panel-tab`, "the service page shows a Panel tab (meshtastic is LIVE)");
    rec.setPhase("panel");
    await ev(`(${TAB}).click(); true`);
    const src = await waitFor(() => ev(`(document.querySelector('iframe') || {}).getAttribute ? document.querySelector('iframe').getAttribute('src') : null`), { timeout: 30000, what: "the panel iframe" });
    const ticket = new URL(src).searchParams.get("p111_ticket") || "";

    // --- the panel frame, its DOM through an isolated world
    const panelFrame = async () => {
      const { frameTree } = await b.send("Page.getFrameTree", {}, S);
      return (frameTree.childFrames || []).map((c) => c.frame).find((f) => originOf(f.url) === panelOrigin) || null;
    };
    const panelEval = async (expr) => {
      const f = await panelFrame();
      if (!f) throw new Error("no panel frame yet");
      const { executionContextId } = await b.send("Page.createIsolatedWorld", { frameId: f.id, worldName: "lint" }, S);
      return b.evaluate(S, expr, { contextId: executionContextId });
    };
    const real = args["bundle-kind"] === "real";
    const connected = async (what) => {
      let last = null;
      try {
        await waitFor(
          async () => {
            if (real) {
              // a real client: it has spoken the shim's two calls from its own origin
              const rq = rec.requests().filter((h) => h.origin === panelOrigin && h.status === 200 && h.phase !== "spent");
              last = { puts: rq.filter((h) => h.method === "PUT").length, gets: rq.filter((h) => h.method === "GET" && new URL(h.url).pathname === "/api/v1/fromradio").length };
              return last.puts >= 1 && last.gets >= 1;
            }
            last = await panelEval(DOM).catch(() => null);
            return last && last.state === "connected";
          },
          { timeout: real ? 90000 : 45000, interval: 300, what },
        );
      } catch (e) {
        say("FAIL", `${name}/panel-running`, `${e.message}; last panel state ${JSON.stringify(last)}`);
        throw e;
      }
    };
    await connected("the panel to reach 'connected' through the shim");
    say("ok", `${name}/panel-running`, `the panel loaded in the iframe at ${panelOrigin} and is talking to the node's radio shim`);
    info(`note: ${name}: letting the panel run ${opts.dwell} s ...`);
    await sleep(opts.dwell * 1000);

    // --- what the dashboard and the panel look like after the dwell
    const iframes = await ev(`[...document.querySelectorAll('iframe')].map((f) => ({ src: f.getAttribute('src'), sandbox: f.getAttribute('sandbox'), allow: f.getAttribute('allow'), referrerpolicy: f.getAttribute('referrerpolicy'), title: f.title }))`);
    const panelDom = await panelEval(real ? REAL_DOM : DOM);
    const pf = await panelFrame();
    const dashUrl = await ev("location.href");
    const { cookies } = await b.send("Network.getCookies", { urls: [panelOrigin, dashboardOrigin] }, S);
    const panelRecord = await ev(`fetch('/api/v1/panels', { credentials: 'same-origin' }).then((r) => r.json()).then((j) => (j.panels || []).find((p) => p.id === ${JSON.stringify(panelId)}) || null)`).catch(() => null);
    const sessionPost = rec.requests().find((h) => h.method === "POST" && new URL(h.url).pathname === `/api/v1/panels/${panelId}/session`);
    let postedUrl = null;
    if (sessionPost) {
      const body = await b.send("Network.getResponseBody", { requestId: sessionPost.requestId }, sessionPost.sid).catch(() => null);
      try {
        postedUrl = JSON.parse(body.body).url;
      } catch {
        /* unreadable */
      }
    }

    const probes = [];
    let remint = null;
    let spent = null;

    // --- the dashboard's own origin cannot call the panel API (CSP connect-src 'self'; the panel's guard behind it)
    rec.setPhase("probe-dash");
    {
      const target = `${panelOrigin}/api/v1/fromradio?all=true`;
      const js = await ev(`fetch(${JSON.stringify(target)}, { credentials: 'include' }).then((r) => ({ ok: true, status: r.status, type: r.type })).catch((e) => ({ ok: false, err: String(e).slice(0, 80) }))`);
      await sleep(500);
      probes.push({
        name: "dashboard-fetch",
        kind: "csp-or-refused",
        title: "fetch of the panel API from the dashboard origin",
        fromOrigin: dashboardOrigin,
        js,
        cspMessages: rec.logs("probe-dash").filter((m) => /Content Security Policy|violates/i.test(m)),
        hops: rec.requests().filter((h) => h.phase === "probe-dash" && h.docOrigin === dashboardOrigin && h.origin === panelOrigin),
      });
    }

    if (opts.probes) {
      // --- a frame inside the dashboard navigating to the panel API (frame-src allows the panel origin; the guard must still say no)
      rec.setPhase("probe-dash-frame");
      await ev(`(() => { const f = document.createElement('iframe'); f.id = 'lint-probe'; f.setAttribute('aria-hidden', 'true'); f.src = ${JSON.stringify(`${panelOrigin}/api/v1/fromradio?all=true`)}; document.body.appendChild(f); return true; })()`);
      await waitFor(() => rec.requests().some((h) => h.phase === "probe-dash-frame" && h.origin === panelOrigin && h.type === "Document" && h.status), { timeout: 10000, what: "the dashboard-frame navigation to be answered" }).catch(() => {});
      await sleep(300);
      probes.push({
        name: "dashboard-frame-get",
        kind: "refused",
        title: "navigation of a frame inside the dashboard to the panel API",
        fromOrigin: dashboardOrigin,
        noJs: true,
        statuses: [403],
        cookieSent: true,
        ...(opts.probes.sec ? { expectFetchSite: "same-site" } : {}),
        // the dashboard sends Referrer-Policy: no-referrer, so this frame navigation names no origin at all: with no Fetch Metadata
        // (a LAN-IP origin) it is the stealth case, not the Origin/Referer case the guard can close
        residual: opts.probes.residual,
        hops: rec.requests().filter((h) => h.phase === "probe-dash-frame" && h.origin === panelOrigin && h.type === "Document"),
      });
      await ev(`(() => { const f = document.getElementById('lint-probe'); if (f) f.remove(); return true; })()`);
    }

    if (opts.full) {
      // --- the dashboard re-mints: Reload gives a new ticket and a new session-less swap (the cookie is already there)
      rec.setPhase("remint");
      const before = rec.requests().filter((h) => h.method === "POST" && new URL(h.url).pathname === `/api/v1/panels/${panelId}/session`).length;
      await ev(`(${BUTTON("Reload")}).click(); true`);
      const src2 = await waitFor(async () => {
        const s2 = await ev(`(document.querySelector('iframe') || { getAttribute: () => null }).getAttribute('src')`);
        return s2 && s2 !== src ? s2 : null;
      }, { timeout: 30000, what: "a new ticket in the iframe after Reload" });
      const ticket2 = new URL(src2).searchParams.get("p111_ticket") || "";
      let again = true;
      try {
        await connected("the reloaded panel to reach 'connected'");
      } catch {
        again = false;
      }
      await sleep(300);
      const rq = rec.requests();
      remint = {
        posts: rq.filter((h) => h.method === "POST" && new URL(h.url).pathname === `/api/v1/panels/${panelId}/session`).length - before,
        ticket2,
        swapStatus: (rq.find((h) => ticket2 && h.url.includes(ticket2)) || {}).status || 0,
        connectedAgain: again,
        distinct: !!ticket2 && ticket2 !== ticket,
      };
    }
    await rec.closeTab(dash.targetId); // the panel stops polling; nothing but the steps below is on the wire
    cleanup.length = 0;

    if (opts.full) {
      // --- a spent ticket answers 403 and sets nothing (a fresh browser, so no cookie turns it into a plain redirect)
      rec.setPhase("spent");
      const fresh = await b.send("Target.createBrowserContext", { disposeOnDetach: false });
      const spentTab = await rec.openTab(`${panelOrigin}/?p111_ticket=${ticket}`, "spent-replay", fresh.browserContextId);
      const spentHop = await waitFor(() => rec.requests().find((h) => h.label === "spent-replay" && h.url.includes(ticket) && h.status), { timeout: 15000, what: "the spent-ticket replay to be answered" });
      await sleep(300);
      let spentBody = null;
      const raw = await b.send("Network.getResponseBody", { requestId: spentHop.requestId }, spentHop.sid).catch(() => null);
      try {
        spentBody = JSON.parse(raw.body);
      } catch {
        spentBody = raw ? { raw: String(raw.body).slice(0, 120) } : null;
      }
      spent = { status: spentHop.status, code: spentBody && spentBody.code, setCookie: (spentHop.rawResponseHeaders || {})["set-cookie"] || null, cors: Object.keys(spentHop.rawResponseHeaders || {}).filter((k) => k.startsWith("access-control-")) };
      await rec.closeTab(spentTab.targetId);
      await b.send("Target.disposeBrowserContext", { browserContextId: fresh.browserContextId }).catch(() => {});
    }

    // --- other origins on the node's host drive the panel API with the victim's cookie jar
    if (opts.probes) {
      const pr = opts.probes; // { same, cross, sec: Sec-Fetch-* expected (trustworthy origin), gap }
      rec.setPhase("probe-attackers"); // loading the other-origin pages
      const attackers = {};
      for (const [label, origin] of [["attacker-same", pr.same], ["attacker-cross", pr.cross]]) {
        const t = await rec.openTab(`${origin}/`, label, browserContextId);
        await waitFor(() => b.evaluate(t.sid, "document.readyState === 'complete'"), { timeout: 15000, what: `${origin} to load` });
        attackers[label] = { ...t, origin };
      }
      const run = async (label, name2, kind, title, init, extra) => {
        rec.setPhase(`probe-${name2}`);
        const a = attackers[label];
        const js = await b.evaluate(
          a.sid,
          `(async () => { try { const r = await fetch(${JSON.stringify(panelOrigin + extra.path)}, ${JSON.stringify(init)}); return { ok: true, status: r.status, type: r.type }; } catch (e) { return { ok: false, err: String(e).slice(0, 80) }; } })()`,
        );
        await sleep(400);
        probes.push({
          name: name2,
          kind,
          title,
          fromOrigin: a.origin,
          js,
          ...extra,
          hops: rec.requests().filter((h) => h.phase === `probe-${name2}` && h.label === label && h.origin === panelOrigin),
        });
      };
      const sec = (site) => (pr.sec ? { expectFetchSite: site } : {});
      const post = { method: "POST", mode: "no-cors", credentials: "include", headers: { "Content-Type": "text/plain" }, body: "want_config" };
      await run("attacker-same", "same-site-get", "refused", "same-site cross-origin GET with the panel cookie", { credentials: "include" }, { path: "/api/v1/fromradio?all=true", statuses: [403], cookieSent: true, ...sec("same-site"), gap: pr.gap });
      await run("attacker-same", "stealth-get", "refused", "same-site no-cors GET that names no origin (referrerPolicy no-referrer)", { credentials: "include", mode: "no-cors", referrerPolicy: "no-referrer" }, { path: "/api/v1/fromradio?all=true", statuses: [403], cookieSent: true, opaque: true, ...sec("same-site"), residual: pr.residual });
      await run("attacker-cross", "cross-site-get", "refused", "cross-site GET", { credentials: "include" }, { path: "/api/v1/fromradio?all=true", statuses: pr.sec ? [403] : [401, 403], cookieSent: false, ...sec("cross-site") });
      await run("attacker-same", "drive-by-post", "refused", "same-site drive-by POST (a simple no-cors request)", post, { path: "/api/v1/toradio", expectMethod: "POST", statuses: [403], cookieSent: true, opaque: true, ...sec("same-site") });
      await run("attacker-cross", "cross-site-post", "refused", "cross-site drive-by POST", post, { path: "/api/v1/toradio", expectMethod: "POST", statuses: [403], cookieSent: false, opaque: true, ...sec("cross-site") });
      await run("attacker-same", "preflight", "preflight", "non-simple PUT", { method: "PUT", credentials: "include", headers: { "Content-Type": "application/x-protobuf" }, body: "x" }, { path: "/api/v1/toradio" });
      for (const a of Object.values(attackers)) await rec.closeTab(a.targetId);
    }

    // --- evaluate
    await sleep(300);
    const reqs = rec.requests();
    const panelFrameIds = rec.frameTable().filter((f) => originOf(f.url) === panelOrigin && f.parentId).map((f) => f.id);
    const ctx = {
      scenario: name,
      panelId,
      dashboardOrigin,
      panelOrigin,
      nodeHost: opts.nodeHost,
      optionalHosts: opts.manifest.optional_hosts,
      manifest: { allow: opts.manifest.allow, sandboxDefault: undefined },
      reqs,
      webSockets: rec.webSockets(),
      popups: rec.popups,
      panelFrameIds,
      ticket,
      entryPath: "/",
      iframes,
      cookies,
      frameUrls: { panel: pf && pf.url, dashboard: dashUrl },
      panelDom,
      probes,
      remint,
      spent,
      panelRecord,
      postedUrl,
      localhost: opts.localhostStart ? { startUrl: opts.localhostStart } : null,
      entryUrls: [opts.startUrl.replace(/#.*$/, "")],
      bundleKind: args["bundle-kind"],
      knownGaps: process.env.PANEL_LINT_KNOWN_GAPS === "1",
    };
    writeFileSync(join(args.out, `${name}.recording.json`), JSON.stringify({ ctx: { ...ctx, reqs: undefined }, reqs }, null, 1));
    return ctx;
  } finally {
    rec.stop();
    for (const c of cleanup) await Promise.resolve(c()).catch(() => {});
    await b.send("Target.disposeBrowserContext", { browserContextId }).catch(() => {});
  }
}

// ------------------------------------------------------------------------------------------------ main

async function main() {
  const manifestPath = args.manifest || join(HERE, "../../../1-bootstrap/crates/node111/assets/panels/meshtastic.toml");
  const manifest = readManifest(manifestPath);
  const panelId = manifest.id || "meshtastic";
  const slots = (() => {
    try {
      return JSON.parse(readFileSync(join(args.root, "state/panels.json"), "utf8")).slots || {};
    } catch {
      return {};
    }
  })();
  const panelPort = Number(slots[panelId] || 11190);
  const served = join(args.root, "current", manifest.bundle || "panels/meshtastic-web");
  const dport = args["dashboard-port"];
  const code = (() => {
    try {
      return JSON.parse(readFileSync(join(args.root, "state/dashboard-pair.json"), "utf8")).code;
    } catch {
      return "";
    }
  })();

  // --- the static bundle, offline
  info(`== static bundle lint: ${served}`);
  emit("bundle", lintBundle(served, { optionalHosts: manifest.optional_hosts }).results);
  if (args["bundle-label"]) info(`note: bundle under test: ${args["bundle-label"]}`);

  const chromePath = findChrome();
  if (!chromePath) {
    say("FAIL", "chrome", "no Chrome/Chromium found (set CHROME=<path>)");
    return finish([]);
  }
  info(`\n== headless chrome: ${chromePath}`);
  const chrome = await launchChrome({ chrome: chromePath });
  const b = await Browser.connect(chrome.wsUrl);
  const tables = [];
  try {
    const lanHost = args["lan-host"];
    // each scenario on its own: one that breaks must not hide the others
    const guarded = async (name, fn) => {
      try {
        await fn();
      } catch (e) {
        say("FAIL", `${name}/driver`, `${e && e.stack ? e.stack.split("\n").slice(0, 3).join(" | ") : e}`);
      }
    };

    // ---------------------------------------------------------------- lan, paired
    await guarded("lan", async () => {
      if (!code) say("FAIL", "lan/pair-code", "state/dashboard-pair.json has no code");
      const lan = await scenario(b, {
        name: "lan",
        startUrl: `http://${lanHost}:${dport}/#/pair?code=${code}`,
        dashboardOrigin: `http://${lanHost}:${dport}`,
        panelOrigin: `http://${lanHost}:${panelPort}`,
        nodeHost: lanHost,
        code,
        dwell: args.dwell,
        full: true,
        // http://<LAN IP> is not a trustworthy origin: Chrome sends no Sec-Fetch-* to it, so the guard's Fetch Metadata rules
        // cannot fire there. The same-site probes are the ones that notice (see the gap text).
        probes: {
          same: `http://${lanHost}:${args["attacker-port"]}`,
          cross: `http://${args["cross-host"]}:${args["attacker-port"]}`,
          sec: false,
          gap: "blocked-by P1 guard.rs: Chrome sends no Sec-Fetch-* to http://<LAN IP>, so a same-site GET to an API path is served; refuse a foreign Origin/Referer on API paths too",
          residual: "spec decision: with no Origin, no Referer and no Sec-Fetch-* the request is identical to the panel's own GET; only a per-origin proof the client sends (not available to an unmodified client) closes it",
        },
        manifest,
        panelId,
      });
      manifestVsApi("lan", manifest, lan.panelRecord, panelPort);
      emit("lan", evaluate(lan));
      emit("lan", extra(lan));
      tables.push(["lan", lan.reqs, lan.dashboardOrigin]);
    });

    // ---------------------------------------------------------------- localhost -> 308 -> 127.0.0.1
    await guarded("localhost", async () => {
      const lo = await scenario(b, {
        name: "localhost",
        startUrl: `http://localhost:${dport}/#/services/${panelId}`,
        localhostStart: `http://localhost:${dport}/`,
        dashboardOrigin: `http://127.0.0.1:${dport}`,
        panelOrigin: `http://127.0.0.1:${panelPort}`,
        nodeHost: "127.0.0.1",
        code: null,
        dwell: Math.min(args.dwell, 5),
        full: false,
        probes: { same: `http://127.0.0.1:${args["attacker-port"]}`, cross: `http://localhost:${args["attacker-port"]}`, sec: true },
        manifest,
        panelId,
      });
      emit("localhost", evaluate(lo));
      tables.push(["localhost", lo.reqs, lo.dashboardOrigin]);
    });

    // ---------------------------------------------------------------- canary: the lint must catch a hostile bundle
    if (!args["no-canary"]) await guarded("canary", () => canary(b, { served, dport, panelPort, manifest, panelId, tables }));
  } catch (e) {
    say("FAIL", "driver", `${e && e.stack ? e.stack : e}`);
  } finally {
    b.close();
    await chrome.stop();
  }
  finish(tables);
}

function manifestVsApi(scope, manifest, rec, port) {
  const sameArr = (a, b) => JSON.stringify([...(a || [])].sort()) === JSON.stringify([...(b || [])].sort());
  if (!rec) return say("FAIL", `${scope}/manifest-vs-api`, "GET /api/v1/panels did not list the panel");
  const good = sameArr(rec.allow, manifest.allow) && sameArr(rec.optional_hosts, manifest.optional_hosts) && rec.port === port && rec.state && rec.state.enabled && rec.state.listening;
  say(good ? "ok" : "FAIL", `${scope}/manifest-vs-api`, good ? `GET /api/v1/panels agrees with assets/panels/${manifest.id}.toml: allow [${rec.allow}], optional_hosts [${rec.optional_hosts}], port ${rec.port}, enabled + listening` : `API record ${JSON.stringify(rec)} vs manifest ${JSON.stringify(manifest)}`);
}

/** Checks that live in the driver (they need the whole recording, not one rule's slice). */
function extra(ctx) {
  const R = [];
  const add = (level, id, text) => R.push({ id, level, text });
  if (ctx.remint) {
    const r = ctx.remint;
    add(r.posts === 1 && r.distinct && r.swapStatus === 303 && r.connectedAgain ? "ok" : "FAIL", "remint.reload", r.posts === 1 && r.distinct && r.swapStatus === 303 && r.connectedAgain ? "Reload re-mints: one new POST /panels/{id}/session, a different single-use ticket, swapped 303, and the panel reconnected" : `reload did not re-mint cleanly: ${JSON.stringify(r)}`);
  }
  if (ctx.spent) {
    const s = ctx.spent;
    const good = s.status === 403 && s.code === "panel_ticket_expired" && !s.setCookie && s.cors.length === 0;
    add(good ? "ok" : "FAIL", "ticket.spent", good ? "a spent ticket replayed in a fresh browser answers 403 panel_ticket_expired, sets no cookie, no CORS" : `spent-ticket replay: ${JSON.stringify(s)}`);
  }
  if (ctx.postedUrl) {
    const same = (ctx.iframes[0] && ctx.iframes[0].src) !== undefined && ctx.postedUrl.includes(ctx.ticket);
    add(same ? "ok" : "FAIL", "ticket.posted", same ? "the ticket in the iframe src is the one POST /api/v1/panels/{id}/session returned" : `session answer ${ctx.postedUrl.replace(/p111_ticket=[^&]*/, "p111_ticket=…")} does not carry the iframe's ticket`);
  } else add("note", "ticket.posted", "the POST answer body was not readable over CDP; the iframe src was used as the ticket");
  return R;
}

async function canary(b, { served, dport, panelPort, manifest, panelId, tables }) {
  const canaryDir = join(HERE, "canary-bundle");
  info(`\n== canary: serving a hostile bundle from ${canaryDir} (files restored afterwards)`);
  // overwrite in place: the directory must never vanish (the listener reconciles every 2 s)
  const orig = new Map();
  const names = readdirSync(canaryDir);
  for (const n of names) {
    const p = join(served, n);
    orig.set(n, existsSync(p) ? readFileSync(p) : null);
    writeFileSync(p, readFileSync(join(canaryDir, n)));
  }
  let ctx;
  try {
    const staticRes = lintBundle(served, { optionalHosts: manifest.optional_hosts }).results;
    const flaggedStatic = new Set(staticRes.filter((r) => r.level === "FAIL").map((r) => r.id));
    for (const id of ["bundle.same-origin-srcs", "bundle.inline-handlers"]) {
      say(flaggedStatic.has(id) ? "ok" : "FAIL", `canary/${id}`, flaggedStatic.has(id) ? `the static lint flags the hostile bundle (${staticRes.find((r) => r.id === id).text.slice(0, 140)})` : `the static lint did NOT flag the hostile bundle at ${id}`);
    }
    ctx = await scenario(b, {
      name: "canary",
      startUrl: `http://localhost:${dport}/#/services/${panelId}`,
      localhostStart: null,
      dashboardOrigin: `http://127.0.0.1:${dport}`,
      panelOrigin: `http://127.0.0.1:${panelPort}`,
      nodeHost: "127.0.0.1",
      code: null,
      dwell: 3,
      full: false,
      probes: null,
      manifest,
      panelId,
    });
  } finally {
    for (const [n, buf] of orig) {
      const p = join(served, n);
      if (buf === null) rmSync(p, { force: true });
      else writeFileSync(p, buf);
    }
  }
  const results = evaluate(ctx);
  const byId = new Map(results.map((r) => [r.id, r]));
  const mustFlag = [
    ["net.own-origin-only", "a font CDN, a script CDN, an analytics pixel and a telemetry endpoint"],
    ["net.hosts", "requests to hosts other than the node's own"],
    ["net.no-websocket", "a WebSocket"],
    ["net.no-dashboard", "a request from the panel to the dashboard origin"],
  ];
  for (const [id, what] of mustFlag) {
    const r = byId.get(id);
    say(r && r.level === "FAIL" ? "ok" : "FAIL", `canary/${id}`, r && r.level === "FAIL" ? `flagged as expected (${what}): ${r.text.slice(0, 170)}` : `NOT flagged — the lint cannot see ${what}`);
  }
  const opt = byId.get("net.optional-hosts");
  say(opt && opt.level === "note" ? "ok" : "FAIL", "canary/net.optional-hosts", opt ? `a declared optional host is reported, not failed: ${opt.text.slice(0, 120)}` : "the declared optional host request (tiles.openfreemap.org) was not recorded");
  const others = results.filter((r) => r.level === "FAIL" && !mustFlag.some(([id]) => id === r.id));
  if (others.length) info(`note: canary: other rules also flagged it: ${others.map((r) => r.id).join(", ")}`);
  else info("note: canary: only the leak rules flagged it (the shim, cookie and framing rules still pass on a hostile bundle: they check the node, not the bundle)");
  tables.push(["canary (a hostile bundle, on purpose)", ctx.reqs.filter((h) => h.phase === "panel"), ctx.dashboardOrigin]);
}

function finish(tables) {
  const out = [];
  for (const [name, reqs, dash] of tables) {
    out.push(`== requests: ${name} (phase, origin, request, type, status, count; ticket redacted, dashboard page chatter folded)`);
    out.push(requestTable(reqs, { collapse: dash }));
    out.push("");
  }
  const text = out.join("\n");
  if (args["tables-file"]) {
    writeFileSync(args["tables-file"], text);
    info(`\nnote: the request tables (${tables.length}) are written to ${args["tables-file"]} and printed at the end of the run`);
  } else info(`\n${text}`);
  info(`\nDRIVER FAILS=${FAILS}`);
  process.exitCode = FAILS > 0 ? 1 : 0;
}

main().catch((e) => {
  say("FAIL", "driver", `${e && e.stack ? e.stack : e}`);
  info(`\nDRIVER FAILS=${FAILS}`);
  process.exit(1);
});
