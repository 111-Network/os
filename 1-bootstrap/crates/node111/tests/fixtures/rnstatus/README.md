# rnstatus fixtures (Layer 6 §11 "`transport.read`", B4)

Golden inputs for `daemon/transport/rnstatus.rs` (`parse_text`, `parse_json`), driven through a fake `Runner`;
no test starts Python or imports Reticulum.

Provenance, **unverified until V3 captures real output** (`verify/spine.sh` G4 runs a real rnsd and rnstatus):

- `hub.txt`, `phone.txt`: the RNS 1.5.x `rnstatus` print layout (one ` Type[name]` header per interface, then
  `Status`, `Serving`/`Clients`/`Peers`, `Mode`, `Rate`, `Traffic ↑tx / ↓rx` lines), with the values R07 §2 check 1
  recorded (`Shared Instance[rns/nodeA] Status: Up, Serving: 1 program`) and R05 §4.1 recorded for the Backbone pair
  (`Up, Clients: 1`), renamed to this node's stanza names.
- `legacy-traffic.txt`: the older traffic layout (`1.21 KB↑` / `980 B↓`).
- `arrows.txt` (B4 review F1, synthetic): both arrows on one line in the old and the new layout, a non-ASCII byte
  right before a size, an unparsable value; the parser must read what it can and never panic.
- `no-instance.txt`: what rnstatus prints with no shared instance to ask.
- `status.json`: the `rnstatus -j` shape as the RNS source builds it (interface stats dicts); **schema not captured
  from a run**, so the daemon does not use `-j` yet (§11: "`-j` is used once its schema is captured").
