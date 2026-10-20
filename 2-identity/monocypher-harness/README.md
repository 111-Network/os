# Monocypher C cross-vector harness (task T9)

Host-side proof that **vendored Monocypher 4.0.3 (C)** agrees byte-for-byte with the **Rust `id111`** crate on the frozen Layer 2 test vectors — spec §14 *"Rust (dalek) ↔ Monocypher-C agreement vectors REQUIRED before any Cat-1 port"*. Its run is also the first-party smoke-test log for the `monocypher` KB entry (§11 / D14): `../test-vectors/monocypher-smoke.md`.

```bash
bash os/2-identity/monocypher-harness/run.sh             # gate: exit 0 iff every file PASS
bash os/2-identity/monocypher-harness/run.sh --sanitize  # same under ASan + UBSan
bash os/2-identity/monocypher-harness/run.sh --log out.txt
bash os/2-identity/monocypher-harness/mutation-check.sh  # proves the harness can fail (not part of the gate)
```

Needs: a C99 compiler (`CC` overrides `cc`), bash, GNU `sha256sum`. No network, no Python at run time. `python3 -I` (stdlib only) is used only by `mutation-check.sh` and, offline, by `supplement/gen_supplement.py`.

## Layout

| Path | What |
|---|---|
| `harness.c` | the harness (C99, libc + vendored Monocypher only) |
| `run.sh` | hash-pin check → build (`cc -std=c99 -O2 -Wall -Wextra -Werror`) → inventory check → run |
| `vendor/` | Monocypher 4.0.3 `.c/.h` (core + `optional/monocypher-ed25519`) + `LICENCE.md` — see `VENDOR.md` |
| `SHA256SUMS` | pins the five vendored files; `run.sh` refuses to build on any mismatch |
| `VENDOR.md` | version, URL, date, tarball hashes, second-source cross-check |
| `supplement/` | harness-local, **non-frozen** cofactor-class edge inputs + their generator (see below) |
| `mutation-check.sh` | applies one-line mutants to a scratch copy of `harness.c`; each must make the harness exit non-zero |

## What is executed

| Vector file | Executed against Monocypher |
|---|---|
| `ed25519-rfc8032.json` | `crypto_ed25519_key_pair` (keygen-from-seed), `crypto_ed25519_sign` (deterministic ⇒ byte-equal), raw check + strict check, tamper controls |
| `x25519-rfc7748.json` | `crypto_x25519_public_key`, `crypto_x25519` (RFC 7748 §5.2, §6.1, 1 and 1000 iterations), raw and via the wrapped `ecdh()` |
| `slip0010-published.json` | SLIP-0010 (HMAC-SHA512) key **and** chain halves, built on the vendored `crypto_sha512_hmac` |
| `slip0010-project-paths.json` | BIP-39 seed (PBKDF2-HMAC-SHA512 ×2048 written in the harness) → `m/111'/0'/0'`, `m/111'/0'/1'`; pre-hardened-index negatives |
| `keyset-project.json` | full chain mnemonic → NodeId + X25519 pub + 64B public/private keysets; sign/verify and ECDH on the derived keys |
| `edge-vectors.json` | the §3 strict predicate (all 40 Ed25519 cases incl. the 4 mixed-order rows of spec v2.1, **exact `VerifyError` variants**), the 8-point torsion subgroup (each also checked `[ℓ]T = O` iff `T = O`), the 11 X25519 low-order inputs + 4 controls |
| `announce.json` | tag-prefixed sign/verify, byte-exact re-sign of KAT + positives, the full `process_announce` ordering (length → version → low-order enc → strict sig → self-NodeId → pin store) over all malformed cases and scenario steps |
| `sealed-packet.json` | §6.4 recipe in C: X25519, `BLAKE2b-256/192` KDFs (`crypto_blake2b` with `hash_size` 32 / 24), XChaCha20-Poly1305 (`crypto_aead_lock/unlock`, 24-byte nonce): **seal reproduces the frozen packet**, **open decrypts it**, all negatives by `seal::open` ordering, seal-side policy negatives, plus a C-only round-trip sweep over block-boundary sizes |
| `attestation.json` | §10 `node_sig` under `111/attest-node/v1` (signed-input field order ≠ wire order), KAT re-signed byte-exactly; every wire in `negatives/ordering/supersedure` gets the node-leg verdict; stages that are *not* C's (transport leg = Layer 7, expiry, supersedure) are checked as "node leg accepts" |

Not applicable (no Monocypher primitive involved; listed by the harness with a reason, and `run.sh` fails if a new `test-vectors/*.json` appears that the harness does not account for): `encoding.json`, `weak-key.json`, `backup-format.json`, `restore-counters.json`, `d9-migration.json`. Deliberately skipped inside a consumed file: `attestation.json` `expiry[]` (5 entries — a mesh-epoch comparison, no crypto), shown in the SKIP column.

## Design decisions

1. **Ed25519 = `crypto_ed25519_*` (the `optional/` module).** Monocypher's core `crypto_eddsa_*` is EdDSA-BLAKE2b and does not interoperate with dalek.
2. **`crypto_ed25519_check` is wrapped (`strict_verify`).** Upstream's `crypto_eddsa_check_equation` is cofactored and *accepts* non-canonical A/R. The wrapper applies the §3 legs in the same order as `id111::keyset::verify_strict` — canonical A → canonical R → `S < ℓ` → A decodes → R decodes → small-order R/A → torsion-free A, then R (`[ℓ]P = O`, spec v2.1) → Monocypher equation — so it returns the same `VerifyError` variant, and the harness asserts variant equality, not just accept/reject. "Decodes" is tested through Monocypher's own decompression (public `crypto_eddsa_check_equation` with a probe signature `R = identity, S = 0` and `h = 0`, which is accepted iff A decodes). Small-order detection is a 5-entry `y` table (the 8 torsion points, either `x` sign), validated against `edge-vectors.json`'s torsion subgroup.
3. **`crypto_x25519` is wrapped (`ecdh`).** Monocypher 4.x returns `void` and does not abort on all-zero output. The wrapper applies the 7-entry low-order blacklist (top bit masked; `0, 1, two order-8, p-1, p, p+1`) and the all-zero abort.
4. **SLIP-0010/PBKDF2 are done in C** with the vendored HMAC-SHA512 (≈40 lines), so the keyset chain mnemonic → NodeId is exercised end to end. The mnemonic is an opaque ASCII string (NFKD = identity for the public English test phrases); BIP-39 wordlist/checksum validation is Rust-side policy.
5. **JSON is read in C** by a small strict reader (rejects duplicate keys and trailing garbage, like the Rust conformance reader) rather than a Python pre-processor: one fewer moving part between the frozen bytes and the assertions.
6. **No silent skips:** per-file executed-entry count must equal the array lengths in the JSON; unknown `expect` tokens fail; each token comparison is exact.
7. **`[ℓ]P` is computed by the harness — host reference only (spec v2.1).** Monocypher 4.0.3's public API cannot multiply an arbitrary point by ℓ: `crypto_eddsa_check_equation` multiplies by the cofactor 8 before comparing (a torsion component vanishes), `crypto_eddsa_scalarbase` multiplies only the base point, `crypto_eddsa_mul_add` is scalar arithmetic, and the X25519 functions clamp the scalar to a multiple of 8. The vendored sources stay untouched, so `point_torsion_free()` uses a small self-contained Edwards implementation (TweetNaCl-shaped 16×16-bit limbs, complete addition law, plain double-and-add; variable time, about 6× the cost of a Monocypher verify on the host). It is validated at start-up against Monocypher (`[s]B` must equal `crypto_eddsa_scalarbase(s)` for six scalars, base-point round-trip, `[ℓ]B = O`, `B + T8` mixed-order) and it is **not** the Cat-1 core's implementation. The production core owns its own `[ℓ]P` on Monocypher-class field arithmetic, which costs about 0.7–0.9 of a verify per point on the host (spec §3 note).

## The supplement (`supplement/cofactor-edges.json`)

Not frozen, not in `test-vectors/` (the Rust inventory test fails on any new `.json` there). The frozen edge vectors are built by swapping one field of RFC 8032 TEST 2, so most are false under *any* verifier. The supplement builds **validly-signed** inputs, from an independent pure-Python cofactorless §3 oracle (`gen_supplement.py`, deterministic; `python3 -I gen_supplement.py | cmp - cofactor-edges.json`):

* `wrapper-bites` (gated): raw Monocypher **accepts**, the wrapper must reject with the oracle's variant — small-order R, non-canonical R alias, order-8 R. Proves those legs are load-bearing.
* `wrapper-redundant` (gated): `S + ℓ` — the core already rejects it.
* `residual-mixed-order` (**gated since spec v2.1**, still printed as `KNOWN-DELTA … [closed]`): `R = [r]B + T` and `A' = A + T` for torsion `T` (not small-order). A cofactored verifier accepts; `id111` rejects. Until 2026-10-14 the wrapper could not close this and the rows were reported, never failed (T9 finding F3). Spec v2.1 requires A and R torsion-free, so the wrapper now rejects all three with `InvalidSignature` and a regression fails the run. The same three rows, plus a ground `h ≡ 0 (mod 8)` mixed-order-A row that dalek's `verify_strict` alone accepted, are frozen in `../test-vectors/edge-vectors.json`. See `../test-vectors/monocypher-smoke.md` F3.
