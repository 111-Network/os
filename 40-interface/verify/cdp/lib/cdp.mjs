// Minimal Chrome DevTools Protocol client (no npm deps; Node >= 22 for the built-in WebSocket).
//
//   const chrome = await launchChrome({ chrome: "/usr/bin/google-chrome" });
//   const b = await Browser.connect(chrome.wsUrl);
//   const { targetId } = await b.send("Target.createTarget", { url: "about:blank" });
//
// One flat connection to the browser target; page and iframe/worker targets are attached with
// `flatten: true`, so every command takes an optional `sessionId` and every event arrives with one.
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Poll `fn` until it returns a truthy value; throws `what` on timeout. */
export async function waitFor(fn, { timeout = 15000, interval = 100, what = "condition" } = {}) {
  const t0 = Date.now();
  let last;
  for (;;) {
    try {
      last = await fn();
      if (last) return last;
    } catch (e) {
      last = e;
    }
    if (Date.now() - t0 > timeout) {
      throw new Error(`timed out after ${timeout} ms waiting for ${what}${last instanceof Error ? `: ${last.message}` : ""}`);
    }
    await sleep(interval);
  }
}

/** First existing Chrome/Chromium: $CHROME, google-chrome, then playwright's chromium (read-only). */
export function findChrome(env = process.env) {
  const cands = [env.CHROME, "/usr/bin/google-chrome", "/usr/bin/google-chrome-stable", "/usr/bin/chromium", "/usr/bin/chromium-browser"];
  const pw = join(env.HOME || "", ".cache/ms-playwright");
  try {
    for (const d of readdirSync(pw).sort().reverse()) {
      if (d.startsWith("chromium-")) cands.push(join(pw, d, "chrome-linux/chrome"), join(pw, d, "chrome-linux64/chrome"));
    }
  } catch {
    /* no playwright cache */
  }
  return cands.find((c) => c && existsSync(c)) || null;
}

/** Start headless Chrome on a fixed debugging port (the namespace is private). Returns { wsUrl, proc, dir, stop() }. */
export async function launchChrome({ chrome, port = 9222, extraArgs = [] }) {
  const dir = mkdtempSync(join(tmpdir(), "111-chrome-"));
  const args = [
    "--headless=new",
    "--no-sandbox", // root in a user namespace; the namespace is the sandbox
    "--disable-gpu",
    "--disable-dev-shm-usage",
    `--remote-debugging-port=${port}`,
    "--remote-debugging-address=127.0.0.1",
    `--user-data-dir=${dir}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-sync",
    "--disable-default-apps",
    "--disable-domain-reliability",
    "--disable-client-side-phishing-detection",
    "--metrics-recording-only",
    "--password-store=basic",
    "--use-mock-keychain",
    "--disable-features=HttpsUpgrades,HttpsFirstBalancedModeAutoEnable,Translate,MediaRouter,OptimizationHints",
    ...extraArgs,
    "about:blank",
  ];
  const proc = spawn(chrome, args, { stdio: "ignore", detached: false });
  let exited = null;
  proc.on("exit", (code, sig) => {
    exited = { code, sig };
  });
  const wsUrl = await waitFor(
    async () => {
      if (exited) throw new Error(`chrome exited early (${JSON.stringify(exited)})`);
      const r = await fetch(`http://127.0.0.1:${port}/json/version`, { signal: AbortSignal.timeout(1000) }).catch(() => null);
      if (!r || !r.ok) return null;
      return (await r.json()).webSocketDebuggerUrl || null;
    },
    { timeout: 30000, interval: 150, what: "chrome's debugging endpoint" },
  );
  return {
    wsUrl,
    proc,
    dir,
    async stop() {
      try {
        proc.kill("SIGTERM");
      } catch {
        /* gone */
      }
      for (let i = 0; i < 30 && !exited; i++) await sleep(100);
      if (!exited) {
        try {
          proc.kill("SIGKILL");
        } catch {
          /* gone */
        }
      }
      try {
        rmSync(dir, { recursive: true, force: true });
      } catch {
        /* best effort */
      }
    },
  };
}

export class Browser {
  static async connect(wsUrl) {
    const ws = new WebSocket(wsUrl);
    await new Promise((res, rej) => {
      ws.addEventListener("open", res, { once: true });
      ws.addEventListener("error", () => rej(new Error(`cannot open ${wsUrl}`)), { once: true });
    });
    return new Browser(ws);
  }

  constructor(ws) {
    this.ws = ws;
    this.nextId = 1;
    this.pending = new Map();
    this.handlers = new Set();
    this.closed = false;
    ws.addEventListener("message", (ev) => {
      let m;
      try {
        m = JSON.parse(typeof ev.data === "string" ? ev.data : Buffer.from(ev.data).toString("utf8"));
      } catch {
        return;
      }
      if (m.id !== undefined) {
        const p = this.pending.get(m.id);
        if (!p) return;
        this.pending.delete(m.id);
        if (m.error) p.rej(Object.assign(new Error(`${p.method}: ${m.error.message}`), { cdp: m.error }));
        else p.res(m.result || {});
      } else if (m.method) {
        for (const h of this.handlers) {
          try {
            h(m.method, m.params || {}, m.sessionId || null);
          } catch (e) {
            process.stderr.write(`cdp handler error: ${e && e.stack ? e.stack : e}\n`);
          }
        }
      }
    });
    ws.addEventListener("close", () => {
      this.closed = true;
      for (const p of this.pending.values()) p.rej(new Error(`${p.method}: connection closed`));
      this.pending.clear();
    });
  }

  /** Subscribe to every event: fn(method, params, sessionId). Returns an unsubscribe function. */
  on(fn) {
    this.handlers.add(fn);
    return () => this.handlers.delete(fn);
  }

  send(method, params = {}, sessionId = undefined, timeout = 30000) {
    if (this.closed) return Promise.reject(new Error(`${method}: connection closed`));
    const id = this.nextId++;
    const msg = { id, method, params };
    if (sessionId) msg.sessionId = sessionId;
    return new Promise((res, rej) => {
      const t = setTimeout(() => {
        this.pending.delete(id);
        rej(new Error(`${method}: no answer in ${timeout} ms`));
      }, timeout);
      this.pending.set(id, {
        method,
        res: (v) => (clearTimeout(t), res(v)),
        rej: (e) => (clearTimeout(t), rej(e)),
      });
      this.ws.send(JSON.stringify(msg));
    });
  }

  /** Evaluate an expression in a session's page (or in a given execution context); the value comes back by value. */
  async evaluate(sessionId, expression, { contextId, awaitPromise = true, timeout = 30000 } = {}) {
    const params = { expression, returnByValue: true, awaitPromise };
    if (contextId) params.contextId = contextId;
    const r = await this.send("Runtime.evaluate", params, sessionId, timeout);
    if (r.exceptionDetails) {
      const d = r.exceptionDetails;
      throw new Error(`evaluate: ${(d.exception && d.exception.description) || d.text}`);
    }
    return r.result ? r.result.value : undefined;
  }

  close() {
    try {
      this.ws.close();
    } catch {
      /* closed */
    }
  }
}
