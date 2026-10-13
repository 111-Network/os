import { describe, expect, it } from "vitest";
import type { DeviceRecord, ServiceRecord, Status } from "../src/lib/api/sse.gen";
import {
  FIRMWARE_UNKNOWN,
  KIND_ORDER,
  agoText,
  aliasFromDraft,
  confidenceLabel,
  deviceAgentAction,
  deviceDot,
  devicePrompt,
  displayName,
  drivenBy,
  alsoLabel,
  durationText,
  firmwareText,
  forgetConfirm,
  groupByKind,
  identityRows,
  isLoopbackUrl,
  isMdnsUrl,
  isNumericIpv4Url,
  kindTransport,
  kindWord,
  pairingHelp,
  pickPrimaryUrl,
  presenceText,
  registryPrompt,
  showsFirmware,
  stateLabel,
  summary,
  transportWord,
} from "../src/lib/devices";
import { nearbySentence, radarArt } from "../src/views/nearby/radar";

function dev(over: Partial<DeviceRecord> = {}): DeviceRecord {
  return {
    id: "usb-1",
    since: "2026-10-13T10:00:00Z",
    kind: "radio",
    alias: null,
    name: "Heltec WiFi LoRa 32 V3",
    state: "connected",
    transport: "usb-serial",
    first_seen: "2026-10-13T09:00:00Z",
    last_seen: "2026-10-13T10:00:00Z",
    reason_code: "present",
    reason: "The board is plugged in.",
    capabilities: ["lora"],
    identity: {},
    actions: ["rename", "forget", "agent"],
    detail: { driver: "cdc_acm" },
    ...over,
  };
}

const status = {
  node: { label: "k7m2-9x4p-aa", alias: null, category: "Node2B", version: "2026.10.13" },
  layers: [0, 1, 2, 3],
  transport_state: "isolated",
  lan_enabled: true,
  bind: { urls: ["http://111.localhost", "http://111.local", "http://192.168.1.42", "http://127.0.0.1:11180"], lan: true },
} as unknown as Status;

const radio = { id: "radio", name: "radio", plain_name: "LoRa radio", state: "unset", reason_code: "radio_unbound", reason: "nothing bound", can_run_here: true, kb_entry: "projects/meshtastic.md", detail: {} } as unknown as ServiceRecord;

describe("deviceDot", () => {
  it("maps each state to a dot", () => {
    expect(deviceDot("connected", false)).toBe("live");
    expect(deviceDot("needs_setup", false)).toBe("unset");
    expect(deviceDot("error", false)).toBe("error");
    expect(deviceDot("disconnected", false)).toBe("unknown");
  });
  it("goes hollow for every state while the node link is down", () => {
    for (const s of ["connected", "needs_setup", "error", "disconnected"] as const) expect(deviceDot(s, true)).toBe("unknown");
  });
  it("always has a text label", () => {
    expect(stateLabel("connected")).toBe("connected");
    expect(stateLabel("needs_setup")).toBe("needs setup");
    expect(stateLabel("error")).toBe("error");
    expect(stateLabel("disconnected")).toBe("disconnected");
    expect(stateLabel("connected", true)).toBe("unknown");
  });
});

describe("names", () => {
  it("prefers the alias, ignores a blank one", () => {
    expect(displayName({ alias: "roof board", name: "Heltec" })).toBe("roof board");
    expect(displayName({ alias: null, name: "Heltec" })).toBe("Heltec");
    expect(displayName({ alias: "  ", name: "Heltec" })).toBe("Heltec");
  });
  it("empty text or the detected name clears the alias", () => {
    expect(aliasFromDraft("  roof board ", { name: "Heltec" })).toBe("roof board");
    expect(aliasFromDraft("", { name: "Heltec" })).toBeNull();
    expect(aliasFromDraft("   ", { name: "Heltec" })).toBeNull();
    expect(aliasFromDraft("Heltec", { name: "Heltec" })).toBeNull();
  });
});

describe("groupByKind", () => {
  const all = [
    dev({ id: "u", kind: "unknown", name: "USB serial device" }),
    dev({ id: "c", kind: "companion", name: "Android phone" }),
    dev({ id: "r1", kind: "radio", state: "disconnected" }),
    dev({ id: "b", kind: "bluetooth", name: "Bluetooth adapter" }),
    dev({ id: "r2", kind: "radio" }),
    dev({ id: "a", kind: "analog", name: "Baofeng" }),
    dev({ id: "n", kind: "node", name: "Other node" }),
  ];
  it("uses the fixed order and plain titles", () => {
    const g = groupByKind(all);
    expect(g.map((x) => x.kind)).toEqual([...KIND_ORDER]);
    expect(g.map((x) => x.title)).toEqual(["LoRa boards", "Bluetooth", "Your phones & laptops", "Your other nodes", "Handheld radios", "Unidentified"]);
  });
  it("omits empty groups", () => {
    expect(groupByKind([dev({ kind: "companion" })]).map((x) => x.title)).toEqual(["Your phones & laptops"]);
    expect(groupByKind([])).toEqual([]);
  });
  it("keeps registry order but sinks disconnected devices", () => {
    const radios = groupByKind(all)[0]!.devices.map((d) => d.id);
    expect(radios).toEqual(["r2", "r1"]);
  });
  it("files a kind it does not know under Unidentified", () => {
    const g = groupByKind([dev({ id: "x", kind: "quantum" as unknown as DeviceRecord["kind"] })]);
    expect(g).toHaveLength(1);
    expect(g[0]!.kind).toBe("unknown");
    expect(g[0]!.title).toBe("Unidentified");
  });
  it("has plain words for kind and transport, with a safe fallback", () => {
    expect(kindWord("radio")).toBe("LoRa board");
    expect(kindWord("hologram")).toBe("Unidentified");
    expect(transportWord("usb-serial")).toBe("USB serial");
    expect(transportWord("lan")).toBe("Local network");
    expect(transportWord("telepathy")).toBe("unknown");
    expect(kindTransport(dev({ kind: "radio", transport: "usb-serial" }))).toBe("LoRa board · USB serial");
    expect(kindTransport(dev({ kind: "bluetooth", transport: "bluetooth" }))).toBe("Bluetooth");
  });
});

describe("drivenBy and confidenceLabel", () => {
  const driver = (stack: NonNullable<DeviceRecord["driver"]>["stack"]) => ({ stack, interface: null, live: true });
  it("names the stack in plain words", () => {
    expect(drivenBy(dev({ driver: driver("reticulum") }))).toBe("Mesh transport (RNode)");
    expect(drivenBy(dev({ driver: driver("meshtastic") }))).toBe("Meshtastic");
    expect(drivenBy(dev({ driver: driver("meshcore") }))).toBe("MeshCore");
    expect(drivenBy(dev({ kind: "companion", driver: driver("dashboard") }))).toBe("Dashboard session");
  });
  it("says nothing yet when no stack is bound", () => {
    expect(drivenBy(dev({ driver: driver(null) }))).toBe("nothing yet");
    expect(drivenBy(dev({ driver: null }))).toBe("nothing yet");
    expect(drivenBy(dev())).toBe("nothing yet");
  });
  it("treats a paired dashboard session without a driver as driven by the session", () => {
    expect(drivenBy(dev({ kind: "companion", capabilities: ["dashboard-session"] }))).toBe("Dashboard session");
    expect(drivenBy(dev({ kind: "companion", capabilities: ["dashboard-session"], driver: driver(null) }))).toBe("nothing yet");
  });
  it("confidence: exact is identified, likely stays likely, anything else is unknown", () => {
    expect(confidenceLabel(dev({ match: { confidence: "exact", rule: "probe" } }))).toBe("identified");
    expect(confidenceLabel(dev({ match: { confidence: "likely", rule: "lora-table 303a:1001" } }))).toBe("likely");
    expect(confidenceLabel(dev({ match: { confidence: "unknown", rule: "serial-by-id" } }))).toBe("unknown");
    expect(confidenceLabel(dev())).toBe("unknown");
  });
});

describe("firmware and identity", () => {
  it("is honest about unknown firmware", () => {
    expect(firmwareText(dev())).toBe(FIRMWARE_UNKNOWN);
    expect(firmwareText(dev({ firmware: { family: "unknown", version: null, source: "unknown" } }))).toBe(FIRMWARE_UNKNOWN);
    expect(firmwareText(dev({ firmware: { family: "meshtastic", version: "2.5.9", source: "probe" } }))).toBe("Meshtastic 2.5.9 (reported by a probe)");
    expect(firmwareText(dev({ firmware: { family: "none", version: null, source: "user" } }))).toBe("no firmware installed (reported by you)");
  });
  it("shows the firmware row for boards and unidentified hardware only", () => {
    expect(showsFirmware(dev({ kind: "radio" }))).toBe(true);
    expect(showsFirmware(dev({ kind: "unknown" }))).toBe(true);
    expect(showsFirmware(dev({ kind: "companion" }))).toBe(false);
    expect(showsFirmware(dev({ kind: "companion", firmware: { family: "rnode", version: null, source: "agent" } }))).toBe(true);
  });
  it("lists identity in a fixed order, cleans control characters, copies where it helps", () => {
    const rows = identityRows(
      dev({ identity: { user_agent: "UA\u0007‮x", port: "/dev/ttyUSB0", product_id: "1001", vendor_id: "303a", extra_thing: 7, empty: "", nothing: null } }),
    );
    expect(rows.map((r) => r.key)).toEqual(["vendor_id", "product_id", "port", "user_agent", "extra_thing"]);
    expect(rows.find((r) => r.key === "user_agent")!.value).toBe("UAx");
    expect(rows.find((r) => r.key === "port")!.copy).toBe(true);
    expect(rows.find((r) => r.key === "vendor_id")!.copy).toBe(false);
    expect(rows.find((r) => r.key === "extra_thing")!.label).toBe("extra thing");
  });
});

describe("time", () => {
  const t0 = Date.parse("2026-10-13T10:00:00Z");
  it("humanizes how long ago", () => {
    expect(agoText("2026-10-13T10:00:00Z", t0 + 3_000)).toBe("just now");
    expect(agoText("2026-10-13T10:00:00Z", t0 + 42_000)).toBe("42s ago");
    expect(agoText("2026-10-13T10:00:00Z", t0 + 5 * 60_000)).toBe("5m ago");
    expect(agoText("2026-10-13T10:00:00Z", t0 + 3 * 3600_000)).toBe("3h ago");
    expect(agoText("2026-10-13T10:00:00Z", t0 + 2 * 86_400_000)).toBe("2d ago");
    expect(agoText("nonsense", t0)).toBe("unknown");
    expect(agoText(undefined, t0)).toBe("unknown");
    expect(agoText("2026-10-13T10:00:00Z", t0 - 5_000)).toBe("just now");
  });
  it("describes presence", () => {
    expect(durationText("2026-10-13T10:00:00Z", t0 + 4 * 60_000)).toBe("4m");
    expect(presenceText(dev({ state: "needs_setup" }), t0 + 12_000)).toBe("for 12s");
    expect(presenceText(dev({ state: "connected" }), t0 + 4 * 60_000)).toBe("for 4m");
    expect(presenceText(dev({ state: "disconnected" }), t0 + 3 * 3600_000)).toBe("last seen 3h ago");
    expect(presenceText(dev({ state: "connected" }), t0 + 60_000, true)).toBe("last known state, last seen 1m ago");
  });
});

describe("summary", () => {
  it("counts every state", () => {
    const s = summary([dev({ state: "connected" }), dev({ state: "connected" }), dev({ state: "needs_setup" }), dev({ state: "error" }), dev({ state: "disconnected" })]);
    expect(s).toMatchObject({ total: 5, connected: 2, needsSetup: 1, error: 1, disconnected: 1 });
    expect(s.sentence).toBe("5 devices: 2 connected, 1 needs setup, 1 has an error, 1 disconnected.");
  });
  it("reads well at the edges", () => {
    expect(summary([]).sentence).toBe("Nothing connected yet.");
    expect(summary([dev()]).sentence).toBe("1 device, connected.");
    expect(summary([dev(), dev()]).sentence).toBe("2 devices, all connected.");
    expect(summary([dev({ state: "needs_setup" }), dev({ state: "needs_setup" })]).sentence).toBe("2 devices: 2 need setup.");
    expect(summary([dev({ state: "error" }), dev({ state: "error" })]).sentence).toBe("2 devices: 2 have errors.");
  });
});

describe("prompts", () => {
  it("the registry prompt carries the whole registry and, when asked, raw facts and a hint", () => {
    const text = registryPrompt({ devices: [dev({ detail: { marker_fact: 1 } })], services: [radio], status, facts: true, hint: "I am connecting a LoRa board over USB" });
    expect(text).toContain("Look at everything connected or paired to this node");
    expect(text).toContain("Hint from the user: I am connecting a LoRa board over USB");
    expect(text).toContain("marker_fact");
    const bare = registryPrompt({ devices: [dev({ detail: { marker_fact: 1 } })], services: [radio], status });
    expect(bare).not.toContain("raw facts");
  });
  it("picks the goal from the device state", () => {
    expect(deviceAgentAction(dev({ state: "error" }))).toEqual({ goal: "fix", label: "Let the agent handle it" });
    expect(deviceAgentAction(dev({ state: "needs_setup" }))).toEqual({ goal: "setup", label: "Let the agent handle it" });
    expect(deviceAgentAction(dev({ state: "connected" })).goal).toBe("check");
    expect(deviceAgentAction(dev({ state: "connected" })).label).toBe("Ask the agent about this device");
    expect(deviceAgentAction(dev({ state: "disconnected" })).goal).toBe("check");
  });
  it("a needs-setup device prompt sets up the device and includes its service", () => {
    const { text, goal } = devicePrompt({ device: dev({ state: "needs_setup", service_id: "radio", kb_entry: "devices/heltec.md" }), services: [radio], status });
    expect(goal).toBe("setup");
    expect(text.startsWith("Set up the Heltec WiFi LoRa 32 V3 on this node and test it.")).toBe(true);
    expect(text).toContain('"plain_name":"LoRa radio"');
    expect(text).toContain("KB entries: devices/heltec.md");
    expect(text).toContain("- raw facts:");
  });
  it("an error device prompt asks for a fix under the alias", () => {
    const { text, goal } = devicePrompt({ device: dev({ state: "error", alias: "roof board" }), services: [], status });
    expect(goal).toBe("fix");
    expect(text.startsWith("Fix the roof board on this node.")).toBe(true);
  });
  it("a healthy device prompt asks what it is, with the name cleaned", () => {
    const { text, goal } = devicePrompt({ device: dev({ name: "Evil\u0007‮ name" }), services: [], status });
    expect(goal).toBe("check");
    expect(text.startsWith("About the device Evil name: what is it, is it working, what can it do?\n\n")).toBe(true);
    expect(text).toContain("Check this node end to end");
  });
});

describe("forgetConfirm", () => {
  it("warns a companion about being logged out", () => {
    const t = forgetConfirm(dev({ kind: "companion", name: "Android phone" }));
    expect(t).toContain("Forget Android phone?");
    expect(t).toContain("If this is the phone you are using right now, you will be logged out");
  });
  it("says hardware comes back when plugged in again", () => {
    expect(forgetConfirm(dev({ kind: "radio", alias: "roof board" }))).toContain("Forget roof board?");
    expect(forgetConfirm(dev({ kind: "radio" }))).toContain("shows up again if it is still plugged in");
  });
});

describe("pairingHelp (Add device, Control this node)", () => {
  const base = { lan_enabled: true, bind: { urls: ["http://111.local", "http://192.168.1.42", "http://127.0.0.1:11180"], lan: true } };
  it("waits for the node", () => {
    expect(pairingHelp(null).state).toBe("unknown");
    expect(pairingHelp(null).urls).toEqual([]);
  });
  it("LAN off: says so, links to Settings, shows no addresses and no pair address", () => {
    const h = pairingHelp({ lan_enabled: false, bind: { urls: ["http://111.localhost", "http://127.0.0.1:11180"], lan: false } });
    expect(h.state).toBe("off");
    expect(h.body).toBe("LAN sharing is off — turn it on in Settings → Sharing (from the node itself).");
    expect(h.settingsHref).toBe("/settings");
    expect(h.urls).toEqual([]);
    expect(h.pairUrl).toBeNull();
    expect(h.command).toBeNull();
  });
  it("LAN switched on but not bound yet: needs a restart", () => {
    const h = pairingHelp({ lan_enabled: true, bind: { urls: ["http://127.0.0.1:11180"], lan: false } });
    expect(h.state).toBe("pending");
    expect(h.body).toContain("restarts");
    expect(h.pairUrl).toBeNull();
  });
  it("LAN on: the LAN IP is primary (Android cannot resolve .local); .local follows as an extra", () => {
    const h = pairingHelp(base);
    expect(h.state).toBe("on");
    expect(h.primaryUrl).toBe("http://192.168.1.42");
    expect(h.alsoUrls).toEqual(["http://111.local"]);
    expect(h.urls).toEqual(["http://192.168.1.42", "http://111.local"]);
    expect(h.pairUrl).toBe("http://192.168.1.42/#/pair");
    expect(h.command).toBe("111 dashboard");
    expect(h.settingsHref).toBeNull();
  });
  it("LAN on with only a .local name: falls back to it as the primary address", () => {
    const h = pairingHelp({ lan_enabled: true, bind: { urls: ["http://111.local", "http://127.0.0.1:11180"], lan: true } });
    expect(h.primaryUrl).toBe("http://111.local");
    expect(h.alsoUrls).toEqual([]);
    expect(h.pairUrl).toBe("http://111.local/#/pair");
  });
  it("picks the first numeric IPv4 when several are bound", () => {
    expect(pickPrimaryUrl(["http://111.local", "http://10.0.0.7", "http://192.168.1.42"])).toBe("http://10.0.0.7");
    expect(pickPrimaryUrl(["http://127.0.0.1:11180", "http://111.localhost"])).toBeNull();
    expect(pickPrimaryUrl([])).toBeNull();
    // a port stays on the address; loopback IPv4 is never primary; an out-of-range quad is not an IP
    expect(pickPrimaryUrl(["http://127.0.0.1:11180", "http://192.168.1.42:8080"])).toBe("http://192.168.1.42:8080");
    expect(isNumericIpv4Url("http://999.1.1.1")).toBe(false);
    expect(isNumericIpv4Url("http://192.168.1.42")).toBe(true);
    expect(isNumericIpv4Url("http://111.local")).toBe(false);
    expect(isNumericIpv4Url("not a url")).toBe(false);
  });
  it("labels the extra addresses, flagging .local as not working on Android", () => {
    expect(alsoLabel("http://111.local")).toBe("also: http://111.local (not on Android)");
    expect(alsoLabel("http://node.lan")).toBe("also: http://node.lan");
    expect(isMdnsUrl("http://111.local")).toBe(true);
    expect(isMdnsUrl("http://111.localhost")).toBe(false);
  });
  it("LAN on with only loopback addresses says no address was found", () => {
    const h = pairingHelp({ lan_enabled: true, bind: { urls: ["http://127.0.0.1:11180", "http://111.localhost"], lan: true } });
    expect(h.state).toBe("on");
    expect(h.urls).toEqual([]);
    expect(h.pairUrl).toBeNull();
    expect(h.body).toContain("No network address");
  });
  it("tells loopback addresses apart", () => {
    expect(isLoopbackUrl("http://127.0.0.1:11180")).toBe(true);
    expect(isLoopbackUrl("http://localhost:11180")).toBe(true);
    expect(isLoopbackUrl("http://111.localhost")).toBe(true);
    expect(isLoopbackUrl("http://[::1]:11180")).toBe(true);
    expect(isLoopbackUrl("http://111.local")).toBe(false);
    expect(isLoopbackUrl("http://192.168.1.42:11180")).toBe(false);
    expect(isLoopbackUrl("not a url")).toBe(false);
  });
});

describe("Nearby", () => {
  it("has one sentence per transport state", () => {
    expect(nearbySentence("joined")).toBe("At least one nearby node is heard on the mesh.");
    expect(nearbySentence("isolated")).toBe("No nearby nodes heard right now.");
    expect(nearbySentence("no transport")).toBe("No mesh transport yet.");
    expect(nearbySentence("transport degraded")).toBe("The mesh transport has a problem.");
    expect(nearbySentence(null)).toBe("Waiting for the node to report what it hears.");
  });
  it("the radar has rings and this node at the centre, and no peers", () => {
    const art = radarArt();
    const rows = art.split("\n");
    expect(rows).toHaveLength(17);
    expect(rows[8]!.indexOf("@")).toBe(16);
    expect(art.match(/@/g)).toHaveLength(1);
    expect(art.replace(/[@·\n ]/g, "")).toBe("");
    expect(art.match(/·/g)!.length).toBeGreaterThan(60);
    // symmetric top to bottom, so the rings read as circles around the node
    expect(rows.slice().reverse().join("\n")).toBe(art);
    expect(radarArt()).toBe(art);
  });
});
