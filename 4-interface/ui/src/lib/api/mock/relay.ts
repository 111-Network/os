// Layer 7 mock (os/7-relay/README.md §3.5, §4.6, §6.3, §7, §9.3, §12, §13): the relay status and settings with the three consent gates
// (`carrier_enable`, `gateway_bit`, `origin_serve`), the heartbeat refusals, pairing with the role-complement and `pair_plain` rules,
// the registry browse, the hop read and the update origins. The routes are synchronous and throw ApiError; the adapter (index.ts) wraps
// them in its latency, pairing gate and structuredClone, and republishes the frames this file hands to `publish`.
// Secrets are write-only here too: no route returns a token or topic secret, only `{set: boolean}`. Nothing here is imported by
// production code.
import { ApiError } from "../problem";
import type {
  EventsFrame,
  NodeId,
  PairOffer,
  PairOfferRequest,
  PairedHub,
  Problem,
  ProblemCode,
  RegistryRow,
  RelayAccepted,
  RelaySettingsPatch,
  RelayStatus,
  UpdateOrigin,
} from "../types";
import type { PairInput } from "../relay";
import { labelOf, mockCardText } from "./messaging";
import { cardFor } from "./qr";
import { sha256Hex } from "./sha256";

// --- The consent texts, verbatim from the spec (§6.2, §12) ---------------------------------------------------------------------

export const carrierConsentText = (server: string, far: string, capBytes: number | null = null): string =>
  `To carry messages between this hub and ${far}, 111 will post sealed packets to the ntfy server at ${server} and read packets back from it. The server's operator can see this hub's internet address, the topic names and a short tag of the two hubs' ids, how many packets are posted, their sizes and when. The operator cannot read them or tell which person they are for. Packets stay only as long as the server keeps them (12 hours on a default server); a lost packet is sent again. Nothing is sent while this switch is off.${capBytes === null ? "" : ` A daily cap of ${capBytes} bytes applies on a link you marked metered.`}`;
export const GATEWAY_BIT_TEXT =
  "This adds 'willing to act as a gateway' to this node's signed card. Everyone who receives the card, on any network, learns that this node has internet. Your hub's members learn it without this.";
export const ORIGIN_SERVE_TEXT =
  "Any device on this network can download this hub's verified update files without signing in. They are public files signed by the 111 release key; nothing about you is in them.";
export const PAIR_PLAIN_TEXT =
  "This hub is on the open internet and this link cannot be encrypted. Anyone on the path between the two hubs can see which hubs talk, when, and how much. Message contents stay sealed. Pair anyway?";

const PRIVATE_V4 = /^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|169\.254\.|100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.)/;
/** RFC 1918, RFC 6598, link-local and ULA are private (§4.1 rule 1); a host name counts as not private. */
export const isPrivateAddress = (address: string): boolean => {
  const host = address.replace(/^\[/, "").replace(/\]?:\d+$/, "").replace(/\]$/, "");
  return PRIVATE_V4.test(host) || /^f[cd][0-9a-f]{2}:/i.test(host) || /^fe80:/i.test(host);
};

export interface RelayMockDeps {
  now(): number;
  later(ms: number, fn: () => void): void;
  publish(frame: EventsFrame): number;
  /** `loopback_only` routes refuse unless this says the caller is on the node itself. */
  loopback(): boolean;
  /** A `nearby` frame is due (the hop read changed `hops_away`). */
  onHops?(): void;
}

export interface RelayMockOptions {
  /** This node has the hub role (default true): `needs_hub` otherwise. */
  hub?: boolean;
  /** LAN sharing is on (default true): the registry browse answers `lan_off` otherwise. */
  lan?: boolean;
  /** The Reticulum component is on (default true): the hop read answers `component_unavailable` otherwise. */
  reticulum?: boolean;
}

/** The synchronous routes; index.ts wraps each in its latency, auth gate and clone. */
export interface RelayRoutes {
  status(): RelayStatus;
  patchSettings(patch: RelaySettingsPatch, acceptWarning?: string): RelayStatus;
  pairOffer(body: PairOfferRequest): PairOffer;
  pair(body: PairInput): PairedHub;
  unpair(nodeId: NodeId): void;
  checkCarrier(): RelayAccepted;
  flushCarrier(): RelayAccepted;
  registry(): RegistryRow[];
  browseRegistry(): RelayAccepted;
  nearbyHops(): RelayAccepted;
  updateOrigins(): UpdateOrigin[];
  /** Mock controls for tests and scenario scripts. */
  readonly control: {
    setMeteredCapReached(on: boolean): void;
    /** What the carrier's next check answers. */
    setCarrierReachable(ok: boolean): void;
    setPending(n: number): void;
    /** Bump the session counters of the fabric's forwarder. */
    bumpForward(by: { refused?: number; too_large?: number }): void;
  };
}

function problem(status: number, code: ProblemCode, title: string, detail?: string, consent?: Problem["consent"]): ApiError {
  const p: Problem = { type: "about:blank", title, status, code };
  if (detail) p.detail = detail;
  if (consent) p.consent = consent;
  return new ApiError(p);
}

const ROLE_COMPLEMENT: Record<string, string> = { peer: "peer", leaf: "gateway", gateway: "leaf" };

export function createRelayMock(deps: RelayMockDeps, opts: RelayMockOptions = {}): RelayRoutes {
  const isHub = opts.hub ?? true;
  const lan = opts.lan ?? true;
  const reticulum = opts.reticulum ?? true;

  let carrierReachable = true;
  let checkedAt: number | null = null;
  let flushedAt: number | null = null;
  let hopsAt: number | null = null;
  let offerSeq = 0;
  const offers = new Map<string, { role: string; via: string }>();
  let registry: RegistryRow[] = [];

  const s: RelayStatus = {
    schema: 1,
    gateway: { enabled: false, state: "gateway_none", carrier: null, endpoint: null, mode: null, checked_ago_s: null, last_flush_ago_s: null, pending: 0, metered_cap_reached: false },
    carrier: { kind: "none", server: null, mode: "5A", token: { set: false }, topic_secret: { set: false }, consent_hash: null },
    paired_hubs: [],
    forward: { refused: 0, too_large: 0 },
    presence: {
      heartbeat: [
        { transport: "tcp-lan", interval_min: 0, owner: "layer7" },
        { transport: "rns", interval_min: 60, owner: "layer6" },
      ],
    },
    discovery: { lan_record: false, public_gateway_bit: false, offload_bit: false },
    origin: { serve: false, targets: ["x86_64-unknown-linux-musl"], bytes: 0 },
    updates: { check_internet: true },
    metered: [],
  };
  let gatewayBitHash: string | null = null;
  let originHash: string | null = null;
  const origins: UpdateOrigin[] = [
    { kind: "hub", url: "http://192.168.1.20:11180/origin/v1/", last_answer: null, newest_seen: null },
    { kind: "internet", url: "https://install.111.network/", last_answer: { ok: true, ago_s: 1800, detail: null }, newest_seen: "2026.11.0" },
    { kind: "internet", url: "https://github.com/111network/os/releases/", last_answer: null, newest_seen: null },
  ];

  const ageS = (at: number | null): number | null => (at === null ? null : Math.max(0, Math.floor((deps.now() - at) / 1000)));
  const farLabel = (): string => s.paired_hubs[0]?.label ?? "the other hub";
  const capBytes = (): number | null => (s.metered.length > 0 ? Math.max(...s.metered.map((m) => m.daily_cap_bytes)) : null);

  function derive(): void {
    const g = s.gateway;
    const configured = s.carrier.kind !== "none";
    g.carrier = configured ? s.carrier.kind : null;
    g.endpoint = configured && s.carrier.server ? new URL(s.carrier.server).host : null;
    g.mode = configured ? (s.carrier.mode as "5A" | "5B") : null;
    g.checked_ago_s = ageS(checkedAt);
    g.last_flush_ago_s = ageS(flushedAt);
    g.state = !configured ? "gateway_none" : !g.enabled ? "gateway_off" : checkedAt !== null && carrierReachable && g.pending === 0 ? "carrier_ok" : "carrier_pending";
  }
  const snapshot = (): RelayStatus => {
    derive();
    return structuredClone(s);
  };
  const changed = (): void => {
    deps.publish({ event: "relay", data: snapshot() });
  };

  const consent = (action: "carrier_enable" | "gateway_bit" | "origin_serve" | "pair_plain", subject: string, text: string, addons: string[] = []): ApiError =>
    problem(409, "consent_required", "Confirm first", "This needs your confirmation.", { action, subject, warning_text: text, hash: sha256Hex(text), addons });
  const needConsent = (accept: string | undefined, recorded: string | null, text: string): boolean => sha256Hex(text) !== recorded && accept !== sha256Hex(text);
  const loopbackOnly = (): void => {
    if (!deps.loopback()) throw problem(403, "loopback_only", "Available on the node itself", "Change this from the machine the node runs on.");
  };
  const needsHub = (): ApiError => problem(409, "needs_hub", "This needs a hub", "Turn the hub role on first (Settings, Messaging).");

  function guardCarrierCall(): void {
    if (s.carrier.kind === "none") throw problem(409, "bad_request", "No carrier", "Set up a carrier first (Settings, Layer 7).");
    if (s.carrier.consent_hash === null) throw consent("carrier_enable", `${s.carrier.kind}:${s.carrier.server ?? ""}`, carrierConsentText(s.carrier.server ?? "", farLabel(), capBytes()));
    if (s.gateway.metered_cap_reached) throw problem(409, "metered_cap", "Daily cap reached", "The carrier has used today's allowance on this metered link; nothing is sent until it passes.");
  }

  return {
    status: snapshot,

    patchSettings(patch, acceptWarning) {
      const accept = acceptWarning ?? patch.accept_warning;
      const hb = patch.presence?.heartbeat;
      if (hb && hb.length > 0) {
        if (hb.some((h) => h.transport === "rns")) throw problem(409, "settings_locked", "Set by Layer 6", "The Reticulum announce interval is Layer 6's setting: change it with PATCH /api/v1/bridge/settings.");
        throw problem(409, "no_such_transport", "No such transport", "No transport takes a presence beat in this version: events only.");
      }
      if (patch.gateway?.enabled === true && !isHub) throw needsHub();
      if (patch.origin?.serve === true) {
        if (!isHub) throw needsHub();
        if (!lan) throw problem(409, "lan_off", "LAN sharing is off", "Serving update files needs LAN sharing.");
      }
      const next = {
        kind: patch.carrier?.kind ?? s.carrier.kind,
        server: patch.carrier?.server !== undefined ? patch.carrier.server : s.carrier.server,
        metered: patch.metered ?? s.metered,
      };
      if (next.kind === "ntfy") {
        let ok = false;
        try {
          const u = new URL(next.server ?? "");
          ok = u.protocol === "https:" || (u.protocol === "http:" && isPrivateAddress(u.host));
        } catch {
          ok = false;
        }
        if (!ok) throw problem(400, "bad_request", "Not a usable server", "An https:// address, or http:// for a private address only.");
        const text = carrierConsentText(next.server ?? "", farLabel(), next.metered.length > 0 ? Math.max(...next.metered.map((m) => m.daily_cap_bytes)) : null);
        const turningOn = s.carrier.kind !== "ntfy" || next.server !== s.carrier.server || (patch.metered !== undefined && JSON.stringify(patch.metered) !== JSON.stringify(s.metered));
        if (turningOn && needConsent(accept, s.carrier.consent_hash, text)) throw consent("carrier_enable", `ntfy:${next.server}`, text, next.metered.length > 0 ? ["metered"] : []);
        if (turningOn) s.carrier.consent_hash = sha256Hex(text);
      }
      if (patch.discovery?.public_gateway_bit === true && !s.discovery.public_gateway_bit && needConsent(accept, gatewayBitHash, GATEWAY_BIT_TEXT)) throw consent("gateway_bit", "public_gateway_bit", GATEWAY_BIT_TEXT);
      if (patch.origin?.serve === true && !s.origin.serve && needConsent(accept, originHash, ORIGIN_SERVE_TEXT)) throw consent("origin_serve", "origin.serve", ORIGIN_SERVE_TEXT);

      // validated; apply
      if (patch.gateway?.enabled !== undefined) s.gateway.enabled = patch.gateway.enabled;
      if (patch.gateway?.termux_poll_min !== undefined) s.gateway.termux_poll_min = patch.gateway.termux_poll_min;
      if (patch.carrier) {
        s.carrier.kind = next.kind;
        s.carrier.server = next.kind === "none" ? null : next.server;
        if (patch.carrier.mode) s.carrier.mode = patch.carrier.mode;
        if (patch.carrier.token !== undefined) s.carrier.token = { set: patch.carrier.token !== null && patch.carrier.token !== "" };
        if (patch.carrier.topic_secret !== undefined) s.carrier.topic_secret = { set: patch.carrier.topic_secret !== null && patch.carrier.topic_secret !== "" };
        if (next.kind === "none") s.carrier.consent_hash = null;
      }
      if (patch.metered) s.metered = structuredClone(patch.metered);
      if (patch.discovery) {
        Object.assign(s.discovery, patch.discovery);
        if (patch.discovery.public_gateway_bit === true) gatewayBitHash = sha256Hex(GATEWAY_BIT_TEXT);
      }
      if (patch.origin) {
        if (patch.origin.serve !== undefined) s.origin.serve = patch.origin.serve;
        if (patch.origin.serve === true) originHash = sha256Hex(ORIGIN_SERVE_TEXT);
        if (patch.origin.targets) s.origin.targets = [...patch.origin.targets];
      }
      if (patch.updates?.check_internet !== undefined) s.updates.check_internet = patch.updates.check_internet;
      changed();
      return snapshot();
    },

    pairOffer(body) {
      loopbackOnly();
      if (!isHub) throw needsHub();
      if (body.via === "carrier" && body.role !== "peer") throw problem(409, "pair_role_mismatch", "Not a possible pair", "A carrier pair is a peer pair.");
      if ((body.role === "leaf" || body.role === "gateway") && body.via !== "tcp") throw problem(409, "pair_role_mismatch", "Not a possible pair", "A leaf or gateway pair uses a direct link.");
      const code = `PAIR${(++offerSeq).toString().padStart(4, "0")}`;
      offers.set(code, { role: body.role, via: body.via });
      const card = mockCardText("9f2c41d7");
      const spki = mockCardText("a1b2c3d4").slice(0, 71);
      const text = `111PAIR:${card}:${code}:${spki}:192.168.1.42:11183:${body.role}:${body.via}`;
      let bundle;
      try {
        bundle = cardFor(text);
      } catch {
        bundle = { text, qr: cardFor("111PAIR").qr }; // a real QR of this length needs a larger symbol than the mock draws
      }
      return { schema: 1, bundle, code, link_spki: spki };
    },

    pair(body) {
      loopbackOnly();
      if (!isHub) throw needsHub();
      const parts = body.bundle.startsWith("111PAIR:") ? body.bundle.slice(8).split(":") : [];
      if (parts.length < 6) throw problem(400, "bad_request", "Not a pairing bundle", "Paste the text that starts with 111PAIR:.");
      const role = parts[parts.length - 2]!;
      const via = parts[parts.length - 1]!;
      const card = parts[0]!;
      const address = parts.slice(3, parts.length - 2).join(":");
      if (ROLE_COMPLEMENT[role] !== body.role || via !== body.via) throw problem(409, "pair_role_mismatch", "The roles do not match", `The other hub proposes ${role} over ${via}: pick ${ROLE_COMPLEMENT[role] ?? "the complement"} over ${via}.`);
      const nodeId = sha256Hex(card);
      if (s.paired_hubs.some((h) => h.node_id === nodeId)) throw problem(409, "bad_request", "Already paired", "This hub is already paired.");
      const lanMark = body.lan === true;
      const priv = address === "" || isPrivateAddress(address);
      const tlsPossible = false; // the mock's far hub has no TLS build: the warning applies to any non-private address that is not marked lan
      if (via === "tcp" && !priv && !lanMark && !tlsPossible && body.accept_warning !== sha256Hex(PAIR_PLAIN_TEXT)) throw consent("pair_plain", nodeId, PAIR_PLAIN_TEXT);
      const hub: PairedHub = {
        node_id: nodeId,
        label: labelOf(nodeId),
        role: body.role,
        via: body.via,
        tls: via === "carrier" || lanMark || priv ? "none" : "plain",
        lan: lanMark,
        reachable: false,
        pending: 0,
        reposts: 0,
      };
      s.paired_hubs.push(hub);
      changed();
      deps.later(1500, () => {
        const h = s.paired_hubs.find((x) => x.node_id === nodeId);
        if (h && !h.reachable) {
          h.reachable = true;
          changed();
        }
      });
      return structuredClone(hub);
    },

    unpair(nodeId) {
      loopbackOnly();
      const n = s.paired_hubs.length;
      s.paired_hubs = s.paired_hubs.filter((h) => h.node_id !== nodeId);
      if (s.paired_hubs.length !== n) changed();
    },

    checkCarrier() {
      guardCarrierCall();
      deps.later(400, () => {
        checkedAt = deps.now();
        changed();
      });
      return { schema: 1, started: true };
    },

    flushCarrier() {
      guardCarrierCall();
      deps.later(600, () => {
        checkedAt = deps.now();
        flushedAt = checkedAt;
        s.gateway.pending = 0;
        for (const h of s.paired_hubs) h.pending = 0;
        changed();
      });
      return { schema: 1, started: true };
    },

    registry: () => structuredClone(registry),

    browseRegistry() {
      if (!lan) throw problem(409, "lan_off", "LAN sharing is off", "Looking for nodes here needs LAN sharing.");
      deps.later(800, () => {
        const k = sha256Hex("registry-hint");
        registry = [
          { id: k, label: labelOf(k), node_id: k, address: "192.168.1.31:11183", roles: ["mailbox", "hub"], answered_by: "hint", heard_ago_s: 1 },
          { id: sha256Hex("registry-signed"), label: labelOf(sha256Hex("registry-signed")), node_id: sha256Hex("registry-signed"), address: null, roles: ["mailbox"], answered_by: "signed", heard_ago_s: 95 },
        ];
        deps.publish({ event: "registry", data: { rows: structuredClone(registry) } });
      });
      return { schema: 1, started: true };
    },

    nearbyHops() {
      if (!reticulum) throw problem(409, "component_unavailable", "Reticulum is off", "Turn the Reticulum component on to read hop counts.");
      const now = deps.now();
      if (hopsAt !== null && now - hopsAt < 30_000) throw problem(429, "rate_limited", "Too soon", "Hop counts were read less than 30 seconds ago.");
      hopsAt = now;
      deps.later(500, () => deps.onHops?.());
      return { schema: 1, started: true };
    },

    updateOrigins: () => structuredClone(origins),

    control: {
      setMeteredCapReached(on) {
        s.gateway.metered_cap_reached = on;
        changed();
      },
      setCarrierReachable(ok) {
        carrierReachable = ok;
        changed();
      },
      setPending(n) {
        s.gateway.pending = Math.max(0, n);
        changed();
      },
      bumpForward(by) {
        s.forward.refused += by.refused ?? 0;
        s.forward.too_large += by.too_large ?? 0;
        changed();
      },
    },
  };
}
