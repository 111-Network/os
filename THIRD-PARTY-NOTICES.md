# Third-party notices

The 111network OS includes the third-party components listed below. Each keeps its own license; the licenses are reproduced here
so that their notice and attribution requirements are met. 111network's own license is in `LICENSE.md` (draft). The reasoning behind
this list is in `LICENSING.md`.

**Release requirements (follow-ups for the lead, not done by this file):**

1. This file must ship inside the release payload, as a Layer 1 manifest item, so every installed node carries it.
2. It must be reachable from the dashboard: Settings, Essentials, a "Licenses" link.

## Scope and how this file is generated

- **Rust:** all normal dependencies of `111` (crate `node111`, which includes `id111`) and `111-agent` (crate `agent111`) for the Linux musl
  targets `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, excluding our own crates. It includes compile-time proc-macro crates.
  Source: `cargo metadata` and `cargo tree -e normal --target <triple>` in `os/bootstrap` and `os/intelligence`.
- **JavaScript:** the production dependencies of the dashboard, from `pnpm licenses list --prod --json` in `os/interface/ui`.
- **Fonts, vendored code, embedded data:** see their sections.
- **Copyright lines** are taken from each component's license files (Rust: the unpacked crate in the cargo registry; JS: the installed package). Where
  a component's license files have no copyright line, its `authors` metadata is shown instead.
- **Choice of license.** Where a component offers a choice (`OR`), the text of any one offered license suffices. This file reproduces the MIT or Apache-2.0
  option wherever one is offered.
- Not covered here: separate programs that run next to our code and are not linked (Reticulum, Meshtastic, Termux and its packages, llama.cpp,
  opencode, RNode firmware). See `LICENSING.md` section 3. Their texts and source offers are added when a release carries them.

Inputs at generation time (SHA-256):

```
c2724281b2643dbba247ee54bbc0739a6bd7111675bea35b3e2bdaa5b5c830da  bootstrap/Cargo.lock
9f779bdf23242c905a0106412a0cc6aa7b1b90ad985547d18674f6639e0b1d47  intelligence/Cargo.lock
93b9b10f1fd806baeff102300e06e1a7532fae650043eb370283dbaf18793954  interface/ui/pnpm-lock.yaml
```

Regenerate with `python3 -I os/bootstrap/scripts/gen-notices.py` whenever a lockfile changes. `--check` (run by `os/bootstrap/scripts/check.sh`) fails when the hashes above no longer match.
The output is sorted and contains no timestamps, so identical inputs give identical output.

## Summary

| Group | Components |
|---|---|
| Rust crates | 187 |
| JavaScript packages | 6 |
| Fonts | 4 files, 3 families |
| Vendored source | 1 (Monocypher) |
| Embedded data | 1 (Mozilla CA roots, via `webpki-roots`, also counted under Rust crates) |

## Rust crates

187 crates, grouped by the license expression in their `Cargo.toml`. The last field says which binary links the crate:
`111`, `111-agent` or `both`.


### (MIT OR Apache-2.0) AND Unicode-3.0 (1)

- `unicode-ident` 1.0.27 — https://github.com/dtolnay/unicode-ident — Copyright © 1991-2023 Unicode, Inc — both

### 0BSD OR Apache-2.0 OR MIT (1)

- `adler2` 2.0.1 — https://github.com/oyvindln/adler2 — Copyright (C) Jonas Schievink — 111

### Apache-2.0 (1)

- `sync_wrapper` 1.0.2 — https://github.com/Actyx/sync_wrapper — authors: Actyx AG — both

### Apache-2.0 AND ISC (1)

- `ring` 0.17.14 — https://github.com/briansmith/ring — Copyright 2015-2025 Brian Smith — 111

### Apache-2.0 OR Apache-2.0 WITH LLVM-exception OR MIT (4)

- `linux-raw-sys` 0.12.1 — https://github.com/sunfishcode/linux-raw-sys — Copyrights in the `linux-raw-sys` project are retained by their contributors — 111
- `linux-raw-sys` 0.4.15 — https://github.com/sunfishcode/linux-raw-sys — Copyrights in the `linux-raw-sys` project are retained by their contributors — 111
- `rustix` 0.38.44 — https://github.com/bytecodealliance/rustix — Copyrights in the `rustix` project are retained by their contributors — 111
- `rustix` 1.1.5 — https://github.com/bytecodealliance/rustix — Copyrights in the `rustix` project are retained by their contributors — 111

### Apache-2.0 OR BSL-1.0 (1)

- `ryu` 1.0.23 — https://github.com/dtolnay/ryu — authors: David Tolnay — both

### Apache-2.0 OR ISC OR MIT (1)

- `rustls` 0.23.45 — https://github.com/rustls/rustls — Copyright (c) 2016, Joseph Birr-Pixton; Copyright (c) 2016 Joseph Birr-Pixton — 111

### Apache-2.0 OR MIT (117)

- `aead` 0.6.1 — https://github.com/RustCrypto/traits — Copyright (c) 2019-2026 The RustCrypto Project Developers; Copyright (c) 2019 MobileCoin, LLC — 111
- `anyhow` 1.0.104 — https://github.com/dtolnay/anyhow — authors: David Tolnay — both
- `arrayvec` 0.7.8 — https://github.com/bluss/arrayvec — Copyright (c) Ulrik Sverdrup "bluss" 2015-2023 — 111
- `atomic-waker` 1.1.2 — https://github.com/smol-rs/atomic-waker — Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors — both
- `base64` 0.23.1 — https://github.com/marshallpierce/rust-base64 — Copyright (c) 2025 Alice Maz, Marshall Pierce — both
- `bitflags` 2.13.2 — https://github.com/bitflags/bitflags — Copyright (c) 2014 The Rust Project Developers — 111
- `blake2` 0.11.0 — https://github.com/RustCrypto/hashes — Copyright (c) 2017-2026 The RustCrypto Project Developers; Copyright (c) 2017 Artyom Pavlov; Copyright (c) 2015-2016 The blake2-rfc Developers, Cesar Barros — 111
- `block-buffer` 0.10.4 — https://github.com/RustCrypto/utils — Copyright (c) 2018-2019 The RustCrypto Project Developers — 111
- `block-buffer` 0.12.1 — https://github.com/RustCrypto/utils — Copyright (c) 2018-2025 The RustCrypto Project Developers — 111
- `cfg-if` 1.0.5 — https://github.com/rust-lang/cfg-if — Copyright (c) 2014 Alex Crichton — 111
- `chacha20` 0.10.2 — https://github.com/RustCrypto/stream-ciphers — Copyright (c) 2019-2026 The RustCrypto Project Developers — 111
- `chacha20poly1305` 0.11.0 — https://github.com/RustCrypto/AEADs — Copyright (c) 2019-2026 The RustCrypto Project Developers — 111
- `cipher` 0.5.2 — https://github.com/RustCrypto/traits — Copyright (c) 2016-2025 RustCrypto Developers — 111
- `cmov` 0.5.4 — https://github.com/RustCrypto/utils — Copyright (c) 2022-2026 The RustCrypto Project Developers — 111
- `const-oid` 0.10.2 — https://github.com/RustCrypto/formats — Copyright (c) 2020-2026 The RustCrypto Project Developers — 111
- `cpufeatures` 0.2.17 — https://github.com/RustCrypto/utils — Copyright (c) 2020-2025 The RustCrypto Project Developers — 111
- `cpufeatures` 0.3.1 — https://github.com/RustCrypto/utils — Copyright (c) 2020-2026 The RustCrypto Project Developers — 111
- `crc32fast` 1.5.2 — https://github.com/srijs/rust-crc32fast — Copyright (c) 2018 Sam Rijs, Alex Crichton and contributors — 111
- `crossbeam-deque` 0.8.8 — https://github.com/crossbeam-rs/crossbeam — Copyright (c) 2019 The Crossbeam Project Developers — 111
- `crossbeam-epoch` 0.9.21 — https://github.com/crossbeam-rs/crossbeam — Copyright (c) 2019 The Crossbeam Project Developers — 111
- `crossbeam-utils` 0.8.23 — https://github.com/crossbeam-rs/crossbeam — Copyright (c) 2019 The Crossbeam Project Developers — 111
- `crypto-common` 0.1.7 — https://github.com/RustCrypto/traits — Copyright (c) 2021 RustCrypto Developers — 111
- `crypto-common` 0.2.2 — https://github.com/RustCrypto/traits — Copyright (c) 2021-2026 RustCrypto Developers — 111
- `ctutils` 0.4.3 — https://github.com/RustCrypto/utils — Copyright (c) 2025-2026 The RustCrypto Project Developers — 111
- `curve25519-dalek-derive` 0.1.1 — https://github.com/dalek-cryptography/curve25519-dalek — no copyright line in package; see repository — 111
- `deranged` 0.5.8 — https://github.com/jhpratt/deranged — Copyright 2024 Jacob Pratt et al; Copyright (c) 2024 Jacob Pratt et al — both
- `digest` 0.10.7 — https://github.com/RustCrypto/traits — Copyright (c) 2017 Artyom Pavlov — 111
- `digest` 0.11.3 — https://github.com/RustCrypto/traits — Copyright (c) 2017-2025 RustCrypto Developers; Copyright (c) 2017 Artyom Pavlov — 111
- `ed25519` 2.2.3 — https://github.com/RustCrypto/signatures/tree/master/ed25519 — Copyright 2018-2022 RustCrypto Developers; Copyright (c) 2018-2023 RustCrypto Developers — 111
- `either` 1.19.0 — https://github.com/rayon-rs/either — Copyright (c) 2015 — 111
- `equivalent` 1.0.2 — https://github.com/indexmap-rs/equivalent — Copyright (c) 2016--2023 — 111
- `errno` 0.3.14 — https://github.com/lambda-fairy/rust-errno — Copyright (c) 2014 Chris Wong — both
- `fastrand` 2.5.0 — https://github.com/smol-rs/fastrand — authors: Stjepan Glavina — 111
- `filetime` 0.2.29 — https://github.com/alexcrichton/filetime — Copyright (c) 2014 Alex Crichton — 111
- `flate2` 1.1.10 — https://github.com/rust-lang/flate2-rs — Copyright (c) 2014-2026 Alex Crichton — 111
- `flume` 0.12.0 — https://github.com/zesterer/flume — authors: Joshua Barretto — 111
- `form_urlencoded` 1.2.2 — https://github.com/servo/rust-url — Copyright (c) 2013-2016 The rust-url developers — both
- `futures` 0.3.34 — https://github.com/rust-lang/futures-rs — Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors — 111
- `futures-channel` 0.3.34 — https://github.com/rust-lang/futures-rs — Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors — 111
- `futures-core` 0.3.34 — https://github.com/rust-lang/futures-rs — Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors — both
- `futures-executor` 0.3.34 — https://github.com/rust-lang/futures-rs — Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors — 111
- `futures-io` 0.3.34 — https://github.com/rust-lang/futures-rs — Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors — 111
- `futures-macro` 0.3.34 — https://github.com/rust-lang/futures-rs — Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors — 111
- `futures-sink` 0.3.34 — https://github.com/rust-lang/futures-rs — Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors — both
- `futures-task` 0.3.34 — https://github.com/rust-lang/futures-rs — Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors — both
- `futures-util` 0.3.34 — https://github.com/rust-lang/futures-rs — Copyright (c) 2016 Alex Crichton; Copyright (c) 2017 The Tokio Authors — both
- `getrandom` 0.2.17 — https://github.com/rust-random/getrandom — Copyright (c) 2018-2024 The rust-random Project Developers; Copyright (c) 2014 The Rust Project Developers — 111
- `getrandom` 0.4.3 — https://github.com/rust-random/getrandom — Copyright (c) 2018-2026 The rust-random Project Developers; Copyright (c) 2014 The Rust Project Developers — 111
- `hashbrown` 0.17.1 — https://github.com/rust-lang/hashbrown — Copyright (c) 2016 Amanieu d'Antras — 111
- `hmac` 0.13.0 — https://github.com/RustCrypto/MACs — Copyright (c) 2017 Artyom Pavlov — 111
- `http` 1.5.0 — https://github.com/hyperium/http — Copyright 2017 http-rs authors; Copyright (c) 2017 http-rs authors — both
- `httparse` 1.10.1 — https://github.com/seanmonstar/httparse — Copyright (c) 2015-2025 Sean McArthur — both
- `httpdate` 1.0.3 — https://github.com/pyfisch/httpdate — Copyright (c) 2016 Pyfisch — both
- `hybrid-array` 0.4.15 — https://github.com/RustCrypto/hybrid-array — Copyright (c) 2022-2026 The RustCrypto Project Developers — 111
- `indexmap` 2.14.2 — https://github.com/indexmap-rs/indexmap — Copyright (c) 2016--2017 — 111
- `inout` 0.2.2 — https://github.com/RustCrypto/utils — Copyright (c) 2022-2025 The RustCrypto Project Developers; Copyright (c) 2022 Artyom Pavlov — 111
- `itoa` 1.0.18 — https://github.com/dtolnay/itoa — authors: David Tolnay — both
- `libc` 0.2.190 — https://github.com/rust-lang/libc — Copyright (c) The Rust Project Developers — both
- `lock_api` 0.4.14 — https://github.com/Amanieu/parking_lot — Copyright (c) 2016 The Rust Project Developers — 111
- `log` 0.4.34 — https://github.com/rust-lang/log — Copyright (c) 2014 The Rust Project Developers — both
- `mdns-sd` 0.21.5 — https://github.com/keepsimple1/mdns-sd — Copyright [2021-2022] [Han Xu, keepsimple@gmail.com]; Copyright (c) 2021-2022, Han Xu, keepsimple@gmail.com — 111
- `mime` 0.3.17 — https://github.com/hyperium/mime — Copyright (c) 2014 Sean McArthur — both
- `num-conv` 0.2.2 — https://github.com/jhpratt/num-conv — Copyright (c) Jacob Pratt — both
- `nusb` 0.1.14 — https://github.com/kevinmehall/nusb — authors: Kevin Mehall — 111
- `once_cell` 1.21.4 — https://github.com/matklad/once_cell — authors: Aleksey Kladov — both
- `paste` 1.0.15 — https://github.com/dtolnay/paste — authors: David Tolnay — 111
- `pbkdf2` 0.13.0 — https://github.com/RustCrypto/password-hashes — Copyright (c) 2017 Artyom Pavlov; Copyright (c) 2018-2023 The RustCrypto Project Developers — 111
- `percent-encoding` 2.3.2 — https://github.com/servo/rust-url/ — Copyright (c) 2013-2025 The rust-url developers — both
- `pin-project-lite` 0.2.17 — https://github.com/taiki-e/pin-project-lite — no copyright line in package; see repository — both
- `poly1305` 0.9.1 — https://github.com/RustCrypto/universal-hashes — Copyright (c) 2015-2026 The RustCrypto Project Developers — 111
- `powerfmt` 0.2.1 — https://github.com/jhpratt/powerfmt — Copyright 2023 Jacob Pratt et al; Copyright (c) 2023 Jacob Pratt et al — both
- `proc-macro2` 1.0.107 — https://github.com/dtolnay/proc-macro2 — authors: David Tolnay, Alex Crichton — both
- `qrcode` 0.14.1 — https://github.com/kennytm/qrcode-rust — Copyright (c) 2016 kennytm — 111
- `quote` 1.0.47 — https://github.com/dtolnay/quote — authors: David Tolnay — both
- `rand_core` 0.6.4 — https://github.com/rust-random/rand — Copyrights in the Rand project are retained by their contributors. No; Copyright 2018 Developers of the Rand project; Copyright (c) 2014 The Rust Project Developers — 111
- `rayon` 1.12.0 — https://github.com/rayon-rs/rayon — Copyright (c) 2010 The Rust Project Developers — 111
- `rayon-core` 1.13.0 — https://github.com/rayon-rs/rayon — Copyright (c) 2010 The Rust Project Developers — 111
- `rustls-pki-types` 1.15.1 — https://github.com/rustls/pki-types — Copyright 2023 Dirkjan Ochtman; Copyright (c) 2023 Dirkjan Ochtman — 111
- `scopeguard` 1.2.0 — https://github.com/bluss/scopeguard — Copyright (c) 2016-2019 Ulrik Sverdrup "bluss" and scopeguard developers — 111
- `sd-notify` 0.4.5 — https://github.com/lnicola/sd-notify — Copyright (c) 2019 Laurențiu Nicola — 111
- `serde` 1.0.229 — https://github.com/serde-rs/serde — authors: Erick Tryzelaar, David Tolnay — both
- `serde_core` 1.0.229 — https://github.com/serde-rs/serde — authors: Erick Tryzelaar, David Tolnay — both
- `serde_derive` 1.0.229 — https://github.com/serde-rs/serde — authors: Erick Tryzelaar, David Tolnay — both
- `serde_json` 1.0.154 — https://github.com/serde-rs/json — no copyright line in package; see repository — both
- `serde_path_to_error` 0.1.20 — https://github.com/dtolnay/path-to-error — authors: David Tolnay — both
- `serde_spanned` 0.6.9 — https://github.com/toml-rs/toml — Copyright (c) Individual contributors — 111
- `serde_urlencoded` 0.7.1 — https://github.com/nox/serde_urlencoded — Copyright (c) 2016 Anthony Ramine — both
- `sha2` 0.10.9 — https://github.com/RustCrypto/hashes — Copyright (c) 2006-2009 Graydon Hoare; Copyright (c) 2009-2013 Mozilla Foundation; Copyright (c) 2016 Artyom Pavlov — 111
- `sha2` 0.11.1 — https://github.com/RustCrypto/hashes — Copyright (c) 2016-2026 The RustCrypto Project Developers; Copyright (c) 2016 Artyom Pavlov; Copyright (c) 2009-2013 Mozilla Foundation ... — 111
- `signal-hook-registry` 1.4.8 — https://github.com/vorner/signal-hook — Copyright (c) 2017 tokio-jsonrpc developers — both
- `signature` 2.2.0 — https://github.com/RustCrypto/traits/tree/master/signature — Copyright (c) 2018-2023 RustCrypto Developers — 111
- `smallvec` 1.16.3 — https://github.com/servo/rust-smallvec — Copyright (c) 2018 The Servo Project Developers — both
- `socket2` 0.6.5 — https://github.com/rust-lang/socket2 — Copyright (c) 2014 Alex Crichton — both
- `syn` 2.0.119 — https://github.com/dtolnay/syn — authors: David Tolnay — 111
- `syn` 3.0.8 — https://github.com/dtolnay/syn — authors: David Tolnay — both
- `tar` 0.4.46 — https://github.com/composefs/tar-rs — Copyright (c) The tar-rs Project Contributors — 111
- `tempfile` 3.27.0 — https://github.com/Stebalien/tempfile — Copyright (c) 2015 Steven Allen — 111
- `thiserror` 1.0.69 — https://github.com/dtolnay/thiserror — authors: David Tolnay — 111
- `thiserror` 2.0.21 — https://github.com/dtolnay/thiserror — authors: David Tolnay — 111
- `thiserror-impl` 1.0.69 — https://github.com/dtolnay/thiserror — authors: David Tolnay — 111
- `thiserror-impl` 2.0.21 — https://github.com/dtolnay/thiserror — authors: David Tolnay — 111
- `time` 0.3.55 — https://github.com/time-rs/time — Copyright (c) Jacob Pratt et al — both
- `time-core` 0.1.9 — https://github.com/time-rs/time — Copyright (c) Jacob Pratt et al — both
- `toml` 0.8.23 — https://github.com/toml-rs/toml — Copyright (c) Individual contributors — 111
- `toml_datetime` 0.6.11 — https://github.com/toml-rs/toml — Copyright (c) Individual contributors — 111
- `toml_edit` 0.22.27 — https://github.com/toml-rs/toml — Copyright (c) Individual contributors — 111
- `toml_write` 0.1.2 — https://github.com/toml-rs/toml — Copyright (c) Individual contributors — 111
- `typenum` 1.20.1 — https://github.com/paholg/typenum — Copyright 2014 Paho Lurie-Gregg; Copyright (c) 2014 Paho Lurie-Gregg — 111
- `unicase` 2.10.0 — https://github.com/seanmonstar/unicase — Copyright (c) 2014-2026 Sean McArthur — 111
- `unicode-normalization` 0.1.25 — https://github.com/unicode-rs/unicode-normalization — Copyright (c) 2015 The Rust Project Developers — 111
- `universal-hash` 0.6.1 — https://github.com/RustCrypto/traits — Copyright (c) 2019-2025 RustCrypto Developers — 111
- `ureq` 3.4.2 — https://github.com/algesten/ureq — Copyright (c) 2019 Martin Algesten — both
- `ureq-proto` 0.6.4 — https://github.com/algesten/ureq-proto — Copyright 2022 Martin Algesten — both
- `utf8-zero` 0.8.1 — https://github.com/algesten/utf8-zero — authors: Simon Sapin, Martin Algesten — both
- `xattr` 1.6.1 — https://github.com/Stebalien/xattr — Copyright (c) 2015 Steven Allen — 111
- `zeroize` 1.9.1 — https://github.com/RustCrypto/utils — Copyright (c) 2018-2026 The RustCrypto Project Developers — 111
- `zeroize_derive` 1.5.0 — https://github.com/RustCrypto/utils — Copyright (c) 2019-2026 The RustCrypto Project Developers — 111

### Apache-2.0 OR MIT OR Zlib (2)

- `miniz_oxide` 0.9.1 — https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide — Copyright 2013-2014 RAD Game Tools and Valve Software; Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC; Copyright (c) 2017 Frommi ... — 111
- `tinyvec` 1.13.3 — https://github.com/Lokathor/tinyvec — Copyright (c) 2019 Daniel "Lokathor" Gee — 111

### BSD-3-Clause (4)

- `curve25519-dalek` 4.1.3 — https://github.com/dalek-cryptography/curve25519-dalek/tree/main/curve25519-dalek — Copyright (c) 2016-2021 isis agora lovecruft.; Copyright (c) 2016-2021 Henry de Valence.; Copyright (c) 2012 The Go Authors. — 111
- `ed25519-dalek` 2.2.0 — https://github.com/dalek-cryptography/curve25519-dalek/tree/main/ed25519-dalek — Copyright (c) 2017-2019 isis agora lovecruft. — 111
- `subtle` 2.6.1 — https://github.com/dalek-cryptography/subtle — Copyright (c) 2016-2017 Isis Agora Lovecruft, Henry de Valence.; Copyright (c) 2016-2024 Isis Agora Lovecruft. — 111
- `x25519-dalek` 2.0.1 — https://github.com/dalek-cryptography/curve25519-dalek/tree/main/x25519-dalek — Copyright (c) 2017-2021 isis agora lovecruft.; Copyright (c) 2019-2021 DebugSteven. — 111

### BSD-3-Clause OR MIT (1)

- `if-addrs` 0.15.0 — https://github.com/messense/if-addrs — Copyright 2018 MaidSafe.net limited; Copyright 2020 messense — 111

### CC0-1.0 (3)

- `bip39` 3.0.0 — https://github.com/rust-bitcoin/rust-bip39/ — authors: Steven Roose — 111
- `bitcoin_hashes` 0.14.101 — https://github.com/rust-bitcoin/rust-bitcoin — authors: Andrew Poelstra — 111
- `hex-conservative` 0.2.3 — https://github.com/rust-bitcoin/hex-conservative — authors: Martin Habovštiak, Andrew Poelstra — 111

### CDLA-Permissive-2.0 (1)

- `webpki-roots` 1.0.9 — https://github.com/rustls/webpki-roots — no copyright line in package; see repository — 111

### ISC (2)

- `rustls-webpki` 0.103.15 — https://github.com/rustls/webpki — Copyright 2015 Brian Smith — 111
- `untrusted` 0.9.0 — https://github.com/briansmith/untrusted — authors: Brian Smith — 111

### MIT (42)

- `axum` 0.8.9 — https://github.com/tokio-rs/axum — Copyright (c) 2019 axum Contributors — both
- `axum-core` 0.5.6 — https://github.com/tokio-rs/axum — Copyright (c) 2019–2025 axum Contributors — both
- `bech32` 0.12.0 — https://github.com/rust-bitcoin/rust-bech32 — Copyright (c) 2017 Clark Moody — 111
- `bytes` 1.12.1 — https://github.com/tokio-rs/bytes — Copyright (c) 2018 Carl Lerche — both
- `generic-array` 0.14.7 — https://github.com/fizyk20/generic-array.git — Copyright (c) 2015 Bartłomiej Kamiński — 111
- `http-body` 1.1.0 — https://github.com/hyperium/http-body — Copyright (c) 2019-2026 Sean McArthur & Hyper Contributors — both
- `http-body-util` 0.1.5 — https://github.com/hyperium/http-body — Copyright (c) 2019-2026 Sean McArthur & Hyper Contributors — both
- `hyper` 1.12.0 — https://github.com/hyperium/hyper — Copyright (c) 2014-2026 Sean McArthur — both
- `hyper-util` 0.1.21 — https://github.com/hyperium/hyper-util — Copyright (c) 2023-2025 Sean McArthur — both
- `mime_guess` 2.0.5 — https://github.com/abonander/mime_guess — Copyright (c) 2015 Austin Bonander — 111
- `minisign-verify` 0.2.5 — https://github.com/jedisct1/rust-minisign-verify — Copyright (c) 2019-2025 Frank Denis; Copyright (c) 2006-2009 Graydon Hoare; Copyright (c) 2009-2013 Mozilla Foundation — 111
- `mio` 1.2.4 — https://github.com/tokio-rs/mio — Copyright (c) 2014 Carl Lerche and other MIO contributors — both
- `netlink-packet-core` 0.7.0 — https://github.com/rust-netlink/netlink-packet-core — authors: Corentin Henry — 111
- `netlink-packet-route` 0.19.0 — https://github.com/rust-netlink/netlink-packet-route — authors: Corentin Henry — 111
- `netlink-packet-utils` 0.5.2 — https://github.com/rust-netlink/netlink-packet-utils — authors: Corentin Henry — 111
- `netlink-proto` 0.11.5 — https://github.com/rust-netlink/netlink-proto — authors: Corentin Henry — 111
- `netlink-sys` 0.8.8 — https://github.com/rust-netlink/netlink-sys — authors: Corentin Henry — 111
- `nix` 0.27.1 — https://github.com/nix-rust/nix — Copyright (c) 2015 Carl Lerche + nix-rust Authors — 111
- `nix` 0.31.3 — https://github.com/nix-rust/nix — Copyright (c) 2015 Carl Lerche + nix-rust Authors — 111
- `os_info` 3.15.0 — https://github.com/stanislav-tkach/os_info — Copyright (c) 2017 Stanislav Tkach — 111
- `rtnetlink` 0.14.1 — https://github.com/rust-netlink/rtnetlink — authors: Corentin Henry — 111
- `rust-embed` 8.13.0 — https://pyrossh.dev/repos/rust-embed — Copyright (c) 2018 pyros2097 — 111
- `rust-embed-impl` 8.13.0 — https://pyrossh.dev/repos/rust-embed — Copyright (c) 2018 pyros2097 — 111
- `rust-embed-utils` 8.13.0 — https://pyrossh.dev/repos/rust-embed — Copyright (c) 2018 pyros2097 — 111
- `simd-adler32` 0.3.10 — https://github.com/mcountryman/simd-adler32 — Copyright (c) [2021] [Marvin Countryman] — 111
- `slab` 0.4.12 — https://github.com/tokio-rs/slab — Copyright (c) 2019 Carl Lerche — both
- `socket-pktinfo` 0.4.1 — https://github.com/pixsper/socket-pktinfo — Copyright (c) 2025 Pixsper — 111
- `spin` 0.9.9 — https://github.com/mvdnes/spin-rs.git — Copyright (c) 2014 Mathijs van de Nes — 111
- `sysinfo` 0.33.1 — https://github.com/GuillaumeGomez/sysinfo — Copyright (c) 2015 Guillaume Gomez — 111
- `tokio` 1.53.2 — https://github.com/tokio-rs/tokio — Copyright (c) Tokio Contributors — both
- `tokio-macros` 2.7.2 — https://github.com/tokio-rs/tokio — Copyright (c) 2019 Yoshua Wuyts; Copyright (c) Tokio Contributors — both
- `tokio-stream` 0.1.19 — https://github.com/tokio-rs/tokio — Copyright (c) Tokio Contributors — 111-agent
- `tokio-util` 0.7.20 — https://github.com/tokio-rs/tokio — Copyright (c) Tokio Contributors — 111-agent
- `tower` 0.5.3 — https://github.com/tower-rs/tower — Copyright (c) 2019 Tower Contributors — both
- `tower-layer` 0.3.3 — https://github.com/tower-rs/tower — Copyright (c) 2019 Tower Contributors — both
- `tower-service` 0.3.3 — https://github.com/tower-rs/tower — Copyright (c) 2019 Tower Contributors — both
- `tracing` 0.1.44 — https://github.com/tokio-rs/tracing — Copyright (c) 2019 Tokio Contributors — both
- `tracing-core` 0.1.36 — https://github.com/tokio-rs/tracing — Copyright (c) 2019 Tokio Contributors — both
- `try-lock` 0.2.5 — https://github.com/seanmonstar/try-lock — Copyright (c) 2018-2023 Sean McArthur; Copyright (c) 2016 Alex Crichton — 111
- `want` 0.3.2 — https://github.com/seanmonstar/want — Copyright (c) 2018-2019 Sean McArthur — 111
- `winnow` 0.7.15 — https://github.com/winnow-rs/winnow — no copyright line in package; see repository — 111
- `zmij` 1.0.23 — https://github.com/dtolnay/zmij — authors: David Tolnay — both

### MIT AND BSD-3-Clause (1)

- `matchit` 0.8.4 — https://github.com/ibraheemdev/matchit — Copyright (c) 2022 Ibraheem Ahmed; Copyright (c) 2013, Julien Schmidt — both

### MIT OR Unlicense (4)

- `byteorder` 1.5.0 — https://github.com/BurntSushi/byteorder — Copyright (c) 2015 Andrew Gallant — 111
- `memchr` 2.8.3 — https://github.com/BurntSushi/memchr — Copyright (c) 2015 Andrew Gallant — both
- `same-file` 1.0.6 — https://github.com/BurntSushi/same-file — Copyright (c) 2017 Andrew Gallant — 111
- `walkdir` 2.5.0 — https://github.com/BurntSushi/walkdir — Copyright (c) 2015 Andrew Gallant — 111

Notes:

- `ring` 0.17.14 (Apache-2.0 AND ISC): its own code is ISC (Copyright 2015-2025 Brian Smith). Code taken from BoringSSL is under Apache-2.0 or ISC as stated at the top of each file.
  Its `LICENSE-BoringSSL` also lists licenses for test-suite code that is not compiled into the library.
- `matchit` 0.8.4 (MIT AND BSD-3-Clause): the BSD-3-Clause part is code from `julienschmidt/httprouter` (its `LICENSE.httprouter`).
- `unicode-ident` 1.0.27 ((MIT OR Apache-2.0) AND Unicode-3.0): the Unicode-3.0 part covers data files from Unicode, Inc.
- `webpki-roots` 1.0.9 (CDLA-Permissive-2.0): Mozilla's CA root certificates, embedded as data in the `111` binary for TLS. The agreement text is reproduced below, as its section 2.1 requires.
- Options named in expressions but not elected, so not reproduced: 0BSD, BSL-1.0, Zlib.

## JavaScript packages (dashboard bundle)

6 production packages from `interface/ui`.

- `react` 19.3.0 — MIT — https://github.com/react/react.git — Copyright (c) Meta Platforms, Inc. and affiliates
- `react-dom` 19.3.0 — MIT — https://github.com/react/react.git — Copyright (c) Meta Platforms, Inc. and affiliates
- `regexparam` 3.0.0 — MIT — https://github.com/lukeed/regexparam — Copyright (c) Luke Edwards (lukeed.com)
- `scheduler` 0.28.0 — MIT — https://github.com/react/react.git — Copyright (c) Meta Platforms, Inc. and affiliates
- `use-sync-external-store` 1.7.0 — MIT — https://github.com/react/react.git — Copyright (c) Meta Platforms, Inc. and affiliates
- `wouter` 3.13.0 — Unlicense — https://github.com/molefrog/wouter.git — none in package; license from package.json

`wouter` publishes no license file in its package; its license is the `Unlicense` declared in `package.json`. Dev-only tooling is not shipped and is not listed.

## Fonts

Self-hosted woff2 files in the dashboard, unmodified copies (provenance in `interface/FONTS.md`). All four are under the SIL Open Font License 1.1.

| Font | File | SHA-256 | Copyright | Source |
|---|---|---|---|---|
| Geist Mono | `GeistMono-Variable.woff2` | `5f687a5dd4c87da13deaff9f6b9503d5e62249ff501265a96b134565f9aa8c87` | Copyright (c) 2023 Vercel, in collaboration with basement.studio | https://github.com/vercel/geist-font (npm package `geist` 1.7.0) |
| Geist Pixel Line | `GeistPixel-Line.woff2` | `3a22f02388d8b30780ec260eb064e9701f75a1f9bac7f9e79d5efdf96f8d4259` | Copyright (c) 2023 Vercel, in collaboration with basement.studio | https://github.com/vercel/geist-font (npm package `geist` 1.7.0) |
| Silkscreen 400 | `Silkscreen-400-latin.woff2` | `e6c72ea4702249202bcdd79d3343057e4e25ef1f04e3fcffd8602ab53b40b4cc` | Copyright 2001 The Silkscreen Project Authors (https://github.com/googlefonts/silkscreen) | https://github.com/googlefonts/silkscreen (served by Google Fonts) |
| Silkscreen 700 | `Silkscreen-700-latin.woff2` | `5741b7f01a127f40ed70281fa2f686e8fc2cfe241930a120e4d1a1cebe78f995` | Copyright 2001 The Silkscreen Project Authors (https://github.com/googlefonts/silkscreen) | https://github.com/googlefonts/silkscreen (served by Google Fonts) |

The OFL text reproduced below is Geist's. Silkscreen's `LICENSE-silkscreen.txt` is the same license with the Silkscreen copyright line above and minor typographic differences.
The fonts may be bundled with software under any license but may not be sold on their own.

## Vendored source

- **Monocypher** 4.0.3 — `BSD-2-Clause OR CC0-1.0` — https://monocypher.org — Copyright (c) 2017-2023 Loup Vaillant; Copyright (c) 2017-2019 Michael Savage; Copyright (c) 2017-2023 Fabio Scotoni.
  Files: `identity/monocypher-harness/vendor/monocypher.{c,h}` and `optional/monocypher-ed25519.{c,h}`, hash-pinned in `SHA256SUMS`. It is used only by a host-side test harness and is not linked into any
  shipped binary. Its full dual license is `identity/monocypher-harness/vendor/LICENCE.md`; the BSD-2-Clause and CC0-1.0 texts are reproduced below.

## License texts

### MIT

Canonical: https://spdx.org/licenses/MIT.html. Source of this copy: template body from a local copy (`byteorder` 1.5.0 `LICENSE-MIT`); the copyright line is generic and each component's own line is listed above.

```text
MIT License

Copyright (c) <year> <copyright holders: see the per-component lines above>

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

### Apache-2.0

Canonical: https://www.apache.org/licenses/LICENSE-2.0. Source of this copy: full text from a local copy (`ring` 0.17.14 `LICENSE-BoringSSL`, which carries the unmodified Apache License 2.0).

```text
                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright [yyyy] [name of copyright owner]

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
```

### BSD-3-Clause

Canonical: https://spdx.org/licenses/BSD-3-Clause.html. Source of this copy: body from a local copy (`curve25519-dalek` 4.1.3 `LICENSE`); generic copyright line.

```text
Copyright (c) <year> <copyright holders: see the per-component lines above>

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

1. Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
contributors may be used to endorse or promote products derived from
this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS
IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED
TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A
PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED
TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE. 

========================================================================

Portions of curve25519-dalek were originally derived from Adam Langley's
Go ed25519 implementation, found at <https://github.com/agl/ed25519/>,
under the following licence:

========================================================================

Copyright (c) 2012 The Go Authors. All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

   * Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.
   * Redistributions in binary form must reproduce the above
copyright notice, this list of conditions and the following disclaimer
in the documentation and/or other materials provided with the
distribution.
   * Neither the name of Google Inc. nor the names of its
contributors may be used to endorse or promote products derived from
this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS
IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED
TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A
PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER
OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### BSD-2-Clause

Canonical: https://spdx.org/licenses/BSD-2-Clause.html. Source of this copy: body from the vendored Monocypher `LICENCE.md`; generic copyright line (Monocypher's own lines are in its entry).

```text
Copyright (c) <year> <copyright holders: see the per-component lines above>
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

1. Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the
   distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### ISC

Canonical: https://spdx.org/licenses/ISC.html. Source of this copy: body from a local copy (`ring` 0.17.14 `LICENSE-other-bits`); generic copyright line.

```text
Copyright (c) <year> <copyright holders: see the per-component lines above>

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY
SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION
OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
```

### CC0-1.0

Canonical: https://creativecommons.org/publicdomain/zero/1.0/legalcode. Source of this copy: full text from a local copy (`bip39` 3.0.0 `LICENSE`).

```text
Creative Commons Legal Code

CC0 1.0 Universal

    CREATIVE COMMONS CORPORATION IS NOT A LAW FIRM AND DOES NOT PROVIDE
    LEGAL SERVICES. DISTRIBUTION OF THIS DOCUMENT DOES NOT CREATE AN
    ATTORNEY-CLIENT RELATIONSHIP. CREATIVE COMMONS PROVIDES THIS
    INFORMATION ON AN "AS-IS" BASIS. CREATIVE COMMONS MAKES NO WARRANTIES
    REGARDING THE USE OF THIS DOCUMENT OR THE INFORMATION OR WORKS
    PROVIDED HEREUNDER, AND DISCLAIMS LIABILITY FOR DAMAGES RESULTING FROM
    THE USE OF THIS DOCUMENT OR THE INFORMATION OR WORKS PROVIDED
    HEREUNDER.

Statement of Purpose

The laws of most jurisdictions throughout the world automatically confer
exclusive Copyright and Related Rights (defined below) upon the creator
and subsequent owner(s) (each and all, an "owner") of an original work of
authorship and/or a database (each, a "Work").

Certain owners wish to permanently relinquish those rights to a Work for
the purpose of contributing to a commons of creative, cultural and
scientific works ("Commons") that the public can reliably and without fear
of later claims of infringement build upon, modify, incorporate in other
works, reuse and redistribute as freely as possible in any form whatsoever
and for any purposes, including without limitation commercial purposes.
These owners may contribute to the Commons to promote the ideal of a free
culture and the further production of creative, cultural and scientific
works, or to gain reputation or greater distribution for their Work in
part through the use and efforts of others.

For these and/or other purposes and motivations, and without any
expectation of additional consideration or compensation, the person
associating CC0 with a Work (the "Affirmer"), to the extent that he or she
is an owner of Copyright and Related Rights in the Work, voluntarily
elects to apply CC0 to the Work and publicly distribute the Work under its
terms, with knowledge of his or her Copyright and Related Rights in the
Work and the meaning and intended legal effect of CC0 on those rights.

1. Copyright and Related Rights. A Work made available under CC0 may be
protected by copyright and related or neighboring rights ("Copyright and
Related Rights"). Copyright and Related Rights include, but are not
limited to, the following:

  i. the right to reproduce, adapt, distribute, perform, display,
     communicate, and translate a Work;
 ii. moral rights retained by the original author(s) and/or performer(s);
iii. publicity and privacy rights pertaining to a person's image or
     likeness depicted in a Work;
 iv. rights protecting against unfair competition in regards to a Work,
     subject to the limitations in paragraph 4(a), below;
  v. rights protecting the extraction, dissemination, use and reuse of data
     in a Work;
 vi. database rights (such as those arising under Directive 96/9/EC of the
     European Parliament and of the Council of 11 March 1996 on the legal
     protection of databases, and under any national implementation
     thereof, including any amended or successor version of such
     directive); and
vii. other similar, equivalent or corresponding rights throughout the
     world based on applicable law or treaty, and any national
     implementations thereof.

2. Waiver. To the greatest extent permitted by, but not in contravention
of, applicable law, Affirmer hereby overtly, fully, permanently,
irrevocably and unconditionally waives, abandons, and surrenders all of
Affirmer's Copyright and Related Rights and associated claims and causes
of action, whether now known or unknown (including existing as well as
future claims and causes of action), in the Work (i) in all territories
worldwide, (ii) for the maximum duration provided by applicable law or
treaty (including future time extensions), (iii) in any current or future
medium and for any number of copies, and (iv) for any purpose whatsoever,
including without limitation commercial, advertising or promotional
purposes (the "Waiver"). Affirmer makes the Waiver for the benefit of each
member of the public at large and to the detriment of Affirmer's heirs and
successors, fully intending that such Waiver shall not be subject to
revocation, rescission, cancellation, termination, or any other legal or
equitable action to disrupt the quiet enjoyment of the Work by the public
as contemplated by Affirmer's express Statement of Purpose.

3. Public License Fallback. Should any part of the Waiver for any reason
be judged legally invalid or ineffective under applicable law, then the
Waiver shall be preserved to the maximum extent permitted taking into
account Affirmer's express Statement of Purpose. In addition, to the
extent the Waiver is so judged Affirmer hereby grants to each affected
person a royalty-free, non transferable, non sublicensable, non exclusive,
irrevocable and unconditional license to exercise Affirmer's Copyright and
Related Rights in the Work (i) in all territories worldwide, (ii) for the
maximum duration provided by applicable law or treaty (including future
time extensions), (iii) in any current or future medium and for any number
of copies, and (iv) for any purpose whatsoever, including without
limitation commercial, advertising or promotional purposes (the
"License"). The License shall be deemed effective as of the date CC0 was
applied by Affirmer to the Work. Should any part of the License for any
reason be judged legally invalid or ineffective under applicable law, such
partial invalidity or ineffectiveness shall not invalidate the remainder
of the License, and in such case Affirmer hereby affirms that he or she
will not (i) exercise any of his or her remaining Copyright and Related
Rights in the Work or (ii) assert any associated claims and causes of
action with respect to the Work, in either case contrary to Affirmer's
express Statement of Purpose.

4. Limitations and Disclaimers.

 a. No trademark or patent rights held by Affirmer are waived, abandoned,
    surrendered, licensed or otherwise affected by this document.
 b. Affirmer offers the Work as-is and makes no representations or
    warranties of any kind concerning the Work, express, implied,
    statutory or otherwise, including without limitation warranties of
    title, merchantability, fitness for a particular purpose, non
    infringement, or the absence of latent or other defects, accuracy, or
    the present or absence of errors, whether or not discoverable, all to
    the greatest extent permissible under applicable law.
 c. Affirmer disclaims responsibility for clearing rights of other persons
    that may apply to the Work or any use thereof, including without
    limitation any person's Copyright and Related Rights in the Work.
    Further, Affirmer disclaims responsibility for obtaining any necessary
    consents, permissions or other rights required for any use of the
    Work.
 d. Affirmer understands and acknowledges that Creative Commons is not a
    party to this document and has no duty or obligation with respect to
    this CC0 or use of the Work.
```

### Unlicense

Canonical: https://unlicense.org/. Source of this copy: full text from a local copy (`byteorder` 1.5.0 `UNLICENSE`).

```text
This is free and unencumbered software released into the public domain.

Anyone is free to copy, modify, publish, use, compile, sell, or
distribute this software, either in source code form or as a compiled
binary, for any purpose, commercial or non-commercial, and by any
means.

In jurisdictions that recognize copyright laws, the author or authors
of this software dedicate any and all copyright interest in the
software to the public domain. We make this dedication for the benefit
of the public at large and to the detriment of our heirs and
successors. We intend this dedication to be an overt act of
relinquishment in perpetuity of all present and future rights to this
software under copyright law.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
IN NO EVENT SHALL THE AUTHORS BE LIABLE FOR ANY CLAIM, DAMAGES OR
OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE,
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR
OTHER DEALINGS IN THE SOFTWARE.

For more information, please refer to <http://unlicense.org/>
```

### Unicode-3.0

Canonical: https://www.unicode.org/license.txt. Source of this copy: full text from a local copy (`unicode-ident` 1.0.27 `LICENSE-UNICODE`).

```text
UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

Copyright © 1991-2023 Unicode, Inc.

NOTICE TO USER: Carefully read the following legal agreement. BY
DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.

Permission is hereby granted, free of charge, to any person obtaining a
copy of data files and any associated documentation (the "Data Files") or
software and any associated documentation (the "Software") to deal in the
Data Files or Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, and/or sell
copies of the Data Files or Software, and to permit persons to whom the
Data Files or Software are furnished to do so, provided that either (a)
this copyright and permission notice appear with all copies of the Data
Files or Software, or (b) this copyright and permission notice appear in
associated Documentation.

THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
THIRD PARTY RIGHTS.

IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
FILES OR SOFTWARE.

Except as contained in this notice, the name of a copyright holder shall
not be used in advertising or otherwise to promote the sale, use or other
dealings in these Data Files or Software without prior written
authorization of the copyright holder.
```

### OFL-1.1

Canonical: https://openfontlicense.org/open-font-license-official-text/. Source of this copy: full text as shipped with Geist (`interface/ui/public/fonts/LICENSE-geist.txt`). The Silkscreen file differs only in its copyright line and trivial typography, shown under Fonts.

```text
Copyright (c) 2023 Vercel, in collaboration with basement.studio

This Font Software is licensed under the SIL Open Font License, Version 1.1.
This license is copied below, and is also available with a FAQ at:
http://scripts.sil.org/OFL

-----------------------------------------------------------
SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007
-----------------------------------------------------------

PREAMBLE
The goals of the Open Font License (OFL) are to stimulate worldwide
development of collaborative font projects, to support the font creation
efforts of academic and linguistic communities, and to provide a free and
open framework in which fonts may be shared and improved in partnership
with others.

The OFL allows the licensed fonts to be used, studied, modified and
redistributed freely as long as they are not sold by themselves. The
fonts, including any derivative works, can be bundled, embedded,
redistributed and/or sold with any software provided that any reserved
names are not used by derivative works. The fonts and derivatives,
however, cannot be released under any other type of license. The
requirement for fonts to remain under this license does not apply
to any document created using the fonts or their derivatives.

DEFINITIONS
"Font Software" refers to the set of files released by the Copyright
Holder(s) under this license and clearly marked as such. This may
include source files, build scripts and documentation.

"Reserved Font Name" refers to any names specified as such after the
copyright statement(s).

"Original Version" refers to the collection of Font Software components as
distributed by the Copyright Holder(s).

"Modified Version" refers to any derivative made by adding to, deleting,
or substituting -- in part or in whole -- any of the components of the
Original Version, by changing formats or by porting the Font Software to a
new environment.

"Author" refers to any designer, engineer, programmer, technical
writer or other person who contributed to the Font Software.

PERMISSION AND CONDITIONS
Permission is hereby granted, free of charge, to any person obtaining
a copy of the Font Software, to use, study, copy, merge, embed, modify,
redistribute, and sell modified and unmodified copies of the Font
Software, subject to the following conditions:

1) Neither the Font Software nor any of its individual components,
in Original or Modified Versions, may be sold by itself.

2) Original or Modified Versions of the Font Software may be bundled,
redistributed and/or sold with any software, provided that each copy
contains the above copyright notice and this license. These can be
included either as stand-alone text files, human-readable headers or
in the appropriate machine-readable metadata fields within text or
binary files as long as those fields can be easily viewed by the user.

3) No Modified Version of the Font Software may use the Reserved Font
Name(s) unless explicit written permission is granted by the corresponding
Copyright Holder. This restriction only applies to the primary font name as
presented to the users.

4) The name(s) of the Copyright Holder(s) or the Author(s) of the Font
Software shall not be used to promote, endorse or advertise any
Modified Version, except to acknowledge the contribution(s) of the
Copyright Holder(s) and the Author(s) or with their explicit written
permission.

5) The Font Software, modified or unmodified, in part or in whole,
must be distributed entirely under this license, and must not be
distributed under any other license. The requirement for fonts to
remain under this license does not apply to any document created
using the Font Software.

TERMINATION
This license becomes null and void if any of the above conditions are
not met.

DISCLAIMER
THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT
OF COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL THE
COPYRIGHT HOLDER BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL
DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF THE USE OR INABILITY TO USE THE FONT SOFTWARE OR FROM
OTHER DEALINGS IN THE FONT SOFTWARE.
```

### CDLA-Permissive-2.0

Canonical: https://cdla.dev/permissive-2-0/. Source of this copy: full text from a local copy (`webpki-roots` 1.0.9 `LICENSE`). Applies to the embedded Mozilla CA root data.

```text
# Community Data License Agreement - Permissive - Version 2.0

This is the Community Data License Agreement - Permissive, Version
2.0 (the "agreement"). Data Provider(s) and Data Recipient(s) agree
as follows:

## 1. Provision of the Data

1.1. A Data Recipient may use, modify, and share the Data made
available by Data Provider(s) under this agreement if that Data
Recipient follows the terms of this agreement.

1.2. This agreement does not impose any restriction on a Data
Recipient's use, modification, or sharing of any portions of the
Data that are in the public domain or that may be used, modified,
or shared under any other legal exception or limitation.

## 2. Conditions for Sharing Data

2.1. A Data Recipient may share Data, with or without modifications, so
long as the Data Recipient makes available the text of this agreement
with the shared Data.

## 3. No Restrictions on Results

3.1. This agreement does not impose any restriction or obligations
with respect to the use, modification, or sharing of Results.

## 4. No Warranty; Limitation of Liability

4.1. All Data Recipients receive the Data subject to the following
terms:

THE DATA IS PROVIDED ON AN "AS IS" BASIS, WITHOUT REPRESENTATIONS,
WARRANTIES OR CONDITIONS OF ANY KIND, EITHER EXPRESS OR IMPLIED
INCLUDING, WITHOUT LIMITATION, ANY WARRANTIES OR CONDITIONS OF TITLE,
NON-INFRINGEMENT, MERCHANTABILITY OR FITNESS FOR A PARTICULAR PURPOSE.

NO DATA PROVIDER SHALL HAVE ANY LIABILITY FOR ANY DIRECT, INDIRECT,
INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING
WITHOUT LIMITATION LOST PROFITS), HOWEVER CAUSED AND ON ANY THEORY OF
LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE DATA OR RESULTS,
EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGES.

## 5. Definitions

5.1. "Data" means the material received by a Data Recipient under
this agreement.

5.2. "Data Provider" means any person who is the source of Data
provided under this agreement and in reliance on a Data Recipient's
agreement to its terms.

5.3. "Data Recipient" means any person who receives Data directly
or indirectly from a Data Provider and agrees to the terms of this
agreement.

5.4. "Results" means any outcome obtained by computational analysis
of Data, including for example machine learning models and models'
insights.
```

### Silkscreen copyright line (OFL-1.1)

```text
Copyright 2001 The Silkscreen Project Authors (https://github.com/googlefonts/silkscreen)
```
