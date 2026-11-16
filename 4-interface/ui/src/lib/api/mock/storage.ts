// Layer 8 mock (os/8-storage/README.md §4, §13.7, §15, §16): the storage status, the settings with the two serving consent gates
// (`datasets_serve`, `components_serve`), the export offers, and the three store-state writes (an update without a rollback copy, a
// set-aside file, an empty store), and the contract's second part (task C2): the catalogues and holdings, the jobs with their pauses and
// the two dataset consents, drafts and publishing, the encyclopedia search, the tiles listing and the stop of an on-demand component. The
// routes are synchronous and throw ApiError; the adapter (index.ts) wraps them in its latency, auth gate and structuredClone, and
// republishes the `storage` and `dataset` frames this file hands to `publish`. Nothing here is imported by production code.
import { ApiError } from "../problem";
import { datasetPath, type DatasetAccept, type DatasetRef, type DatasetSearchQuery } from "../storage";
import { formatBytes } from "../bridge";
import type {
  ConsentRequired,
  DatasetDraftRequest,
  DatasetDrafts,
  DatasetEntry,
  DatasetPublishRequest,
  DatasetSearchResult,
  DatasetsListing,
  EventsFrame,
  Holding,
  Job,
  Problem,
  ProblemCode,
  Publisher,
  TileSets,
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
const ID = /^[a-z0-9][a-z0-9_.-]{0,63}$/;
const VERSION = /^[A-Za-z0-9][A-Za-z0-9_.+-]{0,39}$/;
/** The pinned home hub of the mock's catalogue, and the node's own id when its status carries none. */
export const MOCK_HOME_HUB = "9f2c41d7a0b35e68c1d4f7092ab36e5d8c70f1e24b9a6d3057c8e1f2a4b6d9c3";
const OWN_FALLBACK = "c3".repeat(32);
const HOME_SOURCE = "http://192.168.1.2:11180";

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

/** `<publisher short>`: the first 16 hex characters of the NodeId (§5.1), the form a consent's subject and text use. */
const shortOf = (node: string): string => node.slice(0, 16);

/** `dataset_fetch` (§15.2, verbatim): the stable facts only; the live figures ride `facts`. */
export function datasetFetchText(e: Pick<DatasetEntry, "name" | "version" | "size" | "licence" | "attribution" | "used_by" | "fits" | "non_commercial">, publisher: string, source: string, metered: { match: string; cap: number } | null): string {
  const connection = metered
    ? `'${metered.match}' is marked metered with a daily cap of ${formatBytes(metered.cap)}: this download pauses before the cap, leaving room for messages, and goes on in the next day's allowance.`
    : "No metered link is set.";
  const meant = e.fits ? ` Meant for: ${e.fits} and up.` : "";
  return `Download ${e.name} ${e.version}, ${formatBytes(e.size)}, from ${source}. It is signed by ${labelOf(publisher)} (${shortOf(publisher)}). Licence: ${e.licence}${e.non_commercial ? ", non-commercial" : ""}. ${e.attribution} Used by: ${e.used_by.join(", ")}.${meant} Connection: ${connection} You can pause or cancel it at any time.`;
}
/** `dataset_override` (§15.2, verbatim). */
export const datasetOverrideText = (name: string, match: string, cap: number): string =>
  `Continue downloading ${name} past today's allowance on '${match}'. This uses more than the ${formatBytes(cap)} you set for this link, which may cost you money. It applies to this download only; messages keep their own room.`;

/** A metered rule the mock's connection matches (§7.8): the label the texts quote, and the daily cap. */
export interface MockMeteredLink {
  match: { iface?: string; ssid?: string };
  cap: number;
}
const matchLabel = (m: MockMeteredLink["match"]): string => m.ssid ?? m.iface ?? "this link";

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
  /** How another component starts (`always` | `on_demand`), or undefined for one that is not declared: the stop route's check. */
  componentStart?(id: string): "always" | "on_demand" | undefined;
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
  datasets(): DatasetsListing;
  checkDatasets(): void;
  fetchDataset(ref: DatasetRef, accept?: DatasetAccept): Job;
  removeDataset(ref: DatasetRef): void;
  cancelDatasetJob(job: string): void;
  resumeDatasetJob(job: string, accept?: DatasetAccept): Job;
  datasetDrafts(): DatasetDrafts;
  createDatasetDraft(req: DatasetDraftRequest): Job;
  publishDatasets(req: DatasetPublishRequest): { seq: number };
  searchDatasets(query: DatasetSearchQuery): DatasetSearchResult;
  tiles(): TileSets;
  stopComponent(id: string): void;
  /** Mock controls for tests and scenario scripts. */
  readonly control: {
    /** A metered link the job's connection matches (null: none, "No metered link is set."). */
    setMetered(link: MockMeteredLink | null): void;
    /** The metered window is spent: a fetch asks for the `dataset_override` consent too, and a running job pauses `paused (metered cap)`. */
    setMeteredCapReached(on: boolean): void;
    /** Move a running job's bytes on (a verified block each); the job ends `done` at its total, and the next queued one starts. One `dataset` frame. */
    advanceJob(id: string, bytes: number): void;
    /** Pause a running or queued job (`offline`, `metered cap`, `metered link`, `disk low`, `source busy`): one `dataset` frame. */
    pauseJob(id: string, why: "offline" | "metered cap" | "metered link" | "disk low" | "source busy"): void;
    /** End a running job `failed (<reason>)`. */
    failJob(id: string, reason: string): void;
    /** kiwix-serve runs (the panel was opened or a search came) or does not (stopped, or idle). */
    setKiwix(running: boolean): void;
    kiwixRunning(): boolean;
    /** A held file's state changed (an unclean shutdown, a detached disk). */
    setHoldingState(ref: DatasetRef, state: "present" | "unverified" | "disk_absent"): void;
    /** The catalogue of a followed publisher arrived (`POST /datasets/check`), with these entries. */
    addCatalogue(publisher: string, entries: readonly CatalogueEntry[], over?: { seq?: number; skipped?: number; source?: string | null }): void;
    setDiskLow(on: boolean): void;
    /** The free bytes of the root filesystem, for the space checks (`insufficient_space`, `facts.free`). */
    setFree(bytes: number): void;
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

/** An entry as a catalogue lists it (§5.3); the node adds `state`, `location` and `non_commercial`. */
export type CatalogueEntry = Omit<DatasetEntry, "state" | "location" | "non_commercial">;

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

  // --- datasets (the contract's second part) -------------------------------------------------------------------------------------------
  const own = (): string => deps.nodeId?.() ?? OWN_FALLBACK;
  const sha = (what: string): string => sha256Hex(`mock-dataset:${what}`);
  const entry = (id: string, version: string, seq: number, over: Partial<CatalogueEntry>): CatalogueEntry => ({
    id,
    kind: "zim",
    name: id,
    version,
    seq,
    size: 1_000_000,
    sha256: sha(`${id}@${version}`),
    blake3: sha(`b3:${id}@${version}`),
    ob_log: 12,
    outboard_sha256: sha(`ob:${id}@${version}`),
    licence: "CC-BY-SA-4.0",
    attribution: "Text from the contributors of the source, CC BY-SA 4.0.",
    source_url: "https://example.org/source",
    built: "2026-10-02",
    used_by: [],
    ...over,
  });
  /** What each publisher's verified catalogue lists (the entries as signed). The home hub's is the mock's start. */
  const catalogues = new Map<string, { seq: number; skipped: number; source: string | null; entries: CatalogueEntry[] }>();
  catalogues.set(MOCK_HOME_HUB, {
    seq: 3,
    skipped: 0,
    source: HOME_SOURCE,
    entries: [
      entry("wikipedia-en-simple", "2026-10", 3, {
        name: "Simple English Wikipedia",
        size: 7_400_028_224,
        attribution: "Text from Wikipedia contributors, CC BY-SA 4.0.",
        source_url: "https://download.kiwix.org/zim/wikipedia/",
        requires: "kiwix",
        fits: "3B",
        used_by: ["encyclopedia", "agent"],
        book: "wikipedia_en_simple_all",
      }),
      entry("world-overview", "2026-09", 2, {
        kind: "pmtiles",
        name: "World overview map",
        size: 120_000_000,
        licence: "ODbL-1.0",
        attribution: "Contains information from OpenStreetMap, which is made available here under the Open Database License (ODbL).",
        source_url: "https://maps.example.org/world",
        used_by: ["map"],
        fits: "2A",
      }),
      entry("field-medicine", "2026-08", 1, {
        name: "Field medicine handbook",
        size: 2_100_000_000,
        licence: "CC-BY-NC-4.0",
        attribution: "Handbook text, CC BY-NC 4.0.",
        requires: "kiwix",
        used_by: ["encyclopedia"],
        book: "field_medicine",
      }),
    ],
  });
  const drafts = new Map<string, CatalogueEntry>(); // `<id>@<version>` -> an entry not yet published
  const held: Holding[] = [];
  const wiki = catalogues.get(MOCK_HOME_HUB)!.entries[0]!;
  held.push({ publisher: MOCK_HOME_HUB, id: wiki.id, version: wiki.version, sha256: wiki.sha256, bytes: wiki.size, location: "origin", state: "present", used_by: [...wiki.used_by] });
  const jobs: Job[] = [];
  const firstCopy = new Map<string, CatalogueEntry>(); // job id -> the draft it will write
  const overridden = new Set<string>(); // jobs the person let run past the cap (`dataset_override`)
  let jobSeq = 0;
  let ownSeq = 0;
  let metered: MockMeteredLink | null = null;
  let capReached = false;
  let kiwix = false;
  const catalogueKey = (e: Pick<DatasetEntry, "id" | "version">): string => `${e.id}@${e.version}`;
  const ended = (j: Job): boolean => j.state === "done" || j.state === "cancelled" || j.state.startsWith("failed");
  const sameRef = (a: DatasetRef, r: DatasetRef): boolean => a.publisher === r.publisher && a.id === r.id && a.version === r.version;
  /** A job's dataset key (its `entry` is the entry id; its own `id` is the job's). */
  const refOf = (j: Job): DatasetRef => ({ publisher: j.publisher, id: j.entry, version: j.version });
  const trusted = (publisher: string): boolean => publisher === MOCK_HOME_HUB || settings.follow.includes(publisher);
  const heldAt = (r: DatasetRef): Holding | undefined => held.find((h) => sameRef(h, r));
  const jobOf = (r: DatasetRef): Job | undefined => jobs.find((j) => !ended(j) && sameRef(refOf(j), r));
  const frame = (j: Job): void => void deps.publish({ event: "dataset", data: structuredClone(j) });
  const setDatasetsBytes = (): void => {
    cls("datasets").bytes = held.filter((h) => h.state !== "partial").reduce((n, h) => n + h.bytes, 0);
  };
  const metering = (): Job["metered"] => (metered ? { match: { ...metered.match }, cap: metered.cap, days: null } : null);
  const daysFor = (size: number): number | undefined => (metered ? Math.max(1, Math.ceil(size / metered.cap)) : undefined);

  /** At most one job runs; the first queued one starts when it ends (§7.4). */
  function startNext(): void {
    if (jobs.some((j) => j.state === "running")) return;
    const next = jobs.find((j) => j.state === "queued");
    if (next) {
      next.state = "running";
      frame(next);
    }
  }
  function finish(j: Job, state: Job["state"]): void {
    j.state = state;
    const i = held.findIndex((h) => h.state === "partial" && sameRef(h, refOf(j)));
    if (state === "done") {
      const draft = firstCopy.get(j.id);
      const sz = j.total_bytes;
      s.disk.free = Math.max(0, s.disk.free - sz);
      if (draft) drafts.set(catalogueKey(draft), draft);
      const rec: Holding = { publisher: j.publisher, id: j.entry, version: j.version, sha256: draft?.sha256 ?? entryOf(refOf(j))?.sha256 ?? sha(`${j.entry}@${j.version}`), bytes: sz, location: "origin", state: "present", used_by: [...(draft?.used_by ?? entryOf(refOf(j))?.used_by ?? [])] };
      if (i >= 0) held[i] = rec;
      else held.push(rec);
    } else if (i >= 0) {
      held.splice(i, 1);
    }
    firstCopy.delete(j.id);
    overridden.delete(j.id);
    setDatasetsBytes();
    frame(j);
    startNext();
  }
  function entryOf(r: DatasetRef): CatalogueEntry | undefined {
    return catalogues.get(r.publisher)?.entries.find((e) => e.id === r.id && e.version === r.version);
  }
  const jobById = (id: string): Job | undefined => jobs.find((j) => j.id === id);

  /** The space check of §4.6 in the mock's terms: the file must leave the disk its floor. */
  function checkSpace(size: number): void {
    if (s.disk.free - size < s.disk.floor) throw problem(409, "insufficient_space", "Not enough room", `This needs ${formatBytes(size)} and the node has ${formatBytes(s.disk.free)} free, with ${formatBytes(s.disk.floor)} kept in reserve.`);
  }
  /** The `dataset_fetch` consent, then (at a cap) the `dataset_override` one; throws the first still owed. */
  function consentFor(e: CatalogueEntry, publisher: string, accept: DatasetAccept | undefined, name: string, source: string): void {
    const nc = /-NC-|-NC$/i.test(e.licence);
    const link = metered ? { match: matchLabel(metered.match), cap: metered.cap } : null;
    const text = datasetFetchText({ ...e, non_commercial: nc }, publisher, source, link);
    const subject = `${shortOf(publisher)}:${e.id}@${e.version}`;
    if (accept?.warning !== sha256Hex(text)) {
      const addons = [...(metered ? ["metered"] : []), ...(nc ? ["non_commercial"] : [])];
      const days = daysFor(e.size);
      throw problem(409, "consent_required", "Confirm first", "This needs your confirmation.", {
        action: "dataset_fetch",
        subject,
        warning_text: text,
        hash: sha256Hex(text),
        addons,
        facts: { free: s.disk.free, free_after: Math.max(0, s.disk.free - e.size), ...(days === undefined ? {} : { days }) },
      });
    }
    if (metered && capReached) {
      const o = datasetOverrideText(name, matchLabel(metered.match), metered.cap);
      if (accept?.override !== sha256Hex(o)) {
        throw problem(409, "consent_required", "Confirm first", "This needs your confirmation.", { action: "dataset_override", subject, warning_text: o, hash: sha256Hex(o), addons: ["metered"], facts: { remaining: e.size } });
      }
    }
  }
  function newJob(publisher: string, e: CatalogueEntry, source: string | null): Job {
    const j: Job = {
      id: `job-${(++jobSeq).toString(36).padStart(4, "0")}`,
      publisher,
      entry: e.id,
      version: e.version,
      state: jobs.some((x) => x.state === "running") ? "queued" : "running",
      done_bytes: 0,
      total_bytes: e.size,
      source: source,
      metered: metering(),
      override_bytes: 0,
    };
    if (j.metered) j.metered.days = daysFor(e.size) ?? null;
    jobs.push(j);
    while (jobs.filter(ended).length > 3) jobs.splice(jobs.findIndex(ended), 1);
    held.push({ publisher, id: e.id, version: e.version, sha256: e.sha256, bytes: 0, location: "origin", state: "partial", used_by: [...e.used_by] });
    frame(j);
    return j;
  }
  const entryView = (publisher: string, e: CatalogueEntry): DatasetEntry => {
    const h = heldAt({ publisher, id: e.id, version: e.version });
    const active = jobOf({ publisher, id: e.id, version: e.version });
    const state: DatasetEntry["state"] = active ? "partial" : h ? h.state : "not_held";
    return { ...e, used_by: [...e.used_by], state, location: state === "not_held" ? null : (h?.location ?? "origin"), non_commercial: /-NC-|-NC$/i.test(e.licence) };
  };
  const bad = (message: string, pointer?: string): ApiError => new ApiError({ type: "about:blank", title: "Invalid request", status: 400, code: "bad_request", detail: message, ...(pointer ? { errors: [{ pointer, message }] } : {}) });

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


    datasets() {
      const pubs: Publisher[] = [];
      for (const [id, c] of catalogues) {
        if (!trusted(id)) continue;
        pubs.push({ node_id: id, label: labelOf(id), home: id === MOCK_HOME_HUB, seq: c.seq, source: c.source, sources: [...(settings.sources[id] ?? [])], skipped: c.skipped, entries: c.entries.map((e) => entryView(id, e)) });
      }
      for (const f of settings.follow) {
        if (!catalogues.has(f)) pubs.push({ node_id: f, label: labelOf(f), home: false, seq: 0, source: null, sources: [...(settings.sources[f] ?? [])], skipped: 0, entries: [] });
      }
      if (isHub() && ownSeq > 0) {
        const mine = catalogues.get(own());
        if (mine) pubs.push({ node_id: own(), label: labelOf(own()), home: false, seq: mine.seq, source: null, sources: [], skipped: 0, entries: mine.entries.map((e) => entryView(own(), e)) });
      }
      return structuredClone({ schema: 1, publishers: pubs, holdings: held.filter((h) => h.state !== "partial"), jobs });
    },

    checkDatasets() {
      // One GET per source and catalogue name; the mock's catalogues are already the newest it will see (a test brings a newer one with `addCatalogue`).
    },

    fetchDataset(ref, accept) {
      if (!ID.test(ref.id) || !VERSION.test(ref.version) || !HEX64.test(ref.publisher)) throw bad("The publisher, id or version is not well formed.");
      if (!trusted(ref.publisher)) throw problem(409, "untrusted_publisher", "Not a publisher this node follows", "Only the home hub and the publishers you follow are listed.");
      const e = entryOf({ publisher: ref.publisher, id: ref.id, version: ref.version });
      if (!e) throw problem(404, "not_found", "No such dataset", "That publisher's catalogue lists no such entry.");
      if (jobOf(ref)) throw problem(409, "job_running", "Already on its way", "That dataset already has a job.");
      if (heldAt(ref)?.state === "present") throw bad("That dataset is already held.");
      checkSpace(e.size);
      const source = settings.sources[ref.publisher]?.[0] ?? (catalogues.get(ref.publisher)?.source ?? HOME_SOURCE);
      consentFor(e, ref.publisher, accept, e.name, source);
      const j = newJob(ref.publisher, e, source);
      if (accept?.override !== undefined && capReached) overridden.add(j.id);
      return structuredClone(j);
    },

    removeDataset(ref) {
      const i = held.findIndex((h) => sameRef(h, ref) && h.state !== "partial");
      if (i < 0) throw problem(404, "not_found", "Not held", "This node does not hold that dataset.");
      held.splice(i, 1);
      setDatasetsBytes();
    },

    cancelDatasetJob(job) {
      const j = jobById(job);
      if (!j || ended(j)) throw problem(404, "not_found", "No such job", "That job does not exist or has already ended.");
      finish(j, "cancelled");
    },

    resumeDatasetJob(job, accept) {
      const j = jobById(job);
      if (!j || ended(j)) throw problem(404, "not_found", "No such job", "That job does not exist or has already ended.");
      if (!j.state.startsWith("paused")) throw problem(409, "bad_request", "It is not paused", "Only a paused job resumes.");
      const e = entryOf(refOf(j)) ?? firstCopy.get(j.id);
      if (j.state === "paused (metered link)" && e) {
        const nc = /-NC-|-NC$/i.test(e.licence);
        const link = metered ? { match: matchLabel(metered.match), cap: metered.cap } : null;
        const text = datasetFetchText({ ...e, non_commercial: nc }, j.publisher, j.source ?? HOME_SOURCE, link);
        if (accept?.warning !== sha256Hex(text)) {
          throw problem(409, "consent_required", "Confirm first", "This needs your confirmation.", { action: "dataset_fetch", subject: `${shortOf(j.publisher)}:${e.id}@${e.version}`, warning_text: text, hash: sha256Hex(text), addons: [...(metered ? ["metered"] : []), ...(nc ? ["non_commercial"] : [])], facts: { free: s.disk.free, free_after: Math.max(0, s.disk.free - e.size), ...(daysFor(e.size) === undefined ? {} : { days: daysFor(e.size)! }) } });
        }
        j.metered = metering();
      }
      if (j.state === "paused (metered cap)" && metered) {
        const o = datasetOverrideText(e?.name ?? j.entry, matchLabel(metered.match), metered.cap);
        if (accept?.override !== sha256Hex(o)) {
          throw problem(409, "consent_required", "Confirm first", "This needs your confirmation.", { action: "dataset_override", subject: `${shortOf(j.publisher)}:${j.entry}@${j.version}`, warning_text: o, hash: sha256Hex(o), addons: ["metered"], facts: { remaining: Math.max(0, j.total_bytes - j.done_bytes) } });
        }
        overridden.add(j.id);
      }
      j.state = jobs.some((x) => x !== j && x.state === "running") ? "queued" : "running";
      frame(j);
      return structuredClone(j);
    },

    datasetDrafts() {
      return structuredClone({ schema: 1, drafts: [...drafts.values()].map((e) => ({ draft: catalogueKey(e), entry: { ...e, used_by: [...e.used_by], state: "present", location: "origin", non_commercial: /-NC-|-NC$/i.test(e.licence) } satisfies DatasetEntry })) });
    },

    createDatasetDraft(req) {
      if (!isHub()) throw problem(409, "needs_hub", "This needs a hub", "Turn the hub role on first (Settings, Messaging).");
      if (typeof req.url !== "string" || req.url === "" || req.url.length > 2048 || !/^https?:\/\//.test(req.url)) throw bad("The upstream must be an http or https URL.", "/url");
      if (!ID.test(req.id)) throw bad("The id is not well formed.", "/id");
      if (!VERSION.test(req.version)) throw bad("The version is not well formed.", "/version");
      if (req.kind !== "zim" && req.kind !== "pmtiles") throw bad("The kind must be zim or pmtiles.", "/kind");
      if (req.sha256 !== undefined && !HEX64.test(req.sha256)) throw bad("The sha256 must be 64 lowercase hex characters.", "/sha256");
      if (drafts.has(`${req.id}@${req.version}`) || jobs.some((j) => !ended(j) && j.publisher === own() && j.entry === req.id && j.version === req.version)) throw problem(409, "job_running", "Already on its way", "That draft already has a job or a file.");
      const e = entry(req.id, req.version, 0, { kind: req.kind, name: req.name, size: 1_000_000_000, licence: req.licence, attribution: req.attribution, source_url: req.source_url, built: req.built ?? "", used_by: [...req.used_by], ...(req.requires ? { requires: req.requires } : {}), ...(req.fits ? { fits: req.fits } : {}), ...(req.book ? { book: req.book } : {}), ...(req.sha256 ? { sha256: req.sha256 } : {}) });
      checkSpace(e.size);
      consentFor(e, own(), { warning: req.accept_warning, override: req.accept_override }, e.name, req.url);
      const j = newJob(own(), e, req.url);
      firstCopy.set(j.id, e);
      if (req.accept_override !== undefined && capReached) overridden.add(j.id);
      return structuredClone(j);
    },

    publishDatasets(req) {
      if (!isHub()) throw problem(409, "needs_hub", "This needs a hub", "Turn the hub role on first (Settings, Messaging).");
      const mine = catalogues.get(own()) ?? { seq: 0, skipped: 0, source: null, entries: [] as CatalogueEntry[] };
      for (const d of req.drafts) if (!drafts.has(d)) throw bad(`No draft named ${d}.`, "/drafts");
      for (const r of req.remove) if (!mine.entries.some((e) => e.id === r.id && e.version === r.version)) throw bad(`The catalogue lists no ${r.id}@${r.version}.`, "/remove");
      if (req.drafts.length === 0 && req.remove.length === 0) throw bad("Nothing to publish.");
      ownSeq = Math.max(ownSeq, mine.seq) + 1;
      mine.entries = mine.entries.filter((e) => !req.remove.some((r) => r.id === e.id && r.version === e.version));
      for (const d of req.drafts) {
        const e = drafts.get(d)!;
        mine.entries.push({ ...e, seq: ownSeq });
        drafts.delete(d);
      }
      mine.seq = ownSeq;
      catalogues.set(own(), mine);
      return { seq: ownSeq };
    },

    searchDatasets(query) {
      if (typeof query.q !== "string" || query.q.trim() === "" || query.q.length > 256) throw bad("A search needs words.", "/q");
      if (query.n !== undefined && (!Number.isInteger(query.n) || query.n < 1 || query.n > 20)) throw bad("n is between 1 and 20.", "/n");
      if (query.publisher !== undefined && !HEX64.test(query.publisher)) throw bad("The publisher is a NodeId.", "/publisher");
      if (query.book !== undefined) {
        const e = catalogues.get(query.publisher ?? MOCK_HOME_HUB)?.entries.find((x) => x.id === query.book);
        if (!e || !e.book) throw bad("This encyclopedia cannot be searched by name.", "/book");
      }
      if (!kiwix) throw problem(409, "component_unavailable", "The encyclopedia is off", "The encyclopedia is off.");
      const title = query.q.replace(/[<>&"']/g, "");
      const body = `<?xml version="1.0" encoding="UTF-8"?><feed xmlns="http://www.w3.org/2005/Atom"><title>Search: ${title}</title><totalResults>1</totalResults><entry><title>${title}</title></entry></feed>`;
      return { schema: 1, format: "opensearch-xml", body: body.slice(0, 8192) };
    },

    tiles() {
      const sets: TileSets["sets"] = [];
      for (const h of held) {
        if (h.state !== "present") continue;
        const e = entryOf(h);
        if (!e || e.kind !== "pmtiles") continue;
        sets.push({ id: h.id, publisher: h.publisher, version: h.version, name: e.name, bytes: h.bytes, licence: e.licence, attribution: e.attribution, url: `/api/v1/tiles/${datasetPath(h)}` });
      }
      return structuredClone({ schema: 1, sets });
    },

    stopComponent(id) {
      const start = id === "kiwix" ? "on_demand" : deps.componentStart?.(id);
      if (start === undefined) throw problem(404, "not_found", "No such component", "No component has that id.");
      if (start !== "on_demand") throw problem(400, "bad_request", "It is not an on-demand component", "Only an on-demand component stops this way; turn others off with Disable.");
      kiwix = false;
    },

    control: {
      setMetered(link) {
        metered = link ? { match: { ...link.match }, cap: link.cap } : null;
      },
      setMeteredCapReached(on) {
        capReached = on;
        if (on && metered) {
          for (const j of jobs) {
            if (j.state === "running" && !overridden.has(j.id)) {
              j.state = "paused (metered cap)";
              frame(j);
            }
          }
          startNext();
        }
      },
      advanceJob(id, bytes) {
        const j = jobById(id);
        if (!j || j.state !== "running") return;
        const step = Math.max(0, Math.min(bytes, j.total_bytes - j.done_bytes));
        j.done_bytes += step;
        if (overridden.has(j.id) && capReached) j.override_bytes += step;
        if (j.done_bytes >= j.total_bytes) finish(j, "done");
        else frame(j);
      },
      pauseJob(id, why) {
        const j = jobById(id);
        if (!j || ended(j)) return;
        j.state = `paused (${why})`;
        frame(j);
        startNext();
      },
      failJob(id, reason) {
        const j = jobById(id);
        if (j && !ended(j)) finish(j, `failed (${reason})`);
      },
      setKiwix(running) {
        kiwix = running;
      },
      kiwixRunning: () => kiwix,
      setHoldingState(ref, state) {
        const h = heldAt(ref);
        if (h && h.state !== "partial") h.state = state;
      },
      addCatalogue(publisher, entries, over = {}) {
        const prev = catalogues.get(publisher);
        catalogues.set(publisher, { seq: over.seq ?? (prev?.seq ?? 0) + 1, skipped: over.skipped ?? 0, source: over.source === undefined ? HOME_SOURCE : over.source, entries: entries.map((e) => ({ ...e, used_by: [...e.used_by] })) });
      },
      setFree(bytes) {
        s.disk.free = Math.max(0, Math.floor(bytes));
      },
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
