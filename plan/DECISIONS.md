# DECISIONS — 111network os build log

ADR-lite. Every decision, its rejected alternatives, and why. Newest last; never delete, only supersede.

## D1 — Deliverable shape: proper README per layer, main README stays general

- **Date:** 2026-10-12
- **Decision:** Each layer gets its own proper README/spec. The main `os/README.md` remains (or becomes) user-facing, consumer-focused, and general. The deep architecture content currently living in `os/README.md` (it was placed there only because the repo was empty) will be moved into the per-layer specs as they are created.
- **Rejected:** (a) expand the main README itself into one giant buildable document — would get unwieldy and mixes audiences; (b) implement code immediately per layer before specs — decisions would evaporate between sessions.
- **Why:** per-layer files match the project's own Knowledge Layer conventions (one topic per file, small files, AI-readable) and make sessions disposable.

## D2 — Build harness and model roster

- **Date:** 2026-10-12
- **Decision:** Claude Code is the harness for building this project (not open code — open code is a *candidate component* decision inside the OS itself, e.g. Layer 3). Orchestration via Claude Code workflows (ultracode). Model assignments: k3[1m] main session; k3-256k research fleet; kimi-for-coding (K2.8) cheap bulk/background; fable/opus adversarial review; sonnet drafting; haiku tiny tasks.
- **Rejected:** running all agents on the main-session model — wastes the 1M window on scoped tasks and spends more than needed.
- **Why:** right model per job; the three Kimi tiers (K2.8, k3-1M, k3-256k) cover main/fleet/background roles.

## D3 — Session strategy: hybrid

- **Date:** 2026-10-12
- **Decision:** The research → options → decision loop stays in one continuous session (cross-layer consistency lives here). Builds of decided layers fork into fresh ultracode sessions when substantial, with the layer spec + this file as the handoff brief. Plan state lives on disk in `os/plan/`, never only in chat.
- **Rejected:** (a) everything in one session — context rot loses rejected-option rationale across 10 layers; (b) fully separate sessions per layer — re-reading everything each session drifts cross-layer decisions.
- **Why:** md files make session boundaries cheap; fork builds, not thinking.

## D4 — First layer: Layer 0 (Knowledge)

- **Date:** 2026-10-12
- **Decision:** Start with Layer 0, the Knowledge Layer.
- **Rejected:** Layer 3 first (highest leverage, but ungrounded before the KB exists); Layer 1 first (forces device decisions before research habits exist).
- **Why:** README designates it the foundation every other layer reads from; building it first establishes the format + research workflow every later layer reuses.

## D5 — Build execution protocol + commit discipline

- **Date:** 2026-10-12
- **Decision:** Every layer build follows `EXECUTION.md`: small independent tasks with acceptance criteria + verifiable checks, parallel subagents on disjoint file sets, self-review (run checks, read diffs) with bounce-back fixes, one commit per verified task, done = criteria met + full suite green + built/deferred summary. Commits carry **no** Claude co-author or attribution lines, ever.
- **Why:** save work at every step; review loop catches subagent drift; user wants no attribution noise in git history.

## D6 — Layer 0 (Knowledge) design decisions

- **Date:** 2026-10-12
- **Decision:**
  - **(A) Index contract:** hierarchical llms.txt v2 — root `os/knowledge/llms.txt` + one per category folder, CI-generated from entry frontmatter (never hand-written), proven-only entries in main sections, candidate/testing in the v2 `Optional` section, NO llms-full.txt. Plus an AGENTS.md pair (repo root + `os/knowledge/AGENTS.md`) teaching the retrieval ritual. No CLAUDE.md inside `knowledge/`.
  - **(D) Rubric:** three-gate candidate→testing→proven. External signals (activity, Scorecard, adopters, distro packaging) can at most justify `testing`; `proven` requires first-party field evidence logged in the entry (≥3 months on ≥2 device profiles, zero-WAN clean install, power-loss recovery). Quarterly CI re-checks; demotion + hold ring. Consequence accepted: seed entries enter as `candidate`; `proven` is earned in the field.
  - **(E) Publish:** DEFERRED — will most probably be part of the Next.js app in `website/`, not Quartz/Starlight. Binding constraint recorded instead: the KB format stays publisher-agnostic (vault-wide-unique slugs, wikilinks resolvable by name, CommonMark body, YAML frontmatter, regenerable indexes) so any pipeline can render it later.
  - **(F) v1 scope:** skeleton — format machinery (schema, validator/generator, AGENTS.md, README spec) + ~8 flagship entries (meshtastic, reticulum, nostr, llama-cpp, qwen3-4b, opencode, automerge, heltec-wifi-lora-32-v3). Full ~35-entry seed deferred.
- **Rejected:** AGENTS.md-only/flat index; external-signals-suffice proven; locking Quartz/Starlight now; full seed; building the site pipeline this cycle.
- **Why:** build correctly and compatible; the publish tool must not constrain the format.

## D7 — Layer 0 spec locked; build amendments

- **Date:** 2026-10-12
- **Decision:** Layer 0 spec v2 (`os/knowledge/README.md`) locked after the adversarial panel (4 judges, 62 findings → 42 accepted fixes; triage in `layer-0-review.md`). Amendments adopted with the lock: (a) seed set 8 → 9, adding `recipes/install-meshtastic-on-heltec-wifi-lora-32-v3.md` so the recipe machinery ships exercised; (b) validator uses pinned PyYAML instead of stdlib-only parsing (hand-rolled YAML would get the 1.1 traps wrong; PyYAML is the boring, system-packaged choice), with a strict linted YAML authoring subset on top; (c) seeds are real researched content whose purpose is exercising the machinery — all enter as `candidate`.
- **Rejected:** hand-rolled stdlib YAML parser; entry_type frontmatter field (derivable); reopening skeleton scope.
- **Why:** the rubric's "proven = field-proven" promise is only real if the linter can check its evidence structurally; the skeleton's job is proving the machinery end-to-end on real data.

## D8 — The vision doc names no tools

- **Date:** 2026-10-12
- **Decision:** `os/README.md` states goals, intentions, and ideas only — no specific tool or project names. Removed: Nostr (5 mentions), llms.txt, GitHub Actions, Wikipedia. Generic technology *classes* (LoRa, Wi-Fi, BLE, Markdown, YAML) stay. Tool/project adoption lives in Knowledge Layer entries and this decisions log, earned through the research→decision cycle.
- **Why:** the founding document stays stable while implementation choices evolve; nothing gets prematurely fixed by being named in it.

## D9 — Layer 1 (Bootstrap) design decisions

- **Date:** 2026-10-12
- **Decision:** Scope locked with user: targets = Linux laptops/servers/RPi (x86_64 + aarch64, systemd-only) + Android phones provisioned over USB-C from a Linux computer running the same command; end state = running minimal node; offline prefetch = minimal with post-install offer of a standard tier (small model). From the options brief (`layer-1-options.md`; 7-scout sweep + completeness critic, 24 gaps folded):
  - **A1** two-stage installer: version-pinned POSIX shim on own domain (function-wrapped, artifact sha256 pinned inside, downloads+verifies as user then re-execs via sudo) → ONE musl-static Rust binary from GitHub Releases + own-domain mirror; `receipt.json` + on-disk `111-uninstall.sh`; zero telemetry.
  - **B** implementation: Rust static binary (spec detail, not polled).
  - **C2** minimal node = `111d` kernel (secp256k1 identity, tokio supervision tree, embedded minimal dashboard + `/health`, minisign self-updater with A/B + health-gated auto-rollback, KB fetcher) + KB tarball snapshot + conditional `rnsd` child when python3 exists (meshtasticd/radios detection-only). User amendment: v1 dashboard stays truly minimal — "the very minimums live is enough"; real dashboard work is Layer 4's.
  - **D1** identity: provisional secp256k1 (k256, NCC-audited), BIP-39 mnemonic born silently at install (NIP-06 path), shown only via post-install `111 identity backup`. Provisional: Layer 2 owns final crypto on merit (its scope is explicitly NOT Nostr-locked; see also D8 — README no longer names Nostr); a one-time re-key on Layer 2 landing is accepted.
  - **E1** phone path: same command drives adb — pinned GitHub debuggable Termux APKs (single signature source) + `run-as com.termux` headless channel + Termux:Boot/runit supervision + adb hardening; aarch64 phones only; debuggable-build trust model accepted with post-install revocation of USB-debugging authorizations; postmarketOS stays an experimental opt-in tier.
  - Detail-level (spec-reviewed, not polled — full text in the brief): FHS layout `/opt/111` + `/var/lib/111` + `/etc/111`; rustup-v2 TOML channel manifest + minisign with pre-designed key-rotation chain; systemd system unit (user `node111`, ambient caps, `dialout`, watchdog, journald cap); category rule set with proposed default thresholds; pre-build verification matrix V1–V8.
- **Rejected:** package-repo installer; TLS-only shim; bundle C1 (fakes "mesh joined") and C3 (dead weight, non-portable); raw-key-only identity; blocking Layer 1 on Layer 2's crypto; release-APK `input text` injection (kept as fallback); own provisioner APK (revisit v1.1); postmarketOS as default; armv7 phones; non-systemd distros (clean refusal); proot-distro; GPG signing; installer telemetry.
- **Why:** strongest verified installer anatomy (uv + Determinate + k3s patterns); smallest node that honestly satisfies "running minimal node"; identity seam cheap to swap; run-as is the only verified fully-headless phone channel.

## D10 — Layer 3 (Local Intelligence) design decisions

- **Date:** 2026-10-12
- **Decision:** Scope locked with user: **no new research** — every adopted component comes from existing KB entries (`llama-cpp`, `qwen3-4b`, `opencode`), whose own use-when/do-not-use-when sections dictated the split. Layer boundary: **Layer 3 = capability** (models, serving, harness); **Layer 9 = behavior** (monitor cadence, diagnosis playbooks, sandbox, PR gate) — Layer 9 consumes Layer 3 through one task API and stays harness-agnostic.
  - **H1** hybrid harness: thin always-on Rust agent loop (`111-agent`, supervised child of `111d`, same pattern as the `rnsd` child) for chat, KB grounding, monitoring, and the task API; opencode adopted on hub tiers (4A/4B, opt-in on 2B) for patch-class tasks via headless JSONL — invoked **by** `111-agent`, never directly by Layer 9.
  - **S1** serving: pinned `llama-server` binary as supervised child of `111d`; OpenAI-compatible endpoint `127.0.0.1:11181`; GGUF blobs sha256-pinned in the signed channel manifest, fetched via Layer 1's standard-tier prefetch; no runtime model downloads.
  - **M1** model default: qwen3-4b Q4_K_M is the standard tier wherever a local model runs (2B, 4A, 4B); the 4B hub ladder (8B → 32B / 30B-A3B MoE) is one config table, not a new adoption. Offload-only nodes (2A, 3) run `111-agent` as a pure client of a nearby hub.
  - Ports: dashboard 11180 (Layer 1), llama-server 11181, `111-agent` 11182 — localhost; LAN binding follows the bootstrap dashboard's explicit opt-in rule.
- **Rejected:** opencode everywhere (a 24/7 Node/TypeScript runtime on every intelligence-capable node; the KB entry itself warns it "belongs on hubs, not relays"); custom-only thin loop (reinvents an agent harness — against adopt-don't-reinvent — and caps patch-generation quality); Ollama (a second heavy always-on daemon whose supervision/update duties overlap `111d`'s; would require new adoption research); llama.cpp embedded as a library in `111d` (couples the kernel's release cycle to the inference engine and adds custom serving plumbing); on-node fine-tuning (weights change only via signed channel-manifest swaps); WAN/cloud model calls (breaks offline-first).
- **Why:** the smallest layer that honestly satisfies the README's "continuous agent" without a resident Node daemon; the harness-agnostic task seam keeps Layer 9 unblocked; every component traces to a KB entry, so the adopt-vs-build rule is exercised, not bypassed.

## D11 — Layer 4 (Interface & Dashboard) design decisions

- **Date:** 2026-10-12
- **Decision:** From the options brief (`layer-4-options.md`; 3-scout fast sweep), all four polled options accepted as recommended:
  - **Stack:** React + Vite static bundle at `os/interface/ui/`, later embedded into 111d via `rust-embed` (`debug-embed` for dev). Tailwind CSS 4 via `@tailwindcss/vite`, DESIGN.md tokens in `@theme` ported from `website/src/app/globals.css` (the LIVE file; `src/styles/globals.css` is a stale shadcn decoy). Fonts self-hosted woff2 + hand-written `@font-face` (Geist Mono variable, Geist Pixel Line extracted from the `geist` package, Silkscreen 400/700 — all SIL OFL). **No shadcn** (rounded primitives banned), **no framer-motion** (CSS keyframes + IntersectionObserver), **no state library** (context + hooks), wouter or hash routing. `build.target: es2017`; ~200 KB gz JS+CSS bundle budget checked in CI script; preact/compat alias held in reserve for Cat 2A.
  - **Mode model:** one component tree, `ModeProvider` flag `'simple' | 'advanced'` (default simple). Segmented `[SIMPLE|ADVANCED]` toggle in the header (DESIGN.md interactive-toggle primitive) + persistent `ADV` badge; client-side only: localStorage `111.ui.mode` + `?mode=` URL override (URL > storage > default), never a node write. Mode changes visibility/density ONLY — `minMode` flags on nav items and panel sections; advanced **appends**, simple cards never move. Every actionable item (agent approvals, restart, identity backup, updates, real errors) visible in BOTH modes. Simple nav capped at 5: Chat, Node, Mesh, Inbox, Settings. Advanced appends: Peers, Transports, Bridges, Gateway, Logs, Agent, Identity, Storage, Config. Plain language in simple view, but `#ff3333` stays reserved for real errors in both modes. Action-registry seam kept for the v2 Cmd-K palette.
  - **Contract:** REST + ONE multiplexed SSE stream (no WebSocket) under `/api/v1/`: `POST /chat`, `GET /chat/history`, `GET /status/node`, `GET /status/mesh`, `GET /notifications`, `POST /settings`, `GET /events` (named events `agent.token`, `agent.message`, `notify`, `status.node`, `status.mesh`; monotonic ids + `Last-Event-ID` ring-buffer resume). Versioning by path prefix, additive-only. Interim source of truth: `os/interface/api/openapi.yaml` → TS types via `openapi-typescript` at build; when 111d lands, Rust DTOs + `utoipa` take over and CI drift-checks. RFC-9457-style `problem+json` errors; CORS closed (same-origin in prod).
  - **Scope (user, 2026-10-12):** "build a lot of UI — as much as you can plan and execute clean; bases to be wired to, so when other layers are ready we start wiring." So: full view/panel inventory (simple + advanced) on a typed mock adapter with simulated SSE, primitives, shell, mode system, action registry, budget check. NOT this cycle: 111d embedding, a real API server, e2e.
- **Rejected:** Next.js static export (heaviest runtime for offline old-phone target, imports the moving Next 16 target); HTMX + askama (double UI surface next to the agent's JSON API; kept as fallback if Node in os/ CI ever blocks); vanilla JS/WebComponents (bespoke mini-framework, philosophy violation); two separate simple/advanced apps (GL.iNet/LuCI drift trap); WebSocket (no client→server streaming need); types-only contract (too weak for sibling layers coding against it).
- **Why:** only React gives the typed component model the "component-scalable" requirement needs while porting DESIGN.md exactly; one JSON contract keeps the agent's function surface single-source; one component tree with density flags kills the two-UIs maintenance trap; SSE is the minimal live channel that survives hub topology and old phones.

## D12 — Layer 1 spec locked (v2.1); lock amendments

- **Date:** 2026-10-12
- **Decision:** Layer 1 spec locked at v2.1 with three user amendments over the reviewed v2 draft: (a) **Privileged root update helper ships in v1** (panel-deferred, user promoted): the daemon only fetches/verifies/stages signed updates into `state/staging/`; a root oneshot (`111 update-helper`, path-triggered) re-verifies, promotes to `releases/`, flips `current`, and writes the probation journal + monotonic floor (both root-owned); a root `update-guard` ExecStartPre reverts expired probation before every start. User rationale: the daemon can never touch its own payload — a failed or hostile update always leaves the running version intact. (b) **Calendar versioning** `YYYY.MM.DD[.N]` for releases, shim URLs, and `min-upgrade-from` — no semver in v1. (c) Phone multicast limitation accepted as honest v1 scope (phones isolated-capable; LAN-join acceptance is Linux-only) with the full fix (MulticastLock helper APK) recorded as designed and planned for v1.1/Layer 6.
- **Rejected:** keeping `/opt/111` with a weakened sandbox (ReadWritePaths + chown); daemon-side symlink flips; deferring the privileged helper to v1.1; building the MulticastLock APK in v1.
- **Why:** strongest integrity boundary for unattended offline nodes; calver matches the project's dated-everything conventions and is trivially monotonic.

## D13 — Layer 2 (Identity) design decisions

- **Date:** 2026-10-12
- **Decision:** From the options brief (`layer-2-options.md`; 8-agent sweep `wf_73d7f290-f74`, ~927k tokens, 365 tool calls, cross-checked; scope locked w/ user 2026-10-12: node identity only, backup+recovery in scope, rotation/revocation deferred, crypto on merit, agentic-native):
  - **A1** dual keyset: **Ed25519 (signing) + X25519 (ECDH)**, Reticulum-shaped, from one 32-byte seed. Implementations per category, all RFC 8032/X25519-interoperable (mixed fleet cross-verifies): **Monocypher 4.x on Cat 1** (Cure53-audited 3.1.1 + formal-methods CI; CC0/BSD, ~10KB flash, no heap) · **ed25519-dalek v2** (+curve25519-dalek ≥4.1.3) in Rust for the `111d` world · libsodium ≥1.0.21 acceptable Cat 2/4. Pin + vendor hash-verified releases.
  - **B2 (user pick over recommended B1)** NodeId = **raw 32-byte Ed25519 pubkey** (Iroh/MeshCore style) — simplest, Iroh-proven at 200M+ endpoints; accepts 32B address fields and a noted PQ-migration corner (deferred list). Hard rule: derived/truncated display labels are never authenticators.
  - **C1** wire contract: sign announces/proofs/control only — never routine traffic (64B sig + 32B key ≈ 40% of a LoRa payload). Traffic = per-packet **ephemeral X25519 + AEAD** (sealed_box shape; packet-level forward secrecy, zero handshake). Signed announces distribute full pubkeys, TOFU + immutable-once-pinned. Versioned envelope, **fail-closed crypto-mode field**, per-peer persistent 64-bit counters + 32-entry sliding window (no wall-clock). Online links upgrade to Noise IK (EDHOC on watch). Envelope reserves: `signer_pubkey`, optional delegation trailer, future ratchet space. Full envelope = Layer 5's; C1 locks the identity-side contract only.
  - **D1** backup/recovery: **BIP-39 mnemonic** (12 words default, 24 hubs) + **SLIP-0010 hardened-only Ed25519** derivation, pinned with test vectors (NOT BIP32-Ed25519/Cardano — incompatible) inside our **versioned, self-describing backup spec** (curve+path+encoding+version; Electrum lesson). Single-key derivation; BIP-85 subtree reserved. Words + single-frame CompactSeedQR shown at commissioning **on the commissioning device** (Cat 1 relays never touch mnemonics); verify-on-write-down; no passphrase in v1. D9's mnemonic survives the derivation switch (same words, new key; NodeId changes once — pre-accepted). Phase-2 (spec now, build later): **recipient-mode encrypted export** (encrypt TO node's own pubkey — no scrypt; blob signed by node key, verify-before-decrypt; hub-storable) + **SSKR guardian split** (ROS-2021-audited C, BIP-39 round-trip).
  - **E1** agentic hook (built now, delegation deferred): NodeId binds the **root** pubkey; envelopes/handshakes carry explicit `signer_pubkey` + versioned optional delegation-proof trailer (v1 always empty); verifier rule fixed: valid iff signer==root(N) OR signer==agent + single-hop root-signed fixed-field binary cert `{version, agent_pubkey, scope, expiry-or-mesh-epoch, serial, root_signature}`, attenuation-only, no sub-delegation. Layer 3 adopts signer-daemon + default-deny policy engine pattern (Amber model).
  - **F1** transport anchoring: identity stays transport-independent; any global-transport credential (nostr npub or future) = separate per-node key bound by **bidirectional cross-signed attestation** (gossiped in-mesh, published when gatewayed). Attestation machinery = the deferred rotation machinery. Never: npub as root, NIP-05, NIP-26, shared seed material. L2 defines the attestation format only; Layer 7 decides transports.
  - **G1** audit/proven policy: prior-major-version audit + formal-methods CI **counts** for adopted primitives (Monocypher enters `testing`, `proven` via field evidence per rubric); FIPS 140-3 CMVP counts as third-party validation (wolfCrypt); we **commission/fund an audit of our identity module** (envelope + backup construction + glue) as the honest path to `proven`; until then our own module reports `testing`.
  - **H1** v1 build scope: `os/identity/README.md` spec + KB entries (adopted components + case studies, enter `candidate`; Monocypher fast-track to `testing`) + **`111id` Rust core** (ed25519-dalek: keygen/sign/verify/ECDH/announce/backup/restore/attestation) consumed by Layer 1's `111d` + **cross-implementation test vectors** (RFC 8032 + Rust↔Monocypher-C agreement). C/Monocypher Cat-1 core deferred-but-pinned until a layer owns ESP32 firmware.
- **Rejected:** A2 secp256k1-native (no audited Cat-1 path → rubric caps at `testing`; contradicts D8 "not Nostr-locked") · A3 single-key birational conversion (couples signing+encryption compromise) · B1 truncated-hash NodeId (user chose raw) · C2 static-ECDH-minimal (zero FS; harvest-now-decrypt-later is the default exposure on buffered mesh) · C3 sign-everything (frame budget) · C4 Signal-grade ratchets v1 (server-mediated, state-heavy; hooks reserved) · D2 NIP-06 path (upstream-deprecated, only coherent with A2) · D3 raw-key-only (no paper restore) · BIP-39 passphrase v1 (silent-wrong-wallet trap) · SLIP-39 guardian split (no audit → `testing` cap; SSKR chosen) · E2 agent-uses-root (hot root key) · E3 UCAN/Biscuit now (unaudited, moving specs; track + align scope vocabulary) · F2 seed-derived transport keys (compromise cascade) · F3 transport-native identity · G2 strict exact-version audits (proven gate shut too long) · G3 audit-before-build (procurement blocks the layer) · H2 spec-only (test vectors are the point) · H3 dual-impl now (no Cat-1 firmware owner yet) · Reticulum CODE adoption (non-OSI "Reticulum License", AI-training ban — radioactive for an agentic OS; protocol is public-domain design reference only).
- **Why:** the audit gate + Cat-1 feasibility are decisive and both point at Ed25519+X25519/Monocypher; B2 trades 16B/packet for zero hash machinery and deployed-at-scale proof (user's call); G1 keeps `proven` reachable without blocking the layer; E1's three spec lines keep agentic delegation a future soft upgrade instead of a fleet re-key.

## Pending

- Move deep architecture content out of `os/README.md` into per-layer specs (blocked on layer homes existing).
