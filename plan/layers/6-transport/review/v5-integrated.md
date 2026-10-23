# V5 — integrated review on the running system (2026-10-23)

The judge's own notes (run `wf_feb6a10f-f14`, `runs/workflows/wave-3b-review.js`), copied verbatim from its scratch file. Verdict: **fix-then-lock** — one major (the posture/device applier runs inline in the request future, so a client that disconnects mid-apply cancels it and leaves the posture applied, nothing reverted and a pending record) and four minors; every gate green by the judge's own run; the Pi floor measured. The fixes land in fix wave B (`runs/workflows/wave-3d-fix.js`); the verdict and findings are in the V5 row of `6-transport/TASKS.md`.

---

# Layer 6 V5 — integrated review (fable judge), 2026-10-23

Branch `claude/layer-6-implementation-006mte` at `71ad970`. Debug `111` built in the warm target dir (`cargo build -p node111`,
CARGO_INCREMENTAL=0, CARGO_BUILD_JOBS=2: already current). Nothing was committed or edited in the repo.

## Gates run by the judge (last line verbatim)

| gate | last line | notes |
|---|---|---|
| G1 `bridge.sh` (G1_SKIP_BUILD=1) | `G1 GREEN` (`G1: 129 ok, 0 FAIL in 242.0s`) | 129 = the lead's 130 minus the skipped cargo-build ok line |
| G3 `lxmf.sh` (SMOKE_SKIP_BUILD=1) | `G3 GREEN` (`G3: 146 ok, 0 FAIL in 232.5s`) | venv built from PyPI under the scratchpad and deleted after |
| G4 `spine.sh` | `G4 GREEN` (`G4: 128 ok, 0 FAIL in 247.0s`) | |
| G5 `devices.sh` | `G5 GREEN` (`G5: 98 ok, 0 FAIL in 180.9s`) | |
| G7 `zero-weight.sh` (REQUIRE_G7=1, real 111-agent symlinked, daemon in unshare) | `G7 GREEN` | process set = {111d, 111-agent}; listeners 11180 + 80 (111d), 11182 (agent); nothing cached/installed |

## Hand-driven probes (one `111 daemon` + the G1 scenario stubs, fresh user+net+pid namespace)

What holds (verified by running):
- The bearer is refused (401 `unauthenticated`) on every Layer 6 write tried, with path variants (`?query`, trailing slash → 404, case → 404);
  a bearer GET of a thread never clears `unread`; a human GET of the newest window does; `since_seq` reads never do.
- Hostile bytes end to end (real code points, python-built feed): U+202E/200D/200B/FEFF/00AD/2066/2069/0085/NUL/CR stripped from body,
  name and conversation title; `\n` `\t` kept; NFC applied; name cut to 64 B at a UTF-8 boundary; an all-Cf name reads `""`;
  a 70,000 B body stored cut to 65,536 B with `truncated: 70000`; an adapter's `meta` cannot forge `handoff`/`truncated`/`not_relayed`.
- Consent replay: an edge's hash is refused on another edge, on a HAM confirmation, and on the same edge after the destination's
  `licensed` flag flipped between the 409 and the create; the HAM text's hash is refused on an edge; a duplicate edge is 400.
- Restart mid-handoff (stub `hold`, `kill -9`, restart): the row reads `queued`, `uncertain: true`, `reason_code: handoff_pending`;
  not re-sent (stub log 1 → 1); a later row goes out; cancel → 409 `bad_request`; retry → 200 `ForeignSendAccepted` with a NEW id.
- fm- semantics: a pre-air `failed` row → cancel 409, retry 200 new id; a sent row → retry 409, cancel 409 `bad_request`;
  unknown/`fm-0`/`fm-1x` → 404; a never-handed queued row (adapter `error`) queues with `adapter_down` and cancels with 202.
- Component enable with a missing payload: lxmf → 409 `consent_required` (hash = sha256(notice)) → with the hash 409
  `component_unavailable` `requires_unmet`, nothing installed, `bridge/lxmf` absent, the acknowledgement recorded; madmail →
  `payload_missing`; deltachat before madmail → `requires_unmet`; unknown id 404; non-string hash 400; disable/remove-files of nothing → 200.
- PATCH /transport/posture with a client that waits: `enable_transport` on a root without `venvs/rns` → 200 `applied: false,
  reverted: true, reason "probe_failed: rnsd is not running (reverted)"` after 29 s, TOML byte-identical, no decision record left,
  the next PATCH applies; unknown region → 400 and nothing written; callsign with a bidi control → 400; `instance_name ../x` → 400;
  `region: EU_868` → applied.
- SSE: a 1,000-row channel burst → 4 `network_message` frames (per conversation, no `id`), 0 `bridge` frames; DM/own rows one
  frame per message with `id`; every frame `{network, conv, change, seq[, id]}`; edge creation publishes one `bridge {edges}` frame.
- Fan-out: preview per-target refusals, atomic 409 `oversize` with `targets[]`, unknown conv 404, empty text / no targets 400,
  bidi in a DM native id 400, LXMF syntax enforced, a conv under the wrong network 404.

## Findings

MAJOR — PATCH /transport/posture (and a device proposal's accept) is cancelled by a client disconnect mid-apply: no revert, the new
posture persists, an orphan decision record. `dashboard/api_v1/transport.rs:361-374` (`through_applier` awaits `applier::apply` inline in
the request future), `devices/applier.rs:974-1040` (snapshot → apply → probe (≤ 30 s) → revert inline), `proposals.rs:1034`
(`accept_device`). Axum drops the handler future when the client goes away. Reproduced twice (`curl --max-time 3` and 20): 45 s later
`interfaces.toml` holds `enable_transport = true`, no revert line, `agent/decisions/transport-posture-<id>.json` left `pending: true`,
GET /transport shows the new posture live. Fix: run the applier call in a spawned task the handler awaits, so the loop finishes and
reverts whether or not the response is still wanted.

minor — `transport/interfaces.rs:565`: the Backbone port validator is only the 1024–65535 range; `lan.port = 11180` (111d's own
dashboard port) is `applied: true`. Fix: refuse 111d's own ports and PortTable holders in `validate()`.

minor — `bridge/state.rs:846` `resend` refuses nothing once `meta.resent_as` is set: two retries of one crashed row sent its text twice
more (stub log 1 → 3 for one user act). Fix: refuse (409) a row whose `resent_as` is set and expose it (the recorded UI item's daemon half).

minor — `bridge/ingest.rs`: a body of only Cf characters is kept as an inbound row with `text: ""`. Fix: drop as noise or label it.

nit — a bearer POST on `//api/v1/bridge/edges` answers 405 with an empty body (the write gate is a `route_layer`; nothing executed).

inconclusive — a stub `state` change (caps only, no `Inbound::Link`) publishes no `bridge` snapshot; the product adapters emit Link items.

## Measurements (debug build, x86_64)

- 111d idle, no component, scenario stubs on: RSS 45.5 MB, 9 threads, 16 fds, listeners 127.0.0.1:11180 + :80.
- after the component-enable attempts: 48.3 MB; after the 1,000-row flood: 49.6 MB; 10 s idle later: 49.6 MB.
- G7 root (real 111-agent): 111d 44.9 MB, 45.2 MB after +80 s idle, 10 threads, 17 fds; 111-agent 8.9 MB.
- G3 (lxmf enabled, sampled from the host every 3 s): 111d max 47.5–49.7 MB (10–12 threads); rnsd 40.6–40.8 MB (10–11 threads);
  the LXMF helper 43.9–44.2 MB (9 threads), no listener of its own; B's namespace lists 11180 + 80 only; A's adds rnsd's Backbone
  0.0.0.0:4242 and the gate's fake board (4403, 11230).
- Host listeners before and after: 0.0.0.0:2024, 127.0.0.1:33173, 127.0.0.1:36427 (the box's own).
