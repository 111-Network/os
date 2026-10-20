# Layer 5 research, lens 4: completing the message envelope on top of Layer 2

Date 2026-10-14. Scope: 2-identity/README.md section 6 and 13 hand Layer 5 the full envelope. Marked (unverified) = not confirmed from a fetched source this session. Marked (derived) = my arithmetic or analysis from fetched facts, not a quote.

## 1. Summary

- **L2's sealed packet is Noise pattern "N": the sender is unauthenticated.** Anyone holding an announced `enc_pubkey` can seal a packet claiming to be Alice, and L2 section 6.5's per-peer windows are keyed by that claim, so a forged huge counter can poison Alice's window (derived). L5 must fix this for 0 to 16 bytes before pinning anything else (finding 1).
- **Pinned shape:** 15-byte immutable AD-covered header, L2's epk and tag, a small inner header (4-B sender ref, kind, flags, varint `msg_seq`). Hop counts live in an L6 shim outside the AD. Real overhead is 63 B, not 56. Payload per 233-B packet: 163 / 147 / 99 B (unauthenticated / pair-authenticated / signed).
- **Message identity = (sender, stream, `msg_seq`)**, not a hash or random id: O(1) dedup state, no clock, BPv7 precedent. Content hashes only for blobs.
- **v1 machinery:** one packet is the normal case; multi-packet = fragment-then-seal with SACK ranges; erasure coding (raptorq) reserved behind a `coding` field, not built.
- **Not COSE** (no XChaCha20 in the registry, about 31 B more per sealed packet) and **not MLS** for v1 (needs commit ordering). Groups v1 = pairwise fan-out.
- **No timestamps in the envelope;** order by sequence plus a causal hint, display receipt time.

## 2. Candidates

| Name | Upstream | License (verified?) | Lang | Maturity | Fit | Why |
|---|---|---|---|---|---|---|
| raptorq | github.com/cberner/raptorq | Apache-2.0 (LICENSE file + crates.io, verified) | Rust | v2.0.1 2026-03-09; pushed 2026-06-22; 1.38M downloads | adopt (deferred) | Only maintained RFC 6330 Rust crate. Needed only for broadcast/multi-source bulk. Qualcomm RFC 6330 IPR needs legal review first. |
| reed-solomon-simd | github.com/AndersTrier/reed-solomon-simd | MIT AND BSD-3-Clause (LICENSE header, verified) | Rust | v3.1.0 2025-10-14 | reference | Simpler MDS alternative for small objects. |
| openmls | github.com/openmls/openmls | MIT (LICENSE, verified) | Rust | v0.9.1 2026-10-07; pushed 2026-10-14; 1,045 stars; no standalone audit found | reference | RFC 9420 impl. Needs a delivery service for commit ordering; poor fit for partitioned LoRa groups. Possible Cat 2/4 upgrade later. |
| mls-rs | github.com/awslabs/mls-rs | Apache-2.0 (LICENSE-apache via GitHub API; crate says Apache-2.0 OR MIT) | Rust | v0.56.0 2026-08-19 | reference | Second MLS option; same DS dependency. |
| Keyhive / BeeKEM | github.com/inkandswitch/keyhive | Apache-2.0 (GitHub API) | Rust | active (pushed 2026-10-14); paper ePrint 2026/1434 (July 2026) | reference (watch) | Decentralized group key agreement for offline replicas; young, research-grade. |
| ciborium | github.com/enarx/ciborium | Apache-2.0 (ciborium/LICENSE, verified) | Rust | v0.2.2 2024-01-24; repo pushed 2026-09 | adopt (Cat 2+ app payloads only) | Serde CBOR for message bodies on IP-class nodes; never in the relay path. |
| postcard | github.com/jamesmunns/postcard | MIT and Apache-2.0 (LICENSE-MIT verified, Apache via API) | Rust | pushed 2026-09-21; 1,541 stars | reference | Fine for local state; not self-describing, so poor as an evolving wire. |
| coset | github.com/google/coset | Apache-2.0 (GitHub API) | Rust | pushed 2026-03-02 | reference | COSE types for large signed manifests, Cat 2/4 only. |
| Meshtastic firmware + protobufs | github.com/meshtastic | GPL-3.0 (verified) | C++/protobuf | pushed 2026-10-14; 8,427 stars | bridge | Prior art (16-B header, 233-B payload, ack_proof). Bridge target; envelope rides as an opaque app payload. |
| MeshCore | github.com/meshcore-dev/MeshCore | MIT (license.txt, verified) | C++ | pushed 2026-10-14; 3,854 stars | bridge | Prior art for a relay-visible header byte; 184-B payload cap constrains bridging. |
| LXMF | github.com/markqvist/LXMF | Reticulum License, non-OSI (LICENSE header, verified) | Python | pushed 2026-09-30 | reference | Format facts only (README). Code must not enter our tree (LICENSING.md); reachable as a connected service. |
| Bramble BSP/BTP | code.briarproject.org/briar/briar-spec | GPL-3.0 per KB (spec repo license unverified) | Java | specs fetched | reference | Dedup-by-hash and 32-entry windows; both lean on wall clocks, so do not copy those parts. |
| BPv7 (RFC 9171); dtn7-rs | rfc-editor.org/rfc/rfc9171 | RFC; dtn7-rs MIT/Apache-2.0 (LICENSE-MIT header verified) | Rust | pushed 2026-09-09 | reference | (source, seq) identity and no-clock rule inform `msg_seq`; CBOR primary block too heavy for LoRa. |
| COSE (RFC 9052/9053/9864) | rfc-editor.org | IETF standards | n/a | RFC 9864 Oct 2025 | reference | Rejected as the packet envelope; usable for big signed artifacts. |

## 3. Findings that change the plan

1. **L2 sealed packet = Noise "N": Source authentication level 0, "anyone, including an active attacker, could have sent the payload".** Noise "K" (`-> e, es, ss`) reaches level 1 with the sender's static key already known (it is pinned from announces), and costs no wire bytes. https://noiseprotocol.org/noise.html . L2 section 6.4 says the sender id travels inside the ciphertext and section 6.5 windows are per peer, so a forged claim reaches window state (derived from 2-identity/README.md 6.4, 6.5).
2. **Mutable fields cannot be in the AD.** L2 section 6.4 AD-covers "every cleartext header byte". Meshtastic mutates hop_limit and relay_node per hop (https://meshtastic.org/docs/overview/mesh-algo/); MeshCore carries a hop-count-sized `path` of per-hop hashes (https://github.com/meshcore-dev/MeshCore/blob/main/docs/packet_format.md). If those sat in the L5 header, relaying would break the AEAD. They belong to an L6 shim.
3. **A relay-side packet id already exists for free:** epk is a fresh random 32 B per sealed packet. `epk[0..8]` is a dedup key with no extra field (derived). Meshtastic dedups on (sender, 32-bit id), id "only needs to be unique per sender ... for a few minutes" (https://raw.githubusercontent.com/meshtastic/protobufs/master/meshtastic/mesh.proto, https://raw.githubusercontent.com/meshtastic/firmware/master/src/mesh/PacketHistory.h). That is flood suppression, not idempotency.
4. **Meshtastic moved since the L2 research:** the proto now has `Routing.ack_proof` (HMAC-SHA256, first 8 B, key = SHA-256 of the static-static X25519 secret) and `xeddsa_signed`; broadcasts are signed from firmware 2.8 at +66 B; `ack_proof_status` is "reported only and not enforced". Its `priority` is "never sent over the air", so relays cannot see class. Sources: mesh.proto above; https://meshtastic.org/docs/overview/encryption/ . Channel PSK stays AES-CTR, published default `AQ==`, no forward secrecy, any key holder can forge.
5. **COSE cannot carry L2's AEAD.** The IANA COSE table has ChaCha20/Poly1305 (24, 12-byte nonce class) and no XChaCha20 entry; EdDSA (-8) is now "Deprecated" with Ed25519 = -19 (https://www.iana.org/assignments/cose/cose.xhtml, https://www.rfc-editor.org/rfc/rfc9864.html). Hand-counted CBOR (derived): COSE_Sign1 adds 9 to 10 B of framing around the 64-B signature; COSE_Encrypt0 adds 23 B framing (incl. 12-B IV); an ephemeral COSE_Key is 40 B vs 32 raw. A COSE-shaped sealed packet is about 79 B vs L2's 48 B (epk+tag).
6. **MLS needs ordering.** RFC 9750: the group "must agree on a single MLS Commit"; strongly consistent DS breaks ties; an eventually consistent DS leaves "clients responsible for reconciliation" and forked state (sections 5.2 to 5.2.2). https://www.rfc-editor.org/rfc/rfc9750.html . The openmls README mentions no audit (https://github.com/openmls/openmls), and I found no standalone OpenMLS audit (unverified absence).
7. **Briar/Bramble is clock-dependent.** BTP key periods are numbered from the Unix epoch with a 24 h clock-difference assumption; reordering window W = 32 (per prev/current/next period); BSP message_id = BLAKE2b-256 over (group_id, int64 ms timestamp, body hash), 32 B; ACK/OFFER/REQUEST records carry 32-B ids; frames cap 1,024 B. https://code.briarproject.org/briar/briar-spec/-/raw/master/protocols/BTP.md , .../BSP.md (WebFetch got 403; fetched with curl). Briar later clamped outgoing timestamps above the last conversation message to fix skew reordering (https://code.briarproject.org/briar/briar/-/merge_requests/1310, https://code.briarproject.org/goapunk/briar/-/commit/90d984ee52721f758969894b8e70c53b325f0788.patch).
8. **BPv7 is the clean no-clock prior art:** bundle identity = source node + creation timestamp + sequence number; with no clock "creation time be set to zero" plus a Bundle Age block; sequence "should never be reset"; fragments carry offset and total length and may be reassembled at any node (RFC 9171 sections 4.2.7, 4.4.2, 5.8, 5.9). https://www.rfc-editor.org/rfc/rfc9171.html . 
9. **Reticulum numbers:** MTU 500; ENCRYPTED_MDU 383; PLAIN_MDU 464; link setup 3 packets, 297 B; ratchet adds 32 B to an announce, default interval 1,800 s, 512 retained, received ratchets expire after 30 days (clock-based). https://reticulum.network/manual/understanding.html , https://reticulum.network/manual/reference.html . LXMF overhead is 111 B (16+16+64 signature + msgpack), timestamp = float64 sender seconds, message-id = SHA-256 of dest, source, payload, not transmitted. https://github.com/markqvist/LXMF . A 500-B Reticulum packet is split over two RNode LoRa frames with a 1-B sequence/flag header (mirror source, unverified: https://git.hackliberty.org/Git-Mirrors/RNode_Firmware/src/branch/revert-4-patch-1/RNode_Firmware.ino).
10. **Meshtastic and MeshCore size anchors:** payload max 239 B per docs, `DATA_PAYLOAD_LEN = 233` in the proto; 16-B header (dest 4, sender 4, id 4, flags 1, channel hash 1, next_hop 1, relay 1); no fragmentation (TOO_LARGE error). MeshCore: 1-B header (route 2 bits, payload type 4 bits, version 2 bits), payload at most 184 B, text MAC 2 B, ACK = 4-B CRC of timestamp/text/sender pubkey, group text "contains no sender signature", MULTIPART undocumented (packet_format.md, payloads.md).
11. **LoRa economics (derived from the Semtech time-on-air formula, explicit header, CRC, CR 4/5, 125 kHz; formula not fetched this session):** a 233-B frame is 0.37 s at SF7, 1.15 s at SF9, 8.4 s at SF12. EU868 sub-bands L, M, Q allow 1%, K and N 0.1%, P 10% (https://www.thethingsnetwork.org/docs/lorawan/duty-cycle/). A 10-KB object at SF9 is about 70 packets, 80 s airtime, over 2 hours at 1%. Bulk classes belong on IP links.
12. **FEC prior art on LoRa:** LoRaWAN Fragmented Data Block Transport (TS004) uses an LDPC parity matrix, "arbitrary redundancy" (10% example), over unicast or multicast (https://lora-alliance.org/wp-content/uploads/2020/11/fragmented_data_block_transport_v1.0.0.pdf; Zephyr caps fragment size 232 B, https://codex.ac6.fr/Zephyr/xref/Zephyr-latest/subsys/lorawan/services/frag_transport.c). Rateless FUOTA: https://arxiv.org/pdf/2508.10588 . Qualcomm filed IPR disclosures for RFC 6330 (Feb 2013, Mar 2015) with conditional, non-royalty-free language in the draft-era disclosure (https://datatracker.ietf.org/ipr/2554/ and /ipr/1958; full terms not read, legal review needed).

## 4. Recommendation (pinned per item)

### Layout (all items hang off this)
- **L6 shim (owned by L6, mutable, NOT in AD):** frame_type 2 bits (announce / sealed / foreign) + hop_limit 3 bits + 3 spare = 1 B. L2 announces and sealed packets both start with a `version` byte, so frame_type must live here (seam).
- **L5 outer header, 15 B, immutable, whole header is AD:** `version` 1 | `crypto_mode` 1 | `prio_flags` 1 | `dest_hint` 4 | `counter` 8 (L2, LE). `prio_flags`: bits 7-6 relay priority (0 control, 1 interactive, 2 bulk-small, 3 bulk-large), bit 5 BCAST/GROUP, bit 4 RATCHET (v1 must be 0), rest reserved-zero and rejected if set. `dest_hint` = NodeId[0..4], a locator, never an authenticator (L2 B2). Then `epk` 32 at fixed offset 15, ciphertext, tag 16.
- **Inner plaintext:** `sender_ref` 4 | `kind` 1 | `flags` 1 | `msg_seq` varint | optional fields by flag | payload | `pair_mac` 16 or `sig` 64. TLV extensions (type 1, len 1) behind a HAS_EXT flag.
- **Budget at a 233-B L5 packet:** 63 B header+epk+tag (L2's 56 plus 7 for version, mode, flags, hint), inner header about 7. Payload 163 B (mode 0x02), 147 B (pair_mac), 99 B (signed). MeshCore's 184-B cap would need smaller fragments (open question 3).

### (1) Message identity and idempotency
Prior art: Briar 32-B hash incl. timestamp, "seen" flag per message per peer, no window (BSP); LXMF SHA-256 of dest/source/payload, untransmitted; Meshtastic 32-bit sender id, minutes-scale; BPv7 (source, time, seq). Dedup windows: Bramble W = 32 tags; L2 32 packets; Meshtastic runtime ring, 20-B records.
Options: (A) per-stream sequence, (B) random 64-bit id plus bounded recent-set, (C) content hash.
- **Pick A:** `msg_id = (sender NodeId, stream, msg_seq)`, wire cost 1 to 3 B (varint) since sender and stream are implicit in a 1:1 conversation. Receiver state per (sender, stream) is a contiguous prefix plus a bounded range list, about 24 to 48 B (derived), and it never forgets anything below the prefix. Duplicates MUST be re-ACKed, not redelivered. B forgets under capacity pruning and then redelivers old duplicates. C needs a nonce to allow repeated "ok" messages and then equals B; and costs 16 to 32 B.
- **Rollback hazard (new):** a restored sender that reuses `msg_seq` would be silently deduped. Extend L2 section 8.4 with a forward jump of 2^32 on restore or unclean loss (range list stores the gap as one entry).
- Content-addressed `object_id` = BLAKE2b-256 only for blobs (KB, update, patch, attachments), so multiple holders interoperate.

### (2) Fragmentation and erasure coding
Prior art: Meshtastic none; MeshCore MULTIPART undocumented; Reticulum 500-B MTU split in two RNode frames, Resource windowed with hash checks (sizes undocumented in the manual); BPv7 offset + total length, any-node reassembly, primary block replicated; Bramble relies on byte streams (frames at most 1,024 B); LoRaWAN TS004 fragments plus LDPC.
- **Pick:** three tiers. Tier 0, one packet, no fragment fields (the chat case). Tier 1, up to about 16 fragments (2 KB): **fragment-then-seal**, each fragment a normal sealed packet with `frag idx,count` varints (+2 B). Costs about 31 to 38% overhead, but each fragment is independently authenticated (no fragment poisoning) and resumable. Re-seal on retransmit so the 32-entry window never starves (reordering depth is bounded by the ARQ window, 16 outstanding). Tier 2, bulk objects: advert (sealed packet carrying object_id, size, chunk size, `coding`) then symmetric chunks keyed from the advert, about 22 B overhead per chunk. This needs a new L2 crypto_mode: **reserve the field and the mode number now, build later.**
- Seal-once-then-fragment is rejected: raw slices cannot be individually verified.
- **Erasure coding:** not in v1. It pays for broadcast and multi-source bulk (no per-receiver feedback; any K+epsilon symbols from any source), which is exactly TS004's multicast use. For unicast, selective-ACK ARQ is simpler and cheaper. Reserve `coding` (0 none, 1 Reed-Solomon, 2 RFC 6330) in the Tier-2 advert; pick raptorq then, after legal review.
- **MTU profiles:** fabric cap 233 B; MeshCore RAW_CUSTOM paths 184 B; rnsd-carried 383 B encrypted MDU. Relays cannot refragment sealed packets, so the sender sizes from the destination's path profile (L6 exposes it).

### (3) Receipts
Prior art: Meshtastic `ack_proof` (8-B truncated HMAC, static-static key); Reticulum signature over the packet hash (64 B); MeshCore 4-B CRC; Briar ACK records of 32-B ids.
- **Pick:** an ACK is an ordinary authenticated packet of `kind = ACK`, class 0, with a cumulative `ack_prefix` plus up to 4 SACK ranges (varints, about 4 B per range, versus Briar's 32 B per id). Authenticity is inherited from finding 1's fix (0 B in mode 0x02, the 16-B pair_mac otherwise), and `kind` sits inside the authenticated region so a data packet cannot be replayed as an ACK. Domain tags: `111/ack/v1` (receipt binding), `111/pairmac/v1`, `111/msg/v1` (inner signature). Optional flag RECEIPT_STRONG adds `BLAKE2b-128(payload)` (proof of decrypting this content; use for agent ops and patches). Optional transferable receipt: signed ACK under `111/ack-sig/v1`, 64 B, only on request. Piggyback ACKs on reverse-direction data (flag), re-ACK duplicates. Hop-by-hop implicit ACK is L6's.

### (4) Group and broadcast
Prior art: Meshtastic PSK channel (cautionary); MeshCore GRP_TXT (no sender signature); Megolm sender-key ratchet with an Ed25519 signature on every message, 229-B key export (https://gitlab.matrix.org/matrix-org/olm/-/raw/master/docs/megolm.md); MLS (finding 6); BeeKEM.
- **Options:** G0 pairwise fan-out; G1 sender keys sealed per member (about 20 B overhead per message plus a 64-B signature to prevent member forgery); G2 MLS.
- **Pick G0 for v1:** N sealed packets, each pair-authenticated, forward-secret, with a per-member ACK (reliable-first, exact delivery tracking), no key state, trivial member removal. Airtime is N times (8 members at SF9, about 9 s), fine for small LoRa groups. Reserve `GROUP` flag and a `stream_id` field for G1 (rotation = each member re-distributes its sender key, no clock). G2 only for Cat 2/4 groups behind a hub DS, later. Public broadcast is a separate kind: cleartext-by-design, always signed.

### (5) Authorship
Prior art: LXMF signs every message (64 B); Meshtastic broadcasts +66 B; Megolm 64 B; Noise K/X give level-1 (KCI-vulnerable) auth.
- **Fork:** (a) mode 0x02 = fold `ss = X25519(my_enc_sk, peer_enc_pk)` into the L2 KDF (Noise K): 0 B, but an L2 amendment (L2 section 6.4 sanctions append-only modes), sender is identified by trial over pinned peers or a hint; (b) inner `pair_mac`: 16 B, no L2 change, uses only X25519 and keyed BLAKE2b already in L2 scope, key = BLAKE2b("111/pair/v1" || ss || both pubkeys), covers header, epk and inner; (c) per-packet Ed25519 signature: 64 B, transferable.
- **Pick:** **(b) now, (a) reserved as the 16-B saver**, and (c) for the classes that need transfer: group posts, anything a hub stores for others, KB entries, patches, update manifests (always signed). Signature input = `"111/msg/v1" || sender || stream || msg_seq || kind || BLAKE2b(payload)`, so it survives re-sealing and re-fragmentation. KCI exposure equals the recipient-side residual L2 already records (section 12). `signer_pubkey` is omitted in v1 (= NodeId); the delegation trailer and TLV 0x20 are rejected.

### (6) Ratchet field
Prior art: Reticulum rolling per-destination ratchets (32 B, in announces, clock-based interval, 512 retained); Double Ratchet header = ratchet pubkey + PN + N (about 40 B, derived; the spec gives no sizes) with MAX_SKIP and skipped-key storage (https://signal.org/docs/specifications/doubleratchet/).
- **Pick:** reserve, do not build. RATCHET flag (v1 must be 0) adds a 4-B `ratchet_id` after `counter` (header 19 B, relays read the flag); inner TLV 0x10 reserves a 32-B ratchet pubkey (+2 B). Worst case 38 B.
- **Offline viability:** a Reticulum-style rolling recipient ratchet fits (no prekey server): signed ratchet updates, sender seals to the newest known one, recipient deletes private ratchets by capacity, never by time (DTN delay is unbounded). A Signal-style per-conversation DH ratchet needs two-way traffic and stateful skipped keys, which fights multi-path duplicates and re-sealing. Not v1.

### (7) Wire encoding
- **Pick:** hand-packed fixed-offset outer header (relay-parsed, AD-covered, no parser); compact binary inner header (flags, LEB128, TLVs); message bodies for Cat 2+ as deterministic CBOR (RFC 8949) via ciborium. Reject COSE for packets (finding 5); keep COSE_Sign1 (Ed25519, -19) available for large signed manifests. Protobuf and postcard are rejected as the wire (codegen weight; postcard is not self-describing, so evolution is brittle; both unverified, general knowledge).

### (8) Time without clocks
Prior art: Meshtastic text packets carry no sender time, `rx_time` is stamped by the receiving device and may be absent; LXMF sender float64 inside the signed payload; Briar sender ms timestamp plus clamping; BPv7 time zero plus Bundle Age.
- **Pick:** no timestamp field. Order = (sender, stream, `msg_seq`) per direction plus `seen_peer_seq` (varint, 1 to 3 B) giving a happens-before link across directions in 1:1; groups use up to 2 `deps` (sender_ref 4 B + varint). Display = local receipt time, labelled; an optional sender-claimed `sent_at` in the CBOR body is shown as unverified. Matches L2 rule 15.

### (9) Traffic classes
Prior art: MeshCore 4-bit payload type visible to repeaters; Meshtastic priority local-only, so relays are blind; Reticulum types plus a 2% announce budget.
- **Pick (B):** 2-bit relay priority in the header (control / interactive / bulk-small / bulk-large); fine `kind` (human, agent op, KB, update, patch, ACK) inside the ciphertext. Leaks 2 bits instead of the full application type. Policy-free: priority orders the queue and pruning, never rate-caps or expires. Alternatives: full kind in clear (leaks more); none (bulk starves chat).

### (10) What a Cat-1 relay parses
Shim (frame_type, hop_limit); `prio_flags`; `dest_hint` only if it is also an endpoint; `epk[0..8]` as dedup key (256-entry ring = 2 KB); header length from the RATCHET flag. No decryption, no signatures, no counter. Unknown `version` is forwarded blind (receivers still fail closed per L2). Announce-verify policy stays L6/7's.

### L2 sign-offs this needs
(i) AD = L5 header only, never the L6 shim; (ii) `msg_seq` jump on restore in section 8.4; (iii) domain-tag registry above; (iv) optional crypto_modes 0x02 (pair auth folded in) and a Tier-2 chunk mode, both append-only and no weaker than 0x01.

## 5. Open questions for the owner

1. Authorship: default to deniable pair authentication (16 B) with signatures only for stored, public, and code classes, or sign everything on IP links?
2. Take mode 0x02 (needs an L2 amendment) now, or ship `pair_mac` and keep 0x02 for later?
3. Canonical fabric packet cap: 233 B (Meshtastic/L2), 184 B (MeshCore bridging), or per-path profiles?
4. Groups v1: is pairwise fan-out acceptable, and what is the largest group you want LoRa to serve?
5. Is receipt-time display with an "unverified sender time" label acceptable for threads?
6. Are 4-B `dest_hint`, 2-bit priority, and the 8-B counter in cleartext acceptable metadata?
7. Is bulk (KB, updates) expected over LoRa at all, or IP links only in v1?
8. Appetite for RaptorQ given the Qualcomm IPR disclosures; or Reed-Solomon instead?
9. Recipient-side forward secrecy (rolling ratchets) in v1 or deferred?

## 6. Sources

Repo: 2-identity/README.md (6, 8.4, 12, 13, 15), LICENSING.md, plan/cross-layer/devices.md, plan/DECISIONS.md (D13, D17), 0-knowledge/ entries for briar-bramble, meshcore, reticulum-protocol.
- https://github.com/meshcore-dev/MeshCore/blob/main/docs/packet_format.md
- https://github.com/meshcore-dev/MeshCore/blob/main/docs/payloads.md
- https://meshtastic.org/docs/overview/mesh-algo/
- https://meshtastic.org/docs/overview/encryption/
- https://raw.githubusercontent.com/meshtastic/protobufs/master/meshtastic/mesh.proto
- https://raw.githubusercontent.com/meshtastic/firmware/master/src/mesh/PacketHistory.h
- https://reticulum.network/manual/understanding.html
- https://reticulum.network/manual/reference.html
- https://github.com/markqvist/LXMF
- https://code.briarproject.org/briar/briar-spec/-/raw/master/protocols/BTP.md
- https://code.briarproject.org/briar/briar-spec/-/raw/master/protocols/BSP.md
- https://raw.githubusercontent.com/briar/briar/master/bramble-api/src/main/java/org/briarproject/bramble/api/sync/Message.java
- https://code.briarproject.org/briar/briar/-/merge_requests/1310
- https://www.rfc-editor.org/rfc/rfc9171.html
- https://www.rfc-editor.org/rfc/rfc9052.html
- https://www.rfc-editor.org/rfc/rfc9864.html
- https://www.iana.org/assignments/cose/cose.xhtml
- https://www.rfc-editor.org/rfc/rfc9750.html
- https://noiseprotocol.org/noise.html
- https://signal.org/docs/specifications/doubleratchet/
- https://gitlab.matrix.org/matrix-org/olm/-/raw/master/docs/megolm.md
- https://github.com/cberner/raptorq ; https://crates.io/api/v1/crates/raptorq
- https://datatracker.ietf.org/ipr/2554/ ; https://datatracker.ietf.org/ipr/1958
- https://github.com/openmls/openmls ; https://crates.io/api/v1/crates/openmls ; https://crates.io/api/v1/crates/mls-rs
- https://inkandswitch.com/keyhive/notebook/02 ; https://eprint.iacr.org/2026/1434
- https://lora-alliance.org/wp-content/uploads/2020/11/fragmented_data_block_transport_v1.0.0.pdf
- https://codex.ac6.fr/Zephyr/xref/Zephyr-latest/subsys/lorawan/services/frag_transport.c
- https://arxiv.org/pdf/2508.10588
- https://www.thethingsnetwork.org/docs/lorawan/duty-cycle/
- https://git.hackliberty.org/Git-Mirrors/RNode_Firmware/src/branch/revert-4-patch-1/RNode_Firmware.ino
- License checks: GitHub license API and raw LICENSE files for every repo in section 2
