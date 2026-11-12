# `l7-D7`: hub-origin fixtures (§9.1; row D7)

Two signed stable releases for `tests/origin_serve.rs` and the `origin` unit tests
(`src/daemon/origin/mod.rs`), Layer 7 row D7 (`os/7-relay/README.md` §9.1). Nothing here is a
real key or a real payload.

| Path | What |
|---|---|
| `2026.10.12/channel-111-stable.toml{,.minisig}` | release 2026.10.12 |
| `2026.11.02/channel-111-stable.toml{,.minisig}` | release 2026.11.02 (the newer one: admission order, pruning) |
| `payload/<release>-<arch>.bin` | the bytes each release's `[payload.<arch>-unknown-linux-musl]` pin names (sha256 and size) |

Each manifest is the canonical fixture (`../manifest/channel-111-stable.toml`) with its
`date` and `release` set, its two payload pins replaced by the files under `payload/`, and a
keychain of generation 0 only (the compiled DEV key, `../minisign/test.pub` =
`1-bootstrap/keys/manifest-pub.key`), signed by `../minisign/test.key` with the trusted
comment `l7-D7 fixture`. A target's payload has the same size in both releases, so the other
release's payload passes the size check and is refused by its hash.

## How they were made

Run `make.sh` from this directory (minisign 0.11; `MINISIGN=/path/to/minisign` to pick one).
The payload bytes are fixed text, and Ed25519 signing is deterministic, so a re-run is
byte-identical.
