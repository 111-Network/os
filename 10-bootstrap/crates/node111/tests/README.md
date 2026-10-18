# node111 integration tests — notes

## Layer 5 fabric (F2) — how the pieces compose for the live run (V1, Wave 3)

`tests/api_v1_messages.rs` runs several nodes in one process (real identities, real SQLCipher stores,
real `tcp-lan` on loopback with `refuse_loopback` off). The netns run (V1, `40-interface/verify/smoke.sh`)
uses the real `111 daemon` per root instead; nothing here needs a test hook there:

- **Roots.** One `NODE111_ROOT` per node. The daemon keeps the identity loaded at start and opens
  `identity/counters` itself (the single runtime writer); the fabric lives in `<root>/fabric/`
  (`store.db`, `announce`, `settings`, `display-times`, `blobs/`), created at first start (0700).
- **Hub.** A Cat 4 category defaults to `hub_role` (listener on `0.0.0.0:11183`); any node becomes a
  hub with `PATCH /api/v1/fabric/settings {"hub_role": true}` from loopback (the listener binds before
  the answer). `111 fabric offer` (or `POST /api/v1/fabric/hub/offer`, loopback-only) prints the code,
  the `ip:11183` address and the `111HUB:<card>:<code>:<address>` bundle. In a netns the address is
  the first LAN IPv4 (`bind::lan_ipv4s`) — the veth address.
- **Member.** `111 fabric enroll '<bundle>'` (or `POST /api/v1/fabric/hub/enroll {address, code,
  card}`) waits ≤ 20 s for `ENROLL_OK`; the home hub is persisted to `fabric/settings` and kept
  dialed. `111 fabric status` reads `GET /api/v1/fabric`.
- **Direct links without a hub.** Add `"peers": [{"node_id": "<hex>", "address": "ip:11183"}]` to
  `<root>/fabric/settings` before start; the daemon keeps a dial to each (the far end must listen:
  `listen_lan` or `hub_role`).
- **Contacts and messages.** Cards: `GET /contacts/me/card` → `POST /contacts {card}` on the other
  node; members of one hub can also add each other by `node_id` (the hub answers the card request).
  `POST /messages {to, text}` → `202 {id}`; watch `/api/v1/events` for `message` frames (`new` →
  `state` … `delivered`; `relayed` when the hub took custody for an away member).
- **Agent.** `111-agent` gets `AGENT111_DAEMON_TOKEN` + `AGENT111_DAEMON_API_URL`; its bearer reads
  `/messages*`, `/contacts*`, `/fabric`; `messages.send` writes a `kind: message` proposal that a
  human accepts with `POST /proposals/{id}/decision` (session or trusted loopback, never the bearer).
- **Tapes.** Every `message`/`contact`/`fabric` frame is contract-shaped (the tests validate them
  with the same JSON-Schema subset the UI uses), so V1 can record them as SSE tapes directly.
- **Restore-pending.** Deleting `identity/counters` before start reproduces L2 §8.4's state-absent
  case: no announce, no `tcp-lan`, `409 restore_pending` on send.
