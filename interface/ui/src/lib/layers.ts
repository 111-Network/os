// The ten layers of the 111 OS as the dashboard shows them (settings pages /settings/l0 .. /settings/l9).
// Titles are plain words first; the layer number is secondary. Descriptions paraphrase the layer
// sections of os/README.md in one sentence each. Pure data, no React.

export interface LayerInfo {
  /** Layer number, 0 to 9. */
  n: number;
  /** `l0` to `l9`: the route segment, `/settings/${slug}`. */
  slug: string;
  /** Plain title shown first in the sidebar and page header. */
  title: string;
  /** One sentence, plain words. */
  description: string;
}

export const LAYERS: readonly LayerInfo[] = [
  {
    n: 0,
    slug: "l0",
    title: "Knowledge",
    description: "A curated, AI-readable library of proven projects, device profiles and protocols that every other layer reads from.",
  },
  {
    n: 1,
    slug: "l1",
    title: "Install & updates",
    description: "The single entry point: detects this device, decides which layers it can run, installs only what it needs and keeps it current.",
  },
  {
    n: 2,
    slug: "l2",
    title: "Identity",
    description: "Every node gets an automatic, wallet-style cryptographic identity that signs its traffic and needs no account or central authority.",
  },
  {
    n: 3,
    slug: "l3",
    title: "Intelligence",
    description: "A local model acts as the node's agent: your main way to talk to the node and the monitor of its health.",
  },
  {
    n: 4,
    slug: "l4",
    title: "Interface",
    description: "This dashboard: a local, offline-first web app served by the node itself, with chat, status and agent notifications.",
  },
  {
    n: 5,
    slug: "l5",
    title: "Messaging & data",
    description: "End-to-end-encrypted, signed messages with store-and-forward delivery across whichever transports are available, and the encrypted store they are kept in.",
  },
  {
    n: 6,
    slug: "l6",
    title: "Transport",
    description: "Joins existing mesh networks as pure transport, tries a preferred list in order and bridges channels that cannot talk to each other.",
  },
  {
    n: 7,
    slug: "l7",
    title: "Relay & presence",
    description: "Nodes announce that they are reachable, and nodes with internet become gateways that sync the offline mesh with the wider network.",
  },
  {
    n: 8,
    slug: "l8",
    title: "Storage",
    description: "An encrypted local store pruned by capacity, plus shared hub datasets such as offline encyclopedias, maps and updates.",
  },
  {
    n: 9,
    slug: "l9",
    title: "Repairs",
    description: "The agent watches every layer, diagnoses failures, tests fixes in a sandbox and prepares a change for a human to approve.",
  },
];

/** The layer for a route segment such as `l3`, or undefined. */
export function layerBySlug(slug: string | undefined): LayerInfo | undefined {
  return slug === undefined ? undefined : LAYERS.find((l) => l.slug === slug);
}
