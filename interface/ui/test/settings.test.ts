// Pure-function tests for Settings (Essentials rows, loopback-only rule, layer status matrix, ask
// prompt) and Logs (tab selection, line cleaning, summaries).
import { describe, expect, it } from "vitest";
import { ApiError } from "../src/lib/api/problem";
import { buildFixtures } from "../src/lib/api/mock/fixtures";
import type { LogTail, ServiceRecord, Settings, Status } from "../src/lib/api/types";
import { LAYERS, layerBySlug } from "../src/lib/layers";
import { cleanLines, emptyLogText, followText, hasNoSource, logServices, tailSummary, LOG_POLL_MS } from "../src/views/logs/model";
import {
  BACKUP_COMMAND,
  BACKUP_NOTE,
  LAN_LOCKED_TEXT,
  LOOPBACK_ONLY_TEXT,
  aliasPatchValue,
  essentialRows,
  formatWhen,
  isLoopbackOnlyError,
  isSettingsLockedError,
  lanNote,
  lanRestartPending,
  layerAskPrompt,
  layerStatusRows,
  offerActions,
  offerLabel,
  problemText,
  restartConfirmText,
  sharingDisabled,
  updateRows,
} from "../src/views/settings/model";

const NOW = Date.parse("2026-10-13T12:00:00Z");
const fx = buildFixtures(NOW);
const status = (): Status => structuredClone(fx.status);
const services = (): ServiceRecord[] => structuredClone(fx.services);

function problem(code: ConstructorParameters<typeof ApiError>[0]["code"], status: number, detail?: string) {
  return new ApiError({ type: "about:blank", title: "Title", status, code, ...(detail ? { detail } : {}) });
}

describe("essentialRows", () => {
  it("has exactly five rows in the fixed order", () => {
    const rows = essentialRows();
    expect(rows.map((r) => r.id)).toEqual(["alias", "mode", "identity-backup", "restart", "update-check"]);
    expect(rows.map((r) => r.label)).toEqual(["Node name", "View mode", "Identity backup", "Restart node", "Check for updates"]);
  });
  it("every row has a label and help text", () => {
    for (const r of essentialRows()) {
      expect(r.label.length).toBeGreaterThan(0);
      expect(r.help.length).toBeGreaterThan(10);
    }
  });
  it("identity backup carries the exact instruction, command and never-reveal sentence", () => {
    const row = essentialRows()[2]!;
    expect(row.help).toBe("Your identity is 12 words. Back them up once, from the node itself:");
    expect(row.note).toBe("The dashboard never shows or sends your keys.");
    expect(BACKUP_NOTE).toBe(row.note);
    expect(BACKUP_COMMAND).toBe("sudo -u node111 111 identity backup");
  });
  it("returns a fresh array so a caller cannot reorder the shared list", () => {
    const a = essentialRows();
    a.reverse();
    a[0]!.label = "changed";
    expect(essentialRows().map((r) => r.id)[0]).toBe("alias");
    expect(essentialRows()[0]!.label).toBe("Node name");
  });
});

describe("loopback-only rule", () => {
  it("disables sharing only when the node says the session is not loopback", () => {
    const s = status();
    expect(sharingDisabled(s)).toBe(false);
    s.auth = { loopback: false, paired: true };
    expect(sharingDisabled(s)).toBe(true);
    s.auth = { loopback: false, paired: false };
    expect(sharingDisabled(s)).toBe(true);
  });
  it("never throws on unknown status and does not disable on it", () => {
    expect(sharingDisabled(null)).toBe(false);
    expect(sharingDisabled(undefined)).toBe(false);
    expect(sharingDisabled({} as Status)).toBe(false);
  });
  it("uses the spec text", () => {
    expect(LOOPBACK_ONLY_TEXT).toBe("available on the node itself");
  });
  it("a 403 loopback_only ApiError reads as the same text", () => {
    const e = problem("loopback_only", 403, "Change this setting from the machine the node runs on.");
    expect(isLoopbackOnlyError(e)).toBe(true);
    expect(problemText(e)).toBe(LOOPBACK_ONLY_TEXT);
  });
  it("other problems keep their own detail, then title", () => {
    expect(isLoopbackOnlyError(problem("bad_request", 400))).toBe(false);
    expect(problemText(problem("bad_request", 400, "must be at most 64 characters"))).toBe("must be at most 64 characters");
    expect(problemText(problem("not_found", 404))).toBe("Title");
    expect(problemText(ApiError.network(new Error("refused")))).toMatch(/could not be reached/);
    expect(problemText(new Error("boom"))).toBe("boom");
    expect(problemText("x")).toBe("Something went wrong.");
  });
});

describe("small rules", () => {
  it("restart confirm names the node: alias, then label, then a fallback", () => {
    const s = status();
    expect(restartConfirmText(s)).toBe("Restart Workshop laptop");
    s.node.alias = null;
    expect(restartConfirmText(s)).toBe("Restart k7m2-9x4p-aa");
    s.node.label = null;
    expect(restartConfirmText(s)).toBe("Restart this node");
    expect(restartConfirmText(null)).toBe("Restart this node");
  });
  it("alias patch sends null when empty or blank, trimmed text otherwise", () => {
    expect(aliasPatchValue("")).toBeNull();
    expect(aliasPatchValue("   ")).toBeNull();
    expect(aliasPatchValue("  Roof node ")).toBe("Roof node");
  });
  it("a 409 settings_locked reads as the pinned note, not a generic error", () => {
    const e = problem("settings_locked", 409, "DASHBOARD_LAN pins it.");
    expect(isSettingsLockedError(e)).toBe(true);
    expect(isSettingsLockedError(problem("loopback_only", 403))).toBe(false);
    expect(problemText(e)).toBe(LAN_LOCKED_TEXT);
    expect(LAN_LOCKED_TEXT).toBe("Pinned by DASHBOARD_LAN in the node's environment file — change it there.");
  });
  it("the LAN note follows the settings the node returned, never the requested value", () => {
    const s = status();
    s.bind.lan = false;
    const set: Settings = { schema: 1, alias: null, lan_enabled: true, lan_locked: false, offer: null };
    // asked for off, node answered on (pinned): the sentence must say on, and the toggle reads on too
    expect(lanNote(s, set)).toEqual({ kind: "restart", text: "LAN sharing on. Restart the node to apply." });
    expect(lanNote(s, { ...set, lan_enabled: false })).toBeNull(); // matches the bind: nothing pending
    s.bind.lan = true;
    expect(lanNote(s, { ...set, lan_enabled: false })).toEqual({ kind: "restart", text: "LAN sharing off. Restart the node to apply." });
    expect(lanNote(s, set)).toBeNull();
    expect(lanNote(null, set)).toBeNull();
    expect(lanNote(s, null)).toBeNull();
  });
  it("a pinned setting shows the locked note, whatever the bind says", () => {
    const s = status();
    s.bind.lan = false;
    const set: Settings = { schema: 1, alias: null, lan_enabled: true, lan_locked: true, offer: null };
    expect(lanNote(s, set)).toEqual({ kind: "locked", text: LAN_LOCKED_TEXT });
    // a 409 seen before the settings were refetched counts too
    expect(lanNote(s, { ...set, lan_locked: false }, true)).toEqual({ kind: "locked", text: LAN_LOCKED_TEXT });
    expect(lanNote(null, null, true)).toEqual({ kind: "locked", text: LAN_LOCKED_TEXT });
  });
  it("a LAN change is pending until the bind catches up", () => {
    const s = status();
    const set: Settings = { schema: 1, alias: null, lan_enabled: true, lan_locked: false, offer: null };
    s.bind.lan = true;
    expect(lanRestartPending(s, set)).toBe(false);
    expect(lanRestartPending(s, { ...set, lan_enabled: false })).toBe(true);
    expect(lanRestartPending(null, set)).toBe(false);
    expect(lanRestartPending(s, null)).toBe(false);
  });
  it("offer: accept until accepted, dismiss only while offered", () => {
    expect(offerActions("offered")).toEqual({ accept: true, dismiss: true });
    expect(offerActions("dismissed")).toEqual({ accept: true, dismiss: false });
    expect(offerActions("accepted")).toEqual({ accept: false, dismiss: false });
    expect(offerActions(null)).toEqual({ accept: false, dismiss: false });
    expect(offerLabel(null)).toBe("not offered on this node");
    expect(offerLabel("accepted")).toBe("accepted");
  });
  it("update rows show state, last check and last result in plain words", () => {
    const s = status();
    s.update = { ...s.update, state: "idle", last_check: null, last_result: null };
    expect(updateRows(s)).toEqual([
      { label: "State", value: "idle" },
      { label: "Last check", value: "never" },
      { label: "Last result", value: "none yet" },
    ]);
    s.update = { ...s.update, state: "staged", last_check: "2026-10-13T09:00:00Z", last_result: "up_to_date", pending_version: "2026.10.20" };
    const rows = updateRows(s);
    expect(rows[2]).toEqual({ label: "Last result", value: "up to date" });
    expect(rows[1]!.value).toBe(formatWhen("2026-10-13T09:00:00Z"));
    expect(rows[3]).toEqual({ label: "Pending version", value: "2026.10.20" });
    expect(updateRows(null)).toEqual([{ label: "State", value: "waiting for the node" }]);
    expect(formatWhen("not a date")).toBe("not a date");
  });
});

describe("layerStatusRows matrix", () => {
  const labels = (n: number, ctx = {}) => layerStatusRows(n, status(), ctx).map((r) => r.label);

  it("never throws for any layer on a null status", () => {
    for (let n = -1; n <= 10; n++) {
      expect(() => layerStatusRows(n, null)).not.toThrow();
      expect(() => layerStatusRows(n, undefined, { services: null, openProposals: null })).not.toThrow();
      expect(Array.isArray(layerStatusRows(n, null))).toBe(true);
    }
  });
  it("never throws on a sparse status", () => {
    const sparse = { node: {}, bind: { urls: [] }, update: {}, auth: {}, intelligence: {}, health: {} } as unknown as Status;
    for (let n = 0; n <= 9; n++) expect(() => layerStatusRows(n, sparse)).not.toThrow();
  });
  it("every layer 0..9 yields at least one labelled row with a value", () => {
    for (let n = 0; n <= 9; n++) {
      const rows = layerStatusRows(n, status(), { services: services(), openProposals: 1 });
      expect(rows.length).toBeGreaterThan(0);
      for (const r of rows) {
        expect(r.label.length).toBeGreaterThan(0);
        expect(r.value.length).toBeGreaterThan(0);
      }
    }
  });
  it("null status says waiting for layers that read /status", () => {
    for (const n of [0, 1, 2, 3, 4, 6]) expect(layerStatusRows(n, null)).toEqual([{ label: "Status", value: "waiting for the node" }]);
  });
  it("L0 shows kb sha and entries, and copies the full hash", () => {
    const rows = layerStatusRows(0, status());
    expect(rows.map((r) => r.label)).toEqual(["Snapshot sha256", "Entries"]);
    expect(rows[0]!.value).toBe(`${fx.status.kb!.sha256!.slice(0, 16)}…`);
    expect(rows[0]!.copy).toBe(fx.status.kb!.sha256);
    expect(rows[1]!.value).toBe(String(fx.status.kb!.entries));
    const s = status();
    s.kb = null;
    expect(layerStatusRows(0, s)).toEqual([{ label: "Knowledge base", value: "not present on this node" }]);
  });
  it("L1 shows version, the update block, bind urls and lan", () => {
    const rows = layerStatusRows(1, status());
    expect(rows[0]).toEqual({ label: "Version", value: fx.status.node.version });
    expect(rows.map((r) => r.label)).toEqual(
      expect.arrayContaining(["Update state", "Update last check", "Update last result", "Clock fault", "Listening on", "LAN bound now"]),
    );
    expect(rows.find((r) => r.label === "Listening on")!.value).toEqual(fx.status.bind.urls);
  });
  it("L2 shows label, bech32 and node id with copy text", () => {
    const rows = layerStatusRows(2, status());
    expect(rows.map((r) => r.label)).toEqual(["Node label", "Address (bech32)", "Node id (hex)"]);
    expect(rows[1]!.copy).toBe(fx.status.node.bech32);
    expect(rows[2]!.copy).toBe(fx.status.node.node_id);
    const s = status();
    s.node = { ...s.node, bech32: null, node_id: null, label: null };
    const bare = layerStatusRows(2, s);
    expect(bare.every((r) => r.value === "not set" && r.copy === undefined)).toBe(true);
  });
  it("L3 shows the intelligence block", () => {
    const rows = layerStatusRows(3, status());
    expect(rows).toEqual([
      { label: "Agent", value: "running" },
      { label: "Model server", value: "running" },
      { label: "Agent mode", value: "local" },
      { label: "Model", value: fx.status.intelligence.model_id },
    ]);
  });
  it("L4 shows bind urls, loopback/paired and configured lan", () => {
    const s = status();
    s.auth = { loopback: false, paired: true };
    s.lan_enabled = false;
    const rows = layerStatusRows(4, s);
    expect(rows.map((r) => r.label)).toEqual(["Listening on", "Loopback session", "Paired session", "LAN sharing (configured)"]);
    expect(rows.slice(1).map((r) => r.value)).toEqual(["no", "yes", "off"]);
  });
  it("L5 and L8 say when they arrive, even without status", () => {
    expect(layerStatusRows(5, null)).toEqual([{ label: "Status", value: "arrives with Layer 5" }]);
    expect(layerStatusRows(8, status())).toEqual([{ label: "Status", value: "arrives with Layer 8" }]);
  });
  it("L6 shows transport state and the component when present", () => {
    expect(layerStatusRows(6, status())).toEqual([
      { label: "Transport state", value: "joined" },
      { label: "Transport component", value: "alive" },
    ]);
  });
  it("L7 reads the gateway service state", () => {
    const list = services();
    const gw = list.find((s) => s.id === "gateway")!;
    const rows = layerStatusRows(7, status(), { services: list });
    expect(rows[0]).toEqual({ label: "Gateway service", value: gw.state });
    expect(rows[1]).toEqual({ label: "Reason", value: gw.reason });
    expect(layerStatusRows(7, status(), { services: [] })[0]!.value).toBe("not reported by this node");
    expect(layerStatusRows(7, status(), { services: null })[0]!.value).toBe("loading");
    expect(labels(7, { services: list })).toEqual(["Gateway service", "Reason"]);
  });
  it("L9 shows the open proposal count", () => {
    expect(layerStatusRows(9, status(), { openProposals: 3 })).toEqual([{ label: "Open proposals", value: "3" }]);
    expect(layerStatusRows(9, status(), { openProposals: 0 })[0]!.value).toBe("0");
    expect(layerStatusRows(9, null, { openProposals: null })[0]!.value).toBe("loading");
  });
  it("an unknown layer number is reported, not thrown", () => {
    expect(layerStatusRows(42, status())).toEqual([{ label: "Status", value: "no such layer" }]);
  });
});

describe("layer catalogue and ask prompt", () => {
  it("has ten layers whose slugs resolve and whose copy is filled in", () => {
    expect(LAYERS).toHaveLength(10);
    LAYERS.forEach((l, i) => {
      expect(l.n).toBe(i);
      expect(l.slug).toBe(`l${i}`);
      expect(layerBySlug(l.slug)).toBe(l);
      expect(l.title.length).toBeGreaterThan(2);
      expect(l.description.length).toBeGreaterThan(40);
    });
    expect(layerBySlug("l10")).toBeUndefined();
    expect(layerBySlug(undefined)).toBeUndefined();
  });
  it("prefixes the node check prompt with the layer question", () => {
    const p = layerAskPrompt(3, "Intelligence", status());
    expect(p.text.startsWith("About Layer 3 (Intelligence): what is its state on this node and what should I configure?\n\n")).toBe(true);
    expect(p.text).toContain("Check this node end to end");
    expect(p.text).toContain("Workshop laptop");
    expect(p.from).toBe("Intelligence");
    expect(p.goal).toBe("check");
  });
  it("builds a prompt even without status", () => {
    expect(() => layerAskPrompt(0, "Knowledge", null)).not.toThrow();
    expect(layerAskPrompt(0, "Knowledge", null).text).toContain("node unknown");
  });
});

describe("logs model", () => {
  it("tabs are the services that offer the log action, in registry order", () => {
    const list = services();
    const tabs = logServices(list);
    expect(tabs.length).toBeGreaterThan(0);
    expect(tabs.every((s) => s.actions.includes("log"))).toBe(true);
    expect(tabs.map((s) => s.id)).toEqual(list.filter((s) => s.actions.includes("log")).map((s) => s.id));
    expect(list.some((s) => !s.actions.includes("log"))).toBe(true);
    expect(logServices(null)).toEqual([]);
    expect(logServices(undefined)).toEqual([]);
  });
  it("strips control and bidi characters from untrusted lines", () => {
    expect(cleanLines(["ok\u0007line", "a‮b", "tab\tkept", "cr\r"])).toEqual(["okline", "ab", "tab\tkept", "cr"]);
    expect(cleanLines(null)).toEqual([]);
  });
  it("describes a tail honestly", () => {
    const tail = (lines: string[], source: string): LogTail => ({ schema: 1, service: "agent", source, lines });
    expect(tailSummary(null)).toBe("no tail loaded yet");
    expect(tailSummary(tail(["a"], "/var/lib/111/log/x.log"))).toBe("1 line · source /var/lib/111/log/x.log");
    expect(tailSummary(tail(["a", "b"], "none"))).toBe("2 lines · source none");
    expect(hasNoSource(tail([], "none"))).toBe(true);
    expect(hasNoSource(tail([], "/x.log"))).toBe(false);
    expect(hasNoSource(null)).toBe(false);
    expect(emptyLogText(null, true)).toBe("Loading the log…");
    expect(emptyLogText(null, false)).toBe("No log could be loaded.");
    expect(emptyLogText(tail([], "/x.log"), false)).toBe("The log file is empty.");
  });
  it("follow text states the poll period", () => {
    expect(LOG_POLL_MS).toBe(5000);
    expect(followText(true)).toBe("following, refreshed every 5 s");
    expect(followText(false)).toBe("not following");
  });
});
