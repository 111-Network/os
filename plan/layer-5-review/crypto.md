# Layer 5 adversarial review — lens: crypto / envelope / protocol

Judge: fable (crypto), 2026-10-14. Spec: `os/messaging/README.md` v1 draft, §3–§12, §18, §19. Evidence: `identity/README.md` §6/§8.4/§12/§13, `id111/src/{seal,counters,store}.rs`, research 04 and 07. "Verified" = a real check; "reasoned" = argued from quoted text.

## 1. Verdict: **not-ready**

1. The architecture (fragment-then-seal, `(sender, stream, msg_seq)` identity, `pair_mac` over Noise-"N", capacity-only store) is right and matches D19 b/e and L2 §6.
2. Two blockers re-open the hole A2 closes: SIGNED packets drop `pair_mac`, so any holder of signed plaintext can re-seal it and poison the signer's window (B1); the L2 32-entry window rejects delayed custody items whenever a fresher packet arrived by another path — D19 b's model loses messages (B2). Both need an L2 amendment missing from §19.
3. The receipt/fragment machinery (§5–§6) is unbuildable as written: no fragment-level SACK, no delimiter for piggybacked ACKs, no `msg_seq` rule for control kinds (M1–M3).
4. `msg_seq` durability and the 2^32 jump have no persistent floor; "never redeliver below the prefix" becomes silent loss after a crash or a second words-only restore (M4).
5. Fixes are text-level, but §3–§6 and §18.1 need one coherent rewrite and a re-read, not patch edits.

## 2. Findings

| id | severity | section | title |
|---|---|---|---|
| B1 | blocking | §3, §18.1 | SIGNED packets drop `pair_mac`: re-seal poisons the signer's window |
| B2 | blocking | §4, §9, §18.1 | L2 32-entry window rejects delayed custody items under multi-path delivery |
| M1 | major | §5, §6.1 | ACK is in `msg_seq` units; fragments share one `msg_seq`; no fragment SACK |
| M2 | major | §6.1 | `PIGGYBACK_ACK` has no length delimiter; `strong` block undetectable |
| M3 | major | §3, §4 | Control kinds carry a mandatory `msg_seq` with undefined semantics |
| M4 | major | §4, §10 | `msg_seq` not durable before seal; 2^32 jump has no persistent epoch |
| M5 | major | §3 | `sig` binds no recipient: signed AGENT_OP/PATCH forwardable to Carol |
| M6 | major | §7.2, §8.1 | Link-up announce: stale rule drops honest peers; pushes to unauthenticated connections |
| M7 | major | §5, §7.2 | INTRO cannot fit 160 B; card placement and unverifiable fragments unspecified |
| M8 | major | §5, §8.1, §19 A5 | Unsealed Tier-2 chunks have no frame type; `session_tag` undefined; mode 0x03 misfiled |
| M9 | major | §18 | KCI residual of `pair_mac` unstated; §18.5 wording contradicts the construction |
| M10 | major | §3.1, §6 | `ack-sig`/`custody-sig` signed inputs undefined; `111/ack/v1`, `111/obj/v1` unused |
| m1 | minor | §3 | Size table omits the L6 shim byte |
| m2 | minor | §3 | Two RATCHET bits; inner reserved bit has no reject rule |
| m3 | minor | §3 | Feature-gated 32 B sender form shares `version = 0x01` |
| m4 | minor | §3, L2 §6.7 | `signer_pubkey` reservation has no envelope home |
| m5 | minor | §7.3 | Enrollment code: lifetime, lockout, keyed-hash inputs unspecified |
| m6 | minor | §10.2, §10.3 | Object-key nonce scheme unpinned; HKDF-SHA256 adds a hash family |
| m7 | minor | §8.1, §18.4 | Card request is a contact-membership oracle; unauthenticated pin churn |
| m8 | minor | §6.2, §9 | "Forces the custodian to hold" overclaims; COMMIT replay rule missing |
| m9 | minor | §9 | Unstated D19 b consequence: both parties enrolled at one hub; no blob custody |
| m10 | minor | §3, §18.3 | Pair-key caching at the identity handle unstated |
| m11 | minor | §5 | Reassembly bounds unpinned |

## 3. Findings in detail

### B1 — SIGNED packets drop `pair_mac` (blocking, reasoned)
Quoted: "`auth  pair_mac(16) unless SIGNED · sig(64) if SIGNED (replaces pair_mac)`"; "`sig` … Covers content, not the sealed packet, so it survives re-sealing"; "The receiver MUST verify `pair_mac` (or `sig`) **before** touching counter/window state".
Attack: Alice sends a signed group post to Bob and Carol (§12 fan-out), or publishes any signed `KB`/`UPDATE`/`PATCH`. Carol, or anyone holding the public object, re-seals the identical signed plaintext to Bob with L2 `counter = 2^60`. Bob's check passes ("sig verifies ⇒ authenticated sender = Alice"), Alice's window advances to 2^60, and every genuine packet from Alice is rejected as a replay. `msg_seq` dedup runs *after* the window step, so it does not help. The A2 poisoning, re-opened for every stored/public/code class.
Fix: `pair_mac` is always present (`auth = pair_mac(16) ‖ [sig(64) if SIGNED]`); only `pair_mac` keys window state; `sig` is authorship/transfer only. SIGNED payload at 160 B drops to ≈ 9 B — irrelevant, those classes are Tier-2 or TCP. G4 gains "a re-sealed valid signature never advances a window".

### B2 — L2 window vs store-and-forward (blocking, verified: `counters.rs:160` rejects `d ≥ WINDOW_SIZE` behind high-water)
Quoted: "Verify before state: `pair_mac`/`sig` → counter/window → dedup → store" (§18.1); "direct links + home-hub mailbox" (§1, D19 b); "Members fetch on link-up" (§9).
Scenario: Alice sends Bob 40 messages while Bob is away (custody, counters c…c+39). Bob returns, meets Alice on the LAN first; one direct packet arrives (c+40). Bob then fetches the mailbox: items c…c+8 are > 32 behind high-water and L2 §6.5 drops them before `msg_seq` dedup runs. Same failure for a flood echo heard before the hub copy, or fragments split across paths. "Delivery is non-negotiable" is broken by the spec's own step order while Alice's UI says `relayed`. G1 is single-path and would not catch it.
Fix: amendment **A11** to L2 §6.5: for L5 kinds carrying a `msg_seq`, replay authority is the `(sender, stream, msg_seq)` state (never forgets below the prefix); the L2 counter stays AD-covered and strictly increasing, but the window is not enforced below high-water for such kinds. Control kinds (M3) are idempotent by construction (ACK, FETCH) or by `pkt_ref`/cursor-max (PUT, COMMIT). Rewrite §18.1 as `pair_mac → msg_seq dedup → store`. State the residual: delay-and-replay of an undelivered old packet delivers it — DTN behaviour, not a hole.

### M1 — No fragment-level SACK (major, verified against §3 and §6.1)
Quoted: "`[frag_idx, frag_count] varint ×2 if HAS_FRAG`"; "`ack_prefix(varint) ‖ n_ranges(1 ≤ 4) ‖ [start, len](varint ×2)*`"; "the receiver ACKs fragments by SACK (§6.1)".
All fragments of one message share one `msg_seq`; the ACK speaks only in `msg_seq` units. "Got fragments 0, 2, 3 of message 17" is inexpressible, so selective retransmit and "resumable" cannot be built; any loss retransmits all fragments.
Fix: append `n_partial(1) ‖ (seq varint ‖ frag_bitmap u16)*` to the ACK (Tier 1 caps at 16 fragments, so u16 suffices).

### M2 — Piggybacked ACK undelimited (major, verified)
Quoted: "`PIGGYBACK_ACK`: the ACK payload is appended before `auth`"; "`[strong: n(1) ‖ …]`".
Inner layout is `… ‖ payload ‖ auth`; nothing marks where `payload` ends, and the optional `strong` block has no presence byte.
Fix: ACK block = `ack_len(varint) ‖ ack_prefix ‖ n_ranges ‖ ranges ‖ n_partial… ‖ n_strong(1, 0 allowed) ‖ …`, placed before `payload`.

### M3 — Control kinds and `msg_seq` (major, reasoned)
Quoted: "`msg_seq varint per (sender, stream) sequence (§4); stream 0 = the 1:1 conversation`" (mandatory field); "Receiver state per (sender, stream): a contiguous delivered prefix plus a bounded range list".
If `ACK`, `CUSTODY_*`, `MAILBOX_*`, `ENROLL` consume stream-0 numbers, every lost ACK (never retransmitted) leaves a permanent hole: the prefix stalls and the "bounded" range list overflows. If they do not, the spec never says what they carry.
Fix: control kinds set `msg_seq = 0`, exempt from `rx_state`/ACK, idempotent per kind (B2 fix).

### M4 — `msg_seq` durability and restore epoch (major, verified: `RESTORE_JUMP = 1024` and the clean-flag discipline in `counters.rs`)
Quoted: "`msg_seq` … persisted with the store"; "Restore / unclean loss: `msg_seq` jumps forward by 2^32"; "a duplicate is re-ACKed and never redelivered".
(a) Crash after send, before the WAL commit (`synchronous` unpinned): the sender reuses `seq = n` for different content; the receiver re-ACKs and never delivers; the sender shows `delivered`. Silent loss with a green tick. (b) Words-only restore with a lost DB jumps to 2^32; a second such restore jumps to 2^32 again and [2^32, 2^32+k] is deduped. (c) Each restore adds one range entry; with `n_ranges ≤ 4` the fifth epoch is unrepresentable.
Fix: `msg_seq = (epoch << 32) | n`, `epoch` = L2 `announce_last` at stream (re)start (restored by §8.4's floor+1024, hence monotone across disaster recovery); jump on any store opened without a clean flag; commit `n` (`synchronous = FULL`) before the first fragment is sealed; a gap ≥ 2^32 from an authenticated sender collapses the receiver prefix to `seq − 1`. Make this A3's text.

### M5 — Signature binds no recipient (major, reasoned)
Quoted: "Ed25519 over `"111/msg/v1" ‖ sender NodeId ‖ stream(varint) ‖ msg_seq(varint) ‖ kind ‖ BLAKE2b-256(payload)`"; "`AGENT_OP` … SIGNED".
Alice signs an `AGENT_OP` for Bob. Bob forwards the signed plaintext to Carol, who verifies "Alice, stream 0, seq 7, AGENT_OP" and executes it. Transferability is wanted for group posts and public classes, not 1:1 code paths.
Fix: signed input gains `dest` (recipient NodeId; `group_id` for group posts; zeros for public classes) and `flags`.

### M6 — Link-up announce semantics (major, verified: `store.rs:272` returns `StaleCounter` on `counter <= highest`)
Quoted: "Each side sends its announce first; a peer whose announce fails verification is dropped" (§8.1); "hubs push `MAILBOX_ITEM` proactively to connected members" (§9).
(i) `process_announce` errors on an equal counter, so a peer re-sending its stored announce on reconnect is dropped unless it mints a fresh announce (fsync) per link-up — reconnect loops become counter churn, and scanning a known peer's genesis card (counter 1) fails. (ii) Announces are public and replayable: an attacker replays Bob's latest announce on connect, becomes "connected member Bob", and receives Bob's sealed mailbox (ciphertext for harvest-now-decrypt-later, plus counts, sizes, timing) without any pair-authenticated packet.
Fix: (i) stale-with-identical-keys ⇒ accept, no state change; only `KeyConflict` or a bad signature drops; card requests are answered from stored announce bytes, never minted. (ii) A connection is attributed to X only after a `pair_mac`-verified packet from X on it; push and card forwarding gate on that. §18.4 says hello/announce authenticate nothing.

### M7 — INTRO at the 160 B floor (major, verified: 64 + 23 + 137 = 224 B)
Quoted: "`[card] 137–169 B if INTRO`"; "every fragment is … independently authenticated … no fragment poisoning".
An INTRO never fits one flood frame, so on LoRa it is Tier 1. With A2 the receiver has only a 4 B `sender_ref` for an unknown peer; fragments without the card cannot be `pair_mac`-verified until it arrives and must be buffered unverified — the tier's selling point fails — and the spec never says which fragment carries the card.
Fix: card in fragment 0 only; fragments with an unresolvable `sender_ref` wait in a capacity-bounded pending-INTRO buffer until fragment 0 lands; or INTRO is tcp-lan-only in v1 and LoRa first contact needs a card.

### M8 — Unsealed chunks have no carrier (major, verified against both type registries)
Quoted: "`OBJ_CHUNK` frames carry `session_tag(8) ‖ chunk_idx ‖ bao-verified bytes`, **unsealed** when …"; tcp-lan "Types: `0x01` … `0x04`"; shim "frame_type (… 2 public: announce / card request, 3 foreign)"; A5 "`0x03` (bulk chunk mode)".
No tcp-lan type and no shim value carries an unsealed chunk; a `crypto_mode` number cannot describe a frame with no crypto-mode byte. `session_tag` is underived: hash-derived tags link every fetch of one object across recipients and sessions; observed tags let anyone inject garbage chunks (bao rejects them, airtime is spent).
Fix: tcp-lan `0x05 chunk`; shim type 2 gains a sub-type byte; `session_tag` = 8 random bytes chosen in the sealed `OBJ_REQ`; strike mode 0x03 from A5 or redefine it as a sealed chunk mode.

### M9 — KCI unstated, §18.5 misleading (major, reasoned)
Quoted: "`pair_mac` is deniable (either party could have produced it)"; "no static-static-only protection anywhere" (§18.5).
`pair_mac` is Noise-K `ss` authentication: a leak of Bob's X25519 key lets anyone impersonate every contact to Bob, not merely read his history. §18 lists only recipient-side forward secrecy. Deniability is right for human traffic and ACKs; its KCI cost must be written, and §18.5 as worded forbids what §3 adopts.
Fix: §18 rule "KCI residual: a recipient whose X25519 key leaks can be fed forged pair-authenticated traffic from any contact; SIGNED classes are immune." Reword 18.5 to "static-static ECDH never protects confidentiality".

### M10 — Undefined signed inputs, unused tags (major, verified: nothing in the spec references `111/ack/v1` or `111/obj/v1`)
Quoted: "`111/ack-sig/v1`, 64 B, on demand only" (§6.1); "A transferable form (`111/custody-sig/v1`, 64 B) exists on request" (§6.2).
L2 §6.1 requires every signed artifact to pin its tagged input; neither receipt has one, so G4 cannot write their vectors. `111/ack/v1` and `111/obj/v1` are registered with no construction.
Fix: `ack-sig` = Ed25519(`"111/ack-sig/v1" ‖ acker ‖ sender ‖ stream ‖ ack block bytes`); `custody-sig` = Ed25519(`"111/custody-sig/v1" ‖ hub ‖ dest ‖ status ‖ custody_seq ‖ pkt_ref`); delete or define the other two.

### Minor
- **m1** (verified): 160 − 64 − 23/25 ignores the shim byte inside the bearer payload; Tier 0 = 70–72 B, Tier 1 (+2 B frag) = 68–70 B; 16 × 68 = 1088 B.
- **m2**: RATCHET sits in `prio_flags` bit 4 and inner `flags` bit 6; keep the outer one; state "inner reserved bits set ⇒ reject".
- **m3**: the feature-gated 32 B sender form puts two wires under `version 0x01`; the gate never reaches a signed release.
- **m4**: L2 §6.7 reserves `signer_pubkey`; reserve TLV `0x21` (32 B) so delegation stays a soft upgrade.
- **m5**: pin single-use, invalidation on dismissal, and an attempt lockout carved out of "no policy limits"; define the `111/enroll/v1` keyed hash (key, data, member NodeId).
- **m6**: pin `object_key` random per object, nonce = LE chunk index. HKDF-SHA256 adds SHA-256 beside L2's BLAKE2b; justify or use BLAKE2b keyed KDF.
- **m7**: tcp-lan `0x04` is a "is X your contact?" oracle for any LAN peer; unauthenticated `process_announce` churns the L2 LRU cache (harmless under A7). State both in §18.4.
- **m8**: `pkt_ref` proves the hub *received* the bytes, not that it holds them; `MAILBOX_COMMIT` takes `max(cursor)` on replay.
- **m9** (integration judge): `REJECT_UNENROLLED` plus the hub needing the recipient's announce to seal `MAILBOX_ITEM` ⇒ v1 delivers only within one hub's members; blobs have no custody path.
- **m10**: the identity handle must cache per-contact pair keys or run the MAC itself; say which.
- **m11**: pin `frag_count ≤ 16`, `frag_idx < frag_count`, partials bounded by capacity, oldest-first.

Checked and clean: no wall clock in crypto or ordering; A1 matches `seal.rs` (AD = exact header slice); `pair_mac` key order `A ‖ B` is direction-bound (no reflection); `sender_ref` collision odds at 256 contacts ≈ 7.6 × 10⁻⁶ (verified); a stock MeshCore repeater can carry the frames.

## 4. What is right (do not touch)
- Fragment-then-seal, re-seal on retransmit, ≤ 16 outstanding: the window arithmetic holds (a window-rejected packet has already been superseded).
- `(sender, stream, msg_seq)` identity with prefix + ranges; no timestamp field; receipt-time display.
- The `pair_mac` construction itself: keyed BLAKE2b-128 over header ‖ epk ‖ inner, key from `ss ‖ A ‖ B` with a domain tag — sound, direction-bound, deniable as intended. Never let `sig` replace it (B1).
- Custody receipt bound to exact sealed bytes; `relayed` only from an authenticated `CUSTODY_ACK ACCEPTED`.
- Mode S store key: HKDF off the keyset, salt = NodeId, dedicated info; honest residual statement.
