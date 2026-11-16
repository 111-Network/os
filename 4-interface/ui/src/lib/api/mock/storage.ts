// Layer 8 mock (os/8-storage/README.md §4, §13.7, §15, §16): the storage status, the settings with the two serving consent gates
// (`datasets_serve`, `components_serve`), the export offers, and the three store-state writes (an update without a rollback copy, a
// set-aside file, an empty store). The routes are synchronous and throw ApiError; the adapter (index.ts) wraps them in its latency, auth
// gate and structuredClone, and republishes the `storage` frames this file hands to `publish`. The dataset routes are the contract's
// second part. Nothing here is imported by production code.
import { ApiError } from "../problem";
import type {
  ConsentRequired,
  EventsFrame,
  Problem,
  ProblemCode,
  StorageClass,
  StorageExportOfferRequest,
  StorageSettings,
  StorageSettingsPatch,
  StorageStatus,
  StorageStore,
} from "../types";
import { buildStorageStatus } from "./fixtures";
import { labelOf } from "./messaging";
import { sha256Hex } from "./sha256";

const MIB = 1024 * 1024;
const HEX64 = /^[0-9a-f]{64}$/;
/** The classes whose figures a frame carries (§4.1): the four budgeted ones. */
const BUDGETED = ["fabric", "agent", "heal", "logs"] as const;

// --- The consent texts, verbatim from the spec (§15.2) -------------------------------------------------------------------------------

export const datasetsServeText = (): string =>
  "Any device on this network can download this hub's dataset files without signing in: which encyclopedias and maps it holds, their versions and their contents. A map file shows the area it covers, which tells visitors where this community is. Every device that holds this hub's card also learns that it serves datasets. Requests are not logged. When you share these files, their licences ask you to keep their notices; this hub shows them on its Credits page.";
export const componentsServeText = (termux: boolean, list = "kiwix-serve", termuxSize = "180 MB"): string =>
  `Any device on this network can download the program files this hub cached for itself (${list})${termux ? `, and the Termux install files (${termuxSize}), which this hub downloads first,` : ""} without signing in. When you share a program file, you are the one who passes it on under its licence: this hub serves each file with its licence notice and where its source is, which is what those licences ask of you. 111 and its author do not distribute these files.`;
export const updateWithoutCopyText = (release: string, store: StorageStore): string => {
  const what = store === "fabric" ? "messages" : "the agent's history and repair records";
  return `Install ${release} without a rollback copy of ${what}. This node cannot keep a copy while it updates: there is not enough free space, or the copy would not finish in time. If this release fails, ${what} stay closed until the next update.`;
};
export const startEmptyText = (store: StorageStore, file: string): string => {
  const what = store === "fabric" ? "messages" : "the agent's history and repair records";
  return `Start an empty store for ${what}. The damaged file stays on this node as ${file} and is not read again; only you can delete it. What it held comes back only from an export.`;
};

export interface StorageMockDeps {
  publish(frame: EventsFrame): number;
  /** `loopback_only` routes refuse unless this says the caller is on the node itself. */
  loopback(): boolean;
  /** This node has the hub role (`needs_hub` otherwise). Read on each call: a scenario may turn the role on. */
  hub?(): boolean;
  /** LAN sharing is on (`lan_off` otherwise). Read on each call. */
  lan?(): boolean;
  /** The hub's NodeId, for a consent subject (short form). */
  nodeId?(): string | undefined;
}

export interface StorageMockOptions {
  /** Pins for the Termux install files carry a verified licence in this release (default false: the tick is not offered, §11.5). */
  termuxPins?: boolean;
}

/** The synchronous routes; index.ts wraps each in its latency, auth gate and clone. */
export interface StorageRoutes {
  status(): StorageStatus;
  patchSettings(patch: StorageSettingsPatch, acceptWarning?: string): StorageSettings;
  answerExportOffer(cls: StorageExportOfferRequest["class"]): void;
  acceptUpdateWithoutCopy(release: string, store: StorageStore, acceptWarning?: string): void;
  deleteSetAside(file: string): void;
  startEmpty(store: StorageStore, acceptWarning?: string): void;
  /** Mock controls for tests and scenario scripts. */
  readonly control: {
    setDiskLow(on: boolean): void;
    /** The class needs an export offer before it may reclaim and, with `full`, stops growing (§4.5). */
    setExportOfferNeeded(cls: "fabric" | "agent" | "heal", needed: boolean, full?: boolean): void;
    setNodeState(state: "open" | "closed" | "unavailable", reason?: string | null): void;
    setIntegrity(store: StorageStore, state: "checking" | "ok" | "failed" | null): void;
    /** A store was damaged and set aside: its `.damaged-<n>` file is listed and the store is closed (the next open creates it empty only after `start-empty`). */
    damageStore(store: StorageStore, file: string, bytes: number): void;
    /** An update was refused for want of room for a rollback copy (§13.6 (5)). */
    refuseUpdateForSpace(release: string, store: StorageStore, neededBytes: number): void;
    setUnreadableAgentFiles(count: number): void;
  };
}

function problem(status: number, code: ProblemCode, title: string, detail?: string, consent?: Problem["consent"]): ApiError {
  const p: Problem = { type: "about:blank", title, status, code };
  if (detail) p.detail = detail;
  if (consent) p.consent = consent;
  return new ApiError(p);
}

/** A file a `set_aside_store` notice may name (§13.2, §13.6, §13.7): a name, never a path. */
export const SET_ASIDE_NAME = /^[A-Za-z0-9._-]{1,128}\.(rolled-back-v\d{1,10}|damaged-\d{1,10}|replaced-\d{1,10})(-wal|-shm)?$/;

export function createStorageMock(deps: StorageMockDeps, opts: StorageMockOptions = {}): StorageRoutes {
  const s: StorageStatus = buildStorageStatus();
  const settings: StorageSettings = {
    keep_bytes: null,
    keep_default: s.keep.default,
    caps: { agent: null, heal: null },
    logs: { max_file_bytes: 4 * MIB },
    follow: [],
    sources: {},
    origin: { datasets: false, components: false, termux: false, max_streams: 4 },
  };
  const termuxPins = opts.termuxPins ?? false;
  const recorded = new Map<string, string>(); // consent action -> the hash on record
  const closed = new Map<StorageStore, string>(); // a store closed after a set-aside -> its damaged file
  const wantsCopy = new Set<string>(); // `<release>:<store>` consents on record
  const isHub = () => deps.hub?.() ?? true;
  const isLan = () => deps.lan?.() ?? true;

  const cls = (id: string): StorageClass => s.classes.find((c) => c.id === id)!;

  /** §4.2 item 3 at the budget in force; `caps.agent` and `caps.heal` override a share. */
  function recompute(): void {
    const keep = settings.keep_bytes ?? settings.keep_default;
    const logs = 13 * MIB;
    const rest = Math.max(0, keep - logs);
    const fabricShare = Math.floor((rest * 3) / 4);
    const ns = rest - fabricShare;
    const heal = settings.caps.heal ?? Math.floor(ns / 4);
    const agent = settings.caps.agent ?? Math.max(0, ns - heal);
    // The built 512 MiB watermark is a floor until a human sets the budget (§4.2 item 3): the budget only raises it.
    const fabric = settings.keep_bytes === null ? Math.max(fabricShare, 512 * MIB) : fabricShare;
    cls("fabric").cap = fabric;
    cls("agent").cap = agent;
    cls("heal").cap = heal;
    cls("logs").cap = logs;
    s.keep = { bytes: keep, default: settings.keep_default, effective: fabric + agent + heal + logs, rule: settings.keep_bytes === null ? (isHub() ? "hub" : "linux") : "user" };
  }
  recompute();

  const snapshot = (): StorageStatus => ({ ...structuredClone(s), settings: structuredClone(settings) });
  /** The frame: what the pass already holds (§16.3), the four budgeted classes, no settings. */
  const published = (): void => {
    const frame = structuredClone(s);
    frame.classes = frame.classes.filter((c) => (BUDGETED as readonly string[]).includes(c.id));
    deps.publish({ event: "storage", data: frame });
  };
  const loopbackOnly = (): void => {
    if (!deps.loopback()) throw problem(403, "loopback_only", "Available on the node itself", "Change this from the machine the node runs on.");
  };
  const storeUnavailable = (): ApiError => problem(503, "store_unavailable", "The agent's store is not available", "Its history and repair records cannot be reached right now.");
  const consent = (action: "datasets_serve" | "components_serve" | "update_without_copy" | "store_start_empty", subject: string, text: string, addons: string[] = [], facts?: ConsentRequired["facts"]): ApiError =>
    problem(409, "consent_required", "Confirm first", "This needs your confirmation.", { action, subject, warning_text: text, hash: sha256Hex(text), addons, ...(facts ? { facts } : {}) });
  const hubShort = (): string => labelOf(deps.nodeId?.() ?? "9f2c41d7a0b35e68c1d4f7092ab36e5d8c70f1e24b9a6d3057c8e1f2a4b6d9c3");

  /** The consent owed is on record (same text) or the repeat carries its hash. */
  const owed = (key: string, text: string, accept: string | undefined): boolean => sha256Hex(text) !== recorded.get(key) && accept !== sha256Hex(text);

  function validate(patch: StorageSettingsPatch): void {
    const bad = (pointer: string, message: string): ApiError => new ApiError({ type: "about:blank", title: "Invalid setting", status: 400, code: "bad_request", errors: [{ pointer, message }] });
    const size = (v: unknown, pointer: string, min = 0): void => {
      if (v === null) return;
      if (typeof v !== "number" || !Number.isSafeInteger(v) || v < min) throw bad(pointer, `must be a whole number of bytes, at least ${min}`);
    };
    if ("keep_bytes" in patch) size(patch.keep_bytes, "/keep_bytes");
    if (patch.caps) {
      if ("agent" in patch.caps) size(patch.caps.agent, "/caps/agent");
      if ("heal" in patch.caps) size(patch.caps.heal, "/caps/heal");
    }
    if (patch.logs && "max_file_bytes" in patch.logs) size(patch.logs.max_file_bytes, "/logs/max_file_bytes", 1);
    if (patch.follow && (!Array.isArray(patch.follow) || patch.follow.some((f) => typeof f !== "string" || !HEX64.test(f)))) throw bad("/follow", "must be a list of 64-character lowercase hex NodeIds");
    if (patch.sources) {
      for (const [k, v] of Object.entries(patch.sources)) {
        if (!HEX64.test(k) || !Array.isArray(v) || v.some((u) => typeof u !== "string" || u.length === 0 || u.length > 512)) throw bad("/sources", "must map a 64-character hex NodeId to a list of origin URLs");
      }
    }
    const o = patch.origin;
    if (o?.max_streams !== undefined && (!Number.isInteger(o.max_streams) || o.max_streams < 1 || o.max_streams > 16)) throw bad("/origin/max_streams", "must be between 1 and 16");
    if (o?.termux === true && !termuxPins) throw bad("/origin/termux", "this release has no verified licence for the Termux install files, so the tick is not offered");
  }

  return {
    status: snapshot,

    patchSettings(patch, acceptWarning) {
      loopbackOnly();
      validate(patch);
      const accept = acceptWarning ?? patch.accept_warning;
      const o = patch.origin;
      const turnsOn = (k: "datasets" | "components"): boolean => o?.[k] === true && !settings.origin[k];
      if (turnsOn("datasets") || turnsOn("components") || (o?.termux === true && !settings.origin.termux)) {
        if (!isHub()) throw problem(409, "needs_hub", "This needs a hub", "Turn the hub role on first (Settings, Messaging).");
        if (!isLan()) throw problem(409, "lan_off", "LAN sharing is off", "Serving files needs LAN sharing.");
      }
      if (o?.termux === true && !(o.components ?? settings.origin.components)) throw problem(400, "bad_request", "Turn program files on first", "The Termux install files are a tick under serving program files.");
      const subject = hubShort();
      if (turnsOn("datasets") && owed("datasets_serve", datasetsServeText(), accept)) throw consent("datasets_serve", subject, datasetsServeText());
      const termuxOn = o?.termux ?? settings.origin.termux;
      const compNow = o?.components ?? settings.origin.components;
      if ((turnsOn("components") || (o?.termux === true && !settings.origin.termux)) && owed("components_serve", componentsServeText(termuxOn), accept)) {
        throw consent("components_serve", subject, componentsServeText(termuxOn), termuxOn ? ["termux"] : []);
      }

      // validated; apply
      if ("keep_bytes" in patch) settings.keep_bytes = patch.keep_bytes ?? null;
      if (patch.caps) {
        if ("agent" in patch.caps) settings.caps.agent = patch.caps.agent ?? null;
        if ("heal" in patch.caps) settings.caps.heal = patch.caps.heal ?? null;
      }
      if (patch.logs?.max_file_bytes !== undefined) settings.logs.max_file_bytes = patch.logs.max_file_bytes;
      if (patch.follow) settings.follow = [...patch.follow];
      if (patch.sources) settings.sources = structuredClone(patch.sources);
      if (o) {
        if (o.datasets !== undefined) {
          settings.origin.datasets = o.datasets;
          if (o.datasets) recorded.set("datasets_serve", sha256Hex(datasetsServeText()));
        }
        if (o.components !== undefined) settings.origin.components = o.components;
        if (o.termux !== undefined) settings.origin.termux = o.termux && compNow;
        if (settings.origin.components) recorded.set("components_serve", sha256Hex(componentsServeText(settings.origin.termux)));
        if (!settings.origin.components) settings.origin.termux = false;
        if (o.max_streams !== undefined) settings.origin.max_streams = o.max_streams;
      }
      if ("keep_bytes" in patch || patch.caps) {
        recompute();
        published();
      }
      return structuredClone(settings);
    },

    answerExportOffer(which) {
      if (which !== "fabric" && which !== "agent" && which !== "heal") throw problem(400, "bad_request", "Not a class with an export offer", "Use fabric, agent or heal.");
      if (which !== "fabric" && s.node_state.state === "unavailable") throw storeUnavailable();
      const c = cls(which);
      if (!c.export_offer_needed && !c.full) return;
      c.export_offer_needed = false;
      c.full = false;
      published();
    },

    acceptUpdateWithoutCopy(release, store, acceptWarning) {
      const n = s.notices.find((x) => x.kind === "update_refused_space" && x.release === release && x.store === store);
      if (!n) throw problem(400, "bad_request", "No such refused update", "No update was refused for lack of room for that release and store.");
      const text = updateWithoutCopyText(release, store);
      const key = `${release}:${store}`;
      if (!wantsCopy.has(key) && acceptWarning !== sha256Hex(text)) {
        throw consent("update_without_copy", key, text, [], { needed: n.needed_bytes ?? 0, free: s.disk.free });
      }
      wantsCopy.add(key);
    },

    deleteSetAside(file) {
      loopbackOnly();
      const i = SET_ASIDE_NAME.test(file) ? s.notices.findIndex((x) => x.kind === "set_aside_store" && x.file === file) : -1;
      if (i < 0) throw problem(404, "not_found", "No such file", "No set-aside store file has that name.");
      s.notices.splice(i, 1);
      published();
    },

    startEmpty(store, acceptWarning) {
      loopbackOnly();
      const file = closed.get(store);
      if (file === undefined) throw problem(400, "bad_request", "That store is not closed", "Only a store that was damaged and set aside can start empty.");
      const text = startEmptyText(store, file);
      if (acceptWarning !== sha256Hex(text)) throw consent("store_start_empty", store, text);
      closed.delete(store);
      if (store === "node_state") {
        s.node_state = { state: "closed", reason: null, file_bytes: null };
        s.integrity.node_state = null;
        cls("agent").bytes = 0;
        cls("heal").bytes = 0;
      } else {
        s.integrity.fabric = null;
        cls("fabric").bytes = 0;
      }
      published();
    },

    control: {
      setDiskLow(on) {
        s.disk.disk_low = on;
        published();
      },
      setExportOfferNeeded(which, needed, full = false) {
        const c = cls(which);
        c.export_offer_needed = needed;
        c.full = needed && full && which !== "fabric";
        published();
      },
      setNodeState(state, reason = null) {
        s.node_state = { ...s.node_state, state, reason: state === "unavailable" ? (reason ?? "unavailable") : null };
        // §16.3: an open or a close on use sends no frame; a change to or from `unavailable` does.
        if (state === "unavailable" || reason === "restored") published();
      },
      setIntegrity(store, state) {
        s.integrity[store] = state;
        published();
      },
      damageStore(store, file, bytes) {
        closed.set(store, file);
        s.notices.push({ kind: "set_aside_store", file, bytes });
        if (store === "node_state") s.node_state = { state: "unavailable", reason: "its file was damaged and set aside", file_bytes: null };
        s.integrity[store] = "failed";
        published();
      },
      refuseUpdateForSpace(release, store, needed_bytes) {
        s.notices = s.notices.filter((n) => !(n.kind === "update_refused_space" && n.release === release && n.store === store));
        s.notices.push({ kind: "update_refused_space", release, store, needed_bytes });
        published();
      },
      setUnreadableAgentFiles(count) {
        s.notices = s.notices.filter((n) => n.kind !== "unreadable_agent_files");
        if (count > 0) s.notices.push({ kind: "unreadable_agent_files", count });
        published();
      },
    },
  };
}
