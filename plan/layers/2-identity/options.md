# Layer 2 (Identity) — Options Brief

Status: 🧭 awaiting user decisions
Source: 8-agent research sweep, 2026-10-12 (k3-256k fleet, workflow `wf_73d7f290-f74`, resumed after quota reset; ~927k subagent tokens, 365 tool calls across identity-systems · embedded-crypto · backup-recovery · wire-auth ×2 · agentic-identity · nostr-axis · mesh-case-studies · encodings-standards). Cross-checked against Layer 1 state (D8/D9, `layers/1-bootstrap/options.md`). Raw findings: workflow journal; discrepancies resolved in §Cross-check.

## Already decided — not relitigated

From `os/README.md`, the 2026-10-12 scope lock (with user), and D8/D9:

- Every node gets an automatically created cryptographic, wallet-style identity at install; it signs outgoing traffic and authenticates the node to peers. Offline-first; no phone numbers, email, or central authorities. Runs on **all 6 device categories** (incl. ESP32-S3/nRF52-class Cat 1 relays).
- Adopted from proven systems via the KB rubric; **third-party audit mandatory for `proven` on identity/keys/crypto** (D6/D7).
- Scope lock: **node/machine identity only** (no human contact/trust/profile model) · lifecycle = generate, store, use, **backup + recovery** (rotation/revocation deferred, no corner-painting) · crypto **picked on merit, NOT Nostr-locked** (D8 removed Nostr from the README; Nostr is not final) · **agentic-native**: the node's agent is a first-class key user · scalable, global, registry-free.
- D9 constraint: Layer 1's identity is a **provisional seam** (secp256k1 k256, BIP-39 mnemonic at install, NIP-06 path); a one-time re-key on Layer 2 landing is pre-accepted. Since Layer 1 is still ✍️ speccing (not built), deciding here **before** its spec lock means **zero deployed re-keys** — its D1 just gets amended.

## What research converged on (≥5 of 8 dimensions independently)

1. **Ed25519 (sign) + X25519 (ECDH) is the mesh-native identity primitive.** It is the only signature primitive that is simultaneously Cat-1-feasible (single-digit-to-tens of ms, ~10KB flash, ~3KB stack on Cortex-M4/Xtensa), available in a **third-party-audited embedded library** (Monocypher, Cure53 June 2020), and registry-free (Iroh NodeId, MeshCore, Reticulum, SSB lineage). secp256k1 has **no audited embedded path** (libsecp256k1: no audit, too heavy for bare-metal; micro-ecc: unaudited, no point validation) — under our rubric a secp256k1 fleet mechanically caps at `testing`, and Cat 1 must sign heartbeats.
2. **Pubkey-derived NodeId + signed announces.** Keys distribute via signed announces (~100–170B: full pubkey + nonce/timestamp + signature), TOFU-pinned immutable (Meshtastic CVE-2025-55293 lesson). Truncated address = locator, never authenticator; full pubkey is the trust root.
3. **Sign identity artifacts only; never routine traffic.** 64B sig + 32B key = ~40% of a ~237B LoRa payload. Traffic uses per-packet **ephemeral X25519 + AEAD** (packet-level forward secrecy with zero handshake — the only FS model that works when peers are offline for days). Replay via per-peer counters + sliding window (no clocks).
4. **Universal audit gap in mesh identity systems** (RNS, Meshtastic, MeshCore, Iroh, libp2p, Briar — none release-audited). Consequence: copy the architecture, adopt audited primitives, and our own module needs a commissioned audit to ever reach `proven`.
5. **Reticulum = best architecture, radioactive code.** The April 2025 custom "Reticulum License" is non-OSI and bans AI-training use — directly hostile to an agentic-native OS; Debian packaging died over it; bus factor 1 (founder stepping back, FOSDEM 2026). The **protocol is public domain (2016)**: clean-room design adoption is the safe path. Its identity model (dual keyset, hash-derived addressing, signed announces, per-packet ephemeral ECDH) is the top design reference.
6. **BIP-39 mnemonic + explicitly pinned derivation is the wallet-style backup.** But "BIP-39 phrase" is not a complete spec (Electrum's decade-old critique): the format must pin curve + path + encoding + version with test vectors. SLIP-0010 (hardened-only Ed25519) ≠ BIP32-Ed25519 (Cardano) — incompatible; pin one. Full HD trees buy a node identity nothing; BIP-85-style child-secret subtree reserved as escape hatch (agent key, storage key).
7. **scrypt-class KDFs are physically impossible on Cat 1** (64MiB+ vs 512KB SRAM). Key ceremonies (mnemonic display, passphrase-encrypted export) happen on commissioning devices; relays store derived keys. age-recipient-mode (encrypt TO the node's own pubkey) avoids KDF entirely. Backup blobs stored on hubs must be **signed by the node key and verified before decrypt** (Matrix 2022 key-injection lesson).
8. **Agentic authority = cold root key + root-signed, single-hop, attenuated delegation cert to a hot agent key** (GPG subkey / short-lived SSH cert / Signal signed-prekey / SPIFFE SVID — all the same shape). No delegation framework is audited (UCAN 1.0 finalized Jul 2026, Biscuit, macaroons) → adopt the **pattern** with the audited primitive, not a library.
9. **RNG/keygen integration is the #1 field-proven identity killer** (Meshtastic CVE-2025-52464: golden-image cloning + `micros()`-seeded PRNG → duplicate keypairs fleet-wide; COLDCARD 32-bit reseed). Hard rules: entropy-verified keygen at first user interaction (never factory/golden image), hardware TRNG + stirred sources, no silent PRNG fallback, weak-key self-test (ESP32-S3 TRNG is only guaranteed with RF/ADC active).
10. **Key-at-rest per class** (spec detail, not polled): ESP32-S3 = flash-encryption + Secure Boot v2 + NVS-encrypted partition (eFuse, irreversible; published VFI residual risk); nRF52840 = Rev3+ silicon, APPROTECT+ACL (pre-Rev3 bypassable, CVE-2020-27211 — flag residual physical risk); **ATECC608A is P-256-only — useless here**; NXP SE050B+ does Ed25519+secp256k1 (CC EAL 6+, ~261ms I2C sign) for hubs/high-value nodes; Cat 2 = OS keystore; Cat 4 = TPM2/SE050. Current Meshtastic/MeshCore store keys in **plain flash** — we can beat the incumbent state of the art cheaply.
11. **Encodings** (spec detail, not polled): wire/disk = raw 32B key + 2-byte multicodec varint (`0xed 0x01`) = 34B (algorithm agility + free did:key-string compat later, no multiformats dependency); machine string = lowercase hex (protocol/logs/APIs); human-transfer = **bech32 (BIP-173, NOT bech32m)** with typed HRPs, ≤90 chars, lowercase display / UPPERCASE QR; operator label = SHA-256(pubkey)→8–12 base32 chars grouped 4-4 + editable **local** alias (never gossiped; label never used for auth); no X.509/PEM/JOSE (ASN.1 weight + wall-clock validity traps on clockless nodes); metadata versioning via monotonic counters, no timestamps.
12. **Global scale is free**: 256-bit keys, birthday collision p ≈ 2⁻¹⁶¹ at 2⁴⁸ nodes. Registry-free uniqueness is a non-issue.

---

## Decision A — Identity primitive & key architecture

**Recommend A1: dual keyset — Ed25519 (signing) + X25519 (ECDH), Reticulum-shaped.** Implementations per category, all RFC 8032/X25519-interoperable so a mixed fleet cross-verifies: **Monocypher 4.x** on Cat 1 (only candidate both Cure53-audited AND MCU-feasible; CC0/BSD, single-file C99, ~10KB flash, no heap) · **ed25519-dalek v2** (+ curve25519-dalek ≥4.1.3) in Rust for the `111d` world (matches Layer 1's musl-static Rust binary) · libsodium ≥1.0.21 acceptable on Cat 2/4. Identity = 512-bit keyset (two keypairs), one seed derives both; signing and encryption identities stay separable for the deferred-rotation future.

| Option | Verdict |
|---|---|
| **A1: Ed25519+X25519 dual keyset (recommended)** | Only primitive passing the audit gate AND Cat 1; mesh-ecosystem curve family (RNS/MeshCore/Iroh); keeps every transport exit open |
| A2: keep Layer 1's provisional secp256k1/BIP340 | k256 is NCC-audited (Cat 2/4 fine) but **no audited Cat-1 path exists** → fleet caps at `testing`; locks the fleet into the Nostr curve family — contradicts D8 + scope lock |
| A3: single Ed25519 key + birational X25519 conversion (MeshCore/SSB style) | Simplest backup (one key); couples signing+encryption compromise, blocks independent rotation; dual keyset is cleaner for +32B private storage |

⚠️ Note: A1 revisits Layer-1 D4's ed25519 rejection — that rejection's premise ("permanently breaks the Nostr anchor") was removed by D8 and the scope lock.

## Decision B — NodeId form

**Recommend B1: NodeId = 128-bit truncated hash of the pubkey** (SHA-256(pubkey)[0:16]; RNS/Yggdrasil/cjdns lineage). Close call vs B2.

| Option | Verdict |
|---|---|
| **B1: truncated hash-of-pubkey (recommended)** | 16B addresses in every packet; future ML-DSA/hybrid identity keeps 16B addressing (no PQ corner); ~2⁶⁴ collision work is routing-level only — full pubkey in announces remains the authenticator |
| B2: raw 32B pubkey = NodeId (Iroh/MeshCore) | Simplest, zero hash machinery, proven at 200M+ endpoints (Iroh); 2× address bytes per packet; hardcodes 32-byte keys into addressing |

Hard rule either way: truncated/derived addresses are locators, never authenticators (Meshtastic CRC-32/MAC-derived NodeID CVE class dies here).

## Decision C — Identity's on-wire contract

**Recommend C1: sign announces/proofs/control only; traffic = per-packet ephemeral X25519 + AEAD** (the Reticulum construction, libsodium `sealed_box` shape on Cat 2/4, Monocypher equivalent on Cat 1). Keys distribute via signed announces, TOFU + immutable-once-pinned. Versioned envelope with **fail-closed crypto-mode field** (CVE-2025-53627 lesson). Replay: per-peer persistent 64-bit counter in AEAD associated data + 32-entry sliding window (flash-wear-aware batch commit). Online bidirectional links upgrade to **Noise IK** (responder key known from announces; 2 messages; EDHOC/RFC 9528 on watch for constrained). Cat 2/4 TCP: TLS 1.3 RPK acceptable. **Scope: C locks only the identity-side contract (announce format, key distribution, signer rules, envelope hooks) — the full message envelope is Layer 5's.** Envelope reserves: `signer_pubkey` field + optional delegation trailer (→ Decision E) + future ratchet-field space.

| Option | Verdict |
|---|---|
| **C1: ephemeral-ECDH + AEAD, sign-announces-only (recommended)** | Packet-level FS with zero handshake — fits store-and-forward; ~20–24% frame overhead vs 40% for signing; strictly better than Meshtastic static ECDH for +20B |
| C2: Meshtastic-minimal (static ECDH + CCM, 12B) | Smallest; **zero forward secrecy** — relays buffer ciphertext, so harvest-now-decrypt-later is the default exposure; rejected |
| C3: sign every packet | ~40% of LoRa payload; rejected |
| C4: Signal-grade ratcheting in v1 | Server-mediated (prekey bundles), state-heavy, anti-fit for multi-day-offline Cat 1; hooks reserved instead |

## Decision D — Backup & recovery (in scope)

**Recommend D1: BIP-39 mnemonic as the single root secret + our own versioned, self-describing backup spec.** 12 words default (24 for hubs); derivation **SLIP-0010 hardened-only Ed25519, pinned with test vectors** (NOT BIP32-Ed25519/Cardano — incompatible); single-key derivation (no HD tree); BIP-85-style subtree reserved for future child secrets. Format pins curve + path + encoding + version (Electrum lesson). Words + single-frame CompactSeedQR shown **at commissioning on the commissioning device** (Cat 1 relays never touch mnemonics — operator-owned identity), verify-on-write-down (Codex32-inspired type-back), no passphrase in v1 (silent-wrong-wallet trap). **Migration: D9's mnemonic survives — same words, new derivation; NodeId changes once (pre-accepted).**

| Option | Verdict |
|---|---|
| **D1: BIP-39 + SLIP-0010 + versioned spec (recommended)** | Universal interop (every wallet ingests BIP-39); deterministic paper/QR restore; zero install interaction (D9 shape kept) |
| D2: keep NIP-06 path (BIP32 secp256k1) | Only coherent with A2; NIP-06 upstream-deprecated ("prefer a single nsec") — we'd own a frozen path alone |
| D3: raw random key + encrypted export only | Simplest; loses paper restore forever (can't reverse a key into words); fails the scope lock |

**Phase-2 redundancy (pick now, build later — not v1 blockers):**
- **(i) Encrypted digital export:** age-recipient-style (encrypt TO the node's own pubkey — no scrypt, Cat-1-feasible) **or** NIP-49-construction passphrase envelope (NFKC + scrypt + XChaCha20-Poly1305; Cat 2–4 only). Blob **signed by node key, verify-before-decrypt** on restore; hub-storable (hub untrusted).
- **(ii) Guardian threshold split — user values call:** **SSKR** (Radically Open Security 2021-audited C libs, BIP-39 round-trip, embeds on ESP32; niche tooling) vs **SLIP-39** (Trezor-default since 2024, word-share UX, Sparrow/Electrum recovery; **no audit** → caps at `testing` per rubric). Research leans SSKR for rubric compliance.

## Decision E — Agentic hook (forward-compat, cheap now)

**Recommend E1: build the 3-line hook into the spec now, delegation itself deferred.** (1) NodeId binds the **root** pubkey. (2) Envelopes + handshakes carry explicit `signer_pubkey` (MAY differ from root) + a versioned, optional **delegation-proof trailer** (v1 nodes always leave empty). (3) Verifier rule fixed now: a signature is valid for node N iff `signer == root(N)`, OR `signer == agent_key` accompanied by a single-hop, root-signed, fixed-field binary cert `{version, agent_pubkey, scope, expiry-or-mesh-epoch, serial, root_signature}` — attenuation-only, **no sub-delegation**, expiry does the bulk of revocation work. The cert adds no new crypto → inherits the primitive's audit. Later agentic delegation becomes a **pure soft upgrade**: no rekeying, no identifier change, no wire break.

| Option | Verdict |
|---|---|
| **E1: hook now, delegate later (recommended)** | ~3 spec lines + one optional field; the agentic-native requirement stays unpainted-into-a-corner |
| E2: nothing — agent uses root key directly | Root becomes a hot key held by software; worst of both worlds; fleet re-key later |
| E3: adopt UCAN 1.0 / Biscuit now | Unaudited, moving (UCAN finalized Jul 2026, invocation-order bugs into 2026), DAG-CBOR/did:key weight; rejected — track, and align our scope vocabulary with UCAN command paths to keep the door open |

Adopt-now patterns for Layer 3's build (recorded): signer-daemon architecture + default-deny policy engine with durable per-scope grants and activity log (Amber/NIP-55 model).

## Decision F — Transport anchoring (the Nostr exit/bridge)

**Recommend F1: node identity stays transport-independent; any global-transport credential (nostr npub or future) is a separate per-node key bound by bidirectional cross-signed attestation.** Node key signs `{"npub X speaks for node N for transport T until epoch t"}`; X signs the mirror; both gossip in-mesh (offline-verifiable) and publish when gatewayed (Keybase-proof/Matrix-cross-signing pattern; precedented inside nostr by NIP-46's client-key/user-key split). The same attestation machinery becomes the deferred rotation/revocation machinery (AT-proto rotation-key lesson, minus the registry). Never: npub as root identity (zero-lifecycle: no rotation/revocation/recovery, five migration PRs unmerged), NIP-05 (dead offline), NIP-26 (deprecated), shared seed material (compromise cascade). L2 defines the attestation **format** only; Layer 7 decides transports.

| Option | Verdict |
|---|---|
| **F1: cross-signed attestation bridge (recommended)** | Nostr adoption costs a few hundred lines once; Nostr abandonment costs ~zero (delete gateway module); no curve lock-in |
| F2: derive transport keys from node seed (BIP-85 branch) | Simpler; transport-key compromise cascades into seed-derived siblings; weaker separation |
| F3: transport-native identity (node key = npub if L7 picks Nostr) | Rejected with A2; contradicts "Nostr is not final" |

## Decision G — Proven path & audit policy (rubric interpretation — user call)

Research found a **universal audit gap**: no mesh identity system is release-audited; and our two best embedded audit signals predate current code (Monocypher audit covers 3.1.1, current 4.0.3; dalek's Quarkslab 2019 predates v2). Options:

| Option | Verdict |
|---|---|
| **G1 (recommended): prior-version audit + formal-methods CI counts for ADOPTED PRIMITIVES; commission an audit of OUR module as the proven path** | Monocypher (Cure53 2020 + TrustInSoft/Frama-C CI) enters `testing` now, `proven` after field evidence; we budget/seek funding (NLnet-style) for an audit of the identity module (envelope + backup construction + glue). Vendor pinned, hash-verified releases (Monocypher bus factor ~1, mitigated by CC0). Also confirms: wolfCrypt FIPS 140-3 CMVP counts as third-party validation |
| G2: strict — only audits of the exact pinned version count | Identity entries sit at `testing` through v1 field deployment; honest but the rubric's proven gate stays shut for a long time |
| G3: commission the audit BEFORE building | Blocks the layer on procurement; rejected |

## Decision H — v1 build scope

**Recommend H1: spec + KB entries + Rust core + cross-implementation test vectors.** Deliverables: `os/2-identity/README.md` (spec, adversarial-reviewed) · KB entries for adopted components + case studies (all enter `candidate`; Monocypher can reach `testing` fast — audit exists) · `111id` Rust core (ed25519-dalek): keygen / sign / verify / ECDH / announce encode+verify / backup encode+restore / attestation format — consumed by Layer 1's `111d` (same Rust static world) and driving `111 identity backup/restore` · cross-implementation test-vector suite (RFC 8032 vectors + Rust↔Monocypher-C agreement vectors) so the later Cat-1 port verifies identically · the C/Monocypher Cat-1 core itself **deferred** until a layer owns ESP32 firmware, but pinned in the spec so the port is determined.

| Option | Verdict |
|---|---|
| **H1 (recommended)** | Identity machinery built once, test-vector-proven, exactly where its first consumer (Layer 1, Rust) lives; Cat-1 port pre-determined, not premature |
| H2: spec only; implementation inside Layer 1's build | Faster on paper; identity gets built ad-hoc by the bootstrap layer without its own vectors; rejected — test vectors are the point |
| H3: full dual implementation now (Rust + C + ESP32 firmware demo) | No layer owns Cat-1 firmware yet; premature; rejected |

## Cross-check & confidence notes (discrepancies found, resolved)

- **Monocypher audit:** the run-1 wire-auth agent claimed "no audit (author is explicit)" — **wrong**. Three other dimensions cite the Cure53 June 2020 audit (OTF-funded, v3.1.1; cure53.de/pentest-report_monocypher.pdf), zero findings above Medium. Resolved: audited, with the version-mismatch caveat → Decision G. (That agent also ran with the safety classifier unavailable; all its unique claims were cross-checked — this was the only material contradiction.)
- **Reticulum license:** two reports said "MIT" (stale training data). Two others cite primary sources: April 2025 flip to the custom non-OSI "Reticulum License" (GitHub Discussion #790; LICENSE text; Debian packaging abandoned; our own KB already carries `LicenseRef-Reticulum`). Resolved: code eliminated; protocol public domain.
- **age audit:** none exists (an agent caught and corrected its own false "NCC audit" memory against v1.0.0 release notes + C2SP spec). age X25519 stanzas are unauthenticated by design — never an authenticity channel.
- **TLS 1.3 RPK:** OpenSSL 3.2+ (Oct 2023) DOES support it (one report stale); fallback-only relevance anyway.
- **MeshCore license:** GPL-3.0 vs MIT conflict between reports — case-study only; verify at KB-entry drafting.
- **Benchmark ms figures** are extrapolations across chips/clocks (high confidence in rankings, medium in absolute ms) → on-device Monocypher benchmark firmware on ESP32-S3 + nRF52840 is a build-phase task, not a brief blocker.
- **RNS version:** reported 1.3.5–1.5.5 (2026); single-author project, founder stepping back (FOSDEM 2026) — bus-factor-1 noted either way.

## Deferred (recorded, not forgotten)

Full ratcheting (envelope version-space reserved; RNS per-destination ratchet lineage, NOT Signal port) · EDHOC for constrained online handshakes · hybrid PQ KEM field space (Cat 2+ only; ML-KEM ~1–2KB can't fit LoRa frames) · UCAN/Biscuit adoption (scope-vocabulary alignment now) · guardian threshold split build (D phase 2) · bc-ur animated QRs (payloads >400B) · NIP-46 as agent↔signer IPC (only if Nostr lands at L7) · human key-verification UX + petnames (human-identity layer; deterministic label + local alias meanwhile) · FROST threshold futures (both curves have drafts) · did:key interop shim (mintable from our pubkey anytime — free) · commissioned-audit procurement (output of Decision G) · ESP32-S3 VFI/glitch-detector silicon revision check · nRF5340-vs-nRF52840 TF-M ITS tradeoff · Cat-1 relay announce-verification policy (verify-before-relay vs relay-blind — with Layer 6/7) · announce-as-routing vs announce-as-identity-only (biggest cross-layer question wire-auth forces — for Layers 6/7) · per-peer state budget on Cat 1 (peer-table size caps replay windows and future ratchet state).
