export const meta = {
  name: 'layer-5-wave-3-fix',
  description: 'Layer 5 Wave 3 fix wave: P1 guard Fetch-Metadata fallback + docs rows (A), then presence venv build + V1 smoke verification with the jq -e fix (B); lead runs panel-lint, the gates, the lock entry',
  phases: [
    { title: 'Fix A', detail: 'P1g guard fallback (opus) · DOCS release-checklist rows + Overview copy (sonnet)' },
    { title: 'Fix B', detail: 'VENV presence venv build in the installer (opus) · V1v smoke verification + jq -e audit (opus xhigh)' },
  ],
}

const ROOT = '/home/kia/projects/111network/os'
const SCRATCH = '/tmp/claude-1000/-home-kia-projects-111network-os/af41ab70-058e-4e3c-bc5e-eec7688f5c7a/scratchpad'
const J = `${ROOT}/plan/layers/5-messaging/runs/journals`
const HANDOFF = `${ROOT}/plan/HANDOFF-2026-10-15.md`

const CTX = `
You are a BUILD/FIX subagent for the 111network OS, Layer 5 (Messaging & Data), Wave 3 fix wave. Today is 2026-10-19.
Repo root: ${ROOT} (git; you NEVER run git commit/add/stash/checkout/reset/clean — the lead reviews and commits). You
will not be messaged mid-task; everything you need is in this brief and on disk.

STATE: Waves 0–2 are built and their review bounces have landed on main (read ${HANDOFF} §1–§3 for the map, then the
Wave 2 section of ${ROOT}/plan/layers/5-messaging/review/wave-2.md). THE LAW is the locked spec ${ROOT}/5-messaging/README.md
(v1.2 + as-built amendments) and ${ROOT}/plan/DECISIONS.md D19–D21. The exported run journals (one JSON line per finished
agent; read with python3 -I) are under ${J}/: wave-2-build.*.jsonl, wave-2-review.*.jsonl, wave-2-bounce.*.jsonl,
wave-3-verify.*.jsonl (V2 panel lint: its result.gaps list is the defect list this wave closes).

FILE OWNERSHIP IS STRICT: only the files your brief lists (plus new files inside those dirs). One other agent runs in
parallel on other files; a read-only Layer 6 research sweep may be reading the tree (ignore it). A change you need
elsewhere: do not make it; describe it under "deviations".

HOUSE RULES: Rust edition 2021, typed errors, no panics on input, doc comments citing the spec, rustfmt + clippy -D
warnings, no new dependencies; UI: TypeScript strict, Tailwind tokens, D21 budget (256 KB eager + 256 KB lazy, 64 KB
per lazy chunk); shell: shellcheck clean, every check prints ok:/FAIL:/note: lines and exits 1 on any FAIL. Toolchain:
export PATH="$HOME/.cargo/bin:$PATH". THIS BOX HAS 12 GB RAM and killed three sessions under parallel builds: every cargo
invocation of yours uses the target dir your brief names AND CARGO_BUILD_JOBS=6 (export both; never more jobs); never run
two cargo commands at once; no cargo clean on other dirs. Nothing downloads at runtime. No Go. Never commit.

RUN YOUR CHECK until green; paste its tail. RETURN (structured): task id, status (done | partial | blocked), files
changed, the check command + tail (≤ 30 lines), findings_addressed (one line per item), gaps (what could not be
asserted or fixed, where, suspected owner), deviations, notes for the reviewer.
`

const P1G = { id: 'P1g', model: 'opus', effort: 'high', brief: `TASK P1g — close V2's gap #1 in the panel guard (spec §13 "Guards"; P1 review m1). Defect (from the V2 journal line, result.gaps[0]): ${ROOT}/1-bootstrap/crates/node111/src/daemon/panels/guard.rs check() relies on Sec-Fetch-Site alone for the API-path same-origin rule, and Chrome sends NO Sec-Fetch-* headers to http://<LAN IP> (not a trustworthy origin; verified in the CDP recording: absent on 192.168.77.5, present on 127.0.0.1). So on a LAN origin another origin on the node's host (same site: a second panel or any local service on another port) can GET the panel's /api/v1/fromradio with the victim's p111 cookie (SameSite=Strict attaches it) and gets 200 — the shim dequeues a frame the viewer never sees. Fix, exactly: on API paths, when no Sec-Fetch-Site header is present, apply the Origin-else-Referer authority rule to EVERY method including GET/HEAD — if Origin or Referer is present and its authority (host and port) is not the request authority, refuse with the existing same-origin refusal (403, no CORS); if neither header is present, allow (this is the documented residual #2: a deliberate no-referrer no-cors GET is byte-identical to the panel's own GET over plain HTTP on a LAN IP — the owner decides the spec wording; do NOT edit the spec, propose the sentence under notes). Keep every existing rule and test; the panel's own requests (Referrer-Policy: no-referrer, so no Referer, no Origin on same-origin GET) must keep working. Tests in tests/api_v1_panels.rs: a LAN-origin header matrix (no Sec-Fetch-*): foreign Referer → 403, foreign Origin → 403, Referer with the right host but another port → 403, same authority Referer → 200, no headers → 200 (named as the residual), plus the loopback cases unchanged; unit tests in guard.rs for the new branch. Also re-read the guard's module doc (lines 1–30) and update it to describe the fallback honestly.
Owned: ${ROOT}/1-bootstrap/crates/node111/src/daemon/panels/guard.rs, src/daemon/panels/mod.rs (only if the call site must pass a flag it does not pass today), tests/api_v1_panels.rs.
Check (CARGO_TARGET_DIR=${SCRATCH}/target-P1g): cd ${ROOT}/1-bootstrap/crates/node111 && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --test api_v1_panels && cargo test --lib panels. The lead then runs 4-interface/verify/panel-lint.sh, which must go GREEN without PANEL_LINT_KNOWN_GAPS.` }

const DOCS = { id: 'DOCS', model: 'sonnet', effort: 'high', brief: `TASK DOCS — two documentation follow-ups the reviewers carried (no Rust, no cargo). (1) Release checklist ${ROOT}/1-bootstrap/docs/release-checklist.md: read it and what the B1 bounce may already have added; make sure it carries rows for the Layer 5 release files — bin/meshmud (CI-built from the pinned tarball, 1-bootstrap/scripts/meshmud-build.sh), panels/meshtastic-web (1-bootstrap/scripts/meshtastic-web-build.sh), share/meshtastic-presence + its lock (meshtastic-presence-lock.txt), the Layer 5 manifest sections written by 1-bootstrap/manifest/gen-manifest.py — and that these files are LISTED in share/payload.manifest (the B1 reviewer's deviation 5: verify_tree checks only listed files, so update_guard's torn-payload check skips unlisted Layer 5 assets; read 1-bootstrap/crates/node111/src/manifest.rs and the README's payload-manifest wording to state the rule correctly); plus the D22 row: publish the tagged Corresponding Source beside every binary release (${ROOT}/LICENSING.md section 8). Keep the file's existing style. (2) Overview copy: the U2 build report (${J}/wave-2-build.*.jsonl, the U2 line, result.notes / deviations) noted stale sentences on the dashboard Overview after Layer 5; find them in ${ROOT}/4-interface/ui/src/views/overview/** and fix the copy to the as-built state (Messages, requests row, fabric service), adjusting tests. If the note names nothing concrete, grep the Overview view for pre-Layer-5 phrasing ("coming with Layer 5", "stub", "not yet") and report what you changed.
Owned: ${ROOT}/1-bootstrap/docs/release-checklist.md, ${ROOT}/4-interface/ui/src/views/overview/**, ${ROOT}/4-interface/ui/test/** (never test/tapes/**), ${ROOT}/4-interface/README.md (one as-built line only if the Overview change needs it).
Check: cd ${ROOT}/4-interface/ui && pnpm typecheck && pnpm test && pnpm build && pnpm budget; and bash ${ROOT}/hooks/pre-commit if it runs a markdown/link check (read it first; if it is not runnable standalone, say so).` }

const VENV = { id: 'VENV', model: 'opus', effort: 'high', brief: `TASK VENV — the node builds the presence venv (spec §14, §19 L1; B1 reviewer deviation 6: "Nothing builds venvs/presence yet … M1's presence child has no venv to run until then"). Pattern to copy, not reinvent: the rns venv — ${ROOT}/1-bootstrap/crates/node111/src/daemon/transport/venv.rs (pip install --require-hashes from cache/packages/python/rns-lock.txt), src/install/packages.rs and src/install/stage.rs (how the rns lock is fetched, pin-verified and staged), and whatever rebuilds the rns venv at update promotion (find it: grep venvs/rns and rns-lock across src/). Build venvs/presence (manifest.rs PRESENCE_VENV) from the lock the signed manifest pins (manifest.rs presence_lock / PRESENCE_LOCK) the same way, at install and when an update that changes the lock is promoted; fail closed (a bad hash or missing python3 leaves the previous venv in place and surfaces as the meshtastic service's reason, never as a crash); the daemon/meshtastic/mod.rs reader_python path stays; add the "venv missing / build failed" reason to the meshtastic service row only if it is not already there (M1 bounce added python3-skip visibility — read it first). Tests with a fake pip/python3 on PATH (the rns venv tests show how), covering: fresh install builds it; unchanged lock on update does nothing; changed lock rebuilds; failure keeps the old venv.
Owned: ${ROOT}/1-bootstrap/crates/node111/src/daemon/transport/venv.rs (generalising the helper is allowed; keep rnsd behaviour byte-identical), src/install/{packages.rs,stage.rs} (the presence parts; B1's verify_layer5_tree call sites are already in place — do not move them), src/update_helper.rs (only if the rns venv rebuild lives there), src/daemon/meshtastic/mod.rs (the reason string only), tests/install_flow.rs and tests/updater_stage.rs (extend), ${ROOT}/1-bootstrap/README.md (the dirs/venvs line).
Check (CARGO_TARGET_DIR=${SCRATCH}/target-VENV): cd ${ROOT}/1-bootstrap/crates/node111 && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test.` }

const V1V = { id: 'V1v', model: 'opus', effort: 'xhigh', brief: `TASK V1v — verify and finish the three-node smoke (spec §20 G1). The V1 agent (${J}/wave-3-verify.*.jsonl has only V2; V1 never returned its report — the handoff §2 says it reached "4/4 green (38–69 s)" and "tapes gate passes (43 tests)" at its final read-through) built the --three-node mode of ${ROOT}/4-interface/verify/smoke.sh plus helpers (stub-llama.py, tcp-lan-relay.py, stub-meshmud.py shared with V2 — its CLI roles are a shared contract, keep them) and the L5 SSE tapes under 4-interface/ui/test/tapes/. Since then the F2 bounce changed the daemon (restore-pending guard, cancelMessage, settings validation, announce re-mint) and the contract regen landed. Do, in order: (1) fix the vacuous-pass bug V2 found: jb() is 'jq -e "$@" "$W/body"' and jq -e exits 0 on EMPTY input, so JSON assertions pass on an empty body — guard with a non-empty-body test, then AUDIT every jb/t use (61 jb uses, ~210 t checks) for the same vacuity (empty captures, grep -q on empty files, "$(…)" = "" comparisons) and list which checks changed outcome; (2) run bash 4-interface/verify/smoke.sh (single-node; must keep passing) then bash 4-interface/verify/smoke.sh --three-node; read every ok:/FAIL:/note: line; (3) fix harness bugs in your files; a FAIL caused by the daemon is a gap (exact scenario, assertion, suspected owner F2/P1/M1) — do not touch daemon code; (4) if the daemon's SSE shape changed, re-record the affected L5 tapes from the real daemon (never hand-edit) and keep the tapes README true; (5) run cd 4-interface/ui && pnpm test (the tapes gate) green. Never run panel-lint.sh in parallel (both build ui/dist); the lead runs it after you. Keep the run under ~10 minutes; the smoke builds debug binaries in the default target dir (export CARGO_BUILD_JOBS=6; another agent builds in a private dir meanwhile).
Owned: ${ROOT}/4-interface/verify/smoke.sh, ${ROOT}/4-interface/verify/*.py, ${ROOT}/4-interface/ui/test/tapes/**, ${ROOT}/4-interface/verify/README.md (if it exists; the three-node mode must be documented there or in smoke.sh's header).
Check: bash ${ROOT}/4-interface/verify/smoke.sh && bash ${ROOT}/4-interface/verify/smoke.sh --three-node (both exit 0) && cd ${ROOT}/4-interface/ui && pnpm test; paste the ok:/FAIL:/note: counts and every FAIL/note line of the three-node run.` }

const RESULT = {
  type: 'object',
  required: ['task', 'status', 'files', 'check', 'findings_addressed', 'gaps', 'deviations', 'notes'],
  properties: {
    task: { type: 'string' },
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files: { type: 'array', items: { type: 'string' } },
    check: { type: 'string' },
    findings_addressed: { type: 'array', items: { type: 'string' } },
    gaps: { type: 'array', items: { type: 'string' } },
    deviations: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}

const run = (t, ph) => agent(CTX + '\n\n' + t.brief, { label: `fix:${t.id}`, phase: ph, schema: RESULT, model: t.model, effort: t.effort }).then(r => r && { ...r, id: t.id })

phase('Fix A')
log('Wave 3 fix A: P1g guard fallback ‖ DOCS')
const a = (await parallel([() => run(DOCS, 'Fix A'), () => run(P1G, 'Fix A')])).filter(Boolean)
log('A returned: ' + a.map(r => `${r.id}=${r.status}`).join(' · '))

phase('Fix B')
log('Wave 3 fix B: VENV ‖ V1v (two cargo builds at most)')
const b = (await parallel([() => run(VENV, 'Fix B'), () => run(V1V, 'Fix B')])).filter(Boolean)
log('B returned: ' + b.map(r => `${r.id}=${r.status}`).join(' · '))
return [...a, ...b]
