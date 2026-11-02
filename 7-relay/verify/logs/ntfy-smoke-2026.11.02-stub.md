# ntfy carrier smoke log — stub, 2026.11.02

Row P1 (`7-relay/TASKS.md`), spec `7-relay/README.md` §6.2 and §6.4. Written by `7-relay/verify/carrier.sh --smoke` (the test `smoke_log` in `1-bootstrap/crates/node111/tests/carrier_ntfy.rs`) on 2026-11-02T04:40Z (UTC).

**Server: `7-relay/verify/stub-ntfy.py`, the stdlib stand-in, not ntfy.** Every figure below is the prototype client (`N/carrier/ntfy.rs`) against the stub on loopback; the real-server log is `logs/ntfy-smoke-<calver>.md` (G10, row V4, an owner-run server). The calver names the build: `NODE111_BUILD_VERSION` when a release build is stamped, else the date of the run (no signed release exists yet).

## Requests (method, status, body bytes, latency)

Client latency from the call to the end of the answer read (a held stream: to its response head). `sent` is the request body, `received` the response body bytes read.

| Step | Method | Kind | Status | Sent B | Received B | Latency ms |
|---|---|---|---|---|---|---|
| A: 5A flush (drain, post 3 puts, drain) | GET | poll | 200 | 0 | 0 | 3.0 |
| A: 5A flush (drain, post 3 puts, drain) | POST | publish | 200 | 843 | 992 | 2.4 |
| A: 5A flush (drain, post 3 puts, drain) | POST | publish | 200 | 4096 | 4245 | 2.5 |
| A: 5A flush (drain, post 3 puts, drain) | GET | poll | 200 | 0 | 0 | 2.2 |
| B: 5A flush (drain 3 puts, post 3 acks, drain) | GET | poll | 200 | 0 | 5237 | 2.7 |
| B: 5A flush (drain 3 puts, post 3 acks, drain) | POST | publish | 200 | 451 | 600 | 2.1 |
| B: 5A flush (drain 3 puts, post 3 acks, drain) | GET | poll | 200 | 0 | 0 | 1.9 |
| A: 5A flush (drain 3 acks, nothing owed) | GET | poll | 200 | 0 | 600 | 2.3 |
| A: 5A flush (drain 3 acks, nothing owed) | GET | poll | 200 | 0 | 0 | 2.2 |
| raw: 4,096 B body | POST | publish | 200 | 4096 | n/a | 3.2 |
| raw: 4,097 B body | POST | publish | 413 | 4097 | n/a | 2.9 |
| B: 5B held stream (to the response head) | GET | stream | 200 | 0 | 0 | 2.0 |
| since=<id> after a restart, then since=all | POST | publish | 200 | 72 | 221 | 3.5 |
| since=<id> after a restart, then since=all | GET | poll | 400 | 0 | 0 | 3.8 |
| since=<id> after a restart, then since=all | GET | poll | 200 | 0 | 221 | 3.5 |
| a flush inside the limit | POST | publish | 429 | 19 | 74 | 2.1 |

The 5A sequence over the fake hand-off: A posted 3 puts in 2 messages (taken 3), B's first drain took 3 frames and posted 3 acks, A's next first drain took 3 acks; `forward_done` for A's puts: [(1, "taken"), (2, "taken"), (3, "taken")].

## The 4,096 / 4,097 B boundary

- a 4,096 B body: **200**; a 4,097 B body: **413** (the stub's `--message-limit 4096`, attachments off).
- the carrier's largest frame, 3068 B, makes a body of exactly 4,096 B (posted above); a frame of 3069 B is `rejected` by the carrier and makes **0 requests** (never posted).

## 429 (the default limiter: a burst of 60, then one request per 5 s)

- the first 429 came at request **61** (60 inside the burst);
- a flush inside the limit: `retry` (`Retry`, the row stays; the rest of a flush is not posted after a 429);
- after 5.2 s one request: **200**, the next: **429**.

## `since` after a stub restart

The stub keeps its cache in memory, so a restart loses it (as an ntfy server without `cache-file`). B's saved id before the restart: `c2QahMcLj9kt`.

- `--since-miss error` (the stub's default; an explicit signal): the drain with `since=<id>` gave **Err: carrier: the saved since id is gone**; the next drain, `since=all`, gave **Ok, 1 frame(s)**.
- `--since-miss all` (a silent fallback: every cached message for an unknown id): the drain gave **Ok, 1 frame(s)**, with no miss visible to the client, so §5.9 re-post trigger (3) cannot fire on such a server. Which answer upstream gives is the §6.4 bench item (*unverified from this box*).

## Keepalive and the 5B thread

- the stub ran with `--keepalive 2`; the client measured a gap of **2000 ms** between keepalive events (upstream's default `keepalive-interval` is 45 s, *unverified from this box*); the client's read timeout is 3 × the configured interval (6 s here).
- threads named `111d-ntfy-5b`: 0 before the hold, **1** while holding, 0 after the stream was dropped; a frame posted by A arrived on the stream: true.

## RSS during a flush

`/proc/self/status` of the test process (the carrier, ureq + rustls, the tokio runtime and the test harness, not `111d`): a flush of 40 frames × 1,500 B (20 messages, 40 frames taken) in 73.0 ms. VmRSS before **9632 kB**, peak during **9644 kB** (sampled every 2 ms), after **9644 kB**: the flush added **12 kB** at its peak to a process that had already run the steps above (allocator pages reused); VmHWM 9632 → 9644 kB. The flush's own data: 60000 B of frames, 80160 B of bodies.

## Bytes per frame (base64 4/3 + the batch's 2 B per frame)

A batch body is `ceil((2 + Σ(2 + frame)) × 4/3)` B; the HTTP request adds its headers.

| Frames in one message | Body B | Body per frame B | Overhead per frame B |
|---|---|---|---|
| 1 × 110 B | 152 | 152.0 | 42.0 |
| 1 × 313 B | 423 | 423.0 | 110.0 |
| 3 × 313 B | 1263 | 421.0 | 108.0 |
| 9 × 313 B | 3783 | 420.3 | 107.3 |
| 1 × 1500 B | 2006 | 2006.0 | 506.0 |
| 2 × 1500 B | 4008 | 2004.0 | 504.0 |
| 1 × 3068 B | 4096 | 4096.0 | 1028.0 |

## Owner-server items (§6.4)

`NTFY_BIN` is not set or not on disk: no real ntfy server ran.

- the server version and its config (`7-relay/carriers/ntfy/server.yml` is the recommended one): **not run from this box**
- per request method, status, body bytes and latency against a real server: **not run from this box**
- the 4,096 / 4,097 B boundary on a real server: **not run from this box**
- 429 behaviour and the 30-connections-per-visitor limit on a real server: **not run from this box**
- retention across a server restart with and without `cache-file`: **not run from this box**
- `since=<id>` after eviction, and `since=all`: **not run from this box**
- the held stream's keepalive interval on a real server: **not run from this box**
- whether the server forwards messages to a push service (Firebase, an upstream server): **not run from this box**
- the token header (`Authorization: Bearer`): **not run from this box**
- ntfy.sh's anonymous limits and retention: **not run from this box**
- end-to-end delivery between two gateway hubs in netns (G10, row V4): **not run from this box**
