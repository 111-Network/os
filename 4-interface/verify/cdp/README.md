# 4-interface/verify/cdp — the panel lint's browser harness

Driven by `../panel-lint.sh` (V2, Layer 5 §13 "Offline lint" and §20 G3). No npm dependencies: Node >= 22 (built-in
`WebSocket`) and a Chrome/Chromium (`$CHROME`, `/usr/bin/google-chrome`, then playwright's chromium).

| file | what |
|---|---|
| `panel-lint.mjs` | the driver: launches headless Chrome in the namespace, runs the scenarios, prints `ok:` / `FAIL:` / `note:` and the request tables |
| `lib/cdp.mjs` | a ~150-line CDP client (flat sessions) and the Chrome launcher |
| `lib/record.mjs` | attaches to every target (page, iframe, worker, popup), keeps all `Network.*` events, folds them into one entry per request hop (raw headers from the `*ExtraInfo` events) |
| `lib/rules.mjs` | the assertions, as a pure function `evaluate(ctx)` over a recording; plus the request table |
| `lib/bundle.mjs` | the static (offline) bundle lint: relative `src`/`href`s, no inline handlers, third-party hostnames by grep |
| `selftest.mjs` | proves every rule can fail: a clean synthetic recording passes, each of ~60 hostile mutations is flagged by its rule |
| `placeholder-bundle/` | stands in for the Meshtastic web client when none is present: same API shape (`PUT /api/v1/toradio`, `GET /api/v1/fromradio?all=true`), nothing else |
| `canary-bundle/` | a hostile bundle (font CDN, script CDN, pixel, telemetry, WebSocket, call to the dashboard origin, a declared optional host, an inline handler) served in place of the real one at the end of every run — the lint must flag it |
| `attacker/` | the page other origins on the node's host are served as in the probes (no CSP, like any other local service) |

Scenarios (each in its own browser context, every request recorded):

- **lan** — dashboard on `http://192.168.77.5:11180` (a LAN visitor), pair through the form, service page > Panel tab, the
  ticket URL into the panel origin, the panel runs for `--dwell` seconds (10), then: a fetch of the panel API from the
  dashboard origin, a frame in the dashboard navigating to it, Reload (the dashboard re-mints), a spent-ticket replay in a
  fresh browser, probes from another origin on the same host (`:11300`, same site) and from a second address
  (`192.168.77.9:11300`, cross site): GET, drive-by POST, non-simple PUT (preflight).
- **localhost** — `http://localhost:11180` (the 308 to `127.0.0.1`), implicit loopback session, the same checks. Chrome
  sends Fetch Metadata (`Sec-Fetch-*`) only to trustworthy origins (https, `localhost`, `127.0.0.1`), so here the
  guard's `Sec-Fetch-Site` rules are visible; on the LAN address they are not.
- **canary** — the hostile bundle (see above); the rules that must flag it are named in `panel-lint.mjs`.

Real bundle: `PANEL_LINT_BUNDLE=<dir> bash ../panel-lint.sh` (with `--bundle-kind real` the driver stops reading the
placeholder's DOM and judges the client by its traffic; the connect-dialog prefill is reported if an input holds the panel
authority). The placeholder run does not replace that run; it is the external gate.

`PANEL_LINT_KNOWN_GAPS=1` turns the checks labelled `GAP (blocked-by …)` — defects already reported to their owner — into
notes; everything else stays strict.
