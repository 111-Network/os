// Service page logic: the state matrix (what each state shows and offers) and the detail flattening.
import { describe, expect, it } from "vitest";
import { buildFixtures } from "../src/lib/api/mock/fixtures";
import type { ServiceRecord } from "../src/lib/api/types";
import { ApiError } from "../src/lib/api/problem";
import {
  acceptRestart,
  agoText,
  beginRestart,
  flattenDetail,
  formatDuration,
  humanizeKey,
  NO_SETUP_SERVICES,
  reasonText,
  RESTART_IDLE,
  restartButtonLabel,
  restartErrorText,
  servicePageState,
  settleRestart,
  sinceText,
} from "../src/views/services/model";

const NOW = Date.parse("2026-10-13T12:00:00Z");
const fx = buildFixtures(NOW);
const svc = (id: ServiceRecord["id"], over: Partial<ServiceRecord> = {}): ServiceRecord => ({ ...structuredClone(fx.services.find((s) => s.id === id) as ServiceRecord), ...over });

describe("servicePageState", () => {
  it("live: control panel, log and restart only when the registry lists them, no hand-off", () => {
    const p = servicePageState(svc("transport"), true);
    expect(p).toMatchObject({ body: "live", handoff: null, log: true, restart: true, kbPath: "/kb/projects/reticulum.md", embedUrl: null });
    const noActions = servicePageState(svc("kb"), true);
    expect(noActions).toMatchObject({ body: "live", log: false, restart: false, kbPath: null });
  });

  it("live with embed_url offers the link-out; unsafe schemes are dropped", () => {
    expect(servicePageState(svc("radio", { state: "live", embed_url: "https://111.local:9443/" }), true).embedUrl).toBe("https://111.local:9443/");
    expect(servicePageState(svc("radio", { state: "live", embed_url: "javascript:alert(1)" }), true).embedUrl).toBeNull();
    expect(servicePageState(svc("radio", { state: "live", embed_url: "//evil.example/x" }), true).embedUrl).toBeNull();
  });

  it("unset and runnable: Set up with the agent (setup goal)", () => {
    const p = servicePageState(svc("radio"), true);
    expect(p.body).toBe("unset");
    expect(p.handoff).toEqual({ goal: "setup", label: "Set up with the agent", mode: "agent" });
    expect(p.log).toBe(false);
    expect(p.restart).toBe(false);
  });

  it("unset and runnable with the agent not live: Show steps", () => {
    expect(servicePageState(svc("radio"), false).handoff).toEqual({ goal: "setup", label: "Show steps", mode: "steps" });
  });

  it("unset but cannot run here: unavailable, no button at all", () => {
    for (const agentLive of [true, false]) {
      const p = servicePageState(svc("codeagent"), agentLive);
      expect(p.body).toBe("unavailable");
      expect(p.handoff).toBeNull();
    }
    expect(servicePageState(svc("radio", { can_run_here: false }), true)).toMatchObject({ body: "unavailable", handoff: null });
  });

  it("unset with no setup step (internet gateway): explains, offers nothing", () => {
    expect(NO_SETUP_SERVICES.has("gateway")).toBe(true);
    const p = servicePageState(svc("gateway", { state: "unset" }), true);
    expect(p).toMatchObject({ body: "unset", handoff: null });
  });

  it("error: Fix with the agent (fix goal), log and restart offered", () => {
    const p = servicePageState(svc("agent", { state: "error" }), true);
    expect(p.body).toBe("error");
    expect(p.handoff).toEqual({ goal: "fix", label: "Fix with the agent", mode: "agent" });
    expect(p.log).toBe(true);
    expect(p.restart).toBe(true);
  });

  it("error with child_degraded also links to Settings -> Restart node; other errors do not", () => {
    expect(servicePageState(svc("agent", { state: "error", reason_code: "child_degraded" }), true).nodeRestartLink).toBe(true);
    expect(servicePageState(svc("agent", { state: "error", reason_code: "child_backoff" }), true).nodeRestartLink).toBe(false);
    expect(servicePageState(svc("agent", { state: "live", reason_code: "child_degraded" }), true).nodeRestartLink).toBe(false);
  });

  it("Restart is offered exactly when the registry lists the restart action (live and error)", () => {
    expect(servicePageState(svc("model", { state: "error", actions: ["log"] }), true).restart).toBe(false);
    expect(servicePageState(svc("model", { state: "error", actions: ["restart", "log"] }), true).restart).toBe(true);
    expect(servicePageState(svc("model", { state: "live", actions: ["restart"] }), true).restart).toBe(true);
    expect(servicePageState(svc("model", { state: "unset", actions: ["restart"] }), true).restart).toBe(false);
  });

  it("error with the agent not live (including the agent's own page): Show steps", () => {
    expect(servicePageState(svc("agent", { state: "error" }), false).handoff).toEqual({ goal: "fix", label: "Show steps", mode: "steps" });
  });

  it("starting: no hand-off, the log is still offered", () => {
    const p = servicePageState(svc("model", { state: "starting" }), true);
    expect(p).toMatchObject({ body: "starting", handoff: null, log: true, restart: false });
  });

  it("matrix: state x can_run_here x agentLive never offers a button where there is nothing to do", () => {
    for (const state of ["live", "starting", "unset", "error"] as const) {
      for (const can of [true, false]) {
        for (const agentLive of [true, false]) {
          const p = servicePageState(svc("radio", { state, can_run_here: can }), agentLive);
          if (state === "live" || state === "starting") expect(p.handoff).toBeNull();
          if (state === "unset" && !can) expect(p.handoff).toBeNull();
          if (p.handoff) expect(p.handoff.mode).toBe(agentLive ? "agent" : "steps");
        }
      }
    }
  });

  it("refuses a kb_entry that tries to leave the knowledge base", () => {
    expect(servicePageState(svc("radio", { kb_entry: "../etc/passwd" }), true).kbPath).toBeNull();
    expect(servicePageState(svc("radio", { kb_entry: "devices/heltec wifi.md" }), true).kbPath).toBe("/kb/devices/heltec%20wifi.md");
  });
});

describe("flattenDetail", () => {
  it("humanizes keys and stringifies scalars", () => {
    expect(flattenDetail({ child: "running", port: 11182, model_reachable: true, model_id: "qwen" }, NOW)).toEqual([
      { label: "Child", value: "running" },
      { label: "Port", value: "11182" },
      { label: "Model reachable", value: "yes" },
      { label: "Model id", value: "qwen" },
    ]);
  });

  it("hides empty values", () => {
    expect(flattenDetail({ a: "", b: null, c: undefined, d: [], e: {}, f: "  ", g: 0, h: false }, NOW)).toEqual([
      { label: "G", value: "0" },
      { label: "H", value: "no" },
    ]);
  });

  it("flattens one level of nesting and leaves deeper structures as compact JSON", () => {
    expect(flattenDetail({ probe: { status: 204, deep: { a: 1 } }, devices: ["a", "b"], mixed: [1, { x: 2 }] }, NOW)).toEqual([
      { label: "Probe status", value: "204" },
      { label: "Probe deep", value: '{"a":1}' },
      { label: "Devices", value: "a, b" },
      { label: "Mixed", value: '[1,{"x":2}]' },
    ]);
  });

  it("shows timestamps as time ago", () => {
    expect(flattenDetail({ last_probe: new Date(NOW - 40_000).toISOString() }, NOW)).toEqual([{ label: "Last probe", value: "40 s ago" }]);
  });

  it("clips very long strings, caps rows and never throws on odd input", () => {
    expect(flattenDetail({ blob: "x".repeat(1000) }, NOW)[0]?.value).toHaveLength(301);
    const many = Object.fromEntries(Array.from({ length: 100 }, (_, i) => [`k${i}`, i]));
    expect(flattenDetail(many, NOW)).toHaveLength(40);
    expect(flattenDetail(null, NOW)).toEqual([]);
    const circular: Record<string, unknown> = {};
    circular.self = { loop: circular };
    expect(() => flattenDetail(circular, NOW)).not.toThrow();
  });

  it("does not repeat a label", () => {
    expect(flattenDetail({ probe_status: 1, probe: { status: 2 } }, NOW)).toHaveLength(1);
  });

  it("works on every real fixture record", () => {
    for (const s of fx.services) expect(() => flattenDetail(s.detail, NOW)).not.toThrow();
    expect(flattenDetail(svc("gateway").detail, NOW).map((r) => r.label)).toEqual(["Probe", "Interval sec", "Last probe"]);
  });
});

describe("words and time", () => {
  it("humanizeKey", () => {
    expect(humanizeKey("model_id")).toBe("Model id");
    expect(humanizeKey("lastProbe")).toBe("Last probe");
    expect(humanizeKey("next-retry-sec")).toBe("Next retry sec");
    expect(humanizeKey("")).toBe("");
  });

  it("formatDuration", () => {
    expect(formatDuration(0)).toBe("0 s");
    expect(formatDuration(59)).toBe("59 s");
    expect(formatDuration(60)).toBe("1 min");
    expect(formatDuration(3600)).toBe("1 h");
    expect(formatDuration(7 * 3600 + 412)).toBe("7 h 6 min");
    expect(formatDuration(2 * 86400)).toBe("2 d");
    expect(formatDuration(2 * 86400 + 5 * 3600 + 900)).toBe("2 d 5 h");
    expect(formatDuration(-5)).toBe("0 s");
    expect(formatDuration(Number.NaN)).toBe("0 s");
  });

  it("agoText and sinceText", () => {
    const at = (ms: number) => new Date(NOW - ms).toISOString();
    expect(agoText(at(3000), NOW)).toBe("just now");
    expect(agoText(at(40_000), NOW)).toBe("40 s ago");
    expect(agoText(at(5 * 60_000), NOW)).toBe("5 min ago");
    expect(agoText(at(7 * 3_600_000), NOW)).toBe("7 h ago");
    expect(agoText(at(3 * 86_400_000), NOW)).toBe("3 d ago");
    expect(agoText(new Date(NOW + 60_000).toISOString(), NOW)).toBe("just now");
    expect(agoText(null, NOW)).toBe("unknown");
    expect(agoText("garbage", NOW)).toBe("unknown");
    expect(sinceText(at(7 * 3_600_000), NOW)).toBe("since 7 h ago");
  });
});

describe("restart button phases", () => {
  it("reads Restart, then Restarting… from the click until a newer service list arrives", () => {
    expect(restartButtonLabel(RESTART_IDLE)).toBe("Restart");
    const sending = beginRestart(4);
    expect(restartButtonLabel(sending)).toBe("Restarting…");
    expect(settleRestart(sending, 9)).toEqual(sending); // not accepted yet: a frame cannot end it
    const waiting = acceptRestart(sending);
    expect(waiting).toEqual({ kind: "waiting", rev: 4 });
    expect(restartButtonLabel(waiting)).toBe("Restarting…");
    expect(settleRestart(waiting, 4)).toEqual(waiting); // same list as at the click
    expect(settleRestart(waiting, 5)).toEqual(RESTART_IDLE); // the next services frame
  });

  it("a frame that arrives before the 202 still ends the wait once the 202 lands", () => {
    const waiting = acceptRestart(beginRestart(4));
    expect(settleRestart(waiting, 6)).toEqual(RESTART_IDLE);
    expect(acceptRestart(RESTART_IDLE)).toEqual(RESTART_IDLE);
  });

  it("explains a failed restart in plain words", () => {
    const problem = (code: ConstructorParameters<typeof ApiError>[0]["code"], status: number, detail?: string) =>
      new ApiError({ type: "about:blank", title: "Title", status, code, ...(detail ? { detail } : {}) });
    expect(restartErrorText(problem("not_supervised", 409))).toMatch(/does not run this one itself/);
    expect(restartErrorText(ApiError.network(new Error("x")))).toMatch(/could not be reached/);
    expect(restartErrorText(problem("loopback_only", 403))).toBe("Available on the node itself.");
    expect(restartErrorText(problem("internal", 500, "boom"))).toBe("boom");
    expect(restartErrorText(new Error("odd"))).toBe("odd");
    expect(restartErrorText("odd")).toBe("Something went wrong.");
  });
});

describe("reasonText (NOT SET reads as unfinished, not as a failure)", () => {
  const transport = (reason_code: string, state: ServiceRecord["state"] = "unset") =>
    svc("transport", { state, reason_code, reason: "Mesh transport is off: python3 is installed, but the Reticulum environment could not be set up." });

  it("rewords the transport's venv_unavailable and no_python while unset", () => {
    expect(reasonText(transport("venv_unavailable"))).toBe("Python is installed. The Reticulum environment is not set up yet.");
    expect(reasonText(transport("no_python"))).toBe("Python is not installed on this machine yet. The mesh transport needs it.");
    expect(reasonText(transport("venv_unavailable"))).not.toMatch(/could not/);
  });
  it("leaves the daemon's text alone for every other code and for any state but unset", () => {
    expect(reasonText(transport("venv_unavailable", "error"))).toMatch(/could not be set up/);
    expect(reasonText(transport("no_radio"))).toMatch(/could not be set up/);
    expect(reasonText(svc("radio"))).toBe(svc("radio").reason);
  });
});
