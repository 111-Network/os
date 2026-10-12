# Layer 4 (Interface & Dashboard) — Options Brief

Date: 2026-10-12 · Status: awaiting user decisions
Source: 3-scout fast sweep (stack / design extraction / dual-mode UX), per user request: deliberately light research, focus on the build.

## Context and constraints (from README + D9)

- Dashboard is served by the node (111d, musl-static Rust binary) or by a nearby hub for constrained devices; opened in any browser, laptops **and old phones** (Cat 2A light terminals).
- Fully offline: zero runtime external requests (fonts, scripts, all bundled).
- Content: chat with the agent (primary interface), node status, mesh status, agent notifications, settings.
- The UI↔daemon API is a **first-class deliverable** — "the agent's clean, stable function surface".
- Strict `website/docs/DESIGN.md`: monochrome, terminal aesthetic, sharp corners, mono + pixel type.
- Component-scalable; "code that can be avoided should be avoided".
- 111d (Layer 1) is still speccing → this cycle builds the **bases** (scaffold, design system, shell, views, mock seams); wiring to the real daemon is deferred by design.

## D1 — Tech stack

| Option | Fit | Verdict |
|---|---|---|
| **A. React + Vite static bundle → rust-embed into 111d** | 4.5/5 | **Recommended** |
| B. Next.js static export (reuse website app) | 2/5 | Rejected: heaviest runtime for an offline old-phone target; website itself warns Next 16 is a moving target; reuse is tokens+patterns, which port without the framework |
| C. HTMX + Rust templates (askama) + Tailwind standalone CLI | 3/5 | Honest fallback if Node in os/ CI becomes a blocker; rejected primary because it forces a second UI surface (HTML fragments) alongside the agent's JSON API — every feature built twice |
| D. Vanilla JS / WebComponents, hand-rolled CSS | 2/5 | Rejected: rebuilds Tailwind + a mini-framework by hand — bespoke surface the philosophy forbids |

**Option A shape:**
- Standalone pnpm + Vite + React app at `os/interface/ui/`, static `dist/` later embedded into 111d via `rust-embed` (`debug-embed` for dev).
- Tailwind CSS 4 via `@tailwindcss/vite`; DESIGN.md tokens in `@theme`; `globals.css` ported from `website/src/app/globals.css`.
- **No shadcn** (DESIGN.md bans its rounded primitives; ours are raw elements), **no framer-motion** (CSS keyframes + IntersectionObserver hook instead), **no state library** (React context + hooks), wouter or hash routing only.
- Fonts self-hosted woff2 with hand-written `@font-face` (all SIL OFL): Geist Mono (variable), Geist Pixel Line (extract from `geist` package — it peer-depends on Next and can't be imported), Silkscreen 400/700.
- `build.target: es2017` for old-phone browsers; bundle budget enforced in CI (~200 KB gz JS+CSS); preact/compat alias held in reserve (~13 KB runtime) if Cat 2A profiling demands it.
- Cost: one Node/pnpm step in os/ CI (team already runs this for the website).

## D2 — Design system port (facts from extraction scout)

- Live tokens: `website/src/app/globals.css` (⚠️ `src/styles/globals.css` is a **stale decoy** — default shadcn oklch, radius 0.625rem; ignore).
- Token ladder: `#000/#fff/#0a0a/#1a1a/#262626/#a0a0a0/#ff3333`, `--radius: 0rem`, dark-only (`html.dark`, no theme switching).
- Keyframes to copy verbatim: `blink`, `scanline`, `glitch` (no utility), `.animate-blink`, `.animate-scanline`, reduced-motion kill switch, inverted `::selection`, 4px `#333`-on-`#000` scrollbar, the single layered box-shadow string.
- Primitives to replicate first (all zero-dep JSX in the website): **terminal chrome header** (3 dots + `{title} ~ terminal`), **panel** (`border border-border`, `bg-secondary/20` body tint), **SpecTable** (`divide-y`, `hover:bg-secondary/50`), **interactive toggle button** (active = `bg-foreground text-background` inversion), **badge/status pill**, **MiniTerminal** (typewriter log), **PseudoTerminal** (command registry = power-user console), ghost-number section header, hover inversion everywhere, `focus-visible:ring-2`.
- Portability: ascii-hub components use no `next/link`/`next/image`; only Next couplings are fonts + framer-motion. `tw-animate-css` import must be kept or inlined.

## D3 — Simple/advanced model

**Recommended: Home-Assistant-style single-tree mode flag** (not two UIs, not GL.iNet's escape-hatch door).

- `ModeProvider` context: `'simple' | 'advanced'`, default simple. Client-side only (localStorage `111.ui.mode` + `?mode=advanced` URL override) — never a node write (hub-served devices must behave identically; no auth phase yet).
- Toggle = segmented `[SIMPLE|ADVANCED]` control in the **header** (not buried in settings — the documented HA complaint), built from the DESIGN.md interactive-toggle primitive; mirrored once in Settings; persistent `ADV` badge when advanced is on (second documented HA complaint).
- Mode changes **visibility/density only**: nav items and panel sections carry `minMode` flags; advanced **appends** panels/rows; routes, grid, and simple-card positions never move (no layout shift, no bait-and-switch).
- Hard rule: every actionable item (agent approvals, restart, identity backup, updates, real errors) visible in **both** modes.
- Simple nav capped at **5**: Chat (first, primary), Node, Mesh, Inbox (agent notifications w/ Approve/Dismiss/Why?), Settings.
- Advanced appends: Peers, Transports, Bridges, Gateway, Logs, Agent diagnostics, Identity, Storage, Raw config + debug console (PseudoTerminal pattern).
- Plain language in simple view ("Your node is running normally"), but `#ff3333` stays reserved for real errors in both modes — never false calm.
- v2 seam (scaffold now, build later): an **action registry** (id, label, minMode, handler) consumed later by a Cmd-K palette and the agent chat.

## D4 — UI↔daemon contract

**Recommended: REST + ONE multiplexed SSE stream (no WebSocket).**

- All live needs are server→client (agent tokens, notifications, status ticks); SSE auto-reconnects via native `EventSource`, survives HTTP/1.1 through hub topology, needs no upgrade dance in axum, is curl-debuggable.
- Endpoints under `/api/v1/`: `POST /chat`, `GET /chat/history`, `GET /status/node`, `GET /status/mesh`, `GET /notifications`, `POST /settings`, `GET /events` (SSE; named events `agent.token`, `agent.message`, `notify`, `status.node`, `status.mesh`; monotonic ids + `Last-Event-ID` resume from a ring buffer).
- One SSE channel avoids the browser 6-connections-per-origin limit — one pipe for old phones.
- Versioning: path prefix, additive-only evolution; UI+daemon ship in one binary → short compatibility windows.
- Typing (interim): `os/interface/api/openapi.yaml` is the source of truth **until 111d lands**; UI generates TS types via `openapi-typescript` at build. When 111d lands: Rust DTOs + `utoipa` take over as source, CI drift-checks emitted doc vs the yaml (fallback: ts-rs/specta).
- Errors: RFC-9457-style `problem+json` with machine-readable `code`. Auth: localhost/LAN bind, optional bearer token later; CORS stays closed (same-origin in prod).
- **Mock seam (this cycle):** every panel talks to one typed data-adapter interface; a mock adapter serves fixtures + simulated SSE in-browser (zero server needed for dev/demo); the real adapter is a drop-in when 111d exists.

## D5 — Layer home and this cycle's build scope

- Home: `os/interface/` (per D1 layer-slug convention): `README.md` (spec), `TASKS.md`, `api/openapi.yaml`, `ui/` (Vite app), `fonts/` (vendored woff2, license notes).
- Scope = **bases only**: scaffold, tokens, fonts, primitives, shell + mode system, 5 simple views + advanced panels with mock data, data-adapter seam, action-registry seam. No 111d embedding (Layer 1 dependency), no real API server, no e2e.

## Open decisions for user

1. Stack: Option A (React+Vite) as recommended?
2. Mode model: single-tree HA-style as recommended?
3. Contract: REST + single SSE + interim `openapi.yaml` source-of-truth?
4. Build scope: bases as listed — trim or extend?
