export const meta = {
  name: 'layer-6-wave-0-spike',
  description: 'Layer 6 Wave 0 S5: the Delta Chat netns spike (deltachat-rpc-server + Madmail, IP-only, self-signed) — report only; decides whether Delta Chat stays in R-B (D26 #6)',
  phases: [{ title: 'Spike', detail: 'S5 alone; runs beside wave-0.js' }],
}

const ROOT = '/home/user/os'
const SP = '/tmp/claude-0/-home-user-os/e7852691-6eb6-50f1-9e84-b01d744ea3a9/scratchpad'

const BRIEF = `
You are a RESEARCH-SPIKE subagent for the 111network OS, Layer 6 (Transport & Bridge). Today is 2026-10-19. Repo root: ${ROOT}
(a git repo; you NEVER run git commit/add/stash/checkout). You will not be messaged mid-task.

TASK S5 — the Delta Chat spike (spec ${ROOT}/6-transport/README.md §7 "Spike first", §2 ports, §15.10 egress; D26 #6; your row in
${ROOT}/6-transport/TASKS.md). Read §7 and §2 first, then ${ROOT}/plan/layers/6-transport/research/03-phone-apps.md §3.9–§3.11 (the
research facts: deltachat-rpc-server speaks JSON-RPC 2.0 over stdio, Config::Bot, Secure-Join; Madmail is the Rust single-binary chatmail
relay with an IP-only self-signed mode). The spike is evidence, not a build: nothing you make ships.

GOAL: in network namespaces with NO internet, run deltachat-rpc-server (the official static linux x86_64 release binary from the chatmail
core GitHub releases — find the exact asset and pin its URL, tag and sha256) and Madmail (its release binary or, failing that, a cargo build
from its pinned tag — record which) in IP-only self-signed mode; create two headless accounts on it, perform Secure-Join between them
(the QR/invite text one account produces, consumed by the other), send a message each way, observe the message states the core reports
(pending → delivered/read receipts), log every egress attempt (strace -f -e trace=connect on both programs, and a packet count on the
namespace's interfaces), and record the exact ports Madmail serves (SMTP/IMAP/HTTPS) and whether they need CAP_NET_BIND_SERVICE (they will
be < 1024 unless Madmail lets you move them — record both facts).

BOX: 4 CPUs, 15 GB RAM, ~20 GB free disk; unprivileged netns work (unshare -Urnm) but there is NO \`ip\` binary (no veth): run BOTH
programs on loopback inside ONE netns (that still proves "no internet" — record this as the spike's shape; the two-netns veth form is the
runbook's). Outbound HTTPS goes through a proxy (curl works; GitHub releases and crates.io are reachable; use curl -L -o, verify sha256).
python3 3.13 is present (a tiny JSON-RPC driver over the rpc-server's stdio is fine; no pip packages needed — write stdlib code), cargo 1.97
(a Madmail cargo build, if needed, goes to CARGO_TARGET_DIR=${SP}/S5/target with CARGO_BUILD_JOBS=2; delete it when done), strace, jq, nc.
Load WebFetch/WebSearch via ToolSearch when you need upstream docs. Time-box: if Madmail cannot be obtained or does not start in IP-only
mode within ~60 minutes of trying, STOP and report that as the failure with the exact error — a failed spike is a valid, useful result
(D26 #6: Delta Chat then leaves the first release). Scratch under ${SP}/S5/ only; delete it at the end. Linux hygiene: nothing of yours
stays running when you return (ps check; kill what you started), the netns dies with its shell.

DELIVERABLE (the only file you write in the repo): ${ROOT}/plan/layers/6-transport/research/08-spike-deltachat.md — the spike report in the
style of the other research reports there (read 07-gap-1.md for the shape): header with date and the exact pins (URLs, tags, sha256 of every
binary), what ran and how (commands), the results table (each step: ran / pass / fail with evidence), the exact ports and the capability
verdict, the message-state mapping observed (what the core reports for queued/sent/delivered/read), the egress log result (zero off-link
connects expected), the residuals (what a phone would still need: the attended gate), and a one-line VERDICT: PASS (Delta Chat stays in
R-B: the C5 row builds against these ports) or FAIL (Delta Chat leaves R-B; name the cause). Also record anything about the rpc-server's
stdio protocol the C5 adapter will need (how to create an account, set the bot flag, get the Secure-Join QR, the event stream shape).

RETURN (structured): task id "S5", status done|partial|blocked, files (the report path), check (the commands that prove nothing is left
running + the verdict line), deviations, notes.
`

const RESULT = {
  type: 'object',
  required: ['task', 'status', 'files', 'check', 'deviations', 'notes'],
  properties: {
    task: { type: 'string' },
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files: { type: 'array', items: { type: 'string' } },
    check: { type: 'string' },
    deviations: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}

phase('Spike')
log('Layer 6 S5: Delta Chat netns spike')
const r = await agent(BRIEF, { label: 'L6:S5', phase: 'Spike', schema: RESULT, model: 'opus', effort: 'high' })
log('S5 returned: ' + (r ? r.status : 'null'))
return r
