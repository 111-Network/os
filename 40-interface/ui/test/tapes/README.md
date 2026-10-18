# SSE tapes

Byte-for-byte recordings of what a real `111 daemon` sent, made by `interface/verify/smoke.sh`.
Never hand-written, never edited: the only rewrite is the scratch root path → `/var/lib/111`.
`test/tapes.test.ts` replays every `*.sse` here through the real parser and frame mappers and
validates each frame against the contract (`openapi.yaml` `x-sse-events`).

## Naming

- `events*.sse` — a `GET /api/v1/events` stream; `chat*.sse` — a `POST /api/v1/chat` stream.
  The prefix is what the gate keys on (`kindOf` in `tapes.test.ts`); any other name fails it.
- Layer 4 (single-node smoke): `events.sse`, `events-replay.sse`, `events-activity.sse`,
  `chat.sse`, `chat-tool.sse`, `chat-error.sse`.
- Layer 5 (`smoke.sh --three-node`): `events-l5-<scenario>.sse`, `chat-l5-<scenario>.sse`.

| Tape | Recorded on | What it holds |
|---|---|---|
| `events-l5-direct.sse` | member A | a direct `tcp-lan` message to B: `message` new → state delivered (+ `fabric`) |
| `events-l5-request.sse` | member A | a first message from an unknown node: `contact` request, `message` request, accept → `contact` updated |
| `events-l5-relayed.sse` | member B | a message to an offline member: `message` sent → relayed (hub custody) → delivered |
| `events-l5-hub.sse` | hub H | the hub's first lifetime: services, `fabric` (hub role, enrollments, custody), `contact`, `nearby` (stub Meshtastic presence), `panels` |
| `chat-l5-tool.sse` | member A | an agent turn calling `messages.read` with the daemon-minted bearer: token, tool, done |

## Recording

Routine runs record only missing tapes; `SMOKE_TAPES=1` re-records (single-node mode: the Layer 4
tapes; `--three-node`: the Layer 5 ones). After a re-record run `pnpm test` (or
`pnpm exec vitest run test/tapes.test.ts`) — a daemon change the UI would drop fails there.
