# Layer 4 — adversarial panel triage (v1 draft → v2)

Panel: 3 fable lenses (architecture / design-UX / contract), 2026-10-12, 41 findings. Verdict: fix-then-lock.
Full findings in the session journal; this file records the disposition folded into spec v2.

| Finding | Disposition |
|---|---|
| @types/react missing from the "exact" dep set (critical) | fixed §2 |
| Tailwind 4 browser floor vs old phones (critical ×2) | owner decision D15: keep TW4, document floor, feature-detect notice, `/v1` fallback page |
| Identity backup over HTTP contradicts Layer 1/2 locks (critical ×2) | removed; CLI instruction card only |
| LAN auth disowned (critical ×2) | owner decision D15: pairing auth now (§11) |
| Chat streaming correlation unspecified (critical) | client-minted conversation id; chat tokens ride the POST reply; named events (§10) |
| restart/update-check endpoints missing; Layer 1 GET-only clause (critical) | added; clause scoped (D15 sibling amendment 1) |
| Layer 3 path/stream mismatch; relay unowned (critical) | relay owned here; translation table §10 |
| Sticky `?mode=` kills the toggle (major ×2) | consume-once + strip (§5) |
| Contract gaps: actions, agent history, log params, settings merge, transport reorder (major ×3) | folded into openapi.yaml; agent history cut (no L3 backing); transport reorder deferred to L6 |
| Mock bundled into prod; lucide import rule (major) | DEV-gated dynamic import; named imports only (§2) |
| Status vocabulary undefined (major) | §7 registry states + `reason_code`; dots §3 |
| "Advanced never owns an action" contradicted (major) | reworded: safety-critical actions in both modes; configuration may be advanced (§5) |
| Missing primitives: data-table, inputs, disclosure, buttons/confirm (major) | §4 |
| Header unbuildable at 360 px; no drawer (major) | drawer + MENU button + icon rail (§5) |
| Token stream render batching + aria-live (major) | ≤10 Hz flush; live-region rules (§3, §6.1) |
| SSE semantics loose (major) | §10.1 + api/README.md |
| Update state enum + rollback alerts producer (major) | `status.update` block; `proposal`/`agent` events; alerts via errored services |
| Config validation error structure (major) | `errors[]` pointer/message in Problem; raw config editor deferred |
| Mock scenarios incomplete; http adapter untested (major) | 6 scenarios incl. first-run/agent-down/updating; tapes recorded from the real daemon (§9, I1) |
| /health + /api/status relationship (major) | kept as Layer 1 seam, `/api/v1/status` is a superset (§10) |
| Logs: journald false on Android (major) | per-service file tails from `111d`; journald note (§6.6) |
| minors (shadow provenance, rust-embed wording, font stanzas, clipboard fallback, tsconfig lib, ui tree/ownership, budget buckets, status-pill provenance, reduced-motion scope, ASCII aria, query placement, scenario on http, ADV badge/console history, font preload, wording traps, wiring-map sequencing, dev CORS proxy, port succession, chat history pagination) | all folded (§2–§6, §9, §14, TASKS ownership column) except: console command history (console deferred), chat history pagination (no L3 backing; deferred) |
