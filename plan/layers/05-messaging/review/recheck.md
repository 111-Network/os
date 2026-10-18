# Layer 5 — re-check of spec v1.1 (crypto + panels lenses)

Judge: fable (re-check) · 2026-10-14 · `os/50-messaging/README.md` v1.1 (`21cd4cb`) vs the triage and the three reviews.
Real checks: `git show 97fd9bb` (v1 text); `counters.rs` (`next_announce_counter` fsyncs on mint, errors
`RestoreFloorRequired` while a floor is pending; `open()` jumps traffic counters only); `store.rs:272` (stale check
after signature verification); `routes.rs:197` (`host_allowed` accepts `localhost`); varint arithmetic; a line-grammar
check of the manifest (no TOML parser installed here).

## 1. Verdict: **fix-then-lock**

1. Every accepted finding is present: 53 landed, 4 partial, 0 not landed. The architecture (D19) stands.
2. One new blocker inside an accepted fix: the §4 epoch-collapse rule (M4's text) drops old-epoch custody items under
   §9 store-and-forward while the sender shows `delivered` — B2's symptom, re-opened.
3. A11 relaxed the replay window without keeping its freshness verdict: a replayed sealed packet **attributes** an
   attacker's socket and the hub pushes the victim's mailbox to it — M6's hole, re-opened.
4. Three defects would make a builder diverge: `msg_seq` costs 5–7 B, not 1–5; `tx_state` keyed by `(stream)` makes
   stream 0 global; the dashboard on `localhost` cannot frame any panel.
5. Every fix is a paragraph: one pass on §4, §8.1, §10.3, §13, then lock (D20).

## 2. Fix ledger

| id | status | note |
|---|---|---|
| crypto B1 | landed | §3 `auth = pair_mac(16) ‖ [sig(64) if SIGNED]`; "only `pair_mac` keys replay state"; G4 vector |
| crypto B2 | landed | §4 A11 + §19; §18.1 `pair_mac → msg_seq dedup → store`; DTN residual |
| crypto M1 | landed | §6.1 `n_partial(1) ‖ [seq, frag_bitmap(u16 LE)]*` |
| crypto M2 | landed | §6.1 `ack_len` first, block before `payload`, `n_strong(1, 0 allowed)` |
| crypto M3 | landed | §3.2 `msg_seq = 0` column; exempt from `rx_state`/ACK; idempotent per kind |
| crypto M4 | **partial** | epoch/`n`/FULL-sync present; collapse rule contradicts §9 (N1); "1–5 B" contradicts §3 (N3); clean backup mints no epoch (N5) |
| crypto M5 | landed | sig input `… ‖ dest ‖ stream ‖ msg_seq ‖ kind ‖ flags ‖ …`; `dest` for 1:1/group/public |
| crypto M6 | **partial** | stale rule, stored bytes, attribution, "authenticate nothing" present; card-answer gate stated two ways (N7); attribution replayable (N2) |
| crypto M7 | landed ✅* | §5/§7.2 INTRO `tcp-lan`-only; §21 records fragment-0 design |
| crypto M8 | landed ✅* | `0x05 chunk (reserved)`; `session_tag` random; A5 names mode `0x02` only |
| crypto M9 | landed | §18.5 KCI residual; "never protects confidentiality" |
| crypto M10 | landed | `ack_sig`, `custody-sig`, `obj` inputs pinned; `111/ack/v1` gone |
| crypto m1–m6 | landed | Shim column · one RATCHET bit, inner reserved ⇒ reject · one wire per version · TLV `0x21` · enroll hash + code rules · random `object_key`, keyed BLAKE2b |
| crypto m7 | **partial** | oracle stated and narrowed, never forwarded; gate ambiguous (N7) |
| crypto m8, m9, m11 | landed | "received", `max(cursor)` · scope sentence + `hub_not_yours` · frag bounds, partials capacity-bounded |
| crypto m10 | **partial** | handle "caches the per-contact pair key"; whether the MAC runs behind the handle or the pair key crosses into `mesh111` is unsaid (§1: "never key bytes") |
| panels P1 | landed | CORS stripped, OPTIONS refused, cross-site refused "any method and mode", no loopback skip; G3 |
| panels P2 | landed ✅* | §14 shim on the panel origin over per-session meshmud TCP; "No HTTP downlink is configured" |
| panels P3 | landed | drop upstream `Set-Cookie` `s111`/`p111_*`; exact-name reads, every copy |
| panels P4 | landed | §13 lifecycle paragraph verbatim |
| panels P5 | landed | CDP lint, `optional_hosts`, honest copy, CI-built bundle, tag named |
| panels P6 | landed | "Linux only — Termux cannot open USB serial"; TCP-to-board; §20 gate |
| panels P7 | landed | 40-byte names, `O_NOFOLLOW`, ≤ 1 MiB, schema, ≤ 4096, keep-last; untrusted block |
| panels P8 | landed | "several viewers each own a downlink" |
| panels P9 | landed | `panel: {id, port, …}`; origin from `location.hostname` |
| panels P10, P11 | landed | authority host+port, WS/SSE guarded, OPTIONS refused · per-request CSP, `nosniff`, `no-referrer`, hop-by-hop, timeouts, `502 upstream_error` |
| panels P12 | landed | one expression per line; `toml` crate at build (grammar-checked) |
| panels P13 | landed | `secure_note` reworded; `localhost` excluded; G3 prefill check — consequence N6 |
| panels P14–P17 | landed | `Problem.code` `port_busy`, in-place child swap · Termux residual · "panel transmits" copy · path regex, `advertise = false`, 0600 |
| integ B1 | landed | §17 `AGENT111_DAEMON_TOKEN`, Bearer "from a loopback peer only"; §18.9; §19 |
| integ J1 | landed | `REJECT_UNKNOWN_DEST`, scope sentence, FETCH = own items on attributed connection, A+ lifts |
| integ J2, J3 | landed | `meshmud_bound`, `radio_bound_meshtastic` · `health = { listen }`, never a connect |
| integ J4 | landed | Meshtastic step only, port-owner table, `devices.json` → L6 |
| integ J5 | landed ✅* | attachments v1.1; A5 moved |
| integ J6, J7 | landed | `[proxy]` reserved, true fails adoption · GPL conveyance label, LICENSING rows, Go notices path, §4 wording kept |
| integ M1–M4 | landed | TOML · `qr: {size, rows}` + `jsQR` · "43–45" · `Link` types + `BoxFuture` |
| integ M5–M9 | landed | harness task + `lossy-loopback` · five wording fixes · pre-connect · MeshCore range + updater intake · narrowing + coalescing |

## 3. New findings

**N1 — blocking · §4 epoch collapse loses old-epoch custody items; the sender shows `delivered`.**
Quoted: "A gap ≥ 2^32 from an authenticated sender (a new epoch) collapses the prefix to `seq − 1`; old-epoch
stragglers are then dropped as below-prefix." Alice sends (E,1..10) to absent Bob via custody; Alice crashes, restarts
with E′ > E; Bob returns, meets Alice on the LAN first, receives (E′,1), collapses to (E′<<32)|0; Bob then fetches the
mailbox: (E,1..10) are below-prefix ⇒ re-ACKed, never delivered; Alice's UI shows ten messages `delivered`. Not an
attack (only the real sender moves epochs), but exactly the silent loss B2 was fixed for. Sender-side re-numbering is
worse (old copies delivered first ⇒ visible duplicates).
Fix: "A new epoch (a `pair_mac`-verified packet whose epoch exceeds the current) opens a new (prefix, ranges, partials)
entry; the previous epoch's entry is **frozen and kept** and old-epoch packets are still delivered against it (custody
and multi-path legitimately deliver an earlier epoch after a later one). `rx_state` keeps the newest **4** epochs per
(peer, stream); older are stale. `ack_prefix` is the current epoch's; frozen epochs are acknowledged as absolute
ranges (`n_ranges ≤ 8`)." G1: sender crash with items in custody, every message once. G4: a two-epoch ACK vector.

**N2 — major · §8.1 attribution is replayable under A11.**
Quoted: "attributed to node X only after a `pair_mac`-verified packet from X has arrived on it"; §4 "the receive window
is **not enforced below high-water**". A sniffed `MAILBOX_FETCH` from Bob replayed on an attacker's socket passes
`pair_mac`, is not window-rejected, attributes the socket to Bob; the hub answers it and pushes Bob's custody items
there (ciphertext harvest, counts, timing — M6 ii). Symmetric toward a member with a replayed hub packet.
Fix: "A11 relaxes *rejection*, not bookkeeping: every verified packet is still offered to the L2 window; `msg_seq`
kinds are processed for content whatever the verdict; **attribution, card answers and proactive pushes require a
packet the window accepted as fresh**." The L2 unclean-reboot jump (J = N) covers the last-N replays. G1: "a replayed
sealed packet on a fresh socket never attributes it."

**N3 — major · `msg_seq` wire cost contradicts the §3 size table.**
Quoted: §4 "Wire cost 1–5 B"; §3 "Inner hdr (≈7–9)". With `epoch` an announce-counter value, `msg_seq ≥ 2^33` ⇒ 5
varint bytes; after one restore jump (`epoch ≥ 1025`) ⇒ 7 B. Inner header 11–13 B; 160 B payload 66–68 B (Tier 1
64–66; 16 × 64 = 1024 B); every `seq` in the ACK lists and `seen_peer_seq` grows alike. An operator restore floor
≥ 2^32 breaks the packing.
Fix: identity stays `(sender, stream, epoch, n)`; on the wire every `msg_seq` is `epoch(varint) ‖ n(varint)` (2–4 B
typical); new-epoch rule = "epoch greater than the current"; table "Inner hdr (≈8–10)"; reword A3.

**N4 — major · `tx_state` keyed by `(stream)` makes stream 0 global.**
Quoted: §10.3 "`tx_state` (`(stream)` → next `n`, `epoch`)"; §3 "stream 0 = the 1:1 conversation". Read literally,
Alice's stream 0 is one counter across all peers; Bob's `rx_state (Alice, 0)` gets a gap for every message to anyone
else, the prefix never advances, the ≤ 4-range list overflows on the second peer. New in v1.1.
Fix: "`tx_state` is keyed by `(peer, stream)`: stream 0 is the pair's 1:1 stream; `msg_id = (sender, recipient,
stream, epoch, n)`; `messages` carries `peer`."

**N5 — major · no epoch on the restore paths the spec advertises.**
Quoted: §4 "on a store opened without its clean-shutdown flag, and on restore"; §10.2 "protects DB-only copies,
backups and synced app dirs". A cleanly closed `store.db` restored from a backup onto a live identity opens *with* its
clean flag ⇒ no epoch ⇒ old `n` reused ⇒ the receiver dedups and re-ACKs ⇒ `delivered`, nothing shown. Unstated:
behaviour while `next_announce_counter()` errors `RestoreFloorRequired`; `n` reaching 2^32.
Fix: "The store records L2 `announce_last` at clean close. A new epoch (one mint per open, shared by every stream
started in that run) is minted when the clean flag is absent, when the recorded `announce_last` differs from the live
one, after a restore floor is set, and when `n` would wrap. While `RestoreFloorRequired`, nothing is sealed: outbox
items stay `queued`, reason `restore_pending`. Residual: one consistent older image of store **and** identity dir is
L2 §8.4's state-present case, undetectable locally."

**N6 — major · the dashboard on `localhost` cannot frame any panel.**
Quoted: §13 "exact Host match … (`localhost` excluded …)"; "the UI builds it from `location.hostname`". `host_allowed`
accepts `localhost`, so `http://localhost:11180` yields a panel origin the panel refuses; mapping it to `127.0.0.1`
makes the iframe navigation `Sec-Fetch-Site: cross-site`, refused too. G3 runs on a netns IP and would not see it.
Checked and clean: same host, different port is **same-site** for `Sec-Fetch-Site` and `SameSite=Strict`, so the
iframe navigation, ticket swap and same-origin fetches pass; the ticket leaves the URL at the redirect and
`no-referrer` is set, so nothing reaches `optional_hosts`.
Fix: "The main listener answers `Host: localhost` with a 308 to the same path on `127.0.0.1`; panels never see it.
`[::1]` is matched bracketed." Fallback: panels accept `localhost`, G3 prefill check kept.

**N7 — minor · card answers gate two ways.** §7.2/§18.4 "past the announce hello" vs §8.1 "gate on attribution". Pick
attribution (replay-safe after N2; the member's `MAILBOX_FETCH` attributes the socket); §18.4 reads "answered only to
members on attributed connections".

**N8 — minor · `ENROLL` from an unknown member.** §7.3 "`ENROLL {card, code}`": the hub cannot resolve `sender_ref` or
verify `pair_mac` before parsing the payload, against §18.1. Fix: "`ENROLL` sets `INTRO`; the card rides the §3 card
slot per §7.2(3); payload `{code}`."

**N9 — minor · `ack_sig` placement.** "appended after the block" vs "`ack_len` covers everything after it". Fix:
"`ack_sig` only in a standalone `ACK` packet, as the last 64 B of its payload; never piggybacked."

**N10 — minor · `[auth] mode = loopback-only` undefined** once "no loopback skip" holds. Fix: "= the listener binds
loopback regardless of LAN sharing; the ticket flow is unchanged" — or drop it.

**N11 — minor · shim session lifecycle.** Per-session downlinks (§14) × cookies revoked only on rotate/disable/LAN-off
(§13) × "Open full client" re-mints ⇒ downlinks and `FromRadio` buffers accumulate. Fix: "the downlink opens on the
session's first `/api/v1/*` call, closes on revocation or idle; the buffer is capacity-bounded, drop-oldest, surfaced
in `panel.state`. A ticket ignored because a valid cookie is present is also spent."

**N12 — minor · shim type-2 sub-type byte has no position.** §5 "shim type 2 with a sub-type byte" vs §3 "bits 2–0
reserved (0)". Fix: "frame_type 2 uses bits 2–0 as sub-type (0 announce · 1 card request · 2 chunk)."

**N13 — minor · D19 is stale; D20 must list the deltas (in the triage, not the log):** g (HTTP downlink → 111d shim;
one client → per-session; "vendored" → CI-built tarball) · e (HKDF-SHA256 → keyed BLAKE2b, accepted as m6 without an
owner flag) · d (WS/SSE "forwarded" → reserved; `Sec-Fetch-Site` on mutations → every request) · detail (2^32 jump
→ epochs; A5 deferred).

## 4. What must not be touched

`pair_mac` always present and alone keying replay state (B1) · A11's replay authority (keep; add the freshness verdict,
N2) · fragment-then-seal, ≤ 16 outstanding, SACK bitmaps · the stale-announce rule (a lower counter with identical keys
never overwrites: no genesis-over-newer pin is possible) · `REJECT_UNKNOWN_DEST` and the one-hub scope · keyed-BLAKE2b
store KDF · the shim over per-session meshmud TCP, "no HTTP downlink" · ticket → cookie everywhere, CORS stripped,
`OPTIONS` refused, per-request CSP, the lifecycle paragraph · presence-file hardening · the loopback-only bearer token ·
the manifest as written · INTRO `tcp-lan`-only · attachments in v1.1.
