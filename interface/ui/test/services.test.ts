// Service page logic: the state matrix (what each state shows and offers) and the detail flattening.
import { describe, expect, it } from "vitest";
import { buildFixtures } from "../src/lib/api/mock/fixtures";
import type { ServiceRecord } from "../src/lib/api/types";
import { agoText, flattenDetail, formatDuration, humanizeKey, NO_SETUP_SERVICES, servicePageState, sinceText } from "../src/views/services/model";

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
