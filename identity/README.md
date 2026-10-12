# Layer 2 — Identity

Every node's cryptographic self: how a node's identity is generated, stored, used, backed up, and recovered. The identity is a dual Ed25519+X25519 keyset derived from one seed; the 32-byte Ed25519 public key **is** the node (NodeId). It signs the node's announces and control artifacts, authenticates the node to peers, keys the per-packet encryption of node traffic, and anchors both the agentic-delegation hook and the transport-attestation bridge. Offline-first, registry-free, wallet-style backup — identical in kind across all six device categories, from an ESP32-S3 relay to a compute hub.

Status of this document: **draft v1 — adversarial review pending** (decisions: `os/plan/DECISIONS.md` D13; options brief: `os/plan/layer-2-options.md`; research: 8-agent sweep `wf_73d7f290-f74`, 2026-10-12).

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
  - the **Ed25519 signing key** at path `m/111'/0'/0'` — the 32-byte SLIP-0010 output used directly as the RFC 8032 private seed;
  - the **X25519 ECDH key** at sibling path `m/111'/0'/1'` — the 32-byte output used as the X25519 private scalar (clamped per RFC 7748 at point of use).
  - All components hardened; one account, fixed key indices (0′ = sign, 1′ = ECDH); no further HD tree (a BIP-85-style child-secret subtree is reserved as the escape hatch, §15). This is **NOT** BIP32-Ed25519/Cardano — the two are incompatible and restore different keys from the same words.

<!-- FLAG for the panel: D13 pins "SLIP-0010 hardened-only at a pinned path" but leaves the exact path and the seed→X25519 construction to this spec; the brief's research notes both as open items. m/111'/0'/{0',1'} is the minimal project-owned pin (purpose 111', account 0', key index 0'/1'). A SLIP-44 registry collision check was NOT part of the research sweep — run it at build. -->

## 3. Primitives & implementations per category

One signature scheme (Ed25519, RFC 8032), one ECDH scheme (X25519, RFC 7748), fleet-wide. Interoperability is REQUIRED and byte-exact: a mixed fleet cross-verifies, proven by the agreement vectors of §14 before any Cat-1 port ships.

| Category | Library (pinned, vendored, hash-verified) | Assurance | Notes |
|---|---|---|---|
| Cat 1 (ESP32-S3, nRF52) | **Monocypher 4.x** (C99, CC0/BSD) | Cure53 audit 2020-06 on v3.1.1 (OTF-funded; 6 findings, none above Medium, all fixed) + TrustInSoft/Frama-C formal-methods CI — counts per D13 G1 (§11) | ~10–13KB flash, ~3KB stack, no heap; Ed25519 verify ~45ms on Cortex-M4 class (on-device benchmark firmware is a build task, §15). Single maintainer → vendored copy + CC0 forkability is the mitigation. |
| Cat 2/3/4 (the `111d` Rust world) | **ed25519-dalek v2** + **curve25519-dalek ≥4.1.3** | Quarkslab 2019 audit (pre-v2, ed25519-dalek marginal); v1's pubkey-oracle class structurally excluded by the v2 API | Matches Layer 1's musl-static Rust binary. RUSTSEC floors below. |
| Cat 2/4 (C acceptable) | **libsodium ≥1.0.21** | PIA-funded 2017 assessment (v1.0.12/1.0.13, partial) — dated | Fallback/parity option, not the fleet's assurance anchor. High-level `crypto_sign*`/`crypto_box*` APIs only. |

RUSTSEC floors (normative): ed25519-dalek **MUST** be ≥2.0 (RUSTSEC-2022-0093: v1 `Keypair` assembly enabled private-key extraction; never expose signing with externally supplied pubkeys) · curve25519-dalek **MUST** be ≥4.1.3 (RUSTSEC-2024-0344 timing variability) · libsodium **MUST** be ≥1.0.21 (CVE-2025-69277 Ed25519 point-validation flaw). All crypto dependencies are pinned to exact versions and vendored hash-verified.

## 4. Key generation discipline — entropy-gated, never at the factory

The #1 field-proven identity killer is RNG/keygen integration, not primitive strength. Normative rules:

1. Keygen **MUST** run at the first user interaction — install for Cat 2+, the commissioning ceremony for Cat 1 (§8) — **never** at factory time, never in golden images, never from a code path identical across units (Meshtastic CVE-2025-52464: vendor golden-image cloning + `micros()`-seeded PRNG produced duplicate keypairs across production batches; fixed by deferring keygen to first configuration and stirring real entropy).
2. Entropy **MUST** come from the hardware TRNG, stirred with independent noise sources (cycle counter, ADC noise, MAC, interrupt timing). **No silent PRNG fallback, ever** (COLDCARD: predictable fallback + 32-bit reseed destroyed seed quality) — a failed or unavailable TRNG aborts keygen loudly.
3. **ESP32-S3:** the TRNG is guaranteed random only while the RF subsystem (WiFi/BT) or the SAR ADC is active; Espressif's own documentation marks the output pseudo-random-only otherwise. Firmware **MUST** burn `bootloader_random_enable()` and/or initialize WiFi/BT **before** keygen.
4. **nRF52:** hardware RNG with bias correction enabled; under SoftDevice, `sd_rand_application_vector_get()`.
5. **Weak-key self-test before persisting:** reject all-zero, sequential, and known-compromised outputs (the Meshtastic PR7018 pattern: wipe keys matching known-weak hashes). A failed self-test aborts; nothing is persisted.
6. Every device generates its own keyset at its own ceremony; cloning an identity onto a second device destroys private-channel semantics fleet-wide (Meshtastic lesson) and **MUST NOT** be offered as a feature.

## 5. Key storage per device class

The private keyset (64B) never leaves these mechanisms: never logged, never transmitted, never on any HTTP endpoint (carried from Layer 1). Residual physical-extraction risk per class is written down in §12 — accepted explicitly, not discovered later.

| Class | At-rest mechanism (normative) |
|---|---|
| Cat 1 · ESP32-S3 | Flash encryption (XTS-AES, key in read-protected eFuse BLOCK_KEYn) + Secure Boot v2 (RSA-3072-PSS on S3) + private keyset in an **NVS-encrypted** partition. eFuse burning (SECURE_BOOT_EN, FLASH_CRYPT_CNT) is **irreversible** — provisioning scripts treat eFuse steps as one-way; dev-mode chips for bring-up. |
| Cat 1 · nRF52840 | **Rev 3+ silicon required** (pre-Rev3 APPROTECT is publicly bypassable — CVE-2020-27211, voltage-glitch debug resurrection with an open ESP32-based tool). APPROTECT + ACL; keyset in protected flash (the chip has no TrustZone/KMU — no better native option exists). |
| Cat 1 · nRF5340 (if adopted) | TF-M internal trusted storage (~20–30KB SRAM budget); note the KMU cannot push 256-bit ECC private keys into CryptoCell-312. (Silicon choice itself: §15.) |
| Cat 2 | OS keystore where available (Android Keystore-class); otherwise the Layer-1 identity-dir contract (§13) is the floor: 0600 files in a 0700 dir, owner `node111`. |
| Cat 4 | TPM2 or **SE050B+** option (Ed25519+X25519 capable; CC EAL 6+, FIPS 140-2 L3). |

Secure-element rules: **ATECC608A is BANNED** — it is P-256-only (datasheet 40001977A) and can neither hold nor accelerate Ed25519/secp256k1. SE050 must be variant B or later (A lacks Ed25519). SE050 Ed25519 sign costs ~261ms over I2C (wolfSSL measurement) — secure elements are **never** in the packet-forwarding hot path; they sign identity/announce artifacts only.

## 6. Identity on the wire — the C1 contract

Scope: this section locks the **identity-side** contract — the announce format, key distribution, signer rules, traffic-protection requirements, and the envelope's identity hooks. The full message envelope is Layer 5's (§13).

### 6.1 What identity signs

Identity keys sign **announces, proofs, and control artifacts only — never routine traffic.** A 64B Ed25519 signature plus 32B key is 96B ≈ 40% of a ~233B LoRa payload (C3 sign-everything rejected on the frame budget). Delivery proofs and ACKs are authenticated (Meshtastic's DM-ack forgery → HMAC `ack_proof`; Reticulum delivery proofs = Ed25519 signatures over the packet hash); construction details are Layer 5's, the requirement ships from here.

### 6.2 Announce format (normative)

| Offset | Size | Field | Rule |
|---|---|---|---|
| 0 | 1 | `version` | `0x01` for v1; unknown version ⇒ reject (fail closed) |
| 1 | 32 | `sign_pubkey` | Ed25519 public key = NodeId |
| 33 | 32 | `enc_pubkey` | X25519 public key |
| 65 | 8 | `counter` | LE u64, strictly monotonic per node; the **announce** sequence (distinct from §6.5's per-peer traffic counters); persists across reboots with the §6.5 batch-commit + conservative-forward-jump rule (§6.5) |
| 73 | 0–32 | `appdata` | opaque to this layer; reserved for L6/7 (role flags, transport hints) |
| 73+n | 64 | `signature` | Ed25519 signature over **all preceding bytes** |

Base 137B, max 169B — the lean ~100–170B class (MeshCore advert ≈ 100B; Reticulum announce ≈ 167B). Verification rules: the signature **MUST** verify before any state change (real nostr clients were caught skipping verification — EuroS&P 2025 forgery demos); `counter` **MUST** exceed the last-seen value for that NodeId (stale ⇒ drop); conflicting key material for a pinned NodeId ⇒ reject (§6.3).

### 6.3 Key distribution — TOFU + immutable-once-pinned

Keys distribute exclusively via signed announces; there is no registry. The pinning store holds `{node_id, sign_pubkey, enc_pubkey, highest_announce_counter}` per peer. First-seen pins. **Pinned entries are immutable**: any announce presenting different key material for a known NodeId is rejected (Meshtastic CVE-2025-55293: NodeInfo packets could overwrite an established contact key). The store carries no wall-clock fields.

### 6.4 Traffic protection — per-packet ephemeral X25519 + AEAD

- Each protected packet uses a **fresh ephemeral X25519 keypair**; ECDH against the recipient's announced `enc_pubkey` keys the AEAD. The 32B ephemeral public key rides with the packet (sealed-box construction). This buys packet-level forward secrecy with **zero handshake traffic** — the only FS model that works when peers are offline for days on a store-and-forward mesh. Static-static ECDH as the sole protection is banned (C2 rejected: one stolen node key decrypts its entire recorded DM history — harvest-now-decrypt-later is the default exposure here, not an edge case).
- AEAD = **XChaCha20-Poly1305**, the single v1 wire AEAD (one cipher at the wire: multi-algorithm agility is attack surface — libp2p's RSA-key CPU-DoS CVE-2023-39533). The 24-byte nonce is **derived, never transmitted**: `nonce = BLAKE2b-192(ephemeral_pubkey ‖ recipient_pubkey)` (the sealed-box construction — a fresh ephemeral key per packet makes the derived nonce unique per packet); receivers derive identically. **Never** 4-byte random nonces (birthday collision ≈ 2^16 messages per peer); per-packet ephemeral keys make nonce rollback a non-event.
- The sender's NodeId **MUST** travel **inside** the ciphertext. A sealed box authenticates the channel, never authorship (either endpoint could have produced it); non-repudiation, if ever wanted, is a Layer-5 inner signature — MAC auth **MUST NOT** be presented as proof of authorship.
- Cost: ~44–56B per packet (32B ephemeral key + tag + counter) ≈ 20–24% of a LoRa frame — strictly better than Meshtastic's 12B/zero-FS floor for +20B, and half of sign-everything.
- **Crypto-mode field, fail-closed:** every packet carries an explicit version + crypto-mode; receivers **MUST** reject unknown or weaker-than-expected modes (Meshtastic CVE-2025-53627: PKI messages silently downgraded to legacy AES-CTR while displaying "encrypted"). No legacy mode exists in v1; none may ever be added silently.

<!-- FLAG for the panel: the brief calls AES-256-CCM "acceptable where ESP32 hardware AES matters"; v1 pins one AEAD (XChaCha20-Poly1305) fleet-wide for guaranteed cross-category interop and minimal agility surface, reading D13 C1's "AEAD (sealed_box construction)" as singular. -->

### 6.5 Replay protection — counters, not clocks

Per-peer **persistent 64-bit counter** carried in AEAD associated data + a **32-entry sliding acceptance window** (LoRaWAN/IPsec class; Briar confirms 32-entry windows at mesh scale). No wall-clock anywhere. Flash-wear rule: receive windows and counters are **batch-committed** (every N packets and on clean shutdown, with a conservative forward jump after unclean reboot) — per-message flash writes kill Cat-1 flash. Residual, stated honestly: counters do not stop delay-and-replay of still-valid frames; content-level idempotency (message IDs) is handed to Layer 5 (§13).

### 6.6 Online link upgrades

When both peers are online, links **SHOULD** upgrade to **Noise IK** (2 messages; the responder's key is already known from announces). Cat 2/4 TCP links **MAY** use TLS 1.3 raw-public-key (RFC 7250; OpenSSL ≥3.2 supports it). Implementation audit status of Rust Noise stacks is recorded in §11; EDHOC (RFC 9528) is the constrained-handshake standards track on watch (§15).

### 6.7 Envelope identity hooks (contracted to Layer 5)

The envelope reserves, and this spec owns the semantics of: **`signer_pubkey`** (32B; equals NodeId when the root signs) · a **versioned optional delegation-proof trailer** (§9; v1 nodes always leave it empty) · **reserved ratchet-field space** (§15). Everything else in the envelope is Layer 5's.

## 7. Encodings & display

- **Wire/disk binary records:** raw 32B key prefixed with the 2-byte multicodec varint `0xed 0x01` (ed25519-pub) = **34B** (`0xec 0x01` x25519-pub where a record needs it). Algorithm agility plus free did:key-string compatibility later, **without** a multiformats dependency.
- **Machine string:** lowercase hex of the raw 32B key — the canonical form in protocol messages, logs, and APIs.
- **Human-transfer string:** **bech32 per BIP-173 — explicitly NOT bech32m** (bech32m exists only for segwit v1+ and would break interop with nostr/age tooling for zero gain). Project HRPs; v1 assigns **`node`** for the public NodeId (≈63 chars for a 32B payload — inside bech32's ≤90-char proven-checksum zone). Display **lowercase**; QR transfer **UPPERCASE** (alphanumeric-mode density, BOLT11 convention); mixed case is invalid.

<!-- FLAG for the panel: the research lists the exact HRP set as an open question pending a registry collision check; 'node' is the minimal v1 assignment — verify at build. -->

- **Operator label:** SHA-256(pubkey) truncated to 8–12 base32 chars, grouped 4-4 (e.g. `abcd-efgh`) — grouped alphanumeric measurably beats visual fingerprint art (SSH randomart underperforms; out-of-band verification essentially never happens). The label is **display-only, never an authenticator** (Git short-hash collision attacks are the precedent). Plus an editable **local** alias per operator — stored locally, never gossiped, never authoritative (proto-petname; the real petname system belongs to the human-identity layer).
- **Metadata versioning:** monotonic version counters, **no timestamps** — air-gapped nodes lack trusted clocks, and wall-clock validity windows are the X.509 trap.
- A project-scheme URI wrapper (`<scheme>:<bech32>` with optional TLV transport hints) MAY exist as a convenience; the bare bech32 string stays canonical; no IANA registration now.
- **BANNED:** X.509 / PEM / PKCS#8 (ASN.1 parser weight + wall-clock validity semantics on clockless nodes) · JOSE/JWK (~110–150B per key + JSON machinery vs 34B raw — pure waste on 512KB-SRAM relays) · bech32m · reusing `nsec`/`npub` HRPs for our secrets (cross-protocol secret-confusion and leakage into the wrong systems — if nostr compat is wanted, it happens at the gateway in hex form, §10). COSE_Key is a fallback only if a future transport adopts EDHOC/OSCORE.

## 8. Backup & recovery (D1)

### 8.1 Root secret and derivation

The single root secret is a **BIP-39 mnemonic**: **12 words (128-bit) default, 24 words (256-bit) for hubs** (Cat 4 SHOULD use 24). Derivation = §2: SLIP-0010 hardened-only at the pinned paths, with published test vectors (§14) — a "BIP-39 phrase" alone is not a backup spec (Electrum's decade-old critique), so the format below pins curve + path + encoding + version. Single-key derivation; the BIP-85-style subtree (agent key, storage key) is reserved, not built (§15). **No passphrase in v1** — BIP-39 passphrases carry no checksum: any typo silently derives a valid-but-different identity (the silent-wrong-wallet trap); an expert option, if ever, requires confirm-at-creation + NFKC (§15).

### 8.2 The versioned, self-describing backup format (normative)

Every backup artifact carries a **4-byte descriptor**: `version` (0x01) ‖ `curve` (0x01 = ed25519+x25519 dual keyset) ‖ `derivation` (0x01 = SLIP-0010 `m/111'/0'/{0',1'}`) ‖ `encoding` (0x01 = BIP-39 English 12 words · 0x02 = 24 words). Carried: printed under the words on paper, as the header of the QR payload, and alongside the identity dir. Restore **MUST** honor the descriptor and **MUST** fail closed on any unknown field; a v1 restore with no descriptor present assumes these v1 defaults. Round-trip test vectors are REQUIRED (§14).

### 8.3 Commissioning ceremony (normative)

1. Entropy-verified keygen per §4.
2. The **commissioning device** displays the words **and** a single-frame CompactSeedQR (payload = 4B descriptor ‖ raw entropy, 20–36B — one static QR frame, version 2–3 at EC level L). For a Cat 1 relay the ceremony runs **on the commissioning device, never on the relay**: Cat 1 relays **never touch mnemonics** — they receive only the derived 64B keyset over the local commissioning channel and store it per §5 (relay identity is operator-owned).
3. **Verify-on-write-down** (Codex32-inspired): the user types back sampled words or re-scans the QR; the ceremony does not complete until verification passes.
4. No passphrase. Paper is the default medium; metal for hubs.

<!-- FLAG for the panel: the brief does not choose between on-relay keygen with a one-time mnemonic export versus commissioner-side key generation. This spec adopts the reading most consistent with D13's "Cat 1 relays never touch mnemonics": the mnemonic lives only on the commissioning device. The §4 on-chip TRNG rules remain normative wherever keygen runs on ESP32-S3/nRF52 silicon. -->

### 8.4 Restore

`111 identity restore` accepts words or a QR scan on the commissioning device (for Cat 2+ nodes, the node is its own commissioning device): descriptor check (fail closed) → SLIP-0010 derivation → atomic identity-dir write (§13) → NodeId displayed for confirmation. It **MUST** refuse to run against a live, unquarantined identity dir (never overwrite). A Cat 1 relay is restored by the commissioning app re-deriving the keyset and re-provisioning it over the local link — the relay never parses words. Layer-1 migration note: a mnemonic born under D9's provisional scheme survives the derivation switch — **same words, new key; the NodeId changes once** (pre-accepted, D9/D13).

### 8.5 Phase-2 formats (specified now, built later — not v1 blockers)

- **(i) Recipient-mode encrypted export:** encrypt the backup **TO the node's own X25519 pubkey** — no scrypt (log_n=16 wants ~64MiB; physically impossible on Cat 1's 512KB SRAM / 8MB PSRAM). The blob is **signed by the node key**, and restore **MUST verify the signature before decrypting** (Matrix 2022 key-injection, GHSA-r48r-j8fx-mq2c: never trust-on-decrypt). Hub-storable — the hub is untrusted. Encrypted blobs are **never published publicly** (aggregation aids offline cracking — NIP-49's own warning).
- **(ii) SSKR guardian split:** Blockchain Commons bc-sskr (C; Radically Open Security 2021 audit, Bitmark-funded; all findings fixed; accepted residual: the 4-byte share HMAC gives computational, not information-theoretic, security; round-trips with BIP-39 — SLIP-39 cannot and is unaudited, rubric-capped at `testing`). Guardians are **user-chosen peers**; no single guardian — and no hub — can reconstruct alone (Ledger Recover lessons: opt-in, transparent, no subpoena-surface custodians). Splitting protects the **backup**, not live signing; recombination is a temporary single point of failure, stated plainly in UX copy.

## 9. Agentic hook (E1 — built now, delegation deferred)

- NodeId binds the **root** key. The root is cold: ceremony and identity-class operations only. Every signed envelope and handshake carries an explicit `signer_pubkey` that **MAY** differ from root(N), plus the versioned optional delegation-proof trailer (§6.7).
- **Verifier rule (normative, fixed now):** a signature is valid for node N iff
  1. `signer == root(N)`, **or**
  2. `signer == agent` **and** it is accompanied by a valid **single-hop, root-signed, fixed-field binary delegation cert** `{version, agent_pubkey, scope, expiry (mesh-epoch counter), serial, root_signature}`.
- Cert rules: fixed-field binary, signed **byte-exact**, **never re-serialized** before verification (the canonicalization-bug class — UCAN's JWT→DAG-CBOR move; SPIFFE CVE-2021-27099 identifier normalization; pubkeys compare byte-exact, never as parsed structures). **Attenuation-only** (scope ⊆ root's). **No sub-delegation** — exactly one hop (multi-hop chains invite confused-deputy attacks; UCAN was fixing invocation-proof-order bugs into 2026). Expiry does the bulk of revocation work and counts in **mesh-epochs, never wall-clock** (the epoch source is Layer 7's — §13).
- **Containment:** a delegation scope **MUST** exclude re-delegation **and** identity-class operations (seed export, cert issuance, backup/restore). The agent key never mints new authority.
- The cert adds no new crypto — it inherits the primitive's audit status. That is the decisive rubric point: **no delegation framework is audited** (UCAN 1.0 finalized 2026-07, Biscuit, macaroons — all cap at `testing`), so the pattern is adopted on the audited primitive, never the library. Scope vocabulary SHOULD align with UCAN command paths to keep a migration door open.
- **v1 ships the fields + the rule with the trailer always empty**; all v1 signing is root. Activation later is a **pure soft upgrade**: no rekeying, no identifier change, no wire break; un-upgraded peers simply reject agent-signed traffic until updated (transition bridging, e.g. a root co-sign window, is that milestone's design). The **append-only gossiped revocation set keyed by cert serial** (UCAN-revocation-shaped, minimal) is the designed future, not v1.
- Cat 1 relays **MAY** ignore the trailer entirely — they only check node signatures.
- Recorded for Layer 3: the **signer-daemon** pattern (the agent holds **zero** key material and requests signatures — the NIP-46-at-MCP precedent) with a **default-deny policy engine**: durable per-scope grants, an activity log, background signing only after durable approval (Amber/NIP-55 model; approval-fatigue trade-off documented).

## 10. Transport attestation (F1)

Node identity is transport-independent. Any global-transport credential (a nostr npub or a future transport's key) is a **separate per-node key** bound to the node by **bidirectional cross-signed attestation** (Keybase-proof / Matrix cross-signing pattern; precedented inside nostr by NIP-46's client-key/user-key split).

**Format (normative; 202B):** `version` (1B = 0x01) ‖ `node_pubkey` (32B) ‖ `transport_id` (1B registry; 0x01 = nostr-secp256k1) ‖ `transport_key` (32B) ‖ `expiry_epoch` (8B mesh-epoch) ‖ `node_sig` (64B, over `transport_key ‖ transport_id ‖ expiry_epoch`) ‖ `transport_sig` (64B, over `node_pubkey`).

- **Distribution:** gossiped in-mesh (offline-verifiable — two signature checks, no fetch) and published to the transport when gatewayed. Verification checks both signatures and the expiry epoch; expired attestations are dead.
- **This machinery is the designed future rotation/revocation path** (the AT-proto rotation-key lesson, minus the registry): a rotation is a new attestation binding a successor key. Rotation itself stays deferred (§15) — the format is what ships.
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
| **Our identity module (`111id`: envelope + backup construction + glue)** | none — new code | enters **`testing`** when built | **commissioned third-party audit** (NLnet-style funding is a procurement task, §15) **+** the rubric's field-evidence gates ⇒ `proven`; reports `testing` until then |

G1 also confirms: wolfCrypt's FIPS 140-3 CMVP certificate counts as third-party validation (kept as a fallback primitive source, not adopted).

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
10. **No paste-your-key UX anywhere.** Words are shown once at commissioning and typed back to verify; keys are never pasted into forms ("paste your nsec" phishing is the dominant nostr compromise vector, and compromise there is permanent — here it must be unrepresentable).
11. **Encrypted backup blobs are never published publicly**, and restore verifies the node's signature **before** decrypting (Matrix 2022, GHSA-r48r-j8fx-mq2c — never trust-on-decrypt).
12. **No static-static ECDH as sole protection** (zero forward secrecy; buffered mesh ciphertext makes harvest-now-decrypt-later the default exposure). Long-term-key-only handshakes equally (Briar CVE-2023-33982).
13. **MAC auth is not authorship** — sealed channels authenticate the channel; authorship needs inner signatures (Layer 5's option).
14. **One signature scheme, one ECDH scheme, one AEAD at the wire** — multi-algorithm agility is attack surface (libp2p RSA-key CPU-DoS, CVE-2023-39533).
15. **No wall-clock validity in identity metadata** — monotonic counters and mesh-epochs only (clockless nodes; the X.509 datetime trap).
16. **Identity is a keypair, never an immutable log** (SSB anti-lesson: feed-as-identity makes compromise permanently unrecoverable; recovery is §8, the rotation path is §10).
17. **Physical-extraction residual risks are written down per class** — accepted explicitly, never discovered later:

| Class | At-rest (§5) | Residual risk (accepted, written) |
|---|---|---|
| ESP32-S3 | flash encryption + Secure Boot v2 + NVS encryption | Published voltage-fault-injection AES-key recovery; the S3 die has no glitch detector. eFuse steps irreversible. |
| nRF52840 | APPROTECT + ACL, Rev 3+ required | Pre-Rev3 bypass via CVE-2020-27211 (open $5-class ESP32 tool); even Rev 3+ is raise-the-bar, not SE-grade. |
| Cat 2 | OS keystore / 0600-0700 dir | Layer 1's accepted threat model applies (debuggable-build adb exposure until lockdown — `os/bootstrap/README.md` §Security & trust model). |
| Cat 4 | TPM2 / SE050B+ | Strongest at-rest option (CC EAL 6+ / FIPS 140-2 L3); ~261ms I2C signing keeps it out of packet hot paths. |

## 13. Cross-layer seams

- **Layer 1 (Bootstrap) consumes:** §4 keygen at install/commissioning; the identity-dir layout and CLI contract below — **replacing its provisional secp256k1 seam** (D9 D1). Layer 1 is specced-but-unbuilt, so the swap is a spec amendment, not a fleet re-key; any mnemonic already born under D9 survives (same words, new derivation, one NodeId change — pre-accepted).
  - `identity/` (0700, owner `node111`): `mnemonic` 0600 (BIP-39 words, NFKC UTF-8, space-separated, trailing newline) · `keyset` 0600 (64B private keyset — derived cache, regenerable from the mnemonic) · `nodeid` 0644 (34B record: `0xed 0x01` ‖ 32B pubkey, §7). Atomic temp-dir-then-rename write; never overwrite.
  - Boot integrity (carried): unreadable/corrupt identity ⇒ quarantine the dir, distinct error state, **never silent re-key**; regeneration only via explicit restore.
  - CLI: `sudo -u node111 111 identity backup` (the §8.3 ceremony: words + single-frame QR on a local TTY) · `111 identity restore` (§8.4). Neither is reachable over HTTP, ever. Layer 1's provisional `identity export` (NIP-49 ncryptsec) does **not** carry over — the phase-2 recipient-mode export (§8.5) replaces the concept; scrypt at ~64MiB has no place in this fleet.
- **Layer 3 (Local Intelligence) consumes:** the E1 hook (§9) and the recorded signer-daemon + default-deny policy-engine pattern; the agent holds zero key material and requests signatures from the identity holder (`111id` core in the `111d` world).
- **Layer 4 (Interface) displays:** the §7 operator label + editable local alias, and the identity-backup action surface. Label and alias are display-only — never auth.
- **Layer 5 (Messaging & Data) owns the full envelope.** Contracted identity hooks (§6.7): `signer_pubkey`, the versioned optional delegation trailer (v1 always empty), reserved ratchet space. Also handed to L5: authorship/non-repudiation (inner signatures if wanted), content message-ID idempotency (the delay-and-replay residual, §6.5), and group/broadcast key distribution.
- **Layers 6/7 (Transport / Relay & Presence) — open questions handed over, not decided here:** Cat-1 relay announce-verification policy (verify-before-relay vs relay-blind; Ed25519 verify is Cat-1-feasible) · announce-as-routing vs announce-as-identity-only (the biggest cross-layer question the wire shape forces) · announce rate-limit policy (Reticulum caps ~2% interface bandwidth — announce flooding is the registry-free tax) · attestation gossip and gateway publication (§10) · the **mesh-epoch source** that §9/§10 expiries count in (the time/epoch authority lives in Relay & Presence).
- **Layer 0 (Knowledge):** receives the §16 entries at build.

## 14. Test vectors & conformance

All vectors live in `os/identity/test-vectors/` at build. Conformance gates:

- **RFC 8032 (Ed25519) + RFC 7748 (X25519)** reference vectors MUST pass on every adopted implementation.
- **Rust (dalek) ↔ Monocypher-C agreement vectors** — keygen-from-seed, sign/verify, ECDH, sealed-packet round-trip — REQUIRED before any Cat-1 port (D13 H1; the port is pre-determined, not premature).
- **SLIP-0010 pinned-path vectors** for `m/111'/0'/0'` and `m/111'/0'/1'`: published SLIP-0010 vectors plus project-path vectors generated at build and frozen.
- **Announce encode/verify round-trip**, including the §6.2 verification rules.
- **Backup format round-trip**: mnemonic → descriptor → seed → keyset → restore → identical NodeId.
- **Negative tests**: downgrade rejection (unknown/weaker crypto-mode), stale-counter announce rejection, conflicting-key announce rejection (immutability), descriptor fail-closed on unknown fields.

## 15. Deferred (recorded, not forgotten)

Full ratcheting (envelope version-space reserved; Reticulum per-destination ratchet lineage — NOT a Signal port; Signal-class prekey servers have no registry-free offline role) · EDHOC for constrained online handshakes (5+ formal-verification studies, young deployment — watch) · hybrid PQ KEM field space (Cat 2+ only; ML-KEM ~1–2KB cannot fit LoRa frames) · the B2 PQ-migration corner (raw-32B addressing hardcodes key size — accepted with the user's B2 pick) · UCAN/Biscuit adoption (unaudited, moving; scope-vocabulary alignment now) · phase-2 backup builds: recipient-mode encrypted export, SSKR guardian split (§8.5) · BIP-85 child-secret subtree (reserved) · BIP-39 passphrase expert option (if ever: confirm-at-creation + NFKC mandatory) · bc-ur animated QRs (only if payloads exceed ~400B) · NIP-46 as agent↔signer IPC (only if nostr lands at Layer 7) · human key-verification UX + petnames (human-identity layer) · FROST threshold futures (drafts exist for both curves) · did:key interop shim (mintable from our pubkey anytime — free) · commissioned-audit procurement (NLnet-style; the §11 `proven` path) · ESP32-S3 VFI/glitch-detector silicon-revision check · nRF5340-vs-nRF52840 TF-M ITS tradeoff · Cat-1 relay announce-verification policy and announce-as-routing vs identity-only (handed to L6/7, §13) · per-peer state budget on Cat 1 (peer-table size caps replay windows and future ratchet state) · on-device Monocypher benchmark firmware (ESP32-S3 @240MHz Xtensa, nRF52840 @64MHz — published ms figures are cross-chip extrapolations) · the C/Monocypher Cat-1 core build itself (D13 H1: deferred-but-pinned until a layer owns ESP32 firmware; §14 vectors pre-determine it) · key rotation + compromised-key revocation as operational features (the whole deferred lifecycle phase — the machinery is §10, the activation is a future soft upgrade).

## 16. KB adoption list (entries written at build)

All enter `candidate` per D13 H1 (desk-research cap); Monocypher fast-tracks to `testing`. Existing seed entries (`meshtastic`, `reticulum`) are cross-linked/updated, not duplicated.

| Entry | Kind | Status at build |
|---|---|---|
| `monocypher` | project | fast-track `testing` (Cure53 + formal-methods CI, G1) |
| `ed25519-dalek` | project | `candidate` |
| `curve25519-dalek` | project | `candidate` |
| `libsodium` | project | `candidate` |
| `slip-0010` | protocol | `candidate` |
| `bc-sskr` | project | `candidate` |
| `noise-framework` | protocol (IK pattern) | `candidate` |
| `edhoc` | protocol | `candidate` (watch) |
| `reticulum-protocol` | design reference | license-flagged: code adoption banned (non-OSI "Reticulum License", AI-training clause); the protocol is public domain — clean-room design reference only |
| `meshtastic-pki` | case study | `candidate` (cautionary: its CVE cascade is our §12/§14 test plan) |
| `meshcore` | case study | `candidate` (verify license at drafting — research conflicted GPL-3.0/MIT) |
| `iroh-nodeid` | case study | `candidate` (raw-pubkey NodeId proven at 200M+ endpoints) |

Build tasks: `os/identity/TASKS.md` (created at build start per `os/plan/EXECUTION.md`).
