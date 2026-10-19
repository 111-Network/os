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
  - **(A) Index contract:** hierarchical llms.txt v2 — root `os/00-knowledge/llms.txt` + one per category folder, CI-generated from entry frontmatter (never hand-written), proven-only entries in main sections, candidate/testing in the v2 `Optional` section, NO llms-full.txt. Plus an AGENTS.md pair (repo root + `os/00-knowledge/AGENTS.md`) teaching the retrieval ritual. No CLAUDE.md inside `00-knowledge/`.
  - **(D) Rubric:** three-gate candidate→testing→proven. External signals (activity, Scorecard, adopters, distro packaging) can at most justify `testing`; `proven` requires first-party field evidence logged in the entry (≥3 months on ≥2 device profiles, zero-WAN clean install, power-loss recovery). Quarterly CI re-checks; demotion + hold ring. Consequence accepted: seed entries enter as `candidate`; `proven` is earned in the field.
  - **(E) Publish:** DEFERRED — will most probably be part of the Next.js app in `website/`, not Quartz/Starlight. Binding constraint recorded instead: the KB format stays publisher-agnostic (vault-wide-unique slugs, wikilinks resolvable by name, CommonMark body, YAML frontmatter, regenerable indexes) so any pipeline can render it later.
  - **(F) v1 scope:** skeleton — format machinery (schema, validator/generator, AGENTS.md, README spec) + ~8 flagship entries (meshtastic, reticulum, nostr, llama-cpp, qwen3-4b, opencode, automerge, heltec-wifi-lora-32-v3). Full ~35-entry seed deferred.
- **Rejected:** AGENTS.md-only/flat index; external-signals-suffice proven; locking Quartz/Starlight now; full seed; building the site pipeline this cycle.
- **Why:** build correctly and compatible; the publish tool must not constrain the format.

## D7 — Layer 0 spec locked; build amendments

- **Date:** 2026-10-12
- **Decision:** Layer 0 spec v2 (`os/00-knowledge/README.md`) locked after the adversarial panel (4 judges, 62 findings → 42 accepted fixes; triage in `layers/00-knowledge/review/triage.md`). Amendments adopted with the lock: (a) seed set 8 → 9, adding `recipes/install-meshtastic-on-heltec-wifi-lora-32-v3.md` so the recipe machinery ships exercised; (b) validator uses pinned PyYAML instead of stdlib-only parsing (hand-rolled YAML would get the 1.1 traps wrong; PyYAML is the boring, system-packaged choice), with a strict linted YAML authoring subset on top; (c) seeds are real researched content whose purpose is exercising the machinery — all enter as `candidate`.
- **Rejected:** hand-rolled stdlib YAML parser; entry_type frontmatter field (derivable); reopening skeleton scope.
- **Why:** the rubric's "proven = field-proven" promise is only real if the linter can check its evidence structurally; the skeleton's job is proving the machinery end-to-end on real data.

## D8 — The vision doc names no tools

- **Date:** 2026-10-12
- **Decision:** `os/README.md` states goals, intentions, and ideas only — no specific tool or project names. Removed: Nostr (5 mentions), llms.txt, GitHub Actions, Wikipedia. Generic technology *classes* (LoRa, Wi-Fi, BLE, Markdown, YAML) stay. Tool/project adoption lives in Knowledge Layer entries and this decisions log, earned through the research→decision cycle.
- **Why:** the founding document stays stable while implementation choices evolve; nothing gets prematurely fixed by being named in it.

## D9 — Layer 1 (Bootstrap) design decisions

- **Date:** 2026-10-12
- **Decision:** Scope locked with user: targets = Linux laptops/servers/RPi (x86_64 + aarch64, systemd-only) + Android phones provisioned over USB-C from a Linux computer running the same command; end state = running minimal node; offline prefetch = minimal with post-install offer of a standard tier (small model). From the options brief (`layers/10-bootstrap/options.md`; 7-scout sweep + completeness critic, 24 gaps folded):
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
- **Decision:** From the options brief (`layers/40-interface/options.md`; 3-scout fast sweep), all four polled options accepted as recommended:
  - **Stack:** React + Vite static bundle at `os/40-interface/ui/`, later embedded into 111d via `rust-embed` (`debug-embed` for dev). Tailwind CSS 4 via `@tailwindcss/vite`, DESIGN.md tokens in `@theme` ported from `website/src/app/globals.css` (the LIVE file; `src/styles/globals.css` is a stale shadcn decoy). Fonts self-hosted woff2 + hand-written `@font-face` (Geist Mono variable, Geist Pixel Line extracted from the `geist` package, Silkscreen 400/700 — all SIL OFL). **No shadcn** (rounded primitives banned), **no framer-motion** (CSS keyframes + IntersectionObserver), **no state library** (context + hooks), wouter or hash routing. `build.target: es2017`; ~200 KB gz JS+CSS bundle budget checked in CI script; preact/compat alias held in reserve for Cat 2A.
  - **Mode model:** one component tree, `ModeProvider` flag `'simple' | 'advanced'` (default simple). Segmented `[SIMPLE|ADVANCED]` toggle in the header (DESIGN.md interactive-toggle primitive) + persistent `ADV` badge; client-side only: localStorage `111.ui.mode` + `?mode=` URL override (URL > storage > default), never a node write. Mode changes visibility/density ONLY — `minMode` flags on nav items and panel sections; advanced **appends**, simple cards never move. Every actionable item (agent approvals, restart, identity backup, updates, real errors) visible in BOTH modes. Simple nav capped at 5: Chat, Node, Mesh, Inbox, Settings. Advanced appends: Peers, Transports, Bridges, Gateway, Logs, Agent, Identity, Storage, Config. Plain language in simple view, but `#ff3333` stays reserved for real errors in both modes. Action-registry seam kept for the v2 Cmd-K palette.
  - **Contract:** REST + ONE multiplexed SSE stream (no WebSocket) under `/api/v1/`: `POST /chat`, `GET /chat/history`, `GET /status/node`, `GET /status/mesh`, `GET /notifications`, `POST /settings`, `GET /events` (named events `agent.token`, `agent.message`, `notify`, `status.node`, `status.mesh`; monotonic ids + `Last-Event-ID` ring-buffer resume). Versioning by path prefix, additive-only. Interim source of truth: `os/40-interface/api/openapi.yaml` → TS types via `openapi-typescript` at build; when 111d lands, Rust DTOs + `utoipa` take over and CI drift-checks. RFC-9457-style `problem+json` errors; CORS closed (same-origin in prod).
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
- **Decision:** From the options brief (`layers/20-identity/options.md`; 8-agent sweep `wf_73d7f290-f74`, ~927k tokens, 365 tool calls, cross-checked; scope locked w/ user 2026-10-12: node identity only, backup+recovery in scope, rotation/revocation deferred, crypto on merit, agentic-native):
  - **A1** dual keyset: **Ed25519 (signing) + X25519 (ECDH)**, Reticulum-shaped, from one 32-byte seed. Implementations per category, all RFC 8032/X25519-interoperable (mixed fleet cross-verifies): **Monocypher 4.x on Cat 1** (Cure53-audited 3.1.1 + formal-methods CI; CC0/BSD, ~10KB flash, no heap) · **ed25519-dalek v2** (+curve25519-dalek ≥4.1.3) in Rust for the `111d` world · libsodium ≥1.0.21 acceptable Cat 2/4. Pin + vendor hash-verified releases.
  - **B2 (user pick over recommended B1)** NodeId = **raw 32-byte Ed25519 pubkey** (Iroh/MeshCore style) — simplest, Iroh-proven at 200M+ endpoints; accepts 32B address fields and a noted PQ-migration corner (deferred list). Hard rule: derived/truncated display labels are never authenticators.
  - **C1** wire contract: sign announces/proofs/control only — never routine traffic (64B sig + 32B key ≈ 40% of a LoRa payload). Traffic = per-packet **ephemeral X25519 + AEAD** (sealed_box shape; packet-level forward secrecy, zero handshake). Signed announces distribute full pubkeys, TOFU + immutable-once-pinned. Versioned envelope, **fail-closed crypto-mode field**, per-peer persistent 64-bit counters + 32-entry sliding window (no wall-clock). Online links upgrade to Noise IK (EDHOC on watch). Envelope reserves: `signer_pubkey`, optional delegation trailer, future ratchet space. Full envelope = Layer 5's; C1 locks the identity-side contract only.
  - **D1** backup/recovery: **BIP-39 mnemonic** (12 words default, 24 hubs) + **SLIP-0010 hardened-only Ed25519** derivation, pinned with test vectors (NOT BIP32-Ed25519/Cardano — incompatible) inside our **versioned, self-describing backup spec** (curve+path+encoding+version; Electrum lesson). Single-key derivation; BIP-85 subtree reserved. Words + single-frame CompactSeedQR shown at commissioning **on the commissioning device** (Cat 1 relays never touch mnemonics); verify-on-write-down; no passphrase in v1. D9's mnemonic survives the derivation switch (same words, new key; NodeId changes once — pre-accepted). Phase-2 (spec now, build later): **recipient-mode encrypted export** (encrypt TO node's own pubkey — no scrypt; blob signed by node key, verify-before-decrypt; hub-storable) + **SSKR guardian split** (ROS-2021-audited C, BIP-39 round-trip).
  - **E1** agentic hook (built now, delegation deferred): NodeId binds the **root** pubkey; envelopes/handshakes carry explicit `signer_pubkey` + versioned optional delegation-proof trailer (v1 always empty); verifier rule fixed: valid iff signer==root(N) OR signer==agent + single-hop root-signed fixed-field binary cert `{version, agent_pubkey, scope, expiry-or-mesh-epoch, serial, root_signature}`, attenuation-only, no sub-delegation. Layer 3 adopts signer-daemon + default-deny policy engine pattern (Amber model).
  - **F1** transport anchoring: identity stays transport-independent; any global-transport credential (nostr npub or future) = separate per-node key bound by **bidirectional cross-signed attestation** (gossiped in-mesh, published when gatewayed). Attestation machinery = the deferred rotation machinery. Never: npub as root, NIP-05, NIP-26, shared seed material. L2 defines the attestation format only; Layer 7 decides transports.
  - **G1** audit/proven policy: prior-major-version audit + formal-methods CI **counts** for adopted primitives (Monocypher enters `testing`, `proven` via field evidence per rubric); FIPS 140-3 CMVP counts as third-party validation (wolfCrypt); we **commission/fund an audit of our identity module** (envelope + backup construction + glue) as the honest path to `proven`; until then our own module reports `testing`.
  - **H1** v1 build scope: `os/20-identity/README.md` spec + KB entries (adopted components + case studies, enter `candidate`; Monocypher fast-track to `testing`) + **`111id` Rust core** (ed25519-dalek: keygen/sign/verify/ECDH/announce/backup/restore/attestation) consumed by Layer 1's `111d` + **cross-implementation test vectors** (RFC 8032 + Rust↔Monocypher-C agreement). C/Monocypher Cat-1 core deferred-but-pinned until a layer owns ESP32 firmware.
- **Rejected:** A2 secp256k1-native (no audited Cat-1 path → rubric caps at `testing`; contradicts D8 "not Nostr-locked") · A3 single-key birational conversion (couples signing+encryption compromise) · B1 truncated-hash NodeId (user chose raw) · C2 static-ECDH-minimal (zero FS; harvest-now-decrypt-later is the default exposure on buffered mesh) · C3 sign-everything (frame budget) · C4 Signal-grade ratchets v1 (server-mediated, state-heavy; hooks reserved) · D2 NIP-06 path (upstream-deprecated, only coherent with A2) · D3 raw-key-only (no paper restore) · BIP-39 passphrase v1 (silent-wrong-wallet trap) · SLIP-39 guardian split (no audit → `testing` cap; SSKR chosen) · E2 agent-uses-root (hot root key) · E3 UCAN/Biscuit now (unaudited, moving specs; track + align scope vocabulary) · F2 seed-derived transport keys (compromise cascade) · F3 transport-native identity · G2 strict exact-version audits (proven gate shut too long) · G3 audit-before-build (procurement blocks the layer) · H2 spec-only (test vectors are the point) · H3 dual-impl now (no Cat-1 firmware owner yet) · Reticulum CODE adoption (non-OSI "Reticulum License", AI-training ban — radioactive for an agentic OS; protocol is public-domain design reference only).
- **Why:** the audit gate + Cat-1 feasibility are decisive and both point at Ed25519+X25519/Monocypher; B2 trades 16B/packet for zero hash machinery and deployed-at-scale proof (user's call); G1 keeps `proven` reachable without blocking the layer; E1's three spec lines keep agentic delegation a future soft upgrade instead of a fleet re-key.

## D14 — Layer 2 spec locked (v2); lock amendments

- **Date:** 2026-10-12
- **Decision:** Layer 2 spec locked at v2 (`os/20-identity/README.md`) after the adversarial panel (4 fable judges: crypto / completeness / embedded / seams — 71 raw findings, 4 verdicts of not-lock-ready, all gating items fixed in place; triage: `layers/20-identity/review/triage.md`). Amendments adopted with the lock:
  - **(a) KB rubric reconciliation (user call):** the flat bus-factor ≥2 gate for `testing` gains a **vendored-primitive exception** — permitted when the component is permissive/CC0-licensed, vendored hash-pinned, and forkability is recorded (Monocypher is the case). The spec's §14 agreement-vector harness run doubles as the rubric's first-party smoke-test log. `proven` is unchanged: field evidence only.
  - **(b) Commissioning ownership (user call):** the Cat-1 commissioning ceremony is owned by the **operator's computer running the `111` CLI** (`111 identity commission`, in 111id scope). Channel minimums pinned in the spec (point-to-point local physical, never routed/broadcast; challenge-verify; commissioner wipes); byte-level provisioning protocol spec'd when Cat-1 firmware lands.
  - **(c) Wire AEAD:** single fleet-wide XChaCha20-Poly1305 confirmed (the drafter's narrowing of a research-open option; software cost trivial at LoRa rates; minimal agility surface). The user answered the poll by redirecting to build momentum — recorded as accepted-recommended; reversible via the crypto-mode registry if ever revisited.
  - **(d) Layer 1 amendment (v2.1 → v2.2):** the provisional secp256k1 identity seam is replaced by this layer's contract — `identity/` = mnemonic/keyset/nodeid/descriptor/counters; verbs {backup, restore, commission}; `recover` subsumed by `restore`, `export` (NIP-49) deleted; `bip32.rs`/`nip49.rs` deleted, `identity/mod.rs` becomes the `111id` binding with error-class/quarantine semantics preserved; npub → NodeId in install print/dashboard/`/api/status`. No signed release has shipped, so this is a code+spec amendment, not a fleet event; D9-born mnemonics migrate per the spec's §8.4 path.
- **Rejected:** none outright in triage; two sub-suggestions partially deferred (full commissioning protocol skeleton; cert scope-bit semantics at activation).
- **Why:** the panel's blockers were all normative-pinning gaps (sealed-packet recipe, counter-state-in-restore, announce-counter commit rule), not architectural flaws — the architecture got zero-defect verdicts; locking now keeps Layer 1's freshly-built code aligned before the provisional seam hardens.

## D15 — Layer 4 spec v2: IA redirect, auth now, hostnames, status colors, sibling amendments

- **Date:** 2026-10-13
- **Decision:** After the v1 adversarial panel (3 lenses, 41 findings; triage in `layers/40-interface/review/triage.md`) and the owner's IA redirect, Layer 4 locks spec v2 (`os/40-interface/README.md`):
  - **IA:** core = **Overview** + **Agent** (chat box; default route). Collapsible left sidebar: Overview · Agent · Messages (structural stub for L5) · services with 3-state dots · (advanced) Logs · bottom collapsible **Settings** → Essentials + one page per layer L0–L9 (plain title first, "Layer N" second; structure only). Service page: LIVE → native control panel; NOT SET → explanation + "Set up with the agent" (structured, editable, never auto-sent prompt); ERROR → "Fix with the agent". Status is computed by `111d` from deterministic probes with 2-tick debounce — never by the agent or the UI.
  - **Chat engine:** the chat UI is ours (thin); the brain is Layer 3's `111-agent` (already adopted; opencode stays hub-side behind it). No new adoption.
  - **Mode:** simple|advanced = density only; the sidebar's icon-rail collapse is the "agent-only" clean view; no kiosk mode. `?mode=` consumed once then stripped (fixes the sticky-param defect).
  - **Status colors (owner):** scoped DESIGN.md exception — `status-live #22c55e` / `status-unset #a0a0a0` / `status-error #ff3333` on status dots only, always with a text label. Committed to `website/docs/DESIGN.md` (c117f45).
  - **Hostnames (owner):** `http://111.localhost` + `http://127.0.0.1` on the node (`:80` best-effort via `CAP_NET_BIND_SERVICE`, `:11180` always); `http://111.local` via mDNS (`mdns-sd`) when LAN sharing is on. `.111` TLD deferred to hub/AP nodes running the LAN's DNS. **No TLS in v1.**
  - **Browser floor (owner):** keep Tailwind 4; floor Chrome 111+/Safari 16.4+/Firefox 128+ (≈ Android 7+ with updated WebView, iOS 16.4+); feature-detect notice + Layer 1 minimal page kept at `/v1` for older browsers; build target es2020.
  - **Auth (owner: "token auth now"):** loopback bypasses auth; non-loopback needs a session cookie obtained with an 8-char pair code (`111 dashboard` prints it; `111 dashboard rotate`); rate-limited; SameSite=Strict + JSON-only + Origin/Host check as CSRF defense; loopback-only operations (`lan_enabled`, `offer`) refuse paired LAN sessions. Cookie (not bearer header) because `EventSource` cannot send headers.
  - **Contract:** `/api/v1` (`os/40-interface/api/openapi.yaml`) = REST + one `EventSource` stream (`status`, `services`, `proposal`, `agent`, `reset`; global monotonic ids, ring 256, `Last-Event-ID` else `reset`) + chat replies streamed on the POST response (`token`/`tool`/`done`/`error`) — amends D11's "one SSE stream" wording: no WebSocket, live state on one stream, chat tokens on the request's own reply. Identity backup over HTTP removed (Layer 2 rule); restart + update-check POSTs added.
  - **Proposals:** `111d` reads `<root>/agent/proposals/*.json` and records decisions in `<root>/agent/decisions/` (`accepted|dismissed`); applying accepted proposals is Layer 9's job — the UI says so.
  - **Scope this cycle:** real wiring (UI ↔ `111d` ↔ `111-agent`), embed, auth, hostnames, install→open; structural stubs for L5–L9 surfaces.
- **Sibling amendments (Layer 4 owns these edits in Layer 1/3 trees):** (1) `111d`'s dashboard surface gains POST/PATCH routes + SSE under `/api/v1`; the "strictly GET/read-only" clause now scopes to `/api/status` + `/health`. (2) Host guard widened (`[::1]`, `111.localhost`, `111.local`, own addresses when LAN) — the as-built LAN opt-in refused every LAN Host. (3) The `111-agent` relay lands in `111d` (the "L4 relay = L4 build item" seam). (4) `/` serves the Layer 4 UI; v1 minimal page moves to `/v1`. (5) `/etc/111/env` + `agent.env` get `chown root:node111` (as-built defect: root-owned 0640 while the daemon runs as `node111`). (6) `status.update.last_check` and `clock_fault` are read from their state files (never populated as built). (7) New crates in `node111`: `mdns-sd`, `qrcode`; hyper-util client features for the relay. (8) `state/settings.json` (alias, lan_enabled, offer) read by `111d` alongside the env file.
- **Rejected:** iframe-embedding third-party web UIs (mixed content, framing headers, secure-context APIs — link out instead); a separate agent-only app/kiosk; `.111` custom TLD now (unresolvable without node-run DNS); self-signed TLS (warnings for non-technical users); bearer-token header auth (breaks `EventSource`); LAN read-only v1 (owner chose real auth); agent/UI computing service status (LLM outage must not paint everything red); fake "Approve = applied" semantics before Layer 9 exists.
- **Why:** the owner's IA keeps two taps between a non-technical user and "is my node OK / ask the agent"; everything that could drift (status, auth, contract) lives in the daemon as code; the pairing cookie is the smallest auth that works with SSE and old phones; wiring now (Layers 1–3 exist) replaces an entire mock cycle with real integration.

## D16 — Layer 4 locked (as built); I3 fix-wave amendments

- **Date:** 2026-10-14
- **Decision:** Layer 4 (Interface & Dashboard) is **built and locked** at spec v2 + the amendments below. I3 = three fable judges on the integrated system (security · UI/UX/a11y · install→dashboard integration, all with real runs: netns smoke + headless Chrome over CDP + raw mDNS). All three said fix-then-lock (security 5 major/13 minor; UI 1 critical/8 major/20 minor; integration 1 critical/6 major/9 minor). Every finding was fixed in a three-agent wave (two opus Rust fixers on disjoint files, one sonnet UI fixer), each task verified by the lead and committed separately; the remaining external gates are listed under "Open".
  - **(a) Loopback trust is a policy (lead call, reversible).** Android/Termux has no per-app loopback isolation, so under Termux (`TERMUX_VERSION` or `/data/data/com.termux/files/usr`) loopback peers need a session like LAN peers; the phone installer opens `#/pair?code=…` once. `DASHBOARD_TRUST_LOOPBACK=true|false` overrides. Desktop Linux unchanged. Reported in `GET /auth/session.loopback_trusted`.
  - **(b) LAN sharing has one source of truth:** `state/settings.json` `lan_enabled` (the CLI `111 config dashboard.lan` and the dashboard toggle write the same file; `DASHBOARD_LAN` in the env file pins it → `Settings.lan_locked`, PATCH `409 settings_locked`). Replaces the as-built "env OR settings" that could only turn on.
  - **(c) Per-service restart exists:** `POST /api/v1/services/{id}/restart` for 111d-supervised children (agent, model, transport while rnsd is a child) — SIGTERM, 5 s grace, SIGKILL, immediate respawn, back-off reset; `409 not_supervised` otherwise. `ChildHandle::stop_and_wait` replaces the fire-and-forget SIGTERM; the daemon joins its children before exit.
  - **(d) Phone one-liner runs unprivileged:** the shim's `--phone` branch uses privilege for the binary copy only and execs the installer as the invoking user; the two operator prompts read `/dev/tty` and fail closed (`--yes` pre-answers). All phone `run-as` shell commands export Termux `PREFIX`/`HOME`/`PATH` (Android's system shell sets none).
  - **(e) Pairing address = LAN IP.** QR and UI pair address encode `http://<lan-ip>/#/pair?code=…` (Android does not resolve `.local`); `111.local` listed as an alternative.
  - **(f) Hardening:** fchown-before-rename for state files; every `s111` cookie tried; bounded failure map; `/api/status` + `/v1` gated for non-loopback; CSP (`script-src 'self'`, inline feature-detect moved to `detect.js`), nosniff, no-referrer; problem+json for framework errors; Meshtastic probe is a LISTEN check (never a connect — meshtasticd accepts one client); proposal `kind` validated; `status.layers` means working, not installed; `[::1]` loopback bind; stale pid detection by start time; cross-build fails without a fresh UI dist (`--allow-no-ui`); install verify GETs `/`.
  - **(g) UI:** Agent route owns the scroll on phones with proposals inline; "steps" → "request" honesty copy; focus never drops to body while loading; live region re-announces identical text; countdown out of `role=alert`; one prompt builder with a `context` goal (no double-goal prompts, no hard-coded service starters); SSE tapes are a vitest gate; stream paused while unpaired; "this browser" marker from `auth/session.device_id`.
- **Rejected:** keeping loopback trusted on Android (any app could drive the node); env-var-wins-only LAN switch (the recommended install command would then lock the dashboard toggle); deferring per-service restart to Layer 9 (a crash-looped model child stayed dead with no honest action); nonce-based CSP (dynamic HTML for one script).
- **Why:** the judges' blockers were integration honesty gaps (a documented one-liner that could not run, two switches that disagreed, a promised action with no route) and a11y defects, not architecture; fixing in place keeps the contract additive-only (every new field/route is additive) and the three-command gate (`check.sh`, `pnpm check`, `40-interface/verify/smoke.sh`) green.
- **Open (external gates, not blockers):** V8/V11 on-device phone run (run-as env, pair link, `.local` on Android) before the first signed release; real-Android check of the LAN-IP pair link; the Layer-6 device-connection decisions (`cross-layer/devices.md` §7, 12 items with defaults) at Layer 6 kickoff.

## D17 — Checkpoint through Layer 4: owner decisions on the open items, Layer 6 device decisions, licensing

- **Date:** 2026-10-14
- **Decision (checkpoint items, built in this cycle):**
  - **Phone trust (D16a confirmed):** on a phone that runs the node, the phone's own browser pairs once; loopback is not trusted under Termux. Built.
  - **Decision (Layer 6 device-connection core, recorded in `cross-layer/devices.md` §7, built at Layer 6 kickoff):** (1) flash offers for blank boards only, with a super-clear confirmation UI (the UI flow is built now: `flash_offer` + typed confirmation; the flashing core is Layer 6); (2) Meshtastic ignored for now — bridge parked; (3) Termux is the phone path; "agent installs and tests existing apps on the phone (Briar…)" parked; (4) TCP-to-hub first; (5) rnsd posture is a user setting with the recommended default; (6) 111d renders `interfaces.toml`, agent proposes typed actions; (7) apply→verify→undo engine wanted, not built now — parked; (8) identify once per device, remembered; (9) flashing human-confirmed from manifest-pinned images; (10) region/callsign asked once, agent suggests only; (11) analog out of scope — parked as the worked example; (12) Reticulum as above.
- **Rejected:** trusting loopback on Android; deferring the Layer 2 residual to Cat-1 port time (owner chose to close it); automatic identify on plug-in; agent-set region; runtime firmware downloads; building the Meshtastic bridge, analog tier or applier now.
- **Why:** the owner wants a clean checkpoint through Layer 4 with nothing provisional left: every open question answered, built where it touches Layers 0–4, parked explicitly (`cross-layer/parked.md`) where it belongs to a later layer.

## D18 — Layer 5 (Messaging & Data) scope lock: next layer, hosted panels, Meshtastic this cycle

- **Date:** 2026-10-14
- **Decision (owner, at kickoff, before research):**
  - **(1) Next layer = Layer 5, "with Layer 6 in mind."** The messaging/data fabric must plug into the device-connection core (`cross-layer/devices.md`: RNode/Meshtastic/MeshCore boards, phone relay over TCP to a hub, BLE later) without redesign; Layer 6 follows on top of it.
  - **(2) Hosting principle for the dashboard (verbatim):** "its important that we do ALL the heavy lifting and give users the unified dashboard the literally can host any mesh and offline project." Adopted projects appear **inside** our dashboard with their own panel; we maintain the integration and never rebuild their UI. This reopens D15's link-out-only / no-iframe rule: the hosting mechanics are a research item (lens 3 of the Layer 5 sweep) and the D15 amendment is decided together with the Layer 5 options brief.
  - **(3) Meshtastic this cycle:** identified (consented probe, D17 #8) + a service row with its hosted panel + read-only presence (nodes it hears) into Nearby; **no message bridge** (`cross-layer/parked.md` P1 stays parked until the envelope exists). Supersedes the "detect and name only" part of D17 #2.
- **Rejected:** Layer 6 opener first (owner chose messaging first); a full Meshtastic bridge now (needs the Layer 5 envelope); keeping Meshtastic at detect-and-name.
- **Why:** human messaging is the flagship service (README §5) and the owner wants one dashboard to be where every network lives; the device core (Layer 6) then binds real radios to a fabric that already exists.

## D19 — Layer 5 (Messaging & Data) design decisions

- **Date:** 2026-10-14
- **Decision:** From the options brief (`layers/50-messaging/options.md`; 8-agent sweep `wf_a4b7e5d6-d10` + one gap scout, ~1.9M tokens, reports in `layers/50-messaging/research/`; polled with the owner 2026-10-14):
  - **(a) Fabric = thin custom Rust core over adopted pipes.** `mesh111`, a library crate under `os/50-messaging/crates/`, wired into `111d` as a module (the keyset never leaves the daemon). It reimplements exactly: envelope codec + message ids · encrypted store with capacity eviction · duplicate suppression + anti-entropy summaries · fragmentation with selective retransmit · authenticated delivery and custody receipts · contacts · the `Link` trait + local API + agent tools. Not reimplemented: crypto (`id111`), radios (RNode/MeshCore/Meshtastic firmware), IP transport, bulk verification (BLAKE3 + bao-tree). Bundle Protocol v7 (dtn7-rs / uD3TN, separate process, `bp7` codec) stays an optional later gateway bridge.
  - **(b) v1 scope = direct links + home-hub mailbox + contact cards.** Contacts enter by card/QR (the L2 genesis announce), by `node1…` (fetch-and-verify over any link), or by INTRO (first message = a message request the user accepts or ignores). Announces are emitted on demand only (card, link-up, INTRO), never on a timer. A node enrolls with a home hub (the L4 §6.9 pair-code ceremony reused); the hub is custodian via `CUSTODY_PUT / CUSTODY_ACK / MAILBOX_FETCH / MAILBOX_COMMIT` over sealed frames on a length-prefixed TCP stream — the `tcp-lan` link, built with L5; the same frames ride LoRa later. Enrolled-only custody; `store_full` refuses honestly; custodians = hubs and always-on laptops, phones opt-in. States: `queued` · `sent` (frame written to a link) · `relayed` (= an authenticated `CUSTODY_ACK ACCEPTED` from a mailbox-role node; a repeater echo is only detail on `sent`) · `delivered` (= the recipient's own ACK, which alone releases the sender's copy). Hub-to-hub replication (A+) follows without redesign.
  - **(c) LoRa ownership and the Cat-1 relay (Layer 6 direction, decided now):** 111-owned boards run the **MeshCore KISS modem** as a raw modem; `111d` builds MeshCore `GRP_DATA` frames (163 B payload) that **stock MeshCore repeaters flood blind** — so the Cat-1 relay is the stock MeshCore repeater (MIT, released, deployed) and 111 writes **no relay firmware**. Reticulum users keep RNode on a second board, or opt into "shared" mode (111d owns the port in HOST mode; rnsd attaches by loopback TCP-KISS). Fragment floor **F = 160 B**. Amends D17 #1: a blank board is offered the 111 modem first, RNode and Meshtastic as alternatives; each board carries `owner = 111d | rnsd` in `state/interfaces.toml`. Before Layer 6 locks: a two-board bench (flood of our data type, scoped regions, airtime) and a data-type range registered with MeshCore; KISS-modem images are CI-built and manifest-pinned (D17 #9).
  - **(d) Hosted panels = one origin per panel by port, behind 111d's proxy.** One listener per enabled panel (block 11190–11229; bound where the dashboard is bound), a manifest per panel (`assets/panels/<id>.toml`: source `static|child|detected`, entry, health, `needs_secure_context`, allowed features, auth mode), one-time ticket (`POST /api/v1/panels/{id}/session`) → host-only cookie `p111_<id>`, exact-Host + `Sec-Fetch-Site` guards on mutations, `s111` stripped upstream, `X-Frame-Options` dropped and `frame-ancestors` replaced at the proxy, `Domain=` stripped from upstream cookies, bodies never rewritten, WebSocket/SSE/large bodies forwarded; the LIVE service page gains a Panel tab (sandboxed iframe) with "Open full client" kept; panels flagged `needs_secure_context` are framed only on a loopback origin with honest copy on LAN; every panel must run in a no-internet netns. **Amends D15:** the iframe ban narrows to "no iframe of any URL that is not a registry-declared panel"; `ServiceRecord.embed_url` is superseded by an additive `panel` object; one `ServiceId` per hosted project; dashboard CSP keeps `frame-ancestors 'none'` and adds an exact `frame-src` list; no TLS in v1 stands.
  - **(e) Store = SQLCipher via rusqlite (`bundled-sqlcipher-vendored-openssl`).** +6.4 MB x86_64 / +5.4 MB aarch64 measured on our musl toolchains; 4.x file format pinned (5.0 is incompatible; `cipher_migrate` planned); WAL, in-memory temp store, incremental vacuum. `KeyProvider` seam: **Mode S** (default) = HKDF-SHA256 from the L2 keyset with info `111/store/v1`; **Mode T** (Cat 4 with TPM2/SE050B) = random key sealed in hardware; never bound to an OS keyring without a recovery path. Blobs = BLAKE3-named files with a refcount table; private blobs encrypted once with an object key carried in the envelope. Layer 1 dependency sign-off (C + vendored OpenSSL) recorded at build in `10-bootstrap/TASKS.md`.
  - **(f) Second panel: Meshtastic only this cycle.** MeshChatX / MeshChat parked (`cross-layer/parked.md` P7); the Reticulum side stays "transport LIVE" with no messaging surface until then.
  - **(g) Meshtastic = Meshtastic's own web client + meshmud multiplexer + presence reader.** The gap-3 scout (`layers/50-messaging/research/07-gap-3-meshmonitor.md`) found that MeshMonitor's Virtual Node speaks raw Meshtastic TCP only, so the official web client (HTTP / Web Serial / Web Bluetooth transports) cannot ride it; **meshmud** (MIT, Go, v1.0.0 2026-03-20, ~2.5k lines) holds the single serial or `:4403` slot and fans out — its HTTP downlink (`/api/v1/toradio|fromradio`) serves the self-hosted **Meshtastic web client** (GPL-3.0-only static bundle, hosted as a panel; HTTP mode works on plain HTTP), its TCP downlink feeds a small **GPL Python presence reader** (meshtastic-python, own source dir, own hash-locked venv, separate process) that writes a presence file 111d reads into Nearby. Consented identify (D17 #8), Meshtastic branch = start the multiplexer and read `want_config` once. "Read-only" = 111 never transmits on the mesh and never changes node config; config reads are fine. One HTTP panel client at a time (meshmud's limit). meshmud is vendored and version-pinned; a hardware test (config replay to a second client, DTR/RTS on open, Termux build) gates the first signed release, not the spec lock. MeshMonitor stays a candidate for a later optional service (best presence fields, MeshCore coverage), not the Meshtastic panel.
  - **Detail-level (spec-reviewed, not polled — full text in the brief §3):** envelope layout (L6 shim outside the AD; L5 outer header in the sealed packet's `extra`: `prio_flags` + 4 B `dest_hint`; inner `sender_ref` 4 B + `pair_mac` 16 B; 64 B signatures mandatory for stored, public and code classes) · message id = (sender, stream, `msg_seq`) with a 2^32 jump on restore · fragment-then-seal tiers (Tier 0 one packet; Tier 1 ≤ ~16 authenticated fragments, ≤ 16 outstanding; Tier 2 sealed advert + hash-verified bao chunks, unsealed for public/already-encrypted objects) · authenticated ACK and custody receipts with domain tags · groups v1 = pairwise fan-out · ratchet space reserved · no timestamp field (sequence order + receipt-time display) · 2-bit relay priority in the clear, fine `kind` inside · capacity-only pruning order with "never undelivered outbound" · multi-device v1 = hub-is-home · L3 offload stays on the LAN binding (not v1 fabric traffic); link keys never seed-derived, iroh not in v1 · bridges = seams only · `messages.send` is proposal-gated.
- **Rejected:** MeshMonitor as the Meshtastic panel (not Meshtastic's own client; Node 22+, ~189 MB image, sends admin/LocalStats requests to the node, binds all interfaces) · loopback-only Web Serial with no node-side presence (fails D18 #3) · a multiplexer of our own (GPL protobuf decoding would enter our binary) · BPv7 via dtn7-rs as the core (clock-based lifetime, 37–57 B per fragment, no LoRa layer, unauthenticated loopback API) · LXMF/Reticulum, Berty/wesh, qaul, Veilid, SSB as the fabric (D17 #12; their identity replaces ours) · LAN-multicast epidemic and announce-as-routing in v1 (pull Layer 7 forward; phones excluded) · RNode-everywhere with 111-native Cat-1 firmware (reinvents a repeater) · deferring the radio choice (frame budget designed blind) · rnsd owning the port with 111 traffic over LXMF (no clean carrier) · Home-Assistant-style sub-path hosting (third-party JS shares our origin) · subdomain-only hosting (loopback only) · link-out only · plain SQLite + own record AEAD and pure-Rust KV stores (we would invent and audit a format) · a second panel this cycle · COSE as the packet envelope (+31 B, no XChaCha20) · MLS in v1 (needs commit ordering) · seal-once-then-fragment (slices unverifiable) · iroh in v1 (QUIC-only, +13.3 MB, blobs pre-production).
- **Why:** no adopted project carries our envelope across 160 B frames without a clock and with our identity, so the custom surface is honestly seven small parts and everything else is adopted; the home-hub mailbox is the smallest delivery model that is true on phones today; MeshCore's stock repeater removes the only firmware 111 would otherwise have to write; port-per-panel is the one hosting shape that survives an offline LAN without handing our origin to third-party code; SQLCipher is the adopted, production-proven store and its cost is a binary-size line item, not a design.

## D20 — Layer 5 spec locked (v1.2); lock amendments

- **Date:** 2026-10-14
- **Decision:** `os/50-messaging/README.md` is **locked at v1.2** after the adversarial panel (3 fable judges — crypto *not-ready* 2 blocking/10 major/11 minor · panels *fix-then-lock* 1/8/8 · integration *fix-then-lock* 1/7/9; triage `layers/50-messaging/review/triage.md`, reviews `layers/50-messaging/review/`) and the fable re-check of the rewrite (*fix-then-lock*: 53 fixes landed, 4 partial, 0 missing; 13 new findings, all adopted; `layers/50-messaging/review/recheck.md`). The D19 architecture is unchanged. Amendments adopted with the lock, recorded as deltas against D19 (lead calls inside the decided architecture; the owner may reverse any of them):
  - **(a) Scope cuts for v1:** attachments and the Tier-2 wire transfer move to v1.1 (the blob store, BLAKE3 addressing and reserved kinds ship now) · the generic reverse proxy with WebSocket/SSE is specified but reserved — v1 ships static panels plus the built-in Meshtastic shim · first contact by message (INTRO) is `tcp-lan`-only; LoRa first contact needs a card · the identify route builds the Meshtastic step only (daemon port-owner table, binding in `devices.json`; Layer 6 completes the sequence and creates `interfaces.toml`).
  - **(b) D19 g delta:** meshmud's HTTP downlink is not configured; `111d`'s shim serves `/api/v1/toradio|fromradio` on the guarded panel origin by reframing onto per-session meshmud TCP downlinks (no browser-speakable upstream, no one-client limit) · meshmud is CI-built from a pinned upstream tarball, never copied into `os/`, and the binary is conveyed as GPL-3.0 (own code MIT; it links Meshtastic's GPL Go protobufs — verified at build).
  - **(c) D19 e delta:** the store key KDF is keyed BLAKE2b off a keyset-derived secret (`111/store/v1`), not HKDF-SHA256 — one hash family with Layer 2.
  - **(d) D19 d delta:** guards on **every** panel request (cross-site refused for any method, `OPTIONS` refused, every `Access-Control-*` header stripped), no loopback ticket skip, ticket/cookie lifecycle pinned, per-request dashboard CSP, the main listener answers `Host: localhost` with a 308 to `127.0.0.1`, snapshots carry `panel.port` never an origin.
  - **(e) Detail deltas:** sender authentication = `pair_mac` always present plus an optional `sig` that never keys replay state · message identity = `(sender, recipient, stream, epoch, n)` with `msg_seq` on the wire as two varints; epochs minted once per store open (unclean record, `announce_last` mismatch, restore floor, wrap) and frozen old-epoch receive state (newest 4) · L2 amendment A11: replay authority for `msg_seq` kinds is the fabric's state, every packet still offered to the L2 window whose fresh/stale verdict gates connection attribution and pushes · custody `REJECT_UNKNOWN_DEST` and the one-hub v1 scope sentence · A5 (crypto-mode `0x02` and the unsealed-chunk carve-out) deferred to v1.1 · the agent reaches `/api/v1` with a per-spawn loopback-only bearer token.
- **Layer 2 amendments requested (A1–A4, A6–A11):** land as `id111` code (task I1) + `20-identity/README.md` text (task C5) in this build, mirroring D14's L1 pattern; the Cat-1 harness vectors are untouched.
- **Rejected:** nothing in triage — every panel and re-check finding was accepted, four with lead modifications recorded in `layers/50-messaging/review/triage.md`.
- **Why:** both passes found the architecture right and the defects textual (unbuildable receipt formats, a replay-attribution hole, an epoch rule that re-opened the custody-loss case, a `localhost` corner); fixing in place keeps every D19 decision intact and the contract additive, and the four scope cuts remove the only pieces no gate would have exercised this cycle.

## D21 — UI bundle budget amended: 256 KB eager + lazy buckets

- **Date:** 2026-10-14
- **Decision (owner, polled after Wave 1 U1):** "raise to 256 and add lazy buckets". Encoded by the lead as **two budgets
  of 256 KB gzip each** — *eager* (the first-paint download: entry script + every `modulepreload` + every `.css`) and *lazy*
  (every chunk `index.html` neither loads nor preloads: the view routes, jsQR, the notices page) — plus a **64 KB cap per
  lazy chunk**. `40-interface/ui/scripts/check-budget.mjs` classifies by reading `dist/index.html`; the grand total is printed
  for information only. Measured at the decision: eager 101.2 KB · lazy 154.2 KB (largest chunks jsQR 47.2, notices 34.1).
- **Lead detail, reversible:** the per-bucket reading (rather than 256 KB grand total) is the lead's; a grand-total cap
  would have passed with 0.6 KB to spare and tripped again on Wave 2's three views. The 64 KB per-chunk cap is the lead's
  guard. Either can be changed with a one-line edit to the gate.
- **Rejected:** patching jsQR's Kanji table out (a dependency patch to maintain; the cap moves again next wave anyway) ·
  dropping jsQR for the browser `BarcodeDetector` (Chrome/Android only) · keeping 200 KB total.
- **Why:** the Layer 4 number guarded first-load weight on constrained devices; lazy chunks do not load until a feature is
  used, so budgeting them apart keeps that guard honest while letting the dashboard grow a view per layer.

## D22 — Relicensed to GPL-3.0-or-later: open source, commercial use allowed, no military clause

- **Date:** 2026-10-19
- **Decision (owner):** 111network's own code is free software under the **GNU General Public License, version 3 or (at your option) any later
  version** (`GPL-3.0-or-later`). Copyright holder on the notice: "111network contributors". The source-available draft of D17 (PolyForm Noncommercial 1.0.0
  plus a no-military-use condition, commercial use by separate agreement) is withdrawn; it was never part of a release. Applies to `os/` and to the website
  repo (`LICENSE` added there). `os/LICENSE.md` carries the verbatim FSF text; the four crate manifests and `package.json` declare the SPDX id instead of
  `license-file`; `LICENSING.md` section 7 now checks GPL-3.0 compatibility of dependencies instead of excluding copyleft; contributions are accepted under
  the same license (inbound = outbound, no assignment, no CLA), and any sort of contribution is appreciated (README, `LICENSE.md`, `LICENSING.md`).
- **Rejected:** Apache-2.0, MIT and MIT OR Apache-2.0 (permissive: distributed forks could go closed) · keeping the military ban as an added condition (an
  OSI-approved license may not restrict fields of use, so the project could not call itself open source) · keeping the draft pending a lawyer's review (a
  standard, unchanged FSF license needs none; the legal reads that remain are the third-party ones in `LICENSING.md` section 9).
- **Why:** the owner wants the project to be open source in the OSI sense with commercial use allowed; copyleft keeps every distributed change free, which
  matches "free secure communication is everyone's right", and it sits naturally beside the GPL-3.0 neighbours 111 hosts (Meshtastic firmware and web client,
  meshtastic-python). Follow-up: the release checklist must publish the tagged Corresponding Source beside every binary release (`LICENSING.md` section 8).

## D23 — Layer 6 (Transport & Bridge) kickoff: sequencing, bridge semantics, v1 networks, hardware

- **Date:** 2026-10-19
- **Decision (owner, at kickoff):**
  - **(1) Sequencing.** Layer 6 research and its options brief start now; the Layer 5 closure (Wave 2 bounce, V1–V4, lock entry, tag)
    runs alongside with at most two or three build agents; the Layer 6 build starts only after the Layer 5 lock. The plan folder is
    updated as work lands (`plan/layers/60-transport/`).
  - **(2) What "bridge" means.** Readable text flows both ways between our network and the foreign ones. Owner's words: "the chances
    are that people will get used to our network fully, even the dashboard. The UI of other services must be available fully, but
    still text appears in ours, in the Messages view, from different networks. Keep it categorized for other networks." So: the
    foreign networks' own UIs stay hosted as panels (Layer 5 §13), **and** their messages appear in our Messages view under their own
    per-network categories (never merged into 111 threads); our users can send into those networks in each one's native format; a
    node on several networks relays between them (per-channel opt-in, loop guard; design in the Layer 6 spec). Sealed 111 traffic
    keeps riding those radios opaquely between 111 nodes through the Layer 5 §8 link seams (`meshtastic-client`, `rns-bridge`).
    This supersedes the Layer 5 "no message bridge" scope line (D18 #3, D20) and the §14 "111 never transmits on the Meshtastic
    mesh" promise, which the Layer 6 spec restates honestly as "111 transmits only what the user sends or bridges".
  - **(3) v1 networks.** Meshtastic, Reticulum/LXMF, and phone apps reached through Termux (parked P2: Briar and the like; the research
    sweep maps what is honestly feasible before anything is promised). The MeshCore *network* was not selected as a bridge target;
    the 111 KISS modem on MeshCore firmware (D19 c) stays a Layer 6 transport item, sequenced by the options brief.
  - **(4) Hardware.** No LoRa boards are available yet. The owner will set up a full test setup later to test everything fully; until
    then every radio path is built and verified against stubs in network namespaces, and the hardware gates stay in the runbooks.
- **Rejected:** closing Layer 5 before any Layer 6 work (owner wants progress on both) · Layer 6 only, leaving Layer 5 open · carrier-only
  bridging (foreign meshes carry sealed 111 envelopes, nothing readable) · inbound-only content bridging · MeshCore as a v1 bridge target.
- **Why:** the bridge is the point of the layer ("we can send the same message in several networks, we receive from other networks and
  bridge; the bridge function is super important"); users may end up living in our dashboard, so our Messages view must carry every
  network, while the adopted projects' own UIs remain available unmodified (adopt, don't reinvent).

## D25 — Layer 6 (Transport & Bridge) design decisions

- **Date:** 2026-10-19
- **Decision (owner, over the options brief `plan/layers/60-transport/options.md`):**
  - **(F1) Foreign messages live in the mesh111 store:** additive schema V3 (foreign conversations, foreign messages, relay edges, send
    groups) in the same SQLCipher store with typed `Fabric::foreign_*` calls and a prune class in the §10.4 order. Amends the locked Layer 5
    §10.3 additively.
  - **(F2) Meshtastic text in-binary through the official `meshtastic` Rust crate** (GPL-3.0), as one more client on the existing meshmud
    downlink, with 111's own allowlist on top (text to a channel or a node; never admin, config or a heartbeat with nonce 1). Accepted: the
    shipped `111d` binary is effectively GPL-3.0-only (our files stay "or later"; a legal read on the protobufs' missing version notice).
    Nearby folds into that client and the Python presence reader retires after parity.
  - **(F3) LXMF carrier — owner's rule, lead's pick.** Owner (verbatim): "try to keep it as clean as possible, separate and on demand
    running, meaning if it's not running, it doesn't make our system heavier! you decide this one, performance is super important + I want
    users to have 100% freedom." Lead's pick within that rule: headless MeshChatX 4.10.7 (slim closure) as an **optional, on-demand
    supervised child** behind a swappable carrier trait — nothing of it runs, loads or listens unless the user enables the Reticulum
    bridge; on hosts whose `python3` is older than 3.11 it runs from a bundled interpreter that is likewise installed only when enabled;
    the same child is the hosted Reticulum panel. The RNS-importing sidecar (option C) stays the contingency for MeshChatX's 2027-01-01
    sunset or for verified-sender labels. Nothing LXMF enters the `111d` binary.
  - **(F4) Phone apps — same rule, lead's pick.** Owner (verbatim): "keep it as thin as possible and separate so we don't make the repo
    heavier, decide yourself. again, the goal is users get hooked to our UI but we offer opening the native of other apps as well in our
    dashboard separate." Lead's pick: one app-peer adapter shape; Delta Chat first (`deltachat-rpc-server` plus the Madmail chatmail relay
    on the hub, both optional on-demand children; a Termux phone can be a Delta Chat node); SimpleX second, optional, on glibc hubs only;
    Briar documented online-only (Tor); each app's native UI offered separately (hosted panel where a web UI exists, otherwise a link-out
    with honest copy); driving a phone's UI stays a user-attended runbook outside v1.
  - **(F5) Bridge-first on a minimal transport spine; first signed Layer 6 release = bridge + spine + phone relay + device core (R-B).**
    Waves: 0 spine (`interfaces.toml`, rnsd posture v2, live attach, `transport.read`, store V3 + contract, A10 registry amendment) ·
    1 bridge core + Meshtastic + Messages categories + agent tools · 2 LXMF + app peers + phone relay v1 · 3 device core (hotplug, consented
    identify via adopted tools, RNode stanza proposals, the narrow apply→verify→undo cut) + Settings → Layer 6 · 4 netns verification + fable
    review. The 111 KISS modem link, its CI image, the MeshCore data-type PR and **(F6) sealed 111 carriage over foreign radios** form the
    **second release** once the owner's bench exists.
  - **(F7) Relay defaults:** edges are explicit, directed, non-transitive, between channel or group conversations only, enabled by a human on
    a paired or loopback session, the agent proposes; a relayed copy is never relayed again; own-echo suppression; a visible `[name/TAG] `
    marker (plus the structured field where the carrier allows it). Accepted defaults: no courtesy notice on the source channel; readable
    relay into an amateur-licensed radio only after a human confirms, sealed frames never; a backlog is held to capacity, then shed oldest
    with a visible counter. **Not accepted as a default:** honouring Meshtastic `ok_to_mqtt` as a relay filter — it becomes a per-edge
    switch, off by default (user freedom).
  - **(F8, not polled) Fan-out:** a targets picker, one send group, one message row per target with its own state and retry, a per-target
    preview (bytes, parts, airtime fact, refusals); oversize refused by default, split an explicit per-send choice, never silent truncation.
  - **Optional components — owner's framing (verbatim):** "the whole idea is to ship thin layer of own, handselect a few that run native,
    not fully decided which yet, then we do a full review and integrate." Reading recorded for the spec: the base install is the thin 111
    layer; adapters and their adopted programs are separate, on-demand components that weigh nothing while disabled; **which adopted
    projects run "native" (shipped and integrated) is hand-selected by the owner after a full review** — the spec designs the component
    seam (signed, manifest-pinned, enable-time install from the payload or a pre-seeded cache) and does not pre-decide a download path.
  - Detail picks from the brief §3 (categories rail, delivery evidence, foreign DMs land in this node's Messages, Meshtastic defaults incl.
    never writing node configuration in v1, A10 `TransportId` values for Meshtastic and Reticulum, rnsd posture v2 per D17 #5, identify via
    adopted `rnodeconf --info`, README §6's ordered-network list as KB entry + adapter order with the runtime fall-through deferred to
    Layer 7, MeshCore dev range `FF00–FFFF` until the bench, PSK keyed hash, MQTT-uplinked channels off by default for sends, LXMF visible
    prefix + hash dedupe, the narrow P4 cut for device actions) are the lead's, to be attacked by the fable panel before the spec locks.
- **Rejected:** a separate bridge store in node111 (second key and prune) · pseudo-NodeIds for foreign senders · an own prost client as the
  first choice (adopt, don't reinvent) · a Python two-way Meshtastic sidecar · MeshMonitor as the send path · lxmd alone · Rust Reticulum
  reimplementations (not upstream) · raising the LXMF floor instead of bundling (owner wants freedom; decided by the lead within the
  on-demand rule) · SimpleX first (glibc-only, no Android peer) · Briar first (Tor-only) · "framework only" phone apps · transport-first
  sequencing · sealed carriage in the first release · matterbridge or a Matrix/XMPP hub as the bridge core · a courtesy notice by default ·
  `ok_to_mqtt` as a default relay filter.
- **Why:** the bridge is the point of the layer and every bridge path is provable in network namespaces today, while the LoRa modem and the
  device probes need boards the owner does not have yet; the owner's rules for adopted programs (separate, on demand, no weight when off,
  users hooked on our UI with the native UIs one click away) decide every adapter's shape.

## D26 — Layer 6 spec amendments after the fable panel: own LXMF helper, install-time component cache, every restart a proposal, licensing posture

- **Date:** 2026-10-19
- **Context:** draft v0.1 (`23283ba`) went to three fable judges (bridge · carriers · integration), all **amend-then-lock**; the lead's triage
  (`plan/layers/60-transport/review/triage.md`) accepts every finding except where the owner had to decide. The owner's answers:
- **Decision (owner):**
  - **(1) Optional component bytes — install-time cache.** `111 install` pre-downloads each declared optional component's hash-locked payload into
    `cache/packages/<id>/` when online (the existing rns / presence pattern); nothing is installed, loaded or running until a human enables the
    component; the cached size is shown. "Zero weight" means runtime weight (no process, listener, loaded code, timer or open file), not disk.
  - **(2) Agent restarts — every restart is a proposal.** No bearer exception at all; one global rule: the agent's bearer is refused on every write
    (an explicit gate in `require_session`), `identify` included; G1 asserts it on every Layer 6 write.
  - **(3) Attestation — bytes and verifier now, labels later.** A10 transport ids `0x02`/`0x03` and the `TransportVerifier` plug land in Wave 0; the
    gossip kind, the `attested-111` label, "already bridged by node X" and "Open the sealed thread" move to the second release.
  - **(4) LXMF carrier — our own thin helper, not MeshChatX.** Owner: "lets make our own helper, attribute their work, I will contact them to find
    out if we are ok." The carrier is a small sidecar of ours (the brief's option C): a separate process in its own hash-locked venv that imports the
    **user-installed** `rns` / `lxmf` packages, speaks JSON Lines to `111d`, and credits the Reticulum project; `111d` still imports nothing of
    Reticulum. MeshChatX is dropped as the carrier (and with it the 128 MiB slim closure, the bundled CPython 3.11 and the 2027 sunset); it may return
    later as an optional hosted panel only. The licensing row for the helper is **pending the owner's contact with upstream**; until that answer the
    helper ships behind the same enable gate as every optional component, with the notice below. D17 #12 is amended accordingly: Reticulum stays a
    separate service never incorporated into `111d`; the helper is the one named program of ours that imports it, as a separate process.
  - **(5) Reticulum wheels and licensing posture — always download, never bundle; the user installs.** `rns` / `lxmf` (and every other third-party
    program a component needs) are downloaded hash-locked from their public index at install or enable time and never ride in a 111 release payload.
    Across the whole licensing story the **user is the one who installs third-party software**: before any such download or enable, 111 shows a short
    notice naming the program, its licence and where to read it, and the user acknowledges it (recorded); 111 and its author are not the distributor.
    Owner: "always download, never bundle. also, in the whole licensing, try to make the user responsible and keep me as the author safe … make
    little notice to read their licence etc of that app." Applies to the Layer 1 rns venv at install as well (one notice line; a Layer 1 doc/UX item).
  - **(6) Delta Chat — sandbox spike first; drop from R-B if it fails.** A netns spike (`deltachat-rpc-server` + Madmail) precedes any build task;
    if it fails, Delta Chat leaves the first release and returns with the phone bench. Bridge-first in R-B = Meshtastic + LXMF (+ Delta Chat only on a
    passed spike).
  - **(7) Everything the triage marks ✅** (marker alphabet and marker-first parts, tick retries with capped back-off, post-air NAK honesty, live
    consent with pause, `relayed_seq`, Foreign shaping quota and user-first queueing, preformatted proposal bodies, Cc+Cf stripping, 0600 rnsd
    config, child ordering, batched foreign ingest, the gate script homes, `interfaces.toml` as the single source, `ForeignMessageState`, the pin
    list, two stale citations) goes into draft v0.2, then one fable re-check, then the lock entry.
  - **Owner's framing for the round:** "finalize this round without over engineering but wired to previous layers."
- **Rejected:** MeshChatX as the carrier (sunset, closure weight, a second interpreter) · bundling Reticulum wheels or any third-party program in a 111
  payload · a direct bearer restart route · attestation gossip in R-B · USB-only component payloads (no stock path) · shipping Delta Chat on doc-only
  evidence.
- **Why:** the owner wants a thin layer of our own wired to the built layers, users responsible for the third-party software they choose to install,
  and the author kept safe; the panel's blockers were all seams, not design, and these answers close every one.

## Pending

- Move deep architecture content out of `os/README.md` into per-layer specs (blocked on layer homes existing).
- **Layer 0 hook activation** — owner chose to leave it off (D17); `build.py --check` stays manual per task.
- **Release source offer** — the release checklist must publish the tagged Corresponding Source beside every binary release (D22; `LICENSING.md` section 8).
