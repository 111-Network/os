# Vendored Monocypher — provenance and hash pins

Spec: `os/20-identity/README.md` §3 (Cat 1 primitive source), §11/D14 (vendored-primitive rubric exception: CC0/permissive + **vendored hash-pinned** + forkability recorded).

| Field | Value |
|---|---|
| Library | Monocypher |
| Version | **4.0.3** (the "Latest" release on the download page on the date below; CHANGELOG dates it 2026/06/15) |
| Source URL | `https://monocypher.org/download/monocypher-4.0.3.tar.gz` |
| Upstream checksum files | `https://monocypher.org/download/monocypher-4.0.3.tar.gz.sha512`, `…tar.gz.blake2b` |
| Downloaded | **2026-10-13** (15:58 UTC), `curl` from the host that ran T9; nothing else was fetched into `vendor/` |
| Tarball SHA-256 (ours) | `8cc9bc341a66249016db9bd70e9142d8d0aef9945973744b1ac05dbc55d8ee66` |
| Tarball SHA-512 | `40904ada5c7ee4f7741733e38b69a30a4b0561cbffba5ffe7c2dce16136d540251ec0d9056ff606510d3b5b708fb8a40db7e0870d4a0b2dc17ba2bfb880f8965` — **matches** upstream's published `.sha512` |
| Tarball BLAKE2b-512 | `5c586019e8e78ce6fb70f73f4ded10c852335138394b22fb704d0b4379edfc1adaea342f0e4c4b8c52fcf053df401d1fe47812ee5cb5062813de19fc51f8ae93` — **matches** upstream's published `.blake2b` (`b2sum`) |
| Licence | dual: BSD-2-Clause **or** CC0-1.0 (`vendor/LICENCE.md`) |

The tarball has no entries outside its `monocypher-4.0.3/` prefix. The vendored files are byte-identical to the corresponding extracted tarball paths (`cmp`).

## What is vendored (and why)

| Vendored path | Tarball path | SHA-256 |
|---|---|---|
| `vendor/monocypher.c` | `monocypher-4.0.3/src/monocypher.c` | `57eb914fc88136119bd41655cccb8c250048bf54d470540625186f8ab16f64be` |
| `vendor/monocypher.h` | `monocypher-4.0.3/src/monocypher.h` | `c494da712122da7ff679fdcf318a5317e84972b6c950fe9d896212947797facd` |
| `vendor/optional/monocypher-ed25519.c` | `monocypher-4.0.3/src/optional/monocypher-ed25519.c` | `60fce3578fb00b00da96490653d993c4cb427b1e1be38183285c66e04d22cc18` |
| `vendor/optional/monocypher-ed25519.h` | `monocypher-4.0.3/src/optional/monocypher-ed25519.h` | `abc4fad381879f5c29176ebe014b9189956b3dfe0a3e36459b6990bc57212380` |
| `vendor/LICENCE.md` | `monocypher-4.0.3/LICENCE.md` | `5f8360e4c06ddcc584bdb4b210c6af824c4bb301e6a9a521869b6d90795ca4b3` |

* **Core `monocypher.c/.h`**: X25519, BLAKE2b, XChaCha20-Poly1305 (`crypto_aead_*`), and the EdDSA building blocks (incl. `crypto_eddsa_check_equation`, used by the harness's decode probe). Only `<stddef.h>` / `<stdint.h>` are included and no libc function is called by name; the one undefined symbol at `-O2` (`nm -u`) is `memcpy`, which the compiler emits for aggregate copies (the usual freestanding requirement), plus the distro's `__stack_chk_fail` stack-protector hook. That is what makes the Cat-1 port plausible.
* **`optional/monocypher-ed25519.c/.h`**: **required.** Monocypher's core EdDSA is *EdDSA-BLAKE2b* (`crypto_eddsa_*`), which is **not** RFC 8032 Ed25519 and does not verify dalek signatures. The Ed25519/SHA-512 variant (`crypto_ed25519_*`), plus the SHA-512 and HMAC-SHA-512 the harness uses for BIP-39/SLIP-0010, live in this optional module.
* **`LICENCE.md`**: the only non-`.c/.h` file; included because the BSD option requires the notice to travel with redistributed source. Everything else in the tarball (docs, tests, makefile, `change-prefix.sh`, `AUTHORS.md`, CHANGELOG) is deliberately **not** vendored.

`SHA256SUMS` (in this directory) pins the five files above; `run.sh` refuses to build if any byte differs (`sha256sum --check --strict`).

## Second-source cross-check (verification only, nothing vendored from it)

The GitHub tag `4.0.3` (`https://github.com/LoupVaillant/Monocypher/archive/refs/tags/4.0.3.tar.gz`, SHA-256 `a7cbae546fbdc489bca632c3747e1ceb8ca3d4bd39e2706a0916f28ccd280e50`) was fetched into a scratch directory and diffed against the monocypher.org extraction: `src/monocypher.{c,h}` and `src/optional/monocypher-ed25519.{c,h}` differ **only on line 1** (`// Monocypher version __git__` in the git tag vs `// Monocypher version 4.0.3` stamped into the release tarball). `LICENCE.md` differs only by a trailing "Special notes" section about `tests/externals/` (not vendored). The monocypher.org checksum files are served from the same origin as the tarball, so they detect corruption, not a compromised origin; the GitHub diff is the independent leg.

## Release facts worth knowing (from the vendored upstream CHANGELOG)

* **4.0.3 (2026/06/15): "Fixed timing leak vulnerability in EdDSA/Ed25519 signatures."** Versions before 4.0.3 are affected; any Cat-1 port must start from ≥ 4.0.3 (this pin).
* **4.0.0: `crypto_x25519()` now returns `void`.** Monocypher 4.x does **not** reject all-zero X25519 output natively. Spec §3 and the KB `monocypher` entry say it does (true for 3.x) — see "Findings" in `test-vectors/monocypher-smoke.md`; the harness's `ecdh()` wrapper supplies the ingress blacklist and the all-zero abort.
* `crypto_eddsa_check_equation` is *cofactored* and *accepts non-canonical A/R* (source comment: "*Allow* non-cannonical encoding for A and R"; final step multiplies by 8). The §3 strict predicate is therefore a wrapper concern (`strict_verify()` in `harness.c`).
* The Cure53 audit (2020-06) covered v3.1.1, not 4.x (`os/20-identity/README.md` §3, §11).

## Re-verifying / updating

```bash
# re-verify the pin (no network)
cd os/20-identity/monocypher-harness && sha256sum --check --strict SHA256SUMS

# re-verify provenance (network)
curl -fLO https://monocypher.org/download/monocypher-4.0.3.tar.gz
sha256sum monocypher-4.0.3.tar.gz     # == 8cc9bc34…8ee66
sha512sum monocypher-4.0.3.tar.gz     # == 40904ada…f8965 (upstream .sha512)
```

Changing the pin (new Monocypher release) is a spec-level event: re-vendor in a fresh directory, update this file and `SHA256SUMS`, re-run `run.sh`, and re-write `test-vectors/monocypher-smoke.md`. Never edit a vendored file in place — carry any patch as a separate, documented diff.
