import { describe, expect, it } from "vitest";
import type { Adapter } from "../src/lib/api/adapter";
import { createHttpAdapter, panelsSeenAtLoad, resetPanelsAtLoadForTests } from "../src/lib/api/http";
import { ApiError } from "../src/lib/api/problem";
import type { ChatFrame } from "../src/lib/api/types";
import { operations } from "./contract.gen";
import { bytes, collect, jsonResponse, problemResponse, splitBytes, sseResponse } from "./helpers/fakes";
import { validate } from "./helpers/validate";

interface Call {
  method: string;
  url: string;
  path: string;
  query: URLSearchParams;
  headers: Record<string, string>;
  body: unknown;
  init: RequestInit;
}

function recorder(respond: (call: Call) => Response | Promise<Response>) {
  const calls: Call[] = [];
  const fetchFn = (async (input: RequestInfo | URL, init: RequestInit = {}) => {
    const url = new URL(String(input), "http://node.test");
    const call: Call = {
      method: init.method ?? "GET",
      url: String(input),
      path: url.pathname,
      query: url.searchParams,
      headers: (init.headers ?? {}) as Record<string, string>,
      body: typeof init.body === "string" ? JSON.parse(init.body) : undefined,
      init,
    };
    calls.push(call);
    return respond(call);
  }) as typeof fetch;
  return { calls, fetch: fetchFn };
}

const adapterWith = (respond: (c: Call) => Response | Promise<Response>) => {
  const r = recorder(respond);
  return { ...r, adapter: createHttpAdapter({ fetch: r.fetch }) };
};

const STATUS = { schema: 1, node: { version: "1" } };
/** A public NodeId: 64 lowercase hex. */
const PEER = "ab".repeat(32);

describe("request basics", () => {
  it("GETs JSON same-origin without caching", async () => {
    const { adapter, calls } = adapterWith(() => jsonResponse(STATUS));
    expect(await adapter.status()).toEqual(STATUS);
    expect(calls[0]).toMatchObject({ method: "GET", url: "/api/v1/status" });
    expect(calls[0]!.headers).toMatchObject({ Accept: "application/json" });
    expect(calls[0]!.headers["Content-Type"]).toBeUndefined();
    expect(calls[0]!.init).toMatchObject({ credentials: "same-origin", cache: "no-store" });
  });

  it("prefixes the configured base", async () => {
    const r = recorder(() => jsonResponse(STATUS));
    await createHttpAdapter({ fetch: r.fetch, base: "http://127.0.0.1:11180/" }).status();
    expect(r.calls[0]!.url).toBe("http://127.0.0.1:11180/api/v1/status");
  });

  it("sends Content-Type: application/json on every mutating call, with or without a body", async () => {
    const { adapter, calls } = adapterWith((c) => (c.path.endsWith("/auth/logout") ? new Response(null, { status: 204 }) : jsonResponse({ schema: 1 })));
    await adapter.restart();
    await adapter.updateCheck();
    await adapter.auth.logout();
    await adapter.patchSettings({ alias: "x" });
    for (const c of calls) expect(c.headers["Content-Type"]).toBe("application/json");
    expect(calls[0]!.body).toBeUndefined();
    expect(calls[3]!.body).toEqual({ alias: "x" });
    expect(calls[3]!.method).toBe("PATCH");
  });

  it("unwraps the services and proposals envelopes", async () => {
    const { adapter } = adapterWith((c) => jsonResponse(c.path.endsWith("services") ? { schema: 1, services: [{ id: "agent" }] } : { schema: 1, proposals: [{ id: "p" }] }));
    expect(await adapter.services()).toEqual([{ id: "agent" }]);
    expect(await adapter.proposals()).toEqual([{ id: "p" }]);
  });

  it("encodes path segments and query values", async () => {
    const { adapter, calls } = adapterWith((c) =>
      c.path.includes("/kb") ? new Response("# md", { status: 200 }) : jsonResponse(c.path.endsWith("/proposals") ? { proposals: [] } : {}),
    );
    await adapter.kb.entry("projects/meshtastic.md");
    await adapter.kb.entry("a b/ü.md");
    await adapter.proposal("prop a/b");
    await adapter.proposals("all");
    await adapter.serviceLog("transport", 20);
    await adapter.chat.history("conv-1");
    expect(calls[0]!.url).toBe("/api/v1/kb/entry?path=projects%2Fmeshtastic.md");
    expect(calls[1]!.query.get("path")).toBe("a b/ü.md");
    expect(calls[2]!.path).toBe("/api/v1/proposals/prop%20a%2Fb");
    expect(calls[3]!.query.get("status")).toBe("all");
    expect(calls[4]!.url).toBe("/api/v1/services/transport/log?n=20");
    expect(calls[5]!.url).toBe("/api/v1/chat/history?conversation_id=conv-1");
  });

  it("omits optional query parameters that were not given", async () => {
    const { adapter, calls } = adapterWith(() => jsonResponse({ schema: 1, proposals: [] }));
    await adapter.proposals();
    await adapter.serviceLog("agent");
    expect(calls[0]!.url).toBe("/api/v1/proposals");
    expect(calls[1]!.url).toBe("/api/v1/services/agent/log");
  });

  it("reads knowledge base text", async () => {
    const { adapter } = adapterWith(() => new Response("# index\n", { headers: { "content-type": "text/plain" } }));
    expect(await adapter.kb.index()).toBe("# index\n");
  });

  it("decides a proposal with the decision body", async () => {
    const { adapter, calls } = adapterWith(() => jsonResponse({ id: "prop-1" }));
    await adapter.decide("prop-1", "accepted");
    expect(calls[0]).toMatchObject({ method: "POST", path: "/api/v1/proposals/prop-1/decision", body: { decision: "accepted" } });
  });
});

describe("devices", () => {
  it("lists, reads, renames and forgets with the right verbs", async () => {
    const device = { id: "usb-1", alias: "Bench radio" };
    const { adapter, calls } = adapterWith((c) => {
      if (c.method === "DELETE") return new Response(null, { status: 204 });
      return jsonResponse(c.path === "/api/v1/devices" ? { schema: 1, devices: [device] } : device);
    });
    expect(await adapter.devices()).toEqual([device]);
    expect(await adapter.device("usb-1")).toEqual(device);
    expect(await adapter.patchDevice("usb-1", { alias: "Bench radio" })).toEqual(device);
    await adapter.patchDevice("usb-1", { alias: null });
    expect(await adapter.forgetDevice("usb-1")).toBeUndefined();
    expect(calls.map((c) => `${c.method} ${c.path}`)).toEqual([
      "GET /api/v1/devices",
      "GET /api/v1/devices/usb-1",
      "PATCH /api/v1/devices/usb-1",
      "PATCH /api/v1/devices/usb-1",
      "DELETE /api/v1/devices/usb-1",
    ]);
    expect(calls[2]!.body).toEqual({ alias: "Bench radio" });
    expect(calls[3]!.body).toEqual({ alias: null });
    for (const c of calls.slice(2)) expect(c.headers["Content-Type"]).toBe("application/json");
    expect(calls[4]!.body).toBeUndefined();
  });

  it("encodes device ids that contain slashes or colons", async () => {
    const { adapter, calls } = adapterWith(() => jsonResponse({}));
    await adapter.device("usb:1/a b");
    expect(calls[0]!.path).toBe("/api/v1/devices/usb%3A1%2Fa%20b");
  });

  it("rejects a missing device and a malformed list", async () => {
    const a = adapterWith(() => problemResponse(404, "not_found", "No such device"));
    expect(await a.adapter.device("x").catch((e: unknown) => e)).toMatchObject({ status: 404, code: "not_found" });
    expect(await a.adapter.forgetDevice("x").catch((e: unknown) => e)).toMatchObject({ code: "not_found" });
    const b = adapterWith(() => jsonResponse({ schema: 1 }));
    expect(await b.adapter.devices().catch((e: unknown) => e)).toMatchObject({ code: "upstream_error" });
  });
});

describe("errors", () => {
  it("rejects with ApiError carrying the problem+json", async () => {
    const { adapter } = adapterWith(() => problemResponse(409, "already_decided", "Already decided", "It was accepted."));
    const err = await adapter.decide("p", "dismissed").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ status: 409, code: "already_decided", message: "It was accepted." });
    expect((err as ApiError).problem.type).toBe("about:blank");
  });

  it("maps 401 to unauthenticated on every route, even with an empty body", async () => {
    const { adapter } = adapterWith(() => new Response(null, { status: 401 }));
    for (const call of [() => adapter.status(), () => adapter.services(), () => adapter.kb.index(), () => adapter.restart(), () => adapter.chat.history("c")]) {
      const err = await call().catch((e: unknown) => e);
      expect(err).toMatchObject({ code: "unauthenticated", status: 401 });
    }
  });

  it("maps a refused connection to code network", async () => {
    const adapter = createHttpAdapter({ fetch: (async () => Promise.reject(new TypeError("Failed to fetch"))) as typeof fetch });
    const err = await adapter.status().catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ code: "network", status: 0 });
  });

  it("lets an abort through unchanged", async () => {
    const abort = Object.assign(new Error("aborted"), { name: "AbortError" });
    const adapter = createHttpAdapter({ fetch: (async () => Promise.reject(abort)) as typeof fetch });
    expect(await adapter.status().catch((e: unknown) => e)).toBe(abort);
  });

  it("rejects a 200 that is not JSON", async () => {
    const { adapter } = adapterWith(() => new Response("<html>spa fallback</html>", { status: 200, headers: { "content-type": "text/html" } }));
    expect(await adapter.status().catch((e: unknown) => e)).toMatchObject({ code: "upstream_error" });
  });
});

describe("chat.send", () => {
  const FRAMES =
    'event: token\ndata: {"text":"Hel"}\n\n' +
    'event: token\ndata: {"text":"lo "}\n\n' +
    'event: tool\ndata: {"name":"kb.read"}\n\n' +
    'event: token\ndata: {"text":"wörld"}\n\n' +
    'event: done\ndata: {"conversation_id":"conv-1","reply":"world","tool_trace":[{"name":"kb.read","ok":true}]}\n\n';

  const req = { conversation_id: "conv-1", message: "hi" };

  it("posts with stream:true and yields typed frames, split at awkward byte boundaries", async () => {
    const total = bytes(FRAMES).length;
    const offsets = [3, 17, 18, 52, 90, Math.floor(total / 2), total - 5];
    const { adapter, calls } = adapterWith(() => sseResponse(splitBytes(FRAMES, offsets)));
    const frames = await collect(adapter.chat.send(req));
    expect(frames.map((f) => f.event)).toEqual(["token", "token", "tool", "token", "done"]);
    expect(frames[3]).toEqual({ event: "token", data: { text: "wörld" } });
    expect(frames[4]).toEqual({ event: "done", data: { conversation_id: "conv-1", reply: "world", tool_trace: [{ name: "kb.read", ok: true }] } });
    expect(calls[0]).toMatchObject({ method: "POST", path: "/api/v1/chat", body: { conversation_id: "conv-1", message: "hi", stream: true } });
    expect(calls[0]!.headers).toMatchObject({ Accept: "text/event-stream", "Content-Type": "application/json" });
  });

  it("stops after done even if the server keeps talking", async () => {
    const { adapter } = adapterWith(() => sseResponse([bytes(FRAMES + 'event: token\ndata: {"text":"late"}\n\n')]));
    const frames = await collect(adapter.chat.send(req));
    expect(frames[frames.length - 1]!.event).toBe("done");
    expect(frames).toHaveLength(5);
  });

  it("yields an error frame and ends", async () => {
    const body = 'event: token\ndata: {"text":"a"}\n\nevent: error\ndata: {"error":"model fell over","code":"upstream_error"}\n\n';
    const { adapter } = adapterWith(() => sseResponse([bytes(body)]));
    expect(await collect(adapter.chat.send(req))).toEqual<ChatFrame[]>([
      { event: "token", data: { text: "a" } },
      { event: "error", data: { error: "model fell over", code: "upstream_error" } },
    ]);
  });

  it("synthesizes an error frame when the stream ends without a terminal frame", async () => {
    const { adapter } = adapterWith(() => sseResponse([bytes('event: token\ndata: {"text":"a"}\n\n')]));
    const frames = await collect(adapter.chat.send(req));
    expect(frames).toHaveLength(2);
    expect(frames[1]).toMatchObject({ event: "error", data: { code: "stream_ended" } });
  });

  it("ignores unknown event names and comments, and reports an unreadable frame", async () => {
    const ok = ": keep-alive\n\nevent: future\ndata: {}\n\n" + FRAMES;
    const a = adapterWith(() => sseResponse([bytes(ok)]));
    expect((await collect(a.adapter.chat.send(req))).map((f) => f.event)).toEqual(["token", "token", "tool", "token", "done"]);
    const b = adapterWith(() => sseResponse([bytes("event: token\ndata: {oops\n\n")]));
    expect(await collect(b.adapter.chat.send(req))).toEqual([{ event: "error", data: { error: expect.any(String), code: "bad_frame" } }]);
  });

  it("with stream:false posts stream:false and yields one synthesized done", async () => {
    const reply = { schema: 1, conversation_id: "conv-1", reply: "all good", tool_trace: [{ name: "kb.read", ok: true }] };
    const { adapter, calls } = adapterWith(() => jsonResponse(reply));
    const frames = await collect(adapter.chat.send({ ...req, stream: false }));
    expect(frames).toEqual([{ event: "done", data: { conversation_id: "conv-1", reply: "all good", tool_trace: [{ name: "kb.read", ok: true }] } }]);
    expect(calls[0]!.body).toMatchObject({ stream: false });
  });

  it("tolerates a JSON reply to a streaming request", async () => {
    const { adapter } = adapterWith(() => jsonResponse({ schema: 1, conversation_id: "c", reply: "r", tool_trace: [] }));
    expect(await collect(adapter.chat.send(req))).toEqual([{ event: "done", data: { conversation_id: "c", reply: "r", tool_trace: [] } }]);
  });

  it("rejects pre-stream failures with ApiError on the first iteration, never as SSE", async () => {
    const { adapter } = adapterWith(() => problemResponse(503, "model_unavailable", "Model unavailable", "llama-server is down"));
    const it = adapter.chat.send(req)[Symbol.asyncIterator]();
    const err = await it.next().catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ code: "model_unavailable", status: 503 });
  });

  it("rejects 401 and 400 pre-stream the same way", async () => {
    const a = adapterWith(() => new Response("", { status: 401 }));
    expect(await collect(a.adapter.chat.send(req)).catch((e: unknown) => e)).toMatchObject({ code: "unauthenticated" });
    const b = adapterWith(() => problemResponse(400, "bad_request", "Bad"));
    expect(await collect(b.adapter.chat.send(req)).catch((e: unknown) => e)).toMatchObject({ code: "bad_request" });
  });

  it("turns a connection reset mid-stream into a network ApiError", async () => {
    const stream = new ReadableStream<Uint8Array>({
      start(c) {
        c.enqueue(bytes('event: token\ndata: {"text":"a"}\n\n'));
      },
      pull() {
        throw new TypeError("network error");
      },
    });
    const { adapter } = adapterWith(() => new Response(stream, { headers: { "content-type": "text/event-stream" } }));
    const seen: string[] = [];
    const err = await (async () => {
      for await (const f of adapter.chat.send(req)) seen.push(f.event);
    })().catch((e: unknown) => e);
    expect(seen).toEqual(["token"]);
    expect(err).toMatchObject({ code: "network" });
  });

  it("aborts the request and cancels the body when the consumer stops early", async () => {
    let signal: AbortSignal | undefined;
    let cancelled = false;
    const { adapter } = adapterWith((c) => {
      signal = c.init.signal ?? undefined;
      return sseResponse([bytes('event: token\ndata: {"text":"a"}\n\n')], () => (cancelled = true), {}, true);
    });
    for await (const f of adapter.chat.send(req)) {
      expect(f.event).toBe("token");
      break;
    }
    expect(signal?.aborted).toBe(true);
    expect(cancelled).toBe(true);
  });

  it("aborts when the caller's signal fires", async () => {
    const ctl = new AbortController();
    let signal: AbortSignal | undefined;
    const { adapter } = adapterWith((c) => {
      signal = c.init.signal ?? undefined;
      return sseResponse([bytes('event: token\ndata: {"text":"a"}\n\n')], undefined, {}, true);
    });
    const it = adapter.chat.send(req, { signal: ctl.signal })[Symbol.asyncIterator]();
    await it.next();
    ctl.abort();
    expect(signal?.aborted).toBe(true);
    await it.return?.();
  });
});

// Every operation in api/openapi.yaml has an adapter method that hits the right method, path,
// query and request body; no adapter method talks to a route the contract does not define.
describe("adapter <-> openapi.yaml", () => {
  const MIN_JSON: Record<string, unknown> = {
    "/api/v1/services": { schema: 1, services: [] },
    "/api/v1/proposals": { schema: 1, proposals: [] },
    "/api/v1/devices": { schema: 1, devices: [] },
    "/api/v1/messages/threads": { schema: 1, threads: [] },
    [`/api/v1/messages/threads/${PEER}`]: { schema: 1, peer: PEER, request: false, messages: [], more: false },
    "/api/v1/contacts": { schema: 1, contacts: [] },
    "/api/v1/nearby": { schema: 1, sources: [] },
    "/api/v1/panels": { schema: 1, panels: [] },
  };
  const CALLS: Record<string, (a: Adapter) => Promise<unknown>> = {
    getStatus: (a) => a.status(),
    listServices: (a) => a.services(),
    getService: (a) => a.service("agent"),
    getServiceLog: (a) => a.serviceLog("agent", 20),
    restartService: (a) => a.restartService("model"),
    chat: (a) => collect(a.chat.send({ conversation_id: "conv-1", message: "hi", stream: false })),
    chatHistory: (a) => a.chat.history("conv-1"),
    listDevices: (a) => a.devices(),
    getDevice: (a) => a.device("usb-7c1e0b94a2d3"),
    patchDevice: (a) => a.patchDevice("usb-7c1e0b94a2d3", { alias: "Bench radio" }),
    forgetDevice: (a) => a.forgetDevice("usb-7c1e0b94a2d3"),
    flashDevice: (a) => a.flashDevice("usb-7c1e0b94a2d3", { sha256: "9f".repeat(32), confirm: "Bench radio" }),
    identifyDevice: (a) => a.identifyDevice("usb-7c1e0b94a2d3", { consent: true }),
    agentStatus: (a) => a.agentStatus(),
    agentTask: (a) => a.agentTask({ kind: "diagnose", refs: ["svc:transport"], note: null }),
    listProposals: (a) => a.proposals("decided"),
    getProposal: (a) => a.proposal("prop-1"),
    decideProposal: (a) => a.decide("prop-1", "accepted"),
    getSettings: (a) => a.settings(),
    patchSettings: (a) => a.patchSettings({ alias: "kitchen", offer: "accepted" }),
    restartNode: (a) => a.restart(),
    updateCheck: (a) => a.updateCheck(),
    kbIndex: (a) => a.kb.index(),
    kbEntry: (a) => a.kb.entry("projects/meshtastic.md"),
    authSession: (a) => a.auth.session(),
    authPair: (a) => a.auth.pair("AAAA-BBBB"),
    authLogout: (a) => a.auth.logout(),
    listThreads: (a) => a.messages.threads(),
    getThread: (a) => a.messages.thread(PEER, { sinceSeq: 4, limit: 50 }),
    sendMessage: (a) => a.messages.send({ to: PEER, text: "hello" }),
    retryMessage: (a) => a.messages.retry("msg-01j9z3k8m2q4"),
    // F2's contract pass adds `cancelMessage` (POST /messages/{id}/cancel, 202); this table follows the contract either way
    ...("cancelMessage" in operations ? { cancelMessage: (a: Adapter) => a.messages.cancel!("msg-01j9z3k8m2q4") } : {}),
    listContacts: (a) => a.contacts.list(),
    addContact: (a) => a.contacts.add({ node_id: PEER }),
    getOwnCard: (a) => a.contacts.ownCard(),
    getContact: (a) => a.contacts.get(PEER),
    patchContact: (a) => a.contacts.patch(PEER, { alias: "Sam", pinned: true, read_seq: 3 }),
    forgetContact: (a) => a.contacts.forget(PEER),
    getFabric: (a) => a.fabric.status(),
    enrollHub: (a) => a.fabric.enrollHub({ address: "192.168.1.20:11260", code: "K7M29X4P" }),
    offerHub: (a) => a.fabric.offerHub(),
    dismissHubOffer: (a) => a.fabric.dismissHubOffer(),
    patchFabricSettings: (a) => a.fabric.patchSettings({ hub_role: true, quotas: { msg: null } }),
    listNearby: (a) => a.nearby(),
    listPanels: (a) => a.panels.list(),
    openPanelSession: (a) => a.panels.openSession("meshtastic"),
  };
  const toRegex = (tpl: string) => new RegExp(`^${tpl.replace(/\{[^}]+\}/g, "[^/]+")}$`);

  it("covers every operation except the event stream", () => {
    expect(Object.keys(CALLS).sort()).toEqual(Object.keys(operations).filter((id) => id !== "events").sort());
  });

  for (const [id, call] of Object.entries(CALLS)) {
    it(`${id} matches its contract operation`, async () => {
      const op = operations[id]!;
      const { adapter, calls } = adapterWith((c) => {
        if (op.success === 204) return new Response(null, { status: 204 });
        const wantsJson = op.contentType === "application/json" || (c.body as { stream?: boolean } | undefined)?.stream === false;
        if (!wantsJson) return new Response("text", { status: op.success });
        return jsonResponse(MIN_JSON[c.path] ?? { schema: 1, conversation_id: "conv-1", reply: "r", tool_trace: [] }, op.success);
      });
      await call(adapter);
      expect(calls).toHaveLength(1);
      const c = calls[0]!;
      expect(c.method).toBe(op.method);
      expect(c.path).toMatch(toRegex(op.path));
      for (const q of op.query) if (q.required) expect(c.query.has(q.name), `required query ${q.name}`).toBe(true);
      for (const name of c.query.keys()) expect(op.query.map((q) => q.name)).toContain(name);
      if (op.requestSchema) expect(validate(c.body, op.requestSchema)).toEqual([]);
      else expect(c.body).toBeUndefined();
    });
  }
});


describe("cancelling a queued message (POST /messages/{id}/cancel, spec 4)", () => {
  it("posts to the cancel route of that message with the CSRF header and no body, and resolves on 202 without reading a reply", async () => {
    const { adapter, calls } = adapterWith(() => new Response(null, { status: 202 }));
    await expect(adapter.messages.cancel!("msg-01j9z3k8m2q4")).resolves.toBeUndefined();
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ method: "POST", path: "/api/v1/messages/msg-01j9z3k8m2q4/cancel", body: undefined });
    expect(calls[0]!.headers["Content-Type"]).toBe("application/json");
  });

  it("a JSON 202 is fine too, and the id is encoded into the path", async () => {
    const { adapter, calls } = adapterWith(() => jsonResponse({ schema: 1, id: "x" }, 202));
    await adapter.messages.cancel!("a/b c");
    expect(calls[0]!.path).toBe("/api/v1/messages/a%2Fb%20c/cancel");
  });

  it("a refusal is the node's own problem: 409 once the message has left", async () => {
    const { adapter } = adapterWith(() => problemResponse(409, "bad_request", "Not cancellable", "That message has already left this node."));
    const err = await adapter.messages.cancel!("msg-1").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).status).toBe(409);
    expect((err as ApiError).code).toBe("bad_request");
    expect((err as ApiError).problem.detail).toBe("That message has already left this node.");
  });
});

describe("the first panel list of a page load", () => {
  it("is remembered from the real adapter's GET /panels (the page's best evidence of the CSP it was loaded with); later lists do not replace it", async () => {
    resetPanelsAtLoadForTests();
    try {
      const off = { id: "meshtastic", port: 11190, entry: "/", needs_secure_context: false, allow: [], optional_hosts: [], state: { enabled: false, listening: false, health: "unknown" } };
      const on = { ...off, state: { enabled: true, listening: true, health: "up" } };
      let list = [off];
      const { adapter } = adapterWith(() => jsonResponse({ schema: 1, panels: list }));
      expect(panelsSeenAtLoad()).toBeNull();
      await adapter.panels.list();
      expect([...panelsSeenAtLoad()!]).toEqual([["meshtastic", false]]);
      list = [on];
      expect(await adapter.panels.list()).toEqual([on]); // the caller gets the new list...
      expect([...panelsSeenAtLoad()!]).toEqual([["meshtastic", false]]); // ...the page keeps what it first saw
    } finally {
      resetPanelsAtLoadForTests();
    }
  });

  it("a failed first request records nothing", async () => {
    resetPanelsAtLoadForTests();
    const { adapter } = adapterWith(() => problemResponse(503, "upstream_error", "Unavailable"));
    await adapter.panels.list().catch(() => undefined);
    expect(panelsSeenAtLoad()).toBeNull();
  });
});
