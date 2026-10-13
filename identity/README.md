# Layer 2 — Identity

Every node's cryptographic self: how a node's identity is generated, stored, used, backed up, and recovered. The identity is a dual Ed25519+X25519 keyset derived from one seed; the 32-byte Ed25519 public key **is** the node (NodeId). It signs the node's announces and control artifacts, authenticates the node to peers, keys the per-packet encryption of node traffic, and anchors both the agentic-delegation hook and the transport-attestation bridge. Offline-first, registry-free, wallet-style backup — identical in kind across all six device categories, from an ESP32-S3 relay to a compute hub.

Status of this document: **spec v2 — LOCKED** (decisions: `os/plan/DECISIONS.md` D13 + D14 lock amendments incl. three user calls at lock; panel triage: `os/plan/layer-2-review.md` — 4 judges, 71 raw findings, all gating items resolved in v2; options brief: `os/plan/layer-2-options.md`; research: 8-agent sweep `wf_73d7f290-f74`, 2026-10-12).

The keywords **MUST / MUST NOT / SHOULD / MAY** are normative (RFC 2119).

## 1. Scope & non-goals

In scope (locked with the user 2026-10-12):

- **Node/machine identity only.** Lifecycle: generate · store · use (sign, authenticate peers, key traffic) · **backup · recover**.
- Runs on **all six device categories** (Cat 1 relays, 2A/2B terminals, Cat 3 bridges, 4A/4B hubs; Cat 5 is an overlay role, not hardware). One keyset shape, one rule set; only storage backends and ceremony devices differ per class.
- Offline-first and registry-free: no phone numbers, email, DNS names, CAs, or central authorities anywhere in the lifecycle. Registry-free uniqueness is a non-issue: with 256-bit keys the birthday-collision probability is ≈ 2⁻¹⁶¹ at 2⁴⁸ nodes.
- **Agentic-native**: the node's agent is a first-class key user (§9).

Hard non-goals:

- Human identity: no contact model, no trust/verification UX, no profiles, no petname system (a later layer's job; §7 ships a display label + local alias meanwhile).
- Key rotation and compromised-key revocation as operational features — **deferred**, with the no-corner-painting constraint: the E1 delegation hook (§9), the F1 attestation machinery (§10), and versioned formats everywhere keep rotation/revocation a future soft upgrade.
- The full message envelope (Layer 5), routing (Layer 6), transports (Layer 7). This spec owns: the keyset, the announce format, key distribution, signer rules, backup/recovery, and the envelope's identity hooks.

## 2. Core architecture — one seed, two keypairs; the keyset IS the node

- **Dual keyset (D13 A1):** every node identity is an Ed25519 signing keypair **plus** an X25519 ECDH keypair, Reticulum-shaped (the public-domain protocol design; its code is never adopted — non-OSI license with an AI-training clause). Signing and encryption identities stay separable for the deferred-rotation future. Single-key birational conversion is rejected (A3): it couples signing and encryption compromise and blocks independent rotation.
- **NodeId = the raw 32-byte Ed25519 public key (D13 B2, user pick over B1).** No hashing step, no truncation — Iroh/MeshCore style, proven at 200M+ endpoints (Iroh public relays). Every address field is 32 bytes; that cost is accepted, and the PQ-migration corner it creates is recorded in §15. Hard rule (B2): any derived or truncated label is a locator/display aid, **never** an authenticator (§7, §12).
- **Sizes (normative):** public keyset = 64B (32B Ed25519 pub ‖ 32B X25519 pub) · private keyset = 64B (32B Ed25519 private seed ‖ 32B X25519 private scalar) · NodeId = 32B.
- **Derivation (normative; test vectors per §14).** The node's root secret is a BIP-39 mnemonic (§8). From the mnemonic's 64-byte BIP-39 seed, **SLIP-0010** (hardened-only Ed25519 derivation, HMAC-SHA512 keyed `"ed25519 seed"`) derives:
  - the **Ed25519 signing key** at path `m/111'/0'/0'` — the key half (first 32 bytes) of the SLIP-0010 output, used directly as the RFC 8032 private seed (chain-code half discarded);
  - the **X25519 ECDH key** at sibling path `m/111'/0'/1'` — the 32-byte key half, used as the X25519 private scalar (clamped per RFC 7748 at point of use).
  - All components hardened; one account, fixed key indices (0′ = sign, 1′ = ECDH); no further HD tree (a BIP-85-style child-secret subtree is reserved as the escape hatch, §15). This is **NOT** BIP32-Ed25519/Cardano — the two are incompatible and restore different keys from the same words.
  - Registry check (recorded, panel crypto#13): SLIP-44 coin type 111 is registered to ARK, but BIP-44 layouts place coin types at `m/44'/111'/…` — disjoint from our `m/111'/0'/{0',1'}`; no BIP-43 purpose-111 registration exists. No practical collision; the backup descriptor (§8.2) pins this derivation explicitly, so no foreign software can misread it.

## 3. Primitives & implementations per category

One signature scheme (Ed25519, RFC 8032), one ECDH scheme (X25519, RFC 7748), one AEAD (XChaCha20-Poly1305, §6.4), fleet-wide. Interoperability is REQUIRED and byte-exact: a mixed fleet cross-verifies, proven by the agreement vectors of §14 before any Cat-1 port ships.

**Verification predicate (normative, fleet-wide — panel cluster crypto#3/complete#4/embedded#2).** Library defaults diverge, so this spec pins the acceptance rules: Ed25519 verification **MUST** apply the strict predicate — canonical encodings of R and A (field elements < 2²⁵⁵−19), S < ℓ, small-order A and R **rejected** (ed25519-dalek's `verify_strict`-class rules; libsodium ≥1.0.21 is strict by default; **Monocypher 4.x is NOT** — its `crypto_ed25519_check` is cofactored and accepts non-canonical/small-order A and R, so the Cat-1 core MUST wrap it with the predicate above (reference wrapper: `monocypher-harness/harness.c` `strict_verify`, T9 smoke log); dalek's default `verify()` MUST NOT be used for wire artifacts). X25519: low-order public keys **MUST** be rejected on ingress, and an all-zero shared-secret output **MUST** abort the exchange (RFC 7748 §6.1 check; x25519-dalek's contributory check is opt-in — enable it; Monocypher 4.x's `crypto_x25519` returns `void` and does NOT reject all-zero output — 3.x did — so the Cat-1 core MUST carry the low-order ingress blacklist and the all-zero abort itself). Every §14 agreement vector suite includes Wycheproof/malleability edge vectors (non-canonical S/R/A, small-order points, low-order X25519 keys) — byte-exact interop at the edges, not just the happy path. **Known residual (T9, open — DECISIONS pending):** the predicate as written leaves *mixed-order* A/R (a prime-order point plus a torsion component, neither canonical-invalid nor small-order) accepted by cofactored verifiers (Monocypher) and rejected by cofactorless ones (dalek `verify_strict`); only the key holder can mint such signatures, so the exposure is self-inflicted malleability and a Cat-1-only ghost NodeId `A+T`, not forgery. Closing it means adding a torsion-free check (`[ℓ]P = O`) to the predicate at a per-verify cost on Cat 1 — see `test-vectors/monocypher-smoke.md` F3.

| Category | Library (pinned, vendored, hash-verified) | Assurance | Notes |
|---|---|---|---|
| Cat 1 (ESP32-S3, nRF52) | **Monocypher 4.x** (C99, CC0/BSD) | Cure53 audit 2020-06 on v3.1.1 (OTF-funded; 6 findings, none above Medium, all fixed) + TrustInSoft/Frama-C formal-methods CI — counts per D13 G1 (§11) | ~10–13KB flash, ~3–5KB stack, no heap; Ed25519 verify ~45ms on Cortex-M4 class (on-device benchmark firmware is a build task, §15). Single maintainer → vendored copy + CC0 forkability is the mitigation (D14 records the rubric exception, §11). |
| Cat 2/3/4 (the `111d` Rust world) | **ed25519-dalek v2** + **curve25519-dalek ≥4.1.3** + x25519-dalek + chacha20poly1305 + blake2 (exact pins at build) | Quarkslab 2019 audit (pre-v2, ed25519-dalek marginal); v1's pubkey-oracle class structurally excluded by the v2 API | Matches Layer 1's musl-static Rust binary. RUSTSEC floors below. `verify()` is banned for wire artifacts — see the predicate above. |
| Cat 2/4 parity reference (C) | **libsodium ≥1.0.21** | PIA-funded 2017 assessment (v1.0.12/1.0.13, partial) — dated | **Parity/reference only — out of §14's vector scope unless actually linked.** Permitted APIs: `crypto_sign*`, `crypto_aead_xchacha20poly1305_ietf_*`, `crypto_generichash`. **Never** `crypto_box_seal` verbatim — it is XSalsa20-Poly1305 with a differently-derived nonce, NOT the §6.4 construction (panel seams#3/complete#9). |

RUSTSEC floors (normative): ed25519-dalek **MUST** be ≥2.0 (RUSTSEC-2022-0093: v1 `Keypair` assembly enabled private-key extraction; never expose signing with externally supplied pubkeys) · curve25519-dalek **MUST** be ≥4.1.3 (RUSTSEC-2024-0344 timing variability) · libsodium **MUST** be ≥1.0.21 (CVE-2025-69277 Ed25519 point-validation flaw). All crypto dependencies are pinned to exact versions and vendored hash-verified.

## 4. Key generation discipline — entropy-gated, never at the factory

The #1 field-proven identity killer is RNG/keygen integration, not primitive strength. **This section governs ALL key generation — identity keys at install/ceremony AND operational per-packet ephemeral keys (§6.4) and any future child keys** (panel embedded#3: under the §8.3 commissioning model, identity keygen never runs on relay silicon, while ephemeral keygen runs there forever — the rules must cover both). Normative rules:

1. Identity keygen **MUST** run at the first user interaction — install for Cat 2+, the commissioning ceremony for Cat 1 (§8) — **never** at factory time, never in golden images, never from a code path identical across units (Meshtastic CVE-2025-52464: vendor golden-image cloning + `micros()`-seeded PRNG produced duplicate keypairs across production batches; fixed by deferring keygen to first configuration and stirring real entropy).
2. Entropy **MUST** come from the hardware TRNG, stirred with independent noise sources (cycle counter, ADC noise, MAC, interrupt timing). Identity keys draw from the stirred pool directly; **operational per-packet ephemeral keys MAY use a TRNG-seeded CSPRNG with reseed + health-check discipline** (nobody burns raw TRNG reads per packet — the COLDCARD lesson is a *predictable fallback*, not CSPRNG use). **No silent weak fallback, ever** — a failed or unavailable entropy source aborts keygen loudly. Cat 2+ (Linux/Android): `getrandom()` is the named source.
3. **ESP32-S3:** the TRNG is guaranteed random only while the RF subsystem (WiFi/BT) or the SAR ADC is active; Espressif's own documentation marks the output pseudo-random-only otherwise. Firmware **MUST** enable the SAR-ADC entropy path (`bootloader_random_enable()` — a runtime/Kconfig setting, NOT an eFuse) **and/or** initialize WiFi/BT before keygen. For RF-off relay duty this is a **standing lifetime config** (power cost noted), not a keygen-time action — it covers §6.4's per-packet ephemeral generation, including the wake-from-deep-sleep window.
4. **nRF52:** hardware RNG with bias correction enabled; under SoftDevice, `sd_rand_application_vector_get()`.
5. **Weak-key self-test before persisting** (concrete checks): reject all-zero, uniform-byte, and counter-pattern outputs; for X25519 outputs, reject RFC 7748 low-order public keys. A known-weak blocklist (the Meshtastic PR7018 pattern) is **empty in v1**; when populated it distributes via the signed channel manifest (Layer 1's update machinery). A failed self-test aborts; nothing is persisted.
6. Every device generates its own keyset at its own ceremony; cloning an identity onto a second device destroys private-channel semantics fleet-wide (Meshtastic lesson) and **MUST NOT** be offered as a feature. (Restore onto a second live device is the duplicate-NodeId case — §6.2/§8.4 handle it.)

## 5. Key storage per device class

The private keyset (64B) never leaves these mechanisms: never logged, never transmitted, never on any HTTP endpoint (carried from Layer 1). **One carve-out exists — the one-time Cat-1 commissioning transit (§8.3)**: the derived keyset crosses exactly one point-to-point local channel from commissioning device to relay, under the channel minimums pinned in §8.3, and the commissioner's copy is wiped at ceremony end. Residual physical-extraction risk per class is written down in §12 — accepted explicitly, not discovered later.

| Class | At-rest mechanism (normative) |
|---|---|
| Cat 1 · ESP32-S3 | Flash encryption (XTS-AES, key in read-protected eFuse BLOCK_KEYn) + Secure Boot v2 (RSA-3072-PSS on S3) + private keyset in an **NVS-encrypted** partition. eFuse burning (SECURE_BOOT_EN, FLASH_CRYPT_CNT) is **irreversible** — provisioning scripts treat eFuse steps as one-way; dev-mode chips for bring-up. |
| Cat 1 · nRF52840 | **Rev 3+ silicon required** (pre-Rev3 APPROTECT is publicly bypassable — CVE-2020-27211, voltage-glitch debug resurrection with an open ESP32-based tool). APPROTECT + ACL; keyset in protected flash (the chip has no TrustZone/KMU — no better native option exists). Firmware **MUST** read the silicon variant from FICR at boot; on pre-Rev3 silicon the commissioning ceremony warns loudly and requires explicit operator acknowledgment, and the residual risk is recorded (node still operates). |
| Cat 1 · nRF5340 (if adopted) | TF-M internal trusted storage (~20–30KB SRAM budget); note the KMU cannot push 256-bit ECC private keys into CryptoCell-312. (Silicon choice itself: §15.) |
| Cat 2 | OS keystore where available (Android Keystore-class); otherwise the Layer-1 identity-dir contract (§13) is the floor: 0600 files in a 0700 dir, owner `node111`. |
| Cat 3 | Inherits the Cat 2 rule (bridges are Linux-class nodes in this fleet). |
| Cat 4 | **TPM2-sealed at-rest encryption** (the TPM holds the storage key, NOT the node keyset — commodity TPM2 hardware ships NIST-curve-only; resident Ed25519/X25519 custody is effectively unavailable) **or SE050B+** (native Ed25519+X25519 custody; CC EAL 6+, FIPS 140-2 L3). When neither is present, the Cat-2 contract is the floor. |

Secure-element rules: **ATECC608A is BANNED** — it is P-256-only (datasheet 40001977A) and can neither hold nor accelerate Ed25519/secp256k1. SE050 must be variant B or later (A lacks Ed25519). SE050 Ed25519 sign costs ~261ms over I2C (wolfSSL measurement) — secure elements are **never** in the packet-forwarding hot path; they sign identity/announce artifacts only.

## 6. Identity on the wire — the C1 contract

Scope: this section locks the **identity-side** contract — the announce format, key distribution, signer rules, traffic-protection requirements, and the envelope's identity hooks. The full message envelope is Layer 5's (§13).

### 6.1 What identity signs

Identity keys sign **announces, proofs, and control artifacts only — never routine traffic.** A 64B Ed25519 signature plus 32B key is 96B ≈ 40% of a ~233B LoRa payload (C3 sign-everything rejected on the frame budget). Delivery proofs and ACKs are authenticated (Meshtastic's DM-ack forgery → HMAC `ack_proof`; Reticulum delivery proofs = Ed25519 signatures over the packet hash); construction details are Layer 5's, the requirement ships from here. **Every signed artifact — here and in any future layer — MUST prepend its own ASCII domain tag inside the signed region** (the pattern is fixed at §6.2, §9, §10; Layer 5's proof/ACK constructions carry their own tags — normative, panel crypto#4/seams#5).

### 6.2 Announce format (normative)

The signature input is the ASCII domain tag `"111/announce/v1"` prepended to the wire bytes below (the tag is part of the signed payload, reconstructed by the verifier; it is not a wire field).

| Offset | Size | Field | Rule |
|---|---|---|---|
| 0 | 1 | `version` | `0x01` for v1; unknown version ⇒ reject (fail closed) |
| 1 | 32 | `sign_pubkey` | Ed25519 public key = NodeId |
| 33 | 32 | `enc_pubkey` | X25519 public key — MUST pass the §3 low-order check |
| 65 | 8 | `counter` | LE u64, strictly monotonic per node; the **announce** sequence (distinct from §6.5's per-peer traffic counters). Initial value 0; the genesis announce uses 1. **Committed synchronously (fsync) BEFORE the announce it authorizes is transmitted** — announces are rare; the batch-commit/flash-wear argument applies to per-packet traffic counters only (§6.5), never to this counter (panel seams#1: an unclean reboot with uncommitted announces otherwise strands the node at its peers permanently). Restore-time resumption: §8.4. |
| 73 | 0–32 | `appdata` | opaque to this layer; reserved for L6/7 (role flags, transport hints). Transport-MUTABLE data (e.g. hop counts) is never carried inside this signed blob (§13). |
| 73+n | 64 | `signature` | Ed25519 signature over the domain tag ‖ all preceding wire bytes |

Base 137B, max 169B — the lean ~100–170B class (MeshCore advert ≈ 100B; Reticulum announce ≈ 167B). Verification rules: the signature **MUST** verify under the §3 strict predicate before any state change (real nostr clients were caught skipping verification — EuroS&P 2025 forgery demos); `counter` **MUST** exceed the last-seen value for that NodeId (stale ⇒ drop); conflicting key material for a pinned NodeId ⇒ reject (§6.3). An announce carrying the receiver's **own** NodeId MUST NOT be pinned and MUST surface a distinct **identity-collision** error state (dashboard/`/api/status`) — two live nodes on one mnemonic is the §8.4 duplicate case, and the remedy is there.

### 6.3 Key distribution — TOFU + immutable key material, bounded store

Keys distribute exclusively via signed announces; there is no registry. The pinning store holds `{sign_pubkey, enc_pubkey, highest_announce_counter, receive_window}` per peer (no redundant `node_id` field — it IS `sign_pubkey` under B2; ≈80–130B per peer). First-seen pins. **Pinned key material is immutable while held**: any announce presenting different key material for a known NodeId is rejected (Meshtastic CVE-2025-55293: NodeInfo packets could overwrite an established contact key). The store carries no wall-clock fields.

Immutability binds **key material**, not **retention** (panel seams#6): the store is **bounded** — v1 default cap 256 peers (≈33KB flash — trivial on ESP32-S3), LRU eviction permitted. This is safe under B2 because NodeId = the public key itself: no attacker can impersonate an evicted NodeId, and re-appearance re-pins only the true key holder's material. Per-interface announce rate limits are handed to L6/7 (§13) — they gate mesh bandwidth, not store integrity.

### 6.4 Traffic protection — per-packet ephemeral X25519 + AEAD

**The sealed-packet construction (normative, byte-exact — panel crypto#1):** for each protected packet,

1. Generate a **fresh ephemeral X25519 keypair** `(epk, esk)` — esk drawn per §4's entropy discipline (TRNG-seeded CSPRNG permitted; note the ESP32-S3 wake-from-sleep RF-inactive window, §4.3).
2. `shared = X25519(esk, recipient_enc_pubkey)` — **MUST abort on all-zero `shared`** (RFC 7748 §6.1) and the recipient's announced `enc_pubkey` MUST have passed the §3 low-order check at pin time.
3. `key   = BLAKE2b-256("111/seal/v1"   ‖ shared ‖ epk ‖ rpk)`
   `nonce = BLAKE2b-192("111/seal-n/v1" ‖ epk ‖ rpk)` — the nonce is **derived, never transmitted** (a fresh ephemeral per packet makes it unique per packet).
4. `ciphertext = XChaCha20-Poly1305-Encrypt(key, nonce, plaintext, AD)` with **AD = `version ‖ crypto_mode ‖ counter(LE u64) ‖ all other cleartext header bytes** — every cleartext header byte is AD-covered. Tag 16B.
5. Wire packet = cleartext header (incl. `version`, `crypto_mode`, `counter`) ‖ `epk` (32B) ‖ ciphertext+tag.

This is a **project-pinned composition** of audited primitives — **NOT wire-compatible with libsodium `crypto_box_seal`** (that one is XSalsa20-Poly1305 with a tag-less nonce derivation). Implementations compose from `crypto_aead_xchacha20poly1305_ietf_*` / Monocypher's AEAD + generichash; nobody calls a sealed-box API verbatim. Overhead: 32B epk + 16B tag + 8B counter = **56B ≈ 24% of a 233B LoRa frame**, stated honestly (a LoRaWAN-style short-counter wire optimization is deferred, §15).

- **What this buys, stated precisely (panel crypto#2):** sender-side forward secrecy (discarded ephemerals; the sender's long-term key is uninvolved) + per-packet key independence — with **zero handshake traffic**, the only model that works when peers are offline for days. **Recipient-side residual, stated plainly:** a future compromise of the recipient's long-term X25519 key decrypts its recorded *incoming* history — this remains the default exposure until Noise IK (§6.6, online links) or deferred ratchets (§15) close it. Written into §12's residual table. Static-static ECDH as the sole protection stays banned (it adds sender-key compromise to the same exposure).
- The sender's NodeId **MUST** travel **inside** the ciphertext. A sealed box authenticates the channel, never authorship (either endpoint could have produced it); non-repudiation, if ever wanted, is a Layer-5 inner signature — MAC auth **MUST NOT** be presented as proof of authorship.
- **Crypto-mode registry (panel complete#3):** `crypto_mode` `0x01` = the construction above, pinned here. Future modes are append-only and may never weaken below 0x01's bar; the registry is owned by this layer (L2), the envelope `version` field by L5, and changes to either require identity-layer sign-off (§13). Receivers **MUST** reject unknown or weaker-than-expected modes (Meshtastic CVE-2025-53627: silent PKI→legacy downgrade displayed as "encrypted"). No legacy mode exists in v1; none may ever be added silently.

### 6.5 Replay protection — counters, not clocks

Per-peer **persistent 64-bit counter** carried in AEAD associated data (§6.4) + a **32-entry sliding acceptance window** (LoRaWAN/IPsec class; Briar confirms 32-entry windows at mesh scale). No wall-clock anywhere. Flash-wear rule (per-packet traffic counters only — the announce counter is §6.2's sync-commit rule): receive windows and send counters are **batch-committed** every N packets and on clean shutdown. After an unclean reboot, **both** sides apply a conservative forward jump with **J ≥ N** (send-side counter and receive-side high-water alike — panel crypto#7/seams#1): send-side jumps past any uncommitted emission, receive-side accepts the resulting gap harmlessly (a jump only advances the window). Residuals, stated honestly: counters do not stop delay-and-replay of still-valid frames, and a receiver's stale-by-≤N persisted window can re-accept already-consumed frames after unclean reboot — content-level idempotency (message IDs) covers both and is handed to Layer 5 (§13), with a §14 negative test.

Counter state (announce counter + per-peer traffic counters/windows) is **protocol state, not secret**: it lives in the identity dir as `counters` (§13) / the Cat-1 NVS map, batch-committed as above. Restore semantics for a state-absent recovery are §8.4's.

### 6.6 Online link upgrades

When both peers are online, links **SHOULD** upgrade to **Noise IK** (2 messages; the responder's key is already known from announces). Cat 2/4 TCP links **MAY** use TLS 1.3 raw-public-key (RFC 7250; OpenSSL ≥3.2 supports it). §12 rule 14's one-AEAD rule scopes to the packet crypto-mode (§6.4); these upgrades are separately pinned named protocols (Noise's IETF ChaCha20-Poly1305 suite and TLS suites are their own registries, not new crypto-modes). Implementation audit status of Rust Noise stacks is recorded in §11 (snow/clatter unaudited → `testing` cap; formally-verified Noise* C noted); EDHOC (RFC 9528) is the constrained-handshake standards track on watch (§15).

### 6.7 Envelope identity hooks (contracted to Layer 5)

The envelope reserves, and this spec owns the semantics of: **`signer_pubkey`** (32B; equals NodeId when the root signs) · a **versioned optional delegation-proof trailer** (§9; **v1-conformant verifiers MUST reject any envelope with a non-empty trailer** — the cert branch activates at the delegation milestone, and an undefined mesh-epoch source means expiry is never evaluated in v1) · **reserved ratchet-field space** (v1: absent; size pinned by L5 at envelope lock, §15). The trailer's empty encoding is **absent-field**, not zero-length. Everything else in the envelope is Layer 5's; the per-peer counter/window **state machine** is this layer's, its **serialization placement** in the envelope is L5's. Envelope-version and crypto-mode changes require identity-layer sign-off (§6.4).

## 7. Encodings & display

- **Wire/disk binary records:** raw 32B key prefixed with the 2-byte multicodec varint `0xed 0x01` (ed25519-pub) = **34B** (`0xec 0x01` x25519-pub where a record needs it). Algorithm agility plus free did:key-string compatibility later, **without** a multiformats dependency.
- **Machine string:** lowercase hex of the raw 32B key — the canonical form in protocol messages, logs, and APIs.
- **Human-transfer string:** **bech32 per BIP-173 — explicitly NOT bech32m** (bech32m exists only for segwit v1+ and would break interop with nostr/age tooling for zero gain). Project HRPs; v1 assigns **`node`** for the public NodeId (≈63 chars for a 32B payload — inside bech32's ≤90-char proven-checksum zone; HRP registry collision check recorded in §15). Display **lowercase**; QR transfer **UPPERCASE** (alphanumeric-mode density, BOLT11 convention); mixed case is invalid. BIP-173's insertion/deletion weakness is only closed for fixed-size payloads: decoders **MUST** enforce the expected data-part length (embedded#15).

- **Operator label:** SHA-256(pubkey) truncated to **10 base32 chars, displayed grouped `xxxx-xxxx-xx`** (pinned length — two UIs rendering different labels for the same node defeats the label's point; grouped alphanumeric measurably beats visual fingerprint art — SSH randomart underperforms, and out-of-band verification essentially never happens). The label is **display-only, never an authenticator** (Git short-hash collision attacks are the precedent). Plus an editable **local** alias per operator — stored locally, never gossiped, never authoritative (proto-petname; the real petname system belongs to the human-identity layer).
- **Metadata versioning:** monotonic version counters, **no timestamps** — air-gapped nodes lack trusted clocks, and wall-clock validity windows are the X.509 trap.
- A project-scheme URI wrapper (`<scheme>:<bech32>` with optional TLV transport hints) MAY exist as a convenience; the bare bech32 string stays canonical; no IANA registration now.
- **BANNED:** X.509 / PEM / PKCS#8 (ASN.1 parser weight + wall-clock validity semantics on clockless nodes) · JOSE/JWK (~110–150B per key + JSON machinery vs 34B raw — pure waste on 512KB-SRAM relays) · bech32m · reusing `nsec`/`npub` HRPs for our secrets (cross-protocol secret-confusion and leakage into the wrong systems — if nostr compat is wanted, it happens at the gateway in hex form, §10). COSE_Key is a fallback only if a future transport adopts EDHOC/OSCORE.

## 8. Backup & recovery (D1)

### 8.1 Root secret and derivation

The single root secret is a **BIP-39 mnemonic**: **12 words (128-bit) default, 24 words (256-bit) for hubs** (Cat 4 SHOULD use 24). Known limit, stated (panel crypto#10): the BIP-39 checksum is only 4 bits at 12 words (8 bits at 24) — ~1/16 (resp. ~1/256) of single-word transcription errors pass checksum silently, hence the NodeId confirmation MUST in §8.4. Derivation = §2: SLIP-0010 hardened-only at the pinned paths, with published test vectors (§14) — a "BIP-39 phrase" alone is not a backup spec (Electrum's decade-old critique), so the format below pins curve + path + encoding + version. Single-key derivation; the BIP-85-style subtree (agent key, storage key) is reserved, not built (§15). **No passphrase in v1** — BIP-39 passphrases carry no checksum: any typo silently derives a valid-but-different identity (the silent-wrong-wallet trap); an expert option, if ever, requires confirm-at-creation + NFKC (§15).

### 8.2 The versioned, self-describing backup format (normative)

Every backup artifact carries a **4-byte descriptor**: `version` (0x01) ‖ `curve` (0x01 = ed25519+x25519 dual keyset) ‖ `derivation` (0x01 = SLIP-0010 `m/111'/0'/{0',1'}`) ‖ `encoding` (0x01 = BIP-39 English 12 words · 0x02 = 24 words). Carried: printed under the words on paper, as the header of the QR payload, and as a `descriptor` file (0644, 4B) in the identity dir (§13). Restore **MUST** honor the descriptor and **MUST** fail closed on any unknown field; a v1 restore with no descriptor present assumes these v1 defaults. Round-trip test vectors are REQUIRED (§14).

### 8.3 Commissioning ceremony (normative)

Two timings, by category (panel seams#4/complete#18 — the gate never blocks node operation on Cat 2+, and never breaks Layer 1's zero-interaction install):

- **Cat 2+ (the node's own ceremony):** keygen is **silent at install** (Layer 1's zero-interaction shape carried). The ceremony below runs as the **post-install `111 identity backup` invocation** on a local TTY; its verification gate applies to the **backup**, never to node operation — the node is fully functional before it. Backup MUST refuse non-TTY stdout (words on piped stdout land in CI logs and shell history). On Termux the invocation runs as the Termux user (no `sudo -u node111` — single-user platform).
- **Cat 1 (commissioning ceremony, on the commissioning device — never on the relay):** the operator's computer running the `111` CLI owns the ceremony (user decision, D14). The mnemonic lives only on the commissioning device; relays **never touch mnemonics** — they receive only the derived 64B keyset and store it per §5 (relay identity is operator-owned). Order is **verify-then-provision** (step 3 completes before step 4's transfer). The relay-side keyset write MUST be atomic (commit flag); a relay MUST accept re-provisioning iff no committed keyset exists, and MUST refuse it otherwise (never overwrite, §13).

1. Entropy-verified keygen per §4.
2. The commissioning device displays the words **and** a single-frame **CompactSeedQR-style project QR** (payload = 4B descriptor ‖ raw entropy, 20–36B — one static frame, QR version 2–3 at EC level L; this is a project variant of SeedSigner's CompactSeedQR — raw-entropy-only tooling will NOT parse it, named accordingly, panel complete#12).
3. **Verify-on-write-down** (Codex32-inspired): the user types back sampled words or re-scans the QR; the backup is not considered made until verification passes.
4. **(Cat 1 only) Provision:** the derived keyset transits the commissioning channel under the §5 carve-out. Channel minimums (normative — panel crypto#5/complete#2/embedded#4): point-to-point local physical channel (USB-UART provisioning mode, or BLE with numeric-comparison pairing), **never** a routed network path, **never** broadcast radio. The ceremony MUST end with a **challenge-verify** (relay signs a commissioner-chosen nonce with the provisioned signing key — a misprovisioned relay fails loudly, not silently). The commissioner **MUST wipe** its copy of the derived keyset at ceremony end; only the event is logged, never the material.
5. No passphrase. Paper is the default medium; metal for hubs.

### 8.4 Restore

`111 identity restore` accepts words or a QR scan on the commissioning device (for Cat 2+ nodes, the node is its own commissioning device). Input hygiene (normative): words are NFKC-normalized (case/whitespace folded) and the **BIP-39 checksum MUST be validated** before derivation (its 4/8-bit weakness is §8.1's stated limit — that is why the next rule exists). Then: descriptor check (fail closed) → SLIP-0010 derivation → atomic identity-dir write (§13) → **the restored NodeId is displayed and the operator MUST confirm it against a previously recorded reference when one exists** (label or bech32). It **MUST** refuse to run against a live, unquarantined identity dir (never overwrite). A Cat 1 relay is restored by the commissioning app re-deriving the keyset and re-provisioning it per §8.3 — the relay never parses words.

**Restore recovers wire function, not just key custody** (panel BLOCKER complete#1/embedded#1). Counter rules on restore:

- **State present** (restore from a live dir image): persisted counters are reused as-is.
- **State absent** (words-only disaster recovery): the node MUST NOT announce until a counter floor is set — learned from mesh-gossiped copies of its own prior announces where a mesh exists, else an operator-supplied floor, else 0 only for a never-before-deployed identity. It then sets `counter = floor + 1024` (safety margin; over-jumping is harmless — gaps only advance windows) and applies the same jump to per-peer traffic send-counters on new sessions. Restore→re-announce and restore→send conformance vectors: §14.
- **Old-instance retirement:** the ceremony MUST instruct retiring/quarantining the old instance and MUST refuse to proceed until the operator confirms — two live instances on one keyset produce diverging counters and flapping announces fleet-wide (§6.2's rule reads each side stale after the other). If the old device powers back on anyway, §6.2's identity-collision state surfaces it.

**Compromise & loss runbook (rotation/revocation stay deferred):** a compromised mnemonic's only remedy is a **burn**: new mnemonic, new NodeId (old pins rot at peers; the new NodeId is simply a new peer; §10's attestation lineage is the designed future). Identity lost without backup is **unrecoverable by design** — stated plainly.

**Layer-1 migration (D9-format dirs):** on first run of the L2-wired binary against a provisional identity dir, a dedicated migration path reads the preserved `mnemonic` file, derives the L2 keyset (same words, new derivation — the NodeId changes once, pre-accepted D9/D13), writes the new-format files (§13), quarantines the legacy `secret.hex`/`npub`, and logs the old→new NodeId mapping. Reinstall is the alternative for throwaway dev nodes.

### 8.5 Phase-2 formats (specified now, built later — not v1 blockers)

- **(i) Recipient-mode encrypted export:** encrypt **to the node's own X25519 pubkey** with the §6.4 construction — no scrypt (log_n=16 wants ~64MiB; physically impossible on Cat 1's 512KB SRAM / 8MB PSRAM). **Purpose pinned (panel crypto#11/complete#5/embedded#17):** this is a **post-root-restore state carrier** — pinning store, config, future child secrets — NOT a root-recovery path (the root recovers from words only; the blob is decryptable only after the root is restored). Byte format: `version(1) ‖ ephemeral_pk(32) ‖ ciphertext ‖ node_sig(64)`. The blob is **signed by the node key**, and restore **MUST verify the signature before decrypting** (Matrix 2022 key-injection, GHSA-r48r-j8fx-mq2c: never trust-on-decrypt). Restore sequencing: words → root → fetch blob → verify → decrypt → state. Hub-storable — the hub is untrusted. Encrypted blobs are **never published publicly** (aggregation aids offline cracking — NIP-49's own warning). Phase-2 alternative recorded: encrypt to a distinct operator-held recovery key named in the descriptor, if a key-independent export is ever wanted.
- **(ii) SSKR guardian split:** Blockchain Commons bc-sskr (C; Radically Open Security 2021 audit, Bitmark-funded; all findings fixed; accepted residual: the 4-byte share HMAC gives computational, not information-theoretic, security; round-trips with BIP-39 — SLIP-39 cannot and is unaudited, rubric-capped at `testing`). Guardians are **user-chosen peers**; no single guardian — and no hub — can reconstruct alone (Ledger Recover lessons: opt-in, transparent, no subpoena-surface custodians). Splitting protects the **backup**, not live signing; recombination is a temporary single point of failure, stated plainly in UX copy.

## 9. Agentic hook (E1 — built now, delegation deferred)

- NodeId binds the **root** key. The root is cold: ceremony and identity-class operations only. Every signed envelope and handshake carries an explicit `signer_pubkey` that **MAY** differ from root(N), plus the versioned optional delegation-proof trailer (§6.7 — v1 verifiers MUST reject non-empty trailers).
- **Verifier rule (normative, fixed now):** a signature is valid for node N iff
  1. `signer == root(N)`, **or**
  2. `signer == agent` **and** it is accompanied by a valid **single-hop, root-signed, fixed-field binary delegation cert**.
- **Cert layout (pinned now; scope-bit semantics assigned at activation):** body = `version` (1B = 0x01) ‖ `agent_pubkey` (32B) ‖ `scope` (4B LE flags) ‖ `expiry` (8B, mesh-epoch counter) ‖ `serial` (8B LE); the signed input is `"111/delegation/v1"` ‖ body; the cert is transmitted as body ‖ `root_signature` (64B).
- Cert rules: fixed-field binary, signed **byte-exact**, **never re-serialized** before verification (the canonicalization-bug class — UCAN's JWT→DAG-CBOR move; SPIFFE CVE-2021-27099 identifier normalization; pubkeys compare byte-exact, never as parsed structures). **Attenuation-only** (scope ⊆ root's). **No sub-delegation** — exactly one hop (multi-hop chains invite confused-deputy attacks; UCAN was fixing invocation-proof-order bugs into 2026). Expiry does the bulk of revocation work and counts in **mesh-epochs, never wall-clock** (the epoch source is Layer 7's — §13; v1 never evaluates expiry, §6.7).
- **Containment:** a delegation scope **MUST** exclude re-delegation **and** identity-class operations (seed export, cert issuance, backup/restore). The agent key never mints new authority.
- **Revocation set (designed future, shaped now):** append-only gossiped set keyed by **(root_pubkey ‖ serial)** (or cert digest) — serials collide across roots otherwise; entries are **signed by the issuing root**. Stated trust dependency: expiry-by-mesh-epoch inherits the L7 epoch source's honesty — suppressing epoch advancement extends compromised-cert lifetimes; forced advancement is DoS. Both directions are Layer 7's design input.
- The cert adds no new crypto — it inherits the primitive's audit status. That is the decisive rubric point: **no delegation framework is audited** (UCAN 1.0 finalized 2026-07, Biscuit, macaroons — all cap at `testing`), so the pattern is adopted on the audited primitive, never the library. Scope vocabulary SHOULD align with UCAN command paths to keep a migration door open.
- **v1 ships the fields + the rule with the trailer always empty and v1 verifiers rejecting it**; all v1 signing is root. Activation later is a **pure soft upgrade**: no rekeying, no identifier change, no wire break; un-upgraded peers reject agent-signed traffic until updated (transition bridging, e.g. a root co-sign window, is that milestone's design). **Agent certs are disposable: post-restore they are re-issued, never re-derived** (recorded §15).
- Cat 1 relays **MAY** ignore the trailer entirely — they only check node signatures.
- Recorded for Layer 3: the **signer-daemon** pattern (v1 custody: the agent holds **zero** key material and requests signatures from the identity holder — the NIP-46-at-MCP precedent; post-activation the agent holds **only** its own attenuated key, never root or seed material) with a **default-deny policy engine**: durable per-scope grants, an activity log, background signing only after durable approval (Amber/NIP-55 model; approval-fatigue trade-off documented).

## 10. Transport attestation (F1)

Node identity is transport-independent. Any global-transport credential (a nostr npub or a future transport's key) is a **separate per-node key** bound to the node by **bidirectional cross-signed attestation** (Keybase-proof / Matrix cross-signing pattern; precedented inside nostr by NIP-46's client-key/user-key split).

**Format (normative; 202B wire):** `version` (1B = 0x01; unknown ⇒ fail closed) ‖ `node_pubkey` (32B) ‖ `transport_id` (1B registry owned by this layer, L2; 0x01 = nostr-secp256k1) ‖ `transport_key` (32B) ‖ `expiry_epoch` (8B mesh-epoch) ‖ `node_sig` (64B) ‖ `transport_sig` (64B). Signed inputs are domain-tagged: `node_sig` covers `"111/attest-node/v1" ‖ version ‖ node_pubkey ‖ transport_key ‖ transport_id ‖ expiry_epoch`; `transport_sig` covers `"111/attest-transport/v1" ‖ node_pubkey`.

- **Distribution:** gossiped in-mesh (offline-verifiable — two signature checks, no fetch) and published to the transport when gatewayed. Verification checks both signatures and the expiry epoch; expired attestations are dead.
- **Supersedure (normative — panel crypto#6):** per (`node_pubkey`, `transport_id`) the attestation with the **greatest `expiry_epoch` wins**; verifiers retain only it and reject re-publication of a superseded one. Without this rule a replayed old attestation defeats every rotation.
- **This machinery is the designed future rotation/revocation path** (the AT-proto rotation-key lesson, minus the registry): a rotation is a new attestation binding a successor key — and under B2 **rotation is key succession**: the successor key is a **new NodeId**, pinned fresh under §6.3 first-seen rules, with the old key's attestation providing verifiable lineage. A stable identifier across rotation is explicitly **NOT** provided under B2 (accepted with the user's B2 pick). Rotation itself stays deferred (§15) — the format is what ships.
- Hard rules: a transport key **MUST NOT** share seed material with the node key — no common seed, no shared derivation branch (compromise cascade; F2 rejected) · a transport credential is **never** the root identity (nostr keys have zero lifecycle: no rotation, revocation, or recovery; five migration PRs unmerged since 2024) · **NIP-05 is never load-bearing** (requires DNS+HTTPS — dead air-gapped; it identifies, never verifies) · **NIP-26 is never load-bearing** (deprecated upstream) · presence/heartbeat publication uses per-purpose transport keys — a stable public key publicly tracks the node's timeline.
- This layer defines the **format only**; Layer 7 decides transports and gateway publication policy.

## 11. Audit & the proven path (G1)

The KB rubric makes a third-party audit mandatory for `proven` on anything touching identity/keys/crypto. Research found a **universal audit gap**: no mesh identity system is release-audited, so the architecture is copied from field-proven designs while the assurance comes from audited primitives plus a commissioned audit of our own module. D13 G1 relaxes exact-version audit matching for **adopted primitives**; our own code gets no such relaxation.

| Component | Third-party assurance | KB status | Path to `proven` |
|---|---|---|---|
| Monocypher 4.x (Cat 1) | Cure53 2020-06 on **v3.1.1** (OTF; 6 findings, none above Medium) + TrustInSoft/Frama-C formal-methods CI on 4.x | fast-track **`testing`** at build (G1) | rubric field evidence (≥3 months, ≥2 device profiles, zero-WAN clean install, power-loss recovery) |
| ed25519-dalek v2 + curve25519-dalek ≥4.1.3 | Quarkslab 2019 (pre-v2, ed25519 marginal); RUSTSEC-2022-0093 class excluded by v2 API; RUSTSEC-2024-0344 fixed ≥4.1.3 | `candidate` | smoke test ⇒ `testing`; field evidence ⇒ `proven`; exact-version audit gap recorded |
| libsodium ≥1.0.21 | PIA-funded 2017 assessment (v1.0.12/1.0.13, partial) — dated; CVE-2025-69277 fixed in 1.0.21 | `candidate` | Cat 2/4 fallback; not the assurance anchor |
| k256 (future secp256k1 transport credentials, §10) | NCC Group 2023 (40 person-days; findings fixed) | `candidate` when adopted | available when Layer 7 needs secp256k1 |
| bc-sskr (phase-2 §8.5) | Radically Open Security 2021 (C libs only; accepted residual: 4-byte share HMAC) | `candidate` | field evidence when built |
| Noise implementations (§6.6 link upgrade) | snow / clatter: **unaudited** (RUSTSEC-2024-0011 fixed in snow 0.9.5) | `testing` cap per rubric | **Noise\*** (IACR ePrint 2022/607 — formally verified C for all 59 patterns) is the rubric-clean path; an implementation audit is a procurement item (§15) |
| **Our identity module (`111id`: envelope + backup construction + glue)** | none — new code | enters **`testing`** when built | **commissioned third-party audit** (NLnet-style funding is a procurement task, §15) **+** the rubric's field-evidence gates ⇒ `proven`; reports `testing` until then |

G1 also confirms: wolfCrypt's FIPS 140-3 CMVP certificate counts as third-party validation (kept as a fallback primitive source, not adopted). **D14 rubric reconciliation (user-approved 2026-10-12):** the KB's flat bus-factor ≥2 gate for `testing` gains a **vendored-primitive exception** — permitted when the component is CC0/permissive-licensed, vendored hash-pinned, and forkability is recorded (Monocypher is the case: single maintainer, CC0, vendored). The §14 agreement-vector harness run doubles as the rubric's first-party smoke-test log; `proven` remains field-evidence-only, unchanged.

## 12. Threat model & hard rules

Assets: the 64B private keyset, the mnemonic, and the integrity of the peer pinning store. Adversaries: passive collectors of buffered ciphertext (harvest-now-decrypt-later), active spoofers/MITM on radio links, malicious relays and hubs (store-and-forward is untrusted), physical extraction against unattended relays, factory/supply-chain cloning.

Normative rules, each traced to a field incident:

1. **No silent crypto downgrade.** Every packet/announce carries explicit version + mode; receivers fail closed on unknown or weaker (Meshtastic CVE-2025-53627).
2. **Pubkey-bound identity only** — never MAC-derived, self-asserted, or truncated identifiers (CVE-2025-55292; DEFCON-33 MeshMarauder pubkey-replacement demos).
3. **Pinned keys are immutable** — first-seen wins; conflicts rejected (CVE-2025-55293).
4. **No default or well-known keys, ever** (Meshtastic's public `AQ==` default PSK made "encrypted" channels plaintext).
5. **Entropy-gated keygen, never at factory/image time** (CVE-2025-52464; COLDCARD reseed) — §4, no silent PRNG fallback.
6. **AEAD everywhere** — no unauthenticated encryption on any path (Meshtastic legacy AES-CTR; MeshCore's AES-128-ECB + 2-byte truncated HMAC is what not to copy).
7. **Size-limit unauthenticated input** before any parse/decompression (Reticulum's critical BZ2 decompression bomb, patched 1.1.9). Identity wire formats are fixed-field binary for the same reason.
8. **Authenticated ACKs/proofs** (Meshtastic DM-ack forgery → `ack_proof`; Reticulum delivery proofs = signatures over packet hashes).
9. **Counter-derived or 24-byte XChaCha nonces** — never 4-byte random nonces (birthday ≈ 2^16 per peer); persistent counters make nonce rollback a non-event.
10. **No paste-your-key UX anywhere.** Words are shown only at explicit local-TTY ceremonies (commissioning, `111 identity backup`) and typed back to verify; keys are never pasted into forms ("paste your nsec" phishing is the dominant nostr compromise vector, and compromise there is permanent — here it must be unrepresentable).
11. **Encrypted backup blobs are never published publicly**, and restore verifies the node's signature **before** decrypting (Matrix 2022, GHSA-r48r-j8fx-mq2c — never trust-on-decrypt).
12. **No static-static ECDH as sole protection.** Long-term-key-only handshakes equally banned (Briar CVE-2023-33982). And the §6.4 property is stated honestly: per-packet ephemeral ECDH gives **sender-side** forward secrecy + packet independence — a future compromise of the recipient's long-term X25519 key still decrypts its recorded *incoming* history (residual below; online links upgrade to Noise IK per §6.6, ratchets are §15).
13. **MAC auth is not authorship** — sealed channels authenticate the channel; authorship needs inner signatures (Layer 5's option).
14. **One signature scheme, one ECDH scheme, one AEAD at the wire** — multi-algorithm agility is attack surface (libp2p RSA-key CPU-DoS, CVE-2023-39533). Scoped: the rule governs the §6.4 packet crypto-mode; §6.6's link upgrades (Noise IK, TLS 1.3 RPK) are separately pinned named protocols, not new crypto-modes.
15. **No wall-clock validity in identity metadata** — monotonic counters and mesh-epochs only (clockless nodes; the X.509 datetime trap).
16. **Identity is a keypair, never an immutable log** (SSB anti-lesson: feed-as-identity makes compromise permanently unrecoverable; recovery is §8, the rotation path is §10).
17. **Physical-extraction residual risks are written down per class** — accepted explicitly, never discovered later:

| Class | At-rest (§5) | Residual risk (accepted, written) |
|---|---|---|
| ESP32-S3 | flash encryption + Secure Boot v2 + NVS encryption | Published voltage-fault-injection AES-key recovery; the S3 die has no glitch detector. eFuse steps irreversible. |
| nRF52840 | APPROTECT + ACL, Rev 3+ required (boot-time FICR check, §5) | Pre-Rev3 bypass via CVE-2020-27211 (open $5-class ESP32 tool); even Rev 3+ is raise-the-bar, not SE-grade. |
| Cat 2 | OS keystore / 0600-0700 dir | Layer 1's accepted threat model applies (debuggable-build adb exposure until lockdown — `os/bootstrap/README.md` §Security & trust model). |
| Cat 3 | Cat 2 rule (Linux-class) | Same as Cat 2. |
| Cat 4 | TPM2-sealed at-rest encryption / SE050B+ resident custody | TPM2 path seals storage (key in RAM while running), not resident ECC custody; SE050 path is the strongest at-rest option (CC EAL 6+ / FIPS 140-2 L3); ~261ms I2C signing keeps SEs out of packet hot paths. |

Plus two written residuals outside the table: **recipient-side harvest-now-decrypt-later** (§6.4: sender-side FS only until Noise IK / ratchets) and **mnemonic-at-rest**: the on-disk 0600 mnemonic is the master of both keys AND of any future BIP-85 child — any future child secret is only as safe as the node's disk; hubs MAY shred the mnemonic file after a verified backup (restore re-derives the keyset regardless).

## 13. Cross-layer seams

- **Layer 1 (Bootstrap) consumes:** §4 keygen at install/commissioning; the identity-dir layout and CLI contract below. Layer 1 is **built and tested** (spec v2.1, D9/D12) against the provisional secp256k1 seam — this layer replaces that seam as a **code + spec amendment (L1 v2.2, D14)**, not a fleet event: no signed release has shipped, and any mnemonic already born under D9 survives via the §8.4 migration path (same words, new derivation, one NodeId change — pre-accepted).
  - `identity/` (0700, owner `node111`): `mnemonic` 0600 (BIP-39 words, NFKC UTF-8, space-separated, trailing newline) · `keyset` 0600 (64B private keyset — derived cache, regenerable from the mnemonic) · `nodeid` 0644 (34B record: `0xed 0x01` ‖ 32B pubkey, §7) · `descriptor` 0644 (4B, §8.2) · `counters` 0600 (announce counter + per-peer traffic counters/windows — protocol state, not secret; §6.2/§6.5 discipline) · `peers` 0600 (the §6.3 pinning store — protocol state, not secret; created on first pin, absent on a fresh identity). Atomic temp-dir-then-rename write; never overwrite. **The `counters`/`peers` byte formats are owned by the `111id` `counters`/`store` modules** — no other code hand-rolls them (a build-time drift between restore and the counters owner was caught at review; the contract test in `111id` guards it).
  - Boot integrity (carried): unreadable/corrupt identity ⇒ quarantine dir, distinct error state, **never silent re-key**; regeneration only via explicit restore.
  - **CLI verb set (pinned, complete):** `111 identity backup` (the §8.3 ceremony: words + single-frame QR on a local TTY, refuses non-TTY stdout) · `111 identity restore` (§8.4) · `111 identity commission` (Cat-1 ceremony, commissioner side, §8.3). Neither verb is reachable over HTTP, ever. **Disposition of Layer 1's provisional verbs:** `recover` is subsumed by `restore` (same quarantine-then-explicit-regeneration semantics) and does not carry over as a separate verb; `export` (NIP-49 ncryptsec) is **deleted** — scrypt at ~64MiB has no place in this fleet, and the phase-2 recipient-mode export (§8.5) replaces the concept. Code disposition: `bip32.rs` and `nip49.rs` are deleted; `identity/mod.rs` is replaced by the `111id` binding, preserving the Missing/Corrupt/Exists error-class mapping into `/health` and the quarantine semantics. Display amendments: install verify-print and the dashboard show **NodeId** (hex in APIs per §7, bech32 `node1…` for human transfer) in place of npub.
  - **`111id` module scope (normative — the v1 build deliverable, D13 H1 + panel embedded#7):** keygen · sign/verify (strict predicate, §3) · X25519 ECDH (all-zero abort) · sealed-packet construction (§6.4) · announce encode/verify · pinning store + counter state · backup/restore/migrate · attestation encode/verify · encodings (hex/bech32/label) · commission support. The host-side **Monocypher C vector/smoke harness** is an explicit deliverable alongside (it feeds §14's agreement gate and §11's smoke-test evidence).
- **Layer 3 (Local Intelligence) consumes:** the E1 hook (§9) and the recorded signer-daemon + default-deny policy-engine pattern; in v1 the agent holds zero key material and requests signatures from the identity holder (`111id` core in the `111d` world). The signer IPC (socket path, request schema, policy-engine ownership) is Layer 3's to spec.
- **Layer 4 (Interface) displays:** the §7 operator label (10 base32 chars, `xxxx-xxxx-xx`) + editable local alias, and the identity-backup action surface (CLI instruction only under the HTTP-never rule) plus the identity error states incl. identity-collision. Label and alias are display-only — never auth. Delivered 2026-10-13: the dashboard shows `node.label`, `node.bech32`, the editable local alias (`state/settings.json`), and the backup instruction card (CLI only). A non-secret 'backed up' marker remains a Layer 2 amendment (dashboard shows 'unknown — run the CLI' until then).
- **Layer 5 (Messaging & Data) owns the full envelope.** Contracted identity hooks (§6.7): `signer_pubkey`, the delegation trailer (v1: absent; non-empty ⇒ reject), ratchet-field space (size pinned by L5 at envelope lock). Also handed to L5: authorship/non-repudiation (inner signatures if wanted), content message-ID idempotency (the §6.5 delay-and-replay + rollback residuals), and group/broadcast key distribution (never static-static on node keys). Announces are L2 artifacts, not L5 envelopes. Every future L5 signed artifact carries its own domain tag (§6.1).
- **Layers 6/7 (Transport / Relay & Presence) — open questions handed over, not decided here:** Cat-1 relay announce-verification policy (verify-before-relay vs relay-blind; Ed25519 verify is Cat-1-feasible) · announce-as-routing vs announce-as-identity-only (the biggest cross-layer question the wire shape forces) · announce rate-limit policy (Reticulum caps ~2% interface bandwidth — announce flooding is the registry-free tax) · attestation gossip and gateway publication (§10) · the **mesh-epoch source** that §9/§10 expiries count in (v1 fail-mode pinned in §6.7: undefined ⇒ not evaluated, non-empty trailers rejected). `appdata` is 0–32B and versioned — the stated escape if L6/7's routing data outgrows it; transport-mutable data (hop counts) rides outside L2's signed blob. Safe to defer: v1 emits no announces at all (Layer 1's transport child has independent identity), so these gate L6/7's start, not v1's deployment.
- **Layer 0 (Knowledge):** receives the §16 entries at build.
- **Cross-layer connection check:** the 20-item checklist this spec must be re-verified against when sibling layers land lives in `os/plan/layer-2-review.md`.

## 14. Test vectors & conformance

All vectors live in `os/identity/test-vectors/` at build. Conformance gates:

- **RFC 8032 (Ed25519) + RFC 7748 (X25519)** reference vectors MUST pass on every adopted implementation.
- **Rust (dalek) ↔ Monocypher-C agreement vectors** — keygen-from-seed, sign/verify, ECDH, sealed-packet round-trip — REQUIRED before any Cat-1 port (D13 H1; the port is pre-determined, not premature). The harness run is recorded as the rubric's smoke-test log (§11, D14).
- **Edge/malleability vectors (panel cluster):** Wycheproof-class Ed25519 edges — non-canonical S/R/A, small-order A and R (strict-predicate acceptance/rejection MUST match across implementations) — plus X25519 low-order pubkey rejection and all-zero-DH abort negatives.
- **SLIP-0010 pinned-path vectors** for `m/111'/0'/0'` and `m/111'/0'/1'`: published SLIP-0010 vectors plus project-path vectors generated at build and frozen.
- **Announce encode/verify round-trip**, including the §6.2 verification rules; negatives: downgrade (unknown/weaker crypto-mode), stale counter, conflicting key (immutability), **self-NodeId rejection**, domain-tag confusion (an artifact signed under another tag MUST fail).
- **Sealed-packet round-trip** (§6.4 recipe) + negatives (wrong recipient, tampered AD/mode/counter).
- **Backup format round-trip**: mnemonic → descriptor → seed → keyset → restore → identical NodeId; descriptor-absent ⇒ v1-defaults fallback; checksum-failure and unknown-descriptor-field refusals.
- **Restore wire-function vectors:** restore→re-announce (counter floor + jump rule) and restore→send (per-peer counter resumption).
- **D9-migration round-trip:** provisional mnemonic → L2 keyset → quarantined legacy files.
- **Attestation encode/verify round-trip** + negatives (expired, field-swap, cross-node transplant, superseded-but-replayed per §10's supersedure rule). Attestation vectors cover encoding + signature verification only; expiry semantics are untestable until the mesh-epoch source lands (§15) — stated honestly.
- **Weak-key self-test vectors** (§4.5's concrete checks).
- **Sync-commit rule test** (§6.2): an announce counter value is durable before its announce is transmitted.

## 15. Deferred (recorded, not forgotten)

Full ratcheting (envelope version-space reserved; Reticulum per-destination ratchet lineage — NOT a Signal port; Signal-class prekey servers have no registry-free offline role) · EDHOC for constrained online handshakes (5+ formal-verification studies, young deployment — watch) · hybrid PQ KEM field space (Cat 2+ only; ML-KEM ~1–2KB cannot fit LoRa frames) · the B2 PQ-migration corner (raw-32B addressing hardcodes key size — accepted with the user's B2 pick) · UCAN/Biscuit adoption (unaudited, moving; scope-vocabulary alignment now) · phase-2 backup builds: recipient-mode encrypted export, SSKR guardian split (§8.5); operator-recovery-key export variant (§8.5) · BIP-85 child-secret subtree (reserved) · BIP-39 passphrase expert option (if ever: confirm-at-creation + NFKC mandatory) · bc-ur animated QRs (only if payloads exceed ~400B) · NIP-46 as agent↔signer IPC (only if nostr lands at Layer 7) · human key-verification UX + petnames (human-identity layer) · FROST threshold futures (drafts exist for both curves) · did:key interop shim (mintable from our pubkey anytime — free) · commissioned-audit procurement (NLnet-style; the §11 `proven` path) · Noise-implementation audit procurement (§11) · ESP32-S3 VFI/glitch-detector silicon-revision check · nRF5340-vs-nRF52840 TF-M ITS tradeoff · Cat-1 relay announce-verification policy and announce-as-routing vs identity-only (handed to L6/7, §13) · per-peer state budget refinement at the Cat-1-owning layer (v1 default: 256-peer cap, §6.3) · on-device Monocypher benchmark firmware (ESP32-S3 @240MHz Xtensa, nRF52840 @64MHz — published ms figures are cross-chip extrapolations) · the C/Monocypher Cat-1 core build itself (D13 H1: deferred-but-pinned until a layer owns ESP32 firmware; §14 vectors pre-determine it) · **commissioning channel protocol bytes** (channel minimums + ownership pinned in §8.3; byte-level protocol spec'd when Cat-1 firmware lands; owner: `111` CLI per D14) · LoRaWAN-style short-counter wire optimization (§6.4's honest 56B) · known-weak blocklist population path (empty in v1; manifest-distributed, §4.5) · agent-cert restore policy (certs disposable — re-issued post-restore, never re-derived) · SLIP-44/HRP registry collision re-checks as standing build gates (§2/§7 results recorded at build) · mesh-epoch source + expiry exercisability (Layer 7) · key rotation + compromised-key revocation as operational features (the whole deferred lifecycle phase — the machinery is §10, the activation is a future soft upgrade).

## 16. KB adoption list (entries written at build)

All enter `candidate` per D13 H1 (desk-research cap); Monocypher fast-tracks to `testing`. Existing seed entries (`meshtastic`, `reticulum`) are cross-linked/updated, not duplicated.

| Entry | Kind | Status at build |
|---|---|---|
| `monocypher` | project | fast-track `testing` (Cure53 + formal-methods CI, G1/D14 exception; smoke log = §14 harness) |
| `ed25519-dalek` | project | `candidate` |
| `curve25519-dalek` | project | `candidate` |
| `libsodium` | project | `candidate` |
| `slip-0010` | protocol | `candidate` |
| `bip-39` | protocol | `candidate` |
| `bc-sskr` | project | `candidate` |
| `noise-framework` | protocol (IK pattern) | `candidate` |
| `edhoc` | protocol | `candidate` (watch) |
| `reticulum-protocol` | protocol (case-study role) | `candidate` — LicenseRef-Reticulum machine-visible: code adoption banned (non-OSI, AI-training clause); public-domain protocol = clean-room design reference only |
| `meshtastic-pki` | case study | `candidate` (cautionary: its CVE cascade is our §12/§14 test plan) |
| `meshcore` | case study | `candidate` (verify license at drafting — research conflicted GPL-3.0/MIT) |
| `iroh-nodeid` | case study | `candidate` (raw-pubkey NodeId proven at 200M+ endpoints) |
| `briar-bramble` | case study | `candidate` (cited normatively in §6.5/§12; partial audits 2017/2023 noted) |

Build tasks: `os/identity/TASKS.md` (created at build start per `os/plan/EXECUTION.md`).
