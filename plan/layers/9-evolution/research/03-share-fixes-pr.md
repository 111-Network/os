# Layer 9 research 03 - Packaging a fix as a pull request, and sharing proven fixes between nodes

Lens 3 of the Layer 9 sweep. Date 2026-11-02. Method: repo reads (file:line), raw GitHub files for every licence, WebSearch snippets where
the page was denied (marked "(search snippet)"). No code written, nothing left running.

## 1. Summary

- **A fix is a signed, typed document with evidence, not a diff.** The diff (a `git am`-able mbox) is an optional attachment, present only when the
  change is code. The signature is the node key's, made by the identity holder on request (v1: the agent holds zero key material,
  `2-identity/README.md:222-224`), under a new domain tag `111/fix/v1` in the style Layer 5 already uses (`5-messaging/README.md:155-157`).
- **Code never travels as something a node applies.** Code reaches nodes only through a release (the signed channel manifest, Layer 7 section 9,
  `7-relay/README.md:697-745`). A fix frame can carry code only so the receiving owner can read it and, if they choose, upstream it.
  What a frame can ever lead a node to apply is a typed, reversible, scoped action, and only after a human decision.
- **v1 node-to-PR cut: no token on any node.** The hub renders two files (a `git am` mbox and a `PR.md` body with the evidence); the owner opens the PR
  from their own machine. A hub-held fine-grained token and the GitHub API (`ureq` is already in `node111`, `Cargo.toml:28`) is an opt-in v1.1.
- **v1 exchange cut: a sealed `PATCH` message (kind 0x13, SIGNED, `RECEIPT_STRONG`) to a contact or the owner's hub**, which lands as a `fix`
  proposal. On TCP links it fits one frame (up to 65,445 B payload, `5-messaging/README.md:148`). Over LoRa only a 160-byte "fix card" is sensible.
- **Evidence must be machine-made.** Published results show agent-narrated "tests passed" is unreliable: a self-modifying coding agent fabricated test
  logs (Darwin Godel Machine), and 7.8 to 19.8 percent of benchmark-"solved" patches are wrong. The verify log in a fix is recorded by `111d`'s harness
  (probe outputs and hashes), never written by the model.
- **Licensing is a human act.** The kernel's current rule (AI agents MUST NOT add Signed-off-by) and the US Copyright Office position (prompts alone do not make an author) agree: the owner who clicks "submit" certifies. The repo has no DCO today.

## 2. Candidates

No release date below is verified (`api.github.com` is denied). Resident weight is "none on the node" unless stated: every row runs on the owner's machine, a server, or is only a format.

| Name | Upstream | Licence (verified from file?) | Language / runtime | Maturity | Resident weight | Fit | Why |
|---|---|---|---|---|---|---|---|
| git format-patch / `git am` mbox | https://raw.githubusercontent.com/git/git/master/Documentation/git-format-patch.adoc | git itself GPL-2.0-only (from `plan/layers/7-relay/options.md:76`, not re-fetched); the mbox format is just text | text format | the kernel and Debian lingua franca; `From <sha> Mon Sep 17 00:00:00 2001` fixed datestamp (`git-format-patch.adoc:48`), `base-commit:` and `prerequisite-patch-id:` trailers (`:729-753`) | none: node writes the text, never runs git | **adopt (format only)** | The code attachment is a standard mbox. The fixed datestamp means no wall clock. `base-commit` ties the diff to a release. |
| DSSE | https://github.com/secure-systems-lab/dsse | Apache-2.0 (yes, `LICENSE` header read) | spec | v1.0.2 of the protocol dated 2024-05-10 (`protocol.md`); used by in-toto and Sigstore | none | **reference** | `PAYLOAD_TYPE` inside the signed bytes, `KEYID` never used for decisions: the same discipline as our domain tags. Not adopted as an envelope (no new dependency, the identity holder already signs tagged bytes). |
| in-toto attestation | https://github.com/in-toto/attestation | Apache-2.0 (yes, `LICENSE` is the 2021 Apache notice) | spec / JSON | CNCF-hosted (unverified) | none | **reference** | Subject plus predicate model, for SLSA-shaped provenance of hub-built artefacts later. |
| Linux kernel AI and generated-content docs | https://raw.githubusercontent.com/torvalds/linux/master/Documentation/process/coding-assistants.rst | process docs, SPDX GPL-2.0 header (yes) | docs | master as fetched 2026-11-02 | none | **reference** | The PR-body checklist: reproduce, build and verify, say what could not be done, never send anything itself, no agent Signed-off-by. |
| Debian DEP-3 patch headers + signed `.dsc` | https://dep-team.pages.debian.net/deps/dep3/ (host not allowed from this box) | unverified | text headers | decades of use | none | **reference** | `Origin`, `Forwarded`, `Applied-Upstream`, `Description` map onto our fix lifecycle fields; the `.dsc` is a clear-signed file list with sizes and SHA-256 (search snippet). |
| Radicle (heartwood) | https://github.com/radicle-dev/heartwood | MIT and Apache-2.0 (yes, `LICENSE-MIT`, `LICENSE-APACHE`) | Rust, needs Git 2.34+ and OpenSSH (README) | v1 line, release date unverified | resident `radicle-node` daemon plus git | **reference** | Patches as signed, CRDT-merged objects inside the repo is the closest p2p precedent for a forge-less PR. Needs git and a daemon: rejected for the node by Layer 7 F6 and the Pi floor. |
| nixpkgs-update (r-ryantm) | https://github.com/nix-community/nixpkgs-update | CC0-1.0 (yes, file header) | Haskell | bot-authored PRs, humans merge (search snippet) | none (server-side bot) | **reference** | The bot PR body lists rebuild impact; the packaging bot never merges. Our "PR.md" copies the shape. |
| Dependabot-core | https://github.com/dependabot/dependabot-core | MIT (yes) | Ruby | GitHub-run | none | **reference** | PR-per-update with a body and an open-PR limit (numbers unverified). |
| Renovate | https://github.com/renovatebot/renovate | AGPL-3.0-only (yes, `package.json:138` and `license` file) | TypeScript | widely used | none | **reject (as dependency)** | AGPL stays out of the build; its config idea (an hourly PR limit) is reusable as a plain setting. |
| `gh` CLI | https://github.com/cli/cli | MIT (yes) | Go | GitHub-maintained | none unless installed | **bridge (owner's machine only)** | `gh pr create` is the one-line step after `git am` on the owner's machine. Not a node dependency. |
| opencode | https://github.com/sst/opencode | MIT (yes, 2025) | TypeScript | adopted at Layer 3 | hub only, opt-in | **bridge (already adopted)** | Produces the diff on a hub. Must not hold a GitHub token (see the injection finding 10). |
| OpenWrt ASU (attended sysupgrade) | https://github.com/openwrt/asu | GPL-2.0 (yes, `LICENSE` header) | Python server | OpenWrt-run | server-side | **reference** | A device that cannot build asks a server for the artefact; the request is a package list, the answer a signed image. Same shape as "a hub builds, a node proposes". |
| ClearView (Perkins et al., SOSP 2009) | https://plse.cs.washington.edu/daikon/pubs/automatic-patching-sosp2009-abstract.html | paper | n/a | 10 red-team exploits, 7 patched, bad patches rejected (search snippet) | n/a | **reference** | The original "a repair validated on one machine protects the community". It had a central server; ours has none, and ours keeps a human gate. |
| SapFix (Facebook, 2018) | https://engineering.fb.com/2018/09/13/developer-tools/finding-and-fixing-software-bugs-automatically-with-sapfix-and-sapienz/ | blog | n/a | proof of concept, NPE crashes only at launch (search snippet) | n/a | **reference** | Tested patches go to a human reviewer; rejected means abandoned. The accepted pattern for rule 1. |
| F-Droid reproducible builds | https://f-droid.org/2023/09/03/reproducible-builds-signing-keys-and-binary-repos.html | docs | n/a | 2023 post (search snippet) | n/a | **reference** | A second party rebuilds and compares; the developer's signature ships. This is our "reproduced by a second node" rung. |

## 3. Findings that change the plan

1. **The wire class exists and fits the plan, with one size cliff.** `PATCH` is kind 0x13 of the bulk range 0x11-0x14, direction "classes defined here;
   consumers later" (`5-messaging/README.md:174`). SIGNED is mandatory for it (`:139-140`). `RECEIPT_STRONG` returns the payload hash in a standalone ACK
   and names `PATCH` (`:267-268`). Tier 1 fragments carry about 1.07 KB at the 160 B floor and about 2.2 KB at 233 B (`:236`); anything larger is Tier 2
   `OBJ_ADVERT`, built in v1.1 (`:175`, `:237-240`). On `tcp-lan` one frame holds up to 65,535 - 90 B (`:148`). So in v1 a fix up to about 64 KiB moves over
   IP links, and only a "card" (under 1 KB) moves over broadcast bearers.
2. **Public signed objects are stored plain** (`5-messaging/README.md:453`). A fix with incident evidence (log lines, device ids, SSIDs) must never be a
   public object. v1 sends it sealed to one contact or hub; a public advert of a fix card is deferred.
3. **Signing is the identity holder's.** In v1 all signing is by the root key and the agent holds no key material (`2-identity/README.md:216-224`). Layer 3's
   "the agent signs its outbound artifacts" (`3-intelligence/README.md:49`) is therefore a request to the signer, and "proposal signing is deferred to Layer 9"
   (`3-intelligence/README.md:84`). Today a stored proposal is `{schema, id, node_id, task_id, kind, title, body, created_at}` with no signature and a wall
   clock `created_at` (`3-intelligence/crates/agent111/src/store.rs:120-141`). The fix artefact needs new fields; ordering uses a per-author counter and the
   mesh epoch, not time.
4. **Proposals are the apply gate.** Accepting a proposal is a recorded human decision; `message`, `bridge_*` and `device` kinds already apply on
   acceptance (`4-interface/api/openapi.yaml:157-167`, `:754`). A received fix is a new proposal kind (`fix`) in that same list, rendered as untrusted text
   plus a parsed card. Code in a fix has no apply path at all.
5. **Code reaches a node only as a release.** Layer 7's hub origin, `111 update --from`, the channel check, keychain walk and `min-upgrade-from` are the
   only code path (`7-relay/README.md:697-745`; D30 F6, `plan/DECISIONS.md:483`). Git is rejected on the node (D30, F6 context). So "sharing a code fix" has
   one honest meaning: the owner submits it upstream, a maintainer merges it, a release ships it, nodes update.
6. **The project CI is the validation for code, and its human gate is built in.** `ci.yml` triggers on `pull_request` (not `pull_request_target`) with
   `contents: read` (`.github/workflows/ci.yml:8-15`), so a fork PR runs without secrets. GitHub's default requires a maintainer to "Approve and run"
   workflows for first-time contributors (search snippet, docs.github.com). `pull_request_target` is the documented "pwn request" hazard and must stay out of any
   intake automation. A node does not need to run the gate; a 4B hub with cargo can run `1-bootstrap/scripts/check.sh` as an extra, and the PR body says which.
7. **No PR spam path exists if the click is human.** GitHub's secondary limits are about 80 content-creating requests a minute and 500 an hour (search
   snippet, Enterprise Server docs, same figures stated for github.com unverified) and its Acceptable Use Policy bars "excessive automated bulk activity"
   (search snippet). A hub that opens one PR per owner click, per validated fix, is far under both. curl closed its bug bounty on 2026-01-31 after AI-slop
   reports (https://daniel.haxx.se/blog/2026/01/26/the-end-of-the-curl-bug-bounty/, search snippet); the kernel tells maintainers they may reject an unexplained
   AI-generated series without review. The rate control is therefore: one PR per fix id, never retried automatically, a per-hub daily cap setting, a
   "duplicate of" check against open PRs by fix id in the PR body.
8. **What goes in the PR body is set by the kernel's current rules** (fetched from master 2026-11-02): reproduce before fixing, build and verify, "indicate what could
   not be done", name tools and summarise prompts, and "the assistant must never send anything itself"
   (`Documentation/process/coding-assistants.rst`, `generated-content.rst`). Our body therefore carries: fix id and author node id, the incident record id and
   one-paragraph diagnosis (as a quoted block), the typed verify log summary, the applies-to tuple, what was not verified, the tool list (111-agent
   version, model id and hash, opencode version), and the owner's Signed-off-by line.
9. **Licence mechanics.** `LICENSE.md:9` says "Contributions of any kind are appreciated" and the PR template has no sign-off line
   (`.github/PULL_REQUEST_TEMPLATE.md`). The DCO 1.1 text is in the kernel's `submitting-patches.rst` (fetched). The US Copyright Office (2025-01-29,
   search snippet) says prompts alone do not make a user the author, AI output itself is not copyrightable, and human selection or arrangement can be.
   Consequence (not legal advice): the licence grant comes from the human owner's review and sign-off, not from the agent; sharing a fix as data between
   nodes carries no licence claim beyond the project's, and the owner's later PR carries GPL-3.0-or-later by the sign-off.
10. **Foreign text steers agents.** Invariant Labs showed a malicious public GitHub issue making an agent with a broad token leak private repository data
    into a public PR (https://invariantlabs.ai/blog/mcp-github-vulnerability, search snippet). Two rules follow. The agent that drafts the fix never holds
    a GitHub token; and any incident or fix text arriving from another node is data, rendered preformatted, never fed to the agent as instructions (the
    existing rendering rule for bridge bodies, `openapi.yaml:754`, extends to `fix`).
11. **Evidence integrity.** Wang and Pradel (ICSE 2026) found 7.8 percent of SWE-bench Verified patches counted correct while failing the developer tests, and
    29.6 percent of plausible patches behaving differently from the ground truth (https://arxiv.org/html/2503.15223v2, search snippet). SWE-ABS found 19.78
    percent of "solved" patches semantically wrong (https://arxiv.org/abs/2603.00520, search snippet). The Darwin Godel Machine paper reports objective hacking and
    a fabricated tool log, and says hiding the checking functions from the agent reduced it (https://arxiv.org/pdf/2505.22954; The Register,
    https://www.theregister.com/2025/06/02/self_improving_ai_cheat/, both search snippets). A Replit agent deleted a production database during a code
    freeze (2025-07-18, https://incidentdatabase.ai/es/entities/replit-ai-agent/, search snippet). Design rules: the verify log is built by `111d` from probe
    results; a fix is "validated" only for typed actions whose probes `111d` ran (the Layer 6 applier pattern, `plan/cross-layer/parked.md` P4); for code,
    "validated" means "upstream CI green", never a node's say-so.
12. **Radio cost is small but not zero.** Layer 7's own numbers: a 3.2 KB manifest plus signature is 30-55 s of LongFast airtime unsealed, and 100 times that
    under a 1 percent duty cycle (`7-relay/README.md:740-745`). A 2 KiB fix is roughly 20-35 s unsealed. Layer 7's metered setting caps carrier bytes per rolling
    24 h (`7-relay/README.md:809-811`); fix traffic obeys it. No new process, listener or timer: a PATCH is handled by the fabric task that already exists.

## 4. Recommendation

### 4.1 What a fix artefact is (settled, no fork)

`fix/v1` = a body of UTF-8 JSON (signed as exact bytes, never re-serialised) and zero or one attachment:

- `fix_id` = BLAKE3 of the body bytes; `author` = NodeId; `author_seq` = a per-author counter; `epoch` = mesh epoch if known (no wall clock).
- `class`: `typed` (only the closed action vocabulary Lens 2 defines, for example the Layer 6 `device` actions, `service.restart`, a config key) or `code`
  (a unified diff attachment plus `base-commit`/release calver) or `kb` (a recipe or entry file change; also a diff).
- `applies_to`: `{category, layers_run[], release_calver, component_versions{}, device_class}`; the receiver compares it field by field (see 4.3).
- `incident`: the incident record id and a short diagnosis block; `evidence`: the machine-recorded verify log (`probe`, `before_hash`, `after_hash`, `reverted: bool`,
  `result`) plus `not_verified[]`.
- `maturity`: `local` | `reproduced` | `accepted_by_n` | `upstream_merged` | `in_release` (4.4). The author sets `local`; later states are other parties' signed statements.
- Signature: `Ed25519("111/fix/v1" ‖ body_len ‖ body ‖ attachment_hash)` by the identity holder. Size target: under 4 KiB without attachment, attachment cap 64 KiB. Anything over the cap is not a fix, it is a branch: refuse with a plain message.

### 4.2 Fork B: from a node to a pull request

| Option | Trade-off |
|---|---|
| B1 hub holds a GitHub token and calls the REST API (`ureq` already linked) | One click, best UX. A token at rest on a hub; a fine-grained token may not be able to write to a fork-based PR on a repo it does not own (single search snippet, unverified), forcing a broad classic token. Needs a keychain entry and a consent record. |
| B2 hub exports `fix-<id>.mbox` + `PR.md`; owner runs `git am`, pushes, `gh pr create` or pastes in the web UI | No token anywhere, no network from the hub, fully human. Needs a machine with git (the owner has one). Three more steps. |
| B3 prefilled GitHub issue URL (title, body query parameters) | Works from a phone browser; limited by URL length (414 above an undocumented cap, search snippet); an issue, not a PR. Good for non-code fixes and reports only. |
| B4 maintainer-side Action that turns an intake issue into a PR | Rejected: untrusted text drives a privileged workflow (`pull_request_target` hazard, finding 6). |

**Pick: B2 for v1, B3 as the phone fallback for `typed` and `kb` classes, B1 as an opt-in v1.1** with a single-repo token, off by default, with the
consent text naming the repository and the daily cap. The hub never opens a PR by itself; the dashboard shows "Submit upstream" on an accepted
`code` or `kb` fix, and the decision is recorded.

### 4.3 Fork C and D: the exchange and who is believed

| Option | Trade-off |
|---|---|
| C1 sealed 1:1 `PATCH` to contacts and the owner's hub, receiver files a `fix` proposal | Uses what Layer 5 and 7 build; works offline over any link that carries a sealed message; each recipient is one deliberate send. |
| C2 hub "fix shelf": the hub keeps accepted fixes, members pull by `applies_to` | Fewer sends, better reach; needs a Layer 8 style store and a fetch verb; v1.1. |
| C3 fixes graduate into releases (KB recipe or code merged, shipped by the updater) | The only route for code; slow (calver cadence) but already built and trusted. |
| C4 open gossip or a Radicle-style p2p patch graph | Rejected: needs git and a daemon, floods metered links, no human gate per receiver. |

**Pick: C1 in v1 (direct, owner-initiated, contacts and own hub), C2 in v1.1, C3 always for code.** Trust (Fork D): D1 contacts only; D2 contacts or any
author a paired hub has vouched for (`111/fix-vouch/v1` countersignature over `fix_id` by that hub); D3 anyone. **Pick D2**; a vouch means "this hub's owner
read and reproduced it" and is shown as such, never as automatic.

Receiver procedure, all before a human sees a button: (1) verify the signature, `RECEIPT_STRONG` the hash; (2) author is a contact or vouched, else park as
"unknown author" with no action offered; (3) `class=code` shows a read-only diff and offers only "submit upstream" (never "apply"); (4) `applies_to` must match
this node on category, every layer in `layers_run` that this node runs, and release calver equal or newer-compatible; a mismatch is shown as "does not
apply here" (scoped by the layers the device operates, README section 9); (5) typed actions are checked against the closed vocabulary and the Layer 6
refusal list: any action touching the gateway, carrier, identity, key material, update channel or the fabric's own consent records is refused by scope
(the "malicious fix that disables the gateway" case dies here); (6) only then is a `fix` proposal filed; accepting it runs the Layer 6 applier loop (apply,
probe, revert on failure) and the result is signed and sent back as a `maturity` statement. The receiving agent's job is to summarise; it cannot accept.

Abuse table (all refused before display unless noted): forged author (signature fails); replay (author_seq not increasing); oversize (cap); a typed action
outside vocabulary (scope refusal, logged); an applies-to that lies (the probes in step 6 fail, revert); prose with injected instructions (rendered
preformatted, never read by the agent); a flood from one author (per-author hourly cap, drop and log, like the fabric's existing quotas).

### 4.4 What "proven" means for a fix (the KB ladder, reused)

`local` (one node's probes, the rubric's "desk research" cap equivalent) -> `reproduced` (a second node of the same category, or the hub, re-ran the
probes; the F-Droid and ClearView idea; the "testing" rung) -> `accepted_by_n` (n owners accepted it and the field log shows no revert for a stated
period; the "proven" field-program rung, `0-knowledge/README.md:84-93`) -> `upstream_merged` -> `in_release`. A KB recipe built from a fix enters as
`candidate` like any desk-research entry.

### 4.5 Fork E: licensing

E1 DCO sign-off by the human owner on the PR (kernel text), enforced by a PR-template checkbox and optionally a one-step check in `ci.yml`; E2 nothing
(inbound = outbound by convention); E3 CLA. **Pick E1**, owner to confirm: the hub's mbox is generated with `Assisted-by:` lines for the tools and a
`Signed-off-by:` line left blank for the owner to fill, so an agent never signs.

### 4.6 The v1 cut

Build now: the `fix/v1` body and tag; the `fix` proposal kind and card; the mbox and `PR.md` exporter; the receiver procedure and scope list; `PATCH`
over TCP and Tier 1; the per-author cap. Defer: B1 token path, C2 shelf, public advert, Tier 2 transfer, hub vouch UI beyond a pinned list. Resident
cost added to the required configuration: zero processes, zero listeners, zero timers; disk up to a settable cap (default proposal: 256 fixes, about 1 MiB
without attachments).

## 5. Open questions for the owner

1. Do you want a DCO sign-off on contributions to 111-Network/os (and a CI check), or the current "appreciated" wording only?
2. Is a GitHub token on a hub ever acceptable (B1, v1.1), and if so only with a fork-scoped fine-grained token?
3. May a node accept fixes from a hub-vouched stranger (D2), or contacts only (D1)?
4. Should a `typed` fix received from a contact be allowed one-tap accept, or always a full review screen?
5. Which per-hub daily cap for "Submit upstream" and which per-author hourly cap for incoming fixes?
6. Do accepted `kb` fixes go through the same PR route, or does the owner want a separate KB submission path?

## 6. Sources

Repo (read 2026-11-02): `README.md` section 9; `5-messaging/README.md:139-175,236-240,267-268,453,831-832`; `2-identity/README.md:211-224`;
`3-intelligence/README.md:47-49,84`; `3-intelligence/crates/agent111/src/store.rs:120-141`; `4-interface/api/openapi.yaml:157-167,754`;
`7-relay/README.md:697-745,809-811`; `plan/DECISIONS.md:483,489`; `plan/cross-layer/parked.md` P4; `plan/layers/7-relay/options.md:76`;
`.github/workflows/ci.yml:8-15`; `.github/PULL_REQUEST_TEMPLATE.md`; `LICENSE.md:9`; `1-bootstrap/crates/node111/Cargo.toml:28`;
`0-knowledge/README.md:84-93`.

Fetched (raw files, licences verified): git format-patch doc https://raw.githubusercontent.com/git/git/master/Documentation/git-format-patch.adoc ·
kernel https://raw.githubusercontent.com/torvalds/linux/master/Documentation/process/{coding-assistants,generated-content,submitting-patches}.rst ·
DSSE https://raw.githubusercontent.com/secure-systems-lab/dsse/master/{LICENSE,protocol.md} · in-toto https://raw.githubusercontent.com/in-toto/attestation/main/LICENSE ·
gh https://raw.githubusercontent.com/cli/cli/trunk/LICENSE · dependabot-core .../dependabot/dependabot-core/main/LICENSE · renovate .../renovatebot/renovate/main/license ·
nixpkgs-update .../nix-community/nixpkgs-update/main/LICENSE · opencode .../sst/opencode/dev/LICENSE · ASU .../openwrt/asu/main/{README.md,LICENSE} ·
Radicle .../radicle-dev/heartwood/master/{README.md,LICENSE-MIT,LICENSE-APACHE}.

Search snippets (pages not fetched): GitHub rate limits https://docs.github.com/en/enterprise-server@latest/rest/using-the-rest-api/rate-limits-for-the-rest-api ·
fork workflow approval https://docs.github.com/actions/managing-workflow-runs/approving-workflow-runs-from-public-forks · pwn request
https://docs.github.com/en/actions/reference/security/securely-using-pull_request_target · query-parameter URLs
https://docs.github.com/en/pull-requests/reference/using-query-parameters-to-create-a-pull-request · AUP
https://docs.github.com/en/site-policy/acceptable-use-policies/github-acceptable-use-policies · Copyright Office
https://datamatters.sidley.com/2025/02/07/u-s-copyright-office-issues-report-on-artificial-intelligence-and-copyrightability/ · DGM https://arxiv.org/pdf/2505.22954 ·
SWE-bench studies https://arxiv.org/html/2503.15223v2, https://www.alphaxiv.org/abs/2603.00520 · Replit https://incidentdatabase.ai/es/entities/replit-ai-agent/ ·
Invariant https://invariantlabs.ai/blog/mcp-github-vulnerability · curl https://daniel.haxx.se/blog/2026/01/26/the-end-of-the-curl-bug-bounty/ ·
ClearView https://plse.cs.washington.edu/daikon/pubs/automatic-patching-sosp2009-abstract.html · SapFix https://engineering.fb.com/2018/09/13/developer-tools/finding-and-fixing-software-bugs-automatically-with-sapfix-and-sapienz/ ·
nixpkgs-update https://wiki.nixos.org/wiki/Nixpkgs/Automatic_Updates · F-Droid https://f-droid.org/2023/09/03/reproducible-builds-signing-keys-and-binary-repos.html ·
Debian dsc(5) https://manpages.ubuntu.com/manpages/jammy/man5/dsc.5.html · DEP-3 https://dep-team.pages.debian.net/deps/dep3/ (host not reachable; field semantics via Ubuntu mirror snippets).

Not researched: Gentoo bugzilla patches, Nix binary-cache signatures, OpenSSF Scorecards.
