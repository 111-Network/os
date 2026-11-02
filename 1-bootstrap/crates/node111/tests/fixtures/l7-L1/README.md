# `l7-L1`: manifest trust fixtures (§9.5 T1, T2; G12)

Throwaway keys and signed channel manifests for `tests/manifest_trust.rs` and the
`manifest` unit tests (`src/manifest.rs`), Layer 7 row L1 (`os/7-relay/README.md` §9.5).
Nothing here is a real key. Generation 0 is the compiled DEV key
(`1-bootstrap/keys/manifest-pub.key` = `../minisign/test.pub`); `gen1.key` / `gen2.key`
are fresh unencrypted keypairs made for this row (`minisign -G -W`).

Every manifest is the canonical fixture (`../manifest/channel-111-stable.toml`) up to its
`[[keychain]]` section, with the keychain below. A `signature_by_previous` is the full
`.minisig` text over the exact bytes of the next generation's `pubkey` string (the base64
`RW…` line, no newline), signed by the previous generation's key.

| Directory | Channel | Keychain | Manifest signed by | `parse_verified` with the compiled key |
|---|---|---|---|---|
| `beta/` | `beta` | 0 = test | test (gen 0) | refused (T1) |
| `gen1/` | `stable` | 0 = test, 1 = gen1 (link by test) | gen1 | passes at generation 1 |
| `gen1-root-signed/` | `stable` | as `gen1/` | test (gen 0) | refused: not the highest generation reached |
| `broken-link/` | `stable` | 0 = test, 1 = gen1 (link by `other`) | gen1 | refused: the chain ends at 0 |
| `gen2-broken/` | `stable` | 0 = test, 1 = gen1 (link by test), 2 = gen2 (link by test, not gen1) | gen1 | passes at generation 1 |
| `gen2-broken-gen2-signed/` | `stable` | as `gen2-broken/` | gen2 | refused: generation 2 is never reached |
| `unlisted-root/` | `stable` | 0 = `RWRoot0…` placeholder (the updater tests' root) | test | passes at generation 0 |

## How they were made

Run `make.sh` from this directory (minisign 0.11; `MINISIGN=/path/to/minisign` to pick
one). It creates `gen1.{key,pub}` and `gen2.{key,pub}` only when absent, then rewrites
every manifest and signature. Every signature carries the fixed trusted comment
`l7-L1 fixture` and Ed25519 signing is deterministic, so a re-run with the committed keys
is byte-identical. The commands it runs, in short:

```sh
minisign -G -W -p gen1.pub -s gen1.key          # once; likewise gen2
printf '%s' "$(tail -n1 gen1.pub)" > link.txt   # the next pubkey, no newline
minisign -S -W -s ../minisign/test.key -t "l7-L1 fixture" -m link.txt -x link.minisig
#   → the .minisig text becomes generation 1's signature_by_previous ("""…""")
minisign -S -W -s gen1.key -t "l7-L1 fixture" \
  -m gen1/channel-111-stable.toml -x gen1/channel-111-stable.toml.minisig
```

The existing fixtures (`../manifest/`, `../minisign/`) are untouched; they still verify
(`tests/manifest_trust.rs`, `canonical_fixture_still_verifies_at_generation_zero`).
