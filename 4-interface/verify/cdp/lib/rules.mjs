// The G3 / offline-lint assertions (os/5-messaging/README.md section 13 "Offline lint", section 20 G3) as a pure
// function over what the driver recorded, so selftest.mjs can prove each rule fails on a hostile recording.
//
//   evaluate(ctx) -> [{ id, level: "ok" | "FAIL" | "note", text }]
//
// ctx (everything is plain data):
//   scenario         "lan" | "localhost" | "canary"
//   dashboardOrigin  http://192.168.77.5:11180          panelOrigin  http://192.168.77.5:11190
//   nodeHost         the host the browser typed          optionalHosts  [] from the manifest
//   reqs             Recorder.requests()                 webSockets  Recorder.webSockets()
//   panelFrameIds    ids of frames whose document is on the panel origin
//   ticket           the one-time ticket (string)        entryPath  "/"
//   iframes          [{src, sandbox, allow, referrerpolicy}] in the dashboard DOM
//   manifest         { allow: [..], sandboxDefault: [..] }
//   cookies          the browser's cookies for the panel URL
//   frameUrls        { panel, dashboard } final URLs     panelDom { cookie, state, nodes, board, polls, frames, errors, put, host }
//   probes           [{ name, js, kind }]                (see panel-lint.mjs)
//   localhost        { startUrl } for the localhost scenario
import { headerLines, hostOf, isNetworkUrl } from "./record.mjs";

export const DEFAULT_SANDBOX = ["allow-scripts", "allow-same-origin", "allow-forms", "allow-popups", "allow-modals", "allow-downloads"];
const TOP_NAV = /^allow-top-navigation/;

const ok = (id, text) => ({ id, level: "ok", text });
const fail = (id, text) => ({ id, level: "FAIL", text });
const note = (id, text) => ({ id, level: "note", text });
// good/bad may be thunks, so a message that dereferences something the failing case lacks is only built when used
const check = (id, cond, good, bad) => (cond ? ok(id, typeof good === "function" ? good() : good) : fail(id, typeof bad === "function" ? bad() : bad));

const pathOf = (url) => {
  try {
    const u = new URL(url);
    return u.pathname + (u.search ? "?…" : "");
  } catch {
    return url;
  }
};
const short = (h) => `${h.method} ${h.origin}${pathOf(h.url)} (${h.type})`;
const list = (xs, n = 6) => xs.slice(0, n).join("; ") + (xs.length > n ? `; … +${xs.length - n}` : "");
const setEq = (a, b) => a.length === b.length && [...a].sort().join("\0") === [...b].sort().join("\0");

/** frame-ancestors the panel must answer with: the dashboard on the host the request was addressed to. */
export function expectedAncestors(ctx) {
  const h = new URL(ctx.dashboardOrigin).hostname;
  const bracket = h.includes(":") ? `[${h}]` : h;
  return [`http://${bracket}:11180`, `http://${bracket}:80`];
}

function cspDirective(lines, name) {
  const out = [];
  for (const line of lines) {
    for (const policy of line.split(",")) {
      for (const d of policy.split(";")) {
        const t = d.trim().split(/\s+/);
        if (t[0] && t[0].toLowerCase() === name) out.push(t.slice(1));
      }
    }
  }
  return out;
}

export function evaluate(ctx) {
  const R = [];
  const reqs = ctx.reqs || [];
  const panelFrames = new Set(ctx.panelFrameIds || []);
  const optional = new Set((ctx.optionalHosts || []).map((h) => h.toLowerCase()));
  const mainPhase = (h) => h.phase === "setup" || h.phase === "panel" || h.phase === "remint";
  const entry = new Set([...(ctx.entryUrls || []), ctx.localhost && ctx.localhost.startUrl].filter(Boolean));
  const panelHops = reqs.filter((h) => h.origin === ctx.panelOrigin);
  const panelInit = reqs.filter((h) => (h.phase === "panel" || h.phase === "remint") && (h.docOrigin === ctx.panelOrigin || (h.frameId && panelFrames.has(h.frameId))));

  // ---- the recording is not vacuous
  const docs = panelHops.filter((h) => h.phase === "panel" && h.type === "Document" && h.status === 200);
  R.push(
    check(
      "cap.panel-document",
      docs.length >= 1 && panelInit.length >= (ctx.bundleKind === "real" ? 3 : 4),
      `the panel document loaded in the frame (${docs.length} document, ${panelInit.length} requests made by the panel, ${reqs.length} recorded in all)`,
      `the recording is empty or the panel never loaded (panel documents with 200: ${docs.length}; panel-initiated requests: ${panelInit.length}) — nothing below means anything`,
    ),
  );

  // ---- G3: nothing leaves the panel origin
  const leaving = panelInit.filter((h) => isNetworkUrl(h.url) && h.origin !== ctx.panelOrigin);
  const leavingUndeclared = leaving.filter((h) => !optional.has(hostOf(h.url).toLowerCase()));
  const leavingDeclared = leaving.filter((h) => optional.has(hostOf(h.url).toLowerCase()));
  R.push(
    check(
      "net.own-origin-only",
      leavingUndeclared.length === 0,
      `every request the panel made went to its own origin ${ctx.panelOrigin} (${panelInit.length} requests; no fonts, CDN, telemetry or analytics)`,
      `the panel made ${leavingUndeclared.length} request(s) to hosts it did not declare: ${list(leavingUndeclared.map(short))}`,
    ),
  );
  if (leavingDeclared.length) {
    R.push(note("net.optional-hosts", `the panel contacted declared optional host(s) ${[...new Set(leavingDeclared.map((h) => hostOf(h.url)))].join(", ")} (allowed by [offline] optional_hosts and shown on the service page)`));
  }
  const mainHops = reqs.filter(mainPhase).filter((h) => isNetworkUrl(h.url));
  const hosts = [...new Set(mainHops.map((h) => hostOf(h.url)))].sort();
  // the URL the person typed (http://localhost:11180/, which 308s to 127.0.0.1) is the one request allowed off the node's host
  const strange = mainHops.filter((h) => hostOf(h.url) !== ctx.nodeHost && !optional.has(hostOf(h.url).toLowerCase()) && !entry.has(h.url));
  R.push(
    check(
      "net.hosts",
      strange.length === 0,
      `the whole recording (dashboard and panel) touches only the node's own host${entry.size ? " (plus the typed entry URL)" : ""}: ${hosts.join(", ") || "(none)"}`,
      `requests to hosts other than ${ctx.nodeHost}: ${list([...new Set(strange.map((h) => hostOf(h.url)))])} (${list(strange.map(short))})`,
    ),
  );
  const socks = [...(ctx.webSockets || []), ...reqs.filter((h) => h.type === "WebSocket" || /^wss?:/i.test(h.url)).map((h) => ({ url: h.url }))];
  R.push(check("net.no-websocket", socks.length === 0, "no WebSocket was opened by the dashboard or the panel", `WebSocket(s) opened: ${list(socks.map((s) => s.url))}`));
  const popups = (ctx.popups || []).filter((p) => p.phase === "setup" || p.phase === "panel" || p.phase === "remint");
  R.push(check("net.no-popups", popups.length === 0, "no window was opened by the dashboard or the panel (allow-popups is in the sandbox; nothing used it)", `window(s) opened: ${list(popups.map((p) => p.url))}`));
  const toDashboard = panelInit.filter((h) => h.origin === ctx.dashboardOrigin);
  R.push(
    check(
      "net.no-dashboard",
      toDashboard.length === 0,
      `the panel never requested the dashboard origin ${ctx.dashboardOrigin}`,
      `the panel requested the dashboard origin: ${list(toDashboard.map(short))}`,
    ),
  );

  // ---- G3: CORS absent, framing headers rewritten, on every panel response
  const answered = panelHops.filter((h) => h.status > 0);
  const raw = answered.filter((h) => h.hasExtraInfo);
  const cors = answered.filter((h) => Object.keys(h.rawResponseHeaders || {}).some((k) => k.startsWith("access-control-")));
  R.push(
    check(
      "cors.absent",
      cors.length === 0 && raw.length > 0,
      `no panel response carries an Access-Control-* header (${answered.length} responses inspected, ${raw.length} with raw headers; refusals included)`,
      cors.length ? `Access-Control-* on a panel response: ${list(cors.map(short))}` : "no raw response headers were captured, so CORS absence is unproven",
    ),
  );
  const want = expectedAncestors(ctx);
  const framingBad = [];
  for (const h of answered) {
    const hd = h.rawResponseHeaders || {};
    const why = [];
    if ("x-frame-options" in hd) why.push("X-Frame-Options present");
    if ((hd["x-content-type-options"] || "").toLowerCase() !== "nosniff") why.push("no nosniff");
    if ((hd["referrer-policy"] || "").toLowerCase() !== "no-referrer") why.push("referrer-policy != no-referrer");
    const fa = cspDirective(headerLines(hd, "content-security-policy"), "frame-ancestors");
    if (fa.length !== 1) why.push(`${fa.length} frame-ancestors directives`);
    else {
      const isRefusal = h.status >= 400;
      const same = setEq(fa[0], want);
      const none = fa[0].length === 1 && fa[0][0] === "'none'";
      if (!(same || (isRefusal && none))) why.push(`frame-ancestors ${fa[0].join(" ")}`);
    }
    if (why.length) framingBad.push(`${short(h)} -> ${h.status}: ${why.join(", ")}`);
  }
  R.push(
    check(
      "headers.framing",
      framingBad.length === 0 && answered.length > 0,
      `every panel response: no X-Frame-Options, frame-ancestors = the dashboard origin(s) ${want.join(" ")} (refusals: 'none'), nosniff, no-referrer`,
      framingBad.length ? `framing headers wrong on ${framingBad.length} response(s): ${list(framingBad, 4)}` : "no panel responses to check",
    ),
  );

  // ---- G3: the ticket
  const t = ctx.ticket || "";
  if (!t) {
    R.push(fail("ticket.once", "the driver did not learn the ticket (no POST /panels/{id}/session answer, no iframe src) — the single-use rule is unchecked"));
  } else {
    // (the "spent" phase replays this very ticket on purpose; ticket.spent in the driver judges that answer)
    const withT = reqs.filter((h) => h.phase !== "spent" && h.url.includes(t));
    const inLocation = reqs.filter((h) => h.phase !== "spent" && h.redirectedTo && h.redirectedTo.includes(t));
    const inReferer = reqs.filter((h) => h.phase !== "spent" && String((h.rawRequestHeaders || {}).referer || "").includes(t));
    const finals = [ctx.frameUrls && ctx.frameUrls.panel, ctx.frameUrls && ctx.frameUrls.dashboard].filter((u) => u && u.includes(t));
    const swap = withT[0];
    const swapOk = withT.length === 1 && swap.origin === ctx.panelOrigin && swap.type === "Document" && swap.method === "GET" && swap.status === 303 && swap.redirectedTo && !swap.redirectedTo.includes(t);
    R.push(
      check(
        "ticket.once",
        swapOk && inLocation.length === 0 && inReferer.length === 0 && finals.length === 0,
        () => `the ticket appears in exactly one request URL — the swap (GET ${pathOf(swap.url).replace(/\?.*/, "")}?p111_ticket=… -> 303 ${swap.redirectedTo ? pathOf(swap.redirectedTo) : "?"}) — and in no Referer, Location or later document URL`,
        () => `ticket seen in ${withT.length} request URL(s) [${list(withT.map(short), 3)}], ${inLocation.length} Location, ${inReferer.length} Referer, ${finals.length} final document URL(s); swap ok=${swapOk}`,
      ),
    );
  }

  // ---- G3: the p111 cookie
  const swapHop = t ? reqs.find((h) => h.phase !== "spent" && h.url.includes(t)) : null;
  const setCookies = swapHop ? headerLines(swapHop.rawResponseHeaders, "set-cookie") : [];
  const cookieName = `p111_${ctx.panelId || "meshtastic"}`;
  const mine = setCookies.filter((c) => c.startsWith(`${cookieName}=`));
  const attrs = mine[0] ? mine[0].split(";").map((s) => s.trim()) : [];
  const attrNames = attrs.slice(1).map((a) => a.split("=")[0].toLowerCase());
  const rawOk =
    mine.length === 1 &&
    /^[0-9a-f]{32,}$/.test(attrs[0].split("=")[1] || "") &&
    attrNames.includes("httponly") &&
    attrs.some((a) => a.toLowerCase() === "samesite=strict") &&
    attrNames.includes("path") &&
    !attrNames.includes("domain") &&
    !attrNames.includes("max-age") &&
    !attrNames.includes("expires") &&
    !attrNames.includes("secure");
  R.push(
    check(
      "cookie.set-cookie",
      rawOk,
      `swap response: ${cookieName}=<hex>; ${attrs.slice(1).join("; ")} — HttpOnly, SameSite=Strict, no Domain (host-only), session cookie`,
      `Set-Cookie on the swap is wrong: ${JSON.stringify(setCookies)}`,
    ),
  );
  const jar = (ctx.cookies || []).filter((c) => c.name === cookieName);
  const jc = jar[0];
  R.push(
    check(
      "cookie.jar",
      jar.length === 1 && jc.httpOnly === true && jc.sameSite === "Strict" && jc.path === "/" && !String(jc.domain).startsWith(".") && jc.domain === ctx.nodeHost && jc.session === true,
      `browser cookie jar: ${cookieName} httpOnly, sameSite=Strict, host-only (domain ${jc && jc.domain}), path /, session`,
      `cookie jar entry wrong: ${JSON.stringify(jar)}`,
    ),
  );
  const docCookie = (ctx.panelDom && ctx.panelDom.cookie) ?? null;
  R.push(
    check(
      "cookie.js-blind",
      docCookie === "",
      "document.cookie inside the panel frame is empty — the session cookie is invisible to the panel's own JS",
      `document.cookie in the panel frame = ${JSON.stringify(docCookie)}`,
    ),
  );

  // ---- G3: the frame the dashboard draws
  const fr = ctx.iframes || [];
  const f0 = fr[0];
  const sb = f0 && f0.sandbox != null ? f0.sandbox.split(/\s+/).filter(Boolean) : null;
  const allowToks = f0 && f0.allow != null ? f0.allow.split(";").map((s) => s.trim()).filter(Boolean) : [];
  const wantSandbox = (ctx.manifest && ctx.manifest.sandboxDefault) || DEFAULT_SANDBOX;
  const wantAllow = (ctx.manifest && ctx.manifest.allow) || [];
  const srcOrigin = f0 ? (() => { try { return new URL(f0.src, ctx.dashboardOrigin).origin; } catch { return ""; } })() : "";
  R.push(
    check(
      "iframe.only-the-panel",
      fr.length === 1 && srcOrigin === ctx.panelOrigin,
      `the dashboard frames exactly one thing: the panel origin ${ctx.panelOrigin}`,
      `iframes in the dashboard DOM: ${JSON.stringify(fr.map((f) => f.src && f.src.replace(/p111_ticket=[^&]*/, "p111_ticket=…")))}`,
    ),
  );
  R.push(
    check(
      "iframe.sandbox",
      !!sb && setEq(sb, wantSandbox) && !sb.some((s) => TOP_NAV.test(s)),
      `iframe sandbox="${sb && sb.join(" ")}" = the manifest's "default" set, never allow-top-navigation`,
      `iframe sandbox=${JSON.stringify(f0 && f0.sandbox)}; manifest default set is "${wantSandbox.join(" ")}"`,
    ),
  );
  R.push(
    check(
      "iframe.allow",
      !!f0 && setEq(allowToks, wantAllow) && allowToks.every((a) => /^[a-z][a-z0-9-]*$/.test(a)),
      `iframe allow="${f0 && f0.allow}" = the manifest's frame.allow [${wantAllow.join(", ")}] as bare (frame-origin-scoped) feature names`,
      `iframe allow=${JSON.stringify(f0 && f0.allow)}; manifest frame.allow = [${wantAllow.join(", ")}]`,
    ),
  );

  // cookies are not port-scoped: the dashboard's s111 rides along to the panel origin on the same host (information)
  const toPanel = panelHops.filter((h) => h.phase === "panel");
  const withS111 = toPanel.filter((h) => /(^|;\s*)s111=/.test(String((h.rawRequestHeaders || {}).cookie || "")));
  if (withS111.length) {
    R.push(note("cookie.port-blind", `${withS111.length} of ${toPanel.length} requests to the panel origin carried the dashboard session cookie s111 (cookies ignore ports; it is HttpOnly so the panel's JS cannot read it, and the static + shim server never forwards it — a future generic proxy must strip s111 and p111_* as spec section 13 says)`));
  }

  // ---- G3: the dashboard's own CSP
  const dashDocs = reqs.filter((h) => h.origin === ctx.dashboardOrigin && h.type === "Document" && h.status === 200);
  const csps = dashDocs.map((h) => headerLines(h.rawResponseHeaders, "content-security-policy"));
  const fsrc = csps.map((c) => cspDirective(c, "frame-src"));
  const fanc = csps.map((c) => cspDirective(c, "frame-ancestors"));
  const okFsrc = dashDocs.length > 0 && fsrc.every((d) => d.length === 1 && setEq(d[0], [ctx.panelOrigin]));
  R.push(
    check(
      "csp.frame-src",
      okFsrc,
      `dashboard CSP frame-src lists only the enabled panel origin: frame-src ${ctx.panelOrigin} (${dashDocs.length} dashboard document responses)`,
      `dashboard CSP frame-src is ${JSON.stringify(fsrc.map((d) => d.map((x) => x.join(" "))))} (want exactly ${ctx.panelOrigin})`,
    ),
  );
  R.push(
    check(
      "csp.frame-ancestors-none",
      dashDocs.length > 0 && fanc.every((d) => d.length === 1 && setEq(d[0], ["'none'"])) && dashDocs.every((h) => String((h.rawResponseHeaders || {})["x-frame-options"] || "").toUpperCase() === "DENY"),
      "the dashboard itself still refuses to be framed (frame-ancestors 'none' + X-Frame-Options: DENY)",
      "the dashboard document lost frame-ancestors 'none' / X-Frame-Options: DENY",
    ),
  );

  // ---- the shim round trip the panel made
  const puts = panelInit.filter((h) => h.method === "PUT" && new URL(h.url).pathname === "/api/v1/toradio");
  const gets = panelInit.filter((h) => h.method === "GET" && new URL(h.url).pathname === "/api/v1/fromradio");
  const getsFull = gets.filter((h) => Number((h.rawResponseHeaders || {})["content-length"] || 0) > 0);
  R.push(
    check(
      "shim.roundtrip",
      puts.length >= 1 && puts.every((h) => h.status === 200) && gets.length >= (ctx.bundleKind === "real" ? 1 : 4) && gets.every((h) => h.status === 200) && (ctx.bundleKind === "real" || gets.every((h) => new URL(h.url).searchParams.get("all") === "true")),
      `shim round trip from the panel's own origin: ${puts.length}x PUT /api/v1/toradio -> 200, ${gets.length}x GET /api/v1/fromradio?all=true -> 200 (${getsFull.length} carried a frame)`,
      `shim calls: PUT toradio ${JSON.stringify(puts.map((h) => h.status))}, GET fromradio ${JSON.stringify(gets.map((h) => h.status))}`,
    ),
  );
  const dom = ctx.panelDom;
  if (ctx.bundleKind === "real") {
    // a real client has no DOM contract with this lint: its traffic (above) is the evidence; the inputs are information
    const authority = new URL(ctx.panelOrigin).host;
    const inputs = (dom && dom.inputs) || [];
    R.push(note("shim.panel-state", `real client: ${gets.length} GET /fromradio and ${puts.length} PUT /toradio from its own origin; the lint does not read its UI`));
    R.push(
      inputs.some((v) => String(v).includes(authority))
        ? ok("connect-dialog.prefill", `a field in the real client is prefilled with the panel authority ${authority}`)
        : note("connect-dialog.prefill", `no input in the real client holds the panel authority ${authority} (inputs: ${JSON.stringify(inputs.slice(0, 6))}) — the connect-dialog prefill is checked at build against the pinned tag (spec section 14)`),
    );
  } else {
    R.push(
      check(
        "shim.panel-state",
        !!dom && dom.state === "connected" && Number(dom.nodes) >= 3 && dom.put === "ok" && Number(dom.errors) === 0 && Number(dom.polls) >= 6,
        `the panel reached "connected" through the shim: board "${dom && dom.board}", ${dom && dom.nodes} nodes, ${dom && dom.polls} polls, 0 errors`,
        `panel state after the dwell: ${JSON.stringify(dom)}`,
      ),
    );
    if (ctx.scenario !== "canary") {
      R.push(
        check(
          "connect-dialog.prefill",
          !!dom && dom.host === new URL(ctx.panelOrigin).host,
          `connect-dialog prefill (placeholder client: location.host) = the panel authority ${new URL(ctx.panelOrigin).host}`,
          `connect-dialog prefill is ${JSON.stringify(dom && dom.host)}, panel authority is ${new URL(ctx.panelOrigin).host}`,
        ),
      );
    }
  }

  // ---- probes from other origins (G3: refused, no CORS, nothing readable)
  for (const p of ctx.probes || []) R.push(...evalProbe(p, ctx));

  // ---- the localhost scenario: the 308
  if (ctx.scenario === "localhost" && ctx.localhost) {
    const first = reqs.find((h) => h.url === ctx.localhost.startUrl);
    const loc = first && (first.rawResponseHeaders || {}).location;
    R.push(
      check(
        "localhost.308",
        !!first && first.status === 308 && !!loc && new URL(loc, first.url).origin === ctx.dashboardOrigin,
        `${ctx.localhost.startUrl} answered 308 -> ${loc} (the panel never sees Host: localhost)`,
        `GET ${ctx.localhost.startUrl} -> ${first ? first.status : "no request"} Location ${JSON.stringify(loc)}; wanted 308 to ${ctx.dashboardOrigin}`,
      ),
    );
    const sawLocalhostPanel = reqs.filter((h) => mainPhase(h) && hostOf(h.url) === "localhost" && !entry.has(h.url));
    R.push(check("localhost.no-panel-on-localhost", sawLocalhostPanel.length === 0, "after the 308 nothing was ever requested on localhost (panels never see it)", `requests to localhost besides the 308: ${list(sawLocalhostPanel.map(short))}`));
  }
  return R;
}

/** A probe hop's request carried a p111_* cookie? */
const cookieOf = (h) => /(^|;\s*)p111_/.test(String((h.rawRequestHeaders || {}).cookie || ""));
const fetchSiteOf = (h) => (h.rawRequestHeaders || {})["sec-fetch-site"];
const noCorsHeaders = (h) => !Object.keys(h.rawResponseHeaders || {}).some((k) => k.startsWith("access-control-"));

function evalProbe(p, ctx) {
  const id = `probe.${p.name}`;
  const hops = p.hops || [];
  const out = [];
  const verdict = (good, goodText, badText) => {
    if (!good && p.residual) return note(id, `RESIDUAL (${p.residual}): ${badText}`); // known and accepted by design: information
    const r = good ? ok(id, goodText) : fail(id, badText);
    if (!good && p.gap) {
      r.gap = p.gap;
      if (ctx.knownGaps) {
        r.level = "note";
        r.text = `KNOWN GAP (${p.gap}): ${badText}`;
      } else r.text = `GAP (${p.gap}): ${badText}`;
    }
    return r;
  };
  switch (p.kind) {
    case "refused": {
      // the request reached the panel and its guard refused it; the page that sent it could not read the answer
      const target = hops.find((h) => h.method === (p.expectMethod || "GET")) || hops[0];
      const why = [];
      if (!target) why.push("the request never reached the panel");
      else {
        if (!p.statuses.includes(target.status)) why.push(`answered ${target.status} (wanted ${p.statuses.join(" or ")})`);
        if (!noCorsHeaders(target)) why.push("the refusal carries Access-Control-* headers");
        if (p.expectFetchSite && fetchSiteOf(target) !== p.expectFetchSite) why.push(`Sec-Fetch-Site ${fetchSiteOf(target)} (wanted ${p.expectFetchSite})`);
        if (p.cookieSent !== undefined && cookieOf(target) !== p.cookieSent) why.push(`the panel cookie was ${cookieOf(target) ? "attached" : "withheld"} (wanted ${p.cookieSent ? "attached" : "withheld"})`);
      }
      const js = p.js || {};
      if (!p.noJs && (p.opaque ? js.type !== "opaque" : js.ok !== false)) why.push(`the sending page saw ${JSON.stringify(js)}`);
      out.push(
        verdict(
          why.length === 0,
          `${p.title}: ${target && target.method} ${target && pathOf(target.url)} from ${p.fromOrigin} -> ${target && target.status}; Sec-Fetch-Site ${(target && fetchSiteOf(target)) || "not sent"}; panel cookie ${target && cookieOf(target) ? "attached" : "withheld"}; no Access-Control-*${p.noJs ? "" : p.opaque ? "; the page got an opaque response" : `; the page's fetch rejected (${js.err})`}`,
          `${p.title}: ${why.join("; ")}${target ? ` [${target.method} ${pathOf(target.url)}, Sec-Fetch-Site ${fetchSiteOf(target) || "not sent"}, Origin ${(target.rawRequestHeaders || {}).origin || "-"}]` : ""}`,
        ),
      );
      break;
    }
    case "preflight": {
      const pre = hops.find((h) => h.method === "OPTIONS");
      const sent = hops.find((h) => h.method === "PUT" && h.status > 0);
      out.push(
        verdict(
          !!pre && pre.status === 403 && noCorsHeaders(pre) && !sent && !!p.js && p.js.ok === false,
          `${p.title}: OPTIONS preflight from ${p.fromOrigin} -> 403 with no Access-Control-*, so the PUT was never sent`,
          `${p.title}: preflight ${pre ? pre.status : "not seen"}, CORS ${pre ? (noCorsHeaders(pre) ? "absent" : "PRESENT") : "?"}, PUT answered: ${!!sent}, page JS ${JSON.stringify(p.js)}`,
        ),
      );
      break;
    }
    case "csp-or-refused": {
      // the dashboard's own CSP (connect-src 'self') stops it before it is sent; if a request did go out the panel refused it
      const reached = hops.filter((h) => h.status > 0);
      const csp = p.cspMessages || [];
      const good = !!p.js && p.js.ok === false && reached.every((h) => h.status === 403 && noCorsHeaders(h)) && (csp.length > 0 || reached.length > 0);
      out.push(
        verdict(
          good,
          `${p.title}: ${csp.length ? "blocked by the dashboard's own CSP (connect-src 'self') before it was sent" : `refused ${reached.map((h) => h.status).join(",")} by the panel`}; the page's fetch rejected (${p.js && p.js.err})`,
          `${p.title}: page JS ${JSON.stringify(p.js)}, CSP messages ${csp.length}, requests that reached the panel ${JSON.stringify(reached.map((h) => [h.method, h.status]))}`,
        ),
      );
      break;
    }
    default:
      out.push(fail(id, `unknown probe kind ${p.kind}`));
  }
  return out;
}

/**
 * Table of what was requested: phase, origin, request (method + path), type, status, count. Tickets are redacted and
 * query strings elided. With `collapse` (the dashboard origin) that origin's own page chatter — static files, API reads,
 * the event stream — is folded into one line per phase; its panel calls (/panels) stay listed.
 */
export function requestTable(reqs, { collapse = null } = {}) {
  const rows = new Map();
  const folded = new Map();
  for (const h of reqs) {
    if (!/^(https?|wss?):/i.test(h.url)) continue;
    let u;
    try {
      u = new URL(h.url);
    } catch {
      continue;
    }
    if (collapse && u.origin === collapse && !u.pathname.includes("/panels")) {
      const k = `${h.phase}\t${u.origin}`;
      if (!folded.has(k)) folded.set(k, { n: 0, types: new Map() });
      const f = folded.get(k);
      f.n++;
      f.types.set(h.type, (f.types.get(h.type) || 0) + 1);
      continue;
    }
    const q = u.search ? (/p111_ticket=/.test(u.search) ? "?p111_ticket=<redacted>" : "?…") : "";
    const status = h.status ? String(h.status) : h.failed ? `x ${h.failed.blockedReason || h.failed.errorText || "failed"}` : "-";
    const key = [h.phase, u.origin, `${h.method} ${u.pathname}${q}`, h.type, status].join("\t");
    rows.set(key, (rows.get(key) || 0) + 1);
  }
  const lines = [...rows.entries()].map(([k, n]) => [...k.split("\t"), String(n)]);
  for (const [k, f] of folded) {
    const [phase, origin] = k.split("\t");
    lines.push([phase, origin, `(the dashboard's own page: ${[...f.types.entries()].map(([t, n]) => `${t}x${n}`).join(" ")})`, "-", "-", String(f.n)]);
  }
  lines.sort((a, b) => a[0].localeCompare(b[0]) || a[1].localeCompare(b[1]) || a[2].localeCompare(b[2]));
  const head = ["phase", "origin", "request", "type", "status", "n"];
  const widths = head.map((_, i) => Math.max(head[i].length, ...lines.map((l) => l[i].length)));
  const fmt = (cols) => cols.map((c, i) => c.padEnd(widths[i])).join("  ").trimEnd();
  return [fmt(head), fmt(widths.map((w) => "-".repeat(w))), ...lines.map(fmt)].join("\n");
}

export function print(results, out = (s) => process.stdout.write(`${s}\n`)) {
  let fails = 0;
  for (const r of results) {
    if (r.level === "FAIL") fails++;
    out(`${r.level}: ${r.id}: ${r.text}`);
  }
  return fails;
}
