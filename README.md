# Mesh Self-Healing Agentic Operating System

A minimal, open-source operating layer (GPL-3.0-or-later: free for anyone, for any purpose, including commercial use) that connects local AI with existing mesh and offline networks — empowering any device, from a dedicated radio relay to a laptop or an old phone, with offline-first communication, local intelligence, and self-healing.

## Vision

Create a self-bootstrapping, self-healing layer that turns any device into a node in a resilient mesh. It reuses existing mesh networks, bridges them or uses them as transports rather than inventing new ones. A local AI agent serves as both the user interface and the autonomous maintenance system. Custom code stays deliberately small; the intelligence lives in the agent and the bridges.

The system is not a messaging app with built-in rules; it is a general empowerment layer for offline and mesh capability. Human messaging is the flagship service, but the same fabric carries agent operations, knowledge synchronization, software updates, and validated patches. Every node receives an automatically generated cryptographic identity. Nodes with internet access act as gateways to the global network. The agent continuously monitors health, diagnoses failures, validates fixes internally, and proposes changes for human review.

## Core Goals

- One command, one tap, or the simplest possible installation for each device type — on a fresh device or one already in use (e.g., a laptop, or an old/current phone over USB-C).
- Automatic discovery and bridging of existing mesh networks.
- Reliable delivery first; latency is secondary.
- End-to-end encryption with automatic per-device cryptographic identity; proven identity projects are adopted through the Knowledge Layer wherever they meet or exceed this bar.
- Local AI as the primary interface and self-healing engine (where device capacity allows and the user agrees).
- Minimal custom surface area; maximum reuse of proven systems.
- An on-brand, local, offline, web-based dashboard: super minimal, easy to navigate, lightweight, and component-scalable, following the 111 Network design system (`website/docs/DESIGN.md`). It is one of the first things users see and the main way they interact with the system.
- Progressive autonomy: the agent improves the system, proposes validated patches, sends updates and PRs, and coordinates with other nodes to apply fixes where a proven path exists.

## What We Build vs. What We Adopt

The aim is to never reinvent the wheel. Proven external projects are assets, not competitors.

**Custom (deliberately small, isolated in `os/`):**
- The node bootstrapper and thin glue code that detects hardware, selects the device's role, pulls in adopted components, and assembles them into a running node. This is the only intentionally custom piece, kept isolated so the project stays scalable — and retired the moment a proven project can do the job.

**Adopted (tracked in the Knowledge Layer, each one swappable):**
- Mesh and radio stacks, cryptographic identity, local model runtimes, sync transports, storage engines, and the dashboard's design system.

If a solid project exists, we use it. A custom piece exists only where nothing proven fits.

## Layered Architecture

Layers are modular: not every device can run — or needs — every layer. Each device takes, operates, and improves only the layers its hardware and its user's choices allow. The layer-to-category mapping is defined below.

In the repository, each layer is a folder named `N-<slug>`, `N` being the layer number (`0-knowledge`, `1-bootstrap`, `2-identity`, `3-intelligence`, `4-interface`, `5-messaging`, `6-transport`), so the tree reads top to bottom in this order.

### 0. Knowledge Layer

The foundation every other layer reads from. A curated, AI-readable knowledge base covering:

- **Proven external projects**: mesh stacks per platform, identity and encryption systems, local model runtimes, sync transports. Example: if ten proven, solid mesh projects exist for phones, each gets its own entry here.
- **Device and hardware profiles**: what each device class can run, known quirks, install recipes.
- **Protocols and formats** the system speaks.
- **This project's own documentation**.

Format follows AI-readability best practice: Markdown only, one topic per file, short lines, YAML frontmatter with tags/category/status (`proven`, `testing`, `candidate`), wikilinks between entries, a lightweight agent-readable index, and many small files rather than few large ones — structured so the knowledge can be graphed later.

The source of truth lives at the operating-system level in `os/0-knowledge/`. The website has its own clear job; the knowledge base is published to it later via CI.

This layer drives adoption decisions ("if a solid project exists, we use it"), grounds the local agent's reasoning, and provides the per-device install recipes the Bootstrap Layer consumes.

### 1. Bootstrap Layer

The single entry point. One command or one tap per device type, on a fresh device or one already operating — a laptop, or an old/current phone connected over USB-C. On first run it detects hardware and operating environment, determines the device's category and the set of layers it can run, generates a fresh cryptographic identity via the Identity Layer, and installs only what the device needs. No configuration is required from the user.

This bootstrapper is the custom glue described above: isolated, minimal, and replaceable by an adopted project at any time.

### 2. Identity Layer

Every node receives an automatically created cryptographic, wallet-style identity at install time. This identity signs all outgoing traffic and authenticates the node to peers. Identity is offline-first and independent of phone numbers, email, or central authorities. It is adopted from proven identity systems tracked in the Knowledge Layer — and the same keypair can anchor the node's identity on the global network when it operates as a gateway. Users who later care about key management can back up or secure their keys; until then the system remains fully functional without any extra steps.

### 3. Local Intelligence Layer

A local model acts as the continuous agent: the primary interface to the human operator and the autonomous monitor of system health. It runs where device capacity allows and the user agrees — natively on AI-native terminals and compute hubs, while lighter devices route agent requests to a nearby hub over the local mesh. The agent can diagnose problems, test candidate fixes in a safe internal sandbox, and, when a fix is validated, prepare a pull request for human review. The model itself can be updated or expanded by the same self-healing process, allowing the node to grow more capable over time.

### 4. Interface & Dashboard Layer

A local, offline-first, web-based dashboard — one of the first things users see and the main way they interact with the system. Super minimal, easy to navigate, lightweight, and scalable in components, strictly following the 111 Network design system (`website/docs/DESIGN.md`): monochrome, terminal aesthetic, sharp corners, mono and pixel type.

The node serves it locally; the user opens it in any browser on the laptop or phone. It exposes chat, node and mesh status, and agent notifications — and provides the agent with a clean, stable function surface. On devices too constrained to host it, a nearby hub serves it instead.

### 5. Messaging & Data Layer

End-to-end-encrypted, signed payloads with reliable delivery first and latency secondary. Store-and-forward across heterogeneous transports; messages stored encrypted on the device.

The OS imposes no usage policies at runtime: no rate limits, no forced expiry, no governance rules. Payload size and timing are shaped by what the underlying transports can physically carry, not by invented restrictions. Local pruning is capacity-based, not policy-based.

The same fabric carries every kind of traffic the layers need: human messages, agent operations, knowledge-base synchronization, software updates, and validated patches.

### 6. Transport & Bridge Layer

Existing mesh networks are treated as pure transport and knowledge sources. The system maintains a short, ordered list of preferred networks — curated in the Knowledge Layer — and attempts to join them in sequence. When a network is found, the node begins heartbeats and data exchange; if none are present, it falls back through the list.

Bridges join physically incompatible channels (long-range radio with Wi-Fi/BLE) and remain thin: they translate only the minimal common format and never reimplement the underlying radio or routing protocols.

### 7. Relay & Presence Layer

Nodes continuously announce reachability through heartbeats. Nodes that possess internet connectivity advertise that fact and become gateways. Gateways synchronize the offline local mesh state with the global network through an asynchronous store-and-forward transport adopted via the Knowledge Layer: intermittent gateways flush pending queues whenever connectivity appears, while permanent gateways stream global broadcasts and encrypted messages into the local mesh in real time. When a node loses connectivity it continues to serve as a pure mesh participant. Delivery paths form dynamically from the combination of local mesh links and internet-capable gateways.

### 8. Storage & Shared State Layer

Each node keeps an encrypted local store, pruned by capacity rather than by imposed rules. Knowledge hubs host the large shared datasets: offline encyclopedias, regional maps, software updates, and network troubleshooting patches. A lightweight append-only shared log may be used for the small subset of state that benefits from collective consistency — adopted from a proven project, never a custom consensus implementation.

### 9. Self-Healing & Evolution Layer

The local agent continuously observes the health of every layer the device runs. On detecting a failure or degradation it formulates a diagnosis, generates and internally validates a corrective change, and — if the change succeeds — packages it as a pull request. Humans remain the final gate for merging. Nodes share proven fixes with each other through the gateways, so a repair validated on one node can heal others. Over time the same process improves the agent itself, expands the Knowledge Layer, and refines bridging logic — always scoped to the layers the device actually operates.

## Layer ↔ Device Category Mapping

✓ runs natively · ◐ partial or delegated to a nearby node · — not run

| Layer | Cat 1 | Cat 2A | Cat 2B | Cat 3 | Cat 4A | Cat 4B |
|---|---|---|---|---|---|---|
| 0 Knowledge | — | ◐ | ✓ | ◐ | ✓ | ✓ |
| 1 Bootstrap | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 2 Identity | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 3 Local Intelligence | — | ◐ | ✓ | ◐ | ◐ | ✓ |
| 4 Interface & Dashboard | — | ✓ | ✓ | ◐ | ✓ | ✓ |
| 5 Messaging & Data | ◐ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 6 Transport & Bridge | — | — | — | ✓ | ✓ | ✓ |
| 7 Relay & Presence | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 8 Storage & Shared State | — | ◐ | ✓ | ◐ | ✓ | ✓ |
| 9 Self-Healing & Evolution | — | ◐ | ✓ | ◐ | ✓ | ✓ |

Category 5 is not shown as a column: it is an overlay role (see below) that adds gateway duties to whatever tier hosts it. Category 1 relays forward Layer 5 traffic without reading it. Categories 2A and 3 consume the intelligence and self-healing layers through a nearby hub.

## Device Categories

### Category 1: Edge Relays (Physical Infrastructure)

- **General Role**: Pure hardware relays with no user interface and no local AI. They act as the physical extension of the mesh network.
- **Network Capability**: Single-radio devices (e.g., dedicated LoRa, Sub-GHz RF, or Bluetooth).
- **Primary Task**: Packet forwarding, range extension, and minimal signal repeating.

### Category 2: User Terminals (User Interfaces)

- **General Role**: Portable, human-facing interaction endpoints. This is where the user interacts with the dashboard, the chat interface, and the local AI harness.
- **Network Capability**: Lacks long-range RF antennas; relies on short-range local transports (Wi-Fi or Bluetooth/BLE) to pair with nearby bridge nodes.
- **Subcategories**:
  - **2A — Light Terminals**: Hardware-constrained endpoints that cannot run local AI models. They route user input through a nearby local hub or paired node for processing.
  - **2B — AI-Native Terminals**: Devices capable of hosting small, highly compressed local language models (1B–3B parameters). They independently process user prompts, hold encryption keys, and manage contact routing locally.

### Category 3: Transport Bridges (Protocol Translators)

- **General Role**: Multi-transport nodes that bridge physically incompatible communication channels. This is the critical hardware link that joins long-range radio networks with local high-speed Wi-Fi/BLE networks.
- **Network Capability**: Must possess two or more distinct hardware network interfaces (e.g., USB radio + Wi-Fi, BLE + Ethernet).
- **Primary Task**: Real-time packet conversion, local traffic management, protocol translation, and short-term packet buffering.

### Category 4: Local Hubs (Offline Network Brains)

- **General Role**: High-compute, high-storage local network servers that operate completely offline. They serve as the community service layer.
- **Network Capability**: High-throughput local Wi-Fi, Ethernet, or multi-hop local mesh connections.
- **Subcategories**:
  - **4A — Knowledge Hubs**: Nodes equipped with expanded storage that host local offline databases (e.g., offline encyclopedias, regional maps, software updates, and network troubleshooting patches), including a local copy of the Knowledge Layer.
  - **4B — Compute Hubs**: Nodes equipped with dedicated GPUs/accelerators running larger open models (8B–70B parameters). Weaker local devices can offload heavy AI reasoning or code-patch generation to these hubs over the local mesh.

### Category 5: Global Gateways (Global Sync Bridges)

- **General Role**: An overlay role, not a hardware tier. Any node across Categories 2, 3, or 4 that establishes active reachability to the public internet becomes a gateway. (Category 1 devices lack the compute and interfaces to hold this role.)
- **Network Capability**: Dual presence — an active connection to both the local offline mesh and public internet relays.
- **Primary Task**: Synchronizes the offline local mesh state with the global network using the adopted asynchronous transport layer.
- **Subcategories**:
  - **5A — Intermittent Gateways**: Mobile or opportunistic endpoints (e.g., laptops or phones connecting briefly to public networks) that sync pending queues when internet access is available.
  - **5B — Permanent Gateways**: Always-on nodes with stable broadband access that stream incoming global broadcasts and encrypted messages directly into the local physical mesh in real time.

## Core OS Behavioral Principles

- **Automatic Role Inheritance**: Higher-tier nodes automatically absorb and run the software duties of all lower tiers their hardware supports (e.g., a Category 4 Compute Hub with a connected radio board simultaneously operates as a Category 3 bridge, a Category 2B AI-native terminal, and a Category 1 relay). Category 5 is excluded from inheritance — it is an overlay role granted by connectivity, not hardware.
- **Capability-Graded Layer Adoption**: Every device runs only the layers its hardware and its user's consent allow, and contributes improvements to exactly those layers.
- **Intent-Based Routing**: The user's terminal queries the local Service Discovery Registry broadcast by nearby nodes. The user never manually configures IP addresses or routing protocols — the AI handles routing based on current device availability.

## Operating Principles

- Delivery is non-negotiable; speed is optional.
- Code that can be avoided should be avoided.
- Existing networks, tools, and projects for secure offline communication are ALL assets, not competitors.
- Adopt, don't reinvent: wherever a proven project exists, integrate it; write custom code only where nothing proven fits.
- The software imposes no usage policies at runtime — no telemetry, no content policy, no policing of how a node is used. Limits come from hardware and physics, never from invented governance. The license terms (`LICENSE.md`) are a separate matter: they say on what terms the software may be shared and changed, not what a running node allows.
- The agent is both servant and guardian of the node.
- Identity is automatic and cryptographic by default.
- Autonomy is progressive and always human-supervised at the final step.

License: [LICENSE.md](LICENSE.md), the GNU General Public License v3 or (at your option) any later version. Free secure communication is everyone's right: anyone may use, study, change and share 111network for any purpose, including commercial use, and distributed changes stay under the same license with their source. Contributions of any kind are appreciated. Third-party licensing and the dependency boundary (including how Reticulum is used as a separate, unmodified service): [LICENSING.md](LICENSING.md). Third-party notices and license texts: [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
