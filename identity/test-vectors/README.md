# Layer 2 — frozen test vectors (spec §14)

Frozen **2026-10-13** (task T8); amended **2026-10-14** under spec v2.1 (owner decision, §3 torsion-free A/R): four mixed-order rows appended to `edge-vectors.json`, no existing row changed. Spec: `../README.md` (v2.1 LOCKED, §14 "Test vectors & conformance"). Every file below freezes one §14 gate; `../crates/id111/tests/conformance.rs` loads **every** file and re-asserts it against the public `id111` API, so the vectors are live gates, not dead data. `crosscheck.py` re-derives them with no `id111` code at all.

**Frozen means frozen.** A vector changes only with an identity-layer sign-off and a spec amendment, never as a side effect of a code change: if `cargo test` fails here, the implementation drifted (fix the code) — do not regenerate the file.

## Files ↔ §14 gates

| File | §14 gate | Harvested from (`cargo test -- --nocapture`) |
|---|---|---|
| `ed25519-rfc8032.json` | RFC 8032 reference vectors; Monocypher-agreement **keygen-from-seed, sign/verify** | `keyset.rs::rfc8032_vectors` (TESTs 1–3, lifted) + RFC 8032 TEST SHA(abc) |
| `x25519-rfc7748.json` | RFC 7748 reference vectors; Monocypher-agreement **ECDH** | `keyset.rs::rfc7748_vector_and_agreement` (§6.1, lifted) + §5.2 vectors + the 1- and 1000-iteration test |
| `slip0010-published.json` | SLIP-0010 published vectors (ed25519 vectors 1 and 2, key **and** chain halves) | `slip0010.rs::slip0010_published_vectors` (lifted) |
| `slip0010-project-paths.json` | SLIP-0010 pinned-path vectors `m/111'/0'/0'` and `m/111'/0'/1'` from the canonical mnemonic (+ the BIP-39 seed they sit on) | `slip0010.rs::slip0010_project_path_vectors` (12-word, printed); 24-word generated for this freeze |
| `keyset-project.json` | project keyset: mnemonic → NodeId + X25519 pub (+ 64B public/private keysets) | `keyset.rs::project_keyset_vector` (12-word, printed); 24-word generated for this freeze |
| `encoding.json` | §7 encodings: hex / bech32 (BIP-173, **not** bech32m) / QR upper-case / 10-char label / 34B records; fail-closed decode negatives incl. bech32m rejection and data-part length enforcement | `encoding.rs::kat_fixed_pubkey` (sequential key); project NodeId from `backup.rs::backup_format_kat` (printed); negatives generated |
| `edge-vectors.json` | **edge/malleability**: strict Ed25519 predicate (non-canonical S/R/A, all 8 small-order points as A and R, malformed A, mixed-order A and R — v2.1) + X25519 low-order blacklist | constants lifted from `keyset.rs` `strict_predicate_*` / `x25519_low_order_rejected_on_ingress`; extended (torsion subgroup, aliases, top-bit variants); 2026-10-14: the three T9 smoke-log mixed-order rows (`../monocypher-harness/supplement/`) byte-for-byte + one ground `h ≡ 0 (mod 8)` mixed-order-A row |
| `weak-key.json` | **weak-key self-test** (§4.5): all-zero / uniform / counter-pattern halves | `keyset.rs::weak_key_self_test_vectors` (lifted) + extended |
| `announce.json` | **announce** encode/verify round-trip + negatives: downgrade (unknown version), stale counter, conflicting key, **self-NodeId**, **domain-tag confusion**, length, low-order enc, strict-predicate | `announce.rs::announce_round_trip_and_kat` (printed); negatives/scenarios generated and recorded from `process_announce()` |
| `sealed-packet.json` | **sealed-packet** KAT (esk/rpk/header/plaintext → packet) + negatives: wrong recipient, tampered AD / mode / counter, low-order epk, truncation; Monocypher-agreement sealed-packet round-trip | `seal.rs::known_answer_vector` (printed) |
| `backup-format.json` | **backup format**: 4B descriptor, QR payload, §13 file bodies, round-trip, descriptor-absent ⇒ v1 defaults, checksum-failure / unknown-field refusals, NFKC hygiene, verify-on-write-down | `backup.rs::backup_format_kat` (12-word, printed); 24-word + negatives generated |
| `attestation.json` | **attestation** encode/verify round-trip + negatives: expired ordering, field-swap, cross-node transplant, domain-tag confusion, superseded-but-replayed | `attest.rs::attestation_kat_for_t8` (printed); negatives/supersedure generated |
| `restore-counters.json` | **restore wire-function**: restore→re-announce (floor + 1024, next = +1), restore→send (per-peer resumption at 1025); §6.2 **sync-commit** rule; §6.5 unclean-reboot jump J ≥ N | derived from `counters.rs` / `backup.rs` semantics, asserted against the live `CounterStore` |
| `d9-migration.json` | **D9-migration round-trip**: provisional Layer-1 dir → L2 keyset → quarantined legacy files; fail-closed detection | dir body from `migrate.rs` tests (Layer 1's NIP-06 test values); new NodeId generated |

`crosscheck.py` is tooling, not a gate file.

## Layer 5 amendment vectors — `layer5/` (frozen 2026-10-14, task L5-I1)

The Layer 5 spec (`../../messaging/README.md` v1.2, D20) asks Layer 2 for amendments A2, A3, A6 and A11 (§19 there). They land as `id111` code and these vectors, in a **subdirectory on purpose**: the top-level set above is the Cat-1 Monocypher harness contract and stays byte-identical (D20 — "the Cat-1 harness vectors are untouched"); `monocypher-harness/run.sh` and `conformance.rs` inventory the top level only. `../crates/id111/tests/layer5_amendments.rs` loads **every** `layer5/*.json` (its own inventory, frozen date `2026-10-14`) and re-asserts it against the public API; `crosscheck.py` (`check_layer5_*`) re-derives it with no `id111` code.

| File | Amendment | What it freezes |
|---|---|---|
| `layer5/sealed-payload.json` | **A2** — `seal::seal_payload` / `seal::open_payload` | the §6.4 recipe with a **caller-owned inner** (no 32 B sender NodeId): three KATs with pinned `esk` (L5-shaped `extra` = `prio_flags ‖ dest_hint`; empty inner = the 59 B `MIN_PACKET_LEN` packet; 255 B `extra` + counter `u64::MAX`), 22 `open_payload` negatives (tampered AD/epk/ct/tag, wrong mode incl. mode-before-ECDH, truncation, extra overrun, low-order epk), wrong recipient, seal-side refusals (which never invoke the inner closure), and the **layout ambiguity** that forces "one inner layout per `version`" |
| `layer5/announce-a6.json` | **A6** — `PinOutcome::Unchanged` | same key material at a counter ≤ the pin ⇒ `ok:unchanged:<last_seen>`, no state change (reconnect, genesis card after a newer announce, different appdata); different key material ⇒ `err:conflict` at a lower, equal **and** higher counter; signature verified before the stale rule; per-step pin state (`after`) |
| `layer5/observe-counter.json` | **A11** — `CounterStore::observe_counter` | the §6.5 window as a `fresh`/`stale` verdict with the window state after each step (gap fill, window edge, a custody item hundreds behind, counter 0 and `u64::MAX`); `on_receive` parity is asserted alongside |

A3 (`next_announce_counter` as the L5 `msg_seq` epoch source) adds no wire; it is documented and unit-tested in `counters.rs`.

**Relation to the frozen L2 announce vectors.** `announce.json` still names a stale announce `err:stale:<last_seen>:<got>` (the Cat-1 harness replays exactly that). Since A6 `id111` reports the same no-state-change outcome as `Ok(Unchanged)`; `conformance.rs` maps the frozen token to `ok:unchanged:<last_seen>` **only when the announce's key material is identical to the pinned material** (a key conflict is `err:conflict` under both rules) and asserts the pin did not move. The 32 B-NodeId `seal`/`open` and `sealed-packet.json` stay as the pre-release Cat-1 gate and are removed before the first signed release (L5 §3 "one wire per version").

## Format

* Plain JSON, UTF-8. Every file starts with `spec` (the §-reference(s) it freezes), `frozen` (`2026-10-13`), `description`, `provenance`.
* All byte strings are **lowercase hex**. `u64` counters / epochs are **decimal strings** (JSON numbers lose precision above 2⁵³); small integers (indices, lengths, versions) are plain numbers.
* `expect` fields are **outcome tokens** (`ok:pinned`, `err:stale:5:2`, `err:sig:NonCanonicalS`, `Mnemonic:InvalidChecksum`, …) whose grammar is spelled out in each file's `description`. Acceptance/rejection is the cross-implementation contract; the specific variant name is Rust-side diagnostics.
* The only mnemonics used are **public well-known test phrases** (`abandon×11 about`, `abandon×23 art`, a BIP-39 `legal winner…` phrase, Layer 1's NIP-06 phrase). The private keysets in these files are **test material — never use them for a real node.**
* No `serde_json`: the JSON dependency is not in the frozen dep set (spec §3), so `conformance.rs` carries a ~100-line strict reader (rejects duplicate keys and trailing garbage).

## Running

```bash
cd os/identity
export PATH="$HOME/.cargo/bin:$PATH"
cargo test                                  # unit tests + tests/conformance.rs
cargo clippy --tests -- -D warnings
python3 -I test-vectors/crosscheck.py       # independent re-derivation (python3 + `cryptography`)
```

`crosscheck.py` re-implements, from the spec text, what the crate decides: the strict Ed25519 predicate (pure-Python curve arithmetic), the X25519 low-order class (RFC 7748 ladder × cofactor 8), the §6.2 announce / §6.3 pinning rules, the §6.4 recipe (HChaCha20 included), §10 attestation + supersedure, bech32 / label, BIP-39 seed + SLIP-0010. It never imports or calls `id111`.

## Monocypher-C harness (T9) — what to consume

§14 requires Rust (dalek) ↔ Monocypher-C agreement on **keygen-from-seed, sign/verify, ECDH, sealed-packet round-trip** before any Cat-1 port:

* keygen-from-seed + sign/verify → `ed25519-rfc8032.json` (`vectors[]`), `keyset-project.json`
* ECDH → `x25519-rfc7748.json` (`diffie_hellman`, `scalarmult[]`, `iterated`)
* sealed-packet → `sealed-packet.json` (`kat`: pinned `esk`, so the packet is deterministic; plus `negatives[]`)
* strict-predicate / low-order edges → `edge-vectors.json`; announce wire → `announce.json`

**Watch the cofactor:** Monocypher's `crypto_eddsa_check` is a *cofactored* (ZIP-215-style) verifier and will accept some small-order cases the §3 strict predicate rejects. The port must wrap it with the strict pre-checks (canonical A/R, S < ℓ, small-order A/R, and since v2.1 torsion-free A and R via its own `[ℓ]P = O`), and the agreement run must record the delta in the §11/D14 smoke log. `edge-vectors.json` is the oracle for exactly that. Its last four `ed25519_strict` rows are mixed-order: raw Monocypher accepts all four, and the last one (a ghost NodeId `A + T` with `h ≡ 0 mod 8`) was also accepted by dalek `verify_strict` alone, which is why `id111` checks `is_torsion_free()` before it.

## Honest limits (what these vectors cannot show)

* **Sealed-packet KAT is pinned through a model, not the production seam.** `seal_with_ephemeral` is private, so a fixed-`esk` packet cannot be produced through the public API. `conformance.rs` therefore (a) recomputes the packet with an independent in-test model of the §6.4 recipe built from the primitive crates, and (b) opens it through the public `seal::open`. Fresh-ephemeral `seal()` is covered by round-trips.
* **All-zero-DH abort (`NonContributory`) is unreachable by design:** the §3 low-order ingress check fires first, so every input that would produce an all-zero secret is rejected as `LowOrderPublicKey`. The vectors pin that observable behavior (no secret is ever returned); the abort arm itself is defense-in-depth.
* **Weak-key leg 4 (derived X25519 public key is low-order) has no vector:** an honestly clamped scalar can never yield one. The low-order class is covered directly in `edge-vectors.json`.
* **Attestation expiry semantics are untestable until the mesh-epoch source lands (§15).** The vectors pin encoding, `node_sig` verification, the `current ≥ expiry_epoch` ordering against a caller-supplied epoch, and supersedure. `transport_sig` is opaque bytes (secp256k1/BIP340 verification is Layer 7's, behind `TransportVerifier`); the suite proves the signed input handed to that seam is byte-exact.
* **dalek decompresses `x = 0` with the sign bit set** (an invalid encoding per RFC 8032 §5.1.3); the strict predicate still rejects the resulting identity point as small-order, so no vector accepts it — other implementations must reject it one way or the other.
* **Gap surfaced by the freeze — FIXED at review (2026-10-13):** a peer first contacted fewer than N operations before an *unclean* reboot was absent from the persisted `counters` file, so its send counter restarted at `traffic_base` (counter reuse → its first N packets dropped as replays). Fix in `counters.rs`: the unclean jump now also raises `traffic_base` by J, and the first emission after any clean state marks the file running (so the crash is detectable). Regression test: `counters::tests::unclean_reboot_covers_peers_absent_from_persisted_file`.
