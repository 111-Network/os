export const meta = {
  name: 'test-system-x4c-x1c',
  description: 'Test system rows X4c (the X4b review findings in lint.py and matrix.py) then X1c (the seam map and check catalog: the re-review items and the six unmapped files), in sequence; the lead checks and commits each',
  phases: [
    { title: 'X4c', detail: 'lint and matrix fixes', model: 'sonnet' },
    { title: 'X1c', detail: 'the map and the catalog', model: 'sonnet' },
  ],
}
const ROOT = '/home/user/os'
const SP = '/tmp/claude-0/-home-user-os/f8991dd8-132d-5f0d-807d-446e0e7eb394/scratchpad'
const TS = `${ROOT}/plan/cross-layer/test-system`
const RESULT = {
  type: 'object',
  required: ['task', 'status', 'files', 'check', 'deviations', 'notes'],
  properties: {
    task: { type: 'string' }, status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files: { type: 'array', items: { type: 'string' } }, check: { type: 'string' },
    deviations: { type: 'array', items: { type: 'string' } }, notes: { type: 'string' },
  },
}
const CTX = `You are a builder in ${ROOT} (the 111network mesh OS), branch claude/confident-ramanujan-y5ob49, working on the cross-layer test system
(plan/DECISIONS.md D36, D37): ${ROOT}/verify (read verify/README.md first; the runner is verify/run.py and verify/t111/, the seam map
verify/seams.toml, the check catalog verify/checks.toml). Its board is ${TS}/TASKS.md and its design ${TS}/options.md. This system exists so
that an agent can trust a GREEN and act on a RED: no change may let a real failure read as PASS or SKIPPED.
RULES: Python standard library only; no third-party import; no shell=True with interpolated text; no model names anywhere; run python with
-I -B (leave no __pycache__ in the tree); scratch only under ${SP}. Do NOT run cargo, pnpm, any shell gate or any namespace harness: other
agents are building Rust on this box. Other agents are editing other parts of the repo (1-bootstrap, 8-storage, 9-evolution, plan/): never
touch their files; never run git stash, checkout, reset or commit. Never weaken an existing assertion. Leave no process running.
CHECKS: cd ${ROOT} && python3 -I -B -m unittest discover -s verify/tests -t .   (320 tests green now)   and   python3 -I -B verify/run.py --lint
RETURN (structured): task id, status, files changed or created (exact paths), the check commands with their last lines, every deviation
with its reason, notes (names other rows may rely on; anything found outside the row).`

phase('X4c')
const x4c = await agent(`${CTX}

TASK X4c (row "| X4c |" of the board; read it and row "| X4b |" whole). Row X4b landed at f017664; an independent review left one medium
and seven low findings: ${SP}/results/X4b-review.json (read \`findings\` and \`coverage_gaps\` whole: each has file:line, a scenario and a fix).
Fix all eight, each with a test that fails on the code as it is now (say for each how you saw it fail):
1 (medium) pi-floor: os.path.join(..., "..", "..", "verify") and Path(...) / ".." / "verify" are hits (literals made only of dots, and
  os.pardir, are not earlier path parts); a glob of the root directory ("verify/*", "verify/**", "verify/[a-z]*") is a hit.
2 a bold lead is a numbered section anchor only when the number is followed by ".", by a space and a capitalised word or "(", or by the end
  of the lead: "**3 nodes** is the minimum" and "**1-bootstrap/README.md**" are not anchors.
3 rule known: when a [[known]] names both \`check\` and a \`test\` prefix, the prefix must match that check (fnmatch), else FAIL.
4 rule known: a \`source\` that is absolute or leaves the tree is a FAIL.
5 a cargo check with --bench / --benches / --bench= claims no integration test file; --test with a glob value claims the files whose stem
  matches it (fnmatch).
6 pi-floor: a \`verify\` word that is the value of --exclude, --exclude=, --filter or sits before -prune is not a hit (an exclusion keeps
  verify/ OUT of a payload).
7 matrix: a run record whose \`via\` is not a mapping or list as expected, or whose \`tiers\` is not a list, is a named error (exit 2), not a
  traceback.
8 the stub-marker warning expands glob and directory side files (files only) before it reads for the T0 marker.
FILES: verify/t111/lint.py, verify/t111/matrix.py, verify/tests/test_lint.py, verify/tests/test_matrix.py, verify/tests/fixtures/lint/**.
Do NOT edit verify/seams.toml or verify/checks.toml (the next row does); the real map's lint is RED on six unclaimed files and stays so
after your row: your pass mark is the unit suite green and the lint's FAIL lines unchanged apart from what your fixes legitimately change
(list any change).`, { label: 'TS:X4c', phase: 'X4c', schema: RESULT, model: 'sonnet', effort: 'high' })
log('X4c: ' + (x4c ? x4c.status : 'no result'))

phase('X1c')
const x1c = await agent(`${CTX}

TASK X1c (row "| X1c |" of the board; read it whole, with rows "| X1 |", "| X1b |", "| X3 |", "| X4b |" for the as-built names). The full
re-review this row answers: ${SP}/results/X1b-review.json (read it whole). You edit the DATA: verify/seams.toml and verify/checks.toml
(and the one golden named below). The row's items:
a. no \`requires\` on l1.gate; l3.agent111.lib has layers = [3]; each F check's own fake comes first on its primary seam; --lib and UI-gate
   records are mapped by module path / phase through X3's contract (read verify/t111/adapters.py's module docstring for the record ids);
   lock = "ui-dist" on the three dist-reading units; \`requires\` on the vitest units that read the contract digest.
b. promote L6-L7:netlink-gateway to built (rows D3, D4, D3b, D4c of ${ROOT}/7-relay/TASKS.md landed: read their as-built facts for the
   side files and the tests that prove the seam: relay/netwatch.rs, transport/netlink.rs, relay/gateway.rs).
c. claim the six files the lint names as unclaimed (run --lint to see them): 1-bootstrap/crates/node111/tests/{origin_serve,relay_tls,
   relay_pair}.rs and 3-intelligence/crates/agent111/tests/layer7_tools.rs each get a unit check (the exact cargo command a lead runs:
   read the board rows D7, D5, D5b, D6, A1 of 7-relay/TASKS.md for it), covered by its gate, with the seams it proves (for example the
   hub origin, the pinned-key TLS link, pairing, the agent's Layer 7 tools: add a seam where the map has none, with both side files, the
   spec section in 7-relay/README.md and state built or stub as the board says; relay_pair.rs belongs to row D6, which is still being
   built: map it as its board row says and mark the seam stub until D6 lands). The two Layer 6 harness helpers
   (6-transport/verify/devices/fake-rnodeconf.sh, 6-transport/verify/spine/node.sh) are helpers of existing harnesses: make them side
   files of the check that sources them, or list them under [unmapped] with a reason, whichever the map's own convention is.
d. regenerate the golden verify/tests/fixtures/okfail/lint-green.{txt,jsonl} so it shows the eight rules (use the suite's own switch,
   T111_REGEN=1, and read every changed line).
PASS MARK: \`python3 -I -B verify/run.py --lint\` prints LINT GREEN on the real map; --list, --matrix, --explain on two of your new seams
and on one new check run clean; the unit suite green. Report the lint's last five lines verbatim and each new seam and check id.
FILES: verify/seams.toml, verify/checks.toml, verify/tests/fixtures/okfail/lint-green.{txt,jsonl}. If the lint cannot go green without a
code change in verify/t111/, STOP that item and report it (do not edit the code).`, { label: 'TS:X1c', phase: 'X1c', schema: RESULT, model: 'sonnet', effort: 'high' })
log('X1c: ' + (x1c ? x1c.status : 'no result'))
return { x4c, x1c }
