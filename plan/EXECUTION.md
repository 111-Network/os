# EXECUTION — how every layer gets built

Applies once a layer's spec is decided. Save work at every step; nothing lives only in chat.

1. **Break down** — split the layer into small, independent tasks. Each task gets: goal, acceptance criteria, a verifiable check (test / lint / script — for docs layers: validators like frontmatter lint or link check), and the files it owns. Recorded as a checklist in `os/<layer>/TASKS.md`.
2. **Delegate in parallel** — one subagent per task. Tasks that touch the same files never run in parallel; the file-ownership column in TASKS.md decides what runs concurrently. Model per roster (default k3-256k; bulk/mechanical → kimi-for-coding).
3. **Review yourself** — for every returned task: run the check, read the diff, compare against each criterion. Fail → send back to the same subagent with specific fixes. No rubber-stamping.
4. **Commit each verified task** — one commit per task, short imperative message, in the `os/` repo. **Never** add Claude co-author or "generated with" attribution lines.
5. **Done means:** all criteria met, full suite green, and a short summary appended to `TASKS.md` — what was built, what was deferred and why.
