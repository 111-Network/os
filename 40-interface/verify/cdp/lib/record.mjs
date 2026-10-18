// Network recording over CDP: attach to every target (page, iframe, worker), keep every Network.* event
// tagged with the phase the driver was in, then fold the events into one entry per request hop.
//
// A request hop = one `requestWillBeSent`; a redirect is a second hop under the same requestId. The raw
// headers (Set-Cookie, Sec-Fetch-*, Cookie, the CORS headers that Chrome hides from page JS) come from the
// `*ExtraInfo` events, which can arrive before or after their request, so folding happens at the end.
import { sleep } from "./cdp.mjs";

const lowerKeys = (h) => Object.fromEntries(Object.entries(h || {}).map(([k, v]) => [k.toLowerCase(), v]));

export function originOf(url) {
  try {
    const u = new URL(url);
    if (u.protocol === "http:" || u.protocol === "https:" || u.protocol === "ws:" || u.protocol === "wss:") return u.origin;
    return `${u.protocol}`; // data:, blob:, about:, chrome-extension: …: no network origin
  } catch {
    return "invalid:";
  }
}

export const isNetworkUrl = (url) => /^(https?|wss?):/i.test(url);

export function hostOf(url) {
  try {
    return new URL(url).hostname.replace(/^\[|\]$/g, "");
  } catch {
    return "";
  }
}

export class Recorder {
  constructor(browser) {
    this.b = browser;
    this.raw = [];
    this.phase = "setup";
    this.sessions = new Map(); // sessionId -> { label, targetType, url }
    this.frames = new Map(); // frameId -> { url, parentId, sid }
    this.popups = []; // windows opened by a page (window.open, target=_blank): recorded too
    this.off = browser.on((m, p, sid) => this.onEvent(m, p, sid));
    browser.send("Target.setDiscoverTargets", { discover: true }).catch(() => {});
  }

  setPhase(phase) {
    this.phase = phase;
  }

  onEvent(method, params, sid) {
    if (method === "Target.targetCreated") {
      const ti = params.targetInfo;
      // a tab this driver opens has no opener; a window a page opened does — attach to it so its requests are not missed
      if (ti.type === "page" && ti.openerId && !this.popups.some((p) => p.targetId === ti.targetId)) {
        this.popups.push({ targetId: ti.targetId, url: ti.url, openerId: ti.openerId, phase: this.phase });
        this.b
          .send("Target.attachToTarget", { targetId: ti.targetId, flatten: true })
          .then(({ sessionId }) => {
            this.sessions.set(sessionId, { label: "popup", targetType: "page", url: ti.url, parent: null });
            return this.enableOn(sessionId);
          })
          .catch(() => {});
      }
      return;
    }
    if (method === "Target.attachedToTarget") {
      const { sessionId, targetInfo } = params;
      const parent = this.sessions.get(sid);
      this.sessions.set(sessionId, {
        label: `${parent ? parent.label : "?"}/${targetInfo.type}`,
        targetType: targetInfo.type,
        url: targetInfo.url,
        parent: sid,
      });
      // iframes, workers and out-of-process frames: record them too
      this.enableOn(sessionId).catch(() => {});
      return;
    }
    if (method === "Page.frameNavigated") {
      const f = params.frame;
      this.frames.set(f.id, { url: f.url, parentId: f.parentId || null, sid });
    }
    if (method.startsWith("Network.") || method === "Page.frameNavigated" || method === "Page.frameDetached" || method === "Log.entryAdded") {
      this.raw.push({ t: Date.now(), phase: this.phase, sid, label: (this.sessions.get(sid) || {}).label || "?", method, params });
    }
  }

  async enableOn(sid) {
    const send = (m, p) => this.b.send(m, p, sid).catch(() => {});
    await send("Network.enable", { maxPostDataSize: 4096 });
    await send("Network.setCacheDisabled", { cacheDisabled: true });
    await send("Page.enable", {});
    await send("Runtime.enable", {});
    await send("Log.enable", {}); // CSP violations are reported here, not as network events
    await send("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true });
    await send("Runtime.runIfWaitingForDebugger", {});
  }

  /** Open a tab (in `browserContextId` when given), attach, enable the domains, return { sid, targetId }. */
  async openTab(url, label, browserContextId) {
    const params = { url: "about:blank" };
    if (browserContextId) params.browserContextId = browserContextId;
    const { targetId } = await this.b.send("Target.createTarget", params);
    const { sessionId } = await this.b.send("Target.attachToTarget", { targetId, flatten: true });
    this.sessions.set(sessionId, { label, targetType: "page", url: "about:blank", parent: null });
    await this.enableOn(sessionId);
    if (url && url !== "about:blank") await this.b.send("Page.navigate", { url }, sessionId);
    return { sid: sessionId, targetId };
  }

  async closeTab(targetId) {
    await this.b.send("Target.closeTarget", { targetId }).catch(() => {});
  }

  /** The folded entries, one per request hop, in the order the browser announced them. */
  requests() {
    const hopsBy = new Map(); // `${sid}|${requestId}` -> hops[]
    const reqExtra = new Map();
    const respExtra = new Map();
    const order = [];
    const keyOf = (ev) => `${ev.sid}|${ev.params.requestId}`;
    const push = (m, k, v) => {
      if (!m.has(k)) m.set(k, []);
      m.get(k).push(v);
    };
    for (const ev of this.raw) {
      const p = ev.params;
      switch (ev.method) {
        case "Network.requestWillBeSent": {
          const k = keyOf(ev);
          const hops = hopsBy.get(k) || [];
          hopsBy.set(k, hops);
          if (p.redirectResponse && hops.length) {
            const prev = hops[hops.length - 1];
            prev.response = foldResponse(p.redirectResponse);
            prev.redirectedTo = p.request.url;
          }
          const hop = {
            key: k,
            hop: hops.length,
            t: ev.t,
            phase: ev.phase,
            label: ev.label,
            sid: ev.sid,
            requestId: p.requestId,
            method: p.request.method,
            url: p.request.url,
            origin: originOf(p.request.url),
            docUrl: p.documentURL || "",
            docOrigin: originOf(p.documentURL || ""),
            type: p.type || "Other",
            frameId: p.frameId || null,
            initiator: p.initiator || {},
            headers: lowerKeys(p.request.headers),
            hasPostData: !!p.request.hasPostData,
            response: null,
            failed: null,
            redirectedTo: null,
          };
          hops.push(hop);
          order.push(hop);
          break;
        }
        case "Network.requestWillBeSentExtraInfo":
          push(reqExtra, keyOf(ev), lowerKeys(p.headers));
          break;
        case "Network.responseReceivedExtraInfo":
          push(respExtra, keyOf(ev), { status: p.statusCode, headers: lowerKeys(p.headers), blockedCookies: p.blockedCookies || [] });
          break;
        case "Network.responseReceived": {
          const hops = hopsBy.get(keyOf(ev));
          if (hops && hops.length) hops[hops.length - 1].response = foldResponse(p.response);
          break;
        }
        case "Network.loadingFailed": {
          const hops = hopsBy.get(keyOf(ev));
          if (hops && hops.length) {
            hops[hops.length - 1].failed = {
              errorText: p.errorText,
              canceled: !!p.canceled,
              blockedReason: p.blockedReason || null,
              cors: p.corsErrorStatus ? p.corsErrorStatus.corsError : null,
            };
          }
          break;
        }
        default:
      }
    }
    for (const [k, hops] of hopsBy) {
      const rq = reqExtra.get(k) || [];
      const rs = respExtra.get(k) || [];
      hops.forEach((h, i) => {
        h.rawRequestHeaders = { ...h.headers, ...(rq[i] || {}) };
        const ex = rs[i];
        const base = h.response ? h.response.headers : {};
        h.rawResponseHeaders = ex ? ex.headers : base;
        h.status = ex ? ex.status : h.response ? h.response.status : 0;
        h.blockedCookies = ex ? ex.blockedCookies : [];
        h.hasExtraInfo = !!ex;
      });
    }
    return order;
  }

  /** Browser log lines (CSP violations among them) seen in `phase`. */
  logs(phase) {
    return this.raw.filter((e) => e.method === "Log.entryAdded" && e.phase === phase).map((e) => `${e.params.entry.source}: ${e.params.entry.text}`);
  }

  webSockets() {
    return this.raw
      .filter((e) => e.method === "Network.webSocketCreated")
      .map((e) => ({ url: e.params.url || "", phase: e.phase, label: e.label, method: e.method }));
  }

  /** Frames seen navigating (frameId -> last URL). */
  frameTable() {
    return [...this.frames.entries()].map(([id, f]) => ({ id, ...f }));
  }

  async settle(ms = 300) {
    await sleep(ms);
  }

  stop() {
    this.off();
  }
}

function foldResponse(r) {
  return {
    status: r.status,
    statusText: r.statusText || "",
    headers: lowerKeys(r.headers),
    mimeType: r.mimeType || "",
    remoteIPAddress: r.remoteIPAddress || "",
    remotePort: r.remotePort || 0,
    fromCache: !!(r.fromDiskCache || r.fromPrefetchCache || r.fromServiceWorker),
  };
}

/** Header values may be several lines in the raw (ExtraInfo) form; split them. */
export const headerLines = (h, name) => {
  const v = h ? h[name] : undefined;
  return v === undefined || v === null ? [] : String(v).split("\n");
};
